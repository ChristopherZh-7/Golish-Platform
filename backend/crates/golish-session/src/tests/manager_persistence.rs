use super::*;

#[tokio::test]
#[serial]
async fn test_session_manager_creation() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    // Set VT_SESSION_DIR for this test
    std::env::set_var("VT_SESSION_DIR", temp_dir.path());

    let manager =
        GolishSessionManager::new(temp_dir.path().to_path_buf(), "test-model", "test-provider")
            .await;

    assert!(manager.is_ok());
    let manager = manager.unwrap();
    assert_eq!(manager.message_count(), 0);
    assert!(manager.tools_used().is_empty());

    // Clean up
    std::env::remove_var("VT_SESSION_DIR");
}

#[tokio::test]
#[serial]
async fn test_session_manager_add_messages() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    std::env::set_var("VT_SESSION_DIR", temp_dir.path());

    let mut manager =
        GolishSessionManager::new(temp_dir.path().to_path_buf(), "test-model", "test-provider")
            .await
            .expect("Failed to create manager");

    manager.add_user_message("Hello, how are you?");
    assert_eq!(manager.message_count(), 1);

    manager.add_assistant_message("I'm doing well, thank you!");
    assert_eq!(manager.message_count(), 2);

    manager.add_tool_use("read_file", "File contents: hello world");
    assert_eq!(manager.message_count(), 3);
    assert!(manager.tools_used().contains(&"read_file".to_string()));

    std::env::remove_var("VT_SESSION_DIR");
}

#[tokio::test]
#[serial]
async fn test_session_manager_tools_tracking() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    std::env::set_var("VT_SESSION_DIR", temp_dir.path());

    let mut manager =
        GolishSessionManager::new(temp_dir.path().to_path_buf(), "test-model", "test-provider")
            .await
            .expect("Failed to create manager");

    manager.add_tool_use("read_file", "contents");
    manager.add_tool_use("write_file", "success");
    manager.add_tool_use("read_file", "more contents"); // Duplicate tool

    let tools = manager.tools_used();
    assert_eq!(tools.len(), 2); // Should dedupe
    assert!(tools.contains(&"read_file".to_string()));
    assert!(tools.contains(&"write_file".to_string()));

    std::env::remove_var("VT_SESSION_DIR");
}

#[tokio::test]
#[serial]
async fn test_list_empty_sessions_dir() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    std::env::set_var("VT_SESSION_DIR", temp_dir.path());

    let sessions = list_recent_sessions(10).await.expect("Failed to list");
    assert!(sessions.is_empty());

    std::env::remove_var("VT_SESSION_DIR");
}

#[tokio::test]
#[serial]
async fn test_list_recent_sessions_with_limit() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    std::env::set_var("VT_SESSION_DIR", temp_dir.path());

    // Create 5 sessions
    for i in 0..5 {
        let mut manager = GolishSessionManager::new(
            temp_dir.path().to_path_buf(),
            format!("model-{}", i),
            "provider",
        )
        .await
        .expect("Failed to create manager");

        manager.add_user_message(&format!("Message {}", i));
        manager.finalize().expect("Failed to finalize");
    }

    let sessions = list_recent_sessions(2).await.expect("Failed to list");
    assert_eq!(sessions.len(), 2);

    std::env::remove_var("VT_SESSION_DIR");
}

#[test]
fn test_session_message_roundtrip() {
    // Test that messages survive serialization roundtrip
    let original = GolishSessionMessage {
        role: GolishMessageRole::Tool,
        content: "Tool result with special chars: <>&\"'".to_string(),
        tool_call_id: Some("call_123".to_string()),
        tool_name: Some("read_file".to_string()),
        tokens_used: None,
    };

    let json = serde_json::to_string(&original).unwrap();
    let restored: GolishSessionMessage = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.role, original.role);
    assert_eq!(restored.content, original.content);
    assert_eq!(restored.tool_call_id, original.tool_call_id);
    assert_eq!(restored.tool_name, original.tool_name);
    assert_eq!(restored.tokens_used, original.tokens_used);
}

#[tokio::test]
#[serial]
async fn test_session_finalization_creates_persisted_session() {
    // Test that finalizing a session creates a persistent file
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    std::env::set_var("VT_SESSION_DIR", temp_dir.path());

    // Create and populate a session
    let mut manager =
        GolishSessionManager::new(temp_dir.path().to_path_buf(), "test-model", "test-provider")
            .await
            .expect("Failed to create manager");

    manager.add_user_message("Test user message for finalization");
    manager.add_assistant_message("Test assistant response");

    // Finalize the session
    let finalized_path = manager.finalize().expect("Failed to finalize session");

    // Verify the file exists
    assert!(
        finalized_path.exists(),
        "Finalized session file should exist"
    );

    // Verify the file is in the temp directory
    assert!(
        finalized_path.starts_with(temp_dir.path()),
        "Session file should be in temp dir"
    );

    // Verify the file has expected content (JSON format)
    let content = std::fs::read_to_string(&finalized_path).expect("Failed to read session");
    assert!(
        content.contains("test-model"),
        "Session file should contain model name"
    );
    assert!(
        content.contains("test-provider"),
        "Session file should contain provider name"
    );
    // Check for message content or structure
    assert!(
        content.contains("messages") || content.contains("Test user message"),
        "Session file should contain messages data"
    );

    std::env::remove_var("VT_SESSION_DIR");
}

