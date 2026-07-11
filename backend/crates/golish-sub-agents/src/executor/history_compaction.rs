//! Deterministic, provider-facing compaction for durable sub-agent chains.
//!
//! Normal tool dispatch already writes a model-visible projection, but durable
//! chains can predate that projection and a single long worker can still
//! accumulate many individually-bounded turns. Exact resume must therefore
//! compact the restored body before the first provider request, and every loop
//! iteration must enforce the same total-history ceiling.

use std::collections::BTreeMap;

use rig::completion::{AssistantContent, Message};
use rig::message::{Text, ToolResultContent, UserContent};
use rig::one_or_many::OneOrMany;

const REPAIR_DIRECTIVE_PREFIX: &str = "RESUME REPAIR DIRECTIVE (deterministic):";

/// History-only budget. The system prompt, tool schemas, and reserved output
/// tokens are outside this value; keeping replay below 512 KiB leaves a wide
/// safety margin for providers whose tokenizers are less byte-efficient than
/// the usual four-bytes-per-token estimate.
pub(super) const MAX_PROVIDER_HISTORY_BYTES: usize = 512 * 1024;
const MAX_RESUMED_TOOL_RESULT_BYTES: usize = 32 * 1024;
const MAX_REPAIR_DIRECTIVE_BYTES: usize = 8 * 1024;
const MAX_PLAIN_TEXT_BYTES: usize = 8 * 1024;
const MAX_TOOL_ARGUMENT_BYTES: usize = 16 * 1024;
const MAX_TOOL_RESULT_SUFFIX_BYTES: usize = 2 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct HistoryCompactionStats {
    pub before_bytes: usize,
    pub after_bytes: usize,
    pub compacted_tool_results: usize,
    pub collapsed_repair_directives: usize,
    pub omitted_messages: usize,
}

impl HistoryCompactionStats {
    pub fn changed(self) -> bool {
        self.before_bytes != self.after_bytes
            || self.compacted_tool_results > 0
            || self.collapsed_repair_directives > 0
            || self.omitted_messages > 0
    }
}

/// Compact a complete, already pair-valid chat history.
///
/// Tool call/result ids are never rewritten. If the total ceiling still cannot
/// be met after structured projections, the oldest *complete units* are
/// omitted: an assistant tool-call turn and its immediate result turn move as
/// one unit, so provider pairing remains legal. Retained units always form a
/// contiguous newest suffix; older small units never cross an omitted unit.
pub(super) fn compact_history_for_provider(
    messages: Vec<Message>,
) -> anyhow::Result<(Vec<Message>, HistoryCompactionStats)> {
    let before_bytes = encoded_len(&messages)?;
    let (mut messages, collapsed_repair_directives) = collapse_repair_directives(messages);
    let compacted_tool_results = compact_tool_turns(&mut messages)?;
    compact_large_tool_arguments(&mut messages);

    if encoded_len(&messages)? > MAX_PROVIDER_HISTORY_BYTES {
        compact_plain_text(&mut messages);
    }

    let mut omitted_messages = 0;
    if encoded_len(&messages)? > MAX_PROVIDER_HISTORY_BYTES {
        let compacted = retain_newest_complete_units(messages)?;
        messages = compacted.0;
        omitted_messages = compacted.1;
    }

    let after_bytes = encoded_len(&messages)?;
    anyhow::ensure!(
        after_bytes <= MAX_PROVIDER_HISTORY_BYTES,
        "sub-agent history remains above provider budget after deterministic compaction: {after_bytes} > {MAX_PROVIDER_HISTORY_BYTES} bytes"
    );

    Ok((
        messages,
        HistoryCompactionStats {
            before_bytes,
            after_bytes,
            compacted_tool_results,
            collapsed_repair_directives,
            omitted_messages,
        },
    ))
}

fn encoded_len(messages: &[Message]) -> anyhow::Result<usize> {
    Ok(serde_json::to_vec(messages)?.len())
}

fn is_repair_directive_message(message: &Message) -> bool {
    let Message::User { content } = message else {
        return false;
    };
    let mut items = content.iter();
    let Some(UserContent::Text(text)) = items.next() else {
        return false;
    };
    items.next().is_none() && text.text.starts_with(REPAIR_DIRECTIVE_PREFIX)
}

fn collapse_repair_directives(messages: Vec<Message>) -> (Vec<Message>, usize) {
    let latest = messages.iter().rposition(is_repair_directive_message);
    let count = messages
        .iter()
        .filter(|message| is_repair_directive_message(message))
        .count();
    if count == 0 {
        return (messages, 0);
    }

    let mut compacted = Vec::with_capacity(messages.len() - count.saturating_sub(1));
    for (index, message) in messages.into_iter().enumerate() {
        if is_repair_directive_message(&message) {
            if Some(index) != latest {
                continue;
            }
            let Message::User { content } = message else {
                unreachable!("repair directive predicate accepts only user messages")
            };
            let UserContent::Text(text) = content.first() else {
                unreachable!("repair directive predicate accepts only text messages")
            };
            compacted.push(Message::User {
                content: OneOrMany::one(UserContent::Text(Text {
                    text: bounded_repair_directive(&text.text),
                })),
            });
        } else {
            compacted.push(message);
        }
    }
    (compacted, count.saturating_sub(1))
}

