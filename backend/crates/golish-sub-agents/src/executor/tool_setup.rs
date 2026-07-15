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
use crate::executor_types::{
    StageTeamLeaderBinding, SubAgentExecutorContext, ToolProvider, BARRIER_TOOL_NAME,
    STAGE_TEAM_DISPATCH_WORKERS_TOOL_NAME, STAGE_TEAM_PREPARE_FINAL_SUBMISSION_TOOL_NAME,
    STAGE_TEAM_UPDATE_PLAN_TOOL_NAME,
};
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
    let update_plan_catalog = all_tools
        .iter()
        .find(|tool| tool.name == STAGE_TEAM_UPDATE_PLAN_TOOL_NAME)
        .cloned();
    let mut tools = tool_provider.filter_tools_by_allowed(all_tools, &agent_def.allowed_tools);

    // Layer in dynamically registered tools (pentest_list_tools, pentest_run, etc.)
    // that are in the agent's allowed_tools but not in the static definitions.
    {
        let existing_names: HashSet<String> = tools.iter().map(|t| t.name.clone()).collect();
        let allowed_set: HashSet<&str> =
            agent_def.allowed_tools.iter().map(|s| s.as_str()).collect();
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

    // These are host controls rather than ordinary allowlisted tools. Only an
    // exact server-owned Company Controller binding grants visibility.
    configure_stage_team_leader_tools(
        &mut tools,
        ctx.bound_worker_chain.is_some(),
        ctx.bound_worker_chain
            .as_ref()
            .and_then(|bound| bound.stage_team_leader.as_ref()),
        update_plan_catalog,
    );

    // Nested delegation shims (PentAGI hierarchical pattern, e.g. pentester
    // delegates to coder/searcher).
    if !agent_def.delegatable_agents.is_empty() && sub_context.depth < MAX_AGENT_DEPTH - 1 {
        if let Some(registry) = ctx.sub_agent_registry {
            let reg = registry.read().await;
            for delegate_id in &agent_def.delegatable_agents {
                if let Some(delegate_def) = reg.get(delegate_id) {
                    if delegate_def.pipeline_only {
                        tracing::debug!(
                            "[sub-agent:{}] Skipping pipeline-only agent: {}",
                            agent_id,
                            delegate_id
                        );
                        continue;
                    }
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

    // D1 · hide tools the active harness stage forbids entirely (e.g. scan tools
    // in scoping) so the model never even attempts one — mirrors the main agent's
    // tool-list filter. The per-call `stage_tool_guard` stays as the backstop.
    apply_stage_tool_hiding(&mut tools, &ctx.hide_tool_in_stage, agent_id);

    tools
}

/// D1 · remove tools the active harness stage forbids entirely from a sub-agent's
/// visible list (e.g. scan tools in `scoping`). No-op when no hider is set.
fn apply_stage_tool_hiding(
    tools: &mut Vec<ToolDefinition>,
    hider: &Option<crate::executor_types::StageToolHider>,
    agent_id: &str,
) {
    let Some(hide) = hider.as_ref() else {
        return;
    };
    let before = tools.len();
    tools.retain(|t| !hide(&t.name));
    if tools.len() != before {
        tracing::debug!(
            "[sub-agent:{}] hid {} stage-forbidden tool(s) from the list",
            agent_id,
            before - tools.len()
        );
    }
}

/// Return the [`BARRIER_TOOL_NAME`] tool definition.
///
/// Calling this tool terminates the agent loop and the structured result is
/// surfaced to the caller (PentAGI `hack_result` / `code_result` pattern).
fn barrier_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: BARRIER_TOOL_NAME.to_string(),
        description:
            "Submit your final structured result and complete this task. You MUST call this \
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

fn configure_stage_team_leader_tools(
    tools: &mut Vec<ToolDefinition>,
    has_bound_worker: bool,
    binding: Option<&StageTeamLeaderBinding>,
    update_plan_catalog: Option<ToolDefinition>,
) {
    // Unbound orchestrator agents keep their existing generic update_plan
    // allowlist and router. Any bound stage worker is narrower: only the exact
    // trusted Company Controller may see the local planning control.
    if !has_bound_worker {
        return;
    }
    tools.retain(|tool| tool.name != STAGE_TEAM_UPDATE_PLAN_TOOL_NAME);
    if binding.is_none() {
        return;
    }
    if let Some(mut update_plan) = update_plan_catalog {
        update_plan.description = "Create or update this Company Controller's local working plan. The plan is recorded only in this durable Controller message chain; it does not update the global task plan. At most one plan item may be in_progress. Plan status tracks the Controller's current focus, not tool or worker concurrency: when multiple tools or workers run in parallel, describe that batch in one composite in_progress step instead of marking one in_progress step per operation.".to_string();
        let Some(plan_schema) = update_plan
            .parameters
            .pointer_mut("/properties/plan")
            .and_then(serde_json::Value::as_object_mut)
        else {
            tracing::error!("static update_plan catalogue entry has no plan schema");
            tools.push(stage_team_dispatch_workers_tool_definition());
            tools.push(stage_team_prepare_final_submission_tool_definition());
            return;
        };
        plan_schema.insert("minItems".to_string(), serde_json::json!(1));
        plan_schema.insert("maxItems".to_string(), serde_json::json!(12));
        plan_schema.insert(
            "description".to_string(),
            serde_json::json!(
                "One to twelve Controller work steps. At most one item may be in_progress. Parallel tools or workers must share one composite in_progress step."
            ),
        );
        let Some(item_schema) = plan_schema
            .get_mut("items")
            .and_then(serde_json::Value::as_object_mut)
        else {
            tracing::error!("static update_plan catalogue entry has no plan item schema");
            tools.push(stage_team_dispatch_workers_tool_definition());
            tools.push(stage_team_prepare_final_submission_tool_definition());
            return;
        };
        item_schema.insert(
            "required".to_string(),
            serde_json::json!(["step", "status"]),
        );
        item_schema.insert("additionalProperties".to_string(), serde_json::json!(false));
        let Some(status_schema) = item_schema
            .get_mut("properties")
            .and_then(|properties| properties.get_mut("status"))
            .and_then(serde_json::Value::as_object_mut)
        else {
            tracing::error!("static update_plan catalogue entry has no status schema");
            tools.push(stage_team_dispatch_workers_tool_definition());
            tools.push(stage_team_prepare_final_submission_tool_definition());
            return;
        };
        status_schema.insert(
            "enum".to_string(),
            serde_json::json!(["pending", "in_progress", "completed"]),
        );
        status_schema.insert(
            "description".to_string(),
            serde_json::json!(
                "Controller focus status. Never assign in_progress to more than one item, even when operations execute concurrently."
            ),
        );
        tools.push(update_plan);
    }
    tools.push(stage_team_dispatch_workers_tool_definition());
    tools.push(stage_team_prepare_final_submission_tool_definition());
}

fn stage_team_dispatch_workers_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: STAGE_TEAM_DISPATCH_WORKERS_TOOL_NAME.to_string(),
        description: "Durably request a bounded batch of Stage Team workers for this company. A successful dispatch parks this Controller and returns control to the outer scheduler; it does not run children inside the Controller lease.".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "workers": {
                    "type": "array",
                    "minItems": 1,
                    "description": "Bounded worker requests. Role, kind, subject scope, budgets, and limits are revalidated by the host.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "dedupe_key": {
                                "type": "string",
                                "minLength": 1,
                                "description": "Stable replay key for this logical assignment."
                            },
                            "role": {
                                "type": "string",
                                "minLength": 1,
                                "description": "Requested server-allowlisted worker role."
                            },
                            "kind": {
                                "type": "string",
                                "minLength": 1,
                                "description": "Requested server-allowlisted work-item kind."
                            },
                            "objective": {
                                "type": "string",
                                "minLength": 1,
                                "description": "Specific bounded assignment for the worker."
                            },
                            "subject_refs": {
                                "type": "array",
                                "description": "Optional canonical subject references inside this company's frozen scope.",
                                "items": { "type": "object" }
                            }
                        },
                        "required": ["dedupe_key", "role", "kind", "objective"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["workers"],
            "additionalProperties": false
        }),
    }
}

