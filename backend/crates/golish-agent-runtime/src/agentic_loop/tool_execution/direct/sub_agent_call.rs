//! `execute_sub_agent_call` — handles sub-agent tool calls (tool names
//! starting with `sub_agent_`), branching between built-in execution and the
//! registry-driven dispatch path, with best-effort dispatch lifecycle
//! persistence.

use anyhow::Result;
use rig::completion::CompletionModel as RigCompletionModel;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashSet};

use golish_agent_kit::db_traits::{
    CommitEnumerationBrowserProducerV2, CommitEnumerationJsApiProducerV2,
    EnumerationBrowserProducerArtifactV2, EnumerationJsApiProducerArtifactV2,
    EnumerationLaneClosureReceiptV2, EnumerationLaneKindV2, ReadTargetIntelReviewSection,
    RecordTargetIntelReviewVerdict, RecoverEnumerationLaneReceiptV2, ReduceEnumerationParameterV2,
    RequestStageWorker, ReviewEnumerationCoverageV2, RuntimeMemoryRepository, RuntimeWorkerFence,
    StageWorkerRequestDecision,
};
use golish_agent_kit::harness::{CanonicalFactKey, IntelReviewSectionKind, IntelReviewVerdict};
use golish_agent_kit::planner::{PlanManager, StepStatus, UpdatePlanArgs};
use golish_agent_kit::task_orchestrator::agent_run_checkpoint::{
    agent_run_from_state_blob, state_blob_with_agent_run, state_blob_without_agent_run,
    AgentRunCheckpoint, AgentRunStatus, RuntimeCorrectionCheckpoint, ToolCheckpoint,
    ToolCheckpointState,
};
use golish_agent_kit::task_orchestrator::runtime_supervisor::{
    directive_from_model_response, runtime_supervisor_system_prompt,
    runtime_supervisor_user_prompt, RuntimeSupervisorContext,
};
use golish_agent_kit::task_orchestrator::stage_refiner::{
    refine_submit_needs_fix, RefinerContext, RepairDirective,
};
use golish_core::events::{AiEvent, HarnessTraceKind};
use golish_core::utils::truncate_str;
use golish_sub_agents::{
    execute_sub_agent, BoundWorkerChainContext, SubAgentChainError, SubAgentContext,
    SubAgentDefinition, SubAgentExecutorContext, SubAgentToolObservation, SubmitRepairMode,
    STAGE_TEAM_DISPATCH_ACCEPTED_STATUS, STAGE_TEAM_DISPATCH_WORKERS_TOOL_NAME,
    STAGE_TEAM_PREPARE_FINAL_STATUS, STAGE_TEAM_PREPARE_FINAL_SUBMISSION_TOOL_NAME,
    STAGE_TEAM_UPDATE_PLAN_TOOL_NAME,
};

use super::super::super::context::{retrieve_scoped_context_data, BoundScopedContextIdentity};
use super::super::super::llm_helpers::runtime_supervisor_one_shot;
use super::super::super::sub_agent_dispatch::{
    build_sub_agent_briefing, execute_sub_agent_with_client,
};
use super::super::super::{AgenticLoopContext, ToolExecutionResult};
use super::stage_team_scheduler::sha256_json;
use golish_agent_kit::tool_executors::extract_and_upsert_entities;
use golish_agent_kit::tool_provider_impl::DefaultToolProvider;

