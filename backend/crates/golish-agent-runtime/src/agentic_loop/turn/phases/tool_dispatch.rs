//! ToolDispatch phase — filter the LLM's tool calls against the allowed
//! tool list, push synthetic tool-result errors for blocked calls, and
//! dispatch the rest via the sibling `tool_dispatch::dispatch_tool_calls`.
//!
//! In Task mode the primary agent only has orchestration tools; the
//! model may hallucinate direct-tool calls from the system prompt or
//! restored history. This phase is the chokepoint that prevents those
//! from reaching the executors and feeds back an explicit error to the
//! model so it learns to delegate via `sub_agent_*` tools.

use rig::completion::Message;
use rig::message::{Text, ToolCall, ToolResult, ToolResultContent, UserContent};
use rig::one_or_many::OneOrMany;
use tracing::Span;

use golish_agent_kit::system_hooks::HookRegistry;
use golish_agent_kit::tool_policy::PolicyConstraintResult;
use golish_core::events::AiEvent;
use golish_sub_agents::SubAgentContext;

use super::super::super::context::{AgenticLoopContext, LoopCaptureContext};
use super::super::super::llm_stream_start::SUBMIT_STAGE_DELIVERABLE_TOOL;
use super::super::super::tool_dispatch::dispatch_tool_calls;
use super::super::super::tool_dispatch::ToolDispatchOutcome;
use super::super::super::tool_gate::{decide_tool_intent, ToolGateDecision};
use super::super::super::tool_intent::{ToolIntent, ToolIntentSource};
use super::super::super::unified_helpers::push_unavailable_tool_results;

/// Partition the LLM-issued tool calls into permitted/rejected, surface
/// errors for the rejected ones, and dispatch the permitted batch.
///
/// `submit_only_lock` (设计 2026-06-12 防御 B): when a targeted gate-repair
/// pass has locked this turn onto `submit_stage_deliverable`, any other tool
/// call (including ones the textual tool-adapter recovered from prose, which
/// bypass the API-layer `tool_choice`) is refused here, before it reaches an
/// executor, and answered with a corrective tool-result.
///
/// `forced_tool_lock` is the same hardening for deterministic resume turns:
/// only the named tool is allowed until it has been dispatched once.
#[allow(clippy::too_many_arguments)]
pub async fn run<M>(
    tool_calls_to_execute: Vec<ToolCall>,
    tools: &[rig::completion::ToolDefinition],
    ctx: &AgenticLoopContext<'_>,
    capture_ctx: &LoopCaptureContext,
    model: &M,
    sub_agent_context: &SubAgentContext,
    hook_registry: &HookRegistry,
    llm_span: &Span,
    chat_history: &mut Vec<Message>,
    submit_only_lock: bool,
    forced_tool_lock: Option<&str>,
) -> ToolDispatchOutcome
where
    M: rig::completion::CompletionModel + Sync,
{
    // 防御 B · submit-only 闭锁：锁定期只放过 submit_stage_deliverable，其余（含
    // textual-adapter 恢复的、本来在 allow-list 里的 update_plan）一律拒 + 回灌定向
    // 纠正。每个被拒调用配一条 ToolResult，保持 assistant tool_call ↔ tool_result
    // 配对完整（否则 provider 报错）。
    let tool_calls_to_execute = if submit_only_lock {
        let (submit, blocked) = split_for_submit_only(tool_calls_to_execute);
        if !blocked.is_empty() {
            let blocked_names: Vec<&str> =
                blocked.iter().map(|tc| tc.function.name.as_str()).collect();
            tracing::warn!(
                target: "agent-observe",
                blocked = ?blocked_names,
                "[submit-only-lock] refused non-submit tool call(s) during gate-repair pass"
            );
            push_submit_only_rejections(chat_history, &blocked);
        }
        submit
    } else if let Some(forced_tool) = forced_tool_lock {
        let (allowed, blocked) = split_for_forced_tool(tool_calls_to_execute, forced_tool);
        if !blocked.is_empty() {
            let blocked_names: Vec<&str> =
                blocked.iter().map(|tc| tc.function.name.as_str()).collect();
            tracing::warn!(
                target: "agent-observe",
                forced_tool,
                blocked = ?blocked_names,
                "[forced-tool-lock] refused non-forced tool call(s) during deterministic resume"
            );
            push_forced_tool_rejections(chat_history, &blocked, forced_tool);
        }
        allowed
    } else {
        tool_calls_to_execute
    };

    let mut gated_tool_calls = Vec::new();
    let mut gate_rejected = Vec::new();
    for tc in tool_calls_to_execute {
        let decision = gate_tool_call_for_dispatch(&tc, ctx.harness_stage, ctx.harness_authz);
        emit_tool_intent_observation(ctx, &tc, &decision);
        match decision {
            ToolGateDecision::Allow => gated_tool_calls.push(tc),
            ToolGateDecision::RequireApproval { reason } => {
                tracing::info!(
                    target: "agent-observe",
                    tool_name = %tc.function.name,
                    reason = %reason,
                    "tool intent requires approval before dispatch"
                );
                gated_tool_calls.push(tc);
            }
            ToolGateDecision::RequireHumanAnswer { question } => {
                tracing::info!(
                    target: "agent-observe",
                    tool_name = %tc.function.name,
                    question_preview = %truncate_for_log(&question),
                    "tool intent requires human answer before continuation"
                );
                gated_tool_calls.push(tc);
            }
            ToolGateDecision::Reject { reason } => {
                tracing::warn!(
                    target: "agent-observe",
                    tool_name = %tc.function.name,
                    reason = %reason,
                    "tool intent rejected before dispatch"
                );
                gate_rejected.push(tc);
            }
        }
    }

    let allowed_names: std::collections::HashSet<&str> =
        tools.iter().map(|t| t.name.as_str()).collect();
    let (permitted, mut rejected): (Vec<_>, Vec<_>) = gated_tool_calls
        .into_iter()
        .partition(|tc| allowed_names.contains(tc.function.name.as_str()));
    rejected.extend(gate_rejected);

    if !rejected.is_empty() {
        let rejected_names: Vec<&str> = rejected
            .iter()
            .map(|tc| tc.function.name.as_str())
            .collect();
        tracing::warn!(
            "[tool-guard] Blocked {} tool call(s) not in allowed list: {:?}",
            rejected.len(),
            rejected_names,
        );
        emit_policy_denials_for_rejected(&rejected, ctx, capture_ctx).await;
        push_unavailable_tool_results(chat_history, &rejected);
    }

    if !permitted.is_empty() {
        return dispatch_tool_calls(
            permitted,
            ctx,
            capture_ctx,
            model,
            sub_agent_context,
            hook_registry,
            llm_span,
            chat_history,
        )
        .await;
    }
    ToolDispatchOutcome::default()
}

