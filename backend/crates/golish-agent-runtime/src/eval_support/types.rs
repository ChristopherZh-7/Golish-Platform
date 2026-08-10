//! Eval-only DTOs and configuration.

use std::path::PathBuf;

use rig::completion::Message;
use serde::{Deserialize, Serialize};

use golish_core::events::AiEvent;

use super::target_intel_goal_shadow::TargetIntelGoalShadowFixture;

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
#[derive(Clone)]
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
    /// Explicit eval-only Target Intel Goal shadow. Production constructors
    /// and ordinary eval defaults leave this absent.
    pub target_intel_goal_shadow: Option<TargetIntelGoalShadowFixture>,
    /// Optional fake-only host evidence adapter shared by fixture root and
    /// SubAgent execution. Production configuration has no corresponding
    /// constructor or environment fallback.
    pub intel_public_adapter:
        Option<std::sync::Arc<dyn golish_agent_kit::tool_executors::IntelPublicEvidenceAdapter>>,
}

impl std::fmt::Debug for EvalConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EvalConfig")
            .field("provider_name", &self.provider_name)
            .field("model_name", &self.model_name)
            .field("require_hitl", &self.require_hitl)
            .field("workspace", &self.workspace)
            .field("verbose", &self.verbose)
            .field("target_intel_goal_shadow", &self.target_intel_goal_shadow)
            .field(
                "intel_public_adapter_configured",
                &self.intel_public_adapter.is_some(),
            )
            .finish()
    }
}

impl Default for EvalConfig {
    fn default() -> Self {
        Self {
            provider_name: "anthropic".to_string(),
            model_name: "claude-3-sonnet".to_string(),
            require_hitl: false,
            workspace: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            verbose: false,
            target_intel_goal_shadow: None,
            intel_public_adapter: None,
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
            target_intel_goal_shadow: None,
            intel_public_adapter: None,
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
            target_intel_goal_shadow: None,
            intel_public_adapter: None,
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
            target_intel_goal_shadow: None,
            intel_public_adapter: None,
        }
    }

    /// Enable verbose output.
    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// Enables the strictly passive fake-transport shadow fixture. This
    /// method exists only on eval configuration and cannot reinterpret a
    /// production or already-created operation.
    pub fn with_target_intel_goal_shadow_fixture(mut self) -> Self {
        self.target_intel_goal_shadow = Some(TargetIntelGoalShadowFixture::strict_passive());
        self
    }

    /// Inject the fake transport/evidence adapter used by the explicit shadow
    /// fixture. Construction of the adapter itself rejects real transports.
    pub fn with_intel_public_fixture_adapter(
        mut self,
        adapter: std::sync::Arc<dyn golish_agent_kit::tool_executors::IntelPublicEvidenceAdapter>,
    ) -> Self {
        self.target_intel_goal_shadow = Some(TargetIntelGoalShadowFixture::strict_passive());
        self.intel_public_adapter = Some(adapter);
        self
    }
}
