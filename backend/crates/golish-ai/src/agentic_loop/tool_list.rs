//! Build the per-turn list of `rig::completion::ToolDefinition`s to expose to
//! the model.
//!
//! The available tools depend on the execution mode and sub-agent depth:
//!
//! | mode  | depth        | exposed tools                                    |
//! |-------|--------------|---------------------------------------------------|
//! | task  | 0 (primary)  | `update_plan` + knowledge tools + ask_human       |
//! | task  | >0 (subtask) | full toolset minus `update_plan` + ask_human      |
//! | chat  | any          | full toolset + ask_human                          |
//!
//! Sub-agent dispatch tools are appended whenever depth + 1 < `MAX_AGENT_DEPTH`.

use std::collections::HashSet;

use golish_sub_agents::{SubAgentContext, MAX_AGENT_DEPTH};

use super::context::AgenticLoopContext;
use super::super::tool_definitions::{
    get_all_tool_definitions_with_config, get_ask_human_tool_definition,
    get_run_command_tool_definition, get_sub_agent_tool_definitions, sanitize_schema,
};

/// Build the list of tool definitions exposed to the model for one turn.
pub(super) async fn build_tool_list(
    ctx: &AgenticLoopContext<'_>,
    sub_agent_context: &SubAgentContext,
) -> Vec<rig::completion::ToolDefinition> {
    // Task mode: delegation-only (sub-agents + planning + ask_human) — matches
    // PentAGI primary agent. Chat mode: full tool set (file, shell, web, ...).
    let is_task_primary = ctx.execution_mode.is_task() && sub_agent_context.depth == 0;
    let is_task_subtask = ctx.execution_mode.is_task() && sub_agent_context.depth > 0;

    let mut tools: Vec<rig::completion::ToolDefinition> = if is_task_primary {
        tracing::info!(
            "[Task mode] Primary agent: delegation-only tools (no direct environment access)"
        );
        // Only the planning tool from the static catalog
        get_all_tool_definitions_with_config(ctx.tool_config)
            .into_iter()
            .filter(|t| t.name == "update_plan")
            .collect::<Vec<_>>()
    } else {
        // Chat mode or sub-agent execution: full tool set
        let mut t = get_all_tool_definitions_with_config(ctx.tool_config);
        t.push(get_run_command_tool_definition());
        // Subtask agents in Task mode must not call update_plan — only the
        // orchestrator's refiner phase manages plan modifications.
        if is_task_subtask {
            t.retain(|tool| tool.name != "update_plan");
        }
        t
    };

    // Always add ask_human barrier tool (HITL: AI asks user for input)
    tools.push(get_ask_human_tool_definition());

    if !is_task_primary {
        // Add any additional tools (e.g., SWE-bench test tool, MCP tools)
        tools.extend(ctx.additional_tool_definitions.iter().cloned());

        // Add dynamically registered tools from the registry (Tavily, PTY
        // interactive, pentest, ...).
        let registry = ctx.tool_registry.read().await;
        let registry_tools = registry.get_tool_definitions();
        drop(registry);

        let existing_names: HashSet<String> = tools.iter().map(|t| t.name.clone()).collect();

        for tool in registry_tools {
            if existing_names.contains(&tool.name) {
                continue;
            }

            let always_include = tool.name.starts_with("pentest_");
            let tavily_enabled = tool.name.starts_with("tavily_")
                && ctx.tool_config.is_tool_enabled(&tool.name);

            if always_include || tavily_enabled {
                tools.push(rig::completion::ToolDefinition {
                    name: tool.name,
                    description: tool.description,
                    parameters: sanitize_schema(tool.parameters),
                });
            }
        }
    } else {
        // Task-mode primary: add knowledge/memory tools for context retrieval
        let registry = ctx.tool_registry.read().await;
        let registry_tools = registry.get_tool_definitions();
        drop(registry);

        for tool in registry_tools {
            let is_knowledge = tool.name == "search_knowledge_base"
                || tool.name == "read_knowledge"
                || tool.name == "search_memories"
                || tool.name == "query_target_data";
            if is_knowledge {
                tools.push(rig::completion::ToolDefinition {
                    name: tool.name,
                    description: tool.description,
                    parameters: sanitize_schema(tool.parameters),
                });
            }
        }
    }

    // Add sub-agent dispatch tools when depth budget remains.
    // Sub-agents are controlled by the registry, not the tool config.
    if sub_agent_context.depth < MAX_AGENT_DEPTH - 1 {
        let registry = ctx.sub_agent_registry.read().await;
        tools.extend(get_sub_agent_tool_definitions(&registry).await);
    }

    tracing::debug!(
        "Available tools (unified loop, mode={}, depth={}): {:?}",
        ctx.execution_mode,
        sub_agent_context.depth,
        tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>()
    );

    tools
}
