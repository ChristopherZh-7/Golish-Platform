/**
 * Pipeline IPC wrappers — read-side (list/save/templates) and write-side
 * (execute/cancel/delete). All pipeline IPC lives here.
 */

import type { Pipeline } from "../pentest/pipeline-types";
import { invoke } from "./client";

// Backend `pipeline_list` returns full `Vec<Pipeline>` (see
// backend/crates/golish/src/tools/pipeline/commands.rs::pipeline_list), so the
// editor can hydrate steps/connections directly from a list row.
export async function listPipelines(projectPath: string | null): Promise<Pipeline[]> {
  return invoke<Pipeline[]>("pipeline_list", { projectPath });
}

export async function savePipeline(
  pipeline: Pipeline,
  projectPath: string | null
): Promise<string> {
  return invoke<string>("pipeline_save", { pipeline, projectPath });
}

export async function savePipelineTemplate(pipeline: Pipeline): Promise<string> {
  return invoke<string>("pipeline_save_template", { pipeline });
}

export async function listPipelineTemplates(): Promise<Pipeline[]> {
  return invoke<Pipeline[]>("pipeline_list_templates");
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
