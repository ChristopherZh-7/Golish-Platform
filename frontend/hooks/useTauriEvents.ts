import type { UnlistenFn } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { isAiSessionInitialized, updateAiWorkspace } from "@/lib/ai";
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
  isFastCommand,
  PROCESS_DETECTION_DELAY_MS,
  SHELL_PROCESSES,
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

            clearProcessDetectionTimer(session_id);
            state.setProcessName(session_id, null);
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

    unlisteners.push(
      onEvent("stdin_wait", (payload) => {
        if (isStale()) return;
        const { session_id, detector } = payload;
        const state = store.getState();
        const session = state.sessions[session_id];
        if (!session || session.renderMode === "fullterm") return;
        if (!state.pendingCommand[session_id]?.command) return;

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

    // Cleanup
    return () => {
      for (const timer of processDetectionTimers.values()) clearTimeout(timer);
      processDetectionTimers.clear();
      for (const { fallbackTimer } of deferredExitCodes.values()) clearTimeout(fallbackTimer);
      deferredExitCodes.clear();
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
