use super::*;
use super::summarizer::*;
use golish_core::events::AiEvent;

mod writer_tests {
    use super::*;
    use serde_json::Value;
    use std::sync::Arc;
    use tempfile::TempDir;

    /// Verifies that the TranscriptWriter creates the session directory on first append.
    #[tokio::test]
    async fn test_transcript_writer_creates_file() {
        let temp_dir = TempDir::new().unwrap();
        let session_id = "test-session-001";

        let writer = TranscriptWriter::new(temp_dir.path(), session_id)
            .await
            .expect("Failed to create TranscriptWriter");

        // Append an event to trigger file creation
        let event = AiEvent::Started {
            turn_id: "turn-1".to_string(),
        };
        writer.append(&event).await.expect("Failed to append event");

        // Verify the file was created
        assert!(writer.path().exists(), "Transcript file should exist");

        // Verify the path is correct
        let expected_path = temp_dir
            .path()
            .join("test-session-001")
            .join("transcript.json");
        assert_eq!(writer.path(), expected_path);
    }

    /// Helper to parse JSONL format for tests
    fn parse_jsonl(content: &str) -> Vec<Value> {
        content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("Invalid JSONL line"))
            .collect()
    }

    /// Verifies that events are stored as a valid JSON array.
    #[tokio::test]
    async fn test_transcript_writer_appends_events() {
        let temp_dir = TempDir::new().unwrap();
        let session_id = "test-session-002";

        let writer = TranscriptWriter::new(temp_dir.path(), session_id)
            .await
            .expect("Failed to create TranscriptWriter");

        // Append several events
        let events = vec![
            AiEvent::Started {
                turn_id: "turn-1".to_string(),
            },
            AiEvent::TextDelta {
                delta: "Hello".to_string(),
                accumulated: "Hello".to_string(),
            },
            AiEvent::Completed {
                response: "Done".to_string(),
                reasoning: None,
                input_tokens: Some(100),
                output_tokens: Some(50),
                duration_ms: Some(1000),
            },
        ];

        for event in &events {
            writer.append(event).await.expect("Failed to append event");
        }

        // Read the file and parse as JSON array
        let content = tokio::fs::read_to_string(writer.path())
            .await
            .expect("Failed to read transcript file");

        let entries = parse_jsonl(&content);
        assert_eq!(entries.len(), 3, "Should have 3 entries");

        // Verify each entry
        assert_eq!(entries[0]["type"], "started");
        assert_eq!(entries[0]["turn_id"], "turn-1");

        assert_eq!(entries[1]["type"], "text_delta");
        assert_eq!(entries[1]["delta"], "Hello");

        assert_eq!(entries[2]["type"], "completed");
        assert_eq!(entries[2]["response"], "Done");
    }

    /// Verifies thread safety by performing concurrent writes.
    #[tokio::test]
    async fn test_transcript_writer_handles_concurrent_writes() {
        let temp_dir = TempDir::new().unwrap();
        let session_id = "test-session-003";

        let writer = Arc::new(
            TranscriptWriter::new(temp_dir.path(), session_id)
                .await
                .expect("Failed to create TranscriptWriter"),
        );

        // Spawn 10 concurrent write tasks
        let mut handles = Vec::new();
        for i in 0..10 {
            let writer_clone = Arc::clone(&writer);
            let handle = tokio::spawn(async move {
                let event = AiEvent::TextDelta {
                    delta: format!("chunk-{i}"),
                    accumulated: format!("accumulated-{i}"),
                };
                writer_clone
                    .append(&event)
                    .await
                    .expect("Failed to append event");
            });
            handles.push(handle);
        }

        // Wait for all tasks to complete
        for handle in handles {
            handle.await.expect("Task panicked");
        }

        // Read and verify all 10 entries were written
        let content = tokio::fs::read_to_string(writer.path())
            .await
            .expect("Failed to read transcript file");

        let entries = parse_jsonl(&content);
        assert_eq!(
            entries.len(),
            10,
            "Should have 10 entries from concurrent writes"
        );

        // Verify each entry is a text_delta event
        for entry in &entries {
            assert_eq!(entry["type"], "text_delta");
        }
    }

    /// Verifies that each entry includes a `_timestamp` field.
    #[tokio::test]
    async fn test_transcript_writer_includes_timestamp() {
        let temp_dir = TempDir::new().unwrap();
        let session_id = "test-session-004";

        let writer = TranscriptWriter::new(temp_dir.path(), session_id)
            .await
            .expect("Failed to create TranscriptWriter");

        let event = AiEvent::Started {
            turn_id: "turn-ts".to_string(),
        };
        writer.append(&event).await.expect("Failed to append event");

        // Read and parse the array
        let content = tokio::fs::read_to_string(writer.path())
            .await
            .expect("Failed to read transcript file");

        let entries = parse_jsonl(&content);
        assert_eq!(entries.len(), 1);

        let entry = &entries[0];

        // Verify _timestamp field exists and is a valid ISO 8601 string
        assert!(
            entry.get("_timestamp").is_some(),
            "_timestamp field should exist"
        );
        let timestamp_str = entry["_timestamp"]
            .as_str()
            .expect("_timestamp should be a string");

        // Verify it can be parsed as a DateTime
        let parsed: Result<DateTime<Utc>, _> = timestamp_str.parse();
        assert!(
            parsed.is_ok(),
            "_timestamp should be a valid ISO 8601 datetime"
        );

        // Verify the event fields are also present (flattened)
        assert_eq!(entry["type"], "started");
        assert_eq!(entry["turn_id"], "turn-ts");
    }

    /// Verifies the transcript_path helper constructs the correct path.
    #[test]
    fn test_transcript_path_helper() {
        let base_dir = Path::new("/var/log/golish/transcripts");
        let session_id = "abc-123";

        let path = transcript_path(base_dir, session_id);

        assert_eq!(
            path,
            PathBuf::from("/var/log/golish/transcripts/abc-123/transcript.json")
        );
    }

    /// Verifies path construction with various session ID formats.
    #[test]
    fn test_transcript_path_with_various_session_ids() {
        let base_dir = Path::new("/tmp/transcripts");

        // UUID-style session ID
        let path1 = transcript_path(base_dir, "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(
            path1,
            PathBuf::from("/tmp/transcripts/550e8400-e29b-41d4-a716-446655440000/transcript.json")
        );

        // Simple numeric session ID
        let path2 = transcript_path(base_dir, "12345");
        assert_eq!(
            path2,
            PathBuf::from("/tmp/transcripts/12345/transcript.json")
        );

        // Empty session ID (edge case)
        let path3 = transcript_path(base_dir, "");
        assert_eq!(path3, PathBuf::from("/tmp/transcripts//transcript.json"));
    }

    /// Verifies that JSONL format is used (one JSON object per line).
    #[tokio::test]
    async fn test_transcript_writer_produces_jsonl() {
        let temp_dir = TempDir::new().unwrap();
        let session_id = "test-session-jsonl";

        let writer = TranscriptWriter::new(temp_dir.path(), session_id)
            .await
            .expect("Failed to create TranscriptWriter");

        writer
            .append(&AiEvent::Started {
                turn_id: "turn-1".to_string(),
            })
            .await
            .expect("Failed to append event");
        writer
            .append(&AiEvent::UserMessage {
                content: "test".to_string(),
            })
            .await
            .expect("Failed to append second event");

        let content = tokio::fs::read_to_string(writer.path())
            .await
            .expect("Failed to read transcript file");

        // JSONL should have 2 lines, each a valid JSON object
        let lines: Vec<_> = content.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 2, "JSONL should have 2 lines");

        // Verify each line is valid JSON
        for line in lines {
            serde_json::from_str::<Value>(line).expect("Each line should be valid JSON");
        }
    }
}

