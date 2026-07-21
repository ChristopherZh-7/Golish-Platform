use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rig::completion::request::ToolDefinition;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::sync::RwLock;

use golish_core::events::AiEvent;
use golish_core::ApiRequestStats;
use golish_tools::ToolRegistry;

/// Barrier tool name used by all sub-agents to submit structured results.
/// When a sub-agent calls this tool, the executor terminates the loop and
/// returns the structured result to the parent agent (PentAGI barrier pattern).
pub const BARRIER_TOOL_NAME: &str = "submit_result";
/// Host-routed control tool exposed only to a trusted Company Controller.
pub const STAGE_TEAM_DISPATCH_WORKERS_TOOL_NAME: &str = "stage_team_dispatch_workers";
/// Host-routed control tool used by a trusted Company Controller to close its
/// current request epoch and hand finalization back to the outer scheduler.
pub const STAGE_TEAM_PREPARE_FINAL_SUBMISSION_TOOL_NAME: &str =
    "stage_team_prepare_final_submission";
/// Local planning tool exposed through the static catalogue only to an exact
/// bound Company Controller. Unbound orchestrator agents retain their existing
/// generic `update_plan` path.
pub const STAGE_TEAM_UPDATE_PLAN_TOOL_NAME: &str = "update_plan";
/// Exact successful router status that parks the Controller after dispatch.
pub const STAGE_TEAM_DISPATCH_ACCEPTED_STATUS: &str = "dispatch_accepted";
/// Exact successful router status that transfers finalization to the scheduler.
pub const STAGE_TEAM_PREPARE_FINAL_STATUS: &str = "prepare_final";

const MODEL_RECOVERY_ACTION_SAMPLE_LIMIT: usize = 20;
const MODEL_RECOVERY_INSTRUCTION_MAX_BYTES: usize = 32 * 1024;
const MODEL_RECOVERY_BLOCK_PAYLOAD_MAX_BYTES: usize = 64 * 1024;
const MODEL_RECOVERY_PROJECTION_MARKER: &str = "Recovery actions: total=";
const MODEL_RECOVERY_TRUNCATION_SUFFIX: &str =
    "\n[Recovery instruction truncated. Use stage_worklist_next for bounded DB-backed pages.]";

/// Durable sub-agent chain failures that callers must distinguish from ordinary
/// model/tool failures. An explicit resume must never silently degrade to a
/// different or fresh worker, and a failed finalize must never advertise a
/// resumable chain marker.
#[derive(Debug, thiserror::Error)]
pub enum SubAgentChainError {
    #[error("exact sub-agent chain {chain_id} is unavailable: {reason}")]
    ExactResumeUnavailable {
        chain_id: uuid::Uuid,
        reason: String,
    },
    #[error("latest sub-agent chain for '{agent_id}' is unavailable: {reason}")]
    LatestResumeUnavailable { agent_id: String, reason: String },
    #[error("failed to create fresh sub-agent chain for '{agent_id}': {reason}")]
    CreateFreshFailed { agent_id: String, reason: String },
    #[error("failed to finalize sub-agent chain {chain_id}: {reason}")]
    FinalizeFailed {
        chain_id: uuid::Uuid,
        checkpointed_chain_id: Option<uuid::Uuid>,
        reason: String,
    },
    #[error("provider context limit exceeded for sub-agent chain {chain_id:?}: {reason}")]
    ProviderContextLimitExceeded {
        chain_id: Option<uuid::Uuid>,
        reason: String,
    },
    #[error("prebound stage worker {worker_run_id} is unavailable: {reason}")]
    BoundWorkerUnavailable {
        worker_run_id: uuid::Uuid,
        reason: String,
    },
}

/// Trusted V2 stage-worker chain identity returned only after the atomic
/// claim-and-bind transaction commits.
///
/// The executor never derives any field here from model-visible arguments. The
/// shared checkpoint counter is the CAS witness for chain checkpoints; the
/// lease-loss flag is set by the runtime heartbeat/fencing supervisor and is
/// checked before subsequent provider/tool work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageTeamLeaderBinding {
    pub stage_team_plan_id: uuid::Uuid,
    pub leader_work_item_id: uuid::Uuid,
    pub expected_dispatch_epoch: i64,
    pub expected_plan_row_version: i64,
    pub expected_work_item_row_version: i64,
}

#[derive(Clone)]
pub struct BoundWorkerChainContext {
    pub operation_id: uuid::Uuid,
    pub stage_execution_id: uuid::Uuid,
    pub organization_id: uuid::Uuid,
    pub worker_lease: golish_core::WorkerLeaseContext,
    /// Opaque Candidate identity for a prebound verifier WorkerRun. Generic
    /// stage workers keep this `None`.
    pub candidate_attempt: Option<golish_core::CandidateAttemptContextRef>,
    /// Host-owned continuation mode. Once a terminal action exists without a
    /// terminal intent, the same durable chain may read evidence and submit
    /// only; it may never dispatch another external action.
    pub candidate_submit_only: bool,
    /// Host-owned Team Aggregator policy. Once `submit_stage_deliverable`
    /// durably persists either an accepted or needs-fix submission, return the
    /// submission id to the outer scheduler so it owns Gate repair routing.
    /// Ordinary stage workers keep this disabled and retain their in-chain
    /// repair behavior.
    pub return_on_first_durable_stage_submission: bool,
    /// Trusted Company Controller identity. This is populated only from a
    /// server-owned Stage Team claim; agent-visible arguments can never create
    /// or widen it. Ordinary workers and legacy Aggregators keep this `None`.
    pub stage_team_leader: Option<StageTeamLeaderBinding>,
    pub chain_id: uuid::Uuid,
    pub session_id: uuid::Uuid,
    pub agent_type: String,
    /// Whole-record source chosen once for this resumed operation. Fresh worker
    /// runs leave it unset and use the operation's frozen rollout contract.
    pub runtime_memory_source: Option<BoundWorkerRuntimeMemorySource>,
    pub initial_chain: serde_json::Value,
    /// `true` only for a freshly claimed worker whose objective was included in
    /// the atomic initial chain. Resumed workers append the new objective after
    /// loading their prior checkpoint.
    pub initial_prompt_already_checkpointed: bool,
    pub checkpoint_version: Arc<AtomicI64>,
    pub checkpoint_body: Arc<std::sync::RwLock<serde_json::Value>>,
    pub lease_lost: Arc<AtomicBool>,
    /// Serializes heartbeat/checkpoint/tool-fence mutations so a legitimate
    /// checkpoint-version advance cannot race a heartbeat using the prior CAS
    /// witness and be misclassified as lease loss.
    pub mutation_lock: Arc<tokio::sync::Mutex<()>>,
    /// Host-owned lifecycle fence for every regular worker tool call. V2
    /// executors fail closed when the binding exists but this hook is absent.
    pub tool_lifecycle: Option<Arc<dyn BoundWorkerToolLifecycle>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundWorkerRuntimeMemorySource {
    Legacy,
    V2,
    LegacyFallback,
}

impl BoundWorkerChainContext {
    /// Whether this binding belongs to an ordinary durable Stage Team child.
    ///
    /// Candidate verifiers and Company Controllers have their own terminal
    /// contracts. Only ordinary children must return `stage_worker_output.v1`
    /// through the generic `submit_result` barrier.
    pub fn is_stage_team_child(&self) -> bool {
        self.candidate_attempt.is_none()
            && self.stage_team_leader.is_none()
            && !self.return_on_first_durable_stage_submission
    }

    pub fn current_checkpoint_version(&self) -> i64 {
        self.checkpoint_version.load(Ordering::SeqCst)
    }

    pub fn lease_is_lost(&self) -> bool {
        self.lease_lost.load(Ordering::SeqCst)
    }

    pub fn mark_lease_lost(&self) {
        self.lease_lost.store(true, Ordering::SeqCst);
    }

    pub fn current_checkpoint_body(&self) -> serde_json::Value {
        self.checkpoint_body
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn publish_checkpoint_body(&self, checkpoint: serde_json::Value) {
        *self
            .checkpoint_body
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = checkpoint;
    }
}

/// Awaited tool-call lifecycle for a claimed V2 worker.
///
/// `begin` must durably create the generic tool-call record before setting the
/// worker's active-tool marker. `finish` must clear that marker under the same
/// lease/version fence before the result is allowed to land in model history.
/// The concrete implementation owns typed begin-error classification and must
/// update its shared bound lease flag before returning an actual lease-loss
/// error; the generic executor cannot infer lease loss from an arbitrary
/// pre-dispatch storage error.
#[async_trait::async_trait]
pub trait BoundWorkerToolLifecycle: Send + Sync {
    async fn begin(
        &self,
        request_id: &str,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> anyhow::Result<uuid::Uuid>;

    async fn finish(
        &self,
        tool_call_record_id: uuid::Uuid,
        success: bool,
        result: &serde_json::Value,
    ) -> anyhow::Result<()>;
}

/// Trait for providing tool definitions to the sub-agent executor.
/// This allows the executor to be decoupled from the tool definition source.
#[async_trait::async_trait]
pub trait ToolProvider: Send + Sync {
    /// Get all available tool definitions
    fn get_all_tool_definitions(&self) -> Vec<ToolDefinition>;

    /// Filter tools to only those allowed by the sub-agent
    fn filter_tools_by_allowed(
        &self,
        tools: Vec<ToolDefinition>,
        allowed: &[String],
    ) -> Vec<ToolDefinition>;

