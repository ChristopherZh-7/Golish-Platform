use serde_json::json;

/// Result type for tool execution: (json_result, success_flag)
pub type ToolResult = (serde_json::Value, bool);

/// Helper to create an error result
pub fn error_result(msg: impl Into<String>) -> ToolResult {
    (json!({"error": msg.into()}), false)
}

/// Try to extract a string parameter from args, checking multiple possible key names.
/// Handles models that pass null, numbers, or alternative key names.
pub fn extract_string_param(args: &serde_json::Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(val) = args.get(*key) {
            if let Some(s) = val.as_str() {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            } else if !val.is_null() {
                let s = val.to_string();
                let s = s.trim().trim_matches('"');
                if !s.is_empty() && s != "null" {
                    return Some(s.to_string());
                }
            }
        }
    }
    None
}
