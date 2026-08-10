use golish_core::events::AiEvent;
use golish_core::hitl::ApprovalDecision;
use golish_core::runtime::GolishRuntime;
use golish_core::ApiRequestStats;
use golish_sub_agents::SubAgentRegistry;
use golish_tools::ToolRegistry;
use rig::completion::Message;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
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
    /// Product-composed Application Understanding controller invoked only by
    /// the visible Primary Agent's `stage_run` call.
    pub application_understanding_runtime:
        Option<Arc<dyn golish_agent_kit::task_orchestrator::ApplicationUnderstandingStageRuntime>>,
    /// Optional host-owned Plan B Candidate analysis runtime. The bridge only
    /// snapshots this capability; repository construction and authorization
    /// remain in the production composition root.
    pub hypothesis_analysis_runtime: Option<
        Arc<
            dyn golish_agent_kit::task_orchestrator::hypothesis_analysis::HypothesisAnalysisStageRuntime,
        >,
    >,
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
    /// Explicit eval/fixture-only Goal Loop selector. Production operation
    /// state and profile names can never synthesize this authority.
    pub target_intel_goal_shadow:
        Option<&'a crate::eval_support::TargetIntelGoalShadowFixture>,
    /// Shared host-owned evidence adapter used by both root and SubAgent paths
    /// while the fixture selector above is active.
    pub intel_public_adapter:
        Option<Arc<dyn golish_agent_kit::tool_executors::IntelPublicEvidenceAdapter>>,

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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BoundScopedContextIdentity {
    pub operation_id: uuid::Uuid,
    pub stage_execution_id: uuid::Uuid,
    pub stage_run_unit_id: uuid::Uuid,
    pub worker_run_id: uuid::Uuid,
    pub organization_id: uuid::Uuid,
}

/// Redacted prompt data plus the exact-set receipt material needed by the
/// unified Investigation read-session ledger. Raw ContextPack values stay in
/// the runtime; only counts and hashes cross the persistence port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RetrievedScopedContextData {
    pub rendered: String,
    pub context_item_count: usize,
    pub context_item_set_sha256: String,
    pub omission_count: usize,
    pub omission_set_sha256: String,
    pub omission_members: Vec<String>,
}

pub(crate) fn scoped_context_exact_set_receipt(
    schema: &str,
    mut members: Vec<String>,
) -> (usize, String) {
    use sha2::{Digest, Sha256};

    members.sort();
    members.dedup();
    let mut hasher = Sha256::new();
    hasher.update((schema.len() as u64).to_be_bytes());
    hasher.update(schema.as_bytes());
    hasher.update((members.len() as u64).to_be_bytes());
    for member in &members {
        hasher.update((member.len() as u64).to_be_bytes());
        hasher.update(member.as_bytes());
    }
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    (members.len(), format!("sha256:{encoded}"))
}

impl RetrievedScopedContextData {
    pub(crate) fn with_host_omission(mut self, source: &str, reason: &str) -> Self {
        self.omission_members
            .push(format!("host:{source}:{reason}"));
        self.omission_count = self.omission_count.saturating_add(1);
        let (_, digest) = scoped_context_exact_set_receipt(
            "investigation_context_omissions.v1",
            self.omission_members.clone(),
        );
        self.omission_set_sha256 = digest;
        self
    }
}

fn context_omission_members(
    omitted: &golish_memory_app::ContextOmissionSummary,
) -> Result<Vec<String>, String> {
    if omitted.item_ids.len() > omitted.omitted_count
        || (omitted.omitted_count == 0
            && (!omitted.item_ids.is_empty() || !omitted.reasons.is_empty()))
    {
        return Err("knowledge_context_omission_census_invalid".to_string());
    }
    let (_, reason_set_sha256) = scoped_context_exact_set_receipt(
        "investigation_context_omission_reasons.v1",
        omitted.reasons.clone(),
    );
    let mut members = omitted
        .item_ids
        .iter()
        .map(|item_id| format!("{reason_set_sha256}:item:{item_id}"))
        .collect::<Vec<_>>();
    for ordinal in members.len()..omitted.omitted_count {
        members.push(format!("{reason_set_sha256}:anonymous:{ordinal}"));
    }
    Ok(members)
}

