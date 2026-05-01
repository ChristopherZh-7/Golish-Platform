export type { StepStatus, PlanSummary } from "@/lib/generated";
export type { PlanStep } from "@/lib/generated";
import type { PlanStep, PlanSummary } from "@/lib/generated";

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
