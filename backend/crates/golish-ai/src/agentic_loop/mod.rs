//! Agentic tool loop for LLM execution.
//!
//! This module contains the main agentic loop that handles:
//! - Tool execution with HITL approval
//! - Loop detection and prevention
//! - Context window management
//! - Message history management
//! - Extended thinking (streaming reasoning content)

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use futures::StreamExt;
use rig::completion::{
    AssistantContent, CompletionModel as RigCompletionModel, GetTokenUsage, Message,
};
use rig::message::{
    Reasoning, ReasoningContent, Text, ToolCall, ToolResult, ToolResultContent, UserContent,
};
use rig::one_or_many::OneOrMany;
use rig::streaming::StreamedAssistantContent;
use serde_json::json;
use tokio::sync::{mpsc, oneshot, RwLock};
use tracing::Instrument;

use golish_tools::ToolRegistry;

use super::system_hooks::{format_system_hooks, HookRegistry, MessageHookContext, PostToolContext};
use super::tool_definitions::{
    get_all_tool_definitions_with_config, get_ask_human_tool_definition,
    get_run_command_tool_definition, get_sub_agent_tool_definitions, sanitize_schema, ToolConfig,
};
use super::tool_executors::normalize_run_pty_cmd_args;
use crate::hitl::ApprovalRecorder;
use crate::indexer::IndexerState;
use crate::loop_detection::LoopDetector;
use crate::tool_policy::ToolPolicyManager;
use golish_context::token_budget::TokenUsage;
use golish_context::{CompactionState, ContextManager};
use golish_core::events::AiEvent;
use golish_core::hitl::ApprovalDecision;
use golish_core::runtime::GolishRuntime;
use golish_core::utils::truncate_str;
use golish_core::ApiRequestStats;
use golish_llm_providers::ModelCapabilities;
use golish_sidecar::{CaptureContext, SidecarState};
use golish_sub_agents::{SubAgentContext, SubAgentRegistry, MAX_AGENT_DEPTH};

use crate::event_coordinator::CoordinatorHandle;

mod helpers;
mod sub_agent_dispatch;
mod tool_execution;
pub mod toolcall_fixer;

use helpers::{estimate_message_tokens, handle_loop_detection};
use sub_agent_dispatch::{detect_repetitive_text, partition_tool_calls};
pub use tool_execution::{
    execute_tool_direct_generic, execute_with_hitl_generic,
};

/// Maximum number of tool call iterations before stopping
pub const MAX_TOOL_ITERATIONS: usize = 100;

/// Timeout for approval requests in seconds (30 minutes)
pub const APPROVAL_TIMEOUT_SECS: u64 = 1800;

/// Maximum tokens for a single completion request
pub const MAX_COMPLETION_TOKENS: u32 = 10_000;

mod stream_retry;
use stream_retry::*;

