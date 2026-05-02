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
        let rejected_names: Vec<&str> =
            rejected.iter().map(|tc| tc.function.name.as_str()).collect();
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
