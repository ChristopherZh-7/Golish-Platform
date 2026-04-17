use serde::{Deserialize, Serialize};
use tauri::Emitter;
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

pub fn get_builtin_recon_basic() -> Pipeline {
    recon_basic_template()
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

/// Resolve per-port target URLs from the `targets.ports` JSONB column.
/// Returns a vec of URLs like `http://8.138.179.62:8080`, `https://8.138.179.62:443`.
async fn resolve_port_targets(
    pool: &sqlx::PgPool,
    target: &str,
    project_path: Option<&str>,
) -> Vec<String> {
    let ports_json: Option<serde_json::Value> = sqlx::query_scalar(
        r#"SELECT ports FROM targets
           WHERE value = $1 AND ($2::text IS NULL OR project_path = $2 OR project_path IS NULL)
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
            x: (i as f64) * 220.0 + 40.0,
            y: 80.0,
        })
        .collect();

    let connections: Vec<PipelineConnection> = steps
        .windows(2)
        .map(|w| PipelineConnection {
            from_step: w[0].id.to_string(),
            to_step: w[1].id.to_string(),
        })
        .collect();

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
// Pipeline executor: run steps sequentially with output parsing and DB storage
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

#[tauri::command]
pub async fn pipeline_execute(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    pipeline: Pipeline,
    target: String,
    project_path: Option<String>,
) -> Result<PipelineRunResult, String> {
    let pool = state.db_pool_ready().await?;
    let total_steps = pipeline.steps.len();
    let pipeline_id = pipeline.id.clone();
    let run_id = Uuid::new_v4().to_string();
    let mut step_results = Vec::new();
    let mut total_stored = 0usize;
    let start = std::time::Instant::now();
    let target_type = detect_target_type(&target);

    // Temp directory for intermediate output files
    let tmp_dir = std::env::temp_dir().join(format!("golish-pipeline-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&tmp_dir).map_err(|e| e.to_string())?;

    let parent_target_id: Option<Uuid> = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM targets WHERE value = $1 AND project_path IS NOT DISTINCT FROM $2 LIMIT 1",
    )
    .bind(&target)
    .bind(project_path.as_deref())
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    // Emit "started" event so the frontend can build the full progress block immediately.
    let _ = app.emit(
        "pipeline-event",
        PipelineEvent {
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
            target: Some(target.clone()),
            all_steps: Some(
                pipeline.steps.iter().map(|s| PipelineStepInfo {
                    id: s.id.clone(),
                    tool_name: s.tool_name.clone(),
                    command_template: s.command_template.clone(),
                }).collect(),
            ),
            output: None, duration_ms: None, exit_code: None,
        },
    );

    // Map step_id → output file path for input_from references
    let mut step_outputs: std::collections::HashMap<String, std::path::PathBuf> = std::collections::HashMap::new();
    let mut prev_output_file: Option<std::path::PathBuf> = None;

    for (idx, step) in pipeline.steps.iter().enumerate() {
        // Skip step if target type doesn't match the requires condition
        if let Some(ref req) = step.requires {
            if req != target_type {
                tracing::info!(
                    "[pipeline] Skipping step '{}': requires={}, target_type={}",
                    step.tool_name, req, target_type
                );
                let _ = app.emit(
                    "pipeline-event",
                    PipelineEvent {
                        pipeline_id: pipeline_id.clone(),
                        run_id: run_id.clone(),
                        step_id: step.id.clone(),
                        step_index: idx,
                        total_steps,
                        status: "skipped".to_string(),
                        tool_name: step.tool_name.clone(),
                        message: Some(format!("Skipped: requires {} target", req)),
                        store_stats: None,
                        pipeline_name: None, target: None, all_steps: None,
                        output: None, duration_ms: None, exit_code: None,
                    },
                );
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

        let step_start = std::time::Instant::now();

        // Emit "running" event
        let _ = app.emit(
            "pipeline-event",
            PipelineEvent {
                pipeline_id: pipeline_id.clone(),
                run_id: run_id.clone(),
                step_id: step.id.clone(),
                step_index: idx,
                total_steps,
                status: "running".to_string(),
                tool_name: step.tool_name.clone(),
                message: None,
                store_stats: None,
                pipeline_name: None, target: None, all_steps: None,
                output: None, duration_ms: None, exit_code: None,
            },
        );

        // Resolve the input file: explicit input_from step, or fallback to previous step
        let mut input_file = step.input_from.as_ref()
            .and_then(|id| step_outputs.get(id).cloned())
            .or_else(|| prev_output_file.clone());

        // When the command uses {prev_output} but no prior step produced output
        // (e.g. domain-only steps were skipped for an IP target), create a seed
        // file containing just the target value so the tool has valid input.
        let full_cmd_preview = format!("{} {}", step.command_template, step.args.join(" "));
        if input_file.is_none() && full_cmd_preview.contains("{prev_output}") {
            let seed = tmp_dir.join(format!("seed-{}.txt", step.id));
            let _ = std::fs::write(&seed, &target);
            input_file = Some(seed);
        }

        // Resolve iteration targets (per-port or single)
        let iter_targets: Vec<String> = if step.iterate_over.as_deref() == Some("ports") {
            resolve_port_targets(pool, &target, project_path.as_deref()).await
        } else {
            vec![target.clone()]
        };

        let resolved_cmd = resolve_tool_command(&step.command_template, &state.pentest_config_manager).await;
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
                idx + 1,
                total_steps,
                step.tool_name,
                cmd_str,
                if iter_targets.len() > 1 { format!(" (port iter {}/{})", iter_targets.iter().position(|t| t == iter_target).unwrap_or(0) + 1, iter_targets.len()) } else { String::new() }
            );

            let mut child = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(&cmd_str)
                .stdin(if let Some(ref pf) = input_file {
                    if step.exec_mode == "pipe" {
                        std::process::Stdio::from(
                            std::fs::File::open(pf).map_err(|e| e.to_string())?,
                        )
                    } else {
                        std::process::Stdio::null()
                    }
                } else {
                    std::process::Stdio::null()
                })
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| format!("Failed to execute '{}': {}", cmd_str, e))?;

            let stdout_pipe = child.stdout.take();
            let stderr_pipe = child.stderr.take();

            let app_for_stdout = app.clone();
            let pid_for_stdout = pipeline_id.clone();
            let rid_for_stdout = run_id.clone();
            let sid_for_stdout = step.id.clone();
            let tool_for_stdout = step.tool_name.clone();
            let step_idx = idx;
            let ts = total_steps;

            let app_for_stderr = app.clone();
            let pid_for_stderr = pipeline_id.clone();
            let rid_for_stderr = run_id.clone();
            let sid_for_stderr = step.id.clone();
            let tool_for_stderr = step.tool_name.clone();

            // Read stdout and stderr concurrently
            let stdout_handle = tokio::spawn(async move {
                let mut collected = String::new();
                if let Some(pipe) = stdout_pipe {
                    use tokio::io::{AsyncBufReadExt, BufReader};
                    let mut reader = BufReader::new(pipe);
                    let mut line = String::new();
                    loop {
                        line.clear();
                        match reader.read_line(&mut line).await {
                            Ok(0) => break,
                            Ok(_) => {
                                collected.push_str(&line);
                                let _ = app_for_stdout.emit(
                                    "pipeline-event",
                                    PipelineEvent {
                                        pipeline_id: pid_for_stdout.clone(),
                                        run_id: rid_for_stdout.clone(),
                                        step_id: sid_for_stdout.clone(),
                                        step_index: step_idx,
                                        total_steps: ts,
                                        status: "output".to_string(),
                                        tool_name: tool_for_stdout.clone(),
                                        message: None,
                                        store_stats: None,
                                        pipeline_name: None, target: None, all_steps: None,
                                        output: Some(line.clone()),
                                        duration_ms: None, exit_code: None,
                                    },
                                );
                            }
                            Err(_) => break,
                        }
                    }
                }
                collected
            });

            let stderr_handle = tokio::spawn(async move {
                let mut collected = String::new();
                if let Some(pipe) = stderr_pipe {
                    use tokio::io::{AsyncBufReadExt, BufReader};
                    let mut reader = BufReader::new(pipe);
                    let mut line = String::new();
                    loop {
                        line.clear();
                        match reader.read_line(&mut line).await {
                            Ok(0) => break,
                            Ok(_) => {
                                collected.push_str(&line);
                                let _ = app_for_stderr.emit(
                                    "pipeline-event",
                                    PipelineEvent {
                                        pipeline_id: pid_for_stderr.clone(),
                                        run_id: rid_for_stderr.clone(),
                                        step_id: sid_for_stderr.clone(),
                                        step_index: step_idx,
                                        total_steps: ts,
                                        status: "output".to_string(),
                                        tool_name: tool_for_stderr.clone(),
                                        message: Some("stderr".to_string()),
                                        store_stats: None,
                                        pipeline_name: None, target: None, all_steps: None,
                                        output: Some(line.clone()),
                                        duration_ms: None, exit_code: None,
                                    },
                                );
                            }
                            Err(_) => break,
                        }
                    }
                }
                collected
            });

            let (stdout_result, stderr_result) = tokio::join!(stdout_handle, stderr_handle);
            let iter_stdout = stdout_result.unwrap_or_default();
            let iter_stderr = stderr_result.unwrap_or_default();

            let status = child.wait().await.map_err(|e| format!("wait failed: {e}"))?;

            combined_stdout.push_str(&iter_stdout);
            combined_stderr.push_str(&iter_stderr);
            if status.code() != Some(0) {
                last_exit_code = status.code();
            }
        }

        let stdout = combined_stdout;
        let stderr = combined_stderr;
        let exit_code = last_exit_code;

        // Save stdout to temp file and register in step_outputs
        let output_file = tmp_dir.join(format!("step-{}-{}.txt", idx, step.tool_name));
        let _ = std::fs::write(&output_file, &stdout);
        step_outputs.insert(step.id.clone(), output_file.clone());
        prev_output_file = Some(output_file);

        // Parse and store if tool has output config
        let store_stats = if let Some(mut output_config) = load_tool_output_config(&step.tool_name) {
            if let Some(ref override_action) = step.db_action {
                output_config.db_action = Some(override_action.clone());
            }

            tracing::info!(
                tool = %step.tool_name,
                format = %output_config.format,
                db_action = ?output_config.db_action,
                stdout_len = stdout.len(),
                stdout_lines = stdout.lines().count(),
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
                    super::output_parser::parse_json_standalone(&parse_input, &output_config.fields, output_config.format == "json_lines")
                }
                _ => vec![],
            };

            tracing::info!(
                tool = %step.tool_name,
                parsed_count = items.len(),
                "[pipeline-store] Parsed items"
            );

            let parsed_count = items.len();
            let mut stored_count = 0usize;
            let mut new_count = 0usize;
            let mut skipped_count = 0usize;
            let mut errors = Vec::new();
            let tool_name = &step.tool_name;

            if let Some(ref db_action) = output_config.db_action {
                for item in &items {
                    let mut item = item.clone();
                    if !item.fields.contains_key("host") && !item.fields.contains_key("ip") && !item.fields.contains_key("url") {
                        item.fields.insert("host".to_string(), target.clone());
                    }
                    if db_action == "target_add" {
                        match store_target_from_item(pool, &item, project_path.as_deref(), parent_target_id).await {
                            Ok(is_new) => { stored_count += 1; if is_new { new_count += 1; } }
                            Err(e) => {
                                tracing::warn!(tool = %step.tool_name, error = %e, "[pipeline-store] Store error");
                                skipped_count += 1;
                                if errors.len() < 5 { errors.push(e); }
                            }
                        }
                        continue;
                    }
                    let result = match db_action.as_str() {
                        "target_update_recon" => {
                            store_recon_from_item(pool, &item, project_path.as_deref()).await
                        }
                        "directory_entry_add" => {
                            store_dirent_from_item(pool, &item, tool_name, project_path.as_deref())
                                .await
                        }
                        "finding_add" => {
                            store_finding_from_item(pool, &item, tool_name, project_path.as_deref())
                                .await
                        }
                        _ => {
                            skipped_count += 1;
                            continue;
                        }
                    };
                    match result {
                        Ok(()) => { stored_count += 1; new_count += 1; }
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
            total_stored += stored_count;
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

        // After katana step: merge discovered URLs into ZAP sitemap
        if step.step_type == "web_crawl" && exit_code == Some(0) && !stdout.is_empty() {
            let urls: Vec<String> = stdout.lines()
                .filter(|l| l.starts_with("http://") || l.starts_with("https://"))
                .map(|l| l.trim().to_string())
                .collect();
            if !urls.is_empty() {
                tracing::info!(count = urls.len(), "[pipeline] Merging katana URLs into sitemap");
                merge_urls_into_sitemap(pool, &urls, project_path.as_deref()).await;
                let _ = app.emit("sitemap-updated", serde_json::json!({ "source": "katana" }));
            }
        }

        let duration_ms = step_start.elapsed().as_millis() as u64;

        // Emit "completed" event with truncated output
        let truncated_output = if stdout.len() > 4096 {
            let mut s = stdout[..4096].to_string();
            s.push_str("\n… (truncated)");
            Some(s)
        } else if stdout.is_empty() {
            None
        } else {
            Some(stdout.clone())
        };
        let _ = app.emit(
            "pipeline-event",
            PipelineEvent {
                pipeline_id: pipeline_id.clone(),
                run_id: run_id.clone(),
                step_id: step.id.clone(),
                step_index: idx,
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
                pipeline_name: None, target: None, all_steps: None,
                output: truncated_output,
                duration_ms: Some(duration_ms),
                exit_code,
            },
        );

        step_results.push(StepResult {
            step_id: step.id.clone(),
            tool_name: step.tool_name.clone(),
            command: last_cmd_str,
            exit_code,
            stdout_lines: stdout.lines().count(),
            stderr_preview: stderr.chars().take(500).collect(),
            store_stats,
            duration_ms,
        });

        // Stop on failure if exec_mode is sequential/on_success
        if exit_code != Some(0)
            && matches!(step.exec_mode.as_str(), "sequential" | "on_success")
        {
            tracing::warn!(
                "[pipeline] Step '{}' failed (exit={}), stopping",
                step.tool_name,
                exit_code.unwrap_or(-1),
            );
            break;
        }
    }

    // Cleanup temp dir
    let _ = std::fs::remove_dir_all(&tmp_dir);

    let total_duration_ms = start.elapsed().as_millis() as u64;
    let completed_steps = step_results.iter().filter(|s| s.exit_code == Some(0)).count();
    let failed_steps = step_results.iter().filter(|s| s.exit_code.is_some() && s.exit_code != Some(0)).count();

    let step_summaries: Vec<serde_json::Value> = step_results.iter().map(|s| {
        serde_json::json!({
            "tool": s.tool_name,
            "stored": s.store_stats.as_ref().map(|st| st.stored_count).unwrap_or(0),
            "new": s.store_stats.as_ref().map(|st| st.new_count).unwrap_or(0),
            "parsed": s.store_stats.as_ref().map(|st| st.parsed_count).unwrap_or(0),
            "exit": s.exit_code,
            "ms": s.duration_ms,
        })
    }).collect();

    let total_new: usize = step_results.iter()
        .filter_map(|s| s.store_stats.as_ref())
        .map(|st| st.new_count)
        .sum();

    let resolved_target_id = if parent_target_id.is_some() {
        parent_target_id
    } else {
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM targets WHERE value = $1 AND project_path IS NOT DISTINCT FROM $2 LIMIT 1",
        )
        .bind(&target)
        .bind(project_path.as_deref())
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
    };

    let _ = golish_db::repo::audit::log_operation(
        pool, "pipeline_executed", "recon",
        &format!("Pipeline '{}' on {}: {}/{} steps completed, {} items stored",
            pipeline.name, target, completed_steps, total_steps, total_stored),
        project_path.as_deref(), "pipeline",
        resolved_target_id, None, Some(&pipeline.name),
        if failed_steps == 0 { "completed" } else { "partial" },
        &serde_json::json!({
            "pipeline_id": pipeline_id,
            "run_id": run_id,
            "target": target,
            "total_steps": total_steps,
            "completed_steps": completed_steps,
            "failed_steps": failed_steps,
            "total_stored": total_stored,
            "total_new": total_new,
            "duration_ms": total_duration_ms,
            "steps": step_summaries,
        }),
    ).await;

    if total_stored > 0 {
        let _ = app.emit("targets-changed", serde_json::json!({
            "source": "pipeline",
            "target": &target,
            "stored": total_stored,
            "new": total_new,
        }));
    }

    Ok(PipelineRunResult {
        pipeline_name: pipeline.name,
        target,
        steps: step_results,
        total_stored,
        total_duration_ms,
    })
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
        "SELECT EXISTS(SELECT 1 FROM targets WHERE value = $1 AND project_path IS NOT DISTINCT FROM $2)",
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

async fn store_recon_from_item(
    pool: &sqlx::PgPool,
    item: &ParsedItem,
    project_path: Option<&str>,
) -> Result<(), String> {
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
    if let Some(techs) = item.fields.get("technologies") {
        if let Ok(arr) = serde_json::from_str::<serde_json::Value>(techs) {
            update.technologies = arr;
        } else {
            let tech_list: Vec<&str> = techs.split(',').map(|s| s.trim()).collect();
            update.technologies = serde_json::to_value(tech_list).unwrap_or_default();
        }
    }

    super::targets::db_target_update_recon_extended(pool, target_uuid, &update).await
}

async fn store_dirent_from_item(
    pool: &sqlx::PgPool,
    item: &ParsedItem,
    tool_name: &str,
    project_path: Option<&str>,
) -> Result<(), String> {
    let url = item.fields.get("url").ok_or("No url field")?;
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
    Ok(())
}

async fn store_finding_from_item(
    pool: &sqlx::PgPool,
    item: &ParsedItem,
    tool_name: &str,
    project_path: Option<&str>,
) -> Result<(), String> {
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

    sqlx::query(
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
    Ok(())
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

/// Pipeline executor for AI tool usage. Optionally emits `pipeline-event` to
/// the frontend when an `AppHandle` is provided, enabling real-time step progress.
pub async fn execute_pipeline_headless(
    pool: &sqlx::PgPool,
    pipeline: &Pipeline,
    target: &str,
    project_path: Option<&str>,
    config_manager: &golish_pentest::ConfigManager,
    app: Option<&tauri::AppHandle>,
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
        "SELECT id FROM targets WHERE value = $1 AND project_path IS NOT DISTINCT FROM $2 LIMIT 1",
    )
    .bind(target)
    .bind(project_path)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let mut step_outputs: std::collections::HashMap<String, std::path::PathBuf> = std::collections::HashMap::new();
    let mut prev_output_file: Option<std::path::PathBuf> = None;

    // Emit "started" event with full step manifest so the frontend can
    // build the progress block immediately.
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
            pipeline.steps.iter().map(|s| PipelineStepInfo {
                id: s.id.clone(),
                tool_name: s.tool_name.clone(),
                command_template: s.command_template.clone(),
            }).collect(),
        ),
        output: None, duration_ms: None, exit_code: None,
    });

    for (idx, step) in pipeline.steps.iter().enumerate() {
        // Skip step if target type doesn't match
        if let Some(ref req) = step.requires {
            if req != target_type {
                tracing::info!(
                    "[pipeline-headless] Skipping '{}': requires={}, target_type={}",
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
                    pipeline_name: None, target: None, all_steps: None,
                    output: None, duration_ms: None, exit_code: None,
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

        let step_start = std::time::Instant::now();

        emit_pipeline_event(app, &PipelineEvent {
            pipeline_id: pipeline_id.clone(),
            run_id: run_id.clone(),
            step_id: step.id.clone(),
            step_index: idx,
            total_steps,
            status: "running".to_string(),
            tool_name: step.tool_name.clone(),
            message: None,
            store_stats: None,
            pipeline_name: None, target: None, all_steps: None,
            output: None, duration_ms: None, exit_code: None,
        });

        // Resolve the input file
        let mut input_file = step.input_from.as_ref()
            .and_then(|id| step_outputs.get(id).cloned())
            .or_else(|| prev_output_file.clone());

        let full_cmd_preview = format!("{} {}", step.command_template, step.args.join(" "));
        if input_file.is_none() && full_cmd_preview.contains("{prev_output}") {
            let seed = tmp_dir.join(format!("seed-{}.txt", step.id));
            let _ = std::fs::write(&seed, target);
            input_file = Some(seed);
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
                "[pipeline-headless] Step {}/{}: {} → {}{}",
                idx + 1, total_steps, step.tool_name, cmd_str,
                if iter_targets.len() > 1 { format!(" (port iter {}/{})", iter_targets.iter().position(|t| t == iter_target).unwrap_or(0) + 1, iter_targets.len()) } else { String::new() }
            );

            let output = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(&cmd_str)
                .stdin(if let Some(ref pf) = input_file {
                    if step.exec_mode == "pipe" {
                        std::process::Stdio::from(std::fs::File::open(pf)?)
                    } else {
                        std::process::Stdio::null()
                    }
                } else {
                    std::process::Stdio::null()
                })
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output()
                .await?;

            combined_stdout.push_str(&String::from_utf8_lossy(&output.stdout));
            combined_stderr.push_str(&String::from_utf8_lossy(&output.stderr));
            if output.status.code() != Some(0) {
                last_exit_code = output.status.code();
            }
        }

        let stdout = combined_stdout;
        let stderr = combined_stderr;
        let exit_code = last_exit_code;

        let output_file = tmp_dir.join(format!("step-{}-{}.txt", idx, step.tool_name));
        let _ = std::fs::write(&output_file, &stdout);
        step_outputs.insert(step.id.clone(), output_file.clone());
        prev_output_file = Some(output_file);

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
                    super::output_parser::parse_json_standalone(&parse_input, &output_config.fields, output_config.format == "json_lines")
                }
                _ => vec![],
            };

            tracing::info!(
                tool = %step.tool_name,
                parsed_count = items.len(),
                "[pipeline-store] Parsed items"
            );

            let parsed_count = items.len();
            let mut stored_count = 0usize;
            let mut new_count = 0usize;
            let mut skipped_count = 0usize;
            let mut errors = Vec::new();
            let tool_name = &step.tool_name;

            if let Some(ref db_action) = output_config.db_action {
                for item in &items {
                    let mut item = item.clone();
                    if !item.fields.contains_key("host") && !item.fields.contains_key("ip") && !item.fields.contains_key("url") {
                        item.fields.insert("host".to_string(), target.to_string());
                    }
                    if db_action == "target_add" {
                        match store_target_from_item(pool, &item, project_path, parent_target_id).await {
                            Ok(is_new) => { stored_count += 1; if is_new { new_count += 1; } }
                            Err(e) => {
                                tracing::warn!(tool = %step.tool_name, error = %e, "[pipeline-store] Store error");
                                skipped_count += 1;
                                if errors.len() < 5 { errors.push(e); }
                            }
                        }
                        continue;
                    }
                    let result = match db_action.as_str() {
                        "target_update_recon" => store_recon_from_item(pool, &item, project_path).await,
                        "directory_entry_add" => store_dirent_from_item(pool, &item, tool_name, project_path).await,
                        "finding_add" => store_finding_from_item(pool, &item, tool_name, project_path).await,
                        _ => { skipped_count += 1; continue; }
                    };
                    match result {
                        Ok(()) => { stored_count += 1; new_count += 1; }
                        Err(e) => {
                            tracing::warn!(tool = %step.tool_name, error = %e, "[pipeline-store] Store error");
                            skipped_count += 1;
                            if errors.len() < 5 { errors.push(e); }
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
            total_stored += stored_count;
            Some(StoreStats { parsed_count, stored_count, new_count, skipped_count, errors })
        } else {
            tracing::debug!(tool = %step.tool_name, "[pipeline-store] No output config found");
            None
        };

        if step.step_type == "web_crawl" && exit_code == Some(0) && !stdout.is_empty() {
            let urls: Vec<String> = stdout.lines()
                .filter(|l| l.starts_with("http://") || l.starts_with("https://"))
                .map(|l| l.trim().to_string())
                .collect();
            if !urls.is_empty() {
                tracing::info!(count = urls.len(), "[pipeline] Merging katana URLs into sitemap");
                merge_urls_into_sitemap(pool, &urls, project_path).await;
                emit_pipeline_event(app, &PipelineEvent {
                    pipeline_id: pipeline_id.clone(),
                    run_id: run_id.clone(),
                    step_id: "sitemap_merge".to_string(),
                    step_index: idx,
                    total_steps,
                    status: "info".to_string(),
                    tool_name: "katana".to_string(),
                    message: Some(format!("Merged {} URLs into sitemap", urls.len())),
                    store_stats: None,
                    pipeline_name: None, target: None, all_steps: None,
                    output: None, duration_ms: None, exit_code: None,
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
            pipeline_id: pipeline_id.clone(),
            run_id: run_id.clone(),
            step_id: step.id.clone(),
            step_index: idx,
            total_steps,
            status: if exit_code == Some(0) { "completed".to_string() } else { "error".to_string() },
            tool_name: step.tool_name.clone(),
            message: Some(format!(
                "exit={}, lines={}, stored={}",
                exit_code.unwrap_or(-1),
                stdout.lines().count(),
                store_stats.as_ref().map(|s| s.stored_count).unwrap_or(0),
            )),
            store_stats: store_stats.clone(),
            pipeline_name: None, target: None, all_steps: None,
            output: truncated_output,
            duration_ms: Some(duration_ms),
            exit_code,
        });

        step_results.push(StepResult {
            step_id: step.id.clone(),
            tool_name: step.tool_name.clone(),
            command: last_cmd_str,
            exit_code,
            stdout_lines: stdout.lines().count(),
            stderr_preview: stderr.chars().take(500).collect(),
            store_stats,
            duration_ms,
        });

        // Only skip if this step explicitly depends on a failed upstream step.
        // Independent steps (no input_from, different exec_mode) continue running.
        if exit_code != Some(0) && step.exec_mode == "on_success" {
            break;
        }
    }

    let _ = std::fs::remove_dir_all(&tmp_dir);

    let total_duration_ms = start.elapsed().as_millis() as u64;
    let completed_steps = step_results.iter().filter(|s| s.exit_code == Some(0)).count();
    let failed_steps = step_results.iter().filter(|s| s.exit_code.is_some() && s.exit_code != Some(0)).count();

    let resolved_target_id = if parent_target_id.is_some() {
        parent_target_id
    } else {
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM targets WHERE value = $1 AND project_path IS NOT DISTINCT FROM $2 LIMIT 1",
        )
        .bind(target)
        .bind(project_path)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
    };

    let _ = golish_db::repo::audit::log_operation(
        pool, "pipeline_executed", "recon",
        &format!("Pipeline '{}' on {}: {}/{} steps completed, {} items stored",
            pipeline.name, target, completed_steps, total_steps, total_stored),
        project_path, "pipeline",
        resolved_target_id, None, Some(&pipeline.name),
        if failed_steps == 0 { "completed" } else { "partial" },
        &serde_json::json!({
            "pipeline_id": pipeline_id,
            "total_steps": total_steps,
            "completed_steps": completed_steps,
            "failed_steps": failed_steps,
            "total_stored": total_stored,
            "total_new": step_results.iter().filter_map(|s| s.store_stats.as_ref()).map(|st| st.new_count).sum::<usize>(),
            "duration_ms": total_duration_ms,
        }),
    ).await;

    if total_stored > 0 {
        if let Some(app) = app {
            let _ = app.emit("targets-changed", serde_json::json!({
                "source": "pipeline",
                "target": target,
                "stored": total_stored,
            }));
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

/// Merge crawler-discovered URLs into the ZAP sitemap (topology_scans).
/// Only appends entries whose dedup key (method:host:path) doesn't already exist.
async fn merge_urls_into_sitemap(
    pool: &sqlx::PgPool,
    urls: &[String],
    project_path: Option<&str>,
) {
    if urls.is_empty() { return; }
    let pp = project_path.filter(|s| !s.is_empty());

    let existing: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT data FROM topology_scans WHERE name = 'zap-sitemap' AND project_path IS NOT DISTINCT FROM $1",
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
        "DELETE FROM topology_scans WHERE name = 'zap-sitemap' AND project_path IS NOT DISTINCT FROM $1",
    )
    .bind(pp)
    .execute(pool)
    .await;

    let _ = sqlx::query(
        r#"INSERT INTO topology_scans (name, data, project_path)
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
}
