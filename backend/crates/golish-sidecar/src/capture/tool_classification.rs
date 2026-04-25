//! Classifying tool names by what they do (read / write / edit).


/// Check if tool is a read operation
pub(super) fn is_read_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "read_file" | "list_files" | "list_directory" | "grep" | "find_path" | "diagnostics"
    )
}

/// Check if tool is a write operation
pub(super) fn is_write_tool(tool_name: &str) -> bool {
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
            | "ast_grep_replace"
    )
}

/// Check if tool is an edit operation (for diff generation)
pub(super) fn is_edit_tool(tool_name: &str) -> bool {
    matches!(tool_name, "edit_file" | "write_file" | "create_file")
}
