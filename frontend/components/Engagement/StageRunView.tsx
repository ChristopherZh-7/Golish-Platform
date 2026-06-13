/**
 * StageRunView — the synchronized per-org view for ONE harness stage's run,
 * rendered in the LEFT detail pane (设计 `docs/design/2026-06-13-stage-run-fanout-design.md`).
 *
 * Generic over the stage: a `stage run` fans the current stage out across every
 * in-scope org, one specialist per org (intel=Recon, EAS=Prober, attack=Pentester),
 * each AI-driven and gated by its own evidence. This view reuses the
 * SubAgentInlineCard visual vocabulary (status glyph + role chip + live activity
 * line + tool count + drill-in) so the hierarchy reads with zero new vocabulary.
 * Stage-specific bits (role label, coverage axis, stage label) are PROPS, driven
 * by per-stage config — no per-stage component.
 *
 * Purely presentational + callback-driven; the caller (live pool / preview mock)
 * supplies the data. Clicking a row expands its tool stream INLINE (no new tab).
 */

import {
  AlertTriangle,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Circle,
  Clock,
  Loader2,
  Radar,
} from "lucide-react";
import { cn } from "@/lib/utils";

export type StageRunStatus = "passed" | "running" | "queued" | "blocked" | "pending";

/** Per-technique terminal state on the coverage axis (mirrors the gate contract). */
export type TechniqueState = "found" | "checked_empty" | "blocked" | "pending";

export interface StageRunToolLine {
  name: string;
  detail?: string;
  done: boolean;
}

export interface StageRunRow {
  id: string;
  name: string;
  ownershipPercent: number | null;
  status: StageRunStatus;
  /** Live one-liner while running (e.g. "subfinder · pingan.com.cn"). */
  activity?: string;
  /** Evidence rows this org's specialist has booked into the ledger. */
  evidenceCount: number;
  /** Per-technique state, keyed by the stage's `coverageAxis` entries. */
  coverage: Record<string, TechniqueState>;
  /** Inline detail (the org's tool stream) is expanded. */
  expanded: boolean;
  /** Recent tool lines shown when expanded (drill-in, no new tab). */
  toolLines: StageRunToolLine[];
}

export interface StageRunSummary {
  total: number;
  covered: number;
  active: number;
  queued: number;
  blocked: number;
}

const STATUS_META: Record<StageRunStatus, { label: string; className: string }> = {
  passed: { label: "Covered", className: "bg-emerald-500/15 text-emerald-400" },
  running: { label: "Running", className: "bg-sky-500/15 text-sky-400" },
  queued: { label: "Queued", className: "bg-indigo-500/15 text-indigo-400" },
  blocked: { label: "Blocked", className: "bg-amber-500/15 text-amber-400" },
  pending: { label: "Pending", className: "bg-slate-500/15 text-slate-400" },
};

const TECH_META: Record<TechniqueState, { className: string; mark: string }> = {
  found: { className: "bg-emerald-500/15 text-emerald-300 border-emerald-500/30", mark: "✓" },
  checked_empty: { className: "bg-slate-500/15 text-slate-400 border-slate-500/30", mark: "∅" },
  blocked: { className: "bg-amber-500/15 text-amber-300 border-amber-500/30", mark: "!" },
  pending: { className: "bg-transparent text-muted-foreground/40 border-border/40", mark: "·" },
};

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
  onToggle,
}: {
  row: StageRunRow;
  roleLabel: string;
  coverageAxis: string[];
  onToggle: (id: string) => void;
}) {
  const meta = STATUS_META[row.status];
  const drillable = row.status !== "pending" && row.status !== "queued";
  return (
    <div
      className={cn(
        "rounded-lg border bg-background/50 p-2.5 transition-colors",
        row.status === "running"
          ? "border-l-2 border-l-sky-400/80"
          : row.status === "blocked"
            ? "border-amber-500/40"
            : "border-border/40"
      )}
    >
      <div className="flex items-center gap-2">
        <CollectorGlyph status={row.status} />
        <span className="inline-flex shrink-0 items-center gap-1 rounded bg-cyan-500/15 px-1.5 py-0.5 text-[10px] font-medium text-cyan-300">
          <Radar className="h-2.5 w-2.5" />
          {roleLabel}
        </span>
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
        {drillable && (
          <button
            type="button"
            onClick={() => onToggle(row.id)}
            className="flex shrink-0 items-center gap-0.5 text-[10px] text-muted-foreground/55 hover:text-primary/70"
          >
            详情
            {row.expanded ? (
              <ChevronDown className="h-2.5 w-2.5" />
            ) : (
              <ChevronRight className="h-2.5 w-2.5" />
            )}
          </button>
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

      <CoverageChips coverageAxis={coverageAxis} coverage={row.coverage} />

      {row.expanded && (
        <div className="ml-5 mt-2 space-y-1 rounded border border-border/40 bg-background/40 p-2">
          <div className="text-[10px] uppercase tracking-wide text-muted-foreground/50">
            工具流（内联，无需新 tab）
          </div>
          {row.toolLines.map((tl, i) => (
            <div
              key={`${row.id}-${i}`}
              className="flex items-center gap-1.5 text-[11px] text-muted-foreground/80"
            >
              {tl.done ? (
                <CheckCircle2 className="h-3 w-3 shrink-0 text-emerald-400" />
              ) : (
                <Loader2 className="h-3 w-3 shrink-0 animate-spin text-sky-400" />
              )}
              <span className="font-mono text-[10.5px]">{tl.name}</span>
              {tl.detail && (
                <span className="truncate text-muted-foreground/55">· {tl.detail}</span>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export interface StageRunViewProps {
  rows: StageRunRow[];
  summary: StageRunSummary;
  concurrency: number;
  /** Stage display name, e.g. "Target Intel". */
  stageLabel: string;
  /** Short tag, e.g. "被动 · zero-touch". Omit to hide. */
  stageTag?: string;
  /** The stage specialist's label shown on each row, e.g. "Recon". */
  roleLabel: string;
  /** Coverage technique columns for this stage (config-driven). */
  coverageAxis: string[];
  onToggleRow: (id: string) => void;
}

export function StageRunView({
  rows,
  summary,
  concurrency,
  stageLabel,
  stageTag,
  roleLabel,
  coverageAxis,
  onToggleRow,
}: StageRunViewProps) {
  const summaryText = [
    `${summary.covered}/${summary.total} covered`,
    summary.active > 0 ? `${summary.active} active` : null,
    summary.queued > 0 ? `${summary.queued} queued` : null,
    summary.blocked > 0 ? `${summary.blocked} blocked` : null,
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <div className="mx-4 my-2">
      {/* Stage marker — the chat flows INTO this stage (not an abrupt morph). */}
      <div className="mb-1.5 flex items-center gap-2 text-[12px]">
        <span className="font-semibold text-foreground">▶ {stageLabel}</span>
        {stageTag && (
          <span className="rounded bg-cyan-500/15 px-1.5 py-0.5 text-[10px] font-medium text-cyan-300">
            {stageTag}
          </span>
        )}
        <span className="min-w-0 flex-1 truncate text-[11px] text-muted-foreground">
          {summaryText} · K{concurrency} 并行
        </span>
      </div>

      <div className="space-y-1.5 rounded-lg border border-border/60 bg-background/30 p-2">
        {rows.map((row) => (
          <CollectorCard
            key={row.id}
            row={row}
            roleLabel={roleLabel}
            coverageAxis={coverageAxis}
            onToggle={onToggleRow}
          />
        ))}
      </div>
    </div>
  );
}
