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
use crate::executor_types::{
    cancellation_requested, wait_for_cancelled, CoverageGapAction, SubAgentExecutorContext,
    SubAgentToolObservation, SubmitRepairKind, SubmitRepairMode, ToolProvider, BARRIER_TOOL_NAME,
};
use crate::transcript::SubAgentTranscriptWriter;
use golish_core::events::{AiEvent, ToolSource};
use golish_core::utils::{is_tool_result_success, truncate_str};

const HARD_SUPERVISOR_MARKER: &str = "--- EXECUTION SUPERVISOR (HARD) ---";

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
    /// A deterministic repair-only lock to apply after certain
    /// `submit_stage_deliverable needs_fix` responses. The next turn may repair
    /// the submission, query existing state, or wait for background jobs, but it
    /// must not restart broad scanning just because the prior submit failed.
    pub submit_repair_update: Option<SubmitRepairModeUpdate>,
    pub cancelled: bool,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SubmitRepairModeUpdate {
    Set(SubmitRepairMode),
    Clear,
}

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

    let command = if (tool_name == "run_pty_cmd" || tool_name == "run_command") && success {
        result
            .get("command")
            .and_then(|c| c.as_str())
            .map(str::to_string)
            .or_else(|| {
                tool_args
                    .get("command")
                    .and_then(|c| c.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_default()
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

fn collect_json_array_strings(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .or_else(|| v.as_i64().map(|n| n.to_string()))
                        .or_else(|| v.as_u64().map(|n| n.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn missing_min_invocation_tools(reasons: &[String]) -> Vec<String> {
    const PREFIX: &str = "min tool invocations not satisfied for '";
    let mut out = Vec::new();
    for reason in reasons {
        let mut rest = reason.as_str();
        while let Some(idx) = rest.find(PREFIX) {
            let after = &rest[idx + PREFIX.len()..];
            let Some(end) = after.find('\'') else {
                break;
            };
            let tool = after[..end].trim();
            if !tool.is_empty() && !out.iter().any(|seen| seen == tool) {
                out.push(tool.to_string());
            }
            rest = &after[end + 1..];
        }
    }
    out
}

fn model_visible_tool_result(tool_name: &str, value: &serde_json::Value) -> serde_json::Value {
    match tool_name {
        "route_probe_paths" => compact_route_probe_result(value),
        "list_enumeration_web_roots" => compact_enumeration_web_roots_result(value),
        "browser_collect_js_api" => compact_browser_collect_result(value),
        "js_extract_apis" => compact_js_extract_result(value),
        _ => compact_large_json_result(value),
    }
}

fn compact_route_probe_result(value: &serde_json::Value) -> serde_json::Value {
    let matches = value
        .get("matches")
        .and_then(|v| v.as_array())
        .map(Vec::len)
        .unwrap_or_default();
    let rejected = value
        .get("rejected_candidates")
        .and_then(|v| v.as_array())
        .map(Vec::len)
        .unwrap_or_default();
    let errors = value
        .get("errors")
        .and_then(|v| v.as_array())
        .map(Vec::len)
        .unwrap_or_default();
    let prefixes = value
        .get("prefixes_tested")
        .and_then(|v| v.as_array())
        .map(Vec::len)
        .unwrap_or_default();
    let outcome = value
        .get("outcome")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let queue_completed = value
        .get("queue_completed")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let next_action = if outcome == "error" {
        "Refresh stage_worklist/check_stage_asset_coverage, then retry only this failed root or mark blocked with the concrete error evidence."
    } else if queue_completed {
        "Refresh stage_worklist/check_stage_asset_coverage; do not rerun this root unless coverage still reports DIR pending/error."
    } else {
        "Queue did not complete; refresh coverage, then continue this root only if DIR remains pending/error."
    };

    serde_json::json!({
        "success": value.get("success").cloned().unwrap_or(serde_json::Value::Null),
        "base_url": value.get("base_url").cloned().unwrap_or(serde_json::Value::Null),
        "requested_base_url": value.get("requested_base_url").cloned().unwrap_or(serde_json::Value::Null),
        "outcome": outcome,
        "outcome_persisted": value.get("outcome_persisted").cloned().unwrap_or(serde_json::Value::Null),
        "status": value.get("status").cloned().unwrap_or(serde_json::Value::Null),
        "timed_out": value.get("timed_out").cloned().unwrap_or(serde_json::Value::Null),
        "queue_completed": queue_completed,
        "queue_remaining": value.get("queue_remaining").cloned().unwrap_or(serde_json::Value::Null),
        "requests_sent": value.get("requests_sent").cloned().unwrap_or(serde_json::Value::Null),
        "candidate_requests_sent": value.get("candidate_requests_sent").cloned().unwrap_or(serde_json::Value::Null),
        "baseline_requests_sent": value.get("baseline_requests_sent").cloned().unwrap_or(serde_json::Value::Null),
        "persisted_directory_entries": value.get("persisted_directory_entries").cloned().unwrap_or(serde_json::Value::Null),
        "matches_count": matches,
        "matches_sample": sample_array(value.get("matches"), 5),
        "rejected_candidates_count": rejected,
        "rejected_candidates_sample": sample_array(value.get("rejected_candidates"), 3),
        "errors_count": errors,
        "errors_top": error_counts(value.get("errors"), 5),
        "errors_sample": sample_array(value.get("errors"), 5),
        "prefixes_tested_count": prefixes,
        "prefixes_tested_sample": sample_array(value.get("prefixes_tested"), 10),
        "wordlist": value.get("wordlist").cloned().unwrap_or(serde_json::Value::Null),
        "seed_paths": value.get("seed_paths").cloned().unwrap_or(serde_json::Value::Null),
        "run_id": value.get("run_id").cloned().unwrap_or(serde_json::Value::Null),
        "dry_run": value.get("dry_run").cloned().unwrap_or(serde_json::Value::Null),
        "next_action": next_action,
        "model_visible_compacted": true,
        "raw_result_retained_in_transcript": true,
        "omitted_large_fields": ["matches", "rejected_candidates", "errors", "prefixes_tested"],
    })
}

fn compact_enumeration_web_roots_result(value: &serde_json::Value) -> serde_json::Value {
    let roots = value
        .get("web_roots")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut pending_roots = Vec::new();
    let mut terminal_roots = Vec::new();
    for root in &roots {
        let pending = root
            .get("pending_techniques")
            .and_then(|v| v.as_array())
            .map(|arr| !arr.is_empty())
            .unwrap_or(false);
        let summary = serde_json::json!({
            "target_id": root.get("target_id").cloned().unwrap_or(serde_json::Value::Null),
            "root_url": root.get("root_url").cloned().unwrap_or(serde_json::Value::Null),
            "target_type": root.get("target_type").cloned().unwrap_or(serde_json::Value::Null),
            "pending_techniques": root.get("pending_techniques").cloned().unwrap_or_else(|| serde_json::json!([])),
            "terminal_techniques": root.get("terminal_techniques").cloned().unwrap_or_else(|| serde_json::json!([])),
            "suggested_tools": root.get("suggested_tools").cloned().unwrap_or_else(|| serde_json::json!([])),
            "next_steps": root.get("next_steps").cloned().unwrap_or_else(|| serde_json::json!([])),
        });
        if pending && pending_roots.len() < 20 {
            pending_roots.push(summary);
        } else if !pending && terminal_roots.len() < 5 {
            terminal_roots.push(summary);
        }
    }

    serde_json::json!({
        "stage": value.get("stage").cloned().unwrap_or(serde_json::Value::Null),
        "organization_id": value.get("organization_id").cloned().unwrap_or(serde_json::Value::Null),
        "session_id": value.get("session_id").cloned().unwrap_or(serde_json::Value::Null),
        "count": value.get("count").cloned().unwrap_or(serde_json::Value::Null),
        "total": value.get("total").cloned().unwrap_or(serde_json::Value::Null),
        "truncated": value.get("truncated").cloned().unwrap_or(serde_json::Value::Null),
        "pending_roots_sample": pending_roots,
        "terminal_roots_sample": terminal_roots,
        "omitted_root_count": roots.len().saturating_sub(25),
        "worklist_semantics": value.get("worklist_semantics").cloned().unwrap_or(serde_json::Value::Null),
        "execution_order": value.get("execution_order").cloned().unwrap_or(serde_json::Value::Null),
        "tool_boundary": value.get("tool_boundary").cloned().unwrap_or(serde_json::Value::Null),
        "next_action": "Process pending_roots_sample in batches, then call stage_worklist_next or check_stage_asset_coverage before submit.",
        "model_visible_compacted": true,
        "raw_result_retained_in_transcript": true,
        "omitted_large_fields": ["web_roots.coverage"],
    })
}

fn compact_browser_collect_result(value: &serde_json::Value) -> serde_json::Value {
    let api_requests = value
        .get("api_requests")
        .or_else(|| value.get("requests"))
        .or_else(|| value.get("endpoints"));
    let scripts = value.get("scripts").or_else(|| value.get("script_urls"));
    serde_json::json!({
        "success": value.get("success").cloned().unwrap_or(serde_json::Value::Null),
        "target_id": value.get("target_id").cloned().unwrap_or(serde_json::Value::Null),
        "url": value.get("url").or_else(|| value.get("target_url")).cloned().unwrap_or(serde_json::Value::Null),
        "summary": value.get("summary").cloned().unwrap_or(serde_json::Value::Null),
        "api_requests_count": api_requests.and_then(|v| v.as_array()).map(Vec::len).unwrap_or_default(),
        "api_requests_sample": sample_array(api_requests, 10),
        "scripts_count": scripts.and_then(|v| v.as_array()).map(Vec::len).unwrap_or_default(),
        "scripts_sample": sample_array(scripts, 10),
        "persisted": value.get("persisted").cloned().unwrap_or(serde_json::Value::Null),
        "skipped": value.get("skipped").cloned().unwrap_or(serde_json::Value::Null),
        "ai_recipe_rounds": value.get("ai_recipe_rounds").cloned().unwrap_or(serde_json::Value::Null),
        "ai_recipe_rationale": value.get("ai_recipe_rationale").cloned().unwrap_or(serde_json::Value::Null),
        "next_action": "Run js_extract_apis on collected JS if JS/API coverage remains pending, then refresh stage_worklist/check_stage_asset_coverage.",
        "model_visible_compacted": true,
        "raw_result_retained_in_transcript": true,
    })
}

fn compact_js_extract_result(value: &serde_json::Value) -> serde_json::Value {
    let endpoints = value
        .get("endpoints")
        .or_else(|| value.get("endpoints_found"))
        .or_else(|| value.pointer("/summary/endpoints"));
    let params = value.get("params").or_else(|| value.get("param_hints"));
    serde_json::json!({
        "success": value.get("success").cloned().unwrap_or(serde_json::Value::Null),
        "target_id": value.get("target_id").cloned().unwrap_or(serde_json::Value::Null),
        "url": value.get("url").or_else(|| value.get("target_url")).cloned().unwrap_or(serde_json::Value::Null),
        "summary": value.get("summary").cloned().unwrap_or(serde_json::Value::Null),
        "endpoints_count": endpoints.and_then(|v| v.as_array()).map(Vec::len).unwrap_or_default(),
        "endpoints_sample": sample_array(endpoints, 12),
        "params_count": params.and_then(|v| v.as_array()).map(Vec::len).unwrap_or_default(),
        "params_sample": sample_array(params, 12),
        "persisted_api_endpoints": value.get("persisted_api_endpoints").cloned().unwrap_or(serde_json::Value::Null),
        "ai_used": value.pointer("/summary/ai_used").or_else(|| value.get("ai_used")).cloned().unwrap_or(serde_json::Value::Null),
        "ai_endpoints_added": value.pointer("/summary/ai_endpoints_added").or_else(|| value.get("ai_endpoints_added")).cloned().unwrap_or(serde_json::Value::Null),
        "next_action": "Refresh stage_worklist/check_stage_asset_coverage; only retry this root if JS/API/PARAM remains pending or errored.",
        "model_visible_compacted": true,
        "raw_result_retained_in_transcript": true,
    })
}

fn compact_large_json_result(value: &serde_json::Value) -> serde_json::Value {
    let raw = serde_json::to_string(value).unwrap_or_default();
    if raw.chars().count() <= 24_000 {
        return value.clone();
    }
    let mut compacted = summarize_json_for_model(value, 3);
    if let Some(obj) = compacted.as_object_mut() {
        obj.insert(
            "model_visible_compacted".to_string(),
            serde_json::json!(true),
        );
        obj.insert(
            "raw_result_retained_in_transcript".to_string(),
            serde_json::json!(true),
        );
    }
    compacted
}

fn summarize_json_for_model(value: &serde_json::Value, depth: usize) -> serde_json::Value {
    if depth == 0 {
        return match value {
            serde_json::Value::Array(arr) => serde_json::json!({
                "omitted_array_items": arr.len()
            }),
            serde_json::Value::Object(obj) => serde_json::json!({
                "omitted_object_keys": obj.len()
            }),
            serde_json::Value::String(s) if s.chars().count() > 500 => {
                serde_json::json!(truncate_str(s, 500))
            }
            _ => value.clone(),
        };
    }

    match value {
        serde_json::Value::Array(arr) => {
            let sample: Vec<_> = arr
                .iter()
                .take(5)
                .map(|item| summarize_json_for_model(item, depth - 1))
                .collect();
            serde_json::json!({
                "count": arr.len(),
                "sample": sample,
                "omitted_count": arr.len().saturating_sub(5),
            })
        }
        serde_json::Value::Object(obj) => {
            let mut out = serde_json::Map::new();
            for (key, item) in obj.iter().take(16) {
                out.insert(key.clone(), summarize_json_for_model(item, depth - 1));
            }
            if obj.len() > 16 {
                out.insert(
                    "omitted_object_keys".to_string(),
                    serde_json::json!(obj.len() - 16),
                );
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::String(s) if s.chars().count() > 500 => {
            serde_json::json!(truncate_str(s, 500))
        }
        _ => value.clone(),
    }
}

fn sample_array(value: Option<&serde_json::Value>, limit: usize) -> serde_json::Value {
    let Some(arr) = value.and_then(|v| v.as_array()) else {
        return serde_json::json!([]);
    };
    serde_json::Value::Array(
        arr.iter()
            .take(limit)
            .map(|item| summarize_json_for_model(item, 2))
            .collect(),
    )
}

fn error_counts(value: Option<&serde_json::Value>, limit: usize) -> serde_json::Value {
    let Some(arr) = value.and_then(|v| v.as_array()) else {
        return serde_json::json!([]);
    };
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for item in arr {
        let key = item
            .get("error")
            .and_then(|v| v.as_str())
            .map(|s| truncate_str(s, 180).to_string())
            .unwrap_or_else(|| "unknown".to_string());
        *counts.entry(key).or_default() += 1;
    }
    let mut entries: Vec<_> = counts.into_iter().collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    serde_json::Value::Array(
        entries
            .into_iter()
            .take(limit)
            .map(|(error, count)| serde_json::json!({ "error": error, "count": count }))
            .collect(),
    )
}

fn needs_fix_evidence_ref_problem(reasons: &[String]) -> bool {
    let reason_lc = reasons.join(" | ").to_ascii_lowercase();
    reason_lc.contains("evidence_ref")
        || reason_lc.contains("evidence id")
        || reason_lc.contains("evidence_ids")
        || reason_lc.contains("fabricated")
        || reason_lc.contains("fakepattern")
        || reason_lc.contains("real evidence")
}

fn needs_fix_coverage_gap(reasons: &[String]) -> bool {
    let reason_lc = reasons.join(" | ").to_ascii_lowercase();
    reason_lc.contains("coverage")
        || reason_lc.contains("not_attempted")
        || reason_lc.contains("never attempted")
        || reason_lc.contains("never reached a terminal state")
        || reason_lc.contains("missing terminal")
        || reason_lc.contains("external attack surface incomplete")
        || reason_lc.contains("liveness")
        || reason_lc.contains("service-fingerprint")
        || reason_lc.contains("service fingerprint")
}

pub fn submit_coverage_gap_repair_mode_from_reasons(
    reasons: &[String],
) -> Option<SubmitRepairMode> {
    submit_coverage_gap_repair_mode(reasons, Vec::new())
}

fn submit_coverage_gap_repair_mode(
    reasons: &[String],
    coverage_gap_actions: Vec<CoverageGapAction>,
) -> Option<SubmitRepairMode> {
    if !needs_fix_coverage_gap(reasons) && coverage_gap_actions.is_empty() {
        return None;
    }
    Some(SubmitRepairMode {
        kind: SubmitRepairKind::CoverageGap,
        reason: if reasons.is_empty() {
            "(no reasons returned)".to_string()
        } else {
            reasons.join(" | ")
        },
        missing_required_checks: missing_min_invocation_tools(reasons),
        coverage_gap_actions,
        allowed_tools_override: Vec::new(),
        forbidden_tools: Vec::new(),
        directive_message: None,
    })
}

pub fn submit_repair_mode_from_submit_result(
    tool_name: &str,
    result: &serde_json::Value,
) -> Option<SubmitRepairMode> {
    if tool_name != "submit_stage_deliverable" {
        return None;
    }
    let status = result.get("status").and_then(|s| s.as_str());
    if status != Some("needs_fix") {
        return None;
    }

    let reasons = collect_json_array_strings(result.get("reasons"));
    let reason = if reasons.is_empty() {
        "(no reasons returned)".to_string()
    } else {
        reasons.join(" | ")
    };
    let reason_lc = reason.to_ascii_lowercase();
    let has_running_jobs = result
        .get("running_background_jobs")
        .and_then(|v| v.as_array())
        .map(|jobs| !jobs.is_empty())
        .unwrap_or(false)
        || reason_lc.contains("background job")
        || reason_lc.contains("wait_for_background_jobs");
    if has_running_jobs {
        return Some(SubmitRepairMode {
            kind: SubmitRepairKind::BackgroundJobs,
            reason,
            missing_required_checks: Vec::new(),
            coverage_gap_actions: Vec::new(),
            allowed_tools_override: Vec::new(),
            forbidden_tools: Vec::new(),
            directive_message: None,
        });
    }

    if let Some(mode) =
        submit_coverage_gap_repair_mode(&reasons, collect_coverage_gap_actions(result))
    {
        return Some(mode);
    }

    let available_ids = collect_json_array_strings(result.get("available_evidence_ids"));
    if !available_ids.is_empty() && needs_fix_evidence_ref_problem(&reasons) {
        return Some(SubmitRepairMode {
            kind: SubmitRepairKind::EvidenceRefs,
            reason,
            missing_required_checks: missing_min_invocation_tools(&reasons),
            coverage_gap_actions: Vec::new(),
            allowed_tools_override: Vec::new(),
            forbidden_tools: Vec::new(),
            directive_message: None,
        });
    }
    None
}

fn submit_repair_update(
    tool_name: &str,
    result: &serde_json::Value,
) -> Option<SubmitRepairModeUpdate> {
    if tool_name != "submit_stage_deliverable" {
        return None;
    }
    let status = result.get("status").and_then(|s| s.as_str());
    if matches!(status, Some("accepted" | "received")) {
        return Some(SubmitRepairModeUpdate::Clear);
    }
    submit_repair_mode_from_submit_result(tool_name, result).map(SubmitRepairModeUpdate::Set)
}

fn submit_needs_fix_runtime_correction(
    tool_name: &str,
    result: &mut serde_json::Value,
) -> Option<String> {
    if tool_name != "submit_stage_deliverable" {
        return None;
    }
    if result.get("status").and_then(|s| s.as_str()) != Some("needs_fix") {
        return None;
    }

    let available_ids = collect_json_array_strings(result.get("available_evidence_ids"));
    let reasons = collect_json_array_strings(result.get("reasons"));
    let coverage_gap_actions = collect_coverage_gap_actions(result);
    let reason_text = reasons.join(" | ");
    if needs_fix_coverage_gap(&reasons) {
        let ids_hint = if available_ids.is_empty() {
            "No available_evidence_ids were returned for this needs_fix.".to_string()
        } else {
            let mut preview_ids = available_ids.iter().take(20).cloned().collect::<Vec<_>>();
            if available_ids.len() > preview_ids.len() {
                preview_ids.push(format!(
                    "... +{} more",
                    available_ids.len() - preview_ids.len()
                ));
            }
            format!(
                "Real evidence ids currently available in this run: [{}].",
                preview_ids.join(", ")
            )
        };
        let mut correction = format!(
            "submit_stage_deliverable returned coverage gaps, not a pure evidence-ref rewrite. \
             {ids_hint} Close ONLY the assets/techniques named by the gate: query_target_data or \
             prior wait_for_background_jobs output first, then run targeted stage-allowed probes \
             only for missing coverage cells. Do not restart broad discovery or rescan \
             already-covered assets. After every named gap has a terminal coverage status, \
             resubmit the StageDeliverable. Gate reasons: {}",
            if reason_text.is_empty() {
                "(none provided)"
            } else {
                reason_text.as_str()
            }
        );
        if !coverage_gap_actions.is_empty() {
            correction.push(' ');
            correction.push_str(&format_coverage_gap_actions_for_runtime(
                &coverage_gap_actions,
            ));
        }
        if let Some(obj) = result.as_object_mut() {
            obj.insert(
                "runtime_correction".to_string(),
                serde_json::Value::String(correction.clone()),
            );
        }
        return Some(correction);
    }

    if available_ids.is_empty() {
        return None;
    }

    if !needs_fix_evidence_ref_problem(&reasons) {
        return None;
    }
    let missing_checks = missing_min_invocation_tools(&reasons);
    let required_checks_instruction = if missing_checks.is_empty() {
        String::new()
    } else {
        format!(
            " Also set required_checks_done to include [{}] when the cited evidence backs those checks.",
            missing_checks.join(", ")
        )
    };

    let mut preview_ids = available_ids.iter().take(20).cloned().collect::<Vec<_>>();
    if available_ids.len() > preview_ids.len() {
        preview_ids.push(format!(
            "... +{} more",
            available_ids.len() - preview_ids.len()
        ));
    }

    let correction = format!(
        "submit_stage_deliverable already returned real evidence ids available in this run: [{}]. \
         Do NOT launch more scans just to fix evidence_refs. Rebuild and resubmit the \
         StageDeliverable using these exact ids in top-level evidence_refs and in every \
         claim/finding evidence_ids/evidence_refs required by the gate.{required_checks_instruction} \
         If the mapping is unclear, call query_target_data or inspect the prior \
         wait_for_background_jobs output, then submit again. Gate reasons: {}",
        preview_ids.join(", "),
        if reason_text.is_empty() {
            "(none provided)"
        } else {
            reason_text.as_str()
        }
    );
    if let Some(obj) = result.as_object_mut() {
        obj.insert(
            "runtime_correction".to_string(),
            serde_json::Value::String(correction.clone()),
        );
    }
    Some(correction)
}

fn collect_coverage_gap_actions(result: &serde_json::Value) -> Vec<CoverageGapAction> {
    result
        .get("coverage_gap_actions")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value::<CoverageGapAction>(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn format_coverage_gap_actions_for_runtime(actions: &[CoverageGapAction]) -> String {
    let mut preview = actions
        .iter()
        .take(40)
        .enumerate()
        .map(|(idx, action)| {
            let tools = if action.suggested_tools.is_empty() {
                String::new()
            } else {
                format!("; suggested_tools={}", action.suggested_tools.join(", "))
            };
            format!(
                "{}. asset={} technique={} reason={}{}",
                idx + 1,
                action.asset,
                action.technique,
                action.reason,
                tools
            )
        })
        .collect::<Vec<_>>();
    if actions.len() > preview.len() {
        preview.push(format!("... +{} more", actions.len() - preview.len()));
    }
    format!(
        "Exact coverage_gap_actions returned by the gate: {}. Run ONLY these listed target/technique pairs.",
        preview.join(" | ")
    )
}

fn background_failure_runtime_correction(
    tool_args: &serde_json::Value,
    result: &serde_json::Value,
    success: bool,
) -> Option<String> {
    if success || tool_args.get("background").and_then(|v| v.as_bool()) != Some(true) {
        return None;
    }
    let error_summary = result
        .get("error")
        .or_else(|| result.get("stderr"))
        .or_else(|| result.get("stdout"))
        .and_then(|v| v.as_str())
        .map(|s| truncate_str(s, 500).to_string())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "no error text provided".to_string());

    Some(format!(
        "This tool was requested with background:true, but the returned result is a FAILURE. \
         Do NOT treat it as a running background job or as collected coverage. Read the error, \
         correct the arguments (or choose another allowed tool), then retry once if needed. \
         Error summary: {error_summary}"
    ))
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
    submit_repair_mode: Option<&SubmitRepairMode>,
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
    let mut submit_repair_update_seen: Option<SubmitRepairModeUpdate> = None;
    let mut hard_supervisor_active = false;

    for tool_call in tool_calls {
        let tool_name = &tool_call.function.name;
        if cancellation_requested(ctx.cancelled) {
            tracing::info!(
                "[sub-agent:{}] cancelled before tool call '{}'",
                agent_id,
                tool_name
            );
            return ToolDispatchResult {
                tool_results,
                barrier_hit,
                barrier_response,
                stage_block_signature: last_block_sig,
                submit_repair_update: submit_repair_update_seen,
                cancelled: true,
            };
        }
        if hard_supervisor_active {
            let tool_args = if tool_name == "run_pty_cmd" {
                tool_provider.normalize_run_pty_cmd_args(tool_call.function.arguments.clone())
            } else {
                tool_call.function.arguments.clone()
            };
            let request_id = Uuid::new_v4().to_string();
            let tool_request_event = AiEvent::SubAgentToolRequest {
                agent_id: agent_id.to_string(),
                tool_name: tool_name.to_string(),
                args: tool_args,
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

            let result_value = serde_json::json!({
                "error": "Skipped without execution because a hard execution supervisor correction was injected earlier in this tool batch. Start a new turn, read the supervisor correction, and choose the next action only after satisfying it.",
                "blocked_by_hard_supervisor": true,
            });
            let tool_result_event = AiEvent::SubAgentToolResult {
                agent_id: agent_id.to_string(),
                tool_name: tool_name.to_string(),
                success: false,
                result: result_value.clone(),
                request_id,
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
            let result_text = serde_json::to_string(&result_value).unwrap_or_default();
            tool_results.push(UserContent::ToolResult(ToolResult {
                id: tool_id,
                call_id: Some(tool_call_id),
                content: OneOrMany::one(ToolResultContent::Text(Text { text: result_text })),
            }));
            last_activity.store(epoch_secs(), Ordering::Relaxed);
            continue;
        }

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
                        active_org_id_source: ctx.active_org_id_source.clone(),
                        active_org_id_override: ctx.active_org_id_override,
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
                        post_tool_result_hook: ctx.post_tool_result_hook.clone(),
                        tool_observer: ctx.tool_observer.clone(),
                        initial_submit_repair_mode: ctx.initial_submit_repair_mode.clone(),
                        cancelled: ctx.cancelled,
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
        let use_outer_tool_timeout = use_sub_agent_outer_tool_timeout(tool_name);
        let tool_result = tokio::select! {
            _ = wait_for_cancelled(ctx.cancelled) => {
                tracing::info!(
                    "[sub-agent:{}] cancelled while waiting for tool '{}'",
                    agent_id,
                    tool_name
                );
                return ToolDispatchResult {
                    tool_results,
                    barrier_hit,
                    barrier_response,
                    stage_block_signature: last_block_sig,
                    submit_repair_update: submit_repair_update_seen,
                    cancelled: true,
                };
            }
            result = async {
                let tool_fut = async {
            if let Some(blocked) = submit_repair_mode
                .and_then(|mode| mode.block_result_with_args(tool_name, &tool_args))
            {
                tracing::warn!(
                    target: "harness::submit_repair",
                    agent_id = %agent_id,
                    tool = %tool_name,
                    "sub-agent tool call BLOCKED by submit repair mode"
                );
                return (blocked, false);
            }
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
                let tool_context = golish_core::AgentToolContext {
                    request_id: request_id.clone(),
                    tool_name: tool_name.to_string(),
                    source: ToolSource::SubAgent {
                        agent_id: agent_id.to_string(),
                        agent_name: agent_def.name.clone(),
                    },
                    organization_id: ctx.active_org_id_override,
                };
                golish_core::with_agent_session(
                    ctx.session_id.map(str::to_string),
                    golish_core::with_agent_tool_context(
                        Some(tool_context),
                        golish_core::with_agent_tool_output_sender(
                            Some(ctx.event_tx.clone()),
                            async {
                                // Try the injected router first (security/graph tools that live
                                // outside the ToolRegistry); fall through to the registry.
                                let routed = match &ctx.sub_tool_router {
                                    Some(router) => {
                                        router(tool_name.to_string(), tool_args.clone()).await
                                    }
                                    None => None,
                                };
                                match routed {
                                    Some((value, success)) => (value, success),
                                    None => {
                                        match execute_registry_tool_with_active_org(
                                            ctx,
                                            tool_name,
                                            tool_args.clone(),
                                        )
                                        .await
                                        {
                                            Ok(v) => registry_tool_outcome(v),
                                            Err(e) => (
                                                serde_json::json!({ "error": e.to_string() }),
                                                false,
                                            ),
                                        }
                                    }
                                }
                            },
                        ),
                    ),
                )
                .await
            }
                };
                if use_outer_tool_timeout {
                    tokio::time::timeout(tool_timeout, tool_fut).await
                } else {
                    Ok(tool_fut.await)
                }
            } => result,
        };

        let (mut result_value, mut success) = match tool_result {
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

        if let Some(hook) = ctx.post_tool_result_hook.as_ref() {
            let hook = Arc::clone(hook);
            let (hooked_value, hooked_success) = hook(
                tool_name.to_string(),
                tool_args.clone(),
                result_value,
                success,
            )
            .await;
            result_value = hooked_value;
            success = hooked_success;
        }

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

        let mut model_visible_notes: Vec<String> = Vec::new();
        if let Some(correction) = submit_needs_fix_runtime_correction(tool_name, &mut result_value)
        {
            model_visible_notes.push(format!(
                "\n\n--- RUNTIME CORRECTION ---\n{}\n--------------------------",
                correction
            ));
        }
        if let Some(correction) =
            background_failure_runtime_correction(&tool_args, &result_value, success)
        {
            model_visible_notes.push(format!(
                "\n\n--- RUNTIME CORRECTION ---\n{}\n--------------------------",
                correction
            ));
        }

        if let Some(observer) = ctx.tool_observer.as_ref() {
            let observation = SubAgentToolObservation {
                agent_id: agent_id.to_string(),
                agent_name: agent_def.name.clone(),
                parent_request_id: parent_request_id.to_string(),
                tool_name: tool_name.to_string(),
                tool_args: tool_args.clone(),
                result: result_value.clone(),
                success,
            };
            if let Some(note) = observer(observation).await {
                model_visible_notes.push(note);
            }
        }

        // Stage-stall circuit breaker: record a submit_stage_deliverable BLOCK so
        // the loop can bail after STAGE_STALL_THRESHOLD identical ones.
        if let Some(sig) = stage_block_signature(tool_name, &result_value) {
            last_block_sig = Some(sig);
        }
        if let Some(update) = submit_repair_update(tool_name, &result_value) {
            submit_repair_update_seen = Some(update);
        }

        if let Some(payload) =
            structured_storage_hook_payload(tool_name, &tool_args, &result_value, success)
        {
            if let Some(hook) = ctx.post_shell_hook.as_ref() {
                let pp = {
                    let ws = ctx.workspace.read().await;
                    ws.to_string_lossy().to_string()
                };
                let hook = Arc::clone(hook);
                let org_id = ctx.active_org_id_override;
                tokio::spawn(async move {
                    hook(payload.command, payload.stdout, Some(pp), org_id).await;
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

        let model_visible_result = model_visible_tool_result(tool_name, &result_value);
        let mut result_text = serde_json::to_string(&model_visible_result).unwrap_or_default();
        let hard_supervisor_injected = model_visible_notes
            .iter()
            .any(|note| note.contains(HARD_SUPERVISOR_MARKER));
        for note in model_visible_notes {
            result_text.push_str(&note);
        }
        tool_results.push(UserContent::ToolResult(ToolResult {
            id: tool_id,
            call_id: Some(tool_call_id),
            content: OneOrMany::one(ToolResultContent::Text(Text { text: result_text })),
        }));
        if hard_supervisor_injected {
            hard_supervisor_active = true;
        }
    }

    ToolDispatchResult {
        tool_results,
        barrier_hit,
        barrier_response,
        stage_block_signature: last_block_sig,
        submit_repair_update: submit_repair_update_seen,
        cancelled: false,
    }
}

fn registry_tool_outcome(value: serde_json::Value) -> (serde_json::Value, bool) {
    let success = is_tool_result_success(&value);
    (value, success)
}

fn use_sub_agent_outer_tool_timeout(tool_name: &str) -> bool {
    !matches!(
        tool_name,
        // These are Rust direct bridge tools that emit their own progress chunks
        // and can legitimately run past a sub-agent's LLM idle timeout on large
        // targets. Applying `tokio::time::timeout` here drops the future, so the
        // tool is killed before it can persist its final DB truth. Shell/pentest
        // commands already have a separate soft-timeout -> background-job path.
        "browser_collect_js_api" | "js_extract_apis" | "route_probe_paths"
    )
}

async fn execute_registry_tool_with_active_org(
    ctx: &SubAgentExecutorContext<'_>,
    tool_name: &str,
    mut tool_args: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let injected_org_arg =
        inject_harness_org_id_arg(tool_name, &mut tool_args, ctx.active_org_id_override);
    if injected_org_arg {
        let registry = ctx.tool_registry.read().await;
        return registry.execute_tool(tool_name, tool_args).await;
    }

    let previous = match (
        ctx.active_org_id_source.as_ref(),
        ctx.active_org_id_override,
    ) {
        (Some(source), Some(org_id)) => {
            let mut guard = source.write().await;
            let previous = *guard;
            *guard = Some(org_id);
            Some(previous)
        }
        _ => None,
    };

    let result = {
        let registry = ctx.tool_registry.read().await;
        registry.execute_tool(tool_name, tool_args).await
    };

    if let (Some(source), Some(previous)) = (ctx.active_org_id_source.as_ref(), previous) {
        *source.write().await = previous;
    }

    result
}

const HARNESS_ORG_ID_ARG: &str = "__harness_org_id";

fn inject_harness_org_id_arg(
    tool_name: &str,
    tool_args: &mut serde_json::Value,
    org_id: Option<uuid::Uuid>,
) -> bool {
    let Some(org_id) = org_id else {
        return false;
    };
    if !matches!(tool_name, "manage_targets" | "manage_organizations") {
        return false;
    }
    if !tool_args.is_object() {
        *tool_args = serde_json::json!({});
    }
    if let Some(obj) = tool_args.as_object_mut() {
        obj.insert(HARNESS_ORG_ID_ARG.to_string(), org_id.to_string().into());
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{
        annotate_list_tools_with_guard, background_failure_runtime_correction,
        execute_registry_tool_with_active_org, model_visible_tool_result, registry_tool_outcome,
        structured_storage_hook_payload, submit_coverage_gap_repair_mode_from_reasons,
        submit_needs_fix_runtime_correction, submit_repair_update,
        use_sub_agent_outer_tool_timeout, SubmitRepairModeUpdate,
    };
    use crate::SubmitRepairKind;
    use golish_core::Tool;
    use golish_tools::ToolRegistry;
    use std::path::Path;
    use std::sync::Arc;
    use tokio::sync::{mpsc, Mutex, RwLock};
    use uuid::Uuid;

    struct ObserveActiveOrgTool {
        active_org_id: Arc<RwLock<Option<Uuid>>>,
    }

    #[async_trait::async_trait]
    impl Tool for ObserveActiveOrgTool {
        fn name(&self) -> &'static str {
            "observe_active_org"
        }

        fn description(&self) -> &'static str {
            "test helper"
        }

        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({"type":"object","properties":{}})
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
            _workspace: &Path,
        ) -> anyhow::Result<serde_json::Value> {
            let observed = *self.active_org_id.read().await;
            Ok(serde_json::json!({
                "observed_org_id": observed.map(|id| id.to_string())
            }))
        }
    }

    struct ObserveManageTargetsArgsTool {
        observed_args: Arc<Mutex<Option<serde_json::Value>>>,
    }

    #[async_trait::async_trait]
    impl Tool for ObserveManageTargetsArgsTool {
        fn name(&self) -> &'static str {
            "manage_targets"
        }

        fn description(&self) -> &'static str {
            "test helper"
        }

        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({"type":"object","properties":{}})
        }

        async fn execute(
            &self,
            args: serde_json::Value,
            _workspace: &Path,
        ) -> anyhow::Result<serde_json::Value> {
            *self.observed_args.lock().await = Some(args.clone());
            Ok(args)
        }
    }

    #[tokio::test]
    async fn registry_tool_exec_temporarily_overrides_active_org_id() {
        let parent_org = Uuid::new_v4();
        let child_org = Uuid::new_v4();
        let active_org_id = Arc::new(RwLock::new(Some(parent_org)));

        let tmp = tempfile::tempdir().expect("tmpdir");
        let mut registry = ToolRegistry::new(tmp.path().to_path_buf()).await;
        registry.register_tool(Arc::new(ObserveActiveOrgTool {
            active_org_id: active_org_id.clone(),
        }));
        let registry = Arc::new(RwLock::new(registry));
        let workspace = Arc::new(RwLock::new(tmp.path().to_path_buf()));
        let (event_tx, _event_rx) = mpsc::unbounded_channel();

        let ctx = crate::executor_types::SubAgentExecutorContext {
            event_tx: &event_tx,
            tool_registry: &registry,
            workspace: &workspace,
            provider_name: "test",
            model_name: "test",
            session_id: None,
            transcript_base_dir: None,
            api_request_stats: None,
            cancelled: None,
            briefing: None,
            temperature_override: None,
            max_tokens_override: None,
            top_p_override: None,
            chain_persistence: None,
            sub_agent_registry: None,
            post_shell_hook: None,
            resume: None,
            sub_tool_router: None,
            active_org_id_source: Some(active_org_id.clone()),
            active_org_id_override: Some(child_org),
            post_tool_result_hook: None,
            tool_observer: None,
            initial_submit_repair_mode: None,
            stage_tool_guard: None,
            hide_tool_in_stage: None,
        };

        let out = execute_registry_tool_with_active_org(
            &ctx,
            "observe_active_org",
            serde_json::json!({}),
        )
        .await
        .expect("tool executes");

        assert_eq!(out["observed_org_id"], child_org.to_string());
        assert_eq!(
            *active_org_id.read().await,
            Some(parent_org),
            "parent active org must be restored after the registry tool call"
        );
    }

    #[tokio::test]
    async fn manage_targets_gets_hidden_harness_org_arg_without_mutating_global_org() {
        let parent_org = Uuid::new_v4();
        let child_org = Uuid::new_v4();
        let active_org_id = Arc::new(RwLock::new(Some(parent_org)));
        let observed_args = Arc::new(Mutex::new(None));

        let tmp = tempfile::tempdir().expect("tmpdir");
        let mut registry = ToolRegistry::new(tmp.path().to_path_buf()).await;
        registry.register_tool(Arc::new(ObserveManageTargetsArgsTool {
            observed_args: observed_args.clone(),
        }));
        let registry = Arc::new(RwLock::new(registry));
        let workspace = Arc::new(RwLock::new(tmp.path().to_path_buf()));
        let (event_tx, _event_rx) = mpsc::unbounded_channel();

        let ctx = crate::executor_types::SubAgentExecutorContext {
            event_tx: &event_tx,
            tool_registry: &registry,
            workspace: &workspace,
            provider_name: "test",
            model_name: "test",
            session_id: None,
            transcript_base_dir: None,
            api_request_stats: None,
            cancelled: None,
            briefing: None,
            temperature_override: None,
            max_tokens_override: None,
            top_p_override: None,
            chain_persistence: None,
            sub_agent_registry: None,
            post_shell_hook: None,
            resume: None,
            sub_tool_router: None,
            active_org_id_source: Some(active_org_id.clone()),
            active_org_id_override: Some(child_org),
            post_tool_result_hook: None,
            tool_observer: None,
            initial_submit_repair_mode: None,
            stage_tool_guard: None,
            hide_tool_in_stage: None,
        };

        let out = execute_registry_tool_with_active_org(
            &ctx,
            "manage_targets",
            serde_json::json!({"action": "list"}),
        )
        .await
        .expect("tool executes");

        assert_eq!(out["__harness_org_id"], child_org.to_string());
        assert_eq!(
            observed_args.lock().await.as_ref().unwrap()["__harness_org_id"],
            child_org.to_string()
        );
        assert_eq!(
            *active_org_id.read().await,
            Some(parent_org),
            "manage_targets must not rely on temporarily mutating the global active org"
        );
    }

    #[test]
    fn registry_tool_success_detection_rejects_whatweb_runtime_error() {
        let result = serde_json::json!({
            "stdout": "",
            "stderr": "ERROR Opening: https://example.test - can't modify frozen Hash: {verify_mode: 1, verify_hostname: true}",
            "exit_code": 0,
            "tool": "whatweb",
        });

        let (_value, success) = registry_tool_outcome(result);

        assert!(
            !success,
            "sub-agent registry fallback must not turn WhatWeb stderr ERROR into a green check"
        );
    }

    #[test]
    fn route_probe_model_visible_result_keeps_counts_and_samples() {
        let result = serde_json::json!({
            "success": true,
            "base_url": "https://example.test/",
            "requested_base_url": "https://example.test/",
            "outcome": "found",
            "outcome_persisted": true,
            "status": "ok",
            "timed_out": false,
            "queue_completed": true,
            "queue_remaining": 0,
            "requests_sent": 42,
            "candidate_requests_sent": 40,
            "baseline_requests_sent": 2,
            "persisted_directory_entries": 1,
            "matches": [
                {"url": "https://example.test/admin", "status": 403, "verification": {"verdict": "auth_wall"}}
            ],
            "rejected_candidates": [
                {"url": "https://example.test/nope", "status": 200, "verification": {"verdict": "soft_404"}}
            ],
            "errors": [
                {"url": "https://example.test/a", "error": "request timed out"},
                {"url": "https://example.test/b", "error": "request timed out"},
                {"url": "https://example.test/c", "error": "connection reset"}
            ],
            "prefixes_tested": ["/", "/admin"],
            "wordlist": {"entries_loaded": 100},
            "seed_paths": {"api_endpoints": 2},
            "run_id": "run-1",
            "dry_run": false
        });

        let compact = model_visible_tool_result("route_probe_paths", &result);

        assert_eq!(compact["model_visible_compacted"], true);
        assert_eq!(compact["raw_result_retained_in_transcript"], true);
        assert_eq!(compact["matches_count"], 1);
        assert_eq!(compact["rejected_candidates_count"], 1);
        assert_eq!(compact["errors_count"], 3);
        assert_eq!(compact["errors_top"][0]["error"], "request timed out");
        assert_eq!(compact["errors_top"][0]["count"], 2);
        assert!(compact.get("matches").is_none());
        assert!(compact.get("rejected_candidates").is_none());
        assert!(compact.get("errors").is_none());
        assert_eq!(
            compact["next_action"],
            "Refresh stage_worklist/check_stage_asset_coverage; do not rerun this root unless coverage still reports DIR pending/error."
        );
    }

    #[test]
    fn small_generic_tool_result_is_left_untouched() {
        let result = serde_json::json!({
            "success": true,
            "message": "ok",
            "items": [1, 2, 3]
        });

        let compact = model_visible_tool_result("some_small_tool", &result);

        assert_eq!(compact, result);
    }

    #[test]
    fn pentest_run_result_feeds_structured_storage_hook() {
        let payload = structured_storage_hook_payload(
            "pentest_run",
            &serde_json::json!({"tool_name": "httpx", "args": "-u https://example.com -sc"}),
            &serde_json::json!({
                "command": "httpx -u https://example.com -sc",
                "stdout": "https://example.com [200]",
                "stderr": "",
                "exit_code": 0
            }),
            true,
        )
        .expect("pentest_run should produce structured-storage payload");

        assert_eq!(payload.command, "httpx -u https://example.com -sc");
        assert_eq!(payload.stdout, "https://example.com [200]");
    }

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

    #[test]
    fn submit_needs_fix_correction_points_to_available_evidence_ids() {
        let mut v = serde_json::json!({
            "status": "needs_fix",
            "available_evidence_ids": [101, 102, 103],
            "reasons": [
                "FakePattern: total evidence_refs (0) below min_invocations sum (1)",
                "min tool invocations not satisfied for 'http_probe' (not in required_checks_done)",
                "Every finding must cite real evidence ids"
            ]
        });

        let note = submit_needs_fix_runtime_correction("submit_stage_deliverable", &mut v)
            .expect("evidence-ref needs_fix should produce correction");

        assert!(note.contains("101, 102, 103"));
        assert!(note.contains("Do NOT launch more scans"));
        assert!(note.contains("required_checks_done"));
        assert!(note.contains("http_probe"));
        assert!(v["runtime_correction"]
            .as_str()
            .unwrap()
            .contains("top-level evidence_refs"));
    }

    #[test]
    fn evidence_ref_needs_fix_enters_repair_mode_and_blocks_scans() {
        let v = serde_json::json!({
            "status": "needs_fix",
            "available_evidence_ids": [101],
            "reasons": [
                "FakePattern: total evidence_refs (0) below min_invocations sum (1)",
                "min tool invocations not satisfied for 'http_probe' (not in required_checks_done)",
                "This operation's REAL evidence ids (newest first) are [101]."
            ]
        });
        let update = submit_repair_update("submit_stage_deliverable", &v)
            .expect("needs_fix should activate repair mode");
        let SubmitRepairModeUpdate::Set(mode) = update else {
            panic!("expected Set repair mode");
        };

        assert!(mode.block_result("query_target_data").is_none());
        assert!(mode.block_result("submit_stage_deliverable").is_none());
        let blocked = mode
            .block_result("pentest_run")
            .expect("repair mode blocks fresh scans");
        assert_eq!(blocked["blocked_by_submit_repair"], true);
        assert!(blocked["error"]
            .as_str()
            .unwrap()
            .contains("Do NOT start fresh"));
        assert!(blocked["error"].as_str().unwrap().contains("http_probe"));
    }

    #[test]
    fn background_jobs_needs_fix_enters_wait_only_repair_mode() {
        let v = serde_json::json!({
            "status": "needs_fix",
            "reasons": ["1 background job(s) you launched are still running"],
            "running_background_jobs": [{"job_id": "job_1"}]
        });
        let update = submit_repair_update("submit_stage_deliverable", &v)
            .expect("running jobs should activate wait repair mode");
        let SubmitRepairModeUpdate::Set(mode) = update else {
            panic!("expected Set repair mode");
        };

        assert!(mode.block_result("wait_for_background_jobs").is_none());
        assert!(mode.block_result("submit_stage_deliverable").is_none());
        let blocked = mode
            .block_result("pentest_run")
            .expect("wait repair mode blocks replacement scans");
        assert!(blocked["error"]
            .as_str()
            .unwrap()
            .contains("wait_for_background_jobs"));
    }

    #[test]
    fn coverage_gap_needs_fix_enters_targeted_gap_closure_mode() {
        let v = serde_json::json!({
            "status": "needs_fix",
            "available_evidence_ids": [4336, 4335],
            "coverage_gap_actions": [
                {
                    "asset": "101.69.134.6",
                    "technique": "GOLISH-EAS-LIVENESS",
                    "reason": "missing_terminal_coverage",
                    "suggested_tools": ["httpx", "nmap -sn"]
                },
                {
                    "asset": "www.example.com",
                    "technique": "GOLISH-EAS-SERVICE-FINGERPRINT",
                    "reason": "missing_terminal_coverage",
                    "suggested_tools": ["nmap -sV", "whatweb"]
                }
            ],
            "reasons": [
                "external attack surface incomplete: (101.69.134.6 x GOLISH-EAS-LIVENESS) never attempted",
                "This operation's REAL evidence ids (newest first) are [4336, 4335]."
            ]
        });
        let update = submit_repair_update("submit_stage_deliverable", &v)
            .expect("coverage gaps should activate targeted repair mode");
        let SubmitRepairModeUpdate::Set(mode) = update else {
            panic!("expected Set repair mode");
        };

        assert_eq!(mode.kind, SubmitRepairKind::CoverageGap);
        assert_eq!(mode.coverage_gap_actions.len(), 2);
        assert!(mode
            .model_instruction()
            .contains("Exact coverage_gap_actions"));
        assert!(mode.model_instruction().contains("www.example.com"));
        assert!(mode.block_result("query_target_data").is_none());
        assert!(mode.block_result("check_stage_asset_coverage").is_none());
        assert!(mode.block_result("pentest_run").is_none());
        assert!(mode.block_result("submit_stage_deliverable").is_none());
        let blocked = mode
            .block_result("list_in_scope_targets")
            .expect("coverage repair should not restart full inventory");
        assert_eq!(blocked["repair_kind"], "coverage_gap");
        let blocked = mode
            .block_result("subfinder")
            .expect("unknown fresh discovery tool remains blocked");
        assert_eq!(blocked["repair_kind"], "coverage_gap");
        assert_eq!(
            blocked["coverage_gap_actions"]
                .as_array()
                .expect("actions included in block payload")
                .len(),
            2
        );
        assert!(blocked["error"]
            .as_str()
            .unwrap()
            .contains("Targeted gap-closure"));
    }

    #[test]
    fn coverage_gap_repair_allows_batch_input_lines_for_listed_gap_targets() {
        let v = serde_json::json!({
            "status": "needs_fix",
            "reasons": ["external attack surface incomplete: never attempted"],
            "coverage_gap_actions": [
                {
                    "asset": "112.65.238.93",
                    "technique": "GOLISH-EAS-LIVENESS",
                    "reason": "missing_terminal_coverage",
                    "suggested_tools": ["httpx"]
                },
                {
                    "asset": "113.105.78.22",
                    "technique": "GOLISH-EAS-LIVENESS",
                    "reason": "missing_terminal_coverage",
                    "suggested_tools": ["httpx"]
                }
            ]
        });
        let update = submit_repair_update("submit_stage_deliverable", &v)
            .expect("coverage gaps should activate repair mode");
        let SubmitRepairModeUpdate::Set(mode) = update else {
            panic!("expected Set repair mode");
        };

        assert!(mode
            .block_result_with_args(
                "pentest_run",
                &serde_json::json!({
                    "tool_name": "httpx",
                    "args": "-json -sc -silent -l {{input_file}}",
                    "input_lines": ["112.65.238.93", "113.105.78.22"]
                }),
            )
            .is_none());
    }

    #[test]
    fn coverage_gap_repair_allows_direct_enumeration_tools_for_listed_gap_targets() {
        let v = serde_json::json!({
            "status": "needs_fix",
            "reasons": ["enumeration incomplete: never attempted"],
            "coverage_gap_actions": [
                {
                    "asset": "https://app.example.com",
                    "technique": "GOLISH-ENUM-JSAPI",
                    "reason": "missing_terminal_coverage",
                    "suggested_tools": ["browser_collect_js_api", "js_extract_apis"]
                },
                {
                    "asset": "https://app.example.com",
                    "technique": "GOLISH-ENUM-DIR",
                    "reason": "missing_terminal_coverage",
                    "suggested_tools": ["route_probe_paths"]
                }
            ]
        });
        let update = submit_repair_update("submit_stage_deliverable", &v)
            .expect("enumeration coverage gaps should activate repair mode");
        let SubmitRepairModeUpdate::Set(mode) = update else {
            panic!("expected Set repair mode");
        };

        assert!(mode.allows("browser_collect_js_api"));
        assert!(mode.allows("js_collect"));
        assert!(mode.allows("js_extract_apis"));
        assert!(mode.allows("route_probe_paths"));
        assert!(mode
            .block_result_with_args(
                "browser_collect_js_api",
                &serde_json::json!({"target_url": "https://app.example.com"})
            )
            .is_none());
        assert!(mode
            .block_result_with_args(
                "route_probe_paths",
                &serde_json::json!({
                    "target_id": "target-1",
                    "base_url": "https://app.example.com"
                })
            )
            .is_none());

        let missing_target = mode
            .block_result_with_args(
                "route_probe_paths",
                &serde_json::json!({"target_id": "target-1"}),
            )
            .expect("direct enumeration repair tools must name their root");
        assert!(missing_target["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("base_url"));

        let blocked = mode
            .block_result_with_args(
                "browser_collect_js_api",
                &serde_json::json!({"target_url": "https://other.example.com"}),
            )
            .expect("unlisted direct enumeration target should be blocked");
        assert!(blocked["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("not in coverage_gap_actions"));
    }

    #[test]
    fn coverage_gap_repair_blocks_list_file_without_visible_targets() {
        let v = serde_json::json!({
            "status": "needs_fix",
            "reasons": ["external attack surface incomplete: never attempted"],
            "coverage_gap_actions": [{
                "asset": "112.65.238.93",
                "technique": "GOLISH-EAS-LIVENESS",
                "reason": "missing_terminal_coverage",
                "suggested_tools": ["httpx"]
            }]
        });
        let update = submit_repair_update("submit_stage_deliverable", &v)
            .expect("coverage gaps should activate repair mode");
        let SubmitRepairModeUpdate::Set(mode) = update else {
            panic!("expected Set repair mode");
        };

        let blocked = mode
            .block_result_with_args(
                "pentest_run",
                &serde_json::json!({
                    "tool_name": "httpx",
                    "args": "-l /tmp/all-targets.txt -silent"
                }),
            )
            .expect("hidden list-file probes should be blocked");

        assert_eq!(blocked["blocked_by_submit_repair"], true);
        assert_eq!(blocked["repair_kind"], "coverage_gap");
        assert!(blocked["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("input_lines"));
    }

    #[test]
    fn coverage_gap_repair_without_actions_blocks_pentest_run() {
        let mode = submit_coverage_gap_repair_mode_from_reasons(&[
            "external attack surface incomplete: never attempted (112.65.238.93 x GOLISH-EAS-SERVICE-FINGERPRINT)"
                .to_string(),
        ])
        .expect("coverage gaps should activate repair mode");

        let blocked = mode
            .block_result_with_args(
                "pentest_run",
                &serde_json::json!({
                    "tool_name": "nmap",
                    "args": "-sV --top-ports 100 -T4 112.65.238.93"
                }),
            )
            .expect("coverage repair without structured actions should not scan");

        assert_eq!(blocked["repair_kind"], "coverage_gap");
        assert!(blocked["error"].as_str().unwrap().contains("did not name"));
        assert!(blocked["allowed_tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| tool.as_str() != Some("pentest_run")));
    }

    #[test]
    fn coverage_gap_repair_blocks_single_target_outside_action_list() {
        let v = serde_json::json!({
            "status": "needs_fix",
            "reasons": ["external attack surface incomplete: never attempted"],
            "coverage_gap_actions": [{
                "asset": "112.65.238.93",
                "technique": "GOLISH-EAS-LIVENESS",
                "reason": "missing_terminal_coverage",
                "suggested_tools": ["httpx"]
            }]
        });
        let update = submit_repair_update("submit_stage_deliverable", &v)
            .expect("structured coverage gaps should activate repair mode");
        let SubmitRepairModeUpdate::Set(mode) = update else {
            panic!("expected Set repair mode");
        };

        assert!(mode
            .block_result_with_args(
                "pentest_run",
                &serde_json::json!({
                    "tool_name": "httpx",
                    "args": "-u https://112.65.238.93 -silent -json"
                }),
            )
            .is_none());

        let blocked = mode
            .block_result_with_args(
                "pentest_run",
                &serde_json::json!({
                    "tool_name": "httpx",
                    "args": "-u https://203.0.113.10 -silent"
                }),
            )
            .expect("unlisted target should be blocked");
        assert_eq!(blocked["repair_kind"], "coverage_gap");
        assert!(blocked["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("not in coverage_gap_actions"));
    }

    #[test]
    fn coverage_gap_repair_blocks_cidr_pentest_run() {
        let v = serde_json::json!({
            "status": "needs_fix",
            "reasons": ["external attack surface incomplete: never attempted"],
            "coverage_gap_actions": [{
                "asset": "124.196.9.134",
                "technique": "GOLISH-EAS-LIVENESS",
                "reason": "missing_terminal_coverage",
                "suggested_tools": ["httpx"]
            }]
        });
        let update = submit_repair_update("submit_stage_deliverable", &v)
            .expect("coverage gaps should activate repair mode");
        let SubmitRepairModeUpdate::Set(mode) = update else {
            panic!("expected Set repair mode");
        };

        let blocked = mode
            .block_result_with_args(
                "pentest_run",
                &serde_json::json!({
                    "tool_name": "masscan",
                    "args": "124.196.9.0/24 -p 53,80,443,22 --rate 1000 -oL masscan.out"
                }),
            )
            .expect("CIDR sweeps should be blocked during coverage repair");

        assert_eq!(blocked["repair_kind"], "coverage_gap");
        assert!(blocked["blocked_reason"].as_str().unwrap().contains("CIDR"));
    }

    #[test]
    fn coverage_gap_repair_blocks_multi_target_pentest_run() {
        let v = serde_json::json!({
            "status": "needs_fix",
            "reasons": ["external attack surface incomplete: never attempted"],
            "coverage_gap_actions": [{
                "asset": "124.196.9.134",
                "technique": "GOLISH-EAS-LIVENESS",
                "reason": "missing_terminal_coverage",
                "suggested_tools": ["httpx"]
            }]
        });
        let update = submit_repair_update("submit_stage_deliverable", &v)
            .expect("coverage gaps should activate repair mode");
        let SubmitRepairModeUpdate::Set(mode) = update else {
            panic!("expected Set repair mode");
        };

        let blocked = mode
            .block_result_with_args(
                "pentest_run",
                &serde_json::json!({
                    "tool_name": "nmap",
                    "args": "-Pn -p 80,443,8080,8443,22,3389 -T4 124.196.9.134 124.196.9.146"
                }),
            )
            .expect("multi-target probes should be blocked during coverage repair");

        assert_eq!(blocked["repair_kind"], "coverage_gap");
        assert!(blocked["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("not in coverage_gap_actions"));
    }

    #[test]
    fn submit_needs_fix_correction_guides_coverage_gap_without_ids() {
        let mut v = serde_json::json!({
            "status": "needs_fix",
            "reasons": ["coverage cell missing for host x service fingerprint"],
            "coverage_gap_actions": [{
                "asset": "app.example.com",
                "technique": "GOLISH-EAS-SERVICE-FINGERPRINT",
                "reason": "missing_terminal_coverage",
                "suggested_tools": ["nmap -sV"]
            }]
        });

        let note = submit_needs_fix_runtime_correction("submit_stage_deliverable", &mut v)
            .expect("coverage needs_fix should produce targeted correction");

        assert!(note.contains("coverage gaps"));
        assert!(note.contains("Exact coverage_gap_actions"));
        assert!(note.contains("app.example.com"));
        assert!(note.contains("targeted stage-allowed probes"));
        assert!(!note.contains("Do NOT launch more scans"));
        assert!(v.get("runtime_correction").is_some());
    }

    #[test]
    fn background_true_failure_gets_runtime_correction() {
        let note = background_failure_runtime_correction(
            &serde_json::json!({ "background": true, "tool_name": "naabu" }),
            &serde_json::json!({
                "stdout": "flag provided but not defined: -ports",
                "exit_code": 2
            }),
            false,
        )
        .expect("failed background request should be corrected");

        assert!(note.contains("background:true"));
        assert!(note.contains("Do NOT treat it as a running background job"));
        assert!(note.contains("-ports"));
    }

    #[test]
    fn background_true_success_gets_no_runtime_correction() {
        assert!(background_failure_runtime_correction(
            &serde_json::json!({ "background": true }),
            &serde_json::json!({ "job_id": "job_1" }),
            true,
        )
        .is_none());
    }

    #[test]
    fn long_direct_bridge_tools_bypass_sub_agent_outer_timeout() {
        for tool_name in [
            "browser_collect_js_api",
            "js_extract_apis",
            "route_probe_paths",
        ] {
            assert!(
                !use_sub_agent_outer_tool_timeout(tool_name),
                "{tool_name} should keep running instead of being dropped by the sub-agent outer timeout"
            );
        }

        assert!(use_sub_agent_outer_tool_timeout("query_target_data"));
        assert!(use_sub_agent_outer_tool_timeout("pentest_run"));
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
