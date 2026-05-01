use super::*;
use crate::session::archive::SessionArchiveMetadata;
use crate::session::message::SessionMessage;
use chrono::Utc;
use serial_test::serial;
use tempfile::TempDir;

fn create_test_snapshot(workspace: &str, session_id: &str) -> SessionSnapshot {
    SessionSnapshot {
        metadata: SessionArchiveMetadata {
            session_id: session_id.to_string(),
            workspace_label: workspace.to_string(),
            workspace_path: format!("/tmp/{}", workspace),
            model: "test-model".to_string(),
            provider: "test-provider".to_string(),
            theme: "default".to_string(),
            reasoning_effort: "standard".to_string(),
        },
        started_at: Utc::now(),
        ended_at: Utc::now(),
        total_messages: 2,
        distinct_tools: vec![],
        transcript: vec!["User: Hello".to_string(), "Assistant: Hi".to_string()],
        messages: vec![
            SessionMessage::user("Hello"),
            SessionMessage::assistant("Hi"),
        ],
    }
}

// ==========================================================================
// get_sessions_dir Tests
// ==========================================================================

mod sessions_dir {
    use super::*;

    #[test]
    #[serial]
    fn returns_custom_dir_from_env() {
        let temp = TempDir::new().unwrap();
        std::env::set_var("VT_SESSION_DIR", temp.path());

        let dir = get_sessions_dir().unwrap();
        assert_eq!(dir, temp.path());

        std::env::remove_var("VT_SESSION_DIR");
    }

    #[test]
    #[serial]
    fn creates_directory_if_not_exists() {
        let temp = TempDir::new().unwrap();
        let nested = temp.path().join("nested").join("sessions");
        std::env::set_var("VT_SESSION_DIR", &nested);

        let dir = get_sessions_dir().unwrap();
        assert!(dir.exists());

        std::env::remove_var("VT_SESSION_DIR");
    }
}

// ==========================================================================
// generate_filename Tests
// ==========================================================================

mod filename {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn generates_correct_format() {
        let timestamp = chrono::Utc
            .with_ymd_and_hms(2025, 12, 14, 8, 43, 35)
            .unwrap();
        let filename = generate_filename("my-project", &timestamp, "abc123def456");

        assert!(filename.starts_with("session-my-project-20251214T"));
        assert!(filename.ends_with(".json"));
        assert!(filename.contains("abc12"));
    }

    #[test]
    fn handles_short_session_id() {
        let timestamp = Utc::now();
        let filename = generate_filename("test", &timestamp, "ab");

        assert!(filename.contains("ab"));
    }
}

// ==========================================================================
// save_session Tests
// ==========================================================================

mod save {
    use super::*;

    #[test]
    #[serial]
    fn saves_session_to_disk() {
        let temp = TempDir::new().unwrap();
        std::env::set_var("VT_SESSION_DIR", temp.path());

        let snapshot = create_test_snapshot("test-workspace", "session123456");
        let path = save_session(temp.path(), &snapshot).unwrap();

        assert!(path.exists());

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("test-workspace"));
        assert!(content.contains("test-model"));

        std::env::remove_var("VT_SESSION_DIR");
    }

    #[test]
    #[serial]
    fn creates_valid_json() {
        let temp = TempDir::new().unwrap();
        let snapshot = create_test_snapshot("json-test", "jsonid12345");
        let path = save_session(temp.path(), &snapshot).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        let restored: SessionSnapshot = serde_json::from_str(&content).unwrap();

        assert_eq!(restored.metadata.workspace_label, "json-test");
        assert_eq!(restored.metadata.session_id, "jsonid12345");
        assert_eq!(restored.messages.len(), 2);
    }
}

// ==========================================================================
// find_session Tests
// ==========================================================================

mod find {
    use super::*;

    #[test]
    #[serial]
    fn finds_by_session_id_prefix() {
        let temp = TempDir::new().unwrap();
        std::env::set_var("VT_SESSION_DIR", temp.path());

        let snapshot = create_test_snapshot("find-test", "unique123456789");
        save_session(temp.path(), &snapshot).unwrap();

        let found = find_session("unique123").unwrap();
        assert!(found.is_some());
        assert_eq!(
            found.unwrap().snapshot.metadata.session_id,
            "unique123456789"
        );

        std::env::remove_var("VT_SESSION_DIR");
    }

