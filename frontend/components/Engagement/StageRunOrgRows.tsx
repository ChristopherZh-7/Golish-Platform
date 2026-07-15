/**
 * StageRunOrgRows — the exact routing seam from `stage_run_org_progress` refresh
 * pointers into the durable Company Controller read model. Company-scoped stages
 * no longer expose the legacy `Main Agent -> Specialist` event-snapshot UI.
 * Historical/non-V2 runs without one exact operation + execution pointer are
 * explicitly rerun-required instead of being rendered as a different scheduler.
 */

import { AlertTriangle } from "lucide-react";
import { type StageTeamReadApi, StageTeamRunView } from "@/components/Engagement/StageTeamRunView";

export type StageRunStatus = "passed" | "running" | "queued" | "blocked" | "pending" | "stopped";

/** Per-technique terminal state on the coverage axis (mirrors the gate contract). */
export type TechniqueState = "found" | "checked_empty" | "blocked" | "not_applicable" | "pending";

export interface StageRunRow {
  id: string;
  /** Trusted operation identity from the harness trace; IPC reauthorizes it. */
  operationId?: string;
  /** Refresh-only pointer to the exact durable Stage execution. */
  stageExecutionId?: string | null;
  /** Refresh-only pointer to this organization's StageRunUnit. */
  stageRunUnitId?: string | null;
  name: string;
  ownershipPercent: number | null;
  status: StageRunStatus;
  /**
   * The org's specialist sub-agent `parentRequestId` (= the backend
   * `StageRunOrgProgress` event's `agent_request_id`). When present, the row is
   * drill-in-able into that sub-agent's own conversation / reasoning / tool calls
   * via {@link StageRunOrgRowsProps.onDrillIn}.
   */
  agentRequestId?: string | null;
  /** Live one-liner while running (e.g. "subfinder · pingan.com.cn"). */
  activity?: string;
  /** Evidence rows this org's specialist has booked into the ledger. */
  evidenceCount: number;
  /** Per-technique state, keyed by the stage's `coverageAxis` entries. */
  coverage: Record<string, TechniqueState>;
  /** Stable harness stage key from the trace event, e.g. `external_attack_surface`. */
  stage?: string;
}

export interface StageRunSummary {
  total: number;
  covered: number;
  active: number;
  queued: number;
  blocked: number;
}

export interface StageRunOrgRowsProps {
  rows: StageRunRow[];
  /**
   * Drill from an org row into that org's specialist sub-agent detail. Given the
   * row's `agentRequestId`; only rows that carry one render as clickable. Wired
   * by {@link ToolCallDetailView} to open the sub-agent detail pane.
   */
  onDrillIn?: (agentRequestId: string) => void;
  /** Exact Stage Team WorkerRun -> SubAgent parent request identities. */
  agentRequestIdsByWorker?: Readonly<Record<string, string>>;
  /** Test seam / alternate transport for the exact durable Team read model. */
  teamApi?: StageTeamReadApi;
}

const COMPANY_CONTROLLER_STAGES = new Set([
  "target_intel",
  "external_attack_surface",
  "enumeration",
  "vuln_triage",
]);

/** Keep Candidate/Verification and post-exploit rows on their own typed views. */
export function isCompanyControllerStageRunRows(rows: readonly StageRunRow[]): boolean {
  return (
    rows.length > 0 &&
    rows.every((row) => Boolean(row.stage) && COMPANY_CONTROLLER_STAGES.has(row.stage ?? ""))
  );
}

/**
 * Render only the authoritative Team view. Historical rows without an exact
 * pointer are intentionally not reinterpreted as a legacy scheduler.
 */
export function StageRunOrgRows({
  rows,
  onDrillIn,
  agentRequestIdsByWorker,
  teamApi,
}: StageRunOrgRowsProps) {
  if (!isCompanyControllerStageRunRows(rows)) return null;

  const teamPointers = rows.filter(
    (row) => Boolean(row.operationId?.trim()) && Boolean(row.stageExecutionId?.trim())
  );
  const firstTeamPointer = teamPointers[0];
  const exactTeamPointer =
    teamPointers.length === rows.length &&
    firstTeamPointer &&
    teamPointers.every(
      (row) =>
        row.operationId === firstTeamPointer.operationId &&
        row.stageExecutionId === firstTeamPointer.stageExecutionId
    )
      ? firstTeamPointer
      : null;
  const teamRefreshVersion = rows
    .map(
      (row) =>
        `${row.stageRunUnitId ?? "legacy"}:${row.status}:${row.activity ?? ""}:${row.evidenceCount}`
    )
    .join("|");

  if (exactTeamPointer?.operationId && exactTeamPointer.stageExecutionId) {
    return (
      <StageTeamRunView
        operationId={exactTeamPointer.operationId}
        stageExecutionId={exactTeamPointer.stageExecutionId}
        refreshVersion={teamRefreshVersion}
        api={teamApi}
        agentRequestIdsByWorker={agentRequestIdsByWorker}
        onOpenAgent={onDrillIn}
      />
    );
  }

  return (
    <div
      className="flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2.5"
      data-testid="stage-team-rerun-required"
    >
      <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-amber-400" />
      <div className="min-w-0">
        <div className="text-xs font-medium text-amber-200">
          Company Controller data unavailable
        </div>
        <div className="mt-0.5 text-[11px] leading-relaxed text-amber-100/70">
          This historical run has no exact durable Team pointer. Start a new V2 run and rerun this
          stage to use the current Company Controller scheduler.
        </div>
      </div>
    </div>
  );
}
