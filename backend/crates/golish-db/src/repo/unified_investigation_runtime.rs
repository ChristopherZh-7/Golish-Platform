//! DB-backed typed authority for the unified Investigation runtime.
//!
//! This repository is intentionally orchestration-free. It persists exact
//! operation/stage-request/unit/organization identities and delegates replay,
//! CAS, exact-set sealing, stop fencing, and closure validation to migration
//! `20260802000013_unified_investigation_runtime_closure.sql` and its
//! forward-only full closure upgrade in `20260802000016`.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum UnifiedInvestigationRuntimeStoreError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error("invalid unified Investigation runtime input: {0}")]
    InvalidInput(&'static str),
    #[error("unified Investigation runtime identity conflict: {0}")]
    IdentityConflict(&'static str),
    #[error("unified Investigation runtime CAS conflict: {0}")]
    CasConflict(&'static str),
}

pub type UnifiedInvestigationRuntimeStoreResult<T> =
    Result<T, UnifiedInvestigationRuntimeStoreError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationStageIdentity {
    pub authority_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub scope_snapshot_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationUnitIdentity {
    pub stage: InvestigationStageIdentity,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvestigationWorkKind {
    Analysis,
    ReadSession,
    Query,
    Enrichment,
    Outbox,
    VerificationTask,
    PentagiSubtask,
    WorkerRequest,
    Campaign,
    PreparedAction,
    ActionExecution,
    FactDelta,
    Consolidation,
}

impl InvestigationWorkKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Analysis => "analysis",
            Self::ReadSession => "read_session",
            Self::Query => "query",
            Self::Enrichment => "enrichment",
            Self::Outbox => "outbox",
            Self::VerificationTask => "verification_task",
            Self::PentagiSubtask => "pentagi_subtask",
            Self::WorkerRequest => "worker_request",
            Self::Campaign => "campaign",
            Self::PreparedAction => "prepared_action",
            Self::ActionExecution => "action_execution",
            Self::FactDelta => "fact_delta",
            Self::Consolidation => "consolidation",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvestigationWorkState {
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

impl InvestigationWorkState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::WaitingAuthorization => "waiting_authorization",
            Self::Unknown => "unknown",
            Self::StopPending => "stop_pending",
            Self::Draining => "draining",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Blocked => "blocked",
            Self::Residual => "residual",
            Self::RecoveryRequired => "recovery_required",
            Self::FixedPoint => "fixed_point",
            Self::Superseded => "superseded",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PentagiSubjectKind {
    AnalysisAttempt,
    VerificationTask,
}

impl PentagiSubjectKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::AnalysisAttempt => "analysis_attempt",
            Self::VerificationTask => "verification_task",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PentagiActorKind {
    Primary,
    Worker,
    NestedWorker,
}

impl PentagiActorKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Worker => "worker",
            Self::NestedWorker => "nested_worker",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PentagiDispatchOutcome {
    Completed,
    Blocked,
    Residual,
    RecoveryRequired,
    UnknownHeld,
}

impl PentagiDispatchOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Blocked => "blocked",
            Self::Residual => "residual",
            Self::RecoveryRequired => "recovery_required",
            Self::UnknownHeld => "unknown_held",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PentagiPipelineEventKind {
    GeneratorSealed,
    RefinerPatch,
    ReflectorAttempt,
    ResultBarrier,
    PrimarySynthesis,
}

impl PentagiPipelineEventKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::GeneratorSealed => "generator_sealed",
            Self::RefinerPatch => "refiner_patch",
            Self::ReflectorAttempt => "reflector_attempt",
            Self::ResultBarrier => "result_barrier",
            Self::PrimarySynthesis => "primary_synthesis",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvestigationClosureDisposition {
    Pass,
    PassWithGaps,
    Stopped,
}

impl InvestigationClosureDisposition {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::PassWithGaps => "pass_with_gaps",
            Self::Stopped => "stopped",
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct InvestigationRunHeadRow {
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
pub struct StartInvestigationRunInput {
    pub identity: InvestigationStageIdentity,
    pub stable_start_request_id: Uuid,
    pub initial_change_seq: u64,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct InvestigationRunWorkRow {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterInvestigationWorkInput {
    pub identity: InvestigationUnitIdentity,
    pub work_id: Uuid,
    pub asset_lane_id: Uuid,
    pub stable_work_key_sha256: String,
    pub work_kind: InvestigationWorkKind,
    pub external_identity_sha256: String,
    pub initial_state: InvestigationWorkState,
    pub observed_stop_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnsureDynamicAssetAnalysisWorkInput {
    pub identity: InvestigationUnitIdentity,
    pub stable_cutover_request_id: Uuid,
    pub asset_lane_id: Uuid,
    pub legacy_stable_work_key_sha256: String,
    pub dynamic_work_id: Uuid,
    pub dynamic_stable_work_key_sha256: String,
    pub dynamic_external_identity_sha256: String,
    pub observed_stop_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnsuredDynamicAssetAnalysisWorkRow {
    pub work: InvestigationRunWorkRow,
    pub cutover_authority_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionInvestigationWorkInput {
    pub identity: InvestigationUnitIdentity,
    pub work_id: Uuid,
    pub event_id: Uuid,
    pub stable_request_id: Uuid,
    pub expected_head_version: u64,
    pub from_state: InvestigationWorkState,
    pub to_state: InvestigationWorkState,
    pub observed_stop_epoch: u64,
    pub reason_code: String,
    pub event_sha256: String,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct PentagiTaskPlanRow {
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
pub struct BeginPentagiTaskPlanInput {
    pub identity: InvestigationUnitIdentity,
    pub task_plan_id: Uuid,
    pub stable_request_id: Uuid,
    pub run_request_id: Uuid,
    pub stage_team_plan_id: Uuid,
    pub subject_kind: PentagiSubjectKind,
    pub subject_id: Uuid,
    pub subject_fingerprint_sha256: String,
    pub task_plan_version: u32,
    pub task_plan_sha256: String,
    pub allowed_role_catalog: Value,
    pub cognitive_tool_envelope_sha256: String,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct PentagiSubtaskRow {
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
pub struct InsertPentagiSubtaskInput {
    pub identity: InvestigationUnitIdentity,
    pub task_plan_id: Uuid,
    pub subtask_id: Uuid,
    pub subtask_ordinal: u32,
    pub label: String,
    pub runnable: bool,
    pub input_manifest_sha256: String,
    pub expected_output_schema: String,
    pub member_sha256: String,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct PentagiTaskRunRequestRow {
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
pub struct InsertPentagiTaskRunRequestInput {
    pub identity: InvestigationUnitIdentity,
    pub run_request_id: Uuid,
    pub stable_request_id: Uuid,
    pub subject_kind: PentagiSubjectKind,
    pub subject_id: Uuid,
    pub subject_fingerprint_sha256: String,
    pub request_sha256: String,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct PentagiLogicalDispatchRow {
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
pub struct InsertPentagiLogicalDispatchInput {
    pub identity: InvestigationUnitIdentity,
    pub dispatch_receipt_id: Uuid,
    pub stable_request_id: Uuid,
    pub logical_dispatch_key_sha256: String,
    pub task_plan_id: Uuid,
    pub subtask_id: Option<Uuid>,
    pub parent_dispatch_receipt_id: Option<Uuid>,
    pub dispatch_ordinal: u32,
    pub actor_kind: PentagiActorKind,
    pub stage_work_item_id: Uuid,
    pub stage_worker_request_id: Option<Uuid>,
    pub worker_run_id: Uuid,
    pub transcript_request_id: String,
    pub parent_actor_transcript_request_id: Option<String>,
    pub parent_dispatch_tool_request_id: Option<String>,
    pub snapshot_sha256: String,
    pub receipt_sha256: String,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct PentagiDispatchAttemptRow {
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
pub struct InsertPentagiDispatchAttemptInput {
    pub identity: InvestigationUnitIdentity,
    pub task_plan_id: Uuid,
    pub dispatch_attempt_id: Uuid,
    pub stable_request_id: Uuid,
    pub dispatch_receipt_id: Uuid,
    pub attempt_epoch: u64,
    pub lease_token: Uuid,
    pub fence_sha256: String,
    pub outcome: PentagiDispatchOutcome,
    pub result_sha256: String,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct PentagiPipelineEventRow {
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
pub struct InsertPentagiPipelineEventInput {
    pub identity: InvestigationUnitIdentity,
    pub pipeline_event_id: Uuid,
    pub stable_request_id: Uuid,
    pub task_plan_id: Uuid,
    pub subtask_id: Option<Uuid>,
    pub event_ordinal: u64,
    pub event_kind: PentagiPipelineEventKind,
    pub actor_worker_run_id: Uuid,
    pub parent_dispatch_receipt_id: Uuid,
    pub event_sha256: String,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct InvestigationRefinerPlanLedgerRow {
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
pub struct CreateInvestigationRefinerPlanLedgerInput {
    pub identity: InvestigationUnitIdentity,
    pub ledger_id: Uuid,
    pub stable_request_id: Uuid,
    pub task_plan_id: Uuid,
    pub generator_pipeline_event_id: Uuid,
    pub generator_manifest: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationGeneratorSubtaskInput {
    pub subtask_id: Uuid,
    pub subtask_ordinal: u32,
    pub label: String,
    pub runnable: bool,
    pub input_manifest_sha256: String,
    pub expected_output_schema: String,
    pub member_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationGeneratorConsumerFenceInput {
    pub current_consumer_work_item_id: Uuid,
    pub current_consumer_worker_run_id: Uuid,
    pub current_consumer_lease_token: Uuid,
    pub expected_consumer_attempt_epoch: u64,
    pub expected_consumer_checkpoint_version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializeInvestigationGeneratorInput {
    pub identity: InvestigationUnitIdentity,
    pub task_plan_id: Uuid,
    pub ledger_id: Uuid,
    pub stable_request_id: Uuid,
    pub generator_pipeline_event_id: Uuid,
    pub source_receipt_id: Uuid,
    pub source_tool_call_id: Uuid,
    pub consumer_fence: InvestigationGeneratorConsumerFenceInput,
    pub subtasks: Vec<InvestigationGeneratorSubtaskInput>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct InvestigationFinishedSubmitResultCandidateRow {
    pub source_tool_call_id: Uuid,
    pub source_provider_call_id: String,
    pub source_attempt_epoch: i64,
    pub source_work_item_id: Uuid,
    pub source_worker_run_id: Uuid,
    pub canonical_result: Value,
    pub canonical_result_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingInvestigationGeneratorRecoveryRow {
    pub task_plan_id: Uuid,
    pub primary_dispatch_receipt_id: Uuid,
    pub primary_work_item_id: Uuid,
    pub primary_worker_run_id: Uuid,
    pub existing_subtasks: Vec<PentagiSubtaskRow>,
    pub candidates: Vec<InvestigationFinishedSubmitResultCandidateRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdoptInvestigationOrphanGeneratorInput {
    pub identity: InvestigationUnitIdentity,
    pub task_plan_id: Uuid,
    pub adoption_receipt_id: Uuid,
    pub stable_request_id: Uuid,
    pub ledger_id: Uuid,
    pub ledger_stable_request_id: Uuid,
    pub generator_pipeline_event_id: Uuid,
    pub source_tool_call_id: Uuid,
    pub consumer_fence: InvestigationGeneratorConsumerFenceInput,
    pub expected_existing_subtask_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationGeneratorMaterializationRow {
    pub ledger: InvestigationRefinerPlanLedgerRow,
    pub subtasks: Vec<PentagiSubtaskRow>,
    pub source: InvestigationFinishedSubmitResultCandidateRow,
    pub source_receipt_id: Uuid,
    pub adoption_receipt_id: Option<Uuid>,
    pub replayed: bool,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct InvestigationGeneratorSourceReceiptRow {
    source_receipt_id: Uuid,
    stable_request_id: Uuid,
    task_plan_id: Uuid,
    ledger_id: Uuid,
    generator_pipeline_event_id: Uuid,
    source_tool_call_id: Uuid,
    source_provider_call_id: String,
    source_attempt_epoch: i64,
    source_work_item_id: Uuid,
    source_worker_run_id: Uuid,
    current_consumer_work_item_id: Uuid,
    current_consumer_worker_run_id: Uuid,
    current_consumer_lease_token: Uuid,
    current_consumer_attempt_epoch: i64,
    current_consumer_checkpoint_version: i64,
    canonical_result_sha256: String,
    adopted_subtask_count: i64,
    adopted_subtask_set_sha256: String,
    receipt_kind: String,
    receipt_sha256: String,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct InvestigationRefinerPlanPatchRow {
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
pub struct AppendInvestigationRefinerPlanPatchInput {
    pub identity: InvestigationUnitIdentity,
    pub patch_id: Uuid,
    pub stable_request_id: Uuid,
    pub ledger_id: Uuid,
    pub task_plan_id: Uuid,
    pub refiner_pipeline_event_id: Uuid,
    pub expected_previous_state_sha256: String,
    pub remaining_plan_payload: Value,
    pub active_realized_subtask_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendInvestigationDynamicRefinerPlanPatchInput {
    pub identity: InvestigationUnitIdentity,
    pub patch_id: Uuid,
    pub stable_request_id: Uuid,
    pub ledger_id: Uuid,
    pub task_plan_id: Uuid,
    pub refiner_pipeline_event_id: Uuid,
    pub expected_previous_state_sha256: String,
    pub remaining_plan_payload: Value,
    pub ordered_active_subtask_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct InvestigationRefinerPlanLedgerSealRow {
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
pub struct SealInvestigationRefinerPlanLedgerInput {
    pub identity: InvestigationUnitIdentity,
    pub seal_id: Uuid,
    pub stable_request_id: Uuid,
    pub ledger_id: Uuid,
    pub task_plan_id: Uuid,
    pub result_barrier_pipeline_event_id: Uuid,
    pub expected_final_patch_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealInvestigationDynamicRefinerPlanLedgerInput {
    pub identity: InvestigationUnitIdentity,
    pub seal_id: Uuid,
    pub stable_request_id: Uuid,
    pub ledger_id: Uuid,
    pub task_plan_id: Uuid,
    pub result_barrier_pipeline_event_id: Uuid,
    pub expected_final_patch_sha256: String,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct PentagiDelegationCensusRow {
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
pub struct SealPentagiDelegationCensusInput {
    pub identity: InvestigationUnitIdentity,
    pub census_seal_id: Uuid,
    pub stable_request_id: Uuid,
    pub task_plan_id: Uuid,
    pub primary_dispatch_receipt_id: Uuid,
    pub primary_worker_run_id: Uuid,
    pub seal_sha256: String,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct InvestigationStopIntentRow {
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
pub struct RequestInvestigationStopInput {
    pub identity: InvestigationStageIdentity,
    pub stop_intent_id: Uuid,
    pub idempotency_key: Uuid,
    pub expected_run_head_sha256: String,
    pub expected_change_seq: u64,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct InvestigationRunClosureRow {
    pub closure_id: Uuid,
    pub stable_request_id: Uuid,
    pub authority_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub scope_snapshot_id: Uuid,
    pub run_state_head_version: i64,
    pub stop_epoch: i64,
    pub snapshot_member_count: i64,
    pub snapshot_member_set_sha256: String,
    pub main_read_session_member_count: i64,
    pub main_read_session_member_set_sha256: String,
    pub generation_member_count: i64,
    pub generation_member_set_sha256: String,
    pub admission_member_count: i64,
    pub admission_member_set_sha256: String,
    pub verification_task_member_count: i64,
    pub verification_task_member_set_sha256: String,
    pub objective_assignment_member_count: i64,
    pub objective_assignment_member_set_sha256: String,
    pub objective_outcome_member_count: i64,
    pub objective_outcome_member_set_sha256: String,
    pub work_total_count: i64,
    pub work_terminal_count: i64,
    pub work_cancelled_before_start_count: i64,
    pub work_recovery_required_count: i64,
    pub work_member_set_sha256: String,
    pub campaign_total_count: i64,
    pub campaign_terminal_count: i64,
    pub campaign_cancelled_before_start_count: i64,
    pub campaign_recovery_required_count: i64,
    pub campaign_member_set_sha256: String,
    pub prepared_action_total_count: i64,
    pub prepared_action_terminal_count: i64,
    pub prepared_action_cancelled_before_start_count: i64,
    pub prepared_action_recovery_required_count: i64,
    pub prepared_action_member_set_sha256: String,
    pub fact_delta_total_count: i64,
    pub fact_delta_terminal_count: i64,
    pub fact_delta_cancelled_before_start_count: i64,
    pub fact_delta_recovery_required_count: i64,
    pub fact_delta_member_set_sha256: String,
    pub delegation_task_count: i64,
    pub delegation_primary_count: i64,
    pub delegation_runnable_subtask_count: i64,
    pub delegation_independently_dispatched_subtask_count: i64,
    pub delegation_logical_dispatch_count: i64,
    pub delegation_unique_logical_dispatch_count: i64,
    pub delegation_sealed_task_census_count: i64,
    pub delegation_member_set_sha256: String,
    pub fuel_reservation_count: i64,
    pub fuel_consumed_count: i64,
    pub fuel_refunded_count: i64,
    pub fuel_unknown_held_count: i64,
    pub fuel_open_count: i64,
    pub fuel_semantic_cycle_count: i64,
    pub fuel_reservation_set_sha256: String,
    pub fuel_semantic_cycle_set_sha256: String,
    pub fixed_point_receipt_id: Uuid,
    pub fixed_point_receipt_sha256: String,
    pub residual_member_count: i64,
    pub residual_member_set_sha256: String,
    pub disposition: String,
    pub contract_version: String,
    pub closure_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishInvestigationStageClosureInput {
    pub identity: InvestigationStageIdentity,
    pub publication_id: Uuid,
    pub stable_request_id: Uuid,
    pub closure_id: Uuid,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct InvestigationStageClosurePublicationRow {
    pub publication_id: Uuid,
    pub stable_request_id: Uuid,
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
    pub published_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct InvestigationStageClosurePublicationMemberRow {
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
    pub passed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedInvestigationStageClosureRow {
    pub publication: InvestigationStageClosurePublicationRow,
    pub members: Vec<InvestigationStageClosurePublicationMemberRow>,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealInvestigationRunClosureInput {
    pub identity: InvestigationStageIdentity,
    pub closure_id: Uuid,
    pub stable_request_id: Uuid,
    pub expected_run_head_sha256: String,
}

#[derive(Clone)]
pub struct PgUnifiedInvestigationRuntimeRepository {
    pool: Arc<PgPool>,
}

impl PgUnifiedInvestigationRuntimeRepository {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }

    pub async fn start_run(
        &self,
        input: &StartInvestigationRunInput,
    ) -> UnifiedInvestigationRuntimeStoreResult<InvestigationRunHeadRow> {
        validate_stage_identity(&input.identity)?;
        validate_ids(&[input.stable_start_request_id])?;
        let change_seq = to_i64(input.initial_change_seq, "initial_change_seq")?;
        Ok(sqlx::query_as::<_, InvestigationRunHeadRow>(
            r#"SELECT authority_id,stable_start_request_id,operation_id,stage_execution_id,
                      owning_stage_run_request_id,scope_snapshot_id,run_state,admission_open,
                      stop_epoch,change_seq,head_version,head_sha256,latest_event_id
                 FROM register_investigation_run_v1($1,$2,$3,$4,$5,$6,$7)"#,
        )
        .bind(input.identity.authority_id)
        .bind(input.stable_start_request_id)
        .bind(input.identity.operation_id)
        .bind(input.identity.stage_execution_id)
        .bind(&input.identity.owning_stage_run_request_id)
        .bind(input.identity.scope_snapshot_id)
        .bind(change_seq)
        .fetch_one(&*self.pool)
        .await?)
    }

    pub async fn load_run_head(
        &self,
        identity: &InvestigationStageIdentity,
    ) -> UnifiedInvestigationRuntimeStoreResult<Option<InvestigationRunHeadRow>> {
        validate_stage_identity(identity)?;
        Ok(sqlx::query_as::<_, InvestigationRunHeadRow>(
            r#"SELECT authority_id,stable_start_request_id,operation_id,stage_execution_id,
                      owning_stage_run_request_id,scope_snapshot_id,run_state,admission_open,
                      stop_epoch,change_seq,head_version,head_sha256,latest_event_id
                 FROM investigation_run_heads
                WHERE authority_id=$1 AND operation_id=$2 AND stage_execution_id=$3
                  AND owning_stage_run_request_id=$4 AND scope_snapshot_id=$5"#,
        )
        .bind(identity.authority_id)
        .bind(identity.operation_id)
        .bind(identity.stage_execution_id)
        .bind(&identity.owning_stage_run_request_id)
        .bind(identity.scope_snapshot_id)
        .fetch_optional(&*self.pool)
        .await?)
    }

    /// Resolve the one durable unified-Investigation run selected by the
    /// externally visible stage identity. The caller still has to authorize
    /// the operation and frozen scope before exposing this row. Returning an
    /// identity error for duplicate rows prevents an arbitrary "latest" pick
    /// if persisted authority is ever corrupted.
    pub async fn load_run_head_for_stage_selector(
        &self,
        operation_id: Uuid,
        stage_execution_id: Uuid,
        owning_stage_run_request_id: &str,
    ) -> UnifiedInvestigationRuntimeStoreResult<Option<InvestigationRunHeadRow>> {
        if operation_id.is_nil() || stage_execution_id.is_nil() {
            return Err(UnifiedInvestigationRuntimeStoreError::InvalidInput(
                "stage_selector_id",
            ));
        }
        let owning_stage_run_request_id = owning_stage_run_request_id.trim();
        if owning_stage_run_request_id.is_empty() || owning_stage_run_request_id.len() > 512 {
            return Err(UnifiedInvestigationRuntimeStoreError::InvalidInput(
                "owning_stage_run_request_id",
            ));
        }
        let mut rows = sqlx::query_as::<_, InvestigationRunHeadRow>(
            r#"SELECT authority_id,stable_start_request_id,operation_id,stage_execution_id,
                      owning_stage_run_request_id,scope_snapshot_id,run_state,admission_open,
                      stop_epoch,change_seq,head_version,head_sha256,latest_event_id
                 FROM investigation_run_heads
                WHERE operation_id=$1 AND stage_execution_id=$2
                  AND owning_stage_run_request_id=$3
                ORDER BY authority_id
                LIMIT 2"#,
        )
        .bind(operation_id)
        .bind(stage_execution_id)
        .bind(owning_stage_run_request_id)
        .fetch_all(&*self.pool)
        .await?;
        if rows.len() > 1 {
            return Err(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                "stage_selector_ambiguous",
            ));
        }
        Ok(rows.pop())
    }

    pub async fn register_work(
        &self,
        input: &RegisterInvestigationWorkInput,
    ) -> UnifiedInvestigationRuntimeStoreResult<InvestigationRunWorkRow> {
        validate_unit_identity(&input.identity)?;
        validate_ids(&[input.work_id, input.asset_lane_id])?;
        validate_hashes(&[
            &input.stable_work_key_sha256,
            &input.external_identity_sha256,
        ])?;
        let stop_epoch = to_i64(input.observed_stop_epoch, "observed_stop_epoch")?;
        sqlx::query(
            r#"INSERT INTO investigation_run_work_items(
                   work_id,asset_lane_id,stable_work_key_sha256,authority_id,operation_id,
                   stage_execution_id,owning_stage_run_request_id,stage_run_unit_id,
                   scope_snapshot_id,organization_id,work_kind,external_identity_sha256,
                   current_state,observed_stop_epoch
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
               ON CONFLICT(authority_id,stable_work_key_sha256) DO NOTHING"#,
        )
        .bind(input.work_id)
        .bind(input.asset_lane_id)
        .bind(&input.stable_work_key_sha256)
        .bind(input.identity.stage.authority_id)
        .bind(input.identity.stage.operation_id)
        .bind(input.identity.stage.stage_execution_id)
        .bind(&input.identity.stage.owning_stage_run_request_id)
        .bind(input.identity.stage_run_unit_id)
        .bind(input.identity.stage.scope_snapshot_id)
        .bind(input.identity.organization_id)
        .bind(input.work_kind.as_str())
        .bind(&input.external_identity_sha256)
        .bind(input.initial_state.as_str())
        .bind(stop_epoch)
        .execute(&*self.pool)
        .await?;
        let row = self
            .load_work_by_key(&input.identity, &input.stable_work_key_sha256)
            .await?
            .ok_or(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                "work_replay_missing",
            ))?;
        if row.work_id != input.work_id
            || row.asset_lane_id != input.asset_lane_id
            || row.work_kind != input.work_kind.as_str()
            || row.external_identity_sha256 != input.external_identity_sha256
            || row.observed_stop_epoch != stop_epoch
            || (row.head_version == 0 && row.current_state != input.initial_state.as_str())
        {
            return Err(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                "work_replay_mismatch",
            ));
        }
        Ok(row)
    }

    pub async fn ensure_dynamic_asset_analysis_work(
        &self,
        input: &EnsureDynamicAssetAnalysisWorkInput,
    ) -> UnifiedInvestigationRuntimeStoreResult<EnsuredDynamicAssetAnalysisWorkRow> {
        validate_unit_identity(&input.identity)?;
        validate_ids(&[
            input.stable_cutover_request_id,
            input.asset_lane_id,
            input.dynamic_work_id,
        ])?;
        validate_hashes(&[
            &input.legacy_stable_work_key_sha256,
            &input.dynamic_stable_work_key_sha256,
            &input.dynamic_external_identity_sha256,
        ])?;
        if input.legacy_stable_work_key_sha256 == input.dynamic_stable_work_key_sha256 {
            return Err(UnifiedInvestigationRuntimeStoreError::InvalidInput(
                "dynamic_analysis_work_key",
            ));
        }
        let stop_epoch = to_i64(input.observed_stop_epoch, "observed_stop_epoch")?;
        let returned_work_id: Uuid = sqlx::query_scalar(
            r#"SELECT ensure_investigation_dynamic_asset_analysis_work_v2(
                   $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"#,
        )
        .bind(input.stable_cutover_request_id)
        .bind(input.identity.stage.authority_id)
        .bind(input.identity.stage.operation_id)
        .bind(input.identity.stage.stage_execution_id)
        .bind(&input.identity.stage.owning_stage_run_request_id)
        .bind(input.identity.stage_run_unit_id)
        .bind(input.identity.stage.scope_snapshot_id)
        .bind(input.identity.organization_id)
        .bind(input.asset_lane_id)
        .bind(&input.legacy_stable_work_key_sha256)
        .bind(input.dynamic_work_id)
        .bind(&input.dynamic_stable_work_key_sha256)
        .bind(&input.dynamic_external_identity_sha256)
        .bind(stop_epoch)
        .fetch_one(&*self.pool)
        .await?;
        if returned_work_id != input.dynamic_work_id {
            return Err(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                "dynamic_analysis_work_replay_mismatch",
            ));
        }
        let work = self
            .load_work_by_key(&input.identity, &input.dynamic_stable_work_key_sha256)
            .await?
            .ok_or(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                "dynamic_analysis_work_missing",
            ))?;
        if work.work_id != input.dynamic_work_id
            || work.asset_lane_id != input.asset_lane_id
            || work.work_kind != InvestigationWorkKind::Analysis.as_str()
            || work.external_identity_sha256 != input.dynamic_external_identity_sha256
            || work.current_state != InvestigationWorkState::Running.as_str()
            || work.observed_stop_epoch != stop_epoch
            || work.head_version != 0
            || work.latest_event_id.is_some()
        {
            return Err(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                "dynamic_analysis_work_replay_mismatch",
            ));
        }
        let cutover_authority_id = sqlx::query_scalar::<_, Uuid>(
            r#"SELECT cutover_authority_id
                 FROM investigation_dynamic_analysis_work_cutovers
                WHERE stable_request_id=$1 AND dynamic_work_id=$2 AND status='applied'"#,
        )
        .bind(input.stable_cutover_request_id)
        .bind(input.dynamic_work_id)
        .fetch_optional(&*self.pool)
        .await?;
        Ok(EnsuredDynamicAssetAnalysisWorkRow {
            work,
            cutover_authority_id,
        })
    }

    pub async fn transition_work(
        &self,
        input: &TransitionInvestigationWorkInput,
    ) -> UnifiedInvestigationRuntimeStoreResult<InvestigationRunWorkRow> {
        validate_unit_identity(&input.identity)?;
        validate_ids(&[input.work_id, input.event_id, input.stable_request_id])?;
        validate_hashes(&[&input.event_sha256])?;
        validate_bounded(&input.reason_code, 512, "reason_code")?;
        let expected = to_i64(input.expected_head_version, "expected_head_version")?;
        let epoch = to_i64(input.observed_stop_epoch, "observed_stop_epoch")?;
        if let Some(existing) = self.load_work_event(input.stable_request_id).await? {
            validate_work_event_replay(&existing, input, expected, epoch)?;
            return self.load_work_exact(&input.identity, input.work_id).await;
        }
        let ordinal =
            expected
                .checked_add(1)
                .ok_or(UnifiedInvestigationRuntimeStoreError::InvalidInput(
                    "event_ordinal",
                ))?;
        let result = sqlx::query(
            r#"INSERT INTO investigation_run_work_state_events(
                   event_id,stable_request_id,work_id,expected_head_version,event_ordinal,
                   from_state,to_state,observed_stop_epoch,reason_code,event_sha256
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)"#,
        )
        .bind(input.event_id)
        .bind(input.stable_request_id)
        .bind(input.work_id)
        .bind(expected)
        .bind(ordinal)
        .bind(input.from_state.as_str())
        .bind(input.to_state.as_str())
        .bind(epoch)
        .bind(&input.reason_code)
        .bind(&input.event_sha256)
        .execute(&*self.pool)
        .await;
        if let Err(error) = result {
            if let Some(existing) = self.load_work_event(input.stable_request_id).await? {
                validate_work_event_replay(&existing, input, expected, epoch)?;
            } else {
                return Err(error.into());
            }
        }
        self.load_work_exact(&input.identity, input.work_id).await
    }

    pub async fn begin_pentagi_plan(
        &self,
        input: &BeginPentagiTaskPlanInput,
    ) -> UnifiedInvestigationRuntimeStoreResult<PentagiTaskPlanRow> {
        validate_unit_identity(&input.identity)?;
        validate_ids(&[
            input.task_plan_id,
            input.stable_request_id,
            input.run_request_id,
            input.stage_team_plan_id,
            input.subject_id,
        ])?;
        validate_hashes(&[
            &input.subject_fingerprint_sha256,
            &input.task_plan_sha256,
            &input.cognitive_tool_envelope_sha256,
        ])?;
        let version = i32::try_from(input.task_plan_version).map_err(|_| {
            UnifiedInvestigationRuntimeStoreError::InvalidInput("task_plan_version")
        })?;
        if version <= 0 || !json_string_array_is_valid(&input.allowed_role_catalog) {
            return Err(UnifiedInvestigationRuntimeStoreError::InvalidInput(
                "pentagi_plan",
            ));
        }
        sqlx::query(
            r#"INSERT INTO investigation_pentagi_task_plans(
                   task_plan_id,stable_request_id,run_request_id,authority_id,stage_team_plan_id,
                   operation_id,stage_execution_id,owning_stage_run_request_id,
                   stage_run_unit_id,scope_snapshot_id,organization_id,subject_kind,
                   subject_id,subject_fingerprint_sha256,task_plan_version,task_plan_sha256,
                   allowed_role_catalog,cognitive_tool_envelope_sha256
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)
               ON CONFLICT(stable_request_id) DO NOTHING"#,
        )
        .bind(input.task_plan_id)
        .bind(input.stable_request_id)
        .bind(input.run_request_id)
        .bind(input.identity.stage.authority_id)
        .bind(input.stage_team_plan_id)
        .bind(input.identity.stage.operation_id)
        .bind(input.identity.stage.stage_execution_id)
        .bind(&input.identity.stage.owning_stage_run_request_id)
        .bind(input.identity.stage_run_unit_id)
        .bind(input.identity.stage.scope_snapshot_id)
        .bind(input.identity.organization_id)
        .bind(input.subject_kind.as_str())
        .bind(input.subject_id)
        .bind(&input.subject_fingerprint_sha256)
        .bind(version)
        .bind(&input.task_plan_sha256)
        .bind(&input.allowed_role_catalog)
        .bind(&input.cognitive_tool_envelope_sha256)
        .execute(&*self.pool)
        .await?;
        let row = self
            .load_plan_by_request(&input.identity, input.stable_request_id)
            .await?
            .ok_or(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                "pentagi_plan_replay_missing",
            ))?;
        if row.task_plan_id != input.task_plan_id
            || row.run_request_id != input.run_request_id
            || row.stage_team_plan_id != input.stage_team_plan_id
            || row.subject_kind != input.subject_kind.as_str()
            || row.subject_id != input.subject_id
            || row.subject_fingerprint_sha256 != input.subject_fingerprint_sha256
            || row.task_plan_version != version
            || row.task_plan_sha256 != input.task_plan_sha256
            || row.allowed_role_catalog != input.allowed_role_catalog
            || row.cognitive_tool_envelope_sha256 != input.cognitive_tool_envelope_sha256
        {
            return Err(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                "pentagi_plan_replay_mismatch",
            ));
        }
        Ok(row)
    }

    pub async fn insert_pentagi_subtask(
        &self,
        input: &InsertPentagiSubtaskInput,
    ) -> UnifiedInvestigationRuntimeStoreResult<PentagiSubtaskRow> {
        validate_unit_identity(&input.identity)?;
        validate_ids(&[input.task_plan_id, input.subtask_id])?;
        validate_hashes(&[&input.input_manifest_sha256, &input.member_sha256])?;
        validate_bounded(&input.label, 512, "subtask_label")?;
        validate_bounded(&input.expected_output_schema, 512, "expected_output_schema")?;
        let ordinal = i32::try_from(input.subtask_ordinal)
            .map_err(|_| UnifiedInvestigationRuntimeStoreError::InvalidInput("subtask_ordinal"))?;
        sqlx::query(
            r#"INSERT INTO investigation_pentagi_subtasks(
                   subtask_id,task_plan_id,authority_id,operation_id,stage_execution_id,
                   stage_run_unit_id,organization_id,subtask_ordinal,label,runnable,
                   input_manifest_sha256,expected_output_schema,member_sha256
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
               ON CONFLICT(subtask_id) DO NOTHING"#,
        )
        .bind(input.subtask_id)
        .bind(input.task_plan_id)
        .bind(input.identity.stage.authority_id)
        .bind(input.identity.stage.operation_id)
        .bind(input.identity.stage.stage_execution_id)
        .bind(input.identity.stage_run_unit_id)
        .bind(input.identity.organization_id)
        .bind(ordinal)
        .bind(&input.label)
        .bind(input.runnable)
        .bind(&input.input_manifest_sha256)
        .bind(&input.expected_output_schema)
        .bind(&input.member_sha256)
        .execute(&*self.pool)
        .await?;
        let row = self
            .load_subtask_exact(&input.identity, input.task_plan_id, input.subtask_id)
            .await?;
        if row.subtask_ordinal != ordinal
            || row.label != input.label
            || row.runnable != input.runnable
            || row.input_manifest_sha256 != input.input_manifest_sha256
            || row.expected_output_schema != input.expected_output_schema
            || row.member_sha256 != input.member_sha256
        {
            return Err(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                "pentagi_subtask_replay_mismatch",
            ));
        }
        Ok(row)
    }

    pub async fn insert_pentagi_run_request(
        &self,
        input: &InsertPentagiTaskRunRequestInput,
    ) -> UnifiedInvestigationRuntimeStoreResult<PentagiTaskRunRequestRow> {
        validate_unit_identity(&input.identity)?;
        validate_ids(&[
            input.run_request_id,
            input.stable_request_id,
            input.subject_id,
        ])?;
        validate_hashes(&[&input.subject_fingerprint_sha256, &input.request_sha256])?;
        sqlx::query(
            r#"INSERT INTO pentagi_task_run_requests(
                   run_request_id,stable_request_id,task_plan_id,authority_id,operation_id,
                   stage_execution_id,owning_stage_run_request_id,stage_run_unit_id,
                   organization_id,subject_kind,subject_id,subject_fingerprint_sha256,
                   request_sha256
               ) VALUES($1,$2,NULL,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
               ON CONFLICT(stable_request_id) DO NOTHING"#,
        )
        .bind(input.run_request_id)
        .bind(input.stable_request_id)
        .bind(input.identity.stage.authority_id)
        .bind(input.identity.stage.operation_id)
        .bind(input.identity.stage.stage_execution_id)
        .bind(&input.identity.stage.owning_stage_run_request_id)
        .bind(input.identity.stage_run_unit_id)
        .bind(input.identity.organization_id)
        .bind(input.subject_kind.as_str())
        .bind(input.subject_id)
        .bind(&input.subject_fingerprint_sha256)
        .bind(&input.request_sha256)
        .execute(&*self.pool)
        .await?;
        let row = self
            .load_run_request_exact(&input.identity, input.stable_request_id)
            .await?;
        if row.run_request_id != input.run_request_id
            || row.task_plan_id.is_some()
            || row.subject_kind != input.subject_kind.as_str()
            || row.subject_id != input.subject_id
            || row.subject_fingerprint_sha256 != input.subject_fingerprint_sha256
            || row.request_sha256 != input.request_sha256
        {
            return Err(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                "pentagi_run_request_replay_mismatch",
            ));
        }
        Ok(row)
    }

    pub async fn insert_logical_dispatch(
        &self,
        input: &InsertPentagiLogicalDispatchInput,
    ) -> UnifiedInvestigationRuntimeStoreResult<PentagiLogicalDispatchRow> {
        validate_unit_identity(&input.identity)?;
        validate_ids(&[
            input.dispatch_receipt_id,
            input.stable_request_id,
            input.task_plan_id,
            input.stage_work_item_id,
            input.worker_run_id,
        ])?;
        validate_optional_id(input.subtask_id, "subtask_id")?;
        validate_optional_id(
            input.parent_dispatch_receipt_id,
            "parent_dispatch_receipt_id",
        )?;
        validate_optional_id(input.stage_worker_request_id, "stage_worker_request_id")?;
        validate_hashes(&[
            &input.logical_dispatch_key_sha256,
            &input.snapshot_sha256,
            &input.receipt_sha256,
        ])?;
        validate_bounded(&input.transcript_request_id, 512, "transcript_request_id")?;
        validate_optional_bounded(
            input.parent_actor_transcript_request_id.as_deref(),
            512,
            "parent_actor_transcript_request_id",
        )?;
        validate_optional_bounded(
            input.parent_dispatch_tool_request_id.as_deref(),
            512,
            "parent_dispatch_tool_request_id",
        )?;
        let ordinal = i32::try_from(input.dispatch_ordinal)
            .map_err(|_| UnifiedInvestigationRuntimeStoreError::InvalidInput("dispatch_ordinal"))?;
        sqlx::query(
            r#"INSERT INTO pentagi_logical_dispatch_receipts(
                   dispatch_receipt_id,stable_request_id,logical_dispatch_key_sha256,
                   task_plan_id,subtask_id,parent_dispatch_receipt_id,dispatch_ordinal,
                   actor_kind,stage_work_item_id,stage_worker_request_id,worker_run_id,
                   operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
                   organization_id,transcript_request_id,parent_actor_transcript_request_id,
                   parent_dispatch_tool_request_id,snapshot_sha256,receipt_sha256
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,
                        $17,$18,$19,$20,$21)
               ON CONFLICT(stable_request_id) DO NOTHING"#,
        )
        .bind(input.dispatch_receipt_id)
        .bind(input.stable_request_id)
        .bind(&input.logical_dispatch_key_sha256)
        .bind(input.task_plan_id)
        .bind(input.subtask_id)
        .bind(input.parent_dispatch_receipt_id)
        .bind(ordinal)
        .bind(input.actor_kind.as_str())
        .bind(input.stage_work_item_id)
        .bind(input.stage_worker_request_id)
        .bind(input.worker_run_id)
        .bind(input.identity.stage.operation_id)
        .bind(input.identity.stage.stage_execution_id)
        .bind(input.identity.stage_run_unit_id)
        .bind(input.identity.stage.scope_snapshot_id)
        .bind(input.identity.organization_id)
        .bind(&input.transcript_request_id)
        .bind(&input.parent_actor_transcript_request_id)
        .bind(&input.parent_dispatch_tool_request_id)
        .bind(&input.snapshot_sha256)
        .bind(&input.receipt_sha256)
        .execute(&*self.pool)
        .await?;
        let row = self
            .load_dispatch_exact(&input.identity, input.stable_request_id)
            .await?;
        if row.dispatch_receipt_id != input.dispatch_receipt_id
            || row.logical_dispatch_key_sha256 != input.logical_dispatch_key_sha256
            || row.task_plan_id != input.task_plan_id
            || row.subtask_id != input.subtask_id
            || row.parent_dispatch_receipt_id != input.parent_dispatch_receipt_id
            || row.dispatch_ordinal != ordinal
            || row.actor_kind != input.actor_kind.as_str()
            || row.stage_work_item_id != input.stage_work_item_id
            || row.stage_worker_request_id != input.stage_worker_request_id
            || row.worker_run_id != input.worker_run_id
            || row.transcript_request_id != input.transcript_request_id
            || row.parent_actor_transcript_request_id != input.parent_actor_transcript_request_id
            || row.parent_dispatch_tool_request_id != input.parent_dispatch_tool_request_id
            || row.snapshot_sha256 != input.snapshot_sha256
            || row.receipt_sha256 != input.receipt_sha256
        {
            return Err(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                "pentagi_dispatch_replay_mismatch",
            ));
        }
        Ok(row)
    }

    pub async fn insert_dispatch_attempt(
        &self,
        input: &InsertPentagiDispatchAttemptInput,
    ) -> UnifiedInvestigationRuntimeStoreResult<PentagiDispatchAttemptRow> {
        validate_unit_identity(&input.identity)?;
        validate_ids(&[
            input.task_plan_id,
            input.dispatch_attempt_id,
            input.stable_request_id,
            input.dispatch_receipt_id,
            input.lease_token,
        ])?;
        validate_hashes(&[&input.fence_sha256, &input.result_sha256])?;
        let epoch = to_i64(input.attempt_epoch, "attempt_epoch")?;
        let dispatch = self
            .load_dispatch_by_id_exact(
                &input.identity,
                input.task_plan_id,
                input.dispatch_receipt_id,
            )
            .await?;
        if dispatch.task_plan_id != input.task_plan_id {
            return Err(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                "dispatch_attempt_owner_mismatch",
            ));
        }
        sqlx::query(
            r#"INSERT INTO pentagi_logical_dispatch_attempts(
                   dispatch_attempt_id,stable_request_id,dispatch_receipt_id,attempt_epoch,
                   lease_token,fence_sha256,outcome,result_sha256
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8)
               ON CONFLICT(stable_request_id) DO NOTHING"#,
        )
        .bind(input.dispatch_attempt_id)
        .bind(input.stable_request_id)
        .bind(input.dispatch_receipt_id)
        .bind(epoch)
        .bind(input.lease_token)
        .bind(&input.fence_sha256)
        .bind(input.outcome.as_str())
        .bind(&input.result_sha256)
        .execute(&*self.pool)
        .await?;
        let row = sqlx::query_as::<_, PentagiDispatchAttemptRow>(
            r#"SELECT attempt.dispatch_attempt_id,attempt.stable_request_id,
                      attempt.dispatch_receipt_id,attempt.attempt_epoch,attempt.lease_token,
                      attempt.fence_sha256,attempt.outcome,attempt.result_sha256
                 FROM pentagi_logical_dispatch_attempts attempt
                 JOIN pentagi_logical_dispatch_receipts dispatch
                   ON dispatch.dispatch_receipt_id=attempt.dispatch_receipt_id
                WHERE attempt.stable_request_id=$1 AND dispatch.task_plan_id=$2
                  AND dispatch.operation_id=$3 AND dispatch.stage_execution_id=$4
                  AND dispatch.stage_run_unit_id=$5 AND dispatch.organization_id=$6"#,
        )
        .bind(input.stable_request_id)
        .bind(input.task_plan_id)
        .bind(input.identity.stage.operation_id)
        .bind(input.identity.stage.stage_execution_id)
        .bind(input.identity.stage_run_unit_id)
        .bind(input.identity.organization_id)
        .fetch_one(&*self.pool)
        .await?;
        if row.dispatch_attempt_id != input.dispatch_attempt_id
            || row.dispatch_receipt_id != input.dispatch_receipt_id
            || row.attempt_epoch != epoch
            || row.lease_token != input.lease_token
            || row.fence_sha256 != input.fence_sha256
            || row.outcome != input.outcome.as_str()
            || row.result_sha256 != input.result_sha256
        {
            return Err(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                "dispatch_attempt_replay_mismatch",
            ));
        }
        Ok(row)
    }

    pub async fn insert_pipeline_event(
        &self,
        input: &InsertPentagiPipelineEventInput,
    ) -> UnifiedInvestigationRuntimeStoreResult<PentagiPipelineEventRow> {
        validate_unit_identity(&input.identity)?;
        validate_ids(&[
            input.pipeline_event_id,
            input.stable_request_id,
            input.task_plan_id,
            input.actor_worker_run_id,
            input.parent_dispatch_receipt_id,
        ])?;
        validate_optional_id(input.subtask_id, "subtask_id")?;
        validate_hashes(&[&input.event_sha256])?;
        let ordinal = to_i64(input.event_ordinal, "pipeline_event_ordinal")?;
        self.load_dispatch_by_id_exact(
            &input.identity,
            input.task_plan_id,
            input.parent_dispatch_receipt_id,
        )
        .await?;
        sqlx::query(
            r#"INSERT INTO investigation_pentagi_pipeline_events(
                   pipeline_event_id,stable_request_id,task_plan_id,subtask_id,
                   event_ordinal,event_kind,actor_worker_run_id,
                   parent_dispatch_receipt_id,event_sha256
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)
               ON CONFLICT(stable_request_id) DO NOTHING"#,
        )
        .bind(input.pipeline_event_id)
        .bind(input.stable_request_id)
        .bind(input.task_plan_id)
        .bind(input.subtask_id)
        .bind(ordinal)
        .bind(input.event_kind.as_str())
        .bind(input.actor_worker_run_id)
        .bind(input.parent_dispatch_receipt_id)
        .bind(&input.event_sha256)
        .execute(&*self.pool)
        .await?;
        let row = sqlx::query_as::<_, PentagiPipelineEventRow>(
            r#"SELECT event.pipeline_event_id,event.stable_request_id,event.task_plan_id,
                      event.subtask_id,event.event_ordinal,event.event_kind,
                      event.actor_worker_run_id,event.parent_dispatch_receipt_id,event.event_sha256
                 FROM investigation_pentagi_pipeline_events event
                 JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=event.task_plan_id
                WHERE event.stable_request_id=$1 AND plan.authority_id=$2
                  AND plan.operation_id=$3 AND plan.stage_execution_id=$4
                  AND plan.stage_run_unit_id=$5 AND plan.organization_id=$6"#,
        )
        .bind(input.stable_request_id)
        .bind(input.identity.stage.authority_id)
        .bind(input.identity.stage.operation_id)
        .bind(input.identity.stage.stage_execution_id)
        .bind(input.identity.stage_run_unit_id)
        .bind(input.identity.organization_id)
        .fetch_one(&*self.pool)
        .await?;
        if row.pipeline_event_id != input.pipeline_event_id
            || row.task_plan_id != input.task_plan_id
            || row.subtask_id != input.subtask_id
            || row.event_ordinal != ordinal
            || row.event_kind != input.event_kind.as_str()
            || row.actor_worker_run_id != input.actor_worker_run_id
            || row.parent_dispatch_receipt_id != input.parent_dispatch_receipt_id
            || row.event_sha256 != input.event_sha256
        {
            return Err(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                "pipeline_event_replay_mismatch",
            ));
        }
        Ok(row)
    }

    pub async fn create_refiner_plan_ledger(
        &self,
        input: &CreateInvestigationRefinerPlanLedgerInput,
    ) -> UnifiedInvestigationRuntimeStoreResult<InvestigationRefinerPlanLedgerRow> {
        validate_unit_identity(&input.identity)?;
        validate_ids(&[
            input.ledger_id,
            input.stable_request_id,
            input.task_plan_id,
            input.generator_pipeline_event_id,
        ])?;
        if input
            .generator_manifest
            .as_object()
            .is_none_or(|manifest| manifest.is_empty())
        {
            return Err(UnifiedInvestigationRuntimeStoreError::InvalidInput(
                "generator_manifest",
            ));
        }
        self.load_plan_exact(&input.identity, input.task_plan_id)
            .await?;
        Ok(sqlx::query_as::<_, InvestigationRefinerPlanLedgerRow>(
            r#"SELECT ledger_id,stable_request_id,task_plan_id,generator_pipeline_event_id,
                      generator_manifest,generator_manifest_sha256,generator_subtask_count,
                      generator_subtask_set_sha256,ledger_sha256
                 FROM create_investigation_refiner_plan_ledger_v2($1,$2,$3,$4,$5)"#,
        )
        .bind(input.ledger_id)
        .bind(input.stable_request_id)
        .bind(input.task_plan_id)
        .bind(input.generator_pipeline_event_id)
        .bind(&input.generator_manifest)
        .fetch_one(&*self.pool)
        .await?)
    }

    pub async fn load_pending_generator_recovery(
        &self,
        identity: &InvestigationUnitIdentity,
        task_plan_id: Uuid,
    ) -> UnifiedInvestigationRuntimeStoreResult<Option<PendingInvestigationGeneratorRecoveryRow>>
    {
        validate_unit_identity(identity)?;
        validate_ids(&[task_plan_id])?;
        let mut tx = self.pool.begin().await?;
        let plan_exists: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1 FROM investigation_pentagi_task_plans plan
                    WHERE plan.task_plan_id=$1 AND plan.authority_id=$2
                      AND plan.operation_id=$3 AND plan.stage_execution_id=$4
                      AND plan.owning_stage_run_request_id=$5
                      AND plan.stage_run_unit_id=$6 AND plan.scope_snapshot_id=$7
                      AND plan.organization_id=$8 AND plan.status='open')"#,
        )
        .bind(task_plan_id)
        .bind(identity.stage.authority_id)
        .bind(identity.stage.operation_id)
        .bind(identity.stage.stage_execution_id)
        .bind(&identity.stage.owning_stage_run_request_id)
        .bind(identity.stage_run_unit_id)
        .bind(identity.stage.scope_snapshot_id)
        .bind(identity.organization_id)
        .fetch_one(&mut *tx)
        .await?;
        if !plan_exists {
            tx.commit().await?;
            return Ok(None);
        }
        let dispatch: Option<(Uuid, Uuid, Uuid)> = sqlx::query_as(
            r#"SELECT dispatch.dispatch_receipt_id,dispatch.stage_work_item_id,
                      dispatch.worker_run_id
                 FROM pentagi_logical_dispatch_receipts dispatch
                WHERE dispatch.task_plan_id=$1 AND dispatch.actor_kind='primary'
                  AND investigation_refiner_primary_source_is_current_v3(
                      $1,dispatch.stage_work_item_id,dispatch.worker_run_id)
                ORDER BY dispatch.dispatch_ordinal DESC,dispatch.dispatch_receipt_id
                LIMIT 1"#,
        )
        .bind(task_plan_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((primary_dispatch_receipt_id, primary_work_item_id, primary_worker_run_id)) =
            dispatch
        else {
            tx.commit().await?;
            return Ok(None);
        };
        let existing_subtasks = load_generator_subtasks_on(&mut tx, identity, task_plan_id).await?;
        let candidates =
            load_generator_candidates_on(&mut tx, identity, task_plan_id, None).await?;
        tx.commit().await?;
        Ok(Some(PendingInvestigationGeneratorRecoveryRow {
            task_plan_id,
            primary_dispatch_receipt_id,
            primary_work_item_id,
            primary_worker_run_id,
            existing_subtasks,
            candidates,
        }))
    }

    pub async fn materialize_generator(
        &self,
        input: &MaterializeInvestigationGeneratorInput,
    ) -> UnifiedInvestigationRuntimeStoreResult<InvestigationGeneratorMaterializationRow> {
        validate_generator_materialization_input(input)?;
        let mut tx = self.pool.begin().await?;
        lock_generator_consumer_on(
            &mut tx,
            &input.identity,
            input.task_plan_id,
            &input.consumer_fence,
        )
        .await?;
        let source = load_generator_source_on(
            &mut tx,
            &input.identity,
            input.task_plan_id,
            input.source_tool_call_id,
        )
        .await?;
        let replayed = load_generator_source_receipt_on(&mut tx, input.stable_request_id)
            .await?
            .is_some();
        if !replayed
            && sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM investigation_refiner_plan_ledgers WHERE task_plan_id=$1)",
            )
            .bind(input.task_plan_id)
            .fetch_one(&mut *tx)
            .await?
        {
            return Err(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                "generator_materialization_requires_adoption",
            ));
        }
        for member in &input.subtasks {
            insert_generator_subtask_on(&mut tx, &input.identity, input.task_plan_id, member)
                .await?;
        }
        let subtasks =
            load_generator_subtasks_on(&mut tx, &input.identity, input.task_plan_id).await?;
        ensure_generator_subtask_request_exact(&subtasks, &input.subtasks)?;
        let ledger = create_generator_ledger_on(
            &mut tx,
            input.ledger_id,
            input.stable_request_id,
            input.task_plan_id,
            input.generator_pipeline_event_id,
            &source.canonical_result,
        )
        .await?;
        insert_generator_source_receipt_on(
            &mut tx,
            input.source_receipt_id,
            input.stable_request_id,
            &input.identity,
            &ledger,
            &source,
            &input.consumer_fence,
            "materialized",
        )
        .await?;
        tx.commit().await?;
        Ok(InvestigationGeneratorMaterializationRow {
            ledger,
            subtasks,
            source,
            source_receipt_id: input.source_receipt_id,
            adoption_receipt_id: None,
            replayed,
        })
    }

    pub async fn adopt_orphan_generator(
        &self,
        input: &AdoptInvestigationOrphanGeneratorInput,
    ) -> UnifiedInvestigationRuntimeStoreResult<InvestigationGeneratorMaterializationRow> {
        validate_unit_identity(&input.identity)?;
        validate_ids(&[
            input.task_plan_id,
            input.adoption_receipt_id,
            input.stable_request_id,
            input.ledger_id,
            input.ledger_stable_request_id,
            input.generator_pipeline_event_id,
            input.source_tool_call_id,
        ])?;
        validate_ids(&input.expected_existing_subtask_ids)?;
        if input.expected_existing_subtask_ids.is_empty() {
            return Err(UnifiedInvestigationRuntimeStoreError::InvalidInput(
                "orphan_generator_subtasks_empty",
            ));
        }
        validate_generator_consumer_fence(&input.consumer_fence)?;
        let mut tx = self.pool.begin().await?;
        lock_generator_consumer_on(
            &mut tx,
            &input.identity,
            input.task_plan_id,
            &input.consumer_fence,
        )
        .await?;
        let source = load_generator_source_on(
            &mut tx,
            &input.identity,
            input.task_plan_id,
            input.source_tool_call_id,
        )
        .await?;
        let replayed = load_generator_source_receipt_on(&mut tx, input.stable_request_id)
            .await?
            .is_some();
        let subtasks =
            load_generator_subtasks_on(&mut tx, &input.identity, input.task_plan_id).await?;
        let actual_ids = subtasks
            .iter()
            .map(|row| row.subtask_id)
            .collect::<Vec<_>>();
        if actual_ids != input.expected_existing_subtask_ids {
            return Err(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                "orphan_generator_subtask_census_mismatch",
            ));
        }
        let ledger = create_generator_ledger_on(
            &mut tx,
            input.ledger_id,
            input.ledger_stable_request_id,
            input.task_plan_id,
            input.generator_pipeline_event_id,
            &source.canonical_result,
        )
        .await?;
        insert_generator_source_receipt_on(
            &mut tx,
            input.adoption_receipt_id,
            input.stable_request_id,
            &input.identity,
            &ledger,
            &source,
            &input.consumer_fence,
            "orphan_adoption",
        )
        .await?;
        tx.commit().await?;
        Ok(InvestigationGeneratorMaterializationRow {
            ledger,
            subtasks,
            source,
            source_receipt_id: input.adoption_receipt_id,
            adoption_receipt_id: Some(input.adoption_receipt_id),
            replayed,
        })
    }

    pub async fn load_refiner_plan_ledger(
        &self,
        identity: &InvestigationUnitIdentity,
        task_plan_id: Uuid,
    ) -> UnifiedInvestigationRuntimeStoreResult<Option<InvestigationRefinerPlanLedgerRow>> {
        validate_unit_identity(identity)?;
        validate_ids(&[task_plan_id])?;
        self.load_plan_exact(identity, task_plan_id).await?;
        Ok(sqlx::query_as::<_, InvestigationRefinerPlanLedgerRow>(
            r#"SELECT ledger_id,stable_request_id,task_plan_id,generator_pipeline_event_id,
                      generator_manifest,generator_manifest_sha256,generator_subtask_count,
                      generator_subtask_set_sha256,ledger_sha256
                 FROM investigation_refiner_plan_ledgers
                WHERE task_plan_id=$1"#,
        )
        .bind(task_plan_id)
        .fetch_optional(&*self.pool)
        .await?)
    }

    pub async fn load_logical_dispatch(
        &self,
        identity: &InvestigationUnitIdentity,
        task_plan_id: Uuid,
        dispatch_receipt_id: Uuid,
    ) -> UnifiedInvestigationRuntimeStoreResult<Option<PentagiLogicalDispatchRow>> {
        validate_unit_identity(identity)?;
        validate_ids(&[task_plan_id, dispatch_receipt_id])?;
        self.load_plan_exact(identity, task_plan_id).await?;
        Ok(
            sqlx::query_as::<_, PentagiLogicalDispatchRow>(DISPATCH_ROW_SELECT_BY_ID)
                .bind(dispatch_receipt_id)
                .bind(task_plan_id)
                .bind(identity.stage.operation_id)
                .bind(identity.stage.stage_execution_id)
                .bind(identity.stage_run_unit_id)
                .bind(identity.stage.scope_snapshot_id)
                .bind(identity.organization_id)
                .bind(identity.stage.authority_id)
                .bind(&identity.stage.owning_stage_run_request_id)
                .fetch_optional(&*self.pool)
                .await?,
        )
    }

    pub async fn load_latest_refiner_plan_patch(
        &self,
        identity: &InvestigationUnitIdentity,
        task_plan_id: Uuid,
    ) -> UnifiedInvestigationRuntimeStoreResult<Option<InvestigationRefinerPlanPatchRow>> {
        validate_unit_identity(identity)?;
        validate_ids(&[task_plan_id])?;
        self.load_plan_exact(identity, task_plan_id).await?;
        let patch = sqlx::query_as::<_, InvestigationRefinerPlanPatchRow>(
            r#"SELECT patch.patch_id,patch.stable_request_id,patch.ledger_id,
                      patch.task_plan_id,patch.patch_ordinal,patch.refiner_pipeline_event_id,
                      patch.expected_previous_state_sha256,patch.remaining_plan_payload,
                      patch.remaining_plan_payload_sha256,patch.active_realized_subtask_count,
                      patch.active_realized_subtask_set_sha256,patch.patch_sha256
                 FROM investigation_refiner_plan_patches patch
                 JOIN investigation_refiner_plan_ledgers ledger
                   ON ledger.ledger_id=patch.ledger_id
                  AND ledger.task_plan_id=patch.task_plan_id
                WHERE patch.task_plan_id=$1
                ORDER BY patch.patch_ordinal DESC
                LIMIT 1"#,
        )
        .bind(task_plan_id)
        .fetch_optional(&*self.pool)
        .await?;
        if let Some(patch) = &patch {
            let payload_hash_exact: bool = sqlx::query_scalar(
                "SELECT $1=investigation_refiner_payload_hash_v1('remaining_plan_patch',$2)",
            )
            .bind(&patch.remaining_plan_payload_sha256)
            .bind(&patch.remaining_plan_payload)
            .fetch_one(&*self.pool)
            .await?;
            if !payload_hash_exact {
                return Err(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                    "investigation_refiner_plan_patch_payload_hash",
                ));
            }
        }
        Ok(patch)
    }

    pub async fn append_refiner_plan_patch(
        &self,
        input: &AppendInvestigationRefinerPlanPatchInput,
    ) -> UnifiedInvestigationRuntimeStoreResult<InvestigationRefinerPlanPatchRow> {
        validate_unit_identity(&input.identity)?;
        validate_ids(&[
            input.patch_id,
            input.stable_request_id,
            input.ledger_id,
            input.task_plan_id,
            input.refiner_pipeline_event_id,
        ])?;
        validate_ids(&input.active_realized_subtask_ids)?;
        validate_hashes(&[&input.expected_previous_state_sha256])?;
        if input.remaining_plan_payload.as_object().is_none() {
            return Err(UnifiedInvestigationRuntimeStoreError::InvalidInput(
                "remaining_plan_payload",
            ));
        }
        self.load_plan_exact(&input.identity, input.task_plan_id)
            .await?;
        Ok(sqlx::query_as::<_, InvestigationRefinerPlanPatchRow>(
            r#"SELECT patch_id,stable_request_id,ledger_id,task_plan_id,patch_ordinal,
                      refiner_pipeline_event_id,expected_previous_state_sha256,
                      remaining_plan_payload,remaining_plan_payload_sha256,
                      active_realized_subtask_count,active_realized_subtask_set_sha256,
                      patch_sha256
                 FROM append_investigation_refiner_plan_patch_v1($1,$2,$3,$4,$5,$6,$7,$8)"#,
        )
        .bind(input.patch_id)
        .bind(input.stable_request_id)
        .bind(input.ledger_id)
        .bind(input.task_plan_id)
        .bind(input.refiner_pipeline_event_id)
        .bind(&input.expected_previous_state_sha256)
        .bind(&input.remaining_plan_payload)
        .bind(&input.active_realized_subtask_ids)
        .fetch_one(&*self.pool)
        .await?)
    }

    pub async fn append_dynamic_refiner_plan_patch(
        &self,
        input: &AppendInvestigationDynamicRefinerPlanPatchInput,
    ) -> UnifiedInvestigationRuntimeStoreResult<InvestigationRefinerPlanPatchRow> {
        validate_unit_identity(&input.identity)?;
        validate_ids(&[
            input.patch_id,
            input.stable_request_id,
            input.ledger_id,
            input.task_plan_id,
            input.refiner_pipeline_event_id,
        ])?;
        validate_ids(&input.ordered_active_subtask_ids)?;
        validate_hashes(&[&input.expected_previous_state_sha256])?;
        if input.remaining_plan_payload.as_object().is_none() {
            return Err(UnifiedInvestigationRuntimeStoreError::InvalidInput(
                "remaining_plan_payload",
            ));
        }
        self.load_plan_exact(&input.identity, input.task_plan_id)
            .await?;
        Ok(sqlx::query_as::<_, InvestigationRefinerPlanPatchRow>(
            r#"SELECT patch_id,stable_request_id,ledger_id,task_plan_id,patch_ordinal,
                      refiner_pipeline_event_id,expected_previous_state_sha256,
                      remaining_plan_payload,remaining_plan_payload_sha256,
                      active_realized_subtask_count,active_realized_subtask_set_sha256,
                      patch_sha256
                 FROM append_investigation_refiner_plan_patch_v2($1,$2,$3,$4,$5,$6,$7,$8)"#,
        )
        .bind(input.patch_id)
        .bind(input.stable_request_id)
        .bind(input.ledger_id)
        .bind(input.task_plan_id)
        .bind(input.refiner_pipeline_event_id)
        .bind(&input.expected_previous_state_sha256)
        .bind(&input.remaining_plan_payload)
        .bind(&input.ordered_active_subtask_ids)
        .fetch_one(&*self.pool)
        .await?)
    }

    pub async fn seal_refiner_plan_ledger(
        &self,
        input: &SealInvestigationRefinerPlanLedgerInput,
    ) -> UnifiedInvestigationRuntimeStoreResult<InvestigationRefinerPlanLedgerSealRow> {
        validate_unit_identity(&input.identity)?;
        validate_ids(&[
            input.seal_id,
            input.stable_request_id,
            input.ledger_id,
            input.task_plan_id,
            input.result_barrier_pipeline_event_id,
        ])?;
        validate_hashes(&[&input.expected_final_patch_sha256])?;
        self.load_plan_exact(&input.identity, input.task_plan_id)
            .await?;
        Ok(sqlx::query_as::<_, InvestigationRefinerPlanLedgerSealRow>(
            r#"SELECT seal_id,stable_request_id,ledger_id,task_plan_id,
                      result_barrier_pipeline_event_id,patch_count,patch_set_sha256,
                      final_patch_id,final_patch_sha256,
                      final_active_realized_subtask_count,
                      final_active_realized_subtask_set_sha256,generator_subtask_count,
                      generator_subtask_set_sha256,seal_sha256
                 FROM seal_investigation_refiner_plan_ledger_v1($1,$2,$3,$4,$5,$6)"#,
        )
        .bind(input.seal_id)
        .bind(input.stable_request_id)
        .bind(input.ledger_id)
        .bind(input.task_plan_id)
        .bind(input.result_barrier_pipeline_event_id)
        .bind(&input.expected_final_patch_sha256)
        .fetch_one(&*self.pool)
        .await?)
    }

    pub async fn seal_dynamic_refiner_plan_ledger(
        &self,
        input: &SealInvestigationDynamicRefinerPlanLedgerInput,
    ) -> UnifiedInvestigationRuntimeStoreResult<InvestigationRefinerPlanLedgerSealRow> {
        validate_unit_identity(&input.identity)?;
        validate_ids(&[
            input.seal_id,
            input.stable_request_id,
            input.ledger_id,
            input.task_plan_id,
            input.result_barrier_pipeline_event_id,
        ])?;
        validate_hashes(&[&input.expected_final_patch_sha256])?;
        self.load_plan_exact(&input.identity, input.task_plan_id)
            .await?;
        Ok(sqlx::query_as::<_, InvestigationRefinerPlanLedgerSealRow>(
            r#"SELECT seal_id,stable_request_id,ledger_id,task_plan_id,
                      result_barrier_pipeline_event_id,patch_count,patch_set_sha256,
                      final_patch_id,final_patch_sha256,
                      final_active_realized_subtask_count,
                      final_active_realized_subtask_set_sha256,generator_subtask_count,
                      generator_subtask_set_sha256,seal_sha256
                 FROM seal_investigation_refiner_plan_ledger_v2($1,$2,$3,$4,$5,$6)"#,
        )
        .bind(input.seal_id)
        .bind(input.stable_request_id)
        .bind(input.ledger_id)
        .bind(input.task_plan_id)
        .bind(input.result_barrier_pipeline_event_id)
        .bind(&input.expected_final_patch_sha256)
        .fetch_one(&*self.pool)
        .await?)
    }

    pub async fn load_refiner_plan_ledger_seal(
        &self,
        identity: &InvestigationUnitIdentity,
        task_plan_id: Uuid,
    ) -> UnifiedInvestigationRuntimeStoreResult<Option<InvestigationRefinerPlanLedgerSealRow>> {
        validate_unit_identity(identity)?;
        validate_ids(&[task_plan_id])?;
        self.load_plan_exact(identity, task_plan_id).await?;
        Ok(sqlx::query_as::<_, InvestigationRefinerPlanLedgerSealRow>(
            r#"SELECT seal_id,stable_request_id,ledger_id,task_plan_id,
                      result_barrier_pipeline_event_id,patch_count,patch_set_sha256,
                      final_patch_id,final_patch_sha256,
                      final_active_realized_subtask_count,
                      final_active_realized_subtask_set_sha256,generator_subtask_count,
                      generator_subtask_set_sha256,seal_sha256
                 FROM investigation_refiner_plan_ledger_seals
                WHERE task_plan_id=$1"#,
        )
        .bind(task_plan_id)
        .fetch_optional(&*self.pool)
        .await?)
    }

    pub async fn seal_delegation_census(
        &self,
        input: &SealPentagiDelegationCensusInput,
    ) -> UnifiedInvestigationRuntimeStoreResult<PentagiDelegationCensusRow> {
        validate_unit_identity(&input.identity)?;
        validate_ids(&[
            input.census_seal_id,
            input.stable_request_id,
            input.task_plan_id,
            input.primary_dispatch_receipt_id,
            input.primary_worker_run_id,
        ])?;
        validate_hashes(&[&input.seal_sha256])?;
        if let Some(existing) = self
            .load_census_by_request(&input.identity, input.stable_request_id)
            .await?
        {
            validate_census_replay(&existing, input)?;
            return Ok(existing);
        }
        sqlx::query(
            r#"WITH census AS (
                   SELECT * FROM investigation_effective_delegation_census_v2($3)
               )
               INSERT INTO investigation_pentagi_delegation_census_seals(
                   census_seal_id,stable_request_id,task_plan_id,primary_dispatch_receipt_id,
                   primary_worker_run_id,runnable_subtask_count,runnable_subtask_set_sha256,
                   dispatch_count,dispatch_set_sha256,pipeline_event_count,
                   pipeline_event_set_sha256,seal_sha256
               ) SELECT $1,$2,$3,$4,$5,census.runnable_subtask_count,
                        census.runnable_subtask_set_sha256,census.dispatch_count,
                        census.dispatch_set_sha256,census.pipeline_event_count,
                        census.pipeline_event_set_sha256,$6
                   FROM census"#,
        )
        .bind(input.census_seal_id)
        .bind(input.stable_request_id)
        .bind(input.task_plan_id)
        .bind(input.primary_dispatch_receipt_id)
        .bind(input.primary_worker_run_id)
        .bind(&input.seal_sha256)
        .execute(&*self.pool)
        .await?;
        let row = self
            .load_census_by_request(&input.identity, input.stable_request_id)
            .await?
            .ok_or(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                "delegation_census_missing",
            ))?;
        validate_census_replay(&row, input)?;
        Ok(row)
    }

    pub async fn seal_pentagi_plan(
        &self,
        identity: &InvestigationUnitIdentity,
        task_plan_id: Uuid,
        expected_row_version: u64,
    ) -> UnifiedInvestigationRuntimeStoreResult<PentagiTaskPlanRow> {
        validate_unit_identity(identity)?;
        validate_ids(&[task_plan_id])?;
        let expected = to_i64(expected_row_version, "plan_row_version")?;
        let current = self.load_plan_exact(identity, task_plan_id).await?;
        if current.status == "sealed" {
            if current.row_version != expected.saturating_add(1) {
                return Err(UnifiedInvestigationRuntimeStoreError::CasConflict(
                    "pentagi_plan_head",
                ));
            }
            return Ok(current);
        }
        let result = sqlx::query(
            r#"WITH census AS (
                   SELECT COUNT(*) AS member_count,
                          unified_investigation_exact_set_hash(
                              'investigation_pentagi_subtasks.v1',
                              COALESCE(array_agg(member_sha256 ORDER BY subtask_ordinal),ARRAY[]::TEXT[])
                          ) AS member_hash
                     FROM investigation_pentagi_subtasks WHERE task_plan_id=$1
               )
               UPDATE investigation_pentagi_task_plans plan
                  SET status='sealed',subtask_count=census.member_count,
                      subtask_set_sha256=census.member_hash,row_version=plan.row_version+1,
                      sealed_at=statement_timestamp()
                 FROM census
                WHERE plan.task_plan_id=$1 AND plan.authority_id=$2
                  AND plan.operation_id=$3 AND plan.stage_execution_id=$4
                  AND plan.stage_run_unit_id=$5 AND plan.organization_id=$6
                  AND plan.status='open' AND plan.row_version=$7"#,
        )
        .bind(task_plan_id)
        .bind(identity.stage.authority_id)
        .bind(identity.stage.operation_id)
        .bind(identity.stage.stage_execution_id)
        .bind(identity.stage_run_unit_id)
        .bind(identity.organization_id)
        .bind(expected)
        .execute(&*self.pool)
        .await?;
        if result.rows_affected() != 1 {
            return Err(UnifiedInvestigationRuntimeStoreError::CasConflict(
                "pentagi_plan_head",
            ));
        }
        self.load_plan_exact(identity, task_plan_id).await
    }

    pub async fn request_stop(
        &self,
        input: &RequestInvestigationStopInput,
    ) -> UnifiedInvestigationRuntimeStoreResult<InvestigationStopIntentRow> {
        validate_stage_identity(&input.identity)?;
        validate_ids(&[input.stop_intent_id, input.idempotency_key])?;
        validate_hashes(&[&input.expected_run_head_sha256])?;
        let change_seq = to_i64(input.expected_change_seq, "expected_change_seq")?;
        Ok(sqlx::query_as::<_, InvestigationStopIntentRow>(
            r#"SELECT stop_intent_id,idempotency_key,authority_id,operation_id,
                      stage_execution_id,owning_stage_run_request_id,
                      expected_run_head_sha256,expected_change_seq,stop_epoch,
                      frozen_work_count,frozen_work_set_sha256,receipt_sha256
                 FROM investigation_request_stop_v1($1,$2,$3,$4,$5,$6,$7,$8)"#,
        )
        .bind(input.stop_intent_id)
        .bind(input.idempotency_key)
        .bind(input.identity.authority_id)
        .bind(input.identity.operation_id)
        .bind(input.identity.stage_execution_id)
        .bind(&input.identity.owning_stage_run_request_id)
        .bind(&input.expected_run_head_sha256)
        .bind(change_seq)
        .fetch_one(&*self.pool)
        .await?)
    }

    pub async fn load_stop_intent(
        &self,
        identity: &InvestigationStageIdentity,
    ) -> UnifiedInvestigationRuntimeStoreResult<Option<InvestigationStopIntentRow>> {
        validate_stage_identity(identity)?;
        let rows = sqlx::query_as::<_, InvestigationStopIntentRow>(
            r#"SELECT stop.stop_intent_id,stop.idempotency_key,stop.authority_id,
                      stop.operation_id,stop.stage_execution_id,
                      stop.owning_stage_run_request_id,
                      stop.expected_run_head_sha256,stop.expected_change_seq,
                      stop.stop_epoch,stop.frozen_work_count,
                      stop.frozen_work_set_sha256,stop.receipt_sha256
                 FROM investigation_stop_intents stop
                 JOIN investigation_stage_run_authorities authority
                   ON authority.authority_id=stop.authority_id
                  AND authority.operation_id=stop.operation_id
                  AND authority.stage_execution_id=stop.stage_execution_id
                  AND authority.owning_stage_run_request_id=
                      stop.owning_stage_run_request_id
                WHERE stop.authority_id=$1 AND stop.operation_id=$2
                  AND stop.stage_execution_id=$3
                  AND stop.owning_stage_run_request_id=$4
                  AND authority.scope_snapshot_id=$5
                ORDER BY stop.stop_epoch,stop.stop_intent_id"#,
        )
        .bind(identity.authority_id)
        .bind(identity.operation_id)
        .bind(identity.stage_execution_id)
        .bind(&identity.owning_stage_run_request_id)
        .bind(identity.scope_snapshot_id)
        .fetch_all(&*self.pool)
        .await?;
        let stop = match rows.as_slice() {
            [] => return Ok(None),
            [stop] => stop.clone(),
            _ => {
                return Err(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                    "investigation_stop_intent_not_unique",
                ));
            }
        };
        let post_stop_head_is_exact: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1
                     FROM investigation_run_heads head
                     JOIN investigation_run_state_events event
                       ON event.event_id=head.latest_event_id
                      AND event.authority_id=head.authority_id
                    WHERE head.authority_id=$1
                      AND head.operation_id=$2
                      AND head.stage_execution_id=$3
                      AND head.owning_stage_run_request_id=$4
                      AND head.scope_snapshot_id=$5
                      AND head.run_state='stop_pending'
                      AND NOT head.admission_open
                      AND head.stop_epoch=$6
                      AND head.change_seq=$7+1
                      AND event.stable_request_id=$8
                      AND event.event_ordinal=head.head_version
                      AND event.expected_head_sha256=$9
                      AND event.from_state='running'
                      AND event.to_state='stop_pending'
                      AND event.stop_epoch=head.stop_epoch
                      AND event.change_seq=head.change_seq
                      AND head.head_sha256=unified_investigation_runtime_head_sha256(
                          head.authority_id,'stop_pending',FALSE,head.stop_epoch,
                          head.change_seq,head.head_version
                      )
               )"#,
        )
        .bind(identity.authority_id)
        .bind(identity.operation_id)
        .bind(identity.stage_execution_id)
        .bind(&identity.owning_stage_run_request_id)
        .bind(identity.scope_snapshot_id)
        .bind(stop.stop_epoch)
        .bind(stop.expected_change_seq)
        .bind(stop.idempotency_key)
        .bind(&stop.expected_run_head_sha256)
        .fetch_one(&*self.pool)
        .await?;
        if !post_stop_head_is_exact {
            return Err(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                "investigation_stop_intent_head_authority_mismatch",
            ));
        }
        Ok(Some(stop))
    }

    pub async fn seal_closure(
        &self,
        input: &SealInvestigationRunClosureInput,
    ) -> UnifiedInvestigationRuntimeStoreResult<InvestigationRunClosureRow> {
        validate_stage_identity(&input.identity)?;
        validate_ids(&[input.closure_id, input.stable_request_id])?;
        validate_hashes(&[&input.expected_run_head_sha256])?;
        let row = sqlx::query_as::<_, InvestigationRunClosureRow>(
            r#"SELECT header.stable_request_id,sealed.*
                 FROM seal_investigation_run_closure_v1($1,$2,$3,$4) sealed
                 JOIN investigation_run_closures header
                   ON header.closure_id=sealed.closure_id
                  AND header.authority_id=sealed.authority_id"#,
        )
        .bind(input.closure_id)
        .bind(input.stable_request_id)
        .bind(input.identity.authority_id)
        .bind(&input.expected_run_head_sha256)
        .fetch_one(&*self.pool)
        .await?;
        if row.operation_id != input.identity.operation_id
            || row.stage_execution_id != input.identity.stage_execution_id
            || row.owning_stage_run_request_id != input.identity.owning_stage_run_request_id
            || row.scope_snapshot_id != input.identity.scope_snapshot_id
        {
            return Err(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                "closure_stage_identity_mismatch",
            ));
        }
        Ok(row)
    }

    /// Load the complete DB-derived closure for a stage authority.
    pub async fn load_closure(
        &self,
        identity: &InvestigationStageIdentity,
    ) -> UnifiedInvestigationRuntimeStoreResult<Option<InvestigationRunClosureRow>> {
        validate_stage_identity(identity)?;
        Ok(sqlx::query_as::<_, InvestigationRunClosureRow>(
            r#"SELECT header.stable_request_id,detail.*
                 FROM investigation_run_closure_v1_authorities detail
                 JOIN investigation_run_closures header
                   ON header.closure_id=detail.closure_id
                  AND header.authority_id=detail.authority_id
                WHERE detail.authority_id=$1 AND detail.operation_id=$2
                  AND detail.stage_execution_id=$3
                  AND detail.owning_stage_run_request_id=$4
                  AND detail.scope_snapshot_id=$5"#,
        )
        .bind(identity.authority_id)
        .bind(identity.operation_id)
        .bind(identity.stage_execution_id)
        .bind(&identity.owning_stage_run_request_id)
        .bind(identity.scope_snapshot_id)
        .fetch_optional(&*self.pool)
        .await?)
    }

    async fn load_work_by_key(
        &self,
        identity: &InvestigationUnitIdentity,
        stable_work_key_sha256: &str,
    ) -> UnifiedInvestigationRuntimeStoreResult<Option<InvestigationRunWorkRow>> {
        Ok(
            sqlx::query_as::<_, InvestigationRunWorkRow>(WORK_ROW_SELECT_BY_KEY)
                .bind(identity.stage.authority_id)
                .bind(identity.stage.operation_id)
                .bind(identity.stage.stage_execution_id)
                .bind(&identity.stage.owning_stage_run_request_id)
                .bind(identity.stage_run_unit_id)
                .bind(identity.stage.scope_snapshot_id)
                .bind(identity.organization_id)
                .bind(stable_work_key_sha256)
                .fetch_optional(&*self.pool)
                .await?,
        )
    }

    async fn load_work_exact(
        &self,
        identity: &InvestigationUnitIdentity,
        work_id: Uuid,
    ) -> UnifiedInvestigationRuntimeStoreResult<InvestigationRunWorkRow> {
        Ok(
            sqlx::query_as::<_, InvestigationRunWorkRow>(WORK_ROW_SELECT_BY_ID)
                .bind(work_id)
                .bind(identity.stage.authority_id)
                .bind(identity.stage.operation_id)
                .bind(identity.stage.stage_execution_id)
                .bind(&identity.stage.owning_stage_run_request_id)
                .bind(identity.stage_run_unit_id)
                .bind(identity.stage.scope_snapshot_id)
                .bind(identity.organization_id)
                .fetch_one(&*self.pool)
                .await?,
        )
    }

    async fn load_work_event(
        &self,
        stable_request_id: Uuid,
    ) -> UnifiedInvestigationRuntimeStoreResult<Option<WorkStateEventRow>> {
        Ok(sqlx::query_as::<_, WorkStateEventRow>(
            r#"SELECT event_id,stable_request_id,work_id,expected_head_version,
                      event_ordinal,from_state,to_state,observed_stop_epoch,
                      reason_code,event_sha256
                 FROM investigation_run_work_state_events WHERE stable_request_id=$1"#,
        )
        .bind(stable_request_id)
        .fetch_optional(&*self.pool)
        .await?)
    }

    async fn load_plan_by_request(
        &self,
        identity: &InvestigationUnitIdentity,
        stable_request_id: Uuid,
    ) -> UnifiedInvestigationRuntimeStoreResult<Option<PentagiTaskPlanRow>> {
        Ok(
            sqlx::query_as::<_, PentagiTaskPlanRow>(PLAN_ROW_SELECT_BY_REQUEST)
                .bind(stable_request_id)
                .bind(identity.stage.authority_id)
                .bind(identity.stage.operation_id)
                .bind(identity.stage.stage_execution_id)
                .bind(&identity.stage.owning_stage_run_request_id)
                .bind(identity.stage_run_unit_id)
                .bind(identity.stage.scope_snapshot_id)
                .bind(identity.organization_id)
                .fetch_optional(&*self.pool)
                .await?,
        )
    }

    async fn load_plan_exact(
        &self,
        identity: &InvestigationUnitIdentity,
        task_plan_id: Uuid,
    ) -> UnifiedInvestigationRuntimeStoreResult<PentagiTaskPlanRow> {
        Ok(
            sqlx::query_as::<_, PentagiTaskPlanRow>(PLAN_ROW_SELECT_BY_ID)
                .bind(task_plan_id)
                .bind(identity.stage.authority_id)
                .bind(identity.stage.operation_id)
                .bind(identity.stage.stage_execution_id)
                .bind(&identity.stage.owning_stage_run_request_id)
                .bind(identity.stage_run_unit_id)
                .bind(identity.stage.scope_snapshot_id)
                .bind(identity.organization_id)
                .fetch_one(&*self.pool)
                .await?,
        )
    }

    async fn load_subtask_exact(
        &self,
        identity: &InvestigationUnitIdentity,
        task_plan_id: Uuid,
        subtask_id: Uuid,
    ) -> UnifiedInvestigationRuntimeStoreResult<PentagiSubtaskRow> {
        Ok(sqlx::query_as::<_, PentagiSubtaskRow>(
            r#"SELECT subtask.subtask_id,subtask.task_plan_id,subtask.authority_id,
                      subtask.operation_id,subtask.stage_execution_id,
                      subtask.stage_run_unit_id,subtask.organization_id,
                      subtask.subtask_ordinal,subtask.label,subtask.runnable,
                      subtask.input_manifest_sha256,subtask.expected_output_schema,
                      subtask.member_sha256
                 FROM investigation_pentagi_subtasks subtask
                 JOIN investigation_pentagi_task_plans plan
                   ON plan.task_plan_id=subtask.task_plan_id
                WHERE subtask.subtask_id=$1 AND subtask.task_plan_id=$2
                  AND subtask.authority_id=$3 AND subtask.operation_id=$4
                  AND subtask.stage_execution_id=$5 AND subtask.stage_run_unit_id=$6
                  AND subtask.organization_id=$7
                  AND plan.owning_stage_run_request_id=$8
                  AND plan.scope_snapshot_id=$9"#,
        )
        .bind(subtask_id)
        .bind(task_plan_id)
        .bind(identity.stage.authority_id)
        .bind(identity.stage.operation_id)
        .bind(identity.stage.stage_execution_id)
        .bind(identity.stage_run_unit_id)
        .bind(identity.organization_id)
        .bind(&identity.stage.owning_stage_run_request_id)
        .bind(identity.stage.scope_snapshot_id)
        .fetch_one(&*self.pool)
        .await?)
    }

    async fn load_run_request_exact(
        &self,
        identity: &InvestigationUnitIdentity,
        stable_request_id: Uuid,
    ) -> UnifiedInvestigationRuntimeStoreResult<PentagiTaskRunRequestRow> {
        Ok(sqlx::query_as::<_, PentagiTaskRunRequestRow>(
            r#"SELECT run_request_id,stable_request_id,task_plan_id,authority_id,
                      operation_id,stage_execution_id,owning_stage_run_request_id,
                      stage_run_unit_id,organization_id,subject_kind,subject_id,
                      subject_fingerprint_sha256,request_sha256
                 FROM pentagi_task_run_requests
                WHERE stable_request_id=$1 AND authority_id=$2 AND operation_id=$3
                  AND stage_execution_id=$4 AND owning_stage_run_request_id=$5
                  AND stage_run_unit_id=$6 AND organization_id=$7"#,
        )
        .bind(stable_request_id)
        .bind(identity.stage.authority_id)
        .bind(identity.stage.operation_id)
        .bind(identity.stage.stage_execution_id)
        .bind(&identity.stage.owning_stage_run_request_id)
        .bind(identity.stage_run_unit_id)
        .bind(identity.organization_id)
        .fetch_one(&*self.pool)
        .await?)
    }

    async fn load_dispatch_exact(
        &self,
        identity: &InvestigationUnitIdentity,
        stable_request_id: Uuid,
    ) -> UnifiedInvestigationRuntimeStoreResult<PentagiLogicalDispatchRow> {
        Ok(
            sqlx::query_as::<_, PentagiLogicalDispatchRow>(DISPATCH_ROW_SELECT_BY_REQUEST)
                .bind(stable_request_id)
                .bind(identity.stage.operation_id)
                .bind(identity.stage.stage_execution_id)
                .bind(identity.stage_run_unit_id)
                .bind(identity.stage.scope_snapshot_id)
                .bind(identity.organization_id)
                .bind(identity.stage.authority_id)
                .bind(&identity.stage.owning_stage_run_request_id)
                .fetch_one(&*self.pool)
                .await?,
        )
    }

    async fn load_dispatch_by_id_exact(
        &self,
        identity: &InvestigationUnitIdentity,
        task_plan_id: Uuid,
        dispatch_receipt_id: Uuid,
    ) -> UnifiedInvestigationRuntimeStoreResult<PentagiLogicalDispatchRow> {
        Ok(
            sqlx::query_as::<_, PentagiLogicalDispatchRow>(DISPATCH_ROW_SELECT_BY_ID)
                .bind(dispatch_receipt_id)
                .bind(task_plan_id)
                .bind(identity.stage.operation_id)
                .bind(identity.stage.stage_execution_id)
                .bind(identity.stage_run_unit_id)
                .bind(identity.stage.scope_snapshot_id)
                .bind(identity.organization_id)
                .bind(identity.stage.authority_id)
                .bind(&identity.stage.owning_stage_run_request_id)
                .fetch_one(&*self.pool)
                .await?,
        )
    }

    async fn load_census_by_request(
        &self,
        identity: &InvestigationUnitIdentity,
        stable_request_id: Uuid,
    ) -> UnifiedInvestigationRuntimeStoreResult<Option<PentagiDelegationCensusRow>> {
        Ok(sqlx::query_as::<_, PentagiDelegationCensusRow>(
            r#"SELECT census.census_seal_id,census.stable_request_id,census.task_plan_id,
                      census.primary_dispatch_receipt_id,census.primary_worker_run_id,
                      census.runnable_subtask_count,census.runnable_subtask_set_sha256,
                      census.dispatch_count,census.dispatch_set_sha256,
                      census.pipeline_event_count,census.pipeline_event_set_sha256,
                      census.seal_sha256
                 FROM investigation_pentagi_delegation_census_seals census
                 JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=census.task_plan_id
                WHERE census.stable_request_id=$1 AND plan.authority_id=$2
                  AND plan.operation_id=$3 AND plan.stage_execution_id=$4
                  AND plan.owning_stage_run_request_id=$5
                  AND plan.stage_run_unit_id=$6 AND plan.scope_snapshot_id=$7
                  AND plan.organization_id=$8"#,
        )
        .bind(stable_request_id)
        .bind(identity.stage.authority_id)
        .bind(identity.stage.operation_id)
        .bind(identity.stage.stage_execution_id)
        .bind(&identity.stage.owning_stage_run_request_id)
        .bind(identity.stage_run_unit_id)
        .bind(identity.stage.scope_snapshot_id)
        .bind(identity.organization_id)
        .fetch_optional(&*self.pool)
        .await?)
    }

    pub async fn publish_closure(
        &self,
        input: &PublishInvestigationStageClosureInput,
    ) -> UnifiedInvestigationRuntimeStoreResult<PublishedInvestigationStageClosureRow> {
        validate_stage_identity(&input.identity)?;
        validate_ids(&[
            input.publication_id,
            input.stable_request_id,
            input.closure_id,
        ])?;
        let replayed: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM investigation_stage_closure_publications
                            WHERE stable_request_id=$1 OR closure_id=$2)",
        )
        .bind(input.stable_request_id)
        .bind(input.closure_id)
        .fetch_one(&*self.pool)
        .await?;
        let publication = sqlx::query_as::<_, InvestigationStageClosurePublicationRow>(
            "SELECT * FROM publish_investigation_stage_closure_v1($1,$2,$3)",
        )
        .bind(input.publication_id)
        .bind(input.stable_request_id)
        .bind(input.closure_id)
        .fetch_one(&*self.pool)
        .await?;
        if publication.authority_id != input.identity.authority_id
            || publication.operation_id != input.identity.operation_id
            || publication.stage_execution_id != input.identity.stage_execution_id
            || publication.scope_snapshot_id != input.identity.scope_snapshot_id
            || publication.closure_id != input.closure_id
            || publication.publication_id != input.publication_id
            || publication.stable_request_id != input.stable_request_id
        {
            return Err(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                "closure_publication_stage_identity_mismatch",
            ));
        }
        let members = sqlx::query_as::<_, InvestigationStageClosurePublicationMemberRow>(
            r#"SELECT publication_member_id,publication_id,member_ordinal,operation_id,
                      stage_execution_id,scope_snapshot_id,
                      stage_run_unit_id,organization_id,stage_team_plan_id,
                      member_sha256,passed_at
                 FROM investigation_stage_closure_publication_members
                WHERE publication_id=$1 ORDER BY member_ordinal"#,
        )
        .bind(publication.publication_id)
        .fetch_all(&*self.pool)
        .await?;
        if i64::try_from(members.len()).ok() != Some(publication.member_count) {
            return Err(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                "closure_publication_member_count_mismatch",
            ));
        }
        Ok(PublishedInvestigationStageClosureRow {
            publication,
            members,
            replayed,
        })
    }

    /// Reload the one immutable closure publication owned by an operation and
    /// revalidate its complete member/hash authority. This is the read-side
    /// seam shared by the specialist Gate and Reporting; neither caller may
    /// treat the mutable per-org completion projection as the closure itself.
    pub async fn load_closure_publication_for_operation(
        &self,
        operation_id: Uuid,
    ) -> UnifiedInvestigationRuntimeStoreResult<Option<PublishedInvestigationStageClosureRow>> {
        validate_ids(&[operation_id])?;
        let publications = sqlx::query_as::<_, InvestigationStageClosurePublicationRow>(
            r#"SELECT publication_id,stable_request_id,closure_id,authority_id,operation_id,
                      stage_execution_id,owning_stage_run_request_id,scope_snapshot_id,
                      closure_sha256,disposition,member_count,member_set_sha256,
                      publication_sha256,published_at
                 FROM investigation_stage_closure_publications
                WHERE operation_id=$1
                ORDER BY published_at,publication_id"#,
        )
        .bind(operation_id)
        .fetch_all(&*self.pool)
        .await?;
        let publication = match publications.as_slice() {
            [] => return Ok(None),
            [publication] => publication.clone(),
            _ => {
                return Err(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                    "closure_publication_operation_not_unique",
                ));
            }
        };
        let members = sqlx::query_as::<_, InvestigationStageClosurePublicationMemberRow>(
            r#"SELECT publication_member_id,publication_id,member_ordinal,operation_id,
                      stage_execution_id,scope_snapshot_id,stage_run_unit_id,
                      organization_id,stage_team_plan_id,member_sha256,passed_at
                 FROM investigation_stage_closure_publication_members
                WHERE publication_id=$1 ORDER BY member_ordinal"#,
        )
        .bind(publication.publication_id)
        .fetch_all(&*self.pool)
        .await?;
        let authority_valid: bool = sqlx::query_scalar(
            r#"SELECT publication.member_count=(
                           SELECT COUNT(*)
                             FROM investigation_stage_closure_publication_members member
                            WHERE member.publication_id=publication.publication_id
                       )
                      AND publication.member_set_sha256=(
                           SELECT unified_investigation_exact_set_hash(
                               'investigation_stage_closure_publication_members.v1',
                               COALESCE(array_agg(member.member_sha256
                                                  ORDER BY member.organization_id,
                                                           member.stage_run_unit_id),
                                        ARRAY[]::TEXT[])
                           )
                             FROM investigation_stage_closure_publication_members member
                            WHERE member.publication_id=publication.publication_id
                       )
                      AND publication.publication_sha256=tool_truth_sha256(jsonb_build_object(
                           'contract_version','investigation-stage-closure-publication.v1',
                           'publication_id',publication.publication_id,
                           'closure_id',publication.closure_id,
                           'closure_sha256',publication.closure_sha256,
                           'authority_id',publication.authority_id,
                           'operation_id',publication.operation_id,
                           'stage_execution_id',publication.stage_execution_id,
                           'owning_stage_run_request_id',publication.owning_stage_run_request_id,
                           'scope_snapshot_id',publication.scope_snapshot_id,
                           'disposition',publication.disposition,
                           'member_count',publication.member_count,
                           'member_set_sha256',publication.member_set_sha256
                       )::TEXT)
                      AND closure.closure_sha256=publication.closure_sha256
                      AND closure_authority.disposition=publication.disposition
                      AND closure_authority.operation_id=publication.operation_id
                      AND closure_authority.stage_execution_id=publication.stage_execution_id
                      AND closure_authority.owning_stage_run_request_id=
                          publication.owning_stage_run_request_id
                      AND closure_authority.scope_snapshot_id=publication.scope_snapshot_id
                      AND closure_authority.closure_sha256=publication.closure_sha256
                      AND publication.member_count=closure_authority.snapshot_member_count
                      AND EXISTS(
                           SELECT 1 FROM investigation_run_heads head
                            WHERE head.authority_id=publication.authority_id
                              AND head.operation_id=publication.operation_id
                              AND head.stage_execution_id=publication.stage_execution_id
                              AND head.owning_stage_run_request_id=
                                  publication.owning_stage_run_request_id
                              AND head.scope_snapshot_id=publication.scope_snapshot_id
                              AND head.run_state='closed' AND NOT head.admission_open
                       )
                      AND NOT EXISTS(
                           SELECT 1
                             FROM investigation_stage_closure_publication_members member
                             JOIN stage_run_units unit ON unit.id=member.stage_run_unit_id
                             JOIN stage_team_plans plan ON plan.id=member.stage_team_plan_id
                            WHERE member.publication_id=publication.publication_id
                              AND (unit.operation_id<>publication.operation_id
                                   OR unit.stage_execution_id<>publication.stage_execution_id
                                   OR unit.scope_snapshot_id<>publication.scope_snapshot_id
                                   OR unit.organization_id<>member.organization_id
                                   OR member.member_sha256<>tool_truth_sha256(
                                        jsonb_build_object(
                                            'contract_version',
                                            'investigation-stage-closure-member.v1',
                                            'closure_id',publication.closure_id,
                                            'closure_sha256',publication.closure_sha256,
                                            'stage_run_unit_id',member.stage_run_unit_id,
                                            'organization_id',member.organization_id,
                                            'stage_team_plan_id',member.stage_team_plan_id
                                        )::TEXT)
                                   OR unit.status<>'passed'
                                   OR unit.terminal_at IS DISTINCT FROM member.passed_at
                                   OR plan.requests_closed_at IS NULL
                                   OR unit.pass_watermark->>'schema' IS DISTINCT FROM
                                      'investigation_stage_closure_publication.v1'
                                   OR unit.pass_watermark->>'publication_id' IS DISTINCT FROM
                                      publication.publication_id::TEXT
                                   OR unit.pass_watermark->>'closure_id' IS DISTINCT FROM
                                      publication.closure_id::TEXT
                                   OR unit.pass_watermark->>'closure_sha256' IS DISTINCT FROM
                                      publication.closure_sha256
                                   OR unit.pass_watermark->>'disposition' IS DISTINCT FROM
                                      publication.disposition
                                   OR unit.pass_watermark->>'member_sha256' IS DISTINCT FROM
                                      member.member_sha256
                                   OR plan.stage_run_unit_id<>unit.id
                                   OR plan.organization_id<>member.organization_id)
                       )
                      AND NOT EXISTS(
                           SELECT 1
                             FROM investigation_stage_closure_publication_members member
                             LEFT JOIN org_stage_completions completion
                               ON completion.organization_id=member.organization_id
                              AND completion.stage_kind='investigation'
                            WHERE member.publication_id=publication.publication_id
                              AND (completion.stage_run_id IS DISTINCT FROM
                                      publication.operation_id::TEXT
                                   OR completion.passed_at IS DISTINCT FROM member.passed_at)
                       )
                      AND NOT EXISTS(
                           SELECT 1
                             FROM (
                                  SELECT member.member_ordinal,
                                         row_number() OVER(ORDER BY member.member_ordinal)-1
                                             AS expected_ordinal
                                    FROM investigation_stage_closure_publication_members member
                                   WHERE member.publication_id=publication.publication_id
                             ) ordered
                            WHERE ordered.member_ordinal<>ordered.expected_ordinal
                       )
                      AND NOT EXISTS(
                           SELECT 1
                             FROM stage_run_units unit
                            WHERE unit.operation_id=publication.operation_id
                              AND unit.stage_execution_id=publication.stage_execution_id
                              AND unit.scope_snapshot_id=publication.scope_snapshot_id
                              AND NOT EXISTS(
                                   SELECT 1
                                     FROM investigation_stage_closure_publication_members member
                                    WHERE member.publication_id=publication.publication_id
                                      AND member.stage_run_unit_id=unit.id
                              )
                       )
                 FROM investigation_stage_closure_publications publication
                 JOIN investigation_run_closures closure
                   ON closure.closure_id=publication.closure_id
                  AND closure.authority_id=publication.authority_id
                 JOIN investigation_run_closure_v1_authorities closure_authority
                   ON closure_authority.closure_id=publication.closure_id
                WHERE publication.publication_id=$1"#,
        )
        .bind(publication.publication_id)
        .fetch_one(&*self.pool)
        .await?;
        if !authority_valid || i64::try_from(members.len()).ok() != Some(publication.member_count) {
            return Err(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                "closure_publication_authority_invalid",
            ));
        }
        Ok(Some(PublishedInvestigationStageClosureRow {
            publication,
            members,
            replayed: true,
        }))
    }
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
struct WorkStateEventRow {
    event_id: Uuid,
    stable_request_id: Uuid,
    work_id: Uuid,
    expected_head_version: i64,
    event_ordinal: i64,
    from_state: String,
    to_state: String,
    observed_stop_epoch: i64,
    reason_code: String,
    event_sha256: String,
}

#[cfg(test)]
const WORK_ROW_COLUMNS: &str = r#"work.work_id,work.asset_lane_id,work.stable_work_key_sha256,
    work.authority_id,work.operation_id,work.stage_execution_id,
    work.owning_stage_run_request_id,work.stage_run_unit_id,work.scope_snapshot_id,
    work.organization_id,work.work_kind,work.external_identity_sha256,
    work.current_state,work.observed_stop_epoch,work.head_version,work.latest_event_id"#;

const WORK_ROW_SELECT_BY_KEY: &str = r#"SELECT work.work_id,work.asset_lane_id,work.stable_work_key_sha256,
    work.authority_id,work.operation_id,work.stage_execution_id,
    work.owning_stage_run_request_id,work.stage_run_unit_id,work.scope_snapshot_id,
    work.organization_id,work.work_kind,work.external_identity_sha256,
    work.current_state,work.observed_stop_epoch,work.head_version,work.latest_event_id
    FROM investigation_run_work_items work
    WHERE work.authority_id=$1 AND work.operation_id=$2 AND work.stage_execution_id=$3
      AND work.owning_stage_run_request_id=$4 AND work.stage_run_unit_id=$5
      AND work.scope_snapshot_id=$6 AND work.organization_id=$7
      AND work.stable_work_key_sha256=$8"#;

const WORK_ROW_SELECT_BY_ID: &str = r#"SELECT work.work_id,work.asset_lane_id,work.stable_work_key_sha256,
    work.authority_id,work.operation_id,work.stage_execution_id,
    work.owning_stage_run_request_id,work.stage_run_unit_id,work.scope_snapshot_id,
    work.organization_id,work.work_kind,work.external_identity_sha256,
    work.current_state,work.observed_stop_epoch,work.head_version,work.latest_event_id
    FROM investigation_run_work_items work
    WHERE work.work_id=$1 AND work.authority_id=$2 AND work.operation_id=$3
      AND work.stage_execution_id=$4 AND work.owning_stage_run_request_id=$5
      AND work.stage_run_unit_id=$6 AND work.scope_snapshot_id=$7
      AND work.organization_id=$8"#;

fn validate_generator_consumer_fence(
    fence: &InvestigationGeneratorConsumerFenceInput,
) -> UnifiedInvestigationRuntimeStoreResult<()> {
    validate_ids(&[
        fence.current_consumer_work_item_id,
        fence.current_consumer_worker_run_id,
        fence.current_consumer_lease_token,
    ])?;
    to_i64(
        fence.expected_consumer_attempt_epoch,
        "generator_consumer_attempt_epoch",
    )?;
    to_i64(
        fence.expected_consumer_checkpoint_version,
        "generator_consumer_checkpoint_version",
    )?;
    Ok(())
}

fn validate_generator_materialization_input(
    input: &MaterializeInvestigationGeneratorInput,
) -> UnifiedInvestigationRuntimeStoreResult<()> {
    validate_unit_identity(&input.identity)?;
    validate_ids(&[
        input.task_plan_id,
        input.ledger_id,
        input.stable_request_id,
        input.generator_pipeline_event_id,
        input.source_receipt_id,
        input.source_tool_call_id,
    ])?;
    validate_generator_consumer_fence(&input.consumer_fence)?;
    if input.subtasks.len() > 8 {
        return Err(UnifiedInvestigationRuntimeStoreError::InvalidInput(
            "generator_subtask_count",
        ));
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut ordinals = std::collections::BTreeSet::new();
    for member in &input.subtasks {
        validate_ids(&[member.subtask_id])?;
        validate_hashes(&[&member.input_manifest_sha256, &member.member_sha256])?;
        validate_bounded(&member.label, 512, "subtask_label")?;
        validate_bounded(
            &member.expected_output_schema,
            512,
            "expected_output_schema",
        )?;
        if !ids.insert(member.subtask_id)
            || !ordinals.insert(member.subtask_ordinal)
            || member.subtask_ordinal as usize >= input.subtasks.len()
        {
            return Err(UnifiedInvestigationRuntimeStoreError::InvalidInput(
                "generator_subtask_census",
            ));
        }
    }
    Ok(())
}

async fn lock_generator_consumer_on(
    tx: &mut Transaction<'_, Postgres>,
    identity: &InvestigationUnitIdentity,
    task_plan_id: Uuid,
    fence: &InvestigationGeneratorConsumerFenceInput,
) -> UnifiedInvestigationRuntimeStoreResult<(Uuid, Uuid, Uuid)> {
    let attempt_epoch = to_i64(
        fence.expected_consumer_attempt_epoch,
        "generator_consumer_attempt_epoch",
    )?;
    let checkpoint_version = to_i64(
        fence.expected_consumer_checkpoint_version,
        "generator_consumer_checkpoint_version",
    )?;
    sqlx::query_as::<_, (Uuid, Uuid, Uuid)>(
        r#"SELECT current_authority.source_schedule_receipt_id,
                  current_authority.primary_work_item_id,
                  current_authority.primary_worker_run_id
             FROM investigation_pentagi_task_plans plan
             JOIN investigation_asset_primary_current_authorities current_authority
               ON current_authority.stage_team_plan_id=plan.stage_team_plan_id
              AND current_authority.operation_id=plan.operation_id
              AND current_authority.stage_execution_id=plan.stage_execution_id
              AND current_authority.stage_run_unit_id=plan.stage_run_unit_id
              AND current_authority.scope_snapshot_id=plan.scope_snapshot_id
              AND current_authority.organization_id=plan.organization_id
             JOIN stage_work_items consumer_item
               ON consumer_item.id=current_authority.primary_work_item_id
              AND consumer_item.team_plan_id=current_authority.stage_team_plan_id
              AND consumer_item.operation_id=current_authority.operation_id
              AND consumer_item.stage_execution_id=current_authority.stage_execution_id
              AND consumer_item.stage_run_unit_id=current_authority.stage_run_unit_id
              AND consumer_item.organization_id=current_authority.organization_id
             JOIN stage_worker_runs consumer_worker
               ON consumer_worker.id=current_authority.primary_worker_run_id
              AND consumer_worker.work_item_id=consumer_item.id
              AND consumer_worker.operation_id=current_authority.operation_id
              AND consumer_worker.stage_execution_id=current_authority.stage_execution_id
              AND consumer_worker.stage_run_unit_id=current_authority.stage_run_unit_id
              AND consumer_worker.organization_id=current_authority.organization_id
            WHERE plan.task_plan_id=$1 AND plan.authority_id=$2
              AND plan.operation_id=$3 AND plan.stage_execution_id=$4
              AND plan.owning_stage_run_request_id=$5 AND plan.stage_run_unit_id=$6
              AND plan.scope_snapshot_id=$7 AND plan.organization_id=$8
              AND plan.status='open'
              AND consumer_item.id=$9 AND consumer_item.status='running'
              AND consumer_worker.id=$10 AND consumer_worker.status='running'
              AND consumer_worker.lease_token=$11
              AND consumer_worker.attempt_epoch=$12
              AND consumer_worker.checkpoint_version=$13
              AND consumer_worker.active_tool_call_id IS NULL
            FOR UPDATE OF plan,consumer_item,consumer_worker"#,
    )
    .bind(task_plan_id)
    .bind(identity.stage.authority_id)
    .bind(identity.stage.operation_id)
    .bind(identity.stage.stage_execution_id)
    .bind(&identity.stage.owning_stage_run_request_id)
    .bind(identity.stage_run_unit_id)
    .bind(identity.stage.scope_snapshot_id)
    .bind(identity.organization_id)
    .bind(fence.current_consumer_work_item_id)
    .bind(fence.current_consumer_worker_run_id)
    .bind(fence.current_consumer_lease_token)
    .bind(attempt_epoch)
    .bind(checkpoint_version)
    .fetch_one(&mut **tx)
    .await
    .map_err(UnifiedInvestigationRuntimeStoreError::from)
}

async fn load_generator_candidates_on(
    tx: &mut Transaction<'_, Postgres>,
    identity: &InvestigationUnitIdentity,
    task_plan_id: Uuid,
    source_tool_call_id: Option<Uuid>,
) -> UnifiedInvestigationRuntimeStoreResult<Vec<InvestigationFinishedSubmitResultCandidateRow>> {
    Ok(
        sqlx::query_as::<_, InvestigationFinishedSubmitResultCandidateRow>(
            r#"SELECT source_call.id AS source_tool_call_id,
                  source_call.call_id AS source_provider_call_id,
                  source_call.attempt_epoch AS source_attempt_epoch,
                  source_worker.work_item_id AS source_work_item_id,
                  source_worker.id AS source_worker_run_id,
                  source_call.args->'result' AS canonical_result,
                  tool_truth_sha256((source_call.args->'result')::TEXT)
                      AS canonical_result_sha256
             FROM investigation_pentagi_task_plans plan
             JOIN pentagi_logical_dispatch_receipts dispatch
               ON dispatch.task_plan_id=plan.task_plan_id AND dispatch.actor_kind='primary'
             JOIN stage_worker_runs source_worker
               ON source_worker.id=dispatch.worker_run_id
              AND source_worker.work_item_id=dispatch.stage_work_item_id
              AND source_worker.operation_id=plan.operation_id
              AND source_worker.stage_execution_id=plan.stage_execution_id
              AND source_worker.stage_run_unit_id=plan.stage_run_unit_id
              AND source_worker.organization_id=plan.organization_id
             JOIN tool_calls source_call
               ON source_call.worker_run_id=source_worker.id
              AND source_call.operation_id=plan.operation_id
              AND source_call.stage_execution_id=plan.stage_execution_id
              AND source_call.stage_run_unit_id=plan.stage_run_unit_id
              AND source_call.organization_id=plan.organization_id
            WHERE plan.task_plan_id=$1 AND plan.authority_id=$2
              AND plan.operation_id=$3 AND plan.stage_execution_id=$4
              AND plan.owning_stage_run_request_id=$5 AND plan.stage_run_unit_id=$6
              AND plan.scope_snapshot_id=$7 AND plan.organization_id=$8
              AND plan.status='open'
              AND investigation_refiner_primary_source_is_current_v3(
                  plan.task_plan_id,dispatch.stage_work_item_id,dispatch.worker_run_id)
              AND source_call.name='submit_result' AND source_call.status='finished'
              AND source_call.result IS NOT NULL
              AND source_call.result::JSONB->>'status'='result submitted'
              AND source_call.args ? 'result'
              AND ($9::UUID IS NULL OR source_call.id=$9)
            ORDER BY source_call.created_at,source_call.id"#,
        )
        .bind(task_plan_id)
        .bind(identity.stage.authority_id)
        .bind(identity.stage.operation_id)
        .bind(identity.stage.stage_execution_id)
        .bind(&identity.stage.owning_stage_run_request_id)
        .bind(identity.stage_run_unit_id)
        .bind(identity.stage.scope_snapshot_id)
        .bind(identity.organization_id)
        .bind(source_tool_call_id)
        .fetch_all(&mut **tx)
        .await?,
    )
}

async fn load_generator_source_on(
    tx: &mut Transaction<'_, Postgres>,
    identity: &InvestigationUnitIdentity,
    task_plan_id: Uuid,
    source_tool_call_id: Uuid,
) -> UnifiedInvestigationRuntimeStoreResult<InvestigationFinishedSubmitResultCandidateRow> {
    load_generator_candidates_on(tx, identity, task_plan_id, Some(source_tool_call_id))
        .await?
        .into_iter()
        .next()
        .ok_or(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
            "generator_source_authority_mismatch",
        ))
}

async fn load_generator_subtasks_on(
    tx: &mut Transaction<'_, Postgres>,
    identity: &InvestigationUnitIdentity,
    task_plan_id: Uuid,
) -> UnifiedInvestigationRuntimeStoreResult<Vec<PentagiSubtaskRow>> {
    Ok(sqlx::query_as::<_, PentagiSubtaskRow>(
        r#"SELECT subtask.subtask_id,subtask.task_plan_id,subtask.authority_id,
                  subtask.operation_id,subtask.stage_execution_id,
                  subtask.stage_run_unit_id,subtask.organization_id,
                  subtask.subtask_ordinal,subtask.label,subtask.runnable,
                  subtask.input_manifest_sha256,subtask.expected_output_schema,
                  subtask.member_sha256
             FROM investigation_pentagi_subtasks subtask
             JOIN investigation_pentagi_task_plans plan
               ON plan.task_plan_id=subtask.task_plan_id
            WHERE subtask.task_plan_id=$1 AND subtask.authority_id=$2
              AND subtask.operation_id=$3 AND subtask.stage_execution_id=$4
              AND subtask.stage_run_unit_id=$5 AND subtask.organization_id=$6
              AND plan.owning_stage_run_request_id=$7 AND plan.scope_snapshot_id=$8
            ORDER BY subtask.subtask_ordinal,subtask.subtask_id"#,
    )
    .bind(task_plan_id)
    .bind(identity.stage.authority_id)
    .bind(identity.stage.operation_id)
    .bind(identity.stage.stage_execution_id)
    .bind(identity.stage_run_unit_id)
    .bind(identity.organization_id)
    .bind(&identity.stage.owning_stage_run_request_id)
    .bind(identity.stage.scope_snapshot_id)
    .fetch_all(&mut **tx)
    .await?)
}

async fn insert_generator_subtask_on(
    tx: &mut Transaction<'_, Postgres>,
    identity: &InvestigationUnitIdentity,
    task_plan_id: Uuid,
    member: &InvestigationGeneratorSubtaskInput,
) -> UnifiedInvestigationRuntimeStoreResult<()> {
    let ordinal = i32::try_from(member.subtask_ordinal)
        .map_err(|_| UnifiedInvestigationRuntimeStoreError::InvalidInput("subtask_ordinal"))?;
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_subtasks(
               subtask_id,task_plan_id,authority_id,operation_id,stage_execution_id,
               stage_run_unit_id,organization_id,subtask_ordinal,label,runnable,
               input_manifest_sha256,expected_output_schema,member_sha256)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
           ON CONFLICT(subtask_id) DO NOTHING"#,
    )
    .bind(member.subtask_id)
    .bind(task_plan_id)
    .bind(identity.stage.authority_id)
    .bind(identity.stage.operation_id)
    .bind(identity.stage.stage_execution_id)
    .bind(identity.stage_run_unit_id)
    .bind(identity.organization_id)
    .bind(ordinal)
    .bind(&member.label)
    .bind(member.runnable)
    .bind(&member.input_manifest_sha256)
    .bind(&member.expected_output_schema)
    .bind(&member.member_sha256)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn ensure_generator_subtask_request_exact(
    actual: &[PentagiSubtaskRow],
    requested: &[InvestigationGeneratorSubtaskInput],
) -> UnifiedInvestigationRuntimeStoreResult<()> {
    if actual.len() != requested.len()
        || actual.iter().zip(requested).any(|(row, member)| {
            row.subtask_id != member.subtask_id
                || row.subtask_ordinal != member.subtask_ordinal as i32
                || row.label != member.label
                || row.runnable != member.runnable
                || row.input_manifest_sha256 != member.input_manifest_sha256
                || row.expected_output_schema != member.expected_output_schema
                || row.member_sha256 != member.member_sha256
        })
    {
        return Err(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
            "generator_subtask_request_mismatch",
        ));
    }
    Ok(())
}

async fn create_generator_ledger_on(
    tx: &mut Transaction<'_, Postgres>,
    ledger_id: Uuid,
    stable_request_id: Uuid,
    task_plan_id: Uuid,
    generator_pipeline_event_id: Uuid,
    generator_manifest: &Value,
) -> UnifiedInvestigationRuntimeStoreResult<InvestigationRefinerPlanLedgerRow> {
    Ok(sqlx::query_as::<_, InvestigationRefinerPlanLedgerRow>(
        r#"SELECT ledger_id,stable_request_id,task_plan_id,generator_pipeline_event_id,
                  generator_manifest,generator_manifest_sha256,generator_subtask_count,
                  generator_subtask_set_sha256,ledger_sha256
             FROM create_investigation_refiner_plan_ledger_v2($1,$2,$3,$4,$5)"#,
    )
    .bind(ledger_id)
    .bind(stable_request_id)
    .bind(task_plan_id)
    .bind(generator_pipeline_event_id)
    .bind(generator_manifest)
    .fetch_one(&mut **tx)
    .await?)
}

async fn load_generator_source_receipt_on(
    tx: &mut Transaction<'_, Postgres>,
    stable_request_id: Uuid,
) -> UnifiedInvestigationRuntimeStoreResult<Option<InvestigationGeneratorSourceReceiptRow>> {
    Ok(sqlx::query_as::<_, InvestigationGeneratorSourceReceiptRow>(
        r#"SELECT source_receipt_id,stable_request_id,task_plan_id,ledger_id,
                  generator_pipeline_event_id,source_tool_call_id,source_provider_call_id,
                  source_attempt_epoch,source_work_item_id,source_worker_run_id,
                  current_consumer_work_item_id,current_consumer_worker_run_id,
                  current_consumer_lease_token,current_consumer_attempt_epoch,
                  current_consumer_checkpoint_version,
                  canonical_result_sha256,adopted_subtask_count,
                  adopted_subtask_set_sha256,receipt_kind,receipt_sha256
             FROM investigation_generator_source_receipts WHERE stable_request_id=$1"#,
    )
    .bind(stable_request_id)
    .fetch_optional(&mut **tx)
    .await?)
}

async fn insert_generator_source_receipt_on(
    tx: &mut Transaction<'_, Postgres>,
    source_receipt_id: Uuid,
    stable_request_id: Uuid,
    identity: &InvestigationUnitIdentity,
    ledger: &InvestigationRefinerPlanLedgerRow,
    source: &InvestigationFinishedSubmitResultCandidateRow,
    fence: &InvestigationGeneratorConsumerFenceInput,
    receipt_kind: &str,
) -> UnifiedInvestigationRuntimeStoreResult<()> {
    let (subtask_count, subtask_set_sha256): (i64, String) = sqlx::query_as(
        r#"SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_generator_adopted_subtasks.v1',
               COALESCE(array_agg(subtask.subtask_id::TEXT || ':' || subtask.member_sha256
                                  ORDER BY subtask.subtask_ordinal),ARRAY[]::TEXT[]))
             FROM investigation_pentagi_subtasks subtask WHERE subtask.task_plan_id=$1"#,
    )
    .bind(ledger.task_plan_id)
    .fetch_one(&mut **tx)
    .await?;
    let receipt_sha256: String = sqlx::query_scalar(
        r#"SELECT investigation_generator_source_receipt_sha256_v1(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,
               $19,$20,$21,$22,$23,$24)"#,
    )
    .bind(source_receipt_id)
    .bind(stable_request_id)
    .bind(ledger.task_plan_id)
    .bind(ledger.ledger_id)
    .bind(ledger.generator_pipeline_event_id)
    .bind(source.source_tool_call_id)
    .bind(&source.source_provider_call_id)
    .bind(source.source_attempt_epoch)
    .bind(source.source_work_item_id)
    .bind(source.source_worker_run_id)
    .bind(fence.current_consumer_work_item_id)
    .bind(fence.current_consumer_worker_run_id)
    .bind(fence.current_consumer_lease_token)
    .bind(to_i64(
        fence.expected_consumer_attempt_epoch,
        "generator_consumer_attempt_epoch",
    )?)
    .bind(to_i64(
        fence.expected_consumer_checkpoint_version,
        "generator_consumer_checkpoint_version",
    )?)
    .bind(identity.stage.operation_id)
    .bind(identity.stage.stage_execution_id)
    .bind(identity.stage_run_unit_id)
    .bind(identity.stage.scope_snapshot_id)
    .bind(identity.organization_id)
    .bind(&source.canonical_result_sha256)
    .bind(subtask_count)
    .bind(&subtask_set_sha256)
    .bind(receipt_kind)
    .fetch_one(&mut **tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO investigation_generator_source_receipts(
               source_receipt_id,stable_request_id,task_plan_id,ledger_id,
               generator_pipeline_event_id,source_tool_call_id,source_provider_call_id,
               source_attempt_epoch,source_work_item_id,source_worker_run_id,
               current_consumer_work_item_id,current_consumer_worker_run_id,
               current_consumer_lease_token,current_consumer_attempt_epoch,
               current_consumer_checkpoint_version,
               operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
               organization_id,canonical_result_sha256,adopted_subtask_count,
               adopted_subtask_set_sha256,receipt_kind,receipt_sha256,status)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,
                  $18,$19,$20,$21,$22,$23,$24,$25,'applied')
           ON CONFLICT(stable_request_id) DO NOTHING"#,
    )
    .bind(source_receipt_id)
    .bind(stable_request_id)
    .bind(ledger.task_plan_id)
    .bind(ledger.ledger_id)
    .bind(ledger.generator_pipeline_event_id)
    .bind(source.source_tool_call_id)
    .bind(&source.source_provider_call_id)
    .bind(source.source_attempt_epoch)
    .bind(source.source_work_item_id)
    .bind(source.source_worker_run_id)
    .bind(fence.current_consumer_work_item_id)
    .bind(fence.current_consumer_worker_run_id)
    .bind(fence.current_consumer_lease_token)
    .bind(to_i64(
        fence.expected_consumer_attempt_epoch,
        "generator_consumer_attempt_epoch",
    )?)
    .bind(to_i64(
        fence.expected_consumer_checkpoint_version,
        "generator_consumer_checkpoint_version",
    )?)
    .bind(identity.stage.operation_id)
    .bind(identity.stage.stage_execution_id)
    .bind(identity.stage_run_unit_id)
    .bind(identity.stage.scope_snapshot_id)
    .bind(identity.organization_id)
    .bind(&source.canonical_result_sha256)
    .bind(subtask_count)
    .bind(&subtask_set_sha256)
    .bind(receipt_kind)
    .bind(&receipt_sha256)
    .execute(&mut **tx)
    .await?;
    let receipt = load_generator_source_receipt_on(tx, stable_request_id)
        .await?
        .ok_or(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
            "generator_source_receipt_missing",
        ))?;
    if receipt.source_receipt_id != source_receipt_id
        || receipt.stable_request_id != stable_request_id
        || receipt.task_plan_id != ledger.task_plan_id
        || receipt.ledger_id != ledger.ledger_id
        || receipt.generator_pipeline_event_id != ledger.generator_pipeline_event_id
        || receipt.source_tool_call_id != source.source_tool_call_id
        || receipt.source_provider_call_id != source.source_provider_call_id
        || receipt.source_attempt_epoch != source.source_attempt_epoch
        || receipt.source_work_item_id != source.source_work_item_id
        || receipt.source_worker_run_id != source.source_worker_run_id
        || receipt.current_consumer_work_item_id != fence.current_consumer_work_item_id
        || receipt.current_consumer_worker_run_id != fence.current_consumer_worker_run_id
        || receipt.current_consumer_lease_token != fence.current_consumer_lease_token
        || receipt.current_consumer_attempt_epoch != fence.expected_consumer_attempt_epoch as i64
        || receipt.current_consumer_checkpoint_version
            != fence.expected_consumer_checkpoint_version as i64
        || receipt.canonical_result_sha256 != source.canonical_result_sha256
        || receipt.adopted_subtask_count != subtask_count
        || receipt.adopted_subtask_set_sha256 != subtask_set_sha256
        || receipt.receipt_kind != receipt_kind
        || receipt.receipt_sha256 != receipt_sha256
    {
        return Err(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
            "generator_source_receipt_replay_mismatch",
        ));
    }
    Ok(())
}

#[cfg(test)]
const PLAN_ROW_COLUMNS: &str = r#"plan.task_plan_id,plan.stable_request_id,
    plan.run_request_id,plan.authority_id,plan.stage_team_plan_id,plan.operation_id,plan.stage_execution_id,
    plan.owning_stage_run_request_id,plan.stage_run_unit_id,plan.scope_snapshot_id,
    plan.organization_id,plan.subject_kind,plan.subject_id,
    plan.subject_fingerprint_sha256,plan.task_plan_version,plan.task_plan_sha256,
    plan.allowed_role_catalog,plan.cognitive_tool_envelope_sha256,plan.status,
    plan.subtask_count,plan.subtask_set_sha256,plan.row_version"#;

const PLAN_ROW_SELECT_BY_REQUEST: &str = r#"SELECT plan.task_plan_id,plan.stable_request_id,
    plan.run_request_id,plan.authority_id,plan.stage_team_plan_id,plan.operation_id,plan.stage_execution_id,
    plan.owning_stage_run_request_id,plan.stage_run_unit_id,plan.scope_snapshot_id,
    plan.organization_id,plan.subject_kind,plan.subject_id,
    plan.subject_fingerprint_sha256,plan.task_plan_version,plan.task_plan_sha256,
    plan.allowed_role_catalog,plan.cognitive_tool_envelope_sha256,plan.status,
    plan.subtask_count,plan.subtask_set_sha256,plan.row_version
    FROM investigation_pentagi_task_plans plan
    WHERE plan.stable_request_id=$1 AND plan.authority_id=$2 AND plan.operation_id=$3
      AND plan.stage_execution_id=$4 AND plan.owning_stage_run_request_id=$5
      AND plan.stage_run_unit_id=$6 AND plan.scope_snapshot_id=$7
      AND plan.organization_id=$8"#;

const PLAN_ROW_SELECT_BY_ID: &str = r#"SELECT plan.task_plan_id,plan.stable_request_id,
    plan.run_request_id,plan.authority_id,plan.stage_team_plan_id,plan.operation_id,plan.stage_execution_id,
    plan.owning_stage_run_request_id,plan.stage_run_unit_id,plan.scope_snapshot_id,
    plan.organization_id,plan.subject_kind,plan.subject_id,
    plan.subject_fingerprint_sha256,plan.task_plan_version,plan.task_plan_sha256,
    plan.allowed_role_catalog,plan.cognitive_tool_envelope_sha256,plan.status,
    plan.subtask_count,plan.subtask_set_sha256,plan.row_version
    FROM investigation_pentagi_task_plans plan
    WHERE plan.task_plan_id=$1 AND plan.authority_id=$2 AND plan.operation_id=$3
      AND plan.stage_execution_id=$4 AND plan.owning_stage_run_request_id=$5
      AND plan.stage_run_unit_id=$6 AND plan.scope_snapshot_id=$7
      AND plan.organization_id=$8"#;

const DISPATCH_ROW_SELECT_BY_REQUEST: &str = r#"SELECT dispatch.dispatch_receipt_id,
    dispatch.stable_request_id,dispatch.logical_dispatch_key_sha256,
    dispatch.task_plan_id,dispatch.subtask_id,dispatch.parent_dispatch_receipt_id,
    dispatch.dispatch_ordinal,dispatch.actor_kind,dispatch.stage_work_item_id,
    dispatch.stage_worker_request_id,dispatch.worker_run_id,dispatch.operation_id,
    dispatch.stage_execution_id,dispatch.stage_run_unit_id,dispatch.scope_snapshot_id,
    dispatch.organization_id,dispatch.transcript_request_id,
    dispatch.parent_actor_transcript_request_id,dispatch.parent_dispatch_tool_request_id,
    dispatch.snapshot_sha256,dispatch.receipt_sha256
    FROM pentagi_logical_dispatch_receipts dispatch
    JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=dispatch.task_plan_id
    WHERE dispatch.stable_request_id=$1 AND dispatch.operation_id=$2
      AND dispatch.stage_execution_id=$3 AND dispatch.stage_run_unit_id=$4
      AND dispatch.scope_snapshot_id=$5 AND dispatch.organization_id=$6
      AND plan.authority_id=$7 AND plan.owning_stage_run_request_id=$8"#;

const DISPATCH_ROW_SELECT_BY_ID: &str = r#"SELECT dispatch.dispatch_receipt_id,
    dispatch.stable_request_id,dispatch.logical_dispatch_key_sha256,
    dispatch.task_plan_id,dispatch.subtask_id,dispatch.parent_dispatch_receipt_id,
    dispatch.dispatch_ordinal,dispatch.actor_kind,dispatch.stage_work_item_id,
    dispatch.stage_worker_request_id,dispatch.worker_run_id,dispatch.operation_id,
    dispatch.stage_execution_id,dispatch.stage_run_unit_id,dispatch.scope_snapshot_id,
    dispatch.organization_id,dispatch.transcript_request_id,
    dispatch.parent_actor_transcript_request_id,dispatch.parent_dispatch_tool_request_id,
    dispatch.snapshot_sha256,dispatch.receipt_sha256
    FROM pentagi_logical_dispatch_receipts dispatch
    JOIN investigation_pentagi_task_plans plan ON plan.task_plan_id=dispatch.task_plan_id
    WHERE dispatch.dispatch_receipt_id=$1 AND dispatch.task_plan_id=$2
      AND dispatch.operation_id=$3 AND dispatch.stage_execution_id=$4
      AND dispatch.stage_run_unit_id=$5 AND dispatch.scope_snapshot_id=$6
      AND dispatch.organization_id=$7 AND plan.authority_id=$8
      AND plan.owning_stage_run_request_id=$9"#;

fn validate_stage_identity(
    identity: &InvestigationStageIdentity,
) -> UnifiedInvestigationRuntimeStoreResult<()> {
    validate_ids(&[
        identity.authority_id,
        identity.operation_id,
        identity.stage_execution_id,
        identity.scope_snapshot_id,
    ])?;
    validate_bounded(
        &identity.owning_stage_run_request_id,
        512,
        "owning_stage_run_request_id",
    )
}

fn validate_unit_identity(
    identity: &InvestigationUnitIdentity,
) -> UnifiedInvestigationRuntimeStoreResult<()> {
    validate_stage_identity(&identity.stage)?;
    validate_ids(&[identity.stage_run_unit_id, identity.organization_id])
}

fn validate_ids(ids: &[Uuid]) -> UnifiedInvestigationRuntimeStoreResult<()> {
    if ids.iter().any(Uuid::is_nil) {
        return Err(UnifiedInvestigationRuntimeStoreError::InvalidInput("uuid"));
    }
    Ok(())
}

fn validate_optional_id(
    id: Option<Uuid>,
    field: &'static str,
) -> UnifiedInvestigationRuntimeStoreResult<()> {
    if id.is_some_and(|value| value.is_nil()) {
        return Err(UnifiedInvestigationRuntimeStoreError::InvalidInput(field));
    }
    Ok(())
}

fn validate_hashes(hashes: &[&str]) -> UnifiedInvestigationRuntimeStoreResult<()> {
    if hashes.iter().any(|value| {
        value.len() != 71
            || !value.starts_with("sha256:")
            || !value[7..].bytes().all(|byte| byte.is_ascii_hexdigit())
    }) {
        return Err(UnifiedInvestigationRuntimeStoreError::InvalidInput(
            "sha256",
        ));
    }
    Ok(())
}

fn validate_bounded(
    value: &str,
    max: usize,
    field: &'static str,
) -> UnifiedInvestigationRuntimeStoreResult<()> {
    if value.trim().is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(UnifiedInvestigationRuntimeStoreError::InvalidInput(field));
    }
    Ok(())
}

fn validate_optional_bounded(
    value: Option<&str>,
    max: usize,
    field: &'static str,
) -> UnifiedInvestigationRuntimeStoreResult<()> {
    if let Some(value) = value {
        validate_bounded(value, max, field)?;
    }
    Ok(())
}

fn to_i64(value: u64, field: &'static str) -> UnifiedInvestigationRuntimeStoreResult<i64> {
    i64::try_from(value).map_err(|_| UnifiedInvestigationRuntimeStoreError::InvalidInput(field))
}

fn json_string_array_is_valid(value: &Value) -> bool {
    value.as_array().is_some_and(|items| {
        !items.is_empty()
            && items.iter().all(|item| {
                item.as_str()
                    .is_some_and(|text| !text.trim().is_empty() && text.len() <= 128)
            })
    })
}

fn validate_work_event_replay(
    existing: &WorkStateEventRow,
    input: &TransitionInvestigationWorkInput,
    expected: i64,
    epoch: i64,
) -> UnifiedInvestigationRuntimeStoreResult<()> {
    if existing.event_id != input.event_id
        || existing.work_id != input.work_id
        || existing.expected_head_version != expected
        || existing.event_ordinal != expected.saturating_add(1)
        || existing.from_state != input.from_state.as_str()
        || existing.to_state != input.to_state.as_str()
        || existing.observed_stop_epoch != epoch
        || existing.reason_code != input.reason_code
        || existing.event_sha256 != input.event_sha256
    {
        return Err(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
            "work_event_replay_mismatch",
        ));
    }
    Ok(())
}

fn validate_census_replay(
    existing: &PentagiDelegationCensusRow,
    input: &SealPentagiDelegationCensusInput,
) -> UnifiedInvestigationRuntimeStoreResult<()> {
    if existing.census_seal_id != input.census_seal_id
        || existing.task_plan_id != input.task_plan_id
        || existing.primary_dispatch_receipt_id != input.primary_dispatch_receipt_id
        || existing.primary_worker_run_id != input.primary_worker_run_id
        || existing.seal_sha256 != input.seal_sha256
    {
        return Err(UnifiedInvestigationRuntimeStoreError::IdentityConflict(
            "delegation_census_replay_mismatch",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_runtime_values_match_frozen_sql_contract() {
        assert_eq!(
            PentagiSubjectKind::AnalysisAttempt.as_str(),
            "analysis_attempt"
        );
        assert_eq!(PentagiActorKind::NestedWorker.as_str(), "nested_worker");
        assert_eq!(PentagiDispatchOutcome::UnknownHeld.as_str(), "unknown_held");
        assert_eq!(
            InvestigationWorkKind::PreparedAction.as_str(),
            "prepared_action"
        );
        assert_eq!(
            InvestigationWorkState::RecoveryRequired.as_str(),
            "recovery_required"
        );
        assert_eq!(
            InvestigationClosureDisposition::PassWithGaps.as_str(),
            "pass_with_gaps"
        );
    }

    #[test]
    fn role_catalog_and_hash_validation_fail_closed() {
        assert!(json_string_array_is_valid(&serde_json::json!([
            "primary",
            "researcher"
        ])));
        assert!(!json_string_array_is_valid(&serde_json::json!([])));
        assert!(validate_hashes(&[&format!("sha256:{}", "a".repeat(64))]).is_ok());
        assert!(validate_hashes(&["sha256:xyz"]).is_err());
    }

    #[test]
    fn every_select_shape_keeps_exact_identity_axes() {
        for query in [
            WORK_ROW_SELECT_BY_KEY,
            WORK_ROW_SELECT_BY_ID,
            PLAN_ROW_SELECT_BY_REQUEST,
            PLAN_ROW_SELECT_BY_ID,
            DISPATCH_ROW_SELECT_BY_REQUEST,
            DISPATCH_ROW_SELECT_BY_ID,
        ] {
            assert!(query.contains("operation_id"));
            assert!(query.contains("stage_execution_id"));
            assert!(query.contains("stage_run_unit_id"));
            assert!(query.contains("organization_id"));
        }
        assert!(WORK_ROW_COLUMNS.contains("owning_stage_run_request_id"));
        assert!(PLAN_ROW_COLUMNS.contains("scope_snapshot_id"));
    }
}
