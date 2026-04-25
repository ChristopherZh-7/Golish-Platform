//! Build the tool list visible to a sub-agent invocation.
//!
//! Composition order:
//! 1. Filter the static tool catalogue against the agent's `allowed_tools`.
//! 2. Add any dynamically registered tools that match `allowed_tools`
//!    (e.g. `pentest_*`, MCP-loaded tools).
//! 3. Append the universal [`BARRIER_TOOL_NAME`] (`submit_result`).
//! 4. Append nested-delegation `sub_agent_*` shims for each agent listed in
//!    `delegatable_agents`, gated on [`crate::MAX_AGENT_DEPTH`].

use std::collections::HashSet;

use rig::completion::ToolDefinition;

use crate::definition::{SubAgentContext, SubAgentDefinition};
use crate::executor_types::{SubAgentExecutorContext, ToolProvider, BARRIER_TOOL_NAME};
use crate::MAX_AGENT_DEPTH;

/// Construct the full tool list for a sub-agent iteration.
pub(super) async fn build_tool_definitions<P: ToolProvider>(
    agent_def: &SubAgentDefinition,
    sub_context: &SubAgentContext,
    ctx: &SubAgentExecutorContext<'_>,
    tool_provider: &P,
) -> Vec<ToolDefinition> {
    let agent_id = &agent_def.id;

    // Filter static catalogue against the agent's allowlist.
    let all_tools = tool_provider.get_all_tool_definitions();
    let mut tools = tool_provider.filter_tools_by_allowed(all_tools, &agent_def.allowed_tools);

    // Layer in dynamically registered tools (pentest_list_tools, pentest_run, etc.)
    // that are in the agent's allowed_tools but not in the static definitions.
    {
        let existing_names: HashSet<String> = tools.iter().map(|t| t.name.clone()).collect();
        let allowed_set: HashSet<&str> = agent_def
            .allowed_tools
            .iter()
            .map(|s| s.as_str())
            .collect();
        let registry = ctx.tool_registry.read().await;
        for td in registry.get_tool_definitions() {
            if allowed_set.contains(td.name.as_str()) && !existing_names.contains(&td.name) {
                tools.push(td);
            }
        }
    }

    // Universal barrier tool — every sub-agent uses this to submit its final
    // structured result.
    tools.push(barrier_tool_definition());

    // Nested delegation shims (PentAGI hierarchical pattern, e.g. pentester
    // delegates to coder/searcher).
    if !agent_def.delegatable_agents.is_empty() && sub_context.depth < MAX_AGENT_DEPTH - 1 {
        if let Some(registry) = ctx.sub_agent_registry {
            let reg = registry.read().await;
            for delegate_id in &agent_def.delegatable_agents {
                if let Some(delegate_def) = reg.get(delegate_id) {
                    tools.push(nested_delegation_tool_definition(delegate_id, delegate_def));
                    tracing::debug!(
                        "[sub-agent:{}] Added nested delegation tool: sub_agent_{}",
                        agent_id,
                        delegate_id
                    );
                }
            }
        }
    }

    tools
}

/// Return the [`BARRIER_TOOL_NAME`] tool definition.
///
/// Calling this tool terminates the agent loop and the structured result is
/// surfaced to the caller (PentAGI `hack_result` / `code_result` pattern).
fn barrier_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: BARRIER_TOOL_NAME.to_string(),
        description: "Submit your final structured result and complete this task. You MUST call this \
            tool when your work is done — do NOT end with a plain text message. Include your key \
            findings, outputs, and whether the task succeeded."
            .to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "result": {
                    "type": "string",
                    "description": "Your complete result: findings, outputs, code, data, or error details"
                },
                "success": {
                    "type": "boolean",
                    "description": "Whether the task was completed successfully"
                },
                "summary": {
                    "type": "string",
                    "description": "A one-line summary of what was accomplished"
                }
            },
            "required": ["result", "success", "summary"],
            "additionalProperties": false
        }),
    }
}

/// Return a `sub_agent_<id>` tool definition that, when invoked, dispatches a
/// nested sub-agent execution.
fn nested_delegation_tool_definition(
    delegate_id: &str,
    delegate_def: &SubAgentDefinition,
) -> ToolDefinition {
    ToolDefinition {
        name: format!("sub_agent_{}", delegate_id),
        description: format!("[{}] {}", delegate_def.name, delegate_def.description),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "The specific task for this sub-agent"
                },
                "context": {
                    "type": "string",
                    "description": "Additional context to help the sub-agent"
                }
            },
            "required": ["task"],
            "additionalProperties": false
        }),
    }
}
