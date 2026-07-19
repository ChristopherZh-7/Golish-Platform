//! Typed, sqlx-free contract for operation-scoped runtime-memory persistence.

use async_trait::async_trait;
use serde_json::Value;
use uuid::Uuid;

use super::{OperationStateView, StageAssetWaveView, TaskView};
use crate::runtime_memory::RuntimeMemoryContract;
use crate::task_orchestrator::stage_execution::{
    CompleteTerminalStageExecution, StageExecution, TransitionStageExecution,
    TransitionedStageExecution,
};

#[derive(Debug, thiserror::Error)]
pub enum RuntimeMemoryError {
    #[error("runtime memory repository is unavailable")]
    Unavailable,
    #[error("invalid runtime contract transition: {from} -> {to}")]
    InvalidContractTransition {
        from: RuntimeMemoryContract,
        to: RuntimeMemoryContract,
    },
    #[error("stale runtime row version: expected {expected}")]
    StaleVersion { expected: i64 },
    #[error("runtime memory conflict: {code}")]
    Conflict { code: &'static str },
    #[error("runtime memory identity mismatch: {code}")]
    IdentityMismatch { code: &'static str },
    #[error("runtime memory row missing: {entity}")]
    Missing { entity: &'static str },
    #[error("runtime worker lease lost: worker={worker_run_id}, epoch={attempt_epoch}")]
    LeaseLost {
        worker_run_id: Uuid,
        attempt_epoch: i64,
    },
    #[error("runtime memory storage failure: {0}")]
    Storage(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectScopeRegistration {
    pub project_scope_id: Uuid,
    pub canonical_project_path: String,
    pub path_sha256: String,
    pub row_version: i64,
}

#[derive(Debug, Clone)]
pub struct CreateRuntimeOperation {
    pub operation_id: Uuid,
    pub initial_stage_execution_id: Uuid,
    pub session_id: Uuid,
    pub title: Option<String>,
    pub input: String,
    pub profile: String,
    pub entry_stage: String,
    pub project_scope: ProjectScopeRegistration,
    /// Trusted CLI-only scope material. When present, operation creation must
    /// freeze this decision and organization snapshot in the same transaction
    /// as the task/operation/stage-execution roots. Model-authored stage input
    /// can never populate this field.
    pub cli_scope: Option<CliRuntimeScope>,
    /// Trusted source-operation lineage for a fresh stage-testing fork. The
    /// repository validates and freezes this authority in the same transaction
    /// as the new operation roots; it is never reconstructed from model text.
    pub stage_fork: Option<StageForkCreate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageForkCreate {
    pub source_operation_id: Uuid,
    pub source_scope_snapshot_id: Uuid,
    pub entry_stage: String,
    pub terminal_stage: String,
    pub adopted_stage_kinds: Vec<String>,
}

/// One organization selected by trusted CLI flags before a V2-writing
/// operation starts. The repository rebinds these rows to the newly allocated
/// operation and stage-execution identities; neither identity is caller data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliRuntimeScopeUnit {
    pub organization_id: Uuid,
    pub parent_organization_id: Option<Uuid>,
    pub organization_name: String,
    pub depth: i32,
    pub ordinal: i32,
    pub ownership_percent: Option<String>,
    pub approval_source: Value,
}

/// Immutable scope selected once by the headless CLI. `units[0]` must be the
/// root; descendants have already been filtered by the explicit ownership
/// threshold, so later stage workers consume only the frozen rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliRuntimeScope {
    pub root_organization_id: Uuid,
    pub include_subsidiaries: bool,
    pub subsidiary_threshold: u8,
    pub units: Vec<CliRuntimeScopeUnit>,
}

#[derive(Debug, Clone)]
pub struct CreatedRuntimeOperation {
    pub task: TaskView,
    pub operation: OperationStateView,
    pub initial_stage_execution_id: Uuid,
}

/// SQLx-free command for one immutable trusted deliverable submission. Every
/// identity field is server-derived; model JSON is confined to the canonical
/// payload string and can never select its operation/stage/unit/tool owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewStageDeliverableSubmission {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Option<Uuid>,
    pub worker_run_id: Option<Uuid>,
    pub organization_id: Option<Uuid>,
    pub tool_call_record_id: Uuid,
    pub tool_request_id: String,
    pub stage_kind: String,
    pub attempt_epoch: Option<i64>,
    pub lease_token: Option<Uuid>,
    pub canonical_deliverable_json: String,
    pub payload_sha256: String,
}

/// Durable read model returned by the runtime-memory repository. The caller
/// supplies the canonical string separately when capturing a freshly inserted
/// submission because JSONB intentionally does not preserve input key order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedStageDeliverableSubmission {
    pub deliverable_submission_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Option<Uuid>,
    pub worker_run_id: Option<Uuid>,
    pub organization_id: Option<Uuid>,
    pub tool_call_record_id: Uuid,
    pub tool_request_id: String,
    pub stage_kind: String,
    pub attempt_epoch: Option<i64>,
    pub lease_token: Option<Uuid>,
    pub payload: Value,
    pub payload_sha256: String,
}

/// Typed side-channel consumed by stage close. It binds the exact canonical
/// gate input to the durable submission identity rather than passing an
/// untrusted prose/JSON string by itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedStageSubmission {
    pub deliverable_submission_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Option<Uuid>,
    pub canonical_deliverable_json: String,
    pub payload_sha256: String,
}

/// Server-owned identities required to freeze Scoping. The agent can submit a
/// canonical deliverable, but it cannot choose or rewrite any of these owner
/// keys; orchestration derives them from the active execution and trusted tool
/// capture before calling the repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizeScopingScope {
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub stage_execution_id: Uuid,
    pub root_organization_id: Uuid,
    pub deliverable_submission_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub scoping_root_unit_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenOrganizationScopeUnit {
    pub organization_id: Uuid,
    pub parent_organization_id: Option<Uuid>,
    pub organization_name_at_freeze: String,
    pub role: String,
    pub depth: i32,
    pub ordinal: i32,
    pub ownership_percent: Option<String>,
    pub decision_row_id: String,
    pub approval_source: Value,
}

/// SQLx-free result of the atomic decision/snapshot/root-unit transaction.
/// `replayed` means the complete identity tuple already named the same sealed
/// scope; it never means that a mismatched tuple was silently accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizedScopingScope {
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub stage_execution_id: Uuid,
    pub root_organization_id: Uuid,
    pub deliverable_submission_id: Uuid,
    pub scope_decision_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub scoping_root_unit_id: Uuid,
    pub mode: String,
    pub scope_hash: String,
    pub units: Vec<FrozenOrganizationScopeUnit>,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeStageUnitStatus {
    Queued,
    Running,
    GateBlocked,
    Passed,
    Exhausted,
    Superseded,
}

impl RuntimeStageUnitStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::GateBlocked => "gate_blocked",
            Self::Passed => "passed",
            Self::Exhausted => "exhausted",
            Self::Superseded => "superseded",
        }
    }

    pub fn try_parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "gate_blocked" => Some(Self::GateBlocked),
            "passed" => Some(Self::Passed),
            "exhausted" => Some(Self::Exhausted),
            "superseded" => Some(Self::Superseded),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeWorkerStatus {
    Queued,
    Running,
    WaitingBackground,
    GateBlocked,
    Passed,
    Failed,
    Exhausted,
    Superseded,
    RecoveryRequired,
}

