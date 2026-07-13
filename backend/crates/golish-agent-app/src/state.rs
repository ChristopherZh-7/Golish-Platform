//! Agent service state.
//!
//! - [`AiState`] — per-session agent runtime state (per-session bridges /
//!   runtime). Moved out of the god-crate `golish` (was `ai/commands/mod.rs`)
//!   so the agent command surface can later live in this crate without a
//!   `golish ↔ golish-agent-app` cycle (crate-per-service M4-A).
//! - [`AgentState`] — the narrow managed state the agent `#[tauri::command]`
//!   handlers take instead of the monolithic `golish::AppState`. It aggregates
//!   the same shared `Arc` handles the commands need; the main app constructs it
//!   via `AppState::extract_agent_state()` so both share one set of handles.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use parking_lot::Mutex;
use sqlx::PgPool;
use tokio::sync::RwLock;

use golish_agent_bridge::{
    AgentBridge, SessionRequestBusy, SessionRequestSlot, SessionRequestTransitionLease,
};
use golish_app_core::domain::operator::TrustedOperatorPrincipalProvider;
use golish_app_core::ports::pentest::PentestToolFactory;
use golish_app_core::pty_interactive::PtyOutputTap;
use golish_app_core::GolishError;
use golish_core::runtime::GolishRuntime;
use golish_db::DbReadyGate;
use golish_reporting_app::ReportArtifactStoreFactory;

// Re-export so the moved `conversation_store` keeps resolving `crate::state::DbState`.
pub use golish_app_core::DbState;

/// Shared AI state supporting multiple per-session agents.
/// Uses tokio RwLock for async compatibility with AgentBridge methods.
///
/// IMPORTANT: Bridges are wrapped in Arc to allow cloning references without
/// holding the map lock during long-running operations like execute().
/// This enables concurrent agent execution across multiple tabs.
///
/// `Clone` shares the underlying `Arc`s (the bridges/runtime), so a clone held
/// by `AgentState` and the original held by `AppState` see the same agents.
#[derive(Clone)]
pub struct AiState {
    /// Map of session_id -> Arc<AgentBridge> for per-tab AI isolation.
    /// The Arc wrapper allows commands to clone the bridge reference and
    /// release the map lock before calling long-running async methods.
    pub bridges: Arc<RwLock<HashMap<String, Arc<AgentBridge>>>>,
    /// Stable request authority per logical session. Slots intentionally outlive
    /// bridge removal so late clones from an invalidated generation can never
    /// become current again.
    session_slots: Arc<RwLock<HashMap<String, Arc<AiSessionSlot>>>>,
    /// Runtime abstraction for event emission and approval handling.
    /// Stored here for later phases when AgentBridge will use it directly.
    /// Currently created during init but the existing event_tx path is used.
    pub runtime: Arc<RwLock<Option<Arc<dyn GolishRuntime>>>>,
}

struct AiSessionSlot {
    request: Arc<SessionRequestSlot>,
    lifecycle: Arc<tokio::sync::Mutex<()>>,
}

impl Default for AiSessionSlot {
    fn default() -> Self {
        Self {
            request: Arc::new(SessionRequestSlot::default()),
            lifecycle: Arc::new(tokio::sync::Mutex::new(())),
        }
    }
}

/// Reservation held while a replacement bridge is being constructed. It blocks
/// new requests on the old generation and serializes init vs shutdown for only
/// this logical session; unrelated sessions remain fully parallel.
pub(crate) struct SessionBridgeInstall {
    session_id: String,
    slot: Arc<AiSessionSlot>,
    transition: SessionRequestTransitionLease,
    _lifecycle: tokio::sync::OwnedMutexGuard<()>,
}

impl Default for AiState {
    fn default() -> Self {
        Self {
            bridges: Arc::new(RwLock::new(HashMap::new())),
            session_slots: Arc::new(RwLock::new(HashMap::new())),
            runtime: Arc::new(RwLock::new(None)),
        }
    }
}

/// Build a `GolishError` for an uninitialized session.
pub fn ai_session_not_initialized_error(session_id: &str) -> GolishError {
    GolishError::Internal(format!(
        "AI agent not initialized for session '{}'. Call init_ai_session first.",
        session_id
    ))
}

impl AiState {
    pub fn new() -> Self {
        Self::default()
    }

    // ========== Session-specific bridge methods ==========

    /// Get an Arc clone of a session's bridge.
    ///
    /// This is the preferred method for accessing bridges as it allows releasing
    /// the map lock immediately. Use this for long-running operations like execute().
    pub async fn get_session_bridge(&self, session_id: &str) -> Option<Arc<AgentBridge>> {
        self.bridges.read().await.get(session_id).cloned()
    }

