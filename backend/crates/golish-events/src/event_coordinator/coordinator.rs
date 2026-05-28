//! [`EventCoordinator`] — owns event/approval/transcript state and runs as a
//! single tokio task that drains [`CoordinatorCommand`] messages.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};

use golish_core::events::{AiEvent, AiEventEnvelope};
use golish_core::hitl::ApprovalDecision;
use golish_core::runtime::{GolishRuntime, RuntimeEvent};

use crate::transcript::TranscriptWriter;

use super::commands::{CoordinatorCommand, CoordinatorState};
use super::handle::CoordinatorHandle;

/// The EventCoordinator owns all event-related state and processes commands
/// in a single tokio task, ensuring deterministic ordering and eliminating deadlocks.
pub struct EventCoordinator {
    /// Monotonically increasing sequence number for events.
    event_sequence: u64,
    /// Whether the frontend has signaled it is ready to receive events.
    frontend_ready: bool,
    /// Buffer for events emitted before frontend signals ready.
    event_buffer: Vec<AiEventEnvelope>,
    /// Pending approval requests waiting for decisions.
    pending_approvals: HashMap<String, oneshot::Sender<ApprovalDecision>>,
    /// Session ID for event routing.
    session_id: String,
    /// Runtime for emitting events.
    runtime: Arc<dyn GolishRuntime>,
    /// Transcript writer for persisting events (optional).
    transcript_writer: Option<Arc<TranscriptWriter>>,
}

impl EventCoordinator {
    /// Spawn a new EventCoordinator task.
    ///
    /// Returns a handle for sending commands to the coordinator.
    pub fn spawn(
        session_id: String,
        runtime: Arc<dyn GolishRuntime>,
        transcript_writer: Option<Arc<TranscriptWriter>>,
    ) -> CoordinatorHandle {
        let (tx, rx) = mpsc::unbounded_channel();

        let coordinator = Self {
            event_sequence: 0,
            frontend_ready: false,
            event_buffer: Vec::new(),
            pending_approvals: HashMap::new(),
            session_id,
            runtime,
            transcript_writer,
        };

        tokio::spawn(coordinator.run(rx));

        CoordinatorHandle { tx }
    }

    /// Run the coordinator event loop.
    async fn run(mut self, mut rx: mpsc::UnboundedReceiver<CoordinatorCommand>) {
        tracing::debug!(
            session_id = %self.session_id,
            "EventCoordinator started"
        );

        while let Some(command) = rx.recv().await {
            match command {
                CoordinatorCommand::EmitEvent { event } => {
                    self.handle_emit_event(*event).await;
                }
                CoordinatorCommand::MarkFrontendReady => {
                    self.handle_mark_frontend_ready().await;
                }
                CoordinatorCommand::RegisterApproval {
                    request_id,
                    response_tx,
                } => {
                    self.handle_register_approval(request_id, response_tx);
                }
                CoordinatorCommand::ResolveApproval { decision } => {
                    self.handle_resolve_approval(decision);
                }
                CoordinatorCommand::SetTranscriptWriter { writer } => {
                    self.transcript_writer = Some(writer);
                }
                CoordinatorCommand::QueryState { response_tx } => {
                    let state = CoordinatorState {
                        event_sequence: self.event_sequence,
                        frontend_ready: self.frontend_ready,
                        buffered_event_count: self.event_buffer.len(),
                        pending_approval_count: self.pending_approvals.len(),
                        pending_approval_ids: self.pending_approvals.keys().cloned().collect(),
                    };
                    let _ = response_tx.send(state);
                }
                CoordinatorCommand::Shutdown => {
                    tracing::debug!(
                        session_id = %self.session_id,
                        "EventCoordinator shutting down"
                    );
                    break;
                }
            }
        }

        tracing::debug!(
            session_id = %self.session_id,
            pending_approvals = self.pending_approvals.len(),
            buffered_events = self.event_buffer.len(),
            "EventCoordinator stopped"
        );
    }

    /// Create an event envelope with sequence number and timestamp.
    fn create_envelope(&mut self, event: AiEvent) -> AiEventEnvelope {
        let seq = self.event_sequence;
        self.event_sequence += 1;
        let ts = chrono::Utc::now().to_rfc3339();
        AiEventEnvelope { seq, ts, event }
    }

    /// Write an event to the transcript (if configured).
    async fn write_to_transcript(&self, event: &AiEvent) {
        if let Some(ref writer) = self.transcript_writer {
            if crate::transcript::should_transcript(event) {
                if let Err(e) = writer.append(event).await {
                    tracing::warn!("Failed to write to transcript: {}", e);
                }
            }
        }
    }

    fn transcript_path_for_log(&self) -> Option<String> {
        self.transcript_writer
            .as_ref()
            .map(|writer| writer.path().display().to_string())
    }

    fn preview_text(value: &str, max_chars: usize) -> String {
        let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut preview = compact.chars().take(max_chars).collect::<String>();
        if compact.chars().count() > max_chars {
            preview.push_str("...");
        }
        preview
    }

    fn preview_json(value: &serde_json::Value, max_chars: usize) -> String {
        serde_json::to_string(value)
            .map(|s| Self::preview_text(&s, max_chars))
            .unwrap_or_else(|_| "<unserializable>".to_string())
    }

