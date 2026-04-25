//! `EventCoordinator` integration tests covering sequencing, buffering,
//! approval lifecycle, shutdown, and transcript persistence behavior.

use std::sync::Arc;

use golish_core::events::AiEvent;
use golish_core::hitl::ApprovalDecision;
use golish_core::runtime::{GolishRuntime, RuntimeEvent};

use crate::transcript::TranscriptWriter;

use super::commands::CoordinatorState;
use super::coordinator::EventCoordinator;
use super::handle::CoordinatorHandle;
use super::*;
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A simple mock runtime for testing.
struct MockRuntime {
    emit_count: AtomicUsize,
}

impl MockRuntime {
    fn new() -> Self {
        Self {
            emit_count: AtomicUsize::new(0),
        }
    }

    fn emit_count(&self) -> usize {
        self.emit_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl GolishRuntime for MockRuntime {
    fn emit(&self, _event: RuntimeEvent) -> Result<(), golish_core::runtime::RuntimeError> {
        self.emit_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn request_approval(
        &self,
        _request_id: String,
        _tool_name: String,
        _args: serde_json::Value,
        _risk_level: String,
    ) -> Result<golish_core::runtime::ApprovalResult, golish_core::runtime::RuntimeError> {
        Ok(golish_core::runtime::ApprovalResult::Approved)
    }

    fn is_interactive(&self) -> bool {
        false
    }

    fn auto_approve(&self) -> bool {
        true
    }

    async fn shutdown(&self) -> Result<(), golish_core::runtime::RuntimeError> {
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[tokio::test]
async fn test_event_sequencing() {
    let runtime = Arc::new(MockRuntime::new());
    let handle = EventCoordinator::spawn("test-session".to_string(), runtime.clone(), None);

    // Mark frontend ready first
    handle.mark_frontend_ready();
    tokio::task::yield_now().await;

    // Emit multiple events
    handle.emit(AiEvent::Started {
        turn_id: "1".to_string(),
    });
    handle.emit(AiEvent::Started {
        turn_id: "2".to_string(),
    });
    handle.emit(AiEvent::Started {
        turn_id: "3".to_string(),
    });

    // Give coordinator time to process
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Query state
    let state = handle.query_state().await.unwrap();
    assert_eq!(state.event_sequence, 3);
    assert!(state.frontend_ready);
    assert_eq!(state.buffered_event_count, 0);

    // Check emit count
    assert_eq!(runtime.emit_count(), 3);

    handle.shutdown();
}

#[tokio::test]
async fn test_buffering_before_frontend_ready() {
    let runtime = Arc::new(MockRuntime::new());
    let handle = EventCoordinator::spawn("test-session".to_string(), runtime.clone(), None);

    // Emit events before frontend is ready
    handle.emit(AiEvent::Started {
        turn_id: "1".to_string(),
    });
    handle.emit(AiEvent::Started {
        turn_id: "2".to_string(),
    });

    // Give coordinator time to process
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Events should be buffered
    let state = handle.query_state().await.unwrap();
    assert!(!state.frontend_ready);
    assert_eq!(state.buffered_event_count, 2);
    assert_eq!(runtime.emit_count(), 0);

    // Mark frontend ready
    handle.mark_frontend_ready();
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Buffer should be flushed
    let state = handle.query_state().await.unwrap();
    assert!(state.frontend_ready);
    assert_eq!(state.buffered_event_count, 0);
    assert_eq!(runtime.emit_count(), 2);

    handle.shutdown();
}

#[tokio::test]
async fn test_approval_registration_and_resolution() {
    let runtime = Arc::new(MockRuntime::new());
    let handle = EventCoordinator::spawn("test-session".to_string(), runtime, None);

    // Register an approval
    let decision_rx = handle.register_approval("request-123".to_string());

    // Give coordinator time to process
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Check state
    let state = handle.query_state().await.unwrap();
    assert_eq!(state.pending_approval_count, 1);
    assert!(state
        .pending_approval_ids
        .contains(&"request-123".to_string()));

    // Resolve the approval
    handle.resolve_approval(ApprovalDecision {
        request_id: "request-123".to_string(),
        approved: true,
        reason: Some("Test approval".to_string()),
        remember: false,
        always_allow: false,
    });

    // Receive the decision
    let decision = decision_rx.await.unwrap();
    assert!(decision.approved);
    assert_eq!(decision.request_id, "request-123");

    // Check state - approval should be removed
    let state = handle.query_state().await.unwrap();
    assert_eq!(state.pending_approval_count, 0);

    handle.shutdown();
}

#[tokio::test]
async fn test_shutdown() {
    let runtime = Arc::new(MockRuntime::new());
    let handle = EventCoordinator::spawn("test-session".to_string(), runtime, None);

    assert!(handle.is_alive());

    handle.shutdown();
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // After shutdown, query_state should return None
    assert!(handle.query_state().await.is_none());
}

/// Helper to create a transcript writer in a temp directory.
async fn create_test_writer(
    temp_dir: &tempfile::TempDir,
    session_id: &str,
) -> Arc<TranscriptWriter> {
    Arc::new(
        TranscriptWriter::new(temp_dir.path(), session_id)
            .await
            .unwrap(),
    )
}

/// Helper to wait for the coordinator to process pending commands.
async fn flush_coordinator(handle: &CoordinatorHandle) {
    // query_state is a round-trip through the command queue, so when it
    // returns we know all prior commands have been processed.
    let _ = handle.query_state().await;
}

/// Regression test for the bug where the coordinator was spawned with
/// transcript_writer=None and never received the writer that was set later
/// on the AgentBridge. Events emitted after set_transcript_writer must
/// appear in the transcript file.
#[tokio::test]
async fn test_set_transcript_writer_enables_event_persistence() {
    let runtime = Arc::new(MockRuntime::new());
    let handle = EventCoordinator::spawn("test-session".to_string(), runtime, None);
    handle.mark_frontend_ready();

    let temp_dir = tempfile::TempDir::new().unwrap();
    let session_id = "test-transcript";
    let writer = create_test_writer(&temp_dir, session_id).await;

    // Set the transcript writer after construction (mirrors real usage
    // where AgentBridge::set_transcript_writer is called post-construction)
    handle.set_transcript_writer(writer);
    flush_coordinator(&handle).await;

    // Emit a realistic turn: Started → UserMessage → Completed
    handle.emit(AiEvent::Started {
        turn_id: "turn-1".to_string(),
    });
    handle.emit(AiEvent::UserMessage {
        content: "Hello, agent".to_string(),
    });
    handle.emit(AiEvent::Completed {
        response: "Hi there!".to_string(),
        reasoning: Some("I should greet the user".to_string()),
        input_tokens: Some(10),
        output_tokens: Some(5),
        duration_ms: Some(100),
    });
    flush_coordinator(&handle).await;

    let events = crate::transcript::read_transcript(temp_dir.path(), session_id)
        .await
        .unwrap();
    assert_eq!(
        events.len(),
        3,
        "Expected Started + UserMessage + Completed"
    );

    assert!(matches!(events[0].event, AiEvent::Started { .. }));
    assert!(matches!(
        events[1].event,
        AiEvent::UserMessage { ref content } if content == "Hello, agent"
    ));
    assert!(matches!(
        events[2].event,
        AiEvent::Completed { ref response, ref reasoning, .. }
            if response == "Hi there!" && reasoning.as_deref() == Some("I should greet the user")
    ));
}

/// Events emitted before set_transcript_writer is called must not
/// retroactively appear in the transcript.
#[tokio::test]
async fn test_events_before_writer_not_persisted() {
    let runtime = Arc::new(MockRuntime::new());
    let handle = EventCoordinator::spawn("test-session".to_string(), runtime, None);
    handle.mark_frontend_ready();

    let temp_dir = tempfile::TempDir::new().unwrap();
    let session_id = "test-before-writer";

    // Emit an event BEFORE the writer is set
    handle.emit(AiEvent::Started {
        turn_id: "turn-0".to_string(),
    });
    handle.emit(AiEvent::UserMessage {
        content: "lost message".to_string(),
    });
    flush_coordinator(&handle).await;

    // Now set the writer
    let writer = create_test_writer(&temp_dir, session_id).await;
    handle.set_transcript_writer(writer);
    flush_coordinator(&handle).await;

    // Emit an event AFTER the writer is set
    handle.emit(AiEvent::Started {
        turn_id: "turn-1".to_string(),
    });
    flush_coordinator(&handle).await;

    let events = crate::transcript::read_transcript(temp_dir.path(), session_id)
        .await
        .unwrap();
    // Only the post-writer event should appear
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].event,
        AiEvent::Started { ref turn_id } if turn_id == "turn-1"
    ));
}

/// All four filtered event types (TextDelta, Reasoning, SubAgentToolRequest,
/// SubAgentToolResult) must be excluded from the transcript while other
/// events pass through.
#[tokio::test]
async fn test_all_filtered_event_types_excluded() {
    let runtime = Arc::new(MockRuntime::new());
    let temp_dir = tempfile::TempDir::new().unwrap();
    let session_id = "test-filtered";
    let writer = create_test_writer(&temp_dir, session_id).await;

    let handle = EventCoordinator::spawn("test-session".to_string(), runtime, Some(writer));
    handle.mark_frontend_ready();

    // One event that should persist (anchor)
    handle.emit(AiEvent::Started {
        turn_id: "turn-1".to_string(),
    });

    // All four filtered types
    handle.emit(AiEvent::TextDelta {
        delta: "He".to_string(),
        accumulated: "He".to_string(),
    });
    handle.emit(AiEvent::Reasoning {
        content: "thinking...".to_string(),
    });
    handle.emit(AiEvent::SubAgentToolRequest {
        request_id: "req-1".to_string(),
        tool_name: "read_file".to_string(),
        args: serde_json::json!({}),
        agent_id: "explorer".to_string(),
        parent_request_id: "parent-1".to_string(),
    });
    handle.emit(AiEvent::SubAgentToolResult {
        request_id: "req-1".to_string(),
        tool_name: "read_file".to_string(),
        result: serde_json::json!("file contents"),
        success: true,
        agent_id: "explorer".to_string(),
        parent_request_id: "parent-1".to_string(),
    });

    // Two more events that should persist
    handle.emit(AiEvent::UserMessage {
        content: "test".to_string(),
    });
    handle.emit(AiEvent::Completed {
        response: "Hello".to_string(),
        reasoning: Some("thinking...".to_string()),
        input_tokens: Some(10),
        output_tokens: Some(5),
        duration_ms: Some(100),
    });
    flush_coordinator(&handle).await;

    let events = crate::transcript::read_transcript(temp_dir.path(), session_id)
        .await
        .unwrap();

    // Only Started, UserMessage, Completed should remain (3 of 7 emitted)
    assert_eq!(
        events.len(),
        3,
        "Expected 3 events; 4 filtered types should be excluded"
    );
    assert!(matches!(events[0].event, AiEvent::Started { .. }));
    assert!(matches!(events[1].event, AiEvent::UserMessage { .. }));
    assert!(matches!(events[2].event, AiEvent::Completed { .. }));
}

/// Tool events (ToolAutoApproved, ToolResult) must be persisted since they
/// are NOT in the filter list. This is important because the summarizer
/// needs them to reconstruct the conversation.
#[tokio::test]
async fn test_tool_events_persisted() {
    let runtime = Arc::new(MockRuntime::new());
    let temp_dir = tempfile::TempDir::new().unwrap();
    let session_id = "test-tool-events";
    let writer = create_test_writer(&temp_dir, session_id).await;

    let handle = EventCoordinator::spawn("test-session".to_string(), runtime, Some(writer));
    handle.mark_frontend_ready();

    handle.emit(AiEvent::ToolAutoApproved {
        request_id: "req-1".to_string(),
        tool_name: "read_file".to_string(),
        args: serde_json::json!({"path": "src/main.rs"}),
        reason: "Allowed by policy".to_string(),
        source: golish_core::events::ToolSource::Main,
    });
    handle.emit(AiEvent::ToolResult {
        tool_name: "read_file".to_string(),
        result: serde_json::json!("fn main() {}"),
        success: true,
        request_id: "req-1".to_string(),
        source: golish_core::events::ToolSource::Main,
    });
    flush_coordinator(&handle).await;

    let events = crate::transcript::read_transcript(temp_dir.path(), session_id)
        .await
        .unwrap();
    assert_eq!(events.len(), 2);
    assert!(matches!(events[0].event, AiEvent::ToolAutoApproved { .. }));
    assert!(matches!(events[1].event, AiEvent::ToolResult { .. }));
}
