//! Sub-agent lifecycle, streaming, tool, and prompt-generation events.

pub(super) fn sub_agent_started(
    agent_id: &str,
    agent_name: &str,
    task: &str,
    depth: usize,
    parent_request_id: &str,
) -> serde_json::Value {
    serde_json::json!({
        "agent_id": agent_id,
        "agent_name": agent_name,
        "task": task,
        "depth": depth,
        "parent_request_id": parent_request_id
    })
}

pub(super) fn sub_agent_tool_request(
    agent_id: &str,
    tool_name: &str,
    args: &serde_json::Value,
    request_id: &str,
    parent_request_id: &str,
) -> serde_json::Value {
    serde_json::json!({
        "agent_id": agent_id,
        "tool_name": tool_name,
        "request_id": request_id,
        "input": args,
        "parent_request_id": parent_request_id
    })
}

pub(super) fn sub_agent_tool_result(
    agent_id: &str,
    tool_name: &str,
    success: bool,
    result: &serde_json::Value,
    request_id: &str,
    parent_request_id: &str,
) -> serde_json::Value {
    serde_json::json!({
        "agent_id": agent_id,
        "tool_name": tool_name,
        "request_id": request_id,
        "success": success,
        "result": result,
        "parent_request_id": parent_request_id
    })
}

pub(super) fn sub_agent_text_delta(
    agent_id: &str,
    delta: &str,
    accumulated: &str,
    parent_request_id: &str,
) -> serde_json::Value {
    serde_json::json!({
        "agent_id": agent_id,
        "delta": delta,
        "accumulated": accumulated,
        "parent_request_id": parent_request_id
    })
}

pub(super) fn sub_agent_completed(
    agent_id: &str,
    response: &str,
    duration_ms: u64,
    parent_request_id: &str,
) -> serde_json::Value {
    serde_json::json!({
        "agent_id": agent_id,
        "response": response,
        "duration_ms": duration_ms,
        "parent_request_id": parent_request_id
    })
}

pub(super) fn sub_agent_error(
    agent_id: &str,
    error: &str,
    parent_request_id: &str,
) -> serde_json::Value {
    serde_json::json!({
        "agent_id": agent_id,
        "error": error,
        "parent_request_id": parent_request_id
    })
}

pub(super) fn prompt_generation_started(
    agent_id: &str,
    parent_request_id: &str,
    architect_system_prompt: &str,
    architect_user_message: &str,
) -> serde_json::Value {
    serde_json::json!({
        "agent_id": agent_id,
        "parent_request_id": parent_request_id,
        "architect_system_prompt": architect_system_prompt,
        "architect_user_message": architect_user_message
    })
}

pub(super) fn prompt_generation_completed(
    agent_id: &str,
    parent_request_id: &str,
    generated_prompt: &Option<String>,
    success: bool,
    duration_ms: u64,
) -> serde_json::Value {
    serde_json::json!({
        "agent_id": agent_id,
        "parent_request_id": parent_request_id,
        "generated_prompt": generated_prompt,
        "success": success,
        "duration_ms": duration_ms
    })
}
