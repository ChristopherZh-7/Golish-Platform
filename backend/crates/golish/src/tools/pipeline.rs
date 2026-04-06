use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::db::open_db;

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
    #[serde(default = "default_exec_mode")]
    pub exec_mode: String,
    pub x: f64,
    pub y: f64,
}

fn default_exec_mode() -> String { "pipe".to_string() }

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
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}

#[tauri::command]
pub async fn pipeline_list(project_path: Option<String>) -> Result<Vec<Pipeline>, String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        let mut stmt = conn.prepare("SELECT data FROM pipelines ORDER BY updated_at DESC").map_err(|e| e.to_string())?;
        let items: Vec<Pipeline> = stmt
            .query_map([], |row| { let j: String = row.get(0)?; Ok(j) })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .filter_map(|j| serde_json::from_str(&j).ok())
            .collect();
        Ok(items)
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn pipeline_save(pipeline: Pipeline, project_path: Option<String>) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        let id = if pipeline.id.is_empty() { Uuid::new_v4().to_string() } else { pipeline.id.clone() };
        let ts = now_ts();
        let entry = Pipeline {
            id: id.clone(), updated_at: ts,
            created_at: if pipeline.created_at == 0 { ts } else { pipeline.created_at },
            ..pipeline
        };
        let json = serde_json::to_string(&entry).map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR REPLACE INTO pipelines (id, data, updated_at) VALUES (?1,?2,?3)",
            params![id, json, ts],
        ).map_err(|e| e.to_string())?;
        Ok(id)
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn pipeline_delete(id: String, project_path: Option<String>) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        conn.execute("DELETE FROM pipelines WHERE id=?1", params![id]).map_err(|e| e.to_string())?;
        Ok(())
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn pipeline_load(id: String, project_path: Option<String>) -> Result<Pipeline, String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        let json: String = conn.query_row("SELECT data FROM pipelines WHERE id=?1", params![id], |r| r.get(0)).map_err(|e| e.to_string())?;
        serde_json::from_str(&json).map_err(|e| e.to_string())
    }).await.map_err(|e| e.to_string())?
}
