/**
 * InlinePlanCard
 *
 * Windsurf-style inline plan card rendered inside the message stream (chat mode /
 * non-harness single plan). Collapsed: shows "N / M tasks done", last completed
 * step + current in-progress step. Expanded: shows all steps.
 *
 * Task-mode per-stage cards use `StageRow` instead; both share `PlanStepRow` for
 * the step rows.
 */
import { CheckCircle2, ChevronRight, Loader2 } from "lucide-react";
import { memo, useCallback, useState } from "react";
import { cn } from "@/lib/utils";
import { StepRow } from "./PlanStepRow";
import type { TaskPlanViewModel } from "./TaskPlan";

export const InlinePlanCard = memo(function InlinePlanCard({ plan }: { plan: TaskPlanViewModel }) {
  const [expanded, setExpanded] = useState(false);
  const toggle = useCallback(() => setExpanded((v) => !v), []);

  const { steps, summary } = plan;
  const { total, completed } = summary;
  const isDone = total > 0 && completed === total;

  const lastCompletedIdx = steps.reduce((acc, s, i) => (s.status === "completed" ? i : acc), -1);
  const currentIdx = steps.findIndex((s) => s.status === "in_progress");

  const visibleIndices: number[] = [];
  if (!expanded) {
    if (lastCompletedIdx >= 0) visibleIndices.push(lastCompletedIdx);
    if (currentIdx >= 0 && currentIdx !== lastCompletedIdx) visibleIndices.push(currentIdx);
    if (visibleIndices.length === 0 && steps.length > 0) {
      visibleIndices.push(0);
    }
  }

  // Guard the empty case: spreading an empty array into Math.min/Math.max
  // yields Infinity/-Infinity, which previously rendered as "Infinity more".
  const hasVisible = visibleIndices.length > 0;
  const beforeCount = expanded || !hasVisible ? 0 : Math.max(0, Math.min(...visibleIndices));
  const afterCount =
    expanded || !hasVisible ? 0 : Math.max(0, steps.length - 1 - Math.max(...visibleIndices));

  return (
    <div className="mx-0 my-1.5 rounded-lg border border-[var(--border-subtle)] bg-background/60 overflow-hidden">
      {/* Header */}
      <button
        type="button"
        onClick={toggle}
        className="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-accent/[0.05] transition-colors group"
      >
        {isDone ? (
          <CheckCircle2 className="w-3.5 h-3.5 text-green-500 flex-shrink-0" />
        ) : (
          <Loader2 className="w-3 h-3 text-accent animate-spin flex-shrink-0" />
        )}
        <span className="text-[12px] font-medium text-foreground">
          {completed} / {total} tasks done
        </span>
        <ChevronRight
          className={cn(
            "w-3 h-3 text-muted-foreground/50 transition-transform flex-shrink-0",
            expanded && "rotate-90"
          )}
        />
      </button>

      {/* Step list */}
      <div className="px-2 pb-1.5">
        {expanded ? (
          steps.map((step, i) => <StepRow key={`${i}-${step.step}`} step={step} index={i} />)
        ) : (
          <>
            {beforeCount > 0 && (
              <button
                type="button"
                onClick={toggle}
                className="w-full text-left px-2 py-0.5 text-[11px] text-muted-foreground/50 hover:text-muted-foreground transition-colors"
              >
                {beforeCount} more
              </button>
            )}
            {visibleIndices.map((i) => (
              <StepRow key={`${i}-${steps[i].step}`} step={steps[i]} index={i} />
            ))}
            {afterCount > 0 && (
              <button
                type="button"
                onClick={toggle}
                className="w-full text-left px-2 py-0.5 text-[11px] text-muted-foreground/50 hover:text-muted-foreground transition-colors"
              >
                {afterCount} more
              </button>
            )}
          </>
        )}
      </div>
    </div>
  );
});
