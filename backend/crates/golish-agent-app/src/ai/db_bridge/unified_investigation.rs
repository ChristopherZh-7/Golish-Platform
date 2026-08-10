//! App-owned adapter for the unified Investigation persistence port.
//!
//! All SQL and replay/CAS behavior remains inside `golish-db`; this module only
//! translates the SQL-free agent-kit contract and preserves typed failures.

use std::sync::Arc;

use async_trait::async_trait;
use golish_agent_kit::db_traits::*;
use golish_core::investigation_main_read_session::{
    BindMainOrganizationReadSessionV1, MainOrganizationReadSessionV1,
};
use golish_core::investigation_run_closure::{
    InvestigationDelegationCensusV1, InvestigationExactSetCensusV1, InvestigationFuelClosureV1,
    InvestigationRunClosureDispositionV1, InvestigationRunClosureV1,
    InvestigationTerminalWorkCensusV1,
};
use golish_db::repo::investigation_main_sessions as main_sessions;
use golish_db::repo::unified_investigation_runtime as db;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone)]
pub struct PgUnifiedInvestigationRepository {
    pool: Arc<PgPool>,
    writer: db::PgUnifiedInvestigationRuntimeRepository,
}

#[derive(sqlx::FromRow)]
struct SealedMainReadSessionAuthorityRow {
    stage_run_unit_id: Uuid,
    organization_id: Uuid,
    snapshot_id: Uuid,
    snapshot_sha256: String,
    context_item_count: i64,
    context_item_set_sha256: String,
    methodology_hit_count: i64,
    methodology_result_set_sha256: String,
    omission_count: i64,
    omission_set_sha256: String,
    main_read_session_id: Uuid,
    context_chain_id: Uuid,
    transcript_partition_id: Uuid,
}

impl PgUnifiedInvestigationRepository {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self {
            writer: db::PgUnifiedInvestigationRuntimeRepository::new(pool.clone()),
            pool,
        }
    }

    async fn ensure_stage_authority(
        &self,
        identity: &UnifiedInvestigationStageIdentity,
    ) -> UnifiedInvestigationRepoResult<()> {
        main_sessions::register_stage_authority(
            &self.pool,
            &main_sessions::RegisterInvestigationStageAuthority {
                authority_id: identity.authority_id,
                operation_id: identity.operation_id,
                stage_execution_id: identity.stage_execution_id,
                owning_stage_run_request_id: identity.owning_stage_run_request_id.clone(),
                scope_snapshot_id: identity.scope_snapshot_id,
            },
        )
        .await
        .map(|_| ())
        .map_err(map_main_session_error)
    }
}

fn map_main_session_error(
    error: main_sessions::InvestigationMainSessionStoreError,
) -> UnifiedInvestigationRepositoryError {
    match error {
        main_sessions::InvestigationMainSessionStoreError::InvalidInput(detail) => {
            UnifiedInvestigationRepositoryError::InvalidRequest {
                detail: detail.to_string(),
            }
        }
        main_sessions::InvestigationMainSessionStoreError::IdentityConflict(detail) => {
            UnifiedInvestigationRepositoryError::AuthorityMismatch {
                detail: detail.to_string(),
            }
        }
        main_sessions::InvestigationMainSessionStoreError::CasConflict(detail) => {
            UnifiedInvestigationRepositoryError::Conflict {
                detail: detail.to_string(),
            }
        }
        main_sessions::InvestigationMainSessionStoreError::Sqlx(sqlx::Error::RowNotFound) => {
            UnifiedInvestigationRepositoryError::NotFound {
                detail: "unified investigation main-session row not found".to_string(),
            }
        }
        main_sessions::InvestigationMainSessionStoreError::Sqlx(error) => {
            UnifiedInvestigationRepositoryError::Infrastructure {
                detail: error.to_string(),
            }
        }
    }
}

fn map_storage_error(
    error: db::UnifiedInvestigationRuntimeStoreError,
) -> UnifiedInvestigationRepositoryError {
    match error {
        db::UnifiedInvestigationRuntimeStoreError::InvalidInput(detail) => {
            UnifiedInvestigationRepositoryError::InvalidRequest {
                detail: detail.to_string(),
            }
        }
        db::UnifiedInvestigationRuntimeStoreError::IdentityConflict(detail) => {
            UnifiedInvestigationRepositoryError::AuthorityMismatch {
                detail: detail.to_string(),
            }
        }
        db::UnifiedInvestigationRuntimeStoreError::CasConflict(detail) => {
            UnifiedInvestigationRepositoryError::Conflict {
                detail: detail.to_string(),
            }
        }
        db::UnifiedInvestigationRuntimeStoreError::Sqlx(sqlx::Error::RowNotFound) => {
            UnifiedInvestigationRepositoryError::NotFound {
                detail: "unified investigation row not found".to_string(),
            }
        }
        db::UnifiedInvestigationRuntimeStoreError::Sqlx(error) => {
            let detail = error.to_string();
            if detail.contains("CONFLICT")
                || detail.contains("CAS")
                || detail.contains("STALE")
                || detail.contains("FENCE")
                || detail.contains("REPLAY")
            {
                UnifiedInvestigationRepositoryError::Conflict { detail }
            } else if detail.contains("IDENTITY")
                || detail.contains("AUTHORITY")
                || detail.contains("OWNERSHIP")
                || detail.contains("SCOPE")
            {
                UnifiedInvestigationRepositoryError::AuthorityMismatch { detail }
            } else if detail.contains("INVALID")
                || detail.contains("NOT_OPEN")
                || detail.contains("ADMISSION_CLOSED")
                || detail.contains("INVESTIGATION_CLOSURE_")
            {
                UnifiedInvestigationRepositoryError::InvalidRequest { detail }
            } else {
                UnifiedInvestigationRepositoryError::Infrastructure { detail }
            }
        }
    }
}

fn stage_identity(identity: &UnifiedInvestigationStageIdentity) -> db::InvestigationStageIdentity {
    db::InvestigationStageIdentity {
        authority_id: identity.authority_id,
        operation_id: identity.operation_id,
        stage_execution_id: identity.stage_execution_id,
        owning_stage_run_request_id: identity.owning_stage_run_request_id.clone(),
        scope_snapshot_id: identity.scope_snapshot_id,
    }
}

fn unit_identity(identity: &UnifiedInvestigationUnitIdentity) -> db::InvestigationUnitIdentity {
    db::InvestigationUnitIdentity {
        stage: stage_identity(&identity.stage),
        stage_run_unit_id: identity.stage_run_unit_id,
        organization_id: identity.organization_id,
    }
}

fn work_kind(kind: UnifiedInvestigationWorkKind) -> db::InvestigationWorkKind {
    match kind {
        UnifiedInvestigationWorkKind::Analysis => db::InvestigationWorkKind::Analysis,
        UnifiedInvestigationWorkKind::ReadSession => db::InvestigationWorkKind::ReadSession,
        UnifiedInvestigationWorkKind::Query => db::InvestigationWorkKind::Query,
        UnifiedInvestigationWorkKind::Enrichment => db::InvestigationWorkKind::Enrichment,
        UnifiedInvestigationWorkKind::VerificationTask => {
            db::InvestigationWorkKind::VerificationTask
        }
        UnifiedInvestigationWorkKind::PentagiSubtask => db::InvestigationWorkKind::PentagiSubtask,
        UnifiedInvestigationWorkKind::WorkerRequest => db::InvestigationWorkKind::WorkerRequest,
        UnifiedInvestigationWorkKind::Campaign => db::InvestigationWorkKind::Campaign,
        UnifiedInvestigationWorkKind::PreparedAction => db::InvestigationWorkKind::PreparedAction,
        UnifiedInvestigationWorkKind::ActionExecution => db::InvestigationWorkKind::ActionExecution,
        UnifiedInvestigationWorkKind::FactDelta => db::InvestigationWorkKind::FactDelta,
        UnifiedInvestigationWorkKind::Consolidation => db::InvestigationWorkKind::Consolidation,
    }
}

