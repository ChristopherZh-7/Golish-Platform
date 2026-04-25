//! Request types: tool definitions + the [`CompletionRequest`] body.

use serde::{Deserialize, Serialize};

use super::messages::{CacheControl, Message, SystemBlock, ThinkingConfig};
use super::web_tools::ToolEntry;
use super::{ANTHROPIC_VERSION, DEFAULT_MAX_TOKENS};

/// Tool definition for the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    /// Optional cache control marker for caching tool definitions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

/// Request body for the Anthropic Vertex AI API.
#[derive(Debug, Clone, Serialize)]
pub struct CompletionRequest {
    /// Anthropic API version.
    pub anthropic_version: String,
    /// Messages in the conversation.
    pub messages: Vec<Message>,
    /// Maximum tokens to generate.
    pub max_tokens: u32,
    /// System prompt as array of blocks (required for caching).
    /// If `None`, no system prompt is sent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Vec<SystemBlock>>,
    /// Temperature for sampling (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Top-p sampling (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    /// Top-k sampling (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    /// Stop sequences (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
    /// Tools available to the model (optional). Can contain both
    /// function tools and server tools (web_search, web_fetch).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolEntry>>,
    /// Whether to stream the response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    /// Extended thinking configuration (optional). When enabled,
    /// `temperature` must be `1` and `budget_tokens` >= 1024.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingConfig>,
}

impl Default for CompletionRequest {
    fn default() -> Self {
        Self {
            anthropic_version: ANTHROPIC_VERSION.to_string(),
            messages: Vec::new(),
            max_tokens: DEFAULT_MAX_TOKENS,
            system: None,
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: None,
            tools: None,
            stream: None,
            thinking: None,
        }
    }
}
