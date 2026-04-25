//! Agent bridge for LLM interaction.
//!
//! This module provides the main [`AgentBridge`] struct that orchestrates:
//! - LLM communication (vtcode-core and Vertex AI Anthropic)
//! - Tool execution with HITL approval
//! - Conversation history management
//! - Session persistence
//! - Context window management
//! - Loop detection
//!
//! The implementation is split across many submodules:
//! - [`constructors`] - Bridge construction (`new`, `with_*` builders).
//! - [`execution`] - The `execute_with_*_model` entry points and shared
//!   execution helpers.
//! - [`events`] - Event emission, sequence numbering, frontend-ready buffering.
//! - [`prepare`] - Per-turn context prep + finalization (system prompt build,
//!   session start, history seeding, completion event emission).
//! - [`config`] - Setters/accessors for optional services (DB, PTY, sidecar,
//!   transcript, settings, ...) plus skill discovery and mode toggles.
//! - [`terminal_error`] - Helpers for propagating partial state via
//!   `TerminalErrorEmitted`.
//!
//! The crate-level `bridge_*.rs` modules contain additional `impl AgentBridge`
//! blocks (sessions, HITL, policy, context window) that pre-date this directory
//! split and are still mounted in `lib.rs`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use rig::completion::Message;
use tokio::sync::{mpsc, oneshot, RwLock};

use golish_tools::ToolRegistry;

use crate::hitl::ApprovalRecorder;
use golish_core::events::{AiEvent, AiEventEnvelope};
use golish_core::hitl::ApprovalDecision;
use golish_core::{ApiRequestStats, ApiRequestStatsSnapshot};

use super::agent_mode::AgentMode;
use super::llm_client::LlmClient;
use super::tool_definitions::ToolConfig;
use crate::loop_detection::LoopDetector;
use crate::tool_policy::ToolPolicyManager;
use golish_context::{CompactionState, ContextManager};
use golish_core::runtime::GolishRuntime;
use golish_session::GolishSessionManager;
use golish_sub_agents::SubAgentRegistry;

use crate::indexer::IndexerState;
use crate::planner::PlanManager;

use golish_pty::PtyManager;
use golish_sidecar::SidecarState;
use golish_skills::SkillMetadata;

use crate::event_coordinator::CoordinatorHandle;
use crate::transcript::TranscriptWriter;

mod config;
mod constructors;
mod events;
mod execution;
mod prepare;
mod terminal_error;

/// Bridge between Golish and LLM providers.
/// Handles LLM streaming and tool execution.
pub struct AgentBridge {
    // Core fields
    pub(crate) workspace: Arc<RwLock<PathBuf>>,
    pub(crate) provider_name: String,
    pub(crate) model_name: String,
    pub(crate) tool_registry: Arc<RwLock<ToolRegistry>>,
    pub(crate) client: Arc<RwLock<LlmClient>>,

    // Event emission - dual mode during transition.
    // The event_tx channel is the legacy path, runtime is the new abstraction.
    // During transition, emit_event() sends through BOTH to verify parity.
    pub(crate) event_tx: Option<mpsc::UnboundedSender<AiEvent>>,
    pub(crate) runtime: Option<Arc<dyn GolishRuntime>>,
    /// Session ID for event routing (set for per-session bridges)
    pub(crate) event_session_id: Option<String>,

    // Event reliability - sequence tracking and buffering for frontend init.
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

    // Base directory for transcript files (e.g., `~/.golish/transcripts`).
    // Used to create separate transcript files for sub-agent internal events.
    pub(crate) transcript_base_dir: Option<PathBuf>,

    // Skill cache for automatic skill discovery.
    // Contains pre-computed SkillMetadata for efficient matching.
    pub(crate) skill_cache: Arc<RwLock<Vec<SkillMetadata>>>,

    // MCP (Model Context Protocol) integration.
    // Tool definitions from connected MCP servers.
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

    // Event coordinator for message-passing based event management.
    // When present, this replaces the atomic-based event_sequence/frontend_ready/event_buffer.
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

    pub fn reset_cancelled(&self) {
        self.cancelled.store(false, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub async fn get_api_request_stats_snapshot(&self) -> ApiRequestStatsSnapshot {
        self.api_request_stats.snapshot().await
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
