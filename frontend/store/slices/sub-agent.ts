import type { ActiveSubAgent, TaskPlan, UnifiedBlock } from "../store-types";
import type { ImmerSet } from "./types";
import type { WorkflowActions } from "./workflow";

type SubAgentActionKeys =
  | "startPromptGeneration"
  | "completePromptGeneration"
  | "startSubAgent"
  | "addSubAgentToolCall"
  | "completeSubAgentToolCall"
  | "completeSubAgent"
  | "failSubAgent"
  | "updateSubAgentStreamingText"
  | "appendSubAgentToolOutput"
  | "clearActiveSubAgents";

/**
 * Mirror a sub-agent's data into its corresponding timeline representation.
 * If the sub-agent is mapped to a pipeline step, write there; otherwise update
 * the standalone `sub_agent_activity` block.
 */
export function syncSubAgentToTimeline(
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

export const createSubAgentActions = (
  set: ImmerSet<any>,
): Pick<WorkflowActions, SubAgentActionKeys> => ({
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
});
