use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;
use uuid::Uuid;

use crate::state::AppState;

static PIPELINE_CANCELLED: AtomicBool = AtomicBool::new(false);

#[tauri::command]
pub async fn pipeline_cancel() -> Result<(), String> {
    PIPELINE_CANCELLED.store(true, Ordering::SeqCst);
    Ok(())
}

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
    /// Which step's output to use as input (by step id). None = previous step.
    #[serde(default)]
    pub input_from: Option<String>,
    #[serde(default = "default_exec_mode")]
    pub exec_mode: String,
    /// Target type required for this step to run: "domain", "ip", "url", or null (always run)
    #[serde(default)]
    pub requires: Option<String>,
    /// Iterate this step over a target attribute. "ports" = run once per HTTP port.
    #[serde(default)]
    pub iterate_over: Option<String>,
    /// Override the tool's default db_action for this step.
    #[serde(default)]
    pub db_action: Option<String>,
    /// What to do when this step fails: "abort" (default), "skip", "continue"
    #[serde(default = "default_on_failure")]
    pub on_failure: String,
    /// Timeout for this step in seconds. None = no timeout.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    /// Template ID to execute as a nested sub-pipeline (step_type = "sub_pipeline").
    #[serde(default)]
    pub sub_pipeline: Option<String>,
    /// Inline pipeline definition for nesting (alternative to sub_pipeline ID reference).
    #[serde(default)]
    pub inline_pipeline: Option<Box<Pipeline>>,
    /// Step ID whose output lines become iteration targets (step_type = "foreach").
    #[serde(default)]
    pub foreach_source: Option<String>,
    /// Max concurrent iterations for foreach steps. Defaults to 5.
    #[serde(default)]
    pub max_parallel: Option<usize>,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
}

fn default_step_type() -> String { "shell_command".to_string() }
fn default_exec_mode() -> String { "pipe".to_string() }
fn default_on_failure() -> String { "abort".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConnection {
    pub from_step: String,
    pub to_step: String,
    /// Condition expression evaluated against the upstream step's result.
    /// None = always pass. Examples: "exit_ok", "output_contains:80", "output_not_empty".
    #[serde(default)]
    pub condition: Option<String>,
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
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
}

fn now_ts() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// Deserialize a Pipeline from JSON, filling in missing defaults (x/y layout, timestamps).
fn pipeline_from_json(json: &str) -> Option<Pipeline> {
    let mut p: Pipeline = serde_json::from_str(json).ok()?;
    for (i, step) in p.steps.iter_mut().enumerate() {
        if step.x == 0.0 && step.y == 0.0 {
            step.x = (i as f64) * 220.0 + 40.0;
            step.y = 80.0;
        }
    }
    Some(p)
}

/// Embedded built-in templates (compiled into the binary).
fn embedded_templates() -> Vec<Pipeline> {
    const RECON_BASIC: &str = include_str!("templates/recon_basic.json");
    [RECON_BASIC]
        .iter()
        .filter_map(|json| pipeline_from_json(json))
        .collect()
}

/// Load user-created templates from the `flow-templates/` directory in app data.
fn user_templates() -> Vec<Pipeline> {
    let Some(dir) = templates_dir() else {
        return vec![];
    };
    if !dir.exists() {
        return vec![];
    }
    let mut templates = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json") {
                if let Ok(data) = std::fs::read_to_string(&path) {
                    if let Some(mut p) = pipeline_from_json(&data) {
                        p.is_template = true;
                        templates.push(p);
                    }
                }
            }
        }
    }
    templates
}

fn builtin_templates() -> Vec<Pipeline> {
    let mut all = embedded_templates();
    let user = user_templates();
    let user_ids: std::collections::HashSet<&str> =
        user.iter().map(|p| p.id.as_str()).collect();
    all.retain(|p| !user_ids.contains(p.id.as_str()));
    all.extend(user);
    all
}

pub fn get_builtin_recon_basic() -> Pipeline {
    embedded_templates()
        .into_iter()
        .find(|p| p.id == "recon_basic")
        .unwrap_or_else(recon_basic_template)
}

/// Detect target type: "domain", "ip", or "url"
pub fn detect_target_type(target: &str) -> &'static str {
    if target.starts_with("http://") || target.starts_with("https://") {
        return "url";
    }
    // Check if it looks like an IP (v4 only for simplicity)
    if target.split('.').count() == 4
        && target.split('.').all(|s| s.parse::<u8>().is_ok())
    {
        return "ip";
    }
    "domain"
}

/// Topological sort of pipeline steps into execution layers.
/// Steps in the same layer have no dependencies between each other and can run concurrently.
/// Falls back to sequential execution (one step per layer) when connections are empty.
fn topo_layers<'a>(
    steps: &'a [PipelineStep],
    connections: &[PipelineConnection],
) -> Vec<Vec<&'a PipelineStep>> {
    if connections.is_empty() {
        return steps.iter().map(|s| vec![s]).collect();
    }

    let step_ids: std::collections::HashSet<&str> =
        steps.iter().map(|s| s.id.as_str()).collect();

    let mut in_degree: std::collections::HashMap<&str, usize> =
        steps.iter().map(|s| (s.id.as_str(), 0)).collect();

    let mut adj: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();

    for conn in connections {
        if step_ids.contains(conn.from_step.as_str())
            && step_ids.contains(conn.to_step.as_str())
        {
            *in_degree.entry(conn.to_step.as_str()).or_insert(0) += 1;
            adj.entry(conn.from_step.as_str())
                .or_default()
                .push(conn.to_step.as_str());
        }
    }

    let step_map: std::collections::HashMap<&str, &PipelineStep> =
        steps.iter().map(|s| (s.id.as_str(), s)).collect();

    let mut layers: Vec<Vec<&PipelineStep>> = Vec::new();
    let mut visited = std::collections::HashSet::new();

    let mut current: Vec<&str> = in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(&id, _)| id)
        .collect();
    current.sort();

    while !current.is_empty() {
        let layer: Vec<&PipelineStep> = current
            .iter()
            .filter_map(|id| step_map.get(id).copied())
            .collect();

        for &id in &current {
            visited.insert(id);
        }

        if !layer.is_empty() {
            layers.push(layer);
        }

        let mut next = Vec::new();
        for &id in &current {
            if let Some(neighbors) = adj.get(id) {
                for &nid in neighbors {
                    if let Some(deg) = in_degree.get_mut(nid) {
                        *deg = deg.saturating_sub(1);
                        if *deg == 0 && !visited.contains(nid) {
                            next.push(nid);
                        }
                    }
                }
            }
        }
        next.sort();
        next.dedup();
        current = next;
    }

    let remaining: Vec<&PipelineStep> = steps
        .iter()
        .filter(|s| !visited.contains(s.id.as_str()))
        .collect();
    if !remaining.is_empty() {
        layers.push(remaining);
    }

    layers
}

/// Resolve the input file for a step, using explicit `input_from`, upstream connections,
/// or falling back to the target seed when the command uses `{prev_output}`.
fn resolve_step_input(
    step: &PipelineStep,
    step_outputs: &std::collections::HashMap<String, std::path::PathBuf>,
    connections: &[PipelineConnection],
    tmp_dir: &std::path::Path,
    target: &str,
) -> Option<std::path::PathBuf> {
    if let Some(ref from_id) = step.input_from {
        if let Some(path) = step_outputs.get(from_id) {
            return Some(path.clone());
        }
    }

    let upstream: Vec<&str> = connections
        .iter()
        .filter(|c| c.to_step == step.id)
        .map(|c| c.from_step.as_str())
        .collect();
    for uid in &upstream {
        if let Some(path) = step_outputs.get(*uid) {
            return Some(path.clone());
        }
    }

    let full_cmd_preview = format!("{} {}", step.command_template, step.args.join(" "));
    if full_cmd_preview.contains("{prev_output}") {
        let seed = tmp_dir.join(format!("seed-{}.txt", step.id));
        let _ = std::fs::write(&seed, target);
        return Some(seed);
    }

    None
}

/// Evaluate a condition expression against an upstream step's result and output file.
/// Returns `true` if the condition passes (step should run), `false` to skip.
fn evaluate_condition(
    condition: &str,
    result: &StepResult,
    output_path: &std::path::Path,
) -> bool {
    match condition {
        "exit_ok" => result.exit_code == Some(0),
        "exit_fail" => result.exit_code.is_some() && result.exit_code != Some(0),
        "output_not_empty" => result.stdout_lines > 0,
        _ if condition.starts_with("output_contains:") => {
            let pattern = &condition["output_contains:".len()..];
            std::fs::read_to_string(output_path)
                .map(|s| s.contains(pattern))
                .unwrap_or(false)
        }
        _ if condition.starts_with("output_lines_gt:") => {
            let n: usize = condition["output_lines_gt:".len()..].parse().unwrap_or(0);
            result.stdout_lines > n
        }
        _ if condition.starts_with("stored_gt:") => {
            let n: usize = condition["stored_gt:".len()..].parse().unwrap_or(0);
            result
                .store_stats
                .as_ref()
                .map(|s| s.stored_count > n)
                .unwrap_or(false)
        }
        other => {
            tracing::warn!("[pipeline] Unknown condition '{}', treating as pass", other);
            true
        }
    }
}

/// Resolve per-port target URLs from the `targets.ports` JSONB column.
/// Returns a vec of URLs like `http://8.138.179.62:8080`, `https://8.138.179.62:443`.
async fn resolve_port_targets(
    pool: &sqlx::PgPool,
    target: &str,
    project_path: Option<&str>,
) -> Vec<String> {
    let ports_json: Option<serde_json::Value> = sqlx::query_scalar(
        r#"SELECT ports FROM targets
           WHERE value = $1 AND project_path = $2
           LIMIT 1"#,
    )
    .bind(target)
    .bind(project_path)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let Some(serde_json::Value::Array(ports)) = ports_json else {
        tracing::info!(target = %target, "[resolve_port_targets] No ports column or not an array, falling back to default");
        return vec![format!("http://{}", target)];
    };

    if ports.is_empty() {
        tracing::info!(target = %target, "[resolve_port_targets] Empty ports array, falling back to default");
        return vec![format!("http://{}", target)];
    }

    tracing::info!(target = %target, port_count = ports.len(), "[resolve_port_targets] Found ports in DB");

    let urls: Vec<String> = ports
        .iter()
        .filter_map(|entry| {
            let port = entry.get("port")?.as_u64()? as u16;
            let service = entry
                .get("service")
                .and_then(|s| s.as_str())
                .unwrap_or("http");
            let scheme = if service == "https" || port == 443 {
                "https"
            } else {
                "http"
            };
            let url = if (scheme == "http" && port == 80) || (scheme == "https" && port == 443) {
                format!("{}://{}", scheme, target)
            } else {
                format!("{}://{}:{}", scheme, target, port)
            };
            Some(url)
        })
        .collect();

    tracing::info!(
        target = %target,
        resolved_count = urls.len(),
        urls = ?urls,
        "[resolve_port_targets] Resolved URLs for iteration"
    );
    urls
}

