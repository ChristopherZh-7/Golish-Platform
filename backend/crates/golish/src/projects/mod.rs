//! Project configuration storage and management.
//!
//! The pure business logic (config CRUD, file storage, directory structure)
//! now lives in the `golish-projects` crate. This module provides Tauri
//! command wrappers and re-exports the library types.

pub mod commands;

pub use golish_projects::file_storage;
