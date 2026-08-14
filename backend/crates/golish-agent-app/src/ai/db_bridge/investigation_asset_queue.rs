//! App-owned adapter for the durable Investigation company/asset queue.

use std::sync::Arc;

use async_trait::async_trait;
use golish_agent_kit::db_traits::*;
use golish_db::repo::investigation_asset_queue as db;
use sqlx::PgPool;

#[derive(Clone)]
pub struct PgInvestigationAssetQueueRepository {
    pool: Arc<PgPool>,
}

impl PgInvestigationAssetQueueRepository {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

fn map_error(error: golish_db::DbError) -> InvestigationAssetQueueRepositoryError {
    let detail = error.to_string();
    if detail.contains(db::CONTRACT_INVALID) {
        InvestigationAssetQueueRepositoryError::InvalidRequest { detail }
    } else if detail.contains(db::AUTHORITY_MISMATCH)
        || detail.contains("AUTHORITY_MISMATCH")
        || detail.contains("ORDER_CONFLICT")
        || detail.contains("ASSETS_OPEN")
        || detail.contains("RECEIPT_REQUIRED")
    {
        InvestigationAssetQueueRepositoryError::AuthorityMismatch { detail }
    } else if detail.contains(db::CAS_CONFLICT)
        || detail.contains(db::REPLAY_DRIFT)
        || detail.contains("CAS_CONFLICT")
        || detail.contains("REPLAY_DRIFT")
        || detail.contains("40001")
    {
        InvestigationAssetQueueRepositoryError::Conflict { detail }
    } else if matches!(
        error,
        golish_db::DbError::Sqlx(sqlx::Error::RowNotFound) | golish_db::DbError::NotFound(_)
    ) {
        InvestigationAssetQueueRepositoryError::NotFound { detail }
    } else {
        InvestigationAssetQueueRepositoryError::Infrastructure { detail }
    }
}

fn map_asset_error(
    error: golish_db::DbError,
    asset_lane_id: uuid::Uuid,
) -> InvestigationAssetQueueRepositoryError {
    if error.to_string().contains(db::EVOLUTION_FUEL_EXHAUSTED) {
        InvestigationAssetQueueRepositoryError::EvolutionFuelExhausted { asset_lane_id }
    } else {
        map_error(error)
    }
}

fn company_state(value: &str) -> InvestigationAssetQueueResult<InvestigationCompanyLaneState> {
    match value {
        "queued" => Ok(InvestigationCompanyLaneState::Queued),
        "active" => Ok(InvestigationCompanyLaneState::Active),
        "completed" => Ok(InvestigationCompanyLaneState::Completed),
        "blocked" => Ok(InvestigationCompanyLaneState::Blocked),
        _ => Err(InvestigationAssetQueueRepositoryError::Infrastructure {
            detail: format!("unknown Investigation company state {value}"),
        }),
    }
}

fn asset_state(value: &str) -> InvestigationAssetQueueResult<InvestigationAssetLaneState> {
    match value {
        "queued" => Ok(InvestigationAssetLaneState::Queued),
        "analyzing" => Ok(InvestigationAssetLaneState::Analyzing),
        "verifying" => Ok(InvestigationAssetLaneState::Verifying),
        "consolidating" => Ok(InvestigationAssetLaneState::Consolidating),
        "evolving" => Ok(InvestigationAssetLaneState::Evolving),
        "fixed_point" => Ok(InvestigationAssetLaneState::FixedPoint),
        "blocked" => Ok(InvestigationAssetLaneState::Blocked),
        "residual" => Ok(InvestigationAssetLaneState::Residual),
        _ => Err(InvestigationAssetQueueRepositoryError::Infrastructure {
            detail: format!("unknown Investigation asset state {value}"),
        }),
    }
}

fn company_view(
    row: db::InvestigationCompanyQueueMemberRow,
) -> InvestigationAssetQueueResult<InvestigationCompanyQueueMemberView> {
    Ok(InvestigationCompanyQueueMemberView {
        company_member_id: row.company_member_id,
        company_queue_head_version: row.company_queue_head_version,
        organization_id: row.organization_id,
        depth: u32::try_from(row.depth).map_err(|_| {
            InvestigationAssetQueueRepositoryError::Infrastructure {
                detail: "negative company queue depth".to_string(),
            }
        })?,
        ordinal: u32::try_from(row.ordinal).map_err(|_| {
            InvestigationAssetQueueRepositoryError::Infrastructure {
                detail: "negative company queue ordinal".to_string(),
            }
        })?,
        state: company_state(&row.state)?,
        row_version: row.row_version,
    })
}

fn asset_view(
    row: db::InvestigationAssetLaneRow,
) -> InvestigationAssetQueueResult<InvestigationAssetLaneView> {
    Ok(InvestigationAssetLaneView {
        asset_lane_id: row.asset_lane_id,
        asset_queue_id: row.asset_queue_id,
        asset_queue_head_version: row.asset_queue_head_version,
        company_member_id: row.company_member_id,
        organization_id: row.organization_id,
        target_id: row.target_id,
        target_type: row.target_type_at_freeze,
        target_value: row.target_value_at_freeze,
        target_source: row.target_source_at_freeze,
        target_identity_sha256: row.target_identity_sha256,
        ordinal: u32::try_from(row.ordinal).map_err(|_| {
            InvestigationAssetQueueRepositoryError::Infrastructure {
                detail: "negative asset queue ordinal".to_string(),
            }
        })?,
        state: asset_state(&row.state)?,
        evolution_epoch: u32::try_from(row.evolution_epoch).map_err(|_| {
            InvestigationAssetQueueRepositoryError::Infrastructure {
                detail: "negative asset evolution epoch".to_string(),
            }
        })?,
        max_evolution_epochs: u32::try_from(row.max_evolution_epochs).map_err(|_| {
            InvestigationAssetQueueRepositoryError::Infrastructure {
                detail: "negative asset evolution budget".to_string(),
            }
        })?,
        row_version: row.row_version,
    })
}

fn queue_view(
    row: db::InvestigationCompanyAssetQueueRow,
) -> InvestigationAssetQueueResult<InvestigationCompanyAssetQueueView> {
    Ok(InvestigationCompanyAssetQueueView {
        company_queue_id: row.company_queue_id,
        stage: UnifiedInvestigationStageIdentity {
            authority_id: row.authority_id,
            operation_id: row.operation_id,
            stage_execution_id: row.stage_execution_id,
            owning_stage_run_request_id: row.owning_stage_run_request_id,
            scope_snapshot_id: row.scope_snapshot_id,
        },
        company_member_count: u32::try_from(row.company_member_count).map_err(|_| {
            InvestigationAssetQueueRepositoryError::Infrastructure {
                detail: "invalid company member count".to_string(),
            }
        })?,
        company_member_set_sha256: row.company_member_set_sha256,
        company_head_version: row.company_head_version,
        companies: row
            .companies
            .into_iter()
            .map(company_view)
            .collect::<InvestigationAssetQueueResult<_>>()?,
        assets: row
            .assets
            .into_iter()
            .map(asset_view)
            .collect::<InvestigationAssetQueueResult<_>>()?,
        replayed: row.replayed,
    })
}

fn state_name(state: InvestigationAssetLaneState) -> &'static str {
    match state {
        InvestigationAssetLaneState::Queued => "queued",
        InvestigationAssetLaneState::Analyzing => "analyzing",
        InvestigationAssetLaneState::Verifying => "verifying",
        InvestigationAssetLaneState::Consolidating => "consolidating",
        InvestigationAssetLaneState::Evolving => "evolving",
        InvestigationAssetLaneState::FixedPoint => "fixed_point",
        InvestigationAssetLaneState::Blocked => "blocked",
        InvestigationAssetLaneState::Residual => "residual",
    }
}

