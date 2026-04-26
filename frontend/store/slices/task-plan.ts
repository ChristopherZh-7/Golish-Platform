import type {
  PipelineExecution,
  PipelineStepExecution,
  PipelineStepStatus,
  StepStatus,
  TaskPlan,
  UnifiedBlock,
} from "../store-types";
import type { ImmerSet } from "./types";
import type { WorkflowActions } from "./workflow";
import { planStepsStructurallyChanged } from "@/lib/plan-structural-change";

type TaskPlanActionKeys = "setPlan" | "syncPlanToPipeline";

export const createTaskPlanActions = (
  set: ImmerSet<any>,
): Pick<WorkflowActions, TaskPlanActionKeys> => ({
  setPlan: (sessionId, plan, currentMessageId, newMessageId) =>
    set((state: any) => {
      if (!state.sessions[sessionId]) {
        state.sessions[sessionId] = {
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
      if (prev && msgId) {
        const stepsChanged = planStepsStructurallyChanged(prev.steps, plan.steps);
        if (stepsChanged) {
          if (!state.sessions[sessionId].retiredPlans) {
            state.sessions[sessionId].retiredPlans = [];
          }
          const retiredSteps = prev.steps.map((s) =>
            s.status === "in_progress" || s.status === "pending"
              ? { ...s, status: "cancelled" as const }
              : { ...s },
          );
          state.sessions[sessionId].retiredPlans.push({
            plan: { ...prev, steps: retiredSteps },
            messageId: msgId,
            retiredAt: new Date().toISOString(),
          });
        }
      }

      state.sessions[sessionId].plan = plan;
      if (newMessageId !== undefined) {
        state.sessions[sessionId].planMessageId = newMessageId;
      }

      const timeline = state.timelines[sessionId] as UnifiedBlock[] | undefined;
      if (timeline) {
        const firstInProgress = plan.steps.findIndex((s) => s.status === "in_progress");
        if (firstInProgress >= 0) {
          const stepId = plan.steps[firstInProgress].id;
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

  syncPlanToPipeline: (sessionId, plan) =>
    set((state: any) => {
      if (!state.timelines[sessionId]) state.timelines[sessionId] = [];
      const timeline = state.timelines[sessionId] as UnifiedBlock[];

      const STATUS_MAP: Record<StepStatus, PipelineStepStatus> = {
        pending: "pending",
        in_progress: "running",
        completed: "success",
        cancelled: "skipped",
        failed: "failed",
      };

      const blockId = `plan-pipeline-${sessionId}`;
      const now = new Date().toISOString();

      const steps: PipelineStepExecution[] = plan.steps.map((s, i) => ({
        stepId: s.id ?? `plan-step-${i}`,
        name: s.step,
        command: "",
        status: STATUS_MAP[s.status] ?? "pending",
        startedAt: s.status !== "pending" ? now : undefined,
      }));

      const anyRunning = plan.summary.in_progress > 0;
      const allDone = plan.summary.total > 0 && plan.summary.completed === plan.summary.total;
      const pipelineStatus = allDone ? "completed" : anyRunning ? "running" : "pending";

      const existing = timeline.find((b) => b.id === blockId);
      if (existing && existing.type === "pipeline_progress") {
        for (const newStep of steps) {
          const prev = existing.data.steps.find((s) => s.stepId === newStep.stepId);
          if (prev?.subAgents) newStep.subAgents = prev.subAgents;
        }
        existing.data.steps = steps;
        existing.data.status = pipelineStatus;
        if (allDone) existing.data.finishedAt = now;
      } else if (!existing) {
        const execution: PipelineExecution = {
          pipelineId: `plan-${sessionId}`,
          pipelineName: plan.explanation ?? "Task Plan",
          target: "",
          steps,
          status: pipelineStatus,
          startedAt: now,
        };
        timeline.push({
          id: blockId,
          type: "pipeline_progress",
          timestamp: now,
          data: execution,
        });
      }
    }),
});
