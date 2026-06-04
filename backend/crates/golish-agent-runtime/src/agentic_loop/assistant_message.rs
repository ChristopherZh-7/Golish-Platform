//! Build the per-iteration assistant message (text + reasoning + tool calls)
//! and push it into the chat history.
//!
//! Reasoning items are conditionally included depending on provider:
//!
//! - `openai_reasoning` (gpt-5.2 / Codex / o-series via rig-openai-responses):
//!   Always include reasoning when present. The OpenAI Responses API tracks
//!   `rs_...` IDs server-side and requires them to be echoed back in every
//!   subsequent turn. A reasoning item MUST be followed by the next output item
//!   (text or tool call); omitting it produces:
//!   `Item 'rs_...' of type 'reasoning' was provided without its required following item`.
//!
//! - `openai_responses` (rig-core built-in non-reasoning models on the Responses API):
//!   Only include reasoning when paired with a tool call. Without a following
//!   `function_call`, the API rejects the request:
//!   `reasoning was provided without its required following item`.
//!
//! - Other providers (Anthropic, ...): include reasoning when present.
//!
//! When thinking is enabled, the reasoning block MUST come first in the
//! assistant content vector (required by the Anthropic API).

use rig::completion::{AssistantContent, Message};
use rig::message::{Reasoning, Text, ToolCall};
use rig::one_or_many::OneOrMany;

/// Build the assistant content for one iteration and append it to `chat_history`.
///
/// Always pushes the assistant message even when content is otherwise empty
/// (matters for maintaining conversation context across turns).
pub(crate) fn push_assistant_message(
    chat_history: &mut Vec<Message>,
    text_content: &str,
    thinking_content: &str,
    thinking_signature: &Option<String>,
    thinking_id: &Option<String>,
    tool_calls_to_execute: &[ToolCall],
    has_tool_calls: bool,
    supports_thinking: bool,
    provider_name: &str,
) {
    let mut assistant_content: Vec<AssistantContent> = Vec::new();

    let is_openai_reasoning_provider = provider_name == "openai_reasoning";
    let is_openai_responses_api = provider_name == "openai_responses";
    let has_reasoning = !thinking_content.is_empty() || thinking_id.is_some();

    let should_include_reasoning = if is_openai_reasoning_provider {
        // Always include reasoning for openai_reasoning — rs_ IDs must be echoed back
        has_reasoning
    } else if is_openai_responses_api {
        // For openai_responses: only include reasoning when paired with a tool call
        has_reasoning && has_tool_calls
    } else {
        // For other providers (Anthropic, ...): include reasoning when present
        has_reasoning
    };

    if supports_thinking && should_include_reasoning {
        tracing::info!(
            "[OpenAI Debug] Building assistant content with reasoning: id={:?}, signature_len={:?}",
            thinking_id,
            thinking_signature.as_ref().map(|s| s.len())
        );
        assistant_content.push(AssistantContent::Reasoning(
            Reasoning::new_with_signature(thinking_content, thinking_signature.clone())
                .optional_id(thinking_id.clone()),
        ));
    }

    if !text_content.is_empty() {
        assistant_content.push(AssistantContent::Text(Text {
            text: text_content.to_string(),
        }));
    }

    for tool_call in tool_calls_to_execute {
        assistant_content.push(AssistantContent::ToolCall(normalize_tool_call_for_history(
            tool_call,
        )));
    }

    // ALWAYS add assistant message to history (even when no tool calls).
    // This is critical for maintaining conversation context across turns.
    if !assistant_content.is_empty() {
        chat_history.push(Message::Assistant {
            id: None,
            content: OneOrMany::many(assistant_content).unwrap_or_else(|_| {
                OneOrMany::one(AssistantContent::Text(Text {
                    text: String::new(),
                }))
            }),
        });
    }
}

/// Guarantee a tool call's `arguments` is a JSON **object** before it enters
/// chat history. Tool arguments are objects by contract, but some providers
/// (e.g. Xiaomi MiMo) stream a bare scalar that, replayed as a JSON string on
/// the next turn, crashes the provider's chat template — `arguments.items()`
/// raises `Can only get item pairs from a mapping` (HTTP 500). Coercing to an
/// object here keeps history replay valid for strict providers.
fn normalize_tool_call_for_history(tool_call: &ToolCall) -> ToolCall {
    let mut normalized = tool_call.clone();
    normalized.function.arguments =
        golish_json_repair::ensure_tool_args_object(normalized.function.arguments);
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::message::ToolFunction;

    fn tool_call_with_args(arguments: serde_json::Value) -> ToolCall {
        ToolCall {
            id: "t1".to_string(),
            call_id: Some("t1".to_string()),
            function: ToolFunction {
                name: "graph_add_entity".to_string(),
                arguments,
            },
            signature: None,
            additional_params: None,
        }
    }

    #[test]
    fn bare_string_arguments_become_object_for_history() {
        let tc = tool_call_with_args(serde_json::Value::String("example.com".to_string()));
        let normalized = normalize_tool_call_for_history(&tc);
        assert!(
            normalized.function.arguments.is_object(),
            "string args must be coerced to an object so MiMo history replay does not 500"
        );
    }

    #[test]
    fn json_object_string_arguments_are_recovered_for_history() {
        let tc = tool_call_with_args(serde_json::Value::String(
            r#"{"entity_type": "host", "name": "10.0.0.5"}"#.to_string(),
        ));
        let normalized = normalize_tool_call_for_history(&tc);
        assert_eq!(normalized.function.arguments["entity_type"], "host");
        assert_eq!(normalized.function.arguments["name"], "10.0.0.5");
    }

    #[test]
    fn object_arguments_pass_through_for_history() {
        let tc = tool_call_with_args(serde_json::json!({"name": "x"}));
        let normalized = normalize_tool_call_for_history(&tc);
        assert_eq!(normalized.function.arguments["name"], "x");
    }
}
