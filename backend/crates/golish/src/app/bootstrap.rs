//! Process-level bootstrap phases (CLI args, env, telemetry, DB pool,
//! embedded PostgreSQL, settings template, history manager).
//!
//! Lifted out of the old monolithic `run_gui` body and broken into
//! single-purpose helpers so individual phases (telemetry init, DB pool
//! creation, settings construction) can be reused or unit-tested without
//! pulling in the whole startup sequence.

use std::sync::Arc;

use sqlx::PgPool;
use tauri::{async_runtime, Emitter, Manager};
use tokio::sync::RwLock;

use crate::app::workspace::expand_tilde;
use crate::history::{HistoryConfig, HistoryManager};
use crate::settings::SettingsManager;
use crate::state::AppState;
use crate::telemetry::{self, TelemetryGuard, TelemetryStats};
use golish_db::DbReadyGate;

/// Parse the optional `[path]` positional CLI argument and, if provided,
/// store it as `QBIT_WORKSPACE` so the rest of the bootstrap picks it up
/// through the environment variable (matching legacy behaviour).
pub(crate) fn apply_cli_workspace_arg() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        let path_arg = &args[1];
        if !path_arg.starts_with('-') {
            let workspace = expand_tilde(path_arg);
            std::env::set_var("QBIT_WORKSPACE", &workspace);
        }
    }
}

/// Install the rustls `ring` crypto provider. Required by rustls 0.23+ before
/// any TLS operation (e.g. `reqwest`).
pub(crate) fn install_rustls_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Load a `.env` file from the project root (if present). Missing-file errors
/// are ignored; other errors are logged to stderr to match legacy behaviour.
pub(crate) fn load_dotenv() {
    if let Err(e) = dotenvy::dotenv() {
        if !matches!(e, dotenvy::Error::Io(_)) {
            eprintln!("Warning: Failed to load .env file: {}", e);
        }
    }
}

/// Ensure `VT_SESSION_DIR` points to `~/.golish/sessions` when unset. The
/// variable is consumed by `golish-core`'s session archive subsystem.
pub(crate) fn set_default_session_dir() {
    if std::env::var_os("VT_SESSION_DIR").is_none() {
        if let Some(home) = dirs::home_dir() {
            let golish_sessions = home.join(".golish").join("sessions");
            std::env::set_var("VT_SESSION_DIR", &golish_sessions);
        }
    }
}

/// Build and load the shared [`SettingsManager`]. Panics on any failure
/// because the application cannot run without settings.
pub(crate) async fn init_settings_manager() -> Arc<SettingsManager> {
    Arc::new(
        SettingsManager::new()
            .await
            .expect("Failed to initialize settings manager"),
    )
}

/// Initialise tracing/telemetry. Falls back to a plain `tracing_subscriber`
/// fmt layer when OpenTelemetry init fails so logs still reach stderr.
///
/// Returns the optional guard (kept alive for the lifetime of the process so
/// the OTel BatchSpanProcessor flushes on drop) plus whether Langfuse is
/// active and the shared stats handle.
pub(crate) fn init_telemetry(
    langfuse_config: Option<telemetry::LangfuseConfig>,
    log_level: &str,
) -> (Option<TelemetryGuard>, bool, Option<Arc<TelemetryStats>>) {
    match telemetry::init_tracing(langfuse_config, log_level, &[]) {
        Ok(guard) => {
            let active = guard.langfuse_active;
            let stats = guard.stats.clone();
            (Some(guard), active, stats)
        }
        Err(e) => {
            eprintln!("Warning: Failed to initialize OpenTelemetry: {}", e);
            let _ = tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::from_default_env()
                        .add_directive("golish=debug".parse().unwrap()),
                )
                .try_init();
            (None, false, None)
        }
    }
}

/// Create the **lazy** PostgreSQL connection pool and readiness gate.
///
/// Delegates the pool tuning (max/min connections, acquire timeout) to
/// [`golish_db::create_lazy_pool`] so the GUI and the eager `GolishDb::start`
/// path share one source of truth. No TCP connection is opened here — the
/// pool auto-connects on the first query after the background
/// `spawn_embedded_pg` task flips the gate.
pub(crate) fn create_lazy_db_pool() -> (Arc<PgPool>, DbReadyGate) {
    let db_config = golish_db::DbConfig::default();
    let pool = golish_db::create_lazy_pool(&db_config.connection_string())
        .expect("Failed to create lazy PG pool");
    (pool, DbReadyGate::new())
}

