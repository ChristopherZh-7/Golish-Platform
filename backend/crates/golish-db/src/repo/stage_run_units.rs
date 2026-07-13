//! Per-execution, per-organization final Gate unit schema contract.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Executor, PgPool, Postgres};
use uuid::Uuid;

use super::runtime_memory_tx::{RuntimeMemoryStoreError, RuntimeMemoryStoreResult};

pub const TABLE_NAME: &str = "stage_run_units";
pub const STATUS_CHECK_SQL: &str =
    "CHECK (status IN ('queued','running','gate_blocked','passed','exhausted','superseded'))";
pub const EXECUTION_ORG_UNIQUE_SQL: &str = "UNIQUE(stage_execution_id, organization_id)";
pub const UNIT_OWNER_UNIQUE_SQL: &str =
    "UNIQUE(id, operation_id, stage_execution_id, organization_id)";
pub const STAGE_EXECUTION_OWNER_FK_SQL: &str =
    "FOREIGN KEY(stage_execution_id, operation_id) REFERENCES stage_runs(id, operation_id)";
pub const SCOPE_OPERATION_FK_SQL: &str = "FOREIGN KEY(scope_snapshot_id, operation_id) \
     REFERENCES operation_org_scope_snapshots(id, operation_id)";
pub const SCOPE_MEMBERSHIP_FK_SQL: &str = "FOREIGN KEY(scope_snapshot_id, organization_id) \
     REFERENCES operation_org_scope_units(snapshot_id, organization_id)";

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize, PartialEq, Eq)]
pub struct StageRunUnitRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub stage_kind: String,
    pub generation: i32,
    pub specialist: Option<String>,
    pub status: String,
    pub gate_attempt: i32,
    pub pass_watermark: Value,
    pub row_version: i64,
    pub started_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub terminal_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StageRunUnitStatus {
    Queued,
    Running,
    GateBlocked,
    Passed,
    Exhausted,
    Superseded,
}

impl StageRunUnitStatus {
    pub const ALL: [Self; 6] = [
        Self::Queued,
        Self::Running,
        Self::GateBlocked,
        Self::Passed,
        Self::Exhausted,
        Self::Superseded,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::GateBlocked => "gate_blocked",
            Self::Passed => "passed",
            Self::Exhausted => "exhausted",
            Self::Superseded => "superseded",
        }
    }

    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Passed | Self::Exhausted | Self::Superseded)
    }

    pub const fn may_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Queued, Self::Running)
                | (
                    Self::Running,
                    Self::GateBlocked | Self::Passed | Self::Exhausted
                )
                | (Self::GateBlocked, Self::Running)
                | (
                    Self::Queued | Self::Running | Self::GateBlocked,
                    Self::Superseded
                )
        )
    }
}

const COLUMNS: &str = r#"id, operation_id, stage_execution_id, scope_snapshot_id,
    organization_id, stage_kind, generation, specialist, status, gate_attempt,
    pass_watermark, row_version, started_at, updated_at, terminal_at"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewStageRunUnit {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub stage_kind: String,
    pub generation: i32,
    pub specialist: Option<String>,
}

pub async fn insert_with_executor<'e, E>(
    executor: E,
    input: &NewStageRunUnit,
) -> RuntimeMemoryStoreResult<StageRunUnitRow>
where
    E: Executor<'e, Database = Postgres>,
{
    if input.stage_kind.trim().is_empty() || input.generation < 0 {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "invalid_stage_run_unit_identity",
        });
    }
    let sql = format!(
        "INSERT INTO stage_run_units (
            id, operation_id, stage_execution_id, scope_snapshot_id,
            organization_id, stage_kind, generation, specialist, status
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'queued')
         RETURNING {COLUMNS}"
    );
    Ok(sqlx::query_as::<_, StageRunUnitRow>(&sql)
        .bind(input.id)
        .bind(input.operation_id)
        .bind(input.stage_execution_id)
        .bind(input.scope_snapshot_id)
        .bind(input.organization_id)
        .bind(&input.stage_kind)
        .bind(input.generation)
        .bind(&input.specialist)
        .fetch_one(executor)
        .await?)
}