fn count_u32(value: i64, field: &'static str) -> InvestigationAssetQueueResult<u32> {
    u32::try_from(value).map_err(|_| InvestigationAssetQueueRepositoryError::Infrastructure {
        detail: format!("invalid Investigation asset backlog count for {field}"),
    })
}

fn backlog_view(
    row: db::InvestigationAssetBacklogRow,
) -> InvestigationAssetQueueResult<InvestigationAssetBacklogView> {
    Ok(InvestigationAssetBacklogView {
        asset_lane: asset_view(row.asset_lane)?,
        latest_generation_id: row.latest_generation_id,
        latest_generation_seal_id: row.latest_generation_seal_id,
        generation_count: count_u32(row.generation_count, "generation_count")?,
        hypothesis_root_count: count_u32(row.hypothesis_root_count, "hypothesis_root_count")?,
        dynamically_resolved_root_count: count_u32(
            row.dynamically_resolved_root_count,
            "dynamically_resolved_root_count",
        )?,
        revision_count: count_u32(row.revision_count, "revision_count")?,
        verification_task_count: count_u32(row.verification_task_count, "verification_task_count")?,
        open_verification_task_count: count_u32(
            row.open_verification_task_count,
            "open_verification_task_count",
        )?,
        campaign_count: count_u32(row.campaign_count, "campaign_count")?,
        open_campaign_count: count_u32(row.open_campaign_count, "open_campaign_count")?,
        prepared_action_count: count_u32(row.prepared_action_count, "prepared_action_count")?,
        open_prepared_action_count: count_u32(
            row.open_prepared_action_count,
            "open_prepared_action_count",
        )?,
        action_execution_count: count_u32(row.action_execution_count, "action_execution_count")?,
        open_action_execution_count: count_u32(
            row.open_action_execution_count,
            "open_action_execution_count",
        )?,
        oracle_count: count_u32(row.oracle_count, "oracle_count")?,
        fact_delta_count: count_u32(row.fact_delta_count, "fact_delta_count")?,
        wave_count: count_u32(row.wave_count, "wave_count")?,
        advanced_wave_count: count_u32(row.advanced_wave_count, "advanced_wave_count")?,
        fixed_point_wave_count: count_u32(row.fixed_point_wave_count, "fixed_point_wave_count")?,
        pending_evolution_count: count_u32(row.pending_evolution_count, "pending_evolution_count")?,
        pending_hypothesis_discovery_count: count_u32(
            row.pending_hypothesis_discovery_count,
            "pending_hypothesis_discovery_count",
        )?,
        backlog_member_count: count_u32(row.backlog_member_count, "backlog_member_count")?,
        backlog_set_sha256: row.backlog_set_sha256,
        obligation_set_sha256: row.obligation_set_sha256,
        residual_set_sha256: row.residual_set_sha256,
        zero_hypothesis_fixed_point_receipt_id: row.zero_hypothesis_fixed_point_receipt_id,
    })
}

