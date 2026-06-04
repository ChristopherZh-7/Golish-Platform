/**
 * StageProgressBar
 *
 * Cursor-style "you-are-here" header for the task-mode roadmap (design 2026-06-04
 * · roadmap UX overhaul). The full `StagePlanStack` is anchored inline where the
 * plan first appeared; this bar is an OVERLAY the panel reveals only once that
 * inline roadmap has scrolled out of view above (see `AIChatPanel`'s
 * IntersectionObserver on `[data-stage-roadmap]`).
 *
 * Clicking the bar expands the CURRENT stage's full plan (its todos) in place —
 * not a scroll-to-top jump — so the user can inspect where they are without
 * leaving their scroll position.
 *
 * Derived entirely on the frontend from the per-stage view model — no new events.
 */
import { CheckCircle2, ChevronDown, Loader2 } from "lucide-react";
import { memo, useMemo, useState } from "react";
import { cn } from "@/lib/utils";
import { StepRow } from "./PlanStepRow";
import { prettyStageName } from "./StageMarker";
import type { StagePlansViewModel } from "./TaskPlan";

export const StageProgressBar = memo(function StageProgressBar({
  stagePlans,
  isRunning,
  className,
}: {
  stagePlans: StagePlansViewModel;
  /**
   * Whether the agent is actually live (the conversation is streaming) now. The
   * "you-are-here" spinner only animates while true; on a restored/idle run the
   * bar shows a static "current" ring so it doesn't look like work is happening.
   */
  isRunning: boolean;
  className?: string;
}) {
  const { stageOrder, plansByStage, passedStages } = stagePlans;
  const [expanded, setExpanded] = useState(false);

  const view = useMemo(() => {
    if (stageOrder.length === 0) return null;
    const passed = new Set(passedStages);
    // Current = first not-yet-passed stage; prefer one already live (has a
    // non-pending step), else the next one queued, else the last (all passed).
    let idx = stageOrder.findIndex(
      (id) =>
        !passed.has(id) && (plansByStage[id]?.steps.some((s) => s.status !== "pending") ?? false)
    );
    if (idx < 0) idx = stageOrder.findIndex((id) => !passed.has(id));
    if (idx < 0) idx = stageOrder.length - 1;
    const stageId = stageOrder[idx];
    const plan = plansByStage[stageId];
    // Only surface concrete steps once the agent has emitted a real `update_plan`
    // (version >= 1). A version-0 seed's title is synthesized, so showing it would
    // display a fake "step x/y" while the agent is still thinking.
    const hasRealPlan = (plan?.version ?? 0) >= 1;
    const steps = hasRealPlan ? (plan?.steps ?? []) : [];
    const step = hasRealPlan
      ? (plan?.steps.find((s) => s.status === "in_progress") ??
        plan?.steps.find((s) => s.status === "pending"))
      : undefined;
    return {
      idx,
      stageId,
      step,
      steps,
      total: plan?.summary.total ?? 0,
      completed: plan?.summary.completed ?? 0,
      passedCount: passed.size,
      allPassed: passed.size >= stageOrder.length,
    };
  }, [stageOrder, plansByStage, passedStages]);

  if (!view) return null;

  const canExpand = view.steps.length > 0;
  const isOpen = expanded && canExpand;
  const progress =
    stageOrder.length > 0 ? Math.round((view.passedCount / stageOrder.length) * 100) : 0;

  return (
    <div
      className={cn(
        // Snap-in feel as it crosses the scrolled-past threshold.
        "animate-in fade-in slide-in-from-top-1 duration-150 bg-background/95 backdrop-blur-md shadow-[0_8px_18px_-14px_rgba(0,0,0,0.7)]",
        className
      )}
    >
      <button
        type="button"
        onClick={canExpand ? () => setExpanded((e) => !e) : undefined}
        disabled={!canExpand}
        title={canExpand ? "Show this stage's plan" : undefined}
        className={cn(
          "group w-full flex items-center gap-2 px-3 py-2 text-left transition-colors",
          canExpand && "cursor-pointer hover:bg-accent/[0.06]"
        )}
      >
        {view.allPassed ? (
          <CheckCircle2 className="w-3.5 h-3.5 text-green-500 flex-shrink-0" />
        ) : isRunning ? (
          <Loader2 className="w-3.5 h-3.5 text-accent animate-spin flex-shrink-0" />
        ) : (
          // Restored / idle / stopped run: static "current" ring, no spinner.
          <div className="w-3.5 h-3.5 rounded-full border-[1.5px] border-accent/50 flex-shrink-0" />
        )}
        <span className="text-[11.5px] font-semibold text-foreground truncate flex-shrink-0">
          {prettyStageName(view.stageId)}
        </span>
        <span className="text-[10px] text-muted-foreground/55 tabular-nums flex-shrink-0">
          {view.idx + 1}/{stageOrder.length}
        </span>
        {view.step && (
          <span className="min-w-0 flex-1 truncate text-[11px] text-muted-foreground/65">
            {view.total > 0
              ? `· step ${Math.min(view.completed + 1, view.total)}/${view.total}: `
              : "· "}
            {view.step.step}
          </span>
        )}
        {canExpand && (
          <ChevronDown
            className={cn(
              "w-3.5 h-3.5 text-muted-foreground/45 group-hover:text-accent/80 transition-transform flex-shrink-0 ml-auto",
              isOpen && "rotate-180"
            )}
          />
        )}
      </button>

      {/* Click-to-expand: the current stage's full plan (todos) in place. */}
      {isOpen && (
        <div className="px-2 pb-1.5 pt-0.5 max-h-64 overflow-y-auto border-t border-[var(--border-subtle)]/60">
          {view.steps.map((step, i) => (
            <StepRow key={`${i}-${step.step}`} step={step} index={i} />
          ))}
        </div>
      )}

      {/* Overall roadmap progress (passed stages / total) — Cursor-like thin track. */}
      <div className="h-0.5 w-full bg-[var(--border-subtle)]/40">
        <div
          className="h-full bg-accent/70 transition-[width] duration-500 ease-out"
          style={{ width: `${progress}%` }}
        />
      </div>
    </div>
  );
});