pub async fn get_with_executor<'e, E>(
    executor: E,
    id: Uuid,
) -> RuntimeMemoryStoreResult<Option<StageRunUnitRow>>
where
    E: Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {COLUMNS} FROM stage_run_units WHERE id = $1");
    Ok(sqlx::query_as::<_, StageRunUnitRow>(&sql)
        .bind(id)
        .fetch_optional(executor)
        .await?)
}

pub async fn get(pool: &PgPool, id: Uuid) -> RuntimeMemoryStoreResult<Option<StageRunUnitRow>> {
    get_with_executor(pool, id).await
}

pub async fn list_for_execution(
    pool: &PgPool,
    operation_id: Uuid,
    stage_execution_id: Uuid,
) -> RuntimeMemoryStoreResult<Vec<StageRunUnitRow>> {
    list_for_execution_with_executor(pool, operation_id, stage_execution_id).await
}

pub async fn list_for_execution_with_executor<'e, E>(
    executor: E,
    operation_id: Uuid,
    stage_execution_id: Uuid,
) -> RuntimeMemoryStoreResult<Vec<StageRunUnitRow>>
where
    E: Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {COLUMNS} FROM stage_run_units
         WHERE operation_id = $1 AND stage_execution_id = $2
         ORDER BY organization_id, id"
    );
    Ok(sqlx::query_as::<_, StageRunUnitRow>(&sql)
        .bind(operation_id)
        .bind(stage_execution_id)
        .fetch_all(executor)
        .await?)
}

#[allow(clippy::too_many_arguments)]
pub async fn transition_cas<'e, E>(
    executor: E,
    id: Uuid,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    organization_id: Uuid,
    expected: StageRunUnitStatus,
    expected_row_version: i64,
    next: StageRunUnitStatus,
    pass_watermark: Option<&Value>,
) -> RuntimeMemoryStoreResult<StageRunUnitRow>
where
    E: Executor<'e, Database = Postgres>,
{
    if next == StageRunUnitStatus::Passed {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "unit_pass_requires_final_seal",
        });
    }
    transition_cas_inner(
        executor,
        id,
        operation_id,
        stage_execution_id,
        organization_id,
        expected,
        expected_row_version,
        next,
        pass_watermark,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn transition_to_passed_for_final_seal<'e, E>(
    executor: E,
    id: Uuid,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    organization_id: Uuid,
    expected: StageRunUnitStatus,
    expected_row_version: i64,
    pass_watermark: &Value,
) -> RuntimeMemoryStoreResult<StageRunUnitRow>
where
    E: Executor<'e, Database = Postgres>,
{
    transition_cas_inner(
        executor,
        id,
        operation_id,
        stage_execution_id,
        organization_id,
        expected,
        expected_row_version,
        StageRunUnitStatus::Passed,
        Some(pass_watermark),
    )
    .await
}

/// Persist a Gate-PASS continuation checkpoint without changing the Unit's
/// Running status. Used only by the compound wave-close transaction after it
/// has atomically completed the current wave and parked the Worker.
pub(crate) async fn checkpoint_running_pass_watermark<'e, E>(
    executor: E,
    id: Uuid,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    organization_id: Uuid,
    expected_row_version: i64,
    pass_watermark: &Value,
) -> RuntimeMemoryStoreResult<StageRunUnitRow>
where
    E: Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "UPDATE stage_run_units
            SET pass_watermark=$6, row_version=row_version+1, updated_at=NOW()
          WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
            AND organization_id=$4 AND status='running' AND row_version=$5
          RETURNING {COLUMNS}"
    );
    sqlx::query_as::<_, StageRunUnitRow>(&sql)
        .bind(id)
        .bind(operation_id)
        .bind(stage_execution_id)
        .bind(organization_id)
        .bind(expected_row_version)
        .bind(pass_watermark)
        .fetch_optional(executor)
        .await?
        .ok_or(RuntimeMemoryStoreError::StaleVersion {
            entity: TABLE_NAME,
            expected: expected_row_version,
            actual: -1,
        })
}

