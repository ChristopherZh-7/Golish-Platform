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
use golish_sub_agents::SubAgentContext;

use super::super::super::context::{AgenticLoopContext, LoopCaptureContext};
use super::super::super::tool_dispatch::dispatch_tool_calls;
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
    let allowed_names: std::collections::HashSet<&str> =
        tools.iter().map(|t| t.name.as_str()).collect();
    let (permitted, rejected): (Vec<_>, Vec<_>) = tool_calls_to_execute
        .into_iter()
        .partition(|tc| allowed_names.contains(tc.function.name.as_str()));

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
}