fn work_state(state: UnifiedInvestigationWorkState) -> db::InvestigationWorkState {
    match state {
        UnifiedInvestigationWorkState::Queued => db::InvestigationWorkState::Queued,
        UnifiedInvestigationWorkState::Running => db::InvestigationWorkState::Running,
        UnifiedInvestigationWorkState::WaitingAuthorization => {
            db::InvestigationWorkState::WaitingAuthorization
        }
        UnifiedInvestigationWorkState::Unknown => db::InvestigationWorkState::Unknown,
        UnifiedInvestigationWorkState::StopPending => db::InvestigationWorkState::StopPending,
        UnifiedInvestigationWorkState::Draining => db::InvestigationWorkState::Draining,
        UnifiedInvestigationWorkState::Completed => db::InvestigationWorkState::Completed,
        UnifiedInvestigationWorkState::Cancelled => db::InvestigationWorkState::Cancelled,
        UnifiedInvestigationWorkState::Blocked => db::InvestigationWorkState::Blocked,
        UnifiedInvestigationWorkState::Residual => db::InvestigationWorkState::Residual,
        UnifiedInvestigationWorkState::RecoveryRequired => {
            db::InvestigationWorkState::RecoveryRequired
        }
        UnifiedInvestigationWorkState::FixedPoint => db::InvestigationWorkState::FixedPoint,
        UnifiedInvestigationWorkState::Superseded => db::InvestigationWorkState::Superseded,
    }
}

fn subject_kind(kind: UnifiedInvestigationSubjectKind) -> db::PentagiSubjectKind {
    match kind {
        UnifiedInvestigationSubjectKind::AnalysisAttempt => db::PentagiSubjectKind::AnalysisAttempt,
        UnifiedInvestigationSubjectKind::VerificationTask => {
            db::PentagiSubjectKind::VerificationTask
        }
    }
}

fn actor_kind(kind: UnifiedInvestigationActorKind) -> db::PentagiActorKind {
    match kind {
        UnifiedInvestigationActorKind::Primary => db::PentagiActorKind::Primary,
        UnifiedInvestigationActorKind::Worker => db::PentagiActorKind::Worker,
        UnifiedInvestigationActorKind::NestedWorker => db::PentagiActorKind::NestedWorker,
    }
}

pub(super) fn dispatch_outcome(
    outcome: UnifiedInvestigationDispatchOutcome,
) -> db::PentagiDispatchOutcome {
    match outcome {
        UnifiedInvestigationDispatchOutcome::Completed => db::PentagiDispatchOutcome::Completed,
        UnifiedInvestigationDispatchOutcome::Blocked => db::PentagiDispatchOutcome::Blocked,
        UnifiedInvestigationDispatchOutcome::Residual => db::PentagiDispatchOutcome::Residual,
        UnifiedInvestigationDispatchOutcome::RecoveryRequired => {
            db::PentagiDispatchOutcome::RecoveryRequired
        }
        UnifiedInvestigationDispatchOutcome::UnknownHeld => db::PentagiDispatchOutcome::UnknownHeld,
    }
}

fn pipeline_event_kind(
    kind: UnifiedInvestigationPipelineEventKind,
) -> db::PentagiPipelineEventKind {
    match kind {
        UnifiedInvestigationPipelineEventKind::GeneratorSealed => {
            db::PentagiPipelineEventKind::GeneratorSealed
        }
        UnifiedInvestigationPipelineEventKind::RefinerPatch => {
            db::PentagiPipelineEventKind::RefinerPatch
        }
        UnifiedInvestigationPipelineEventKind::ReflectorAttempt => {
            db::PentagiPipelineEventKind::ReflectorAttempt
        }
        UnifiedInvestigationPipelineEventKind::ResultBarrier => {
            db::PentagiPipelineEventKind::ResultBarrier
        }
        UnifiedInvestigationPipelineEventKind::PrimarySynthesis => {
            db::PentagiPipelineEventKind::PrimarySynthesis
        }
    }
}

#[allow(dead_code)]
fn closure_disposition(
    disposition: UnifiedInvestigationClosureDisposition,
) -> db::InvestigationClosureDisposition {
    match disposition {
        UnifiedInvestigationClosureDisposition::Pass => db::InvestigationClosureDisposition::Pass,
        UnifiedInvestigationClosureDisposition::PassWithGaps => {
            db::InvestigationClosureDisposition::PassWithGaps
        }
        UnifiedInvestigationClosureDisposition::Stopped => {
            db::InvestigationClosureDisposition::Stopped
        }
    }
}

fn run_head(row: db::InvestigationRunHeadRow) -> UnifiedInvestigationRunHead {
    UnifiedInvestigationRunHead {
        authority_id: row.authority_id,
        stable_start_request_id: row.stable_start_request_id,
        operation_id: row.operation_id,
        stage_execution_id: row.stage_execution_id,
        owning_stage_run_request_id: row.owning_stage_run_request_id,
        scope_snapshot_id: row.scope_snapshot_id,
        run_state: row.run_state,
        admission_open: row.admission_open,
        stop_epoch: row.stop_epoch,
        change_seq: row.change_seq,
        head_version: row.head_version,
        head_sha256: row.head_sha256,
        latest_event_id: row.latest_event_id,
    }
}

fn work(row: db::InvestigationRunWorkRow) -> UnifiedInvestigationWork {
    UnifiedInvestigationWork {
        work_id: row.work_id,
        stable_work_key_sha256: row.stable_work_key_sha256,
        authority_id: row.authority_id,
        operation_id: row.operation_id,
        stage_execution_id: row.stage_execution_id,
        owning_stage_run_request_id: row.owning_stage_run_request_id,
        stage_run_unit_id: row.stage_run_unit_id,
        scope_snapshot_id: row.scope_snapshot_id,
        organization_id: row.organization_id,
        work_kind: row.work_kind,
        external_identity_sha256: row.external_identity_sha256,
        current_state: row.current_state,
        observed_stop_epoch: row.observed_stop_epoch,
        head_version: row.head_version,
        latest_event_id: row.latest_event_id,
    }
}

fn task_request(row: db::PentagiTaskRunRequestRow) -> UnifiedInvestigationTaskRequest {
    UnifiedInvestigationTaskRequest {
        run_request_id: row.run_request_id,
        stable_request_id: row.stable_request_id,
        task_plan_id: row.task_plan_id,
        authority_id: row.authority_id,
        operation_id: row.operation_id,
        stage_execution_id: row.stage_execution_id,
        owning_stage_run_request_id: row.owning_stage_run_request_id,
        stage_run_unit_id: row.stage_run_unit_id,
        organization_id: row.organization_id,
        subject_kind: row.subject_kind,
        subject_id: row.subject_id,
        subject_fingerprint_sha256: row.subject_fingerprint_sha256,
        request_sha256: row.request_sha256,
    }
}

fn task_plan(row: db::PentagiTaskPlanRow) -> UnifiedInvestigationTaskPlan {
    UnifiedInvestigationTaskPlan {
        task_plan_id: row.task_plan_id,
        stable_request_id: row.stable_request_id,
        run_request_id: row.run_request_id,
        authority_id: row.authority_id,
        stage_team_plan_id: row.stage_team_plan_id,
        operation_id: row.operation_id,
        stage_execution_id: row.stage_execution_id,
        owning_stage_run_request_id: row.owning_stage_run_request_id,
        stage_run_unit_id: row.stage_run_unit_id,
        scope_snapshot_id: row.scope_snapshot_id,
        organization_id: row.organization_id,
        subject_kind: row.subject_kind,
        subject_id: row.subject_id,
        subject_fingerprint_sha256: row.subject_fingerprint_sha256,
        task_plan_version: row.task_plan_version,
        task_plan_sha256: row.task_plan_sha256,
        allowed_role_catalog: row.allowed_role_catalog,
        cognitive_tool_envelope_sha256: row.cognitive_tool_envelope_sha256,
        status: row.status,
        subtask_count: row.subtask_count,
        subtask_set_sha256: row.subtask_set_sha256,
        row_version: row.row_version,
    }
}

