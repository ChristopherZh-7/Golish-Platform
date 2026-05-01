//! Project configuration storage and file management for Golish.
//!
//! This crate owns the project lifecycle: creating, loading, saving, and
//! deleting project configurations, plus managing the on-disk directory
//! structure (captures, tool output, evidence, scripts, analysis) under
//! `{project_root}/.golish/`.
//!
//! It has **no** Tauri dependency — the application layer provides thin
//! `#[tauri::command]` wrappers.
//!
//! ## Layout
//! - [`schema`]       — `ProjectConfig` struct.
//! - [`storage`]      — CRUD operations for `~/.golish/projects/<slug>/`.
//! - [`file_storage`] — on-disk file management under `{root}/.golish/`.

pub mod file_storage;
pub mod schema;
pub mod storage;

pub use file_storage::PentestProjectConfig;
pub use schema::ProjectConfig;
pub use storage::{
    delete_project, list_projects, load_project, save_project,
    load_workspace, save_workspace,
};
