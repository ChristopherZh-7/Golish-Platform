/**
 * Persistent task status directly below the conversation tabs.
 *
 * This is intentionally one line: stage-local todos live in their historical
 * message cards. The separate workflow control is the only expandable region.
 */
import { CheckCircle2, ListTree, Loader2 } from "lucide-react";
import { memo, useMemo, useState } from "react";
import { cn } from "@/lib/utils";
import { prettyStageName } from "./StageMarker";
import { resolveCurrentStagePlan } from "./StagePlanStack";
import { StageRow } from "./StageRow";
import type { StagePlansViewModel } from "./TaskPlan";

export const StageProgressBar = memo(function StageProgressBar({
  stagePlans,
  isRunning,
  className,
}: {
  stagePlans: StagePlansViewModel;
  isRunning: boolean;
  className?: string;
}) {
  const { stageOrder, plansByStage, passedStages } = stagePlans;
  const [workflowOpen, setWorkflowOpen] = useState(false);
  const view = useMemo(() => resolveCurrentStagePlan(stagePlans), [stagePlans]);

  if (!view) return null;

  const total = view.plan?.summary.total ?? 0;
  const completed = view.plan?.summary.completed ?? 0;
  const progress = Math.round((view.passedCount / stageOrder.length) * 100);
  const activity = view.allPassed
    ? "Workflow complete"
    : (view.currentStep?.step ??
      (view.hasRealPlan
        ? "Checking stage evidence…"
        : isRunning
          ? "Preparing stage plan…"
          : "Stage plan pending"));

  return (
    <div
      className={cn(
        "border-b border-[var(--border-subtle)] bg-background/92 backdrop-blur-md",
        className
      )}
      data-stage-progress-bar
    >
      <div className="flex min-w-0 items-stretch">
        <div className="flex min-w-0 flex-1 items-center gap-2 px-3 py-1.5">
          {view.allPassed ? (
            <CheckCircle2 className="h-3.5 w-3.5 flex-shrink-0 text-emerald-500" />
          ) : isRunning ? (
            <Loader2 className="h-3.5 w-3.5 flex-shrink-0 animate-spin text-accent" />
          ) : (
            <span className="h-3.5 w-3.5 flex-shrink-0 rounded-full border-[1.5px] border-accent/50" />
          )}
          <span className="flex-shrink-0 truncate text-[11.5px] font-semibold text-foreground">
            {prettyStageName(view.stageId)}
          </span>
          <span className="flex-shrink-0 text-[10px] tabular-nums text-muted-foreground/55">
            Stage {view.idx + 1}/{stageOrder.length}
          </span>
          {view.hasRealPlan && total > 0 && (
            <span className="flex-shrink-0 text-[10px] tabular-nums text-muted-foreground/55">
              · Step {completed}/{total}
            </span>
          )}
          <span className="min-w-0 flex-1 truncate text-[11px] text-muted-foreground/65">
            · {activity}
          </span>
        </div>

        <button
          type="button"
          aria-expanded={workflowOpen}
          aria-controls="top-stage-workflow"
          aria-label={workflowOpen ? "Hide workflow" : "Show workflow"}
          title={workflowOpen ? "Hide workflow" : "Show workflow"}
          onClick={() => setWorkflowOpen((open) => !open)}
          className="flex w-10 flex-shrink-0 items-center justify-center border-l border-[var(--border-subtle)] text-muted-foreground/55 transition-colors hover:bg-accent/[0.05] hover:text-foreground"
        >
          <ListTree className="h-3.5 w-3.5" />
        </button>
      </div>

      {workflowOpen && (
        <div
          id="top-stage-workflow"
          role="region"
          aria-label="Stage workflow"
          className="max-h-64 space-y-0.5 overflow-y-auto border-t border-[var(--border-subtle)]/60 px-2 py-1.5"
        >
          {stageOrder.map((stageId) => {
            const stagePlan = plansByStage[stageId];
            if (!stagePlan) return null;
            return (
              <StageRow
                key={stageId}
                stageId={stageId}
                plan={stagePlan}
                passed={passedStages.includes(stageId)}
                isRunning={isRunning}
              />
            );
          })}
        </div>
      )}

      <div className="h-px w-full bg-[var(--border-subtle)]/35">
        <div
          className="h-full bg-accent/70 transition-[width] duration-500 ease-out"
          style={{ width: `${progress}%` }}
        />
      </div>
    </div>
  );
});
