//! `execute_sub_agent_call` — handles sub-agent tool calls (tool names
//! starting with `sub_agent_`), branching between built-in execution and the
//! registry-driven dispatch path, with best-effort dispatch lifecycle
//! persistence.

use anyhow::Result;
use rig::completion::CompletionModel as RigCompletionModel;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;

use golish_agent_kit::db_traits::{
    RequestStageWorker, RuntimeMemoryRepository, RuntimeWorkerFence, StageWorkerRequestDecision,
};
use golish_agent_kit::harness::CanonicalFactKey;
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

    let mut parsed = match serde_json::from_value::<StageTeamDispatchWorkersArgs>(args.clone()) {
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
        let budget_hint = json!({});
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

fn sub_agent_stage_tool_hidden(
    tool_name: &str,
    hide_scan_tools: bool,
    deny_vuln_finding: bool,
) -> bool {
    (hide_scan_tools && golish_agent_kit::harness::is_scan_tool_name(tool_name))
        || (deny_vuln_finding && tool_name == "record_finding")
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
    bound_worker_chain: Option<BoundWorkerChainContext>,
) -> Result<ToolExecutionResult>
where
    M: RigCompletionModel + Sync,
{
    let agent_id = tool_name.strip_prefix("sub_agent_").unwrap_or("");

    let registry = ctx.sub_agent_registry.read().await;
    let agent_def = match registry.get(agent_id) {
        Some(def) => def.clone(),
        None => {
            return Ok(ToolExecutionResult {
                value: json!({ "error": format!("Sub-agent '{}' not found", agent_id) }),
                success: false,
            });
        }
    };
    drop(registry);

    let tool_provider = DefaultToolProvider::with_db_tracker(ctx.events.db_tracker);
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
                let project_path = project_path.clone();
                let session_id = session_id.clone();
                let harness_stage = harness_stage;
                let harness_operation_id = harness_operation_id;
                let runtime_memory = runtime_memory.clone();
                let stage_team_bound = stage_team_bound.clone();
                Box::pin(async move {
                    let tool_context = golish_core::current_agent_tool_context();
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
    let hide_tool_in_stage: Option<golish_sub_agents::StageToolHider> =
        (hide_scan_tools || deny_vuln_finding).then(|| {
            let hider: golish_sub_agents::StageToolHider =
                std::sync::Arc::new(move |name: &str| {
                    sub_agent_stage_tool_hidden(name, hide_scan_tools, deny_vuln_finding)
                });
            hider
        });

    let sub_tool_result_hook: Option<golish_sub_agents::SubAgentToolResultHook> =
        ctx.harness_stage.map(|stage| {
            let tracker = ctx.events.db_tracker.cloned();
            let session_id = ctx.events.session_id.map(str::to_string);
            let harness_operation_id = ctx.harness_operation_id;
            let stage_execution_id = ctx.stage_execution_id;
            let harness_org_id = effective_harness_org_id;
            let hook: golish_sub_agents::SubAgentToolResultHook = std::sync::Arc::new(
                move |tool_name: String,
                      tool_args: serde_json::Value,
                      mut result: serde_json::Value,
                      success: bool| {
                    let tracker = tracker.clone();
                    let session_id = session_id.clone();
                    Box::pin(async move {
                        let mut persisted_success = success;
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
                            success,
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
        agent_run_from_state_blob, dispatch_status_for_sub_agent_success,
        route_stage_team_leader_host_tool, stage_run_org_id_from_request_id,
        state_blob_with_refined_eas_web_repair_checkpoint, sub_agent_checkpoint_agent_path,
        sub_agent_execution_error_result, sub_agent_stage_tool_hidden,
        sub_agent_tool_execution_result, sub_agent_tool_observer_needed,
        submit_repair_mode_from_agent_run, vuln_triage_hides_record_finding,
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
    use golish_sub_agents::{BoundWorkerChainContext, StageTeamLeaderBinding};
    use std::sync::atomic::{AtomicBool, AtomicI64};
    use std::sync::{Arc, Mutex};

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
            }),
            chain_id: uuid::Uuid::new_v4(),
            session_id: uuid::Uuid::new_v4(),
            agent_type: "recon".to_string(),
            runtime_memory_source: None,
            initial_chain: serde_json::json!([]),
            initial_prompt_already_checkpointed: false,
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
        assert!(sub_agent_stage_tool_hidden("record_finding", false, true));
        assert!(!sub_agent_stage_tool_hidden("record_finding", false, false,));
        assert!(sub_agent_stage_tool_hidden("pentest_run", true, false));
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