async fn emit_policy_denials_for_rejected(
    rejected: &[ToolCall],
    ctx: &AgenticLoopContext<'_>,
    capture_ctx: &LoopCaptureContext,
) {
    for tool_call in rejected {
        if ctx
            .access
            .tool_policy_manager
            .is_denied(&tool_call.function.name)
            .await
        {
            emit_tool_denied_event(
                tool_call,
                "Tool is denied by policy".to_string(),
                ctx,
                capture_ctx,
            );
            continue;
        }

        if let PolicyConstraintResult::Violated(reason) = ctx
            .access
            .tool_policy_manager
            .apply_constraints(&tool_call.function.name, &tool_call.function.arguments)
            .await
        {
            emit_tool_denied_event(tool_call, reason, ctx, capture_ctx);
        }
    }
}

fn emit_tool_denied_event(
    tool_call: &ToolCall,
    reason: String,
    ctx: &AgenticLoopContext<'_>,
    capture_ctx: &LoopCaptureContext,
) {
    let event = AiEvent::ToolDenied {
        request_id: tool_call.id.clone(),
        tool_name: tool_call.function.name.clone(),
        args: tool_call.function.arguments.clone(),
        reason,
        source: golish_core::events::ToolSource::Main,
    };
    let _ = ctx.events.event_tx.send(event.clone());
    capture_ctx.process(&event);
}

