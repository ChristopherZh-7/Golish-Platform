//! Tool definitions and executors used by the LLM judge when
//! workspace-exploration is enabled.

use std::path::Path;

use rig::completion::ToolDefinition;
use serde::Deserialize;
use serde_json::json;

/// Tool arguments deserialised from a tool call.
#[derive(Debug, Deserialize)]
pub(super) struct PathArg {
    pub(super) path: String,
}

/// Build tool definitions for the judge.
pub(super) fn build_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "read_file".to_string(),
            description: "Read the contents of a file from the workspace. Use this to verify \
                 actual code changes, check file contents, or examine implementation details."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to read, relative to the workspace root."
                    }
                },
                "required": ["path"]
            }),
        },
        ToolDefinition {
            name: "list_files".to_string(),
            description: "List files and directories in a path. Directories end with '/'. \
                 Use this to discover what files exist in the workspace."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path relative to workspace root. Use '.' for root."
                    }
                },
                "required": ["path"]
            }),
        },
    ]
}

/// Execute the `read_file` tool, refusing paths outside the workspace.
pub(super) fn execute_read_file(workspace: &Path, path: &str) -> String {
    let full_path = workspace.join(path);

    let canonical = match full_path.canonicalize() {
        Ok(p) => p,
        Err(e) => return format!("Error: Cannot resolve path '{}': {}", path, e),
    };
    let workspace_canonical = match workspace.canonicalize() {
        Ok(p) => p,
        Err(e) => return format!("Error: Cannot resolve workspace: {}", e),
    };

    if !canonical.starts_with(&workspace_canonical) {
        return format!("Error: Path '{}' is outside the workspace", path);
    }

    match std::fs::read_to_string(&canonical) {
        Ok(content) => content,
        Err(e) => format!("Error: Cannot read '{}': {}", path, e),
    }
}

/// Execute the `list_files` tool, refusing paths outside the workspace.
pub(super) fn execute_list_files(workspace: &Path, path: &str) -> String {
    let full_path = workspace.join(path);

    let canonical = match full_path.canonicalize() {
        Ok(p) => p,
        Err(e) => return format!("Error: Cannot resolve path '{}': {}", path, e),
    };
    let workspace_canonical = match workspace.canonicalize() {
        Ok(p) => p,
        Err(e) => return format!("Error: Cannot resolve workspace: {}", e),
    };

    if !canonical.starts_with(&workspace_canonical) {
        return format!("Error: Path '{}' is outside the workspace", path);
    }

    match std::fs::read_dir(&canonical) {
        Ok(entries) => {
            let files: Vec<String> = entries
                .filter_map(|entry| entry.ok())
                .map(|entry| {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if entry.path().is_dir() {
                        format!("{}/", name)
                    } else {
                        name
                    }
                })
                .collect();
            files.join("\n")
        }
        Err(e) => format!("Error: Cannot read directory '{}': {}", path, e),
    }
}
