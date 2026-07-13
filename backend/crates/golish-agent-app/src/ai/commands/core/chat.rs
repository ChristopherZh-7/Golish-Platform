//! AI chat commands: send prompt (with attachments), clear conv, signal ready,
//! cancel, vision capabilities.

use crate::error::GolishError;
use std::sync::Arc;

use tauri::State;

use super::super::super::agent_bridge::AgentBridge;
use crate::state::AgentState;

/// Send a prompt to the AI agent for a specific session.
///
/// This is the session-specific version of send_ai_prompt that routes to
/// the correct agent bridge based on session_id.
///
/// Execution mode dispatch:
/// - **Chat**: normal agentic loop (conversational with tools)
/// - **Task**: PentAGI-style automated orchestration (Generator → Subtasks → Refiner → Reporter)
///
/// IMPORTANT: Uses get_session_bridge() to clone the Arc and release the map
/// lock immediately. This allows other sessions to initialize/shutdown while
/// this session is executing, enabling true concurrent multi-tab agent execution.
#[tauri::command]
pub async fn send_ai_prompt_session(
    state: State<'_, AgentState>,
    session_id: String,
    prompt: String,
) -> Result<String, GolishError> {
    tracing::info!(
        message = "[send_ai_prompt_session] Received prompt",
        session_id = %session_id,
        prompt_len = prompt.len(),
    );

    // Get Arc clone and release map lock immediately
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| {
            tracing::error!(
                message = "[send_ai_prompt_session] Session not initialized",
                session_id = %session_id,
            );
            super::super::ai_session_not_initialized_error(&session_id)
        })?;

    // Universal per-bridge boundary: acquire before reading the mode or entering
    // Chat/Task/profile/attachment execution. A busy contender cannot reset the
    // shared cancellation flag or touch history/harness side-channels.
    let request = bridge.begin_top_level_request().await.map_err(|error| {
        tracing::warn!(
            message = "[send_ai_prompt_session] Agent session is busy",
            session_id = %session_id,
            error = %error,
        );
        GolishError::Internal(error.to_string())
    })?;

    let result = execute_owned_prompt_request(
        bridge.clone(),
        &session_id,
        &prompt,
        state.inner(),
        request.clone(),
    )
    .await;

    if let Err(error) = bridge.clear_top_level_request_state(&request).await {
        tracing::error!(
            message = "[send_ai_prompt_session] Failed to clear request-local bridge state",
            session_id = %session_id,
            error = %error,
        );
        if result.is_ok() {
            return Err(GolishError::Internal(error.to_string()));
        }
    }

    result
}

async fn execute_owned_prompt_request(
    bridge: Arc<AgentBridge>,
    session_id: &str,
    prompt: &str,
    state: &AgentState,
    request: golish_agent_bridge::TopLevelRequestLease,
) -> Result<String, GolishError> {
    let mode = bridge.get_execution_mode().await;

    tracing::info!(
        message = "[send_ai_prompt_session] Got bridge, executing prompt",
        session_id = %session_id,
        execution_mode = %mode,
    );

    match mode {
        golish_agent_kit::execution_mode::ExecutionMode::Chat => {
            bridge.execute(prompt).await.map_err(|e| {
                tracing::error!(
                    message = "[send_ai_prompt_session] Chat execution error",
                    session_id = %session_id,
                    error = %e,
                );
                GolishError::Internal(e.to_string())
            })
        }
        golish_agent_kit::execution_mode::ExecutionMode::Task => {
            match should_resume_existing_task_operation(state, bridge.as_ref(), session_id, prompt)
                .await
            {
                Ok(true) => {
                    tracing::info!(
                        target: "harness::task_mode",
                        session_id = %session_id,
                        "resume-like prompt matched a checkpointed task; entering task harness directly"
                    );
                    return execute_task_mode(bridge, session_id, prompt, state, request.clone())
                        .await
                        .map_err(|e| {
                            let error = format!("{:#}", e);
                            tracing::error!(
                                message = "[send_ai_prompt_session] Task resume execution error",
                                session_id = %session_id,
                                error = %error,
                            );
                            GolishError::Internal(error)
                        });
                }
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(
                        target: "harness::task_mode",
                        session_id = %session_id,
                        error = %error,
                        "resume preflight failed; falling back to normal task/profile routing"
                    );
                }
            }

            let task_entry = explicit_task_prompt(prompt)
                .map(|task_prompt| ("Explicit task prefix detected", task_prompt))
                .or_else(|| {
                    implicit_operation_prompt(prompt)
                        .map(|task_prompt| ("Task/profile operation intent detected", task_prompt))
                });

            if let Some((entry_reason, task_prompt)) = task_entry {
                tracing::info!(
                    message = "[send_ai_prompt_session] entering task harness",
                    reason = entry_reason,
                    session_id = %session_id,
                );
                return execute_task_mode(bridge, session_id, &task_prompt, state, request.clone())
                    .await
                    .map_err(|e| {
                        let error = format!("{:#}", e);
                        tracing::error!(
                            message = "[send_ai_prompt_session] Task execution error",
                            session_id = %session_id,
                            error = %error,
                        );
                        GolishError::Internal(error)
                    });
            }

            // Profile/task selection is a behavior preset. A normal send runs a
            // flexible lead-agent turn with the usual coding/chat tools plus the
            // `start_operation` handoff tool. If the model calls that tool, we
            // enter the heavyweight Scoping→Reporting harness; otherwise the
            // lead turn's reply is the final answer.
            execute_task_profile_turn(bridge, session_id, prompt, state, request)
                .await
                .map_err(|e| {
                    let error = format!("{:#}", e);
                    tracing::error!(
                        message = "[send_ai_prompt_session] Task/profile execution error",
                        session_id = %session_id,
                        error = %error,
                    );
                    GolishError::Internal(error)
                })
        }
    }
}

async fn should_resume_existing_task_operation(
    state: &AgentState,
    bridge: &AgentBridge,
    session_id: &str,
    prompt: &str,
) -> anyhow::Result<bool> {
    let task_input = extract_user_message_from_wrapped_prompt(prompt);
    if !looks_like_resume_operation_prompt(task_input) {
        return Ok(false);
    }
    super::operation_resume::has_resumable_task_for_session(state, bridge, session_id, task_input)
        .await
}

/// Run Task mode orchestration (PentAGI-style).
///
/// Emits a short initial Started→TextDelta→Completed cycle so the frontend
/// immediately shows a response while the Generator LLM call runs.
/// Each subtask then manages its own Started/Completed lifecycle via
/// `execute_isolated` → `execute_with_context`.
async fn execute_task_mode(
    bridge: Arc<AgentBridge>,
    _session_id: &str,
    prompt: &str,
    state: &AgentState,
    request: golish_agent_bridge::TopLevelRequestLease,
) -> anyhow::Result<String> {
    execute_task_mode_with_continuity(
        bridge,
        _session_id,
        prompt,
        state,
        golish_agent_kit::harness::ContinuityDecision::AskBeforeReuse,
        request,
    )
    .await
}