    #[test]
    #[serial]
    fn returns_none_for_nonexistent() {
        let temp = TempDir::new().unwrap();
        std::env::set_var("VT_SESSION_DIR", temp.path());

        let found = find_session("nonexistent").unwrap();
        assert!(found.is_none());

        std::env::remove_var("VT_SESSION_DIR");
    }

    #[test]
    #[serial]
    fn finds_by_filename_content() {
        let temp = TempDir::new().unwrap();
        std::env::set_var("VT_SESSION_DIR", temp.path());

        let snapshot = create_test_snapshot("myproject", "projid12345");
        save_session(temp.path(), &snapshot).unwrap();

        let found = find_session("myproject").unwrap();
        assert!(found.is_some());

        std::env::remove_var("VT_SESSION_DIR");
    }
}

// ==========================================================================
// list_sessions Tests
// ==========================================================================

mod list {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    #[serial]
    fn returns_empty_for_empty_dir() {
        let temp = TempDir::new().unwrap();
        std::env::set_var("VT_SESSION_DIR", temp.path());

        let sessions = list_sessions(10).unwrap();
        assert!(sessions.is_empty());

        std::env::remove_var("VT_SESSION_DIR");
    }

    #[test]
    #[serial]
    fn returns_all_sessions() {
        let temp = TempDir::new().unwrap();
        std::env::set_var("VT_SESSION_DIR", temp.path());

        for i in 0..3 {
            let snapshot =
                create_test_snapshot(&format!("workspace-{}", i), &format!("id{}", i));
            save_session(temp.path(), &snapshot).unwrap();
            thread::sleep(Duration::from_millis(10));
        }

        let sessions = list_sessions(0).unwrap();
        assert_eq!(sessions.len(), 3);

        std::env::remove_var("VT_SESSION_DIR");
    }

    #[test]
    #[serial]
    fn respects_limit() {
        let temp = TempDir::new().unwrap();
        std::env::set_var("VT_SESSION_DIR", temp.path());

        for i in 0..5 {
            let snapshot = create_test_snapshot(&format!("limit-{}", i), &format!("lid{}", i));
            save_session(temp.path(), &snapshot).unwrap();
        }

        let sessions = list_sessions(2).unwrap();
        assert_eq!(sessions.len(), 2);

        std::env::remove_var("VT_SESSION_DIR");
    }

    #[test]
    #[serial]
    fn sorts_by_date_descending() {
        let temp = TempDir::new().unwrap();
        std::env::set_var("VT_SESSION_DIR", temp.path());

        for i in 0..3 {
            let mut snapshot =
                create_test_snapshot(&format!("sort-{}", i), &format!("sid{}", i));
            thread::sleep(Duration::from_millis(50));
            snapshot.started_at = Utc::now();
            save_session(temp.path(), &snapshot).unwrap();
        }

        let sessions = list_sessions(0).unwrap();

        for i in 0..sessions.len() - 1 {
            assert!(sessions[i].started_at >= sessions[i + 1].started_at);
        }

        std::env::remove_var("VT_SESSION_DIR");
    }

    #[test]
    #[serial]
    fn ignores_non_json_files() {
        let temp = TempDir::new().unwrap();
        std::env::set_var("VT_SESSION_DIR", temp.path());

        let snapshot = create_test_snapshot("valid", "validid");
        save_session(temp.path(), &snapshot).unwrap();

        fs::write(temp.path().join("readme.txt"), "not a session").unwrap();
        fs::write(temp.path().join("data.csv"), "a,b,c").unwrap();

        let sessions = list_sessions(0).unwrap();
        assert_eq!(sessions.len(), 1);

        std::env::remove_var("VT_SESSION_DIR");
    }

    #[test]
    #[serial]
    fn ignores_invalid_json() {
        let temp = TempDir::new().unwrap();
        std::env::set_var("VT_SESSION_DIR", temp.path());

        let snapshot = create_test_snapshot("valid", "validid2");
        save_session(temp.path(), &snapshot).unwrap();

        fs::write(temp.path().join("invalid.json"), "{ not valid json }").unwrap();

        let sessions = list_sessions(0).unwrap();
        assert_eq!(sessions.len(), 1);

        std::env::remove_var("VT_SESSION_DIR");
    }
}
