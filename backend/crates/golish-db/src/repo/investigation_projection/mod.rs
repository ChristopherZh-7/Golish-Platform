//! Deterministic whole-batch materializer for the Plan B investigation read model.
//!
//! Canonical transactions append immutable typed source snapshots and advance
//! only the source head. The projector consumes batches strictly by source
//! sequence and publishes entity/change rows plus the projection head in one
//! transaction.

mod projector;
mod types;

pub use projector::{
    capture_projection_head, claim_next_projection_batch, project_next_projection_batch,
    project_projection_batch, read_projection_at_head,
};
pub use types::{
    CapturedProjectionHead, InvestigationProjectionChange, InvestigationProjectionError,
    InvestigationProjectionResult, MaterializedProjectionEntity, ProjectionBatchClaim,
    ProjectionBatchEnqueueReceipt, ProjectionBatchReceipt, ProjectionProjectOutcome,
    ProjectionReadPage,
};

pub use crate::repo::hypothesis_legacy_projection::{
    AppendProjectionSourceBatchRow as ProjectionOutboxBatchInput,
    ProjectionOutboxSourceRow as ProjectionOutboxMemberInput, ProjectionSourceStorageV1,
};

use sqlx::{Postgres, Transaction};

/// Public typed enqueue seam for canonical sources outside the Hypothesis
/// finalizer. It delegates to the single source-head writer; no materialized
/// entity, legacy compatibility row, or projection head is touched here.
pub async fn enqueue_projection_batch_on(
    tx: &mut Transaction<'_, Postgres>,
    input: ProjectionOutboxBatchInput,
) -> crate::Result<ProjectionBatchEnqueueReceipt> {
    let replayed = sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS(
               SELECT 1 FROM investigation_projection_outbox_batches
                WHERE operation_id=$1 AND stable_request_id=$2
           )"#,
    )
    .bind(input.operation_id)
    .bind(input.stable_request_id)
    .fetch_one(&mut **tx)
    .await?;
    let view =
        crate::repo::hypothesis_legacy_projection::append_projection_source_batch_on(tx, input)
            .await?;
    Ok(ProjectionBatchEnqueueReceipt {
        batch_id: view.batch_id,
        operation_id: view.operation_id,
        source_batch_seq: view.source_batch_seq,
        predecessor_batch_id: view.predecessor_batch_id,
        member_count: view.member_count,
        member_set_hash: view.member_set_hash,
        replayed,
    })
}
