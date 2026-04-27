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
//! # Decomposition
//!
//! `AgentBridge` is composed of focused subsystems to keep each concern isolated:
//!
//! - [`BridgeEventBus`] — Event emission, sequence numbering, frontend-ready
//!   buffering, coordinator, and transcript writing.
//! - [`BridgeLlmConfig`] — LLM client, provider/model identifiers, and
//!   provider-specific configuration (web search, reasoning effort, etc.).
//! - [`BridgeServices`] — Optional external service handles (DB, PTY, sidecar,
//!   indexer, settings manager).
//! - [`BridgeAccessControl`] — Tool policy, HITL approval, agent mode, and
//!   loop detection.
//! - [`BridgeSession`] — Conversation history, session persistence manager.
//!
//! The remaining top-level fields represent cross-cutting identity and
//! orchestration state (workspace, tool registry, sub-agents, context, MCP).
//!
//! # Impl blocks
//!
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

// ============================================================================
// Composed Subsystems
// ============================================================================

/// Event emission, sequencing, buffering, and transcript writing.
pub(crate) struct BridgeEventBus {
    /// Legacy event channel (being phased out in favour of `runtime`).
    pub(crate) event_tx: Option<mpsc::UnboundedSender<AiEvent>>,
    /// New runtime abstraction for event emission.
    pub(crate) runtime: Option<Arc<dyn GolishRuntime>>,
    /// Session ID for routing events to the correct frontend tab.
    pub(crate) event_session_id: Option<String>,
    /// Monotonically increasing sequence number (per-session).
    pub(crate) event_sequence: AtomicU64,
    /// Whether the frontend has signaled it is ready to receive events.
    pub(crate) frontend_ready: AtomicBool,
    /// Buffer for events emitted before frontend signals ready.
    pub(crate) event_buffer: RwLock<Vec<AiEventEnvelope>>,
    /// Message-passing coordinator (replaces the atomic-based path when present).
    pub(crate) coordinator: Option<CoordinatorHandle>,
    /// Transcript writer for persisting AI events to JSONL.
    pub(crate) transcript_writer: Option<Arc<TranscriptWriter>>,
    /// Base directory for transcript files (sub-agent internal events go here).
    pub(crate) transcript_base_dir: Option<PathBuf>,
}

/// LLM client handle and provider-specific configuration.
pub(crate) struct BridgeLlmConfig {
    pub(crate) client: Arc<RwLock<LlmClient>>,
    pub(crate) provider_name: String,
    pub(crate) model_name: String,
    /// Factory for creating sub-agent model override clients.
    pub(crate) model_factory: Option<Arc<super::llm_client::LlmClientFactory>>,
    pub(crate) openai_web_search_config: Option<golish_llm_providers::OpenAiWebSearchConfig>,
    pub(crate) openai_reasoning_effort: Option<String>,
    pub(crate) openrouter_provider_preferences: Option<serde_json::Value>,
}

/// Optional external service handles wired in after construction.
pub(crate) struct BridgeServices {
    pub(crate) db_pool: Option<Arc<sqlx::PgPool>>,
    pub(crate) db_tracker: Option<crate::db_tracking::DbTracker>,
    pub(crate) indexer_state: Option<Arc<IndexerState>>,
    pub(crate) sidecar_state: Option<Arc<SidecarState>>,
    pub(crate) settings_manager: Option<Arc<golish_settings::SettingsManager>>,
    pub(crate) pty_manager: Option<Arc<PtyManager>>,
}

/// Tool access control: policy engine, HITL approval, agent mode, loop detection.
pub(crate) struct BridgeAccessControl {
    pub(crate) approval_recorder: Arc<ApprovalRecorder>,
    pub(crate) pending_approvals: Arc<RwLock<HashMap<String, oneshot::Sender<ApprovalDecision>>>>,
    pub(crate) tool_policy_manager: Arc<ToolPolicyManager>,
    pub(crate) agent_mode: Arc<RwLock<AgentMode>>,
    pub(crate) loop_detector: Arc<RwLock<LoopDetector>>,
}

/// Conversation history and session persistence.
pub(crate) struct BridgeSession {
    pub(crate) conversation_history: Arc<RwLock<Vec<Message>>>,
    pub(crate) session_manager: Arc<RwLock<Option<GolishSessionManager>>>,
    pub(crate) session_persistence_enabled: Arc<RwLock<bool>>,
}

// ============================================================================
// AgentBridge
// ============================================================================

/// Bridge between Golish and LLM providers.
/// Handles LLM streaming and tool execution.
pub struct AgentBridge {
    // -- Composed subsystems --------------------------------------------------
    pub(crate) events: BridgeEventBus,
    pub(crate) llm: BridgeLlmConfig,
    pub(crate) services: BridgeServices,
    pub(crate) access: BridgeAccessControl,
    pub(crate) session: BridgeSession,

    // -- Cross-cutting identity & orchestration -------------------------------
    pub(crate) workspace: Arc<RwLock<PathBuf>>,
    pub(crate) tool_registry: Arc<RwLock<ToolRegistry>>,
    pub(crate) tool_config: ToolConfig,
    pub(crate) cancelled: Arc<AtomicBool>,
    pub(crate) api_request_stats: Arc<ApiRequestStats>,

    // -- Sub-agents -----------------------------------------------------------
    pub(crate) sub_agent_registry: Arc<RwLock<SubAgentRegistry>>,
    pub(crate) prompt_registry: golish_sub_agents::PromptRegistry,
    pub(crate) use_agents: Arc<RwLock<bool>>,
    pub(crate) execution_mode: Arc<RwLock<super::execution_mode::ExecutionMode>>,

    // -- Context / planning ---------------------------------------------------
    pub(crate) context_manager: Arc<ContextManager>,
    pub(crate) compaction_state: Arc<RwLock<CompactionState>>,
    pub(crate) plan_manager: Arc<PlanManager>,
    pub(crate) current_session_id: Arc<RwLock<Option<String>>>,
    pub(crate) memory_file_path: Arc<RwLock<Option<PathBuf>>>,

    // -- Skills & MCP ---------------------------------------------------------
    pub(crate) skill_cache: Arc<RwLock<Vec<SkillMetadata>>>,
    pub(crate) mcp_tool_definitions: Arc<RwLock<Vec<rig::completion::ToolDefinition>>>,
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
        if let Ok(mut guard) = self.session.session_manager.try_write() {
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

        if let Some(ref sidecar) = self.services.sidecar_state {
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
