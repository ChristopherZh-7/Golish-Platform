//! Agent bridge for LLM interaction.
//!
//! This module provides the main AgentBridge struct that orchestrates:
//! - LLM communication (vtcode-core and Vertex AI Anthropic)
//! - Tool execution with HITL approval
//! - Conversation history management
//! - Session persistence
//! - Context window management
//! - Loop detection
//!
//! The implementation is split across multiple extension modules:
//! - `bridge_session` - Session persistence and conversation history
//! - `bridge_hitl` - HITL approval handling
//! - `bridge_policy` - Tool policies and loop protection
//! - `bridge_context` - Context window management
//!
//! Core execution logic is in:
//! - `agentic_loop` - Main tool execution loop
//! - `system_prompt` - System prompt building
//! - `sub_agent_executor` - Sub-agent execution

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use rig::completion::Message;
use rig::message::{AssistantContent, Text, UserContent};
use rig::one_or_many::OneOrMany;
use tokio::sync::{mpsc, oneshot, RwLock};

use golish_tools::ToolRegistry;

use crate::hitl::ApprovalRecorder;
use golish_core::events::{AiEvent, AiEventEnvelope};
use golish_core::hitl::ApprovalDecision;
use golish_core::{ApiRequestStats, ApiRequestStatsSnapshot};

use super::agent_mode::AgentMode;
use super::agentic_loop::{AgenticLoopContext, TerminalErrorEmitted};
use super::contributors::create_default_contributors;
use super::llm_client::LlmClient;
use super::prompt_registry::PromptContributorRegistry;
use super::system_prompt::build_system_prompt_with_contributions;
use super::tool_definitions::ToolConfig;
use crate::loop_detection::LoopDetector;
use crate::tool_policy::ToolPolicyManager;
use golish_context::token_budget::TokenUsage;
use golish_context::{CompactionState, ContextManager};
use golish_core::runtime::{GolishRuntime, RuntimeEvent};
use golish_core::{PromptContext, PromptMatchedSkill, PromptSkillInfo};
use golish_session::GolishSessionManager;
use golish_sub_agents::SubAgentRegistry;

use crate::indexer::IndexerState;
use crate::planner::PlanManager;

use golish_pty::PtyManager;
use golish_sidecar::SidecarState;
use golish_skills::SkillMetadata;

use crate::event_coordinator::CoordinatorHandle;
use crate::transcript::TranscriptWriter;

mod constructors;
mod execution;

fn should_emit_execution_error_event(error: &anyhow::Error) -> bool {
    !error.is::<TerminalErrorEmitted>()
}

#[derive(Debug, Clone)]
struct TerminalErrorState {
    partial_response: Option<String>,
    final_history: Option<Vec<Message>>,
}

fn extract_terminal_error_state(error: &anyhow::Error) -> Option<TerminalErrorState> {
    let terminal_error = error.downcast_ref::<TerminalErrorEmitted>()?;

    Some(TerminalErrorState {
        partial_response: terminal_error.partial_response().map(ToOwned::to_owned),
        final_history: terminal_error.final_history().map(ToOwned::to_owned),
    })
}

/// Bridge between Golish and LLM providers.
/// Handles LLM streaming and tool execution.
pub struct AgentBridge {
    // Core fields
    pub(crate) workspace: Arc<RwLock<PathBuf>>,
    pub(crate) provider_name: String,
    pub(crate) model_name: String,
    pub(crate) tool_registry: Arc<RwLock<ToolRegistry>>,
    pub(crate) client: Arc<RwLock<LlmClient>>,

    // Event emission - dual mode during transition
    // The event_tx channel is the legacy path, runtime is the new abstraction.
    // During transition, emit_event() sends through BOTH to verify parity.
    pub(crate) event_tx: Option<mpsc::UnboundedSender<AiEvent>>,
    pub(crate) runtime: Option<Arc<dyn GolishRuntime>>,
    /// Session ID for event routing (set for per-session bridges)
    pub(crate) event_session_id: Option<String>,

    // Event reliability - sequence tracking and buffering for frontend initialization
    /// Monotonically increasing sequence number for events (per-session)
    pub(crate) event_sequence: AtomicU64,
    /// Whether the frontend has signaled it is ready to receive events
    pub(crate) frontend_ready: AtomicBool,
    /// Buffer for events emitted before frontend signals ready
    pub(crate) event_buffer: RwLock<Vec<AiEventEnvelope>>,

    // Sub-agents
    pub(crate) sub_agent_registry: Arc<RwLock<SubAgentRegistry>>,
    pub(crate) prompt_registry: golish_sub_agents::PromptRegistry,

    // Debug: per-session API request stats (main + sub-agents)
    pub(crate) api_request_stats: Arc<ApiRequestStats>,

    // Terminal integration
    pub(crate) pty_manager: Option<Arc<PtyManager>>,
    pub(crate) current_session_id: Arc<RwLock<Option<String>>>,

    // Conversation state
    pub(crate) conversation_history: Arc<RwLock<Vec<Message>>>,

    // Session persistence
    pub(crate) session_manager: Arc<RwLock<Option<GolishSessionManager>>>,
    pub(crate) session_persistence_enabled: Arc<RwLock<bool>>,

    // HITL approval
    pub(crate) approval_recorder: Arc<ApprovalRecorder>,
    pub(crate) pending_approvals: Arc<RwLock<HashMap<String, oneshot::Sender<ApprovalDecision>>>>,

    // Cancellation flag - set by shutdown to stop the agentic loop
    pub(crate) cancelled: Arc<AtomicBool>,

    // Tool policy
    pub(crate) tool_policy_manager: Arc<ToolPolicyManager>,

    // Context management
    pub(crate) context_manager: Arc<ContextManager>,

    // Compaction state for tracking token usage
    pub(crate) compaction_state: Arc<RwLock<CompactionState>>,

    // Loop detection
    pub(crate) loop_detector: Arc<RwLock<LoopDetector>>,

    // Tool configuration
    pub(crate) tool_config: ToolConfig,

    // Agent mode (controls tool approval behavior)
    pub(crate) agent_mode: Arc<RwLock<AgentMode>>,

    // Plan manager for update_plan tool
    pub(crate) plan_manager: Arc<PlanManager>,

    // Sidecar context capture
    pub(crate) sidecar_state: Option<Arc<SidecarState>>,

