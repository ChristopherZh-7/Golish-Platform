//! `read_file` tool: reads UTF-8 file contents with optional line-range slicing.
use std::fs;
use std::path::Path;

use anyhow::Result;
use serde_json::{json, Value};

use golish_core::Tool;
use golish_core::utils::{get_optional_i64, get_required_str};

use crate::path_policy::resolve_path_checked as resolve_path;

use super::helpers::is_binary_file;


// ============================================================================
// read_file
// ============================================================================

/// Tool for reading file contents.
pub struct ReadFileTool;

#[async_trait::async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &'static str {
        "read_file"
    }

    fn description(&self) -> &'static str {
        "Read the contents of a file. Supports optional line range for reading specific sections."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file (relative to workspace)"
                },
                "line_start": {
                    "type": "integer",
                    "description": "Starting line number (1-indexed)"
                },
                "line_end": {
                    "type": "integer",
                    "description": "Ending line number (1-indexed, inclusive)"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value, workspace: &Path) -> Result<Value> {
        let path_str = match get_required_str(&args, "path") {
            Ok(p) => p,
            Err(e) => return Ok(e),
        };

        let resolved = match resolve_path(path_str, workspace) {
            Ok(p) => p,
            Err(e) => return Ok(json!({"error": e})),
        };

        // Check if file exists
        if !resolved.exists() {
            return Ok(json!({
                "error": format!("File not found: {}", path_str),
                "resolved_path": resolved.display().to_string(),
                "workspace": workspace.display().to_string(),
                "hint": "If workspace is wrong, the terminal cwd may not be synced"
            }));
        }

        // Check if it's a directory
        if resolved.is_dir() {
            return Ok(json!({"error": format!("Path is a directory: {}", path_str)}));
        }

        // Read raw bytes first to check for binary
        let bytes = match fs::read(&resolved) {
            Ok(b) => b,
            Err(e) => return Ok(json!({"error": format!("Failed to read file: {}", e)})),
        };

        if is_binary_file(&bytes) {
            return Ok(json!({"error": format!("Cannot read binary file: {}", path_str)}));
        }

        // Convert to string
        let content = match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) => return Ok(json!({"error": format!("File is not valid UTF-8: {}", e)})),
        };

        // Apply line range if specified
        let line_start = get_optional_i64(&args, "line_start").map(|n| n as usize);
        let line_end = get_optional_i64(&args, "line_end").map(|n| n as usize);

        let result_content = match (line_start, line_end) {
            (Some(start), Some(end)) => {
                let lines: Vec<&str> = content.lines().collect();
                let start_idx = start.saturating_sub(1); // Convert to 0-indexed
                let end_idx = end.min(lines.len());
                if start_idx >= lines.len() {
                    return Ok(json!({
                        "error": format!("Line {} is beyond end of file ({} lines)", start, lines.len())
                    }));
                }
                lines[start_idx..end_idx].join("\n")
            }
            (Some(start), None) => {
                let lines: Vec<&str> = content.lines().collect();
                let start_idx = start.saturating_sub(1);
                if start_idx >= lines.len() {
                    return Ok(json!({
                        "error": format!("Line {} is beyond end of file ({} lines)", start, lines.len())
                    }));
                }
                lines[start_idx..].join("\n")
            }
            (None, Some(end)) => {
                let lines: Vec<&str> = content.lines().collect();
                let end_idx = end.min(lines.len());
                lines[..end_idx].join("\n")
            }
            (None, None) => content,
        };

        Ok(json!({
            "content": result_content,
            "path": path_str
        }))
    }
}
