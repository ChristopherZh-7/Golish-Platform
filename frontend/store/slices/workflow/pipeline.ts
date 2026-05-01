import type { PipelineExecution, PipelineStepExecution, TaskPlan } from "../../store-types";
import type { ImmerSet } from "../types";
import type { WorkflowStoreDraft } from "./types";

export function createPipelineActions(set: ImmerSet<WorkflowStoreDraft>) {
  return {
    startPipelineExecution: (sessionId: string, execution: PipelineExecution, blockId?: string) =>
      set((state) => {
        if (!state.timelines[sessionId]) state.timelines[sessionId] = [];
        let planStepIndex: number | undefined;
        const plan = state.sessions?.[sessionId]?.plan as TaskPlan | undefined;
        if (plan) {
          const idx = plan.steps.findIndex((s) => s.status === "in_progress");
          if (idx >= 0) planStepIndex = idx;
        }
        state.timelines[sessionId].push({
          id: blockId ?? `pipeline-${execution.pipelineId}-${Date.now()}`,
          type: "pipeline_progress",
          timestamp: new Date().toISOString(),
          data: execution,
          planStepIndex,
        });
      }),

    updatePipelineStep: (
      sessionId: string,
      executionId: string,
      stepId: string,
      update: Partial<PipelineStepExecution>,
    ) =>
      set((state) => {
        const timeline = state.timelines[sessionId];
        if (!timeline) return;
        const block = timeline.find(
          (b) => b.type === "pipeline_progress" && b.id === executionId,
        );
        if (!block || block.type !== "pipeline_progress") return;
        const step = block.data.steps.find((s) => s.stepId === stepId);
        if (step) Object.assign(step, update);
      }),

    completePipelineExecution: (
      sessionId: string,
      executionId: string,
      status: "completed" | "failed",
    ) =>
      set((state) => {
        const timeline = state.timelines[sessionId];
        if (!timeline) return;
        const block = timeline.find(
          (b) => b.type === "pipeline_progress" && b.id === executionId,
        );
        if (!block || block.type !== "pipeline_progress") return;
        block.data.status = status;
        block.data.finishedAt = new Date().toISOString();
      }),
  };
}