fn gate_tool_call_for_dispatch(
    tool_call: &ToolCall,
    harness_stage: Option<golish_agent_kit::harness::StageKind>,
    harness_authz: Option<golish_agent_kit::harness::HarnessAuthz>,
) -> ToolGateDecision {
    // C3 · harness stage per-tool dispatch authorization (when a stage is active).
    //
    // Category whitelist (deny-by-default), enforced ONLY on scan invocations
    // (`pentest_run` / `run_pty_cmd` or tools in the scan taxonomy). Agent/meta
    // tools (`sub_agent_*`, `submit_stage_deliverable`, `query_target_data`,
    // `record_finding`, `manage_targets`, `log_*`, memory/graph) are exempt — they
    // are never scan invocations, so they pass without being listed per stage. A
    // scan whose resolved tool type is not in the stage's `allowed_tool_types` is
    // rejected; an allowed scan additionally must not exceed the profile's
    // authorization ceiling. `harness_stage == None` (non-stage turn) skips the
    // whole block.
    if let Some(kind) = harness_stage {
        if let Ok(spec) = golish_agent_kit::harness::load_embedded_stage_spec(kind) {
            let raw_name = tool_call.function.name.as_str();
            let args = &tool_call.function.arguments;
            // Category whitelist (deny-by-default), enforced ONLY on scan
            // invocations — pentest_run / run_pty_cmd or tools in the scan
            // taxonomy. Agent/meta tools (sub_agent_*, submit, query_target_data,
            // record_finding, manage_targets, log_*, memory/graph) are exempt.
            if golish_agent_kit::harness::is_scan_invocation(raw_name, args) {
                if !golish_agent_kit::harness::stage_allows(
                    raw_name,
                    args,
                    &spec.allowed_tool_types,
                ) {
                    return ToolGateDecision::Reject {
                        reason: not_in_whitelist_reason(raw_name, &spec.id),
                    };
                }
                // Orthogonal profile authorization ceiling (intent vs max_authz).
                if let Some(authz) = harness_authz {
                    if let Err(err) =
                        golish_agent_kit::harness::PreActionAuthorizer::check_intent_ceiling(
                            authz.intent,
                            authz.max_authorization,
                        )
                    {
                        return ToolGateDecision::Reject {
                            reason: err.to_string(),
                        };
                    }
                }
            }
        }
    }

    let source = infer_tool_intent_source(tool_call);
    let intent = if source == ToolIntentSource::NativeToolCall {
        ToolIntent::from_native(tool_call.clone())
    } else {
        ToolIntent::recovered_textual_xml(
            tool_call.id.clone(),
            tool_call.function.name.clone(),
            tool_call.function.arguments.clone(),
            None,
        )
    };

    // Full target registration checks need target context. Until that state is
    // available in this phase, preserve existing runtime behaviour and use the
    // gate for hard barriers/recovered-call classification.
    decide_tool_intent(&intent, true)
}

fn infer_tool_intent_source(tool_call: &ToolCall) -> ToolIntentSource {
    if tool_call.id.starts_with("textual-tool-call-") {
        ToolIntentSource::TextualXml
    } else {
        ToolIntentSource::NativeToolCall
    }
}

fn emit_tool_intent_observation(
    ctx: &AgenticLoopContext<'_>,
    tool_call: &ToolCall,
    decision: &ToolGateDecision,
) {
    let (decision_label, reason) = match decision {
        ToolGateDecision::Allow => ("allow", None),
        ToolGateDecision::RequireApproval { reason } => ("require_approval", Some(reason.clone())),
        ToolGateDecision::RequireHumanAnswer { question } => {
            ("require_human_answer", Some(truncate_for_log(question)))
        }
        ToolGateDecision::Reject { reason } => ("reject", Some(reason.clone())),
    };
    let source = match infer_tool_intent_source(tool_call) {
        ToolIntentSource::NativeToolCall => "native_tool_call",
        ToolIntentSource::TextualXml => "textual_xml",
        ToolIntentSource::TextualJson => "textual_json",
        ToolIntentSource::Recovered => "recovered",
    };

    let _ = ctx.events.event_tx.send(AiEvent::ToolIntentObservation {
        request_id: tool_call.id.clone(),
        tool_name: tool_call.function.name.clone(),
        source: source.to_string(),
        decision: decision_label.to_string(),
        reason,
        raw_preview: None,
    });
}

fn not_in_whitelist_reason(tool: &str, stage_id: &str) -> String {
    format!(
        "tool '{}' is not in the allowed tool types for harness stage '{}'",
        tool, stage_id
    )
}