    /// Execute a web fetch tool
    async fn execute_web_fetch_tool(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> (serde_json::Value, bool);

    async fn execute_memory_tool(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Option<(serde_json::Value, bool)>;

    /// Execute a knowledge-base / vulnerability-wiki tool (`search_knowledge_base`,
    /// `read_knowledge`, `write_knowledge`, `ingest_cve`, `save_poc`, …). Returns
    /// `None` when `tool_name` isn't a KB tool so the caller falls through to the
    /// router / registry — same contract as [`Self::execute_memory_tool`].
    async fn execute_knowledge_base_tool(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Option<(serde_json::Value, bool)>;

    /// Normalize run_pty_cmd arguments
    fn normalize_run_pty_cmd_args(&self, args: serde_json::Value) -> serde_json::Value;
}

/// Trait for sub-agent chain persistence operations (decoupled from sqlx).
///
/// Provides the message-chain DB operations the executor needs to
/// save/restore persistent chains across sub-agent invocations.
#[async_trait::async_trait]
pub trait SubAgentChainPersistence: Send + Sync {
    async fn chain_create(
        &self,
        session_id: uuid::Uuid,
        task_id: Option<uuid::Uuid>,
        subtask_id: Option<uuid::Uuid>,
        agent_type: &str,
        parent_chain_id: Option<uuid::Uuid>,
        model: Option<&str>,
    ) -> anyhow::Result<uuid::Uuid>;

    async fn chain_update(
        &self,
        id: uuid::Uuid,
        chain_json: &serde_json::Value,
    ) -> anyhow::Result<()>;

    async fn chain_update_usage(
        &self,
        id: uuid::Uuid,
        input_tokens: i32,
        output_tokens: i32,
        cache_read_tokens: i32,
        input_cost: f64,
        output_cost: f64,
        duration_ms: i32,
    ) -> anyhow::Result<()>;

    /// Load the most recent persisted chain for `(session, agent)`, if any, so a
    /// `resume` delegation can replay the same sub-agent's prior conversation
    /// (including the tool results / evidence ids it produced). Returns
    /// `(chain_id, chain_json)`. Default impl: no prior chain (never resumes).
    async fn chain_load_latest(
        &self,
        _session_id: uuid::Uuid,
        _task_id: Option<uuid::Uuid>,
        _agent_type: &str,
    ) -> anyhow::Result<Option<(uuid::Uuid, serde_json::Value)>> {
        Ok(None)
    }

    /// Load a SPECIFIC persisted chain by its id, scoped to the current DB
    /// session and agent. The scope check is part of the authorization contract:
    /// `resume` is model-visible, so possession of another chain UUID alone must
    /// not grant cross-session/cross-agent access. Default impl: not found.
    async fn chain_load_by_id(
        &self,
        _chain_id: uuid::Uuid,
        _session_id: uuid::Uuid,
        _agent_type: &str,
    ) -> anyhow::Result<Option<serde_json::Value>> {
        Ok(None)
    }

    /// Validate and load the checkpoint for a worker whose message-chain row
    /// was already bound by the runtime-memory claim transaction.
    async fn chain_load_bound_worker(
        &self,
        _bound: &BoundWorkerChainContext,
    ) -> anyhow::Result<Option<serde_json::Value>> {
        Ok(None)
    }

    /// Atomically checkpoint a prebound V2 worker under its exact lease/version
    /// fence. Implementations must not fall back to a raw message-chain update.
    async fn chain_checkpoint_bound_worker(
        &self,
        _bound: &BoundWorkerChainContext,
        _chain_id: uuid::Uuid,
        _chain_json: &serde_json::Value,
        _expected_checkpoint_version: i64,
    ) -> anyhow::Result<i64> {
        anyhow::bail!("bound worker checkpoint persistence is unavailable")
    }

    async fn load_prompt_template_overrides(&self) -> Vec<(String, String)>;
}

/// Shared cancellation probe for sub-agent workers.
///
/// The top-level [`AgentBridge`](golish_agent_bridge) owns the flag. Worker
/// executors only borrow it so user "Stop" requests can interrupt nested
/// sub-agent loops without letting a child clear the parent cancellation.
pub(crate) fn cancellation_requested(cancelled: Option<&Arc<AtomicBool>>) -> bool {
    cancelled
        .map(|flag| flag.load(Ordering::SeqCst))
        .unwrap_or(false)
}

pub(crate) async fn wait_for_cancelled(cancelled: Option<&Arc<AtomicBool>>) {
    loop {
        if cancellation_requested(cancelled) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Async callback invoked after a shell command completes.
///
/// Arguments: (command, stdout, project_path, organization_id).
/// The closure captures external resources (e.g. a DB pool) it needs.
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

/// Extra tool executor for tools that live OUTSIDE the [`ToolRegistry`]
/// (for example security-analysis read helpers and graph tools). Given
/// `(tool_name, args)` it returns
/// `Some((value, success))` if it handled the call, else `None` to fall through
/// to the registry. A plain `Fn` so this crate stays free of harness-crate
/// deps; the runtime builds it with the needed app/runtime backends wired in.
pub type SubAgentToolRouter = Arc<
    dyn Fn(
            String,
            serde_json::Value,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Option<(serde_json::Value, bool)>> + Send>,
        > + Send
        + Sync,
>;

/// Optional hook invoked after a sub-agent tool returns and before the result is
/// emitted back to the model. Runtime crates use this to mirror main-agent
/// harness side effects (for example evidence/source-query materialization)
/// without making `golish-sub-agents` depend on the harness or database layers.
pub type SubAgentToolResultHook = Arc<
    dyn Fn(
            String,
            serde_json::Value,
            serde_json::Value,
            bool,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = (serde_json::Value, bool)> + Send>>
        + Send
        + Sync,
>;

/// Owned snapshot of a completed sub-agent tool call for runtime observers.
///
/// This crate intentionally does not know about harness gates, DB evidence, or
/// mentor LLMs. Runtime crates can attach a callback that watches the generic
/// stream of tool outcomes and optionally returns model-visible guidance.
#[derive(Debug, Clone)]
pub struct SubAgentToolObservation {
    pub agent_id: String,
    pub agent_name: String,
    pub parent_request_id: String,
    pub tool_call_id: String,
    pub tool_name: String,
    pub tool_args: serde_json::Value,
    pub result: serde_json::Value,
    pub success: bool,
}

/// Optional observer invoked after normal sub-agent tool post-processing.
///
/// Returning `Some(text)` appends that text to the model-visible ToolResult.
/// Returning `None` keeps the observation trace-only.
pub type SubAgentToolObserver = Arc<
    dyn Fn(
            SubAgentToolObservation,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<String>> + Send>>
        + Send
        + Sync,
>;

/// Per-stage tool-boundary guard for sub-agent tool calls.
///
/// Given `(tool_name, args)` it returns `Err(reason)` when the call is NOT
/// allowed in the active harness stage (deny-by-default category whitelist —
/// see `docs/design/2026-06-02-stage-tool-whitelist-enforcement.md`). The
/// sub-agent executor calls it before running each tool and turns `Err` into a
/// synthetic error result instead of executing.
///
/// A plain `Fn` so this crate stays free of harness-crate deps; the runtime
/// builds it from the stage's `allowed_tool_types`.
pub type StageToolGuard = Arc<dyn Fn(&str, &serde_json::Value) -> Result<(), String> + Send + Sync>;

/// Per-stage tool-list filter controlling tool *visibility* (D1).
///
/// Returns `true` when a tool name should be HIDDEN from the sub-agent's tool
/// list for the active harness stage (e.g. scan tools during a no-scan stage
/// like `scoping`). Mirrors the main agent's `hide_scans_for_zero_scan_stage`
/// so a delegated sub-agent never even *sees* a tool it could only be denied —
/// which stops the retry spin where the model hammers a blocked tool. The
/// call-time [`StageToolGuard`] remains the backstop.
///
/// A plain `Fn` so this crate stays free of harness-crate deps; the runtime
/// builds it from the stage's `allowed_tool_types`.
pub type StageToolHider = Arc<dyn Fn(&str) -> bool + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmitRepairKind {
    EvidenceRefs,
    BackgroundJobs,
    CoverageGap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageCapabilitySuggestion {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub tools: Vec<String>,
    pub risk: String,
    pub batchable: bool,
    pub max_batch: usize,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageGapAction {
    pub asset: String,
    pub technique: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggested_capabilities: Vec<StageCapabilitySuggestion>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggested_tools: Vec<String>,
}

/// Exact DB-backed EAS WEB repair input returned by the stage worklist.  The
/// pair is authorization data for the submit-repair guard, not a model hint:
/// object-form wrapper inputs must match both fields and bare URL inputs must
/// match `target_url` exactly.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EasWebRepairTarget {
    pub target_id: String,
    pub target_url: String,
}

/// Deterministic refiner directive produced from `submit_stage_deliverable`
/// `needs_fix`. It is intentionally small and serializable so upper runtime
/// layers can persist it in `operation_state.state_blob.agent_run`, then inject
/// it back into a resumed sub-agent without depending on transient loop memory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitRepairMode {
    pub kind: SubmitRepairKind,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_required_checks: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coverage_gap_actions: Vec<CoverageGapAction>,
    /// `None` means the host-level gate actions have not yet been refined by a
    /// current DB worklist page. `Some`, including an empty vector, means a
    /// DB-backed refresh occurred and is the sole exact-origin authorization
    /// source for EAS WEB calls in this repair turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eas_web_repair_targets: Option<Vec<EasWebRepairTarget>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools_override: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forbidden_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directive_message: Option<String>,
}

impl SubmitRepairMode {
    fn base_allowed_tools(&self) -> &'static [&'static str] {
        match self.kind {
            SubmitRepairKind::EvidenceRefs => &[
                "submit_stage_deliverable",
                "list_recent_evidence",
                "check_stage_asset_coverage",
                "query_target_data",
                "wait_for_background_jobs",
            ],
            SubmitRepairKind::BackgroundJobs => &[
                "check_job",
                "kill_job",
                "check_stage_asset_coverage",
                "submit_stage_deliverable",
            ],
            SubmitRepairKind::CoverageGap if self.coverage_gap_actions.is_empty() => &[
                "list_recent_evidence",
                "check_stage_asset_coverage",
                "query_target_data",
                "wait_for_background_jobs",
                "check_job",
                "kill_job",
                "submit_stage_deliverable",
            ],
            SubmitRepairKind::CoverageGap => &[
                "pentest_list_tools",
                "pentest_run",
                "eas_probe_http_liveness",
                "eas_discover_ports",
                "eas_fingerprint_services",
                "eas_fingerprint_web_stack",
                "vuln_nuclei_general",
                "vuln_nuclei_fingerprint_targeted",
                "vuln_probe_anonymous_access",
                "list_recent_evidence",
                "check_stage_asset_coverage",
                "query_target_data",
                "wait_for_background_jobs",
                "check_job",
                "kill_job",
                "submit_stage_deliverable",
            ],
        }
    }

    fn effective_allowed_tools(&self) -> Vec<String> {
        let mut tools = if self.allowed_tools_override.is_empty() {
            self.base_allowed_tools()
                .iter()
                .map(|tool| (*tool).to_string())
                .collect()
        } else {
            self.allowed_tools_override.clone()
        };
        if self.kind == SubmitRepairKind::CoverageGap {
            append_coverage_gap_worklist_tools(&mut tools, &self.coverage_gap_actions);
            if !self.coverage_gap_actions.is_empty() {
                append_direct_intel_repair_tools(&mut tools, &self.coverage_gap_actions);
                append_direct_enumeration_repair_tools(&mut tools, &self.coverage_gap_actions);
                append_direct_eas_repair_tools(&mut tools, &self.coverage_gap_actions);
                append_direct_vuln_repair_tools(&mut tools, &self.coverage_gap_actions);
                if has_vuln_gap_actions(&self.coverage_gap_actions) {
                    tools.retain(|tool| {
                        !matches!(
                            tool.as_str(),
                            "pentest_run"
                                | "pentest_list_tools"
                                | "wait_for_background_jobs"
                                | "check_job"
                                | "kill_job"
                        )
                    });
                }
            }
        }
        tools
    }

    pub fn kind_str(&self) -> &'static str {
        match self.kind {
            SubmitRepairKind::EvidenceRefs => "evidence_refs",
            SubmitRepairKind::BackgroundJobs => "background_jobs",
            SubmitRepairKind::CoverageGap => "coverage_gap",
        }
    }

    pub fn allowed_tool_names(&self) -> Vec<String> {
        self.effective_allowed_tools()
    }

    pub fn allows(&self, tool_name: &str) -> bool {
        !self.forbidden_tools.iter().any(|tool| tool == tool_name)
            && self
                .effective_allowed_tools()
                .iter()
                .any(|tool| tool == tool_name)
    }

    pub fn model_instruction(&self) -> String {
        let mut message = self.directive_message.clone().unwrap_or_else(|| match self.kind {
                SubmitRepairKind::EvidenceRefs => {
                    "submit_stage_deliverable returned needs_fix for evidence/submit fields, so \
                 repair-only mode is active. Do NOT start fresh discovery or launch new scans. \
                 Rebuild the StageDeliverable from the real evidence ids already returned, use \
                 query_target_data only if you need to map ids to claims, then resubmit."
                        .to_string()
                }
                SubmitRepairKind::BackgroundJobs => {
                    "submit_stage_deliverable reached the exceptional system reconciliation \
                 deadline. Do NOT launch replacement scans or enter a polling loop. Inspect each \
                 listed job at most once with check_job; use kill_job only if it is genuinely \
                 stuck, then resubmit after its terminal notification."
                        .to_string()
                }
                SubmitRepairKind::CoverageGap => {
                    if self.coverage_gap_actions.is_empty() {
                        "submit_stage_deliverable returned needs_fix for coverage, but the gate \
                     did not name any concrete coverage_gap_actions. Repair-only mode is active. \
                     Do NOT launch discovery, guessed-domain probes, pentest_run, CIDR/range \
                     sweeps, or broad rediscovery. Call stage_worklist_status/stage_worklist_next, \
                     check_stage_asset_coverage, or query_target_data to reconcile DB truth; if \
                     there are no in-scope assets or no named gaps, resubmit the \
                     no-asset/terminal deliverable instead of inventing targets."
                            .to_string()
                    } else {
                        "submit_stage_deliverable returned needs_fix because the stage coverage matrix \
                     still has missing or non-terminal cells. Targeted gap-closure mode is active: \
                     use the gate feedback plus read-only stage_worklist_status/stage_worklist_next, \
                     check_stage_asset_coverage, or query_target_data instead of re-listing the \
                     entire attack surface. Run only stage-allowed probes for the exact \
                     asset/technique pairs named in the gate feedback. Batch sibling gap assets with \
                     input_lines/list-file mode when every target is present in coverage_gap_actions. \
                     Vuln-triage general WSTG gaps must use vuln_nuclei_general; WSTG-ATHN-04 \
                     must use vuln_probe_anonymous_access after a complete endpoint review; \
                     GOLISH-NDAY must use vuln_nuclei_fingerprint_targeted. Nuclei calls pass one \
                     server-side target_id, one exact target_url, and explicit techniques[] from \
                     the gap list. Anonymous-access calls pass the same exact target binding, the \
                     complete reviewed_endpoint_ids[] witness, and at most 16 selected_probes[] \
                     entries containing only endpoint_id/query_values/rationale. \
                     Do NOT call list_in_scope_targets, list_attack_surface_seeds, \
                     CIDR/range sweeps, targets outside coverage_gap_actions, or broad rediscovery. \
                     When each named gap has a terminal coverage cell, resubmit."
                            .to_string()
                    }
                }
            });
        if !self.missing_required_checks.is_empty() {
            message.push_str(&format!(
                " Also include these required_checks_done entries if the cited evidence backs \
                 them: [{}].",
                bounded_model_list(&self.missing_required_checks, 32, 128).join(", ")
            ));
        }
        if self.kind == SubmitRepairKind::CoverageGap
            && !self.coverage_gap_actions.is_empty()
            && !message.contains(MODEL_RECOVERY_PROJECTION_MARKER)
        {
            message.push_str(&coverage_gap_action_instruction(&self.coverage_gap_actions));
        }
        cap_recovery_model_text(message)
    }

    pub fn block_result(&self, tool_name: &str) -> Option<serde_json::Value> {
        self.block_result_with_args(tool_name, &serde_json::Value::Null)
    }

    pub fn block_result_with_args(
        &self,
        tool_name: &str,
        tool_args: &serde_json::Value,
    ) -> Option<serde_json::Value> {
        if self.kind == SubmitRepairKind::CoverageGap
            && tool_name == "pentest_run"
            && has_vuln_gap_actions(&self.coverage_gap_actions)
        {
            return Some(self.block_payload(
                tool_name,
                Some(
                    "Vuln-triage coverage-gap repair must use vuln_nuclei_general, vuln_probe_anonymous_access, or vuln_nuclei_fingerprint_targeted, not raw pentest_run"
                        .to_string(),
                ),
            ));
        }
        if self.allows(tool_name) {
            if self.kind == SubmitRepairKind::CoverageGap && tool_name == "pentest_run" {
                if has_eas_gap_actions(&self.coverage_gap_actions) {
                    return Some(self.block_payload(
                        tool_name,
                        Some(
                            "EAS coverage-gap repair must use the eas_* backend wrapper tools, not raw pentest_run"
                                .to_string(),
                        ),
                    ));
                }
                if has_enumeration_gap_actions(&self.coverage_gap_actions) {
                    return Some(self.block_payload(
                        tool_name,
                        Some(
                            "Enumeration coverage-gap repair must use direct enumeration tools, including enum_crawl_same_origin_urls for crawler supplements, not raw pentest_run"
                                .to_string(),
                        ),
                    ));
                }
                if has_vuln_gap_actions(&self.coverage_gap_actions) {
                    return Some(self.block_payload(
                        tool_name,
                        Some(
                            "Vuln-triage coverage-gap repair must use vuln_nuclei_general, vuln_probe_anonymous_access, or vuln_nuclei_fingerprint_targeted, not raw pentest_run"
                                .to_string(),
                        ),
                    ));
                }
                if let Some(reason) = coverage_gap_pentest_run_block_reason(tool_args) {
                    return Some(self.block_payload(tool_name, Some(reason)));
                }
                if let Some(reason) =
                    coverage_gap_action_target_block_reason(tool_args, &self.coverage_gap_actions)
                {
                    return Some(self.block_payload(tool_name, Some(reason)));
                }
            }
            if self.kind == SubmitRepairKind::CoverageGap
                && is_direct_enumeration_repair_tool(tool_name)
            {
                if let Some(reason) = coverage_gap_direct_tool_target_block_reason(
                    tool_name,
                    tool_args,
                    &self.coverage_gap_actions,
                ) {
                    return Some(self.block_payload(tool_name, Some(reason)));
                }
            }
            if self.kind == SubmitRepairKind::CoverageGap && is_direct_eas_repair_tool(tool_name) {
                if let Some(reason) = coverage_gap_eas_wrapper_target_block_reason(
                    tool_name,
                    tool_args,
                    &self.coverage_gap_actions,
                    self.eas_web_repair_targets.as_deref(),
                ) {
                    return Some(self.block_payload(tool_name, Some(reason)));
                }
            }
            if self.kind == SubmitRepairKind::CoverageGap && is_direct_vuln_repair_tool(tool_name) {
                if let Some(reason) = coverage_gap_vuln_wrapper_block_reason(
                    tool_name,
                    tool_args,
                    &self.coverage_gap_actions,
                ) {
                    return Some(self.block_payload(tool_name, Some(reason)));
                }
            }
            return None;
        }
        Some(self.block_payload(tool_name, None))
    }

    fn block_payload(&self, tool_name: &str, blocked_reason: Option<String>) -> serde_json::Value {
        let mut value = serde_json::json!({
            "error": self.model_instruction(),
            "blocked_by_submit_repair": true,
            "repair_kind": self.kind_str(),
            "blocked_tool": bounded_model_field(tool_name, 256),
            "allowed_tools": bounded_model_list(&self.effective_allowed_tools(), 64, 128),
            "last_needs_fix_reason": bounded_model_field(&self.reason, 2_048),
        });
        if let Some(reason) = blocked_reason {
            value["blocked_reason"] =
                serde_json::Value::String(bounded_model_field(&reason, 2_048));
        }
        if !self.coverage_gap_actions.is_empty() {
            value["coverage_gap_actions"] =
                bounded_coverage_gap_projection(&self.coverage_gap_actions);
        }
        if !self.forbidden_tools.is_empty() {
            value["forbidden_tools"] =
                serde_json::to_value(bounded_model_list(&self.forbidden_tools, 64, 128))
                    .unwrap_or_default();
        }
        cap_recovery_block_payload(value)
    }
}

pub(crate) fn coverage_gap_action_instruction(actions: &[CoverageGapAction]) -> String {
    let mut lines = vec![format!(
        "\n{MODEL_RECOVERY_PROJECTION_MARKER}{} stable_hash={} sample_count={}. \
         Only this bounded sample is shown; the complete ordered action set remains enforced \
         internally. Call stage_worklist_next for bounded DB-backed pages; do not infer \
         authorization from this sample. Exact coverage_gap_actions projection from the gate: \
         run ONLY target/technique pairs authorized by the complete guard data. EAS gaps must use \
         eas_probe_http_liveness / eas_discover_ports / eas_fingerprint_services / \
         eas_fingerprint_web_stack instead of raw \
         pentest_run. Vuln-triage general WSTG gaps must use vuln_nuclei_general, WSTG-ATHN-04 \
         must use vuln_probe_anonymous_access after reviewing the complete eligible endpoint set, \
         and GOLISH-NDAY must use vuln_nuclei_fingerprint_targeted. Nuclei calls pass singular \
         target_id + exact target_url + explicit techniques[]. Anonymous-access calls pass the \
         complete reviewed_endpoint_ids[] witness plus a bounded selected_probes[] subset; never \
         raw nuclei, manual authorization probes, pentest_run, or background controls. Direct enumeration tools \
         (browser_collect_js_api/js_extract_apis/route_probe_paths/enum_crawl_same_origin_urls) may be called by \
         name when suggested here; directory discovery must use route_probe_paths, not external \
         ffuf/gobuster/feroxbuster. Bounded crawler URL supplements must use \
         enum_crawl_same_origin_urls, not raw katana or pentest_run. For EAS \
         WEB-FINGERPRINT gaps, copy details.recommended_args.target_urls directly when present, \
         or pair the work item target_id with each exact details.missing_origins value. Never \
         guess or rewrite an origin scheme from its port.",
        actions.len(),
        stable_action_hash(actions),
        actions.len().min(MODEL_RECOVERY_ACTION_SAMPLE_LIMIT)
    )];
    for (idx, action) in actions
        .iter()
        .take(MODEL_RECOVERY_ACTION_SAMPLE_LIMIT)
        .enumerate()
    {
        let capabilities = if action.suggested_capabilities.is_empty() {
            String::new()
        } else {
            let ids = action
                .suggested_capabilities
                .iter()
                .take(5)
                .map(|capability| bounded_model_field(&capability.id, 128))
                .collect::<Vec<_>>()
                .join(", ");
            format!("; suggested_capabilities={ids}")
        };
        let tools = if action.suggested_tools.is_empty() {
            String::new()
        } else {
            format!(
                "; suggested_tools={}",
                bounded_model_list(&action.suggested_tools, 5, 128).join(", ")
            )
        };
        lines.push(format!(
            "{}. asset={} technique={} reason={}{}{}",
            idx + 1,
            bounded_model_field(&action.asset, 512),
            bounded_model_field(&action.technique, 256),
            bounded_model_field(&action.reason, 512),
            capabilities,
            tools
        ));
    }
    cap_recovery_model_text(lines.join("\n"))
}

fn bounded_coverage_gap_projection(actions: &[CoverageGapAction]) -> serde_json::Value {
    let sample = actions
        .iter()
        .take(MODEL_RECOVERY_ACTION_SAMPLE_LIMIT)
        .map(|action| {
            serde_json::json!({
                "asset": bounded_model_field(&action.asset, 256),
                "technique": bounded_model_field(&action.technique, 128),
                "reason": bounded_model_field(&action.reason, 256),
                "suggested_capability_ids": action
                    .suggested_capabilities
                    .iter()
                    .take(3)
                    .map(|capability| bounded_model_field(&capability.id, 96))
                    .collect::<Vec<_>>(),
                "suggested_tools": bounded_model_list(&action.suggested_tools, 3, 96),
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "total": actions.len(),
        "stable_hash": stable_action_hash(actions),
        "sample_count": sample.len(),
        "sample": sample,
        "omitted": actions.len().saturating_sub(MODEL_RECOVERY_ACTION_SAMPLE_LIMIT),
        "next_page_tool": "stage_worklist_next",
        "authorization_note": "The full ordered action set remains enforced internally; this sample is not the authorization source.",
    })
}

fn stable_action_hash<T: Serialize + ?Sized>(value: &T) -> String {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x00000100000001b3;

    let mut hash = FNV_OFFSET_BASIS;
    for byte in serde_json::to_vec(value).unwrap_or_default() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("fnv1a64:{hash:016x}")
}

fn bounded_model_list(values: &[String], limit: usize, field_max_bytes: usize) -> Vec<String> {
    let mut sample = values
        .iter()
        .take(limit)
        .map(|value| bounded_model_field(value, field_max_bytes))
        .collect::<Vec<_>>();
    if values.len() > sample.len() {
        sample.push(format!("... +{} more", values.len() - sample.len()));
    }
    sample
}

fn bounded_model_field(value: &str, max_bytes: usize) -> String {
    let sanitized = value
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>();
    if sanitized.len() <= max_bytes {
        return sanitized;
    }
    const SUFFIX: &str = "...[truncated]";
    let prefix_budget = max_bytes.saturating_sub(SUFFIX.len());
    let end = utf8_boundary_at_or_before(&sanitized, prefix_budget);
    format!("{}{}", &sanitized[..end], SUFFIX)
}

fn cap_recovery_model_text(mut value: String) -> String {
    if value.len() <= MODEL_RECOVERY_INSTRUCTION_MAX_BYTES {
        return value;
    }
    let prefix_budget =
        MODEL_RECOVERY_INSTRUCTION_MAX_BYTES.saturating_sub(MODEL_RECOVERY_TRUNCATION_SUFFIX.len());
    let end = utf8_boundary_at_or_before(&value, prefix_budget);
    value.truncate(end);
    value.push_str(MODEL_RECOVERY_TRUNCATION_SUFFIX);
    value
}

fn cap_recovery_block_payload(mut value: serde_json::Value) -> serde_json::Value {
    if encoded_json_len(&value) <= MODEL_RECOVERY_BLOCK_PAYLOAD_MAX_BYTES {
        return value;
    }

    if let Some(projection) = value
        .get_mut("coverage_gap_actions")
        .and_then(serde_json::Value::as_object_mut)
    {
        truncate_projection_sample(projection, 5);
    }
    let bounded_error = value
        .get("error")
        .and_then(serde_json::Value::as_str)
        .map(|error| bounded_model_field(error, 16 * 1024));
    if let Some(error) = bounded_error {
        value["error"] = serde_json::Value::String(error);
    }
    if encoded_json_len(&value) <= MODEL_RECOVERY_BLOCK_PAYLOAD_MAX_BYTES {
        return value;
    }

    if let Some(projection) = value
        .get_mut("coverage_gap_actions")
        .and_then(serde_json::Value::as_object_mut)
    {
        truncate_projection_sample(projection, 0);
    }
    value["error"] = serde_json::Value::String(
        "Submit repair blocked this tool. Use stage_worklist_next for bounded DB-backed recovery pages."
            .to_string(),
    );
    if let Some(object) = value.as_object_mut() {
        object.remove("last_needs_fix_reason");
        object.remove("blocked_reason");
        object.remove("allowed_tools");
        object.remove("forbidden_tools");
    }
    value
}

fn truncate_projection_sample(
    projection: &mut serde_json::Map<String, serde_json::Value>,
    limit: usize,
) {
    let total = projection
        .get("total")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default() as usize;
    let sample_len = projection
        .get_mut("sample")
        .and_then(serde_json::Value::as_array_mut)
        .map(|sample| {
            sample.truncate(limit);
            sample.len()
        });
    if let Some(sample_len) = sample_len {
        projection.insert("sample_count".to_string(), serde_json::json!(sample_len));
        projection.insert(
            "omitted".to_string(),
            serde_json::json!(total.saturating_sub(sample_len)),
        );
    }
}

fn encoded_json_len(value: &serde_json::Value) -> usize {
    serde_json::to_vec(value).map_or(0, |encoded| encoded.len())
}

fn utf8_boundary_at_or_before(value: &str, max_bytes: usize) -> usize {
    let mut end = max_bytes.min(value.len());
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    end
}

fn append_direct_intel_repair_tools(tools: &mut Vec<String>, actions: &[CoverageGapAction]) {
    for action in actions
        .iter()
        .filter(|action| action.technique.starts_with("GOLISH-INTEL-"))
    {
        for suggested in &action.suggested_tools {
            let suggested = suggested
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if matches!(
                suggested.as_str(),
                "recon_map_assets" | "recon_lookup_whois"
            ) {
                push_unique_tool(tools, &suggested);
            }
        }
        if action.technique == "GOLISH-INTEL-WHOIS" {
            push_unique_tool(tools, "recon_lookup_whois");
        }
    }
}

fn append_direct_enumeration_repair_tools(tools: &mut Vec<String>, actions: &[CoverageGapAction]) {
    if has_enumeration_gap_actions(actions) {
        push_unique_tool(tools, "enum_preflight_web_origins");
    }
    for action in actions {
        for suggested in &action.suggested_tools {
            let suggested = suggested
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if is_direct_enumeration_repair_tool(&suggested) {
                push_unique_tool(tools, &suggested);
            }
            if suggested == "katana" {
                push_unique_tool(tools, "enum_crawl_same_origin_urls");
            }
        }
        match action.technique.as_str() {
            "GOLISH-ENUM-JSAPI" => {
                push_unique_tool(tools, "browser_collect_js_api");
                push_unique_tool(tools, "js_extract_apis");
                push_unique_tool(tools, "enum_crawl_same_origin_urls");
            }
            "GOLISH-ENUM-DIR" => push_unique_tool(tools, "route_probe_paths"),
            "GOLISH-ENUM-PARAM" => {
                push_unique_tool(tools, "js_extract_apis");
                push_unique_tool(tools, "enum_crawl_same_origin_urls");
            }
            "GOLISH-ENUM-JS" => push_unique_tool(tools, "browser_collect_js_api"),
            _ => {}
        }
    }
}

fn append_direct_eas_repair_tools(tools: &mut Vec<String>, actions: &[CoverageGapAction]) {
    for action in actions {
        for suggested in &action.suggested_tools {
            let suggested = suggested
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if is_direct_eas_repair_tool(&suggested) {
                push_unique_tool(tools, &suggested);
            }
            if suggested == "whatweb" {
                push_unique_tool(tools, "eas_fingerprint_web_stack");
            }
        }
        match action.technique.as_str() {
            "GOLISH-EAS-LIVENESS" => push_unique_tool(tools, "eas_probe_http_liveness"),
            "GOLISH-EAS-PORT" => push_unique_tool(tools, "eas_discover_ports"),
            "GOLISH-EAS-SERVICE-FINGERPRINT" => push_unique_tool(tools, "eas_fingerprint_services"),
            "GOLISH-EAS-WEB-FINGERPRINT" => push_unique_tool(tools, "eas_fingerprint_web_stack"),
            _ => {}
        }
    }
}

fn append_direct_vuln_repair_tools(tools: &mut Vec<String>, actions: &[CoverageGapAction]) {
    for action in actions {
        for suggested in &action.suggested_tools {
            let suggested = suggested
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if is_direct_vuln_repair_tool(&suggested) {
                push_unique_tool(tools, &suggested);
            }
        }
        if action.technique == "GOLISH-NDAY" {
            push_unique_tool(tools, "vuln_nuclei_fingerprint_targeted");
        } else if action.technique == "WSTG-ATHN-04" {
            push_unique_tool(tools, "vuln_probe_anonymous_access");
        } else if action.technique.starts_with("WSTG-") {
            push_unique_tool(tools, "vuln_nuclei_general");
        }
    }
}

fn append_coverage_gap_worklist_tools(tools: &mut Vec<String>, actions: &[CoverageGapAction]) {
    push_unique_tool(tools, "stage_worklist_status");
    push_unique_tool(tools, "stage_worklist_next");
    push_unique_tool(tools, "list_recent_evidence");
    if has_enumeration_gap_actions(actions) {
        push_unique_tool(tools, "list_enumeration_web_roots");
    }
}

fn push_unique_tool(tools: &mut Vec<String>, tool: &str) {
    if !tool.is_empty() && !tools.iter().any(|existing| existing == tool) {
        tools.push(tool.to_string());
    }
}

fn is_direct_enumeration_repair_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "browser_collect_js_api"
            | "js_extract_apis"
            | "route_probe_paths"
            | "enum_crawl_same_origin_urls"
            | "enum_preflight_web_origins"
    )
}

fn is_direct_eas_repair_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "eas_probe_http_liveness"
            | "eas_discover_ports"
            | "eas_fingerprint_services"
            | "eas_fingerprint_web_stack"
    )
}

fn is_direct_vuln_repair_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "vuln_nuclei_general" | "vuln_nuclei_fingerprint_targeted" | "vuln_probe_anonymous_access"
    )
}

fn has_eas_gap_actions(actions: &[CoverageGapAction]) -> bool {
    actions
        .iter()
        .any(|action| action.technique.starts_with("GOLISH-EAS-"))
}

fn has_enumeration_gap_actions(actions: &[CoverageGapAction]) -> bool {
    actions
        .iter()
        .any(|action| action.technique.starts_with("GOLISH-ENUM-"))
}

fn has_vuln_gap_actions(actions: &[CoverageGapAction]) -> bool {
    actions
        .iter()
        .any(|action| action.technique.starts_with("WSTG-") || action.technique == "GOLISH-NDAY")
}

fn coverage_gap_eas_wrapper_target_block_reason(
    tool_name: &str,
    tool_args: &serde_json::Value,
    actions: &[CoverageGapAction],
    eas_web_repair_targets: Option<&[EasWebRepairTarget]>,
) -> Option<String> {
    if actions.is_empty() {
        return None;
    }
    let batch_key = if tool_name == "eas_fingerprint_web_stack" {
        "target_urls"
    } else {
        "targets"
    };

    if tool_name == "eas_fingerprint_web_stack" {
        return coverage_gap_eas_web_target_block_reason(
            tool_args,
            actions,
            eas_web_repair_targets,
        );
    }

    let mut targets = Vec::new();
    if let Some(batch) = tool_args.get(batch_key).and_then(|v| v.as_array()) {
        for item in batch {
            if let Some(candidate) = item.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                targets.push(candidate.to_string());
            }
        }
    }
    if targets.is_empty() {
        return Some(format!(
            "coverage-gap repair requires {tool_name} {batch_key}[] so they can be checked against coverage_gap_actions"
        ));
    }
    let allowed = actions
        .iter()
        .filter(|action| action.technique.starts_with("GOLISH-EAS-"))
        .map(|action| normalize_probe_target(&action.asset))
        .filter(|asset| !asset.is_empty())
        .collect::<HashSet<_>>();
    if allowed.is_empty() {
        return Some(format!(
            "coverage-gap repair blocks {tool_name} because no EAS coverage_gap_actions are active"
        ));
    }
    let blocked = targets
        .iter()
        .find(|target| !allowed.contains(&normalize_probe_target(target)))?;
    Some(format!(
        "coverage-gap repair blocks {tool_name} target '{blocked}' because it is not in the EAS coverage_gap_actions"
    ))
}

#[derive(Debug)]
enum EasWebWrapperTarget<'a> {
    Bare(&'a str),
    Bound {
        target_id: &'a str,
        target_url: &'a str,
    },
}

fn coverage_gap_eas_web_target_block_reason(
    tool_args: &serde_json::Value,
    actions: &[CoverageGapAction],
    eas_web_repair_targets: Option<&[EasWebRepairTarget]>,
) -> Option<String> {
    let Some(batch) = tool_args
        .get("target_urls")
        .and_then(|value| value.as_array())
    else {
        return Some(
            "coverage-gap repair requires eas_fingerprint_web_stack target_urls[] so exact DB-backed origins can be checked"
                .to_string(),
        );
    };
    if batch.is_empty() {
        return Some(
            "coverage-gap repair requires eas_fingerprint_web_stack target_urls[] so exact DB-backed origins can be checked"
                .to_string(),
        );
    }

    let mut requested = Vec::with_capacity(batch.len());
    for item in batch {
        if let Some(target_url) = item
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            requested.push(EasWebWrapperTarget::Bare(target_url));
            continue;
        }
        let target_id = item
            .get("target_id")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let target_url = item
            .get("target_url")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let (Some(target_id), Some(target_url)) = (target_id, target_url) else {
            return Some(
                "coverage-gap repair requires every eas_fingerprint_web_stack object entry to contain the exact DB-backed target_id and target_url"
                    .to_string(),
            );
        };
        requested.push(EasWebWrapperTarget::Bound {
            target_id,
            target_url,
        });
    }

