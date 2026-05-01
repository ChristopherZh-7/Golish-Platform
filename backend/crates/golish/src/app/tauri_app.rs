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

/// Apply plugins, managed state and lifecycle hooks to the given Tauri
/// builder. The caller is responsible for chaining `invoke_handler`,
/// `build`, and `run` afterwards.
pub(crate) fn configure_builder(
    builder: tauri::Builder<tauri::Wry>,
    app_state: AppState,
    history_manager: Arc<RwLock<Option<HistoryManager>>>,
) -> tauri::Builder<tauri::Wry> {
    let db_state = app_state.extract_db_state();
    let settings_mgr = app_state.settings_manager.clone();
    let pentest_cfg = app_state.pentest_config_manager.clone();

    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .manage(app_state)
        .manage(db_state)
        .manage(settings_mgr)
        .manage(pentest_cfg)
        .manage(history_manager)
        .manage(Arc::new(FileWatcherState::new()))
        .manage(tools::pentest::PentestState::new())
        .on_window_event(|window, event| {
            crate::app::window_lifecycle::handle_window_event(window, event);
        })
        .setup(|app| crate::app::bootstrap::setup_subsystems(app))
}