    /// Get a read guard to the bridges map.
    ///
    /// WARNING: Only use for short operations. For long-running async operations,
    /// use get_session_bridge() instead to avoid blocking other sessions.
    pub async fn get_bridges(
        &self,
    ) -> tokio::sync::RwLockReadGuard<'_, HashMap<String, Arc<AgentBridge>>> {
        self.bridges.read().await
    }

    /// Check if a session has an initialized AI agent.
    pub async fn has_session_bridge(&self, session_id: &str) -> bool {
        self.bridges.read().await.contains_key(session_id)
    }

    async fn get_or_create_session_slot(&self, session_id: &str) -> Arc<AiSessionSlot> {
        if let Some(slot) = self.session_slots.read().await.get(session_id).cloned() {
            return slot;
        }
        self.session_slots
            .write()
            .await
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(AiSessionSlot::default()))
            .clone()
    }

    /// Reserve this logical session before doing any replacement work. Busy old
    /// requests fail here, so provider construction cannot disrupt or replace
    /// the current bridge.
    pub(crate) async fn begin_session_bridge_install(
        &self,
        session_id: &str,
    ) -> anyhow::Result<SessionBridgeInstall> {
        // Opportunistically collect tombstones whose late bridge/request refs
        // finished unwinding after an earlier shutdown attempt. This keeps many
        // closed busy tabs from accumulating forever without weakening the
        // stable-slot boundary for any still-live generation.
        self.prune_inactive_session_slots().await;
        let slot = self.get_or_create_session_slot(session_id).await;
        let lifecycle = slot
            .lifecycle
            .clone()
            .try_lock_owned()
            .map_err(|_| anyhow::Error::new(SessionRequestBusy))?;
        let transition = slot.request.try_begin_transition()?;
        Ok(SessionBridgeInstall {
            session_id: session_id.to_string(),
            slot,
            transition,
            _lifecycle: lifecycle,
        })
    }

    pub(crate) async fn finish_session_bridge_install<F>(
        &self,
        mut install: SessionBridgeInstall,
        mut bridge: AgentBridge,
        on_published: F,
    ) -> anyhow::Result<(Arc<AgentBridge>, Option<Arc<AgentBridge>>)>
    where
        F: FnOnce(&Arc<AgentBridge>),
    {
        // Take the map writer before publishing the new generation so readers
        // never observe an old bridge that still appears current.
        let mut bridges = self.bridges.write().await;
        let generation = install.transition.activate_next_generation()?;
        bridge.bind_session_request_slot(install.slot.request.clone(), generation);
        if let Some(old) = bridges.get(&install.session_id) {
            bridge.inherit_background_notes(old.background_notes_handle());
        }
        let current = Arc::new(bridge);
        let replaced = bridges.insert(install.session_id.clone(), current.clone());
        if let Some(old) = replaced.as_ref() {
            old.retire_session_generation();
        }
        // The transition still owns the shared in-flight bit here. Host
        // listeners were pre-subscribed before this method, so activation after
        // old retirement has no broadcast gap and no new top-level request can
        // launch until both listener tasks exist.
        on_published(&current);
        Ok((current, replaced))
    }

    /// Atomically install an already-built bridge as the next generation.
    /// Production init reserves the session before construction; this
    /// convenience remains for tests and internal callers with a ready bridge.
    pub async fn install_session_bridge(
        &self,
        session_id: String,
        bridge: AgentBridge,
    ) -> anyhow::Result<Option<Arc<AgentBridge>>> {
        let install = self.begin_session_bridge_install(&session_id).await?;
        self.finish_session_bridge_install(install, bridge, |_| {})
            .await
            .map(|(_, replaced)| replaced)
    }

    /// Insert a bridge for a session.
    ///
    /// The bridge is wrapped in Arc for concurrent access.
    pub async fn insert_session_bridge(
        &self,
        session_id: String,
        bridge: AgentBridge,
    ) -> anyhow::Result<()> {
        self.install_session_bridge(session_id, bridge)
            .await
            .map(|_| ())
    }

    /// Remove and return the bridge for a session.
    ///
    /// Returns the Arc-wrapped bridge if it existed.
    pub async fn remove_session_bridge(&self, session_id: &str) -> Option<Arc<AgentBridge>> {
        let slot = self.get_or_create_session_slot(session_id).await;
        let _lifecycle = slot.lifecycle.clone().lock_owned().await;
        let mut bridges = self.bridges.write().await;
        // Invalidation does not wait for an active owner: shutdown must stop new
        // work immediately, then signal cancellation to the removed bridge.
        slot.request.invalidate();
        let removed = bridges.remove(session_id);
        if let Some(bridge) = removed.as_ref() {
            bridge.retire_session_generation();
        }
        removed
    }

    /// Remove an inactive logical-session tombstone once no concurrent
    /// init/shutdown reservation still holds the slot. Late bridge clones keep
    /// their own invalidated request-slot Arc, so pruning the map entry cannot
    /// make an old generation current again.
    pub(crate) async fn prune_inactive_session_slot(&self, session_id: &str) -> bool {
        let mut slots = self.session_slots.write().await;
        let Some(slot) = slots.get(session_id) else {
            return false;
        };
        // Bridges and request leases retain the inner request-slot Arc directly,
        // not the AiSessionSlot wrapper. Removing the tombstone while either is
        // alive would let same-id init create a fresh gate and run concurrently
        // with the invalidated-but-still-unwinding old owner.
        if Arc::strong_count(slot) != 1
            || Arc::strong_count(&slot.request) != 1
            || self.bridges.read().await.contains_key(session_id)
        {
            return false;
        }
        slots.remove(session_id);
        true
    }

    async fn prune_inactive_session_slots(&self) -> usize {
        let current_sessions: HashSet<String> = self.bridges.read().await.keys().cloned().collect();
        let mut slots = self.session_slots.write().await;
        let before = slots.len();
        slots.retain(|session_id, slot| {
            current_sessions.contains(session_id)
                || Arc::strong_count(slot) != 1
                || Arc::strong_count(&slot.request) != 1
        });
        before.saturating_sub(slots.len())
    }
}

