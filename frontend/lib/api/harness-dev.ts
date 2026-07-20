import type { HarnessDevStageCheckpointResetResult } from "../generated/HarnessDevStageCheckpointResetResult";
import { invoke } from "./client";

export type HarnessDevStageCheckpointResetMode =
  | "clear_repair"
  | "restart_stage"
  | "restart_from_stage"
  // Full reset: rewind the cursor/checkpoint to the selected stage AND delete the
  // discovered facts produced by that stage + its DAG descendants (engagement org
  // subtree), so re-testing the stage starts from a clean slate. Logs are kept.
  | "restart_from_stage_purge";

export interface HarnessDevResetStageCheckpointArgs {
  operationId?: string | null;
  sessionId?: string | null;
  organizationId?: string | null;
  stage: string;
  mode: HarnessDevStageCheckpointResetMode;
}

export async function resetHarnessStageCheckpoint({
  operationId,
  sessionId,
  organizationId,
  stage,
  mode,
}: HarnessDevResetStageCheckpointArgs): Promise<HarnessDevStageCheckpointResetResult> {
  return invoke<HarnessDevStageCheckpointResetResult>("harness_dev_reset_stage_checkpoint", {
    operationId: operationId ?? null,
    sessionId: sessionId ?? null,
    organizationId: organizationId ?? null,
    stage,
    mode,
  });
}
