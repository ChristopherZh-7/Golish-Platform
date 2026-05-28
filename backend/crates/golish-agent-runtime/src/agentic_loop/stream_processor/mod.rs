//! Process the streaming chunks from one LLM call.
//!
//! Walks the [`StreamingCompletionResponse`] returned by `model.stream(...)`,
//! demultiplexes [`StreamedAssistantContent`] variants into per-iteration
//! accumulators (text, reasoning, tool calls, token usage), forwards user-
//! visible deltas through `ctx.event_tx`, and finalizes any pending tool call
//! that wasn't closed by a `Final` chunk.
//!
//! Provider quirks handled:
//! - **`[Thinking] ` text prefix** — older streaming impls put thinking content
//!   into Text chunks with this prefix; we route them to the reasoning bucket.
//! - **`[WEB_SEARCH_RESULT:..]` / `[WEB_FETCH_RESULT:..]` markers** — server
//!   tool results emitted as raw text by the OpenAI Responses provider; parsed
//!   and re-emitted as structured `AiEvent`s.
//! - **OpenAI Responses `reasoning_encrypted_content`** — required for stateless
//!   multi-turn conversations with reasoning models. Extracted from the `Final`
//!   payload via JSON serialization since it isn't exposed on the typed struct.
//! - **Function-call streaming** — OpenAI delivers tool call args as multiple
//!   delta chunks; we accumulate them and run them through `golish_json_repair`
//!   on close.

use anyhow::Result;
use futures::StreamExt;
use rig::completion::Message;
use rig::message::{ReasoningContent, ToolCall};
use rig::streaming::{StreamedAssistantContent, StreamingCompletionResponse};

use golish_context::token_budget::TokenUsage;
use golish_core::events::AiEvent;
use golish_core::utils::truncate_str;
use golish_llm_providers::{ProviderStreamQuirks, ReasoningHandling};

use super::context::{emit_event, is_cancelled, AgenticLoopContext};
use super::stream_retry::classify_stream_start_error;

mod encrypted;
mod span;
mod textual_tool_calls;
mod usage;

use self::encrypted::extract_openai_reasoning_encrypted_content;
use self::span::{record_completion_for_span, record_reasoning_for_span};
use self::textual_tool_calls::{extract_textual_tool_call, strip_textual_tool_call_markup};
use self::usage::record_token_usage;

const CANCEL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

/// Per-iteration accumulated stream state, returned to the agentic loop after
/// the stream has been fully consumed (and any trailing pending tool call has
/// been finalized).
pub(crate) struct StreamProcessOutcome {
    pub has_tool_calls: bool,
    pub tool_calls_to_execute: Vec<ToolCall>,
    pub text_content: String,
    pub thinking_content: String,
    pub thinking_signature: Option<String>,
    pub thinking_id: Option<String>,
}

/// Outcome enum for the agentic loop: either keep going with the accumulated
/// stream state, or break out (the stream produced no usable content and a
/// terminal error has already been emitted to the user).
pub(crate) enum StreamOutcome {
    Continue(StreamProcessOutcome),
    BreakAgentLoop,
}

