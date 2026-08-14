//! App-owned adapter for asset-bound dynamic Tool Manager verification.

use std::sync::Arc;

use async_trait::async_trait;
use golish_agent_kit::db_traits::*;
use golish_db::repo::investigation_asset_verification as db;
use sqlx::PgPool;

use super::runtime_memory::{
    claimed_stage_work_item_from_db, runtime_stage_unit_from_db, runtime_worker_from_db,
    stage_team_plan_from_db, stage_work_item_from_db, stage_worker_output_from_db,
};

use golish_app_core::ports::pentest::{
    InvestigationAssetVerificationGuardPort, InvestigationVerificationInvocationGuard,
    InvestigationVerificationWorkerFence,
};

#[derive(Clone)]
pub struct PgInvestigationAssetVerificationRepository {
    pool: Arc<PgPool>,
}
impl PgInvestigationAssetVerificationRepository {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl InvestigationAssetVerificationGuardPort for PgInvestigationAssetVerificationRepository {
    async fn load_guard(
        &self,
        invocation_id: uuid::Uuid,
        worker_fence: InvestigationVerificationWorkerFence,
        wrapper_name: String,
        selected_tool_name: Option<String>,
        selected_tool_config_sha256: Option<String>,
        model_args_sha256: String,
    ) -> anyhow::Result<InvestigationVerificationInvocationGuard> {
        let fence = db::VerificationWorkerFenceInput {
            worker_run_id: worker_fence.worker_run_id,
            lease_token: worker_fence.lease_token,
            attempt_epoch: worker_fence.attempt_epoch,
            checkpoint_version: worker_fence.checkpoint_version,
        };
        let row = db::load_invocation_guard(
            &self.pool,
            invocation_id,
            &fence,
            &wrapper_name,
            selected_tool_name.as_deref(),
            selected_tool_config_sha256.as_deref(),
            &model_args_sha256,
        )
        .await?;
        Ok(InvestigationVerificationInvocationGuard {
            target_live_id: row.target_live_id,
            organization_id: row.organization_id,
            target_project_path: row.target_project_path,
            target_name: row.target_name,
            target_value_at_freeze: row.target_value_at_freeze,
            target_ports: row.target_ports,
        })
    }
}

fn map_error(error: golish_db::DbError) -> InvestigationAssetVerificationRepositoryError {
    let detail = error.to_string();
    if detail.contains(db::CONTRACT_INVALID) {
        InvestigationAssetVerificationRepositoryError::InvalidRequest { detail }
    } else if detail.contains(db::REPLAY_DRIFT)
        || detail.contains(db::CAS_CONFLICT)
        || detail.contains("40001")
    {
        InvestigationAssetVerificationRepositoryError::Conflict { detail }
    } else if detail.contains(db::AUTHORITY_MISMATCH)
        || detail.contains("AUTHORITY")
        || detail.contains("SCOPE")
    {
        InvestigationAssetVerificationRepositoryError::AuthorityMismatch { detail }
    } else if matches!(
        error,
        golish_db::DbError::Sqlx(sqlx::Error::RowNotFound) | golish_db::DbError::NotFound(_)
    ) {
        InvestigationAssetVerificationRepositoryError::NotFound { detail }
    } else {
        InvestigationAssetVerificationRepositoryError::Infrastructure { detail }
    }
}
fn strings(value: serde_json::Value) -> InvestigationAssetVerificationResult<Vec<String>> {
    serde_json::from_value(value).map_err(|error| {
        InvestigationAssetVerificationRepositoryError::Infrastructure {
            detail: error.to_string(),
        }
    })
}
fn actor(row: db::AssetVerificationActorRow) -> InvestigationAssetVerificationActorView {
    InvestigationAssetVerificationActorView {
        role: row.role,
        work_item_id: row.work_item_id,
        worker_run_id: row.worker_run_id,
        message_chain_id: row.message_chain_id,
    }
}
fn dynamic_actor_call(
    row: db::DynamicVerificationActorCallRow,
) -> InvestigationDynamicVerificationActorCallView {
    InvestigationDynamicVerificationActorCallView {
        actor_call_id: row.actor_call_id,
        stable_request_id: row.stable_request_id,
        session_id: row.session_id,
        actor_ordinal: row.actor_ordinal,
        subtask_id: row.subtask_id,
        specialist_role: row.specialist_role,
        objective_redacted: row.objective_redacted,
        objective_sha256: row.objective_sha256,
        work_item_id: row.work_item_id,
        worker_run_id: row.worker_run_id,
        message_chain_id: row.message_chain_id,
        primary_turn_id: row.primary_turn_id,
        turn_actor_ordinal: row.turn_actor_ordinal,
        actor_call_sha256: row.actor_call_sha256,
        state: row.state,
        created_at: row.created_at,
        completed_at: row.completed_at,
        replayed: row.replayed,
    }
}
fn dynamic_round(
    row: db::DynamicVerificationRoundRow,
) -> InvestigationDynamicVerificationRoundView {
    InvestigationDynamicVerificationRoundView {
        session_id: row.session_id,
        stable_request_id: row.stable_request_id,
        operation_id: row.operation_id,
        project_scope_id: row.project_scope_id,
        stage_execution_id: row.stage_execution_id,
        stage_run_unit_id: row.stage_run_unit_id,
        scope_snapshot_id: row.scope_snapshot_id,
        organization_id: row.organization_id,
        asset_lane_id: row.asset_lane_id,
        target_live_id: row.target_live_id,
        hypothesis_revision_id: row.hypothesis_revision_id,
        verification_task_id: row.verification_task_id,
        evolution_epoch: row.evolution_epoch,
        stage_team_plan_id: row.stage_team_plan_id,
        dispatch_epoch: row.dispatch_epoch,
        session_authorization_id: row.session_authorization_id,
        authorization_expires_at: row.authorization_expires_at,
        session_budget_envelope_id: row.session_budget_envelope_id,
        source_primary_work_item_id: row.source_primary_work_item_id,
        source_primary_worker_run_id: row.source_primary_worker_run_id,
        primary: actor(row.primary),
        actor_calls: row
            .actor_calls
            .into_iter()
            .map(dynamic_actor_call)
            .collect(),
        maximum_primary_turns: row.maximum_primary_turns,
        consumed_primary_turns: row.consumed_primary_turns,
        maximum_actor_calls: row.maximum_actor_calls,
        consumed_actor_calls: row.consumed_actor_calls,
        state: if row.state == "open" {
            InvestigationAssetVerificationSessionState::Open
        } else {
            InvestigationAssetVerificationSessionState::Resolved
        },
        head_version: row.head_version,
        resolution_authority_id: row.resolution_authority_id,
        opened_at: row.opened_at,
        resolved_at: row.resolved_at,
        replayed: row.replayed,
    }
}
fn dynamic_primary_turn(
    row: db::DynamicVerificationPrimaryTurnRow,
) -> InvestigationDynamicVerificationPrimaryTurnView {
    InvestigationDynamicVerificationPrimaryTurnView {
        primary_turn_id: row.primary_turn_id,
        stable_request_id: row.stable_request_id,
        session_id: row.session_id,
        turn_ordinal: row.turn_ordinal,
        decision_kind: row.decision_kind,
        expected_session_head_version: row.expected_session_head_version,
        source_primary_checkpoint_version: row.source_primary_checkpoint_version,
        source_primary_checkpoint_sha256: row.source_primary_checkpoint_sha256,
        source_tool_call_record_id: row.source_tool_call_record_id,
        source_provider_call_id: row.source_provider_call_id,
        canonical_turn_sha256: row.canonical_turn_sha256,
        actor_call_set_sha256: row.actor_call_set_sha256,
        actors: row.actors.into_iter().map(dynamic_actor_call).collect(),
        replayed: row.replayed,
    }
}
fn fence(value: &InvestigationAssetVerificationWorkerFence) -> db::VerificationWorkerFenceInput {
    db::VerificationWorkerFenceInput {
        worker_run_id: value.worker_run_id,
        lease_token: value.lease_token,
        attempt_epoch: value.attempt_epoch,
        checkpoint_version: value.checkpoint_version,
    }
}
fn invocation_state(
    value: &str,
) -> InvestigationAssetVerificationResult<InvestigationAssetVerificationInvocationState> {
    match value {
        "running" => Ok(InvestigationAssetVerificationInvocationState::Running),
        "succeeded" => Ok(InvestigationAssetVerificationInvocationState::Succeeded),
        "failed" => Ok(InvestigationAssetVerificationInvocationState::Failed),
        "outcome_unknown" => Ok(InvestigationAssetVerificationInvocationState::OutcomeUnknown),
        _ => Err(
            InvestigationAssetVerificationRepositoryError::Infrastructure {
                detail: format!("unknown invocation state {value}"),
            },
        ),
    }
}
fn invocation(
    row: db::AssetVerificationInvocationRow,
) -> InvestigationAssetVerificationResult<InvestigationAssetVerificationInvocationView> {
    let session_id = row.dynamic_session_id.or(row.session_id).ok_or_else(|| {
        InvestigationAssetVerificationRepositoryError::Infrastructure {
            detail: "verification invocation has no owning session".into(),
        }
    })?;
    Ok(InvestigationAssetVerificationInvocationView {
        invocation_id: row.invocation_id,
        stable_request_id: row.stable_request_id,
        session_id,
        invocation_ordinal: row.invocation_ordinal,
        actor_call_id: row.actor_call_id,
        actor_ordinal: row.actor_ordinal,
        actor_subtask_id: row.actor_subtask_id,
        actor_role: row.actor_role,
        actor_work_item_id: row.actor_work_item_id,
        actor_worker_run_id: row.actor_worker_run_id,
        actor_message_chain_id: row.actor_message_chain_id,
        inventory_snapshot_id: row.inventory_snapshot_id,
        inventory_member_id: row.inventory_member_id,
        wrapper_name: row.wrapper_name,
        selected_tool_name: row.selected_tool_name,
        selected_tool_config_sha256: row.selected_tool_config_sha256,
        invocation_authorization_id: row.invocation_authorization_id,
        invocation_authorization_sha256: row.invocation_authorization_sha256,
        invocation_authorization_expires_at: row.invocation_authorization_expires_at,
        effect_class: row.effect_class,
        risk_tier: row.risk_tier,
        credential_binding_sha256: row.credential_binding_sha256,
        network_request_limit: row.network_request_limit,
        wall_time_limit_ms: row.wall_time_limit_ms,
        output_byte_limit: row.output_byte_limit,
        model_args_redacted: row.model_args_redacted,
        model_args_sha256: row.model_args_sha256,
        request_manifest_sha256: row.request_manifest_sha256,
        state: invocation_state(&row.state)?,
        row_version: row.row_version,
        capability_execution_receipt_id: row.capability_execution_receipt_id,
        oracle_receipt_id: row.oracle_receipt_id,
        audit_evidence_ids: row.audit_evidence_ids,
        evidence_set_sha256: row.evidence_set_sha256,
        redacted_result: row.redacted_result,
        result_sha256: row.result_sha256,
        started_at: row.started_at,
        completed_at: row.completed_at,
        replayed: row.replayed,
    })
}
fn inventory_member(
    row: db::DynamicToolInventoryMemberRow,
) -> InvestigationAssetVerificationResult<DynamicToolInventoryMemberView> {
    Ok(DynamicToolInventoryMemberView {
        inventory_member_id: row.inventory_member_id,
        member_ordinal: row.member_ordinal,
        tool_id: row.tool_id,
        tool_name: row.tool_name,
        config_sha256: row.config_sha256,
        executable_identity_sha256: row.executable_identity_sha256,
        runtime: row.runtime,
        runtime_version: row.runtime_version,
        launch_mode: row.launch_mode,
        parameter_schema: row.parameter_schema,
        output_schema: row.output_schema,
        tags: strings(row.tags)?,
        member_sha256: row.member_sha256,
    })
}
fn inventory(
    row: db::DynamicToolInventoryRow,
) -> InvestigationAssetVerificationResult<InvestigationDynamicToolInventoryView> {
    Ok(InvestigationDynamicToolInventoryView {
        inventory_snapshot_id: row.inventory_snapshot_id,
        stable_request_id: row.stable_request_id,
        session_id: row.session_id,
        inventory_source_sha256: row.inventory_source_sha256,
        member_count: row.member_count,
        member_set_sha256: row.member_set_sha256,
        members: row
            .members
            .into_iter()
            .map(inventory_member)
            .collect::<InvestigationAssetVerificationResult<_>>()?,
        sealed_at: row.sealed_at,
        replayed: row.replayed,
    })
}
fn discovery(
    row: db::PendingHypothesisDiscoveryRow,
) -> InvestigationPendingHypothesisDiscoveryView {
    InvestigationPendingHypothesisDiscoveryView {
        discovery_authority_id: row.discovery_authority_id,
        resolution_authority_id: row.resolution_authority_id,
        session_id: row.session_id,
        asset_lane_id: row.asset_lane_id,
        target_live_id: row.target_live_id,
        source_hypothesis_revision_id: row.source_hypothesis_revision_id,
        discovery_ordinal: row.discovery_ordinal,
        subject_kind: row.subject_kind,
        subject_identity_sha256: row.subject_identity_sha256,
        semantic_key_sha256: row.semantic_key_sha256,
        canonical_proposal: row.canonical_proposal,
        structured_claim: row.structured_claim,
        structured_claim_sha256: row.structured_claim_sha256,
        rationale_redacted: row.rationale_redacted,
        discovery_sha256: row.discovery_sha256,
    }
}
fn resolution_disposition(
    value: &str,
) -> InvestigationAssetVerificationResult<InvestigationHypothesisResolutionDisposition> {
    match value {
        "verified" => Ok(InvestigationHypothesisResolutionDisposition::Verified),
        "refuted" => Ok(InvestigationHypothesisResolutionDisposition::Refuted),
        "invalid" => Ok(InvestigationHypothesisResolutionDisposition::Invalid),
        _ => Err(
            InvestigationAssetVerificationRepositoryError::Infrastructure {
                detail: format!("unknown resolution {value}"),
            },
        ),
    }
}
fn dynamic_resolution(
    row: db::DynamicHypothesisResolutionRow,
) -> InvestigationAssetVerificationResult<InvestigationDynamicHypothesisResolutionView> {
    Ok(InvestigationDynamicHypothesisResolutionView {
        resolution_authority_id: row.resolution_authority_id,
        stable_request_id: row.stable_request_id,
        session_id: row.session_id,
        asset_lane_id: row.asset_lane_id,
        target_live_id: row.target_live_id,
        hypothesis_revision_id: row.hypothesis_revision_id,
        primary_work_item_id: row.primary_work_item_id,
        primary_worker_run_id: row.primary_worker_run_id,
        primary_message_chain_id: row.primary_message_chain_id,
        disposition: resolution_disposition(&row.disposition)?,
        primary_conclusion_sha256: row.primary_conclusion_sha256,
        conclusion_redacted: row.conclusion_redacted,
        citation_count: row.citation_count,
        citation_set_sha256: row.citation_set_sha256,
        resolution_sha256: row.resolution_sha256,
        new_hypothesis_proposals: row
            .new_hypothesis_proposals
            .into_iter()
            .map(discovery)
            .collect(),
        resolved_at: row.resolved_at,
        replayed: row.replayed,
    })
}

fn completed_worker(
    row: golish_db::repo::stage_teams::CompletedStageWorkerRow,
) -> InvestigationAssetVerificationResult<CompletedStageWorkerView> {
    let plan = stage_team_plan_from_db(row.plan).map_err(|error| {
        InvestigationAssetVerificationRepositoryError::Infrastructure {
            detail: error.to_string(),
        }
    })?;
    let aggregator_role = plan.aggregator_role.clone();
    Ok(CompletedStageWorkerView {
        unit: runtime_stage_unit_from_db(row.unit).map_err(|error| {
            InvestigationAssetVerificationRepositoryError::Infrastructure {
                detail: error.to_string(),
            }
        })?,
        work_item: stage_work_item_from_db(row.work_item, aggregator_role.as_deref()).map_err(
            |error| InvestigationAssetVerificationRepositoryError::Infrastructure {
                detail: error.to_string(),
            },
        )?,
        worker: runtime_worker_from_db(row.worker).map_err(|error| {
            InvestigationAssetVerificationRepositoryError::Infrastructure {
                detail: error.to_string(),
            }
        })?,
        output: stage_worker_output_from_db(row.output).map_err(|error| {
            InvestigationAssetVerificationRepositoryError::Infrastructure {
                detail: error.to_string(),
            }
        })?,
        plan,
        replayed: row.replayed,
    })
}

#[async_trait]
impl InvestigationAssetVerificationRepository for PgInvestigationAssetVerificationRepository {
    async fn open_dynamic_round(
        &self,
        r: OpenInvestigationDynamicVerificationRound,
    ) -> InvestigationAssetVerificationResult<InvestigationDynamicVerificationRoundView> {
        db::open_dynamic_round(
            &self.pool,
            &db::OpenDynamicVerificationRoundInput {
                stable_request_id: r.stable_request_id,
                operation_id: r.operation_id,
                stage_execution_id: r.stage_execution_id,
                stage_run_unit_id: r.stage_run_unit_id,
                scope_snapshot_id: r.scope_snapshot_id,
                organization_id: r.organization_id,
                asset_lane_id: r.asset_lane_id,
                target_live_id: r.target_live_id,
                hypothesis_revision_id: r.hypothesis_revision_id,
                verification_task_id: r.verification_task_id,
                session_authorization_id: r.session_authorization_id,
                session_budget_envelope_id: r.session_budget_envelope_id,
            },
        )
        .await
        .map(dynamic_round)
        .map_err(map_error)
    }

