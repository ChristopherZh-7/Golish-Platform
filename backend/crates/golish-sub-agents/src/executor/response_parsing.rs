//! Tool call dispatch and response parsing for sub-agent execution.
//!
//! Extracts the tool execution loop from the main orchestrator, handling
//! barrier tools, nested sub-agent delegation, regular tool execution,
//! event emission, and file modification tracking.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rig::completion::CompletionModel as RigCompletionModel;
use rig::message::{Text, ToolCall, ToolResult, ToolResultContent, UserContent};
use rig::one_or_many::OneOrMany;
use uuid::Uuid;

use crate::definition::{SubAgentContext, SubAgentDefinition};
use crate::executor_helpers::{epoch_secs, extract_file_path, is_write_tool};
use crate::executor_types::{SubAgentExecutorContext, ToolProvider, BARRIER_TOOL_NAME};
use crate::transcript::SubAgentTranscriptWriter;
use golish_core::events::{AiEvent, ToolSource};
use golish_core::utils::truncate_str;

/// Result of dispatching tool calls within a sub-agent iteration.
pub(super) struct ToolDispatchResult {
    pub tool_results: Vec<UserContent>,
    pub barrier_hit: bool,
    /// When the barrier tool is hit, this holds the response text.
    pub barrier_response: Option<String>,
}

