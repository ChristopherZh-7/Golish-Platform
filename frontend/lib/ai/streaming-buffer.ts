import { useStore } from "@/store";

const FLUSH_INTERVAL_MS = 16;
const pendingTextBatches = new Map<string, string>();
const pendingThinkingBatches = new Map<string, string>();
const pendingConversationThinkingBatches = new Map<string, string>();
const pendingSubAgentThinkingBatches = new Map<
  string,
  { sessionId: string; parentRequestId: string; text: string }
>();
const pendingToolOutputBatches = new Map<
  string,
  { sessionId: string; toolId: string; chunk: string; target: "main" | "sub_agent" }
>();
let lastTextBatchFlush = 0;
let lastRealtimeBatchFlush = 0;
let scheduledTextBatchFlush: ReturnType<typeof setTimeout> | null = null;
let scheduledRealtimeBatchFlush: ReturnType<typeof setTimeout> | null = null;

export function runTextBatchFlush() {
  scheduledTextBatchFlush = null;
  if (pendingTextBatches.size === 0) return;
  const state = useStore.getState();
  for (const [sessionId, delta] of pendingTextBatches) {
    state.updateAgentStreaming(sessionId, delta);
  }
  pendingTextBatches.clear();
  lastTextBatchFlush = Date.now();
}

export function scheduleTextBatchFlush() {
  if (pendingTextBatches.size === 0) return;
  const now = Date.now();
  if (now - lastTextBatchFlush >= FLUSH_INTERVAL_MS) {
    if (scheduledTextBatchFlush) {
      clearTimeout(scheduledTextBatchFlush);
      scheduledTextBatchFlush = null;
    }
    runTextBatchFlush();
  } else if (!scheduledTextBatchFlush) {
    scheduledTextBatchFlush = setTimeout(
      runTextBatchFlush,
      Math.max(0, FLUSH_INTERVAL_MS - (now - lastTextBatchFlush))
    );
  }
}

function hasRealtimeBatches() {
  return (
    pendingThinkingBatches.size > 0 ||
    pendingConversationThinkingBatches.size > 0 ||
    pendingSubAgentThinkingBatches.size > 0 ||
    pendingToolOutputBatches.size > 0
  );
}

function flushRealtimeBatchesWhere(predicate?: (sessionId: string) => boolean) {
  const state = useStore.getState();

  for (const [sessionId, content] of Array.from(pendingThinkingBatches.entries())) {
    if (predicate && !predicate(sessionId)) continue;
    state.appendThinkingContent(sessionId, content);
    pendingThinkingBatches.delete(sessionId);
  }

  for (const [convId, content] of Array.from(pendingConversationThinkingBatches.entries())) {
    state.appendMessageThinking(convId, content);
    pendingConversationThinkingBatches.delete(convId);
  }

  for (const [key, entry] of Array.from(pendingSubAgentThinkingBatches.entries())) {
    if (predicate && !predicate(entry.sessionId)) continue;
    state.updateSubAgentThinking(entry.sessionId, entry.parentRequestId, entry.text);
    pendingSubAgentThinkingBatches.delete(key);
  }

  for (const [key, entry] of Array.from(pendingToolOutputBatches.entries())) {
    if (predicate && !predicate(entry.sessionId)) continue;
    if (entry.target === "sub_agent") {
      state.appendSubAgentToolOutput(entry.sessionId, entry.toolId, entry.chunk);
    } else {
      state.appendToolStreamingOutput(entry.sessionId, entry.toolId, entry.chunk);
      state.appendToolExecutionOutput(entry.sessionId, entry.toolId, entry.chunk);
    }
    pendingToolOutputBatches.delete(key);
  }

  if (!hasRealtimeBatches() && scheduledRealtimeBatchFlush) {
    clearTimeout(scheduledRealtimeBatchFlush);
    scheduledRealtimeBatchFlush = null;
  }
  lastRealtimeBatchFlush = Date.now();
}

export function runRealtimeBatchFlush() {
  scheduledRealtimeBatchFlush = null;
  if (!hasRealtimeBatches()) return;
  flushRealtimeBatchesWhere();
}

export function runRealtimeBatchFlushForSession(sessionId: string) {
  flushRealtimeBatchesWhere((candidate) => candidate === sessionId);
}

export function runRealtimeBatchFlushForConversation(convId: string) {
  const content = pendingConversationThinkingBatches.get(convId);
  if (!content) return;
  useStore.getState().appendMessageThinking(convId, content);
  pendingConversationThinkingBatches.delete(convId);
  if (!hasRealtimeBatches() && scheduledRealtimeBatchFlush) {
    clearTimeout(scheduledRealtimeBatchFlush);
    scheduledRealtimeBatchFlush = null;
  }
}

