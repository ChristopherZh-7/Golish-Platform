//! Construct the assistant content vector for chat history (reasoning,
//! text, tool calls in the order required by Anthropic + OpenAI Responses).

use rig::completion::{AssistantContent, Message};
use std::collections::BTreeSet;

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

    // Add tool calls. Guarantee `arguments` is a JSON object before it enters
    // history: some providers (e.g. Xiaomi MiMo) stream a bare scalar as the
    // whole arguments, which — replayed as a JSON string on the next turn —
    // crashes the provider's chat template (`arguments.items()` raises
    // `Can only get item pairs from a mapping`, surfacing as an HTTP 500).
    for tc in tool_calls {
        let mut tc = tc.clone();
        tc.function.arguments = golish_json_repair::ensure_tool_args_object(tc.function.arguments);
        content.push(AssistantContent::ToolCall(tc));
    }

    content
}

/// Serialize rig Message history to JSON for DB storage.
pub(crate) fn serialize_chat_history(messages: &[Message]) -> anyhow::Result<serde_json::Value> {
    // Full-fidelity serialization (rig `Message` is `Serialize`): preserves tool
    // calls AND tool results — so a later `resume` delegation can replay the
    // exact conversation, including the evidence ids that live in tool results
    // (the previous text-only encoding dropped them). A serialization failure is
    // a durability failure: callers must not publish a resume marker for an empty
    // replacement chain.
    validate_chat_history_tool_pairs(messages)?;
    Ok(serde_json::to_value(messages)?)
}

/// Inverse of [`serialize_chat_history`] for resumable sub-agent chains.
///
/// Deserialization mismatches are returned to the explicit-resume caller. A
/// corrupt/legacy row must not masquerade as an empty fresh conversation while
/// retaining the old chain id.
pub(crate) fn deserialize_chat_history(value: &serde_json::Value) -> anyhow::Result<Vec<Message>> {
    let messages = serde_json::from_value::<Vec<Message>>(value.clone())?;
    validate_chat_history_tool_pairs(&messages)?;
    Ok(messages)
}

/// Validate the provider-level invariant for resumable chat histories: every
/// assistant tool call must be followed immediately by a user tool-result turn
/// containing a result with the same provider call id.
///
/// A durable chain is replayed verbatim on resume. Persisting a dangling call
/// therefore turns a local control-flow bug into a permanently unresumable
/// chain (OpenAI-compatible providers reject it before generation). Keep this
/// strict and fail closed; ordinary user text must never stand in for a tool
/// result.
pub(crate) fn validate_chat_history_tool_pairs(messages: &[Message]) -> anyhow::Result<()> {
    for (index, message) in messages.iter().enumerate() {
        let Message::Assistant { content, .. } = message else {
            continue;
        };
        let tool_calls = content
            .iter()
            .filter_map(|item| match item {
                AssistantContent::ToolCall(call) => Some((
                    call.call_id.as_deref().unwrap_or(call.id.as_str()),
                    call.function.name.as_str(),
                )),
                _ => None,
            })
            .collect::<Vec<_>>();
        if tool_calls.is_empty() {
            continue;
        }

        let Some(Message::User { content: results }) = messages.get(index + 1) else {
            let missing = tool_calls
                .iter()
                .map(|(id, name)| format!("{name}:{id}"))
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::bail!(
                "assistant message {index} has tool call(s) without an immediate tool-result turn: {missing}"
            );
        };
        let result_ids = results
            .iter()
            .filter_map(|item| match item {
                UserContent::ToolResult(result) => {
                    Some(result.call_id.as_deref().unwrap_or(result.id.as_str()))
                }
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        let missing = tool_calls
            .iter()
            .filter(|(id, _)| !result_ids.contains(id))
            .map(|(id, name)| format!("{name}:{id}"))
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            anyhow::bail!(
                "assistant message {index} has tool call(s) without matching immediate results: {}",
                missing.join(", ")
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::message::{ToolFunction, ToolResult, ToolResultContent, UserContent};
    use rig::one_or_many::OneOrMany;

    fn tool_call(arguments: serde_json::Value) -> ToolCall {
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

    fn only_tool_call_args(content: &[AssistantContent]) -> &serde_json::Value {
        content
            .iter()
            .find_map(|c| match c {
                AssistantContent::ToolCall(tc) => Some(&tc.function.arguments),
                _ => None,
            })
            .expect("expected a tool call in assistant content")
    }

    #[test]
    fn bare_string_tool_args_are_coerced_to_object_in_history() {
        // The Xiaomi MiMo failure mode: a bare scalar streamed as the whole
        // arguments. If it reaches history as a JSON string, the next-turn
        // replay 500s the provider's chat template.
        let content = build_assistant_content(
            false,
            "",
            None,
            None,
            "",
            &[tool_call(serde_json::Value::String(
                "example.com".to_string(),
            ))],
        );
        assert!(
            only_tool_call_args(&content).is_object(),
            "string tool args must be coerced to an object before entering history"
        );
    }

    #[test]
    fn object_tool_args_pass_through_in_history() {
        let content = build_assistant_content(
            false,
            "",
            None,
            None,
            "",
            &[tool_call(serde_json::json!({"name": "10.0.0.5"}))],
        );
        assert_eq!(only_tool_call_args(&content)["name"], "10.0.0.5");
    }

    #[test]
    fn persisted_history_rejects_assistant_tool_call_without_immediate_result() {
        let call = tool_call(serde_json::json!({"name": "example"}));
        let messages = vec![
            Message::Assistant {
                id: None,
                content: OneOrMany::one(AssistantContent::ToolCall(call)),
            },
            Message::User {
                content: OneOrMany::one(UserContent::Text(Text {
                    text: "resume without the missing tool result".to_string(),
                })),
            },
        ];

        let error = validate_chat_history_tool_pairs(&messages)
            .expect_err("dangling tool calls must fail before persistence or resume");
        assert!(error.to_string().contains("t1"));
    }

    #[test]
    fn persisted_history_accepts_all_results_for_a_multi_tool_turn() {
        let first = tool_call(serde_json::json!({"name": "one"}));
        let mut second = tool_call(serde_json::json!({"name": "two"}));
        second.id = "t2".to_string();
        second.call_id = Some("t2".to_string());
        let result = |id: &str| {
            UserContent::ToolResult(ToolResult {
                id: id.to_string(),
                call_id: Some(id.to_string()),
                content: OneOrMany::one(ToolResultContent::Text(Text {
                    text: "{}".to_string(),
                })),
            })
        };
        let messages = vec![
            Message::Assistant {
                id: None,
                content: OneOrMany::many(vec![
                    AssistantContent::ToolCall(first),
                    AssistantContent::ToolCall(second),
                ])
                .expect("two assistant content items"),
            },
            Message::User {
                content: OneOrMany::many(vec![result("t1"), result("t2")])
                    .expect("two tool results"),
            },
        ];

        validate_chat_history_tool_pairs(&messages).expect("balanced tool history must be valid");
    }
}