    async fn load_dynamic_round(
        &self,
        session_id: uuid::Uuid,
    ) -> InvestigationAssetVerificationResult<Option<InvestigationDynamicVerificationRoundView>>
    {
        db::load_dynamic_round(&self.pool, session_id)
            .await
            .map(|row| row.map(dynamic_round))
            .map_err(map_error)
    }

    async fn renew_dynamic_authorization(
        &self,
        r: RenewInvestigationDynamicVerificationAuthorization,
    ) -> InvestigationAssetVerificationResult<
        InvestigationDynamicVerificationAuthorizationRenewalView,
    > {
        db::renew_dynamic_authorization(&self.pool, r.stable_request_id, r.renewal_id, r.session_id)
            .await
            .map(
                |row| InvestigationDynamicVerificationAuthorizationRenewalView {
                    renewal_id: row.renewal_id,
                    stable_request_id: row.stable_request_id,
                    session_id: row.session_id,
                    previous_expires_at: row.previous_expires_at,
                    renewed_expires_at: row.renewed_expires_at,
                    renewal_sha256: row.renewal_sha256,
                    replayed: row.replayed,
                },
            )
            .map_err(map_error)
    }

    async fn dispatch_dynamic_actor_batch(
        &self,
        r: DispatchInvestigationDynamicVerificationActorBatch,
    ) -> InvestigationAssetVerificationResult<InvestigationDynamicVerificationPrimaryTurnView> {
        db::dispatch_dynamic_actor_batch(
            &self.pool,
            &db::DispatchDynamicVerificationActorBatchInput {
                stable_request_id: r.stable_request_id,
                primary_turn_id: r.primary_turn_id,
                session_id: r.session_id,
                expected_session_head_version: r.expected_session_head_version,
                primary_worker_fence: fence(&r.primary_worker_fence),
                source_tool_call_record_id: r.source_tool_call_record_id,
                source_provider_call_id: r.source_provider_call_id,
                actors: r
                    .actors
                    .into_iter()
                    .map(|actor| db::DynamicVerificationActorRequestInput {
                        actor_call_id: actor.actor_call_id,
                    })
                    .collect(),
            },
        )
        .await
        .map(dynamic_primary_turn)
        .map_err(map_error)
    }