const MAX_CONTEXT_QUERY_CHARS: usize = 4_096;

fn bounded_context_query(query: &str, stage: &str) -> String {
    let prefix = format!("stage={stage}\n");
    let prefix_chars = prefix.chars().count();
    let available = MAX_CONTEXT_QUERY_CHARS.saturating_sub(prefix_chars);
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return format!("{prefix}retrieve exact-scope operational context");
    }
    let chars = trimmed.chars().collect::<Vec<_>>();
    if chars.len() <= available {
        return format!("{prefix}{trimmed}");
    }
    const SEPARATOR: &str = "\n[...bounded...]\n";
    let content_budget = available.saturating_sub(SEPARATOR.chars().count());
    let head_len = content_budget.saturating_mul(3) / 4;
    let tail_len = content_budget.saturating_sub(head_len);
    let head = chars.iter().take(head_len).collect::<String>();
    let tail = chars
        .iter()
        .skip(chars.len().saturating_sub(tail_len))
        .collect::<String>();
    format!("{prefix}{head}{SEPARATOR}{tail}")
}

fn validate_bound_scoped_context_identity(
    bound: BoundScopedContextIdentity,
    outer_operation_id: Option<uuid::Uuid>,
    outer_stage_execution_id: Option<uuid::Uuid>,
    outer_stage_run_unit_id: Option<uuid::Uuid>,
    outer_organization_id: Option<uuid::Uuid>,
) -> Result<BoundScopedContextIdentity, String> {
    if outer_operation_id.is_some_and(|value| value != bound.operation_id)
        || outer_stage_execution_id.is_some_and(|value| value != bound.stage_execution_id)
        || outer_stage_run_unit_id.is_some_and(|value| value != bound.stage_run_unit_id)
        || outer_organization_id.is_some_and(|value| value != bound.organization_id)
    {
        return Err("knowledge_context_bound_identity_mismatch".to_string());
    }
    Ok(bound)
}

fn scoped_context_request(
    query: String,
    include_mutable_runtime: bool,
) -> golish_memory_domain::ContextRequest {
    let mut request = golish_memory_domain::ContextRequest::for_harness(query, 2_048);
    if !include_mutable_runtime {
        request
            .requested_classes
            .remove(&golish_memory_domain::KnowledgeClass::RuntimeState);
    }
    request
}