fn bounded_repair_directive(text: &str) -> String {
    if text.len() <= MAX_REPAIR_DIRECTIVE_BYTES {
        return text.to_string();
    }
    bounded_head_tail(
        text,
        MAX_REPAIR_DIRECTIVE_BYTES,
        &format!(
            "\n[historical_repair_projection: original_bytes={} full action list omitted; refresh stage_worklist_status/stage_worklist_next for the authoritative next page]\n",
            text.len()
        ),
    )
}

fn assistant_tool_names(message: &Message) -> BTreeMap<String, String> {
    let Message::Assistant { content, .. } = message else {
        return BTreeMap::new();
    };
    content
        .iter()
        .filter_map(|item| match item {
            AssistantContent::ToolCall(call) => Some((
                call.call_id.clone().unwrap_or_else(|| call.id.clone()),
                call.function.name.clone(),
            )),
            _ => None,
        })
        .collect()
}

fn compact_tool_turns(messages: &mut [Message]) -> anyhow::Result<usize> {
    let mut compacted = 0;
    for index in 0..messages.len().saturating_sub(1) {
        let tool_names = assistant_tool_names(&messages[index]);
        if tool_names.is_empty() {
            continue;
        }
        let Message::User { content } = &messages[index + 1] else {
            continue;
        };
        let mut rebuilt = Vec::with_capacity(content.len());
        let mut changed = false;
        for item in content.iter() {
            let UserContent::ToolResult(result) = item else {
                rebuilt.push(item.clone());
                continue;
            };
            let call_id = result.call_id.as_deref().unwrap_or(result.id.as_str());
            let Some(tool_name) = tool_names.get(call_id) else {
                rebuilt.push(item.clone());
                continue;
            };
            let mut rebuilt_result = result.clone();
            let mut rebuilt_content = Vec::with_capacity(result.content.len());
            for result_content in result.content.iter() {
                match result_content {
                    ToolResultContent::Text(text) => {
                        let compacted_text = compact_tool_result_text(tool_name, &text.text)?;
                        changed |= compacted_text != text.text;
                        rebuilt_content.push(ToolResultContent::Text(Text {
                            text: compacted_text,
                        }));
                    }
                    other => rebuilt_content.push(other.clone()),
                }
            }
            rebuilt_result.content = OneOrMany::many(rebuilt_content).map_err(|_| {
                anyhow::anyhow!("tool result content became empty during compaction")
            })?;
            rebuilt.push(UserContent::ToolResult(rebuilt_result));
        }
        if changed {
            compacted += 1;
            messages[index + 1] = Message::User {
                content: OneOrMany::many(rebuilt).map_err(|_| {
                    anyhow::anyhow!("tool result turn became empty during compaction")
                })?,
            };
        }
    }
    Ok(compacted)
}

fn compact_tool_result_text(tool_name: &str, text: &str) -> anyhow::Result<String> {
    if text.len() <= MAX_RESUMED_TOOL_RESULT_BYTES
        && text.contains("\"resume_history_compacted\":true")
    {
        return Ok(text.to_string());
    }

    let Some((value, suffix)) = parse_first_json(text) else {
        return Ok(if text.len() <= MAX_RESUMED_TOOL_RESULT_BYTES {
            text.to_string()
        } else {
            bounded_head_tail(
                text,
                MAX_RESUMED_TOOL_RESULT_BYTES,
                &format!(
                    "\n[historical_tool_result_projection: tool={tool_name} original_bytes={}]\n",
                    text.len()
                ),
            )
        });
    };

    let original_bytes = text.len();
    let mut projected = project_tool_result(tool_name, &value);
    if let Some(object) = projected.as_object_mut() {
        object.insert(
            "resume_history_compacted".to_string(),
            serde_json::Value::Bool(true),
        );
        object.insert(
            "resume_original_bytes".to_string(),
            serde_json::json!(original_bytes),
        );
        object.insert(
            "resume_compacted_tool".to_string(),
            serde_json::json!(tool_name),
        );
    } else {
        projected = serde_json::json!({
            "value": projected,
            "resume_history_compacted": true,
            "resume_original_bytes": original_bytes,
            "resume_compacted_tool": tool_name,
        });
    }

    let suffix = if suffix.trim().is_empty() {
        String::new()
    } else {
        bounded_head_tail(
            suffix,
            MAX_TOOL_RESULT_SUFFIX_BYTES,
            "\n[historical_tool_result_suffix_compacted]\n",
        )
    };
    let json_budget = MAX_RESUMED_TOOL_RESULT_BYTES.saturating_sub(suffix.len());
    let encoded = fit_json_to_limit(projected, json_budget)?;
    let combined = format!("{encoded}{suffix}");
    anyhow::ensure!(
        combined.len() <= MAX_RESUMED_TOOL_RESULT_BYTES,
        "compacted tool result exceeds byte budget"
    );
    Ok(combined)
}

fn parse_first_json(text: &str) -> Option<(serde_json::Value, &str)> {
    let mut stream = serde_json::Deserializer::from_str(text).into_iter::<serde_json::Value>();
    let value = stream.next()?.ok()?;
    Some((value, &text[stream.byte_offset()..]))
}