    async fn load_pending_dynamic_primary_submission(
        &self,
        session_id: uuid::Uuid,
    ) -> InvestigationAssetVerificationResult<
        Option<InvestigationDynamicVerificationPendingPrimarySubmissionView>,
    > {
        db::load_pending_dynamic_primary_submission(&self.pool, session_id)
            .await
            .map(|row| {
                row.map(
                    |row| InvestigationDynamicVerificationPendingPrimarySubmissionView {
                        session_id: row.session_id,
                        source_tool_call_record_id: row.source_tool_call_record_id,
                        source_provider_call_id: row.source_provider_call_id,
                        canonical_turn: row.canonical_turn,
                        canonical_turn_sha256: row.canonical_turn_sha256,
                    },
                )
            })
            .map_err(map_error)
    }

    async fn claim_dynamic_primary(
        &self,
        r: ClaimInvestigationDynamicVerificationPrimary,
    ) -> InvestigationAssetVerificationResult<ClaimedStageWorkItemView> {
        db::claim_dynamic_primary(
            &self.pool,
            &db::ClaimDynamicVerificationPrimaryInput {
                session_id: r.session_id,
                lease_owner: r.lease_owner,
                lease_seconds: r.lease_seconds,
            },
        )
        .await
        .map_err(map_error)
        .and_then(|row| {
            claimed_stage_work_item_from_db(row).map_err(|error| {
                InvestigationAssetVerificationRepositoryError::Infrastructure {
                    detail: error.to_string(),
                }
            })
        })
    }

