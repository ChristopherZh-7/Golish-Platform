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
