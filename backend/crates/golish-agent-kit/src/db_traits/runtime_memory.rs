//! Typed, sqlx-free contract for operation-scoped runtime-memory persistence.

use async_trait::async_trait;
use serde_json::Value;
use uuid::Uuid;

use super::{OperationStateView, StageAssetWaveView, TaskView};
use crate::runtime_memory::RuntimeMemoryContract;
use crate::task_orchestrator::stage_execution::{
    StageExecution, TransitionStageExecution, TransitionedStageExecution,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeededStageRuntime {
    pub unit: RuntimeStageUnitView,
    pub worker: RuntimeWorkerView,
    pub organization_name: String,
    pub scope_hash: String,
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
    pub attempt_id: Uuid,
    pub disposition: String,
    pub finding_id: Option<Uuid>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStageHandoffView {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub from_stage_kind: String,
    pub stage_execution_id: Uuid,
    pub source_stage_run_unit_id: Uuid,
    pub deliverable_submission_id: Uuid,
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

    async fn claim_worker_and_bind_chain(
        &self,
        input: ClaimWorkerAndBindChain,
    ) -> Result<ClaimedWorkerView, RuntimeMemoryError> {
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
}
