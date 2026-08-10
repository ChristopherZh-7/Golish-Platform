/**
 * StageRow
 *
 * One harness stage in the task-mode roadmap. Every stage — future / active /
 * passed — shares one compact, border-light visual anatomy:
 *
 *   [status icon]  Stage Name   c/t   ⌄
 *
 * The stage name leads and "c/t" remains secondary. No row auto-expands: the
 * roadmap itself is opt-in, and each stage's local steps require another
 * explicit click.
 *
 * Seed vs real plan: the backend emits a `version: 0` seed for every stage (a
 * roadmap placeholder, and an `in_progress` marker at stage entry). A seed's step
 * title is *synthesized*, not a real todo — so we only surface the step list /
 * counts once the agent emits a real `update_plan` (version >= 1). Before that an
 * active stage just shows "starting…", so the card never displays a fake todo
 * while the agent is still thinking.
 *
 * Completion is authoritative: a stage shows the green check only when its
 * evidence gate PASSED (`passed`, from the backend `stage_passed` signal) — NOT
 * when the model self-reports its todos done.
 */
import { CheckCircle2, ChevronRight, Loader2 } from "lucide-react";
import { memo, useState } from "react";
import { cn } from "@/lib/utils";
import { StepRow } from "./PlanStepRow";
import { prettyStageName } from "./StageMarker";
import type { TaskPlanViewModel } from "./TaskPlan";

export const StageRow = memo(function StageRow({
  stageId,
  plan,
  passed,
  isRunning,
}: {
  stageId: string;
  plan: TaskPlanViewModel;
  passed: boolean;
  /**
   * Whether the agent is actually live (the conversation is streaming) right now.
   * The spinner only animates when this is true — a stage can be "active" purely
   * from restored plan state (app restart / between turns / stopped run), and an
   * animate-spin there would falsely imply work is in progress. When active but
   * idle we show a static "current" ring instead, until the user resumes.
   */
  isRunning: boolean;
}) {
  // A real `update_plan` bumps version to >= 1; version 0 is a backend seed
  // (roadmap placeholder or stage-entry marker). Only real plans get the detail
  // (step list + counts) — a seed's synthesized title must not show as a fake todo.
  const hasRealPlan = plan.version >= 1;
  const future = !passed && !hasRealPlan && plan.steps.every((s) => s.status === "pending");
  const active = !passed && !future;
  const showDetail = hasRealPlan;

  // The roadmap is secondary detail: even the active stage remains collapsed
  // until the user explicitly asks for its local plan.
  const [expanded, setExpanded] = useState(false);

  const { total, completed } = plan.summary;

  const statusIcon = passed ? (
    <CheckCircle2 className="w-3.5 h-3.5 text-green-500 flex-shrink-0" />
  ) : active && isRunning ? (
    <Loader2 className="w-3.5 h-3.5 text-accent animate-spin flex-shrink-0" />
  ) : active ? (
    // Active stage, but the agent is NOT live (restored after restart, between
    // turns, or a stopped run): static "current" ring, never a spinner.
    <div className="w-3.5 h-3.5 rounded-full border-[1.5px] border-accent/50 flex-shrink-0" />
  ) : (
    <div className="w-3.5 h-3.5 rounded-full border-[1.5px] border-muted-foreground/30 flex-shrink-0" />
  );

  return (
    <div
      className={cn(
        "overflow-hidden rounded-md border-l-2",
        active
          ? "border-accent/55 bg-accent/[0.045]"
          : "border-[var(--border-subtle)]/55 bg-transparent"
      )}
    >
      <button
        type="button"
        aria-expanded={showDetail ? expanded : undefined}
        onClick={showDetail ? () => setExpanded((open) => !open) : undefined}
        disabled={!showDetail}
        className={cn(
          "flex min-h-8 w-full items-center gap-2 px-2 py-1 text-left",
          showDetail && "hover:bg-accent/[0.05] transition-colors cursor-pointer"
        )}
      >
        {statusIcon}
        <span
          className={cn(
            "text-[12px] font-medium truncate",
            future ? "text-muted-foreground/55" : "text-foreground"
          )}
        >
          {prettyStageName(stageId)}
        </span>
        {/* Seed-only active stage: no fake todo, just a faint "starting…" — but
            only while the agent is actually live; an idle/restored seed shows
            nothing (the static "current" ring already marks where you are). */}
        {active && !showDetail && isRunning && (
          <span className="text-[10.5px] text-muted-foreground/45 truncate">starting…</span>
        )}
        {showDetail && !passed && total > 0 && (
          <span className="text-[10.5px] text-muted-foreground/50 tabular-nums flex-shrink-0">
            {completed}/{total}
          </span>
        )}
        {showDetail && (
          <ChevronRight
            className={cn(
              "w-3 h-3 text-muted-foreground/50 transition-transform flex-shrink-0 ml-auto",
              expanded && "rotate-90"
            )}
          />
        )}
      </button>

      {expanded && (
        <div className="px-2 pb-1.5 pl-4">
          {plan.steps.map((step, i) => (
            <StepRow key={`${i}-${step.step}`} step={step} index={i} />
          ))}
        </div>
      )}
    </div>
  );
});