    // Memory file path for project instructions (from codebase settings)
    pub(crate) memory_file_path: Arc<RwLock<Option<PathBuf>>>,

    // Settings manager for dynamic memory file lookup
    pub(crate) settings_manager: Option<Arc<golish_settings::SettingsManager>>,

    // OpenAI web search configuration (if enabled)
    pub(crate) openai_web_search_config: Option<golish_llm_providers::OpenAiWebSearchConfig>,

    // OpenAI reasoning effort level (if set)
    pub(crate) openai_reasoning_effort: Option<String>,

    // Database pool for session persistence dual-write
    pub(crate) db_pool: Option<Arc<sqlx::PgPool>>,

    // Database tracker for background recording (tool calls, tokens, logs)
    pub(crate) db_tracker: Option<crate::db_tracking::DbTracker>,

    // Factory for creating sub-agent model override clients (optional)
    pub(crate) model_factory: Option<Arc<super::llm_client::LlmClientFactory>>,

    // OpenRouter provider preferences JSON for routing and filtering (optional)
    pub(crate) openrouter_provider_preferences: Option<serde_json::Value>,

    // External services
    pub(crate) indexer_state: Option<Arc<IndexerState>>,

    // Transcript writer for persisting AI events to JSONL
    pub(crate) transcript_writer: Option<Arc<TranscriptWriter>>,

    // Base directory for transcript files (e.g., `~/.golish/transcripts`)
    // Used to create separate transcript files for sub-agent internal events.
    pub(crate) transcript_base_dir: Option<PathBuf>,

    // Skill cache for automatic skill discovery
    // Contains pre-computed SkillMetadata for efficient matching
    pub(crate) skill_cache: Arc<RwLock<Vec<SkillMetadata>>>,

    // MCP (Model Context Protocol) integration
    // Tool definitions from connected MCP servers
    pub(crate) mcp_tool_definitions: Arc<RwLock<Vec<rig::completion::ToolDefinition>>>,
    // Custom executor for MCP tool calls (RwLock for interior mutability - allows
    // updating the executor from &self when the global MCP manager changes)
    #[allow(clippy::type_complexity)]
    pub(crate) mcp_tool_executor: Arc<
        RwLock<
            Option<
                Arc<
                    dyn Fn(
                            &str,
                            &serde_json::Value,
                        ) -> std::pin::Pin<
                            Box<
                                dyn std::future::Future<Output = Option<(serde_json::Value, bool)>>
                                    + Send,
                            >,
                        > + Send
                        + Sync,
                >,
            >,
        >,
    >,

    // Event coordinator for message-passing based event management
    // When present, this replaces the atomic-based event_sequence/frontend_ready/event_buffer
    pub(crate) coordinator: Option<CoordinatorHandle>,

    // Whether sub-agent delegation is enabled (PentAGI-style useAgents toggle).
    // When false, the system prompt excludes the team delegation section,
    // limiting the AI to direct tools only.
    pub(crate) use_agents: Arc<RwLock<bool>>,

    // Execution mode: Chat (conversational) or Task (automated PentAGI-style).
    pub(crate) execution_mode: Arc<RwLock<super::execution_mode::ExecutionMode>>,
}

impl AgentBridge {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub async fn get_api_request_stats_snapshot(&self) -> ApiRequestStatsSnapshot {
        self.api_request_stats.snapshot().await
    }

    // Constructor methods are in constructors.rs

    // ========================================================================
    // Event Emission Helpers
    // ========================================================================

    /// Create an event envelope with sequence number and timestamp.
    fn create_envelope(&self, event: AiEvent) -> AiEventEnvelope {
        let seq = self.event_sequence.fetch_add(1, Ordering::SeqCst);
        let ts = chrono::Utc::now().to_rfc3339();
        AiEventEnvelope { seq, ts, event }
    }

    /// Helper to emit events through available channels.
    ///
    /// Events are wrapped in an AiEventEnvelope with sequence number and timestamp.
    /// If the frontend has not signaled ready, events are buffered instead of emitted.
    ///
    /// When a coordinator is available, events are routed through it for deterministic
    /// ordering and deadlock-free processing. Otherwise, the legacy atomic-based path
    /// is used for backward compatibility.
    ///
    /// Uses `event_session_id` for routing events to the correct frontend tab.
    pub fn emit_event(&self, event: AiEvent) {
        // If coordinator is available, use it (new path)
        if let Some(ref coordinator) = self.coordinator {
            coordinator.emit(event);
            return;
        }

        // Legacy path: write to transcript and use atomic-based buffering
        // Skip: streaming events (TextDelta/Reasoning), sub-agent internal events (go to separate file)
        if let Some(ref writer) = self.transcript_writer {
            if !matches!(
                event,
                AiEvent::TextDelta { .. }
                    | AiEvent::Reasoning { .. }
                    | AiEvent::SubAgentToolRequest { .. }
                    | AiEvent::SubAgentToolResult { .. }
            ) {
                let writer = Arc::clone(writer);
                let event_clone = event.clone();
                tokio::spawn(async move {
                    if let Err(e) = writer.append(&event_clone).await {
                        tracing::warn!("Failed to write to transcript: {}", e);
                    }
                });
            }
        }

        // Create envelope with sequence number and timestamp
        let envelope = self.create_envelope(event.clone());

        // If frontend is not ready, buffer the event
        if !self.frontend_ready.load(Ordering::SeqCst) {
            if let Ok(mut buffer) = self.event_buffer.try_write() {
                tracing::debug!(
                    message = "[emit_event] Buffering event (frontend not ready)",
                    seq = envelope.seq,
                    event_type = envelope.event.event_type(),
                );
                buffer.push(envelope);
                return;
            }
            // If we can't acquire the lock, fall through to emit directly
            // This is a rare race condition during mark_frontend_ready
            tracing::debug!(
                message = "[emit_event] Could not acquire buffer lock, emitting directly",
                seq = envelope.seq,
            );
        }

        // Emit the envelope
        self.emit_envelope(envelope, event);
    }

