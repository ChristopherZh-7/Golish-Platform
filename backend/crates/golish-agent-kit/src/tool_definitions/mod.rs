//! Tool definitions for the agent system.
//!
//! ## Layout
//!
//! - [`preset`]: [`preset::ToolPreset`] enum + per-preset allow-lists.
//! - [`config`]: [`config::ToolConfig`] (preset + add/disable overrides) and
//!   the [`config::ToolConfig::main_agent`] factory.
//! - [`sanitize`]: [`sanitize::sanitize_schema`] — recursive JSON Schema
//!   transformer for OpenAI strict mode + Anthropic compatibility.
//! - [`definitions`]: hand-rolled tool descriptors (`run_command`,
//!   `ask_human`, registry-driven `sub_agent_*` shims).
//! - [`selection`]: the public `get_*_tool_definitions*` family that
//!   composes preset filtering, schema sanitising, and per-tool description
//!   overrides; plus [`selection::filter_tools_by_allowed`].
//!
//! ## Tool Selection
//!
//! Tools can be filtered using presets or custom configuration:
//! - [`preset::ToolPreset::Minimal`] — essential file operations only.
//! - [`preset::ToolPreset::Standard`] — core development tools (default).
//! - [`preset::ToolPreset::Full`] — all registered tools.
//!
//! Use [`config::ToolConfig`] to override presets with custom allow/block lists.

mod config;
mod definitions;
mod preset;
mod sanitize;
mod selection;

#[cfg(test)]
mod tests;

pub use config::ToolConfig;
pub use definitions::{
    get_ask_human_tool_definition, get_run_command_tool_definition, get_sub_agent_tool_definitions,
};
pub use preset::ToolPreset;
pub use sanitize::sanitize_schema;
pub use selection::{
    filter_tools_by_allowed, get_all_tool_definitions, get_all_tool_definitions_with_config,
    get_standard_tool_definitions, get_tool_definitions_for_preset,
    get_tool_definitions_with_config,
};