    async fn park_dynamic_primary(
        &self,
        r: ParkInvestigationDynamicVerificationPrimary,
    ) -> InvestigationAssetVerificationResult<ClaimedStageWorkItemView> {
        db::park_dynamic_primary(
            &self.pool,
            &db::ParkDynamicVerificationPrimaryInput {
                session_id: r.session_id,
                worker_fence: fence(&r.worker_fence),
                checkpoint: r.checkpoint,
                evidence_watermark: r.evidence_watermark,
            },
        )
        .await
        .map_err(map_error)
        .and_then(|row| {
            claimed_stage_work_item_from_db(row).map_err(|error| {
                InvestigationAssetVerificationRepositoryError::Infrastructure {
                    detail: error.to_string(),
                }
            })
        })
    }

    async fn claim_dynamic_actor(
        &self,
        r: ClaimInvestigationDynamicVerificationActor,
    ) -> InvestigationAssetVerificationResult<ClaimedStageWorkItemView> {
        db::claim_dynamic_actor(
            &self.pool,
            &db::ClaimDynamicVerificationActorInput {
                session_id: r.session_id,
                actor_call_id: r.actor_call_id,
                lease_owner: r.lease_owner,
                lease_seconds: r.lease_seconds,
            },
        )
        .await
        .map_err(map_error)
        .and_then(|row| {
            claimed_stage_work_item_from_db(row).map_err(|error| {
                InvestigationAssetVerificationRepositoryError::Infrastructure {
                    detail: error.to_string(),
                }
            })
        })
    }

