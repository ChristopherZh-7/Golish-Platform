//! `execute_tool_direct_generic` — runs a tool when no human approval is
//! required (auto-approved or already approved).
//!
//! The private `execute_sub_agent_call` helper (which branches between
//! built-in sub-agent execution and the registry-driven sub-agent dispatch
//! path) lives in the [`sub_agent_call`] sibling module.

use std::sync::Arc;

use anyhow::Result;
use rig::completion::CompletionModel as RigCompletionModel;
use serde_json::json;

use golish_core::utils::is_tool_result_success;
use golish_sub_agents::SubAgentContext;

use super::super::{AgenticLoopContext, ToolExecutionResult};
use golish_agent_kit::tool_executors::{
    execute_ask_human_tool, execute_plan_patch_tool, execute_plan_tool, execute_web_fetch_tool,
    extract_and_upsert_entities,
};

mod sub_agent_call;
use self::sub_agent_call::execute_sub_agent_call;

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

    if tool_name == "update_plan_patch" {
        let (value, success) =
            execute_plan_patch_tool(ctx.plan_manager, ctx.events.event_tx, tool_args).await;
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

                // P0 · evidence ledger: record this tool run as an
                // `audit_role='evidence'` row (OpenFang-style hash chain) so the
                // harness gate can later cross-check the deliverable's
                // `evidence_refs` against real ledger ids. Scoped to harness-
                // staged subtasks with a known operation; failure only warns and
                // never blocks the tool path.
                if ctx.harness_stage.is_some() {
                    if let Some(tracker) = ctx.events.db_tracker {
                        if let Some(repo) = tracker.repo() {
                            // Operation grouping key for the hash chain: the
                            // task_id when a task scope is set, else the session
                            // uuid. (Per-task scoping via `set_task_context` has no
                            // callers yet; session keeps the chain working today
                            // and auto-upgrades to task_id once that is wired.)
                            let op_id = tracker.task_id().unwrap_or_else(|| tracker.session_uuid());
                            let ev_stdout = v
                                .get("stdout")
                                .and_then(|s| s.as_str())
                                .unwrap_or("")
                                .to_string();
                            let ev_subject = tool_args
                                .get("command")
                                .and_then(|c| c.as_str())
                                .filter(|c| !c.is_empty())
                                .unwrap_or(effective_tool_name);
                            if let Err(e) = repo
                                .evidence_append(
                                    op_id,
                                    None,
                                    ctx.events.session_id,
                                    tracker.project_path(),
                                    effective_tool_name,
                                    effective_tool_name,
                                    ev_subject,
                                    &ev_stdout,
                                )
                                .await
                            {
                                tracing::warn!(
                                    target: "harness::evidence",
                                    error = %e,
                                    "evidence append failed (continuing)"
                                );
                            }
                        }
                    }
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
                            let pid_opt = if pp == "." || pp.is_empty() {
                                None
                            } else {
                                Some(pp)
                            };
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
