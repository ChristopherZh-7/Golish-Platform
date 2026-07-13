//! Bounded, final-PASS StageHandoff schema contract.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Executor, PgPool, Postgres};
use uuid::Uuid;

use super::runtime_memory_tx::{RuntimeMemoryStoreError, RuntimeMemoryStoreResult};

pub const TABLE_NAME: &str = "stage_handoffs";
pub const SOURCE_UNIT_UNIQUE_SQL: &str = "UNIQUE(source_stage_run_unit_id)";
pub const DELIVERABLE_UNIQUE_SQL: &str = "UNIQUE(deliverable_submission_id)";
pub const EXECUTION_ORG_UNIQUE_SQL: &str = "UNIQUE(stage_execution_id, organization_id)";
pub const SCOPE_OPERATION_FK_SQL: &str = "FOREIGN KEY(scope_snapshot_id, operation_id) \
     REFERENCES operation_org_scope_snapshots(id, operation_id)";
pub const SCOPE_MEMBERSHIP_FK_SQL: &str = "FOREIGN KEY(scope_snapshot_id, organization_id) \
     REFERENCES operation_org_scope_units(snapshot_id, organization_id)";
pub const SOURCE_UNIT_OWNER_FK_SQL: &str =
    "FOREIGN KEY(source_stage_run_unit_id, operation_id, stage_execution_id, organization_id) \
     REFERENCES stage_run_units(id, operation_id, stage_execution_id, organization_id)";
pub const DELIVERABLE_OWNER_FK_SQL: &str =
    "FOREIGN KEY(deliverable_submission_id, operation_id, stage_execution_id) \
     REFERENCES stage_deliverable_submissions(id, operation_id, stage_execution_id)";

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize, PartialEq, Eq)]
pub struct StageHandoffRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub from_stage_kind: String,
    pub stage_execution_id: Uuid,
    pub source_stage_run_unit_id: Uuid,
    pub deliverable_submission_id: Uuid,
    pub scope_hash: String,
    pub payload: Value,
    pub payload_sha256: String,
    pub evidence_ids: Vec<i64>,
    pub coverage_watermark: Value,
    pub unit_gate_decision_hash: String,
    pub aggregate_pass_token_hash: Option<String>,
    pub gate_passed_at: DateTime<Utc>,
    pub invalidated_at: Option<DateTime<Utc>>,
    pub schema_version: i32,
}

const COLUMNS: &str = r#"id, operation_id, organization_id, scope_snapshot_id,
    from_stage_kind, stage_execution_id, source_stage_run_unit_id,
    deliverable_submission_id, scope_hash, payload, payload_sha256,
    evidence_ids, coverage_watermark, unit_gate_decision_hash,
    aggregate_pass_token_hash, gate_passed_at, invalidated_at, schema_version"#;

#[derive(Debug, Clone)]
pub(crate) struct NewStageHandoffRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub from_stage_kind: String,
    pub stage_execution_id: Uuid,
    pub source_stage_run_unit_id: Uuid,
    pub deliverable_submission_id: Uuid,
    pub scope_hash: String,
    pub payload: Value,
    pub payload_sha256: String,
    pub evidence_ids: Vec<i64>,
    pub coverage_watermark: Value,
    pub unit_gate_decision_hash: String,
    pub aggregate_pass_token_hash: Option<String>,
    pub schema_version: i32,
}