    async fn load_dynamic_actor_completion(
        &self,
        r: LoadInvestigationDynamicVerificationActorCompletion,
    ) -> InvestigationAssetVerificationResult<Option<CompletedStageWorkerView>> {
        db::load_dynamic_actor_completion(&self.pool, r.session_id, r.actor_call_id)
            .await
            .map_err(map_error)?
            .map(completed_worker)
            .transpose()
    }

    async fn load_pending_dynamic_actor_submission(
        &self,
        session_id: uuid::Uuid,
        actor_call_id: uuid::Uuid,
    ) -> InvestigationAssetVerificationResult<
        Option<InvestigationDynamicVerificationPendingActorSubmissionView>,
    > {
        db::load_pending_dynamic_actor_submission(&self.pool, session_id, actor_call_id)
            .await
            .map(|row| {
                row.map(
                    |row| InvestigationDynamicVerificationPendingActorSubmissionView {
                        session_id: row.session_id,
                        actor_call_id: row.actor_call_id,
                        source_tool_call_record_id: row.source_tool_call_record_id,
                        source_provider_call_id: row.source_provider_call_id,
                        canonical_observation: row.canonical_observation,
                        canonical_observation_sha256: row.canonical_observation_sha256,
                    },
                )
            })
            .map_err(map_error)
    }

    async fn list_dynamic_invocation_authorities(
        &self,
        r: ListInvestigationDynamicVerificationInvocationAuthorities,
    ) -> InvestigationAssetVerificationResult<
        Vec<InvestigationDynamicVerificationInvocationAuthorityView>,
    > {
        db::list_dynamic_invocation_authorities(&self.pool, r.session_id, r.actor_call_id)
            .await
            .map_err(map_error)?
            .into_iter()
            .map(|row| {
                Ok(InvestigationDynamicVerificationInvocationAuthorityView {
                    invocation_id: row.invocation_id,
                    actor_call_id: row.actor_call_id,
                    actor_ordinal: row.actor_ordinal,
                    specialist_role: row.specialist_role,
                    state: invocation_state(&row.state)?,
                    capability_execution_receipt_id: row.capability_execution_receipt_id,
                    oracle_receipt_id: row.oracle_receipt_id,
                    audit_evidence_ids: row.audit_evidence_ids,
                    evidence_set_sha256: row.evidence_set_sha256,
                    result_sha256: row.result_sha256,
                })
            })
            .collect()
    }

    async fn park_dynamic_actor(
        &self,
        r: ParkInvestigationDynamicVerificationActor,
    ) -> InvestigationAssetVerificationResult<ClaimedStageWorkItemView> {
        db::park_dynamic_actor(
            &self.pool,
            &db::ParkDynamicVerificationActorInput {
                session_id: r.session_id,
                actor_call_id: r.actor_call_id,
                worker_fence: fence(&r.worker_fence),
                checkpoint: r.checkpoint,
                evidence_watermark: r.evidence_watermark,
            },
        )
        .await
        .map_err(map_error)
        .and_then(|row| {
            claimed_stage_work_item_from_db(row).map_err(|error| {
                InvestigationAssetVerificationRepositoryError::Infrastructure {
                    detail: error.to_string(),
                }
            })
        })
    }

    async fn complete_dynamic_actor(
        &self,
        r: CompleteInvestigationDynamicVerificationActor,
    ) -> InvestigationAssetVerificationResult<CompletedStageWorkerView> {
        db::complete_dynamic_actor(
            &self.pool,
            &db::CompleteDynamicVerificationActorInput {
                session_id: r.session_id,
                actor_call_id: r.actor_call_id,
                worker_fence: fence(&r.worker_fence),
                expected_work_item_row_version: r.expected_work_item_row_version,
                source_tool_call_record_id: r.source_tool_call_record_id,
                source_provider_call_id: r.source_provider_call_id,
                terminal_checkpoint: r.terminal_checkpoint,
                evidence_watermark: r.evidence_watermark,
            },
        )
        .await
        .map_err(map_error)
        .and_then(completed_worker)
    }