fn project_tool_result(tool_name: &str, value: &serde_json::Value) -> serde_json::Value {
    match tool_name {
        "stage_worklist_status" | "stage_worklist_next" | "check_stage_asset_coverage" => {
            project_stage_worklist(value)
        }
        "list_enumeration_web_roots" => project_enumeration_roots(value),
        "enum_preflight_web_origins" => project_preflight(value),
        "route_probe_paths" => project_route_probe(value),
        "browser_collect_js_api" => project_browser_collect(value),
        "js_extract_apis" => project_js_extract(value),
        "submit_stage_deliverable" => project_submit_result(value),
        _ if text_size(value) > 24_000 => bounded_json(value, 3, 16, 1024),
        _ => value.clone(),
    }
}

fn project_stage_worklist(value: &serde_json::Value) -> serde_json::Value {
    let mut output = take_fields(
        value,
        &[
            "tool",
            "stage",
            "organization_id",
            "session_id",
            "ready_to_submit",
            "coverage_denominator_missing",
            "summary",
            "cell_summary",
            "limit",
            "prefer",
            "root_limit",
            "root_count",
            "matching_root_count",
            "omitted_root_count",
            "omitted_item_count",
            "omitted_gap_count",
            "next_tool",
            "next_action",
            "worklist_contract",
            "worklist_semantics",
            "deliverable_contract",
        ],
    );
    let exact_origin_page = value
        .get("exact_origin_page")
        .cloned()
        .or_else(|| super::response_parsing::compact_enumeration_exact_origin_page(value));
    if let Some(page) = exact_origin_page {
        output.insert("exact_origin_page".to_string(), page);
        output.insert(
            "exact_origin_page_contract".to_string(),
            serde_json::json!("Copy target_id + target_url from exact_origin_page exactly; never reconstruct them from older history."),
        );
    } else {
        project_array_field(&mut output, value, "items", 12, project_work_item);
    }
    project_array_field(&mut output, value, "gap_examples", 8, project_work_item);
    if let Some(preview) = value.get("terminal_exceptions_preview") {
        output.insert(
            "terminal_exceptions_preview".to_string(),
            bounded_json(preview, 2, 8, 512),
        );
    }
    serde_json::Value::Object(output)
}

fn project_work_item(value: &serde_json::Value) -> serde_json::Value {
    serde_json::Value::Object(take_fields(
        value,
        &[
            "work_item_id",
            "target_id",
            "asset",
            "target_type",
            "technique",
            "label",
            "state",
            "source",
            "reason",
            "evidence_refs",
            "suggested_capabilities",
            "suggested_tools",
            "root_url",
            "base_url",
            "scheme",
            "port",
            "origin_resolution",
            "enumeration_focus",
            "eas_focus",
        ],
    ))
}

fn project_enumeration_roots(value: &serde_json::Value) -> serde_json::Value {
    let mut output = take_fields(
        value,
        &[
            "stage",
            "organization_id",
            "session_id",
            "count",
            "total",
            "truncated",
            "omitted_root_count",
            "worklist_semantics",
            "execution_order",
            "tool_boundary",
            "next_action",
        ],
    );
    for key in ["pending_roots_sample", "terminal_roots_sample", "web_roots"] {
        project_array_field(&mut output, value, key, 12, |root| {
            serde_json::Value::Object(take_fields(
                root,
                &[
                    "target_id",
                    "root_url",
                    "target_type",
                    "pending_techniques",
                    "terminal_techniques",
                    "suggested_tools",
                    "next_steps",
                ],
            ))
        });
    }
    serde_json::Value::Object(output)
}

fn project_preflight(value: &serde_json::Value) -> serde_json::Value {
    // Preflight partitions are the exact authorization-safe handoff to the
    // same-page producers. Reuse the immediate projection so durable replay
    // keeps all 1..=50 classified identities instead of applying the generic
    // 20-item array sampler a second time.
    super::response_parsing::compact_enum_preflight_result(value)
}

fn project_route_probe(value: &serde_json::Value) -> serde_json::Value {
    let mut output = take_fields(
        value,
        &[
            "batch",
            "status",
            "timed_out",
            "count",
            "processed",
            "succeeded",
            "terminal_completed",
            "incomplete_targets",
            "failed",
            "batch_concurrency",
            "dir_found_targets",
            "elapsed_ms",
            "next_action",
        ],
    );
    if value.get("batch").and_then(serde_json::Value::as_bool) == Some(true) {
        project_array_field(&mut output, value, "results", 50, project_route_entry);
        project_array_field(&mut output, value, "errors", 20, project_failure_entry);
        project_array_field(&mut output, value, "skipped", 20, project_failure_entry);
    } else {
        output.extend(project_route_diagnostic(value));
    }
    serde_json::Value::Object(output)
}

fn project_route_entry(value: &serde_json::Value) -> serde_json::Value {
    let mut output = take_fields(value, &["target_id", "base_url", "target_url"]);
    let diagnostic = value.get("result").unwrap_or(value);
    output.insert(
        "result".to_string(),
        serde_json::Value::Object(project_route_diagnostic(diagnostic)),
    );
    serde_json::Value::Object(output)
}

fn project_route_diagnostic(
    value: &serde_json::Value,
) -> serde_json::Map<String, serde_json::Value> {
    take_fields(
        value,
        &[
            "success",
            "base_url",
            "requested_base_url",
            "status",
            "outcome",
            "attempted_outcome",
            "completion_state",
            "outcome_persisted",
            "timed_out",
            "request_limited",
            "candidate_generation_limited",
            "recovery_exhausted",
            "automatic_retry_allowed",
            "queue_completed",
            "queue_remaining",
            "checkpoint_persisted",
            "checkpoint_version",
            "checkpoint_resume_applied",
            "checkpoint_resume_count",
            "attempt_generation_guarded",
            "attempt_superseded",
            "pending_business_writes",
            "pending_terminal_outcome",
            "terminal_cursor",
            "manual_repair_required",
            "manual_repair_reason",
            "recovery_action",
            "retry",
            "matches_count",
            "errors_count",
            "persistence_errors_count",
            "requests_sent",
            "next_action",
        ],
    )
}