    // A refreshed worklist/check-coverage page is authoritative even when it
    // contains no WEB rows. Host-level coverage actions must never broaden it.
    if let Some(authoritative) = eas_web_repair_targets {
        let allowed_pairs = authoritative
            .iter()
            .filter_map(|target| {
                Some((
                    target.target_id.trim(),
                    normalize_exact_web_origin(&target.target_url)?,
                ))
            })
            .collect::<HashSet<_>>();
        let allowed_urls = allowed_pairs
            .iter()
            .map(|(_, target_url)| target_url.clone())
            .collect::<HashSet<_>>();

        for target in requested {
            let blocked = match target {
                EasWebWrapperTarget::Bare(target_url) => normalize_exact_web_origin(target_url)
                    .is_none_or(|target_url| !allowed_urls.contains(&target_url)),
                EasWebWrapperTarget::Bound {
                    target_id,
                    target_url,
                } => normalize_exact_web_origin(target_url).is_none_or(|target_url| {
                    !allowed_pairs.contains(&(target_id.trim(), target_url))
                }),
            };
            if blocked {
                let display = match target {
                    EasWebWrapperTarget::Bare(target_url) => target_url,
                    EasWebWrapperTarget::Bound { target_url, .. } => target_url,
                };
                return Some(format!(
                    "coverage-gap repair blocks eas_fingerprint_web_stack target '{display}' because its exact target_id/origin pair is not in the current DB-backed stage worklist"
                ));
            }
        }
        return None;
    }

