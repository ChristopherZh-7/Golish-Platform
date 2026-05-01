use std::path::PathBuf;
use uuid::Uuid;
use super::*;
use super::helpers::{parse_context_xml, extract_xml_tag, truncate};

#[test]
fn test_event_type_names() {
    assert_eq!(
        EventType::UserPrompt {
            intent: "test".into()
        }
        .name(),
        "user_prompt"
    );
    assert_eq!(
        EventType::FileEdit {
            path: PathBuf::from("test"),
            operation: FileOperation::Create,
            summary: None
        }
        .name(),
        "file_edit"
    );
}

#[test]
fn test_event_type_high_signal() {
    assert!(EventType::UserPrompt {
        intent: "test".into()
    }
    .is_high_signal());
    assert!(EventType::AgentReasoning {
        content: "test".into(),
        decision_type: None
    }
    .is_high_signal());
    assert!(!EventType::ToolCall {
        tool_name: "test".into(),
        args_summary: "test".into(),
        reasoning: None,
        success: true
    }
    .is_high_signal());
}

#[test]
fn test_session_event_creation() {
    let session_id = Uuid::new_v4().to_string();

    let event = SessionEvent::user_prompt(session_id.clone(), "Add authentication");
    assert_eq!(event.session_id, session_id);
    assert!(event.content.contains("authentication"));
    assert!(event.embedding.is_none());
}

#[test]
fn test_file_edit_event() {
    let session_id = Uuid::new_v4().to_string();
    let path = PathBuf::from("/src/lib.rs");

    let event = SessionEvent::file_edit(session_id, path.clone(), FileOperation::Modify, None);

    assert_eq!(event.files_modified, vec![path]);
    assert!(event.content.contains("modified"));
}

#[test]
fn test_parse_context_xml() {
    let input = r#"<context>
<cwd>/Users/xlyk/Code/golish</cwd>
<session_id>abc-123</session_id>
</context>

which files are in the current directory?"#;

    let (cwd, clean) = parse_context_xml(input);
    assert_eq!(cwd, Some("/Users/xlyk/Code/golish".to_string()));
    assert_eq!(clean, "which files are in the current directory?");
}

#[test]
fn test_parse_context_xml_no_context() {
    let input = "just a regular message";
    let (cwd, clean) = parse_context_xml(input);
    assert_eq!(cwd, None);
    assert_eq!(clean, "just a regular message");
}

#[test]
fn test_user_prompt_with_context() {
    let session_id = Uuid::new_v4().to_string();
    let input = r#"<context>
<cwd>/Users/xlyk/Code/golish</cwd>
</context>

list files"#;

    let event = SessionEvent::user_prompt(session_id, input);
    assert_eq!(event.cwd, Some("/Users/xlyk/Code/golish".to_string()));
    assert_eq!(event.content, "list files");
}

#[test]
fn test_parse_context_xml_with_session_id() {
    let input = r#"<context>
<cwd>/path/to/project</cwd>
<session_id>abc-123-def</session_id>
</context>

What files exist?"#;

    let (cwd, clean) = parse_context_xml(input);
    assert_eq!(cwd, Some("/path/to/project".to_string()));
    assert_eq!(clean, "What files exist?");
}

#[test]
fn test_parse_context_xml_multiline_message() {
    let input = r#"<context>
<cwd>/project</cwd>
</context>

First line
Second line
Third line"#;

    let (cwd, clean) = parse_context_xml(input);
    assert_eq!(cwd, Some("/project".to_string()));
    assert!(clean.contains("First line"));
    assert!(clean.contains("Second line"));
    assert!(clean.contains("Third line"));
}

#[test]
fn test_parse_context_xml_missing_cwd() {
    let input = r#"<context>
<session_id>abc-123</session_id>
</context>

message without cwd"#;

    let (cwd, clean) = parse_context_xml(input);
    assert_eq!(cwd, None);
    assert_eq!(clean, "message without cwd");
}

#[test]
fn test_tool_call_with_output_basic() {
    let session_id = Uuid::new_v4().to_string();
    let event = SessionEvent::tool_call_with_output(
        session_id,
        "read_file".to_string(),
        Some("path=src/main.rs".to_string()),
        None,
        true,
        Some("fn main() { println!(\"Hello\"); }".to_string()),
        None,
    );

    assert_eq!(event.event_type.name(), "tool_call");
    assert_eq!(
        event.tool_output,
        Some("fn main() { println!(\"Hello\"); }".to_string())
    );
    assert!(event.diff.is_none());
}

