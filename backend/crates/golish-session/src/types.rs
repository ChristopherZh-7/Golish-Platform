//! Public types: roles, messages, snapshots, and session listings.
//!
//! Also hosts the small `truncate` / `strip_xml_tags` helpers used by the
//! manager and archive modules.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use rig::completion::{AssistantContent, Message};
use rig::message::UserContent;
use rig::one_or_many::OneOrMany;
use serde::{Deserialize, Serialize};


/// Role of a message in the conversation (simplified for Golish).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GolishMessageRole {
    User,
    Assistant,
    System,
    Tool,
}

/// A simplified message format for Golish sessions.
/// This provides a bridge between rig's Message type and golish-core's SessionMessage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GolishSessionMessage {
    pub role: GolishMessageRole,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_used: Option<u32>,
}

impl GolishSessionMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: GolishMessageRole::User,
            content: content.into(),
            tool_call_id: None,
            tool_name: None,
            tokens_used: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: GolishMessageRole::Assistant,
            content: content.into(),
            tool_call_id: None,
            tool_name: None,
            tokens_used: None,
        }
    }

    #[allow(dead_code)] // Public API for session construction
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: GolishMessageRole::System,
            content: content.into(),
            tool_call_id: None,
            tool_name: None,
            tokens_used: None,
        }
    }

    #[allow(dead_code)] // Public API for session construction
    pub fn tool_use(tool_name: impl Into<String>, result: impl Into<String>) -> Self {
        let tool_name = tool_name.into();
        Self {
            role: GolishMessageRole::Tool,
            content: result.into(),
            tool_call_id: None,
            tool_name: Some(tool_name),
            tokens_used: None,
        }
    }

    #[allow(dead_code)] // Public API for session construction
    pub fn tool_result(content: impl Into<String>, tool_call_id: impl Into<String>) -> Self {
        Self {
            role: GolishMessageRole::Tool,
            content: content.into(),
            tool_call_id: Some(tool_call_id.into()),
            tool_name: None,
            tokens_used: None,
        }
    }
}

/// Convert rig Message to GolishSessionMessage for persistence.
impl From<&Message> for GolishSessionMessage {
    fn from(message: &Message) -> Self {
        match message {
            Message::User { content } => {
                let text = content
                    .iter()
                    .filter_map(|c| match c {
                        rig::message::UserContent::Text(t) => Some(t.text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                Self::user(text)
            }
            Message::Assistant { content, .. } => {
                let text = content
                    .iter()
                    .filter_map(|c| match c {
                        rig::completion::AssistantContent::Text(t) => Some(t.text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                Self::assistant(text)
            }
        }
    }
}

impl GolishSessionMessage {
    /// Convert GolishSessionMessage back to rig Message for restoring sessions.
    /// Note: Tool messages are converted to assistant messages since rig's Message
    /// enum only supports User and Assistant variants for chat history.
    pub fn to_rig_message(&self) -> Option<Message> {
        match self.role {
            GolishMessageRole::User => Some(Message::User {
                content: OneOrMany::one(UserContent::Text(rig::message::Text {
                    text: self.content.clone(),
                })),
            }),
            GolishMessageRole::Assistant => Some(Message::Assistant {
                id: None,
                content: OneOrMany::one(AssistantContent::Text(rig::message::Text {
                    text: self.content.clone(),
                })),
            }),
            // System and Tool messages cannot be directly represented in rig's Message enum
            // for chat history, so we skip them (they were already processed)
            GolishMessageRole::System | GolishMessageRole::Tool => None,
        }
    }
}

/// Golish session snapshot containing conversation data.
#[cfg_attr(not(feature = "tauri"), allow(dead_code))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GolishSessionSnapshot {
    /// Session metadata
    pub workspace_label: String,
    pub workspace_path: String,
    pub model: String,
    pub provider: String,

    /// Timestamps
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,

    /// Session statistics
    pub total_messages: usize,
    pub distinct_tools: Vec<String>,

    /// Human-readable transcript lines
    pub transcript: Vec<String>,

    /// Full message history
    pub messages: Vec<GolishSessionMessage>,

    /// Associated sidecar session ID (for context restoration)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidecar_session_id: Option<String>,

    /// Total tokens used in this session
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,

    /// Agent mode used in this session ("default", "auto-approve", "planning")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_mode: Option<String>,
}

/// Session listing information for display.
#[cfg_attr(not(feature = "tauri"), allow(dead_code))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionListingInfo {
    pub identifier: String,
    pub path: PathBuf,
    pub workspace_label: String,
    pub workspace_path: String,
    pub model: String,
    pub provider: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub total_messages: usize,
    pub distinct_tools: Vec<String>,
    pub first_prompt_preview: Option<String>,
    pub first_reply_preview: Option<String>,
    /// Session status: "active", "completed", or "abandoned"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// LLM-generated session title
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// Truncate a string to a maximum length.
#[allow(dead_code)]
pub(crate) fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let mut result: String = s.chars().take(max_len.saturating_sub(1)).collect();
        result.push('…');
        result
    }
}

/// Strip XML context tags from text.
/// Removes <context>...</context>, <cwd>...</cwd>, <session_id>...</session_id> tags.
pub(crate) fn strip_xml_tags(text: &str) -> String {
    let mut result = text.to_string();

    // List of tags to strip (with their content)
    let tags = ["context", "cwd", "session_id"];

    for tag in tags {
        let open_tag = format!("<{}>", tag);
        let close_tag = format!("</{}>", tag);

        // Remove tag and its content
        while let Some(start) = result.find(&open_tag) {
            if let Some(end_offset) = result[start..].find(&close_tag) {
                let end = start + end_offset + close_tag.len();
                result = format!("{}{}", &result[..start], &result[end..]);
            } else {
                // No closing tag found, just remove opening tag
                result = result.replace(&open_tag, "");
                break;
            }
        }
    }

    result.trim().to_string()
}
