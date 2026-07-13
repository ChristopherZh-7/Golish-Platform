//! Schema contract for the monotonic runtime-memory rollout singleton.
//!
//! Owns the stable row, contract vocabulary, shared-lock read, and adjacent
//! row-version CAS used to roll the platform forward without per-process drift.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Executor, PgConnection, PgPool, Postgres};

use super::runtime_memory_tx::{RuntimeMemoryStoreError, RuntimeMemoryStoreResult};

pub const TABLE_NAME: &str = "runtime_memory_rollout";
pub const SINGLETON_ID: i16 = 1;
pub const CONTRACT_VALUES: &[&str] = &[
    "legacy_v1",
    "dual_write_legacy_read",
    "dual_write_v2_preferred",
    "v2_only",
];
pub const CONTRACT_RANK_CHECK_SQL: &str = "CHECK (contract_rank = CASE contract \
     WHEN 'legacy_v1' THEN 0 \
     WHEN 'dual_write_legacy_read' THEN 1 \
     WHEN 'dual_write_v2_preferred' THEN 2 \
     WHEN 'v2_only' THEN 3 END)";
pub const SINGLETON_CHECK_SQL: &str = "CHECK (singleton_id = 1)";

const GET_SQL: &str = r#"SELECT singleton_id, contract, contract_rank, row_version, updated_at
    FROM runtime_memory_rollout
    WHERE singleton_id = 1"#;
const GET_FOR_SHARE_SQL: &str = r#"SELECT singleton_id, contract, contract_rank, row_version, updated_at
    FROM runtime_memory_rollout
    WHERE singleton_id = 1
    FOR SHARE"#;
const GET_FOR_UPDATE_SQL: &str = r#"SELECT singleton_id, contract, contract_rank, row_version, updated_at
    FROM runtime_memory_rollout
    WHERE singleton_id = 1
    FOR UPDATE"#;
const ADVANCE_SQL: &str = r#"UPDATE runtime_memory_rollout
SET contract = $2,
    contract_rank = $3,
    row_version = row_version + 1,
    updated_at = NOW()
WHERE singleton_id = 1
  AND contract = $1
  AND contract_rank + 1 = $3
  AND row_version = $4
RETURNING contract, contract_rank, row_version, updated_at"#;

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeMemoryRolloutRow {
    pub singleton_id: i16,
    pub contract: String,
    pub contract_rank: i16,
    pub row_version: i64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeMemoryRolloutReconcileOutcome {
    AlreadyCurrent(RuntimeMemoryRolloutRow),
    Promoted(RuntimeMemoryRolloutRow),
    NotReady {
        contract: String,
        contract_rank: i16,
        row_version: i64,
        reason: String,
    },
}

#[derive(Debug, sqlx::FromRow)]
struct RuntimeMemoryCohortGateRow {
    admission_count: i64,
    sample_count: i64,
    ready: bool,
    reason: String,
    aggregate_digest: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct RuntimeMemoryRolloutAdvanceRow {
    contract: String,
    contract_rank: i16,
    row_version: i64,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeMemoryContract {
    LegacyV1,
    DualWriteLegacyRead,
    DualWriteV2Preferred,
    V2Only,
}

impl RuntimeMemoryContract {
    pub const ALL: [Self; 4] = [
        Self::LegacyV1,
        Self::DualWriteLegacyRead,
        Self::DualWriteV2Preferred,
        Self::V2Only,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LegacyV1 => "legacy_v1",
            Self::DualWriteLegacyRead => "dual_write_legacy_read",
            Self::DualWriteV2Preferred => "dual_write_v2_preferred",
            Self::V2Only => "v2_only",
        }
    }

    pub const fn rank(self) -> i16 {
        match self {
            Self::LegacyV1 => 0,
            Self::DualWriteLegacyRead => 1,
            Self::DualWriteV2Preferred => 2,
            Self::V2Only => 3,
        }
    }

    pub const fn may_advance_to(self, next: Self) -> bool {
        next.rank() == self.rank() + 1
    }
}

pub async fn get(pool: &PgPool) -> RuntimeMemoryStoreResult<RuntimeMemoryRolloutRow> {
    let row = sqlx::query_as::<_, RuntimeMemoryRolloutRow>(GET_SQL)
        .fetch_optional(pool)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing { entity: TABLE_NAME })?;
    Ok(row)
}

/// Read and share-lock the singleton inside the caller's transaction. Runtime
/// operation creation holds this lock until commit so the frozen operation
/// contract always corresponds to one persisted rollout row version.
pub async fn get_for_share<'e, E>(executor: E) -> RuntimeMemoryStoreResult<RuntimeMemoryRolloutRow>
where
    E: Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_as::<_, RuntimeMemoryRolloutRow>(GET_FOR_SHARE_SQL)
        .fetch_optional(executor)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing { entity: TABLE_NAME })?;
    Ok(row)
}

