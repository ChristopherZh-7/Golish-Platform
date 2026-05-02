/**
 * Pipeline IPC wrappers (read-side; write-side `executePipeline` /
 * `cancelPipeline` / `deletePipeline` currently live in
 * `lib/api/targets.ts` for historical reasons — consolidate in a
 * follow-up PR).
 */

import type { PipelineSummary } from "../pentest/pipeline-types";
import { invoke } from "./client";

export async function listPipelines(projectPath: string | null): Promise<PipelineSummary[]> {
  return invoke<PipelineSummary[]>("pipeline_list", { projectPath });
}
