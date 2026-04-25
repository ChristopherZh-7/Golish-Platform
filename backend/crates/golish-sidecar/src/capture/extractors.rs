//! Extracting structured data from raw tool args/results: tool output,
//! affected file paths, and rename operations.

use std::path::PathBuf;

use super::context::MAX_TOOL_OUTPUT_LEN;
use super::format::truncate;


/// Extract tool output from result
pub(super) fn extract_tool_output(result: &serde_json::Value) -> Option<String> {
    // Try different output formats
    let output = if let Some(s) = result.as_str() {
        s.to_string()
    } else if let Some(content) = result.get("content").and_then(|v| v.as_str()) {
        content.to_string()
    } else if let Some(output) = result.get("output").and_then(|v| v.as_str()) {
        output.to_string()
    } else {
        // Serialize the whole thing
        serde_json::to_string(result).ok()?
    };

    // Truncate if needed
    Some(truncate(&output, MAX_TOOL_OUTPUT_LEN).to_string())
}

/// Extract files from a tool result
pub(super) fn extract_files_from_result(
    tool_name: &str,
    args: &Option<serde_json::Value>,
    _result: &serde_json::Value,
) -> Option<Vec<PathBuf>> {
    match tool_name {
        "read_file" => {
            let args = args.as_ref()?;
            let path = extract_path_from_args(args)?;
            Some(vec![path])
        }
        "list_files" | "list_directory" => {
            let args = args.as_ref()?;
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .map(PathBuf::from)?;
            Some(vec![path])
        }
        "grep" | "find_path" => {
            // These tools return multiple files in results, but we just track the search
            None
        }
        _ => None,
    }
}

/// Extract path from tool args (supports all vtcode-core path aliases)
pub(super) fn extract_path_from_args(args: &serde_json::Value) -> Option<PathBuf> {
    args.get("path")
        .or_else(|| args.get("file_path"))
        .or_else(|| args.get("filepath"))
        .or_else(|| args.get("target_path"))
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
}

/// Extract files modified from tool args
pub(super) fn extract_files_modified(tool_name: &str, args: Option<&serde_json::Value>) -> Vec<PathBuf> {
    let args = match args {
        Some(a) => a,
        None => return vec![],
    };

    match tool_name {
        "write_file" | "create_file" | "edit_file" | "delete_file" | "delete_path" => {
            if let Some(path) = extract_path_from_args(args) {
                vec![path]
            } else {
                vec![]
            }
        }
        "rename_file" | "move_file" | "move_path" => {
            let mut files = vec![];
            if let Some(from) = args
                .get("source_path")
                .or_else(|| args.get("from"))
                .and_then(|v| v.as_str())
            {
                files.push(PathBuf::from(from));
            }
            if let Some(to) = args
                .get("destination_path")
                .or_else(|| args.get("to"))
                .and_then(|v| v.as_str())
            {
                files.push(PathBuf::from(to));
            }
            files
        }
        "copy_path" => {
            if let Some(dest) = args.get("destination_path").and_then(|v| v.as_str()) {
                vec![PathBuf::from(dest)]
            } else {
                vec![]
            }
        }
        "create_directory" => {
            if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
                vec![PathBuf::from(path)]
            } else {
                vec![]
            }
        }
        _ => vec![],
    }
}
