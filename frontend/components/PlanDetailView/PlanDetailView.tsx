import {
  ArrowLeft,
  CheckCircle2,
  Circle,
  ChevronDown,
  ChevronRight,
  Loader2,
  Target,
} from "lucide-react";
import { memo, useCallback, useMemo, useState } from "react";
import { ToolExecutionCard } from "@/components/ToolExecutionCard";
import { cn } from "@/lib/utils";
import type { AiToolExecution, TaskPlan, UnifiedBlock } from "@/store";
import { useStore } from "@/store";

interface PlanDetailViewProps {
  sessionId: string;
  plan: TaskPlan;
}

function StepStatusIcon({ status }: { status: string }) {
  switch (status) {
    case "completed":
      return <CheckCircle2 className="w-4 h-4 text-emerald-400 flex-shrink-0" />;
    case "in_progress":
      return <Loader2 className="w-4 h-4 text-blue-400 animate-spin flex-shrink-0" />;
    default:
      return <Circle className="w-4 h-4 text-muted-foreground/40 flex-shrink-0" />;
  }
}

/**
 * Groups tool executions by plan step. Uses planStepIndex when tagged,
 * otherwise infers from the ordering of update_plan calls in the timeline.
 */
function groupExecutionsByStep(
  timeline: UnifiedBlock[],
  stepCount: number,
): AiToolExecution[][] {
  const groups: AiToolExecution[][] = Array.from({ length: stepCount }, () => []);

  // Track inferred step index from update_plan call ordering
  let inferredStepIdx = 0;

  for (const block of timeline) {
    if (block.type !== "ai_tool_execution") continue;

    if (block.data.toolName === "update_plan") {
      inferredStepIdx++;
      continue;
    }

    // Use explicit tag if available, otherwise infer from update_plan position
    const stepIdx =
      block.data.planStepIndex !== undefined
        ? block.data.planStepIndex
        : Math.min(Math.max(inferredStepIdx - 1, 0), stepCount - 1);

    if (stepIdx >= 0 && stepIdx < stepCount) {
      groups[stepIdx].push(block.data);
    }
  }
  return groups;
}

