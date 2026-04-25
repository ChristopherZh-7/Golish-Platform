//! Response types: usage stats, stop reason, and the
//! [`CompletionResponse`] container with text/tool-uses/thinking
//! accessors.

use serde::{Deserialize, Serialize};

use super::messages::ContentBlock;

/// Usage statistics in the response.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Usage {
    /// Input tokens (may be missing in `message_delta` events).
    #[serde(default)]
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Tokens used to create new cache entries.
    #[serde(default)]
    pub cache_creation_input_tokens: u32,
    /// Tokens read from cache (cache hit).
    #[serde(default)]
    pub cache_read_input_tokens: u32,
}

/// Stop reason for completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    StopSequence,
    ToolUse,
}

/// Response from the Anthropic Vertex AI API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    /// Unique ID for the response.
    pub id: String,
    /// Type of response (always `"message"`).
    #[serde(rename = "type")]
    pub response_type: String,
    /// Role (always `"assistant"`).
    pub role: String,
    /// Content blocks.
    pub content: Vec<ContentBlock>,
    /// Model that generated the response.
    pub model: String,
    /// Reason the model stopped generating.
    pub stop_reason: Option<StopReason>,
    /// Stop sequence that triggered stopping (if applicable).
    pub stop_sequence: Option<String>,
    /// Token usage statistics.
    pub usage: Usage,
}

impl CompletionResponse {
    /// Extract text content from the response.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    /// Extract tool use blocks from the response.
    pub fn tool_uses(&self) -> Vec<(&str, &str, &serde_json::Value)> {
        self.content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolUse { id, name, input } => {
                    Some((id.as_str(), name.as_str(), input))
                }
                _ => None,
            })
            .collect()
    }

    /// Extract thinking/reasoning content from the response.
    pub fn thinking(&self) -> Option<&str> {
        self.content.iter().find_map(|block| match block {
            ContentBlock::Thinking { thinking, .. } => Some(thinking.as_str()),
            _ => None,
        })
    }
}
