//! `execute_tool_direct_generic` — runs a tool when no human approval is
//! required (auto-approved or already approved).
//!
//! The private `execute_sub_agent_call` helper (which branches between
//! built-in sub-agent execution and the registry-driven sub-agent dispatch
//! path) lives in the [`sub_agent_call`] sibling module.

use std::sync::Arc;

use anyhow::Result;
use rig::completion::CompletionModel as RigCompletionModel;
use serde_json::{json, Value};

use golish_core::utils::is_tool_result_success;
use golish_sub_agents::SubAgentContext;

use super::super::{AgenticLoopContext, ToolExecutionResult};
use golish_agent_kit::tool_executors::{
    execute_ask_human_tool, execute_plan_patch_tool, execute_plan_tool, execute_web_fetch_tool,
    extract_and_upsert_entities,
};

mod sub_agent_call;
use self::sub_agent_call::execute_sub_agent_call;

#[allow(dead_code)] // wired into stage_run_call by Task 9 composition work
pub mod candidate_analysis_agent_runner;
pub mod candidate_verification;
mod stage_team_scheduler;

// `pub(crate)` so `execution_mode::selection_apply` can pull the tool definition
// (co-located with its handler) when exposing `stage_run` to the primary agent.
pub(crate) mod stage_run_call;
use self::stage_run_call::execute_stage_run;

#[derive(Debug, Clone, PartialEq, Eq)]
struct StructuredStorageHookPayload {
    command: String,
    stdout: String,
}

fn pentest_underlying_invocation(
    tool_name: &str,
    tool_args: &serde_json::Value,
    result: &serde_json::Value,
) -> Option<(String, String)> {
    if tool_name == "pentest_run" {
        let tool = tool_args.get("tool_name").and_then(|v| v.as_str())?;
        let args = tool_args.get("args").and_then(|v| v.as_str()).unwrap_or("");
        return Some((tool.to_string(), args.to_string()));
    }
    let tool = result.get("wrapped_tool_name").and_then(|v| v.as_str())?;
    let args = result
        .get("wrapped_args")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    Some((tool.to_string(), args.to_string()))
}

fn structured_storage_hook_payload(
    tool_name: &str,
    tool_args: &serde_json::Value,
    result: &serde_json::Value,
    success: bool,
) -> Option<StructuredStorageHookPayload> {
    if result
        .get("structured_storage_disabled")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return None;
    }
    let stdout = result
        .get("stdout")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    if stdout.trim().is_empty() {
        return None;
    }

    let command = if tool_name == "run_pty_cmd" && success {
        tool_args
            .get("command")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string()
    } else if let Some((tool, args)) = pentest_underlying_invocation(tool_name, tool_args, result) {
        result
            .get("command")
            .and_then(|c| c.as_str())
            .map(str::to_string)
            .or_else(|| Some(format!("{tool} {args}").trim().to_string()))
            .unwrap_or_default()
    } else {
        String::new()
    };

    if command.trim().is_empty() {
        return None;
    }

    Some(StructuredStorageHookPayload { command, stdout })
}

fn generic_pentest_evidence_enabled(result: &serde_json::Value) -> bool {
    !result
        .get("generic_evidence_disabled")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn combined_stdout_stderr(result: &serde_json::Value) -> String {
    let stdout = result.get("stdout").and_then(|s| s.as_str()).unwrap_or("");
    let stderr = result.get("stderr").and_then(|s| s.as_str()).unwrap_or("");
    if stderr.trim().is_empty() {
        stdout.to_string()
    } else {
        format!("{stdout}\n[stderr]\n{stderr}")
    }
}

fn stage_evidence_operation_id(
    harness_operation_id: Option<uuid::Uuid>,
) -> std::result::Result<uuid::Uuid, &'static str> {
    harness_operation_id.ok_or(
        "active harness stage has no durable operation id; refusing to book foreign evidence",
    )
}