/// Narrow managed state for the agent service's Tauri command handlers.
///
/// Holds exactly the shared handles `ai/commands/*` need — a subset of
/// `golish::AppState` (everything except the platform-only `command_index`,
/// `telemetry_stats`, `langfuse_active`). The main app builds one via
/// `AppState::extract_agent_state()`, sharing the same `Arc`s, so behaviour is
/// identical to taking the full `AppState`. Letting commands take this (instead
/// of `golish::AppState`) is what unblocks moving the agent command surface into
/// this crate (M4).
pub struct AgentState {
    pub ai_state: AiState,
    pub pty_manager: Arc<golish_pty::PtyManager>,
    pub indexer_state: Arc<golish_indexer::IndexerState>,
    pub settings_manager: Arc<golish_settings::SettingsManager>,
    pub sidecar_config: golish_sidecar::SidecarConfig,
    pub sidecar_state: Arc<golish_sidecar::SidecarState>,
    pub mcp_manager: Arc<RwLock<Option<Arc<golish_mcp::McpManager>>>>,
    pub pentest_config_manager: Arc<golish_pentest::ConfigManager>,
    pub pty_output_tap: Arc<PtyOutputTap>,
    pub active_terminal_session: Arc<Mutex<Option<String>>>,
    pub pentest_busy_sessions: Arc<Mutex<HashSet<String>>>,
    pub db_pool: Arc<PgPool>,
    pub db_ready: DbReadyGate,
    /// Process-shared canonical Memory Fabric UoW. Per-session bridge setup may
    /// clone this handle but never owns or starts projector workers.
    pub knowledge_memory: Arc<dyn golish_memory_app::KnowledgeUnitOfWork>,
    /// Server-owned local actor identity. Privileged commands resolve this
    /// provider instead of accepting actor UUIDs in request DTOs.
    pub operator_principal_provider: Arc<dyn TrustedOperatorPrincipalProvider>,
    /// Server-resolved project-local Reporting artifact storage. Callers can
    /// select only a report/revision; project roots and storage keys never
    /// cross IPC or model boundaries.
    pub reporting_artifact_store_factory: Arc<dyn ReportArtifactStoreFactory>,
    /// Inbound port supplying the pentest tool set the bridge registers, so this
    /// crate needs no compile-time `golish-pentest-app` dependency (S1-3). The
    /// composition root (`golish`) injects the concrete factory.
    pub pentest_tool_factory: Arc<dyn PentestToolFactory>,
}

#[cfg(test)]
mod session_slot_tests {
    use std::any::Any;
    use std::sync::Arc;

    use async_trait::async_trait;
    use golish_core::runtime::{ApprovalResult, GolishRuntime, RuntimeError, RuntimeEvent};

    use super::AiState;

    #[derive(Debug)]
    struct MockRuntime;

    #[async_trait]
    impl GolishRuntime for MockRuntime {
        fn emit(&self, _event: RuntimeEvent) -> Result<(), RuntimeError> {
            Ok(())
        }