fn recon_basic_template() -> Pipeline {
    // step: (id, name, step_type, cmd, args, input_from, requires)
    struct StepDef {
        id: &'static str,
        name: &'static str,
        step_type: &'static str,
        cmd: &'static str,
        args: Vec<&'static str>,
        input_from: Option<&'static str>,
        requires: Option<&'static str>,
        iterate_over: Option<&'static str>,
        db_action: Option<&'static str>,
    }

    let steps = vec![
        StepDef {
            id: "dns_lookup", name: "dig", step_type: "dns_lookup",
            cmd: "dig", args: vec!["+short", "{target}"],
            input_from: None, requires: Some("domain"),
            iterate_over: None, db_action: None,
        },
        StepDef {
            id: "subdomain_enum", name: "subfinder", step_type: "subdomain_enum",
            cmd: "subfinder", args: vec!["-d", "{target}", "-silent"],
            input_from: None, requires: Some("domain"),
            iterate_over: None, db_action: None,
        },
        StepDef {
            id: "port_scan", name: "naabu", step_type: "port_scan",
            cmd: "naabu", args: vec!["-host", "{target}", "-top-ports", "1000", "-json", "-silent"],
            input_from: None, requires: None,
            iterate_over: None, db_action: None,
        },
        StepDef {
            id: "http_probe", name: "httpx", step_type: "http_probe",
            cmd: "httpx", args: vec!["-u", "{target}", "-sc", "-title", "-tech-detect", "-json", "-silent"],
            input_from: None, requires: None,
            iterate_over: Some("ports"), db_action: None,
        },
        StepDef {
            id: "tech_fingerprint", name: "whatweb", step_type: "tech_fingerprint",
            cmd: "whatweb", args: vec!["{target}", "--color=never"],
            input_from: None, requires: None,
            iterate_over: Some("ports"), db_action: None,
        },
        StepDef {
            id: "web_crawl", name: "katana", step_type: "web_crawl",
            cmd: "katana", args: vec!["-u", "{target}", "-d", "3", "-js-crawl", "-silent"],
            input_from: None, requires: None,
            iterate_over: Some("ports"), db_action: Some("target_add"),
        },
    ];

    let pipeline_steps: Vec<PipelineStep> = steps
        .iter()
        .enumerate()
        .map(|(i, s)| PipelineStep {
            id: s.id.to_string(),
            step_type: s.step_type.to_string(),
            tool_name: s.name.to_string(),
            tool_id: String::new(),
            command_template: s.cmd.to_string(),
            args: s.args.iter().map(|a| a.to_string()).collect(),
            params: serde_json::json!({}),
            input_from: s.input_from.map(|v| v.to_string()),
            exec_mode: "sequential".to_string(),
            requires: s.requires.map(|v| v.to_string()),
            iterate_over: s.iterate_over.map(|v| v.to_string()),
            db_action: s.db_action.map(|v| v.to_string()),
            on_failure: "continue".to_string(),
            timeout_secs: None,
            sub_pipeline: None,
            inline_pipeline: None,
            foreach_source: None,
            max_parallel: None,
            x: (i as f64) * 220.0 + 40.0,
            y: 80.0,
        })
        .collect();

    // DAG connections: Layer 0 (dig, subfinder, naabu) → Layer 1 (httpx, whatweb) → Layer 2 (katana)
    let connections: Vec<PipelineConnection> = vec![
        // port_scan feeds into http_probe and tech_fingerprint
        PipelineConnection { from_step: "port_scan".into(), to_step: "http_probe".into(), condition: None },
        PipelineConnection { from_step: "port_scan".into(), to_step: "tech_fingerprint".into(), condition: None },
        PipelineConnection { from_step: "http_probe".into(), to_step: "web_crawl".into(), condition: None },
        PipelineConnection { from_step: "tech_fingerprint".into(), to_step: "web_crawl".into(), condition: None },
    ];

    Pipeline {
        id: "recon_basic".to_string(),
        name: "Basic Reconnaissance".to_string(),
        description: "DNS, subdomains, port scan, HTTP probe, tech fingerprint, web crawl (katana). Use {target} as placeholder.".to_string(),
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
    let pool = state.db_pool_ready().await?;
    let rows: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT data FROM pipelines WHERE project_path = $1 ORDER BY updated_at DESC",
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
    let pool = state.db_pool_ready().await?;
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
    let pool = state.db_pool_ready().await?;
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
    let pool = state.db_pool_ready().await?;
    let _ = project_path;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let data: serde_json::Value = sqlx::query_scalar("SELECT data FROM pipelines WHERE id=$1")
        .bind(uid)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::from_value(data).map_err(|e| e.to_string())
}

// ============================================================================
// Template management: save/list/delete user flow templates (JSON files)
// ============================================================================

fn templates_dir() -> Option<std::path::PathBuf> {
    let home = dirs::home_dir()?;
    #[cfg(target_os = "macos")]
    let base = home.join("Library").join("Application Support").join("golish-platform");
    #[cfg(target_os = "windows")]
    let base = home.join("AppData").join("Local").join("golish-platform");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let base = home.join(".golish-platform");
    Some(base.join("flow-templates"))
}

#[tauri::command]
pub async fn pipeline_list_templates() -> Result<Vec<Pipeline>, String> {
    let mut all = builtin_templates();
    for p in &mut all {
        p.is_template = true;
    }
    Ok(all)
}

/// Save a pipeline as a JSON template file (non-async, for use from AI tools).
pub fn pipeline_save_template_inner(pipeline: &Pipeline) -> Result<String, String> {
    let dir = templates_dir().ok_or("Cannot determine app data directory")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let id = if pipeline.id.is_empty() {
        Uuid::new_v4().to_string()
    } else {
        pipeline.id.clone()
    };
    let ts = now_ts();
    let entry = Pipeline {
        id: id.clone(),
        is_template: true,
        updated_at: ts,
        created_at: if pipeline.created_at == 0 { ts } else { pipeline.created_at },
        ..pipeline.clone()
    };
    let filename = format!("{}.json", entry.name.to_lowercase().replace(' ', "_"));
    let path = dir.join(&filename);
    let json = serde_json::to_string_pretty(&entry).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    tracing::info!("[pipeline] Saved template '{}' to {}", entry.name, path.display());
    Ok(id)
}

#[tauri::command]
pub async fn pipeline_save_template(pipeline: Pipeline) -> Result<String, String> {
    let dir = templates_dir().ok_or("Cannot determine app data directory")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let id = if pipeline.id.is_empty() {
        Uuid::new_v4().to_string()
    } else {
        pipeline.id.clone()
    };
    let ts = now_ts();
    let entry = Pipeline {
        id: id.clone(),
        is_template: true,
        updated_at: ts,
        created_at: if pipeline.created_at == 0 { ts } else { pipeline.created_at },
        ..pipeline
    };

    let filename = format!("{}.json", entry.name.to_lowercase().replace(' ', "_"));
    let path = dir.join(&filename);
    let json = serde_json::to_string_pretty(&entry).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    tracing::info!("[pipeline] Saved template '{}' to {}", entry.name, path.display());
    Ok(id)
}

#[tauri::command]
pub async fn pipeline_delete_template(id: String) -> Result<(), String> {
    let dir = templates_dir().ok_or("Cannot determine app data directory")?;
    if !dir.exists() {
        return Ok(());
    }
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json") {
                if let Ok(data) = std::fs::read_to_string(&path) {
                    if let Ok(p) = serde_json::from_str::<Pipeline>(&data) {
                        if p.id == id {
                            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
                            tracing::info!("[pipeline] Deleted template '{}' at {}", id, path.display());
                            return Ok(());
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

// ============================================================================
// Pipeline executor: run steps with DAG-parallel execution and DB storage
// ============================================================================

use super::output_parser::{OutputParserConfig, PatternConfig, StoreStats};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub step_id: String,
    pub tool_name: String,
    pub command: String,
    pub exit_code: Option<i32>,
    pub stdout_lines: usize,
    pub stderr_preview: String,
    pub store_stats: Option<StoreStats>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineRunResult {
    pub pipeline_name: String,
    pub target: String,
    pub steps: Vec<StepResult>,
    pub total_stored: usize,
    pub total_duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PipelineEvent {
    pipeline_id: String,
    /// Unique identifier for this specific pipeline execution run.
    /// Allows the frontend to isolate events from concurrent runs.
    run_id: String,
    step_id: String,
    step_index: usize,
    total_steps: usize,
    status: String,
    tool_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    store_stats: Option<StoreStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pipeline_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<String>,
    /// All step names/ids emitted with the "started" event so the frontend
    /// can build the full progress block immediately.
    #[serde(skip_serializing_if = "Option::is_none")]
    all_steps: Option<Vec<PipelineStepInfo>>,
    /// Truncated stdout of the completed step (max 4 KB).
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PipelineStepInfo {
    id: String,
    tool_name: String,
    command_template: String,
}

fn app_data_dirs() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let home = dirs::home_dir()?;
    #[cfg(target_os = "macos")]
    let base = home.join("Library").join("Application Support").join("golish-platform");
    #[cfg(target_os = "windows")]
    let base = home.join("AppData").join("Local").join("golish-platform");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let base = home.join(".golish-platform");
    Some((base.join("toolsconfig"), base.join("tools")))
}

fn find_tool_json(tool_name: &str) -> Option<serde_json::Value> {
    let (config_dir, _) = app_data_dirs()?;
    if !config_dir.exists() { return None; }
    let lower = tool_name.to_lowercase();
    for entry in walkdir::WalkDir::new(&config_dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            if let Ok(data) = std::fs::read_to_string(path) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&data) {
                    let name = val.pointer("/tool/name").and_then(|v| v.as_str()).unwrap_or("");
                    if name.to_lowercase() == lower {
                        return Some(val);
                    }
                }
            }
        }
    }
    None
}

fn load_tool_output_config(tool_name: &str) -> Option<OutputParserConfig> {
    let val = find_tool_json(tool_name)?;
    val.pointer("/tool/output").and_then(|o| serde_json::from_value(o.clone()).ok())
}

/// Resolve a bare command name to a full launch command using the unified command builder.
/// Looks up the tool's JSON config, parses it into a ToolConfig, and delegates to
/// `golish_pentest::build_run_command`.
async fn resolve_tool_command(bare_cmd: &str, config_manager: &golish_pentest::ConfigManager) -> String {
    let Some(val) = find_tool_json(bare_cmd) else { return bare_cmd.to_string() };

    let tool_config: golish_pentest::ToolConfig = match serde_json::from_value(val["tool"].clone()) {
        Ok(tc) => tc,
        Err(_) => return bare_cmd.to_string(),
    };

    let config = config_manager.get().await;
    let ctx = golish_pentest::CommandContext {
        tools_dir: config.tools_dir(),
        conda_base: config.conda_path(),
        nvm_path: config.nvm_path(),
    };

    match golish_pentest::build_run_command(&tool_config, "", &ctx).await {
        Ok(result) => result.command,
        Err(e) => {
            tracing::warn!("[pipeline] build_run_command failed for '{}': {e}", bare_cmd);
            bare_cmd.to_string()
        }
    }
}

/// Tauri command wrapper around [`execute_pipeline_headless`] with cancel support.
/// Resets the cancel flag before execution; cancellation is checked between layers.
#[tauri::command]
pub async fn pipeline_execute(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    pipeline: Pipeline,
    target: String,
    project_path: Option<String>,
) -> Result<PipelineRunResult, String> {
    PIPELINE_CANCELLED.store(false, Ordering::SeqCst);

    let pool = state.db_pool_ready().await?;
    let result = execute_pipeline_headless(
        pool,
        &pipeline,
        &target,
        project_path.as_deref(),
        &state.pentest_config_manager,
        Some(&app),
    )
    .await
    .map_err(|e| e.to_string());

    PIPELINE_CANCELLED.store(false, Ordering::SeqCst);
    result
}

// Helper DB storage functions for pipeline executor
use super::output_parser::ParsedItem;

/// Returns `Ok(true)` when a brand-new target was created, `Ok(false)` when it already existed.
async fn store_target_from_item(
    pool: &sqlx::PgPool,
    item: &ParsedItem,
    project_path: Option<&str>,
    parent_id: Option<Uuid>,
) -> Result<bool, String> {
    let hostname = if let Some(h) = item.fields.get("hostname")
        .or_else(|| item.fields.get("host"))
        .or_else(|| item.fields.get("ip"))
    {
        h.clone()
    } else if let Some(url) = item.fields.get("url") {
        extract_hostname(url)
    } else {
        return Err("No hostname/host/ip/url field".to_string());
    };

    let existed = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM targets WHERE value = $1 AND project_path = $2)",
    )
    .bind(&hostname)
    .bind(project_path)
    .fetch_one(pool)
    .await
    .unwrap_or(false);

    super::targets::db_target_add(pool, &hostname, &hostname, None, project_path, "discovered", parent_id)
        .await?;
    Ok(!existed)
}

/// Returns `Ok(true)` if a port that didn't previously exist was added.
async fn store_recon_from_item(
    pool: &sqlx::PgPool,
    item: &ParsedItem,
    project_path: Option<&str>,
) -> Result<bool, String> {
    let host_val = item
        .fields
        .get("host")
        .or_else(|| item.fields.get("ip"))
        .or_else(|| item.fields.get("url"))
        .ok_or("No host/ip field")?;

    // Find or create target
    let hostname = extract_hostname(host_val);
    let target =
        super::targets::db_target_add(pool, &hostname, &hostname, None, project_path, "discovered", None)
            .await?;
    let target_uuid: Uuid = target.id.parse().map_err(|e: uuid::Error| e.to_string())?;

    let mut update = super::targets::ReconUpdate::new();
    if let Some(ip) = item.fields.get("ip") {
        update.real_ip = ip.clone();
    }
    if let Some(cdn) = item.fields.get("cdn") {
        update.cdn_waf = cdn.clone();
    }
    if let Some(os) = item.fields.get("os") {
        update.os_info = os.clone();
    }

    // Build port entry with embedded HTTP metadata when available.
    // This stores per-port service info (title, status, server, tech)
    // inside the port JSONB entry rather than at the target level.
    if let Some(port_str) = item.fields.get("port") {
        let mut port_entry = serde_json::json!({
            "port": port_str.parse::<u16>().unwrap_or(0),
            "proto": item.fields.get("protocol").cloned().unwrap_or_else(|| "tcp".to_string()),
            "service": item.fields.get("service").cloned().unwrap_or_default(),
            "state": item.fields.get("state").cloned().unwrap_or_else(|| "open".to_string()),
        });
        if let Some(title) = item.fields.get("title") {
            port_entry["http_title"] = serde_json::Value::String(title.clone());
        }
        if let Some(status) = item.fields.get("status_code").or_else(|| item.fields.get("status")) {
            if let Ok(code) = status.parse::<i32>() {
                port_entry["http_status"] = serde_json::json!(code);
                port_entry["service"] = serde_json::Value::String("http".to_string());
            }
        }
        if let Some(ws) = item.fields.get("webserver") {
            port_entry["webserver"] = serde_json::Value::String(ws.clone());
        }
        if let Some(ct) = item.fields.get("content_type") {
            port_entry["content_type"] = serde_json::Value::String(ct.clone());
        }
        if let Some(techs) = item.fields.get("technologies") {
            if let Ok(arr) = serde_json::from_str::<serde_json::Value>(techs) {
                port_entry["technologies"] = arr;
            } else {
                let tech_list: Vec<&str> = techs.split(',').map(|s| s.trim()).collect();
                port_entry["technologies"] = serde_json::to_value(tech_list).unwrap_or_default();
            }
        }
        if let Some(url) = item.fields.get("url") {
            port_entry["url"] = serde_json::Value::String(url.clone());
        }
        update.ports = serde_json::json!([port_entry]);
    }

    // Check if the specific port already exists on this target
    let is_new_port = if let Some(port_str) = item.fields.get("port") {
        let port_num: i32 = port_str.parse().unwrap_or(0);
        let proto = item.fields.get("protocol").cloned().unwrap_or_else(|| "tcp".to_string());
        !sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM targets WHERE id = $1 AND ports @> $2::jsonb)",
        )
        .bind(target_uuid)
        .bind(serde_json::json!([{"port": port_num, "proto": proto}]))
        .fetch_one(pool)
        .await
        .unwrap_or(false)
    } else {
        false
    };

    // Still set target-level fields for backward compatibility and summary display.
    if let Some(title) = item.fields.get("title") {
        update.http_title = title.clone();
    }
    if let Some(status) = item.fields.get("status_code").or_else(|| item.fields.get("status")) {
        update.http_status = status.parse().ok();
    }
    if let Some(ws) = item.fields.get("webserver") {
        update.webserver = ws.clone();
    }

    super::targets::db_target_update_recon_extended(pool, target_uuid, &update).await?;

    let tool_source = item.fields.get("_tool").map(|s| s.as_str()).unwrap_or("httpx");
    super::output_parser::store_recon_fingerprints(pool, target_uuid, project_path, item, tool_source).await;

    Ok(is_new_port)
}