mod reader_tests {
    use super::*;
    use tempfile::TempDir;

    /// Verifies that read_transcript returns events that were written by TranscriptWriter.
    #[tokio::test]
    async fn test_read_transcript_returns_events() {
        let temp_dir = TempDir::new().unwrap();
        let session_id = "test-read";

        let writer = TranscriptWriter::new(temp_dir.path(), session_id)
            .await
            .unwrap();
        writer
            .append(&AiEvent::Started {
                turn_id: "turn-1".to_string(),
            })
            .await
            .unwrap();
        writer
            .append(&AiEvent::Completed {
                response: "Done".to_string(),
                reasoning: None,
                input_tokens: Some(100),
                output_tokens: Some(50),
                duration_ms: Some(1000),
            })
            .await
            .unwrap();

        let result = read_transcript(temp_dir.path(), session_id).await.unwrap();
        assert_eq!(result.len(), 2);

        // Verify first event is Started
        assert!(matches!(result[0].event, AiEvent::Started { .. }));

        // Verify second event is Completed
        assert!(matches!(result[1].event, AiEvent::Completed { .. }));

        // Verify timestamps are present and in order
        assert!(result[0].timestamp <= result[1].timestamp);
    }

    /// Verifies that read_transcript returns an error for missing files.
    #[tokio::test]
    async fn test_read_transcript_handles_missing_file() {
        let temp_dir = TempDir::new().unwrap();
        let result = read_transcript(temp_dir.path(), "nonexistent").await;
        assert!(result.is_err());
    }

