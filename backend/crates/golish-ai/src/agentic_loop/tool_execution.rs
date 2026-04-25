//! Tool execution functions extracted from the main agentic loop.
//!
//! Contains `execute_tool_direct_generic`, `execute_shell_command_streaming`,
//! and `execute_with_hitl_generic`.

use anyhow::Result;
use rig::completion::CompletionModel as RigCompletionModel;
use serde_json::json;
use tokio::sync::mpsc;

use golish_core::events::AiEvent;
use golish_core::hitl::RiskLevel;
use golish_core::utils::{is_tool_result_success, truncate_str};
use golish_sub_agents::{execute_sub_agent, SubAgentContext, SubAgentExecutorContext};

use super::sub_agent_dispatch::{build_sub_agent_briefing, execute_sub_agent_with_client};
use super::{
    emit_event, emit_to_frontend, AgenticLoopContext, LoopCaptureContext, ToolExecutionResult,
    APPROVAL_TIMEOUT_SECS,
};
use crate::tool_executors::{execute_ask_human_tool, execute_plan_tool, execute_web_fetch_tool};
use crate::tool_policy::{PolicyConstraintResult, ToolPolicy};
use crate::tool_provider_impl::DefaultToolProvider;

/// Execute a tool directly for generic models (after approval or auto-approved).
pub async fn execute_tool_direct_generic<M>(
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
    if tool_name.starts_with("indexer_") {
        return Ok(ToolExecutionResult {
            value: json!({"error": "Indexer tools are no longer available. Use grep_file, ast_grep, read_file, or sub-agents for code analysis."}),
            success: false,
        });
    }

    if tool_name == "web_fetch" {
        let (value, success) = execute_web_fetch_tool(tool_name, tool_args).await;
        return Ok(ToolExecutionResult { value, success });
    }

    if tool_name == "update_plan" {
        let (value, success) = execute_plan_tool(ctx.plan_manager, ctx.event_tx, tool_args).await;
        return Ok(ToolExecutionResult { value, success });
    }

    if matches!(
        tool_name,
        "search_memories" | "store_memory" | "list_memories"
        | "search_code" | "save_code" | "search_guide" | "save_guide"
    ) {
        if let Some((value, success)) =
            crate::tool_executors::execute_memory_tool(tool_name, tool_args, ctx.db_tracker).await
        {
            return Ok(ToolExecutionResult { value, success });
        }
    }

    if matches!(
        tool_name,
        "search_knowledge_base"
            | "write_knowledge"
            | "read_knowledge"
            | "ingest_cve"
            | "save_poc"
    ) {
        if let Some((value, success)) =
            crate::tool_executors::execute_knowledge_base_tool(tool_name, tool_args, ctx.db_tracker)
                .await
        {
            return Ok(ToolExecutionResult { value, success });
        }
    }

    if matches!(
        tool_name,
        "log_operation"
            | "discover_apis"
            | "save_js_analysis"
            | "fingerprint_target"
            | "log_scan_result"
            | "query_target_data"
    ) {
        let ws_path = ctx.workspace.read().await;
        let project_path_str = ws_path.to_string_lossy().to_string();
        drop(ws_path);
        if let Some((value, success)) = crate::tool_executors::execute_security_analysis_tool(
            tool_name,
            tool_args,
            ctx.db_tracker,
            Some(project_path_str.as_str()),
            ctx.session_id,
        )
        .await
        {
            return Ok(ToolExecutionResult { value, success });
        }
    }

    if tool_name == "ask_human" {
        let (value, success) = execute_ask_human_tool(
            tool_args,
            ctx.event_tx,
            ctx.coordinator,
            ctx.pending_approvals,
        )
        .await;
        return Ok(ToolExecutionResult { value, success });
    }

    if let Some(ref executor) = ctx.custom_tool_executor {
        if let Some((value, success)) = executor(tool_name, tool_args).await {
            return Ok(ToolExecutionResult { value, success });
        }
    }

    if tool_name.starts_with("sub_agent_") {
        return execute_sub_agent_call(tool_name, tool_args, ctx, model, context, tool_id).await;
    }

    let effective_tool_name = if tool_name == "run_command" {
        "run_pty_cmd"
    } else {
        tool_name
    };

    let registry = ctx.tool_registry.read().await;
    let result = registry
        .execute_tool(effective_tool_name, tool_args.clone())
        .await;

    match &result {
        Ok(v) => {
            let is_success = is_tool_result_success(v);

            if effective_tool_name == "run_pty_cmd" && is_success {
                if let Some(tracker) = ctx.db_tracker {
                    let pool = tracker.pool_arc().clone();
                    let stdout = v.get("stdout").and_then(|s| s.as_str()).unwrap_or("").to_string();
                    let command = tool_args.get("command").and_then(|c| c.as_str()).unwrap_or("").to_string();
                    let ws = ctx.workspace.read().await;
                    let pp = ws.to_string_lossy().to_string();
                    drop(ws);
                    tokio::spawn(async move {
                        let _ = golish_pentest::output_store::maybe_detect_and_store(
                            &pool, &command, &stdout, Some(&pp),
                        ).await;
                    });
                }
            }

            Ok(ToolExecutionResult {
                value: v.clone(),
                success: is_success,
            })
        }
        Err(e) => Ok(ToolExecutionResult {
            value: json!({"error": e.to_string()}),
            success: false,
        }),
    }
}