fn project_browser_collect(value: &serde_json::Value) -> serde_json::Value {
    let mut output = take_fields(
        value,
        &[
            "batch",
            "input_count",
            "accepted",
            "rejected",
            "truncated",
            "skipped",
            "succeeded",
            "failed",
            "next_action",
        ],
    );
    if value.get("batch").and_then(serde_json::Value::as_bool) == Some(true) {
        if value.get("root_diagnostics").is_some() {
            project_array_field(
                &mut output,
                value,
                "root_diagnostics",
                50,
                project_browser_diagnostic,
            );
        } else {
            project_array_field(&mut output, value, "results", 50, |entry| {
                let mut projected =
                    project_browser_diagnostic(entry.get("result").unwrap_or(entry));
                if let Some(object) = projected.as_object_mut() {
                    if let Some(target_id) = entry.get("target_id") {
                        object.insert("target_id".to_string(), target_id.clone());
                    }
                    if let Some(target_url) = entry.get("target_url") {
                        object.insert("url".to_string(), target_url.clone());
                    }
                }
                projected
            });
        }
        project_array_field(&mut output, value, "errors", 20, project_failure_entry);
    } else {
        output.extend(
            project_browser_diagnostic(value)
                .as_object()
                .cloned()
                .unwrap_or_default(),
        );
    }
    serde_json::Value::Object(output)
}

fn project_browser_diagnostic(value: &serde_json::Value) -> serde_json::Value {
    let mut output = take_fields(
        value,
        &[
            "success",
            "target_id",
            "url",
            "target_url",
            "status",
            "completion_state",
            "closure_complete",
            "closure_incomplete_reasons",
            "page_queue_remaining",
            "page_resume_applied",
            "page_resume_count",
            "page_resume_prior_visited",
            "recursive_queue_remaining",
            "recursive_resume_applied",
            "recursive_resume_prior_pending",
            "recursive_errors_total",
            "checkpoint_resume_applied",
            "checkpoint_resume_count",
            "checkpoint_version",
            "attempt_generation_guarded",
            "automatic_retry_allowed",
            "recovery_exhausted",
            "recovery_instruction",
            "hard_deadline_hit",
            "hard_timeout_ms",
            "js_outcome",
            "jsapi_outcome",
            "param_outcome",
            "outcome_persisted",
            "outcome_persisted_count",
            "scripts_count",
            "scripts_total",
            "api_requests_count",
            "summary",
        ],
    );
    if let Some(failures) = value
        .get("recovery_failures")
        .and_then(serde_json::Value::as_array)
    {
        output.insert(
            "recovery_failures_count".to_string(),
            serde_json::json!(failures.len()),
        );
        output.insert(
            "recovery_failures_sample".to_string(),
            serde_json::Value::Array(
                failures
                    .iter()
                    .take(4)
                    .map(|failure| {
                        serde_json::Value::Object(take_fields(
                            failure,
                            &["kind", "count", "reason", "signature", "url"],
                        ))
                    })
                    .collect(),
            ),
        );
    }
    serde_json::Value::Object(output)
}

fn project_js_extract(value: &serde_json::Value) -> serde_json::Value {
    let mut output = take_fields(
        value,
        &[
            "batch",
            "input_count",
            "accepted",
            "rejected",
            "truncated",
            "skipped",
            "count",
            "succeeded",
            "failed",
            "jsapi_found_targets",
            "param_found_targets",
            "results_count",
            "errors_count",
            "omissions_count",
            "next_action",
        ],
    );
    if value.get("batch").and_then(serde_json::Value::as_bool) == Some(true) {
        if value.get("root_diagnostics").is_some() {
            project_array_field(
                &mut output,
                value,
                "root_diagnostics",
                50,
                project_js_diagnostic,
            );
        } else {
            project_array_field(&mut output, value, "results", 50, |entry| {
                let mut projected = project_js_diagnostic(entry.get("result").unwrap_or(entry));
                if let Some(object) = projected.as_object_mut() {
                    if let Some(target_id) = entry.get("target_id") {
                        object.insert("target_id".to_string(), target_id.clone());
                    }
                    if let Some(target_url) = entry.get("target_url") {
                        object.insert("url".to_string(), target_url.clone());
                    }
                }
                projected
            });
        }
        project_array_field(&mut output, value, "errors", 20, project_failure_entry);
    } else {
        output.extend(
            project_js_diagnostic(value)
                .as_object()
                .cloned()
                .unwrap_or_default(),
        );
    }
    serde_json::Value::Object(output)
}

fn project_js_diagnostic(value: &serde_json::Value) -> serde_json::Value {
    serde_json::Value::Object(take_fields(
        value,
        &[
            "success",
            "target_id",
            "url",
            "target_url",
            "status",
            "completion_state",
            "jsapi_outcome",
            "outcome_persisted",
            "param_outcome",
            "param_outcome_persisted",
            "authorization_drift",
            "endpoints_count",
            "endpoints_total",
            "params_count",
            "param_endpoints",
            "persisted_rows",
            "persisted_endpoint_rows",
            "outcome_persisted_count",
            "retry",
            "capture_manifest",
            "detail_reference",
            "summary",
            "error",
        ],
    ))
}

