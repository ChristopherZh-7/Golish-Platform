import type { UnlistenFn } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { isAiSessionInitialized, updateAiWorkspace } from "@/lib/ai";
import { addCommandHistory } from "@/lib/history";
import { logger } from "@/lib/logger";
import { notify } from "@/lib/notify";
import { getSettings } from "@/lib/settings";
import { getGitBranch, gitStatus, ptyGetForegroundProcess } from "@/lib/tauri";
import { runTauriUnlistenFn } from "@/lib/run-tauri-unlisten";
import { listen } from "@/lib/tauri-listen";
import { liveTerminalManager, virtualTerminalManager } from "@/lib/terminal";
import { useStore, _drainOutputBufferSize } from "@/store";
import {
  type AlternateScreenEvent,
  BUILTIN_FULLTERM_COMMANDS,
  type CommandBlockEvent,
  type DirectoryChangedEvent,
  GIT_STATUS_POLL_INTERVAL_MS,
  PROCESS_DETECTION_DELAY_MS,
  SHELL_PROCESSES,
  type SessionEndedEvent,
  type TerminalOutputEvent,
  type VirtualEnvChangedEvent,
  extractProcessName,
  isFastCommand,
  shouldRefreshGitInfo,
} from "./tauri-event-types";

let activeGeneration = 0;

export function useTauriEvents() {
  const store = useStore;

  // biome-ignore lint/correctness/useExhaustiveDependencies: store.getState is stable zustand API
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
      listen<CommandBlockEvent>("command_block", (event) => {
        if (isStale()) return;
        const { session_id, command, exit_code, event_type } = event.payload;
        const state = store.getState();

        switch (event_type) {
          case "prompt_start": {
            const pendingCommand = state.pendingCommand[session_id]?.command;
            const deferred = deferredExitCodes.get(session_id);

            if (deferred) {
              clearTimeout(deferred.fallbackTimer);
              deferredExitCodes.delete(session_id);
              liveTerminalManager.scrollToBottom(session_id);

              void (async () => {
                await new Promise((resolve) => setTimeout(resolve, 150));
                logger.debug("[output-trace] deferred block creation", {
                  session_id: session_id.slice(0, 8),
                  bufferSize: _drainOutputBufferSize(session_id),
                  exitCode: deferred.exitCode,
                });
                virtualTerminalManager.dispose(session_id);
                liveTerminalManager.dispose(session_id);
                store.getState().handleCommandEnd(session_id, deferred.exitCode, deferred.endTime);
                store.getState().handlePromptStart(session_id);
              })();
            } else {
              virtualTerminalManager.dispose(session_id);
              liveTerminalManager.scrollToBottom(session_id);
              liveTerminalManager.dispose(session_id);
              state.handlePromptStart(session_id);
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
              logger.debug("[fullterm] Switching to fullterm mode for", { session_id, processName });
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
            const commandText =
              command ?? lastStartedCommand.get(session_id) ?? state.pendingCommand[session_id]?.command ?? null;

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
              command ?? lastStartedCommand.get(session_id) ?? state.pendingCommand[session_id]?.command;

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

    // Terminal output
    unlisteners.push(
      listen<TerminalOutputEvent>("terminal_output", (event) => {
        if (isStale()) return;
        const { session_id, data } = event.payload;
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

    // Directory changed
    unlisteners.push(
      listen<DirectoryChangedEvent>("directory_changed", async (event) => {
        if (isStale()) return;
        const { session_id, path } = event.payload;
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

    // Virtual environment changed
    unlisteners.push(
      listen<VirtualEnvChangedEvent>("virtual_env_changed", (event) => {
        if (isStale()) return;
        store.getState().updateVirtualEnv(event.payload.session_id, event.payload.name);
      })
    );

    // Session ended
    unlisteners.push(
      listen<SessionEndedEvent>("session_ended", (event) => {
        if (isStale()) return;
        store.getState().removeSession(event.payload.sessionId);
      })
    );

    // Alternate screen buffer state changes
    unlisteners.push(
      listen<AlternateScreenEvent>("alternate_screen", (event) => {
        if (isStale()) return;
        const { session_id, enabled } = event.payload;
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
      clearInterval(gitStatusPollInterval);
      Promise.all(unlisteners.map((p) => p.then((unlisten) => { runTauriUnlistenFn(unlisten); }))).catch((err) => {
        logger.warn("Failed to unlisten from some events:", err);
      });
    };
  }, []);
}
