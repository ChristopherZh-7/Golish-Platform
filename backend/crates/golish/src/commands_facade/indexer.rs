//! Indexer and codebase management commands.
//!
//! Expected commands exposed here (documentation only):
//! - **Core**: `init_indexer`, `is_indexer_initialized`,
//!   `get_indexer_workspace`, `get_indexed_file_count`,
//!   `get_all_indexed_files`, `index_file`, `index_directory`,
//!   `search_code`, `search_files`, `shutdown_indexer`
//! - **Codebases**: `list_indexed_codebases`, `add_indexed_codebase`,
//!   `remove_indexed_codebase`, `reindex_codebase`,
//!   `migrate_codebase_index`, `update_codebase_memory_file`,
//!   `detect_memory_files`
//! - **Hidden dirs / recents**: `list_recent_directories`,
//!   `remove_recent_directory`

pub use crate::indexer::commands::*;
