//! Context-window management events: warnings, compaction, and system-hook
//! injections.

pub(super) fn context_warning(
    utilization: f64,
    total_tokens: usize,
    max_tokens: usize,
) -> serde_json::Value {
    serde_json::json!({
        "utilization": utilization,
        "total_tokens": total_tokens,
        "max_tokens": max_tokens
    })
}

pub(super) fn compaction_started(
    tokens_before: u64,
    messages_before: usize,
) -> serde_json::Value {
    serde_json::json!({
        "tokens_before": tokens_before,
        "messages_before": messages_before
    })
}

pub(super) fn compaction_completed(
    tokens_before: u64,
    messages_before: usize,
    messages_after: usize,
    summary_length: usize,
) -> serde_json::Value {
    serde_json::json!({
        "tokens_before": tokens_before,
        "messages_before": messages_before,
        "messages_after": messages_after,
        "summary_length": summary_length
    })
}

pub(super) fn compaction_failed(
    tokens_before: u64,
    messages_before: usize,
    error: &str,
) -> serde_json::Value {
    serde_json::json!({
        "tokens_before": tokens_before,
        "messages_before": messages_before,
        "error": error
    })
}

pub(super) fn system_hooks_injected(hooks: &[String]) -> serde_json::Value {
    serde_json::json!({
        "hooks": hooks
    })
}
