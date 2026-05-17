import type { TargetStore } from "../dashboard";
import type { Target } from "../pentest/types";
import { invoke } from "./client";

export type { TargetStore };

export async function listTargets(projectPath: string | null): Promise<TargetStore> {
  return invoke<TargetStore>("target_list", { projectPath });
}

export async function addTarget(params: {
  name: string;
  value: string;
  grp?: string;
  owner?: string;
  timeWindowStart?: string;
  timeWindowEnd?: string;
  /** Required in redteam projects; must be `undefined` (or omitted) in pentest projects. */
  organizationId?: string;
  projectPath: string | null;
}): Promise<void> {
  await invoke("target_add", params);
}

export async function batchAddTargets(params: {
  values: string;
  grp: string;
  projectPath: string | null;
}): Promise<Target[]> {
  return invoke<Target[]>("target_batch_add", params);
}

export async function deleteTarget(id: string, projectPath: string | null): Promise<void> {
  await invoke("target_delete", { id, projectPath });
}

export async function updateTarget(params: {
  id: string;
  scope?: string;
  notes?: string;
  grp?: string;
  owner?: string;
  timeWindowStart?: string;
  timeWindowEnd?: string;
  organizationId?: string;
  projectPath: string | null;
}): Promise<void> {
  await invoke("target_update", params);
}

export async function clearAllTargets(projectPath: string | null): Promise<void> {
  await invoke("target_clear_all", { projectPath });
}

export async function executePipeline(params: {
  pipeline: unknown;
  target: string;
  projectPath: string | null;
}): Promise<void> {
  await invoke("pipeline_execute", params);
}

export async function cancelPipeline(): Promise<void> {
  await invoke("pipeline_cancel");
}

export async function deletePipeline(id: string, projectPath: string | null): Promise<void> {
  await invoke("pipeline_delete", { id, projectPath });
}
