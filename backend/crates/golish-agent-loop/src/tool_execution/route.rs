//! Routing layer: pick which direct executor handles a given tool name.
//!
//! [`route_tool_execution`] is the single public entry point. It uses
//! [`ToolRoutingCategory::from_tool_name`] to bucket the tool, then
//! delegates to the matching helper in [`super::direct`].

use serde_json::Value;

use golish_core::ToolName;

use super::direct::{
    execute_plan_tool_routed, execute_registry_tool, execute_sub_agent_placeholder,
    execute_web_fetch_tool_routed, ToolExecutionError, ToolExecutionResult,
};
use super::hitl::{ToolExecutionConfig, ToolExecutionContext};

/// Identifies which category a tool belongs to based on its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolRoutingCategory {
    /// Web fetch tool (readability extraction).
    WebFetch,
    /// Plan update tool.
    UpdatePlan,
    /// Sub-agent delegation tool.
    SubAgent,
    /// Standard registry-based tool.
    Registry,
}

impl ToolRoutingCategory {
    /// Categorize a tool by its name.
    ///
    /// Uses [`ToolName`] for type-safe matching where possible, falling back
    /// to string prefix matching for dynamic tools (`sub_agent_*`).
    pub fn from_tool_name(name: &str) -> Self {
        if let Some(tool) = ToolName::from_str(name) {
            return Self::from_known_tool(tool);
        }

        if ToolName::is_sub_agent_tool(name) {
            Self::SubAgent
        } else {
            Self::Registry
        }
    }

    /// Categorize a known tool by its [`ToolName`] enum.
    pub fn from_known_tool(tool: ToolName) -> Self {
        match tool {
            // Indexer tools route through the registry.
            ToolName::IndexerSearchCode
            | ToolName::IndexerSearchFiles
            | ToolName::IndexerAnalyzeFile
            | ToolName::IndexerExtractSymbols
            | ToolName::IndexerGetMetrics
            | ToolName::IndexerDetectLanguage => Self::Registry,

            // Web fetch (special handling, not registry-based).
            ToolName::WebFetch => Self::WebFetch,

            // Plan update.
            ToolName::UpdatePlan => Self::UpdatePlan,

            // Everything else goes through the registry.
            _ => Self::Registry,
        }
    }
}

/// Route tool execution to the appropriate handler.
///
/// This is the main entry point for tool execution. It categorises the tool
/// by name and delegates to the matching helper in [`super::direct`].
///
/// # Arguments
///
/// * `tool_name` — name of the tool to execute.
/// * `tool_args` — arguments to pass to the tool.
/// * `ctx` — context providing access to tool dependencies.
/// * `config` — configuration for tool execution behaviour (HITL, source,
///   sub-agent permissions).
///
/// # Returns
///
/// `Ok(ToolExecutionResult)` on success, or `Err(ToolExecutionError)` on
/// failure.
///
/// # Tool routing
///
/// Tools are routed based on their name prefix:
/// - `indexer_*` → registry tools (code search, file analysis).
/// - `web_fetch` → readability-aware fetcher.
/// - `update_plan` → task planning updates.
/// - `sub_agent_*` → sub-agent delegation (gated on
///   `config.allow_sub_agents`).
/// - `run_command` → mapped to `run_pty_cmd` by the registry layer.
/// - Everything else → registry-based execution.
pub async fn route_tool_execution(
    tool_name: &str,
    tool_args: &Value,
    ctx: &ToolExecutionContext<'_>,
    config: &ToolExecutionConfig,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    let category = ToolRoutingCategory::from_tool_name(tool_name);

    tracing::debug!(
        tool = %tool_name,
        category = ?category,
        source = ?config.source,
        "Routing tool execution"
    );

    match category {
        ToolRoutingCategory::WebFetch => execute_web_fetch_tool_routed(tool_name, tool_args).await,

        ToolRoutingCategory::UpdatePlan => {
            execute_plan_tool_routed(ctx.plan_manager, tool_args).await
        }

        ToolRoutingCategory::SubAgent => {
            if !config.allow_sub_agents {
                return Err(ToolExecutionError::ToolNotAllowed(format!(
                    "Sub-agent tools not allowed from {:?}",
                    config.source
                )));
            }
            execute_sub_agent_placeholder(ctx.sub_agent_registry, tool_name, tool_args).await
        }

        ToolRoutingCategory::Registry => {
            execute_registry_tool(ctx.tool_registry, tool_name, tool_args).await
        }
    }
}
