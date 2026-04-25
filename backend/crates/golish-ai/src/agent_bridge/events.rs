//! Event emission helpers for [`AgentBridge`].
//!
//! Bridges incoming `AiEvent`s to the legacy `event_tx` mpsc channel and the
//! newer `GolishRuntime` abstraction, and provides sequence numbering / buffering
//! for the frontend-ready handshake.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use tokio::sync::mpsc;

use golish_core::events::{AiEvent, AiEventEnvelope};
use golish_core::runtime::RuntimeEvent;

use crate::event_coordinator::{CoordinatorHandle, CoordinatorState};

use super::AgentBridge;

impl AgentBridge {
    /// Create an event envelope with sequence number and timestamp.
    fn create_envelope(&self, event: AiEvent) -> AiEventEnvelope {
        let seq = self.event_sequence.fetch_add(1, Ordering::SeqCst);
        let ts = chrono::Utc::now().to_rfc3339();
        AiEventEnvelope { seq, ts, event }
    }

    /// Helper to emit events through available channels.
    ///
    /// Events are wrapped in an AiEventEnvelope with sequence number and timestamp.
    /// If the frontend has not signaled ready, events are buffered instead of emitted.
    ///
    /// When a coordinator is available, events are routed through it for deterministic
    /// ordering and deadlock-free processing. Otherwise, the legacy atomic-based path
    /// is used for backward compatibility.
    ///
    /// Uses `event_session_id` for routing events to the correct frontend tab.
    pub fn emit_event(&self, event: AiEvent) {
        // If coordinator is available, use it (new path)
        if let Some(ref coordinator) = self.coordinator {
            coordinator.emit(event);
            return;
        }

        // Legacy path: write to transcript and use atomic-based buffering.
        // Skip: streaming events (TextDelta/Reasoning), sub-agent internal events
        // (those go to a separate file).
        if let Some(ref writer) = self.transcript_writer {
            if !matches!(
                event,
                AiEvent::TextDelta { .. }
                    | AiEvent::Reasoning { .. }
                    | AiEvent::SubAgentToolRequest { .. }
                    | AiEvent::SubAgentToolResult { .. }
            ) {
                let writer = Arc::clone(writer);
                let event_clone = event.clone();
                tokio::spawn(async move {
                    if let Err(e) = writer.append(&event_clone).await {
                        tracing::warn!("Failed to write to transcript: {}", e);
                    }
                });
            }
        }

        let envelope = self.create_envelope(event.clone());

        // If frontend is not ready, buffer the event
        if !self.frontend_ready.load(Ordering::SeqCst) {
            if let Ok(mut buffer) = self.event_buffer.try_write() {
                tracing::debug!(
                    message = "[emit_event] Buffering event (frontend not ready)",
                    seq = envelope.seq,
                    event_type = envelope.event.event_type(),
                );
                buffer.push(envelope);
                return;
            }
            // Rare race condition during mark_frontend_ready: fall through to emit directly.
            tracing::debug!(
                message = "[emit_event] Could not acquire buffer lock, emitting directly",
                seq = envelope.seq,
            );
        }

        self.emit_envelope(envelope, event);
    }

    /// Emit an envelope through available channels.
    ///
    /// This is separated from `emit_event` to allow both direct emission and
    /// buffer flushing.
    fn emit_envelope(&self, envelope: AiEventEnvelope, event: AiEvent) {
        // Legacy event_tx path (without envelope for backward compat)
        if let Some(ref tx) = self.event_tx {
            let _ = tx.send(event.clone());
        }

        if let Some(ref rt) = self.runtime {
            // Use stored session_id for routing, fall back to "unknown" if not set
            let session_id = self.event_session_id.clone().unwrap_or_else(|| {
                tracing::warn!(
                    message = "[emit_event] event_session_id is None! Falling back to 'unknown'",
                    event_type = ?std::mem::discriminant(&event),
                );
                "unknown".to_string()
            });
            tracing::debug!(
                message = "[emit_event] Emitting event through runtime",
                session_id = %session_id,
                seq = envelope.seq,
                event_type = envelope.event.event_type(),
            );
            if let Err(e) = rt.emit(RuntimeEvent::AiEnvelope {
                session_id,
                envelope: Box::new(envelope),
            }) {
                tracing::warn!("Failed to emit event through runtime: {}", e);
            }
        } else {
            tracing::warn!(
                message = "[emit_event] No runtime available to emit event",
                event_type = ?std::mem::discriminant(&event),
            );
        }
    }