    /// Emit an envelope through available channels.
    ///
    /// This is separated from emit_event to allow both direct emission and buffer flushing.
    fn emit_envelope(&self, envelope: AiEventEnvelope, event: AiEvent) {
        // Emit through legacy event_tx channel if available (without envelope for backward compat)
        if let Some(ref tx) = self.event_tx {
            let _ = tx.send(event.clone());
        }

        // Emit through runtime abstraction if available
        if let Some(ref rt) = self.runtime {
            // Use stored session_id for routing, fall back to "unknown" if not set
            let session_id = self.event_session_id.clone().unwrap_or_else(|| {
                tracing::warn!(
                    message = "[emit_event] event_session_id is None! Falling back to 'unknown'",
                    event_type = ?std::mem::discriminant(&event),
                );
                "unknown".to_string()
            });
            tracing::debug!(
                message = "[emit_event] Emitting event through runtime",
                session_id = %session_id,
                seq = envelope.seq,
                event_type = envelope.event.event_type(),
            );
            // Emit the envelope (which contains the event)
            if let Err(e) = rt.emit(RuntimeEvent::AiEnvelope {
                session_id,
                envelope: Box::new(envelope),
            }) {
                tracing::warn!("Failed to emit event through runtime: {}", e);
            }
        } else {
            tracing::warn!(
                message = "[emit_event] No runtime available to emit event",
                event_type = ?std::mem::discriminant(&event),
            );
        }
    }

    /// Mark the frontend as ready to receive events.
    ///
    /// This flushes any buffered events and allows future events to be emitted directly.
    /// Should be called by the frontend after it has set up its event listeners.
    pub async fn mark_frontend_ready(&self) {
        // If coordinator is available, use it (new path)
        if let Some(ref coordinator) = self.coordinator {
            coordinator.mark_frontend_ready();
            return;
        }

        // Legacy path: use atomic-based state management
        // Take the buffer contents while holding the lock
        let buffered_events = {
            let mut buffer = self.event_buffer.write().await;
            std::mem::take(&mut *buffer)
        };

        let event_count = buffered_events.len();

        // Set the ready flag AFTER taking the buffer to avoid race conditions
        self.frontend_ready.store(true, Ordering::SeqCst);

        tracing::info!(
            message = "[mark_frontend_ready] Flushing buffered events",
            count = event_count,
        );

        // Flush buffered events in order
        for envelope in buffered_events {
            let event = envelope.event.clone();
            self.emit_envelope(envelope, event);
        }
    }

    /// Get the current event sequence number (for testing).
    ///
    /// Note: When coordinator is available, this returns 0 as the sequence
    /// is managed by the coordinator. Use `coordinator_state()` for accurate info.
    pub fn current_event_sequence(&self) -> u64 {
        self.event_sequence.load(Ordering::SeqCst)
    }

    /// Check if frontend is marked as ready (for testing).
    ///
    /// Note: When coordinator is available, this returns false as the state
    /// is managed by the coordinator. Use `coordinator_state()` for accurate info.
    pub fn is_frontend_ready(&self) -> bool {
        self.frontend_ready.load(Ordering::SeqCst)
    }

    /// Get the number of buffered events (for testing).
    ///
    /// Note: When coordinator is available, this returns 0 as the buffer
    /// is managed by the coordinator. Use `coordinator_state()` for accurate info.
    pub fn buffered_event_count(&self) -> usize {
        self.event_buffer.blocking_read().len()
    }

    /// Get the coordinator handle (if available).
    pub fn coordinator(&self) -> Option<&CoordinatorHandle> {
        self.coordinator.as_ref()
    }

    /// Query the coordinator state (for testing/debugging).
    ///
    /// Returns None if no coordinator is available or if it has shut down.
    pub async fn coordinator_state(&self) -> Option<crate::event_coordinator::CoordinatorState> {
        if let Some(ref coordinator) = self.coordinator {
            coordinator.query_state().await
        } else {
            None
        }
    }

