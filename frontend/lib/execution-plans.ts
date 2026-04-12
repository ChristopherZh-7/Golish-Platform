/**
 * Typed wrappers for execution plan Tauri commands.
 * Execution plans track structured multi-step AI tasks for continuation.
 */

import { invoke } from "@tauri-apps/api/core";

export interface PlanStep {
  id: string;
  title: string;
  description: string;
  status: "pending" | "in_progress" | "completed" | "failed" | "skipped";
  agent?: string | null;
  result?: string | null;
  startedAt?: string | null;
  completedAt?: string | null;
}

export interface ExecutionPlan {
  id: string;
  sessionId: string | null;
  projectPath: string | null;
  title: string;
  description: string;
  steps: PlanStep[];
  status:
    | "planning"
    | "in_progress"
    | "paused"
    | "completed"
    | "failed"
    | "cancelled";
  currentStep: number;
  context: Record<string, unknown>;
  createdAt: string;
  updatedAt: string;
}

export async function planCreate(
  projectPath: string,
  title: string,
  description: string,
  steps: PlanStep[],
  sessionId?: string
): Promise<ExecutionPlan> {
  return invoke("plan_create", {
    projectPath,
    title,
    description,
    steps,
    sessionId,
  });
}

export async function planGet(id: string): Promise<ExecutionPlan | null> {
  return invoke("plan_get", { id });
}

export async function planList(
  projectPath: string,
  includeCompleted?: boolean
): Promise<ExecutionPlan[]> {
  return invoke("plan_list", { projectPath, includeCompleted });
}

export async function planListActive(
  projectPath: string
): Promise<ExecutionPlan[]> {
  return invoke("plan_list_active", { projectPath });
}

export async function planUpdateSteps(
  id: string,
  steps: PlanStep[],
  currentStep: number,
  status: ExecutionPlan["status"]
): Promise<void> {
  return invoke("plan_update_steps", { id, steps, currentStep, status });
}

export async function planUpdateStatus(
  id: string,
  status: ExecutionPlan["status"]
): Promise<void> {
  return invoke("plan_update_status", { id, status });
}

export async function planUpdateContext(
  id: string,
  context: Record<string, unknown>
): Promise<void> {
  return invoke("plan_update_context", { id, context });
}

export async function planDelete(id: string): Promise<void> {
  return invoke("plan_delete", { id });
}
