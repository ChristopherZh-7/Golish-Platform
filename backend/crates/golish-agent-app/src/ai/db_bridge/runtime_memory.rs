//! Runtime-memory repository bridge from the sqlx-free agent-kit contract to
//! the concrete `golish-db` repositories.

use async_trait::async_trait;
use serde_json::Value;
use uuid::Uuid;

use golish_agent_kit::db_traits::{
    AttackV2WaveAuthorityView, AttackV2WaveEntryView, AttackV2WaveRuntimeUnitView,
    AttackV2WaveUnitStateView, BindStageTeamLeaderFinalSubmitter, BlockStageTeamUnit,
    BlockedStageTeamUnitView, BoundStageTeamLeaderFinalSubmitterView,
    CandidateExecutionContinuationView, CandidateHeartbeatView, CandidateRecoveryCaseView,
    CandidateRecoveryDecision, CandidateReleaseView, CandidateTerminalBarrierView,
    CandidateTerminalIntentStatus, CandidateTerminalIntentView, CheckpointBoundWorkerChain,
    CheckpointCandidateTerminalBarrier, CheckpointWorker, ClaimCandidateAttempt,
    ClaimStageAggregator, ClaimStageTeamLeader, ClaimStageWorkItem, ClaimWorkerAndBindChain,
    ClaimedCandidateAttemptView, ClaimedStageWorkItemView, ClaimedWorkerView,
    CloseAttackV2VerificationUnit, CloseStageRequestEpoch, CloseWaveGatePass,
    ClosedAttackV2VerificationUnitView, ClosedStageRequestEpochView, ClosedWaveGatePass,
    CompleteStageWorker, CompletedStageWorkerView, ControlCandidateAttempt,
    ConvergedCandidateRecoveryView, CreateRuntimeOperation, CreatedRuntimeOperation,
    FinalizeScopingScope, FinalizeStageTeamUnit, FinalizeUnitPass, FinalizedScopingScope,
    FinalizedStageTeamUnitView, FinalizedUnitPass, FinishWorkerAttempt, FinishedWorkerAttempt,
    FrozenOrganizationScopeUnit, HeartbeatCandidateAttempt, LoadBoundWorkerChain,
    LoadInheritedStageHandoffs, LoadStageTeamBarrier, LoadWorkerCheckpoint, LoadedBoundWorkerChain,
    LoadedWorkerCheckpoint, NewStageDeliverableSubmission, OpenStageTeamRepair,
    OpenedStageTeamRepairView, OperationStateView, ParkStageTeamLeader, ParkedStageTeamLeaderView,
    PauseWorkerForContinuation, PersistedStageDeliverableSubmission, ProjectScopeRegistration,
    ReapedRuntimeWorker, RecoverCandidateTerminalIntent, ReopenStageTeamLeaderAfterGateBlock,
    ReopenedStageTeamLeaderAfterGateBlockView, RequestStageWorker, RequestedStageWorkerView,
    ResolveCandidateRecovery, ResolveStageTeamRecovery, ResolvedCandidateRecoveryView,
    ResolvedStageTeamRecoveryView, RetriedStageWorkerView, RetryStageWorker,
    RuntimeExpiredWorkerDisposition, RuntimeMemoryError, RuntimeMemoryRecordSource,
    RuntimeMemoryRepository, RuntimeStageHandoffView, RuntimeStageTeamPlanStatus,
    RuntimeStageUnitStatus, RuntimeStageUnitView, RuntimeStageWorkItemStatus, RuntimeWorkerFence,
    RuntimeWorkerStatus, RuntimeWorkerView, SeedStageRuntime, SeedStageTeamRuntime,
    SeededStageRuntime, SeededStageTeamRuntime, StageTeamBarrierView, StageTeamPlanView,
    StageWorkItemView, StageWorkerOutputDisposition, StageWorkerOutputView,
    StageWorkerRequestDecision, StageWorkerRequestView, SubmitCandidateAttempt,
    SubmittedCandidateAttemptView, TaskView, TerminalizeCandidateAttempt,
    TerminalizeCandidateIntent, TerminalizedCandidateAttemptView, WorkerToolMutation,
};
use golish_agent_kit::harness::attack_execution::{
    select_attack_read, AttackDecisionSemantic, AttackDecisionSemanticKind, AttackReadSelection,
    AttackReadSource, AttackReviewCounts, AttackShadowComparison, CompleteAttackRead, V2AttackRead,
};
use golish_agent_kit::harness::{CanonicalFactKey, CanonicalFactRef, StageKind};
use golish_agent_kit::runtime_memory::RuntimeMemoryContract;
use golish_agent_kit::task_orchestrator::stage_execution::{
    CompleteTerminalStageExecution, StageExecution, StageExecutionStatus, TransitionStageExecution,
    TransitionedStageExecution,
};
use golish_db::repo::runtime_memory_rollout::RuntimeMemoryContract as DbRuntimeMemoryContract;
use golish_db::repo::runtime_memory_tx::{
    BindStageTeamLeaderFinalSubmitterRow, BlockStageTeamUnitRow, CheckpointBoundWorkerChainRow,
    ClaimStageAggregatorRow, ClaimStageTeamLeaderRow, ClaimStageWorkItemRow,
    ClaimWorkerAndBindChainRow, CloseStageRequestEpochRow, CloseWaveGatePassRow,
    ClosedWaveGatePassRow, CompleteTerminalStageExecutionRow, CreateRuntimeOperationRow,
    CreatedRuntimeOperationRow, FinalizeScopingScopeRow, FinalizeStageTeamUnitRow,
    FinalizeUnitPassRow, FinalizedScopingScopeRow, FinishWorkerAttemptRow, LoadBoundWorkerChainRow,
    LoadStageTeamBarrierRow, LoadWorkerCheckpointRow, OpenStageTeamRepairRow,
    ParkStageTeamLeaderRow, PauseWorkerForContinuationRow, ReopenStageTeamLeaderAfterGateBlockRow,
    RequestStageWorkerRow, RuntimeMemoryStoreError, RuntimeMemoryTxFence, SeedStageRuntimeRow,
    SeedStageTeamRuntimeRow, StageTeamPlanSeedRow, StageWorkItemSeedRow,
    TransitionStageExecutionRow,
};
use golish_db::repo::stage_teams::{
    CompleteStageWorkerRow, ResolveStageTeamRecoveryRow, RetryStageWorkerRow,
};

use super::convert::{convert_agent_type_back, convert_task_status};
use super::GolishDbRepoProvider;

fn runtime_memory_contract_from_db(contract: DbRuntimeMemoryContract) -> RuntimeMemoryContract {
    match contract {
        DbRuntimeMemoryContract::LegacyV1 => RuntimeMemoryContract::LegacyV1,
        DbRuntimeMemoryContract::DualWriteLegacyRead => RuntimeMemoryContract::DualWriteLegacyRead,
        DbRuntimeMemoryContract::DualWriteV2Preferred => {
            RuntimeMemoryContract::DualWriteV2Preferred
        }
        DbRuntimeMemoryContract::V2Only => RuntimeMemoryContract::V2Only,
    }
}

fn runtime_memory_error_from_db(error: RuntimeMemoryStoreError) -> RuntimeMemoryError {
    match error {
        RuntimeMemoryStoreError::InvalidContractTransition { from, to } => {
            RuntimeMemoryError::InvalidContractTransition {
                from: runtime_memory_contract_from_db(from),
                to: runtime_memory_contract_from_db(to),
            }
        }
        RuntimeMemoryStoreError::StaleVersion { expected, .. } => {
            RuntimeMemoryError::StaleVersion { expected }
        }
        RuntimeMemoryStoreError::Conflict { code } => RuntimeMemoryError::Conflict { code },
        RuntimeMemoryStoreError::IdentityMismatch { code } => {
            RuntimeMemoryError::IdentityMismatch { code }
        }
        RuntimeMemoryStoreError::Missing { entity } => RuntimeMemoryError::Missing { entity },
        RuntimeMemoryStoreError::LeaseLost {
            worker_run_id,
            attempt_epoch,
        } => RuntimeMemoryError::LeaseLost {
            worker_run_id,
            attempt_epoch,
        },
        RuntimeMemoryStoreError::Sqlx(error) => RuntimeMemoryError::Storage(error.to_string()),
        RuntimeMemoryStoreError::Repository(error) => {
            RuntimeMemoryError::Storage(error.to_string())
        }
    }
}

fn attack_wave_entry_from_db(
    entry: &golish_db::repo::attack_waves::AttackWaveEntry,
) -> AttackV2WaveEntryView {
    match entry {
        golish_db::repo::attack_waves::AttackWaveEntry::VulnTriageHandoff { .. } => {
            AttackV2WaveEntryView::VulnTriageHandoff
        }
        golish_db::repo::attack_waves::AttackWaveEntry::FactDeltaConsolidation { .. } => {
            AttackV2WaveEntryView::FactDeltaConsolidation
        }
        golish_db::repo::attack_waves::AttackWaveEntry::ForkedVulnHandoff { .. } => {
            AttackV2WaveEntryView::ForkedVulnHandoff
        }
    }
}

fn attack_wave_authority_from_db(
    authority: golish_db::repo::attack_waves::AttackWaveAuthority,
) -> AttackV2WaveAuthorityView {
    use golish_db::repo::attack_waves::{AttackWaveAuthority, CurrentAttackWaveUnitState};

    match authority {
        AttackWaveAuthority::Initial(initial) => AttackV2WaveAuthorityView::Initial {
            operation_id: initial.operation_id,
            scope_snapshot_id: initial.scope_snapshot_id,
            generation: initial.generation,
            units: initial
                .units
                .into_iter()
                .map(|unit| AttackV2WaveRuntimeUnitView {
                    wave_unit_id: None,
                    organization_id: unit.organization_id,
                    ordinal: unit.ordinal,
                    status: "initial".to_string(),
                    entry: attack_wave_entry_from_db(&unit.entry),
                    state: AttackV2WaveUnitStateView::AwaitingManifest,
                })
                .collect(),
        },
        AttackWaveAuthority::Current(current) => AttackV2WaveAuthorityView::Current {
            operation_id: current.wave.operation_id,
            scope_snapshot_id: current.wave.scope_snapshot_id,
            wave_run_id: current.wave.id,
            generation: current.wave.generation,
            status: current.wave.status,
            units: current
                .units
                .into_iter()
                .map(|authority| AttackV2WaveRuntimeUnitView {
                    wave_unit_id: Some(authority.unit.id),
                    organization_id: authority.unit.organization_id,
                    ordinal: authority.unit.ordinal,
                    status: authority.unit.status.clone(),
                    entry: attack_wave_entry_from_db(&authority.unit.entry),
                    state: match authority.state {
                        CurrentAttackWaveUnitState::AwaitingManifest => {
                            AttackV2WaveUnitStateView::AwaitingManifest
                        }
                        CurrentAttackWaveUnitState::Runnable { .. } => {
                            AttackV2WaveUnitStateView::FrozenManifest
                        }
                        CurrentAttackWaveUnitState::TerminalNoInput => {
                            AttackV2WaveUnitStateView::TerminalNoInput
                        }
                    },
                })
                .collect(),
        },
        AttackWaveAuthority::Terminal(terminal) => AttackV2WaveAuthorityView::Terminal {
            operation_id: terminal.last_wave.operation_id,
            scope_snapshot_id: terminal.last_wave.scope_snapshot_id,
            wave_run_id: terminal.last_wave.id,
            generation: terminal.last_wave.generation,
        },
    }
}

fn stage_submission_error_from_db(
    error: golish_db::repo::stage_deliverable_submissions::StageDeliverableSubmissionError,
) -> RuntimeMemoryError {
    use golish_db::repo::stage_deliverable_submissions::StageDeliverableSubmissionError;

    match error {
        StageDeliverableSubmissionError::IdentityMismatch { code } => {
            RuntimeMemoryError::IdentityMismatch { code }
        }
        StageDeliverableSubmissionError::Conflict { code }
        | StageDeliverableSubmissionError::InvalidPayload { code } => {
            RuntimeMemoryError::Conflict { code }
        }
        StageDeliverableSubmissionError::Missing { entity } => {
            RuntimeMemoryError::Missing { entity }
        }
        StageDeliverableSubmissionError::Sqlx(error) => {
            RuntimeMemoryError::Storage(error.to_string())
        }
    }
}

fn stage_submission_from_db(
    row: golish_db::repo::stage_deliverable_submissions::StageDeliverableSubmissionRow,
) -> PersistedStageDeliverableSubmission {
    PersistedStageDeliverableSubmission {
        deliverable_submission_id: row.id,
        operation_id: row.operation_id,
        stage_execution_id: row.stage_execution_id,
        stage_run_unit_id: row.stage_run_unit_id,
        worker_run_id: row.worker_run_id,
        organization_id: row.organization_id,
        tool_call_record_id: row.tool_call_record_id,
        tool_request_id: row.tool_request_id,
        stage_kind: row.stage_kind,
        attempt_epoch: row.attempt_epoch,
        lease_token: row.lease_token,
        payload: row.payload,
        payload_sha256: row.payload_sha256,
    }
}

fn finalized_scoping_scope_from_db(row: FinalizedScopingScopeRow) -> FinalizedScopingScope {
    FinalizedScopingScope {
        operation_id: row.scope.snapshot.operation_id,
        project_scope_id: row.scope.snapshot.project_scope_id,
        stage_execution_id: row.decision.stage_execution_id,
        root_organization_id: row.scope.snapshot.root_organization_id,
        deliverable_submission_id: row.submission.id,
        scope_decision_id: row.decision.id,
        scope_snapshot_id: row.scope.snapshot.id,
        scoping_root_unit_id: row.root_unit.id,
        mode: row.scope.snapshot.mode,
        scope_hash: row.scope.snapshot.scope_hash,
        units: row
            .scope
            .units
            .into_iter()
            .map(|unit| FrozenOrganizationScopeUnit {
                organization_id: unit.organization_id,
                parent_organization_id: unit.parent_organization_id,
                organization_name_at_freeze: unit.organization_name_at_freeze,
                role: unit.role,
                depth: unit.depth,
                ordinal: unit.ordinal,
                ownership_percent: unit.ownership_percent,
                decision_row_id: unit.decision_row_id,
                approval_source: unit.approval_source,
            })
            .collect(),
        replayed: row.replayed,
    }
}

fn runtime_stage_unit_from_db(
    row: golish_db::repo::stage_run_units::StageRunUnitRow,
) -> Result<RuntimeStageUnitView, RuntimeMemoryError> {
    let status = RuntimeStageUnitStatus::try_parse(&row.status).ok_or_else(|| {
        RuntimeMemoryError::Storage(format!("decode runtime stage-unit status: {}", row.status))
    })?;
    Ok(RuntimeStageUnitView {
        id: row.id,
        operation_id: row.operation_id,
        stage_execution_id: row.stage_execution_id,
        scope_snapshot_id: row.scope_snapshot_id,
        organization_id: row.organization_id,
        stage_kind: row.stage_kind,
        generation: row.generation,
        specialist: row.specialist,
        status,
        gate_attempt: row.gate_attempt,
        pass_watermark: row.pass_watermark,
        row_version: row.row_version,
    })
}

fn runtime_worker_from_db(
    row: golish_db::repo::stage_worker_runs::StageWorkerRunRow,
) -> Result<RuntimeWorkerView, RuntimeMemoryError> {
    let status = RuntimeWorkerStatus::try_parse(&row.status).ok_or_else(|| {
        RuntimeMemoryError::Storage(format!("decode runtime worker status: {}", row.status))
    })?;
    Ok(RuntimeWorkerView {
        id: row.id,
        operation_id: row.operation_id,
        stage_execution_id: row.stage_execution_id,
        stage_run_unit_id: row.stage_run_unit_id,
        work_item_id: row.work_item_id,
        organization_id: row.organization_id,
        worker_generation: row.worker_generation,
        specialist: row.specialist,
        work_item_kind: row.work_item_kind,
        work_item_key: row.work_item_key,
        agent_path: row.agent_path,
        parent_request_id: row.parent_request_id,
        message_chain_id: row.message_chain_id,
        status,
        gate_attempt: row.gate_attempt,
        checkpoint: row.checkpoint,
        checkpoint_version: row.checkpoint_version,
        lease_token: row.lease_token,
        lease_owner: row.lease_owner,
        lease_expires_at: row.lease_expires_at,
        heartbeat_at: row.heartbeat_at,
        attempt_epoch: row.attempt_epoch,
        active_tool_call_id: row.active_tool_call_id,
        active_tool_started_at: row.active_tool_started_at,
        evidence_watermark: row.evidence_watermark,
    })
}

