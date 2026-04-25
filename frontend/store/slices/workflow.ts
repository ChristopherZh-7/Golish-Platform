/**
 * Workflow slice for the Zustand store.
 *
 * Owns the orchestration state: in-flight workflows, sub-agents, the per-session
 * task plan (which lives on `Session.plan` but is mutated through plan actions
 * here), and pipeline-progress timeline blocks. Sub-agent and pipeline blocks
 * are written into the session-owned `timelines` map via untyped `(state: any)`
 * writes so this slice never imports from another slice.
 */

import type {
  ActiveSubAgent,
  ActiveWorkflow,
  PipelineExecution,
  PipelineStepExecution,
  PipelineStepStatus,
  StepStatus,
  TaskPlan,
  UnifiedBlock,
} from "../store-types";
import type { SliceCreator } from "./types";

export interface WorkflowState {
  /** Currently-running workflow per session (null when none). */
  activeWorkflows: Record<string, ActiveWorkflow | null>;
  /** Completed workflows kept for history rendering. */
  workflowHistory: Record<string, ActiveWorkflow[]>;
  /** Sub-agents currently running for a session, ordered by start time. */
  activeSubAgents: Record<string, ActiveSubAgent[]>;
  /** Monotonic counter used to group concurrently-spawned standalone sub-agents. */
  subAgentBatchCounter: Record<string, number>;
  /** parentRequestId → pipeline block/step that owns the sub-agent (for sync). */
  subAgentPipelineMap: Record<string, { blockId: string; stepId: string }>;
}

export interface WorkflowActions {
  // Workflow lifecycle
  startWorkflow: (
    sessionId: string,
    workflow: { workflowId: string; workflowName: string; workflowSessionId: string },
  ) => void;
  workflowStepStarted: (
    sessionId: string,
    step: { stepName: string; stepIndex: number; totalSteps: number },
  ) => void;
  workflowStepCompleted: (
    sessionId: string,
    step: { stepName: string; output: string | null; durationMs: number },
  ) => void;
  completeWorkflow: (
    sessionId: string,
    result: { finalOutput: string; totalDurationMs: number },
  ) => void;
  failWorkflow: (sessionId: string, error: { stepName: string | null; error: string }) => void;
  clearActiveWorkflow: (sessionId: string) => void;
  preserveWorkflowToolCalls: (sessionId: string) => void;

  // Sub-agent lifecycle
  startPromptGeneration: (
    sessionId: string,
    agentId: string,
    parentRequestId: string,
    data: { architectSystemPrompt: string; architectUserMessage: string },
  ) => void;
  completePromptGeneration: (
    sessionId: string,
    agentId: string,
    parentRequestId: string,
    data: { generatedPrompt?: string; success: boolean; durationMs: number },
  ) => void;
  startSubAgent: (
    sessionId: string,
    agent: {
      agentId: string;
      agentName: string;
      parentRequestId: string;
      task: string;
      depth: number;
    },
  ) => void;
  addSubAgentToolCall: (
    sessionId: string,
    parentRequestId: string,
    toolCall: { id: string; name: string; args: Record<string, unknown> },
  ) => void;
  completeSubAgentToolCall: (
    sessionId: string,
    parentRequestId: string,
    toolId: string,
    success: boolean,
    result?: unknown,
  ) => void;
  completeSubAgent: (
    sessionId: string,
    parentRequestId: string,
    result: { response: string; durationMs: number },
  ) => void;
  failSubAgent: (sessionId: string, parentRequestId: string, error: string) => void;
  updateSubAgentStreamingText: (
    sessionId: string,
    parentRequestId: string,
    text: string,
  ) => void;
  appendSubAgentToolOutput: (sessionId: string, toolId: string, chunk: string) => void;
  clearActiveSubAgents: (sessionId: string) => void;

  // Pipeline timeline blocks
  startPipelineExecution: (
    sessionId: string,
    execution: PipelineExecution,
    blockId?: string,
  ) => void;
  updatePipelineStep: (
    sessionId: string,
    executionId: string,
    stepId: string,
    update: Partial<PipelineStepExecution>,
  ) => void;
  completePipelineExecution: (
    sessionId: string,
    executionId: string,
    status: "completed" | "failed",
  ) => void;

