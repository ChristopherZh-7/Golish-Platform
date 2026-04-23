import {
  ArrowLeft,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Circle,
  Loader2,
  XCircle,
} from "lucide-react";
import { memo, useCallback, useEffect, useMemo, useState } from "react";
import { PipelineProgressBlock } from "@/components/PipelineProgressBlock";
import { SubAgentCard } from "@/components/SubAgentCard";
import { ToolExecutionCard } from "@/components/ToolExecutionCard";
import { cn } from "@/lib/utils";
import type { ActiveSubAgent, AiToolExecution, PipelineExecution } from "@/store";
import type { PlanStep } from "@/store/store-types";
import { useStore } from "@/store";

type DetailItem =
  | { kind: "tool"; data: AiToolExecution }
  | { kind: "sub_agent"; data: ActiveSubAgent }
  | { kind: "pipeline"; data: PipelineExecution; id: string };

/** A single plan step row with its nested tool/pipeline/subagent cards. */
const PlanStepGroup = memo(function PlanStepGroup({
  step,
  items,
}: {
  step: PlanStep;
  items: DetailItem[];
}) {
  const isCompleted = step.status === "completed";
  const isInProgress = step.status === "in_progress";
  const isFailed = step.status === "cancelled" || step.status === "failed";
  const isPending = step.status === "pending";
  const hasItems = items.length > 0;

  const [open, setOpen] = useState(isInProgress || (!isCompleted && hasItems));

  useEffect(() => {
    if (isInProgress) setOpen(true);
  }, [isInProgress]);

  const toggle = useCallback(() => setOpen((v) => !v), []);

  return (
    <div className="mb-0.5">
      {/* Step header */}
      <button
        type="button"
        onClick={hasItems ? toggle : undefined}
        className={cn(
          "w-full flex items-center gap-2 px-3 py-2 text-[11px] rounded transition-colors text-left",
          isInProgress && "bg-blue-500/8",
          isCompleted && "opacity-75",
          isFailed && "opacity-50",
          hasItems && "hover:bg-accent/30 cursor-pointer",
          !hasItems && "cursor-default",
        )}
      >
        {/* Status icon */}
        {isCompleted && (
          <CheckCircle2 className="w-3.5 h-3.5 text-emerald-400 flex-shrink-0" />
        )}
        {isInProgress && (
          <Loader2 className="w-3.5 h-3.5 text-blue-400 animate-spin flex-shrink-0" />
        )}
        {isFailed && (
          <XCircle className="w-3.5 h-3.5 text-red-400/70 flex-shrink-0" />
        )}
        {isPending && (
          <Circle className="w-3.5 h-3.5 text-muted-foreground/50 flex-shrink-0" />
        )}

        {/* Step label */}
        <span
          className={cn(
            "flex-1 font-medium",
            isCompleted && "text-foreground/70",
            isInProgress && "text-blue-300",
            isPending && "text-muted-foreground/50",
            isFailed && "text-red-400/60 line-through",
          )}
        >
          {step.step}
        </span>

        {/* Tool count + chevron */}
        {hasItems && (
          <span className="flex items-center gap-1 flex-shrink-0 text-muted-foreground/40">
            <span className="text-[10px]">{items.length}</span>
            {open ? (
              <ChevronDown className="w-3 h-3" />
            ) : (
              <ChevronRight className="w-3 h-3" />
            )}
          </span>
        )}
      </button>

      {/* Nested items */}
      {hasItems && open && (
        <div className="ml-5 pl-3 border-l-2 border-[var(--border-subtle)] space-y-0.5 pb-1">
          {items.map((item) =>
            item.kind === "tool" ? (
              <ToolExecutionCard key={item.data.requestId} execution={item.data} compact />
            ) : item.kind === "pipeline" ? (
              <PipelineProgressBlock key={item.id} execution={item.data} />
            ) : (
              <SubAgentCard key={item.data.parentRequestId} subAgent={item.data} />
            ),
          )}
        </div>
      )}
    </div>
  );
});

interface ToolDetailViewProps {
  sessionId: string;
}

