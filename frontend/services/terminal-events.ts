/**
 * Terminal event processing service.
 *
 * Extracts the business logic that was embedded in useTauriEvents hook.
 * This service can be used outside of React and is independently testable.
 */

import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  BUILTIN_FULLTERM_COMMANDS,
  extractProcessName,
  GIT_STATUS_POLL_INTERVAL_MS,
  isFastCommand,
  PROCESS_DETECTION_DELAY_MS,
  SHELL_PROCESSES,
  shouldRefreshGitInfo,
} from "@/hooks/tauri-event-types";
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
import { useStore } from "@/store";

export interface TerminalEventServiceState {
  isActive: boolean;
}

export function createTerminalEventService() {
  let generation = 0;
  const processDetectionTimers = new Map<string, ReturnType<typeof setTimeout>>();
  const usedAlternateScreen = new Map<string, boolean>();
  const deferredExitCodes = new Map<
    string,
    { exitCode: number; endTime: number; fallbackTimer: ReturnType<typeof setTimeout> }
  >();
  const gitRefreshSeq = new Map<string, number>();
  const gitRefreshInFlight = new Set<string>();
  const lastStartedCommand = new Map<string, string | null>();
  // Per-session timestamp of the last prompt_start that consumed a pending
  // deferred command_end. Used to suppress a stray *second* prompt_start that
  // arrives within a short window after the first one — see the PowerShell /
  // ConPTY race documented in the prompt_start handler below.
  const lastPromptStartConsumedAt = new Map<string, number>();
  const PROMPT_START_DEDUPE_MS = 300;
  // Per-session flag tracking whether we've seen at least one command_end on
  // this session. Used to guard against the *first* prompt_start emitted by a
  // freshly-spawned shell (zsh/bash/PowerShell all emit 133;A immediately on
  // startup before the user has run anything): on Windows the chat input box
  // can race the shell's startup, synthesise an OSC 133;C for the user's
  // command before the first 133;A reaches the front-end, and then have the
  // shell's startup 133;A turn the still-running command's pending entry
  // into an empty timeline card. While `seenCommandEnd` is false we discard
  // pending instead of converting it into a (necessarily empty) block.
  const seenCommandEnd = new Set<string>();
  let fulltermCommands = new Set(BUILTIN_FULLTERM_COMMANDS);
  let gitStatusPollInterval: ReturnType<typeof setInterval> | null = null;

  function refreshGitInfo(sessionId: string, cwd: string) {
    if (gitRefreshInFlight.has(sessionId)) return;

    const state = useStore.getState();
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

  async function start(): Promise<() => void> {
    const currentGen = ++generation;
    const isStale = () => currentGen !== generation;
    const store = useStore;
    const unlisteners: Promise<UnlistenFn>[] = [];

    try {
      const settings = await getSettings();
      const userCommands = settings.terminal.fullterm_commands ?? [];
      fulltermCommands = new Set([...BUILTIN_FULLTERM_COMMANDS, ...userCommands]);
    } catch (err) {
      logger.debug("Failed to load settings for fullterm commands:", err);
    }

    unlisteners.push(
      onEvent("command_block", (payload) => {
        if (isStale()) return;
        const { session_id, command, exit_code, event_type } = payload;
        const state = store.getState();

        switch (event_type) {
          case "prompt_start": {
            const pendingCommand = state.pendingCommand[session_id]?.command;
            const deferred = deferredExitCodes.get(session_id);

            if (deferred) {
              clearTimeout(deferred.fallbackTimer);
              deferredExitCodes.delete(session_id);
              liveTerminalManager.scrollToBottom(session_id);
              lastPromptStartConsumedAt.set(session_id, Date.now());

              void (async () => {
                await new Promise((resolve) => setTimeout(resolve, 150));
                virtualTerminalManager.dispose(session_id);
                liveTerminalManager.dispose(session_id);
                store.getState().handleCommandEnd(session_id, deferred.exitCode, deferred.endTime);
                store.getState().handlePromptStart(session_id);
              })();
            } else {
              // Dedupe: on Windows PowerShell with ConPTY we sometimes see a
              // *second* prompt_start hit within ~300 ms of the first one, in
              // which case the first prompt_start already scheduled an async
              // handleCommandEnd + handlePromptStart pair. Running another
              // handlePromptStart synchronously here would race the async
              // cleanup and create an empty CommandBlock (no exit code, no
              // duration) for the still-pending command — the "two cards on
              // first dir" bug. Skip when we're inside the dedupe window.
              const consumedAt = lastPromptStartConsumedAt.get(session_id);
              const withinDedupeWindow =
                consumedAt !== undefined && Date.now() - consumedAt < PROMPT_START_DEDUPE_MS;
              if (withinDedupeWindow) {
                logger.debug(
                  "[prompt_start] Suppressing duplicate prompt_start within dedupe window:",
                  session_id
                );
              } else if (!seenCommandEnd.has(session_id) && pendingCommand) {
                // First prompt_start on this session AND pending is already
                // armed with a real command — this is the
                // PowerShell-on-Windows startup race: the user clicked Send
                // before the shell's startup 133;A reached us, so the
                // synthetic 133;C set pending="dir" first and the shell's
                // very first prompt_start is now about to turn that pending
                // entry into an empty timeline card. Drop the pending without
                // producing a block; the real handleCommandEnd that follows
                // dir's 133;D will create the proper card with output + time.
                logger.debug(
                  "[prompt_start] Discarding pending on first-prompt race:",
                  session_id,
                  pendingCommand
                );
                virtualTerminalManager.dispose(session_id);
                liveTerminalManager.scrollToBottom(session_id);
                liveTerminalManager.dispose(session_id);
                store.getState().discardPendingCommand(session_id);
              } else {
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
            state.handlePromptEnd(session_id);
            break;

          case "command_start": {
            state.handleCommandStart(session_id, command);
            lastStartedCommand.set(session_id, command);
            usedAlternateScreen.set(session_id, false);
            virtualTerminalManager.create(session_id);

            const processName = extractProcessName(command);
            if (processName && fulltermCommands.has(processName)) {
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
            // Mark this session as having completed at least one command so
            // future prompt_start events fall through to handlePromptStart
            // (the first-prompt-on-startup guard above is keyed off this).
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
        // Clear per-session bookkeeping so a future session id reuse (rare,
        // but possible if tests recycle uuids) doesn't inherit stale guards.
        lastPromptStartConsumedAt.delete(payload.sessionId);
        seenCommandEnd.delete(payload.sessionId);
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

    gitStatusPollInterval = setInterval(() => {
      const state = store.getState();
      for (const sessionId of Object.keys(state.sessions)) {
        const session = state.sessions[sessionId];
        if (session?.workingDirectory) refreshGitInfo(sessionId, session.workingDirectory);
      }
    }, GIT_STATUS_POLL_INTERVAL_MS);

    return () => {
      for (const timer of processDetectionTimers.values()) clearTimeout(timer);
      processDetectionTimers.clear();
      for (const { fallbackTimer } of deferredExitCodes.values()) clearTimeout(fallbackTimer);
      deferredExitCodes.clear();
      lastPromptStartConsumedAt.clear();
      seenCommandEnd.clear();
      if (gitStatusPollInterval) clearInterval(gitStatusPollInterval);
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
  }

  return { start };
}

export type TerminalEventService = ReturnType<typeof createTerminalEventService>;
