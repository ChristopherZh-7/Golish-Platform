import { useStore } from "@/store";

const FLUSH_INTERVAL_MS = 16;
const pendingTextBatches = new Map<string, string>();
let lastTextBatchFlush = 0;
let scheduledTextBatchFlush: ReturnType<typeof setTimeout> | null = null;

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

export { pendingTextBatches, scheduledTextBatchFlush };

/**
 * Remove queued text (not yet applied to the store) for an AI or resolved PTY session.
 * Call this synchronously when the user cancels so no delayed flush re-applies tokens.
 */
export function discardPendingBatchedDeltasForAiSession(rawAiSessionId: string) {
  pendingTextBatches.delete(rawAiSessionId);
  const s = useStore.getState();
  const conv = s.getConversationBySessionId(rawAiSessionId);
  if (conv) {
    const termId = s.conversationTerminals[conv.id]?.[0];
    if (termId) pendingTextBatches.delete(termId);
  }
  if (pendingTextBatches.size === 0 && scheduledTextBatchFlush) {
    clearTimeout(scheduledTextBatchFlush);
    scheduledTextBatchFlush = null;
  }
}

/** Drop all queued `text_delta` batches (e.g. chat panel hidden) — keys may be AI or terminal ids. */
export function discardAllPendingBatchedDeltas() {
  pendingTextBatches.clear();
  if (scheduledTextBatchFlush) {
    clearTimeout(scheduledTextBatchFlush);
    scheduledTextBatchFlush = null;
  }
}