    /// Verifies that read_transcript returns empty Vec for empty files.
    #[tokio::test]
    async fn test_read_transcript_handles_empty_file() {
        let temp_dir = TempDir::new().unwrap();
        let session_id = "test-empty";
        let path = transcript_path(temp_dir.path(), session_id);

        // Create parent directory and empty file
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "").unwrap();

        let result = read_transcript(temp_dir.path(), session_id).await.unwrap();
        assert!(result.is_empty());
    }

    /// Verifies that read_transcript returns empty Vec for files containing empty JSON array.
    #[tokio::test]
    async fn test_read_transcript_handles_empty_array() {
        let temp_dir = TempDir::new().unwrap();
        let session_id = "test-empty-array";
        let path = transcript_path(temp_dir.path(), session_id);

        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[]").unwrap();

        let result = read_transcript(temp_dir.path(), session_id).await.unwrap();
        assert!(result.is_empty());
    }

    /// Verifies that timestamps are correctly parsed from transcript entries.
    #[tokio::test]
    async fn test_read_transcript_preserves_timestamps() {
        let temp_dir = TempDir::new().unwrap();
        let session_id = "test-timestamps";

        let writer = TranscriptWriter::new(temp_dir.path(), session_id)
            .await
            .unwrap();

        let before = Utc::now();
        writer
            .append(&AiEvent::Started {
                turn_id: "turn-1".to_string(),
            })
            .await
            .unwrap();
        let after = Utc::now();

        let result = read_transcript(temp_dir.path(), session_id).await.unwrap();
        assert_eq!(result.len(), 1);

        assert!(result[0].timestamp >= before);
        assert!(result[0].timestamp <= after);
    }
}

mod formatter_tests {
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
}

mod artifact_tests {
    use super::*;
    use tempfile::TempDir;

    /// Verifies that save_summarizer_input creates a file with the expected content.
    #[test]
    fn test_save_summarizer_input() {
        let temp_dir = TempDir::new().unwrap();
        let session_id = "test-save";
        let content = "# Test Content\n\nSome conversation here.";

        let path = save_summarizer_input(temp_dir.path(), session_id, content).unwrap();

        assert!(path.exists());
        assert!(path.to_string_lossy().contains("summarizer-input-"));
        assert!(path.to_string_lossy().contains(session_id));

        let saved = std::fs::read_to_string(&path).unwrap();
        assert_eq!(saved, content);
    }

    /// Verifies that save_summary creates a file with the expected content.
    #[test]
    fn test_save_summary() {
        let temp_dir = TempDir::new().unwrap();
        let session_id = "test-summary";
        let summary = "## Summary\n\nUser asked for help.";

        let path = save_summary(temp_dir.path(), session_id, summary).unwrap();

        assert!(path.exists());
        assert!(path.to_string_lossy().contains("summary-"));
        assert!(path.to_string_lossy().contains(session_id));

        let saved = std::fs::read_to_string(&path).unwrap();
        assert_eq!(saved, summary);
    }

    /// Verifies that save_summarizer_input creates nested directories as needed.
    #[test]
    fn test_save_summarizer_input_creates_directory() {
        let temp_dir = TempDir::new().unwrap();
        let nested_dir = temp_dir.path().join("artifacts").join("compaction");
        let session_id = "test-nested";
        let content = "Test content";

        // Directory doesn't exist yet
        assert!(!nested_dir.exists());

        let path = save_summarizer_input(&nested_dir, session_id, content).unwrap();

        // Directory should now exist
        assert!(nested_dir.exists());
        assert!(path.exists());
    }

