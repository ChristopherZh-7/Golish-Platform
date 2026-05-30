//! Per-chunk handlers for the streaming loop: text deltas (including provider
//! `[Thinking]` / `[WEB_SEARCH_RESULT]` / `[WEB_FETCH_RESULT]` markers) and
//! native reasoning chunks/deltas.

use rig::message::{Reasoning, ReasoningContent};

use golish_core::events::AiEvent;
use golish_core::utils::truncate_str;
use golish_llm_providers::ReasoningHandling;

use super::super::context::{emit_event, AgenticLoopContext};

/// Handle a streamed text chunk. Returns `true` when degenerate repetition
/// was detected and the stream loop should break.
#[allow(clippy::too_many_arguments)]
pub(super) fn handle_text_chunk(
    ctx: &AgenticLoopContext<'_>,
    text: String,
    supports_thinking: bool,
    chunk_count: usize,
    text_content: &mut String,
    accumulated_response: &mut String,
    thinking_content: &mut String,
    accumulated_thinking: &mut String,
    last_repetition_check_len: &mut usize,
) -> bool {
    if let Some(thinking) = text.strip_prefix("[Thinking] ") {
        if supports_thinking {
            tracing::trace!(
                "[Unified] Received [Thinking]-prefixed text chunk #{}: {} chars",
                chunk_count,
                thinking.len()
            );
            thinking_content.push_str(thinking);
            accumulated_thinking.push_str(thinking);
        }
        emit_event(
            ctx,
            AiEvent::Reasoning {
                content: thinking.to_string(),
            },
        );
    } else if let Some(rest) = text.strip_prefix("[WEB_SEARCH_RESULT:") {
        // [WEB_SEARCH_RESULT:tool_use_id:json_results]
        if let Some(colon_pos) = rest.find(':') {
            let tool_use_id = &rest[..colon_pos];
            let json_rest = rest[colon_pos + 1..].trim_end_matches(']');
            if let Ok(results) = serde_json::from_str::<serde_json::Value>(json_rest) {
                tracing::info!("Parsed web search results for {}", tool_use_id);
                emit_event(
                    ctx,
                    AiEvent::WebSearchResult {
                        request_id: tool_use_id.to_string(),
                        results,
                    },
                );
            }
        }
    } else if let Some(rest) = text.strip_prefix("[WEB_FETCH_RESULT:") {
        // [WEB_FETCH_RESULT:tool_use_id:url:json_content]
        let parts: Vec<&str> = rest.splitn(3, ':').collect();
        if parts.len() >= 3 {
            let tool_use_id = parts[0];
            let url = parts[1];
            let json_rest = parts[2].trim_end_matches(']');
            let content_preview = if json_rest.len() > 200 {
                format!("{}...", truncate_str(json_rest, 200))
            } else {
                json_rest.to_string()
            };
            tracing::info!("Parsed web fetch result for {}: {}", tool_use_id, url);
            emit_event(
                ctx,
                AiEvent::WebFetchResult {
                    request_id: tool_use_id.to_string(),
                    url: url.to_string(),
                    content_preview,
                },
            );
        }
    } else {
        // Regular text content
        text_content.push_str(&text);
        accumulated_response.push_str(&text);
        let _ = ctx.events.event_tx.send(AiEvent::TextDelta {
            delta: text,
            accumulated: accumulated_response.clone(),
        });

        // Detect degenerate repetitive generation
        if text_content.len() > *last_repetition_check_len + 200 {
            *last_repetition_check_len = text_content.len();
            if super::super::sub_agent_dispatch::detect_repetitive_text(text_content.as_str()) {
                tracing::warn!(
                    text_len = text_content.len(),
                    "Repetitive text detected, stopping generation"
                );
                return true;
            }
        }
    }
    false
}

