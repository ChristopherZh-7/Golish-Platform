use super::*;

#[test]
fn test_commit_boundary_detector_file_tracking() {
    let mut detector = CommitBoundaryDetector::new();
    let session_id = Uuid::new_v4().to_string();

    // Add some file edits
    let event1 = SessionEvent::file_edit(
        session_id.clone(),
        PathBuf::from("/src/lib.rs"),
        FileOperation::Modify,
        None,
    );
    let event2 = SessionEvent::file_edit(
        session_id,
        PathBuf::from("/src/main.rs"),
        FileOperation::Modify,
        None,
    );

    detector.check_boundary(&event1);
    detector.check_boundary(&event2);

    assert_eq!(detector.pending_files().len(), 2);
}

#[test]
fn test_commit_boundary_completion_signal() {
    let mut detector = CommitBoundaryDetector::with_thresholds(2, 60);
    let session_id = Uuid::new_v4().to_string();

    // Add file edits
    detector.check_boundary(&SessionEvent::file_edit(
        session_id.clone(),
        PathBuf::from("/src/a.rs"),
        FileOperation::Modify,
        None,
    ));
    detector.check_boundary(&SessionEvent::file_edit(
        session_id.clone(),
        PathBuf::from("/src/b.rs"),
        FileOperation::Create,
        None,
    ));

    // Add completion signal
    let boundary = detector.check_boundary(&SessionEvent::reasoning(
        session_id,
        "Implementation is complete",
        None,
    ));

    assert!(boundary.is_some());
    let boundary = boundary.unwrap();
    assert_eq!(boundary.files_in_scope.len(), 2);
    assert!(boundary.reason.contains("Completion"));
}

#[test]
fn test_commit_boundary_user_approval() {
    let mut detector = CommitBoundaryDetector::with_thresholds(1, 60);
    let session_id = Uuid::new_v4().to_string();

    detector.check_boundary(&SessionEvent::file_edit(
        session_id.clone(),
        PathBuf::from("/src/lib.rs"),
        FileOperation::Modify,
        None,
    ));

    let boundary = detector.check_boundary(&SessionEvent::feedback(
        session_id,
        FeedbackType::Approve,
        Some("edit".into()),
        None,
    ));

    assert!(boundary.is_some());
    assert!(boundary.unwrap().reason.contains("approved"));
}

#[test]
fn test_commit_boundary_clear() {
    let mut detector = CommitBoundaryDetector::new();
    let session_id = Uuid::new_v4().to_string();

    detector.check_boundary(&SessionEvent::file_edit(
        session_id,
        PathBuf::from("/src/lib.rs"),
        FileOperation::Modify,
        None,
    ));

    assert!(!detector.pending_files().is_empty());

    detector.clear();

    assert!(detector.pending_files().is_empty());
}

#[test]
fn test_session_export() {
    let session = SidecarSession::new(PathBuf::from("/project"), "Test request".into());
    let session_id = session.id;
    let session_id_str = session_id.to_string();

    let events = vec![
        SessionEvent::user_prompt(session_id_str.clone(), "Add feature"),
        SessionEvent::file_edit(
            session_id_str,
            PathBuf::from("/src/lib.rs"),
            FileOperation::Modify,
            None,
        ),
    ];

    let checkpoints = vec![Checkpoint::new(
        session_id,
        "Test checkpoint".into(),
        vec![events[0].id],
        vec![],
    )];

    let export = SessionExport::new(session, events, checkpoints);

    // Test JSON serialization
    let json = export.to_json().unwrap();
    assert!(json.contains("Test request"));

    // Test deserialization
    let imported = SessionExport::from_json(&json).unwrap();
    assert_eq!(imported.version, SessionExport::VERSION);
    assert_eq!(imported.session.id, session_id);
    assert_eq!(imported.events.len(), 2);
    assert_eq!(imported.checkpoints.len(), 1);
}

#[test]
fn test_should_embed_filtering() {
    let session_id = Uuid::new_v4().to_string();

    // User prompts should be embedded
    let user_prompt = SessionEvent::user_prompt(session_id.clone(), "Add authentication");
    assert!(user_prompt.should_embed(), "user_prompt should be embedded");

    // Agent reasoning should be embedded
    let reasoning = SessionEvent::reasoning(session_id.clone(), "I'll use JWT for auth", None);
    assert!(reasoning.should_embed(), "reasoning should be embedded");

    // File edits should NOT be embedded (structured, search by path)
    let file_edit = SessionEvent::file_edit(
        session_id.clone(),
        PathBuf::from("src/auth.rs"),
        FileOperation::Modify,
        None,
    );
    assert!(
        !file_edit.should_embed(),
        "file_edit should NOT be embedded"
    );

    // Regular tool calls should NOT be embedded
    let tool_call = SessionEvent::tool_call_with_output(
        session_id.clone(),
        "write".to_string(),
        Some("path=test.rs".to_string()),
        None,
        true,
        Some("File written".to_string()),
        None,
    );
    assert!(
        !tool_call.should_embed(),
        "write tool should NOT be embedded"
    );

    // Read tool calls WITH output SHOULD be embedded
    let read_tool = SessionEvent::tool_call_with_output(
        session_id.clone(),
        "read_file".to_string(),
        Some("path=src/main.rs".to_string()),
        None,
        true,
        Some("fn main() { println!(\"Hello\"); }".to_string()),
        None,
    );
    assert!(
        read_tool.should_embed(),
        "read_file with output should be embedded"
    );

    // Read tool without output should NOT be embedded
    let read_no_output = SessionEvent::tool_call_with_output(
        session_id.clone(),
        "read_file".to_string(),
        Some("path=missing.rs".to_string()),
        None,
        false,
        None, // No output
        None,
    );
    assert!(
        !read_no_output.should_embed(),
        "read_file without output should NOT be embedded"
    );

    // Grep tool with output should be embedded
    let grep_tool = SessionEvent::tool_call_with_output(
        session_id,
        "grep".to_string(),
        Some("pattern=authenticate".to_string()),
        None,
        true,
        Some("src/auth.rs:1: fn authenticate".to_string()),
        None,
    );
    assert!(
        grep_tool.should_embed(),
        "grep with output should be embedded"
    );
}
