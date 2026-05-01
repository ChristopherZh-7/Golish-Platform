use super::*;

#[test]
fn started_event_json_format() {
    let event = AiEvent::Started {
        turn_id: "turn-123".to_string(),
    };
    let json = serde_json::to_value(&event).unwrap();

    assert_eq!(json["type"], "started");
    assert_eq!(json["turn_id"], "turn-123");

    let expected = json!({
        "type": "started",
        "turn_id": "turn-123"
    });
    assert_eq!(json, expected);
}

#[test]
fn text_delta_event_json_format() {
    let event = AiEvent::TextDelta {
        delta: "Hello".to_string(),
        accumulated: "Hello world".to_string(),
    };
    let json = serde_json::to_value(&event).unwrap();

    assert_eq!(json["type"], "text_delta");
    assert_eq!(json["delta"], "Hello");
    assert_eq!(json["accumulated"], "Hello world");

    let expected = json!({
        "type": "text_delta",
        "delta": "Hello",
        "accumulated": "Hello world"
    });
    assert_eq!(json, expected);
}

#[test]
fn tool_request_event_json_format() {
    let event = AiEvent::ToolRequest {
        tool_name: "read_file".to_string(),
        args: json!({"path": "/src/main.rs"}),
        request_id: "req-456".to_string(),
        source: ToolSource::Main,
    };
    let json = serde_json::to_value(&event).unwrap();

    assert_eq!(json["type"], "tool_request");
    assert_eq!(json["tool_name"], "read_file");
    assert_eq!(json["args"]["path"], "/src/main.rs");
    assert_eq!(json["request_id"], "req-456");
    assert_eq!(json["source"]["type"], "main");
}

#[test]
fn tool_approval_request_event_json_format() {
    use chrono::{DateTime, Utc};
    use crate::hitl::{ApprovalPattern, RiskLevel};

    let event = AiEvent::ToolApprovalRequest {
        request_id: "req-789".to_string(),
        tool_name: "write_file".to_string(),
        args: json!({"path": "/src/lib.rs", "content": "// code"}),
        stats: Some(ApprovalPattern {
            tool_name: "write_file".to_string(),
            total_requests: 5,
            approvals: 4,
            denials: 1,
            always_allow: false,
            last_updated: DateTime::<Utc>::from_timestamp(1700000000, 0).unwrap(),
            justifications: vec!["User approved".to_string()],
        }),
        risk_level: RiskLevel::Medium,
        can_learn: true,
        suggestion: Some("1 more approval for auto-approve".to_string()),
        source: ToolSource::Main,
    };
    let json = serde_json::to_value(&event).unwrap();

    assert_eq!(json["type"], "tool_approval_request");
    assert_eq!(json["request_id"], "req-789");
    assert_eq!(json["tool_name"], "write_file");
    assert_eq!(json["risk_level"], "medium");
    assert_eq!(json["can_learn"], true);
    assert_eq!(json["stats"]["total_requests"], 5);
    assert_eq!(json["stats"]["approvals"], 4);
}

#[test]
fn tool_auto_approved_event_json_format() {
    let event = AiEvent::ToolAutoApproved {
        request_id: "req-auto-1".to_string(),
        tool_name: "read_file".to_string(),
        args: json!({"path": "/readme.md"}),
        reason: "Always allowed by user".to_string(),
        source: ToolSource::Main,
    };
    let json = serde_json::to_value(&event).unwrap();

    assert_eq!(json["type"], "tool_auto_approved");
    assert_eq!(json["request_id"], "req-auto-1");
    assert_eq!(json["tool_name"], "read_file");
    assert_eq!(json["reason"], "Always allowed by user");
}

#[test]
fn tool_denied_event_json_format() {
    let event = AiEvent::ToolDenied {
        request_id: "req-denied-1".to_string(),
        tool_name: "shell_exec".to_string(),
        args: json!({"command": "rm -rf /"}),
        reason: "Dangerous command blocked".to_string(),
        source: ToolSource::Main,
    };
    let json = serde_json::to_value(&event).unwrap();

    assert_eq!(json["type"], "tool_denied");
    assert_eq!(json["request_id"], "req-denied-1");
    assert_eq!(json["tool_name"], "shell_exec");
    assert_eq!(json["reason"], "Dangerous command blocked");
}

#[test]
fn tool_result_event_json_format() {
    let event = AiEvent::ToolResult {
        tool_name: "read_file".to_string(),
        result: json!({"content": "file contents here"}),
        success: true,
        request_id: "req-result-1".to_string(),
        source: ToolSource::Main,
    };
    let json = serde_json::to_value(&event).unwrap();

    assert_eq!(json["type"], "tool_result");
    assert_eq!(json["tool_name"], "read_file");
    assert_eq!(json["success"], true);
    assert_eq!(json["request_id"], "req-result-1");
    assert_eq!(json["result"]["content"], "file contents here");
}

