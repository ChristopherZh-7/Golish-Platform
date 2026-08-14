//! Build the tool list visible to a sub-agent invocation.
//!
//! Composition order:
//! 1. Filter the static tool catalogue against the agent's `allowed_tools`.
//! 2. Add any dynamically registered tools that match `allowed_tools`
//!    (e.g. `pentest_*`, MCP-loaded tools).
//! 3. Append the universal [`BARRIER_TOOL_NAME`] (`submit_result`), except for
//!    the closed Target Intel reviewer whose durable verdict is its barrier.
//! 4. Append nested-delegation `sub_agent_*` shims for each agent listed in
//!    `delegatable_agents`, gated on [`crate::MAX_AGENT_DEPTH`].

use std::collections::HashSet;

use rig::completion::ToolDefinition;

use crate::definition::{SubAgentContext, SubAgentDefinition};
use crate::executor_types::{
    BoundTerminalExecutionContract, StageTeamLeaderBinding, SubAgentExecutorContext, ToolProvider,
    BARRIER_TOOL_NAME, ENUMERATION_REDUCE_PARAMETERS_TOOL_NAME,
    ENUMERATION_REVIEW_COVERAGE_TOOL_NAME,
    INVESTIGATION_ASSET_VERIFICATION_ACTOR_OBSERVATION_SCHEMA,
    INVESTIGATION_ASSET_VERIFICATION_PRIMARY_RESOLUTION_SCHEMA,
    INVESTIGATION_DYNAMIC_VERIFICATION_PRIMARY_TURN_SCHEMA,
    INVESTIGATION_PRIMARY_SYNTHESIS_RESULT_SCHEMA, INVESTIGATION_REFINER_PATCH_RESULT_SCHEMA,
    INVESTIGATION_TASK_PLAN_RESULT_SCHEMA, STAGE_TEAM_DISPATCH_WORKERS_TOOL_NAME,
    STAGE_TEAM_PREPARE_FINAL_SUBMISSION_TOOL_NAME, STAGE_TEAM_UPDATE_PLAN_TOOL_NAME,
};
use crate::MAX_AGENT_DEPTH;

fn is_target_intel_reviewer_role(agent_id: &str) -> bool {
    agent_id == "target_intel_reviewer"
}

fn final_submission_only_tool_definitions(mut tools: Vec<ToolDefinition>) -> Vec<ToolDefinition> {
    tools.retain(|tool| tool.name == "submit_stage_deliverable");
    tools
}

/// Fail closed if a mutable/custom definition attempts to widen a static,
/// host-reviewed specialist role.
pub(super) fn validate_static_specialist_definition(
    agent_def: &SubAgentDefinition,
) -> anyhow::Result<()> {
    if is_target_intel_reviewer_role(&agent_def.id) {
        let expected = [
            crate::TARGET_INTEL_READ_REVIEW_SECTION,
            crate::TARGET_INTEL_RECORD_REVIEW_VERDICT,
        ];
        if !agent_def.readonly
            || agent_def.allowed_tools.as_slice() != expected
            || !agent_def.delegatable_agents.is_empty()
            || agent_def.prompt_template.is_some()
        {
            anyhow::bail!(
                "Target Intel reviewer must be static, read-only, non-delegating, and expose only the ordered review tools"
            );
        }
        return Ok(());
    }
    if agent_def.id == "resolution_analyst" {
        let expected = [
            "enum_js_get_resolution_cluster",
            "enum_js_submit_resolution",
        ];
        if !agent_def.readonly
            || agent_def.allowed_tools.as_slice() != expected
            || !agent_def.delegatable_agents.is_empty()
            || agent_def.prompt_template.is_some()
        {
            anyhow::bail!(
                "resolution_analyst must be read-only, static-prompt, non-delegating, and expose only its two bounded cluster tools"
            );
        }
        return Ok(());
    }
    Ok(())
}

