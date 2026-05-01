import type { ActiveToolCall, ToolCall } from "./tool-call";

export type StreamingBlock =
  | { type: "text"; content: string }
  | { type: "tool"; toolCall: ActiveToolCall }
  | { type: "udiff_result"; response: string; durationMs: number }
  | { type: "system_hooks"; hooks: string[] }
  | { type: "thinking"; content: string };

export type FinalizedStreamingBlock =
  | { type: "text"; content: string }
  | { type: "tool"; toolCall: ToolCall }
  | { type: "udiff_result"; response: string; durationMs: number }
  | { type: "system_hooks"; hooks: string[] }
  | { type: "thinking"; content: string };

export type CompactionResult =
  | {
      status: "success";
      tokensBefore: number;
      messagesBefore: number;
      messagesAfter: number;
      summaryLength: number;
      summary?: string;
      summarizerInput?: string;
    }
  | {
      status: "failed";
      tokensBefore: number;
      messagesBefore: number;
      error: string;
      summarizerInput?: string;
    };