/// Handle sub-agent tool calls (tool names starting with `sub_agent_`).
async fn execute_sub_agent_call<M>(
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

    let tool_provider = DefaultToolProvider::new();

    let task_desc = tool_args
        .get("task")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let briefing = build_sub_agent_briefing(ctx.db_tracker, agent_id, task_desc).await;

    let result = if let Some((override_provider, override_model)) = &agent_def.model_override {
        let override_client = if let Some(factory) = ctx.model_factory {
            match factory
                .get_or_create(override_provider, override_model)
                .await
            {
                Ok(client) => Some(client),
                Err(e) => {
                    tracing::warn!(
                        "Failed to create override model {}/{} for sub-agent '{}': {}. Using main model.",
                        override_provider, override_model, agent_id, e
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
                event_tx: ctx.event_tx,
                tool_registry: ctx.tool_registry,
                workspace: ctx.workspace,
                provider_name: override_provider,
                model_name: override_model,
                session_id: ctx.session_id,
                transcript_base_dir: ctx.transcript_base_dir,
                api_request_stats: Some(ctx.api_request_stats),
                briefing: briefing.clone(),
                temperature_override: agent_def.temperature,
                max_tokens_override: agent_def.max_tokens,
                top_p_override: agent_def.top_p,
                db_pool: ctx.db_tracker.map(|t| t.pool_arc()),
                sub_agent_registry: Some(ctx.sub_agent_registry),
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
                ctx.provider_name,
                ctx.model_name
            );
            let sub_ctx = SubAgentExecutorContext {
                event_tx: ctx.event_tx,
                tool_registry: ctx.tool_registry,
                workspace: ctx.workspace,
                provider_name: ctx.provider_name,
                model_name: ctx.model_name,
                session_id: ctx.session_id,
                transcript_base_dir: ctx.transcript_base_dir,
                api_request_stats: Some(ctx.api_request_stats),
                briefing: briefing.clone(),
                temperature_override: agent_def.temperature,
                max_tokens_override: agent_def.max_tokens,
                top_p_override: agent_def.top_p,
                db_pool: ctx.db_tracker.map(|t| t.pool_arc()),
                sub_agent_registry: Some(ctx.sub_agent_registry),
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
            ctx.provider_name,
            ctx.model_name
        );
        let sub_ctx = SubAgentExecutorContext {
            event_tx: ctx.event_tx,
            tool_registry: ctx.tool_registry,
            workspace: ctx.workspace,
            provider_name: ctx.provider_name,
            model_name: ctx.model_name,
            session_id: ctx.session_id,
            transcript_base_dir: ctx.transcript_base_dir,
            api_request_stats: Some(ctx.api_request_stats),
            briefing,
            temperature_override: agent_def.temperature,
            max_tokens_override: agent_def.max_tokens,
            top_p_override: agent_def.top_p,
            db_pool: ctx.db_tracker.map(|t| t.pool_arc()),
            sub_agent_registry: Some(ctx.sub_agent_registry),
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

    match result {
        Ok(result) => {
            if let Some(tracker) = ctx.db_tracker {
                let result_preview = truncate_str(&result.response, 500);
                tracker.record_agent_call(
                    "primary",
                    agent_id,
                    &context.original_request,
                    Some(result_preview),
                    result.duration_ms,
                );
            }

            Ok(ToolExecutionResult {
                value: json!({
                    "agent_id": result.agent_id,
                    "response": result.response,
                    "success": result.success,
                    "duration_ms": result.duration_ms,
                    "files_modified": result.files_modified
                }),
                success: result.success,
            })
        }
        Err(e) => Ok(ToolExecutionResult {
            value: json!({ "error": e.to_string() }),
            success: false,
        }),
    }
}

/// Execute a shell command with streaming output (background execution).
///
/// Currently unused: commands route through VisibleRunPtyCmdTool in the registry.
#[allow(dead_code)]
pub(crate) async fn execute_shell_command_streaming(
    tool_args: &serde_json::Value,
    tool_id: &str,
    ctx: &AgenticLoopContext<'_>,
) -> Result<ToolExecutionResult> {
    use golish_shell_exec::{execute_streaming, OutputChunk};

    let command = tool_args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required argument: command"))?;

    let cwd = tool_args.get("cwd").and_then(|v| v.as_str());

    const MAX_SHELL_TIMEOUT_SECS: u64 = 600;
    let timeout_secs = tool_args
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(120)
        .min(MAX_SHELL_TIMEOUT_SECS);

    let workspace = ctx.workspace.read().await;
    let shell_override: Option<String> = None;
    let (chunk_tx, mut chunk_rx) = mpsc::channel::<OutputChunk>(100);

    let event_tx = ctx.event_tx.clone();
    let request_id = tool_id.to_string();

    let chunk_forwarder = tokio::spawn(async move {
        tracing::debug!("Chunk forwarder started for tool: {}", request_id);
        while let Some(chunk) = chunk_rx.recv().await {
            tracing::debug!(
                "Received output chunk for {}: {} bytes",
                request_id,
                chunk.data.len()
            );
            let event = AiEvent::ToolOutputChunk {
                request_id: request_id.clone(),
                tool_name: "run_pty_cmd".to_string(),
                chunk: chunk.data,
                stream: chunk.stream.as_str().to_string(),
                source: golish_core::events::ToolSource::Main,
            };
            if let Err(e) = event_tx.send(event) {
                tracing::error!("Failed to send ToolOutputChunk event: {:?}", e);
            } else {
                tracing::debug!("Sent ToolOutputChunk event for {}", request_id);
            }
        }
        tracing::debug!("Chunk forwarder finished for tool");
    });

    let result = execute_streaming(
        command,
        cwd,
        timeout_secs,
        &workspace,
        shell_override.as_deref(),
        chunk_tx,
    )
    .await;

    let _ = chunk_forwarder.await;

    match result {
        Ok(streaming_result) => {
            let exit_code = streaming_result.exit_code;
            let is_success = exit_code == 0 && !streaming_result.timed_out;

            let mut value = json!({
                "stdout": streaming_result.stdout,
                "stderr": streaming_result.stderr,
                "exit_code": exit_code,
                "command": command
            });

            if let Some(c) = cwd {
                value["cwd"] = json!(c);
            }

            if streaming_result.timed_out {
                value["error"] =
                    json!(format!("Command timed out after {} seconds", timeout_secs));
                value["timeout"] = json!(true);
            } else if exit_code != 0 {
                let error_output = if streaming_result.stderr.is_empty() {
                    &streaming_result.stdout
                } else {
                    &streaming_result.stderr
                };
                value["error"] = json!(format!(
                    "Command exited with code {}: {}",
                    exit_code, error_output
                ));
            }

            Ok(ToolExecutionResult {
                value,
                success: is_success,
            })
        }
        Err(e) => Ok(ToolExecutionResult {
            value: json!({"error": e.to_string(), "exit_code": 1}),
            success: false,
        }),
    }
}

/// Execute a tool with HITL approval check for generic models.
pub async fn execute_with_hitl_generic<M>(
    tool_name: &str,
    tool_args: &serde_json::Value,
    tool_id: &str,
    ctx: &AgenticLoopContext<'_>,
    capture_ctx: &LoopCaptureContext,
    model: &M,
    context: &SubAgentContext,
) -> Result<ToolExecutionResult>
where
    M: RigCompletionModel + Sync,
{
    capture_ctx.process(&AiEvent::ToolRequest {
        request_id: tool_id.to_string(),
        tool_name: tool_name.to_string(),
        args: tool_args.clone(),
        source: golish_core::events::ToolSource::Main,
    });

    let agent_mode = *ctx.agent_mode.read().await;

    let is_auto_approve =
        agent_mode.is_auto_approve() || ctx.runtime.is_some_and(|r| r.auto_approve());

    // Planning mode: only allow read-only tools
    if agent_mode.is_planning() {
        use crate::tool_policy::ALLOW_TOOLS;
        if !ALLOW_TOOLS.contains(&tool_name) {
            let denied_event = AiEvent::ToolDenied {
                request_id: tool_id.to_string(),
                tool_name: tool_name.to_string(),
                args: tool_args.clone(),
                reason: "Planning mode: only read-only tools are allowed".to_string(),
                source: golish_core::events::ToolSource::Main,
            };
            emit_to_frontend(ctx, denied_event.clone());
            capture_ctx.process(&denied_event);
            return Ok(ToolExecutionResult {
                value: json!({
                    "error": format!("Tool '{}' is not allowed in planning mode (read-only)", tool_name),
                    "planning_mode_denied": true
                }),
                success: false,
            });
        }
    }

    if !is_auto_approve && ctx.tool_policy_manager.is_denied(tool_name).await {
        let denied_event = AiEvent::ToolDenied {
            request_id: tool_id.to_string(),
            tool_name: tool_name.to_string(),
            args: tool_args.clone(),
            reason: "Tool is denied by policy".to_string(),
            source: golish_core::events::ToolSource::Main,
        };
        emit_to_frontend(ctx, denied_event.clone());
        capture_ctx.process(&denied_event);
        return Ok(ToolExecutionResult {
            value: json!({
                "error": format!("Tool '{}' is denied by policy", tool_name),
                "denied_by_policy": true
            }),
            success: false,
        });
    }

    let (effective_args, constraint_note) = match ctx
        .tool_policy_manager
        .apply_constraints(tool_name, tool_args)
        .await
    {
        PolicyConstraintResult::Allowed => (tool_args.clone(), None),
        PolicyConstraintResult::Violated(reason) => {
            emit_event(
                ctx,
                AiEvent::ToolDenied {
                    request_id: tool_id.to_string(),
                    tool_name: tool_name.to_string(),
                    args: tool_args.clone(),
                    reason: reason.clone(),
                    source: golish_core::events::ToolSource::Main,
                },
            );
            return Ok(ToolExecutionResult {
                value: json!({
                    "error": format!("Tool constraint violated: {}", reason),
                    "constraint_violated": true
                }),
                success: false,
            });
        }
        PolicyConstraintResult::Modified(modified_args, note) => {
            tracing::info!("Tool '{}' args modified by constraint: {}", tool_name, note);
            (modified_args, Some(note))
        }
    };

    let policy = ctx.tool_policy_manager.get_policy(tool_name).await;
    if policy == ToolPolicy::Allow {
        let reason = if let Some(note) = constraint_note {
            format!("Allowed by policy ({})", note)
        } else {
            "Allowed by tool policy".to_string()
        };
        emit_event(
            ctx,
            AiEvent::ToolAutoApproved {
                request_id: tool_id.to_string(),
                tool_name: tool_name.to_string(),
                args: effective_args.clone(),
                reason,
                source: golish_core::events::ToolSource::Main,
            },
        );

        return execute_tool_direct_generic(tool_name, &effective_args, ctx, model, context, tool_id)
            .await;
    }

    if ctx.approval_recorder.should_auto_approve(tool_name).await {
        emit_event(
            ctx,
            AiEvent::ToolAutoApproved {
                request_id: tool_id.to_string(),
                tool_name: tool_name.to_string(),
                args: effective_args.clone(),
                reason: "Auto-approved based on learned patterns or always-allow list".to_string(),
                source: golish_core::events::ToolSource::Main,
            },
        );

        return execute_tool_direct_generic(tool_name, &effective_args, ctx, model, context, tool_id)
            .await;
    }

    if tool_name.starts_with("pentest_") {
        tracing::info!("[hitl] Auto-approving pentest tool: {}", tool_name);
        emit_event(
            ctx,
            AiEvent::ToolAutoApproved {
                request_id: tool_id.to_string(),
                tool_name: tool_name.to_string(),
                args: effective_args.clone(),
                reason: "Auto-approved: Golish platform tool".to_string(),
                source: golish_core::events::ToolSource::Main,
            },
        );

        return execute_tool_direct_generic(tool_name, &effective_args, ctx, model, context, tool_id)
            .await;
    }

    if is_auto_approve {
        let reason = if agent_mode.is_auto_approve() {
            "Auto-approved via agent mode"
        } else {
            "Auto-approved via --auto-approve flag"
        };
        emit_event(
            ctx,
            AiEvent::ToolAutoApproved {
                request_id: tool_id.to_string(),
                tool_name: tool_name.to_string(),
                args: effective_args.clone(),
                reason: reason.to_string(),
                source: golish_core::events::ToolSource::Main,
            },
        );

        return execute_tool_direct_generic(tool_name, &effective_args, ctx, model, context, tool_id)
            .await;
    }

    // Need HITL approval
    let stats = ctx.approval_recorder.get_pattern(tool_name).await;
    let risk_level = RiskLevel::for_tool(tool_name);
    let config = ctx.approval_recorder.get_config().await;
    let can_learn = !config
        .always_require_approval
        .contains(&tool_name.to_string());
    let suggestion = ctx.approval_recorder.get_suggestion(tool_name).await;

    let rx = if let Some(coordinator) = ctx.coordinator {
        coordinator.register_approval(tool_id.to_string())
    } else {
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut pending = ctx.pending_approvals.write().await;
            pending.insert(tool_id.to_string(), tx);
        }
        rx
    };

    emit_to_frontend(
        ctx,
        AiEvent::ToolApprovalRequest {
            request_id: tool_id.to_string(),
            tool_name: tool_name.to_string(),
            args: effective_args.clone(),
            stats,
            risk_level,
            can_learn,
            suggestion,
            source: golish_core::events::ToolSource::Main,
        },
    );

    tracing::info!(
        "[hitl] Waiting for user approval: tool={}, risk={:?}, id={}",
        tool_name,
        risk_level,
        tool_id
    );

    match tokio::time::timeout(std::time::Duration::from_secs(APPROVAL_TIMEOUT_SECS), rx).await {
        Ok(Ok(decision)) => {
            tracing::info!(
                "[hitl] User decision: tool={}, approved={}",
                tool_name,
                decision.approved
            );
            if decision.approved {
                let _ = ctx
                    .approval_recorder
                    .record_approval(tool_name, true, decision.reason, decision.always_allow)
                    .await;

                execute_tool_direct_generic(
                    tool_name,
                    &effective_args,
                    ctx,
                    model,
                    context,
                    tool_id,
                )
                .await
            } else {
                let _ = ctx
                    .approval_recorder
                    .record_approval(tool_name, false, decision.reason, false)
                    .await;

                Ok(ToolExecutionResult {
                    value: json!({"error": "Tool execution denied by user. Do NOT retry this tool with the same or similar arguments.", "denied": true}),
                    success: false,
                })
            }
        }
        Ok(Err(_)) => Ok(ToolExecutionResult {
            value: json!({"error": "Approval request cancelled", "cancelled": true}),
            success: false,
        }),
        Err(_) => {
            tracing::warn!(
                "[hitl] Approval TIMED OUT after {}s: tool={}",
                APPROVAL_TIMEOUT_SECS,
                tool_name
            );
            if ctx.coordinator.is_none() {
                let mut pending = ctx.pending_approvals.write().await;
                pending.remove(tool_id);
            }

            Ok(ToolExecutionResult {
                value: json!({
                    "error": format!("Approval request timed out after {} seconds. The user did not respond to the approval prompt. Do NOT retry this tool or attempt alternative approaches — inform the user that the tool requires their approval and wait for their next message.", APPROVAL_TIMEOUT_SECS),
                    "timeout": true,
                    "requires_user_action": true
                }),
                success: false,
            })
        }
    }
}