fn scoping_recon_requested_organization_id(
    harness_stage: Option<golish_agent_kit::harness::StageKind>,
    tool_name: &str,
    tool_args: &serde_json::Value,
) -> Option<uuid::Uuid> {
    if harness_stage != Some(golish_agent_kit::harness::StageKind::Scoping)
        || tool_name != "recon_discover_subsidiaries"
    {
        return None;
    }
    tool_args
        .get("organization_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| value.parse::<uuid::Uuid>().ok())
}

async fn recon_evidence_organization_id(
    repo: &dyn golish_agent_kit::db_traits::DbRepoProvider,
    harness_stage: Option<golish_agent_kit::harness::StageKind>,
    harness_operation_id: Option<uuid::Uuid>,
    stage_execution_id: Option<uuid::Uuid>,
    harness_org_id: Option<uuid::Uuid>,
    tool_name: &str,
    tool_args: &serde_json::Value,
) -> std::result::Result<uuid::Uuid, String> {
    if let Some(organization_id) = harness_org_id {
        return Ok(organization_id);
    }
    let organization_id =
        scoping_recon_requested_organization_id(harness_stage, tool_name, tool_args).ok_or_else(
            || "active recon has no trusted organization id for evidence persistence".to_string(),
        )?;
    let operation_id = stage_evidence_operation_id(harness_operation_id)?;
    let stage_execution_id = stage_execution_id.ok_or_else(|| {
        "active Scoping recon has no exact stage execution for evidence persistence".to_string()
    })?;
    let authorized = repo
        .scoping_passive_recon_organization_authorized(
            operation_id,
            stage_execution_id,
            organization_id,
        )
        .await
        .map_err(|error| format!("Scoping passive recon authorization failed: {error}"))?;
    if !authorized {
        return Err(
            "Scoping passive recon organization was not authorized by the exact human choice"
                .to_string(),
        );
    }
    Ok(organization_id)
}

async fn record_recon_passive_evidence(
    tracker: Option<&golish_agent_kit::db_tracking::DbTracker>,
    session_id: Option<&str>,
    harness_stage: Option<golish_agent_kit::harness::StageKind>,
    harness_operation_id: Option<uuid::Uuid>,
    stage_execution_id: Option<uuid::Uuid>,
    harness_org_id: Option<uuid::Uuid>,
    tool_name: &str,
    tool_args: &serde_json::Value,
    result: &serde_json::Value,
    success: bool,
) -> std::result::Result<Option<i64>, String> {
    if !matches!(
        tool_name,
        "recon_discover_subsidiaries" | "recon_map_assets" | "recon_lookup_whois"
    ) || harness_stage.is_none()
    {
        return Ok(None);
    }

    let tracker = tracker.ok_or_else(|| {
        "active Target Intel recon has no DB tracker for evidence persistence".to_string()
    })?;
    let repo = tracker.repo().ok_or_else(|| {
        "active Target Intel recon has no DB repository for evidence persistence".to_string()
    })?;
    let op_id = stage_evidence_operation_id(harness_operation_id)?;
    let organization_id = recon_evidence_organization_id(
        repo,
        harness_stage,
        harness_operation_id,
        stage_execution_id,
        harness_org_id,
        tool_name,
        tool_args,
    )
    .await?;
    let ev_subject = result
        .get("company")
        .and_then(|c| c.as_str())
        .filter(|c| !c.is_empty())
        .unwrap_or(tool_name);
    let ev_raw = serde_json::to_string(result).unwrap_or_default();
    // PR2 · enrich's subject is a COMPANY name, not an in-scope asset
    // (domain/IP) — no deterministic asset, so no facts. Subsidiary discovery
    // DOES derive an org-level GOLISH-INTEL-SUBSIDIARY fact from its structured
    // summary so "ran → 0 qualifying child" can be checked_empty instead of
    // not_attempted (I8).
    let facts = (success && tool_name == "recon_discover_subsidiaries")
        .then(|| golish_agent_kit::harness::evidence_facts::subsidiary_discovery_facts(result))
        .flatten();

    let append_result = repo
        .evidence_append_for_organization(
            op_id,
            organization_id,
            None,
            session_id,
            tracker.project_path(),
            tool_name,
            tool_name,
            ev_subject,
            &ev_raw,
            facts.as_ref().map(|(t, a, o)| (*t, a.as_str(), *o)),
        )
        .await;

    match append_result {
        Ok(id) => {
            tracing::info!(
                target: "harness::evidence",
                tool = %tool_name,
                evidence_id = id,
                "recon passive evidence appended; surfacing id to agent"
            );

            if let (Some(rid), Some((tech, asset, outcome))) = (
                session_id,
                facts.as_ref().map(|(t, a, o)| (*t, a.as_str(), *o)),
            ) {
                if let Err(e) = repo
                    .upsert_technique_outcome(
                        organization_id,
                        rid,
                        asset,
                        tech,
                        outcome,
                        Some(tool_name),
                        Some(ev_subject),
                        &[id],
                    )
                    .await
                {
                    return Err(format!(
                        "Target Intel technique outcome persistence failed: {e}"
                    ));
                }
            }

            if tool_name == "recon_discover_subsidiaries" {
                if let Some(rid) = session_id {
                    for lead in
                        golish_agent_kit::harness::evidence_facts::expansion_leads_from_subsidiary_discovery(result)
                    {
                        if let Err(e) = repo
                            .enqueue_expansion_lead(
                                organization_id,
                                rid,
                                lead.lead_type,
                                &lead.lead_value,
                                Some(tool_name),
                                lead.confidence,
                                &[id],
                            )
                            .await
                        {
                            tracing::warn!(
                                target: "harness::evidence",
                                error = %e,
                                "expansion_queue enqueue failed (continuing)"
                            );
                        }
                    }
                }
            }

            if let Some(rid) = session_id {
                let source_rows = recon_source_query_rows_for_call(tool_name, tool_args, result);
                for row in &source_rows {
                    if let Err(e) = repo
                        .upsert_source_query(
                            organization_id,
                            rid,
                            &row.source,
                            &row.query,
                            &row.target,
                            row.technique,
                            row.status,
                            &[id],
                        )
                        .await
                    {
                        return Err(format!(
                            "Target Intel exact source status persistence failed for {} {}: {e}",
                            row.source, row.query
                        ));
                    }
                }
                // The source rows above form one logical result. They are
                // intentionally followed by a completion marker, so a DB error
                // after only the generic row cannot make the duplicate guard
                // skip a retry before exact technique rows were persisted.
                for row in source_status_completion_rows(tool_name, &source_rows) {
                    if let Err(e) = repo
                        .upsert_source_query(
                            organization_id,
                            rid,
                            &row.source,
                            &row.query,
                            &row.target,
                            row.technique,
                            row.status,
                            &[id],
                        )
                        .await
                    {
                        return Err(format!(
                            "Target Intel source-status completion marker persistence failed for {}: {e}",
                            row.target
                        ));
                    }
                }
            }

            if should_refresh_target_intel_dns(tool_name, success) {
                if let Some(rid) = session_id {
                    match repo
                        .mark_target_intel_dns_empty_outcomes(organization_id, rid, &[id])
                        .await
                    {
                        Ok(count) if count > 0 => tracing::info!(
                            target: "harness::evidence",
                            tool = %tool_name,
                            organization_id = %organization_id,
                            marked = count,
                            "target_intel DNS attempt outcomes recorded"
                        ),
                        Ok(_) => {}
                        Err(e) => {
                            return Err(format!(
                                "Target Intel DNS outcome persistence failed: {e}"
                            ));
                        }
                    }
                }
            }

            Ok(Some(id))
        }
        Err(e) => Err(format!("Target Intel evidence persistence failed: {e}")),
    }
}

fn should_refresh_target_intel_dns(tool_name: &str, success: bool) -> bool {
    success && tool_name == "recon_map_assets"
}

fn is_security_analysis_direct_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "log_operation"
            | "discover_apis"
            | "save_js_analysis"
            | "fingerprint_target"
            | "log_scan_result"
            | "query_target_data"
            | "list_in_scope_targets"
            | "list_attack_surface_seeds"
            | "stage_worklist_status"
            | "stage_worklist_next"
            | "check_stage_asset_coverage"
            | "list_recent_evidence"
    )
}

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
        let (value, success) = execute_plan_tool(
            ctx.plan_manager,
            ctx.events.event_tx,
            tool_args,
            ctx.harness_stage.map(|s| s.as_str()),
        )
        .await;
        return Ok(ToolExecutionResult { value, success });
    }

    if tool_name == "update_plan_patch" {
        let (value, success) = execute_plan_patch_tool(
            ctx.plan_manager,
            ctx.events.event_tx,
            tool_args,
            ctx.harness_stage.map(|s| s.as_str()),
        )
        .await;
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

    if is_security_analysis_direct_tool(tool_name) {
        let ws_path = ctx.workspace.read().await;
        let project_path_str = ws_path.to_string_lossy().to_string();
        drop(ws_path);
        if let Some((value, success)) = Box::pin(
            golish_agent_kit::tool_executors::execute_security_analysis_tool(
                tool_name,
                tool_args,
                ctx.events.db_tracker,
                Some(project_path_str.as_str()),
                ctx.events.session_id,
                ctx.harness_org_id,
                ctx.harness_stage,
                ctx.harness_operation_id,
            ),
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

    // `stage_run` — fan the current stage's specialist out per in-scope org
    // (design 2026-06-13-stage-run-fanout). Special-cased here (like
    // `sub_agent_*`) because it dispatches sub-agents per org, which needs the
    // agentic-loop context a registry tool cannot reach.
    if tool_name == "stage_run" {
        return execute_stage_run(tool_args, ctx, model, context, tool_id).await;
    }

    if tool_name.starts_with("sub_agent_") {
        return execute_sub_agent_call(tool_name, tool_args, ctx, model, context, tool_id).await;
    }

    let effective_tool_name = if tool_name == "run_command" {
        "run_pty_cmd"
    } else {
        tool_name
    };

    // P2-d · guardrail: inside a harness stage, block dangerous tool calls
    // (SSRF / destructive shell) BEFORE they execute. This is the execution
    // chokepoint (catches even calls that bypass the dispatch-phase gate); a
    // Block returns a synthetic error result instead of running the tool.
    if let Some(reason) = guardrail_block_reason(ctx.harness_stage, effective_tool_name, tool_args)
    {
        tracing::warn!(
            target: "harness::guardrail",
            tool = %effective_tool_name,
            reason = %reason,
            "tool call BLOCKED by guardrail"
        );
        return Ok(ToolExecutionResult {
            value: json!({
                "error": format!("blocked by guardrail: {reason}"),
                "blocked_by_guardrail": true,
            }),
            success: false,
        });
    }

    if let Some(result) = duplicate_source_query_guard(effective_tool_name, tool_args, ctx).await {
        return Ok(result);
    }

    let registry = ctx.tool_registry.read().await;
    let result = Box::pin(registry.execute_tool(effective_tool_name, tool_args.clone())).await;

    match &result {
        Ok(v) => {
            let is_success = is_tool_result_success(v);
            // P1 · the evidence-ledger id appended for this tool run (if any), so
            // we can surface it to the agent below — letting it cite a REAL id in
            // its StageDeliverable instead of fabricating one (which the gate
            // would then BLOCK).
            let mut appended_evidence_ids: Vec<i64> = Vec::new();

            if let Some(payload) =
                structured_storage_hook_payload(effective_tool_name, tool_args, v, is_success)
            {
                if let Some(hook) = &ctx.post_shell_hook {
                    let ws = ctx.workspace.read().await;
                    let pp = ws.to_string_lossy().to_string();
                    drop(ws);
                    let hook = Arc::clone(hook);
                    let org_id = ctx.harness_org_id;
                    if ctx.harness_stage.is_some() {
                        hook(payload.command, payload.stdout, Some(pp), org_id).await;
                    } else {
                        tokio::spawn(async move {
                            hook(payload.command, payload.stdout, Some(pp), org_id).await;
                        });
                    }
                }
            }

            if effective_tool_name == "run_pty_cmd" && is_success {
                // P0 · evidence ledger: record this tool run as an
                // `audit_role='evidence'` row (OpenFang-style hash chain) so the
                // harness gate can later cross-check the deliverable's
                // `evidence_refs` against real ledger ids. Scoped to harness-
                // staged subtasks with a known operation; failure only warns and
                // never blocks the tool path.
                if ctx.harness_stage.is_some() {
                    if let Some(tracker) = ctx.events.db_tracker {
                        if let Some(repo) = tracker.repo() {
                            let ev_output = combined_stdout_stderr(v);
                            let ev_subject = tool_args
                                .get("command")
                                .and_then(|c| c.as_str())
                                .filter(|c| !c.is_empty())
                                .unwrap_or(effective_tool_name);
                            // PR2 (coverage 投影) · deterministically derive
                            // (technique, asset, outcome) from the shell command;
                            // unmapped commands stay None (never project).
                            let facts = golish_agent_kit::harness::evidence_facts::coverage_facts_from_command(ev_subject)
                                .map(|(technique, asset)| {
                                    let outcome = golish_agent_kit::harness::evidence_facts::coverage_outcome_for_run(technique, &ev_output, true, false);
                                    (technique, asset, outcome)
                                });
                            let append_result = match (
                                stage_evidence_operation_id(ctx.harness_operation_id),
                                ctx.harness_org_id,
                            ) {
                                (Ok(op_id), Some(organization_id)) => {
                                    repo.evidence_append_for_organization(
                                        op_id,
                                        organization_id,
                                        None,
                                        ctx.events.session_id,
                                        tracker.project_path(),
                                        effective_tool_name,
                                        effective_tool_name,
                                        ev_subject,
                                        &ev_output,
                                        facts.as_ref().map(|(t, a, o)| (*t, a.as_str(), *o)),
                                    )
                                    .await
                                }
                                (Err(error), _) => Err(anyhow::anyhow!(error)),
                                (_, None) => Err(anyhow::anyhow!(
                                    "active harness stage has no organization id; refusing to book unowned evidence"
                                )),
                            };
                            match append_result {
                                Ok(id) => {
                                    appended_evidence_ids.push(id);
                                    tracing::info!(
                                        target: "harness::evidence",
                                        tool = %effective_tool_name,
                                        evidence_id = id,
                                        "evidence appended; surfacing id to agent"
                                    );
                                }
                                Err(e) => tracing::warn!(
                                    target: "harness::evidence",
                                    error = %e,
                                    "evidence append failed (continuing)"
                                ),
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
                                let stats = extract_and_upsert_entities(
                                    graph.as_ref(),
                                    &stdout_owned,
                                    pid_opt.as_deref(),
                                )
                                .await;
                                if stats.nodes > 0 || stats.edges > 0 {
                                    tracing::info!(
                                        nodes = stats.nodes,
                                        edges = stats.edges,
                                        "[kg-extract] auto-upserted from run_pty_cmd stdout"
                                    );
                                }
                            });
                        }
                    }
                }
            }

            // P3a · `pentest_run` is the primary scan path (nmap / httpx / dig /
            // …). Like `run_pty_cmd` it must append its output to the evidence
            // ledger so scanning stages produce REAL ids the deliverable can
            // cite (otherwise every scan-stage deliverable is "fabricated" and
            // the gate loops). Kept as a separate block so the working
            // run_pty_cmd path above is untouched.
            // NOTE: NOT gated on `is_success`. A mapped coverage probe that FAILS
            // (non-zero exit / timeout / flaky external service such as crt.sh) must
            // still book a terminal fact — otherwise the cell stays not_attempted
            // and the deterministic gate loops forever on a service it can never
            // reach.
            if (effective_tool_name == "pentest_run"
                || v.get("wrapped_tool_name")
                    .and_then(|s| s.as_str())
                    .is_some())
                && ctx.harness_stage.is_some()
                && generic_pentest_evidence_enabled(v)
            {
                if let Some(tracker) = ctx.events.db_tracker {
                    if let Some(repo) = tracker.repo() {
                        let ev_output = combined_stdout_stderr(v);
                        let (pt_tool, pt_args) =
                            pentest_underlying_invocation(effective_tool_name, tool_args, v)
                                .unwrap_or_else(|| {
                                    (effective_tool_name.to_string(), String::new())
                                });
                        let ev_subject = if pt_args.is_empty() {
                            pt_tool.clone()
                        } else {
                            format!("{pt_tool} {pt_args}")
                        };
                        // PR2 · "{tool} {args}" has the same shape as a shell
                        // command line; same deterministic facts derivation. A FAILED
                        // run resolves to checked_empty (I8) via
                        // `coverage_outcome_for_run`; success keeps the
                        // output-derived verdict.
                        // T2 (设计 2026-06-23-failure-outcome-not-checked-empty): gray-switch
                        // 决定失败检查记 error 还是 empty（默认 off = empty，旧行为）。
                        let distinguish_failure =
                            golish_agent_kit::harness::feature_flags::failure_outcome_error_enabled(
                            );
                        let facts = golish_agent_kit::harness::evidence_facts::coverage_facts_from_command(&ev_subject)
                            .map(|(technique, asset)| {
                                let outcome = golish_agent_kit::harness::evidence_facts::coverage_outcome_for_run(technique, &ev_output, is_success, distinguish_failure);
                                (technique, asset, outcome)
                            });
                        // A successful run always books (its ledger id is citable); a
                        // FAILED run books only when it carries a derived coverage
                        // fact, so the coverage cell gets its terminal status — a
                        // failed unmapped command has nothing citable and is skipped.
                        if is_success || facts.is_some() {
                            // On failure stdout is usually empty; record the error so
                            // the ledger row carries the real reason (crt.sh 502 /
                            // timeout) for audit, not a blank body.
                            let ev_body = if is_success || !ev_output.trim().is_empty() {
                                ev_output.clone()
                            } else {
                                v.get("error")
                                    .and_then(|s| s.as_str())
                                    .unwrap_or("coverage check failed")
                                    .to_string()
                            };
                            let append_result = match (
                                stage_evidence_operation_id(ctx.harness_operation_id),
                                ctx.harness_org_id,
                            ) {
                                (Ok(op_id), Some(organization_id)) => {
                                    repo.evidence_append_for_organization(
                                        op_id,
                                        organization_id,
                                        None,
                                        ctx.events.session_id,
                                        tracker.project_path(),
                                        pt_tool.as_str(),
                                        pt_tool.as_str(),
                                        &ev_subject,
                                        &ev_body,
                                        facts.as_ref().map(|(t, a, o)| (*t, a.as_str(), *o)),
                                    )
                                    .await
                                }
                                (Err(error), _) => Err(anyhow::anyhow!(error)),
                                (_, None) => Err(anyhow::anyhow!(
                                    "active harness stage has no organization id; refusing to book unowned evidence"
                                )),
                            };
                            match append_result {
                                Ok(id) => {
                                    appended_evidence_ids.push(id);
                                    tracing::info!(
                                        target: "harness::evidence",
                                        tool = %pt_tool,
                                        evidence_id = id,
                                        success = is_success,
                                        "pentest_run evidence appended; surfacing id to agent"
                                    );
                                    // PR-C step2b (#4/E3, 设计 2026-06-23-technique-outcomes-
                                    // provenance): 同步 upsert technique_outcomes（provenance
                                    // 物化）。**始终写**（无灰度开关）；仅 org 绑定 + 有派生 fact
                                    // 时写；非致命 warn（证据为底、表为物化，失败不回滚证据）。
                                    if let (Some(org_id), Some(rid), Some((tech, asset, outcome))) = (
                                        ctx.harness_org_id,
                                        ctx.events.session_id,
                                        facts.as_ref().map(|(t, a, o)| (*t, a.as_str(), *o)),
                                    ) {
                                        if let Err(e) = repo
                                            .upsert_technique_outcome(
                                                org_id,
                                                rid,
                                                asset,
                                                tech,
                                                outcome,
                                                Some(pt_tool.as_str()),
                                                Some(ev_subject.as_str()),
                                                &[id],
                                            )
                                            .await
                                        {
                                            tracing::warn!(
                                                target: "harness::evidence",
                                                error = %e,
                                                "technique_outcomes upsert failed (continuing)"
                                            );
                                        }
                                    }
                                    // #5 (设计 2026-06-23-source-query-log): 同步 upsert
                                    // source_query_log（逐源查询日志，比 technique_outcomes 更细：
                                    // 每 source 各一行）。**始终写**（无灰度开关）；仅 org 绑定 + 有
                                    // 派生 fact 时写；非致命 warn。消费模型 A：仅写 + reviewer 读，
                                    // gate 不读。source=工具名、query=命令、target=asset。
                                    if let (Some(org_id), Some(rid), Some((tech, asset, outcome))) = (
                                        ctx.harness_org_id,
                                        ctx.events.session_id,
                                        facts.as_ref().map(|(t, a, o)| (*t, a.as_str(), *o)),
                                    ) {
                                        if let Err(e) = repo
                                            .upsert_source_query(
                                                org_id,
                                                rid,
                                                pt_tool.as_str(),
                                                ev_subject.as_str(),
                                                asset,
                                                Some(tech),
                                                outcome,
                                                &[id],
                                            )
                                            .await
                                        {
                                            tracing::warn!(
                                                target: "harness::evidence",
                                                error = %e,
                                                "source_query_log upsert failed (continuing)"
                                            );
                                        }
                                    }
                                }
                                Err(e) => tracing::warn!(
                                    target: "harness::evidence",
                                    error = %e,
                                    "pentest_run evidence append failed (continuing)"
                                ),
                            }
                        }
                    }
                }
            }

            let mut recon_persistence_error = None;
            let mut recon_outcome_persisted = false;
            match record_recon_passive_evidence(
                ctx.events.db_tracker,
                ctx.events.session_id,
                ctx.harness_stage,
                ctx.harness_operation_id,
                ctx.stage_execution_id,
                ctx.harness_org_id,
                effective_tool_name,
                tool_args,
                v,
                is_success,
            )
            .await
            {
                Ok(Some(id)) => {
                    appended_evidence_ids.push(id);
                    recon_outcome_persisted = true;
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(
                        target: "harness::evidence",
                        tool = %effective_tool_name,
                        %error,
                        "Target Intel persistence incomplete; returning a retryable tool error"
                    );
                    recon_persistence_error = Some(error);
                }
            }

            // NOTE (design 2026-07-03): enumeration four-axis evidence
            // (GOLISH-ENUM-JS/DIR/PARAM/JSAPI) is now booked by the
            // `golish-pentest-app` bridge tools themselves (they run on both the
            // primary-agent direct path AND the enumerator sub-agent path), so
            // the old primary-only `record_enumeration_bridge_evidence` hook was
            // removed to avoid double-booking JS.

            // P1 · surface the appended evidence id so the agent cites a REAL
            // ledger id in its StageDeliverable. Additive `_evidence_id` field;
            // absent when nothing was appended (non-shell tool / no stage).
            let mut value = v.clone();
            if let Some(id) = appended_evidence_ids.last().copied() {
                if let Some(obj) = value.as_object_mut() {
                    obj.insert("_evidence_id".to_string(), json!(id));
                    obj.insert("_evidence_ids".to_string(), json!(appended_evidence_ids));
                }
            }
            if recon_outcome_persisted {
                if let Some(object) = value.as_object_mut() {
                    object.insert("outcome_persisted".to_string(), Value::Bool(true));
                }
            }
            if recon_persistence_error.is_some() {
                if let Some(object) = value.as_object_mut() {
                    object.insert(
                        "error".to_string(),
                        Value::String(
                            "Target Intel evidence/source status persistence was incomplete; retry this recon action"
                                .to_string(),
                        ),
                    );
                    object.insert(
                        "completion_state".to_string(),
                        Value::String("partial".to_string()),
                    );
                    object.insert("outcome_persisted".to_string(), Value::Bool(false));
                }
            }

            // Q3 ③ · stage-annotate `pentest_list_tools` so the agent sees, per
            // tool, whether the active stage permits it — instead of discovering
            // the boundary by hitting a BLOCK. The verdict uses the same
            // `stage_allows` predicate the dispatch guard enforces with, so the
            // annotation never disagrees with what the gate will actually do.
            if effective_tool_name == "pentest_list_tools" && is_success {
                if let Some(kind) = ctx.harness_stage {
                    if let Ok(spec) = golish_agent_kit::harness::load_embedded_stage_spec(kind) {
                        crate::agentic_loop::tool_execution::stage_list_tools::annotate_pentest_list_tools(
                            &mut value,
                            &spec.id,
                            &spec.allowed_tool_types,
                        );
                    }
                }
            }
            Ok(ToolExecutionResult {
                value,
                success: is_success && recon_persistence_error.is_none(),
            })
        }
        Err(e) => Ok(ToolExecutionResult {
            value: json!({"error": e.to_string()}),
            success: false,
        }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReconSourceQueryRow {
    source: String,
    query: String,
    target: String,
    technique: Option<&'static str>,
    status: &'static str,
}

const SOURCE_STATUS_COMPLETE_SOURCE: &str = "golish-runtime";
const SOURCE_STATUS_COMPLETE_SUFFIX: &str = "source-status-complete";

fn recon_source_query_rows_for_call(
    tool_name: &str,
    tool_args: &serde_json::Value,
    result: &serde_json::Value,
) -> Vec<ReconSourceQueryRow> {
    let mut rows = recon_source_query_rows(tool_name, result);
    // A targeted repair is not the organization-wide survey. Bind every
    // otherwise broad top-level row to the requested domain so it cannot create
    // a broad completion marker and suppress a later full survey.
    if let Some(target) = duplicate_guard_target(tool_name, tool_args) {
        for row in &mut rows {
            if row.target.is_empty() {
                row.target.clone_from(&target);
            }
        }
    }
    rows
}

fn source_status_completion_rows(
    tool_name: &str,
    source_rows: &[ReconSourceQueryRow],
) -> Vec<ReconSourceQueryRow> {
    let Some(action) = duplicate_guard_query(tool_name) else {
        return Vec::new();
    };
    let targets: std::collections::BTreeSet<String> =
        source_rows.iter().map(|row| row.target.clone()).collect();
    targets
        .into_iter()
        .map(|target| ReconSourceQueryRow {
            source: SOURCE_STATUS_COMPLETE_SOURCE.to_string(),
            query: format!("{action}:{SOURCE_STATUS_COMPLETE_SUFFIX}"),
            target,
            technique: None,
            status: "found",
        })
        .collect()
}

fn recon_source_query_rows(
    tool_name: &str,
    result: &serde_json::Value,
) -> Vec<ReconSourceQueryRow> {
    match tool_name {
        "recon_discover_subsidiaries" | "recon_map_assets" => {
            let query = result
                .get("action")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(tool_name)
                .to_string();
            let mut rows = provider_status_rows(result, &query, "");
            if tool_name == "recon_map_assets" {
                rows.extend(technique_status_rows(result, &query, ""));
                rows.extend(domain_expansion_source_query_rows(result, &query));
            }
            if rows.is_empty() && result_has_error(result) {
                rows.push(ReconSourceQueryRow {
                    source: tool_name.to_string(),
                    query,
                    target: String::new(),
                    technique: None,
                    status: "error",
                });
            }
            rows
        }
        "recon_lookup_whois" => {
            let query = result
                .get("action")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(tool_name)
                .to_string();
            let typed_status = result
                .get("whois_status")
                .or_else(|| result.get("whoisStatus"))
                .and_then(|value| value.as_str())
                .and_then(|status| match status {
                    "found" => Some("found"),
                    "empty" | "checked_empty" => Some("empty"),
                    "error" => Some("error"),
                    "blocked" => Some("blocked"),
                    _ => None,
                });
            let fallback_status = if result_has_error(result) {
                "error"
            } else {
                result
                    .get("whois_landed")
                    .or_else(|| result.get("whoisLanded"))
                    .and_then(|v| v.as_bool())
                    .map_or("empty", |landed| if landed { "found" } else { "empty" })
            };
            vec![ReconSourceQueryRow {
                source: "rdap".to_string(),
                query,
                target: String::new(),
                technique: Some("GOLISH-INTEL-WHOIS"),
                status: typed_status.unwrap_or(fallback_status),
            }]
        }
        _ => Vec::new(),
    }
}

fn result_has_error(result: &serde_json::Value) -> bool {
    result
        .get("error")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|error| !error.trim().is_empty())
}

fn domain_expansion_source_query_rows(
    result: &serde_json::Value,
    query: &str,
) -> Vec<ReconSourceQueryRow> {
    result
        .get("domainExpansions")
        .or_else(|| result.get("domain_expansions"))
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .flat_map(|expansion| {
            let target = expansion
                .get("domain")
                .and_then(|value| value.as_str())
                .map(normalize_source_query_target)
                .unwrap_or_default();
            let mut rows = provider_status_rows(expansion, query, &target);
            rows.extend(technique_status_rows(expansion, query, &target));
            rows
        })
        .collect()
}

fn technique_status_rows(
    result: &serde_json::Value,
    query: &str,
    target: &str,
) -> Vec<ReconSourceQueryRow> {
    result
        .get("techniqueStatus")
        .or_else(|| result.get("technique_status"))
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let source = item
                .get("source")
                .or_else(|| item.get("providerId"))
                .or_else(|| item.get("provider_id"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())?;
            let technique = item
                .get("technique")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| value.starts_with("GOLISH-INTEL-"))?;
            let raw_status = item
                .get("status")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            Some(ReconSourceQueryRow {
                source: source.to_string(),
                // `source_query_log` is unique without technique in its key, so
                // exact rows get a deterministic query suffix while the generic
                // provider row remains available as the survey-attempt proof.
                query: format!("{query}:{technique}"),
                target: target.to_string(),
                technique: Some(match technique {
                    "GOLISH-INTEL-DNS" => "GOLISH-INTEL-DNS",
                    "GOLISH-INTEL-WHOIS" => "GOLISH-INTEL-WHOIS",
                    "GOLISH-INTEL-ASN" => "GOLISH-INTEL-ASN",
                    "GOLISH-INTEL-CT" => "GOLISH-INTEL-CT",
                    "GOLISH-INTEL-SUBDOMAIN" => "GOLISH-INTEL-SUBDOMAIN",
                    "GOLISH-INTEL-OSINT" => "GOLISH-INTEL-OSINT",
                    _ => return None,
                }),
                status: provider_status_for_source_query(raw_status),
            })
        })
        .collect()
}

fn provider_status_rows(
    result: &serde_json::Value,
    query: &str,
    target: &str,
) -> Vec<ReconSourceQueryRow> {
    result
        .get("providerStatus")
        .or_else(|| result.get("provider_status"))
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let source = item
                .get("providerId")
                .or_else(|| item.get("provider_id"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())?;
            let raw_status = item.get("status").and_then(|v| v.as_str()).unwrap_or("");
            Some(ReconSourceQueryRow {
                source: source.to_string(),
                query: query.to_string(),
                target: target.to_string(),
                technique: None,
                status: provider_status_for_source_query(raw_status),
            })
        })
        .collect()
}