    /// Verifies that save_summary creates nested directories as needed.
    #[test]
    fn test_save_summary_creates_directory() {
        let temp_dir = TempDir::new().unwrap();
        let nested_dir = temp_dir.path().join("summaries");
        let session_id = "test-dir";
        let summary = "Summary content";

        assert!(!nested_dir.exists());

        let path = save_summary(&nested_dir, session_id, summary).unwrap();

        assert!(nested_dir.exists());
        assert!(path.exists());
    }
}

mod integration_tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_build_summarizer_input_end_to_end() {
        let temp_dir = TempDir::new().unwrap();
        let session_id = "test-e2e";

        // Create a transcript using the writer
        let writer = TranscriptWriter::new(temp_dir.path(), session_id)
            .await
            .unwrap();

        writer
            .append(&AiEvent::Started {
                turn_id: "turn-1".to_string(),
            })
            .await
            .unwrap();
        writer
            .append(&AiEvent::UserMessage {
                content: "Read the main.rs file".to_string(),
            })
            .await
            .unwrap();
        writer
            .append(&AiEvent::ToolRequest {
                tool_name: "read_file".to_string(),
                args: serde_json::json!({"path": "/src/main.rs"}),
                request_id: "req-1".to_string(),
                source: Default::default(),
            })
            .await
            .unwrap();
        writer
            .append(&AiEvent::ToolResult {
                tool_name: "read_file".to_string(),
                result: serde_json::json!({"content": "fn main() { println!(\"hello\"); }"}),
                success: true,
                request_id: "req-1".to_string(),
                source: Default::default(),
            })
            .await
            .unwrap();
        writer
            .append(&AiEvent::Completed {
                response: "I found the main function.".to_string(),
                reasoning: None,
                input_tokens: Some(200),
                output_tokens: Some(100),
                duration_ms: Some(2000),
            })
            .await
            .unwrap();

        // Now read and format
        let input = build_summarizer_input(temp_dir.path(), session_id)
            .await
            .unwrap();

        // Verify the formatted output contains expected content
        assert!(input.contains("[turn 001]"), "Should contain turn marker");
        assert!(input.contains("USER:"), "Should contain user message");
        assert!(
            input.contains("Read the main.rs file"),
            "Should contain user's request"
        );
        assert!(input.contains("read_file"), "Should contain tool name");
        assert!(
            input.contains("TOOL_REQUEST"),
            "Should contain tool request"
        );
        assert!(input.contains("TOOL_RESULT"), "Should contain tool result");
        assert!(
            input.contains("ASSISTANT"),
            "Should contain assistant response"
        );
        assert!(
            input.contains("I found the main function"),
            "Should contain assistant's response"
        );
        assert!(
            input.contains("200 in / 100 out tokens"),
            "Should contain token counts"
        );
    }
}

mod should_transcript_tests {
    use super::*;
    use golish_core::events::AiEvent;

