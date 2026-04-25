//! Stream-event mapping for the Anthropic Vertex provider.
//!
//! Translates each [`crate::streaming::StreamChunk`] coming off the SSE
//! decoder into a single rig [`RawStreamingChoice`].  Lives in its own
//! module because the chunk shape is rich (text / tool / thinking /
//! signature / server-tool / web-search / web-fetch) and the mapping is
//! the one place we have to keep aligned with both Anthropic's wire format
//! and rig's streaming abstraction.

use rig::message::ReasoningContent;
use rig::streaming::{RawStreamingChoice, RawStreamingToolCall, ToolCallDeltaContent};

use crate::response::StreamingCompletionResponseData;
use crate::streaming::StreamChunk;

/// Translate a single Anthropic stream chunk into a rig `RawStreamingChoice`.
///
/// The mapping is total: every `StreamChunk` variant produces some
/// `RawStreamingChoice`, even when the only thing we can do is surface an
/// out-of-band event (server-tool result, web-search payload) as a tagged
/// message that the agentic loop knows how to parse.
pub(crate) fn map_stream_chunk(chunk: StreamChunk) -> RawStreamingChoice<StreamingCompletionResponseData> {
    match chunk {
        StreamChunk::TextDelta { text, .. } => RawStreamingChoice::Message(text),
        StreamChunk::ToolUseStart { id, name } => {
            RawStreamingChoice::ToolCall(RawStreamingToolCall {
                id: id.clone(),
                internal_call_id: nanoid::nanoid!(),
                call_id: Some(id),
                name,
                arguments: serde_json::json!({}), // Must be a valid object
                signature: None,
                additional_params: None,
            })
        }
        StreamChunk::ToolInputDelta { partial_json } => RawStreamingChoice::ToolCallDelta {
            id: String::new(),
            internal_call_id: nanoid::nanoid!(),
            content: ToolCallDeltaContent::Delta(partial_json),
        },
        StreamChunk::Done { usage, .. } => {
            // Return final response with usage info
            RawStreamingChoice::FinalResponse(StreamingCompletionResponseData {
                text: String::new(),
                usage,
            })
        }
        StreamChunk::Error { message } => {
            // Can't return error directly, emit as message
            RawStreamingChoice::Message(format!("[Error: {}]", message))
        }
        StreamChunk::ThinkingDelta { thinking } => {
            // Emit thinking content using native reasoning type
            RawStreamingChoice::Reasoning {
                id: None,
                content: ReasoningContent::Text {
                    text: thinking,
                    signature: None,
                },
            }
        }
        StreamChunk::ThinkingSignature { signature } => {
            // Emit signature as a Reasoning event (empty text, signature set)
            RawStreamingChoice::Reasoning {
                id: None,
                content: ReasoningContent::Text {
                    text: String::new(),
                    signature: Some(signature),
                },
            }
        }
        // Server tool events - emit as tool calls for now
        // The agentic loop will handle these specially
        StreamChunk::ServerToolUseStart { id, name, input } => {
            tracing::info!("Server tool started: {} ({})", name, id);
            RawStreamingChoice::ToolCall(RawStreamingToolCall {
                id: id.clone(),
                internal_call_id: nanoid::nanoid!(),
                call_id: Some(format!("server:{}", id)),
                name,
                arguments: input,
                signature: None,
                additional_params: None,
            })
        }
        StreamChunk::WebSearchResult {
            tool_use_id,
            results,
        } => {
            // Emit as a special message that can be parsed by the agentic loop
            tracing::info!("Web search results received for {}", tool_use_id);
            RawStreamingChoice::Message(format!(
                "[WEB_SEARCH_RESULT:{}:{}]",
                tool_use_id,
                serde_json::to_string(&results).unwrap_or_default()
            ))
        }
        StreamChunk::WebFetchResult {
            tool_use_id,
            url,
            content,
        } => {
            // Emit as a special message that can be parsed by the agentic loop
            tracing::info!(
                "Web fetch result received for {}: {}",
                tool_use_id,
                url
            );
            RawStreamingChoice::Message(format!(
                "[WEB_FETCH_RESULT:{}:{}:{}]",
                tool_use_id,
                url,
                serde_json::to_string(&content).unwrap_or_default()
            ))
        }
    }
}