fn project_submit_result(value: &serde_json::Value) -> serde_json::Value {
    let mut output = take_fields(
        value,
        &[
            "status",
            "reasons",
            "available_evidence_ids",
            "running_background_jobs",
            "next_action",
        ],
    );
    if let Some(actions) = value
        .get("coverage_gap_actions")
        .and_then(serde_json::Value::as_array)
    {
        output.insert(
            "coverage_gap_actions_count".to_string(),
            serde_json::json!(actions.len()),
        );
        output.insert(
            "coverage_gap_actions_sample".to_string(),
            serde_json::Value::Array(actions.iter().take(8).map(project_work_item).collect()),
        );
    }
    if let Some(correction) = value.get("runtime_correction") {
        output.insert(
            "runtime_correction".to_string(),
            bounded_json(correction, 1, 4, 2048),
        );
    }
    serde_json::Value::Object(output)
}

fn project_failure_entry(value: &serde_json::Value) -> serde_json::Value {
    serde_json::Value::Object(take_fields(
        value,
        &[
            "target_id",
            "target_url",
            "base_url",
            "status",
            "completion_state",
            "outcome",
            "error",
            "reason",
            "outcome_marker",
            "retry",
        ],
    ))
}

fn take_fields(
    value: &serde_json::Value,
    fields: &[&str],
) -> serde_json::Map<String, serde_json::Value> {
    let mut output = serde_json::Map::new();
    for field in fields {
        if let Some(item) = value.get(*field) {
            output.insert((*field).to_string(), bounded_json(item, 3, 20, 1024));
        }
    }
    output
}

fn project_array_field(
    output: &mut serde_json::Map<String, serde_json::Value>,
    source: &serde_json::Value,
    field: &str,
    limit: usize,
    project: impl Fn(&serde_json::Value) -> serde_json::Value,
) {
    let Some(items) = source.get(field).and_then(serde_json::Value::as_array) else {
        return;
    };
    output.insert(format!("{field}_count"), serde_json::json!(items.len()));
    output.insert(
        field.to_string(),
        serde_json::Value::Array(items.iter().take(limit).map(project).collect()),
    );
    if items.len() > limit {
        output.insert(
            format!("resume_{field}_omitted"),
            serde_json::json!(items.len() - limit),
        );
    }
}

fn fit_json_to_limit(mut value: serde_json::Value, limit: usize) -> anyhow::Result<String> {
    let mut encoded = serde_json::to_string(&value)?;
    if encoded.len() <= limit {
        return Ok(encoded);
    }
    for (depth, arrays, strings) in [(3, 12, 512), (2, 8, 256), (1, 4, 128)] {
        value = bounded_json(&value, depth, arrays, strings);
        encoded = serde_json::to_string(&value)?;
        if encoded.len() <= limit {
            return Ok(encoded);
        }
    }
    let fallback = serde_json::json!({
        "resume_history_compacted": true,
        "resume_projection_fallback": true,
        "omitted_shape": bounded_json(&value, 0, 0, 96),
    });
    let encoded = serde_json::to_string(&fallback)?;
    anyhow::ensure!(encoded.len() <= limit, "JSON fallback exceeds byte budget");
    Ok(encoded)
}

