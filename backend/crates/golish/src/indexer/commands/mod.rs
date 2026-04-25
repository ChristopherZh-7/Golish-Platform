//! Tauri commands for code-indexer operations.
//!
//! Originally a single file mixing five distinct concerns; split here by
//! domain so each surface has its own home and the diff churn from one
//! domain doesn't pollute the others:
//!
//! - [`core`]         — the raw indexer lifecycle: init / index / search /
//!   shutdown.  Operates on the live `AppState::indexer_state`.
//! - [`codebases`]    — the multi-codebase registry surfaced in Settings:
//!   add / remove / re-index / migrate / memory-file management.
//! - [`home_view`]    — the project & worktree summary that backs the
//!   Home view (`list_projects_for_home`).
//! - [`hidden_dirs`]  — recent-directories listing + the user-managed
//!   hidden-dirs exclusion list (`~/.golish/hidden_dirs.json`).
//! - [`worktrees`]    — git worktree CRUD (`list_git_branches`,
//!   `create_git_worktree`).
//!
//! Public command names and DTO shapes are unchanged; `lib.rs` glob-imports
//! `indexer::commands::*` so `tauri::generate_handler!` continues to find
//! every `#[tauri::command]` by its bare name.

mod codebases;
mod core;
mod hidden_dirs;
mod home_view;
mod worktrees;

// Re-export every public surface so the glob `use indexer::commands::*;`
// in lib.rs (used by the giant `tauri::generate_handler!` block) resolves
// each command name without reaching into submodule paths.  The DTO
// re-exports look "unused" to rustc because they're referenced only
// through `#[tauri::command]` return-type positions and the macro-emitted
// `__cmd__$name` infrastructure — silence that here so the back-compat
// path `indexer::commands::CodebaseInfo` etc. stays reachable.
#[allow(unused_imports)]
pub use codebases::{
    add_indexed_codebase, detect_memory_files, list_indexed_codebases, migrate_codebase_index,
    reindex_codebase, remove_indexed_codebase, update_codebase_memory_file, CodebaseInfo,
};
#[allow(unused_imports)]
pub use core::{
    get_all_indexed_files, get_indexed_file_count, get_indexer_workspace, index_directory,
    index_file, init_indexer, is_indexer_initialized, search_code, search_files, shutdown_indexer,
    IndexResult, IndexSearchResult,
};
#[allow(unused_imports)]
pub use hidden_dirs::{list_recent_directories, remove_recent_directory, RecentDirectory};
#[allow(unused_imports)]
pub use home_view::{list_projects_for_home, BranchInfo, ProjectInfo};
#[allow(unused_imports)]
pub use worktrees::{create_git_worktree, list_git_branches, WorktreeCreated};
