/**
 * StagePlanStack
 *
 * Task-mode per-stage plan view (design 2026-06-04 · per-stage-plan-cards).
 *
 * Part 1: one `InlinePlanCard` per harness stage, in run order, each under a
 * small stage-name header — so scoping / recon / vuln / … render as distinct
 * cards instead of one merged list that keeps updating.
 *
 * Part 2: the backend seeds a `pending` placeholder for EVERY stage in the
 * projected DAG up front, so this renders the full operation roadmap from the
 * start. Stages that haven't started yet (a `version: 0` seed whose steps are
 * all `pending`) show as compact greyed rows; once a stage runs, its card fills
 * in with live todos.
 */
import { memo } from "react";
import { InlinePlanCard } from "./InlinePlanCard";
import { prettyStageName } from "./StageMarker";
import type { StagePlansViewModel, TaskPlanViewModel } from "./TaskPlan";

/**
 * A stage is "not started yet" while it only holds the backend seed: version 0
 * with every step still `pending`. The stage-entry seed flips a step to
 * `in_progress` and real `update_plan`s bump the version, so either of those
 * means the stage is live and should render a full card.
 */
function isFutureStage(plan: TaskPlanViewModel): boolean {
  return plan.version === 0 && plan.steps.every((s) => s.status === "pending");
}

export const StagePlanStack = memo(function StagePlanStack({
  stagePlans,
}: {
  stagePlans: StagePlansViewModel;
}) {
  const { stageOrder, plansByStage } = stagePlans;

  return (
    <div className="flex flex-col gap-1.5">
      {stageOrder.map((stageId) => {
        const plan = plansByStage[stageId];
        if (!plan) return null;

        if (isFutureStage(plan)) {
          return (
            <div
              key={stageId}
              className="flex items-center gap-2 px-3 py-1.5 rounded-lg border border-[var(--border-subtle)]/40 bg-background/30 opacity-55"
            >
              <div className="w-3.5 h-3.5 rounded-full border-[1.5px] border-muted-foreground/30 flex-shrink-0" />
              <span className="text-[11.5px] text-muted-foreground/60 truncate">
                {prettyStageName(stageId)}
              </span>
            </div>
          );
        }

        return (
          <div key={stageId} className="flex flex-col gap-0.5">
            <div className="px-1 text-[10.5px] font-medium uppercase tracking-wide text-muted-foreground/45">
              {prettyStageName(stageId)}
            </div>
            <InlinePlanCard plan={plan} />
          </div>
        );
      })}
    </div>
  );
});
