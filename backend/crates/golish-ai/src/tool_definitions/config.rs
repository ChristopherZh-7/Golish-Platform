//! Tool selection configuration: presets plus optional add/disable overrides.

use serde::Deserialize;

use super::preset::ToolPreset;

/// Configuration for tool selection with optional overrides.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ToolConfig {
    /// Base preset to use.
    #[serde(default)]
    pub preset: ToolPreset,
    /// Additional tools to enable (on top of preset).
    #[serde(default)]
    pub additional: Vec<String>,
    /// Tools to disable (removed from preset).
    #[serde(default)]
    pub disabled: Vec<String>,
}

impl ToolConfig {
    /// Create a new config with the given preset.
    pub fn with_preset(preset: ToolPreset) -> Self {
        Self {
            preset,
            additional: vec![],
            disabled: vec![],
        }
    }

    /// Create the default tool config for the main agent.
    ///
    /// This is the recommended configuration for golish's primary AI agent.
    /// It uses the [`ToolPreset::Standard`] preset with additional tools that
    /// are useful for the main agent but not sub-agents.
    ///
    /// Sub-agents are added dynamically from the registry and don't need to
    /// be listed here.
    pub fn main_agent() -> Self {
        Self {
            preset: ToolPreset::Standard,
            additional: vec![
                // Code execution for complex operations
                "execute_code".to_string(),
                // Patch-based editing for large changes
                "apply_patch".to_string(),
                // Tavily-powered web tools (requires API key + settings.tools.web_search = true)
                "tavily_search".to_string(),
                "tavily_search_answer".to_string(),
                "tavily_extract".to_string(),
                "tavily_crawl".to_string(),
                "tavily_map".to_string(),
                // Vulnerability knowledge base
                "search_knowledge_base".to_string(),
                "write_knowledge".to_string(),
                "read_knowledge".to_string(),
                "ingest_cve".to_string(),
                // Security analysis
                "log_operation".to_string(),
                "discover_apis".to_string(),
                "save_js_analysis".to_string(),
                "fingerprint_target".to_string(),
                "log_scan_result".to_string(),
                "query_target_data".to_string(),
            ],
            // Hide run_pty_cmd — we expose it as run_command instead.
            disabled: vec!["run_pty_cmd".to_string()],
        }
    }

    /// Check if a tool name is enabled by this config.
    ///
    /// Disabled list takes precedence over additional list, which takes
    /// precedence over the preset's allow-list. Full preset accepts all
    /// tools that aren't explicitly disabled.
    pub fn is_tool_enabled(&self, tool_name: &str) -> bool {
        if self.disabled.iter().any(|t| t == tool_name) {
            return false;
        }

        if self.additional.iter().any(|t| t == tool_name) {
            return true;
        }

        match self.preset.tool_names() {
            Some(names) => names.contains(&tool_name),
            None => true,
        }
    }
}
