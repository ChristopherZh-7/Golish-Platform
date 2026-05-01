export type StepStatus = "pending" | "in_progress" | "completed" | "cancelled" | "failed";

export interface PlanStep {
  id?: string;
  step: string;
  status: StepStatus;
}

export interface PlanSummary {
  total: number;
  completed: number;
  in_progress: number;
  pending: number;
}

export interface TaskPlan {
  explanation: string | null;
  steps: PlanStep[];
  summary: PlanSummary;
  version: number;
  updated_at: string;
}

export interface RetiredPlan {
  plan: TaskPlan;
  messageId: string;
  retiredAt: string;
}
