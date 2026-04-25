//! State tests.

use super::*;
use super::*;
use tempfile::TempDir;

fn test_config(temp_dir: &std::path::Path) -> SidecarConfig {
    SidecarConfig {
        enabled: true,
        sessions_dir: Some(temp_dir.to_path_buf()),
        retention_days: 0,
        max_state_size: 16 * 1024,
        write_raw_events: false,
        use_llm_for_state: false,
        capture_tool_calls: true,
        capture_reasoning: true,
        synthesis_enabled: true,
        synthesis_backend: golish_synthesis::SynthesisBackend::Template,
        artifact_synthesis_backend: golish_artifacts::ArtifactSynthesisBackend::Template,
        synthesis_vertex: Default::default(),
        synthesis_openai: Default::default(),
        synthesis_grok: Default::default(),
    }
}

#[tokio::test]
async fn test_sidecar_state_creation() {
    let state = SidecarState::new();
    let status = state.status();
    assert!(!status.active_session);
    assert!(status.session_id.is_none());
}

#[tokio::test]
async fn test_sidecar_initialization() {
    let temp = TempDir::new().unwrap();
    let config = test_config(temp.path());
    let state = SidecarState::with_config(config);

    state.initialize(temp.path().to_path_buf()).await.unwrap();

    let status = state.status();
    assert!(status.enabled);
    assert!(status.workspace_path.is_some());
}

#[tokio::test]
async fn test_session_lifecycle() {
    let temp = TempDir::new().unwrap();
    let config = test_config(temp.path());
    let state = SidecarState::with_config(config);

    state.initialize(temp.path().to_path_buf()).await.unwrap();

    let session_id = state.start_session("Test request").unwrap();
    assert!(!session_id.is_empty());
    assert!(state.current_session_id().is_some());

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let _meta = state.end_session().unwrap();
    assert!(state.current_session_id().is_none());
}

#[tokio::test]
async fn test_status() {
    let temp = TempDir::new().unwrap();
    let config = test_config(temp.path());
    let state = SidecarState::with_config(config);

    let status = state.status();
    assert!(status.enabled);
    assert!(!status.active_session);
}

#[tokio::test]
async fn test_start_session_idempotent() {
    // Test that calling start_session when a session already exists
    // returns the existing session ID (not a new one)
    let temp = TempDir::new().unwrap();
    let config = test_config(temp.path());
    let state = SidecarState::with_config(config);

    state.initialize(temp.path().to_path_buf()).await.unwrap();

    // First call creates a session
    let session_id_1 = state.start_session("First request").unwrap();
    assert!(!session_id_1.is_empty());

    // Second call should return the same session ID
    let session_id_2 = state.start_session("Second request").unwrap();
    assert_eq!(
        session_id_1, session_id_2,
        "start_session should be idempotent"
    );

    // Third call should also return the same session ID
    let session_id_3 = state.start_session("Third request").unwrap();
    assert_eq!(
        session_id_1, session_id_3,
        "start_session should be idempotent"
    );
}

#[tokio::test]
async fn test_start_session_concurrent_returns_same_id() {
    // Test that concurrent calls to start_session all return the same session ID
    // This verifies the race condition fix - the atomic check-and-set
    use std::sync::Arc;

    let temp = TempDir::new().unwrap();
    let config = test_config(temp.path());
    let state = Arc::new(SidecarState::with_config(config));

    state.initialize(temp.path().to_path_buf()).await.unwrap();

    // Spawn multiple concurrent tasks that all try to start a session
    let mut handles = vec![];
    for i in 0..10 {
        let state_clone = Arc::clone(&state);
        let handle =
            tokio::spawn(async move { state_clone.start_session(&format!("Request {}", i)) });
        handles.push(handle);
    }

    // Collect all results
    let mut session_ids = vec![];
    for handle in handles {
        let result = handle.await.unwrap();
        session_ids.push(result.unwrap());
    }

    // All session IDs should be the same
    let first_id = &session_ids[0];
    for (i, id) in session_ids.iter().enumerate() {
        assert_eq!(
            first_id, id,
            "All concurrent start_session calls should return the same ID. \
             Call {} returned {} but expected {}",
            i, id, first_id
        );
    }
}

