import type { ActiveSubAgent } from "./sub-agent";

export type PipelineStepStatus =
  | "pending"
  | "running"
  | "success"
  | "failed"
  | "skipped"
  | "interrupted";

export interface PipelineSubTarget {
  target: string;
  status: PipelineStepStatus;
  output?: string;
  exitCode?: number | null;
  durationMs?: number;
}

export interface PipelineStepExecution {
  stepId: string;
  name: string;
  command: string;
  status: PipelineStepStatus;
  output?: string;
  exitCode?: number | null;
  startedAt?: string;
  finishedAt?: string;
  durationMs?: number;
  discoveredTargets?: string[];
  subTargets?: PipelineSubTarget[];
  subAgents?: ActiveSubAgent[];
}

export interface PipelineExecution {
  pipelineId: string;
  pipelineName: string;
  target: string;
  steps: PipelineStepExecution[];
  status: "pending" | "running" | "completed" | "failed" | "interrupted";
  startedAt: string;
  finishedAt?: string;
}
