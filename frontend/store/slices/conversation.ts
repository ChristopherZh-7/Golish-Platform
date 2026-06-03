/**
 * Conversation slice for the Zustand store.
 *
 * Manages right-side AI chat conversations and their association with
 * left-side terminal tabs. Each conversation "owns" a group of terminals.
 */

import type { SliceCreator } from "./types";

/**
 * Cross-slice fields accessed by conversation actions when switching
 * the active conversation also switches the active terminal tab.
 */
interface ConversationStoreDraft extends ConversationState {
  activeSessionId?: string | null;
  tabActivationHistory?: string[];
}

export interface ChatToolCall {
  name: string;
  args: string;
  result?: string;
  success?: boolean;
  requestId?: string;
}

/**
 * A single contiguous burst of model reasoning. A new segment is opened
 * whenever reasoning resumes after the model emitted answer text or made a
 * tool call, so the chat can interleave thinking with content/tools in time
 * order (instead of merging every reasoning chunk into one block at the top).
 *
 * Runtime-only: this is derived during streaming and is NOT persisted to the
 * DB. Restored history falls back to the merged `thinking` string.
 */
export interface ThinkingSegment {
  content: string;
  /** Epoch ms when this segment's first reasoning chunk arrived. */
  startedAt: number;
  /** Epoch ms when this segment's last reasoning chunk arrived. */
  endedAt: number;
  /** `content.length` when this segment started (interleave anchor). */
  contentOffset: number;
  /** `toolCalls.length` when this segment started (interleave anchor). */
  toolIndex: number;
}

/**
 * A staged-orchestration boundary surfaced inline in the chat (task mode).
 * Renders as a divider between stage narrations so consecutive stages don't
 * read as one continuous monologue and the user can see the runtime advancing.
 *
 * Two completion granularities are intentionally distinct (rendered with
 * different prominence by `StageMarker`):
 *  - `subtask_completed` = a single *step* inside a stage finished (subdued).
 *  - `stage_completed`    = a whole harness *stage* passed its gate (prominent),
 *    surfaced when `submit_stage_deliverable` is accepted.
 *
 * Stored on a `role: "system"` message; `message.content` mirrors `label` so a
 * restored history message (which loses the structured field) still renders.
 */
export interface StageEvent {
  kind: "subtask_completed" | "stage_completed" | "task_progress" | "task_resumed";
  /** Short headline shown on the divider (also persisted via message.content). */
  label: string;
  /** Subtask/step title (for subtask_completed) or stage name (stage_completed). */
  title?: string;
  /** task_progress status (e.g. "finished", "waiting_approval"). */
  status?: string;
  /** Optional collapsible detail (e.g. a truncated stage result). */
  detail?: string;
}

export interface ChatMessage {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  timestamp: number;
  isStreaming?: boolean;
  error?: string;
  /**
   * Visual severity of `error`. `warning` (amber) for soft/recoverable
   * conditions surfaced through the error channel (e.g. the planner replying in
   * prose instead of a plan); defaults to `error` (red) when absent. Runtime-only.
   */
  errorSeverity?: "error" | "warning";
  toolCalls?: ChatToolCall[];
  /** Present on `role: "system"` divider messages (task-mode stage boundaries). */
  stageEvent?: StageEvent;
  thinking?: string;
  /**
   * Time-ordered reasoning segments for interleaved rendering. Runtime-only
   * (not persisted); when absent (e.g. restored history) the UI falls back to
   * the merged `thinking` string rendered as a single top block.
   */
  thinkingSegments?: ThinkingSegment[];
  /** Epoch ms when the first thinking chunk arrived (set lazily by the streaming sync layer). */
  thinkingStartedAt?: number;
  /** Epoch ms when the last thinking chunk arrived. */
  thinkingEndedAt?: number;
  /** Content offset when the first tool call was added (for interleaved rendering) */
  toolCallsContentOffset?: number;
  /** Content offset at which each toolCalls[i] was inserted (for per-call interleaving) */
  toolCallOffsets?: number[];
}

export interface ChatConversation {
  id: string;
  title: string;
  messages: ChatMessage[];
  createdAt: number;
  aiSessionId: string;
  aiInitialized: boolean;
  isStreaming: boolean;
  /**
   * Epoch ms when the current streaming turn started (set on the false→true
   * streaming edge). Anchors the "preparing" elapsed counter so it survives the
   * indicator remounting (e.g. switching conversation tabs). Runtime-only — not
   * persisted to the conversation DB.
   */
  streamingStartedAt?: number;
}