fn normalize_source_query_target(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("*.")
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

fn provider_status_for_source_query(status: &str) -> &'static str {
    match status {
        "found" | "completed" | "Completed" => "found",
        "empty" | "checked_empty" | "CheckedEmpty" => "empty",
        "blocked" | "unavailable" | "Unavailable" => "blocked",
        "error" | "failed" | "Failed" => "error",
        _ => "error",
    }
}

#[cfg(test)]
mod direct_tool_routing_tests {
    use super::*;

    #[test]
    fn attack_surface_seeds_routes_through_security_analysis_executor() {
        assert!(
            is_security_analysis_direct_tool("list_attack_surface_seeds"),
            "tool_list exposes list_attack_surface_seeds to the main stage orchestrator, \
             so direct execution must route it instead of falling through to Unknown tool"
        );
        assert!(
            is_security_analysis_direct_tool("check_stage_asset_coverage"),
            "stage coverage preflight is exposed to active stages and must route through \
             the security-analysis executor"
        );
        assert!(
            is_security_analysis_direct_tool("stage_worklist_next"),
            "stage worklist tools are exposed to active stages and must route through \
             the security-analysis executor"
        );
        assert!(
            is_security_analysis_direct_tool("stage_worklist_status"),
            "stage worklist status is exposed to active stages and must route through \
             the security-analysis executor"
        );
        assert!(
            is_security_analysis_direct_tool("list_recent_evidence"),
            "list_recent_evidence is exposed to active stages so workers can cite real \
             evidence ids and must route through the security-analysis executor"
        );
    }
}

