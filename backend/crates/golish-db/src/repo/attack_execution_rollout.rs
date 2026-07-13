//! Monotonic deployment default for the Candidate V2 execution contract.

use chrono::{DateTime, Utc};
use golish_core::AttackExecutionContract;
use sqlx::{PgConnection, Postgres, Transaction};

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct AttackExecutionRolloutRow {
    pub singleton: bool,
    pub contract: String,
    pub rank: i16,
    pub row_version: i64,
    pub updated_at: DateTime<Utc>,
}

pub async fn get_for_share(
    connection: &mut PgConnection,
) -> crate::Result<AttackExecutionRolloutRow> {
    sqlx::query_as(
        r#"SELECT singleton,contract,rank,row_version,updated_at
           FROM attack_execution_rollout WHERE singleton=TRUE FOR SHARE"#,
    )
    .fetch_optional(connection)
    .await?
    .ok_or_else(|| crate::DbError::NotFound("attack_execution_rollout".to_string()))
}

/// Advance exactly one persisted rank with an optimistic row-version CAS.
pub async fn advance_attack_execution_rollout(
    tx: &mut Transaction<'_, Postgres>,
    expected_version: i64,
    next: AttackExecutionContract,
) -> crate::Result<AttackExecutionRolloutRow> {
    if expected_version < 0 || next == AttackExecutionContract::Legacy {
        return Err(crate::DbError::Other(anyhow::anyhow!(
            "attack rollout must advance from an existing lower rank"
        )));
    }
    let row = sqlx::query_as::<_, AttackExecutionRolloutRow>(
        r#"UPDATE attack_execution_rollout
           SET contract=$2,rank=$3,row_version=row_version+1,updated_at=NOW()
           WHERE singleton=TRUE AND row_version=$1 AND rank+1=$3
           RETURNING singleton,contract,rank,row_version,updated_at"#,
    )
    .bind(expected_version)
    .bind(next.as_str())
    .bind(match next {
        AttackExecutionContract::Legacy => 0_i16,
        AttackExecutionContract::DualWriteReadLegacy => 1_i16,
        AttackExecutionContract::DualWriteReadV2Fallback => 2_i16,
        AttackExecutionContract::V2Only => 3_i16,
    })
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(row) = row {
        return Ok(row);
    }
    let current = get_for_share(tx).await?;
    Err(crate::DbError::Other(anyhow::anyhow!(
        "stale or non-adjacent attack rollout: expected_version={expected_version}, current_version={}, next={}",
        current.row_version,
        next.as_str()
    )))
}