        async fn request_approval(
            &self,
            _request_id: String,
            _tool_name: String,
            _args: serde_json::Value,
            _risk_level: String,
        ) -> Result<ApprovalResult, RuntimeError> {
            Err(RuntimeError::ApprovalTimeout(0))
        }

        fn is_interactive(&self) -> bool {
            false
        }

        fn auto_approve(&self) -> bool {
            false
        }

        async fn shutdown(&self) -> Result<(), RuntimeError> {
            Ok(())
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    async fn bridge(label: &str) -> (tempfile::TempDir, golish_agent_bridge::AgentBridge) {
        let workspace = tempfile::tempdir().expect("temp workspace");
        let model_name = format!("test-{label}");
        let bridge = golish_agent_bridge::AgentBridge::new_openrouter_with_runtime(
            workspace.path().to_path_buf(),
            &model_name,
            "test-key",
            None,
            Arc::new(MockRuntime),
        )
        .await
        .expect("test bridge");
        (workspace, bridge)
    }

    #[tokio::test]
    async fn busy_generation_rejects_replacement_and_preserves_current_bridge() {
        let state = AiState::new();
        let (_workspace_a, bridge_a) = bridge("a").await;
        state
            .install_session_bridge("session".to_string(), bridge_a)
            .await
            .unwrap();
        let current = state.get_session_bridge("session").await.unwrap();
        let old_notes = current.background_notes_handle();
        old_notes.lock().unwrap().push("before handoff".to_string());
        let old_listener = current
            .claim_background_listener_lifecycle()
            .expect("published old bridge claims listeners");
        let owner = current.begin_top_level_request().await.unwrap();

        let (_workspace_b, bridge_b) = bridge("b").await;
        assert!(state
            .install_session_bridge("session".to_string(), bridge_b)
            .await
            .is_err());
        assert!(Arc::ptr_eq(
            &current,
            &state.get_session_bridge("session").await.unwrap()
        ));

        drop(owner);
        let (_workspace_c, bridge_c) = bridge("c").await;
        state
            .install_session_bridge("session".to_string(), bridge_c)
            .await
            .unwrap();
        let replacement = state.get_session_bridge("session").await.unwrap();
        assert!(!Arc::ptr_eq(&current, &replacement));
        let replacement_notes = replacement.background_notes_handle();
        assert!(Arc::ptr_eq(&old_notes, &replacement_notes));
        old_notes
            .lock()
            .unwrap()
            .push("old retirement drain".to_string());
        assert_eq!(
            replacement_notes.lock().unwrap().as_slice(),
            ["before handoff", "old retirement drain"]
        );
        assert!(
            *old_listener.borrow(),
            "replacement retires the old generation listener owner"
        );
        assert!(replacement.claim_background_listener_lifecycle().is_some());
    }

    #[tokio::test]
    async fn shutdown_invalidates_late_clone_and_old_generation_forever() {
        let state = AiState::new();
        let (_workspace_a, bridge_a) = bridge("a").await;
        state
            .install_session_bridge("session".to_string(), bridge_a)
            .await
            .unwrap();
        let late_old = state.get_session_bridge("session").await.unwrap();
        let old_listener = late_old
            .claim_background_listener_lifecycle()
            .expect("published bridge claims listeners");

        let removed = state.remove_session_bridge("session").await.unwrap();
        removed.cancel();
        assert!(late_old.begin_top_level_request().await.is_err());
        assert!(*old_listener.borrow());

        let (_workspace_b, bridge_b) = bridge("b").await;
        state
            .install_session_bridge("session".to_string(), bridge_b)
            .await
            .unwrap();
        let current = state.get_session_bridge("session").await.unwrap();
        let owner = current.begin_top_level_request().await.unwrap();
        assert!(late_old.begin_top_level_request().await.is_err());
        drop(owner);
    }

    #[tokio::test]
    async fn inactive_slot_prunes_without_reviving_late_old_generation() {
        let state = AiState::new();
        let (_workspace_a, bridge_a) = bridge("a").await;
        state
            .install_session_bridge("session".to_string(), bridge_a)
            .await
            .unwrap();
        let late_old = state.get_session_bridge("session").await.unwrap();

        state.remove_session_bridge("session").await.unwrap();
        assert!(!state.prune_inactive_session_slot("session").await);
        drop(late_old);
        assert!(state.prune_inactive_session_slot("session").await);
        assert!(state.session_slots.read().await.is_empty());

        let (_workspace_b, bridge_b) = bridge("b").await;
        state
            .install_session_bridge("session".to_string(), bridge_b)
            .await
            .unwrap();
        assert!(state
            .get_session_bridge("session")
            .await
            .unwrap()
            .begin_top_level_request()
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn active_old_owner_blocks_prune_and_same_id_reinit_until_unwound() {
        let state = AiState::new();
        let (_workspace_a, bridge_a) = bridge("a").await;
        state
            .install_session_bridge("session".to_string(), bridge_a)
            .await
            .unwrap();
        let old = state.get_session_bridge("session").await.unwrap();
        let owner = old.begin_top_level_request().await.unwrap();
        let removed = state.remove_session_bridge("session").await.unwrap();

        assert!(!state.prune_inactive_session_slot("session").await);
        assert!(state.begin_session_bridge_install("session").await.is_err());

        drop(owner);
        drop(old);
        drop(removed);
        assert!(state.prune_inactive_session_slot("session").await);

        let (_workspace_b, bridge_b) = bridge("b").await;
        state
            .install_session_bridge("session".to_string(), bridge_b)
            .await
            .unwrap();
        assert!(state
            .get_session_bridge("session")
            .await
            .unwrap()
            .begin_top_level_request()
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn later_session_init_sweeps_tombstone_after_busy_owner_finishes() {
        let state = AiState::new();
        let (_workspace_a, bridge_a) = bridge("a").await;
        state
            .install_session_bridge("session-a".to_string(), bridge_a)
            .await
            .unwrap();
        let old = state.get_session_bridge("session-a").await.unwrap();
        let owner = old.begin_top_level_request().await.unwrap();
        let removed = state.remove_session_bridge("session-a").await.unwrap();
        assert!(!state.prune_inactive_session_slot("session-a").await);

        drop(owner);
        drop(old);
        drop(removed);
        assert!(state.session_slots.read().await.contains_key("session-a"));

        let install_b = state
            .begin_session_bridge_install("session-b")
            .await
            .unwrap();
        assert!(!state.session_slots.read().await.contains_key("session-a"));
        drop(install_b);
    }

    #[tokio::test]
    async fn failed_initial_install_reservation_does_not_leave_tombstone() {
        let state = AiState::new();
        let install = state
            .begin_session_bridge_install("failed-first-init")
            .await
            .unwrap();
        drop(install);

        assert!(state.prune_inactive_session_slot("failed-first-init").await);
        assert!(state.session_slots.read().await.is_empty());
    }

    #[tokio::test]
    async fn concurrent_init_is_fail_fast_but_different_sessions_remain_parallel() {
        let state = AiState::new();
        let install_a = state.begin_session_bridge_install("a").await.unwrap();
        let second_a = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            state.begin_session_bridge_install("a"),
        )
        .await
        .expect("second init must not wait behind provider construction");
        assert!(second_a.is_err());

        let install_b = state.begin_session_bridge_install("b").await.unwrap();
        let (_workspace_a, bridge_a) = bridge("a").await;
        let (_workspace_b, bridge_b) = bridge("b").await;
        state
            .finish_session_bridge_install(install_a, bridge_a, |_| {})
            .await
            .unwrap();
        state
            .finish_session_bridge_install(install_b, bridge_b, |_| {})
            .await
            .unwrap();

        let owner_a = state
            .get_session_bridge("a")
            .await
            .unwrap()
            .begin_top_level_request()
            .await
            .unwrap();
        let owner_b = state
            .get_session_bridge("b")
            .await
            .unwrap()
            .begin_top_level_request()
            .await
            .unwrap();
        drop((owner_a, owner_b));
    }

    #[tokio::test]
    async fn shutdown_arriving_during_init_wins_after_candidate_publish() {
        let state = AiState::new();
        let (_workspace_old, old_bridge) = bridge("old").await;
        state
            .install_session_bridge("session".to_string(), old_bridge)
            .await
            .unwrap();
        let late_old = state.get_session_bridge("session").await.unwrap();

        let install = state.begin_session_bridge_install("session").await.unwrap();
        let shutdown_state = state.clone();
        let shutdown =
            tokio::spawn(async move { shutdown_state.remove_session_bridge("session").await });
        tokio::task::yield_now().await;
        assert!(!shutdown.is_finished());

        let (_workspace_new, new_bridge) = bridge("new").await;
        state
            .finish_session_bridge_install(install, new_bridge, |_| {})
            .await
            .unwrap();
        let removed_new = shutdown.await.unwrap().unwrap();

        assert_eq!(removed_new.model_name(), "test-new");
        assert!(state.get_session_bridge("session").await.is_none());
        assert!(late_old.begin_top_level_request().await.is_err());
        assert!(removed_new.begin_top_level_request().await.is_err());
    }
}