/// Construct the full tool list for a sub-agent iteration.
pub(super) async fn build_tool_definitions<P: ToolProvider>(
    agent_def: &SubAgentDefinition,
    sub_context: &SubAgentContext,
    ctx: &SubAgentExecutorContext<'_>,
    tool_provider: &P,
) -> Vec<ToolDefinition> {
    let agent_id = &agent_def.id;

    // A host-bound terminal modeler sees exactly one submit_result schema.
    // Resolve this before any catalogue, registry, role, or delegation logic so
    // mutable definitions cannot widen the closed execution surface.
    if let Some(contract) = ctx
        .bound_worker_chain
        .as_ref()
        .and_then(|bound| bound.terminal_execution.as_ref())
    {
        return terminal_only_tool_definitions(contract);
    }

    // A Verification Primary delegates through its closed turn output; it
    // never inherits Analysis cognition tools or the actor's target-I/O
    // wrappers. The host performs durable dispatch/resolution after validating
    // the exact session-bound payload.
    if let Some(crate::InvestigationActorContract::AssetVerificationPrimary(binding)) = ctx
        .bound_worker_chain
        .as_ref()
        .and_then(|bound| bound.investigation_actor_contract.as_ref())
    {
        return if binding.validate().is_ok() {
            vec![barrier_tool_definition(Some(
                INVESTIGATION_DYNAMIC_VERIFICATION_PRIMARY_TURN_SCHEMA,
            ))]
        } else {
            vec![barrier_tool_definition(None)]
        };
    }

    // Every Primary-requested Verification actor sees the same four host
    // wrappers plus bounded cognition and may make multiple invocations. Role
    // is not a roster slot or capability selector; inner tools stay dynamic in
    // the real Tool Manager inventory.
    if let Some(crate::InvestigationActorContract::AssetVerification(binding)) = ctx
        .bound_worker_chain
        .as_ref()
        .and_then(|bound| bound.investigation_actor_contract.as_ref())
    {
        if binding.validate().is_err() {
            return vec![barrier_tool_definition(None)];
        }
        let mut candidates = tool_provider.get_all_tool_definitions();
        let registry = ctx.tool_registry.read().await;
        candidates.extend(registry.get_tool_definitions());
        drop(registry);
        let stage_team_output_schema = ctx
            .bound_worker_chain
            .as_ref()
            .and_then(|bound| bound.stage_team_output_schema.as_deref());
        return asset_verification_tool_definitions(candidates, stage_team_output_schema);
    }

    if is_target_intel_reviewer_role(agent_id) {
        return target_intel_reviewer_tool_definitions();
    }

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

    // A closed Company Controller plan has already frozen its child barrier
    // and exact final submitter. This server-owned retry mode exists only to
    // persist the StageDeliverable on that same WorkerRun/message chain. Do
    // not expose the generic submit_result barrier (or any work tool): it is a
    // valid specialist terminator but can never satisfy the stage finalizer,
    // which otherwise creates an endless FINAL_SUBMISSION_MISSING resume loop.
    if ctx
        .bound_worker_chain
        .as_ref()
        .is_some_and(|bound| bound.return_on_first_durable_stage_submission)
    {
        return final_submission_only_tool_definitions(tools);
    }

    // Universal barrier tool — every sub-agent uses this to submit its final
    // structured result.
    let stage_team_output_schema = ctx
        .bound_worker_chain
        .as_ref()
        .filter(|bound| bound.is_stage_team_child())
        .and_then(|bound| bound.stage_team_output_schema.as_deref());
    tools.push(barrier_tool_definition(stage_team_output_schema));

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

    // Enumeration receipt reducers are host-routed controls, not registry
    // executors. Only an explicitly allowlisted actor can see the schema; the
    // runtime still requires an exact bound Enumeration Controller and a live
    // worker-tool fence before executing either reducer.
    for name in [
        ENUMERATION_REDUCE_PARAMETERS_TOOL_NAME,
        ENUMERATION_REVIEW_COVERAGE_TOOL_NAME,
    ] {
        if agent_def
            .allowed_tools
            .iter()
            .any(|allowed| allowed == name)
            && !tools.iter().any(|tool| tool.name == name)
        {
            tools.push(enumeration_receipt_reducer_tool_definition(name));
        }
    }

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

    // Investigation planning Primaries return one typed plan to the host-owned
    // TaskOrchestrator. Do not expose a second recursive dispatch loop or the
    // generic stage-deliverable writer on that planning turn.
    if ctx
        .bound_worker_chain
        .as_ref()
        .and_then(|bound| bound.stage_team_leader.as_ref())
        .is_some_and(|leader| leader.planning_only)
    {
        tools.retain(|tool| {
            tool.name != "submit_stage_deliverable" && !tool.name.starts_with("sub_agent_")
        });
    }

    // D1 · hide tools the active harness stage forbids entirely (e.g. scan tools
    // in scoping) so the model never even attempts one — mirrors the main agent's
    // tool-list filter. The per-call `stage_tool_guard` stays as the backstop.
    apply_stage_tool_hiding(&mut tools, &ctx.hide_tool_in_stage, agent_id);

    // Fixture/dev Goal Loop uses one host-owned evidence adapter for root and
    // sub-agents. Legacy web tools are invisible so neither path can bypass
    // evidence-first persistence.
    configure_intel_public_fixture_tools(&mut tools, tool_provider.intel_public_tool_definitions());
    tools
}

fn asset_verification_tool_definitions(
    candidates: Vec<ToolDefinition>,
    output_schema: Option<&str>,
) -> Vec<ToolDefinition> {
    let mut seen = HashSet::new();
    let mut tools = candidates
        .into_iter()
        .filter(|tool| {
            (crate::is_investigation_asset_verification_tool(&tool.name)
                || crate::is_investigation_asset_verification_cognition_tool(&tool.name))
                && tool.name != BARRIER_TOOL_NAME
                && seen.insert(tool.name.clone())
        })
        .collect::<Vec<_>>();
    tools.push(barrier_tool_definition(output_schema));
    tools
}

fn target_intel_reviewer_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: crate::TARGET_INTEL_READ_REVIEW_SECTION.to_string(),
            description: "Read exactly the next immutable Target Intel review section. Sections must be read once in server order.".to_string(),
            parameters: crate::target_intel_read_review_section_schema(),
        },
        ToolDefinition {
            name: crate::TARGET_INTEL_RECORD_REVIEW_VERDICT.to_string(),
            description: "Record one terminal intel_review.v1 verdict against the exact frozen bundle and reviewer attempt. This durable verdict is the reviewer's sole terminal barrier; do not call submit_result.".to_string(),
            parameters: crate::target_intel_record_review_verdict_schema(),
        },
    ]
}

fn enumeration_receipt_reducer_tool_definition(name: &str) -> ToolDefinition {
    let description = if name == ENUMERATION_REDUCE_PARAMETERS_TOOL_NAME {
        "Reduce the exact current Browser and JS/API receipt pair into a value-free Parameter receipt. Call only after both producer results returned typed receipts. The host selects every dependency and evidence id."
    } else {
        "Review the exact current Browser, JS/API, Parameter, and Resolution receipt set for one Web Origin. This seals only the typed result DAG; it does not run or schedule any producer."
    };
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "target_id": {"type": "string", "format": "uuid"},
                "exact_origin": {"type": "string", "description": "Canonical scheme://host:port exact Web Origin with no path, query, or fragment."}
            },
            "required": ["target_id", "exact_origin"],
            "additionalProperties": false
        }),
    }
}

