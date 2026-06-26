/**
 * StageRunOrgRows — the per-org fan-out rows for ONE harness stage's `stage_run`,
 * rendered INSIDE the standard tool-call detail pane ({@link ToolCallDetailView})
 * when the selected tool is `stage_run`. This is the convergence target: a
 * `stage_run` is "just a tool, with a bit more state" — its live per-org progress
 * shows in the standard tool's Details (left pane), not in a bespoke chat card or
 * dedicated stage-run pane (设计 2026-06-13-stage-run-fanout, superseded UI).
 *
 * Generic over the stage: each row is one in-scope org handled by that stage's
 * specialist (intel=Recon, EAS=Prober, attack=Pentester), AI-driven and gated by
 * its own evidence. Stage-specific bits (role label, coverage axis, stage label)
 * are PROPS, driven by the per-stage harness config — no per-stage component.
 *
 * Purely presentational; the live data comes from the session's `stageRun` state,
 * upserted from `stage_run_org_progress` harness-trace events.
 */

import {
  AlertTriangle,
  Bot,
  CheckCircle2,
  ChevronRight,
  Circle,
  Clock,
  Loader2,
  Workflow,
} from "lucide-react";
import { cn } from "@/lib/utils";

export type StageRunStatus = "passed" | "running" | "queued" | "blocked" | "pending" | "stopped";

/** Per-technique terminal state on the coverage axis (mirrors the gate contract). */
export type TechniqueState = "found" | "checked_empty" | "blocked" | "not_applicable" | "pending";

