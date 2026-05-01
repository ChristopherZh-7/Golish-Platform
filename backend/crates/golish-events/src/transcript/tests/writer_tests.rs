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