fn configure_intel_public_fixture_tools(
    tools: &mut Vec<ToolDefinition>,
    public_tools: Option<Vec<ToolDefinition>>,
) {
    if let Some(public_tools) = public_tools {
        tools.retain(|tool| {
            !matches!(
                tool.name.as_str(),
                "web_search" | "web_fetch" | "intel_public_search" | "intel_public_fetch"
            )
        });
        tools.extend(public_tools);
    }
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
fn terminal_only_tool_definitions(
    contract: &BoundTerminalExecutionContract,
) -> Vec<ToolDefinition> {
    let mut barrier = barrier_tool_definition(None);
    barrier.parameters["properties"]["result"] = contract.result_schema.clone();
    vec![barrier]
}

fn barrier_tool_definition(stage_team_output_schema: Option<&str>) -> ToolDefinition {
    fn asset_verification_hypothesis_proposal_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "description": "Complete semantic proposal for a distinct follow-up hypothesis on the same asset. The host derives subject identity, dedupe keys, authority ids, and hashes.",
            "properties": {
                "predicate_schema": {"type": "string", "minLength": 1, "maxLength": 128},
                "predicate_version": {"type": "integer", "minimum": 1},
                "predicate_arguments": {
                    "type": "array", "maxItems": 64,
                    "items": {
                        "type": "array", "minItems": 2, "maxItems": 2,
                        "prefixItems": [
                            {"type": "string", "minLength": 1, "maxLength": 128},
                            {"type": "string", "maxLength": 4096}
                        ],
                        "items": false
                    }
                },
                "trust_boundary": {"type": "string", "minLength": 1, "maxLength": 256},
                "polarity": {"type": "string", "enum": ["positive", "negative"]},
                "structured_claim": {"type": "string", "minLength": 1, "maxLength": 8192},
                "preconditions": {
                    "type": "array", "maxItems": 64,
                    "items": {"type": "string", "minLength": 1, "maxLength": 1024}
                },
                "impact": {"type": "string", "minLength": 1, "maxLength": 4096},
                "rationale": {"type": "string", "minLength": 1, "maxLength": 4096}
            },
            "required": [
                "predicate_schema", "predicate_version", "predicate_arguments",
                "trust_boundary", "polarity", "structured_claim", "preconditions",
                "impact", "rationale"
            ],
            "additionalProperties": false
        })
    }

    fn asset_verification_citation_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "description": "One durable fresh authority citation. Supply exactly one reference kind.",
            "properties": {
                "audit_evidence_id": {"type": ["integer", "null"], "minimum": 1},
                "authority_id": {"type": ["string", "null"], "format": "uuid"}
            },
            "required": ["audit_evidence_id", "authority_id"],
            "oneOf": [
                {
                    "properties": {
                        "audit_evidence_id": {"type": "integer", "minimum": 1},
                        "authority_id": {"type": "null"}
                    }
                },
                {
                    "properties": {
                        "audit_evidence_id": {"type": "null"},
                        "authority_id": {"type": "string", "format": "uuid"}
                    }
                }
            ],
            "additionalProperties": false
        })
    }

    fn investigation_subject_ref_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": [
                        "organization", "target", "asset", "endpoint", "application_model",
                        "evidence", "hypothesis_revision", "verification_task"
                    ]
                },
                "id": {
                    "oneOf": [
                        {"type": "string", "minLength": 1},
                        {"type": "integer", "minimum": 1}
                    ],
                    "description": "Canonical UUID string, or a positive decimal evidence id when kind=evidence"
                }
            },
            "required": ["kind", "id"],
            "additionalProperties": false
        })
    }

    fn investigation_subtask_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "stable_key": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 128,
                    "pattern": "^[A-Za-z0-9_:-]+$"
                },
                "role": {
                    "type": "string",
                    "enum": [
                        "pentester", "researcher", "browser", "coder", "installer",
                        "enricher", "memorist", "adviser"
                    ]
                },
                "objective": {"type": "string", "minLength": 1, "maxLength": 4096},
                "rationale": {"type": "string", "minLength": 1, "maxLength": 2048},
                "subject_refs": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 32,
                    "items": investigation_subject_ref_schema()
                }
            },
            "required": ["stable_key", "role", "objective", "rationale", "subject_refs"],
            "additionalProperties": false
        })
    }

    fn dynamic_verification_subtask_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "description": "One ordered actor call on the exact current target and hypothesis revision. Roles may repeat within the same turn.",
            "properties": {
                "stable_key": {
                    "type": "string", "minLength": 1, "maxLength": 128,
                    "pattern": "^[A-Za-z0-9_:-]+$"
                },
                "role": {
                    "type": "string",
                    "enum": [
                        "pentester", "researcher", "browser", "coder", "installer",
                        "enricher", "memorist", "adviser"
                    ]
                },
                "objective": {"type": "string", "minLength": 1, "maxLength": 4096},
                "rationale": {"type": "string", "minLength": 1, "maxLength": 2048},
                "subject_refs": {
                    "type": "array", "minItems": 2, "maxItems": 2,
                    "prefixItems": [
                        {
                            "type": "object",
                            "properties": {
                                "kind": {"type": "string", "enum": ["target"]},
                                "id": {"type": "string", "format": "uuid"}
                            },
                            "required": ["kind", "id"],
                            "additionalProperties": false
                        },
                        {
                            "type": "object",
                            "properties": {
                                "kind": {"type": "string", "enum": ["hypothesis_revision"]},
                                "id": {"type": "string", "format": "uuid"}
                            },
                            "required": ["kind", "id"],
                            "additionalProperties": false
                        }
                    ],
                    "items": false
                }
            },
            "required": ["stable_key", "role", "objective", "rationale", "subject_refs"],
            "additionalProperties": false
        })
    }

    let result_schema = if stage_team_output_schema == Some(INVESTIGATION_TASK_PLAN_RESULT_SCHEMA) {
        serde_json::json!({
            "type": "object",
            "description": "The exact InvestigationGeneratedTaskPlanV1 object. Return it directly without prose or a JSON string wrapper.",
            "properties": {
                "schema_version": {"type": "integer", "enum": [1]},
                "summary": {"type": "string", "minLength": 1, "maxLength": 2048},
                "subtasks": {
                    "type": "array",
                    "minItems": 0,
                    "maxItems": 8,
                    "items": investigation_subtask_schema()
                }
            },
            "required": ["schema_version", "summary", "subtasks"],
            "additionalProperties": false
        })
    } else if stage_team_output_schema == Some(INVESTIGATION_REFINER_PATCH_RESULT_SCHEMA) {
        serde_json::json!({
            "type": "object",
            "description": "The exact InvestigationRefinerPatchV1 object. Return it directly without prose or a JSON string wrapper.",
            "properties": {
                "schema_version": {"type": "integer", "enum": [1]},
                "summary": {"type": "string", "minLength": 1, "maxLength": 2048},
                "completed_subtask_key": {"type": "string", "minLength": 1, "maxLength": 128},
                "accepted_output_sha256": {
                    "type": "string",
                    "pattern": "^sha256:[0-9a-f]{64}$"
                },
                "remaining_subtasks": {
                    "type": "array",
                    "maxItems": 8,
                    "items": investigation_subtask_schema()
                }
            },
            "required": [
                "schema_version", "summary", "completed_subtask_key",
                "accepted_output_sha256", "remaining_subtasks"
            ],
            "additionalProperties": false
        })
    } else if stage_team_output_schema == Some(INVESTIGATION_PRIMARY_SYNTHESIS_RESULT_SCHEMA) {
        serde_json::json!({
            "type": "object",
            "description": "The exact InvestigationPrimarySynthesisV1 object. Return it directly without prose or a JSON string wrapper.",
            "properties": {
                "schema_version": {"type": "integer", "enum": [1]},
                "summary": {"type": "string", "minLength": 1, "maxLength": 4096},
                "accepted_output_sha256": {
                    "type": "array",
                    "items": {"type": "string", "pattern": "^sha256:[0-9a-f]{64}$"}
                },
                "proposal_signals": {"type": "array", "maxItems": 64, "items": {"type": "object"}},
                "action_intents": {"type": "array", "maxItems": 64, "items": {"type": "object"}},
                "residuals": {"type": "array", "maxItems": 64, "items": {"type": "object"}}
            },
            "required": [
                "schema_version", "summary", "accepted_output_sha256",
                "proposal_signals", "action_intents", "residuals"
            ],
            "additionalProperties": false
        })
    } else if stage_team_output_schema
        == Some(INVESTIGATION_ASSET_VERIFICATION_ACTOR_OBSERVATION_SCHEMA)
    {
        serde_json::json!({
            "type": "object",
            "description": "Fresh evidence-grounded observation from one exact Primary-requested actor in an asset Verification session.",
            "properties": {
                "schema_version": {"type": "integer", "enum": [1]},
                "session_id": {"type": "string", "format": "uuid"},
                "hypothesis_revision_id": {"type": "string", "format": "uuid"},
                "actor_call_id": {"type": "string", "format": "uuid"},
                "actor_ordinal": {"type": "integer", "minimum": 1},
                "subtask_id": {"type": "string", "format": "uuid"},
                "specialist_role": {
                    "type": "string",
                    "enum": [
                        "pentester", "researcher", "browser", "coder", "installer",
                        "enricher", "memorist", "adviser"
                    ]
                },
                "summary": {"type": "string", "minLength": 1, "maxLength": 4096},
                "cited_evidence_ids": {
                    "type": "array", "maxItems": 256, "uniqueItems": true,
                    "items": {"type": "integer", "minimum": 1}
                },
                "new_hypothesis_proposals": {
                    "type": "array", "maxItems": 64,
                    "items": asset_verification_hypothesis_proposal_schema()
                }
            },
            "required": [
                "schema_version", "session_id", "hypothesis_revision_id", "actor_call_id",
                "actor_ordinal", "subtask_id", "specialist_role", "summary",
                "cited_evidence_ids", "new_hypothesis_proposals"
            ],
            "additionalProperties": false
        })
    } else if stage_team_output_schema
        == Some(INVESTIGATION_DYNAMIC_VERIFICATION_PRIMARY_TURN_SCHEMA)
    {
        let common_properties = serde_json::json!({
            "schema_version": {"type": "integer", "enum": [1]},
            "session_id": {"type": "string", "format": "uuid"},
            "hypothesis_revision_id": {"type": "string", "format": "uuid"}
        });
        let mut delegate_properties = common_properties
            .as_object()
            .expect("dynamic Verification Primary properties are an object")
            .clone();
        delegate_properties.insert(
            "decision".to_string(),
            serde_json::json!({"type": "string", "enum": ["delegate"]}),
        );
        delegate_properties.insert(
            "subtasks".to_string(),
            serde_json::json!({
                "type": "array", "minItems": 1, "maxItems": 8,
                "items": dynamic_verification_subtask_schema()
            }),
        );

        let mut resolve_properties = common_properties
            .as_object()
            .expect("dynamic Verification Primary properties are an object")
            .clone();
        resolve_properties.insert(
            "decision".to_string(),
            serde_json::json!({"type": "string", "enum": ["resolve"]}),
        );
        resolve_properties.insert(
            "subtasks".to_string(),
            serde_json::json!({"type": "array", "maxItems": 0}),
        );
        resolve_properties.insert(
            "disposition".to_string(),
            serde_json::json!({"type": "string", "enum": ["verified", "refuted", "invalid"]}),
        );
        resolve_properties.insert(
            "conclusion".to_string(),
            serde_json::json!({"type": "string", "minLength": 1, "maxLength": 8192}),
        );
        resolve_properties.insert(
            "cited_evidence_ids".to_string(),
            serde_json::json!({
                "type": "array", "maxItems": 256, "uniqueItems": true,
                "items": {"type": "integer", "minimum": 1}
            }),
        );
        resolve_properties.insert(
            "new_hypothesis_proposals".to_string(),
            serde_json::json!({
                "type": "array", "maxItems": 64,
                "items": asset_verification_hypothesis_proposal_schema()
            }),
        );

        serde_json::json!({
            "description": "One closed semantic turn by the exact Asset Verification Primary: delegate an ordered dynamic batch, or resolve the current hypothesis without creating an actor.",
            "oneOf": [
                {
                    "type": "object",
                    "properties": delegate_properties,
                    "required": [
                        "schema_version", "session_id", "hypothesis_revision_id",
                        "decision", "subtasks"
                    ],
                    "additionalProperties": false
                },
                {
                    "type": "object",
                    "properties": resolve_properties,
                    "required": [
                        "schema_version", "session_id", "hypothesis_revision_id",
                        "decision", "subtasks", "disposition", "conclusion",
                        "cited_evidence_ids", "new_hypothesis_proposals"
                    ],
                    "additionalProperties": false
                }
            ]
        })
    } else if stage_team_output_schema
        == Some(INVESTIGATION_ASSET_VERIFICATION_PRIMARY_RESOLUTION_SCHEMA)
    {
        serde_json::json!({
            "type": "object",
            "description": "The durable Asset Primary's semantic terminal decision. The host validates the current revision and cited session evidence and supplies all durable authority ids.",
            "properties": {
                "schema_version": {"type": "integer", "enum": [1]},
                "session_id": {"type": "string", "format": "uuid"},
                "hypothesis_revision_id": {"type": "string", "format": "uuid"},
                "disposition": {"type": "string", "enum": ["verified", "refuted", "invalid"]},
                "conclusion": {"type": "string", "minLength": 1, "maxLength": 8192},
                "citations": {
                    "type": "array", "maxItems": 256,
                    "items": asset_verification_citation_schema()
                },
                "new_hypothesis_proposals": {
                    "type": "array", "maxItems": 64,
                    "items": asset_verification_hypothesis_proposal_schema()
                }
            },
            "required": [
                "schema_version", "session_id", "hypothesis_revision_id", "disposition",
                "conclusion", "citations", "new_hypothesis_proposals"
            ],
            "additionalProperties": false
        })
    } else if let Some(output_schema) = stage_team_output_schema {
        let mut schema = serde_json::json!({
            "type": "object",
            "description": "The exact Stage Team business result. Return this object directly; do not wrap it in Markdown or prose.",
            "properties": {
                "business_disposition": {
                    "type": "string",
                    "enum": ["found", "checked_empty", "blocked"],
                    "description": "Business outcome for this WorkItem"
                },
                "summary": {
                    "type": "string",
                    "description": "Concise evidence-grounded outcome summary"
                },
                "fact_refs": {
                    "type": "array",
                    "items": {"type": "object"},
                    "description": "Typed canonical fact references returned by tools; use [] when none were returned"
                },
                "evidence_ids": {
                    "type": "array",
                    "items": {"type": "integer", "minimum": 1},
                    "description": "IDs of evidence already booked in the evidence ledger"
                },
                "checked_empty_units": {
                    "type": "array",
                    "items": {"type": "object"},
                    "description": "Exact provider or asset subunits checked empty; use [] when none"
                },
                "blocker_code": {
                    "type": ["string", "null"],
                    "description": "Stable blocker code for blocked outcomes, otherwise null"
                }
            },
            "required": [
                "business_disposition",
                "summary",
                "fact_refs",
                "evidence_ids",
                "checked_empty_units",
                "blocker_code"
            ],
            "additionalProperties": false
        });
        if output_schema == "investigation_cognitive_output.v1" {
            let properties = schema
                .get_mut("properties")
                .and_then(serde_json::Value::as_object_mut)
                .expect("barrier schema properties are an object");
            properties.insert(
                "business_disposition".to_string(),
                serde_json::json!({
                    "type": "string",
                    "enum": ["found", "blocked"],
                    "description": "Successful cognition is found advisory material; blocked is reserved for a typed provider/runtime blocker"
                }),
            );
            for field in ["fact_refs", "evidence_ids", "checked_empty_units"] {
                let property = properties
                    .get_mut(field)
                    .and_then(serde_json::Value::as_object_mut)
                    .expect("authority-bearing barrier property is an object");
                property.insert("maxItems".to_string(), serde_json::json!(0));
                property.insert(
                    "description".to_string(),
                    serde_json::json!("Must be empty for cognition-only Investigation output; inherited subject refs remain sealed read-only context and are never re-emitted as worker authority"),
                );
            }
            properties.insert(
                "proposal_signals".to_string(),
                serde_json::json!({
                    "type": "array",
                    "items": {"type": "object"},
                    "description": "Advisory typed proposal signals for the Investigation host reducer"
                }),
            );
            properties.insert(
                "action_intents".to_string(),
                serde_json::json!({
                    "type": "array",
                    "items": {"type": "object"},
                    "description": "Advisory non-executable action intents for the Investigation host reducer"
                }),
            );
            properties.insert(
                "residuals".to_string(),
                serde_json::json!({
                    "type": "array",
                    "items": {"type": "object"},
                    "description": "Typed unresolved Investigation obligations"
                }),
            );
            let required = schema
                .get_mut("required")
                .and_then(serde_json::Value::as_array_mut)
                .expect("barrier schema required is an array");
            required.extend([
                serde_json::json!("proposal_signals"),
                serde_json::json!("action_intents"),
                serde_json::json!("residuals"),
            ]);
        }
        schema
    } else {
        serde_json::json!({
            "type": "string",
            "description": "Your complete result: findings, outputs, code, data, or error details"
        })
    };
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
                "result": result_schema,
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
    fn push_dispatch_if_available(
        tools: &mut Vec<ToolDefinition>,
        binding: Option<&StageTeamLeaderBinding>,
    ) {
        if binding.is_some_and(|binding| {
            binding.controller_action_compiler.as_deref() != Some("enumeration_v2")
                || !binding.compiled_actions.is_empty()
        }) {
            tools.push(stage_team_dispatch_workers_tool_definition(binding));
        }
    }
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
        update_plan.description = if binding.is_some_and(|binding| binding.planning_only) {
            "Create or revise this Investigation Task Primary's bounded cognitive plan. The plan is checkpointed on this exact Primary chain and is advisory until submit_result returns the typed ordered subtask set for host validation. At most one item may be in_progress.".to_string()
        } else {
            "Create or update this Company Controller's local working plan. The plan is recorded only in this durable Controller message chain; it does not update the global task plan. At most one plan item may be in_progress. Plan status tracks the Controller's current focus, not tool or worker concurrency: when multiple tools or workers run in parallel, describe that batch in one composite in_progress step instead of marking one in_progress step per operation.".to_string()
        };
        let Some(plan_schema) = update_plan
            .parameters
            .pointer_mut("/properties/plan")
            .and_then(serde_json::Value::as_object_mut)
        else {
            tracing::error!("static update_plan catalogue entry has no plan schema");
            push_dispatch_if_available(tools, binding);
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
            push_dispatch_if_available(tools, binding);
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
            push_dispatch_if_available(tools, binding);
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
    if binding.is_some_and(|binding| binding.planning_only) {
        return;
    }
    push_dispatch_if_available(tools, binding);
    tools.push(stage_team_prepare_final_submission_tool_definition());
}

