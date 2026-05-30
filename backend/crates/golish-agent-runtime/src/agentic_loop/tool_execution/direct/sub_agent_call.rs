//! `execute_sub_agent_call` — handles sub-agent tool calls (tool names
//! starting with `sub_agent_`), branching between built-in execution and the
//! registry-driven dispatch path, with best-effort dispatch lifecycle
//! persistence.

use anyhow::Result;
use rig::completion::CompletionModel as RigCompletionModel;
use serde_json::json;

use golish_core::utils::truncate_str;
use golish_sub_agents::{execute_sub_agent, SubAgentContext, SubAgentExecutorContext};

use super::super::super::sub_agent_dispatch::{
    build_sub_agent_briefing, execute_sub_agent_with_client,
};
use super::super::super::{AgenticLoopContext, ToolExecutionResult};
use golish_agent_kit::tool_executors::extract_and_upsert_entities;
use golish_agent_kit::tool_provider_impl::DefaultToolProvider;

/// Handle sub-agent tool calls (tool names starting with `sub_agent_`).
pub(super) async fn execute_sub_agent_call<M>(
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

    // P0-4: persist dispatch lifecycle so the next session can list
    // mid-flight invocations after a crash/restart. Best-effort —
    // missing tracker / repo / DB error leaves dispatch_id = None and
    // the lifecycle becomes a no-op.
    let dispatch_id: Option<uuid::Uuid> = if let Some(tracker) = ctx.events.db_tracker {
        if let Some(repo) = tracker.repo() {
            match golish_agent_kit::db_shim::sub_agent_dispatches::record_start(
                repo,
                tracker.session_uuid(),
                None, // parent_dispatch_id: tree-tracking deferred (P1)
                agent_id,
                Some(tool_id),
                0, // depth: tracking deferred (P1)
                tool_args,
            )
            .await
            {
                Ok(id) => Some(id),
                Err(e) => {
                    tracing::warn!(
                        agent_id = agent_id,
                        error = %e,
                        "[dispatch-track] record_start failed; proceeding without persistence",
                    );
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

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

    // P0-4: complement record_start above with record_finish so the
    // dispatch row gets `completed/failed` + result/error before we
    // hand control back to the caller. Best-effort like record_start.
    if let (Some(id), Some(tracker)) = (dispatch_id, ctx.events.db_tracker) {
        if let Some(repo) = tracker.repo() {
            let (status, result_json, err_msg) = match &result {
                Ok(r) => (
                    golish_agent_kit::db_traits::DispatchStatus::Completed,
                    Some(serde_json::json!({
                        "agent_id": r.agent_id,
                        "response": truncate_str(&r.response, 1000),
                        "success": r.success,
                        "duration_ms": r.duration_ms,
                    })),
                    None,
                ),
                Err(e) => (
                    golish_agent_kit::db_traits::DispatchStatus::Failed,
                    None,
                    Some(e.to_string()),
                ),
            };
            if let Err(e) = golish_agent_kit::db_shim::sub_agent_dispatches::record_finish(
                repo,
                id,
                status,
                result_json.as_ref(),
                err_msg.as_deref(),
            )
            .await
            {
                tracing::warn!(
                    dispatch_id = %id,
                    error = %e,
                    "[dispatch-track] record_finish failed",
                );
            }
        }
    }

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
                    let inserted =
                        extract_and_upsert_entities(graph.as_ref(), &response_text, pid.as_deref())
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
