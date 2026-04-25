//! Tool-call related events: requests, approvals, results, output chunks,
//! and Claude server-side tools (web_search / web_fetch).

use golish_core::events::ToolSource;
use golish_core::hitl::{ApprovalPattern, RiskLevel};

pub(super) fn tool_request(
    tool_name: &str,
    args: &serde_json::Value,
    request_id: &str,
    source: &ToolSource,
) -> serde_json::Value {
    serde_json::json!({
        "tool_name": tool_name,
        "input": args, // Renamed from "args" for the CLI JSON contract.
        "request_id": request_id,
        "source": source
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn tool_approval_request(
    request_id: &str,
    tool_name: &str,
    args: &serde_json::Value,
    stats: &Option<ApprovalPattern>,
    risk_level: &RiskLevel,
    can_learn: bool,
    suggestion: &Option<String>,
    source: &ToolSource,
) -> serde_json::Value {
    serde_json::json!({
        "request_id": request_id,
        "tool_name": tool_name,
        "input": args,
        "stats": stats,
        "risk_level": risk_level,
        "can_learn": can_learn,
        "suggestion": suggestion,
        "source": source
    })
}

pub(super) fn tool_auto_approved(
    request_id: &str,
    tool_name: &str,
    args: &serde_json::Value,
    reason: &str,
    source: &ToolSource,
) -> serde_json::Value {
    serde_json::json!({
        "request_id": request_id,
        "tool_name": tool_name,
        "input": args,
        "reason": reason,
        "source": source
    })
}

pub(super) fn tool_denied(
    request_id: &str,
    tool_name: &str,
    args: &serde_json::Value,
    reason: &str,
    source: &ToolSource,
) -> serde_json::Value {
    serde_json::json!({
        "request_id": request_id,
        "tool_name": tool_name,
        "input": args,
        "reason": reason,
        "source": source
    })
}

pub(super) fn tool_result(
    tool_name: &str,
    result: &serde_json::Value,
    success: bool,
    request_id: &str,
    source: &ToolSource,
) -> serde_json::Value {
    serde_json::json!({
        "tool_name": tool_name,
        "output": result, // Renamed from "result" for the CLI JSON contract.
        "success": success,
        "request_id": request_id,
        "source": source
    })
}

pub(super) fn tool_output_chunk(
    request_id: &str,
    tool_name: &str,
    chunk: &str,
    stream: &str,
    source: &ToolSource,
) -> serde_json::Value {
    serde_json::json!({
        "request_id": request_id,
        "tool_name": tool_name,
        "chunk": chunk,
        "stream": stream,
        "source": source
    })
}

pub(super) fn tool_response_truncated(
    tool_name: &str,
    original_tokens: usize,
    truncated_tokens: usize,
) -> serde_json::Value {
    serde_json::json!({
        "tool_name": tool_name,
        "original_tokens": original_tokens,
        "truncated_tokens": truncated_tokens
    })
}

pub(super) fn server_tool_started(
    request_id: &str,
    tool_name: &str,
    input: &serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "request_id": request_id,
        "tool_name": tool_name,
        "input": input
    })
}

pub(super) fn web_search_result(
    request_id: &str,
    results: &serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "request_id": request_id,
        "results": results
    })
}

pub(super) fn web_fetch_result(
    request_id: &str,
    url: &str,
    content_preview: &str,
) -> serde_json::Value {
    serde_json::json!({
        "request_id": request_id,
        "url": url,
        "content_preview": content_preview
    })
}