fn candidate_terminal_intent_from_db(
    row: golish_db::repo::candidate_recovery::CandidateTerminalIntentQueueRow,
) -> CandidateTerminalIntentView {
    let status = if row.receipt_id.is_some() {
        CandidateTerminalIntentStatus::Consumed
    } else if row.barrier_id.is_some() {
        CandidateTerminalIntentStatus::BarrierReady
    } else {
        CandidateTerminalIntentStatus::Pending
    };
    CandidateTerminalIntentView {
        id: row.intent_id,
        request_id: row.request_id,
        operation_id: row.operation_id,
        organization_id: row.organization_id,
        candidate_id: row.candidate_id,
        attempt_id: row.attempt_id,
        worker_run_id: row.worker_run_id,
        tool_call_record_id: row.tool_call_record_id,
        candidate_plan_hash: row.candidate_plan_hash,
        result_hash: row.result_hash,
        evidence_manifest_hash: row.evidence_manifest_hash,
        tool_result_hash: row.tool_result_hash,
        intent_hash: row.intent_hash,
        barrier_id: row.barrier_id,
        barrier_hash: row.barrier_hash,
        status,
        created_at: row.created_at,
    }
}

fn candidate_terminal_barrier_from_db(
    recorded: golish_db::repo::candidate_recovery::RecordedCandidateTerminalBarrier,
) -> CandidateTerminalBarrierView {
    candidate_terminal_barrier_row_from_db(recorded.barrier, recorded.replayed)
}

fn candidate_terminal_barrier_row_from_db(
    row: golish_db::repo::candidate_recovery::CandidateTerminalBarrierRow,
    replayed: bool,
) -> CandidateTerminalBarrierView {
    CandidateTerminalBarrierView {
        id: row.id,
        request_id: row.request_id,
        terminal_intent_id: row.intent_id,
        attempt_id: row.attempt_id,
        worker_run_id: row.worker_run_id,
        tool_call_record_id: row.tool_call_record_id,
        message_chain_id: row.message_chain_id,
        attempt_epoch: row.attempt_epoch,
        checkpoint_version: row.checkpoint_version,
        checkpoint_hash: row.checkpoint_hash,
        tool_result_hash: row.tool_result_hash,
        barrier_hash: row.barrier_hash,
        created_at: row.created_at,
        replayed,
    }
}

fn terminalized_candidate_from_db(
    row: golish_db::repo::finding_lineage::TerminalizedCandidateAttempt,
) -> TerminalizedCandidateAttemptView {
    TerminalizedCandidateAttemptView {
        scope_snapshot_id: row.scope_snapshot_id,
        wave_run_id: row.wave_run_id,
        wave_unit_id: row.wave_unit_id,
        organization_id: row.organization_id,
        candidate_id: row.candidate_id,
        attempt_id: row.attempt_id,
        status: row.status,
        disposition: row.disposition,
        finding_id: row.finding_id,
        evidence_count: row.evidence_count,
        fact_delta_count: row.fact_delta_count,
        replayed: row.replayed,
    }
}

