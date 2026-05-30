// Commands module for AI agent interaction.
//
// This module provides Tauri command handlers for the AI agent system,
// organized into logical submodules for maintainability.

use crate::error::GolishError;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::agent_bridge::AgentBridge;
use golish_core::runtime::GolishRuntime;

pub mod agents;
pub mod analytics;
pub mod config;
pub mod context;
pub mod core;
pub mod debug;
pub mod dispatch;
pub mod graph;
pub mod hitl;
pub mod loop_detection;
pub mod mode;
pub mod plan;
pub mod policy;
pub mod session;
pub mod summarizer;
pub mod workflow;

mod bridge_config;

// Re-export all commands for easier access
pub use agents::*;
pub use analytics::*;
pub use config::*;
pub use context::*;
pub use core::*;
pub use debug::*;
pub use dispatch::*;
pub use graph::*;
pub use hitl::*;
pub use loop_detection::*;
pub use mode::*;
pub use plan::*;
pub use policy::*;
pub use session::*;
pub use summarizer::*;
pub use workflow::*;

// Bridge wiring lives in `bridge_config`; re-export at the previous paths.
pub use bridge_config::configure_bridge;
pub(crate) use bridge_config::{setup_bridge_mcp_tools, McpManagerToolExecutor};

/// Shared AI state supporting multiple per-session agents.
/// Uses tokio RwLock for async compatibility with AgentBridge methods.
///
/// IMPORTANT: Bridges are wrapped in Arc to allow cloning references without
/// holding the map lock during long-running operations like execute().
/// This enables concurrent agent execution across multiple tabs.
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