/// Returns `Ok(true)` if this is a new directory entry.
async fn store_dirent_from_item(
    pool: &sqlx::PgPool,
    item: &ParsedItem,
    tool_name: &str,
    project_path: Option<&str>,
) -> Result<bool, String> {
    let url = item.fields.get("url").ok_or("No url field")?;

    let existed = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM directory_entries WHERE url = $1 AND project_path = $2)",
    )
    .bind(url)
    .bind(project_path)
    .fetch_one(pool)
    .await
    .unwrap_or(false);

    let status: Option<i32> = item.fields.get("status").and_then(|s| s.parse().ok());
    let size: Option<i32> = item
        .fields
        .get("size")
        .or_else(|| item.fields.get("content_length"))
        .and_then(|s| s.parse().ok());
    let lines: Option<i32> = item.fields.get("lines").and_then(|s| s.parse().ok());
    let words: Option<i32> = item.fields.get("words").and_then(|s| s.parse().ok());

    super::targets::db_directory_entry_add(
        pool,
        None,
        url,
        status,
        size,
        lines,
        words,
        tool_name,
        project_path,
    )
    .await?;
    Ok(!existed)
}

/// Returns `Ok(true)` if this is a new finding (not a duplicate).
async fn store_finding_from_item(
    pool: &sqlx::PgPool,
    item: &ParsedItem,
    tool_name: &str,
    project_path: Option<&str>,
) -> Result<bool, String> {
    let title = item
        .fields
        .get("title")
        .cloned()
        .unwrap_or_else(|| "Untitled Finding".to_string());
    let severity = item
        .fields
        .get("severity")
        .cloned()
        .unwrap_or_else(|| "info".to_string());
    let url = item.fields.get("url").cloned().unwrap_or_default();
    let template = item.fields.get("template").cloned().unwrap_or_default();
    let description = item.fields.get("description").cloned().unwrap_or_default();

    let sev = match severity.to_lowercase().as_str() {
        "critical" => "critical",
        "high" => "high",
        "medium" => "medium",
        "low" => "low",
        _ => "info",
    };

    let result = sqlx::query(
        r#"INSERT INTO findings (title, sev, url, target, description, tool, template, project_path)
           VALUES ($1, $2::severity, $3, $4, $5, $6, $7, $8)
           ON CONFLICT DO NOTHING"#,
    )
    .bind(&title)
    .bind(sev)
    .bind(&url)
    .bind(&url)
    .bind(&description)
    .bind(tool_name)
    .bind(&template)
    .bind(project_path)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(result.rows_affected() > 0)
}

/// Emit a pipeline event to the frontend if an AppHandle is available.
fn emit_pipeline_event(app: Option<&tauri::AppHandle>, event: &PipelineEvent) {
    if let Some(app) = app {
        tracing::info!(
            "[pipeline-event] Emitting: status={}, step={}, pipeline={}",
            event.status, event.tool_name, event.pipeline_id
        );
        let _ = app.emit("pipeline-event", event);
    } else {
        tracing::warn!("[pipeline-event] No AppHandle available, skipping event emission");
    }
}

/// Result of executing a single pipeline step.
struct SingleStepResult {
    step_result: StepResult,
    output_path: std::path::PathBuf,
    stored_count: usize,
}

const MAX_NESTING_DEPTH: usize = 5;

/// Resolve a sub-pipeline by template ID or inline definition.
fn resolve_sub_pipeline(step: &PipelineStep) -> Option<Pipeline> {
    if let Some(ref inline) = step.inline_pipeline {
        return Some(*inline.clone());
    }
    if let Some(ref template_id) = step.sub_pipeline {
        let all = builtin_templates();
        if let Some(p) = all.into_iter().find(|p| p.id == *template_id || p.name == *template_id) {
            return Some(p);
        }
    }
    None
}

