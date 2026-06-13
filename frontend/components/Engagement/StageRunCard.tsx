/**
 * StageRunCard — the compact card that lives in the CHAT stream for a stage run
 * (设计 `docs/design/2026-06-13-stage-run-fanout-design.md` §3.3). Clicking it
 * opens the full per-org {@link StageRunView} in the LEFT detail pane (reusing
 * the app's `detailViewMode` pattern, like SubAgentInlineCard). Keeps the chat
 * clean — one card — while the heavy synchronized view lives in the detail pane.
 *
 * Purely presentational + callback-driven.
 */

import { Target } from "lucide-react";
import { cn } from "@/lib/utils";
import type { StageRunSummary } from "./StageRunView";

export interface StageRunCardProps {
  /** Stage display name, e.g. "Target Intel". */
  stageLabel: string;
  /** Short tag, e.g. "Recon · 被动". Omit to hide. */
  roleTag?: string;
  summary: StageRunSummary;
  /** Whether the detail pane is currently showing this stage run. */
  open: boolean;
  onOpen: () => void;
}

export function StageRunCard({ stageLabel, roleTag, summary, open, onOpen }: StageRunCardProps) {
  const pct = summary.total > 0 ? Math.round((summary.covered / summary.total) * 100) : 0;
  return (
    <div className="mx-3 my-2">
      <button
        type="button"
        onClick={onOpen}
        className={cn(
          "w-full rounded-lg border bg-background/50 p-2.5 text-left transition-colors hover:border-primary/50",
          open ? "border-primary/50 ring-1 ring-primary/30" : "border-border/50"
        )}
      >
        <div className="flex items-center gap-2">
          <Target className="h-4 w-4 shrink-0 text-primary" />
          <span className="text-[12.5px] font-semibold">{stageLabel}</span>
          {roleTag && (
            <span className="rounded bg-cyan-500/15 px-1.5 py-0.5 text-[10px] font-medium text-cyan-300">
              {roleTag}
            </span>
          )}
          <span className="flex-1" />
          <span className="text-[11px] text-primary">{open ? "查看中" : "在详情查看 →"}</span>
        </div>
        <div className="mt-1 text-[11px] text-muted-foreground">
          {summary.total} 家并行 · {summary.covered}/{summary.total} covered · {summary.active}{" "}
          active
          {summary.blocked > 0 ? ` · ${summary.blocked} blocked` : ""}
        </div>
        <div className="mt-1.5 h-1.5 w-full overflow-hidden rounded bg-muted/40">
          <div className="h-full bg-emerald-500/60" style={{ width: `${pct}%` }} />
        </div>
      </button>
    </div>
  );
}
