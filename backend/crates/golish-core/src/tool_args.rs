//! Helpers for normalizing streamed LLM tool-call arguments.
//!
//! Some OpenAI-compatible providers (e.g. Xiaomi MiMo) deliver
//! `function.arguments` as a raw JSON *string* — and often as a partial fragment
//! — across streaming chunks instead of a single complete object. Treating such
//! a fragment as a finished argument set dispatches malformed/empty args to tool
//! handlers (e.g. `search_memories` receiving `{"category": ` and failing with
//! "requires a non-empty 'query'").
//!
//! Both the main agentic loop and the sub-agent executor share these helpers so
//! their streaming behavior cannot diverge.

use serde_json::Value;

/// Returns `true` only when `args` is already a usable, fully-formed argument set.
///
/// Null, empty objects, and *string* values are treated as incomplete: a string
/// is a partial JSON fragment the provider is still streaming, so it must be
/// accumulated via tool-call deltas and parsed once complete — never dispatched
/// as-is.
pub fn has_complete_tool_args(args: &Value) -> bool {
    match args {
        Value::Null => false,
        Value::Object(map) if map.is_empty() => false,
        Value::String(_) => false,
        _ => true,
    }
}

/// Extracts the initial argument fragment used to seed delta accumulation from
/// the first tool-call chunk.
///
/// A streamed string fragment is returned verbatim so subsequent deltas append
/// to it; any other shape is serialized so accumulation has a consistent buffer.
pub fn initial_tool_args_fragment(args: &Value) -> String {
    match args {
        Value::Null => String::new(),
        Value::Object(map) if map.is_empty() => String::new(),
        Value::String(value) => value.clone(),
        value => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn string_args_are_incomplete_streaming_fragments() {
        // The exact shape observed leaking into search_memories: a truncated
        // JSON string the provider was still streaming.
        let args = json!("{\"category\": ");
        assert!(!has_complete_tool_args(&args));
        assert_eq!(initial_tool_args_fragment(&args), "{\"category\": ");
    }

    #[test]
    fn object_args_are_complete() {
        let args = json!({"query": "example.com recon", "limit": 10});
        assert!(has_complete_tool_args(&args));
    }

    #[test]
    fn null_and_empty_object_args_are_incomplete() {
        assert!(!has_complete_tool_args(&Value::Null));
        assert!(!has_complete_tool_args(&json!({})));
        assert_eq!(initial_tool_args_fragment(&Value::Null), "");
        assert_eq!(initial_tool_args_fragment(&json!({})), "");
    }
}
