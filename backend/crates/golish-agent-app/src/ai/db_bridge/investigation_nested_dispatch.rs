//! App-owned adapter for the compound Investigation nested dispatch port.

use std::sync::Arc;

use async_trait::async_trait;
use golish_agent_kit::db_traits::*;
use golish_db::repo::investigation_nested_dispatch as db;
use golish_db::repo::runtime_memory_tx::{RuntimeMemoryStoreError, RuntimeMemoryTxFence};
use golish_db::repo::stage_teams::CompleteStageWorkerRow;
use sqlx::PgPool;

use super::convert::convert_agent_type_back;
use super::runtime_memory::{
    runtime_stage_unit_from_db, runtime_worker_fence_to_db, runtime_worker_from_db,
    stage_team_plan_from_db, stage_work_item_from_db, stage_worker_output_from_db,
};

#[derive(Clone)]
pub struct PgInvestigationNestedDispatchRepository {
    writer: db::PgInvestigationNestedDispatchRepository,
}

impl PgInvestigationNestedDispatchRepository {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self {
            writer: db::PgInvestigationNestedDispatchRepository::new(pool),
        }
    }
}

fn map_runtime_error(error: RuntimeMemoryStoreError) -> InvestigationNestedDispatchRepositoryError {
    match error {
        RuntimeMemoryStoreError::LeaseLost {
            worker_run_id,
            attempt_epoch,
        } => InvestigationNestedDispatchRepositoryError::LeaseLost {
            worker_run_id,
            attempt_epoch,
        },
        RuntimeMemoryStoreError::Missing { entity } => {
            InvestigationNestedDispatchRepositoryError::NotFound {
                detail: entity.to_string(),
            }
        }
        RuntimeMemoryStoreError::IdentityMismatch { code } => {
            InvestigationNestedDispatchRepositoryError::AuthorityMismatch {
                detail: code.to_string(),
            }
        }
        RuntimeMemoryStoreError::Conflict { code } => {
            InvestigationNestedDispatchRepositoryError::Conflict {
                detail: code.to_string(),
            }
        }
        RuntimeMemoryStoreError::StaleVersion {
            entity,
            expected,
            actual,
        } => InvestigationNestedDispatchRepositoryError::Conflict {
            detail: format!("{entity}: expected {expected}, actual {actual}"),
        },
        RuntimeMemoryStoreError::InvalidContractTransition { from, to } => {
            InvestigationNestedDispatchRepositoryError::AuthorityMismatch {
                detail: format!("runtime contract transition {from:?}->{to:?}"),
            }
        }
        RuntimeMemoryStoreError::Sqlx(error) => {
            InvestigationNestedDispatchRepositoryError::Infrastructure {
                detail: error.to_string(),
            }
        }
        RuntimeMemoryStoreError::Repository(error) => {
            InvestigationNestedDispatchRepositoryError::Infrastructure {
                detail: error.to_string(),
            }
        }
    }
}

fn map_error(
    error: db::InvestigationNestedDispatchStoreError,
) -> InvestigationNestedDispatchRepositoryError {
    match error {
        db::InvestigationNestedDispatchStoreError::Runtime(error) => map_runtime_error(error),
        db::InvestigationNestedDispatchStoreError::InvalidInput(detail) => {
            InvestigationNestedDispatchRepositoryError::InvalidRequest {
                detail: detail.to_string(),
            }
        }
        db::InvestigationNestedDispatchStoreError::AuthorityMismatch(detail) => {
            InvestigationNestedDispatchRepositoryError::AuthorityMismatch {
                detail: detail.to_string(),
            }
        }
        db::InvestigationNestedDispatchStoreError::ReplayConflict(detail) => {
            InvestigationNestedDispatchRepositoryError::Conflict {
                detail: detail.to_string(),
            }
        }
        db::InvestigationNestedDispatchStoreError::Sqlx(sqlx::Error::RowNotFound) => {
            InvestigationNestedDispatchRepositoryError::NotFound {
                detail: "nested Investigation lifecycle row".to_string(),
            }
        }
        db::InvestigationNestedDispatchStoreError::Sqlx(error) => {
            let detail = error.to_string();
            if detail.contains("CONFLICT")
                || detail.contains("REPLAY")
                || detail.contains("FENCE")
                || detail.contains("LEASE")
            {
                InvestigationNestedDispatchRepositoryError::Conflict { detail }
            } else if detail.contains("AUTHORITY")
                || detail.contains("IDENTITY")
                || detail.contains("SCOPE")
            {
                InvestigationNestedDispatchRepositoryError::AuthorityMismatch { detail }
            } else {
                InvestigationNestedDispatchRepositoryError::Infrastructure { detail }
            }
        }
        db::InvestigationNestedDispatchStoreError::Db(error) => {
            InvestigationNestedDispatchRepositoryError::Infrastructure {
                detail: error.to_string(),
            }
        }
    }
}

