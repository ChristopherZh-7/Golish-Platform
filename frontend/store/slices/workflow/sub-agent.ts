import type { ActiveSubAgent, TaskPlan, UnifiedBlock } from "../../store-types";
import type { ImmerSet } from "../types";
import type { WorkflowStoreDraft } from "./types";

export function syncSubAgentToTimeline(
  timeline: UnifiedBlock[],
  parentRequestId: string,
  agent: ActiveSubAgent
): void {
  const block = timeline.find(
    (b) => b.type === "sub_agent_activity" && b.data.parentRequestId === parentRequestId
  );
  if (block && block.type === "sub_agent_activity") {
    block.data = { ...agent };
  }
}

function isBackgroundedToolResult(result: unknown): boolean {
  return (
    result != null &&
    typeof result === "object" &&
    (result as { status?: unknown }).status === "backgrounded"
  );
}

function backgroundJobIdFromResult(result: unknown): string | null {
  if (result == null || typeof result !== "object") return null;
  const jobId = (result as { job_id?: unknown }).job_id;
  return typeof jobId === "string" ? jobId : null;
}

export function createSubAgentActions(set: ImmerSet<WorkflowStoreDraft>) {
  return {
    startPromptGeneration: (
      sessionId: string,
      agentId: string,
      parentRequestId: string,
      data: { architectSystemPrompt: string; architectUserMessage: string }
    ) =>
      set((state) => {
        if (!state.activeSubAgents[sessionId]) {
          state.activeSubAgents[sessionId] = [];
        }
        const now = new Date().toISOString();
        const existing = state.activeSubAgents[sessionId].find(
          (a) => a.parentRequestId === parentRequestId
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
        const timeline = state.timelines[sessionId];
        const blockId = `sub-agent-${parentRequestId}`;
        const agentData = state.activeSubAgents[sessionId].find(
          (a) => a.parentRequestId === parentRequestId
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

    completePromptGeneration: (
      sessionId: string,
      _agentId: string,
      parentRequestId: string,
      data: { generatedPrompt?: string; success: boolean; durationMs: number }
    ) =>
      set((state) => {
        const agents = state.activeSubAgents[sessionId];
        if (!agents) return;
        const agent = agents.find((a) => a.parentRequestId === parentRequestId);
        if (agent?.promptGeneration) {
          agent.promptGeneration.status = data.success ? "completed" : "failed";
          agent.promptGeneration.generatedPrompt = data.generatedPrompt;
          agent.promptGeneration.durationMs = data.durationMs;
        }
        const timeline = state.timelines[sessionId];
        if (timeline && agent) {
          const block = timeline.find(
            (b) => b.type === "sub_agent_activity" && b.data.parentRequestId === parentRequestId
          );
          if (block && block.type === "sub_agent_activity") {
            block.data = { ...agent };
          }
        }
      }),

    startSubAgent: (
      sessionId: string,
      agent: {
        agentId: string;
        agentName: string;
        parentRequestId: string;
        task: string;
        depth: number;
      }
    ) =>
      set((state) => {
        if (!state.activeSubAgents[sessionId]) {
          state.activeSubAgents[sessionId] = [];
        }
        const now = new Date().toISOString();
        const existing = agent.parentRequestId
          ? state.activeSubAgents[sessionId].find(
              (a) => a.parentRequestId === agent.parentRequestId
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
        const timeline = state.timelines[sessionId];
        const agentData = state.activeSubAgents[sessionId].find(
          (a) => a.parentRequestId === agent.parentRequestId
        );
        if (!agentData) return;

        const blockId = `sub-agent-${agent.parentRequestId}`;
        const existingBlock = timeline.find((b) => b.id === blockId);
        if (existingBlock && existingBlock.type === "sub_agent_activity") {
          existingBlock.data = { ...agentData };
        } else if (!existingBlock) {
          const currentAgents = state.activeSubAgents[sessionId];
          const anyRunning = currentAgents.some(
            (a) => a.status === "running" && a.parentRequestId !== agent.parentRequestId
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

    addSubAgentToolCall: (
      sessionId: string,
      parentRequestId: string,
      toolCall: { id: string; name: string; args: Record<string, unknown> }
    ) =>
      set((state) => {
        const agents = state.activeSubAgents[sessionId];
        if (!agents) return;
        const agent = agents.find((a) => a.parentRequestId === parentRequestId);
        if (agent) {
          if (agent.toolCalls.some((tc) => tc.id === toolCall.id)) return;
          agent.toolCalls.push({
            ...toolCall,
            status: "running",
            startedAt: new Date().toISOString(),
          });
          agent.entries.push({ kind: "tool_call", toolCallId: toolCall.id });
        }
        const timeline = state.timelines[sessionId];
        if (timeline && agent) {
          syncSubAgentToTimeline(timeline, parentRequestId, agent);
        }
      }),

    completeSubAgentToolCall: (
      sessionId: string,
      parentRequestId: string,
      toolId: string,
      success: boolean,
      result?: unknown
    ) =>
      set((state) => {
        const agents = state.activeSubAgents[sessionId];
        if (!agents) return;
        const agent = agents.find((a) => a.parentRequestId === parentRequestId);
        if (agent) {
          const tool = agent.toolCalls.find((t) => t.id === toolId);
          if (tool) {
            tool.status = isBackgroundedToolResult(result)
              ? "backgrounded"
              : success
                ? "completed"
                : "error";
            tool.result = result;
            if (tool.status === "backgrounded") {
              delete tool.completedAt;
            } else {
              tool.completedAt = new Date().toISOString();
            }
          }
        }
        const timeline = state.timelines[sessionId];
        if (timeline && agent) {
          syncSubAgentToTimeline(timeline, parentRequestId, agent);
        }
      }),

    completeBackgroundedSubAgentToolCall: (
      sessionId: string,
      jobId: string,
      success: boolean,
      result?: unknown
    ) =>
      set((state) => {
        const agents = state.activeSubAgents[sessionId];
        if (!agents) return;
        for (const agent of agents) {
          const tool = agent.toolCalls.find((candidate) => {
            if ((candidate.status as string) !== "backgrounded") return false;
            return backgroundJobIdFromResult(candidate.result) === jobId;
          });
          if (!tool) continue;

          tool.status = success ? "completed" : "error";
          tool.result = result;
          tool.completedAt = new Date().toISOString();

          const timeline = state.timelines[sessionId];
          if (timeline) {
            syncSubAgentToTimeline(timeline, agent.parentRequestId, agent);
          }
          return;
        }
      }),

    completeSubAgent: (
      sessionId: string,
      parentRequestId: string,
      result: { response: string; durationMs: number }
    ) =>
      set((state) => {
        const agents = state.activeSubAgents[sessionId];
        if (!agents) return;
        const agent = agents.find((a) => a.parentRequestId === parentRequestId);
        if (agent) {
          agent.status = "completed";
          agent.response = result.response;
          agent.durationMs = result.durationMs;
          agent.completedAt = new Date().toISOString();
        }
        const timeline = state.timelines[sessionId];
        if (timeline && agent) {
          syncSubAgentToTimeline(timeline, parentRequestId, agent);
        }
      }),

    failSubAgent: (sessionId: string, parentRequestId: string, error: string) =>
      set((state) => {
        const agents = state.activeSubAgents[sessionId];
        if (!agents) return;
        const agent = agents.find((a) => a.parentRequestId === parentRequestId);
        if (agent) {
          agent.status = "error";
          agent.error = error;
          agent.completedAt = new Date().toISOString();
        }
        const timeline = state.timelines[sessionId];
        if (timeline && agent) {
          syncSubAgentToTimeline(timeline, parentRequestId, agent);
        }
      }),

    updateSubAgentStreamingText: (sessionId: string, parentRequestId: string, text: string) =>
      set((state) => {
        const agents = state.activeSubAgents[sessionId];
        if (!agents) return;
        const agent = agents.find((a) => a.parentRequestId === parentRequestId);
        if (agent) {
          agent.streamingText = text;
          const lastEntry = agent.entries[agent.entries.length - 1];
          if (lastEntry && lastEntry.kind === "text") {
            lastEntry.text = text;
          } else {
            agent.entries.push({ kind: "text", text });
          }
        }
        const timeline = state.timelines[sessionId];
        if (timeline && agent) {
          syncSubAgentToTimeline(timeline, parentRequestId, agent);
        }
      }),

    updateSubAgentThinking: (sessionId: string, parentRequestId: string, text: string) =>
      set((state) => {
        const agents = state.activeSubAgents[sessionId];
        if (!agents) return;
        const agent = agents.find((a) => a.parentRequestId === parentRequestId);
        if (agent) {
          const now = Date.now();
          agent.thinking = text;
          agent.thinkingStartedAt ??= now;
          agent.thinkingEndedAt = now;
          const lastEntry = agent.entries[agent.entries.length - 1];
          if (lastEntry?.kind === "thinking") {
            lastEntry.text = text;
            lastEntry.startedAt ??= now;
            lastEntry.endedAt = now;
          } else {
            agent.entries.push({
              kind: "thinking",
              text,
              startedAt: now,
              endedAt: now,
            });
          }
        }
        const timeline = state.timelines[sessionId];
        if (timeline && agent) {
          syncSubAgentToTimeline(timeline, parentRequestId, agent);
        }
      }),

    appendSubAgentToolOutput: (sessionId: string, toolId: string, chunk: string) =>
      set((state) => {
        const agents = state.activeSubAgents[sessionId];
        if (!agents) return;
        for (const agent of agents) {
          const tool = agent.toolCalls.find((t) => t.id === toolId);
          if (tool) {
            tool.streamingOutput = (tool.streamingOutput ?? "") + chunk;
            const timeline = state.timelines[sessionId];
            if (timeline) {
              syncSubAgentToTimeline(timeline, agent.parentRequestId, agent);
            }
            return;
          }
        }
      }),

    clearActiveSubAgents: (sessionId: string) =>
      set((state) => {
        state.activeSubAgents[sessionId] = [];
      }),
  };
}
