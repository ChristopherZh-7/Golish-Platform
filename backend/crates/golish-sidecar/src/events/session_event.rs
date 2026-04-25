use std::path::PathBuf;
use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};
use uuid::Uuid;

use super::event_type::{EventType, FileOperation, DecisionType, FeedbackType};
use super::helpers::{parse_context_xml, truncate};

/// A captured session event with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEvent {
    /// Unique identifier for this event
    pub id: Uuid,
    /// Session this event belongs to
    pub session_id: String,
    /// When this event occurred
    pub timestamp: DateTime<Utc>,
    /// The type and details of this event
    pub event_type: EventType,
    /// Full content for embedding (human-readable summary)
    pub content: String,
    /// Working directory extracted from context XML (for user_prompt events)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Tool result content, truncated to 2000 chars
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_output: Option<String>,
    /// Files read/listed by tool (JSON array of paths)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files_accessed: Option<Vec<PathBuf>>,
    /// Files modified by tool (replaces old `files` field for modifications)
    pub files_modified: Vec<PathBuf>,
    /// Unified diff for edit operations, truncated to 4000 chars
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    /// 384-dimensional embedding vector (computed async)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
}

impl SessionEvent {
    /// Create a new session event
    pub fn new(session_id: String, event_type: EventType, content: String) -> Self {
        let files_modified = Self::extract_files_modified(&event_type);
        Self {
            id: Uuid::new_v4(),
            session_id,
            timestamp: Utc::now(),
            event_type,
            content,
            cwd: None,
            tool_output: None,
            files_accessed: None,
            files_modified,
            diff: None,
            embedding: None,
        }
    }

    /// Create a user prompt event
    ///
    /// Parses context XML if present:
    /// ```text
    /// <context>
    /// <cwd>/path/to/dir</cwd>
    /// <session_id>abc-123</session_id>
    /// </context>
    ///
    /// actual user message
    /// ```
    pub fn user_prompt(session_id: String, prompt: &str) -> Self {
        let (cwd, clean_prompt) = parse_context_xml(prompt);

        let mut event = Self::new(
            session_id,
            EventType::UserPrompt {
                intent: clean_prompt.clone(),
            },
            truncate(&clean_prompt, 500),
        );
        event.cwd = cwd;
        event
    }

    /// Create a file edit event
    pub fn file_edit(
        session_id: String,
        path: PathBuf,
        operation: FileOperation,
        summary: Option<String>,
    ) -> Self {
        let content = match &operation {
            FileOperation::Create => format!(
                "File created: {}{}",
                path.display(),
                summary
                    .as_ref()
                    .map(|s| format!(" - {}", s))
                    .unwrap_or_default()
            ),
            FileOperation::Modify => format!(
                "File modified: {}{}",
                path.display(),
                summary
                    .as_ref()
                    .map(|s| format!(" - {}", s))
                    .unwrap_or_default()
            ),
            FileOperation::Delete => format!(
                "File deleted: {}{}",
                path.display(),
                summary
                    .as_ref()
                    .map(|s| format!(" - {}", s))
                    .unwrap_or_default()
            ),
            FileOperation::Rename { from } => format!(
                "Renamed {} to {}{}",
                from.display(),
                path.display(),
                summary
                    .as_ref()
                    .map(|s| format!(": {}", s))
                    .unwrap_or_default()
            ),
        };

        Self::new(
            session_id,
            EventType::FileEdit {
                path: path.clone(),
                operation,
                summary,
            },
            content,
        )
    }

    /// Create a tool call event
    pub fn tool_call(
        session_id: String,
        tool_name: &str,
        args_summary: Option<String>,
        reasoning: Option<String>,
        success: bool,
    ) -> Self {
        let status = if success { "succeeded" } else { "failed" };
        let args_str = args_summary.as_deref().unwrap_or("");
        Self::new(
            session_id,
            EventType::ToolCall {
                tool_name: tool_name.to_string(),
                args_summary: args_str.to_string(),
                reasoning: reasoning.clone(),
                success,
            },
            format!(
                "Tool {} {}: {}{}",
                tool_name,
                status,
                truncate(args_str, 200),
                reasoning
                    .map(|r| format!(" ({})", truncate(&r, 100)))
                    .unwrap_or_default()
            ),
        )
    }

