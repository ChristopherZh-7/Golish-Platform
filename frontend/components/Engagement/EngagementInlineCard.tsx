/**
 * EngagementInlineCard — the engagement control surface rendered INLINE in the
 * chat message stream (woven between bubbles, like {@link AskHumanInline}),
 * instead of as a panel pinned above the messages.
 *
 * Design: 2026-06-13-engagement-scoping-fanout §2.2「总览要长在 chat 流里」+ §5④.
 * Each org is rendered as a WORKER card that reuses the SubAgentInlineCard visual
 * language (status glyph + phase chip + name + a single live activity line +
 * tool count + 详情→), so the "engagement → worker → sub-agent → tool" hierarchy
 * reads with zero new vocabulary. This card lives in the ORCHESTRATOR
 * conversation (the scoping chat that locked the scope). It only scopes + fans
 * out + shows per-org worker status; the actual recon / attack / sub-agent
 * delegation (e.g. 委托给 Pentester) happens one level down, inside each spawned
 * WORKER conversation, reached by clicking a card to drill in. Keeping the two
 * levels in separate conversations is what stops the "orchestrator chat is also
 * delegating to Pentester" weirdness.
 *
 * Purely presentational + callback-driven so it is trivial to unit-test. The
 * live wiring (DB-truth snapshot + worker pool) is supplied by the caller
 * (see EngagementOverview).
 */

import {
  AlertTriangle,
  Building2,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Circle,
  Clock,
  Loader2,
  Play,
  Radar,
  Square,
  Swords,
  Target,
  XCircle,
} from "lucide-react";
import { cn } from "@/lib/utils";

/** Pool-aware display status (superset of the DB snapshot's status). */
export type EngagementRowStatus =
  | "passed"
  | "skippedAlreadyComplete"
  | "running"
  | "queued"
  | "blocked"
  | "failed"
  | "pending";

/** The worker phase a row's conversation is in (recon = family, attack = per-org). */
export type EngagementWorkerPhase = "recon" | "attack";

const STATUS_META: Record<EngagementRowStatus, { label: string; className: string }> = {
  passed: { label: "Covered", className: "bg-emerald-500/15 text-emerald-400" },
  skippedAlreadyComplete: { label: "Covered", className: "bg-emerald-500/15 text-emerald-400" },
  running: { label: "Running", className: "bg-sky-500/15 text-sky-400" },
  queued: { label: "Queued", className: "bg-indigo-500/15 text-indigo-400" },
  blocked: { label: "Blocked", className: "bg-amber-500/15 text-amber-400" },
  failed: { label: "Failed", className: "bg-red-500/15 text-red-400" },
  pending: { label: "Pending", className: "bg-slate-500/15 text-slate-400" },
};

const PHASE_META: Record<EngagementWorkerPhase, { label: string; className: string }> = {
  recon: { label: "recon", className: "bg-cyan-500/15 text-cyan-300" },
  attack: { label: "attack", className: "bg-rose-500/15 text-rose-300" },
};

/** Left-edge glyph mirroring SubAgentInlineCard's StatusGlyph, per row status. */
function WorkerGlyph({ status }: { status: EngagementRowStatus }) {
  switch (status) {
    case "running":
      return <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-sky-400" />;
    case "passed":
    case "skippedAlreadyComplete":
      return <CheckCircle2 className="h-3.5 w-3.5 shrink-0 text-emerald-400" />;
    case "queued":
      return <Clock className="h-3.5 w-3.5 shrink-0 text-indigo-400" />;
    case "blocked":
      return <AlertTriangle className="h-3.5 w-3.5 shrink-0 text-amber-400" />;
    case "failed":
      return <XCircle className="h-3.5 w-3.5 shrink-0 text-red-400" />;
    default:
      return <Circle className="h-3.5 w-3.5 shrink-0 text-slate-500" />;
  }
}

function PhaseChip({ phase }: { phase: EngagementWorkerPhase }) {
  const meta = PHASE_META[phase];
  const Icon = phase === "recon" ? Radar : Swords;
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center gap-1 rounded px-1.5 py-0.5 text-[10px] font-medium",
        meta.className
      )}
    >
      <Icon className="h-2.5 w-2.5" />
      {meta.label}
    </span>
  );
}