pub mod compaction;
pub use compaction::{
    apply_compaction, get_artifacts_dir, get_artifacts_dir_for, get_summaries_dir,
    get_summaries_dir_for, get_transcript_dir, get_transcript_dir_for, maybe_compact,
    CompactionResult,
};

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
    pub agent_mode: &'a Arc<RwLock<super::agent_mode::AgentMode>>,
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
    pub model_factory: Option<&'a Arc<super::llm_client::LlmClientFactory>>,
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
fn emit_to_frontend(ctx: &AgenticLoopContext<'_>, event: AiEvent) {
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
fn emit_event(ctx: &AgenticLoopContext<'_>, event: AiEvent) {
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


/// Execute the main agentic loop with tool calling.
///
/// This function runs the LLM completion loop, handling:
/// - Tool calls and results
/// - Loop detection
/// - Context window management
/// - HITL approval
/// - Extended thinking (streaming reasoning content)
///
/// Returns a tuple of (response_text, message_history, token_usage)
///
/// Note: This is the Anthropic-specific entry point that delegates to the unified loop
/// with thinking history support enabled.
///
/// Returns: (response, reasoning, history, token_usage)
pub async fn run_agentic_loop(
    model: &rig_anthropic_vertex::CompletionModel,
    system_prompt: &str,
    initial_history: Vec<Message>,
    context: SubAgentContext,
    ctx: &AgenticLoopContext<'_>,
) -> Result<(String, Option<String>, Vec<Message>, Option<TokenUsage>)> {
    // Delegate to unified loop with Anthropic configuration (thinking history enabled)
    run_agentic_loop_unified(
        model,
        system_prompt,
        initial_history,
        context,
        ctx,
        AgenticLoopConfig::main_agent_anthropic(),
    )
    .await
}


/// Generic agentic loop that works with any rig CompletionModel.
///
/// This is a simplified version of `run_agentic_loop` that:
/// - Works with any model implementing `rig::completion::CompletionModel`
/// - Does NOT support extended thinking (Anthropic-specific)
/// - Supports sub-agent calls (uses the same model for sub-agents)
///
/// Returns: (response, reasoning, history, token_usage)
///
/// Note: This is the generic entry point that delegates to the unified loop.
/// Model capabilities are detected from the provider/model name in the context.
pub async fn run_agentic_loop_generic<M>(
    model: &M,
    system_prompt: &str,
    initial_history: Vec<Message>,
    context: SubAgentContext,
    ctx: &AgenticLoopContext<'_>,
) -> Result<(String, Option<String>, Vec<Message>, Option<TokenUsage>)>
where
    M: RigCompletionModel + Sync,
{
    // Detect capabilities from provider/model name for proper temperature handling
    let config = AgenticLoopConfig::with_detection(ctx.provider_name, ctx.model_name, false);

    // Delegate to unified loop with detected configuration
    run_agentic_loop_unified(model, system_prompt, initial_history, context, ctx, config).await
}

// ============================================================================
// UNIFIED AGENTIC LOOP (Phase 1.3)
// ============================================================================

/// Configuration for the unified agentic loop.
///
/// This struct controls model-specific behavior in the unified loop,
/// allowing it to handle both Anthropic-style (thinking-enabled) and
/// generic model execution paths.
#[derive(Debug, Clone)]
pub struct AgenticLoopConfig {
    /// Model capabilities (thinking support, temperature, etc.)
    pub capabilities: ModelCapabilities,
    /// Whether HITL approval is required for tool execution.
    pub require_hitl: bool,
    /// Whether this is a sub-agent execution (affects tool restrictions).
    pub is_sub_agent: bool,
    /// Whether to invoke the reflector agent when the model produces text
    /// without any tool calls. Default: false (only enabled for main agent).
    pub enable_reflector: bool,
    /// Tool names hint passed to the reflector so it can suggest specific tools.
    pub tool_names_for_reflector: Option<Vec<String>>,
}

impl AgenticLoopConfig {
    /// Create config for main agent with Anthropic model.
    ///
    /// Anthropic models support extended thinking (reasoning history tracking)
    /// and require HITL approval for tool execution.
    pub fn main_agent_anthropic() -> Self {
        Self {
            capabilities: ModelCapabilities::anthropic_defaults(),
            require_hitl: true,
            is_sub_agent: false,
            enable_reflector: true,
            tool_names_for_reflector: None,
        }
    }

    /// Create config for main agent with generic model.
    ///
    /// Generic models use conservative defaults (no thinking history tracking)
    /// and require HITL approval for tool execution.
    pub fn main_agent_generic() -> Self {
        Self {
            capabilities: ModelCapabilities::conservative_defaults(),
            require_hitl: true,
            is_sub_agent: false,
            enable_reflector: true,
            tool_names_for_reflector: None,
        }
    }

    /// Create config for sub-agent (trusted, no HITL).
    ///
    /// Sub-agents are trusted and do not require HITL approval.
    /// The capabilities should match the model being used.
    pub fn sub_agent(capabilities: ModelCapabilities) -> Self {
        Self {
            capabilities,
            require_hitl: false,
            is_sub_agent: true,
            enable_reflector: false,
            tool_names_for_reflector: None,
        }
    }

    /// Create config with detected capabilities based on provider and model name.
    ///
    /// This factory method detects capabilities automatically and is useful
    /// when calling from code that has provider/model info but not an LlmClient.
    pub fn with_detection(provider_name: &str, model_name: &str, is_sub_agent: bool) -> Self {
        Self {
            capabilities: ModelCapabilities::detect(provider_name, model_name),
            require_hitl: !is_sub_agent,
            is_sub_agent,
            enable_reflector: !is_sub_agent,
            tool_names_for_reflector: None,
        }
    }
}

/// Unified agentic loop that handles all model types.
///
/// This function replaces both `run_agentic_loop` (Anthropic) and
/// `run_agentic_loop_generic` by using configuration to control behavior.
///
/// # Key Differences from Separate Loops
///
/// 1. **Thinking History**: When `config.capabilities.supports_thinking_history` is true,
///    reasoning content from the model is preserved in the message history
///    (required by Anthropic API when extended thinking is enabled).
///
/// 2. **HITL Approval**: When `config.require_hitl` is true, tool execution
///    requires human-in-the-loop approval (unless auto-approved by policy).
///
/// 3. **Sub-Agent Restrictions**: When `config.is_sub_agent` is true,
///    certain tool restrictions may apply.
///
/// # Arguments
/// * `model` - The completion model to use
/// * `system_prompt` - System prompt for the agent
/// * `initial_history` - Starting conversation history
/// * `sub_agent_context` - Sub-agent execution context (includes depth tracking)
/// * `ctx` - Agent loop context with dependencies
/// * `config` - Configuration controlling behavior
///
/// # Returns
/// Tuple of (response_text, updated_history, token_usage)
///
/// # Example
/// ```ignore
/// use golish_ai::agentic_loop::{run_agentic_loop_unified, AgenticLoopConfig};
///
/// // For Anthropic models (with thinking support)
/// let config = AgenticLoopConfig::main_agent_anthropic();
/// let (response, history, usage) = run_agentic_loop_unified(
///     &model, system_prompt, history, context, &ctx, config
/// ).await?;
///
/// // For generic models (without thinking support)
/// let config = AgenticLoopConfig::main_agent_generic();
/// let (response, history, usage) = run_agentic_loop_unified(
///     &model, system_prompt, history, context, &ctx, config
/// ).await?;
/// ```
pub async fn run_agentic_loop_unified<M>(
    model: &M,
    system_prompt: &str,
    initial_history: Vec<Message>,
    sub_agent_context: SubAgentContext,
    ctx: &AgenticLoopContext<'_>,
    config: AgenticLoopConfig,
) -> Result<(String, Option<String>, Vec<Message>, Option<TokenUsage>)>
where
    M: rig::completion::CompletionModel + Sync,
{
    let supports_thinking = config.capabilities.supports_thinking_history;

    let agent_label = if config.is_sub_agent {
        format!("sub-agent (depth={})", sub_agent_context.depth)
    } else {
        "main-agent".to_string()
    };

    tracing::info!(
        "[{}] Starting agentic loop: provider={}, model={}, thinking={}, temperature={}",
        agent_label,
        ctx.provider_name,
        ctx.model_name,
        supports_thinking,
        config.capabilities.supports_temperature
    );

    // Create root span for the entire agent turn (this becomes the Langfuse trace)
    // All child spans (llm_completion, tool_call) will be nested under this
    // Extract user input from initial history for the trace input
    let trace_input: String = initial_history
        .iter()
        .rev()
        .find_map(|msg| {
            if let Message::User { content } = msg {
                Some(
                    content
                        .iter()
                        .filter_map(|c| {
                            if let rig::message::UserContent::Text(text) = c {
                                Some(text.text.clone())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
            } else {
                None
            }
        })
        .unwrap_or_default();
    let trace_input_truncated = if trace_input.len() > 2000 {
        format!("{}... [truncated]", truncate_str(&trace_input, 2000))
    } else {
        trace_input
    };

    // Create outer trace span (this becomes the Langfuse trace)
    let chat_message_span = tracing::info_span!(
        "chat_message",
        "langfuse.session.id" = ctx.session_id.unwrap_or(""),
        "langfuse.observation.input" = %trace_input_truncated,
        "langfuse.observation.output" = tracing::field::Empty,
    );

    // Create agent span as child of trace (this is the main agent observation)
    let agent_span = tracing::info_span!(
        parent: &chat_message_span,
        "agent",
        "langfuse.observation.type" = "agent",
        "langfuse.session.id" = ctx.session_id.unwrap_or(""),
        "langfuse.observation.input" = %trace_input_truncated,
        "langfuse.observation.output" = tracing::field::Empty,
        agent_type = %agent_label,
        model = %ctx.model_name,
        provider = %ctx.provider_name,
    );
    // Instrument the main loop body with both spans so they're properly exported to OpenTelemetry.
    // Using nested .instrument() ensures both spans are entered for the duration of the loop.
    let (accumulated_response, accumulated_thinking, chat_history, total_usage) = async {
        // Reset loop detector for new turn
        {
        let mut detector = ctx.loop_detector.write().await;
        detector.reset();
    }

    // Create persistent capture context for file event correlation
    let capture_ctx = LoopCaptureContext::new(ctx.sidecar_state);

    // Create hook registry for system hooks
    let hook_registry = HookRegistry::new();

    // Get all available tools (filtered by config + web search)
    let mut tools = get_all_tool_definitions_with_config(ctx.tool_config);

    // Add run_command (wrapper for run_pty_cmd with better naming)
    tools.push(get_run_command_tool_definition());

    // Add ask_human barrier tool (HITL: AI asks user for input)
    tools.push(get_ask_human_tool_definition());

    // Add any additional tools (e.g., SWE-bench test tool)
    tools.extend(ctx.additional_tool_definitions.iter().cloned());

    tracing::debug!(
        "Available tools (unified loop): {:?}",
        tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>()
    );

    // Add dynamically registered tools from the registry (Tavily, PTY interactive, pentest, etc.)
    // Dynamic tools are registered at runtime by configure_bridge and should always be included.
    // Tavily tools still respect tool_config since they can be disabled in settings.
    // Apply sanitize_schema for OpenAI strict mode compatibility.
    {
        let registry = ctx.tool_registry.read().await;
        let registry_tools = registry.get_tool_definitions();
        drop(registry);

        let existing_names: HashSet<String> = tools.iter().map(|t| t.name.clone()).collect();

        for tool in registry_tools {
            if existing_names.contains(&tool.name) {
                continue;
            }

            let always_include = tool.name.starts_with("pentest_");
            let tavily_enabled = tool.name.starts_with("tavily_")
                && ctx.tool_config.is_tool_enabled(&tool.name);

            if always_include || tavily_enabled {
                tools.push(rig::completion::ToolDefinition {
                    name: tool.name,
                    description: tool.description,
                    parameters: sanitize_schema(tool.parameters),
                });
            }
        }
    }

    // Only add sub-agent tools if we're not at max depth
    // Sub-agents are controlled by the registry, not the tool config
    if sub_agent_context.depth < MAX_AGENT_DEPTH - 1 {
        let registry = ctx.sub_agent_registry.read().await;
        tools.extend(get_sub_agent_tool_definitions(&registry).await);
    }

    let mut chat_history = initial_history;

    // Update context manager with current history
    ctx.context_manager
        .update_from_messages(&chat_history)
        .await;

    // Note: Context compaction is now handled by the summarizer agent
    // which is triggered via should_compact() in the agentic loop

    // Audit: record agent turn start
    if let Some(tracker) = ctx.db_tracker {
        tracker.audit(
            "agent_turn_start",
            "ai",
            &format!("model={} provider={}", ctx.model_name, ctx.provider_name),
        );
    }

    let mut accumulated_response = String::new();
    // Thinking history tracking - only used when supports_thinking is true
    let mut accumulated_thinking = String::new();
    let mut total_usage = TokenUsage::default();
    let mut iteration = 0;
    let mut consecutive_no_tool_turns: u32 = 0;
    let mut total_reflector_nudges: u32 = 0;
    // Tracks whether the memory gatekeeper decided this message warrants tool usage.
    // When false (simple chat), the reflector won't nudge the agent to use tools.
    let mut gatekeeper_wants_tools = false;

    loop {
        iteration += 1;

        // Reset compaction state for this turn (preserves last_input_tokens)
        {
            let mut compaction_state = ctx.compaction_state.write().await;
            compaction_state.reset_turn();
        }

        // Check for compaction at start of turn (using tokens from previous turn)
        // This is important when the agent completes in a single iteration
        if iteration == 1 {
            {
                let compaction_state = ctx.compaction_state.read().await;
                if compaction_state.last_input_tokens.is_some() {
                    tracing::info!(
                        "[compaction] Pre-turn check - tokens: {:?}, using_heuristic: {}",
                        compaction_state.last_input_tokens,
                        compaction_state.using_heuristic
                    );
                }
            }

            if let Some(session_id) = ctx.session_id {
                match maybe_compact(ctx, session_id, &mut chat_history).await {
                    Ok(Some(result)) => {
                        if result.success {
                            let _ = ctx.event_tx.send(AiEvent::CompactionCompleted {
                                tokens_before: result.tokens_before,
                                messages_before: result.messages_before,
                                messages_after: chat_history.len(),
                                summary_length: result.summary.as_ref().map(|s| s.len()).unwrap_or(0),
                                summary: result.summary.clone(),
                                summarizer_input: result.summarizer_input.clone(),
                            });
                            ctx.context_manager
                                .update_from_messages(&chat_history)
                                .await;
                        } else {
                            let _ = ctx.event_tx.send(AiEvent::CompactionFailed {
                                tokens_before: result.tokens_before,
                                messages_before: result.messages_before,
                                error: result.error.clone().unwrap_or_else(|| "Unknown error".to_string()),
                                summarizer_input: result.summarizer_input.clone(),
                            });
                        }
                    }
                    Ok(None) => {} // No compaction needed
                    Err(e) => {
                        tracing::error!("[compaction] Pre-turn compaction error: {}", e);
                    }
                }
            }
        }

        if let Some(flag) = &ctx.cancelled {
            if flag.load(std::sync::atomic::Ordering::SeqCst) {
                tracing::info!("Agent loop cancelled by user (iteration {})", iteration);
                let _ = ctx.event_tx.send(AiEvent::Error {
                    message: "Agent stopped by user".to_string(),
                    error_type: "cancelled".to_string(),
                });
                break;
            }
        }

        if iteration > MAX_TOOL_ITERATIONS {
            // Record max iterations event in Langfuse
            let _max_iter_event = tracing::info_span!(
                parent: &agent_span,
                "max_iterations_reached",
                "langfuse.observation.type" = "event",
                "langfuse.session.id" = ctx.session_id.unwrap_or(""),
                max_iterations = MAX_TOOL_ITERATIONS,
            );

            let _ = ctx.event_tx.send(AiEvent::Error {
                message: "Maximum tool iterations reached".to_string(),
                error_type: "max_iterations".to_string(),
            });
            break;
        }

        // Check for context compaction need (between turns, after iteration 1)
        if iteration > 1 {
            // Log compaction state at start of each iteration
            {
                let compaction_state = ctx.compaction_state.read().await;
                tracing::info!(
                    "[compaction] Iteration {} - tokens: {:?}, using_heuristic: {}, attempted: {}",
                    iteration,
                    compaction_state.last_input_tokens,
                    compaction_state.using_heuristic,
                    compaction_state.attempted_this_turn
                );
            }

            if let Some(session_id) = ctx.session_id {
                // Check if compaction is needed and perform it if so
                match maybe_compact(ctx, session_id, &mut chat_history).await {
                    Ok(Some(result)) => {
                        if result.success {
                            // Emit success event
                            let _ = ctx.event_tx.send(AiEvent::CompactionCompleted {
                                tokens_before: result.tokens_before,
                                messages_before: result.messages_before,
                                messages_after: chat_history.len(),
                                summary_length: result.summary.as_ref().map(|s| s.len()).unwrap_or(0),
                                summary: result.summary.clone(),
                                summarizer_input: result.summarizer_input.clone(),
                            });

                            // Update context manager with new (compacted) history
                            ctx.context_manager
                                .update_from_messages(&chat_history)
                                .await;
                        } else {
                            // Emit failure event
                            let _ = ctx.event_tx.send(AiEvent::CompactionFailed {
                                tokens_before: result.tokens_before,
                                messages_before: result.messages_before,
                                error: result.error.clone().unwrap_or_else(|| "Unknown error".to_string()),
                                summarizer_input: result.summarizer_input.clone(),
                            });

                            // Check if we're still over the limit after failed compaction
                            let compaction_state = ctx.compaction_state.read().await;
                            let check = ctx
                                .context_manager
                                .should_compact(&compaction_state, ctx.model_name);
                            drop(compaction_state);

                            if check.should_compact {
                                // We needed compaction but it failed, and we're still over limit
                                tracing::error!(
                                    "[compaction] Failed and context still exceeded: {} tokens",
                                    check.current_tokens
                                );
                                let _ = ctx.event_tx.send(AiEvent::Error {
                                    message: format!(
                                        "Context compaction failed and limit exceeded ({} tokens). {}",
                                        check.current_tokens,
                                        result.error.unwrap_or_else(|| "Unknown error".to_string())
                                    ),
                                    error_type: "compaction_failed".to_string(),
                                });
                                return Err(TerminalErrorEmitted::with_partial_state(
                                    "Context compaction failed and limit exceeded",
                                    (!accumulated_response.is_empty())
                                        .then(|| accumulated_response.clone()),
                                    Some(chat_history.clone()),
                                )
                                .into());
                            }
                        }
                    }
                    Ok(None) => {
                        // No compaction needed, continue normally
                    }
                    Err(e) => {
                        // Error checking compaction (non-fatal, log and continue)
                        tracing::warn!("[compaction] Error during compaction check: {}", e);
                    }
                }
            }
        }

        // Fire message hooks and memory gatekeeper on first iteration (before first LLM call).
        if iteration == 1 && !config.is_sub_agent {
            let last_user_text = chat_history.iter().rev().find_map(|msg| {
                if let Message::User { content } = msg {
                    content.iter().find_map(|c| {
                        if let UserContent::Text(t) = c {
                            Some(t.text.as_str())
                        } else {
                            None
                        }
                    })
                } else {
                    None
                }
            });

            if let Some(user_text) = last_user_text {
                // Run synchronous message hooks
                let msg_ctx = MessageHookContext::user_input(
                    user_text,
                    ctx.session_id.unwrap_or(""),
                );
                let mut hook_messages = hook_registry.run_message_hooks(&msg_ctx);

                // Run async memory gatekeeper: small model classifies whether
                // memory search is warranted for this message.
                {
                    let client = ctx.client.read().await;
                    let wants_memory = crate::memory_gatekeeper::should_search_memory(&client, user_text).await;
                    gatekeeper_wants_tools = wants_memory;
                    if wants_memory {
                        hook_messages.push(
                            "[Memory-First] The gatekeeper determined this message may benefit \
                             from prior context. Call `search_memories` with relevant keywords \
                             before responding."
                                .to_string(),
                        );
                    }
                }

                if !hook_messages.is_empty() {
                    let formatted = format_system_hooks(&hook_messages);
                    tracing::info!(
                        count = hook_messages.len(),
                        "Injecting message hooks before first LLM call"
                    );

                    let _ = ctx.event_tx.send(AiEvent::SystemHooksInjected {
                        hooks: hook_messages,
                    });

                    chat_history.push(Message::User {
                        content: OneOrMany::one(UserContent::Text(Text {
                            text: formatted,
                        })),
                    });
                }
            }
        }

        // Create span for Langfuse observability (child of agent_span)
        // Token usage fields are Empty and will be recorded when available
        // Note: Langfuse expects prompt_tokens/completion_tokens per GenAI semantic conventions
        // Using both gen_ai.* and langfuse.observation.* for maximum compatibility
        let llm_span = tracing::info_span!(
            parent: &agent_span,
            "llm_completion",
            "gen_ai.operation.name" = "chat_completion",
            "gen_ai.request.model" = %ctx.model_name,
            "gen_ai.system" = %ctx.provider_name,
            "gen_ai.request.temperature" = 0.3_f64,
            "gen_ai.request.max_tokens" = MAX_COMPLETION_TOKENS as i64,
            "langfuse.observation.type" = "generation",
            "langfuse.session.id" = ctx.session_id.unwrap_or(""),
            iteration = iteration,
            "gen_ai.usage.prompt_tokens" = tracing::field::Empty,
            "gen_ai.usage.completion_tokens" = tracing::field::Empty,
            // Use both gen_ai.* and langfuse.observation.* for input/output mapping
            "gen_ai.reasoning" = tracing::field::Empty,
            "gen_ai.prompt" = tracing::field::Empty,
            "gen_ai.completion" = tracing::field::Empty,
            "langfuse.observation.input" = tracing::field::Empty,
            "langfuse.observation.output" = tracing::field::Empty,
        );
        // Note: We use explicit parent instead of span.enter() for async compatibility

        // Extract user text for Langfuse prompt tracking
        // Only record actual user text - tool results are already in previous tool spans
        let last_user_text: String = chat_history
            .iter()
            .rev()
            .find_map(|msg| {
                if let Message::User { content } = msg {
                    let text_parts: Vec<String> = content
                        .iter()
                        .filter_map(|c| {
                            if let rig::message::UserContent::Text(text) = c {
                                if !text.text.is_empty() {
                                    Some(text.text.clone())
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        })
                        .collect();
                    if !text_parts.is_empty() {
                        Some(text_parts.join("\n"))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .unwrap_or_default();

        // Only record input if there's actual user text (not just tool results)
        if !last_user_text.is_empty() {
            let prompt_for_span = if last_user_text.len() > 2000 {
                format!("{}... [truncated]", truncate_str(&last_user_text, 2000))
            } else {
                last_user_text
            };
            llm_span.record("gen_ai.prompt", prompt_for_span.as_str());
            llm_span.record("langfuse.observation.input", prompt_for_span.as_str());
        }
        // When continuing after tool results: don't record input, context is in previous spans

        // Build request - conditionally set temperature based on model support
        let temperature = if config.capabilities.supports_temperature {
            Some(0.3)
        } else {
            tracing::debug!(
                "Model {} does not support temperature parameter, omitting",
                ctx.model_name
            );
            None
        };

        // Build additional_params for provider-specific features
        let mut additional_params_json = serde_json::Map::new();

        // Add web search if enabled (OpenAI)
        if let Some(web_config) = ctx.openai_web_search_config {
            tracing::info!(
                "Adding OpenAI web_search_preview tool with context_size={}",
                web_config.search_context_size
            );
            additional_params_json.insert(
                "tools".to_string(),
                json!([web_config.to_tool_json()]),
            );
        }

        // Add reasoning config if set (for OpenAI o-series and GPT-5 Codex models)
        // OpenAI Responses API expects a nested "reasoning" object with:
        // - effort: how much thinking the model should do
        // - summary: enables streaming reasoning text to the client ("detailed" shows full reasoning)
        if let Some(effort) = ctx.openai_reasoning_effort {
            tracing::info!("Setting OpenAI reasoning.effort={}, reasoning.summary=detailed", effort);
            additional_params_json.insert(
                "reasoning".to_string(),
                json!({
                    "effort": effort,
                    "summary": "detailed"
                }),
            );
        }

        // Add OpenRouter provider preferences if set
        if let Some(prefs) = ctx.openrouter_provider_preferences {
            if let serde_json::Value::Object(prefs_map) = prefs {
                for (key, value) in prefs_map {
                    tracing::info!("Adding OpenRouter provider preference: {}={}", key, value);
                    additional_params_json.insert(key.clone(), value.clone());
                }
            }
        }

        let additional_params = if additional_params_json.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(additional_params_json))
        };

        // Diagnostic logging — only traverse history when log level permits
        if tracing::enabled!(tracing::Level::DEBUG) {
            let image_count: usize = chat_history
                .iter()
                .map(|msg| {
                    if let Message::User { content } = msg {
                        content
                            .iter()
                            .filter(|c| matches!(c, rig::message::UserContent::Image(_)))
                            .count()
                    } else {
                        0
                    }
                })
                .sum();
            if image_count > 0 {
                tracing::debug!(
                    "[Unified] Chat history contains {} image(s) across {} messages",
                    image_count,
                    chat_history.len()
                );
            }

            let has_reasoning_in_history = chat_history.iter().any(|m| {
                if let Message::Assistant { content, .. } = m {
                    content
                        .iter()
                        .any(|c| matches!(c, AssistantContent::Reasoning(_)))
                } else {
                    false
                }
            });
            tracing::debug!(
                "[OpenAI Debug] Starting stream: iteration={}, history_len={}, provider={}, has_reasoning_history={}, thinking={}",
                iteration,
                chat_history.len(),
                ctx.provider_name,
                has_reasoning_in_history,
                supports_thinking
            );
        }

        // Wrap stream request in timeout to prevent infinite hangs (3 minutes)
        let stream_timeout = std::time::Duration::from_secs(180);

        // Proactive token count: estimate tokens BEFORE sending to detect compaction need early.
        // This is a leading indicator vs the lagging provider-reported count after the response.
        {
            let system_prompt_tokens = tokenx_rs::estimate_token_count(system_prompt);
            let history_tokens: usize = chat_history.iter().map(estimate_message_tokens).sum();
            let estimated_input_tokens = (system_prompt_tokens + history_tokens) as u64;

            let mut compaction_state = ctx.compaction_state.write().await;
            compaction_state.update_tokens_estimated(estimated_input_tokens);
            tracing::debug!(
                "[compaction] Pre-call estimate: ~{} tokens (system={}, history={})",
                estimated_input_tokens,
                system_prompt_tokens,
                history_tokens,
            );
        }

        let mut stream_start_failure: Option<(String, StreamStartErrorClassification)> = None;
        let mut started_stream = None;

        // NVIDIA NIM workaround: rig-core's OpenAI provider serializes system
        // message content as [{"type":"text","text":"..."}] (array), but NVIDIA
        // NIM only accepts plain strings. Move the system prompt into a user
        // message so it's serialized correctly via rig-core's user content
        // flattener.
        let is_nvidia_provider = ctx.provider_name == "nvidia";

        // Build request components once before the retry loop to avoid
        // re-cloning chat_history, tools, and additional_params on each attempt.
        let (preamble, request_history) = if is_nvidia_provider {
            let mut nvidia_history = vec![Message::User {
                content: OneOrMany::one(UserContent::text(system_prompt)),
            }];
            nvidia_history.extend(chat_history.clone());
            (None, nvidia_history)
        } else {
            (Some(system_prompt.to_string()), chat_history.clone())
        };
        let request_chat_history = OneOrMany::many(request_history.clone())
            .unwrap_or_else(|_| OneOrMany::one(request_history[0].clone()));
        let request_tools = tools.clone();

        for attempt in 1..=STREAM_START_MAX_ATTEMPTS {
            let request = rig::completion::CompletionRequest {
                preamble: preamble.clone(),
                chat_history: request_chat_history.clone(),
                documents: vec![],
                tools: request_tools.clone(),
                temperature,
                max_tokens: Some(MAX_COMPLETION_TOKENS as u64),
                tool_choice: None,
                additional_params: additional_params.clone(),
                model: None,
                output_schema: None,
            };

            // Record outgoing request at the stream boundary (main agent)
            ctx.api_request_stats.record_sent(ctx.provider_name).await;

            let stream_result = tokio::time::timeout(
                stream_timeout,
                async { model.stream(request).await }.instrument(llm_span.clone()),
            )
            .await;

            match stream_result {
                Ok(Ok(s)) => {
                    ctx.api_request_stats.record_received(ctx.provider_name).await;
                    tracing::info!(
                        "[OpenAI Debug] Stream created successfully on attempt {}",
                        attempt
                    );
                    started_stream = Some(s);
                    break;
                }
                Ok(Err(e)) => {
                    let error_str = e.to_string();
                    let classification = classify_stream_start_error(&error_str);
                    tracing::warn!(
                        "Stream start failed (attempt {}/{}): {}",
                        attempt,
                        STREAM_START_MAX_ATTEMPTS,
                        error_str
                    );

                    if should_retry_stream_start(attempt, &classification) {
                        let delay = compute_retry_backoff_delay(attempt);
                        let delay_ms = delay.as_millis();
                        let _ = ctx.event_tx.send(AiEvent::Warning {
                            message: format!(
                                "AI request failed ({}). Retrying in {}ms (attempt {}/{})",
                                classification.error_type,
                                delay_ms,
                                attempt + 1,
                                STREAM_START_MAX_ATTEMPTS
                            ),
                        });
                        sleep_for_retry_delay(delay).await;
                        continue;
                    }

                    stream_start_failure = Some((error_str, classification));
                    break;
                }
                Err(_elapsed) => {
                    let timeout_secs = stream_timeout.as_secs();
                    let error_str = format!("Stream request timeout after {}s", timeout_secs);
                    let classification = stream_start_timeout_classification(timeout_secs);
                    tracing::warn!(
                        "[OpenAI Debug] Stream request timed out (attempt {}/{}): {}",
                        attempt,
                        STREAM_START_MAX_ATTEMPTS,
                        error_str
                    );

                    if should_retry_stream_start(attempt, &classification) {
                        let delay = compute_retry_backoff_delay(attempt);
                        let delay_ms = delay.as_millis();
                        let _ = ctx.event_tx.send(AiEvent::Warning {
                            message: format!(
                                "AI request timed out. Retrying in {}ms (attempt {}/{})",
                                delay_ms,
                                attempt + 1,
                                STREAM_START_MAX_ATTEMPTS
                            ),
                        });
                        sleep_for_retry_delay(delay).await;
                        continue;
                    }

                    stream_start_failure = Some((error_str, classification));
                    break;
                }
            }
        }

        let mut stream = if let Some(stream) = started_stream {
            stream
        } else {
            let (error_str, classification) = stream_start_failure.unwrap_or_else(|| {
                (
                    "Failed to start streaming response".to_string(),
                    StreamStartErrorClassification {
                        error_type: "api_error",
                        user_message: "Failed to start streaming response".to_string(),
                        retriable: false,
                    },
                )
            });

            let _ = ctx.event_tx.send(AiEvent::Error {
                message: classification.user_message,
                error_type: classification.error_type.to_string(),
            });

            return Err(TerminalErrorEmitted::with_partial_state(
                error_str,
                (!accumulated_response.is_empty()).then(|| accumulated_response.clone()),
                Some(chat_history.clone()),
            )
            .into());
        };

        tracing::debug!("[Unified] Stream started - listening for content");

        // Process streaming response
        let mut has_tool_calls = false;
        let mut tool_calls_to_execute: Vec<ToolCall> = vec![];
        let mut text_content = String::new();
        // Per-iteration thinking tracking (for history building)
        let mut thinking_content = String::new();
        let mut thinking_signature: Option<String> = None;
        // Reasoning ID for OpenAI Responses API (rs_... IDs that function calls reference)
        let mut thinking_id: Option<String> = None;
        let mut chunk_count = 0;
        let mut last_stream_chunk_error: Option<String> = None;
        let mut last_repetition_check_len: usize = 0;

        // Track tool call state for streaming
        let mut current_tool_id: Option<String> = None;
        // Separate call_id (OpenAI's call_id, e.g. "call_abc") from item id (e.g. "fc_abc").
        // These differ in the OpenAI Responses API and must be tracked independently.
        let mut current_tool_call_id: Option<String> = None;
        let mut current_tool_name: Option<String> = None;
        let mut current_tool_args = String::new();

        while let Some(chunk_result) = stream.next().await {
            chunk_count += 1;
            // Log progress every 50 chunks to avoid spam but track stream activity
            if chunk_count % 50 == 0 {
                tracing::debug!(
                    "[OpenAI Debug] Stream progress: {} chunks processed",
                    chunk_count
                );
            }
            match chunk_result {
                Ok(chunk) => {
                    match chunk {
                        StreamedAssistantContent::Text(text_msg) => {
                            // Check if this is thinking content (prefixed by our streaming impl)
                            // This handles the case where thinking is sent as a [Thinking] prefixed message
                            if let Some(thinking) = text_msg.text.strip_prefix("[Thinking] ") {
                                if supports_thinking {
                                    tracing::trace!(
                                        "[Unified] Received [Thinking]-prefixed text chunk #{}: {} chars",
                                        chunk_count,
                                        thinking.len()
                                    );
                                    thinking_content.push_str(thinking);
                                    accumulated_thinking.push_str(thinking);
                                }
                                // Always emit reasoning event (to frontend and sidecar)
                                emit_event(
                                    ctx,
                                    AiEvent::Reasoning {
                                        content: thinking.to_string(),
                                    },
                                );
                            } else {
                                // Check for server tool result markers
                                if let Some(rest) =
                                    text_msg.text.strip_prefix("[WEB_SEARCH_RESULT:")
                                {
                                    // Parse: [WEB_SEARCH_RESULT:tool_use_id:json_results]
                                    if let Some(colon_pos) = rest.find(':') {
                                        let tool_use_id = &rest[..colon_pos];
                                        let json_rest = rest[colon_pos + 1..].trim_end_matches(']');
                                        if let Ok(results) =
                                            serde_json::from_str::<serde_json::Value>(json_rest)
                                        {
                                            tracing::info!(
                                                "Parsed web search results for {}",
                                                tool_use_id
                                            );
                                            emit_event(
                                                ctx,
                                                AiEvent::WebSearchResult {
                                                    request_id: tool_use_id.to_string(),
                                                    results,
                                                },
                                            );
                                        }
                                    }
                                } else if let Some(rest) =
                                    text_msg.text.strip_prefix("[WEB_FETCH_RESULT:")
                                {
                                    // Parse: [WEB_FETCH_RESULT:tool_use_id:url:json_content]
                                    let parts: Vec<&str> = rest.splitn(3, ':').collect();
                                    if parts.len() >= 3 {
                                        let tool_use_id = parts[0];
                                        let url = parts[1];
                                        let json_rest = parts[2].trim_end_matches(']');
                                        let content_preview = if json_rest.len() > 200 {
                                            format!("{}...", truncate_str(json_rest, 200))
                                        } else {
                                            json_rest.to_string()
                                        };
                                        tracing::info!(
                                            "Parsed web fetch result for {}: {}",
                                            tool_use_id,
                                            url
                                        );
                                        emit_event(
                                            ctx,
                                            AiEvent::WebFetchResult {
                                                request_id: tool_use_id.to_string(),
                                                url: url.to_string(),
                                                content_preview,
                                            },
                                        );
                                    }
                                } else {
                                    // Regular text content
                                    text_content.push_str(&text_msg.text);
                                    accumulated_response.push_str(&text_msg.text);
                                    let _ = ctx.event_tx.send(AiEvent::TextDelta {
                                        delta: text_msg.text,
                                        accumulated: accumulated_response.clone(),
                                    });

                                    // Detect degenerate repetitive generation
                                    if text_content.len() > last_repetition_check_len + 200 {
                                        last_repetition_check_len = text_content.len();
                                        if detect_repetitive_text(&text_content) {
                                            tracing::warn!(
                                                text_len = text_content.len(),
                                                "Repetitive text detected, stopping generation"
                                            );
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        StreamedAssistantContent::Reasoning(reasoning) => {
                            // Native reasoning/thinking content from extended thinking models
                            let reasoning_text = reasoning
                                .content
                                .iter()
                                .filter_map(|c| {
                                    if let ReasoningContent::Text { text, .. } = c {
                                        Some(text.as_str())
                                    } else {
                                        None
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join("");
                            let chunk_signature = reasoning.content.iter().find_map(|c| {
                                if let ReasoningContent::Text { signature, .. } = c {
                                    signature.clone()
                                } else {
                                    None
                                }
                            });
                            if supports_thinking {
                                tracing::trace!(
                                    "[Unified] Received native reasoning chunk #{}: {} chars, has_signature: {}",
                                    chunk_count,
                                    reasoning_text.len(),
                                    chunk_signature.is_some()
                                );
                                thinking_content.push_str(&reasoning_text);
                                accumulated_thinking.push_str(&reasoning_text);
                                // Capture the signature (needed for Anthropic API when sending back history)
                                if chunk_signature.is_some() {
                                    thinking_signature = chunk_signature;
                                }
                                // Capture the ID (needed for OpenAI Responses API - rs_... IDs that function calls reference)
                                if reasoning.id.is_some() {
                                    thinking_id = reasoning.id.clone();
                                }
                            }
                            // Always emit reasoning event (to frontend and sidecar)
                            emit_event(
                                ctx,
                                AiEvent::Reasoning {
                                    content: reasoning_text,
                                },
                            );
                        }
                        StreamedAssistantContent::ReasoningDelta { id, reasoning } => {
                            // Streaming reasoning delta (similar to Reasoning but delivered as deltas)
                            if supports_thinking {
                                tracing::trace!(
                                    "[Unified] Received reasoning delta chunk #{}: {} chars",
                                    chunk_count,
                                    reasoning.len()
                                );
                                thinking_content.push_str(&reasoning);
                                accumulated_thinking.push_str(&reasoning);
                                // Capture the ID if present (for OpenAI Responses API)
                                if id.is_some() && thinking_id.is_none() {
                                    thinking_id = id;
                                }
                            }
                            // Always emit reasoning event (to frontend and sidecar)
                            emit_event(ctx, AiEvent::Reasoning { content: reasoning });
                        }
                        StreamedAssistantContent::ToolCall { tool_call, .. } => {
                            // Check if this is a server tool (executed by provider, not us)
                            let is_server_tool = tool_call
                                .call_id
                                .as_ref()
                                .map(|id: &String| id.starts_with("server:"))
                                .unwrap_or(false);

                            if is_server_tool {
                                // Server tool (web_search/web_fetch) - already executed by provider
                                tracing::info!(
                                    "Server tool detected: {} ({})",
                                    tool_call.function.name,
                                    tool_call.id
                                );
                                emit_event(
                                    ctx,
                                    AiEvent::ServerToolStarted {
                                        request_id: tool_call.id.clone(),
                                        tool_name: tool_call.function.name.clone(),
                                        input: tool_call.function.arguments.clone(),
                                    },
                                );
                                // Don't add to tool_calls_to_execute - provider handles execution
                                continue;
                            }

                            has_tool_calls = true;

                            // Finalize any previous pending tool call first
                            if let (Some(prev_id), Some(prev_name)) =
                                (current_tool_id.take(), current_tool_name.take())
                            {
                                let args = golish_json_repair::parse_tool_args(&current_tool_args);
                                let prev_call_id = current_tool_call_id.take().unwrap_or_else(|| prev_id.clone());
                                tool_calls_to_execute.push(ToolCall {
                                    id: prev_id,
                                    call_id: Some(prev_call_id),
                                    function: rig::message::ToolFunction {
                                        name: prev_name,
                                        arguments: args,
                                    },
                                    signature: None,
                                    additional_params: None,
                                });
                                current_tool_args.clear();
                            }

                            // Check if this tool call has complete args (non-streaming case)
                            // If args are empty object {}, we'll wait for deltas
                            let has_complete_args = !tool_call.function.arguments.is_null()
                                && tool_call.function.arguments != serde_json::json!({});

                            if has_complete_args {
                                // Tool call came complete, add directly
                                // Ensure call_id is set for OpenAI compatibility
                                let mut tool_call = tool_call;
                                if tool_call.call_id.is_none() {
                                    tool_call.call_id = Some(tool_call.id.clone());
                                }
                                tool_calls_to_execute.push(tool_call);
                            } else {
                                // Tool call has empty args, wait for deltas
                                current_tool_id = Some(tool_call.id.clone());
                                // Preserve the OpenAI call_id (e.g. "call_abc") separately from
                                // the item id (e.g. "fc_abc") — these differ in the Responses API
                                // and the call_id must match when sending function_call_output back.
                                current_tool_call_id = tool_call.call_id.clone();
                                current_tool_name = Some(tool_call.function.name.clone());
                                // Start with any existing args (might be empty object serialized)
                                if !tool_call.function.arguments.is_null()
                                    && tool_call.function.arguments != serde_json::json!({})
                                {
                                    current_tool_args = tool_call.function.arguments.to_string();
                                }
                            }
                        }
                        StreamedAssistantContent::ToolCallDelta { id, content, .. } => {
                            // If we don't have a current tool ID but the delta has one, use it
                            if current_tool_id.is_none() && !id.is_empty() {
                                current_tool_id = Some(id);
                            }
                            // Accumulate tool call argument deltas (extract string from enum)
                            if let rig::streaming::ToolCallDeltaContent::Delta(delta) = content {
                                current_tool_args.push_str(&delta);
                            }
                        }
                        StreamedAssistantContent::Final(ref resp) => {
                            // Extract and accumulate token usage
                            if let Some(usage) = resp.token_usage() {
                                total_usage.input_tokens += usage.input_tokens;
                                total_usage.output_tokens += usage.output_tokens;
                                // Record token usage as span attributes for Langfuse
                                // Using prompt_tokens/completion_tokens per GenAI semantic conventions
                                llm_span.record(
                                    "gen_ai.usage.prompt_tokens",
                                    usage.input_tokens as i64,
                                );
                                llm_span.record(
                                    "gen_ai.usage.completion_tokens",
                                    usage.output_tokens as i64,
                                );
                                tracing::info!(
                                    "[compaction] Token usage iter {}: input={}, output={}, cumulative={}",
                                    iteration,
                                    usage.input_tokens,
                                    usage.output_tokens,
                                    total_usage.total()
                                );

                                // Update compaction state with provider token count
                                {
                                    let mut compaction_state = ctx.compaction_state.write().await;
                                    compaction_state.update_tokens(usage.input_tokens);
                                    tracing::info!(
                                        "[compaction] State updated: {} input tokens from provider",
                                        usage.input_tokens
                                    );
                                }

                                // Emit context utilization event for frontend
                                let model_config = golish_context::TokenBudgetConfig::for_model(ctx.model_name);
                                let max_tokens = model_config.max_context_tokens;
                                let utilization = usage.input_tokens as f64 / max_tokens as f64;
                                let _ = ctx.event_tx.send(AiEvent::ContextWarning {
                                    utilization,
                                    total_tokens: usage.input_tokens as usize,
                                    max_tokens,
                                });
                            } else {
                                // Fallback: estimate tokens from message content using tokenx-rs
                                let estimated_tokens: usize = chat_history
                                    .iter()
                                    .map(estimate_message_tokens)
                                    .sum();

                                // Update total_usage with estimate so it's reported to frontend
                                // We split roughly 80/20 input/output as a reasonable approximation
                                let estimated_input = (estimated_tokens as f64 * 0.8) as u64;
                                let estimated_output = (estimated_tokens as f64 * 0.2) as u64;
                                total_usage.input_tokens += estimated_input;
                                total_usage.output_tokens += estimated_output;

                                {
                                    let mut compaction_state = ctx.compaction_state.write().await;
                                    compaction_state.update_tokens_estimated(estimated_tokens as u64);
                                    tracing::info!(
                                        "[compaction] State updated (tokenx-rs estimate): ~{} estimated tokens",
                                        estimated_tokens,
                                    );
                                }

                                // Emit context utilization event for frontend (heuristic)
                                let model_config = golish_context::TokenBudgetConfig::for_model(ctx.model_name);
                                let max_tokens = model_config.max_context_tokens;
                                let utilization = estimated_tokens as f64 / max_tokens as f64;
                                let _ = ctx.event_tx.send(AiEvent::ContextWarning {
                                    utilization,
                                    total_tokens: estimated_tokens,
                                    max_tokens,
                                });
                            }

                            // Extract reasoning encrypted_content from OpenAI Responses API
                            // The Final response may contain reasoning_encrypted_content which is
                            // required for stateless multi-turn conversations with reasoning models.
                            // We serialize to JSON and check for the OpenAI-specific field.
                            if let Ok(json_value) = serde_json::to_value(resp) {
                                // Log what we're seeing in the Final response
                                let has_encrypted_field = json_value.get("reasoning_encrypted_content").is_some();
                                tracing::info!(
                                    "[OpenAI Debug] Final response: has_reasoning_encrypted_content={}, thinking_id={:?}, thinking_signature_before={:?}",
                                    has_encrypted_field,
                                    thinking_id,
                                    thinking_signature.as_ref().map(|s| s.len())
                                );

                                if let Some(encrypted_map) = json_value
                                    .get("reasoning_encrypted_content")
                                    .and_then(|v| v.as_object())
                                {
                                    tracing::info!(
                                        "[OpenAI Debug] encrypted_map has {} entries: {:?}",
                                        encrypted_map.len(),
                                        encrypted_map.keys().collect::<Vec<_>>()
                                    );

                                    // If we have accumulated thinking and captured a thinking_id,
                                    // look up the encrypted_content for that reasoning item
                                    if let Some(ref tid) = thinking_id {
                                        if let Some(encrypted) = encrypted_map.get(tid).and_then(|v| v.as_str()) {
                                            tracing::info!(
                                                "[OpenAI Debug] Found encrypted_content for reasoning item {}: {} bytes",
                                                tid,
                                                encrypted.len()
                                            );
                                            thinking_signature = Some(encrypted.to_string());
                                        } else {
                                            tracing::warn!(
                                                "[OpenAI Debug] thinking_id {} NOT FOUND in encrypted_map!",
                                                tid
                                            );
                                        }
                                    }
                                    // If we don't have a thinking_id but have exactly one reasoning item,
                                    // use that one (common case: single reasoning block in response)
                                    if thinking_signature.is_none() && encrypted_map.len() == 1 {
                                        if let Some((id, encrypted)) = encrypted_map.iter().next() {
                                            if let Some(encrypted_str) = encrypted.as_str() {
                                                tracing::info!(
                                                    "[OpenAI Debug] Using single encrypted_content for reasoning item {}: {} bytes",
                                                    id,
                                                    encrypted_str.len()
                                                );
                                                thinking_signature = Some(encrypted_str.to_string());
                                                // Also set thinking_id if not set
                                                if thinking_id.is_none() {
                                                    thinking_id = Some(id.clone());
                                                }
                                            }
                                        }
                                    }
                                }
                            } else {
                                tracing::warn!("[OpenAI Debug] Failed to serialize Final response to JSON");
                            }

                            // Finalize any pending tool call from deltas
                            if let (Some(id), Some(name)) =
                                (current_tool_id.take(), current_tool_name.take())
                            {
                                let args = golish_json_repair::parse_tool_args(&current_tool_args);
                                let call_id = current_tool_call_id.take().unwrap_or_else(|| id.clone());
                                tool_calls_to_execute.push(ToolCall {
                                    id,
                                    call_id: Some(call_id),
                                    function: rig::message::ToolFunction {
                                        name,
                                        arguments: args,
                                    },
                                    signature: None,
                                    additional_params: None,
                                });
                                current_tool_args.clear();
                            }
                        }
                    }
                }
                Err(e) => {
                    last_stream_chunk_error = Some(e.to_string());
                    tracing::warn!("Stream chunk error at #{}: {}", chunk_count, e);
                }
            }
        }

        // If the stream produced no usable content but had errors, surface the error
        if text_content.is_empty()
            && thinking_content.is_empty()
            && tool_calls_to_execute.is_empty()
            && current_tool_name.is_none()
        {
            if let Some(ref err_msg) = last_stream_chunk_error {
                let classification = classify_stream_start_error(err_msg);
                let _ = ctx.event_tx.send(AiEvent::Error {
                    message: classification.user_message.clone(),
                    error_type: classification.error_type.to_string(),
                });
                tracing::error!(
                    "Stream produced no content; last chunk error: {}",
                    err_msg
                );
                break;
            }
        }

        tracing::info!(
            "[OpenAI Debug] Stream completed: iteration={}, chunks={}, text_chars={}, thinking_chars={}, tool_calls={}",
            iteration,
            chunk_count,
            text_content.len(),
            thinking_content.len(),
            tool_calls_to_execute.len()
        );
        tracing::debug!(
            "Stream completed (unified): {} chunks, {} chars text, {} chars thinking, {} tool calls",
            chunk_count,
            text_content.len(),
            thinking_content.len(),
            tool_calls_to_execute.len()
        );

        // Record the completion for Langfuse (truncated to avoid huge spans)
        // Only record text content - tool call details are in child tool spans
        let completion_for_span = if !text_content.is_empty() {
            // Model produced text: record it (truncated)
            let mut end = text_content.len().min(2000);
            while end > 0 && !text_content.is_char_boundary(end) {
                end -= 1;
            }
            if text_content.len() > 2000 {
                format!("{}... [truncated]", &text_content[..end])
            } else {
                text_content.clone()
            }
        } else if !tool_calls_to_execute.is_empty() {
            // Model produced only tool calls (common for GPT-5.2/Codex): record tool names
            // so the span is not empty and traces show what the model decided to do.
            let names: Vec<&str> = tool_calls_to_execute
                .iter()
                .map(|tc| tc.function.name.as_str())
                .collect();
            format!("[tool_calls: {}]", names.join(", "))
        } else {
            String::new()
        };
        if !completion_for_span.is_empty() {
            llm_span.record("gen_ai.completion", completion_for_span.as_str());
            llm_span.record("langfuse.observation.output", completion_for_span.as_str());
        }

        // Record reasoning/thinking content on the span if present.
        // This is the model's internal reasoning displayed in the UI ThinkingBlock —
        // it must also appear in traces so Langfuse shows what the model was thinking.
        if !thinking_content.is_empty() {
            let mut end = thinking_content.len().min(2000);
            while end > 0 && !thinking_content.is_char_boundary(end) {
                end -= 1;
            }
            let reasoning_for_span = if thinking_content.len() > 2000 {
                format!("{}... [truncated]", &thinking_content[..end])
            } else {
                thinking_content.clone()
            };
            llm_span.record("gen_ai.reasoning", reasoning_for_span.as_str());
        }

        // Finalize any remaining tool call that wasn't closed by FinalResponse
        if let (Some(id), Some(name)) = (current_tool_id.take(), current_tool_name.take()) {
            let args = golish_json_repair::parse_tool_args(&current_tool_args);
            let call_id = current_tool_call_id.take().unwrap_or_else(|| id.clone());
            tool_calls_to_execute.push(ToolCall {
                id,
                call_id: Some(call_id),
                function: rig::message::ToolFunction {
                    name,
                    arguments: args,
                },
                signature: None,
                additional_params: None,
            });
            has_tool_calls = true;
        }

        // Log thinking content if present (for debugging)
        if supports_thinking && !thinking_content.is_empty() {
            tracing::debug!("Model thinking: {} chars", thinking_content.len());
        }

        // Build assistant content for history
        // IMPORTANT: When thinking is enabled, thinking blocks MUST come first (required by Anthropic API)
        let mut assistant_content: Vec<AssistantContent> = vec![];

        // Conditionally add thinking content first (required by Anthropic API when thinking is enabled)
        // OpenAI Responses API reasoning handling differs between providers:
        //
        // "openai_reasoning" (rig-openai-responses, gpt-5.2, Codex, o-series):
        //   - Always include reasoning in history when present. OpenAI tracks rs_... IDs
        //     server-side and requires them to be echoed back in every subsequent turn.
        //   - A reasoning item MUST be followed by the next output item (text OR tool call).
        //   - Omitting a reasoning item from a turn where it was generated causes:
        //     "Item 'rs_...' of type 'reasoning' was provided without its required following item"
        //
        // "openai_responses" (rig-core built-in, non-reasoning models via Responses API):
        //   - Only include reasoning when there are tool calls. Without a following function_call
        //     the API returns: "reasoning was provided without its required following item"
        //   - These models use internal reasoning IDs that are only meaningful when paired with
        //     a function call; standalone reasoning items are not valid for text-only turns.
        let is_openai_reasoning_provider = ctx.provider_name == "openai_reasoning";
        let is_openai_responses_api = ctx.provider_name == "openai_responses";
        let has_reasoning = !thinking_content.is_empty() || thinking_id.is_some();
        let should_include_reasoning = if is_openai_reasoning_provider {
            // Always include reasoning for openai_reasoning — rs_ IDs must be echoed back
            has_reasoning
        } else if is_openai_responses_api {
            // For openai_responses: only include reasoning when paired with a tool call
            has_reasoning && has_tool_calls
        } else {
            // For other providers (Anthropic, etc.): include reasoning when present
            has_reasoning
        };
        if supports_thinking && should_include_reasoning {
            tracing::info!(
                "[OpenAI Debug] Building assistant content with reasoning: id={:?}, signature_len={:?}",
                thinking_id,
                thinking_signature.as_ref().map(|s| s.len())
            );
            assistant_content.push(AssistantContent::Reasoning(
                Reasoning::new_with_signature(&thinking_content, thinking_signature.clone())
                    .optional_id(thinking_id.clone()),
            ));
        }

        if !text_content.is_empty() {
            assistant_content.push(AssistantContent::Text(Text {
                text: text_content.clone(),
            }));
        }

        // Add tool calls to assistant content if present
        for tool_call in &tool_calls_to_execute {
            assistant_content.push(AssistantContent::ToolCall(tool_call.clone()));
        }

        // ALWAYS add assistant message to history (even when no tool calls)
        // This is critical for maintaining conversation context across turns
        if !assistant_content.is_empty() {
            chat_history.push(Message::Assistant {
                id: None,
                content: OneOrMany::many(assistant_content).unwrap_or_else(|_| {
                    OneOrMany::one(AssistantContent::Text(Text {
                        text: String::new(),
                    }))
                }),
            });
        }

        // If no tool calls, either invoke reflector or finish
        if !has_tool_calls {
            consecutive_no_tool_turns += 1;

            // Reflector: if the agent produced text but no tool calls, and we haven't
            // exhausted reflector attempts, invoke the reflector to diagnose and correct.
            // Skip reflector when the gatekeeper classified the message as simple chat
            // (no memory/tool usage warranted) — the text-only response is expected.
            let should_reflect = consecutive_no_tool_turns <= 3
                && total_reflector_nudges < 3
                && !text_content.trim().is_empty()
                && config.enable_reflector
                && gatekeeper_wants_tools;

            if should_reflect {
                let registry = ctx.sub_agent_registry.read().await;
                if registry.get("reflector").is_some() {
                    drop(registry);

                    total_reflector_nudges += 1;
                    tracing::info!(
                        attempt = consecutive_no_tool_turns,
                        total_nudges = total_reflector_nudges,
                        text_len = text_content.len(),
                        "[reflector] Agent produced text without tool calls, generating correction"
                    );

                    let correction = format!(
                        "[System: You responded with text but did not use any tools. \
                         If you have completed the task, that's fine. \
                         Otherwise, please execute the next step using the appropriate tool. \
                         Available tools include: run_pty_cmd, read_file, write_file, \
                         web_search, web_fetch, search_memories, store_memory. \
                         Attempt {}/3]",
                        total_reflector_nudges
                    );

                    chat_history.push(Message::User {
                        content: OneOrMany::one(UserContent::Text(rig::message::Text {
                            text: correction,
                        })),
                    });
                    continue;
                }
            }

            break;
        } else {
            consecutive_no_tool_turns = 0;
        }

        // Execute tool calls and collect results (with concurrent dispatch for sub-agents)
        let total_tool_count = tool_calls_to_execute.len();
        let (sub_agent_calls, other_calls) = partition_tool_calls(tool_calls_to_execute);
        let has_concurrent_sub_agents = sub_agent_calls.len() >= 2;

        // Pre-allocate indexed results: (UserContent, Vec<system_hooks>)
        let mut indexed_results: Vec<Option<(UserContent, Vec<String>)>> = vec![None; total_tool_count];

        // Execute sub-agent calls concurrently if there are 2+
        if has_concurrent_sub_agents {
            tracing::info!(
                count = sub_agent_calls.len(),
                "Executing sub-agent tool calls concurrently"
            );

            let futures: Vec<_> = sub_agent_calls
                .into_iter()
                .map(|(original_idx, tool_call)| {
                    let llm_span = &llm_span;
                    let capture_ctx = &capture_ctx;
                    let sub_agent_context = &sub_agent_context;
                    let hook_registry = &hook_registry;
                    async move {
                        let result = execute_single_tool_call(
                            tool_call, ctx, capture_ctx, model, sub_agent_context,
                            hook_registry, llm_span,
                        )
                        .await;
                        (original_idx, result)
                    }
                })
                .collect();

            let concurrent_results = futures::future::join_all(futures).await;
            for (idx, result) in concurrent_results {
                indexed_results[idx] = Some(result);
            }
        } else {
            // 0 or 1 sub-agent calls — execute sequentially (no spawn overhead)
            for (original_idx, tool_call) in sub_agent_calls {
                let result = execute_single_tool_call(
                    tool_call, ctx, &capture_ctx, model, &sub_agent_context,
                    &hook_registry, &llm_span,
                )
                .await;
                indexed_results[original_idx] = Some(result);
            }
        }

        // Execute non-sub-agent calls sequentially (always)
        for (original_idx, tool_call) in other_calls {
            let result = execute_single_tool_call(
                tool_call, ctx, &capture_ctx, model, &sub_agent_context,
                &hook_registry, &llm_span,
            )
            .await;
            indexed_results[original_idx] = Some(result);
        }

        // Flatten results in original order
        let mut tool_results: Vec<UserContent> = Vec::with_capacity(total_tool_count);
        let mut system_hooks: Vec<String> = vec![];
        for (user_content, hooks) in indexed_results.into_iter().flatten() {
            tool_results.push(user_content);
            system_hooks.extend(hooks);
        }

        // Merge system hooks into the tool results message to avoid
        // "user after tool" ordering violations with OpenAI-compatible APIs.
        if !system_hooks.is_empty() {
            let formatted_hooks = format_system_hooks(&system_hooks);

            tracing::info!(
                count = system_hooks.len(),
                content_len = formatted_hooks.len(),
                "Injecting system hooks into tool results message"
            );

            let _ = ctx
                .event_tx
                .send(AiEvent::SystemHooksInjected { hooks: system_hooks.clone() });

            let _system_hook_event = tracing::info_span!(
                parent: &llm_span,
                "system_hooks_injected",
                "langfuse.observation.type" = "event",
                "langfuse.observation.level" = "DEFAULT",
                "langfuse.session.id" = ctx.session_id.unwrap_or(""),
                hook_count = system_hooks.len(),
                "langfuse.observation.input" = %formatted_hooks,
            );

            tool_results.push(UserContent::Text(Text {
                text: formatted_hooks,
            }));
        }

        // Add tool results (+ any system hooks) as a single user message
        chat_history.push(Message::User {
            content: OneOrMany::many(tool_results).unwrap_or_else(|_| {
                OneOrMany::one(UserContent::Text(Text {
                    text: "Tool executed".to_string(),
                }))
            }),
        });
    }

    // Log thinking stats at debug level
    if supports_thinking && !accumulated_thinking.is_empty() {
        tracing::debug!(
            "[Unified] Total thinking content: {} chars",
            accumulated_thinking.len()
        );
    }

    let agent_label = if config.is_sub_agent {
        format!("sub-agent (depth={})", sub_agent_context.depth)
    } else {
        "main-agent".to_string()
    };
    tracing::info!(
        "[{}] Turn complete: provider={}, model={}, tokens={{input={}, output={}, total={}}}",
        agent_label,
        ctx.provider_name,
        ctx.model_name,
        total_usage.input_tokens,
        total_usage.output_tokens,
        total_usage.total()
    );

        Ok::<_, anyhow::Error>((accumulated_response, accumulated_thinking, chat_history, total_usage))
    }
    .instrument(agent_span.clone())
    .instrument(chat_message_span.clone())
    .await?;

    // Record the final output on both trace and agent spans
    let output_for_span = if accumulated_response.len() > 2000 {
        format!("{}... [truncated]", truncate_str(&accumulated_response, 2000))
    } else {
        accumulated_response.clone()
    };
    chat_message_span.record("langfuse.observation.output", output_for_span.as_str());
    agent_span.record("langfuse.observation.output", output_for_span.as_str());

    // Record token usage to DB
    if let Some(tracker) = ctx.db_tracker {
        if total_usage.input_tokens > 0 || total_usage.output_tokens > 0 {
            tracker.record_token_usage(
                total_usage.input_tokens,
                total_usage.output_tokens,
                ctx.model_name,
                ctx.provider_name,
                0,
            );
        }
    }

    // Convert accumulated_thinking to Option (None if empty)
    let reasoning = if accumulated_thinking.is_empty() {
        None
    } else {
        Some(accumulated_thinking)
    };

    Ok((
        accumulated_response,
        reasoning,
        chat_history,
        Some(total_usage),
    ))
}

// =============================================================================
// CONTEXT COMPACTION ORCHESTRATION
// =============================================================================

/// Execute a single tool call with loop detection, HITL approval, event emission,
/// truncation, and post-tool hooks. Returns (UserContent, system_hooks).
///
/// This function is extracted from the tool execution loop to enable both
/// sequential and concurrent (via `join_all`) execution of tool calls.
#[allow(clippy::too_many_arguments)]
async fn execute_single_tool_call<M>(
    tool_call: ToolCall,
    ctx: &AgenticLoopContext<'_>,
    capture_ctx: &LoopCaptureContext,
    model: &M,
    sub_agent_context: &SubAgentContext,
    hook_registry: &HookRegistry,
    llm_span: &tracing::Span,
) -> (UserContent, Vec<String>)
where
    M: RigCompletionModel + Sync,
{
    let tool_name = &tool_call.function.name;
    let tool_args = if tool_name == "run_pty_cmd" || tool_name == "run_command" {
        normalize_run_pty_cmd_args(tool_call.function.arguments.clone())
    } else {
        tool_call.function.arguments.clone()
    };
    let tool_id = tool_call.id.clone();
    let tool_call_id = tool_call.call_id.clone().unwrap_or_else(|| tool_id.clone());

    tracing::info!(
        "[tool-dispatch] Executing tool: name={}, id={}, args_len={}",
        tool_name,
        tool_id,
        serde_json::to_string(&tool_args).map(|s| s.len()).unwrap_or(0),
    );

    // Create span for tool call
    let args_str = serde_json::to_string(&tool_args).unwrap_or_default();
    let args_for_span = if args_str.len() > 1000 {
        format!("{}... [truncated]", truncate_str(&args_str, 1000))
    } else {
        args_str
    };
    let tool_span = tracing::info_span!(
        parent: llm_span,
        "tool_call",
        "otel.name" = %tool_name,
        "langfuse.span.name" = %tool_name,
        "langfuse.observation.type" = "tool",
        "langfuse.session.id" = ctx.session_id.unwrap_or(""),
        tool.name = %tool_name,
        tool.id = %tool_id,
        "langfuse.observation.input" = %args_for_span,
        "langfuse.observation.output" = tracing::field::Empty,
        success = tracing::field::Empty,
    );

    // Check for loop detection
    let loop_result = {
        let mut detector = ctx.loop_detector.write().await;
        detector.record_tool_call(tool_name, &tool_args)
    };

    // Handle loop detection (may return a blocked result)
    if let Some(blocked_result) =
        handle_loop_detection(&loop_result, &tool_id, &tool_call_id, ctx.event_tx)
    {
        let loop_info = match &loop_result {
            crate::loop_detection::LoopDetectionResult::Blocked {
                repeat_count,
                max_count,
                ..
            } => format!("repeat_count={}, max={}", repeat_count, max_count),
            crate::loop_detection::LoopDetectionResult::MaxIterationsReached {
                iterations,
                max_iterations,
                ..
            } => format!("iterations={}, max={}", iterations, max_iterations),
            _ => String::new(),
        };
        let _loop_event = tracing::info_span!(
            parent: llm_span,
            "loop_blocked",
            "langfuse.observation.type" = "event",
            "langfuse.session.id" = ctx.session_id.unwrap_or(""),
            tool_name = %tool_name,
            details = %loop_info,
        );
        tool_span.record("success", false);
        tool_span.record("langfuse.observation.output", "blocked by loop detection");
        return (blocked_result, vec![]);
    }

    // Start DB tracking for tool call timing
    let db_guard = ctx
        .db_tracker
        .map(|t| t.start_tool_call(&tool_id, tool_name, &tool_args));

    // Execute tool with HITL approval check
    let mut result = execute_with_hitl_generic(
        tool_name,
        &tool_args,
        &tool_id,
        ctx,
        capture_ctx,
        model,
        sub_agent_context,
    )
    .await
    .unwrap_or_else(|e| ToolExecutionResult {
        value: json!({ "error": e.to_string() }),
        success: false,
    });

    // Tool Call Auto-Fixer: if execution failed with a schema/argument error,
    // try a lightweight LLM call to repair the args and retry once.
    if !result.success {
        let error_text = result.value.get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let tool_schema = {
            let registry = ctx.tool_registry.read().await;
            registry.get_tool_definitions()
                .into_iter()
                .find(|td| td.name == *tool_name)
                .map(|td| td.parameters)
        };

        if let Some(fixed_args) = toolcall_fixer::try_fix_tool_args(
            model,
            tool_name,
            &tool_args,
            &error_text,
            tool_schema.as_ref(),
        ).await {
            tracing::info!(
                "[toolcall-fixer] Retrying '{}' with repaired args",
                tool_name
            );
            result = execute_with_hitl_generic(
                tool_name,
                &fixed_args,
                &tool_id,
                ctx,
                capture_ctx,
                model,
                sub_agent_context,
            )
            .await
            .unwrap_or_else(|e| ToolExecutionResult {
                value: json!({ "error": e.to_string() }),
                success: false,
            });
        }
    }

    // Finish DB tracking with result
    if let (Some(tracker), Some(guard)) = (ctx.db_tracker, db_guard) {
        let result_text = serde_json::to_string(&result.value).unwrap_or_default();
        tracker.finish_tool_call(guard, result.success, &result_text);

        // Record search logs for web search tools
        if tool_name.starts_with("tavily_") || tool_name.starts_with("web_search") {
            let query = tool_args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let result_preview = serde_json::to_string(&result.value)
                .ok()
                .map(|s| truncate_str(&s, 10000).to_string());
            tracker.record_search(
                if tool_name.starts_with("tavily_") { "tavily" } else { "web" },
                query,
                result_preview.as_deref(),
            );
        }

        // Record terminal logs for shell/PTY commands
        if tool_name == "run_pty_cmd" || tool_name == "run_command" || tool_name == "run_shell_cmd" {
            let output = result.value.get("output")
                .or_else(|| result.value.get("stdout"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !output.is_empty() {
                tracker.record_terminal_output("stdout", output);
            }
            if let Some(stderr) = result.value.get("stderr").and_then(|v| v.as_str()) {
                if !stderr.is_empty() {
                    tracker.record_terminal_output("stderr", stderr);
                }
            }
        }

        // Memory gatekeeper: decide if this tool result is worth persisting
        tracker.maybe_store_tool_memory(tool_name, &tool_args, &result.value, result.success);
    }

    // Record tool result in span
    let result_str = serde_json::to_string(&result.value).unwrap_or_default();
    let result_for_span = if result_str.len() > 1000 {
        format!("{}... [truncated]", truncate_str(&result_str, 1000))
    } else {
        result_str
    };
    tool_span.record("langfuse.observation.output", result_for_span.as_str());
    tool_span.record("success", result.success);

    // Emit tool result event
    let result_event = AiEvent::ToolResult {
        tool_name: tool_name.clone(),
        result: result.value.clone(),
        success: result.success,
        request_id: tool_id.clone(),
        source: golish_core::events::ToolSource::Main,
    };
    emit_to_frontend(ctx, result_event.clone());
    capture_ctx.process(&result_event);

    // Execution Mentor check (PentAGI pattern): when the monitor detects
    // repetitive tool usage, generate corrective advice and append it.
    let mentor_advice = if let Some(ref monitor) = ctx.execution_monitor {
        let args_summary = serde_json::to_string(&tool_args).unwrap_or_default();
        let should_mentor = {
            let mut mon = monitor.write().await;
            mon.record_and_check(tool_name, &args_summary)
        };
        if should_mentor {
            let (repeated_tool, repeat_count, recent_summary) = {
                let mon = monitor.read().await;
                (
                    mon.repeated_tool_name().to_string(),
                    mon.same_tool_count(),
                    mon.recent_calls_summary(),
                )
            };
            tracing::info!(
                "[ExecutionMentor] Monitor triggered: '{}' called {} times, invoking mentor",
                repeated_tool,
                repeat_count,
            );
            // For now, provide a static corrective message.
            // TODO: Use LLM-based mentor when simple_completion is accessible here.
            let advice = format!(
                "\n\n--- EXECUTION ADVISOR ---\n\
                 You have called '{}' {} times. Consider a different approach:\n\
                 - Try a different tool to make progress\n\
                 - Check if previous results already contain the information you need\n\
                 - If stuck, use a different strategy entirely\n\
                 Recent calls: {}\n\
                 -------------------------",
                repeated_tool, repeat_count, recent_summary,
            );
            {
                let mut mon = monitor.write().await;
                mon.reset_after_mentor();
            }
            Some(advice)
        } else {
            None
        }
    } else {
        None
    };

    // Convert result to text and truncate if necessary
    let mut raw_result_text = serde_json::to_string(&result.value).unwrap_or_default();
    if let Some(ref advice) = mentor_advice {
        raw_result_text.push_str(advice);
    }
    let truncation_result = ctx
        .context_manager
        .truncate_tool_response(&raw_result_text, tool_name)
        .await;

    if truncation_result.truncated {
        let original_tokens = golish_context::TokenBudgetManager::estimate_tokens(&raw_result_text);
        let truncated_tokens =
            golish_context::TokenBudgetManager::estimate_tokens(&truncation_result.content);
        let _ = ctx.event_tx.send(AiEvent::ToolResponseTruncated {
            tool_name: tool_name.clone(),
            original_tokens,
            truncated_tokens,
        });
    }

    let user_content = UserContent::ToolResult(ToolResult {
        id: tool_id.clone(),
        call_id: Some(tool_call_id),
        content: OneOrMany::one(ToolResultContent::Text(Text {
            text: truncation_result.content,
        })),
    });

    // Run post-tool hooks
    let post_ctx = PostToolContext::new(
        tool_name,
        &tool_args,
        &result.value,
        result.success,
        0,
        ctx.session_id.unwrap_or(""),
    );
    let hooks = hook_registry.run_post_tool_hooks(&post_ctx);

    (user_content, hooks)
}


#[cfg(test)]
mod concurrent_dispatch_tests {
    use super::*;

    fn make_tool_call(name: &str, id: &str) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            call_id: Some(id.to_string()),
            function: rig::message::ToolFunction {
                name: name.to_string(),
                arguments: json!({}),
            },
            signature: None,
            additional_params: None,
        }
    }

    #[test]
    fn test_is_sub_agent_tool() {
        assert!(is_sub_agent_tool("sub_agent_coder"));
        assert!(is_sub_agent_tool("sub_agent_explorer"));
        assert!(!is_sub_agent_tool("read_file"));
        assert!(!is_sub_agent_tool("run_pty_cmd"));
    }

    #[test]
    fn test_partition_tool_calls_mixed() {
        let calls = vec![
            make_tool_call("read_file", "tc1"),
            make_tool_call("sub_agent_coder", "tc2"),
            make_tool_call("write_file", "tc3"),
            make_tool_call("sub_agent_explorer", "tc4"),
        ];
        let (sub_agents, others) = partition_tool_calls(calls);
        assert_eq!(sub_agents.len(), 2);
        assert_eq!(others.len(), 2);
        assert_eq!(sub_agents[0].0, 1);
        assert_eq!(sub_agents[1].0, 3);
        assert_eq!(others[0].0, 0);
        assert_eq!(others[1].0, 2);
    }

    #[test]
    fn test_partition_tool_calls_empty() {
        let (sub_agents, others) = partition_tool_calls(vec![]);
        assert_eq!(sub_agents.len(), 0);
        assert_eq!(others.len(), 0);
    }
}

#[cfg(test)]
mod loop_capture_context_tests {
    use super::*;

    #[test]
    fn test_loop_capture_context_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<LoopCaptureContext>();
    }

    #[test]
    fn test_loop_capture_context_shared_ref_process() {
        let ctx = LoopCaptureContext::new(None);
        let event = AiEvent::ToolRequest {
            request_id: "test".to_string(),
            tool_name: "read_file".to_string(),
            args: json!({}),
            source: golish_core::events::ToolSource::Main,
        };
        ctx.process(&event);
        ctx.process(&event);
    }

    #[tokio::test]
    async fn test_loop_capture_context_concurrent_access() {
        let ctx = Arc::new(LoopCaptureContext::new(None));
        let mut handles = vec![];
        for i in 0..5 {
            let ctx = Arc::clone(&ctx);
            handles.push(tokio::spawn(async move {
                let event = AiEvent::ToolRequest {
                    request_id: format!("req-{}", i),
                    tool_name: "read_file".to_string(),
                    args: json!({}),
                    source: golish_core::events::ToolSource::Main,
                };
                ctx.process(&event);
            }));
        }
        for handle in handles {
            handle.await.unwrap();
        }
    }
}

#[cfg(test)]
mod unified_loop_tests {
    use super::*;

    #[test]
    fn test_agentic_loop_config_main_agent_anthropic() {
        let config = AgenticLoopConfig::main_agent_anthropic();
        assert!(
            config.capabilities.supports_thinking_history,
            "Anthropic config should support thinking history"
        );
        assert!(
            config.capabilities.supports_temperature,
            "Anthropic config should support temperature"
        );
        assert!(config.require_hitl, "Main agent should require HITL");
        assert!(!config.is_sub_agent, "Main agent should not be sub-agent");
    }

    #[test]
    fn test_agentic_loop_config_main_agent_generic() {
        let config = AgenticLoopConfig::main_agent_generic();
        assert!(
            !config.capabilities.supports_thinking_history,
            "Generic config should not support thinking history"
        );
        assert!(
            config.capabilities.supports_temperature,
            "Generic config should support temperature"
        );
        assert!(config.require_hitl, "Main agent should require HITL");
        assert!(!config.is_sub_agent, "Main agent should not be sub-agent");
    }

    #[test]
    fn test_agentic_loop_config_sub_agent() {
        let config = AgenticLoopConfig::sub_agent(ModelCapabilities::conservative_defaults());
        assert!(
            !config.capabilities.supports_thinking_history,
            "Conservative defaults should not support thinking history"
        );
        assert!(!config.require_hitl, "Sub-agent should not require HITL");
        assert!(config.is_sub_agent, "Should be marked as sub-agent");
    }

    #[test]
    fn test_agentic_loop_config_sub_agent_with_anthropic_capabilities() {
        let config = AgenticLoopConfig::sub_agent(ModelCapabilities::anthropic_defaults());
        assert!(
            config.capabilities.supports_thinking_history,
            "Anthropic sub-agent should support thinking history"
        );
        assert!(!config.require_hitl, "Sub-agent should not require HITL");
        assert!(config.is_sub_agent, "Should be marked as sub-agent");
    }

    #[test]
    fn test_agentic_loop_config_with_detection_anthropic() {
        let config = AgenticLoopConfig::with_detection("anthropic", "claude-3-opus", false);
        assert!(
            config.capabilities.supports_thinking_history,
            "Anthropic detection should enable thinking history"
        );
        assert!(
            config.capabilities.supports_temperature,
            "Anthropic detection should enable temperature"
        );
        assert!(config.require_hitl, "Non-sub-agent should require HITL");
        assert!(!config.is_sub_agent);
    }

    #[test]
    fn test_agentic_loop_config_with_detection_openai_reasoning() {
        let config = AgenticLoopConfig::with_detection("openai", "o3-mini", false);
        assert!(
            config.capabilities.supports_thinking_history,
            "OpenAI reasoning model should support thinking history"
        );
        assert!(
            !config.capabilities.supports_temperature,
            "OpenAI reasoning model should not support temperature"
        );
        assert!(config.require_hitl);
    }

    #[test]
    fn test_agentic_loop_config_with_detection_openai_regular() {
        let config = AgenticLoopConfig::with_detection("openai", "gpt-4o", false);
        assert!(
            !config.capabilities.supports_thinking_history,
            "Regular OpenAI model should not support thinking history"
        );
        assert!(
            config.capabilities.supports_temperature,
            "Regular OpenAI model should support temperature"
        );
    }

    #[test]
    fn test_agentic_loop_config_with_detection_sub_agent() {
        let config = AgenticLoopConfig::with_detection("openai", "gpt-4o", true);
        assert!(!config.require_hitl, "Sub-agent should not require HITL");
        assert!(config.is_sub_agent, "Should be marked as sub-agent");
    }

    #[test]
    fn test_agentic_loop_config_with_detection_openai_gpt5_series() {
        // GPT-5 base model
        let config = AgenticLoopConfig::with_detection("openai", "gpt-5", false);
        assert!(
            config.capabilities.supports_thinking_history,
            "GPT-5 should support thinking history (reasoning model)"
        );
        assert!(
            !config.capabilities.supports_temperature,
            "GPT-5 should not support temperature (reasoning model)"
        );

        // GPT-5.1
        let config = AgenticLoopConfig::with_detection("openai", "gpt-5.1", false);
        assert!(
            config.capabilities.supports_thinking_history,
            "GPT-5.1 should support thinking history"
        );
        assert!(
            !config.capabilities.supports_temperature,
            "GPT-5.1 should not support temperature"
        );

        // GPT-5.2
        let config = AgenticLoopConfig::with_detection("openai", "gpt-5.2", false);
        assert!(
            config.capabilities.supports_thinking_history,
            "GPT-5.2 should support thinking history"
        );
        assert!(
            !config.capabilities.supports_temperature,
            "GPT-5.2 should not support temperature"
        );

        // GPT-5-mini
        let config = AgenticLoopConfig::with_detection("openai", "gpt-5-mini", false);
        assert!(
            config.capabilities.supports_thinking_history,
            "GPT-5-mini should support thinking history"
        );
        assert!(
            !config.capabilities.supports_temperature,
            "GPT-5-mini should not support temperature"
        );
    }

    #[test]
    fn test_agentic_loop_config_with_detection_openai_responses_gpt5() {
        // OpenAI Responses API with GPT-5.2
        let config = AgenticLoopConfig::with_detection("openai_responses", "gpt-5.2", false);
        assert!(
            config.capabilities.supports_thinking_history,
            "OpenAI Responses API should support thinking history"
        );
        assert!(
            !config.capabilities.supports_temperature,
            "GPT-5.2 via Responses API should not support temperature"
        );

        // Contrast with GPT-4.1 which DOES support temperature
        let config = AgenticLoopConfig::with_detection("openai_responses", "gpt-4.1", false);
        assert!(
            config.capabilities.supports_thinking_history,
            "OpenAI Responses API should support thinking history"
        );
        assert!(
            config.capabilities.supports_temperature,
            "GPT-4.1 via Responses API should support temperature"
        );
    }

    #[test]
    fn test_agentic_loop_config_with_detection_openai_codex() {
        // Codex models don't support temperature
        let config = AgenticLoopConfig::with_detection("openai", "gpt-5.1-codex-max", false);
        assert!(
            !config.capabilities.supports_temperature,
            "Codex models should not support temperature"
        );

        let config = AgenticLoopConfig::with_detection("openai_responses", "gpt-5.2-codex", false);
        assert!(
            !config.capabilities.supports_temperature,
            "Codex models via Responses API should not support temperature"
        );
    }
}

#[cfg(test)]
mod repetitive_text_tests {
    use super::*;

    #[test]
    fn test_short_text_not_repetitive() {
        assert!(!detect_repetitive_text("你好"));
        assert!(!detect_repetitive_text(""));
        assert!(!detect_repetitive_text("这是一个正常的回答。"));
    }

    #[test]
    fn test_normal_text_not_repetitive() {
        let text = "example.com 是一个官方保留的测试域名。\
                    它解析到 104.20.23.154 和 172.66.147.243。\
                    这些地址由 Cloudflare 托管。";
        assert!(!detect_repetitive_text(text));
    }

    #[test]
    fn test_repeated_sentences_detected() {
        // Simulate real degenerate output: repeated "I've completed your request" sentences
        let text = "该网站运行的是一个基于Vue3构建的前端应用，名为管理系统，以下是关键发现。\
                    我已经完成了对该网站的JavaScript代码分析。如果你有其他需要测试或分析的域名或目标，请告诉我。\
                    我已经完成了对该网站的JavaScript代码分析。如果你有其他需要，请直接告诉我。\
                    我已经完成了对该网站的JavaScript代码分析。请告诉我你接下来需要什么帮助。";
        assert!(detect_repetitive_text(text));
    }

    #[test]
    fn test_repeated_english_detected() {
        let text = "The scan has completed successfully and found the following services running on the target.\n\
                    I have completed your request. Let me know if you need anything else or any other targets to scan.\n\
                    I have completed your request. If you have other targets or need further analysis, let me know.\n\
                    I have completed your request. Please tell me what you need next or if there are other targets.\n";
        assert!(detect_repetitive_text(text));
    }

    #[test]
    fn test_two_similar_not_detected() {
        // Only 2 repeats — threshold is 3
        let text = "该网站运行的是一个基于Vue3构建的前端应用，名为管理系统，以下是关键发现。\
                    我已经完成了对该网站的JavaScript代码分析。如果你有其他需要测试或分析的域名或目标，请告诉我。\
                    我已经完成了对该网站的JavaScript代码分析。请告诉我你接下来需要什么帮助。";
        assert!(!detect_repetitive_text(text));
    }
}

#[cfg(test)]
mod utf8_truncation_tests {
    #[test]
    fn test_utf8_safe_truncation_ascii() {
        let text = "Hello, World!";
        let mut end = 5;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        assert_eq!(&text[..end], "Hello");
    }

    #[test]
    fn test_utf8_safe_truncation_multibyte() {
        // "─" is 3 bytes (E2 94 80), testing truncation at various positions
        let text = "abc─def"; // a=0, b=1, c=2, ─=3-5, d=6, e=7, f=8

        // Truncate at position 4 (middle of ─)
        let mut end = 4;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        assert_eq!(end, 3); // Should back up to position 3 (start of ─)
        assert_eq!(&text[..end], "abc");

        // Truncate at position 5 (still in ─)
        let mut end = 5;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        assert_eq!(end, 3);
        assert_eq!(&text[..end], "abc");

        // Truncate at position 6 (after ─)
        let mut end = 6;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        assert_eq!(end, 6);
        assert_eq!(&text[..end], "abc─");
    }

    #[test]
    fn test_utf8_safe_truncation_emoji() {
        // Emoji like 🎉 is 4 bytes
        let text = "Hi🎉!"; // H=0, i=1, 🎉=2-5, !=6

        // Truncate at position 3 (middle of emoji)
        let mut end = 3;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        assert_eq!(end, 2);
        assert_eq!(&text[..end], "Hi");

        // Truncate at position 6 (after emoji)
        let mut end = 6;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        assert_eq!(end, 6);
        assert_eq!(&text[..end], "Hi🎉");
    }

    #[test]
    fn test_utf8_safe_truncation_mixed_box_drawing() {
        // Box drawing characters like those that caused the original panic
        let text = "Summary:\n─────────";
        let target = 12; // Might land in middle of a box char

        let mut end = target.min(text.len());
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }

        // Should not panic and result should be valid UTF-8
        let truncated = &text[..end];
        assert!(truncated.len() <= target);
        // Verify it's valid UTF-8 by checking we can iterate chars
        assert!(truncated.chars().count() > 0);
    }
}


#[cfg(test)]
mod token_estimation_tests {
    use super::*;

    fn user_text_msg(text: &str) -> Message {
        Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: text.to_string(),
            })),
        }
    }

    fn assistant_text_msg(text: &str) -> Message {
        Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::Text(Text {
                text: text.to_string(),
            })),
        }
    }

    fn tool_result_msg(id: &str, result_text: &str) -> Message {
        Message::User {
            content: OneOrMany::one(UserContent::ToolResult(ToolResult {
                id: id.to_string(),
                call_id: Some(id.to_string()),
                content: OneOrMany::one(ToolResultContent::Text(Text {
                    text: result_text.to_string(),
                })),
            })),
        }
    }

    fn tool_call_msg(name: &str, args: serde_json::Value) -> Message {
        Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::ToolCall(ToolCall {
                id: "call_1".to_string(),
                call_id: Some("call_1".to_string()),
                function: rig::message::ToolFunction {
                    name: name.to_string(),
                    arguments: args,
                },
                signature: None,
                additional_params: None,
            })),
        }
    }

    #[test]
    fn test_estimate_user_text_message() {
        let msg = user_text_msg("Hello, how are you doing today?");
        let tokens = estimate_message_tokens(&msg);
        // ~7 words, should be roughly 7-8 tokens
        assert!(
            (5..=12).contains(&tokens),
            "Simple text should estimate 5-12 tokens, got {}",
            tokens
        );
    }

    #[test]
    fn test_estimate_empty_message() {
        let msg = user_text_msg("");
        let tokens = estimate_message_tokens(&msg);
        assert_eq!(tokens, 0, "Empty message should be 0 tokens");
    }

    #[test]
    fn test_estimate_large_tool_result() {
        // Simulate reading a file — this is the key scenario for proactive counting
        let file_content = "use std::collections::HashMap;\n".repeat(200);
        let msg = tool_result_msg("read_file_1", &file_content);
        let tokens = estimate_message_tokens(&msg);

        // ~6000 chars of code, should be well over 1000 tokens
        assert!(
            tokens > 1000,
            "Large tool result should estimate >1000 tokens, got {}",
            tokens
        );
        assert!(
            tokens < 3000,
            "Large tool result should not wildly overcount, got {}",
            tokens
        );
    }

    #[test]
    fn test_estimate_tool_call_message() {
        let args = json!({
            "path": "src/main.rs",
            "line_start": 1,
            "line_end": 50
        });
        let msg = tool_call_msg("read_file", args);
        let tokens = estimate_message_tokens(&msg);
        assert!(
            tokens > 5,
            "Tool call should estimate some tokens, got {}",
            tokens
        );
    }

    #[test]
    fn test_estimate_assistant_text() {
        let msg = assistant_text_msg(
            "I'll help you with that. Let me read the file first to understand the codebase.",
        );
        let tokens = estimate_message_tokens(&msg);
        assert!(
            (10..=25).contains(&tokens),
            "Assistant text should estimate 10-25 tokens, got {}",
            tokens
        );
    }

    #[test]
    fn test_estimate_multiple_messages_accumulate() {
        // Simulate a realistic tool-heavy conversation fragment
        let messages = [
            user_text_msg("Read the main.rs file and fix the bug"),
            tool_call_msg("read_file", json!({"path": "src/main.rs"})),
            tool_result_msg("r1", &"fn main() { todo!() }\n".repeat(100)),
            tool_call_msg(
                "edit_file",
                json!({"path": "src/main.rs", "old_text": "todo!()", "new_text": "println!(\"fixed\")"}),
            ),
            tool_result_msg("r2", r#"{"success": true, "path": "src/main.rs"}"#),
        ];

        let total: usize = messages.iter().map(estimate_message_tokens).sum();

        // Should be dominated by the large tool result (~2200 chars of code)
        assert!(
            total > 400,
            "Multi-message conversation should estimate >400 tokens, got {}",
            total
        );
    }

    #[test]
    fn test_estimate_extracts_tool_result_content() {
        // Tests that estimate_message_tokens correctly extracts text from ToolResult
        // (our extraction logic, not tokenx-rs accuracy)
        let small_result = tool_result_msg("r1", "ok");
        let large_result = tool_result_msg("r1", &"x".repeat(10_000));

        let small_tokens = estimate_message_tokens(&small_result);
        let large_tokens = estimate_message_tokens(&large_result);

        assert!(small_tokens > 0, "Non-empty tool result should have tokens");
        assert!(
            large_tokens > small_tokens * 10,
            "10x larger content should produce substantially more tokens (small={}, large={})",
            small_tokens,
            large_tokens
        );
    }

    #[test]
    fn test_estimate_extracts_tool_call_args() {
        // Tests that estimate_message_tokens serializes and counts tool call arguments
        let small_call = tool_call_msg("read_file", json!({"path": "a.rs"}));
        let large_call = tool_call_msg(
            "edit_file",
            json!({
                "path": "src/very/long/path/to/some/module.rs",
                "old_text": "fn old() { todo!() }".repeat(50),
                "new_text": "fn new() { println!(\"done\") }".repeat(50),
            }),
        );

        let small_tokens = estimate_message_tokens(&small_call);
        let large_tokens = estimate_message_tokens(&large_call);

        assert!(small_tokens > 0, "Tool call should produce tokens");
        assert!(
            large_tokens > small_tokens,
            "Larger args should produce more tokens (small={}, large={})",
            small_tokens,
            large_tokens
        );
    }

    #[test]
    fn test_estimate_messages_scale_linearly() {
        // Adding more messages should increase the total proportionally
        let one_msg: usize = std::iter::once(user_text_msg("Hello world"))
            .map(|m| estimate_message_tokens(&m))
            .sum();

        let five_msgs: usize = (0..5)
            .map(|_| user_text_msg("Hello world"))
            .map(|m| estimate_message_tokens(&m))
            .sum();

        assert_eq!(
            five_msgs,
            one_msg * 5,
            "Token count should scale linearly with identical messages"
        );
    }

    #[test]
    fn test_tool_heavy_session_compaction_pipeline() {
        // End-to-end: builds realistic messages → estimate_message_tokens → compaction state → should_compact
        // Tests the full pipeline without testing tokenx-rs accuracy
        use golish_context::context_manager::{CompactionState, ContextManagerConfig};
        use golish_context::ContextManager;

        let manager = ContextManager::with_config(
            "claude-3-5-sonnet",
            ContextManagerConfig {
                enabled: true,
                compaction_threshold: 0.80,
                ..Default::default()
            },
        );

        // Build messages with tool results of known relative sizes
        let small_session: Vec<Message> = vec![user_text_msg("hello"), tool_result_msg("r1", "ok")];

        let large_session: Vec<Message> = (0..50)
            .flat_map(|i| {
                vec![
                    tool_call_msg("read_file", json!({"path": format!("file_{}.rs", i)})),
                    tool_result_msg(&format!("r{}", i), &"use std::io::Result;\n".repeat(200)),
                ]
            })
            .collect();

        let small_tokens: u64 = small_session
            .iter()
            .map(estimate_message_tokens)
            .sum::<usize>() as u64;
        let large_tokens: u64 = large_session
            .iter()
            .map(estimate_message_tokens)
            .sum::<usize>() as u64;

        // Small session should not trigger compaction
        let mut state = CompactionState::new();
        state.update_tokens_estimated(small_tokens);
        assert!(
            !manager
                .should_compact(&state, "claude-3-5-sonnet")
                .should_compact,
            "Small session ({} tokens) should not trigger compaction",
            small_tokens
        );

        // Large session (50 file reads) should produce enough tokens to matter
        // The exact threshold depends on tokenx-rs output, but 50 files x 200 lines
        // should be substantial
        assert!(
            large_tokens > small_tokens * 100,
            "Large session should be much bigger than small (small={}, large={})",
            small_tokens,
            large_tokens
        );
    }
}

#[cfg(test)]
mod openai_tracing_tests {
    use super::*;
    use crate::test_utils::{MockCompletionModel, MockResponse, TestContextBuilder};
    use golish_llm_providers::LlmClient;
    use golish_sub_agents::SubAgentContext;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn openai_reasoning_sub_context() -> SubAgentContext {
        SubAgentContext {
            original_request: "Test OpenAI tracing".to_string(),
            ..Default::default()
        }
    }

    /// Verify that Reasoning events are emitted when the model returns thinking content.
    /// This is critical for GPT-5.2/Codex: thinking shown in the UI must also appear in traces.
    #[tokio::test]
    async fn test_openai_reasoning_emits_reasoning_event() {
        let test_ctx = TestContextBuilder::new()
            .agent_mode(crate::agent_mode::AgentMode::AutoApprove)
            .build()
            .await;

        // Model returns thinking + text (simulates gpt-5.2 with reasoning summary)
        let model = MockCompletionModel::new(vec![MockResponse::text_with_thinking(
            "I will read the file now.",
            "Let me think: I should use read_file to inspect the codebase.",
        )]);

        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let mut ctx = test_ctx.as_agentic_context_with_client(&client);
        // Use openai_reasoning provider to test the correct code path
        ctx.provider_name = "openai_reasoning";
        ctx.model_name = "gpt-5.2";

        let initial_history = vec![rig::completion::Message::User {
            content: rig::one_or_many::OneOrMany::one(rig::message::UserContent::Text(
                rig::message::Text {
                    text: "Read the main.rs file".to_string(),
                },
            )),
        }];

        let result = run_agentic_loop_generic(
            &model,
            "You are a helpful assistant.",
            initial_history,
            openai_reasoning_sub_context(),
            &ctx,
        )
        .await;

        assert!(result.is_ok(), "Loop should succeed: {:?}", result.err());
        let (response, reasoning, _history, _usage) = result.unwrap();

        // The reasoning content must be returned (for Langfuse span recording)
        assert!(
            reasoning.is_some(),
            "Reasoning content must be returned when model provides thinking"
        );
        assert!(
            reasoning.as_ref().unwrap().contains("read_file"),
            "Reasoning should contain thinking content, got: {:?}",
            reasoning
        );

        // The response text must also be present
        assert!(
            response.contains("I will read"),
            "Response should contain model text, got: {:?}",
            response
        );

        // Verify AiEvent::Reasoning was emitted (so UI ThinkingBlock works)
        let mut test_ctx = test_ctx;
        let events = test_ctx.collect_events();
        let reasoning_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, AiEvent::Reasoning { .. }))
            .collect();
        assert!(
            !reasoning_events.is_empty(),
            "AiEvent::Reasoning must be emitted for UI ThinkingBlock, but no Reasoning events found"
        );
    }

    /// Verify that a tool-call-only response (no text) still produces a Completed event
    /// with token usage, and that the loop correctly handles the no-text case.
    /// GPT-5.2/Codex commonly return tool calls without any accompanying text.
    #[tokio::test]
    async fn test_openai_tool_call_only_response_completes() {
        let test_ctx = TestContextBuilder::new()
            .agent_mode(crate::agent_mode::AgentMode::AutoApprove)
            .build()
            .await;

        // Create a file the tool can actually read
        let ws = test_ctx.workspace_path().await;
        std::fs::write(ws.join("test.txt"), "hello world").unwrap();

        // First response: tool call only (no text) — simulates gpt-5.2 behaviour
        // Second response: text summary
        let model = MockCompletionModel::new(vec![
            MockResponse::tool_call("read_file", serde_json::json!({"path": "test.txt"})),
            MockResponse::text("I read the file and it contains 'hello world'."),
        ]);

        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let mut ctx = test_ctx.as_agentic_context_with_client(&client);
        ctx.provider_name = "openai_reasoning";
        ctx.model_name = "gpt-5.2";

        let initial_history = vec![rig::completion::Message::User {
            content: rig::one_or_many::OneOrMany::one(rig::message::UserContent::Text(
                rig::message::Text {
                    text: "Read test.txt".to_string(),
                },
            )),
        }];

        let result = run_agentic_loop_generic(
            &model,
            "You are a helpful assistant.",
            initial_history,
            openai_reasoning_sub_context(),
            &ctx,
        )
        .await;

        assert!(
            result.is_ok(),
            "Loop should succeed even with tool-call-only first response: {:?}",
            result.err()
        );
        let (response, _reasoning, _history, _usage) = result.unwrap();
        assert!(
            response.contains("hello world"),
            "Final response should include file content reference, got: {:?}",
            response
        );

        // Verify the loop produced a final text response (loop emits TextDelta events)
        // Note: AiEvent::Completed is emitted by agent_bridge.rs, not run_agentic_loop_generic directly.
        let mut test_ctx = test_ctx;
        let events = test_ctx.collect_events();
        let text_deltas: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, AiEvent::TextDelta { .. }))
            .collect();
        assert!(
            !text_deltas.is_empty(),
            "TextDelta events must be emitted for the text response after the tool call"
        );
        // Also verify a tool was auto-approved (auto-approve mode was set)
        let auto_approved = events
            .iter()
            .any(|e| matches!(e, AiEvent::ToolAutoApproved { .. }));
        assert!(
            auto_approved,
            "Tool should have been auto-approved in AutoApprove mode"
        );
    }

    /// Verify that reasoning/thinking content from the model is returned in the
    /// (response, reasoning, history, usage) tuple so the caller can record it on spans.
    #[tokio::test]
    async fn test_openai_thinking_returned_in_result() {
        let test_ctx = TestContextBuilder::new()
            .agent_mode(crate::agent_mode::AgentMode::AutoApprove)
            .build()
            .await;

        let thinking = "Step 1: understand the request. Step 2: formulate response.";
        let model = MockCompletionModel::new(vec![
            MockResponse::text("Here is my answer.").with_thinking(thinking)
        ]);

        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let mut ctx = test_ctx.as_agentic_context_with_client(&client);
        ctx.provider_name = "openai_reasoning";
        ctx.model_name = "gpt-5.2-codex";

        let initial_history = vec![rig::completion::Message::User {
            content: rig::one_or_many::OneOrMany::one(rig::message::UserContent::Text(
                rig::message::Text {
                    text: "What is 2+2?".to_string(),
                },
            )),
        }];

        let (_, reasoning, _, _) = run_agentic_loop_generic(
            &model,
            "You are a math tutor.",
            initial_history,
            openai_reasoning_sub_context(),
            &ctx,
        )
        .await
        .unwrap();

        assert!(
            reasoning.is_some(),
            "Reasoning must be returned when model provides thinking content"
        );
        let r = reasoning.unwrap();
        assert!(
            r.contains("Step 1"),
            "Returned reasoning should match model thinking, got: {:?}",
            r
        );
    }

    /// Verify that the "openai_reasoning" provider correctly detects model capabilities
    /// so the loop uses the right temperature/thinking settings.
    #[test]
    fn test_openai_reasoning_loop_config_detection() {
        // gpt-5.2 via openai_reasoning: reasoning model, no temperature, thinking history
        let config = AgenticLoopConfig::with_detection("openai_reasoning", "gpt-5.2", false);
        assert!(
            config.capabilities.supports_thinking_history,
            "gpt-5.2 via openai_reasoning must support thinking history for span recording"
        );
        assert!(
            !config.capabilities.supports_temperature,
            "gpt-5.2 via openai_reasoning must not use temperature"
        );

        // gpt-5.2-codex via openai_reasoning
        let config = AgenticLoopConfig::with_detection("openai_reasoning", "gpt-5.2-codex", false);
        assert!(
            config.capabilities.supports_thinking_history,
            "gpt-5.2-codex via openai_reasoning must support thinking history"
        );
        assert!(
            !config.capabilities.supports_temperature,
            "gpt-5.2-codex must not use temperature"
        );

        // o4-mini via openai_reasoning
        let config = AgenticLoopConfig::with_detection("openai_reasoning", "o4-mini", false);
        assert!(
            config.capabilities.supports_thinking_history,
            "o4-mini via openai_reasoning must support thinking history"
        );
        assert!(
            !config.capabilities.supports_temperature,
            "o4-mini must not use temperature"
        );
    }

    /// Verify that "openai_reasoning" ALWAYS includes reasoning in conversation history,
    /// even for text-only responses (no tool calls). The OpenAI Responses API tracks rs_...
    /// IDs server-side and requires them to be echoed back in every subsequent turn.
    ///
    /// Contrast with "openai_responses" where reasoning must only be included when paired
    /// with a tool call.
    #[tokio::test]
    async fn test_openai_reasoning_includes_reasoning_in_history_for_text_only_turns() {
        let test_ctx = TestContextBuilder::new()
            .agent_mode(crate::agent_mode::AgentMode::AutoApprove)
            .build()
            .await;

        // Model returns thinking + text (no tool calls). For openai_reasoning, the reasoning
        // MUST be included in history so OpenAI can find the rs_... item on the next turn.
        let model = MockCompletionModel::new(vec![MockResponse::text_with_thinking(
            "The answer is 4.",
            "Simple arithmetic: 2+2=4",
        )]);

        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let mut ctx = test_ctx.as_agentic_context_with_client(&client);
        ctx.provider_name = "openai_reasoning";
        ctx.model_name = "gpt-5.2";

        let initial_history = vec![rig::completion::Message::User {
            content: rig::one_or_many::OneOrMany::one(rig::message::UserContent::Text(
                rig::message::Text {
                    text: "What is 2+2?".to_string(),
                },
            )),
        }];

        let result = run_agentic_loop_generic(
            &model,
            "You are a math tutor.",
            initial_history,
            openai_reasoning_sub_context(),
            &ctx,
        )
        .await;

        assert!(result.is_ok(), "Loop should succeed: {:?}", result.err());
        let (response, _reasoning, history, _usage) = result.unwrap();
        assert!(response.contains("4"), "Response should contain the answer");

        // For openai_reasoning, the Reasoning block MUST be present in the assistant history
        // even for text-only turns. OpenAI's server tracks rs_... IDs and requires them on
        // subsequent turns (failing with "Item 'rs_...' was provided without its required
        // following item" if a previously-seen rs_ ID is absent from the next request).
        let has_reasoning_in_history = history.iter().any(|msg| {
            if let rig::completion::Message::Assistant { content, .. } = msg {
                content
                    .iter()
                    .any(|c| matches!(c, rig::completion::AssistantContent::Reasoning(_)))
            } else {
                false
            }
        });
        assert!(
            has_reasoning_in_history,
            "openai_reasoning MUST include reasoning in history for text-only turns \
             so OpenAI can find the rs_... item on subsequent turns"
        );
    }
}
