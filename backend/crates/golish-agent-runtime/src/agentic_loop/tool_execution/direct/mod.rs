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

// `pub(crate)` so `execution_mode::selection_apply` can pull the tool definition
// (co-located with its handler) when exposing `stage_run` to the primary agent.
pub(crate) mod stage_run_call;
use self::stage_run_call::execute_stage_run;

#[derive(Debug, Clone, PartialEq, Eq)]
struct StructuredStorageHookPayload {
    command: String,
    stdout: String,
}

fn structured_storage_hook_payload(
    tool_name: &str,
    tool_args: &serde_json::Value,
    result: &serde_json::Value,
    success: bool,
) -> Option<StructuredStorageHookPayload> {
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
    } else if tool_name == "pentest_run" {
        result
            .get("command")
            .and_then(|c| c.as_str())
            .map(str::to_string)
            .or_else(|| {
                let tool = tool_args.get("tool_name").and_then(|v| v.as_str())?;
                let args = tool_args.get("args").and_then(|v| v.as_str()).unwrap_or("");
                Some(format!("{tool} {args}").trim().to_string())
            })
            .unwrap_or_default()
    } else {
        String::new()
    };

    if command.trim().is_empty() {
        return None;
    }

    Some(StructuredStorageHookPayload { command, stdout })
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