fn bounded_json(
    value: &serde_json::Value,
    depth: usize,
    array_limit: usize,
    string_limit: usize,
) -> serde_json::Value {
    match value {
        serde_json::Value::String(text) => {
            serde_json::Value::String(if text.len() > string_limit {
                bounded_head_tail(text, string_limit.max(64), "...[bounded]...")
            } else {
                text.clone()
            })
        }
        serde_json::Value::Array(items) if depth == 0 => {
            serde_json::json!({"omitted_array_items": items.len()})
        }
        serde_json::Value::Array(items) => {
            let mut projected = items
                .iter()
                .take(array_limit)
                .map(|item| bounded_json(item, depth - 1, array_limit, string_limit))
                .collect::<Vec<_>>();
            if items.len() > projected.len() {
                projected.push(serde_json::json!({
                    "omitted_array_items": items.len() - projected.len()
                }));
            }
            serde_json::Value::Array(projected)
        }
        serde_json::Value::Object(object) if depth == 0 => {
            serde_json::json!({"omitted_object_keys": object.len()})
        }
        serde_json::Value::Object(object) => serde_json::Value::Object(
            object
                .iter()
                .take(64)
                .map(|(key, value)| {
                    (
                        key.clone(),
                        bounded_json(value, depth - 1, array_limit, string_limit),
                    )
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

fn text_size(value: &serde_json::Value) -> usize {
    serde_json::to_vec(value)
        .map(|value| value.len())
        .unwrap_or(0)
}

fn compact_large_tool_arguments(messages: &mut [Message]) {
    for message in messages {
        let Message::Assistant { content, .. } = message else {
            continue;
        };
        let mut rebuilt = Vec::with_capacity(content.len());
        let mut changed = false;
        for item in content.iter() {
            let AssistantContent::ToolCall(call) = item else {
                rebuilt.push(item.clone());
                continue;
            };
            if text_size(&call.function.arguments) <= MAX_TOOL_ARGUMENT_BYTES {
                rebuilt.push(item.clone());
                continue;
            }
            let mut call = call.clone();
            call.function.arguments = serde_json::json!({
                "historical_args_compacted": true,
                "original_bytes": text_size(&call.function.arguments),
                "keys": call
                    .function
                    .arguments
                    .as_object()
                    .map(|object| object.keys().take(32).cloned().collect::<Vec<_>>())
                    .unwrap_or_default(),
                "summary": bounded_json(&call.function.arguments, 2, 8, 256),
            });
            rebuilt.push(AssistantContent::ToolCall(call));
            changed = true;
        }
        if changed {
            *content = OneOrMany::many(rebuilt)
                .expect("assistant content containing a tool call remains non-empty");
        }
    }
}

fn compact_plain_text(messages: &mut [Message]) {
    for message in messages {
        match message {
            Message::System { content } => {
                if content.len() > MAX_PLAIN_TEXT_BYTES {
                    *content = bounded_head_tail(
                        content,
                        MAX_PLAIN_TEXT_BYTES,
                        "\n[historical_system_text_compacted]\n",
                    );
                }
            }
            Message::User { content } => {
                let mut rebuilt = Vec::with_capacity(content.len());
                let mut changed = false;
                for item in content.iter() {
                    match item {
                        UserContent::Text(text) if text.text.len() > MAX_PLAIN_TEXT_BYTES => {
                            rebuilt.push(UserContent::Text(Text {
                                text: bounded_head_tail(
                                    &text.text,
                                    MAX_PLAIN_TEXT_BYTES,
                                    "\n[historical_user_text_compacted]\n",
                                ),
                            }));
                            changed = true;
                        }
                        other => rebuilt.push(other.clone()),
                    }
                }
                if changed {
                    *content =
                        OneOrMany::many(rebuilt).expect("user message content remains non-empty");
                }
            }
            Message::Assistant { content, .. } => {
                let mut rebuilt = Vec::with_capacity(content.len());
                let mut changed = false;
                for item in content.iter() {
                    match item {
                        AssistantContent::Text(text) if text.text.len() > MAX_PLAIN_TEXT_BYTES => {
                            rebuilt.push(AssistantContent::Text(Text {
                                text: bounded_head_tail(
                                    &text.text,
                                    MAX_PLAIN_TEXT_BYTES,
                                    "\n[historical_assistant_text_compacted]\n",
                                ),
                            }));
                            changed = true;
                        }
                        other => rebuilt.push(other.clone()),
                    }
                }
                if changed {
                    *content = OneOrMany::many(rebuilt)
                        .expect("assistant message content remains non-empty");
                }
            }
        }
    }
}

fn retain_newest_complete_units(messages: Vec<Message>) -> anyhow::Result<(Vec<Message>, usize)> {
    let mut units = Vec::<Vec<Message>>::new();
    let mut iter = messages.into_iter().peekable();
    while let Some(message) = iter.next() {
        let has_calls = !assistant_tool_names(&message).is_empty();
        if has_calls {
            let mut unit = vec![message];
            if let Some(result) = iter.next() {
                unit.push(result);
            }
            units.push(unit);
        } else {
            units.push(vec![message]);
        }
    }

    let summary = Message::User {
        content: OneOrMany::one(UserContent::Text(Text {
            text: "[historical chain compacted] Older complete turns were omitted to stay within the provider context budget. Authoritative progress remains in DB/evidence; refresh stage_worklist_status/stage_worklist_next before acting.".to_string(),
        })),
    };
    let summary_bytes = encoded_len(std::slice::from_ref(&summary))?;
    let mut selected = Vec::<Vec<Message>>::new();
    let mut used = summary_bytes;
    let mut omitted_messages = 0;
    let mut reached_budget_boundary = false;
    for unit in units.into_iter().rev() {
        if reached_budget_boundary {
            omitted_messages += unit.len();
            continue;
        }
        let unit_bytes = encoded_len(&unit)?;
        if used.saturating_add(unit_bytes) <= MAX_PROVIDER_HISTORY_BYTES {
            used += unit_bytes;
            selected.push(unit);
        } else {
            omitted_messages += unit.len();
            reached_budget_boundary = true;
        }
    }
    selected.reverse();
    let mut output = Vec::new();
    if omitted_messages > 0 {
        output.push(summary);
    }
    output.extend(selected.into_iter().flatten());
    Ok((output, omitted_messages))
}

fn bounded_head_tail(text: &str, max_bytes: usize, marker: &str) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let marker = if marker.len() >= max_bytes {
        &marker[..floor_char_boundary(marker, max_bytes)]
    } else {
        marker
    };
    let remaining = max_bytes.saturating_sub(marker.len());
    let head_budget = remaining * 2 / 3;
    let tail_budget = remaining.saturating_sub(head_budget);
    let head_end = floor_char_boundary(text, head_budget);
    let tail_start = ceil_char_boundary(text, text.len().saturating_sub(tail_budget));
    format!("{}{}{}", &text[..head_end], marker, &text[tail_start..])
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::message::{ToolCall, ToolFunction, ToolResult};

    fn tool_turn(call_id: &str, tool_name: &str, result: serde_json::Value) -> Vec<Message> {
        vec![
            Message::Assistant {
                id: None,
                content: OneOrMany::one(AssistantContent::ToolCall(ToolCall {
                    id: call_id.to_string(),
                    call_id: Some(call_id.to_string()),
                    function: ToolFunction {
                        name: tool_name.to_string(),
                        arguments: serde_json::json!({}),
                    },
                    signature: None,
                    additional_params: None,
                })),
            },
            Message::User {
                content: OneOrMany::one(UserContent::ToolResult(ToolResult {
                    id: call_id.to_string(),
                    call_id: Some(call_id.to_string()),
                    content: OneOrMany::one(ToolResultContent::Text(Text {
                        text: serde_json::to_string(&result).expect("result serializes"),
                    })),
                })),
            },
        ]
    }

    fn first_tool_result(messages: &[Message]) -> serde_json::Value {
        messages
            .iter()
            .find_map(|message| {
                let Message::User { content } = message else {
                    return None;
                };
                content.iter().find_map(|item| {
                    let UserContent::ToolResult(result) = item else {
                        return None;
                    };
                    result.content.iter().find_map(|item| {
                        let ToolResultContent::Text(text) = item else {
                            return None;
                        };
                        serde_json::from_str(&text.text).ok()
                    })
                })
            })
            .expect("compacted history contains a JSON tool result")
    }

    #[test]
    fn same_segment_total_cap_omits_only_complete_tool_turns() {
        let mut history = Vec::new();
        for index in 0..100 {
            history.extend(tool_turn(
                &format!("call-{index}"),
                "small_tool",
                serde_json::json!({
                    "resume_history_compacted": true,
                    "padding": "x".repeat(30 * 1024),
                    "index": index,
                }),
            ));
        }

        let (compacted, stats) = compact_history_for_provider(history).expect("history compacts");

        assert!(stats.omitted_messages > 0);
        assert!(serde_json::to_vec(&compacted).unwrap().len() <= MAX_PROVIDER_HISTORY_BYTES);
        crate::executor_helpers::serialize_chat_history(&compacted)
            .expect("old tool turns must be omitted as complete pairs");
        let encoded = serde_json::to_string(&compacted).unwrap();
        assert!(
            encoded.contains("call-99"),
            "newest recovery turn is retained"
        );
    }

    #[test]
    fn total_cap_keeps_a_contiguous_newest_complete_unit_suffix() {
        let oldest_small = Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: "oldest-small-must-not-cross-history-hole".to_string(),
            })),
        };
        let next_older_oversized = Message::User {
            content: OneOrMany::many(
                (0..70)
                    .map(|index| {
                        UserContent::Text(Text {
                            text: format!(
                                "oversized-unit-{index:02}-{}",
                                "x".repeat(MAX_PLAIN_TEXT_BYTES - 64)
                            ),
                        })
                    })
                    .collect::<Vec<_>>(),
            )
            .expect("oversized fixture has content"),
        };
        let newest_small = Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: "newest-small-must-remain".to_string(),
            })),
        };

        let (compacted, stats) =
            compact_history_for_provider(vec![oldest_small, next_older_oversized, newest_small])
                .expect("history compacts");

        let encoded = serde_json::to_string(&compacted).expect("compacted history serializes");
        assert!(encoded.contains("newest-small-must-remain"));
        assert!(!encoded.contains("oversized-unit-"));
        assert!(
            !encoded.contains("oldest-small-must-not-cross-history-hole"),
            "once a newer complete unit cannot fit, older units must not cross that history hole"
        );
        assert_eq!(stats.omitted_messages, 2);
    }

    #[test]
    fn worklist_projection_keeps_all_fifty_exact_roots_with_current_ids() {
        let techniques = [
            "GOLISH-ENUM-JS",
            "GOLISH-ENUM-DIR",
            "GOLISH-ENUM-PARAM",
            "GOLISH-ENUM-JSAPI",
        ];
        let items = (0..50)
            .flat_map(|root| {
                techniques.into_iter().map(move |technique| {
                    serde_json::json!({
                        "work_item_id": format!("current-{root}:https://root-{root}.example:443:{technique}"),
                        "target_id": format!("current-{root}"),
                        "asset": format!("https://root-{root}.example:443"),
                        "root_url": format!("https://root-{root}.example:443/"),
                        "base_url": format!("https://root-{root}.example:443/"),
                        "technique": technique,
                        "state": "pending",
                        "padding": "x".repeat(500),
                    })
                })
            })
            .collect::<Vec<_>>();
        let raw = serde_json::json!({
            "tool": "stage_worklist_next",
            "stage": "enumeration",
            "root_limit": 50,
            "root_count": 50,
            "items": items,
        });
        let immediate = crate::executor::response_parsing::model_visible_tool_result(
            "stage_worklist_next",
            &raw,
        );
        assert_eq!(immediate["exact_origin_page"].as_array().unwrap().len(), 50);
        assert!(immediate.get("items").is_none());
        let history = tool_turn("call-worklist", "stage_worklist_next", immediate);

        let (compacted, _) = compact_history_for_provider(history).expect("worklist compacts");
        let result = first_tool_result(&compacted);
        let roots = result["exact_origin_page"].as_array().unwrap();
        assert_eq!(roots.len(), 50);
        assert_eq!(roots[0]["target_id"], "current-0");
        assert_eq!(roots[49]["target_id"], "current-49");
        assert_eq!(roots[49]["target_url"], "https://root-49.example:443");
        assert_eq!(
            roots[49]["unfinished_techniques"].as_array().unwrap().len(),
            4
        );
        assert!(result.get("items").is_none());
        assert!(serde_json::to_vec(&result).unwrap().len() <= MAX_RESUMED_TOOL_RESULT_BYTES);
    }

    #[test]
    fn preflight_projection_keeps_all_fifty_classified_origins_with_strict_shape() {
        let mut reachable = Vec::new();
        let mut blocked = Vec::new();
        let mut pending = Vec::new();
        for index in 0..50 {
            let entry = serde_json::json!({
                "target_id": format!("00000000-0000-0000-0000-{index:012}"),
                "target_url": format!("https://origin-{index:02}.example.test:443/"),
                "root_url": format!("https://origin-{index:02}.example.test:443/"),
                "unfinished_techniques": ["GOLISH-ENUM-DIR"],
            });
            match index % 3 {
                0 => reachable.push(entry),
                1 => blocked.push(entry),
                _ => pending.push(entry),
            }
        }
        let raw = serde_json::json!({
            "status": "partial",
            "input_count": 50,
            "reachable_count": reachable.len(),
            "blocked_count": blocked.len(),
            "incomplete_count": pending.len(),
            "fixed_concurrency": 8,
            "reachable_origins": reachable,
            "blocked_origins": blocked,
            "pending_origins": pending,
            "next_action": "Run producers only for reachable or pending origins.",
        });
        let immediate = crate::executor::response_parsing::model_visible_tool_result(
            "enum_preflight_web_origins",
            &raw,
        );
        assert!(
            serde_json::to_vec(&immediate).unwrap().len() <= MAX_RESUMED_TOOL_RESULT_BYTES,
            "the immediate 50-origin projection must stay within the durable per-result budget"
        );
        let history = tool_turn("call-preflight", "enum_preflight_web_origins", immediate);

        let (compacted, _) =
            compact_history_for_provider(history).expect("preflight history compacts");
        let result = first_tool_result(&compacted);

        let mut projected_identities = std::collections::BTreeMap::new();
        for (field, expected_state) in [
            ("reachable_origins", "reachable"),
            ("blocked_origins", "blocked"),
            ("pending_origins", "pending"),
        ] {
            let projected = result[field]
                .as_array()
                .expect("preflight partition remains an array");
            let expected = raw[field]
                .as_array()
                .expect("fixture partition is an array");
            assert_eq!(projected.len(), expected.len(), "partition={field}");
            for (actual, source) in projected.iter().zip(expected) {
                let keys = actual
                    .as_object()
                    .expect("projected origin is an object")
                    .keys()
                    .map(String::as_str)
                    .collect::<std::collections::BTreeSet<_>>();
                assert_eq!(
                    keys,
                    std::collections::BTreeSet::from(["target_id", "target_url"]),
                    "preflight origins must match the strict input schema"
                );
                assert_eq!(actual["target_id"], source["target_id"]);
                assert_eq!(actual["target_url"], source["target_url"]);
                projected_identities.insert(
                    actual["target_id"].as_str().unwrap().to_string(),
                    expected_state,
                );
            }
        }
        assert_eq!(projected_identities.len(), 50);
        for index in 0..50 {
            let target_id = format!("00000000-0000-0000-0000-{index:012}");
            let expected_state = match index % 3 {
                0 => "reachable",
                1 => "blocked",
                _ => "pending",
            };
            assert_eq!(projected_identities.get(&target_id), Some(&expected_state));
        }
        assert!(serde_json::to_vec(&result).unwrap().len() <= MAX_RESUMED_TOOL_RESULT_BYTES);
    }

    #[test]
    fn browser_projection_keeps_checkpoint_and_retry_state() {
        let history = tool_turn(
            "call-browser",
            "browser_collect_js_api",
            serde_json::json!({
                "batch": true,
                "input_count": 1,
                "accepted": 1,
                "succeeded": 1,
                "results": [{
                    "target_id": "target-partial",
                    "target_url": "https://partial.example:443/",
                    "result": {
                        "status": "closure_partial",
                        "completion_state": "partial",
                        "closure_complete": false,
                        "closure_incomplete_reasons": ["page_queue_remaining", "recursive_queue_remaining"],
                        "page_queue_remaining": 1,
                        "recursive_queue_remaining": 180,
                        "checkpoint_resume_applied": false,
                        "checkpoint_resume_count": 0,
                        "checkpoint_version": 2,
                        "automatic_retry_allowed": true,
                        "recovery_exhausted": false,
                        "recovery_failures": [{
                            "kind": "recursive_fetch",
                            "count": 1,
                            "reason": "fetch failed",
                            "signature": "sig-1",
                            "url": "https://partial.example/_next/a.js"
                        }],
                        "scripts": (0..500).map(|index| format!("https://partial.example/{index}.js")).collect::<Vec<_>>(),
                    }
                }]
            }),
        );

        let (compacted, _) =
            compact_history_for_provider(history).expect("browser history compacts");
        let result = first_tool_result(&compacted);
        let diagnostic = &result["results"][0];
        assert_eq!(diagnostic["target_id"], "target-partial");
        assert_eq!(diagnostic["url"], "https://partial.example:443/");
        assert_eq!(diagnostic["page_queue_remaining"], 1);
        assert_eq!(diagnostic["recursive_queue_remaining"], 180);
        assert_eq!(diagnostic["checkpoint_version"], 2);
        assert_eq!(diagnostic["automatic_retry_allowed"], true);
        assert_eq!(diagnostic["recovery_exhausted"], false);
        assert_eq!(diagnostic["recovery_failures_count"], 1);
        assert!(result.get("resume_history_compacted").is_some());
    }
}
