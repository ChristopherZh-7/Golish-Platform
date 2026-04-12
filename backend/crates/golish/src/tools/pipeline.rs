use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStep {
    pub id: String,
    #[serde(default = "default_step_type")]
    pub step_type: String,
    pub tool_name: String,
    #[serde(default)]
    pub tool_id: String,
    #[serde(default)]
    pub command_template: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub params: serde_json::Value,
    #[serde(default)]
    pub input_from: Option<String>,
    #[serde(default = "default_exec_mode")]
    pub exec_mode: String,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
}

fn default_step_type() -> String { "shell_command".to_string() }
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
    #[serde(default)]
    pub is_template: bool,
    #[serde(default)]
    pub workflow_id: Option<String>,
    pub steps: Vec<PipelineStep>,
    pub connections: Vec<PipelineConnection>,
    pub created_at: u64,
    pub updated_at: u64,
}

fn now_ts() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn builtin_templates() -> Vec<Pipeline> {
    vec![recon_basic_template()]
}

fn recon_basic_template() -> Pipeline {
    // Real shell commands with {target} placeholder.
    // Users can freely edit these in the UI.
    let steps: Vec<(&str, &str, &str, &str, Vec<&str>)> = vec![
        ("dns_lookup",        "dig",          "dns_lookup",        "dig",       vec!["+short", "{target}"]),
        ("subdomain_enum",    "subfinder",    "subdomain_enum",    "subfinder", vec!["-d", "{target}", "-silent"]),
        ("http_probe",        "httpx",        "http_probe",        "httpx",     vec!["-u", "{target}", "-sc", "-title", "-tech-detect", "-silent"]),
        ("port_scan",         "nmap",         "port_scan",         "nmap",      vec!["-sV", "--top-ports", "1000", "{target}"]),
        ("tech_fingerprint",  "whatweb",      "tech_fingerprint",  "whatweb",   vec!["{target}", "--color=never"]),
        ("js_harvest",        "js_harvest",   "js_harvest",        "",          vec!["{target}"]),
    ];

    let pipeline_steps: Vec<PipelineStep> = steps
        .iter()
        .enumerate()
        .map(|(i, (id, name, step_type, cmd, args))| PipelineStep {
            id: id.to_string(),
            step_type: step_type.to_string(),
            tool_name: name.to_string(),
            tool_id: String::new(),
            command_template: cmd.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
            params: serde_json::json!({}),
            input_from: None,
            exec_mode: "sequential".to_string(),
            x: (i as f64) * 220.0 + 40.0,
            y: 80.0,
        })
        .collect();

    let connections: Vec<PipelineConnection> = steps
        .windows(2)
        .map(|w| PipelineConnection {
            from_step: w[0].0.to_string(),
            to_step: w[1].0.to_string(),
        })
        .collect();

    Pipeline {
        id: "recon_basic".to_string(),
        name: "Basic Reconnaissance".to_string(),
        description: "DNS, subdomains, HTTP probe, port scan, tech fingerprint, JS collection. Use {target} as placeholder.".to_string(),
        is_template: false,
        workflow_id: Some("recon_basic".to_string()),
        steps: pipeline_steps,
        connections,
        created_at: 0,
        updated_at: 0,
    }
}

#[tauri::command]
pub async fn pipeline_list(
    state: tauri::State<'_, AppState>,
    project_path: Option<String>,
) -> Result<Vec<Pipeline>, String> {
    let pool = &*state.db_pool;
    let rows: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT data FROM pipelines WHERE project_path IS NOT DISTINCT FROM $1 ORDER BY updated_at DESC",
    )
    .bind(project_path.as_deref())
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let items: Vec<Pipeline> = rows
        .into_iter()
        .filter_map(|j| serde_json::from_value(j).ok())
        .collect();

    // Only include built-in defaults that haven't been saved/customized by the user yet
    let saved_workflow_ids: std::collections::HashSet<&str> = items
        .iter()
        .filter_map(|p| p.workflow_id.as_deref())
        .collect();

    let mut result: Vec<Pipeline> = builtin_templates()
        .into_iter()
        .filter(|t| {
            t.workflow_id.as_deref().map_or(true, |wid| !saved_workflow_ids.contains(wid))
        })
        .collect();
    result.extend(items);
    Ok(result)
}

#[tauri::command]
pub async fn pipeline_save(
    state: tauri::State<'_, AppState>,
    pipeline: Pipeline,
    project_path: Option<String>,
) -> Result<String, String> {
    let pool = &*state.db_pool;
    // Generate a new UUID for empty ids or non-UUID ids (built-in defaults)
    let id = if pipeline.id.is_empty() || pipeline.id.parse::<Uuid>().is_err() {
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
    let json = serde_json::to_value(&entry).map_err(|e| e.to_string())?;
    let uid: Uuid = id.parse().unwrap();
    sqlx::query(
        r#"INSERT INTO pipelines (id, data, project_path)
           VALUES ($1, $2, $3)
           ON CONFLICT (id) DO UPDATE SET data = $2, updated_at = NOW()"#,
    )
    .bind(uid)
    .bind(&json)
    .bind(project_path.as_deref())
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub async fn pipeline_delete(
    state: tauri::State<'_, AppState>,
    id: String,
    project_path: Option<String>,
) -> Result<(), String> {
    let pool = &*state.db_pool;
    let _ = project_path;
    let Ok(uid) = id.parse::<Uuid>() else {
        // Non-UUID ids are built-in defaults (not stored in DB), nothing to delete
        return Ok(());
    };
    sqlx::query("DELETE FROM pipelines WHERE id=$1")
        .bind(uid)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn pipeline_load(
    state: tauri::State<'_, AppState>,
    id: String,
    project_path: Option<String>,
) -> Result<Pipeline, String> {
    let pool = &*state.db_pool;
    let _ = project_path;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let data: serde_json::Value = sqlx::query_scalar("SELECT data FROM pipelines WHERE id=$1")
        .bind(uid)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::from_value(data).map_err(|e| e.to_string())
}
