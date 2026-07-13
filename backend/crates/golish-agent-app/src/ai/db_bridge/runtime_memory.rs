//! Runtime-memory repository bridge from the sqlx-free agent-kit contract to
//! the concrete `golish-db` repositories.

use async_trait::async_trait;
use uuid::Uuid;

use golish_agent_kit::db_traits::{
    CandidateHeartbeatView, CheckpointBoundWorkerChain, CheckpointWorker, ClaimCandidateAttempt,
    ClaimWorkerAndBindChain, ClaimedCandidateAttemptView, ClaimedWorkerView, CloseWaveGatePass,
    ClosedWaveGatePass, CreateRuntimeOperation, CreatedRuntimeOperation, FinalizeScopingScope,
    FinalizeUnitPass, FinalizedScopingScope, FinalizedUnitPass, FinishWorkerAttempt,
    FinishedWorkerAttempt, FrozenOrganizationScopeUnit, HeartbeatCandidateAttempt,
    LoadBoundWorkerChain, LoadInheritedStageHandoffs, LoadWorkerCheckpoint, LoadedBoundWorkerChain,
    LoadedWorkerCheckpoint, NewStageDeliverableSubmission, OperationStateView,
    PauseWorkerForContinuation, PersistedStageDeliverableSubmission, ProjectScopeRegistration,
    ReapedRuntimeWorker, RuntimeExpiredWorkerDisposition, RuntimeMemoryError,
    RuntimeMemoryRecordSource, RuntimeMemoryRepository, RuntimeStageHandoffView,
    RuntimeStageUnitStatus, RuntimeStageUnitView, RuntimeWorkerFence, RuntimeWorkerStatus,
    RuntimeWorkerView, SeedStageRuntime, SeededStageRuntime, SubmitCandidateAttempt,
    SubmittedCandidateAttemptView, TaskView, TerminalizeCandidateAttempt,
    TerminalizedCandidateAttemptView, WorkerToolMutation,
};
use golish_agent_kit::harness::{CanonicalFactKey, CanonicalFactRef, StageKind};
use golish_agent_kit::runtime_memory::RuntimeMemoryContract;
use golish_agent_kit::task_orchestrator::stage_execution::{
    StageExecution, StageExecutionStatus, TransitionStageExecution, TransitionedStageExecution,
};
use golish_db::repo::runtime_memory_rollout::RuntimeMemoryContract as DbRuntimeMemoryContract;
use golish_db::repo::runtime_memory_tx::{
    CheckpointBoundWorkerChainRow, ClaimWorkerAndBindChainRow, CloseWaveGatePassRow,
    ClosedWaveGatePassRow, CreateRuntimeOperationRow, CreatedRuntimeOperationRow,
    FinalizeScopingScopeRow, FinalizeUnitPassRow, FinalizedScopingScopeRow, FinishWorkerAttemptRow,
    LoadBoundWorkerChainRow, LoadWorkerCheckpointRow, PauseWorkerForContinuationRow,
    RuntimeMemoryStoreError, RuntimeMemoryTxFence, SeedStageRuntimeRow,
    TransitionStageExecutionRow,
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
        deliverable_submission_id: row.deliverable_submission_id,
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
        let created = golish_db::repo::runtime_memory_tx::create_runtime_operation(
            &self.pool,
            &CreateRuntimeOperationRow {
                operation_id: input.operation_id,
                initial_stage_execution_id: expected_initial_stage_execution_id,
                session_id: input.session_id,
                title: input.title,
                input: input.input,
                profile: input.profile,
                entry_stage: input.entry_stage,
                project_scope_id: expected_project_scope_id,
                cli_scope,
            },
        )
        .await
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
                  AND wave.status IN ('open','review','verification')
                 JOIN attack_wave_units wave_unit
                   ON wave_unit.wave_run_id=wave.id
                  AND wave_unit.operation_id=wave.operation_id
                  AND wave_unit.scope_snapshot_id=wave.scope_snapshot_id
                  AND wave_unit.organization_id=unit.organization_id
                  AND wave_unit.review_closed
                  AND NOT wave_unit.verification_closed
                  AND wave_unit.terminal_at IS NULL
                WHERE unit.id=$1 AND unit.operation_id=$2
                  AND unit.stage_execution_id=$3 AND unit.organization_id=$4
                  AND unit.stage_kind='verification'
                  AND unit.specialist='candidate_verifier'
                ORDER BY wave.generation DESC
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
        type SubmissionAuthority = (Uuid, Uuid, Uuid, String, i64);
        let authority: SubmissionAuthority = sqlx::query_as(
            r#"SELECT attempt.scope_snapshot_id,attempt.wave_run_id,attempt.wave_unit_id,
                      COALESCE(worker.lease_owner,''),worker.checkpoint_version
                 FROM candidate_attempts attempt
                 JOIN stage_worker_runs worker ON worker.id=attempt.stage_worker_run_id
                WHERE attempt.id=$1 AND attempt.candidate_id=$2 AND attempt.approval_id=$3
                  AND attempt.operation_id=$4 AND attempt.organization_id=$5
                  AND attempt.candidate_plan_hash=$6 AND attempt.stage_worker_run_id=$7
                  AND worker.stage_execution_id=$8 AND worker.stage_run_unit_id=$9
                  AND worker.lease_token=$10 AND worker.attempt_epoch=$11
                  AND worker.status='running' AND worker.lease_expires_at>NOW()"#,
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
        let (scope_snapshot_id, wave_run_id, wave_unit_id, lease_owner, checkpoint_version) =
            authority;
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
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        let submission = golish_db::repo::candidate_attempts::record_attempt_submission(
            &mut tx,
            golish_db::repo::candidate_attempts::RecordAttemptSubmission {
                operation_id: input.fence.operation_id,
                scope_snapshot_id,
                wave_run_id,
                wave_unit_id,
                organization_id: input.organization_id,
                candidate_id: input.candidate_attempt.candidate_id,
                approval_id: input.candidate_attempt.approval_id,
                attempt_id: input.candidate_attempt.attempt_id,
                candidate_plan_hash: input.candidate_attempt.candidate_plan_hash,
                worker_run_id: input.fence.worker_run_id,
                stage_execution_id: input.fence.stage_execution_id,
                stage_run_unit_id: input.fence.stage_run_unit_id,
                lease_token: input.fence.lease_token,
                lease_owner,
                attempt_epoch: input.fence.attempt_epoch,
                expected_checkpoint_version: checkpoint_version,
                result_json,
                evidence,
            },
        )
        .await
        .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        tx.commit()
            .await
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        Ok(SubmittedCandidateAttemptView {
            attempt_id: submission.attempt.id,
            result_hash: submission
                .attempt
                .result_hash
                .ok_or(RuntimeMemoryError::Missing {
                    entity: "candidate_attempt.result_hash",
                })?,
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
            attempt_id: terminal.attempt_id,
            disposition: terminal.disposition,
            finding_id: terminal.finding_id,
            replayed: terminal.replayed,
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
        let finalized = golish_db::repo::runtime_memory_tx::finalize_unit_pass(
            &self.pool,
            &finalize_unit_pass_to_db(input)?,
        )
        .await
        .map_err(runtime_memory_error_from_db)?;
        finalized_unit_pass_from_db(finalized)
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
        .map(|rows| rows.into_iter().map(stage_handoff_from_db).collect())
        .map_err(runtime_memory_error_from_db)
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use golish_agent_kit::db_traits::TaskStatus;

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
