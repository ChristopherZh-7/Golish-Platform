//! Project configuration storage and management.
//!
//! Projects are stored as directories in `~/.golish/projects/<slug>/`.
//! Each directory contains:
//! - `config.toml` — project configuration (name, root path)
//! - `workspace.json` — persisted workspace state (conversations, chat history)

mod schema;
mod storage;

pub mod commands;

pub use schema::ProjectConfig;
pub use storage::{delete_project, list_projects, load_project, save_project};
