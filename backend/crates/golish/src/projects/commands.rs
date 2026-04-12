//! Tauri commands for project configuration management.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::{
    delete_project as storage_delete, list_projects as storage_list, load_project as storage_load,
    save_project as storage_save, ProjectConfig,
};
use super::storage::{load_workspace, save_workspace};
use crate::state::AppState;

/// Project form data from the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectFormData {
    pub name: String,
    pub root_path: String,
}

/// Project data returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectData {
    pub name: String,
    pub root_path: String,
}

impl From<ProjectConfig> for ProjectData {
    fn from(config: ProjectConfig) -> Self {
        Self {
            name: config.name,
            root_path: config.root_path.to_string_lossy().to_string(),
        }
    }
}

impl From<ProjectFormData> for ProjectConfig {
    fn from(form: ProjectFormData) -> Self {
        ProjectConfig {
            name: form.name,
            root_path: PathBuf::from(form.root_path),
        }
    }
}

/// Save a new or updated project configuration.
#[tauri::command]
pub async fn save_project(form: ProjectFormData) -> Result<(), String> {
    let config: ProjectConfig = form.into();

    storage_save(&config)
        .await
        .map_err(|e| format!("Failed to save project: {}", e))
}

/// Delete a project configuration by name, including associated DB records.
#[tauri::command]
pub async fn delete_project_config(
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<bool, String> {
    let project_path = storage_load(&name)
        .await
        .ok()
        .flatten()
        .map(|c| c.root_path.to_string_lossy().to_string());

    if let Some(ref path) = project_path {
        let pool = &*state.db_pool;
        let tables_with_project_path = [
            "memories",
            "tool_calls",
            "audit_log",
            "targets",
            "findings",
            "notes",
            "vault_entries",
            "topology_scans",
            "methodology_projects",
            "pipelines",
        ];
        for table in &tables_with_project_path {
            if let Err(e) = sqlx::query(&format!(
                "DELETE FROM {} WHERE project_path = $1",
                table
            ))
            .bind(path)
            .execute(pool)
            .await
            {
                tracing::warn!(
                    "[delete-project] Failed to clean {}: {}",
                    table,
                    e
                );
            }
        }
        tracing::info!(
            "[delete-project] Cleaned DB records for project_path={}",
            path
        );
    }

    storage_delete(&name)
        .await
        .map_err(|e| format!("Failed to delete project: {}", e))
}

/// List all saved project configurations.
#[tauri::command]
pub async fn list_project_configs() -> Result<Vec<ProjectData>, String> {
    let projects = storage_list()
        .await
        .map_err(|e| format!("Failed to list projects: {}", e))?;

    Ok(projects.into_iter().map(ProjectData::from).collect())
}

/// Get a single project configuration by name.
#[tauri::command]
pub async fn get_project_config(name: String) -> Result<Option<ProjectData>, String> {
    let project = storage_load(&name)
        .await
        .map_err(|e| format!("Failed to load project: {}", e))?;

    Ok(project.map(ProjectData::from))
}

/// Save workspace state (conversations, chat history) for a project.
#[tauri::command]
pub async fn save_project_workspace(project_name: String, state_json: String) -> Result<(), String> {
    save_workspace(&project_name, &state_json)
        .await
        .map_err(|e| format!("Failed to save workspace: {}", e))
}

/// Load workspace state for a project. Returns None if no saved state exists.
#[tauri::command]
pub async fn load_project_workspace(project_name: String) -> Result<Option<String>, String> {
    load_workspace(&project_name)
        .await
        .map_err(|e| format!("Failed to load workspace: {}", e))
}