/// Dispatch and execute a batch of tool calls from a sub-agent iteration.
///
/// Handles three categories of tool calls:
/// 1. **Barrier tool** — captures the structured result and signals loop termination.
/// 2. **Nested delegation** (`sub_agent_*`) — dispatches to child sub-agents.
/// 3. **Regular tools** — executed via the tool registry with timeout protection.
///
/// Emits `SubAgentToolRequest` / `SubAgentToolResult` events, writes to the
/// transcript, runs the post-shell hook, and tracks file modifications.
#[allow(clippy::too_many_arguments)]
pub(super) async fn dispatch_tool_calls<M, P>(
    tool_calls: Vec<ToolCall>,
    agent_def: &SubAgentDefinition,
    sub_context: &SubAgentContext,
    ctx: &SubAgentExecutorContext<'_>,
    tool_provider: &P,
    model: &M,
    parent_request_id: &str,
    last_activity: &Arc<AtomicU64>,
    timeout_duration: Duration,
    idle_timeout: Option<Duration>,
    transcript_writer: &Option<Arc<SubAgentTranscriptWriter>>,
    files_modified: &mut Vec<String>,
    llm_span: &tracing::Span,
) -> ToolDispatchResult
where
    M: RigCompletionModel + Sync,
    P: ToolProvider,
{
    let agent_id = &agent_def.id;
    let mut tool_results: Vec<UserContent> = vec![];
    let mut barrier_hit = false;
    let mut barrier_response: Option<String> = None;

    for tool_call in tool_calls {
        let tool_name = &tool_call.function.name;

        // ── Barrier tool ────────────────────────────────────────────────
        if tool_name == BARRIER_TOOL_NAME {
            let args = &tool_call.function.arguments;
            let result_text = args
                .get("result")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let summary = args
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            tracing::info!(
                "[sub-agent] Barrier tool '{}' called: summary='{}', result_len={}",
                BARRIER_TOOL_NAME,
                summary,
                result_text.len()
            );

            barrier_response = Some(if result_text.is_empty() {
                summary.to_string()
            } else {
                result_text
            });

            let _ = ctx.event_tx.send(AiEvent::SubAgentToolResult {
                agent_id: agent_id.to_string(),
                tool_name: BARRIER_TOOL_NAME.to_string(),
                success: true,
                result: serde_json::json!({ "status": "result submitted" }),
                request_id: Uuid::new_v4().to_string(),
                parent_request_id: parent_request_id.to_string(),
            });

            barrier_hit = true;
            break;
        }

        // ── Nested delegation ───────────────────────────────────────────
        if tool_name.starts_with("sub_agent_") {
            let delegate_id = &tool_name["sub_agent_".len()..];
            let delegate_task = tool_call
                .function
                .arguments
                .get("task")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            tracing::info!(
                "[sub-agent:{}] Nested delegation to '{}': {}",
                agent_id,
                delegate_id,
                truncate_str(&delegate_task, 100)
            );

            let delegate_result = if let Some(registry) = ctx.sub_agent_registry {
                let reg = registry.read().await;
                if let Some(delegate_def) = reg.get(delegate_id) {
                    let delegate_def = delegate_def.clone();
                    drop(reg);
                    let nested_ctx = SubAgentExecutorContext {
                        event_tx: ctx.event_tx,
                        tool_registry: ctx.tool_registry,
                        workspace: ctx.workspace,
                        provider_name: ctx.provider_name,
                        model_name: ctx.model_name,
                        session_id: ctx.session_id,
                        transcript_base_dir: ctx.transcript_base_dir,
                        api_request_stats: ctx.api_request_stats,
                        briefing: None,
                        temperature_override: delegate_def.temperature,
                        max_tokens_override: delegate_def.max_tokens,
                        top_p_override: delegate_def.top_p,
                        chain_persistence: ctx.chain_persistence,
                        sub_agent_registry: ctx.sub_agent_registry,
                        post_shell_hook: ctx.post_shell_hook.clone(),
                    };
                    match Box::pin(super::execute_sub_agent(
                        &delegate_def,
                        &tool_call.function.arguments,
                        sub_context,
                        model,
                        nested_ctx,
                        tool_provider,
                        parent_request_id,
                    ))
                    .await
                    {
                        Ok(result) => serde_json::json!({
                            "success": result.success,
                            "response": result.response,
                        }),
                        Err(e) => serde_json::json!({
                            "success": false,
                            "error": e.to_string(),
                        }),
                    }
                } else {
                    serde_json::json!({
                        "error": format!("Unknown delegate agent: {}", delegate_id),
                    })
                }
            } else {
                serde_json::json!({
                    "error": "Sub-agent registry not available for nested delegation",
                })
            };

            let tool_id = tool_call.id.clone();
            let tool_call_id = tool_call
                .call_id
                .clone()
                .unwrap_or_else(|| tool_call.id.clone());
            let result_text = serde_json::to_string(&delegate_result).unwrap_or_default();
            tool_results.push(UserContent::ToolResult(ToolResult {
                id: tool_id,
                call_id: Some(tool_call_id),
                content: OneOrMany::one(ToolResultContent::Text(Text { text: result_text })),
            }));

            last_activity.store(epoch_secs(), Ordering::Relaxed);
            continue;
        }

        // ── Regular tool execution ──────────────────────────────────────
        let tool_args = if tool_name == "run_pty_cmd" {
            tool_provider.normalize_run_pty_cmd_args(tool_call.function.arguments.clone())
        } else {
            tool_call.function.arguments.clone()
        };
        let tool_id = tool_call.id.clone();
        let tool_call_id = tool_call
            .call_id
            .clone()
            .unwrap_or_else(|| tool_call.id.clone());

        let request_id = Uuid::new_v4().to_string();
        let tool_request_event = AiEvent::SubAgentToolRequest {
            agent_id: agent_id.to_string(),
            tool_name: tool_name.to_string(),
            args: tool_args.clone(),
            request_id: request_id.clone(),
            parent_request_id: parent_request_id.to_string(),
        };
        let _ = ctx.event_tx.send(tool_request_event.clone());

        if let Some(ref writer) = transcript_writer {
            let writer = Arc::clone(writer);
            let event = tool_request_event;
            tokio::spawn(async move {
                if let Err(e) = writer.append(&event).await {
                    tracing::warn!("Failed to write to sub-agent transcript: {}", e);
                }
            });
        }

        let args_for_span =
            serde_json::to_string(&tool_args).unwrap_or_else(|_| "{}".to_string());
        let args_truncated = if args_for_span.chars().count() > 500 {
            format!("{}...[truncated]", truncate_str(&args_for_span, 500))
        } else {
            args_for_span
        };
        let tool_span = tracing::info_span!(
            parent: llm_span,
            "tool_call",
            "otel.name" = %tool_name,
            "langfuse.span.name" = %tool_name,
            "langfuse.observation.type" = "tool",
            "langfuse.session.id" = ctx.session_id.unwrap_or(""),
            tool.name = %tool_name,
            tool.id = %tool_id,
            "langfuse.observation.input" = %args_truncated,
            "langfuse.observation.output" = tracing::field::Empty,
            success = tracing::field::Empty,
        );
        let _tool_guard = tool_span.enter();

        let tool_timeout = idle_timeout.unwrap_or(timeout_duration);
        let tool_result = tokio::time::timeout(tool_timeout, async {
            if tool_name == "web_fetch" {
                tool_provider
                    .execute_web_fetch_tool(tool_name, &tool_args)
                    .await
            } else if let Some(result) = tool_provider
                .execute_memory_tool(tool_name, &tool_args)
                .await
            {
                result
            } else if tool_name == "run_pty_cmd" || tool_name == "run_command" {
                let command = tool_args.get("command").and_then(|c| c.as_str()).unwrap_or("");
                let cwd = tool_args.get("cwd").and_then(|c| c.as_str());
                let timeout_secs = tool_args.get("timeout").and_then(|t| t.as_u64()).unwrap_or(120);
                let workspace = ctx.workspace.read().await;

                let (chunk_tx, mut chunk_rx) = tokio::sync::mpsc::channel::<golish_shell_exec::OutputChunk>(64);

                let event_tx = ctx.event_tx.clone();
                let chunk_request_id = request_id.clone();
                let chunk_tool_name = tool_name.to_string();
                let chunk_agent_id = agent_id.to_string();
                let chunk_agent_name = agent_def.name.clone();
                tokio::spawn(async move {
                    while let Some(chunk) = chunk_rx.recv().await {
                        let _ = event_tx.send(AiEvent::ToolOutputChunk {
                            request_id: chunk_request_id.clone(),
                            tool_name: chunk_tool_name.clone(),
                            chunk: chunk.data,
                            stream: chunk.stream.as_str().to_string(),
                            source: ToolSource::SubAgent {
                                agent_id: chunk_agent_id.clone(),
                                agent_name: chunk_agent_name.clone(),
                            },
                        });
                    }
                });

                match golish_shell_exec::execute_streaming(
                    command, cwd, timeout_secs, &workspace, None, chunk_tx,
                ).await {
                    Ok(r) => {
                        let ok = r.exit_code == 0;
                        let mut v = serde_json::json!({
                            "stdout": r.stdout,
                            "stderr": r.stderr,
                            "exit_code": r.exit_code,
                            "command": command,
                        });
                        if let Some(c) = cwd {
                            v["cwd"] = serde_json::json!(c);
                        }
                        if !ok {
                            let err_detail = if r.stderr.is_empty() { &r.stdout } else { &r.stderr };
                            v["error"] = serde_json::json!(format!(
                                "Command exited with code {}: {}", r.exit_code, err_detail
                            ));
                        }
                        if r.timed_out {
                            v["timeout"] = serde_json::json!(true);
                        }
                        (v, ok)
                    }
                    Err(e) => (serde_json::json!({ "error": e.to_string() }), false),
                }
            } else {
                let registry = ctx.tool_registry.read().await;
                let result = registry.execute_tool(tool_name, tool_args.clone()).await;

                match &result {
                    Ok(v) => (v.clone(), true),
                    Err(e) => (serde_json::json!({ "error": e.to_string() }), false),
                }
            }
        })
        .await;

        let (result_value, success) = match tool_result {
            Ok(result) => result,
            Err(_) => {
                let error_msg = format!(
                    "Sub-agent tool '{}' timed out after {}s",
                    tool_name,
                    tool_timeout.as_secs()
                );
                tracing::warn!("[sub-agent] {}", error_msg);
                let _ = ctx.event_tx.send(AiEvent::SubAgentError {
                    agent_id: agent_id.to_string(),
                    error: error_msg.clone(),
                    parent_request_id: parent_request_id.to_string(),
                });
                (serde_json::json!({ "error": error_msg }), false)
            }
        };

        if success && (tool_name == "run_pty_cmd" || tool_name == "run_command") {
            if let Some(hook) = ctx.post_shell_hook.as_ref() {
                let cmd = result_value
                    .get("command")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string();
                let stdout = result_value
                    .get("stdout")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                let pp = {
                    let ws = ctx.workspace.read().await;
                    ws.to_string_lossy().to_string()
                };
                let hook = Arc::clone(hook);
                tokio::spawn(async move {
                    hook(cmd, stdout, Some(pp)).await;
                });
            }
        }

        let result_str = serde_json::to_string(&result_value).unwrap_or_default();
        let result_truncated = if result_str.chars().count() > 500 {
            format!("{}...[truncated]", truncate_str(&result_str, 500))
        } else {
            result_str
        };
        tool_span.record("langfuse.observation.output", &result_truncated);
        tool_span.record("success", success);

        let tool_result_event = AiEvent::SubAgentToolResult {
            agent_id: agent_id.to_string(),
            tool_name: tool_name.to_string(),
            success,
            result: result_value.clone(),
            request_id: request_id.clone(),
            parent_request_id: parent_request_id.to_string(),
        };
        let _ = ctx.event_tx.send(tool_result_event.clone());

        last_activity.store(epoch_secs(), Ordering::Relaxed);

        if let Some(ref writer) = transcript_writer {
            let writer = Arc::clone(writer);
            let event = tool_result_event;
            tokio::spawn(async move {
                if let Err(e) = writer.append(&event).await {
                    tracing::warn!("Failed to write to sub-agent transcript: {}", e);
                }
            });
        }

        if success && is_write_tool(tool_name) {
            if let Some(file_path) = extract_file_path(tool_name, &tool_args) {
                if !files_modified.contains(&file_path) {
                    tracing::debug!(
                        "[sub-agent] Tracking modified file: {} (tool: {})",
                        file_path,
                        tool_name
                    );
                    files_modified.push(file_path);
                }
            }
        }

        let result_text = serde_json::to_string(&result_value).unwrap_or_default();
        tool_results.push(UserContent::ToolResult(ToolResult {
            id: tool_id,
            call_id: Some(tool_call_id),
            content: OneOrMany::one(ToolResultContent::Text(Text { text: result_text })),
        }));
    }

    ToolDispatchResult {
        tool_results,
        barrier_hit,
        barrier_response,
    }
}