const MAX_STAGE_TEAM_CONTROLLER_DISPATCH_BATCH: usize = 32;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StageTeamDispatchWorkersArgs {
    workers: Vec<StageTeamDispatchWorkerArgs>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StageTeamDispatchWorkerArgs {
    dedupe_key: String,
    role: String,
    kind: String,
    objective: String,
    #[serde(default)]
    subject_refs: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnumerationDispatchWorkersArgs {
    workers: Vec<EnumerationDispatchWorkerArgs>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnumerationDispatchWorkerArgs {
    action_id: String,
    rationale: String,
}

#[derive(Serialize)]
struct StageTeamControllerRequestEnvelope<'a> {
    schema: &'static str,
    parent_tool_request_id: &'a str,
    objective: &'a str,
}

fn stage_team_leader_router_error(code: &'static str, error: impl Into<String>) -> (Value, bool) {
    (json!({"code": code, "error": error.into()}), false)
}

fn canonicalize_stage_team_subject_refs(subject_refs: &[Value]) -> Result<Vec<Value>, String> {
    let mut canonical_refs = Vec::with_capacity(subject_refs.len());
    let mut seen = HashSet::with_capacity(subject_refs.len());

    for subject_ref in subject_refs {
        let canonical_key = match serde_json::from_value::<CanonicalFactKey>(subject_ref.clone()) {
            Ok(canonical_key) => canonical_key,
            Err(_) => {
                let Some(selector) = subject_ref.as_object() else {
                    return Err(
                        "subject_refs must contain canonical objects such as {\"kind\":\"target\",\"target_id\":\"<uuid>\"}"
                            .to_string(),
                    );
                };
                if !selector
                    .keys()
                    .all(|key| matches!(key.as_str(), "target_id" | "target_url"))
                    || selector
                        .get("target_url")
                        .is_some_and(|target_url| !target_url.is_string())
                {
                    return Err(
                        "non-canonical subject ref; target shorthand may contain target_id and target_url only"
                            .to_string(),
                    );
                }
                let target_id = selector
                    .get("target_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "target shorthand requires a UUID target_id".to_string())?
                    .parse::<uuid::Uuid>()
                    .map_err(|_| "target shorthand requires a UUID target_id".to_string())?;
                CanonicalFactKey::Target { target_id }
            }
        };
        let canonical_ref = serde_json::to_value(canonical_key)
            .map_err(|error| format!("canonical subject ref was not serializable: {error}"))?;
        let dedupe_key = serde_json::to_string(&canonical_ref)
            .map_err(|error| format!("canonical subject ref was not serializable: {error}"))?;
        if seen.insert(dedupe_key) {
            canonical_refs.push(canonical_ref);
        }
    }

    Ok(canonical_refs)
}

fn stage_team_dispatch_assignment_identity(
    worker: &StageTeamDispatchWorkerArgs,
) -> Result<String, String> {
    let mut subject_refs = worker
        .subject_refs
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("canonical subject ref was not serializable: {error}"))?;
    subject_refs.sort_unstable();
    serde_json::to_string(&json!({
        "kind": worker.kind.trim(),
        "objective": worker.objective.split_whitespace().collect::<Vec<_>>().join(" "),
        "role": worker.role.trim(),
        "subject_refs": subject_refs,
    }))
    .map_err(|error| format!("Stage Team assignment identity was not serializable: {error}"))
}

fn stage_team_leader_tool_context_matches(
    tool_name: &str,
    bound: &BoundWorkerChainContext,
    context: &golish_core::AgentToolContext,
) -> bool {
    !context.request_id.trim().is_empty()
        && context.tool_call_record_id.is_some()
        && context.tool_name == tool_name
        && context.operation_id == Some(bound.operation_id)
        && context.stage_execution_id == Some(bound.stage_execution_id)
        && context.stage_run_unit_id == Some(bound.worker_lease.stage_run_unit_id)
        && context.organization_id == Some(bound.organization_id)
        && context.worker_lease.as_ref() == Some(&bound.worker_lease)
}

fn enumeration_producer_tool_context(
    tool_name: &str,
) -> Result<golish_core::AgentToolContext, (Value, bool)> {
    let Some(context) = golish_core::current_agent_tool_context() else {
        return Err(stage_team_leader_router_error(
            "ENUMERATION_V2_PRODUCER_CONTEXT_REQUIRED",
            "Enumeration producer settlement requires the live trusted tool context",
        ));
    };
    if context.tool_name != tool_name
        || context.tool_call_record_id.is_none()
        || context.operation_id.is_none()
        || context.stage_execution_id.is_none()
        || context.stage_run_unit_id.is_none()
        || context.organization_id.is_none()
        || context.worker_lease.is_none()
    {
        return Err(stage_team_leader_router_error(
            "ENUMERATION_V2_PRODUCER_CONTEXT_MISMATCH",
            "Enumeration producer context is not bound to one live Worker ToolCall",
        ));
    }
    Ok(context)
}

fn enumeration_result_target_and_origin(result: &Value) -> Result<(uuid::Uuid, String), String> {
    let target_id = result
        .get("target_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "producer result is missing target_id".to_string())?
        .parse::<uuid::Uuid>()
        .map_err(|_| "producer result target_id is not a UUID".to_string())?;
    let exact_origin = ["effective_target_url", "target_url", "requested_target_url"]
        .into_iter()
        .find_map(|key| result.get(key).and_then(Value::as_str))
        .filter(|origin| !origin.trim().is_empty())
        .ok_or_else(|| "producer result is missing its exact Web Origin".to_string())?;
    let exact_origin = golish_pentest_domain::canonical_web_origin(exact_origin)
        .map(|origin| origin.key)
        .ok_or_else(|| "producer result exact Web Origin is not canonicalizable".to_string())?;
    Ok((target_id, exact_origin))
}

fn enumeration_producer_args_target_and_origin(
    args: &Value,
) -> Result<(uuid::Uuid, String), String> {
    if args.get("target_urls").is_some() {
        return Err(
            "Enumeration v2 producer prerequisites require one exact target_id and target_url"
                .to_string(),
        );
    }
    let target_id = args
        .get("target_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "producer arguments are missing target_id".to_string())?
        .parse::<uuid::Uuid>()
        .map_err(|_| "producer argument target_id is not a UUID".to_string())?;
    let requested_origin = args
        .get("target_url")
        .and_then(Value::as_str)
        .filter(|origin| !origin.trim().is_empty())
        .ok_or_else(|| "producer arguments are missing target_url".to_string())?;
    let exact_origin = golish_pentest_domain::canonical_web_origin(requested_origin)
        .map(|origin| origin.key)
        .ok_or_else(|| "producer argument target_url is not a canonical Web Origin".to_string())?;
    Ok((target_id, exact_origin))
}

fn enumeration_receipt_result(
    mut result: Value,
    receipt: EnumerationLaneClosureReceiptV2,
) -> Value {
    if let Some(object) = result.as_object_mut() {
        object.insert(
            "lane_closure_receipt_v2".to_string(),
            serde_json::to_value(receipt)
                .unwrap_or_else(|error| json!({"serialization_error": error.to_string()})),
        );
        object.insert("typed_receipt_committed".to_string(), json!(true));
    }
    result
}

async fn settle_enumeration_producer_result(
    repo: Option<&std::sync::Arc<dyn golish_agent_kit::db_traits::DbRepoProvider>>,
    tool_name: &str,
    result: Value,
    success: bool,
) -> (Value, bool) {
    let artifact_key = match tool_name {
        "browser_collect_js_api" => "enumeration_browser_producer_artifact_v2",
        "js_extract_apis" => "enumeration_js_api_producer_artifact_v2",
        _ => return (result, success),
    };
    if !success {
        return (result, false);
    }
    // The v2 receipt identifies one exact Target + Web Origin. Batch adapters
    // compact nested results and cannot expose one trustworthy artifact root;
    // Main AI must call these producers once per exact origin.
    if result.get("results").is_some() {
        return (
            json!({
                "code": "ENUMERATION_V2_SINGLE_EXACT_ORIGIN_REQUIRED",
                "error": "Enumeration v2 producer calls must contain exactly one target_id and exact Web Origin; split the batch and retry",
                "completion_state": "partial",
                "outcome_persisted": false,
            }),
            false,
        );
    }
    if result.get("completion_state").and_then(Value::as_str) != Some("complete")
        || result.get("outcome_persisted").and_then(Value::as_bool) != Some(true)
    {
        // A normal partial/checkpoint is planning feedback, not an exhausted
        // formulaic attempt and not a receipt-worthy terminal assertion.
        return (result, true);
    }
    let Some(repo) = repo else {
        return stage_team_leader_router_error(
            "ENUMERATION_V2_PRODUCER_REPOSITORY_REQUIRED",
            "Enumeration producer completed but no durable receipt repository is bound",
        );
    };
    let context = match enumeration_producer_tool_context(tool_name) {
        Ok(context) => context,
        Err(error) => return error,
    };
    let (target_id, exact_origin) = match enumeration_result_target_and_origin(&result) {
        Ok(subject) => subject,
        Err(error) => {
            return stage_team_leader_router_error("ENUMERATION_V2_PRODUCER_SUBJECT_MISSING", error)
        }
    };
    let operation_id = context.operation_id.expect("validated producer operation");
    let organization_id = context
        .organization_id
        .expect("validated producer organization");
    let stage_execution_id = context
        .stage_execution_id
        .expect("validated producer stage execution");
    let stage_run_unit_id = context
        .stage_run_unit_id
        .expect("validated producer stage unit");
    let source_tool_call_id = context
        .tool_call_record_id
        .expect("validated producer tool call");
    let worker = context
        .worker_lease
        .expect("validated producer worker lease");

    let receipt = if tool_name == "browser_collect_js_api" {
        let artifact = match result
            .get(artifact_key)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("completed producer result omitted {artifact_key}"))
            .and_then(|value| {
                serde_json::from_value::<EnumerationBrowserProducerArtifactV2>(value)
                    .map_err(Into::into)
            }) {
            Ok(artifact) => artifact,
            Err(error) => {
                return stage_team_leader_router_error(
                    "ENUMERATION_V2_PRODUCER_ARTIFACT_MISSING",
                    error.to_string(),
                )
            }
        };
        let stable_request_id =
            uuid::Uuid::new_v5(&source_tool_call_id, artifact.artifact_sha256.as_bytes());
        repo.enumeration_commit_browser_producer_v2(CommitEnumerationBrowserProducerV2 {
            stable_request_id,
            operation_id,
            organization_id,
            stage_execution_id,
            stage_run_unit_id,
            target_id,
            exact_origin,
            worker_run_id: worker.worker_run_id,
            worker_attempt_epoch: worker.attempt_epoch,
            lease_token: worker.lease_token,
            source_tool_call_id,
            artifact,
        })
        .await
    } else {
        let artifact = match result
            .get(artifact_key)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("completed producer result omitted {artifact_key}"))
            .and_then(|value| {
                serde_json::from_value::<EnumerationJsApiProducerArtifactV2>(value)
                    .map_err(Into::into)
            }) {
            Ok(artifact) => artifact,
            Err(error) => {
                return stage_team_leader_router_error(
                    "ENUMERATION_V2_PRODUCER_ARTIFACT_MISSING",
                    error.to_string(),
                )
            }
        };
        let browser_receipt = match repo
            .enumeration_recover_lane_receipt_v2(RecoverEnumerationLaneReceiptV2 {
                operation_id,
                organization_id,
                stage_execution_id,
                stage_run_unit_id,
                target_id,
                exact_origin: exact_origin.clone(),
                lane: EnumerationLaneKindV2::Browser,
                resolution_occurrence_id: None,
                dependency_receipt_ids: vec![],
            })
            .await
        {
            Ok(Some(receipt)) => receipt,
            Ok(None) => return stage_team_leader_router_error(
                "ENUMERATION_V2_BROWSER_RECEIPT_REQUIRED",
                "JS/API analysis requires the exact Browser receipt for this Target + Web Origin",
            ),
            Err(error) => {
                return stage_team_leader_router_error(
                    "ENUMERATION_V2_BROWSER_RECEIPT_RECOVERY_FAILED",
                    error.to_string(),
                )
            }
        };
        let stable_request_id =
            uuid::Uuid::new_v5(&source_tool_call_id, artifact.artifact_sha256.as_bytes());
        repo.enumeration_commit_js_api_producer_v2(CommitEnumerationJsApiProducerV2 {
            stable_request_id,
            operation_id,
            organization_id,
            stage_execution_id,
            stage_run_unit_id,
            target_id,
            exact_origin,
            worker_run_id: worker.worker_run_id,
            worker_attempt_epoch: worker.attempt_epoch,
            lease_token: worker.lease_token,
            source_tool_call_id,
            artifact,
            browser_receipt,
        })
        .await
    };
    match receipt {
        Ok(receipt) => (enumeration_receipt_result(result, receipt), true),
        Err(error) => stage_team_leader_router_error(
            "ENUMERATION_V2_PRODUCER_COMMIT_FAILED",
            format!("{error:#}"),
        ),
    }
}

async fn recover_enumeration_receipt(
    repo: &dyn golish_agent_kit::db_traits::DbRepoProvider,
    bound: &BoundWorkerChainContext,
    target_id: uuid::Uuid,
    exact_origin: &str,
    lane: EnumerationLaneKindV2,
    resolution_occurrence_id: Option<uuid::Uuid>,
    dependency_receipt_ids: Vec<uuid::Uuid>,
) -> Result<EnumerationLaneClosureReceiptV2, String> {
    let dependency_receipt_ids =
        canonical_enumeration_dependency_receipt_ids(dependency_receipt_ids);
    repo.enumeration_recover_lane_receipt_v2(RecoverEnumerationLaneReceiptV2 {
        operation_id: bound.operation_id,
        organization_id: bound.organization_id,
        stage_execution_id: bound.stage_execution_id,
        stage_run_unit_id: bound.worker_lease.stage_run_unit_id,
        target_id,
        exact_origin: exact_origin.to_string(),
        lane,
        resolution_occurrence_id,
        dependency_receipt_ids,
    })
    .await
    .map_err(|error| error.to_string())?
    .ok_or_else(|| format!("missing exact {lane:?} receipt"))
}

/// Enforce typed lane dependencies before a registry producer can create any
/// capture, business row, evidence, or compatibility outcome. Settlement does
/// the same recovery again after execution to close the dependency→commit
/// race; this preflight exists to make an out-of-order AI call side-effect free.
async fn route_enumeration_producer_preflight(
    tool_name: &str,
    args: &Value,
    stage: Option<golish_agent_kit::harness::StageKind>,
    repo: Option<&std::sync::Arc<dyn golish_agent_kit::db_traits::DbRepoProvider>>,
    bound: Option<&BoundWorkerChainContext>,
    tool_context: Option<&golish_core::AgentToolContext>,
) -> Option<(Value, bool)> {
    if tool_name != "js_extract_apis"
        || stage != Some(golish_agent_kit::harness::StageKind::Enumeration)
    {
        return None;
    }
    let bound = bound.filter(|bound| bound.stage_team_leader.is_some())?;
    if !tool_context
        .is_some_and(|context| stage_team_leader_tool_context_matches(tool_name, bound, context))
    {
        return Some(stage_team_leader_router_error(
            "ENUMERATION_V2_PRODUCER_CONTEXT_MISMATCH",
            "JS/API producer prerequisite check is not bound to the live Enumeration Main AI ToolCall",
        ));
    }
    let Some(repo) = repo else {
        return Some(stage_team_leader_router_error(
            "ENUMERATION_V2_PRODUCER_REPOSITORY_REQUIRED",
            "JS/API producer prerequisite check requires the durable receipt repository",
        ));
    };
    let (target_id, exact_origin) = match enumeration_producer_args_target_and_origin(args) {
        Ok(subject) => subject,
        Err(error) => {
            return Some(stage_team_leader_router_error(
                "ENUMERATION_V2_PRODUCER_SUBJECT_INVALID",
                error,
            ))
        }
    };
    match recover_enumeration_receipt(
        repo.as_ref(),
        bound,
        target_id,
        &exact_origin,
        EnumerationLaneKindV2::Browser,
        None,
        vec![],
    )
    .await
    {
        Ok(_) => None,
        Err(error) => Some(stage_team_leader_router_error(
            "ENUMERATION_V2_BROWSER_RECEIPT_REQUIRED",
            error,
        )),
    }
}

fn canonical_enumeration_dependency_receipt_ids(
    mut dependency_receipt_ids: Vec<uuid::Uuid>,
) -> Vec<uuid::Uuid> {
    dependency_receipt_ids.sort_unstable();
    dependency_receipt_ids.dedup();
    dependency_receipt_ids
}

fn enumeration_reducer_subject(args: &Value) -> Result<(uuid::Uuid, String), String> {
    let object = args
        .as_object()
        .ok_or_else(|| "reducer arguments must be an object".to_string())?;
    if !object
        .keys()
        .all(|key| matches!(key.as_str(), "target_id" | "exact_origin"))
    {
        return Err("reducer accepts only target_id and exact_origin".to_string());
    }
    let target_id = object
        .get("target_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "reducer target_id is required".to_string())?
        .parse::<uuid::Uuid>()
        .map_err(|_| "reducer target_id must be a UUID".to_string())?;
    let exact_origin = object
        .get("exact_origin")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "reducer exact_origin is required".to_string())?
        .to_string();
    Ok((target_id, exact_origin))
}

async fn route_enumeration_receipt_reducer_tool(
    tool_name: &str,
    args: &Value,
    stage: Option<golish_agent_kit::harness::StageKind>,
    repo: Option<&std::sync::Arc<dyn golish_agent_kit::db_traits::DbRepoProvider>>,
    bound: Option<&BoundWorkerChainContext>,
    tool_context: Option<&golish_core::AgentToolContext>,
) -> Option<(Value, bool)> {
    if !matches!(
        tool_name,
        golish_sub_agents::ENUMERATION_REDUCE_PARAMETERS_TOOL_NAME
            | golish_sub_agents::ENUMERATION_REVIEW_COVERAGE_TOOL_NAME
    ) {
        return None;
    }
    if stage != Some(golish_agent_kit::harness::StageKind::Enumeration) {
        return Some(stage_team_leader_router_error(
            "ENUMERATION_V2_REDUCER_STAGE_REQUIRED",
            "Enumeration receipt reducers are unavailable outside Enumeration",
        ));
    }
    let Some(bound) = bound.filter(|bound| bound.stage_team_leader.is_some()) else {
        return Some(stage_team_leader_router_error(
            "ENUMERATION_V2_CONTROLLER_BINDING_REQUIRED",
            "Only the exact bound Enumeration Main AI may invoke receipt reducers",
        ));
    };
    if !tool_context
        .is_some_and(|context| stage_team_leader_tool_context_matches(tool_name, bound, context))
    {
        return Some(stage_team_leader_router_error(
            "ENUMERATION_V2_REDUCER_CONTEXT_MISMATCH",
            "Reducer ToolCall context does not match the bound Enumeration Main AI",
        ));
    }
    let Some(repo) = repo else {
        return Some(stage_team_leader_router_error(
            "ENUMERATION_V2_REDUCER_REPOSITORY_REQUIRED",
            "Enumeration receipt reducer requires the durable repository",
        ));
    };
    let (target_id, exact_origin) = match enumeration_reducer_subject(args) {
        Ok(subject) => subject,
        Err(error) => {
            return Some(stage_team_leader_router_error(
                "ENUMERATION_V2_REDUCER_SUBJECT_INVALID",
                error,
            ))
        }
    };
    let browser_receipt = match recover_enumeration_receipt(
        repo.as_ref(),
        bound,
        target_id,
        &exact_origin,
        EnumerationLaneKindV2::Browser,
        None,
        vec![],
    )
    .await
    {
        Ok(receipt) => receipt,
        Err(error) => {
            return Some(stage_team_leader_router_error(
                "ENUMERATION_V2_BROWSER_RECEIPT_REQUIRED",
                error,
            ))
        }
    };
    let js_api_receipt = match recover_enumeration_receipt(
        repo.as_ref(),
        bound,
        target_id,
        &exact_origin,
        EnumerationLaneKindV2::JsApi,
        None,
        vec![browser_receipt.receipt_id],
    )
    .await
    {
        Ok(receipt) => receipt,
        Err(error) => {
            return Some(stage_team_leader_router_error(
                "ENUMERATION_V2_JS_API_RECEIPT_REQUIRED",
                error,
            ))
        }
    };
    let context = tool_context.expect("validated reducer tool context");
    let source_tool_call_id = context
        .tool_call_record_id
        .expect("validated reducer tool call id");
    let evidence_ids = browser_receipt
        .evidence_audit_ids
        .iter()
        .chain(js_api_receipt.evidence_audit_ids.iter())
        .copied()
        .filter(|id| *id > 0)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    if tool_name == golish_sub_agents::ENUMERATION_REDUCE_PARAMETERS_TOOL_NAME {
        let artifact_sha256 = sha256_json(&json!({
            "browser_closure_graph_sha256": browser_receipt.closure_graph_sha256,
            "browser_receipt_id": browser_receipt.receipt_id,
            "browser_receipt_hash": browser_receipt.receipt_set_sha256,
            "evidence_audit_ids": evidence_ids,
            "exact_origin": exact_origin,
            "js_api_closure_graph_sha256": js_api_receipt.closure_graph_sha256,
            "js_api_receipt_id": js_api_receipt.receipt_id,
            "js_api_receipt_hash": js_api_receipt.receipt_set_sha256,
            "target_id": target_id,
        }));
        return Some(
            match repo
                .enumeration_reduce_parameter_v2(ReduceEnumerationParameterV2 {
                    stable_request_id: uuid::Uuid::new_v5(
                        &source_tool_call_id,
                        artifact_sha256.as_bytes(),
                    ),
                    operation_id: bound.operation_id,
                    organization_id: bound.organization_id,
                    stage_execution_id: bound.stage_execution_id,
                    stage_run_unit_id: bound.worker_lease.stage_run_unit_id,
                    target_id,
                    exact_origin,
                    worker_run_id: bound.worker_lease.worker_run_id,
                    worker_attempt_epoch: bound.worker_lease.attempt_epoch,
                    lease_token: bound.worker_lease.lease_token,
                    source_tool_call_id,
                    evidence_audit_ids: evidence_ids,
                    browser_receipt,
                    js_api_receipt,
                })
                .await
            {
                Ok(receipt) => (
                    json!({"lane_closure_receipt_v2": receipt, "typed_receipt_committed": true}),
                    true,
                ),
                Err(error) => stage_team_leader_router_error(
                    "ENUMERATION_V2_PARAMETER_COMMIT_FAILED",
                    error.to_string(),
                ),
            },
        );
    }

    let parameter_receipt = match recover_enumeration_receipt(
        repo.as_ref(),
        bound,
        target_id,
        &exact_origin,
        EnumerationLaneKindV2::Parameter,
        None,
        vec![browser_receipt.receipt_id, js_api_receipt.receipt_id],
    )
    .await
    {
        Ok(receipt) => receipt,
        Err(error) => {
            return Some(stage_team_leader_router_error(
                "ENUMERATION_V2_PARAMETER_RECEIPT_REQUIRED",
                error,
            ))
        }
    };
    let occurrences = match repo
        .enumeration_unresolved_occurrences(
            bound.operation_id,
            bound.organization_id,
            bound.stage_execution_id,
            bound.worker_lease.stage_run_unit_id,
        )
        .await
    {
        Ok(occurrences) => occurrences
            .into_iter()
            .filter(|occurrence| {
                occurrence.source_target_id == target_id && occurrence.exact_origin == exact_origin
            })
            .collect::<Vec<_>>(),
        Err(error) => {
            return Some(stage_team_leader_router_error(
                "ENUMERATION_V2_UNRESOLVED_READ_FAILED",
                error.to_string(),
            ))
        }
    };
    let mut resolution_receipts = Vec::with_capacity(occurrences.len());
    let mut missing_occurrence_ids = Vec::new();
    for occurrence in occurrences {
        match recover_enumeration_receipt(
            repo.as_ref(),
            bound,
            target_id,
            &exact_origin,
            EnumerationLaneKindV2::Resolution,
            Some(occurrence.occurrence_id),
            vec![occurrence.producer_receipt.receipt_id],
        )
        .await
        {
            Ok(receipt) => resolution_receipts.push(receipt),
            Err(_) => missing_occurrence_ids.push(occurrence.occurrence_id),
        }
    }
    if !missing_occurrence_ids.is_empty() {
        return Some((
            json!({
                "code": "ENUMERATION_RESOLUTION_RECEIPTS_REQUIRED",
                "error": "Coverage remains open until Main AI dispatches one bounded Resolution Analyst for each unresolved occurrence",
                "unresolved_occurrence_ids": missing_occurrence_ids,
                "dispatch_role": "resolution_analyst",
                "dispatch_kind": "enumeration_resolution",
                "target_id": target_id,
                "exact_origin": exact_origin,
            }),
            false,
        ));
    }
    resolution_receipts.sort_by_key(|receipt| receipt.receipt_id);
    let evidence_ids = evidence_ids
        .into_iter()
        .chain(parameter_receipt.evidence_audit_ids.iter().copied())
        .chain(
            resolution_receipts
                .iter()
                .flat_map(|receipt| receipt.evidence_audit_ids.iter().copied()),
        )
        .filter(|id| *id > 0)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let resolution_hashes = resolution_receipts
        .iter()
        .map(|receipt| {
            json!({
                "closure_graph_sha256": receipt.closure_graph_sha256,
                "occurrence_id": receipt.resolution_occurrence_id,
                "receipt_id": receipt.receipt_id,
                "receipt_set_sha256": receipt.receipt_set_sha256,
            })
        })
        .collect::<Vec<_>>();
    let artifact_sha256 = sha256_json(&json!({
        "browser_closure_graph_sha256": browser_receipt.closure_graph_sha256,
        "browser_receipt_hash": browser_receipt.receipt_set_sha256,
        "browser_receipt_id": browser_receipt.receipt_id,
        "evidence_audit_ids": evidence_ids,
        "exact_origin": exact_origin,
        "js_api_closure_graph_sha256": js_api_receipt.closure_graph_sha256,
        "js_api_receipt_hash": js_api_receipt.receipt_set_sha256,
        "js_api_receipt_id": js_api_receipt.receipt_id,
        "parameter_closure_graph_sha256": parameter_receipt.closure_graph_sha256,
        "parameter_receipt_hash": parameter_receipt.receipt_set_sha256,
        "parameter_receipt_id": parameter_receipt.receipt_id,
        "resolution_receipts": resolution_hashes,
        "target_id": target_id,
    }));
    Some(
        match repo
            .enumeration_review_coverage_v2(ReviewEnumerationCoverageV2 {
                stable_request_id: uuid::Uuid::new_v5(
                    &source_tool_call_id,
                    artifact_sha256.as_bytes(),
                ),
                operation_id: bound.operation_id,
                organization_id: bound.organization_id,
                stage_execution_id: bound.stage_execution_id,
                stage_run_unit_id: bound.worker_lease.stage_run_unit_id,
                target_id,
                exact_origin,
                worker_run_id: bound.worker_lease.worker_run_id,
                worker_attempt_epoch: bound.worker_lease.attempt_epoch,
                lease_token: bound.worker_lease.lease_token,
                source_tool_call_id,
                evidence_audit_ids: evidence_ids,
                browser_receipt,
                js_api_receipt,
                parameter_receipt,
                resolution_receipts,
            })
            .await
        {
            Ok(receipt) => (
                json!({"lane_closure_receipt_v2": receipt, "typed_receipt_committed": true}),
                true,
            ),
            Err(error) => stage_team_leader_router_error(
                "ENUMERATION_V2_COVERAGE_COMMIT_FAILED",
                error.to_string(),
            ),
        },
    )
}

async fn route_target_intel_reviewer_host_tool(
    tool_name: &str,
    args: &Value,
    runtime_memory: Option<&std::sync::Arc<dyn RuntimeMemoryRepository>>,
    bound: Option<&BoundWorkerChainContext>,
    tool_context: Option<&golish_core::AgentToolContext>,
) -> Option<(Value, bool)> {
    if !matches!(
        tool_name,
        golish_sub_agents::TARGET_INTEL_READ_REVIEW_SECTION
            | golish_sub_agents::TARGET_INTEL_RECORD_REVIEW_VERDICT
    ) {
        return None;
    }
    let Some(bound) = bound else {
        return Some(stage_team_leader_router_error(
            "TARGET_INTEL_REVIEWER_BINDING_REQUIRED",
            "Target Intel review tools require a trusted bound reviewer Worker",
        ));
    };
    let Some(review) = bound.target_intel_review.as_ref() else {
        return Some(stage_team_leader_router_error(
            "TARGET_INTEL_REVIEWER_BINDING_REQUIRED",
            "ordinary StageTeam workers cannot invoke Target Intel review tools",
        ));
    };
    let Some(repository) = runtime_memory else {
        return Some(stage_team_leader_router_error(
            "TARGET_INTEL_REVIEW_RUNTIME_MEMORY_REQUIRED",
            "Target Intel review tools require durable runtime memory",
        ));
    };
    if !tool_context
        .is_some_and(|context| stage_team_leader_tool_context_matches(tool_name, bound, context))
    {
        return Some(stage_team_leader_router_error(
            "TARGET_INTEL_REVIEW_TOOL_CONTEXT_MISMATCH",
            "Target Intel review tool context does not match the bound Worker",
        ));
    }
    if tool_name == golish_sub_agents::TARGET_INTEL_READ_REVIEW_SECTION {
        let requested_kind = match args.get("requested_kind").and_then(Value::as_str) {
            Some("durable_state") => IntelReviewSectionKind::DurableState,
            Some("observable_actions") => IntelReviewSectionKind::ObservableActions,
            Some("frozen_contract") => IntelReviewSectionKind::FrozenContract,
            Some("completion_claim") => IntelReviewSectionKind::CompletionClaim,
            _ => {
                return Some(stage_team_leader_router_error(
                    "TARGET_INTEL_REVIEW_SECTION_KIND_INVALID",
                    "requested_kind must name one of the four ordered review sections",
                ));
            }
        };
        return Some(
            match repository
                .read_target_intel_review_section(ReadTargetIntelReviewSection {
                    review_id: review.review_id,
                    reviewer_worker_run_id: bound.worker_lease.worker_run_id,
                    requested_kind,
                    expected_bundle_sha256: review.bundle_sha256.clone(),
                    expected_worker_attempt_epoch: bound.worker_lease.attempt_epoch,
                })
                .await
            {
                Ok(section) => (
                    json!({
                        "review_id": section.review_id,
                        "review_row_version": section.review_row_version,
                        "section_kind": section.section_kind.as_str(),
                        "section_sha256": section.section_sha256,
                        "payload": section.payload,
                        "next_section": section.next_section.map(IntelReviewSectionKind::as_str),
                        "replayed": section.replayed,
                    }),
                    true,
                ),
                Err(error) => (json!({"error": error.to_string()}), false),
            },
        );
    }
    let Some(expected_review_row_version) = args
        .get("expected_review_row_version")
        .and_then(Value::as_i64)
    else {
        return Some(stage_team_leader_router_error(
            "TARGET_INTEL_REVIEW_VERDICT_VERSION_REQUIRED",
            "terminal verdict requires the row version returned by the final ordered read",
        ));
    };
    let verdict = match args
        .get("verdict")
        .cloned()
        .and_then(|value| serde_json::from_value::<IntelReviewVerdict>(value).ok())
    {
        Some(verdict) => verdict,
        None => {
            return Some(stage_team_leader_router_error(
                "TARGET_INTEL_REVIEW_VERDICT_SCHEMA_INVALID",
                "verdict must match the closed intel_review.v1 schema",
            ));
        }
    };
    Some(
        match repository
            .record_target_intel_review_verdict(RecordTargetIntelReviewVerdict {
                review_id: review.review_id,
                reviewer_worker_run_id: bound.worker_lease.worker_run_id,
                expected_worker_attempt_epoch: bound.worker_lease.attempt_epoch,
                expected_review_row_version,
                expected_bundle_sha256: review.bundle_sha256.clone(),
                verdict,
            })
            .await
        {
            Ok(recorded) => (
                json!({
                    "review_id": recorded.review_id,
                    "review_row_version": recorded.review_row_version,
                    "decision": match recorded.decision {
                        golish_agent_kit::harness::IntelReviewDecision::Pass => "PASS",
                        golish_agent_kit::harness::IntelReviewDecision::Rework => "REWORK",
                        golish_agent_kit::harness::IntelReviewDecision::NeedsHuman => "NEEDS_HUMAN",
                    },
                    "verdict_sha256": recorded.verdict_sha256,
                    "hold_id": recorded.hold_id,
                    "replayed": recorded.replayed,
                    "terminal": true,
                }),
                true,
            ),
            Err(error) => (json!({"error": error.to_string()}), false),
        },
    )
}

/// Route Company Controller host tools before generic security/graph fallbacks.
/// Exact reserved names on a bound stage worker are always consumed: an
/// ordinary Worker can never turn a missing trusted binding into a registry or
/// MCP fallback. Unbound orchestrators retain their existing update_plan route.
async fn route_stage_team_leader_host_tool(
    tool_name: &str,
    args: &Value,
    runtime_memory: Option<&std::sync::Arc<dyn RuntimeMemoryRepository>>,
    bound: Option<&BoundWorkerChainContext>,
    tool_context: Option<&golish_core::AgentToolContext>,
) -> Option<(Value, bool)> {
    if tool_name == STAGE_TEAM_UPDATE_PLAN_TOOL_NAME && bound.is_none() {
        // Preserve the existing generic update_plan route for unbound
        // orchestrator agents. Only bound stage workers enter this reserved
        // Company Controller router.
        return None;
    }
    if !matches!(
        tool_name,
        STAGE_TEAM_DISPATCH_WORKERS_TOOL_NAME
            | STAGE_TEAM_PREPARE_FINAL_SUBMISSION_TOOL_NAME
            | STAGE_TEAM_UPDATE_PLAN_TOOL_NAME
    ) {
        return None;
    }

    let Some(bound) = bound else {
        return Some(stage_team_leader_router_error(
            "STAGE_TEAM_LEADER_BINDING_REQUIRED",
            "Stage Team controller tools require a trusted bound Worker",
        ));
    };
    let Some(leader) = bound.stage_team_leader.as_ref() else {
        return Some(stage_team_leader_router_error(
            "STAGE_TEAM_LEADER_BINDING_REQUIRED",
            "ordinary stage Workers cannot use Company Controller tools",
        ));
    };
    let Some(tool_context) = tool_context else {
        return Some(stage_team_leader_router_error(
            "STAGE_TEAM_LEADER_TOOL_CONTEXT_MISSING",
            "Company Controller tool request has no trusted host context",
        ));
    };
    if !stage_team_leader_tool_context_matches(tool_name, bound, tool_context) {
        return Some(stage_team_leader_router_error(
            "STAGE_TEAM_LEADER_TOOL_CONTEXT_MISMATCH",
            "Company Controller tool context does not match the bound Worker fence",
        ));
    }

    let _mutation_guard = bound.mutation_lock.lock().await;
    if bound.lease_is_lost() {
        return Some(stage_team_leader_router_error(
            "STAGE_TEAM_LEADER_LEASE_LOST",
            "Company Controller lease was lost before host-tool routing",
        ));
    }

    if tool_name == STAGE_TEAM_UPDATE_PLAN_TOOL_NAME {
        let Some(plan_items) = args.get("plan").and_then(Value::as_array) else {
            return Some(stage_team_leader_router_error(
                "STAGE_TEAM_UPDATE_PLAN_ARGS_INVALID",
                "Company Controller update_plan requires a plan array",
            ));
        };
        if plan_items
            .iter()
            .any(|item| item.get("status").and_then(Value::as_str).is_none())
        {
            return Some(stage_team_leader_router_error(
                "STAGE_TEAM_UPDATE_PLAN_STATUS_INVALID",
                "every Company Controller plan item requires status pending, in_progress, or completed",
            ));
        }
        let update_args = match serde_json::from_value::<UpdatePlanArgs>(args.clone()) {
            Ok(update_args) => update_args,
            Err(error) => {
                return Some(stage_team_leader_router_error(
                    "STAGE_TEAM_UPDATE_PLAN_ARGS_INVALID",
                    format!("invalid Company Controller update_plan arguments: {error}"),
                ));
            }
        };
        if update_args.plan.iter().any(|item| {
            !matches!(
                item.status,
                StepStatus::Pending | StepStatus::InProgress | StepStatus::Completed
            )
        }) {
            return Some(stage_team_leader_router_error(
                "STAGE_TEAM_UPDATE_PLAN_STATUS_INVALID",
                "every Company Controller plan item requires status pending, in_progress, or completed",
            ));
        }
        // This intentionally has no DB repository or event emitter. PlanManager
        // supplies the canonical 1..12/description/in_progress validation and
        // normalization only; the bound chain checkpoints this tool call/result.
        let normalized = match PlanManager::new().update_plan(update_args, None).await {
            Ok(plan) => plan,
            Err(error) => {
                return Some(stage_team_leader_router_error(
                    "STAGE_TEAM_UPDATE_PLAN_INVALID",
                    format!("Company Controller plan was rejected: {error}"),
                ));
            }
        };
        return Some((
            json!({
                "explanation": normalized.explanation,
                "plan": normalized.steps,
                "plan_version": bound.current_checkpoint_version().saturating_add(1),
                "plan_version_scope": "bound_chain_checkpoint_hint",
                "success": true,
                "summary": {
                    "completed": normalized.summary.completed,
                    "in_progress": normalized.summary.in_progress,
                    "pending": normalized.summary.pending,
                    "total": normalized.summary.total,
                },
            }),
            true,
        ));
    }

    if tool_name == STAGE_TEAM_PREPARE_FINAL_SUBMISSION_TOOL_NAME {
        if !args.as_object().is_some_and(serde_json::Map::is_empty) {
            return Some(stage_team_leader_router_error(
                "STAGE_TEAM_PREPARE_FINAL_ARGS_INVALID",
                "prepare-final accepts an empty object only",
            ));
        }
        return Some((
            json!({
                "request_epoch_closed": true,
                "status": STAGE_TEAM_PREPARE_FINAL_STATUS,
            }),
            true,
        ));
    }

    let mut compiled_budget_hints = std::collections::HashMap::<String, Value>::new();
    let mut parsed = if leader.controller_action_compiler.as_deref() == Some("enumeration_v2") {
        let selected = match serde_json::from_value::<EnumerationDispatchWorkersArgs>(args.clone())
        {
            Ok(selected)
                if !selected.workers.is_empty()
                    && selected.workers.len() <= MAX_STAGE_TEAM_CONTROLLER_DISPATCH_BATCH =>
            {
                selected
            }
            Ok(_) => {
                return Some(stage_team_leader_router_error(
                    "STAGE_TEAM_DISPATCH_BATCH_INVALID",
                    "dispatch requires between 1 and 32 bounded Enumeration actions",
                ));
            }
            Err(error) => {
                return Some(stage_team_leader_router_error(
                    "STAGE_TEAM_DISPATCH_ARGS_INVALID",
                    format!("invalid Enumeration action selection: {error}"),
                ));
            }
        };
        let mut selected_ids = HashSet::with_capacity(selected.workers.len());
        let mut compiled = Vec::with_capacity(selected.workers.len());
        for selection in selected.workers {
            let action_id = selection.action_id.trim();
            if action_id.is_empty()
                || selection.rationale.trim().is_empty()
                || !selected_ids.insert(action_id.to_string())
            {
                return Some(stage_team_leader_router_error(
                    "ENUMERATION_CONTROLLER_ACTION_INVALID",
                    "each Enumeration action needs a unique current action_id and non-empty planning rationale",
                ));
            }
            let matches = leader
                .compiled_actions
                .iter()
                .filter(|action| action.action_id == action_id)
                .collect::<Vec<_>>();
            let [action] = matches.as_slice() else {
                return Some(stage_team_leader_router_error(
                    "ENUMERATION_CONTROLLER_ACTION_NOT_READY",
                    format!("Enumeration action '{action_id}' is absent or ambiguous in the current host-compiled catalogue"),
                ));
            };
            compiled_budget_hints.insert(action.dedupe_key.clone(), action.budget_hint.clone());
            compiled.push(StageTeamDispatchWorkerArgs {
                dedupe_key: action.dedupe_key.clone(),
                role: action.requested_role.clone(),
                kind: action.requested_kind.clone(),
                objective: action.objective.clone(),
                subject_refs: action.subject_refs.clone(),
            });
        }
        StageTeamDispatchWorkersArgs { workers: compiled }
    } else {
        match serde_json::from_value::<StageTeamDispatchWorkersArgs>(args.clone()) {
            Ok(parsed)
                if !parsed.workers.is_empty()
                    && parsed.workers.len() <= MAX_STAGE_TEAM_CONTROLLER_DISPATCH_BATCH =>
            {
                parsed
            }
            Ok(_) => {
                return Some(stage_team_leader_router_error(
                    "STAGE_TEAM_DISPATCH_BATCH_INVALID",
                    "dispatch requires between 1 and 32 bounded worker requests",
                ));
            }
            Err(error) => {
                return Some(stage_team_leader_router_error(
                    "STAGE_TEAM_DISPATCH_ARGS_INVALID",
                    format!("invalid Stage Team dispatch arguments: {error}"),
                ));
            }
        }
    };
    let Some(runtime_memory) = runtime_memory else {
        return Some(stage_team_leader_router_error(
            "STAGE_TEAM_RUNTIME_MEMORY_REQUIRED",
            "Company Controller dispatch requires durable runtime memory",
        ));
    };

    let mut dedupe_keys = HashSet::with_capacity(parsed.workers.len());
    for worker in &parsed.workers {
        if worker.dedupe_key.trim().is_empty()
            || worker.role.trim().is_empty()
            || worker.kind.trim().is_empty()
            || worker.objective.trim().is_empty()
            || worker.subject_refs.iter().any(|value| !value.is_object())
            || !dedupe_keys.insert(worker.dedupe_key.trim())
        {
            return Some(stage_team_leader_router_error(
                "STAGE_TEAM_DISPATCH_WORKER_INVALID",
                "each worker needs unique non-empty identity fields, an objective, and object subject refs",
            ));
        }
    }
    for worker in &mut parsed.workers {
        worker.subject_refs = match canonicalize_stage_team_subject_refs(&worker.subject_refs) {
            Ok(subject_refs) => subject_refs,
            Err(error) => {
                return Some(stage_team_leader_router_error(
                    "STAGE_TEAM_DISPATCH_WORKER_INVALID",
                    format!(
                        "worker '{}' has invalid subject_refs: {error}",
                        worker.dedupe_key.trim()
                    ),
                ));
            }
        };
    }
    let mut assignment_identities = HashSet::with_capacity(parsed.workers.len());
    for worker in &parsed.workers {
        let assignment_identity = match stage_team_dispatch_assignment_identity(worker) {
            Ok(identity) => identity,
            Err(error) => {
                return Some(stage_team_leader_router_error(
                    "STAGE_TEAM_DISPATCH_WORKER_INVALID",
                    error,
                ));
            }
        };
        if !assignment_identities.insert(assignment_identity) {
            return Some(stage_team_leader_router_error(
                "STAGE_TEAM_DISPATCH_ASSIGNMENT_OVERLAP",
                format!(
                    "worker '{}' duplicates another normalized role/kind/objective/subject assignment in this batch; split disjoint subjects or submit one whole-company worker",
                    worker.dedupe_key.trim()
                ),
            ));
        }
    }

    let fence = RuntimeWorkerFence {
        operation_id: bound.operation_id,
        stage_execution_id: bound.stage_execution_id,
        stage_run_unit_id: bound.worker_lease.stage_run_unit_id,
        worker_run_id: bound.worker_lease.worker_run_id,
        lease_token: bound.worker_lease.lease_token,
        attempt_epoch: bound.worker_lease.attempt_epoch,
        expected_checkpoint_version: bound.current_checkpoint_version(),
    };
    let request_count = parsed.workers.len();
    let mut decisions = Vec::with_capacity(parsed.workers.len());
    let mut accepted_count = 0usize;
    for worker in parsed.workers {
        let dedupe_key = worker.dedupe_key.trim().to_string();
        let requested_role = worker.role.trim().to_string();
        let requested_kind = worker.kind.trim().to_string();
        let objective = worker.objective.trim().to_string();
        let reason = match serde_json::to_string(&StageTeamControllerRequestEnvelope {
            schema: "stage_team_controller_request.v1",
            parent_tool_request_id: tool_context.request_id.as_str(),
            objective: objective.as_str(),
        }) {
            Ok(reason) => reason,
            Err(error) => {
                return Some(stage_team_leader_router_error(
                    "STAGE_TEAM_CONTROLLER_REASON_INVALID",
                    format!("controller request envelope was not serializable: {error}"),
                ));
            }
        };
        let output_schema = json!("stage_worker_output.v1");
        let budget_hint = compiled_budget_hints
            .remove(&dedupe_key)
            .unwrap_or_else(|| json!({}));
        let request_material = json!({
            "budget_hint": &budget_hint,
            "dedupe_key": &dedupe_key,
            "dispatch_epoch": leader.expected_dispatch_epoch,
            "operation_id": fence.operation_id,
            "output_schema": &output_schema,
            "parent_work_item_id": leader.leader_work_item_id,
            "reason": &reason,
            "requested_kind": &requested_kind,
            "requested_role": &requested_role,
            "stage_execution_id": fence.stage_execution_id,
            "stage_run_unit_id": fence.stage_run_unit_id,
            "stage_team_plan_id": leader.stage_team_plan_id,
            "subject_refs": &worker.subject_refs,
        });
        let persisted = match runtime_memory
            .request_stage_worker(RequestStageWorker {
                fence: fence.clone(),
                stage_team_plan_id: leader.stage_team_plan_id,
                parent_work_item_id: leader.leader_work_item_id,
                expected_dispatch_epoch: leader.expected_dispatch_epoch,
                requested_role,
                requested_kind,
                subject_refs: worker.subject_refs,
                reason,
                output_schema,
                budget_hint,
                dedupe_key,
                request_sha256: sha256_json(&request_material),
            })
            .await
        {
            Ok(persisted) => persisted,
            Err(error) => {
                if accepted_count > 0 {
                    return Some((
                        json!({
                            "accepted_count": accepted_count,
                            "partial_persist_error": error.to_string(),
                            "rejected_count": decisions.len() - accepted_count,
                            "request_count": decisions.len(),
                            "requests": decisions,
                            "status": STAGE_TEAM_DISPATCH_ACCEPTED_STATUS,
                            "tool_request_id": tool_context.request_id,
                        }),
                        true,
                    ));
                }
                let (mut result, success) = stage_team_leader_router_error(
                    "STAGE_TEAM_DISPATCH_PERSIST_FAILED",
                    format!("durable Stage Team worker request failed: {error}"),
                );
                if let Some(result) = result.as_object_mut() {
                    result.insert("accepted_count".to_string(), json!(0));
                    result.insert("request_count".to_string(), json!(request_count));
                    result.insert("requests".to_string(), json!(decisions));
                    result.insert("status".to_string(), json!("dispatch_failed"));
                    result.insert(
                        "tool_request_id".to_string(),
                        json!(tool_context.request_id),
                    );
                }
                return Some((result, success));
            }
        };
        if persisted.request.decision == StageWorkerRequestDecision::Accepted {
            accepted_count += 1;
        }
        decisions.push(json!({
            "created_work_item_id": persisted.request.created_work_item_id,
            "decision": persisted.request.decision.as_str(),
            "decision_code": persisted.request.decision_code,
            "dedupe_key": persisted.request.dedupe_key,
            "replayed": persisted.replayed,
            "request_id": persisted.request.id,
        }));
    }

    if accepted_count == 0 {
        return Some((
            json!({
                "accepted_count": 0,
                "code": "STAGE_TEAM_DISPATCH_NONE_ACCEPTED",
                "error": "no requested Stage Team worker was accepted; revise the dispatch in this Controller turn",
                "next_action": "Retry with canonical {\"kind\":\"target\",\"target_id\":\"<uuid>\"} subject refs, or omit subject_refs only for an intentional whole-company assignment.",
                "rejected_count": decisions.len(),
                "request_count": decisions.len(),
                "requests": decisions,
                "retryable": true,
                "status": "dispatch_rejected",
                "tool_request_id": tool_context.request_id,
            }),
            false,
        ));
    }

    Some((
        json!({
            "accepted_count": accepted_count,
            "rejected_count": decisions.len() - accepted_count,
            "request_count": decisions.len(),
            "requests": decisions,
            "status": STAGE_TEAM_DISPATCH_ACCEPTED_STATUS,
            "tool_request_id": tool_context.request_id,
        }),
        true,
    ))
}

fn sub_agent_runtime_agent_path(agent_id: &str) -> String {
    format!("main>{agent_id}")
}

fn vuln_triage_hides_record_finding(
    stage: Option<golish_agent_kit::harness::StageKind>,
    tool_name: &str,
) -> bool {
    stage == Some(golish_agent_kit::harness::StageKind::VulnTriage) && tool_name == "record_finding"
}

fn unified_investigation_cognitive_tool_allowed(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "update_plan"
            | "query_target_data"
            | "list_in_scope_targets"
            | "list_recent_evidence"
            | "search_memories"
            | "search_knowledge_base"
            | "read_knowledge"
            | "graph_search"
            | "graph_neighbors"
            | "graph_attack_paths"
            | "harness_trace"
            | "submit_result"
            | "submit_stage_deliverable"
    ) || tool_name.starts_with("sub_agent_")
}

fn unified_investigation_reducer_request(
    stage: Option<golish_agent_kit::harness::StageKind>,
    tool_id: &str,
) -> bool {
    stage == Some(golish_agent_kit::harness::StageKind::Investigation)
        && tool_id.contains("::synthesis-attempt:")
}

fn production_target_intel_sub_agent_tool_allowed(
    tool_name: &str,
    is_company_controller: bool,
    is_final_submitter: bool,
    is_reviewer: bool,
) -> bool {
    if is_reviewer {
        return matches!(
            tool_name,
            golish_sub_agents::TARGET_INTEL_READ_REVIEW_SECTION
                | golish_sub_agents::TARGET_INTEL_RECORD_REVIEW_VERDICT
                | "submit_result"
        );
    }
    if is_final_submitter {
        return tool_name == "submit_stage_deliverable";
    }
    if is_company_controller {
        return matches!(
            tool_name,
            "update_plan"
                | "recon_search_intel"
                | STAGE_TEAM_DISPATCH_WORKERS_TOOL_NAME
                | STAGE_TEAM_PREPARE_FINAL_SUBMISSION_TOOL_NAME
        );
    }
    matches!(
        tool_name,
        "recon_search_intel" | "list_recent_evidence" | "submit_result"
    )
}

fn sub_agent_stage_tool_hidden(
    tool_name: &str,
    hide_scan_tools: bool,
    deny_vuln_finding: bool,
    investigation_cognition_only: bool,
) -> bool {
    (hide_scan_tools && golish_agent_kit::harness::is_scan_tool_name(tool_name))
        || (deny_vuln_finding && tool_name == "record_finding")
        || (investigation_cognition_only
            && !unified_investigation_cognitive_tool_allowed(tool_name))
}

fn deny_vuln_finding_writes(ctx: &AgenticLoopContext<'_>) -> bool {
    vuln_triage_hides_record_finding(ctx.harness_stage, "record_finding")
}

fn sub_agent_execution_error_result(error: anyhow::Error) -> ToolExecutionResult {
    let chain_failure_contract =
        error
            .downcast_ref::<SubAgentChainError>()
            .map(|error| match error {
                SubAgentChainError::ExactResumeUnavailable { .. } => (
                    "sub_agent_chain_exact_resume_unavailable",
                    "restore_exact",
                    None,
                ),
                SubAgentChainError::LatestResumeUnavailable { .. } => (
                    "sub_agent_chain_latest_resume_unavailable",
                    "restore_latest",
                    None,
                ),
                SubAgentChainError::CreateFreshFailed { .. } => {
                    ("sub_agent_chain_create_fresh_failed", "create_fresh", None)
                }
                SubAgentChainError::FinalizeFailed {
                    checkpointed_chain_id,
                    ..
                } => (
                    "sub_agent_chain_finalize_failed",
                    "finalize",
                    *checkpointed_chain_id,
                ),
                SubAgentChainError::ProviderContextLimitExceeded { chain_id, .. } => (
                    "sub_agent_provider_context_limit_exceeded",
                    "context_limit",
                    *chain_id,
                ),
                SubAgentChainError::BoundWorkerUnavailable { .. } => {
                    ("sub_agent_bound_worker_unavailable", "bound_worker", None)
                }
            });
    let error = error.to_string();
    let value = match chain_failure_contract {
        Some((error_code, chain_failure_kind, chain_id)) => {
            let mut value = json!({
                "error": error,
                "error_code": error_code,
                "chain_failure_kind": chain_failure_kind,
            });
            if let Some(chain_id) = chain_id {
                value["chain_id"] = json!(chain_id.to_string());
            }
            value
        }
        None => json!({ "error": error }),
    };
    ToolExecutionResult {
        value,
        success: false,
    }
}

fn dispatch_status_for_sub_agent_success(
    success: bool,
) -> golish_agent_kit::db_traits::DispatchStatus {
    if success {
        golish_agent_kit::db_traits::DispatchStatus::Completed
    } else {
        golish_agent_kit::db_traits::DispatchStatus::Failed
    }
}

fn sub_agent_tool_execution_result(
    result: golish_sub_agents::SubAgentResult,
) -> ToolExecutionResult {
    let success = result.success;
    let mut value = json!({
        "agent_id": result.agent_id,
        "response": result.response,
        "success": result.success,
        "duration_ms": result.duration_ms,
        "files_modified": result.files_modified,
    });
    if let Some(chain_id) = result.chain_id {
        value["chain_id"] = json!(chain_id.to_string());
    }
    ToolExecutionResult { value, success }
}

fn sub_agent_checkpoint_agent_path(
    stage: Option<golish_agent_kit::harness::StageKind>,
    parent_request_id: &str,
    agent_id: &str,
) -> String {
    match (stage, stage_run_org_id_from_request_id(parent_request_id)) {
        (Some(stage), Some(org_id)) => {
            format!(
                "main>stage_run:{}>org:{}>{}",
                stage.as_str(),
                org_id,
                agent_id
            )
        }
        _ => sub_agent_runtime_agent_path(agent_id),
    }
}

fn evidence_ids_from_submit_result(result: &Value) -> Vec<i64> {
    result
        .get("available_evidence_ids")
        .and_then(|ids| ids.as_array())
        .into_iter()
        .flatten()
        .filter_map(|id| {
            id.as_i64()
                .or_else(|| id.as_u64().and_then(|u| i64::try_from(u).ok()))
        })
        .collect()
}

fn background_job_ids_from_submit_result(result: &Value) -> Vec<String> {
    result
        .get("running_background_jobs")
        .and_then(|jobs| jobs.as_array())
        .into_iter()
        .flatten()
        .filter_map(|job| {
            job.get("job_id")
                .and_then(|id| id.as_str())
                .map(str::to_string)
        })
        .collect()
}

fn strings_from_json_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(|items| items.as_array())
        .into_iter()
        .flatten()
        .filter_map(|item| item.as_str().map(str::to_string))
        .collect()
}