async fn retrieve_scoped_context_receipt_data_with_runtime_policy(
    ctx: &AgenticLoopContext<'_>,
    query: &str,
    organization_id: Option<uuid::Uuid>,
    worker_run_id: Option<uuid::Uuid>,
    bound_identity: Option<BoundScopedContextIdentity>,
    include_mutable_runtime: bool,
) -> Result<Option<RetrievedScopedContextData>, String> {
    let Some(provider) = ctx.knowledge_context.as_ref() else {
        return Ok(None);
    };
    let Some(stage) = ctx.harness_stage else {
        return Ok(None);
    };
    let (operation_id, stage_execution_id, stage_run_unit_id, worker_run_id, organization_id) =
        if let Some(bound) = bound_identity {
            let bound = validate_bound_scoped_context_identity(
                bound,
                ctx.harness_operation_id,
                ctx.stage_execution_id,
                ctx.stage_run_unit_id,
                organization_id.or(ctx.harness_org_id),
            )?;
            (
                bound.operation_id,
                bound.stage_execution_id,
                bound.stage_run_unit_id,
                Some(bound.worker_run_id),
                bound.organization_id,
            )
        } else {
            let (Some(operation_id), Some(stage_execution_id), Some(stage_run_unit_id)) = (
                ctx.harness_operation_id,
                ctx.stage_execution_id,
                ctx.stage_run_unit_id,
            ) else {
                return Ok(None);
            };
            let Some(organization_id) = organization_id.or(ctx.harness_org_id) else {
                return Ok(None);
            };
            let worker_run_id = worker_run_id
                .or_else(|| ctx.worker_lease.as_ref().map(|lease| lease.worker_run_id));
            (
                operation_id,
                stage_execution_id,
                stage_run_unit_id,
                worker_run_id,
                organization_id,
            )
        };
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
    let query = bounded_context_query(query, stage.as_str());
    let pack = provider
        .retrieve(
            subject,
            scoped_context_request(query, include_mutable_runtime),
        )
        .await
        .map_err(|error| {
            tracing::warn!(
                target: "harness::knowledge_context",
                code = error.code(),
                detail = %error,
                "exact-scope ContextPack provider rejected retrieval"
            );
            error.code().to_string()
        })?;
    tracing::info!(
        target: "harness::knowledge_context",
        canonical = pack.canonical_items.len(),
        runtime = pack.runtime_items.len(),
        handoffs = pack.handoff_items.len(),
        episodes = pack.episode_items.len(),
        assertions = pack.assertion_items.len(),
        documents = pack.document_items.len(),
        graph = pack.graph_items.len(),
        vector = pack.vector_items.len(),
        omitted = pack.omitted.omitted_count,
        omission_reasons = ?pack.omitted.reasons,
        "exact-scope ContextPack retrieved"
    );
    let context_members = pack
        .items()
        .map(|item| format!("{}:{}", item.item_id, item.content_hash))
        .collect::<Vec<_>>();
    let (context_item_count, context_item_set_sha256) =
        scoped_context_exact_set_receipt("investigation_context_items.v1", context_members);
    let omission_count = pack.omitted.omitted_count;
    let omission_members = context_omission_members(&pack.omitted)?;
    let (_, omission_set_sha256) = scoped_context_exact_set_receipt(
        "investigation_context_omissions.v1",
        omission_members.clone(),
    );
    let rendered =
        golish_agent_kit::harness::render_context_pack(&pack).map_err(|error| error.to_string())?;
    Ok(Some(RetrievedScopedContextData {
        rendered: rendered.data_block().to_string(),
        context_item_count,
        context_item_set_sha256,
        omission_count,
        omission_set_sha256,
        omission_members,
    }))
}

/// Retrieve the immutable input census sealed by the unified Investigation
/// read-session authority. The current Unit/Worker rows are deliberately not
/// members: their attempt/checkpoint fields change as soon as the Primary
/// executes and would make response-loss replay fail by construction.
pub(crate) async fn retrieve_scoped_context_receipt_data(
    ctx: &AgenticLoopContext<'_>,
    query: &str,
    organization_id: Option<uuid::Uuid>,
    worker_run_id: Option<uuid::Uuid>,
    bound_identity: Option<BoundScopedContextIdentity>,
) -> Result<Option<RetrievedScopedContextData>, String> {
    retrieve_scoped_context_receipt_data_with_runtime_policy(
        ctx,
        query,
        organization_id,
        worker_run_id,
        bound_identity,
        false,
    )
    .await
}

