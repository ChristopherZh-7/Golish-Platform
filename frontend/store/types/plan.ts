export type { StepStatus, PlanSummary } from "@/lib/generated";
export type { PlanStep } from "@/lib/generated";
export type { TaskPlan } from "@/lib/generated";

export interface RetiredPlan {
  plan: TaskPlan;
  messageId: string;
  retiredAt: string;
}