#[tokio::test]
#[serial]
async fn test_session_finalization_is_one_shot() {
    // Test that finalize() can only be called once - subsequent calls fail
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    std::env::set_var("VT_SESSION_DIR", temp_dir.path());

    let mut manager =
        GolishSessionManager::new(temp_dir.path().to_path_buf(), "test-model", "test-provider")
            .await
            .expect("Failed to create manager");

    manager.add_user_message("Test message");

    // First finalize should succeed
    let result1 = manager.finalize();
    assert!(result1.is_ok(), "First finalize should succeed");

    // Second finalize should fail (archive already taken)
    let result2 = manager.finalize();
    assert!(
        result2.is_err(),
        "Second finalize should fail - session already finalized"
    );

    std::env::remove_var("VT_SESSION_DIR");
}

#[tokio::test]
#[serial]
async fn test_session_save_allows_incremental_persistence() {
    // Test that save() can be called multiple times (unlike finalize)
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    std::env::set_var("VT_SESSION_DIR", temp_dir.path());

    let mut manager =
        GolishSessionManager::new(temp_dir.path().to_path_buf(), "test-model", "test-provider")
            .await
            .expect("Failed to create manager");

    manager.add_user_message("First message");

    // First save should succeed
    let path1 = manager.save().expect("First save should succeed");
    assert!(path1.exists());

    // Add more messages and save again
    manager.add_assistant_message("Response to first");
    manager.add_user_message("Second message");

    // Second save should also succeed
    let path2 = manager.save().expect("Second save should succeed");
    assert!(path2.exists());
    assert_eq!(path1, path2, "Save should write to the same file");

    // Finalize should still work after saves
    let final_path = manager
        .finalize()
        .expect("Finalize should work after saves");
    assert!(final_path.exists());

    std::env::remove_var("VT_SESSION_DIR");
}

#[test]
fn test_backwards_compatibility_message_without_tokens() {
    // Test that old messages without tokens_used field can still be deserialized
    let json_without_tokens = r#"{
        "role": "user",
        "content": "Hello world",
        "tool_call_id": null,
        "tool_name": null
    }"#;

    let message: GolishSessionMessage =
        serde_json::from_str(json_without_tokens).expect("Failed to deserialize old format");

    assert_eq!(message.role, GolishMessageRole::User);
    assert_eq!(message.content, "Hello world");
    assert_eq!(message.tokens_used, None);
}

#[test]
fn test_backwards_compatibility_snapshot_without_total_tokens() {
    // Test that old snapshots without total_tokens field can still be deserialized
    let json_without_total_tokens = r#"{
        "workspace_label": "test",
        "workspace_path": "/tmp/test",
        "model": "claude-3",
        "provider": "anthropic",
        "started_at": "2024-01-01T00:00:00Z",
        "ended_at": "2024-01-01T01:00:00Z",
        "total_messages": 2,
        "distinct_tools": [],
        "transcript": [],
        "messages": [
            {
                "role": "user",
                "content": "Hello"
            },
            {
                "role": "assistant",
                "content": "Hi"
            }
        ]
    }"#;

    let snapshot: GolishSessionSnapshot = serde_json::from_str(json_without_total_tokens)
        .expect("Failed to deserialize old format");

    assert_eq!(snapshot.workspace_label, "test");
    assert_eq!(snapshot.total_messages, 2);
    assert_eq!(snapshot.total_tokens, None);
}

#[test]
fn test_new_fields_are_not_serialized_when_none() {
    // Verify that None values are omitted from JSON (keeps files compact)
    let message = GolishSessionMessage::user("Test");
    let json = serde_json::to_string(&message).expect("Failed to serialize");

    // Should not contain tokens_used field
    assert!(!json.contains("tokens_used"));

    let snapshot = GolishSessionSnapshot {
        workspace_label: "test".to_string(),
        workspace_path: "/tmp".to_string(),
        model: "test".to_string(),
        provider: "test".to_string(),
        started_at: Utc::now(),
        ended_at: Utc::now(),
        total_messages: 0,
        distinct_tools: vec![],
        transcript: vec![],
        messages: vec![],
        sidecar_session_id: None,
        total_tokens: None,
        agent_mode: None,
    };
    let json = serde_json::to_string(&snapshot).expect("Failed to serialize");

    // Should not contain total_tokens field
    assert!(!json.contains("total_tokens"));

    // Should not contain agent_mode field
    assert!(!json.contains("agent_mode"));
}
