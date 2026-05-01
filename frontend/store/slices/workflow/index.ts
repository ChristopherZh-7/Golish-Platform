/**
 * Workflow slice — composed from focused sub-modules.
 *
 * Sub-modules:
 *   types.ts      — State & action interfaces, initial state
 *   sub-agent.ts  — Sub-agent lifecycle actions + timeline sync helper
 *   pipeline.ts   — Pipeline timeline block actions
 *   plan.ts       — Plan management actions (setPlan, syncPlanToPipeline)
 */

export type { WorkflowActions, WorkflowSlice, WorkflowState, WorkflowStoreDraft } from "./types";
export { initialWorkflowState } from "./types";

import type { SliceCreator } from "../types";
import { createPipelineActions } from "./pipeline";
import { createPlanActions } from "./plan";
import { createSubAgentActions } from "./sub-agent";
import type { WorkflowSlice, WorkflowStoreDraft } from "./types";
import { initialWorkflowState } from "./types";

export const createWorkflowSlice: SliceCreator<WorkflowSlice, WorkflowStoreDraft> = (set) => ({
  ...initialWorkflowState,

  // ── Workflow lifecycle ──────────────────────────────────────────────

  startWorkflow: (sessionId, workflow) =>
    set((state) => {
      state.activeWorkflows[sessionId] = {
        workflowId: workflow.workflowId,
        workflowName: workflow.workflowName,
        sessionId: workflow.workflowSessionId,
        status: "running",
        steps: [],
        currentStepIndex: -1,
        totalSteps: 0,
        startedAt: new Date().toISOString(),
      };
    }),

  workflowStepStarted: (sessionId, step) =>
    set((state) => {
      const workflow = state.activeWorkflows[sessionId];
      if (!workflow) return;
      workflow.currentStepIndex = step.stepIndex;
      workflow.totalSteps = step.totalSteps;
      if (!workflow.steps[step.stepIndex]) {
        workflow.steps[step.stepIndex] = {
          name: step.stepName,
          index: step.stepIndex,
          status: "running",
          startedAt: new Date().toISOString(),
        };
      } else {
        workflow.steps[step.stepIndex].status = "running";
        workflow.steps[step.stepIndex].startedAt = new Date().toISOString();
      }
    }),

  workflowStepCompleted: (sessionId, step) =>
    set((state) => {
      const workflow = state.activeWorkflows[sessionId];
      if (!workflow) return;
      const stepData = workflow.steps.find((s) => s.name === step.stepName);
      if (stepData) {
        stepData.status = "completed";
        stepData.output = step.output;
        stepData.durationMs = step.durationMs;
        stepData.completedAt = new Date().toISOString();
      }
    }),

  completeWorkflow: (sessionId, result) =>
    set((state) => {
      const workflow = state.activeWorkflows[sessionId];
      if (!workflow) return;
      workflow.status = "completed";
      workflow.finalOutput = result.finalOutput;
      workflow.totalDurationMs = result.totalDurationMs;
      workflow.completedAt = new Date().toISOString();
      if (!state.workflowHistory[sessionId]) {
        state.workflowHistory[sessionId] = [];
      }
      state.workflowHistory[sessionId].push({ ...workflow });
    }),

  failWorkflow: (sessionId, error) =>
    set((state) => {
      const workflow = state.activeWorkflows[sessionId];
      if (!workflow) return;
      workflow.status = "error";
      workflow.error = error.error;
      workflow.completedAt = new Date().toISOString();
      if (error.stepName) {
        const stepData = workflow.steps.find((s) => s.name === error.stepName);
        if (stepData) {
          stepData.status = "error";
        }
      }
      if (!state.workflowHistory[sessionId]) {
        state.workflowHistory[sessionId] = [];
      }
      state.workflowHistory[sessionId].push({ ...workflow });
    }),

  clearActiveWorkflow: (sessionId) =>
    set((state) => {
      state.activeWorkflows[sessionId] = null;
    }),

  preserveWorkflowToolCalls: (sessionId) =>
    set((state) => {
      const workflow = state.activeWorkflows[sessionId];
      const toolCalls = state.activeToolCalls?.[sessionId];
      if (!workflow || !toolCalls) return;
      const workflowToolCalls = toolCalls.filter((tool) => {
        const source = tool.source;
        return source?.type === "workflow" && source.workflowId === workflow.workflowId;
      });
      workflow.toolCalls = workflowToolCalls;
    }),

  // ── Composed sub-module actions ────────────────────────────────────

  ...createSubAgentActions(set),
  ...createPipelineActions(set),
  ...createPlanActions(set),
});

// ── Selectors ──────────────────────────────────────────────────────────

import type { ActiveSubAgent, ActiveWorkflow } from "../../store-types";
import type { WorkflowState } from "./types";

export const selectActiveWorkflow = <T extends WorkflowState>(
  state: T,
  sessionId: string
): ActiveWorkflow | null => state.activeWorkflows[sessionId] ?? null;

export const selectActiveSubAgents = <T extends WorkflowState>(
  state: T,
  sessionId: string
): ActiveSubAgent[] => state.activeSubAgents[sessionId] ?? [];
