//! Code indexer foundation for Golish.
//!
//! This crate owns the indexer trait, state container, path resolution, and
//! the vtcode-indexer backend implementation. It also hosts shared helper
//! functions for codebase path resolution and git utilities used by the
//! home view and codebase management commands.
//!
//! It has **no** Tauri dependency and **no** dependency on other Layer-3
//! crates (`golish-ai`, `golish-tools`, etc.). Higher-level consumers
//! (notably `golish-ai`) depend on this crate to access `IndexerState` and
//! the `IndexerBackend` trait.
//!
//! ## Layout
//! - [`state`]          — `IndexerBackend` trait + `IndexerState` + `CodeSearchResult`.
//! - [`paths`]          — `compute_index_dir`, `find_existing_index_dir`, `migrate_index`.
//! - [`vtcode_bridge`]  — `VtcodeIndexerBackend` + `initialize_vtcode_indexer`.
//! - [`path_helpers`]   — `expand_home_dir`, `contract_home_dir`, `get_codebase_file_count`.
//! - [`git_helpers`]    — git stats, worktree listing, relative time formatting.
//! - [`types`]          — DTOs shared with the Tauri command layer.

pub mod git_helpers;
pub mod path_helpers;
pub mod paths;
pub mod state;
pub mod types;
pub mod vtcode_bridge;

pub use git_helpers::{format_relative_time, get_git_stats};
pub use path_helpers::{contract_home_dir, expand_home_dir, get_codebase_file_count};
pub use paths::{compute_index_dir, find_existing_index_dir, migrate_index};
pub use state::{CodeSearchResult, IndexerBackend, IndexerState};
pub use types::{
    BranchInfo, CodebaseInfo, IndexResult, IndexSearchResult, ProjectInfo, RecentDirectory,
    WorktreeCreated,
};
pub use vtcode_bridge::initialize_vtcode_indexer;
