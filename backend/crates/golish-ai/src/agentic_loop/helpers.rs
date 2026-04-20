//! Small helper functions extracted from the agentic loop.

use rig::completion::{AssistantContent, Message};
use rig::message::{
    ReasoningContent, Text, ToolResult, ToolResultContent, UserContent,
};
use rig::one_or_many::OneOrMany;
use serde_json::json;
use tokio::sync::mpsc;

use crate::loop_detection::LoopDetectionResult;
use golish_core::events::AiEvent;

/// Estimate the token count of a message for heuristic token estimation.
///
/// Uses tokenx-rs for ~96% accuracy vs tiktoken cl100k_base.
/// Falls back to character-count heuristics for media content.
pub(crate) fn estimate_message_tokens(message: &Message) -> usize {
    match message {
        Message::User { content } => content
            .iter()
            .map(|c| match c {
                UserContent::Text(text) => tokenx_rs::estimate_token_count(&text.text),
                UserContent::ToolResult(result) => {
                    tokenx_rs::estimate_token_count(&result.id)
                        + result
                            .content
                            .iter()
                            .map(|r| match r {
                                ToolResultContent::Text(t) => {
                                    tokenx_rs::estimate_token_count(&t.text)
                                }
                                ToolResultContent::Image(_) => 250,
                            })
                            .sum::<usize>()
                }
                UserContent::Image(_) => 250,
                UserContent::Audio(_) => 1250,
                UserContent::Video(_) => 2500,
                UserContent::Document(_) => 1250,
            })
            .sum(),
        Message::Assistant { content, .. } => content
            .iter()
            .map(|c| match c {
                AssistantContent::Text(text) => tokenx_rs::estimate_token_count(&text.text),
                AssistantContent::ToolCall(call) => {
                    tokenx_rs::estimate_token_count(&call.function.name)
                        + serde_json::to_string(&call.function.arguments)
                            .map(|s| tokenx_rs::estimate_token_count(&s))
                            .unwrap_or(0)
                }
                AssistantContent::Reasoning(reasoning) => reasoning
                    .content
                    .iter()
                    .map(|c| match c {
                        ReasoningContent::Text { text, .. } => {
                            tokenx_rs::estimate_token_count(text)
                        }
                        _ => 0,
                    })
                    .sum::<usize>(),
                AssistantContent::Image(_) => 250,
            })
            .sum(),
    }
}

/// Handle loop detection result and create appropriate tool result if blocked.
///
/// `tool_id` is the main identifier (used for events/UI).
/// `tool_call_id` is used for the tool result's call_id (OpenAI uses call_* format).
pub fn handle_loop_detection(
    loop_result: &LoopDetectionResult,
    tool_id: &str,
    tool_call_id: &str,
    event_tx: &mpsc::UnboundedSender<AiEvent>,
) -> Option<UserContent> {
    match loop_result {
        LoopDetectionResult::Blocked {
            tool_name,
            repeat_count,
            max_count,
            message,
        } => {
            let _ = event_tx.send(AiEvent::LoopBlocked {
                tool_name: tool_name.clone(),
                repeat_count: *repeat_count,
                max_count: *max_count,
                message: message.clone(),
            });
            let result_text = serde_json::to_string(&json!({
                "error": message,
                "loop_detected": true,
                "repeat_count": repeat_count,
                "suggestion": "Try a different approach or modify the arguments"
            }))
            .unwrap_or_default();
            Some(UserContent::ToolResult(ToolResult {
                id: tool_id.to_string(),
                call_id: Some(tool_call_id.to_string()),
                content: OneOrMany::one(ToolResultContent::Text(Text { text: result_text })),
            }))
        }
        LoopDetectionResult::MaxIterationsReached {
            iterations,
            max_iterations,
            message,
        } => {
            let _ = event_tx.send(AiEvent::MaxIterationsReached {
                iterations: *iterations,
                max_iterations: *max_iterations,
                message: message.clone(),
            });
            let result_text = serde_json::to_string(&json!({
                "error": message,
                "max_iterations_reached": true,
                "suggestion": "Provide a final response to the user"
            }))
            .unwrap_or_default();
            Some(UserContent::ToolResult(ToolResult {
                id: tool_id.to_string(),
                call_id: Some(tool_call_id.to_string()),
                content: OneOrMany::one(ToolResultContent::Text(Text { text: result_text })),
            }))
        }
        LoopDetectionResult::Warning {
            tool_name,
            current_count,
            max_count,
            message,
        } => {
            let _ = event_tx.send(AiEvent::LoopWarning {
                tool_name: tool_name.clone(),
                current_count: *current_count,
                max_count: *max_count,
                message: message.clone(),
            });
            None
        }
        LoopDetectionResult::Allowed => None,
    }
}
