//! Path resolution helpers for codebase management.

use std::path::Path;

use golish_settings::schema::IndexLocation;

use crate::paths::{compute_index_dir, find_existing_index_dir};

// Tilde-expansion helpers are owned by `golish-core::paths`; re-export at
// the previous `golish_indexer::path_helpers::{expand_home_dir,contract_home_dir}`
// paths so existing call sites stay stable.
pub use golish_core::paths::{contract_home_dir, expand_tilde as expand_home_dir};

/// Count the indexed files for a codebase by inspecting the on-disk index directory.
pub fn get_codebase_file_count(path: &Path) -> usize {
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