/// 防御 B · split a batch into (submit_stage_deliverable calls, everything else).
/// Pure for unit testing the lock partition independent of dispatch wiring.
fn split_for_submit_only(calls: Vec<ToolCall>) -> (Vec<ToolCall>, Vec<ToolCall>) {
    calls
        .into_iter()
        .partition(|tc| tc.function.name == SUBMIT_STAGE_DELIVERABLE_TOOL)
}

fn split_for_forced_tool(
    calls: Vec<ToolCall>,
    forced_tool: &str,
) -> (Vec<ToolCall>, Vec<ToolCall>) {
    calls
        .into_iter()
        .partition(|tc| tc.function.name == forced_tool)
}

/// 防御 B · push one corrective `ToolResult` per blocked call so the assistant
/// tool_call ↔ tool_result pairing stays intact and the model is told plainly
/// that only the submission is permitted this turn.
fn push_submit_only_rejections(chat_history: &mut Vec<Message>, blocked: &[ToolCall]) {
    let results: Vec<UserContent> = blocked
        .iter()
        .map(|tc| {
            UserContent::ToolResult(ToolResult {
                id: tc.id.clone(),
                call_id: Some(tc.id.clone()),
                content: OneOrMany::one(ToolResultContent::Text(Text {
                    text: format!(
                        "Error: '{}' is not allowed right now. This stage's scan work is already \
                         done and its evidence is recorded. You are in SUBMIT-ONLY mode: the only \
                         permitted action is calling `{}` once with the real evidence ids. Call \
                         `{}` now.",
                        tc.function.name,
                        SUBMIT_STAGE_DELIVERABLE_TOOL,
                        SUBMIT_STAGE_DELIVERABLE_TOOL
                    ),
                })),
            })
        })
        .collect();

    if let Ok(content) = OneOrMany::many(results) {
        chat_history.push(Message::User { content });
    }
}

fn push_forced_tool_rejections(
    chat_history: &mut Vec<Message>,
    blocked: &[ToolCall],
    forced_tool: &str,
) {
    let results: Vec<UserContent> = blocked
        .iter()
        .map(|tc| {
            UserContent::ToolResult(ToolResult {
                id: tc.id.clone(),
                call_id: Some(tc.id.clone()),
                content: OneOrMany::one(ToolResultContent::Text(Text {
                    text: format!(
                        "Error: '{}' is not allowed right now. This is a deterministic resume \
                         turn; the only permitted action is calling `{}` once. Call `{}` now.",
                        tc.function.name, forced_tool, forced_tool
                    ),
                })),
            })
        })
        .collect();

    if let Ok(content) = OneOrMany::many(results) {
        chat_history.push(Message::User { content });
    }
}