async fn execute_task_mode_with_continuity(
    bridge: Arc<AgentBridge>,
    _session_id: &str,
    prompt: &str,
    state: &AgentState,
    continuity_decision: golish_agent_kit::harness::ContinuityDecision,
    request: golish_agent_bridge::TopLevelRequestLease,
) -> anyhow::Result<String> {
    use anyhow::Context;
    use golish_agent_bridge::bridge_executor::BridgeAgentExecutor;
    use golish_agent_kit::task_orchestrator::TaskOrchestrator;
    use golish_core::events::AiEvent;
    use golish_db::{models::NewSession, repo::sessions};

    let task_input = extract_user_message_from_wrapped_prompt(prompt);
    let continuity_decision =
        continuity_decision_from_prompt(task_input).unwrap_or(continuity_decision);

    // Anchor the chat-panel session string to one stable `sessions` row (Task
    // 断线恢复 · L1). Upserting by `chat_session_key` (instead of creating a row
    // per message) is what lets us find + resume this chat's prior operation
    // below; it also satisfies the `tasks.session_id` FK. Same chat session →
    // same DB session id on every message.
    let session_row = sessions::upsert_by_chat_key(
        &state.db_pool,
        _session_id,
        NewSession {
            title: Some(truncate_for_title(task_input, 80)),
            workspace_path: None,
            workspace_label: None,
            model: Some(bridge.model_name().to_string()),
            provider: Some(bridge.provider_name().to_string()),
            project_path: None,
        },
    )
    .await
    .context("Failed to upsert session row for task mode (FK precondition for tasks)")?;
    let uuid_session_id = session_row.id;
    tracing::info!(
        target: "harness::task_mode",
        session_db_id = %uuid_session_id,
        chat_session_id = %_session_id,
        "task mode session row created"
    );
    // The bridge is configured before the durable TaskMode session row exists, so
    // its DB tracker initially carries a random UUID. Rebind the shared tracker
    // identity now, before constructing/executing the stage runner, so every
    // ask_human lifecycle row is visible to the session-scoped Scoping gate.
    bridge.set_tracker_session_uuid(uuid_session_id);
    let executor = BridgeAgentExecutor::from_request(bridge.clone(), request.clone())
        .context("upgrade owned request into Task execution")?;

    let event_tx = bridge.get_or_create_event_tx();

    // Echo the user's task input into the event stream (chat-mode parity).
    // Progress feedback is surfaced by the orchestrator's TaskProgress events.
    bridge.emit_event(AiEvent::UserMessage {
        content: task_input.to_string(),
    });

    let start_time = std::time::Instant::now();
    let provider = std::sync::Arc::new(crate::ai::db_bridge::GolishDbRepoProvider::new(
        state.db_pool.clone(),
    ));
    let db_repo: std::sync::Arc<dyn golish_agent_kit::db_traits::DbRepoProvider> = provider.clone();
    let runtime_repo: std::sync::Arc<dyn golish_agent_kit::db_traits::RuntimeMemoryRepository> =
        provider;
    let db_repo_for_scope_authorization = db_repo.clone();
    let runtime_repo_for_scope_authorization = runtime_repo.clone();
    let profile_override = bridge.get_harness_profile().await;
    let profile_id = profile_override
        .clone()
        .unwrap_or_else(|| golish_agent_kit::harness::active_profile_id().to_string());
    let continuity_plan = match continuity_decision {
        golish_agent_kit::harness::ContinuityDecision::StartFresh => None,
        _ => match golish_agent_kit::task_orchestrator::build_existing_db_continuity_plan(
            &*db_repo,
            &profile_id,
            None,
        )
        .await
        {
            Ok(plan) => plan,
            Err(error) => {
                tracing::warn!(
                    target: "harness::task_mode",
                    %profile_id,
                    error = %error,
                    "continuity preflight failed; starting without adoption"
                );
                None
            }
        },
    };
    let mut orchestrator = TaskOrchestrator::new(db_repo, runtime_repo, uuid_session_id, event_tx);
    orchestrator.set_profile_override(profile_override.clone());
    // Scope evidence-ledger lookups to THIS chat session so gate repair
    // corrections can name the operation's real evidence ids (the string
    // `_session_id` is what both evidence write paths stamp on `audit_log`).
    orchestrator.set_chat_session_id(_session_id);
    // Wire the HITL coordinator so the two-level phase-approval gate can request a
    // clickable Confirm/Skip decision (the same `ask_human` channel) instead of
    // the legacy text channel, which has no production feeder and would otherwise
    // leave the run stuck at "Waiting for approval" with no way to approve.
    orchestrator.set_approval_coordinator(bridge.coordinator().cloned());

    // Resume-aware entry (Task 断线恢复 · L2): if this chat session has a
    // non-terminal operation with a persisted harness checkpoint, resume it from
    // where it left off instead of starting a new run at scoping. The decision is
    // purely state-driven — the user's text ("继续" / anything / empty) is NOT
    // parsed as a keyword. `None` (nothing resumable) → fresh run.
    let resumable =
        golish_db::repo::tasks::latest_resumable_by_session(&state.db_pool, uuid_session_id)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(
                    target: "harness::task_mode",
                    error = %e,
                    "resume lookup failed; starting a fresh run"
                );
                None
            });
    let mut continuity_adoption =
        if continuity_decision == golish_agent_kit::harness::ContinuityDecision::ReuseExisting {
            continuity_plan.clone()
        } else {
            None
        };
    if resumable.is_none()
        && continuity_decision == golish_agent_kit::harness::ContinuityDecision::AskBeforeReuse
    {
        if let Some(plan) = continuity_plan.as_ref() {
            match request_continuity_adoption_decision(&bridge, plan).await {
                ContinuityAdoptionDecision::ReuseExisting => {
                    continuity_adoption = Some(plan.clone());
                }
                ContinuityAdoptionDecision::StartFresh => {}
                ContinuityAdoptionDecision::TextFallback(message) => {
                    emit_immediate_task_response(&bridge, &message, "continuity_offer");
                    return Ok(message);
                }
            }
        }
    }
    orchestrator.set_continuity_adoption(continuity_adoption);

    let workspace = bridge.workspace().read().await.clone();
    let (canonical_path, path_sha256) =
        golish_agent_kit::runtime_memory::canonical_workspace_identity(&workspace)
            .map_err(anyhow::Error::new)
            .context("Resolve trusted workspace identity for runtime operation")?;
    let current_project_scope = runtime_repo_for_scope_authorization
        .project_scope_register_first_open(&canonical_path, &path_sha256)
        .await
        .map_err(anyhow::Error::new)
        .context("Register trusted project scope for runtime operation")?;

    let result = match resumable {
        Some(task) => {
            tracing::info!(
                target: "harness::task_mode",
                task_id = %task.id,
                "task mode: resuming prior operation for this chat session"
            );
            if looks_like_bare_stage_run_resume_prompt(task_input) {
                tracing::info!(
                    target: "harness::task_mode",
                    task_id = %task.id,
                    "bare continuation prompt: enabling one-shot stage_run fast resume"
                );
                orchestrator.set_force_stage_run_on_resume_once(true);
            }
            super::operation_resume::authorize_operation_resume(
                db_repo_for_scope_authorization.as_ref(),
                task.id,
                &current_project_scope,
            )
            .await?;
            orchestrator.resume(task.id, task_input, &executor).await
        }
        None => {
            orchestrator
                .run(task_input, current_project_scope, &executor)
                .await
        }
    };

    // The executor (and outer GUI request) still hold ownership here. Clear the
    // harness side-channels before any planner-declined Chat fallback can build a
    // loop context, and before the top-level command releases the lease.
    bridge
        .clear_top_level_request_state(&request)
        .await
        .context("clear Task request-local bridge state")?;

    let duration_ms = start_time.elapsed().as_millis() as u64;

    match &result {
        Ok(response) => {
            tracing::info!(
                "[TaskMode] Completed in {:.1}s, report length: {} chars",
                duration_ms as f64 / 1000.0,
                response.len(),
            );
            // Emit the final report as a separate completed message
            let report_turn = uuid::Uuid::new_v4().to_string();
            bridge.emit_event(AiEvent::Started {
                turn_id: report_turn,
            });
            bridge.emit_event(AiEvent::TextDelta {
                delta: response.clone(),
                accumulated: response.clone(),
            });
            bridge.emit_event(AiEvent::Completed {
                response: response.clone(),
                reasoning: None,
                input_tokens: None,
                output_tokens: None,
                duration_ms: Some(duration_ms),
            });
        }
        Err(e) => {
            let err_msg = format!("{:#}", e);
            // Safety net: the planner could not produce a plan because the input
            // was conversational (model replied in prose / `{"message": …}`).
            // Answer via the main agent instead of surfacing a "Generator failed"
            // error. The deterministic triage in the caller catches most of these
            // before the planner runs; this handles the ambiguous misjudged ones.
            if is_conversational_planner_failure(&err_msg) {
                tracing::info!(
                    target: "harness::task_mode",
                    error = %err_msg,
                    "planner declined (conversational input) — falling back to Chat policy reply"
                );
                return execute_chat_policy_turn(bridge, prompt)
                    .await
                    .context("Chat fallback after planner declined");
            }
            bridge.emit_event(AiEvent::Error {
                message: err_msg,
                error_type: "task_orchestrator".to_string(),
            });
        }
    }

    result
}

