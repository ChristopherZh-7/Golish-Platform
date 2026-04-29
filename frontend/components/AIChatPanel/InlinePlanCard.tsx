/**
 * InlinePlanCard
 *
 * Windsurf-style inline plan card rendered inside the message stream.
 * Collapsed: shows "N / M tasks done", last completed step + current in-progress step.
 * Expanded: shows all steps.
 */
import { CheckCircle2, ChevronRight, Loader2, XCircle } from "lucide-react";
import { memo, useCallback, useState } from "react";
import { cn } from "@/lib/utils";
import type { TaskPlanViewModel } from "./TaskPlan";

function StepIcon({ status, index }: { status: string; index: number }) {
  switch (status) {
    case "completed":
      return <CheckCircle2 className="w-3.5 h-3.5 text-green-500 flex-shrink-0" />;
    case "in_progress":
      return (
        <span className="inline-flex items-center justify-center w-4 h-4 rounded-full bg-accent/90 text-[10px] font-bold text-accent-foreground flex-shrink-0 tabular-nums">
          {index + 1}
        </span>
      );
    case "failed":
    case "cancelled":
      return <XCircle className="w-3.5 h-3.5 text-red-400/70 flex-shrink-0" />;
    default:
      return (
        <span className="inline-flex items-center justify-center w-4 h-4 rounded-full border-[1.5px] border-muted-foreground/25 text-[10px] text-muted-foreground/50 flex-shrink-0 tabular-nums">
          {index + 1}
        </span>
      );
  }
}

function StepRow({ step, index }: { step: { step: string; status: string }; index: number }) {
  const isCompleted = step.status === "completed";
  const isInProgress = step.status === "in_progress";
  const isFailed = step.status === "failed" || step.status === "cancelled";

  return (
    <div
      className={cn(
        "flex items-center gap-2 py-1 px-2 rounded text-[11.5px]",
        isInProgress && "bg-accent/[0.06]",
      )}
    >
      <StepIcon status={step.status} index={index} />
      <span
        className={cn(
          "flex-1 truncate leading-relaxed",
          isCompleted && "text-muted-foreground/70",
          isInProgress && "font-semibold text-foreground",
          isFailed && "line-through text-red-400/70",
          !isCompleted && !isInProgress && !isFailed && "text-muted-foreground/50",
        )}
      >
        {step.step}
      </span>
    </div>
  );
}

export const InlinePlanCard = memo(function InlinePlanCard({
  plan,
}: {
  plan: TaskPlanViewModel;
}) {
  const [expanded, setExpanded] = useState(false);
  const toggle = useCallback(() => setExpanded((v) => !v), []);

  const { steps, summary } = plan;
  const { total, completed } = summary;
  const isDone = total > 0 && completed === total;

  const lastCompletedIdx = steps.reduce(
    (acc, s, i) => (s.status === "completed" ? i : acc),
    -1,
  );
  const currentIdx = steps.findIndex((s) => s.status === "in_progress");

  const visibleIndices: number[] = [];
  if (!expanded) {
    if (lastCompletedIdx >= 0) visibleIndices.push(lastCompletedIdx);
    if (currentIdx >= 0 && currentIdx !== lastCompletedIdx) visibleIndices.push(currentIdx);
    if (visibleIndices.length === 0 && steps.length > 0) {
      visibleIndices.push(0);
    }
  }

  const beforeCount = expanded ? 0 : Math.max(0, Math.min(...visibleIndices));
  const afterCount = expanded
    ? 0
    : Math.max(0, steps.length - 1 - Math.max(...visibleIndices));

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
            expanded && "rotate-90",
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
