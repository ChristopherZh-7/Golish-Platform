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
use super::tool_definitions::ToolSelectionConfig;
use crate::loop_detection::LoopDetector;
use crate::tool_policy::ToolPolicyManager;
use golish_context::{CompactionState, ContextManager};
use golish_core::runtime::GolishRuntime;
use golish_core::SessionManager;
use golish_sub_agents::SubAgentRegistry;

use crate::planner::PlanManager;
use golish_indexer::IndexerState;

use golish_core::SkillMetadata;

use crate::sidecar_trait::SessionCaptureBackend;
use crate::tool_executors::graph_trait::GraphKnowledgeBase;

use crate::event_coordinator::CoordinatorHandle;
use crate::transcript::TranscriptWriter;

mod backends;
mod config;
mod constructors;
mod events;
mod execution;
mod failover;
mod prepare;
mod task_request;
mod terminal_error;

pub use backends::BridgeBackends;
pub use task_request::{
    SessionRequestBusy, SessionRequestSlot, SessionRequestTransitionLease, TopLevelRequestLease,
};

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
    /// Per-(provider, model) user override sourced from settings / the chat
    /// model settings popover. Owns the value here; `LoopLlmRefs` borrows.
    pub(crate) model_override: Option<golish_settings::schema::ModelOverride>,
}

/// Optional external service handles wired in after construction.
pub(crate) struct BridgeServices {
    pub(crate) db_tracker: Option<crate::db_tracking::DbTracker>,
    pub(crate) chain_persistence: Option<Arc<dyn golish_sub_agents::SubAgentChainPersistence>>,
    pub(crate) indexer_state: Option<Arc<IndexerState>>,
    pub(crate) sidecar_state: Option<Arc<dyn SessionCaptureBackend>>,
    pub(crate) graph_backend: Option<Arc<dyn GraphKnowledgeBase>>,
    pub(crate) settings_manager: Option<Arc<golish_settings::SettingsManager>>,
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
    pub(crate) session_manager: Arc<RwLock<Option<Box<dyn SessionManager>>>>,
    pub(crate) session_persistence_enabled: Arc<RwLock<bool>>,
    /// Notes about background jobs that finished *between* turns (see
    /// `golish-app-core/background_jobs.rs`). Pushed by the per-session
    /// completion listener and drained into the system prompt on the next turn,
    /// so the agent learns the outcome of a job it had moved to the background.
    pub(crate) pending_background: Arc<std::sync::Mutex<Vec<String>>>,
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

    /// Durable-history backup installed synchronously before isolated Task
    /// execution crosses an await. Abort/panic leaves it here; the next owner
    /// restores it before any new execution.
    pub(crate) isolated_history_recovery: Arc<std::sync::Mutex<Option<Vec<Message>>>>,

    // -- Cross-cutting identity & orchestration -------------------------------
    pub(crate) workspace: Arc<RwLock<PathBuf>>,
    pub(crate) tool_registry: Arc<RwLock<ToolRegistry>>,
    pub(crate) tool_config: ToolSelectionConfig,
    pub(crate) cancelled: Arc<AtomicBool>,
    /// Monotonic cancellation epoch. A request samples this before acquiring
    /// the top-level slot, then only clears the boolean flag if no Stop raced
    /// with acquisition. This closes the CAS-success -> reset gap without
    /// letting a busy contender clear the active owner's cancellation.
    pub(crate) cancel_epoch: AtomicU64,
    /// Per-generation retirement signal for host-owned background listeners.
    /// A normal generation Stop must not close these listeners; bridge
    /// replacement/shutdown and Drop do.
    background_listener_retired: tokio::sync::watch::Sender<bool>,
    background_listeners_started: AtomicBool,
    pub(crate) api_request_stats: Arc<ApiRequestStats>,

    // -- Sub-agents -----------------------------------------------------------
    pub(crate) sub_agent_registry: Arc<RwLock<SubAgentRegistry>>,
    pub(crate) prompt_registry: golish_sub_agents::PromptRegistry,
    pub(crate) execution_mode: Arc<RwLock<super::execution_mode::ExecutionMode>>,
    /// Per-mode tool exposure registry shared across every loop turn.
    /// `Default` registers built-in `chat` + `task` policies; downstream
    /// crates that want extra modes can replace this via the
    /// `with_execution_mode_registry` builder.
    pub(crate) execution_mode_registry:
        Arc<golish_agent_runtime::execution_mode::ExecutionModeRegistry>,

    // -- Context / planning ---------------------------------------------------
    pub(crate) context_manager: Arc<ContextManager>,
    pub(crate) compaction_state: Arc<RwLock<CompactionState>>,
    pub(crate) plan_manager: Arc<PlanManager>,
    pub(crate) current_session_id: Arc<RwLock<Option<String>>>,
    pub(crate) memory_file_path: Arc<RwLock<Option<PathBuf>>>,