/// Handle a native reasoning chunk, routing it to the thinking or text
/// channel per the provider's [`ReasoningHandling`] quirk.
#[allow(clippy::too_many_arguments)]
pub(super) fn handle_reasoning(
    ctx: &AgenticLoopContext<'_>,
    reasoning: Reasoning,
    reasoning_handling: ReasoningHandling,
    supports_thinking: bool,
    chunk_count: usize,
    text_content: &mut String,
    accumulated_response: &mut String,
    thinking_content: &mut String,
    accumulated_thinking: &mut String,
    thinking_signature: &mut Option<String>,
    thinking_id: &mut Option<String>,
) {
    let reasoning_text = reasoning
        .content
        .iter()
        .filter_map(|c| {
            if let ReasoningContent::Text { text, .. } = c {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("");
    let chunk_signature = reasoning.content.iter().find_map(|c| {
        if let ReasoningContent::Text { signature, .. } = c {
            signature.clone()
        } else {
            None
        }
    });

    match reasoning_handling {
        ReasoningHandling::AlwaysContent => {
            if !reasoning_text.is_empty() {
                tracing::trace!(
                    "[Unified] quirks=AlwaysContent: routing reasoning chunk #{} ({} chars) to text channel",
                    chunk_count,
                    reasoning_text.len()
                );
                text_content.push_str(&reasoning_text);
                accumulated_response.push_str(&reasoning_text);
                let _ = ctx.events.event_tx.send(AiEvent::TextDelta {
                    delta: reasoning_text,
                    accumulated: accumulated_response.clone(),
                });
            }
        }
        ReasoningHandling::Standard | ReasoningHandling::FallbackToContent => {
            tracing::trace!(
                "[Unified] Received native reasoning chunk #{}: {} chars, has_signature: {}",
                chunk_count,
                reasoning_text.len(),
                chunk_signature.is_some()
            );
            // Always accumulate into the thinking buffer when
            // quirks route reasoning to the thinking channel.
            // `supports_thinking` only controls whether we
            // *persist* the reasoning in chat history (downstream
            // assistant_push phase); it must not gate runtime
            // display.
            thinking_content.push_str(&reasoning_text);
            if supports_thinking {
                accumulated_thinking.push_str(&reasoning_text);
                if chunk_signature.is_some() {
                    *thinking_signature = chunk_signature;
                }
                if reasoning.id.is_some() {
                    *thinking_id = reasoning.id.clone();
                }
            }
            emit_event(
                ctx,
                AiEvent::Reasoning {
                    content: reasoning_text,
                },
            );
        }
    }
}

/// Handle a streamed reasoning delta (incremental reasoning text).
#[allow(clippy::too_many_arguments)]
pub(super) fn handle_reasoning_delta(
    ctx: &AgenticLoopContext<'_>,
    id: Option<String>,
    reasoning: String,
    reasoning_handling: ReasoningHandling,
    supports_thinking: bool,
    chunk_count: usize,
    text_content: &mut String,
    accumulated_response: &mut String,
    thinking_content: &mut String,
    accumulated_thinking: &mut String,
    thinking_id: &mut Option<String>,
) {
    match reasoning_handling {
        ReasoningHandling::AlwaysContent => {
            if !reasoning.is_empty() {
                tracing::trace!(
                    "[Unified] quirks=AlwaysContent: routing reasoning delta chunk #{} ({} chars) to text channel",
                    chunk_count,
                    reasoning.len()
                );
                text_content.push_str(&reasoning);
                accumulated_response.push_str(&reasoning);
                let _ = ctx.events.event_tx.send(AiEvent::TextDelta {
                    delta: reasoning,
                    accumulated: accumulated_response.clone(),
                });
            }
        }
        ReasoningHandling::Standard | ReasoningHandling::FallbackToContent => {
            tracing::trace!(
                "[Unified] Received reasoning delta chunk #{}: {} chars",
                chunk_count,
                reasoning.len()
            );
            thinking_content.push_str(&reasoning);
            if supports_thinking {
                accumulated_thinking.push_str(&reasoning);
                if id.is_some() && thinking_id.is_none() {
                    *thinking_id = id;
                }
            }
            emit_event(ctx, AiEvent::Reasoning { content: reasoning });
        }
    }
}
