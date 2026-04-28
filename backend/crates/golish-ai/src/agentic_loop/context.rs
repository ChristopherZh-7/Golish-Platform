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

/// LLM client handle and provider-specific configuration references.
pub struct LoopLlmRefs<'a> {
    pub client: &'a Arc<RwLock<golish_llm_providers::LlmClient>>,
    pub provider_name: &'a str,
    pub model_name: &'a str,
    pub openai_web_search_config: Option<&'a golish_llm_providers::OpenAiWebSearchConfig>,
    pub openai_reasoning_effort: Option<&'a str>,
    pub openrouter_provider_preferences: Option<&'a serde_json::Value>,
    pub model_factory: Option<&'a Arc<crate::llm_client::LlmClientFactory>>,
}

/// Tool access control: policy engine, HITL approval, agent mode, loop detection.
pub struct LoopAccessControl<'a> {
    pub approval_recorder: &'a Arc<ApprovalRecorder>,
    pub pending_approvals: &'a Arc<RwLock<HashMap<String, oneshot::Sender<ApprovalDecision>>>>,
    pub tool_policy_manager: &'a Arc<ToolPolicyManager>,
    pub agent_mode: &'a Arc<RwLock<crate::agent_mode::AgentMode>>,
    pub loop_detector: &'a Arc<RwLock<LoopDetector>>,
    pub coordinator: Option<&'a CoordinatorHandle>,
}

/// Event emission, transcript, tracing, and runtime references.
pub struct LoopEventRefs<'a> {
    pub event_tx: &'a mpsc::UnboundedSender<AiEvent>,
    pub transcript_writer: Option<&'a Arc<crate::transcript::TranscriptWriter>>,
    pub transcript_base_dir: Option<&'a std::path::Path>,
    pub session_id: Option<&'a str>,
    pub db_tracker: Option<&'a crate::db_tracking::DbTracker>,
    pub runtime: Option<&'a Arc<dyn GolishRuntime>>,
}

/// Async callback invoked after a shell command completes, used to store
/// structured output (e.g. pentest tool results) without `golish-ai` depending
/// on domain-specific crates.
pub type PostShellHook = Arc<
    dyn Fn(
            Arc<sqlx::PgPool>,
            String,
            String,
            Option<String>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        + Send
        + Sync,
>;

/// Synchronous classifier that returns `true` when a shell command's output
/// already has domain-specific structured storage, so the generic memory store
/// can skip it.
pub type OutputClassifier = Arc<dyn Fn(&str, &str) -> bool + Send + Sync>;

/// Context for the agentic loop execution.
pub struct AgenticLoopContext<'a> {
    // -- Composed subsystems --------------------------------------------------
    pub llm: LoopLlmRefs<'a>,
    pub access: LoopAccessControl<'a>,
    pub events: LoopEventRefs<'a>,

    // -- Cross-cutting references ---------------------------------------------
    pub tool_registry: &'a Arc<RwLock<ToolRegistry>>,
    pub sub_agent_registry: &'a Arc<RwLock<SubAgentRegistry>>,
    pub indexer_state: Option<&'a Arc<IndexerState>>,
    pub workspace: &'a Arc<RwLock<std::path::PathBuf>>,
    pub context_manager: &'a Arc<ContextManager>,
    pub compaction_state: &'a Arc<RwLock<CompactionState>>,
    pub tool_config: &'a ToolConfig,
    pub sidecar_state: Option<&'a Arc<SidecarState>>,
    pub plan_manager: &'a Arc<crate::planner::PlanManager>,
    pub api_request_stats: &'a Arc<ApiRequestStats>,
    pub additional_tool_definitions: Vec<rig::completion::ToolDefinition>,
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
    pub cancelled: Option<&'a Arc<std::sync::atomic::AtomicBool>>,
    pub execution_monitor: Option<Arc<RwLock<crate::loop_detection::ExecutionMonitor>>>,
    pub execution_mode: crate::execution_mode::ExecutionMode,

    // -- Domain hooks (injected by the host crate) ----------------------------
    /// Called after a successful `run_pty_cmd` execution to detect and store
    /// structured output (e.g. pentest scan results) in the database.
    pub post_shell_hook: Option<PostShellHook>,
    /// Returns `true` when a shell command's output already has structured
    /// storage, so the generic memory store can skip duplicating it.
    pub output_classifier: Option<OutputClassifier>,
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
    if let Some(writer) = ctx.events.transcript_writer {
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

    let _ = ctx.events.event_tx.send(event);
}

/// Helper to emit an event to both frontend and sidecar (stateless capture)
/// Use this for events that don't need state correlation (e.g., Reasoning)
pub(super) fn emit_event(ctx: &AgenticLoopContext<'_>, event: AiEvent) {
    if let AiEvent::Reasoning { ref content } = event {
        tracing::trace!(
            "[Thinking] Emitting reasoning event to frontend: {} chars",
            content.len()
        );
    }

    if let Some(writer) = ctx.events.transcript_writer {
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

    let _ = ctx.events.event_tx.send(event.clone());

    if let Some(sidecar) = ctx.sidecar_state {
        let mut capture = CaptureContext::new(sidecar.clone());
        capture.process(&event);
    }
}
