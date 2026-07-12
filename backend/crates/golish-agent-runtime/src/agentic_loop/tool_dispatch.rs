//! Dispatch a batch of tool calls produced by one assistant turn and append the
//! collected tool-result user message to the chat history.
//!
//! Sub-agent calls run concurrently when there are >= 2 of them (no spawn
//! overhead for single calls); other tool calls always run sequentially.
//! System hooks emitted by individual tool executions are merged into the same
//! tool-results user message to avoid "user after tool" ordering violations
//! with OpenAI-compatible APIs.

use rig::completion::Message;
use rig::message::{Text, ToolCall, ToolResult, ToolResultContent, UserContent};
use rig::one_or_many::OneOrMany;

use golish_core::events::AiEvent;
use golish_sub_agents::SubAgentContext;

use super::context::{is_cancelled, AgenticLoopContext, LoopCaptureContext};
use super::single_tool_call::execute_single_tool_call;
use super::sub_agent_dispatch::partition_tool_calls;
use golish_agent_kit::system_hooks::{format_system_hooks, HookRegistry};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ToolDispatchOutcome {
    pub stage_submission_accepted: bool,
}

fn tool_result_has_json_status(content: &UserContent, expected: &str) -> bool {
    let UserContent::ToolResult(result) = content else {
        return false;
    };
    result.content.iter().any(|content| {
        let ToolResultContent::Text(text) = content else {
            return false;
        };
        serde_json::Deserializer::from_str(&text.text)
            .into_iter::<serde_json::Value>()
            .next()
            .and_then(Result::ok)
            .and_then(|value| {
                value
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .as_deref()
            == Some(expected)
    })
}

fn paired_synthetic_tool_result(tool_call: &ToolCall, value: serde_json::Value) -> UserContent {
    UserContent::ToolResult(ToolResult {
        id: tool_call.id.clone(),
        call_id: Some(
            tool_call
                .call_id
                .clone()
                .unwrap_or_else(|| tool_call.id.clone()),
        ),
        content: OneOrMany::one(ToolResultContent::Text(Text {
            text: value.to_string(),
        })),
    })
}

fn stage_submission_barrier_result(tool_call: &ToolCall) -> (UserContent, Vec<String>) {
    (
        paired_synthetic_tool_result(
            tool_call,
            serde_json::json!({
                "status": "skipped",
                "blocked_by_stage_submission": true,
                "error": "Skipped without execution because an earlier submit_stage_deliverable in this tool batch was accepted."
            }),
        ),
        Vec::new(),
    )
}

fn cancelled_batch_tool_result(tool_call: &ToolCall) -> (UserContent, Vec<String>) {
    (
        paired_synthetic_tool_result(
            tool_call,
            serde_json::json!({
                "status": "cancelled",
                "cancelled": true,
                "error": "Skipped without execution because the agent was cancelled."
            }),
        ),
        Vec::new(),
    )
}

/// Execute a batch containing a stage submission in original assistant order.
/// Once that submission returns accepted, every later call receives a paired
/// synthetic ToolResult and is never dispatched. This is intentionally separate
/// from ordinary sub-agent concurrency: a terminal barrier and speculative
/// parallel work cannot safely coexist in the same assistant batch.
async fn dispatch_submit_barrier_batch<F, Fut>(
    calls: Vec<(usize, ToolCall)>,
    mut execute: F,
) -> (Vec<(usize, (UserContent, Vec<String>))>, bool)
where
    F: FnMut(ToolCall) -> Fut,
    Fut: std::future::Future<Output = (UserContent, Vec<String>)>,
{
    let mut results = Vec::with_capacity(calls.len());
    let mut accepted = false;
    for (index, tool_call) in calls {
        if accepted {
            results.push((index, stage_submission_barrier_result(&tool_call)));
            continue;
        }
        let is_submission = tool_call.function.name == "submit_stage_deliverable";
        let result = execute(tool_call).await;
        if is_submission && tool_result_has_json_status(&result.0, "accepted") {
            accepted = true;
        }
        results.push((index, result));
    }
    (results, accepted)
}

/// Run all `tool_calls_to_execute` and append the resulting user message
/// (tool results + any merged system hooks) to `chat_history`.
pub(crate) async fn dispatch_tool_calls<M>(
    tool_calls_to_execute: Vec<ToolCall>,
    ctx: &AgenticLoopContext<'_>,
    capture_ctx: &LoopCaptureContext,
    model: &M,
    sub_agent_context: &SubAgentContext,
    hook_registry: &HookRegistry,
    llm_span: &tracing::Span,
    chat_history: &mut Vec<Message>,
) -> ToolDispatchOutcome
where
    M: rig::completion::CompletionModel + Sync,
{
    let total_tool_count = tool_calls_to_execute.len();
    let has_stage_submission = tool_calls_to_execute
        .iter()
        .any(|call| call.function.name == "submit_stage_deliverable");

    let mut indexed_results: Vec<Option<(UserContent, Vec<String>)>> = vec![None; total_tool_count];
    let stage_submission_accepted = if has_stage_submission {
        let indexed_calls = tool_calls_to_execute.into_iter().enumerate().collect();
        let (results, accepted) =
            dispatch_submit_barrier_batch(indexed_calls, |tool_call| async move {
                if is_cancelled(ctx) {
                    tracing::info!(
                        "Agent cancelled before tool execution: {}",
                        tool_call.function.name
                    );
                    cancelled_batch_tool_result(&tool_call)
                } else {
                    execute_single_tool_call(
                        tool_call,
                        ctx,
                        capture_ctx,
                        model,
                        sub_agent_context,
                        hook_registry,
                        llm_span,
                    )
                    .await
                }
            })
            .await;
        for (index, result) in results {
            indexed_results[index] = Some(result);
        }
        accepted
    } else {
        let (sub_agent_calls, other_calls) = partition_tool_calls(tool_calls_to_execute);
        let has_concurrent_sub_agents = sub_agent_calls.len() >= 2;

        if has_concurrent_sub_agents {
            tracing::info!(
                count = sub_agent_calls.len(),
                "Executing sub-agent tool calls concurrently"
            );

            let futures: Vec<_> = sub_agent_calls
                .into_iter()
                .map(|(original_idx, tool_call)| async move {
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
        false
    };
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
    ToolDispatchOutcome {
        stage_submission_accepted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::message::{Text, ToolFunction, ToolResult};

    fn tool_call(id: &str, name: &str) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            call_id: Some(format!("provider-{id}")),
            function: ToolFunction {
                name: name.to_string(),
                arguments: serde_json::json!({}),
            },
            signature: None,
            additional_params: None,
        }
    }

    fn fake_result(call: &ToolCall, status: &str) -> (UserContent, Vec<String>) {
        (
            UserContent::ToolResult(ToolResult {
                id: call.id.clone(),
                call_id: call.call_id.clone(),
                content: OneOrMany::one(ToolResultContent::Text(Text {
                    text: serde_json::json!({"status": status}).to_string(),
                })),
            }),
            Vec::new(),
        )
    }

    #[test]
    fn accepted_submit_status_is_read_from_json_before_runtime_notes() {
        let content = UserContent::ToolResult(ToolResult {
            id: "call-1".to_string(),
            call_id: Some("provider-call-1".to_string()),
            content: OneOrMany::one(ToolResultContent::Text(Text {
                text: "{\"status\":\"accepted\",\"note\":\"ok\"}\n\n--- RUNTIME NOTE ---"
                    .to_string(),
            })),
        });
        assert!(tool_result_has_json_status(&content, "accepted"));
        assert!(!tool_result_has_json_status(&content, "needs_fix"));
    }

    #[tokio::test]
    async fn accepted_submit_short_circuits_later_batch_calls_with_paired_results() {
        let calls = vec![
            (0, tool_call("before", "update_plan")),
            (1, tool_call("submit", "submit_stage_deliverable")),
            (2, tool_call("after", "manage_targets")),
        ];
        let executed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = std::sync::Arc::clone(&executed);

        let (results, accepted) = dispatch_submit_barrier_batch(calls, move |call| {
            observed.lock().unwrap().push(call.id.clone());
            let status = if call.function.name == "submit_stage_deliverable" {
                "accepted"
            } else {
                "ok"
            };
            async move { fake_result(&call, status) }
        })
        .await;

        assert!(accepted);
        assert_eq!(
            executed.lock().unwrap().as_slice(),
            &["before".to_string(), "submit".to_string()]
        );
        assert_eq!(
            results.len(),
            3,
            "every assistant call needs one ToolResult"
        );
        let (index, (skipped, hooks)) = &results[2];
        assert_eq!(*index, 2);
        assert!(hooks.is_empty());
        let UserContent::ToolResult(skipped) = skipped else {
            panic!("skipped call must be paired with a ToolResult")
        };
        assert_eq!(skipped.id, "after");
        assert_eq!(skipped.call_id.as_deref(), Some("provider-after"));
        let ToolResultContent::Text(text) = skipped.content.first() else {
            panic!("skipped ToolResult must carry JSON text")
        };
        let value: serde_json::Value = serde_json::from_str(&text.text).unwrap();
        assert_eq!(value["blocked_by_stage_submission"], true);
        assert_eq!(value["status"], "skipped");
    }
}