impl RuntimeWorkerStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::WaitingBackground => "waiting_background",
            Self::GateBlocked => "gate_blocked",
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Exhausted => "exhausted",
            Self::Superseded => "superseded",
            Self::RecoveryRequired => "recovery_required",
        }
    }

    pub fn try_parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "waiting_background" => Some(Self::WaitingBackground),
            "gate_blocked" => Some(Self::GateBlocked),
            "passed" => Some(Self::Passed),
            "failed" => Some(Self::Failed),
            "exhausted" => Some(Self::Exhausted),
            "superseded" => Some(Self::Superseded),
            "recovery_required" => Some(Self::RecoveryRequired),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStageUnitView {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub stage_kind: String,
    pub generation: i32,
    pub specialist: Option<String>,
    pub status: RuntimeStageUnitStatus,
    pub gate_attempt: i32,
    pub pass_watermark: Value,
    pub row_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeWorkerView {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub work_item_id: Option<Uuid>,
    pub organization_id: Uuid,
    pub worker_generation: i32,
    pub specialist: String,
    pub work_item_kind: String,
    pub work_item_key: String,
    pub agent_path: String,
    pub parent_request_id: Option<String>,
    pub message_chain_id: Option<Uuid>,
    pub status: RuntimeWorkerStatus,
    pub gate_attempt: i32,
    pub checkpoint: Value,
    pub checkpoint_version: i64,
    pub lease_token: Option<Uuid>,
    pub lease_owner: Option<String>,
    pub lease_expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub heartbeat_at: Option<chrono::DateTime<chrono::Utc>>,
    pub attempt_epoch: i64,
    pub active_tool_call_id: Option<Uuid>,
    pub active_tool_started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub evidence_watermark: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeStageTeamPlanStatus {
    Active,
    Finalizing,
    GateBlocked,
    Passed,
    Superseded,
}

impl RuntimeStageTeamPlanStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Finalizing => "finalizing",
            Self::GateBlocked => "gate_blocked",
            Self::Passed => "passed",
            Self::Superseded => "superseded",
        }
    }

    pub fn try_parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "finalizing" => Some(Self::Finalizing),
            "gate_blocked" => Some(Self::GateBlocked),
            "passed" => Some(Self::Passed),
            "superseded" => Some(Self::Superseded),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeStageWorkItemStatus {
    Queued,
    Claimed,
    Running,
    WaitingDependency,
    RetryPending,
    Completed,
    Exhausted,
    Superseded,
    RecoveryRequired,
}

impl RuntimeStageWorkItemStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Claimed => "claimed",
            Self::Running => "running",
            Self::WaitingDependency => "waiting_dependency",
            Self::RetryPending => "retry_pending",
            Self::Completed => "completed",
            Self::Exhausted => "exhausted",
            Self::Superseded => "superseded",
            Self::RecoveryRequired => "recovery_required",
        }
    }

    pub fn try_parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "claimed" => Some(Self::Claimed),
            "running" => Some(Self::Running),
            "waiting_dependency" => Some(Self::WaitingDependency),
            "retry_pending" => Some(Self::RetryPending),
            "completed" => Some(Self::Completed),
            "exhausted" => Some(Self::Exhausted),
            "superseded" => Some(Self::Superseded),
            "recovery_required" => Some(Self::RecoveryRequired),
            _ => None,
        }
    }

    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Exhausted | Self::Superseded)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageWorkerOutputDisposition {
    Found,
    CheckedEmpty,
    Blocked,
}

