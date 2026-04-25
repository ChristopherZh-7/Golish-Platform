//! Display-formatting helpers: arg summarization, decision-type inference,
//! string + path truncation.

use super::super::events::DecisionType;


/// Summarize tool args for logging
pub(super) fn summarize_args(args: &serde_json::Value) -> String {
    let mut parts = Vec::new();

    if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
        parts.push(format!("path={}", truncate_path(path, 50)));
    }
    if let Some(desc) = args.get("display_description").and_then(|v| v.as_str()) {
        parts.push(format!("desc={}", truncate(desc, 40)));
    }
    if let Some(query) = args.get("query").and_then(|v| v.as_str()) {
        parts.push(format!("query={}", truncate(query, 30)));
    }
    if let Some(regex) = args.get("regex").and_then(|v| v.as_str()) {
        parts.push(format!("regex={}", truncate(regex, 30)));
    }

    if parts.is_empty() {
        // Fallback: show keys
        if let Some(obj) = args.as_object() {
            let keys: Vec<&str> = obj.keys().map(|s| s.as_str()).take(3).collect();
            format!("keys=[{}]", keys.join(", "))
        } else {
            "...".to_string()
        }
    } else {
        parts.join(", ")
    }
}

/// Infer decision type from reasoning content
pub(super) fn infer_decision_type(content: &str) -> Option<DecisionType> {
    let lower = content.to_lowercase();

    // Check for approach/strategy decisions
    if lower.contains("i'll use")
        || lower.contains("i will use")
        || lower.contains("let's use")
        || lower.contains("going with")
        || lower.contains("choosing")
        || lower.contains("decided to")
    {
        return Some(DecisionType::ApproachChoice);
    }

    // Check for tradeoff decisions
    if lower.contains("tradeoff")
        || lower.contains("trade-off")
        || lower.contains("balance between")
        || lower.contains("weighing")
    {
        return Some(DecisionType::Tradeoff);
    }

    // Check for fallback decisions
    if lower.contains("instead")
        || lower.contains("fallback")
        || lower.contains("alternative")
        || lower.contains("workaround")
    {
        return Some(DecisionType::Fallback);
    }

    // Check for assumptions
    if lower.contains("assuming") || lower.contains("i assume") || lower.contains("presumably") {
        return Some(DecisionType::Assumption);
    }

    None
}

/// Truncate a string to a maximum length
pub(super) fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        let mut end = max_len;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        &s[..end]
    }
}

/// Truncate a path string, keeping the end
pub(super) fn truncate_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len {
        path.to_string()
    } else {
        let keep = max_len.saturating_sub(3);
        format!("...{}", &path[path.len() - keep..])
    }
}

