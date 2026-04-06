use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;
use uuid::Uuid;

fn pipelines_dir(project_path: Option<&str>) -> PathBuf {
    if let Some(pp) = project_path {
        if !pp.is_empty() {
            return PathBuf::from(pp).join(".golish").join("pipelines");
        }
    }
    let home = dirs::home_dir().expect("cannot resolve home directory");
    #[cfg(target_os = "macos")]
    let base = home
        .join("Library")
        .join("Application Support")
        .join("golish-platform");
    #[cfg(target_os = "windows")]
    let base = home.join("AppData").join("Local").join("golish-platform");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let base = home.join(".golish-platform");
    base.join("pipelines")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStep {
    pub id: String,
    pub tool_name: String,
    pub tool_id: String,
    pub command_template: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub input_from: Option<String>,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConnection {
    pub from_step: String,
    pub to_step: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pipeline {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub steps: Vec<PipelineStep>,
    pub connections: Vec<PipelineConnection>,
    pub created_at: u64,
    pub updated_at: u64,
}

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[tauri::command]
pub async fn pipeline_list(project_path: Option<String>) -> Result<Vec<Pipeline>, String> {
    let dir = pipelines_dir(project_path.as_deref());
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut items = Vec::new();
    let mut entries = fs::read_dir(&dir).await.map_err(|e| e.to_string())?;
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            if let Ok(data) = fs::read_to_string(&path).await {
                if let Ok(pipeline) = serde_json::from_str::<Pipeline>(&data) {
                    items.push(pipeline);
                }
            }
        }
    }
    items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(items)
}

#[tauri::command]
pub async fn pipeline_save(pipeline: Pipeline, project_path: Option<String>) -> Result<String, String> {
    let dir = pipelines_dir(project_path.as_deref());
    fs::create_dir_all(&dir).await.map_err(|e| e.to_string())?;

    let id = if pipeline.id.is_empty() {
        Uuid::new_v4().to_string()
    } else {
        pipeline.id.clone()
    };

    let ts = now_ts();
    let entry = Pipeline {
        id: id.clone(),
        updated_at: ts,
        created_at: if pipeline.created_at == 0 { ts } else { pipeline.created_at },
        ..pipeline
    };

    let path = dir.join(format!("{}.json", id));
    let json = serde_json::to_string_pretty(&entry).map_err(|e| e.to_string())?;
    fs::write(&path, json).await.map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub async fn pipeline_delete(id: String, project_path: Option<String>) -> Result<(), String> {
    let path = pipelines_dir(project_path.as_deref()).join(format!("{}.json", id));
    if path.exists() {
        fs::remove_file(&path).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn pipeline_load(id: String, project_path: Option<String>) -> Result<Pipeline, String> {
    let path = pipelines_dir(project_path.as_deref()).join(format!("{}.json", id));
    let data = fs::read_to_string(&path).await.map_err(|e| e.to_string())?;
    serde_json::from_str(&data).map_err(|e| e.to_string())
}
