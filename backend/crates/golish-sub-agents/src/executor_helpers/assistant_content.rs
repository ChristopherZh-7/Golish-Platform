//! Construct the assistant content vector for chat history (reasoning,
//! text, tool calls in the order required by Anthropic + OpenAI Responses).

use rig::completion::{AssistantContent, Message};
use rig::message::{Reasoning, Text, ToolCall};

/// Build assistant content for chat history with proper ordering.
///
/// When thinking is enabled, thinking blocks MUST come first (required by Anthropic API).
/// This function ensures the correct order: Reasoning -> Text -> ToolCalls
///
/// # Arguments
/// * `supports_thinking_history` - Whether the model supports thinking history
/// * `thinking_text` - The accumulated thinking/reasoning text
/// * `thinking_id` - Optional reasoning ID (used by OpenAI Responses API)
/// * `thinking_signature` - Optional thinking signature (used by Anthropic)
/// * `text_content` - The text response content
/// * `tool_calls` - List of tool calls to include
///
/// # Returns
/// A vector of AssistantContent in the correct order for the API
pub fn build_assistant_content(
    supports_thinking_history: bool,
    thinking_text: &str,
    thinking_id: Option<String>,
    thinking_signature: Option<String>,
    text_content: &str,
    tool_calls: &[ToolCall],
) -> Vec<AssistantContent> {
    let mut content: Vec<AssistantContent> = vec![];

    // Add thinking content FIRST (required by Anthropic API when thinking is enabled)
    let has_reasoning = !thinking_text.is_empty() || thinking_id.is_some();
    if supports_thinking_history && has_reasoning {
        content.push(AssistantContent::Reasoning(
            Reasoning::new_with_signature(thinking_text, thinking_signature)
                .optional_id(thinking_id),
        ));
    }

    // Add text content
    if !text_content.is_empty() {
        content.push(AssistantContent::Text(Text {
            text: text_content.to_string(),
        }));
    }

    // Add tool calls
    for tc in tool_calls {
        content.push(AssistantContent::ToolCall(tc.clone()));
    }

    content
}

/// Serialize rig Message history to JSON for DB storage.
pub(crate) fn serialize_chat_history(messages: &[Message]) -> serde_json::Value {
    // Full-fidelity serialization (rig `Message` is `Serialize`): preserves tool
    // calls AND tool results — so a later `resume` delegation can replay the
    // exact conversation, including the evidence ids that live in tool results
    // (the previous text-only encoding dropped them). Falls back to an empty
    // array if serialization fails, so chain persistence never panics.
    serde_json::to_value(messages).unwrap_or_else(|_| serde_json::json!([]))
}

/// Inverse of [`serialize_chat_history`] for resumable sub-agent chains.
///
/// Returns an empty history on any deserialization mismatch (e.g. a row written
/// by the older, lossy text-only encoding), so `resume` degrades gracefully to
/// a fresh conversation instead of failing the sub-agent.
pub(crate) fn deserialize_chat_history(value: &serde_json::Value) -> Vec<Message> {
    serde_json::from_value::<Vec<Message>>(value.clone()).unwrap_or_default()
}
