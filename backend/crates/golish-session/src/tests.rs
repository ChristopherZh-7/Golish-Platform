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

mod manager_persistence;