fn coverage_gap_actions_from_submit_result(
    result: &Value,
) -> Vec<golish_agent_kit::harness::CoverageGapAction> {
    result
        .get("coverage_gap_actions")
        .and_then(|items| items.as_array())
        .into_iter()
        .flatten()
        .filter_map(|item| {
            serde_json::from_value::<golish_agent_kit::harness::CoverageGapAction>(item.clone())
                .ok()
        })
        .collect()
}

fn repair_directive_from_submit_result(
    stage: Option<golish_agent_kit::harness::StageKind>,
    org_id: Option<uuid::Uuid>,
    agent_path: String,
    result: &Value,
) -> Option<RepairDirective> {
    if result.get("status").and_then(|s| s.as_str()) != Some("needs_fix") {
        return None;
    }
    let stage = stage?;
    Some(refine_submit_needs_fix(RefinerContext {
        stage,
        org_id,
        agent_path,
        reasons: strings_from_json_array(result.get("reasons")),
        coverage_gap_actions: coverage_gap_actions_from_submit_result(result),
        available_evidence_ids: evidence_ids_from_submit_result(result),
        running_background_jobs: background_job_ids_from_submit_result(result),
    }))
}

fn repair_kind_label(directive: &RepairDirective) -> String {
    serde_json::to_string(&directive.repair_kind)
        .unwrap_or_else(|_| "\"generic\"".to_string())
        .trim_matches('"')
        .to_string()
}

