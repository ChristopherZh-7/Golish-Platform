//! Message-shape types: thinking + cache + system blocks, content blocks,
//! image source, role, and the [`Message`] container.

use serde::{Deserialize, Serialize};

/// Configuration for extended thinking (reasoning) mode. When enabled,
/// the model will show its reasoning process before responding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingConfig {
    /// Must be `"enabled"` to activate extended thinking.
    #[serde(rename = "type")]
    pub thinking_type: String,
    /// Token budget for thinking (must be >= 1024).
    pub budget_tokens: u32,
}

impl ThinkingConfig {
    /// Create a new thinking configuration with the specified budget.
    /// Budget must be at least 1024 tokens.
    pub fn new(budget_tokens: u32) -> Self {
        Self {
            thinking_type: "enabled".to_string(),
            budget_tokens: budget_tokens.max(1024),
        }
    }

    /// Create a thinking config with a default budget of 10,000 tokens.
    pub fn default_budget() -> Self {
        Self::new(10_000)
    }
}

/// Cache control configuration for prompt caching. When set, marks
/// content as cacheable with the specified type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheControl {
    /// The cache type. Currently only `"ephemeral"` is supported.
    #[serde(rename = "type")]
    pub cache_type: String,
}

impl CacheControl {
    /// Create an ephemeral cache control marker. Cached content has a
    /// 5-minute TTL, refreshed on each hit.
    pub fn ephemeral() -> Self {
        Self {
            cache_type: "ephemeral".to_string(),
        }
    }
}

/// A block in the system prompt array. Required for prompt caching —
/// the single-string format does not support `cache_control`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemBlock {
    /// Block type (always `"text"` for system prompts).
    #[serde(rename = "type")]
    pub block_type: String,
    /// The text content.
    pub text: String,
    /// Optional cache control marker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

impl SystemBlock {
    /// Create a new text system block without caching.
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            block_type: "text".to_string(),
            text: content.into(),
            cache_control: None,
        }
    }

    /// Create a new text system block with ephemeral caching.
    pub fn cached(content: impl Into<String>) -> Self {
        Self {
            block_type: "text".to_string(),
            text: content.into(),
            cache_control: Some(CacheControl::ephemeral()),
        }
    }
}

/// Content block in a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Text content.
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    /// Image content (base64 encoded).
    Image {
        source: ImageSource,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    /// Tool use request from the model.
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    /// Tool result from execution.
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    /// Thinking/reasoning content from extended thinking mode.
    Thinking {
        thinking: String,
        /// Signature for verification (provided by API).
        signature: String,
    },
    /// Server tool use (Claude's native web_search/web_fetch). These
    /// are initiated by Claude and executed server-side.
    ServerToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    /// Web search tool result from Claude's native web search.
    WebSearchToolResult {
        tool_use_id: String,
        content: serde_json::Value, // WebSearchToolResultContent
    },
    /// Web fetch tool result from Claude's native web fetch.
    WebFetchToolResult {
        tool_use_id: String,
        content: serde_json::Value, // WebFetchToolResultContent
    },
}

/// Image source for image content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub media_type: String,
    pub data: String,
}

/// Role in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// A message in the conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    /// Create a user message with text content.
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: text.into(),
                cache_control: None,
            }],
        }
    }

    /// Create an assistant message with text content.
    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: vec![ContentBlock::Text {
                text: text.into(),
                cache_control: None,
            }],
        }
    }
}
