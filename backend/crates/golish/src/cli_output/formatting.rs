// ────────────────────────────────────────────────────────────────────────────────
// Constants for terminal mode truncation
// ────────────────────────────────────────────────────────────────────────────────

/// Maximum characters for tool output in terminal mode
pub(super) const TERMINAL_TOOL_OUTPUT_MAX: usize = 500;

/// Maximum characters for reasoning content in terminal mode
pub(super) const TERMINAL_REASONING_MAX: usize = 2000;

// ────────────────────────────────────────────────────────────────────────────────
// Box-drawing constants for terminal output
// ────────────────────────────────────────────────────────────────────────────────

pub(super) const BOX_TOP: &str = "+-";
pub(super) const BOX_MID: &str = "|";
pub(super) const BOX_BOT: &str = "+-";

// ────────────────────────────────────────────────────────────────────────────────
// Helper functions
// ────────────────────────────────────────────────────────────────────────────────

/// Format a JSON value with pretty printing (indented).
pub(crate) fn format_json_pretty(value: &serde_json::Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

/// Truncate a string to a maximum number of characters.
///
/// This is used for terminal mode output only. JSON mode does NOT truncate.
/// Handles unicode correctly by iterating over chars rather than bytes.
pub(crate) fn truncate_output(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        s.chars().take(max_chars).collect()
    }
}

/// Format tool arguments for display (truncated summary).
///
/// NOTE: This is kept for backward compatibility in tests but is no longer
/// used in terminal output (we now show full tool inputs with `format_json_pretty`).
#[cfg(test)]
pub(crate) fn format_args_summary(args: &serde_json::Value) -> String {
    let s = args.to_string();
    if s.len() > 60 {
        format!("{}...", &s[..57])
    } else {
        s
    }
}

/// Truncate a string to a maximum length.
pub fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}
