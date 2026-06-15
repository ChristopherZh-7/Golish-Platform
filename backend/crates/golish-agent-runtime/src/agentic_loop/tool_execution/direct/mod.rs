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

    if matches!(
        tool_name,
        "log_operation"
            | "discover_apis"
            | "save_js_analysis"
            | "fingerprint_target"
            | "log_scan_result"
            | "query_target_data"
            | "list_in_scope_targets"
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
                ctx.harness_org_id,
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
                            // PR2 (coverage 投影) · deterministically derive
                            // (technique, asset, outcome) from the shell command;
                            // unmapped commands stay None (never project).
                            let facts = golish_agent_kit::harness::evidence_facts::passive_intel_facts_from_command(ev_subject)
                                .map(|(technique, asset)| {
                                    let outcome = golish_agent_kit::harness::evidence_facts::passive_intel_outcome(technique, &ev_stdout);
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
                                    &ev_stdout,
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
            if effective_tool_name == "pentest_run" && is_success && ctx.harness_stage.is_some() {
                if let Some(tracker) = ctx.events.db_tracker {
                    if let Some(repo) = tracker.repo() {
                        let op_id = tracker.task_id().unwrap_or_else(|| tracker.session_uuid());
                        let ev_stdout = v
                            .get("stdout")
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_string();
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
                        // command line; same deterministic facts derivation.
                        let facts = golish_agent_kit::harness::evidence_facts::passive_intel_facts_from_command(&ev_subject)
                            .map(|(technique, asset)| {
                                let outcome = golish_agent_kit::harness::evidence_facts::passive_intel_outcome(technique, &ev_stdout);
                                (technique, asset, outcome)
                            });
                        match repo
                            .evidence_append(
                                op_id,
                                None,
                                ctx.events.session_id,
                                tracker.project_path(),
                                pt_tool,
                                pt_tool,
                                &ev_subject,
                                &ev_stdout,
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
                                    "pentest_run evidence appended; surfacing id to agent"
                                );
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

            // Passive recon agent tools (recon_discover_subsidiaries /
            // recon_enrich_assets) return a JSON summary (not stdout). Book it to
            // the ledger so target_intel coverage cells can cite a REAL evidence id
            // (otherwise the passive-intel deliverable is "fabricated" and the gate
            // loops). Mirrors the pentest_run block above.
            if matches!(
                effective_tool_name,
                "recon_discover_subsidiaries" | "recon_enrich_assets"
            ) && is_success
                && ctx.harness_stage.is_some()
            {
                if let Some(tracker) = ctx.events.db_tracker {
                    if let Some(repo) = tracker.repo() {
                        let op_id = tracker.task_id().unwrap_or_else(|| tracker.session_uuid());
                        let ev_subject = v
                            .get("company")
                            .and_then(|c| c.as_str())
                            .filter(|c| !c.is_empty())
                            .unwrap_or(effective_tool_name);
                        let ev_raw = serde_json::to_string(&v).unwrap_or_default();
                        // PR2 · enrich's subject is a COMPANY name, not an in-scope
                        // asset (domain/IP) — no deterministic asset, so no facts
                        // (设计 §4 约束3: 歧义即不派生). Per-asset enrich rows are
                        // the A1 follow-up.
                        // Phase 2 (2026-06-12-redteam-phase2) · subsidiary discovery
                        // DOES derive an org-level GOLISH-INTEL-SUBSIDIARY fact from
                        // its structured summary (promoted_children / status): the
                        // Empty outcome is what lets "ran → 0 qualifying child" be
                        // checked_empty instead of not_attempted (I8). The gate hook
                        // re-projects the company-name asset onto in-scope assets.
                        let facts = (effective_tool_name == "recon_discover_subsidiaries")
                            .then(|| {
                                golish_agent_kit::harness::evidence_facts::subsidiary_discovery_facts(v)
                            })
                            .flatten();
                        match repo
                            .evidence_append(
                                op_id,
                                None,
                                ctx.events.session_id,
                                tracker.project_path(),
                                effective_tool_name,
                                effective_tool_name,
                                ev_subject,
                                &ev_raw,
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
                                    "recon passive evidence appended; surfacing id to agent"
                                );
                            }
                            Err(e) => tracing::warn!(
                                target: "harness::evidence",
                                error = %e,
                                "recon passive evidence append failed (continuing)"
                            ),
                        }
                    }
                }
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
    use super::guardrail_block_reason;
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
}
