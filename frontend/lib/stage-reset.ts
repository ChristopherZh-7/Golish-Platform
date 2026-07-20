import type { TaskPlan } from "@/store/store-types";

/** Stages whose discovered facts can be safely purged in-place. */
export const IN_PLACE_RESET_STAGES = new Set([
  "target_intel",
  "external_attack_surface",
  "enumeration",
  "vuln_triage",
]);

/** Every stage understood by the currently shipped harness DAG. */
export const KNOWN_HARNESS_STAGES = new Set([
  "scoping",
  ...IN_PLACE_RESET_STAGES,
  "attack_candidate",
  "verification",
  "access_validation",
  "internal_discovery",
  "objective_pathing",
  "objective_simulation",
  "cleanup",
  "reporting",
]);

/** Create the authoritative local marker for a backend-committed reset frontier. */
export function createResetStageSeed(
  stage: string,
  updatedAt = new Date().toISOString()
): TaskPlan {
  return {
    version: 0,
    explanation: null,
    updated_at: updatedAt,
    steps: [{ step: stage, status: "in_progress" }],
    summary: { total: 1, completed: 0, in_progress: 1, pending: 0 },
  };
}

/**
 * Infer the real running frontier from a non-pending stage marker. Whole-DAG
 * pending seeds describe ordering only, so treating their first unpassed node
 * as current would enable a destructive reset against the wrong backend stage.
 */
export function inferCurrentResetStage<TPlan extends { steps: Array<{ status: string }> }>(
  stageOrder: string[],
  plansByStage: Record<string, TPlan | undefined>,
  passedStages: string[]
): string | null {
  const passed = new Set(passedStages);
  return (
    stageOrder.find(
      (stage) =>
        KNOWN_HARNESS_STAGES.has(stage) &&
        !passed.has(stage) &&
        (plansByStage[stage]?.steps.some((step) => step.status !== "pending") ?? false)
    ) ?? null
  );
}

/** Derive the local DAG suffix that a restart-from-stage commit invalidates. */
export function localResetAffectedStages(stageOrder: string[], selectedStage: string): string[] {
  const selectedIndex = stageOrder.indexOf(selectedStage);
  const candidates = selectedIndex >= 0 ? stageOrder.slice(selectedIndex) : [selectedStage];
  return [...new Set(candidates.filter((stage) => KNOWN_HARNESS_STAGES.has(stage)))];
}

/**
 * Reconcile from the local DAG suffix even when post-commit metadata is null or
 * incomplete. The backend command resolving is the commit boundary; receipt
 * validation is diagnostic and must not leave known descendant plans stale.
 */
export function trustedResetAffectedStages(
  _receipt: unknown,
  selectedStage: string,
  stageOrder: string[]
): string[] {
  return localResetAffectedStages(stageOrder, selectedStage);
}

/** Validate every field the UI relies on after the backend transaction commits. */
export function validateCommittedStageResetReceipt(
  receipt: unknown,
  selectedStage: string,
  expectedAffectedStages: string[]
): string | null {
  if (receipt === null || typeof receipt !== "object" || Array.isArray(receipt)) {
    return "receipt 不是对象";
  }
  const value = receipt as Record<string, unknown>;
  if (typeof value.operationId !== "string" || value.operationId.length === 0) {
    return "operationId 缺失";
  }
  if (value.stage !== selectedStage) return "stage 与所选阶段不一致";
  if (value.mode !== "restart_from_stage_purge") return "mode 不是完整重置";
  if (value.currentStage !== selectedStage) return "currentStage 未指向所选阶段";
  if (value.refreshedStageCursor !== true) return "refreshedStageCursor 未刷新";
  if (value.resetGraphFlow !== true) return "resetGraphFlow 未重置";
  if (value.purgedFacts !== true) return "purgedFacts 未确认清理";
  if (
    typeof value.purgeScopeOrgCount !== "number" ||
    !Number.isFinite(value.purgeScopeOrgCount) ||
    value.purgeScopeOrgCount <= 0
  ) {
    return "purgeScopeOrgCount 不是有效范围";
  }
  if (
    value.purgeCounts === null ||
    typeof value.purgeCounts !== "object" ||
    Array.isArray(value.purgeCounts)
  ) {
    return "purgeCounts 缺失";
  }
  if (value.purgeNote !== null) return "purgeNote 表示重置未完整执行";
  const affectedStages = value.affectedStages;
  if (
    !Array.isArray(affectedStages) ||
    affectedStages.some((stage) => typeof stage !== "string" || !KNOWN_HARNESS_STAGES.has(stage)) ||
    expectedAffectedStages.some((stage) => !affectedStages.includes(stage))
  ) {
    return "affectedStages 不可信";
  }
  return null;
}
