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
