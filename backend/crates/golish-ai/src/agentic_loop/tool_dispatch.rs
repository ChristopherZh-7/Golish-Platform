//! Dispatch a batch of tool calls produced by one assistant turn and append the
//! collected tool-result user message to the chat history.
//!
//! Sub-agent calls run concurrently when there are >= 2 of them (no spawn
//! overhead for single calls); other tool calls always run sequentially.
//! System hooks emitted by individual tool executions are merged into the same
//! tool-results user message to avoid "user after tool" ordering violations
//! with OpenAI-compatible APIs.

use rig::completion::Message;
use rig::message::{Text, ToolCall, UserContent};
use rig::one_or_many::OneOrMany;

use golish_core::events::AiEvent;
use golish_sub_agents::SubAgentContext;

use super::context::{is_cancelled, AgenticLoopContext, LoopCaptureContext};
use super::single_tool_call::execute_single_tool_call;
use super::sub_agent_dispatch::partition_tool_calls;
use super::super::system_hooks::{format_system_hooks, HookRegistry};

/// Run all `tool_calls_to_execute` and append the resulting user message
/// (tool results + any merged system hooks) to `chat_history`.
pub(super) async fn dispatch_tool_calls<M>(
    tool_calls_to_execute: Vec<ToolCall>,
    ctx: &AgenticLoopContext<'_>,
    capture_ctx: &LoopCaptureContext,
    model: &M,
    sub_agent_context: &SubAgentContext,
    hook_registry: &HookRegistry,
    llm_span: &tracing::Span,
    chat_history: &mut Vec<Message>,
) where
    M: rig::completion::CompletionModel + Sync,
{
    let total_tool_count = tool_calls_to_execute.len();
    let (sub_agent_calls, other_calls) = partition_tool_calls(tool_calls_to_execute);
    let has_concurrent_sub_agents = sub_agent_calls.len() >= 2;

    let mut indexed_results: Vec<Option<(UserContent, Vec<String>)>> =
        vec![None; total_tool_count];

    if has_concurrent_sub_agents {
        tracing::info!(
            count = sub_agent_calls.len(),
            "Executing sub-agent tool calls concurrently"
        );

        let futures: Vec<_> = sub_agent_calls
            .into_iter()
            .map(|(original_idx, tool_call)| {
                let llm_span = llm_span;
                let capture_ctx = capture_ctx;
                let sub_agent_context = sub_agent_context;
                let hook_registry = hook_registry;
                async move {
                    let result = execute_single_tool_call(
                        tool_call,
                        ctx,
                        capture_ctx,
                        model,
                        sub_agent_context,
                        hook_registry,
                        llm_span,
                    )
                    .await;
                    (original_idx, result)
                }
            })
            .collect();

        let concurrent_results = futures::future::join_all(futures).await;
        for (idx, result) in concurrent_results {
            indexed_results[idx] = Some(result);
        }
    } else {
        // 0 or 1 sub-agent calls — execute sequentially (no spawn overhead)
        for (original_idx, tool_call) in sub_agent_calls {
            if is_cancelled(ctx) {
                tracing::info!(
                    "Agent cancelled before sub-agent call: {}",
                    tool_call.function.name
                );
                break;
            }
            let result = execute_single_tool_call(
                tool_call,
                ctx,
                capture_ctx,
                model,
                sub_agent_context,
                hook_registry,
                llm_span,
            )
            .await;
            indexed_results[original_idx] = Some(result);
        }
    }

    for (original_idx, tool_call) in other_calls {
        if is_cancelled(ctx) {
            tracing::info!(
                "Agent cancelled before tool execution: {}",
                tool_call.function.name
            );
            break;
        }
        let result = execute_single_tool_call(
            tool_call,
            ctx,
            capture_ctx,
            model,
            sub_agent_context,
            hook_registry,
            llm_span,
        )
        .await;
        indexed_results[original_idx] = Some(result);
    }

    let mut tool_results: Vec<UserContent> = Vec::with_capacity(total_tool_count);
    let mut system_hooks: Vec<String> = vec![];
    for (user_content, hooks) in indexed_results.into_iter().flatten() {
        tool_results.push(user_content);
        system_hooks.extend(hooks);
    }

    if !system_hooks.is_empty() {
        let formatted_hooks = format_system_hooks(&system_hooks);

        tracing::info!(
            count = system_hooks.len(),
            content_len = formatted_hooks.len(),
            "Injecting system hooks into tool results message"
        );

        let _ = ctx.events.event_tx.send(AiEvent::SystemHooksInjected {
            hooks: system_hooks.clone(),
        });

        let _system_hook_event = tracing::info_span!(
            parent: llm_span,
            "system_hooks_injected",
            "langfuse.observation.type" = "event",
            "langfuse.observation.level" = "DEFAULT",
            "langfuse.session.id" = ctx.events.session_id.unwrap_or(""),
            hook_count = system_hooks.len(),
            "langfuse.observation.input" = %formatted_hooks,
        );

        tool_results.push(UserContent::Text(Text {
            text: formatted_hooks,
        }));
    }

    chat_history.push(Message::User {
        content: OneOrMany::many(tool_results).unwrap_or_else(|_| {
            OneOrMany::one(UserContent::Text(Text {
                text: "Tool executed".to_string(),
            }))
        }),
    });
}