async fn get_for_update(
    connection: &mut PgConnection,
) -> RuntimeMemoryStoreResult<RuntimeMemoryRolloutRow> {
    sqlx::query_as::<_, RuntimeMemoryRolloutRow>(GET_FOR_UPDATE_SQL)
        .fetch_optional(connection)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing { entity: TABLE_NAME })
}

/// Serialize both deployment singletons before taking either row lock. The DB
/// statement triggers use the same advisory key, covering raw SQL callers too.
pub async fn lock_execution_rollout_pair(
    connection: &mut PgConnection,
) -> RuntimeMemoryStoreResult<()> {
    sqlx::query("SELECT lock_execution_rollout_pair()")
        .execute(connection)
        .await?;
    Ok(())
}

async fn cohort_gate(
    connection: &mut PgConnection,
    current: &RuntimeMemoryRolloutRow,
) -> RuntimeMemoryStoreResult<RuntimeMemoryCohortGateRow> {
    let cutoff: Option<i64> = sqlx::query_scalar(
        r#"SELECT MAX(admission_seq)
             FROM runtime_memory_rollout_admissions
            WHERE runtime_memory_contract=$1 AND rollout_rank=$2"#,
    )
    .bind(&current.contract)
    .bind(current.contract_rank)
    .fetch_one(&mut *connection)
    .await?;
    let Some(cutoff) = cutoff else {
        return Ok(RuntimeMemoryCohortGateRow {
            admission_count: 0,
            sample_count: 0,
            ready: false,
            reason: "runtime_shadow_cohort_empty".to_string(),
            aggregate_digest: None,
        });
    };
    sqlx::query_as::<_, RuntimeMemoryCohortGateRow>(
        r#"SELECT admission_count,sample_count,ready,reason,aggregate_digest
             FROM runtime_memory_rollout_cohort_gate($1,$2,$3)"#,
    )
    .bind(&current.contract)
    .bind(current.contract_rank)
    .bind(cutoff)
    .fetch_one(connection)
    .await
    .map_err(Into::into)
}

/// Reconcile at most one adjacent rank from database-owned retained truth.
/// Not-ready cohorts are expected typed no-ops and commit no mutation.
pub async fn reconcile(
    pool: &PgPool,
) -> RuntimeMemoryStoreResult<RuntimeMemoryRolloutReconcileOutcome> {
    let mut tx = pool.begin().await?;
    lock_execution_rollout_pair(&mut tx).await?;
    let current = get_for_update(&mut tx).await?;
    let next = match current.contract_rank {
        0 => RuntimeMemoryContract::DualWriteLegacyRead,
        1 => RuntimeMemoryContract::DualWriteV2Preferred,
        2 => RuntimeMemoryContract::V2Only,
        3 => {
            tx.commit().await?;
            return Ok(RuntimeMemoryRolloutReconcileOutcome::AlreadyCurrent(
                current,
            ));
        }
        _ => {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "runtime_memory_rollout_rank_invalid",
            });
        }
    };
    if current.contract_rank > 0 {
        let gate = cohort_gate(&mut tx, &current).await?;
        if !gate.ready {
            let outcome = RuntimeMemoryRolloutReconcileOutcome::NotReady {
                contract: current.contract,
                contract_rank: current.contract_rank,
                row_version: current.row_version,
                reason: gate.reason,
            };
            tx.commit().await?;
            return Ok(outcome);
        }
        if gate.admission_count <= 0
            || gate.sample_count < gate.admission_count
            || gate.aggregate_digest.as_deref().map(str::len) != Some(64)
        {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "runtime_memory_rollout_gate_projection_invalid",
            });
        }
    }
    let advanced = sqlx::query_as::<_, RuntimeMemoryRolloutAdvanceRow>(ADVANCE_SQL)
        .bind(&current.contract)
        .bind(next.as_str())
        .bind(next.rank())
        .bind(current.row_version)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Conflict {
            code: "runtime_memory_rollout_reconcile_cas_failed",
        })?;
    let row = RuntimeMemoryRolloutRow {
        singleton_id: SINGLETON_ID,
        contract: advanced.contract,
        contract_rank: advanced.contract_rank,
        row_version: advanced.row_version,
        updated_at: advanced.updated_at,
    };
    tx.commit().await?;
    Ok(RuntimeMemoryRolloutReconcileOutcome::Promoted(row))
}

