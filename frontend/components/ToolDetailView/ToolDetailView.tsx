import {
  ArrowLeft,
  ChevronDown,
  ChevronRight,
  History,
  Loader2,
} from "lucide-react";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { PlanStepIcon } from "@/components/AIChatPanel/ChatSubComponents";
import { PipelineProgressBlock } from "@/components/PipelineProgressBlock";
import { SubAgentCard } from "@/components/SubAgentCard";
import { ToolExecutionCard } from "@/components/ToolExecutionCard";
import { cn } from "@/lib/utils";
import type { ActiveSubAgent, AiToolExecution, PipelineExecution } from "@/store";
import type { PlanStep, RetiredPlan } from "@/store/store-types";
import { useStore } from "@/store";

type DetailItem =
  | { kind: "tool"; data: AiToolExecution }
  | { kind: "sub_agent"; data: ActiveSubAgent }
  | { kind: "pipeline"; data: PipelineExecution; id: string };

const EMPTY_RETIRED_PLANS: RetiredPlan[] = [];
const EMPTY_TIMELINE: never[] = [];

/** A single plan step row with its nested tool/pipeline/subagent cards. */
const PlanStepGroup = memo(function PlanStepGroup({
  step,
  items,
  highlightSet,
  expandedCardId,
  onExpandCard,
  versionLabel,
}: {
  step: PlanStep;
  items: DetailItem[];
  highlightSet?: Set<string> | null;
  expandedCardId?: string | null;
  onExpandCard?: (id: string | null) => void;
  versionLabel?: string;
}) {
  const isCompleted = step.status === "completed";
  const isInProgress = step.status === "in_progress";
  const isFailed = step.status === "cancelled" || step.status === "failed";
  const isPending = step.status === "pending";
  const hasItems = items.length > 0;

  const hasHighlight = items.some((item) => {
    if (!highlightSet) return false;
    if (item.kind === "tool") return highlightSet.has(item.data.requestId);
    if (item.kind === "sub_agent") return highlightSet.has(item.data.parentRequestId);
    if (item.kind === "pipeline") return highlightSet.has(item.id);
    return false;
  });
  const [open, setOpen] = useState(isInProgress || (!isCompleted && hasItems) || hasHighlight);
  const prevStatusRef = useRef(step.status);

  useEffect(() => {
    if (isInProgress || hasHighlight) {
      setOpen(true);
    }
  }, [isInProgress, hasHighlight]);

  useEffect(() => {
    const prev = prevStatusRef.current;
    prevStatusRef.current = step.status;
    if (prev === "in_progress" && step.status !== "in_progress") {
      setOpen(false);
    }
  }, [step.status]);

  const toggle = useCallback(() => setOpen((v) => !v), []);

  return (
    <div className="mb-0.5 transition-opacity duration-200">
      {/* Step header */}
      <button
        type="button"
        onClick={hasItems ? toggle : undefined}
        className={cn(
          "w-full flex items-center gap-2 px-3 py-2 text-[11px] rounded transition-all duration-200 text-left",
          isInProgress && "bg-accent/8",
          isCompleted && "opacity-75",
          isFailed && "opacity-50",
          hasItems && "hover:bg-accent/30 cursor-pointer",
          !hasItems && "cursor-default",
        )}
      >
        <PlanStepIcon status={step.status} size="md" />

        <span
          className={cn(
            "flex-1 font-medium truncate",
            isCompleted && "text-muted-foreground/60",
            isInProgress && "text-accent",
            isPending && "text-muted-foreground/50",
            isFailed && "text-red-400/60",
          )}
          title={step.step}
        >
          {step.step.length > 120 ? `${step.step.slice(0, 120)}…` : step.step}
        </span>

        {versionLabel && (
          <span className="text-[9px] px-1 py-0.5 rounded bg-muted/50 text-muted-foreground/50 flex-shrink-0">
            {versionLabel}
          </span>
        )}

        {/* Tool count + chevron */}
        {hasItems && (
          <span className="flex items-center gap-1 flex-shrink-0 text-muted-foreground/60">
            <span className="text-[10px]">{items.length}</span>
            {open ? (
              <ChevronDown className="w-3 h-3" />
            ) : (
              <ChevronRight className="w-3 h-3" />
            )}
          </span>
        )}
      </button>

      {/* Nested items — animated with CSS grid for smooth height transition */}
      {hasItems && (
        <div
          className="grid transition-[grid-template-rows] duration-200 ease-out"
          style={{ gridTemplateRows: open ? "1fr" : "0fr" }}
        >
          <div className="overflow-hidden">
            <div className="ml-5 pl-3 border-l-2 border-[var(--border-subtle)] space-y-0.5 pb-1">
              {items.map((item) =>
                item.kind === "tool" ? (
                  <ToolExecutionCard
                    key={item.data.requestId}
                    execution={item.data}
                    compact
                    highlighted={!!highlightSet?.has(item.data.requestId)}
                    isOpen={expandedCardId === item.data.requestId}
                    onToggle={() => onExpandCard?.(expandedCardId === item.data.requestId ? null : item.data.requestId)}
                  />
                ) : item.kind === "pipeline" ? (
                  <PipelineProgressBlock key={item.id} execution={item.data} />
                ) : (
                  <SubAgentCard
                    key={item.data.parentRequestId}
                    subAgent={item.data}
                    highlighted={!!highlightSet?.has(item.data.parentRequestId)}
                  />
                ),
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
});

/** Collapsible section showing steps from previous plan versions that have tool executions. */
const RetiredPlanSection = memo(function RetiredPlanSection({
  retiredPlans,
  retiredStepItems,
  highlightSet,
  expandedCardId,
  onExpandCard,
}: {
  retiredPlans: RetiredPlan[];
  retiredStepItems: Map<string, DetailItem[]>;
  highlightSet?: Set<string> | null;
  expandedCardId?: string | null;
  onExpandCard?: (id: string | null) => void;
}) {
  const [open, setOpen] = useState(false);
  const toggle = useCallback(() => setOpen((v) => !v), []);

  // Collect all retired steps (with or without tool executions)
  const stepsWithItems = useMemo(() => {
    const result: { step: PlanStep; items: DetailItem[]; retiredAt: string }[] = [];
    for (const rp of retiredPlans) {
      for (const step of rp.plan.steps) {
        const items = (step.id && retiredStepItems.get(step.id)) || [];
        result.push({ step, items, retiredAt: rp.retiredAt });
      }
    }
    return result;
  }, [retiredPlans, retiredStepItems]);

  if (stepsWithItems.length === 0) return null;

  const totalItems = stepsWithItems.reduce((sum, s) => sum + s.items.length, 0);

  return (
    <div className="mt-2 pt-2 border-t border-[var(--border-subtle)] animate-in fade-in slide-in-from-top-1 duration-200">
      <button
        type="button"
        onClick={toggle}
        className="w-full flex items-center gap-2 px-3 py-1.5 text-[11px] text-muted-foreground/70 hover:text-muted-foreground hover:bg-accent/20 rounded transition-colors"
      >
        <History className="w-3.5 h-3.5" />
        <span className="font-medium">Previous Plan Steps</span>
        <span className="text-[10px] opacity-60">{stepsWithItems.length} steps{totalItems > 0 ? `, ${totalItems} executions` : ""}</span>
        <span className="ml-auto">
          {open ? <ChevronDown className="w-3 h-3" /> : <ChevronRight className="w-3 h-3" />}
        </span>
      </button>
      <div
        className="grid transition-[grid-template-rows] duration-200 ease-out"
        style={{ gridTemplateRows: open ? "1fr" : "0fr" }}
      >
        <div className="overflow-hidden">
          <div className="mt-1 opacity-80">
            {stepsWithItems.map(({ step, items }) => (
              <PlanStepGroup
                key={step.id}
                step={step}
                items={items}
                highlightSet={highlightSet}
                expandedCardId={expandedCardId}
                onExpandCard={onExpandCard}
                versionLabel="Previous"
              />
            ))}
          </div>
        </div>
      </div>
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
  const timeline = useStore((s) => s.timelines[sessionId] ?? EMPTY_TIMELINE);
  const plan = useStore((s) => s.sessions[sessionId]?.plan ?? null);
  const retiredPlans = useStore((s) => s.sessions[sessionId]?.retiredPlans ?? EMPTY_RETIRED_PLANS);
  const highlightIds = useStore(
    (s) => s.sessions[sessionId]?.toolDetailRequestIds ?? null,
  );
  const highlightSet = useMemo(() => highlightIds ? new Set(highlightIds) : null, [highlightIds]);

  // Accordion state: only one ToolExecutionCard open at a time
  const [expandedCardId, setExpandedCardId] = useState<string | null>(
    highlightIds?.[0] ?? null,
  );
  // Auto-expand highlighted card when it changes
  useEffect(() => {
    if (highlightIds?.[0]) setExpandedCardId(highlightIds[0]);
  }, [highlightIds]);

  // Stick-to-bottom auto-scroll: follows streaming output, not just new items
  const scrollContainerRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const isStuckToBottomRef = useRef(true);
  const isProgrammaticScrollRef = useRef(false);

  const allItems = useMemo(() => {
    const result: DetailItem[] = [];
    for (const block of timeline) {
      if (block.type === "ai_tool_execution") {
        if (block.data.toolName === "update_plan" || block.data.toolName === "run_pipeline") continue;
        result.push({ kind: "tool", data: block.data });
      } else if (block.type === "sub_agent_activity") {
        result.push({ kind: "sub_agent", data: block.data });
      } else if (block.type === "pipeline_progress") {
        result.push({ kind: "pipeline", data: block.data, id: block.id });
      }
    }
    return result;
  }, [timeline]);

  const planStepLookup = useMemo(() => {
    const idMap = new Map<string, string>();
    const indexMap = new Map<string, number>();
    for (const block of timeline) {
      if (block.type === "ai_tool_execution") {
        if (block.data.planStepId) idMap.set(block.data.requestId, block.data.planStepId);
        if (block.data.planStepIndex != null) indexMap.set(block.data.requestId, block.data.planStepIndex);
      } else if (block.type === "pipeline_progress" && block.planStepIndex != null) {
        indexMap.set(block.id, block.planStepIndex);
      } else if (block.type === "sub_agent_activity" && block.planStepIndex != null) {
        indexMap.set(block.data.parentRequestId, block.planStepIndex);
      }
    }
    return { idMap, indexMap };
  }, [timeline]);

  // Build a set of all known step IDs from the current plan
  const currentStepIds = useMemo(() => {
    if (!plan) return new Set<string>();
    return new Set(plan.steps.map((s) => s.id).filter(Boolean) as string[]);
  }, [plan]);

  const { stepItems, retiredStepItems, ungrouped } = useMemo(() => {
    if (!plan) return { stepItems: new Map<string, DetailItem[]>(), retiredStepItems: new Map<string, DetailItem[]>(), ungrouped: allItems };

    // Use step IDs as keys when available, fall back to index-based string keys
    const grouped = new Map<string, DetailItem[]>();
    const retiredGrouped = new Map<string, DetailItem[]>();
    const rest: DetailItem[] = [];

    // Collect all step IDs from retired plans
    const retiredStepIdSet = new Set<string>();
    for (const rp of retiredPlans) {
      for (const s of rp.plan.steps) {
        if (s.id && !currentStepIds.has(s.id)) retiredStepIdSet.add(s.id);
      }
    }

    for (const item of allItems) {
      let stepId: string | undefined;
      let idx: number | undefined;

      if (item.kind === "tool") {
        stepId = item.data.planStepId ?? planStepLookup.idMap.get(item.data.requestId);
        idx = item.data.planStepIndex ?? planStepLookup.indexMap.get(item.data.requestId);
      } else if (item.kind === "pipeline") {
        idx = planStepLookup.indexMap.get(item.id);
      } else if (item.kind === "sub_agent") {
        idx = planStepLookup.indexMap.get(item.data.parentRequestId);
      }

      // Prefer grouping by step ID
      if (stepId && currentStepIds.has(stepId)) {
        let list = grouped.get(stepId);
        if (!list) { list = []; grouped.set(stepId, list); }
        list.push(item);
      } else if (stepId && retiredStepIdSet.has(stepId)) {
        let list = retiredGrouped.get(stepId);
        if (!list) { list = []; retiredGrouped.set(stepId, list); }
        list.push(item);
      } else if (idx != null && idx < plan.steps.length && !stepId) {
        // Legacy fallback: use index-based grouping for old data without step IDs
        const fallbackKey = plan.steps[idx].id ?? `idx-${idx}`;
        let list = grouped.get(fallbackKey);
        if (!list) { list = []; grouped.set(fallbackKey, list); }
        list.push(item);
      } else {
        rest.push(item);
      }
    }
    return { stepItems: grouped, retiredStepItems: retiredGrouped, ungrouped: rest };
  }, [plan, allItems, planStepLookup, currentStepIds, retiredPlans]);

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

  const debugPlanVersionRef = useRef(plan?.version ?? 1);
  useEffect(() => {
    if (plan) debugPlanVersionRef.current = plan.version;
  }, [plan?.version]);
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.shiftKey && e.key === 'U') {
        e.preventDefault();
        const store = useStore.getState();
        const currentPlan = store.sessions[sessionId]?.plan;
        if (!currentPlan) return;
        const nextV = debugPlanVersionRef.current + 1;
        debugPlanVersionRef.current = nextV;
        const newSteps = currentPlan.steps.map((s, i) => {
          if (s.status === 'in_progress') return { ...s, status: 'completed' as const };
          if (s.status === 'pending' && currentPlan.steps[i - 1]?.status === 'in_progress') return { ...s, status: 'in_progress' as const };
          return { ...s };
        });
        const completed = newSteps.filter(s => s.status === 'completed').length;
        const inProgress = newSteps.filter(s => s.status === 'in_progress').length;
        store.setPlan(sessionId, {
          ...currentPlan,
          version: nextV,
          steps: newSteps,
          summary: { total: newSteps.length, completed, in_progress: inProgress, pending: newSteps.length - completed - inProgress },
          updated_at: new Date().toISOString(),
        });
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [sessionId]);

  const hasContent = allItems.length > 0 || plan;

  // Track user scroll to detect manual scroll-up (unstick from bottom).
  // Programmatic scrolls are ignored so users can freely scroll up.
  const handleScroll = useCallback(() => {
    if (isProgrammaticScrollRef.current) return;
    const el = scrollContainerRef.current;
    if (!el) return;
    const threshold = 60;
    isStuckToBottomRef.current =
      el.scrollHeight - el.scrollTop - el.clientHeight < threshold;
  }, []);

  const scrollToBottom = useCallback(() => {
    const el = scrollContainerRef.current;
    if (!el || !isStuckToBottomRef.current) return;
    isProgrammaticScrollRef.current = true;
    el.scrollTop = el.scrollHeight;
    requestAnimationFrame(() => {
      isProgrammaticScrollRef.current = false;
    });
  }, []);

  // ResizeObserver detects when the content area grows (streaming output,
  // new cards, expanded sections) and keeps the scroll pinned to the bottom.
  // Throttled via rAF to prevent feedback loops between observers and layout.
  useEffect(() => {
    const scrollEl = scrollContainerRef.current;
    const contentEl = contentRef.current;
    if (!scrollEl || !contentEl) return;

    let rafId: number | null = null;

    const throttledScroll = () => {
      if (rafId != null) return;
      rafId = requestAnimationFrame(() => {
        rafId = null;
        scrollToBottom();
      });
    };

    const ro = new ResizeObserver(throttledScroll);
    ro.observe(contentEl);

    const mo = new MutationObserver(throttledScroll);
    mo.observe(contentEl, { childList: true, subtree: true });

    return () => {
      ro.disconnect();
      mo.disconnect();
      if (rafId != null) cancelAnimationFrame(rafId);
    };
  }, [hasContent, scrollToBottom]);

  return (
    <div className="h-full flex flex-col">
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
      <div ref={scrollContainerRef} onScroll={handleScroll} className="flex-1 overflow-y-auto px-3 py-2">
        {!hasContent ? (
          <div className="h-full flex items-center justify-center text-muted-foreground/50 text-sm">
            No tool executions yet
          </div>
        ) : (
          <div ref={contentRef} className="space-y-0.5">
            {/* Ungrouped items (pre-plan tool calls) rendered above the plan */}
            {ungrouped.filter((i) => i.kind !== "sub_agent").length > 0 && (
              <div className={cn(plan && "mb-2 pb-2 border-b border-[var(--border-subtle)]")}>
                {ungrouped.map((item) =>
                  item.kind === "tool" ? (
                    <ToolExecutionCard
                      key={item.data.requestId}
                      execution={item.data}
                      highlighted={!!highlightSet?.has(item.data.requestId)}
                      isOpen={expandedCardId === item.data.requestId}
                      onToggle={() => setExpandedCardId(expandedCardId === item.data.requestId ? null : item.data.requestId)}
                    />
                  ) : item.kind === "pipeline" ? (
                    <PipelineProgressBlock key={item.id} execution={item.data} />
                  ) : null,
                )}
              </div>
            )}

            {/* Integrated plan + grouped tools */}
            {plan &&
              plan.steps.map((step, i) => {
                const key = step.id ?? `idx-${i}`;
                return (
                  <PlanStepGroup
                    key={key}
                    step={step}
                    items={stepItems.get(key) ?? []}
                    highlightSet={highlightSet}
                    expandedCardId={expandedCardId}
                    onExpandCard={setExpandedCardId}
                  />
                );
              })}

            {/* Retired plan steps (with or without associated tool executions) */}
            {retiredPlans.length > 0 && (
              <RetiredPlanSection
                retiredPlans={retiredPlans}
                retiredStepItems={retiredStepItems}
                highlightSet={highlightSet}
                expandedCardId={expandedCardId}
                onExpandCard={setExpandedCardId}
              />
            )}
          </div>
        )}
      </div>

      {/* Footer — running count */}
      {runningCount > 0 && (
        <div className="px-4 py-2 border-t border-[var(--border-subtle)] bg-accent/5">
          <div className="flex items-center gap-2">
            <Loader2 className="w-3 h-3 text-accent animate-spin" />
            <span className="text-[11px] text-accent/80">
              {runningCount} running...
            </span>
          </div>
        </div>
      )}
    </div>
  );
});
