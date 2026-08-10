/**
 * Message-scoped harness plans.
 *
 * Each stage owns the card at the assistant message where that stage began.
 * Future roadmap seeds have no message anchor and therefore never appear here.
 * An active real plan opens by default; once its evidence gate passes, the card
 * collapses in place and remains available for manual review.
 */
import { CheckCircle2, ChevronRight, Loader2 } from "lucide-react";
import { memo, useMemo, useState } from "react";
import { cn } from "@/lib/utils";
import { useStore } from "@/store";
import { StepRow } from "./PlanStepRow";
import { prettyStageName } from "./StageMarker";
import type { StagePlansViewModel, TaskPlanViewModel } from "./TaskPlan";

export interface CurrentStagePlanView {
  idx: number;
  stageId: string;
  plan?: TaskPlanViewModel;
  hasRealPlan: boolean;
  currentStep?: TaskPlanViewModel["steps"][number];
  allPassed: boolean;
  passedCount: number;
}

/** Shared current-stage selection for inline cards and the top status strip. */
export function resolveCurrentStagePlan(
  stagePlans: StagePlansViewModel
): CurrentStagePlanView | null {
  const { stageOrder, plansByStage, passedStages } = stagePlans;
  if (stageOrder.length === 0) return null;

  const passed = new Set(passedStages);
  const passedCount = stageOrder.filter((stageId) => passed.has(stageId)).length;
  const allPassed = passedCount === stageOrder.length;
  let idx = stageOrder.findIndex(
    (stageId) =>
      !passed.has(stageId) &&
      (plansByStage[stageId]?.steps.some((step) => step.status !== "pending") ?? false)
  );
  if (idx < 0) idx = stageOrder.findIndex((stageId) => !passed.has(stageId));
  if (idx < 0) idx = stageOrder.length - 1;

  const stageId = stageOrder[idx];
  const plan = plansByStage[stageId];
  const hasRealPlan = (plan?.version ?? 0) >= 1;
  const currentStep = hasRealPlan
    ? (plan?.steps.find((step) => step.status === "in_progress") ??
      plan?.steps.find((step) => step.status === "pending"))
    : undefined;

  return { idx, stageId, plan, hasRealPlan, currentStep, allPassed, passedCount };
}

const StageTimelinePlanCard = memo(function StageTimelinePlanCard({
  stageId,
  stageIndex,
  stageCount,
  plan,
  passed,
  current,
  isRunning,
}: {
  stageId: string;
  stageIndex: number;
  stageCount: number;
  plan: TaskPlanViewModel;
  passed: boolean;
  current: boolean;
  isRunning: boolean;
}) {
  const hasRealPlan = plan.version >= 1 && plan.steps.length > 0;
  const [activeCollapsed, setActiveCollapsed] = useState(false);
  const [completedExpanded, setCompletedExpanded] = useState(false);
  const expanded = hasRealPlan && (passed ? completedExpanded : !activeCollapsed);
  const currentStep = plan.steps.find((step) => step.status === "in_progress");
  const { completed, total } = plan.summary;

  const toggle = () => {
    if (!hasRealPlan) return;
    if (passed) setCompletedExpanded((open) => !open);
    else setActiveCollapsed((collapsed) => !collapsed);
  };

  const statusIcon = passed ? (
    <CheckCircle2 className="h-4 w-4 flex-shrink-0 text-emerald-500" />
  ) : current && isRunning ? (
    <Loader2 className="h-4 w-4 flex-shrink-0 animate-spin text-accent" />
  ) : (
    <span className="h-4 w-4 flex-shrink-0 rounded-full border-[1.5px] border-accent/55" />
  );

  const collapsedDetail = passed
    ? `Completed · ${completed}/${total} steps`
    : hasRealPlan
      ? (currentStep?.step ?? "Checking stage evidence…")
      : current && isRunning
        ? "Preparing stage plan…"
        : "Stage plan pending";

  return (
    <section
      className={cn(
        "overflow-hidden rounded-lg border bg-background/45",
        passed
          ? "border-emerald-500/15"
          : current
            ? "border-accent/25"
            : "border-[var(--border-subtle)]"
      )}
      data-stage-plan-card={stageId}
    >
      <button
        type="button"
        aria-expanded={hasRealPlan ? expanded : undefined}
        disabled={!hasRealPlan}
        onClick={toggle}
        className={cn(
          "flex w-full min-w-0 items-center gap-2 px-3 py-2 text-left",
          hasRealPlan && "cursor-pointer transition-colors hover:bg-accent/[0.05]"
        )}
      >
        {statusIcon}
        <span className="truncate text-[12px] font-semibold text-foreground">
          {prettyStageName(stageId)}
        </span>
        <span className="flex-shrink-0 text-[10px] tabular-nums text-muted-foreground/55">
          Stage {stageIndex + 1}/{stageCount}
        </span>
        {hasRealPlan && total > 0 && (
          <span className="flex-shrink-0 text-[10px] tabular-nums text-muted-foreground/55">
            · Step {completed}/{total}
          </span>
        )}
        <span className="min-w-0 flex-1 truncate text-[11px] text-muted-foreground/60">
          · {collapsedDetail}
        </span>
        {hasRealPlan && (
          <ChevronRight
            className={cn(
              "h-3.5 w-3.5 flex-shrink-0 text-muted-foreground/45 transition-transform",
              expanded && "rotate-90"
            )}
          />
        )}
      </button>

      {expanded && (
        <div className="border-t border-[var(--border-subtle)]/65 px-2 py-1.5">
          {plan.steps.map((step, index) => (
            <StepRow key={`${index}-${step.step}`} step={step} index={index} />
          ))}
        </div>
      )}
    </section>
  );
});

export const StagePlanStack = memo(function StagePlanStack({
  stagePlans,
  stageIds,
}: {
  stagePlans: StagePlansViewModel;
  stageIds: string[];
}) {
  const { stageOrder, plansByStage, passedStages } = stagePlans;
  const current = useMemo(() => resolveCurrentStagePlan(stagePlans), [stagePlans]);
  const requested = useMemo(() => new Set(stageIds), [stageIds]);
  const anchoredStages = stageOrder.filter(
    (stageId) => requested.has(stageId) && plansByStage[stageId]
  );
  const isRunning = useStore((state) =>
    state.activeConversationId
      ? (state.conversations[state.activeConversationId]?.isStreaming ?? false)
      : false
  );

  if (anchoredStages.length === 0) return null;

  return (
    <div className="space-y-1.5" data-stage-plan-stack>
      {anchoredStages.map((stageId) => (
        <StageTimelinePlanCard
          key={stageId}
          stageId={stageId}
          stageIndex={stageOrder.indexOf(stageId)}
          stageCount={stageOrder.length}
          plan={plansByStage[stageId]}
          passed={passedStages.includes(stageId)}
          current={current?.stageId === stageId && !passedStages.includes(stageId)}
          isRunning={isRunning}
        />
      ))}
    </div>
  );
});