    /// Helper that constructs a sample instance for each AiEvent variant.
    /// Uses a match statement so the compiler will force an update when new variants are added.
    fn all_variants() -> Vec<(AiEvent, bool)> {
        // Each entry: (event, expected should_transcript result)
        // The match ensures exhaustiveness — adding a new AiEvent variant will cause a compile error here.
        let variants: Vec<(AiEvent, bool)> = vec![
            (
                AiEvent::Started {
                    turn_id: "t".into(),
                },
                true,
            ),
            (
                AiEvent::UserMessage {
                    content: "hi".into(),
                },
                true,
            ),
            (AiEvent::SystemHooksInjected { hooks: vec![] }, true),
            (
                AiEvent::TextDelta {
                    delta: "x".into(),
                    accumulated: "x".into(),
                },
                false,
            ),
            (
                AiEvent::ToolRequest {
                    tool_name: "t".into(),
                    args: serde_json::json!({}),
                    request_id: "r".into(),
                    source: Default::default(),
                },
                true,
            ),
            (
                AiEvent::ToolApprovalRequest {
                    request_id: "r".into(),
                    tool_name: "t".into(),
                    args: serde_json::json!({}),
                    stats: None,
                    risk_level: golish_core::hitl::RiskLevel::Low,
                    can_learn: false,
                    suggestion: None,
                    source: Default::default(),
                },
                true,
            ),
            (
                AiEvent::ToolAutoApproved {
                    request_id: "r".into(),
                    tool_name: "t".into(),
                    args: serde_json::json!({}),
                    reason: "ok".into(),
                    source: Default::default(),
                },
                true,
            ),
            (
                AiEvent::ToolDenied {
                    request_id: "r".into(),
                    tool_name: "t".into(),
                    args: serde_json::json!({}),
                    reason: "no".into(),
                    source: Default::default(),
                },
                true,
            ),
            (
                AiEvent::ToolResult {
                    tool_name: "t".into(),
                    result: serde_json::json!(null),
                    success: true,
                    request_id: "r".into(),
                    source: Default::default(),
                },
                true,
            ),
            (
                AiEvent::ToolOutputChunk {
                    request_id: "r".into(),
                    tool_name: "t".into(),
                    chunk: "out".into(),
                    stream: "stdout".into(),
                    source: Default::default(),
                },
                false,
            ),
            (
                AiEvent::Reasoning {
                    content: "think".into(),
                },
                false,
            ),
            (
                AiEvent::Completed {
                    response: "done".into(),
                    reasoning: None,
                    input_tokens: None,
                    output_tokens: None,
                    duration_ms: None,
                },
                true,
            ),
            (
                AiEvent::Error {
                    message: "err".into(),
                    error_type: "e".into(),
                },
                true,
            ),
            (
                AiEvent::SubAgentStarted {
                    agent_id: "a".into(),
                    agent_name: "n".into(),
                    task: "t".into(),
                    depth: 0,
                    parent_request_id: "p".into(),
                },
                true,
            ),
            (
                AiEvent::SubAgentToolRequest {
                    agent_id: "a".into(),
                    tool_name: "t".into(),
                    args: serde_json::json!({}),
                    request_id: "r".into(),
                    parent_request_id: "p".into(),
                },
                false,
            ),
            (
                AiEvent::SubAgentToolResult {
                    agent_id: "a".into(),
                    tool_name: "t".into(),
                    success: true,
                    result: serde_json::json!(null),
                    request_id: "r".into(),
                    parent_request_id: "p".into(),
                },
                false,
            ),
            (
                AiEvent::SubAgentCompleted {
                    agent_id: "a".into(),
                    response: "ok".into(),
                    duration_ms: 0,
                    parent_request_id: "p".into(),
                },
                true,
            ),
            (
                AiEvent::SubAgentError {
                    agent_id: "a".into(),
                    error: "err".into(),
                    parent_request_id: "p".into(),
                },
                true,
            ),
            (
                AiEvent::ContextWarning {
                    utilization: 0.8,
                    total_tokens: 800,
                    max_tokens: 1000,
                },
                true,
            ),
            (
                AiEvent::ToolResponseTruncated {
                    tool_name: "t".into(),
                    original_tokens: 100,
                    truncated_tokens: 50,
                },
                true,
            ),
            (
                AiEvent::Warning {
                    message: "warn".into(),
                },
                true,
            ),
            (
                AiEvent::CompactionStarted {
                    tokens_before: 100,
                    messages_before: 5,
                },
                true,
            ),
            (
                AiEvent::CompactionCompleted {
                    tokens_before: 100,
                    messages_before: 5,
                    messages_after: 2,
                    summary_length: 50,
                    summary: None,
                    summarizer_input: None,
                },
                true,
            ),
            (
                AiEvent::CompactionFailed {
                    tokens_before: 100,
                    messages_before: 5,
                    error: "err".into(),
                    summarizer_input: None,
                },
                true,
            ),
            (
                AiEvent::LoopWarning {
                    tool_name: "t".into(),
                    current_count: 5,
                    max_count: 10,
                    message: "w".into(),
                },
                true,
            ),
            (
                AiEvent::LoopBlocked {
                    tool_name: "t".into(),
                    repeat_count: 10,
                    max_count: 10,
                    message: "b".into(),
                },
                true,
            ),
            (
                AiEvent::MaxIterationsReached {
                    iterations: 50,
                    max_iterations: 50,
                    message: "m".into(),
                },
                true,
            ),
            (
                AiEvent::WorkflowStarted {
                    workflow_id: "w".into(),
                    workflow_name: "n".into(),
                    session_id: "s".into(),
                },
                true,
            ),
            (
                AiEvent::WorkflowStepStarted {
                    workflow_id: "w".into(),
                    step_name: "s".into(),
                    step_index: 0,
                    total_steps: 1,
                },
                true,
            ),
            (
                AiEvent::WorkflowStepCompleted {
                    workflow_id: "w".into(),
                    step_name: "s".into(),
                    output: None,
                    duration_ms: 0,
                },
                true,
            ),
            (
                AiEvent::WorkflowCompleted {
                    workflow_id: "w".into(),
                    final_output: "ok".into(),
                    total_duration_ms: 0,
                },
                true,
            ),
            (
                AiEvent::WorkflowError {
                    workflow_id: "w".into(),
                    step_name: None,
                    error: "err".into(),
                },
                true,
            ),
            (
                AiEvent::PlanUpdated {
                    version: 1,
                    summary: golish_core::plan::PlanSummary {
                        total: 0,
                        completed: 0,
                        in_progress: 0,
                        pending: 0,
                    },
                    steps: vec![],
                    explanation: None,
                },
                true,
            ),
            (
                AiEvent::ServerToolStarted {
                    request_id: "r".into(),
                    tool_name: "web_search".into(),
                    input: serde_json::json!({}),
                },
                true,
            ),
            (
                AiEvent::WebSearchResult {
                    request_id: "r".into(),
                    results: serde_json::json!([]),
                },
                true,
            ),
            (
                AiEvent::WebFetchResult {
                    request_id: "r".into(),
                    url: "http://example.com".into(),
                    content_preview: "preview".into(),
                },
                true,
            ),
            (
                AiEvent::PromptGenerationStarted {
                    agent_id: "a".into(),
                    parent_request_id: "p".into(),
                    architect_system_prompt: "sys".into(),
                    architect_user_message: "usr".into(),
                },
                true,
            ),
            (
                AiEvent::PromptGenerationCompleted {
                    agent_id: "a".into(),
                    parent_request_id: "p".into(),
                    generated_prompt: Some("prompt".into()),
                    success: true,
                    duration_ms: 100,
                },
                true,
            ),
            (
                AiEvent::SubAgentTextDelta {
                    agent_id: "a".into(),
                    delta: "d".into(),
                    accumulated: "d".into(),
                    parent_request_id: "p".into(),
                },
                false,
            ),
            (
                AiEvent::SubtaskWaitingForInput {
                    task_id: "t".into(),
                    subtask_id: "s".into(),
                    title: "title".into(),
                    prompt: "question".into(),
                },
                true,
            ),
            (
                AiEvent::SubtaskUserInput {
                    task_id: "t".into(),
                    subtask_id: "s".into(),
                    input: "answer".into(),
                },
                true,
            ),
            (
                AiEvent::TaskResumed {
                    task_id: "t".into(),
                    subtask_index: 2,
                    total_subtasks: 5,
                },
                true,
            ),
            (
                AiEvent::EnricherResult {
                    task_id: "t".into(),
                    subtask_id: "s".into(),
                    context_added: "extra context".into(),
                },
                true,
            ),
        ];

        // Compile-time exhaustiveness check: if a new variant is added to AiEvent,
        // this match will fail to compile, reminding you to add it above.
        fn _assert_exhaustive(e: &AiEvent) {
            match e {
                AiEvent::Started { .. }
                | AiEvent::UserMessage { .. }
                | AiEvent::SystemHooksInjected { .. }
                | AiEvent::TextDelta { .. }
                | AiEvent::ToolRequest { .. }
                | AiEvent::ToolApprovalRequest { .. }
                | AiEvent::ToolAutoApproved { .. }
                | AiEvent::ToolDenied { .. }
                | AiEvent::ToolResult { .. }
                | AiEvent::ToolOutputChunk { .. }
                | AiEvent::Reasoning { .. }
                | AiEvent::Completed { .. }
                | AiEvent::Error { .. }
                | AiEvent::SubAgentStarted { .. }
                | AiEvent::SubAgentToolRequest { .. }
                | AiEvent::SubAgentToolResult { .. }
                | AiEvent::SubAgentTextDelta { .. }
                | AiEvent::SubAgentCompleted { .. }
                | AiEvent::SubAgentError { .. }
                | AiEvent::ContextWarning { .. }
                | AiEvent::ToolResponseTruncated { .. }
                | AiEvent::Warning { .. }
                | AiEvent::CompactionStarted { .. }
                | AiEvent::CompactionCompleted { .. }
                | AiEvent::CompactionFailed { .. }
                | AiEvent::LoopWarning { .. }
                | AiEvent::LoopBlocked { .. }
                | AiEvent::MaxIterationsReached { .. }
                | AiEvent::WorkflowStarted { .. }
                | AiEvent::WorkflowStepStarted { .. }
                | AiEvent::WorkflowStepCompleted { .. }
                | AiEvent::WorkflowCompleted { .. }
                | AiEvent::WorkflowError { .. }
                | AiEvent::PlanUpdated { .. }
                | AiEvent::ServerToolStarted { .. }
                | AiEvent::WebSearchResult { .. }
                | AiEvent::WebFetchResult { .. }
                | AiEvent::PromptGenerationStarted { .. }
                | AiEvent::PromptGenerationCompleted { .. }
                | AiEvent::AskHumanRequest { .. }
                | AiEvent::AskHumanResponse { .. }
                | AiEvent::SubtaskWaitingForInput { .. }
                | AiEvent::SubtaskUserInput { .. }
                | AiEvent::TaskResumed { .. }
                | AiEvent::EnricherResult { .. }
                | AiEvent::TaskProgress { .. }
                | AiEvent::SubtaskCreated { .. }
                | AiEvent::SubtaskCompleted { .. } => {}
            }
        }

        variants
    }

