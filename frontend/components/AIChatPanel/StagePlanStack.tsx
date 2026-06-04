/**
 * StagePlanStack
 *
 * Task-mode roadmap (design 2026-06-04 · per-stage-plan-cards + roadmap UX
 * overhaul). Renders one `StageRow` per harness stage in run order: the backend
 * seeds a `pending` placeholder for every stage in the projected DAG up front, so
 * the whole operation roadmap (scoping → recon → … → reporting) shows from the
 * start. Each row fills in with live todos when its stage runs, and flips to a
 * green check only when the stage's evidence gate PASSES (`passedStages`).
 */
import { memo } from "react";
import { useStore } from "@/store";
import { StageRow } from "./StageRow";
import type { StagePlansViewModel } from "./TaskPlan";

export const StagePlanStack = memo(function StagePlanStack({
  stagePlans,
}: {
  stagePlans: StagePlansViewModel;
}) {
  const { stageOrder, plansByStage, passedStages } = stagePlans;

  // Live "agent is working" signal for the active conversation. The roadmap is
  // restored from localStorage on refresh/restart, so an "active" stage must NOT
  // animate a spinner unless a turn is actually streaming right now — otherwise a
  // closed/idle run looks like it's still running until the user resumes.
  const isRunning = useStore((s) =>
    s.activeConversationId ? (s.conversations[s.activeConversationId]?.isStreaming ?? false) : false
  );

  return (
    // `data-stage-roadmap` anchors the sticky `StageProgressBar`: the panel only
    // reveals that bar once this inline roadmap has scrolled out of view above.
    <div className="flex flex-col gap-1.5" data-stage-roadmap>
      {stageOrder.map((stageId) => {
        const plan = plansByStage[stageId];
        if (!plan) return null;
        return (
          <StageRow
            key={stageId}
            stageId={stageId}
            plan={plan}
            passed={passedStages.includes(stageId)}
            isRunning={isRunning}
          />
        );
      })}
    </div>
  );
});