fn submit_repair_mode_from_agent_run(checkpoint: &AgentRunCheckpoint) -> Option<SubmitRepairMode> {
    serde_json::from_value(checkpoint.submit_repair_mode.clone()?).ok()
}

async fn load_sub_agent_submit_repair_checkpoint(
    ctx: &AgenticLoopContext<'_>,
    agent_path: &str,
) -> Option<SubmitRepairMode> {
    let tracker = ctx.events.db_tracker?;
    let repo = tracker.repo()?;
    let operation_id = ctx.harness_operation_id?;
    let state = golish_agent_kit::db_shim::operation_state::get(repo, operation_id)
        .await
        .ok()
        .flatten()?;
    let checkpoint = agent_run_from_state_blob(&state.state_blob)?;
    if checkpoint.agent_path != agent_path {
        return None;
    }
    submit_repair_mode_from_agent_run(&checkpoint)
}

async fn persist_sub_agent_submit_repair_checkpoint(
    tracker: Option<golish_agent_kit::db_tracking::DbTracker>,
    operation_id: Option<uuid::Uuid>,
    stage: Option<golish_agent_kit::harness::StageKind>,
    agent_path: String,
    tool_call_id: String,
    directive: RepairDirective,
    mode: SubmitRepairMode,
    result: Value,
) {
    let (Some(tracker), Some(operation_id)) = (tracker, operation_id) else {
        return;
    };
    let Some(repo) = tracker.repo() else {
        return;
    };
    let current = golish_agent_kit::db_shim::operation_state::get(repo, operation_id)
        .await
        .ok()
        .flatten()
        .map(|state| state.state_blob)
        .unwrap_or_default();
    let mode = agent_run_from_state_blob(&current)
        .filter(|checkpoint| checkpoint.agent_path == agent_path)
        .and_then(|checkpoint| submit_repair_mode_from_agent_run(&checkpoint))
        .map(|active_mode| {
            golish_sub_agents::retain_eas_web_repair_targets_for_same_gap(
                mode.clone(),
                &active_mode,
            )
        })
        .unwrap_or(mode);
    let message = mode.model_instruction();
    let job_ids = background_job_ids_from_submit_result(&result);
    let evidence_ids = evidence_ids_from_submit_result(&result);
    let submit_repair_mode = serde_json::to_value(&mode).ok();
    let repair_directive = serde_json::to_value(&directive).ok();
    let checkpoint = AgentRunCheckpoint {
        schema_v: 1,
        operation_id: Some(operation_id),
        stage: stage.map(|stage| stage.as_str().to_string()),
        stage_attempt_id: None,
        agent_path: agent_path.clone(),
        status: AgentRunStatus::RuntimeCorrectionQueued,
        llm_turn_index: None,
        message_chain_ref: None,
        pending_gate_correction: Some(message.clone()),
        pending_submit_only: true,
        submit_repair_mode,
        repair_directive,
        runtime_corrections: vec![RuntimeCorrectionCheckpoint {
            source: "stage_refiner".to_string(),
            kind: format!("submit_{}", mode.kind_str()),
            message,
            job_ids: job_ids.clone(),
            evidence_ids: evidence_ids.clone(),
            submit_allowed: matches!(mode.kind, golish_sub_agents::SubmitRepairKind::EvidenceRefs),
        }],
        background_job_ids: job_ids,
        evidence_watermark: evidence_ids.iter().copied().max(),
        last_tool: Some(ToolCheckpoint {
            tool_call_id,
            tool_name: "submit_stage_deliverable".to_string(),
            state: ToolCheckpointState::Completed,
            result_ref: None,
        }),
        updated_at: chrono::Utc::now(),
    };
    let next = state_blob_with_agent_run(current, &checkpoint);
    if let Err(e) =
        golish_agent_kit::db_shim::operation_state::write_state_blob(repo, operation_id, next).await
    {
        tracing::warn!(
            target: "harness::sub_agent_resume",
            agent_path = %agent_path,
            error = %e,
            "failed to persist submit repair checkpoint"
        );
    }
}

fn state_blob_with_refined_eas_web_repair_checkpoint(
    current: Value,
    agent_path: &str,
    tool_call_id: &str,
    tool_name: &str,
    result: &Value,
    updated_at: chrono::DateTime<chrono::Utc>,
) -> Option<Value> {
    if !matches!(
        tool_name,
        "stage_worklist_next" | "check_stage_asset_coverage"
    ) {
        return None;
    }
    let mut checkpoint = agent_run_from_state_blob(&current)?;
    if checkpoint.agent_path != agent_path {
        return None;
    }
    let active_mode = submit_repair_mode_from_agent_run(&checkpoint)?;
    let refined =
        golish_sub_agents::refine_eas_web_repair_mode_from_worklist(&active_mode, result)?;
    checkpoint.submit_repair_mode = serde_json::to_value(refined).ok();
    checkpoint.last_tool = Some(ToolCheckpoint {
        tool_call_id: tool_call_id.to_string(),
        tool_name: tool_name.to_string(),
        state: ToolCheckpointState::Completed,
        result_ref: None,
    });
    checkpoint.updated_at = updated_at;
    Some(state_blob_with_agent_run(current, &checkpoint))
}

async fn persist_refined_eas_web_repair_checkpoint(
    tracker: Option<golish_agent_kit::db_tracking::DbTracker>,
    operation_id: Option<uuid::Uuid>,
    agent_path: String,
    tool_call_id: String,
    tool_name: String,
    result: Value,
) {
    let (Some(tracker), Some(operation_id)) = (tracker, operation_id) else {
        return;
    };
    let Some(repo) = tracker.repo() else {
        return;
    };
    let Some(current) = golish_agent_kit::db_shim::operation_state::get(repo, operation_id)
        .await
        .ok()
        .flatten()
        .map(|state| state.state_blob)
    else {
        return;
    };
    let Some(next) = state_blob_with_refined_eas_web_repair_checkpoint(
        current,
        &agent_path,
        &tool_call_id,
        &tool_name,
        &result,
        chrono::Utc::now(),
    ) else {
        return;
    };
    if let Err(error) =
        golish_agent_kit::db_shim::operation_state::write_state_blob(repo, operation_id, next).await
    {
        tracing::warn!(
            target: "harness::sub_agent_resume",
            agent_path = %agent_path,
            tool = %tool_name,
            %error,
            "failed to persist refined EAS WEB repair checkpoint"
        );
    }
}

async fn clear_sub_agent_submit_repair_checkpoint(
    tracker: Option<golish_agent_kit::db_tracking::DbTracker>,
    operation_id: Option<uuid::Uuid>,
    agent_path: String,
) {
    let (Some(tracker), Some(operation_id)) = (tracker, operation_id) else {
        return;
    };
    let Some(repo) = tracker.repo() else {
        return;
    };
    let current = golish_agent_kit::db_shim::operation_state::get(repo, operation_id)
        .await
        .ok()
        .flatten()
        .map(|state| state.state_blob)
        .unwrap_or_default();
    let should_clear = agent_run_from_state_blob(&current)
        .map(|checkpoint| {
            checkpoint.agent_path == agent_path && checkpoint.submit_repair_mode.is_some()
        })
        .unwrap_or(false);
    if !should_clear {
        return;
    }
    let next = state_blob_without_agent_run(current);
    if let Err(e) =
        golish_agent_kit::db_shim::operation_state::write_state_blob(repo, operation_id, next).await
    {
        tracing::warn!(
            target: "harness::sub_agent_resume",
            agent_path = %agent_path,
            error = %e,
            "failed to clear submit repair checkpoint"
        );
    }
}

fn sub_agent_tool_observer_needed(
    has_execution_monitor: bool,
    has_db_tracker: bool,
    has_harness_operation: bool,
) -> bool {
    has_execution_monitor || (has_db_tracker && has_harness_operation)
}

fn build_sub_agent_tool_observer(
    ctx: &AgenticLoopContext<'_>,
    agent_id: &str,
    agent_def: &SubAgentDefinition,
    task_desc: &str,
    restored_submit_repair_mode: Option<SubmitRepairMode>,
) -> Option<golish_sub_agents::SubAgentToolObserver> {
    let monitor = ctx.execution_monitor.as_ref().cloned();
    if !sub_agent_tool_observer_needed(
        monitor.is_some(),
        ctx.events.db_tracker.is_some(),
        ctx.harness_operation_id.is_some(),
    ) {
        return None;
    }
    let llm_client = std::sync::Arc::clone(ctx.llm.client);
    let event_tx = (*ctx.events.event_tx).clone();
    let operation_id = ctx
        .harness_operation_id
        .map(|id| id.to_string())
        .or_else(|| ctx.events.session_id.map(str::to_string));
    let stage = ctx
        .harness_stage
        .map(|stage| stage.as_str().to_string())
        .unwrap_or_default();
    let agent_id_for_path = agent_id.to_string();
    let harness_stage = ctx.harness_stage;
    let db_tracker = ctx.events.db_tracker.cloned();
    let harness_operation_id = ctx.harness_operation_id;
    let visible_tools = agent_def.allowed_tools.clone();
    let agent_role = agent_def.id.clone();
    let task_desc = task_desc.to_string();
    let active_repair_directive = restored_submit_repair_mode.map(|mode| mode.model_instruction());

    let observer: golish_sub_agents::SubAgentToolObserver = std::sync::Arc::new(
        move |observation: SubAgentToolObservation| {
            let monitor = monitor.clone();
            let llm_client = std::sync::Arc::clone(&llm_client);
            let event_tx = event_tx.clone();
            let operation_id = operation_id.clone();
            let stage = stage.clone();
            let visible_tools = visible_tools.clone();
            let agent_role = agent_role.clone();
            let task_desc = task_desc.clone();
            let active_repair_directive = active_repair_directive.clone();
            let agent_path = sub_agent_checkpoint_agent_path(
                harness_stage,
                &observation.parent_request_id,
                &agent_id_for_path,
            );
            let db_tracker = db_tracker.clone();
            Box::pin(async move {
                if observation.success
                    && matches!(
                        observation.tool_name.as_str(),
                        "stage_worklist_next" | "check_stage_asset_coverage"
                    )
                {
                    persist_refined_eas_web_repair_checkpoint(
                        db_tracker.clone(),
                        harness_operation_id,
                        agent_path.clone(),
                        observation.tool_call_id.clone(),
                        observation.tool_name.clone(),
                        observation.result.clone(),
                    )
                    .await;
                }
                if observation.tool_name == "submit_stage_deliverable" {
                    if let Some(directive) = repair_directive_from_submit_result(
                        harness_stage,
                        stage_run_org_id_from_request_id(&observation.parent_request_id),
                        agent_path.clone(),
                        &observation.result,
                    ) {
                        if let Some(operation_id) = operation_id.as_deref() {
                            let _ = event_tx.send(AiEvent::HarnessTrace {
                                operation_id: operation_id.to_string(),
                                stage: stage.clone(),
                                agent_path: agent_path.clone(),
                                trace: HarnessTraceKind::StageRefinerDecision {
                                    repair_kind: repair_kind_label(&directive),
                                    root_cause: directive.root_cause.clone(),
                                    action_count: directive.actions.len().min(u32::MAX as usize)
                                        as u32,
                                    gap_count: directive
                                        .submit_guidance
                                        .required_coverage_cells
                                        .len()
                                        .min(u32::MAX as usize)
                                        as u32,
                                    llm_escalated: directive.llm_escalated,
                                    directive_hash: directive.gate_reason_hash.clone(),
                                },
                            });
                        }
                        if let Some(mode) = directive.to_submit_repair_mode() {
                            persist_sub_agent_submit_repair_checkpoint(
                                db_tracker.clone(),
                                harness_operation_id,
                                harness_stage,
                                agent_path.clone(),
                                observation.tool_call_id.clone(),
                                directive,
                                mode,
                                observation.result.clone(),
                            )
                            .await;
                        }
                    } else if matches!(
                        observation.result.get("status").and_then(|s| s.as_str()),
                        Some("accepted" | "received")
                    ) {
                        clear_sub_agent_submit_repair_checkpoint(
                            db_tracker.clone(),
                            harness_operation_id,
                            agent_path.clone(),
                        )
                        .await;
                    }
                }

                let monitor = monitor?;

                let args_summary =
                    serde_json::to_string(&observation.tool_args).unwrap_or_default();
                let monitor_tool_name = golish_agent_kit::harness::underlying_tool_name(
                    &observation.tool_name,
                    &observation.tool_args,
                );
                let result_summary = serde_json::to_string(&observation.result).unwrap_or_default();
                let should_supervise = {
                    let mut mon = monitor.write().await;
                    mon.record_result_and_check(
                        &monitor_tool_name,
                        &args_summary,
                        observation.success,
                        &result_summary,
                    )
                };
                if !should_supervise {
                    return None;
                }

                let (mode, repeated_tool, repeat_count, recent_summary) = {
                    let mon = monitor.read().await;
                    (
                        mon.mode(),
                        mon.repeated_tool_name().to_string(),
                        mon.same_tool_count(),
                        mon.recent_calls_summary(),
                    )
                };
                tracing::info!(
                    "[RuntimeSupervisor] Sub-agent monitor recorded repeated failed tool pattern: '{}' failed {} times in {}",
                    repeated_tool,
                    repeat_count,
                    observation.agent_id,
                );

                let supervisor_ctx = RuntimeSupervisorContext {
                    stage: harness_stage,
                    agent_path: agent_path.clone(),
                    agent_role: agent_role.clone(),
                    task: task_desc.clone(),
                    trigger: "execution_monitor".to_string(),
                    repeated_tool: repeated_tool.clone(),
                    repeat_count,
                    recent_calls: recent_summary.clone(),
                    last_tool_name: observation.tool_name.clone(),
                    last_tool_result: result_summary,
                    visible_tools: visible_tools.clone(),
                    active_repair_directive: active_repair_directive.clone(),
                };
                let user_prompt = runtime_supervisor_user_prompt(&supervisor_ctx);
                let model_response = match runtime_supervisor_one_shot(
                    &llm_client,
                    runtime_supervisor_system_prompt(),
                    &user_prompt,
                )
                .await
                {
                    Ok(response) => Some(response),
                    Err(e) => {
                        tracing::warn!(
                            target: "harness::runtime_supervisor",
                            agent_id = %observation.agent_id,
                            error = %e,
                            "sub-agent runtime supervisor LLM call failed; using deterministic fallback"
                        );
                        None
                    }
                };
                let directive =
                    directive_from_model_response(&supervisor_ctx, model_response.as_deref());
                let injected = mode.injects();
                tracing::info!(
                    target: "harness::runtime_supervisor",
                    mode = mode.as_str(),
                    repeated_tool = %repeated_tool,
                    repeat_count,
                    agent_id = %observation.agent_id,
                    parent_request_id = %observation.parent_request_id,
                    strategy_kind = directive.strategy_kind_label(),
                    directive_hash = %directive.directive_hash,
                    root_cause = %truncate_str(&directive.root_cause, 500),
                    injected,
                    "sub-agent runtime supervisor decision recorded"
                );
                if let Some(operation_id) = operation_id {
                    let trace = AiEvent::HarnessTrace {
                        operation_id,
                        stage,
                        agent_path,
                        trace: HarnessTraceKind::RuntimeSupervisorDecision {
                            mode: mode.as_str().to_string(),
                            trigger: "execution_monitor".to_string(),
                            tool: repeated_tool.clone(),
                            repeat_count: repeat_count.min(u32::MAX as usize) as u32,
                            injected,
                            strategy_kind: directive.strategy_kind_label().to_string(),
                            root_cause: directive.root_cause.clone(),
                            action_count: directive.actions.len().min(u32::MAX as usize) as u32,
                            directive_hash: directive.directive_hash.clone(),
                        },
                    };
                    let _ = event_tx.send(trace);
                }

                {
                    let mut mon = monitor.write().await;
                    mon.reset_after_supervisor();
                }

                injected.then(|| {
                    directive.model_instruction(matches!(
                        mode,
                        golish_agent_kit::loop_detection::ExecutionMonitorMode::HardInject
                    ))
                })
            })
        },
    );
    Some(observer)
}

/// Handle sub-agent tool calls (tool names starting with `sub_agent_`).
pub(super) async fn execute_sub_agent_call<M>(
    tool_name: &str,
    tool_args: &serde_json::Value,
    ctx: &AgenticLoopContext<'_>,
    model: &M,
    context: &SubAgentContext,
    tool_id: &str,
) -> Result<ToolExecutionResult>
where
    M: RigCompletionModel + Sync,
{
    execute_sub_agent_call_with_bound(tool_name, tool_args, ctx, model, context, tool_id, None)
        .await
}