fn closure_view(
    publication: db::InvestigationResolutionClosurePublicationRow,
) -> InvestigationResolutionClosurePublicationView {
    InvestigationResolutionClosurePublicationView {
        publication_id: publication.publication_id,
        operation_id: publication.operation_id,
        stage_execution_id: publication.stage_execution_id,
        scope_snapshot_id: publication.scope_snapshot_id,
        member_set_sha256: publication.member_set_sha256,
        members: publication
            .members
            .into_iter()
            .map(|member| InvestigationResolutionClosureMemberView {
                organization_id: member.organization_id,
                stage_run_unit_id: member.stage_run_unit_id,
                stage_team_plan_id: member.stage_team_plan_id,
                passed_at: member.passed_at,
            })
            .collect(),
    }
}

fn progression_view(
    row: db::InvestigationAssetProgressionRow,
) -> InvestigationAssetQueueResult<InvestigationAssetProgressionView> {
    let disposition = match row.disposition {
        db::InvestigationAssetProgressionDispositionRow::NextAsset => {
            InvestigationAssetProgressionDisposition::NextAsset
        }
        db::InvestigationAssetProgressionDispositionRow::NextCompany => {
            InvestigationAssetProgressionDisposition::NextCompany
        }
        db::InvestigationAssetProgressionDispositionRow::InvestigationComplete => {
            InvestigationAssetProgressionDisposition::InvestigationComplete
        }
    };
    Ok(InvestigationAssetProgressionView {
        progression_receipt_id: row.progression_receipt_id,
        fixed_asset_lane_id: row.fixed_asset_lane_id,
        disposition,
        next_company_member_id: row.next_company_member_id,
        next_asset_lane: row.next_asset_lane.map(asset_view).transpose()?,
        company_queue_head_version: row.company_queue_head_version,
        stage_closure: row.stage_closure.map(closure_view),
        replayed: row.replayed,
    })
}

#[async_trait]
impl InvestigationAssetQueueRepository for PgInvestigationAssetQueueRepository {
    async fn freeze(
        &self,
        request: FreezeInvestigationCompanyAssetQueue,
    ) -> InvestigationAssetQueueResult<InvestigationCompanyAssetQueueView> {
        let max_evolution_epochs = i32::try_from(request.max_evolution_epochs).map_err(|_| {
            InvestigationAssetQueueRepositoryError::InvalidRequest {
                detail: "max_evolution_epochs exceeds database range".to_string(),
            }
        })?;
        queue_view(
            db::freeze_company_asset_queue(
                &self.pool,
                &db::FreezeInvestigationCompanyAssetQueueRow {
                    stable_request_id: request.stable_request_id,
                    authority_id: request.stage.authority_id,
                    operation_id: request.stage.operation_id,
                    stage_execution_id: request.stage.stage_execution_id,
                    owning_stage_run_request_id: request.stage.owning_stage_run_request_id,
                    scope_snapshot_id: request.stage.scope_snapshot_id,
                    max_evolution_epochs,
                },
            )
            .await
            .map_err(map_error)?,
        )
    }

