//! CLI bootstrap - Initialize the full Golish stack for CLI usage.
//!
//! This module provides `CliContext` which initializes all the same services
//! as the Tauri GUI application, ensuring feature parity between CLI and GUI.

mod agent_init;
pub(crate) use agent_init::initialize_agent;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::{mpsc, RwLock};

use crate::ai::agent_bridge::AgentBridge;
use crate::history::{HistoryConfig, HistoryManager};
use crate::indexer::IndexerState;
use crate::pty::PtyManager;
use crate::runtime::CliRuntime;
use crate::settings::SettingsManager;
use crate::sidecar::SidecarState;
use golish_core::runtime::{GolishRuntime, RuntimeEvent};

use super::args::Args;

/// Context for CLI execution containing all initialized services.
///
/// This mirrors the Tauri `AppState` but is owned rather than managed by Tauri.
pub struct CliContext {
    /// Runtime abstraction for event emission
    pub runtime: Arc<dyn GolishRuntime>,

    /// Global history manager (best-effort)
    pub history: Option<crate::history::HistoryManager>,

    /// Resolved provider/model used by the CLI for this run (for history metadata)
    pub provider: String,
    pub model: String,

    /// Event receiver for output handling
    pub event_rx: mpsc::UnboundedReceiver<RuntimeEvent>,

    /// Agent bridge (initialized lazily via `ensure_agent`)
    bridge: Arc<RwLock<Option<AgentBridge>>>,

    /// Resolved workspace path
    pub workspace: PathBuf,

    /// Settings manager
    pub settings_manager: Arc<SettingsManager>,

    /// PTY manager for shell execution
    pub pty_manager: Arc<PtyManager>,

    /// Code indexer
    pub indexer_state: Arc<IndexerState>,

    /// Sidecar context capture
    pub sidecar_state: Arc<SidecarState>,

    /// MCP manager for external tool servers (optional)
    pub mcp_manager: Option<Arc<golish_mcp::McpManager>>,

    /// Command-line arguments
    pub args: Args,

    /// Same process-global Memory Fabric lifecycle used by desktop mode.
    memory_supervisor: golish_agent_app::ai::db_bridge::knowledge_memory::KnowledgeMemoryRuntime,

    /// Same DB-global Cleanup closeout worker used by desktop mode.
    cleanup_closeout: golish_cleanup_app::CleanupCloseoutRuntime,

    /// CLI-owned Reporting orphan blob GC lifecycle.
    reporting_artifact_gc: crate::reporting_artifact_store::ReportArtifactGcRuntime,

    /// CLI-owned asynchronous Hypothesis Registry read-model projector.
    investigation_projection_worker:
        golish_db::repo::investigation_projection::InvestigationProjectionWorker,

    /// Owned embedded database handle for ordinary headless CLI mode.
    embedded_db: Option<golish_db::GolishDb>,
    stop_embedded_db: bool,
}

impl CliContext {
    /// Get a reference to the agent bridge, if initialized.
    pub async fn bridge(&self) -> tokio::sync::RwLockReadGuard<'_, Option<AgentBridge>> {
        self.bridge.read().await
    }

    /// Get a mutable reference to the agent bridge.
    pub async fn bridge_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, Option<AgentBridge>> {
        self.bridge.write().await
    }

    /// Check if the agent is initialized.
    pub async fn is_agent_initialized(&self) -> bool {
        self.bridge.read().await.is_some()
    }

    /// Graceful shutdown - flush sidecar, end sessions, MCP servers, etc.
    pub async fn shutdown(self) -> Result<()> {
        // Finalize agent session if needed
        if let Some(ref bridge) = *self.bridge.read().await {
            bridge.finalize_session().await;
        }

        // Shutdown MCP servers (cancels child processes gracefully)
        if let Some(ref manager) = self.mcp_manager {
            manager.shutdown().await;
        }

        // Gracefully shutdown sidecar (waits for processor to flush pending events)
        self.sidecar_state.shutdown();

        // Stop the read-model projector before the pool/server can disappear.
        self.investigation_projection_worker.shutdown().await;

        // Stop cleanup claims before stopping the invalidation projectors they
        // depend on, then await the current projector batch.
        self.cleanup_closeout
            .shutdown()
            .await
            .map_err(|error| anyhow::anyhow!(error.code().to_string()))?;
        self.reporting_artifact_gc.shutdown().await;
        // before the DB pool/server can disappear underneath it.
        self.memory_supervisor
            .shutdown()
            .await
            .map_err(|error| anyhow::anyhow!(error.code().to_string()))?;

        // Shutdown the runtime
        if let Err(e) = self.runtime.shutdown().await {
            tracing::warn!("Runtime shutdown error: {}", e);
        }

        if let Some(mut db) = self.embedded_db {
            if self.stop_embedded_db {
                db.stop().await;
            } else {
                db.pool().close().await;
                std::mem::forget(db);
            }
        }

        Ok(())
    }
}