fn subtask(row: db::PentagiSubtaskRow) -> UnifiedInvestigationSubtask {
    UnifiedInvestigationSubtask {
        subtask_id: row.subtask_id,
        task_plan_id: row.task_plan_id,
        authority_id: row.authority_id,
        operation_id: row.operation_id,
        stage_execution_id: row.stage_execution_id,
        stage_run_unit_id: row.stage_run_unit_id,
        organization_id: row.organization_id,
        subtask_ordinal: row.subtask_ordinal,
        label: row.label,
        runnable: row.runnable,
        input_manifest_sha256: row.input_manifest_sha256,
        expected_output_schema: row.expected_output_schema,
        member_sha256: row.member_sha256,
    }
}

pub(super) fn dispatch(row: db::PentagiLogicalDispatchRow) -> UnifiedInvestigationDispatch {
    UnifiedInvestigationDispatch {
        dispatch_receipt_id: row.dispatch_receipt_id,
        stable_request_id: row.stable_request_id,
        logical_dispatch_key_sha256: row.logical_dispatch_key_sha256,
        task_plan_id: row.task_plan_id,
        subtask_id: row.subtask_id,
        parent_dispatch_receipt_id: row.parent_dispatch_receipt_id,
        dispatch_ordinal: row.dispatch_ordinal,
        actor_kind: row.actor_kind,
        stage_work_item_id: row.stage_work_item_id,
        stage_worker_request_id: row.stage_worker_request_id,
        worker_run_id: row.worker_run_id,
        operation_id: row.operation_id,
        stage_execution_id: row.stage_execution_id,
        stage_run_unit_id: row.stage_run_unit_id,
        scope_snapshot_id: row.scope_snapshot_id,
        organization_id: row.organization_id,
        transcript_request_id: row.transcript_request_id,
        parent_actor_transcript_request_id: row.parent_actor_transcript_request_id,
        parent_dispatch_tool_request_id: row.parent_dispatch_tool_request_id,
        snapshot_sha256: row.snapshot_sha256,
        receipt_sha256: row.receipt_sha256,
    }
}

pub(super) fn dispatch_attempt(
    row: db::PentagiDispatchAttemptRow,
) -> UnifiedInvestigationDispatchAttempt {
    UnifiedInvestigationDispatchAttempt {
        dispatch_attempt_id: row.dispatch_attempt_id,
        stable_request_id: row.stable_request_id,
        dispatch_receipt_id: row.dispatch_receipt_id,
        attempt_epoch: row.attempt_epoch,
        lease_token: row.lease_token,
        fence_sha256: row.fence_sha256,
        outcome: row.outcome,
        result_sha256: row.result_sha256,
    }
}

fn pipeline_event(row: db::PentagiPipelineEventRow) -> UnifiedInvestigationPipelineEvent {
    UnifiedInvestigationPipelineEvent {
        pipeline_event_id: row.pipeline_event_id,
        stable_request_id: row.stable_request_id,
        task_plan_id: row.task_plan_id,
        subtask_id: row.subtask_id,
        event_ordinal: row.event_ordinal,
        event_kind: row.event_kind,
        actor_worker_run_id: row.actor_worker_run_id,
        parent_dispatch_receipt_id: row.parent_dispatch_receipt_id,
        event_sha256: row.event_sha256,
    }
}

fn refiner_plan_ledger(
    row: db::InvestigationRefinerPlanLedgerRow,
) -> UnifiedInvestigationRefinerPlanLedger {
    UnifiedInvestigationRefinerPlanLedger {
        ledger_id: row.ledger_id,
        stable_request_id: row.stable_request_id,
        task_plan_id: row.task_plan_id,
        generator_pipeline_event_id: row.generator_pipeline_event_id,
        generator_manifest: row.generator_manifest,
        generator_manifest_sha256: row.generator_manifest_sha256,
        generator_subtask_count: row.generator_subtask_count,
        generator_subtask_set_sha256: row.generator_subtask_set_sha256,
        ledger_sha256: row.ledger_sha256,
    }
}

fn refiner_plan_patch(
    row: db::InvestigationRefinerPlanPatchRow,
) -> UnifiedInvestigationRefinerPlanPatch {
    UnifiedInvestigationRefinerPlanPatch {
        patch_id: row.patch_id,
        stable_request_id: row.stable_request_id,
        ledger_id: row.ledger_id,
        task_plan_id: row.task_plan_id,
        patch_ordinal: row.patch_ordinal,
        refiner_pipeline_event_id: row.refiner_pipeline_event_id,
        expected_previous_state_sha256: row.expected_previous_state_sha256,
        remaining_plan_payload: row.remaining_plan_payload,
        remaining_plan_payload_sha256: row.remaining_plan_payload_sha256,
        active_realized_subtask_count: row.active_realized_subtask_count,
        active_realized_subtask_set_sha256: row.active_realized_subtask_set_sha256,
        patch_sha256: row.patch_sha256,
    }
}

fn refiner_plan_ledger_seal(
    row: db::InvestigationRefinerPlanLedgerSealRow,
) -> UnifiedInvestigationRefinerPlanLedgerSeal {
    UnifiedInvestigationRefinerPlanLedgerSeal {
        seal_id: row.seal_id,
        stable_request_id: row.stable_request_id,
        ledger_id: row.ledger_id,
        task_plan_id: row.task_plan_id,
        result_barrier_pipeline_event_id: row.result_barrier_pipeline_event_id,
        patch_count: row.patch_count,
        patch_set_sha256: row.patch_set_sha256,
        final_patch_id: row.final_patch_id,
        final_patch_sha256: row.final_patch_sha256,
        final_active_realized_subtask_count: row.final_active_realized_subtask_count,
        final_active_realized_subtask_set_sha256: row.final_active_realized_subtask_set_sha256,
        generator_subtask_count: row.generator_subtask_count,
        generator_subtask_set_sha256: row.generator_subtask_set_sha256,
        seal_sha256: row.seal_sha256,
    }
}

fn census(row: db::PentagiDelegationCensusRow) -> UnifiedInvestigationDelegationCensus {
    UnifiedInvestigationDelegationCensus {
        census_seal_id: row.census_seal_id,
        stable_request_id: row.stable_request_id,
        task_plan_id: row.task_plan_id,
        primary_dispatch_receipt_id: row.primary_dispatch_receipt_id,
        primary_worker_run_id: row.primary_worker_run_id,
        runnable_subtask_count: row.runnable_subtask_count,
        runnable_subtask_set_sha256: row.runnable_subtask_set_sha256,
        dispatch_count: row.dispatch_count,
        dispatch_set_sha256: row.dispatch_set_sha256,
        pipeline_event_count: row.pipeline_event_count,
        pipeline_event_set_sha256: row.pipeline_event_set_sha256,
        seal_sha256: row.seal_sha256,
    }
}

fn stop_intent(row: db::InvestigationStopIntentRow) -> UnifiedInvestigationStopIntent {
    UnifiedInvestigationStopIntent {
        stop_intent_id: row.stop_intent_id,
        idempotency_key: row.idempotency_key,
        authority_id: row.authority_id,
        operation_id: row.operation_id,
        stage_execution_id: row.stage_execution_id,
        owning_stage_run_request_id: row.owning_stage_run_request_id,
        expected_run_head_sha256: row.expected_run_head_sha256,
        expected_change_seq: row.expected_change_seq,
        stop_epoch: row.stop_epoch,
        frozen_work_count: row.frozen_work_count,
        frozen_work_set_sha256: row.frozen_work_set_sha256,
        receipt_sha256: row.receipt_sha256,
    }
}

