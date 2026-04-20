//! Tool Call Auto-Fixer (PentAGI pattern).
//!
//! When a tool execution fails due to malformed arguments (correct JSON but
//! wrong schema — missing fields, wrong types, etc.), this module uses a
//! lightweight LLM call to repair the arguments and retry once.
//!
//! This is a *semantic* fixer, complementing `golish-json-repair` which
//! handles *syntactic* issues (unclosed braces, unquoted keys, etc.).

use rig::completion::{CompletionModel as RigCompletionModel, CompletionRequest};
use rig::message::{AssistantContent, Text, UserContent};
use rig::one_or_many::OneOrMany;
use serde_json::Value;

const FIXER_SYSTEM_PROMPT: &str = r#"# TOOL CALL ARGUMENT REPAIR SPECIALIST

You fix tool call arguments in JSON format according to the defined schema.

## INPUT STRUCTURE

The next message contains information about a failed tool call:

- <tool_call_name>: The function name that was called
- <tool_call_args>: The original JSON arguments
- <error_message>: The error that occurred
- <json_schema>: The expected JSON schema for the arguments

## RULES

- Make minimal changes to fix the identified error
- Preserve original content and intent
- Ensure output conforms to the provided schema
- Return a single line of valid JSON — nothing else

## OUTPUT

Your response must contain ONLY the fixed JSON with no additional text."#;

const MAX_FIXER_RETRIES: usize = 1;

/// Errors that suggest the arguments are semantically wrong (not just execution errors).
fn is_fixable_error(error_text: &str) -> bool {
    let lower = error_text.to_lowercase();
    lower.contains("missing field")
        || lower.contains("missing required")
        || lower.contains("invalid type")
        || lower.contains("unknown field")
        || lower.contains("expected")
        || lower.contains("invalid value")
        || lower.contains("deserialize")
        || lower.contains("parse error")
        || lower.contains("schema")
        || lower.contains("required property")
        || lower.contains("does not match")
        || lower.contains("wrong type")
}

/// Build the user message for the fixer LLM call.
fn build_fixer_prompt(tool_name: &str, args: &Value, error: &str, schema: &Value) -> String {
    format!(
        "<tool_call_name>{tool_name}</tool_call_name>\n\
         <tool_call_args>{args}</tool_call_args>\n\
         <error_message>{error}</error_message>\n\
         <json_schema>{schema}</json_schema>",
        tool_name = tool_name,
        args = serde_json::to_string_pretty(args).unwrap_or_default(),
        error = error,
        schema = serde_json::to_string_pretty(schema).unwrap_or_default(),
    )
}

/// Attempt to fix tool call arguments using a lightweight LLM call.
///
/// Returns `Some(fixed_args)` if the fixer succeeded, `None` if it couldn't help.
pub async fn try_fix_tool_args<M>(
    model: &M,
    tool_name: &str,
    original_args: &Value,
    error_message: &str,
    tool_schema: Option<&Value>,
) -> Option<Value>
where
    M: RigCompletionModel + Sync,
{
    if !is_fixable_error(error_message) {
        tracing::debug!(
            "[toolcall-fixer] Error not fixable by semantic repair: {}",
            &error_message[..error_message.len().min(100)]
        );
        return None;
    }

    let schema = tool_schema.cloned().unwrap_or_else(|| serde_json::json!({}));
    let user_message = build_fixer_prompt(tool_name, original_args, error_message, &schema);

    let request = CompletionRequest {
        model: None,
        preamble: Some(FIXER_SYSTEM_PROMPT.to_string()),
        chat_history: OneOrMany::one(rig::completion::Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: user_message,
            })),
        }),
        documents: vec![],
        tools: vec![],
        temperature: Some(0.0),
        max_tokens: Some(4096),
        tool_choice: None,
        additional_params: None,
        output_schema: None,
    };

    match model.completion(request).await {
        Ok(response) => {
            let text = extract_text(&response.choice);
            let trimmed = text.trim();
            match serde_json::from_str::<Value>(trimmed) {
                Ok(fixed) => {
                    if &fixed != original_args {
                        tracing::info!(
                            "[toolcall-fixer] Successfully repaired args for '{}'",
                            tool_name
                        );
                        Some(fixed)
                    } else {
                        tracing::debug!(
                            "[toolcall-fixer] Fixer returned identical args for '{}'",
                            tool_name
                        );
                        None
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "[toolcall-fixer] Fixer returned invalid JSON: {}",
                        e
                    );
                    golish_json_repair::parse_tool_args_opt(trimmed)
                        .filter(|v| v != original_args)
                }
            }
        }
        Err(e) => {
            tracing::warn!("[toolcall-fixer] LLM call failed: {}", e);
            None
        }
    }
}

fn extract_text(choice: &OneOrMany<AssistantContent>) -> String {
    choice
        .iter()
        .filter_map(|c| match c {
            AssistantContent::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Maximum number of fixer retry attempts.
pub fn max_fixer_retries() -> usize {
    MAX_FIXER_RETRIES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_fixable_error() {
        assert!(is_fixable_error("missing field `target`"));
        assert!(is_fixable_error("invalid type: expected string, got integer"));
        assert!(is_fixable_error("failed to deserialize arguments"));
        assert!(is_fixable_error("required property 'command' is missing"));

        assert!(!is_fixable_error("connection refused"));
        assert!(!is_fixable_error("permission denied"));
        assert!(!is_fixable_error("file not found"));
        assert!(!is_fixable_error("timeout"));
    }

    #[test]
    fn test_build_fixer_prompt() {
        let args = serde_json::json!({"target": 123});
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "target": {"type": "string"}
            },
            "required": ["target"]
        });

        let prompt = build_fixer_prompt("scan_target", &args, "invalid type: expected string", &schema);
        assert!(prompt.contains("scan_target"));
        assert!(prompt.contains("invalid type"));
        assert!(prompt.contains("\"target\""));
    }
}