export function scheduleRealtimeBatchFlush() {
  if (!hasRealtimeBatches()) return;
  const now = Date.now();
  if (now - lastRealtimeBatchFlush >= FLUSH_INTERVAL_MS) {
    if (scheduledRealtimeBatchFlush) {
      clearTimeout(scheduledRealtimeBatchFlush);
      scheduledRealtimeBatchFlush = null;
    }
    runRealtimeBatchFlush();
  } else if (!scheduledRealtimeBatchFlush) {
    scheduledRealtimeBatchFlush = setTimeout(
      runRealtimeBatchFlush,
      Math.max(0, FLUSH_INTERVAL_MS - (now - lastRealtimeBatchFlush))
    );
  }
}

export function batchThinkingContent(sessionId: string, content: string) {
  if (!content) return;
  pendingThinkingBatches.set(sessionId, (pendingThinkingBatches.get(sessionId) ?? "") + content);
  scheduleRealtimeBatchFlush();
}

export function batchConversationThinking(convId: string, content: string) {
  if (!content) return;
  pendingConversationThinkingBatches.set(
    convId,
    (pendingConversationThinkingBatches.get(convId) ?? "") + content
  );
  scheduleRealtimeBatchFlush();
}

export function batchSubAgentThinking(sessionId: string, parentRequestId: string, text: string) {
  pendingSubAgentThinkingBatches.set(`${sessionId}\0${parentRequestId}`, {
    sessionId,
    parentRequestId,
    text,
  });
  scheduleRealtimeBatchFlush();
}

export function batchToolOutputChunk(
  sessionId: string,
  toolId: string,
  chunk: string,
  target: "main" | "sub_agent"
) {
  if (!chunk) return;
  const key = `${target}\0${sessionId}\0${toolId}`;
  const current = pendingToolOutputBatches.get(key);
  pendingToolOutputBatches.set(key, {
    sessionId,
    toolId,
    target,
    chunk: (current?.chunk ?? "") + chunk,
  });
  scheduleRealtimeBatchFlush();
}

export {
  pendingTextBatches,
  scheduledRealtimeBatchFlush,
  scheduledTextBatchFlush,
  pendingThinkingBatches,
  pendingToolOutputBatches,
};

/**
 * Remove queued text (not yet applied to the store) for an AI or resolved PTY session.
 * Call this synchronously when the user cancels so no delayed flush re-applies tokens.
 */
export function discardPendingBatchedDeltasForAiSession(rawAiSessionId: string) {
  pendingTextBatches.delete(rawAiSessionId);
  pendingThinkingBatches.delete(rawAiSessionId);
  for (const [key, entry] of Array.from(pendingSubAgentThinkingBatches.entries())) {
    if (entry.sessionId === rawAiSessionId) pendingSubAgentThinkingBatches.delete(key);
  }
  for (const [key, entry] of Array.from(pendingToolOutputBatches.entries())) {
    if (entry.sessionId === rawAiSessionId) pendingToolOutputBatches.delete(key);
  }
  const s = useStore.getState();
  const conv = s.getConversationBySessionId(rawAiSessionId);
  if (conv) {
    const termId = s.conversationTerminals[conv.id]?.[0];
    if (termId) pendingTextBatches.delete(termId);
    pendingConversationThinkingBatches.delete(conv.id);
    if (termId) {
      pendingThinkingBatches.delete(termId);
      for (const [key, entry] of Array.from(pendingSubAgentThinkingBatches.entries())) {
        if (entry.sessionId === termId) pendingSubAgentThinkingBatches.delete(key);
      }
      for (const [key, entry] of Array.from(pendingToolOutputBatches.entries())) {
        if (entry.sessionId === termId) pendingToolOutputBatches.delete(key);
      }
    }
  }
  if (pendingTextBatches.size === 0 && scheduledTextBatchFlush) {
    clearTimeout(scheduledTextBatchFlush);
    scheduledTextBatchFlush = null;
  }
  if (!hasRealtimeBatches() && scheduledRealtimeBatchFlush) {
    clearTimeout(scheduledRealtimeBatchFlush);
    scheduledRealtimeBatchFlush = null;
  }
}

/** Drop all queued `text_delta` batches (e.g. chat panel hidden) — keys may be AI or terminal ids. */
export function discardAllPendingBatchedDeltas() {
  pendingTextBatches.clear();
  pendingThinkingBatches.clear();
  pendingConversationThinkingBatches.clear();
  pendingSubAgentThinkingBatches.clear();
  pendingToolOutputBatches.clear();
  if (scheduledTextBatchFlush) {
    clearTimeout(scheduledTextBatchFlush);
    scheduledTextBatchFlush = null;
  }
  if (scheduledRealtimeBatchFlush) {
    clearTimeout(scheduledRealtimeBatchFlush);
    scheduledRealtimeBatchFlush = null;
  }
}
