use golish_core::events::AiEvent;
use golish_core::hitl::ApprovalDecision;
use golish_core::runtime::GolishRuntime;
use golish_core::ApiRequestStats;
use golish_sub_agents::SubAgentRegistry;
use golish_tools::ToolRegistry;
use rig::completion::Message;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, RwLock};

/// Trait for custom/MCP tool executors.
///
/// Implementors handle tool calls by name and return `Some((result, success))`
/// when handled, or `None` to fall through to built-in tool dispatch.
#[async_trait::async_trait]
pub trait McpToolExecutor: Send + Sync {
    async fn execute_tool(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Option<(serde_json::Value, bool)>;
}
use super::ToolSelectionConfig;
use golish_agent_kit::hitl::ApprovalRecorder;
use golish_agent_kit::loop_detection::LoopDetector;
use golish_agent_kit::sidecar_trait::{AiEventProcessor, SessionCaptureBackend};
use golish_agent_kit::tool_policy::ToolPolicyManager;
use golish_context::{CompactionState, ContextManager};
use golish_events::event_coordinator::CoordinatorHandle;
use golish_indexer::IndexerState;

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
    pub model_factory: Option<&'a Arc<golish_agent_kit::llm_client::LlmClientFactory>>,
    /// User-supplied per-model override (thinking on/off, effort, max_tokens, …).
    /// Forwarded into `resolve_stream_quirks` to customize stream parsing and
    /// into request builders to inject `enable_thinking=false` etc.
    pub model_override: Option<&'a golish_settings::schema::ModelOverride>,
}

/// Tool access control: policy engine, HITL approval, agent mode, loop detection.
pub struct LoopAccessControl<'a> {
    pub approval_recorder: &'a Arc<ApprovalRecorder>,
    pub pending_approvals: &'a Arc<RwLock<HashMap<String, oneshot::Sender<ApprovalDecision>>>>,
    pub tool_policy_manager: &'a Arc<ToolPolicyManager>,
    pub agent_mode: &'a Arc<RwLock<golish_agent_kit::agent_mode::AgentMode>>,
    pub loop_detector: &'a Arc<RwLock<LoopDetector>>,
    pub coordinator: Option<&'a CoordinatorHandle>,
}

/// Event emission, transcript, tracing, and runtime references.
pub struct LoopEventRefs<'a> {
    pub event_tx: &'a mpsc::UnboundedSender<AiEvent>,
    pub transcript_writer: Option<&'a Arc<golish_events::transcript::TranscriptWriter>>,
    pub transcript_base_dir: Option<&'a std::path::Path>,
    pub session_id: Option<&'a str>,
    pub db_tracker: Option<&'a golish_agent_kit::db_tracking::DbTracker>,
    pub runtime: Option<&'a Arc<dyn GolishRuntime>>,
}

