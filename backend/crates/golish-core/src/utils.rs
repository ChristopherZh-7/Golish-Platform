//! Utility functions for common operations.

/// Truncates a string to at most `max_bytes` bytes, respecting UTF-8 character boundaries.
///
/// This function ensures the truncation point falls on a valid UTF-8 character boundary,
/// preventing panics that would occur from slicing in the middle of a multi-byte character.
///
/// # Arguments
/// * `s` - The string to truncate
/// * `max_bytes` - Maximum number of bytes in the result
///
/// # Returns
/// A string slice that is at most `max_bytes` bytes long, ending at a valid character boundary.
///
/// # Example
/// ```
/// # use golish_core::utils::truncate_str;
/// let s = "Hello, 世界!"; // "世" and "界" are 3 bytes each
/// assert_eq!(truncate_str(s, 10), "Hello, 世");
/// assert_eq!(truncate_str(s, 7), "Hello, ");
/// assert_eq!(truncate_str(s, 100), s); // No truncation needed
/// ```
pub fn truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }

    // Find the last character boundary at or before max_bytes
    // char_indices() yields (byte_position, char) for each character
    let mut end = 0;
    for (idx, _) in s.char_indices() {
        if idx > max_bytes {
            break;
        }
        end = idx;
    }

    // Handle edge case: if first char is already beyond max_bytes, return empty
    // Also handle the case where we stopped at a boundary that fits
    if end == 0 && s.len() > max_bytes {
        // Check if first character fits
        if let Some((first_char_end, _)) = s.char_indices().nth(1) {
            if first_char_end <= max_bytes {
                end = first_char_end;
            }
        } else if s.len() <= max_bytes {
            // Single character string that fits
            return s;
        }
    }

    // Get the byte position of the next character (or end of string)
    // to include the character at position `end`
    let actual_end = s[end..]
        .char_indices()
        .nth(1)
        .map(|(idx, _)| end + idx)
        .unwrap_or(s.len());

    if actual_end <= max_bytes {
        &s[..actual_end]
    } else {
        &s[..end]
    }
}

/// Truncate to at most `max_chars` Unicode scalar values (head-keeping, no
/// marker). The char-counted companion to the byte-counted [`truncate_str`];
/// single source of truth for the `truncate_*` helpers previously copied into
/// `golish-cli-output`, `golish-agent-runtime`, `golish` and `golish-shell-exec`.
pub fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        s.chars().take(max_chars).collect()
    }
}

/// Truncate using a 70/30 head/tail strategy, preserving both start and end.
pub fn truncate_head_tail(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let head_len = (max_bytes as f64 * 0.7) as usize;
    let tail_len = max_bytes.saturating_sub(head_len);
    let head = truncate_str(s, head_len);
    let tail_start = s.len().saturating_sub(tail_len);
    let mut tail_boundary = tail_start;
    while tail_boundary < s.len() && !s.is_char_boundary(tail_boundary) {
        tail_boundary += 1;
    }
    let tail = &s[tail_boundary..];
    format!(
        "{}\n\n... [truncated {} chars] ...\n\n{}",
        head,
        s.len() - head.len() - tail.len(),
        tail
    )
}

