//! Tauri builder configuration (plugins, managed state, lifecycle hooks).
//!
//! The giant `tauri::generate_handler![...]` invocation lives separately in
//! `crate::commands_registry::install_handlers`. Everything else that used
//! to live inline in `tauri::Builder::default()...` now lives here.

use std::sync::Arc;

use tauri::Manager;
use tokio::sync::RwLock;

use crate::commands::FileWatcherState;
use crate::history::HistoryManager;
use crate::state::AppState;
use crate::tools;
use golish_recon_app::integrations::capture::CaptureEngine;
use golish_recon_app::integrations::IntegrationsState;

/// Apply plugins, managed state and lifecycle hooks to the given Tauri
/// builder. The caller is responsible for chaining `invoke_handler`,
/// `build`, and `run` afterwards.
pub(crate) fn configure_builder(
    builder: tauri::Builder<tauri::Wry>,
    app_state: AppState,
    history_manager: Arc<RwLock<Option<HistoryManager>>>,
) -> tauri::Builder<tauri::Wry> {
    let db_state = app_state.extract_db_state();
    let agent_state = app_state.extract_agent_state();
    let telemetry_state = app_state.extract_telemetry_state();
    let mcp_managed = app_state.extract_mcp_managed();
    let pty_state = app_state.extract_pty_state();
    let sidecar_managed = app_state.extract_sidecar_managed();
    let settings_mgr = app_state.settings_manager.clone();
    let pentest_cfg = app_state.pentest_config_manager.clone();
    // Integrations: schema resolver + tester + bundled in-code schemas
    // (intel providers + `resources/integrations/core.json`).
    //
    // Snapshot tools_dir + toolsconfig_dir once at startup so the
    // `{{exec}}` resolver embedded in the tester can run sync. The
    // resulting `ExecResolver` closure captures the snapshot; new
    // tools installed at runtime require a Golish restart to surface
    // in the Test Connection button (acceptable trade-off).
    let (pentest_tools_dir, pentest_toolsconfig_dir) = tauri::async_runtime::block_on(async {
        (
            pentest_cfg.tools_dir().await,
            pentest_cfg.toolsconfig_dir().await,
        )
    });
    let integrations_state = IntegrationsState::build_default(
        settings_mgr.clone(),
        pentest_tools_dir,
        pentest_toolsconfig_dir,
    );
    // Credential Capture Engine. Tauri-managed as `Arc<...>` so the
    // setup hook can clone an owning handle for the TTL watcher
    // background task without lifetime headaches.
    let capture_engine: Arc<CaptureEngine> = Arc::new(CaptureEngine::new());

    // Recon-app asset-intel (crate-per-service M2b) resolves `toolsconfig_dir`
    // via the pentest config manager. Share the same `Arc<ConfigManager>` the
    // PentestState uses so behaviour is identical to the pre-extraction code.
    let pentest_state = tools::pentest::PentestState::new();
    let asset_intel_tools_config =
        golish_recon_app::asset_intel::ToolsConfigState(pentest_state.config_manager.clone());

    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .manage(app_state)
        .manage(db_state)
        .manage(agent_state)
        .manage(telemetry_state)
        .manage(mcp_managed)
        .manage(pty_state)
        .manage(sidecar_managed)
        .manage(settings_mgr)
        .manage(pentest_cfg)
        .manage(history_manager)
        .manage(Arc::new(FileWatcherState::new()))
        .manage(pentest_state)
        .manage(asset_intel_tools_config)
        .manage(integrations_state)
        .manage(capture_engine)
        .on_window_event(|window, event| {
            crate::app::window_lifecycle::handle_window_event(window, event);
        })
        .setup(|app| {
            crate::app::bootstrap::setup_subsystems(app)?;
            // Kick off the capture-session TTL watcher. The watcher
            // ticks every 10s, transitions any sessions past their
            // recipe-declared TTL to `Timeout`, then GCs terminal
            // sessions older than 1h.
            let engine: tauri::State<Arc<CaptureEngine>> = app.state();
            let engine = engine.inner().clone();
            engine.spawn_ttl_watcher(app.handle().clone());
            Ok(())
        })
}
