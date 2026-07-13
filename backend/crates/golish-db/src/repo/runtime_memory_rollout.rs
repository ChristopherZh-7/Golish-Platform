//! Schema contract for the monotonic runtime-memory rollout singleton.
//!
//! Owns the stable row, contract vocabulary, shared-lock read, and adjacent
//! row-version CAS used to roll the platform forward without per-process drift.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Executor, PgPool, Postgres};

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
