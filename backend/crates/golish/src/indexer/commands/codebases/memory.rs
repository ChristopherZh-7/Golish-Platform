//! Tauri commands: per-codebase memory file (AGENTS.md / CLAUDE.md) management.

use tauri::State;

use super::expand_home_dir;
use crate::state::AppState;

/// Update the memory file setting for a codebase.
#[tauri::command]
pub async fn update_codebase_memory_file(
    path: String,
    memory_file: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    use crate::settings::schema::CodebaseConfig;

    tracing::info!(
        "update_codebase_memory_file called with path: {}, memory_file: {:?}",
        path,
        memory_file
    );

    let expanded_path = expand_home_dir(&path);
    let normalized_path = expanded_path.canonicalize().ok();

    let settings = state.settings_manager.get().await;
    let mut updated_settings = settings.clone();

    let mut found = false;
    for config in &mut updated_settings.codebases {
        let config_expanded = expand_home_dir(&config.path);
        let matches = match (&config_expanded.canonicalize().ok(), &normalized_path) {
            (Some(a), Some(b)) => a == b,
            _ => config.path == path,
        };

        if matches {
            config.memory_file = memory_file.clone();
            found = true;
            break;
        }
    }

    // Migrate from legacy `indexed_codebases` to the richer `codebases` field
    // when the user touches a path that's still in the old format.
    if !found {
        for legacy_path in &settings.indexed_codebases {
            let legacy_expanded = expand_home_dir(legacy_path);
            let matches = match (&legacy_expanded.canonicalize().ok(), &normalized_path) {
                (Some(a), Some(b)) => a == b,
                _ => legacy_path == &path,
            };

            if matches {
                updated_settings.codebases.push(CodebaseConfig {
                    path: legacy_path.clone(),
                    memory_file: memory_file.clone(),
                });
                updated_settings
                    .indexed_codebases
                    .retain(|p| p != legacy_path);
                found = true;
                break;
            }
        }
    }

    if !found {
        return Err(format!("Codebase not found: {}", path));
    }

    state
        .settings_manager
        .update(updated_settings)
        .await
        .map_err(|e| format!("Failed to save settings: {}", e))?;

    tracing::info!("Updated memory_file for {}: {:?}", path, memory_file);
    Ok(())
}

/// Detect memory files at the root of a codebase.
///
/// Returns the detected memory file based on priority: AGENTS.md > CLAUDE.md > None.
#[tauri::command]
pub async fn detect_memory_files(path: String) -> Result<Option<String>, String> {
    let expanded_path = expand_home_dir(&path);

    if !expanded_path.exists() {
        return Err(format!("Path does not exist: {}", path));
    }

    let agents_md = expanded_path.join("AGENTS.md");
    if agents_md.exists() && agents_md.is_file() {
        return Ok(Some("AGENTS.md".to_string()));
    }

    let claude_md = expanded_path.join("CLAUDE.md");
    if claude_md.exists() && claude_md.is_file() {
        return Ok(Some("CLAUDE.md".to_string()));
    }

    Ok(None)
}
