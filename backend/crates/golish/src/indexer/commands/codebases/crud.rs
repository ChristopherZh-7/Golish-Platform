//! Tauri commands: add / remove indexed codebases.

use golish_indexer::paths::compute_index_dir;
use tauri::State;

use super::{contract_home_dir, expand_home_dir, get_codebase_file_count, CodebaseInfo};
use crate::settings::schema::IndexLocation;
use crate::state::AppState;

/// Add a new codebase to the indexed list and start indexing.
#[tauri::command]
pub async fn add_indexed_codebase(
    path: String,
    state: State<'_, AppState>,
) -> Result<CodebaseInfo, String> {
    use crate::settings::schema::CodebaseConfig;

    tracing::info!("add_indexed_codebase called with path: {}", path);

    let expanded_path = expand_home_dir(&path);
    let normalized_path = expanded_path
        .canonicalize()
        .map_err(|e| format!("Invalid path: {}", e))?;

    if !normalized_path.exists() {
        return Err(format!("Path does not exist: {}", path));
    }

    if !normalized_path.is_dir() {
        return Err(format!("Path is not a directory: {}", path));
    }

    let display_path = contract_home_dir(&normalized_path);

    let settings = state.settings_manager.get().await;

    for existing in &settings.codebases {
        let existing_expanded = expand_home_dir(&existing.path);
        if let Ok(existing_canonical) = existing_expanded.canonicalize() {
            if existing_canonical == normalized_path {
                return Err(format!("Codebase already indexed: {}", display_path));
            }
        }
    }

    for existing in &settings.indexed_codebases {
        let existing_expanded = expand_home_dir(existing);
        if let Ok(existing_canonical) = existing_expanded.canonicalize() {
            if existing_canonical == normalized_path {
                return Err(format!("Codebase already indexed: {}", display_path));
            }
        }
    }

    let mut updated_settings = settings.clone();
    updated_settings.codebases.push(CodebaseConfig {
        path: display_path.clone(),
        memory_file: None,
    });

    let index_location = updated_settings.indexer.index_location;

    state
        .settings_manager
        .update(updated_settings)
        .await
        .map_err(|e| format!("Failed to save settings: {}", e))?;

    tracing::info!("Added codebase to settings: {}", display_path);

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

    tracing::info!(
        "Indexed codebase {} with {} files",
        display_path,
        file_count
    );

    Ok(CodebaseInfo {
        path: display_path,
        file_count,
        status: "synced".to_string(),
        error: None,
        memory_file: None,
    })
}

/// Remove a codebase from the indexed list and delete its index files.
#[tauri::command]
pub async fn remove_indexed_codebase(
    path: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    use crate::settings::schema::CodebaseConfig;

    tracing::info!("remove_indexed_codebase called with path: {}", path);

    let expanded_path = expand_home_dir(&path);

    let settings = state.settings_manager.get().await;

    let new_codebases: Vec<CodebaseConfig> = settings
        .codebases
        .iter()
        .filter(|config| {
            let p_expanded = expand_home_dir(&config.path);
            match (p_expanded.canonicalize(), expanded_path.canonicalize()) {
                (Ok(a), Ok(b)) => a != b,
                _ => config.path != path,
            }
        })
        .cloned()
        .collect();

    let legacy_codebases: Vec<String> = settings
        .indexed_codebases
        .iter()
        .filter(|p| {
            let p_expanded = expand_home_dir(p);
            match (p_expanded.canonicalize(), expanded_path.canonicalize()) {
                (Ok(a), Ok(b)) => a != b,
                _ => *p != &path,
            }
        })
        .cloned()
        .collect();

    let mut updated_settings = settings.clone();
    updated_settings.codebases = new_codebases;
    updated_settings.indexed_codebases = legacy_codebases;
    state
        .settings_manager
        .update(updated_settings)
        .await
        .map_err(|e| format!("Failed to save settings: {}", e))?;

    let global_index_dir = compute_index_dir(&expanded_path, IndexLocation::Global);
    if global_index_dir.exists() {
        std::fs::remove_dir_all(&global_index_dir)
            .map_err(|e| format!("Failed to delete global index directory: {}", e))?;
        tracing::info!("Deleted global index directory: {:?}", global_index_dir);
    }

    let local_index_dir = compute_index_dir(&expanded_path, IndexLocation::Local);
    if local_index_dir.exists() {
        std::fs::remove_dir_all(&local_index_dir)
            .map_err(|e| format!("Failed to delete local index directory: {}", e))?;
        tracing::info!("Deleted local index directory: {:?}", local_index_dir);
    }

    tracing::info!("Removed codebase: {}", path);
    Ok(())
}
