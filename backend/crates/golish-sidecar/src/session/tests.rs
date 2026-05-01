use super::*;
use tempfile::TempDir;

async fn setup_test_dir() -> TempDir {
    TempDir::new().unwrap()
}

#[tokio::test]
async fn test_session_create_and_load() {
    let temp = setup_test_dir().await;
    let sessions_dir = temp.path();

    let session = Session::create(
        sessions_dir,
        "test-session".to_string(),
        PathBuf::from("/home/user/project"),
        "Build something amazing".to_string(),
    )
    .await
    .unwrap();

    assert_eq!(session.meta().session_id, "test-session");
    assert_eq!(session.meta().status, SessionStatus::Active);

    assert!(sessions_dir.join("test-session/state.md").exists());
    assert!(sessions_dir.join("test-session/patches/staged").exists());
    assert!(sessions_dir.join("test-session/patches/applied").exists());
    assert!(sessions_dir.join("test-session/artifacts/pending").exists());
    assert!(sessions_dir.join("test-session/artifacts/applied").exists());

    let loaded = Session::load(sessions_dir, "test-session").await.unwrap();
    assert_eq!(loaded.meta().session_id, "test-session");
    assert_eq!(loaded.meta().initial_request, "Build something amazing");
}

#[tokio::test]
async fn test_session_state_operations() {
    let temp = setup_test_dir().await;
    let sessions_dir = temp.path();

    let mut session = Session::create(
        sessions_dir,
        "state-test".to_string(),
        PathBuf::from("/tmp"),
        "Test state ops".to_string(),
    )
    .await
    .unwrap();

    let state = session.read_state().await.unwrap();
    assert!(state.contains("Test state ops"));
    assert!(state.contains("# Goal"));

    let new_body = "# Goal\nUpdated goal\n\n## Progress\nMade progress!";
    session.update_state(new_body).await.unwrap();

    let updated = session.read_state().await.unwrap();
    assert!(updated.contains("Updated goal"));
    assert!(updated.contains("Made progress!"));
}

#[tokio::test]
async fn test_session_lifecycle() {
    let temp = setup_test_dir().await;
    let sessions_dir = temp.path();

    let mut session = Session::create(
        sessions_dir,
        "lifecycle-test".to_string(),
        PathBuf::from("/tmp"),
        "Test lifecycle".to_string(),
    )
    .await
    .unwrap();

    assert_eq!(session.meta().status, SessionStatus::Active);

    session.complete().await.unwrap();
    assert_eq!(session.meta().status, SessionStatus::Completed);

    let loaded = Session::load(sessions_dir, "lifecycle-test").await.unwrap();
    assert_eq!(loaded.meta().status, SessionStatus::Completed);
}

#[tokio::test]
async fn test_list_sessions() {
    let temp = setup_test_dir().await;
    let sessions_dir = temp.path();

    Session::create(
        sessions_dir,
        "session-1".to_string(),
        PathBuf::from("/tmp"),
        "First".to_string(),
    )
    .await
    .unwrap();

    Session::create(
        sessions_dir,
        "session-2".to_string(),
        PathBuf::from("/tmp"),
        "Second".to_string(),
    )
    .await
    .unwrap();

    let sessions = list_sessions(sessions_dir).await.unwrap();
    assert_eq!(sessions.len(), 2);
}

#[tokio::test]
async fn test_frontmatter_parsing() {
    let content = r#"---
session_id: test-123
created_at: 2025-12-10T14:30:00Z
updated_at: 2025-12-10T15:00:00Z
status: active
cwd: /home/user/project
initial_request: Build something
---

# Goal
Build something

## Progress
Working on it.
"#;

    let (meta, body) = Session::parse_state_file(content).unwrap();
    assert_eq!(meta.session_id, "test-123");
    assert_eq!(meta.status, SessionStatus::Active);
    assert!(body.contains("# Goal"));
    assert!(body.contains("Working on it."));
}
