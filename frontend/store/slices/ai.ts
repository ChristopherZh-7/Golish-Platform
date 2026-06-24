/**
 * AI slice for the Zustand store.
 *
 * Owns the agent's runtime state: streaming buffers, thinking content, the
 * global / per-session AI provider config, the live tool-call list, and the
 * `ai_tool_execution` timeline-block helpers.
 */

import type {
  ActiveToolCall,
  AgentMessage,
  AiConfig,
  AiStatus,
  AiToolExecution,
  Session,
  StreamingBlock,
  TaskPlan,
  ToolCall,
  ToolCallSource,
  UnifiedBlock,
} from "../store-types";
import type { SliceCreator } from "./types";

/**
 * Cross-slice fields accessed by AI actions during Immer mutations.
 */
interface AiStoreDraft extends AiState {
  sessions: Record<string, Session>;
  timelines: Record<string, UnifiedBlock[]>;
  streamingBlocks: Record<string, StreamingBlock[]>;
  streamingTextOffset: Record<string, number>;
  sessionTokenUsage?: Record<string, { input: number; output: number }>;
}

function inferToolIntent(
  requestId: string,
  toolName: string,
  args: Record<string, unknown>
): AiToolExecution["toolIntent"] {
  const isRecoveredTextual = requestId.startsWith("textual-tool-call-");
  if (!isRecoveredTextual) {
    return {
      modelWanted: toolName,
      source: "native_tool_call",
      decision: "allow",
    };
  }

  const action = typeof args.action === "string" ? args.action : null;
  const decision =
    toolName === "ask_human"
      ? "require_human_answer"
      : toolName === "manage_targets" && action === "add"
        ? "require_approval"
        : "allow";

  return {
    modelWanted: toolName,
    source: "textual_xml",
    decision,
    reason:
      decision === "require_human_answer"
        ? "Recovered textual tool intent must become a real human prompt before continuation."
        : decision === "require_approval"
          ? "Recovered textual side-effecting tool intent requires explicit approval."
          : undefined,
  };
}

/** A backgrounded shell/pentest job still running (Cursor-style bottom indicator). */
export interface BackgroundJob {
  /** Job id returned by a `status:"backgrounded"` tool result. */
  jobId: string;
  /** The command that was detached to the background. */
  command: string;
  /** ms epoch when it was detached, for an elapsed-time display. */
  startedAt: number;
}

export interface AiState {
  /** Global (legacy) AI configuration, kept for backwards compatibility. */
  aiConfig: AiConfig;
  /** Currently-selected provider/model from the picker UI. */
  selectedAiModel: { model: string; provider: string } | null;
  /** Buffer of text deltas per session (avoids string concat in hot path). */
  agentStreamingBuffer: Record<string, string[]>;
  /** Cached joined streaming text per session. */
  agentStreaming: Record<string, string>;
  /** Whether the agent has completed its initial handshake for a session. */
  agentInitialized: Record<string, boolean>;
  /** True while waiting for first content from the agent. */
  isAgentThinking: Record<string, boolean>;
  /** True while the agent is actively responding (started → completed). */
  isAgentResponding: Record<string, boolean>;
  /** Accumulated extended-thinking content per session. */
  thinkingContent: Record<string, string>;
  /** Whether the thinking section is expanded per session. */
  isThinkingExpanded: Record<string, boolean>;
  /** Currently-running tool calls per session. */
  activeToolCalls: Record<string, ActiveToolCall[]>;
  /** Per-session set of request IDs already processed (dedup). */
  processedToolRequests: Record<string, Set<string>>;
  /** Tool intent observations that arrived before the timeline card exists. */
  pendingToolIntentObservations: Record<
    string,
    Record<string, NonNullable<AiToolExecution["toolIntent"]>>
  >;
  /** Backgrounded shell/pentest jobs still running per session (Cursor-style). */
  backgroundJobs: Record<string, BackgroundJob[]>;
}

export interface AiActions {
  setAiConfig: (config: Partial<AiConfig>) => void;
  setSelectedAiModel: (model: { model: string; provider: string } | null) => void;
  setSessionAiConfig: (sessionId: string, config: Partial<AiConfig>) => void;
  getSessionAiConfig: (sessionId: string) => AiConfig | undefined;

