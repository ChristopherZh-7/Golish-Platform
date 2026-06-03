import type { CompactionResult, FinalizedStreamingBlock } from "./streaming";
import type { ActiveSubAgent } from "./sub-agent";
import type { ToolCall, ToolCallSource } from "./tool-call";
import type { ActiveWorkflow } from "./workflow";

export interface CommandBlock {
  id: string;
  sessionId: string;
  command: string;
  output: string;
  exitCode: number | null;
  startTime: string;
  durationMs: number | null;
  workingDirectory: string;
  isCollapsed: boolean;
}

export interface AgentMessage {
  id: string;
  sessionId: string;
  role: "user" | "assistant" | "system";
  content: string;
  timestamp: string;
  isStreaming?: boolean;
  attachments?: {
    type: "image";
    data: string;
    media_type?: string;
    filename?: string;
  }[];
  toolCalls?: ToolCall[];
  streamingHistory?: FinalizedStreamingBlock[];
  thinkingContent?: string;
  workflow?: ActiveWorkflow;
  subAgents?: ActiveSubAgent[];
  systemHooks?: string[];
  inputTokens?: number;
  outputTokens?: number;
  compaction?: CompactionResult;
  workingDirectory?: string;
}

export interface AiToolExecution {
  requestId: string;
  toolName: string;
  args: Record<string, unknown>;
  toolIntent?: {
    modelWanted: string;
    source: "native_tool_call" | "textual_xml" | "textual_json" | "recovered";
    decision: "allow" | "require_approval" | "require_human_answer" | "reject";
    reason?: string;
    rawPreview?: string;
  };
  status: "running" | "backgrounded" | "completed" | "error" | "interrupted";
  result?: unknown;
  startedAt: string;
  completedAt?: string;
  durationMs?: number;
  autoApproved?: boolean;
  riskLevel?: string;
  streamingOutput?: string;
  source?: ToolCallSource;
  planStepIndex?: number;
  planStepId?: string;
}

export interface PendingCommand {
  command: string | null;
  output: string;
  startTime: string;
  workingDirectory: string;
}