fn closure_count(value: i64, field: &'static str) -> UnifiedInvestigationRepoResult<u32> {
    u32::try_from(value).map_err(|_| UnifiedInvestigationRepositoryError::InvalidRequest {
        detail: format!("database closure {field} is outside the v1 u32 range"),
    })
}

fn closure_version(value: i64, field: &'static str) -> UnifiedInvestigationRepoResult<u64> {
    u64::try_from(value).map_err(|_| UnifiedInvestigationRepositoryError::InvalidRequest {
        detail: format!("database closure {field} is negative"),
    })
}

fn exact_set(
    count: i64,
    hash: String,
    field: &'static str,
) -> UnifiedInvestigationRepoResult<InvestigationExactSetCensusV1> {
    Ok(InvestigationExactSetCensusV1 {
        member_count: closure_count(count, field)?,
        member_set_sha256: hash,
    })
}

fn terminal_set(
    total: i64,
    terminal: i64,
    cancelled: i64,
    recovery: i64,
    hash: String,
    field: &'static str,
) -> UnifiedInvestigationRepoResult<InvestigationTerminalWorkCensusV1> {
    Ok(InvestigationTerminalWorkCensusV1 {
        total_count: closure_count(total, field)?,
        terminal_count: closure_count(terminal, field)?,
        cancelled_before_start_count: closure_count(cancelled, field)?,
        recovery_required_count: closure_count(recovery, field)?,
        member_set_sha256: hash,
    })
}

fn closure(
    row: db::InvestigationRunClosureRow,
) -> UnifiedInvestigationRepoResult<InvestigationRunClosureV1> {
    let disposition = match row.disposition.as_str() {
        "pass" => InvestigationRunClosureDispositionV1::Pass,
        "pass_with_gaps" => InvestigationRunClosureDispositionV1::PassWithGaps,
        _ => {
            return Err(UnifiedInvestigationRepositoryError::InvalidRequest {
                detail: "database closure has an unsupported disposition".to_string(),
            });
        }
    };
    let residual_hash = row.residual_member_set_sha256.clone();
    let closure = InvestigationRunClosureV1 {
        closure_id: row.closure_id,
        operation_id: row.operation_id,
        stage_execution_id: row.stage_execution_id,
        owning_stage_run_request_id: row.owning_stage_run_request_id,
        scope_snapshot_id: row.scope_snapshot_id,
        run_state_head_version: closure_version(
            row.run_state_head_version,
            "run_state_head_version",
        )?,
        stop_epoch: closure_version(row.stop_epoch, "stop_epoch")?,
        snapshot_set: exact_set(
            row.snapshot_member_count,
            row.snapshot_member_set_sha256,
            "snapshot_member_count",
        )?,
        main_read_session_set: exact_set(
            row.main_read_session_member_count,
            row.main_read_session_member_set_sha256,
            "main_read_session_member_count",
        )?,
        generation_set: exact_set(
            row.generation_member_count,
            row.generation_member_set_sha256,
            "generation_member_count",
        )?,
        admission_set: exact_set(
            row.admission_member_count,
            row.admission_member_set_sha256,
            "admission_member_count",
        )?,
        verification_task_set: exact_set(
            row.verification_task_member_count,
            row.verification_task_member_set_sha256,
            "verification_task_member_count",
        )?,
        objective_assignment_set: exact_set(
            row.objective_assignment_member_count,
            row.objective_assignment_member_set_sha256,
            "objective_assignment_member_count",
        )?,
        objective_outcome_set: exact_set(
            row.objective_outcome_member_count,
            row.objective_outcome_member_set_sha256,
            "objective_outcome_member_count",
        )?,
        work: terminal_set(
            row.work_total_count,
            row.work_terminal_count,
            row.work_cancelled_before_start_count,
            row.work_recovery_required_count,
            row.work_member_set_sha256,
            "work",
        )?,
        campaigns: terminal_set(
            row.campaign_total_count,
            row.campaign_terminal_count,
            row.campaign_cancelled_before_start_count,
            row.campaign_recovery_required_count,
            row.campaign_member_set_sha256,
            "campaigns",
        )?,
        prepared_actions: terminal_set(
            row.prepared_action_total_count,
            row.prepared_action_terminal_count,
            row.prepared_action_cancelled_before_start_count,
            row.prepared_action_recovery_required_count,
            row.prepared_action_member_set_sha256,
            "prepared_actions",
        )?,
        fact_deltas: terminal_set(
            row.fact_delta_total_count,
            row.fact_delta_terminal_count,
            row.fact_delta_cancelled_before_start_count,
            row.fact_delta_recovery_required_count,
            row.fact_delta_member_set_sha256,
            "fact_deltas",
        )?,
        delegation: InvestigationDelegationCensusV1 {
            task_count: closure_count(row.delegation_task_count, "delegation_task_count")?,
            primary_count: closure_count(row.delegation_primary_count, "delegation_primary_count")?,
            runnable_subtask_count: closure_count(
                row.delegation_runnable_subtask_count,
                "delegation_runnable_subtask_count",
            )?,
            independently_dispatched_subtask_count: closure_count(
                row.delegation_independently_dispatched_subtask_count,
                "delegation_independently_dispatched_subtask_count",
            )?,
            logical_dispatch_count: closure_count(
                row.delegation_logical_dispatch_count,
                "delegation_logical_dispatch_count",
            )?,
            unique_logical_dispatch_count: closure_count(
                row.delegation_unique_logical_dispatch_count,
                "delegation_unique_logical_dispatch_count",
            )?,
            sealed_task_census_count: closure_count(
                row.delegation_sealed_task_census_count,
                "delegation_sealed_task_census_count",
            )?,
            member_set_sha256: row.delegation_member_set_sha256,
        },
        fuel: InvestigationFuelClosureV1 {
            reservation_count: closure_count(row.fuel_reservation_count, "fuel_reservation_count")?,
            consumed_count: closure_count(row.fuel_consumed_count, "fuel_consumed_count")?,
            refunded_count: closure_count(row.fuel_refunded_count, "fuel_refunded_count")?,
            unknown_held_count: closure_count(
                row.fuel_unknown_held_count,
                "fuel_unknown_held_count",
            )?,
            open_count: closure_count(row.fuel_open_count, "fuel_open_count")?,
            semantic_cycle_count: closure_count(
                row.fuel_semantic_cycle_count,
                "fuel_semantic_cycle_count",
            )?,
            reservation_set_sha256: row.fuel_reservation_set_sha256,
            semantic_cycle_set_sha256: row.fuel_semantic_cycle_set_sha256,
        },
        fixed_point_receipt_id: row.fixed_point_receipt_id,
        fixed_point_receipt_sha256: row.fixed_point_receipt_sha256,
        residual_set: exact_set(
            row.residual_member_count,
            row.residual_member_set_sha256,
            "residual_member_count",
        )?,
        disposition,
        contract_version: row.contract_version,
    };
    let valid_residual_hash = residual_hash.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    });
    if !valid_residual_hash {
        return Err(UnifiedInvestigationRepositoryError::InvalidRequest {
            detail: "database closure residual exact-set hash is invalid".to_string(),
        });
    }
    closure.validate().map_err(
        |error| UnifiedInvestigationRepositoryError::InvalidRequest {
            detail: format!("database closure failed v1 validation: {error}"),
        },
    )?;
    Ok(closure)
}