  addAgentMessage: (sessionId: string, message: AgentMessage) => void;
  updateAgentStreaming: (sessionId: string, delta: string) => void;
  getAgentStreamingText: (sessionId: string) => string;
  clearAgentStreaming: (sessionId: string) => void;
  setAgentInitialized: (sessionId: string, initialized: boolean) => void;
  setAgentThinking: (sessionId: string, thinking: boolean) => void;
  setAgentResponding: (sessionId: string, responding: boolean) => void;

  markToolRequestProcessed: (sessionId: string, requestId: string) => void;
  isToolRequestProcessed: (sessionId: string, requestId: string) => boolean;

  updateToolCallStatus: (
    sessionId: string,
    toolId: string,
    status: ToolCall["status"],
    result?: unknown
  ) => void;
  clearAgentMessages: (sessionId: string) => void;
  restoreAgentMessages: (sessionId: string, messages: AgentMessage[]) => void;

  addActiveToolCall: (
    sessionId: string,
    toolCall: {
      id: string;
      name: string;
      args: Record<string, unknown>;
      executedByAgent?: boolean;
      source?: ToolCallSource;
    }
  ) => void;
  completeActiveToolCall: (
    sessionId: string,
    toolId: string,
    success: boolean,
    result?: unknown
  ) => void;
  clearActiveToolCalls: (sessionId: string) => void;

  appendThinkingContent: (sessionId: string, content: string) => void;
  clearThinkingContent: (sessionId: string) => void;
  setThinkingExpanded: (sessionId: string, expanded: boolean) => void;

  addToolExecutionBlock: (
    sessionId: string,
    execution: {
      requestId: string;
      toolName: string;
      args: Record<string, unknown>;
      toolIntent?: AiToolExecution["toolIntent"];
      autoApproved?: boolean;
      riskLevel?: string;
      source?: ToolCallSource;
    }
  ) => void;
  recordToolIntentObservation: (
    sessionId: string,
    observation: NonNullable<AiToolExecution["toolIntent"]> & { requestId: string }
  ) => void;
  completeToolExecutionBlock: (
    sessionId: string,
    requestId: string,
    success: boolean,
    result?: unknown
  ) => void;
  /**
   * Mark a running timeline tool card as interrupted when the conversation ended
   * without a tool result (e.g. an expired/restored tool call). This keeps detail
   * panes from rendering stale "running" state forever.
   */
  interruptToolExecutionBlock: (sessionId: string, requestId: string, result?: unknown) => void;
  /**
   * Mark a timeline tool card as "backgrounded": the command exceeded its soft
   * timeout and was detached to a background job (still running). The card stays
   * non-terminal until a `tool_background_completed` event flips it via
   * {@link completeToolExecutionBlock}.
   */
  backgroundToolExecutionBlock: (sessionId: string, requestId: string, result?: unknown) => void;
  appendToolExecutionOutput: (sessionId: string, requestId: string, chunk: string) => void;
  finalizeRunningToolExecutions: (sessionId: string) => void;

  /** Track a job just detached to the background (Cursor-style indicator). */
  addBackgroundJob: (sessionId: string, job: BackgroundJob) => void;
  /** Drop a background job once it terminates (completed / killed). */
  removeBackgroundJob: (sessionId: string, jobId: string) => void;
}

export interface AiSlice extends AiState, AiActions {}

export const initialAiState: AiState = {
  aiConfig: {
    provider: "",
    model: "",
    status: "disconnected" as AiStatus,
  },
  selectedAiModel: null,
  agentStreamingBuffer: {},
  agentStreaming: {},
  agentInitialized: {},
  isAgentThinking: {},
  isAgentResponding: {},
  thinkingContent: {},
  isThinkingExpanded: {},
  activeToolCalls: {},
  processedToolRequests: {},
  pendingToolIntentObservations: {},
  backgroundJobs: {},
};