    // -- Domain hooks (injected by the host crate) ----------------------------
    pub(crate) post_shell_hook: Option<crate::agentic_loop::PostShellHook>,
    pub(crate) output_classifier: Option<crate::agentic_loop::OutputClassifier>,
    pub(crate) web_fetcher: Option<std::sync::Arc<dyn golish_core::WebFetchProvider>>,
    pub(crate) skill_provider: Option<std::sync::Arc<dyn golish_core::SkillProvider>>,
    pub(crate) session_factory: Option<std::sync::Arc<dyn golish_core::SessionManagerFactory>>,

    // -- Skills & MCP ---------------------------------------------------------
    pub(crate) skill_cache: Arc<RwLock<Vec<SkillMetadata>>>,
    pub(crate) mcp_tool_definitions: Arc<RwLock<Vec<rig::completion::ToolDefinition>>>,
    pub(crate) mcp_tool_executor:
        Arc<RwLock<Option<Arc<dyn crate::agentic_loop::McpToolExecutor>>>>,

    // -- Operation Harness (C3) ----------------------------------------------
    /// Active harness stage side-channel. Set per-subtask by the Task-mode
    /// executor (`BridgeAgentExecutor::execute_subtask`) before running the loop;
    /// read by `build_loop_context` so the agentic loop's per-tool dispatch can
    /// enforce the stage forbidden-tool barrier. `None` = no stage (flag off /
    /// non-stage turn / chat mode).
    pub(crate) harness_active_stage: Arc<RwLock<Option<golish_agent_kit::harness::StageKind>>>,
    /// Active harness authorization context (profile ceiling + classified
    /// intent). Set per-subtask alongside `harness_active_stage`; read by
    /// `build_loop_context` so per-tool dispatch can run the full pre-action
    /// authorizer (allowed_tools confinement + intent vs ceiling) on real
    /// executor tools. `None` = no stage (flag off / non-stage turn / chat mode).
    pub(crate) harness_active_authz: Arc<RwLock<Option<golish_agent_kit::harness::HarnessAuthz>>>,
    /// 设计 2026-06-11 (weak-model-submit-channel) · `true` only while running a
    /// targeted gate-repair pass whose sole remaining action is the stage
    /// submission. Set per-subtask alongside `harness_active_stage`; read by
    /// `build_loop_context` so the turn's `tool_choice` can be locked to
    /// `submit_stage_deliverable`. `false` = normal pass.
    pub(crate) harness_submit_only: Arc<RwLock<bool>>,
    /// One-shot forced tool for deterministic harness continuations. Set
    /// per-subtask alongside `harness_active_stage`; read by `build_loop_context`
    /// so the runtime can force the first turn to a known orchestration tool
    /// such as `stage_run`. `None` = normal provider/tool-choice behavior.
    pub(crate) harness_forced_tool: Arc<RwLock<Option<String>>>,
    /// Engagement-org isolation (设计 2026-06-15-engagement-org-isolation): the
    /// scoping-confirmed root organization id of the active operation. Set
    /// per-subtask alongside `harness_active_stage`; read by `build_loop_context`
    /// so fan-out / in-scope reads confine to this org's subtree (root + subs).
    /// `None` = no bound org (legacy whole-DB axis / chat / pre-scoping turn).
    pub(crate) harness_active_org_id: Arc<RwLock<Option<uuid::Uuid>>>,
    /// Active harness operation id (Task id for graph-flow task mode). Set
    /// per-subtask so loop tools that need operation-scoped state (for example
    /// `stage_run` worker resume) can update the same `operation_state` row as the
    /// graph checkpointer.
    pub(crate) harness_active_operation_id: Arc<RwLock<Option<uuid::Uuid>>>,
    /// Circuit breaker shared by every Primary loop/reflector pass in one
    /// top-level Task request. `BridgeAgentExecutor` resets it only after the
    /// universal request owner upgrades into Task execution; `stage_run` closes
    /// it per stage on exhaustion.
    pub(crate) stage_run_reentry_guard: Arc<crate::agentic_loop::StageRunReentryGuard>,
    /// Fail-fast single-flight boundary for every top-level request on this
    /// bridge (GUI text/attachments, ordinary CLI, and headless stage-run).
    /// Task/profile lead handoff and `BridgeAgentExecutor` share one RAII token.
    pub(crate) session_request_slot: Arc<SessionRequestSlot>,
    /// Exact generation this bridge is allowed to serve. GUI bridge
    /// replacement advances the stable session slot; standalone bridges use 1.
    pub(crate) session_request_generation: u64,

    /// Per-session selected harness operation profile id (e.g. "assessment" /
    /// "red_team"), chosen via the chat-panel mode picker. `None` = chat mode /
    /// no profile selected; a Task run with `None` falls back to the
    /// `GOLISH_HARNESS_PROFILE` env default.
    pub(crate) harness_profile: Arc<RwLock<Option<String>>>,

