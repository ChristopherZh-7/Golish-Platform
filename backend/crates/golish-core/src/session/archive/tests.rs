    use super::*;
    use crate::session::message::MessageRole;
    use serial_test::serial;
    use tempfile::TempDir;

    // ==========================================================================
    // SessionArchiveMetadata Tests
    // ==========================================================================

    mod metadata {
        use super::*;

        #[test]
        fn new_creates_metadata_with_all_fields() {
            let meta = SessionArchiveMetadata::new(
                "my-workspace",
                "/path/to/workspace".to_string(),
                "claude-3-opus",
                "anthropic",
                "dark",
                "high",
            );

            assert!(!meta.session_id.is_empty());
            assert_eq!(meta.workspace_label, "my-workspace");
            assert_eq!(meta.workspace_path, "/path/to/workspace");
            assert_eq!(meta.model, "claude-3-opus");
            assert_eq!(meta.provider, "anthropic");
            assert_eq!(meta.theme, "dark");
            assert_eq!(meta.reasoning_effort, "high");
        }

        #[test]
        fn new_generates_unique_session_ids() {
            let meta1 = SessionArchiveMetadata::new(
                "ws1",
                "/path".to_string(),
                "model",
                "provider",
                "default",
                "standard",
            );
            let meta2 = SessionArchiveMetadata::new(
                "ws2",
                "/path".to_string(),
                "model",
                "provider",
                "default",
                "standard",
            );

            assert_ne!(meta1.session_id, meta2.session_id);
        }

        #[test]
        fn serialization_roundtrip() {
            let meta = SessionArchiveMetadata::new(
                "test-workspace",
                "/test/path".to_string(),
                "test-model",
                "test-provider",
                "default",
                "standard",
            );

            let json = serde_json::to_string(&meta).unwrap();
            let restored: SessionArchiveMetadata = serde_json::from_str(&json).unwrap();

            assert_eq!(restored.session_id, meta.session_id);
            assert_eq!(restored.workspace_label, meta.workspace_label);
            assert_eq!(restored.workspace_path, meta.workspace_path);
            assert_eq!(restored.model, meta.model);
            assert_eq!(restored.provider, meta.provider);
            assert_eq!(restored.theme, meta.theme);
            assert_eq!(restored.reasoning_effort, meta.reasoning_effort);
        }

        #[test]
        fn deserializes_existing_format() {
            // Test compatibility with existing session files
            let json = r#"{
                "workspace_label": "golish",
                "workspace_path": "/Users/test/Code/golish",
                "model": "claude-opus-4-5@20251101",
                "provider": "anthropic_vertex",
                "theme": "default",
                "reasoning_effort": "standard"
            }"#;

            let meta: SessionArchiveMetadata = serde_json::from_str(json).unwrap();
            assert_eq!(meta.workspace_label, "golish");
            assert_eq!(meta.model, "claude-opus-4-5@20251101");
            // session_id should be generated via default
            assert!(!meta.session_id.is_empty());
        }
    }

    // ==========================================================================
    // SessionArchive Tests
    // ==========================================================================

    mod archive {
        use super::*;

        #[tokio::test]
        #[serial]
        async fn new_creates_archive() {
            let temp = TempDir::new().unwrap();
            std::env::set_var("VT_SESSION_DIR", temp.path());

            let meta = SessionArchiveMetadata::new(
                "test",
                "/test".to_string(),
                "model",
                "provider",
                "default",
                "standard",
            );

            let archive = SessionArchive::new(meta).await;
            assert!(archive.is_ok());

            std::env::remove_var("VT_SESSION_DIR");
        }

        #[tokio::test]
        #[serial]
        async fn finalize_creates_file() {
            let temp = TempDir::new().unwrap();
            std::env::set_var("VT_SESSION_DIR", temp.path());

            let meta = SessionArchiveMetadata::new(
                "finalize-test",
                "/test/workspace".to_string(),
                "test-model",
                "test-provider",
                "default",
                "standard",
            );

            let archive = SessionArchive::new(meta).await.unwrap();

            let messages = vec![
                SessionMessage::user("Hello"),
                SessionMessage::assistant("Hi there!"),
            ];
            let transcript = vec![
                "User: Hello".to_string(),
                "Assistant: Hi there!".to_string(),
            ];

            let path = archive.finalize(transcript, 2, vec![], messages).unwrap();

            assert!(path.exists());
            assert!(path.extension().map(|e| e == "json").unwrap_or(false));

            std::env::remove_var("VT_SESSION_DIR");
        }

        #[tokio::test]
        #[serial]
        async fn finalize_persists_all_data() {
            let temp = TempDir::new().unwrap();
            std::env::set_var("VT_SESSION_DIR", temp.path());

            let meta = SessionArchiveMetadata::new(
                "persist-test",
                "/workspace".to_string(),
                "claude-3",
                "anthropic",
                "dark",
                "high",
            );

            let archive = SessionArchive::new(meta).await.unwrap();
            let session_id = archive.session_id().to_string();

            let messages = vec![
                SessionMessage::user("What is 2+2?"),
                SessionMessage::assistant("2+2 equals 4."),
                SessionMessage::tool("Result: 4", "tool_123"),
            ];
            let transcript = vec![
                "User: What is 2+2?".to_string(),
                "Assistant: 2+2 equals 4.".to_string(),
                "Tool: Result: 4".to_string(),
            ];
            let tools = vec!["calculator".to_string()];

            let path = archive
                .finalize(transcript.clone(), 3, tools.clone(), messages)
                .unwrap();

            // Read and verify the saved file
            let content = std::fs::read_to_string(&path).unwrap();
            let snapshot: SessionSnapshot = serde_json::from_str(&content).unwrap();

            assert_eq!(snapshot.metadata.session_id, session_id);
            assert_eq!(snapshot.metadata.workspace_label, "persist-test");
            assert_eq!(snapshot.metadata.model, "claude-3");
            assert_eq!(snapshot.total_messages, 3);
            assert_eq!(snapshot.distinct_tools, tools);
            assert_eq!(snapshot.transcript, transcript);
            assert_eq!(snapshot.messages.len(), 3);
            assert_eq!(snapshot.messages[2].role, MessageRole::Tool);

            std::env::remove_var("VT_SESSION_DIR");
        }

        #[tokio::test]
        #[serial]
        async fn started_at_is_set_on_creation() {
            let temp = TempDir::new().unwrap();
            std::env::set_var("VT_SESSION_DIR", temp.path());

            let before = Utc::now();

            let meta = SessionArchiveMetadata::new(
                "timing-test",
                "/test".to_string(),
                "model",
                "provider",
                "default",
                "standard",
            );

            let archive = SessionArchive::new(meta).await.unwrap();
            let started = archive.started_at();

            let after = Utc::now();

            assert!(started >= before);
            assert!(started <= after);

            std::env::remove_var("VT_SESSION_DIR");
        }

        #[tokio::test]
        #[serial]
        async fn ended_at_is_set_on_finalize() {
            let temp = TempDir::new().unwrap();
            std::env::set_var("VT_SESSION_DIR", temp.path());

            let meta = SessionArchiveMetadata::new(
                "end-timing",
                "/test".to_string(),
                "model",
                "provider",
                "default",
                "standard",
            );

            let archive = SessionArchive::new(meta).await.unwrap();

            // Small delay
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

            let before_finalize = Utc::now();
            let path = archive.finalize(vec![], 0, vec![], vec![]).unwrap();
            let after_finalize = Utc::now();

            let content = std::fs::read_to_string(&path).unwrap();
            let snapshot: SessionSnapshot = serde_json::from_str(&content).unwrap();

            assert!(snapshot.ended_at >= before_finalize);
            assert!(snapshot.ended_at <= after_finalize);
            assert!(snapshot.ended_at > snapshot.started_at);

            std::env::remove_var("VT_SESSION_DIR");
        }
    }

    // ==========================================================================
    // Integration Tests - Compatibility with Existing Format
    // ==========================================================================

    mod compatibility {
        use super::*;

        #[tokio::test]
        #[serial]
        async fn produces_compatible_json_structure() {
            let temp = TempDir::new().unwrap();
            std::env::set_var("VT_SESSION_DIR", temp.path());

            let meta = SessionArchiveMetadata::new(
                "compat-test",
                "/Users/test/Code/project".to_string(),
                "claude-opus-4-5@20251101",
                "anthropic_vertex",
                "default",
                "standard",
            );

            let archive = SessionArchive::new(meta).await.unwrap();

            let messages = vec![
                SessionMessage::user("<context>\n<cwd>/Users/test</cwd>\n</context>\n\nHello"),
                SessionMessage::assistant("Hi there!"),
            ];
            let transcript = vec![
                "User: <context>...".to_string(),
                "Assistant: Hi there!".to_string(),
            ];

            let path = archive.finalize(transcript, 2, vec![], messages).unwrap();

            let content = std::fs::read_to_string(&path).unwrap();

            // Verify JSON structure matches existing format
            assert!(content.contains("\"metadata\""));
            assert!(content.contains("\"workspace_label\""));
            assert!(content.contains("\"workspace_path\""));
            assert!(content.contains("\"model\""));
            assert!(content.contains("\"provider\""));
            assert!(content.contains("\"theme\""));
            assert!(content.contains("\"reasoning_effort\""));
            assert!(content.contains("\"started_at\""));
            assert!(content.contains("\"ended_at\""));
            assert!(content.contains("\"total_messages\""));
            assert!(content.contains("\"distinct_tools\""));
            assert!(content.contains("\"transcript\""));
            assert!(content.contains("\"messages\""));

            // Verify role format is PascalCase
            assert!(
                content.contains("\"role\": \"User\"") || content.contains("\"role\":\"User\"")
            );
            assert!(
                content.contains("\"role\": \"Assistant\"")
                    || content.contains("\"role\":\"Assistant\"")
            );

            std::env::remove_var("VT_SESSION_DIR");
        }
    }
