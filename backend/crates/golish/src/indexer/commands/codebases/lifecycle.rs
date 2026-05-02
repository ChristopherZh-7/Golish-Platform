//! Tauri commands: re-index + index-location migration.

use golish_indexer::paths::{compute_index_dir, migrate_index};
use tauri::State;

use super::{contract_home_dir, expand_home_dir, get_codebase_file_count, CodebaseInfo};
use crate::error::GolishError;
use crate::settings::schema::IndexLocation;
use crate::state::AppState;

/// Re-index a codebase (clear and rebuild the index).
#[tauri::command]
pub async fn reindex_codebase(
    path: String,
    state: State<'_, AppState>,
) -> Result<CodebaseInfo, String> {
    tracing::info!("reindex_codebase called with path: {}", path);

    let expanded_path = expand_home_dir(&path);
    let normalized_path = expanded_path
        .canonicalize()
        .map_err(|e| format!("Invalid path: {}", e))?;

    if !normalized_path.exists() {
        return Err(format!("Path does not exist: {}", path));
    }

    let settings = state.settings_manager.get().await;
    let index_location = settings.indexer.index_location;
    let memory_file = settings
        .codebases
        .iter()
        .find(|config| {
            let config_expanded = expand_home_dir(&config.path);
            config_expanded
                .canonicalize()
                .ok()
                .map(|p| p == normalized_path)
                .unwrap_or(false)
        })
        .and_then(|config| config.memory_file.clone());

    let global_index_dir = compute_index_dir(&normalized_path, IndexLocation::Global);
    if global_index_dir.exists() {
        std::fs::remove_dir_all(&global_index_dir)
            .map_err(|e| format!("Failed to delete global index directory: {}", e))?;
        tracing::info!(
            "Deleted existing global index directory: {:?}",
            global_index_dir
        );
    }
    let local_index_dir = compute_index_dir(&normalized_path, IndexLocation::Local);
    if local_index_dir.exists() {
        std::fs::remove_dir_all(&local_index_dir)
            .map_err(|e| format!("Failed to delete local index directory: {}", e))?;
        tracing::info!(
            "Deleted existing local index directory: {:?}",
            local_index_dir
        );
    }

    crate::indexer::vtcode_bridge::initialize_vtcode_indexer(
        &state.indexer_state,
        normalized_path.clone(),
        index_location,
    )
    .map_err(|e| format!("Failed to initialize indexer: {}", e))?;

    state
        .indexer_state
        .with_indexer_mut(|indexer| {
            indexer.index_directory(&normalized_path)?;
            Ok(())
        })
        .map_err(|e| format!("Failed to index directory: {}", e))?;

    let file_count = get_codebase_file_count(&normalized_path);
    let display_path = contract_home_dir(&normalized_path);

    tracing::info!(
        "Re-indexed codebase {} with {} files",
        display_path,
        file_count
    );

    Ok(CodebaseInfo {
        path: display_path,
        file_count,
        status: "synced".to_string(),
        error: None,
        memory_file,
    })
}

/// Migrate a codebase's index to the configured storage location.
#[tauri::command]
pub async fn migrate_codebase_index(
    path: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, GolishError> {
    tracing::info!("migrate_codebase_index called with path: {}", path);

    let expanded_path = expand_home_dir(&path);
    let normalized_path = expanded_path
        .canonicalize()
        .map_err(|e| GolishError::Validation(format!("Invalid path: {}", e)))?;

    let settings = state.settings_manager.get().await;
    let target_location = settings.indexer.index_location;

    let from_location = if compute_index_dir(&normalized_path, IndexLocation::Local).exists() {
        IndexLocation::Local
    } else if compute_index_dir(&normalized_path, IndexLocation::Global).exists() {
        IndexLocation::Global
    } else {
        return Ok(None);
    };

    migrate_index(&normalized_path, from_location, target_location)
        .map(|opt| opt.map(|p| p.to_string_lossy().to_string()))
        .map_err(GolishError::from)
}