async fn record_recon_passive_evidence(
    tracker: Option<&golish_agent_kit::db_tracking::DbTracker>,
    session_id: Option<&str>,
    harness_stage: Option<golish_agent_kit::harness::StageKind>,
    harness_org_id: Option<uuid::Uuid>,
    tool_name: &str,
    result: &serde_json::Value,
    success: bool,
) -> Option<i64> {
    if !matches!(
        tool_name,
        "recon_discover_subsidiaries" | "recon_map_assets" | "recon_lookup_whois"
    ) || !success
        || harness_stage.is_none()
    {
        return None;
    }

    let tracker = tracker?;
    let repo = tracker.repo()?;
    let op_id = tracker.task_id().unwrap_or_else(|| tracker.session_uuid());
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
    let facts = (tool_name == "recon_discover_subsidiaries")
        .then(|| golish_agent_kit::harness::evidence_facts::subsidiary_discovery_facts(result))
        .flatten();

    match repo
        .evidence_append(
            op_id,
            None,
            session_id,
            tracker.project_path(),
            tool_name,
            tool_name,
            ev_subject,
            &ev_raw,
            facts.as_ref().map(|(t, a, o)| (*t, a.as_str(), *o)),
        )
        .await
    {
        Ok(id) => {
            tracing::info!(
                target: "harness::evidence",
                tool = %tool_name,
                evidence_id = id,
                "recon passive evidence appended; surfacing id to agent"
            );

            if let (Some(org_id), Some(rid), Some((tech, asset, outcome))) = (
                harness_org_id,
                session_id,
                facts.as_ref().map(|(t, a, o)| (*t, a.as_str(), *o)),
            ) {
                if let Err(e) = repo
                    .upsert_technique_outcome(
                        org_id,
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
                    tracing::warn!(
                        target: "harness::evidence",
                        error = %e,
                        "technique_outcomes upsert failed (continuing)"
                    );
                }
            }

            if tool_name == "recon_discover_subsidiaries" {
                if let (Some(org_id), Some(rid)) = (harness_org_id, session_id) {
                    for lead in
                        golish_agent_kit::harness::evidence_facts::expansion_leads_from_subsidiary_discovery(result)
                    {
                        if let Err(e) = repo
                            .enqueue_expansion_lead(
                                org_id,
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

            if let (Some(org_id), Some(rid)) = (harness_org_id, session_id) {
                for row in recon_source_query_rows(tool_name, result) {
                    if let Err(e) = repo
                        .upsert_source_query(
                            org_id,
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
                        tracing::warn!(
                            target: "harness::evidence",
                            error = %e,
                            source = %row.source,
                            query = %row.query,
                            "source_query_log provider upsert failed (continuing)"
                        );
                    }
                }
            }

            Some(id)
        }
        Err(e) => {
            tracing::warn!(
                target: "harness::evidence",
                error = %e,
                "recon passive evidence append failed (continuing)"
            );
            None
        }
    }
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
        if let Some((value, success)) =
            golish_agent_kit::tool_executors::execute_security_analysis_tool(
                tool_name,
                tool_args,
                ctx.events.db_tracker,
                Some(project_path_str.as_str()),
                ctx.events.session_id,
                ctx.harness_org_id,
                ctx.harness_stage,
                ctx.harness_operation_id,
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

    if let Some(result) = duplicate_source_query_guard(effective_tool_name, ctx).await {
        return Ok(result);
    }

    let registry = ctx.tool_registry.read().await;
    let result = registry
        .execute_tool(effective_tool_name, tool_args.clone())
        .await;

    match &result {
        Ok(v) => {
            let is_success = is_tool_result_success(v);
            // P1 · the evidence-ledger id appended for this tool run (if any), so
            // we can surface it to the agent below — letting it cite a REAL id in
            // its StageDeliverable instead of fabricating one (which the gate
            // would then BLOCK).
            let mut appended_evidence_id: Option<i64> = None;

            if let Some(payload) =
                structured_storage_hook_payload(effective_tool_name, tool_args, v, is_success)
            {
                if let Some(hook) = &ctx.post_shell_hook {
                    let ws = ctx.workspace.read().await;
                    let pp = ws.to_string_lossy().to_string();
                    drop(ws);
                    let hook = Arc::clone(hook);
                    let org_id = ctx.harness_org_id;
                    tokio::spawn(async move {
                        hook(payload.command, payload.stdout, Some(pp), org_id).await;
                    });
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
                            // Operation grouping key for the hash chain: the
                            // task_id when a task scope is set, else the session
                            // uuid. (Per-task scoping via `set_task_context` has no
                            // callers yet; session keeps the chain working today
                            // and auto-upgrades to task_id once that is wired.)
                            let op_id = tracker.task_id().unwrap_or_else(|| tracker.session_uuid());
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
                            match repo
                                .evidence_append(
                                    op_id,
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
                            {
                                Ok(id) => {
                                    appended_evidence_id = Some(id);
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
            if effective_tool_name == "pentest_run" && ctx.harness_stage.is_some() {
                if let Some(tracker) = ctx.events.db_tracker {
                    if let Some(repo) = tracker.repo() {
                        let op_id = tracker.task_id().unwrap_or_else(|| tracker.session_uuid());
                        let ev_output = combined_stdout_stderr(v);
                        let pt_tool = tool_args
                            .get("tool_name")
                            .and_then(|s| s.as_str())
                            .filter(|s| !s.is_empty())
                            .unwrap_or("pentest_run");
                        let pt_args = tool_args.get("args").and_then(|s| s.as_str()).unwrap_or("");
                        let ev_subject = if pt_args.is_empty() {
                            pt_tool.to_string()
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
                            match repo
                                .evidence_append(
                                    op_id,
                                    None,
                                    ctx.events.session_id,
                                    tracker.project_path(),
                                    pt_tool,
                                    pt_tool,
                                    &ev_subject,
                                    &ev_body,
                                    facts.as_ref().map(|(t, a, o)| (*t, a.as_str(), *o)),
                                )
                                .await
                            {
                                Ok(id) => {
                                    appended_evidence_id = Some(id);
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
                                                Some(pt_tool),
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
                                                pt_tool,
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

            if let Some(id) = record_recon_passive_evidence(
                ctx.events.db_tracker,
                ctx.events.session_id,
                ctx.harness_stage,
                ctx.harness_org_id,
                effective_tool_name,
                v,
                is_success,
            )
            .await
            {
                appended_evidence_id = Some(id);
            }

            // P1 · surface the appended evidence id so the agent cites a REAL
            // ledger id in its StageDeliverable. Additive `_evidence_id` field;
            // absent when nothing was appended (non-shell tool / no stage).
            let mut value = v.clone();
            if let Some(id) = appended_evidence_id {
                if let Some(obj) = value.as_object_mut() {
                    obj.insert("_evidence_id".to_string(), json!(id));
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
                success: is_success,
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
                        query: query.clone(),
                        target: String::new(),
                        technique: None,
                        status: provider_status_for_source_query(raw_status),
                    })
                })
                .collect()
        }
        "recon_lookup_whois" => {
            let query = result
                .get("action")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(tool_name)
                .to_string();
            let landed = result
                .get("whois_landed")
                .or_else(|| result.get("whoisLanded"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            vec![ReconSourceQueryRow {
                source: "rdap".to_string(),
                query,
                target: String::new(),
                technique: Some("GOLISH-INTEL-WHOIS"),
                status: if landed { "found" } else { "empty" },
            }]
        }
        _ => Vec::new(),
    }
}

fn provider_status_for_source_query(status: &str) -> &'static str {
    match status {
        "completed" | "Completed" => "found",
        "checked_empty" | "CheckedEmpty" => "empty",
        "unavailable" | "Unavailable" => "blocked",
        "failed" | "Failed" => "error",
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
    }
}

async fn duplicate_source_query_guard(
    tool_name: &str,
    ctx: &AgenticLoopContext<'_>,
) -> Option<ToolExecutionResult> {
    let query = duplicate_guard_query(tool_name)?;
    if ctx.harness_stage != Some(golish_agent_kit::harness::StageKind::TargetIntel) {
        return None;
    }
    let org_id = ctx.harness_org_id?;
    let run_id = ctx.events.session_id?;
    let repo = ctx.events.db_tracker?.repo()?;
    let rows = repo.source_query_facts(org_id, run_id).await;
    let matching: Vec<_> = rows
        .iter()
        .filter(|row| row.query == query && is_terminal_source_query_status(&row.status))
        .collect();
    if matching.is_empty() {
        return None;
    }
    let mut evidence_ids: Vec<i64> = matching
        .iter()
        .flat_map(|row| row.evidence_ids.iter().copied())
        .collect();
    evidence_ids.sort_unstable();
    evidence_ids.dedup();
    tracing::info!(
        target: "harness::duplicate_guard",
        tool = %tool_name,
        query = %query,
        source_rows = matching.len(),
        "skipping duplicate target_intel recon tool call; terminal source_query_log rows already exist"
    );
    Some(ToolExecutionResult {
        value: json!({
            "action": query,
            "skipped_duplicate": true,
            "reason": "terminal source_query_log rows already exist for this run/action; not re-running providers",
            "source_query_rows": matching.len(),
            "existing_evidence_ids": evidence_ids,
        }),
        success: true,
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

fn is_terminal_source_query_status(status: &str) -> bool {
    matches!(
        status,
        "found" | "empty" | "checked_empty" | "error" | "blocked"
    )
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
        duplicate_guard_query, guardrail_block_reason, is_terminal_source_query_status,
        recon_source_query_rows, structured_storage_hook_payload,
    };
    use golish_agent_kit::harness::StageKind;
    use serde_json::json;

    fn in_stage() -> Option<StageKind> {
        Some(StageKind::ExternalAttackSurface)
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
    fn duplicate_guard_only_treats_terminal_source_statuses_as_skippable() {
        for status in ["found", "empty", "checked_empty", "error", "blocked"] {
            assert!(
                is_terminal_source_query_status(status),
                "{status} should be terminal"
            );
        }
        assert!(!is_terminal_source_query_status("running"));
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
}