/// Strip ANSI / VT escape sequences (`ESC [ … <letter>`) from a string.
///
/// Single source of truth for the `strip_ansi` helpers previously copied into
/// `golish-db` and `golish-agent-kit`.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip ESC [ ... <letter> sequences
            if chars.peek() == Some(&'[') {
                chars.next();
                for nc in chars.by_ref() {
                    if nc.is_ascii_alphabetic() || nc == 'm' {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Extract a required string argument from a JSON `Value` object.
/// Returns the string reference or a JSON error value.
pub fn get_required_str<'a>(
    args: &'a serde_json::Value,
    key: &str,
) -> Result<&'a str, serde_json::Value> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| serde_json::json!({"error": format!("Missing required argument: {}", key)}))
}

/// Extract an optional string argument from a JSON `Value` object.
pub fn get_optional_str<'a>(args: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}

pub fn get_optional_u64(args: &serde_json::Value, key: &str) -> Option<u64> {
    args.get(key).and_then(|v| v.as_u64())
}

pub fn get_optional_i64(args: &serde_json::Value, key: &str) -> Option<i64> {
    args.get(key).and_then(|v| v.as_i64())
}

pub fn get_optional_bool(args: &serde_json::Value, key: &str) -> Option<bool> {
    args.get(key).and_then(|v| v.as_bool())
}

pub fn get_optional_usize(args: &serde_json::Value, key: &str) -> Option<usize> {
    args.get(key).and_then(|v| v.as_u64()).map(|n| n as usize)
}

pub fn get_optional_u32(args: &serde_json::Value, key: &str) -> Option<u32> {
    args.get(key).and_then(|v| v.as_u64()).map(|n| n as u32)
}

/// Check whether a tool's JSON result represents a successful execution.
///
/// A result is considered a failure if:
/// - It has a non-zero `exit_code` field
/// - It has an `"error"` field present
pub fn is_tool_result_success(value: &serde_json::Value) -> bool {
    let is_failure_by_exit_code = value
        .get("exit_code")
        .and_then(|ec| ec.as_i64())
        .map(|ec| ec != 0)
        .unwrap_or(false);
    let has_error_field = value.get("error").is_some();
    // Explicit boolean failure flags some tools surface without an `error` key or a
    // non-zero exit code (`success`/`ok` false, `is_error` true).
    let explicit_failure_flag = value.get("success").and_then(|v| v.as_bool()) == Some(false)
        || value.get("ok").and_then(|v| v.as_bool()) == Some(false)
        || value.get("is_error").and_then(|v| v.as_bool()) == Some(true);
    // Terminal failure conveyed via a `status` string (e.g. submit_stage_deliverable
    // returns "rejected"/"needs_fix"); matched case-insensitively.
    let is_failure_by_status = value
        .get("status")
        .and_then(|v| v.as_str())
        .map(|s| {
            matches!(
                s.trim().to_ascii_lowercase().as_str(),
                "rejected" | "needs_fix" | "error" | "failed"
            )
        })
        .unwrap_or(false);
    !is_failure_by_exit_code && !has_error_field && !explicit_failure_flag && !is_failure_by_status
}

/// Simple JSONPath-like resolver: supports `$.foo.bar` and `$.foo[0].bar` patterns.
/// Strips leading `$.` prefix. Returns `None` for null or empty arrays.
pub fn resolve_json_path(val: &serde_json::Value, path: &str) -> Option<String> {
    let path = path.strip_prefix("$.").unwrap_or(path);
    let mut current = val;
    for segment in path.split('.') {
        if let Some(idx_start) = segment.find('[') {
            let key = &segment[..idx_start];
            let idx_str = &segment[idx_start + 1..segment.len() - 1];
            if !key.is_empty() {
                current = current.get(key)?;
            }
            let idx: usize = idx_str.parse().ok()?;
            current = current.get(idx)?;
        } else {
            current = current.get(segment)?;
        }
    }
    match current {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        serde_json::Value::Null => None,
        serde_json::Value::Array(arr) if arr.is_empty() => None,
        other => Some(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_str_ascii() {
        let s = "Hello, World!";
        assert_eq!(truncate_str(s, 5), "Hello");
        assert_eq!(truncate_str(s, 7), "Hello, ");
        assert_eq!(truncate_str(s, 100), s);
        assert_eq!(truncate_str(s, 0), "");
    }

    #[test]
    fn test_truncate_str_unicode() {
        // Each CJK character is 3 bytes
        let s = "Hello, 世界!";
        assert_eq!(truncate_str(s, 10), "Hello, 世"); // 7 + 3 = 10
        assert_eq!(truncate_str(s, 9), "Hello, "); // Can't fit 世 (3 bytes)
        assert_eq!(truncate_str(s, 8), "Hello, "); // Can't fit 世 (3 bytes)
        assert_eq!(truncate_str(s, 7), "Hello, ");
    }

    #[test]
    fn test_truncate_str_box_drawing() {
        // Box drawing character ─ is 3 bytes (the one that caused the original panic)
        let s = "Result: ─────";
        assert_eq!(truncate_str(s, 8), "Result: ");
        assert_eq!(truncate_str(s, 11), "Result: ─"); // 8 + 3 = 11
        assert_eq!(truncate_str(s, 10), "Result: "); // 8 + 2 not enough for ─
    }

    #[test]
    fn test_truncate_str_emoji() {
        // Emoji can be 4 bytes
        let s = "Hi 👋 there";
        assert_eq!(truncate_str(s, 3), "Hi ");
        assert_eq!(truncate_str(s, 7), "Hi 👋"); // 3 + 4 = 7
        assert_eq!(truncate_str(s, 6), "Hi "); // Can't fit emoji
    }

    #[test]
    fn test_truncate_str_empty() {
        assert_eq!(truncate_str("", 10), "");
        assert_eq!(truncate_str("", 0), "");
    }

    #[test]
    fn test_truncate_str_exact_boundary() {
        let s = "abc";
        assert_eq!(truncate_str(s, 3), "abc");
        assert_eq!(truncate_str(s, 2), "ab");
        assert_eq!(truncate_str(s, 1), "a");
    }

    #[test]
    fn test_truncate_chars_counts_scalars_not_bytes() {
        // Each CJK char is 1 scalar (3 bytes) — char-counted, not byte-counted.
        let s = "Hello, 世界!";
        assert_eq!(truncate_chars(s, 100), s);
        assert_eq!(truncate_chars(s, 8), "Hello, 世");
        assert_eq!(truncate_chars(s, 0), "");
        assert_eq!(truncate_chars("", 5), "");
    }

    #[test]
    fn test_strip_ansi_removes_color_and_keeps_text() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m text"), "red text");
        assert_eq!(strip_ansi("plain"), "plain");
        assert_eq!(strip_ansi("\x1b[1;32mok\x1b[0m"), "ok");
        assert_eq!(strip_ansi(""), "");
    }

    // ── is_tool_result_success: semantic failure detection ──

    #[test]
    fn tool_result_failure_on_terminal_failure_status() {
        // submit_stage_deliverable et al. report failure via a `status` field with
        // no `error` key and no exit_code — these must read as failure (UI ❌).
        for s in ["rejected", "needs_fix", "error", "failed"] {
            assert!(
                !is_tool_result_success(&serde_json::json!({ "status": s })),
                "status={s} must be a failure"
            );
        }
        // Case-insensitive (recon_discover_subsidiaries returns "Failed").
        assert!(!is_tool_result_success(
            &serde_json::json!({ "status": "Failed" })
        ));
        assert!(!is_tool_result_success(
            &serde_json::json!({ "status": "NEEDS_FIX" })
        ));
    }

    #[test]
    fn tool_result_failure_on_explicit_false_or_error_flags() {
        assert!(!is_tool_result_success(
            &serde_json::json!({ "success": false })
        ));
        assert!(!is_tool_result_success(&serde_json::json!({ "ok": false })));
        assert!(!is_tool_result_success(
            &serde_json::json!({ "is_error": true })
        ));
    }

    #[test]
    fn tool_result_failure_on_exit_code_or_error_field() {
        // Existing contract stays intact.
        assert!(!is_tool_result_success(
            &serde_json::json!({ "exit_code": 127 })
        ));
        assert!(!is_tool_result_success(
            &serde_json::json!({ "error": "boom" })
        ));
    }

    #[test]
    fn tool_result_success_for_non_failure_results() {
        // Non-failure status values and clean results stay success (no false ❌).
        assert!(is_tool_result_success(
            &serde_json::json!({ "status": "completed" })
        ));
        assert!(is_tool_result_success(
            &serde_json::json!({ "status": "accepted" })
        ));
        assert!(is_tool_result_success(
            &serde_json::json!({ "success": true })
        ));
        assert!(is_tool_result_success(
            &serde_json::json!({ "is_error": false })
        ));
        assert!(is_tool_result_success(
            &serde_json::json!({ "stdout": "ok", "exit_code": 0 })
        ));
        assert!(is_tool_result_success(&serde_json::json!({})));
    }
}
