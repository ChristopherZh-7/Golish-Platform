//! Top-level lifecycle and meta-status events.
//!
//! Started / UserMessage / Completed / Error / Warning.

pub(super) fn started(turn_id: &str) -> serde_json::Value {
    serde_json::json!({ "turn_id": turn_id })
}

pub(super) fn user_message(content: &str) -> serde_json::Value {
    serde_json::json!({ "content": content })
}

pub(super) fn completed(
    response: &str,
    reasoning: &Option<String>,
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
    duration_ms: Option<u64>,
) -> serde_json::Value {
    serde_json::json!({
        "response": response,
        "reasoning": reasoning,
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "duration_ms": duration_ms
    })
}

pub(super) fn error(message: &str, error_type: &str) -> serde_json::Value {
    serde_json::json!({
        "message": message,
        "error_type": error_type
    })
}

pub(super) fn warning(message: &str) -> serde_json::Value {
    serde_json::json!({ "message": message })
}