    /// Tests that should_transcript returns the correct value for every AiEvent variant.
    /// If a new variant is added to AiEvent, the exhaustive match in all_variants() will
    /// fail to compile, forcing the developer to decide whether it should be transcribed.
    #[test]
    fn test_should_transcript_exhaustive() {
        for (event, expected) in all_variants() {
            let result = should_transcript(&event);
            assert_eq!(
                result,
                expected,
                "should_transcript({}) = {}, expected {}",
                event.event_type(),
                result,
                expected
            );
        }
    }

    /// Verify the specific filtered events return false.
    #[test]
    fn test_filtered_events() {
        let filtered = [
            AiEvent::TextDelta {
                delta: "x".into(),
                accumulated: "x".into(),
            },
            AiEvent::Reasoning {
                content: "think".into(),
            },
            AiEvent::ToolOutputChunk {
                request_id: "r".into(),
                tool_name: "t".into(),
                chunk: "out".into(),
                stream: "stdout".into(),
                source: Default::default(),
            },
            AiEvent::SubAgentToolRequest {
                agent_id: "a".into(),
                tool_name: "t".into(),
                args: serde_json::json!({}),
                request_id: "r".into(),
                parent_request_id: "p".into(),
            },
            AiEvent::SubAgentToolResult {
                agent_id: "a".into(),
                tool_name: "t".into(),
                success: true,
                result: serde_json::json!(null),
                request_id: "r".into(),
                parent_request_id: "p".into(),
            },
        ];

        for event in &filtered {
            assert!(
                !should_transcript(event),
                "{} should be filtered from transcript",
                event.event_type()
            );
        }
    }
}