async fn execute_task_profile_turn(
    bridge: Arc<AgentBridge>,
    session_id: &str,
    prompt: &str,
    state: &AgentState,
    request: golish_agent_bridge::TopLevelRequestLease,
) -> anyhow::Result<String> {
    {
        let pending = bridge.pending_plan_request_handle();
        *pending.write().await = None;
    }

    let lead_response = bridge
        .execute_with_turn_instructions(prompt, task_profile_lead_instructions())
        .await?;
    let pending_payload = {
        let pending = bridge.pending_plan_request_handle();
        let payload = pending.write().await.take();
        payload
    };

    let Some(payload) = pending_payload else {
        return Ok(lead_response);
    };

    let continuity_decision = continuity_decision_from_start_operation_payload(&payload);
    let task_prompt = task_prompt_from_start_operation(prompt, &payload);
    tracing::info!(
        target: "harness::task_mode",
        session_id = %session_id,
        "start_operation requested by task/profile lead turn; entering task harness"
    );
    execute_task_mode_with_continuity(
        bridge,
        session_id,
        &task_prompt,
        state,
        continuity_decision,
        request,
    )
    .await
}

fn task_profile_lead_instructions() -> &'static str {
    "\
[Task/Profile Lead Policy]
You are in a flexible Task/Profile lead turn.
- For casual chat, explanations, coding, debugging, repo edits, or other non-pentest requests, answer directly.
- In Task/Profile mode, informal operation requests are enough to enter the structured operation harness. Phrases like \"搞一下 <company>\", \"弄一下 <target>\", \"帮我打/测/扫 <target>\", \"开始/开打/开搞\", \"进红队模式\", \"就搞他\", or \"整个集团\" after a target was named mean the user wants to start an operation. Call `start_operation`.
- Use `start_operation` as a handoff into Scoping, not as permission to scan everything. Preserve the user's target label and requested activity in a concise `objective`; do not invent domains, subsidiaries, IP ranges, or expanded scope in the lead turn. The structured harness will confirm legal scope, evidence, stage gates, and specialist execution.
- Only ask one concise clarification when the message has no usable target and no recoverable previous target, or when it is not clear whether the user wants chat/research/help versus an operation. Otherwise prefer `start_operation` over repeated clarification.
- If the recent conversation asked whether to reuse existing DB-backed progress and the user answers reuse/复用/接着已有, call `start_operation` with the remembered objective and `continuity_decision=\"reuse_existing\"`. If the user answers restart/重新开始/不要复用, call it with `continuity_decision=\"start_fresh\"`.
- When you call `start_operation`, do not continue with prose in the lead turn except for a brief handoff sentence if needed.
- This lead turn can use only these tools: read/list/grep/edit/create/write files, AST/code search, knowledge/memory helpers, `ask_human`, and `start_operation`. Ignore any earlier or frontend-provided tool documentation for recon/scanning tools; do not call `recon_lookup_company`, `manage_organizations`, `manage_targets`, `pentest_run`, `run_pty_cmd`, `run_command`, `stage_run`, or web search tools in this lead turn.
"
}

fn continuity_decision_from_start_operation_payload(
    payload: &str,
) -> golish_agent_kit::harness::ContinuityDecision {
    serde_json::from_str::<serde_json::Value>(payload)
        .ok()
        .and_then(|v| {
            v.get("continuity_decision")
                .and_then(|x| x.as_str())
                .and_then(golish_agent_kit::harness::ContinuityDecision::try_parse)
        })
        .unwrap_or_default()
}