    /// Create a tool call event with full capture data
    ///
    /// This is the enhanced version that captures tool output, accessed files, and diffs.
    #[allow(clippy::too_many_arguments)]
    pub fn tool_call_with_output(
        session_id: String,
        tool_name: String,
        args_summary: Option<String>,
        reasoning: Option<String>,
        success: bool,
        tool_output: Option<String>,
        diff: Option<String>,
    ) -> Self {
        // Create content summary
        let args_str = args_summary.as_deref().unwrap_or("");
        let content = format!("{} {}", tool_name, args_str);

        Self {
            id: Uuid::new_v4(),
            session_id,
            timestamp: Utc::now(),
            event_type: EventType::ToolCall {
                tool_name: tool_name.to_string(),
                args_summary: args_str.to_string(),
                reasoning,
                success,
            },
            content,
            cwd: None,
            tool_output: tool_output.map(|o| truncate(&o, 2000)),
            files_accessed: None,
            files_modified: vec![],
            diff: diff.map(|d| truncate(&d, 4000)),
            embedding: None,
        }
    }

    /// Create a reasoning event
    pub fn reasoning(
        session_id: String,
        content: &str,
        decision_type: Option<DecisionType>,
    ) -> Self {
        Self::new(
            session_id,
            EventType::AgentReasoning {
                content: content.to_string(),
                decision_type,
            },
            format!("Agent reasoning: {}", truncate(content, 500)),
        )
    }

    /// Create a user feedback event
    pub fn feedback(
        session_id: String,
        feedback_type: FeedbackType,
        target_tool: Option<String>,
        comment: Option<String>,
    ) -> Self {
        let action = match feedback_type {
            FeedbackType::Approve => "approved",
            FeedbackType::Deny => "denied",
            FeedbackType::Modify => "modified",
            FeedbackType::Annotate => "annotated",
        };

        Self::new(
            session_id,
            EventType::UserFeedback {
                feedback_type,
                target_tool: target_tool.clone(),
                comment: comment.clone(),
            },
            format!(
                "User {} {}{}",
                action,
                target_tool.unwrap_or_else(|| "action".to_string()),
                comment.map(|c| format!(": {}", c)).unwrap_or_default()
            ),
        )
    }

    /// Create an error event
    pub fn error(session_id: String, error_message: &str, recovery_action: Option<String>) -> Self {
        Self::new(
            session_id,
            EventType::ErrorRecovery {
                error_message: error_message.to_string(),
                recovery_action: recovery_action.clone(),
                resolved: false,
            },
            format!(
                "Error: {}{}",
                truncate(error_message, 200),
                recovery_action
                    .map(|r| format!(" → {}", truncate(&r, 100)))
                    .unwrap_or_default()
            ),
        )
    }

    /// Create a commit boundary event
    pub fn commit_boundary(
        session_id: String,
        files: Vec<PathBuf>,
        message: Option<String>,
    ) -> Self {
        let file_count = files.len();
        Self::new(
            session_id,
            EventType::CommitBoundary {
                suggested_message: message.clone(),
                files_in_scope: files,
            },
            format!(
                "Commit boundary: {} file(s){}",
                file_count,
                message.map(|m| format!(" - {}", m)).unwrap_or_default()
            ),
        )
    }

    /// Create an AI response event
    pub fn ai_response(session_id: String, response: &str) -> Self {
        // Truncate for storage but mark if truncated
        const MAX_RESPONSE_LEN: usize = 2000;
        let truncated = response.len() > MAX_RESPONSE_LEN;
        let content = truncate(response, MAX_RESPONSE_LEN);

        Self::new(
            session_id,
            EventType::AiResponse {
                content: content.clone(),
                truncated,
                duration_ms: None,
            },
            content.to_string(),
        )
    }

    /// Extract modified file paths from event type
    fn extract_files_modified(event_type: &EventType) -> Vec<PathBuf> {
        match event_type {
            EventType::FileEdit { path, .. } => vec![path.clone()],
            EventType::CommitBoundary { files_in_scope, .. } => files_in_scope.clone(),
            _ => vec![],
        }
    }

    /// Check if this event should have an embedding generated.
    /// Returns true for:
    /// - User prompts (primary search use case)
    /// - Agent reasoning (find what the agent was thinking)
    /// - Read tool outputs (find sessions that accessed specific content)
    pub fn should_embed(&self) -> bool {
        // Always embed user prompts and reasoning
        if self.event_type.should_embed() {
            return true;
        }

        // Also embed read tool outputs (they contain file content worth searching)
        if self.event_type.is_read_tool() && self.tool_output.is_some() {
            return true;
        }

        false
    }
}
