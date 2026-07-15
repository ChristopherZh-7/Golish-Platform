/** Exact DB-backed Stage Team Scheduler read model. */

import type { StageTeamReadModel } from "@/lib/generated/StageTeamReadModel";
import type { StageTeamReadRequest } from "@/lib/generated/StageTeamReadRequest";
import type { StageTeamRecoveryResolveRequest } from "@/lib/generated/StageTeamRecoveryResolveRequest";
import type { StageTeamRecoveryResolveResponse } from "@/lib/generated/StageTeamRecoveryResolveResponse";
import { invoke } from "./client";

export type {
  StageTeamReadModel,
  StageTeamReadRequest,
  StageTeamRecoveryResolveRequest,
  StageTeamRecoveryResolveResponse,
};

export function getStageTeamReadModel(request: StageTeamReadRequest): Promise<StageTeamReadModel> {
  return invoke("ai_get_stage_team_read_model", { request });
}

/**
 * Terminalize one exact expired active-tool Worker as outcome-unknown.
 * This never replays the external tool and is safe to retry with the same requestId.
 */
export function resolveStageTeamRecovery(
  request: StageTeamRecoveryResolveRequest
): Promise<StageTeamRecoveryResolveResponse> {
  return invoke("ai_resolve_stage_team_recovery", { request });
}
