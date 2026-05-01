import type { ActiveToolCall } from "./tool-call";

export type WorkflowStatus = "idle" | "running" | "completed" | "error";

export interface WorkflowStep {
  name: string;
  index: number;
  status: "pending" | "running" | "completed" | "error";
  output?: string | null;
  durationMs?: number;
  startedAt?: string;
  completedAt?: string;
}

export interface ActiveWorkflow {
  workflowId: string;
  workflowName: string;
  sessionId: string;
  status: WorkflowStatus;
  steps: WorkflowStep[];
  currentStepIndex: number;
  totalSteps: number;
  startedAt: string;
  completedAt?: string;
  totalDurationMs?: number;
  finalOutput?: string;
  error?: string;
  toolCalls?: ActiveToolCall[];
}
