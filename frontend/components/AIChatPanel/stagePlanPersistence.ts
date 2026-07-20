/**
 * Persist the per-harness-stage roadmap (task mode, design 2026-06-04) per
 * conversation so the `StagePlanStack` / `StageProgressBar` survive a webview
 * refresh or app restart.
 *
 * Unlike the single merged `plan` (DB-backed via `terminal_state.planJson`), the
 * per-stage buckets (`stageOrder` / `plansByStage` / `passedStages`) live only in
 * the in-memory store and are rebuilt from streamed `plan_updated` / `stage_passed`
 * events — which don't replay on restore — so without this they vanish on refresh.
 *
 * Conversation ids are stable across reloads (persisted to the conversation DB and
 * restored by id), so they make a safe localStorage key — the same approach
 * {@link ./contextUsagePersistence} uses. Storage is a single JSON map and every
 * access is best-effort: a missing or corrupt entry just yields `null`.
 */

import { createResetStageSeed, localResetAffectedStages } from "@/lib/stage-reset";
import type { TaskPlan } from "@/store/store-types";

/** localStorage key holding the `{ [conversationId]: snapshot }` map. */
export const STAGE_PLAN_STORAGE_KEY = "golish.stagePlansByConversation";

export interface PersistedStagePlans {
  /** Order stages first appeared (run order), drives card render order. */
  stageOrder: string[];
  /** Per-stage plan buckets keyed by stage id. */
  plansByStage: Record<string, TaskPlan>;
  /** Stages whose authoritative evidence gate PASSED. */
  passedStages: string[];
}

type StagePlanMap = Record<string, PersistedStagePlans>;

function isPersistedStagePlans(value: unknown): value is PersistedStagePlans {
  if (!value || typeof value !== "object") return false;
  const v = value as Record<string, unknown>;
  return (
    Array.isArray(v.stageOrder) &&
    Array.isArray(v.passedStages) &&
    !!v.plansByStage &&
    typeof v.plansByStage === "object"
  );
}

function readMap(): StagePlanMap {
  try {
    const raw = globalThis.localStorage?.getItem(STAGE_PLAN_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as unknown;
    if (!parsed || typeof parsed !== "object") return {};
    return parsed as StagePlanMap;
  } catch {
    return {};
  }
}

/** Read the persisted per-stage roadmap for a conversation (best-effort, validated). */
export function readStagePlans(conversationId: string): PersistedStagePlans | null {
  if (!conversationId) return null;
  const snapshot = readMap()[conversationId];
  return isPersistedStagePlans(snapshot) ? snapshot : null;
}

/**
 * Persist the per-stage roadmap for a conversation so it survives reloads.
 * No-ops when there is nothing to persist (empty `stageOrder`) so an
 * uninitialized store can never clobber a previously saved snapshot.
 */
export function writeStagePlans(conversationId: string, data: PersistedStagePlans): void {
  if (!conversationId || data.stageOrder.length === 0) return;
  try {
    const map = readMap();
    map[conversationId] = data;
    globalThis.localStorage?.setItem(STAGE_PLAN_STORAGE_KEY, JSON.stringify(map));
  } catch {
    // localStorage unavailable (privacy mode / non-browser) — best-effort only.
  }
}

/** Drop the persisted snapshot for a conversation (e.g. on conversation delete). */
export function clearStagePlans(conversationId: string): void {
  if (!conversationId) return;
  try {
    const map = readMap();
    if (!(conversationId in map)) return;
    delete map[conversationId];
    globalThis.localStorage?.setItem(STAGE_PLAN_STORAGE_KEY, JSON.stringify(map));
  } catch {
    // best-effort only
  }
}

/**
 * Apply a committed backend reset receipt to the durable roadmap snapshot.
 * Unlike `writeStagePlans`, an empty remaining order is meaningful here and
 * must replace the old snapshot rather than be treated as uninitialized state.
 */
export function rewindPersistedStagePlans(
  conversationId: string,
  affectedStages: string[],
  selectedStage: string,
  updatedAt?: string
): string[] {
  if (
    !conversationId ||
    !selectedStage ||
    affectedStages.length === 0 ||
    !affectedStages.includes(selectedStage)
  ) {
    return affectedStages;
  }
  let reconciledAffectedStages = [...new Set(affectedStages)];
  try {
    const map = readMap();
    const stored = map[conversationId];
    const snapshot = isPersistedStagePlans(stored)
      ? stored
      : { stageOrder: [], plansByStage: {}, passedStages: [] };
    reconciledAffectedStages = [
      ...new Set([
        ...reconciledAffectedStages,
        ...localResetAffectedStages(snapshot.stageOrder, selectedStage),
      ]),
    ];
    const affected = new Set(reconciledAffectedStages);
    const plansByStage = { ...snapshot.plansByStage };
    for (const stage of affected) delete plansByStage[stage];
    plansByStage[selectedStage] = createResetStageSeed(selectedStage, updatedAt);
    const stageOrder = snapshot.stageOrder.filter((stage) => !affected.has(stage));
    stageOrder.push(selectedStage);
    map[conversationId] = {
      stageOrder,
      plansByStage,
      passedStages: snapshot.passedStages.filter((stage) => !affected.has(stage)),
    };
    globalThis.localStorage?.setItem(STAGE_PLAN_STORAGE_KEY, JSON.stringify(map));
  } catch {
    // localStorage unavailable — in-memory rewind still remains authoritative.
  }
  return reconciledAffectedStages;
}
