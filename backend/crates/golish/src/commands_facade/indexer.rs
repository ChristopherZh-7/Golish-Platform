//! Indexer and codebase management commands.

pub use crate::indexer::commands::core::{
    init_indexer, is_indexer_initialized, get_indexer_workspace,
    get_indexed_file_count, get_all_indexed_files,
    index_file, index_directory, search_code, search_files, shutdown_indexer,
};
pub use crate::indexer::commands::codebases::{
    list_indexed_codebases, add_indexed_codebase, remove_indexed_codebase,
    reindex_codebase, migrate_codebase_index, update_codebase_memory_file,
    detect_memory_files,
};
pub use crate::indexer::commands::hidden_dirs::{
    list_projects_for_home, list_recent_directories, remove_recent_directory,
};
pub use crate::indexer::commands::worktrees::{
    list_git_branches, create_git_worktree,
};
