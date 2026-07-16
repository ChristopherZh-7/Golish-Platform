import type { ActiveSubAgent, SubAgentEntry, TaskPlan, UnifiedBlock } from "../../store-types";
import { appendLiveToolOutput } from "../live-output";
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

function lastToolCallEntryIndex(entries: SubAgentEntry[]): number {
  for (let i = entries.length - 1; i >= 0; i--) {
    if (entries[i].kind === "tool_call") return i;
  }
  return -1;
}

function latestEntryIndexSinceLastToolCall(
  entries: SubAgentEntry[],
  kind: "text" | "thinking",
  attemptEntryStart = 0
): number {
  const floor = Math.max(lastToolCallEntryIndex(entries), attemptEntryStart - 1);
  for (let i = entries.length - 1; i > floor; i--) {
    if (entries[i].kind === kind) return i;
  }
  return -1;
}

function updateAccumulatedEntry(
  entries: SubAgentEntry[],
  kind: "text" | "thinking",
  text: string,
  fallbackEntry: SubAgentEntry,
  attemptEntryStart?: number
): boolean {
  const idx = latestEntryIndexSinceLastToolCall(entries, kind, attemptEntryStart);
  if (idx < 0) return false;

  const existing = entries[idx].text ?? "";
  if (!existing || text.startsWith(existing)) {
    entries[idx].text = text;
    if (kind === "thinking") {
      entries[idx].startedAt ??= fallbackEntry.startedAt;
      entries[idx].endedAt = fallbackEntry.endedAt;
    }
    return true;
  }

  // Streaming batches can occasionally flush out of order; never let a shorter
  // accumulated frame regress the detail view.
  if (existing.startsWith(text)) return true;

  return false;
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
          const isResume = existing.status !== "running";
          existing.agentId = agent.agentId;
          existing.agentName = agent.agentName;
          existing.task = agent.task;
          existing.depth = agent.depth;
          // Durable stage workers resume with the same parent request identity.
          // A restored timeline deliberately projects an in-flight worker as
          // interrupted, so the next authoritative started event must revive
          // that exact card without discarding its checkpointed history.
          existing.status = "running";
          if (isResume) {
            existing.attemptEntryStart = existing.entries.length;
            delete existing.error;
            delete existing.response;
            delete existing.completedAt;
            delete existing.durationMs;
            delete existing.streamingText;
            delete existing.thinking;
            delete existing.thinkingStartedAt;
            delete existing.thinkingEndedAt;
          }
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
            if (candidate.status !== "backgrounded" && candidate.status !== "interrupted") {
              return false;
            }
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
          if (
            !updateAccumulatedEntry(
              agent.entries,
              "text",
              text,
              { kind: "text", text },
              agent.attemptEntryStart
            )
          ) {
            agent.entries.push({ kind: "text", text });
          }
        }
        const timeline = state.timelines[sessionId];
        if (timeline && agent) {
          syncSubAgentToTimeline(timeline, parentRequestId, agent);
        }
      }),

    updateSubAgentThinking: (
      sessionId: string,
      parentRequestId: string,
      text: string,
      timing?: { startedAt: number; endedAt: number }
    ) =>
      set((state) => {
        const agents = state.activeSubAgents[sessionId];
        if (!agents) return;
        const agent = agents.find((a) => a.parentRequestId === parentRequestId);
        if (agent) {
          const endedAt = timing?.endedAt ?? Date.now();
          const startedAt = timing?.startedAt ?? endedAt;
          agent.thinking = text;
          agent.thinkingStartedAt ??= startedAt;
          agent.thinkingEndedAt = endedAt;
          if (
            !updateAccumulatedEntry(
              agent.entries,
              "thinking",
              text,
              {
                kind: "thinking",
                text,
                startedAt,
                endedAt,
              },
              agent.attemptEntryStart
            )
          ) {
            agent.entries.push({
              kind: "thinking",
              text,
              startedAt,
              endedAt,
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
            tool.streamingOutput = appendLiveToolOutput(tool.streamingOutput, chunk);
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