#[test]
fn test_tool_call_with_output_edit() {
    let session_id = Uuid::new_v4().to_string();
    let diff =
        "--- src/lib.rs\n+++ src/lib.rs\n@@ -1,1 +1,2 @@\n-old line\n+new line\n+added line";
    let event = SessionEvent::tool_call_with_output(
        session_id,
        "edit_file".to_string(),
        Some("path=src/lib.rs".to_string()),
        None,
        true,
        Some("Edit applied successfully".to_string()),
        Some(diff.to_string()),
    );

    assert!(event.diff.is_some());
    assert!(event.diff.as_ref().unwrap().contains("-old line"));
    assert!(event.diff.as_ref().unwrap().contains("+new line"));
}

#[test]
fn test_tool_call_with_output_truncation() {
    let session_id = Uuid::new_v4().to_string();
    // Create content longer than 2000 chars
    let long_output = "x".repeat(3000);
    let event = SessionEvent::tool_call_with_output(
        session_id,
        "read_file".to_string(),
        Some("path=big.txt".to_string()),
        None,
        true,
        Some(long_output),
        None,
    );

    // Should be truncated to ~2000 chars (use char count, not byte len due to ellipsis)
    assert!(event.tool_output.is_some());
    let char_count = event.tool_output.as_ref().unwrap().chars().count();
    assert!(
        char_count <= 2000,
        "Expected <= 2000 chars, got {}",
        char_count
    );
}

#[test]
fn test_tool_call_with_output_diff_truncation() {
    let session_id = Uuid::new_v4().to_string();
    // Create diff longer than 4000 chars
    let long_diff = "+".repeat(5000);
    let event = SessionEvent::tool_call_with_output(
        session_id,
        "write".to_string(),
        Some("path=big.rs".to_string()),
        None,
        true,
        None,
        Some(long_diff),
    );

    // Should be truncated to ~4000 chars (use char count, not byte len due to ellipsis)
    assert!(event.diff.is_some());
    let char_count = event.diff.as_ref().unwrap().chars().count();
    assert!(
        char_count <= 4000,
        "Expected <= 4000 chars, got {}",
        char_count
    );
}

#[test]
fn test_session_event_new_fields_initialized() {
    let session_id = Uuid::new_v4().to_string();
    let event = SessionEvent::new(
        session_id,
        EventType::UserPrompt {
            intent: "test".to_string(),
        },
        "test content".to_string(),
    );

    // New fields should be None/empty by default
    assert!(event.cwd.is_none());
    assert!(event.tool_output.is_none());
    assert!(event.files_accessed.is_none());
    assert!(event.files_modified.is_empty());
    assert!(event.diff.is_none());
}

#[test]
fn test_extract_xml_tag_various_formats() {
    // Normal case
    assert_eq!(
        extract_xml_tag("<tag>value</tag>", "tag"),
        Some("value".to_string())
    );

    // With whitespace
    assert_eq!(
        extract_xml_tag("<tag>  value  </tag>", "tag"),
        Some("value".to_string())
    );

    // Nested in other content
    assert_eq!(
        extract_xml_tag("prefix <tag>value</tag> suffix", "tag"),
        Some("value".to_string())
    );

    // Missing tag
    assert_eq!(extract_xml_tag("<other>value</other>", "tag"), None);

    // Empty value
    assert_eq!(extract_xml_tag("<tag></tag>", "tag"), Some("".to_string()));
}

#[test]
fn test_session_lifecycle() {
    let mut session = SidecarSession::new(PathBuf::from("/project"), "Initial request".into());

    assert!(session.is_active());
    assert_eq!(session.event_count, 0);

    session.increment_events();
    session.touch_file(PathBuf::from("/src/lib.rs"));
    session.touch_file(PathBuf::from("/src/lib.rs")); // Duplicate

    assert_eq!(session.event_count, 1);
    assert_eq!(session.files_touched.len(), 1);

    session.end(Some("Summary".into()));
    assert!(!session.is_active());
    assert!(session.final_summary.is_some());
}

#[test]
fn test_truncate() {
    assert_eq!(truncate("short", 10), "short");
    assert_eq!(truncate("a longer string here", 10), "a longer …");
}

#[test]
fn test_checkpoint_creation() {
    let session_id = Uuid::new_v4();
    let checkpoint = Checkpoint::new(
        session_id,
        "Summary".into(),
        vec![Uuid::new_v4()],
        vec![PathBuf::from("/src/lib.rs")],
    );

    assert_eq!(checkpoint.session_id, session_id);
    assert!(checkpoint.embedding.is_none());
}

#[test]
fn test_event_serialization() {
    let session_id = Uuid::new_v4().to_string();
    let event = SessionEvent::reasoning(
        session_id.clone(),
        "Choosing approach A",
        Some(DecisionType::ApproachChoice),
    );

    let json = serde_json::to_string(&event).unwrap();
    let deserialized: SessionEvent = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.id, event.id);
    assert_eq!(deserialized.session_id, event.session_id);
}

mod lifecycle_and_filtering;

// =========================================================================
// SidecarEvent (UI Event) Tests
// =========================================================================

mod sidecar_event;
