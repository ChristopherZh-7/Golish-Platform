//! Tauri commands for project configuration management.

use crate::error::GolishError;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::state::DbState;
use golish_projects::file_storage;
use golish_projects::{
    delete_project as storage_delete, list_projects as storage_list, load_project as storage_load,
    load_workspace, save_project as storage_save, save_workspace, PentestProjectConfig,
    ProjectConfig,
};

/// Project form data from the frontend.
///
/// Schema E (2026-05-17) removed the `mode` field — every project relies on
/// the implicit-organization model and the "pentest vs redteam" distinction
/// is derived from the org-tree shape at the UI layer.
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
///
/// Side-effect (Schema E): on a **first save** for a given project name we
/// also seed an implicit root organization in the DB so that targets attached
/// to this project always have a parent org to bind to. We name the implicit
/// org after the project itself — the user can rename it later from the
/// OrganizationsPanel. Subsequent saves are idempotent: if the project
/// already has at least one root org, nothing extra happens.
#[tauri::command]
pub async fn save_project(
    state: tauri::State<'_, DbState>,
    form: ProjectFormData,
) -> Result<(), GolishError> {
    let config: ProjectConfig = form.into();
    let project_path = config.root_path.to_string_lossy().to_string();
    let project_name = config.name.clone();

    let already_existed = storage_load(&config.name)
        .await
        .map_err(|e| GolishError::Internal(format!("Failed to load project: {e}")))?
        .is_some();

    storage_save(&config)
        .await
        .map_err(|e| GolishError::Internal(format!("Failed to save project: {}", e)))?;

    // Only seed the implicit root org on first creation — keeps subsequent
    // saves cheap and avoids racing with whatever the OrganizationsPanel may
    // have done already.
    if !already_existed {
        if let Err(e) = ensure_root_org(&state, &project_path, &project_name).await {
            tracing::warn!(
                "[save_project] Failed to seed implicit root org for {}: {}",
                project_name,
                e
            );
        }
    }

    Ok(())
}

/// Insert a root organization for `project_path` named after the project,
/// unless one already exists. Idempotent.
async fn ensure_root_org(
    state: &tauri::State<'_, DbState>,
    project_path: &str,
    project_name: &str,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    let existing: Option<uuid::Uuid> = sqlx::query_scalar(
        r#"SELECT id FROM organizations
           WHERE project_path = $1 AND parent_id IS NULL
           LIMIT 1"#,
    )
    .bind(project_path)
    .fetch_optional(pool)
    .await?;

    if existing.is_some() {
        return Ok(());
    }

    sqlx::query(
        r#"INSERT INTO organizations (project_path, name, parent_id)
           VALUES ($1, $2, NULL)
           ON CONFLICT DO NOTHING"#,
    )
    .bind(project_path)
    .bind(project_name)
    .execute(pool)
    .await?;

    Ok(())
}

/// Delete a project configuration by name, including associated DB records.
#[tauri::command]
pub async fn delete_project_config(
    state: tauri::State<'_, DbState>,
    name: String,
) -> Result<bool, GolishError> {
    let project_path = storage_load(&name)
        .await
        .ok()
        .flatten()
        .map(|c| c.root_path.to_string_lossy().to_string());

    if let Some(ref path) = project_path {
        let pool = state.pool();
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
            "workspace_preferences",
            "sessions",
            "target_groups",
            "recordings",
            "execution_plans",
            "scan_queue",
            "custom_passive_rules",
            "msg_logs",
            "screenshots",
            "vector_store_logs",
            "prompt_templates",
        ];
        let mut total_deleted = 0u64;
        for table in &tables_with_project_path {
            match sqlx::query(&format!("DELETE FROM {} WHERE project_path = $1", table))
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
            total_deleted,
            path
        );
    }

    storage_delete(&name)
        .await
        .map_err(|e| GolishError::Internal(format!("Failed to delete project: {}", e)))
}

/// List all saved project configurations.
#[tauri::command]
pub async fn list_project_configs() -> Result<Vec<ProjectData>, GolishError> {
    let projects = storage_list()
        .await
        .map_err(|e| GolishError::Internal(format!("Failed to list projects: {}", e)))?;

    Ok(projects.into_iter().map(ProjectData::from).collect())
}