function StepRow({
  step,
  stepIndex,
  executions,
}: {
  step: { step: string; status: string };
  stepIndex: number;
  executions: AiToolExecution[];
}) {
  const [expanded, setExpanded] = useState(false);
  const isActive = step.status === "in_progress";
  const isDone = step.status === "completed";
  const hasContent = executions.length > 0;

  const handleToggle = useCallback(() => {
    if (hasContent) setExpanded((prev) => !prev);
  }, [hasContent]);

  return (
    <div
      className={cn(
        "transition-colors",
        isActive && "bg-blue-500/5",
      )}
    >
      <button
        type="button"
        onClick={handleToggle}
        disabled={!hasContent}
        className={cn(
          "w-full px-4 py-3 flex items-start gap-3 text-left",
          hasContent && "cursor-pointer hover:bg-accent/5",
          !hasContent && "cursor-default",
        )}
      >
        <div className="mt-0.5 flex-shrink-0">
          {hasContent ? (
            expanded ? (
              <ChevronDown className="w-4 h-4 text-muted-foreground" />
            ) : (
              <ChevronRight className="w-4 h-4 text-muted-foreground" />
            )
          ) : (
            <StepStatusIcon status={step.status} />
          )}
        </div>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            {hasContent && <StepStatusIcon status={step.status} />}
            <span className="text-[10px] text-muted-foreground/50 tabular-nums font-mono">
              {stepIndex + 1}
            </span>
            <span
              className={cn(
                "text-sm",
                isDone && "text-foreground/60",
                isActive && "text-foreground font-medium",
                !isDone && !isActive && "text-muted-foreground/70",
              )}
            >
              {step.step}
            </span>
          </div>
          {isActive && !expanded && (
            <div className="mt-1.5 flex items-center gap-1.5 ml-6">
              <div className="w-1.5 h-1.5 rounded-full bg-blue-400 animate-pulse" />
              <span className="text-[11px] text-blue-400/80">Running...</span>
            </div>
          )}
        </div>
        {hasContent && (
          <span className="text-[10px] text-muted-foreground/40 flex-shrink-0 tabular-nums">
            {executions.length} action{executions.length > 1 ? "s" : ""}
          </span>
        )}
      </button>

      {expanded && hasContent && (
        <div className="px-4 pb-3 pl-11">
          <div className="space-y-1">
            {executions.map((exec) => (
              <ToolExecutionCard key={exec.requestId} execution={exec} />
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

export const PlanDetailView = memo(function PlanDetailView({
  sessionId,
  plan,
}: PlanDetailViewProps) {
  const setDetailViewMode = useStore((s) => s.setDetailViewMode);
  const timeline = useStore((s) => s.timelines[sessionId] ?? []);

  const { completed, inProgress, total, progressPct } = useMemo(() => {
    const c = plan.steps.filter((s) => s.status === "completed").length;
    const ip = plan.steps.filter((s) => s.status === "in_progress").length;
    const t = plan.steps.length;
    return {
      completed: c,
      inProgress: ip,
      total: t,
      progressPct: t > 0 ? (c / t) * 100 : 0,
    };
  }, [plan.steps]);

  const isAllDone = completed === total && total > 0;

  const stepExecutions = useMemo(
    () => groupExecutionsByStep(timeline, plan.steps.length),
    [timeline, plan.steps.length],
  );

  return (
    <div className="h-full flex flex-col bg-background">
      {/* Header */}
      <div className="flex items-center gap-2 px-3 py-2 border-b border-[var(--border-subtle)]">
        <button
          type="button"
          onClick={() => setDetailViewMode(sessionId, "timeline")}
          className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground transition-colors"
        >
          <ArrowLeft className="w-3.5 h-3.5" />
          Back to Terminal
        </button>
        <div className="flex-1" />
        <span className="text-[10px] text-muted-foreground tabular-nums">
          {completed}/{total} steps
        </span>
      </div>

      {/* Plan title + progress */}
      <div className="px-4 py-3 border-b border-[var(--border-subtle)]">
        <div className="flex items-center gap-2 mb-2">
          <Target className="w-4 h-4 text-accent" />
          <span className="text-sm font-medium text-foreground">
            {plan.explanation ?? "Task Plan"}
          </span>
          {isAllDone && (
            <span className="text-[10px] px-1.5 py-0.5 rounded bg-emerald-500/15 text-emerald-400 font-medium">
              Complete
            </span>
          )}
        </div>
        <div className="h-1.5 bg-muted/30 rounded-full overflow-hidden">
          <div
            className={cn(
              "h-full transition-all duration-500 ease-out rounded-full",
              isAllDone ? "bg-emerald-400" : "bg-[#7aa2f7]",
            )}
            style={{ width: `${progressPct}%` }}
          />
        </div>
      </div>

      {/* Steps list */}
      <div className="flex-1 overflow-y-auto">
        <div className="divide-y divide-[var(--border-subtle)]">
          {plan.steps.map((step, idx) => (
            <StepRow
              key={`step-${idx}-${step.step}`}
              step={step}
              stepIndex={idx}
              executions={stepExecutions[idx]}
            />
          ))}
        </div>
      </div>

      {/* Footer summary */}
      {inProgress > 0 && (
        <div className="px-4 py-2 border-t border-[var(--border-subtle)] bg-blue-500/5">
          <div className="flex items-center gap-2">
            <Loader2 className="w-3 h-3 text-blue-400 animate-spin" />
            <span className="text-[11px] text-blue-400/80">
              {inProgress} step{inProgress > 1 ? "s" : ""} running...
            </span>
          </div>
        </div>
      )}
    </div>
  );
});
