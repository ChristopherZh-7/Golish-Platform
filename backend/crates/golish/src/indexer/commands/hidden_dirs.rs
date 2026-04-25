//! Recent-directory listing + the user-managed hidden-dirs exclusion list.
//!
//! "Recent directories" come from `golish_session::list_recent_sessions`
//! and are deduplicated by workspace path.  The "hidden" list
//! (`~/.golish/hidden_dirs.json`) is just a flat array of paths the user
//! has chosen not to see again — `remove_recent_directory` adds an entry,
//! `list_recent_directories` filters them out.
//!
//! We borrow [`super::home_view::get_git_stats`] and
//! [`super::home_view::format_relative_time`] for the per-directory
//! summary so this module stays focused on the exclusion logic.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::home_view::{format_relative_time, get_git_stats};

/// Recent directory information for the home view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentDirectory {
    /// Full path to the directory
    pub path: String,
    /// Directory name
    pub name: String,
    /// Current git branch (if in a git repo)
    pub branch: Option<String>,
    /// Number of files with changes
    pub file_count: u32,
    /// Lines added
    pub insertions: i32,
    /// Lines deleted
    pub deletions: i32,
    /// Last accessed time (relative, e.g., "2h ago")
    pub last_accessed: String,
}

fn hidden_dirs_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".golish").join("hidden_dirs.json"))
}

fn load_hidden_dirs() -> Vec<String> {
    let Some(path) = hidden_dirs_path() else {
        return Vec::new();
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(&contents).unwrap_or_default()
}

fn save_hidden_dirs(dirs: &[String]) -> Result<(), String> {
    let path = hidden_dirs_path().ok_or("Could not determine home directory")?;
    let contents = serde_json::to_string(dirs).map_err(|e| e.to_string())?;
    std::fs::write(&path, contents).map_err(|e| e.to_string())
}

/// Remove a directory from the recent directories list by adding it to
/// the hidden-dirs exclusion list.
#[tauri::command]
pub async fn remove_recent_directory(path: String) -> Result<(), String> {
    let mut hidden = load_hidden_dirs();
    if !hidden.contains(&path) {
        hidden.push(path);
        save_hidden_dirs(&hidden)?;
    }
    Ok(())
}

/// List recent directories from AI session history.
#[tauri::command]
pub async fn list_recent_directories(limit: Option<usize>) -> Result<Vec<RecentDirectory>, String> {
    let hidden_dirs = load_hidden_dirs();

    let sessions = golish_session::list_recent_sessions(limit.unwrap_or(20))
        .await
        .map_err(|e| e.to_string())?;

    // Deduplicate by workspace_path, keeping the most recent
    let mut seen_paths = std::collections::HashSet::new();
    let mut directories = Vec::new();

    for session in sessions {
        if seen_paths.contains(&session.workspace_path) {
            continue;
        }
        // Skip paths that have been hidden by the user
        if hidden_dirs.contains(&session.workspace_path) {
            continue;
        }
        seen_paths.insert(session.workspace_path.clone());

        let path = PathBuf::from(&session.workspace_path);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| session.workspace_label.clone());

        // Get current git stats if the directory still exists
        let (branch, insertions, deletions, file_count) = if path.exists() {
            get_git_stats(&path).map_or((None, 0, 0, 0), |(b, i, d, f)| (Some(b), i, d, f))
        } else {
            (None, 0, 0, 0)
        };

        directories.push(RecentDirectory {
            path: session.workspace_path,
            name,
            branch,
            file_count,
            insertions,
            deletions,
            last_accessed: format_relative_time(session.ended_at),
        });
    }

    Ok(directories)
}
