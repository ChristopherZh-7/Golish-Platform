/**
 * Conversation slice for the Zustand store.
 *
 * Manages right-side AI chat conversations and their association with
 * left-side terminal tabs. Each conversation "owns" a group of terminals.
 */

import type { SliceCreator } from "./types";

export interface ChatToolCall {
  name: string;
  args: string;
  result?: string;
  success?: boolean;
}

export interface ChatMessage {
  id: string;
  role: "user" | "assistant";
  content: string;
  timestamp: number;
  isStreaming?: boolean;
  error?: string;
  toolCalls?: ChatToolCall[];
  thinking?: string;
}

export interface ChatConversation {
  id: string;
  title: string;
  messages: ChatMessage[];
  createdAt: number;
  aiSessionId: string;
  aiInitialized: boolean;
  isStreaming: boolean;
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
  /** Append text delta to the last streaming assistant message */
  appendMessageDelta: (convId: string, delta: string) => void;
  /** Append thinking content to the last streaming assistant message */
  appendMessageThinking: (convId: string, content: string) => void;
  /** Add a tool call to the last assistant message */
  addMessageToolCall: (convId: string, toolCall: ChatToolCall) => void;
  /** Update a tool call result on the last assistant message */
  updateMessageToolResult: (convId: string, toolName: string, result: string, success: boolean) => void;
  /** Finalize the last streaming message */
  finalizeStreamingMessage: (convId: string, response?: string, reasoning?: string) => void;
  /** Set error on the last streaming message or add an error message */
  setMessageError: (convId: string, errorMsg: string) => void;
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
    activeId: string | null,
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
export const createConversationSlice: SliceCreator<ConversationSlice> = (set, get) => ({
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
        state.activeConversationId =
          remaining.length > 0 ? remaining[remaining.length - 1] : null;
      }
    }),

  setActiveConversation: (convId) =>
    set((state) => {
      if (state.conversations[convId]) {
        state.activeConversationId = convId;
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
        last.thinking = (last.thinking || "") + content;
      }
    }),

  addMessageToolCall: (convId, toolCall) =>
    set((state) => {
      const conv = state.conversations[convId];
      if (!conv) return;
      const last = conv.messages[conv.messages.length - 1];
      if (last?.role === "assistant") {
        if (!last.toolCalls) last.toolCalls = [];
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

  setMessageError: (convId, errorMsg) =>
    set((state) => {
      const conv = state.conversations[convId];
      if (!conv) return;
      const last = conv.messages[conv.messages.length - 1];
      if (last?.role === "assistant" && last.isStreaming) {
        last.isStreaming = false;
        last.error = errorMsg;
      } else {
        conv.messages.push({
          id: `err-${Date.now()}`,
          role: "assistant",
          content: "",
          timestamp: Date.now(),
          error: errorMsg,
        });
      }
      conv.isStreaming = false;
    }),

  addTerminalToConversation: (convId, terminalId) =>
    set((state) => {
      if (!state.conversationTerminals[convId]) {
        state.conversationTerminals[convId] = [];
      }
      if (!state.conversationTerminals[convId].includes(terminalId)) {
        state.conversationTerminals[convId].push(terminalId);
      }
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
        activeId && state.conversations[activeId]
          ? activeId
          : state.conversationOrder[0] ?? null;
    }),
});

// Selectors
export const selectActiveConversation = <T extends ConversationState>(
  state: T
): ChatConversation | null => {
  const convId = state.activeConversationId;
  return convId ? state.conversations[convId] ?? null : null;
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
