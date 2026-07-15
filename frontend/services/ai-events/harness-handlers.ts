/**
 * Harness-trace AI event handlers.
 *
 * Routes `harness_trace` events to the stage-run view: a `stage_run_org_progress`
 * trace upserts one org's row into the session's `stageRun`, so the standard
 * tool-call detail pane (ToolCallDetailView → StageRunOrgRows) renders live
 * per-org fan-out progress on the `stage_run` tool row (design
 * 2026-06-13-stage-run-fanout, Task 6 — the real-event path that supersedes the
 * dev-only `__mockStageRun`). Candidate review traces are refresh-only hints:
 * the UI still reloads the durable review/barrier rows through IPC. Reporting
 * gate/deliverable traces likewise only point the chat at a DB-backed read.
 */

import type {
  StageRunRow,
  StageRunStatus,
  TechniqueState,
} from "@/components/Engagement/StageRunOrgRows";
import type { AiEvent } from "@/lib/ai";
import type { EventHandler, EventHandlerContext } from "./types";

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
  "not_applicable",
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
  const indexes = ["::org::", "::team::"]
    .map((marker) => agentRequestId.indexOf(marker))
    .filter((index) => index > 0);
  if (indexes.length === 0) return null;
  return agentRequestId.slice(0, Math.min(...indexes));
}

/**
 * Bump an already-open Candidate read model without inventing a review cursor.
 * Terminal/consolidation traces do not carry `resume_version`, so they may only
 * reuse the exact operation/wave hint established by the authoritative review
 * trace. A missed review trace remains a DB/bootstrap concern, not trace truth.
 */
function refreshExistingCandidateView(
  ctx: EventHandlerContext,
  operationId: string,
  waveRunId: string,
  status: string
): void {
  const state = ctx.getState();
  const current = state.sessions[ctx.sessionId]?.candidateReviewHint;
  if (!current || current.operationId !== operationId || current.waveRunId !== waveRunId) {
    return;
  }
  state.setCandidateReviewHint(ctx.sessionId, {
    operationId,
    waveRunId,
    status,
    resumeVersion: current.resumeVersion,
  });
}

/**
 * Handle the two UI-facing trace families. Neither trace is authoritative:
 * stage rows are progress display and Candidate review traces only trigger a
 * DB reload in the detail panel.
 */
export const handleHarnessTrace: EventHandler<Extract<AiEvent, { type: "harness_trace" }>> = (
  event,
  ctx
) => {
  if (event.kind === "candidate_review_required" || event.kind === "candidate_review_resumed") {
    ctx.getState().setCandidateReviewHint(ctx.sessionId, {
      operationId: event.operation_id,
      waveRunId: event.wave_run_id,
      status: event.kind === "candidate_review_resumed" ? "resumed" : event.status,
      resumeVersion: event.resume_version,
    });
    return;
  }
  if (event.kind === "candidate_attempt_terminalized") {
    refreshExistingCandidateView(ctx, event.operation_id, event.wave_run_id, event.status);
    return;
  }
  if (event.kind === "attack_wave_consolidated") {
    refreshExistingCandidateView(
      ctx,
      event.operation_id,
      event.source_wave_run_id,
      event.decision_kind
    );
    return;
  }
  if (
    event.stage === "reporting" &&
    (event.kind === "gate_decision" || event.kind === "deliverable_submitted") &&
    event.operation_id.trim()
  ) {
    ctx.getState().setReportingReadModelHint(ctx.sessionId, {
      operationId: event.operation_id,
    });
    return;
  }
  if (event.kind !== "stage_run_org_progress") return;

  const row: StageRunRow = {
    id: event.org_id,
    operationId: event.operation_id,
    stageExecutionId: event.stage_execution_id ?? undefined,
    stageRunUnitId: event.stage_run_unit_id ?? undefined,
    name: event.org_name,
    ownershipPercent: event.ownership_percent ?? null,
    status: toStageRunStatus(event.status),
    // The org's specialist sub-agent `parentRequestId`: lets the detail pane
    // make this row drill-in-able into that org's own conversation/tools.
    agentRequestId: event.agent_request_id ?? undefined,
    activity: event.activity ?? undefined,
    evidenceCount: event.evidence_count ?? 0,
    coverage: toCoverage(event.coverage ?? []),
    stage: event.stage,
  };

  ctx.getState().upsertStageRunRow(ctx.sessionId, row, {
    stageLabel: event.stage_label,
    roleLabel: event.role_label,
    coverageAxis: event.coverage_axis ?? [],
    requestId: stageRunRequestIdFromAgentRequestId(event.agent_request_id),
  });
};