fn task_prompt_from_start_operation(original_prompt: &str, payload: &str) -> String {
    let objective = serde_json::from_str::<serde_json::Value>(payload)
        .ok()
        .and_then(|v| {
            v.get("objective")
                .and_then(|x| x.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| extract_user_message_from_wrapped_prompt(original_prompt).to_string());

    replace_wrapped_user_message(original_prompt, &objective)
}

fn continuity_decision_from_prompt(
    task_input: &str,
) -> Option<golish_agent_kit::harness::ContinuityDecision> {
    let lower = task_input.to_lowercase();
    if contains_any(
        &lower,
        &[
            "不要复用",
            "不复用",
            "重新开始",
            "从头开始",
            "全新开始",
            "start fresh",
            "do not reuse",
            "don't reuse",
            "ignore existing",
        ],
    ) {
        return Some(golish_agent_kit::harness::ContinuityDecision::StartFresh);
    }
    if contains_any(
        &lower,
        &[
            "复用",
            "沿用",
            "用已有",
            "用之前",
            "接着已有",
            "接着之前",
            "reuse existing",
            "reuse previous",
            "adopt existing",
            "continue from db",
        ],
    ) {
        return Some(golish_agent_kit::harness::ContinuityDecision::ReuseExisting);
    }
    None
}

enum ContinuityAdoptionDecision {
    ReuseExisting,
    StartFresh,
    TextFallback(String),
}

const CONTINUITY_REUSE_OPTION: &str = "复用已有数据继续";
const CONTINUITY_START_FRESH_OPTION: &str = "重新开始";
const CONTINUITY_CONFIRM_TIMEOUT_SECS: u64 = 600;

async fn request_continuity_adoption_decision(
    bridge: &AgentBridge,
    plan: &golish_agent_kit::harness::ContinuityAdoptionPlan,
) -> ContinuityAdoptionDecision {
    use golish_core::events::AiEvent;

    let Some(coordinator) = bridge.coordinator().cloned() else {
        return ContinuityAdoptionDecision::TextFallback(render_continuity_offer(plan));
    };

    let request_id = uuid::Uuid::new_v4().to_string();
    let decision_rx = coordinator.register_approval(request_id.clone());
    bridge.emit_event(AiEvent::AskHumanRequest {
        request_id: request_id.clone(),
        question: render_continuity_offer(plan),
        input_type: "choice".to_string(),
        // Keep the safe default first because choice prompts auto-submit their
        // first option on timeout; adoption must never happen silently.
        options: vec![
            CONTINUITY_START_FRESH_OPTION.to_string(),
            CONTINUITY_REUSE_OPTION.to_string(),
        ],
        context: "Operation continuity gate: reuse adopts durable DB-backed facts from older runs; Skip or timeout starts fresh.".to_string(),
    });

    let decision = tokio::time::timeout(
        std::time::Duration::from_secs(CONTINUITY_CONFIRM_TIMEOUT_SECS),
        decision_rx,
    )
    .await;

    match decision {
        Ok(Ok(decision)) => {
            let response = decision.reason.clone().unwrap_or_default();
            bridge.emit_event(AiEvent::AskHumanResponse {
                request_id,
                response: response.clone(),
                skipped: !decision.approved,
            });
            if !decision.approved {
                return ContinuityAdoptionDecision::StartFresh;
            }
            match continuity_decision_from_prompt(&response) {
                Some(golish_agent_kit::harness::ContinuityDecision::ReuseExisting) => {
                    ContinuityAdoptionDecision::ReuseExisting
                }
                _ => ContinuityAdoptionDecision::StartFresh,
            }
        }
        Ok(Err(_)) => {
            bridge.emit_event(AiEvent::AskHumanResponse {
                request_id,
                response: String::new(),
                skipped: true,
            });
            ContinuityAdoptionDecision::StartFresh
        }
        Err(_) => {
            tracing::warn!(
                target: "harness::task_mode",
                timeout_secs = CONTINUITY_CONFIRM_TIMEOUT_SECS,
                "continuity adoption prompt timed out; starting fresh"
            );
            bridge.emit_event(AiEvent::AskHumanResponse {
                request_id,
                response: String::new(),
                skipped: true,
            });
            ContinuityAdoptionDecision::StartFresh
        }
    }
}

fn render_continuity_offer(plan: &golish_agent_kit::harness::ContinuityAdoptionPlan) -> String {
    let adopted = plan
        .adopted_stages
        .iter()
        .map(|stage| stage.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let next = plan.entry_stage.as_str();
    let scope_units = plan.snapshot.scope_units;
    format!(
        "我发现本地数据库里已有可复用的进度：scope units={scope_units}，可跳过阶段：{adopted}。要复用这些 DB-backed facts 并直接从 `{next}` 继续吗？选择“{CONTINUITY_REUSE_OPTION}”，或选择“{CONTINUITY_START_FRESH_OPTION}”。"
    )
}

fn emit_immediate_task_response(bridge: &AgentBridge, message: &str, response_type: &str) {
    use golish_core::events::AiEvent;

    let turn_id = uuid::Uuid::new_v4().to_string();
    bridge.emit_event(AiEvent::Started { turn_id });
    bridge.emit_event(AiEvent::TextDelta {
        delta: message.to_string(),
        accumulated: message.to_string(),
    });
    bridge.emit_event(AiEvent::Completed {
        response: message.to_string(),
        reasoning: Some(response_type.to_string()),
        input_tokens: None,
        output_tokens: None,
        duration_ms: Some(0),
    });
}

fn replace_wrapped_user_message(prompt: &str, user_message: &str) -> String {
    const MARKER: &str = "[User Message]\n";
    if let Some(idx) = prompt.find(MARKER) {
        let user_start = idx + MARKER.len();
        return format!("{}{}", &prompt[..user_start], user_message.trim());
    }
    user_message.trim().to_string()
}

fn explicit_task_prompt(prompt: &str) -> Option<String> {
    const MARKER: &str = "[User Message]\n";
    if let Some(idx) = prompt.find(MARKER) {
        let user_start = idx + MARKER.len();
        let prefix = &prompt[..user_start];
        let user_message = &prompt[user_start..];
        let task_message = strip_explicit_task_prefix(user_message)?;
        return Some(format!("{prefix}{task_message}"));
    }

    strip_explicit_task_prefix(prompt).map(str::to_string)
}

fn implicit_operation_prompt(prompt: &str) -> Option<String> {
    let user_message = extract_user_message_from_wrapped_prompt(prompt);
    should_auto_start_task_operation(user_message).then(|| prompt.to_string())
}

fn looks_like_resume_operation_prompt(user_message: &str) -> bool {
    let trimmed = user_message
        .trim()
        .trim_matches(|c: char| c.is_ascii_punctuation() || "，。！？；：、 ".contains(c));
    if trimmed.is_empty() || trimmed.len() > 96 {
        return false;
    }

    let lower = trimmed.to_lowercase();
    if matches!(
        lower.as_str(),
        "继续"
            | "继续吧"
            | "接着"
            | "接着来"
            | "接着跑"
            | "继续跑"
            | "继续推进"
            | "继续执行"
            | "继续任务"
            | "继续流程"
            | "continue"
            | "go on"
            | "resume"
            | "resume it"
            | "continue it"
    ) {
        return true;
    }

    let resume_prefixes = [
        "继续刚才",
        "继续上次",
        "继续之前",
        "接着刚才",
        "接着上次",
        "接着之前",
        "接上刚才",
        "接上上次",
        "resume ",
        "continue ",
    ];
    if resume_prefixes.iter().any(|p| lower.starts_with(p)) {
        return true;
    }

    let stage_terms = [
        "阶段",
        "任务",
        "流程",
        "operation",
        "stage",
        "harness",
        "eas",
        "target_intel",
        "external_attack_surface",
        "扫描",
        "测绘",
        "红队",
    ];
    (lower.starts_with("继续") || lower.starts_with("接着") || lower.starts_with("接上"))
        && stage_terms.iter().any(|term| lower.contains(term))
}

fn looks_like_bare_stage_run_resume_prompt(user_message: &str) -> bool {
    let trimmed = user_message
        .trim()
        .trim_matches(|c: char| c.is_ascii_punctuation() || "，。！？；：、 ".contains(c));
    if trimmed.is_empty() || trimmed.len() > 64 {
        return false;
    }

    let lower = trimmed.to_lowercase();
    if matches!(
        lower.as_str(),
        "继续"
            | "继续吧"
            | "接着"
            | "接着来"
            | "接着跑"
            | "继续跑"
            | "继续推进"
            | "继续执行"
            | "继续任务"
            | "继续流程"
            | "continue"
            | "go on"
            | "resume"
            | "resume it"
            | "continue it"
    ) {
        return true;
    }

    if contains_any(
        &lower,
        &[
            "但是", "不过", "先", "别", "不要", "换", "改", "解释", "分析", "看看", "日志", "代码",
            "文档", "why", "but", "first", "don't", "dont", "instead",
        ],
    ) {
        return false;
    }

    let resume_prefixes = [
        "继续刚才",
        "继续上次",
        "继续之前",
        "继续补",
        "接着刚才",
        "接着上次",
        "接着之前",
        "接上刚才",
        "接上上次",
        "resume ",
        "continue ",
    ];
    let stage_terms = [
        "阶段",
        "任务",
        "流程",
        "operation",
        "stage",
        "harness",
        "eas",
        "target_intel",
        "external_attack_surface",
        "blocked",
        "补",
        "那几个",
        "这几个",
        "扫描",
        "测绘",
    ];
    resume_prefixes.iter().any(|p| lower.starts_with(p))
        && stage_terms.iter().any(|term| lower.contains(term))
}

fn should_auto_start_task_operation(user_message: &str) -> bool {
    let trimmed = user_message.trim();
    if trimmed.is_empty() {
        return false;
    }

    let lower = trimmed.to_lowercase();
    if contains_any(
        &lower,
        &[
            "代码",
            "仓库",
            "组件",
            "函数",
            "报错",
            "日志",
            "编译",
            "类型",
            "样式",
            "前端",
            "后端",
            "react",
            "rust",
            "typescript",
            "javascript",
            "python",
            "terminal",
            "css",
            "tsc",
            "cargo",
            "pnpm",
        ],
    ) {
        return false;
    }

    const INFORMAL_TARGET_ACTIONS: &[&str] = &[
        "搞一下",
        "搞下",
        "搞一搞",
        "帮我搞",
        "弄一下",
        "弄下",
        "帮我弄",
        "打一下",
        "测一下",
        "扫一下",
        "扫一遍",
        "扫下",
    ];
    if has_nontrivial_tail_after_any(trimmed, INFORMAL_TARGET_ACTIONS) {
        return true;
    }

    let has_operation_signal = contains_any(
        &lower,
        &[
            "渗透",
            "红队",
            "开打",
            "开搞",
            "扫描",
            "攻击面",
            "信息收集",
            "被动信息",
            "pentest",
            "recon",
            "scan ",
            "scan:",
            "scan\t",
        ],
    );
    if !has_operation_signal {
        return false;
    }

    contains_network_target(&lower)
        || contains_any(
            trimmed,
            &["集团", "公司", "银行", "保险", "大学", "医院", "目标"],
        )
        || (trimmed.contains('对') && trimmed.contains('做'))
        || has_nontrivial_tail_after_any(trimmed, &["目标", "target", "scan", "扫描"])
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn has_nontrivial_tail_after_any(text: &str, triggers: &[&str]) -> bool {
    triggers.iter().any(|trigger| {
        let Some(idx) = text.find(trigger) else {
            return false;
        };
        let tail = text[idx + trigger.len()..].trim_matches(|c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    ':' | '：' | ',' | '，' | '.' | '。' | ';' | '；' | '-' | '—' | '_' | ' '
                )
        });
        tail.chars()
            .filter(|c| !c.is_whitespace() && !c.is_ascii_punctuation())
            .count()
            >= 2
    })
}

fn contains_network_target(lower: &str) -> bool {
    lower.contains("http://")
        || lower.contains("https://")
        || split_target_tokens(lower).any(|token| is_ipv4_like(token) || is_domain_like(token))
}

fn split_target_tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                ',' | '，'
                    | '。'
                    | ';'
                    | '；'
                    | '('
                    | ')'
                    | '（'
                    | '）'
                    | '['
                    | ']'
                    | '【'
                    | '】'
                    | '<'
                    | '>'
                    | '"'
                    | '\''
                    | '`'
            )
    })
    .filter(|token| !token.is_empty())
}

