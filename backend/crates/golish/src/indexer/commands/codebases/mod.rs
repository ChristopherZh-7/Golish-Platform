//! Multi-codebase registry: the list of indexed codebases surfaced in
//! Settings, plus the lifecycle around each entry.
//!
//! Two persistence formats are supported in parallel for migration: the
//! legacy flat list (`Settings::indexed_codebases: Vec<String>`) and the
//! richer `Settings::codebases: Vec<CodebaseConfig>` which carries an
//! optional `memory_file` (AGENTS.md / CLAUDE.md).  Every command in this
//! module reads from both and writes to the new format, slowly migrating
//! the user's settings file as they touch it.
//!
//! Path-handling helpers ([`expand_home_dir`], [`contract_home_dir`],
//! [`get_codebase_file_count`]) are kept `pub(super)` so sibling
//! modules can reuse them without re-implementing the
//! `~/`-expansion convention.

use golish_indexer::paths::{compute_index_dir, find_existing_index_dir};
use serde::{Deserialize, Serialize};

use crate::settings::schema::IndexLocation;

mod crud;
mod lifecycle;
mod list;
mod memory;

pub use crud::{add_indexed_codebase, remove_indexed_codebase};
pub use lifecycle::{migrate_codebase_index, reindex_codebase};
pub use list::list_indexed_codebases;
pub use memory::{detect_memory_files, update_codebase_memory_file};

/// Information about an indexed codebase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodebaseInfo {
    /// The path to the codebase
    pub path: String,
    /// Number of indexed files (0 if not yet indexed)
    pub file_count: usize,
    /// Current status: "synced", "indexing", "not_indexed", or "error"
    pub status: String,
    /// Error message if status is "error"
    pub error: Option<String>,
    /// Memory file associated with this codebase: "AGENTS.md", "CLAUDE.md", or None
    pub memory_file: Option<String>,
}

/// Tilde-expansion helpers live in `golish-core::paths`; re-export at the
/// previous `pub(super)` paths so sibling modules keep working.
pub(super) use golish_core::paths::{contract_home_dir, expand_tilde as expand_home_dir};

/// Helper to get file count for a codebase's index directory.
///
/// Checks both global and local locations for backward compatibility — old
/// installs may still have on-disk indices in either spot.
pub(super) fn get_codebase_file_count(path: &std::path::Path) -> usize {
    let index_dir = find_existing_index_dir(path, IndexLocation::Global)
        .unwrap_or_else(|| compute_index_dir(path, IndexLocation::Global));

    if !index_dir.exists() {
        return 0;
    }

    std::fs::read_dir(&index_dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().and_then(|ext| ext.to_str()) == Some("md"))
                .count()
        })
        .unwrap_or(0)
}
