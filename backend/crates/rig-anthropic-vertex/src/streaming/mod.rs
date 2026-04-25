//! Streaming response handling for Anthropic Vertex AI.
//!
//! Originally one 661-line file. Split here by phase of the SSE
//! lifecycle:
//!
//! - This `mod.rs`: the [`StreamingResponse`] struct + [`StreamChunk`]
//!   enum + constructor — the public API surface.
//! - [`parse`]: SSE line parsing into [`StreamEvent`]s.
//! - [`poll`]: `Stream` trait impl driving the byte-stream pump.
//! - [`event`]: `event_to_chunk` — translates parsed events into
//!   [`StreamChunk`]s, handling thinking signature accumulation,
//!   tool-use start, web-search/web-fetch results, and the final usage
//!   roll-up.
//!
//! [`StreamEvent`]: crate::types::StreamEvent

use std::pin::Pin;

use futures::Stream;

use crate::types::Usage;

mod event;
mod parse;
mod poll;

/// A streaming response from the Anthropic Vertex AI API.
pub struct StreamingResponse {
    /// The underlying byte stream.
    pub(super) inner: Pin<Box<dyn Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>>,
    /// Buffer for incomplete SSE data.
    pub(super) buffer: String,
    /// Accumulated text content.
    pub(super) accumulated_text: String,
    /// Accumulated thinking signature (for extended thinking).
    pub(super) accumulated_signature: String,
    /// Whether the stream has completed.
    pub(super) done: bool,
    /// Input tokens from `MessageStart` — Anthropic sends `input_tokens`
    /// in `message_start` but only `output_tokens` in `message_delta`,
    /// so we track them separately.
    pub(super) input_tokens: Option<u32>,
}

impl StreamingResponse {
    /// Create a new streaming response from a reqwest response.
    pub fn new(response: reqwest::Response) -> Self {
        tracing::info!("StreamingResponse::new - creating stream from response");
        tracing::debug!(
            "StreamingResponse::new - content-type: {:?}",
            response.headers().get("content-type")
        );
        tracing::debug!(
            "StreamingResponse::new - content-length: {:?}",
            response.headers().get("content-length")
        );
        Self {
            inner: Box::pin(response.bytes_stream()),
            buffer: String::new(),
            accumulated_text: String::new(),
            accumulated_signature: String::new(),
            done: false,
            input_tokens: None,
        }
    }
}

/// A chunk from the streaming response.
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// Text delta.
    TextDelta {
        text: String,
        /// Accumulated text so far (for convenience).
        #[allow(dead_code)] // Available for consumers who need running total
        accumulated: String,
    },
    /// Thinking/reasoning delta (extended thinking mode).
    ThinkingDelta { thinking: String },
    /// Thinking signature (emitted when signature is complete).
    ThinkingSignature { signature: String },
    /// Tool use started.
    ToolUseStart { id: String, name: String },
    /// Tool input delta.
    ToolInputDelta { partial_json: String },
    /// Stream completed.
    Done {
        /// The reason the stream stopped.
        #[allow(dead_code)] // Created for API completeness; pattern matched with `..`
        stop_reason: Option<String>,
        usage: Option<Usage>,
    },
    /// Error occurred.
    Error { message: String },

    // Server tool events (Claude's native web_search/web_fetch).
    /// Server tool (web_search/web_fetch) started by Claude.
    ServerToolUseStart {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    /// Web search results received from Claude's native web search.
    WebSearchResult {
        tool_use_id: String,
        results: serde_json::Value,
    },
    /// Web fetch result received from Claude's native web fetch.
    WebFetchResult {
        tool_use_id: String,
        url: String,
        content: serde_json::Value,
    },
}

#[cfg(test)]
mod tests;