fn is_ipv4_like(token: &str) -> bool {
    let mut count = 0;
    for part in token.split('.') {
        count += 1;
        if part.parse::<u8>().is_err() {
            return false;
        }
    }
    count == 4
}

fn is_domain_like(token: &str) -> bool {
    if !token.contains('.') || !token.chars().any(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    let Some(tld) = token.rsplit('.').next() else {
        return false;
    };
    tld.len() >= 2
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

fn strip_explicit_task_prefix(message: &str) -> Option<&str> {
    let trimmed = message.trim_start();
    for prefix in ["/task", "/harness"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return Some(rest.trim_start_matches([' ', '\n', '\t', ':', '：']));
        }
    }
    for prefix in ["task:", "task：", "harness:", "harness："] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return Some(rest.trim_start());
        }
    }
    None
}

/// Execute one fallback turn with Chat-mode policy even when the UI picker is on
/// a Task/profile mode, then restore the original execution mode. This keeps
/// profile mode flexible for casual chat and ordinary coding/debugging requests
/// while preventing Task-only tools (`stage_run`, harness submit, recon fan-out)
/// from leaking into that fallback turn.
async fn execute_chat_policy_turn(
    bridge: Arc<AgentBridge>,
    prompt: &str,
) -> anyhow::Result<String> {
    use golish_agent_kit::execution_mode::ExecutionMode;

    let original_mode = bridge.get_execution_mode().await;
    if original_mode == ExecutionMode::Chat {
        return bridge.execute(prompt).await;
    }

    bridge.set_execution_mode(ExecutionMode::Chat).await;
    let result = bridge.execute(prompt).await;

    // Do not clobber an explicit mode change that happened while this turn was
    // running. The usual UI path disallows concurrent sends, but this keeps the
    // helper well-behaved for direct IPC/tests too.
    if bridge.get_execution_mode().await == ExecutionMode::Chat {
        bridge.set_execution_mode(original_mode).await;
    }

    result
}

