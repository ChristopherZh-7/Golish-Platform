//! SQL-free persistence boundary for the unified Investigation runtime.
//!
//! The application owns the concrete database writer. Runtime callers only
//! receive typed identities and immutable receipts, so neither a pool nor a
//! transaction can escape through this port.

use async_trait::async_trait;
use golish_core::investigation_run_closure::InvestigationRunClosureV1;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub type UnifiedInvestigationRepoResult<T> = Result<T, UnifiedInvestigationRepositoryError>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UnifiedInvestigationRepositoryError {
    #[error("unified_investigation_repository_unavailable: {operation}")]
    Unavailable { operation: &'static str },
    #[error("unified_investigation_repository_invalid_request: {detail}")]
    InvalidRequest { detail: String },
    #[error("unified_investigation_repository_not_found: {detail}")]
    NotFound { detail: String },
    #[error("unified_investigation_repository_conflict: {detail}")]
    Conflict { detail: String },
    #[error("unified_investigation_repository_authority_mismatch: {detail}")]
    AuthorityMismatch { detail: String },
    #[error("unified_investigation_repository_infrastructure: {detail}")]
    Infrastructure { detail: String },
}

impl UnifiedInvestigationRepositoryError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "unified_investigation_repository_unavailable",
            Self::InvalidRequest { .. } => "unified_investigation_repository_invalid_request",
            Self::NotFound { .. } => "unified_investigation_repository_not_found",
            Self::Conflict { .. } => "unified_investigation_repository_conflict",
            Self::AuthorityMismatch { .. } => "unified_investigation_repository_authority_mismatch",
            Self::Infrastructure { .. } => "unified_investigation_repository_infrastructure",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedInvestigationStageIdentity {
    pub authority_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub scope_snapshot_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedInvestigationUnitIdentity {
    pub stage: UnifiedInvestigationStageIdentity,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedInvestigationWorkKind {
    Analysis,
    ReadSession,
    Query,
    Enrichment,
    VerificationTask,
    PentagiSubtask,
    WorkerRequest,
    Campaign,
    PreparedAction,
    ActionExecution,
    FactDelta,
    Consolidation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedInvestigationWorkState {
    Queued,
    Running,
    WaitingAuthorization,
    Unknown,
    StopPending,
    Draining,
    Completed,
    Cancelled,
    Blocked,
    Residual,
    RecoveryRequired,
    FixedPoint,
    Superseded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedInvestigationSubjectKind {
    AnalysisAttempt,
    VerificationTask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedInvestigationActorKind {
    Primary,
    Worker,
    NestedWorker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedInvestigationDispatchOutcome {
    Completed,
    Blocked,
    Residual,
    RecoveryRequired,
    UnknownHeld,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedInvestigationPipelineEventKind {
    GeneratorSealed,
    RefinerPatch,
    ReflectorAttempt,
    ResultBarrier,
    PrimarySynthesis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedInvestigationClosureDisposition {
    Pass,
    PassWithGaps,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartUnifiedInvestigationRun {
    pub identity: UnifiedInvestigationStageIdentity,
    pub stable_start_request_id: Uuid,
    pub initial_change_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedInvestigationRunHead {
    pub authority_id: Uuid,
    pub stable_start_request_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub scope_snapshot_id: Uuid,
    pub run_state: String,
    pub admission_open: bool,
    pub stop_epoch: i64,
    pub change_seq: i64,
    pub head_version: i64,
    pub head_sha256: String,
    pub latest_event_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterUnifiedInvestigationWork {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub work_id: Uuid,
    pub asset_lane_id: Uuid,
    pub stable_work_key_sha256: String,
    pub work_kind: UnifiedInvestigationWorkKind,
    pub external_identity_sha256: String,
    pub initial_state: UnifiedInvestigationWorkState,
    pub observed_stop_epoch: u64,
}

/// Atomically adopts a new dynamic Asset-Primary Analysis work or, when the
/// exact legacy fixed-roster authority still exists and has never executed,
/// cuts that authority over before installing the new work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnsureDynamicAssetAnalysisWork {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub stable_cutover_request_id: Uuid,
    pub asset_lane_id: Uuid,
    pub legacy_stable_work_key_sha256: String,
    pub dynamic_work_id: Uuid,
    pub dynamic_stable_work_key_sha256: String,
    pub dynamic_external_identity_sha256: String,
    pub observed_stop_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnsuredDynamicAssetAnalysisWork {
    pub work: UnifiedInvestigationWork,
    pub cutover_authority_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionUnifiedInvestigationWork {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub work_id: Uuid,
    pub event_id: Uuid,
    pub stable_request_id: Uuid,
    pub expected_head_version: u64,
    pub from_state: UnifiedInvestigationWorkState,
    pub to_state: UnifiedInvestigationWorkState,
    pub observed_stop_epoch: u64,
    pub reason_code: String,
    pub event_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedInvestigationWork {
    pub work_id: Uuid,
    pub asset_lane_id: Uuid,
    pub stable_work_key_sha256: String,
    pub authority_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub work_kind: String,
    pub external_identity_sha256: String,
    pub current_state: String,
    pub observed_stop_epoch: i64,
    pub head_version: i64,
    pub latest_event_id: Option<Uuid>,
}

/// Request-first TaskOrchestrator admission. A plan identifier is deliberately
/// absent: the request must durably exist before a plan can be authored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestUnifiedInvestigationTask {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub run_request_id: Uuid,
    pub stable_request_id: Uuid,
    pub subject_kind: UnifiedInvestigationSubjectKind,
    pub subject_id: Uuid,
    pub subject_fingerprint_sha256: String,
    pub request_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedInvestigationTaskRequest {
    pub run_request_id: Uuid,
    pub stable_request_id: Uuid,
    pub task_plan_id: Option<Uuid>,
    pub authority_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub subject_kind: String,
    pub subject_id: Uuid,
    pub subject_fingerprint_sha256: String,
    pub request_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeginUnifiedInvestigationTaskPlan {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub task_plan_id: Uuid,
    pub stable_request_id: Uuid,
    pub run_request_id: Uuid,
    pub stage_team_plan_id: Uuid,
    pub subject_kind: UnifiedInvestigationSubjectKind,
    pub subject_id: Uuid,
    pub subject_fingerprint_sha256: String,
    pub task_plan_version: u32,
    pub task_plan_sha256: String,
    pub allowed_role_catalog: Value,
    pub cognitive_tool_envelope_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedInvestigationTaskPlan {
    pub task_plan_id: Uuid,
    pub stable_request_id: Uuid,
    pub run_request_id: Uuid,
    pub authority_id: Uuid,
    pub stage_team_plan_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub subject_kind: String,
    pub subject_id: Uuid,
    pub subject_fingerprint_sha256: String,
    pub task_plan_version: i32,
    pub task_plan_sha256: String,
    pub allowed_role_catalog: Value,
    pub cognitive_tool_envelope_sha256: String,
    pub status: String,
    pub subtask_count: Option<i64>,
    pub subtask_set_sha256: Option<String>,
    pub row_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertUnifiedInvestigationSubtask {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub task_plan_id: Uuid,
    pub subtask_id: Uuid,
    pub subtask_ordinal: u32,
    pub label: String,
    pub runnable: bool,
    pub input_manifest_sha256: String,
    pub expected_output_schema: String,
    pub member_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedInvestigationSubtask {
    pub subtask_id: Uuid,
    pub task_plan_id: Uuid,
    pub authority_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub subtask_ordinal: i32,
    pub label: String,
    pub runnable: bool,
    pub input_manifest_sha256: String,
    pub expected_output_schema: String,
    pub member_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertUnifiedInvestigationDispatch {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub dispatch_receipt_id: Uuid,
    pub stable_request_id: Uuid,
    pub logical_dispatch_key_sha256: String,
    pub task_plan_id: Uuid,
    pub subtask_id: Option<Uuid>,
    pub parent_dispatch_receipt_id: Option<Uuid>,
    pub dispatch_ordinal: u32,
    pub actor_kind: UnifiedInvestigationActorKind,
    pub stage_work_item_id: Uuid,
    pub stage_worker_request_id: Option<Uuid>,
    pub worker_run_id: Uuid,
    pub transcript_request_id: String,
    pub parent_actor_transcript_request_id: Option<String>,
    pub parent_dispatch_tool_request_id: Option<String>,
    pub snapshot_sha256: String,
    pub receipt_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedInvestigationDispatch {
    pub dispatch_receipt_id: Uuid,
    pub stable_request_id: Uuid,
    pub logical_dispatch_key_sha256: String,
    pub task_plan_id: Uuid,
    pub subtask_id: Option<Uuid>,
    pub parent_dispatch_receipt_id: Option<Uuid>,
    pub dispatch_ordinal: i32,
    pub actor_kind: String,
    pub stage_work_item_id: Uuid,
    pub stage_worker_request_id: Option<Uuid>,
    pub worker_run_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub transcript_request_id: String,
    pub parent_actor_transcript_request_id: Option<String>,
    pub parent_dispatch_tool_request_id: Option<String>,
    pub snapshot_sha256: String,
    pub receipt_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadUnifiedInvestigationDispatch {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub task_plan_id: Uuid,
    pub dispatch_receipt_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertUnifiedInvestigationDispatchAttempt {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub task_plan_id: Uuid,
    pub dispatch_attempt_id: Uuid,
    pub stable_request_id: Uuid,
    pub dispatch_receipt_id: Uuid,
    pub attempt_epoch: u64,
    pub lease_token: Uuid,
    pub fence_sha256: String,
    pub outcome: UnifiedInvestigationDispatchOutcome,
    pub result_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedInvestigationDispatchAttempt {
    pub dispatch_attempt_id: Uuid,
    pub stable_request_id: Uuid,
    pub dispatch_receipt_id: Uuid,
    pub attempt_epoch: i64,
    pub lease_token: Uuid,
    pub fence_sha256: String,
    pub outcome: String,
    pub result_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertUnifiedInvestigationPipelineEvent {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub pipeline_event_id: Uuid,
    pub stable_request_id: Uuid,
    pub task_plan_id: Uuid,
    pub subtask_id: Option<Uuid>,
    pub event_ordinal: u64,
    pub event_kind: UnifiedInvestigationPipelineEventKind,
    pub actor_worker_run_id: Uuid,
    pub parent_dispatch_receipt_id: Uuid,
    pub event_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedInvestigationPipelineEvent {
    pub pipeline_event_id: Uuid,
    pub stable_request_id: Uuid,
    pub task_plan_id: Uuid,
    pub subtask_id: Option<Uuid>,
    pub event_ordinal: i64,
    pub event_kind: String,
    pub actor_worker_run_id: Uuid,
    pub parent_dispatch_receipt_id: Uuid,
    pub event_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateUnifiedInvestigationRefinerPlanLedger {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub ledger_id: Uuid,
    pub stable_request_id: Uuid,
    pub task_plan_id: Uuid,
    pub generator_pipeline_event_id: Uuid,
    pub generator_manifest: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedInvestigationRefinerPlanLedger {
    pub ledger_id: Uuid,
    pub stable_request_id: Uuid,
    pub task_plan_id: Uuid,
    pub generator_pipeline_event_id: Uuid,
    pub generator_manifest: Value,
    pub generator_manifest_sha256: String,
    pub generator_subtask_count: i64,
    pub generator_subtask_set_sha256: String,
    pub ledger_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadUnifiedInvestigationRefinerPlanLedger {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub task_plan_id: Uuid,
}

/// One canonical Generator member written in the same transaction as the
/// Generator event and Refiner ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedInvestigationGeneratorSubtaskInput {
    pub subtask_id: Uuid,
    pub subtask_ordinal: u32,
    pub label: String,
    pub runnable: bool,
    pub input_manifest_sha256: String,
    pub expected_output_schema: String,
    pub member_sha256: String,
}

/// Exact current-consumer fence.  The durable `submit_result` may belong to
/// the current Primary or to its exact rearm predecessor, while this fence
/// always names the current Primary that is consuming the Generator result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedInvestigationGeneratorConsumerFence {
    pub current_consumer_work_item_id: Uuid,
    pub current_consumer_worker_run_id: Uuid,
    pub current_consumer_lease_token: Uuid,
    pub expected_consumer_attempt_epoch: u64,
    pub expected_consumer_checkpoint_version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializeUnifiedInvestigationGenerator {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub task_plan_id: Uuid,
    pub ledger_id: Uuid,
    pub stable_request_id: Uuid,
    pub generator_pipeline_event_id: Uuid,
    pub source_receipt_id: Uuid,
    pub source_tool_call_id: Uuid,
    pub consumer_fence: UnifiedInvestigationGeneratorConsumerFence,
    pub subtasks: Vec<UnifiedInvestigationGeneratorSubtaskInput>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedInvestigationFinishedSubmitResultCandidate {
    pub source_tool_call_id: Uuid,
    pub source_provider_call_id: String,
    pub source_attempt_epoch: i64,
    pub source_work_item_id: Uuid,
    pub source_worker_run_id: Uuid,
    pub canonical_result: Value,
    pub canonical_result_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadPendingUnifiedInvestigationGeneratorRecovery {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub task_plan_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingUnifiedInvestigationGeneratorRecovery {
    pub task_plan_id: Uuid,
    pub primary_dispatch_receipt_id: Uuid,
    pub primary_work_item_id: Uuid,
    pub primary_worker_run_id: Uuid,
    pub existing_subtasks: Vec<UnifiedInvestigationSubtask>,
    pub candidates: Vec<UnifiedInvestigationFinishedSubmitResultCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdoptUnifiedInvestigationOrphanGenerator {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub task_plan_id: Uuid,
    pub adoption_receipt_id: Uuid,
    pub stable_request_id: Uuid,
    pub ledger_id: Uuid,
    pub ledger_stable_request_id: Uuid,
    pub generator_pipeline_event_id: Uuid,
    pub source_tool_call_id: Uuid,
    pub consumer_fence: UnifiedInvestigationGeneratorConsumerFence,
    pub expected_existing_subtask_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedInvestigationGeneratorMaterialization {
    pub ledger: UnifiedInvestigationRefinerPlanLedger,
    pub subtasks: Vec<UnifiedInvestigationSubtask>,
    pub source: UnifiedInvestigationFinishedSubmitResultCandidate,
    pub source_receipt_id: Uuid,
    pub adoption_receipt_id: Option<Uuid>,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendUnifiedInvestigationRefinerPlanPatch {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub patch_id: Uuid,
    pub stable_request_id: Uuid,
    pub ledger_id: Uuid,
    pub task_plan_id: Uuid,
    pub refiner_pipeline_event_id: Uuid,
    pub expected_previous_state_sha256: String,
    pub remaining_plan_payload: Value,
    pub active_realized_subtask_ids: Vec<Uuid>,
}

/// Dynamic Asset Primary Refiner patch. The ordered active set is the new
/// denominator: members may be added, dropped, retried, or reordered between
/// patches. The repository derives all member/asset authority server-side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendUnifiedInvestigationDynamicRefinerPlanPatch {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub patch_id: Uuid,
    pub stable_request_id: Uuid,
    pub ledger_id: Uuid,
    pub task_plan_id: Uuid,
    pub refiner_pipeline_event_id: Uuid,
    pub expected_previous_state_sha256: String,
    pub remaining_plan_payload: Value,
    pub ordered_active_subtask_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedInvestigationRefinerPlanPatch {
    pub patch_id: Uuid,
    pub stable_request_id: Uuid,
    pub ledger_id: Uuid,
    pub task_plan_id: Uuid,
    pub patch_ordinal: i64,
    pub refiner_pipeline_event_id: Uuid,
    pub expected_previous_state_sha256: String,
    pub remaining_plan_payload: Value,
    pub remaining_plan_payload_sha256: String,
    pub active_realized_subtask_count: i64,
    pub active_realized_subtask_set_sha256: String,
    pub patch_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadLatestUnifiedInvestigationRefinerPlanPatch {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub task_plan_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealUnifiedInvestigationRefinerPlanLedger {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub seal_id: Uuid,
    pub stable_request_id: Uuid,
    pub ledger_id: Uuid,
    pub task_plan_id: Uuid,
    pub result_barrier_pipeline_event_id: Uuid,
    pub expected_final_patch_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealUnifiedInvestigationDynamicRefinerPlanLedger {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub seal_id: Uuid,
    pub stable_request_id: Uuid,
    pub ledger_id: Uuid,
    pub task_plan_id: Uuid,
    pub result_barrier_pipeline_event_id: Uuid,
    pub expected_final_patch_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedInvestigationRefinerPlanLedgerSeal {
    pub seal_id: Uuid,
    pub stable_request_id: Uuid,
    pub ledger_id: Uuid,
    pub task_plan_id: Uuid,
    pub result_barrier_pipeline_event_id: Uuid,
    pub patch_count: i64,
    pub patch_set_sha256: String,
    pub final_patch_id: Uuid,
    pub final_patch_sha256: String,
    pub final_active_realized_subtask_count: i64,
    pub final_active_realized_subtask_set_sha256: String,
    pub generator_subtask_count: i64,
    pub generator_subtask_set_sha256: String,
    pub seal_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadUnifiedInvestigationRefinerPlanLedgerSeal {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub task_plan_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealUnifiedInvestigationDelegationCensus {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub census_seal_id: Uuid,
    pub stable_request_id: Uuid,
    pub task_plan_id: Uuid,
    pub primary_dispatch_receipt_id: Uuid,
    pub primary_worker_run_id: Uuid,
    pub seal_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedInvestigationDelegationCensus {
    pub census_seal_id: Uuid,
    pub stable_request_id: Uuid,
    pub task_plan_id: Uuid,
    pub primary_dispatch_receipt_id: Uuid,
    pub primary_worker_run_id: Uuid,
    pub runnable_subtask_count: i64,
    pub runnable_subtask_set_sha256: String,
    pub dispatch_count: i64,
    pub dispatch_set_sha256: String,
    pub pipeline_event_count: i64,
    pub pipeline_event_set_sha256: String,
    pub seal_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealUnifiedInvestigationTaskPlan {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub task_plan_id: Uuid,
    pub expected_row_version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestUnifiedInvestigationStop {
    pub identity: UnifiedInvestigationStageIdentity,
    pub stop_intent_id: Uuid,
    pub idempotency_key: Uuid,
    pub expected_run_head_sha256: String,
    pub expected_change_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedInvestigationStopIntent {
    pub stop_intent_id: Uuid,
    pub idempotency_key: Uuid,
    pub authority_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub expected_run_head_sha256: String,
    pub expected_change_seq: i64,
    pub stop_epoch: i64,
    pub frozen_work_count: i64,
    pub frozen_work_set_sha256: String,
    pub receipt_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealUnifiedInvestigationClosure {
    pub identity: UnifiedInvestigationStageIdentity,
    pub closure_id: Uuid,
    pub stable_request_id: Uuid,
    pub expected_run_head_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishUnifiedInvestigationClosure {
    pub identity: UnifiedInvestigationStageIdentity,
    pub publication_id: Uuid,
    pub stable_request_id: Uuid,
    pub closure_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnifiedInvestigationClosurePublicationMember {
    pub publication_member_id: Uuid,
    pub publication_id: Uuid,
    pub member_ordinal: i32,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub stage_team_plan_id: Uuid,
    pub member_sha256: String,
    pub passed_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnifiedInvestigationClosurePublication {
    pub publication_id: Uuid,
    pub closure_id: Uuid,
    pub authority_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub closure_sha256: String,
    pub disposition: String,
    pub member_count: i64,
    pub member_set_sha256: String,
    pub publication_sha256: String,
    pub published_at: chrono::DateTime<chrono::Utc>,
    pub members: Vec<UnifiedInvestigationClosurePublicationMember>,
    pub replayed: bool,
}

impl UnifiedInvestigationClosurePublication {
    /// Return the immutable per-organization completion authority carried by
    /// this publication. Investigation closeout must use this exact set as its
    /// pass-token denominator: an engagement organization subtree is mutable
    /// and can contain organizations that the operation-frozen scope excluded.
    ///
    /// The persistence adapter remains responsible for recomputing the
    /// publication hashes. This SQL-free boundary still validates the complete
    /// owner/shape projection so an incomplete or foreign adapter result cannot
    /// silently become closeout authority.
    pub fn exact_completion_authority(
        &self,
        expected_operation_id: Uuid,
    ) -> UnifiedInvestigationRepoResult<Vec<(Uuid, chrono::DateTime<chrono::Utc>)>> {
        let mismatch =
            |detail: &'static str| UnifiedInvestigationRepositoryError::AuthorityMismatch {
                detail: detail.to_string(),
            };
        if expected_operation_id.is_nil()
            || self.publication_id.is_nil()
            || self.closure_id.is_nil()
            || self.authority_id.is_nil()
            || self.stage_execution_id.is_nil()
            || self.scope_snapshot_id.is_nil()
            || self.operation_id != expected_operation_id
        {
            return Err(mismatch("closure_publication_identity_mismatch"));
        }
        if !matches!(self.disposition.as_str(), "pass" | "pass_with_gaps") {
            return Err(mismatch("closure_publication_disposition_not_pass"));
        }
        if self.member_count <= 0
            || i64::try_from(self.members.len()).ok() != Some(self.member_count)
        {
            return Err(mismatch("closure_publication_member_count_mismatch"));
        }

        let mut organization_ids = std::collections::BTreeSet::new();
        let mut unit_ids = std::collections::BTreeSet::new();
        let mut completion_authority = Vec::with_capacity(self.members.len());
        for (expected_ordinal, member) in self.members.iter().enumerate() {
            if member.publication_member_id.is_nil()
                || member.publication_id != self.publication_id
                || member.member_ordinal != i32::try_from(expected_ordinal).unwrap_or(i32::MAX)
                || member.operation_id != self.operation_id
                || member.stage_execution_id != self.stage_execution_id
                || member.scope_snapshot_id != self.scope_snapshot_id
                || member.stage_run_unit_id.is_nil()
                || member.organization_id.is_nil()
                || member.stage_team_plan_id.is_nil()
                || member.member_sha256.trim().is_empty()
                || !organization_ids.insert(member.organization_id)
                || !unit_ids.insert(member.stage_run_unit_id)
            {
                return Err(mismatch("closure_publication_member_authority_mismatch"));
            }
            completion_authority.push((member.organization_id, member.passed_at));
        }
        completion_authority.sort_by_key(|(organization_id, _)| *organization_id);
        Ok(completion_authority)
    }
}

/// One host-bound organization member of the complete Investigation stage
/// read-session set. A raw ContextPack body is intentionally unrepresentable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedInvestigationMainReadSessionMember {
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub snapshot_id: Uuid,
    pub snapshot_sha256: String,
    pub context_item_count: u32,
    pub context_item_set_sha256: String,
    pub methodology_hit_count: u32,
    pub methodology_result_set_sha256: String,
    pub omission_count: u32,
    pub omission_set_sha256: String,
    pub context_chain_id: Uuid,
    pub transcript_partition_id: Uuid,
    pub receipt_id: Uuid,
}

/// Open, populate and seal the complete non-superseded Unit denominator for an
/// Investigation stage. The adapter derives each immutable session/receipt;
/// callers provide only exact partitions and redacted counts/hashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAndSealUnifiedInvestigationMainReadSessionSet {
    pub identity: UnifiedInvestigationStageIdentity,
    pub session_set_id: Uuid,
    pub session_set_stable_request_id: Uuid,
    pub session_set_ordinal: u64,
    pub members: Vec<UnifiedInvestigationMainReadSessionMember>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedInvestigationMainReadSessionReceipt {
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub snapshot_id: Uuid,
    pub snapshot_sha256: String,
    pub main_read_session_id: Uuid,
    pub context_chain_id: Uuid,
    pub transcript_partition_id: Uuid,
    pub session_contract_version: String,
    pub receipt_id: Uuid,
    pub receipt_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedInvestigationMainReadSessionSetSeal {
    pub authority_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub scope_snapshot_id: Uuid,
    pub session_set_id: Uuid,
    pub member_count: i64,
    pub member_set_sha256: String,
    pub row_version: i64,
    pub receipts: Vec<UnifiedInvestigationMainReadSessionReceipt>,
}

/// Immutable per-organization read authority recovered after the stage-wide
/// read-session denominator has sealed. Resume callers must reuse this exact
/// snapshot identity instead of rebuilding it from later runtime projections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedInvestigationMainReadSessionAuthority {
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub snapshot_id: Uuid,
    pub snapshot_sha256: String,
    pub context_item_count: u32,
    pub context_item_set_sha256: String,
    pub methodology_hit_count: u32,
    pub methodology_result_set_sha256: String,
    pub omission_count: u32,
    pub omission_set_sha256: String,
    pub main_read_session_id: Uuid,
    pub context_chain_id: Uuid,
    pub transcript_partition_id: Uuid,
}

#[async_trait]
pub trait UnifiedInvestigationRepository: Send + Sync {
    async fn start_run(
        &self,
        request: StartUnifiedInvestigationRun,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationRunHead>;

    async fn load_run_head(
        &self,
        identity: UnifiedInvestigationStageIdentity,
    ) -> UnifiedInvestigationRepoResult<Option<UnifiedInvestigationRunHead>>;

    async fn register_work(
        &self,
        request: RegisterUnifiedInvestigationWork,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationWork>;

    async fn ensure_dynamic_asset_analysis_work(
        &self,
        request: EnsureDynamicAssetAnalysisWork,
    ) -> UnifiedInvestigationRepoResult<EnsuredDynamicAssetAnalysisWork>;

    async fn transition_work(
        &self,
        request: TransitionUnifiedInvestigationWork,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationWork>;

    async fn request_task(
        &self,
        request: RequestUnifiedInvestigationTask,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationTaskRequest>;

    async fn begin_task_plan(
        &self,
        request: BeginUnifiedInvestigationTaskPlan,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationTaskPlan>;

    async fn insert_subtask(
        &self,
        request: InsertUnifiedInvestigationSubtask,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationSubtask>;

    async fn insert_dispatch(
        &self,
        request: InsertUnifiedInvestigationDispatch,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationDispatch>;

    async fn load_dispatch(
        &self,
        request: LoadUnifiedInvestigationDispatch,
    ) -> UnifiedInvestigationRepoResult<Option<UnifiedInvestigationDispatch>> {
        let _ = request;
        Err(UnifiedInvestigationRepositoryError::Unavailable {
            operation: "load_dispatch",
        })
    }

    async fn insert_dispatch_attempt(
        &self,
        request: InsertUnifiedInvestigationDispatchAttempt,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationDispatchAttempt>;

    async fn insert_pipeline_event(
        &self,
        request: InsertUnifiedInvestigationPipelineEvent,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationPipelineEvent>;

    async fn create_refiner_plan_ledger(
        &self,
        request: CreateUnifiedInvestigationRefinerPlanLedger,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationRefinerPlanLedger>;

    async fn materialize_generator(
        &self,
        request: MaterializeUnifiedInvestigationGenerator,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationGeneratorMaterialization>;

    async fn load_pending_generator_recovery(
        &self,
        request: LoadPendingUnifiedInvestigationGeneratorRecovery,
    ) -> UnifiedInvestigationRepoResult<Option<PendingUnifiedInvestigationGeneratorRecovery>>;

    async fn adopt_orphan_generator(
        &self,
        request: AdoptUnifiedInvestigationOrphanGenerator,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationGeneratorMaterialization>;

    async fn load_refiner_plan_ledger(
        &self,
        request: LoadUnifiedInvestigationRefinerPlanLedger,
    ) -> UnifiedInvestigationRepoResult<Option<UnifiedInvestigationRefinerPlanLedger>> {
        let _ = request;
        Err(UnifiedInvestigationRepositoryError::Unavailable {
            operation: "load_refiner_plan_ledger",
        })
    }

    async fn append_refiner_plan_patch(
        &self,
        request: AppendUnifiedInvestigationRefinerPlanPatch,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationRefinerPlanPatch>;

    async fn append_dynamic_refiner_plan_patch(
        &self,
        request: AppendUnifiedInvestigationDynamicRefinerPlanPatch,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationRefinerPlanPatch>;

    async fn load_latest_refiner_plan_patch(
        &self,
        request: LoadLatestUnifiedInvestigationRefinerPlanPatch,
    ) -> UnifiedInvestigationRepoResult<Option<UnifiedInvestigationRefinerPlanPatch>> {
        let _ = request;
        Err(UnifiedInvestigationRepositoryError::Unavailable {
            operation: "load_latest_refiner_plan_patch",
        })
    }

    async fn seal_refiner_plan_ledger(
        &self,
        request: SealUnifiedInvestigationRefinerPlanLedger,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationRefinerPlanLedgerSeal>;

    async fn seal_dynamic_refiner_plan_ledger(
        &self,
        request: SealUnifiedInvestigationDynamicRefinerPlanLedger,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationRefinerPlanLedgerSeal>;

    async fn load_refiner_plan_ledger_seal(
        &self,
        request: LoadUnifiedInvestigationRefinerPlanLedgerSeal,
    ) -> UnifiedInvestigationRepoResult<Option<UnifiedInvestigationRefinerPlanLedgerSeal>> {
        let _ = request;
        Err(UnifiedInvestigationRepositoryError::Unavailable {
            operation: "load_refiner_plan_ledger_seal",
        })
    }

    async fn seal_delegation_census(
        &self,
        request: SealUnifiedInvestigationDelegationCensus,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationDelegationCensus>;

    async fn seal_task_plan(
        &self,
        request: SealUnifiedInvestigationTaskPlan,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationTaskPlan>;

    async fn request_stop(
        &self,
        request: RequestUnifiedInvestigationStop,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationStopIntent>;

    /// Reloads the one durable stop receipt for an exact stage authority.
    /// This is the response-loss path between stop commit and closure commit;
    /// it never creates a second stop epoch or relaxes the run-head CAS.
    async fn load_stop_intent(
        &self,
        identity: UnifiedInvestigationStageIdentity,
    ) -> UnifiedInvestigationRepoResult<Option<UnifiedInvestigationStopIntent>>;

    async fn seal_closure(
        &self,
        request: SealUnifiedInvestigationClosure,
    ) -> UnifiedInvestigationRepoResult<InvestigationRunClosureV1>;

    async fn load_closure(
        &self,
        identity: UnifiedInvestigationStageIdentity,
    ) -> UnifiedInvestigationRepoResult<Option<InvestigationRunClosureV1>>;

    /// Atomically projects one sealed fixed-point closure into passed Units and
    /// per-org completion rows. No Worker-authored deliverable is fabricated.
    async fn publish_closure(
        &self,
        request: PublishUnifiedInvestigationClosure,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationClosurePublication>;

    /// Reload and validate the exact immutable closure publication for one
    /// operation. `None` means the stage has not been published; duplicate or
    /// drifted authority is a typed error rather than an arbitrary row choice.
    async fn load_closure_publication_for_operation(
        &self,
        operation_id: Uuid,
    ) -> UnifiedInvestigationRepoResult<Option<UnifiedInvestigationClosurePublication>>;

    async fn open_and_seal_main_read_session_set(
        &self,
        request: OpenAndSealUnifiedInvestigationMainReadSessionSet,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationMainReadSessionSetSeal>;

    async fn load_main_read_session_authority(
        &self,
        identity: UnifiedInvestigationUnitIdentity,
    ) -> UnifiedInvestigationRepoResult<Option<UnifiedInvestigationMainReadSessionAuthority>>;
}

#[cfg(test)]
mod closure_publication_tests {
    use super::*;

    fn root_only_publication() -> UnifiedInvestigationClosurePublication {
        let publication_id = Uuid::from_u128(1);
        let operation_id = Uuid::from_u128(2);
        let stage_execution_id = Uuid::from_u128(3);
        let scope_snapshot_id = Uuid::from_u128(4);
        UnifiedInvestigationClosurePublication {
            publication_id,
            closure_id: Uuid::from_u128(5),
            authority_id: Uuid::from_u128(6),
            operation_id,
            stage_execution_id,
            scope_snapshot_id,
            closure_sha256: "closure".to_string(),
            disposition: "pass".to_string(),
            member_count: 1,
            member_set_sha256: "members".to_string(),
            publication_sha256: "publication".to_string(),
            published_at: chrono::Utc::now(),
            members: vec![UnifiedInvestigationClosurePublicationMember {
                publication_member_id: Uuid::from_u128(7),
                publication_id,
                member_ordinal: 0,
                operation_id,
                stage_execution_id,
                scope_snapshot_id,
                stage_run_unit_id: Uuid::from_u128(8),
                organization_id: Uuid::from_u128(9),
                stage_team_plan_id: Uuid::from_u128(10),
                member_sha256: "member".to_string(),
                passed_at: chrono::Utc::now(),
            }],
            replayed: true,
        }
    }

    #[test]
    fn root_only_publication_exposes_exact_completion_authority() {
        let publication = root_only_publication();
        let authority = publication
            .exact_completion_authority(publication.operation_id)
            .expect("valid immutable publication");

        assert_eq!(
            authority,
            vec![(
                publication.members[0].organization_id,
                publication.members[0].passed_at,
            )]
        );
    }

    #[test]
    fn completion_authority_rejects_foreign_or_duplicate_members() {
        let mut foreign = root_only_publication();
        foreign.members[0].operation_id = Uuid::from_u128(11);
        assert!(matches!(
            foreign.exact_completion_authority(foreign.operation_id),
            Err(UnifiedInvestigationRepositoryError::AuthorityMismatch { .. })
        ));

        let mut duplicate = root_only_publication();
        let mut second = duplicate.members[0].clone();
        second.publication_member_id = Uuid::from_u128(12);
        second.stage_run_unit_id = Uuid::from_u128(13);
        second.stage_team_plan_id = Uuid::from_u128(14);
        second.member_ordinal = 1;
        duplicate.members.push(second);
        duplicate.member_count = 2;
        assert!(matches!(
            duplicate.exact_completion_authority(duplicate.operation_id),
            Err(UnifiedInvestigationRepositoryError::AuthorityMismatch { .. })
        ));
    }
}