    async fn resolve_dynamic_hypothesis(
        &self,
        r: ResolveInvestigationDynamicHypothesis,
    ) -> InvestigationAssetVerificationResult<InvestigationDynamicHypothesisResolutionView> {
        db::resolve_dynamic_hypothesis(
            &self.pool,
            &db::ResolveDynamicHypothesisInput {
                stable_request_id: r.stable_request_id,
                resolution_authority_id: r.resolution_authority_id,
                session_id: r.session_id,
                expected_session_head_version: r.expected_session_head_version,
                primary_worker_fence: fence(&r.primary_worker_fence),
                primary_turn_id: r.primary_turn_id,
                source_tool_call_record_id: r.source_tool_call_record_id,
                source_provider_call_id: r.source_provider_call_id,
            },
        )
        .await
        .map_err(map_error)
        .and_then(dynamic_resolution)
    }

    async fn load_pending_dynamic_primary_terminalization(
        &self,
        r: LoadPendingInvestigationDynamicVerificationPrimaryTerminalization,
    ) -> InvestigationAssetVerificationResult<
        Option<PendingInvestigationDynamicVerificationPrimaryTerminalizationView>,
    > {
        db::load_pending_dynamic_primary_terminalization(
            &self.pool,
            r.operation_id,
            r.asset_lane_id,
        )
        .await
        .map_err(map_error)?
        .map(|row| {
            Ok(
                PendingInvestigationDynamicVerificationPrimaryTerminalizationView {
                    round: dynamic_round(row.round),
                    resolution: dynamic_resolution(row.resolution)?,
                    primary_worker_fence: row.primary_worker_fence.map(|value| {
                        InvestigationAssetVerificationWorkerFence {
                            worker_run_id: value.worker_run_id,
                            lease_token: value.lease_token,
                            attempt_epoch: value.attempt_epoch,
                            checkpoint_version: value.checkpoint_version,
                        }
                    }),
                    expected_work_item_row_version: row.expected_work_item_row_version,
                    expected_plan_row_version: row.expected_plan_row_version,
                },
            )
        })
        .transpose()
    }

    async fn complete_dynamic_primary(
        &self,
        r: CompleteInvestigationDynamicVerificationPrimary,
    ) -> InvestigationAssetVerificationResult<
        ResolvedAndTerminalizedInvestigationDynamicHypothesisView,
    > {
        let (resolution, completion) = db::complete_dynamic_primary(
            &self.pool,
            &db::CompleteDynamicVerificationPrimaryInput {
                session_id: r.session_id,
                resolution_authority_id: r.resolution_authority_id,
                primary_worker_fence: fence(&r.primary_worker_fence),
                expected_work_item_row_version: r.expected_work_item_row_version,
                expected_plan_row_version: r.expected_plan_row_version,
                terminal_checkpoint: r.terminal_checkpoint,
            },
        )
        .await
        .map_err(map_error)?;
        Ok(ResolvedAndTerminalizedInvestigationDynamicHypothesisView {
            resolution: dynamic_resolution(resolution)?,
            primary_completion: completed_worker(completion)?,
        })
    }

