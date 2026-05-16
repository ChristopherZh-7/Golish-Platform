//! `execute_tool_direct_generic` — runs a tool when no human approval is
//! required (auto-approved or already approved).
//!
//! Also contains the private `execute_sub_agent_call` helper that branches
//! between built-in sub-agent execution and the registry-driven sub-agent
//! dispatch path.

use std::sync::Arc;

use anyhow::Result;
use rig::completion::CompletionModel as RigCompletionModel;
use serde_json::json;

use golish_core::utils::{is_tool_result_success, truncate_str};
use golish_sub_agents::{execute_sub_agent, SubAgentContext, SubAgentExecutorContext};

use super::super::sub_agent_dispatch::{build_sub_agent_briefing, execute_sub_agent_with_client};
use super::super::{AgenticLoopContext, ToolExecutionResult};
use golish_agent_kit::tool_executors::{
    execute_ask_human_tool, execute_plan_tool, execute_web_fetch_tool,
    extract_and_upsert_entities,
};
use golish_agent_kit::tool_provider_impl::DefaultToolProvider;

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
        if let Some(ref fetcher) = ctx.web_fetcher {
            let (value, success) =
                execute_web_fetch_tool(fetcher.as_ref(), tool_name, tool_args).await;
            return Ok(ToolExecutionResult { value, success });
        }
        return Ok(ToolExecutionResult {
            value: json!({"error": "Web fetch provider not configured"}),
            success: false,
        });
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
        if let Some((value, success)) = golish_agent_kit::tool_executors::execute_memory_tool(
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
        "search_knowledge_base"
            | "write_knowledge"
            | "read_knowledge"
            | "ingest_cve"
            | "save_poc"
            | "list_cves_with_pocs"
            | "list_unresearched_cves"
            | "poc_stats"
    ) {
        if let Some((value, success)) =
            golish_agent_kit::tool_executors::execute_knowledge_base_tool(
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
        if let Some((value, success)) =
            golish_agent_kit::tool_executors::execute_security_analysis_tool(
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

    if matches!(
        tool_name,
        "graph_add_entity"
            | "graph_add_relation"
            | "graph_search"
            | "graph_neighbors"
            | "graph_attack_paths"
    ) {
        if let Some((value, success)) = golish_agent_kit::tool_executors::execute_graph_tool(
            tool_name,
            tool_args,
            ctx.graph_backend.as_deref(),
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
        if let Some((value, success)) = executor.execute_tool(tool_name, tool_args).await {
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
                if let Some(hook) = &ctx.post_shell_hook {
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
                    let hook = Arc::clone(hook);
                    tokio::spawn(async move {
                        hook(command, stdout, Some(pp)).await;
                    });
                }

                // P-KG: regex-scan stdout for IP/CVE/URL hints and
                // upsert them into the graph in the background. The
                // sub-agent path does the same on its response text;
                // this catches facts that surface during raw shell
                // execution before any agent summarisation.
                if let Some(graph) = ctx.graph_backend.clone() {
                    if let Some(stdout) = v.get("stdout").and_then(|s| s.as_str()) {
                        if !stdout.is_empty() {
                            let stdout_owned = stdout.to_string();
                            let ws = ctx.workspace.read().await;
                            let pp = ws.to_string_lossy().to_string();
                            drop(ws);
                            let pid_opt =
                                if pp == "." || pp.is_empty() { None } else { Some(pp) };
                            tokio::spawn(async move {
                                let inserted = extract_and_upsert_entities(
                                    graph.as_ref(),
                                    &stdout_owned,
                                    pid_opt.as_deref(),
                                )
                                .await;
                                if inserted > 0 {
                                    tracing::info!(
                                        inserted,
                                        "[kg-extract] auto-upserted entities from run_pty_cmd stdout"
                                    );
                                }
                            });
                        }
                    }
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
    let project_id = {
        let ws = ctx.workspace.read().await;
        ws.to_string_lossy().to_string()
    };
    let project_id_opt = if project_id == "." || project_id.is_empty() {
        None
    } else {
        Some(project_id)
    };
    let briefing = build_sub_agent_briefing(
        ctx.events.db_tracker,
        ctx.graph_backend.as_deref(),
        project_id_opt.as_deref(),
        agent_id,
        task_desc,
    )
    .await;

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
                chain_persistence: ctx.chain_persistence.as_ref(),
                sub_agent_registry: Some(ctx.sub_agent_registry),
                post_shell_hook: ctx.post_shell_hook.clone(),
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
                chain_persistence: ctx.chain_persistence.as_ref(),
                sub_agent_registry: Some(ctx.sub_agent_registry),
                post_shell_hook: ctx.post_shell_hook.clone(),
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
            chain_persistence: ctx.chain_persistence.as_ref(),
            sub_agent_registry: Some(ctx.sub_agent_registry),
            post_shell_hook: ctx.post_shell_hook.clone(),
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

            // P-C (KG auto-extract): scan the sub-agent's response text
            // for IP/CVE/URL mentions and upsert them into the graph.
            // Fire-and-forget so it never blocks the agent loop; missing
            // graph backend / DB error is logged + ignored inside.
            if let Some(graph) = ctx.graph_backend.clone() {
                let response_text = result.response.clone();
                let pid = project_id_opt.clone();
                tokio::spawn(async move {
                    let inserted = extract_and_upsert_entities(
                        graph.as_ref(),
                        &response_text,
                        pid.as_deref(),
                    )
                    .await;
                    if inserted > 0 {
                        tracing::info!(
                            inserted,
                            "[kg-extract] auto-upserted entities from sub-agent response"
                        );
                    }
                });
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