  // Plan management
  setPlan: (
    sessionId: string,
    plan: TaskPlan,
    currentMessageId?: string | null,
    newMessageId?: string | null,
  ) => void;
  syncPlanToPipeline: (sessionId: string, plan: TaskPlan) => void;
}

export interface WorkflowSlice extends WorkflowState, WorkflowActions {}

export const initialWorkflowState: WorkflowState = {
  activeWorkflows: {},
  workflowHistory: {},
  activeSubAgents: {},
  subAgentBatchCounter: {},
  subAgentPipelineMap: {},
};

/**
 * Determine whether two step lists represent a structural change (different
 * steps, not just status updates). Uses step IDs when available; for ID-less
 * steps, normalises the text and compares Jaccard similarity ≥ 0.5.
 */
function planStepsStructurallyChanged(
  prev: Array<{ id?: string; step: string }>,
  next: Array<{ id?: string; step: string }>,
): boolean {
  if (prev.length !== next.length) return true;

  const allHaveIds = prev.every((s) => s.id) && next.every((s) => s.id);
  if (allHaveIds) {
    const prevIds = new Set(prev.map((s) => s.id));
    return next.some((s) => !prevIds.has(s.id));
  }

  for (let i = 0; i < prev.length; i++) {
    const pId = prev[i].id;
    const nId = next[i].id;
    if (pId && nId) {
      if (pId !== nId) return true;
      continue;
    }
    const pWords = new Set(
      prev[i].step.toLowerCase().replace(/[^\w\s]/g, "").split(/\s+/).filter(Boolean),
    );
    const nWords = new Set(
      next[i].step.toLowerCase().replace(/[^\w\s]/g, "").split(/\s+/).filter(Boolean),
    );
    const union = new Set([...pWords, ...nWords]);
    const intersection = [...pWords].filter((w) => nWords.has(w)).length;
    if (union.size > 0 && intersection / union.size < 0.5) return true;
  }
  return false;
}

/**
 * Mirror a sub-agent's data into its corresponding timeline representation.
 * If the sub-agent is mapped to a pipeline step, write there; otherwise update
 * the standalone `sub_agent_activity` block.
 */
function syncSubAgentToTimeline(
  subAgentPipelineMap: Record<string, { blockId: string; stepId: string }>,
  timeline: UnifiedBlock[],
  parentRequestId: string,
  agent: ActiveSubAgent,
): void {
  const mapping = subAgentPipelineMap[parentRequestId];
  if (mapping) {
    const pBlock = timeline.find((b) => b.id === mapping.blockId);
    if (pBlock && pBlock.type === "pipeline_progress") {
      const step = pBlock.data.steps.find((s) => s.stepId === mapping.stepId);
      if (step?.subAgents) {
        const idx = step.subAgents.findIndex((a) => a.parentRequestId === parentRequestId);
        if (idx >= 0) {
          step.subAgents[idx] = { ...agent };
        } else {
          step.subAgents.push({ ...agent });
        }
        return;
      }
    }
  }
  const block = timeline.find(
    (b) => b.type === "sub_agent_activity" && b.data.parentRequestId === parentRequestId,
  );
  if (block && block.type === "sub_agent_activity") {
    block.data = { ...agent };
  }
}

