//! Tauri command surface for the GUI process, grouped by domain.
//!
//! Each subdomain owns a small, cohesive subset of `#[tauri::command]`
//! functions plus their helper types. Sub-modules then re-export their
//! public items here so `lib.rs` can keep using `use commands::*;` and
//! `tauri::generate_handler!` resolves names without per-symbol prefixes.
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

pub use fs::*;
pub use proc::*;
pub use project::*;
pub use ui::*;