    /// C2c · StageDeliverable captured from a delegated sub-agent (e.g.
    /// `reporter`) during the active subtask. The Primary orchestrator often
    /// narrates instead of inlining the `StageDeliverable` JSON in its final
    /// message (it delegates production to `sub_agent_reporter`), so the gate —
    /// which parses only the orchestrator's content — would never see it. The
    /// agentic loop stashes any sub-agent result that carries a deliverable
    /// signature here; `BridgeAgentExecutor::execute_subtask` appends it to the
    /// content before the gate runs. Reset per-subtask. `None` = none captured.
    pub(crate) harness_last_deliverable: Arc<RwLock<Option<String>>>,
    /// Lead-agent decision side-channel: the `start_operation` tool writes a JSON
    /// `{objective, analysis}` here when the orchestrator decides the request needs
    /// the structured planner. The Task-mode router reads it after the lead turn to
    /// hand off to the planner. `None` = lead answered directly (no planning).
    pub(crate) pending_plan_request: Arc<RwLock<Option<String>>>,
}

impl AgentBridge {
    /// Bind a newly constructed GUI bridge to the stable `AiState` session slot
    /// before publishing it in the session map.
    pub fn bind_session_request_slot(&mut self, slot: Arc<SessionRequestSlot>, generation: u64) {
        debug_assert!(slot.accepts_generation(generation));
        self.session_request_slot = slot;
        self.session_request_generation = generation;
    }

    /// Acquire this bridge's universal top-level request boundary.
    ///
    /// A busy contender returns before cancellation, history, or harness
    /// side-channels can be touched. Once ownership exists, scrub any state left
    /// by a previously dropped/panicked future and open a fresh cancellation
    /// epoch for this request.
    pub async fn begin_top_level_request(&self) -> anyhow::Result<TopLevelRequestLease> {
        let cancel_epoch_before_acquire = self.cancel_epoch.load(Ordering::Acquire);
        let lease = self
            .session_request_slot
            .try_begin_request(self.session_request_generation)?;
        // Reset immediately after ownership, but preserve any Stop that raced
        // after this request began and before/while the slot CAS completed.
        self.reset_cancelled_unless_epoch_advanced(cancel_epoch_before_acquire);
        self.restore_isolated_history_recovery().await;
        self.clear_top_level_request_state(&lease).await?;
        anyhow::ensure!(
            lease.is_current_and_accepting(),
            "agent session generation changed while the request was starting"
        );
        Ok(lease)
    }

    /// Clear request-local harness state while the caller still owns this bridge.
    ///
    /// Normal entrypoints call this before returning. Acquisition calls it again
    /// after ownership as a fallback for async future drop/unwind, because Rust
    /// cannot await from `Drop`.
    pub async fn clear_top_level_request_state(
        &self,
        lease: &TopLevelRequestLease,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            lease.belongs_to(&self.session_request_slot, self.session_request_generation),
            "top-level request lease belongs to a different agent session"
        );

        *self.harness_active_stage.write().await = None;
        *self.harness_active_authz.write().await = None;
        *self.harness_active_org_id.write().await = None;
        *self.harness_active_operation_id.write().await = None;
        *self.harness_submit_only.write().await = false;
        *self.harness_forced_tool.write().await = None;
        *self.harness_last_deliverable.write().await = None;
        *self.pending_plan_request.write().await = None;
        Ok(())
    }

    pub fn cancel(&self) {
        self.cancel_epoch.fetch_add(1, Ordering::AcqRel);
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn reset_cancelled(&self) {
        self.cancelled.store(false, Ordering::SeqCst);
    }

    fn reset_cancelled_unless_epoch_advanced(&self, observed_epoch: u64) {
        self.reset_cancelled();
        if self.cancel_epoch.load(Ordering::Acquire) != observed_epoch {
            self.cancelled.store(true, Ordering::SeqCst);
        }
    }

    /// Claim the single pair of host-owned background listeners for this exact
    /// bridge generation. GUI candidates claim and pre-subscribe after their
    /// final setup await, then activate inside the publish transition;
    /// standalone bridges claim after their final `Arc` is ready.
    pub fn claim_background_listener_lifecycle(
        &self,
    ) -> Option<tokio::sync::watch::Receiver<bool>> {
        if *self.background_listener_retired.borrow() {
            return None;
        }
        if self
            .background_listeners_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return None;
        }
        let receiver = self.background_listener_retired.subscribe();
        if *receiver.borrow() {
            return None;
        }
        Some(receiver)
    }

    /// Permanently retire host listeners for this bridge generation. This is
    /// distinct from `cancel()`: an operator Stop keeps completion routing alive,
    /// while init replacement/shutdown must remove the stale listener owner.
    pub fn retire_session_generation(&self) {
        self.background_listener_retired.send_replace(true);
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
        self.background_listener_retired.send_replace(true);
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
                Ok(Some(info)) => {
                    tracing::debug!(
                        "AgentBridge::drop - sidecar session {} ended",
                        info.session_id
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
