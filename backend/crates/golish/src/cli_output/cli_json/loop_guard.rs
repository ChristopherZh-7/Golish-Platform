//! Loop-detection events: warnings, blocks, and max-iteration ceilings.

pub(super) fn loop_warning(
    tool_name: &str,
    current_count: usize,
    max_count: usize,
    message: &str,
) -> serde_json::Value {
    serde_json::json!({
        "tool_name": tool_name,
        "current_count": current_count,
        "max_count": max_count,
        "message": message
    })
}

pub(super) fn loop_blocked(
    tool_name: &str,
    repeat_count: usize,
    max_count: usize,
    message: &str,
) -> serde_json::Value {
    serde_json::json!({
        "tool_name": tool_name,
        "repeat_count": repeat_count,
        "max_count": max_count,
        "message": message
    })
}

pub(super) fn max_iterations_reached(
    iterations: usize,
    max_iterations: usize,
    message: &str,
) -> serde_json::Value {
    serde_json::json!({
        "iterations": iterations,
        "max_iterations": max_iterations,
        "message": message
    })
}
