//! Preset levels for the agent's tool surface.

use serde::Deserialize;

/// Tool preset levels for different use cases.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolPreset {
    /// No tools at all (for non-agentic sessions like title generation).
    None,
    /// Minimal tools: read, edit, write files + shell command.
    Minimal,
    /// Standard tools for most development tasks (default).
    #[default]
    Standard,
    /// All registered tools.
    Full,
}

impl ToolPreset {
    /// Get the list of tool names for this preset.
    ///
    /// Returns `None` for [`ToolPreset::Full`] which means *all registered
    /// tools* (no allow-listing applied).
    pub fn tool_names(&self) -> Option<Vec<&'static str>> {
        match self {
            ToolPreset::None => Some(vec![]),
            ToolPreset::Minimal => {
                Some(vec!["read_file", "edit_file", "write_file", "run_pty_cmd"])
            }
            ToolPreset::Standard => Some(vec![
                // Search & discovery
                "grep_file",
                "list_files",
                // Structural code search & replace (AST-based)
                "ast_grep",
                "ast_grep_replace",
                // File operations
                "read_file",
                "create_file",
                "edit_file",
                "write_file",
                "delete_file",
                // Shell execution
                "run_pty_cmd",
                // Web
                "web_fetch",
                // Planning
                "update_plan",
            ]),
            ToolPreset::Full => None,
        }
    }
}
