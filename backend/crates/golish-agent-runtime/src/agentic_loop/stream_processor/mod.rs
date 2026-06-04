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
use rig::message::ToolCall;
use rig::streaming::{StreamedAssistantContent, StreamingCompletionResponse};

use golish_context::token_budget::TokenUsage;
use golish_core::events::AiEvent;
use golish_core::{has_complete_tool_args, initial_tool_args_fragment};
use golish_llm_providers::ProviderStreamQuirks;

use super::context::{emit_event, is_cancelled, AgenticLoopContext};
use super::stream_retry::classify_stream_start_error;

mod chunks;
mod encrypted;
mod span;
mod textual_tool_calls;
mod usage;

#[cfg(test)]
mod tests;

use self::chunks::{handle_reasoning, handle_reasoning_delta, handle_text_chunk};
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
    /// E1 · the stream was cut short because the model degenerated into
    /// repeating itself (`detect_repetitive_text`). The turn loop uses this to
    /// inject a bounded recovery re-prompt instead of accepting the garbage.
    pub repetition_detected: bool,
    /// E2 · a *retriable* error occurred mid-stream **after** some content had
    /// already streamed (so it wasn't surfaced as a terminal error). The turn
    /// loop uses this to retry (bounded) rather than silently accept the
    /// truncated output.
    pub mid_stream_error: Option<String>,
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
    // E1 · set when the text loop breaks because of degenerate repetition (vs a
    // normal end-of-stream). Threaded out so the turn loop can recover.
    let mut repetition_detected = false;

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
                    if handle_text_chunk(
                        ctx,
                        text_msg.text,
                        supports_thinking,
                        chunk_count,
                        &mut text_content,
                        accumulated_response,
                        &mut thinking_content,
                        accumulated_thinking,
                        &mut last_repetition_check_len,
                    ) {
                        // E1 · `handle_text_chunk` returns true only on degenerate
                        // repetition. Record it so the turn loop can re-prompt.
                        repetition_detected = true;
                        break;
                    }
                }
                StreamedAssistantContent::Reasoning(reasoning) => {
                    handle_reasoning(
                        ctx,
                        reasoning,
                        quirks.reasoning_handling,
                        supports_thinking,
                        chunk_count,
                        &mut text_content,
                        accumulated_response,
                        &mut thinking_content,
                        accumulated_thinking,
                        &mut thinking_signature,
                        &mut thinking_id,
                    );
                }
                StreamedAssistantContent::ReasoningDelta { id, reasoning } => {
                    handle_reasoning_delta(
                        ctx,
                        id,
                        reasoning,
                        quirks.reasoning_handling,
                        supports_thinking,
                        chunk_count,
                        &mut text_content,
                        accumulated_response,
                        &mut thinking_content,
                        accumulated_thinking,
                        &mut thinking_id,
                    );
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

    // Strip textual tool-call markup from displayed text unconditionally so it
    // never leaks, regardless of whether a call is recovered below or the
    // provider already produced native tool calls.
    let cleaned_text = strip_textual_tool_call_markup(&text_content);
    let cleaned_accumulated = strip_textual_tool_call_markup(accumulated_response);

    // Recover a textual call to execute only when the provider produced none
    // (so the same intent is not executed twice).
    if tool_calls_to_execute.is_empty() {
        if let Some(tool_call) = extract_textual_tool_call(&text_content, iteration) {
            let tool_name = tool_call.function.name.clone();
            tracing::warn!(
                iteration,
                tool_name = %tool_name,
                text_len = text_content.len(),
                "[tool-adapter] Converted textual XML-style tool call into structured tool call"
            );
            tool_calls_to_execute.push(tool_call);
            has_tool_calls = true;
        }
    }

    text_content = cleaned_text;
    *accumulated_response = cleaned_accumulated;

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

    // E2 · a mid-stream chunk error that left *some* usable content was
    // previously swallowed (only the no-content case surfaced an error). Thread
    // out the last *retriable* one so the turn loop can retry instead of
    // accepting truncated output.
    let mid_stream_error =
        last_stream_chunk_error.filter(|err| classify_stream_start_error(err).retriable);
    if let Some(ref err) = mid_stream_error {
        tracing::warn!(
            error = %err,
            "[resilience] retriable mid-stream error with partial content; flagging for bounded retry"
        );
    }

    Ok(StreamOutcome::Continue(StreamProcessOutcome {
        has_tool_calls,
        tool_calls_to_execute,
        text_content,
        thinking_content,
        thinking_signature,
        thinking_id,
        repetition_detected,
        mid_stream_error,
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