/// Drive a single LLM stream to completion.
///
/// Mutates the supplied accumulators (`accumulated_response`, `accumulated_thinking`,
/// `total_usage`) so they keep growing across iterations of the outer agent loop.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn process_stream<M>(
    mut stream: StreamingCompletionResponse<M::StreamingResponse>,
    ctx: &AgenticLoopContext<'_>,
    chat_history: &[Message],
    llm_span: &tracing::Span,
    iteration: usize,
    supports_thinking: bool,
    quirks: &ProviderStreamQuirks,
    accumulated_response: &mut String,
    accumulated_thinking: &mut String,
    total_usage: &mut TokenUsage,
) -> Result<StreamOutcome>
where
    M: rig::completion::CompletionModel + Sync,
{
    tracing::debug!(
        "[Unified] Stream started - listening for content (reasoning_handling={:?})",
        quirks.reasoning_handling
    );

    let mut has_tool_calls = false;
    let mut tool_calls_to_execute: Vec<ToolCall> = vec![];
    let mut text_content = String::new();
    let mut thinking_content = String::new();
    let mut thinking_signature: Option<String> = None;
    // Reasoning ID for OpenAI Responses API (rs_... IDs that function calls reference)
    let mut thinking_id: Option<String> = None;
    let mut chunk_count = 0_usize;
    let mut last_stream_chunk_error: Option<String> = None;
    let mut last_repetition_check_len: usize = 0;

    // Track in-flight tool-call state across delta chunks.
    // call_id (OpenAI's "call_abc") is tracked separately from the item id
    // ("fc_abc") because they differ in the Responses API; the call_id must
    // match when sending function_call_output back.
    let mut current_tool_id: Option<String> = None;
    let mut current_tool_call_id: Option<String> = None;
    let mut current_tool_name: Option<String> = None;
    let mut current_tool_args = String::new();

    loop {
        let chunk_result = tokio::select! {
            biased;
            _ = wait_for_cancelled(ctx) => {
                tracing::info!(
                    "Agent cancelled while waiting for stream chunk (chunk {})",
                    chunk_count
                );
                drop(stream);
                let _ = ctx.events.event_tx.send(AiEvent::Error {
                    message: "Agent stopped by user".to_string(),
                    error_type: "cancelled".to_string(),
                });
                return Err(anyhow::anyhow!("Agent stopped by user"));
            }
            chunk = stream.next() => chunk,
        };

        let Some(chunk_result) = chunk_result else {
            break;
        };

        if is_cancelled(ctx) {
            tracing::info!(
                "Agent cancelled during stream processing (chunk {})",
                chunk_count
            );
            drop(stream);
            let _ = ctx.events.event_tx.send(AiEvent::Error {
                message: "Agent stopped by user".to_string(),
                error_type: "cancelled".to_string(),
            });
            return Err(anyhow::anyhow!("Agent stopped by user"));
        }
        chunk_count += 1;
        // Log progress every 50 chunks to avoid spam but track stream activity
        if chunk_count.is_multiple_of(50) {
            tracing::debug!(
                "[OpenAI Debug] Stream progress: {} chunks processed",
                chunk_count
            );
        }

        match chunk_result {
            Ok(chunk) => match chunk {
                StreamedAssistantContent::Text(text_msg) => {
                    if let Some(thinking) = text_msg.text.strip_prefix("[Thinking] ") {
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
                    } else if let Some(rest) = text_msg.text.strip_prefix("[WEB_SEARCH_RESULT:") {
                        // [WEB_SEARCH_RESULT:tool_use_id:json_results]
                        if let Some(colon_pos) = rest.find(':') {
                            let tool_use_id = &rest[..colon_pos];
                            let json_rest = rest[colon_pos + 1..].trim_end_matches(']');
                            if let Ok(results) =
                                serde_json::from_str::<serde_json::Value>(json_rest)
                            {
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
                    } else if let Some(rest) = text_msg.text.strip_prefix("[WEB_FETCH_RESULT:") {
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
                        text_content.push_str(&text_msg.text);
                        accumulated_response.push_str(&text_msg.text);
                        let _ = ctx.events.event_tx.send(AiEvent::TextDelta {
                            delta: text_msg.text,
                            accumulated: accumulated_response.clone(),
                        });

                        // Detect degenerate repetitive generation
                        if text_content.len() > last_repetition_check_len + 200 {
                            last_repetition_check_len = text_content.len();
                            if super::sub_agent_dispatch::detect_repetitive_text(&text_content) {
                                tracing::warn!(
                                    text_len = text_content.len(),
                                    "Repetitive text detected, stopping generation"
                                );
                                break;
                            }
                        }
                    }
                }
                StreamedAssistantContent::Reasoning(reasoning) => {
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

                    match quirks.reasoning_handling {
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
                                    thinking_signature = chunk_signature;
                                }
                                if reasoning.id.is_some() {
                                    thinking_id = reasoning.id.clone();
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
                StreamedAssistantContent::ReasoningDelta { id, reasoning } => {
                    match quirks.reasoning_handling {
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
                                    thinking_id = id;
                                }
                            }
                            emit_event(ctx, AiEvent::Reasoning { content: reasoning });
                        }
                    }
                }
                StreamedAssistantContent::ToolCall { tool_call, .. } => {
                    // Server tool (web_search/web_fetch executed by provider)
                    let is_server_tool = tool_call
                        .call_id
                        .as_ref()
                        .map(|id: &String| id.starts_with("server:"))
                        .unwrap_or(false);

                    if is_server_tool {
                        tracing::info!(
                            "Server tool detected: {} ({})",
                            tool_call.function.name,
                            tool_call.id
                        );
                        emit_event(
                            ctx,
                            AiEvent::ServerToolStarted {
                                request_id: tool_call.id.clone(),
                                tool_name: tool_call.function.name.clone(),
                                input: tool_call.function.arguments.clone(),
                            },
                        );
                        // Don't add to tool_calls_to_execute - provider handles execution
                        continue;
                    }

                    has_tool_calls = true;

                    // Finalize any previous pending tool call first
                    if let (Some(prev_id), Some(prev_name)) =
                        (current_tool_id.take(), current_tool_name.take())
                    {
                        let args = golish_json_repair::parse_tool_args(&current_tool_args);
                        let prev_call_id = current_tool_call_id
                            .take()
                            .unwrap_or_else(|| prev_id.clone());
                        tool_calls_to_execute.push(ToolCall {
                            id: prev_id,
                            call_id: Some(prev_call_id),
                            function: rig::message::ToolFunction {
                                name: prev_name,
                                arguments: args,
                            },
                            signature: None,
                            additional_params: None,
                        });
                        current_tool_args.clear();
                    }

                    // Empty args and string args mean the provider is still streaming
                    // function arguments. Some OpenAI-compatible providers send the
                    // first partial JSON fragment as a string; treating that as complete
                    // causes duplicate approval prompts with malformed args.
                    let has_complete_args = has_complete_tool_args(&tool_call.function.arguments);

                    if has_complete_args {
                        let mut tool_call = tool_call;
                        if tool_call.call_id.is_none() {
                            tool_call.call_id = Some(tool_call.id.clone());
                        }
                        tool_calls_to_execute.push(tool_call);
                    } else {
                        current_tool_id = Some(tool_call.id.clone());
                        current_tool_call_id = tool_call.call_id.clone();
                        current_tool_name = Some(tool_call.function.name.clone());
                        current_tool_args =
                            initial_tool_args_fragment(&tool_call.function.arguments);
                    }
                }
                StreamedAssistantContent::ToolCallDelta { id, content, .. } => {
                    if current_tool_id.is_none() && !id.is_empty() {
                        current_tool_id = Some(id);
                    }
                    if let rig::streaming::ToolCallDeltaContent::Delta(delta) = content {
                        current_tool_args.push_str(&delta);
                    }
                }
                StreamedAssistantContent::Final(ref resp) => {
                    record_token_usage(ctx, chat_history, llm_span, iteration, total_usage, resp)
                        .await;
                    extract_openai_reasoning_encrypted_content(
                        resp,
                        &mut thinking_id,
                        &mut thinking_signature,
                    );

                    // Finalize any pending tool call from deltas
                    if let (Some(id), Some(name)) =
                        (current_tool_id.take(), current_tool_name.take())
                    {
                        let args = golish_json_repair::parse_tool_args(&current_tool_args);
                        let call_id = current_tool_call_id.take().unwrap_or_else(|| id.clone());
                        tool_calls_to_execute.push(ToolCall {
                            id,
                            call_id: Some(call_id),
                            function: rig::message::ToolFunction {
                                name,
                                arguments: args,
                            },
                            signature: None,
                            additional_params: None,
                        });
                        current_tool_args.clear();
                    }
                }
            },
            Err(e) => {
                last_stream_chunk_error = Some(e.to_string());
                tracing::warn!("Stream chunk error at #{}: {}", chunk_count, e);
            }
        }
    }

    // Finalize any tool call that wasn't closed by a Final chunk.
    if let (Some(id), Some(name)) = (current_tool_id.take(), current_tool_name.take()) {
        let args = golish_json_repair::parse_tool_args(&current_tool_args);
        let call_id = current_tool_call_id.take().unwrap_or_else(|| id.clone());
        tool_calls_to_execute.push(ToolCall {
            id,
            call_id: Some(call_id),
            function: rig::message::ToolFunction {
                name,
                arguments: args,
            },
            signature: None,
            additional_params: None,
        });
        has_tool_calls = true;
    }

    if tool_calls_to_execute.is_empty() {
        if let Some(tool_call) = extract_textual_tool_call(&text_content, iteration) {
            let tool_name = tool_call.function.name.clone();
            let cleaned_text = strip_textual_tool_call_markup(&text_content);
            let cleaned_accumulated = strip_textual_tool_call_markup(accumulated_response);

            tracing::warn!(
                iteration,
                tool_name = %tool_name,
                text_len = text_content.len(),
                "[tool-adapter] Converted textual XML-style tool call into structured tool call"
            );

            text_content = cleaned_text;
            *accumulated_response = cleaned_accumulated;
            tool_calls_to_execute.push(tool_call);
            has_tool_calls = true;
        }
    }

    // No usable content + chunk errors observed: surface the error and break.
    if text_content.is_empty() && thinking_content.is_empty() && tool_calls_to_execute.is_empty() {
        if let Some(ref err_msg) = last_stream_chunk_error {
            let classification = classify_stream_start_error(err_msg);
            let _ = ctx.events.event_tx.send(AiEvent::Error {
                message: classification.user_message.clone(),
                error_type: classification.error_type.to_string(),
            });
            tracing::error!("Stream produced no content; last chunk error: {}", err_msg);
            return Ok(StreamOutcome::BreakAgentLoop);
        }
    }

    tracing::info!(
        "[OpenAI Debug] Stream completed: iteration={}, chunks={}, text_chars={}, thinking_chars={}, tool_calls={}",
        iteration,
        chunk_count,
        text_content.len(),
        thinking_content.len(),
        tool_calls_to_execute.len()
    );
    tracing::debug!(
        "Stream completed (unified): {} chunks, {} chars text, {} chars thinking, {} tool calls",
        chunk_count,
        text_content.len(),
        thinking_content.len(),
        tool_calls_to_execute.len()
    );

    record_completion_for_span(llm_span, &text_content, &tool_calls_to_execute);
    record_reasoning_for_span(llm_span, &thinking_content);

    if supports_thinking && !thinking_content.is_empty() {
        tracing::debug!("Model thinking: {} chars", thinking_content.len());
    }

    Ok(StreamOutcome::Continue(StreamProcessOutcome {
        has_tool_calls,
        tool_calls_to_execute,
        text_content,
        thinking_content,
        thinking_signature,
        thinking_id,
    }))
}

async fn wait_for_cancelled(ctx: &AgenticLoopContext<'_>) {
    loop {
        if is_cancelled(ctx) {
            return;
        }
        tokio::time::sleep(CANCEL_POLL_INTERVAL).await;
    }
}

fn has_complete_tool_args(args: &serde_json::Value) -> bool {
    match args {
        serde_json::Value::Null => false,
        serde_json::Value::Object(map) if map.is_empty() => false,
        serde_json::Value::String(_) => false,
        _ => true,
    }
}

fn initial_tool_args_fragment(args: &serde_json::Value) -> String {
    match args {
        serde_json::Value::Null => String::new(),
        serde_json::Value::Object(map) if map.is_empty() => String::new(),
        serde_json::Value::String(value) => value.clone(),
        value => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{has_complete_tool_args, initial_tool_args_fragment};

    #[test]
    fn string_tool_args_are_treated_as_streaming_fragments() {
        let args = json!("{\"command\":\"dig example.com A +short\", \"cwd\":");

        assert!(!has_complete_tool_args(&args));
        assert_eq!(
            initial_tool_args_fragment(&args),
            "{\"command\":\"dig example.com A +short\", \"cwd\":"
        );
    }

    #[test]
    fn object_tool_args_are_complete() {
        let args = json!({"command": "dig example.com A +short", "cwd": ""});

        assert!(has_complete_tool_args(&args));
        assert_eq!(
            initial_tool_args_fragment(&args),
            "{\"command\":\"dig example.com A +short\",\"cwd\":\"\"}"
        );
    }
}