/// Compose the bootstrap phases that must run on the Tauri async runtime
/// (so background tasks like the OTel BatchSpanProcessor stay alive):
///
/// 1. build the shared [`SettingsManager`],
/// 2. initialise telemetry/tracing,
/// 3. create a lazy PG pool + ready-gate,
/// 4. build the [`AppState`] with the same settings manager,
/// 5. export proxy settings as environment variables.
pub(crate) fn init_telemetry_and_app_state() -> (Option<TelemetryGuard>, AppState) {
    async_runtime::block_on(async {
        let settings_manager = init_settings_manager().await;

        let (langfuse_config, log_level) = {
            let settings = settings_manager.get().await;
            let langfuse = telemetry::LangfuseConfig::from_settings(&settings.telemetry.langfuse);
            let level = settings.advanced.log_level.to_string();
            (langfuse, level)
        };

        let (telemetry_guard, langfuse_active, telemetry_stats) =
            init_telemetry(langfuse_config, &log_level);

        let (db_pool, db_ready) = create_lazy_db_pool();

        let app_state = AppState::new(
            settings_manager,
            langfuse_active,
            telemetry_stats,
            db_pool,
            db_ready,
        )
        .await;

        // Apply proxy settings as environment variables so all HTTP clients
        // (including rig-core's internal reqwest) automatically use them.
        {
            let settings = app_state.settings_manager.get().await;
            golish_settings::apply_proxy_env(&settings);
        }

        (telemetry_guard, app_state)
    })
}

/// Spawn the embedded PostgreSQL startup task. When the DB becomes ready, the
/// ready-gate is flipped and a dummy `GolishDb` handle is leaked to keep the
/// backing process alive for the lifetime of the app.
pub(crate) fn spawn_embedded_pg(db_ready: golish_db::DbReadyGate) {
    async_runtime::spawn(async move {
        tracing::info!("Starting embedded PostgreSQL database (background)...");
        match golish_db::GolishDb::start(golish_db::DbConfig::default()).await {
            Ok(_db) => {
                tracing::info!(
                    has_pgvector = _db.has_pgvector,
                    "Embedded PostgreSQL is fully ready"
                );
                db_ready.set_pgvector_available(_db.has_pgvector);
                db_ready.mark_ready();
                std::mem::forget(_db);
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to start embedded PostgreSQL");
                db_ready.mark_failed();
            }
        }
    });
}

/// Seed default agent `.md` files on first run.
pub(crate) fn seed_default_agent_files() {
    if let Err(e) = golish_sub_agents::discovery::seed_default_agent_files() {
        tracing::warn!(error = %e, "Failed to seed default agent files");
    }
}

/// Allocate an empty [`HistoryManager`] slot and initialise it asynchronously
/// in the background so application startup is not blocked on the filesystem.
pub(crate) fn init_history_manager_background() -> Arc<RwLock<Option<HistoryManager>>> {
    let history_manager: Arc<RwLock<Option<HistoryManager>>> = Arc::new(RwLock::new(None));
    {
        let history_manager = history_manager.clone();
        async_runtime::spawn(async move {
            match HistoryManager::new(HistoryConfig::default()) {
                Ok(manager) => {
                    *history_manager.write().await = Some(manager);
                    tracing::debug!("HistoryManager initialized in background");
                }
                Err(e) => {
                    tracing::warn!("Failed to initialize HistoryManager: {}", e);
                }
            }
        });
    }
    history_manager
}

/// Spawn a background task that ensures the settings file template exists on
/// disk (creates it on first run).
pub(crate) fn spawn_ensure_settings_file(settings_manager: Arc<SettingsManager>) {
    async_runtime::spawn(async move {
        if let Err(e) = settings_manager.ensure_settings_file().await {
            tracing::warn!("Failed to create settings template: {}", e);
        }
    });
}

/// Run all the per-app `setup` phase work that used to live inline in
/// `tauri::Builder::setup`. Installs the macOS menu, kicks off background
/// initialisations (sidecar, command index, MCP), wires up the `db-ready`
/// emitter, and triggers an early window-state restore.
///
/// The signature matches `tauri::Builder::setup`'s expected closure, which
/// boxes its error.
pub(crate) fn setup_subsystems(
    app: &mut tauri::App,
) -> Result<(), Box<dyn std::error::Error>> {
    crate::app::menu::install_app_menu(app)?;

    let state = app.state::<AppState>();
    let app_handle = app.handle().clone();

    crate::app::sidecar_bootstrap::spawn_sidecar_initialization(
        state.sidecar_state.clone(),
        state.settings_manager.clone(),
        app_handle.clone(),
    );

    {
        let command_index = state.command_index.clone();
        async_runtime::spawn_blocking(move || {
            command_index.build();
        });
    }

    crate::app::mcp_bootstrap::spawn_mcp_initialization(
        state.mcp_manager.clone(),
        app_handle.clone(),
    );

    {
        let mut db_gate = state.db_ready.clone();
        let db_handle = app_handle.clone();
        async_runtime::spawn(async move {
            let ready = db_gate.wait().await;
            let _ = db_handle.emit("db-ready", ready);
            if ready {
                tracing::debug!("Emitted db-ready event to frontend");
            } else {
                tracing::error!("Emitted db-ready(false) — database startup failed");
            }
        });
    }

    let restore_handle = app_handle.clone();
    async_runtime::spawn(async move {
        crate::app::window_lifecycle::restore_window_state_on_startup(&restore_handle).await;
    });

    Ok(())
}
