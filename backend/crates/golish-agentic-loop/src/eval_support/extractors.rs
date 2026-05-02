//! Helpers shared by the eval runners: extracting structured tool calls /
//! modified files from the captured event stream, plus a verbose human-friendly
//! printer used when `EvalConfig::verbose` is on.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use golish_core::events::AiEvent;

use super::types::EvalToolCall;

/// Extract tool calls and modified files from captured events.
///
/// This function processes the event stream to build:
/// 1. A list of all tool calls with their inputs and outputs
/// 2. A list of files that were modified by write operations
pub(super) fn extract_tool_calls_and_files(
    events: &[AiEvent],
    workspace: &Path,
) -> (Vec<EvalToolCall>, Vec<PathBuf>) {
    // Map from request_id to tool args (captured from ToolAutoApproved)
    let mut args_by_request: HashMap<String, serde_json::Value> = HashMap::new();

    // First pass: collect args from ToolAutoApproved events
    for event in events {
        if let AiEvent::ToolAutoApproved {
            request_id, args, ..
        } = event
        {
            args_by_request.insert(request_id.clone(), args.clone());
        }
    }

    let mut tool_calls = Vec::new();
    let mut files_modified = Vec::new();

    // Second pass: build tool calls from ToolResult events
    for event in events {
        if let AiEvent::ToolResult {
            tool_name,
            result,
            success,
            request_id,
            ..
        } = event
        {
            // Get args from the corresponding ToolAutoApproved event
            let input = args_by_request
                .get(request_id)
                .cloned()
                .unwrap_or(serde_json::Value::Null);

            tool_calls.push(EvalToolCall {
                name: tool_name.clone(),
                input: input.clone(),
                output: Some(serde_json::to_string(result).unwrap_or_default()),
                success: *success,
            });

            // Track files modified by write operations
            if *success && is_write_tool(tool_name) {
                if let Some(path) = extract_file_path(tool_name, &input) {
                    let full_path = workspace.join(&path);
                    if !files_modified.contains(&full_path) {
                        files_modified.push(full_path);
                    }
                }
            }
        }
    }

    (tool_calls, files_modified)
}

/// Check if a tool modifies files.
fn is_write_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "write_file" | "create_file" | "edit_file" | "delete_file" | "ast_grep_replace"
    )
}

/// Extract file path from tool arguments.
fn extract_file_path(tool_name: &str, args: &serde_json::Value) -> Option<String> {
    match tool_name {
        "write_file" | "create_file" | "edit_file" | "delete_file" => args
            .get("path")
            .or_else(|| args.get("file_path"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        _ => None,
    }
}

/// Print an event in verbose mode.
pub(super) fn print_event_verbose(event: &AiEvent) {
    match event {
        AiEvent::TextDelta { delta, .. } => {
            // Print text deltas without newline for streaming effect
            eprint!("{}", delta);
        }
        AiEvent::Reasoning { content } => {
            eprintln!(
                "\n\x1b[90m[Thinking] {}\x1b[0m",
                truncate_string(content, 200)
            );
        }
        AiEvent::ToolApprovalRequest {
            tool_name, args, ..
        } => {
            let args_preview = truncate_string(&format!("{}", args), 100);
            eprintln!("\n\x1b[33m[Tool] {} {}\x1b[0m", tool_name, args_preview);
        }
        AiEvent::ToolAutoApproved {
            tool_name, args, ..
        } => {
            let args_preview = truncate_string(&format!("{}", args), 100);
            eprintln!("\n\x1b[36m[Tool] {} {}\x1b[0m", tool_name, args_preview);
        }
        AiEvent::ToolResult {
            tool_name,
            success,
            result,
            ..
        } => {
            let status = if *success {
                "\x1b[32m✓\x1b[0m"
            } else {
                "\x1b[31m✗\x1b[0m"
            };
            let result_preview = truncate_string(&format!("{}", result), 150);
            eprintln!("  {} {} → {}", status, tool_name, result_preview);
        }
        AiEvent::Completed {
            input_tokens,
            output_tokens,
            ..
        } => {
            if let (Some(input), Some(output)) = (input_tokens, output_tokens) {
                eprintln!(
                    "\n\x1b[90m[Tokens] input={}, output={}\x1b[0m",
                    input, output
                );
            }
        }
        AiEvent::Error { message, .. } => {
            eprintln!("\n\x1b[31m[Error] {}\x1b[0m", message);
        }
        _ => {} // Ignore other events
    }
}

/// Truncate a string for display (UTF-8 safe).
fn truncate_string(s: &str, max_chars: usize) -> String {
    let s = s.replace('\n', " ").replace('\r', "");
    // Use char_indices to find valid UTF-8 boundaries
    if s.chars().count() > max_chars {
        let end_idx = s
            .char_indices()
            .nth(max_chars)
            .map(|(idx, _)| idx)
            .unwrap_or(s.len());
        format!("{}...", &s[..end_idx])
    } else {
        s
    }
}

