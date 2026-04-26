/**
 * Session streaming-block actions: tool blocks, udiff results, system hooks,
 * and streaming output.
 */

import type { ToolCallSource } from "../store-types";
import type { ImmerSet, StateGet } from "./types";

export function createSessionStreamingActions(set: ImmerSet<any>, _get: StateGet<any>) {
  return {
    addStreamingToolBlock: (
      sessionId: string,
      toolCall: {
        id: string;
        name: string;
        args: Record<string, unknown>;
        executedByAgent?: boolean;
        source?: ToolCallSource;
      },
    ) =>
      set((state: any) => {
        if (!state.streamingBlocks[sessionId]) {
          state.streamingBlocks[sessionId] = [];
        }
        state.streamingBlocks[sessionId].push({
          type: "tool",
          toolCall: {
            ...toolCall,
            status: "running",
            startedAt: new Date().toISOString(),
          },
        });
      }),

    updateStreamingToolBlock: (
      sessionId: string,
      toolId: string,
      success: boolean,
      result?: unknown,
    ) =>
      set((state: any) => {
        const blocks = state.streamingBlocks[sessionId];
        if (blocks) {
          for (const block of blocks) {
            if (block.type === "tool" && block.toolCall.id === toolId) {
              block.toolCall.status = success ? "completed" : "error";
              block.toolCall.result = result;
              block.toolCall.completedAt = new Date().toISOString();
              break;
            }
          }
          state.streamingBlockRevision[sessionId] =
            (state.streamingBlockRevision[sessionId] ?? 0) + 1;
        }
      }),

    clearStreamingBlocks: (sessionId: string) =>
      set((state: any) => {
        state.streamingBlocks[sessionId] = [];
      }),

    addUdiffResultBlock: (sessionId: string, response: string, durationMs: number) =>
      set((state: any) => {
        if (!state.streamingBlocks[sessionId]) {
          state.streamingBlocks[sessionId] = [];
        }
        state.streamingBlocks[sessionId].push({
          type: "udiff_result",
          response,
          durationMs,
        });
      }),

    addStreamingSystemHooksBlock: (sessionId: string, hooks: string[]) =>
      set((state: any) => {
        if (!state.streamingBlocks[sessionId]) {
          state.streamingBlocks[sessionId] = [];
        }
        state.streamingBlocks[sessionId].push({
          type: "system_hooks",
          hooks,
        });
      }),

    appendToolStreamingOutput: (sessionId: string, toolId: string, chunk: string) =>
      set((state: any) => {
        const tools = state.activeToolCalls?.[sessionId];
        if (tools) {
          const toolIndex = tools.findIndex((t: any) => t.id === toolId);
          if (toolIndex !== -1) {
            tools[toolIndex] = {
              ...tools[toolIndex],
              streamingOutput: (tools[toolIndex].streamingOutput ?? "") + chunk,
            };
          }
        }
        const blocks = state.streamingBlocks[sessionId];
        if (blocks) {
          for (let i = 0; i < blocks.length; i++) {
            const block = blocks[i];
            if (block.type === "tool" && block.toolCall.id === toolId) {
              blocks[i] = {
                ...block,
                toolCall: {
                  ...block.toolCall,
                  streamingOutput: (block.toolCall.streamingOutput ?? "") + chunk,
                },
              };
              break;
            }
          }
          state.streamingBlockRevision[sessionId] =
            (state.streamingBlockRevision[sessionId] ?? 0) + 1;
        }
      }),
  };
}
