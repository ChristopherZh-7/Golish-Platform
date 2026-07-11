//! `execute_sub_agent_call` — handles sub-agent tool calls (tool names
//! starting with `sub_agent_`), branching between built-in execution and the
//! registry-driven dispatch path, with best-effort dispatch lifecycle
//! persistence.

use anyhow::Result;
use rig::completion::CompletionModel as RigCompletionModel;
use serde_json::{json, Value};

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
    execute_sub_agent, SubAgentChainError, SubAgentContext, SubAgentDefinition,
    SubAgentExecutorContext, SubAgentToolObservation, SubmitRepairMode,
};

use super::super::super::llm_helpers::runtime_supervisor_one_shot;
use super::super::super::sub_agent_dispatch::{
    build_sub_agent_briefing, execute_sub_agent_with_client,
};
use super::super::super::{AgenticLoopContext, ToolExecutionResult};
use golish_agent_kit::tool_executors::extract_and_upsert_entities;
use golish_agent_kit::tool_provider_impl::DefaultToolProvider;

fn sub_agent_runtime_agent_path(agent_id: &str) -> String {
    format!("main>{agent_id}")
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

fn build_sub_agent_tool_observer(
    ctx: &AgenticLoopContext<'_>,
    agent_id: &str,
    agent_def: &SubAgentDefinition,
    task_desc: &str,
    restored_submit_repair_mode: Option<SubmitRepairMode>,
) -> Option<golish_sub_agents::SubAgentToolObserver> {
    let monitor = ctx.execution_monitor.as_ref()?.clone();
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
                                observation.parent_request_id.clone(),
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
    let resume_arg: Option<String> = match tool_args.get("resume") {
        Some(serde_json::Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Some(serde_json::Value::Bool(true)) => Some("latest".to_string()),
        _ => None,
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
        let router: golish_sub_agents::SubAgentToolRouter =
            std::sync::Arc::new(move |name: String, args: serde_json::Value| {
                let graph = graph.clone();
                let tracker = tracker.clone();
                let project_path = project_path.clone();
                let session_id = session_id.clone();
                let harness_stage = harness_stage;
                let harness_operation_id = harness_operation_id;
                Box::pin(async move {
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
    let briefing = build_sub_agent_briefing(
        ctx.events.db_tracker,
        ctx.graph_backend.as_deref(),
        project_id_opt.as_deref(),
        agent_id,
        task_desc,
    )
    .await;

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
    let hide_tool_in_stage: Option<golish_sub_agents::StageToolHider> = ctx
        .harness_stage
        .and_then(|kind| golish_agent_kit::harness::load_embedded_stage_spec(kind).ok())
        .filter(|spec| spec.allowed_tool_types.is_empty())
        .map(|_| {
            let hider: golish_sub_agents::StageToolHider = std::sync::Arc::new(|name: &str| {
                golish_agent_kit::harness::is_scan_tool_name(name)
            });
            hider
        });

    let sub_tool_result_hook: Option<golish_sub_agents::SubAgentToolResultHook> =
        ctx.harness_stage.map(|stage| {
            let tracker = ctx.events.db_tracker.cloned();
            let session_id = ctx.events.session_id.map(str::to_string);
            let harness_org_id = effective_harness_org_id;
            let hook: golish_sub_agents::SubAgentToolResultHook = std::sync::Arc::new(
                move |tool_name: String,
                      _tool_args: serde_json::Value,
                      mut result: serde_json::Value,
                      success: bool| {
                    let tracker = tracker.clone();
                    let session_id = session_id.clone();
                    Box::pin(async move {
                        if let Some(id) = super::record_recon_passive_evidence(
                            tracker.as_ref(),
                            session_id.as_deref(),
                            Some(stage),
                            harness_org_id,
                            &tool_name,
                            &result,
                            success,
                        )
                        .await
                        {
                            if let Some(obj) = result.as_object_mut() {
                                obj.insert("_evidence_id".to_string(), json!(id));
                            }
                        }
                        (result, success)
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
        dispatch_status_for_sub_agent_success, stage_run_org_id_from_request_id,
        sub_agent_checkpoint_agent_path, sub_agent_execution_error_result,
        sub_agent_tool_execution_result, submit_repair_mode_from_agent_run,
    };
    use golish_agent_kit::harness::StageKind;
    use golish_agent_kit::task_orchestrator::agent_run_checkpoint::{
        AgentRunCheckpoint, AgentRunStatus,
    };
    use golish_sub_agents::{SubAgentContext, SubAgentResult, SubmitRepairKind, SubmitRepairMode};

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
}