// State interface
export interface ConversationState {
  conversations: Record<string, ChatConversation>;
  activeConversationId: string | null;
  conversationOrder: string[];
  /** Maps conversation ID to the terminal tab IDs it owns */
  conversationTerminals: Record<string, string[]>;
}

// Actions interface
export interface ConversationActions {
  addConversation: (conv: ChatConversation) => void;
  removeConversation: (convId: string) => void;
  setActiveConversation: (convId: string) => void;
  updateConversation: (convId: string, update: Partial<ChatConversation>) => void;
  updateConversationMessages: (convId: string, messages: ChatMessage[]) => void;
  setConversationStreaming: (convId: string, streaming: boolean) => void;
  addConversationMessage: (convId: string, message: ChatMessage) => void;
  /** Append a task-mode stage-boundary divider (role: "system") to the chat */
  addConversationStageMarker: (convId: string, marker: StageEvent) => void;
  /** Append text delta to the last streaming assistant message */
  appendMessageDelta: (convId: string, delta: string) => void;
  /** Append thinking content to the last streaming assistant message */
  appendMessageThinking: (convId: string, content: string) => void;
  /** Add a tool call to the last assistant message */
  addMessageToolCall: (convId: string, toolCall: ChatToolCall) => void;
  /** Update a tool call result on the last assistant message */
  updateMessageToolResult: (
    convId: string,
    toolName: string,
    result: string,
    success: boolean
  ) => void;
  /** Finalize the last streaming message */
  finalizeStreamingMessage: (convId: string, response?: string, reasoning?: string) => void;
  /** Set error on the last streaming message or add an error message */
  setMessageError: (convId: string, errorMsg: string, severity?: "error" | "warning") => void;
  addTerminalToConversation: (convId: string, terminalId: string) => void;
  removeTerminalFromConversation: (convId: string, terminalId: string) => void;
  /** Get all terminal IDs belonging to the active conversation */
  getActiveConversationTerminals: () => string[];
  /** Find which conversation a terminal belongs to */
  getConversationForTerminal: (terminalId: string) => string | null;
  /** Find conversation by AI session ID */
  getConversationBySessionId: (sessionId: string) => ChatConversation | null;
  /** Bulk-restore conversations from persisted state (replaces existing) */
  restoreConversations: (
    convs: ChatConversation[],
    order: string[],
    activeId: string | null
  ) => void;
}

// Combined slice interface
export interface ConversationSlice extends ConversationState, ConversationActions {}

// Initial state
export const initialConversationState: ConversationState = {
  conversations: {},
  activeConversationId: null,
  conversationOrder: [],
  conversationTerminals: {},
};

let _convCounter = 0;

/**
 * Whether two surfaced error strings describe the same underlying failure. One
 * containing the other catches the common case where the invoke rejection wraps
 * the backend error message with an `[API trace=…] <command>:` prefix.
 */
function isSameError(a: string, b: string): boolean {
  const x = a.trim();
  const y = b.trim();
  return x.length > 0 && y.length > 0 && (x.includes(y) || y.includes(x));
}

export function createNewConversation(): ChatConversation {
  _convCounter += 1;
  const id = `pentest-chat-${Date.now()}-${_convCounter}`;
  return {
    id,
    title: "New Chat",
    messages: [],
    createdAt: Date.now(),
    aiSessionId: id,
    aiInitialized: false,
    isStreaming: false,
  };
}

/**
 * Creates the conversation slice.
 */
