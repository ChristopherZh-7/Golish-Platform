//! Session file I/O operations.
//!
//! This module handles reading and writing session files to disk.
//! Sessions are stored as JSON files in `~/.golish/sessions/` (or `$VT_SESSION_DIR`).

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};

use super::listing::{SessionListing, SessionSnapshot};

#[cfg(test)]
mod tests;

/// Get the sessions directory path.
///
/// Respects the `VT_SESSION_DIR` environment variable for compatibility
/// with vtcode-core's session_archive module.
///
/// Default: `~/.golish/sessions/`
pub fn get_sessions_dir() -> Result<PathBuf> {
    let dir = if let Ok(custom) = std::env::var("VT_SESSION_DIR") {
        PathBuf::from(custom)
    } else {
        dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?
            .join(".golish")
            .join("sessions")
    };

    fs::create_dir_all(&dir).context("Failed to create sessions directory")?;

    Ok(dir)
}

/// Generate a filename for a session based on its metadata.
///
/// Format: `session-{workspace_label}-{timestamp}_{session_id_prefix}.json`
///
/// Example: `session-my-project-20251214T084335Z_012542-99688.json`
pub fn generate_filename(
    workspace_label: &str,
    started_at: &chrono::DateTime<chrono::Utc>,
    session_id: &str,
) -> String {
    let timestamp = started_at.format("%Y%m%dT%H%M%SZ_%f");
    let id_prefix = &session_id[..session_id.len().min(5)];

    format!(
        "session-{}-{}-{}.json",
        workspace_label, timestamp, id_prefix
    )
}

/// Save a session snapshot to disk.
///
/// Returns the path to the saved file.
pub fn save_session(dir: &std::path::Path, snapshot: &SessionSnapshot) -> Result<PathBuf> {
    let filename = generate_filename(
        &snapshot.metadata.workspace_label,
        &snapshot.started_at,
        &snapshot.metadata.session_id,
    );
    let path = dir.join(&filename);

    let json =
        serde_json::to_string_pretty(snapshot).context("Failed to serialize session snapshot")?;

    fs::write(&path, json).context("Failed to write session file")?;

    Ok(path)
}

/// Find a session by its identifier.
///
/// The identifier can be:
/// - A session ID (or prefix thereof)
/// - Part of the filename
///
/// Returns the first matching session.
pub fn find_session(identifier: &str) -> Result<Option<SessionListing>> {
    let dir = get_sessions_dir()?;

    let entries: Vec<_> = fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "json")
                .unwrap_or(false)
        })
        .collect();

    for entry in entries {
        let path = entry.path();

        let filename = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();

        let matches_filename = filename.contains(identifier);

        if matches_filename {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(snapshot) = serde_json::from_str::<SessionSnapshot>(&content) {
                    if snapshot.metadata.session_id.starts_with(identifier)
                        || filename.contains(identifier)
                    {
                        return Ok(Some(SessionListing::from_snapshot(snapshot, path)));
                    }
                }
            }
        } else if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(snapshot) = serde_json::from_str::<SessionSnapshot>(&content) {
                if snapshot.metadata.session_id.starts_with(identifier) {
                    return Ok(Some(SessionListing::from_snapshot(snapshot, path)));
                }
            }
        }
    }

    Ok(None)
}

/// List all sessions, sorted by start time (most recent first).
///
/// # Arguments
/// * `limit` - Maximum number of sessions to return. Pass 0 for unlimited.
pub fn list_sessions(limit: usize) -> Result<Vec<SessionListing>> {
    let dir = get_sessions_dir()?;
    let mut sessions = Vec::new();

    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();

        if !path.extension().map(|ext| ext == "json").unwrap_or(false) {
            continue;
        }

        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(snapshot) = serde_json::from_str::<SessionSnapshot>(&content) {
                sessions.push(SessionListing::from_snapshot(snapshot, path));
            }
        }
    }

    sessions.sort_by(|a, b| b.started_at.cmp(&a.started_at));

    if limit > 0 && sessions.len() > limit {
        sessions.truncate(limit);
    }

    Ok(sessions)
}

/// Get the sessions directory for a specific workspace (project).
///
/// Returns `{workspace}/.golish/sessions/` when workspace is a real path,
/// or falls back to the global `~/.golish/sessions/`.
pub fn get_sessions_dir_for(workspace: &std::path::Path) -> Result<PathBuf> {
    let ws_str = workspace.to_string_lossy();
    let dir = if ws_str != "." && !ws_str.is_empty() {
        workspace.join(".golish").join("sessions")
    } else {
        return get_sessions_dir();
    };

    fs::create_dir_all(&dir).context("Failed to create project sessions directory")?;
    Ok(dir)
}

/// Find a session by identifier, searching in a specific workspace first,
/// then falling back to the global sessions directory.
pub fn find_session_in_workspace(
    identifier: &str,
    workspace: &std::path::Path,
) -> Result<Option<SessionListing>> {
    let project_dir = get_sessions_dir_for(workspace)?;
    if let Some(session) = find_session_in_dir(identifier, &project_dir)? {
        return Ok(Some(session));
    }
    find_session(identifier)
}

/// List sessions from a specific workspace, merged with global sessions.
pub fn list_sessions_for_workspace(
    limit: usize,
    workspace: &std::path::Path,
) -> Result<Vec<SessionListing>> {
    let mut sessions = Vec::new();

    let project_dir = get_sessions_dir_for(workspace)?;
    if project_dir.exists() {
        sessions.extend(collect_sessions_from_dir(&project_dir)?);
    }

    let global_dir = get_sessions_dir()?;
    if global_dir.exists() {
        sessions.extend(collect_sessions_from_dir(&global_dir)?);
    }

    sessions.sort_by(|a, b| b.started_at.cmp(&a.started_at));

    if limit > 0 && sessions.len() > limit {
        sessions.truncate(limit);
    }

    Ok(sessions)
}

fn find_session_in_dir(
    identifier: &str,
    dir: &std::path::Path,
) -> Result<Option<SessionListing>> {
    if !dir.exists() {
        return Ok(None);
    }

    let entries: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "json")
                .unwrap_or(false)
        })
        .collect();

    for entry in entries {
        let path = entry.path();
        let filename = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();

        if filename.contains(identifier) {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(snapshot) = serde_json::from_str::<SessionSnapshot>(&content) {
                    if snapshot.metadata.session_id.starts_with(identifier)
                        || filename.contains(identifier)
                    {
                        return Ok(Some(SessionListing::from_snapshot(snapshot, path)));
                    }
                }
            }
        } else if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(snapshot) = serde_json::from_str::<SessionSnapshot>(&content) {
                if snapshot.metadata.session_id.starts_with(identifier) {
                    return Ok(Some(SessionListing::from_snapshot(snapshot, path)));
                }
            }
        }
    }

    Ok(None)
}

fn collect_sessions_from_dir(dir: &std::path::Path) -> Result<Vec<SessionListing>> {
    let mut sessions = Vec::new();
    if !dir.exists() {
        return Ok(sessions);
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.extension().map(|ext| ext == "json").unwrap_or(false) {
            continue;
        }
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(snapshot) = serde_json::from_str::<SessionSnapshot>(&content) {
                sessions.push(SessionListing::from_snapshot(snapshot, path));
            }
        }
    }

    Ok(sessions)
}