/// Initialize the CLI context with all services.
///
/// This is the main entry point for CLI initialization, mirroring
/// what happens in the Tauri app's `AppState::new()` and `init_ai_agent`.
pub async fn initialize(args: &Args) -> Result<CliContext> {
    // Install TLS provider (required for rustls 0.23+)
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Load .env file if present
    if let Err(e) = dotenvy::dotenv() {
        // Only warn on errors other than file not found
        if !matches!(e, dotenvy::Error::Io(_)) {
            tracing::warn!("Failed to load .env file: {}", e);
        }
    }

    // Set session directory to ~/.golish/sessions
    if std::env::var_os("VT_SESSION_DIR").is_none() {
        if let Some(home) = dirs::home_dir() {
            let golish_sessions = home.join(".golish").join("sessions");
            std::env::set_var("VT_SESSION_DIR", &golish_sessions);
        }
    }

    // Determine log level based on verbosity
    let log_level = if args.verbose { "debug" } else { "warn" };

    // Resolve workspace path
    let workspace = args.resolve_workspace()?;

    if args.verbose {
        eprintln!("[cli] Workspace: {}", workspace.display());
    }

    // Load settings
    let settings_manager = Arc::new(
        SettingsManager::new()
            .await
            .context("Failed to initialize settings manager")?,
    );

    // Ensure settings file exists (creates template on first run)
    if let Err(e) = settings_manager.ensure_settings_file().await {
        // Can't use tracing yet, use eprintln
        eprintln!("[cli] Warning: Failed to create settings template: {}", e);
    }

    let settings = settings_manager.get().await;

    // Apply proxy settings as environment variables
    golish_settings::apply_proxy_env(&settings);

    // Initialize tracing with optional Langfuse export
    let langfuse_config =
        crate::telemetry::LangfuseConfig::from_settings(&settings.telemetry.langfuse);

    // Build log directives based on mode.
    // A3: `golish_ai=` umbrella was removed; expand to the individual
    // implementation crates so log filtering keeps the same coverage.
    let directives: Vec<String> = vec![
        format!("golish={}", log_level),
        format!("golish_agent_kit={}", log_level),
        format!("golish_agent_runtime={}", log_level),
        format!("golish_agent_bridge={}", log_level),
        format!("golish_prompts={}", log_level),
        // Harness gate/transition/eval events use custom `harness::*` targets
        // (not a `golish_*` module path), so they need their own directive or
        // they get dropped by the `golish=*` filters above.
        format!("harness={}", log_level),
    ];

    let extra_directives: Vec<&str> = directives.iter().map(|s| s.as_str()).collect();

    // Initialize telemetry (this sets up the global subscriber)
    // We ignore the guard since CLI runs to completion
    if let Err(e) = crate::telemetry::init_tracing(langfuse_config, log_level, &extra_directives) {
        eprintln!("[cli] Warning: Failed to initialize tracing: {}", e);
        // Fall back to basic tracing
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::from_default_env()
                    .add_directive(format!("golish={}", log_level).parse().unwrap()),
            )
            .try_init();
    }

    if args.verbose {
        eprintln!(
            "[cli] Settings loaded from {}",
            settings_manager.path().display()
        );
        eprintln!("[cli] Default provider: {}", settings.ai.default_provider);
        eprintln!("[cli] Default model: {}", settings.ai.default_model);
        if settings.telemetry.langfuse.enabled {
            eprintln!("[cli] Langfuse tracing enabled");
        }
    }

    // Ordinary headless CLI now owns the same DB-ready Memory Supervisor as
    // desktop/stage-run. The constructor is side-effect free; workers start
    // only after the readiness gate confirms migrations are available.
    let db_config = golish_db::DbConfig::default();
    let preexisting_pg_on_port =
        std::net::TcpStream::connect(("127.0.0.1", db_config.port)).is_ok();
    let (db_pool, db_ready) = crate::app::bootstrap::create_lazy_db_pool_with_config(&db_config);
    let pg_handle_rx =
        crate::app::bootstrap::spawn_embedded_pg_owned_with_config(db_ready.clone(), db_config);
    let wait_gate = db_ready.clone();
    if !wait_gate
        .wait_timeout(std::time::Duration::from_secs(60))
        .await
        || !db_ready.is_ready()
    {
        anyhow::bail!("embedded PostgreSQL did not become ready for CLI memory services");
    }
    let mut embedded_db = pg_handle_rx
        .await
        .context("embedded PostgreSQL handle disappeared during CLI startup")?;
    let cleanup_pool = db_pool.clone();
    let investigation_projection_worker =
        golish_db::repo::investigation_projection::InvestigationProjectionWorker::new(
            cleanup_pool.clone(),
        );
    let memory_supervisor =
        golish_agent_app::ai::db_bridge::knowledge_memory::KnowledgeMemoryRuntime::from_settings(
            db_pool, &settings,
        );
    if let Err(error) = memory_supervisor.start().await {
        if !preexisting_pg_on_port {
            embedded_db.stop().await;
        } else {
            embedded_db.pool().close().await;
            std::mem::forget(embedded_db);
        }
        return Err(anyhow::anyhow!(error.code().to_string()));
    }
    let cleanup_closeout = golish_cleanup_app::CleanupCloseoutRuntime::new(
        cleanup_pool.clone(),
        Arc::new(
            golish_recon_app::organizations::DbBackedOrganizationArtifactCleaner::new(
                cleanup_pool.clone(),
            ),
        ),
        format!(
            "cli-cleanup-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ),
    )
    .map_err(|error| anyhow::anyhow!(error.code().to_string()))?;
    if let Err(error) = cleanup_closeout.start().await {
        let _ = memory_supervisor.shutdown().await;
        if !preexisting_pg_on_port {
            embedded_db.stop().await;
        } else {
            embedded_db.pool().close().await;
            std::mem::forget(embedded_db);
        }
        return Err(anyhow::anyhow!(error.code().to_string()));
    }
    let reporting_artifact_gc = crate::reporting_artifact_store::ReportArtifactGcRuntime::new(
        cleanup_pool.clone(),
        Arc::new(crate::reporting_artifact_store::ProjectReportArtifactStoreFactory),
    );
    reporting_artifact_gc.start();
    investigation_projection_worker.start().await;

    // Create event channel
    let (event_tx, event_rx) = mpsc::unbounded_channel::<RuntimeEvent>();

    // Create CLI runtime
    let runtime: Arc<dyn GolishRuntime> =
        Arc::new(CliRuntime::new(event_tx, args.auto_approve, args.json));

    // Initialize services
    let pty_manager = Arc::new(PtyManager::new());
    let indexer_state = Arc::new(IndexerState::new());
    let sidecar_state = Arc::new(SidecarState::new());
    let history = HistoryManager::new(HistoryConfig::default()).ok();

    // Initialize sidecar
    if settings.sidecar.enabled {
        if let Err(e) = sidecar_state.initialize(workspace.clone()).await {
            tracing::warn!("Failed to initialize sidecar: {}", e);
        } else if args.verbose {
            eprintln!("[cli] Sidecar initialized");
        }
    }

    // Resolve provider/model for this run (used for history metadata)
    let provider = args
        .provider
        .clone()
        .unwrap_or_else(|| settings.ai.default_provider.to_string());
    let model = args
        .model
        .clone()
        .unwrap_or_else(|| settings.ai.default_model.clone());

    // Initialize the agent bridge and MCP manager. The interactive CLI runs as
    // a single anonymous session, so "cli" is its event/evidence identity.
    let agent = initialize_agent(
        &workspace,
        &settings,
        args,
        runtime.clone(),
        indexer_state.clone(),
        sidecar_state.clone(),
        "cli",
        Some(memory_supervisor.unit_of_work()),
        Some(std::sync::Arc::new(
            golish_agent_app::ai::db_bridge::knowledge_context::PgKnowledgeContextAdapter::with_query_embedding(
                cleanup_pool.clone(),
                memory_supervisor.query_embedding_provider(),
            )
            .context("initialize exact-scope CLI ContextPack adapter")?,
        )),
    )
    .await;
    let (bridge, mcp_manager) = match agent {
        Ok(agent) => agent,
        Err(error) => {
            investigation_projection_worker.shutdown().await;
            reporting_artifact_gc.shutdown().await;
            let _ = cleanup_closeout.shutdown().await;
            let _ = memory_supervisor.shutdown().await;
            if !preexisting_pg_on_port {
                embedded_db.stop().await;
            } else {
                embedded_db.pool().close().await;
                std::mem::forget(embedded_db);
            }
            return Err(error);
        }
    };

    if args.verbose {
        eprintln!("[cli] Agent initialized successfully");
    }

    Ok(CliContext {
        runtime,
        history,
        provider,
        model,
        event_rx,
        bridge: Arc::new(RwLock::new(Some(bridge))),
        workspace,
        settings_manager,
        pty_manager,
        indexer_state,
        sidecar_state,
        mcp_manager,
        args: args.clone(),
        memory_supervisor,
        cleanup_closeout,
        reporting_artifact_gc,
        investigation_projection_worker,
        embedded_db: Some(embedded_db),
        stop_embedded_db: !preexisting_pg_on_port,
    })
}
