//! `edit_file` tool: targeted string replacements with returned diff.
use std::fs;
use std::path::Path;

use anyhow::Result;
use serde_json::{json, Value};

use golish_core::Tool;
use golish_core::utils::{get_optional_str, get_required_str};

use crate::path_policy::resolve_path_checked as resolve_path;



// ============================================================================
// edit_file
// ============================================================================

/// Tool for editing a file by search/replace.
pub struct EditFileTool;

#[async_trait::async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &'static str {
        "edit_file"
    }

    fn description(&self) -> &'static str {
        "Edit a file by replacing text. The old_text must match exactly once in the file."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file (relative to workspace)"
                },
                "old_text": {
                    "type": "string",
                    "description": "Text to find and replace (must match exactly once)"
                },
                "new_text": {
                    "type": "string",
                    "description": "Replacement text"
                },
                "display_description": {
                    "type": "string",
                    "description": "Human-readable description of the edit"
                }
            },
            "required": ["path", "old_text", "new_text"]
        })
    }

    async fn execute(&self, args: Value, workspace: &Path) -> Result<Value> {
        let path_str = match get_required_str(&args, "path") {
            Ok(p) => p,
            Err(e) => return Ok(e),
        };

        let old_text = match get_required_str(&args, "old_text") {
            Ok(t) => t,
            Err(e) => return Ok(e),
        };

        let new_text = match get_required_str(&args, "new_text") {
            Ok(t) => t,
            Err(e) => return Ok(e),
        };

        let description = get_optional_str(&args, "display_description");

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

        // Read the file
        let content = match fs::read_to_string(&resolved) {
            Ok(c) => c,
            Err(e) => return Ok(json!({"error": format!("Failed to read file: {}", e)})),
        };

        // Count occurrences
        let match_count = content.matches(old_text).count();

        if match_count == 0 {
            return Ok(json!({
                "error": "Edit failed: no matches found for the search text",
                "search_text": old_text,
                "suggestion": "Verify the exact text to replace, including whitespace and line endings"
            }));
        }

        if match_count > 1 {
            return Ok(json!({
                "error": format!("Edit failed: found {} matches, expected exactly 1", match_count),
                "match_count": match_count,
                "suggestion": "Provide more context to make the match unique"
            }));
        }

        // Perform the replacement
        let new_content = content.replacen(old_text, new_text, 1);

        // Generate diff preview
        let diff = generate_diff(&content, &new_content);

        // Write the file
        match fs::write(&resolved, &new_content) {
            Ok(()) => {
                let mut result = json!({
                    "success": true,
                    "path": path_str,
                    "diff": diff
                });
                if let Some(desc) = description {
                    result["description"] = json!(desc);
                }
                Ok(result)
            }
            Err(e) => Ok(json!({"error": format!("Failed to write file: {}", e)})),
        }
    }
}

/// Generate a simple unified diff between old and new content.
fn generate_diff(old: &str, new: &str) -> String {
    use similar::{ChangeTag, TextDiff};

    let diff = TextDiff::from_lines(old, new);
    let mut result = String::new();

    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
        };
        result.push_str(sign);
        result.push_str(change.value());
        if !change.value().ends_with('\n') {
            result.push('\n');
        }
    }

    result
}