async fn duplicate_source_query_guard(
    tool_name: &str,
    tool_args: &serde_json::Value,
    ctx: &AgenticLoopContext<'_>,
) -> Option<ToolExecutionResult> {
    let query = duplicate_guard_query(tool_name)?;
    let target = duplicate_guard_target(tool_name, tool_args);
    if ctx.harness_stage != Some(golish_agent_kit::harness::StageKind::TargetIntel) {
        return None;
    }
    let org_id = ctx.harness_org_id?;
    let run_id = ctx.events.session_id?;
    let repo = ctx.events.db_tracker?.repo()?;
    let rows = match repo.source_query_facts(org_id, run_id).await {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(
                target: "harness::duplicate_guard",
                %error,
                "source-query duplicate check failed; executing the requested tool"
            );
            return None;
        }
    };
    let relevant: Vec<_> = rows
        .iter()
        .filter(|row| {
            source_query_belongs_to_action(&row.query, query)
                && target.as_deref().is_none_or(|target| {
                    row.target == target
                        || (row.target.is_empty()
                            && row.query == "map_assets:GOLISH-INTEL-DNS"
                            && !is_skippable_source_query_status(&row.status))
                })
        })
        .collect();
    let has_completion_marker = relevant.iter().any(|row| {
        source_status_completion_marker_matches(
            &row.source,
            &row.query,
            &row.target,
            query,
            target.as_deref(),
        )
    });
    if !has_completion_marker {
        return None;
    }
    // The generic provider row and its exact `action:GOLISH-INTEL-*` rows are
    // one logical attempt. A generic `found` must never hide a sibling exact
    // `error`/`running` row, otherwise the retry guard makes a non-terminal
    // technique permanently unreachable for the rest of this run.
    if !source_query_statuses_all_terminal(relevant.iter().map(|row| row.status.as_str())) {
        return None;
    }
    if tool_name == "recon_map_assets" {
        let outcomes = repo.technique_outcome_facts(org_id, run_id).await;
        if has_retryable_target_intel_dns_outcome(&outcomes, target.as_deref()) {
            tracing::info!(
                target: "harness::duplicate_guard",
                tool = %tool_name,
                query = %query,
                dns_target = target.as_deref().unwrap_or("<organization>"),
                "not skipping duplicate recon call; DNS technique outcome remains retryable"
            );
            return None;
        }
    }
    let mut evidence_ids: Vec<i64> = relevant
        .iter()
        .flat_map(|row| row.evidence_ids.iter().copied())
        .collect();
    evidence_ids.sort_unstable();
    evidence_ids.dedup();
    tracing::info!(
        target: "harness::duplicate_guard",
        tool = %tool_name,
        query = %query,
        source_rows = relevant.len(),
        "skipping duplicate target_intel recon tool call; skippable source_query_log rows already exist"
    );
    Some(ToolExecutionResult {
        value: json!({
            "action": query,
            "skipped_duplicate": true,
            "reason": "the runtime completion marker exists and all generic/exact source_query_log rows for this run/action are terminal; not re-running providers",
            "source_query_rows": relevant.len(),
            "existing_evidence_ids": evidence_ids,
        }),
        success: true,
    })
}

