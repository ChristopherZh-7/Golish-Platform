import type { UnlistenFn } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { isAiSessionInitialized, updateAiWorkspace } from "@/lib/ai";
import { getGitBranch, gitStatus } from "@/lib/api/git";
import { ptyGetForegroundProcess } from "@/lib/api/pty";
import { onEvent } from "@/lib/events";
import { addCommandHistory } from "@/lib/history";
import { logger } from "@/lib/logger";
import { notify } from "@/lib/notify";
import { runTauriUnlistenFn } from "@/lib/run-tauri-unlisten";
import { getSettings } from "@/lib/settings";
import { virtualTerminalManager } from "@/lib/terminal";
import { _drainOutputBufferSize, useStore } from "@/store";
import {
  ALT_SCREEN_TUI_PROCESSES,
  extractProcessName,
  GIT_STATUS_POLL_INTERVAL_MS,
  isFastCommand,
  PROCESS_DETECTION_DELAY_MS,
  SHELL_PROCESSES,
  shouldRefreshGitInfo,
} from "./tauri-event-types";

const STDIN_WAIT_DETECTORS = [
  "yn_choice",
  "password",
  "powershell_choice",
  "continue",
  "generic_prompt",
] as const;
type StdinWaitDetectorChannel = (typeof STDIN_WAIT_DETECTORS)[number];

function normaliseStdinWaitDetector(value: unknown): StdinWaitDetectorChannel {
  return STDIN_WAIT_DETECTORS.includes(value as StdinWaitDetectorChannel)
    ? (value as StdinWaitDetectorChannel)
    : "generic_prompt";
}

let activeGeneration = 0;