impl StageWorkerOutputDisposition {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Found => "found",
            Self::CheckedEmpty => "checked_empty",
            Self::Blocked => "blocked",
        }
    }

    pub fn try_parse(value: &str) -> Option<Self> {
        match value {
            "found" => Some(Self::Found),
            "checked_empty" => Some(Self::CheckedEmpty),
            "blocked" => Some(Self::Blocked),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageTeamPlanView {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub stage_kind: String,
    pub unit_generation: i32,
    pub schema_version: i32,
    pub plan_version: i32,
    pub plan_sha256: String,
    pub leader_role: String,
    pub allowed_roles: Vec<String>,
    pub aggregator_kind: String,
    pub aggregator_role: Option<String>,
    pub max_workers_total: i32,
    pub max_workers_active: i32,
    pub dynamic_requests_enabled: bool,
    pub dynamic_request_policy: Value,
    pub dispatch_epoch: i64,
    pub requests_closed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub final_submitter_kind: String,
    pub final_submitter_worker_run_id: Option<Uuid>,
    pub created_from_stage_spec_hash: String,
    pub status: RuntimeStageTeamPlanStatus,
    pub row_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageWorkItemView {
    pub id: Uuid,
    pub stage_team_plan_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub stable_key: String,
    pub work_item_kind: String,
    pub role: String,
    pub input_refs: Value,
    pub input_manifest_hash: String,
    pub priority: i32,
    pub required_for_barrier: bool,
    pub is_aggregator: bool,
    pub conflict_key: Option<String>,
    pub attempt_policy: Value,
    pub budget: Value,
    pub output_schema: String,
    pub created_by: String,
    pub status: RuntimeStageWorkItemStatus,
    pub row_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewStageWorkerOutput {
    pub work_item_id: Uuid,
    pub worker_run_id: Uuid,
    pub output_schema: String,
    pub disposition: StageWorkerOutputDisposition,
    pub canonical_output: Value,
    pub fact_refs: Vec<Value>,
    pub evidence_ids: Vec<i64>,
    pub checked_empty_units: Vec<Value>,
    pub blocker_code: Option<String>,
    pub output_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageWorkerOutputView {
    pub id: Uuid,
    pub stage_team_plan_id: Uuid,
    pub work_item_id: Uuid,
    pub worker_run_id: Uuid,
    pub disposition: StageWorkerOutputDisposition,
    pub canonical_output: Value,
    pub fact_refs: Vec<Value>,
    pub evidence_ids: Vec<i64>,
    pub checked_empty_units: Vec<Value>,
    pub blocker_code: Option<String>,
    pub output_sha256: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageWorkerRequestDecision {
    Accepted,
    Rejected,
}

impl StageWorkerRequestDecision {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageWorkerRequestView {
    pub id: Uuid,
    pub stage_team_plan_id: Uuid,
    pub parent_work_item_id: Uuid,
    pub requested_by_worker_run_id: Uuid,
    pub dispatch_epoch: i64,
    pub requested_role: String,
    pub requested_kind: String,
    pub subject_refs: Vec<Value>,
    pub reason: String,
    pub output_schema: Value,
    pub budget_hint: Value,
    pub dedupe_key: String,
    pub decision: StageWorkerRequestDecision,
    pub decision_code: String,
    pub created_work_item_id: Option<Uuid>,
    pub request_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageTeamBarrierView {
    pub stage_team_plan_id: Uuid,
    pub dispatch_epoch: i64,
    pub requests_closed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub required_work_items: i64,
    pub terminal_required_work_items: i64,
    pub live_workers: i64,
    pub retry_pending_work_items: i64,
    pub recovery_required_workers: i64,
    pub missing_outputs: i64,
    pub manifest_sha256: String,
}

impl StageTeamBarrierView {
    pub fn ready_to_finalize(&self) -> bool {
        self.requests_closed_at.is_some()
            && self.required_work_items == self.terminal_required_work_items
            && self.live_workers == 0
            && self.retry_pending_work_items == 0
            && self.recovery_required_workers == 0
            && self.missing_outputs == 0
            && !self.manifest_sha256.trim().is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMemoryRecordSource {
    Legacy,
    V2,
    LegacyFallback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadWorkerCheckpoint {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub worker_run_id: Uuid,
    /// Whole-record source selected once by the trusted resume preflight.
    /// `None` is reserved for fresh/non-resume callers that still select from
    /// the frozen rollout contract.
    pub selected_source: Option<RuntimeMemoryRecordSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedWorkerCheckpoint {
    pub source: RuntimeMemoryRecordSource,
    pub worker: RuntimeWorkerView,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedStageRuntime {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_kind: String,
    pub unit_generation: i32,
    pub specialist: String,
    pub worker_generation: i32,
    pub work_item_kind: String,
    pub work_item_key: String,
    pub agent_path_prefix: String,
    /// Optional server-owned subset of the sealed organization snapshot.
    /// `None` seeds every frozen organization.
    pub organization_ids: Option<Vec<Uuid>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeededStageRuntime {
    pub unit: RuntimeStageUnitView,
    pub worker: RuntimeWorkerView,
    pub organization_name: String,
    pub scope_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageTeamPlanSeed {
    pub schema_version: i32,
    pub plan_version: i32,
    pub plan_sha256: String,
    pub leader_role: String,
    pub allowed_roles: Vec<String>,
    pub aggregator_kind: String,
    pub aggregator_role: Option<String>,
    pub max_workers_total: i32,
    pub max_workers_active: i32,
    pub dynamic_requests_enabled: bool,
    pub dynamic_request_policy: Value,
    pub final_submitter_kind: String,
    pub created_from_stage_spec_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageWorkItemSeed {
    pub stable_key: String,
    pub work_item_kind: String,
    pub role: String,
    pub input_manifest: Value,
    pub input_sha256: String,
    pub conflict_key: Option<String>,
    pub priority: i32,
    pub required_for_barrier: bool,
    pub is_aggregator: bool,
    pub attempt_policy: Value,
    pub budget: Value,
    pub output_schema: String,
    pub created_by: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedStageTeamRuntime {
    pub base: SeedStageRuntime,
    pub plan: StageTeamPlanSeed,
    pub work_items: Vec<StageWorkItemSeed>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeededStageTeamRuntime {
    pub unit: RuntimeStageUnitView,
    pub plan: StageTeamPlanView,
    pub work_items: Vec<StageWorkItemView>,
    pub primary_worker: Option<RuntimeWorkerView>,
    pub organization_name: String,
    pub scope_hash: String,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttackV2WaveEntryView {
    VulnTriageHandoff,
    ForkedVulnHandoff,
    FactDeltaConsolidation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttackV2WaveUnitStateView {
    AwaitingManifest,
    FrozenManifest,
    TerminalNoInput,
}

/// SQLx-free, server-owned WaveUnit routing authority. Initial authority has no
/// WaveUnit row yet (`wave_unit_id=None`); every current authority has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttackV2WaveRuntimeUnitView {
    pub wave_unit_id: Option<Uuid>,
    pub organization_id: Uuid,
    pub ordinal: i32,
    pub status: String,
    pub entry: AttackV2WaveEntryView,
    pub state: AttackV2WaveUnitStateView,
}

/// Durable operation-wide Candidate Wave cursor. `Terminal` is explicit so a
/// response-loss replay can never be mistaken for a new generation-zero run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttackV2WaveAuthorityView {
    Initial {
        operation_id: Uuid,
        scope_snapshot_id: Uuid,
        generation: i32,
        units: Vec<AttackV2WaveRuntimeUnitView>,
    },
    Current {
        operation_id: Uuid,
        scope_snapshot_id: Uuid,
        wave_run_id: Uuid,
        generation: i32,
        status: String,
        units: Vec<AttackV2WaveRuntimeUnitView>,
    },
    Terminal {
        operation_id: Uuid,
        scope_snapshot_id: Uuid,
        wave_run_id: Uuid,
        generation: i32,
    },
}

#[derive(Debug, Clone)]
pub struct ClaimWorkerAndBindChain {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub worker_run_id: Uuid,
    pub expected_unit_status: RuntimeStageUnitStatus,
    pub expected_unit_row_version: i64,
    pub expected_worker_status: RuntimeWorkerStatus,
    pub expected_attempt_epoch: i64,
    pub session_id: Uuid,
    pub subtask_id: Option<Uuid>,
    pub agent: super::AgentType,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub parent_chain_id: Option<Uuid>,
    pub lease_owner: String,
    pub lease_seconds: i32,
    pub initial_chain: Value,
    pub initial_checkpoint: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedWorkerView {
    pub unit: RuntimeStageUnitView,
    pub worker: RuntimeWorkerView,
    pub message_chain_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimStageWorkItem {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub stage_team_plan_id: Uuid,
    pub lease_owner: String,
    pub lease_seconds: i32,
    pub session_id: Uuid,
    pub subtask_id: Option<Uuid>,
    pub agent: super::AgentType,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub parent_chain_id: Option<Uuid>,
    pub initial_chain: Value,
    pub initial_checkpoint: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedStageWorkItemView {
    pub unit: RuntimeStageUnitView,
    pub plan: StageTeamPlanView,
    pub work_item: StageWorkItemView,
    pub worker: RuntimeWorkerView,
    pub message_chain_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestStageWorker {
    pub fence: RuntimeWorkerFence,
    pub stage_team_plan_id: Uuid,
    pub parent_work_item_id: Uuid,
    pub expected_dispatch_epoch: i64,
    pub requested_role: String,
    pub requested_kind: String,
    pub subject_refs: Vec<Value>,
    pub reason: String,
    pub output_schema: Value,
    pub budget_hint: Value,
    pub dedupe_key: String,
    pub request_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestedStageWorkerView {
    pub request: StageWorkerRequestView,
    pub work_item: Option<StageWorkItemView>,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseStageRequestEpoch {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub stage_team_plan_id: Uuid,
    pub expected_dispatch_epoch: i64,
    pub expected_plan_row_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedStageRequestEpochView {
    pub plan: StageTeamPlanView,
    pub barrier: StageTeamBarrierView,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadStageTeamBarrier {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub stage_team_plan_id: Uuid,
    pub dispatch_epoch: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimStageAggregator {
    pub claim: ClaimStageWorkItem,
    pub expected_dispatch_epoch: i64,
    pub expected_manifest_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimStageTeamLeader {
    pub claim: ClaimStageWorkItem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParkStageTeamLeader {
    pub fence: RuntimeWorkerFence,
    pub stage_team_plan_id: Uuid,
    pub leader_work_item_id: Uuid,
    pub expected_work_item_row_version: i64,
    pub checkpoint: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParkedStageTeamLeaderView {
    pub plan: StageTeamPlanView,
    pub work_item: StageWorkItemView,
    pub worker: RuntimeWorkerView,
    pub dependency_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindStageTeamLeaderFinalSubmitter {
    pub fence: RuntimeWorkerFence,
    pub stage_team_plan_id: Uuid,
    pub leader_work_item_id: Uuid,
    pub expected_plan_row_version: i64,
    pub expected_dispatch_epoch: i64,
    pub expected_manifest_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundStageTeamLeaderFinalSubmitterView {
    pub plan: StageTeamPlanView,
    pub barrier: StageTeamBarrierView,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReopenStageTeamLeaderAfterGateBlock {
    pub request_id: String,
    pub fence: RuntimeWorkerFence,
    pub stage_team_plan_id: Uuid,
    pub leader_work_item_id: Uuid,
    pub deliverable_submission_id: Uuid,
    pub expected_dispatch_epoch: i64,
    pub expected_manifest_sha256: String,
    pub gate_decision_sha256: String,
    pub gap_manifest: Value,
    pub gap_manifest_sha256: String,
    /// Complete Controller checkpoint captured after the blocked Gate turn.
    /// This is restored on the same WorkerRun/message chain; it is not a
    /// reduced Gate-only marker.
    pub checkpoint: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReopenedStageTeamLeaderAfterGateBlockView {
    pub plan: StageTeamPlanView,
    pub unit: RuntimeStageUnitView,
    /// Present for a repairable BLOCK. The current schema permits one durable
    /// gap per WorkerRun; a later fuel-exhausted BLOCK is durably represented
    /// by the terminal Controller checkpoint and GateBlocked states instead.
    pub gap_id: Option<Uuid>,
    pub repair_generation: i32,
    pub fuel_exhausted: bool,
    pub leader_work_item: StageWorkItemView,
    pub leader_worker: RuntimeWorkerView,
    pub replayed: bool,
}

/// Compound Candidate/WorkerRun/global-lane claim. All IDs are server-owned
/// scheduler state; none are accepted from verifier model arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimCandidateAttempt {
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub verification_stage_execution_id: Uuid,
    pub verification_stage_run_unit_id: Uuid,
    pub lease_owner: String,
    pub lease_seconds: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedCandidateAttemptView {
    pub candidate_attempt: golish_core::CandidateAttemptContextRef,
    pub worker: RuntimeWorkerView,
    pub message_chain_id: Uuid,
    pub submit_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatCandidateAttempt {
    pub candidate_attempt: golish_core::CandidateAttemptContextRef,
    pub fence: RuntimeWorkerFence,
    pub organization_id: Uuid,
    pub lease_owner: String,
    pub extend_seconds: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateHeartbeatView {
    pub lease_expires_at: chrono::DateTime<chrono::Utc>,
    pub attempt_epoch: i64,
    pub checkpoint_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlCandidateAttempt {
    pub candidate_attempt: golish_core::CandidateAttemptContextRef,
    pub fence: RuntimeWorkerFence,
    pub organization_id: Uuid,
    pub lease_owner: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateExecutionContinuationView {
    SafeRelease,
    SubmitOnly,
    RecoveryRequired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateReleaseView {
    pub requeued: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubmitCandidateAttempt {
    pub candidate_attempt: golish_core::CandidateAttemptContextRef,
    pub fence: RuntimeWorkerFence,
    pub organization_id: Uuid,
    pub result: crate::harness::attack_execution::CandidateAttemptResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmittedCandidateAttemptView {
    pub attempt_id: Uuid,
    pub result_hash: String,
    pub terminal_intent_id: Option<Uuid>,
    pub terminal_intent_hash: Option<String>,
    /// Exact ToolResult persisted inside TerminalIntent. The tool boundary
    /// must return this value unchanged so the post-tool barrier can prove it.
    pub tool_result: serde_json::Value,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateTerminalIntentStatus {
    Pending,
    BarrierReady,
    Consumed,
}

impl CandidateTerminalIntentStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::BarrierReady => "barrier_ready",
            Self::Consumed => "consumed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateTerminalIntentView {
    pub id: Uuid,
    pub request_id: String,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub candidate_id: Uuid,
    pub attempt_id: Uuid,
    pub worker_run_id: Uuid,
    pub tool_call_record_id: Uuid,
    pub candidate_plan_hash: String,
    pub result_hash: String,
    pub evidence_manifest_hash: String,
    pub tool_result_hash: String,
    pub intent_hash: String,
    pub barrier_id: Option<Uuid>,
    pub barrier_hash: Option<String>,
    pub status: CandidateTerminalIntentStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointCandidateTerminalBarrier {
    pub checkpoint: CheckpointBoundWorkerChain,
    pub terminal_intent_id: Uuid,
    pub expected_intent_hash: String,
}

/// Recover the post-submit lifecycle for one immutable Candidate terminal
/// intent. This is server-owned and may only reconcile the recorded tool
/// result/checkpoint/barrier; it never replays an external verification action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoverCandidateTerminalIntent {
    pub operation_id: Uuid,
    pub terminal_intent_id: Uuid,
    pub expected_intent_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateTerminalBarrierView {
    pub id: Uuid,
    pub request_id: String,
    pub terminal_intent_id: Uuid,
    pub attempt_id: Uuid,
    pub worker_run_id: Uuid,
    pub tool_call_record_id: Uuid,
    pub message_chain_id: Uuid,
    pub attempt_epoch: i64,
    pub checkpoint_version: i64,
    pub checkpoint_hash: String,
    pub tool_result_hash: String,
    pub barrier_hash: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalizeCandidateIntent {
    pub operation_id: Uuid,
    pub terminal_intent_id: Uuid,
    pub barrier_id: Uuid,
    pub expected_intent_hash: String,
    pub expected_barrier_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateRecoveryDecision {
    TerminalizeBlockedOutcomeUnknown,
    AbandonBeforeSideEffect,
    AcceptExternalResultWithExactEvidence,
}

impl CandidateRecoveryDecision {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TerminalizeBlockedOutcomeUnknown => "terminalize_blocked_outcome_unknown",
            Self::AbandonBeforeSideEffect => "abandon_before_side_effect",
            Self::AcceptExternalResultWithExactEvidence => {
                "accept_external_result_with_exact_evidence"
            }
        }
    }

    pub fn try_parse(value: &str) -> Option<Self> {
        match value {
            "terminalize_blocked_outcome_unknown" => Some(Self::TerminalizeBlockedOutcomeUnknown),
            "abandon_before_side_effect" => Some(Self::AbandonBeforeSideEffect),
            "accept_external_result_with_exact_evidence" => {
                Some(Self::AcceptExternalResultWithExactEvidence)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateRecoveryCaseView {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub candidate_id: Uuid,
    pub attempt_id: Uuid,
    pub action_id: Option<Uuid>,
    pub worker_run_id: Uuid,
    pub reason_code: String,
    pub status: String,
    pub row_version: i64,
    pub attempt_row_version: i64,
    pub opened_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveCandidateRecovery {
    pub operation_id: Uuid,
    pub request_id: Uuid,
    pub recovery_case_id: Uuid,
    pub decision: CandidateRecoveryDecision,
    pub expected_case_version: i64,
    pub expected_attempt_version: i64,
    pub evidence_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCandidateRecoveryView {
    pub recovery_case: CandidateRecoveryCaseView,
    pub terminal_intent: Option<CandidateTerminalIntentView>,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvergedCandidateRecoveryView {
    pub recovery_case: CandidateRecoveryCaseView,
    pub terminalized: Option<TerminalizedCandidateAttemptView>,
    pub candidate_reopened: bool,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalizeCandidateAttempt {
    pub candidate_attempt: golish_core::CandidateAttemptContextRef,
    pub fence: RuntimeWorkerFence,
    pub organization_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalizedCandidateAttemptView {
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub candidate_id: Uuid,
    pub attempt_id: Uuid,
    pub status: String,
    /// Compatibility name retained for existing scheduler logging. It is
    /// always identical to `status`.
    pub disposition: String,
    pub finding_id: Option<Uuid>,
    pub evidence_count: u32,
    pub fact_delta_count: u32,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloseAttackV2VerificationUnit {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub verification_stage_execution_id: Uuid,
    pub verification_stage_run_unit_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedAttackV2VerificationUnitView {
    pub wave_unit_id: Uuid,
    pub row_version: i64,
    pub verification_closed: bool,
    pub consolidation_status: String,
    pub verification_stage_run_unit_id: Uuid,
    pub verification_stage_run_unit_status: String,
    pub verification_primary_worker_run_id: Uuid,
    pub verification_primary_worker_status: String,
    pub verification_handoff_id: Uuid,
    pub verification_handoff_payload_sha256: String,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeWorkerFence {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub worker_run_id: Uuid,
    pub lease_token: Uuid,
    pub attempt_epoch: i64,
    pub expected_checkpoint_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointWorker {
    pub fence: RuntimeWorkerFence,
    pub checkpoint: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointBoundWorkerChain {
    pub fence: RuntimeWorkerFence,
    pub message_chain_id: Uuid,
    pub chain: Value,
    pub checkpoint: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadBoundWorkerChain {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub worker_run_id: Uuid,
    pub message_chain_id: Uuid,
    pub session_id: Uuid,
    pub agent: super::AgentType,
    /// Pin this worker+chain load to the same whole-record source as the graph
    /// cursor. Preferred mode must never reselect per worker during resume.
    pub selected_source: Option<RuntimeMemoryRecordSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedBoundWorkerChain {
    pub source: RuntimeMemoryRecordSource,
    pub worker: RuntimeWorkerView,
    pub chain: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeExpiredWorkerDisposition {
    Requeued,
    RecoveryRequired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReapedRuntimeWorker {
    pub disposition: RuntimeExpiredWorkerDisposition,
    pub worker: RuntimeWorkerView,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerToolMutation {
    pub fence: RuntimeWorkerFence,
    pub tool_call_record_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishWorkerAttempt {
    pub fence: RuntimeWorkerFence,
    pub expected_status: RuntimeWorkerStatus,
    pub next_status: RuntimeWorkerStatus,
    pub expected_unit_status: RuntimeStageUnitStatus,
    pub expected_unit_row_version: i64,
    pub next_unit_status: RuntimeStageUnitStatus,
    pub checkpoint: Value,
    pub evidence_watermark: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishedWorkerAttempt {
    pub unit: RuntimeStageUnitView,
    pub worker: RuntimeWorkerView,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteStageWorker {
    pub fence: RuntimeWorkerFence,
    pub stage_team_plan_id: Uuid,
    pub work_item_id: Uuid,
    pub expected_work_item_row_version: i64,
    pub output: NewStageWorkerOutput,
    pub terminal_checkpoint: Value,
    pub evidence_watermark: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedStageWorkerView {
    pub unit: RuntimeStageUnitView,
    pub plan: StageTeamPlanView,
    pub work_item: StageWorkItemView,
    pub worker: RuntimeWorkerView,
    pub output: StageWorkerOutputView,
    pub replayed: bool,
}

/// Finish one producer/helper execution attempt without manufacturing a
/// business output. The repository either puts the immutable WorkItem back on
/// the durable queue or marks that item exhausted after its frozen retry
/// budget is consumed; the owning Unit remains running for scheduler/recovery
/// authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryStageWorker {
    pub fence: RuntimeWorkerFence,
    pub stage_team_plan_id: Uuid,
    pub work_item_id: Uuid,
    pub expected_work_item_row_version: i64,
    pub failure_code: String,
    pub terminal_checkpoint: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetriedStageWorkerView {
    pub unit: RuntimeStageUnitView,
    pub plan: StageTeamPlanView,
    pub work_item: StageWorkItemView,
    pub worker: RuntimeWorkerView,
    pub retry_scheduled: bool,
}

/// Local-operator-only convergence command for an expired Team Worker whose
/// exact active tool outcome is unknown. It carries only CAS fields exposed by
/// the sanitized Stage Team read model; the repository reloads every owner and
/// never replays the external tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveStageTeamRecovery {
    pub request_id: String,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub stage_team_plan_id: Uuid,
    pub work_item_id: Uuid,
    pub worker_run_id: Uuid,
    pub tool_call_record_id: Uuid,
    pub expected_work_item_row_version: i64,
    pub expected_checkpoint_version: i64,
    pub expected_attempt_epoch: i64,
    pub resolved_by: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedStageTeamRecoveryView {
    pub decision_id: Uuid,
    pub decision_sha256: String,
    pub work_item: StageWorkItemView,
    pub worker: RuntimeWorkerView,
    pub output: StageWorkerOutputView,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenStageTeamRepair {
    pub request_id: String,
    pub fence: RuntimeWorkerFence,
    pub stage_team_plan_id: Uuid,
    pub aggregator_work_item_id: Uuid,
    pub deliverable_submission_id: Uuid,
    pub expected_dispatch_epoch: i64,
    pub expected_manifest_sha256: String,
    pub gate_decision_sha256: String,
    pub gap_manifest: Value,
    pub gap_manifest_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedStageTeamRepairView {
    pub plan: StageTeamPlanView,
    pub unit: RuntimeStageUnitView,
    pub gap_id: Uuid,
    pub repair_generation: i32,
    pub fuel_exhausted: bool,
    pub repair_work_item: Option<StageWorkItemView>,
    pub aggregator_work_item: Option<StageWorkItemView>,
    pub aggregator_worker: RuntimeWorkerView,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PauseWorkerForContinuation {
    pub fence: RuntimeWorkerFence,
    pub expected_unit_row_version: i64,
    pub checkpoint: Value,
}

#[derive(Debug, Clone)]
pub struct FinalizeUnitPass {
    pub fence: RuntimeWorkerFence,
    pub deliverable_submission_id: Uuid,
    pub expected_unit_status: RuntimeStageUnitStatus,
    pub expected_unit_row_version: i64,
    pub scope_hash: String,
    pub gate_decision: Value,
    pub gate_decision_hash: String,
    pub aggregate_pass_token_hash: Option<String>,
    pub canonical_fact_keys: Vec<crate::harness::handoff_catalog::CanonicalFactKey>,
    pub typed_claims: Vec<Value>,
    pub coverage_watermark: Value,
    pub evidence_ids: Vec<i64>,
    pub terminal_checkpoint: Value,
    /// Present only for attack_candidate. The payload is server-built from the
    /// exact manifest and model draft; final-seal binds all trusted identities.
    pub candidate_acceptance: Option<crate::harness::attack_execution::CandidateAcceptance>,
}

#[derive(Debug, Clone)]
pub struct FinalizeStageTeamUnit {
    pub stage_team_plan_id: Uuid,
    pub aggregator_work_item_id: Uuid,
    pub expected_dispatch_epoch: i64,
    pub expected_manifest_sha256: String,
    pub final_seal: FinalizeUnitPass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizedStageTeamUnitView {
    pub plan: StageTeamPlanView,
    pub aggregator_work_item: StageWorkItemView,
    pub finalized: FinalizedUnitPass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockStageTeamUnit {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub stage_team_plan_id: Uuid,
    pub expected_dispatch_epoch: i64,
    pub expected_manifest_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedStageTeamUnitView {
    pub plan: StageTeamPlanView,
    pub aggregator_work_item: StageWorkItemView,
    pub unit: RuntimeStageUnitView,
    pub barrier: StageTeamBarrierView,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStageHandoffView {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub from_stage_kind: String,
    pub stage_execution_id: Uuid,
    pub source_stage_run_unit_id: Uuid,
    pub deliverable_submission_id: Option<Uuid>,
    pub authority_kind: String,
    pub scope_hash: String,
    pub payload: Value,
    pub payload_sha256: String,
    pub evidence_ids: Vec<i64>,
    pub coverage_watermark: Value,
    pub unit_gate_decision_hash: String,
    pub aggregate_pass_token_hash: Option<String>,
    pub gate_passed_at: chrono::DateTime<chrono::Utc>,
    pub schema_version: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizedUnitPass {
    pub unit: RuntimeStageUnitView,
    pub worker: RuntimeWorkerView,
    pub handoff: RuntimeStageHandoffView,
    pub canonical_fact_refs: Vec<crate::harness::handoff_catalog::CanonicalFactRef>,
    pub replayed: bool,
}

/// One atomic wave-aware Gate-PASS close. The repository must either complete
/// the exact running wave and queue the next wave while parking the Worker, or
/// complete that wave and publish the final Unit PASS/handoff. No legacy
/// completion watermark may be exposed between those alternatives.
#[derive(Debug, Clone)]
pub struct CloseWaveGatePass {
    pub final_seal: FinalizeUnitPass,
    pub wave_id: Uuid,
    pub next_wave_limit: i64,
    pub continuation_pass_watermark: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// Both alternatives are short-lived atomic-close snapshots. Boxing only one
// flips which variant Clippy considers large, while boxing every DTO field
// would add allocation and public API churn to this repository boundary.
#[allow(clippy::large_enum_variant)]
pub enum ClosedWaveGatePass {
    WaitingBackground {
        unit: RuntimeStageUnitView,
        worker: RuntimeWorkerView,
        next_wave: StageAssetWaveView,
    },
    Finalized(FinalizedUnitPass),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadInheritedStageHandoffs {
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub source_stage_kinds: Vec<String>,
}

#[async_trait]
pub trait RuntimeMemoryRepository: Send + Sync {
    async fn project_scope_register_first_open(
        &self,
        canonical_path: &str,
        path_sha256: &str,
    ) -> Result<ProjectScopeRegistration, RuntimeMemoryError>;

    async fn project_scope_rename(
        &self,
        project_scope_id: Uuid,
        expected_old_path: &str,
        expected_row_version: i64,
        new_path: &str,
        new_path_sha256: &str,
    ) -> Result<ProjectScopeRegistration, RuntimeMemoryError>;

    async fn create_runtime_operation(
        &self,
        input: CreateRuntimeOperation,
    ) -> Result<CreatedRuntimeOperation, RuntimeMemoryError>;

    /// Read the exact durable started execution for one operation. Missing and
    /// duplicate active rows both fail closed in the concrete repository.
    async fn active_stage_execution(
        &self,
        operation_id: Uuid,
    ) -> Result<StageExecution, RuntimeMemoryError> {
        let _ = operation_id;
        Err(RuntimeMemoryError::Unavailable)
    }

    /// Atomically close the expected active execution, insert its successor,
    /// and move the operation-stage cursor.
    async fn transition_stage_execution(
        &self,
        input: TransitionStageExecution,
    ) -> Result<TransitionedStageExecution, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    /// Atomically close the exact active terminal execution and finish the
    /// operation task with the generated result. No successor is created.
    async fn complete_terminal_stage_execution(
        &self,
        input: CompleteTerminalStageExecution,
    ) -> Result<StageExecution, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    /// Read the immutable rollout contract frozen on an operation. Defaulting to
    /// unavailable keeps legacy test doubles source-compatible while production
    /// bridges must fail closed when V2 submission persistence is requested.
    async fn runtime_memory_contract_for_operation(
        &self,
        operation_id: Uuid,
    ) -> Result<RuntimeMemoryContract, RuntimeMemoryError> {
        let _ = operation_id;
        Err(RuntimeMemoryError::Unavailable)
    }

    /// Read the immutable Candidate execution contract frozen on the same
    /// operation. Submit normalization uses this to preserve legacy Finding
    /// payloads while making V2-only formulaic stages observation-only.
    async fn attack_execution_contract_for_operation(
        &self,
        operation_id: Uuid,
    ) -> Result<golish_core::AttackExecutionContract, RuntimeMemoryError> {
        let _ = operation_id;
        Err(RuntimeMemoryError::Unavailable)
    }

    /// Load the only durable Candidate Wave cursor for an exact V2 operation.
    /// Missing implementations fail closed; callers must never infer generation
    /// or runnable organizations from model input or process-local counters.
    async fn attack_v2_wave_authority_for_operation(
        &self,
        operation_id: Uuid,
    ) -> Result<AttackV2WaveAuthorityView, RuntimeMemoryError> {
        let _ = operation_id;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn insert_stage_deliverable_submission(
        &self,
        input: NewStageDeliverableSubmission,
    ) -> Result<PersistedStageDeliverableSubmission, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn load_stage_deliverable_submission(
        &self,
        deliverable_submission_id: Uuid,
        operation_id: Uuid,
        stage_execution_id: Uuid,
    ) -> Result<Option<PersistedStageDeliverableSubmission>, RuntimeMemoryError> {
        let _ = (deliverable_submission_id, operation_id, stage_execution_id);
        Err(RuntimeMemoryError::Unavailable)
    }

    /// Freeze the exact Scoping lifecycle and pass its synthetic root unit in
    /// one transaction. This deliberately does not close the active Scoping
    /// execution; stage entry performs the close/open transition atomically.
    async fn finalize_scoping_scope(
        &self,
        input: FinalizeScopingScope,
    ) -> Result<FinalizedScopingScope, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn seed_stage_runtime(
        &self,
        input: SeedStageRuntime,
    ) -> Result<Vec<SeededStageRuntime>, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    /// Seed one immutable TeamPlan and its stable WorkItems per frozen Unit.
    /// Existing legacy/dual callers remain on `seed_stage_runtime`; production
    /// enables sibling workers only for a V2-only operation contract.
    async fn seed_stage_team_runtime(
        &self,
        input: SeedStageTeamRuntime,
    ) -> Result<Vec<SeededStageTeamRuntime>, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn claim_worker_and_bind_chain(
        &self,
        input: ClaimWorkerAndBindChain,
    ) -> Result<ClaimedWorkerView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    /// Claim the next dependency-ready durable WorkItem and create a fresh
    /// sibling WorkerRun/message chain. A WorkerRun is never shared by items.
    async fn claim_stage_work_item(
        &self,
        input: ClaimStageWorkItem,
    ) -> Result<Option<ClaimedStageWorkItemView>, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    /// Claim the server-seeded Company Controller, or resume the same
    /// WorkerRun/message chain once every durable child dependency is terminal.
    async fn claim_stage_team_leader(
        &self,
        input: ClaimStageTeamLeader,
    ) -> Result<Option<ClaimedStageWorkItemView>, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    /// Atomically checkpoint and release the Company Controller while its
    /// durable sibling WorkItems execute.
    async fn park_stage_team_leader(
        &self,
        input: ParkStageTeamLeader,
    ) -> Result<ParkedStageTeamLeaderView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    /// Bind the already-running Controller as the sole final submitter after
    /// its request epoch is closed and the child barrier is complete.
    async fn bind_stage_team_leader_final_submitter(
        &self,
        input: BindStageTeamLeaderFinalSubmitter,
    ) -> Result<BoundStageTeamLeaderFinalSubmitterView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    /// Persist a deterministic Controller Gate BLOCK. While bounded repair
    /// fuel remains, reopen the request epoch and park the exact same
    /// WorkerRun/message chain for immediate continuation. No replacement
    /// Aggregator is created.
    async fn reopen_stage_team_leader_after_gate_block(
        &self,
        input: ReopenStageTeamLeaderAfterGateBlock,
    ) -> Result<ReopenedStageTeamLeaderAfterGateBlockView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    /// Persist a context-bound request for a sibling WorkItem. This records a
    /// durable decision; it never recursively runs another agent in the caller.
    async fn request_stage_worker(
        &self,
        input: RequestStageWorker,
    ) -> Result<RequestedStageWorkerView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn close_stage_request_epoch(
        &self,
        input: CloseStageRequestEpoch,
    ) -> Result<ClosedStageRequestEpochView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn load_stage_team_barrier(
        &self,
        input: LoadStageTeamBarrier,
    ) -> Result<StageTeamBarrierView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    /// Load the immutable producer outputs for one exact TeamPlan. Aggregator
    /// prompts are rebuilt from this durable read after restart; they must not
    /// depend on process-local sibling return values.
    async fn load_stage_team_outputs(
        &self,
        input: LoadStageTeamBarrier,
    ) -> Result<Vec<StageWorkerOutputView>, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn claim_stage_aggregator(
        &self,
        input: ClaimStageAggregator,
    ) -> Result<ClaimedStageWorkItemView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn claim_candidate_attempt(
        &self,
        input: ClaimCandidateAttempt,
    ) -> Result<Option<ClaimedCandidateAttemptView>, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn heartbeat_candidate_attempt(
        &self,
        input: HeartbeatCandidateAttempt,
    ) -> Result<CandidateHeartbeatView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn candidate_execution_continuation(
        &self,
        input: ControlCandidateAttempt,
    ) -> Result<CandidateExecutionContinuationView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn release_candidate_attempt(
        &self,
        input: ControlCandidateAttempt,
    ) -> Result<CandidateReleaseView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn submit_candidate_attempt(
        &self,
        input: SubmitCandidateAttempt,
    ) -> Result<SubmittedCandidateAttemptView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn terminalize_candidate_attempt(
        &self,
        input: TerminalizeCandidateAttempt,
    ) -> Result<TerminalizedCandidateAttemptView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    /// Return the oldest barrier-ready intent for server-owned recovery before
    /// the scheduler is allowed to claim another CandidateAttempt.
    async fn next_candidate_terminal_intent(
        &self,
        operation_id: Uuid,
    ) -> Result<Option<CandidateTerminalIntentView>, RuntimeMemoryError> {
        let _ = operation_id;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn recover_candidate_terminal_intent(
        &self,
        input: RecoverCandidateTerminalIntent,
    ) -> Result<CandidateTerminalBarrierView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    /// Consume a barrier-ready intent using server authority. Unlike the
    /// legacy terminalizer this never requires the original executor lease to
    /// remain alive; the immutable barrier is the authority.
    async fn terminalize_candidate_intent(
        &self,
        input: TerminalizeCandidateIntent,
    ) -> Result<TerminalizedCandidateAttemptView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn resolve_candidate_recovery(
        &self,
        input: ResolveCandidateRecovery,
    ) -> Result<ResolvedCandidateRecoveryView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    /// Revoke running Attempts that can no longer cross their approval's
    /// action-start boundary, before any new Candidate claim is considered.
    async fn expire_candidate_starts_before_claim(
        &self,
        operation_id: Uuid,
    ) -> Result<u32, RuntimeMemoryError> {
        let _ = operation_id;
        Err(RuntimeMemoryError::Unavailable)
    }

    /// Consume the oldest durable operator recovery decision under server
    /// authority. This never replays an external action.
    async fn converge_next_candidate_recovery(
        &self,
        operation_id: Uuid,
    ) -> Result<Option<ConvergedCandidateRecoveryView>, RuntimeMemoryError> {
        let _ = operation_id;
        Err(RuntimeMemoryError::Unavailable)
    }

    /// Close one exact Verification WaveUnit only after its Candidate claim
    /// queue drains and durable terminal truth validates.
    async fn close_attack_v2_verification_unit(
        &self,
        input: CloseAttackV2VerificationUnit,
    ) -> Result<ClosedAttackV2VerificationUnitView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn load_worker_checkpoint(
        &self,
        input: LoadWorkerCheckpoint,
    ) -> Result<LoadedWorkerCheckpoint, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn checkpoint_worker(
        &self,
        input: CheckpointWorker,
    ) -> Result<RuntimeWorkerView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn checkpoint_bound_worker_chain(
        &self,
        input: CheckpointBoundWorkerChain,
    ) -> Result<RuntimeWorkerView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    /// Atomically persist the full bound chain/checkpoint and the exact
    /// Candidate terminal barrier. Splitting these writes would recreate the
    /// response-loss window this protocol closes.
    async fn checkpoint_candidate_terminal_barrier(
        &self,
        input: CheckpointCandidateTerminalBarrier,
    ) -> Result<CandidateTerminalBarrierView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn load_bound_worker_chain(
        &self,
        input: LoadBoundWorkerChain,
    ) -> Result<LoadedBoundWorkerChain, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn reap_expired_worker(
        &self,
        input: LoadWorkerCheckpoint,
    ) -> Result<ReapedRuntimeWorker, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn heartbeat_worker(
        &self,
        fence: RuntimeWorkerFence,
        extend_seconds: i32,
    ) -> Result<RuntimeWorkerView, RuntimeMemoryError> {
        let _ = (fence, extend_seconds);
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn begin_worker_tool(
        &self,
        input: WorkerToolMutation,
    ) -> Result<RuntimeWorkerView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn finish_worker_tool(
        &self,
        input: WorkerToolMutation,
    ) -> Result<RuntimeWorkerView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn finish_worker_attempt(
        &self,
        input: FinishWorkerAttempt,
    ) -> Result<FinishedWorkerAttempt, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    /// Complete a producer/helper Worker and accept one immutable business
    /// Output without changing the StageRunUnit terminal state.
    async fn complete_stage_worker(
        &self,
        input: CompleteStageWorker,
    ) -> Result<CompletedStageWorkerView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    /// Persist execution failure separately from `StageWorkerOutput`. A
    /// provider/runtime failure is never converted into a business `blocked`
    /// result merely to make the sibling barrier advance.
    async fn retry_stage_worker(
        &self,
        input: RetryStageWorker,
    ) -> Result<RetriedStageWorkerView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    /// Local operator resolution for a Worker parked at recovery_required.
    /// This marks the unknown active-tool outcome blocked and never performs
    /// an automatic external-tool replay.
    async fn resolve_stage_team_recovery(
        &self,
        input: ResolveStageTeamRecovery,
    ) -> Result<ResolvedStageTeamRecoveryView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn pause_worker_for_continuation(
        &self,
        input: PauseWorkerForContinuation,
    ) -> Result<FinishedWorkerAttempt, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn finalize_unit_pass(
        &self,
        input: FinalizeUnitPass,
    ) -> Result<FinalizedUnitPass, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    /// The only Team-mode PASS seam. The repository rechecks the closed
    /// manifest and sibling barrier, then atomically closes Aggregator, Unit
    /// and immutable handoff/final seal.
    async fn finalize_stage_team_unit(
        &self,
        input: FinalizeStageTeamUnit,
    ) -> Result<FinalizedStageTeamUnitView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    /// Persist one Aggregator Gate BLOCK and open at most one next bounded
    /// repair generation under the frozen TeamPlan fuel policy.
    async fn open_stage_team_repair(
        &self,
        input: OpenStageTeamRepair,
    ) -> Result<OpenedStageTeamRepairView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    /// Deterministically terminalize a closed Team producer manifest that
    /// contains an immutable blocked output. No Aggregator/model submission is
    /// consulted on this path.
    async fn block_stage_team_unit(
        &self,
        input: BlockStageTeamUnit,
    ) -> Result<BlockedStageTeamUnitView, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn close_wave_gate_pass(
        &self,
        input: CloseWaveGatePass,
    ) -> Result<ClosedWaveGatePass, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn load_inherited_stage_handoffs(
        &self,
        input: LoadInheritedStageHandoffs,
    ) -> Result<Vec<RuntimeStageHandoffView>, RuntimeMemoryError> {
        let _ = input;
        Err(RuntimeMemoryError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn trusted_submission_dto_keeps_server_runtime_identity_separate_from_payload() {
        let operation_id = Uuid::new_v4();
        let stage_execution_id = Uuid::new_v4();
        let stage_run_unit_id = Uuid::new_v4();
        let tool_call_record_id = Uuid::new_v4();
        let input = NewStageDeliverableSubmission {
            operation_id,
            stage_execution_id,
            stage_run_unit_id: Some(stage_run_unit_id),
            worker_run_id: None,
            organization_id: Some(Uuid::new_v4()),
            tool_call_record_id,
            tool_request_id: "trusted-tool-request".to_string(),
            stage_kind: "scoping".to_string(),
            attempt_epoch: None,
            lease_token: None,
            canonical_deliverable_json: format!(
                r#"{{"stage_id":"scoping","stage_run_id":"{stage_execution_id}"}}"#
            ),
            payload_sha256: "a".repeat(64),
        };

        assert_eq!(input.operation_id, operation_id);
        assert_eq!(input.stage_execution_id, stage_execution_id);
        assert_eq!(input.stage_run_unit_id, Some(stage_run_unit_id));
        assert_eq!(input.tool_call_record_id, tool_call_record_id);
    }

    #[test]
    fn captured_submission_carries_durable_id_and_canonical_gate_payload() {
        let captured = CapturedStageSubmission {
            deliverable_submission_id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            stage_execution_id: Uuid::new_v4(),
            stage_run_unit_id: None,
            canonical_deliverable_json: "{\"claims\":[]}".to_string(),
            payload_sha256: "b".repeat(64),
        };
        let persisted = PersistedStageDeliverableSubmission {
            deliverable_submission_id: captured.deliverable_submission_id,
            operation_id: captured.operation_id,
            stage_execution_id: captured.stage_execution_id,
            stage_run_unit_id: captured.stage_run_unit_id,
            worker_run_id: None,
            organization_id: None,
            tool_call_record_id: Uuid::new_v4(),
            tool_request_id: "submit".to_string(),
            stage_kind: "scoping".to_string(),
            attempt_epoch: None,
            lease_token: None,
            payload: json!({"claims": []}),
            payload_sha256: captured.payload_sha256.clone(),
        };

        assert_eq!(
            persisted.deliverable_submission_id,
            captured.deliverable_submission_id
        );
        assert_eq!(persisted.payload_sha256, captured.payload_sha256);
    }

    #[test]
    fn scoping_finalizer_dto_keeps_all_freeze_owner_ids_server_side() {
        let input = FinalizeScopingScope {
            operation_id: Uuid::new_v4(),
            project_scope_id: Uuid::new_v4(),
            stage_execution_id: Uuid::new_v4(),
            root_organization_id: Uuid::new_v4(),
            deliverable_submission_id: Uuid::new_v4(),
            scope_snapshot_id: Uuid::new_v4(),
            scoping_root_unit_id: Uuid::new_v4(),
        };
        assert_ne!(input.operation_id, input.stage_execution_id);
        assert_ne!(input.scope_snapshot_id, input.scoping_root_unit_id);
    }

    #[test]
    fn stage_team_barrier_requires_closed_manifest_and_terminal_required_items() {
        let open_manifest = StageTeamBarrierView {
            stage_team_plan_id: Uuid::new_v4(),
            dispatch_epoch: 3,
            requests_closed_at: None,
            required_work_items: 2,
            terminal_required_work_items: 2,
            live_workers: 0,
            retry_pending_work_items: 0,
            recovery_required_workers: 0,
            missing_outputs: 0,
            manifest_sha256: "sha256:manifest".to_string(),
        };
        assert!(!open_manifest.ready_to_finalize());

        let closed_manifest = StageTeamBarrierView {
            requests_closed_at: Some(chrono::Utc::now()),
            ..open_manifest
        };
        assert!(closed_manifest.ready_to_finalize());
    }

    #[test]
    fn stage_worker_output_separates_execution_success_from_business_blocker() {
        let output = NewStageWorkerOutput {
            work_item_id: Uuid::new_v4(),
            worker_run_id: Uuid::new_v4(),
            output_schema: "stage_worker_output.v1".to_string(),
            disposition: StageWorkerOutputDisposition::Blocked,
            canonical_output: json!({"reason": "provider unavailable"}),
            fact_refs: Vec::new(),
            evidence_ids: Vec::new(),
            checked_empty_units: Vec::new(),
            blocker_code: Some("PROVIDER_UNAVAILABLE".to_string()),
            output_sha256: "sha256:output".to_string(),
        };

        assert_eq!(output.disposition.as_str(), "blocked");
        assert_eq!(output.blocker_code.as_deref(), Some("PROVIDER_UNAVAILABLE"));
    }

    #[test]
    fn candidate_recovery_decisions_are_a_closed_operator_set() {
        for allowed in [
            "terminalize_blocked_outcome_unknown",
            "abandon_before_side_effect",
            "accept_external_result_with_exact_evidence",
        ] {
            assert_eq!(
                CandidateRecoveryDecision::try_parse(allowed)
                    .expect("allowed recovery decision")
                    .as_str(),
                allowed
            );
        }
        assert!(CandidateRecoveryDecision::try_parse("change_target_and_retry").is_none());
        assert!(CandidateRecoveryDecision::try_parse("edit_plan").is_none());
    }
}
