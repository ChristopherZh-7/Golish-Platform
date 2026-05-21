//! Tauri builder configuration (plugins, managed state, lifecycle hooks).
//!
//! The giant `tauri::generate_handler![...]` invocation lives separately in
//! `crate::commands_registry::install_handlers`. Everything else that used
//! to live inline in `tauri::Builder::default()...` now lives here.

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::commands::FileWatcherState;
use crate::history::HistoryManager;
use crate::state::AppState;
use crate::tools;
use crate::tools::integrations::IntegrationsState;

/// Apply plugins, managed state and lifecycle hooks to the given Tauri
/// builder. The caller is responsible for chaining `invoke_handler`,
/// `build`, and `run` afterwards.
pub(crate) fn configure_builder(
    builder: tauri::Builder<tauri::Wry>,
    app_state: AppState,
    history_manager: Arc<RwLock<Option<HistoryManager>>>,
) -> tauri::Builder<tauri::Wry> {
    let db_state = app_state.extract_db_state();
    let telemetry_state = app_state.extract_telemetry_state();
    let mcp_managed = app_state.extract_mcp_managed();
    let pty_state = app_state.extract_pty_state();
    let sidecar_managed = app_state.extract_sidecar_managed();
    let settings_mgr = app_state.settings_manager.clone();
    let pentest_cfg = app_state.pentest_config_manager.clone();
    // Integrations: schema resolver + tester + bundled in-code schemas
    // (intel providers + `resources/integrations/core.json`).
    let integrations_state = IntegrationsState::build_default(settings_mgr.clone());

    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .manage(app_state)
        .manage(db_state)
        .manage(telemetry_state)
        .manage(mcp_managed)
        .manage(pty_state)
        .manage(sidecar_managed)
        .manage(settings_mgr)
        .manage(pentest_cfg)
        .manage(history_manager)
        .manage(Arc::new(FileWatcherState::new()))
        .manage(tools::pentest::PentestState::new())
        .manage(integrations_state)
        .on_window_event(|window, event| {
            crate::app::window_lifecycle::handle_window_event(window, event);
        })
        .setup(|app| crate::app::bootstrap::setup_subsystems(app))
}
