//! PostgreSQL port adapter for the sole Tool Truth revalidation orchestrator.
//!
//! The adapter owns a frozen dispatch-head generation. Consumer/read paths do
//! not construct this type and therefore cannot trigger provider execution.

use std::sync::Arc;

use async_trait::async_trait;
use golish_agent_kit::task_orchestrator::tool_truth_revalidation::{
    ClaimedRevalidation, ToolTruthRevalidationStore,
};
use sqlx::PgPool;
use uuid::Uuid;

pub struct PgToolTruthRevalidationRecorder {
    pool: Arc<PgPool>,
}

impl PgToolTruthRevalidationRecorder {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }

    pub async fn record(
        &self,
        command: &golish_db::repo::tool_truth_revalidation::RecordRevalidationObligation,
    ) -> anyhow::Result<Uuid> {
        Ok(
            golish_db::repo::tool_truth_revalidation::record_obligation(
                self.pool.as_ref(),
                command,
            )
            .await?
            .id,
        )
    }
}

pub struct PgToolTruthRevalidationStore {
    pool: Arc<PgPool>,
    operation_id: Uuid,
    dispatch_generation: i64,
    dispatch_head_row_version: i64,
}

impl PgToolTruthRevalidationStore {
    pub fn new(
        pool: Arc<PgPool>,
        operation_id: Uuid,
        dispatch_generation: i64,
        dispatch_head_row_version: i64,
    ) -> Self {
        Self {
            pool,
            operation_id,
            dispatch_generation,
            dispatch_head_row_version,
        }
    }
}

#[async_trait]
impl ToolTruthRevalidationStore for PgToolTruthRevalidationStore {
    async fn claim_next(&self, owner: &str) -> anyhow::Result<Option<ClaimedRevalidation>> {
        let row = golish_db::repo::tool_truth_revalidation::claim_next(
            self.pool.as_ref(),
            &golish_db::repo::tool_truth_revalidation::ClaimRevalidationObligation {
                operation_id: self.operation_id,
                owner: owner.to_string(),
                expected_dispatch_generation: self.dispatch_generation,
                expected_head_row_version: self.dispatch_head_row_version,
            },
        )
        .await?;
        Ok(row.map(|row| ClaimedRevalidation {
            obligation_id: row.id,
            operation_id: row.operation_id,
            claim_token: row
                .claim_token
                .expect("claimed DB obligation always has a claim token"),
            row_version: row.row_version,
            source_receipt_id: row.source_receipt_id,
            source_input_key: row.source_input_key,
        }))
    }

    async fn complete_success(
        &self,
        owner: &str,
        claim: &ClaimedRevalidation,
        replacement_denominator_id: Uuid,
        replacement_receipt_id: Uuid,
    ) -> anyhow::Result<()> {
        golish_db::repo::tool_truth_revalidation::complete_success(
            self.pool.as_ref(),
            &golish_db::repo::tool_truth_revalidation::CompleteRevalidationObligation {
                obligation_id: claim.obligation_id,
                owner: owner.to_string(),
                claim_token: claim.claim_token,
                expected_row_version: claim.row_version,
                replacement_denominator_id,
                replacement_receipt_id,
            },
        )
        .await?;
        Ok(())
    }

    async fn record_failure(
        &self,
        owner: &str,
        claim: &ClaimedRevalidation,
        progress_fingerprint: &str,
        reason_code: &str,
    ) -> anyhow::Result<()> {
        golish_db::repo::tool_truth_revalidation::record_failure(
            self.pool.as_ref(),
            &golish_db::repo::tool_truth_revalidation::FailRevalidationObligation {
                obligation_id: claim.obligation_id,
                owner: owner.to_string(),
                claim_token: claim.claim_token,
                expected_row_version: claim.row_version,
                progress_fingerprint: progress_fingerprint.to_string(),
                reason_code: reason_code.to_string(),
            },
        )
        .await?;
        Ok(())
    }
}
