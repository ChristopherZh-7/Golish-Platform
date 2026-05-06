//! Tauri command surface for the GUI process, grouped by domain.
//!
//! Each subdomain owns a small, cohesive subset of `#[tauri::command]`
//! functions plus their helper types. Consumers should reach into
//! these submodules via `crate::commands_facade::<domain>`; the
//! parent-level `pub use *` bridges that used to live here were
//! removed once the facade became the single source of truth.
//!
//! Domains:
//! - [`fs`] — filesystem (file CRUD, watcher, path completion)
//! - [`proc`] — processes / terminal / shell / git / history
//! - [`project`] — project-level agent assets (prompts, rules, skills)
//! - [`ui`] — UI chrome (themes, IME, frontend log forwarder)

pub mod fs;
pub mod proc;
pub mod project;
pub mod ui;

// Type re-exports needed by other modules (e.g. `app::tauri_app`
// requires `crate::commands::FileWatcherState` for managed-state
// registration). The Tauri *command* surface no longer flows through
// this `pub use` block — see `commands_facade/<domain>.rs` instead.
pub use fs::*;
pub use proc::*;
