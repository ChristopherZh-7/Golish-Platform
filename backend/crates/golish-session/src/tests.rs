//! Session tests.

use std::path::PathBuf;

use anyhow::Result;
use chrono::{DateTime, Utc};
use rig::completion::{AssistantContent, Message};
use rig::message::UserContent;
use rig::one_or_many::OneOrMany;
use serde_json::json;

use crate::archive::list_recent_sessions;
use crate::manager::GolishSessionManager;
use crate::types::{
    strip_xml_tags, truncate, GolishMessageRole, GolishSessionMessage, GolishSessionSnapshot,
    SessionListingInfo,
};


use rig::message::Text;
use serial_test::serial;
use tempfile::TempDir;

#[test]
fn test_golish_session_message_creation() {
    let user_msg = GolishSessionMessage::user("Hello");
    assert_eq!(user_msg.role, GolishMessageRole::User);
    assert_eq!(user_msg.content, "Hello");

    let assistant_msg = GolishSessionMessage::assistant("Hi there");
    assert_eq!(assistant_msg.role, GolishMessageRole::Assistant);
    assert_eq!(assistant_msg.content, "Hi there");
}

#[test]
fn test_golish_session_message_system() {
    let system_msg = GolishSessionMessage::system("You are a helpful assistant");
    assert_eq!(system_msg.role, GolishMessageRole::System);
    assert_eq!(system_msg.content, "You are a helpful assistant");
    assert!(system_msg.tool_call_id.is_none());
    assert!(system_msg.tool_name.is_none());
}

#[test]
fn test_golish_session_message_tool_result() {
    let tool_msg = GolishSessionMessage::tool_result("File contents here", "call_123");
    assert_eq!(tool_msg.role, GolishMessageRole::Tool);
    assert_eq!(tool_msg.content, "File contents here");
    assert_eq!(tool_msg.tool_call_id, Some("call_123".to_string()));
    assert!(tool_msg.tool_name.is_none());
}

#[test]
fn test_truncate() {
    assert_eq!(truncate("short", 10), "short");
    assert_eq!(truncate("a longer string", 5), "a lo…");
    assert_eq!(truncate("", 10), "");
}

#[test]
fn test_truncate_exact_length() {
    assert_eq!(truncate("12345", 5), "12345");
    assert_eq!(truncate("123456", 5), "1234…");
}

#[test]
fn test_truncate_unicode() {
    // Unicode characters should be counted as single chars
    assert_eq!(truncate("héllo", 5), "héllo");
    assert_eq!(truncate("héllo world", 5), "héll…");
}

#[test]
fn test_rig_message_conversion_user() {
    let rig_msg = Message::User {
        content: OneOrMany::one(UserContent::Text(Text {
            text: "Hello from user".to_string(),
        })),
    };

    let golish_msg = GolishSessionMessage::from(&rig_msg);
    assert_eq!(golish_msg.role, GolishMessageRole::User);
    assert_eq!(golish_msg.content, "Hello from user");
}

#[test]
fn test_rig_message_conversion_assistant() {
    let rig_msg = Message::Assistant {
        id: None,
        content: OneOrMany::one(AssistantContent::Text(Text {
            text: "Hello from assistant".to_string(),
        })),
    };

    let golish_msg = GolishSessionMessage::from(&rig_msg);
    assert_eq!(golish_msg.role, GolishMessageRole::Assistant);
    assert_eq!(golish_msg.content, "Hello from assistant");
}

