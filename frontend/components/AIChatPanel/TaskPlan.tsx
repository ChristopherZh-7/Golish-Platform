import {
  CheckCircle2,
  ChevronDown,
  ChevronUp,
  Eye,
  List,
  Loader2,
  Wrench,
  XCircle,
} from "lucide-react";
import { useMemo, useState } from "react";
import { formatDurationShort, formatRelativeTime } from "@/lib/time";
import { getToolColor, getToolLabel } from "@/lib/tools";
import { cn } from "@/lib/utils";
import { useStore } from "@/store";
import { parseToolPrimary } from "./ToolCallSummary";

export interface TaskPlanState {
  version: number;
  steps: Array<{ step: string; status: "pending" | "in_progress" | "completed" | "cancelled" | "failed" }>;
  summary: { total: number; completed: number; in_progress: number; pending: number };
  retiredAt?: string;
}

interface NestedToolInfo {
  requestId: string;
  toolName: string;
  status: "running" | "completed" | "error" | "interrupted";
  durationMs?: number;
  primary?: string | null;
}

export function PlanStepIcon({ status, size = "sm" }: { status: string; size?: "sm" | "md" }) {
  const s = size === "sm" ? "w-2.5 h-2.5" : "w-3.5 h-3.5";
  switch (status) {
    case "completed": return <CheckCircle2 className={cn(s, "text-green-500/70 flex-shrink-0")} />;
    case "in_progress": return <Loader2 className={cn(s, "text-accent animate-spin flex-shrink-0")} />;
    case "failed":
    case "cancelled": return <XCircle className={cn(s, "text-red-400/60 flex-shrink-0")} />;
    default: return <div className={cn(s, "rounded-full border border-muted-foreground/30 flex-shrink-0")} />;
  }
}

function StepToolItem({ tool }: { tool: NestedToolInfo }) {
  const label = getToolLabel(tool.toolName, "short");
  const color = getToolColor(tool.toolName);
  return (
    <div className="flex items-center gap-1.5 py-0.5 text-[10px] text-muted-foreground/70">
      <Wrench className="w-2.5 h-2.5 flex-shrink-0 opacity-60" style={{ color }} />
      <span className="truncate">
        {label}
        {tool.primary && <span className="ml-1 opacity-50 truncate">{tool.primary}</span>}
      </span>
      <span className="ml-auto flex-shrink-0 flex items-center gap-1">
        {tool.status === "running" && <Loader2 className="w-2.5 h-2.5 animate-spin text-[#7aa2f7]" />}
        {tool.status === "completed" && (
          <>
            <CheckCircle2 className="w-2.5 h-2.5 text-green-500" />
            {tool.durationMs != null && formatDurationShort(tool.durationMs) && (
              <span className="text-[9px] opacity-50">{formatDurationShort(tool.durationMs)}</span>
            )}
          </>
        )}
        {(tool.status === "error" || tool.status === "interrupted") && <XCircle className="w-2.5 h-2.5 text-red-400" />}
      </span>
    </div>
  );
}

function useStepTools(terminalId: string | null): Map<number, NestedToolInfo[]> {
  const raw = useStore((s) => {
    if (!terminalId) return "{}";
    const timeline = s.timelines[terminalId];
    if (!timeline) return "{}";
    const obj: Record<number, NestedToolInfo[]> = {};
    for (const block of timeline) {
      if (block.type !== "ai_tool_execution") continue;
      const exec = block.data;
      if (exec.planStepIndex == null) continue;
      if (!obj[exec.planStepIndex]) obj[exec.planStepIndex] = [];
      const primary = parseToolPrimary(exec.toolName, exec.args ? JSON.stringify(exec.args) : undefined);
      obj[exec.planStepIndex].push({
        requestId: exec.requestId,
        toolName: exec.toolName,
        status: exec.status,
        durationMs: exec.durationMs,
        primary,
      });
    }
    return JSON.stringify(obj);
  });
  return useMemo(() => {
    const obj = JSON.parse(raw) as Record<string, NestedToolInfo[]>;
    const map = new Map<number, NestedToolInfo[]>();
    for (const [k, v] of Object.entries(obj)) map.set(Number(k), v);
    return map;
  }, [raw]);
}

/** Set of requestIds that are nested inside the plan card (so they can be hidden from message stream) */
export function usePlanNestedRequestIds(terminalId: string | null): Set<string> {
  const raw = useStore((s) => {
    if (!terminalId) return "";
    const timeline = s.timelines[terminalId];
    if (!timeline) return "";
    const arr: string[] = [];
    for (const block of timeline) {
      if (block.type === "ai_tool_execution" && block.data.planStepIndex != null) {
        arr.push(block.data.requestId);
      }
    }
    return arr.join("\n");
  });
  return useMemo(() => new Set(raw ? raw.split("\n") : []), [raw]);
}