function StatusBadge({ status }: { status: EngagementRowStatus }) {
  const meta = STATUS_META[status] ?? STATUS_META.pending;
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center gap-1 rounded px-2 py-0.5 text-[11px] font-medium",
        meta.className
      )}
    >
      {status === "running" && <Loader2 className="h-3 w-3 animate-spin" />}
      {meta.label}
    </span>
  );
}

/** One flattened org row (pre-order, honouring expand state). */
export interface EngagementCardRow {
  id: string;
  name: string;
  depth: number;
  ownershipPercent: number | null;
  status: EngagementRowStatus;
  hasChildren: boolean;
  expanded: boolean;
  /** True once a worker conversation exists for this org (row drills in). */
  drillable: boolean;
  /** Worker phase label (recon / attack). Omit for not-yet-spawned orgs. */
  phase?: EngagementWorkerPhase;
  /** Live one-liner of what this worker is doing right now (running only). */
  activity?: string;
  /** Tools booked into the evidence ledger by this worker so far. */
  toolCount?: number;
}

export interface EngagementCardSummary {
  totalOrgs: number;
  covered: number;
  active: number;
  queued: number;
  blocked: number;
  failed: number;
}

function WorkerRow({
  row,
  onToggleRow,
  onDrillIn,
}: {
  row: EngagementCardRow;
  onToggleRow: (id: string) => void;
  onDrillIn: (id: string) => void;
}) {
  const interactive = row.drillable;
  return (
    <div style={{ marginLeft: row.depth * 14 }}>
      <div
        onClick={() => interactive && onDrillIn(row.id)}
        className={cn(
          "group rounded-lg border bg-background/50 p-2.5 transition-colors",
          interactive && "cursor-pointer hover:border-primary/40",
          row.status === "running"
            ? "border-l-2 border-l-sky-400/80"
            : row.status === "blocked"
              ? "border-amber-500/40"
              : row.status === "failed"
                ? "border-red-500/40"
                : "border-border/40"
        )}
      >
        <div className="flex items-center gap-2">
          {row.hasChildren ? (
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                onToggleRow(row.id);
              }}
              className="shrink-0 text-muted-foreground hover:text-foreground"
            >
              {row.expanded ? (
                <ChevronDown className="h-3 w-3" />
              ) : (
                <ChevronRight className="h-3 w-3" />
              )}
            </button>
          ) : (
            <span className="w-3 shrink-0" />
          )}
          <WorkerGlyph status={row.status} />
          {row.phase && <PhaseChip phase={row.phase} />}
          <Building2 className="h-3 w-3 shrink-0 text-muted-foreground/60" />
          <span className="min-w-0 truncate text-[12px] font-medium" title={row.name}>
            {row.name}
          </span>
          {row.ownershipPercent != null && (
            <span className="shrink-0 text-[10px] text-muted-foreground/70">
              {row.ownershipPercent}%
            </span>
          )}
          <div className="flex-1" />
          <StatusBadge status={row.status} />
          {interactive && (
            <span className="flex shrink-0 items-center gap-0.5 text-[10px] text-muted-foreground/55 transition-colors group-hover:text-primary/70">
              详情
              <ChevronRight className="h-2.5 w-2.5" />
            </span>
          )}
        </div>

        {row.status === "running" && row.activity && (
          <div className="ml-5 mt-1.5 flex min-w-0 items-center gap-1.5 text-[11px] text-sky-400/90">
            <span
              className="inline-block h-1.5 w-1.5 shrink-0 animate-pulse rounded-full bg-sky-400"
              aria-hidden="true"
            />
            <span className="truncate" title={row.activity}>
              正在 {row.activity}
            </span>
          </div>
        )}

        {row.toolCount != null && row.toolCount > 0 && (
          <div className="ml-5 mt-1 text-[10px] tabular-nums text-muted-foreground/55">
            {row.toolCount} 工具
          </div>
        )}
      </div>
    </div>
  );
}

export interface EngagementInlineCardProps {
  rows: EngagementCardRow[];
  summary: EngagementCardSummary;
  /** Pool driving state — controls the Fan out / Stop affordance. */
  running: boolean;
  stopping?: boolean;
  concurrency: number;
  /** Whether the card body is collapsed (header still shows the summary). */
  collapsed?: boolean;
  /** Disable Fan out (no model picked / no scope) with a reason for the title. */
  canFanOut: boolean;
  fanOutDisabledReason?: string;
  onFanOut: () => void;
  onStop: () => void;
  onConcurrencyChange: (k: number) => void;
  onToggleRow: (id: string) => void;
  onDrillIn: (id: string) => void;
  onToggleCard?: () => void;
}

