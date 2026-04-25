use std::collections::HashMap;
use std::sync::Arc;
use rig::completion::Message;
use tokio::sync::{mpsc, oneshot, RwLock};
use golish_core::events::AiEvent;
use golish_core::hitl::ApprovalDecision;
use golish_core::runtime::GolishRuntime;
use golish_core::ApiRequestStats;
use golish_tools::ToolRegistry;
use golish_sub_agents::SubAgentRegistry;
use golish_context::{CompactionState, ContextManager};
use golish_sidecar::{CaptureContext, SidecarState};
use crate::event_coordinator::CoordinatorHandle;
use crate::hitl::ApprovalRecorder;
use crate::indexer::IndexerState;
use crate::loop_detection::LoopDetector;
use crate::tool_policy::ToolPolicyManager;
use super::ToolConfig;

/// Marker error indicating that a terminal `AiEvent::Error` has already been emitted.
///
/// `AgentBridge` uses this to avoid duplicate terminal error emission.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct TerminalErrorEmitted {
    message: String,
    partial_response: Option<String>,
    final_history: Option<Vec<Message>>,
}

impl TerminalErrorEmitted {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            partial_response: None,
            final_history: None,
        }
    }

    pub fn with_partial_state(
        message: impl Into<String>,
        partial_response: Option<String>,
        final_history: Option<Vec<Message>>,
    ) -> Self {
        Self {
            message: message.into(),
            partial_response,
            final_history,
        }
    }

    pub fn partial_response(&self) -> Option<&str> {
        self.partial_response.as_deref()
    }

    pub fn final_history(&self) -> Option<&[Message]> {
        self.final_history.as_deref()
    }
}

/// Context for the agentic loop execution.
pub struct AgenticLoopContext<'a> {
    pub event_tx: &'a mpsc::UnboundedSender<AiEvent>,
    pub tool_registry: &'a Arc<RwLock<ToolRegistry>>,
    pub sub_agent_registry: &'a Arc<RwLock<SubAgentRegistry>>,
    pub indexer_state: Option<&'a Arc<IndexerState>>,
    pub workspace: &'a Arc<RwLock<std::path::PathBuf>>,
    pub client: &'a Arc<RwLock<golish_llm_providers::LlmClient>>,
    pub approval_recorder: &'a Arc<ApprovalRecorder>,
    pub pending_approvals: &'a Arc<RwLock<HashMap<String, oneshot::Sender<ApprovalDecision>>>>,
    pub tool_policy_manager: &'a Arc<ToolPolicyManager>,
    pub context_manager: &'a Arc<ContextManager>,
    pub loop_detector: &'a Arc<RwLock<LoopDetector>>,
    /// Compaction state for tracking token usage and triggering context compaction
    pub compaction_state: &'a Arc<RwLock<CompactionState>>,
    /// Tool configuration for filtering available tools
    pub tool_config: &'a ToolConfig,
    /// Sidecar state for context capture (optional)
    pub sidecar_state: Option<&'a Arc<SidecarState>>,
    /// Runtime for auto-approve checks (optional for backward compatibility)
    pub runtime: Option<&'a Arc<dyn GolishRuntime>>,
    /// Agent mode for controlling tool approval behavior
    pub agent_mode: &'a Arc<RwLock<crate::agent_mode::AgentMode>>,
    /// Plan manager for update_plan tool
    pub plan_manager: &'a Arc<crate::planner::PlanManager>,
    /// API request stats collector (per session)
    pub api_request_stats: &'a Arc<ApiRequestStats>,
    /// Provider name for capability detection (e.g., "openai", "anthropic")
    pub provider_name: &'a str,
    /// Model name for capability detection
    pub model_name: &'a str,
    /// OpenAI web search config (if enabled)
    pub openai_web_search_config: Option<&'a golish_llm_providers::OpenAiWebSearchConfig>,
    /// OpenAI reasoning effort level (if set)
    pub openai_reasoning_effort: Option<&'a str>,
    /// OpenRouter provider preferences JSON for routing and filtering (if set)
    pub openrouter_provider_preferences: Option<&'a serde_json::Value>,
    /// Factory for creating sub-agent model override clients (optional)
    pub model_factory: Option<&'a Arc<crate::llm_client::LlmClientFactory>>,
    /// Session ID for Langfuse trace grouping (optional)
    pub session_id: Option<&'a str>,
    /// Transcript writer for persisting AI events (optional)
    pub transcript_writer: Option<&'a Arc<crate::transcript::TranscriptWriter>>,
    /// Base directory for transcript files (e.g., `~/.golish/transcripts`)
    /// Used to create separate transcript files for sub-agent internal events.
    pub transcript_base_dir: Option<&'a std::path::Path>,
    /// Additional tool definitions to include (e.g., SWE-bench test tool).
    /// These are added to the tool list alongside the standard tools.
    pub additional_tool_definitions: Vec<rig::completion::ToolDefinition>,
    /// Custom tool executor for handling additional tools.
    /// If provided, this function is called for tools not handled by the standard executors.
    /// Returns `Some((result, success))` if the tool was handled, `None` otherwise.
    #[allow(clippy::type_complexity)]
    pub custom_tool_executor: Option<
        std::sync::Arc<
            dyn Fn(
                    &str,
                    &serde_json::Value,
                ) -> std::pin::Pin<
                    Box<dyn std::future::Future<Output = Option<(serde_json::Value, bool)>> + Send>,
                > + Send
                + Sync,
        >,
    >,
    /// Event coordinator for message-passing based event management (optional).
    /// When available, approval registration uses the coordinator instead of pending_approvals.
    pub coordinator: Option<&'a CoordinatorHandle>,
    /// Database tracker for background recording of tool calls, token usage, etc.
    pub db_tracker: Option<&'a crate::db_tracking::DbTracker>,
    /// Cancellation flag: checked between loop iterations to support user-initiated stop.
    pub cancelled: Option<&'a Arc<std::sync::atomic::AtomicBool>>,
    /// Execution monitor for the Mentor pattern (PentAGI-style).
    /// When present, tracks tool call patterns and the agentic loop can
    /// inject mentor advice into tool results when the monitor triggers.
    pub execution_monitor: Option<Arc<RwLock<crate::loop_detection::ExecutionMonitor>>>,
    /// Execution mode: Chat (all tools) vs Task (delegation-only).
    /// In Task mode the primary agent loses direct environment access (shell, file, web)
    /// and must delegate to sub-agents, matching PentAGI's primary agent pattern.
    pub execution_mode: crate::execution_mode::ExecutionMode,
}

