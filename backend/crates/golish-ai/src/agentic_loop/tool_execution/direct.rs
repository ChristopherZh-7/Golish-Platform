//! `execute_tool_direct_generic` — runs a tool when no human approval is
//! required (auto-approved or already approved).
//!
//! Also contains the private `execute_sub_agent_call` helper that branches
//! between built-in sub-agent execution and the registry-driven sub-agent
//! dispatch path.

use anyhow::Result;
use rig::completion::CompletionModel as RigCompletionModel;
use serde_json::json;
use tokio::sync::mpsc;

use golish_core::events::AiEvent;
use golish_core::utils::{is_tool_result_success, truncate_str};
use golish_sub_agents::{SubAgentContext, SubAgentExecutorContext, execute_sub_agent};

use super::super::sub_agent_dispatch::{build_sub_agent_briefing, execute_sub_agent_with_client};
use super::super::{AgenticLoopContext, ToolExecutionResult};
use crate::tool_executors::{execute_ask_human_tool, execute_plan_tool, execute_web_fetch_tool};
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
        let (value, success) =
            execute_plan_tool(ctx.plan_manager, ctx.events.event_tx, tool_args).await;
        return Ok(ToolExecutionResult { value, success });
    }

    if matches!(
        tool_name,
        "search_memories"
            | "store_memory"
            | "list_memories"
            | "search_code"
            | "save_code"
            | "search_guide"
            | "save_guide"
    ) {
        if let Some((value, success)) =
            crate::tool_executors::execute_memory_tool(tool_name, tool_args, ctx.events.db_tracker)
                .await
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
            | "list_cves_with_pocs"
            | "list_unresearched_cves"
            | "poc_stats"
    ) {
        if let Some((value, success)) = crate::tool_executors::execute_knowledge_base_tool(
            tool_name,
            tool_args,
            ctx.events.db_tracker,
        )
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
            ctx.events.db_tracker,
            Some(project_path_str.as_str()),
            ctx.events.session_id,
        )
        .await
        {
            return Ok(ToolExecutionResult { value, success });
        }
    }

    if tool_name == "ask_human" {
        let (value, success) = execute_ask_human_tool(
            tool_args,
            ctx.events.event_tx,
            ctx.access.coordinator,
            ctx.access.pending_approvals,
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
                if let Some(tracker) = ctx.events.db_tracker {
                    let pool = tracker.pool_arc().clone();
                    let stdout = v
                        .get("stdout")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    let command = tool_args
                        .get("command")
                        .and_then(|c| c.as_str())
                        .unwrap_or("")
                        .to_string();
                    let ws = ctx.workspace.read().await;
                    let pp = ws.to_string_lossy().to_string();
                    drop(ws);
                    tokio::spawn(async move {
                        let _ = golish_pentest::output_store::maybe_detect_and_store(
                            &pool,
                            &command,
                            &stdout,
                            Some(&pp),
                        )
                        .await;
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

    let tool_provider = DefaultToolProvider::with_db_tracker(ctx.events.db_tracker);

    let task_desc = tool_args.get("task").and_then(|v| v.as_str()).unwrap_or("");
    let briefing = build_sub_agent_briefing(ctx.events.db_tracker, agent_id, task_desc).await;

    let result = if let Some((override_provider, override_model)) = &agent_def.model_override {
        let override_client = if let Some(factory) = ctx.llm.model_factory {
            match factory
                .get_or_create(override_provider, override_model)
                .await
            {
                Ok(client) => Some(client),
                Err(e) => {
                    tracing::warn!(
                        "Failed to create override model {}/{} for sub-agent '{}': {}. Using main model.",
                        override_provider,
                        override_model,
                        agent_id,
                        e
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
                event_tx: ctx.events.event_tx,
                tool_registry: ctx.tool_registry,
                workspace: ctx.workspace,
                provider_name: override_provider,
                model_name: override_model,
                session_id: ctx.events.session_id,
                transcript_base_dir: ctx.events.transcript_base_dir,
                api_request_stats: Some(ctx.api_request_stats),
                briefing: briefing.clone(),
                temperature_override: agent_def.temperature,
                max_tokens_override: agent_def.max_tokens,
                top_p_override: agent_def.top_p,
                db_pool: ctx.events.db_tracker.map(|t| t.pool_arc()),
                sub_agent_registry: Some(ctx.sub_agent_registry),
                post_shell_hook: crate::pentest_hook::make_post_shell_hook(),
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
                ctx.llm.provider_name,
                ctx.llm.model_name
            );
            let sub_ctx = SubAgentExecutorContext {
                event_tx: ctx.events.event_tx,
                tool_registry: ctx.tool_registry,
                workspace: ctx.workspace,
                provider_name: ctx.llm.provider_name,
                model_name: ctx.llm.model_name,
                session_id: ctx.events.session_id,
                transcript_base_dir: ctx.events.transcript_base_dir,
                api_request_stats: Some(ctx.api_request_stats),
                briefing: briefing.clone(),
                temperature_override: agent_def.temperature,
                max_tokens_override: agent_def.max_tokens,
                top_p_override: agent_def.top_p,
                db_pool: ctx.events.db_tracker.map(|t| t.pool_arc()),
                sub_agent_registry: Some(ctx.sub_agent_registry),
                post_shell_hook: crate::pentest_hook::make_post_shell_hook(),
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
            ctx.llm.provider_name,
            ctx.llm.model_name
        );
        let sub_ctx = SubAgentExecutorContext {
            event_tx: ctx.events.event_tx,
            tool_registry: ctx.tool_registry,
            workspace: ctx.workspace,
            provider_name: ctx.llm.provider_name,
            model_name: ctx.llm.model_name,
            session_id: ctx.events.session_id,
            transcript_base_dir: ctx.events.transcript_base_dir,
            api_request_stats: Some(ctx.api_request_stats),
            briefing,
            temperature_override: agent_def.temperature,
            max_tokens_override: agent_def.max_tokens,
            top_p_override: agent_def.top_p,
            db_pool: ctx.events.db_tracker.map(|t| t.pool_arc()),
            sub_agent_registry: Some(ctx.sub_agent_registry),
            post_shell_hook: crate::pentest_hook::make_post_shell_hook(),
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
            if let Some(tracker) = ctx.events.db_tracker {
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
    use golish_shell_exec::{OutputChunk, execute_streaming};

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

    let event_tx = ctx.events.event_tx.clone();
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
                value["error"] = json!(format!("Command timed out after {} seconds", timeout_secs));
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
