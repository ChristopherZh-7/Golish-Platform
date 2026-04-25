//! SSE event types for streaming completions.

use serde::{Deserialize, Serialize};

use super::messages::ContentBlock;
use super::response::{StopReason, Usage};

/// Streaming event types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// Initial message start event.
    MessageStart { message: StreamMessageStart },
    /// Content block started.
    ContentBlockStart {
        index: usize,
        content_block: ContentBlock,
    },
    /// Delta for content block.
    ContentBlockDelta { index: usize, delta: ContentDelta },
    /// Content block finished.
    ContentBlockStop { index: usize },
    /// Final message delta with usage.
    MessageDelta {
        delta: MessageDeltaContent,
        usage: Usage,
    },
    /// Message complete.
    MessageStop,
    /// Ping event (keep-alive).
    Ping,
    /// Error event.
    Error { error: StreamError },
}

/// Message start in streaming.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamMessageStart {
    pub id: String,
    #[serde(rename = "type")]
    pub message_type: String,
    pub role: String,
    pub model: String,
    pub usage: Usage,
}

/// Content delta in streaming.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentDelta {
    TextDelta {
        text: String,
    },
    InputJsonDelta {
        partial_json: String,
    },
    /// Thinking content delta (streamed reasoning).
    ThinkingDelta {
        thinking: String,
    },
    /// Signature delta for thinking blocks.
    SignatureDelta {
        signature: String,
    },
    /// Citations delta from Claude's web search.
    CitationsDelta {
        citation: Citation,
    },
}

/// Citation from Claude's web search.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Citation {
    WebSearchResultLocation {
        cited_text: String,
        url: String,
        title: String,
        encrypted_index: String,
    },
}

/// Message delta content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageDeltaContent {
    pub stop_reason: Option<StopReason>,
    pub stop_sequence: Option<String>,
}

/// Error in streaming.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamError {
    #[serde(rename = "type")]
    pub error_type: String,
    pub message: String,
}
