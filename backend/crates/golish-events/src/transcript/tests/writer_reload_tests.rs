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
