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
const MAX_PENDING_AI_SESSION_BUCKETS = 32;
const MAX_PENDING_AI_EVENTS_PER_SESSION = 512;
const pendingSessionEvents = new Map<string, AiEvent[]>();

type AiEventRoutingState = ReturnType<typeof useStore.getState>;

export function clearPendingAiSessionEvents(): void {
  pendingSessionEvents.clear();
}

export function getPendingAiSessionEventCount(sessionId?: string): number {
  if (sessionId) return pendingSessionEvents.get(sessionId)?.length ?? 0;
  let total = 0;
  for (const events of pendingSessionEvents.values()) total += events.length;
  return total;
}

function resolveAiEventSessionId(rawSessionId: string, state: AiEventRoutingState): string | null {
  if (state.sessions[rawSessionId]) return rawSessionId;

  const conv = state.getConversationBySessionId(rawSessionId);
  if (conv) {
    const termId = state.conversationTerminals[conv.id]?.[0];
    if (termId && state.sessions[termId]) return termId;
  }

  const activeConvId = state.activeConversationId;
  const activeConv = activeConvId ? state.conversations[activeConvId] : null;
  if (activeConv?.aiSessionId !== rawSessionId) return null;
  const activeTermId = state.conversationTerminals[activeConvId!]?.[0];
  return activeTermId && state.sessions[activeTermId] ? activeTermId : null;
}

function bufferPendingAiEvent(sessionId: string, event: AiEvent): void {
  let events = pendingSessionEvents.get(sessionId);
  if (!events) {
    if (pendingSessionEvents.size >= MAX_PENDING_AI_SESSION_BUCKETS) {
      const oldestSessionId = pendingSessionEvents.keys().next().value;
      if (typeof oldestSessionId === "string") {
        pendingSessionEvents.delete(oldestSessionId);
        logger.warn("Dropping oldest pending AI session event bucket after capacity limit:", {
          sessionId: oldestSessionId,
        });
      }
    }
    events = [];
    pendingSessionEvents.set(sessionId, events);
  }

  if (events.length >= MAX_PENDING_AI_EVENTS_PER_SESSION) {
    events.shift();
    logger.warn("Dropping oldest pending AI event after per-session capacity limit:", {
      sessionId,
      eventType: event.type,
    });
  }
  events.push(event);
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

    const dispatchResolvedEvent = (event: AiEvent, sessionId: string) => {
      // Deduplication: check sequence number if present. Pending restore events
      // only reach this point after a real terminal session has been resolved,
      // so buffering never consumes a sequence before it can be replayed.
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

    let drainingPendingEvents = false;
    const drainPendingSessionEvents = () => {
      if (drainingPendingEvents || pendingSessionEvents.size === 0) return;
      drainingPendingEvents = true;
      try {
        for (const [rawSessionId, events] of Array.from(pendingSessionEvents.entries())) {
          const sessionId = resolveAiEventSessionId(rawSessionId, useStore.getState());
          if (!sessionId) continue;
          pendingSessionEvents.delete(rawSessionId);
          for (const event of events) dispatchResolvedEvent(event, sessionId);
        }
      } finally {
        drainingPendingEvents = false;
      }
    };

    const handleEvent = (event: AiEvent) => {
      const state = useStore.getState();
      let rawSessionId = event.session_id;

      if (isTitleGenSessionId(rawSessionId)) {
        logger.debug("Ignoring title-generation AI event:", {
          sessionId: rawSessionId,
          eventType: event.type,
        });
        return;
      }

      // Fall back to activeSessionId if session_id is unknown (shouldn't happen in normal operation)
      if (!rawSessionId || rawSessionId === "unknown") {
        logger.warn("AI event received with unknown session_id, falling back to activeSessionId");
        const fallbackId = state.activeSessionId;
        if (!fallbackId) return;
        rawSessionId = fallbackId;
      }

      const sessionId = resolveAiEventSessionId(rawSessionId, state);
      if (!sessionId) {
        bufferPendingAiEvent(rawSessionId, event);
        logger.warn("AI event buffered while its session is restoring:", {
          sessionId: rawSessionId,
          eventType: event.type,
          activeSessionId: state.activeSessionId,
        });
        return;
      }
      dispatchResolvedEvent(event, sessionId);
    };

    const unsubscribeStore = useStore.subscribe(drainPendingSessionEvents);

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
          drainPendingSessionEvents();
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
      unsubscribeStore();
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