    /// Get or create an event channel for the agentic loop.
    ///
    /// If `event_tx` is available, returns a clone of that sender.
    /// If only `runtime` is available, creates a forwarding channel that sends to runtime.
    ///
    /// This is a transition helper - once we update AgenticLoopContext to use runtime
    /// directly, this method will be removed.
    ///
    /// Uses `event_session_id` for routing events to the correct frontend tab.
    pub fn get_or_create_event_tx(&self) -> mpsc::UnboundedSender<AiEvent> {
        // If we have an event_tx, use it
        if let Some(ref tx) = self.event_tx {
            return tx.clone();
        }

        // Otherwise, create a forwarding channel to runtime
        let runtime = self.runtime.clone().expect(
            "AgentBridge must have either event_tx or runtime - this is a bug in construction",
        );

        // Use stored session_id for routing, fall back to "unknown" if not set
        let session_id = self
            .event_session_id
            .clone()
            .unwrap_or_else(|| "unknown".to_string());

        let (tx, mut rx) = mpsc::unbounded_channel::<AiEvent>();

        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let Err(e) = runtime.emit(RuntimeEvent::Ai {
                    session_id: session_id.clone(),
                    event: Box::new(event),
                }) {
                    tracing::warn!("Failed to forward event to runtime: {}", e);
                }
            }
        });

        tx
    }

    // ========================================================================
    // Execution Helper Methods (DRY extraction for execute_with_*_model)
    // ========================================================================

    /// Prepare the execution context for an agent turn.
    ///
    /// This extracts common setup code that was duplicated across all execute_with_*_model methods:
    /// 1. Build system prompt with agent mode and memory file
    /// 2. Inject session context
    /// 3. Start session for persistence
    /// 4. Record user message
    /// 5. Handle sidecar capture (start session, capture prompt)
    /// 6. Prepare initial history with user message
    /// 7. Get or create event channel
    ///
    /// Returns the system prompt, initial history, and event channel sender.
    async fn prepare_execution_context(
        &self,
        initial_prompt: &str,
    ) -> (String, Vec<Message>, mpsc::UnboundedSender<AiEvent>) {
        // Build system prompt with current agent mode and memory file
        let workspace_path = self.workspace.read().await;
        let agent_mode = *self.agent_mode.read().await;
        let memory_file_path = self.get_memory_file_path_dynamic().await;

        // Create prompt contributor registry with default contributors
        let contributors = create_default_contributors(self.sub_agent_registry.clone());
        let mut registry = PromptContributorRegistry::new();
        for contributor in contributors {
            registry.register(contributor);
        }

        // Create prompt context with provider, model, and feature flags
        let has_web_search = self
            .tool_registry
            .read()
            .await
            .available_tools()
            .iter()
            .any(|t| t.starts_with("web_"));
        let has_sub_agents = *self.use_agents.read().await;

        // Match skills against user prompt and load their bodies
        let (available_skills, matched_skills) = self.match_and_load_skills(initial_prompt).await;

        let prompt_context = PromptContext::new(&self.provider_name, &self.model_name)
            .with_web_search(has_web_search)
            .with_sub_agents(has_sub_agents)
            .with_workspace(workspace_path.display().to_string())
            .with_user_prompt(initial_prompt.to_string())
            .with_available_skills(available_skills)
            .with_matched_skills(matched_skills);

        let mut system_prompt = build_system_prompt_with_contributions(
            &workspace_path,
            agent_mode,
            memory_file_path.as_deref(),
            Some(&registry),
            Some(&prompt_context),
        );
        drop(workspace_path);

        // Inject Layer 1 session context if available
        if let Some(session_context) = self.get_session_context().await {
            if !session_context.is_empty() {
                tracing::debug!(
                    "[agent] Injecting Layer 1 session context ({} chars)",
                    session_context.len()
                );
                system_prompt.push_str("\n\n");
                system_prompt.push_str(&session_context);
            }
        }

        // Inject active execution plan status if one exists
        if let Some(plan_status) = self.plan_manager.format_for_prompt().await {
            tracing::info!(
                "[agent] Injecting active execution plan ({} chars)",
                plan_status.len()
            );
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&plan_status);
        }

        // Start session for persistence
        self.start_session().await;
        self.record_user_message(initial_prompt).await;

        // Capture user prompt in sidecar session
        // Only start a new session if one doesn't already exist (sessions span conversations)
        if let Some(ref sidecar) = self.sidecar_state {
            use golish_sidecar::events::SessionEvent;

            let session_id = if let Some(existing_id) = sidecar.current_session_id() {
                // Reuse existing session
                tracing::debug!("Reusing existing sidecar session: {}", existing_id);
                Some(existing_id)
            } else {
                // Start a new session
                match sidecar.start_session(initial_prompt) {
                    Ok(new_id) => {
                        tracing::info!("Started new sidecar session: {}", new_id);
                        Some(new_id)
                    }
                    Err(e) => {
                        tracing::warn!("Failed to start sidecar session: {}", e);
                        None
                    }
                }
            };

            // Capture the user prompt as an event (if we have a session)
            if let Some(ref sid) = session_id {
                let prompt_event = SessionEvent::user_prompt(sid.clone(), initial_prompt);
                sidecar.capture(prompt_event);

                // Store sidecar session ID in AI session manager for later restoration
                self.with_session_manager(|m| {
                    m.set_sidecar_session_id(sid.clone());
                })
                .await;
            }
        }

        // Prepare initial history with user message
        let mut history_guard = self.conversation_history.write().await;
        history_guard.push(Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: initial_prompt.to_string(),
            })),
        });
        let initial_history = history_guard.clone();
        drop(history_guard);

        // Get or create event channel for the agentic loop
        // This handles both legacy (event_tx) and new (runtime) paths
        let loop_event_tx = self.get_or_create_event_tx();

        (system_prompt, initial_history, loop_event_tx)
    }

    /// Prepare execution context with rich content (text + images).
    ///
    /// Similar to `prepare_execution_context` but accepts `Vec<UserContent>`
    /// instead of a plain string, enabling multi-modal prompts.
    async fn prepare_execution_context_with_content(
        &self,
        content: Vec<UserContent>,
        text_for_logging: &str,
    ) -> (String, Vec<Message>, mpsc::UnboundedSender<AiEvent>) {
        tracing::debug!("[prepare_context] Starting context preparation");

        // Build system prompt with current agent mode and memory file
        tracing::debug!("[prepare_context] Acquiring workspace read lock");
        let workspace_path = self.workspace.read().await;
        tracing::debug!("[prepare_context] Acquiring agent_mode read lock");
        let agent_mode = *self.agent_mode.read().await;
        let memory_file_path = self.get_memory_file_path_dynamic().await;

        // Create prompt contributor registry with default contributors
        let contributors = create_default_contributors(self.sub_agent_registry.clone());
        let mut registry = PromptContributorRegistry::new();
        for contributor in contributors {
            registry.register(contributor);
        }

        // Create prompt context with provider, model, and feature flags
        let has_web_search = self
            .tool_registry
            .read()
            .await
            .available_tools()
            .iter()
            .any(|t| t.starts_with("web_"));
        let has_sub_agents = *self.use_agents.read().await;

        // Match skills against user prompt and load their bodies
        let (available_skills, matched_skills) = self.match_and_load_skills(text_for_logging).await;

        let prompt_context = PromptContext::new(&self.provider_name, &self.model_name)
            .with_web_search(has_web_search)
            .with_sub_agents(has_sub_agents)
            .with_workspace(workspace_path.display().to_string())
            .with_user_prompt(text_for_logging.to_string())
            .with_available_skills(available_skills)
            .with_matched_skills(matched_skills);

        let mut system_prompt = build_system_prompt_with_contributions(
            &workspace_path,
            agent_mode,
            memory_file_path.as_deref(),
            Some(&registry),
            Some(&prompt_context),
        );
        drop(workspace_path);

        // Inject Layer 1 session context if available
        if let Some(session_context) = self.get_session_context().await {
            if !session_context.is_empty() {
                tracing::debug!(
                    "[agent] Injecting Layer 1 session context ({} chars)",
                    session_context.len()
                );
                system_prompt.push_str("\n\n");
                system_prompt.push_str(&session_context);
            }
        }

        // Inject active execution plan status if one exists
        if let Some(plan_status) = self.plan_manager.format_for_prompt().await {
            tracing::info!(
                "[agent] Injecting active execution plan ({} chars)",
                plan_status.len()
            );
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&plan_status);
        }

        // Start session for persistence
        self.start_session().await;
        self.record_user_message(text_for_logging).await;

        // Capture user prompt in sidecar session
        if let Some(ref sidecar) = self.sidecar_state {
            use golish_sidecar::events::SessionEvent;

            let session_id = if let Some(existing_id) = sidecar.current_session_id() {
                tracing::debug!("Reusing existing sidecar session: {}", existing_id);
                Some(existing_id)
            } else {
                match sidecar.start_session(text_for_logging) {
                    Ok(new_id) => {
                        tracing::info!("Started new sidecar session: {}", new_id);
                        Some(new_id)
                    }
                    Err(e) => {
                        tracing::warn!("Failed to start sidecar session: {}", e);
                        None
                    }
                }
            };

            if let Some(ref sid) = session_id {
                let prompt_event = SessionEvent::user_prompt(sid.clone(), text_for_logging);
                sidecar.capture(prompt_event);

                self.with_session_manager(|m| {
                    m.set_sidecar_session_id(sid.clone());
                })
                .await;
            }
        }

        // Prepare initial history with user message (rich content)
        let mut history_guard = self.conversation_history.write().await;

        // Log content parts before creating message
        let incoming_text_count = content
            .iter()
            .filter(|c| matches!(c, UserContent::Text(_)))
            .count();
        let incoming_image_count = content
            .iter()
            .filter(|c| matches!(c, UserContent::Image(_)))
            .count();
        tracing::debug!(
            "prepare_context: {} text part(s), {} image(s)",
            incoming_text_count,
            incoming_image_count
        );

        // Build the user message from content parts
        let user_content = match OneOrMany::many(content) {
            Ok(many) => {
                tracing::debug!(
                    "prepare_execution_context_with_content: Created OneOrMany with {} items",
                    many.len()
                );
                many
            }
            Err(_) => {
                // Empty content - use a placeholder text
                tracing::warn!(
                    "prepare_execution_context_with_content: Empty content, using placeholder"
                );
                OneOrMany::one(UserContent::Text(Text {
                    text: "".to_string(),
                }))
            }
        };
        let user_message = Message::User {
            content: user_content,
        };

        history_guard.push(user_message);
        let initial_history = history_guard.clone();
        drop(history_guard);

        // Get or create event channel for the agentic loop
        let loop_event_tx = self.get_or_create_event_tx();

        (system_prompt, initial_history, loop_event_tx)
    }

    /// Build the AgenticLoopContext with references to all required components.
    ///
    /// This is a helper to construct the context struct without duplication.
    async fn build_loop_context<'a>(
        &'a self,
        loop_event_tx: &'a mpsc::UnboundedSender<AiEvent>,
    ) -> AgenticLoopContext<'a> {
        AgenticLoopContext {
            event_tx: loop_event_tx,
            tool_registry: &self.tool_registry,
            sub_agent_registry: &self.sub_agent_registry,
            indexer_state: self.indexer_state.as_ref(),
            workspace: &self.workspace,
            client: &self.client,
            approval_recorder: &self.approval_recorder,
            pending_approvals: &self.pending_approvals,
            tool_policy_manager: &self.tool_policy_manager,
            context_manager: &self.context_manager,
            compaction_state: &self.compaction_state,
            loop_detector: &self.loop_detector,
            tool_config: &self.tool_config,
            sidecar_state: self.sidecar_state.as_ref(),
            runtime: self.runtime.as_ref(),
            agent_mode: &self.agent_mode,
            plan_manager: &self.plan_manager,
            api_request_stats: &self.api_request_stats,
            provider_name: &self.provider_name,
            model_name: &self.model_name,
            openai_web_search_config: self.openai_web_search_config.as_ref(),
            openai_reasoning_effort: self.openai_reasoning_effort.as_deref(),
            openrouter_provider_preferences: self.openrouter_provider_preferences.as_ref(),
            model_factory: self.model_factory.as_ref(),
            session_id: self.event_session_id.as_deref(),
            transcript_writer: self.transcript_writer.as_ref(),
            transcript_base_dir: self.transcript_base_dir.as_deref(),
            // Additional tools and custom executor are not used in the main app (only for evals)
            // UPDATE: Now used for MCP tools if available
            additional_tool_definitions: {
                let mcp_tools = self.mcp_tool_definitions.read().await;
                mcp_tools.clone()
            },
            custom_tool_executor: self.mcp_tool_executor.read().await.clone(),
            coordinator: self.coordinator.as_ref(),
            db_tracker: self.db_tracker.as_ref(),
            cancelled: Some(&self.cancelled),
            execution_monitor: None,
            execution_mode: *self.execution_mode.read().await,
        }
    }

    /// Set MCP tool definitions.
    /// Called by configure_bridge after MCP manager is initialized.
    pub async fn set_mcp_tools(&self, definitions: Vec<rig::completion::ToolDefinition>) {
        *self.mcp_tool_definitions.write().await = definitions;
    }

    /// Finalize execution after the agentic loop completes.
    ///
    /// This extracts common post-execution code that was duplicated:
    /// 1. Persist assistant response to conversation history
    /// 2. Record and save session
    /// 3. Capture AI response in sidecar session
    /// 4. Emit completion event
    ///
    /// Returns the accumulated response (passed through for convenience).
    async fn finalize_execution(
        &self,
        accumulated_response: String,
        reasoning: Option<String>,
        final_history: Vec<Message>,
        token_usage: Option<TokenUsage>,
        start_time: std::time::Instant,
    ) -> String {
        let duration_ms = start_time.elapsed().as_millis() as u64;

        // Replace conversation history with the full history from the agentic loop.
        // This is critical for OpenAI Responses API where reasoning IDs must be preserved
        // in the history for function calls to work correctly across turns.
        {
            let mut history_guard = self.conversation_history.write().await;
            *history_guard = final_history;
        }

        // Record and save session
        if !accumulated_response.is_empty() {
            self.record_assistant_message(&accumulated_response).await;
            self.save_session().await;
        }

        // Capture AI response in sidecar session
        if let Some(ref sidecar) = self.sidecar_state {
            use golish_sidecar::events::SessionEvent;

            if let Some(session_id) = sidecar.current_session_id() {
                if !accumulated_response.is_empty() {
                    let response_event =
                        SessionEvent::ai_response(session_id, &accumulated_response);
                    sidecar.capture(response_event);
                    tracing::debug!(
                        "[agent] Captured AI response in sidecar ({} chars)",
                        accumulated_response.len()
                    );
                }
            }
        }

        // Emit completion event
        self.emit_event(AiEvent::Completed {
            response: accumulated_response.clone(),
            reasoning,
            input_tokens: token_usage.as_ref().map(|u| u.input_tokens as u32),
            output_tokens: token_usage.as_ref().map(|u| u.output_tokens as u32),
            duration_ms: Some(duration_ms),
        });

        accumulated_response
    }

    /// Restore conversation history from a list of simple messages.
    /// Used when reopening an existing conversation to give the AI context.
    pub async fn restore_conversation_history(&self, messages: Vec<(String, String)>) {
        let mut history = Vec::new();
        for (role, content) in messages {
            match role.as_str() {
                "user" => {
                    history.push(Message::User {
                        content: OneOrMany::one(UserContent::Text(Text {
                            text: content,
                        })),
                    });
                }
                "assistant" => {
                    history.push(Message::Assistant {
                        id: None,
                        content: OneOrMany::one(AssistantContent::Text(Text {
                            text: content,
                        })),
                    });
                }
                _ => {
                    tracing::warn!("[restore] Unknown message role: {}", role);
                }
            }
        }
        let count = history.len();
        let mut guard = self.conversation_history.write().await;
        *guard = history;
        tracing::info!("[restore] Restored {} messages to conversation history", count);
    }

    async fn persist_terminal_error_state(&self, terminal_state: &TerminalErrorState) {
        if let Some(final_history) = terminal_state.final_history.clone() {
            let mut history_guard = self.conversation_history.write().await;
            *history_guard = final_history;
        }

        if let Some(partial_response) = terminal_state
            .partial_response
            .as_deref()
            .filter(|text| !text.is_empty())
        {
            self.record_assistant_message(partial_response).await;

            if let Some(ref sidecar) = self.sidecar_state {
                use golish_sidecar::events::SessionEvent;

                if let Some(session_id) = sidecar.current_session_id() {
                    sidecar.capture(SessionEvent::ai_response(session_id, partial_response));
                }
            }
        }

        self.save_session().await;
    }

    // ========================================================================
    // Configuration Methods
    // ========================================================================

    /// Get a clone of the database pool (if available).
    pub fn db_pool(&self) -> Option<Arc<sqlx::PgPool>> {
        self.db_pool.clone()
    }

    /// Set the database pool for session persistence dual-write and activity tracking.
    pub fn set_db_pool(
        &mut self,
        pool: Arc<sqlx::PgPool>,
        ready_gate: golish_db::DbReadyGate,
    ) {
        let session_uuid = uuid::Uuid::new_v4();
        let ws = self.workspace.try_read().ok();
        let project_path = ws
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .filter(|s| s != ".");
        self.db_tracker = Some(
            crate::db_tracking::DbTracker::new(pool.clone(), session_uuid, ready_gate.clone())
                .with_project_path(project_path),
        );
        self.db_pool = Some(pool.clone());

        // Load prompt template overrides from DB (non-blocking)
        let prompt_reg = self.prompt_registry.clone();
        let pool_for_prompts = pool.clone();
        let sub_reg = self.sub_agent_registry.clone();
        tokio::spawn(async move {
            if let Err(e) = prompt_reg.load_db_overrides(&pool_for_prompts).await {
                tracing::warn!("[prompt-registry] Failed to load DB overrides: {e}");
            } else {
                // Re-create sub-agents with updated templates
                let new_agents = golish_sub_agents::defaults::create_default_sub_agents_from_registry(&prompt_reg).await;
                let mut reg = sub_reg.write().await;
                reg.register_multiple(new_agents);
                tracing::info!("[prompt-registry] Reloaded sub-agents with DB template overrides");
            }
        });

        // Wire up PlanManager with DB persistence
        let ws = self.workspace.try_read().ok();
        let plan_project_path = ws
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .filter(|s| s != ".");
        self.plan_manager = Arc::new(
            PlanManager::new().with_db(pool.clone(), Some(session_uuid), plan_project_path)
        );

        let plan_manager = self.plan_manager.clone();
        let pool_for_session = pool.clone();
        let mut gate = ready_gate;
        tokio::spawn(async move {
            if !gate.is_ready() {
                if tokio::time::timeout(std::time::Duration::from_secs(60), gate.wait())
                    .await
                    .is_err()
                {
                    return;
                }
            }
            let _ = sqlx::query(
                "INSERT INTO sessions (id) VALUES ($1) ON CONFLICT DO NOTHING",
            )
            .bind(session_uuid)
            .execute(pool_for_session.as_ref())
            .await;

            // Load any active plan from the previous session
            plan_manager.load_from_db().await;
        });
    }

    /// Set the PtyManager for executing commands in user's terminal
    pub fn set_pty_manager(&mut self, pty_manager: Arc<PtyManager>) {
        self.pty_manager = Some(pty_manager);
    }

    /// Set the IndexerState for code analysis tools
    pub fn set_indexer_state(&mut self, indexer_state: Arc<IndexerState>) {
        self.indexer_state = Some(indexer_state);
    }

    /// Set the SidecarState for context capture
    pub fn set_sidecar_state(&mut self, sidecar_state: Arc<SidecarState>) {
        self.sidecar_state = Some(sidecar_state);
    }

    /// Set the TranscriptWriter for persisting AI events to JSONL.
    pub fn set_transcript_writer(&mut self, writer: TranscriptWriter, base_dir: PathBuf) {
        let writer = Arc::new(writer);
        // Forward to coordinator so bridge-level events (UserMessage, Completed, etc.)
        // are also written to the transcript
        if let Some(ref coordinator) = self.coordinator {
            coordinator.set_transcript_writer(Arc::clone(&writer));
        }
        self.transcript_writer = Some(writer);
        self.transcript_base_dir = Some(base_dir);
    }

    /// Set the memory file path for project instructions.
    /// This overrides the default CLAUDE.md lookup.
    pub async fn set_memory_file_path(&self, path: Option<PathBuf>) {
        *self.memory_file_path.write().await = path;
    }

    /// Set the SettingsManager for dynamic memory file lookup.
    pub fn set_settings_manager(&mut self, settings_manager: Arc<golish_settings::SettingsManager>) {
        self.settings_manager = Some(settings_manager);
    }

    /// Get the memory file path dynamically from current settings.
    /// This ensures we always use the latest settings, even if they changed
    /// after the AI session was initialized.
    /// Falls back to cached value if settings_manager is not available.
    async fn get_memory_file_path_dynamic(&self) -> Option<PathBuf> {
        // Try dynamic lookup if settings_manager is available (tauri only)

        if let Some(ref settings_manager) = self.settings_manager {
            let workspace_path = self.workspace.read().await;
            let settings = settings_manager.get().await;
            if let Some(path) = crate::memory_file::find_memory_file_for_workspace(
                &workspace_path,
                &settings.codebases,
            ) {
                return Some(path);
            }
        }

        // Fall back to cached value
        self.memory_file_path.read().await.clone()
    }

    /// Set the current session ID for terminal execution
    pub async fn set_session_id(&self, session_id: Option<String>) {
        *self.current_session_id.write().await = session_id;
    }

    /// Update the workspace/working directory.
    /// This also updates the tool registry's workspace so file operations
    /// use the new directory as the base for relative paths.
    pub async fn set_workspace(&self, new_workspace: PathBuf) {
        // Check if workspace actually changed
        {
            let current = self.workspace.read().await;
            if *current == new_workspace {
                tracing::trace!(
                    "[cwd-sync] Workspace unchanged, skipping update: {}",
                    new_workspace.display()
                );
                return;
            }
        }

        // Update bridge workspace
        {
            let mut workspace = self.workspace.write().await;
            *workspace = new_workspace.clone();
        } // Drop workspace write lock before doing anything else

        // Also update the tool registry's workspace so file operations
        // resolve relative paths against the new directory
        {
            let mut registry = self.tool_registry.write().await;
            registry.set_workspace(new_workspace.clone());
        }

        // Also update the session manager's workspace so sessions capture the correct path
        self.update_session_workspace(new_workspace.clone()).await;

        tracing::debug!(
            "[cwd-sync] Updated workspace to: {}",
            new_workspace.display()
        );

        // Refresh skill cache for new workspace
        // NOTE: Must be called after dropping workspace write lock, as refresh_skills
        // acquires workspace read lock internally
        self.refresh_skills().await;
    }

    /// Refresh the skill cache for the current workspace.
    ///
    /// This discovers skills from both global (~/.golish/skills/) and local
    /// (<workspace>/.golish/skills/) directories and caches their metadata
    /// for efficient matching.
    pub async fn refresh_skills(&self) {
        let workspace = self.workspace.read().await;
        let workspace_str = workspace.to_string_lossy().to_string();
        drop(workspace);

        // Run discover_skills in a blocking thread to avoid blocking the tokio runtime.
        // This is important because discover_skills scans directories synchronously.
        let workspace_str_clone = workspace_str.clone();
        let skills = match tokio::task::spawn_blocking(move || {
            golish_skills::discover_skills(Some(&workspace_str_clone))
        })
        .await
        {
            Ok(skills) => skills,
            Err(e) => {
                tracing::warn!("[refresh_skills] Failed to discover skills: {}", e);
                return;
            }
        };

        let metadata: Vec<SkillMetadata> = skills.into_iter().map(Into::into).collect();

        *self.skill_cache.write().await = metadata.clone();
        tracing::debug!(
            "[skills] Refreshed skill cache: {} skills discovered",
            metadata.len()
        );
    }

    /// Match skills against a user prompt and load their bodies.
    ///
    /// This is the progressive loading implementation:
    /// 1. Uses cached skill metadata for efficient matching
    /// 2. Only loads full skill bodies for matched skills
    ///
    /// Returns (available_skills, matched_skills) for PromptContext.
    async fn match_and_load_skills(
        &self,
        prompt: &str,
    ) -> (Vec<PromptSkillInfo>, Vec<PromptMatchedSkill>) {
        let skill_cache = self.skill_cache.read().await;

        if skill_cache.is_empty() {
            return (Vec::new(), Vec::new());
        }

        // Convert cached metadata to PromptSkillInfo for summary
        let available_skills: Vec<PromptSkillInfo> = skill_cache
            .iter()
            .map(|s| PromptSkillInfo {
                name: s.name.clone(),
                description: s.description.clone(),
            })
            .collect();

        // Match skills against prompt
        let matcher = golish_skills::SkillMatcher::default();
        let matches = matcher.match_skills(prompt, &skill_cache);

        if matches.is_empty() {
            tracing::debug!("[skills] No skills matched for prompt");
            return (available_skills, Vec::new());
        }

        tracing::debug!(
            "[skills] {} skills matched for prompt: {:?}",
            matches.len(),
            matches.iter().map(|(s, _, _)| &s.name).collect::<Vec<_>>()
        );

        // Load full bodies for matched skills (progressive loading)
        let mut matched_skills = Vec::new();
        for (meta, score, reason) in matches {
            match golish_skills::load_skill_body(&meta.path) {
                Ok(body) => {
                    matched_skills.push(PromptMatchedSkill {
                        name: meta.name.clone(),
                        description: meta.description.clone(),
                        body,
                        match_score: score,
                        match_reason: reason,
                    });
                }
                Err(e) => {
                    tracing::warn!("[skills] Failed to load body for '{}': {}", meta.name, e);
                }
            }
        }

        (available_skills, matched_skills)
    }

    /// Set the agent mode.
    /// This controls how tool approvals are handled.
    pub async fn set_agent_mode(&self, mode: AgentMode) {
        let mut current = self.agent_mode.write().await;
        tracing::debug!("Agent mode changed: {} -> {}", *current, mode);
        *current = mode;
    }

    /// Get the current agent mode.
    pub async fn get_agent_mode(&self) -> AgentMode {
        *self.agent_mode.read().await
    }

    /// Set the useAgents flag (controls whether sub-agent delegation is available).
    pub async fn set_use_agents(&self, enabled: bool) {
        let mut current = self.use_agents.write().await;
        tracing::debug!("useAgents changed: {} -> {}", *current, enabled);
        *current = enabled;
    }

    /// Get the current useAgents setting.
    pub async fn get_use_agents(&self) -> bool {
        *self.use_agents.read().await
    }

    /// Set the execution mode (Chat vs Task).
    pub async fn set_execution_mode(&self, mode: super::execution_mode::ExecutionMode) {
        let mut current = self.execution_mode.write().await;
        tracing::debug!("Execution mode changed: {} -> {}", *current, mode);
        *current = mode;
    }

    /// Get the current execution mode.
    pub async fn get_execution_mode(&self) -> super::execution_mode::ExecutionMode {
        *self.execution_mode.read().await
    }

    // ========================================================================
    // System Prompt Methods
    // ========================================================================

    /// Build the system prompt for the agent.
    ///
    /// This is a simplified version of the prompt building logic from
    /// `prepare_execution_context`.
    pub async fn build_system_prompt(&self) -> String {
        use super::system_prompt::build_system_prompt_with_contributions;

        let workspace_path = self.workspace.read().await;
        let agent_mode = *self.agent_mode.read().await;
        let memory_file_path = self.get_memory_file_path_dynamic().await;

        build_system_prompt_with_contributions(
            &workspace_path,
            agent_mode,
            memory_file_path.as_deref(),
            None, // No prompt contributors for base prompt
            None, // No prompt context for base prompt
        )
    }

    // ========================================================================
    // Public Accessors (for golish crate)
    // ========================================================================

    /// Get the sub-agent registry.
    pub fn sub_agent_registry(&self) -> &Arc<RwLock<SubAgentRegistry>> {
        &self.sub_agent_registry
    }

    /// Get the prompt template registry.
    pub fn prompt_registry(&self) -> &golish_sub_agents::PromptRegistry {
        &self.prompt_registry
    }

    /// Get the provider name.
    pub fn provider_name(&self) -> &str {
        &self.provider_name
    }

    /// Get the model name.
    pub fn model_name(&self) -> &str {
        &self.model_name
    }

    /// Get the plan manager.
    pub fn plan_manager(&self) -> &Arc<PlanManager> {
        &self.plan_manager
    }

    /// Get the LLM client.
    pub fn client(&self) -> &Arc<RwLock<LlmClient>> {
        &self.client
    }

    /// Get the tool registry.
    pub fn tool_registry(&self) -> &Arc<RwLock<ToolRegistry>> {
        &self.tool_registry
    }

    /// Get the workspace path.
    pub fn workspace(&self) -> &Arc<RwLock<PathBuf>> {
        &self.workspace
    }

    /// Get the indexer state.
    pub fn indexer_state(&self) -> Option<&Arc<IndexerState>> {
        self.indexer_state.as_ref()
    }

    /// Get the model factory (for sub-agent model overrides).
    pub fn model_factory(&self) -> Option<&Arc<super::llm_client::LlmClientFactory>> {
        self.model_factory.as_ref()
    }

    /// Set the model factory for sub-agent model overrides.
    pub fn set_model_factory(&mut self, factory: Arc<super::llm_client::LlmClientFactory>) {
        self.model_factory = Some(factory);
    }

    pub fn event_session_id(&self) -> Option<&str> {
        self.event_session_id.as_deref()
    }

    pub fn transcript_base_dir(&self) -> Option<&std::path::Path> {
        self.transcript_base_dir.as_deref()
    }

    pub fn api_request_stats(&self) -> &Arc<ApiRequestStats> {
        &self.api_request_stats
    }

    /// Get the current MCP tool definitions.
    /// Returns a clone of the tool definitions for external inspection.
    pub async fn mcp_tool_definitions(&self) -> Vec<rig::completion::ToolDefinition> {
        self.mcp_tool_definitions.read().await.clone()
    }

    /// Set MCP tool executor for handling MCP tool calls.
    /// This should be called together with `set_mcp_tools`.
    /// Takes `&self` (uses interior mutability) so it can be called after bridge creation.
    #[allow(clippy::type_complexity)]
    pub async fn set_mcp_executor(
        &self,
        executor: Arc<
            dyn Fn(
                    &str,
                    &serde_json::Value,
                ) -> std::pin::Pin<
                    Box<dyn std::future::Future<Output = Option<(serde_json::Value, bool)>> + Send>,
                > + Send
                + Sync,
        >,
    ) {
        *self.mcp_tool_executor.write().await = Some(executor);
    }
}

