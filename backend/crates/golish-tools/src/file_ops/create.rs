//! `create_file` tool: creates a new file, fails on existing path.
use std::fs;
use std::path::Path;

use anyhow::Result;
use serde_json::{json, Value};

use golish_core::Tool;
use golish_core::utils::get_required_str;

use crate::path_policy::is_in_temp_dir;



// ============================================================================
// create_file
// ============================================================================

/// Tool for creating a new file (fails if file exists).
pub struct CreateFileTool;

#[async_trait::async_trait]
impl Tool for CreateFileTool {
    fn name(&self) -> &'static str {
        "create_file"
    }

    fn description(&self) -> &'static str {
        "Create a new file with the specified content. Fails if the file already exists."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path for the new file (relative to workspace)"
                },
                "content": {
                    "type": "string",
                    "description": "Initial content for the file"
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

        // For create_file, we need to handle non-existent parent directories
        let path = Path::new(path_str);
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            workspace.join(path)
        };

        // Check if file already exists
        if resolved.exists() {
            return Ok(json!({"error": format!("File already exists: {}", path_str)}));
        }

        // Create parent directories if needed
        if let Some(parent) = resolved.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                return Ok(json!({"error": format!("Failed to create parent directories: {}", e)}));
            }
        }

        // Allow writes to temp directories without workspace check
        if !is_in_temp_dir(&resolved) {
            let workspace_canonical = match workspace.canonicalize() {
                Ok(p) => p,
                Err(e) => return Ok(json!({"error": format!("Cannot resolve workspace: {}", e)})),
            };

            let parent_canonical = match resolved.parent().and_then(|p| p.canonicalize().ok()) {
                Some(p) => p,
                None => return Ok(json!({"error": "Invalid path: no parent directory"})),
            };

            if !parent_canonical.starts_with(&workspace_canonical) {
                return Ok(json!({"error": format!("Path '{}' is outside workspace", path_str)}));
            }
        }

        // Write the file
        match fs::write(&resolved, content) {
            Ok(()) => Ok(json!({
                "success": true,
                "path": path_str,
                "bytes_written": content.len()
            })),
            Err(e) => Ok(json!({"error": format!("Failed to create file: {}", e)})),
        }
    }
}