/// Async callback invoked after a shell command completes, used to store
/// structured output (e.g. pentest tool results) without `golish-ai` depending
/// on domain-specific crates.
///
/// Arguments: (command, stdout, project_path, organization_id).
/// The closure captures any external resources (e.g. a DB pool) it needs.
pub type PostShellHook = Arc<
    dyn Fn(
            String,
            String,
            Option<String>,
            Option<uuid::Uuid>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        + Send
        + Sync,
>;

/// Synchronous classifier that returns `true` when a shell command's output
/// already has domain-specific structured storage, so the generic memory store
/// can skip it.
pub type OutputClassifier = Arc<dyn Fn(&str, &str) -> bool + Send + Sync>;

/// Request-scoped circuit breaker for `stage_run` retry exhaustion.
///
/// One instance is shared by every Primary-agent loop/reflector pass belonging
/// to the same top-level Task request. A later user request receives a reset
/// guard, so it can legitimately resume the saved worker chain with a fresh
/// bounded retry budget.
#[derive(Debug, Default)]
pub struct StageRunReentryGuard {
    exhausted_stages: std::sync::Mutex<HashSet<golish_agent_kit::harness::StageKind>>,
}

impl StageRunReentryGuard {
    /// Whether this request already consumed the bounded per-org retry budget
    /// for `stage`.
    pub fn is_exhausted(&self, stage: golish_agent_kit::harness::StageKind) -> bool {
        self.exhausted_stages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(&stage)
    }

    /// Close the circuit for `stage` until the next top-level Task request.
    pub fn mark_exhausted(&self, stage: golish_agent_kit::harness::StageKind) {
        self.exhausted_stages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(stage);
    }

    /// Start a separate top-level Task request with a fresh bounded budget.
    pub fn reset(&self) {
        self.exhausted_stages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }
}

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
    pub tool_config: &'a ToolSelectionConfig,
    pub graph_backend:
        Option<Arc<dyn golish_agent_kit::tool_executors::graph_trait::GraphKnowledgeBase>>,
    pub sidecar_state: Option<&'a Arc<dyn SessionCaptureBackend>>,
    pub chain_persistence: Option<Arc<dyn golish_sub_agents::SubAgentChainPersistence>>,
    pub runtime_memory: Option<Arc<dyn golish_agent_kit::db_traits::RuntimeMemoryRepository>>,
    /// Whole-record source selected once by a trusted resume preflight. Every
    /// graph/worker/chain read in this request must honor the same value.
    pub resume_runtime_memory_source:
        Option<golish_agent_kit::db_traits::RuntimeMemoryRecordSource>,
    /// Exact-scope, server-authorized ContextPack provider. The opaque trusted
    /// context remains inside the provider; runtime supplies only its own
    /// operation/unit/worker identity hint and semantic query.
    pub knowledge_context: Option<Arc<dyn golish_memory_app::ContextPackProvider>>,
    pub plan_manager: &'a Arc<golish_agent_kit::planner::PlanManager>,
    pub api_request_stats: &'a Arc<ApiRequestStats>,
    pub additional_tool_definitions: Vec<rig::completion::ToolDefinition>,
    pub custom_tool_executor: Option<Arc<dyn McpToolExecutor>>,
    pub cancelled: Option<&'a Arc<std::sync::atomic::AtomicBool>>,
    pub execution_monitor: Option<Arc<RwLock<golish_agent_kit::loop_detection::ExecutionMonitor>>>,
    pub execution_mode: golish_agent_kit::execution_mode::ExecutionMode,
    /// Per-mode tool exposure strategy. The agentic loop's
    /// `tool_list::build_tool_list` consults this registry to look up
    /// the active [`crate::execution_mode::policy::ExecutionModePolicy`]
    /// for `execution_mode`. Owned at the `AgentBridge` level and
    /// cloned (cheap `Arc`) into each per-turn loop context.
    pub execution_mode_registry: Arc<crate::execution_mode::ExecutionModeRegistry>,

    // -- Domain hooks (injected by the host crate) ----------------------------
    /// Called after a successful `run_pty_cmd` execution to detect and store
    /// structured output (e.g. pentest scan results) in the database.
    pub post_shell_hook: Option<PostShellHook>,
    /// Returns `true` when a shell command's output already has structured
    /// storage, so the generic memory store can skip duplicating it.
    pub output_classifier: Option<OutputClassifier>,
    /// Web fetch provider (injected by the host crate).
    pub web_fetcher: Option<Arc<dyn golish_core::WebFetchProvider>>,

    /// C3 · active harness stage for per-tool dispatch authz (forbidden-tool
    /// barrier). Set by the host bridge when running a harness-staged subtask;
    /// `None` = no enforcement (flag off / non-stage turn).
    pub harness_stage: Option<golish_agent_kit::harness::StageKind>,
    /// C3 · authorization context (profile ceiling + classified intent) for the
    /// active subtask. Threaded alongside `harness_stage`; lets per-tool dispatch
    /// run the full pre-action authorizer (allowed_tools confinement + intent vs
    /// ceiling) on real executor tools. `None` = no stage (flag off / non-stage).
    pub harness_authz: Option<golish_agent_kit::harness::HarnessAuthz>,
    /// 设计 2026-06-11 (weak-model-submit-channel) · `true` only on a targeted
    /// gate-repair pass whose sole remaining action is the stage submission.
    /// The completion phase then locks the turn's `tool_choice` to
    /// `submit_stage_deliverable` (released once it has been dispatched).
    /// `false` = normal pass, tool_choice behavior unchanged.
    pub harness_submit_only: bool,
    /// Optional one-shot forced orchestration tool for deterministic resume
    /// turns. The turn state releases this after the named tool is dispatched.
    /// `None` = normal provider/tool-choice behavior.
    pub harness_forced_tool: Option<String>,
    /// C2c · optional sink for a `StageDeliverable` produced by a delegated
    /// sub-agent (e.g. `reporter`). When set, the sub-agent call handler writes
    /// any result carrying a deliverable signature here, so the Task-mode
    /// executor can feed it to the deterministic gate even when the Primary
    /// orchestrator narrated instead of inlining the JSON. `None` = no capture
    /// (non-stage turn / chat mode).
    pub harness_deliverable_sink: Option<std::sync::Arc<tokio::sync::RwLock<Option<String>>>>,
    /// Engagement-org isolation (设计 2026-06-15-engagement-org-isolation): the
    /// scoping-confirmed root organization id of the active operation, used to
    /// confine fan-out / in-scope reads to this org's subtree (root + subs).
    /// `None` = no bound org (legacy whole-DB axis; flag off / chat / pre-scoping).
    pub harness_org_id: Option<uuid::Uuid>,
    /// Writable active-org side-channel. Org-aware registry tools still receive a
    /// per-call hidden org arg in stage_run workers, but this handle is kept for
    /// bridge-owned tools that only understand the legacy active-org binding.
    pub harness_org_id_source: Option<Arc<RwLock<Option<uuid::Uuid>>>>,
    /// Current harness operation id (Task id in graph-flow task mode). Loop-level
    /// tools use this to update operation-scoped state such as
    /// `operation_state.state_blob` without guessing from the DB tracker session.
    pub harness_operation_id: Option<uuid::Uuid>,
    /// Trusted `stage_runs.id` for the active execution.
    pub stage_execution_id: Option<uuid::Uuid>,
    /// Trusted per-organization stage unit for the active execution.
    pub stage_run_unit_id: Option<uuid::Uuid>,
    /// Trusted specialist worker fencing tuple. When present, its unit witness
    /// must equal `stage_run_unit_id`.
    pub worker_lease: Option<golish_core::WorkerLeaseContext>,
    /// Opaque Candidate verification identity. Exact action authorization is
    /// reloaded from durable state immediately before every action.
    pub candidate_attempt: Option<golish_core::CandidateAttemptContextRef>,
    /// Shared circuit breaker for this top-level Task request. `stage_run`
    /// closes the stage entry after its bounded per-org retry budget is
    /// exhausted; a separate user request resets it before resuming.
    pub stage_run_reentry_guard: Arc<StageRunReentryGuard>,
}

