//! Agent service state.
//!
//! - [`AiState`] — per-session agent runtime state (bridges / legacy bridge /
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

use golish_agent_bridge::AgentBridge;
use golish_app_core::pty_interactive::PtyOutputTap;
use golish_app_core::GolishError;
use golish_core::runtime::GolishRuntime;
use golish_db::DbReadyGate;

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
    /// Legacy single bridge for backwards compatibility during migration.
    /// TODO: Remove once all commands use session-specific bridges.
    pub bridge: Arc<RwLock<Option<AgentBridge>>>,
    /// Runtime abstraction for event emission and approval handling.
    /// Stored here for later phases when AgentBridge will use it directly.
    /// Currently created during init but the existing event_tx path is used.
    pub runtime: Arc<RwLock<Option<Arc<dyn GolishRuntime>>>>,
}

impl Default for AiState {
    fn default() -> Self {
        Self {
            bridges: Arc::new(RwLock::new(HashMap::new())),
            bridge: Arc::new(RwLock::new(None)),
            runtime: Arc::new(RwLock::new(None)),
        }
    }
}

/// Error message for uninitialized AI agent.
pub const AI_NOT_INITIALIZED_ERROR: &str = "AI agent not initialized. Call init_ai_agent first.";

/// Build a `GolishError` for an uninitialized session.
pub fn ai_session_not_initialized_error(session_id: &str) -> GolishError {
    GolishError::Internal(format!(
        "AI agent not initialized for session '{}'. Call init_ai_session first.",
        session_id
    ))
}

/// Build a `GolishError` for the legacy uninitialized agent.
pub fn ai_not_initialized_error() -> GolishError {
    GolishError::Internal(AI_NOT_INITIALIZED_ERROR.to_string())
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

    /// Insert a bridge for a session.
    ///
    /// The bridge is wrapped in Arc for concurrent access.
    pub async fn insert_session_bridge(&self, session_id: String, bridge: AgentBridge) {
        self.bridges
            .write()
            .await
            .insert(session_id, Arc::new(bridge));
    }

    /// Remove and return the bridge for a session.
    ///
    /// Returns the Arc-wrapped bridge if it existed.
    pub async fn remove_session_bridge(&self, session_id: &str) -> Option<Arc<AgentBridge>> {
        self.bridges.write().await.remove(session_id)
    }

    // ========== Legacy single-bridge methods ==========
    //
    // These access `self.bridge` (the legacy non-keyed bridge stored on
    // `init_ai_agent`). Every command that still calls them is on the
    // migration shortlist documented at the top of this module — switch
    // to `get_session_bridge(session_id)` / `has_session_bridge` once the
    // command's IPC signature carries a `session_id`.

    /// Returns a read guard on the legacy single bridge, erroring if
    /// `init_ai_agent` has not been called.
    ///
    /// **Legacy** — prefer `get_session_bridge(session_id)`. Renamed
    /// from `get_bridge` in QW1 (2026-05) so call sites are obviously
    /// "legacy path" at a glance.
    pub async fn get_legacy_bridge(
        &self,
    ) -> Result<tokio::sync::RwLockReadGuard<'_, Option<AgentBridge>>, GolishError> {
        let guard = self.bridge.read().await;
        if guard.is_none() {
            return Err(ai_not_initialized_error());
        }
        Ok(guard)
    }

    /// Convenience wrapper that maps an `FnOnce` over the legacy
    /// single bridge without the caller needing to deal with the read
    /// guard or `Option`.
    ///
    /// **Legacy** — prefer `get_session_bridge(session_id)` and call
    /// the closure manually after pattern-matching. Renamed from
    /// `with_bridge` in QW1 (2026-05).
    pub async fn with_legacy_bridge<F, T>(&self, f: F) -> Result<T, GolishError>
    where
        F: FnOnce(&AgentBridge) -> T,
    {
        let guard = self.bridge.read().await;
        let bridge = guard.as_ref().ok_or_else(ai_not_initialized_error)?;
        Ok(f(bridge))
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
}