#[test]
fn reasoning_event_json_format() {
    let event = AiEvent::Reasoning {
        content: "Let me think about this...".to_string(),
    };
    let json = serde_json::to_value(&event).unwrap();

    assert_eq!(json["type"], "reasoning");
    assert_eq!(json["content"], "Let me think about this...");

    let expected = json!({
        "type": "reasoning",
        "content": "Let me think about this..."
    });
    assert_eq!(json, expected);
}

#[test]
fn completed_event_json_format() {
    let event = AiEvent::Completed {
        response: "Task completed successfully.".to_string(),
        reasoning: Some("Let me think about this...".to_string()),
        input_tokens: Some(1000),
        output_tokens: Some(500),
        duration_ms: Some(2500),
    };
    let json = serde_json::to_value(&event).unwrap();

    assert_eq!(json["type"], "completed");
    assert_eq!(json["response"], "Task completed successfully.");
    assert_eq!(json["reasoning"], "Let me think about this...");
    assert_eq!(json["input_tokens"], 1000);
    assert_eq!(json["output_tokens"], 500);
    assert_eq!(json["duration_ms"], 2500);
}

#[test]
fn completed_event_with_null_fields() {
    let event = AiEvent::Completed {
        response: "Done".to_string(),
        reasoning: None,
        input_tokens: None,
        output_tokens: None,
        duration_ms: None,
    };
    let json = serde_json::to_value(&event).unwrap();

    assert_eq!(json["type"], "completed");
    assert_eq!(json["response"], "Done");
    // reasoning should be omitted (skip_serializing_if = None)
    assert!(json.get("reasoning").is_none());
    assert!(json["input_tokens"].is_null());
    assert!(json["output_tokens"].is_null());
    assert!(json["duration_ms"].is_null());
}

#[test]
fn error_event_json_format() {
    let event = AiEvent::Error {
        message: "Connection timeout".to_string(),
        error_type: "network".to_string(),
    };
    let json = serde_json::to_value(&event).unwrap();

    assert_eq!(json["type"], "error");
    assert_eq!(json["message"], "Connection timeout");
    assert_eq!(json["error_type"], "network");

    let expected = json!({
        "type": "error",
        "message": "Connection timeout",
        "error_type": "network"
    });
    assert_eq!(json, expected);
}

#[test]
fn sub_agent_started_event_json_format() {
    let event = AiEvent::SubAgentStarted {
        agent_id: "agent-001".to_string(),
        agent_name: "analyzer".to_string(),
        task: "Analyze the codebase structure".to_string(),
        depth: 1,
        parent_request_id: "parent-req-001".to_string(),
    };
    let json = serde_json::to_value(&event).unwrap();

    assert_eq!(json["type"], "sub_agent_started");
    assert_eq!(json["agent_id"], "agent-001");
    assert_eq!(json["agent_name"], "analyzer");
    assert_eq!(json["task"], "Analyze the codebase structure");
    assert_eq!(json["depth"], 1);
    assert_eq!(json["parent_request_id"], "parent-req-001");
}

#[test]
fn sub_agent_completed_event_json_format() {
    let event = AiEvent::SubAgentCompleted {
        agent_id: "agent-001".to_string(),
        response: "Analysis complete".to_string(),
        duration_ms: 5000,
        parent_request_id: "parent-req-001".to_string(),
    };
    let json = serde_json::to_value(&event).unwrap();

    assert_eq!(json["type"], "sub_agent_completed");
    assert_eq!(json["agent_id"], "agent-001");
    assert_eq!(json["response"], "Analysis complete");
    assert_eq!(json["duration_ms"], 5000);
    assert_eq!(json["parent_request_id"], "parent-req-001");
}

#[test]
fn context_warning_event_json_format() {
    let event = AiEvent::ContextWarning {
        utilization: 0.85,
        total_tokens: 170000,
        max_tokens: 200000,
    };
    let json = serde_json::to_value(&event).unwrap();

    assert_eq!(json["type"], "context_warning");
    assert_eq!(json["utilization"], 0.85);
    assert_eq!(json["total_tokens"], 170000);
    assert_eq!(json["max_tokens"], 200000);
}