mod writer_reload_tests {
    use super::*;
    use golish_core::events::AiEvent;
    use tempfile::TempDir;

    /// Verifies that TranscriptWriter::new loads existing entries when the file already exists,
    /// and subsequent appends continue from where it left off (not overwriting).
    #[tokio::test]
    async fn test_transcript_writer_reloads_existing_entries() {
        let temp_dir = TempDir::new().unwrap();
        let session_id = "test-reload";

        // Phase 1: Create writer and append two events
        {
            let writer = TranscriptWriter::new(temp_dir.path(), session_id)
                .await
                .unwrap();
            writer
                .append(&AiEvent::Started {
                    turn_id: "turn-1".into(),
                })
                .await
                .unwrap();
            writer
                .append(&AiEvent::UserMessage {
                    content: "hello".into(),
                })
                .await
                .unwrap();
        }
        // Writer is dropped here

        // Phase 2: Create a new writer for the same session — should load existing entries
        {
            let writer = TranscriptWriter::new(temp_dir.path(), session_id)
                .await
                .unwrap();

            // Append one more event
            writer
                .append(&AiEvent::Completed {
                    response: "done".into(),
                    reasoning: None,
                    input_tokens: Some(10),
                    output_tokens: Some(5),
                    duration_ms: Some(100),
                })
                .await
                .unwrap();
        }

        // Verify: all 3 events should be in the file
        let events = read_transcript(temp_dir.path(), session_id).await.unwrap();
        assert_eq!(
            events.len(),
            3,
            "Should have 3 events total after reload + append"
        );

        assert!(matches!(events[0].event, AiEvent::Started { .. }));
        assert!(matches!(events[1].event, AiEvent::UserMessage { .. }));
        assert!(matches!(events[2].event, AiEvent::Completed { .. }));
    }
}