    /// Mark the frontend as ready to receive events.
    ///
    /// This flushes any buffered events and allows future events to be emitted directly.
    /// Should be called by the frontend after it has set up its event listeners.
    pub async fn mark_frontend_ready(&self) {
        // If coordinator is available, use it (new path)
        if let Some(ref coordinator) = self.coordinator {
            coordinator.mark_frontend_ready();
            return;
        }

        // Legacy path: take the buffer contents while holding the lock
        let buffered_events = {
            let mut buffer = self.event_buffer.write().await;
            std::mem::take(&mut *buffer)
        };

        let event_count = buffered_events.len();

        // Set the ready flag AFTER taking the buffer to avoid race conditions
        self.frontend_ready.store(true, Ordering::SeqCst);

        tracing::info!(
            message = "[mark_frontend_ready] Flushing buffered events",
            count = event_count,
        );

        for envelope in buffered_events {
            let event = envelope.event.clone();
            self.emit_envelope(envelope, event);
        }
    }

    /// Get the current event sequence number (for testing).
    ///
    /// Note: When coordinator is available, this returns 0 as the sequence
    /// is managed by the coordinator. Use `coordinator_state()` for accurate info.
    pub fn current_event_sequence(&self) -> u64 {
        self.event_sequence.load(Ordering::SeqCst)
    }

    /// Check if frontend is marked as ready (for testing).
    ///
    /// Note: When coordinator is available, this returns false as the state
    /// is managed by the coordinator. Use `coordinator_state()` for accurate info.
    pub fn is_frontend_ready(&self) -> bool {
        self.frontend_ready.load(Ordering::SeqCst)
    }

    /// Get the number of buffered events (for testing).
    ///
    /// Note: When coordinator is available, this returns 0 as the buffer
    /// is managed by the coordinator. Use `coordinator_state()` for accurate info.
    pub fn buffered_event_count(&self) -> usize {
        self.event_buffer.blocking_read().len()
    }

    /// Get the coordinator handle (if available).
    pub fn coordinator(&self) -> Option<&CoordinatorHandle> {
        self.coordinator.as_ref()
    }

    /// Query the coordinator state (for testing/debugging).
    ///
    /// Returns None if no coordinator is available or if it has shut down.
    pub async fn coordinator_state(&self) -> Option<CoordinatorState> {
        if let Some(ref coordinator) = self.coordinator {
            coordinator.query_state().await
        } else {
            None
        }
    }

    /// Get or create an event channel for the agentic loop.
    ///
    /// If `event_tx` is available, returns a clone of that sender.
    /// If only `runtime` is available, creates a forwarding channel that sends to runtime.
    ///
    /// This is a transition helper - once we update AgenticLoopContext to use runtime
    /// directly, this method will be removed.
    ///
    /// Uses `event_session_id` for routing events to the correct frontend tab.
    pub fn get_or_create_event_tx(&self) -> mpsc::UnboundedSender<AiEvent> {
        if let Some(ref tx) = self.event_tx {
            return tx.clone();
        }

        let runtime = self.runtime.clone().expect(
            "AgentBridge must have either event_tx or runtime - this is a bug in construction",
        );

        let session_id = self
            .event_session_id
            .clone()
            .unwrap_or_else(|| "unknown".to_string());

        let (tx, mut rx) = mpsc::unbounded_channel::<AiEvent>();

        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let Err(e) = runtime.emit(RuntimeEvent::Ai {
                    session_id: session_id.clone(),
                    event: Box::new(event),
                }) {
                    tracing::warn!("Failed to forward event to runtime: {}", e);
                }
            }
        });

        tx
    }
}