/// Execute a sub-pipeline step by recursively calling `execute_pipeline_headless_inner`.
#[allow(clippy::too_many_arguments)]
async fn run_sub_pipeline_step<'a>(
    pool: &'a sqlx::PgPool,
    step: &'a PipelineStep,
    step_index: usize,
    total_steps: usize,
    target: &'a str,
    project_path: Option<&'a str>,
    _parent_target_id: Option<Uuid>,
    tmp_dir: &'a std::path::Path,
    config_manager: &'a golish_pentest::ConfigManager,
    pipeline_id: &'a str,
    run_id: &'a str,
    app: Option<&'a tauri::AppHandle>,
    depth: usize,
) -> SingleStepResult {
    let step_start = std::time::Instant::now();

    if depth >= MAX_NESTING_DEPTH {
        let msg = format!("Max nesting depth ({}) exceeded", MAX_NESTING_DEPTH);
        tracing::warn!("[pipeline] {}", msg);
        emit_pipeline_event(app, &PipelineEvent {
            pipeline_id: pipeline_id.to_string(), run_id: run_id.to_string(),
            step_id: step.id.clone(), step_index, total_steps,
            status: "error".to_string(), tool_name: step.tool_name.clone(),
            message: Some(msg.clone()), store_stats: None,
            pipeline_name: None, target: None, all_steps: None,
            output: None, duration_ms: Some(0), exit_code: Some(-3),
        });
        return SingleStepResult {
            step_result: StepResult {
                step_id: step.id.clone(), tool_name: step.tool_name.clone(),
                command: String::new(), exit_code: Some(-3),
                stdout_lines: 0, stderr_preview: msg, store_stats: None, duration_ms: 0,
            },
            output_path: tmp_dir.join(format!("step-{}-{}.txt", step_index, step.tool_name)),
            stored_count: 0,
        };
    }

    let sub = match resolve_sub_pipeline(step) {
        Some(p) => p,
        None => {
            let msg = format!("Sub-pipeline '{}' not found", step.sub_pipeline.as_deref().unwrap_or("?"));
            emit_pipeline_event(app, &PipelineEvent {
                pipeline_id: pipeline_id.to_string(), run_id: run_id.to_string(),
                step_id: step.id.clone(), step_index, total_steps,
                status: "error".to_string(), tool_name: step.tool_name.clone(),
                message: Some(msg.clone()), store_stats: None,
                pipeline_name: None, target: None, all_steps: None,
                output: None, duration_ms: Some(0), exit_code: Some(-4),
            });
            return SingleStepResult {
                step_result: StepResult {
                    step_id: step.id.clone(), tool_name: step.tool_name.clone(),
                    command: String::new(), exit_code: Some(-4),
                    stdout_lines: 0, stderr_preview: msg, store_stats: None, duration_ms: 0,
                },
                output_path: tmp_dir.join(format!("step-{}-{}.txt", step_index, step.tool_name)),
                stored_count: 0,
            };
        }
    };

    tracing::info!(
        "[pipeline] Sub-pipeline '{}' at depth {} for target '{}'",
        sub.name, depth + 1, target
    );

    let sub_result = execute_pipeline_headless_inner(
        pool, &sub, target, project_path, config_manager, app, depth + 1,
    ).await;

    let duration_ms = step_start.elapsed().as_millis() as u64;
    let output_file = tmp_dir.join(format!("step-{}-{}.txt", step_index, step.tool_name));

    match sub_result {
        Ok(result) => {
            let summary: String = result.steps.iter()
                .map(|s| format!("{}: exit={}", s.tool_name, s.exit_code.unwrap_or(-1)))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = std::fs::write(&output_file, &summary);

            let all_ok = result.steps.iter().all(|s| s.exit_code == Some(0) || s.exit_code.is_none());
            emit_pipeline_event(app, &PipelineEvent {
                pipeline_id: pipeline_id.to_string(), run_id: run_id.to_string(),
                step_id: step.id.clone(), step_index, total_steps,
                status: if all_ok { "completed" } else { "error" }.to_string(),
                tool_name: step.tool_name.clone(),
                message: Some(format!("Sub-pipeline '{}': {}", sub.name, summary)),
                store_stats: None, pipeline_name: None, target: None, all_steps: None,
                output: Some(summary.clone()), duration_ms: Some(duration_ms),
                exit_code: if all_ok { Some(0) } else { Some(1) },
            });

            SingleStepResult {
                step_result: StepResult {
                    step_id: step.id.clone(), tool_name: step.tool_name.clone(),
                    command: format!("[sub_pipeline:{}]", sub.name),
                    exit_code: if all_ok { Some(0) } else { Some(1) },
                    stdout_lines: summary.lines().count(),
                    stderr_preview: String::new(),
                    store_stats: None,
                    duration_ms,
                },
                output_path: output_file,
                stored_count: result.total_stored,
            }
        }
        Err(e) => {
            let msg = format!("Sub-pipeline error: {}", e);
            let _ = std::fs::write(&output_file, &msg);
            emit_pipeline_event(app, &PipelineEvent {
                pipeline_id: pipeline_id.to_string(), run_id: run_id.to_string(),
                step_id: step.id.clone(), step_index, total_steps,
                status: "error".to_string(), tool_name: step.tool_name.clone(),
                message: Some(msg.clone()), store_stats: None,
                pipeline_name: None, target: None, all_steps: None,
                output: None, duration_ms: Some(duration_ms), exit_code: Some(-5),
            });
            SingleStepResult {
                step_result: StepResult {
                    step_id: step.id.clone(), tool_name: step.tool_name.clone(),
                    command: format!("[sub_pipeline:{}]", step.sub_pipeline.as_deref().unwrap_or("?")),
                    exit_code: Some(-5), stdout_lines: 0,
                    stderr_preview: msg, store_stats: None, duration_ms,
                },
                output_path: output_file,
                stored_count: 0,
            }
        }
    }
}

/// Execute a foreach step: iterate over output lines from a source step, running either
/// a sub-pipeline or a command for each line as the target.
#[allow(clippy::too_many_arguments)]
async fn run_foreach_step<'a>(
    pool: &'a sqlx::PgPool,
    step: &'a PipelineStep,
    step_index: usize,
    total_steps: usize,
    _target: &'a str,
    project_path: Option<&'a str>,
    _parent_target_id: Option<Uuid>,
    tmp_dir: &'a std::path::Path,
    config_manager: &'a golish_pentest::ConfigManager,
    pipeline_id: &'a str,
    run_id: &'a str,
    app: Option<&'a tauri::AppHandle>,
    step_outputs: &'a std::collections::HashMap<String, std::path::PathBuf>,
    depth: usize,
) -> SingleStepResult {
    let step_start = std::time::Instant::now();
    let output_file = tmp_dir.join(format!("step-{}-{}.txt", step_index, step.tool_name));
    let max_par = step.max_parallel.unwrap_or(5);

    let source_id = match step.foreach_source.as_deref() {
        Some(id) => id,
        None => {
            let msg = "foreach step requires foreach_source".to_string();
            let _ = std::fs::write(&output_file, &msg);
            return SingleStepResult {
                step_result: StepResult {
                    step_id: step.id.clone(), tool_name: step.tool_name.clone(),
                    command: String::new(), exit_code: Some(-6),
                    stdout_lines: 0, stderr_preview: msg, store_stats: None, duration_ms: 0,
                },
                output_path: output_file, stored_count: 0,
            };
        }
    };

    let source_path = match step_outputs.get(source_id) {
        Some(p) => p.clone(),
        None => {
            let msg = format!("foreach source step '{}' has no output", source_id);
            let _ = std::fs::write(&output_file, &msg);
            return SingleStepResult {
                step_result: StepResult {
                    step_id: step.id.clone(), tool_name: step.tool_name.clone(),
                    command: String::new(), exit_code: Some(-6),
                    stdout_lines: 0, stderr_preview: msg, store_stats: None, duration_ms: 0,
                },
                output_path: output_file, stored_count: 0,
            };
        }
    };

    let lines: Vec<String> = std::fs::read_to_string(&source_path)
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    if lines.is_empty() {
        let _ = std::fs::write(&output_file, "No iteration targets");
        let duration_ms = step_start.elapsed().as_millis() as u64;
        emit_pipeline_event(app, &PipelineEvent {
            pipeline_id: pipeline_id.to_string(), run_id: run_id.to_string(),
            step_id: step.id.clone(), step_index, total_steps,
            status: "completed".to_string(), tool_name: step.tool_name.clone(),
            message: Some("foreach: 0 iterations".to_string()), store_stats: None,
            pipeline_name: None, target: None, all_steps: None,
            output: None, duration_ms: Some(duration_ms), exit_code: Some(0),
        });
        return SingleStepResult {
            step_result: StepResult {
                step_id: step.id.clone(), tool_name: step.tool_name.clone(),
                command: format!("[foreach:{}]", source_id), exit_code: Some(0),
                stdout_lines: 0, stderr_preview: String::new(), store_stats: None, duration_ms,
            },
            output_path: output_file, stored_count: 0,
        };
    }

    tracing::info!(
        "[pipeline] foreach '{}': {} targets from '{}', max_parallel={}",
        step.tool_name, lines.len(), source_id, max_par
    );

    let mut total_stored = 0usize;
    let mut total_lines = 0usize;
    let mut any_failed = false;
    let mut combined_output = String::new();

    // Process in chunks of max_parallel
    for chunk in lines.chunks(max_par) {
        let chunk_futures = chunk.iter().enumerate().map(|(ci, iter_target)| {
            let sub_tmp = tmp_dir.join(format!("foreach-{}-{}", step.id, ci));
            let _ = std::fs::create_dir_all(&sub_tmp);

            async move {
                if let Some(sub_pipeline) = resolve_sub_pipeline(step) {
                    execute_pipeline_headless_inner(
                        pool, &sub_pipeline, iter_target, project_path,
                        config_manager, app, depth + 1,
                    ).await
                } else if !step.command_template.is_empty() {
                    let cmd = resolve_tool_command(&step.command_template, config_manager).await;
                    let args_str = step.args.join(" ");
                    let mut cmd_str = if args_str.is_empty() { cmd } else { format!("{} {}", cmd, args_str) };
                    cmd_str = cmd_str.replace("{target}", iter_target);

                    let output = tokio::process::Command::new("sh")
                        .arg("-c").arg(&cmd_str)
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped())
                        .output().await;

                    match output {
                        Ok(out) => {
                            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                            Ok(PipelineRunResult {
                                pipeline_name: step.tool_name.clone(),
                                target: iter_target.clone(),
                                steps: vec![StepResult {
                                    step_id: step.id.clone(),
                                    tool_name: step.tool_name.clone(),
                                    command: cmd_str,
                                    exit_code: out.status.code(),
                                    stdout_lines: stdout.lines().count(),
                                    stderr_preview: String::from_utf8_lossy(&out.stderr).chars().take(200).collect(),
                                    store_stats: None,
                                    duration_ms: 0,
                                }],
                                total_stored: 0,
                                total_duration_ms: 0,
                            })
                        }
                        Err(e) => Err(anyhow::anyhow!("foreach cmd error: {}", e)),
                    }
                } else {
                    Err(anyhow::anyhow!("foreach step has neither sub_pipeline nor command_template"))
                }
            }
        });

        let chunk_results = futures::future::join_all(chunk_futures).await;
        for res in chunk_results {
            match res {
                Ok(r) => {
                    total_stored += r.total_stored;
                    for s in &r.steps {
                        total_lines += s.stdout_lines;
                        if s.exit_code.is_some() && s.exit_code != Some(0) {
                            any_failed = true;
                        }
                    }
                    combined_output.push_str(&format!("{}: {} stored\n", r.target, r.total_stored));
                }
                Err(e) => {
                    any_failed = true;
                    combined_output.push_str(&format!("ERROR: {}\n", e));
                }
            }
        }
    }

    let _ = std::fs::write(&output_file, &combined_output);
    let duration_ms = step_start.elapsed().as_millis() as u64;

    emit_pipeline_event(app, &PipelineEvent {
        pipeline_id: pipeline_id.to_string(), run_id: run_id.to_string(),
        step_id: step.id.clone(), step_index, total_steps,
        status: if any_failed { "error" } else { "completed" }.to_string(),
        tool_name: step.tool_name.clone(),
        message: Some(format!("foreach: {} iterations, {} stored", lines.len(), total_stored)),
        store_stats: None, pipeline_name: None, target: None, all_steps: None,
        output: if combined_output.len() > 4096 { Some(combined_output[..4096].to_string()) } else { Some(combined_output) },
        duration_ms: Some(duration_ms),
        exit_code: if any_failed { Some(1) } else { Some(0) },
    });

    SingleStepResult {
        step_result: StepResult {
            step_id: step.id.clone(), tool_name: step.tool_name.clone(),
            command: format!("[foreach:{}:{}]", source_id, lines.len()),
            exit_code: if any_failed { Some(1) } else { Some(0) },
            stdout_lines: total_lines,
            stderr_preview: String::new(), store_stats: None, duration_ms,
        },
        output_path: output_file,
        stored_count: total_stored,
    }
}

