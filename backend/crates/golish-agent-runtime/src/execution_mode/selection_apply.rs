//! Apply a [`super::policy::ToolSelection`] against the live agentic-loop
//! context to produce a `Vec<rig::completion::ToolDefinition>`.
//!
//! Pure async function (no mutation of the context) so it is fully unit
//! testable. All "what tools should be exposed" decisions live in the
//! Policy; this module is just the mechanical filter+merge that turns
//! flags into tool definitions.

use std::collections::HashSet;

use golish_agent_kit::tool_definitions::{
    get_all_tool_definitions_with_config, get_ask_human_tool_definition,
    get_run_command_tool_definition, get_sub_agent_tool_definitions, sanitize_schema,
};
use golish_sub_agents::{SubAgentContext, MAX_AGENT_DEPTH};

use super::policy::ToolSelection;
use crate::agentic_loop::AgenticLoopContext;

pub async fn apply_tool_selection(
    selection: ToolSelection,
    ctx: &AgenticLoopContext<'_>,
    sub_agent_context: &SubAgentContext,
) -> Vec<rig::completion::ToolDefinition> {
    let mut tools: Vec<rig::completion::ToolDefinition> = Vec::new();

    if ctx.tool_config.is_none_preset() {
        tracing::debug!(
            "[tool_list] ToolPreset::None active (depth={}): exposing no tools",
            sub_agent_context.depth
        );
        return tools;
    }

    // 1. Static tool groups via existing ToolConfig + ToolPreset filter.
    //    The Policy decides whether to include any static tools at all;
    //    the existing ToolConfig still narrows by tool name within the
    //    enabled groups.
    if selection.static_groups.any_enabled() {
        tools.extend(get_all_tool_definitions_with_config(ctx.tool_config));
    }

    // 2. run_command (the user-visible alias of run_pty_cmd).
    if selection.include_run_command {
        tools.push(get_run_command_tool_definition());
    }

    // 3. ask_human is only meaningful for the primary agent (depth==0).
    //    Sub-agents must not block on user input.
    if selection.include_ask_human && sub_agent_context.depth == 0 {
        tools.push(get_ask_human_tool_definition());
    }

    // 4. MCP / additional pre-built tool definitions (already
    //    sanitised by their owners).
    tools.extend(ctx.additional_tool_definitions.iter().cloned());

    // 5. Dynamic registry tools — pentest_bridge / pentest_runtime / tavily.
    //    This is the section that historically dropped pentest_bridge
    //    tools in chat mode because the legacy filter only matched
    //    `pentest_*` prefixes. Now we drive inclusion off the explicit
    //    BridgeToolSelection allow-list from the Policy.
    let registry = ctx.tool_registry.read().await;
    let registry_tools = registry.get_tool_definitions();
    drop(registry);

    let bridge_allowed: HashSet<&'static str> = selection
        .bridge_tools
        .enabled_tool_names()
        .into_iter()
        .collect();
    let existing: HashSet<String> = tools.iter().map(|t| t.name.clone()).collect();

    for tool in registry_tools {
        if existing.contains(&tool.name) {
            continue;
        }
        let include = if bridge_allowed.contains(tool.name.as_str()) {
            true
        } else if tool.name.starts_with("pentest_") {
            selection.runtime_tools.pentest_runtime
        } else if tool.name.starts_with("tavily_") {
            selection.runtime_tools.tavily && ctx.tool_config.is_tool_enabled(&tool.name)
        } else {
            false
        };

        if include {
            tools.push(rig::completion::ToolDefinition {
                name: tool.name,
                description: tool.description,
                parameters: sanitize_schema(tool.parameters),
            });
        }
    }

    // 6. Sub-agent dispatch tools — only when the policy enables them
    //    and we still have agent-depth budget.
    if selection.agent_tools.include_dispatch_tools && sub_agent_context.depth + 1 < MAX_AGENT_DEPTH
    {
        let registry = ctx.sub_agent_registry.read().await;
        let mut sub_tools = get_sub_agent_tool_definitions(&registry).await;
        // Orchestrator is always pipeline-only — never expose to the LLM.
        sub_tools.retain(|t| t.name != "sub_agent_orchestrator");
        if !selection.agent_tools.allow_planner {
            sub_tools.retain(|t| t.name != "sub_agent_planner");
        }
        if !selection.agent_tools.allow_refiner {
            sub_tools.retain(|t| t.name != "sub_agent_refiner");
        }
        if !selection.agent_tools.allow_reflector {
            sub_tools.retain(|t| t.name != "sub_agent_reflector");
        }
        tools.extend(sub_tools);
    }

    // 7. Apply deny_overrides last (e.g. update_plan in subtask mode).
    if !selection.deny_overrides.is_empty() {
        let denied: HashSet<&str> = selection
            .deny_overrides
            .iter()
            .map(|s| s.as_str())
            .collect();
        tools.retain(|t| !denied.contains(t.name.as_str()));
    }

    tracing::debug!(
        "[tool_list] policy-driven tools (depth={}): {:?}",
        sub_agent_context.depth,
        tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>()
    );

    tools
}