export interface StageRunRow {
  id: string;
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

const STATUS_META: Record<StageRunStatus, { label: string; className: string }> = {
  passed: { label: "Passed", className: "bg-emerald-500/15 text-emerald-400" },
  running: { label: "Running", className: "bg-sky-500/15 text-sky-400" },
  queued: { label: "Queued", className: "bg-indigo-500/15 text-indigo-400" },
  blocked: { label: "Blocked", className: "bg-amber-500/15 text-amber-400" },
  pending: { label: "Pending", className: "bg-slate-500/15 text-slate-400" },
  stopped: { label: "Stopped", className: "bg-yellow-500/10 text-yellow-400" },
};

const TECH_META: Record<TechniqueState, { className: string; mark: string }> = {
  found: { className: "bg-emerald-500/15 text-emerald-300 border-emerald-500/30", mark: "✓" },
  checked_empty: { className: "bg-slate-500/15 text-slate-400 border-slate-500/30", mark: "∅" },
  blocked: { className: "bg-amber-500/15 text-amber-300 border-amber-500/30", mark: "!" },
  not_applicable: {
    className: "bg-muted/30 text-muted-foreground/50 border-border/30",
    mark: "-",
  },
  pending: { className: "bg-transparent text-muted-foreground/40 border-border/40", mark: "·" },
};

function roleAgentLabel(roleLabel: string) {
  const label = roleLabel.trim();
  if (!label) return "Specialist Agent";
  return /agent$/i.test(label) ? label : `${label} Agent`;
}

function CollectorGlyph({ status }: { status: StageRunStatus }) {
  switch (status) {
    case "running":
      return <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-sky-400" />;
    case "passed":
      return <CheckCircle2 className="h-3.5 w-3.5 shrink-0 text-emerald-400" />;
    case "queued":
      return <Clock className="h-3.5 w-3.5 shrink-0 text-indigo-400" />;
    case "blocked":
      return <AlertTriangle className="h-3.5 w-3.5 shrink-0 text-amber-400" />;
    case "stopped":
      return <Circle className="h-3.5 w-3.5 shrink-0 text-yellow-400" />;
    default:
      return <Circle className="h-3.5 w-3.5 shrink-0 text-slate-500" />;
  }
}

function CoverageChips({
  coverageAxis,
  coverage,
}: {
  coverageAxis: string[];
  coverage: Record<string, TechniqueState>;
}) {
  if (coverageAxis.length === 0) return null;
  return (
    <div className="ml-5 mt-1.5 flex flex-wrap items-center gap-1">
      {coverageAxis.map((tech) => {
        const meta = TECH_META[coverage[tech] ?? "pending"];
        return (
          <span
            key={tech}
            className={cn(
              "inline-flex items-center gap-0.5 rounded border px-1 py-0.5 text-[9px] font-medium",
              meta.className
            )}
            title={`${tech}: ${coverage[tech] ?? "pending"}`}
          >
            {meta.mark} {tech}
          </span>
        );
      })}
    </div>
  );
}

function CollectorCard({
  row,
  roleLabel,
  coverageAxis,
  onDrillIn,
}: {
  row: StageRunRow;
  roleLabel: string;
  coverageAxis: string[];
  onDrillIn?: (agentRequestId: string) => void;
}) {
  const meta = STATUS_META[row.status];
  const drillId = row.agentRequestId ?? null;
  const clickable = Boolean(onDrillIn && drillId);
  const workerLabel = roleAgentLabel(roleLabel);

  const containerClass = cn(
    "block w-full rounded-lg border bg-background/50 p-2.5 text-left transition-colors",
    row.status === "running"
      ? "border-l-2 border-l-sky-400/80"
      : row.status === "blocked"
        ? "border-amber-500/40"
        : "border-border/40",
    clickable && "cursor-pointer hover:border-accent/40 hover:bg-accent/[0.06]"
  );

  const body = (
    <>
      <div className="flex items-center gap-2">
        <CollectorGlyph status={row.status} />
        {roleLabel && (
          <span className="inline-flex shrink-0 items-center gap-1 rounded bg-cyan-500/15 px-1.5 py-0.5 text-[10px] font-medium text-cyan-300">
            <Bot className="h-2.5 w-2.5" />
            {workerLabel}
          </span>
        )}
        <span className="min-w-0 truncate text-[12px] font-medium" title={row.name}>
          {row.name}
        </span>
        {row.ownershipPercent != null && (
          <span className="shrink-0 text-[10px] text-muted-foreground/70">
            {row.ownershipPercent}%
          </span>
        )}
        <div className="flex-1" />
        {row.evidenceCount > 0 && (
          <span className="shrink-0 text-[10px] tabular-nums text-muted-foreground/60">
            {row.evidenceCount} 证据
          </span>
        )}
        <span
          className={cn(
            "inline-flex shrink-0 items-center gap-1 rounded px-2 py-0.5 text-[11px] font-medium",
            meta.className
          )}
        >
          {row.status === "running" && <Loader2 className="h-3 w-3 animate-spin" />}
          {meta.label}
        </span>
        {clickable && <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground/50" />}
      </div>

      {row.status === "running" && row.activity && (
        <div className="ml-5 mt-1.5 flex min-w-0 items-center gap-1.5 text-[11px] text-sky-400/90">
          <span
            className="inline-block h-1.5 w-1.5 shrink-0 animate-pulse rounded-full bg-sky-400"
            aria-hidden="true"
          />
          <span className="truncate" title={row.activity}>
            {workerLabel} 正在 {row.activity}
          </span>
        </div>
      )}

      <CoverageChips coverageAxis={coverageAxis} coverage={row.coverage} />
    </>
  );

  if (clickable && drillId && onDrillIn) {
    return (
      <button
        type="button"
        className={containerClass}
        onClick={() => onDrillIn(drillId)}
        title={`打开 ${row.name} 的 ${workerLabel} 详情（对话 / 思考 / 工具调用）`}
      >
        {body}
      </button>
    );
  }

  return <div className={containerClass}>{body}</div>;
}

export interface StageRunOrgRowsProps {
  rows: StageRunRow[];
  summary: StageRunSummary;
  /** Stage display name, e.g. "Target Intel". */
  stageLabel: string;
  /** The stage specialist's label shown on each row, e.g. "Recon". */
  roleLabel: string;
  /** Coverage technique columns for this stage (config-driven). */
  coverageAxis: string[];
  /**
   * Whether the parent `stage_run` tool is still active. When false, live
   * running/queued rows are projected into a stopped display state so stale
   * progress snapshots do not look like active work.
   */
  isActive?: boolean;
  /**
   * Drill from an org row into that org's specialist sub-agent detail. Given the
   * row's `agentRequestId`; only rows that carry one render as clickable. Wired
   * by {@link ToolCallDetailView} to open the sub-agent detail pane.
   */
  onDrillIn?: (agentRequestId: string) => void;
}

/**
 * Render a `stage_run`'s per-org fan-out as a compact summary line + one card per
 * org. Embedded in {@link ToolCallDetailView} under the standard tool detail.
 */
export function StageRunOrgRows({
  rows,
  summary,
  stageLabel,
  roleLabel,
  coverageAxis,
  isActive = true,
  onDrillIn,
}: StageRunOrgRowsProps) {
  const inactiveStoppedCount = isActive
    ? 0
    : rows.filter((row) => row.status === "running" || row.status === "queued").length;
  const displayRows: StageRunRow[] = isActive
    ? rows
    : rows.map((row): StageRunRow => {
        if (row.status !== "running" && row.status !== "queued") return row;
        return { ...row, status: "stopped", activity: undefined };
      });
  const displaySummary: StageRunSummary = isActive ? summary : { ...summary, active: 0, queued: 0 };
  const summaryText = [
    `${displaySummary.covered}/${displaySummary.total} passed`,
    displaySummary.active > 0 ? `${displaySummary.active} active` : null,
    displaySummary.queued > 0 ? `${displaySummary.queued} queued` : null,
    inactiveStoppedCount > 0 ? `${inactiveStoppedCount} stopped` : null,
    displaySummary.blocked > 0 ? `${displaySummary.blocked} blocked` : null,
  ]
    .filter(Boolean)
    .join(" · ");
  const workerCount = Math.max(displaySummary.total, displayRows.length);
  const workerText = `${workerCount} ${workerCount === 1 ? "worker" : "workers"}`;
  const workerLabel = roleAgentLabel(roleLabel);

  return (
    <div className="space-y-1.5">
      <div className="rounded-md border border-border/30 bg-muted/20 px-2.5 py-2">
        <div className="flex flex-wrap items-center gap-1.5 text-[11px]">
          <span className="font-semibold text-foreground/90">{stageLabel}</span>
          <span className="inline-flex items-center gap-1 rounded bg-muted/50 px-1.5 py-0.5 text-muted-foreground">
            <Workflow className="h-2.5 w-2.5" />
            Main Agent
          </span>
          <ChevronRight className="h-3 w-3 text-muted-foreground/40" />
          <span className="inline-flex items-center gap-1 rounded bg-cyan-500/15 px-1.5 py-0.5 font-medium text-cyan-300">
            <Bot className="h-2.5 w-2.5" />
            {workerLabel} · {workerText}
          </span>
        </div>
        <div className="mt-1 truncate text-[10px] text-muted-foreground/70">{summaryText}</div>
      </div>
      <div className="space-y-1.5">
        {displayRows.map((row) => (
          <CollectorCard
            key={row.id}
            row={row}
            roleLabel={roleLabel}
            coverageAxis={coverageAxis}
            onDrillIn={onDrillIn}
          />
        ))}
      </div>
    </div>
  );
}
