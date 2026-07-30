//! Tool call dispatch and response parsing for sub-agent execution.
//!
//! Extracts the tool execution loop from the main orchestrator, handling
//! barrier tools, nested sub-agent delegation, regular tool execution,
//! event emission, and file modification tracking.

use std::collections::{BTreeMap, BTreeSet, HashSet};
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
    cancellation_requested, coverage_gap_action_instruction, normalize_probe_target,
    wait_for_cancelled, CoverageGapAction, EasWebRepairTarget, StageTeamLeaderBinding,
    SubAgentExecutorContext, SubAgentToolObservation, SubmitRepairKind, SubmitRepairMode,
    ToolProvider, BARRIER_TOOL_NAME, STAGE_TEAM_DISPATCH_ACCEPTED_STATUS,
    STAGE_TEAM_DISPATCH_WORKERS_TOOL_NAME, STAGE_TEAM_PREPARE_FINAL_STATUS,
    STAGE_TEAM_PREPARE_FINAL_SUBMISSION_TOOL_NAME,
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

/// Return the host-visible barrier response for a durable stage submission.
///
/// Accepted submissions remain terminal for every specialist. A durable
/// `needs_fix` submission is terminal only when the host explicitly owns the
/// repair generation (currently the Stage Team Aggregator); ordinary stage
/// workers continue their existing in-chain repair loop.
pub(super) fn stage_submission_barrier_response(
    tool_name: &str,
    result: &serde_json::Value,
    return_on_first_durable_submission: bool,
) -> Option<String> {
    if tool_name != "submit_stage_deliverable" {
        return None;
    }
    let status = result.get("status").and_then(serde_json::Value::as_str)?;
    let submission_id = result
        .get("deliverable_submission_id")
        .and_then(serde_json::Value::as_str);
    match status {
        "accepted" => Some(match submission_id {
            Some(submission_id) => format!(
                "Stage deliverable accepted; returning control to the stage orchestrator.\n\n[deliverable_submission_id: {submission_id}]"
            ),
            None => "Stage deliverable accepted without a durable submission id; returning control to the stage orchestrator."
                .to_string(),
        }),
        "needs_fix" if return_on_first_durable_submission => submission_id.map(|submission_id| {
            format!(
                "Stage deliverable needs deterministic repair; returning the durable submission to the stage orchestrator.\n\n[deliverable_submission_id: {submission_id}]"
            )
        }),
        _ => None,
    }
}

/// A claimed Company Controller coordination turn is host-owned and can only
/// return through one of its trusted router tools. `submit_result` is the
/// generic specialist barrier; accepting it here would let model prose bypass
/// the durable dispatch/prepare-final state machine.
fn stage_team_controller_submit_result_rejection(
    active_company_controller: bool,
) -> Option<serde_json::Value> {
    active_company_controller.then(|| {
        serde_json::json!({
            "error": "Company Controller coordination cannot end with submit_result. Call exactly one trusted coordination tool: stage_team_dispatch_workers or stage_team_prepare_final_submission.",
            "code": "STAGE_TEAM_CONTROLLER_REQUIRES_ROUTER",
            "blocked_by_controller_router": true,
            "next_action": "Continue this same turn and call stage_team_dispatch_workers or stage_team_prepare_final_submission; do not call submit_result.",
        })
    })
}

/// Recognize only successful host-router control results for an exact Company
/// Controller binding. The raw JSON is returned unchanged so the outer
/// scheduler can consume its durable IDs/counts after the tool-result turn has
/// been checkpointed.
pub(super) fn stage_team_leader_router_barrier_response(
    tool_name: &str,
    result: &serde_json::Value,
    binding: Option<&StageTeamLeaderBinding>,
) -> Option<String> {
    binding?;
    let expected_status = match tool_name {
        STAGE_TEAM_DISPATCH_WORKERS_TOOL_NAME => STAGE_TEAM_DISPATCH_ACCEPTED_STATUS,
        STAGE_TEAM_PREPARE_FINAL_SUBMISSION_TOOL_NAME => STAGE_TEAM_PREPARE_FINAL_STATUS,
        _ => return None,
    };
    (result.get("status").and_then(serde_json::Value::as_str) == Some(expected_status))
        .then(|| serde_json::to_string(result).ok())
        .flatten()
}