fn claimed_child(
    row: &db::BegunInvestigationNestedDispatchRow,
) -> InvestigationNestedDispatchResult<ClaimedStageWorkItemView> {
    let plan = stage_team_plan_from_db(row.plan.clone()).map_err(|error| {
        InvestigationNestedDispatchRepositoryError::Infrastructure {
            detail: error.to_string(),
        }
    })?;
    let work_item = stage_work_item_from_db(row.work_item.clone(), plan.aggregator_role.as_deref())
        .map_err(
            |error| InvestigationNestedDispatchRepositoryError::Infrastructure {
                detail: error.to_string(),
            },
        )?;
    Ok(ClaimedStageWorkItemView {
        unit: runtime_stage_unit_from_db(row.unit.clone()).map_err(|error| {
            InvestigationNestedDispatchRepositoryError::Infrastructure {
                detail: error.to_string(),
            }
        })?,
        plan,
        work_item,
        worker: runtime_worker_from_db(row.worker.clone()).map_err(|error| {
            InvestigationNestedDispatchRepositoryError::Infrastructure {
                detail: error.to_string(),
            }
        })?,
        message_chain_id: row.message_chain_id,
    })
}

fn completed_child(
    row: &db::FinishedInvestigationNestedDispatchRow,
) -> InvestigationNestedDispatchResult<CompletedStageWorkerView> {
    let plan = stage_team_plan_from_db(row.completion.plan.clone()).map_err(|error| {
        InvestigationNestedDispatchRepositoryError::Infrastructure {
            detail: error.to_string(),
        }
    })?;
    Ok(CompletedStageWorkerView {
        unit: runtime_stage_unit_from_db(row.completion.unit.clone()).map_err(|error| {
            InvestigationNestedDispatchRepositoryError::Infrastructure {
                detail: error.to_string(),
            }
        })?,
        work_item: stage_work_item_from_db(
            row.completion.work_item.clone(),
            plan.aggregator_role.as_deref(),
        )
        .map_err(
            |error| InvestigationNestedDispatchRepositoryError::Infrastructure {
                detail: error.to_string(),
            },
        )?,
        plan,
        worker: runtime_worker_from_db(row.completion.worker.clone()).map_err(|error| {
            InvestigationNestedDispatchRepositoryError::Infrastructure {
                detail: error.to_string(),
            }
        })?,
        output: stage_worker_output_from_db(row.completion.output.clone()).map_err(|error| {
            InvestigationNestedDispatchRepositoryError::Infrastructure {
                detail: error.to_string(),
            }
        })?,
        replayed: row.completion.replayed,
    })
}

fn fence_to_db(fence: &RuntimeWorkerFence) -> RuntimeMemoryTxFence {
    runtime_worker_fence_to_db(fence.clone())
}