/// Execute a sub-agent against an optional server-owned V2 worker binding.
/// Ordinary callers use [`execute_sub_agent_call`] and retain legacy chain
/// create/resume behavior; stage_run is the only live caller allowed to pass a
/// prebound worker.
pub(super) async fn execute_sub_agent_call_with_bound<M>(
    tool_name: &str,
    tool_args: &serde_json::Value,
    ctx: &AgenticLoopContext<'_>,
    model: &M,
    context: &SubAgentContext,
    tool_id: &str,
    mut bound_worker_chain: Option<BoundWorkerChainContext>,
) -> Result<ToolExecutionResult>
where
    M: RigCompletionModel + Sync,
{
    let agent_id = tool_name.strip_prefix("sub_agent_").unwrap_or("");

    let registry = ctx.sub_agent_registry.read().await;
    let mut agent_def = match registry.get(agent_id) {
        Some(def) => def.clone(),
        None => {
            return Ok(ToolExecutionResult {
                value: json!({ "error": format!("Sub-agent '{}' not found", agent_id) }),
                success: false,
            });
        }
    };
    drop(registry);
    let production_target_intel_goal = ctx.harness_stage
        == Some(golish_agent_kit::harness::StageKind::TargetIntel)
        && ctx.harness_operation_id.is_some();
    let unified_investigation_reducer =
        unified_investigation_reducer_request(ctx.harness_stage, tool_id);
    let target_intel_company_controller = bound_worker_chain
        .as_ref()
        .is_some_and(|bound| bound.stage_team_leader.is_some());
    let target_intel_final_submitter = bound_worker_chain.as_ref().is_some_and(|bound| {
        bound.return_on_first_durable_stage_submission && bound.target_intel_review.is_none()
    });
    let target_intel_reviewer = bound_worker_chain
        .as_ref()
        .is_some_and(|bound| bound.target_intel_review.is_some());

    if unified_investigation_reducer {
        agent_def.name = "Investigation sealed-output reducer".to_string();
        agent_def.description =
            "Reduces one immutable Investigation child-output census into a typed host proposal"
                .to_string();
        agent_def.system_prompt = "You are a narrow typed reducer. The task contains the complete immutable child-output census, exact accepted hashes, subject allowlist, proof selectors, and output JSON contract. Do not plan, search, explain, narrate, or call any tool except submit_result. Think only as much as needed to select and deduplicate the strongest child proposals, then make submit_result your first and only tool call. Copy opaque identifiers and hashes exactly. The host rejects any invented authority or malformed type.".to_string();
        agent_def.allowed_tools = vec!["submit_result".to_string()];
        agent_def.max_iterations = 1;
        agent_def.max_tokens = Some(32_768);
        agent_def.prompt_template = None;
        agent_def.readonly = true;
        agent_def.delegatable_agents.clear();
        if let Some(bound) = bound_worker_chain.as_mut() {
            bound.reset_provider_history = true;
        }
    } else if target_intel_reviewer {
        agent_def.id = "target_intel_reviewer".to_string();
        agent_def.name = "Target Intel read-only reviewer".to_string();
        agent_def.description =
            "Host-bound review of one immutable Target Intel bundle".to_string();
        agent_def.system_prompt = golish_sub_agents::render_neutral_reviewer_prompt().to_string();
        agent_def.allowed_tools = vec![
            golish_sub_agents::TARGET_INTEL_READ_REVIEW_SECTION.to_string(),
            golish_sub_agents::TARGET_INTEL_RECORD_REVIEW_VERDICT.to_string(),
        ];
        agent_def.max_iterations = 8;
        agent_def.max_tokens = Some(12_288);
        agent_def.prompt_template = None;
        agent_def.readonly = true;
        agent_def.delegatable_agents.clear();
    } else if production_target_intel_goal
        && (target_intel_company_controller || target_intel_final_submitter)
    {
        agent_def.id = "target_intel_company_controller".to_string();
        agent_def.name = "Target Intel autonomous Company Controller".to_string();
        agent_def.description =
            "Owns one company's adaptive semantic discovery plan and completion claim".to_string();
        agent_def.system_prompt = golish_sub_agents::render_neutral_controller_prompt().to_string();
        agent_def.allowed_tools = if target_intel_final_submitter {
            vec!["submit_stage_deliverable".to_string()]
        } else {
            vec![
                "update_plan".to_string(),
                "recon_search_intel".to_string(),
                STAGE_TEAM_DISPATCH_WORKERS_TOOL_NAME.to_string(),
                STAGE_TEAM_PREPARE_FINAL_SUBMISSION_TOOL_NAME.to_string(),
            ]
        };
        agent_def.prompt_template = None;
        agent_def.readonly = false;
        agent_def.delegatable_agents.clear();
    } else if production_target_intel_goal {
        agent_def.id = "target_intel_generic_worker".to_string();
        agent_def.name = "Target Intel generic evidence worker".to_string();
        agent_def.description =
            "Executes one bounded semantic frontier task without owning the Goal plan".to_string();
        agent_def.system_prompt =
            golish_sub_agents::neutral_target_intel_worker_system_prompt().to_string();
        agent_def.allowed_tools = vec![
            "recon_search_intel".to_string(),
            "list_recent_evidence".to_string(),
        ];
        agent_def.prompt_template = None;
        agent_def.readonly = false;
        agent_def.delegatable_agents.clear();
    }

    // Runtime Memory persists every Stage Team actor under the closed DB
    // `pentester` family, while Target Intel deliberately gives each bound
    // actor a host-owned prompt/runtime identity. Keep the bound chain's exact
    // executor identity in lockstep with that rewrite. Leaving the frozen
    // specialist name (`recon`) here makes `maybe_restore_chain` correctly
    // reject the same worker before its first provider turn.
    if production_target_intel_goal {
        if let Some(bound) = bound_worker_chain.as_mut() {
            bound.agent_type.clone_from(&agent_def.id);
        }
    }

    let fixture_public_tools = ctx
        .target_intel_goal_shadow
        .is_some_and(|fixture| fixture.strict_passive_public_tools_enabled())
        && ctx.harness_operation_id.is_none();
    let tool_provider = DefaultToolProvider::with_db_tracker(ctx.events.db_tracker)
        .with_intel_public_fixture(fixture_public_tools, ctx.intel_public_adapter.clone());
    let effective_harness_org_id = stage_run_org_id_from_request_id(tool_id).or(ctx.harness_org_id);
    let agent_path = sub_agent_checkpoint_agent_path(ctx.harness_stage, tool_id, agent_id);
    let restored_submit_repair_mode =
        load_sub_agent_submit_repair_checkpoint(ctx, &agent_path).await;

    let task_desc = tool_args.get("task").and_then(|v| v.as_str()).unwrap_or("");
    // AI-controlled resume: a prior sub-agent session id continues that exact
    // worker; `true` continues this agent's latest chain; absent/false = fresh.
    let resume_arg: Option<String> = if bound_worker_chain.is_some() {
        None
    } else {
        match tool_args.get("resume") {
            Some(serde_json::Value::String(s)) if !s.trim().is_empty() => {
                Some(s.trim().to_string())
            }
            Some(serde_json::Value::Bool(true)) => Some("latest".to_string()),
            _ => None,
        }
    };

    let project_id = {
        let ws = ctx.workspace.read().await;
        ws.to_string_lossy().to_string()
    };
    let project_id_opt = if project_id == "." || project_id.is_empty() {
        None
    } else {
        Some(project_id)
    };

    // Route tools that are exposed to sub-agents but live outside the plain
    // ToolRegistry. Without this, read-only stage helpers like
    // list_in_scope_targets are advertised to the model but fail at runtime as
    // UnknownTool.
    let sub_tool_router: Option<golish_sub_agents::SubAgentToolRouter> = {
        let graph = ctx.graph_backend.clone();
        let tracker = ctx.events.db_tracker.cloned();
        let receipt_repo = tracker
            .as_ref()
            .and_then(golish_agent_kit::db_tracking::DbTracker::repo_arc);
        let project_path = project_id_opt.clone();
        let session_id = ctx.events.session_id.map(str::to_string);
        let harness_org_id = effective_harness_org_id;
        let harness_stage = ctx.harness_stage;
        let harness_operation_id = ctx.harness_operation_id;
        let runtime_memory = ctx.runtime_memory.clone();
        let stage_team_bound = bound_worker_chain.clone();
        let router: golish_sub_agents::SubAgentToolRouter =
            std::sync::Arc::new(move |name: String, args: serde_json::Value| {
                let graph = graph.clone();
                let tracker = tracker.clone();
                let receipt_repo = receipt_repo.clone();
                let project_path = project_path.clone();
                let session_id = session_id.clone();
                let harness_stage = harness_stage;
                let harness_operation_id = harness_operation_id;
                let runtime_memory = runtime_memory.clone();
                let stage_team_bound = stage_team_bound.clone();
                Box::pin(async move {
                    let tool_context = golish_core::current_agent_tool_context();
                    if let Some(result) = route_enumeration_producer_preflight(
                        &name,
                        &args,
                        harness_stage,
                        receipt_repo.as_ref(),
                        stage_team_bound.as_ref(),
                        tool_context.as_ref(),
                    )
                    .await
                    {
                        return Some(result);
                    }
                    if let Some(result) = route_enumeration_receipt_reducer_tool(
                        &name,
                        &args,
                        harness_stage,
                        receipt_repo.as_ref(),
                        stage_team_bound.as_ref(),
                        tool_context.as_ref(),
                    )
                    .await
                    {
                        return Some(result);
                    }
                    if let Some(result) = route_target_intel_reviewer_host_tool(
                        &name,
                        &args,
                        runtime_memory.as_ref(),
                        stage_team_bound.as_ref(),
                        tool_context.as_ref(),
                    )
                    .await
                    {
                        return Some(result);
                    }
                    if let Some(result) = route_stage_team_leader_host_tool(
                        &name,
                        &args,
                        runtime_memory.as_ref(),
                        stage_team_bound.as_ref(),
                        tool_context.as_ref(),
                    )
                    .await
                    {
                        return Some(result);
                    }

                    if let Some(result) =
                        golish_agent_kit::tool_executors::execute_security_analysis_tool(
                            &name,
                            &args,
                            tracker.as_ref(),
                            project_path.as_deref(),
                            session_id.as_deref(),
                            harness_org_id,
                            harness_stage,
                            harness_operation_id,
                        )
                        .await
                    {
                        return Some(result);
                    }

                    match graph {
                        Some(graph) => {
                            golish_agent_kit::tool_executors::execute_graph_tool(
                                &name,
                                &args,
                                Some(graph.as_ref()),
                            )
                            .await
                        }
                        None => None,
                    }
                })
                    as std::pin::Pin<
                        Box<
                            dyn std::future::Future<Output = Option<(serde_json::Value, bool)>>
                                + Send,
                        >,
                    >
            });
        Some(router)
    };
    let briefing = if ctx.harness_stage.is_some() {
        let worker_run_id = bound_worker_chain
            .as_ref()
            .map(|bound| bound.worker_lease.worker_run_id)
            .or_else(|| ctx.worker_lease.as_ref().map(|lease| lease.worker_run_id));
        let bound_identity = bound_worker_chain
            .as_ref()
            .map(|bound| BoundScopedContextIdentity {
                operation_id: bound.operation_id,
                stage_execution_id: bound.stage_execution_id,
                stage_run_unit_id: bound.worker_lease.stage_run_unit_id,
                worker_run_id: bound.worker_lease.worker_run_id,
                organization_id: bound.organization_id,
            });
        match retrieve_scoped_context_data(
            ctx,
            task_desc,
            effective_harness_org_id,
            worker_run_id,
            bound_identity,
        )
        .await
        {
            Ok(context) => context,
            Err(error) => {
                tracing::warn!(
                    target: "harness::knowledge_context",
                    %error,
                    "sub-agent scoped ContextPack unavailable; refusing legacy global fallback"
                );
                Some(format!(
                    "[SCOPED CONTEXT UNAVAILABLE] code={error}; do not use global or sibling customer memory."
                ))
            }
        }
    } else {
        build_sub_agent_briefing(
            ctx.events.db_tracker,
            ctx.graph_backend.as_deref(),
            project_id_opt.as_deref(),
            agent_id,
            task_desc,
        )
        .await
    };
    let deny_vuln_finding = deny_vuln_finding_writes(ctx);
    let investigation_cognition_only =
        ctx.harness_stage == Some(golish_agent_kit::harness::StageKind::Investigation);
    // Per-stage tool boundary for the delegated sub-agent: inside a harness
    // stage, enforce the category whitelist (deny-by-default) — a scan invocation
    // must resolve to a tool type in this stage's `allowed_tool_types`. Agent/meta
    // tools are exempt (not scan invocations). Built once; the `Arc<Fn>` is
    // cloned cheaply into each sub_ctx below.
    // See docs/design/2026-06-02-stage-tool-whitelist-enforcement.md.
    let stage_tool_guard: Option<golish_sub_agents::StageToolGuard> = ctx
        .harness_stage
        .and_then(|kind| golish_agent_kit::harness::load_embedded_stage_spec(kind).ok())
        .map(|spec| {
            let stage_id = spec.id.clone();
            let allowed = spec.allowed_tool_types.clone();
            let guard: golish_sub_agents::StageToolGuard =
                std::sync::Arc::new(move |tn: &str, args: &serde_json::Value| {
                    if production_target_intel_goal
                        && !production_target_intel_sub_agent_tool_allowed(
                            tn,
                            target_intel_company_controller,
                            target_intel_final_submitter,
                            target_intel_reviewer,
                        )
                    {
                        return Err(format!(
                            "TARGET_INTEL_GOAL_TOOL_FORBIDDEN: tool '{tn}' is outside this bound Intel Goal actor's closed contract"
                        ));
                    }
                    if investigation_cognition_only
                        && !unified_investigation_cognitive_tool_allowed(tn)
                    {
                        return Err(format!(
                            "Tool '{tn}' is not available to an Investigation cognitive actor. Submit typed strategy/action intent or delegate another cognition-only worker; external I/O is owned by the host-compiled Operator."
                        ));
                    }
                    if deny_vuln_finding && tn == "record_finding" {
                        return Err(
                            "record_finding is not permitted in vuln_triage; the Nuclei scanner records observations/evidence only"
                                .to_string(),
                        );
                    }
                    if golish_agent_kit::harness::is_scan_invocation(tn, args)
                        && !golish_agent_kit::harness::stage_allows(tn, args, &allowed)
                    {
                        // D2 · precise, self-correcting feedback: name the resolved
                        // inner tool, list what IS allowed in this stage, and tell
                        // the model not to retry the same tool — so it corrects
                        // instead of hammering a denied tool (the 26x-retry case).
                        let inner = golish_agent_kit::harness::underlying_tool_name(tn, args);
                        let allowed_list = if allowed.is_empty() {
                            "(none — this stage runs no scan tools)".to_string()
                        } else {
                            allowed.join(", ")
                        };
                        return Err(format!(
                            "Tool '{inner}' is not permitted in the '{stage_id}' stage. \
                             Allowed tool types here: {allowed_list}. Use one of those, or if this \
                             stage's work is complete, submit your StageDeliverable to advance — \
                             do not retry '{inner}'."
                        ));
                    }
                    Ok(())
                });
            guard
        });

    // D1 · also hide scan tools from the delegated sub-agent's *tool list* when
    // the active stage permits none (e.g. scoping) — so the model never even sees
    // `pentest_run` and can't spin retrying it (the 26x-retry case in scoping).
    // Mirrors the main agent's `hide_scans_for_zero_scan_stage`; the call-time
    // `stage_tool_guard` above stays as the backstop.
    let hide_scan_tools = ctx
        .harness_stage
        .and_then(|kind| golish_agent_kit::harness::load_embedded_stage_spec(kind).ok())
        .is_some_and(|spec| spec.allowed_tool_types.is_empty());
    let hide_tool_in_stage: Option<golish_sub_agents::StageToolHider> = (hide_scan_tools
        || deny_vuln_finding
        || investigation_cognition_only
        || production_target_intel_goal)
        .then(|| {
            let hider: golish_sub_agents::StageToolHider =
                std::sync::Arc::new(move |name: &str| {
                    sub_agent_stage_tool_hidden(
                        name,
                        hide_scan_tools,
                        deny_vuln_finding,
                        investigation_cognition_only,
                    ) || (production_target_intel_goal
                        && !production_target_intel_sub_agent_tool_allowed(
                            name,
                            target_intel_company_controller,
                            target_intel_final_submitter,
                            target_intel_reviewer,
                        ))
                });
            hider
        });

    let sub_tool_result_hook: Option<golish_sub_agents::SubAgentToolResultHook> =
        ctx.harness_stage.map(|stage| {
            let tracker = ctx.events.db_tracker.cloned();
            let receipt_repo = tracker
                .as_ref()
                .and_then(golish_agent_kit::db_tracking::DbTracker::repo_arc);
            let session_id = ctx.events.session_id.map(str::to_string);
            let harness_operation_id = ctx.harness_operation_id;
            let stage_execution_id = ctx.stage_execution_id;
            let harness_org_id = effective_harness_org_id;
            let hook: golish_sub_agents::SubAgentToolResultHook = std::sync::Arc::new(
                move |tool_name: String,
                      tool_args: serde_json::Value,
                      result: serde_json::Value,
                      success: bool| {
                    let tracker = tracker.clone();
                    let receipt_repo = receipt_repo.clone();
                    let session_id = session_id.clone();
                    Box::pin(async move {
                        let (mut result, mut persisted_success) = if stage
                            == golish_agent_kit::harness::StageKind::Enumeration
                        {
                            settle_enumeration_producer_result(
                                receipt_repo.as_ref(),
                                &tool_name,
                                result,
                                success,
                            )
                            .await
                        } else {
                            (result, success)
                        };
                        match super::record_recon_passive_evidence(
                            tracker.as_ref(),
                            session_id.as_deref(),
                            Some(stage),
                            harness_operation_id,
                            stage_execution_id,
                            harness_org_id,
                            &tool_name,
                            &tool_args,
                            &result,
                            persisted_success,
                        )
                        .await
                        {
                            Ok(Some(id)) => {
                                if let Some(obj) = result.as_object_mut() {
                                    obj.insert("_evidence_id".to_string(), json!(id));
                                    obj.insert("outcome_persisted".to_string(), json!(true));
                                }
                            }
                            Ok(None) => {}
                            Err(error) => {
                                tracing::warn!(
                                    target: "harness::evidence",
                                    tool = %tool_name,
                                    %error,
                                    "sub-agent Target Intel persistence incomplete; returning retryable error"
                                );
                                persisted_success = false;
                                if let Some(obj) = result.as_object_mut() {
                                    obj.insert(
                                        "error".to_string(),
                                        json!("Target Intel evidence/source status persistence was incomplete; retry this recon action"),
                                    );
                                    obj.insert("completion_state".to_string(), json!("partial"));
                                    obj.insert("outcome_persisted".to_string(), json!(false));
                                }
                            }
                        }
                        (result, persisted_success)
                    })
                        as std::pin::Pin<
                            Box<dyn std::future::Future<Output = (serde_json::Value, bool)> + Send>,
                        >
                },
            );
            hook
        });
    let sub_tool_observer = build_sub_agent_tool_observer(
        ctx,
        agent_id,
        &agent_def,
        task_desc,
        restored_submit_repair_mode.clone(),
    );

    // P0-4: persist dispatch lifecycle so the next session can list
    // mid-flight invocations after a crash/restart. Best-effort —
    // missing tracker / repo / DB error leaves dispatch_id = None and
    // the lifecycle becomes a no-op.
    let dispatch_id: Option<uuid::Uuid> = if let Some(tracker) = ctx.events.db_tracker {
        if let Some(repo) = tracker.repo() {
            match golish_agent_kit::db_shim::sub_agent_dispatches::record_start(
                repo,
                tracker.session_uuid(),
                None, // parent_dispatch_id: tree-tracking deferred (P1)
                agent_id,
                Some(tool_id),
                0, // depth: tracking deferred (P1)
                tool_args,
            )
            .await
            {
                Ok(id) => Some(id),
                Err(e) => {
                    tracing::warn!(
                        agent_id = agent_id,
                        error = %e,
                        "[dispatch-track] record_start failed; proceeding without persistence",
                    );
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    let result = if let Some((override_provider, override_model)) = &agent_def.model_override {
        let override_client = if let Some(factory) = ctx.llm.model_factory {
            match factory
                .get_or_create(override_provider, override_model)
                .await
            {
                Ok(client) => Some(client),
                Err(e) => {
                    tracing::warn!(
                        "Failed to create override model {}/{} for sub-agent '{}': {}. Using main model.",
                        override_provider,
                        override_model,
                        agent_id,
                        e
                    );
                    None
                }
            }
        } else {
            tracing::warn!(
                "Sub-agent '{}' has model override but no factory available. Using main model.",
                agent_id
            );
            None
        };

        if let Some(client) = override_client {
            tracing::info!(
                "[sub-agent:{}] Executing with override model: provider={}, model={}",
                agent_id,
                override_provider,
                override_model
            );
            let sub_ctx = SubAgentExecutorContext {
                event_tx: ctx.events.event_tx,
                tool_registry: ctx.tool_registry,
                workspace: ctx.workspace,
                provider_name: override_provider,
                model_name: override_model,
                resume: resume_arg.clone(),
                sub_tool_router: sub_tool_router.clone(),
                active_org_id_source: ctx.harness_org_id_source.clone(),
                active_org_id_override: effective_harness_org_id,
                operation_id: ctx.harness_operation_id,
                session_id: ctx.events.session_id,
                persistence_session_id: ctx
                    .events
                    .db_tracker
                    .map(golish_agent_kit::db_tracking::DbTracker::session_uuid),
                transcript_base_dir: ctx.events.transcript_base_dir,
                api_request_stats: Some(ctx.api_request_stats),
                cancelled: ctx.cancelled,
                briefing: briefing.clone(),
                temperature_override: agent_def.temperature,
                max_tokens_override: agent_def.max_tokens,
                top_p_override: agent_def.top_p,
                chain_persistence: ctx.chain_persistence.as_ref(),
                bound_worker_chain: bound_worker_chain.clone(),
                sub_agent_registry: Some(ctx.sub_agent_registry),
                post_shell_hook: ctx.post_shell_hook.clone(),
                post_tool_result_hook: sub_tool_result_hook.clone(),
                tool_observer: sub_tool_observer.clone(),
                initial_submit_repair_mode: restored_submit_repair_mode.clone(),
                stage_tool_guard: stage_tool_guard.clone(),
                hide_tool_in_stage: hide_tool_in_stage.clone(),
            };
            execute_sub_agent_with_client(
                &agent_def,
                tool_args,
                context,
                &client,
                sub_ctx,
                &tool_provider,
                tool_id,
            )
            .await
        } else {
            tracing::info!(
                "[sub-agent:{}] Executing with main model (override failed): provider={}, model={}",
                agent_id,
                ctx.llm.provider_name,
                ctx.llm.model_name
            );
            let sub_ctx = SubAgentExecutorContext {
                event_tx: ctx.events.event_tx,
                tool_registry: ctx.tool_registry,
                workspace: ctx.workspace,
                provider_name: ctx.llm.provider_name,
                model_name: ctx.llm.model_name,
                resume: resume_arg.clone(),
                sub_tool_router: sub_tool_router.clone(),
                active_org_id_source: ctx.harness_org_id_source.clone(),
                active_org_id_override: effective_harness_org_id,
                operation_id: ctx.harness_operation_id,
                session_id: ctx.events.session_id,
                persistence_session_id: ctx
                    .events
                    .db_tracker
                    .map(golish_agent_kit::db_tracking::DbTracker::session_uuid),
                transcript_base_dir: ctx.events.transcript_base_dir,
                api_request_stats: Some(ctx.api_request_stats),
                cancelled: ctx.cancelled,
                briefing: briefing.clone(),
                temperature_override: agent_def.temperature,
                max_tokens_override: agent_def.max_tokens,
                top_p_override: agent_def.top_p,
                chain_persistence: ctx.chain_persistence.as_ref(),
                bound_worker_chain: bound_worker_chain.clone(),
                sub_agent_registry: Some(ctx.sub_agent_registry),
                post_shell_hook: ctx.post_shell_hook.clone(),
                post_tool_result_hook: sub_tool_result_hook.clone(),
                tool_observer: sub_tool_observer.clone(),
                initial_submit_repair_mode: restored_submit_repair_mode.clone(),
                stage_tool_guard: stage_tool_guard.clone(),
                hide_tool_in_stage: hide_tool_in_stage.clone(),
            };
            execute_sub_agent(
                &agent_def,
                tool_args,
                context,
                model,
                sub_ctx,
                &tool_provider,
                tool_id,
            )
            .await
        }
    } else {
        tracing::info!(
            "[sub-agent:{}] Executing with main model (no override): provider={}, model={}",
            agent_id,
            ctx.llm.provider_name,
            ctx.llm.model_name
        );
        let sub_ctx = SubAgentExecutorContext {
            event_tx: ctx.events.event_tx,
            tool_registry: ctx.tool_registry,
            workspace: ctx.workspace,
            provider_name: ctx.llm.provider_name,
            model_name: ctx.llm.model_name,
            resume: resume_arg.clone(),
            sub_tool_router: sub_tool_router.clone(),
            active_org_id_source: ctx.harness_org_id_source.clone(),
            active_org_id_override: effective_harness_org_id,
            operation_id: ctx.harness_operation_id,
            session_id: ctx.events.session_id,
            persistence_session_id: ctx
                .events
                .db_tracker
                .map(golish_agent_kit::db_tracking::DbTracker::session_uuid),
            transcript_base_dir: ctx.events.transcript_base_dir,
            api_request_stats: Some(ctx.api_request_stats),
            cancelled: ctx.cancelled,
            briefing,
            temperature_override: agent_def.temperature,
            max_tokens_override: agent_def.max_tokens,
            top_p_override: agent_def.top_p,
            chain_persistence: ctx.chain_persistence.as_ref(),
            bound_worker_chain,
            sub_agent_registry: Some(ctx.sub_agent_registry),
            post_shell_hook: ctx.post_shell_hook.clone(),
            post_tool_result_hook: sub_tool_result_hook.clone(),
            tool_observer: sub_tool_observer.clone(),
            initial_submit_repair_mode: restored_submit_repair_mode.clone(),
            stage_tool_guard: stage_tool_guard.clone(),
            hide_tool_in_stage: hide_tool_in_stage.clone(),
        };
        execute_sub_agent(
            &agent_def,
            tool_args,
            context,
            model,
            sub_ctx,
            &tool_provider,
            tool_id,
        )
        .await
    };

    // P0-4: complement record_start above with record_finish so the
    // dispatch row gets `completed/failed` + result/error before we
    // hand control back to the caller. Best-effort like record_start.
    if let (Some(id), Some(tracker)) = (dispatch_id, ctx.events.db_tracker) {
        if let Some(repo) = tracker.repo() {
            let (status, result_json, err_msg) = match &result {
                Ok(r) => (
                    dispatch_status_for_sub_agent_success(r.success),
                    Some(serde_json::json!({
                        "agent_id": r.agent_id,
                        "response": truncate_str(&r.response, 1000),
                        "success": r.success,
                        "duration_ms": r.duration_ms,
                    })),
                    (!r.success).then(|| truncate_str(&r.response, 1000).to_string()),
                ),
                Err(e) => (
                    golish_agent_kit::db_traits::DispatchStatus::Failed,
                    None,
                    Some(e.to_string()),
                ),
            };
            if let Err(e) = golish_agent_kit::db_shim::sub_agent_dispatches::record_finish(
                repo,
                id,
                status,
                result_json.as_ref(),
                err_msg.as_deref(),
            )
            .await
            {
                tracing::warn!(
                    dispatch_id = %id,
                    error = %e,
                    "[dispatch-track] record_finish failed",
                );
            }
        }
    }

    match result {
        Ok(result) => {
            // C2c · Capture a delegated sub-agent's StageDeliverable so the
            // Task-mode gate can see it even when the Primary orchestrator
            // narrates instead of inlining the JSON. Heuristic: the result
            // carries the `stage_run_id` signature unique to a StageDeliverable.
            // The Task-mode executor reads + appends the last one captured.
            if let Some(sink) = ctx.harness_deliverable_sink.as_ref() {
                if result.response.contains("stage_run_id") {
                    *sink.write().await = Some(result.response.clone());
                }
            }

            if let Some(tracker) = ctx.events.db_tracker {
                let result_preview = truncate_str(&result.response, 500);
                tracker.record_agent_call(
                    "primary",
                    agent_id,
                    &context.original_request,
                    Some(result_preview),
                    result.duration_ms,
                );
            }

            // P-C (KG auto-extract): scan the sub-agent's response text
            // for IP/CVE/URL mentions and upsert them into the graph.
            // Fire-and-forget so it never blocks the agent loop; missing
            // graph backend / DB error is logged + ignored inside.
            if let Some(graph) = ctx.graph_backend.clone() {
                let response_text = result.response.clone();
                let pid = project_id_opt.clone();
                tokio::spawn(async move {
                    let stats =
                        extract_and_upsert_entities(graph.as_ref(), &response_text, pid.as_deref())
                            .await;
                    if stats.nodes > 0 || stats.edges > 0 {
                        tracing::info!(
                            nodes = stats.nodes,
                            edges = stats.edges,
                            "[kg-extract] auto-upserted from sub-agent response"
                        );
                    }
                });
            }

            Ok(sub_agent_tool_execution_result(result))
        }
        Err(error) => Ok(sub_agent_execution_error_result(error)),
    }
}

fn stage_run_org_id_from_request_id(request_id: &str) -> Option<uuid::Uuid> {
    let (_, org_id) = request_id.rsplit_once("::org::")?;
    uuid::Uuid::parse_str(org_id).ok()
}

#[cfg(test)]
mod tests {
    use super::{
        agent_run_from_state_blob, canonical_enumeration_dependency_receipt_ids,
        dispatch_status_for_sub_agent_success, enumeration_producer_args_target_and_origin,
        enumeration_reducer_subject, enumeration_result_target_and_origin,
        production_target_intel_sub_agent_tool_allowed, route_stage_team_leader_host_tool,
        settle_enumeration_producer_result, stage_run_org_id_from_request_id,
        state_blob_with_refined_eas_web_repair_checkpoint, sub_agent_checkpoint_agent_path,
        sub_agent_execution_error_result, sub_agent_stage_tool_hidden,
        sub_agent_tool_execution_result, sub_agent_tool_observer_needed,
        submit_repair_mode_from_agent_run, unified_investigation_cognitive_tool_allowed,
        unified_investigation_reducer_request, vuln_triage_hides_record_finding,
    };
    use golish_agent_kit::harness::StageKind;
    use golish_agent_kit::task_orchestrator::agent_run_checkpoint::{
        state_blob_with_agent_run, AgentRunCheckpoint, AgentRunStatus, RuntimeCorrectionCheckpoint,
        ToolCheckpointState,
    };
    use golish_sub_agents::{SubAgentContext, SubAgentResult, SubmitRepairKind, SubmitRepairMode};

    use async_trait::async_trait;
    use golish_agent_kit::db_traits::{
        CreateRuntimeOperation, CreatedRuntimeOperation, ProjectScopeRegistration,
        RequestStageWorker, RequestedStageWorkerView, RuntimeMemoryError, RuntimeMemoryRepository,
        StageWorkerRequestDecision, StageWorkerRequestView,
    };
    use golish_sub_agents::{
        BoundWorkerChainContext, StageTeamCompiledActionBinding, StageTeamLeaderBinding,
        STAGE_TEAM_DISPATCH_WORKERS_TOOL_NAME, STAGE_TEAM_PREPARE_FINAL_SUBMISSION_TOOL_NAME,
    };
    use std::sync::atomic::{AtomicBool, AtomicI64};
    use std::sync::{Arc, Mutex};

    #[test]
    fn enumeration_receipt_recovery_canonicalizes_dependency_manifest() {
        let first = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let second = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();

        assert_eq!(
            canonical_enumeration_dependency_receipt_ids(vec![second, first, second]),
            vec![first, second]
        );
    }

    #[derive(Default)]
    struct RecordingStageTeamRuntime {
        requests: Mutex<Vec<RequestStageWorker>>,
        reject_all: bool,
        fail_on_request_number: Option<usize>,
    }

    #[async_trait]
    impl RuntimeMemoryRepository for RecordingStageTeamRuntime {
        async fn project_scope_register_first_open(
            &self,
            _canonical_path: &str,
            _path_sha256: &str,
        ) -> Result<ProjectScopeRegistration, RuntimeMemoryError> {
            Err(RuntimeMemoryError::Unavailable)
        }

        async fn project_scope_rename(
            &self,
            _project_scope_id: uuid::Uuid,
            _expected_old_path: &str,
            _expected_row_version: i64,
            _new_path: &str,
            _new_path_sha256: &str,
        ) -> Result<ProjectScopeRegistration, RuntimeMemoryError> {
            Err(RuntimeMemoryError::Unavailable)
        }

        async fn create_runtime_operation(
            &self,
            _input: CreateRuntimeOperation,
        ) -> Result<CreatedRuntimeOperation, RuntimeMemoryError> {
            Err(RuntimeMemoryError::Unavailable)
        }

        async fn request_stage_worker(
            &self,
            input: RequestStageWorker,
        ) -> Result<RequestedStageWorkerView, RuntimeMemoryError> {
            let request_number = {
                let mut requests = self.requests.lock().unwrap();
                requests.push(input.clone());
                requests.len()
            };
            if self.fail_on_request_number == Some(request_number) {
                return Err(RuntimeMemoryError::Unavailable);
            }
            let decision = if self.reject_all {
                StageWorkerRequestDecision::Rejected
            } else {
                StageWorkerRequestDecision::Accepted
            };
            Ok(RequestedStageWorkerView {
                request: StageWorkerRequestView {
                    id: uuid::Uuid::new_v4(),
                    stage_team_plan_id: input.stage_team_plan_id,
                    parent_work_item_id: input.parent_work_item_id,
                    requested_by_worker_run_id: input.fence.worker_run_id,
                    dispatch_epoch: input.expected_dispatch_epoch,
                    requested_role: input.requested_role,
                    requested_kind: input.requested_kind,
                    subject_refs: input.subject_refs,
                    reason: input.reason,
                    output_schema: input.output_schema,
                    budget_hint: input.budget_hint,
                    dedupe_key: input.dedupe_key,
                    decision,
                    decision_code: decision.as_str().to_string(),
                    created_work_item_id: (decision == StageWorkerRequestDecision::Accepted)
                        .then(uuid::Uuid::new_v4),
                    request_sha256: input.request_sha256,
                },
                work_item: None,
                replayed: false,
            })
        }
    }

    fn stage_team_leader_bound() -> BoundWorkerChainContext {
        BoundWorkerChainContext {
            operation_id: uuid::Uuid::new_v4(),
            stage_execution_id: uuid::Uuid::new_v4(),
            organization_id: uuid::Uuid::new_v4(),
            worker_lease: golish_core::WorkerLeaseContext {
                worker_run_id: uuid::Uuid::new_v4(),
                stage_run_unit_id: uuid::Uuid::new_v4(),
                lease_token: uuid::Uuid::new_v4(),
                attempt_epoch: 3,
            },
            candidate_attempt: None,
            candidate_submit_only: false,
            return_on_first_durable_stage_submission: false,
            stage_team_leader: Some(StageTeamLeaderBinding {
                stage_team_plan_id: uuid::Uuid::new_v4(),
                leader_work_item_id: uuid::Uuid::new_v4(),
                expected_dispatch_epoch: 2,
                expected_plan_row_version: 4,
                expected_work_item_row_version: 5,
                controller_action_compiler: None,
                compiled_actions: Vec::new(),
                planning_only: false,
            }),
            target_intel_review: None,
            stage_team_output_schema: None,
            terminal_execution: None,
            chain_id: uuid::Uuid::new_v4(),
            session_id: uuid::Uuid::new_v4(),
            agent_type: "recon".to_string(),
            runtime_memory_source: None,
            initial_chain: serde_json::json!([]),
            initial_prompt_already_checkpointed: false,
            reset_provider_history: false,
            checkpoint_version: Arc::new(AtomicI64::new(7)),
            checkpoint_body: Arc::new(std::sync::RwLock::new(serde_json::json!([]))),
            lease_lost: Arc::new(AtomicBool::new(false)),
            mutation_lock: Arc::new(tokio::sync::Mutex::new(())),
            tool_lifecycle: None,
        }
    }

    fn leader_tool_context(
        bound: &BoundWorkerChainContext,
        request_id: &str,
        tool_name: &str,
    ) -> golish_core::AgentToolContext {
        golish_core::AgentToolContext {
            request_id: request_id.to_string(),
            tool_call_record_id: Some(uuid::Uuid::new_v4()),
            tool_name: tool_name.to_string(),
            source: golish_core::events::ToolSource::SubAgent {
                agent_id: "recon".to_string(),
                agent_name: "Recon".to_string(),
            },
            operation_id: Some(bound.operation_id),
            stage_execution_id: Some(bound.stage_execution_id),
            stage_run_unit_id: Some(bound.worker_lease.stage_run_unit_id),
            organization_id: Some(bound.organization_id),
            worker_lease: Some(bound.worker_lease.clone()),
            candidate_attempt: None,
        }
    }

    #[tokio::test]
    async fn stage_team_host_tools_fail_closed_without_exact_leader_binding() {
        let bound = stage_team_leader_bound();
        let mut ordinary = bound.clone();
        ordinary.stage_team_leader = None;
        let context = leader_tool_context(
            &ordinary,
            "call-ordinary",
            "stage_team_prepare_final_submission",
        );

        let (value, success) = route_stage_team_leader_host_tool(
            "stage_team_prepare_final_submission",
            &serde_json::json!({}),
            None,
            Some(&ordinary),
            Some(&context),
        )
        .await
        .expect("reserved host tool must be recognized, not fall through");

        assert!(!success);
        assert_eq!(value["code"], "STAGE_TEAM_LEADER_BINDING_REQUIRED");
    }

    #[tokio::test]
    async fn stage_team_update_plan_is_reserved_for_a_bound_non_leader() {
        let mut ordinary = stage_team_leader_bound();
        ordinary.stage_team_leader = None;
        let context = leader_tool_context(&ordinary, "call-ordinary-plan", "update_plan");

        let (value, success) = route_stage_team_leader_host_tool(
            "update_plan",
            &serde_json::json!({
                "plan": [{"step":"Inspect current evidence","status":"in_progress"}]
            }),
            None,
            Some(&ordinary),
            Some(&context),
        )
        .await
        .expect("update_plan must be reserved for bound Stage Team workers");

        assert!(!success);
        assert_eq!(value["code"], "STAGE_TEAM_LEADER_BINDING_REQUIRED");
    }

    #[tokio::test]
    async fn unbound_update_plan_keeps_the_existing_generic_router() {
        assert!(route_stage_team_leader_host_tool(
            "update_plan",
            &serde_json::json!({
                "plan": [{"step":"Top-level work","status":"in_progress"}]
            }),
            None,
            None,
            None,
        )
        .await
        .is_none());
    }

    #[tokio::test]
    async fn stage_team_update_plan_returns_a_chain_local_normalized_plan() {
        let bound = stage_team_leader_bound();
        let context = leader_tool_context(&bound, "call-lead-plan", "update_plan");

        let (value, success) = route_stage_team_leader_host_tool(
            "update_plan",
            &serde_json::json!({
                "explanation": "  Cover the company scope  ",
                "plan": [
                    {"step":"  Inspect current evidence  ","status":"completed"},
                    {"step":"Delegate missing coverage","status":"in_progress"},
                    {"step":"Review and submit Gate","status":"pending"}
                ]
            }),
            None,
            Some(&bound),
            Some(&context),
        )
        .await
        .expect("bound Company Controller update_plan");

        assert!(success);
        assert_eq!(value["success"], true);
        assert_eq!(value["explanation"], "Cover the company scope");
        assert_eq!(value["plan_version"], 8);
        assert_eq!(value["plan_version_scope"], "bound_chain_checkpoint_hint");
        assert_eq!(value["summary"]["total"], 3);
        assert_eq!(value["summary"]["completed"], 1);
        assert_eq!(value["summary"]["in_progress"], 1);
        assert_eq!(value["summary"]["pending"], 1);
        let plan = value["plan"].as_array().expect("normalized plan steps");
        assert_eq!(plan.len(), 3);
        assert_eq!(plan[0]["step"], "Inspect current evidence");
        assert_eq!(plan[1]["status"], "in_progress");
        assert!(plan.iter().all(|step| step["id"].is_string()));
    }

    #[tokio::test]
    async fn stage_team_update_plan_enforces_strict_status_and_plan_invariants() {
        let bound = stage_team_leader_bound();
        let context = leader_tool_context(&bound, "call-lead-plan-invalid", "update_plan");

        for (args, expected_code) in [
            (
                serde_json::json!({"plan":[{"step":"Missing status"}]}),
                "STAGE_TEAM_UPDATE_PLAN_STATUS_INVALID",
            ),
            (
                serde_json::json!({"plan":[{"step":"Cancelled","status":"cancelled"}]}),
                "STAGE_TEAM_UPDATE_PLAN_STATUS_INVALID",
            ),
            (
                serde_json::json!({"plan":[]}),
                "STAGE_TEAM_UPDATE_PLAN_INVALID",
            ),
            (
                serde_json::json!({
                    "plan":[
                        {"step":"One","status":"in_progress"},
                        {"step":"Two","status":"in_progress"}
                    ]
                }),
                "STAGE_TEAM_UPDATE_PLAN_INVALID",
            ),
        ] {
            let (value, success) = route_stage_team_leader_host_tool(
                "update_plan",
                &args,
                None,
                Some(&bound),
                Some(&context),
            )
            .await
            .expect("bound update_plan is reserved");
            assert!(!success, "invalid plan unexpectedly succeeded: {args}");
            assert_eq!(value["code"], expected_code, "invalid plan: {args}");
        }

        let too_many_steps = (0..13)
            .map(|index| {
                serde_json::json!({
                    "step": format!("Step {index}"),
                    "status": "pending"
                })
            })
            .collect::<Vec<_>>();
        let (value, success) = route_stage_team_leader_host_tool(
            "update_plan",
            &serde_json::json!({"plan": too_many_steps}),
            None,
            Some(&bound),
            Some(&context),
        )
        .await
        .expect("bound update_plan is reserved");
        assert!(!success);
        assert_eq!(value["code"], "STAGE_TEAM_UPDATE_PLAN_INVALID");
    }

    #[tokio::test]
    async fn stage_team_dispatch_workers_persists_fenced_requests_with_tool_request_envelope() {
        let repository = Arc::new(RecordingStageTeamRuntime::default());
        let repository_port: Arc<dyn RuntimeMemoryRepository> = repository.clone();
        let bound = stage_team_leader_bound();
        let context = leader_tool_context(
            &bound,
            "call-lead-dispatch-17",
            "stage_team_dispatch_workers",
        );
        let subject_id = uuid::Uuid::new_v4();

        let (value, success) = route_stage_team_leader_host_tool(
            "stage_team_dispatch_workers",
            &serde_json::json!({
                "workers": [{
                    "dedupe_key": "dns-and-ct",
                    "role": "intel_provider",
                    "kind": "provider_followup",
                    "objective": "Collect DNS and CT evidence for the canonical target",
                    "subject_refs": [{"kind":"target","target_id":subject_id}]
                }]
            }),
            Some(&repository_port),
            Some(&bound),
            Some(&context),
        )
        .await
        .expect("reserved host tool");

        assert!(success);
        assert_eq!(value["status"], "dispatch_accepted");
        assert_eq!(value["request_count"], 1);
        let requests = repository.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        let leader = bound.stage_team_leader.as_ref().unwrap();
        assert_eq!(request.stage_team_plan_id, leader.stage_team_plan_id);
        assert_eq!(request.parent_work_item_id, leader.leader_work_item_id);
        assert_eq!(
            request.expected_dispatch_epoch,
            leader.expected_dispatch_epoch
        );
        assert_eq!(request.fence.operation_id, bound.operation_id);
        assert_eq!(request.fence.stage_execution_id, bound.stage_execution_id);
        assert_eq!(
            request.fence.worker_run_id,
            bound.worker_lease.worker_run_id
        );
        assert_eq!(request.fence.expected_checkpoint_version, 7);
        assert_eq!(
            request.output_schema,
            serde_json::json!("stage_worker_output.v1")
        );
        assert_eq!(request.budget_hint, serde_json::json!({}));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&request.reason).unwrap(),
            serde_json::json!({
                "schema": "stage_team_controller_request.v1",
                "parent_tool_request_id": "call-lead-dispatch-17",
                "objective": "Collect DNS and CT evidence for the canonical target"
            })
        );
    }

    #[tokio::test]
    async fn enumeration_controller_selects_only_host_compiled_actions() {
        let repository = Arc::new(RecordingStageTeamRuntime::default());
        let repository_port: Arc<dyn RuntimeMemoryRepository> = repository.clone();
        let mut bound = stage_team_leader_bound();
        let target_id = uuid::Uuid::new_v4();
        let trusted_objective = serde_json::json!({
            "assignment_schema": "enumeration_formulaic_shard.v2",
            "exact_origin": "https://example.test:443/",
            "producer": "js_api",
            "target_id": target_id,
            "tool": "js_extract_apis",
            "tool_args": {"target_urls":[{"target_id":target_id,"target_url":"https://example.test:443/"}],"ai":false}
        })
        .to_string();
        let leader = bound.stage_team_leader.as_mut().unwrap();
        leader.controller_action_compiler = Some("enumeration_v2".to_string());
        leader.compiled_actions = vec![StageTeamCompiledActionBinding {
            action_id: "enum-action-1".to_string(),
            dedupe_key: "enumeration:js-api:trusted".to_string(),
            requested_role: "js_api_analyzer".to_string(),
            requested_kind: "formulaic_enumeration".to_string(),
            objective: trusted_objective.clone(),
            subject_refs: vec![serde_json::json!({"kind":"target","target_id":target_id})],
            budget_hint: serde_json::json!({"max_wrapper_calls":1}),
        }];
        let context = leader_tool_context(
            &bound,
            "call-enumeration-action",
            "stage_team_dispatch_workers",
        );

        let (value, success) = route_stage_team_leader_host_tool(
            "stage_team_dispatch_workers",
            &serde_json::json!({
                "workers": [{
                    "action_id": "enum-action-1",
                    "rationale": "Browser receipt is terminal, so analyze the captured scripts next"
                }]
            }),
            Some(&repository_port),
            Some(&bound),
            Some(&context),
        )
        .await
        .expect("reserved host tool");

        assert!(success, "host-compiled action was rejected: {value}");
        {
            let requests = repository.requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            let request = &requests[0];
            assert_eq!(request.requested_role, "js_api_analyzer");
            assert_eq!(request.requested_kind, "formulaic_enumeration");
            assert_eq!(request.dedupe_key, "enumeration:js-api:trusted");
            assert_eq!(
                request.budget_hint,
                serde_json::json!({"max_wrapper_calls":1})
            );
            assert_eq!(
                request.subject_refs,
                vec![serde_json::json!({"kind":"target","target_id":target_id})]
            );
            let reason: serde_json::Value = serde_json::from_str(&request.reason).unwrap();
            assert_eq!(reason["objective"], trusted_objective);
        }

        let (value, success) = route_stage_team_leader_host_tool(
            "stage_team_dispatch_workers",
            &serde_json::json!({
                "workers": [{
                    "action_id": "enum-action-1",
                    "rationale": "try to inject authority",
                    "role": "company_stage_controller"
                }]
            }),
            Some(&repository_port),
            Some(&bound),
            Some(&context),
        )
        .await
        .expect("reserved host tool");
        assert!(!success);
        assert_eq!(value["code"], "STAGE_TEAM_DISPATCH_ARGS_INVALID");
        assert_eq!(repository.requests.lock().unwrap().len(), 1);

        let mut refreshed_bound = bound.clone();
        refreshed_bound
            .stage_team_leader
            .as_mut()
            .unwrap()
            .compiled_actions
            .clear();
        let refreshed_context = leader_tool_context(
            &refreshed_bound,
            "call-enumeration-stale-action",
            "stage_team_dispatch_workers",
        );
        let (value, success) = route_stage_team_leader_host_tool(
            "stage_team_dispatch_workers",
            &serde_json::json!({
                "workers": [{
                    "action_id": "enum-action-1",
                    "rationale": "replay a stale action after durable truth changed"
                }]
            }),
            Some(&repository_port),
            Some(&refreshed_bound),
            Some(&refreshed_context),
        )
        .await
        .expect("reserved host tool");
        assert!(!success);
        assert_eq!(value["code"], "ENUMERATION_CONTROLLER_ACTION_NOT_READY");
        assert_eq!(repository.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn stage_team_dispatch_canonicalizes_and_deduplicates_target_shorthand() {
        let repository = Arc::new(RecordingStageTeamRuntime::default());
        let repository_port: Arc<dyn RuntimeMemoryRepository> = repository.clone();
        let bound = stage_team_leader_bound();
        let context = leader_tool_context(
            &bound,
            "call-lead-dispatch-target-shorthand",
            "stage_team_dispatch_workers",
        );
        let subject_id = uuid::Uuid::new_v4();

        let (_value, success) = route_stage_team_leader_host_tool(
            "stage_team_dispatch_workers",
            &serde_json::json!({
                "workers": [{
                    "dedupe_key": "two-origins-one-target",
                    "role": "enumerator",
                    "kind": "content_enumeration",
                    "objective": "Enumerate two exact web origins for one canonical target",
                    "subject_refs": [
                        {"target_id":subject_id,"target_url":"https://example.test"},
                        {"target_id":subject_id,"target_url":"https://www.example.test"}
                    ]
                }]
            }),
            Some(&repository_port),
            Some(&bound),
            Some(&context),
        )
        .await
        .expect("reserved host tool");

        assert!(success);
        let requests = repository.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].subject_refs,
            serde_json::json!([{"kind":"target","target_id":subject_id}])
                .as_array()
                .unwrap()
                .clone()
        );
    }

    #[tokio::test]
    async fn stage_team_dispatch_rejects_semantically_overlapping_assignments() {
        let repository = Arc::new(RecordingStageTeamRuntime::default());
        let repository_port: Arc<dyn RuntimeMemoryRepository> = repository.clone();
        let bound = stage_team_leader_bound();
        let context = leader_tool_context(
            &bound,
            "call-lead-dispatch-overlap",
            "stage_team_dispatch_workers",
        );

        let (value, success) = route_stage_team_leader_host_tool(
            "stage_team_dispatch_workers",
            &serde_json::json!({
                "workers": [
                    {
                        "dedupe_key": "remaining-assets-1",
                        "role": "vuln_scanner",
                        "kind": "vulnerability_triage",
                        "objective": "Process ALL remaining pending assets",
                        "subject_refs": []
                    },
                    {
                        "dedupe_key": "remaining-assets-2",
                        "role": "vuln_scanner",
                        "kind": "vulnerability_triage",
                        "objective": "Process  ALL  remaining pending assets",
                        "subject_refs": []
                    }
                ]
            }),
            Some(&repository_port),
            Some(&bound),
            Some(&context),
        )
        .await
        .expect("reserved host tool");

        assert!(!success);
        assert_eq!(value["code"], "STAGE_TEAM_DISPATCH_ASSIGNMENT_OVERLAP");
        assert_eq!(repository.requests.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn stage_team_dispatch_all_rejected_does_not_enter_waiting_barrier() {
        let repository = Arc::new(RecordingStageTeamRuntime {
            requests: Mutex::new(Vec::new()),
            reject_all: true,
            fail_on_request_number: None,
        });
        let repository_port: Arc<dyn RuntimeMemoryRepository> = repository.clone();
        let bound = stage_team_leader_bound();
        let context = leader_tool_context(
            &bound,
            "call-lead-dispatch-rejected",
            "stage_team_dispatch_workers",
        );

        let (value, success) = route_stage_team_leader_host_tool(
            "stage_team_dispatch_workers",
            &serde_json::json!({
                "workers": [{
                    "dedupe_key": "duplicate-work",
                    "role": "intel_provider",
                    "kind": "provider_followup",
                    "objective": "Retry a duplicate request",
                    "subject_refs": []
                }]
            }),
            Some(&repository_port),
            Some(&bound),
            Some(&context),
        )
        .await
        .expect("reserved host tool");

        assert!(!success);
        assert_eq!(value["code"], "STAGE_TEAM_DISPATCH_NONE_ACCEPTED");
        assert_eq!(value["status"], "dispatch_rejected");
        assert_eq!(value["accepted_count"], 0);
        assert_eq!(value["rejected_count"], 1);
        assert_eq!(value["request_count"], 1);
        assert_eq!(value["requests"][0]["decision"], "rejected");
        assert!(value["next_action"]
            .as_str()
            .is_some_and(|next_action| next_action.contains("canonical")));
        assert_eq!(repository.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn stage_team_dispatch_persist_failure_returns_terminal_assignment_details() {
        let repository = Arc::new(RecordingStageTeamRuntime {
            requests: Mutex::new(Vec::new()),
            reject_all: false,
            fail_on_request_number: Some(1),
        });
        let repository_port: Arc<dyn RuntimeMemoryRepository> = repository.clone();
        let bound = stage_team_leader_bound();
        let context = leader_tool_context(
            &bound,
            "call-lead-dispatch-persist-failed",
            "stage_team_dispatch_workers",
        );

        let (value, success) = route_stage_team_leader_host_tool(
            "stage_team_dispatch_workers",
            &serde_json::json!({
                "workers": [{
                    "dedupe_key": "retry-five-origins",
                    "role": "prober",
                    "kind": "surface_probe",
                    "objective": "Retry five exact origins",
                    "subject_refs": []
                }]
            }),
            Some(&repository_port),
            Some(&bound),
            Some(&context),
        )
        .await
        .expect("reserved host tool");

        assert!(!success);
        assert_eq!(value["code"], "STAGE_TEAM_DISPATCH_PERSIST_FAILED");
        assert_eq!(value["status"], "dispatch_failed");
        assert_eq!(value["accepted_count"], 0);
        assert_eq!(value["request_count"], 1);
        assert_eq!(value["requests"], serde_json::json!([]));
        assert_eq!(value["tool_request_id"], context.request_id);
        assert_eq!(repository.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn stage_team_dispatch_partial_persist_enters_barrier_for_accepted_children() {
        let repository = Arc::new(RecordingStageTeamRuntime {
            requests: Mutex::new(Vec::new()),
            reject_all: false,
            fail_on_request_number: Some(2),
        });
        let repository_port: Arc<dyn RuntimeMemoryRepository> = repository.clone();
        let bound = stage_team_leader_bound();
        let context = leader_tool_context(
            &bound,
            "call-lead-dispatch-partial",
            "stage_team_dispatch_workers",
        );

        let (value, success) = route_stage_team_leader_host_tool(
            "stage_team_dispatch_workers",
            &serde_json::json!({
                "workers": [
                    {
                        "dedupe_key": "dns-first",
                        "role": "intel_provider",
                        "kind": "provider_followup",
                        "objective": "Collect DNS evidence",
                        "subject_refs": []
                    },
                    {
                        "dedupe_key": "ct-second",
                        "role": "intel_provider",
                        "kind": "provider_followup",
                        "objective": "Collect CT evidence",
                        "subject_refs": []
                    }
                ]
            }),
            Some(&repository_port),
            Some(&bound),
            Some(&context),
        )
        .await
        .expect("reserved host tool");

        assert!(
            success,
            "one durable child already exists and must be drained"
        );
        assert_eq!(value["status"], "dispatch_accepted");
        assert_eq!(value["accepted_count"], 1);
        assert_eq!(value["request_count"], 1);
        assert!(value["partial_persist_error"].is_string());
        assert_eq!(value["requests"].as_array().unwrap().len(), 1);
        assert_eq!(repository.requests.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn stage_team_prepare_final_is_control_only_and_rejects_mismatched_tool_context() {
        let repository = Arc::new(RecordingStageTeamRuntime::default());
        let repository_port: Arc<dyn RuntimeMemoryRepository> = repository.clone();
        let bound = stage_team_leader_bound();
        let context = leader_tool_context(
            &bound,
            "call-lead-final",
            "stage_team_prepare_final_submission",
        );

        let (value, success) = route_stage_team_leader_host_tool(
            "stage_team_prepare_final_submission",
            &serde_json::json!({}),
            Some(&repository_port),
            Some(&bound),
            Some(&context),
        )
        .await
        .expect("reserved host tool");
        assert!(success);
        assert_eq!(value["status"], "prepare_final");
        assert_eq!(value["request_epoch_closed"], true);
        assert!(repository.requests.lock().unwrap().is_empty());

        let mut mismatched = context;
        mismatched.worker_lease.as_mut().unwrap().lease_token = uuid::Uuid::new_v4();
        let (value, success) = route_stage_team_leader_host_tool(
            "stage_team_prepare_final_submission",
            &serde_json::json!({}),
            Some(&repository_port),
            Some(&bound),
            Some(&mismatched),
        )
        .await
        .expect("reserved host tool");
        assert!(!success);
        assert_eq!(value["code"], "STAGE_TEAM_LEADER_TOOL_CONTEXT_MISMATCH");
    }

    #[test]
    fn enumeration_producer_result_normalizes_url_to_exact_origin() {
        let target_id = uuid::Uuid::new_v4();
        let result = serde_json::json!({
            "target_id": target_id,
            "effective_target_url": "HTTP://127.0.0.1:53226/captured/page?query=1#fragment",
        });

        let (actual_target_id, exact_origin) =
            enumeration_result_target_and_origin(&result).expect("canonical producer origin");

        assert_eq!(actual_target_id, target_id);
        assert_eq!(exact_origin, "http://127.0.0.1:53226");
    }

    #[test]
    fn vuln_scanner_hides_finding_writer_for_every_runtime_contract() {
        assert!(vuln_triage_hides_record_finding(
            Some(StageKind::VulnTriage),
            "record_finding",
        ));
        assert!(!vuln_triage_hides_record_finding(
            Some(StageKind::Enumeration),
            "record_finding",
        ));
        for tool in [
            "vuln_nuclei_general",
            "vuln_nuclei_fingerprint_targeted",
            "vuln_probe_anonymous_access",
        ] {
            assert!(!vuln_triage_hides_record_finding(
                Some(StageKind::VulnTriage),
                tool,
            ));
        }
        assert!(sub_agent_stage_tool_hidden(
            "record_finding",
            false,
            true,
            false,
        ));
        assert!(!sub_agent_stage_tool_hidden(
            "record_finding",
            false,
            false,
            false,
        ));
        assert!(sub_agent_stage_tool_hidden(
            "pentest_run",
            true,
            false,
            false,
        ));
    }

    #[test]
    fn investigation_nested_workers_are_cognition_only() {
        for allowed in [
            "update_plan",
            "query_target_data",
            "search_knowledge_base",
            "sub_agent_researcher",
            "submit_result",
            "submit_stage_deliverable",
        ] {
            assert!(unified_investigation_cognitive_tool_allowed(allowed));
            assert!(!sub_agent_stage_tool_hidden(allowed, false, false, true));
        }
        for forbidden in [
            "web_fetch",
            "browser_navigate",
            "pentest_run",
            "run_pty_cmd",
            "vault",
            "record_finding",
            "write_file",
        ] {
            assert!(!unified_investigation_cognitive_tool_allowed(forbidden));
            assert!(sub_agent_stage_tool_hidden(forbidden, false, false, true));
        }
    }

    #[test]
    fn investigation_synthesis_attempt_uses_narrow_reducer_identity() {
        assert!(unified_investigation_reducer_request(
            Some(StageKind::Investigation),
            "request::synthesis-attempt:0"
        ));
        assert!(!unified_investigation_reducer_request(
            Some(StageKind::Investigation),
            "request::worker:0"
        ));
        assert!(!unified_investigation_reducer_request(
            Some(StageKind::Enumeration),
            "request::synthesis-attempt:0"
        ));
    }

    #[test]
    fn production_target_intel_actor_surfaces_are_role_closed() {
        for allowed in [
            "update_plan",
            "recon_search_intel",
            STAGE_TEAM_DISPATCH_WORKERS_TOOL_NAME,
            STAGE_TEAM_PREPARE_FINAL_SUBMISSION_TOOL_NAME,
        ] {
            assert!(production_target_intel_sub_agent_tool_allowed(
                allowed, true, false, false
            ));
        }
        for forbidden in [
            "recon_list_providers",
            "recon_map_assets",
            "recon_lookup_whois",
            "query_target_data",
            "check_stage_asset_coverage",
            "stage_worklist_status",
            "stage_worklist_next",
        ] {
            assert!(
                !production_target_intel_sub_agent_tool_allowed(forbidden, true, false, false),
                "Company Controller must not recover retired tool {forbidden}"
            );
            assert!(
                !production_target_intel_sub_agent_tool_allowed(forbidden, false, false, false),
                "generic Intel worker must not recover retired tool {forbidden}"
            );
        }
        assert!(!production_target_intel_sub_agent_tool_allowed(
            "submit_result",
            true,
            false,
            false,
        ));

        assert!(production_target_intel_sub_agent_tool_allowed(
            "recon_search_intel",
            false,
            false,
            false,
        ));
        assert!(production_target_intel_sub_agent_tool_allowed(
            "list_recent_evidence",
            false,
            false,
            false
        ));
        assert!(production_target_intel_sub_agent_tool_allowed(
            "submit_result",
            false,
            false,
            false
        ));
        assert!(production_target_intel_sub_agent_tool_allowed(
            "submit_stage_deliverable",
            false,
            true,
            false
        ));
        assert!(!production_target_intel_sub_agent_tool_allowed(
            "recon_search_intel",
            false,
            true,
            false
        ));
        for reviewer_tool in [
            golish_sub_agents::TARGET_INTEL_READ_REVIEW_SECTION,
            golish_sub_agents::TARGET_INTEL_RECORD_REVIEW_VERDICT,
            "submit_result",
        ] {
            assert!(production_target_intel_sub_agent_tool_allowed(
                reviewer_tool,
                false,
                false,
                true,
            ));
        }
        assert!(!production_target_intel_sub_agent_tool_allowed(
            "recon_search_intel",
            false,
            false,
            true,
        ));
    }

    #[test]
    fn unsuccessful_sub_agent_result_is_tracked_as_failed_dispatch() {
        assert_eq!(
            dispatch_status_for_sub_agent_success(false),
            golish_agent_kit::db_traits::DispatchStatus::Failed
        );
        assert_eq!(
            dispatch_status_for_sub_agent_success(true),
            golish_agent_kit::db_traits::DispatchStatus::Completed
        );
    }

    #[test]
    fn chain_errors_map_to_stable_runtime_failure_contract() {
        let chain_id = uuid::Uuid::new_v4();
        let cases = [
            (
                golish_sub_agents::SubAgentChainError::ExactResumeUnavailable {
                    chain_id,
                    reason: "load failed".to_string(),
                },
                "sub_agent_chain_exact_resume_unavailable",
                "restore_exact",
            ),
            (
                golish_sub_agents::SubAgentChainError::LatestResumeUnavailable {
                    agent_id: "enumerator".to_string(),
                    reason: "not found".to_string(),
                },
                "sub_agent_chain_latest_resume_unavailable",
                "restore_latest",
            ),
            (
                golish_sub_agents::SubAgentChainError::CreateFreshFailed {
                    agent_id: "enumerator".to_string(),
                    reason: "insert failed".to_string(),
                },
                "sub_agent_chain_create_fresh_failed",
                "create_fresh",
            ),
            (
                golish_sub_agents::SubAgentChainError::FinalizeFailed {
                    chain_id,
                    checkpointed_chain_id: None,
                    reason: "update failed".to_string(),
                },
                "sub_agent_chain_finalize_failed",
                "finalize",
            ),
            (
                golish_sub_agents::SubAgentChainError::ProviderContextLimitExceeded {
                    chain_id: Some(chain_id),
                    reason: "Request body has 1325879 weighted tokens; limit is 1048565"
                        .to_string(),
                },
                "sub_agent_provider_context_limit_exceeded",
                "context_limit",
            ),
        ];

        for (error, expected_code, expected_kind) in cases {
            let result = sub_agent_execution_error_result(anyhow::Error::new(error));
            assert!(!result.success);
            assert_eq!(result.value["error_code"], expected_code);
            assert_eq!(result.value["chain_failure_kind"], expected_kind);
            assert!(result.value["error"]
                .as_str()
                .is_some_and(|s| !s.is_empty()));
        }

        let generic = sub_agent_execution_error_result(anyhow::anyhow!("ordinary failure"));
        assert_eq!(generic.value["error"], "ordinary failure");
        assert!(generic.value.get("error_code").is_none());
        assert!(generic.value.get("chain_failure_kind").is_none());
    }

    #[test]
    fn sub_agent_chain_provider_context_limit_error_preserves_checkpointed_chain_id() {
        let chain_id = uuid::Uuid::new_v4();
        let result = sub_agent_execution_error_result(anyhow::Error::new(
            golish_sub_agents::SubAgentChainError::ProviderContextLimitExceeded {
                chain_id: Some(chain_id),
                reason: "Request body exceeds the model context limit".to_string(),
            },
        ));

        assert!(!result.success);
        assert_eq!(result.value["chain_id"], chain_id.to_string());
    }

    #[test]
    fn sub_agent_chain_finalize_error_publishes_only_previous_checkpoint_id() {
        let checkpoint_id = uuid::Uuid::new_v4();
        let failed_update_id = uuid::Uuid::new_v4();
        let result = sub_agent_execution_error_result(anyhow::Error::new(
            golish_sub_agents::SubAgentChainError::FinalizeFailed {
                chain_id: failed_update_id,
                checkpointed_chain_id: Some(checkpoint_id),
                reason: "synthetic later update failure".to_string(),
            },
        ));

        assert!(!result.success);
        assert_eq!(result.value["chain_failure_kind"], "finalize");
        assert_eq!(result.value["chain_id"], checkpoint_id.to_string());
        assert_ne!(result.value["chain_id"], failed_update_id.to_string());
    }

    #[test]
    fn sub_agent_chain_failed_result_preserves_checkpointed_chain_id() {
        let chain_id = uuid::Uuid::new_v4();
        let result = SubAgentResult {
            agent_id: "enumerator".to_string(),
            response: "provider failed after the initial snapshot".to_string(),
            context: SubAgentContext::default(),
            success: false,
            duration_ms: 42,
            files_modified: Vec::new(),
            chain_id: Some(chain_id),
        };

        let tool_result = sub_agent_tool_execution_result(result);

        assert!(!tool_result.success);
        assert_eq!(tool_result.value["chain_id"], chain_id.to_string());
    }

    #[test]
    fn stage_run_org_id_parses_per_org_request_id() {
        let id = "fb90ef2a-eb1c-4288-8f7c-97dc957a26c0";
        let request_id = format!("call_00_ZRDP0qOpYOpCbInFkBHS5518::org::{id}");
        assert_eq!(
            stage_run_org_id_from_request_id(&request_id).map(|u| u.to_string()),
            Some(id.to_string())
        );
    }

    #[test]
    fn stage_run_org_id_ignores_plain_sub_agent_request_id() {
        assert!(stage_run_org_id_from_request_id("call_00_plain").is_none());
        assert!(stage_run_org_id_from_request_id("call_00::org::not-a-uuid").is_none());
    }

    #[test]
    fn enumeration_reducer_accepts_only_one_exact_subject() {
        let target_id = uuid::Uuid::new_v4();
        assert_eq!(
            enumeration_reducer_subject(&serde_json::json!({
                "target_id": target_id,
                "exact_origin": "https://example.test:443/"
            }))
            .unwrap(),
            (target_id, "https://example.test:443".to_string())
        );
        assert!(enumeration_reducer_subject(&serde_json::json!({
            "target_id": target_id,
            "exact_origin": "https://example.test:443/",
            "dependency_receipt_ids": [uuid::Uuid::new_v4()]
        }))
        .is_err());
    }

    #[test]
    fn enumeration_producer_preflight_canonicalizes_one_exact_subject() {
        let target_id = uuid::Uuid::new_v4();
        assert_eq!(
            enumeration_producer_args_target_and_origin(&serde_json::json!({
                "target_id": target_id,
                "target_url": "https://example.test/path?ignored=true"
            }))
            .unwrap(),
            (target_id, "https://example.test:443/".to_string())
        );
        assert!(
            enumeration_producer_args_target_and_origin(&serde_json::json!({
                "target_urls": [{"target_id": target_id, "target_url": "https://example.test/"}]
            }))
            .is_err()
        );
    }

    #[tokio::test]
    async fn enumeration_producer_partial_is_planning_feedback_not_attempt_exhaustion() {
        let value = serde_json::json!({
            "completion_state": "partial",
            "outcome_persisted": true,
            "checkpoint": {"remaining": 2}
        });
        let (settled, success) =
            settle_enumeration_producer_result(None, "browser_collect_js_api", value.clone(), true)
                .await;
        assert!(success);
        assert_eq!(settled, value);
    }

    #[tokio::test]
    async fn enumeration_producer_batch_fails_closed_before_receipt_commit() {
        let (value, success) = settle_enumeration_producer_result(
            None,
            "js_extract_apis",
            serde_json::json!({"results": []}),
            true,
        )
        .await;
        assert!(!success);
        assert_eq!(value["code"], "ENUMERATION_V2_SINGLE_EXACT_ORIGIN_REQUIRED");
    }

    #[test]
    fn worklist_checkpoint_observer_does_not_require_execution_monitor() {
        assert!(sub_agent_tool_observer_needed(false, true, true));
        assert!(sub_agent_tool_observer_needed(true, false, false));
        assert!(!sub_agent_tool_observer_needed(false, true, false));
        assert!(!sub_agent_tool_observer_needed(false, false, true));
    }

    #[test]
    fn checkpoint_agent_path_uses_stage_run_org_when_present() {
        let org_id = "fb90ef2a-eb1c-4288-8f7c-97dc957a26c0";
        let request_id = format!("call_00::org::{org_id}");

        assert_eq!(
            sub_agent_checkpoint_agent_path(
                Some(StageKind::ExternalAttackSurface),
                &request_id,
                "prober"
            ),
            format!("main>stage_run:external_attack_surface>org:{org_id}>prober")
        );
        assert_eq!(
            sub_agent_checkpoint_agent_path(
                Some(StageKind::ExternalAttackSurface),
                "plain",
                "prober"
            ),
            "main>prober"
        );
    }

    #[test]
    fn submit_repair_mode_restores_from_agent_run_checkpoint() {
        let mode = SubmitRepairMode {
            kind: SubmitRepairKind::EvidenceRefs,
            reason: "real ids are [101]".to_string(),
            missing_required_checks: vec!["http_probe".to_string()],
            coverage_gap_actions: Vec::new(),
            eas_web_repair_targets: None,
            allowed_tools_override: Vec::new(),
            forbidden_tools: Vec::new(),
            directive_message: None,
        };
        let checkpoint = AgentRunCheckpoint {
            schema_v: 1,
            operation_id: None,
            stage: Some(StageKind::ExternalAttackSurface.as_str().to_string()),
            stage_attempt_id: None,
            agent_path: "main>prober".to_string(),
            status: AgentRunStatus::RuntimeCorrectionQueued,
            llm_turn_index: None,
            message_chain_ref: None,
            pending_gate_correction: Some(mode.model_instruction()),
            pending_submit_only: true,
            submit_repair_mode: Some(serde_json::to_value(&mode).unwrap()),
            repair_directive: None,
            runtime_corrections: Vec::new(),
            background_job_ids: Vec::new(),
            evidence_watermark: None,
            last_tool: None,
            updated_at: chrono::Utc::now(),
        };

        let restored = submit_repair_mode_from_agent_run(&checkpoint).expect("mode restores");
        assert_eq!(restored.kind, SubmitRepairKind::EvidenceRefs);
        assert!(restored.block_result("pentest_run").unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("http_probe"));
    }

    #[test]
    fn submit_repair_mode_restores_from_stage_retry_checkpoint() {
        let mode = SubmitRepairMode {
            kind: SubmitRepairKind::CoverageGap,
            reason: "coverage cell missing".to_string(),
            missing_required_checks: Vec::new(),
            coverage_gap_actions: vec![golish_sub_agents::CoverageGapAction {
                asset: "example.com".to_string(),
                technique: "GOLISH-EAS-LIVENESS".to_string(),
                reason: "missing_terminal_coverage".to_string(),
                suggested_capabilities: Vec::new(),
                suggested_tools: vec!["httpx".to_string()],
            }],
            eas_web_repair_targets: None,
            allowed_tools_override: Vec::new(),
            forbidden_tools: Vec::new(),
            directive_message: None,
        };
        let checkpoint = AgentRunCheckpoint {
            schema_v: 1,
            operation_id: None,
            stage: Some(StageKind::ExternalAttackSurface.as_str().to_string()),
            stage_attempt_id: None,
            agent_path: "main>stage_run:external_attack_surface>org:abc>prober".to_string(),
            status: AgentRunStatus::GateBlocked,
            llm_turn_index: Some(1),
            message_chain_ref: None,
            pending_gate_correction: Some("retry 2/3: close coverage gap".to_string()),
            pending_submit_only: false,
            submit_repair_mode: Some(serde_json::to_value(&mode).unwrap()),
            repair_directive: None,
            runtime_corrections: Vec::new(),
            background_job_ids: Vec::new(),
            evidence_watermark: None,
            last_tool: None,
            updated_at: chrono::Utc::now(),
        };

        let restored = submit_repair_mode_from_agent_run(&checkpoint).expect("mode restores");
        assert_eq!(restored.kind, SubmitRepairKind::CoverageGap);
        assert_eq!(restored.coverage_gap_actions.len(), 1);
        assert!(
            restored.block_result("pentest_run").is_some(),
            "EAS coverage repair must keep raw pentest_run blocked after checkpoint restore"
        );
    }

    #[test]
    fn refreshed_eas_web_lock_checkpoint_round_trips_and_preserves_sibling_state() {
        let mode = SubmitRepairMode {
            kind: SubmitRepairKind::CoverageGap,
            reason: "WEB exact-origin coverage remains".to_string(),
            missing_required_checks: Vec::new(),
            coverage_gap_actions: vec![golish_sub_agents::CoverageGapAction {
                asset: "app.example.com".to_string(),
                technique: "GOLISH-EAS-WEB-FINGERPRINT".to_string(),
                reason: "missing_exact_origin".to_string(),
                suggested_capabilities: Vec::new(),
                suggested_tools: vec!["eas_fingerprint_web_stack".to_string()],
            }],
            eas_web_repair_targets: None,
            allowed_tools_override: Vec::new(),
            forbidden_tools: Vec::new(),
            directive_message: None,
        };
        let repair_directive = serde_json::json!({"sentinel": "keep-directive"});
        let checkpoint = AgentRunCheckpoint {
            schema_v: 1,
            operation_id: None,
            stage: Some(StageKind::ExternalAttackSurface.as_str().to_string()),
            stage_attempt_id: None,
            agent_path: "main>stage_run:external_attack_surface>org:abc>prober".to_string(),
            status: AgentRunStatus::RuntimeCorrectionQueued,
            llm_turn_index: Some(2),
            message_chain_ref: Some("chain-1".to_string()),
            pending_gate_correction: Some("close exact origins".to_string()),
            pending_submit_only: true,
            submit_repair_mode: Some(serde_json::to_value(&mode).unwrap()),
            repair_directive: Some(repair_directive.clone()),
            runtime_corrections: vec![RuntimeCorrectionCheckpoint {
                source: "stage_refiner".to_string(),
                kind: "submit_coverage_gap".to_string(),
                message: "keep correction".to_string(),
                job_ids: Vec::new(),
                evidence_ids: vec![42],
                submit_allowed: false,
            }],
            background_job_ids: vec!["job-1".to_string()],
            evidence_watermark: Some(42),
            last_tool: None,
            updated_at: chrono::DateTime::parse_from_rfc3339("2026-07-12T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        };
        let current = state_blob_with_agent_run(
            serde_json::json!({
                "graph_flow": {"next_node": "external_attack_surface"},
                "stage_run_workers": {"external_attack_surface": {"abc": {"chain_id": "chain-1"}}}
            }),
            &checkpoint,
        );
        let worklist = serde_json::json!({
            "ready_to_submit": false,
            "items": [{
                "asset": "app.example.com",
                "target_id": "target-app",
                "technique": "GOLISH-EAS-WEB-FINGERPRINT",
                "details": {"recommended_args": {"target_urls": [{
                    "target_id": "target-app",
                    "target_url": "https://app.example.com:443"
                }]}}
            }]
        });
        let updated_at = chrono::DateTime::parse_from_rfc3339("2026-07-12T00:01:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        let next = state_blob_with_refined_eas_web_repair_checkpoint(
            current,
            &checkpoint.agent_path,
            "call-worklist-1",
            "stage_worklist_next",
            &worklist,
            updated_at,
        )
        .expect("a successful DB-backed refresh updates the durable exact lock");
        let restored = agent_run_from_state_blob(&next).expect("agent checkpoint remains present");
        let restored_mode = submit_repair_mode_from_agent_run(&restored).expect("mode restores");

        assert_eq!(
            restored_mode.eas_web_repair_targets,
            Some(vec![golish_sub_agents::EasWebRepairTarget {
                target_id: "target-app".to_string(),
                target_url: "https://app.example.com:443".to_string(),
            }])
        );
        assert_eq!(restored.repair_directive, Some(repair_directive));
        assert_eq!(restored.runtime_corrections, checkpoint.runtime_corrections);
        assert_eq!(restored.background_job_ids, vec!["job-1"]);
        assert_eq!(restored.updated_at, updated_at);
        let last_tool = restored.last_tool.expect("refresh tool checkpoint");
        assert_eq!(last_tool.tool_call_id, "call-worklist-1");
        assert_eq!(last_tool.tool_name, "stage_worklist_next");
        assert_eq!(last_tool.state, ToolCheckpointState::Completed);
        assert_eq!(next["graph_flow"]["next_node"], "external_attack_surface");
        assert_eq!(
            next["stage_run_workers"]["external_attack_surface"]["abc"]["chain_id"],
            "chain-1"
        );
    }
}