fn stage_team_dispatch_workers_tool_definition(
    binding: Option<&StageTeamLeaderBinding>,
) -> ToolDefinition {
    if binding.is_some_and(|binding| {
        binding.controller_action_compiler.as_deref() == Some("enumeration_v2")
    }) {
        let action_ids = binding
            .into_iter()
            .flat_map(|binding| binding.compiled_actions.iter())
            .map(|action| action.action_id.as_str())
            .collect::<Vec<_>>();
        return ToolDefinition {
            name: STAGE_TEAM_DISPATCH_WORKERS_TOOL_NAME.to_string(),
            description: "Choose one or more currently ready Enumeration actions. The host compiles every target, exact origin, tool argument, receipt dependency, evidence manifest, budget, and replay key; the Controller supplies only an opaque action id and planning rationale.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "workers": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 32,
                        "items": {
                            "type": "object",
                            "properties": {
                                "action_id": {
                                    "type": "string",
                                    "enum": action_ids,
                                    "description": "Opaque id from the current server-authored ready-action catalogue."
                                },
                                "rationale": {
                                    "type": "string",
                                    "minLength": 1,
                                    "description": "Why this ready action is the next step in the Controller's plan. This prose never becomes execution authority."
                                }
                            },
                            "required": ["action_id", "rationale"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["workers"],
                "additionalProperties": false
            }),
        };
    }
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
                                "description": "Optional canonical target references inside this company's frozen scope. Use exact objects shaped as {\"kind\":\"target\",\"target_id\":\"<uuid>\"}; never put target_url in a subject ref. Omit subject_refs only for an intentional whole-company assignment.",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "kind": {
                                            "type": "string",
                                            "enum": ["target"]
                                        },
                                        "target_id": {
                                            "type": "string",
                                            "minLength": 1
                                        }
                                    },
                                    "required": ["kind", "target_id"],
                                    "additionalProperties": false
                                }
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
    use crate::executor_types::{StageTeamCompiledActionBinding, StageTeamLeaderBinding};
    use std::sync::Arc;
    use uuid::Uuid;

    fn td(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: String::new(),
            parameters: serde_json::json!({}),
        }
    }

    #[test]
    fn subagent_fixture_receives_only_host_owned_intel_public_tools() {
        let mut tools = vec![td("web_search"), td("web_fetch"), td("submit_result")];
        configure_intel_public_fixture_tools(
            &mut tools,
            Some(vec![td("intel_public_search"), td("intel_public_fetch")]),
        );
        let names = tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert!(names.contains(&"intel_public_search"));
        assert!(names.contains(&"intel_public_fetch"));
        assert!(!names.contains(&"web_search"));
        assert!(!names.contains(&"web_fetch"));
        assert!(names.contains(&"submit_result"));
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
    fn target_intel_reviewer_exposes_only_ordered_reads_and_durable_verdict() {
        let tools = target_intel_reviewer_tool_definitions();
        assert_eq!(
            tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            vec![
                crate::TARGET_INTEL_READ_REVIEW_SECTION,
                crate::TARGET_INTEL_RECORD_REVIEW_VERDICT,
            ]
        );
        assert!(tools.iter().all(|tool| tool.name != BARRIER_TOOL_NAME));
        assert!(tools[1].description.contains("sole terminal barrier"));
    }

    #[test]
    fn closed_final_submitter_exposes_only_the_stage_deliverable_writer() {
        let tools = final_submission_only_tool_definitions(vec![
            td("query_target_data"),
            td(BARRIER_TOOL_NAME),
            td("submit_stage_deliverable"),
            td("stage_team_prepare_final_submission"),
        ]);

        assert_eq!(
            tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            vec!["submit_stage_deliverable"]
        );
    }

    #[test]
    fn stage_team_child_barrier_requires_a_typed_worker_output_object() {
        let stage_child = barrier_tool_definition(Some("stage_worker_output.v1"));
        let result = &stage_child.parameters["properties"]["result"];

        assert_eq!(result["type"], "object");
        assert_eq!(
            result["required"],
            serde_json::json!([
                "business_disposition",
                "summary",
                "fact_refs",
                "evidence_ids",
                "checked_empty_units",
                "blocker_code"
            ])
        );
        assert_eq!(result["additionalProperties"], false);
        assert_eq!(
            result["properties"]["business_disposition"]["enum"],
            serde_json::json!(["found", "checked_empty", "blocked"])
        );

        let generic = barrier_tool_definition(None);
        assert_eq!(generic.parameters["properties"]["result"]["type"], "string");
    }

    #[test]
    fn investigation_child_barrier_requires_cognitive_advisory_fields() {
        let investigation = barrier_tool_definition(Some("investigation_cognitive_output.v1"));
        let result = &investigation.parameters["properties"]["result"];

        for field in ["proposal_signals", "action_intents", "residuals"] {
            assert_eq!(result["properties"][field]["type"], "array");
            assert!(result["required"]
                .as_array()
                .is_some_and(|required| required.contains(&serde_json::json!(field))));
        }
        for field in ["fact_refs", "evidence_ids", "checked_empty_units"] {
            assert_eq!(result["properties"][field]["maxItems"], 0);
        }
        assert_eq!(
            result["properties"]["business_disposition"]["enum"],
            serde_json::json!(["found", "blocked"])
        );
        assert_eq!(result["additionalProperties"], false);
    }

    #[test]
    fn asset_verification_surface_keeps_dynamic_wrappers_cognition_and_one_barrier() {
        let definition = |name: &str| ToolDefinition {
            name: name.to_string(),
            description: name.to_string(),
            parameters: serde_json::json!({"type":"object"}),
        };
        let tools = asset_verification_tool_definitions(
            [
                "pentest_list_tools",
                "pentest_read_skill",
                "pentest_run",
                "browser_collect_js_api",
                "query_target_data",
                "search_knowledge_base",
                "submit_result",
                "submit_stage_deliverable",
                "sub_agent_coder",
                "browser_navigate",
                "run_pty_cmd",
                "write_file",
                "pentest_run",
            ]
            .into_iter()
            .map(definition)
            .collect(),
            None,
        );
        let names = tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();

        for expected in [
            "pentest_list_tools",
            "pentest_read_skill",
            "pentest_run",
            "browser_collect_js_api",
            "query_target_data",
            "search_knowledge_base",
            "submit_result",
        ] {
            assert_eq!(
                names.iter().filter(|name| **name == expected).count(),
                1,
                "{expected} must appear exactly once"
            );
        }
        for forbidden in [
            "submit_stage_deliverable",
            "sub_agent_coder",
            "browser_navigate",
            "run_pty_cmd",
            "write_file",
        ] {
            assert!(!names.contains(&forbidden));
        }
    }

    #[test]
    fn asset_verification_actor_observation_schema_is_call_bound_and_dynamic_role_bounded() {
        let tool = barrier_tool_definition(Some(
            INVESTIGATION_ASSET_VERIFICATION_ACTOR_OBSERVATION_SCHEMA,
        ));
        let result = &tool.parameters["properties"]["result"];
        assert_eq!(result["properties"]["actor_ordinal"]["minimum"], 1);
        assert!(result["properties"]["specialist_role"]["enum"]
            .as_array()
            .is_some_and(|roles| roles.contains(&serde_json::json!("coder"))));
        assert_eq!(result["properties"]["summary"]["minLength"], 1);
        assert_eq!(
            result["properties"]["cited_evidence_ids"]["uniqueItems"],
            true
        );
        assert_eq!(
            result["properties"]["cited_evidence_ids"]["items"]["minimum"],
            1
        );
        assert_eq!(
            result["properties"]["new_hypothesis_proposals"]["items"]["additionalProperties"],
            false
        );
        assert_eq!(result["additionalProperties"], false);
        for required in [
            "schema_version",
            "session_id",
            "hypothesis_revision_id",
            "actor_call_id",
            "actor_ordinal",
            "subtask_id",
            "specialist_role",
            "summary",
            "cited_evidence_ids",
            "new_hypothesis_proposals",
        ] {
            assert!(result["required"]
                .as_array()
                .is_some_and(|fields| fields.contains(&serde_json::json!(required))));
        }
    }

    #[test]
    fn dynamic_verification_primary_turn_schema_is_closed_delegate_or_resolve() {
        let tool =
            barrier_tool_definition(Some(INVESTIGATION_DYNAMIC_VERIFICATION_PRIMARY_TURN_SCHEMA));
        let result = &tool.parameters["properties"]["result"];
        let branches = result["oneOf"]
            .as_array()
            .expect("Primary turn has two tagged branches");
        assert_eq!(branches.len(), 2);

        let delegate = &branches[0];
        assert_eq!(
            delegate["properties"]["decision"]["enum"],
            serde_json::json!(["delegate"])
        );
        assert_eq!(delegate["properties"]["subtasks"]["minItems"], 1);
        assert_eq!(delegate["properties"]["subtasks"]["maxItems"], 8);
        assert_eq!(
            delegate["properties"]["subtasks"]["items"]["properties"]["role"]["enum"],
            serde_json::json!([
                "pentester",
                "researcher",
                "browser",
                "coder",
                "installer",
                "enricher",
                "memorist",
                "adviser"
            ])
        );
        let subject_refs =
            &delegate["properties"]["subtasks"]["items"]["properties"]["subject_refs"];
        assert_eq!(subject_refs["minItems"], 2);
        assert_eq!(subject_refs["maxItems"], 2);
        assert_eq!(
            subject_refs["prefixItems"][0]["properties"]["kind"]["enum"],
            serde_json::json!(["target"])
        );
        assert_eq!(
            subject_refs["prefixItems"][1]["properties"]["kind"]["enum"],
            serde_json::json!(["hypothesis_revision"])
        );

        let resolve = &branches[1];
        assert_eq!(
            resolve["properties"]["decision"]["enum"],
            serde_json::json!(["resolve"])
        );
        assert_eq!(resolve["properties"]["subtasks"]["maxItems"], 0);
        assert!(resolve["properties"]["cited_evidence_ids"]
            .get("minItems")
            .is_none());
        assert_eq!(
            resolve["properties"]["cited_evidence_ids"]["uniqueItems"],
            true
        );
        assert!(!serde_json::to_string(result)
            .expect("schema serializes")
            .contains("authority_id"));
        assert!(branches
            .iter()
            .all(|branch| branch["additionalProperties"] == false));
    }

    #[test]
    fn asset_verification_primary_resolution_keeps_host_authority_out_of_model_schema() {
        let tool = barrier_tool_definition(Some(
            INVESTIGATION_ASSET_VERIFICATION_PRIMARY_RESOLUTION_SCHEMA,
        ));
        let result = &tool.parameters["properties"]["result"];
        assert_eq!(
            result["properties"]["disposition"]["enum"],
            serde_json::json!(["verified", "refuted", "invalid"])
        );
        assert!(result["properties"]["citations"].get("minItems").is_none());
        for host_owned in [
            "resolution_authority_id",
            "adviser_review_output_id",
            "adviser_review_output_sha256",
            "primary_conclusion_sha256",
            "adviser_concurrence_sha256",
        ] {
            assert!(result["properties"].get(host_owned).is_none());
        }
        assert_eq!(result["additionalProperties"], false);
    }

    #[test]
    fn investigation_primary_barriers_expose_each_host_phase_contract() {
        let plan = barrier_tool_definition(Some(INVESTIGATION_TASK_PLAN_RESULT_SCHEMA));
        let plan_result = &plan.parameters["properties"]["result"];
        assert_eq!(
            plan_result["required"],
            serde_json::json!(["schema_version", "summary", "subtasks"])
        );
        assert_eq!(plan_result["properties"]["subtasks"]["minItems"], 0);
        assert_eq!(plan_result["properties"]["subtasks"]["maxItems"], 8);
        assert_eq!(
            plan_result["properties"]["subtasks"]["items"]["properties"]["role"]["enum"],
            serde_json::json!([
                "pentester",
                "researcher",
                "browser",
                "coder",
                "installer",
                "enricher",
                "memorist",
                "adviser"
            ])
        );

        let refiner = barrier_tool_definition(Some(INVESTIGATION_REFINER_PATCH_RESULT_SCHEMA));
        let refiner_result = &refiner.parameters["properties"]["result"];
        assert!(refiner_result["required"].as_array().is_some_and(
            |required| required.contains(&serde_json::json!("accepted_output_sha256"))
        ));
        assert_eq!(
            refiner_result["properties"]["remaining_subtasks"]["maxItems"],
            8
        );

        let synthesis =
            barrier_tool_definition(Some(INVESTIGATION_PRIMARY_SYNTHESIS_RESULT_SCHEMA));
        let synthesis_result = &synthesis.parameters["properties"]["result"];
        for field in [
            "accepted_output_sha256",
            "proposal_signals",
            "action_intents",
            "residuals",
        ] {
            assert!(synthesis_result["required"]
                .as_array()
                .is_some_and(|required| required.contains(&serde_json::json!(field))));
        }
        assert!(synthesis_result["properties"]
            .get("business_disposition")
            .is_none());
        assert_eq!(synthesis_result["additionalProperties"], false);
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
    fn stage_team_dispatch_schema_requires_canonical_target_refs() {
        let tool = stage_team_dispatch_workers_tool_definition(None);
        let subject_ref = &tool.parameters["properties"]["workers"]["items"]["properties"]
            ["subject_refs"]["items"];

        assert_eq!(subject_ref["type"], "object");
        assert_eq!(
            subject_ref["required"],
            serde_json::json!(["kind", "target_id"])
        );
        assert_eq!(
            subject_ref["properties"]["kind"]["enum"],
            serde_json::json!(["target"])
        );
        assert_eq!(subject_ref["additionalProperties"], false);
        assert!(subject_ref["properties"].get("target_url").is_none());
    }

    #[test]
    fn enumeration_reducers_expose_only_exact_subject_inputs() {
        for name in [
            ENUMERATION_REDUCE_PARAMETERS_TOOL_NAME,
            ENUMERATION_REVIEW_COVERAGE_TOOL_NAME,
        ] {
            let tool = enumeration_receipt_reducer_tool_definition(name);
            assert_eq!(
                tool.parameters["required"],
                serde_json::json!(["target_id", "exact_origin"])
            );
            assert_eq!(tool.parameters["additionalProperties"], false);
            assert!(tool.parameters["properties"]
                .get("dependency_receipt_ids")
                .is_none());
            assert!(tool.parameters["properties"]
                .get("evidence_audit_ids")
                .is_none());
        }
    }

    #[test]
    fn enumeration_dispatch_schema_exposes_only_current_opaque_action_ids() {
        let binding = StageTeamLeaderBinding {
            stage_team_plan_id: Uuid::new_v4(),
            leader_work_item_id: Uuid::new_v4(),
            expected_dispatch_epoch: 3,
            expected_plan_row_version: 5,
            expected_work_item_row_version: 7,
            controller_action_compiler: Some("enumeration_v2".to_string()),
            compiled_actions: vec![StageTeamCompiledActionBinding {
                action_id: "ready-action-1".to_string(),
                dedupe_key: "trusted-dedupe".to_string(),
                requested_role: "browser_runtime".to_string(),
                requested_kind: "formulaic_enumeration".to_string(),
                objective: "host-only objective".to_string(),
                subject_refs: vec![serde_json::json!({
                    "kind":"target",
                    "target_id":Uuid::new_v4()
                })],
                budget_hint: serde_json::json!({"max_wrapper_calls":1}),
            }],
            planning_only: false,
        };
        let tool = stage_team_dispatch_workers_tool_definition(Some(&binding));
        let item = &tool.parameters["properties"]["workers"]["items"];

        assert_eq!(
            item["required"],
            serde_json::json!(["action_id", "rationale"])
        );
        assert_eq!(
            item["properties"]["action_id"]["enum"],
            serde_json::json!(["ready-action-1"])
        );
        assert!(item["properties"].get("role").is_none());
        assert!(item["properties"].get("kind").is_none());
        assert!(item["properties"].get("objective").is_none());
        assert!(item["properties"].get("subject_refs").is_none());
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
            controller_action_compiler: None,
            compiled_actions: Vec::new(),
            planning_only: false,
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

    #[test]
    fn investigation_planning_primary_gets_update_plan_without_company_router() {
        let binding = StageTeamLeaderBinding {
            stage_team_plan_id: Uuid::new_v4(),
            leader_work_item_id: Uuid::new_v4(),
            expected_dispatch_epoch: 0,
            expected_plan_row_version: 1,
            expected_work_item_row_version: 1,
            controller_action_compiler: None,
            compiled_actions: Vec::new(),
            planning_only: true,
        };
        let mut tools = Vec::new();
        configure_stage_team_leader_tools(
            &mut tools,
            true,
            Some(&binding),
            Some(update_plan_catalog_definition()),
        );

        assert_eq!(
            tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            vec!["update_plan"]
        );
        assert!(tools[0].description.contains("Investigation Task Primary"));
    }
}