/// A Candidate terminal intent is a stricter barrier than an ordinary tool
/// success: once persisted, the verifier may not perform any more external
/// action. The host first checkpoints this tool result and then consumes the
/// intent with server authority.
fn candidate_terminal_intent_persisted(
    tool_name: &str,
    result: &serde_json::Value,
) -> Option<String> {
    (tool_name == "submit_candidate_attempt"
        && result.get("status").and_then(|value| value.as_str())
            == Some("terminal_intent_persisted"))
    .then(|| {
        result
            .get("terminal_intent_id")
            .and_then(|value| value.as_str())
            .map(str::to_string)
    })
    .flatten()
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

fn tool_result_for_history(tool_call: &ToolCall, result_value: serde_json::Value) -> UserContent {
    let tool_call_id = tool_call
        .call_id
        .clone()
        .unwrap_or_else(|| tool_call.id.clone());
    let result_text = serde_json::to_string(&result_value).unwrap_or_default();
    UserContent::ToolResult(ToolResult {
        id: tool_call.id.clone(),
        call_id: Some(tool_call_id),
        content: OneOrMany::one(ToolResultContent::Text(Text { text: result_text })),
    })
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

pub(super) fn model_visible_tool_result(
    tool_name: &str,
    value: &serde_json::Value,
) -> serde_json::Value {
    match tool_name {
        "route_probe_paths" => compact_route_probe_result(value),
        "list_enumeration_web_roots" => compact_enumeration_web_roots_result(value),
        "enum_preflight_web_origins" => compact_enum_preflight_result(value),
        "browser_collect_js_api" => compact_browser_collect_result(value),
        "js_extract_apis" => compact_js_extract_result(value),
        "stage_worklist_status" | "stage_worklist_next" | "check_stage_asset_coverage" => {
            compact_stage_preflight_result(value)
        }
        _ => compact_large_json_result(value),
    }
}

const MAX_ENUM_PREFLIGHT_MODEL_ORIGINS: usize = 50;

pub(super) fn compact_enum_preflight_result(value: &serde_json::Value) -> serde_json::Value {
    const SCALAR_FIELDS: &[&str] = &[
        "status",
        "input_count",
        "reachable_count",
        "blocked_count",
        "incomplete_count",
        "fixed_concurrency",
        "next_action",
    ];
    let mut output = serde_json::Map::new();
    for field in SCALAR_FIELDS {
        if let Some(value) = value.get(*field) {
            output.insert((*field).to_string(), value.clone());
        }
    }
    for field in ["reachable_origins", "blocked_origins", "pending_origins"] {
        let Some(origins) = value.get(field).and_then(serde_json::Value::as_array) else {
            continue;
        };
        output.insert(
            field.to_string(),
            serde_json::Value::Array(
                origins
                    .iter()
                    .take(MAX_ENUM_PREFLIGHT_MODEL_ORIGINS)
                    .map(|origin| {
                        serde_json::json!({
                            "target_id": origin
                                .get("target_id")
                                .cloned()
                                .unwrap_or(serde_json::Value::Null),
                            "target_url": origin
                                .get("target_url")
                                .cloned()
                                .unwrap_or(serde_json::Value::Null),
                        })
                    })
                    .collect(),
            ),
        );
        if origins.len() > MAX_ENUM_PREFLIGHT_MODEL_ORIGINS {
            output.insert(
                format!("model_visible_{field}_omitted"),
                serde_json::json!(origins.len() - MAX_ENUM_PREFLIGHT_MODEL_ORIGINS),
            );
        }
    }
    output.insert(
        "model_visible_compacted".to_string(),
        serde_json::json!(true),
    );
    output.insert(
        "raw_result_retained_in_transcript".to_string(),
        serde_json::json!(true),
    );
    serde_json::Value::Object(output)
}

fn compact_stage_preflight_result(value: &serde_json::Value) -> serde_json::Value {
    const TOP_LEVEL_FIELDS: &[&str] = &[
        "tool",
        "stage",
        "organization_id",
        "session_id",
        "limit",
        "prefer",
        "ready_to_submit",
        "coverage_denominator_missing",
        "summary",
        "cell_summary",
        "omitted_item_count",
        "omitted_gap_count",
        "root_limit",
        "root_count",
        "matching_root_count",
        "omitted_root_count",
        "worklist_contract",
        "worklist_semantics",
        "deliverable_contract",
        "next_tool",
        "next_action",
        "assets_omitted",
        "assets_omitted_count",
        "asset_detail_hint",
    ];
    let mut output = serde_json::Map::new();
    for field in TOP_LEVEL_FIELDS {
        if let Some(item) = value.get(*field) {
            output.insert((*field).to_string(), item.clone());
        }
    }

    let exact_origin_page = compact_enumeration_exact_origin_page(value);
    if let Some(page) = exact_origin_page.as_ref() {
        output.insert("exact_origin_page".to_string(), page.clone());
        output.insert(
            "exact_origin_page_contract".to_string(),
            serde_json::json!("Copy target_id + target_url from exact_origin_page exactly into enum_preflight_web_origins and the same-page producers. Do not reconstruct IDs or roots from older history."),
        );
    }

    if exact_origin_page.is_none() {
        if let Some(items) = value.get("items").and_then(|items| items.as_array()) {
            const MAX_ITEMS: usize = 200;
            output.insert(
                "items".to_string(),
                serde_json::Value::Array(
                    items
                        .iter()
                        .take(MAX_ITEMS)
                        .map(compact_stage_work_item)
                        .collect(),
                ),
            );
            output.insert("items_count".to_string(), serde_json::json!(items.len()));
            output.insert(
                "model_visible_items_omitted".to_string(),
                serde_json::json!(items.len().saturating_sub(MAX_ITEMS)),
            );
        }
    }
    if let Some(gaps) = value
        .get("gap_examples")
        .and_then(|examples| examples.as_array())
    {
        const MAX_GAPS: usize = 25;
        output.insert(
            "gap_examples".to_string(),
            serde_json::Value::Array(
                gaps.iter()
                    .take(MAX_GAPS)
                    .map(compact_stage_work_item)
                    .collect(),
            ),
        );
        output.insert(
            "model_visible_gap_examples_omitted".to_string(),
            serde_json::json!(gaps.len().saturating_sub(MAX_GAPS)),
        );
    }

    // This field is already bounded and fail-closed by golish-agent-kit (max
    // 200 entries, bounded note size). Keep it lossless: the Enumerator must
    // copy coverage_to_submit exactly into the unchanged submit gate.
    if let Some(preview) = value.get("terminal_exceptions_preview") {
        output.insert("terminal_exceptions_preview".to_string(), preview.clone());
    }
    output.insert(
        "model_visible_compacted".to_string(),
        serde_json::json!(true),
    );
    output.insert(
        "raw_result_retained_in_transcript".to_string(),
        serde_json::json!(true),
    );
    serde_json::Value::Object(output)
}

/// Collapse the Enumeration worklist's four asset-technique cells per root
/// into one lossless exact-origin page. A raw 200-cell page is large enough to
/// trigger total-history compaction, which used to retain only the first few
/// roots and tempted the model to reuse stale target IDs from older turns.
/// Fifty compact root records fit comfortably inside the per-result budget.
pub(super) fn compact_enumeration_exact_origin_page(
    value: &serde_json::Value,
) -> Option<serde_json::Value> {
    if value.get("stage").and_then(serde_json::Value::as_str) != Some("enumeration") {
        return None;
    }
    let items = value.get("items")?.as_array()?;
    if items.is_empty() {
        return Some(serde_json::Value::Array(Vec::new()));
    }

    let root_limit = value
        .get("root_limit")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(50)
        .clamp(1, 50) as usize;
    let mut indexes = BTreeMap::<(String, String), usize>::new();
    let mut roots = Vec::<serde_json::Value>::new();

    for item in items {
        let Some(target_id) = item
            .get("target_id")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
        else {
            continue;
        };
        let Some(target_url) = item
            .get("asset")
            .and_then(serde_json::Value::as_str)
            .or_else(|| item.get("root_url").and_then(serde_json::Value::as_str))
            .filter(|value| !value.trim().is_empty())
        else {
            continue;
        };
        let target_url = target_url.trim_end_matches('/').to_string();
        let key = (target_id.to_string(), target_url.clone());

        let index = if let Some(index) = indexes.get(&key).copied() {
            index
        } else {
            if roots.len() >= root_limit {
                continue;
            }
            let index = roots.len();
            indexes.insert(key, index);
            roots.push(serde_json::json!({
                "target_id": target_id,
                "target_url": target_url,
                "root_url": item.get("root_url").cloned().unwrap_or(serde_json::Value::Null),
                "base_url": item.get("base_url").cloned().unwrap_or(serde_json::Value::Null),
                "unfinished_techniques": [],
            }));
            index
        };

        let Some(technique) = item
            .get("technique")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
        else {
            continue;
        };
        let state = item
            .get("state")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("pending");
        let techniques = roots[index]
            .get_mut("unfinished_techniques")
            .and_then(serde_json::Value::as_array_mut)
            .expect("new exact-origin worklist rows always own an array");
        if !techniques.iter().any(|entry| {
            entry.get("technique").and_then(serde_json::Value::as_str) == Some(technique)
        }) {
            techniques.push(serde_json::json!({"technique": technique, "state": state}));
        }
    }

    Some(serde_json::Value::Array(roots))
}

fn compact_stage_work_item(item: &serde_json::Value) -> serde_json::Value {
    const FIELDS: &[&str] = &[
        "work_item_id",
        "target_id",
        "asset",
        "target_type",
        "technique",
        "label",
        "state",
        "source",
        "note",
        "details",
        "evidence_refs",
        "suggested_capabilities",
        "suggested_tools",
        "worklist_source",
        "enumeration_focus",
        "eas_focus",
        "root_url",
        "base_url",
        "scheme",
        "port",
        "origin_resolution",
    ];
    let Some(object) = item.as_object() else {
        return item.clone();
    };
    let mut compact = serde_json::Map::new();
    for field in FIELDS {
        if let Some(value) = object.get(*field) {
            compact.insert((*field).to_string(), value.clone());
        }
    }
    serde_json::Value::Object(compact)
}

fn compact_route_probe_result(value: &serde_json::Value) -> serde_json::Value {
    if value.get("batch").and_then(|v| v.as_bool()) == Some(true) {
        return compact_route_probe_batch_result(value);
    }
    compact_route_probe_single_result(value)
}

const MAX_ROUTE_PROBE_MODEL_BATCH_TARGETS: usize = 50;
const MAX_ROUTE_PROBE_MODEL_BATCH_BYTES: usize = 512 * 1024;

fn compact_route_probe_batch_result(value: &serde_json::Value) -> serde_json::Value {
    let results = value
        .get("results")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .take(MAX_ROUTE_PROBE_MODEL_BATCH_TARGETS)
                .map(|item| {
                    serde_json::json!({
                        "target_id": compact_route_probe_scalar(item.get("target_id")),
                        "base_url": compact_route_probe_scalar(item.get("base_url")),
                        "result": item
                            .get("result")
                            .map(compact_route_probe_single_result)
                            .unwrap_or(serde_json::Value::Null),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let result_count = value
        .get("results")
        .and_then(|v| v.as_array())
        .map(Vec::len)
        .unwrap_or_default();
    let errors = value
        .get("errors")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .take(MAX_ROUTE_PROBE_MODEL_BATCH_TARGETS)
                .map(compact_route_probe_batch_failure)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let skipped = value
        .get("skipped")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .take(MAX_ROUTE_PROBE_MODEL_BATCH_TARGETS)
                .map(compact_route_probe_batch_failure)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut compact = serde_json::json!({
        "batch": true,
        "status": compact_route_probe_scalar(value.get("status")),
        "timed_out": compact_route_probe_scalar(value.get("timed_out")),
        "count": compact_route_probe_scalar(value.get("count")),
        "processed": compact_route_probe_scalar(value.get("processed")),
        "succeeded": compact_route_probe_scalar(value.get("succeeded")),
        "terminal_completed": compact_route_probe_scalar(value.get("terminal_completed")),
        "incomplete_targets": compact_route_probe_scalar(value.get("incomplete_targets")),
        "failed": compact_route_probe_scalar(value.get("failed")),
        "batch_concurrency": compact_route_probe_scalar(value.get("batch_concurrency")),
        "per_target_max_runtime_ms": compact_route_probe_scalar(value.get("per_target_max_runtime_ms")),
        "per_target_timeout_targets": compact_route_probe_scalar(value.get("per_target_timeout_targets")),
        "per_target_request_limited_targets": compact_route_probe_scalar(value.get("per_target_request_limited_targets")),
        "dir_found_targets": compact_route_probe_scalar(value.get("dir_found_targets")),
        "elapsed_ms": compact_route_probe_scalar(value.get("elapsed_ms")),
        "max_runtime_ms": compact_route_probe_scalar(value.get("max_runtime_ms")),
        "batch_max_runtime_ms": compact_route_probe_scalar(value.get("batch_max_runtime_ms")),
        "per_target_result_max_bytes": compact_route_probe_scalar(value.get("per_target_result_max_bytes")),
        "serialized_size_limit_bytes": compact_route_probe_scalar(value.get("serialized_size_limit_bytes")),
        "detail_contract": compact_route_probe_scalar(value.get("detail_contract")),
        "results_count": result_count,
        "results": results,
        "model_visible_results_omitted": result_count.saturating_sub(MAX_ROUTE_PROBE_MODEL_BATCH_TARGETS),
        "skipped_count": value.get("skipped").and_then(|v| v.as_array()).map(Vec::len).unwrap_or_default(),
        "skipped": skipped,
        "errors_count": value.get("errors").and_then(|v| v.as_array()).map(Vec::len).unwrap_or_default(),
        "errors": errors,
        "next_action": "Refresh stage_worklist/check_stage_asset_coverage. Continue only roots whose compact result has retry.recommended=true and whose DIR cell remains pending/error. If retry.recommended=false, stop automatic retry and follow that root's recovery_action before any new attempt.",
        "model_visible_compacted": true,
        "raw_result_retained_in_transcript": true,
        "omitted_large_fields": ["results.result.matches", "results.result.rejected_candidates", "results.result.errors", "results.result.prefixes_tested"],
        "model_visible_size_limit_bytes": MAX_ROUTE_PROBE_MODEL_BATCH_BYTES,
    });
    prune_route_probe_model_batch_to_limit(&mut compact);
    debug_assert!(
        serde_json::to_vec(&compact)
            .map(|encoded| encoded.len() <= MAX_ROUTE_PROBE_MODEL_BATCH_BYTES)
            .unwrap_or(false),
        "model-visible route batch summary exceeded its byte budget"
    );
    compact
}

fn compact_route_probe_single_result(value: &serde_json::Value) -> serde_json::Value {
    let matches_source = value.get("matches_sample").or_else(|| value.get("matches"));
    let rejected_source = value
        .get("rejected_candidates_sample")
        .or_else(|| value.get("rejected_candidates"));
    let errors_source = value.get("errors_sample").or_else(|| value.get("errors"));
    let persistence_errors_source = value
        .get("persistence_errors_sample")
        .or_else(|| value.get("persistence_errors"));
    let prefixes_source = value
        .get("prefixes_tested_sample")
        .or_else(|| value.get("prefixes_tested"));
    let matches = route_probe_detail_count(value, "matches_detail_count", "matches");
    let rejected = route_probe_detail_count(
        value,
        "rejected_candidates_detail_count",
        "rejected_candidates",
    );
    let errors = route_probe_detail_count(value, "errors_detail_count", "errors");
    let persistence_errors = route_probe_detail_count(
        value,
        "persistence_errors_detail_count",
        "persistence_errors",
    );
    let prefixes =
        route_probe_detail_count(value, "prefixes_tested_detail_count", "prefixes_tested");
    let outcome = value
        .get("outcome")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let queue_completed = value
        .get("queue_completed")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let retry = value.get("retry").map_or_else(
        || {
            let recommended = value
                .get("automatic_retry_allowed")
                .and_then(|item| item.as_bool())
                .unwrap_or(outcome == "error" || outcome == "partial" || !queue_completed);
            serde_json::json!({
                "recommended": recommended,
                "reason_codes": route_probe_retry_reason_codes(value),
                "checkpoint_available": value
                    .get("checkpoint_persisted")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                "queue_remaining": compact_route_probe_scalar(value.get("queue_remaining")),
            })
        },
        |retry| {
            let reason_codes = retry
                .get("reason_codes")
                .and_then(|value| value.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str())
                        .map(|reason| serde_json::json!(truncate_str(reason, 128)))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            serde_json::json!({
                "recommended": compact_route_probe_scalar(retry.get("recommended")),
                "reason_codes": reason_codes,
                "checkpoint_available": compact_route_probe_scalar(retry.get("checkpoint_available")),
                "queue_remaining": compact_route_probe_scalar(retry.get("queue_remaining")),
            })
        },
    );
    let retry_recommended = retry
        .get("recommended")
        .and_then(|item| item.as_bool())
        .unwrap_or(false);
    let manual_repair_reason = value
        .get("manual_repair_reason")
        .and_then(|item| item.as_str());
    let recovery_action = value.get("recovery_action").and_then(|item| item.as_str());
    let next_action = if !retry_recommended {
        if let (Some(reason), Some(action)) = (manual_repair_reason, recovery_action) {
            format!("Stop automatic retry. Reason: {reason}. Recovery action: {action}")
        } else if let Some(action) = recovery_action {
            format!("Stop automatic retry. Recovery action: {action}")
        } else if let Some(reason) = manual_repair_reason {
            format!("Stop automatic retry. Reason: {reason}")
        } else if value
            .get("attempt_superseded")
            .and_then(|item| item.as_bool())
            .unwrap_or(false)
        {
            "Stop retrying this attempt; a newer route attempt is authoritative. Refresh stage_worklist/check_stage_asset_coverage.".to_string()
        } else if queue_completed && matches!(outcome, "found" | "empty" | "blocked") {
            "Refresh stage_worklist/check_stage_asset_coverage; do not rerun this root unless coverage still reports DIR pending/error.".to_string()
        } else {
            "Refresh stage_worklist/check_stage_asset_coverage; do not rerun this root unless an explicit recovery action is required.".to_string()
        }
    } else if outcome == "error" {
        "Refresh stage_worklist/check_stage_asset_coverage, then retry only this failed root or mark blocked with the concrete error evidence.".to_string()
    } else if queue_completed {
        "Refresh stage_worklist/check_stage_asset_coverage; do not rerun this root unless coverage still reports DIR pending/error.".to_string()
    } else {
        "Queue did not complete; refresh coverage, then continue this root only if DIR remains pending/error.".to_string()
    };

    let mut compact = serde_json::Map::new();
    for (field, item) in [
        ("success", compact_route_probe_scalar(value.get("success"))),
        (
            "base_url",
            compact_route_probe_scalar(value.get("base_url")),
        ),
        (
            "requested_base_url",
            compact_route_probe_scalar(value.get("requested_base_url")),
        ),
        ("outcome", serde_json::Value::String(outcome.to_string())),
        (
            "attempted_outcome",
            compact_route_probe_scalar(value.get("attempted_outcome")),
        ),
        (
            "completion_state",
            compact_route_probe_scalar(value.get("completion_state")),
        ),
        (
            "outcome_persisted",
            compact_route_probe_scalar(value.get("outcome_persisted")),
        ),
        ("status", compact_route_probe_scalar(value.get("status"))),
        (
            "timed_out",
            compact_route_probe_scalar(value.get("timed_out")),
        ),
        (
            "request_limited",
            compact_route_probe_scalar(value.get("request_limited")),
        ),
        (
            "candidate_generation_limited",
            compact_route_probe_scalar(value.get("candidate_generation_limited")),
        ),
        (
            "recovery_exhausted",
            compact_route_probe_scalar(value.get("recovery_exhausted")),
        ),
        (
            "automatic_retry_allowed",
            compact_route_probe_scalar(value.get("automatic_retry_allowed")),
        ),
        ("queue_completed", serde_json::Value::Bool(queue_completed)),
        (
            "queue_remaining",
            compact_route_probe_scalar(value.get("queue_remaining")),
        ),
        (
            "max_requests",
            compact_route_probe_scalar(value.get("max_requests")),
        ),
        (
            "requests_sent",
            compact_route_probe_scalar(value.get("requests_sent")),
        ),
        (
            "invocation_requests_sent",
            compact_route_probe_scalar(value.get("invocation_requests_sent")),
        ),
        (
            "candidate_requests_sent",
            compact_route_probe_scalar(value.get("candidate_requests_sent")),
        ),
        (
            "baseline_requests_sent",
            compact_route_probe_scalar(value.get("baseline_requests_sent")),
        ),
        (
            "persisted_directory_entries",
            compact_route_probe_scalar(value.get("persisted_directory_entries")),
        ),
        (
            "matches_found",
            compact_route_probe_scalar(value.get("matches_found")),
        ),
        ("matches_count", serde_json::json!(matches)),
        ("matches_sample", sample_array(matches_source, 2)),
        (
            "rejected_count",
            compact_route_probe_scalar(value.get("rejected_count")),
        ),
        ("rejected_candidates_count", serde_json::json!(rejected)),
        (
            "rejected_candidates_sample",
            sample_array(rejected_source, 1),
        ),
        ("errors_count", serde_json::json!(errors)),
        ("errors_top", error_counts(errors_source, 5)),
        ("errors_sample", sample_array(errors_source, 3)),
        (
            "persistence_errors_count",
            serde_json::json!(persistence_errors),
        ),
        (
            "persistence_errors_sample",
            sample_array(persistence_errors_source, 2),
        ),
        ("prefixes_tested_count", serde_json::json!(prefixes)),
        ("prefixes_tested_sample", sample_array(prefixes_source, 5)),
        (
            "wordlist",
            value
                .get("wordlist")
                .map(|item| summarize_json_for_model(item, 2))
                .unwrap_or(serde_json::Value::Null),
        ),
        (
            "seed_paths",
            value
                .get("seed_paths_summary")
                .or_else(|| value.get("seed_paths"))
                .map(|item| summarize_json_for_model(item, 2))
                .unwrap_or(serde_json::Value::Null),
        ),
        ("run_id", compact_route_probe_scalar(value.get("run_id"))),
        ("dry_run", compact_route_probe_scalar(value.get("dry_run"))),
        (
            "checkpoint_resumed",
            compact_route_probe_scalar(value.get("checkpoint_resumed")),
        ),
        (
            "checkpoint_resume_count",
            compact_route_probe_scalar(value.get("checkpoint_resume_count")),
        ),
        (
            "checkpoint_persisted",
            compact_route_probe_scalar(value.get("checkpoint_persisted")),
        ),
        (
            "checkpoint_pending_candidates",
            compact_route_probe_scalar(value.get("checkpoint_pending_candidates")),
        ),
        (
            "checkpoint_pending_directory_writes",
            compact_route_probe_scalar(value.get("checkpoint_pending_directory_writes")),
        ),
        (
            "terminalization_pending",
            compact_route_probe_scalar(value.get("terminalization_pending")),
        ),
        (
            "checkpoint_write_rejected",
            compact_route_probe_scalar(value.get("checkpoint_write_rejected")),
        ),
        (
            "checkpoint_overflow",
            compact_route_probe_scalar(value.get("checkpoint_overflow")),
        ),
        (
            "checkpoint_error",
            compact_route_probe_scalar(value.get("checkpoint_error")),
        ),
        (
            "authorization_drift",
            value
                .get("authorization_drift")
                .map(|item| summarize_json_for_model(item, 1))
                .unwrap_or(serde_json::Value::Null),
        ),
        (
            "authorization_unavailable",
            value
                .get("authorization_unavailable")
                .map(|item| summarize_json_for_model(item, 1))
                .unwrap_or(serde_json::Value::Null),
        ),
        (
            "attempt_superseded",
            compact_route_probe_scalar(value.get("attempt_superseded")),
        ),
        (
            "persistence_recovery_exhausted",
            compact_route_probe_scalar(value.get("persistence_recovery_exhausted")),
        ),
        (
            "retry_exhausted_persistence",
            compact_route_probe_scalar(value.get("retry_exhausted_persistence")),
        ),
        (
            "terminal_publication_recovery_exhausted",
            compact_route_probe_scalar(value.get("terminal_publication_recovery_exhausted")),
        ),
        (
            "terminal_publication_total_failures",
            compact_route_probe_scalar(value.get("terminal_publication_total_failures")),
        ),
        (
            "terminal_publication_stable_failures",
            compact_route_probe_scalar(value.get("terminal_publication_stable_failures")),
        ),
        (
            "terminal_publication_last_failure_kind",
            compact_route_probe_scalar(value.get("terminal_publication_last_failure_kind")),
        ),
        (
            "terminal_publication_last_error_preview",
            compact_route_probe_scalar(value.get("terminal_publication_last_error_preview")),
        ),
        (
            "retry_exhausted_terminalization",
            compact_route_probe_scalar(value.get("retry_exhausted_terminalization")),
        ),
        (
            "manual_repair_reason",
            compact_route_probe_scalar(value.get("manual_repair_reason")),
        ),
        (
            "recovery_action",
            compact_route_probe_scalar(value.get("recovery_action")),
        ),
        ("retry", retry),
        (
            "detail_contract",
            compact_route_probe_scalar(value.get("detail_contract")),
        ),
        (
            "detail_omitted",
            compact_route_probe_scalar(value.get("detail_omitted")),
        ),
        (
            "next_action",
            serde_json::Value::String(next_action.to_string()),
        ),
        ("model_visible_compacted", serde_json::Value::Bool(true)),
        (
            "raw_result_retained_in_transcript",
            serde_json::Value::Bool(true),
        ),
        (
            "omitted_large_fields",
            serde_json::json!([
                "matches",
                "rejected_candidates",
                "errors",
                "prefixes_tested"
            ]),
        ),
    ] {
        compact.insert(field.to_string(), item);
    }
    serde_json::Value::Object(compact)
}

fn compact_route_probe_scalar(value: Option<&serde_json::Value>) -> serde_json::Value {
    value
        .map(|item| summarize_json_for_model(item, 0))
        .unwrap_or(serde_json::Value::Null)
}

fn route_probe_detail_count(
    value: &serde_json::Value,
    count_field: &str,
    detail_field: &str,
) -> usize {
    value
        .get(count_field)
        .and_then(|item| item.as_u64())
        .map(|count| count as usize)
        .or_else(|| {
            value
                .get(detail_field)
                .and_then(|item| item.as_array())
                .map(Vec::len)
        })
        .unwrap_or_default()
}

fn compact_route_probe_batch_failure(item: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "target_id": compact_route_probe_scalar(item.get("target_id")),
        "base_url": compact_route_probe_scalar(item.get("base_url")),
        "error": compact_route_probe_scalar(item.get("error")),
        "reason": compact_route_probe_scalar(item.get("reason")),
        "marker_persisted": compact_route_probe_scalar(item.get("marker_persisted")),
    })
}

fn route_probe_retry_reason_codes(value: &serde_json::Value) -> Vec<&'static str> {
    let mut reasons = Vec::new();
    if value.get("completion_state").and_then(|item| item.as_str()) != Some("complete") {
        reasons.push("completion_incomplete");
    }
    if value
        .get("recovery_exhausted")
        .and_then(|item| item.as_bool())
        .unwrap_or(false)
    {
        reasons.push("recovery_exhausted");
    }
    if value
        .get("baseline_budget_deferred")
        .and_then(|item| item.as_u64())
        .unwrap_or(0)
        > 0
    {
        reasons.push("baseline_budget_deferred");
    }
    for (field, reason) in [
        ("timed_out", "timed_out"),
        ("request_limited", "request_limited"),
        ("checkpoint_write_rejected", "checkpoint_write_rejected"),
        ("checkpoint_overflow", "checkpoint_overflow"),
        ("terminalization_pending", "terminalization_pending"),
        ("attempt_superseded", "attempt_superseded"),
        (
            "persistence_recovery_exhausted",
            "persistence_recovery_exhausted",
        ),
        (
            "terminal_publication_recovery_exhausted",
            "terminal_publication_recovery_exhausted",
        ),
        (
            "candidate_generation_limited",
            "candidate_generation_limited",
        ),
    ] {
        if value
            .get(field)
            .and_then(|item| item.as_bool())
            .unwrap_or(false)
        {
            reasons.push(reason);
        }
    }
    if !value
        .get("queue_completed")
        .and_then(|item| item.as_bool())
        .unwrap_or(false)
    {
        reasons.push("queue_incomplete");
    }
    if value
        .get("checkpoint_error")
        .is_some_and(|error| !error.is_null())
    {
        reasons.push("checkpoint_error");
    }
    for (field, reason) in [
        ("errors", "probe_errors"),
        ("persistence_errors", "persistence_errors"),
    ] {
        if value
            .get(field)
            .and_then(|items| items.as_array())
            .is_some_and(|items| !items.is_empty())
        {
            reasons.push(reason);
        }
    }
    if value
        .get("authorization_drift")
        .is_some_and(|error| !error.is_null())
    {
        reasons.push("authorization_drift");
    }
    if value
        .get("authorization_unavailable")
        .is_some_and(|error| !error.is_null())
    {
        reasons.push("authorization_unavailable");
    }
    if !value
        .get("dry_run")
        .and_then(|item| item.as_bool())
        .unwrap_or(false)
        && !value
            .get("outcome_persisted")
            .and_then(|item| item.as_bool())
            .unwrap_or(false)
    {
        reasons.push("outcome_not_persisted");
    }
    reasons
}

fn prune_route_probe_model_batch_to_limit(value: &mut serde_json::Value) {
    let encoded_len = |value: &serde_json::Value| {
        serde_json::to_vec(value)
            .map(|encoded| encoded.len())
            .unwrap_or(usize::MAX)
    };
    if encoded_len(value) <= MAX_ROUTE_PROBE_MODEL_BATCH_BYTES {
        return;
    }
    if let Some(results) = value
        .get_mut("results")
        .and_then(|items| items.as_array_mut())
    {
        for item in results {
            let Some(result) = item
                .get_mut("result")
                .and_then(|result| result.as_object_mut())
            else {
                continue;
            };
            for field in [
                "matches_sample",
                "rejected_candidates_sample",
                "errors_sample",
                "persistence_errors_sample",
                "prefixes_tested_sample",
                "seed_paths",
            ] {
                result.remove(field);
            }
            result.insert(
                "samples_pruned_to_size_limit".to_string(),
                serde_json::json!(true),
            );
        }
    }
    value["model_visible_samples_pruned"] = serde_json::json!(true);
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

fn compact_browser_root_diagnostic(
    value: &serde_json::Value,
    target_id: Option<&serde_json::Value>,
    target_url: Option<&serde_json::Value>,
    include_payload_samples: bool,
) -> serde_json::Value {
    let api_requests = value
        .get("api_requests")
        .or_else(|| value.get("requests"))
        .or_else(|| value.get("endpoints"));
    let scripts = value.get("scripts").or_else(|| value.get("script_urls"));
    let pages_visited_this_run_count = value
        .get("pages_visited_this_run")
        .and_then(|v| v.as_array())
        .map(Vec::len)
        .unwrap_or_default();
    let mut diagnostic = serde_json::json!({
        "success": value.get("success").cloned().unwrap_or(serde_json::Value::Null),
        "target_id": target_id.or_else(|| value.get("target_id")).cloned().unwrap_or(serde_json::Value::Null),
        "url": target_url.or_else(|| value.get("url")).or_else(|| value.get("target_url")).cloned().unwrap_or(serde_json::Value::Null),
        "status": value.get("status").cloned().unwrap_or(serde_json::Value::Null),
        "completion_state": value.get("completion_state").cloned().unwrap_or(serde_json::Value::Null),
        "closure_complete": value.get("closure_complete").cloned().unwrap_or(serde_json::Value::Null),
        "closure_incomplete_reasons": sample_array(value.get("closure_incomplete_reasons"), 12),
        "page_queue_remaining": value.get("page_queue_remaining").cloned().unwrap_or(serde_json::Value::Null),
        "page_resume_applied": value.get("page_resume_applied").cloned().unwrap_or(serde_json::Value::Null),
        "page_resume_count": value.get("page_resume_count").cloned().unwrap_or(serde_json::Value::Null),
        "page_resume_prior_visited": value.get("page_resume_prior_visited").cloned().unwrap_or(serde_json::Value::Null),
        "pages_visited_this_run_count": pages_visited_this_run_count,
        "js_outcome": value.get("js_outcome").cloned().unwrap_or(serde_json::Value::Null),
        "jsapi_outcome": value.get("jsapi_outcome").cloned().unwrap_or(serde_json::Value::Null),
        "param_outcome": value.get("param_outcome").cloned().unwrap_or(serde_json::Value::Null),
        "terminal_cross_origin_redirects_count": value.get("terminal_cross_origin_redirects").and_then(|v| v.as_array()).map(Vec::len).unwrap_or_default(),
        "terminal_cross_origin_redirects_sample": sample_array(value.get("terminal_cross_origin_redirects"), 3),
        "summary": value.get("summary").cloned().unwrap_or(serde_json::Value::Null),
        "api_requests_count": api_requests.and_then(|v| v.as_array()).map(Vec::len).unwrap_or_default(),
        "api_requests_sample": sample_array(api_requests, 10),
        "scripts_count": scripts.and_then(|v| v.as_array()).map(Vec::len).unwrap_or_default(),
        "scripts_sample": sample_array(scripts, 10),
        "persisted": value.get("persisted").cloned().unwrap_or(serde_json::Value::Null),
        "skipped": value.get("skipped").cloned().unwrap_or(serde_json::Value::Null),
        "ai_recipe_rounds": value.get("ai_recipe_rounds").cloned().unwrap_or(serde_json::Value::Null),
        "ai_recipe_rationale": value.get("ai_recipe_rationale").cloned().unwrap_or(serde_json::Value::Null),
    });
    if !include_payload_samples {
        if let Some(obj) = diagnostic.as_object_mut() {
            obj.remove("api_requests_sample");
            obj.remove("scripts_sample");
        }
    }
    diagnostic
}

fn compact_browser_collect_result(value: &serde_json::Value) -> serde_json::Value {
    if value.get("batch").and_then(|v| v.as_bool()) == Some(true) {
        let root_diagnostics = value
            .get("results")
            .and_then(|v| v.as_array())
            .map(|results| {
                results
                    .iter()
                    .take(50)
                    .filter_map(|entry| {
                        let result = entry.get("result")?;
                        Some(compact_browser_root_diagnostic(
                            result,
                            entry.get("target_id"),
                            entry.get("target_url"),
                            false,
                        ))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let errors = value
            .get("errors")
            .and_then(|v| v.as_array())
            .map(|errors| {
                errors
                    .iter()
                    .take(50)
                    .map(|entry| {
                        serde_json::json!({
                            "target_id": entry.get("target_id").cloned().unwrap_or(serde_json::Value::Null),
                            "target_url": entry.get("target_url").cloned().unwrap_or(serde_json::Value::Null),
                            "error": entry.get("error").cloned().unwrap_or(serde_json::Value::Null),
                            "outcome_marker": entry.get("outcome_marker").cloned().unwrap_or(serde_json::Value::Null),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        return serde_json::json!({
            "batch": true,
            "input_count": value.get("input_count").cloned().unwrap_or(serde_json::Value::Null),
            "accepted": value.get("accepted").cloned().unwrap_or(serde_json::Value::Null),
            "rejected": value.get("rejected").cloned().unwrap_or(serde_json::Value::Null),
            "truncated": value.get("truncated").cloned().unwrap_or(serde_json::Value::Null),
            "skipped": value.get("skipped").cloned().unwrap_or(serde_json::Value::Null),
            "succeeded": value.get("succeeded").cloned().unwrap_or(serde_json::Value::Null),
            "failed": value.get("failed").cloned().unwrap_or(serde_json::Value::Null),
            "root_diagnostics": root_diagnostics,
            "errors": errors,
            "omissions": sample_array(value.get("omissions"), 50),
            "next_action": "Re-run only root diagnostics whose completion_state is partial/error (page_queue_remaining checkpoints resume automatically under the same run/session/operation), then run js_extract_apis and refresh stage_worklist/check_stage_asset_coverage.",
            "model_visible_compacted": true,
            "raw_result_retained_in_transcript": true,
            "omitted_large_fields": ["results.result.scripts", "results.result.api_requests", "results.result.ai_dialogue"],
        });
    }

    let mut compact = compact_browser_root_diagnostic(value, None, None, true);
    if let Some(obj) = compact.as_object_mut() {
        obj.insert(
            "next_action".to_string(),
            serde_json::json!("Run js_extract_apis on collected JS if JS/API coverage remains pending; when completion_state is partial/error, rerun only this exact root so a same-run page checkpoint can resume. Then refresh stage_worklist/check_stage_asset_coverage."),
        );
        obj.insert(
            "model_visible_compacted".to_string(),
            serde_json::json!(true),
        );
        obj.insert(
            "raw_result_retained_in_transcript".to_string(),
            serde_json::json!(true),
        );
    }
    compact
}

fn compact_js_extract_root_diagnostic(
    value: &serde_json::Value,
    target_id: Option<&serde_json::Value>,
    target_url: Option<&serde_json::Value>,
    endpoint_sample_limit: usize,
) -> serde_json::Value {
    const COUNT_FIELDS: &[&str] = &[
        "target_lookup_candidates",
        "files_scanned",
        "files_skipped",
        "total_source_bytes",
        "endpoints_total",
        "endpoints_unique",
        "secrets_total",
        "configs_total",
        "frameworks_total",
        "libraries_total",
        "rule_matches_total",
        "hae_route_candidates_total",
        "hae_method_literal_candidates",
        "hae_direct_promoted",
        "hae_ai_promoted",
        "persisted_rows",
        "persisted_endpoint_rows",
        "duplicate_endpoint_rows",
        "unresolved_endpoint_rows",
        "param_hints_count",
        "param_endpoints",
        "outcome_persisted_count",
        "endpoints_detail_count",
        "secret_candidates_detail_count",
        "config_candidates_detail_count",
        "frameworks_detail_count",
        "libraries_detail_count",
        "rule_matches_detail_count",
        "hae_route_candidates_detail_count",
        "skipped_files_detail_count",
        "skipped_js_files_detail_count",
        "endpoint_persist_errors_detail_count",
        "js_analysis_persist_errors_detail_count",
        "capture_errors_detail_count",
        "outcome_prerequisite_errors_detail_count",
        "outcome_persist_errors_detail_count",
        "read_errors_detail_count",
        "ai_dialogue_count",
    ];
    let endpoint_source = value
        .get("endpoints_sample")
        .or_else(|| value.get("endpoints"))
        .or_else(|| value.get("endpoints_found"))
        .or_else(|| value.pointer("/summary/endpoints"));
    let endpoints_count = value
        .get("endpoints_total")
        .and_then(|v| v.as_u64())
        .or_else(|| value.get("endpoints_count").and_then(|v| v.as_u64()))
        .unwrap_or_else(|| {
            endpoint_source
                .and_then(|v| v.as_array())
                .map(|items| items.len() as u64)
                .unwrap_or_default()
        });
    let params = value.get("params").or_else(|| value.get("param_hints"));
    let params_count = value
        .get("param_endpoints")
        .and_then(|v| v.as_u64())
        .unwrap_or_else(|| {
            params
                .and_then(|v| v.as_array())
                .map(|items| items.len() as u64)
                .unwrap_or_default()
        });
    let mut diagnostic = serde_json::json!({
        "success": value.get("success").cloned().unwrap_or(serde_json::Value::Null),
        "target_id": target_id.or_else(|| value.get("target_id")).cloned().unwrap_or(serde_json::Value::Null),
        "url": target_url
            .or_else(|| value.get("effective_target_url"))
            .or_else(|| value.get("url"))
            .or_else(|| value.get("target_url"))
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        "status": value.get("status").cloned().unwrap_or(serde_json::Value::Null),
        "completion_state": value.get("completion_state").cloned().unwrap_or(serde_json::Value::Null),
        "jsapi_outcome": value.get("jsapi_outcome").cloned().unwrap_or(serde_json::Value::Null),
        "outcome_persisted": value.get("outcome_persisted").cloned().unwrap_or(serde_json::Value::Null),
        "param_outcome": value.get("param_outcome").cloned().unwrap_or(serde_json::Value::Null),
        "param_outcome_persisted": value.get("param_outcome_persisted").cloned().unwrap_or(serde_json::Value::Null),
        "authorization_drift": value.get("authorization_drift").cloned().unwrap_or(serde_json::Value::Null),
        "endpoints_count": endpoints_count,
        "endpoints_sample": sample_array(endpoint_source, endpoint_sample_limit),
        "params_count": params_count,
        "params_sample": sample_array(params, endpoint_sample_limit),
        "summary": value.get("summary").cloned().unwrap_or(serde_json::Value::Null),
        "retry": value.get("retry").cloned().unwrap_or(serde_json::Value::Null),
        "capture_manifest": value.get("capture_manifest").cloned().unwrap_or(serde_json::Value::Null),
        "detail_reference": value.get("detail_reference").cloned().unwrap_or(serde_json::Value::Null),
        "detail_contract": value.get("detail_contract").cloned().unwrap_or(serde_json::Value::Null),
        "persisted_api_endpoints": value.get("persisted_api_endpoints").cloned().unwrap_or(serde_json::Value::Null),
        "ai_used": value.pointer("/summary/ai_used").or_else(|| value.get("ai_used")).cloned().unwrap_or(serde_json::Value::Null),
        "ai_endpoints_added": value.pointer("/summary/ai_endpoints_added").or_else(|| value.get("ai_endpoints_added")).cloned().unwrap_or(serde_json::Value::Null),
    });
    if let Some(object) = diagnostic.as_object_mut() {
        for field in COUNT_FIELDS {
            if let Some(item) = value.get(*field) {
                object.insert((*field).to_string(), item.clone());
            }
        }
        let mut partial_diagnostics = serde_json::Map::new();
        for field in [
            "skipped_files_sample",
            "skipped_js_files_sample",
            "endpoint_persist_errors_sample",
            "js_analysis_persist_errors_sample",
            "capture_errors_sample",
            "outcome_prerequisite_errors_sample",
            "outcome_persist_errors_sample",
            "read_errors_sample",
        ] {
            if let Some(item) = value.get(field) {
                partial_diagnostics.insert(field.to_string(), item.clone());
            }
        }
        if !partial_diagnostics.is_empty() {
            object.insert(
                "partial_diagnostics".to_string(),
                serde_json::Value::Object(partial_diagnostics),
            );
        }
    }
    diagnostic
}

fn compact_js_extract_failure_root_diagnostic(value: &serde_json::Value) -> serde_json::Value {
    let marker = value.get("outcome_marker");
    let marker_field = |field: &str| marker.and_then(|item| item.get(field));
    serde_json::json!({
        "success": false,
        "target_id": value
            .get("target_id")
            .map(|item| summarize_json_for_model(item, 0))
            .unwrap_or(serde_json::Value::Null),
        "url": value
            .get("target_url")
            .map(|item| summarize_json_for_model(item, 0))
            .unwrap_or(serde_json::Value::Null),
        "status": "error",
        "completion_state": marker_field("completion_state").cloned().unwrap_or_else(|| serde_json::json!("error")),
        "jsapi_outcome": marker_field("jsapi_outcome").cloned().unwrap_or_else(|| serde_json::json!("error")),
        "outcome_persisted": marker_field("outcome_persisted")
            .or_else(|| marker_field("jsapi_outcome_persisted"))
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        "param_outcome": marker_field("param_outcome").cloned().unwrap_or_else(|| serde_json::json!("error")),
        "param_outcome_persisted": marker_field("param_outcome_persisted").cloned().unwrap_or(serde_json::Value::Null),
        "error": value
            .get("error")
            .map(|item| summarize_json_for_model(item, 0))
            .unwrap_or(serde_json::Value::Null),
        "outcome_marker": marker
            .map(|item| summarize_json_for_model(item, 2))
            .unwrap_or(serde_json::Value::Null),
        "retry": {
            "recommended": true,
            "reason_codes": ["tool_error"]
        }
    })
}

fn compact_js_extract_result(value: &serde_json::Value) -> serde_json::Value {
    if value.get("batch").and_then(|v| v.as_bool()) == Some(true) {
        let mut root_diagnostics = value
            .get("results")
            .and_then(|v| v.as_array())
            .map(|results| {
                results
                    .iter()
                    .take(50)
                    .filter_map(|entry| {
                        let result = entry.get("result")?;
                        Some(compact_js_extract_root_diagnostic(
                            result,
                            entry.get("target_id"),
                            entry.get("target_url"),
                            3,
                        ))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let remaining = 50usize.saturating_sub(root_diagnostics.len());
        if remaining > 0 {
            root_diagnostics.extend(
                value
                    .get("errors")
                    .and_then(|items| items.as_array())
                    .into_iter()
                    .flatten()
                    .take(remaining)
                    .map(compact_js_extract_failure_root_diagnostic),
            );
        }
        let results_count = value
            .get("results_detail_count")
            .and_then(|v| v.as_u64())
            .map(|count| count as usize)
            .or_else(|| {
                value
                    .get("results")
                    .and_then(|v| v.as_array())
                    .map(Vec::len)
            })
            .unwrap_or_default();
        return serde_json::json!({
            "batch": true,
            "input_count": value.get("input_count").cloned().unwrap_or(serde_json::Value::Null),
            "accepted": value.get("accepted").cloned().unwrap_or(serde_json::Value::Null),
            "rejected": value.get("rejected").cloned().unwrap_or(serde_json::Value::Null),
            "truncated": value.get("truncated").cloned().unwrap_or(serde_json::Value::Null),
            "skipped": value.get("skipped").cloned().unwrap_or(serde_json::Value::Null),
            "count": value.get("count").cloned().unwrap_or(serde_json::Value::Null),
            "succeeded": value.get("succeeded").cloned().unwrap_or(serde_json::Value::Null),
            "failed": value.get("failed").cloned().unwrap_or(serde_json::Value::Null),
            "jsapi_found_targets": value.get("jsapi_found_targets").cloned().unwrap_or(serde_json::Value::Null),
            "param_found_targets": value.get("param_found_targets").cloned().unwrap_or(serde_json::Value::Null),
            "result_contract": value.get("result_contract").cloned().unwrap_or(serde_json::Value::Null),
            "per_target_result_max_bytes": value.get("per_target_result_max_bytes").cloned().unwrap_or(serde_json::Value::Null),
            "serialized_size_limit_bytes": value.get("serialized_size_limit_bytes").cloned().unwrap_or(serde_json::Value::Null),
            "results_count": results_count,
            "root_diagnostics": root_diagnostics,
            "errors_count": value.get("errors_detail_count").and_then(|v| v.as_u64()).map(|count| count as usize).unwrap_or_else(|| value.get("errors").and_then(|v| v.as_array()).map(Vec::len).unwrap_or_default()),
            "errors_omitted": value.get("errors_omitted").cloned().unwrap_or(serde_json::Value::Null),
            "errors_sample": sample_array(value.get("errors"), 10),
            "omissions_count": value.get("omissions_detail_count").and_then(|v| v.as_u64()).map(|count| count as usize).unwrap_or_else(|| value.get("omissions").and_then(|v| v.as_array()).map(Vec::len).unwrap_or_default()),
            "omissions_omitted": value.get("omissions_omitted").cloned().unwrap_or(serde_json::Value::Null),
            "omissions_sample": sample_array(value.get("omissions"), 10),
            "next_action": "Trust per-root endpoints_count/outcomes above, then refresh stage_worklist/check_stage_asset_coverage. Retry only roots whose completion_state is partial/error or whose current JSAPI/PARAM cell remains incomplete.",
            "model_visible_compacted": true,
            "raw_result_retained_in_transcript": true,
            "transcript_result_contract": "bounded_batch_summary_v1",
            "omitted_large_fields": ["results.result.endpoints", "results.result.rule_matches", "results.result.hae_route_candidates", "results.result.ai_dialogue"],
        });
    }

    let mut compact = compact_js_extract_root_diagnostic(value, None, None, 12);
    if let Some(object) = compact.as_object_mut() {
        object.insert(
            "next_action".to_string(),
            serde_json::json!("Refresh stage_worklist/check_stage_asset_coverage; only retry this root if JS/API/PARAM remains pending or errored."),
        );
        object.insert(
            "model_visible_compacted".to_string(),
            serde_json::json!(true),
        );
        object.insert(
            "raw_result_retained_in_transcript".to_string(),
            serde_json::json!(true),
        );
    }
    compact
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
        eas_web_repair_targets: None,
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
            eas_web_repair_targets: None,
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
            eas_web_repair_targets: None,
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

fn submit_repair_update_after_tool_result(
    tool_name: &str,
    result: &serde_json::Value,
    success: bool,
    active_mode: Option<&SubmitRepairMode>,
) -> Option<SubmitRepairModeUpdate> {
    if let Some(update) = submit_repair_update(tool_name, result) {
        return Some(match (update, active_mode) {
            (SubmitRepairModeUpdate::Set(next_mode), Some(active_mode)) => {
                SubmitRepairModeUpdate::Set(retain_eas_web_repair_targets_for_same_gap(
                    next_mode,
                    active_mode,
                ))
            }
            (update, _) => update,
        });
    }
    if !success
        || !matches!(
            tool_name,
            "stage_worklist_next" | "check_stage_asset_coverage"
        )
    {
        return None;
    }
    refine_eas_web_repair_mode_from_worklist(active_mode?, result).map(SubmitRepairModeUpdate::Set)
}

/// Retain a DB-backed EAS WEB exact lock across a repeated `needs_fix` only
/// when the deterministic WEB gap identity is unchanged.
///
/// A changed action set must fail closed and force a fresh worklist read; this
/// prevents already-closed origins from remaining authorized after the gate
/// moves on to a different repair denominator.
pub fn retain_eas_web_repair_targets_for_same_gap(
    mut next_mode: SubmitRepairMode,
    active_mode: &SubmitRepairMode,
) -> SubmitRepairMode {
    if next_mode.kind != SubmitRepairKind::CoverageGap
        || active_mode.kind != SubmitRepairKind::CoverageGap
        || next_mode.eas_web_repair_targets.is_some()
        || active_mode.eas_web_repair_targets.is_none()
    {
        return next_mode;
    }

    let web_gap_assets = |mode: &SubmitRepairMode| {
        mode.coverage_gap_actions
            .iter()
            .filter(|action| action.technique == "GOLISH-EAS-WEB-FINGERPRINT")
            .map(|action| action.asset.trim().to_ascii_lowercase())
            .collect::<BTreeSet<_>>()
    };
    let next_assets = web_gap_assets(&next_mode);
    if !next_assets.is_empty() && next_assets == web_gap_assets(active_mode) {
        next_mode.eas_web_repair_targets = active_mode.eas_web_repair_targets.clone();
    }
    next_mode
}

/// Refine an active EAS WEB coverage-repair lock from DB-backed worklist truth.
///
/// Worklist and coverage responses are bounded projections. A non-empty exact
/// target set is authoritative for the next repair action, but an empty page is
/// not proof that the denominator is closed. Only an explicit
/// `ready_to_submit=true` response may replace the lock with an empty set.
pub fn refine_eas_web_repair_mode_from_worklist(
    active_mode: &SubmitRepairMode,
    result: &serde_json::Value,
) -> Option<SubmitRepairMode> {
    if active_mode.kind != SubmitRepairKind::CoverageGap
        || !active_mode
            .coverage_gap_actions
            .iter()
            .any(|action| action.technique == "GOLISH-EAS-WEB-FINGERPRINT")
    {
        return None;
    }

    let exact_targets = collect_db_backed_eas_web_repair_targets(result);
    if exact_targets.is_empty()
        && result
            .get("ready_to_submit")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
    {
        return None;
    }

    let mut refined = active_mode.clone();
    refined.eas_web_repair_targets = Some(exact_targets);
    Some(refined)
}

fn update_submit_repair_mode_in_batch(
    effective_mode: &mut Option<SubmitRepairMode>,
    observed_update: &mut Option<SubmitRepairModeUpdate>,
    update: Option<SubmitRepairModeUpdate>,
) {
    let Some(update) = update else {
        return;
    };
    match &update {
        SubmitRepairModeUpdate::Set(mode) => *effective_mode = Some(mode.clone()),
        SubmitRepairModeUpdate::Clear => *effective_mode = None,
    }
    *observed_update = Some(update);
}

fn collect_db_backed_eas_web_repair_targets(result: &serde_json::Value) -> Vec<EasWebRepairTarget> {
    let mut targets = Vec::new();
    let mut seen = HashSet::new();
    for collection in ["items", "gap_examples"] {
        let Some(items) = result.get(collection).and_then(serde_json::Value::as_array) else {
            continue;
        };
        for item in items {
            if item.get("technique").and_then(serde_json::Value::as_str)
                != Some("GOLISH-EAS-WEB-FINGERPRINT")
            {
                continue;
            }
            let Some(asset) = item
                .get("asset")
                .and_then(serde_json::Value::as_str)
                .map(normalize_probe_target)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let row_target_id = item
                .get("target_id")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let details = item.get("details").unwrap_or(&serde_json::Value::Null);
            let recommended = details
                .get("recommended_args")
                .and_then(|value| value.get("target_urls"))
                .and_then(serde_json::Value::as_array);
            if let Some(recommended) = recommended {
                for target in recommended {
                    let Some(target_id) = target
                        .get("target_id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    else {
                        continue;
                    };
                    if row_target_id.is_some_and(|row_target_id| row_target_id != target_id) {
                        continue;
                    }
                    let Some(target_url) = target
                        .get("target_url")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    else {
                        continue;
                    };
                    if normalize_probe_target(target_url) != asset {
                        continue;
                    }
                    let key = (target_id.to_string(), target_url.to_ascii_lowercase());
                    if seen.insert(key) {
                        targets.push(EasWebRepairTarget {
                            target_id: target_id.to_string(),
                            target_url: target_url.to_string(),
                        });
                    }
                }
                continue;
            }

            // Compatibility for a DB-backed worklist produced before
            // recommended_args was added: target_id + missing_origins still
            // carries the same deterministic identity.
            let Some(target_id) = row_target_id else {
                continue;
            };
            let Some(missing_origins) = details
                .get("missing_origins")
                .and_then(serde_json::Value::as_array)
            else {
                continue;
            };
            for target_url in missing_origins
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if normalize_probe_target(target_url) != asset {
                    continue;
                }
                let key = (target_id.to_string(), target_url.to_ascii_lowercase());
                if seen.insert(key) {
                    targets.push(EasWebRepairTarget {
                        target_id: target_id.to_string(),
                        target_url: target_url.to_string(),
                    });
                }
            }
        }
    }
    targets
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
    coverage_gap_action_instruction(actions)
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

/// Preserve the typed Stage Team child output across the generic sub-agent
/// barrier. Generic sub-agents still submit a string; durable stage children
/// submit an object that must reach the scheduler as one pure JSON object.
fn submit_result_barrier_response(args: &serde_json::Value) -> String {
    match args.get("result") {
        Some(serde_json::Value::String(result)) if !result.is_empty() => result.clone(),
        Some(result @ serde_json::Value::Object(_)) => {
            serde_json::to_string(result).unwrap_or_default()
        }
        _ => args
            .get("summary")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string(),
    }
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
async fn begin_bound_worker_tool(
    bound: Option<&crate::executor_types::BoundWorkerChainContext>,
    request_id: &str,
    tool_name: &str,
    args: &serde_json::Value,
) -> anyhow::Result<Option<uuid::Uuid>> {
    let Some(bound) = bound else {
        return Ok(None);
    };
    if bound.lease_is_lost() {
        anyhow::bail!("worker lease was lost before the next tool")
    }
    let Some(lifecycle) = bound.tool_lifecycle.as_ref() else {
        bound.mark_lease_lost();
        anyhow::bail!("prebound V2 worker has no durable tool lifecycle backend")
    };
    // The concrete lifecycle owns typed error classification because this
    // generic layer cannot distinguish a rejected lease fence from a
    // pre-dispatch storage failure. It must update the shared bound lease flag
    // itself before returning an actual lease-loss error.
    lifecycle.begin(request_id, tool_name, args).await.map(Some)
}

fn bound_worker_lifecycle_request_id(event_request_id: &str) -> &str {
    event_request_id
}

async fn finish_bound_worker_tool(
    bound: Option<&crate::executor_types::BoundWorkerChainContext>,
    tool_call_record_id: Option<uuid::Uuid>,
    success: bool,
    result: &serde_json::Value,
) -> anyhow::Result<()> {
    let (Some(bound), Some(tool_call_record_id)) = (bound, tool_call_record_id) else {
        return Ok(());
    };
    let lifecycle = bound
        .tool_lifecycle
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("bound worker tool lifecycle disappeared"))?;
    lifecycle
        .finish(tool_call_record_id, success, result)
        .await
        .inspect_err(|_error| {
            bound.mark_lease_lost();
        })
}

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
    use super::tool_setup::is_closed_candidate_analysis_role;

    let agent_id = &agent_def.id;
    let mut tool_results: Vec<UserContent> = vec![];
    let mut barrier_hit = false;
    let mut barrier_response: Option<String> = None;
    // Last `submit_stage_deliverable` BLOCK signature seen this batch (for the
    // loop's stage-stall circuit breaker). Last write wins (one submit/turn).
    let mut last_block_sig: Option<String> = None;
    let mut submit_repair_update_seen: Option<SubmitRepairModeUpdate> = None;
    let mut effective_submit_repair_mode = submit_repair_mode.cloned();
    let mut hard_supervisor_active = false;

    for tool_call in tool_calls {
        let tool_name = &tool_call.function.name;
        let closed_candidate_forbidden =
            is_closed_candidate_analysis_role(agent_id) && tool_name.as_str() != BARRIER_TOOL_NAME;
        if ctx
            .bound_worker_chain
            .as_ref()
            .is_some_and(|bound| bound.lease_is_lost())
        {
            hard_supervisor_active = true;
        }
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
        if hard_supervisor_active || barrier_hit || closed_candidate_forbidden {
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

            let result_value = if closed_candidate_forbidden {
                serde_json::json!({
                    "error": "Closed Candidate analysis roles may call only submit_result; the requested tool was not executed.",
                    "code": "CANDIDATE_ANALYSIS_TOOL_FORBIDDEN",
                    "allowed_tools": [BARRIER_TOOL_NAME],
                })
            } else if barrier_hit {
                serde_json::json!({
                    "error": "Skipped without execution because submit_result is a terminal barrier for this tool batch.",
                    "blocked_by_result_barrier": true,
                })
            } else {
                serde_json::json!({
                    "error": "Skipped without execution because a hard execution supervisor correction was injected earlier in this tool batch. Start a new turn, read the supervisor correction, and choose the next action only after satisfying it.",
                    "blocked_by_hard_supervisor": true,
                })
            };
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

            tool_results.push(tool_result_for_history(&tool_call, result_value));
            last_activity.store(epoch_secs(), Ordering::Relaxed);
            continue;
        }

        // ── Barrier tool ────────────────────────────────────────────────
        if tool_name == BARRIER_TOOL_NAME {
            if let Some(result_value) = stage_team_controller_submit_result_rejection(
                ctx.bound_worker_chain
                    .as_ref()
                    .is_some_and(|bound| bound.stage_team_leader.is_some()),
            ) {
                tracing::warn!(
                    target: "harness::stage_team_controller",
                    agent_id = %agent_id,
                    "rejected generic submit_result barrier from an active Company Controller"
                );
                let result_event = AiEvent::SubAgentToolResult {
                    agent_id: agent_id.to_string(),
                    tool_name: BARRIER_TOOL_NAME.to_string(),
                    success: false,
                    result: result_value.clone(),
                    request_id: tool_call.id.clone(),
                    parent_request_id: parent_request_id.to_string(),
                };
                let _ = ctx.event_tx.send(result_event.clone());
                if let Some(ref writer) = transcript_writer {
                    let writer = Arc::clone(writer);
                    tokio::spawn(async move {
                        if let Err(e) = writer.append(&result_event).await {
                            tracing::warn!(
                                "Failed to write rejected Controller barrier to transcript: {}",
                                e
                            );
                        }
                    });
                }
                tool_results.push(tool_result_for_history(&tool_call, result_value));
                last_activity.store(epoch_secs(), Ordering::Relaxed);
                continue;
            }
            let args = &tool_call.function.arguments;
            let result_text = submit_result_barrier_response(args);
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

            let result_value = serde_json::json!({ "status": "result submitted" });
            let result_event = AiEvent::SubAgentToolResult {
                agent_id: agent_id.to_string(),
                tool_name: BARRIER_TOOL_NAME.to_string(),
                success: true,
                result: result_value.clone(),
                request_id: tool_call.id.clone(),
                parent_request_id: parent_request_id.to_string(),
            };
            let _ = ctx.event_tx.send(result_event.clone());
            if let Some(ref writer) = transcript_writer {
                let writer = Arc::clone(writer);
                tokio::spawn(async move {
                    if let Err(e) = writer.append(&result_event).await {
                        tracing::warn!("Failed to write barrier result to transcript: {}", e);
                    }
                });
            }
            tool_results.push(tool_result_for_history(&tool_call, result_value));

            barrier_hit = true;
            continue;
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

            let delegate_result = if ctx.bound_worker_chain.is_some() {
                serde_json::json!({
                    "success": false,
                    "error": "nested delegation is disabled for a prebound V2 stage worker; the exact worker lease may have only one executor",
                    "code": "BOUND_WORKER_NESTED_DELEGATION_BLOCKED",
                })
            } else if let Some(registry) = ctx.sub_agent_registry {
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
                        operation_id: ctx.operation_id,
                        session_id: ctx.session_id,
                        persistence_session_id: ctx.persistence_session_id,
                        transcript_base_dir: ctx.transcript_base_dir,
                        api_request_stats: ctx.api_request_stats,
                        briefing: None,
                        temperature_override: delegate_def.temperature,
                        max_tokens_override: delegate_def.max_tokens,
                        top_p_override: delegate_def.top_p,
                        chain_persistence: ctx.chain_persistence,
                        bound_worker_chain: ctx.bound_worker_chain.clone(),
                        sub_agent_registry: ctx.sub_agent_registry,
                        post_shell_hook: ctx.post_shell_hook.clone(),
                        post_tool_result_hook: ctx.post_tool_result_hook.clone(),
                        tool_observer: ctx.tool_observer.clone(),
                        initial_submit_repair_mode: effective_submit_repair_mode.clone(),
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

        let candidate = ctx
            .bound_worker_chain
            .as_ref()
            .and_then(|bound| bound.candidate_attempt.as_ref());
        let candidate_submit_only = ctx
            .bound_worker_chain
            .as_ref()
            .is_some_and(|bound| bound.candidate_submit_only);
        let mut lifecycle_start_error = golish_core::check_candidate_tool_boundary_mode(
            candidate,
            candidate_submit_only,
            tool_name,
            &tool_args,
        )
        .err()
        .map(|error| {
            serde_json::json!({
                "error": error.to_string(),
                "code": error.code(),
            })
        });
        let lifecycle_record_id = if lifecycle_start_error.is_some() {
            None
        } else {
            match begin_bound_worker_tool(
                ctx.bound_worker_chain.as_ref(),
                bound_worker_lifecycle_request_id(&request_id),
                tool_name,
                &tool_args,
            )
            .await
            {
                Ok(record_id) => record_id,
                Err(error) => {
                    lifecycle_start_error = Some(serde_json::json!({
                        "error": format!("worker tool dispatch fence failed: {error}"),
                        "code": "WORKER_TOOL_FENCE_BEGIN_FAILED",
                    }));
                    None
                }
            }
        };

        let tool_timeout = idle_timeout.unwrap_or(tool_fallback_timeout);
        let use_outer_tool_timeout = use_sub_agent_outer_tool_timeout(tool_name);
        let mut stage_team_router_barrier_response = None;
        let wrapper_cancellation = matches!(
            tool_name.as_str(),
            "vuln_nuclei_general" | "vuln_nuclei_fingerprint_targeted"
        )
        .then(golish_core::AgentToolCancellation::default);
        let mut cancelled_after_wrapper_landing = false;
        let tool_result = if let Some(error) = lifecycle_start_error {
            Ok((error, false))
        } else {
            let tool_cancellation_scope = wrapper_cancellation.clone();
            let tool_execution = async {
                let tool_fut = async {
                    if let Some(blocked) = effective_submit_repair_mode
                        .as_ref()
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
                    } else {
                        let tool_context = golish_core::AgentToolContext {
                            request_id: request_id.clone(),
                            tool_call_record_id: lifecycle_record_id,
                            tool_name: tool_name.to_string(),
                            source: ToolSource::SubAgent {
                                agent_id: agent_id.to_string(),
                                agent_name: agent_def.name.clone(),
                            },
                            operation_id: ctx
                                .bound_worker_chain
                                .as_ref()
                                .map(|bound| bound.operation_id)
                                .or(ctx.operation_id),
                            stage_execution_id: ctx
                                .bound_worker_chain
                                .as_ref()
                                .map(|bound| bound.stage_execution_id),
                            stage_run_unit_id: ctx
                                .bound_worker_chain
                                .as_ref()
                                .map(|bound| bound.worker_lease.stage_run_unit_id),
                            organization_id: ctx
                                .bound_worker_chain
                                .as_ref()
                                .map(|bound| bound.organization_id)
                                .or(ctx.active_org_id_override),
                            worker_lease: ctx
                                .bound_worker_chain
                                .as_ref()
                                .map(|bound| bound.worker_lease.clone()),
                            candidate_attempt: ctx
                                .bound_worker_chain
                                .as_ref()
                                .and_then(|bound| bound.candidate_attempt.clone()),
                        };
                        golish_core::with_agent_session(
                            ctx.session_id.map(str::to_string),
                            golish_core::with_agent_tool_context(
                                Some(tool_context),
                                golish_core::with_agent_tool_cancellation(
                                    tool_cancellation_scope,
                                    golish_core::with_agent_tool_output_sender(
                                        Some(ctx.event_tx.clone()),
                                        async {
                                            // Try the injected router first (security/graph tools that live
                                            // outside the ToolRegistry); fall through to the registry.
                                            let routed = match &ctx.sub_tool_router {
                                                Some(router) => {
                                                    router(tool_name.to_string(), tool_args.clone())
                                                        .await
                                                }
                                                None => None,
                                            };
                                            match routed {
                                                Some((value, success)) => {
                                                    if success {
                                                        stage_team_router_barrier_response =
                                                            stage_team_leader_router_barrier_response(
                                                                tool_name,
                                                                &value,
                                                                ctx.bound_worker_chain.as_ref().and_then(
                                                                    |bound| bound.stage_team_leader.as_ref(),
                                                                ),
                                                            );
                                                    }
                                                    (value, success)
                                                }
                                                None => {
                                                    let effective_tool_name =
                                                        registry_tool_name(tool_name);
                                                    match execute_registry_tool_with_active_org(
                                                        ctx,
                                                        effective_tool_name,
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
            };
            tokio::pin!(tool_execution);
            tokio::select! {
                _ = wait_for_cancelled(ctx.cancelled) => {
                    if let Some(cancellation) = wrapper_cancellation.as_ref() {
                        tracing::info!(
                            "[sub-agent:{}] cancellation handed to self-bounded tool '{}'; awaiting wrapper landing",
                            agent_id,
                            tool_name
                        );
                        cancellation.cancel();
                        cancelled_after_wrapper_landing = true;
                        tool_execution.await
                    } else {
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
                }
                result = &mut tool_execution => result,
            }
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

        let lifecycle_landing_ok =
            if ctx.bound_worker_chain.is_some() && lifecycle_record_id.is_none() {
                false
            } else if ctx.bound_worker_chain.is_some() {
                match finish_bound_worker_tool(
                    ctx.bound_worker_chain.as_ref(),
                    lifecycle_record_id,
                    success,
                    &result_value,
                )
                .await
                {
                    Ok(()) => true,
                    Err(error) => {
                        result_value = serde_json::json!({
                            "error": format!("worker tool result fence failed: {error}"),
                            "code": "WORKER_TOOL_FENCE_FINISH_FAILED",
                            "stale_result_rejected": true,
                        });
                        success = false;
                        false
                    }
                }
            } else {
                ctx.bound_worker_chain.is_none()
            };

        if lifecycle_landing_ok {
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
                tool_call_id: tool_call.id.clone(),
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
        let submit_repair_update = submit_repair_update_after_tool_result(
            tool_name,
            &result_value,
            success,
            effective_submit_repair_mode.as_ref(),
        );
        update_submit_repair_mode_in_batch(
            &mut effective_submit_repair_mode,
            &mut submit_repair_update_seen,
            submit_repair_update,
        );
        if let Some(response) = stage_submission_barrier_response(
            tool_name,
            &result_value,
            ctx.bound_worker_chain
                .as_ref()
                .is_some_and(|bound| bound.return_on_first_durable_stage_submission),
        ) {
            barrier_hit = true;
            barrier_response = Some(response);
            tracing::info!(
                target: "harness::submit_tool",
                agent_id = %agent_id,
                "durable stage deliverable ended the host-owned specialist loop"
            );
        }
        if success {
            if let Some(response) = stage_team_router_barrier_response.take() {
                barrier_hit = true;
                barrier_response = Some(response);
                tracing::info!(
                    target: "harness::stage_team_controller",
                    agent_id = %agent_id,
                    tool = %tool_name,
                    "trusted Company Controller control result returned to the outer scheduler"
                );
            }
        }
        if let Some(intent_id) = candidate_terminal_intent_persisted(tool_name, &result_value) {
            barrier_hit = true;
            barrier_response = Some(format!(
                "Candidate terminal intent persisted; no further external action is allowed. Returning control for the host checkpoint barrier.\n\n[terminal_intent_id: {intent_id}]"
            ));
            tracing::info!(
                target: "harness::candidate_terminal_intent",
                agent_id = %agent_id,
                terminal_intent_id = %intent_id,
                "persisted Candidate terminal intent ended the verifier loop"
            );
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
        if cancelled_after_wrapper_landing {
            return ToolDispatchResult {
                tool_results,
                barrier_hit,
                barrier_response,
                stage_block_signature: last_block_sig,
                submit_repair_update: submit_repair_update_seen,
                cancelled: true,
            };
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

fn registry_tool_name(tool_name: &str) -> &str {
    if tool_name == "run_command" {
        "run_pty_cmd"
    } else {
        tool_name
    }
}

fn use_sub_agent_outer_tool_timeout(tool_name: &str) -> bool {
    !matches!(
        tool_name,
        // These Rust direct/guarded bridge tools can legitimately run past a
        // sub-agent's LLM idle timeout on large targets. They either emit their
        // own progress or retain synchronous/typed completion authority plus shared cancellation.
        // In particular, the Nuclei wrappers must retain control through parse + DB landing.
        // `submit_stage_deliverable` also owns the event-driven background-job
        // reconciliation barrier, whose bounded wait may exceed the generic
        // per-tool timeout. Applying `tokio::time::timeout` here drops the wrapper future; a spawned
        // child may outlive it, while authorized landing/evidence can no longer
        // publish final DB truth. Generic shell/pentest commands return from their
        // bounded managed-process yield before this outer loop guard is relevant.
        "vuln_nuclei_general"
            | "submit_stage_deliverable"
            | "vuln_nuclei_fingerprint_targeted"
            | "browser_collect_js_api"
            | "js_extract_apis"
            | "route_probe_paths"
            | "enum_preflight_web_origins"
            | "eas_discover_ports"
            | "eas_probe_http_liveness"
            | "eas_fingerprint_services"
            | "eas_fingerprint_web_stack"
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
    if !matches!(
        tool_name,
        "manage_targets"
            | "manage_organizations"
            | "enum_crawl_same_origin_urls"
            | "enum_preflight_web_origins"
            | "browser_collect_js_api"
            | "js_extract_apis"
            | "route_probe_paths"
    ) {
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
        begin_bound_worker_tool, bound_worker_lifecycle_request_id,
        candidate_terminal_intent_persisted, execute_registry_tool_with_active_org,
        finish_bound_worker_tool, inject_harness_org_id_arg, model_visible_tool_result,
        refine_eas_web_repair_mode_from_worklist, registry_tool_name, registry_tool_outcome,
        stage_submission_barrier_response, stage_team_controller_submit_result_rejection,
        stage_team_leader_router_barrier_response, structured_storage_hook_payload,
        submit_coverage_gap_repair_mode_from_reasons, submit_needs_fix_runtime_correction,
        submit_repair_mode_from_submit_result, submit_repair_update,
        submit_repair_update_after_tool_result, submit_result_barrier_response,
        tool_result_for_history, update_submit_repair_mode_in_batch,
        use_sub_agent_outer_tool_timeout, SubmitRepairModeUpdate,
        MAX_ROUTE_PROBE_MODEL_BATCH_BYTES, MAX_ROUTE_PROBE_MODEL_BATCH_TARGETS,
    };

    #[test]
    fn sub_agent_shell_alias_routes_through_the_shared_registry_tool() {
        assert_eq!(registry_tool_name("run_pty_cmd"), "run_pty_cmd");
        assert_eq!(registry_tool_name("run_command"), "run_pty_cmd");
        assert_eq!(registry_tool_name("pentest_run"), "pentest_run");
    }

    #[test]
    fn submit_result_barrier_serializes_a_typed_result_object() {
        let result = serde_json::json!({
            "business_disposition": "found",
            "summary": "Surface evidence booked",
            "fact_refs": [],
            "evidence_ids": [41],
            "checked_empty_units": [],
            "blocker_code": null
        });
        let args = serde_json::json!({
            "result": result,
            "success": true,
            "summary": "Surface evidence booked"
        });

        let response = submit_result_barrier_response(&args);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&response).unwrap(),
            result
        );
    }

    #[test]
    fn bound_worker_lifecycle_uses_the_same_event_request_id_as_tool_context() {
        let provider_tool_id = "call_provider_submit_1";
        let event_request_id = "event-correlation-uuid";

        assert_eq!(
            bound_worker_lifecycle_request_id(event_request_id),
            event_request_id
        );
        assert_ne!(
            bound_worker_lifecycle_request_id(event_request_id),
            provider_tool_id
        );
    }
    use crate::{
        BoundWorkerChainContext, BoundWorkerToolLifecycle, StageTeamLeaderBinding, SubmitRepairKind,
    };
    use golish_core::Tool;
    use golish_tools::ToolRegistry;
    use rig::message::{ToolCall, ToolFunction, UserContent};
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicI64};
    use std::sync::{Arc, RwLock as StdRwLock};
    use tokio::sync::{mpsc, Mutex, RwLock};
    use uuid::Uuid;

    fn stage_team_leader_binding() -> StageTeamLeaderBinding {
        StageTeamLeaderBinding {
            stage_team_plan_id: Uuid::new_v4(),
            leader_work_item_id: Uuid::new_v4(),
            expected_dispatch_epoch: 3,
            expected_plan_row_version: 5,
            expected_work_item_row_version: 7,
        }
    }

    #[test]
    fn only_trusted_leader_router_statuses_transfer_control_to_the_scheduler() {
        let dispatch = serde_json::json!({
            "status": "dispatch_accepted",
            "accepted": 2,
        });
        let prepare_final = serde_json::json!({
            "status": "prepare_final",
            "request_epoch_closed": true,
        });
        let binding = stage_team_leader_binding();

        assert!(stage_team_leader_router_barrier_response(
            "stage_team_dispatch_workers",
            &dispatch,
            None,
        )
        .is_none());
        assert_eq!(
            stage_team_leader_router_barrier_response(
                "stage_team_dispatch_workers",
                &dispatch,
                Some(&binding),
            )
            .as_deref(),
            Some(r#"{"accepted":2,"status":"dispatch_accepted"}"#)
        );
        assert_eq!(
            stage_team_leader_router_barrier_response(
                "stage_team_prepare_final_submission",
                &prepare_final,
                Some(&binding),
            )
            .as_deref(),
            Some(r#"{"request_epoch_closed":true,"status":"prepare_final"}"#)
        );
        assert!(stage_team_leader_router_barrier_response(
            "stage_team_dispatch_workers",
            &serde_json::json!({"status": "rejected"}),
            Some(&binding),
        )
        .is_none());
        assert!(stage_team_leader_router_barrier_response(
            "query_target_data",
            &dispatch,
            Some(&binding),
        )
        .is_none());
        assert!(stage_team_leader_router_barrier_response(
            "update_plan",
            &serde_json::json!({"success":true,"plan":[]}),
            Some(&binding),
        )
        .is_none());
    }

    struct ObserveActiveOrgTool {
        active_org_id: Arc<RwLock<Option<Uuid>>>,
    }

    struct FailingWorkerLifecycle {
        fail_begin: bool,
        fail_finish: bool,
        record_id: Uuid,
    }

    #[async_trait::async_trait]
    impl BoundWorkerToolLifecycle for FailingWorkerLifecycle {
        async fn begin(
            &self,
            _request_id: &str,
            _tool_name: &str,
            _args: &serde_json::Value,
        ) -> anyhow::Result<Uuid> {
            if self.fail_begin {
                anyhow::bail!("synthetic lease loss before tool")
            }
            Ok(self.record_id)
        }

        async fn finish(
            &self,
            _tool_call_record_id: Uuid,
            _success: bool,
            _result: &serde_json::Value,
        ) -> anyhow::Result<()> {
            if self.fail_finish {
                anyhow::bail!("synthetic stale result landing")
            }
            Ok(())
        }
    }

    fn bound_worker_with_lifecycle(
        lifecycle: Arc<dyn BoundWorkerToolLifecycle>,
    ) -> BoundWorkerChainContext {
        BoundWorkerChainContext {
            operation_id: Uuid::new_v4(),
            stage_execution_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            worker_lease: golish_core::WorkerLeaseContext {
                worker_run_id: Uuid::new_v4(),
                stage_run_unit_id: Uuid::new_v4(),
                lease_token: Uuid::new_v4(),
                attempt_epoch: 1,
            },
            candidate_attempt: None,
            candidate_submit_only: false,
            return_on_first_durable_stage_submission: false,
            stage_team_leader: None,
            chain_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            agent_type: "recon".to_string(),
            runtime_memory_source: None,
            initial_chain: serde_json::json!([]),
            initial_prompt_already_checkpointed: false,
            checkpoint_version: Arc::new(AtomicI64::new(0)),
            checkpoint_body: Arc::new(StdRwLock::new(serde_json::json!([]))),
            lease_lost: Arc::new(AtomicBool::new(false)),
            mutation_lock: Arc::new(tokio::sync::Mutex::new(())),
            tool_lifecycle: Some(lifecycle),
        }
    }

    #[tokio::test]
    async fn begin_failure_without_typed_lease_loss_keeps_bound_landable() {
        let bound = bound_worker_with_lifecycle(Arc::new(FailingWorkerLifecycle {
            fail_begin: true,
            fail_finish: false,
            record_id: Uuid::new_v4(),
        }));

        assert!(begin_bound_worker_tool(
            Some(&bound),
            "request-storage-failure",
            "stage_worklist_next",
            &serde_json::json!({}),
        )
        .await
        .is_err());
        assert!(
            !bound.lease_is_lost(),
            "the concrete lifecycle owns typed lease-loss classification"
        );
    }

    #[tokio::test]
    async fn finish_failure_blocks_stale_landing() {
        let record_id = Uuid::new_v4();
        let stale_landing = bound_worker_with_lifecycle(Arc::new(FailingWorkerLifecycle {
            fail_begin: false,
            fail_finish: true,
            record_id,
        }));
        let started = begin_bound_worker_tool(
            Some(&stale_landing),
            "request-2",
            "unknown_side_effect_tool",
            &serde_json::json!({}),
        )
        .await
        .expect("durable begin succeeds");
        assert_eq!(started, Some(record_id));
        assert!(finish_bound_worker_tool(
            Some(&stale_landing),
            started,
            true,
            &serde_json::json!({"side_effect": "completed"}),
        )
        .await
        .is_err());
        assert!(stale_landing.lease_is_lost());
    }

    #[test]
    fn barrier_history_result_preserves_the_provider_call_id() {
        let call = ToolCall {
            id: "tool-id".to_string(),
            call_id: Some("call-provider-123".to_string()),
            function: ToolFunction {
                name: "submit_result".to_string(),
                arguments: serde_json::json!({"result": "done"}),
            },
            signature: None,
            additional_params: None,
        };

        let UserContent::ToolResult(result) =
            tool_result_for_history(&call, serde_json::json!({"status": "result submitted"}))
        else {
            panic!("expected tool result content");
        };
        assert_eq!(result.id, "tool-id");
        assert_eq!(result.call_id.as_deref(), Some("call-provider-123"));
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
            persistence_session_id: None,
            transcript_base_dir: None,
            api_request_stats: None,
            cancelled: None,
            briefing: None,
            temperature_override: None,
            max_tokens_override: None,
            top_p_override: None,
            chain_persistence: None,
            bound_worker_chain: None,
            sub_agent_registry: None,
            post_shell_hook: None,
            resume: None,
            sub_tool_router: None,
            active_org_id_source: Some(active_org_id.clone()),
            active_org_id_override: Some(child_org),
            operation_id: None,
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
            persistence_session_id: None,
            transcript_base_dir: None,
            api_request_stats: None,
            cancelled: None,
            briefing: None,
            temperature_override: None,
            max_tokens_override: None,
            top_p_override: None,
            chain_persistence: None,
            bound_worker_chain: None,
            sub_agent_registry: None,
            post_shell_hook: None,
            resume: None,
            sub_tool_router: None,
            active_org_id_source: Some(active_org_id.clone()),
            active_org_id_override: Some(child_org),
            operation_id: None,
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
    fn enumeration_tools_get_hidden_harness_org_arg() {
        let org_id = Uuid::new_v4();
        for tool_name in [
            "enum_crawl_same_origin_urls",
            "enum_preflight_web_origins",
            "browser_collect_js_api",
            "js_extract_apis",
            "route_probe_paths",
        ] {
            let mut args = serde_json::json!({"target_url": "https://app.example/"});
            assert!(
                inject_harness_org_id_arg(tool_name, &mut args, Some(org_id)),
                "{tool_name} must receive the per-org harness binding"
            );
            assert_eq!(args["__harness_org_id"], org_id.to_string());
        }
    }

    #[test]
    fn enum_preflight_compaction_preserves_stable_origin_partitions() {
        let value = serde_json::json!({
            "status": "complete",
            "input_count": 3,
            "reachable_count": 1,
            "blocked_count": 1,
            "incomplete_count": 1,
            "reachable_origins": [{"target_id": "a", "target_url": "https://a:443/"}],
            "blocked_origins": [{"target_id": "b", "target_url": "https://b:443/"}],
            "pending_origins": [{"target_id": "c", "target_url": "https://c:443/"}],
            "results": [{"large": "detail"}],
            "next_action": "continue"
        });
        let compact = model_visible_tool_result("enum_preflight_web_origins", &value);
        assert_eq!(compact["reachable_origins"], value["reachable_origins"]);
        assert_eq!(compact["blocked_origins"], value["blocked_origins"]);
        assert_eq!(compact["pending_origins"], value["pending_origins"]);
        assert!(compact.get("results").is_none());
        assert_eq!(compact["raw_result_retained_in_transcript"], true);
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
    fn route_probe_model_compactor_honors_finite_manual_recovery_contract() {
        let result = serde_json::json!({
            "success": true,
            "base_url": "https://example.test/",
            "outcome": "partial",
            "attempted_outcome": "partial",
            "completion_state": "partial",
            "outcome_persisted": true,
            "status": "incomplete_partial",
            "queue_completed": false,
            "queue_remaining": 1,
            "checkpoint_persisted": true,
            "checkpoint_pending_candidates": 0,
            "checkpoint_pending_directory_writes": 1,
            "automatic_retry_allowed": false,
            "persistence_recovery_exhausted": true,
            "retry_exhausted_persistence": false,
            "manual_repair_reason": "repair conflict, then retry with retry_exhausted_persistence=true",
            "recovery_action": "repair row conflict, then retry only this root",
            "persistence_errors": [{"error": "row conflict"}],
            "dry_run": false
        });

        let compact = model_visible_tool_result("route_probe_paths", &result);

        assert_eq!(compact["retry"]["recommended"], false);
        assert_eq!(compact["automatic_retry_allowed"], false);
        assert_eq!(compact["persistence_recovery_exhausted"], true);
        assert_eq!(compact["checkpoint_pending_directory_writes"], 1);
        assert_eq!(
            compact["recovery_action"],
            "repair row conflict, then retry only this root"
        );
        assert!(compact["retry"]["reason_codes"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("persistence_recovery_exhausted")));
        assert!(compact["next_action"]
            .as_str()
            .unwrap()
            .starts_with("Stop automatic retry."));
    }

    #[test]
    fn route_probe_terminal_breaker_remains_visible_and_not_recommended() {
        let result = serde_json::json!({
            "success": true,
            "base_url": "https://example.test/",
            "outcome": "partial",
            "attempted_outcome": "found",
            "completion_state": "partial",
            "outcome_persisted": false,
            "status": "incomplete_partial",
            "queue_completed": true,
            "queue_remaining": 0,
            "checkpoint_persisted": true,
            "terminalization_pending": true,
            "automatic_retry_allowed": false,
            "terminal_publication_recovery_exhausted": true,
            "terminal_publication_total_failures": 2,
            "terminal_publication_stable_failures": 2,
            "terminal_publication_last_failure_kind": "conditional_outcome_upsert_failed",
            "terminal_publication_last_error_preview": "database unavailable",
            "retry_exhausted_terminalization": false,
            "manual_repair_reason": "repair publication, then retry with retry_exhausted_terminalization=true",
            "recovery_action": "repair terminal publication and retry only this root",
            "dry_run": false
        });

        let compact = model_visible_tool_result("route_probe_paths", &result);

        assert_eq!(compact["retry"]["recommended"], false);
        assert_eq!(compact["terminalization_pending"], true);
        assert_eq!(compact["terminal_publication_recovery_exhausted"], true);
        assert_eq!(compact["terminal_publication_total_failures"], 2);
        assert_eq!(
            compact["terminal_publication_last_failure_kind"],
            "conditional_outcome_upsert_failed"
        );
        assert_eq!(
            compact["recovery_action"],
            "repair terminal publication and retry only this root"
        );
        assert!(compact["next_action"]
            .as_str()
            .unwrap()
            .contains("retry_exhausted_terminalization=true"));
    }

    #[test]
    fn route_probe_batch_model_visible_result_keeps_nested_counts() {
        let nested_result = serde_json::json!({
            "success": true,
            "base_url": "https://example.test/",
            "outcome": "found",
            "outcome_persisted": true,
            "status": "request_limited_partial",
            "timed_out": false,
            "request_limited": true,
            "candidate_generation_limited": false,
            "queue_completed": false,
            "queue_remaining": 13,
            "max_requests": 2000,
            "requests_sent": 2000,
            "candidate_requests_sent": 1999,
            "baseline_requests_sent": 1,
            "persisted_directory_entries": 1,
            "matches_found": 1,
            "checkpoint_resumed": true,
            "checkpoint_resume_count": 2,
            "checkpoint_persisted": true,
            "checkpoint_pending_candidates": 13,
            "matches": [{"url": "https://example.test/api", "status": 200}],
            "rejected_candidates": [],
            "errors": [],
            "prefixes_tested": ["/"],
            "wordlist": {"entries_loaded": 1882},
            "seed_paths": {"total_after_dedupe": 5},
            "run_id": "run-1",
            "dry_run": false
        });
        let result = serde_json::json!({
            "batch": true,
            "status": "incomplete_partial",
            "timed_out": false,
            "count": 1,
            "processed": 1,
            "succeeded": 1,
            "terminal_completed": 0,
            "incomplete_targets": 1,
            "failed": 0,
            "dir_found_targets": 0,
            "elapsed_ms": 39_557,
            "max_runtime_ms": 60_000,
            "serialized_size_limit_bytes": 512 * 1024,
            "results": [{
                "target_id": "11111111-1111-1111-1111-111111111111",
                "base_url": "https://example.test/",
                "result": nested_result
            }],
            "errors": [],
            "skipped": []
        });

        let compact = model_visible_tool_result("route_probe_paths", &result);
        let nested = &compact["results"][0]["result"];

        assert_eq!(compact["batch"], true);
        assert_eq!(compact["results_count"], 1);
        assert_eq!(compact["status"], "incomplete_partial");
        assert_eq!(compact["processed"], 1);
        assert_eq!(compact["terminal_completed"], 0);
        assert_eq!(compact["incomplete_targets"], 1);
        assert_eq!(
            compact["results"][0]["target_id"],
            "11111111-1111-1111-1111-111111111111"
        );
        assert_eq!(nested["matches_count"], 1);
        assert_eq!(nested["matches_found"], 1);
        assert_eq!(nested["request_limited"], true);
        assert_eq!(nested["max_requests"], 2000);
        assert_eq!(nested["queue_completed"], false);
        assert_eq!(nested["checkpoint_resumed"], true);
        assert_eq!(nested["checkpoint_persisted"], true);
        assert_eq!(nested["retry"]["recommended"], true);
        assert!(nested.get("matches").is_none());
    }

    #[test]
    fn route_probe_model_compactor_keeps_all_targets_below_transcript_limit() {
        let large = "x".repeat(100_000);
        let results = (0..MAX_ROUTE_PROBE_MODEL_BATCH_TARGETS)
            .map(|index| {
                serde_json::json!({
                    "target_id": format!("00000000-0000-0000-0000-{index:012}"),
                    "base_url": format!("https://host-{index}.example.test:443/"),
                    "result": {
                        "success": true,
                        "base_url": format!("https://host-{index}.example.test:443/"),
                        "status": "timeout_partial",
                        "completion_state": "partial",
                        "outcome": "partial",
                        "outcome_persisted": true,
                        "timed_out": true,
                        "queue_completed": false,
                        "queue_remaining": 25,
                        "checkpoint_resumed": true,
                        "checkpoint_resume_count": 2,
                        "checkpoint_persisted": true,
                        "checkpoint_pending_candidates": 25,
                        "matches_found": 5,
                        "rejected_count": 1_000,
                        "matches": vec![serde_json::json!({"url": large, "body": large}); 100],
                        "rejected_candidates": vec![serde_json::json!({"url": large, "reason": large}); 100],
                        "errors": vec![serde_json::json!({"error": large}); 100],
                        "prefixes_tested": vec![large.clone(); 100],
                    }
                })
            })
            .collect::<Vec<_>>();
        let raw = serde_json::json!({
            "batch": true,
            "status": "incomplete_partial",
            "count": MAX_ROUTE_PROBE_MODEL_BATCH_TARGETS,
            "processed": MAX_ROUTE_PROBE_MODEL_BATCH_TARGETS,
            "succeeded": MAX_ROUTE_PROBE_MODEL_BATCH_TARGETS,
            "terminal_completed": 0,
            "incomplete_targets": MAX_ROUTE_PROBE_MODEL_BATCH_TARGETS,
            "failed": 0,
            "results": results,
            "errors": [],
            "skipped": [],
        });

        let compact = model_visible_tool_result("route_probe_paths", &raw);
        let encoded = serde_json::to_vec(&compact).expect("model summary serializes");

        assert_eq!(
            compact["results"].as_array().unwrap().len(),
            MAX_ROUTE_PROBE_MODEL_BATCH_TARGETS
        );
        assert!(compact["results"].as_array().unwrap().iter().all(|item| {
            item.get("target_id").is_some()
                && item.pointer("/result/completion_state") == Some(&serde_json::json!("partial"))
                && item.pointer("/result/outcome_persisted") == Some(&serde_json::json!(true))
                && item.pointer("/result/checkpoint_persisted") == Some(&serde_json::json!(true))
        }));
        assert!(
            encoded.len() <= MAX_ROUTE_PROBE_MODEL_BATCH_BYTES,
            "{} > {} bytes",
            encoded.len(),
            MAX_ROUTE_PROBE_MODEL_BATCH_BYTES
        );
    }

    #[test]
    fn route_probe_retry_reason_codes_remain_a_complete_string_array() {
        let reason_codes = serde_json::json!([
            "completion_incomplete",
            "timed_out",
            "request_limited",
            "candidate_generation_limited",
            "queue_incomplete",
            "probe_errors",
            "persistence_errors",
            "checkpoint_error",
            "authorization_drift",
            "outcome_not_persisted"
        ]);
        let raw = serde_json::json!({
            "batch": true,
            "count": 1,
            "succeeded": 1,
            "failed": 0,
            "results": [{
                "target_id": "11111111-1111-1111-1111-111111111111",
                "base_url": "https://example.test:443/",
                "result": {
                    "detail_contract": "bounded_batch_summary_v1",
                    "status": "partial",
                    "completion_state": "partial",
                    "outcome": "partial",
                    "outcome_persisted": false,
                    "queue_completed": false,
                    "retry": {
                        "recommended": true,
                        "reason_codes": reason_codes,
                        "checkpoint_available": true,
                        "queue_remaining": 17
                    }
                }
            }],
            "errors": [],
            "skipped": []
        });

        let compact = model_visible_tool_result("route_probe_paths", &raw);

        assert_eq!(
            compact["results"][0]["result"]["retry"]["reason_codes"],
            reason_codes
        );
        assert!(compact["results"][0]["result"]["retry"]["reason_codes"]
            .as_array()
            .is_some());
    }

    #[test]
    fn browser_batch_model_visible_result_keeps_per_root_resume_diagnostics() {
        let result = serde_json::json!({
            "batch": true,
            "input_count": 2,
            "accepted": 2,
            "succeeded": 2,
            "failed": 0,
            "results": [
                {
                    "target_id": "11111111-1111-1111-1111-111111111111",
                    "target_url": "https://one.example:443/",
                    "result": {
                        "status": "closure_partial",
                        "completion_state": "partial",
                        "closure_complete": false,
                        "closure_incomplete_reasons": ["page_queue_remaining"],
                        "page_queue_remaining": 7,
                        "page_resume_applied": true,
                        "page_resume_count": 2,
                        "page_resume_prior_visited": 100,
                        "pages_visited_this_run": ["https://one.example:443/a"],
                        "js_outcome": "partial",
                        "jsapi_outcome": "partial",
                        "param_outcome": "partial",
                        "scripts": [{"url": "https://one.example:443/app.js", "body": "large"}]
                    }
                },
                {
                    "target_id": "22222222-2222-2222-2222-222222222222",
                    "target_url": "https://two.example:443/",
                    "result": {
                        "status": "ok",
                        "completion_state": "complete",
                        "closure_complete": true,
                        "closure_incomplete_reasons": [],
                        "page_queue_remaining": 0,
                        "page_resume_applied": false,
                        "page_resume_count": 0,
                        "pages_visited_this_run": ["https://two.example:443/"],
                        "js_outcome": "empty",
                        "jsapi_outcome": "empty",
                        "param_outcome": "empty"
                    }
                }
            ],
            "errors": [],
            "omissions": []
        });

        let compact = model_visible_tool_result("browser_collect_js_api", &result);
        let roots = compact["root_diagnostics"].as_array().unwrap();

        assert_eq!(roots.len(), 2);
        assert_eq!(
            roots[0]["target_id"],
            "11111111-1111-1111-1111-111111111111"
        );
        assert_eq!(roots[0]["completion_state"], "partial");
        assert_eq!(
            roots[0]["closure_incomplete_reasons"],
            serde_json::json!(["page_queue_remaining"])
        );
        assert_eq!(roots[0]["page_queue_remaining"], 7);
        assert_eq!(roots[0]["page_resume_applied"], true);
        assert_eq!(roots[0]["page_resume_count"], 2);
        assert_eq!(roots[0]["pages_visited_this_run_count"], 1);
        assert_eq!(roots[1]["completion_state"], "complete");
        assert!(roots[0].get("scripts").is_none());
        assert!(roots[0].get("scripts_sample").is_none());
        assert!(roots[0].get("api_requests_sample").is_none());
        assert!(compact.get("results").is_none());
    }

    #[test]
    fn js_extract_batch_model_visible_result_keeps_per_root_counts_and_outcomes() {
        let result = serde_json::json!({
            "batch": true,
            "input_count": 2,
            "accepted": 2,
            "succeeded": 2,
            "failed": 0,
            "jsapi_found_targets": 1,
            "param_found_targets": 1,
            "result_contract": "bounded_batch_summary_v1",
            "per_target_result_max_bytes": 8192,
            "results": [
                {
                    "target_id": "11111111-1111-1111-1111-111111111111",
                    "target_url": "https://one.example:443/",
                    "result": {
                        "detail_contract": "bounded_batch_summary_v1",
                        "status": "ok",
                        "completion_state": "complete",
                        "endpoints_total": 321,
                        "endpoints_unique": 300,
                        "rule_matches_total": 900,
                        "hae_route_candidates_total": 700,
                        "persisted_endpoint_rows": 300,
                        "jsapi_outcome": "found",
                        "outcome_persisted": true,
                        "param_endpoints": 22,
                        "param_outcome": "found",
                        "param_outcome_persisted": true,
                        "endpoints_sample": [{"method": "GET", "path": "/api/users"}],
                        "retry": {"recommended": false, "reason_codes": []}
                    }
                },
                {
                    "target_id": "22222222-2222-2222-2222-222222222222",
                    "target_url": "https://two.example:443/",
                    "result": {
                        "detail_contract": "bounded_batch_summary_v1",
                        "status": "partial",
                        "completion_state": "partial",
                        "endpoints_total": 0,
                        "endpoints_unique": 0,
                        "jsapi_outcome": "partial",
                        "outcome_persisted": true,
                        "param_endpoints": 0,
                        "param_outcome": "partial",
                        "param_outcome_persisted": true,
                        "retry": {"recommended": true, "reason_codes": ["read_errors"]},
                        "read_errors_detail_count": 1,
                        "read_errors_sample": ["one source could not be read"]
                    }
                }
            ],
            "errors": [],
            "omissions": []
        });

        let compact = model_visible_tool_result("js_extract_apis", &result);
        let roots = compact["root_diagnostics"].as_array().unwrap();

        assert_eq!(roots.len(), 2);
        assert_eq!(
            roots[0]["target_id"],
            "11111111-1111-1111-1111-111111111111"
        );
        assert_eq!(roots[0]["endpoints_count"], 321);
        assert_eq!(roots[0]["jsapi_outcome"], "found");
        assert_eq!(roots[0]["outcome_persisted"], true);
        assert_eq!(roots[0]["param_endpoints"], 22);
        assert_eq!(roots[1]["completion_state"], "partial");
        assert_eq!(roots[1]["retry"]["recommended"], true);
        assert_eq!(roots[1]["read_errors_detail_count"], 1);
        assert_eq!(
            roots[1]["partial_diagnostics"]["read_errors_sample"],
            serde_json::json!(["one source could not be read"])
        );
        assert_eq!(compact["result_contract"], "bounded_batch_summary_v1");
        assert_eq!(compact["per_target_result_max_bytes"], 8192);
        assert!(compact.get("results").is_none());
    }

    #[test]
    fn js_extract_batch_model_visible_result_keeps_all_fifty_failed_roots() {
        let errors = (0..50)
            .map(|index| {
                serde_json::json!({
                    "target_id": format!("00000000-0000-0000-0000-{index:012}"),
                    "target_url": format!("https://failed-{index}.example.test:443/"),
                    "error": format!("read failed for root {index}"),
                    "outcome_marker": {
                        "jsapi": true,
                        "param": true
                    }
                })
            })
            .collect::<Vec<_>>();
        let raw = serde_json::json!({
            "batch": true,
            "input_count": 50,
            "accepted": 50,
            "rejected": 0,
            "truncated": 0,
            "skipped": 0,
            "count": 50,
            "succeeded": 0,
            "failed": 50,
            "jsapi_found_targets": 0,
            "param_found_targets": 0,
            "result_contract": "bounded_batch_summary_v1",
            "per_target_result_max_bytes": 8192,
            "results": [],
            "errors": errors,
            "omissions": []
        });

        let compact = model_visible_tool_result("js_extract_apis", &raw);
        let roots = compact["root_diagnostics"].as_array().unwrap();

        assert_eq!(roots.len(), 50);
        assert_eq!(roots[0]["completion_state"], "error");
        assert_eq!(
            roots[49]["target_id"],
            "00000000-0000-0000-0000-000000000049"
        );
        assert_eq!(roots[49]["retry"]["recommended"], true);
        assert_eq!(roots[49]["outcome_marker"]["jsapi"], true);
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
    fn stage_preflight_compaction_preserves_exact_origin_page_and_submit_contract() {
        let coverage_to_submit = (0..32)
            .map(|index| {
                serde_json::json!({
                    "asset": format!("https://blocked-{index}.example.com:443"),
                    "technique": "GOLISH-ENUM-DIR",
                    "status": "blocked",
                    "note": format!("same-environment timeout for exact origin {index}")
                })
            })
            .collect::<Vec<_>>();
        let items = (0..200)
            .map(|index| {
                serde_json::json!({
                    "work_item_id": format!("item-{index}"),
                    "target_id": format!("target-{index}"),
                    "asset": format!("https://host-{index}.example.com:443"),
                    "technique": "GOLISH-ENUM-DIR",
                    "state": "pending",
                    "root_url": format!("https://host-{index}.example.com:443/"),
                    "padding_not_for_model": "x".repeat(500)
                })
            })
            .collect::<Vec<_>>();
        let result = serde_json::json!({
            "tool": "stage_worklist_next",
            "stage": "enumeration",
            "ready_to_submit": false,
            "root_limit": 50,
            "root_count": 50,
            "matching_root_count": 60,
            "omitted_root_count": 10,
            "items": items,
            "terminal_exceptions_preview": {
                "preview_only": true,
                "persisted": false,
                "accepted_cells": 32,
                "coverage_to_submit": coverage_to_submit,
                "contract": "copy unchanged"
            }
        });

        for tool_name in [
            "stage_worklist_status",
            "stage_worklist_next",
            "check_stage_asset_coverage",
        ] {
            let compact = model_visible_tool_result(tool_name, &result);
            assert_eq!(compact["model_visible_compacted"], true);
            assert_eq!(compact["raw_result_retained_in_transcript"], true);
            assert_eq!(compact["root_count"], 50);
            assert_eq!(compact["omitted_root_count"], 10);
            assert!(compact.get("items").is_none());
            assert_eq!(compact["exact_origin_page"].as_array().unwrap().len(), 50);
            assert_eq!(compact["exact_origin_page"][0]["target_id"], "target-0");
            assert_eq!(
                compact["exact_origin_page"][49]["target_url"],
                "https://host-49.example.com:443"
            );
            assert_eq!(
                compact["exact_origin_page"][49]["unfinished_techniques"][0]["technique"],
                "GOLISH-ENUM-DIR"
            );
            assert!(serde_json::to_vec(&compact).unwrap().len() < 32 * 1024);
            assert_eq!(
                compact["terminal_exceptions_preview"]["coverage_to_submit"]
                    .as_array()
                    .unwrap()
                    .len(),
                32
            );
            assert_eq!(
                compact["terminal_exceptions_preview"]["coverage_to_submit"],
                result["terminal_exceptions_preview"]["coverage_to_submit"]
            );
        }
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
    fn eas_wrapper_result_feeds_structured_storage_hook() {
        let payload = structured_storage_hook_payload(
            "eas_discover_ports",
            &serde_json::json!({"targets": ["192.0.2.10"]}),
            &serde_json::json!({
                "wrapped_tool_name": "naabu",
                "wrapped_args": "-list {input_file} -top-ports 1000 -s c -silent",
                "command": "naabu -list /tmp/targets -top-ports 1000 -s c -silent",
                "stdout": "192.0.2.10:80",
                "stderr": "",
                "exit_code": 0
            }),
            true,
        )
        .expect("EAS wrapper should produce structured-storage payload");

        assert_eq!(
            payload.command,
            "naabu -list /tmp/targets -top-ports 1000 -s c -silent"
        );
        assert_eq!(payload.stdout, "192.0.2.10:80");
    }

    #[test]
    fn self_landed_eas_wrapper_skips_generic_structured_storage_hook() {
        let payload = structured_storage_hook_payload(
            "eas_discover_ports",
            &serde_json::json!({"targets": ["192.0.2.10"]}),
            &serde_json::json!({
                "wrapped_tool_name": "naabu",
                "wrapped_args": "-list {input_file} -silent",
                "stdout": "192.0.2.10:80",
                "exit_code": 0,
                "structured_storage_disabled": true,
                "generic_evidence_disabled": true
            }),
            true,
        );
        assert!(payload.is_none());
    }

    #[test]
    fn enum_wrapper_result_feeds_structured_storage_hook() {
        let payload = structured_storage_hook_payload(
            "enum_crawl_same_origin_urls",
            &serde_json::json!({"target_urls": ["https://app.example.com/"]}),
            &serde_json::json!({
                "wrapped_tool_name": "katana",
                "wrapped_args": "-list {input_file} -jc -silent -d 2",
                "command": "katana -list /tmp/roots.txt -jc -silent -d 2",
                "stdout": "https://app.example.com/api/v1/users",
                "stderr": "",
                "exit_code": 0
            }),
            true,
        )
        .expect("Enumeration wrapper should produce structured-storage payload");

        assert_eq!(
            payload.command,
            "katana -list /tmp/roots.txt -jc -silent -d 2"
        );
        assert_eq!(payload.stdout, "https://app.example.com/api/v1/users");
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
        assert!(mode.block_result("list_recent_evidence").is_none());
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
    fn background_jobs_needs_fix_enters_one_shot_recovery_mode() {
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

        let blocked_wait = mode
            .block_result("wait_for_background_jobs")
            .expect("recovery mode blocks another model-authored wait loop");
        assert!(blocked_wait["error"]
            .as_str()
            .unwrap()
            .contains("check_job"));
        assert!(mode.block_result("submit_stage_deliverable").is_none());
        let blocked = mode
            .block_result("pentest_run")
            .expect("wait repair mode blocks replacement scans");
        assert!(blocked["error"].as_str().unwrap().contains("check_job"));
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
                    "suggested_tools": ["eas_probe_http_liveness"]
                },
                {
                    "asset": "101.69.134.7",
                    "technique": "GOLISH-EAS-SERVICE-FINGERPRINT",
                    "reason": "missing_terminal_coverage",
                    "suggested_tools": ["eas_fingerprint_services"]
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
        assert!(mode.model_instruction().contains("101.69.134.7"));
        assert!(mode.block_result("query_target_data").is_none());
        assert!(mode.block_result("check_stage_asset_coverage").is_none());
        assert!(mode.block_result("stage_worklist_status").is_none());
        assert!(mode.block_result("stage_worklist_next").is_none());
        assert!(mode.block_result("list_recent_evidence").is_none());
        assert!(mode.allows("eas_probe_http_liveness"));
        assert!(mode.allows("eas_fingerprint_services"));
        assert!(mode.allows("eas_fingerprint_web_stack"));
        assert!(mode
            .block_result_with_args(
                "eas_probe_http_liveness",
                &serde_json::json!({"targets": ["101.69.134.6"]})
            )
            .is_none());
        let raw_blocked = mode
            .block_result("pentest_run")
            .expect("EAS repair should use backend wrappers, not raw pentest_run");
        assert!(raw_blocked["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("EAS coverage-gap repair"));
        assert!(mode.block_result("submit_stage_deliverable").is_none());
        let blocked = mode
            .block_result("list_in_scope_targets")
            .expect("coverage repair should not restart full inventory");
        assert_eq!(blocked["repair_kind"], "coverage_gap");
        let blocked = mode
            .block_result("subfinder")
            .expect("unknown fresh discovery tool remains blocked");
        assert_eq!(blocked["repair_kind"], "coverage_gap");
        let projection = blocked["coverage_gap_actions"]
            .as_object()
            .expect("bounded action projection included in block payload");
        assert_eq!(projection.get("total"), Some(&serde_json::json!(2)));
        assert_eq!(projection.get("sample_count"), Some(&serde_json::json!(2)));
        assert_eq!(projection.get("omitted"), Some(&serde_json::json!(0)));
        assert_eq!(
            projection.get("next_page_tool"),
            Some(&serde_json::json!("stage_worklist_next"))
        );
        assert!(projection
            .get("stable_hash")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| !value.is_empty()));
        assert!(blocked["error"]
            .as_str()
            .unwrap()
            .contains("Targeted gap-closure"));
    }

    #[test]
    fn coverage_gap_repair_allows_eas_wrapper_batch_targets() {
        let v = serde_json::json!({
            "status": "needs_fix",
            "reasons": ["external attack surface incomplete: never attempted"],
            "coverage_gap_actions": [
                {
                    "asset": "112.65.238.93",
                    "technique": "GOLISH-EAS-LIVENESS",
                    "reason": "missing_terminal_coverage",
                    "suggested_tools": ["eas_probe_http_liveness"]
                },
                {
                    "asset": "113.105.78.22",
                    "technique": "GOLISH-EAS-LIVENESS",
                    "reason": "missing_terminal_coverage",
                    "suggested_tools": ["eas_probe_http_liveness"]
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
                "eas_probe_http_liveness",
                &serde_json::json!({
                    "targets": ["112.65.238.93", "113.105.78.22"]
                }),
            )
            .is_none());
        let blocked = mode
            .block_result_with_args(
                "pentest_run",
                &serde_json::json!({
                    "tool_name": "httpx",
                    "args": "-json -sc -silent -l {{input_file}}",
                    "input_lines": ["112.65.238.93", "113.105.78.22"]
                }),
            )
            .expect("EAS gap repair must not use raw pentest_run");
        assert!(blocked["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("EAS coverage-gap repair"));
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

        assert!(mode.block_result("stage_worklist_status").is_none());
        assert!(mode.block_result("stage_worklist_next").is_none());
        assert!(mode.block_result("list_enumeration_web_roots").is_none());
        assert!(mode.block_result("list_recent_evidence").is_none());
        assert!(mode.allows("browser_collect_js_api"));
        assert!(!mode.allows("js_collect"));
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
                "js_extract_apis",
                &serde_json::json!({
                    "target_urls": [{
                        "target_id": "11111111-1111-1111-1111-111111111111",
                        "target_url": "https://app.example.com/"
                    }]
                })
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
    fn coverage_gap_repair_uses_enum_crawler_wrapper_not_raw_pentest_run() {
        let v = serde_json::json!({
            "status": "needs_fix",
            "reasons": ["content enumeration incomplete: never attempted"],
            "coverage_gap_actions": [{
                "asset": "https://app.example.com",
                "technique": "GOLISH-ENUM-JSAPI",
                "reason": "missing_terminal_coverage",
                "suggested_tools": ["katana"]
            }]
        });
        let update = submit_repair_update("submit_stage_deliverable", &v)
            .expect("coverage gaps should activate repair mode");
        let SubmitRepairModeUpdate::Set(mode) = update else {
            panic!("expected Set repair mode");
        };

        assert!(mode.allows("enum_crawl_same_origin_urls"));
        assert!(mode
            .block_result_with_args(
                "enum_crawl_same_origin_urls",
                &serde_json::json!({"target_urls": ["https://app.example.com"]}),
            )
            .is_none());

        let blocked = mode
            .block_result_with_args(
                "pentest_run",
                &serde_json::json!({
                    "tool_name": "katana",
                    "args": "-list /tmp/all-targets.txt -jc -silent"
                }),
            )
            .expect("raw crawler CLI should be blocked");

        assert_eq!(blocked["blocked_by_submit_repair"], true);
        assert_eq!(blocked["repair_kind"], "coverage_gap");
        assert!(blocked["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("Enumeration coverage-gap repair"));
    }

    #[test]
    fn coverage_gap_repair_uses_exact_nuclei_wrappers_not_raw_pentest_run() {
        let v = serde_json::json!({
            "status": "needs_fix",
            "reasons": ["vuln_triage incomplete: never attempted"],
            "coverage_gap_actions": [
                {
                    "asset": "https://app.example.com",
                    "technique": "WSTG-INPV-05",
                    "reason": "missing_terminal_coverage",
                    "suggested_tools": ["vuln_nuclei_general"]
                },
                {
                    "asset": "https://other.example.com",
                    "technique": "GOLISH-NDAY",
                    "reason": "missing_terminal_coverage",
                    "suggested_tools": ["vuln_nuclei_fingerprint_targeted"]
                },
                {
                    "asset": "https://api.example.com",
                    "technique": "WSTG-ATHN-04",
                    "reason": "missing_terminal_coverage",
                    "suggested_tools": ["vuln_probe_anonymous_access"]
                }
            ]
        });
        let update = submit_repair_update("submit_stage_deliverable", &v)
            .expect("coverage gaps should activate repair mode");
        let SubmitRepairModeUpdate::Set(mode) = update else {
            panic!("expected Set repair mode");
        };

        assert!(mode.allows("vuln_nuclei_general"));
        assert!(mode.allows("vuln_nuclei_fingerprint_targeted"));
        assert!(mode.allows("vuln_probe_anonymous_access"));
        assert!(!mode.allows("pentest_run"));
        assert!(mode
            .block_result_with_args(
                "vuln_nuclei_general",
                &serde_json::json!({
                    "target_id": "11111111-1111-1111-1111-111111111111",
                    "target_url": "https://app.example.com",
                    "techniques": ["WSTG-INPV-05"]
                }),
            )
            .is_none());
        assert!(mode
            .block_result_with_args(
                "vuln_nuclei_fingerprint_targeted",
                &serde_json::json!({
                    "target_id": "22222222-2222-2222-2222-222222222222",
                    "target_url": "https://other.example.com",
                    "techniques": ["GOLISH-NDAY"]
                }),
            )
            .is_none());
        assert!(mode
            .block_result_with_args(
                "vuln_probe_anonymous_access",
                &serde_json::json!({
                    "target_id": "33333333-3333-3333-3333-333333333333",
                    "target_url": "https://api.example.com",
                    "reviewed_endpoint_ids": [
                        "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                        "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
                    ],
                    "selected_probes": [{
                        "endpoint_id": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                        "query_values": {"account_id": "acct_01-safe"},
                        "rationale": "Account endpoint is likely to expose authenticated data."
                    }],
                    "timeout_secs": 30
                }),
            )
            .is_none());

        let caller_authored_request = mode
            .block_result_with_args(
                "vuln_probe_anonymous_access",
                &serde_json::json!({
                    "target_id": "33333333-3333-3333-3333-333333333333",
                    "target_url": "https://api.example.com",
                    "reviewed_endpoint_ids": [
                        "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
                    ],
                    "selected_probes": [{
                        "endpoint_id": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                        "query_values": {},
                        "rationale": "Sensitive profile endpoint.",
                        "url": "https://api.example.com/api/me"
                    }]
                }),
            )
            .expect("caller-authored per-endpoint request controls must be blocked");
        assert!(caller_authored_request["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("only endpoint_id, query_values, and rationale"));

        let incomplete_review_shape = mode
            .block_result_with_args(
                "vuln_probe_anonymous_access",
                &serde_json::json!({
                    "target_id": "33333333-3333-3333-3333-333333333333",
                    "target_url": "https://api.example.com",
                    "selected_probes": []
                }),
            )
            .expect("the complete endpoint review witness is mandatory");
        assert!(incomplete_review_shape["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("reviewed_endpoint_ids"));

        let raw = mode
            .block_result_with_args(
                "pentest_run",
                &serde_json::json!({
                    "tool_name": "nuclei",
                    "args": "-u https://app.example.com"
                }),
            )
            .expect("raw vuln CLI should be blocked");
        assert!(raw["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("vuln_nuclei_general"));

        let missing_target_id = mode
            .block_result_with_args(
                "vuln_nuclei_general",
                &serde_json::json!({
                    "target_url": "https://app.example.com",
                    "techniques": ["WSTG-INPV-05"]
                }),
            )
            .expect("the guarded wrapper requires target_id");
        assert!(missing_target_id["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("target_id"));

        let default_all = mode
            .block_result_with_args(
                "vuln_nuclei_general",
                &serde_json::json!({
                    "target_id": "11111111-1111-1111-1111-111111111111",
                    "target_url": "https://app.example.com"
                }),
            )
            .expect("gap repair must name techniques explicitly");
        assert!(default_all["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("techniques[]"));

        let wrong_pair = mode
            .block_result_with_args(
                "vuln_nuclei_general",
                &serde_json::json!({
                    "target_id": "11111111-1111-1111-1111-111111111111",
                    "target_url": "https://app.example.com",
                    "techniques": ["GOLISH-NDAY"]
                }),
            )
            .expect("unlisted target/technique pair should be blocked");
        assert!(wrong_pair["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("target/technique pair"));

        let wrong_targeted_technique = mode
            .block_result_with_args(
                "vuln_nuclei_fingerprint_targeted",
                &serde_json::json!({
                    "target_id": "11111111-1111-1111-1111-111111111111",
                    "target_url": "https://app.example.com",
                    "techniques": ["WSTG-INPV-05"]
                }),
            )
            .expect("fingerprint-targeted wrapper is N-day only");
        assert!(wrong_targeted_technique["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("GOLISH-NDAY"));
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
        assert!(mode.block_result("stage_worklist_status").is_none());
        assert!(mode.block_result("stage_worklist_next").is_none());
    }

    #[test]
    fn coverage_gap_repair_blocks_single_eas_wrapper_target_outside_action_list() {
        let v = serde_json::json!({
            "status": "needs_fix",
            "reasons": ["external attack surface incomplete: never attempted"],
            "coverage_gap_actions": [{
                "asset": "112.65.238.93",
                "technique": "GOLISH-EAS-LIVENESS",
                "reason": "missing_terminal_coverage",
                "suggested_tools": ["eas_probe_http_liveness"]
            }]
        });
        let update = submit_repair_update("submit_stage_deliverable", &v)
            .expect("structured coverage gaps should activate repair mode");
        let SubmitRepairModeUpdate::Set(mode) = update else {
            panic!("expected Set repair mode");
        };

        assert!(mode
            .block_result_with_args(
                "eas_probe_http_liveness",
                &serde_json::json!({
                    "targets": ["https://112.65.238.93"]
                }),
            )
            .is_none());

        let blocked = mode
            .block_result_with_args(
                "eas_probe_http_liveness",
                &serde_json::json!({
                    "targets": ["https://203.0.113.10"]
                }),
            )
            .expect("unlisted target should be blocked");
        assert_eq!(blocked["repair_kind"], "coverage_gap");
        assert!(blocked["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("not in the EAS coverage_gap_actions"));
    }

    #[test]
    fn coverage_gap_repair_allows_eas_web_wrapper_from_technique_without_hint() {
        let v = serde_json::json!({
            "status": "needs_fix",
            "reasons": ["external attack surface incomplete: web fingerprint never attempted"],
            "coverage_gap_actions": [{
                "asset": "https://app.example.com",
                "technique": "GOLISH-EAS-WEB-FINGERPRINT",
                "reason": "missing_terminal_coverage"
            }]
        });
        let update = submit_repair_update("submit_stage_deliverable", &v)
            .expect("coverage gaps should activate repair mode");
        let SubmitRepairModeUpdate::Set(mode) = update else {
            panic!("expected Set repair mode");
        };

        assert!(mode.allows("eas_fingerprint_web_stack"));
        assert!(mode
            .block_result_with_args(
                "eas_fingerprint_web_stack",
                &serde_json::json!({
                    "target_urls": ["https://app.example.com"]
                }),
            )
            .is_none());
        assert!(mode.block_result("whatweb").is_some());
        assert!(mode.block_result("pentest_run").is_some());
    }

    #[test]
    fn coverage_gap_repair_requires_exact_eas_web_origin_when_gate_names_one() {
        let v = serde_json::json!({
            "status": "needs_fix",
            "reasons": ["external attack surface incomplete: web fingerprint never attempted"],
            "coverage_gap_actions": [{
                "asset": "https://211.91.20.180:443",
                "technique": "GOLISH-EAS-WEB-FINGERPRINT",
                "reason": "missing_terminal_coverage",
                "suggested_tools": ["eas_fingerprint_web_stack"]
            }]
        });
        let update = submit_repair_update("submit_stage_deliverable", &v)
            .expect("coverage gaps should activate repair mode");
        let SubmitRepairModeUpdate::Set(mode) = update else {
            panic!("expected Set repair mode");
        };

        assert!(mode
            .block_result_with_args(
                "eas_fingerprint_web_stack",
                &serde_json::json!({
                    "target_urls": ["https://211.91.20.180:443"]
                }),
            )
            .is_none());

        let blocked = mode
            .block_result_with_args(
                "eas_fingerprint_web_stack",
                &serde_json::json!({
                    "target_urls": ["http://211.91.20.180:443"]
                }),
            )
            .expect("a guessed scheme must not inherit authorization from the host");
        assert!(blocked["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("not an exact EAS WEB origin"));
    }

    #[test]
    fn coverage_gap_repair_allows_eas_web_wrapper_worklist_objects() {
        let v = serde_json::json!({
            "status": "needs_fix",
            "reasons": ["external attack surface incomplete: web fingerprint never attempted"],
            "coverage_gap_actions": [{
                "asset": "app.example.com",
                "technique": "GOLISH-EAS-WEB-FINGERPRINT",
                "reason": "missing_terminal_coverage",
                "suggested_tools": ["eas_fingerprint_web_stack"]
            }]
        });
        let update = submit_repair_update("submit_stage_deliverable", &v)
            .expect("coverage gaps should activate repair mode");
        let SubmitRepairModeUpdate::Set(mode) = update else {
            panic!("expected Set repair mode");
        };

        let blocked_before_refresh = mode
            .block_result_with_args(
                "eas_fingerprint_web_stack",
                &serde_json::json!({
                    "target_urls": [{
                        "target_id": "d609c2b7-87de-40cf-bd7d-8de3d213f67b",
                        "target_url": "https://app.example.com:443"
                    }]
                }),
            )
            .expect("host-level gate actions must not authorize guessed WEB origins");
        assert!(blocked_before_refresh["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("stage_worklist_next"));

        let worklist = serde_json::json!({
            "tool": "stage_worklist_next",
            "stage": "external_attack_surface",
            "items": [{
                "asset": "app.example.com",
                "target_id": "d609c2b7-87de-40cf-bd7d-8de3d213f67b",
                "technique": "GOLISH-EAS-WEB-FINGERPRINT",
                "state": "pending",
                "details": {
                    "recommended_args": {
                        "target_urls": [{
                            "target_id": "d609c2b7-87de-40cf-bd7d-8de3d213f67b",
                            "target_url": "https://app.example.com:443"
                        }]
                    }
                }
            }, {
                "asset": "106.117.210.102",
                "target_id": "9fd0f197-83b1-4978-985e-38706e01b83a",
                "technique": "GOLISH-EAS-WEB-FINGERPRINT",
                "state": "partial",
                "details": {
                    "recommended_args": {
                        "target_urls": [{
                            "target_id": "9fd0f197-83b1-4978-985e-38706e01b83a",
                            "target_url": "http://106.117.210.102:1935"
                        }]
                    }
                }
            }]
        });
        let update = submit_repair_update_after_tool_result(
            "stage_worklist_next",
            &worklist,
            true,
            Some(&mode),
        )
        .expect("DB worklist should refine the WEB repair authorization");
        let SubmitRepairModeUpdate::Set(mode) = update else {
            panic!("expected refined repair mode");
        };

        assert_eq!(mode.eas_web_repair_targets.as_ref().unwrap().len(), 2);
        let mode: crate::SubmitRepairMode = serde_json::from_value(
            serde_json::to_value(mode).expect("exact WEB repair lock serializes"),
        )
        .expect("exact WEB repair lock restores");
        assert!(mode
            .block_result_with_args(
                "eas_fingerprint_web_stack",
                &serde_json::json!({
                    "target_urls": [{
                        "target_id": "d609c2b7-87de-40cf-bd7d-8de3d213f67b",
                        "target_url": "https://app.example.com:443"
                    }, {
                        "target_id": "9fd0f197-83b1-4978-985e-38706e01b83a",
                        "target_url": "http://106.117.210.102:1935"
                    }]
                }),
            )
            .is_none());
        assert!(mode
            .block_result_with_args(
                "eas_fingerprint_web_stack",
                &serde_json::json!({
                    "target_urls": ["https://app.example.com:443"]
                }),
            )
            .is_none());

        let blocked = mode
            .block_result_with_args(
                "eas_fingerprint_web_stack",
                &serde_json::json!({
                    "target_urls": [{
                        "target_id": "wrong-target-id",
                        "target_url": "https://app.example.com:443"
                    }]
                }),
            )
            .expect("an object-form target with the wrong binding must remain blocked");
        assert!(blocked["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("current DB-backed stage worklist"));

        for guessed in [
            "http://app.example.com:443",
            "https://app.example.com:80",
            "https://outside.example.com:443",
        ] {
            let blocked = mode
                .block_result_with_args(
                    "eas_fingerprint_web_stack",
                    &serde_json::json!({"target_urls": [guessed]}),
                )
                .expect("non-authoritative scheme/port/host must remain blocked");
            assert!(blocked["blocked_reason"]
                .as_str()
                .unwrap()
                .contains("current DB-backed stage worklist"));
        }
    }

    #[test]
    fn coverage_gap_repair_refines_web_targets_from_check_coverage_gap_examples() {
        let submit = serde_json::json!({
            "status": "needs_fix",
            "reasons": ["external attack surface incomplete: web fingerprint never attempted"],
            "coverage_gap_actions": [{
                "asset": "211.91.20.180",
                "technique": "GOLISH-EAS-WEB-FINGERPRINT",
                "reason": "missing_terminal_coverage"
            }]
        });
        let mode = submit_repair_mode_from_submit_result("submit_stage_deliverable", &submit)
            .expect("coverage repair mode");
        let coverage = serde_json::json!({
            "stage": "external_attack_surface",
            "gap_examples": [{
                "asset": "211.91.20.180",
                "target_id": "target-211",
                "technique": "GOLISH-EAS-WEB-FINGERPRINT",
                "state": "partial",
                "details": {
                    "missing_origins": ["https://211.91.20.180:443"]
                }
            }]
        });
        let update = submit_repair_update_after_tool_result(
            "check_stage_asset_coverage",
            &coverage,
            true,
            Some(&mode),
        )
        .expect("DB coverage gaps should refine exact WEB authorization");
        let SubmitRepairModeUpdate::Set(mode) = update else {
            panic!("expected refined repair mode");
        };
        assert_eq!(
            mode.eas_web_repair_targets,
            Some(vec![crate::EasWebRepairTarget {
                target_id: "target-211".to_string(),
                target_url: "https://211.91.20.180:443".to_string(),
            }])
        );
    }

    #[test]
    fn eas_web_worklist_refresh_is_authoritative_for_later_calls_in_the_same_batch() {
        let submit = serde_json::json!({
            "status": "needs_fix",
            "reasons": ["external attack surface incomplete: web fingerprint never attempted"],
            "coverage_gap_actions": [{
                "asset": "app.example.com",
                "technique": "GOLISH-EAS-WEB-FINGERPRINT",
                "reason": "missing_terminal_coverage"
            }]
        });
        let initial = submit_repair_mode_from_submit_result("submit_stage_deliverable", &submit)
            .expect("coverage repair mode");
        let target = serde_json::json!({
            "target_id": "d609c2b7-87de-40cf-bd7d-8de3d213f67b",
            "target_url": "https://app.example.com:443"
        });
        let worklist = serde_json::json!({
            "ready_to_submit": false,
            "items": [{
                "asset": "app.example.com",
                "target_id": target["target_id"],
                "technique": "GOLISH-EAS-WEB-FINGERPRINT",
                "details": {"recommended_args": {"target_urls": [target.clone()]}}
            }]
        });

        let update = submit_repair_update_after_tool_result(
            "stage_worklist_next",
            &worklist,
            true,
            Some(&initial),
        );
        let mut effective = Some(initial);
        let mut observed = None;
        update_submit_repair_mode_in_batch(&mut effective, &mut observed, update);

        assert!(
            effective
                .as_ref()
                .expect("repair mode remains active")
                .block_result_with_args(
                    "eas_fingerprint_web_stack",
                    &serde_json::json!({"target_urls": [target]}),
                )
                .is_none(),
            "the refreshed exact lock must guard the next call in this assistant batch"
        );
        assert!(matches!(observed, Some(SubmitRepairModeUpdate::Set(_))));
    }

    #[test]
    fn bounded_empty_refresh_cannot_erase_a_nonempty_eas_web_lock() {
        let submit = serde_json::json!({
            "status": "needs_fix",
            "reasons": ["external attack surface incomplete: web fingerprint never attempted"],
            "coverage_gap_actions": [{
                "asset": "app.example.com",
                "technique": "GOLISH-EAS-WEB-FINGERPRINT",
                "reason": "missing_terminal_coverage"
            }]
        });
        let initial = submit_repair_mode_from_submit_result("submit_stage_deliverable", &submit)
            .expect("coverage repair mode");
        let first_page = serde_json::json!({
            "ready_to_submit": false,
            "items": [{
                "asset": "app.example.com",
                "target_id": "target-app",
                "technique": "GOLISH-EAS-WEB-FINGERPRINT",
                "details": {"recommended_args": {"target_urls": [{
                    "target_id": "target-app",
                    "target_url": "https://app.example.com:443"
                }]}}
            }]
        });
        let locked = refine_eas_web_repair_mode_from_worklist(&initial, &first_page)
            .expect("a nonempty DB page establishes the exact lock");

        let bounded_empty = serde_json::json!({
            "ready_to_submit": false,
            "items": [],
            "omitted_item_count": 188
        });
        assert_eq!(
            refine_eas_web_repair_mode_from_worklist(&locked, &bounded_empty),
            None,
            "an empty bounded sample is not proof that no exact WEB work remains"
        );

        let ready = serde_json::json!({"ready_to_submit": true, "items": []});
        let closed = refine_eas_web_repair_mode_from_worklist(&locked, &ready)
            .expect("ready_to_submit explicitly closes the lock");
        assert_eq!(closed.eas_web_repair_targets, Some(Vec::new()));
    }

    #[test]
    fn repeated_needs_fix_for_same_web_gap_keeps_refined_exact_lock() {
        let submit = serde_json::json!({
            "status": "needs_fix",
            "reasons": ["EAS WEB exact origins remain"],
            "coverage_gap_actions": [{
                "asset": "http://218.12.76.157:1935",
                "technique": "GOLISH-EAS-WEB-FINGERPRINT",
                "reason": "missing_exact_origin",
                "suggested_tools": ["eas_fingerprint_web_stack"]
            }]
        });
        let initial = submit_repair_mode_from_submit_result("submit_stage_deliverable", &submit)
            .expect("coverage repair mode");
        let worklist = serde_json::json!({
            "ready_to_submit": false,
            "items": [{
                "asset": "218.12.76.157",
                "target_id": "target-218",
                "technique": "GOLISH-EAS-WEB-FINGERPRINT",
                "details": {"recommended_args": {"target_urls": [{
                    "target_id": "target-218",
                    "target_url": "http://218.12.76.157:1935"
                }]}}
            }]
        });
        let locked = refine_eas_web_repair_mode_from_worklist(&initial, &worklist)
            .expect("DB worklist establishes the exact lock");

        let update = submit_repair_update_after_tool_result(
            "submit_stage_deliverable",
            &submit,
            false,
            Some(&locked),
        )
        .expect("needs_fix keeps repair mode active");
        let SubmitRepairModeUpdate::Set(next) = update else {
            panic!("expected Set repair mode");
        };

        assert_eq!(
            next.eas_web_repair_targets, locked.eas_web_repair_targets,
            "a repeated needs_fix for the identical WEB gap must not erase the DB-backed lock"
        );
    }

    #[test]
    fn coverage_gap_repair_blocks_raw_pentest_run_for_eas_actions() {
        let v = serde_json::json!({
            "status": "needs_fix",
            "reasons": ["external attack surface incomplete: never attempted"],
            "coverage_gap_actions": [{
                "asset": "124.196.9.134",
                "technique": "GOLISH-EAS-LIVENESS",
                "reason": "missing_terminal_coverage",
                "suggested_tools": ["eas_probe_http_liveness"]
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
        assert!(blocked["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("EAS coverage-gap repair"));
    }

    #[test]
    fn coverage_gap_repair_blocks_multi_target_eas_wrapper_when_any_target_unlisted() {
        let v = serde_json::json!({
            "status": "needs_fix",
            "reasons": ["external attack surface incomplete: never attempted"],
            "coverage_gap_actions": [{
                "asset": "124.196.9.134",
                "technique": "GOLISH-EAS-LIVENESS",
                "reason": "missing_terminal_coverage",
                "suggested_tools": ["eas_probe_http_liveness"]
            }]
        });
        let update = submit_repair_update("submit_stage_deliverable", &v)
            .expect("coverage gaps should activate repair mode");
        let SubmitRepairModeUpdate::Set(mode) = update else {
            panic!("expected Set repair mode");
        };

        let blocked = mode
            .block_result_with_args(
                "eas_probe_http_liveness",
                &serde_json::json!({
                    "targets": ["124.196.9.134", "124.196.9.146"]
                }),
            )
            .expect("multi-target probes should be blocked during coverage repair");

        assert_eq!(blocked["repair_kind"], "coverage_gap");
        assert!(blocked["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("not in the EAS coverage_gap_actions"));
    }

    #[test]
    fn coverage_gap_repair_batch_blocks_when_any_target_unlisted() {
        let v = serde_json::json!({
            "status": "needs_fix",
            "reasons": ["content enumeration incomplete: never attempted"],
            "coverage_gap_actions": [{
                "asset": "dayu.moresec.cn",
                "technique": "GOLISH-ENUM-JSAPI",
                "reason": "missing_terminal_coverage",
                "suggested_tools": ["browser_collect_js_api"]
            }]
        });
        let update = submit_repair_update("submit_stage_deliverable", &v)
            .expect("coverage gaps should activate repair mode");
        let SubmitRepairModeUpdate::Set(mode) = update else {
            panic!("expected Set repair mode");
        };

        // A batch that stays entirely inside the named gaps is allowed.
        assert!(mode
            .block_result_with_args(
                "browser_collect_js_api",
                &serde_json::json!({ "target_urls": ["https://dayu.moresec.cn/"] }),
            )
            .is_none());
        assert!(mode
            .block_result_with_args(
                "browser_collect_js_api",
                &serde_json::json!({
                    "target_urls": [{
                        "target_id": "11111111-1111-1111-1111-111111111111",
                        "target_url": "https://dayu.moresec.cn/"
                    }]
                }),
            )
            .is_none());

        // A batch that smuggles an un-named target is blocked as a whole.
        let blocked = mode
            .block_result_with_args(
                "browser_collect_js_api",
                &serde_json::json!({
                    "target_urls": ["https://dayu.moresec.cn/", "https://package.moresec.cn/"]
                }),
            )
            .expect("unlisted batch target should be blocked");
        assert_eq!(blocked["repair_kind"], "coverage_gap");
        assert!(blocked["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("not in coverage_gap_actions"));
    }

    #[test]
    fn coverage_gap_repair_batch_route_probe_checks_each_base_url() {
        let v = serde_json::json!({
            "status": "needs_fix",
            "reasons": ["content enumeration incomplete: never attempted"],
            "coverage_gap_actions": [{
                "asset": "dayu.moresec.cn",
                "technique": "GOLISH-ENUM-DIR",
                "reason": "missing_terminal_coverage",
                "suggested_tools": ["route_probe_paths"]
            }]
        });
        let update = submit_repair_update("submit_stage_deliverable", &v)
            .expect("coverage gaps should activate repair mode");
        let SubmitRepairModeUpdate::Set(mode) = update else {
            panic!("expected Set repair mode");
        };

        let blocked = mode
            .block_result_with_args(
                "route_probe_paths",
                &serde_json::json!({
                    "targets": [
                        {"target_id": "11111111-1111-1111-1111-111111111111", "base_url": "https://dayu.moresec.cn/"},
                        {"target_id": "22222222-2222-2222-2222-222222222222", "base_url": "https://package.moresec.cn/"}
                    ]
                }),
            )
            .expect("unlisted batch base_url should be blocked");
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
    fn submit_needs_fix_runtime_correction_reuses_bounded_recovery_projection() {
        let actions = (0..1_176)
            .map(|index| {
                serde_json::json!({
                    "asset": format!("https://root-{index:04}.example:443"),
                    "technique": "GOLISH-ENUM-DIR",
                    "reason": "missing_terminal_coverage",
                    "suggested_tools": ["route_probe_paths"]
                })
            })
            .collect::<Vec<_>>();
        let mut value = serde_json::json!({
            "status": "needs_fix",
            "reasons": ["enumeration coverage cells remain unfinished"],
            "coverage_gap_actions": actions,
        });

        let note = submit_needs_fix_runtime_correction("submit_stage_deliverable", &mut value)
            .expect("coverage needs_fix should produce a correction");

        assert!(note.contains("Recovery actions: total=1176"));
        assert!(note.contains("stable_hash="));
        assert!(note.contains("stage_worklist_next"));
        assert!(note.contains("root-0019.example"));
        assert!(!note.contains("root-0020.example"));
        assert!(!note.contains("root-1175.example"));
        assert!(note.len() <= 40 * 1024);
        assert_eq!(value["runtime_correction"], serde_json::json!(note));
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
    fn long_guarded_bridge_tools_bypass_sub_agent_outer_timeout() {
        for tool_name in [
            "submit_stage_deliverable",
            "vuln_nuclei_general",
            "vuln_nuclei_fingerprint_targeted",
            "browser_collect_js_api",
            "js_extract_apis",
            "route_probe_paths",
            "eas_discover_ports",
            "eas_probe_http_liveness",
            "eas_fingerprint_services",
            "eas_fingerprint_web_stack",
        ] {
            assert!(
                !use_sub_agent_outer_tool_timeout(tool_name),
                "{tool_name} should keep running instead of being dropped by the sub-agent outer timeout"
            );
        }

        assert!(use_sub_agent_outer_tool_timeout(
            "vuln_probe_anonymous_access"
        ));
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
    fn host_owned_stage_submission_returns_on_first_durable_result() {
        assert!(stage_submission_barrier_response(
            "submit_stage_deliverable",
            &serde_json::json!({"status": "accepted", "deliverable_submission_id": uuid::Uuid::new_v4()}),
            false,
        )
        .is_some());
        assert!(stage_submission_barrier_response(
            "submit_stage_deliverable",
            &serde_json::json!({"status": "needs_fix", "deliverable_submission_id": "submission-1"}),
            true,
        )
        .is_some());
        assert!(stage_submission_barrier_response(
            "submit_stage_deliverable",
            &serde_json::json!({"status": "needs_fix", "deliverable_submission_id": "submission-1"}),
            false,
        )
        .is_none());
        assert!(stage_submission_barrier_response(
            "submit_stage_deliverable",
            &serde_json::json!({"status": "needs_fix"}),
            true,
        )
        .is_none());
        assert!(stage_submission_barrier_response(
            "stage_worklist_status",
            &serde_json::json!({"status": "accepted"}),
            true,
        )
        .is_none());
    }

    #[test]
    fn company_controller_submit_result_is_rejected_until_router_barrier() {
        let rejected = stage_team_controller_submit_result_rejection(true)
            .expect("an active Company Controller must not terminate through submit_result");
        assert_eq!(
            rejected.get("code").and_then(serde_json::Value::as_str),
            Some("STAGE_TEAM_CONTROLLER_REQUIRES_ROUTER")
        );
        assert_eq!(
            rejected.get("blocked_by_controller_router"),
            Some(&serde_json::Value::Bool(true))
        );
        assert!(stage_team_controller_submit_result_rejection(false).is_none());
    }

    #[test]
    fn candidate_terminal_intent_is_a_no_more_external_action_barrier() {
        let intent_id = uuid::Uuid::new_v4();
        let intent_id_text = intent_id.to_string();
        assert_eq!(
            candidate_terminal_intent_persisted(
                "submit_candidate_attempt",
                &serde_json::json!({
                    "status": "terminal_intent_persisted",
                    "terminal_intent_id": intent_id,
                }),
            ),
            Some(intent_id_text)
        );
        assert_eq!(
            candidate_terminal_intent_persisted(
                "submit_candidate_attempt",
                &serde_json::json!({"status": "rejected"}),
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
