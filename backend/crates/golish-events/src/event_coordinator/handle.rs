//! [`CoordinatorHandle`] — cheap-to-clone send-side for the coordinator task.
//!
//! All public API on the bridge funnels through this handle. Methods are
//! fire-and-forget over an unbounded mpsc channel so callers never block on
//! coordinator state.

use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};

use golish_core::events::AiEvent;
use golish_core::hitl::ApprovalDecision;

use crate::transcript::TranscriptWriter;

use super::commands::{CoordinatorCommand, CoordinatorState};

/// Handle for sending commands to the [`EventCoordinator`](super::EventCoordinator).
///
/// Cheap to clone and can be passed around freely. Commands are sent via an
/// unbounded channel for fire-and-forget semantics.
#[derive(Clone)]
pub struct CoordinatorHandle {
    pub(super) tx: mpsc::UnboundedSender<CoordinatorCommand>,
}

impl CoordinatorHandle {
    /// Emit an AI event (fire-and-forget).
    ///
    /// If the frontend is not ready, the event will be buffered.
    pub fn emit(&self, event: AiEvent) {
        let _ = self.tx.send(CoordinatorCommand::EmitEvent {
            event: Box::new(event),
        });
    }

    /// Mark the frontend as ready to receive events.
    ///
    /// This flushes any buffered events in sequence order.
    pub fn mark_frontend_ready(&self) {
        let _ = self.tx.send(CoordinatorCommand::MarkFrontendReady);
    }

    /// Set the transcript writer for event persistence.
    pub fn set_transcript_writer(&self, writer: Arc<TranscriptWriter>) {
        let _ = self
            .tx
            .send(CoordinatorCommand::SetTranscriptWriter { writer });
    }

    /// Register a pending approval request.
    ///
    /// Returns a receiver that will receive the approval decision
    /// when `resolve_approval` is called with a matching request ID.
    pub fn register_approval(&self, request_id: String) -> oneshot::Receiver<ApprovalDecision> {
        let (response_tx, response_rx) = oneshot::channel();
        let _ = self.tx.send(CoordinatorCommand::RegisterApproval {
            request_id,
            response_tx,
        });
        response_rx
    }

    /// Resolve a pending approval with a decision.
    ///
    /// The decision will be sent to the receiver registered with `register_approval`.
    pub fn resolve_approval(&self, decision: ApprovalDecision) {
        let _ = self
            .tx
            .send(CoordinatorCommand::ResolveApproval { decision });
    }

    /// Query the current coordinator state.
    ///
    /// Returns `None` if the coordinator has shut down.
    pub async fn query_state(&self) -> Option<CoordinatorState> {
        let (response_tx, response_rx) = oneshot::channel();
        if self
            .tx
            .send(CoordinatorCommand::QueryState { response_tx })
            .is_err()
        {
            return None;
        }
        response_rx.await.ok()
    }

    /// Shutdown the coordinator.
    pub fn shutdown(&self) {
        let _ = self.tx.send(CoordinatorCommand::Shutdown);
    }

    /// Check if the coordinator is still running.
    pub fn is_alive(&self) -> bool {
        !self.tx.is_closed()
    }
}
