//! AssistantPush phase — append the per-iteration assistant message
//! (text + reasoning + tool calls) to the chat history.
//!
//! Today this is a thin pass-through to `push_assistant_message` in the
//! sibling `assistant_message` module. Lifting it into a phase keeps the
//! main loop strictly composed of phase calls and provides a stable
//! insertion point for future hooks (provider-specific reasoning
//! trimming, message validation, etc.) when C1-7 introduces the
//! `TurnInterceptor` trait.

use rig::completion::Message;
use rig::message::ToolCall;

use super::super::super::assistant_message::push_assistant_message;
use super::super::super::context::AgenticLoopContext;

/// Append the assistant message produced by this iteration to
/// `chat_history`. Provider-specific reasoning rules live inside
/// `push_assistant_message`.
#[allow(clippy::too_many_arguments)]
pub fn run(
    chat_history: &mut Vec<Message>,
    text_content: &str,
    thinking_content: &str,
    thinking_signature: &Option<String>,
    thinking_id: &Option<String>,
    tool_calls_to_execute: &[ToolCall],
    has_tool_calls: bool,
    supports_thinking: bool,
    ctx: &AgenticLoopContext<'_>,
) {
    push_assistant_message(
        chat_history,
        text_content,
        thinking_content,
        thinking_signature,
        thinking_id,
        tool_calls_to_execute,
        has_tool_calls,
        supports_thinking,
        ctx.llm.provider_name,
    );
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use golish_llm_providers::LlmClient;
    use rig::completion::AssistantContent;
    use rig::message::ToolFunction;
    use serde_json::json;
    use tokio::sync::RwLock;

    use crate::test_utils::TestContextBuilder;

    use super::*;

    fn make_tool_call(name: &str) -> ToolCall {
        ToolCall {
            id: "tool-1".to_string(),
            call_id: Some("tool-1".to_string()),
            function: ToolFunction {
                name: name.to_string(),
                arguments: json!({}),
            },
            signature: None,
            additional_params: None,
        }
    }

    #[tokio::test]
    async fn empty_assistant_content_is_not_pushed() {
        let test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);
        let mut history: Vec<Message> = vec![];

        run(&mut history, "", "", &None, &None, &[], false, false, &ctx);

        assert!(
            history.is_empty(),
            "no text + no reasoning + no tool calls => nothing to push"
        );
    }

    #[tokio::test]
    async fn text_only_response_pushes_assistant_text_message() {
        let test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);
        let mut history: Vec<Message> = vec![];

        run(
            &mut history,
            "Hello, world!",
            "",
            &None,
            &None,
            &[],
            false,
            false,
            &ctx,
        );

        assert_eq!(history.len(), 1, "exactly one assistant message");
        let Message::Assistant { content, .. } = &history[0] else {
            panic!("expected Assistant message, got {:?}", history[0]);
        };
        let texts: Vec<&str> = content
            .iter()
            .filter_map(|c| match c {
                AssistantContent::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["Hello, world!"]);
    }

    #[tokio::test]
    async fn text_plus_tool_calls_pushes_combined_assistant_message() {
        let test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);
        let mut history: Vec<Message> = vec![];
        let tool_calls = vec![make_tool_call("read_file")];

        run(
            &mut history,
            "I'll read the file.",
            "",
            &None,
            &None,
            &tool_calls,
            true,
            false,
            &ctx,
        );

        assert_eq!(history.len(), 1);
        let Message::Assistant { content, .. } = &history[0] else {
            panic!("expected Assistant message");
        };
        let has_text = content
            .iter()
            .any(|c| matches!(c, AssistantContent::Text(t) if t.text == "I'll read the file."));
        let has_tool_call = content.iter().any(
            |c| matches!(c, AssistantContent::ToolCall(tc) if tc.function.name == "read_file"),
        );
        assert!(has_text, "text content must be present");
        assert!(has_tool_call, "tool call must be present");
    }
}
