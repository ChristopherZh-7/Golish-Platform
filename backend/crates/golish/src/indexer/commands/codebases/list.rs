//! Tauri command: `list_indexed_codebases`.
//!
//! Reads both the legacy `Settings::indexed_codebases` (flat list) and the
//! richer `Settings::codebases` (`Vec<CodebaseConfig>`) and surfaces a
//! unified [`CodebaseInfo`] vector to the frontend.

use tauri::State;

use super::{expand_home_dir, get_codebase_file_count, CodebaseInfo};
use crate::state::AppState;

/// List all indexed codebases from settings.
#[tauri::command]
pub async fn list_indexed_codebases(
    state: State<'_, AppState>,
) -> Result<Vec<CodebaseInfo>, String> {
    let settings = state.settings_manager.get().await;

    let codebases: Vec<CodebaseInfo> = if !settings.codebases.is_empty() {
        settings
            .codebases
            .iter()
            .map(|config| {
                let path = expand_home_dir(&config.path);
                let exists = path.exists();
                let file_count = if exists {
                    get_codebase_file_count(&path)
                } else {
                    0
                };

                let (status, error) = if !exists {
                    ("error".to_string(), Some("Path does not exist".to_string()))
                } else if file_count > 0 {
                    ("synced".to_string(), None)
                } else {
                    ("not_indexed".to_string(), None)
                };

                CodebaseInfo {
                    path: config.path.clone(),
                    file_count,
                    status,
                    error,
                    memory_file: config.memory_file.clone(),
                }
            })
            .collect()
    } else {
        // Legacy format: migrate from indexed_codebases
        settings
            .indexed_codebases
            .iter()
            .map(|path_str| {
                let path = expand_home_dir(path_str);
                let exists = path.exists();
                let file_count = if exists {
                    get_codebase_file_count(&path)
                } else {
                    0
                };

                let (status, error) = if !exists {
                    ("error".to_string(), Some("Path does not exist".to_string()))
                } else if file_count > 0 {
                    ("synced".to_string(), None)
                } else {
                    ("not_indexed".to_string(), None)
                };

                CodebaseInfo {
                    path: path_str.clone(),
                    file_count,
                    status,
                    error,
                    memory_file: None,
                }
            })
            .collect()
    };

    Ok(codebases)
}
