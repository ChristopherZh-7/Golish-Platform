import { invoke } from "./client";

export async function addTarget(params: {
  name: string;
  value: string;
  group?: string;
  projectPath: string | null;
}): Promise<void> {
  await invoke("target_add", params);
}

export async function batchAddTargets(params: {
  values: string;
  group: string;
  projectPath: string | null;
}): Promise<void> {
  await invoke("target_batch_add", params);
}

export async function deleteTarget(id: string, projectPath: string | null): Promise<void> {
  await invoke("target_delete", { id, projectPath });
}

export async function updateTarget(params: {
  id: string;
  scope?: string;
  notes?: string;
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
