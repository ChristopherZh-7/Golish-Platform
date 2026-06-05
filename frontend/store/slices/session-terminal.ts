/**
 * Session terminal actions: command lifecycle, output handling, and timeline
 * operations.
 */

import { ALT_SCREEN_TUI_PROCESSES, extractProcessName } from "@/hooks/tauri-event-types";
import { logger } from "@/lib/logger";
import { sendNotification } from "@/lib/systemNotifications";
import type { CommandBlock } from "../store-types";
import type { SessionStoreDraft } from "./session-draft-types";
import {
  _drainOutputBuffer,
  deleteOutputBuffer,
  getOutputBuffer,
  getOwningTabIdFromState,
  MAX_OUTPUT_BUFFER_BYTES,
  markTabNewActivityInDraft,
  setOutputBuffer,
} from "./session-helpers";
import type { ImmerSet, StateGet } from "./types";

/**
 * Phase C · Warp-style TUI block folding: detect whether the command
 * that just finished is something whose alt-screen output is restored
 * on exit (vim, htop, less, …). Those blocks open *collapsed* in the
 * timeline since their `output` is rarely useful in static form.
 *
 * Mirrors the `ALT_SCREEN_TUI_PROCESSES` allowlist used by
 * `useTauriEvents.ts` to gate auto-fullterm; sharing the set keeps the
 * two policies — "render this through alt-screen" and "collapse this
 * after it exits" — perfectly aligned.
 */
function detectTuiCommand(command: string | null): boolean {
  const processName = extractProcessName(command);
  return processName !== null && ALT_SCREEN_TUI_PROCESSES.has(processName);
}

