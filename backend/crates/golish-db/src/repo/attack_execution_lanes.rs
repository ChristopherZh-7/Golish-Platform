//! DB-owned global side-effect lane for Candidate V2 verification.

use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub const GLOBAL_EXPLOIT_LANE: &str = "global:exploit";

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct AttackExecutionLaneRow {
    pub lane_key: String,
    pub stage_worker_run_id: Option<Uuid>,
    pub lease_token: Option<Uuid>,
    pub lease_owner: Option<String>,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

const COLUMNS: &str =
    "lane_key,stage_worker_run_id,lease_token,lease_owner,lease_expires_at,updated_at";

pub async fn lock_global(
    tx: &mut Transaction<'_, Postgres>,
) -> crate::Result<AttackExecutionLaneRow> {
    let sql = format!("SELECT {COLUMNS} FROM attack_execution_lanes WHERE lane_key=$1 FOR UPDATE");
    sqlx::query_as(&sql)
        .bind(GLOBAL_EXPLOIT_LANE)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| crate::DbError::NotFound("attack_execution_lane".to_string()))
}

pub async fn claim_global(
    tx: &mut Transaction<'_, Postgres>,
    worker_run_id: Uuid,
    lease_token: Uuid,
    lease_owner: &str,
    lease_expires_at: DateTime<Utc>,
) -> crate::Result<AttackExecutionLaneRow> {
    let sql = format!(
        "UPDATE attack_execution_lanes
         SET stage_worker_run_id=$2,lease_token=$3,lease_owner=$4,
             lease_expires_at=$5,updated_at=NOW()
         WHERE lane_key=$1 AND stage_worker_run_id IS NULL
         RETURNING {COLUMNS}"
    );
    sqlx::query_as(&sql)
        .bind(GLOBAL_EXPLOIT_LANE)
        .bind(worker_run_id)
        .bind(lease_token)
        .bind(lease_owner)
        .bind(lease_expires_at)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| crate::DbError::Other(anyhow::anyhow!("attack execution lane is busy")))
}

pub async fn heartbeat_global(
    tx: &mut Transaction<'_, Postgres>,
    worker_run_id: Uuid,
    lease_token: Uuid,
    lease_owner: &str,
    lease_expires_at: DateTime<Utc>,
) -> crate::Result<AttackExecutionLaneRow> {
    let sql = format!(
        "UPDATE attack_execution_lanes SET lease_expires_at=$5,updated_at=NOW()
         WHERE lane_key=$1 AND stage_worker_run_id=$2 AND lease_token=$3
           AND lease_owner=$4 AND lease_expires_at>NOW()
         RETURNING {COLUMNS}"
    );
    sqlx::query_as(&sql)
        .bind(GLOBAL_EXPLOIT_LANE)
        .bind(worker_run_id)
        .bind(lease_token)
        .bind(lease_owner)
        .bind(lease_expires_at)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| crate::DbError::Other(anyhow::anyhow!("attack execution lane lease lost")))
}

pub async fn release_global(
    tx: &mut Transaction<'_, Postgres>,
    worker_run_id: Uuid,
    lease_token: Uuid,
    lease_owner: &str,
) -> crate::Result<AttackExecutionLaneRow> {
    let sql = format!(
        "UPDATE attack_execution_lanes
         SET stage_worker_run_id=NULL,lease_token=NULL,lease_owner=NULL,
             lease_expires_at=NULL,updated_at=NOW()
         WHERE lane_key=$1 AND stage_worker_run_id=$2 AND lease_token=$3 AND lease_owner=$4
         RETURNING {COLUMNS}"
    );
    sqlx::query_as(&sql)
        .bind(GLOBAL_EXPLOIT_LANE)
        .bind(worker_run_id)
        .bind(lease_token)
        .bind(lease_owner)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| crate::DbError::Other(anyhow::anyhow!("attack execution lane lease lost")))
}
