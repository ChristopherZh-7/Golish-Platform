//! Eval-only DTOs and configuration.

use std::path::PathBuf;

use rig::completion::Message;
use serde::{Deserialize, Serialize};

use golish_core::events::AiEvent;

/// A tool call captured during eval execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalToolCall {
    /// Name of the tool that was called
    pub name: String,
    /// Input arguments to the tool
    pub input: serde_json::Value,
    /// Output from the tool (if available)
    pub output: Option<String>,
    /// Whether the tool execution was successful
    pub success: bool,
}

/// Output from an eval agentic loop run.
#[derive(Debug, Clone)]
pub struct EvalAgentOutput {
    /// Final text response from the agent.
    pub response: String,
    /// All tool calls made during execution.
    pub tool_calls: Vec<EvalToolCall>,
    /// Files that were modified during execution.
    pub files_modified: Vec<PathBuf>,
    /// Duration of execution in milliseconds.
    pub duration_ms: u64,
    /// Token usage (total tokens used).
    pub tokens_used: Option<u32>,
    /// Message history from the conversation.
    pub history: Vec<Message>,
    /// Raw events emitted during execution (for debugging).
    pub events: Vec<AiEvent>,
}

/// Configuration for eval execution.
#[derive(Debug, Clone)]
pub struct EvalConfig {
    /// Provider name for capability detection (e.g., "openai", "anthropic")
    pub provider_name: String,
    /// Model name for capability detection
    pub model_name: String,
    /// Whether to require HITL (always false for evals - auto-approve)
    pub require_hitl: bool,
    /// Workspace directory for tool execution
    pub workspace: PathBuf,
    /// Whether to print live output (tool calls, reasoning, etc.)
    pub verbose: bool,
}

impl Default for EvalConfig {
    fn default() -> Self {
        Self {
            provider_name: "anthropic".to_string(),
            model_name: "claude-3-sonnet".to_string(),
            require_hitl: false,
            workspace: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            verbose: false,
        }
    }
}

impl EvalConfig {
    /// Create config for OpenAI provider.
    pub fn openai(model_name: &str, workspace: PathBuf) -> Self {
        Self {
            provider_name: "openai".to_string(),
            model_name: model_name.to_string(),
            require_hitl: false,
            workspace,
            verbose: false,
        }
    }

    /// Create config for Anthropic provider.
    pub fn anthropic(model_name: &str, workspace: PathBuf) -> Self {
        Self {
            provider_name: "anthropic".to_string(),
            model_name: model_name.to_string(),
            require_hitl: false,
            workspace,
            verbose: false,
        }
    }

    /// Create config for Vertex AI provider.
    pub fn vertex_ai(model_name: &str, workspace: PathBuf) -> Self {
        Self {
            provider_name: "vertex_ai".to_string(),
            model_name: model_name.to_string(),
            require_hitl: false,
            workspace,
            verbose: false,
        }
    }

    /// Enable verbose output.
    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }
}
