//! `delete_file` tool: removes a file (and optionally its empty parent directories).
use std::fs;
use std::path::Path;

use anyhow::Result;
use serde_json::{json, Value};

use golish_core::Tool;
use golish_core::utils::get_required_str;

use crate::path_policy::resolve_path_checked as resolve_path;



// ============================================================================
// delete_file
// ============================================================================

/// Tool for deleting a file.
pub struct DeleteFileTool;

#[async_trait::async_trait]
impl Tool for DeleteFileTool {
    fn name(&self) -> &'static str {
        "delete_file"
    }

    fn description(&self) -> &'static str {
        "Delete a file from the filesystem."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to delete (relative to workspace)"
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
            return Ok(json!({"error": format!("Path is a directory, not a file: {}", path_str)}));
        }

        // Delete the file
        match fs::remove_file(&resolved) {
            Ok(()) => Ok(json!({
                "success": true,
                "path": path_str
            })),
            Err(e) => Ok(json!({"error": format!("Failed to delete file: {}", e)})),
        }
    }
}

