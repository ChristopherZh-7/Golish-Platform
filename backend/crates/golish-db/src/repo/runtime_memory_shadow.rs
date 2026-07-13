//! Immutable whole-record observations for runtime-memory dual-write rollout.
//!
//! PostgreSQL rehydrates both records and computes every comparison/hash. The
//! caller supplies only the admitted WorkerRun identity and a bounded mutation
//! label, so counts or match booleans can never become rollout authority.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]
pub struct RuntimeMemoryShadowSampleRow {
    pub sample_seq: i64,
    pub admission_seq: i64,
    pub worker_run_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub runtime_memory_contract: String,
    pub rollout_rank: i16,
    pub mutation_kind: String,
    pub legacy_record: Option<serde_json::Value>,
    pub v2_record: Option<serde_json::Value>,
    pub legacy_record_hash: Option<String>,
    pub v2_record_hash: Option<String>,
    pub comparison: String,
    pub selected_source: String,
    pub selected_record: Option<serde_json::Value>,
    pub selected_record_hash: Option<String>,
    pub observed_at: DateTime<Utc>,
}

/// Retain one post-mutation observation inside the caller's dual-write
/// transaction. The insert trigger loads the actual persisted legacy and V2
/// records and overwrites every derived column.
pub async fn persist_worker_sample(
    connection: &mut PgConnection,
    worker_run_id: Uuid,
    mutation_kind: &str,
) -> crate::Result<RuntimeMemoryShadowSampleRow> {
    if mutation_kind.is_empty() || mutation_kind.len() > 96 {
        return Err(crate::DbError::Other(anyhow::anyhow!(
            "runtime memory shadow mutation kind is invalid"
        )));
    }
    sqlx::query_as::<_, RuntimeMemoryShadowSampleRow>(
        r#"INSERT INTO runtime_memory_shadow_samples(
               sample_seq,worker_run_id,mutation_kind
           )
           VALUES(0,$1,$2)
           RETURNING sample_seq,admission_seq,worker_run_id,operation_id,
                     stage_execution_id,stage_run_unit_id,organization_id,
                     runtime_memory_contract,rollout_rank,mutation_kind,
                     legacy_record,v2_record,legacy_record_hash,v2_record_hash,
                     comparison,selected_source,selected_record,
                     selected_record_hash,observed_at"#,
    )
    .bind(worker_run_id)
    .bind(mutation_kind)
    .fetch_one(connection)
    .await
    .map_err(Into::into)
}