    async fn authorize_session(
        &self,
        r: AuthorizeInvestigationAssetVerificationSession,
    ) -> InvestigationAssetVerificationResult<InvestigationAssetVerificationSessionAuthorizationView>
    {
        let row = db::authorize_session(
            &self.pool,
            &db::AuthorizeAssetVerificationSessionInput {
                stable_request_id: r.stable_request_id,
                session_authorization_id: r.session_authorization_id,
                session_budget_envelope_id: r.session_budget_envelope_id,
                operation_id: r.operation_id,
                stage_execution_id: r.stage_execution_id,
                stage_run_unit_id: r.stage_run_unit_id,
                scope_snapshot_id: r.scope_snapshot_id,
                organization_id: r.organization_id,
                asset_lane_id: r.asset_lane_id,
                target_live_id: r.target_live_id,
                hypothesis_revision_id: r.hypothesis_revision_id,
                verification_task_id: r.verification_task_id,
                allowed_effect_classes: r.allowed_effect_classes,
                maximum_risk_tier: r.maximum_risk_tier,
                allowed_credential_binding_sha256s: r.allowed_credential_binding_sha256s,
                credential_binding_set_sha256: r.credential_binding_set_sha256,
                maximum_invocations: r.maximum_invocations,
                maximum_network_requests: r.maximum_network_requests,
                maximum_wall_time_ms: r.maximum_wall_time_ms,
                maximum_output_bytes: r.maximum_output_bytes,
                maximum_parallel_invocations: r.maximum_parallel_invocations,
            },
        )
        .await
        .map_err(map_error)?;
        Ok(InvestigationAssetVerificationSessionAuthorizationView {
            session_authorization_id: row.session_authorization_id,
            session_budget_envelope_id: row.session_budget_envelope_id,
            operation_id: row.operation_id,
            project_scope_id: row.project_scope_id,
            stage_execution_id: row.stage_execution_id,
            stage_run_unit_id: row.stage_run_unit_id,
            scope_snapshot_id: row.scope_snapshot_id,
            organization_id: row.organization_id,
            asset_lane_id: row.asset_lane_id,
            target_live_id: row.target_live_id,
            hypothesis_revision_id: row.hypothesis_revision_id,
            verification_task_id: row.verification_task_id,
            allowed_effect_classes: strings(row.allowed_effect_classes)?,
            maximum_risk_tier: row.maximum_risk_tier,
            allowed_credential_binding_sha256s: strings(row.allowed_credential_binding_sha256s)?,
            credential_binding_set_sha256: row.credential_binding_set_sha256,
            authorization_sha256: row.authorization_sha256,
            expires_at: row.expires_at,
            maximum_invocations: row.maximum_invocations,
            remaining_invocations: row.remaining_invocations,
            maximum_network_requests: row.maximum_network_requests,
            remaining_network_requests: row.remaining_network_requests,
            maximum_wall_time_ms: row.maximum_wall_time_ms,
            remaining_wall_time_ms: row.remaining_wall_time_ms,
            maximum_output_bytes: row.maximum_output_bytes,
            remaining_output_bytes: row.remaining_output_bytes,
            maximum_parallel_invocations: row.maximum_parallel_invocations,
            replayed: row.replayed,
        })
    }
    async fn load_next_unresolved_current_hypothesis(
        &self,
        r: LoadNextInvestigationAssetVerificationCandidate,
    ) -> InvestigationAssetVerificationResult<Option<InvestigationAssetVerificationCandidateView>>
    {
        db::load_next_unresolved_current_hypothesis(&self.pool, r.operation_id, r.asset_lane_id)
            .await
            .map(|row| {
                row.map(|row| InvestigationAssetVerificationCandidateView {
                    operation_id: row.operation_id,
                    stage_execution_id: row.stage_execution_id,
                    stage_run_unit_id: row.stage_run_unit_id,
                    scope_snapshot_id: row.scope_snapshot_id,
                    organization_id: row.organization_id,
                    asset_lane_id: row.asset_lane_id,
                    target_live_id: row.target_live_id,
                    hypothesis_root_id: row.hypothesis_root_id,
                    hypothesis_revision_id: row.hypothesis_revision_id,
                    hypothesis_revision_sha256: row.hypothesis_revision_sha256,
                    hypothesis_claim: row.hypothesis_claim,
                    hypothesis_claim_sha256: row.hypothesis_claim_sha256,
                    falsification_conditions: row.falsification_conditions,
                    falsification_conditions_sha256: row.falsification_conditions_sha256,
                    verification_objectives: row.verification_objectives,
                    verification_objectives_sha256: row.verification_objectives_sha256,
                    hypothesis_head_version: row.hypothesis_head_version,
                    verification_task_id: row.verification_task_id,
                    verification_plan_id: row.verification_plan_id,
                    verification_plan_sha256: row.verification_plan_sha256,
                    priority: row.priority,
                    existing_open_round_id: row.existing_open_round_id,
                })
            })
            .map_err(map_error)
    }
    async fn freeze_dynamic_inventory(
        &self,
        r: FreezeInvestigationDynamicToolInventory,
    ) -> InvestigationAssetVerificationResult<InvestigationDynamicToolInventoryView> {
        let row = db::freeze_dynamic_inventory(
            &self.pool,
            &db::FreezeDynamicToolInventoryInput {
                stable_request_id: r.stable_request_id,
                session_id: r.session_id,
                inventory_source_sha256: r.inventory_source_sha256,
                members: r
                    .members
                    .into_iter()
                    .map(|m| db::DynamicToolInventoryMemberInput {
                        tool_id: m.tool_id,
                        tool_name: m.tool_name,
                        config_sha256: m.config_sha256,
                        executable_identity_sha256: m.executable_identity_sha256,
                        runtime: m.runtime,
                        runtime_version: m.runtime_version,
                        launch_mode: m.launch_mode,
                        parameter_schema: m.parameter_schema,
                        output_schema: m.output_schema,
                        tags: m.tags,
                    })
                    .collect(),
            },
        )
        .await
        .map_err(map_error)?;
        inventory(row)
    }
    async fn begin_invocation(
        &self,
        r: BeginInvestigationAssetVerificationInvocation,
    ) -> InvestigationAssetVerificationResult<InvestigationAssetVerificationInvocationView> {
        let row = db::begin_invocation(
            &self.pool,
            &db::BeginAssetVerificationInvocationInput {
                stable_request_id: r.stable_request_id,
                invocation_id: r.invocation_id,
                session_id: r.session_id,
                actor_call_id: r.actor_call_id,
                worker_fence: fence(&r.worker_fence),
                wrapper_name: r.wrapper_name,
                selected_tool_name: r.selected_tool_name,
                credential_binding_sha256: r.credential_binding_sha256,
                model_args_redacted: r.model_args_redacted,
                model_args_sha256: r.model_args_sha256,
            },
        )
        .await
        .map_err(map_error)?;
        invocation(row)
    }
    async fn complete_invocation(
        &self,
        r: CompleteInvestigationAssetVerificationInvocation,
    ) -> InvestigationAssetVerificationResult<InvestigationAssetVerificationInvocationView> {
        let row = db::complete_invocation(
            &self.pool,
            &db::CompleteAssetVerificationInvocationInput {
                stable_request_id: r.stable_request_id,
                invocation_id: r.invocation_id,
                expected_row_version: r.expected_row_version,
                worker_fence: fence(&r.worker_fence),
                disposition: match r.disposition {
                    InvestigationAssetVerificationInvocationState::Running => {
                        return Err(
                            InvestigationAssetVerificationRepositoryError::InvalidRequest {
                                detail: "running is not terminal".into(),
                            },
                        )
                    }
                    InvestigationAssetVerificationInvocationState::Succeeded => "succeeded",
                    InvestigationAssetVerificationInvocationState::Failed => "failed",
                    InvestigationAssetVerificationInvocationState::OutcomeUnknown => {
                        "outcome_unknown"
                    }
                }
                .into(),
                capability_execution_receipt_id: r.capability_execution_receipt_id,
                oracle_receipt_id: r.oracle_receipt_id,
                audit_evidence_ids: r.audit_evidence_ids,
                evidence_set_sha256: r.evidence_set_sha256,
                redacted_result: r.redacted_result,
                result_sha256: r.result_sha256,
            },
        )
        .await
        .map_err(map_error)?;
        invocation(row)
    }
    async fn load_invocation_guard(
        &self,
        r: LoadInvestigationAssetVerificationInvocationGuard,
    ) -> InvestigationAssetVerificationResult<InvestigationAssetVerificationInvocationGuardView>
    {
        let row = db::load_invocation_guard(
            &self.pool,
            r.invocation_id,
            &fence(&r.worker_fence),
            &r.wrapper_name,
            r.selected_tool_name.as_deref(),
            r.selected_tool_config_sha256.as_deref(),
            &r.model_args_sha256,
        )
        .await
        .map_err(map_error)?;
        Ok(InvestigationAssetVerificationInvocationGuardView {
            invocation_id: row.invocation_id,
            session_id: row.session_id,
            operation_id: row.operation_id,
            project_scope_id: row.project_scope_id,
            stage_execution_id: row.stage_execution_id,
            stage_run_unit_id: row.stage_run_unit_id,
            scope_snapshot_id: row.scope_snapshot_id,
            organization_id: row.organization_id,
            asset_lane_id: row.asset_lane_id,
            target_live_id: row.target_live_id,
            target_type_at_freeze: row.target_type_at_freeze,
            target_value_at_freeze: row.target_value_at_freeze,
            target_name: row.target_name,
            target_project_path: row.target_project_path,
            target_ports: row.target_ports,
            session_authorization_id: row.session_authorization_id,
            session_authorization_sha256: row.session_authorization_sha256,
            authorization_expires_at: row.authorization_expires_at,
            session_budget_envelope_id: row.session_budget_envelope_id,
            invocation_authorization_id: row.invocation_authorization_id,
            invocation_authorization_sha256: row.invocation_authorization_sha256,
            invocation_authorization_expires_at: row.invocation_authorization_expires_at,
            actor_call_id: row.actor_call_id,
            actor_ordinal: row.actor_ordinal,
            actor_subtask_id: row.actor_subtask_id,
            actor_role: row.actor_role,
            actor_work_item_id: row.actor_work_item_id,
            actor_worker_run_id: row.actor_worker_run_id,
            actor_message_chain_id: row.actor_message_chain_id,
            inventory_snapshot_id: row.inventory_snapshot_id,
            inventory_member_id: row.inventory_member_id,
            selected_tool_name: row.selected_tool_name,
            selected_tool_config_sha256: row.selected_tool_config_sha256,
        })
    }
    async fn list_pending_hypothesis_discoveries(
        &self,
        r: ListPendingInvestigationHypothesisDiscoveries,
    ) -> InvestigationAssetVerificationResult<Vec<InvestigationPendingHypothesisDiscoveryView>>
    {
        db::list_pending_hypothesis_discoveries(&self.pool, r.operation_id, r.asset_lane_id)
            .await
            .map(|rows| rows.into_iter().map(discovery).collect())
            .map_err(map_error)
    }
    async fn admit_or_dismiss_pending_hypothesis_discovery(
        &self,
        r: AdmitOrDismissInvestigationPendingHypothesisDiscovery,
    ) -> InvestigationAssetVerificationResult<InvestigationPendingHypothesisDiscoveryConsumptionView>
    {
        let row = db::admit_or_dismiss_pending_hypothesis_discovery(
            &self.pool,
            &db::AdmitOrDismissPendingHypothesisDiscoveryInput {
                stable_request_id: r.stable_request_id,
                discovery_authority_id: r.discovery_authority_id,
                expected_asset_lane_id: r.expected_asset_lane_id,
                expected_session_id: r.expected_session_id,
            },
        )
        .await
        .map_err(map_error)?;
        Ok(InvestigationPendingHypothesisDiscoveryConsumptionView {
            consumption_id: row.consumption_id,
            discovery_authority_id: row.discovery_authority_id,
            asset_lane_id: row.asset_lane_id,
            target_live_id: row.target_live_id,
            disposition: if row.disposition == "admitted" {
                InvestigationHypothesisDiscoveryConsumptionDisposition::Admitted
            } else {
                InvestigationHypothesisDiscoveryConsumptionDisposition::DismissedDuplicate
            },
            admitted_root_id: row.admitted_root_id,
            admitted_revision_id: row.admitted_revision_id,
            compiler_receipt_id: row.compiler_receipt_id,
            duplicate_of_revision_id: row.duplicate_of_revision_id,
            consumption_sha256: row.consumption_sha256,
            consumed_at: row.consumed_at,
            replayed: row.replayed,
        })
    }
}