/// Run reconciliation after a durable business transaction. Expected
/// not-ready state and infrastructure failures are observable but never turn
/// the already-committed runtime mutation into failure.
pub async fn reconcile_best_effort(pool: &PgPool, trigger: &'static str) {
    match reconcile(pool).await {
        Ok(RuntimeMemoryRolloutReconcileOutcome::Promoted(row)) => {
            tracing::info!(
                trigger,
                contract = %row.contract,
                rank = row.contract_rank,
                row_version = row.row_version,
                "runtime memory rollout promoted"
            );
        }
        Ok(RuntimeMemoryRolloutReconcileOutcome::AlreadyCurrent(row)) => {
            tracing::debug!(
                trigger,
                contract = %row.contract,
                rank = row.contract_rank,
                row_version = row.row_version,
                "runtime memory rollout already current"
            );
        }
        Ok(RuntimeMemoryRolloutReconcileOutcome::NotReady {
            contract,
            contract_rank,
            row_version,
            reason,
        }) => {
            tracing::debug!(
                trigger,
                %contract,
                rank = contract_rank,
                row_version,
                %reason,
                "runtime memory rollout reconciliation is not ready"
            );
        }
        Err(error) => {
            tracing::warn!(
                trigger,
                error = %error,
                "runtime memory rollout reconciliation failed after the business boundary"
            );
        }
    }
}

/// Advance the persisted contract by exactly one rank using a row-version CAS.
/// A failed CAS is classified from the same transaction so callers receive a
/// stable typed stale-version or invalid-transition result.
pub async fn advance(
    pool: &PgPool,
    from: RuntimeMemoryContract,
    to: RuntimeMemoryContract,
    expected_row_version: i64,
) -> RuntimeMemoryStoreResult<RuntimeMemoryRolloutRow> {
    if !from.may_advance_to(to) {
        return Err(RuntimeMemoryStoreError::InvalidContractTransition { from, to });
    }

    let mut tx = pool.begin().await?;
    let advanced = sqlx::query_as::<_, RuntimeMemoryRolloutAdvanceRow>(ADVANCE_SQL)
        .bind(from.as_str())
        .bind(to.as_str())
        .bind(to.rank())
        .bind(expected_row_version)
        .fetch_optional(&mut *tx)
        .await?;

    if let Some(advanced) = advanced {
        let row = RuntimeMemoryRolloutRow {
            singleton_id: SINGLETON_ID,
            contract: advanced.contract,
            contract_rank: advanced.contract_rank,
            row_version: advanced.row_version,
            updated_at: advanced.updated_at,
        };
        tx.commit().await?;
        return Ok(row);
    }

    let current = get_for_share(&mut *tx).await?;
    if current.row_version != expected_row_version {
        return Err(RuntimeMemoryStoreError::StaleVersion {
            entity: TABLE_NAME,
            expected: expected_row_version,
            actual: current.row_version,
        });
    }
    Err(RuntimeMemoryStoreError::InvalidContractTransition { from, to })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_memory_repo_contract_rollout_is_ranked_and_singleton() {
        assert_eq!(TABLE_NAME, "runtime_memory_rollout");
        assert_eq!(SINGLETON_ID, 1);
        assert_eq!(RuntimeMemoryContract::LegacyV1.rank(), 0);
        assert!(RuntimeMemoryContract::LegacyV1
            .may_advance_to(RuntimeMemoryContract::DualWriteLegacyRead));
        assert!(!RuntimeMemoryContract::LegacyV1.may_advance_to(RuntimeMemoryContract::V2Only));
        assert!(CONTRACT_RANK_CHECK_SQL.contains("contract_rank = CASE contract"));
        assert_eq!(
            RuntimeMemoryContract::ALL.map(RuntimeMemoryContract::as_str),
            CONTRACT_VALUES
        );
    }

    #[test]
    fn runtime_memory_store_rollout_uses_shared_read_and_adjacent_versioned_cas() {
        assert!(GET_FOR_SHARE_SQL.ends_with("FOR SHARE"));
        assert!(ADVANCE_SQL.contains("contract_rank + 1 = $3"));
        assert!(ADVANCE_SQL.contains("row_version = $4"));
        assert!(ADVANCE_SQL.contains("row_version = row_version + 1"));
        assert!(ADVANCE_SQL.contains("RETURNING"));
    }
}
