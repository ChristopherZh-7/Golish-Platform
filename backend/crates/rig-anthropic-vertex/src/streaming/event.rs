//! `event_to_chunk` — translate parsed `StreamEvent`s into
//! [`super::StreamChunk`]s, handling thinking-signature accumulation,
//! tool-use start, web-search/web-fetch results, and the final usage
//! roll-up.

use crate::types::{ContentBlock, ContentDelta, StreamEvent, Usage};

use super::{StreamChunk, StreamingResponse};

impl StreamingResponse {
    /// Convert a stream event to a stream chunk.
    pub(super) fn event_to_chunk(&mut self, event: StreamEvent) -> Option<StreamChunk> {
        match event {
            StreamEvent::ContentBlockDelta { delta, index: _ } => match delta {
                ContentDelta::TextDelta { text } => Some(StreamChunk::TextDelta {
                    text,
                    accumulated: self.accumulated_text.clone(),
                }),
                ContentDelta::InputJsonDelta { partial_json } => {
                    Some(StreamChunk::ToolInputDelta { partial_json })
                }
                ContentDelta::ThinkingDelta { thinking } => {
                    Some(StreamChunk::ThinkingDelta { thinking })
                }
                ContentDelta::SignatureDelta { signature } => {
                    // Accumulate signature for later emission.
                    self.accumulated_signature.push_str(&signature);
                    None
                }
                ContentDelta::CitationsDelta { citation: _ } => {
                    // Citations are metadata for source attribution, not
                    // streamed content.
                    None
                }
            },
            StreamEvent::ContentBlockStart {
                content_block,
                index,
            } => match content_block {
                ContentBlock::ToolUse { id, name, .. } => {
                    tracing::info!(
                        "event_to_chunk: ToolUseStart index={} name={}",
                        index,
                        name
                    );
                    Some(StreamChunk::ToolUseStart { id, name })
                }
                ContentBlock::Thinking { .. } => {
                    tracing::debug!("event_to_chunk: Thinking block start index={}", index);
                    None // Thinking content comes via ThinkingDelta
                }
                // Server tool use (Claude's native web_search/web_fetch).
                ContentBlock::ServerToolUse { id, name, input } => {
                    tracing::info!(
                        "event_to_chunk: ServerToolUseStart index={} name={}",
                        index,
                        name
                    );
                    Some(StreamChunk::ServerToolUseStart { id, name, input })
                }
                ContentBlock::WebSearchToolResult {
                    tool_use_id,
                    content,
                } => {
                    tracing::info!(
                        "event_to_chunk: WebSearchToolResult index={} tool_use_id={}",
                        index,
                        tool_use_id
                    );
                    Some(StreamChunk::WebSearchResult {
                        tool_use_id,
                        results: content,
                    })
                }
                ContentBlock::WebFetchToolResult {
                    tool_use_id,
                    content,
                } => {
                    // Try to extract URL from content for convenience.
                    let url = content
                        .get("url")
                        .and_then(|u| u.as_str())
                        .unwrap_or("")
                        .to_string();
                    tracing::info!(
                        "event_to_chunk: WebFetchToolResult index={} tool_use_id={} url={}",
                        index,
                        tool_use_id,
                        url
                    );
                    Some(StreamChunk::WebFetchResult {
                        tool_use_id,
                        url,
                        content,
                    })
                }
                _ => {
                    tracing::debug!(
                        "event_to_chunk: ContentBlockStart index={} (text, skipped)",
                        index
                    );
                    None // Text blocks don't need special handling at start.
                }
            },
            StreamEvent::MessageDelta { delta, usage } => {
                // Use input_tokens from MessageDelta if available (newer
                // API behavior), otherwise fall back to MessageStart value.
                let input_tokens = if usage.input_tokens > 0 {
                    usage.input_tokens
                } else {
                    self.input_tokens.unwrap_or(0)
                };
                let combined_usage = Usage {
                    input_tokens,
                    output_tokens: usage.output_tokens,
                    cache_creation_input_tokens: usage.cache_creation_input_tokens,
                    cache_read_input_tokens: usage.cache_read_input_tokens,
                };
                tracing::info!(
                    "event_to_chunk: MessageDelta stop_reason={:?} input_tokens={} output_tokens={}",
                    delta.stop_reason, combined_usage.input_tokens, combined_usage.output_tokens
                );
                self.done = true;
                Some(StreamChunk::Done {
                    stop_reason: delta.stop_reason.map(|r| format!("{:?}", r)),
                    usage: Some(combined_usage),
                })
            }
            StreamEvent::MessageStop => {
                tracing::info!("event_to_chunk: MessageStop");
                self.done = true;
                Some(StreamChunk::Done {
                    stop_reason: None,
                    usage: None,
                })
            }
            StreamEvent::Error { error } => {
                tracing::error!(
                    "event_to_chunk: Error type={} message={}",
                    error.error_type,
                    error.message
                );
                Some(StreamChunk::Error {
                    message: error.message,
                })
            }
            StreamEvent::MessageStart { message } => {
                // Capture input_tokens from MessageStart — Anthropic
                // only sends input_tokens here, and only output_tokens
                // in MessageDelta.
                self.input_tokens = Some(message.usage.input_tokens);
                tracing::debug!(
                    "event_to_chunk: MessageStart input_tokens={}",
                    message.usage.input_tokens
                );
                None
            }
            StreamEvent::ContentBlockStop { index } => {
                tracing::debug!("event_to_chunk: ContentBlockStop index={}", index);
                // If we have an accumulated signature, emit it now
                // (thinking block ended).
                if !self.accumulated_signature.is_empty() {
                    let signature = std::mem::take(&mut self.accumulated_signature);
                    tracing::info!(
                        "event_to_chunk: Emitting ThinkingSignature len={}",
                        signature.len()
                    );
                    Some(StreamChunk::ThinkingSignature { signature })
                } else {
                    None
                }
            }
            StreamEvent::Ping => {
                tracing::trace!("event_to_chunk: Ping (skipped)");
                None
            }
        }
    }
}
