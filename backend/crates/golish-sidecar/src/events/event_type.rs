use std::path::PathBuf;
use serde::{Serialize, Deserialize};

// =============================================================================
// Session Capture Events - Stored for later query/analysis
// =============================================================================

/// Types of events captured by the sidecar
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventType {
    /// User prompt with their stated intent
    UserPrompt {
        /// What the user asked for
        intent: String,
    },

    /// File modification with context
    FileEdit {
        /// Path to the file
        path: PathBuf,
        /// Type of operation performed
        operation: FileOperation,
        /// One-line description if available
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
    },

    /// Tool call with reasoning
    ToolCall {
        /// Name of the tool invoked
        tool_name: String,
        /// Truncated/summarized args
        args_summary: String,
        /// Why the agent made this call
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning: Option<String>,
        /// Whether the tool call succeeded
        success: bool,
    },

    /// Agent's explicit reasoning (from extended thinking or text)
    AgentReasoning {
        /// The reasoning content
        content: String,
        /// Type of decision being made
        #[serde(skip_serializing_if = "Option::is_none")]
        decision_type: Option<DecisionType>,
    },

    /// User feedback on agent action
    UserFeedback {
        /// Type of feedback
        feedback_type: FeedbackType,
        /// Tool that was being approved/denied
        #[serde(skip_serializing_if = "Option::is_none")]
        target_tool: Option<String>,
        /// User's comment if any
        #[serde(skip_serializing_if = "Option::is_none")]
        comment: Option<String>,
    },

    /// Error with recovery attempt
    ErrorRecovery {
        /// The error message
        error_message: String,
        /// What action was taken to recover
        #[serde(skip_serializing_if = "Option::is_none")]
        recovery_action: Option<String>,
        /// Whether the error was resolved
        resolved: bool,
    },

    /// Commit boundary marker (detected or explicit)
    CommitBoundary {
        /// Suggested commit message if available
        #[serde(skip_serializing_if = "Option::is_none")]
        suggested_message: Option<String>,
        /// Files that should be included in this commit
        files_in_scope: Vec<PathBuf>,
    },

    /// Session started
    SessionStart {
        /// Initial user request
        initial_request: String,
    },

    /// Session ended
    SessionEnd {
        /// Final summary if available
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
    },

    /// AI response (final accumulated text)
    AiResponse {
        /// The response content (truncated for storage)
        content: String,
        /// Whether this was a complete response or truncated
        truncated: bool,
        /// Duration in milliseconds
        #[serde(skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
    },
}

impl EventType {
    /// Get a short name for this event type
    pub fn name(&self) -> &'static str {
        match self {
            EventType::UserPrompt { .. } => "user_prompt",
            EventType::FileEdit { .. } => "file_edit",
            EventType::ToolCall { .. } => "tool_call",
            EventType::AgentReasoning { .. } => "reasoning",
            EventType::UserFeedback { .. } => "feedback",
            EventType::ErrorRecovery { .. } => "error",
            EventType::CommitBoundary { .. } => "commit_boundary",
            EventType::SessionStart { .. } => "session_start",
            EventType::SessionEnd { .. } => "session_end",
            EventType::AiResponse { .. } => "ai_response",
        }
    }

    /// Check if this is a high-signal event worth including in checkpoints
    pub fn is_high_signal(&self) -> bool {
        matches!(
            self,
            EventType::UserPrompt { .. }
                | EventType::FileEdit { .. }
                | EventType::AgentReasoning { .. }
                | EventType::UserFeedback { .. }
                | EventType::CommitBoundary { .. }
                | EventType::AiResponse { .. }
        )
    }

    /// Check if this event type should have embeddings generated for semantic search.
    /// Returns true for events with searchable semantic content.
    pub fn should_embed(&self) -> bool {
        matches!(
            self,
            EventType::UserPrompt { .. } | EventType::AgentReasoning { .. }
        )
    }

    /// Check if this is a read tool that accessed file content.
    /// Used to determine if tool_output should be embedded.
    pub fn is_read_tool(&self) -> bool {
        match self {
            EventType::ToolCall { tool_name, .. } => {
                matches!(
                    tool_name.as_str(),
                    "read" | "read_file" | "Read" | "cat" | "grep" | "Grep" | "glob" | "Glob"
                )
            }
            _ => false,
        }
    }
}

/// File operation types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileOperation {
    /// File was created
    Create,
    /// File was modified
    Modify,
    /// File was deleted
    Delete,
    /// File was renamed
    Rename {
        /// Original path before rename
        from: PathBuf,
    },
}

/// Types of decisions the agent makes
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DecisionType {
    /// "I'll use X instead of Y because..."
    ApproachChoice,
    /// "This sacrifices A for B"
    Tradeoff,
    /// "Since X didn't work, trying Y"
    Fallback,
    /// "Assuming the user wants..."
    Assumption,
}

/// Types of user feedback
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackType {
    /// User approved the action
    Approve,
    /// User denied the action
    Deny,
    /// User modified the action
    Modify,
    /// User added a comment/annotation
    Annotate,
}