/// Check cancellation flag; returns true when the user has requested a stop.
pub(super) fn is_cancelled(ctx: &AgenticLoopContext<'_>) -> bool {
    ctx.cancelled
        .map(|f| f.load(std::sync::atomic::Ordering::SeqCst))
        .unwrap_or(false)
}

/// Result of a single tool execution.
pub struct ToolExecutionResult {
    pub value: serde_json::Value,
    pub success: bool,
}

/// Wrapper for capture context that persists across the loop
pub struct LoopCaptureContext {
    inner: Option<std::sync::Mutex<CaptureContext>>,
}

impl LoopCaptureContext {
    /// Create a new loop capture context
    pub fn new(sidecar: Option<&Arc<SidecarState>>) -> Self {
        Self {
            inner: sidecar.map(|s| std::sync::Mutex::new(CaptureContext::new(s.clone()))),
        }
    }

    /// Process an event if capture is enabled
    pub fn process(&self, event: &AiEvent) {
        if let Some(ref capture) = self.inner {
            if let Ok(mut guard) = capture.lock() {
                guard.process(event);
            }
        }
    }
}

/// Helper to emit an event to frontend and transcript (but not sidecar)
/// Use this when sidecar capture is handled separately (e.g., with stateful capture_ctx)
pub(super) fn emit_to_frontend(ctx: &AgenticLoopContext<'_>, event: AiEvent) {
    // Write to transcript if configured (skip streaming events)
    if let Some(writer) = ctx.transcript_writer {
        if crate::transcript::should_transcript(&event) {
            let writer = Arc::clone(writer);
            let event_clone = event.clone();
            tokio::spawn(async move {
                if let Err(e) = writer.append(&event_clone).await {
                    tracing::warn!("Failed to write to transcript: {}", e);
                }
            });
        }
    }

    let _ = ctx.event_tx.send(event);
}

/// Helper to emit an event to both frontend and sidecar (stateless capture)
/// Use this for events that don't need state correlation (e.g., Reasoning)
pub(super) fn emit_event(ctx: &AgenticLoopContext<'_>, event: AiEvent) {
    // Log reasoning events being emitted to frontend (trace level to reduce spam)
    if let AiEvent::Reasoning { ref content } = event {
        tracing::trace!(
            "[Thinking] Emitting reasoning event to frontend: {} chars",
            content.len()
        );
    }

    // Write to transcript if configured (skip streaming events)
    if let Some(writer) = ctx.transcript_writer {
        if crate::transcript::should_transcript(&event) {
            let writer = Arc::clone(writer);
            let event_clone = event.clone();
            tokio::spawn(async move {
                if let Err(e) = writer.append(&event_clone).await {
                    tracing::warn!("Failed to write to transcript: {}", e);
                }
            });
        }
    }

    // Send to frontend
    let _ = ctx.event_tx.send(event.clone());

    // Capture in sidecar if available (stateless - creates fresh context each time)
    if let Some(sidecar) = ctx.sidecar_state {
        let mut capture = CaptureContext::new(sidecar.clone());
        capture.process(&event);
    }
}
