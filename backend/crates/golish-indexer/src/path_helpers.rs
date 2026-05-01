//! Path resolution helpers for codebase management.

use std::path::{Path, PathBuf};

use golish_settings::schema::IndexLocation;

use crate::paths::{compute_index_dir, find_existing_index_dir};

pub fn expand_home_dir(path: &str) -> PathBuf {
    if path.starts_with("~/") {
        dirs::home_dir()
            .map(|home| home.join(&path[2..]))
            .unwrap_or_else(|| PathBuf::from(path))
    } else {
        PathBuf::from(path)
    }
}

pub fn contract_home_dir(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(stripped) = path.strip_prefix(&home) {
            return format!("~/{}", stripped.display());
        }
    }
    path.to_string_lossy().to_string()
}

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
