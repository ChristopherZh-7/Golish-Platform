//! Per-domain managed state for Tauri commands.
//!
//! New commands should take narrow sub-states (e.g. `DbState`) instead of
//! the monolithic `AppState`. During the transition both are managed.

pub mod db;
pub mod mcp;
pub mod pty;
pub mod sidecar;
pub mod telemetry;

pub use db::DbState;
// Narrow `AgentState` now lives in golish-agent-app together with the agent
// command surface (M4-proper); golish constructs it via
// `AppState::extract_agent_state()` and no longer re-exports the type.
pub use mcp::McpManaged;
pub use pty::PtyState;
pub use sidecar::SidecarManaged;
pub use telemetry::TelemetryState;

use crate::error::GolishError;
use std::collections::HashSet;
use std::sync::Arc;

use parking_lot::Mutex;
use sqlx::PgPool;
use tokio::sync::RwLock;

use crate::ai::AiState;
use crate::commands::CommandIndex;
use crate::indexer::IndexerState;
use crate::settings::SettingsManager;
use crate::sidecar::{SidecarConfig, SidecarState};
use crate::telemetry::TelemetryStats;
use crate::tools::pty_interactive::PtyOutputTap;
use golish_db::DbReadyGate;

/// Aggregate application state.
///
/// New commands should prefer the narrower sub-state types
/// (`DbState`, `PtyState`, etc.) over this monolith.
pub struct AppState {
    pub pty_manager: Arc<crate::pty::PtyManager>,
    pub ai_state: AiState,
    pub indexer_state: Arc<IndexerState>,
    pub settings_manager: Arc<SettingsManager>,
    pub sidecar_config: SidecarConfig,
    pub sidecar_state: Arc<SidecarState>,
    pub langfuse_active: bool,
    pub telemetry_stats: Option<Arc<TelemetryStats>>,
    pub mcp_manager: Arc<RwLock<Option<Arc<golish_mcp::McpManager>>>>,
    pub command_index: Arc<CommandIndex>,
    pub pentest_config_manager: Arc<golish_pentest::ConfigManager>,
    pub pty_output_tap: Arc<PtyOutputTap>,
    pub active_terminal_session: Arc<Mutex<Option<String>>>,
    pub pentest_busy_sessions: Arc<Mutex<HashSet<String>>>,
    pub db_pool: Arc<PgPool>,
    pub db_ready: DbReadyGate,
    /// Process-global Memory Fabric owner. Session bridges only receive its
    /// shared UoW handle; start/cancel/join stay in this AppState lifecycle.
    pub memory_supervisor:
        golish_agent_app::ai::db_bridge::knowledge_memory::KnowledgeMemoryRuntime,
    /// DB-global Cleanup P7b worker. The lease in Postgres is authoritative,
    /// so a concurrently running CLI/desktop process cannot double-clean.
    pub cleanup_closeout: golish_cleanup_app::CleanupCloseoutRuntime,
    /// GUI-owned orphan blob GC lifecycle. It starts only after DB readiness
    /// and is joined before the process tears down the DB.
    pub reporting_artifact_gc: crate::reporting_artifact_store::ReportArtifactGcRuntime,
}

