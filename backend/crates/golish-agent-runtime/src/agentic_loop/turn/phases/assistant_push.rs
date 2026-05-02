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
