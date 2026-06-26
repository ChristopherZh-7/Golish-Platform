import type { StageAssetCoverageSnapshot } from "@/lib/generated/StageAssetCoverageSnapshot";
import { invoke } from "./client";

export type { StageAssetCoverageSnapshot } from "@/lib/generated/StageAssetCoverageSnapshot";

export interface GetStageAssetCoverageArgs {
  organizationId: string;
  stage: string;
  sessionId?: string | null;
  stageStartedAt?: string | null;
}

export async function getStageAssetCoverage({
  organizationId,
  stage,
  sessionId,
  stageStartedAt,
}: GetStageAssetCoverageArgs): Promise<StageAssetCoverageSnapshot> {
  return invoke<StageAssetCoverageSnapshot>("ai_get_stage_asset_coverage", {
    organizationId,
    stage,
    sessionId: sessionId ?? null,
    stageStartedAt: stageStartedAt ?? null,
  });
}
