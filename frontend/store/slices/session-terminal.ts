/**
 * Session terminal actions: command lifecycle, output handling, and timeline
 * operations.
 */

import { sendNotification } from "@/lib/systemNotifications";
import { logger } from "@/lib/logger";
import type { CommandBlock } from "../store-types";
import {
  _drainOutputBuffer,
  deleteOutputBuffer,
  getOutputBuffer,
  getOwningTabIdFromState,
  markTabNewActivityInDraft,
  MAX_OUTPUT_BUFFER_BYTES,
  setOutputBuffer,
} from "./session-helpers";
import type { ImmerSet, StateGet } from "./types";

export function createSessionTerminalActions(set: ImmerSet<any>, get: StateGet<any>) {
  return {
    handlePromptStart: (sessionId: string) => {
      const drainedOutput = _drainOutputBuffer(sessionId);
      set((state: any) => {
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
          const source = state.pipelineCommandSource[sessionId]
            ? ("pipeline" as const)
            : undefined;
          state.timelines[sessionId].push({
            id: blockId,
            type: "command",
            timestamp: new Date().toISOString(),
            data: { ...block, source },
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

    handleCommandStart: (sessionId: string, command: string | null) => {
      deleteOutputBuffer(sessionId);
      set((state: any) => {
        const session = state.sessions[sessionId];
        const effectiveCommand = command || state.lastSentCommand[sessionId] || null;
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
      const currentState = get() as any;
      const pending = currentState.pendingCommand[sessionId];
      const command = pending?.command;
      const session = currentState.sessions[sessionId];
      const isFullterm = session?.renderMode === "fullterm";
      const shouldNotify = pending && command && !isFullterm;
      const drainedOutput = _drainOutputBuffer(sessionId);
      const owningTabId = shouldNotify ? getOwningTabIdFromState(currentState, sessionId) : null;

      set((state: any) => {
        const pending = state.pendingCommand[sessionId];
        if (pending) {
          const session = state.sessions[sessionId];
          const isFullterm = session?.renderMode === "fullterm";

          if (pending.command && !isFullterm) {
            const blockId = crypto.randomUUID();
            const currentWorkingDir = session?.workingDirectory || pending.workingDirectory;
            const block: CommandBlock = {
              id: blockId,
              sessionId,
              command: pending.command,
              output: drainedOutput,
              exitCode,
              startTime: pending.startTime,
              durationMs: (endTime ?? Date.now()) - new Date(pending.startTime).getTime(),
              workingDirectory: currentWorkingDir,
              isCollapsed: false,
            };

            if (!state.timelines[sessionId]) {
              state.timelines[sessionId] = [];
            }
            const source = state.pipelineCommandSource[sessionId]
              ? ("pipeline" as const)
              : undefined;
            state.timelines[sessionId].push({
              id: blockId,
              type: "command",
              timestamp: new Date().toISOString(),
              data: { ...block, source },
            });
            if (source === "pipeline") {
              state.pipelineCommandSource[sessionId] = false;
            }
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
          }),
        );
      }
    },

    appendOutput: (sessionId: string, data: string) => {
      let current = (getOutputBuffer(sessionId)) + data;
      if (current.length > MAX_OUTPUT_BUFFER_BYTES * 2) {
        current = current.slice(current.length - MAX_OUTPUT_BUFFER_BYTES);
      }
      setOutputBuffer(sessionId, current);
      if (!(get() as any).pendingCommand[sessionId]) {
        set((state: any) => {
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
      set((state: any) => {
        for (const timeline of Object.values<any[]>(state.timelines)) {
          const unifiedBlock = timeline.find(
            (b: any) => b.type === "command" && b.id === blockId,
          );
          if (unifiedBlock && unifiedBlock.type === "command") {
            unifiedBlock.data.isCollapsed = !unifiedBlock.data.isCollapsed;
            break;
          }
        }
      }),

    setLastSentCommand: (sessionId: string, command: string | null) =>
      set((state: any) => {
        state.lastSentCommand[sessionId] = command;
      }),

    clearBlocks: (sessionId: string) => {
      deleteOutputBuffer(sessionId);
      set((state: any) => {
        const timeline = state.timelines[sessionId];
        if (timeline) {
          state.timelines[sessionId] = timeline.filter(
            (block: any) => block.type !== "command",
          );
        }
        state.pendingCommand[sessionId] = null;
      });
    },

    requestTerminalClear: (sessionId: string) =>
      set((state: any) => {
        state.terminalClearRequest[sessionId] =
          (state.terminalClearRequest[sessionId] ?? 0) + 1;
      }),

    setPipelineCommandSource: (sessionId: string, isPipeline: boolean) =>
      set((state: any) => {
        state.pipelineCommandSource[sessionId] = isPipeline;
      }),

    // --- Timeline helpers ---

    addSystemHookBlock: (sessionId: string, hooks: string[]) =>
      set((state: any) => {
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
      set((state: any) => {
        state.timelines[sessionId] = [];
        state.pendingCommand[sessionId] = null;
        if (state.agentStreamingBuffer) state.agentStreamingBuffer[sessionId] = [];
        if (state.agentStreaming) state.agentStreaming[sessionId] = "";
        state.streamingBlocks[sessionId] = [];
      });
    },
  };
}
