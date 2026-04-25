//! Small helpers shared across the executor: epoch time, tool classification,
//! file-path extraction.

pub(crate) fn epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Check if a tool modifies files
pub(crate) fn is_write_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "write_file"
            | "create_file"
            | "edit_file"
            | "delete_file"
            | "delete_path"
            | "rename_file"
            | "move_file"
            | "move_path"
            | "copy_path"
            | "create_directory"
            | "apply_patch"
            | "ast_grep_replace"
    )
}

/// Extract file path from tool arguments
pub(crate) fn extract_file_path(tool_name: &str, args: &serde_json::Value) -> Option<String> {
    match tool_name {
        "write_file" | "create_file" | "edit_file" | "read_file" | "delete_file" => args
            .get("path")
            .or_else(|| args.get("file_path"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        "apply_patch" => {
            // Extract file paths from patch content
            args.get("patch")
                .and_then(|v| v.as_str())
                .and_then(|patch| {
                    // Look for "*** Update File:" or "*** Add File:" lines
                    for line in patch.lines() {
                        if let Some(path) = line.strip_prefix("*** Update File:") {
                            return Some(path.trim().to_string());
                        }
                        if let Some(path) = line.strip_prefix("*** Add File:") {
                            return Some(path.trim().to_string());
                        }
                    }
                    None
                })
        }
        "rename_file" | "move_file" | "move_path" | "copy_path" => args
            .get("destination")
            .or_else(|| args.get("to"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        "delete_path" => args
            .get("path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        "create_directory" => args
            .get("path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        _ => None,
    }
}
