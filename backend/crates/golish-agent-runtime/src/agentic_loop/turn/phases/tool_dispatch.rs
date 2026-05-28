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
use rig::message::ToolCall;
use tracing::Span;

use golish_agent_kit::system_hooks::HookRegistry;
use golish_agent_kit::tool_policy::PolicyConstraintResult;
use golish_core::events::AiEvent;
use golish_sub_agents::SubAgentContext;

use super::super::super::context::{AgenticLoopContext, LoopCaptureContext};
use super::super::super::tool_dispatch::dispatch_tool_calls;
use super::super::super::tool_gate::{decide_tool_intent, ToolGateDecision};
use super::super::super::tool_intent::{ToolIntent, ToolIntentSource};
use super::super::super::unified_helpers::push_unavailable_tool_results;

/// Partition the LLM-issued tool calls into permitted/rejected, surface
/// errors for the rejected ones, and dispatch the permitted batch.
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
) where
    M: rig::completion::CompletionModel + Sync,
{
    let mut gated_tool_calls = Vec::new();
    let mut gate_rejected = Vec::new();
    for tc in tool_calls_to_execute {
        let decision = gate_tool_call_for_dispatch(&tc);
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
        dispatch_tool_calls(
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

fn gate_tool_call_for_dispatch(tool_call: &ToolCall) -> ToolGateDecision {
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

    #[test]
    fn textual_ask_human_is_classified_as_human_barrier() {
        let call = make_tool_call("textual-tool-call-1-0", "ask_human");
        let decision = gate_tool_call_for_dispatch(&call);
        assert!(matches!(
            decision,
            ToolGateDecision::RequireHumanAnswer { .. }
        ));
    }

    #[test]
    fn native_tool_call_passes_gate() {
        let call = make_tool_call("tc-1", "read_file");
        let decision = gate_tool_call_for_dispatch(&call);
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
}