#[async_trait]
impl UnifiedInvestigationRepository for PgUnifiedInvestigationRepository {
    async fn start_run(
        &self,
        request: StartUnifiedInvestigationRun,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationRunHead> {
        self.ensure_stage_authority(&request.identity).await?;
        self.writer
            .start_run(&db::StartInvestigationRunInput {
                identity: stage_identity(&request.identity),
                stable_start_request_id: request.stable_start_request_id,
                initial_change_seq: request.initial_change_seq,
            })
            .await
            .map(run_head)
            .map_err(map_storage_error)
    }

    async fn load_run_head(
        &self,
        identity: UnifiedInvestigationStageIdentity,
    ) -> UnifiedInvestigationRepoResult<Option<UnifiedInvestigationRunHead>> {
        self.writer
            .load_run_head(&stage_identity(&identity))
            .await
            .map(|row| row.map(run_head))
            .map_err(map_storage_error)
    }

    async fn register_work(
        &self,
        request: RegisterUnifiedInvestigationWork,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationWork> {
        self.writer
            .register_work(&db::RegisterInvestigationWorkInput {
                identity: unit_identity(&request.identity),
                work_id: request.work_id,
                stable_work_key_sha256: request.stable_work_key_sha256,
                work_kind: work_kind(request.work_kind),
                external_identity_sha256: request.external_identity_sha256,
                initial_state: work_state(request.initial_state),
                observed_stop_epoch: request.observed_stop_epoch,
            })
            .await
            .map(work)
            .map_err(map_storage_error)
    }

    async fn transition_work(
        &self,
        request: TransitionUnifiedInvestigationWork,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationWork> {
        self.writer
            .transition_work(&db::TransitionInvestigationWorkInput {
                identity: unit_identity(&request.identity),
                work_id: request.work_id,
                event_id: request.event_id,
                stable_request_id: request.stable_request_id,
                expected_head_version: request.expected_head_version,
                from_state: work_state(request.from_state),
                to_state: work_state(request.to_state),
                observed_stop_epoch: request.observed_stop_epoch,
                reason_code: request.reason_code,
                event_sha256: request.event_sha256,
            })
            .await
            .map(work)
            .map_err(map_storage_error)
    }

    async fn request_task(
        &self,
        request: RequestUnifiedInvestigationTask,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationTaskRequest> {
        self.writer
            .insert_pentagi_run_request(&db::InsertPentagiTaskRunRequestInput {
                identity: unit_identity(&request.identity),
                run_request_id: request.run_request_id,
                stable_request_id: request.stable_request_id,
                subject_kind: subject_kind(request.subject_kind),
                subject_id: request.subject_id,
                subject_fingerprint_sha256: request.subject_fingerprint_sha256,
                request_sha256: request.request_sha256,
            })
            .await
            .map(task_request)
            .map_err(map_storage_error)
    }

    async fn begin_task_plan(
        &self,
        request: BeginUnifiedInvestigationTaskPlan,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationTaskPlan> {
        self.writer
            .begin_pentagi_plan(&db::BeginPentagiTaskPlanInput {
                identity: unit_identity(&request.identity),
                task_plan_id: request.task_plan_id,
                stable_request_id: request.stable_request_id,
                run_request_id: request.run_request_id,
                stage_team_plan_id: request.stage_team_plan_id,
                subject_kind: subject_kind(request.subject_kind),
                subject_id: request.subject_id,
                subject_fingerprint_sha256: request.subject_fingerprint_sha256,
                task_plan_version: request.task_plan_version,
                task_plan_sha256: request.task_plan_sha256,
                allowed_role_catalog: request.allowed_role_catalog,
                cognitive_tool_envelope_sha256: request.cognitive_tool_envelope_sha256,
            })
            .await
            .map(task_plan)
            .map_err(map_storage_error)
    }

    async fn insert_subtask(
        &self,
        request: InsertUnifiedInvestigationSubtask,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationSubtask> {
        self.writer
            .insert_pentagi_subtask(&db::InsertPentagiSubtaskInput {
                identity: unit_identity(&request.identity),
                task_plan_id: request.task_plan_id,
                subtask_id: request.subtask_id,
                subtask_ordinal: request.subtask_ordinal,
                label: request.label,
                runnable: request.runnable,
                input_manifest_sha256: request.input_manifest_sha256,
                expected_output_schema: request.expected_output_schema,
                member_sha256: request.member_sha256,
            })
            .await
            .map(subtask)
            .map_err(map_storage_error)
    }

    async fn insert_dispatch(
        &self,
        request: InsertUnifiedInvestigationDispatch,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationDispatch> {
        self.writer
            .insert_logical_dispatch(&db::InsertPentagiLogicalDispatchInput {
                identity: unit_identity(&request.identity),
                dispatch_receipt_id: request.dispatch_receipt_id,
                stable_request_id: request.stable_request_id,
                logical_dispatch_key_sha256: request.logical_dispatch_key_sha256,
                task_plan_id: request.task_plan_id,
                subtask_id: request.subtask_id,
                parent_dispatch_receipt_id: request.parent_dispatch_receipt_id,
                dispatch_ordinal: request.dispatch_ordinal,
                actor_kind: actor_kind(request.actor_kind),
                stage_work_item_id: request.stage_work_item_id,
                stage_worker_request_id: request.stage_worker_request_id,
                worker_run_id: request.worker_run_id,
                transcript_request_id: request.transcript_request_id,
                parent_actor_transcript_request_id: request.parent_actor_transcript_request_id,
                parent_dispatch_tool_request_id: request.parent_dispatch_tool_request_id,
                snapshot_sha256: request.snapshot_sha256,
                receipt_sha256: request.receipt_sha256,
            })
            .await
            .map(dispatch)
            .map_err(map_storage_error)
    }

    async fn insert_dispatch_attempt(
        &self,
        request: InsertUnifiedInvestigationDispatchAttempt,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationDispatchAttempt> {
        self.writer
            .insert_dispatch_attempt(&db::InsertPentagiDispatchAttemptInput {
                identity: unit_identity(&request.identity),
                task_plan_id: request.task_plan_id,
                dispatch_attempt_id: request.dispatch_attempt_id,
                stable_request_id: request.stable_request_id,
                dispatch_receipt_id: request.dispatch_receipt_id,
                attempt_epoch: request.attempt_epoch,
                lease_token: request.lease_token,
                fence_sha256: request.fence_sha256,
                outcome: dispatch_outcome(request.outcome),
                result_sha256: request.result_sha256,
            })
            .await
            .map(dispatch_attempt)
            .map_err(map_storage_error)
    }

    async fn insert_pipeline_event(
        &self,
        request: InsertUnifiedInvestigationPipelineEvent,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationPipelineEvent> {
        self.writer
            .insert_pipeline_event(&db::InsertPentagiPipelineEventInput {
                identity: unit_identity(&request.identity),
                pipeline_event_id: request.pipeline_event_id,
                stable_request_id: request.stable_request_id,
                task_plan_id: request.task_plan_id,
                subtask_id: request.subtask_id,
                event_ordinal: request.event_ordinal,
                event_kind: pipeline_event_kind(request.event_kind),
                actor_worker_run_id: request.actor_worker_run_id,
                parent_dispatch_receipt_id: request.parent_dispatch_receipt_id,
                event_sha256: request.event_sha256,
            })
            .await
            .map(pipeline_event)
            .map_err(map_storage_error)
    }

    async fn create_refiner_plan_ledger(
        &self,
        request: CreateUnifiedInvestigationRefinerPlanLedger,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationRefinerPlanLedger> {
        self.writer
            .create_refiner_plan_ledger(&db::CreateInvestigationRefinerPlanLedgerInput {
                identity: unit_identity(&request.identity),
                ledger_id: request.ledger_id,
                stable_request_id: request.stable_request_id,
                task_plan_id: request.task_plan_id,
                generator_pipeline_event_id: request.generator_pipeline_event_id,
                generator_manifest: request.generator_manifest,
            })
            .await
            .map(refiner_plan_ledger)
            .map_err(map_storage_error)
    }

    async fn load_refiner_plan_ledger(
        &self,
        request: LoadUnifiedInvestigationRefinerPlanLedger,
    ) -> UnifiedInvestigationRepoResult<Option<UnifiedInvestigationRefinerPlanLedger>> {
        self.writer
            .load_refiner_plan_ledger(&unit_identity(&request.identity), request.task_plan_id)
            .await
            .map(|ledger| ledger.map(refiner_plan_ledger))
            .map_err(map_storage_error)
    }

    async fn append_refiner_plan_patch(
        &self,
        request: AppendUnifiedInvestigationRefinerPlanPatch,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationRefinerPlanPatch> {
        self.writer
            .append_refiner_plan_patch(&db::AppendInvestigationRefinerPlanPatchInput {
                identity: unit_identity(&request.identity),
                patch_id: request.patch_id,
                stable_request_id: request.stable_request_id,
                ledger_id: request.ledger_id,
                task_plan_id: request.task_plan_id,
                refiner_pipeline_event_id: request.refiner_pipeline_event_id,
                expected_previous_state_sha256: request.expected_previous_state_sha256,
                remaining_plan_payload: request.remaining_plan_payload,
                active_realized_subtask_ids: request.active_realized_subtask_ids,
            })
            .await
            .map(refiner_plan_patch)
            .map_err(map_storage_error)
    }

    async fn seal_refiner_plan_ledger(
        &self,
        request: SealUnifiedInvestigationRefinerPlanLedger,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationRefinerPlanLedgerSeal> {
        self.writer
            .seal_refiner_plan_ledger(&db::SealInvestigationRefinerPlanLedgerInput {
                identity: unit_identity(&request.identity),
                seal_id: request.seal_id,
                stable_request_id: request.stable_request_id,
                ledger_id: request.ledger_id,
                task_plan_id: request.task_plan_id,
                result_barrier_pipeline_event_id: request.result_barrier_pipeline_event_id,
                expected_final_patch_sha256: request.expected_final_patch_sha256,
            })
            .await
            .map(refiner_plan_ledger_seal)
            .map_err(map_storage_error)
    }

    async fn load_refiner_plan_ledger_seal(
        &self,
        request: LoadUnifiedInvestigationRefinerPlanLedgerSeal,
    ) -> UnifiedInvestigationRepoResult<Option<UnifiedInvestigationRefinerPlanLedgerSeal>> {
        self.writer
            .load_refiner_plan_ledger_seal(&unit_identity(&request.identity), request.task_plan_id)
            .await
            .map(|seal| seal.map(refiner_plan_ledger_seal))
            .map_err(map_storage_error)
    }

    async fn seal_delegation_census(
        &self,
        request: SealUnifiedInvestigationDelegationCensus,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationDelegationCensus> {
        self.writer
            .seal_delegation_census(&db::SealPentagiDelegationCensusInput {
                identity: unit_identity(&request.identity),
                census_seal_id: request.census_seal_id,
                stable_request_id: request.stable_request_id,
                task_plan_id: request.task_plan_id,
                primary_dispatch_receipt_id: request.primary_dispatch_receipt_id,
                primary_worker_run_id: request.primary_worker_run_id,
                seal_sha256: request.seal_sha256,
            })
            .await
            .map(census)
            .map_err(map_storage_error)
    }

    async fn seal_task_plan(
        &self,
        request: SealUnifiedInvestigationTaskPlan,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationTaskPlan> {
        self.writer
            .seal_pentagi_plan(
                &unit_identity(&request.identity),
                request.task_plan_id,
                request.expected_row_version,
            )
            .await
            .map(task_plan)
            .map_err(map_storage_error)
    }

    async fn request_stop(
        &self,
        request: RequestUnifiedInvestigationStop,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationStopIntent> {
        self.writer
            .request_stop(&db::RequestInvestigationStopInput {
                identity: stage_identity(&request.identity),
                stop_intent_id: request.stop_intent_id,
                idempotency_key: request.idempotency_key,
                expected_run_head_sha256: request.expected_run_head_sha256,
                expected_change_seq: request.expected_change_seq,
            })
            .await
            .map(stop_intent)
            .map_err(map_storage_error)
    }

    async fn load_stop_intent(
        &self,
        identity: UnifiedInvestigationStageIdentity,
    ) -> UnifiedInvestigationRepoResult<Option<UnifiedInvestigationStopIntent>> {
        self.writer
            .load_stop_intent(&stage_identity(&identity))
            .await
            .map_err(map_storage_error)
            .map(|row| row.map(stop_intent))
    }

    async fn seal_closure(
        &self,
        request: SealUnifiedInvestigationClosure,
    ) -> UnifiedInvestigationRepoResult<InvestigationRunClosureV1> {
        self.writer
            .seal_closure(&db::SealInvestigationRunClosureInput {
                identity: stage_identity(&request.identity),
                closure_id: request.closure_id,
                stable_request_id: request.stable_request_id,
                expected_run_head_sha256: request.expected_run_head_sha256,
            })
            .await
            .map_err(map_storage_error)
            .and_then(closure)
    }

    async fn load_closure(
        &self,
        identity: UnifiedInvestigationStageIdentity,
    ) -> UnifiedInvestigationRepoResult<Option<InvestigationRunClosureV1>> {
        self.writer
            .load_closure(&stage_identity(&identity))
            .await
            .map_err(map_storage_error)
            .and_then(|row| row.map(closure).transpose())
    }

    async fn publish_closure(
        &self,
        request: PublishUnifiedInvestigationClosure,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationClosurePublication> {
        self.writer
            .publish_closure(&db::PublishInvestigationStageClosureInput {
                identity: stage_identity(&request.identity),
                publication_id: request.publication_id,
                stable_request_id: request.stable_request_id,
                closure_id: request.closure_id,
            })
            .await
            .map_err(map_storage_error)
            .map(|published| UnifiedInvestigationClosurePublication {
                publication_id: published.publication.publication_id,
                closure_id: published.publication.closure_id,
                authority_id: published.publication.authority_id,
                operation_id: published.publication.operation_id,
                stage_execution_id: published.publication.stage_execution_id,
                scope_snapshot_id: published.publication.scope_snapshot_id,
                closure_sha256: published.publication.closure_sha256,
                disposition: published.publication.disposition,
                member_count: published.publication.member_count,
                member_set_sha256: published.publication.member_set_sha256,
                publication_sha256: published.publication.publication_sha256,
                published_at: published.publication.published_at,
                members: published
                    .members
                    .into_iter()
                    .map(|member| UnifiedInvestigationClosurePublicationMember {
                        publication_member_id: member.publication_member_id,
                        publication_id: member.publication_id,
                        member_ordinal: member.member_ordinal,
                        operation_id: member.operation_id,
                        stage_execution_id: member.stage_execution_id,
                        scope_snapshot_id: member.scope_snapshot_id,
                        stage_run_unit_id: member.stage_run_unit_id,
                        organization_id: member.organization_id,
                        stage_team_plan_id: member.stage_team_plan_id,
                        member_sha256: member.member_sha256,
                        passed_at: member.passed_at,
                    })
                    .collect(),
                replayed: published.replayed,
            })
    }

    async fn load_closure_publication_for_operation(
        &self,
        operation_id: Uuid,
    ) -> UnifiedInvestigationRepoResult<Option<UnifiedInvestigationClosurePublication>> {
        self.writer
            .load_closure_publication_for_operation(operation_id)
            .await
            .map_err(map_storage_error)
            .map(|published| {
                published.map(|published| UnifiedInvestigationClosurePublication {
                    publication_id: published.publication.publication_id,
                    closure_id: published.publication.closure_id,
                    authority_id: published.publication.authority_id,
                    operation_id: published.publication.operation_id,
                    stage_execution_id: published.publication.stage_execution_id,
                    scope_snapshot_id: published.publication.scope_snapshot_id,
                    closure_sha256: published.publication.closure_sha256,
                    disposition: published.publication.disposition,
                    member_count: published.publication.member_count,
                    member_set_sha256: published.publication.member_set_sha256,
                    publication_sha256: published.publication.publication_sha256,
                    published_at: published.publication.published_at,
                    members: published
                        .members
                        .into_iter()
                        .map(|member| UnifiedInvestigationClosurePublicationMember {
                            publication_member_id: member.publication_member_id,
                            publication_id: member.publication_id,
                            member_ordinal: member.member_ordinal,
                            operation_id: member.operation_id,
                            stage_execution_id: member.stage_execution_id,
                            scope_snapshot_id: member.scope_snapshot_id,
                            stage_run_unit_id: member.stage_run_unit_id,
                            organization_id: member.organization_id,
                            stage_team_plan_id: member.stage_team_plan_id,
                            member_sha256: member.member_sha256,
                            passed_at: member.passed_at,
                        })
                        .collect(),
                    replayed: true,
                })
            })
    }

    async fn open_and_seal_main_read_session_set(
        &self,
        request: OpenAndSealUnifiedInvestigationMainReadSessionSet,
    ) -> UnifiedInvestigationRepoResult<UnifiedInvestigationMainReadSessionSetSeal> {
        if request.members.is_empty() {
            return Err(UnifiedInvestigationRepositoryError::InvalidRequest {
                detail: "main read-session set cannot be empty".to_string(),
            });
        }
        let mut prepared = Vec::with_capacity(request.members.len());
        for member in request.members {
            let session =
                MainOrganizationReadSessionV1::host_bind(BindMainOrganizationReadSessionV1 {
                    operation_id: request.identity.operation_id,
                    stage_execution_id: request.identity.stage_execution_id,
                    owning_stage_run_request_id: request
                        .identity
                        .owning_stage_run_request_id
                        .clone(),
                    stage_run_unit_id: member.stage_run_unit_id,
                    organization_id: member.organization_id,
                    snapshot_id: member.snapshot_id,
                    snapshot_sha256: member.snapshot_sha256.clone(),
                    context_chain_id: member.context_chain_id,
                    transcript_partition_id: member.transcript_partition_id,
                })
                .map_err(|error| {
                    UnifiedInvestigationRepositoryError::InvalidRequest {
                        detail: error.to_string(),
                    }
                })?;
            let receipt = session
                .host_receipt(
                    member.context_item_count,
                    member.context_item_set_sha256.clone(),
                    member.methodology_hit_count,
                    member.methodology_result_set_sha256.clone(),
                    member.omission_count,
                    member.omission_set_sha256.clone(),
                )
                .map_err(
                    |error| UnifiedInvestigationRepositoryError::InvalidRequest {
                        detail: error.to_string(),
                    },
                )?;
            prepared.push((member, session, receipt));
        }
        let partition_set = prepared
            .iter()
            .map(|(_, session, _)| session.clone())
            .collect::<Vec<_>>();
        golish_core::investigation_main_read_session::validate_main_read_session_partition_set(
            &partition_set,
        )
        .map_err(
            |error| UnifiedInvestigationRepositoryError::InvalidRequest {
                detail: error.to_string(),
            },
        )?;
        self.ensure_stage_authority(&request.identity).await?;
        let session_set_ordinal = i64::try_from(request.session_set_ordinal).map_err(|_| {
            UnifiedInvestigationRepositoryError::InvalidRequest {
                detail: "session_set_ordinal".to_string(),
            }
        })?;
        let session_set = main_sessions::begin_session_set(
            &self.pool,
            &main_sessions::BeginMainSessionSet {
                session_set_id: request.session_set_id,
                stable_request_id: request.session_set_stable_request_id,
                authority_id: request.identity.authority_id,
                operation_id: request.identity.operation_id,
                stage_execution_id: request.identity.stage_execution_id,
                owning_stage_run_request_id: request.identity.owning_stage_run_request_id.clone(),
                scope_snapshot_id: request.identity.scope_snapshot_id,
                session_set_ordinal,
            },
        )
        .await
        .map_err(map_main_session_error)?;
        let mut receipts = Vec::with_capacity(prepared.len());
        if session_set.status == "open" {
            for (member, session, receipt) in &prepared {
                main_sessions::seal_analysis_snapshot(
                    &self.pool,
                    &main_sessions::SealInvestigationAnalysisSnapshot {
                        snapshot_id: member.snapshot_id,
                        authority_id: request.identity.authority_id,
                        operation_id: request.identity.operation_id,
                        stage_execution_id: request.identity.stage_execution_id,
                        owning_stage_run_request_id: request
                            .identity
                            .owning_stage_run_request_id
                            .clone(),
                        stage_run_unit_id: member.stage_run_unit_id,
                        scope_snapshot_id: request.identity.scope_snapshot_id,
                        organization_id: member.organization_id,
                        snapshot_sha256: member.snapshot_sha256.clone(),
                        context_item_count: member.context_item_count,
                        context_item_set_sha256: member.context_item_set_sha256.clone(),
                        methodology_hit_count: member.methodology_hit_count,
                        methodology_result_set_sha256: member.methodology_result_set_sha256.clone(),
                        omission_count: member.omission_count,
                        omission_set_sha256: member.omission_set_sha256.clone(),
                    },
                )
                .await
                .map_err(map_main_session_error)?;
                let persisted = main_sessions::insert_read_session(
                    &self.pool,
                    request.session_set_id,
                    request.identity.authority_id,
                    request.identity.operation_id,
                    request.identity.stage_execution_id,
                    request.identity.scope_snapshot_id,
                    session,
                )
                .await
                .map_err(map_main_session_error)?;
                main_sessions::record_read_receipt(&self.pool, member.receipt_id, receipt)
                    .await
                    .map_err(map_main_session_error)?;
                receipts.push(UnifiedInvestigationMainReadSessionReceipt {
                    stage_run_unit_id: persisted.stage_run_unit_id,
                    organization_id: persisted.organization_id,
                    snapshot_id: persisted.snapshot_id,
                    snapshot_sha256: persisted.snapshot_sha256,
                    main_read_session_id: persisted.main_read_session_id,
                    context_chain_id: persisted.context_chain_id,
                    transcript_partition_id: persisted.transcript_partition_id,
                    session_contract_version: persisted.session_contract_version,
                    receipt_id: member.receipt_id,
                    receipt_sha256: receipt.receipt_sha256.clone(),
                });
            }
        } else if session_set.status == "sealed" {
            for (member, session, receipt) in &prepared {
                let persisted =
                    main_sessions::load_read_session(&self.pool, session.main_read_session_id)
                        .await
                        .map_err(map_main_session_error)?;
                let stored_receipt =
                    main_sessions::load_read_receipt(&self.pool, session.main_read_session_id)
                        .await
                        .map_err(map_main_session_error)?;
                let session_matches = persisted.session_set_id == request.session_set_id
                    && persisted.authority_id == request.identity.authority_id
                    && persisted.operation_id == session.operation_id
                    && persisted.stage_execution_id == session.stage_execution_id
                    && persisted.owning_stage_run_request_id == session.owning_stage_run_request_id
                    && persisted.stage_run_unit_id == session.stage_run_unit_id
                    && persisted.scope_snapshot_id == request.identity.scope_snapshot_id
                    && persisted.organization_id == session.organization_id
                    && persisted.snapshot_id == session.snapshot_id
                    && persisted.snapshot_sha256 == session.snapshot_sha256
                    && persisted.context_chain_id == session.context_chain_id
                    && persisted.transcript_partition_id == session.transcript_partition_id
                    && persisted.session_contract_version == session.session_contract_version;
                let receipt_matches = stored_receipt.receipt_id == member.receipt_id
                    && stored_receipt.main_read_session_id == receipt.main_read_session_id
                    && stored_receipt.operation_id == receipt.operation_id
                    && stored_receipt.stage_execution_id == receipt.stage_execution_id
                    && stored_receipt.stage_run_unit_id == receipt.stage_run_unit_id
                    && stored_receipt.organization_id == receipt.organization_id
                    && stored_receipt.snapshot_id == receipt.snapshot_id
                    && stored_receipt.snapshot_sha256 == receipt.snapshot_sha256
                    && stored_receipt.context_item_count == i64::from(receipt.context_item_count)
                    && stored_receipt.context_item_set_sha256 == receipt.context_item_set_sha256
                    && stored_receipt.methodology_hit_count
                        == i64::from(receipt.methodology_hit_count)
                    && stored_receipt.methodology_result_set_sha256
                        == receipt.methodology_result_set_sha256
                    && stored_receipt.omission_count == i64::from(receipt.omission_count)
                    && stored_receipt.omission_set_sha256 == receipt.omission_set_sha256
                    && stored_receipt.receipt_sha256 == receipt.receipt_sha256;
                if !session_matches || !receipt_matches {
                    return Err(UnifiedInvestigationRepositoryError::AuthorityMismatch {
                        detail: "main read-session sealed replay mismatch".to_string(),
                    });
                }
                receipts.push(UnifiedInvestigationMainReadSessionReceipt {
                    stage_run_unit_id: persisted.stage_run_unit_id,
                    organization_id: persisted.organization_id,
                    snapshot_id: persisted.snapshot_id,
                    snapshot_sha256: persisted.snapshot_sha256,
                    main_read_session_id: persisted.main_read_session_id,
                    context_chain_id: persisted.context_chain_id,
                    transcript_partition_id: persisted.transcript_partition_id,
                    session_contract_version: persisted.session_contract_version,
                    receipt_id: stored_receipt.receipt_id,
                    receipt_sha256: stored_receipt.receipt_sha256,
                });
            }
        } else {
            return Err(UnifiedInvestigationRepositoryError::Conflict {
                detail: "main read-session set has invalid status".to_string(),
            });
        }
        let sealed = if session_set.status == "sealed" {
            session_set
        } else {
            main_sessions::seal_session_set(
                &self.pool,
                request.session_set_id,
                session_set.row_version,
            )
            .await
            .map_err(map_main_session_error)?
        };
        let member_count = sealed.member_count.ok_or_else(|| {
            UnifiedInvestigationRepositoryError::Infrastructure {
                detail: "sealed main-session set has no member count".to_string(),
            }
        })?;
        let member_set_sha256 = sealed.member_set_sha256.ok_or_else(|| {
            UnifiedInvestigationRepositoryError::Infrastructure {
                detail: "sealed main-session set has no member hash".to_string(),
            }
        })?;
        if member_count != i64::try_from(receipts.len()).unwrap_or(i64::MAX) {
            return Err(UnifiedInvestigationRepositoryError::AuthorityMismatch {
                detail: "main read-session sealed member count mismatch".to_string(),
            });
        }
        receipts.sort_by_key(|receipt| (receipt.organization_id, receipt.main_read_session_id));
        Ok(UnifiedInvestigationMainReadSessionSetSeal {
            authority_id: sealed.authority_id,
            operation_id: sealed.operation_id,
            stage_execution_id: sealed.stage_execution_id,
            owning_stage_run_request_id: sealed.owning_stage_run_request_id,
            scope_snapshot_id: sealed.scope_snapshot_id,
            session_set_id: sealed.session_set_id,
            member_count,
            member_set_sha256,
            row_version: sealed.row_version,
            receipts,
        })
    }

    async fn load_main_read_session_authority(
        &self,
        identity: UnifiedInvestigationUnitIdentity,
    ) -> UnifiedInvestigationRepoResult<Option<UnifiedInvestigationMainReadSessionAuthority>> {
        let rows = sqlx::query_as::<_, SealedMainReadSessionAuthorityRow>(
            r#"SELECT read_session.stage_run_unit_id,read_session.organization_id,
                      read_session.snapshot_id,read_session.snapshot_sha256,
                      snapshot.context_item_count,snapshot.context_item_set_sha256,
                      snapshot.methodology_hit_count,snapshot.methodology_result_set_sha256,
                      snapshot.omission_count,snapshot.omission_set_sha256,
                      read_session.main_read_session_id,read_session.context_chain_id,
                      read_session.transcript_partition_id
                 FROM investigation_main_session_sets session_set
                 JOIN investigation_main_read_sessions read_session
                   ON read_session.session_set_id=session_set.session_set_id
                 JOIN investigation_analysis_snapshot_authorities snapshot
                   ON snapshot.snapshot_id=read_session.snapshot_id
                  AND snapshot.authority_id=session_set.authority_id
                  AND snapshot.operation_id=session_set.operation_id
                  AND snapshot.stage_execution_id=session_set.stage_execution_id
                  AND snapshot.stage_run_unit_id=read_session.stage_run_unit_id
                  AND snapshot.scope_snapshot_id=session_set.scope_snapshot_id
                  AND snapshot.organization_id=read_session.organization_id
                WHERE session_set.authority_id=$1 AND session_set.operation_id=$2
                  AND session_set.stage_execution_id=$3
                  AND session_set.owning_stage_run_request_id=$4
                  AND session_set.scope_snapshot_id=$5 AND session_set.status='sealed'
                  AND read_session.stage_run_unit_id=$6
                  AND read_session.organization_id=$7"#,
        )
        .bind(identity.stage.authority_id)
        .bind(identity.stage.operation_id)
        .bind(identity.stage.stage_execution_id)
        .bind(&identity.stage.owning_stage_run_request_id)
        .bind(identity.stage.scope_snapshot_id)
        .bind(identity.stage_run_unit_id)
        .bind(identity.organization_id)
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(
            |error| UnifiedInvestigationRepositoryError::Infrastructure {
                detail: error.to_string(),
            },
        )?;
        if rows.len() > 1 {
            return Err(UnifiedInvestigationRepositoryError::AuthorityMismatch {
                detail: "multiple sealed main read-session authorities matched one Unit"
                    .to_string(),
            });
        }
        rows.into_iter()
            .next()
            .map(|row| {
                Ok(UnifiedInvestigationMainReadSessionAuthority {
                    stage_run_unit_id: row.stage_run_unit_id,
                    organization_id: row.organization_id,
                    snapshot_id: row.snapshot_id,
                    snapshot_sha256: row.snapshot_sha256,
                    context_item_count: closure_count(
                        row.context_item_count,
                        "context_item_count",
                    )?,
                    context_item_set_sha256: row.context_item_set_sha256,
                    methodology_hit_count: closure_count(
                        row.methodology_hit_count,
                        "methodology_hit_count",
                    )?,
                    methodology_result_set_sha256: row.methodology_result_set_sha256,
                    omission_count: closure_count(row.omission_count, "omission_count")?,
                    omission_set_sha256: row.omission_set_sha256,
                    main_read_session_id: row.main_read_session_id,
                    context_chain_id: row.context_chain_id,
                    transcript_partition_id: row.transcript_partition_id,
                })
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_storage_failures_without_erasing_error_class() {
        assert_eq!(
            map_storage_error(db::UnifiedInvestigationRuntimeStoreError::InvalidInput(
                "hash"
            ))
            .code(),
            "unified_investigation_repository_invalid_request"
        );
        assert_eq!(
            map_storage_error(db::UnifiedInvestigationRuntimeStoreError::CasConflict(
                "head"
            ))
            .code(),
            "unified_investigation_repository_conflict"
        );
        assert_eq!(
            map_storage_error(db::UnifiedInvestigationRuntimeStoreError::IdentityConflict(
                "owner"
            ))
            .code(),
            "unified_investigation_repository_authority_mismatch"
        );
    }
}
