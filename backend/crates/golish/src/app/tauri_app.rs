//! Tauri builder configuration (plugins, managed state, lifecycle hooks).
//!
//! The giant `tauri::generate_handler![...]` invocation has to stay in
//! `lib.rs` because Tauri's `#[command]` proc macro generates `__cmd__$name`
//! macros via `#[macro_export]`, which are only directly visible at the
//! crate root.  Everything else that used to live inline in
//! `tauri::Builder::default()...` now lives here.

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::commands::FileWatcherState;
use crate::history::HistoryManager;
use crate::state::AppState;
use crate::tools;

/// Apply plugins, managed state and lifecycle hooks to the given Tauri
/// builder. The caller is responsible for chaining `invoke_handler`,
/// `build`, and `run` afterwards.
///
/// Typed for the default Tauri runtime (Wry); the lifecycle helpers in
/// `crate::app::window_lifecycle` and `crate::app::bootstrap` use the same
/// concrete types.
pub(crate) fn configure_builder(
    builder: tauri::Builder<tauri::Wry>,
    app_state: AppState,
    history_manager: Arc<RwLock<Option<HistoryManager>>>,
) -> tauri::Builder<tauri::Wry> {
    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .manage(app_state)
        .manage(history_manager)
        .manage(Arc::new(FileWatcherState::new()))
        .manage(tools::pentest::PentestState::new())
        .on_window_event(|window, event| {
            crate::app::window_lifecycle::handle_window_event(window, event);
        })
        .setup(|app| crate::app::bootstrap::setup_subsystems(app))
}
