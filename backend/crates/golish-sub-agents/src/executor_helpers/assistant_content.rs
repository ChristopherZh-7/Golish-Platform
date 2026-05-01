//! Construct the assistant content vector for chat history (reasoning,
//! text, tool calls in the order required by Anthropic + OpenAI Responses).

use rig::completion::{AssistantContent, Message};
use rig::message::{Reasoning, Text, ToolCall, UserContent};

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
    let entries: Vec<serde_json::Value> = messages
        .iter()
        .filter_map(|msg| {
            match msg {
                Message::System { content } => Some(serde_json::json!({
                    "role": "system",
                    "content": content,
                })),
                Message::User { content } => {
                    let texts: Vec<String> = content
                        .iter()
                        .filter_map(|c| match c {
                            UserContent::Text(t) => Some(t.text.clone()),
                            _ => None,
                        })
                        .collect();
                    if texts.is_empty() {
                        None
                    } else {
                        Some(serde_json::json!({
                            "role": "user",
                            "content": texts.join("\n"),
                        }))
                    }
                }
                Message::Assistant { content, .. } => {
                    let texts: Vec<String> = content
                        .iter()
                        .filter_map(|c| match c {
                            AssistantContent::Text(t) => Some(t.text.clone()),
                            _ => None,
                        })
                        .collect();
                    if texts.is_empty() {
                        None
                    } else {
                        Some(serde_json::json!({
                            "role": "assistant",
                            "content": texts.join("\n"),
                        }))
                    }
                }
            }
        })
        .collect();
    serde_json::json!(entries)
}