/// Check cancellation flag; returns true when the user has requested a stop.
pub(super) fn is_cancelled(ctx: &AgenticLoopContext<'_>) -> bool {
    ctx.cancelled
        .map(|f| f.load(std::sync::atomic::Ordering::SeqCst))
        .unwrap_or(false)
}

/// Resolve and render one exact-scope ContextPack. Missing runtime identity is
/// an explicit no-context state; retrieval failures never fall back to legacy
/// global memories/wiki/graph searches.
pub(crate) async fn retrieve_scoped_context_data(
    ctx: &AgenticLoopContext<'_>,
    query: &str,
    organization_id: Option<uuid::Uuid>,
    worker_run_id: Option<uuid::Uuid>,
) -> Result<Option<String>, String> {
    let Some(provider) = ctx.knowledge_context.as_ref() else {
        return Ok(None);
    };
    let (Some(operation_id), Some(stage_execution_id), Some(stage_run_unit_id), Some(stage)) = (
        ctx.harness_operation_id,
        ctx.stage_execution_id,
        ctx.stage_run_unit_id,
        ctx.harness_stage,
    ) else {
        return Ok(None);
    };
    let Some(organization_id) = organization_id.or(ctx.harness_org_id) else {
        return Ok(None);
    };
    let worker_run_id =
        worker_run_id.or_else(|| ctx.worker_lease.as_ref().map(|lease| lease.worker_run_id));
    let subject = golish_memory_domain::ContextSubject::from_server_runtime(
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        worker_run_id,
        organization_id,
        stage.as_str(),
        None,
    )
    .map_err(|error| error.code().to_string())?;
    let pack = provider
        .retrieve(
            subject,
            golish_memory_domain::ContextRequest::for_harness(query, 2_048),
        )
        .await
        .map_err(|error| error.code().to_string())?;
    let rendered =
        golish_agent_kit::harness::render_context_pack(&pack).map_err(|error| error.to_string())?;
    Ok(Some(rendered.data_block().to_string()))
}

/// Result of a single tool execution.
pub struct ToolExecutionResult {
    pub value: serde_json::Value,
    pub success: bool,
}

/// Wrapper for capture context that persists across the loop.
///
/// Uses the `AiEventProcessor` trait to decouple from the concrete
/// `golish-sidecar` crate.
pub struct LoopCaptureContext {
    inner: Option<std::sync::Mutex<Box<dyn AiEventProcessor>>>,
}

impl LoopCaptureContext {
    pub fn new(backend: Option<&Arc<dyn SessionCaptureBackend>>) -> Self {
        Self {
            inner: backend.map(|b| std::sync::Mutex::new(b.create_event_processor())),
        }
    }

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
        if golish_events::transcript::should_transcript(&event) {
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
        if golish_events::transcript::should_transcript(&event) {
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
        sidecar.capture_event(&event);
    }
}

#[cfg(test)]
mod stage_run_reentry_guard_tests {
    use super::StageRunReentryGuard;
    use golish_agent_kit::harness::StageKind;

    #[test]
    fn exhaustion_is_stage_local_and_reset_opens_a_new_request_budget() {
        let guard = StageRunReentryGuard::default();

        assert!(!guard.is_exhausted(StageKind::Enumeration));
        guard.mark_exhausted(StageKind::Enumeration);
        assert!(guard.is_exhausted(StageKind::Enumeration));
        assert!(!guard.is_exhausted(StageKind::ExternalAttackSurface));

        guard.reset();
        assert!(!guard.is_exhausted(StageKind::Enumeration));
    }
}