fn source_query_belongs_to_action(row_query: &str, action: &str) -> bool {
    row_query == action
        || row_query
            .strip_prefix(action)
            .is_some_and(|suffix| suffix.starts_with(':') && suffix.len() > 1)
}

fn source_status_completion_marker_matches(
    source: &str,
    row_query: &str,
    row_target: &str,
    action: &str,
    requested_target: Option<&str>,
) -> bool {
    source == SOURCE_STATUS_COMPLETE_SOURCE
        && row_query == format!("{action}:{SOURCE_STATUS_COMPLETE_SUFFIX}")
        && requested_target.map_or(row_target.is_empty(), |target| row_target == target)
}

fn source_query_statuses_all_terminal<'a>(statuses: impl IntoIterator<Item = &'a str>) -> bool {
    let mut saw_status = false;
    for status in statuses {
        saw_status = true;
        if !is_skippable_source_query_status(status) {
            return false;
        }
    }
    saw_status
}

fn has_retryable_target_intel_dns_outcome(
    outcomes: &[golish_agent_kit::db_traits::TechniqueOutcomeFact],
    target: Option<&str>,
) -> bool {
    outcomes.iter().any(|row| {
        row.technique == "GOLISH-INTEL-DNS"
            && matches!(row.outcome.as_str(), "error" | "partial" | "running")
            && target.is_none_or(|target| normalize_source_query_target(&row.asset) == target)
    })
}

