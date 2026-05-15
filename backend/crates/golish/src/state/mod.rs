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
pub use mcp::McpManaged;
pub use pty::PtyState;
pub use sidecar::SidecarManaged;
pub use telemetry::TelemetryState;

use crate::error::GolishError;
use std::collections::HashSet;
use std::sync::Arc;

use parking_lot::Mutex;
use sqlx::PgPool;
use tokio::sync::{Mutex as AsyncMutex, RwLock};

use crate::ai::AiState;
use crate::commands::CommandIndex;
use crate::indexer::IndexerState;
use crate::settings::SettingsManager;
use crate::sidecar::{SidecarConfig, SidecarState};
use crate::telemetry::TelemetryStats;
use crate::tools::pty_interactive::PtyOutputTap;
use golish_db::{DbReadyGate, GolishDb};

/// Shared, mutually-exclusive handle to the embedded PostgreSQL server.
///
/// `Some(db)` while the background bootstrap has produced a live handle that
/// can be `.stop().await`'d during graceful exit; `None` either before the
/// bootstrap finishes or after exit has already drained it (so subsequent
/// exit hooks become no-ops). The async mutex keeps `.stop()` safe to call
/// across `.await` points from the Tauri shutdown handler.
pub type EmbeddedDbHandle = Arc<AsyncMutex<Option<GolishDb>>>;

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
    /// Owned handle to the embedded PostgreSQL server. Populated by the
    /// background `spawn_embedded_pg` task once startup succeeds. Taken
    /// out and `.stop()`'d by the Tauri exit hooks so PG gets a chance
    /// to flush WAL and remove its `postmaster.pid` before the process
    /// is force-killed by the OS — without this the next launch would
    /// see a stale PID file and (historically) trigger a destructive
    /// recovery path.
    pub embedded_db: EmbeddedDbHandle,
}

impl AppState {
    pub async fn new(
        settings_manager: Arc<SettingsManager>,
        langfuse_active: bool,
        telemetry_stats: Option<Arc<TelemetryStats>>,
        db_pool: Arc<PgPool>,
        db_ready: DbReadyGate,
        embedded_db: EmbeddedDbHandle,
    ) -> Self {
        let settings = settings_manager.get().await;
        let sidecar_config = SidecarConfig::from_golish_settings(&settings.sidecar);
        let sidecar_state = Arc::new(SidecarState::with_config(sidecar_config.clone()));

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
            embedded_db,
        }
    }

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