/// Execute a single pipeline step: resolve command, run process(es), parse output, store to DB.
/// This is the shared core used by both the headless and Tauri executors.

async fn run_single_step<'a>(
    pool: &'a sqlx::PgPool,
    step: &'a PipelineStep,
    step_index: usize,
    total_steps: usize,
    target: &'a str,
    project_path: Option<&'a str>,
    parent_target_id: Option<Uuid>,
    input_file: Option<std::path::PathBuf>,
    tmp_dir: &'a std::path::Path,
    config_manager: &'a golish_pentest::ConfigManager,
    pipeline_id: &'a str,
    run_id: &'a str,
    app: Option<&'a tauri::AppHandle>,
    step_outputs: &'a std::collections::HashMap<String, std::path::PathBuf>,
    depth: usize,
) -> SingleStepResult {
    let step_start = std::time::Instant::now();

    emit_pipeline_event(app, &PipelineEvent {
        pipeline_id: pipeline_id.to_string(),
        run_id: run_id.to_string(),
        step_id: step.id.clone(),
        step_index,
        total_steps,
        status: "running".to_string(),
        tool_name: step.tool_name.clone(),
        message: None,
        store_stats: None,
        pipeline_name: None, target: None, all_steps: None,
        output: None, duration_ms: None, exit_code: None,
    });

    // ── Sub-pipeline execution ──
    if step.step_type == "sub_pipeline" {
        return run_sub_pipeline_step(
            pool, step, step_index, total_steps, target, project_path,
            parent_target_id, tmp_dir, config_manager, pipeline_id, run_id, app, depth,
        ).await;
    }

    // ── Foreach iteration ──
    if step.step_type == "foreach" {
        return run_foreach_step(
            pool, step, step_index, total_steps, target, project_path,
            parent_target_id, tmp_dir, config_manager, pipeline_id, run_id, app,
            step_outputs, depth,
        ).await;
    }

    let iter_targets: Vec<String> = if step.iterate_over.as_deref() == Some("ports") {
        resolve_port_targets(pool, target, project_path).await
    } else {
        vec![target.to_string()]
    };

    let resolved_cmd = resolve_tool_command(&step.command_template, config_manager).await;
    let args_str = step.args.join(" ");
    let mut combined_stdout = String::new();
    let mut combined_stderr = String::new();
    let mut last_exit_code: Option<i32> = Some(0);
    let mut last_cmd_str = String::new();

    for iter_target in &iter_targets {
        let mut cmd_str = if args_str.is_empty() {
            resolved_cmd.clone()
        } else {
            format!("{} {}", resolved_cmd, args_str)
        };
        cmd_str = cmd_str.replace("{target}", iter_target);

        if let Some(ref input) = input_file {
            cmd_str = cmd_str.replace("{prev_output}", &input.to_string_lossy());
        }
        last_cmd_str = cmd_str.clone();

        tracing::info!(
            "[pipeline] Step {}/{}: {} → {}{}",
            step_index + 1, total_steps, step.tool_name, cmd_str,
            if iter_targets.len() > 1 { format!(" (port iter {}/{})", iter_targets.iter().position(|t| t == iter_target).unwrap_or(0) + 1, iter_targets.len()) } else { String::new() }
        );

        let proc_result = if let Some(timeout_s) = step.timeout_secs {
            tokio::time::timeout(
                std::time::Duration::from_secs(timeout_s),
                tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(&cmd_str)
                    .stdin(if let Some(ref pf) = input_file {
                        if step.exec_mode == "pipe" {
                            match std::fs::File::open(pf) {
                                Ok(f) => std::process::Stdio::from(f),
                                Err(_) => std::process::Stdio::null(),
                            }
                        } else {
                            std::process::Stdio::null()
                        }
                    } else {
                        std::process::Stdio::null()
                    })
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .output(),
            )
            .await
        } else {
            Ok(
                tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(&cmd_str)
                    .stdin(if let Some(ref pf) = input_file {
                        if step.exec_mode == "pipe" {
                            match std::fs::File::open(pf) {
                                Ok(f) => std::process::Stdio::from(f),
                                Err(_) => std::process::Stdio::null(),
                            }
                        } else {
                            std::process::Stdio::null()
                        }
                    } else {
                        std::process::Stdio::null()
                    })
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .output()
                    .await,
            )
        };

        match proc_result {
            Ok(Ok(output)) => {
                combined_stdout.push_str(&String::from_utf8_lossy(&output.stdout));
                combined_stderr.push_str(&String::from_utf8_lossy(&output.stderr));
                if output.status.code() != Some(0) {
                    last_exit_code = output.status.code();
                }
            }
            Ok(Err(e)) => {
                combined_stderr.push_str(&format!("Process error: {e}\n"));
                last_exit_code = Some(-1);
            }
            Err(_) => {
                combined_stderr.push_str(&format!("Step timed out after {}s\n", step.timeout_secs.unwrap_or(0)));
                last_exit_code = Some(-2);
            }
        }
    }

    let stdout = combined_stdout;
    let stderr = combined_stderr;
    let exit_code = last_exit_code;

    let output_file = tmp_dir.join(format!("step-{}-{}.txt", step_index, step.tool_name));
    let _ = std::fs::write(&output_file, &stdout);

    // Parse output and store to DB
    let mut step_stored = 0usize;
    let store_stats = if let Some(mut output_config) = load_tool_output_config(&step.tool_name) {
        if let Some(ref override_action) = step.db_action {
            output_config.db_action = Some(override_action.clone());
        }
        tracing::info!(
            tool = %step.tool_name,
            format = %output_config.format,
            db_action = ?output_config.db_action,
            stdout_len = stdout.len(),
            "[pipeline-store] Found output config"
        );
        let parse_input = if let Some(ref jq_expr) = output_config.transform {
            super::output_parser::transform_with_jq(&stdout, jq_expr).await
        } else {
            stdout.clone()
        };

        let items = match output_config.format.as_str() {
            "text" => {
                let patterns: Vec<PatternConfig> = output_config
                    .patterns
                    .iter()
                    .map(|p| PatternConfig {
                        data_type: p.data_type.clone(),
                        regex: p.regex.clone(),
                        fields: p.fields.clone(),
                    })
                    .collect();
                super::output_parser::parse_text_standalone(&parse_input, &patterns)
            }
            "json_lines" | "json" => {
                super::output_parser::parse_json_standalone(
                    &parse_input,
                    &output_config.fields,
                    output_config.format == "json_lines",
                )
            }
            _ => vec![],
        };

        let parsed_count = items.len();
        let mut stored_count = 0usize;
        let mut new_count = 0usize;
        let mut skipped_count = 0usize;
        let mut errors = Vec::new();
        let tool_name = &step.tool_name;

        if let Some(ref db_action) = output_config.db_action {
            for item in &items {
                let mut item = item.clone();
                if !item.fields.contains_key("host")
                    && !item.fields.contains_key("ip")
                    && !item.fields.contains_key("url")
                {
                    item.fields.insert("host".to_string(), target.to_string());
                }
                item.fields
                    .entry("_tool".to_string())
                    .or_insert_with(|| tool_name.clone());
                if db_action == "target_add" {
                    match store_target_from_item(pool, &item, project_path, parent_target_id).await
                    {
                        Ok(is_new) => {
                            stored_count += 1;
                            if is_new {
                                new_count += 1;
                            }
                        }
                        Err(e) => {
                            tracing::warn!(tool = %step.tool_name, error = %e, "[pipeline-store] Store error");
                            skipped_count += 1;
                            if errors.len() < 5 {
                                errors.push(e);
                            }
                        }
                    }
                    continue;
                }
                let result = match db_action.as_str() {
                    "target_update_recon" => {
                        store_recon_from_item(pool, &item, project_path).await
                    }
                    "directory_entry_add" => {
                        store_dirent_from_item(pool, &item, tool_name, project_path).await
                    }
                    "finding_add" => {
                        store_finding_from_item(pool, &item, tool_name, project_path).await
                    }
                    _ => {
                        skipped_count += 1;
                        continue;
                    }
                };
                match result {
                    Ok(is_new) => {
                        stored_count += 1;
                        if is_new {
                            new_count += 1;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(tool = %step.tool_name, error = %e, "[pipeline-store] Store error");
                        skipped_count += 1;
                        if errors.len() < 5 {
                            errors.push(e);
                        }
                    }
                }
            }
        }

        tracing::info!(
            tool = %step.tool_name,
            stored = stored_count,
            new = new_count,
            skipped = skipped_count,
            "[pipeline-store] Store complete"
        );
        step_stored = stored_count;
        Some(StoreStats {
            parsed_count,
            stored_count,
            new_count,
            skipped_count,
            errors,
        })
    } else {
        tracing::debug!(tool = %step.tool_name, "[pipeline-store] No output config found");
        None
    };

    // Post-step actions: merge crawler URLs into sitemap
    if step.step_type == "web_crawl" && exit_code == Some(0) && !stdout.is_empty() {
        let urls: Vec<String> = stdout
            .lines()
            .filter(|l| l.starts_with("http://") || l.starts_with("https://"))
            .map(|l| l.trim().to_string())
            .collect();
        if !urls.is_empty() {
            tracing::info!(count = urls.len(), "[pipeline] Merging katana URLs into sitemap");
            merge_urls_into_sitemap(pool, &urls, project_path).await;
            emit_pipeline_event(app, &PipelineEvent {
                pipeline_id: pipeline_id.to_string(),
                run_id: run_id.to_string(),
                step_id: "sitemap_merge".to_string(),
                step_index,
                total_steps,
                status: "info".to_string(),
                tool_name: "katana".to_string(),
                message: Some(format!("Merged {} URLs into sitemap", urls.len())),
                store_stats: None,
                pipeline_name: None,
                target: None,
                all_steps: None,
                output: None,
                duration_ms: None,
                exit_code: None,
            });
        }
    }

    let duration_ms = step_start.elapsed().as_millis() as u64;

    let truncated_output = if stdout.len() > 4096 {
        let mut s = stdout[..4096].to_string();
        s.push_str("\n… (truncated)");
        Some(s)
    } else if stdout.is_empty() {
        None
    } else {
        Some(stdout.clone())
    };
    emit_pipeline_event(app, &PipelineEvent {
        pipeline_id: pipeline_id.to_string(),
        run_id: run_id.to_string(),
        step_id: step.id.clone(),
        step_index,
        total_steps,
        status: if exit_code == Some(0) {
            "completed".to_string()
        } else {
            "error".to_string()
        },
        tool_name: step.tool_name.clone(),
        message: Some(format!(
            "exit={}, lines={}, stored={}",
            exit_code.unwrap_or(-1),
            stdout.lines().count(),
            store_stats
                .as_ref()
                .map(|s| s.stored_count)
                .unwrap_or(0),
        )),
        store_stats: store_stats.clone(),
        pipeline_name: None,
        target: None,
        all_steps: None,
        output: truncated_output,
        duration_ms: Some(duration_ms),
        exit_code,
    });

    SingleStepResult {
        step_result: StepResult {
            step_id: step.id.clone(),
            tool_name: step.tool_name.clone(),
            command: last_cmd_str,
            exit_code,
            stdout_lines: stdout.lines().count(),
            stderr_preview: stderr.chars().take(500).collect(),
            store_stats,
            duration_ms,
        },
        output_path: tmp_dir.join(format!("step-{}-{}.txt", step_index, step.tool_name)),
        stored_count: step_stored,
    }
}

/// Pipeline executor for AI tool usage. Optionally emits `pipeline-event` to
/// the frontend when an `AppHandle` is provided, enabling real-time step progress.
///
/// Uses DAG-based parallel execution: steps are grouped into layers via topological
/// sort of `pipeline.connections`. Steps within the same layer run concurrently.
pub async fn execute_pipeline_headless(
    pool: &sqlx::PgPool,
    pipeline: &Pipeline,
    target: &str,
    project_path: Option<&str>,
    config_manager: &golish_pentest::ConfigManager,
    app: Option<&tauri::AppHandle>,
) -> anyhow::Result<PipelineRunResult> {
    execute_pipeline_headless_inner(pool, pipeline, target, project_path, config_manager, app, 0).await
}

async fn execute_pipeline_headless_inner(
    pool: &sqlx::PgPool,
    pipeline: &Pipeline,
    target: &str,
    project_path: Option<&str>,
    config_manager: &golish_pentest::ConfigManager,
    app: Option<&tauri::AppHandle>,
    depth: usize,
) -> anyhow::Result<PipelineRunResult> {
    let total_steps = pipeline.steps.len();
    let pipeline_id = pipeline.id.clone();
    let run_id = Uuid::new_v4().to_string();
    let mut step_results = Vec::new();
    let mut total_stored = 0usize;
    let start = std::time::Instant::now();
    let target_type = detect_target_type(target);

    let tmp_dir = std::env::temp_dir().join(format!("golish-pipeline-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&tmp_dir)?;

    let parent_target_id: Option<Uuid> = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM targets WHERE value = $1 AND project_path = $2 LIMIT 1",
    )
    .bind(target)
    .bind(project_path)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let mut step_outputs: std::collections::HashMap<String, std::path::PathBuf> =
        std::collections::HashMap::new();

    // Build step-id → original-index map for correct event numbering
    let step_index_map: std::collections::HashMap<&str, usize> = pipeline
        .steps
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.as_str(), i))
        .collect();

    let layers = topo_layers(&pipeline.steps, &pipeline.connections);
    tracing::info!(
        "[pipeline] DAG: {} steps in {} layers, connections={}",
        total_steps,
        layers.len(),
        pipeline.connections.len()
    );

    // Emit "started" event with full step manifest
    emit_pipeline_event(app, &PipelineEvent {
        pipeline_id: pipeline_id.clone(),
        run_id: run_id.clone(),
        step_id: String::new(),
        step_index: 0,
        total_steps,
        status: "started".to_string(),
        tool_name: String::new(),
        message: None,
        store_stats: None,
        pipeline_name: Some(pipeline.name.clone()),
        target: Some(target.to_string()),
        all_steps: Some(
            pipeline
                .steps
                .iter()
                .map(|s| PipelineStepInfo {
                    id: s.id.clone(),
                    tool_name: s.tool_name.clone(),
                    command_template: s.command_template.clone(),
                })
                .collect(),
        ),
        output: None,
        duration_ms: None,
        exit_code: None,
    });

    let mut had_abort = false;

    for (layer_idx, layer) in layers.iter().enumerate() {
        if had_abort {
            break;
        }

        // Check cancel flag between layers
        if PIPELINE_CANCELLED.load(Ordering::SeqCst) {
            tracing::info!("[pipeline] Cancelled by user before layer {}", layer_idx + 1);
            emit_pipeline_event(app, &PipelineEvent {
                pipeline_id: pipeline_id.clone(),
                run_id: run_id.clone(),
                step_id: String::new(),
                step_index: 0,
                total_steps,
                status: "cancelled".to_string(),
                tool_name: String::new(),
                message: Some("Pipeline cancelled by user".to_string()),
                store_stats: None,
                pipeline_name: None,
                target: Some(target.to_string()),
                all_steps: None,
                output: None,
                duration_ms: None,
                exit_code: None,
            });
            break;
        }

        // Separate skipped steps from runnable steps
        let mut runnable: Vec<(&PipelineStep, usize, Option<std::path::PathBuf>)> = Vec::new();

        for &step in layer {
            let idx = step_index_map.get(step.id.as_str()).copied().unwrap_or(0);

            if let Some(ref req) = step.requires {
                if req != target_type {
                    tracing::info!(
                        "[pipeline] Skipping '{}': requires={}, target_type={}",
                        step.tool_name, req, target_type
                    );
                    emit_pipeline_event(app, &PipelineEvent {
                        pipeline_id: pipeline_id.clone(),
                        run_id: run_id.clone(),
                        step_id: step.id.clone(),
                        step_index: idx,
                        total_steps,
                        status: "skipped".to_string(),
                        tool_name: step.tool_name.clone(),
                        message: Some(format!("Skipped: requires {} target", req)),
                        store_stats: None,
                        pipeline_name: None,
                        target: None,
                        all_steps: None,
                        output: None,
                        duration_ms: None,
                        exit_code: None,
                    });
                    step_results.push(StepResult {
                        step_id: step.id.clone(),
                        tool_name: step.tool_name.clone(),
                        command: String::new(),
                        exit_code: None,
                        stdout_lines: 0,
                        stderr_preview: format!("Skipped: requires {} target", req),
                        store_stats: None,
                        duration_ms: 0,
                    });
                    continue;
                }
            }

            // Check conditional connections: all conditions on incoming edges must pass
            let incoming_conds: Vec<(&str, &str)> = pipeline.connections.iter()
                .filter(|c| c.to_step == step.id && c.condition.is_some())
                .map(|c| (c.from_step.as_str(), c.condition.as_deref().unwrap()))
                .collect();

            let mut cond_failed = false;
            for (from_id, cond_expr) in &incoming_conds {
                let upstream = step_results.iter().find(|r| r.step_id == *from_id);
                let upstream_output = step_outputs.get(*from_id);
                if let (Some(res), Some(out)) = (upstream, upstream_output) {
                    if !evaluate_condition(cond_expr, res, out) {
                        tracing::info!(
                            "[pipeline] Skipping '{}': condition '{}' on edge from '{}' not met",
                            step.tool_name, cond_expr, from_id
                        );
                        cond_failed = true;
                        break;
                    }
                } else {
                    tracing::warn!(
                        "[pipeline] Skipping '{}': upstream '{}' has no result for condition eval",
                        step.tool_name, from_id
                    );
                    cond_failed = true;
                    break;
                }
            }
            if cond_failed {
                emit_pipeline_event(app, &PipelineEvent {
                    pipeline_id: pipeline_id.clone(),
                    run_id: run_id.clone(),
                    step_id: step.id.clone(),
                    step_index: idx,
                    total_steps,
                    status: "skipped".to_string(),
                    tool_name: step.tool_name.clone(),
                    message: Some("Skipped: condition not met".to_string()),
                    store_stats: None,
                    pipeline_name: None, target: None, all_steps: None,
                    output: None, duration_ms: None, exit_code: None,
                });
                step_results.push(StepResult {
                    step_id: step.id.clone(),
                    tool_name: step.tool_name.clone(),
                    command: String::new(),
                    exit_code: None,
                    stdout_lines: 0,
                    stderr_preview: "Skipped: condition not met".to_string(),
                    store_stats: None,
                    duration_ms: 0,
                });
                continue;
            }

            let input_file = resolve_step_input(
                step,
                &step_outputs,
                &pipeline.connections,
                &tmp_dir,
                target,
            );
            runnable.push((step, idx, input_file));
        }

        if runnable.is_empty() {
            continue;
        }

        tracing::info!(
            "[pipeline] Layer {}/{}: running {} steps concurrently: [{}]",
            layer_idx + 1,
            layers.len(),
            runnable.len(),
            runnable.iter().map(|(s, _, _)| s.tool_name.as_str()).collect::<Vec<_>>().join(", ")
        );

        // Execute all runnable steps in this layer concurrently
        let layer_futures = runnable.iter().map(|(step, idx, input_file)| {
            run_single_step(
                pool,
                step,
                *idx,
                total_steps,
                target,
                project_path,
                parent_target_id,
                input_file.clone(),
                &tmp_dir,
                config_manager,
                &pipeline_id,
                &run_id,
                app,
                &step_outputs,
                depth,
            )
        });

        let layer_results = futures::future::join_all(layer_futures).await;

        // Merge results from this layer
        for result in layer_results {
            step_outputs.insert(
                result.step_result.step_id.clone(),
                result.output_path,
            );
            total_stored += result.stored_count;

            let failed = result.step_result.exit_code.is_some()
                && result.step_result.exit_code != Some(0);

            // Find the step's on_failure policy
            let on_failure = pipeline
                .steps
                .iter()
                .find(|s| s.id == result.step_result.step_id)
                .map(|s| s.on_failure.as_str())
                .unwrap_or("abort");

            step_results.push(result.step_result);

            if failed && on_failure == "abort" {
                had_abort = true;
            }
        }
    }

    let _ = std::fs::remove_dir_all(&tmp_dir);

    let total_duration_ms = start.elapsed().as_millis() as u64;
    let completed_steps = step_results
        .iter()
        .filter(|s| s.exit_code == Some(0))
        .count();
    let failed_steps = step_results
        .iter()
        .filter(|s| s.exit_code.is_some() && s.exit_code != Some(0))
        .count();

    let resolved_target_id = if parent_target_id.is_some() {
        parent_target_id
    } else {
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM targets WHERE value = $1 AND project_path = $2 LIMIT 1",
        )
        .bind(target)
        .bind(project_path)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
    };

    let step_summaries: Vec<serde_json::Value> = step_results
        .iter()
        .map(|s| {
            serde_json::json!({
                "tool": s.tool_name,
                "stored": s.store_stats.as_ref().map(|st| st.stored_count).unwrap_or(0),
                "new": s.store_stats.as_ref().map(|st| st.new_count).unwrap_or(0),
                "parsed": s.store_stats.as_ref().map(|st| st.parsed_count).unwrap_or(0),
                "exit": s.exit_code,
                "ms": s.duration_ms,
            })
        })
        .collect();

    let _ = golish_db::repo::audit::log_operation(
        pool,
        "pipeline_executed",
        "recon",
        &format!(
            "Pipeline '{}' on {}: {}/{} steps completed, {} items stored",
            pipeline.name, target, completed_steps, total_steps, total_stored
        ),
        project_path,
        "pipeline",
        resolved_target_id,
        None,
        Some(&pipeline.name),
        if failed_steps == 0 {
            "completed"
        } else {
            "partial"
        },
        &serde_json::json!({
            "pipeline_id": pipeline_id,
            "run_id": run_id,
            "target": target,
            "total_steps": total_steps,
            "completed_steps": completed_steps,
            "failed_steps": failed_steps,
            "total_stored": total_stored,
            "total_new": step_results.iter().filter_map(|s| s.store_stats.as_ref()).map(|st| st.new_count).sum::<usize>(),
            "duration_ms": total_duration_ms,
            "steps": step_summaries,
        }),
    )
    .await;

    if total_stored > 0 {
        if let Some(app) = app {
            let _ = app.emit(
                "targets-changed",
                serde_json::json!({
                    "source": "pipeline",
                    "target": target,
                    "stored": total_stored,
                }),
            );
        }
    }

    Ok(PipelineRunResult {
        pipeline_name: pipeline.name.clone(),
        target: target.to_string(),
        steps: step_results,
        total_stored,
        total_duration_ms,
    })
}