fn stage_team_prepare_final_submission_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: STAGE_TEAM_PREPARE_FINAL_SUBMISSION_TOOL_NAME.to_string(),
        description: "Close this Controller's current worker-request epoch and return control to the outer scheduler so it can verify the durable dependency barrier and prepare the sole final submission path.".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {},
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor_types::StageTeamLeaderBinding;
    use std::sync::Arc;
    use uuid::Uuid;

    fn td(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: String::new(),
            parameters: serde_json::json!({}),
        }
    }

    fn update_plan_catalog_definition() -> ToolDefinition {
        ToolDefinition {
            name: "update_plan".to_string(),
            description: "generic plan".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "explanation": {"type":"string"},
                    "plan": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "step": {"type":"string"},
                                "status": {
                                    "type":"string",
                                    "enum":["pending","in_progress","completed"]
                                }
                            },
                            "required": ["step"]
                        }
                    }
                },
                "required": ["plan"]
            }),
        }
    }

    #[test]
    fn hider_removes_matching_tools_keeps_others() {
        let mut tools = vec![
            td("pentest_run"),
            td("submit_stage_deliverable"),
            td("query_target_data"),
        ];
        // Simulate a zero-scan stage hider that hides the scan wrapper only.
        let hider: Option<crate::executor_types::StageToolHider> =
            Some(Arc::new(|name: &str| name == "pentest_run"));
        apply_stage_tool_hiding(&mut tools, &hider, "pentester");
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(!names.contains(&"pentest_run"), "scan tool must be hidden");
        assert!(
            names.contains(&"submit_stage_deliverable") && names.contains(&"query_target_data"),
            "non-scan/meta tools must be kept: {names:?}"
        );
    }

    #[test]
    fn no_hider_is_a_noop() {
        let mut tools = vec![td("pentest_run")];
        apply_stage_tool_hiding(&mut tools, &None, "pentester");
        assert_eq!(tools.len(), 1, "no hider → list untouched");
    }

    #[test]
    fn stage_team_control_tools_require_a_trusted_leader_binding() {
        let catalog = update_plan_catalog_definition();

        let mut unbound_orchestrator_tools = vec![catalog.clone()];
        configure_stage_team_leader_tools(
            &mut unbound_orchestrator_tools,
            false,
            None,
            Some(catalog.clone()),
        );
        assert_eq!(
            unbound_orchestrator_tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            vec!["update_plan"],
            "unbound orchestrators retain the existing generic planning path"
        );

        let mut tools = vec![catalog.clone()];
        configure_stage_team_leader_tools(&mut tools, true, None, Some(catalog.clone()));
        assert!(
            tools.is_empty(),
            "ordinary bound workers see neither update_plan nor team controls"
        );

        let binding = StageTeamLeaderBinding {
            stage_team_plan_id: Uuid::new_v4(),
            leader_work_item_id: Uuid::new_v4(),
            expected_dispatch_epoch: 3,
            expected_plan_row_version: 5,
            expected_work_item_row_version: 7,
        };
        configure_stage_team_leader_tools(&mut tools, true, Some(&binding), Some(catalog));

        let names = tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "update_plan",
                "stage_team_dispatch_workers",
                "stage_team_prepare_final_submission"
            ]
        );
        assert_eq!(tools[0].parameters["required"], serde_json::json!(["plan"]));
        assert_eq!(
            tools[0].parameters["properties"]["plan"]["items"]["required"],
            serde_json::json!(["step", "status"]),
            "Controller update_plan requires an explicit status on every item"
        );
        assert_eq!(tools[0].parameters["properties"]["plan"]["minItems"], 1);
        assert_eq!(tools[0].parameters["properties"]["plan"]["maxItems"], 12);
        assert!(tools[0]
            .description
            .contains("one composite in_progress step"));
        assert!(tools[0].parameters["properties"]["plan"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("Parallel tools or workers")));
        assert_eq!(
            tools[0].parameters["properties"]["plan"]["items"]["additionalProperties"],
            false
        );
        assert_eq!(
            tools[0].parameters["properties"]["plan"]["items"]["properties"]["status"]["enum"],
            serde_json::json!(["pending", "in_progress", "completed"])
        );
        assert!(
            tools[0].parameters["properties"]["plan"]["items"]["properties"]["status"]
                ["description"]
                .as_str()
                .is_some_and(
                    |description| description.contains("even when operations execute concurrently")
                )
        );
        assert_eq!(
            tools[1].parameters["required"],
            serde_json::json!(["workers"])
        );
        assert_eq!(
            tools[1].parameters["properties"]["workers"]["items"]["required"],
            serde_json::json!(["dedupe_key", "role", "kind", "objective"]),
            "subject_refs stays optional"
        );
    }
}
