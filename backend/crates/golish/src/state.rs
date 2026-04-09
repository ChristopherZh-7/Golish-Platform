//! Application state for Tauri commands.
//!
//! This module is only compiled when the `tauri` feature is enabled.

use std::collections::HashSet;
use std::sync::Arc;

use crate::ai::commands::WorkflowState;
use crate::ai::AiState;
use crate::commands::CommandIndex;
use crate::indexer::IndexerState;
use crate::pty::PtyManager;
use crate::settings::SettingsManager;
use crate::sidecar::{SidecarConfig, SidecarState};
use crate::telemetry::TelemetryStats;
use crate::tools::pty_interactive::PtyOutputTap;
use parking_lot::Mutex;
use sqlx::PgPool;
use tokio::sync::RwLock;

pub struct AppState {
    pub pty_manager: Arc<PtyManager>,
    pub ai_state: AiState,
    pub workflow_state: Arc<WorkflowState>,
    pub indexer_state: Arc<IndexerState>,
    pub settings_manager: Arc<SettingsManager>,
    /// Sidecar configuration - used to create per-session SidecarState instances.
    pub sidecar_config: SidecarConfig,
    /// Global sidecar state for UI commands (status, session listing, etc.).
    /// NOTE: Agent bridges have their OWN SidecarState instances (created in configure_bridge)
    /// to enable per-session isolation and avoid blocking between tabs.
    pub sidecar_state: Arc<SidecarState>,
    /// Whether Langfuse tracing is active (enabled and properly configured).
    pub langfuse_active: bool,
    /// Telemetry statistics (only populated when Langfuse is active).
    pub telemetry_stats: Option<Arc<TelemetryStats>>,
    /// Global MCP manager shared across all agent sessions.
    /// Initialized in the background during app startup. None until initialization completes.
    pub mcp_manager: Arc<RwLock<Option<Arc<golish_mcp::McpManager>>>>,
    /// Command index for auto input mode classification.
    pub command_index: Arc<CommandIndex>,
    /// Pentest config manager for AI tool integration.
    pub pentest_config_manager: Arc<golish_pentest::ConfigManager>,
    /// Shared broadcast tap for PTY output events (used by visible run_pty_cmd).
    pub pty_output_tap: Arc<PtyOutputTap>,
    /// Currently active (visible) terminal session ID, set by the frontend.
    pub active_terminal_session: Arc<Mutex<Option<String>>>,
    /// Terminal sessions currently in use by pentest tool executions.
    /// Shared across all AI tabs to prevent concurrent commands on the same terminal.
    pub pentest_busy_sessions: Arc<Mutex<HashSet<String>>>,
    /// PostgreSQL connection pool (from golish-db).
    /// Created lazily via `connect_lazy` — connections are established
    /// on first query after the background PG server finishes starting.
    pub db_pool: Arc<PgPool>,
}

impl AppState {
    /// Create a new AppState with all subsystems initialized.
    ///
    /// This is async because SettingsManager needs to load from disk.
    ///
    /// # Arguments
    /// * `langfuse_active` - Whether Langfuse tracing is enabled and properly configured.
    /// * `telemetry_stats` - Optional telemetry stats for monitoring (only when Langfuse is active).
    pub async fn new(
        langfuse_active: bool,
        telemetry_stats: Option<Arc<TelemetryStats>>,
        db_pool: Arc<PgPool>,
    ) -> Self {
        let settings_manager = Arc::new(
            SettingsManager::new()
                .await
                .expect("Failed to initialize settings manager"),
        );

        let settings = settings_manager.get().await;
        let sidecar_config = SidecarConfig::from_golish_settings(&settings.sidecar);
        tracing::debug!(
            "[app-state] Created sidecar config: enabled={}",
            sidecar_config.enabled
        );

        let sidecar_state = Arc::new(SidecarState::with_config(sidecar_config.clone()));

        Self {
            pty_manager: Arc::new(PtyManager::new()),
            ai_state: AiState::new(),
            workflow_state: Arc::new(WorkflowState::new()),
            indexer_state: Arc::new(IndexerState::new()),
            settings_manager,
            sidecar_config,
            sidecar_state,
            langfuse_active,
            telemetry_stats,
            mcp_manager: Arc::new(RwLock::new(None)),
            command_index: Arc::new(CommandIndex::new()),
            pentest_config_manager: Arc::new(golish_pentest::ConfigManager::with_defaults()),
            pty_output_tap: Arc::new(PtyOutputTap::new()),
            active_terminal_session: Arc::new(Mutex::new(None)),
            pentest_busy_sessions: Arc::new(Mutex::new(HashSet::new())),
            db_pool,
        }
    }

    /// Create a new AppState with a pre-initialized SettingsManager.
    ///
    /// This avoids redundant disk reads when the SettingsManager has already been created.
    ///
    /// # Arguments
    /// * `settings_manager` - Already-initialized settings manager to use.
    /// * `langfuse_active` - Whether Langfuse tracing is enabled and properly configured.
    /// * `telemetry_stats` - Optional telemetry stats for monitoring (only when Langfuse is active).
    pub async fn with_settings_manager(
        settings_manager: Arc<SettingsManager>,
        langfuse_active: bool,
        telemetry_stats: Option<Arc<TelemetryStats>>,
        db_pool: Arc<PgPool>,
    ) -> Self {
        let settings = settings_manager.get().await;
        let sidecar_config = SidecarConfig::from_golish_settings(&settings.sidecar);
        tracing::debug!(
            "[app-state] Created sidecar config: enabled={}",
            sidecar_config.enabled
        );

        let sidecar_state = Arc::new(SidecarState::with_config(sidecar_config.clone()));

        Self {
            pty_manager: Arc::new(PtyManager::new()),
            ai_state: AiState::new(),
            workflow_state: Arc::new(WorkflowState::new()),
            indexer_state: Arc::new(IndexerState::new()),
            settings_manager,
            sidecar_config,
            sidecar_state,
            langfuse_active,
            telemetry_stats,
            mcp_manager: Arc::new(RwLock::new(None)),
            command_index: Arc::new(CommandIndex::new()),
            pentest_config_manager: Arc::new(golish_pentest::ConfigManager::with_defaults()),
            pty_output_tap: Arc::new(PtyOutputTap::new()),
            active_terminal_session: Arc::new(Mutex::new(None)),
            pentest_busy_sessions: Arc::new(Mutex::new(HashSet::new())),
            db_pool,
        }
    }
}