export const createAiSlice: SliceCreator<AiSlice, AiStoreDraft> = (set, get) => ({
  ...initialAiState,

  setAiConfig: (config) =>
    set((state) => {
      state.aiConfig = { ...state.aiConfig, ...config };
    }),

  setSelectedAiModel: (model) =>
    set((state) => {
      state.selectedAiModel = model;
    }),

  setSessionAiConfig: (sessionId, config) =>
    set((state) => {
      if (state.sessions?.[sessionId]) {
        const currentConfig = state.sessions[sessionId].aiConfig || {
          provider: "",
          model: "",
          status: "disconnected" as AiStatus,
        };
        state.sessions[sessionId].aiConfig = { ...currentConfig, ...config };
      }
    }),

  getSessionAiConfig: (sessionId) => {
    const session = get().sessions?.[sessionId];
    return session?.aiConfig;
  },

  addAgentMessage: (sessionId, message) =>
    set((state) => {
      if (!message.workingDirectory) {
        const session = state.sessions?.[sessionId];
        if (session?.workingDirectory) {
          message.workingDirectory = session.workingDirectory;
        }
      }

      if (!state.timelines[sessionId]) {
        state.timelines[sessionId] = [];
      }
      state.timelines[sessionId].push({
        id: message.id,
        type: "agent_message",
        timestamp: message.timestamp,
        data: message,
      });

      if (message.inputTokens || message.outputTokens) {
        const current = state.sessionTokenUsage?.[sessionId] ?? { input: 0, output: 0 };
        if (!state.sessionTokenUsage) state.sessionTokenUsage = {};
        state.sessionTokenUsage[sessionId] = {
          input: current.input + (message.inputTokens ?? 0),
          output: current.output + (message.outputTokens ?? 0),
        };
      }
    }),

  updateAgentStreaming: (sessionId, delta) =>
    set((state) => {
      if (!state.agentStreamingBuffer[sessionId]) {
        state.agentStreamingBuffer[sessionId] = [];
      }
      state.agentStreamingBuffer[sessionId].push(delta);
      state.agentStreaming[sessionId] = state.agentStreamingBuffer[sessionId].join("");

      if (!state.streamingBlocks[sessionId]) {
        state.streamingBlocks[sessionId] = [];
      }
      const blocks = state.streamingBlocks[sessionId];

      const lastBlock = blocks[blocks.length - 1];
      if (lastBlock && lastBlock.type === "text") {
        lastBlock.content += delta;
      } else if (delta) {
        blocks.push({ type: "text", content: delta });
      }
    }),

  getAgentStreamingText: (sessionId) => {
    const state = get();
    const buffer = state.agentStreamingBuffer[sessionId];
    if (!buffer || buffer.length === 0) return "";
    return state.agentStreaming[sessionId] ?? "";
  },

  clearAgentStreaming: (sessionId) =>
    set((state) => {
      state.agentStreamingBuffer[sessionId] = [];
      state.agentStreaming[sessionId] = "";
      state.streamingBlocks[sessionId] = [];
      state.streamingTextOffset[sessionId] = 0;
    }),

  setAgentInitialized: (sessionId, initialized) =>
    set((state) => {
      state.agentInitialized[sessionId] = initialized;
    }),

  setAgentThinking: (sessionId, thinking) =>
    set((state) => {
      state.isAgentThinking[sessionId] = thinking;
    }),

  setAgentResponding: (sessionId, responding) =>
    set((state) => {
      state.isAgentResponding[sessionId] = responding;
    }),

  addBackgroundJob: (sessionId, job) =>
    set((state) => {
      const list = state.backgroundJobs[sessionId] ?? [];
      if (!list.some((j) => j.jobId === job.jobId)) {
        state.backgroundJobs[sessionId] = [...list, job];
      }
    }),

  removeBackgroundJob: (sessionId, jobId) =>
    set((state) => {
      const list = state.backgroundJobs[sessionId];
      if (list) {
        state.backgroundJobs[sessionId] = list.filter((j) => j.jobId !== jobId);
      }
    }),

  markToolRequestProcessed: (sessionId, requestId) =>
    set((state) => {
      if (!state.processedToolRequests[sessionId]) {
        state.processedToolRequests[sessionId] = new Set<string>();
      }
      state.processedToolRequests[sessionId].add(requestId);
    }),

  isToolRequestProcessed: (sessionId, requestId) => {
    return get().processedToolRequests[sessionId]?.has(requestId) ?? false;
  },

  updateToolCallStatus: (sessionId, toolId, status, result) =>
    set((state) => {
      const timeline = state.timelines[sessionId];
      if (timeline) {
        for (const block of timeline) {
          if (block.type === "agent_message") {
            const tool = block.data.toolCalls?.find((t) => t.id === toolId);
            if (tool) {
              tool.status = status;
              if (result !== undefined) tool.result = result;
              return;
            }
          }
        }
      }
    }),

  clearAgentMessages: (sessionId) =>
    set((state) => {
      const timeline = state.timelines[sessionId];
      if (timeline) {
        state.timelines[sessionId] = timeline.filter((block) => block.type !== "agent_message");
      }
      state.agentStreamingBuffer[sessionId] = [];
      state.agentStreaming[sessionId] = "";
    }),

  restoreAgentMessages: (sessionId, messages) =>
    set((state) => {
      state.agentStreamingBuffer[sessionId] = [];
      state.agentStreaming[sessionId] = "";
      state.timelines[sessionId] = [];
      for (const message of messages) {
        state.timelines[sessionId].push({
          id: message.id,
          type: "agent_message",
          timestamp: message.timestamp,
          data: message,
        });
      }
    }),

  addActiveToolCall: (sessionId, toolCall) =>
    set((state) => {
      if (!state.activeToolCalls[sessionId]) {
        state.activeToolCalls[sessionId] = [];
      }
      state.activeToolCalls[sessionId].push({
        ...toolCall,
        status: "running",
        startedAt: new Date().toISOString(),
      });
    }),

  completeActiveToolCall: (sessionId, toolId, success, result) =>
    set((state) => {
      const tools = state.activeToolCalls[sessionId];
      if (tools) {
        const tool = tools.find((t) => t.id === toolId);
        if (tool) {
          tool.status = success ? "completed" : "error";
          tool.result = result;
          tool.completedAt = new Date().toISOString();
        }
      }
    }),

  clearActiveToolCalls: (sessionId) =>
    set((state) => {
      state.activeToolCalls[sessionId] = [];
    }),

  appendThinkingContent: (sessionId, content) =>
    set((state) => {
      if (!state.thinkingContent[sessionId]) {
        state.thinkingContent[sessionId] = "";
      }
      state.thinkingContent[sessionId] += content;

      if (!state.streamingBlocks[sessionId]) {
        state.streamingBlocks[sessionId] = [];
      }
      const blocks = state.streamingBlocks[sessionId];
      const lastBlock = blocks[blocks.length - 1];

      if (lastBlock && lastBlock.type === "thinking") {
        lastBlock.content += content;
      } else {
        state.streamingBlocks[sessionId] = blocks.filter((b) => b.type !== "thinking");
        state.streamingBlocks[sessionId].push({ type: "thinking", content });
      }
    }),

  clearThinkingContent: (sessionId) =>
    set((state) => {
      state.thinkingContent[sessionId] = "";
    }),

  setThinkingExpanded: (sessionId, expanded) =>
    set((state) => {
      state.isThinkingExpanded[sessionId] = expanded;
    }),

  addToolExecutionBlock: (sessionId, execution) =>
    set((state) => {
      if (!state.timelines[sessionId]) {
        state.timelines[sessionId] = [];
      }
      const timeline = state.timelines[sessionId];
      const observedIntent = state.pendingToolIntentObservations[sessionId]?.[execution.requestId];
      const exists = timeline.some(
        (b) => b.type === "ai_tool_execution" && b.data.requestId === execution.requestId
      );
      if (exists) {
        if (observedIntent) {
          const block = timeline.find(
            (b) => b.type === "ai_tool_execution" && b.data.requestId === execution.requestId
          );
          if (block?.type === "ai_tool_execution") block.data.toolIntent = observedIntent;
        }
        return;
      }

      let planStepIndex: number | undefined;
      let planStepId: string | undefined;
      const plan = state.sessions?.[sessionId]?.plan as TaskPlan | undefined;
      if (plan) {
        let idx = plan.steps.findIndex((s) => s.status === "in_progress");
        if (idx < 0) {
          for (let i = plan.steps.length - 1; i >= 0; i--) {
            if (plan.steps[i].status === "completed") {
              idx = i;
              break;
            }
          }
        }
        if (idx >= 0) {
          planStepIndex = idx;
          planStepId = plan.steps[idx].id ?? undefined;
        }
      }
      timeline.push({
        id: `tool-exec-${execution.requestId}`,
        type: "ai_tool_execution",
        timestamp: new Date().toISOString(),
        data: {
          requestId: execution.requestId,
          toolName: execution.toolName,
          args: execution.args,
          toolIntent:
            observedIntent ??
            execution.toolIntent ??
            inferToolIntent(execution.requestId, execution.toolName, execution.args),
          status: "running",
          startedAt: new Date().toISOString(),
          autoApproved: execution.autoApproved,
          riskLevel: execution.riskLevel,
          source: execution.source,
          planStepIndex,
          planStepId,
        },
      });
    }),

  recordToolIntentObservation: (sessionId, observation) =>
    set((state) => {
      if (!state.pendingToolIntentObservations[sessionId]) {
        state.pendingToolIntentObservations[sessionId] = {};
      }
      state.pendingToolIntentObservations[sessionId][observation.requestId] = {
        modelWanted: observation.modelWanted,
        source: observation.source,
        decision: observation.decision,
        reason: observation.reason,
        rawPreview: observation.rawPreview,
      };

      const timeline = state.timelines[sessionId];
      if (!timeline) return;
      const block = timeline.find(
        (b) => b.type === "ai_tool_execution" && b.data.requestId === observation.requestId
      );
      if (block?.type === "ai_tool_execution") {
        block.data.toolIntent =
          state.pendingToolIntentObservations[sessionId][observation.requestId];
      }
    }),

  completeToolExecutionBlock: (sessionId, requestId, success, result) =>
    set((state) => {
      const timeline = state.timelines[sessionId];
      if (!timeline) return;
      const block = timeline.find(
        (b) => b.type === "ai_tool_execution" && b.data.requestId === requestId
      );
      if (block && block.type === "ai_tool_execution") {
        block.data.status = success ? "completed" : "error";
        block.data.result = result;
        block.data.completedAt = new Date().toISOString();
        const start = new Date(block.data.startedAt).getTime();
        block.data.durationMs = Date.now() - start;
      }
    }),

  interruptToolExecutionBlock: (sessionId, requestId, result) =>
    set((state) => {
      const timeline = state.timelines[sessionId];
      if (!timeline) return;
      const block = timeline.find(
        (b) => b.type === "ai_tool_execution" && b.data.requestId === requestId
      );
      if (block && block.type === "ai_tool_execution" && block.data.status === "running") {
        block.data.status = "interrupted";
        block.data.result = result;
        block.data.completedAt = new Date().toISOString();
        const start = new Date(block.data.startedAt).getTime();
        block.data.durationMs = Date.now() - start;
      }
    }),

  backgroundToolExecutionBlock: (sessionId, requestId, result) =>
    set((state) => {
      const timeline = state.timelines[sessionId];
      if (!timeline) return;
      const block = timeline.find(
        (b) => b.type === "ai_tool_execution" && b.data.requestId === requestId
      );
      if (block && block.type === "ai_tool_execution") {
        block.data.status = "backgrounded";
        block.data.result = result;
      }
    }),

  appendToolExecutionOutput: (sessionId, requestId, chunk) =>
    set((state) => {
      const timeline = state.timelines[sessionId];
      if (!timeline) return;
      const block = timeline.find(
        (b) => b.type === "ai_tool_execution" && b.data.requestId === requestId
      );
      if (block && block.type === "ai_tool_execution") {
        block.data.streamingOutput = (block.data.streamingOutput || "") + chunk;
      }
    }),

  finalizeRunningToolExecutions: (sessionId) =>
    set((state) => {
      const timeline = state.timelines[sessionId];
      if (!timeline) return;
      for (const block of timeline) {
        if (block.type === "ai_tool_execution" && block.data.status === "running") {
          block.data.status = "completed";
          block.data.completedAt = new Date().toISOString();
          const start = new Date(block.data.startedAt).getTime();
          block.data.durationMs = Date.now() - start;
        }
      }
    }),
});

export const selectAiConfig = <T extends AiState>(state: T): AiConfig => state.aiConfig;

export const selectActiveToolCalls = <T extends AiState>(
  state: T,
  sessionId: string
): ActiveToolCall[] => state.activeToolCalls[sessionId] ?? [];

export const selectIsAgentThinking = <T extends AiState>(state: T, sessionId: string): boolean =>
  state.isAgentThinking[sessionId] ?? false;

export const selectIsAgentResponding = <T extends AiState>(state: T, sessionId: string): boolean =>
  state.isAgentResponding[sessionId] ?? false;
