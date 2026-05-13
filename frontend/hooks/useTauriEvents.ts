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
import { liveTerminalManager, virtualTerminalManager } from "@/lib/terminal";
import { _drainOutputBufferSize, useStore } from "@/store";
import {
  BUILTIN_FULLTERM_COMMANDS,
  extractProcessName,
  GIT_STATUS_POLL_INTERVAL_MS,
  isFastCommand,
  PROCESS_DETECTION_DELAY_MS,
  SHELL_PROCESSES,
  shouldRefreshGitInfo,
} from "./tauri-event-types";

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

    let fulltermCommands = new Set(BUILTIN_FULLTERM_COMMANDS);

    getSettings()
      .then((settings) => {
        const userCommands = settings.terminal.fullterm_commands ?? [];
        fulltermCommands = new Set([...BUILTIN_FULLTERM_COMMANDS, ...userCommands]);
      })
      .catch((err) => {
        logger.debug("Failed to load settings for fullterm commands:", err);
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
              liveTerminalManager.scrollToBottom(session_id);
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
                liveTerminalManager.dispose(session_id);
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
                liveTerminalManager.scrollToBottom(session_id);
                liveTerminalManager.dispose(session_id);
                store.getState().discardPendingCommand(session_id);
              } else {
                trace("prompt_start.handle", session_id, { pendingCommand });
                virtualTerminalManager.dispose(session_id);
                liveTerminalManager.scrollToBottom(session_id);
                liveTerminalManager.dispose(session_id);
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

            const processName = extractProcessName(command);
            if (processName && fulltermCommands.has(processName)) {
              logger.debug("[fullterm] Switching to fullterm mode for", {
                session_id,
                processName,
              });
              state.setRenderMode(session_id, "fullterm");
              usedAlternateScreen.set(session_id, true);
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

            const commandText =
              command ??
              lastStartedCommand.get(session_id) ??
              state.pendingCommand[session_id]?.command ??
              null;

            if (exit_code !== null) {
              const wasFulltermApp = usedAlternateScreen.get(session_id) ?? false;
              usedAlternateScreen.delete(session_id);

              if (wasFulltermApp) {
                liveTerminalManager.dispose(session_id);
                state.setPendingOutput(session_id, "");
                state.handleCommandEnd(session_id, exit_code);
              } else {
                const prev = deferredExitCodes.get(session_id);
                if (prev) clearTimeout(prev.fallbackTimer);
                const endTime = Date.now();
                const fallbackTimer = setTimeout(() => {
                  deferredExitCodes.delete(session_id);
                  virtualTerminalManager.dispose(session_id);
                  liveTerminalManager.dispose(session_id);
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
        virtualTerminalManager.write(session_id, data);
        liveTerminalManager.write(session_id, data);
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
        store.getState().setRenderMode(session_id, enabled ? "fullterm" : "timeline");
        if (enabled) usedAlternateScreen.set(session_id, true);
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
