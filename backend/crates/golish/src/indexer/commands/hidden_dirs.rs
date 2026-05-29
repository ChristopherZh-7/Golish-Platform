//! Recent-directory listing + the user-managed hidden-dirs exclusion list.
//!
//! "Recent directories" come from `golish_session::list_recent_sessions`
//! and are deduplicated by workspace path.  The "hidden" list
//! (`~/.golish/hidden_dirs.json`) is just a flat array of paths the user
//! has chosen not to see again — `remove_recent_directory` adds an entry,
//! `list_recent_directories` filters them out.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::GolishError;

/// Format a UTC timestamp as a coarse relative time (e.g. "2h ago").
fn format_relative_time(datetime: chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(datetime);

    if duration.num_days() > 0 {
        format!("{}d ago", duration.num_days())
    } else if duration.num_hours() > 0 {
        format!("{}h ago", duration.num_hours())
    } else if duration.num_minutes() > 0 {
        format!("{}m ago", duration.num_minutes())
    } else {
        "just now".to_string()
    }
}

/// Recent directory information for the home view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentDirectory {
    /// Full path to the directory
    pub path: String,
    /// Directory name
    pub name: String,
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

fn save_hidden_dirs(dirs: &[String]) -> Result<(), GolishError> {
    let path = hidden_dirs_path()
        .ok_or_else(|| GolishError::Internal("Could not determine home directory".to_string()))?;
    let contents = serde_json::to_string(dirs).map_err(GolishError::from)?;
    std::fs::write(&path, contents).map_err(GolishError::from)
}

/// Remove a directory from the recent directories list by adding it to
/// the hidden-dirs exclusion list.
#[tauri::command]
pub async fn remove_recent_directory(path: String) -> Result<(), GolishError> {
    let mut hidden = load_hidden_dirs();
    if !hidden.contains(&path) {
        hidden.push(path);
        save_hidden_dirs(&hidden)?;
    }
    Ok(())
}

/// List recent directories from AI session history.
#[tauri::command]
pub async fn list_recent_directories(
    limit: Option<usize>,
) -> Result<Vec<RecentDirectory>, GolishError> {
    let hidden_dirs = load_hidden_dirs();

    let sessions = golish_session::list_recent_sessions(limit.unwrap_or(20))
        .await
        .map_err(GolishError::from)?;

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

        directories.push(RecentDirectory {
            path: session.workspace_path,
            name,
            last_accessed: format_relative_time(session.ended_at),
        });
    }

    Ok(directories)
}
