use super::*;

/// Verifies that formatting an empty event list returns an empty string.
#[test]
fn test_format_empty_events() {
    let events: Vec<TranscriptEvent> = vec![];
    let result = format_for_summarizer(&events);
    assert!(result.is_empty());
}

/// Verifies that a simple conversation formats correctly with turn numbers and token counts.
#[test]
fn test_format_simple_conversation() {
    let events = vec![
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::Started {
                turn_id: "turn-1".to_string(),
            },
        },
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::Completed {
                response: "I'll help you with that.".to_string(),
                reasoning: None,
                input_tokens: Some(100),
                output_tokens: Some(50),
                duration_ms: Some(1000),
            },
        },
    ];

    let result = format_for_summarizer(&events);

    assert!(result.contains("[turn 001]"));
    assert!(result.contains("ASSISTANT"));
    assert!(result.contains("I'll help you with that."));
    assert!(result.contains("100 in / 50 out tokens"));
}

/// Verifies that tool requests and results are included in the output.
#[test]
fn test_format_includes_tool_calls() {
    let events = vec![
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::Started {
                turn_id: "turn-1".to_string(),
            },
        },
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::ToolRequest {
                tool_name: "read_file".to_string(),
                args: serde_json::json!({"path": "/src/main.rs"}),
                request_id: "req-1".to_string(),
                source: Default::default(),
            },
        },
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::ToolResult {
                tool_name: "read_file".to_string(),
                result: serde_json::json!({"content": "fn main() {}"}),
                success: true,
                request_id: "req-1".to_string(),
                source: Default::default(),
            },
        },
    ];

    let result = format_for_summarizer(&events);

    assert!(result.contains("TOOL_REQUEST"));
    assert!(result.contains("read_file"));
    assert!(result.contains("TOOL_RESULT"));
}

/// Verifies that user messages are included in the output.
#[test]
fn test_format_includes_user_message() {
    let events = vec![
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::Started {
                turn_id: "turn-1".to_string(),
            },
        },
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::UserMessage {
                content: "Please help me debug this.".to_string(),
            },
        },
    ];

    let result = format_for_summarizer(&events);

    assert!(result.contains("USER:"));
    assert!(result.contains("Please help me debug this."));
}

/// Verifies that TextDelta events are skipped (only Completed response is included).
#[test]
fn test_format_excludes_text_delta() {
    let events = vec![
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::TextDelta {
                delta: "Hello".to_string(),
                accumulated: "Hello".to_string(),
            },
        },
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::TextDelta {
                delta: " world".to_string(),
                accumulated: "Hello world".to_string(),
            },
        },
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::Started {
                turn_id: "turn-1".to_string(),
            },
        },
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::Completed {
                response: "Hello world".to_string(),
                reasoning: None,
                input_tokens: Some(100),
                output_tokens: Some(50),
                duration_ms: Some(1000),
            },
        },
    ];

    let result = format_for_summarizer(&events);

    // Should only have final response, not streaming deltas
    let hello_count = result.matches("Hello world").count();
    assert_eq!(
        hello_count, 1,
        "Should only have final response, not streaming deltas"
    );
}

/// Verifies that turn numbers increment correctly across multiple turns.
#[test]
fn test_format_tracks_turn_numbers() {
    let events = vec![
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::Started {
                turn_id: "turn-1".to_string(),
            },
        },
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::Completed {
                response: "First response".to_string(),
                reasoning: None,
                input_tokens: Some(100),
                output_tokens: Some(50),
                duration_ms: Some(1000),
            },
        },
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::Started {
                turn_id: "turn-2".to_string(),
            },
        },
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::Completed {
                response: "Second response".to_string(),
                reasoning: None,
                input_tokens: Some(150),
                output_tokens: Some(75),
                duration_ms: Some(1200),
            },
        },
    ];

    let result = format_for_summarizer(&events);

    assert!(result.contains("[turn 001]"));
    assert!(result.contains("[turn 002]"));
}

/// Verifies that very long tool results are truncated.
#[test]
fn test_format_truncates_long_tool_results() {
    // Create a result that's > 4000 chars
    let long_content = "x".repeat(5000);
    let events = vec![
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::Started {
                turn_id: "turn-1".to_string(),
            },
        },
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::ToolResult {
                tool_name: "read_file".to_string(),
                result: serde_json::json!(long_content),
                success: true,
                request_id: "req-1".to_string(),
                source: Default::default(),
            },
        },
    ];

    let result = format_for_summarizer(&events);

    assert!(result.contains("truncated"));
    // Head+tail strategy: should contain content from the end too
    assert!(result.contains("xxx")); // tail portion preserved
}

/// Verifies that error events are included in the output.
#[test]
fn test_format_includes_errors() {
    let events = vec![
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::Started {
                turn_id: "turn-1".to_string(),
            },
        },
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::Error {
                message: "Connection timeout".to_string(),
                error_type: "network".to_string(),
            },
        },
    ];

    let result = format_for_summarizer(&events);

    assert!(result.contains("ERROR"));
    assert!(result.contains("network"));
    assert!(result.contains("Connection timeout"));
}