    fn log_observable_event(&self, event: &AiEvent) {
        match event {
            AiEvent::Completed {
                response,
                input_tokens,
                output_tokens,
                duration_ms,
                ..
            } => {
                tracing::debug!(
                    session_id = %self.session_id,
                    response_chars = response.len(),
                    response_preview = %Self::preview_text(response, 500),
                    input_tokens = ?input_tokens,
                    output_tokens = ?output_tokens,
                    duration_ms = ?duration_ms,
                    transcript_path = ?self.transcript_path_for_log(),
                    "[agent-observe] Turn completed"
                );

                if response.contains("<tool_call") || response.contains("<function=") {
                    tracing::warn!(
                        session_id = %self.session_id,
                        response_preview = %Self::preview_text(response, 500),
                        transcript_path = ?self.transcript_path_for_log(),
                        "[agent-observe] Completed response contains textual tool-call markup"
                    );
                }
            }
            AiEvent::ToolApprovalRequest {
                request_id,
                tool_name,
                args,
                risk_level,
                ..
            } => {
                tracing::info!(
                    session_id = %self.session_id,
                    request_id = %request_id,
                    tool_name = %tool_name,
                    risk_level = ?risk_level,
                    "[agent-observe] Tool approval requested"
                );
                tracing::debug!(
                    session_id = %self.session_id,
                    request_id = %request_id,
                    tool_name = %tool_name,
                    args_preview = %Self::preview_json(args, 500),
                    "[agent-observe] Tool approval args"
                );
            }
            AiEvent::ToolIntentObservation {
                request_id,
                tool_name,
                source,
                decision,
                reason,
                raw_preview,
            } => {
                tracing::info!(
                    session_id = %self.session_id,
                    request_id = %request_id,
                    tool_name = %tool_name,
                    source = %source,
                    decision = %decision,
                    reason = ?reason,
                    "[agent-observe] Tool intent observed"
                );
                if let Some(raw_preview) = raw_preview {
                    tracing::debug!(
                        session_id = %self.session_id,
                        request_id = %request_id,
                        raw_preview = %Self::preview_text(raw_preview, 500),
                        "[agent-observe] Tool intent raw preview"
                    );
                }
            }
            AiEvent::AskHumanRequest {
                request_id,
                question,
                input_type,
                context,
                ..
            } => {
                tracing::info!(
                    session_id = %self.session_id,
                    request_id = %request_id,
                    input_type = %input_type,
                    question_preview = %Self::preview_text(question, 240),
                    "[agent-observe] ask_human requested"
                );
                if !context.trim().is_empty() {
                    tracing::debug!(
                        session_id = %self.session_id,
                        request_id = %request_id,
                        context_preview = %Self::preview_text(context, 500),
                        "[agent-observe] ask_human context"
                    );
                }
            }
            AiEvent::ToolResult {
                request_id,
                tool_name,
                result,
                success,
                ..
            } => {
                tracing::info!(
                    session_id = %self.session_id,
                    request_id = %request_id,
                    tool_name = %tool_name,
                    success = %success,
                    result_chars = serde_json::to_string(result).map(|s| s.len()).unwrap_or(0),
                    "[agent-observe] Tool result"
                );
                tracing::debug!(
                    session_id = %self.session_id,
                    request_id = %request_id,
                    tool_name = %tool_name,
                    result_preview = %Self::preview_json(result, 700),
                    "[agent-observe] Tool result preview"
                );
            }
            _ => {}
        }
    }

    /// Emit an envelope to the frontend via the runtime.
    fn emit_envelope(&self, envelope: AiEventEnvelope) {
        tracing::debug!(
            session_id = %self.session_id,
            seq = envelope.seq,
            event_type = envelope.event.event_type(),
            "Emitting event via coordinator"
        );

        if let Err(e) = self.runtime.emit(RuntimeEvent::AiEnvelope {
            session_id: self.session_id.clone(),
            envelope: Box::new(envelope),
        }) {
            tracing::warn!("Failed to emit event through runtime: {}", e);
        }
    }

    /// Handle EmitEvent command.
    async fn handle_emit_event(&mut self, event: AiEvent) {
        self.log_observable_event(&event);

        // Write to transcript
        self.write_to_transcript(&event).await;

        // Create envelope with sequence number
        let envelope = self.create_envelope(event);

        // If frontend is not ready, buffer the event
        if !self.frontend_ready {
            tracing::debug!(
                session_id = %self.session_id,
                seq = envelope.seq,
                event_type = envelope.event.event_type(),
                "Buffering event (frontend not ready)"
            );
            self.event_buffer.push(envelope);
            return;
        }

        // Emit directly
        self.emit_envelope(envelope);
    }

    /// Handle MarkFrontendReady command.
    async fn handle_mark_frontend_ready(&mut self) {
        let buffered_count = self.event_buffer.len();

        tracing::info!(
            session_id = %self.session_id,
            buffered_events = buffered_count,
            "Marking frontend ready, flushing buffered events"
        );

        // Set ready flag first
        self.frontend_ready = true;

        // Flush buffered events in order
        let buffered_events = std::mem::take(&mut self.event_buffer);
        for envelope in buffered_events {
            self.emit_envelope(envelope);
        }
    }

    /// Handle RegisterApproval command.
    fn handle_register_approval(
        &mut self,
        request_id: String,
        response_tx: oneshot::Sender<ApprovalDecision>,
    ) {
        tracing::debug!(
            session_id = %self.session_id,
            request_id = %request_id,
            "Registering approval request"
        );
        self.pending_approvals.insert(request_id, response_tx);
    }

    /// Handle ResolveApproval command.
    fn handle_resolve_approval(&mut self, decision: ApprovalDecision) {
        let request_id = &decision.request_id;

        if let Some(sender) = self.pending_approvals.remove(request_id) {
            tracing::debug!(
                session_id = %self.session_id,
                request_id = %request_id,
                approved = decision.approved,
                "Resolving approval request"
            );
            let _ = sender.send(decision);
        } else {
            tracing::warn!(
                session_id = %self.session_id,
                request_id = %request_id,
                "No pending approval found for request_id"
            );
        }
    }
}
