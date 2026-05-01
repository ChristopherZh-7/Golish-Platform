export interface SubAgentToolCall {
  id: string;
  name: string;
  args: Record<string, unknown>;
  status: "running" | "completed" | "error";
  result?: unknown;
  streamingOutput?: string;
  startedAt: string;
  completedAt?: string;
}

export interface SubAgentEntry {
  kind: "text" | "tool_call";
  text?: string;
  toolCallId?: string;
}

export interface ActiveSubAgent {
  agentId: string;
  agentName: string;
  parentRequestId: string;
  task: string;
  depth: number;
  status: "running" | "completed" | "error" | "interrupted";
  toolCalls: SubAgentToolCall[];
  entries: SubAgentEntry[];
  response?: string;
  error?: string;
  streamingText?: string;
  startedAt: string;
  completedAt?: string;
  durationMs?: number;
  promptGeneration?: {
    status: "generating" | "completed" | "failed";
    architectSystemPrompt: string;
    architectUserMessage: string;
    generatedPrompt?: string;
    durationMs?: number;
  };
}