    async fn claim_next_company(
        &self,
        request: ClaimNextInvestigationCompany,
    ) -> InvestigationAssetQueueResult<InvestigationCompanyQueueMemberView> {
        company_view(
            db::claim_next_company(
                &self.pool,
                &db::ClaimNextInvestigationCompanyRow {
                    stable_request_id: request.stable_request_id,
                    company_queue_id: request.company_queue_id,
                    operation_id: request.operation_id,
                    scope_snapshot_id: request.scope_snapshot_id,
                    expected_company_member_id: request.expected_company_member_id,
                    expected_queue_head_version: request.expected_queue_head_version,
                    expected_member_row_version: request.expected_member_row_version,
                },
            )
            .await
            .map_err(map_error)?,
        )
    }

    async fn claim_next_asset(
        &self,
        request: ClaimNextInvestigationAsset,
    ) -> InvestigationAssetQueueResult<InvestigationAssetLaneView> {
        asset_view(
            db::claim_next_asset(
                &self.pool,
                &db::ClaimNextInvestigationAssetRow {
                    stable_request_id: request.stable_request_id,
                    company_queue_id: request.company_queue_id,
                    company_member_id: request.company_member_id,
                    asset_queue_id: request.asset_queue_id,
                    operation_id: request.operation_id,
                    scope_snapshot_id: request.scope_snapshot_id,
                    organization_id: request.organization_id,
                    expected_asset_lane_id: request.expected_asset_lane_id,
                    expected_queue_head_version: request.expected_queue_head_version,
                    expected_lane_row_version: request.expected_lane_row_version,
                },
            )
            .await
            .map_err(map_error)?,
        )
    }

    async fn transition_asset(
        &self,
        request: TransitionInvestigationAssetLane,
    ) -> InvestigationAssetQueueResult<InvestigationAssetLaneView> {
        let asset_lane_id = request.asset_lane_id;
        asset_view(
            db::transition_asset_lane(
                &self.pool,
                &db::TransitionInvestigationAssetLaneRow {
                    stable_request_id: request.stable_request_id,
                    company_queue_id: request.company_queue_id,
                    company_member_id: request.company_member_id,
                    asset_queue_id: request.asset_queue_id,
                    asset_lane_id: request.asset_lane_id,
                    operation_id: request.operation_id,
                    scope_snapshot_id: request.scope_snapshot_id,
                    organization_id: request.organization_id,
                    expected_queue_head_version: request.expected_queue_head_version,
                    expected_lane_row_version: request.expected_lane_row_version,
                    from_state: state_name(request.from_state),
                    to_state: state_name(request.to_state),
                },
            )
            .await
            .map_err(|error| map_asset_error(error, asset_lane_id))?,
        )
    }

    async fn load_current_evolution_authority(
        &self,
        request: LoadCurrentInvestigationAssetEvolutionAuthority,
    ) -> InvestigationAssetQueueResult<InvestigationAssetEvolutionAuthorityView> {
        let expected_evolution_epoch =
            i32::try_from(request.expected_evolution_epoch).map_err(|_| {
                InvestigationAssetQueueRepositoryError::InvalidRequest {
                    detail: "expected_evolution_epoch exceeds database range".to_string(),
                }
            })?;
        let authority = db::load_current_evolution_authority(
            &self.pool,
            &db::LoadCurrentInvestigationAssetEvolutionAuthorityRow {
                operation_id: request.operation_id,
                stage_execution_id: request.stage_execution_id,
                scope_snapshot_id: request.scope_snapshot_id,
                organization_id: request.organization_id,
                asset_lane_id: request.asset_lane_id,
                expected_evolution_epoch,
            },
        )
        .await
        .map_err(map_error)?;
        Ok(InvestigationAssetEvolutionAuthorityView {
            asset_lane_id: authority.asset_lane_id,
            evolution_epoch: u32::try_from(authority.evolution_epoch).map_err(|_| {
                InvestigationAssetQueueRepositoryError::Infrastructure {
                    detail: "negative asset evolution epoch".to_string(),
                }
            })?,
            pending_evolution_authority_id: authority.pending_evolution_authority_id,
        })
    }

