import { planStepsStructurallyChanged } from "@/lib/plan-structural-change";
import { createResetStageSeed } from "@/lib/stage-reset";
import type { TaskPlan } from "../../store-types";
import type { ImmerSet } from "../types";
import type { WorkflowStoreDraft } from "./types";

export function createPlanActions(set: ImmerSet<WorkflowStoreDraft>) {
  return {
    setPlan: (
      sessionId: string,
      plan: TaskPlan,
      currentMessageId?: string | null,
      newMessageId?: string | null
    ) =>
      set((state) => {
        if (!state.sessions[sessionId]) {
          (state.sessions as Record<string, unknown>)[sessionId] = {
            id: sessionId,
            tabType: "terminal" as const,
            inputMode: "terminal" as const,
            logicalTerminalId: sessionId,
          };
        }

        const prev = state.sessions[sessionId].plan as TaskPlan | undefined;

        if (prev && prev.version === plan.version) {
          return;
        }

        const msgId = currentMessageId ?? state.sessions[sessionId].planMessageId;
        let stepsChanged = false;
        if (prev && msgId) {
          stepsChanged = planStepsStructurallyChanged(prev.steps, plan.steps);
          if (stepsChanged) {
            if (!state.sessions[sessionId].retiredPlans) {
              state.sessions[sessionId].retiredPlans = [];
            }
            const retiredSteps = prev.steps.map((s) =>
              s.status === "in_progress" || s.status === "pending"
                ? { ...s, status: "cancelled" as const }
                : { ...s }
            );
            state.sessions[sessionId].retiredPlans?.push({
              plan: { ...prev, steps: retiredSteps },
              messageId: msgId,
              retiredAt: new Date().toISOString(),
            });
          }
        }

        (state.sessions[sessionId] as Record<string, unknown>).plan = plan;
        // Pin the card where the plan first appeared. Only move the anchor for a
        // structurally new plan (its predecessor is retired above) — status-only
        // progress (pending→in_progress→completed) must not chase the latest
        // message to the bottom.
        if (newMessageId !== undefined && (!prev || stepsChanged)) {
          (state.sessions[sessionId] as Record<string, unknown>).planMessageId = newMessageId;
        }

        const timeline = state.timelines[sessionId];
        if (timeline) {
          const firstInProgress = plan.steps.findIndex((s) => s.status === "in_progress");
          if (firstInProgress >= 0) {
            const stepId = plan.steps[firstInProgress].id ?? undefined;
            for (const block of timeline) {
              if (
                block.type === "ai_tool_execution" &&
                block.data.planStepIndex == null &&
                block.data.status === "running"
              ) {
                block.data.planStepIndex = firstInProgress;
                block.data.planStepId = stepId;
              }
            }
          }

          if (prev) {
            for (let i = 0; i < plan.steps.length; i++) {
              const wasInProgress = prev.steps[i]?.status === "in_progress";
              const nowCompleted = plan.steps[i].status === "completed";
              if (wasInProgress && nowCompleted) {
                const stepId2 = plan.steps[i].id;
                for (const block of timeline) {
                  if (
                    block.type === "ai_tool_execution" &&
                    block.data.status === "running" &&
                    block.data.planStepId === stepId2
                  ) {
                    block.data.status = "completed";
                    block.data.completedAt = new Date().toISOString();
                    const start = new Date(block.data.startedAt).getTime();
                    block.data.durationMs = Date.now() - start;
                  }
                }
              }
            }
          }
        }
      }),

    setStagePlan: (sessionId: string, stageId: string, plan: TaskPlan) =>
      set((state) => {
        if (!state.sessions[sessionId]) {
          (state.sessions as Record<string, unknown>)[sessionId] = {
            id: sessionId,
            tabType: "terminal" as const,
            inputMode: "terminal" as const,
            logicalTerminalId: sessionId,
          };
        }
        const session = state.sessions[sessionId] as Record<string, unknown> & {
          plansByStage?: Record<string, TaskPlan>;
          stageOrder?: string[];
          stagePlanMessageIds?: Record<string, string>;
        };
        if (!session.plansByStage) session.plansByStage = {};
        if (!session.stageOrder) session.stageOrder = [];
        const prev = session.plansByStage[stageId];
        // Drop a same-version replay of a REAL update (version >= 1). Version 0
        // is the seed sentinel: the op-start roadmap emits a `pending` seed per
        // stage (v0) and the stage-entry emits an `in_progress` seed (v0), so v0
        // updates must always apply to let a newer seed replace an older one.
        // Any real `update_plan` (version >= 1) then supersedes the seed.
        if (prev && prev.version === plan.version && plan.version !== 0) return;
        // A version-0 seed must NEVER downgrade an already-real plan (version >= 1):
        // re-entering a stage (e.g. a gate-BLOCK retry) re-emits the v0 entry seed,
        // which would otherwise wipe the agent's todos and revert the card to
        // "starting…" (the plan "disappears").
        if (prev && prev.version >= 1 && plan.version === 0) return;
        session.plansByStage[stageId] = plan;
        if (!session.stageOrder.includes(stageId)) session.stageOrder.push(stageId);
      }),

    anchorStagePlan: (sessionId: string, stageId: string, messageId: string) =>
      set((state) => {
        // Anchoring must never manufacture an AI-session alias while terminal
        // restoration is still in flight. The central event handler owns plan
        // buffering; a later chat event can retry this no-op safely.
        const session = state.sessions[sessionId];
        if (!session || !stageId || !messageId) return;
        if (!session.stagePlanMessageIds) session.stagePlanMessageIds = {};
        // The first stage-start message owns the card permanently. Later plan
        // versions update its content in place instead of moving history.
        if (!session.stagePlanMessageIds[stageId]) {
          session.stagePlanMessageIds[stageId] = messageId;
        }
      }),

    markStagePassed: (sessionId: string, stageId: string) =>
      set((state) => {
        // Mirror `setStagePlan`: create the session row if absent so the
        // authoritative `stage_passed` write is never silently dropped (the
        // old early `return` left the stage stuck on "starting…" whenever this
        // fired before/without a matching session row).
        if (!state.sessions[sessionId]) {
          (state.sessions as Record<string, unknown>)[sessionId] = {
            id: sessionId,
            tabType: "terminal" as const,
            inputMode: "terminal" as const,
            logicalTerminalId: sessionId,
          };
        }
        const session = state.sessions[sessionId] as Record<string, unknown> & {
          passedStages?: string[];
        };
        if (!session.passedStages) session.passedStages = [];
        if (!session.passedStages.includes(stageId)) session.passedStages.push(stageId);
      }),

    rewindStagePlans: (sessionId: string, affectedStages: string[], selectedStage: string) =>
      set((state) => {
        if (
          !sessionId ||
          !selectedStage ||
          affectedStages.length === 0 ||
          !affectedStages.includes(selectedStage)
        ) {
          return;
        }
        const session = state.sessions[sessionId];
        if (!session) return;
        const affected = new Set(affectedStages);
        if (!session.plansByStage) session.plansByStage = {};
        for (const stage of affected) {
          delete session.plansByStage[stage];
          if (session.stagePlanMessageIds) delete session.stagePlanMessageIds[stage];
        }
        session.plansByStage[selectedStage] = createResetStageSeed(selectedStage);

        session.stageOrder = (session.stageOrder ?? []).filter((stage) => !affected.has(stage));
        session.stageOrder.push(selectedStage);
        session.passedStages = (session.passedStages ?? []).filter((stage) => !affected.has(stage));
      }),
  };
}