export const createConversationSlice: SliceCreator<ConversationSlice, ConversationStoreDraft> = (
  set,
  get
) => ({
  ...initialConversationState,

  addConversation: (conv) =>
    set((state) => {
      state.conversations[conv.id] = conv;
      state.conversationOrder.push(conv.id);
      state.activeConversationId = conv.id;
      state.conversationTerminals[conv.id] = [];
    }),

  removeConversation: (convId) =>
    set((state) => {
      delete state.conversations[convId];
      delete state.conversationTerminals[convId];
      const orderIdx = state.conversationOrder.indexOf(convId);
      if (orderIdx !== -1) {
        state.conversationOrder.splice(orderIdx, 1);
      }
      if (state.activeConversationId === convId) {
        const remaining = state.conversationOrder;
        state.activeConversationId = remaining.length > 0 ? remaining[remaining.length - 1] : null;
      }
    }),

  setActiveConversation: (convId) =>
    set((state) => {
      if (state.conversations[convId]) {
        state.activeConversationId = convId;
        // 1:1 sync: switch center timeline to this conversation's terminal
        const terminals = state.conversationTerminals[convId];
        if (terminals && terminals.length > 0 && state.activeSessionId !== terminals[0]) {
          state.activeSessionId = terminals[0];
          // Update activation history
          if (state.tabActivationHistory) {
            state.tabActivationHistory = state.tabActivationHistory.filter(
              (id: string) => id !== terminals[0]
            );
            state.tabActivationHistory.push(terminals[0]);
          }
        }
      }
    }),

  updateConversation: (convId, update) =>
    set((state) => {
      const conv = state.conversations[convId];
      if (conv) {
        Object.assign(conv, update);
      }
    }),

  updateConversationMessages: (convId, messages) =>
    set((state) => {
      const conv = state.conversations[convId];
      if (conv) {
        conv.messages = messages;
      }
    }),

  setConversationStreaming: (convId, streaming) =>
    set((state) => {
      const conv = state.conversations[convId];
      if (conv) {
        // Anchor the turn start on the false→true edge (and clear on stop) so
        // the preparing-timer reads a stable origin across remounts instead of
        // resetting whenever the indicator re-mounts.
        if (streaming && !conv.isStreaming) {
          conv.streamingStartedAt = Date.now();
        } else if (!streaming) {
          conv.streamingStartedAt = undefined;
        }
        conv.isStreaming = streaming;
      }
    }),

  addConversationMessage: (convId, message) =>
    set((state) => {
      const conv = state.conversations[convId];
      if (conv) {
        conv.messages.push(message);
      }
    }),

  addConversationStageMarker: (convId, marker) =>
    set((state) => {
      const conv = state.conversations[convId];
      if (!conv) return;
      // De-dupe: skip if the previous message is an identical stage marker
      // (defensive against duplicate backend emissions).
      const prev = conv.messages[conv.messages.length - 1];
      if (
        prev?.role === "system" &&
        prev.stageEvent?.kind === marker.kind &&
        prev.stageEvent?.label === marker.label
      ) {
        return;
      }
      conv.messages.push({
        id: `stage-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
        role: "system",
        content: marker.label,
        timestamp: Date.now(),
        stageEvent: marker,
      });
    }),

  appendMessageDelta: (convId, delta) =>
    set((state) => {
      const conv = state.conversations[convId];
      if (!conv) return;
      const last = conv.messages[conv.messages.length - 1];
      if (last?.role === "assistant" && last.isStreaming) {
        last.content += delta;
      }
    }),

  appendMessageThinking: (convId, content) =>
    set((state) => {
      const conv = state.conversations[convId];
      if (!conv) return;
      const last = conv.messages[conv.messages.length - 1];
      if (last?.role === "assistant" && last.isStreaming) {
        const now = Date.now();

        // Interleaved segments: continue the open segment only if no answer
        // text or tool call landed since it started; otherwise open a new one.
        if (!last.thinkingSegments) last.thinkingSegments = [];
        const segs = last.thinkingSegments;
        const curContentLen = last.content.length;
        const curToolIndex = last.toolCalls?.length ?? 0;
        const open = segs[segs.length - 1];
        const sameStep =
          open != null && open.contentOffset === curContentLen && open.toolIndex === curToolIndex;
        if (sameStep) {
          open.content += content;
          open.endedAt = now;
        } else {
          segs.push({
            content,
            startedAt: now,
            endedAt: now,
            contentOffset: curContentLen,
            toolIndex: curToolIndex,
          });
        }

        // Keep the merged string in sync for persistence + history fallback.
        last.thinking = (last.thinking || "") + content;
        if (!last.thinkingStartedAt) last.thinkingStartedAt = now;
        last.thinkingEndedAt = now;
      }
    }),

  addMessageToolCall: (convId, toolCall) =>
    set((state) => {
      const conv = state.conversations[convId];
      if (!conv) return;
      const last = conv.messages[conv.messages.length - 1];
      if (last?.role === "assistant") {
        if (!last.toolCalls) {
          last.toolCalls = [];
          last.toolCallsContentOffset = last.content.length;
        }
        if (!last.toolCallOffsets) {
          last.toolCallOffsets = [];
        }
        if (toolCall.requestId) {
          const existing = last.toolCalls.find((tc) => tc.requestId === toolCall.requestId);
          if (existing) {
            existing.name = toolCall.name;
            existing.args = toolCall.args;
            return;
          }
        }
        last.toolCallOffsets.push(last.content.length);
        last.toolCalls.push(toolCall);
      }
    }),

  updateMessageToolResult: (convId, toolName, result, success) =>
    set((state) => {
      const conv = state.conversations[convId];
      if (!conv) return;
      const last = conv.messages[conv.messages.length - 1];
      if (last?.role === "assistant" && last.toolCalls) {
        const tc = [...last.toolCalls].reverse().find((t) => t.name === toolName);
        if (tc) {
          tc.result = result;
          tc.success = success;
        }
      }
    }),

  finalizeStreamingMessage: (convId, response, reasoning) =>
    set((state) => {
      const conv = state.conversations[convId];
      if (!conv) return;
      const last = conv.messages[conv.messages.length - 1];
      if (last?.role === "assistant") {
        if (response !== undefined) last.content = response;
        if (reasoning !== undefined) last.thinking = reasoning;
        last.isStreaming = false;
      }
      conv.isStreaming = false;
    }),

  setMessageError: (convId, errorMsg, severity = "error") =>
    set((state) => {
      const conv = state.conversations[convId];
      if (!conv) return;
      const last = conv.messages[conv.messages.length - 1];
      if (last?.role === "assistant" && last.isStreaming) {
        last.isStreaming = false;
        last.error = errorMsg;
        last.errorSeverity = severity;
      } else if (last?.role === "assistant" && last.error && isSameError(last.error, errorMsg)) {
        // The same failure surfacing twice — e.g. the backend `error` event and
        // the `send_ai_prompt_session` invoke rejection (which wraps the same
        // text with an `[API trace=…]` prefix). Collapse to one block: keep the
        // shorter/cleaner text and escalate to a hard error if either side is one.
        if (errorMsg.trim().length < last.error.trim().length) last.error = errorMsg;
        if (severity === "error") last.errorSeverity = "error";
      } else {
        conv.messages.push({
          id: `err-${Date.now()}`,
          role: "assistant",
          content: "",
          timestamp: Date.now(),
          error: errorMsg,
          errorSeverity: severity,
        });
      }
      conv.isStreaming = false;
    }),

  addTerminalToConversation: (convId, terminalId) =>
    set((state) => {
      // 1:1 model: each conversation has exactly one terminal
      state.conversationTerminals[convId] = [terminalId];
    }),

  removeTerminalFromConversation: (convId, terminalId) =>
    set((state) => {
      const terminals = state.conversationTerminals[convId];
      if (terminals) {
        const idx = terminals.indexOf(terminalId);
        if (idx !== -1) {
          terminals.splice(idx, 1);
        }
      }
    }),

  getActiveConversationTerminals: () => {
    const state = get() as ConversationState;
    const convId = state.activeConversationId;
    if (!convId) return [];
    return state.conversationTerminals[convId] ?? [];
  },

  getConversationForTerminal: (terminalId: string) => {
    const state = get() as ConversationState;
    for (const [convId, terminals] of Object.entries(state.conversationTerminals)) {
      if (terminals.includes(terminalId)) {
        return convId;
      }
    }
    return null;
  },

  getConversationBySessionId: (sessionId: string) => {
    const state = get() as ConversationState;
    for (const conv of Object.values(state.conversations)) {
      if (conv.aiSessionId === sessionId) {
        return conv;
      }
    }
    return null;
  },

  restoreConversations: (convs, order, activeId) =>
    set((state) => {
      state.conversations = {};
      state.conversationOrder = [];
      state.conversationTerminals = {};

      for (const conv of convs) {
        state.conversations[conv.id] = conv;
        state.conversationTerminals[conv.id] = [];
      }
      state.conversationOrder = order.filter((id) => state.conversations[id]);
      state.activeConversationId =
        activeId && state.conversations[activeId] ? activeId : (state.conversationOrder[0] ?? null);
    }),
});

// Selectors
export const selectActiveConversation = <T extends ConversationState>(
  state: T
): ChatConversation | null => {
  const convId = state.activeConversationId;
  return convId ? (state.conversations[convId] ?? null) : null;
};

export const selectConversationTerminals = <T extends ConversationState>(
  state: T,
  convId: string
): string[] => {
  return state.conversationTerminals[convId] ?? [];
};

export const selectActiveConversationTerminals = <T extends ConversationState>(
  state: T
): string[] => {
  const convId = state.activeConversationId;
  if (!convId) return [];
  return state.conversationTerminals[convId] ?? [];
};

export const selectAllConversations = <T extends ConversationState>(
  state: T
): ChatConversation[] => {
  return state.conversationOrder.map((id) => state.conversations[id]).filter(Boolean);
};
