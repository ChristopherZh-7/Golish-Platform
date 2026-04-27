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
        let seq = self.events.event_sequence.fetch_add(1, Ordering::SeqCst);
        let ts = chrono::Utc::now().to_rfc3339();
        AiEventEnvelope { seq, ts, event }
    }

    pub fn emit_event(&self, event: AiEvent) {
        if let Some(ref coordinator) = self.events.coordinator {
            coordinator.emit(event);
            return;
        }

        if let Some(ref writer) = self.events.transcript_writer {
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

        if !self.events.frontend_ready.load(Ordering::SeqCst) {
            if let Ok(mut buffer) = self.events.event_buffer.try_write() {
                tracing::debug!(
                    message = "[emit_event] Buffering event (frontend not ready)",
                    seq = envelope.seq,
                    event_type = envelope.event.event_type(),
                );
                buffer.push(envelope);
                return;
            }
            tracing::debug!(
                message = "[emit_event] Could not acquire buffer lock, emitting directly",
                seq = envelope.seq,
            );
        }

        self.emit_envelope(envelope, event);
    }

    fn emit_envelope(&self, envelope: AiEventEnvelope, event: AiEvent) {
        if let Some(ref tx) = self.events.event_tx {
            let _ = tx.send(event.clone());
        }

        if let Some(ref rt) = self.events.runtime {
            let session_id = self.events.event_session_id.clone().unwrap_or_else(|| {
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

    pub async fn mark_frontend_ready(&self) {
        if let Some(ref coordinator) = self.events.coordinator {
            coordinator.mark_frontend_ready();
            return;
        }

        let buffered_events = {
            let mut buffer = self.events.event_buffer.write().await;
            std::mem::take(&mut *buffer)
        };

        let event_count = buffered_events.len();
        self.events.frontend_ready.store(true, Ordering::SeqCst);

        tracing::info!(
            message = "[mark_frontend_ready] Flushing buffered events",
            count = event_count,
        );

        for envelope in buffered_events {
            let event = envelope.event.clone();
            self.emit_envelope(envelope, event);
        }
    }

    pub fn current_event_sequence(&self) -> u64 {
        self.events.event_sequence.load(Ordering::SeqCst)
    }

    pub fn is_frontend_ready(&self) -> bool {
        self.events.frontend_ready.load(Ordering::SeqCst)
    }

    pub fn buffered_event_count(&self) -> usize {
        self.events.event_buffer.blocking_read().len()
    }

    pub fn coordinator(&self) -> Option<&CoordinatorHandle> {
        self.events.coordinator.as_ref()
    }

    pub async fn coordinator_state(&self) -> Option<CoordinatorState> {
        if let Some(ref coordinator) = self.events.coordinator {
            coordinator.query_state().await
        } else {
            None
        }
    }

    pub fn get_or_create_event_tx(&self) -> mpsc::UnboundedSender<AiEvent> {
        if let Some(ref tx) = self.events.event_tx {
            return tx.clone();
        }

        let runtime = self.events.runtime.clone().expect(
            "AgentBridge must have either event_tx or runtime - this is a bug in construction",
        );

        let session_id = self
            .events
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
