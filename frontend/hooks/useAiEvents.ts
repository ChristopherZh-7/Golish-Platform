import { useEffect, useRef } from "react";
import { type AiEvent, isTitleGenSessionId, onAiEvent, signalFrontendReady } from "@/lib/ai";
import {
  batchSubAgentThinking,
  batchThinkingContent,
  batchToolOutputChunk,
  pendingTextBatches,
  runRealtimeBatchFlush,
  runRealtimeBatchFlushForSession,
  runTextBatchFlush,
  scheduledRealtimeBatchFlush,
  scheduledTextBatchFlush,
  scheduleTextBatchFlush,
} from "@/lib/ai/streaming-buffer";
import { logger } from "@/lib/logger";
import { dispatchEvent, type EventHandlerContext } from "@/services/ai-events";
import {
  getLastSeenSequence,
  getLastSignaledAt,
  setLastSeenSequence,
  setLastSignaledAt,
} from "@/services/ai-events/session-sequence";
import { useStore } from "@/store";

export {
  discardAllPendingBatchedDeltas,
  discardPendingBatchedDeltasForAiSession,
} from "@/lib/ai/streaming-buffer";
export {
  getSessionSequenceCount,
  resetAllSequences,
  resetLastSignaledAt,
  resetSessionSequence,
} from "@/services/ai-events/session-sequence";

import { convertToolSource } from "@/lib/ai/tool-source";

const SIGNAL_COOLDOWN_MS = 3000;

/**
 * Hook to subscribe to AI events from the Tauri backend
 * and update the store accordingly.
 *
 * Events are routed to the correct session using `event.session_id` from the backend.
 * This ensures proper multi-session isolation even when the user switches tabs
 * during AI streaming.
 *
 * Uses the event handler registry pattern for maintainability.
 */
export function useAiEvents() {
  const unlistenRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    // Track if this effect instance is still mounted (for async cleanup)
    let isMounted = true;

    // Flush all pending text batches to the store
    const flushPendingDeltas = () => {
      if (pendingTextBatches.size > 0) runTextBatchFlush();
      runRealtimeBatchFlush();
    };

    const flushTextDeltas = (sessionId: string) => {
      const pending = pendingTextBatches.get(sessionId);
      if (pending) {
        useStore.getState().updateAgentStreaming(sessionId, pending);
        pendingTextBatches.delete(sessionId);
      }
    };

    // Flush pending deltas for a specific session immediately.
    // Called before adding non-text blocks to ensure correct ordering.
    const flushSessionDeltas = (sessionId: string) => {
      flushTextDeltas(sessionId);
      runRealtimeBatchFlushForSession(sessionId);
    };

    // Add a text delta to the pending batch
    const batchTextDelta = (sessionId: string, delta: string) => {
      runRealtimeBatchFlushForSession(sessionId);
      const current = pendingTextBatches.get(sessionId) ?? "";
      pendingTextBatches.set(sessionId, current + delta);
      scheduleTextBatchFlush();
    };

    const handleEvent = (event: AiEvent) => {
      // Get the session ID from the event for proper routing
      const state = useStore.getState();
      let sessionId = event.session_id;

      if (isTitleGenSessionId(sessionId)) {
        logger.debug("Ignoring title-generation AI event:", {
          sessionId,
          eventType: event.type,
        });
        return;
      }

      // Fall back to activeSessionId if session_id is unknown (shouldn't happen in normal operation)
      if (!sessionId || sessionId === "unknown") {
        logger.warn("AI event received with unknown session_id, falling back to activeSessionId");
        const fallbackId = state.activeSessionId;
        if (!fallbackId) return;
        sessionId = fallbackId;
      }

      // Verify the session exists in the store.
      // For conversation-mode AI sessions (e.g. pentest chat), the session_id is the
      // AI session ID which differs from the PTY session ID. Resolve via conversations.
      if (!state.sessions[sessionId]) {
        let resolved = false;
        const conv = state.getConversationBySessionId(sessionId);
        if (conv) {
          const termIds = state.conversationTerminals[conv.id];
          const termId = termIds?.[0];
          if (termId && state.sessions[termId]) {
            sessionId = termId;
            resolved = true;
          }
        }

        // Fallback: if the active conversation's aiSessionId matches this event,
        // route to the active session's terminal. This handles cases where the
        // conversation lookup fails (e.g. after DB restore or state reset).
        if (!resolved) {
          const activeConvId = state.activeConversationId;
          const activeConv = activeConvId ? state.conversations[activeConvId] : null;
          if (activeConv?.aiSessionId === sessionId) {
            const termIds = state.conversationTerminals[activeConvId!];
            const termId = termIds?.[0];
            if (termId && state.sessions[termId]) {
              sessionId = termId;
              resolved = true;
            }
          }
        }

        if (!resolved) {
          logger.warn("AI event dropped for unknown session:", {
            sessionId,
            eventType: event.type,
            activeSessionId: state.activeSessionId,
          });
          return;
        }
      }

      // Deduplication: check sequence number if present
      if (event.seq !== undefined) {
        const lastSeq = getLastSeenSequence(sessionId);

        // Skip duplicate or out-of-order events
        if (event.seq <= lastSeq) {
          logger.debug(
            `Skipping duplicate/out-of-order event: seq=${event.seq}, lastSeq=${lastSeq}, type=${event.type}`
          );
          return;
        }

        // Warn on sequence gaps (might indicate missed events)
        if (event.seq > lastSeq + 1) {
          logger.warn(
            `Event sequence gap: expected ${lastSeq + 1}, got ${event.seq} for session ${sessionId}`
          );
        }

        // Update last seen sequence
        setLastSeenSequence(sessionId, event.seq);
      }

      // Create handler context
      const ctx: EventHandlerContext = {
        sessionId,
        getState: () => useStore.getState(),
        flushTextDeltas,
        flushSessionDeltas,
        batchTextDelta,
        batchThinkingContent,
        batchSubAgentThinking,
        batchToolOutputChunk,
        convertToolSource,
      };

      // Dispatch to registered handler
      const handled = dispatchEvent(event, ctx);

      if (!handled) {
        logger.warn("Unhandled AI event type:", event.type);
      }
    };

    // Only set up listener once - the handler uses getState() to access current values
    const setupListener = async () => {
      try {
        const unlisten = await onAiEvent(handleEvent);
        // Only store the unlisten function if we're still mounted
        // This handles the React Strict Mode double-mount where cleanup runs
        // before the async setup completes
        if (isMounted) {
          unlistenRef.current = unlisten;

          const now = Date.now();
          const sessions = Object.keys(useStore.getState().sessions);
          for (const sessionId of sessions) {
            const last = getLastSignaledAt(sessionId);
            if (now - last < SIGNAL_COOLDOWN_MS) continue;
            setLastSignaledAt(sessionId, now);
            signalFrontendReady(sessionId).catch((err) => {
              logger.debug("Failed to signal frontend ready:", err);
            });
          }
        } else {
          // We were unmounted before setup completed - clean up immediately
          unlisten();
        }
      } catch {
        // AI backend not yet implemented - this is expected
        logger.debug("AI events not available - backend not implemented yet");
      }
    };

    setupListener();

    return () => {
      isMounted = false;
      if (unlistenRef.current) {
        unlistenRef.current();
        unlistenRef.current = null;
      }
      if (scheduledTextBatchFlush) {
        clearTimeout(scheduledTextBatchFlush);
      }
      if (scheduledRealtimeBatchFlush) {
        clearTimeout(scheduledRealtimeBatchFlush);
      }
      // Flush any remaining deltas before unmount
      flushPendingDeltas();
    };
  }, []);
}
