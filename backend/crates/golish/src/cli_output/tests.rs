use super::formatting;
use super::{convert_to_cli_json, truncate};

#[test]
fn test_truncate() {
    assert_eq!(truncate("hello", 10), "hello");
    assert_eq!(truncate("hello world!", 8), "hello...");
}

#[test]
fn test_format_args_summary() {
    let short = serde_json::json!({"path": "/tmp"});
    assert_eq!(formatting::format_args_summary(&short), r#"{"path":"/tmp"}"#);

    let long = serde_json::json!({
        "path": "/very/long/path/to/some/file/that/exceeds/the/limit.txt",
        "content": "some content"
    });
    let summary = formatting::format_args_summary(&long);
    assert!(summary.len() <= 63); // 60 + "..."
    assert!(summary.ends_with("..."));
}

// ────────────────────────────────────────────────────────────────────────────────
// Tests for new helper functions
// ────────────────────────────────────────────────────────────────────────────────

mod format_json_pretty_tests {
    use super::formatting::format_json_pretty;

    #[test]
    fn formats_simple_object() {
        let value = serde_json::json!({"path": "Cargo.toml"});
        let pretty = format_json_pretty(&value);
        assert!(pretty.contains("\"path\""));
        assert!(pretty.contains("\"Cargo.toml\""));
        // Should be multi-line pretty format
        assert!(pretty.contains('\n'));
    }

    #[test]
    fn formats_nested_object() {
        let value = serde_json::json!({
            "path": "src/main.rs",
            "options": {
                "recursive": true,
                "limit": 100
            }
        });
        let pretty = format_json_pretty(&value);
        assert!(pretty.contains("\"path\""));
        assert!(pretty.contains("\"recursive\""));
        assert!(pretty.contains("true"));
    }

    #[test]
    fn formats_string_value() {
        let value = serde_json::json!("just a string");
        let pretty = format_json_pretty(&value);
        assert_eq!(pretty, "\"just a string\"");
    }

    #[test]
    fn formats_null_value() {
        let value = serde_json::Value::Null;
        let pretty = format_json_pretty(&value);
        assert_eq!(pretty, "null");
    }
}

mod truncate_output_tests {
    use super::formatting::truncate_output;

    #[test]
    fn does_not_truncate_short_string() {
        let short = "Hello, world!";
        let result = truncate_output(short, 500);
        assert_eq!(result, short);
    }

    #[test]
    fn truncates_long_string_at_limit() {
        let long = "a".repeat(1000);
        let result = truncate_output(&long, 500);
        assert_eq!(result.len(), 500);
        assert!(result.chars().all(|c| c == 'a'));
    }

    #[test]
    fn handles_exact_length_string() {
        let exact = "x".repeat(500);
        let result = truncate_output(&exact, 500);
        assert_eq!(result.len(), 500);
    }

    #[test]
    fn handles_empty_string() {
        let result = truncate_output("", 500);
        assert_eq!(result, "");
    }

    #[test]
    fn handles_unicode_correctly() {
        // Unicode characters can be multi-byte, ensure we don't split mid-character
        let unicode = "Hello ";
        let result = truncate_output(unicode, 10);
        // Should handle gracefully (either truncate cleanly or include full char)
        assert!(!result.is_empty());
    }
}

// ────────────────────────────────────────────────────────────────────────────────
// Tests for CliJsonEvent standardized format
// ────────────────────────────────────────────────────────────────────────────────

mod cli_json_event_tests {
    use super::convert_to_cli_json;
    use golish_core::events::AiEvent;
    use golish_core::hitl::RiskLevel;

    #[test]
    fn started_event_has_correct_format() {
        let ai_event = AiEvent::Started {
            turn_id: "test-turn-123".to_string(),
        };
        let cli_json = convert_to_cli_json(&ai_event);
        let json_str = serde_json::to_string(&cli_json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed["event"], "started");
        assert!(parsed["timestamp"].as_u64().is_some());
        assert_eq!(parsed["turn_id"], "test-turn-123");
    }

    #[test]
    fn text_delta_event_has_correct_format() {
        let ai_event = AiEvent::TextDelta {
            delta: "Hello".to_string(),
            accumulated: "Hello World".to_string(),
        };
        let cli_json = convert_to_cli_json(&ai_event);
        let json_str = serde_json::to_string(&cli_json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed["event"], "text_delta");
        assert!(parsed["timestamp"].as_u64().is_some());
        assert_eq!(parsed["delta"], "Hello");
        assert_eq!(parsed["accumulated"], "Hello World");
    }

    #[test]
    fn tool_request_uses_input_not_args() {
        let ai_event = AiEvent::ToolRequest {
            tool_name: "read_file".to_string(),
            args: serde_json::json!({"path": "Cargo.toml"}),
            request_id: "req-123".to_string(),
            source: Default::default(),
        };
        let cli_json = convert_to_cli_json(&ai_event);
        let json_str = serde_json::to_string(&cli_json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed["event"], "tool_call");
        assert_eq!(parsed["tool_name"], "read_file");
        // Should use "input" not "args"
        assert_eq!(parsed["input"]["path"], "Cargo.toml");
        assert!(parsed.get("args").is_none());
    }

    #[test]
    fn tool_result_uses_output_not_result() {
        let ai_event = AiEvent::ToolResult {
            tool_name: "read_file".to_string(),
            result: serde_json::json!("[package]\nname = \"golish\""),
            success: true,
            request_id: "req-123".to_string(),
            source: Default::default(),
        };
        let cli_json = convert_to_cli_json(&ai_event);
        let json_str = serde_json::to_string(&cli_json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed["event"], "tool_result");
        assert_eq!(parsed["tool_name"], "read_file");
        assert_eq!(parsed["success"], true);
        // Should use "output" not "result"
        assert_eq!(parsed["output"], "[package]\nname = \"golish\"");
        assert!(parsed.get("result").is_none());
    }

    #[test]
    fn tool_approval_request_uses_input_not_args() {
        let ai_event = AiEvent::ToolApprovalRequest {
            request_id: "req-456".to_string(),
            tool_name: "write_file".to_string(),
            args: serde_json::json!({"path": "/tmp/test.txt", "content": "hello"}),
            stats: None,
            risk_level: RiskLevel::High,
            can_learn: true,
            suggestion: Some("Approve this operation?".to_string()),
            source: Default::default(),
        };
        let cli_json = convert_to_cli_json(&ai_event);
        let json_str = serde_json::to_string(&cli_json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed["event"], "tool_approval");
        assert_eq!(parsed["tool_name"], "write_file");
        // Should use "input" not "args"
        assert_eq!(parsed["input"]["path"], "/tmp/test.txt");
        assert!(parsed.get("args").is_none());
    }

    #[test]
    fn reasoning_event_has_correct_format() {
        let ai_event = AiEvent::Reasoning {
            content: "Let me think about this step by step...".to_string(),
        };
        let cli_json = convert_to_cli_json(&ai_event);
        let json_str = serde_json::to_string(&cli_json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed["event"], "reasoning");
        assert_eq!(parsed["content"], "Let me think about this step by step...");
    }

    #[test]
    fn completed_event_has_correct_format() {
        let ai_event = AiEvent::Completed {
            response: "Here is the answer".to_string(),
            reasoning: None,
            input_tokens: Some(100),
            output_tokens: Some(50),
            duration_ms: Some(1234),
        };
        let cli_json = convert_to_cli_json(&ai_event);
        let json_str = serde_json::to_string(&cli_json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed["event"], "completed");
        assert_eq!(parsed["response"], "Here is the answer");
        assert_eq!(parsed["input_tokens"], 100);
        assert_eq!(parsed["output_tokens"], 50);
        assert_eq!(parsed["duration_ms"], 1234);
    }

    #[test]
    fn error_event_has_correct_format() {
        let ai_event = AiEvent::Error {
            message: "Something went wrong".to_string(),
            error_type: "api_error".to_string(),
        };
        let cli_json = convert_to_cli_json(&ai_event);
        let json_str = serde_json::to_string(&cli_json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed["event"], "error");
        assert_eq!(parsed["message"], "Something went wrong");
        assert_eq!(parsed["error_type"], "api_error");
    }

    #[test]
    fn all_events_have_timestamp() {
        let events = vec![
            AiEvent::Started {
                turn_id: "t1".to_string(),
            },
            AiEvent::TextDelta {
                delta: "x".to_string(),
                accumulated: "x".to_string(),
            },
            AiEvent::Reasoning {
                content: "thinking".to_string(),
            },
        ];

        for ai_event in events {
            let cli_json = convert_to_cli_json(&ai_event);
            let json_str = serde_json::to_string(&cli_json).unwrap();
            let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

            assert!(
                parsed["timestamp"].as_u64().is_some(),
                "Event should have timestamp: {:?}",
                parsed
            );
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────────
// Tests for NO TRUNCATION in JSON mode
// ────────────────────────────────────────────────────────────────────────────────

mod json_no_truncation_tests {
    use super::convert_to_cli_json;
    use golish_core::events::AiEvent;

    #[test]
    fn tool_output_not_truncated_in_json() {
        // Create a very large tool result (> 500 chars which is terminal limit)
        let large_output = "x".repeat(10000);
        let ai_event = AiEvent::ToolResult {
            tool_name: "read_file".to_string(),
            result: serde_json::json!(large_output),
            success: true,
            request_id: "req-large".to_string(),
            source: Default::default(),
        };
        let cli_json = convert_to_cli_json(&ai_event);
        let json_str = serde_json::to_string(&cli_json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        // Output should be FULL 10000 chars, NOT truncated
        let output = parsed["output"].as_str().unwrap();
        assert_eq!(output.len(), 10000, "JSON output should NOT be truncated");
        assert!(
            !output.contains("truncated"),
            "Should not contain truncation indicator"
        );
    }

    #[test]
    fn reasoning_not_truncated_in_json() {
        // Create very large reasoning content (> 2000 chars which is terminal limit)
        let large_reasoning = "thinking step ".repeat(500);
        let ai_event = AiEvent::Reasoning {
            content: large_reasoning.clone(),
        };
        let cli_json = convert_to_cli_json(&ai_event);
        let json_str = serde_json::to_string(&cli_json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        // Content should be FULL, NOT truncated
        let content = parsed["content"].as_str().unwrap();
        assert_eq!(
            content.len(),
            large_reasoning.len(),
            "JSON reasoning should NOT be truncated"
        );
        assert!(
            !content.contains("truncated"),
            "Should not contain truncation indicator"
        );
    }

    #[test]
    fn tool_input_not_truncated_in_json() {
        // Create a very large tool input
        let large_content = "y".repeat(5000);
        let ai_event = AiEvent::ToolRequest {
            tool_name: "write_file".to_string(),
            args: serde_json::json!({
                "path": "/tmp/test.txt",
                "content": large_content
            }),
            request_id: "req-large-input".to_string(),
            source: Default::default(),
        };
        let cli_json = convert_to_cli_json(&ai_event);
        let json_str = serde_json::to_string(&cli_json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        // Input content should be FULL 5000 chars
        let input_content = parsed["input"]["content"].as_str().unwrap();
        assert_eq!(
            input_content.len(),
            5000,
            "JSON tool input should NOT be truncated"
        );
    }
}
