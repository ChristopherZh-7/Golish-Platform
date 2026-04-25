//! Tab completion support for path navigation.
//!
//! This module provides the `list_path_completions` command that returns
//! file/directory completions for a given partial path, enabling tab completion
//! in the terminal input.

use crate::error::Result;
use crate::state::AppState;
use serde::Serialize;
use std::path::PathBuf;
use tauri::State;

/// Type of filesystem entry for path completions.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PathEntryType {
    File,
    Directory,
    Symlink,
}

/// A single path completion suggestion.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PathCompletion {
    /// Display name (e.g., "Documents/" for directories)
    pub name: String,
    /// Text to insert when this completion is selected
    pub insert_text: String,
    /// Type of filesystem entry
    pub entry_type: PathEntryType,
    /// Fuzzy match score (higher = better match)
    pub score: u32,
    /// Indices of matched characters for highlighting
    pub match_indices: Vec<usize>,
}

/// Response wrapper containing completions and total count.
#[derive(Debug, Clone, Serialize)]
pub struct PathCompletionResponse {
    /// The completions (limited by the limit parameter)
    pub completions: Vec<PathCompletion>,
    /// Total number of matches before limit was applied
    pub total_count: usize,
}

/// Default number of completions to return if no limit is specified.
const DEFAULT_LIMIT: usize = 20;

/// List path completions for a partial path input.
///
/// This command supports:
/// - Empty input (lists current directory)
/// - Tilde expansion (`~/` -> home directory)
/// - Absolute paths (`/`)
/// - Relative paths (`./`, `../`)
/// - Fuzzy matching with scoring and match highlighting
///
/// # Arguments
/// * `state` - Application state containing PTY manager
/// * `session_id` - PTY session ID (used to get working directory)
/// * `partial_path` - The partial path to complete
/// * `limit` - Maximum number of completions to return (default: 20)
///
/// # Returns
/// A `PathCompletionResponse` containing completions and total count.
#[tauri::command]
pub async fn list_path_completions(
    state: State<'_, AppState>,
    session_id: String,
    partial_path: String,
    limit: Option<usize>,
) -> Result<PathCompletionResponse> {
    // Get working directory from PTY session
    let session = state.pty_manager.get_session(&session_id)?;
    let working_dir = PathBuf::from(&session.working_directory);

    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    let response = compute_path_completions(&partial_path, &working_dir, limit);

    Ok(response)
}

mod compute;

#[cfg(test)]
mod tests;

pub use compute::compute_path_completions;
