use rig::completion::{AssistantContent, Message};
use rig::message::{Text, ToolCall, ToolResult, ToolResultContent, UserContent};
use rig::one_or_many::OneOrMany;
use tracing::Span;

use golish_context::token_budget::TokenUsage;
use golish_core::utils::truncate_str;
use golish_sub_agents::SubAgentContext;

use super::{AgenticLoopConfig, AgenticLoopContext};

pub(crate) fn trace_input_for_span(history: &[Message]) -> String {
    let trace_input = history
        .iter()
        .rev()
        .find_map(|msg| {
            if let Message::User { content } = msg {
                Some(
                    content
                        .iter()
                        .filter_map(|c| {
                            if let UserContent::Text(text) = c {
                                Some(text.text.clone())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
            } else {
                None
            }
        })
        .unwrap_or_default();

    if trace_input.len() > 2000 {
        format!("{}... [truncated]", truncate_str(&trace_input, 2000))
    } else {
        trace_input
    }
}

pub(super) fn record_last_user_text_for_span(llm_span: &Span, chat_history: &[Message]) {
    // Only record actual user text; tool results are already represented by previous tool spans.
    let last_user_text = chat_history
        .iter()
        .rev()
        .find_map(|msg| {
            if let Message::User { content } = msg {
                let text_parts: Vec<String> = content
                    .iter()
                    .filter_map(|c| {
                        if let UserContent::Text(text) = c {
                            if !text.text.is_empty() {
                                Some(text.text.clone())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                    .collect();
                if text_parts.is_empty() {
                    None
                } else {
                    Some(text_parts.join("\n"))
                }
            } else {
                None
            }
        })
        .unwrap_or_default();

    if last_user_text.is_empty() {
        return;
    }

    let prompt_for_span = if last_user_text.len() > 2000 {
        format!("{}... [truncated]", truncate_str(&last_user_text, 2000))
    } else {
        last_user_text
    };
    llm_span.record("gen_ai.prompt", prompt_for_span.as_str());
    llm_span.record("langfuse.observation.input", prompt_for_span.as_str());
}

pub(crate) fn record_agent_turn_start(ctx: &AgenticLoopContext<'_>, chat_history: &[Message]) {
    if let Some(tracker) = ctx.events.db_tracker {
        tracker.audit(
            "agent_turn_start",
            "ai",
            &format!(
                "model={} provider={}",
                ctx.llm.model_name, ctx.llm.provider_name
            ),
        );
        let user_msg_preview = chat_history
            .last()
            .map(|m| match m {
                Message::User { content } => content
                    .iter()
                    .filter_map(|c| match c {
                        UserContent::Text(t) => Some(t.text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => String::new(),
            })
            .unwrap_or_default();
        if !user_msg_preview.is_empty() {
            tracker.record_msg_log("user_message", "primary", &user_msg_preview, None);
        }
    }
}

pub(super) fn log_image_and_reasoning_diagnostics(
    chat_history: &[Message],
    iteration: usize,
    provider_name: &str,
    supports_thinking: bool,
) {
    if !tracing::enabled!(tracing::Level::DEBUG) {
        return;
    }

    let image_count: usize = chat_history
        .iter()
        .map(|msg| {
            if let Message::User { content } = msg {
                content
                    .iter()
                    .filter(|c| matches!(c, UserContent::Image(_)))
                    .count()
            } else {
                0
            }
        })
        .sum();
    if image_count > 0 {
        tracing::debug!(
            "[Unified] Chat history contains {} image(s) across {} messages",
            image_count,
            chat_history.len()
        );
    }

    let has_reasoning_in_history = chat_history.iter().any(|m| {
        if let Message::Assistant { content, .. } = m {
            content
                .iter()
                .any(|c| matches!(c, AssistantContent::Reasoning(_)))
        } else {
            false
        }
    });
    tracing::debug!(
        "[OpenAI Debug] Starting stream: iteration={}, history_len={}, provider={}, has_reasoning_history={}, thinking={}",
        iteration,
        chat_history.len(),
        provider_name,
        has_reasoning_in_history,
        supports_thinking
    );
}

pub(crate) fn push_unavailable_tool_results(
    chat_history: &mut Vec<Message>,
    rejected: &[ToolCall],
) {
    let error_results: Vec<UserContent> = rejected
        .iter()
        .map(|tc| {
            UserContent::ToolResult(ToolResult {
                id: tc.id.clone(),
                call_id: Some(tc.id.clone()),
                content: OneOrMany::one(ToolResultContent::Text(Text {
                    text: format!(
                        "Error: Tool '{}' is not available in the current execution mode. \
                         Use sub-agent delegation (sub_agent_*) tools instead.",
                        tc.function.name
                    ),
                })),
            })
        })
        .collect();

    if !error_results.is_empty() {
        chat_history.push(Message::User {
            content: OneOrMany::many(error_results).expect("rejected list is non-empty"),
        });
    }
}

pub(crate) fn record_turn_completion(
    ctx: &AgenticLoopContext<'_>,
    config: &AgenticLoopConfig,
    sub_agent_context: &SubAgentContext,
    supports_thinking: bool,
    accumulated_thinking: &str,
    total_usage: &TokenUsage,
) {
    if supports_thinking && !accumulated_thinking.is_empty() {
        tracing::debug!(
            "[Unified] Total thinking content: {} chars",
            accumulated_thinking.len()
        );
    }

    let agent_label = if config.is_sub_agent {
        format!("sub-agent (depth={})", sub_agent_context.depth)
    } else {
        "main-agent".to_string()
    };
    tracing::info!(
        "[{}] Turn complete: provider={}, model={}, tokens={{input={}, output={}, total={}}}",
        agent_label,
        ctx.llm.provider_name,
        ctx.llm.model_name,
        total_usage.input_tokens,
        total_usage.output_tokens,
        total_usage.total()
    );
}

pub(crate) fn record_final_output_and_usage(
    ctx: &AgenticLoopContext<'_>,
    accumulated_response: &str,
    total_usage: &TokenUsage,
    chat_message_span: &Span,
    agent_span: &Span,
) {
    let output_for_span = if accumulated_response.len() > 2000 {
        format!(
            "{}... [truncated]",
            truncate_str(accumulated_response, 2000)
        )
    } else {
        accumulated_response.to_string()
    };
    chat_message_span.record("langfuse.observation.output", output_for_span.as_str());
    agent_span.record("langfuse.observation.output", output_for_span.as_str());

    if let Some(tracker) = ctx.events.db_tracker {
        if total_usage.input_tokens > 0 || total_usage.output_tokens > 0 {
            tracker.record_token_usage(
                total_usage.input_tokens,
                total_usage.output_tokens,
                ctx.llm.model_name,
                ctx.llm.provider_name,
                0,
            );
        }
    }
}
