//! `list_files` tool: glob pattern matching using `ignore::WalkBuilder`.

use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;
use golish_core::utils::{get_optional_bool, get_optional_str};
use golish_core::Tool;
use ignore::WalkBuilder;
use serde_json::{json, Value};

use crate::path_policy::{is_within_workspace, join_workspace as resolve_path};

/// Tool for listing files matching a glob pattern.
pub struct ListFilesTool;

#[async_trait]
impl Tool for ListFilesTool {
    fn name(&self) -> &'static str {
        "list_files"
    }

    fn description(&self) -> &'static str {
        "List files matching a glob pattern. Respects .gitignore by default."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Directory to search (relative to workspace, default: root)"
                },
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match files (e.g., '*.rs', '**/*.ts')"
                },
                "recursive": {
                    "type": "boolean",
                    "description": "Search recursively (default: true)"
                }
            },
            "required": []
        })
    }

    async fn execute(&self, args: Value, workspace: &Path) -> Result<Value> {
        let path_str = get_optional_str(&args, "path").unwrap_or(".");
        let pattern = get_optional_str(&args, "pattern");
        let recursive = get_optional_bool(&args, "recursive").unwrap_or(true);

        let search_dir = resolve_path(path_str, workspace);

        // Check if directory exists
        if !search_dir.exists() {
            return Ok(json!({"error": format!("Directory not found: {}", path_str)}));
        }

        if !search_dir.is_dir() {
            return Ok(json!({"error": format!("Path is not a directory: {}", path_str)}));
        }

        // Check if within workspace
        if !is_within_workspace(&search_dir, workspace) {
            return Ok(json!({"error": format!("Path is outside workspace: {}", path_str)}));
        }

        // Build glob pattern matcher if provided
        let glob_matcher = pattern.and_then(|p| glob::Pattern::new(p).ok());

        // Walk the directory
        let mut files: Vec<String> = Vec::new();
        let max_depth = if recursive { None } else { Some(1) };

        let walker = WalkBuilder::new(&search_dir)
            .max_depth(max_depth)
            .hidden(false) // Don't ignore hidden files
            .git_ignore(true) // Respect .gitignore
            .git_global(true)
            .git_exclude(true)
            .build();

        for entry in walker.flatten() {
            let path = entry.path();

            // Skip directories
            if path.is_dir() {
                continue;
            }

            // Get path relative to workspace
            let relative = match path.strip_prefix(workspace) {
                Ok(r) => r.to_string_lossy().to_string(),
                Err(_) => continue,
            };

            // Apply glob pattern if provided
            if let Some(ref matcher) = glob_matcher {
                let file_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();

                // Match against filename or full path
                if !matcher.matches(&file_name) && !matcher.matches(&relative) {
                    continue;
                }
            }

            files.push(relative);
        }

        // Sort for consistent output
        files.sort();

        Ok(json!({
            "files": files,
            "count": files.len(),
            "path": path_str
        }))
    }
}