    async fn seal_zero_hypothesis_fixed_point(
        &self,
        request: SealZeroHypothesisAssetFixedPoint,
    ) -> InvestigationAssetQueueResult<InvestigationAssetFixedPointReceiptView> {
        let row = db::seal_zero_hypothesis_fixed_point(
            &self.pool,
            &db::SealZeroHypothesisAssetFixedPointRow {
                stable_request_id: request.stable_request_id,
                company_queue_id: request.company_queue_id,
                company_member_id: request.company_member_id,
                asset_queue_id: request.asset_queue_id,
                asset_lane_id: request.asset_lane_id,
                operation_id: request.operation_id,
                scope_snapshot_id: request.scope_snapshot_id,
                organization_id: request.organization_id,
                expected_queue_head_version: request.expected_queue_head_version,
                expected_lane_row_version: request.expected_lane_row_version,
            },
        )
        .await
        .map_err(map_error)?;
        Ok(InvestigationAssetFixedPointReceiptView {
            fixed_point_receipt_id: row.fixed_point_receipt_id,
            asset_lane: asset_view(row.asset_lane)?,
            receipt_sha256: row.receipt_sha256,
            replayed: row.replayed,
        })
    }

    async fn complete_company(
        &self,
        request: CompleteInvestigationCompany,
    ) -> InvestigationAssetQueueResult<InvestigationCompanyQueueMemberView> {
        company_view(
            db::complete_company(
                &self.pool,
                &db::CompleteInvestigationCompanyRow {
                    stable_request_id: request.stable_request_id,
                    company_queue_id: request.company_queue_id,
                    company_member_id: request.company_member_id,
                    operation_id: request.operation_id,
                    scope_snapshot_id: request.scope_snapshot_id,
                    organization_id: request.organization_id,
                    expected_queue_head_version: request.expected_queue_head_version,
                    expected_member_row_version: request.expected_member_row_version,
                },
            )
            .await
            .map_err(map_error)?,
        )
    }

    async fn load_backlog(
        &self,
        request: LoadInvestigationAssetBacklog,
    ) -> InvestigationAssetQueueResult<InvestigationAssetBacklogView> {
        backlog_view(
            db::load_asset_backlog(
                &self.pool,
                &db::LoadInvestigationAssetBacklogRow {
                    company_queue_id: request.company_queue_id,
                    company_member_id: request.company_member_id,
                    asset_queue_id: request.asset_queue_id,
                    asset_lane_id: request.asset_lane_id,
                    operation_id: request.operation_id,
                    scope_snapshot_id: request.scope_snapshot_id,
                    organization_id: request.organization_id,
                },
            )
            .await
            .map_err(map_error)?,
        )
    }

    async fn close_backlog_and_advance(
        &self,
        request: CloseInvestigationAssetBacklogAndAdvance,
    ) -> InvestigationAssetQueueResult<InvestigationAssetProgressionView> {
        progression_view(
            db::close_asset_backlog_and_advance(
                &self.pool,
                &db::CloseInvestigationAssetBacklogAndAdvanceRow {
                    stable_request_id: request.stable_request_id,
                    company_queue_id: request.company_queue_id,
                    company_member_id: request.company_member_id,
                    asset_queue_id: request.asset_queue_id,
                    asset_lane_id: request.asset_lane_id,
                    operation_id: request.operation_id,
                    scope_snapshot_id: request.scope_snapshot_id,
                    organization_id: request.organization_id,
                    expected_company_queue_head_version: request
                        .expected_company_queue_head_version,
                    expected_company_member_row_version: request
                        .expected_company_member_row_version,
                    expected_asset_queue_head_version: request.expected_asset_queue_head_version,
                    expected_asset_lane_row_version: request.expected_asset_lane_row_version,
                },
            )
            .await
            .map_err(map_error)?,
        )
    }

    async fn load_resolution_closure(
        &self,
        operation_id: uuid::Uuid,
    ) -> InvestigationAssetQueueResult<Option<InvestigationResolutionClosurePublicationView>> {
        Ok(
            db::load_resolution_closure_publication(&self.pool, operation_id)
                .await
                .map_err(map_error)?
                .map(closure_view),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evolution_fuel_exhaustion_keeps_the_exact_asset_lane_identity() {
        let asset_lane_id = uuid::Uuid::new_v4();
        let error = golish_db::DbError::Other(anyhow::anyhow!(db::EVOLUTION_FUEL_EXHAUSTED));
        assert_eq!(
            map_asset_error(error, asset_lane_id),
            InvestigationAssetQueueRepositoryError::EvolutionFuelExhausted { asset_lane_id }
        );
    }
}
