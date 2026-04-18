use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::debug;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedItem {
    pub data_type: String,
    pub fields: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseResult {
    pub tool_id: String,
    pub tool_name: String,
    pub items: Vec<ParsedItem>,
    pub produces: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputParserConfig {
    pub format: String,
    pub produces: Vec<String>,
    #[serde(default)]
    pub patterns: Vec<PatternConfig>,
    #[serde(default)]
    pub fields: HashMap<String, String>,
    #[serde(default)]
    pub detect: Option<String>,
    #[serde(default)]
    pub db_action: Option<String>,
    /// Optional jq expression to pre-process tool output before parsing.
    /// Runs `echo $stdout | jq '$transform'` and replaces stdout with the result.
    #[serde(default)]
    pub transform: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternConfig {
    #[serde(rename = "type")]
    pub data_type: String,
    pub regex: String,
    pub fields: HashMap<String, String>,
}

/// Run a jq expression against raw output to transform it before parsing.
/// Falls back to the original output on any error.
pub async fn transform_with_jq(raw: &str, jq_expr: &str) -> String {
    let result = tokio::process::Command::new("jq")
        .arg("-c")
        .arg(jq_expr)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();

    let mut child = match result {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("[output_parser] jq not available: {e}");
            return raw.to_string();
        }
    };

    if let Some(ref mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        let _ = stdin.write_all(raw.as_bytes()).await;
    }

    match child.wait_with_output().await {
        Ok(output) if output.status.success() => {
            let transformed = String::from_utf8_lossy(&output.stdout).to_string();
            tracing::debug!(
                "[output_parser] jq transform: {} bytes → {} bytes",
                raw.len(),
                transformed.len()
            );
            transformed
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!("[output_parser] jq failed: {stderr}");
            raw.to_string()
        }
        Err(e) => {
            tracing::warn!("[output_parser] jq exec error: {e}");
            raw.to_string()
        }
    }
}

pub fn parse_text_standalone(raw: &str, patterns: &[PatternConfig]) -> Vec<ParsedItem> {
    parse_text(raw, patterns)
}

pub fn parse_json_standalone(
    raw: &str,
    field_mappings: &HashMap<String, String>,
    is_json_lines: bool,
) -> Vec<ParsedItem> {
    if is_json_lines {
        parse_json_lines(raw, field_mappings)
    } else {
        parse_json(raw, field_mappings)
    }
}

fn parse_text(raw: &str, patterns: &[PatternConfig]) -> Vec<ParsedItem> {
    let mut items = Vec::new();
    for pattern in patterns {
        let re = match Regex::new(&pattern.regex) {
            Ok(r) => r,
            Err(e) => {
                debug!("[output_parser] Invalid regex '{}': {}", pattern.regex, e);
                continue;
            }
        };
        for caps in re.captures_iter(raw) {
            let mut fields = HashMap::new();
            for (field_name, group_ref) in &pattern.fields {
                let value = if let Ok(idx) = group_ref.strip_prefix('$').unwrap_or(group_ref).parse::<usize>() {
                    caps.get(idx).map(|m| m.as_str().to_string())
                } else {
                    caps.name(group_ref).map(|m| m.as_str().to_string())
                };
                if let Some(v) = value {
                    fields.insert(field_name.clone(), v);
                }
            }
            if !fields.is_empty() {
                items.push(ParsedItem {
                    data_type: pattern.data_type.clone(),
                    fields,
                });
            }
        }
    }
    items
}

fn parse_json_lines(raw: &str, field_mappings: &HashMap<String, String>) -> Vec<ParsedItem> {
    let mut items = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || !line.starts_with('{') {
            continue;
        }
        let obj: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let mut fields = HashMap::new();
        for (field_name, json_path) in field_mappings {
            if let Some(val) = resolve_json_path(&obj, json_path) {
                fields.insert(field_name.clone(), val);
            }
        }
        if !fields.is_empty() {
            items.push(ParsedItem {
                data_type: "auto".to_string(),
                fields,
            });
        }
    }
    items
}

fn parse_json(raw: &str, field_mappings: &HashMap<String, String>) -> Vec<ParsedItem> {
    let val: serde_json::Value = match serde_json::from_str(raw.trim()) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    if let Some(arr) = val.as_array() {
        arr.iter()
            .filter_map(|obj| {
                let mut fields = HashMap::new();
                for (field_name, json_path) in field_mappings {
                    if let Some(v) = resolve_json_path(obj, json_path) {
                        fields.insert(field_name.clone(), v);
                    }
                }
                if fields.is_empty() {
                    None
                } else {
                    Some(ParsedItem {
                        data_type: "auto".to_string(),
                        fields,
                    })
                }
            })
            .collect()
    } else {
        let mut fields = HashMap::new();
        for (field_name, json_path) in field_mappings {
            if let Some(v) = resolve_json_path(&val, json_path) {
                fields.insert(field_name.clone(), v);
            }
        }
        if fields.is_empty() {
            Vec::new()
        } else {
            vec![ParsedItem {
                data_type: "auto".to_string(),
                fields,
            }]
        }
    }
}

/// Simple JSONPath-like resolver: supports "$.foo.bar" and "$.foo[0].bar" patterns.
fn resolve_json_path(val: &serde_json::Value, path: &str) -> Option<String> {
    let path = path.strip_prefix("$.").unwrap_or(path);
    let mut current = val;
    for segment in path.split('.') {
        if let Some(idx_start) = segment.find('[') {
            let key = &segment[..idx_start];
            let idx_str = &segment[idx_start + 1..segment.len() - 1];
            current = current.get(key)?;
            let idx: usize = idx_str.parse().ok()?;
            current = current.get(idx)?;
        } else {
            current = current.get(segment)?;
        }
    }
    match current {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        serde_json::Value::Null => None,
        other => Some(other.to_string()),
    }
}

#[tauri::command]
pub async fn output_parse(
    raw_output: String,
    config: OutputParserConfig,
    tool_id: Option<String>,
    tool_name: Option<String>,
) -> Result<ParseResult, String> {
    let items = match config.format.as_str() {
        "text" => parse_text(&raw_output, &config.patterns),
        "json_lines" => parse_json_lines(&raw_output, &config.fields),
        "json" => parse_json(&raw_output, &config.fields),
        other => return Err(format!("Unsupported format: {other}")),
    };

    debug!(
        "[output_parse] Parsed {} items from {} output",
        items.len(),
        config.format
    );

    Ok(ParseResult {
        tool_id: tool_id.unwrap_or_default(),
        tool_name: tool_name.unwrap_or_default(),
        items,
        produces: config.produces,
    })
}

fn toolsconfig_dir() -> Result<std::path::PathBuf, String> {
    let home = dirs::home_dir().ok_or("cannot resolve home directory")?;
    #[cfg(target_os = "macos")]
    let dir = home
        .join("Library")
        .join("Application Support")
        .join("golish-platform")
        .join("toolsconfig");
    #[cfg(target_os = "windows")]
    let dir = home
        .join("AppData")
        .join("Local")
        .join("golish-platform")
        .join("toolsconfig");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let dir = home.join(".golish-platform").join("toolsconfig");
    Ok(dir)
}

#[tauri::command]
pub async fn output_detect_tool(
    command: String,
    raw_output: String,
) -> Result<Option<serde_json::Value>, String> {
    let tools_dir = toolsconfig_dir()?;

    if !tools_dir.exists() {
        return Ok(None);
    }

    for entry in walkdir::WalkDir::new(&tools_dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            if let Ok(data) = std::fs::read_to_string(path) {
                if let Ok(config_file) = serde_json::from_str::<serde_json::Value>(&data) {
                    if let Some(output) = config_file.pointer("/tool/output") {
                        if let Ok(output_cfg) = serde_json::from_value::<OutputParserConfig>(output.clone()) {
                            if let Some(ref detect) = output_cfg.detect {
                                if let Ok(re) = Regex::new(detect) {
                                    if re.is_match(&command) || re.is_match(&raw_output) {
                                        let tool_id = config_file
                                            .pointer("/tool/id")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string();
                                        let tool_name = config_file
                                            .pointer("/tool/name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string();
                                        let result = serde_json::json!({
                                            "tool_id": tool_id,
                                            "tool_name": tool_name,
                                            "output_config": output_cfg,
                                        });
                                        return Ok(Some(result));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(None)
}

// ============================================================================
// Parse + Store pipeline: parses tool output and routes to DB by db_action
// ============================================================================

use crate::state::AppState;
use crate::tools::targets::{
    db_directory_entry_add, db_target_add, db_target_update_recon_extended, ReconUpdate,
};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreStats {
    pub parsed_count: usize,
    pub stored_count: usize,
    #[serde(default)]
    pub new_count: usize,
    pub skipped_count: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseAndStoreResult {
    pub parse_result: ParseResult,
    pub store_stats: StoreStats,
}

#[tauri::command]
pub async fn output_parse_and_store(
    state: tauri::State<'_, AppState>,
    raw_output: String,
    config: OutputParserConfig,
    tool_id: Option<String>,
    tool_name: Option<String>,
    project_path: Option<String>,
) -> Result<ParseAndStoreResult, String> {
    let items = match config.format.as_str() {
        "text" => parse_text(&raw_output, &config.patterns),
        "json_lines" => parse_json_lines(&raw_output, &config.fields),
        "json" => parse_json(&raw_output, &config.fields),
        other => return Err(format!("Unsupported format: {other}")),
    };

    let parse_result = ParseResult {
        tool_id: tool_id.clone().unwrap_or_default(),
        tool_name: tool_name.clone().unwrap_or_default(),
        items: items.clone(),
        produces: config.produces.clone(),
    };

    let parsed_count = items.len();
    let mut stored_count = 0usize;
    let mut skipped_count = 0usize;
    let mut errors: Vec<String> = Vec::new();

    let Some(ref db_action) = config.db_action else {
        return Ok(ParseAndStoreResult {
            parse_result,
            store_stats: StoreStats {
                parsed_count,
                stored_count: 0,
                new_count: 0,
                skipped_count: parsed_count,
                errors: vec![],
            },
        });
    };

    let pool = state.db_pool_ready().await?;
    let pp = project_path.as_deref();
    let tname = tool_name.as_deref().unwrap_or("unknown");

    for item in &items {
        let result = match db_action.as_str() {
            "target_add" => store_target_add(pool, item, pp).await,
            "target_update_recon" => store_target_update_recon(pool, item, pp).await,
            "directory_entry_add" => store_directory_entry(pool, item, tname, pp).await,
            "finding_add" => store_finding(pool, item, tname, pp).await,
            other => {
                skipped_count += 1;
                errors.push(format!("Unknown db_action: {other}"));
                continue;
            }
        };

        match result {
            Ok(()) => stored_count += 1,
            Err(e) => {
                skipped_count += 1;
                errors.push(e);
            }
        }
    }

    debug!(
        "[output_parse_and_store] db_action={}, parsed={}, stored={}, skipped={}",
        db_action, parsed_count, stored_count, skipped_count
    );

    Ok(ParseAndStoreResult {
        parse_result,
        store_stats: StoreStats {
            parsed_count,
            stored_count,
            new_count: stored_count,
            skipped_count,
            errors,
        },
    })
}

async fn store_target_add(
    pool: &sqlx::PgPool,
    item: &ParsedItem,
    project_path: Option<&str>,
) -> Result<(), String> {
    let hostname = item
        .fields
        .get("hostname")
        .or_else(|| item.fields.get("host"))
        .or_else(|| item.fields.get("ip"))
        .ok_or("No hostname/host/ip field in parsed record")?;

    db_target_add(pool, hostname, hostname, None, project_path, "discovered", None).await?;
    Ok(())
}

async fn store_target_update_recon(
    pool: &sqlx::PgPool,
    item: &ParsedItem,
    project_path: Option<&str>,
) -> Result<(), String> {
    let host_val = item
        .fields
        .get("host")
        .or_else(|| item.fields.get("ip"))
        .or_else(|| item.fields.get("url"))
        .ok_or("No host/ip/url field for recon update")?;

    let target = find_or_create_target(pool, host_val, project_path).await?;
    let target_uuid: Uuid = target.id.parse().map_err(|e: uuid::Error| e.to_string())?;

    let mut update = ReconUpdate::new();

    if let Some(ip) = item.fields.get("ip") {
        update.real_ip = ip.clone();
    }
    if let Some(cdn) = item.fields.get("cdn") {
        update.cdn_waf = cdn.clone();
    }
    if let Some(title) = item.fields.get("title") {
        update.http_title = title.clone();
    }
    if let Some(status) = item.fields.get("status_code").or_else(|| item.fields.get("status")) {
        update.http_status = status.parse().ok();
    }
    if let Some(ws) = item.fields.get("webserver") {
        update.webserver = ws.clone();
    }
    if let Some(os) = item.fields.get("os") {
        update.os_info = os.clone();
    }
    if let Some(ct) = item.fields.get("content_type") {
        update.content_type = ct.clone();
    }

    // Build port entries with embedded HTTP metadata when available
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

    db_target_update_recon_extended(pool, target_uuid, &update).await?;
    Ok(())
}

async fn store_directory_entry(
    pool: &sqlx::PgPool,
    item: &ParsedItem,
    tool_name: &str,
    project_path: Option<&str>,
) -> Result<(), String> {
    let url = item
        .fields
        .get("url")
        .ok_or("No url field in directory entry")?;

    let status: Option<i32> = item
        .fields
        .get("status")
        .and_then(|s| s.parse().ok());
    let size: Option<i32> = item
        .fields
        .get("size")
        .or_else(|| item.fields.get("content_length"))
        .and_then(|s| s.parse().ok());
    let lines: Option<i32> = item.fields.get("lines").and_then(|s| s.parse().ok());
    let words: Option<i32> = item.fields.get("words").and_then(|s| s.parse().ok());

    db_directory_entry_add(pool, None, url, status, size, lines, words, tool_name, project_path)
        .await?;
    Ok(())
}

async fn store_finding(
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
    let references = item.fields.get("reference").cloned().unwrap_or_default();

    let refs_json: serde_json::Value = if references.is_empty() {
        serde_json::json!([])
    } else if let Ok(arr) = serde_json::from_str::<serde_json::Value>(&references) {
        arr
    } else {
        serde_json::json!([references])
    };

    let sev = match severity.to_lowercase().as_str() {
        "critical" => "critical",
        "high" => "high",
        "medium" => "medium",
        "low" => "low",
        _ => "info",
    };

    sqlx::query(
        r#"INSERT INTO findings (title, sev, url, target, description, tool, template, refs, project_path)
           VALUES ($1, $2::severity, $3, $4, $5, $6, $7, $8, $9)
           ON CONFLICT DO NOTHING"#,
    )
    .bind(&title)
    .bind(sev)
    .bind(&url)
    .bind(&url)
    .bind(&description)
    .bind(tool_name)
    .bind(&template)
    .bind(&refs_json)
    .bind(project_path)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

async fn find_or_create_target(
    pool: &sqlx::PgPool,
    host_val: &str,
    project_path: Option<&str>,
) -> Result<super::targets::Target, String> {
    // Try to extract hostname from URL
    let hostname = if host_val.starts_with("http://") || host_val.starts_with("https://") {
        url::Url::parse(host_val)
            .ok()
            .and_then(|u| u.host_str().map(|s| s.to_string()))
            .unwrap_or_else(|| host_val.to_string())
    } else {
        host_val.to_string()
    };

    // Search for existing target by value
    let existing = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM targets WHERE value = $1 AND project_path = $2 LIMIT 1",
    )
    .bind(&hostname)
    .bind(project_path)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    if let Some(_) = existing {
        // Return existing
        let targets = super::targets::db_target_list(pool, project_path).await?;
        return targets
            .into_iter()
            .find(|t| t.value == hostname)
            .ok_or_else(|| "Target disappeared".to_string());
    }

    db_target_add(pool, &hostname, &hostname, None, project_path, "discovered", None).await
}