fn extract_hostname(val: &str) -> String {
    if val.starts_with("http://") || val.starts_with("https://") {
        url::Url::parse(val)
            .ok()
            .and_then(|u| u.host_str().map(|s| s.to_string()))
            .unwrap_or_else(|| val.to_string())
    } else {
        val.to_string()
    }
}

/// Merge crawler-discovered URLs into the ZAP sitemap (sitemap_store).
/// Only appends entries whose dedup key (method:host:path) doesn't already exist.
async fn merge_urls_into_sitemap(
    pool: &sqlx::PgPool,
    urls: &[String],
    project_path: Option<&str>,
) {
    if urls.is_empty() { return; }
    let pp = project_path.filter(|s| !s.is_empty());

    let existing: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT data FROM sitemap_store WHERE name = 'zap-sitemap' AND project_path = $1",
    )
    .bind(pp)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let mut sitemap_data = existing.unwrap_or_else(|| serde_json::json!({
        "entries": {},
        "meta": { "source": "katana-merge" },
    }));

    let entries = sitemap_data
        .get_mut("entries")
        .and_then(|e| e.as_object_mut());
    let Some(entries) = entries else {
        tracing::warn!("[katana-sitemap] Could not get entries map from sitemap data");
        return;
    };

    let now = chrono::Utc::now().to_rfc3339();
    let mut added = 0usize;
    for raw_url in urls {
        let parsed = match url::Url::parse(raw_url) {
            Ok(u) => u,
            Err(_) => continue,
        };
        let host = parsed.host_str().unwrap_or("").to_string();
        let path = parsed.path().to_string();
        let dedup_key = format!("GET:{}:{}", host, path);

        if entries.contains_key(&dedup_key) {
            continue;
        }

        let port = parsed.port().unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 });

        entries.insert(dedup_key, serde_json::json!({
            "url": raw_url,
            "host": host,
            "method": "GET",
            "path": path,
            "port": port,
            "status_code": 0,
            "content_length": 0,
            "first_seen": &now,
            "last_seen": &now,
            "source": "katana",
            "captured": false,
        }));
        added += 1;
    }

    if added == 0 { return; }

    tracing::info!(
        added = added,
        total = entries.len(),
        "[katana-sitemap] Merged URLs into sitemap"
    );

    let _ = sqlx::query(
        "DELETE FROM sitemap_store WHERE name = 'zap-sitemap' AND project_path = $1",
    )
    .bind(pp)
    .execute(pool)
    .await;

    let _ = sqlx::query(
        r#"INSERT INTO sitemap_store (name, data, project_path)
           VALUES ('zap-sitemap', $1, $2)"#,
    )
    .bind(&sitemap_data)
    .bind(pp)
    .execute(pool)
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_target_type() {
        assert_eq!(detect_target_type("example.com"), "domain");
        assert_eq!(detect_target_type("sub.example.com"), "domain");
        assert_eq!(detect_target_type("192.168.1.1"), "ip");
        assert_eq!(detect_target_type("10.0.0.1"), "ip");
        assert_eq!(detect_target_type("https://example.com"), "url");
        assert_eq!(detect_target_type("http://example.com/path"), "url");
        assert_eq!(detect_target_type("8.138.1.100"), "ip");
    }

    #[test]
    fn test_recon_basic_has_requires() {
        let pipeline = recon_basic_template();
        let dig = pipeline.steps.iter().find(|s| s.tool_name == "dig").unwrap();
        assert_eq!(dig.requires.as_deref(), Some("domain"));

        let subfinder = pipeline.steps.iter().find(|s| s.tool_name == "subfinder").unwrap();
        assert_eq!(subfinder.requires.as_deref(), Some("domain"));

        let httpx = pipeline.steps.iter().find(|s| s.tool_name == "httpx").unwrap();
        assert_eq!(httpx.requires, None);
        assert_eq!(httpx.input_from, None);
        assert_eq!(httpx.iterate_over.as_deref(), Some("ports"));

        let naabu = pipeline.steps.iter().find(|s| s.tool_name == "naabu").unwrap();
        assert_eq!(naabu.requires, None);
    }

    #[test]
    fn test_recon_basic_step_order() {
        let pipeline = recon_basic_template();
        let names: Vec<&str> = pipeline.steps.iter().map(|s| s.tool_name.as_str()).collect();
        assert_eq!(names, &["dig", "subfinder", "naabu", "httpx", "whatweb", "katana"]);

        let naabu_idx = names.iter().position(|n| *n == "naabu").unwrap();
        let httpx_idx = names.iter().position(|n| *n == "httpx").unwrap();
        let whatweb_idx = names.iter().position(|n| *n == "whatweb").unwrap();
        assert!(naabu_idx < httpx_idx, "naabu must run before httpx");
        assert!(naabu_idx < whatweb_idx, "naabu must run before whatweb");
    }

    #[test]
    fn test_recon_basic_iterate_over() {
        let pipeline = recon_basic_template();
        let httpx = pipeline.steps.iter().find(|s| s.tool_name == "httpx").unwrap();
        assert_eq!(httpx.iterate_over.as_deref(), Some("ports"));

        let whatweb = pipeline.steps.iter().find(|s| s.tool_name == "whatweb").unwrap();
        assert_eq!(whatweb.iterate_over.as_deref(), Some("ports"));

        let katana = pipeline.steps.iter().find(|s| s.tool_name == "katana").unwrap();
        assert_eq!(katana.iterate_over.as_deref(), Some("ports"));
        assert_eq!(katana.db_action.as_deref(), Some("target_add"));

        let naabu = pipeline.steps.iter().find(|s| s.tool_name == "naabu").unwrap();
        assert_eq!(naabu.iterate_over, None);
    }

    // ── Helpers ──

    fn conn(from: &str, to: &str) -> PipelineConnection {
        PipelineConnection { from_step: from.into(), to_step: to.into(), condition: None }
    }

    fn conn_if(from: &str, to: &str, cond: &str) -> PipelineConnection {
        PipelineConnection { from_step: from.into(), to_step: to.into(), condition: Some(cond.into()) }
    }

    fn make_step(id: &str) -> PipelineStep {
        PipelineStep {
            id: id.to_string(),
            step_type: "shell_command".to_string(),
            tool_name: id.to_string(),
            tool_id: String::new(),
            command_template: "echo".to_string(),
            args: vec![],
            params: serde_json::json!({}),
            input_from: None,
            exec_mode: "sequential".to_string(),
            requires: None,
            iterate_over: None,
            db_action: None,
            on_failure: "continue".to_string(),
            timeout_secs: None,
            sub_pipeline: None,
            inline_pipeline: None,
            foreach_source: None,
            max_parallel: None,
            x: 0.0,
            y: 0.0,
        }
    }

    #[test]
    fn test_topo_layers_empty_connections_is_sequential() {
        let steps = vec![make_step("a"), make_step("b"), make_step("c")];
        let layers = topo_layers(&steps, &[]);
        assert_eq!(layers.len(), 3, "empty connections → one layer per step");
        assert_eq!(layers[0][0].id, "a");
        assert_eq!(layers[1][0].id, "b");
        assert_eq!(layers[2][0].id, "c");
    }

    #[test]
    fn test_topo_layers_linear_chain() {
        let steps = vec![make_step("a"), make_step("b"), make_step("c")];
        let conns = vec![conn("a", "b"), conn("b", "c")];
        let layers = topo_layers(&steps, &conns);
        assert_eq!(layers.len(), 3);
        assert_eq!(layers[0].len(), 1);
        assert_eq!(layers[0][0].id, "a");
        assert_eq!(layers[1][0].id, "b");
        assert_eq!(layers[2][0].id, "c");
    }

    #[test]
    fn test_topo_layers_parallel_fan_out() {
        // a → b, a → c (b and c should be in the same layer)
        let steps = vec![make_step("a"), make_step("b"), make_step("c")];
        let conns = vec![conn("a", "b"), conn("a", "c")];
        let layers = topo_layers(&steps, &conns);
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].len(), 1);
        assert_eq!(layers[0][0].id, "a");
        assert_eq!(layers[1].len(), 2, "b and c should run in parallel");
        let ids: Vec<&str> = layers[1].iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"b"));
        assert!(ids.contains(&"c"));
    }

    #[test]
    fn test_topo_layers_diamond() {
        // a → b, a → c, b → d, c → d
        let steps = vec![make_step("a"), make_step("b"), make_step("c"), make_step("d")];
        let conns = vec![conn("a", "b"), conn("a", "c"), conn("b", "d"), conn("c", "d")];
        let layers = topo_layers(&steps, &conns);
        assert_eq!(layers.len(), 3);
        assert_eq!(layers[0][0].id, "a");
        assert_eq!(layers[1].len(), 2, "b and c parallel");
        assert_eq!(layers[2].len(), 1);
        assert_eq!(layers[2][0].id, "d");
    }

    #[test]
    fn test_topo_layers_recon_basic_dag() {
        let pipeline = recon_basic_template();
        let layers = topo_layers(&pipeline.steps, &pipeline.connections);

        // Layer 0: dig, subfinder, naabu (no incoming connections)
        assert_eq!(layers[0].len(), 3, "layer 0 should have dig, subfinder, naabu");
        let l0_ids: Vec<&str> = layers[0].iter().map(|s| s.id.as_str()).collect();
        assert!(l0_ids.contains(&"dns_lookup"));
        assert!(l0_ids.contains(&"subdomain_enum"));
        assert!(l0_ids.contains(&"port_scan"));

        // Layer 1: httpx, whatweb (depend on port_scan)
        assert_eq!(layers[1].len(), 2, "layer 1 should have httpx, whatweb");
        let l1_ids: Vec<&str> = layers[1].iter().map(|s| s.id.as_str()).collect();
        assert!(l1_ids.contains(&"http_probe"));
        assert!(l1_ids.contains(&"tech_fingerprint"));

        // Layer 2: katana (depends on httpx and whatweb)
        assert_eq!(layers[2].len(), 1);
        assert_eq!(layers[2][0].id, "web_crawl");
    }

    #[test]
    fn test_topo_layers_disconnected_steps_at_start() {
        // Steps e and f have no connections, should appear in layer 0
        let steps = vec![
            make_step("a"), make_step("b"), make_step("e"), make_step("f"),
        ];
        let conns = vec![conn("a", "b")];
        let layers = topo_layers(&steps, &conns);
        // a, e, f all have in_degree 0 → layer 0
        assert!(layers[0].len() >= 3);
        let l0_ids: Vec<&str> = layers[0].iter().map(|s| s.id.as_str()).collect();
        assert!(l0_ids.contains(&"a"));
        assert!(l0_ids.contains(&"e"));
        assert!(l0_ids.contains(&"f"));
        // b depends on a → later layer
        let all_later: Vec<&str> = layers[1..].iter().flat_map(|l| l.iter().map(|s| s.id.as_str())).collect();
        assert!(all_later.contains(&"b"));
    }

    #[test]
    fn test_new_fields_have_defaults() {
        let json = r#"{"id":"test","tool_name":"echo","steps":[],"connections":[],"name":"t","created_at":0,"updated_at":0}"#;
        let pipeline: Pipeline = serde_json::from_str(json).unwrap();
        assert!(pipeline.steps.is_empty());

        let step_json = r#"{"id":"s1","tool_name":"nmap"}"#;
        let step: PipelineStep = serde_json::from_str(step_json).unwrap();
        assert_eq!(step.on_failure, "abort");
        assert_eq!(step.timeout_secs, None);
        assert!(step.sub_pipeline.is_none());
        assert!(step.inline_pipeline.is_none());
        assert!(step.foreach_source.is_none());
        assert!(step.max_parallel.is_none());
    }

    #[test]
    fn test_connection_condition_default() {
        let json = r#"{"from_step":"a","to_step":"b"}"#;
        let c: PipelineConnection = serde_json::from_str(json).unwrap();
        assert!(c.condition.is_none());

        let json2 = r#"{"from_step":"a","to_step":"b","condition":"exit_ok"}"#;
        let c2: PipelineConnection = serde_json::from_str(json2).unwrap();
        assert_eq!(c2.condition.as_deref(), Some("exit_ok"));
    }

    #[test]
    fn test_step_sub_pipeline_fields_deser() {
        let json = r#"{"id":"s1","step_type":"sub_pipeline","tool_name":"web","sub_pipeline":"web_vuln_v1"}"#;
        let step: PipelineStep = serde_json::from_str(json).unwrap();
        assert_eq!(step.step_type, "sub_pipeline");
        assert_eq!(step.sub_pipeline.as_deref(), Some("web_vuln_v1"));
        assert!(step.inline_pipeline.is_none());
    }

    #[test]
    fn test_step_foreach_fields_deser() {
        let json = r#"{"id":"s1","step_type":"foreach","tool_name":"scan","foreach_source":"subfinder","max_parallel":3}"#;
        let step: PipelineStep = serde_json::from_str(json).unwrap();
        assert_eq!(step.step_type, "foreach");
        assert_eq!(step.foreach_source.as_deref(), Some("subfinder"));
        assert_eq!(step.max_parallel, Some(3));
    }

    // ── evaluate_condition tests ──

    fn make_result(exit: Option<i32>, lines: usize) -> StepResult {
        StepResult {
            step_id: "test".into(),
            tool_name: "test".into(),
            command: String::new(),
            exit_code: exit,
            stdout_lines: lines,
            stderr_preview: String::new(),
            store_stats: None,
            duration_ms: 0,
        }
    }

    #[test]
    fn test_evaluate_condition_exit_ok() {
        let tmp = std::env::temp_dir().join("test_cond_exit_ok.txt");
        std::fs::write(&tmp, "some output").unwrap();

        assert!(evaluate_condition("exit_ok", &make_result(Some(0), 1), &tmp));
        assert!(!evaluate_condition("exit_ok", &make_result(Some(1), 1), &tmp));
        assert!(!evaluate_condition("exit_ok", &make_result(None, 0), &tmp));
    }

    #[test]
    fn test_evaluate_condition_exit_fail() {
        let tmp = std::env::temp_dir().join("test_cond_exit_fail.txt");
        std::fs::write(&tmp, "").unwrap();

        assert!(evaluate_condition("exit_fail", &make_result(Some(1), 0), &tmp));
        assert!(!evaluate_condition("exit_fail", &make_result(Some(0), 0), &tmp));
        assert!(!evaluate_condition("exit_fail", &make_result(None, 0), &tmp));
    }

    #[test]
    fn test_evaluate_condition_output_not_empty() {
        let tmp = std::env::temp_dir().join("test_cond_not_empty.txt");
        std::fs::write(&tmp, "data").unwrap();

        assert!(evaluate_condition("output_not_empty", &make_result(Some(0), 3), &tmp));
        assert!(!evaluate_condition("output_not_empty", &make_result(Some(0), 0), &tmp));
    }

    #[test]
    fn test_evaluate_condition_output_contains() {
        let tmp = std::env::temp_dir().join("test_cond_contains.txt");
        std::fs::write(&tmp, "80/tcp open http\n22/tcp open ssh").unwrap();

        assert!(evaluate_condition("output_contains:80", &make_result(Some(0), 2), &tmp));
        assert!(evaluate_condition("output_contains:22", &make_result(Some(0), 2), &tmp));
        assert!(!evaluate_condition("output_contains:443", &make_result(Some(0), 2), &tmp));
    }

    #[test]
    fn test_evaluate_condition_output_lines_gt() {
        let tmp = std::env::temp_dir().join("test_cond_lines_gt.txt");
        std::fs::write(&tmp, "").unwrap();

        assert!(evaluate_condition("output_lines_gt:5", &make_result(Some(0), 10), &tmp));
        assert!(!evaluate_condition("output_lines_gt:5", &make_result(Some(0), 3), &tmp));
        assert!(!evaluate_condition("output_lines_gt:5", &make_result(Some(0), 5), &tmp));
    }

    #[test]
    fn test_evaluate_condition_stored_gt() {
        let tmp = std::env::temp_dir().join("test_cond_stored_gt.txt");
        std::fs::write(&tmp, "").unwrap();

        let mut res = make_result(Some(0), 0);
        res.store_stats = Some(StoreStats {
            parsed_count: 12,
            stored_count: 10,
            new_count: 10,
            skipped_count: 2,
            errors: vec![],
        });
        assert!(evaluate_condition("stored_gt:5", &res, &tmp));
        assert!(!evaluate_condition("stored_gt:15", &res, &tmp));

        let res2 = make_result(Some(0), 0);
        assert!(!evaluate_condition("stored_gt:0", &res2, &tmp));
    }

    #[test]
    fn test_evaluate_condition_unknown_passes() {
        let tmp = std::env::temp_dir().join("test_cond_unknown.txt");
        std::fs::write(&tmp, "").unwrap();
        assert!(evaluate_condition("some_future_condition", &make_result(Some(0), 0), &tmp));
    }

    // ── Condition-based DAG skipping (via topo) ──

    #[test]
    fn test_topo_with_conditional_connections() {
        let steps = vec![make_step("scan"), make_step("web"), make_step("ssh")];
        let conns = vec![
            conn_if("scan", "web", "output_contains:80"),
            conn_if("scan", "ssh", "output_contains:22"),
        ];
        let layers = topo_layers(&steps, &conns);
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0][0].id, "scan");
        let l1_ids: Vec<&str> = layers[1].iter().map(|s| s.id.as_str()).collect();
        assert!(l1_ids.contains(&"web"));
        assert!(l1_ids.contains(&"ssh"));
    }

    #[test]
    fn test_inline_pipeline_deser() {
        let json = r#"{
            "id": "nest",
            "step_type": "sub_pipeline",
            "tool_name": "inner",
            "inline_pipeline": {
                "id": "inner_p",
                "name": "Inner Pipeline",
                "steps": [{"id": "echo_step", "tool_name": "echo"}],
                "connections": []
            }
        }"#;
        let step: PipelineStep = serde_json::from_str(json).unwrap();
        assert_eq!(step.step_type, "sub_pipeline");
        let inner = step.inline_pipeline.unwrap();
        assert_eq!(inner.id, "inner_p");
        assert_eq!(inner.steps.len(), 1);
        assert_eq!(inner.steps[0].id, "echo_step");
    }

    #[test]
    fn test_advanced_flow_json_roundtrip() {
        let json = r#"{
            "id": "advanced",
            "name": "Advanced Recon",
            "steps": [
                {"id": "subfinder", "tool_name": "subfinder", "command_template": "subfinder", "args": ["-d", "{target}", "-silent"]},
                {"id": "naabu", "tool_name": "naabu", "command_template": "naabu", "args": ["-host", "{target}"]},
                {"id": "web_scan", "step_type": "sub_pipeline", "tool_name": "web", "sub_pipeline": "web_vuln_v1"},
                {"id": "ssh_audit", "tool_name": "ssh-audit", "command_template": "ssh-audit", "args": ["{target}"]},
                {"id": "per_sub", "step_type": "foreach", "tool_name": "scan", "foreach_source": "subfinder", "sub_pipeline": "single_host_recon", "max_parallel": 3}
            ],
            "connections": [
                {"from_step": "naabu", "to_step": "web_scan", "condition": "output_contains:80"},
                {"from_step": "naabu", "to_step": "ssh_audit", "condition": "output_contains:22"},
                {"from_step": "subfinder", "to_step": "per_sub", "condition": "output_not_empty"}
            ]
        }"#;
        let pipeline: Pipeline = serde_json::from_str(json).unwrap();
        assert_eq!(pipeline.steps.len(), 5);
        assert_eq!(pipeline.connections.len(), 3);

        assert_eq!(pipeline.steps[2].step_type, "sub_pipeline");
        assert_eq!(pipeline.steps[2].sub_pipeline.as_deref(), Some("web_vuln_v1"));
        assert_eq!(pipeline.steps[4].step_type, "foreach");
        assert_eq!(pipeline.steps[4].foreach_source.as_deref(), Some("subfinder"));
        assert_eq!(pipeline.steps[4].max_parallel, Some(3));

        assert_eq!(pipeline.connections[0].condition.as_deref(), Some("output_contains:80"));
        assert_eq!(pipeline.connections[1].condition.as_deref(), Some("output_contains:22"));
        assert_eq!(pipeline.connections[2].condition.as_deref(), Some("output_not_empty"));

        let layers = topo_layers(&pipeline.steps, &pipeline.connections);
        assert_eq!(layers[0].len(), 2, "subfinder and naabu in parallel (layer 0)");
        let l0_ids: Vec<&str> = layers[0].iter().map(|s| s.id.as_str()).collect();
        assert!(l0_ids.contains(&"subfinder"));
        assert!(l0_ids.contains(&"naabu"));

        assert!(layers.len() >= 2);
    }
}