async fn candidate_control_to_db(
    pool: &sqlx::PgPool,
    input: ControlCandidateAttempt,
) -> Result<golish_db::repo::candidate_attempts::CandidateExecutionRelease, RuntimeMemoryError> {
    let authority: Option<(Uuid, Uuid, Uuid)> = sqlx::query_as(
        "SELECT scope_snapshot_id,wave_run_id,wave_unit_id
           FROM candidate_attempts
          WHERE id=$1 AND candidate_id=$2 AND approval_id=$3
            AND operation_id=$4 AND organization_id=$5
            AND candidate_plan_hash=$6 AND stage_worker_run_id=$7
            AND status='running'",
    )
    .bind(input.candidate_attempt.attempt_id)
    .bind(input.candidate_attempt.candidate_id)
    .bind(input.candidate_attempt.approval_id)
    .bind(input.fence.operation_id)
    .bind(input.organization_id)
    .bind(&input.candidate_attempt.candidate_plan_hash)
    .bind(input.fence.worker_run_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
    let Some((scope_snapshot_id, wave_run_id, wave_unit_id)) = authority else {
        return Err(RuntimeMemoryError::IdentityMismatch {
            code: "candidate_control_attempt_identity_mismatch",
        });
    };
    Ok(
        golish_db::repo::candidate_attempts::CandidateExecutionRelease {
            operation_id: input.fence.operation_id,
            scope_snapshot_id,
            wave_run_id,
            wave_unit_id,
            organization_id: input.organization_id,
            attempt_id: input.candidate_attempt.attempt_id,
            worker_run_id: input.fence.worker_run_id,
            stage_execution_id: input.fence.stage_execution_id,
            stage_run_unit_id: input.fence.stage_run_unit_id,
            lease_token: input.fence.lease_token,
            lease_owner: input.lease_owner,
            attempt_epoch: input.fence.attempt_epoch,
            expected_checkpoint_version: input.fence.expected_checkpoint_version,
        },
    )
}

fn candidate_recovery_case_from_db(
    row: golish_db::repo::candidate_recovery::CandidateRecoveryCaseRow,
) -> CandidateRecoveryCaseView {
    CandidateRecoveryCaseView {
        id: row.id,
        operation_id: row.operation_id,
        organization_id: row.organization_id,
        candidate_id: row.candidate_id,
        attempt_id: row.attempt_id,
        action_id: row.action_id,
        worker_run_id: row.worker_run_id,
        reason_code: row.reason_code,
        status: row.status,
        row_version: row.row_version,
        attempt_row_version: row.attempt_row_version,
        opened_at: row.created_at,
    }
}

fn converged_candidate_recovery_from_db(
    row: golish_db::repo::candidate_recovery::ConvergedCandidateRecovery,
) -> ConvergedCandidateRecoveryView {
    ConvergedCandidateRecoveryView {
        recovery_case: candidate_recovery_case_from_db(row.recovery_case),
        terminalized: row.terminalized.map(terminalized_candidate_from_db),
        candidate_reopened: row.candidate_reopened,
        replayed: row.replayed,
    }
}

fn stage_team_plan_from_db(
    row: golish_db::repo::stage_teams::StageTeamPlanRow,
) -> Result<StageTeamPlanView, RuntimeMemoryError> {
    let allowed_roles = row
        .allowed_worker_roles
        .as_array()
        .ok_or_else(|| RuntimeMemoryError::Storage("decode StageTeam allowed roles".to_string()))?
        .iter()
        .map(|value| {
            value.as_str().map(str::to_string).ok_or_else(|| {
                RuntimeMemoryError::Storage("decode StageTeam allowed role".to_string())
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let status = if row.requests_closed_at.is_some() {
        RuntimeStageTeamPlanStatus::Finalizing
    } else {
        RuntimeStageTeamPlanStatus::Active
    };
    Ok(StageTeamPlanView {
        id: row.id,
        operation_id: row.operation_id,
        stage_execution_id: row.stage_execution_id,
        stage_run_unit_id: row.stage_run_unit_id,
        scope_snapshot_id: row.scope_snapshot_id,
        organization_id: row.organization_id,
        stage_kind: row.stage_kind,
        unit_generation: row.unit_generation,
        schema_version: row.schema_version,
        plan_version: row.plan_version,
        plan_sha256: row.plan_hash,
        leader_role: row.leader_role,
        allowed_roles,
        aggregator_kind: row.aggregator_kind,
        aggregator_role: row.aggregator_role,
        max_workers_total: row.max_workers_total,
        max_workers_active: row.max_workers_active,
        dynamic_requests_enabled: row.dynamic_requests_allowed,
        dynamic_request_policy: row.dynamic_request_policy,
        dispatch_epoch: row.dispatch_epoch,
        requests_closed_at: row.requests_closed_at,
        final_submitter_kind: row.final_submitter_kind,
        final_submitter_worker_run_id: row.final_submitter_worker_run_id,
        created_from_stage_spec_hash: row.created_from_stage_spec_hash,
        status,
        row_version: row.row_version,
    })
}

fn stage_work_item_from_db(
    row: golish_db::repo::stage_teams::StageWorkItemRow,
    aggregator_role: Option<&str>,
) -> Result<StageWorkItemView, RuntimeMemoryError> {
    let status = RuntimeStageWorkItemStatus::try_parse(&row.status).ok_or_else(|| {
        RuntimeMemoryError::Storage(format!("decode StageWorkItem status: {}", row.status))
    })?;
    Ok(StageWorkItemView {
        id: row.id,
        stage_team_plan_id: row.team_plan_id,
        stage_run_unit_id: row.stage_run_unit_id,
        organization_id: row.organization_id,
        stable_key: row.stable_key,
        work_item_kind: row.kind,
        role: row.role.clone(),
        input_refs: row.input_refs,
        input_manifest_hash: row.input_manifest_hash,
        priority: row.priority,
        required_for_barrier: row.required_for_barrier,
        is_aggregator: aggregator_role == Some(row.role.as_str()),
        conflict_key: row.conflict_key,
        attempt_policy: row.attempt_policy,
        budget: row.budget,
        output_schema: row.output_schema,
        created_by: row.created_by,
        status,
        row_version: row.row_version,
    })
}

fn claim_stage_work_item_to_db(input: ClaimStageWorkItem) -> ClaimStageWorkItemRow {
    ClaimStageWorkItemRow {
        operation_id: input.operation_id,
        stage_execution_id: input.stage_execution_id,
        stage_run_unit_id: input.stage_run_unit_id,
        stage_team_plan_id: input.stage_team_plan_id,
        lease_owner: input.lease_owner,
        lease_seconds: input.lease_seconds,
        session_id: input.session_id,
        subtask_id: input.subtask_id,
        agent: convert_agent_type_back(input.agent),
        model: input.model,
        provider: input.provider,
        parent_chain_id: input.parent_chain_id,
        initial_chain: input.initial_chain,
        initial_checkpoint: input.initial_checkpoint,
    }
}

fn claimed_stage_work_item_from_db(
    claimed: golish_db::repo::runtime_memory_tx::ClaimedStageWorkItemRow,
) -> Result<ClaimedStageWorkItemView, RuntimeMemoryError> {
    let plan = stage_team_plan_from_db(claimed.plan)?;
    let work_item = stage_work_item_from_db(claimed.work_item, plan.aggregator_role.as_deref())?;
    Ok(ClaimedStageWorkItemView {
        unit: runtime_stage_unit_from_db(claimed.unit)?,
        plan,
        work_item,
        worker: runtime_worker_from_db(claimed.worker)?,
        message_chain_id: claimed.message_chain_id,
    })
}

fn stage_team_barrier_from_db(
    barrier: golish_db::repo::stage_teams::StageTeamBarrierRow,
) -> StageTeamBarrierView {
    StageTeamBarrierView {
        stage_team_plan_id: barrier.stage_team_plan_id,
        dispatch_epoch: barrier.dispatch_epoch,
        requests_closed_at: barrier.requests_closed_at,
        required_work_items: barrier.required_work_items,
        terminal_required_work_items: barrier.terminal_required_work_items,
        live_workers: barrier.live_workers,
        retry_pending_work_items: barrier.retry_pending_work_items,
        recovery_required_workers: barrier.recovery_required_workers,
        missing_outputs: barrier.missing_outputs,
        manifest_sha256: barrier.manifest_hash,
    }
}

fn stage_worker_output_from_db(
    output: golish_db::repo::stage_teams::StageWorkerOutputRow,
) -> Result<StageWorkerOutputView, RuntimeMemoryError> {
    let disposition = StageWorkerOutputDisposition::try_parse(&output.business_disposition)
        .ok_or_else(|| {
            RuntimeMemoryError::Storage(format!(
                "decode StageWorkerOutput disposition: {}",
                output.business_disposition
            ))
        })?;
    Ok(StageWorkerOutputView {
        id: output.id,
        stage_team_plan_id: output.team_plan_id,
        work_item_id: output.work_item_id,
        worker_run_id: output.worker_run_id,
        disposition,
        canonical_output: output.canonical_output,
        fact_refs: output
            .canonical_fact_refs
            .as_array()
            .cloned()
            .ok_or_else(|| RuntimeMemoryError::Storage("decode StageWorkerOutput refs".into()))?,
        evidence_ids: output.evidence_ids,
        checked_empty_units: output
            .checked_empty_cells
            .as_array()
            .cloned()
            .ok_or_else(|| {
                RuntimeMemoryError::Storage("decode StageWorkerOutput empty cells".into())
            })?,
        blocker_code: output.blocker_codes.into_iter().next(),
        output_sha256: output.output_hash,
        created_at: output.created_at,
    })
}

fn stage_worker_request_from_db(
    request: golish_db::repo::stage_teams::StageWorkerRequestRow,
) -> Result<StageWorkerRequestView, RuntimeMemoryError> {
    let (decision, decision_code) = match request.status.as_str() {
        "accepted" => (StageWorkerRequestDecision::Accepted, "accepted".to_string()),
        "rejected" => (
            StageWorkerRequestDecision::Rejected,
            request.decision_reason_code.ok_or_else(|| {
                RuntimeMemoryError::Storage(
                    "decode rejected StageWorkerRequest decision code".to_string(),
                )
            })?,
        ),
        status => {
            return Err(RuntimeMemoryError::Storage(format!(
                "decode StageWorkerRequest status: {status}"
            )))
        }
    };
    Ok(StageWorkerRequestView {
        id: request.id,
        stage_team_plan_id: request.team_plan_id,
        parent_work_item_id: request.parent_work_item_id,
        requested_by_worker_run_id: request.parent_worker_run_id,
        dispatch_epoch: request.dispatch_epoch,
        requested_role: request.requested_role,
        requested_kind: request.request_kind,
        subject_refs: request
            .bounded_subject_refs
            .as_array()
            .cloned()
            .ok_or_else(|| {
                RuntimeMemoryError::Storage(
                    "decode StageWorkerRequest bounded subject refs".to_string(),
                )
            })?,
        reason: request.reason_code,
        output_schema: Value::String(request.expected_output_schema),
        budget_hint: request.budget_hint,
        dedupe_key: request.dedupe_key,
        decision,
        decision_code,
        created_work_item_id: request.accepted_work_item_id,
        request_sha256: request.request_payload_hash,
    })
}

fn runtime_unit_status_to_db(
    status: RuntimeStageUnitStatus,
) -> golish_db::repo::stage_run_units::StageRunUnitStatus {
    use golish_db::repo::stage_run_units::StageRunUnitStatus as Db;
    match status {
        RuntimeStageUnitStatus::Queued => Db::Queued,
        RuntimeStageUnitStatus::Running => Db::Running,
        RuntimeStageUnitStatus::GateBlocked => Db::GateBlocked,
        RuntimeStageUnitStatus::Passed => Db::Passed,
        RuntimeStageUnitStatus::Exhausted => Db::Exhausted,
        RuntimeStageUnitStatus::Superseded => Db::Superseded,
    }
}

fn runtime_worker_status_to_db(
    status: RuntimeWorkerStatus,
) -> golish_db::repo::stage_worker_runs::StageWorkerRunStatus {
    use golish_db::repo::stage_worker_runs::StageWorkerRunStatus as Db;
    match status {
        RuntimeWorkerStatus::Queued => Db::Queued,
        RuntimeWorkerStatus::Running => Db::Running,
        RuntimeWorkerStatus::WaitingBackground => Db::WaitingBackground,
        RuntimeWorkerStatus::GateBlocked => Db::GateBlocked,
        RuntimeWorkerStatus::Passed => Db::Passed,
        RuntimeWorkerStatus::Failed => Db::Failed,
        RuntimeWorkerStatus::Exhausted => Db::Exhausted,
        RuntimeWorkerStatus::Superseded => Db::Superseded,
        RuntimeWorkerStatus::RecoveryRequired => Db::RecoveryRequired,
    }
}

fn runtime_worker_fence_to_db(fence: RuntimeWorkerFence) -> RuntimeMemoryTxFence {
    RuntimeMemoryTxFence {
        operation_id: fence.operation_id,
        stage_execution_id: fence.stage_execution_id,
        stage_run_unit_id: fence.stage_run_unit_id,
        worker_run_id: fence.worker_run_id,
        lease_token: fence.lease_token,
        attempt_epoch: fence.attempt_epoch,
        expected_checkpoint_version: fence.expected_checkpoint_version,
    }
}

fn candidate_acceptance_to_db(
    input: golish_agent_kit::harness::attack_execution::CandidateAcceptance,
) -> Result<golish_db::repo::attack_candidates::CandidateAcceptanceInput, RuntimeMemoryError> {
    use golish_agent_kit::harness::attack_execution::VerificationRiskClass;
    use golish_db::repo::attack_candidates::{
        AcceptedCandidateDraft, CandidateAcceptanceInput, NoCandidateDecision,
    };

    let candidates = input
        .candidates
        .into_iter()
        .map(|candidate| {
            let risk_class = match candidate.risk_class {
                VerificationRiskClass::DeterministicSafe => "deterministic_safe",
                VerificationRiskClass::ActiveSafe => "active_safe",
                VerificationRiskClass::Exploit => "exploit",
            };
            Ok(AcceptedCandidateDraft {
                candidate_id: candidate.candidate_id,
                work_item_id: candidate.work_item_id,
                hypothesis: candidate.hypothesis,
                technique: candidate.technique,
                rationale: candidate.rationale,
                prior_refs: candidate.prior_refs,
                suggested_approach: candidate.suggested_approach,
                priority: candidate.priority,
                execution_plan: serde_json::to_value(candidate.execution_plan).map_err(
                    |error| {
                        RuntimeMemoryError::Storage(format!(
                            "serialize immutable Candidate plan: {error}"
                        ))
                    },
                )?,
                candidate_plan_hash: candidate.candidate_plan_hash,
                risk_class: risk_class.to_string(),
                evidence_ids: candidate.evidence_ids,
            })
        })
        .collect::<Result<Vec<_>, RuntimeMemoryError>>()?;
    Ok(CandidateAcceptanceInput {
        wave_run_id: input.wave_run_id,
        wave_unit_id: input.wave_unit_id,
        manifest_hash: input.manifest_hash,
        expected_work_item_ids: input.expected_work_item_ids,
        candidates,
        no_candidate_decisions: input
            .no_candidate_decisions
            .into_iter()
            .map(|decision| NoCandidateDecision {
                work_item_id: decision.work_item_id,
                reason_code: decision.reason_code,
                detail: decision.detail,
                evidence_ids: decision.evidence_ids,
            })
            .collect(),
    })
}

fn canonical_fact_key_to_db(
    key: CanonicalFactKey,
) -> golish_db::repo::canonical_fact_refs::CanonicalFactKey {
    use golish_db::repo::canonical_fact_refs::CanonicalFactKey as Db;
    match key {
        CanonicalFactKey::Organization { organization_id } => Db::Organization { organization_id },
        CanonicalFactKey::Target { target_id } => Db::Target { target_id },
        CanonicalFactKey::TargetAsset { target_asset_id } => Db::TargetAsset { target_asset_id },
        CanonicalFactKey::DnsRecord {
            organization_id,
            domain,
            record_type,
            value,
        } => Db::DnsRecord {
            organization_id,
            domain,
            record_type,
            value,
        },
        CanonicalFactKey::ApiEndpoint { api_endpoint_id } => Db::ApiEndpoint { api_endpoint_id },
        CanonicalFactKey::DirectoryEntry { directory_entry_id } => {
            Db::DirectoryEntry { directory_entry_id }
        }
        CanonicalFactKey::JsAnalysisResult {
            js_analysis_result_id,
        } => Db::JsAnalysisResult {
            js_analysis_result_id,
        },
        CanonicalFactKey::Fingerprint { fingerprint_id } => Db::Fingerprint { fingerprint_id },
        CanonicalFactKey::TechniqueOutcome {
            organization_id,
            run_id,
            asset,
            technique,
        } => Db::TechniqueOutcome {
            organization_id,
            run_id,
            asset,
            technique,
        },
        CanonicalFactKey::TechniqueOutcomeSet {
            organization_id,
            run_id,
            stage,
            terminal_cell_count,
            outcome_set_sha256,
        } => Db::TechniqueOutcomeSet {
            organization_id,
            run_id,
            stage,
            terminal_cell_count,
            outcome_set_sha256,
        },
        CanonicalFactKey::AttackCandidateWorkItem { work_item_id } => {
            Db::AttackCandidateWorkItem { work_item_id }
        }
        CanonicalFactKey::Finding { finding_id } => Db::Finding { finding_id },
    }
}

fn canonical_fact_key_from_db(
    key: golish_db::repo::canonical_fact_refs::CanonicalFactKey,
) -> CanonicalFactKey {
    use golish_db::repo::canonical_fact_refs::CanonicalFactKey as Db;
    match key {
        Db::Organization { organization_id } => CanonicalFactKey::Organization { organization_id },
        Db::Target { target_id } => CanonicalFactKey::Target { target_id },
        Db::TargetAsset { target_asset_id } => CanonicalFactKey::TargetAsset { target_asset_id },
        Db::DnsRecord {
            organization_id,
            domain,
            record_type,
            value,
        } => CanonicalFactKey::DnsRecord {
            organization_id,
            domain,
            record_type,
            value,
        },
        Db::ApiEndpoint { api_endpoint_id } => CanonicalFactKey::ApiEndpoint { api_endpoint_id },
        Db::DirectoryEntry { directory_entry_id } => {
            CanonicalFactKey::DirectoryEntry { directory_entry_id }
        }
        Db::JsAnalysisResult {
            js_analysis_result_id,
        } => CanonicalFactKey::JsAnalysisResult {
            js_analysis_result_id,
        },
        Db::Fingerprint { fingerprint_id } => CanonicalFactKey::Fingerprint { fingerprint_id },
        Db::TechniqueOutcome {
            organization_id,
            run_id,
            asset,
            technique,
        } => CanonicalFactKey::TechniqueOutcome {
            organization_id,
            run_id,
            asset,
            technique,
        },
        Db::TechniqueOutcomeSet {
            organization_id,
            run_id,
            stage,
            terminal_cell_count,
            outcome_set_sha256,
        } => CanonicalFactKey::TechniqueOutcomeSet {
            organization_id,
            run_id,
            stage,
            terminal_cell_count,
            outcome_set_sha256,
        },
        Db::AttackCandidateWorkItem { work_item_id } => {
            CanonicalFactKey::AttackCandidateWorkItem { work_item_id }
        }
        Db::Finding { finding_id } => CanonicalFactKey::Finding { finding_id },
    }
}

fn canonical_fact_ref_from_db(
    canonical_ref: golish_db::repo::canonical_fact_refs::CanonicalFactRef,
) -> CanonicalFactRef {
    CanonicalFactRef {
        key: canonical_fact_key_from_db(canonical_ref.key),
        organization_id: canonical_ref.organization_id,
        observed_at: canonical_ref.observed_at,
        content_sha256: canonical_ref.content_sha256,
        evidence_ids: canonical_ref.evidence_ids,
    }
}

fn stage_handoff_from_db(
    row: golish_db::repo::stage_handoffs::StageHandoffRow,
) -> RuntimeStageHandoffView {
    RuntimeStageHandoffView {
        id: row.id,
        operation_id: row.operation_id,
        organization_id: row.organization_id,
        scope_snapshot_id: row.scope_snapshot_id,
        from_stage_kind: row.from_stage_kind,
        stage_execution_id: row.stage_execution_id,
        source_stage_run_unit_id: row.source_stage_run_unit_id,
        deliverable_submission_id: Some(row.deliverable_submission_id),
        authority_kind: "deliverable_final_seal".to_string(),
        scope_hash: row.scope_hash,
        payload: row.payload,
        payload_sha256: row.payload_sha256,
        evidence_ids: row.evidence_ids,
        coverage_watermark: row.coverage_watermark,
        unit_gate_decision_hash: row.unit_gate_decision_hash,
        aggregate_pass_token_hash: row.aggregate_pass_token_hash,
        gate_passed_at: row.gate_passed_at,
        schema_version: row.schema_version,
    }
}

fn final_sealed_stage_handoff_from_db(
    row: golish_db::repo::stage_handoffs::FinalSealedStageHandoffRow,
) -> RuntimeStageHandoffView {
    RuntimeStageHandoffView {
        id: row.id,
        operation_id: row.operation_id,
        organization_id: row.organization_id,
        scope_snapshot_id: row.scope_snapshot_id,
        from_stage_kind: row.from_stage_kind,
        stage_execution_id: row.stage_execution_id,
        source_stage_run_unit_id: row.source_stage_run_unit_id,
        deliverable_submission_id: row.deliverable_submission_id,
        authority_kind: row.authority_kind,
        scope_hash: row.scope_hash,
        payload: row.payload,
        payload_sha256: row.payload_sha256,
        evidence_ids: row.evidence_ids,
        coverage_watermark: row.coverage_watermark,
        unit_gate_decision_hash: row.unit_gate_decision_hash,
        aggregate_pass_token_hash: row.aggregate_pass_token_hash,
        gate_passed_at: row.gate_passed_at,
        schema_version: row.schema_version,
    }
}

fn finalize_unit_pass_to_db(
    input: FinalizeUnitPass,
) -> Result<FinalizeUnitPassRow, RuntimeMemoryError> {
    let candidate_acceptance = input
        .candidate_acceptance
        .map(candidate_acceptance_to_db)
        .transpose()?;
    Ok(FinalizeUnitPassRow {
        fence: runtime_worker_fence_to_db(input.fence),
        deliverable_submission_id: input.deliverable_submission_id,
        expected_unit_status: runtime_unit_status_to_db(input.expected_unit_status),
        expected_unit_row_version: input.expected_unit_row_version,
        scope_hash: input.scope_hash,
        gate_decision: input.gate_decision,
        gate_decision_hash: input.gate_decision_hash,
        aggregate_pass_token_hash: input.aggregate_pass_token_hash,
        canonical_fact_keys: input
            .canonical_fact_keys
            .into_iter()
            .map(canonical_fact_key_to_db)
            .collect(),
        typed_claims: input.typed_claims,
        coverage_watermark: input.coverage_watermark,
        evidence_ids: input.evidence_ids,
        terminal_checkpoint: input.terminal_checkpoint,
        candidate_acceptance,
    })
}

fn finalized_unit_pass_from_db(
    finalized: golish_db::repo::runtime_memory_tx::FinalizedUnitPassRow,
) -> Result<FinalizedUnitPass, RuntimeMemoryError> {
    Ok(FinalizedUnitPass {
        unit: runtime_stage_unit_from_db(finalized.unit)?,
        worker: runtime_worker_from_db(finalized.worker)?,
        handoff: stage_handoff_from_db(finalized.handoff),
        canonical_fact_refs: finalized
            .canonical_fact_refs
            .into_iter()
            .map(canonical_fact_ref_from_db)
            .collect(),
        replayed: finalized.replayed,
    })
}

fn attack_shadow_complete_from_db(
    record: &golish_db::repo::attack_execution_shadow::AttackShadowCompleteReadRow,
) -> Result<CompleteAttackRead, RuntimeMemoryError> {
    let decisions = record
        .decisions
        .iter()
        .map(|decision| {
            let kind = match decision.kind.as_str() {
                "candidate" => AttackDecisionSemanticKind::Candidate,
                "no_candidate" => AttackDecisionSemanticKind::NoCandidate,
                _ => {
                    return Err(RuntimeMemoryError::Storage(
                        "ATTACK_READ_DECISION_INVALID: unknown persisted decision kind".into(),
                    ));
                }
            };
            AttackDecisionSemantic::try_new(
                decision.work_item_key.clone(),
                kind,
                decision.semantic_hash.clone(),
            )
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    CompleteAttackRead::try_new(
        decisions,
        AttackReviewCounts::new(
            record.review_counts.wave_unit_count,
            record.review_counts.review_closed_unit_count,
            record.review_counts.candidate_decision_count,
            record.review_counts.no_candidate_decision_count,
        ),
    )
    .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))
}

fn select_attack_shadow_sample(
    sample: &golish_db::repo::attack_execution_shadow::AttackExecutionShadowSampleRow,
) -> Result<AttackReadSelection, RuntimeMemoryError> {
    let contract = match sample.contract.as_str() {
        "legacy" => golish_core::AttackExecutionContract::Legacy,
        "dual_write_read_legacy" => golish_core::AttackExecutionContract::DualWriteReadLegacy,
        "dual_write_read_v2_fallback" => {
            golish_core::AttackExecutionContract::DualWriteReadV2Fallback
        }
        "v2_only" => golish_core::AttackExecutionContract::V2Only,
        _ => {
            return Err(RuntimeMemoryError::Storage(
                "unknown persisted attack execution contract".into(),
            ));
        }
    };
    let legacy = sample
        .legacy_record
        .as_ref()
        .map(attack_shadow_complete_from_db)
        .transpose()?;
    let v2 = match &sample.v2_record {
        golish_db::repo::attack_execution_shadow::AttackShadowV2ReadRow::Complete(record) => {
            V2AttackRead::Complete(attack_shadow_complete_from_db(record)?)
        }
        golish_db::repo::attack_execution_shadow::AttackShadowV2ReadRow::Missing => {
            V2AttackRead::Missing
        }
        golish_db::repo::attack_execution_shadow::AttackShadowV2ReadRow::Incomplete => {
            V2AttackRead::Incomplete
        }
    };
    select_attack_read(contract, legacy, v2)
        .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))
}

fn shadow_comparison_str(comparison: AttackShadowComparison) -> &'static str {
    match comparison {
        AttackShadowComparison::Match => "match",
        AttackShadowComparison::Mismatch => "mismatch",
        AttackShadowComparison::V2Missing => "v2_missing",
    }
}

fn attack_read_source_str(source: AttackReadSource) -> &'static str {
    match source {
        AttackReadSource::Legacy => "legacy",
        AttackReadSource::V2 => "v2",
        AttackReadSource::LegacyFallback => "legacy_fallback",
    }
}

fn runtime_record_source_from_db(
    source: golish_db::repo::runtime_memory_tx::RuntimeMemoryRecordSource,
) -> RuntimeMemoryRecordSource {
    use golish_db::repo::runtime_memory_tx::RuntimeMemoryRecordSource as Db;
    match source {
        Db::Legacy => RuntimeMemoryRecordSource::Legacy,
        Db::V2 => RuntimeMemoryRecordSource::V2,
        Db::LegacyFallback => RuntimeMemoryRecordSource::LegacyFallback,
    }
}

fn runtime_record_source_to_db(
    source: RuntimeMemoryRecordSource,
) -> golish_db::repo::runtime_memory_tx::RuntimeMemoryRecordSource {
    use golish_db::repo::runtime_memory_tx::RuntimeMemoryRecordSource as Db;
    match source {
        RuntimeMemoryRecordSource::Legacy => Db::Legacy,
        RuntimeMemoryRecordSource::V2 => Db::V2,
        RuntimeMemoryRecordSource::LegacyFallback => Db::LegacyFallback,
    }
}

fn stage_execution_from_db(
    row: golish_db::repo::stage_runs::StageRunRow,
) -> Result<StageExecution, RuntimeMemoryError> {
    let stage = StageKind::try_parse(&row.stage_kind).ok_or_else(|| {
        RuntimeMemoryError::Storage(format!(
            "decode persisted stage execution kind: {}",
            row.stage_kind
        ))
    })?;
    let status = StageExecutionStatus::try_parse(&row.status).ok_or_else(|| {
        RuntimeMemoryError::Storage(format!(
            "decode persisted stage execution status: {}",
            row.status
        ))
    })?;
    Ok(StageExecution {
        id: row.id,
        operation_id: row.operation_id,
        stage,
        status,
    })
}

fn project_scope_registration_from_db(
    row: golish_db::repo::project_scopes::ProjectScopeRow,
) -> ProjectScopeRegistration {
    ProjectScopeRegistration {
        project_scope_id: row.project_scope_id,
        canonical_project_path: row.canonical_project_path,
        path_sha256: row.path_sha256,
        row_version: row.row_version,
    }
}

fn operation_state_view_from_db(
    row: golish_db::repo::operation_state::OperationStateRow,
) -> Result<OperationStateView, RuntimeMemoryError> {
    let runtime_memory_contract =
        RuntimeMemoryContract::try_from(row.runtime_memory_contract.as_str()).map_err(|error| {
            RuntimeMemoryError::Storage(format!(
                "decode persisted runtime-memory contract: {error}"
            ))
        })?;
    Ok(OperationStateView {
        operation_id: row.operation_id,
        profile: row.profile,
        current_stage: row.current_stage,
        runtime_memory_contract,
        project_scope_id: row.project_scope_id,
        engagement_org_id: row.engagement_org_id,
        state_blob: row.state_blob,
        stage_started_at: row.stage_started_at,
    })
}

fn created_runtime_operation_from_db(
    created: CreatedRuntimeOperationRow,
    expected_project_scope_id: Uuid,
    expected_initial_stage_execution_id: Uuid,
) -> Result<CreatedRuntimeOperation, RuntimeMemoryError> {
    if created.task.id != created.operation.operation_id {
        return Err(RuntimeMemoryError::IdentityMismatch {
            code: "runtime_operation_task_identity_mismatch",
        });
    }
    if created.operation.project_scope_id != Some(expected_project_scope_id) {
        return Err(RuntimeMemoryError::IdentityMismatch {
            code: "runtime_operation_project_scope_mismatch",
        });
    }
    if created.initial_stage_execution_id != expected_initial_stage_execution_id {
        return Err(RuntimeMemoryError::IdentityMismatch {
            code: "runtime_operation_initial_stage_execution_mismatch",
        });
    }
    let task = TaskView {
        id: created.task.id,
        input: created.task.input,
        status: convert_task_status(created.task.status),
        result: created.task.result,
    };
    let operation = operation_state_view_from_db(created.operation)?;
    Ok(CreatedRuntimeOperation {
        task,
        operation,
        initial_stage_execution_id: created.initial_stage_execution_id,
    })
}

#[async_trait]
impl RuntimeMemoryRepository for GolishDbRepoProvider {
    async fn project_scope_register_first_open(
        &self,
        canonical_path: &str,
        path_sha256: &str,
    ) -> Result<ProjectScopeRegistration, RuntimeMemoryError> {
        golish_db::repo::project_scopes::register_first_open(
            &self.pool,
            canonical_path,
            path_sha256,
        )
        .await
        .map(project_scope_registration_from_db)
        .map_err(runtime_memory_error_from_db)
    }

    async fn project_scope_rename(
        &self,
        project_scope_id: Uuid,
        expected_old_path: &str,
        expected_row_version: i64,
        new_path: &str,
        new_path_sha256: &str,
    ) -> Result<ProjectScopeRegistration, RuntimeMemoryError> {
        golish_db::repo::project_scopes::rename(
            &self.pool,
            project_scope_id,
            expected_old_path,
            expected_row_version,
            new_path,
            new_path_sha256,
        )
        .await
        .map(project_scope_registration_from_db)
        .map_err(runtime_memory_error_from_db)
    }

    async fn create_runtime_operation(
        &self,
        input: CreateRuntimeOperation,
    ) -> Result<CreatedRuntimeOperation, RuntimeMemoryError> {
        let expected_project_scope_id = input.project_scope.project_scope_id;
        let expected_initial_stage_execution_id = input.initial_stage_execution_id;
        let cli_scope =
            input.cli_scope.map(
                |scope| golish_db::repo::runtime_memory_tx::CliRuntimeScopeRow {
                    root_organization_id: scope.root_organization_id,
                    include_subsidiaries: scope.include_subsidiaries,
                    subsidiary_threshold: scope.subsidiary_threshold,
                    units: scope
                        .units
                        .into_iter()
                        .map(
                            |unit| golish_db::repo::runtime_memory_tx::CliRuntimeScopeUnitRow {
                                organization_id: unit.organization_id,
                                parent_organization_id: unit.parent_organization_id,
                                organization_name: unit.organization_name,
                                depth: unit.depth,
                                ordinal: unit.ordinal,
                                ownership_percent: unit.ownership_percent,
                                approval_source: unit.approval_source,
                            },
                        )
                        .collect(),
                },
            );
        let stage_fork =
            input.stage_fork.map(
                |fork| golish_db::repo::runtime_memory_tx::StageForkCreateRow {
                    source_operation_id: fork.source_operation_id,
                    source_scope_snapshot_id: fork.source_scope_snapshot_id,
                    entry_stage: fork.entry_stage,
                    terminal_stage: fork.terminal_stage,
                    adopted_stage_kinds: fork.adopted_stage_kinds,
                },
            );
        let row = CreateRuntimeOperationRow {
            operation_id: input.operation_id,
            initial_stage_execution_id: expected_initial_stage_execution_id,
            session_id: input.session_id,
            title: input.title,
            input: input.input,
            profile: input.profile,
            entry_stage: input.entry_stage,
            project_scope_id: expected_project_scope_id,
            cli_scope,
        };
        let created = match stage_fork.as_ref() {
            Some(stage_fork) => {
                golish_db::repo::runtime_memory_tx::create_runtime_operation_with_stage_fork(
                    &self.pool, &row, stage_fork,
                )
                .await
            }
            None => {
                golish_db::repo::runtime_memory_tx::create_runtime_operation(&self.pool, &row).await
            }
        }
        .map_err(runtime_memory_error_from_db)?;
        created_runtime_operation_from_db(
            created,
            expected_project_scope_id,
            expected_initial_stage_execution_id,
        )
    }

    async fn active_stage_execution(
        &self,
        operation_id: Uuid,
    ) -> Result<StageExecution, RuntimeMemoryError> {
        golish_db::repo::stage_runs::get_exact_active_for_operation(&self.pool, operation_id)
            .await
            .map_err(runtime_memory_error_from_db)
            .and_then(stage_execution_from_db)
    }

    async fn transition_stage_execution(
        &self,
        input: TransitionStageExecution,
    ) -> Result<TransitionedStageExecution, RuntimeMemoryError> {
        let transitioned = golish_db::repo::runtime_memory_tx::transition_stage_execution(
            &self.pool,
            &TransitionStageExecutionRow {
                operation_id: input.operation_id,
                current_stage_execution_id: input.current_stage_execution_id,
                next_stage_execution_id: input.next_stage_execution_id,
                next_stage: input.next_stage.as_str().to_string(),
            },
        )
        .await
        .map_err(runtime_memory_error_from_db)?;
        Ok(TransitionedStageExecution {
            previous: stage_execution_from_db(transitioned.previous_stage_execution)?,
            current: stage_execution_from_db(transitioned.current_stage_execution)?,
        })
    }

    async fn complete_terminal_stage_execution(
        &self,
        input: CompleteTerminalStageExecution,
    ) -> Result<StageExecution, RuntimeMemoryError> {
        golish_db::repo::runtime_memory_tx::complete_terminal_stage_execution(
            &self.pool,
            &CompleteTerminalStageExecutionRow {
                operation_id: input.operation_id,
                current_stage_execution_id: input.current_stage_execution_id,
                terminal_stage: input.terminal_stage.as_str().to_string(),
                task_result: input.task_result,
            },
        )
        .await
        .map_err(runtime_memory_error_from_db)
        .and_then(stage_execution_from_db)
    }

    async fn runtime_memory_contract_for_operation(
        &self,
        operation_id: Uuid,
    ) -> Result<RuntimeMemoryContract, RuntimeMemoryError> {
        let operation = golish_db::repo::operation_state::get(&self.pool, operation_id)
            .await
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?
            .ok_or(RuntimeMemoryError::Missing {
                entity: "operation_state",
            })?;
        RuntimeMemoryContract::try_from(operation.runtime_memory_contract.as_str())
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))
    }

    async fn attack_execution_contract_for_operation(
        &self,
        operation_id: Uuid,
    ) -> Result<golish_core::AttackExecutionContract, RuntimeMemoryError> {
        golish_db::repo::operation_state::get_attack_execution_contract(&self.pool, operation_id)
            .await
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?
            .ok_or(RuntimeMemoryError::Missing {
                entity: "operation_state",
            })
    }

    async fn attack_v2_wave_authority_for_operation(
        &self,
        operation_id: Uuid,
    ) -> Result<AttackV2WaveAuthorityView, RuntimeMemoryError> {
        golish_db::repo::attack_waves::load_current_authority(&self.pool, operation_id)
            .await
            .map(attack_wave_authority_from_db)
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))
    }

    async fn insert_stage_deliverable_submission(
        &self,
        input: NewStageDeliverableSubmission,
    ) -> Result<PersistedStageDeliverableSubmission, RuntimeMemoryError> {
        golish_db::repo::stage_deliverable_submissions::insert(
            &self.pool,
            &golish_db::repo::stage_deliverable_submissions::NewStageDeliverableSubmission {
                operation_id: input.operation_id,
                stage_execution_id: input.stage_execution_id,
                stage_run_unit_id: input.stage_run_unit_id,
                worker_run_id: input.worker_run_id,
                organization_id: input.organization_id,
                tool_call_record_id: input.tool_call_record_id,
                tool_request_id: input.tool_request_id,
                stage_kind: input.stage_kind,
                attempt_epoch: input.attempt_epoch,
                lease_token: input.lease_token,
                canonical_payload_json: input.canonical_deliverable_json,
                payload_sha256: input.payload_sha256,
            },
        )
        .await
        .map(stage_submission_from_db)
        .map_err(stage_submission_error_from_db)
    }

    async fn load_stage_deliverable_submission(
        &self,
        deliverable_submission_id: Uuid,
        operation_id: Uuid,
        stage_execution_id: Uuid,
    ) -> Result<Option<PersistedStageDeliverableSubmission>, RuntimeMemoryError> {
        golish_db::repo::stage_deliverable_submissions::load_scoped(
            &self.pool,
            deliverable_submission_id,
            operation_id,
            stage_execution_id,
        )
        .await
        .map(|row| row.map(stage_submission_from_db))
        .map_err(stage_submission_error_from_db)
    }

    async fn finalize_scoping_scope(
        &self,
        input: FinalizeScopingScope,
    ) -> Result<FinalizedScopingScope, RuntimeMemoryError> {
        golish_db::repo::runtime_memory_tx::finalize_scoping_scope(
            &self.pool,
            &FinalizeScopingScopeRow {
                operation_id: input.operation_id,
                project_scope_id: input.project_scope_id,
                stage_execution_id: input.stage_execution_id,
                root_organization_id: input.root_organization_id,
                deliverable_submission_id: input.deliverable_submission_id,
                scope_snapshot_id: input.scope_snapshot_id,
                scoping_root_unit_id: input.scoping_root_unit_id,
            },
        )
        .await
        .map(finalized_scoping_scope_from_db)
        .map_err(runtime_memory_error_from_db)
    }

    async fn seed_stage_runtime(
        &self,
        input: SeedStageRuntime,
    ) -> Result<Vec<SeededStageRuntime>, RuntimeMemoryError> {
        golish_db::repo::runtime_memory_tx::seed_stage_runtime(
            &self.pool,
            &SeedStageRuntimeRow {
                operation_id: input.operation_id,
                stage_execution_id: input.stage_execution_id,
                stage_kind: input.stage_kind,
                unit_generation: input.unit_generation,
                specialist: input.specialist,
                worker_generation: input.worker_generation,
                work_item_kind: input.work_item_kind,
                work_item_key: input.work_item_key,
                agent_path_prefix: input.agent_path_prefix,
                organization_ids: input.organization_ids,
            },
        )
        .await
        .map_err(runtime_memory_error_from_db)?
        .into_iter()
        .map(|seeded| {
            Ok(SeededStageRuntime {
                unit: runtime_stage_unit_from_db(seeded.unit)?,
                worker: runtime_worker_from_db(seeded.worker)?,
                organization_name: seeded.organization_name,
                scope_hash: seeded.scope_hash,
            })
        })
        .collect()
    }

    async fn seed_stage_team_runtime(
        &self,
        input: SeedStageTeamRuntime,
    ) -> Result<Vec<SeededStageTeamRuntime>, RuntimeMemoryError> {
        let base = SeedStageRuntimeRow {
            operation_id: input.base.operation_id,
            stage_execution_id: input.base.stage_execution_id,
            stage_kind: input.base.stage_kind,
            unit_generation: input.base.unit_generation,
            specialist: input.base.specialist,
            worker_generation: input.base.worker_generation,
            work_item_kind: input.base.work_item_kind,
            work_item_key: input.base.work_item_key,
            agent_path_prefix: input.base.agent_path_prefix,
            organization_ids: input.base.organization_ids,
        };
        let seeded = golish_db::repo::runtime_memory_tx::seed_stage_team_runtime(
            &self.pool,
            &SeedStageTeamRuntimeRow {
                base,
                plan: StageTeamPlanSeedRow {
                    schema_version: input.plan.schema_version,
                    plan_version: input.plan.plan_version,
                    plan_hash: input.plan.plan_sha256,
                    leader_role: input.plan.leader_role,
                    allowed_roles: input.plan.allowed_roles,
                    aggregator_kind: input.plan.aggregator_kind,
                    aggregator_role: input.plan.aggregator_role,
                    max_workers_total: input.plan.max_workers_total,
                    max_workers_active: input.plan.max_workers_active,
                    dynamic_requests_enabled: input.plan.dynamic_requests_enabled,
                    dynamic_request_policy: input.plan.dynamic_request_policy,
                    final_submitter_kind: input.plan.final_submitter_kind,
                    created_from_stage_spec_hash: input.plan.created_from_stage_spec_hash,
                },
                work_items: input
                    .work_items
                    .into_iter()
                    .map(|item| StageWorkItemSeedRow {
                        stable_key: item.stable_key,
                        work_item_kind: item.work_item_kind,
                        role: item.role,
                        input_manifest: item.input_manifest,
                        input_manifest_hash: item.input_sha256,
                        conflict_key: item.conflict_key,
                        priority: item.priority,
                        required_for_barrier: item.required_for_barrier,
                        is_aggregator: item.is_aggregator,
                        attempt_policy: item.attempt_policy,
                        budget: item.budget,
                        output_schema: item.output_schema,
                        created_by: item.created_by,
                    })
                    .collect(),
            },
        )
        .await
        .map_err(runtime_memory_error_from_db)?;
        seeded
            .into_iter()
            .map(|seeded| {
                let plan = stage_team_plan_from_db(seeded.plan)?;
                let aggregator_role = plan.aggregator_role.clone();
                let work_items = seeded
                    .work_items
                    .into_iter()
                    .map(|item| stage_work_item_from_db(item, aggregator_role.as_deref()))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(SeededStageTeamRuntime {
                    unit: runtime_stage_unit_from_db(seeded.unit)?,
                    plan,
                    work_items,
                    primary_worker: None,
                    organization_name: seeded.organization_name,
                    scope_hash: seeded.scope_hash,
                    replayed: seeded.replayed,
                })
            })
            .collect()
    }

    async fn claim_stage_work_item(
        &self,
        input: ClaimStageWorkItem,
    ) -> Result<Option<ClaimedStageWorkItemView>, RuntimeMemoryError> {
        golish_db::repo::runtime_memory_tx::claim_stage_work_item(
            &self.pool,
            &claim_stage_work_item_to_db(input),
        )
        .await
        .map_err(runtime_memory_error_from_db)?
        .map(claimed_stage_work_item_from_db)
        .transpose()
    }

    async fn claim_stage_team_leader(
        &self,
        input: ClaimStageTeamLeader,
    ) -> Result<Option<ClaimedStageWorkItemView>, RuntimeMemoryError> {
        golish_db::repo::runtime_memory_tx::claim_stage_team_leader(
            &self.pool,
            &ClaimStageTeamLeaderRow {
                claim: claim_stage_work_item_to_db(input.claim),
            },
        )
        .await
        .map_err(runtime_memory_error_from_db)?
        .map(claimed_stage_work_item_from_db)
        .transpose()
    }

    async fn park_stage_team_leader(
        &self,
        input: ParkStageTeamLeader,
    ) -> Result<ParkedStageTeamLeaderView, RuntimeMemoryError> {
        let parked = golish_db::repo::runtime_memory_tx::park_stage_team_leader(
            &self.pool,
            &ParkStageTeamLeaderRow {
                fence: runtime_worker_fence_to_db(input.fence),
                stage_team_plan_id: input.stage_team_plan_id,
                leader_work_item_id: input.leader_work_item_id,
                expected_work_item_row_version: input.expected_work_item_row_version,
                checkpoint: input.checkpoint,
            },
        )
        .await
        .map_err(runtime_memory_error_from_db)?;
        let plan = stage_team_plan_from_db(parked.plan)?;
        Ok(ParkedStageTeamLeaderView {
            work_item: stage_work_item_from_db(parked.work_item, plan.aggregator_role.as_deref())?,
            worker: runtime_worker_from_db(parked.worker)?,
            dependency_count: parked.dependency_count,
            plan,
        })
    }

    async fn bind_stage_team_leader_final_submitter(
        &self,
        input: BindStageTeamLeaderFinalSubmitter,
    ) -> Result<BoundStageTeamLeaderFinalSubmitterView, RuntimeMemoryError> {
        let bound = golish_db::repo::runtime_memory_tx::bind_stage_team_leader_final_submitter(
            &self.pool,
            &BindStageTeamLeaderFinalSubmitterRow {
                fence: runtime_worker_fence_to_db(input.fence),
                stage_team_plan_id: input.stage_team_plan_id,
                leader_work_item_id: input.leader_work_item_id,
                expected_plan_row_version: input.expected_plan_row_version,
                expected_dispatch_epoch: input.expected_dispatch_epoch,
                expected_manifest_hash: input.expected_manifest_sha256,
            },
        )
        .await
        .map_err(runtime_memory_error_from_db)?;
        Ok(BoundStageTeamLeaderFinalSubmitterView {
            plan: stage_team_plan_from_db(bound.plan)?,
            barrier: stage_team_barrier_from_db(bound.barrier),
            replayed: bound.replayed,
        })
    }

    async fn reopen_stage_team_leader_after_gate_block(
        &self,
        input: ReopenStageTeamLeaderAfterGateBlock,
    ) -> Result<ReopenedStageTeamLeaderAfterGateBlockView, RuntimeMemoryError> {
        let reopened =
            golish_db::repo::runtime_memory_tx::reopen_stage_team_leader_after_gate_block(
                &self.pool,
                &ReopenStageTeamLeaderAfterGateBlockRow {
                    request_id: input.request_id,
                    fence: runtime_worker_fence_to_db(input.fence),
                    stage_team_plan_id: input.stage_team_plan_id,
                    leader_work_item_id: input.leader_work_item_id,
                    deliverable_submission_id: input.deliverable_submission_id,
                    expected_dispatch_epoch: input.expected_dispatch_epoch,
                    expected_manifest_hash: input.expected_manifest_sha256,
                    gate_decision_hash: input.gate_decision_sha256,
                    gap_manifest: input.gap_manifest,
                    gap_manifest_hash: input.gap_manifest_sha256,
                    checkpoint: input.checkpoint,
                },
            )
            .await
            .map_err(runtime_memory_error_from_db)?;
        let plan = stage_team_plan_from_db(reopened.plan)?;
        Ok(ReopenedStageTeamLeaderAfterGateBlockView {
            unit: runtime_stage_unit_from_db(reopened.unit)?,
            gap_id: reopened.gap.map(|gap| gap.id),
            repair_generation: reopened.repair_generation,
            fuel_exhausted: reopened.fuel_exhausted,
            leader_work_item: stage_work_item_from_db(
                reopened.leader_work_item,
                plan.aggregator_role.as_deref(),
            )?,
            leader_worker: runtime_worker_from_db(reopened.leader_worker)?,
            replayed: reopened.replayed,
            plan,
        })
    }

    async fn request_stage_worker(
        &self,
        input: RequestStageWorker,
    ) -> Result<RequestedStageWorkerView, RuntimeMemoryError> {
        let requested = golish_db::repo::runtime_memory_tx::request_stage_worker(
            &self.pool,
            &RequestStageWorkerRow {
                fence: runtime_worker_fence_to_db(input.fence),
                stage_team_plan_id: input.stage_team_plan_id,
                parent_work_item_id: input.parent_work_item_id,
                expected_dispatch_epoch: input.expected_dispatch_epoch,
                requested_role: input.requested_role,
                requested_kind: input.requested_kind,
                subject_refs: input.subject_refs,
                reason: input.reason,
                output_schema: input.output_schema,
                budget_hint: input.budget_hint,
                dedupe_key: input.dedupe_key,
                request_sha256: input.request_sha256,
            },
        )
        .await
        .map_err(runtime_memory_error_from_db)?;
        let plan = stage_team_plan_from_db(requested.plan)?;
        let work_item = requested
            .work_item
            .map(|item| stage_work_item_from_db(item, plan.aggregator_role.as_deref()))
            .transpose()?;
        Ok(RequestedStageWorkerView {
            request: stage_worker_request_from_db(requested.request)?,
            work_item,
            replayed: requested.replayed,
        })
    }

    async fn close_stage_request_epoch(
        &self,
        input: CloseStageRequestEpoch,
    ) -> Result<ClosedStageRequestEpochView, RuntimeMemoryError> {
        let closed = golish_db::repo::runtime_memory_tx::close_stage_request_epoch(
            &self.pool,
            &CloseStageRequestEpochRow {
                operation_id: input.operation_id,
                stage_execution_id: input.stage_execution_id,
                stage_run_unit_id: input.stage_run_unit_id,
                stage_team_plan_id: input.stage_team_plan_id,
                expected_dispatch_epoch: input.expected_dispatch_epoch,
                expected_plan_row_version: input.expected_plan_row_version,
            },
        )
        .await
        .map_err(runtime_memory_error_from_db)?;
        Ok(ClosedStageRequestEpochView {
            plan: stage_team_plan_from_db(closed.plan)?,
            barrier: stage_team_barrier_from_db(closed.barrier),
            replayed: closed.replayed,
        })
    }

    async fn load_stage_team_barrier(
        &self,
        input: LoadStageTeamBarrier,
    ) -> Result<StageTeamBarrierView, RuntimeMemoryError> {
        golish_db::repo::runtime_memory_tx::load_stage_team_barrier(
            &self.pool,
            &LoadStageTeamBarrierRow {
                operation_id: input.operation_id,
                stage_execution_id: input.stage_execution_id,
                stage_run_unit_id: input.stage_run_unit_id,
                stage_team_plan_id: input.stage_team_plan_id,
                dispatch_epoch: input.dispatch_epoch,
            },
        )
        .await
        .map(stage_team_barrier_from_db)
        .map_err(runtime_memory_error_from_db)
    }

    async fn load_stage_team_outputs(
        &self,
        input: LoadStageTeamBarrier,
    ) -> Result<Vec<StageWorkerOutputView>, RuntimeMemoryError> {
        let mut connection = self
            .pool
            .acquire()
            .await
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        let plan = golish_db::repo::stage_teams::get_plan_for_unit_with_executor(
            &mut *connection,
            input.stage_run_unit_id,
        )
        .await
        .map_err(runtime_memory_error_from_db)?
        .ok_or(RuntimeMemoryError::Missing {
            entity: "stage_team_plans",
        })?;
        if plan.operation_id != input.operation_id
            || plan.stage_execution_id != input.stage_execution_id
            || plan.stage_run_unit_id != input.stage_run_unit_id
            || plan.id != input.stage_team_plan_id
            || plan.dispatch_epoch != input.dispatch_epoch
        {
            return Err(RuntimeMemoryError::IdentityMismatch {
                code: "stage_team_output_owner_mismatch",
            });
        }
        golish_db::repo::stage_teams::list_outputs_with_executor(&mut *connection, plan.id)
            .await
            .map_err(runtime_memory_error_from_db)?
            .into_iter()
            .map(stage_worker_output_from_db)
            .collect()
    }

    async fn claim_stage_aggregator(
        &self,
        input: ClaimStageAggregator,
    ) -> Result<ClaimedStageWorkItemView, RuntimeMemoryError> {
        golish_db::repo::runtime_memory_tx::claim_stage_aggregator(
            &self.pool,
            &ClaimStageAggregatorRow {
                claim: claim_stage_work_item_to_db(input.claim),
                expected_dispatch_epoch: input.expected_dispatch_epoch,
                expected_manifest_hash: input.expected_manifest_sha256,
            },
        )
        .await
        .map_err(runtime_memory_error_from_db)
        .and_then(claimed_stage_work_item_from_db)
    }

    async fn claim_worker_and_bind_chain(
        &self,
        input: ClaimWorkerAndBindChain,
    ) -> Result<ClaimedWorkerView, RuntimeMemoryError> {
        let claimed = golish_db::repo::runtime_memory_tx::claim_worker_and_bind_chain(
            &self.pool,
            &ClaimWorkerAndBindChainRow {
                operation_id: input.operation_id,
                stage_execution_id: input.stage_execution_id,
                stage_run_unit_id: input.stage_run_unit_id,
                worker_run_id: input.worker_run_id,
                expected_unit_status: runtime_unit_status_to_db(input.expected_unit_status),
                expected_unit_row_version: input.expected_unit_row_version,
                expected_worker_status: runtime_worker_status_to_db(input.expected_worker_status),
                expected_attempt_epoch: input.expected_attempt_epoch,
                session_id: input.session_id,
                subtask_id: input.subtask_id,
                agent: convert_agent_type_back(input.agent),
                model: input.model,
                provider: input.provider,
                parent_chain_id: input.parent_chain_id,
                lease_owner: input.lease_owner,
                lease_seconds: input.lease_seconds,
                initial_chain: input.initial_chain,
                initial_checkpoint: input.initial_checkpoint,
            },
        )
        .await
        .map_err(runtime_memory_error_from_db)?;
        Ok(ClaimedWorkerView {
            unit: runtime_stage_unit_from_db(claimed.unit)?,
            worker: runtime_worker_from_db(claimed.worker)?,
            message_chain_id: claimed.message_chain_id,
        })
    }

    async fn claim_candidate_attempt(
        &self,
        input: ClaimCandidateAttempt,
    ) -> Result<Option<ClaimedCandidateAttemptView>, RuntimeMemoryError> {
        let authority: Option<(Uuid, Uuid, Uuid)> = sqlx::query_as(
            r#"SELECT unit.scope_snapshot_id,wave.id,wave_unit.id
                 FROM stage_run_units unit
                 JOIN attack_wave_runs wave
                   ON wave.operation_id=unit.operation_id
                  AND wave.scope_snapshot_id=unit.scope_snapshot_id
                  AND wave.generation=unit.generation
                  AND wave.status='verification'
                  AND wave.terminal_at IS NULL
                 JOIN attack_wave_units wave_unit
                   ON wave_unit.wave_run_id=wave.id
                  AND wave_unit.operation_id=wave.operation_id
                  AND wave_unit.scope_snapshot_id=wave.scope_snapshot_id
                  AND wave_unit.organization_id=unit.organization_id
                  AND wave_unit.status='verification'
                  AND wave_unit.review_closed
                  AND NOT wave_unit.verification_closed
                  AND wave_unit.consolidation_status='pending'
                  AND wave_unit.terminal_at IS NULL
                WHERE unit.id=$1 AND unit.operation_id=$2
                  AND unit.stage_execution_id=$3 AND unit.organization_id=$4
                  AND unit.stage_kind='verification'
                  AND unit.specialist='candidate_verifier'
                LIMIT 1"#,
        )
        .bind(input.verification_stage_run_unit_id)
        .bind(input.operation_id)
        .bind(input.verification_stage_execution_id)
        .bind(input.organization_id)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        let Some((scope_snapshot_id, wave_run_id, wave_unit_id)) = authority else {
            return Err(RuntimeMemoryError::Missing {
                entity: "candidate_verification_wave_authority",
            });
        };
        let claimed = golish_db::repo::candidate_attempts::claim_next_candidate_attempt(
            &self.pool,
            golish_db::repo::candidate_attempts::CandidateClaimQuery {
                operation_id: input.operation_id,
                scope_snapshot_id,
                wave_run_id,
                wave_unit_id,
                organization_id: input.organization_id,
                verification_stage_execution_id: input.verification_stage_execution_id,
                verification_stage_run_unit_id: input.verification_stage_run_unit_id,
                lease_owner: input.lease_owner,
                lease_seconds: input.lease_seconds,
            },
        )
        .await
        .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        claimed
            .map(|claimed| {
                let message_chain_id =
                    claimed
                        .worker
                        .message_chain_id
                        .ok_or(RuntimeMemoryError::Missing {
                            entity: "candidate_worker.message_chain_id",
                        })?;
                Ok(ClaimedCandidateAttemptView {
                    candidate_attempt: golish_core::CandidateAttemptContextRef {
                        candidate_id: claimed.attempt.candidate_id,
                        approval_id: claimed.attempt.approval_id,
                        attempt_id: claimed.attempt.id,
                        candidate_plan_hash: claimed.attempt.candidate_plan_hash,
                    },
                    worker: runtime_worker_from_db(claimed.worker)?,
                    message_chain_id,
                    submit_only: claimed.submit_only,
                })
            })
            .transpose()
    }

    async fn heartbeat_candidate_attempt(
        &self,
        input: HeartbeatCandidateAttempt,
    ) -> Result<CandidateHeartbeatView, RuntimeMemoryError> {
        let authority: Option<(Uuid, Uuid, Uuid)> = sqlx::query_as(
            "SELECT scope_snapshot_id,wave_run_id,wave_unit_id
             FROM candidate_attempts
             WHERE id=$1 AND candidate_id=$2 AND approval_id=$3
               AND operation_id=$4 AND organization_id=$5
               AND candidate_plan_hash=$6 AND stage_worker_run_id=$7
               AND status='running'",
        )
        .bind(input.candidate_attempt.attempt_id)
        .bind(input.candidate_attempt.candidate_id)
        .bind(input.candidate_attempt.approval_id)
        .bind(input.fence.operation_id)
        .bind(input.organization_id)
        .bind(&input.candidate_attempt.candidate_plan_hash)
        .bind(input.fence.worker_run_id)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        let Some((scope_snapshot_id, wave_run_id, wave_unit_id)) = authority else {
            return Err(RuntimeMemoryError::IdentityMismatch {
                code: "candidate_heartbeat_attempt_identity_mismatch",
            });
        };
        let heartbeat = golish_db::repo::candidate_attempts::heartbeat_candidate_execution(
            &self.pool,
            golish_db::repo::candidate_attempts::CandidateExecutionHeartbeat {
                operation_id: input.fence.operation_id,
                scope_snapshot_id,
                wave_run_id,
                wave_unit_id,
                organization_id: input.organization_id,
                attempt_id: input.candidate_attempt.attempt_id,
                worker_run_id: input.fence.worker_run_id,
                stage_execution_id: input.fence.stage_execution_id,
                stage_run_unit_id: input.fence.stage_run_unit_id,
                lease_token: input.fence.lease_token,
                lease_owner: input.lease_owner,
                attempt_epoch: input.fence.attempt_epoch,
                expected_checkpoint_version: input.fence.expected_checkpoint_version,
                extend_seconds: input.extend_seconds,
            },
        )
        .await
        .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        Ok(CandidateHeartbeatView {
            lease_expires_at: heartbeat.lease_expires_at,
            attempt_epoch: heartbeat.attempt_epoch,
            checkpoint_version: heartbeat.checkpoint_version,
        })
    }

    async fn candidate_execution_continuation(
        &self,
        input: ControlCandidateAttempt,
    ) -> Result<CandidateExecutionContinuationView, RuntimeMemoryError> {
        let command = candidate_control_to_db(self.pool.as_ref(), input).await?;
        let continuation = golish_db::repo::candidate_attempts::candidate_execution_continuation(
            &self.pool, &command,
        )
        .await
        .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        Ok(match continuation {
            golish_db::repo::candidate_attempts::CandidateExecutionContinuation::SafeRelease => {
                CandidateExecutionContinuationView::SafeRelease
            }
            golish_db::repo::candidate_attempts::CandidateExecutionContinuation::SubmitOnly => {
                CandidateExecutionContinuationView::SubmitOnly
            }
            golish_db::repo::candidate_attempts::CandidateExecutionContinuation::RecoveryRequired => {
                CandidateExecutionContinuationView::RecoveryRequired
            }
        })
    }

    async fn release_candidate_attempt(
        &self,
        input: ControlCandidateAttempt,
    ) -> Result<CandidateReleaseView, RuntimeMemoryError> {
        let command = candidate_control_to_db(self.pool.as_ref(), input).await?;
        let released =
            golish_db::repo::candidate_attempts::release_candidate_execution(&self.pool, command)
                .await
                .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        Ok(CandidateReleaseView {
            requeued: released.requeued,
        })
    }

    async fn submit_candidate_attempt(
        &self,
        input: SubmitCandidateAttempt,
    ) -> Result<SubmittedCandidateAttemptView, RuntimeMemoryError> {
        golish_agent_kit::harness::attack_execution::validate_bound_terminal_result(
            &input.result,
            input.candidate_attempt.attempt_id,
            &input.candidate_attempt.candidate_plan_hash,
        )
        .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        let tool_call_record_id: Uuid = sqlx::query_scalar(
            r#"SELECT tool.id
                 FROM candidate_attempts attempt
                 JOIN stage_worker_runs worker ON worker.id=attempt.stage_worker_run_id
                 JOIN tool_calls tool ON tool.id=worker.active_tool_call_id
                WHERE attempt.id=$1 AND attempt.candidate_id=$2 AND attempt.approval_id=$3
                  AND attempt.operation_id=$4 AND attempt.organization_id=$5
                  AND attempt.candidate_plan_hash=$6 AND attempt.stage_worker_run_id=$7
                  AND worker.stage_execution_id=$8 AND worker.stage_run_unit_id=$9
                  AND worker.lease_token=$10 AND worker.attempt_epoch=$11
                  AND worker.status='running' AND worker.lease_expires_at>NOW()
                  AND tool.name='submit_candidate_attempt'
                  AND tool.status IN ('received','running') AND tool.result IS NULL"#,
        )
        .bind(input.candidate_attempt.attempt_id)
        .bind(input.candidate_attempt.candidate_id)
        .bind(input.candidate_attempt.approval_id)
        .bind(input.fence.operation_id)
        .bind(input.organization_id)
        .bind(&input.candidate_attempt.candidate_plan_hash)
        .bind(input.fence.worker_run_id)
        .bind(input.fence.stage_execution_id)
        .bind(input.fence.stage_run_unit_id)
        .bind(input.fence.lease_token)
        .bind(input.fence.attempt_epoch)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?
        .ok_or(RuntimeMemoryError::IdentityMismatch {
            code: "candidate_submission_identity_mismatch",
        })?;
        let mut evidence = input
            .result
            .proof_evidence_ids
            .iter()
            .map(
                |id| golish_db::repo::candidate_attempts::AttemptEvidenceLink {
                    evidence_id: *id,
                    role: "proof".to_string(),
                },
            )
            .chain(input.result.refutation_evidence_ids.iter().map(|id| {
                golish_db::repo::candidate_attempts::AttemptEvidenceLink {
                    evidence_id: *id,
                    role: "refutation".to_string(),
                }
            }))
            .chain(input.result.blocker_evidence_ids.iter().map(|id| {
                golish_db::repo::candidate_attempts::AttemptEvidenceLink {
                    evidence_id: *id,
                    role: "blocker".to_string(),
                }
            }))
            .chain(input.result.fact_deltas.iter().flat_map(|delta| {
                delta.evidence_ids.iter().map(|id| {
                    golish_db::repo::candidate_attempts::AttemptEvidenceLink {
                        evidence_id: *id,
                        role: "fact_delta".to_string(),
                    }
                })
            }))
            .collect::<Vec<_>>();
        evidence.sort_by(|left, right| {
            (left.evidence_id, left.role.as_str()).cmp(&(right.evidence_id, right.role.as_str()))
        });
        evidence.dedup_by(|left, right| {
            left.evidence_id == right.evidence_id && left.role == right.role
        });
        let result_json = serde_json::to_value(&input.result)
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        let disposition = result_json
            .get("disposition")
            .and_then(serde_json::Value::as_str)
            .ok_or(RuntimeMemoryError::IdentityMismatch {
                code: "candidate_submission_disposition_missing",
            })?
            .to_string();
        let tool_result = serde_json::json!({
            "attempt_id": input.candidate_attempt.attempt_id,
            "instruction": "No further external action is allowed. Return control so the host can checkpoint the post-tool result and terminalize with server authority.",
            "status": "terminal_intent_persisted",
        });
        let tool_result_text = serde_json::to_string(&tool_result)
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        let submission = golish_db::repo::candidate_recovery::record_candidate_terminal_intent(
            &mut tx,
            golish_db::repo::candidate_recovery::RecordCandidateTerminalIntent {
                request_id: format!("candidate-terminal-intent:{tool_call_record_id}"),
                operation_id: input.fence.operation_id,
                organization_id: input.organization_id,
                candidate_id: input.candidate_attempt.candidate_id,
                approval_id: input.candidate_attempt.approval_id,
                attempt_id: input.candidate_attempt.attempt_id,
                candidate_plan_hash: input.candidate_attempt.candidate_plan_hash,
                worker_run_id: input.fence.worker_run_id,
                lease_token: input.fence.lease_token,
                attempt_epoch: input.fence.attempt_epoch,
                tool_call_record_id,
                disposition,
                submitted_result: result_json,
                evidence,
                tool_result_text,
            },
        )
        .await
        .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        tx.commit()
            .await
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        Ok(SubmittedCandidateAttemptView {
            attempt_id: submission.intent.attempt_id,
            result_hash: submission.intent.result_hash,
            terminal_intent_id: Some(submission.intent.id),
            terminal_intent_hash: Some(submission.intent.intent_hash),
            tool_result,
            replayed: submission.replayed,
        })
    }

    async fn terminalize_candidate_attempt(
        &self,
        input: TerminalizeCandidateAttempt,
    ) -> Result<TerminalizedCandidateAttemptView, RuntimeMemoryError> {
        type TerminalAuthority = (Uuid, Uuid, Uuid, String, i64, String);
        let authority: TerminalAuthority = sqlx::query_as(
            r#"SELECT attempt.scope_snapshot_id,attempt.wave_run_id,attempt.wave_unit_id,
                      COALESCE(worker.lease_owner,''),worker.checkpoint_version,
                      attempt.result_hash
                 FROM candidate_attempts attempt
                 JOIN stage_worker_runs worker ON worker.id=attempt.stage_worker_run_id
                WHERE attempt.id=$1 AND attempt.candidate_id=$2 AND attempt.approval_id=$3
                  AND attempt.operation_id=$4 AND attempt.organization_id=$5
                  AND attempt.candidate_plan_hash=$6 AND attempt.stage_worker_run_id=$7
                  AND worker.stage_execution_id=$8 AND worker.stage_run_unit_id=$9
                  AND worker.attempt_epoch=$11
                  AND (
                    (attempt.status='submitted' AND worker.lease_token=$10
                     AND worker.status='running' AND worker.lease_expires_at>NOW())
                    OR attempt.status IN ('verified','refuted','blocked')
                  )"#,
        )
        .bind(input.candidate_attempt.attempt_id)
        .bind(input.candidate_attempt.candidate_id)
        .bind(input.candidate_attempt.approval_id)
        .bind(input.fence.operation_id)
        .bind(input.organization_id)
        .bind(&input.candidate_attempt.candidate_plan_hash)
        .bind(input.fence.worker_run_id)
        .bind(input.fence.stage_execution_id)
        .bind(input.fence.stage_run_unit_id)
        .bind(input.fence.lease_token)
        .bind(input.fence.attempt_epoch)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?
        .ok_or(RuntimeMemoryError::IdentityMismatch {
            code: "candidate_terminalization_identity_mismatch",
        })?;
        let (
            scope_snapshot_id,
            wave_run_id,
            wave_unit_id,
            lease_owner,
            checkpoint_version,
            result_hash,
        ) = authority;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        let terminal = golish_db::repo::finding_lineage::terminalize_candidate_attempt(
            &mut tx,
            golish_db::repo::finding_lineage::TerminalizeCandidateAttempt {
                operation_id: input.fence.operation_id,
                scope_snapshot_id,
                wave_run_id,
                wave_unit_id,
                organization_id: input.organization_id,
                candidate_id: input.candidate_attempt.candidate_id,
                approval_id: input.candidate_attempt.approval_id,
                attempt_id: input.candidate_attempt.attempt_id,
                candidate_plan_hash: input.candidate_attempt.candidate_plan_hash,
                expected_result_hash: result_hash,
                worker_run_id: input.fence.worker_run_id,
                stage_execution_id: input.fence.stage_execution_id,
                stage_run_unit_id: input.fence.stage_run_unit_id,
                lease_token: input.fence.lease_token,
                lease_owner,
                attempt_epoch: input.fence.attempt_epoch,
                expected_checkpoint_version: checkpoint_version,
            },
        )
        .await
        .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        tx.commit()
            .await
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        Ok(TerminalizedCandidateAttemptView {
            scope_snapshot_id: terminal.scope_snapshot_id,
            wave_run_id: terminal.wave_run_id,
            wave_unit_id: terminal.wave_unit_id,
            organization_id: terminal.organization_id,
            candidate_id: terminal.candidate_id,
            attempt_id: terminal.attempt_id,
            status: terminal.status,
            disposition: terminal.disposition,
            finding_id: terminal.finding_id,
            evidence_count: terminal.evidence_count,
            fact_delta_count: terminal.fact_delta_count,
            replayed: terminal.replayed,
        })
    }

    async fn next_candidate_terminal_intent(
        &self,
        operation_id: Uuid,
    ) -> Result<Option<CandidateTerminalIntentView>, RuntimeMemoryError> {
        golish_db::repo::candidate_recovery::next_candidate_terminal_intent(
            &self.pool,
            operation_id,
        )
        .await
        .map(|row| row.map(candidate_terminal_intent_from_db))
        .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))
    }

    async fn checkpoint_candidate_terminal_barrier(
        &self,
        input: CheckpointCandidateTerminalBarrier,
    ) -> Result<CandidateTerminalBarrierView, RuntimeMemoryError> {
        let terminal_intent_id = input.terminal_intent_id;
        let recorded = golish_db::repo::candidate_recovery::checkpoint_candidate_terminal_barrier(
            &self.pool,
            golish_db::repo::candidate_recovery::CheckpointCandidateTerminalBarrier {
                request_id: format!("candidate-terminal-barrier:{terminal_intent_id}"),
                intent_id: terminal_intent_id,
                expected_intent_hash: input.expected_intent_hash,
                checkpoint: CheckpointBoundWorkerChainRow {
                    fence: runtime_worker_fence_to_db(input.checkpoint.fence),
                    message_chain_id: input.checkpoint.message_chain_id,
                    chain: input.checkpoint.chain,
                    checkpoint: input.checkpoint.checkpoint,
                },
            },
        )
        .await
        .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        Ok(candidate_terminal_barrier_from_db(recorded))
    }

    async fn recover_candidate_terminal_intent(
        &self,
        input: RecoverCandidateTerminalIntent,
    ) -> Result<CandidateTerminalBarrierView, RuntimeMemoryError> {
        let recovered =
            golish_db::repo::candidate_recovery::recover_candidate_terminal_intent_barrier(
                &self.pool,
                golish_db::repo::candidate_recovery::RecoverCandidateTerminalIntent {
                    operation_id: input.operation_id,
                    intent_id: input.terminal_intent_id,
                    expected_intent_hash: input.expected_intent_hash,
                },
            )
            .await
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        Ok(candidate_terminal_barrier_row_from_db(
            recovered.barrier,
            recovered.replayed,
        ))
    }

    async fn terminalize_candidate_intent(
        &self,
        input: TerminalizeCandidateIntent,
    ) -> Result<TerminalizedCandidateAttemptView, RuntimeMemoryError> {
        let intent = golish_db::repo::candidate_recovery::load_candidate_terminal_intent(
            &self.pool,
            input.operation_id,
            input.terminal_intent_id,
        )
        .await
        .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?
        .ok_or(RuntimeMemoryError::Missing {
            entity: "candidate_terminal_intent",
        })?;
        if intent.intent_hash != input.expected_intent_hash {
            return Err(RuntimeMemoryError::IdentityMismatch {
                code: "candidate_terminal_intent_hash_mismatch",
            });
        }
        if intent.barrier_id != Some(input.barrier_id) {
            return Err(RuntimeMemoryError::IdentityMismatch {
                code: "candidate_terminal_barrier_identity_mismatch",
            });
        }
        if intent.barrier_hash.as_deref() != Some(input.expected_barrier_hash.as_str()) {
            return Err(RuntimeMemoryError::IdentityMismatch {
                code: "candidate_terminal_barrier_hash_mismatch",
            });
        }
        let terminalized =
            golish_db::repo::candidate_recovery::terminalize_candidate_terminal_intent(
                &self.pool,
                golish_db::repo::candidate_recovery::TerminalizeCandidateTerminalIntent {
                    request_id: format!(
                        "candidate-terminal-receipt:{}:{}",
                        input.terminal_intent_id, input.barrier_id
                    ),
                    operation_id: input.operation_id,
                    intent_id: input.terminal_intent_id,
                    barrier_id: input.barrier_id,
                },
            )
            .await
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        Ok(terminalized_candidate_from_db(terminalized.terminalized))
    }

    async fn resolve_candidate_recovery(
        &self,
        input: ResolveCandidateRecovery,
    ) -> Result<ResolvedCandidateRecoveryView, RuntimeMemoryError> {
        let resolution = match input.decision {
            CandidateRecoveryDecision::TerminalizeBlockedOutcomeUnknown => {
                golish_db::repo::candidate_recovery::CandidateRecoveryResolution::TerminalizeBlockedOutcomeUnknown
            }
            CandidateRecoveryDecision::AbandonBeforeSideEffect => {
                golish_db::repo::candidate_recovery::CandidateRecoveryResolution::AbandonBeforeSideEffect
            }
            CandidateRecoveryDecision::AcceptExternalResultWithExactEvidence => {
                golish_db::repo::candidate_recovery::CandidateRecoveryResolution::AcceptExternalResultWithExactEvidence
            }
        };
        let principal = golish_db::repo::operator_principals::current_local(&self.pool)
            .await
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        let resolved = golish_db::repo::candidate_recovery::resolve_candidate_recovery(
            &self.pool,
            golish_db::repo::candidate_recovery::ResolveCandidateRecovery {
                request_id: input.request_id.to_string(),
                operation_id: input.operation_id,
                recovery_case_id: input.recovery_case_id,
                expected_row_version: input.expected_case_version,
                expected_attempt_row_version: input.expected_attempt_version,
                resolved_by: principal.id,
                resolution,
                evidence_ids: input.evidence_ids,
            },
        )
        .await
        .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        let terminal_intent = match resolved.recovery_case.intent_id {
            Some(intent_id) => golish_db::repo::candidate_recovery::load_candidate_terminal_intent(
                &self.pool,
                input.operation_id,
                intent_id,
            )
            .await
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?
            .map(candidate_terminal_intent_from_db),
            None => None,
        };
        Ok(ResolvedCandidateRecoveryView {
            recovery_case: candidate_recovery_case_from_db(resolved.recovery_case),
            terminal_intent,
            replayed: resolved.replayed,
        })
    }

    async fn expire_candidate_starts_before_claim(
        &self,
        operation_id: Uuid,
    ) -> Result<u32, RuntimeMemoryError> {
        golish_db::repo::candidate_recovery::expire_candidate_starts_before_claim(
            &self.pool,
            operation_id,
        )
        .await
        .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))
    }

    async fn converge_next_candidate_recovery(
        &self,
        operation_id: Uuid,
    ) -> Result<Option<ConvergedCandidateRecoveryView>, RuntimeMemoryError> {
        golish_db::repo::candidate_recovery::converge_next_candidate_recovery(
            &self.pool,
            operation_id,
        )
        .await
        .map(|row| row.map(converged_candidate_recovery_from_db))
        .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))
    }

    async fn close_attack_v2_verification_unit(
        &self,
        input: CloseAttackV2VerificationUnit,
    ) -> Result<ClosedAttackV2VerificationUnitView, RuntimeMemoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        let closed = golish_db::repo::verification_truth::close_verification_unit(
            &mut tx,
            golish_db::repo::verification_truth::CloseVerificationUnit {
                operation_id: input.operation_id,
                scope_snapshot_id: input.scope_snapshot_id,
                wave_run_id: input.wave_run_id,
                wave_unit_id: input.wave_unit_id,
                organization_id: input.organization_id,
                verification_stage_execution_id: input.verification_stage_execution_id,
                verification_stage_run_unit_id: input.verification_stage_run_unit_id,
            },
        )
        .await
        .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        tx.commit()
            .await
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        Ok(ClosedAttackV2VerificationUnitView {
            wave_unit_id: closed.wave_unit_id,
            row_version: closed.row_version,
            verification_closed: closed.verification_closed,
            consolidation_status: closed.consolidation_status,
            verification_stage_run_unit_id: closed.verification_stage_run_unit_id,
            verification_stage_run_unit_status: closed.verification_stage_run_unit_status,
            verification_primary_worker_run_id: closed.verification_primary_worker_run_id,
            verification_primary_worker_status: closed.verification_primary_worker_status,
            verification_handoff_id: closed.verification_handoff_id,
            verification_handoff_payload_sha256: closed.verification_handoff_payload_sha256,
            replayed: closed.replayed,
        })
    }

    async fn load_worker_checkpoint(
        &self,
        input: LoadWorkerCheckpoint,
    ) -> Result<LoadedWorkerCheckpoint, RuntimeMemoryError> {
        let loaded = golish_db::repo::runtime_memory_tx::load_worker_checkpoint(
            &self.pool,
            &LoadWorkerCheckpointRow {
                operation_id: input.operation_id,
                stage_execution_id: input.stage_execution_id,
                stage_run_unit_id: input.stage_run_unit_id,
                worker_run_id: input.worker_run_id,
                selected_source: input.selected_source.map(runtime_record_source_to_db),
            },
        )
        .await
        .map_err(runtime_memory_error_from_db)?;
        Ok(LoadedWorkerCheckpoint {
            source: runtime_record_source_from_db(loaded.source),
            worker: runtime_worker_from_db(loaded.worker)?,
        })
    }

    async fn checkpoint_worker(
        &self,
        input: CheckpointWorker,
    ) -> Result<RuntimeWorkerView, RuntimeMemoryError> {
        golish_db::repo::runtime_memory_tx::checkpoint_worker(
            &self.pool,
            &runtime_worker_fence_to_db(input.fence),
            &input.checkpoint,
        )
        .await
        .map_err(runtime_memory_error_from_db)
        .and_then(runtime_worker_from_db)
    }

    async fn checkpoint_bound_worker_chain(
        &self,
        input: CheckpointBoundWorkerChain,
    ) -> Result<RuntimeWorkerView, RuntimeMemoryError> {
        golish_db::repo::runtime_memory_tx::checkpoint_bound_worker_chain(
            &self.pool,
            &CheckpointBoundWorkerChainRow {
                fence: runtime_worker_fence_to_db(input.fence),
                message_chain_id: input.message_chain_id,
                chain: input.chain,
                checkpoint: input.checkpoint,
            },
        )
        .await
        .map_err(runtime_memory_error_from_db)
        .and_then(runtime_worker_from_db)
    }

    async fn load_bound_worker_chain(
        &self,
        input: LoadBoundWorkerChain,
    ) -> Result<LoadedBoundWorkerChain, RuntimeMemoryError> {
        let loaded = golish_db::repo::runtime_memory_tx::load_bound_worker_chain(
            &self.pool,
            &LoadBoundWorkerChainRow {
                operation_id: input.operation_id,
                stage_execution_id: input.stage_execution_id,
                stage_run_unit_id: input.stage_run_unit_id,
                worker_run_id: input.worker_run_id,
                message_chain_id: input.message_chain_id,
                session_id: input.session_id,
                agent: convert_agent_type_back(input.agent),
                selected_source: input.selected_source.map(runtime_record_source_to_db),
            },
        )
        .await
        .map_err(runtime_memory_error_from_db)?;
        Ok(LoadedBoundWorkerChain {
            source: runtime_record_source_from_db(loaded.source),
            worker: runtime_worker_from_db(loaded.worker)?,
            chain: loaded.chain,
        })
    }

    async fn reap_expired_worker(
        &self,
        input: LoadWorkerCheckpoint,
    ) -> Result<ReapedRuntimeWorker, RuntimeMemoryError> {
        let (disposition, worker) = golish_db::repo::runtime_memory_tx::reap_expired_worker(
            &self.pool,
            &LoadWorkerCheckpointRow {
                operation_id: input.operation_id,
                stage_execution_id: input.stage_execution_id,
                stage_run_unit_id: input.stage_run_unit_id,
                worker_run_id: input.worker_run_id,
                selected_source: input.selected_source.map(runtime_record_source_to_db),
            },
        )
        .await
        .map_err(runtime_memory_error_from_db)?;
        let disposition = match disposition {
            golish_db::repo::stage_worker_runs::ExpiredWorkerDisposition::Requeued => {
                RuntimeExpiredWorkerDisposition::Requeued
            }
            golish_db::repo::stage_worker_runs::ExpiredWorkerDisposition::RecoveryRequired => {
                RuntimeExpiredWorkerDisposition::RecoveryRequired
            }
        };
        Ok(ReapedRuntimeWorker {
            disposition,
            worker: runtime_worker_from_db(worker)?,
        })
    }

    async fn heartbeat_worker(
        &self,
        fence: RuntimeWorkerFence,
        extend_seconds: i32,
    ) -> Result<RuntimeWorkerView, RuntimeMemoryError> {
        golish_db::repo::runtime_memory_tx::heartbeat_worker(
            &self.pool,
            &runtime_worker_fence_to_db(fence),
            extend_seconds,
        )
        .await
        .map_err(runtime_memory_error_from_db)
        .and_then(runtime_worker_from_db)
    }

    async fn begin_worker_tool(
        &self,
        input: WorkerToolMutation,
    ) -> Result<RuntimeWorkerView, RuntimeMemoryError> {
        golish_db::repo::runtime_memory_tx::begin_worker_tool(
            &self.pool,
            &runtime_worker_fence_to_db(input.fence),
            input.tool_call_record_id,
        )
        .await
        .map_err(runtime_memory_error_from_db)
        .and_then(runtime_worker_from_db)
    }

    async fn finish_worker_tool(
        &self,
        input: WorkerToolMutation,
    ) -> Result<RuntimeWorkerView, RuntimeMemoryError> {
        golish_db::repo::runtime_memory_tx::finish_worker_tool(
            &self.pool,
            &runtime_worker_fence_to_db(input.fence),
            input.tool_call_record_id,
        )
        .await
        .map_err(runtime_memory_error_from_db)
        .and_then(runtime_worker_from_db)
    }

    async fn finish_worker_attempt(
        &self,
        input: FinishWorkerAttempt,
    ) -> Result<FinishedWorkerAttempt, RuntimeMemoryError> {
        let finished = golish_db::repo::runtime_memory_tx::finish_worker_attempt(
            &self.pool,
            &FinishWorkerAttemptRow {
                fence: runtime_worker_fence_to_db(input.fence),
                expected_status: runtime_worker_status_to_db(input.expected_status),
                next_status: runtime_worker_status_to_db(input.next_status),
                expected_unit_status: runtime_unit_status_to_db(input.expected_unit_status),
                expected_unit_row_version: input.expected_unit_row_version,
                next_unit_status: runtime_unit_status_to_db(input.next_unit_status),
                checkpoint: input.checkpoint,
                evidence_watermark: input.evidence_watermark,
            },
        )
        .await
        .map_err(runtime_memory_error_from_db)?;
        Ok(FinishedWorkerAttempt {
            unit: runtime_stage_unit_from_db(finished.unit)?,
            worker: runtime_worker_from_db(finished.worker)?,
        })
    }

    async fn complete_stage_worker(
        &self,
        input: CompleteStageWorker,
    ) -> Result<CompletedStageWorkerView, RuntimeMemoryError> {
        if input.output.work_item_id != input.work_item_id
            || input.output.worker_run_id != input.fence.worker_run_id
        {
            return Err(RuntimeMemoryError::IdentityMismatch {
                code: "stage_worker_output_identity_mismatch",
            });
        }
        let output = input.output;
        let completed = golish_db::repo::stage_teams::complete_stage_worker(
            &self.pool,
            CompleteStageWorkerRow {
                fence: runtime_worker_fence_to_db(input.fence),
                team_plan_id: input.stage_team_plan_id,
                work_item_id: input.work_item_id,
                expected_work_item_row_version: input.expected_work_item_row_version,
                output_schema: output.output_schema,
                business_disposition: output.disposition.as_str().to_string(),
                canonical_output: output.canonical_output,
                canonical_fact_refs: Value::Array(output.fact_refs),
                evidence_ids: output.evidence_ids,
                checked_empty_cells: Value::Array(output.checked_empty_units),
                blocker_codes: output.blocker_code.into_iter().collect(),
                output_hash: output.output_sha256,
                terminal_checkpoint: input.terminal_checkpoint,
                evidence_watermark: input.evidence_watermark,
            },
        )
        .await
        .map_err(runtime_memory_error_from_db)?;
        let plan = stage_team_plan_from_db(completed.plan)?;
        let aggregator_role = plan.aggregator_role.clone();
        Ok(CompletedStageWorkerView {
            unit: runtime_stage_unit_from_db(completed.unit)?,
            plan,
            work_item: stage_work_item_from_db(completed.work_item, aggregator_role.as_deref())?,
            worker: runtime_worker_from_db(completed.worker)?,
            output: stage_worker_output_from_db(completed.output)?,
            replayed: completed.replayed,
        })
    }

    async fn retry_stage_worker(
        &self,
        input: RetryStageWorker,
    ) -> Result<RetriedStageWorkerView, RuntimeMemoryError> {
        let retried = golish_db::repo::stage_teams::retry_stage_worker(
            &self.pool,
            RetryStageWorkerRow {
                fence: runtime_worker_fence_to_db(input.fence),
                team_plan_id: input.stage_team_plan_id,
                work_item_id: input.work_item_id,
                expected_work_item_row_version: input.expected_work_item_row_version,
                failure_code: input.failure_code,
                terminal_checkpoint: input.terminal_checkpoint,
            },
        )
        .await
        .map_err(runtime_memory_error_from_db)?;
        let plan = stage_team_plan_from_db(retried.plan)?;
        let aggregator_role = plan.aggregator_role.clone();
        Ok(RetriedStageWorkerView {
            unit: runtime_stage_unit_from_db(retried.unit)?,
            plan,
            work_item: stage_work_item_from_db(retried.work_item, aggregator_role.as_deref())?,
            worker: runtime_worker_from_db(retried.worker)?,
            retry_scheduled: retried.retry_scheduled,
        })
    }

    async fn resolve_stage_team_recovery(
        &self,
        input: ResolveStageTeamRecovery,
    ) -> Result<ResolvedStageTeamRecoveryView, RuntimeMemoryError> {
        let resolved = golish_db::repo::stage_teams::resolve_stage_team_recovery(
            &self.pool,
            &ResolveStageTeamRecoveryRow {
                request_id: input.request_id,
                operation_id: input.operation_id,
                stage_execution_id: input.stage_execution_id,
                stage_run_unit_id: input.stage_run_unit_id,
                scope_snapshot_id: input.scope_snapshot_id,
                team_plan_id: input.stage_team_plan_id,
                work_item_id: input.work_item_id,
                worker_run_id: input.worker_run_id,
                tool_call_record_id: input.tool_call_record_id,
                expected_work_item_row_version: input.expected_work_item_row_version,
                expected_checkpoint_version: input.expected_checkpoint_version,
                expected_attempt_epoch: input.expected_attempt_epoch,
                resolved_by: input.resolved_by,
            },
        )
        .await
        .map_err(runtime_memory_error_from_db)?;
        let plan = golish_db::repo::stage_teams::get_plan_for_unit_with_executor(
            &*self.pool,
            resolved.worker.stage_run_unit_id,
        )
        .await
        .map_err(runtime_memory_error_from_db)?
        .ok_or_else(|| RuntimeMemoryError::Storage("StageTeam plan disappeared".to_string()))?;
        Ok(ResolvedStageTeamRecoveryView {
            decision_id: resolved.decision.id,
            decision_sha256: resolved.decision.resolution_hash,
            work_item: stage_work_item_from_db(
                resolved.work_item,
                plan.aggregator_role.as_deref(),
            )?,
            worker: runtime_worker_from_db(resolved.worker)?,
            output: stage_worker_output_from_db(resolved.output)?,
            replayed: resolved.replayed,
        })
    }

    async fn pause_worker_for_continuation(
        &self,
        input: PauseWorkerForContinuation,
    ) -> Result<FinishedWorkerAttempt, RuntimeMemoryError> {
        let paused = golish_db::repo::runtime_memory_tx::pause_worker_for_continuation(
            &self.pool,
            &PauseWorkerForContinuationRow {
                fence: runtime_worker_fence_to_db(input.fence),
                expected_unit_row_version: input.expected_unit_row_version,
                checkpoint: input.checkpoint,
            },
        )
        .await
        .map_err(runtime_memory_error_from_db)?;
        Ok(FinishedWorkerAttempt {
            unit: runtime_stage_unit_from_db(paused.unit)?,
            worker: runtime_worker_from_db(paused.worker)?,
        })
    }

    async fn finalize_unit_pass(
        &self,
        input: FinalizeUnitPass,
    ) -> Result<FinalizedUnitPass, RuntimeMemoryError> {
        let candidate_final_seal = input.candidate_acceptance.is_some();
        let operation_id = input.fence.operation_id;
        let stage_run_unit_id = input.fence.stage_run_unit_id;
        let db_input = finalize_unit_pass_to_db(input)?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        let finalized = golish_db::repo::runtime_memory_tx::finalize_unit_pass_with_transaction(
            &mut tx, &db_input,
        )
        .await
        .map_err(runtime_memory_error_from_db)?;
        let shadow_trace = if candidate_final_seal {
            let sample =
                golish_db::repo::attack_execution_shadow::load_unit_sample_with_connection(
                    &mut tx,
                    operation_id,
                    stage_run_unit_id,
                )
                .await
                .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?
                .ok_or(RuntimeMemoryError::Missing {
                    entity: "attack_execution_shadow_read",
                })?;
            let selected = select_attack_shadow_sample(&sample)?;
            if let Some(comparison) = selected.shadow_comparison() {
                golish_db::repo::attack_execution_shadow::record_unit_selection_with_connection(
                    &mut tx,
                    operation_id,
                    stage_run_unit_id,
                    shadow_comparison_str(comparison),
                    attack_read_source_str(selected.source()),
                )
                .await
                .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
            }
            Some((
                sample.contract,
                attack_read_source_str(selected.source()),
                selected
                    .shadow_comparison()
                    .map(shadow_comparison_str)
                    .unwrap_or("not_applicable"),
                selected.executes_v2_verifier(),
            ))
        } else {
            None
        };
        tx.commit()
            .await
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        // Runtime is the deployment dependency and must reconcile first for
        // every committed final seal. Both attempts own separate best-effort
        // transactions and cannot roll the business mutation back.
        let reconcile_trigger = if candidate_final_seal {
            "candidate_final_seal_db_bridge"
        } else {
            "final_seal_db_bridge"
        };
        golish_db::repo::runtime_memory_rollout::reconcile_best_effort(
            &self.pool,
            reconcile_trigger,
        )
        .await;
        if candidate_final_seal {
            golish_db::repo::attack_execution_rollout::reconcile_attack_execution_rollout_best_effort(
                &self.pool,
                reconcile_trigger,
            )
            .await;
        }
        if let Some((contract, source, comparison, executes_v2_verifier)) = shadow_trace {
            tracing::info!(
                target: "harness::attack_shadow",
                %operation_id,
                %stage_run_unit_id,
                %contract,
                source,
                comparison,
                executes_v2_verifier,
                "selected one complete Candidate attack read"
            );
        }
        finalized_unit_pass_from_db(finalized)
    }

    async fn finalize_stage_team_unit(
        &self,
        input: FinalizeStageTeamUnit,
    ) -> Result<FinalizedStageTeamUnitView, RuntimeMemoryError> {
        let finalized = golish_db::repo::runtime_memory_tx::finalize_stage_team_unit(
            &self.pool,
            &FinalizeStageTeamUnitRow {
                stage_team_plan_id: input.stage_team_plan_id,
                aggregator_work_item_id: input.aggregator_work_item_id,
                expected_dispatch_epoch: input.expected_dispatch_epoch,
                expected_manifest_hash: input.expected_manifest_sha256,
                final_seal: finalize_unit_pass_to_db(input.final_seal)?,
            },
        )
        .await
        .map_err(runtime_memory_error_from_db)?;
        let mut plan = stage_team_plan_from_db(finalized.plan)?;
        plan.status = RuntimeStageTeamPlanStatus::Passed;
        let aggregator_role = plan.aggregator_role.clone();
        Ok(FinalizedStageTeamUnitView {
            plan,
            aggregator_work_item: stage_work_item_from_db(
                finalized.aggregator_work_item,
                aggregator_role.as_deref(),
            )?,
            finalized: finalized_unit_pass_from_db(finalized.finalized)?,
        })
    }

    async fn open_stage_team_repair(
        &self,
        input: OpenStageTeamRepair,
    ) -> Result<OpenedStageTeamRepairView, RuntimeMemoryError> {
        let opened = golish_db::repo::runtime_memory_tx::open_stage_team_repair(
            &self.pool,
            &OpenStageTeamRepairRow {
                request_id: input.request_id,
                fence: runtime_worker_fence_to_db(input.fence),
                stage_team_plan_id: input.stage_team_plan_id,
                aggregator_work_item_id: input.aggregator_work_item_id,
                deliverable_submission_id: input.deliverable_submission_id,
                expected_dispatch_epoch: input.expected_dispatch_epoch,
                expected_manifest_hash: input.expected_manifest_sha256,
                gate_decision_hash: input.gate_decision_sha256,
                gap_manifest: input.gap_manifest,
                gap_manifest_hash: input.gap_manifest_sha256,
            },
        )
        .await
        .map_err(runtime_memory_error_from_db)?;
        let plan = stage_team_plan_from_db(opened.plan)?;
        let aggregator_role = plan.aggregator_role.clone();
        Ok(OpenedStageTeamRepairView {
            plan,
            unit: runtime_stage_unit_from_db(opened.unit)?,
            gap_id: opened.gap.id,
            repair_generation: opened.gap.repair_generation,
            fuel_exhausted: opened.gap.disposition == "fuel_exhausted",
            repair_work_item: opened
                .repair_work_item
                .map(|item| stage_work_item_from_db(item, aggregator_role.as_deref()))
                .transpose()?,
            aggregator_work_item: opened
                .aggregator_work_item
                .map(|item| stage_work_item_from_db(item, aggregator_role.as_deref()))
                .transpose()?,
            aggregator_worker: runtime_worker_from_db(opened.aggregator_worker)?,
            replayed: opened.replayed,
        })
    }

    async fn block_stage_team_unit(
        &self,
        input: BlockStageTeamUnit,
    ) -> Result<BlockedStageTeamUnitView, RuntimeMemoryError> {
        let blocked = golish_db::repo::runtime_memory_tx::block_stage_team_unit(
            &self.pool,
            &BlockStageTeamUnitRow {
                operation_id: input.operation_id,
                stage_execution_id: input.stage_execution_id,
                stage_run_unit_id: input.stage_run_unit_id,
                stage_team_plan_id: input.stage_team_plan_id,
                expected_dispatch_epoch: input.expected_dispatch_epoch,
                expected_manifest_hash: input.expected_manifest_sha256,
            },
        )
        .await
        .map_err(runtime_memory_error_from_db)?;
        let plan = stage_team_plan_from_db(blocked.plan)?;
        let aggregator_role = plan.aggregator_role.clone();
        Ok(BlockedStageTeamUnitView {
            plan,
            aggregator_work_item: stage_work_item_from_db(
                blocked.aggregator_work_item,
                aggregator_role.as_deref(),
            )?,
            unit: runtime_stage_unit_from_db(blocked.unit)?,
            barrier: stage_team_barrier_from_db(blocked.barrier),
            replayed: blocked.replayed,
        })
    }

    async fn close_wave_gate_pass(
        &self,
        input: CloseWaveGatePass,
    ) -> Result<ClosedWaveGatePass, RuntimeMemoryError> {
        let closed = golish_db::repo::runtime_memory_tx::close_wave_gate_pass(
            &self.pool,
            &CloseWaveGatePassRow {
                final_seal: finalize_unit_pass_to_db(input.final_seal)?,
                wave_id: input.wave_id,
                next_wave_limit: input.next_wave_limit,
                continuation_pass_watermark: input.continuation_pass_watermark,
            },
        )
        .await
        .map_err(runtime_memory_error_from_db)?;
        match closed {
            ClosedWaveGatePassRow::WaitingBackground {
                unit,
                worker,
                next_wave,
            } => Ok(ClosedWaveGatePass::WaitingBackground {
                unit: runtime_stage_unit_from_db(unit)?,
                worker: runtime_worker_from_db(worker)?,
                next_wave: super::orchestration::stage_asset_wave_to_view(next_wave),
            }),
            ClosedWaveGatePassRow::Finalized(finalized) => Ok(ClosedWaveGatePass::Finalized(
                finalized_unit_pass_from_db(finalized)?,
            )),
        }
    }

    async fn load_inherited_stage_handoffs(
        &self,
        input: LoadInheritedStageHandoffs,
    ) -> Result<Vec<RuntimeStageHandoffView>, RuntimeMemoryError> {
        golish_db::repo::stage_handoffs::list_latest_final_sealed_for_sources(
            &self.pool,
            input.operation_id,
            input.organization_id,
            &input.source_stage_kinds,
        )
        .await
        .map(|rows| {
            rows.into_iter()
                .map(final_sealed_stage_handoff_from_db)
                .collect()
        })
        .map_err(runtime_memory_error_from_db)
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use golish_agent_kit::db_traits::TaskStatus;
    use golish_agent_kit::harness::attack_execution::{AttackReadSource, AttackShadowComparison};

    #[test]
    fn technique_outcome_set_key_roundtrips_through_db_bridge() {
        let key = CanonicalFactKey::TechniqueOutcomeSet {
            organization_id: Uuid::new_v4(),
            run_id: Uuid::new_v4().to_string(),
            stage: "vuln_triage".to_string(),
            terminal_cell_count: 360,
            outcome_set_sha256: "1111111111111111111111111111111111111111111111111111111111111111"
                .to_string(),
        };
        assert_eq!(
            canonical_fact_key_from_db(canonical_fact_key_to_db(key.clone())),
            key
        );
    }

    #[test]
    fn final_seal_post_commit_seam_reconciles_runtime_before_attack() {
        let source = include_str!("runtime_memory.rs");
        let finalize_start = source
            .find("async fn finalize_unit_pass(")
            .expect("finalize_unit_pass implementation exists");
        let close_wave_offset = source[finalize_start..]
            .find("async fn close_wave_gate_pass(")
            .expect("close_wave_gate_pass follows final seal");
        let finalize_body = &source[finalize_start..finalize_start + close_wave_offset];
        let runtime_reconcile = finalize_body
            .find("runtime_memory_rollout::reconcile_best_effort")
            .expect("runtime rollout reconciles after the final-seal commit");
        let attack_reconcile = finalize_body
            .find("attack_execution_rollout::reconcile_attack_execution_rollout_best_effort")
            .expect("attack rollout reconciles after a Candidate final seal");
        assert!(
            runtime_reconcile < attack_reconcile,
            "runtime is the dependency and must reconcile before attack"
        );
    }

    fn db_task(id: Uuid) -> golish_db::models::Task {
        golish_db::models::Task {
            id,
            session_id: Uuid::new_v4(),
            title: Some("runtime operation".to_string()),
            input: "inspect scope".to_string(),
            result: None,
            status: golish_db::models::TaskStatus::Created,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn db_operation(
        operation_id: Uuid,
        project_scope_id: Uuid,
        contract: &str,
    ) -> golish_db::repo::operation_state::OperationStateRow {
        golish_db::repo::operation_state::OperationStateRow {
            operation_id,
            profile: "assessment".to_string(),
            current_stage: "scoping".to_string(),
            runtime_memory_contract: contract.to_string(),
            project_scope_id: Some(project_scope_id),
            stage_started_at: Utc::now(),
            last_evidence_audit_id: None,
            last_classification_id: None,
            last_scope_version: None,
            state_blob: serde_json::json!({"checkpoint": 1}),
            superseded_by: None,
            engagement_org_id: None,
        }
    }

    fn shadow_complete_record(
    ) -> golish_db::repo::attack_execution_shadow::AttackShadowCompleteReadRow {
        golish_db::repo::attack_execution_shadow::AttackShadowCompleteReadRow {
            decisions: vec![
                golish_db::repo::attack_execution_shadow::AttackShadowDecisionRow {
                    work_item_key: "target:1".to_string(),
                    kind: "candidate".to_string(),
                    semantic_hash: "a".repeat(64),
                },
            ],
            review_counts: golish_db::repo::attack_execution_shadow::AttackShadowReviewCountsRow {
                wave_unit_count: 1,
                review_closed_unit_count: 0,
                candidate_decision_count: 1,
                no_candidate_decision_count: 0,
            },
        }
    }

    #[test]
    fn production_shadow_adapter_uses_kit_whole_record_selector() {
        let record = shadow_complete_record();
        let sample = golish_db::repo::attack_execution_shadow::AttackExecutionShadowSampleRow {
            operation_id: Uuid::new_v4(),
            stage_run_unit_id: Uuid::new_v4(),
            organization_id: Some(Uuid::new_v4()),
            contract: "dual_write_read_legacy".to_string(),
            legacy_record: Some(record.clone()),
            v2_record: golish_db::repo::attack_execution_shadow::AttackShadowV2ReadRow::Complete(
                record,
            ),
            comparison: None,
            selected_source: None,
            selected_record_hash: None,
        };
        let selected = select_attack_shadow_sample(&sample).expect("select whole legacy record");
        assert_eq!(selected.source(), AttackReadSource::Legacy);
        assert_eq!(
            selected.shadow_comparison(),
            Some(AttackShadowComparison::Match)
        );
        assert!(!selected.executes_v2_verifier());
    }

    #[test]
    fn production_shadow_adapter_keeps_v2_only_missing_fail_closed() {
        let sample = golish_db::repo::attack_execution_shadow::AttackExecutionShadowSampleRow {
            operation_id: Uuid::new_v4(),
            stage_run_unit_id: Uuid::new_v4(),
            organization_id: None,
            contract: "v2_only".to_string(),
            legacy_record: None,
            v2_record: golish_db::repo::attack_execution_shadow::AttackShadowV2ReadRow::Missing,
            comparison: None,
            selected_source: None,
            selected_record_hash: None,
        };
        let error = select_attack_shadow_sample(&sample)
            .expect_err("v2_only must not fall back when V2 is missing");
        assert!(matches!(
            error,
            RuntimeMemoryError::Storage(message)
                if message.contains("ATTACK_V2_READ_REQUIRED")
        ));

        let fallback_sample =
            golish_db::repo::attack_execution_shadow::AttackExecutionShadowSampleRow {
                operation_id: Uuid::new_v4(),
                stage_run_unit_id: Uuid::new_v4(),
                organization_id: Some(Uuid::new_v4()),
                contract: "dual_write_read_v2_fallback".to_string(),
                legacy_record: Some(shadow_complete_record()),
                v2_record:
                    golish_db::repo::attack_execution_shadow::AttackShadowV2ReadRow::Incomplete,
                comparison: None,
                selected_source: None,
                selected_record_hash: None,
            };
        let fallback = select_attack_shadow_sample(&fallback_sample)
            .expect("dual V2-preferred mode falls back as one whole record");
        assert_eq!(fallback.source(), AttackReadSource::LegacyFallback);
        assert_eq!(
            fallback.shadow_comparison(),
            Some(AttackShadowComparison::V2Missing)
        );
        assert!(!fallback.executes_v2_verifier());
    }

    #[test]
    fn runtime_memory_bridge_maps_every_typed_store_error_without_erasing_contract_errors() {
        let transition =
            runtime_memory_error_from_db(RuntimeMemoryStoreError::InvalidContractTransition {
                from: DbRuntimeMemoryContract::LegacyV1,
                to: DbRuntimeMemoryContract::V2Only,
            });
        assert!(matches!(
            transition,
            RuntimeMemoryError::InvalidContractTransition {
                from: RuntimeMemoryContract::LegacyV1,
                to: RuntimeMemoryContract::V2Only,
            }
        ));
        assert!(matches!(
            runtime_memory_error_from_db(RuntimeMemoryStoreError::StaleVersion {
                entity: "runtime_memory_rollout",
                expected: 7,
                actual: 8,
            }),
            RuntimeMemoryError::StaleVersion { expected: 7 }
        ));
        assert!(matches!(
            runtime_memory_error_from_db(RuntimeMemoryStoreError::Conflict { code: "conflict" }),
            RuntimeMemoryError::Conflict { code: "conflict" }
        ));
        assert!(matches!(
            runtime_memory_error_from_db(RuntimeMemoryStoreError::IdentityMismatch {
                code: "identity"
            }),
            RuntimeMemoryError::IdentityMismatch { code: "identity" }
        ));
        assert!(matches!(
            runtime_memory_error_from_db(RuntimeMemoryStoreError::Missing { entity: "scope" }),
            RuntimeMemoryError::Missing { entity: "scope" }
        ));
        assert!(matches!(
            runtime_memory_error_from_db(RuntimeMemoryStoreError::Sqlx(sqlx::Error::RowNotFound)),
            RuntimeMemoryError::Storage(message) if !message.is_empty()
        ));
        assert!(matches!(
            runtime_memory_error_from_db(RuntimeMemoryStoreError::Repository(
                golish_db::DbError::NotFound("operation".to_string())
            )),
            RuntimeMemoryError::Storage(message) if message.contains("operation")
        ));
    }

    #[test]
    fn runtime_memory_bridge_roundtrips_created_rows_and_all_persisted_contracts() {
        let contracts = [
            ("legacy_v1", RuntimeMemoryContract::LegacyV1),
            (
                "dual_write_legacy_read",
                RuntimeMemoryContract::DualWriteLegacyRead,
            ),
            (
                "dual_write_v2_preferred",
                RuntimeMemoryContract::DualWriteV2Preferred,
            ),
            ("v2_only", RuntimeMemoryContract::V2Only),
        ];
        for (persisted, expected) in contracts {
            let operation_id = Uuid::new_v4();
            let project_scope_id = Uuid::new_v4();
            let initial_stage_execution_id = Uuid::new_v4();
            let view = created_runtime_operation_from_db(
                CreatedRuntimeOperationRow {
                    task: db_task(operation_id),
                    operation: db_operation(operation_id, project_scope_id, persisted),
                    initial_stage_execution_id,
                },
                project_scope_id,
                initial_stage_execution_id,
            )
            .expect("convert a valid atomic create result");
            assert_eq!(view.task.id, operation_id);
            assert_eq!(view.task.status, TaskStatus::Created);
            assert_eq!(view.operation.operation_id, operation_id);
            assert_eq!(view.operation.runtime_memory_contract, expected);
            assert_eq!(view.operation.project_scope_id, Some(project_scope_id));
            assert_eq!(view.initial_stage_execution_id, initial_stage_execution_id);
        }
    }

    #[test]
    fn runtime_memory_bridge_maps_exact_stage_execution_identity() {
        let operation_id = Uuid::new_v4();
        let stage_execution_id = Uuid::new_v4();
        let execution = stage_execution_from_db(golish_db::repo::stage_runs::StageRunRow {
            id: stage_execution_id,
            operation_id,
            stage_kind: "external_attack_surface".to_string(),
            started_at: Utc::now(),
            completed_at: None,
            status: "started".to_string(),
            active_sprint_contract_id: None,
        })
        .expect("known stage execution must map");

        assert_eq!(execution.id, stage_execution_id);
        assert_eq!(execution.operation_id, operation_id);
        assert_eq!(execution.stage, StageKind::ExternalAttackSurface);
        assert_eq!(execution.status, StageExecutionStatus::Started);

        let error = stage_execution_from_db(golish_db::repo::stage_runs::StageRunRow {
            id: Uuid::new_v4(),
            operation_id,
            stage_kind: "external_attack_surface".to_string(),
            started_at: Utc::now(),
            completed_at: None,
            status: "future_status".to_string(),
            active_sprint_contract_id: None,
        })
        .expect_err("unknown persisted status must fail closed");
        assert!(matches!(
            error,
            RuntimeMemoryError::Storage(message) if message.contains("future_status")
        ));
    }

    #[test]
    fn runtime_memory_bridge_rejects_unknown_contract_and_identity_drift() {
        let operation_id = Uuid::new_v4();
        let project_scope_id = Uuid::new_v4();
        let initial_stage_execution_id = Uuid::new_v4();
        let unknown = created_runtime_operation_from_db(
            CreatedRuntimeOperationRow {
                task: db_task(operation_id),
                operation: db_operation(operation_id, project_scope_id, "future_contract"),
                initial_stage_execution_id,
            },
            project_scope_id,
            initial_stage_execution_id,
        )
        .expect_err("unknown persisted contract must fail closed");
        assert!(matches!(
            unknown,
            RuntimeMemoryError::Storage(message) if message.contains("future_contract")
        ));

        let wrong_scope = created_runtime_operation_from_db(
            CreatedRuntimeOperationRow {
                task: db_task(operation_id),
                operation: db_operation(operation_id, Uuid::new_v4(), "legacy_v1"),
                initial_stage_execution_id,
            },
            project_scope_id,
            initial_stage_execution_id,
        )
        .expect_err("project-scope identity drift must fail closed");
        assert!(matches!(
            wrong_scope,
            RuntimeMemoryError::IdentityMismatch {
                code: "runtime_operation_project_scope_mismatch"
            }
        ));

        let wrong_execution = created_runtime_operation_from_db(
            CreatedRuntimeOperationRow {
                task: db_task(operation_id),
                operation: db_operation(operation_id, project_scope_id, "legacy_v1"),
                initial_stage_execution_id: Uuid::new_v4(),
            },
            project_scope_id,
            initial_stage_execution_id,
        )
        .expect_err("initial stage execution identity drift must fail closed");
        assert!(matches!(
            wrong_execution,
            RuntimeMemoryError::IdentityMismatch {
                code: "runtime_operation_initial_stage_execution_mismatch"
            }
        ));
    }

    #[test]
    fn runtime_memory_bridge_projects_stable_project_scope_registration() {
        let project_scope_id = Uuid::new_v4();
        let now = Utc::now();
        let registration =
            project_scope_registration_from_db(golish_db::repo::project_scopes::ProjectScopeRow {
                project_scope_id,
                canonical_project_path: "/tmp/workspace".to_string(),
                path_sha256: "sha256".to_string(),
                row_version: 4,
                created_at: now,
                updated_at: now,
                retired_at: None,
            });
        assert_eq!(registration.project_scope_id, project_scope_id);
        assert_eq!(registration.canonical_project_path, "/tmp/workspace");
        assert_eq!(registration.path_sha256, "sha256");
        assert_eq!(registration.row_version, 4);
    }

    #[test]
    fn runtime_memory_bridge_preserves_trusted_submission_identity() {
        let deliverable_submission_id = Uuid::new_v4();
        let operation_id = Uuid::new_v4();
        let stage_execution_id = Uuid::new_v4();
        let tool_call_record_id = Uuid::new_v4();
        let row = golish_db::repo::stage_deliverable_submissions::StageDeliverableSubmissionRow {
            id: deliverable_submission_id,
            operation_id,
            stage_execution_id,
            stage_run_unit_id: None,
            worker_run_id: None,
            organization_id: None,
            tool_call_record_id,
            tool_request_id: "trusted-submit".to_string(),
            stage_kind: "scoping".to_string(),
            attempt_epoch: None,
            lease_token: None,
            payload: serde_json::json!({"stage_id":"scoping"}),
            payload_sha256: "a".repeat(64),
            submitted_at: Utc::now(),
        };

        let mapped = stage_submission_from_db(row);
        assert_eq!(mapped.deliverable_submission_id, deliverable_submission_id);
        assert_eq!(mapped.operation_id, operation_id);
        assert_eq!(mapped.stage_execution_id, stage_execution_id);
        assert_eq!(mapped.tool_call_record_id, tool_call_record_id);
    }

    #[test]
    fn runtime_memory_bridge_maps_submission_identity_errors_without_erasure() {
        let mapped = stage_submission_error_from_db(
            golish_db::repo::stage_deliverable_submissions::StageDeliverableSubmissionError::IdentityMismatch {
                code: "submission_tool_operation_mismatch",
            },
        );
        assert!(matches!(
            mapped,
            RuntimeMemoryError::IdentityMismatch {
                code: "submission_tool_operation_mismatch"
            }
        ));
    }

    #[test]
    fn runtime_memory_bridge_maps_unit_worker_statuses_and_rejects_unknown_rows() {
        let operation_id = Uuid::new_v4();
        let stage_execution_id = Uuid::new_v4();
        let unit_id = Uuid::new_v4();
        let organization_id = Uuid::new_v4();
        let now = Utc::now();
        let unit = runtime_stage_unit_from_db(golish_db::repo::stage_run_units::StageRunUnitRow {
            id: unit_id,
            operation_id,
            stage_execution_id,
            scope_snapshot_id: Uuid::new_v4(),
            organization_id,
            stage_kind: "target_intel".to_string(),
            generation: 0,
            specialist: Some("target_intel".to_string()),
            status: "gate_blocked".to_string(),
            gate_attempt: 1,
            pass_watermark: serde_json::json!({}),
            row_version: 2,
            started_at: Some(now),
            updated_at: now,
            terminal_at: None,
        })
        .expect("known unit status");
        assert_eq!(unit.status, RuntimeStageUnitStatus::GateBlocked);

        let worker_row = golish_db::repo::stage_worker_runs::StageWorkerRunRow {
            id: Uuid::new_v4(),
            operation_id,
            stage_execution_id,
            stage_run_unit_id: unit_id,
            work_item_id: None,
            organization_id,
            worker_generation: 0,
            specialist: "target_intel".to_string(),
            work_item_kind: "stage_unit".to_string(),
            work_item_key: "primary".to_string(),
            agent_path: "main>target_intel".to_string(),
            parent_request_id: None,
            message_chain_id: Some(Uuid::new_v4()),
            status: "recovery_required".to_string(),
            gate_attempt: 1,
            checkpoint: serde_json::json!({"turn": 1}),
            checkpoint_version: 2,
            lease_token: Some(Uuid::new_v4()),
            lease_owner: Some("worker".to_string()),
            lease_acquired_at: Some(now),
            lease_expires_at: Some(now),
            heartbeat_at: Some(now),
            attempt_epoch: 3,
            active_tool_call_id: Some(Uuid::new_v4()),
            active_tool_started_at: Some(now),
            evidence_watermark: Some(42),
            started_at: Some(now),
            updated_at: now,
            terminal_at: None,
        };
        let worker = runtime_worker_from_db(worker_row.clone()).expect("known worker status");
        assert_eq!(worker.status, RuntimeWorkerStatus::RecoveryRequired);

        let mut unknown = worker_row;
        unknown.status = "future_worker_status".to_string();
        assert!(matches!(
            runtime_worker_from_db(unknown),
            Err(RuntimeMemoryError::Storage(message)) if message.contains("future_worker_status")
        ));
    }
}