export function EngagementInlineCard({
  rows,
  summary,
  running,
  stopping = false,
  concurrency,
  collapsed = false,
  canFanOut,
  fanOutDisabledReason,
  onFanOut,
  onStop,
  onConcurrencyChange,
  onToggleRow,
  onDrillIn,
  onToggleCard,
}: EngagementInlineCardProps) {
  const summaryText = [
    `${summary.covered}/${summary.totalOrgs} covered`,
    summary.active > 0 ? `${summary.active} active` : null,
    summary.queued > 0 ? `${summary.queued} queued` : null,
    summary.blocked > 0 ? `${summary.blocked} blocked` : null,
    summary.failed > 0 ? `${summary.failed} failed` : null,
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <div className="mx-4 my-2 overflow-hidden rounded-lg border border-border/60 bg-background/40 text-foreground">
      {/* Header — mirrors a chat tool/result card; click to collapse. */}
      <button
        type="button"
        onClick={onToggleCard}
        className="flex w-full items-center gap-2 px-3 py-2 text-left hover:bg-muted/30"
      >
        <Target className="h-3.5 w-3.5 shrink-0 text-primary" />
        <span className="text-xs font-semibold">Engagement</span>
        <span className="rounded bg-emerald-500/15 px-1.5 py-0.5 text-[10px] font-medium text-emerald-400">
          {running ? "running" : "scope locked"}
        </span>
        <span className="min-w-0 flex-1 truncate text-[11px] text-muted-foreground">
          {summaryText}
        </span>
        {onToggleCard &&
          (collapsed ? (
            <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
          ) : (
            <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
          ))}
      </button>

      {!collapsed && (
        <>
          <div className="max-h-80 space-y-1.5 overflow-y-auto border-t border-border/50 p-2">
            {rows.map((row) => (
              <WorkerRow key={row.id} row={row} onToggleRow={onToggleRow} onDrillIn={onDrillIn} />
            ))}
          </div>

          {/* Footer — K concurrency + Fan out / Stop (the only CTA). */}
          <div className="flex items-center gap-2 border-t border-border/50 px-3 py-2">
            <label className="flex items-center gap-1 text-[11px] text-muted-foreground">
              K
              <input
                type="number"
                min={1}
                max={10}
                value={concurrency}
                disabled={running}
                onChange={(e) => {
                  const k = Math.max(1, Math.min(10, Number(e.target.value) || 1));
                  onConcurrencyChange(k);
                }}
                className="w-12 rounded border border-border bg-transparent px-1 py-0.5 text-xs disabled:opacity-50"
              />
            </label>
            <span className="flex-1" />
            {running ? (
              <button
                type="button"
                onClick={onStop}
                className="flex items-center gap-1 rounded border border-border px-2.5 py-1 text-xs text-amber-400 hover:bg-amber-500/10"
                title="Finish in-flight workers, stop dequeuing"
              >
                <Square className="h-3 w-3" />
                {stopping ? "Stopping…" : "Stop"}
              </button>
            ) : (
              <button
                type="button"
                onClick={onFanOut}
                disabled={!canFanOut}
                title={canFanOut ? "Fan out workers over the locked scope" : fanOutDisabledReason}
                className={cn(
                  "flex items-center gap-1 rounded px-2.5 py-1 text-xs font-medium",
                  canFanOut
                    ? "bg-primary text-primary-foreground hover:bg-primary/90"
                    : "cursor-not-allowed border border-border text-muted-foreground"
                )}
              >
                <Play className="h-3 w-3" />
                Fan out
              </button>
            )}
          </div>

          {summary.blocked > 0 && !running && (
            <div className="flex items-center gap-1.5 border-t border-border/50 px-3 py-1.5 text-[11px] text-amber-400">
              <AlertTriangle className="h-3 w-3 shrink-0" />
              {summary.blocked} org(s) blocked — drill in to review before re-running.
            </div>
          )}
        </>
      )}
    </div>
  );
}
