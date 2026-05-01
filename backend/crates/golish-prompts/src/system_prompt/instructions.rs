use std::path::Path;

use golish_core::AgentMode;

/// Get agent mode-specific instructions to append to the system prompt.
pub fn get_agent_mode_instructions(mode: AgentMode) -> String {
    match mode {
        AgentMode::Planning => r#"

<planning_mode>
# Planning Mode Active

You are in READ-ONLY mode. You may investigate and plan, but NOT execute changes.

**Allowed**:
- `read_file`, `list_files`, `list_directory`, `grep_file`, `find_files`
- `ast_grep` (structural code search)
- `indexer_*` tools (all analysis tools)
- `web_search`, `web_fetch` (research)
- `update_plan` (creating plans)
- Delegating to `explorer`, `analyzer`, `researcher`

**Forbidden**:
- `edit_file`, `write_file`, `create_file`, `delete_file`
- `run_command` (except read-only commands like `git status`, `ls`)
- `apply_patch`, `execute_code`
- Delegating to `executor`

When you have a complete plan, present it and wait for the user to switch to execution mode.
</planning_mode>
"#
        .to_string(),
        AgentMode::AutoApprove => r#"

<autoapprove_mode>
# AutoApprove Mode Active

All tool operations will be automatically approved. Exercise additional caution:
- Double-check destructive operations (delete, overwrite)
- Verify you have the correct file paths
- Run verification after changes
</autoapprove_mode>
"#
        .to_string(),
        AgentMode::Default => String::new(),
    }
}

/// Read project instructions from a memory file.
///
/// # Arguments
/// * `workspace_path` - The current workspace directory
/// * `memory_file_path` - Optional explicit path to a memory file (from codebase settings)
///
/// # Behavior
/// - If `memory_file_path` is provided (from codebase settings), reads from that file.
///   If the file doesn't exist, returns an error message.
/// - If `memory_file_path` is None (no codebase configured or no memory file set),
///   returns empty string (no project instructions).
pub fn read_project_instructions(workspace_path: &Path, memory_file_path: Option<&Path>) -> String {
    // If a memory file path is configured, use it.
    if let Some(path) = memory_file_path {
        // Handle relative paths (just filename like "CLAUDE.md").
        let full_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            workspace_path.join(path)
        };

        if full_path.exists() {
            match std::fs::read_to_string(&full_path) {
                Ok(contents) => return contents.trim().to_string(),
                Err(e) => {
                    tracing::warn!("Failed to read memory file {:?}: {}", full_path, e);
                    return format!(
                        "The {} memory file could not be read. Update in settings.",
                        path.display()
                    );
                }
            }
        } else {
            return format!(
                "The {} memory file not found. Update in settings.",
                path.display()
            );
        }
    }

    String::new()
}