fn truncate_for_log(value: &str) -> String {
    const MAX: usize = 160;
    if value.chars().count() <= MAX {
        return value.to_string();
    }
    value.chars().take(MAX).collect::<String>() + "..."
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use golish_llm_providers::LlmClient;
    use rig::message::{ToolFunction, UserContent};
    use serde_json::json;
    use tokio::sync::RwLock;

    use crate::test_utils::{MockCompletionModel, TestContextBuilder};

    use super::*;

    fn make_tool_call(id: &str, name: &str) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            call_id: Some(id.to_string()),
            function: ToolFunction {
                name: name.to_string(),
                arguments: json!({}),
            },
            signature: None,
            additional_params: None,
        }
    }

    fn tool_def(name: &str) -> rig::completion::ToolDefinition {
        rig::completion::ToolDefinition {
            name: name.to_string(),
            description: format!("Mock {name} tool"),
            parameters: json!({ "type": "object", "properties": {} }),
        }
    }

    #[test]
    fn textual_ask_human_is_classified_as_human_barrier() {
        let call = make_tool_call("textual-tool-call-1-0", "ask_human");
        let decision = gate_tool_call_for_dispatch(&call, None, None);
        assert!(matches!(
            decision,
            ToolGateDecision::RequireHumanAnswer { .. }
        ));
    }

    #[test]
    fn native_tool_call_passes_gate() {
        let call = make_tool_call("tc-1", "read_file");
        let decision = gate_tool_call_for_dispatch(&call, None, None);
        assert_eq!(decision, ToolGateDecision::Allow);
    }

    #[test]
    fn harness_stage_rejects_forbidden_tool() {
        use golish_agent_kit::harness::StageKind;
        // metasploit (exploit/framework) is not in external_attack_surface's
        // allowed_tool_types → rejected (deny-by-default).
        let call = make_tool_call("tc-9", "metasploit");
        let decision =
            gate_tool_call_for_dispatch(&call, Some(StageKind::ExternalAttackSurface), None);
        assert!(matches!(decision, ToolGateDecision::Reject { .. }));
    }

    #[test]
    fn harness_stage_allows_non_forbidden_tool() {
        use golish_agent_kit::harness::StageKind;
        // sub_agent_* / orchestration tools are not forbidden → pass barrier.
        let call = make_tool_call("tc-10", "sub_agent_pentester");
        let decision =
            gate_tool_call_for_dispatch(&call, Some(StageKind::ExternalAttackSurface), None);
        assert_eq!(decision, ToolGateDecision::Allow);
    }

    #[test]
    fn harness_authz_rejects_scan_tool_not_in_allowed_types() {
        use golish_agent_kit::harness::{AuthorizationLevel, HarnessAuthz, IntentAxis, StageKind};
        // A scan tool whose type is NOT in the stage's allowed_tool_types is
        // rejected (deny-by-default). sqlmap (web/injection) is not allowed in
        // external_attack_surface.
        let call = make_tool_call("tc-11", "sqlmap");
        let authz = HarnessAuthz {
            max_authorization: AuthorizationLevel::ActiveRecon,
            intent: IntentAxis::PassiveObserve,
        };
        let decision =
            gate_tool_call_for_dispatch(&call, Some(StageKind::ExternalAttackSurface), Some(authz));
        assert!(matches!(decision, ToolGateDecision::Reject { .. }));
    }

    #[test]
    fn harness_authz_allows_scan_tool_in_allowed_types_within_ceiling() {
        use golish_agent_kit::harness::{AuthorizationLevel, HarnessAuthz, IntentAxis, StageKind};
        // httpx resolves to recon/http, which is in external_attack_surface
        // allowed_tool_types (阶段重排 2026-06-09: dns moved out of EAS, so dig no
        // longer qualifies here); PassiveObserve intent is within the assessment
        // ceiling (ActiveRecon) → Allow.
        let call = make_tool_call("tc-12", "httpx");
        let authz = HarnessAuthz {
            max_authorization: AuthorizationLevel::ActiveRecon,
            intent: IntentAxis::PassiveObserve,
        };
        let decision =
            gate_tool_call_for_dispatch(&call, Some(StageKind::ExternalAttackSurface), Some(authz));
        assert_eq!(decision, ToolGateDecision::Allow);
    }

    #[test]
    fn harness_authz_rejects_intent_above_ceiling() {
        use golish_agent_kit::harness::{AuthorizationLevel, HarnessAuthz, IntentAxis, StageKind};
        // httpx (recon/http) is allowed in eas (so confinement passes), but
        // ExploitValidation intent exceeds the assessment ceiling (ActiveRecon)
        // → reject specifically on authorization.
        let call = make_tool_call("tc-13", "httpx");
        let authz = HarnessAuthz {
            max_authorization: AuthorizationLevel::ActiveRecon,
            intent: IntentAxis::ExploitValidation,
        };
        let decision =
            gate_tool_call_for_dispatch(&call, Some(StageKind::ExternalAttackSurface), Some(authz));
        assert!(matches!(decision, ToolGateDecision::Reject { .. }));
    }

    #[test]
    fn harness_authz_exempts_orchestration_from_allowed_confinement() {
        use golish_agent_kit::harness::{AuthorizationLevel, HarnessAuthz, IntentAxis, StageKind};
        // sub_agent_* is not in allowed_tools, but orchestration delegation is
        // exempt from confinement even when an authz context is present.
        let call = make_tool_call("tc-14", "sub_agent_pentester");
        let authz = HarnessAuthz {
            max_authorization: AuthorizationLevel::ActiveRecon,
            intent: IntentAxis::PassiveObserve,
        };
        let decision =
            gate_tool_call_for_dispatch(&call, Some(StageKind::ExternalAttackSurface), Some(authz));
        assert_eq!(decision, ToolGateDecision::Allow);
    }

    #[tokio::test]
    async fn empty_inputs_are_a_noop() {
        let test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);
        let capture = test_ctx.create_capture_context();
        let model = MockCompletionModel::with_text("ignored");
        let sub_ctx = SubAgentContext::default();
        let registry = HookRegistry::new();
        let mut history: Vec<Message> = vec![];

        run(
            vec![],
            &[],
            &ctx,
            &capture,
            &model,
            &sub_ctx,
            &registry,
            &Span::none(),
            &mut history,
            false,
            None,
        )
        .await;

        assert!(history.is_empty(), "no tool calls => no history mutation");
    }

    #[tokio::test]
    async fn calls_outside_allow_list_push_unavailable_error_results() {
        let test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);
        let capture = test_ctx.create_capture_context();
        let model = MockCompletionModel::with_text("ignored");
        let sub_ctx = SubAgentContext::default();
        let registry = HookRegistry::new();
        let mut history: Vec<Message> = vec![];

        run(
            vec![
                make_tool_call("tc-1", "blocked_tool"),
                make_tool_call("tc-2", "another_blocked"),
            ],
            &[],
            &ctx,
            &capture,
            &model,
            &sub_ctx,
            &registry,
            &Span::none(),
            &mut history,
            false,
            None,
        )
        .await;

        assert_eq!(
            history.len(),
            1,
            "rejected tool calls produce exactly one User message containing tool results"
        );
        let Message::User { content } = &history[0] else {
            panic!("expected User message holding ToolResult content");
        };
        let tool_result_count = content
            .iter()
            .filter(|c| matches!(c, UserContent::ToolResult(_)))
            .count();
        assert_eq!(
            tool_result_count, 2,
            "both rejected calls must surface as ToolResult error entries"
        );
    }

    #[tokio::test]
    async fn textual_ask_human_emits_tool_intent_observation_before_filtering() {
        let mut test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);
        let capture = test_ctx.create_capture_context();
        let model = MockCompletionModel::with_text("ignored");
        let sub_ctx = SubAgentContext::default();
        let registry = HookRegistry::new();
        let mut history: Vec<Message> = vec![];

        run(
            vec![make_tool_call("textual-tool-call-4-0", "ask_human")],
            &[],
            &ctx,
            &capture,
            &model,
            &sub_ctx,
            &registry,
            &Span::none(),
            &mut history,
            false,
            None,
        )
        .await;

        let events = test_ctx.collect_events();
        assert!(
            events.iter().any(|event| {
                matches!(
                    event,
                    AiEvent::ToolIntentObservation {
                        request_id,
                        tool_name,
                        source,
                        decision,
                        ..
                    } if request_id == "textual-tool-call-4-0"
                        && tool_name == "ask_human"
                        && source == "textual_xml"
                        && decision == "require_human_answer"
                )
            }),
            "recovered ask_human intent should be observable before allow-list filtering"
        );
    }

    // ── 设计 2026-06-12 (submit-only-lock-hardening 防御 B) ──────────────────

    #[test]
    fn split_for_submit_only_partitions_submit_vs_rest() {
        let (submit, blocked) = split_for_submit_only(vec![
            make_tool_call("tc-1", "update_plan"),
            make_tool_call("tc-2", "submit_stage_deliverable"),
            make_tool_call("tc-3", "sub_agent_pentester"),
        ]);
        assert_eq!(submit.len(), 1, "only the submit call is kept");
        assert_eq!(submit[0].function.name, "submit_stage_deliverable");
        assert_eq!(blocked.len(), 2, "non-submit calls are blocked");
    }

    #[test]
    fn split_for_forced_tool_partitions_named_tool_vs_rest() {
        let (allowed, blocked) = split_for_forced_tool(
            vec![
                make_tool_call("tc-1", "update_plan"),
                make_tool_call("tc-2", "stage_run"),
                make_tool_call("tc-3", "list_in_scope_targets"),
            ],
            "stage_run",
        );
        assert_eq!(allowed.len(), 1, "only the forced tool is kept");
        assert_eq!(allowed[0].function.name, "stage_run");
        assert_eq!(blocked.len(), 2, "non-forced calls are blocked");
    }

    #[tokio::test]
    async fn submit_only_lock_refuses_non_submit_even_when_allow_listed() {
        // update_plan IS a normally-allowed orchestration tool, but the
        // submit-only lock must override the allow-list and refuse it with the
        // targeted message (not the generic "not available" one).
        let test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);
        let capture = test_ctx.create_capture_context();
        let model = MockCompletionModel::with_text("ignored");
        let sub_ctx = SubAgentContext::default();
        let registry = HookRegistry::new();
        let mut history: Vec<Message> = vec![];
        let tools = [tool_def("update_plan")];

        run(
            vec![make_tool_call("textual-tool-call-1-0", "update_plan")],
            &tools,
            &ctx,
            &capture,
            &model,
            &sub_ctx,
            &registry,
            &Span::none(),
            &mut history,
            true,
            None,
        )
        .await;

        assert_eq!(history.len(), 1, "the blocked call gets one tool-result");
        let Message::User { content } = &history[0] else {
            panic!("expected User message holding ToolResult content");
        };
        let text = content
            .iter()
            .find_map(|c| match c {
                UserContent::ToolResult(tr) => tr.content.iter().find_map(|rc| match rc {
                    ToolResultContent::Text(t) => Some(t.text.clone()),
                    _ => None,
                }),
                _ => None,
            })
            .expect("a textual tool-result");
        assert!(
            text.contains("SUBMIT-ONLY"),
            "rejection must use the submit-only message, got: {text}"
        );
        assert!(
            text.contains("submit_stage_deliverable"),
            "rejection must name the submit tool"
        );
    }

    #[tokio::test]
    async fn submit_only_lock_does_not_refuse_the_submit_call() {
        // With the lock on, the submit call must survive the lock partition
        // (it's the one permitted action). Allow-list is left empty so the call
        // does not reach a real executor; we only assert the lock itself did not
        // push a SUBMIT-ONLY rejection for it.
        let test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);
        let capture = test_ctx.create_capture_context();
        let model = MockCompletionModel::with_text("ignored");
        let sub_ctx = SubAgentContext::default();
        let registry = HookRegistry::new();
        let mut history: Vec<Message> = vec![];

        run(
            vec![make_tool_call("tc-1", "submit_stage_deliverable")],
            &[],
            &ctx,
            &capture,
            &model,
            &sub_ctx,
            &registry,
            &Span::none(),
            &mut history,
            true,
            None,
        )
        .await;

        let pushed_submit_only_rejection = history.iter().any(|m| match m {
            Message::User { content } => content.iter().any(|c| match c {
                UserContent::ToolResult(tr) => tr.content.iter().any(|rc| match rc {
                    ToolResultContent::Text(t) => t.text.contains("SUBMIT-ONLY"),
                    _ => false,
                }),
                _ => false,
            }),
            _ => false,
        });
        assert!(
            !pushed_submit_only_rejection,
            "the submit call must not be refused by the lock"
        );
    }

    #[tokio::test]
    async fn forced_tool_lock_refuses_other_allow_listed_tools() {
        let test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);
        let capture = test_ctx.create_capture_context();
        let model = MockCompletionModel::with_text("ignored");
        let sub_ctx = SubAgentContext::default();
        let registry = HookRegistry::new();
        let mut history: Vec<Message> = vec![];
        let tools = [tool_def("update_plan")];

        run(
            vec![make_tool_call("textual-tool-call-1-0", "update_plan")],
            &tools,
            &ctx,
            &capture,
            &model,
            &sub_ctx,
            &registry,
            &Span::none(),
            &mut history,
            false,
            Some("stage_run"),
        )
        .await;

        assert_eq!(history.len(), 1, "the blocked call gets one tool-result");
        let Message::User { content } = &history[0] else {
            panic!("expected User message holding ToolResult content");
        };
        let text = content
            .iter()
            .find_map(|c| match c {
                UserContent::ToolResult(tr) => tr.content.iter().find_map(|rc| match rc {
                    ToolResultContent::Text(t) => Some(t.text.clone()),
                    _ => None,
                }),
                _ => None,
            })
            .expect("a textual tool-result");
        assert!(
            text.contains("deterministic resume"),
            "rejection must use the forced-tool message, got: {text}"
        );
        assert!(
            text.contains("stage_run"),
            "rejection must name the forced tool"
        );
    }
}
