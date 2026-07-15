//! Trusted server-side deliverable submission identity schema contract.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

pub const TABLE_NAME: &str = "stage_deliverable_submissions";
pub const TRUSTED_TOOL_CALL_UNIQUE_SQL: &str =
    "tool_call_record_id UUID NOT NULL UNIQUE REFERENCES tool_calls(id) ON DELETE RESTRICT";
pub const SUBMISSION_EXECUTION_UNIQUE_SQL: &str = "UNIQUE(id, operation_id, stage_execution_id)";
pub const STAGE_EXECUTION_OWNER_FK_SQL: &str =
    "FOREIGN KEY(stage_execution_id, operation_id) REFERENCES stage_runs(id, operation_id)";
pub const STAGE_UNIT_OWNER_FK_SQL: &str =
    "FOREIGN KEY(stage_run_unit_id, operation_id, stage_execution_id, organization_id) \
     REFERENCES stage_run_units(id, operation_id, stage_execution_id, organization_id)";
pub const SCOPING_UNIT_CHECK_SQL: &str =
    "CHECK ((stage_kind = 'scoping' AND worker_run_id IS NULL) \
     OR stage_run_unit_id IS NOT NULL)";

const MAX_CANONICAL_PAYLOAD_BYTES: usize = 256 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum StageDeliverableSubmissionError {
    #[error("stage deliverable submission identity mismatch: {code}")]
    IdentityMismatch { code: &'static str },
    #[error("stage deliverable submission conflict: {code}")]
    Conflict { code: &'static str },
    #[error("stage deliverable submission row missing: {entity}")]
    Missing { entity: &'static str },
    #[error("invalid stage deliverable payload: {code}")]
    InvalidPayload { code: &'static str },
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

impl StageDeliverableSubmissionError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::IdentityMismatch { code }
            | Self::Conflict { code }
            | Self::InvalidPayload { code } => code,
            Self::Missing { entity } => entity,
            Self::Sqlx(_) => "stage_deliverable_submission_storage",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewStageDeliverableSubmission {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Option<Uuid>,
    pub worker_run_id: Option<Uuid>,
    pub organization_id: Option<Uuid>,
    pub tool_call_record_id: Uuid,
    pub tool_request_id: String,
    pub stage_kind: String,
    pub attempt_epoch: Option<i64>,
    pub lease_token: Option<Uuid>,
    pub canonical_payload_json: String,
    pub payload_sha256: String,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize, PartialEq, Eq)]
pub struct StageDeliverableSubmissionRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Option<Uuid>,
    pub worker_run_id: Option<Uuid>,
    pub organization_id: Option<Uuid>,
    pub tool_call_record_id: Uuid,
    pub tool_request_id: String,
    pub stage_kind: String,
    pub attempt_epoch: Option<i64>,
    pub lease_token: Option<Uuid>,
    pub payload: Value,
    pub payload_sha256: String,
    pub submitted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageSubmissionBinding {
    ScopingPreFreeze,
    UnitBound,
}

impl StageDeliverableSubmissionRow {
    pub fn binding(&self) -> StageSubmissionBinding {
        if self.stage_kind == "scoping" && self.stage_run_unit_id.is_none() {
            StageSubmissionBinding::ScopingPreFreeze
        } else {
            StageSubmissionBinding::UnitBound
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
struct TrustedToolCallRow {
    call_id: String,
    name: String,
    status: String,
    operation_id: Option<Uuid>,
    stage_execution_id: Option<Uuid>,
    stage_run_unit_id: Option<Uuid>,
    worker_run_id: Option<Uuid>,
    organization_id: Option<Uuid>,
    attempt_epoch: Option<i64>,
    lease_token: Option<Uuid>,
    stage_kind: String,
}

fn identity_mismatch(code: &'static str) -> StageDeliverableSubmissionError {
    StageDeliverableSubmissionError::IdentityMismatch { code }
}

fn verify_payload(
    input: &NewStageDeliverableSubmission,
) -> Result<Value, StageDeliverableSubmissionError> {
    if input.canonical_payload_json.len() > MAX_CANONICAL_PAYLOAD_BYTES {
        return Err(StageDeliverableSubmissionError::InvalidPayload {
            code: "submission_payload_too_large",
        });
    }
    let payload = serde_json::from_str::<Value>(&input.canonical_payload_json).map_err(|_| {
        StageDeliverableSubmissionError::InvalidPayload {
            code: "submission_payload_invalid_json",
        }
    })?;
    let actual_sha256 = Sha256::digest(input.canonical_payload_json.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if actual_sha256 != input.payload_sha256 {
        return Err(StageDeliverableSubmissionError::InvalidPayload {
            code: "submission_payload_hash_mismatch",
        });
    }
    if payload.get("stage_id").and_then(Value::as_str) != Some(input.stage_kind.as_str()) {
        return Err(StageDeliverableSubmissionError::InvalidPayload {
            code: "submission_payload_stage_kind_mismatch",
        });
    }
    if payload.get("stage_run_id").and_then(Value::as_str)
        != Some(input.stage_execution_id.to_string().as_str())
    {
        return Err(StageDeliverableSubmissionError::InvalidPayload {
            code: "submission_payload_stage_execution_mismatch",
        });
    }
    Ok(payload)
}

async fn lock_and_verify_tool_call(
    tx: &mut Transaction<'_, Postgres>,
    input: &NewStageDeliverableSubmission,
) -> Result<(), StageDeliverableSubmissionError> {
    let row = sqlx::query_as::<_, TrustedToolCallRow>(
        r#"SELECT tc.call_id,
                  tc.name,
                  tc.status::text AS status,
                  tc.operation_id,
                  tc.stage_execution_id,
                  tc.stage_run_unit_id,
                  tc.worker_run_id,
                  tc.organization_id,
                  tc.attempt_epoch,
                  tc.lease_token,
                  sr.stage_kind
             FROM tool_calls AS tc
             JOIN stage_runs AS sr
               ON sr.id = tc.stage_execution_id
              AND sr.operation_id = tc.operation_id
            WHERE tc.id = $1
            FOR SHARE OF tc, sr"#,
    )
    .bind(input.tool_call_record_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(StageDeliverableSubmissionError::Missing {
        entity: "submission_tool_call",
    })?;

    if row.operation_id != Some(input.operation_id) {
        return Err(identity_mismatch("submission_tool_operation_mismatch"));
    }
    if row.stage_execution_id != Some(input.stage_execution_id) {
        return Err(identity_mismatch(
            "submission_tool_stage_execution_mismatch",
        ));
    }
    if row.stage_run_unit_id != input.stage_run_unit_id {
        return Err(identity_mismatch("submission_tool_stage_unit_mismatch"));
    }
    if row.worker_run_id != input.worker_run_id {
        return Err(identity_mismatch("submission_tool_worker_mismatch"));
    }
    if row.organization_id != input.organization_id {
        return Err(identity_mismatch("submission_tool_organization_mismatch"));
    }
    if row.attempt_epoch != input.attempt_epoch {
        return Err(identity_mismatch("submission_tool_attempt_epoch_mismatch"));
    }
    if row.lease_token != input.lease_token {
        return Err(identity_mismatch("submission_tool_lease_token_mismatch"));
    }
    if row.call_id != input.tool_request_id {
        return Err(identity_mismatch("submission_tool_request_mismatch"));
    }
    if row.name != "submit_stage_deliverable" {
        return Err(identity_mismatch("submission_tool_name_mismatch"));
    }
    if !matches!(row.status.as_str(), "received" | "running") {
        return Err(StageDeliverableSubmissionError::Conflict {
            code: "submission_tool_not_active",
        });
    }
    if row.stage_kind != input.stage_kind {
        return Err(identity_mismatch("submission_tool_stage_kind_mismatch"));
    }

    // A Team-owned Unit has one server-selected final submitter.  Producer and
    // helper prompts intentionally omit this authority, but prompts are not a
    // security boundary: the immutable submission writer rechecks the exact
    // TeamPlan -> Aggregator WorkItem -> WorkerRun tuple under row locks.
    if let Some(stage_run_unit_id) = input.stage_run_unit_id {
        // The retained foundation-migration compatibility tests exercise this
        // repository before the later Stage Team schema exists. Such a schema
        // cannot contain a Team-owned Unit, so only run the stronger Team
        // authority check once that atomic migration is present.
        let stage_team_schema_available: bool =
            sqlx::query_scalar("SELECT TO_REGCLASS('public.stage_team_plans') IS NOT NULL")
                .fetch_one(&mut **tx)
                .await?;
        let team_plan = if stage_team_schema_available {
            sqlx::query_as::<_, (Uuid, Option<Uuid>, Option<String>)>(
                r#"SELECT id,final_submitter_worker_run_id,aggregator_role
                     FROM stage_team_plans
                    WHERE stage_run_unit_id=$1
                    FOR SHARE"#,
            )
            .bind(stage_run_unit_id)
            .fetch_optional(&mut **tx)
            .await?
        } else {
            None
        };
        if let Some((team_plan_id, final_submitter_worker_run_id, aggregator_role)) = team_plan {
            let exact_aggregator = match (
                input.worker_run_id,
                final_submitter_worker_run_id,
                aggregator_role.as_deref(),
            ) {
                (Some(worker_run_id), Some(final_submitter), Some(aggregator_role))
                    if worker_run_id == final_submitter =>
                {
                    sqlx::query_scalar::<_, bool>(
                        r#"SELECT EXISTS (
                               SELECT 1
                                 FROM stage_worker_runs AS worker
                                 JOIN stage_work_items AS item
                                   ON item.id=worker.work_item_id
                                  AND item.operation_id=worker.operation_id
                                  AND item.stage_execution_id=worker.stage_execution_id
                                  AND item.stage_run_unit_id=worker.stage_run_unit_id
                                  AND item.organization_id=worker.organization_id
                                WHERE worker.id=$1
                                  AND worker.operation_id=$2
                                  AND worker.stage_execution_id=$3
                                  AND worker.stage_run_unit_id=$4
                                  AND worker.organization_id=$5
                                  AND worker.status='running'
                                  AND worker.active_tool_call_id=$6
                                  AND item.team_plan_id=$7
                                  AND item.role=$8
                                  AND item.required_for_barrier=FALSE
                                  AND item.status='running'
                           )"#,
                    )
                    .bind(worker_run_id)
                    .bind(input.operation_id)
                    .bind(input.stage_execution_id)
                    .bind(stage_run_unit_id)
                    .bind(input.organization_id)
                    .bind(input.tool_call_record_id)
                    .bind(team_plan_id)
                    .bind(aggregator_role)
                    .fetch_one(&mut **tx)
                    .await?
                }
                _ => false,
            };
            if !exact_aggregator {
                return Err(StageDeliverableSubmissionError::Conflict {
                    code: "stage_team_submission_requires_unique_aggregator",
                });
            }
        }
    }
    Ok(())
}

/// Persist one immutable deliverable submission after locking and comparing the
/// complete runtime identity on its awaited `tool_calls` row. The database FKs
/// and worker-fence trigger remain the final authority; these typed checks make
/// hostile rebinding fail with a stable domain code instead of an opaque FK.
pub async fn insert(
    pool: &PgPool,
    input: &NewStageDeliverableSubmission,
) -> Result<StageDeliverableSubmissionRow, StageDeliverableSubmissionError> {
    let payload = verify_payload(input)?;
    let mut tx = pool.begin().await?;
    lock_and_verify_tool_call(&mut tx, input).await?;
    let id = Uuid::new_v4();
    let inserted = sqlx::query_as::<_, StageDeliverableSubmissionRow>(
        r#"INSERT INTO stage_deliverable_submissions
           (id, operation_id, stage_execution_id, stage_run_unit_id,
            worker_run_id, organization_id, tool_call_record_id,
            tool_request_id, stage_kind, attempt_epoch, lease_token,
            payload, payload_sha256)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
           RETURNING *"#,
    )
    .bind(id)
    .bind(input.operation_id)
    .bind(input.stage_execution_id)
    .bind(input.stage_run_unit_id)
    .bind(input.worker_run_id)
    .bind(input.organization_id)
    .bind(input.tool_call_record_id)
    .bind(&input.tool_request_id)
    .bind(&input.stage_kind)
    .bind(input.attempt_epoch)
    .bind(input.lease_token)
    .bind(payload)
    .bind(&input.payload_sha256)
    .fetch_one(&mut *tx)
    .await
    .map_err(|error| {
        if error
            .as_database_error()
            .and_then(|db_error| db_error.code())
            .is_some_and(|code| code == "23505")
        {
            StageDeliverableSubmissionError::Conflict {
                code: "submission_tool_call_already_used",
            }
        } else {
            StageDeliverableSubmissionError::Sqlx(error)
        }
    })?;
    tx.commit().await?;
    Ok(inserted)
}

/// Load by the complete trusted owner tuple. A valid id from another operation
/// or execution is deliberately indistinguishable from an absent row (I2).
pub async fn load_scoped(
    pool: &PgPool,
    id: Uuid,
    operation_id: Uuid,
    stage_execution_id: Uuid,
) -> Result<Option<StageDeliverableSubmissionRow>, StageDeliverableSubmissionError> {
    Ok(sqlx::query_as::<_, StageDeliverableSubmissionRow>(
        r#"SELECT *
             FROM stage_deliverable_submissions
            WHERE id = $1
              AND operation_id = $2
              AND stage_execution_id = $3"#,
    )
    .bind(id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .fetch_optional(pool)
    .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_memory_repo_contract_submission_uses_trusted_tool_call_identity() {
        assert_eq!(TABLE_NAME, "stage_deliverable_submissions");
        assert!(TRUSTED_TOOL_CALL_UNIQUE_SQL.contains("tool_call_record_id"));
        assert!(SUBMISSION_EXECUTION_UNIQUE_SQL.contains("id, operation_id, stage_execution_id"));
        assert!(SCOPING_UNIT_CHECK_SQL.contains("stage_kind = 'scoping'"));
        assert!(SCOPING_UNIT_CHECK_SQL.contains("stage_run_unit_id IS NOT NULL"));
        assert!(STAGE_UNIT_OWNER_FK_SQL.contains("organization_id"));
    }
}
