//! Streamed assistant content events: incremental text deltas and reasoning.

pub(super) fn text_delta(delta: &str, accumulated: &str) -> serde_json::Value {
    serde_json::json!({
        "delta": delta,
        "accumulated": accumulated
    })
}

pub(super) fn reasoning(content: &str) -> serde_json::Value {
    serde_json::json!({ "content": content })
}