#[test]
fn loop_warning_event_json_format() {
    let event = AiEvent::LoopWarning {
        tool_name: "list_files".to_string(),
        current_count: 8,
        max_count: 10,
        message: "Approaching loop limit".to_string(),
    };
    let json = serde_json::to_value(&event).unwrap();

    assert_eq!(json["type"], "loop_warning");
    assert_eq!(json["tool_name"], "list_files");
    assert_eq!(json["current_count"], 8);
    assert_eq!(json["max_count"], 10);
}

#[test]
fn loop_blocked_event_json_format() {
    let event = AiEvent::LoopBlocked {
        tool_name: "list_files".to_string(),
        repeat_count: 10,
        max_count: 10,
        message: "Loop detected, blocking further calls".to_string(),
    };
    let json = serde_json::to_value(&event).unwrap();

    assert_eq!(json["type"], "loop_blocked");
    assert_eq!(json["tool_name"], "list_files");
    assert_eq!(json["repeat_count"], 10);
    assert_eq!(json["max_count"], 10);
}

#[test]
fn max_iterations_reached_event_json_format() {
    let event = AiEvent::MaxIterationsReached {
        iterations: 50,
        max_iterations: 50,
        message: "Maximum tool iterations reached".to_string(),
    };
    let json = serde_json::to_value(&event).unwrap();

    assert_eq!(json["type"], "max_iterations_reached");
    assert_eq!(json["iterations"], 50);
    assert_eq!(json["max_iterations"], 50);
}

#[test]
fn workflow_started_event_json_format() {
    let event = AiEvent::WorkflowStarted {
        workflow_id: "wf-001".to_string(),
        workflow_name: "git_commit".to_string(),
        session_id: "session-123".to_string(),
    };
    let json = serde_json::to_value(&event).unwrap();

    assert_eq!(json["type"], "workflow_started");
    assert_eq!(json["workflow_id"], "wf-001");
    assert_eq!(json["workflow_name"], "git_commit");
    assert_eq!(json["session_id"], "session-123");
}

#[test]
fn workflow_step_started_event_json_format() {
    let event = AiEvent::WorkflowStepStarted {
        workflow_id: "wf-001".to_string(),
        step_name: "analyze".to_string(),
        step_index: 0,
        total_steps: 4,
    };
    let json = serde_json::to_value(&event).unwrap();

    assert_eq!(json["type"], "workflow_step_started");
    assert_eq!(json["workflow_id"], "wf-001");
    assert_eq!(json["step_name"], "analyze");
    assert_eq!(json["step_index"], 0);
    assert_eq!(json["total_steps"], 4);
}

#[test]
fn workflow_step_completed_event_json_format() {
    let event = AiEvent::WorkflowStepCompleted {
        workflow_id: "wf-001".to_string(),
        step_name: "analyze".to_string(),
        output: Some("Analysis complete".to_string()),
        duration_ms: 1500,
    };
    let json = serde_json::to_value(&event).unwrap();

    assert_eq!(json["type"], "workflow_step_completed");
    assert_eq!(json["workflow_id"], "wf-001");
    assert_eq!(json["step_name"], "analyze");
    assert_eq!(json["output"], "Analysis complete");
    assert_eq!(json["duration_ms"], 1500);
}

#[test]
fn workflow_completed_event_json_format() {
    let event = AiEvent::WorkflowCompleted {
        workflow_id: "wf-001".to_string(),
        final_output: "Commit created successfully".to_string(),
        total_duration_ms: 8500,
    };
    let json = serde_json::to_value(&event).unwrap();

    assert_eq!(json["type"], "workflow_completed");
    assert_eq!(json["workflow_id"], "wf-001");
    assert_eq!(json["final_output"], "Commit created successfully");
    assert_eq!(json["total_duration_ms"], 8500);
}

#[test]
fn workflow_error_event_json_format() {
    let event = AiEvent::WorkflowError {
        workflow_id: "wf-001".to_string(),
        step_name: Some("commit".to_string()),
        error: "Git commit failed".to_string(),
    };
    let json = serde_json::to_value(&event).unwrap();

    assert_eq!(json["type"], "workflow_error");
    assert_eq!(json["workflow_id"], "wf-001");
    assert_eq!(json["step_name"], "commit");
    assert_eq!(json["error"], "Git commit failed");
}

#[test]
fn tool_response_truncated_event_json_format() {
    let event = AiEvent::ToolResponseTruncated {
        tool_name: "read_file".to_string(),
        original_tokens: 50000,
        truncated_tokens: 10000,
    };
    let json = serde_json::to_value(&event).unwrap();

    assert_eq!(json["type"], "tool_response_truncated");
    assert_eq!(json["tool_name"], "read_file");
    assert_eq!(json["original_tokens"], 50000);
    assert_eq!(json["truncated_tokens"], 10000);
}
