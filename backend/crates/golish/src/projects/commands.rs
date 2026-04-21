//! Tauri commands for project configuration management.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::{
    delete_project as storage_delete, list_projects as storage_list, load_project as storage_load,
    save_project as storage_save, ProjectConfig,
};
use super::file_storage::{self, PentestProjectConfig};
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
            "audit_log",
            "targets",
            "findings",
            "notes",
            "vault_entries",
            "sitemap_store",
            "methodology_projects",
            "pipelines",
            "api_endpoints",
            "fingerprints",
            "js_analysis_results",
            "agent_logs",
            "terminal_logs",
            "search_logs",
            "passive_scan_logs",
            "sensitive_scan_results",
            "sensitive_scan_history",
            "directory_entries",
            "target_assets",
            "conversations",
            "topology_scans",
            "workspace_preferences",
        ];
        let mut total_deleted = 0u64;
        for table in &tables_with_project_path {
            match sqlx::query(&format!(
                "DELETE FROM {} WHERE project_path = $1 OR project_path = ''",
                table
            ))
            .bind(path)
            .execute(pool)
            .await
            {
                Ok(r) => total_deleted += r.rows_affected(),
                Err(e) => {
                    tracing::warn!("[delete-project] Failed to clean {}: {}", table, e);
                }
            }
        }
        tracing::info!(
            "[delete-project] Cleaned {} DB records for project_path={}",
            total_deleted, path
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

// ============================================================================
// Pentest project config & file storage commands
// ============================================================================

/// Capture file overview returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureOverview {
    pub hosts: Vec<HostCaptures>,
    pub tool_outputs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostCaptures {
    pub host: String,
    pub ports: Vec<u16>,
}

/// Load the pentest project config (project.json) for a project.
#[tauri::command]
pub async fn get_pentest_config(project_name: String) -> Result<Option<PentestProjectConfig>, String> {
    let project = storage_load(&project_name)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Project '{}' not found", project_name))?;

    file_storage::load_project_json(&project.root_path)
        .await
        .map_err(|e| e.to_string())
}

/// Save the pentest project config (project.json) for a project.
#[tauri::command]
pub async fn save_pentest_config(
    project_name: String,
    config: PentestProjectConfig,
) -> Result<(), String> {
    let project = storage_load(&project_name)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Project '{}' not found", project_name))?;

    file_storage::save_project_json(&project.root_path, &config)
        .await
        .map_err(|e| e.to_string())
}

/// List all captured hosts and their ports.
#[tauri::command]
pub async fn list_captures(project_name: String) -> Result<CaptureOverview, String> {
    let project = storage_load(&project_name)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Project '{}' not found", project_name))?;

    let hosts = file_storage::list_capture_hosts(&project.root_path)
        .await
        .map_err(|e| e.to_string())?;

    let mut host_captures = Vec::new();
    for host in hosts {
        let ports = file_storage::list_capture_ports(&project.root_path, &host)
            .await
            .map_err(|e| e.to_string())?;
        host_captures.push(HostCaptures {
            host,
            ports,
        });
    }

    let tool_outputs = file_storage::list_tool_outputs(&project.root_path)
        .await
        .map_err(|e| e.to_string())?;

    Ok(CaptureOverview {
        hosts: host_captures,
        tool_outputs,
    })
}

/// List files in a specific capture type for a host:port.
#[tauri::command]
pub async fn list_capture_files(
    project_name: String,
    host: String,
    port: u16,
    file_type: String,
) -> Result<Vec<String>, String> {
    let project = storage_load(&project_name)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Project '{}' not found", project_name))?;

    file_storage::list_capture_files(&project.root_path, &host, port, &file_type)
        .await
        .map_err(|e| e.to_string())
}

/// Read a file by relative path from the project root.
#[tauri::command]
pub async fn read_project_file(
    project_name: String,
    rel_path: String,
) -> Result<String, String> {
    let project = storage_load(&project_name)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Project '{}' not found", project_name))?;

    let content = file_storage::read_file(&project.root_path, &rel_path)
        .await
        .map_err(|e| e.to_string())?;

    String::from_utf8(content).map_err(|e| format!("File is not valid UTF-8: {}", e))
}

/// Initialize project directory structure (idempotent).
#[tauri::command]
pub async fn init_project_structure(project_name: String) -> Result<(), String> {
    let project = storage_load(&project_name)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Project '{}' not found", project_name))?;

    file_storage::init_project_dirs(&project.root_path)
        .await
        .map_err(|e| e.to_string())?;

    file_storage::init_project_json(&project.root_path, &project.name)
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}

/// Clean temporary files.
#[tauri::command]
pub async fn clean_project_temp(project_name: String) -> Result<u64, String> {
    let project = storage_load(&project_name)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Project '{}' not found", project_name))?;

    file_storage::clean_temp(&project.root_path)
        .await
        .map_err(|e| e.to_string())
}