#[tokio::test]
async fn test_end_session_allows_new_session() {
    // Test that after ending a session, a new session can be started
    let temp = TempDir::new().unwrap();
    let config = test_config(temp.path());
    let state = SidecarState::with_config(config);

    state.initialize(temp.path().to_path_buf()).await.unwrap();

    // Start first session
    let session_id_1 = state.start_session("First session").unwrap();
    assert!(!session_id_1.is_empty());

    // Wait for session creation to complete
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // End the session
    let _ = state.end_session().unwrap();
    assert!(state.current_session_id().is_none());

    // Start a new session - should get a different ID
    let session_id_2 = state.start_session("Second session").unwrap();
    assert!(!session_id_2.is_empty());
    assert_ne!(
        session_id_1, session_id_2,
        "After ending a session, a new session should have a different ID"
    );
}

#[tokio::test]
async fn test_per_session_sidecar_isolation() {
    // Test that multiple SidecarState instances are completely isolated.
    // This simulates the per-session architecture where each AgentBridge
    // gets its own SidecarState to prevent cross-session blocking.
    use std::sync::Arc;

    let temp = TempDir::new().unwrap();

    // Create two separate SidecarState instances (simulating two tabs/sessions)
    let config_a = test_config(temp.path());
    let config_b = test_config(temp.path());

    let state_a = Arc::new(SidecarState::with_config(config_a));
    let state_b = Arc::new(SidecarState::with_config(config_b));

    // Initialize both
    state_a.initialize(temp.path().to_path_buf()).await.unwrap();
    state_b.initialize(temp.path().to_path_buf()).await.unwrap();

    // Start sessions on both - they should get different IDs
    let session_id_a = state_a.start_session("Session A request").unwrap();
    let session_id_b = state_b.start_session("Session B request").unwrap();

    assert!(!session_id_a.is_empty());
    assert!(!session_id_b.is_empty());
    assert_ne!(
        session_id_a, session_id_b,
        "Different SidecarState instances should have different session IDs"
    );

    // Each state should only know about its own session
    assert_eq!(state_a.current_session_id(), Some(session_id_a.clone()));
    assert_eq!(state_b.current_session_id(), Some(session_id_b.clone()));

    // Ending session A should not affect session B
    let _ = state_a.end_session().unwrap();
    assert!(state_a.current_session_id().is_none());
    assert_eq!(
        state_b.current_session_id(),
        Some(session_id_b.clone()),
        "Ending session A should not affect session B"
    );

    // Session B should still be able to capture events independently
    assert!(state_b.current_session_id().is_some());
}

#[tokio::test]
async fn test_per_session_sidecar_concurrent_initialization() {
    // Test that multiple SidecarState instances can be initialized concurrently
    // without blocking each other. This verifies the fix for the multi-tab
    // agent initialization issue.
    use std::sync::Arc;
    use std::time::Instant;

    let temp = TempDir::new().unwrap();

    // Create 5 separate SidecarState instances
    let states: Vec<Arc<SidecarState>> = (0..5)
        .map(|_| {
            let config = test_config(temp.path());
            Arc::new(SidecarState::with_config(config))
        })
        .collect();

    let start = Instant::now();

    // Initialize and start sessions on all states concurrently
    let mut handles = vec![];
    for (i, state) in states.iter().enumerate() {
        let state_clone = Arc::clone(state);
        let workspace = temp.path().to_path_buf();
        let handle = tokio::spawn(async move {
            state_clone.initialize(workspace).await.unwrap();
            let session_id = state_clone
                .start_session(&format!("Request from session {}", i))
                .unwrap();
            (i, session_id)
        });
        handles.push(handle);
    }

    // Collect all results
    let mut results = vec![];
    for handle in handles {
        let result = handle.await.unwrap();
        results.push(result);
    }

    let duration = start.elapsed();

    // All sessions should have been created successfully
    assert_eq!(results.len(), 5);

    // All session IDs should be unique (different instances)
    let mut session_ids: Vec<String> = results.iter().map(|(_, id)| id.clone()).collect();
    session_ids.sort();
    session_ids.dedup();
    assert_eq!(
        session_ids.len(),
        5,
        "Each SidecarState instance should have a unique session ID"
    );

    // The concurrent operations should complete relatively quickly
    // (not serialized by a shared lock)
    // Note: We don't assert on exact timing to avoid flaky tests,
    // but logging helps verify the fix works
    tracing::info!(
        "Concurrent initialization of 5 SidecarState instances completed in {:?}",
        duration
    );
}
