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
    /// Stage-gate block signature of a `submit_stage_deliverable` call in this
    /// batch (its joined `needs_fix` reasons), or `None` if no submit BLOCKed.
    /// The loop tracks consecutive identical values to break a stuck re-submit.
    pub stage_block_signature: Option<String>,
}

/// Bail-out threshold for the stage submit loop: after this many *consecutive*
/// identical gate BLOCKs the sub-agent stops re-submitting and hands back to the
/// orchestrator instead of burning its whole iteration cap. Observed failure:
/// one per-org recon worker re-submitted the SAME "never attempted" block 22×
/// up to its 40-iteration cap (a wasted ~20 LLM turns per org).
pub(super) const STAGE_STALL_THRESHOLD: usize = 3;

/// Extract a stable block signature from a tool result, or `None` when it is not
/// a stage-gate BLOCK. Only `submit_stage_deliverable` with `status=="needs_fix"`
/// counts; the signature is its joined `reasons` so two identical blocks compare
/// equal across iterations.
pub(super) fn stage_block_signature(tool_name: &str, result: &serde_json::Value) -> Option<String> {
    if tool_name != "submit_stage_deliverable" {
        return None;
    }
    if result.get("status").and_then(|s| s.as_str()) != Some("needs_fix") {
        return None;
    }
    let joined = result
        .get("reasons")
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(" | ")
        })
        .unwrap_or_default();
    Some(joined)
}

/// Tracks consecutive identical stage-gate block signatures across loop
/// iterations. [`record`](StageStallGuard::record) returns how many times the
/// current signature has now repeated in a row; a *different* signature restarts
/// the streak, and `None` (no block this turn) leaves it unchanged — an
/// identical block re-seen after some intervening work is still a stall.
#[derive(Default)]
pub(super) struct StageStallGuard {
    last: Option<String>,
    streak: usize,
}

impl StageStallGuard {
    pub(super) fn record(&mut self, sig: Option<String>) -> usize {
        if let Some(s) = sig {
            if self.last.as_deref() == Some(s.as_str()) {
                self.streak += 1;
            } else {
                self.last = Some(s);
                self.streak = 1;
            }
        }
        self.streak
    }
}