#[async_trait]
impl InvestigationNestedDispatchRepository for PgInvestigationNestedDispatchRepository {
    async fn begin(
        &self,
        request: BeginInvestigationNestedDispatch,
    ) -> InvestigationNestedDispatchResult<BegunInvestigationNestedDispatch> {
        let identity = request.identity;
        let row = self
            .writer
            .begin(&db::BeginInvestigationNestedDispatchRow {
                authority_id: identity.stage.authority_id,
                operation_id: identity.stage.operation_id,
                stage_execution_id: identity.stage.stage_execution_id,
                owning_stage_run_request_id: identity.stage.owning_stage_run_request_id,
                stage_run_unit_id: identity.stage_run_unit_id,
                scope_snapshot_id: identity.stage.scope_snapshot_id,
                organization_id: identity.organization_id,
                stable_request_id: request.stable_request_id,
                task_plan_id: request.task_plan_id,
                subtask_id: request.subtask_id,
                parent_dispatch_receipt_id: request.parent_dispatch_receipt_id,
                parent_fence: fence_to_db(&request.parent_fence),
                stage_team_plan_id: request.stage_team_plan_id,
                parent_work_item_id: request.parent_work_item_id,
                expected_dispatch_epoch: request.expected_dispatch_epoch,
                nested_tool_request_id: request.nested_tool_request_id,
                requested_role: request.requested_role,
                objective: request.objective,
                args_sha256: request.args_sha256,
                snapshot_sha256: request.snapshot_sha256,
                dispatch_ordinal: i32::try_from(request.dispatch_ordinal).map_err(|_| {
                    InvestigationNestedDispatchRepositoryError::InvalidRequest {
                        detail: "dispatch ordinal overflow".to_string(),
                    }
                })?,
                session_id: request.session_id,
                agent: convert_agent_type_back(request.agent),
                model: request.model,
                provider: request.provider,
                lease_owner: request.lease_owner,
                lease_seconds: request.lease_seconds,
                initial_chain: request.initial_chain,
                initial_checkpoint: request.initial_checkpoint,
            })
            .await
            .map_err(map_error)?;
        Ok(BegunInvestigationNestedDispatch {
            begin_receipt_id: row.begin_receipt_id,
            stable_request_id: row.stable_request_id,
            task_plan_id: row.task_plan_id,
            subtask_id: row.subtask_id,
            parent_dispatch_receipt_id: row.parent_dispatch_receipt_id,
            stage_worker_request_id: row.stage_worker_request_id,
            args_sha256: row.args_sha256.clone(),
            request_sha256: row.request_sha256.clone(),
            begin_receipt_sha256: row.begin_receipt_sha256.clone(),
            child: claimed_child(&row)?,
            dispatch: super::unified_investigation::dispatch(row.dispatch),
            replayed: row.replayed,
        })
    }

    async fn finish(
        &self,
        request: FinishInvestigationNestedDispatch,
    ) -> InvestigationNestedDispatchResult<FinishedInvestigationNestedDispatch> {
        let identity = request.identity;
        let child_fence = fence_to_db(&request.child_fence);
        let output = CompleteStageWorkerRow {
            fence: child_fence.clone(),
            team_plan_id: request.stage_team_plan_id,
            work_item_id: request.work_item_id,
            expected_work_item_row_version: request.expected_work_item_row_version,
            output_schema: request.output.output_schema,
            business_disposition: request.output.disposition.as_str().to_string(),
            canonical_output: request.output.canonical_output,
            canonical_fact_refs: serde_json::Value::Array(request.output.fact_refs),
            evidence_ids: request.output.evidence_ids,
            checked_empty_cells: serde_json::Value::Array(request.output.checked_empty_units),
            blocker_codes: request.output.blocker_code.into_iter().collect(),
            output_hash: request.output.output_sha256,
            terminal_checkpoint: request.terminal_checkpoint,
            evidence_watermark: request.evidence_watermark,
        };
        let row = self
            .writer
            .finish(&db::FinishInvestigationNestedDispatchRow {
                authority_id: identity.stage.authority_id,
                operation_id: identity.stage.operation_id,
                stage_execution_id: identity.stage.stage_execution_id,
                owning_stage_run_request_id: identity.stage.owning_stage_run_request_id,
                stage_run_unit_id: identity.stage_run_unit_id,
                scope_snapshot_id: identity.stage.scope_snapshot_id,
                organization_id: identity.organization_id,
                stable_request_id: request.stable_request_id,
                begin_receipt_id: request.begin_receipt_id,
                task_plan_id: request.task_plan_id,
                subtask_id: request.subtask_id,
                parent_dispatch_receipt_id: request.parent_dispatch_receipt_id,
                dispatch_receipt_id: request.dispatch_receipt_id,
                child_fence,
                stage_team_plan_id: request.stage_team_plan_id,
                work_item_id: request.work_item_id,
                expected_work_item_row_version: request.expected_work_item_row_version,
                output,
                outcome: super::unified_investigation::dispatch_outcome(request.outcome),
                result_sha256: request.result_sha256,
                fence_sha256: request.fence_sha256,
            })
            .await
            .map_err(map_error)?;
        Ok(FinishedInvestigationNestedDispatch {
            finish_receipt_id: row.finish_receipt_id,
            stable_request_id: row.stable_request_id,
            begin_receipt_id: row.begin_receipt_id,
            task_plan_id: row.task_plan_id,
            subtask_id: row.subtask_id,
            parent_dispatch_receipt_id: row.parent_dispatch_receipt_id,
            dispatch_receipt_id: row.dispatch_receipt_id,
            result_sha256: row.result_sha256.clone(),
            finish_receipt_sha256: row.finish_receipt_sha256.clone(),
            completion: completed_child(&row)?,
            dispatch_attempt: super::unified_investigation::dispatch_attempt(row.dispatch_attempt),
            replayed: row.replayed,
        })
    }
}