    // Some legacy gates already name an exact absolute origin as the action
    // asset. Preserve that narrow authority for bare URL input only. A host/IP
    // action has no scheme/port identity, and object input has no trusted ID,
    // so both must refresh the DB worklist before active scanning.
    let exact_action_urls = actions
        .iter()
        .filter(|action| action.technique == "GOLISH-EAS-WEB-FINGERPRINT")
        .filter_map(|action| normalize_exact_web_origin(&action.asset))
        .collect::<HashSet<_>>();
    if exact_action_urls.is_empty() {
        return Some(
            "coverage-gap repair has only host-level WEB actions; call stage_worklist_next (or check_stage_asset_coverage) before eas_fingerprint_web_stack so exact DB-backed target_id/target_url pairs can be enforced"
                .to_string(),
        );
    }
    for target in requested {
        let EasWebWrapperTarget::Bare(target_url) = target else {
            return Some(
                "coverage-gap repair cannot authorize an object-form EAS WEB target from a host-level action; refresh stage_worklist_next for its exact DB-backed target_id/target_url pair"
                    .to_string(),
            );
        };
        if normalize_exact_web_origin(target_url)
            .is_none_or(|target_url| !exact_action_urls.contains(&target_url))
        {
            return Some(format!(
                "coverage-gap repair blocks eas_fingerprint_web_stack target '{target_url}' because it is not an exact EAS WEB origin named by the gate"
            ));
        }
    }
    None
}

fn normalize_exact_web_origin(value: &str) -> Option<String> {
    let value = value.trim().trim_end_matches('/');
    let authority = value
        .strip_prefix("http://")
        .or_else(|| value.strip_prefix("https://"))?;
    if authority.is_empty() || authority.contains(['/', '?', '#']) {
        return None;
    }
    Some(value.to_ascii_lowercase())
}

fn coverage_gap_vuln_wrapper_block_reason(
    tool_name: &str,
    tool_args: &serde_json::Value,
    actions: &[CoverageGapAction],
) -> Option<String> {
    if actions.is_empty() {
        return None;
    }
    let allowed_actions = actions
        .iter()
        .filter(|action| match tool_name {
            "vuln_nuclei_general" => {
                action.technique.starts_with("WSTG-") && action.technique != "WSTG-ATHN-04"
            }
            "vuln_nuclei_fingerprint_targeted" => action.technique == "GOLISH-NDAY",
            "vuln_probe_anonymous_access" => action.technique == "WSTG-ATHN-04",
            _ => false,
        })
        .collect::<Vec<_>>();
    if allowed_actions.is_empty() {
        return Some(format!(
            "coverage-gap repair blocks {tool_name} because no compatible vuln_triage coverage_gap_actions are active"
        ));
    }

    let target_id = tool_args
        .get("target_id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if target_id.is_none() {
        return Some(format!(
            "coverage-gap repair requires {tool_name} target_id so the backend can revalidate current ownership"
        ));
    }
    let target_url = tool_args
        .get("target_url")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(target_url) = target_url else {
        return Some(format!(
            "coverage-gap repair requires {tool_name} target_url so the exact web origin can be checked against coverage_gap_actions"
        ));
    };
    let Some(normalized_target_url) = normalize_exact_web_origin(target_url) else {
        return Some(format!(
            "coverage-gap repair requires {tool_name} target_url to be an exact absolute http(s) origin"
        ));
    };

    let allowed_pairs = allowed_actions
        .iter()
        .filter_map(|action| {
            Some((
                normalize_exact_web_origin(&action.asset)?,
                action.technique.as_str(),
            ))
        })
        .collect::<HashSet<_>>();

    if tool_name == "vuln_probe_anonymous_access" {
        if !allowed_pairs.contains(&(normalized_target_url, "WSTG-ATHN-04")) {
            return Some(format!(
                "coverage-gap repair blocks {tool_name} target '{target_url}' because that exact WSTG-ATHN-04 origin is not in vuln_triage coverage_gap_actions"
            ));
        }
        return anonymous_access_repair_args_block_reason(tool_args);
    }

    let techniques = string_array_arg(tool_args, "techniques");
    if tool_name == "vuln_nuclei_fingerprint_targeted"
        && !(techniques.len() == 1 && techniques[0] == "GOLISH-NDAY")
    {
        return Some(
            "coverage-gap repair requires vuln_nuclei_fingerprint_targeted techniques to be exactly [\"GOLISH-NDAY\"]"
                .to_string(),
        );
    }
    if techniques.is_empty() {
        return Some(format!(
            "coverage-gap repair requires {tool_name} techniques[]; default-all is too broad for targeted gap repair"
        ));
    }

    let allowed_techniques = allowed_actions
        .iter()
        .map(|action| action.technique.as_str())
        .collect::<HashSet<_>>();
    if let Some(blocked) = techniques
        .iter()
        .find(|technique| !allowed_techniques.contains(technique.as_str()))
    {
        return Some(format!(
            "coverage-gap repair blocks {tool_name} target/technique pair for '{blocked}' because that technique is not in compatible vuln_triage coverage_gap_actions"
        ));
    }
    for technique in &techniques {
        if !allowed_pairs.contains(&(normalized_target_url.clone(), technique.as_str())) {
            return Some(format!(
                "coverage-gap repair blocks {tool_name} pair '{} × {}' because that exact target/technique pair is not in vuln_triage coverage_gap_actions",
                target_url, technique
            ));
        }
    }
    None
}

fn anonymous_access_repair_args_block_reason(tool_args: &serde_json::Value) -> Option<String> {
    let Some(args) = tool_args.as_object() else {
        return Some(
            "coverage-gap repair requires vuln_probe_anonymous_access arguments to be an object"
                .to_string(),
        );
    };
    if let Some(forbidden) = args.keys().find(|key| {
        !matches!(
            key.as_str(),
            "target_id"
                | "target_url"
                | "reviewed_endpoint_ids"
                | "selected_probes"
                | "timeout_secs"
                | "__harness_org_id"
        )
    }) {
        return Some(format!(
            "coverage-gap repair blocks unsupported anonymous-access request control '{forbidden}'"
        ));
    }

    let Some(reviewed_values) = args
        .get("reviewed_endpoint_ids")
        .and_then(serde_json::Value::as_array)
    else {
        return Some(
            "coverage-gap repair requires reviewed_endpoint_ids[] as the complete eligible endpoint review witness"
                .to_string(),
        );
    };
    if reviewed_values.len() > 5_000 {
        return Some(
            "coverage-gap repair blocks reviewed_endpoint_ids[] above the 5000-endpoint review bound"
                .to_string(),
        );
    }
    let mut reviewed_ids = HashSet::with_capacity(reviewed_values.len());
    for value in reviewed_values {
        let Some(endpoint_id) = value.as_str() else {
            return Some(
                "coverage-gap repair requires reviewed_endpoint_ids[] to contain only UUID strings"
                    .to_string(),
            );
        };
        let Ok(endpoint_id) = uuid::Uuid::parse_str(endpoint_id) else {
            return Some(
                "coverage-gap repair requires reviewed_endpoint_ids[] to contain only UUID strings"
                    .to_string(),
            );
        };
        if !reviewed_ids.insert(endpoint_id) {
            return Some(
                "coverage-gap repair requires reviewed_endpoint_ids[] to be duplicate-free"
                    .to_string(),
            );
        }
    }

    let Some(selected_values) = args
        .get("selected_probes")
        .and_then(serde_json::Value::as_array)
    else {
        return Some(
            "coverage-gap repair requires selected_probes[] after reviewing the complete eligible endpoint set"
                .to_string(),
        );
    };
    if selected_values.len() > 16 {
        return Some(
            "coverage-gap repair blocks selected_probes[] above the 16-probe execution bound"
                .to_string(),
        );
    }

    let mut selected_ids = HashSet::with_capacity(selected_values.len());
    for value in selected_values {
        let Some(probe) = value.as_object() else {
            return Some(
                "coverage-gap repair requires every selected_probes[] entry to be an object"
                    .to_string(),
            );
        };
        if probe.len() != 3
            || !probe.contains_key("endpoint_id")
            || !probe.contains_key("query_values")
            || !probe.contains_key("rationale")
        {
            return Some(
                "coverage-gap repair allows only endpoint_id, query_values, and rationale in each selected_probes[] entry"
                    .to_string(),
            );
        }
        let Some(endpoint_id) = probe.get("endpoint_id").and_then(serde_json::Value::as_str) else {
            return Some(
                "coverage-gap repair requires selected_probes[].endpoint_id to be a UUID string"
                    .to_string(),
            );
        };
        let Ok(endpoint_id) = uuid::Uuid::parse_str(endpoint_id) else {
            return Some(
                "coverage-gap repair requires selected_probes[].endpoint_id to be a UUID string"
                    .to_string(),
            );
        };
        if !reviewed_ids.contains(&endpoint_id) || !selected_ids.insert(endpoint_id) {
            return Some(
                "coverage-gap repair requires selected_probes[] to be a unique subset of reviewed_endpoint_ids[]"
                    .to_string(),
            );
        }

        let Some(query_values) = probe
            .get("query_values")
            .and_then(serde_json::Value::as_object)
        else {
            return Some(
                "coverage-gap repair requires selected_probes[].query_values to be an object"
                    .to_string(),
            );
        };
        if query_values.len() > 16
            || query_values.iter().any(|(key, value)| {
                key.is_empty()
                    || key.len() > 128
                    || value
                        .as_str()
                        .is_none_or(|value| !is_safe_anonymous_query_scalar(value))
            })
        {
            return Some(
                "coverage-gap repair requires query_values to contain at most 16 known query names with short safe scalar strings"
                    .to_string(),
            );
        }

        let rationale = probe
            .get("rationale")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if rationale
            .is_none_or(|value| value.chars().count() > 512 || value.chars().any(char::is_control))
        {
            return Some(
                "coverage-gap repair requires each selected probe to have a bounded non-empty rationale"
                    .to_string(),
            );
        }
    }

    if let Some(timeout) = args.get("timeout_secs") {
        if timeout
            .as_u64()
            .is_none_or(|timeout| !(5..=120).contains(&timeout))
        {
            return Some(
                "coverage-gap repair requires timeout_secs to be an integer between 5 and 120"
                    .to_string(),
            );
        }
    }
    None
}

fn is_safe_anonymous_query_scalar(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed == value
        && trimmed.len() <= 64
        && trimmed.is_ascii()
        && !trimmed.contains("..")
        && trimmed.parse::<std::net::IpAddr>().is_err()
        && trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~'))
}

fn coverage_gap_pentest_run_block_reason(tool_args: &serde_json::Value) -> Option<String> {
    let args = pentest_run_args(tool_args);

    let tool = tool_args
        .get("tool_name")
        .or_else(|| tool_args.get("tool"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(
        tool.as_str(),
        "" | "httpx" | "nmap" | "naabu" | "masscan" | "whatweb"
    ) {
        return None;
    }

    if contains_cidr_target(&args) || contains_cidr_target_values(&input_line_targets(tool_args)) {
        return Some(
            "coverage-gap repair blocks CIDR/range sweeps; probe one exact named asset".to_string(),
        );
    }
    None
}

fn coverage_gap_direct_tool_target_block_reason(
    tool_name: &str,
    tool_args: &serde_json::Value,
    actions: &[CoverageGapAction],
) -> Option<String> {
    if actions.is_empty() {
        return None;
    }
    // Single + batch inputs (design 2026-07-03): route_probe_paths uses
    // base_url / targets[].base_url; the JS tools use target_url / target_urls[].
    let (single_key, batch_key) = match tool_name {
        "route_probe_paths" => ("base_url", "targets"),
        "browser_collect_js_api" | "js_extract_apis" => ("target_url", "target_urls"),
        "enum_crawl_same_origin_urls" => ("target_url", "target_urls"),
        "enum_preflight_web_origins" => ("", "origins"),
        _ => return None,
    };
    let mut targets: Vec<String> = Vec::new();
    if let Some(single) = tool_args.get(single_key).and_then(|v| v.as_str()) {
        let single = single.trim();
        if !single.is_empty() {
            targets.push(single.to_string());
        }
    }
    if let Some(batch) = tool_args.get(batch_key).and_then(|v| v.as_array()) {
        for item in batch {
            // route_probe_paths batch entries are objects with a base_url; JS
            // batch entries may be bare strings or worklist objects carrying
            // target_id + target_url/root_url/base_url/url.
            let candidate = item
                .as_str()
                .or_else(|| item.get("target_url").and_then(|v| v.as_str()))
                .or_else(|| item.get("root_url").and_then(|v| v.as_str()))
                .or_else(|| item.get("base_url").and_then(|v| v.as_str()))
                .or_else(|| item.get("url").and_then(|v| v.as_str()));
            if let Some(candidate) = candidate.map(str::trim).filter(|s| !s.is_empty()) {
                targets.push(candidate.to_string());
            }
        }
    }
    if targets.is_empty() {
        return Some(format!(
            "coverage-gap repair requires {tool_name} target(s) ({single_key} or {batch_key}) so they can be checked against coverage_gap_actions"
        ));
    }
    let allowed = actions
        .iter()
        .map(|action| normalize_probe_target(&action.asset))
        .filter(|asset| !asset.is_empty())
        .collect::<HashSet<_>>();
    // Any single target outside the named gaps blocks the whole call — a batch
    // must not smuggle an un-named target past the coverage-gap fence.
    let blocked = targets
        .iter()
        .find(|target| !allowed.contains(&normalize_probe_target(target)))?;
    let mut preview = actions
        .iter()
        .take(20)
        .map(|action| format!("{} × {}", action.asset, action.technique))
        .collect::<Vec<_>>();
    if actions.len() > preview.len() {
        preview.push(format!("... +{} more", actions.len() - preview.len()));
    }
    Some(format!(
        "coverage-gap repair blocks {tool_name} target '{blocked}' because it is not in coverage_gap_actions; only probe [{}]",
        preview.join(", ")
    ))
}

fn coverage_gap_action_target_block_reason(
    tool_args: &serde_json::Value,
    actions: &[CoverageGapAction],
) -> Option<String> {
    if actions.is_empty() {
        return None;
    }
    let args = pentest_run_args(tool_args);
    if args.is_empty() {
        return None;
    }
    let targets = probe_targets_from_tool_args(tool_args);
    if targets.is_empty() {
        if uses_hidden_target_file(&args) {
            return Some(
                "coverage-gap repair list-file/stdin probes must provide input_lines/stdin so targets can be checked against coverage_gap_actions"
                    .to_string(),
            );
        }
        return None;
    }
    let allowed = actions
        .iter()
        .map(|action| normalize_probe_target(&action.asset))
        .filter(|asset| !asset.is_empty())
        .collect::<HashSet<_>>();
    let blocked = targets
        .iter()
        .find(|target| !allowed.contains(&normalize_probe_target(target)))?;
    let mut preview = actions
        .iter()
        .take(20)
        .map(|action| format!("{} × {}", action.asset, action.technique))
        .collect::<Vec<_>>();
    if actions.len() > preview.len() {
        preview.push(format!("... +{} more", actions.len() - preview.len()));
    }
    Some(format!(
        "coverage-gap repair blocks target '{blocked}' because it is not in coverage_gap_actions; only probe [{}]",
        preview.join(", ")
    ))
}

fn pentest_run_args(tool_args: &serde_json::Value) -> String {
    tool_args
        .get("args")
        .or_else(|| tool_args.get("arguments"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn probe_targets_from_tool_args(tool_args: &serde_json::Value) -> Vec<String> {
    let mut targets = probe_targets(&pentest_run_args(tool_args));
    targets.extend(input_line_targets(tool_args));
    targets.sort();
    targets.dedup();
    targets
}

fn input_line_targets(tool_args: &serde_json::Value) -> Vec<String> {
    let mut targets = Vec::new();
    if let Some(lines) = tool_args.get("input_lines").and_then(|v| v.as_array()) {
        for line in lines {
            if let Some(line) = line.as_str() {
                targets.extend(probe_targets(line));
            }
        }
    }
    if let Some(stdin) = tool_args.get("stdin").and_then(|v| v.as_str()) {
        for line in stdin.lines() {
            targets.extend(probe_targets(line));
        }
    }
    targets
}

fn string_array_arg(tool_args: &serde_json::Value, key: &str) -> Vec<String> {
    tool_args
        .get(key)
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::trim))
                .filter(|item| !item.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn uses_hidden_target_file(args: &str) -> bool {
    let args_lc = args.to_ascii_lowercase();
    args_lc.contains("<<")
        || args_lc.contains("/dev/stdin")
        || args_lc.contains(" -l ")
        || args_lc.starts_with("-l ")
        || args_lc.contains(" --list ")
        || args_lc.starts_with("--list ")
        || args_lc.contains(" -list ")
        || args_lc.starts_with("-list ")
        || args_lc.contains(" -il ")
        || args_lc.starts_with("-il ")
        || args_lc.contains(" --input-file")
        || args_lc.starts_with("--input-file")
}

fn contains_cidr_target(args: &str) -> bool {
    args.split_whitespace().any(|token| {
        let token = clean_probe_token(token);
        let Some((host, prefix)) = token.split_once('/') else {
            return false;
        };
        looks_like_ipv4(host) && prefix.parse::<u8>().is_ok_and(|bits| bits <= 32)
    })
}

fn contains_cidr_target_values(values: &[String]) -> bool {
    values.iter().any(|value| contains_cidr_target(value))
}

fn probe_targets(args: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut skip_next = false;
    for raw in args.split_whitespace() {
        let token = clean_probe_token(raw);
        if token.is_empty() {
            continue;
        }
        let token_lc = token.to_ascii_lowercase();
        if skip_next {
            skip_next = false;
            continue;
        }
        if option_takes_value(&token_lc) {
            skip_next = true;
            continue;
        }
        if token.starts_with('-') {
            continue;
        }
        if is_probe_target_token(&token_lc) {
            targets.push(token.to_string());
        }
    }
    targets
}

fn option_takes_value(token_lc: &str) -> bool {
    matches!(
        token_lc,
        "-o" | "-oa"
            | "-og"
            | "-on"
            | "-ox"
            | "-ol"
            | "-p"
            | "-ports"
            | "--ports"
            | "--top-ports"
            | "--rate"
            | "-rate"
            | "-t"
            | "-timeout"
            | "--timeout"
            | "-output"
            | "--output"
    )
}

fn is_probe_target_token(token_lc: &str) -> bool {
    if matches!(
        token_lc,
        "http" | "https" | "tcp" | "udp" | "true" | "false"
    ) {
        return false;
    }
    token_lc.starts_with("http://")
        || token_lc.starts_with("https://")
        || looks_like_ipv4(token_lc)
        || (token_lc.contains('.') && !looks_like_output_path(token_lc))
}

fn looks_like_output_path(token_lc: &str) -> bool {
    matches!(
        token_lc.rsplit('.').next(),
        Some("out" | "txt" | "json" | "xml" | "csv" | "log")
    )
}

pub(crate) fn normalize_probe_target(value: &str) -> String {
    let mut s = clean_probe_token(value).trim().to_ascii_lowercase();
    if let Some(rest) = s.strip_prefix("http://") {
        s = rest.to_string();
    } else if let Some(rest) = s.strip_prefix("https://") {
        s = rest.to_string();
    }
    if let Some(idx) = s.find(['/', '?', '#']) {
        s.truncate(idx);
    }
    if let Some(idx) = s.rfind('@') {
        s = s[idx + 1..].to_string();
    }
    if let Some((host, port)) = s.rsplit_once(':') {
        if !host.contains(':') && port.chars().all(|c| c.is_ascii_digit()) {
            s = host.to_string();
        }
    }
    s.trim_end_matches('.').to_string()
}

fn clean_probe_token(token: &str) -> &str {
    token.trim_matches(|c: char| {
        matches!(
            c,
            '\'' | '"' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}'
        )
    })
}

fn looks_like_ipv4(value: &str) -> bool {
    let mut parts = value.split('.');
    let mut count = 0;
    for part in &mut parts {
        count += 1;
        if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
        if part.parse::<u8>().is_err() {
            return false;
        }
    }
    count == 4
}

/// Context needed for sub-agent execution.
pub struct SubAgentExecutorContext<'a> {
    pub event_tx: &'a mpsc::UnboundedSender<AiEvent>,
    pub tool_registry: &'a Arc<RwLock<ToolRegistry>>,
    pub workspace: &'a Arc<RwLock<std::path::PathBuf>>,
    /// Provider name (e.g., "openai", "anthropic_vertex") for model capability checks
    pub provider_name: &'a str,
    /// Model name for model capability checks
    pub model_name: &'a str,
    /// Session ID for Langfuse tracing (propagated from parent agent)
    pub session_id: Option<&'a str>,
    /// Stable database session primary key used by message-chain persistence.
    /// This is deliberately separate from `session_id`: event/transcript keys
    /// such as `stage-run-<uuid>` are valid trace identities but are not UUID
    /// foreign keys into the `sessions` table.
    pub persistence_session_id: Option<uuid::Uuid>,
    /// Base directory for transcript files (e.g., `~/.golish/transcripts`)
    /// If set, sub-agent internal events will be written to separate transcript files.
    pub transcript_base_dir: Option<&'a std::path::Path>,
    /// API request stats collector (per session, optional)
    pub api_request_stats: Option<&'a Arc<ApiRequestStats>>,
    /// Shared top-level cancel flag. When set, this worker should stop before
    /// starting a new LLM request/tool call and should interrupt stream waits.
    pub cancelled: Option<&'a Arc<AtomicBool>>,
    /// Orchestrator briefing injected before execution. Contains relevant memories,
    /// execution plan context, and findings from other agents. Appended to the
    /// effective system prompt as a `## Briefing from Orchestrator` section.
    pub briefing: Option<String>,
    /// Per-agent temperature override from settings (None = use default 0.3).
    pub temperature_override: Option<f32>,
    /// Per-agent max_tokens override from settings (None = use default 8192).
    pub max_tokens_override: Option<u32>,
    /// Per-agent top_p override from settings (None = not sent to provider).
    pub top_p_override: Option<f32>,
    /// Chain persistence backend for saving/restoring sub-agent conversation
    /// chains (PentAGI-style). Replaces the raw `sqlx::PgPool`.
    pub chain_persistence: Option<&'a Arc<dyn SubAgentChainPersistence>>,
    /// Trusted V2 worker/chain binding. When present the executor must load and
    /// checkpoint this exact chain; ordinary `chain_create`/latest-resume paths
    /// are forbidden.
    pub bound_worker_chain: Option<BoundWorkerChainContext>,
    /// Sub-agent registry for nested delegation (PentAGI hierarchical pattern).
    /// When set, agents with `delegatable_agents` can invoke other sub-agents.
    pub sub_agent_registry: Option<&'a Arc<RwLock<crate::definition::SubAgentRegistry>>>,
    /// Optional hook called after a successful shell tool execution.
    pub post_shell_hook: Option<PostShellHook>,
    /// AI-controlled resume handle for this delegation:
    /// `Some("<chain_id>")` continues that exact prior sub-agent conversation;
    /// `Some("latest")` continues this agent's most recent chain; `None` is a
    /// fresh sub-agent (default). Enables "go back to the same worker".
    pub resume: Option<String>,
    /// Extra tool executor for tools outside the `ToolRegistry` (security/graph),
    /// tried before the registry fallback so a delegated sub-agent can actually
    /// run them (e.g. `graph_add_entity`) instead of getting "Unknown tool".
    pub sub_tool_router: Option<SubAgentToolRouter>,
    /// Writable active-org side-channel used by legacy registry tools. Per-org
    /// stage_run workers prefer hidden per-call org args for org-aware tools
    /// (`manage_targets` / `manage_organizations`) and keep this as a fallback.
    pub active_org_id_source: Option<Arc<RwLock<Option<uuid::Uuid>>>>,
    /// The org id this sub-agent is bound to. Org-aware registry tools receive it
    /// as an internal hidden arg; non-injectable tools may fall back to the
    /// side-channel above. `None` preserves the parent/global binding.
    pub active_org_id_override: Option<uuid::Uuid>,
    /// Trusted harness operation/stage-attempt identity inherited from the
    /// parent runtime. It is copied into each sub-agent
    /// [`golish_core::AgentToolContext`] and is never sourced from model-visible
    /// tool arguments.
    pub operation_id: Option<uuid::Uuid>,
    /// Optional post-processing hook for regular tool results. This keeps the
    /// executor generic while allowing the agent runtime to attach harness
    /// evidence/source logging to sub-agent tool calls.
    pub post_tool_result_hook: Option<SubAgentToolResultHook>,
    /// Optional observer for completed regular tool calls. Used by upper layers
    /// to implement runtime mentor/supervisor logic without making this crate
    /// depend on those layers.
    pub tool_observer: Option<SubAgentToolObserver>,
    /// Optional persisted submit-repair directive restored from
    /// `operation_state.state_blob.agent_run`. When present, the sub-agent starts
    /// in deterministic repair-only mode instead of rediscovering the same stage
    /// from scratch after a resume/retry.
    pub initial_submit_repair_mode: Option<SubmitRepairMode>,
    /// Optional per-stage tool boundary guard (forbidden-only). When the
    /// sub-agent runs inside a harness stage, this blocks tool calls whose
    /// resolved capability is in the stage's forbidden list (e.g. `dig` in
    /// scoping), before execution. `None` = no active stage (legacy behaviour).
    pub stage_tool_guard: Option<StageToolGuard>,
    /// Optional per-stage tool-list filter (D1). When the sub-agent runs inside
    /// a harness stage that permits no scan tools (e.g. `scoping`), this hides
    /// scan tools from the exposed list so the model never even attempts one —
    /// preventing the retry spin the call-time guard alone can't stop. `None` =
    /// no filtering (legacy behaviour).
    pub hide_tool_in_stage: Option<StageToolHider>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn many_coverage_gap_actions(count: usize) -> Vec<CoverageGapAction> {
        (0..count)
            .map(|idx| CoverageGapAction {
                asset: format!("https://asset-{idx:04}.example.test:443"),
                technique: "GOLISH-ENUM-DIR".to_string(),
                reason: format!("missing-terminal-gap-{idx:04}"),
                suggested_capabilities: Vec::new(),
                suggested_tools: vec!["route_probe_paths".to_string()],
            })
            .collect()
    }

    #[test]
    fn target_intel_repair_allows_only_the_suggested_recon_tool() {
        let mode = SubmitRepairMode {
            kind: SubmitRepairKind::CoverageGap,
            reason: "OSINT is non-terminal".to_string(),
            missing_required_checks: Vec::new(),
            coverage_gap_actions: vec![CoverageGapAction {
                asset: "moresec.cn".to_string(),
                technique: "GOLISH-INTEL-OSINT".to_string(),
                reason: "missing_terminal_coverage".to_string(),
                suggested_capabilities: Vec::new(),
                suggested_tools: vec!["recon_map_assets".to_string()],
            }],
            eas_web_repair_targets: None,
            allowed_tools_override: Vec::new(),
            forbidden_tools: Vec::new(),
            directive_message: None,
        };

        assert!(mode.allows("recon_map_assets"));
        assert!(!mode.allows("recon_discover_subsidiaries"));
    }

    #[test]
    fn recovery_projection_bounds_1176_actions_and_block_payload() {
        let mode = SubmitRepairMode {
            kind: SubmitRepairKind::CoverageGap,
            reason: "enumeration coverage has pending cells".to_string(),
            missing_required_checks: Vec::new(),
            coverage_gap_actions: many_coverage_gap_actions(1_176),
            eas_web_repair_targets: None,
            allowed_tools_override: Vec::new(),
            forbidden_tools: Vec::new(),
            directive_message: None,
        };

        assert_eq!(mode.coverage_gap_actions.len(), 1_176);
        let first = mode.model_instruction();
        assert_eq!(first, mode.model_instruction(), "projection is byte-stable");
        assert!(first.contains("total=1176"));
        assert!(first.contains("stable_hash="));
        assert!(first.contains("stage_worklist_next"));
        assert!(first.contains("asset-0000.example.test"));
        assert!(first.contains("asset-0019.example.test"));
        assert!(!first.contains("asset-0020.example.test"));
        assert!(!first.contains("asset-1175.example.test"));
        assert!(first.len() <= 32 * 1024, "{} bytes", first.len());

        let blocked = mode
            .block_result("list_in_scope_targets")
            .expect("repair lock blocks rediscovery");
        let projection = blocked
            .get("coverage_gap_actions")
            .and_then(serde_json::Value::as_object)
            .expect("blocked payload uses a bounded projection object");
        assert_eq!(projection.get("total"), Some(&serde_json::json!(1_176)));
        assert!(projection
            .get("stable_hash")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|hash| !hash.is_empty()));
        assert_eq!(
            projection
                .get("sample")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len),
            Some(20)
        );
        assert_eq!(projection.get("omitted"), Some(&serde_json::json!(1_156)));
        let encoded = serde_json::to_string(&blocked).expect("blocked payload serializes");
        assert!(encoded.contains("asset-0000.example.test"));
        assert!(encoded.contains("asset-0019.example.test"));
        assert!(!encoded.contains("asset-0020.example.test"));
        assert!(!encoded.contains("asset-1175.example.test"));
        assert!(encoded.len() <= 64 * 1024, "{} bytes", encoded.len());
        assert!(blocked["error"].as_str().unwrap().len() <= 32 * 1024);
        assert_eq!(blocked, mode.block_result("list_in_scope_targets").unwrap());
    }
}