export function createSessionTerminalActions(
  set: ImmerSet<SessionStoreDraft>,
  get: StateGet<SessionStoreDraft>
) {
  return {
    handlePromptStart: (sessionId: string) => {
      const drainedOutput = _drainOutputBuffer(sessionId);
      set((state) => {
        const pending = state.pendingCommand[sessionId];
        if (pending?.command) {
          const session = state.sessions[sessionId];
          if (session?.renderMode === "fullterm") {
            state.pendingCommand[sessionId] = null;
            return;
          }
          const currentWorkingDir = session?.workingDirectory || pending.workingDirectory;
          const blockId = crypto.randomUUID();
          const block: CommandBlock = {
            id: blockId,
            sessionId,
            command: pending.command,
            output: drainedOutput,
            exitCode: null,
            startTime: pending.startTime,
            durationMs: null,
            workingDirectory: currentWorkingDir,
            isCollapsed: false,
          };

          if (!state.timelines[sessionId]) {
            state.timelines[sessionId] = [];
          }
          state.timelines[sessionId].push({
            id: blockId,
            type: "command",
            timestamp: new Date().toISOString(),
            data: { ...block },
          });
        }

        if (pending) {
          markTabNewActivityInDraft(state, sessionId);
        }
        state.pendingCommand[sessionId] = null;
      });
    },

    handlePromptEnd: (_sessionId: string) => {
      // Ready for input - nothing to do for now
    },

    discardPendingCommand: (sessionId: string) => {
      // Drop pending without producing a timeline block, and preserve the
      // output buffer so the eventual handleCommandEnd has the full bytes.
      // See SessionActions.discardPendingCommand for the Windows-specific
      // race this guards against.
      set((state) => {
        if (state.pendingCommand[sessionId]) {
          state.pendingCommand[sessionId] = null;
        }
      });
    },

    handleCommandStart: (sessionId: string, command: string | null) => {
      deleteOutputBuffer(sessionId);
      set((state) => {
        const session = state.sessions[sessionId];
        const userTyped = state.lastSentCommand[sessionId];
        // Bash's DEBUG trap exposes `BASH_COMMAND`, which for a
        // multi-line compound like
        //   select yn in "Yes" "No"; do
        //     echo $yn
        //     break
        //   done
        // collapses into `select yn in "Yes" "No"; doecho $ynbreakdone`
        // — newlines AND inter-token whitespace are dropped. That
        // mangled string is what OSC 133;C carries to the frontend,
        // and what we'd otherwise show in the RunningCommandCard /
        // InteractiveCell header. When the user typed the command
        // through the bottom Warp input box we already have the
        // pristine text in `lastSentCommand`; prefer it whenever its
        // alphanumeric skeleton matches the OSC value so the timeline
        // keeps the original multi-line layout. Commands coming from
        // arrow-up history re-runs (no `lastSentCommand`) or shell-
        // initiated execution (`source ~/.bashrc`, completions, etc.)
        // still fall through to the OSC payload unchanged.
        const skeleton = (s: string | null | undefined) => s?.replace(/\s+/g, "").trim() ?? "";
        const useUserTyped = !!userTyped && !!command && skeleton(userTyped) === skeleton(command);
        const effectiveCommand = useUserTyped ? userTyped : command || userTyped || null;
        state.pendingCommand[sessionId] = {
          command: effectiveCommand,
          output: "",
          startTime: new Date().toISOString(),
          workingDirectory: session?.workingDirectory || "",
        };
        state.lastSentCommand[sessionId] = null;
      });
    },

    handleCommandEnd: (sessionId: string, exitCode: number, endTime?: number) => {
      const currentState = get();
      const pending = currentState.pendingCommand[sessionId];
      const command = pending?.command;
      const session = currentState.sessions[sessionId];
      const isFullterm = session?.renderMode === "fullterm";
      const shouldNotify = pending && command && !isFullterm;
      const drainedOutput = _drainOutputBuffer(sessionId);
      const owningTabId = shouldNotify ? getOwningTabIdFromState(currentState, sessionId) : null;

      set((state) => {
        const pending = state.pendingCommand[sessionId];
        if (pending) {
          const session = state.sessions[sessionId];
          const isFullterm = session?.renderMode === "fullterm";

          if (pending.command && !isFullterm) {
            const blockId = crypto.randomUUID();
            const currentWorkingDir = session?.workingDirectory || pending.workingDirectory;
            // Phase C · TUI commands (vim, htop, less, …) default to
            // collapsed because their alt-screen output is restored on
            // exit — keeping the block expanded would leave a row of
            // dangling escape sequences in the timeline. Matches
            // Warp's behaviour: a single line `~ (Xs) vim foo.txt`
            // that the user can re-expand if they want to inspect the
            // (mostly empty) static output. Detection mirrors the
            // `ALT_SCREEN_TUI_PROCESSES` set used by the alt-screen
            // gating in `useTauriEvents.ts`.
            const isTuiCommand = detectTuiCommand(pending.command);
            const block: CommandBlock = {
              id: blockId,
              sessionId,
              command: pending.command,
              output: drainedOutput,
              exitCode,
              startTime: pending.startTime,
              durationMs: (endTime ?? Date.now()) - new Date(pending.startTime).getTime(),
              workingDirectory: currentWorkingDir,
              isCollapsed: isTuiCommand,
            };

            if (!state.timelines[sessionId]) {
              state.timelines[sessionId] = [];
            }
            state.timelines[sessionId].push({
              id: blockId,
              type: "command",
              timestamp: new Date().toISOString(),
              data: { ...block },
            });
          }

          markTabNewActivityInDraft(state, sessionId);

          state.pendingCommand[sessionId] = null;
        }
      });

      if (shouldNotify && owningTabId) {
        const exitStatus = exitCode === 0 ? "✓" : `✗ ${exitCode}`;
        sendNotification({
          title: "Command completed",
          body: `${exitStatus} ${command}`,
          tabId: owningTabId,
        }).catch((err) => {
          logger.debug("Failed to send command notification:", err);
        });
      }

      if (pending && command && pending.output) {
        window.dispatchEvent(
          new CustomEvent("tool-output-completed", {
            detail: { command, output: pending.output, sessionId },
          })
        );
      }
    },

    appendOutput: (sessionId: string, data: string) => {
      let current = getOutputBuffer(sessionId) + data;
      if (current.length > MAX_OUTPUT_BUFFER_BYTES * 2) {
        current = current.slice(current.length - MAX_OUTPUT_BUFFER_BYTES);
      }
      setOutputBuffer(sessionId, current);
      if (!get().pendingCommand[sessionId]) {
        set((state) => {
          if (state.pendingCommand[sessionId]) return;
          const session = state.sessions[sessionId];
          state.pendingCommand[sessionId] = {
            command: null,
            output: "",
            startTime: new Date().toISOString(),
            workingDirectory: session?.workingDirectory || "",
          };
        });
      }
    },

    setPendingOutput: (sessionId: string, output: string) => {
      setOutputBuffer(sessionId, output);
    },

    toggleBlockCollapse: (blockId: string) =>
      set((state) => {
        for (const timeline of Object.values(state.timelines)) {
          const unifiedBlock = timeline.find((b) => b.type === "command" && b.id === blockId);
          if (unifiedBlock && unifiedBlock.type === "command") {
            unifiedBlock.data.isCollapsed = !unifiedBlock.data.isCollapsed;
            break;
          }
        }
      }),

    setLastSentCommand: (sessionId: string, command: string | null) =>
      set((state) => {
        state.lastSentCommand[sessionId] = command;
      }),

    clearBlocks: (sessionId: string) => {
      deleteOutputBuffer(sessionId);
      set((state) => {
        const timeline = state.timelines[sessionId];
        if (timeline) {
          state.timelines[sessionId] = timeline.filter((block) => block.type !== "command");
        }
        state.pendingCommand[sessionId] = null;
      });
    },

    requestTerminalClear: (sessionId: string) =>
      set((state) => {
        state.terminalClearRequest[sessionId] = (state.terminalClearRequest[sessionId] ?? 0) + 1;
      }),

    // --- Timeline helpers ---

    addSystemHookBlock: (sessionId: string, hooks: string[]) =>
      set((state) => {
        if (!state.timelines[sessionId]) {
          state.timelines[sessionId] = [];
        }
        state.timelines[sessionId].push({
          id: crypto.randomUUID(),
          type: "system_hook",
          timestamp: new Date().toISOString(),
          data: { hooks },
        });
      }),

    clearTimeline: (sessionId: string) => {
      deleteOutputBuffer(sessionId);
      set((state) => {
        state.timelines[sessionId] = [];
        state.pendingCommand[sessionId] = null;
        if (state.agentStreamingBuffer) state.agentStreamingBuffer[sessionId] = [];
        if (state.agentStreaming) state.agentStreaming[sessionId] = "";
        state.streamingBlocks[sessionId] = [];
      });
    },
  };
}