export const createWorkflowSlice: SliceCreator<WorkflowSlice> = (set) => ({
  ...initialWorkflowState,

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
    set((state: any) => {
      const workflow = state.activeWorkflows[sessionId];
      const toolCalls = state.activeToolCalls?.[sessionId];
      if (!workflow || !toolCalls) return;

      const workflowToolCalls = toolCalls.filter((tool: any) => {
        const source = tool.source;
        return source?.type === "workflow" && source.workflowId === workflow.workflowId;
      });

      workflow.toolCalls = workflowToolCalls;
    }),

  startPromptGeneration: (sessionId, agentId, parentRequestId, data) =>
    set((state: any) => {
      if (!state.activeSubAgents[sessionId]) {
        state.activeSubAgents[sessionId] = [];
      }
      const now = new Date().toISOString();
      const existing = state.activeSubAgents[sessionId].find(
        (a: ActiveSubAgent) => a.parentRequestId === parentRequestId,
      );
      if (existing) {
        existing.promptGeneration = {
          status: "generating",
          architectSystemPrompt: data.architectSystemPrompt,
          architectUserMessage: data.architectUserMessage,
        };
      } else {
        const newAgent: ActiveSubAgent = {
          agentId,
          agentName: "",
          parentRequestId,
          task: "",
          depth: 0,
          status: "running",
          toolCalls: [],
          entries: [],
          startedAt: now,
          promptGeneration: {
            status: "generating",
            architectSystemPrompt: data.architectSystemPrompt,
            architectUserMessage: data.architectUserMessage,
          },
        };
        state.activeSubAgents[sessionId].push(newAgent);
      }

      if (!state.timelines[sessionId]) state.timelines[sessionId] = [];
      const timeline = state.timelines[sessionId] as UnifiedBlock[];
      const blockId = `sub-agent-${parentRequestId}`;
      const agentData = state.activeSubAgents[sessionId].find(
        (a: ActiveSubAgent) => a.parentRequestId === parentRequestId,
      );
      if (!agentData) return;
      const existingBlock = timeline.find((b) => b.id === blockId);
      if (existingBlock && existingBlock.type === "sub_agent_activity") {
        existingBlock.data = { ...agentData };
      } else if (!existingBlock) {
        let stepIdx: number | undefined;
        const planObj = state.sessions?.[sessionId]?.plan as TaskPlan | undefined;
        if (planObj) {
          const si = planObj.steps.findIndex((s) => s.status === "in_progress");
          if (si >= 0) stepIdx = si;
        }
        timeline.push({
          id: blockId,
          type: "sub_agent_activity" as const,
          timestamp: now,
          data: { ...agentData },
          planStepIndex: stepIdx,
        });
      }
    }),

  completePromptGeneration: (sessionId, _agentId, parentRequestId, data) =>
    set((state: any) => {
      const agents = state.activeSubAgents[sessionId];
      if (!agents) return;
      const agent = agents.find((a: ActiveSubAgent) => a.parentRequestId === parentRequestId);
      if (agent?.promptGeneration) {
        agent.promptGeneration.status = data.success ? "completed" : "failed";
        agent.promptGeneration.generatedPrompt = data.generatedPrompt;
        agent.promptGeneration.durationMs = data.durationMs;
      }
      const timeline = state.timelines[sessionId] as UnifiedBlock[] | undefined;
      if (timeline && agent) {
        const block = timeline.find(
          (b) => b.type === "sub_agent_activity" && b.data.parentRequestId === parentRequestId,
        );
        if (block && block.type === "sub_agent_activity") {
          block.data = { ...agent };
        }
      }
    }),

  startSubAgent: (sessionId, agent) =>
    set((state: any) => {
      if (!state.activeSubAgents[sessionId]) {
        state.activeSubAgents[sessionId] = [];
      }
      const now = new Date().toISOString();
      const existing = agent.parentRequestId
        ? state.activeSubAgents[sessionId].find(
            (a: ActiveSubAgent) => a.parentRequestId === agent.parentRequestId,
          )
        : undefined;
      if (existing) {
        existing.agentId = agent.agentId;
        existing.agentName = agent.agentName;
        existing.task = agent.task;
        existing.depth = agent.depth;
      } else {
        const newAgent: ActiveSubAgent = {
          agentId: agent.agentId,
          agentName: agent.agentName,
          parentRequestId: agent.parentRequestId,
          task: agent.task,
          depth: agent.depth,
          status: "running",
          toolCalls: [],
          entries: [],
          startedAt: now,
        };
        state.activeSubAgents[sessionId].push(newAgent);
      }

      if (!state.timelines[sessionId]) state.timelines[sessionId] = [];
      const timeline = state.timelines[sessionId] as UnifiedBlock[];
      const agentData = state.activeSubAgents[sessionId].find(
        (a: ActiveSubAgent) => a.parentRequestId === agent.parentRequestId,
      );
      if (!agentData) return;

      const AI_PREFIXES = ["AI:", "ai:"];
      let attachedToPipeline = false;
      for (const block of timeline) {
        if (block.type !== "pipeline_progress" || block.data.status !== "running") continue;
        const aiStep = block.data.steps.find(
          (s) =>
            s.status === "running" &&
            (AI_PREFIXES.some((p) => s.command.startsWith(p)) || s.name.includes("(AI)")),
        );
        if (aiStep) {
          if (!aiStep.subAgents) aiStep.subAgents = [];
          const existingIdx = aiStep.subAgents.findIndex(
            (a) => a.parentRequestId === agent.parentRequestId,
          );
          if (existingIdx >= 0) {
            aiStep.subAgents[existingIdx] = { ...agentData };
          } else {
            aiStep.subAgents.push({ ...agentData });
          }
          state.subAgentPipelineMap[agent.parentRequestId] = {
            blockId: block.id,
            stepId: aiStep.stepId,
          };
          attachedToPipeline = true;
          break;
        }
      }

      if (attachedToPipeline) return;

      const blockId = `sub-agent-${agent.parentRequestId}`;
      const existingBlock = timeline.find((b) => b.id === blockId);
      if (existingBlock && existingBlock.type === "sub_agent_activity") {
        existingBlock.data = { ...agentData };
      } else if (!existingBlock) {
        const currentAgents = state.activeSubAgents[sessionId];
        const anyRunning = currentAgents.some(
          (a: ActiveSubAgent) =>
            a.status === "running" && a.parentRequestId !== agent.parentRequestId,
        );
        if (!anyRunning) {
          state.subAgentBatchCounter[sessionId] =
            (state.subAgentBatchCounter[sessionId] ?? 0) + 1;
        }
        const batchId = `batch-${state.subAgentBatchCounter[sessionId] ?? 1}`;
        let planStepIdx: number | undefined;
        const planData = state.sessions?.[sessionId]?.plan as TaskPlan | undefined;
        if (planData) {
          const idx = planData.steps.findIndex((s) => s.status === "in_progress");
          if (idx >= 0) planStepIdx = idx;
        }
        timeline.push({
          id: blockId,
          type: "sub_agent_activity" as const,
          timestamp: now,
          data: { ...agentData },
          batchId,
          planStepIndex: planStepIdx,
        });
      }
    }),

  addSubAgentToolCall: (sessionId, parentRequestId, toolCall) =>
    set((state: any) => {
      const agents = state.activeSubAgents[sessionId];
      if (!agents) return;

      const agent = agents.find((a: ActiveSubAgent) => a.parentRequestId === parentRequestId);
      if (agent) {
        if (agent.toolCalls.some((tc: { id: string }) => tc.id === toolCall.id)) return;
        agent.toolCalls.push({
          ...toolCall,
          status: "running",
          startedAt: new Date().toISOString(),
        });
        agent.entries.push({ kind: "tool_call", toolCallId: toolCall.id });
      }
      const timeline = state.timelines[sessionId] as UnifiedBlock[] | undefined;
      if (timeline && agent) {
        syncSubAgentToTimeline(state.subAgentPipelineMap, timeline, parentRequestId, agent);
      }
    }),

  completeSubAgentToolCall: (sessionId, parentRequestId, toolId, success, result) =>
    set((state: any) => {
      const agents = state.activeSubAgents[sessionId];
      if (!agents) return;

      const agent = agents.find((a: ActiveSubAgent) => a.parentRequestId === parentRequestId);
      if (agent) {
        const tool = agent.toolCalls.find((t: { id: string }) => t.id === toolId);
        if (tool) {
          tool.status = success ? "completed" : "error";
          tool.result = result;
          tool.completedAt = new Date().toISOString();
        }
      }
      const timeline = state.timelines[sessionId] as UnifiedBlock[] | undefined;
      if (timeline && agent) {
        syncSubAgentToTimeline(state.subAgentPipelineMap, timeline, parentRequestId, agent);
      }
    }),

  completeSubAgent: (sessionId, parentRequestId, result) =>
    set((state: any) => {
      const agents = state.activeSubAgents[sessionId];
      if (!agents) return;

      const agent = agents.find((a: ActiveSubAgent) => a.parentRequestId === parentRequestId);
      if (agent) {
        agent.status = "completed";
        agent.response = result.response;
        agent.durationMs = result.durationMs;
        agent.completedAt = new Date().toISOString();
      }
      const timeline = state.timelines[sessionId] as UnifiedBlock[] | undefined;
      if (timeline && agent) {
        syncSubAgentToTimeline(state.subAgentPipelineMap, timeline, parentRequestId, agent);
      }
    }),

  failSubAgent: (sessionId, parentRequestId, error) =>
    set((state: any) => {
      const agents = state.activeSubAgents[sessionId];
      if (!agents) return;

      const agent = agents.find((a: ActiveSubAgent) => a.parentRequestId === parentRequestId);
      if (agent) {
        agent.status = "error";
        agent.error = error;
        agent.completedAt = new Date().toISOString();
      }
      const timeline = state.timelines[sessionId] as UnifiedBlock[] | undefined;
      if (timeline && agent) {
        syncSubAgentToTimeline(state.subAgentPipelineMap, timeline, parentRequestId, agent);
      }
    }),

  updateSubAgentStreamingText: (sessionId, parentRequestId, text) =>
    set((state: any) => {
      const agents = state.activeSubAgents[sessionId];
      if (!agents) return;
      const agent = agents.find((a: ActiveSubAgent) => a.parentRequestId === parentRequestId);
      if (agent) {
        agent.streamingText = text;
        const lastEntry = agent.entries[agent.entries.length - 1];
        if (lastEntry && lastEntry.kind === "text") {
          lastEntry.text = text;
        } else {
          agent.entries.push({ kind: "text", text });
        }
      }
      const timeline = state.timelines[sessionId] as UnifiedBlock[] | undefined;
      if (timeline && agent) {
        syncSubAgentToTimeline(state.subAgentPipelineMap, timeline, parentRequestId, agent);
      }
    }),

  appendSubAgentToolOutput: (sessionId, toolId, chunk) =>
    set((state: any) => {
      const agents = state.activeSubAgents[sessionId];
      if (!agents) return;
      for (const agent of agents) {
        const tool = agent.toolCalls.find((t: { id: string }) => t.id === toolId);
        if (tool) {
          tool.streamingOutput = (tool.streamingOutput ?? "") + chunk;
          const timeline = state.timelines[sessionId] as UnifiedBlock[] | undefined;
          if (timeline) {
            syncSubAgentToTimeline(
              state.subAgentPipelineMap,
              timeline,
              agent.parentRequestId,
              agent,
            );
          }
          return;
        }
      }
    }),

  clearActiveSubAgents: (sessionId) =>
    set((state) => {
      state.activeSubAgents[sessionId] = [];
    }),

  startPipelineExecution: (sessionId, execution, blockId) =>
    set((state: any) => {
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

  updatePipelineStep: (sessionId, executionId, stepId, update) =>
    set((state: any) => {
      const timeline = state.timelines[sessionId] as UnifiedBlock[] | undefined;
      if (!timeline) return;
      const block = timeline.find(
        (b) => b.type === "pipeline_progress" && b.id === executionId,
      );
      if (!block || block.type !== "pipeline_progress") return;
      const step = block.data.steps.find((s) => s.stepId === stepId);
      if (step) Object.assign(step, update);
    }),

  completePipelineExecution: (sessionId, executionId, status) =>
    set((state: any) => {
      const timeline = state.timelines[sessionId] as UnifiedBlock[] | undefined;
      if (!timeline) return;
      const block = timeline.find(
        (b) => b.type === "pipeline_progress" && b.id === executionId,
      );
      if (!block || block.type !== "pipeline_progress") return;
      block.data.status = status;
      block.data.finishedAt = new Date().toISOString();
    }),

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

export const selectActiveWorkflow = <T extends WorkflowState>(
  state: T,
  sessionId: string,
): ActiveWorkflow | null => state.activeWorkflows[sessionId] ?? null;

export const selectActiveSubAgents = <T extends WorkflowState>(
  state: T,
  sessionId: string,
): ActiveSubAgent[] => state.activeSubAgents[sessionId] ?? [];
