//! Filesystem-domain Tauri commands.
//!
//! - [`files`]: workspace file CRUD and listing
//! - [`file_watcher`]: editor sidebar file-change notifier
//! - [`completions`]: tab-completion for path inputs
//!
//! All public items are re-exported at the parent [`crate::commands`] level so
//! existing `crate::commands::*` and `tauri::generate_handler!` lookups in
//! `lib.rs` keep working unchanged after the directory reshuffle.

mod completions;
mod file_watcher;
mod files;

pub use completions::*;
pub use file_watcher::*;
pub use files::*;
