//! Raw indexer lifecycle commands.
//!
//! These wrap `AppState::indexer_state` directly: initialise the singleton
//! against a workspace, index files or directories into it, search the
//! existing index, and shut it down.  Multi-codebase registry concerns
//! (settings entries, memory files, migration) live one module over in
//! [`super::codebases`] — this file knows nothing about that.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::state::AppState;

/// Result of indexing a file or directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexResult {
    pub files_indexed: usize,
    pub success: bool,
    pub message: String,
}

/// Search result from the indexer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexSearchResult {
    pub file_path: String,
    pub line_number: usize,
    pub line_content: String,
    pub matches: Vec<String>,
}

/// Initialize the code indexer for a workspace.
#[tauri::command]
pub async fn init_indexer(
    workspace_path: String,
    state: State<'_, AppState>,
) -> Result<IndexResult, String> {
    tracing::info!("init_indexer called with workspace: {}", workspace_path);

    let path = PathBuf::from(&workspace_path);

    if !path.exists() {
        tracing::error!("Workspace path does not exist: {}", workspace_path);
        return Err(format!("Workspace path does not exist: {}", workspace_path));
    }

    // Get index location from settings
    let settings = state.settings_manager.get().await;
    let index_location = settings.indexer.index_location;

    tracing::debug!(
        "Workspace path exists, initializing indexer state with location: {:?}",
        index_location
    );

    state
        .indexer_state
        .initialize_with_location(path, index_location)
        .map_err(|e| {
            tracing::error!("Failed to initialize indexer: {}", e);
            e.to_string()
        })?;

    tracing::info!(
        "init_indexer completed successfully for: {}",
        workspace_path
    );

    Ok(IndexResult {
        files_indexed: 0,
        success: true,
        message: format!("Indexer initialized for workspace: {}", workspace_path),
    })
}

/// Check if the indexer is initialized.
#[tauri::command]
pub fn is_indexer_initialized(state: State<'_, AppState>) -> bool {
    state.indexer_state.is_initialized()
}

/// Get the current workspace root.
#[tauri::command]
pub fn get_indexer_workspace(state: State<'_, AppState>) -> Option<String> {
    state
        .indexer_state
        .workspace_root()
        .map(|p| p.to_string_lossy().to_string())
}

/// Get the count of indexed files.
#[tauri::command]
pub fn get_indexed_file_count(state: State<'_, AppState>) -> Result<usize, String> {
    state
        .indexer_state
        .with_indexer(|indexer| {
            // Use all_files() instead of find_files("*") - more efficient and doesn't require regex
            Ok(indexer.all_files().len())
        })
        .map_err(|e| e.to_string())
}

/// Get all indexed file paths as absolute paths.
/// Returns an empty array if the indexer is not initialized (graceful degradation).
#[tauri::command]
pub fn get_all_indexed_files(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    // Return empty array if indexer not initialized - don't error
    if !state.indexer_state.is_initialized() {
        return Ok(Vec::new());
    }

    state
        .indexer_state
        .with_indexer(|indexer| {
            // all_files() already returns Vec<String> of absolute paths
            Ok(indexer.all_files())
        })
        .map_err(|e| e.to_string())
}

/// Index a specific file.
#[tauri::command]
pub async fn index_file(
    file_path: String,
    state: State<'_, AppState>,
) -> Result<IndexResult, String> {
    let path = PathBuf::from(&file_path);

    if !path.exists() {
        return Err(format!("File does not exist: {}", file_path));
    }

    state
        .indexer_state
        .with_indexer_mut(|indexer| {
            indexer.index_file(&path)?;
            Ok(())
        })
        .map_err(|e| e.to_string())?;

    Ok(IndexResult {
        files_indexed: 1,
        success: true,
        message: format!("Indexed file: {}", file_path),
    })
}

/// Index a directory recursively.
#[tauri::command]
pub async fn index_directory(
    dir_path: String,
    state: State<'_, AppState>,
) -> Result<IndexResult, String> {
    tracing::info!("index_directory called with path: {}", dir_path);

    let path = PathBuf::from(&dir_path);

    if !path.exists() {
        tracing::error!("Directory does not exist: {}", dir_path);
        return Err(format!("Directory does not exist: {}", dir_path));
    }

    tracing::debug!("Directory exists, checking indexer state...");
    tracing::debug!(
        "Indexer initialized: {}",
        state.indexer_state.is_initialized()
    );

    state
        .indexer_state
        .with_indexer_mut(|indexer| {
            tracing::info!("Starting directory indexing for: {:?}", path);
            let start = std::time::Instant::now();

            indexer.index_directory(&path)?;

            tracing::info!("Directory indexing completed in {:?}", start.elapsed(),);
            Ok(())
        })
        .map_err(|e| {
            tracing::error!("Failed to index directory: {}", e);
            e.to_string()
        })?;

    // Get the actual file count after indexing
    let files_indexed = state
        .indexer_state
        .with_indexer(|indexer| {
            let files = indexer.all_files();
            tracing::info!("Total files in index after indexing: {}", files.len());
            Ok(files.len())
        })
        .unwrap_or(0);

    tracing::info!(
        "index_directory completed successfully, {} files now in index",
        files_indexed
    );

    Ok(IndexResult {
        files_indexed,
        success: true,
        message: format!(
            "Indexed directory: {} ({} files in index)",
            dir_path, files_indexed
        ),
    })
}

/// Search for content in indexed files.
#[tauri::command]
pub async fn search_code(
    pattern: String,
    path_filter: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<IndexSearchResult>, String> {
    state
        .indexer_state
        .with_indexer(|indexer| {
            let results = indexer.search(&pattern, path_filter.as_deref())?;
            Ok(results
                .into_iter()
                .map(|r| IndexSearchResult {
                    file_path: r.file_path,
                    line_number: r.line_number,
                    line_content: r.line_content,
                    matches: r.matches,
                })
                .collect())
        })
        .map_err(|e| e.to_string())
}

/// Search for files by name pattern.
#[tauri::command]
pub async fn search_files(
    pattern: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    state
        .indexer_state
        .with_indexer(|indexer| {
            let results = indexer.find_files(&pattern)?;
            Ok(results)
        })
        .map_err(|e| e.to_string())
}

/// Shutdown the indexer.
#[tauri::command]
pub fn shutdown_indexer(state: State<'_, AppState>) {
    state.indexer_state.shutdown();
}