pub(crate) async fn retrieve_scoped_context_data(
    ctx: &AgenticLoopContext<'_>,
    query: &str,
    organization_id: Option<uuid::Uuid>,
    worker_run_id: Option<uuid::Uuid>,
    bound_identity: Option<BoundScopedContextIdentity>,
) -> Result<Option<String>, String> {
    retrieve_scoped_context_receipt_data_with_runtime_policy(
        ctx,
        query,
        organization_id,
        worker_run_id,
        bound_identity,
        true,
    )
    .await
    .map(|data| data.map(|data| data.rendered))
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
    use super::{
        bounded_context_query, context_omission_members, scoped_context_exact_set_receipt,
        scoped_context_request, validate_bound_scoped_context_identity, BoundScopedContextIdentity,
        StageRunReentryGuard, MAX_CONTEXT_QUERY_CHARS,
    };
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

    #[test]
    fn bound_context_identity_fills_missing_outer_unit_but_rejects_drift() {
        let bound = BoundScopedContextIdentity {
            operation_id: uuid::Uuid::new_v4(),
            stage_execution_id: uuid::Uuid::new_v4(),
            stage_run_unit_id: uuid::Uuid::new_v4(),
            worker_run_id: uuid::Uuid::new_v4(),
            organization_id: uuid::Uuid::new_v4(),
        };
        assert_eq!(
            validate_bound_scoped_context_identity(
                bound,
                Some(bound.operation_id),
                None,
                None,
                Some(bound.organization_id),
            ),
            Ok(bound)
        );
        assert_eq!(
            validate_bound_scoped_context_identity(
                bound,
                Some(bound.operation_id),
                Some(uuid::Uuid::new_v4()),
                None,
                Some(bound.organization_id),
            ),
            Err("knowledge_context_bound_identity_mismatch".to_string())
        );
    }

    #[test]
    fn scoped_context_query_bounds_large_worker_assignments_without_losing_both_ends() {
        let query = format!("BEGIN-{}-END", "记忆".repeat(3_000));
        let bounded = bounded_context_query(&query, "attack_candidate");

        assert!(bounded.chars().count() <= MAX_CONTEXT_QUERY_CHARS);
        assert!(bounded.starts_with("stage=attack_candidate\nBEGIN-"));
        assert!(bounded.contains("[...bounded...]"));
        assert!(bounded.ends_with("-END"));
        assert_eq!(
            bounded_context_query("   ", "target_intel"),
            "stage=target_intel\nretrieve exact-scope operational context"
        );
    }

    #[test]
    fn scoped_context_receipt_hashes_the_unique_sorted_member_set() {
        let first = scoped_context_exact_set_receipt(
            "investigation_context_items.v1",
            vec!["b".to_string(), "a".to_string(), "a".to_string()],
        );
        let replay = scoped_context_exact_set_receipt(
            "investigation_context_items.v1",
            vec!["a".to_string(), "b".to_string()],
        );

        assert_eq!(first.0, 2);
        assert_eq!(first, replay);
        assert!(first.1.starts_with("sha256:"));
        assert_eq!(first.1.len(), 71);
    }

    #[test]
    fn investigation_read_snapshot_excludes_its_mutating_runtime_rows() {
        let sealed = scoped_context_request("investigation".to_string(), false);
        assert!(!sealed
            .requested_classes
            .contains(&golish_memory_domain::KnowledgeClass::RuntimeState));
        assert!(sealed
            .requested_classes
            .contains(&golish_memory_domain::KnowledgeClass::PassedHandoff));

        let live = scoped_context_request("supervisor".to_string(), true);
        assert!(live
            .requested_classes
            .contains(&golish_memory_domain::KnowledgeClass::RuntimeState));
    }

    #[test]
    fn omission_members_bind_reasons_without_inflating_the_reported_count() {
        let members = context_omission_members(&golish_memory_app::ContextOmissionSummary {
            omitted_count: 2,
            reasons: vec!["redacted".to_string(), "budget".to_string()],
            item_ids: vec!["item-a".to_string()],
        })
        .expect("valid omission census");

        assert_eq!(members.len(), 2);
        assert!(members[0].contains(":item:item-a"));
        assert!(members[1].contains(":anonymous:1"));
        assert_eq!(
            members[0].split(":item:").next(),
            members[1].split(":anonymous:").next()
        );
    }

    #[test]
    fn omission_members_reject_unaccounted_reason_only_state() {
        assert_eq!(
            context_omission_members(&golish_memory_app::ContextOmissionSummary {
                omitted_count: 0,
                reasons: vec!["redacted".to_string()],
                item_ids: Vec::new(),
            }),
            Err("knowledge_context_omission_census_invalid".to_string())
        );
    }
}
