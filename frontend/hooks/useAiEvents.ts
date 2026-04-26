import { useEffect, useRef } from "react";
import { type AiEvent, onAiEvent, signalFrontendReady } from "@/lib/ai";
import { logger } from "@/lib/logger";
import { useStore } from "@/store";
import { dispatchEvent, type EventHandlerContext } from "./ai-events";
import {
  pendingTextBatches,
  scheduledTextBatchFlush,
  runTextBatchFlush,
  scheduleTextBatchFlush,
} from "@/lib/ai/streaming-buffer";
export { discardPendingBatchedDeltasForAiSession, discardAllPendingBatchedDeltas } from "@/lib/ai/streaming-buffer";
import { convertToolSource } from "@/lib/ai/tool-source";

/**
 * Track last seen sequence number per session for deduplication.
 * This is module-level to persist across hook re-renders but within the same app lifecycle.
 */
const lastSeenSeq = new Map<string, number>();

/**
 * Throttle signal_frontend_ready calls to prevent HMR-induced spam.
 * Tracks the last signal time per session; ignores calls within the cooldown window.
 */
const lastSignaledAt = new Map<string, number>();
const SIGNAL_COOLDOWN_MS = 3000;

/**
 * Reset sequence tracking for a session.
 * Called when a session is removed or when the app needs to reset state.
 */
export function resetSessionSequence(sessionId: string): void {
  lastSeenSeq.delete(sessionId);
}

/**
 * Reset all sequence tracking. Useful for testing.
 */
export function resetAllSequences(): void {
  lastSeenSeq.clear();
}

/**
 * Get the number of sessions being tracked.
 * Useful for testing and debugging memory management.
 */
export function getSessionSequenceCount(): number {
  return lastSeenSeq.size;
}

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
      if (pendingTextBatches.size === 0) return;
      runTextBatchFlush();
    };

    // Flush pending deltas for a specific session immediately
    // Called before adding non-text blocks to ensure correct ordering
    const flushSessionDeltas = (sessionId: string) => {
      const pending = pendingTextBatches.get(sessionId);
      if (pending) {
        useStore.getState().updateAgentStreaming(sessionId, pending);
        pendingTextBatches.delete(sessionId);
      }
    };

    // Add a text delta to the pending batch
    const batchTextDelta = (sessionId: string, delta: string) => {
      const current = pendingTextBatches.get(sessionId) ?? "";
      pendingTextBatches.set(sessionId, current + delta);
      scheduleTextBatchFlush();
    };

    const handleEvent = (event: AiEvent) => {
      // Get the session ID from the event for proper routing
      const state = useStore.getState();
      let sessionId = event.session_id;

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

        // Last resort: if the active PTY session exists, use it directly.
        // This keeps events flowing even when conversation state is inconsistent.
        if (!resolved && state.activeSessionId && state.sessions[state.activeSessionId]) {
          logger.debug("AI event routed to active session as fallback:", {
            originalSessionId: sessionId,
            activeSessionId: state.activeSessionId,
            eventType: event.type,
          });
          sessionId = state.activeSessionId;
          resolved = true;
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
        const lastSeq = lastSeenSeq.get(sessionId) ?? -1;

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
        lastSeenSeq.set(sessionId, event.seq);
      }

      // Create handler context
      const ctx: EventHandlerContext = {
        sessionId,
        getState: () => useStore.getState(),
        flushSessionDeltas,
        batchTextDelta,
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
            const last = lastSignaledAt.get(sessionId) ?? 0;
            if (now - last < SIGNAL_COOLDOWN_MS) continue;
            lastSignaledAt.set(sessionId, now);
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
      // Flush any remaining deltas before unmount
      flushPendingDeltas();
    };
  }, []);
}
