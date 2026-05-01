import type { TaskPlan } from "@/lib/generated";

export type { PlanStep, PlanSummary, StepStatus, TaskPlan } from "@/lib/generated";

export interface RetiredPlan {
  plan: TaskPlan;
  messageId: string;
  retiredAt: string;
}