/// Verifies that sub-agent events are included.
#[test]
fn test_format_includes_sub_agent_events() {
    let events = vec![
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::Started {
                turn_id: "turn-1".to_string(),
            },
        },
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::SubAgentStarted {
                agent_id: "agent-001".to_string(),
                agent_name: "analyzer".to_string(),
                task: "Analyze the codebase".to_string(),
                depth: 1,
                parent_request_id: "parent-1".to_string(),
            },
        },
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::SubAgentCompleted {
                agent_id: "agent-001".to_string(),
                response: "Analysis complete".to_string(),
                duration_ms: 5000,
                parent_request_id: "parent-1".to_string(),
            },
        },
    ];

    let result = format_for_summarizer(&events);

    assert!(result.contains("SUB_AGENT_STARTED"));
    assert!(result.contains("analyzer"));
    assert!(result.contains("Analyze the codebase"));
    assert!(result.contains("SUB_AGENT_COMPLETED"));
    assert!(result.contains("Analysis complete"));
}

/// Verifies that tool approval events are included.
#[test]
fn test_format_includes_tool_approval_events() {
    use golish_core::hitl::RiskLevel;

    let events = vec![
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::Started {
                turn_id: "turn-1".to_string(),
            },
        },
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::ToolApprovalRequest {
                request_id: "req-1".to_string(),
                tool_name: "write_file".to_string(),
                args: serde_json::json!({"path": "/src/lib.rs"}),
                stats: None,
                risk_level: RiskLevel::Medium,
                can_learn: true,
                suggestion: None,
                source: Default::default(),
            },
        },
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::ToolAutoApproved {
                request_id: "req-2".to_string(),
                tool_name: "read_file".to_string(),
                args: serde_json::json!({}),
                reason: "Always allowed".to_string(),
                source: Default::default(),
            },
        },
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::ToolDenied {
                request_id: "req-3".to_string(),
                tool_name: "shell_exec".to_string(),
                args: serde_json::json!({}),
                reason: "Dangerous command".to_string(),
                source: Default::default(),
            },
        },
    ];

    let result = format_for_summarizer(&events);

    assert!(result.contains("TOOL_APPROVAL_REQUEST"));
    assert!(result.contains("Medium"));
    assert!(result.contains("TOOL_AUTO_APPROVED"));
    assert!(result.contains("Always allowed"));
    assert!(result.contains("TOOL_DENIED"));
    assert!(result.contains("Dangerous command"));
}

/// Verifies that internal events like context warnings are skipped.
#[test]
fn test_format_excludes_internal_events() {
    let events = vec![
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::Started {
                turn_id: "turn-1".to_string(),
            },
        },
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::ContextWarning {
                utilization: 0.85,
                total_tokens: 170000,
                max_tokens: 200000,
            },
        },
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::LoopWarning {
                tool_name: "read_file".to_string(),
                current_count: 8,
                max_count: 10,
                message: "Approaching limit".to_string(),
            },
        },
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::Reasoning {
                content: "Let me think...".to_string(),
            },
        },
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::Completed {
                response: "Done".to_string(),
                reasoning: None,
                input_tokens: Some(100),
                output_tokens: Some(50),
                duration_ms: Some(1000),
            },
        },
    ];

    let result = format_for_summarizer(&events);

    // These should NOT appear
    assert!(!result.contains("ContextWarning"));
    assert!(!result.contains("utilization"));
    assert!(!result.contains("LoopWarning"));
    assert!(!result.contains("Approaching limit"));
    assert!(!result.contains("Let me think"));

    // But the final response should appear
    assert!(result.contains("Done"));
}

/// Verifies that reasoning/thinking from Completed events is excluded from summarizer output.
/// Reasoning is the model's internal chain-of-thought and is already reflected in the response.
#[test]
fn test_format_excludes_reasoning_from_completed() {
    let events = vec![
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::Started {
                turn_id: "turn-1".to_string(),
            },
        },
        TranscriptEvent {
            timestamp: Utc::now(),
            event: AiEvent::Completed {
                response: "The fix is to add a null check.".to_string(),
                reasoning: Some("Let me analyze this carefully. The bug is caused by a null pointer dereference in the handler function. I should suggest adding a null check.".to_string()),
                input_tokens: Some(500),
                output_tokens: Some(100),
                duration_ms: Some(3000),
            },
        },
    ];

    let result = format_for_summarizer(&events);

    // The assistant response should appear
    assert!(result.contains("The fix is to add a null check."));
    assert!(result.contains("ASSISTANT"));

    // The reasoning/thinking should NOT appear
    assert!(!result.contains("THINKING"));
    assert!(!result.contains("Let me analyze this carefully"));
    assert!(!result.contains("null pointer dereference"));
}
