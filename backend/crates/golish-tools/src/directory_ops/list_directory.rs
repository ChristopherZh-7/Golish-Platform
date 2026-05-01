//! `list_directory` tool: shallow directory listing.

use std::fs;
use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;
use golish_core::Tool;
use golish_core::utils::get_required_str;
use serde_json::{json, Value};

use crate::path_policy::{join_workspace as resolve_path, is_within_workspace};

/// Tool for listing directory contents.
pub struct ListDirectoryTool;

#[async_trait]
impl Tool for ListDirectoryTool {
    fn name(&self) -> &'static str {
        "list_directory"
    }

    fn description(&self) -> &'static str {
        "List the contents of a directory with file/directory type indicators."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Directory path (relative to workspace)"
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

        let dir_path = resolve_path(path_str, workspace);

        // Check if exists
        if !dir_path.exists() {
            return Ok(json!({"error": format!("Directory not found: {}", path_str)}));
        }

        // Check if it's a directory
        if !dir_path.is_dir() {
            return Ok(json!({"error": format!("Path is not a directory: {}", path_str)}));
        }

        // Check if within workspace
        if !is_within_workspace(&dir_path, workspace) {
            return Ok(json!({"error": format!("Path is outside workspace: {}", path_str)}));
        }

        // Read directory contents
        let entries = match fs::read_dir(&dir_path) {
            Ok(e) => e,
            Err(e) => return Ok(json!({"error": format!("Failed to read directory: {}", e)})),
        };

        let mut items: Vec<Value> = Vec::new();

        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let entry_path = entry.path();
            let is_dir = entry_path.is_dir();
            let is_symlink = entry_path.is_symlink();

            let entry_type = if is_symlink {
                "symlink"
            } else if is_dir {
                "directory"
            } else {
                "file"
            };

            // Get file size for files
            let size = if !is_dir {
                entry_path.metadata().ok().map(|m| m.len())
            } else {
                None
            };

            let mut item = json!({
                "name": name,
                "type": entry_type
            });

            if let Some(s) = size {
                item["size"] = json!(s);
            }

            items.push(item);
        }

        // Sort by name
        items.sort_by(|a, b| {
            let a_name = a["name"].as_str().unwrap_or("");
            let b_name = b["name"].as_str().unwrap_or("");
            a_name.cmp(b_name)
        });

        Ok(json!({
            "entries": items,
            "count": items.len(),
            "path": path_str
        }))
    }
}