// ============================================================================
// Drop Implementation for Session Cleanup
// ============================================================================

impl Drop for AgentBridge {
    fn drop(&mut self) {
        // Best-effort session finalization on drop.
        // This ensures sessions are saved even if the bridge is replaced without
        // explicit finalization (e.g., during model switching).
        //
        // We use try_write() because:
        // 1. Drop cannot be async, so we can't use .await
        // 2. If the lock is held, another operation is in progress and will handle cleanup
        // 3. At drop time, we should typically be the only owner
        if let Ok(mut guard) = self.session_manager.try_write() {
            if let Some(ref mut manager) = guard.take() {
                match manager.finalize() {
                    Ok(path) => {
                        tracing::debug!(
                            "AgentBridge::drop - session finalized: {}",
                            path.display()
                        );
                    }
                    Err(e) => {
                        tracing::warn!("AgentBridge::drop - failed to finalize session: {}", e);
                    }
                }
            }
        } else {
            tracing::debug!(
                "AgentBridge::drop - could not acquire session_manager lock, skipping finalization"
            );
        }

        // End sidecar session on bridge drop.
        // This ensures the sidecar session is properly finalized when:
        // - The conversation is cleared
        // - The AgentBridge is replaced (e.g., model switching)
        // - The application shuts down
        if let Some(ref sidecar) = self.sidecar_state {
            match sidecar.end_session() {
                Ok(Some(session)) => {
                    tracing::debug!(
                        "AgentBridge::drop - sidecar session {} ended",
                        session.session_id
                    );
                }
                Ok(None) => {
                    tracing::debug!("AgentBridge::drop - no active sidecar session to end");
                }
                Err(e) => {
                    tracing::warn!("AgentBridge::drop - failed to end sidecar session: {}", e);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_emit_execution_error_for_normal_errors() {
        let err = anyhow::anyhow!("regular execution failure");
        assert!(should_emit_execution_error_event(&err));
    }

    #[test]
    fn should_not_emit_execution_error_for_terminal_error_marker() {
        let err = anyhow::Error::new(TerminalErrorEmitted::new("already emitted"));
        assert!(!should_emit_execution_error_event(&err));
    }

    #[test]
    fn extract_terminal_error_state_returns_none_for_non_terminal_error() {
        let err = anyhow::anyhow!("regular execution failure");
        assert!(extract_terminal_error_state(&err).is_none());
    }

    #[test]
    fn extract_terminal_error_state_returns_partial_response_and_history() {
        let history = vec![Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: "hello".to_string(),
            })),
        }];

        let err = anyhow::Error::new(TerminalErrorEmitted::with_partial_state(
            "stream failed",
            Some("partial assistant text".to_string()),
            Some(history),
        ));

        let state = extract_terminal_error_state(&err).expect("expected terminal error state");
        assert_eq!(
            state.partial_response.as_deref(),
            Some("partial assistant text")
        );
        assert_eq!(state.final_history.as_ref().map(Vec::len), Some(1));
    }
}