export function useTauriEvents() {
  const store = useStore;

  useEffect(() => {
    const generation = ++activeGeneration;
    const isStale = () => generation !== activeGeneration;
    const unlisteners: Promise<UnlistenFn>[] = [];
    const processDetectionTimers = new Map<string, ReturnType<typeof setTimeout>>();
    const usedAlternateScreen = new Map<string, boolean>();

    const deferredExitCodes = new Map<
      string,
      { exitCode: number; endTime: number; fallbackTimer: ReturnType<typeof setTimeout> }
    >();

    const gitRefreshSeq = new Map<string, number>();
    const gitRefreshInFlight = new Set<string>();
    const lastStartedCommand = new Map<string, string | null>();
    // Per-session bookkeeping used to defuse the PowerShell-on-Windows
    // "first dir produces two cards" race. See the prompt_start handler.
    const seenCommandEnd = new Set<string>();
    const lastPromptStartConsumedAt = new Map<string, number>();
    const PROMPT_START_DEDUPE_MS = 300;
    // Bounded per-session counter for the default-on `[pty-trace]`
    // raw-byte dump in the terminal_output handler below — covers about
    // one `dir`'s worth of bytes per fresh session, then goes quiet so
    // the console isn't flooded for the rest of the day.
    const ptyTraceEventsBySession = new Map<string, number>();
    const PTY_TRACE_CAP_PER_SESSION = 80;

    function shortId(id: string): string {
      return id.slice(0, 8);
    }
    // Diagnostic trace for the terminal command-block lifecycle. Lives at
    // debug level so it doesn't show up in the console by default; bump the
    // logger to debug when investigating ordering bugs like the
    // PowerShell-on-Windows first-prompt race.
    function trace(event: string, sessionId: string, extra?: Record<string, unknown>): void {
      logger.debug(
        `[timeline-trace] ${event} sid=${shortId(sessionId)} t=${Date.now()}`,
        extra ?? {}
      );
    }

    // The legacy `terminal.fullterm_commands` setting (a hand-curated
    // allowlist of commands that should jump straight to a full xterm
    // surface) was retired with the Warp-style interactive input mode
    // — we now decide fullterm purely from `alternate_screen` events +
    // the small TUI process name set in `ALT_SCREEN_TUI_PROCESSES`.
    //
    // Settings consumers may still write the old field to disk; we
    // surface a one-shot info log if they do so it's clear the setting
    // is no longer honoured, then otherwise ignore it. The matching
    // schema field will be cleaned up in a follow-up settings PR so we
    // don't drop user data behind their backs in this change.
    getSettings()
      .then((settings) => {
        const legacy = settings.terminal.fullterm_commands ?? [];
        if (legacy.length > 0) {
          logger.info(
            "[useTauriEvents] terminal.fullterm_commands is no longer honoured" +
              " (interactive input + TUI process detection replaces it):",
            legacy
          );
        }
      })
      .catch((err) => {
        logger.debug("Failed to read legacy fullterm_commands setting:", err);
      });

    function refreshGitInfo(sessionId: string, cwd: string) {
      if (gitRefreshInFlight.has(sessionId)) return;

      const state = store.getState();
      const nextSeq = (gitRefreshSeq.get(sessionId) ?? 0) + 1;
      gitRefreshSeq.set(sessionId, nextSeq);
      const isLatest = () => (gitRefreshSeq.get(sessionId) ?? 0) === nextSeq;

      gitRefreshInFlight.add(sessionId);
      state.setGitStatusLoading(sessionId, true);
      void (async () => {
        try {
          const [branch, status] = await Promise.all([getGitBranch(cwd), gitStatus(cwd)]);
          if (!isLatest()) return;
          state.updateGitBranch(sessionId, branch);
          state.setGitStatus(sessionId, status);
        } catch {
          if (!isLatest()) return;
          state.updateGitBranch(sessionId, null);
          state.setGitStatus(sessionId, null);
        } finally {
          gitRefreshInFlight.delete(sessionId);
          if (isLatest()) state.setGitStatusLoading(sessionId, false);
        }
      })();
    }

    function clearProcessDetectionTimer(sessionId: string) {
      const timer = processDetectionTimers.get(sessionId);
      if (timer) {
        clearTimeout(timer);
        processDetectionTimers.delete(sessionId);
      }
    }

    // Command block events
    unlisteners.push(
      onEvent("command_block", (payload) => {
        if (isStale()) return;
        const { session_id, command, exit_code, event_type } = payload;
        const state = store.getState();

        switch (event_type) {
          case "prompt_start": {
            const pendingCommand = state.pendingCommand[session_id]?.command;
            const deferred = deferredExitCodes.get(session_id);
            trace("prompt_start", session_id, {
              pendingCommand,
              hasDeferred: !!deferred,
              seenCommandEnd: seenCommandEnd.has(session_id),
              lastConsumedAgoMs: lastPromptStartConsumedAt.has(session_id)
                ? Date.now() - (lastPromptStartConsumedAt.get(session_id) as number)
                : null,
            });

            if (deferred) {
              clearTimeout(deferred.fallbackTimer);
              deferredExitCodes.delete(session_id);
              lastPromptStartConsumedAt.set(session_id, Date.now());

              void (async () => {
                await new Promise((resolve) => setTimeout(resolve, 150));
                logger.debug("[output-trace] deferred block creation", {
                  session_id: session_id.slice(0, 8),
                  bufferSize: _drainOutputBufferSize(session_id),
                  exitCode: deferred.exitCode,
                });
                trace("prompt_start.deferred.flush", session_id, {
                  exitCode: deferred.exitCode,
                  bufferSize: _drainOutputBufferSize(session_id),
                });
                virtualTerminalManager.dispose(session_id);
                store.getState().handleCommandEnd(session_id, deferred.exitCode, deferred.endTime);
                store.getState().handlePromptStart(session_id);
              })();
            } else {
              // Guard 1 (dedupe): swallow a second prompt_start that lands
              // shortly after one that consumed a deferred command_end. The
              // async cleanup above is still in flight and will handle the
              // block creation; running handlePromptStart synchronously here
              // would race it.
              const consumedAt = lastPromptStartConsumedAt.get(session_id);
              const withinDedupeWindow =
                consumedAt !== undefined && Date.now() - consumedAt < PROMPT_START_DEDUPE_MS;
              // Guard 2 (first-prompt race): the very first prompt_start a
              // session receives, while pending is *already* armed with a
              // real command and no command_end has been processed yet, is
              // the PowerShell-on-Windows startup race — synthetic 133;C
              // armed pending before the shell's startup 133;A reached us.
              // Drop pending without producing a card; the real handleCommand
              // End that follows dir's 133;D will create the proper block.
              const isFirstPromptRace = !seenCommandEnd.has(session_id) && !!pendingCommand;

              if (withinDedupeWindow) {
                trace("prompt_start.suppress.dedupe", session_id, {
                  consumedAgoMs: consumedAt ? Date.now() - consumedAt : null,
                  pendingCommand,
                });
              } else if (isFirstPromptRace) {
                trace("prompt_start.suppress.firstPrompt", session_id, {
                  pendingCommand,
                });
                virtualTerminalManager.dispose(session_id);
                store.getState().discardPendingCommand(session_id);
              } else {
                trace("prompt_start.handle", session_id, { pendingCommand });
                virtualTerminalManager.dispose(session_id);
                state.handlePromptStart(session_id);
              }
            }

            lastStartedCommand.delete(session_id);
            const session = state.sessions[session_id];
            if (session?.renderMode === "fullterm") {
              if (pendingCommand) {
                logger.debug("[fullterm] Exiting fullterm for command:", pendingCommand);
              }
              state.setRenderMode(session_id, "timeline");
            }
            break;
          }
          case "prompt_end":
            trace("prompt_end", session_id);
            state.handlePromptEnd(session_id);
            break;
          case "command_start": {
            trace("command_start", session_id, {
              command,
              pendingBefore: state.pendingCommand[session_id]?.command,
              seenCommandEnd: seenCommandEnd.has(session_id),
            });
            state.handleCommandStart(session_id, command);
            lastStartedCommand.set(session_id, command);
            usedAlternateScreen.set(session_id, false);
            virtualTerminalManager.create(session_id);

            // Per-command fullterm activation (the old
            // `fulltermCommands` allowlist for `claude` / `codex` /
            // etc.) is intentionally not done here any more. Fullterm
            // is now driven exclusively by the `alternate_screen`
            // handler below, gated on `ALT_SCREEN_TUI_PROCESSES`, so
            // command-name guesses can't false-trigger a full xterm
            // takeover.
            const processName = extractProcessName(command);

            // Fast-path for known TUI apps (vim / htop / less / tmux / …).
            // The `setTimeout(…, PROCESS_DETECTION_DELAY_MS)` below
            // confirms the foreground process via
            // `ptyGetForegroundProcess` before committing the name, but
            // vim trips `\x1b[?1049h` (alt-screen enable) within a
            // handful of milliseconds — far faster than the detection
            // timer fires. Without this fast path the `alternate_screen`
            // handler reads `session.processName === null`, misses the
            // whitelist, leaves us in timeline mode, and vim's full-
            // screen ANSI gets dumped into the scrollback as garbled
            // `t;4;2m` / `▽` / `~` fragments. Whitelist-matched commands
            // are safe to claim immediately — if the foreground process
            // turns out to be something else (e.g. `vim` aliased to a
            // wrapper that exec's `cat`), the `setRenderMode("timeline")`
            // on the alt-screen-disable path will recover.
            if (processName && ALT_SCREEN_TUI_PROCESSES.has(processName)) {
              state.setProcessName(session_id, processName);
            }

            if (isFastCommand(command)) break;

            clearProcessDetectionTimer(session_id);

            const timer = setTimeout(async () => {
              try {
                const osProcess = await ptyGetForegroundProcess(session_id);
                if (!osProcess || SHELL_PROCESSES.has(osProcess)) return;
                if (processName) state.setProcessName(session_id, processName);
              } catch (err) {
                logger.debug("Failed to verify foreground process:", err);
              } finally {
                processDetectionTimers.delete(session_id);
              }
            }, PROCESS_DETECTION_DELAY_MS);

            processDetectionTimers.set(session_id, timer);
            break;
          }
          case "command_end": {
            trace("command_end", session_id, {
              command,
              exit_code,
              pendingCommand: state.pendingCommand[session_id]?.command,
            });
            // Mark this session as having completed at least one command so
            // the first-prompt race guard above stops suppressing further
            // prompt_start events.
            seenCommandEnd.add(session_id);

            // History persistence priority: prefer the value that already
            // went through `handleCommandStart`'s
            // `lastSentCommand`-preferring resolution (see Bug-1 fix in
            // `session-terminal.ts`). The raw `lastStartedCommand` cache
            // only holds the bash `BASH_COMMAND` payload, which collapses
            // multi-line compounds (`select X; do\n echo $y\n break\n
            // done` → `select X; doecho $ybreakdone`) and used to poison
            // the persisted history feeding `useCommandHistory` — a
            // pressed-Up arrow would then re-execute the mangled string
            // forever.
            const commandText =
              state.pendingCommand[session_id]?.command ??
              command ??
              lastStartedCommand.get(session_id) ??
              null;

            if (exit_code !== null) {
              const wasFulltermApp = usedAlternateScreen.get(session_id) ?? false;
              usedAlternateScreen.delete(session_id);

              if (wasFulltermApp) {
                state.setPendingOutput(session_id, "");
                state.handleCommandEnd(session_id, exit_code);
              } else {
                const prev = deferredExitCodes.get(session_id);
                if (prev) clearTimeout(prev.fallbackTimer);
                const endTime = Date.now();
                const fallbackTimer = setTimeout(() => {
                  deferredExitCodes.delete(session_id);
                  virtualTerminalManager.dispose(session_id);
                  store.getState().handleCommandEnd(session_id, exit_code, endTime);
                }, 2000);
                deferredExitCodes.set(session_id, { exitCode: exit_code, endTime, fallbackTimer });
              }

              if (commandText) {
                addCommandHistory(session_id, commandText, exit_code).catch((err) => {
                  logger.debug("Failed to save command history:", err);
                });
              }
            }

            const commandForRefresh =
              command ??
              lastStartedCommand.get(session_id) ??
              state.pendingCommand[session_id]?.command;

            if (exit_code === 0 && shouldRefreshGitInfo(commandForRefresh ?? null)) {
              const cwd = state.sessions[session_id]?.workingDirectory;
              if (cwd) refreshGitInfo(session_id, cwd);
            }

            clearProcessDetectionTimer(session_id);
            state.setProcessName(session_id, null);
            // Command is over — leave Warp-style interactive input mode
            // so the bottom box goes back to "type a new command"
            // behaviour. Safe to call when the session was never in
            // interactive mode (the setter no-ops on null→null).
            state.setInteractiveMode(session_id, null);
            break;
          }
        }
      })
    );

    unlisteners.push(
      onEvent("terminal_output", (payload) => {
        if (isStale()) return;
        const { session_id, data } = payload;
        logger.debug("[output-trace] terminal_output received", {
          session_id: session_id.slice(0, 8),
          bytes: data.length,
          hasDeferredEnd: deferredExitCodes.has(session_id),
        });
        // Default-on raw-byte dump for the PowerShell-on-Windows `dir`
        // collapse bug. First N events per session get console.log'd so
        // we can correlate with the backend `[pty-dump]` lines without
        // needing to ferry env vars or localStorage flags through the
        // user's setup. After the per-session cap is hit, this goes
        // quiet for the rest of the page lifetime. Set
        // `localStorage.QBIT_PTY_TRACE_DISABLE = '1'` to opt out.
        try {
          const traceDisabled =
            typeof window !== "undefined" &&
            window.localStorage &&
            window.localStorage.getItem("QBIT_PTY_TRACE_DISABLE") === "1";
          const seenSoFar = ptyTraceEventsBySession.get(session_id) ?? 0;
          if (!traceDisabled && seenSoFar < PTY_TRACE_CAP_PER_SESSION) {
            ptyTraceEventsBySession.set(session_id, seenSoFar + 1);
            const MAX_PREVIEW = 512;
            const preview = data.length > MAX_PREVIEW ? data.slice(0, MAX_PREVIEW) : data;
            const hex = Array.from(preview)
              .map((c) => c.charCodeAt(0).toString(16).padStart(2, "0"))
              .join("");
            // eslint-disable-next-line no-console
            console.log(
              `[pty-trace] sid=${session_id.slice(0, 8)} seq=${seenSoFar + 1} len=${data.length} json=`,
              JSON.stringify(preview),
              "hex=",
              hex
            );
          }
        } catch {
          // localStorage can throw in private mode / Webview restrictions —
          // the trace is best-effort, never let it break output handling.
        }
        virtualTerminalManager.write(session_id, data);
        store.getState().appendOutput(session_id, data);
      })
    );

    unlisteners.push(
      onEvent("directory_changed", async (payload) => {
        if (isStale()) return;
        const { session_id, path } = payload;
        const state = store.getState();

        state.updateWorkingDirectory(session_id, path);

        try {
          const branch = await getGitBranch(path);
          state.updateGitBranch(session_id, branch);
        } catch {
          state.updateGitBranch(session_id, null);
        }

        try {
          const initialized = await isAiSessionInitialized(session_id);
          if (initialized) {
            await updateAiWorkspace(path, session_id);
            notify.info("Workspace synced", { message: path });
          }
        } catch (error) {
          logger.error("Error updating AI workspace:", error);
        }
      })
    );

    unlisteners.push(
      onEvent("virtual_env_changed", (payload) => {
        if (isStale()) return;
        store.getState().updateVirtualEnv(payload.session_id, payload.name);
      })
    );

    unlisteners.push(
      onEvent("session_ended", (payload) => {
        if (isStale()) return;
        trace("session_ended", payload.sessionId);
        seenCommandEnd.delete(payload.sessionId);
        lastPromptStartConsumedAt.delete(payload.sessionId);
        store.getState().removeSession(payload.sessionId);
      })
    );

    unlisteners.push(
      onEvent("alternate_screen", (payload) => {
        if (isStale()) return;
        const { session_id, enabled } = payload;
        const state = store.getState();

        // Disable side: always honour — even non-TUI processes will
        // toggle the alt-screen flag off when they exit, and we want
        // to be back in Block UI by the time they do.
        if (!enabled) {
          state.setRenderMode(session_id, "timeline");
          return;
        }

        // Enable side: only flip into fullterm when the foreground
        // process is a known TUI (vim / htop / less / nano / …). Any
        // other alt-screen toggle (a misbehaving pager, an incidental
        // cursor-visibility flip during a Y/N prompt) is treated as
        // noise and kept in Block UI so the Warp-style interactive
        // input keeps working.
        const session = state.sessions[session_id];
        const processName = session?.processName ?? null;
        if (processName && ALT_SCREEN_TUI_PROCESSES.has(processName)) {
          state.setRenderMode(session_id, "fullterm");
          usedAlternateScreen.set(session_id, true);
        } else {
          logger.debug("[fullterm] alt-screen ignored (non-TUI foreground)", {
            session_id,
            processName,
          });
        }
      })
    );

    unlisteners.push(
      onEvent("stdin_wait", (payload) => {
        if (isStale()) return;
        const { session_id, detector } = payload;
        const state = store.getState();
        const session = state.sessions[session_id];
        if (!session) return;

        // Don't override fullterm mode — vim/htop sessions handle
        // their own keystrokes directly, the bottom Warp input box
        // is hidden anyway.
        if (session.renderMode === "fullterm") return;

        // Resolve the command label shown in the Warp-style cell.
        // We prefer OSC 133;C's `command` field, fall back to the
        // most recent `command_start` event, and finally to whatever
        // the user last submitted from the input box. The last
        // fallback matters in zsh / sh integrations that emit OSC
        // 133;C without a trailing `;<command>`: previously the cell
        // showed a generic "the running command" placeholder, which
        // is useless when several commands are queued back-to-back.
        const command =
          state.pendingCommand[session_id]?.command ??
          lastStartedCommand.get(session_id) ??
          state.lastSentCommand[session_id] ??
          null;

        state.setInteractiveMode(session_id, {
          active: true,
          command,
          detector: normaliseStdinWaitDetector(detector),
          enteredAt: Date.now(),
        });
      })
    );

    // Periodic git status refresh
    const gitStatusPollInterval = setInterval(() => {
      const state = store.getState();
      for (const sessionId of Object.keys(state.sessions)) {
        const session = state.sessions[sessionId];
        if (session?.workingDirectory) refreshGitInfo(sessionId, session.workingDirectory);
      }
    }, GIT_STATUS_POLL_INTERVAL_MS);

    // Cleanup
    return () => {
      for (const timer of processDetectionTimers.values()) clearTimeout(timer);
      processDetectionTimers.clear();
      for (const { fallbackTimer } of deferredExitCodes.values()) clearTimeout(fallbackTimer);
      deferredExitCodes.clear();
      seenCommandEnd.clear();
      lastPromptStartConsumedAt.clear();
      clearInterval(gitStatusPollInterval);
      Promise.all(
        unlisteners.map((p) =>
          p.then((unlisten) => {
            runTauriUnlistenFn(unlisten);
          })
        )
      ).catch((err) => {
        logger.warn("Failed to unlisten from some events:", err);
      });
    };
  }, []);
}