fn duplicate_guard_query(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        "recon_map_assets" => Some("map_assets"),
        "recon_lookup_whois" => Some("lookup_whois"),
        "recon_discover_subsidiaries" => Some("discover_subsidiaries"),
        _ => None,
    }
}

fn duplicate_guard_target(tool_name: &str, tool_args: &serde_json::Value) -> Option<String> {
    if tool_name != "recon_map_assets" {
        return None;
    }
    tool_args
        .get("domain")
        .and_then(|value| value.as_str())
        .map(normalize_source_query_target)
        .filter(|target| !target.is_empty())
}

fn is_skippable_source_query_status(status: &str) -> bool {
    // `error` proves that a source was attempted, but target_intel explicitly
    // keeps it retryable (`error_is_terminal=false`). Do not let the duplicate
    // guard turn a transient provider/RDAP failure into a false completion.
    matches!(status, "found" | "empty" | "checked_empty" | "blocked")
}

/// P2-d guardrail decision for the execution chokepoint.
///
/// Returns `Some(reason)` when a tool call must be blocked, else `None`.
/// Guardrails are scoped to harness stages (like the P0 evidence append): when
/// no stage is active (`harness_stage == None`) nothing is blocked, preserving
/// legacy non-harness behaviour. Inside a stage the most-restrictive guardrail
/// action wins and only a `Block` stops execution — `Audit` / `Sanitize` are
/// advisory and do not halt the call here.
fn guardrail_block_reason(
    harness_stage: Option<golish_agent_kit::harness::StageKind>,
    tool_name: &str,
    args: &serde_json::Value,
) -> Option<String> {
    harness_stage?;
    // A stage deliverable only validates and writes evidence/coverage. Its
    // claims necessarily repeat observed targets, including legitimate private
    // or loopback assets, but it never dereferences them. Applying the SSRF or
    // dangerous-shell string scanner to this control-plane payload blocks the
    // audit record rather than the network action it describes.
    if tool_name == "submit_stage_deliverable" {
        return None;
    }
    use golish_agent_kit::harness::guardrail::{
        default_guardrails, evaluate_guardrails, GuardrailAction,
    };
    match evaluate_guardrails(tool_name, args, &default_guardrails()) {
        GuardrailAction::Block(reason) => Some(reason),
        GuardrailAction::Allow | GuardrailAction::Audit(_) | GuardrailAction::Sanitize(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        duplicate_guard_query, generic_pentest_evidence_enabled, guardrail_block_reason,
        has_retryable_target_intel_dns_outcome, is_skippable_source_query_status,
        recon_source_query_rows, recon_source_query_rows_for_call,
        scoping_recon_requested_organization_id, should_refresh_target_intel_dns,
        source_query_belongs_to_action, source_query_statuses_all_terminal,
        source_status_completion_marker_matches, source_status_completion_rows,
        stage_evidence_operation_id, structured_storage_hook_payload,
    };
    use golish_agent_kit::harness::StageKind;
    use serde_json::json;

    fn in_stage() -> Option<StageKind> {
        Some(StageKind::ExternalAttackSurface)
    }

    #[test]
    fn stage_evidence_requires_the_durable_harness_operation() {
        let operation_id = uuid::Uuid::from_u128(1);

        assert_eq!(
            stage_evidence_operation_id(Some(operation_id)).expect("operation should be accepted"),
            operation_id,
            "final-seal evidence must join the durable operation, not the chat session row"
        );
        assert!(
            stage_evidence_operation_id(None).is_err(),
            "a staged writer must fail closed instead of falling back to task/session UUIDs"
        );
    }

    #[test]
    fn scoping_recon_evidence_requests_authorization_only_for_subsidiary_discovery() {
        let root_id = uuid::Uuid::from_u128(7);
        let args = json!({"organization_id": root_id});

        assert_eq!(
            scoping_recon_requested_organization_id(
                Some(StageKind::Scoping),
                "recon_discover_subsidiaries",
                &args,
            ),
            Some(root_id)
        );
        assert_eq!(
            scoping_recon_requested_organization_id(
                Some(StageKind::TargetIntel),
                "recon_discover_subsidiaries",
                &args,
            ),
            None,
            "Target Intel must continue using its frozen harness organization"
        );
        assert_eq!(
            scoping_recon_requested_organization_id(
                Some(StageKind::Scoping),
                "recon_map_assets",
                &args,
            ),
            None,
            "the Scoping exception must not broaden to other recon actions"
        );
        assert_eq!(
            scoping_recon_requested_organization_id(
                Some(StageKind::Scoping),
                "recon_discover_subsidiaries",
                &json!({"organization_id":"not-a-uuid"}),
            ),
            None
        );
    }

    #[test]
    fn ssrf_target_blocked_inside_stage() {
        let args = json!({ "url": "http://169.254.169.254/latest/meta-data/" });
        assert!(guardrail_block_reason(in_stage(), "http_probe", &args).is_some());
    }

    #[test]
    fn destructive_shell_blocked_inside_stage() {
        let args = json!({ "command": "rm -rf / --no-preserve-root" });
        assert!(guardrail_block_reason(in_stage(), "run_pty_cmd", &args).is_some());
    }

    #[test]
    fn benign_call_allowed_inside_stage() {
        let args = json!({ "command": "subfinder -d example.com -silent" });
        assert!(guardrail_block_reason(in_stage(), "run_pty_cmd", &args).is_none());
    }

    #[test]
    fn submit_deliverable_with_internal_target_observation_is_not_treated_as_ssrf() {
        let args = json!({
            "stage_id": "external_attack_surface",
            "claims": [{
                "kind": "http_service_observed",
                "subject": "127.0.0.1:55230",
                "summary": "http://127.0.0.1:55230 is live"
            }]
        });
        assert!(
            guardrail_block_reason(in_stage(), "submit_stage_deliverable", &args).is_none(),
            "submitting evidence is a DB/control-plane action, not an outbound request"
        );
    }

    #[test]
    fn no_stage_means_no_guardrail() {
        // Outside a harness stage the guardrail is inert even for dangerous args,
        // preserving legacy non-harness tool dispatch.
        let args = json!({ "url": "http://169.254.169.254/" });
        assert!(guardrail_block_reason(None, "http_probe", &args).is_none());
    }

    #[test]
    fn duplicate_guard_maps_recon_tools_to_source_queries() {
        assert_eq!(
            duplicate_guard_query("recon_map_assets"),
            Some("map_assets")
        );
        assert_eq!(
            duplicate_guard_query("recon_lookup_whois"),
            Some("lookup_whois")
        );
        assert_eq!(
            duplicate_guard_query("recon_discover_subsidiaries"),
            Some("discover_subsidiaries")
        );
        assert_eq!(duplicate_guard_query("subfinder"), None);
    }

    #[test]
    fn duplicate_guard_does_not_skip_retryable_source_errors() {
        for status in ["found", "empty", "checked_empty", "blocked"] {
            assert!(
                is_skippable_source_query_status(status),
                "{status} should be skippable"
            );
        }
        assert!(!is_skippable_source_query_status("error"));
        assert!(!is_skippable_source_query_status("running"));
    }

    #[test]
    fn duplicate_guard_action_membership_includes_exact_technique_rows() {
        assert!(source_query_belongs_to_action("map_assets", "map_assets"));
        assert!(source_query_belongs_to_action(
            "map_assets:GOLISH-INTEL-CT",
            "map_assets"
        ));
        assert!(!source_query_belongs_to_action(
            "map_assets_extra:GOLISH-INTEL-CT",
            "map_assets"
        ));
        assert!(!source_query_belongs_to_action(
            "lookup_whois:GOLISH-INTEL-WHOIS",
            "map_assets"
        ));
    }

    #[test]
    fn duplicate_guard_generic_found_does_not_hide_exact_retryable_error() {
        assert!(!source_query_statuses_all_terminal(["found", "error"]));
        assert!(!source_query_statuses_all_terminal(["found", "running"]));
        assert!(source_query_statuses_all_terminal([
            "found", "empty", "blocked"
        ]));
        assert!(!source_query_statuses_all_terminal([]));
    }

    #[test]
    fn duplicate_guard_requires_runtime_completion_marker_for_exact_group() {
        assert!(!source_status_completion_marker_matches(
            "0.zone",
            "map_assets",
            "",
            "map_assets",
            None,
        ));
        assert!(source_status_completion_marker_matches(
            "golish-runtime",
            "map_assets:source-status-complete",
            "",
            "map_assets",
            None,
        ));
        assert!(source_status_completion_marker_matches(
            "golish-runtime",
            "map_assets:source-status-complete",
            "moresec.cn",
            "map_assets",
            Some("moresec.cn"),
        ));
        assert!(!source_status_completion_marker_matches(
            "golish-runtime",
            "map_assets:source-status-complete",
            "",
            "map_assets",
            Some("moresec.cn"),
        ));
    }

    #[test]
    fn targeted_map_assets_rows_and_completion_marker_stay_target_bound() {
        let rows = recon_source_query_rows_for_call(
            "recon_map_assets",
            &json!({"domain": "MoreSec.CN."}),
            &json!({
                "action": "map_assets",
                "providerStatus": [{"providerId": "0.zone", "status": "completed"}],
                "techniqueStatus": [{
                    "source": "0.zone",
                    "technique": "GOLISH-INTEL-DNS",
                    "status": "found"
                }]
            }),
        );
        assert!(rows.iter().all(|row| row.target == "moresec.cn"));

        let markers = source_status_completion_rows("recon_map_assets", &rows);
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].source, "golish-runtime");
        assert_eq!(markers[0].query, "map_assets:source-status-complete");
        assert_eq!(markers[0].target, "moresec.cn");
    }

    #[test]
    fn duplicate_guard_does_not_hide_retryable_dns_outcome() {
        use golish_agent_kit::db_traits::TechniqueOutcomeFact;

        let rows = vec![TechniqueOutcomeFact::new(
            "MoreSec.CN.",
            "GOLISH-INTEL-DNS",
            "error",
            41,
            Some("resolver".to_string()),
        )];
        assert!(has_retryable_target_intel_dns_outcome(
            &rows,
            Some("moresec.cn")
        ));
        assert!(!has_retryable_target_intel_dns_outcome(
            &rows,
            Some("other.example")
        ));
        assert!(has_retryable_target_intel_dns_outcome(&rows, None));

        let terminal = vec![TechniqueOutcomeFact::new(
            "moresec.cn",
            "GOLISH-INTEL-DNS",
            "empty",
            42,
            Some("resolver".to_string()),
        )];
        assert!(!has_retryable_target_intel_dns_outcome(
            &terminal,
            Some("moresec.cn")
        ));
    }

    #[test]
    fn provider_status_rows_map_to_source_query_statuses() {
        let rows = recon_source_query_rows(
            "recon_map_assets",
            &json!({
                "action": "map_assets",
                "providerStatus": [
                    {"providerId": "0.zone", "status": "completed", "message": "ok"},
                    {"providerId": "quake", "status": "checked_empty", "message": "empty"},
                    {"providerId": "fofa", "status": "unavailable", "message": "no key"},
                    {"providerId": "hunter", "status": "failed", "message": "boom"}
                ]
            }),
        );
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].source, "0.zone");
        assert_eq!(rows[0].query, "map_assets");
        assert_eq!(rows[0].status, "found");
        assert_eq!(rows[1].status, "empty");
        assert_eq!(rows[2].status, "blocked");
        assert_eq!(rows[3].status, "error");
        assert!(rows.iter().all(|row| row.target.is_empty()));
    }

    #[test]
    fn failed_or_repair_blocked_map_assets_does_not_refresh_dns() {
        assert!(!should_refresh_target_intel_dns("recon_map_assets", false));
        assert!(should_refresh_target_intel_dns("recon_map_assets", true));
        assert!(!should_refresh_target_intel_dns("recon_lookup_whois", true));
    }

    #[test]
    fn technique_status_rows_preserve_exact_osint_empty_outcome() {
        let rows = recon_source_query_rows(
            "recon_map_assets",
            &json!({
                "action": "map_assets",
                "providerStatus": [{
                    "providerId": "enscan-go-enrichment",
                    "status": "checked_empty",
                    "message": "no candidates"
                }],
                "techniqueStatus": [{
                    "source": "enscan-go-enrichment",
                    "technique": "GOLISH-INTEL-OSINT",
                    "status": "empty",
                    "message": "OSINT provider ran and returned no records"
                }]
            }),
        );

        assert!(rows.iter().any(|row| {
            row.source == "enscan-go-enrichment"
                && row.technique == Some("GOLISH-INTEL-OSINT")
                && row.status == "empty"
                && row.target.is_empty()
        }));
        assert!(rows.iter().any(|row| row.technique.is_none()));
    }

    #[test]
    fn domain_expansion_provider_status_rows_keep_domain_target() {
        let rows = recon_source_query_rows(
            "recon_map_assets",
            &json!({
                "action": "map_assets",
                "providerStatus": [
                    {"providerId": "quake", "status": "completed", "message": "org survey"}
                ],
                "domainExpansions": [
                    {
                        "domain": "MoreSec.CN.",
                        "providerStatus": [
                            {"providerId": "0.zone", "status": "completed", "message": "domain survey"},
                            {"providerId": "fofa", "status": "unavailable", "message": "no key"}
                        ]
                    }
                ]
            }),
        );

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].source, "quake");
        assert_eq!(rows[0].target, "");
        assert_eq!(rows[1].source, "0.zone");
        assert_eq!(rows[1].target, "moresec.cn");
        assert_eq!(rows[1].status, "found");
        assert_eq!(rows[2].target, "moresec.cn");
        assert_eq!(rows[2].status, "blocked");
    }

    #[test]
    fn whois_result_maps_to_rdap_source_query() {
        let rows = recon_source_query_rows(
            "recon_lookup_whois",
            &json!({"action": "lookup_whois", "whois_landed": true}),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source, "rdap");
        assert_eq!(rows[0].query, "lookup_whois");
        assert_eq!(rows[0].technique, Some("GOLISH-INTEL-WHOIS"));
        assert_eq!(rows[0].status, "found");

        let empty = recon_source_query_rows("recon_lookup_whois", &json!({}));
        assert_eq!(empty[0].status, "empty");

        let failed = recon_source_query_rows(
            "recon_lookup_whois",
            &json!({"error": "RDAP transport failed"}),
        );
        assert_eq!(failed[0].status, "error");

        for (typed, expected) in [
            ("found", "found"),
            ("empty", "empty"),
            ("error", "error"),
            ("blocked", "blocked"),
        ] {
            let rows = recon_source_query_rows(
                "recon_lookup_whois",
                &json!({"action": "lookup_whois", "whois_status": typed}),
            );
            assert_eq!(rows[0].status, expected, "typed WHOIS status {typed}");
        }
    }

    #[test]
    fn top_level_recon_provider_error_still_produces_source_attempt_row() {
        let rows = recon_source_query_rows(
            "recon_map_assets",
            &json!({"error": "provider runtime failed"}),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source, "recon_map_assets");
        assert_eq!(rows[0].query, "recon_map_assets");
        assert_eq!(rows[0].status, "error");
        assert!(rows[0].technique.is_none());
    }

    #[test]
    fn pentest_run_result_feeds_structured_storage_hook() {
        let payload = structured_storage_hook_payload(
            "pentest_run",
            &json!({"tool_name": "httpx", "args": "-u https://example.com -sc -title"}),
            &json!({
                "command": "httpx -u https://example.com -sc -title",
                "stdout": "https://example.com [200] [Example Domain]",
                "stderr": "",
                "exit_code": 0
            }),
            true,
        )
        .expect("pentest_run should produce structured-storage payload");

        assert_eq!(payload.command, "httpx -u https://example.com -sc -title");
        assert_eq!(payload.stdout, "https://example.com [200] [Example Domain]");
    }

    #[test]
    fn eas_wrapper_result_feeds_structured_storage_hook() {
        let payload = structured_storage_hook_payload(
            "eas_fingerprint_services",
            &json!({"targets": ["192.0.2.10"], "ports": [80]}),
            &json!({
                "wrapped_tool_name": "nmap",
                "wrapped_args": "-sV -Pn -iL {input_file} -p 80 -T3",
                "command": "nmap -sV -Pn -iL /tmp/targets -p 80 -T3",
                "stdout": "80/tcp open http nginx",
                "stderr": "",
                "exit_code": 0
            }),
            true,
        )
        .expect("EAS wrapper should produce structured-storage payload");

        assert_eq!(payload.command, "nmap -sV -Pn -iL /tmp/targets -p 80 -T3");
        assert_eq!(payload.stdout, "80/tcp open http nginx");
    }

    #[test]
    fn self_landed_eas_wrapper_disables_generic_storage_and_evidence() {
        let result = json!({
            "wrapped_tool_name": "nmap",
            "wrapped_args": "-sV -Pn -iL {input_file} -p 80 -T3",
            "stdout": "80/tcp open http nginx",
            "exit_code": 0,
            "structured_storage_disabled": true,
            "generic_evidence_disabled": true
        });
        assert!(structured_storage_hook_payload(
            "eas_fingerprint_services",
            &json!({"targets": ["192.0.2.10"], "ports": [80]}),
            &result,
            true,
        )
        .is_none());
        assert!(!generic_pentest_evidence_enabled(&result));
    }

    #[test]
    fn enum_wrapper_seed_only_result_skips_structured_storage_hook() {
        let payload = structured_storage_hook_payload(
            "enum_crawl_same_origin_urls",
            &json!({"target_urls": ["https://app.example.com/"]}),
            &json!({
                "wrapped_tool_name": "katana",
                "wrapped_args": "-list {input_file} -jc -silent -d 2",
                "command": "katana -list /tmp/roots.txt -jc -silent -d 2",
                "stdout": "https://app.example.com/api/v1/users",
                "stderr": "",
                "exit_code": 0,
                "structured_storage_disabled": true,
                "generic_evidence_disabled": true
            }),
            true,
        );
        assert!(payload.is_none());
        assert!(!generic_pentest_evidence_enabled(&json!({
            "generic_evidence_disabled": true
        })));
    }
}
