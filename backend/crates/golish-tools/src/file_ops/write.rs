//! `write_file` tool: overwrites or appends to an existing file.
use std::fs;
use std::path::Path;

use anyhow::Result;
use serde_json::{json, Value};

use golish_core::Tool;
use golish_core::utils::get_required_str;

use crate::path_policy::resolve_path_checked as resolve_path;



// ============================================================================
// write_file
// ============================================================================

/// Tool for writing file contents (creates or overwrites).
pub struct WriteFileTool;

#[async_trait::async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &'static str {
        "write_file"
    }

    fn description(&self) -> &'static str {
        "Write content to a file, replacing existing content. Creates the file and parent directories if they don't exist."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file (relative to workspace)"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, args: Value, workspace: &Path) -> Result<Value> {
        let path_str = match get_required_str(&args, "path") {
            Ok(p) => p,
            Err(e) => return Ok(e),
        };

        let content = match get_required_str(&args, "content") {
            Ok(c) => c,
            Err(e) => return Ok(e),
        };

        let resolved = match resolve_path(path_str, workspace) {
            Ok(p) => p,
            Err(e) => return Ok(json!({"error": e})),
        };

        // Check if it's a directory
        if resolved.is_dir() {
            return Ok(json!({"error": format!("Path is a directory: {}", path_str)}));
        }

        // Create parent directories if needed
        if let Some(parent) = resolved.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                return Ok(json!({"error": format!("Failed to create parent directories: {}", e)}));
            }
        }

        // Write the file
        match fs::write(&resolved, content) {
            Ok(()) => Ok(json!({
                "success": true,
                "path": path_str,
                "bytes_written": content.len()
            })),
            Err(e) => Ok(json!({"error": format!("Failed to write file: {}", e)})),
        }
    }
}
