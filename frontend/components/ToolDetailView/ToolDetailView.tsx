import { ArrowLeft, CheckCircle2, Circle, List, Loader2 } from "lucide-react";
import { memo, useMemo } from "react";
import { PipelineProgressBlock } from "@/components/PipelineProgressBlock";
import { SubAgentCard } from "@/components/SubAgentCard";
import { TaskGroupShell } from "@/components/TaskGroupShell";
import { ToolExecutionCard } from "@/components/ToolExecutionCard";
import { cn } from "@/lib/utils";
import type { ActiveSubAgent, AiToolExecution, PipelineExecution } from "@/store";
import type { TaskPlan } from "@/store/store-types";
import { useStore } from "@/store";

type DetailItem =
  | { kind: "tool"; data: AiToolExecution }
  | { kind: "sub_agent"; data: ActiveSubAgent }
  | { kind: "pipeline"; data: PipelineExecution; id: string };

function PlanBlock({ plan }: { plan: TaskPlan }) {
  const inProgress = plan.steps.filter((s) => s.status === "in_progress").length;

  return (
    <TaskGroupShell
      title="Task Plan"
      titleExtra={
        <span className="flex items-center gap-1 text-muted-foreground/40">
          <List className="w-3 h-3" />
          <span className="text-[10px]">v{plan.version}</span>
        </span>
      }
      running={inProgress}
      completed={plan.summary.completed}
      failed={0}
      total={plan.summary.total}
      totalDurationMs={0}
    >
      <div className="divide-y divide-border/10">
        {plan.steps.map((step, i) => (
          <div
            key={`${step.step}-${i}`}
            className={cn(
              "flex items-center gap-2 px-3 py-1.5 text-[11px]",
              step.status === "in_progress" && "bg-blue-500/5",
            )}
          >
            {step.status === "completed" && (
              <CheckCircle2 className="w-3.5 h-3.5 text-emerald-400 flex-shrink-0" />
            )}
            {step.status === "in_progress" && (
              <Loader2 className="w-3.5 h-3.5 text-blue-400 animate-spin flex-shrink-0" />
            )}
            {step.status === "pending" && (
              <Circle className="w-3.5 h-3.5 text-muted-foreground/50 flex-shrink-0" />
            )}
            <span
              className={cn(
                "font-medium",
                step.status === "completed" && "text-foreground/70",
                step.status === "in_progress" && "text-blue-300",
                step.status === "pending" && "text-muted-foreground/50",
              )}
            >
              {step.step}
            </span>
          </div>
        ))}
      </div>
    </TaskGroupShell>
  );
}

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
    (s) => s.sessions[sessionId]?.toolDetailRequestIds ?? null
  );

  const items = useMemo(() => {
    const result: DetailItem[] = [];
    const idSet = filterIds ? new Set(filterIds) : null;
    for (const block of timeline) {
      if (block.type === "ai_tool_execution") {
        if (block.data.toolName === "update_plan") continue;
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

  const runningCount = items.filter((item) => {
    if (item.kind === "pipeline") return item.data.status === "running";
    if (item.kind === "tool") return item.data.status === "running";
    return item.data.status === "running";
  }).length;

  const totalCount = items.length + (plan ? 1 : 0);
  const doneCount = items.filter((item) => {
    const s = item.data.status as string;
    if (item.kind === "pipeline") return s === "completed" || s === "failed" || s === "interrupted";
    if (item.kind === "tool") return s === "completed" || s === "error" || s === "interrupted";
    return s === "completed" || s === "error" || s === "interrupted";
  }).length + (plan && plan.summary.completed === plan.summary.total ? 1 : 0);

  const hasContent = items.length > 0 || plan;

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
          <div className="space-y-1">
            {plan && <PlanBlock plan={plan} />}
            {items.map((item) =>
              item.kind === "tool" ? (
                <ToolExecutionCard key={item.data.requestId} execution={item.data} />
              ) : item.kind === "pipeline" ? (
                <PipelineProgressBlock key={item.id} execution={item.data} />
              ) : (
                <SubAgentCard key={item.data.parentRequestId} subAgent={item.data} />
              )
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
