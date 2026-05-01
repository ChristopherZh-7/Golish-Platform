//! LLM stream processing for sub-agent execution.
//!
//! Extracts the streaming response processing loop from the main orchestrator,
//! handling text deltas, reasoning content, tool call accumulation, and idle
//! timeout detection.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use rig::message::{ToolCall, ToolFunction};
use rig::streaming::StreamedAssistantContent;

use crate::executor_helpers::epoch_secs;
use golish_core::events::AiEvent;

/// Accumulated result from processing an LLM streaming response.
pub(super) struct StreamResult {
    pub text_content: String,
    pub thinking_text: String,
    pub thinking_id: Option<String>,
    pub thinking_signature: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub has_tool_calls: bool,
    pub idle_timeout_hit: bool,
}

/// Process a streaming LLM response, accumulating text, thinking, and tool calls.
///
/// Handles text content deltas (emitted as `SubAgentTextDelta` events),
/// reasoning content (streamed and non-streamed), tool call accumulation
/// (complete and delta-based), idle timeout detection, encrypted reasoning
/// content from OpenAI Responses API, and pending tool call finalization.
///
/// Records reasoning content on `llm_span` before returning.
pub(super) async fn process_llm_stream<S, R, E>(
    stream: &mut S,
    agent_id: &str,
    parent_request_id: &str,
    event_tx: &tokio::sync::mpsc::UnboundedSender<AiEvent>,
    last_activity: &Arc<AtomicU64>,
    idle_timeout: Option<Duration>,
    llm_span: &tracing::Span,
) -> StreamResult
where
    S: futures::Stream<Item = Result<StreamedAssistantContent<R>, E>> + Unpin,
    R: serde::Serialize + rig::completion::GetTokenUsage,
    E: std::fmt::Display,
{
    let mut text_content = String::new();
    let mut thinking_text = String::new();
    let mut thinking_signature: Option<String> = None;
    let mut thinking_id: Option<String> = None;
    let mut has_tool_calls = false;
    let mut tool_calls_to_execute: Vec<ToolCall> = vec![];

    let mut current_tool_id: Option<String> = None;
    let mut current_tool_call_id: Option<String> = None;
    let mut current_tool_name: Option<String> = None;
    let mut current_tool_args = String::new();

    last_activity.store(epoch_secs(), Ordering::Relaxed);

    let mut idle_timeout_hit = false;
    loop {
        let chunk_opt = if let Some(idle_dur) = idle_timeout {
            let last = last_activity.load(Ordering::Relaxed);
            let now = epoch_secs();
            let remaining = idle_dur.as_secs().saturating_sub(now.saturating_sub(last));

            if remaining == 0 {
                idle_timeout_hit = true;
                break;
            }

            match tokio::time::timeout(Duration::from_secs(remaining), stream.next()).await {
                Ok(v) => v,
                Err(_) => {
                    idle_timeout_hit = true;
                    break;
                }
            }
        } else {
            stream.next().await
        };

        let Some(chunk_result) = chunk_opt else {
            break;
        };

        last_activity.store(epoch_secs(), Ordering::Relaxed);

        match chunk_result {
            Ok(chunk) => match chunk {
                StreamedAssistantContent::Text(text_msg) => {
                    text_content.push_str(&text_msg.text);
                    let _ = event_tx.send(AiEvent::SubAgentTextDelta {
                        agent_id: agent_id.to_string(),
                        delta: text_msg.text,
                        accumulated: text_content.clone(),
                        parent_request_id: parent_request_id.to_string(),
                    });
                }
                StreamedAssistantContent::Reasoning(reasoning) => {
                    for item in &reasoning.content {
                        if let rig::message::ReasoningContent::Text { text, signature } = item {
                            if !text.is_empty() {
                                tracing::debug!("[sub-agent] Thinking: {} chars", text.len());
                                thinking_text.push_str(text);
                            }
                            if signature.is_some() && thinking_signature.is_none() {
                                thinking_signature = signature.clone();
                            }
                        }
                    }
                    if reasoning.id.is_some() && thinking_id.is_none() {
                        thinking_id = reasoning.id.clone();
                    }
                }
                StreamedAssistantContent::ReasoningDelta { id, reasoning } => {
                    if !reasoning.is_empty() {
                        thinking_text.push_str(&reasoning);
                    }
                    if id.is_some() && thinking_id.is_none() {
                        thinking_id = id;
                    }
                }
                StreamedAssistantContent::ToolCall { tool_call, .. } => {
                    tracing::debug!(
                        "[sub-agent] Received tool call: {} (id: {})",
                        tool_call.function.name,
                        tool_call.id
                    );

                    if let (Some(prev_id), Some(prev_name)) =
                        (current_tool_id.take(), current_tool_name.take())
                    {
                        let args = golish_json_repair::parse_tool_args(&current_tool_args);
                        tracing::debug!(
                            "[sub-agent] Finalizing previous tool call: {} with args: {}",
                            prev_name,
                            current_tool_args
                        );
                        has_tool_calls = true;
                        let prev_call_id = current_tool_call_id
                            .take()
                            .unwrap_or_else(|| prev_id.clone());
                        tool_calls_to_execute.push(ToolCall {
                            id: prev_id,
                            call_id: Some(prev_call_id),
                            function: ToolFunction {
                                name: prev_name,
                                arguments: args,
                            },
                            signature: None,
                            additional_params: None,
                        });
                        current_tool_args.clear();
                    }

                    let has_complete_args = !tool_call.function.arguments.is_null()
                        && tool_call.function.arguments != serde_json::json!({});

                    if has_complete_args {
                        tracing::debug!(
                            "[sub-agent] Tool call has complete args: {:?}",
                            tool_call.function.arguments
                        );
                        has_tool_calls = true;
                        let mut tc = tool_call;
                        if tc.call_id.is_none() {
                            tc.call_id = Some(tc.id.clone());
                        }
                        tool_calls_to_execute.push(tc);
                    } else {
                        tracing::debug!(
                            "[sub-agent] Tool call has empty args, tracking for delta accumulation"
                        );
                        current_tool_id = Some(tool_call.id.clone());
                        current_tool_call_id = tool_call.call_id.clone();
                        current_tool_name = Some(tool_call.function.name.clone());
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
                    if let Some(usage) = resp.token_usage() {
                        llm_span
                            .record("gen_ai.usage.prompt_tokens", usage.input_tokens as i64);
                        llm_span.record(
                            "gen_ai.usage.completion_tokens",
                            usage.output_tokens as i64,
                        );
                    }

                    if let Ok(json_value) = serde_json::to_value(resp) {
                        if let Some(encrypted_map) = json_value
                            .get("reasoning_encrypted_content")
                            .and_then(|v| v.as_object())
                        {
                            if let Some(ref tid) = thinking_id {
                                if let Some(encrypted) =
                                    encrypted_map.get(tid).and_then(|v| v.as_str())
                                {
                                    tracing::debug!(
                                        "[sub-agent] Found encrypted_content for reasoning item {}: {} bytes",
                                        tid,
                                        encrypted.len()
                                    );
                                    thinking_signature = Some(encrypted.to_string());
                                }
                            }
                            if thinking_signature.is_none() && encrypted_map.len() == 1 {
                                if let Some((id, encrypted)) = encrypted_map.iter().next() {
                                    if let Some(encrypted_str) = encrypted.as_str() {
                                        tracing::debug!(
                                            "[sub-agent] Using single encrypted_content for reasoning item {}: {} bytes",
                                            id,
                                            encrypted_str.len()
                                        );
                                        thinking_signature = Some(encrypted_str.to_string());
                                        if thinking_id.is_none() {
                                            thinking_id = Some(id.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            },
            Err(e) => {
                tracing::warn!("[sub-agent] Stream error: {}", e);
            }
        }
    }

    // Finalize any remaining pending tool call after stream ends
    if let (Some(prev_id), Some(prev_name)) = (current_tool_id.take(), current_tool_name.take())
    {
        let args = golish_json_repair::parse_tool_args(&current_tool_args);
        tracing::debug!(
            "[sub-agent] Finalizing final tool call: {} with args: {}",
            prev_name,
            current_tool_args
        );
        has_tool_calls = true;
        let prev_call_id = current_tool_call_id
            .take()
            .unwrap_or_else(|| prev_id.clone());
        tool_calls_to_execute.push(ToolCall {
            id: prev_id,
            call_id: Some(prev_call_id),
            function: ToolFunction {
                name: prev_name,
                arguments: args,
            },
            signature: None,
            additional_params: None,
        });
    }

    // Record reasoning/thinking content on the llm_completion span if present.
    if !thinking_text.is_empty() {
        let mut end = thinking_text.len().min(2000);
        while end > 0 && !thinking_text.is_char_boundary(end) {
            end -= 1;
        }
        let reasoning_for_span = if thinking_text.len() > 2000 {
            format!("{}... [truncated]", &thinking_text[..end])
        } else {
            thinking_text.clone()
        };
        llm_span.record("gen_ai.reasoning", reasoning_for_span.as_str());
    }

    StreamResult {
        text_content,
        thinking_text,
        thinking_id,
        thinking_signature,
        tool_calls: tool_calls_to_execute,
        has_tool_calls,
        idle_timeout_hit,
    }
}