export function TaskPlanCard({ plan, terminalId, retired }: { plan: TaskPlanState; terminalId?: string | null; retired?: boolean }) {
  const [expanded, setExpanded] = useState(!retired);
  const [collapsedSteps, setCollapsedSteps] = useState<Set<number>>(new Set());
  const stepTools = useStepTools(terminalId ?? null);
  const progress = plan.summary.total > 0 ? (plan.summary.completed / plan.summary.total) * 100 : 0;
  const isAllDone = plan.summary.total > 0 && plan.summary.completed === plan.summary.total;
  const relativeTime = retired ? formatRelativeTime(plan.retiredAt) : null;

  const handleShowDetail = () => {
    const state = useStore.getState();
    const sid = state.activeSessionId;
    if (sid) {
      state.setDetailViewMode(sid, "tool-detail");
    }
  };

  const toggleStep = (idx: number) => {
    setCollapsedSteps((prev) => {
      const next = new Set(prev);
      if (next.has(idx)) next.delete(idx);
      else next.add(idx);
      return next;
    });
  };

  return (
    <div className={cn(
      "mx-4 my-2 rounded-lg border p-3 w-[calc(100%-2rem)] text-left transition-colors group",
      retired
        ? "border-border/20 bg-muted/5 opacity-80"
        : "border-border/30 bg-background/50 hover:border-accent/40 hover:bg-accent/5"
    )}>
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        className="w-full flex items-center gap-2 cursor-pointer"
      >
        <List className={cn("w-3.5 h-3.5 flex-shrink-0", retired ? "text-muted-foreground/60" : "text-accent")} />
        <span className={cn("text-[12px] font-medium", retired ? "text-muted-foreground" : "text-foreground")}>
          Task Plan
        </span>
        {retired && (
          <span className="text-[9px] px-1.5 py-0.5 rounded bg-muted/20 text-muted-foreground/60 font-medium">
            Previous
          </span>
        )}
        {!retired && isAllDone && (
          <span className="text-[9px] px-1.5 py-0.5 rounded bg-green-500/10 text-green-500/70 font-medium">
            Done
          </span>
        )}
        {relativeTime && (
          <span className="text-[9px] text-muted-foreground/50 ml-auto">
            {relativeTime}
          </span>
        )}
        {!retired && (
          <span
            role="link"
            tabIndex={0}
            className="ml-auto text-[10px] text-muted-foreground/70 hover:text-accent/70 transition-colors cursor-pointer"
            onClick={(e) => { e.stopPropagation(); handleShowDetail(); }}
            onKeyDown={(e) => { if (e.key === "Enter") { e.stopPropagation(); handleShowDetail(); } }}
          >
            View details &rarr;
          </span>
        )}
        {expanded ? (
          <ChevronUp className="w-3 h-3 text-muted-foreground/70 group-hover:text-accent/70 transition-colors flex-shrink-0" />
        ) : (
          <ChevronDown className="w-3 h-3 text-muted-foreground/70 group-hover:text-accent/70 transition-colors flex-shrink-0" />
        )}
      </button>

      <div className="mt-2 h-1 rounded-full bg-muted/50 overflow-hidden">
        <div
          className={cn(
            "h-full rounded-full transition-all duration-300",
            retired
              ? "bg-muted-foreground/30"
              : "bg-accent",
          )}
          style={{ width: `${progress}%` }}
        />
      </div>

      {!expanded && retired && (
        <div className="mt-1.5 text-[10px] text-muted-foreground/50">
          {plan.summary.completed}/{plan.summary.total} steps completed
        </div>
      )}

      {expanded && (
        <div className="mt-2 space-y-0.5">
          {plan.steps.map((step, i) => {
            const tools = stepTools.get(i) ?? [];
            const hasTools = tools.length > 0;
            const isActive = step.status === "in_progress";
            const isDone = step.status === "completed";
            const isStepCollapsed = isDone ? !collapsedSteps.has(i) : collapsedSteps.has(i);

            return (
              <div key={`${step.step}-${i}`}>
                <button
                  type="button"
                  onClick={hasTools ? () => toggleStep(i) : undefined}
                  className={cn(
                    "w-full flex items-center gap-1.5 text-[11px] py-0.5",
                    hasTools && "cursor-pointer hover:text-foreground/90",
                  )}
                >
                  <PlanStepIcon status={step.status} />
                  <span
                    className={cn(
                      "text-left truncate",
                      isDone ? "text-muted-foreground/60"
                        : isActive ? "text-accent"
                        : (step.status === "cancelled" || step.status === "failed") ? "text-red-400/60"
                        : "text-muted-foreground",
                    )}
                    title={step.step}
                  >
                    {step.step.length > 100 ? `${step.step.slice(0, 100)}…` : step.step}
                  </span>
                  {hasTools && (
                    <span className="ml-auto flex items-center gap-0.5 text-[9px] text-muted-foreground/60">
                      {tools.length}
                      <ChevronDown className={cn("w-2 h-2 transition-transform", isStepCollapsed && "-rotate-90")} />
                    </span>
                  )}
                </button>
                {hasTools && !isStepCollapsed && (
                  <div className="ml-4 pl-2 border-l border-border/20 space-y-0">
                    {tools.map((t) => <StepToolItem key={t.requestId} tool={t} />)}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

export function PlanUpdatedNotice() {
  return (
    <div className="flex items-center gap-1.5 px-3 py-1 text-[10px] text-muted-foreground/70">
      <List className="w-2.5 h-2.5" />
      <span>Plan updated</span>
    </div>
  );
}

export function StickyPlanProgress({ plan }: { plan: TaskPlanState }) {
  const [expanded, setExpanded] = useState(false);
  const current = plan.steps.find((s) => s.status === "in_progress");
  const currentIdx = current ? plan.steps.indexOf(current) : -1;
  const progress = plan.summary.total > 0 ? (plan.summary.completed / plan.summary.total) * 100 : 0;
  const isAllDone = plan.summary.total > 0 && plan.summary.completed === plan.summary.total;

  const handleViewDetail = () => {
    const state = useStore.getState();
    const sid = state.activeSessionId;
    if (sid) {
      state.setDetailViewMode(sid, "tool-detail");
    }
  };

  return (
    <div className="sticky top-0 z-20 mx-0 border-b border-border/30 bg-[var(--background)]/90 backdrop-blur-sm">
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-muted/20 transition-colors"
      >
        <List className="w-3 h-3 text-accent flex-shrink-0" />
        <span className="text-[11px] font-medium text-foreground/80 truncate flex-1 text-left">
          {isAllDone
            ? "All steps complete"
            : current
              ? `Step ${currentIdx + 1}/${plan.summary.total}: ${current.step.length > 80 ? `${current.step.slice(0, 80)}…` : current.step}`
              : `${plan.summary.completed}/${plan.summary.total} steps`}
        </span>
        {plan.summary.total > 0 && (
          <span className="text-[9px] px-1 py-0.5 rounded bg-accent/10 text-accent/60 flex-shrink-0">
            {plan.summary.completed}/{plan.summary.total}
          </span>
        )}
        <span className="text-[10px] text-muted-foreground/60 flex-shrink-0">
          {Math.round(progress)}%
        </span>
        <ChevronDown className={cn("w-3 h-3 text-muted-foreground/50 flex-shrink-0 transition-transform", expanded && "rotate-180")} />
      </button>

      {expanded && (
        <div className="px-3 pb-2 space-y-0.5">
          {plan.steps.map((step, idx) => {
            const isActive = step.status === "in_progress";
            const isDone = step.status === "completed";
            const isFailed = step.status === "failed" || step.status === "cancelled";
            return (
              <div key={idx} className={cn(
                "flex items-center gap-2 py-1 px-2 rounded text-[11px]",
                isActive && "bg-accent/8",
              )}>
                <PlanStepIcon status={step.status} />
                <span className={cn(
                  "flex-1 truncate",
                  isDone && "text-muted-foreground/60",
                  isActive && "text-accent font-medium",
                  isFailed && "text-red-400/60",
                  !isDone && !isActive && !isFailed && "text-muted-foreground/70",
                )}>
                  {idx + 1}. {step.step}
                </span>
              </div>
            );
          })}
          
          <button
            type="button"
            onClick={(e) => { e.stopPropagation(); handleViewDetail(); }}
            className="flex items-center gap-1.5 mt-1.5 px-2 py-1 rounded text-[10px] text-accent/70 hover:text-accent hover:bg-accent/10 transition-colors"
          >
            <Eye className="w-3 h-3" />
            <span>View Detail</span>
          </button>
        </div>
      )}

      <div className="h-[2px] bg-muted/30">
        <div
          className="h-full bg-accent transition-all duration-500 ease-out"
          style={{ width: `${progress}%` }}
        />
      </div>
    </div>
  );
}
