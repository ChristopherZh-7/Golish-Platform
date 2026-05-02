use super::*;

#[test]
fn test_all_events_roundtrip() {
    let test_events = vec![
        AiEvent::Started {
            turn_id: "turn-1".to_string(),
        },
        AiEvent::TextDelta {
            delta: "Hello".to_string(),
            accumulated: "Hello world".to_string(),
        },
        AiEvent::ToolRequest {
            tool_name: "read_file".to_string(),
            args: json!({"path": "/test"}),
            request_id: "req-1".to_string(),
            source: ToolSource::Main,
        },
        AiEvent::ToolApprovalRequest {
            request_id: "req-2".to_string(),
            tool_name: "write_file".to_string(),
            args: json!({}),
            stats: None,
            risk_level: RiskLevel::High,
            can_learn: false,
            suggestion: None,
            source: ToolSource::Main,
        },
        AiEvent::ToolAutoApproved {
            request_id: "req-3".to_string(),
            tool_name: "read_file".to_string(),
            args: json!({}),
            reason: "Always allowed".to_string(),
            source: ToolSource::Main,
        },
        AiEvent::ToolDenied {
            request_id: "req-4".to_string(),
            tool_name: "shell".to_string(),
            args: json!({}),
            reason: "Blocked".to_string(),
            source: ToolSource::Main,
        },
        AiEvent::ToolResult {
            tool_name: "read_file".to_string(),
            result: json!("content"),
            success: true,
            request_id: "req-5".to_string(),
            source: ToolSource::Main,
        },
        AiEvent::Reasoning {
            content: "Thinking...".to_string(),
        },
        AiEvent::Completed {
            response: "Done".to_string(),
            reasoning: None,
            input_tokens: Some(60),
            output_tokens: Some(40),
            duration_ms: Some(500),
        },
        AiEvent::Error {
            message: "Failed".to_string(),
            error_type: "api".to_string(),
        },
        AiEvent::SubAgentStarted {
            agent_id: "a1".to_string(),
            agent_name: "analyzer".to_string(),
            task: "analyze".to_string(),
            depth: 1,
            parent_request_id: "parent-req-1".to_string(),
        },
        AiEvent::SubAgentToolRequest {
            agent_id: "a1".to_string(),
            tool_name: "read_file".to_string(),
            args: json!({}),
            request_id: "req-1".to_string(),
            parent_request_id: "parent-req-1".to_string(),
        },
        AiEvent::SubAgentToolResult {
            agent_id: "a1".to_string(),
            tool_name: "read_file".to_string(),
            success: true,
            result: json!({"content": "file contents"}),
            request_id: "req-1".to_string(),
            parent_request_id: "parent-req-1".to_string(),
        },
        AiEvent::SubAgentCompleted {
            agent_id: "a1".to_string(),
            response: "Done".to_string(),
            duration_ms: 1000,
            parent_request_id: "parent-req-1".to_string(),
        },
        AiEvent::SubAgentError {
            agent_id: "a1".to_string(),
            error: "Failed".to_string(),
            parent_request_id: "parent-req-1".to_string(),
        },
        AiEvent::ContextWarning {
            utilization: 0.85,
            total_tokens: 170000,
            max_tokens: 200000,
        },
        AiEvent::ToolResponseTruncated {
            tool_name: "read_file".to_string(),
            original_tokens: 50000,
            truncated_tokens: 10000,
        },
        AiEvent::LoopWarning {
            tool_name: "list".to_string(),
            current_count: 8,
            max_count: 10,
            message: "Warning".to_string(),
        },
        AiEvent::LoopBlocked {
            tool_name: "list".to_string(),
            repeat_count: 10,
            max_count: 10,
            message: "Blocked".to_string(),
        },
        AiEvent::MaxIterationsReached {
            iterations: 50,
            max_iterations: 50,
            message: "Max reached".to_string(),
        },
        AiEvent::WorkflowStarted {
            workflow_id: "wf1".to_string(),
            workflow_name: "git_commit".to_string(),
            session_id: "s1".to_string(),
        },
        AiEvent::WorkflowStepStarted {
            workflow_id: "wf1".to_string(),
            step_name: "analyze".to_string(),
            step_index: 0,
            total_steps: 4,
        },
        AiEvent::WorkflowStepCompleted {
            workflow_id: "wf1".to_string(),
            step_name: "analyze".to_string(),
            output: Some("Done".to_string()),
            duration_ms: 1000,
        },
        AiEvent::WorkflowCompleted {
            workflow_id: "wf1".to_string(),
            final_output: "Complete".to_string(),
            total_duration_ms: 5000,
        },
        AiEvent::WorkflowError {
            workflow_id: "wf1".to_string(),
            step_name: Some("commit".to_string()),
            error: "Failed".to_string(),
        },
        AiEvent::PlanUpdated {
            version: 1,
            summary: PlanSummary {
                total: 2,
                completed: 0,
                in_progress: 1,
                pending: 1,
            },
            steps: vec![
                PlanStep {
                    id: Some("step-x".into()),
                    step: "Step 1".to_string(),
                    status: StepStatus::InProgress,
                },
                PlanStep {
                    id: Some("step-y".into()),
                    step: "Step 2".to_string(),
                    status: StepStatus::Pending,
                },
            ],
            explanation: None,
        },
    ];

    for event in test_events {
        // Serialize to JSON string
        let json_str = serde_json::to_string(&event).expect("serialize failed");

        // Deserialize back to AiEvent
        let roundtrip: AiEvent = serde_json::from_str(&json_str).expect("deserialize failed");

        // Verify roundtrip produces identical JSON
        let original_json = serde_json::to_value(&event).unwrap();
        let roundtrip_json = serde_json::to_value(&roundtrip).unwrap();

        assert_eq!(
            original_json,
            roundtrip_json,
            "Roundtrip failed for event type: {}",
            event.event_type()
        );
    }
}

/// Test deserialization from actual JSON strings (simulating frontend)
#[test]
fn test_deserialize_from_json_strings() {
    // Started event
    let json = r#"{"type":"started","turn_id":"turn-123"}"#;
    let event: AiEvent = serde_json::from_str(json).expect("failed to deserialize Started");
    match event {
        AiEvent::Started { turn_id } => assert_eq!(turn_id, "turn-123"),
        _ => panic!("Wrong event type"),
    }

    // TextDelta event
    let json = r#"{"type":"text_delta","delta":"Hi","accumulated":"Hi there"}"#;
    let event: AiEvent = serde_json::from_str(json).expect("failed to deserialize TextDelta");
    match event {
        AiEvent::TextDelta { delta, accumulated } => {
            assert_eq!(delta, "Hi");
            assert_eq!(accumulated, "Hi there");
        }
        _ => panic!("Wrong event type"),
    }

    // Error event
    let json = r#"{"type":"error","message":"Something went wrong","error_type":"unknown"}"#;
    let event: AiEvent = serde_json::from_str(json).expect("failed to deserialize Error");
    match event {
        AiEvent::Error {
            message,
            error_type,
        } => {
            assert_eq!(message, "Something went wrong");
            assert_eq!(error_type, "unknown");
        }
        _ => panic!("Wrong event type"),
    }
}
