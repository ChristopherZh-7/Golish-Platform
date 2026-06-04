//! JSON repair utilities for LLM tool call arguments
//!
//! LLMs (especially GLM models) sometimes produce malformed JSON that fails
//! to parse. This module provides repair functionality using the llm_json crate.

use serde_json::Value;
use tracing::debug;

/// Parse tool call arguments with automatic repair for malformed JSON.
///
/// Attempts standard parsing first, then falls back to repair if that fails.
/// Returns empty object `{}` if both parsing and repair fail.
pub fn parse_tool_args(args: &str) -> Value {
    // Fast path: try standard parsing first
    if let Ok(value) = serde_json::from_str(args) {
        return value;
    }

    // Slow path: attempt repair
    debug!("JSON parse failed, attempting repair");
    repair_and_parse(args).unwrap_or_else(|| {
        debug!("JSON repair failed, returning empty object");
        serde_json::json!({})
    })
}

/// Parse tool-call arguments and guarantee a JSON **object**.
///
/// Tool-call arguments are objects by the OpenAI/Anthropic contract. Some
/// OpenAI-compatible providers (notably Xiaomi MiMo) stream a bare scalar — e.g.
/// `example.com` — as the entire `function.arguments`. Left as a JSON string it
/// serializes back onto the wire as a string and crashes the provider's Jinja
/// chat template on the *next* turn: `arguments.items()` raises
/// `Can only get item pairs from a mapping` (HTTP 500) during history replay.
///
/// Object parses pass through unchanged; any non-object parse collapses to `{}`.
pub fn parse_tool_args_object(args: &str) -> Value {
    ensure_tool_args_object(parse_tool_args(args))
}

/// Coerce an already-parsed value into a tool-call arguments **object**.
///
/// - `Object` → unchanged.
/// - `String` → re-parsed; kept only if it yields an object, otherwise `{}`.
/// - anything else (array / number / bool / null) → `{}`.
///
/// See [`parse_tool_args_object`] for why a non-object would break MiMo replay.
pub fn ensure_tool_args_object(args: Value) -> Value {
    match args {
        Value::Object(_) => args,
        // A string may itself be a streamed JSON-object fragment ("{...}"): try
        // to recover it; keep the parse only when it lands on an object.
        Value::String(s) => {
            let parsed = parse_tool_args(&s);
            if parsed.is_object() {
                parsed
            } else {
                Value::Object(serde_json::Map::new())
            }
        }
        // array / number / bool / null are never valid tool arguments.
        _ => Value::Object(serde_json::Map::new()),
    }
}

/// Parse tool call arguments, returning None on failure instead of default.
///
/// Useful when you need to handle parse failures explicitly.
pub fn parse_tool_args_opt(args: &str) -> Option<Value> {
    // Fast path: try standard parsing first
    if let Ok(value) = serde_json::from_str(args) {
        return Some(value);
    }

    // Slow path: attempt repair
    repair_and_parse(args)
}

/// Repair malformed JSON string and return the fixed string.
///
/// Returns None if repair fails.
pub fn repair_json(args: &str) -> Option<String> {
    llm_json::repair_json(args, &Default::default()).ok()
}

/// Repair and parse JSON in one step.
fn repair_and_parse(args: &str) -> Option<Value> {
    match llm_json::loads(args, &Default::default()) {
        Ok(value) => {
            debug!("JSON repair succeeded");
            Some(value)
        }
        Err(e) => {
            debug!("JSON repair failed: {}", e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_valid_json_passthrough() {
        let json = r#"{"name": "test", "value": 123}"#;
        let result = parse_tool_args(json);
        assert_eq!(result["name"], "test");
        assert_eq!(result["value"], 123);
    }

    #[test]
    fn test_unquoted_keys() {
        let json = r#"{name: "test", value: 123}"#;
        let result = parse_tool_args(json);
        assert_eq!(result["name"], "test");
    }

    #[test]
    fn test_single_quotes() {
        let json = r#"{'name': 'test'}"#;
        let result = parse_tool_args(json);
        assert_eq!(result["name"], "test");
    }

    #[test]
    fn test_trailing_comma() {
        let json = r#"{"name": "test",}"#;
        let result = parse_tool_args(json);
        assert_eq!(result["name"], "test");
    }

    #[test]
    fn test_python_booleans() {
        let json = r#"{"active": True, "disabled": False}"#;
        let result = parse_tool_args(json);
        assert_eq!(result["active"], true);
        assert_eq!(result["disabled"], false);
    }

    #[test]
    fn test_unclosed_object() {
        let json = r#"{"name": "test""#;
        let result = parse_tool_args(json);
        // Should repair by closing the object
        assert_eq!(result["name"], "test");
    }

    #[test]
    fn test_missing_value_quotes() {
        // This pattern was seen in GLM output
        let json = r#"{"explanation":Explore notification-related code}"#;
        let result = parse_tool_args(json);
        // Should repair the unquoted string value
        assert!(!result.is_null());
    }

    #[test]
    fn test_invalid_returns_something() {
        // Note: llm_json is very aggressive at repair and may produce
        // unexpected results from truly malformed input. The important
        // thing is that it doesn't panic.
        let json = "not json at all {{{";
        let result = parse_tool_args(json);
        // Result may be repaired unexpectedly; just verify we get a value
        assert!(result.is_object() || result.is_null());
    }

    #[test]
    fn object_args_object_passes_through() {
        let v = parse_tool_args_object(r#"{"command": "dig example.com"}"#);
        assert!(v.is_object());
        assert_eq!(v["command"], "dig example.com");
    }

    #[test]
    fn bare_scalar_arg_collapses_to_object() {
        // The exact MiMo failure: a single-chunk bare scalar handed in as the
        // whole arguments. Must become an object so a history replay does not
        // 500 the provider's chat template.
        let v = parse_tool_args_object("example.com");
        assert!(v.is_object(), "expected object, got {v:?}");
    }

    #[test]
    fn json_string_literal_arg_collapses_to_object() {
        let v = parse_tool_args_object(r#""example.com""#);
        assert!(v.is_object(), "expected object, got {v:?}");
    }

    #[test]
    fn null_array_empty_args_collapse_to_object() {
        assert!(parse_tool_args_object("null").is_object());
        assert!(parse_tool_args_object("[1, 2, 3]").is_object());
        assert!(parse_tool_args_object("").is_object());
    }

    #[test]
    fn ensure_object_coerces_each_value_variant() {
        assert_eq!(ensure_tool_args_object(serde_json::json!("x")), json!({}));
        assert_eq!(ensure_tool_args_object(serde_json::json!(123)), json!({}));
        assert_eq!(
            ensure_tool_args_object(serde_json::json!([1, 2])),
            json!({})
        );
        assert_eq!(ensure_tool_args_object(serde_json::json!(null)), json!({}));
        assert_eq!(
            ensure_tool_args_object(json!({"a": 1})),
            json!({"a": 1}),
            "objects must pass through unchanged"
        );
        // A string that itself contains a JSON object is recovered to the object.
        assert_eq!(
            ensure_tool_args_object(serde_json::json!("{\"a\": 1}")),
            json!({"a": 1})
        );
    }
}
