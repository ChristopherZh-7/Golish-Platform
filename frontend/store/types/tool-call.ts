import type { ApprovalPattern } from "@/lib/ai";
import type { RiskLevel } from "@/lib/tools";

export type ToolCallSource =
  | { type: "main" }
  | { type: "sub_agent"; agentId: string; agentName: string }
  | {
      type: "workflow";
      workflowId: string;
      workflowName: string;
      stepName?: string;
      stepIndex?: number;
    };

export interface ToolCall {
  id: string;
  name: string;
  args: Record<string, unknown>;
  status: "pending" | "approved" | "denied" | "running" | "backgrounded" | "completed" | "error";
  result?: unknown;
  executedByAgent?: boolean;
  riskLevel?: RiskLevel;
  stats?: ApprovalPattern;
  suggestion?: string;
  canLearn?: boolean;
  autoApproved?: boolean;
  autoApprovalReason?: string;
  source?: ToolCallSource;
}

export interface ActiveToolCall {
  id: string;
  name: string;
  args: Record<string, unknown>;
  status: "running" | "backgrounded" | "completed" | "error";
  result?: unknown;
  startedAt: string;
  completedAt?: string;
  executedByAgent?: boolean;
  source?: ToolCallSource;
  streamingOutput?: string;
}

export interface AskHumanRequest {
  requestId: string;
  sessionId?: string;
  question: string;
  rawInputType?: string;
  inputType:
    | "credentials"
    | "choice"
    | "freetext"
    | "confirmation"
    | "scope_review"
    | "unit_review";
  options: string[];
  context: string;
}
