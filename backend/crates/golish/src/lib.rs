// `too_many_arguments` is intentionally allowed crate-wide: most `#[tauri::command]`
// functions thread `tauri::AppHandle`, multiple `State<'_, ...>` handles and a
// dozen of optional request fields straight from the frontend; refactoring each
// into a dedicated DTO struct adds boilerplate without any safety win.
#![allow(clippy::too_many_arguments)]

//! Golish desktop application crate.
//!
//! Bootstraps the Tauri runtime, manages global state, and wires every
//! `#[tauri::command]` to the frontend through `invoke_handler`. The
//! ~300-entry command list itself lives in `commands_registry.rs`,
//! `include!`-d at crate root so the `__cmd__$name` macros emitted by
//! `#[tauri::command]` (which `#[macro_export]` to crate root, not
//! sub-modules) remain in scope at the `tauri::generate_handler!` call
//! site.
//!
//! Layout:
//! - [`app::bootstrap`]    — process-level setup (CLI args, telemetry, DB,
//!   embedded Postgres, default agent files, history manager).
//! - [`app::tauri_app`]    — plugin / managed-state / lifecycle wiring on
//!   the Tauri builder.
//! - `commands_registry.rs` (`include!`-d) — the giant
//!   `tauri::generate_handler![...]` that this `lib.rs` used to host.
//! - [`app::window_lifecycle`] — runtime event handler.

pub mod ai;
pub use golish_cli_output as cli_output;
pub mod compat;
pub(crate) mod db;
mod error;
pub mod history;
mod indexer;
mod mcp;
mod models;
mod pentest_tool_factory;
mod projects;
mod pty;
pub mod runtime;
mod settings;
mod sidecar;
mod state;
pub mod telemetry;
pub mod tools;
mod window_state;

mod app;
pub mod cli;
mod commands;
mod commands_facade;

/// Tauri application entry point for GUI mode.
///
/// Bootstraps process-level state via `app::bootstrap::*`, configures the
/// Tauri `Builder` (plugins, managed state, lifecycle hooks) via
/// `app::tauri_app::configure_builder`, then attaches the full command
/// registry through [`install_handlers`] (defined in `commands_registry.rs`).
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run_gui() {
    app::bootstrap::apply_cli_workspace_arg();
    app::bootstrap::install_rustls_crypto_provider();
    app::bootstrap::load_dotenv();
    app::bootstrap::set_default_session_dir();

    let (_telemetry_guard, app_state) = app::bootstrap::init_telemetry_and_app_state();

    app::bootstrap::spawn_embedded_pg(app_state.db_ready.clone());
    app::bootstrap::seed_default_agent_files();

    let history_manager = app::bootstrap::init_history_manager_background();
    app::bootstrap::spawn_ensure_settings_file(app_state.settings_manager.clone());

    let builder =
        app::tauri_app::configure_builder(tauri::Builder::default(), app_state, history_manager);

    install_handlers(builder)
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            crate::app::window_lifecycle::handle_run_event(app_handle, event);
        });
}

include!("commands_registry.rs");