impl AppState {
    pub async fn new(
        settings_manager: Arc<SettingsManager>,
        langfuse_active: bool,
        telemetry_stats: Option<Arc<TelemetryStats>>,
        db_pool: Arc<PgPool>,
        db_ready: DbReadyGate,
    ) -> Self {
        let settings = settings_manager.get().await;
        let sidecar_config = SidecarConfig::from_golish_settings(&settings.sidecar);
        let sidecar_state = Arc::new(SidecarState::with_config(sidecar_config.clone()));
        let memory_supervisor =
            golish_agent_app::ai::db_bridge::knowledge_memory::KnowledgeMemoryRuntime::from_settings(
                db_pool.clone(),
                &settings,
            );
        let cleanup_closeout = golish_cleanup_app::CleanupCloseoutRuntime::new(
            db_pool.clone(),
            Arc::new(
                golish_recon_app::organizations::DbBackedOrganizationArtifactCleaner::new(
                    db_pool.clone(),
                ),
            ),
            format!(
                "desktop-cleanup-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ),
        )
        .expect("cleanup closeout worker identity is valid");
        let reporting_artifact_store_factory: std::sync::Arc<
            dyn golish_reporting_app::ReportArtifactStoreFactory,
        > = std::sync::Arc::new(crate::reporting_artifact_store::ProjectReportArtifactStoreFactory);
        let reporting_artifact_gc = crate::reporting_artifact_store::ReportArtifactGcRuntime::new(
            db_pool.clone(),
            reporting_artifact_store_factory,
        );

        Self {
            pty_manager: Arc::new(crate::pty::PtyManager::new()),
            ai_state: AiState::new(),
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
            db_ready,
            memory_supervisor,
            cleanup_closeout,
            reporting_artifact_gc,
        }
    }

    // Legacy DB-readiness wait; commands now go through `extract_db_state()` →
    // `DbState`, so nothing reads this on AppState anymore. Kept for parity /
    // future use (pre-existing; surfaced when M4-A recompiled this module).
    #[allow(dead_code)]
    pub async fn db_pool_ready(&self) -> Result<&PgPool, GolishError> {
        if self.db_ready.is_ready() {
            return Ok(&self.db_pool);
        }
        if self.db_ready.is_failed() {
            return Err(GolishError::Internal("Database failed to start".into()));
        }
        if !self
            .db_ready
            .wait_timeout(std::time::Duration::from_secs(15))
            .await
        {
            return Err(GolishError::Internal(
                "Database is still starting up, please retry".into(),
            ));
        }
        Ok(&self.db_pool)
    }

    /// Extract a `DbState` that shares the same pool + gate.
    pub fn extract_db_state(&self) -> DbState {
        DbState::new(self.db_pool.clone(), self.db_ready.clone())
    }

    /// Extract an [`AgentState`](golish_agent_app::AgentState) sharing the same
    /// handles the agent commands need (everything except the platform-only
    /// `command_index` / `telemetry_stats` / `langfuse_active`). All fields are
    /// `Arc`/`Clone` shares, so the agent commands see the same runtime state as
    /// the rest of the app. This is what lets `ai/commands/*` take the narrow
    /// `AgentState` instead of this monolith (crate-per-service M4-A).
    pub fn extract_agent_state(&self) -> golish_agent_app::AgentState {
        golish_agent_app::AgentState {
            ai_state: self.ai_state.clone(),
            pty_manager: self.pty_manager.clone(),
            indexer_state: self.indexer_state.clone(),
            settings_manager: self.settings_manager.clone(),
            sidecar_config: self.sidecar_config.clone(),
            sidecar_state: self.sidecar_state.clone(),
            mcp_manager: self.mcp_manager.clone(),
            pentest_config_manager: self.pentest_config_manager.clone(),
            pty_output_tap: self.pty_output_tap.clone(),
            active_terminal_session: self.active_terminal_session.clone(),
            pentest_busy_sessions: self.pentest_busy_sessions.clone(),
            db_pool: self.db_pool.clone(),
            db_ready: self.db_ready.clone(),
            knowledge_memory: self.memory_supervisor.unit_of_work(),
            knowledge_query_embedding: self.memory_supervisor.query_embedding_provider(),
            operator_principal_provider: std::sync::Arc::new(
                golish_agent_app::DbTrustedOperatorPrincipalProvider::new(
                    self.db_pool.clone(),
                    self.db_ready.clone(),
                ),
            ),
            reporting_artifact_store_factory: std::sync::Arc::new(
                crate::reporting_artifact_store::ProjectReportArtifactStoreFactory,
            ),
            pentest_tool_factory: std::sync::Arc::new(
                crate::pentest_tool_factory::GolishPentestToolFactory,
            ),
        }
    }

    /// Extract a `TelemetryState` that shares the same stats + langfuse flag.
    pub fn extract_telemetry_state(&self) -> TelemetryState {
        TelemetryState::new(self.langfuse_active, self.telemetry_stats.clone())
    }

    /// Extract an `McpManaged` that shares the same MCP manager slot.
    pub fn extract_mcp_managed(&self) -> McpManaged {
        McpManaged::from_shared(self.mcp_manager.clone())
    }

    /// Extract a `PtyState` that shares the same PTY manager + session data.
    pub fn extract_pty_state(&self) -> PtyState {
        PtyState::from_shared(
            self.pty_manager.clone(),
            self.pty_output_tap.clone(),
            self.active_terminal_session.clone(),
            self.pentest_busy_sessions.clone(),
        )
    }

    /// Extract a `SidecarManaged` that shares the same config + runtime state.
    pub fn extract_sidecar_managed(&self) -> SidecarManaged {
        SidecarManaged::from_shared(self.sidecar_config.clone(), self.sidecar_state.clone())
    }
}
