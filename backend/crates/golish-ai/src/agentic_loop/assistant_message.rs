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
pub(super) fn push_assistant_message(
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
        assistant_content.push(AssistantContent::ToolCall(tool_call.clone()));
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
