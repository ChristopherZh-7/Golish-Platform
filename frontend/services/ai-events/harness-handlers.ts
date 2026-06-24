/**
 * Harness-trace AI event handlers.
 *
 * Routes `harness_trace` events to the stage-run view: a `stage_run_org_progress`
 * trace upserts one org's row into the session's `stageRun`, so the standard
 * tool-call detail pane (ToolCallDetailView → StageRunOrgRows) renders live
 * per-org fan-out progress on the `stage_run` tool row (design
 * 2026-06-13-stage-run-fanout, Task 6 — the real-event path that supersedes the
 * dev-only `__mockStageRun`). Other harness-trace kinds (gate / evidence /
 * deliverable / notes) are observability traces handled elsewhere, ignored here.
 */

import type {
  StageRunRow,
  StageRunStatus,
  TechniqueState,
} from "@/components/Engagement/StageRunOrgRows";
import type { AiEvent } from "@/lib/ai";
import type { EventHandler } from "./types";

const STAGE_RUN_STATUSES: readonly StageRunStatus[] = [
  "passed",
  "running",
  "queued",
  "blocked",
  "pending",
];

const TECHNIQUE_STATES: readonly TechniqueState[] = [
  "found",
  "checked_empty",
  "blocked",
  "pending",
];

/** Clamp an arbitrary backend status string to a known {@link StageRunStatus}. */
function toStageRunStatus(s: string): StageRunStatus {
  return (STAGE_RUN_STATUSES as readonly string[]).includes(s) ? (s as StageRunStatus) : "pending";
}

/** Turn the wire `[technique, state][]` pairs into the view's coverage record. */
function toCoverage(pairs: [string, string][]): Record<string, TechniqueState> {
  const out: Record<string, TechniqueState> = {};
  for (const [technique, state] of pairs) {
    out[technique] = (TECHNIQUE_STATES as readonly string[]).includes(state)
      ? (state as TechniqueState)
      : "pending";
  }
  return out;
}

export function stageRunRequestIdFromAgentRequestId(agentRequestId?: string | null): string | null {
  if (!agentRequestId) return null;
  const marker = "::org::";
  const idx = agentRequestId.indexOf(marker);
  if (idx <= 0) return null;
  return agentRequestId.slice(0, idx);
}

/**
 * Handle a `harness_trace` event. Only `stage_run_org_progress` drives UI today;
 * it upserts the per-org row into the session's stage-run, rendered in the
 * standard tool-call detail pane (ToolCallDetailView → StageRunOrgRows).
 */
export const handleHarnessTrace: EventHandler<Extract<AiEvent, { type: "harness_trace" }>> = (
  event,
  ctx
) => {
  if (event.kind !== "stage_run_org_progress") return;

  const row: StageRunRow = {
    id: event.org_id,
    name: event.org_name,
    ownershipPercent: event.ownership_percent ?? null,
    status: toStageRunStatus(event.status),
    // The org's specialist sub-agent `parentRequestId`: lets the detail pane
    // make this row drill-in-able into that org's own conversation/tools.
    agentRequestId: event.agent_request_id ?? undefined,
    activity: event.activity ?? undefined,
    evidenceCount: event.evidence_count ?? 0,
    coverage: toCoverage(event.coverage ?? []),
  };

  ctx.getState().upsertStageRunRow(ctx.sessionId, row, {
    stageLabel: event.stage_label,
    roleLabel: event.role_label,
    coverageAxis: event.coverage_axis ?? [],
    requestId: stageRunRequestIdFromAgentRequestId(event.agent_request_id),
  });
};
