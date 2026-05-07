//! Pure parsing helpers used by step executors.
//!
//! These are the standalone parts of the legacy `tools/output_parser.rs`
//! module — regex/JSON extraction with no side effects. The Tauri
//! commands (`output_parse`, `output_detect_tool`, `output_parse_and_store`)
//! stay in the main crate and re-export what they need from here.

use std::collections::HashMap;

use regex::Regex;
use serde::{Deserialize, Serialize};
use tracing::debug;

pub use golish_pentest::models::OutputConfig as OutputParserConfig;
pub use golish_pentest::models::OutputPattern as PatternConfig;

/// Extract the hostname from a URL; returns the input unchanged if it
/// is already a bare host. Used by storage callbacks to key off the host
/// portion of heterogeneous input fields (url / host / ip).
pub fn extract_hostname(val: &str) -> String {
    if val.starts_with("http://") || val.starts_with("https://") {
        url::Url::parse(val)
            .ok()
            .and_then(|u| u.host_str().map(|s| s.to_string()))
            .unwrap_or_else(|| val.to_string())
    } else {
        val.to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedItem {
    pub data_type: String,
    pub fields: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreStats {
    pub parsed_count: usize,
    pub stored_count: usize,
    #[serde(default)]
    pub new_count: usize,
    pub skipped_count: usize,
    pub errors: Vec<String>,
}

/// Run a `jq` expression against raw output to transform it before parsing.
/// Falls back to the original output on any error — callers should keep
/// working with `raw` if `jq` is missing or returns an error.
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
            tracing::warn!("[pipeline-parser] jq not available: {e}");
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
                "[pipeline-parser] jq transform: {} bytes → {} bytes",
                raw.len(),
                transformed.len()
            );
            transformed
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!("[pipeline-parser] jq failed: {stderr}");
            raw.to_string()
        }
        Err(e) => {
            tracing::warn!("[pipeline-parser] jq exec error: {e}");
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

pub fn parse_text(raw: &str, patterns: &[PatternConfig]) -> Vec<ParsedItem> {
    let mut items = Vec::new();
    for pattern in patterns {
        let re = match Regex::new(&pattern.regex) {
            Ok(r) => r,
            Err(e) => {
                debug!("[pipeline-parser] Invalid regex '{}': {}", pattern.regex, e);
                continue;
            }
        };
        for caps in re.captures_iter(raw) {
            let mut fields = HashMap::new();
            for (field_name, group_ref) in &pattern.fields {
                let value = if let Ok(idx) = group_ref
                    .strip_prefix('$')
                    .unwrap_or(group_ref)
                    .parse::<usize>()
                {
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

pub fn parse_json_lines(raw: &str, field_mappings: &HashMap<String, String>) -> Vec<ParsedItem> {
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
            if let Some(val) = golish_core::utils::resolve_json_path(&obj, json_path) {
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

pub fn parse_json(raw: &str, field_mappings: &HashMap<String, String>) -> Vec<ParsedItem> {
    let val: serde_json::Value = match serde_json::from_str(raw.trim()) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    if let Some(arr) = val.as_array() {
        arr.iter()
            .filter_map(|obj| {
                let mut fields = HashMap::new();
                for (field_name, json_path) in field_mappings {
                    if let Some(v) = golish_core::utils::resolve_json_path(obj, json_path) {
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
            if let Some(v) = golish_core::utils::resolve_json_path(&val, json_path) {
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
