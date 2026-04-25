//! Public entry points for selecting and filtering tool definitions.
//!
//! Tools are pulled from [`golish_tools::build_function_declarations`],
//! filtered through a [`super::config::ToolConfig`], and have their schemas
//! sanitised by [`super::sanitize::sanitize_schema`] for LLM provider
//! compatibility.

use std::collections::HashSet;

use golish_core::ToolName;
use golish_tools::build_function_declarations;
use rig::completion::ToolDefinition;

use super::config::ToolConfig;
use super::preset::ToolPreset;
use super::sanitize::sanitize_schema;

/// Get tool definitions using the default Standard preset.
///
/// This is the recommended entry point for most use cases.
#[allow(dead_code)] // Public API — used externally or for future internal use.
pub fn get_standard_tool_definitions() -> Vec<ToolDefinition> {
    get_tool_definitions_with_config(&ToolConfig::default())
}

/// Get tool definitions with a specific preset.
#[allow(dead_code)] // Public API — used externally or for future internal use.
pub fn get_tool_definitions_for_preset(preset: ToolPreset) -> Vec<ToolDefinition> {
    get_tool_definitions_with_config(&ToolConfig::with_preset(preset))
}

/// Get tool definitions with full configuration control.
///
/// Filters function declarations based on the provided config, sanitises
/// schemas for Anthropic compatibility, and applies description overrides.
pub fn get_tool_definitions_with_config(config: &ToolConfig) -> Vec<ToolDefinition> {
    build_function_declarations()
        .into_iter()
        .filter(|fd| config.is_tool_enabled(&fd.name))
        .map(|fd| {
            // Override description for run_pty_cmd to instruct the agent
            // not to repeat the user-visible terminal output in its reply.
            let description = if ToolName::from_str(&fd.name) == Some(ToolName::RunPtyCmd) {
                format!(
                    "{}. IMPORTANT: The command output is displayed directly in the user's terminal. \
                     Do NOT repeat or summarize the command output in your response - the user can already see it. \
                     Only mention significant errors or ask clarifying questions if needed.",
                    fd.description
                )
            } else {
                fd.description
            };

            ToolDefinition {
                name: fd.name,
                description,
                parameters: sanitize_schema(fd.parameters),
            }
        })
        .collect()
}

/// Get all tool definitions using the specified config.
///
/// Alias of [`get_tool_definitions_with_config`] kept for clarity at call
/// sites that want to convey "every tool the config enables".
pub fn get_all_tool_definitions_with_config(config: &ToolConfig) -> Vec<ToolDefinition> {
    get_tool_definitions_with_config(config)
}

/// Get all available tool definitions (uses [`ToolPreset::Full`]).
pub fn get_all_tool_definitions() -> Vec<ToolDefinition> {
    get_tool_definitions_with_config(&ToolConfig::with_preset(ToolPreset::Full))
}

/// Filter tools by allow-list. An empty allow-list returns the input
/// unchanged so callers don't accidentally hide everything.
pub fn filter_tools_by_allowed(
    tools: Vec<ToolDefinition>,
    allowed_tools: &[String],
) -> Vec<ToolDefinition> {
    if allowed_tools.is_empty() {
        tools
    } else {
        let allowed_set: HashSet<&str> = allowed_tools.iter().map(|s| s.as_str()).collect();
        tools
            .into_iter()
            .filter(|t| allowed_set.contains(t.name.as_str()))
            .collect()
    }
}