#[allow(clippy::too_many_arguments)]
async fn transition_cas_inner<'e, E>(
    executor: E,
    id: Uuid,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    organization_id: Uuid,
    expected: StageRunUnitStatus,
    expected_row_version: i64,
    next: StageRunUnitStatus,
    pass_watermark: Option<&Value>,
) -> RuntimeMemoryStoreResult<StageRunUnitRow>
where
    E: Executor<'e, Database = Postgres>,
{
    if !expected.may_transition_to(next) {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "invalid_stage_run_unit_transition",
        });
    }
    let sql = format!(
        "UPDATE stage_run_units
         SET status = $7,
             gate_attempt = gate_attempt + CASE WHEN $7 = 'running' AND $5 = 'gate_blocked' THEN 1 ELSE 0 END,
             pass_watermark = COALESCE($8, pass_watermark),
             row_version = row_version + 1,
             started_at = CASE WHEN $7 = 'running' THEN COALESCE(started_at, NOW()) ELSE started_at END,
             updated_at = NOW(),
             terminal_at = CASE WHEN $7 IN ('passed','exhausted','superseded') THEN NOW() ELSE NULL END
         WHERE id = $1 AND operation_id = $2 AND stage_execution_id = $3
           AND organization_id = $4 AND status = $5 AND row_version = $6
         RETURNING {COLUMNS}"
    );
    sqlx::query_as::<_, StageRunUnitRow>(&sql)
        .bind(id)
        .bind(operation_id)
        .bind(stage_execution_id)
        .bind(organization_id)
        .bind(expected.as_str())
        .bind(expected_row_version)
        .bind(next.as_str())
        .bind(pass_watermark)
        .fetch_optional(executor)
        .await?
        .ok_or(RuntimeMemoryStoreError::StaleVersion {
            entity: TABLE_NAME,
            expected: expected_row_version,
            actual: -1,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_memory_repo_contract_stage_unit_is_execution_and_org_scoped() {
        assert_eq!(TABLE_NAME, "stage_run_units");
        assert!(
            UNIT_OWNER_UNIQUE_SQL.contains("id, operation_id, stage_execution_id, organization_id")
        );
        assert!(SCOPE_MEMBERSHIP_FK_SQL.contains("scope_snapshot_id, organization_id"));
        assert!(StageRunUnitStatus::Passed.is_terminal());
        assert!(!StageRunUnitStatus::GateBlocked.is_terminal());
        assert!(StageRunUnitStatus::ALL
            .iter()
            .all(|status| STATUS_CHECK_SQL.contains(status.as_str())));
    }

    #[test]
    fn unit_transition_graph_is_closed_and_monotonic() {
        assert!(StageRunUnitStatus::Queued.may_transition_to(StageRunUnitStatus::Running));
        assert!(StageRunUnitStatus::Running.may_transition_to(StageRunUnitStatus::GateBlocked));
        assert!(StageRunUnitStatus::GateBlocked.may_transition_to(StageRunUnitStatus::Running));
        assert!(StageRunUnitStatus::Running.may_transition_to(StageRunUnitStatus::Passed));
        assert!(StageRunUnitStatus::Queued.may_transition_to(StageRunUnitStatus::Superseded));
        assert!(!StageRunUnitStatus::Queued.may_transition_to(StageRunUnitStatus::Passed));
        assert!(!StageRunUnitStatus::Passed.may_transition_to(StageRunUnitStatus::Running));
        assert!(!StageRunUnitStatus::Exhausted.may_transition_to(StageRunUnitStatus::Superseded));
    }
}