export const ToolDetailView = memo(function ToolDetailView({
  sessionId,
}: ToolDetailViewProps) {
  const setDetailViewMode = useStore((s) => s.setDetailViewMode);
  const timeline = useStore((s) => s.timelines[sessionId] ?? []);
  const plan = useStore((s) => s.sessions[sessionId]?.plan ?? null);
  const filterIds = useStore(
    (s) => s.sessions[sessionId]?.toolDetailRequestIds ?? null,
  );

  const allItems = useMemo(() => {
    const result: DetailItem[] = [];
    const idSet = filterIds ? new Set(filterIds) : null;
    for (const block of timeline) {
      if (block.type === "ai_tool_execution") {
        if (block.data.toolName === "update_plan" || block.data.toolName === "run_pipeline") continue;
        if (!idSet || idSet.has(block.data.requestId)) {
          result.push({ kind: "tool", data: block.data });
        }
      } else if (block.type === "sub_agent_activity") {
        result.push({ kind: "sub_agent", data: block.data });
      } else if (block.type === "pipeline_progress") {
        result.push({ kind: "pipeline", data: block.data, id: block.id });
      }
    }
    return result;
  }, [timeline, filterIds]);

  const planStepIndexMap = useMemo(() => {
    const map = new Map<string, number>();
    for (const block of timeline) {
      if (block.type === "ai_tool_execution" && block.data.planStepIndex != null) {
        map.set(block.data.requestId, block.data.planStepIndex);
      } else if (block.type === "pipeline_progress" && block.planStepIndex != null) {
        map.set(block.id, block.planStepIndex);
      } else if (block.type === "sub_agent_activity" && block.planStepIndex != null) {
        map.set(block.data.parentRequestId, block.planStepIndex);
      }
    }
    return map;
  }, [timeline]);

  const { stepItems, ungrouped } = useMemo(() => {
    if (!plan) return { stepItems: new Map<number, DetailItem[]>(), ungrouped: allItems };

    const grouped = new Map<number, DetailItem[]>();
    const rest: DetailItem[] = [];
    for (const item of allItems) {
      let idx: number | undefined;
      if (item.kind === "tool") {
        idx = item.data.planStepIndex ?? planStepIndexMap.get(item.data.requestId);
      } else if (item.kind === "pipeline") {
        idx = planStepIndexMap.get(item.id);
      } else if (item.kind === "sub_agent") {
        idx = planStepIndexMap.get(item.data.parentRequestId);
      }
      if (idx != null && idx < plan.steps.length) {
        let list = grouped.get(idx);
        if (!list) {
          list = [];
          grouped.set(idx, list);
        }
        list.push(item);
      } else {
        rest.push(item);
      }
    }
    return { stepItems: grouped, ungrouped: rest };
  }, [plan, allItems, planStepIndexMap]);

  const runningCount = allItems.filter((item) => {
    if (item.kind === "pipeline") return item.data.status === "running";
    if (item.kind === "tool") return item.data.status === "running";
    return item.data.status === "running";
  }).length;

  const totalCount = plan ? plan.summary.total : allItems.length;
  const doneCount = plan
    ? plan.summary.completed
    : allItems.filter((item) => {
        const s = item.data.status as string;
        return s === "completed" || s === "error" || s === "interrupted" || s === "failed";
      }).length;

  const hasContent = allItems.length > 0 || plan;

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
          {doneCount}/{totalCount} done
        </span>
      </div>

      {/* Execution list */}
      <div className="flex-1 overflow-y-auto px-3 py-2">
        {!hasContent ? (
          <div className="h-full flex items-center justify-center text-muted-foreground/50 text-sm">
            No tool executions yet
          </div>
        ) : (
          <div className="space-y-0.5">
            {/* Integrated plan + grouped tools */}
            {plan &&
              plan.steps.map((step, i) => (
                <PlanStepGroup
                  key={`step-${i}-${step.step}`}
                  step={step}
                  items={stepItems.get(i) ?? []}
                />
              ))}

            {/* Ungrouped items (no planStepIndex or no plan) */}
            {ungrouped.length > 0 && (
              <div className={cn(plan && "mt-2 pt-2 border-t border-[var(--border-subtle)]")}>
                {ungrouped.map((item) =>
                  item.kind === "tool" ? (
                    <ToolExecutionCard key={item.data.requestId} execution={item.data} />
                  ) : item.kind === "pipeline" ? (
                    <PipelineProgressBlock key={item.id} execution={item.data} />
                  ) : (
                    <SubAgentCard key={item.data.parentRequestId} subAgent={item.data} />
                  ),
                )}
              </div>
            )}
          </div>
        )}
      </div>

      {/* Footer — running count */}
      {runningCount > 0 && (
        <div className="px-4 py-2 border-t border-[var(--border-subtle)] bg-blue-500/5">
          <div className="flex items-center gap-2">
            <Loader2 className="w-3 h-3 text-blue-400 animate-spin" />
            <span className="text-[11px] text-blue-400/80">
              {runningCount} running...
            </span>
          </div>
        </div>
      )}
    </div>
  );
});
