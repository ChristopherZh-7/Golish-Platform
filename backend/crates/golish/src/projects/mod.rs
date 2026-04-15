//! Project configuration storage and management.
//!
//! Projects are stored as directories in `~/.golish/projects/<slug>/`.
//! Each directory contains:
//! - `config.toml` — project configuration (name, root path)
//! - `workspace.json` — persisted workspace state (conversations, chat history)
//!
//! Project data is stored under `{root_path}/.golish/`:
//! - `project.json` — pentest project configuration (scope, proxy, capture settings)
//! - `captures/` — raw captured files (JS, HTML, HTTP dumps)
//! - `tool-output/` — tool execution output
//! - `scripts/` — AI-generated scripts
//! - `evidence/` — finding evidence files
//! - `analysis/` — AI analysis reports
//! - `temp/` — temporary files

mod schema;
mod storage;

pub mod commands;
pub mod file_storage;

pub use schema::ProjectConfig;
pub use storage::{delete_project, list_projects, load_project, save_project};
