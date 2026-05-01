//! Commands accepted by the [`super::EventCoordinator`] task and the
//! [`CoordinatorState`] snapshot returned for `query_state`.

use std::sync::Arc;

use tokio::sync::oneshot;

use golish_core::events::AiEvent;
use golish_core::hitl::ApprovalDecision;

use crate::transcript::TranscriptWriter;

pub enum CoordinatorCommand {
    /// Emit an AI event to the frontend.
    /// Boxed to reduce variant size disparity (AiEvent is large).
    EmitEvent { event: Box<AiEvent> },

    /// Mark the frontend as ready to receive events (flushes buffer).
    MarkFrontendReady,

    /// Register a pending approval request.
    /// The response will be sent back via the oneshot channel.
    RegisterApproval {
        request_id: String,
        response_tx: oneshot::Sender<ApprovalDecision>,
    },

    /// Resolve a pending approval with a decision.
    ResolveApproval { decision: ApprovalDecision },

    /// Set (or update) the transcript writer for persisting events.
    SetTranscriptWriter { writer: Arc<TranscriptWriter> },

    /// Query the current coordinator state (for debugging/testing).
    QueryState {
        response_tx: oneshot::Sender<CoordinatorState>,
    },

    /// Shutdown the coordinator.
    Shutdown,
}

/// Snapshot of coordinator state for debugging/testing.
#[derive(Debug, Clone)]
pub struct CoordinatorState {
    /// Current event sequence number.
    pub event_sequence: u64,
    /// Whether the frontend is ready.
    pub frontend_ready: bool,
    /// Number of buffered events.
    pub buffered_event_count: usize,
    /// Number of pending approvals.
    pub pending_approval_count: usize,
    /// List of pending approval request IDs.
    pub pending_approval_ids: Vec<String>,
}
