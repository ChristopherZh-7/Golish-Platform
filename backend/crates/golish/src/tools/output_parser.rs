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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternConfig {
    #[serde(rename = "type")]
    pub data_type: String,
    pub regex: String,
    pub fields: HashMap<String, String>,
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