pub(crate) async fn insert_with_executor<'e, E>(
    executor: E,
    input: &NewStageHandoffRow,
) -> RuntimeMemoryStoreResult<StageHandoffRow>
where
    E: Executor<'e, Database = Postgres>,
{
    if input.schema_version <= 0 {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "invalid_stage_handoff_schema_version",
        });
    }
    let sql = format!(
        r#"INSERT INTO stage_handoffs (
               id, operation_id, organization_id, scope_snapshot_id,
               from_stage_kind, stage_execution_id, source_stage_run_unit_id,
               deliverable_submission_id, scope_hash, payload, payload_sha256,
               evidence_ids, coverage_watermark, unit_gate_decision_hash,
               aggregate_pass_token_hash, gate_passed_at, schema_version
           ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,NOW(),$16)
           RETURNING {COLUMNS}"#
    );
    Ok(sqlx::query_as::<_, StageHandoffRow>(&sql)
        .bind(input.id)
        .bind(input.operation_id)
        .bind(input.organization_id)
        .bind(input.scope_snapshot_id)
        .bind(&input.from_stage_kind)
        .bind(input.stage_execution_id)
        .bind(input.source_stage_run_unit_id)
        .bind(input.deliverable_submission_id)
        .bind(&input.scope_hash)
        .bind(&input.payload)
        .bind(&input.payload_sha256)
        .bind(&input.evidence_ids)
        .bind(&input.coverage_watermark)
        .bind(&input.unit_gate_decision_hash)
        .bind(&input.aggregate_pass_token_hash)
        .bind(input.schema_version)
        .fetch_one(executor)
        .await?)
}

/// Read one newest immutable, non-invalidated final seal per inherited source
/// stage. The join back to passed Unit and Worker rows prevents a detached or
/// partially written projection from becoming downstream evidence.
pub async fn list_latest_final_sealed_for_sources(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
    source_stage_kinds: &[String],
) -> RuntimeMemoryStoreResult<Vec<StageHandoffRow>> {
    if source_stage_kinds.is_empty() {
        return Ok(Vec::new());
    }
    let sql = r#"SELECT DISTINCT ON (handoff.from_stage_kind) handoff.*
             FROM stage_handoffs AS handoff
             JOIN stage_run_units AS unit
               ON unit.id=handoff.source_stage_run_unit_id
              AND unit.operation_id=handoff.operation_id
              AND unit.stage_execution_id=handoff.stage_execution_id
              AND unit.organization_id=handoff.organization_id
              AND unit.status='passed'
             JOIN stage_deliverable_submissions AS submission
               ON submission.id=handoff.deliverable_submission_id
              AND submission.stage_run_unit_id=unit.id
             LEFT JOIN stage_worker_runs AS worker
               ON worker.id=submission.worker_run_id
              AND worker.operation_id=handoff.operation_id
              AND worker.stage_execution_id=handoff.stage_execution_id
              AND worker.stage_run_unit_id=unit.id
              AND worker.organization_id=handoff.organization_id
            WHERE handoff.operation_id=$1 AND handoff.organization_id=$2
              AND handoff.from_stage_kind=ANY($3)
              AND handoff.invalidated_at IS NULL
              AND (
                    (submission.worker_run_id IS NULL AND handoff.from_stage_kind='scoping')
                    OR worker.status='passed'
                  )
            ORDER BY handoff.from_stage_kind, handoff.gate_passed_at DESC, handoff.id DESC"#;
    Ok(sqlx::query_as::<_, StageHandoffRow>(sql)
        .bind(operation_id)
        .bind(organization_id)
        .bind(source_stage_kinds)
        .fetch_all(pool)
        .await?)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageHandoffStatus {
    Published,
    Invalidated,
}

impl StageHandoffRow {
    pub fn status(&self) -> StageHandoffStatus {
        if self.invalidated_at.is_some() {
            StageHandoffStatus::Invalidated
        } else {
            StageHandoffStatus::Published
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_memory_repo_contract_handoff_is_pass_sealed_and_org_scoped() {
        assert_eq!(TABLE_NAME, "stage_handoffs");
        assert!(SOURCE_UNIT_UNIQUE_SQL.contains("source_stage_run_unit_id"));
        assert!(DELIVERABLE_UNIQUE_SQL.contains("deliverable_submission_id"));
        assert!(SCOPE_MEMBERSHIP_FK_SQL.contains("scope_snapshot_id, organization_id"));
        assert!(SOURCE_UNIT_OWNER_FK_SQL.contains(
            "source_stage_run_unit_id, operation_id, stage_execution_id, organization_id"
        ));
        assert!(DELIVERABLE_OWNER_FK_SQL.contains("stage_deliverable_submissions"));
    }
}