/// Whether a Task-orchestrator failure is really "the planner declined because
/// the input was conversational" (model replied in prose / `{"message": …}`
/// instead of a plan) rather than a genuine error. Mirrors the frontend
/// `classifyErrorSeverity` signals so both layers agree on what is "soft".
fn is_conversational_planner_failure(err_msg: &str) -> bool {
    let lower = err_msg.to_lowercase();
    const SIGNALS: &[&str] = &[
        "declined to produce a plan",
        "returned a message instead of a plan",
        "refused the request or asked a question",
        "failed to parse task planner json",
    ];
    if SIGNALS.iter().any(|s| lower.contains(s)) {
        return true;
    }
    lower.contains("missing field") && lower.contains("subtasks")
}

fn extract_user_message_from_wrapped_prompt(prompt: &str) -> &str {
    prompt
        .find("[User Message]\n")
        .map(|idx| &prompt[idx + "[User Message]\n".len()..])
        .unwrap_or(prompt)
        .trim()
}

/// Truncate a string to at most `max_bytes` bytes without splitting a
/// multi-byte UTF-8 character. Used to derive a short session title from
/// the user prompt; never panics on Chinese/emoji input.
fn truncate_for_title(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

#[cfg(test)]
mod chat_title_tests {
    use super::{
        continuity_decision_from_prompt, continuity_decision_from_start_operation_payload,
        explicit_task_prompt, extract_user_message_from_wrapped_prompt, implicit_operation_prompt,
        is_conversational_planner_failure, looks_like_bare_stage_run_resume_prompt,
        looks_like_resume_operation_prompt, render_continuity_offer,
        should_auto_start_task_operation, task_profile_lead_instructions,
        task_prompt_from_start_operation, truncate_for_title,
    };

    #[tokio::test]
    async fn task_mode_tracker_rebind_updates_existing_runtime_clones() {
        let pool = std::sync::Arc::new(
            sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgres://localhost/golish_test")
                .expect("lazy pool"),
        );
        let initial = uuid::Uuid::new_v4();
        let canonical = uuid::Uuid::new_v4();
        let tracker = golish_agent_kit::db_tracking::DbTracker::new(
            std::sync::Arc::new(crate::ai::tracking_bridge::PgTrackingBackend::new(pool)),
            initial,
            crate::ai::tracking_bridge::CoreDbReadyGate(golish_core::DbReadyGate::new()),
        );
        let runtime_clone = tracker.clone();

        tracker.set_session_uuid(canonical);

        assert_eq!(runtime_clone.session_uuid(), canonical);
    }

    #[test]
    fn ascii_under_limit_returned_as_is() {
        assert_eq!(truncate_for_title("hello world", 80), "hello world");
    }

    #[test]
    fn ascii_over_limit_truncated_to_bytes() {
        let s = "a".repeat(200);
        let out = truncate_for_title(&s, 80);
        assert_eq!(out.len(), 80);
    }

    #[test]
    fn chinese_over_limit_truncated_on_char_boundary() {
        // Each Chinese char is 3 bytes in UTF-8. 30 chars = 90 bytes.
        let s = "评估外部攻击面共三十个字符的中文".repeat(2);
        let out = truncate_for_title(&s, 80);
        assert!(out.len() <= 80);
        // Must still be valid UTF-8 (no panic on indexing); chars() succeeds.
        let char_count = out.chars().count();
        assert!(char_count > 0);
    }

    #[test]
    fn empty_input_returns_empty() {
        assert_eq!(truncate_for_title("", 80), "");
    }

    #[test]
    fn limit_zero_returns_empty() {
        assert_eq!(truncate_for_title("anything", 0), "");
    }

    #[test]
    fn task_mode_extracts_user_message_from_wrapped_prompt() {
        let prompt = "[System Context]\nlarge system prompt\n\n[User Message]\n帮我看看example.com";

        assert_eq!(
            extract_user_message_from_wrapped_prompt(prompt),
            "帮我看看example.com"
        );
    }

    #[test]
    fn task_mode_uses_plain_prompt_when_not_wrapped() {
        assert_eq!(
            extract_user_message_from_wrapped_prompt("帮我看看example.com"),
            "帮我看看example.com"
        );
    }

    #[test]
    fn normal_sends_do_not_bypass_the_lead_turn_with_prefix_parser() {
        for message in [
            "你好",
            "你叫什么啊 哥哥",
            "帮我写个脚本解析日志",
            "这个 React 组件帮我 debug 一下",
            "scan example.com for vulns",
            "对 https://target.tld 做渗透",
            "继续刚才那个 EAS 阶段",
        ] {
            assert_eq!(
                explicit_task_prompt(message),
                None,
                "without an explicit prefix, this must go through the flexible lead turn first: {message:?}"
            );
        }
    }

    #[test]
    fn explicit_task_prefix_enters_harness_and_strips_marker() {
        assert_eq!(
            explicit_task_prompt("/task scan example.com").as_deref(),
            Some("scan example.com")
        );
        assert_eq!(
            explicit_task_prompt("/harness: 对 moresec.cn 做被动扫描").as_deref(),
            Some("对 moresec.cn 做被动扫描")
        );
        assert_eq!(
            explicit_task_prompt("task：继续刚才那个 EAS 阶段").as_deref(),
            Some("继续刚才那个 EAS 阶段")
        );
    }

    #[test]
    fn explicit_task_prefix_preserves_wrapped_system_context() {
        let prompt = "[System Context]\nctx\n\n[User Message]\n/task scan example.com";

        assert_eq!(
            explicit_task_prompt(prompt).as_deref(),
            Some("[System Context]\nctx\n\n[User Message]\nscan example.com")
        );
    }

    #[test]
    fn implicit_operation_prompt_enters_harness_for_targeted_operation_intent() {
        assert_eq!(
            implicit_operation_prompt("帮我搞一下平安咯").as_deref(),
            Some("帮我搞一下平安咯")
        );
        assert_eq!(
            implicit_operation_prompt("[User Message]\n对 https://target.tld 做渗透").as_deref(),
            Some("[User Message]\n对 https://target.tld 做渗透")
        );
        assert!(should_auto_start_task_operation(
            "目标 example.com 进红队模式"
        ));
        assert!(should_auto_start_task_operation(
            "scan example.com for vulns"
        ));
    }

    #[test]
    fn implicit_operation_prompt_leaves_chat_and_code_requests_to_lead() {
        for message in [
            "你好",
            "你是谁",
            "你会渗透测试吗",
            "开搞",
            "帮我搞一下这个 React 组件",
            "帮我弄一下 cargo 编译报错",
            "帮我看看这个日志",
        ] {
            assert_eq!(
                implicit_operation_prompt(message),
                None,
                "message should remain in the flexible lead turn: {message:?}"
            );
        }
    }

    #[test]
    fn resume_operation_prompt_detects_short_resume_commands() {
        for message in [
            "继续",
            "继续吧",
            "接着跑",
            "继续刚才那个 EAS 阶段",
            "继续上次的 target_intel",
            "resume",
            "continue the previous operation",
        ] {
            assert!(
                looks_like_resume_operation_prompt(message),
                "resume phrase should be deterministic: {message:?}"
            );
        }
    }

    #[test]
    fn resume_operation_prompt_ignores_unrelated_continuations() {
        for message in [
            "继续改这个 React 组件",
            "继续分析这个 cargo 编译报错",
            "继续看日志为什么爆了",
            "继续帮我写文档",
        ] {
            assert!(
                !looks_like_resume_operation_prompt(message),
                "non-operation continuation should stay in the flexible lead turn: {message:?}"
            );
        }
    }

    #[test]
    fn bare_resume_prompt_enables_stage_run_fast_path() {
        for message in [
            "继续",
            "继续吧",
            "接着跑",
            "继续刚才那个 EAS 阶段",
            "继续补刚才那3个",
            "continue the previous stage",
        ] {
            assert!(
                looks_like_bare_stage_run_resume_prompt(message),
                "bare operation continuation should enable fast stage_run resume: {message:?}"
            );
        }
    }

    #[test]
    fn steered_resume_prompt_stays_on_normal_resume_path() {
        for message in [
            "继续，但是先别扫端口",
            "继续，不过先解释一下",
            "继续看日志为什么爆了",
            "continue but first inspect the logs",
        ] {
            assert!(
                !looks_like_bare_stage_run_resume_prompt(message),
                "steered continuation should not force stage_run: {message:?}"
            );
        }
    }

    #[test]
    fn start_operation_payload_replaces_wrapped_user_message() {
        let prompt = "[System Context]\nctx\n\n[User Message]\n搞一下平安";
        let payload = r#"{"objective":"对中国平安保险集团做授权范围确认与外部攻击面评估","analysis":"用户已确认目标"}"#;

        assert_eq!(
            task_prompt_from_start_operation(prompt, payload),
            "[System Context]\nctx\n\n[User Message]\n对中国平安保险集团做授权范围确认与外部攻击面评估"
        );
    }

    #[test]
    fn start_operation_payload_falls_back_to_original_message_when_invalid() {
        assert_eq!(
            task_prompt_from_start_operation("搞一下平安", "{not json"),
            "搞一下平安"
        );
    }

    #[test]
    fn start_operation_payload_reads_continuity_decision() {
        use golish_agent_kit::harness::ContinuityDecision;

        assert_eq!(
            continuity_decision_from_start_operation_payload(
                r#"{"objective":"继续","continuity_decision":"reuse_existing"}"#
            ),
            ContinuityDecision::ReuseExisting
        );
        assert_eq!(
            continuity_decision_from_start_operation_payload(
                r#"{"objective":"重跑","continuity_decision":"start_fresh"}"#
            ),
            ContinuityDecision::StartFresh
        );
        assert_eq!(
            continuity_decision_from_start_operation_payload(r#"{"objective":"默认"}"#),
            ContinuityDecision::AskBeforeReuse
        );
    }

    #[test]
    fn prompt_text_can_confirm_or_reject_continuity() {
        use golish_agent_kit::harness::ContinuityDecision;

        assert_eq!(
            continuity_decision_from_prompt("复用已有数据继续"),
            Some(ContinuityDecision::ReuseExisting)
        );
        assert_eq!(
            continuity_decision_from_prompt("不要复用，重新开始"),
            Some(ContinuityDecision::StartFresh)
        );
        assert_eq!(continuity_decision_from_prompt("继续看看日志"), None);
    }

    #[test]
    fn continuity_offer_names_adopted_stages_and_next_entry() {
        use golish_agent_kit::harness::{ContinuityAdoptionPlan, ContinuitySnapshot, StageKind};

        let message = render_continuity_offer(&ContinuityAdoptionPlan {
            schema_v: 1,
            adopted_stages: vec![StageKind::Scoping, StageKind::TargetIntel],
            entry_stage: StageKind::ExternalAttackSurface,
            remaining_stages: vec![StageKind::ExternalAttackSurface, StageKind::Reporting],
            all_projected_stages_reusable: false,
            snapshot: ContinuitySnapshot {
                scope_units: 8,
                stages: Vec::new(),
            },
        });

        assert!(message.contains("scope units=8"));
        assert!(message.contains("scoping, target_intel"));
        assert!(message.contains("`external_attack_surface`"));
        assert!(message.contains("复用已有数据继续"));
        assert!(message.contains("重新开始"));
    }

    #[test]
    fn task_profile_lead_instructions_handoff_informal_operation_requests() {
        let out = task_profile_lead_instructions();

        assert!(out.starts_with("[Task/Profile Lead Policy]"));
        assert!(out.contains("call `start_operation`"));
        assert!(out.contains("搞一下 <company>"));
        assert!(out.contains("就搞他"));
        assert!(out.contains("Preserve the user's target label"));
        assert!(out.contains("The structured harness will confirm legal scope"));
        assert!(out.contains("prefer `start_operation` over repeated clarification"));
        assert!(out.contains("do not call `recon_lookup_company`"));
        assert!(out.contains("`pentest_run`"));
        assert!(out.contains("`run_pty_cmd`"));
        assert!(!out.contains("Do not call `start_operation` yet"));
        assert!(!out.contains("[User Message]"));
    }

    #[test]
    fn conversational_planner_failure_detects_soft_signals() {
        assert!(is_conversational_planner_failure(
            "Generator failed: The task planner declined to produce a plan — ..."
        ));
        assert!(is_conversational_planner_failure(
            "Generator failed: Failed to parse task planner JSON (missing field `subtasks` at line 3)"
        ));
        assert!(is_conversational_planner_failure(
            "[API trace=abc] send_ai_prompt_session: ... missing field `subtasks` ..."
        ));
    }

    #[test]
    fn conversational_planner_failure_ignores_real_errors() {
        assert!(!is_conversational_planner_failure(
            "Generator LLM call failed: connection refused"
        ));
        assert!(!is_conversational_planner_failure(
            "authentication failed (401)"
        ));
    }
}

/// Get vision capabilities for the current model in a session.
///
/// Returns information about whether the model supports images,
/// maximum image size, and supported formats.
#[tauri::command]
pub async fn get_vision_capabilities(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<golish_llm_providers::VisionCapabilities, GolishError> {
    let bridges = state.ai_state.get_bridges().await;
    let bridge = bridges
        .get(&session_id)
        .ok_or_else(|| super::super::ai_session_not_initialized_error(&session_id))?;

    Ok(golish_llm_providers::VisionCapabilities::detect(
        bridge.provider_name(),
        bridge.model_name(),
    ))
}

/// Send a multi-modal prompt (text + images) to the AI agent.
///
/// This command accepts a PromptPayload with multiple parts, enabling
/// image attachments for vision-capable models. If the model doesn't
/// support vision, images are stripped and a warning event is emitted.
///
/// IMPORTANT: Uses get_session_bridge() to clone the Arc and release the map
/// lock immediately. This allows other sessions to initialize/shutdown while
/// this session is executing, enabling true concurrent multi-tab agent execution.
#[tauri::command]
pub async fn send_ai_prompt_with_attachments(
    state: State<'_, AgentState>,
    session_id: String,
    payload: golish_core::PromptPayload,
) -> Result<String, GolishError> {
    use golish_core::PromptPart;
    use rig::message::{ImageMediaType, Text, UserContent};

    // Get Arc clone and release map lock immediately
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| super::super::ai_session_not_initialized_error(&session_id))?;
    let request = bridge
        .begin_top_level_request()
        .await
        .map_err(|error| GolishError::Internal(error.to_string()))?;

    // Check vision capabilities
    let caps = golish_llm_providers::VisionCapabilities::detect(
        bridge.provider_name(),
        bridge.model_name(),
    );

    // If provider doesn't support vision, strip images and emit warning
    let effective_payload = if payload.has_images() && !caps.supports_vision {
        tracing::warn!(
            "Provider {} doesn't support images, sending text-only",
            bridge.provider_name()
        );

        // Emit warning event to frontend
        bridge.emit_event(golish_core::AiEvent::Warning {
            message: format!(
                "Images removed: {} does not support vision",
                bridge.model_name()
            ),
        });

        golish_core::PromptPayload::from_text(payload.text_only())
    } else {
        // Validate payload
        payload.validate(caps.max_image_size_bytes, &caps.supported_formats)?;
        payload
    };

    // Convert PromptPayload to Vec<UserContent>
    let content_parts: Vec<UserContent> = effective_payload
        .parts
        .into_iter()
        .map(|p| match p {
            PromptPart::Text { text } => UserContent::Text(Text { text }),
            PromptPart::Image {
                data, media_type, ..
            } => {
                // Strip data URL prefix if present
                let has_data_url_prefix = data.starts_with("data:");
                let base64_data = if has_data_url_prefix {
                    data.split(',').nth(1).unwrap_or(&data).to_string()
                } else {
                    data
                };

                let img_media_type = media_type.as_deref().and_then(|mime| match mime {
                    "image/png" => Some(ImageMediaType::PNG),
                    "image/jpeg" | "image/jpg" => Some(ImageMediaType::JPEG),
                    "image/gif" => Some(ImageMediaType::GIF),
                    "image/webp" => Some(ImageMediaType::WEBP),
                    _ => None,
                });

                UserContent::image_base64(base64_data, img_media_type, None)
            }
        })
        .collect();

    // Execute without holding the map lock - other sessions can init/shutdown
    let result = bridge.execute_with_content(content_parts).await;
    let cleanup = bridge.clear_top_level_request_state(&request).await;
    match (result, cleanup) {
        (Ok(response), Ok(())) => Ok(response),
        (Err(error), _) => Err(GolishError::from(error)),
        (Ok(_), Err(error)) => Err(GolishError::Internal(error.to_string())),
    }
}

/// Clear the conversation history for a specific session.
///
/// IMPORTANT: Uses get_session_bridge() to clone the Arc and release the map
/// lock immediately, avoiding deadlocks when other tasks need write access.
#[tauri::command]
pub async fn clear_ai_conversation_session(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<(), GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| super::super::ai_session_not_initialized_error(&session_id))?;
    let request = bridge
        .begin_top_level_request()
        .await
        .map_err(|error| GolishError::Internal(error.to_string()))?;
    bridge.clear_conversation_history().await;
    bridge
        .clear_top_level_request_state(&request)
        .await
        .map_err(|error| GolishError::Internal(error.to_string()))?;
    tracing::info!("Conversation cleared for session {}", session_id);
    Ok(())
}

/// Get the conversation length for a specific session.
///
/// IMPORTANT: Uses get_session_bridge() to clone the Arc and release the map
/// lock immediately, avoiding deadlocks when other tasks need write access.
#[tauri::command]
pub async fn get_ai_conversation_length_session(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<usize, GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| super::super::ai_session_not_initialized_error(&session_id))?;
    Ok(bridge.conversation_history_len().await)
}

/// Signal that the frontend is ready to receive AI events for a session.
///
/// This command should be called by the frontend after it has set up its event listeners.
/// It causes any buffered events to be flushed to the frontend and enables direct event
/// emission going forward.
///
/// This solves race conditions where events are emitted before the frontend is ready
/// to receive them.
///
/// # Arguments
/// * `session_id` - The terminal session ID (tab) to signal ready for
#[tauri::command]
pub async fn signal_frontend_ready(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<(), GolishError> {
    tracing::info!(
        message = "[signal_frontend_ready] Frontend signaling ready",
        session_id = %session_id,
    );

    if let Some(bridge) = state.ai_state.get_session_bridge(&session_id).await {
        bridge.mark_frontend_ready().await;
        tracing::debug!(
            message = "[signal_frontend_ready] Marked frontend as ready",
            session_id = %session_id,
        );
    } else {
        tracing::debug!(
            message = "[signal_frontend_ready] No bridge found for session (may not be initialized yet)",
            session_id = %session_id,
        );
    }

    Ok(())
}