/// Get a single project configuration by name.
#[tauri::command]
pub async fn get_project_config(name: String) -> Result<Option<ProjectData>, GolishError> {
    let project = storage_load(&name)
        .await
        .map_err(|e| GolishError::Internal(format!("Failed to load project: {}", e)))?;

    Ok(project.map(ProjectData::from))
}

/// Save workspace state (conversations, chat history) for a project.
#[tauri::command]
pub async fn save_project_workspace(
    project_name: String,
    state_json: String,
) -> Result<(), GolishError> {
    save_workspace(&project_name, &state_json)
        .await
        .map_err(|e| GolishError::Internal(format!("Failed to save workspace: {}", e)))
}

/// Load workspace state for a project. Returns None if no saved state exists.
#[tauri::command]
pub async fn load_project_workspace(project_name: String) -> Result<Option<String>, GolishError> {
    load_workspace(&project_name)
        .await
        .map_err(|e| GolishError::Internal(format!("Failed to load workspace: {}", e)))
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
pub async fn get_pentest_config(
    project_name: String,
) -> Result<Option<PentestProjectConfig>, GolishError> {
    let project = storage_load(&project_name)
        .await?
        .ok_or_else(|| format!("Project '{}' not found", project_name))?;

    file_storage::load_project_json(&project.root_path)
        .await
        .map_err(GolishError::from)
}

/// Save the pentest project config (project.json) for a project.
#[tauri::command]
pub async fn save_pentest_config(
    project_name: String,
    config: PentestProjectConfig,
) -> Result<(), GolishError> {
    let project = storage_load(&project_name)
        .await?
        .ok_or_else(|| format!("Project '{}' not found", project_name))?;

    file_storage::save_project_json(&project.root_path, &config)
        .await
        .map_err(GolishError::from)
}

/// List all captured hosts and their ports.
#[tauri::command]
pub async fn list_captures(project_name: String) -> Result<CaptureOverview, GolishError> {
    let project = storage_load(&project_name)
        .await?
        .ok_or_else(|| format!("Project '{}' not found", project_name))?;

    let hosts = file_storage::list_capture_hosts(&project.root_path).await?;

    let mut host_captures = Vec::new();
    for host in hosts {
        let ports = file_storage::list_capture_ports(&project.root_path, &host).await?;
        host_captures.push(HostCaptures { host, ports });
    }

    let tool_outputs = file_storage::list_tool_outputs(&project.root_path).await?;

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
) -> Result<Vec<String>, GolishError> {
    let project = storage_load(&project_name)
        .await?
        .ok_or_else(|| format!("Project '{}' not found", project_name))?;

    file_storage::list_capture_files(&project.root_path, &host, port, &file_type)
        .await
        .map_err(GolishError::from)
}

/// Read a file by relative path from the project root.
#[tauri::command]
pub async fn read_project_file(
    project_name: String,
    rel_path: String,
) -> Result<String, GolishError> {
    let project = storage_load(&project_name)
        .await?
        .ok_or_else(|| format!("Project '{}' not found", project_name))?;

    let content = file_storage::read_file(&project.root_path, &rel_path).await?;

    String::from_utf8(content)
        .map_err(|e| GolishError::Internal(format!("File is not valid UTF-8: {}", e)))
}

/// Initialize project directory structure (idempotent).
#[tauri::command]
pub async fn init_project_structure(project_name: String) -> Result<(), GolishError> {
    let project = storage_load(&project_name)
        .await?
        .ok_or_else(|| format!("Project '{}' not found", project_name))?;

    file_storage::init_project_dirs(&project.root_path).await?;

    file_storage::init_project_json(&project.root_path, &project.name).await?;

    Ok(())
}

/// Clean temporary files.
#[tauri::command]
pub async fn clean_project_temp(project_name: String) -> Result<u64, GolishError> {
    let project = storage_load(&project_name)
        .await?
        .ok_or_else(|| format!("Project '{}' not found", project_name))?;

    file_storage::clean_temp(&project.root_path)
        .await
        .map_err(GolishError::from)
}