#[test]
fn test_golish_message_to_rig_user() {
    let golish_msg = GolishSessionMessage::user("Test user message");
    let rig_msg = golish_msg.to_rig_message();

    assert!(rig_msg.is_some());
    let rig_msg = rig_msg.unwrap();
    match rig_msg {
        Message::User { content } => {
            let text = content
                .iter()
                .filter_map(|c| match c {
                    UserContent::Text(t) => Some(t.text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            assert_eq!(text, "Test user message");
        }
        _ => panic!("Expected User message"),
    }
}

#[test]
fn test_golish_message_to_rig_assistant() {
    let golish_msg = GolishSessionMessage::assistant("Test assistant message");
    let rig_msg = golish_msg.to_rig_message();

    assert!(rig_msg.is_some());
    let rig_msg = rig_msg.unwrap();
    match rig_msg {
        Message::Assistant { content, .. } => {
            let text = content
                .iter()
                .filter_map(|c| match c {
                    AssistantContent::Text(t) => Some(t.text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            assert_eq!(text, "Test assistant message");
        }
        _ => panic!("Expected Assistant message"),
    }
}

#[test]
fn test_golish_message_to_rig_system_returns_none() {
    let golish_msg = GolishSessionMessage::system("System prompt");
    assert!(golish_msg.to_rig_message().is_none());
}

#[test]
fn test_golish_message_to_rig_tool_returns_none() {
    let golish_msg = GolishSessionMessage::tool_result("Result", "call_id");
    assert!(golish_msg.to_rig_message().is_none());
}

#[test]
fn test_golish_session_snapshot_serialization() {
    let snapshot = GolishSessionSnapshot {
        workspace_label: "test-workspace".to_string(),
        workspace_path: "/tmp/test".to_string(),
        model: "claude-3".to_string(),
        provider: "anthropic".to_string(),
        started_at: Utc::now(),
        ended_at: Utc::now(),
        total_messages: 2,
        distinct_tools: vec!["read_file".to_string(), "write_file".to_string()],
        transcript: vec!["User: Hello".to_string(), "Assistant: Hi".to_string()],
        messages: vec![
            GolishSessionMessage::user("Hello"),
            GolishSessionMessage::assistant("Hi"),
        ],
        sidecar_session_id: None,
        total_tokens: None,
        agent_mode: None,
    };

    let json = serde_json::to_string(&snapshot).expect("Failed to serialize");
    let deserialized: GolishSessionSnapshot =
        serde_json::from_str(&json).expect("Failed to deserialize");

    assert_eq!(deserialized.workspace_label, "test-workspace");
    assert_eq!(deserialized.total_messages, 2);
    assert_eq!(deserialized.messages.len(), 2);
    assert_eq!(deserialized.distinct_tools.len(), 2);
}

#[test]
fn test_session_listing_info_serialization() {
    let info = SessionListingInfo {
        identifier: "session-test-123".to_string(),
        path: PathBuf::from("/tmp/sessions/session-test-123.json"),
        workspace_label: "my-project".to_string(),
        workspace_path: "/home/user/my-project".to_string(),
        model: "claude-3-opus".to_string(),
        provider: "anthropic".to_string(),
        started_at: Utc::now(),
        ended_at: Utc::now(),
        total_messages: 10,
        distinct_tools: vec!["bash".to_string()],
        first_prompt_preview: Some("Help me debug...".to_string()),
        first_reply_preview: Some("I'd be happy to help...".to_string()),
        status: Some("completed".to_string()),
        title: Some("Debug Authentication Bug".to_string()),
    };

    let json = serde_json::to_string(&info).expect("Failed to serialize");
    let deserialized: SessionListingInfo =
        serde_json::from_str(&json).expect("Failed to deserialize");

    assert_eq!(deserialized.identifier, "session-test-123");
    assert_eq!(deserialized.workspace_label, "my-project");
    assert_eq!(
        deserialized.first_prompt_preview,
        Some("Help me debug...".to_string())
    );
}

#[test]
fn test_golish_message_role_serialization() {
    // Test that roles serialize to lowercase as expected
    let user_msg = GolishSessionMessage::user("test");
    let json = serde_json::to_string(&user_msg).unwrap();
    assert!(json.contains("\"role\":\"user\""));

    let assistant_msg = GolishSessionMessage::assistant("test");
    let json = serde_json::to_string(&assistant_msg).unwrap();
    assert!(json.contains("\"role\":\"assistant\""));

    let system_msg = GolishSessionMessage::system("test");
    let json = serde_json::to_string(&system_msg).unwrap();
    assert!(json.contains("\"role\":\"system\""));

    let tool_msg = GolishSessionMessage::tool_result("test", "id");
    let json = serde_json::to_string(&tool_msg).unwrap();
    assert!(json.contains("\"role\":\"tool\""));
}

#[test]
fn test_golish_message_optional_fields_skip_when_none() {
    let msg = GolishSessionMessage::user("Hello");
    let json = serde_json::to_string(&msg).unwrap();

    // tool_call_id and tool_name should not appear when None
    assert!(!json.contains("tool_call_id"));
    assert!(!json.contains("tool_name"));
}

#[test]
fn test_golish_message_includes_tool_call_id_when_present() {
    let msg = GolishSessionMessage::tool_result("result", "call_abc");
    let json = serde_json::to_string(&msg).unwrap();

    assert!(json.contains("\"tool_call_id\":\"call_abc\""));
}

#[test]
fn test_strip_xml_tags() {
    // Test stripping context tags
    let input = "<context>\n<cwd>/Users/test/project</cwd>\n<session_id>abc123</session_id>\n</context>\nActual user prompt here";
    let result = strip_xml_tags(input);
    assert_eq!(result, "Actual user prompt here");

    // Test with no tags
    let input = "Just a normal string";
    let result = strip_xml_tags(input);
    assert_eq!(result, "Just a normal string");

    // Test with partial tags (should still work)
    let input = "<context>Some content</context> More text";
    let result = strip_xml_tags(input);
    assert_eq!(result, "More text");

    // Test with nested content preserved outside tags
    let input = "Before <cwd>/path</cwd> After";
    let result = strip_xml_tags(input);
    assert_eq!(result, "Before  After");
}

// Note: The async tests that interact with the filesystem via golish-core's
// session_archive are integration tests that depend on the VT_SESSION_DIR
// environment variable. These tests are difficult to run in parallel because
// they share global state. For comprehensive session persistence testing,
// see the integration tests or run these with --test-threads=1.
//
// The tests below focus on unit-level functionality that doesn't require
// filesystem isolation.

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
