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

pub use golish_pentest::models::OutputConfig as OutputParserConfig;
pub use golish_pentest::models::OutputPattern as PatternConfig;

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

use golish_core::utils::resolve_json_path;

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
    golish_core::paths::toolsconfig_dir().ok_or_else(|| "cannot resolve home directory".to_string())
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
// Parse + Store pipeline: parses tool output and routes to DB by db_action.
// Storage functions are shared from golish_pentest::output_store.
// ============================================================================

use crate::state::AppState;

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

    use golish_pentest::output_store;
    for item in &items {
        let result = match db_action.as_str() {
            "target_add" => output_store::store_target_add(pool, &item.fields, pp).await,
            "target_update_recon" => {
                output_store::store_target_update_recon(pool, &item.fields, pp, tname).await
            }
            "directory_entry_add" => {
                output_store::store_directory_entry(pool, &item.fields, tname, pp).await
            }
            "finding_add" => {
                output_store::store_finding(pool, &item.fields, tname, pp).await
            }
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

/// Thin wrapper for backward compatibility with callers passing `ParsedItem`.
pub async fn store_recon_fingerprints(
    pool: &sqlx::PgPool,
    target_id: uuid::Uuid,
    project_path: Option<&str>,
    item: &ParsedItem,
    source: &str,
) {
    golish_pentest::output_store::store_fingerprints(pool, target_id, project_path, &item.fields, source).await;
}