/// Q3 ③ · Tag each tool in a `pentest_list_tools` result with `stage_allowed`
/// by probing the active stage guard — the SAME predicate the executor blocks
/// with — plus a top-level `stage_allowed_tools` list and a `stage_note`, so the
/// worker sees the in-stage tool boundary up front instead of discovering it by
/// hitting a BLOCK. No-op when the value has no `tools` array.
fn annotate_list_tools_with_guard(
    value: &mut serde_json::Value,
    guard: &crate::executor_types::StageToolGuard,
) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    let mut allowed: Vec<String> = Vec::new();
    if let Some(arr) = obj.get_mut("tools").and_then(|t| t.as_array_mut()) {
        for entry in arr.iter_mut() {
            let name = entry
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            if name.is_empty() {
                continue;
            }
            // Ok ⟺ the guard would let `pentest_run {tool_name: name}` run.
            let ok = guard("pentest_run", &serde_json::json!({ "tool_name": name })).is_ok();
            if let Some(entry_obj) = entry.as_object_mut() {
                entry_obj.insert("stage_allowed".to_string(), serde_json::Value::Bool(ok));
            }
            if ok {
                allowed.push(name);
            }
        }
    }
    obj.insert(
        "stage_allowed_tools".to_string(),
        serde_json::json!(allowed),
    );
    obj.insert(
        "stage_note".to_string(),
        serde_json::json!(
            "Inside the active stage only tools with stage_allowed=true are usable; calling any \
             other tool here is out-of-stage and will be BLOCKED — do not call it."
        ),
    );
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
    tool_fallback_timeout: Duration,
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
    // Last `submit_stage_deliverable` BLOCK signature seen this batch (for the
    // loop's stage-stall circuit breaker). Last write wins (one submit/turn).
    let mut last_block_sig: Option<String> = None;

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
            let summary = args.get("summary").and_then(|v| v.as_str()).unwrap_or("");

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
        if let Some(delegate_id) = tool_name.strip_prefix("sub_agent_") {
            let delegate_task = tool_call
                .function
                .arguments
                .get("task")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let nested_request_id = tool_call.id.clone();
            let nested_args = tool_call.function.arguments.clone();

            tracing::info!(
                "[sub-agent:{}] Nested delegation to '{}': {}",
                agent_id,
                delegate_id,
                truncate_str(&delegate_task, 100)
            );

            let tool_request_event = AiEvent::SubAgentToolRequest {
                agent_id: agent_id.to_string(),
                tool_name: tool_name.to_string(),
                args: nested_args.clone(),
                request_id: nested_request_id.clone(),
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
                        resume: None,
                        sub_tool_router: None,
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
                        // Propagate the stage boundary to nested sub-agents so a
                        // deeper delegate can't bypass the stage's forbidden tools.
                        stage_tool_guard: ctx.stage_tool_guard.clone(),
                        // Same for the D1 tool-list filter (hide scan tools).
                        hide_tool_in_stage: ctx.hide_tool_in_stage.clone(),
                    };
                    match Box::pin(super::execute_sub_agent(
                        &delegate_def,
                        &nested_args,
                        sub_context,
                        model,
                        nested_ctx,
                        tool_provider,
                        &nested_request_id,
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

            let delegate_success = delegate_result
                .get("success")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            let tool_result_event = AiEvent::SubAgentToolResult {
                agent_id: agent_id.to_string(),
                tool_name: tool_name.to_string(),
                success: delegate_success,
                result: delegate_result.clone(),
                request_id: nested_request_id,
                parent_request_id: parent_request_id.to_string(),
            };
            let _ = ctx.event_tx.send(tool_result_event.clone());

            if let Some(ref writer) = transcript_writer {
                let writer = Arc::clone(writer);
                let event = tool_result_event;
                tokio::spawn(async move {
                    if let Err(e) = writer.append(&event).await {
                        tracing::warn!("Failed to write to sub-agent transcript: {}", e);
                    }
                });
            }

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

        let args_for_span = serde_json::to_string(&tool_args).unwrap_or_else(|_| "{}".to_string());
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

        let tool_timeout = idle_timeout.unwrap_or(tool_fallback_timeout);
        let tool_result = tokio::time::timeout(tool_timeout, async {
            // Stage boundary (forbidden-only): block a tool whose RESOLVED
            // capability is forbidden in the active harness stage BEFORE running
            // it (e.g. `dig` via pentest_run in scoping). The synthetic error
            // flows through the normal result path so the model gets actionable
            // feedback. See docs/design/2026-06-02-stage-tool-whitelist-enforcement.md.
            if let Some(reason) = ctx
                .stage_tool_guard
                .as_ref()
                .and_then(|guard| guard(tool_name, &tool_args).err())
            {
                tracing::warn!(
                    target: "harness::stage_guard",
                    tool = %tool_name,
                    reason = %reason,
                    "sub-agent tool call BLOCKED by stage boundary"
                );
                return (
                    serde_json::json!({ "error": reason, "blocked_by_stage": true }),
                    false,
                );
            }
            if tool_name == "web_fetch" {
                tool_provider
                    .execute_web_fetch_tool(tool_name, &tool_args)
                    .await
            } else if let Some(result) = tool_provider
                .execute_memory_tool(tool_name, &tool_args)
                .await
            {
                result
            } else if let Some(result) = tool_provider
                .execute_knowledge_base_tool(tool_name, &tool_args)
                .await
            {
                result
            } else if tool_name == "run_pty_cmd" || tool_name == "run_command" {
                let command = tool_args
                    .get("command")
                    .and_then(|c| c.as_str())
                    .unwrap_or("");
                let cwd = tool_args.get("cwd").and_then(|c| c.as_str());
                let timeout_secs = tool_args
                    .get("timeout")
                    .and_then(|t| t.as_u64())
                    .unwrap_or(120);
                let workspace = ctx.workspace.read().await;

                let (chunk_tx, mut chunk_rx) =
                    tokio::sync::mpsc::channel::<golish_shell_exec::OutputChunk>(64);

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
                    command,
                    cwd,
                    timeout_secs,
                    &workspace,
                    None,
                    chunk_tx,
                )
                .await
                {
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
                            let err_detail = if r.stderr.is_empty() {
                                &r.stdout
                            } else {
                                &r.stderr
                            };
                            v["error"] = serde_json::json!(format!(
                                "Command exited with code {}: {}",
                                r.exit_code, err_detail
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
                // Try the injected router first (graph/memory tools that live
                // outside the ToolRegistry); fall through to the registry.
                let routed = match &ctx.sub_tool_router {
                    Some(router) => router(tool_name.to_string(), tool_args.clone()).await,
                    None => None,
                };
                match routed {
                    Some((value, success)) => (value, success),
                    None => {
                        let registry = ctx.tool_registry.read().await;
                        match registry.execute_tool(tool_name, tool_args.clone()).await {
                            Ok(v) => (v.clone(), true),
                            Err(e) => (serde_json::json!({ "error": e.to_string() }), false),
                        }
                    }
                }
            }
        })
        .await;

        let (mut result_value, success) = match tool_result {
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

        // Q3 ③ · stage-annotate `pentest_list_tools` so this worker sees, per
        // tool, whether the active stage permits it — instead of discovering the
        // boundary by hitting a BLOCK. Reuses the SAME stage guard the executor
        // enforces with (probe each tool as a `pentest_run` call), so the verdict
        // matches what would actually run. No-op outside a harness stage.
        if success && tool_name == "pentest_list_tools" {
            if let Some(guard) = ctx.stage_tool_guard.as_ref() {
                annotate_list_tools_with_guard(&mut result_value, guard);
            }
        }

        // Stage-stall circuit breaker: record a submit_stage_deliverable BLOCK so
        // the loop can bail after STAGE_STALL_THRESHOLD identical ones.
        if let Some(sig) = stage_block_signature(tool_name, &result_value) {
            last_block_sig = Some(sig);
        }

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
        stage_block_signature: last_block_sig,
    }
}

#[cfg(test)]
mod tests {
    use super::annotate_list_tools_with_guard;
    use std::sync::Arc;

    #[test]
    fn guard_probe_marks_each_tool() {
        // Stage guard that only allows `dig` (mimics a recon/dns-only stage):
        // matches what the executor would enforce, so the annotation agrees.
        let guard: crate::executor_types::StageToolGuard =
            Arc::new(|tn: &str, args: &serde_json::Value| {
                let inner = args.get("tool_name").and_then(|v| v.as_str()).unwrap_or(tn);
                if inner == "dig" {
                    Ok(())
                } else {
                    Err(format!("'{inner}' not allowed"))
                }
            });
        let mut v = serde_json::json!({
            "tools": [ { "name": "dig" }, { "name": "nmap" }, { "name": "sqlmap" } ],
            "total": 3
        });
        annotate_list_tools_with_guard(&mut v, &guard);
        let tools = v["tools"].as_array().unwrap();
        let allowed = |n: &str| {
            tools.iter().find(|t| t["name"] == n).unwrap()["stage_allowed"]
                .as_bool()
                .unwrap()
        };
        assert!(allowed("dig"), "dig allowed");
        assert!(!allowed("nmap"), "nmap blocked");
        assert!(!allowed("sqlmap"), "sqlmap blocked");
        assert_eq!(
            v["stage_allowed_tools"].as_array().unwrap(),
            &vec![serde_json::json!("dig")]
        );
        assert!(v["stage_note"]
            .as_str()
            .unwrap()
            .contains("stage_allowed=true"));
    }

    #[test]
    fn guard_probe_noop_without_tools_array() {
        let guard: crate::executor_types::StageToolGuard = Arc::new(|_, _| Ok(()));
        let mut v = serde_json::json!({ "error": "x" });
        annotate_list_tools_with_guard(&mut v, &guard);
        assert_eq!(v["error"], "x");
        assert!(v["stage_allowed_tools"].as_array().unwrap().is_empty());
    }

    // ── stage-stall circuit breaker (2026-06-16) ────────────────────────────

    #[test]
    fn block_signature_only_for_submit_needs_fix() {
        use super::stage_block_signature;
        // submit_stage_deliverable + needs_fix → joined reasons.
        assert_eq!(
            stage_block_signature(
                "submit_stage_deliverable",
                &serde_json::json!({ "status": "needs_fix", "reasons": ["a", "b"] }),
            ),
            Some("a | b".to_string())
        );
        // accepted (or any non-needs_fix) → not a block.
        assert_eq!(
            stage_block_signature(
                "submit_stage_deliverable",
                &serde_json::json!({ "status": "accepted" }),
            ),
            None
        );
        // needs_fix without a reasons array → empty signature (still a block).
        assert_eq!(
            stage_block_signature(
                "submit_stage_deliverable",
                &serde_json::json!({ "status": "needs_fix" }),
            ),
            Some(String::new())
        );
        // a different tool never counts, even with a needs_fix-shaped body.
        assert_eq!(
            stage_block_signature(
                "pentest_run",
                &serde_json::json!({ "status": "needs_fix", "reasons": ["a"] }),
            ),
            None
        );
    }

    #[test]
    fn stall_guard_counts_consecutive_identical_blocks() {
        use super::{StageStallGuard, STAGE_STALL_THRESHOLD};
        let mut g = StageStallGuard::default();
        // First two identical blocks build the streak below the threshold.
        assert_eq!(g.record(Some("R".into())), 1);
        assert_eq!(g.record(Some("R".into())), 2);
        // Driving it up to the threshold returns exactly the bail-out count.
        let mut streak = 2;
        while streak < STAGE_STALL_THRESHOLD {
            streak = g.record(Some("R".into()));
        }
        assert_eq!(streak, STAGE_STALL_THRESHOLD);
    }

    #[test]
    fn stall_guard_resets_on_different_and_holds_on_none() {
        use super::StageStallGuard;
        let mut g = StageStallGuard::default();
        assert_eq!(g.record(Some("R".into())), 1);
        assert_eq!(g.record(Some("R".into())), 2);
        // a different block restarts the streak at 1.
        assert_eq!(g.record(Some("R2".into())), 1);
        // a non-block turn (None) leaves the streak unchanged.
        assert_eq!(g.record(None), 1);
        assert_eq!(g.record(Some("R2".into())), 2);
    }
}
