//! Monotonic deployment default for the Candidate V2 execution contract.

use chrono::{DateTime, Utc};
use golish_core::AttackExecutionContract;
use sqlx::{PgConnection, PgPool, Postgres, Transaction};

use super::attack_execution_shadow;

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct AttackExecutionRolloutRow {
    pub singleton: bool,
    pub contract: String,
    pub rank: i16,
    pub row_version: i64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttackExecutionRolloutReconcileOutcome {
    AlreadyCurrent(AttackExecutionRolloutRow),
    Promoted(AttackExecutionRolloutRow),
    NotReady {
        contract: String,
        rank: i16,
        row_version: i64,
        reason: String,
    },
}

#[derive(Debug, sqlx::FromRow)]
struct CandidateCohortGateRow {
    admission_count: i64,
    candidate_unit_count: i64,
    sample_count: i64,
    ready: bool,
    reason: String,
}

#[derive(Debug, Clone, Copy)]
struct CandidateCohortAttestation {
    admission_cutoff: i64,
    admission_count: i64,
    candidate_unit_count: i64,
    sample_count: i64,
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

async fn get_for_update(connection: &mut PgConnection) -> crate::Result<AttackExecutionRolloutRow> {
    sqlx::query_as(
        r#"SELECT singleton,contract,rank,row_version,updated_at
           FROM attack_execution_rollout WHERE singleton=TRUE FOR UPDATE"#,
    )
    .fetch_optional(connection)
    .await?
    .ok_or_else(|| crate::DbError::NotFound("attack_execution_rollout".to_string()))
}

async fn lock_execution_rollout_pair(connection: &mut PgConnection) -> crate::Result<()> {
    sqlx::query("SELECT lock_execution_rollout_pair()")
        .execute(connection)
        .await?;
    Ok(())
}

/// Low-level adjacent CAS retained for controlled migration and repository fixtures.
///
/// This seam deliberately does not evaluate shadow-read evidence and must not be used by a
/// production control-plane caller. The database trigger independently applies
/// the Candidate cohort gate, so even this fixture seam cannot bypass a rank
/// transition. Production callers use [`reconcile_attack_execution_rollout`].
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
    let current = get_for_update(tx).await?;
    Err(crate::DbError::Other(anyhow::anyhow!(
        "stale or non-adjacent attack rollout: expected_version={expected_version}, current_version={}, next={}",
        current.row_version,
        next.as_str()
    )))
}

async fn candidate_cohort_attestation(
    connection: &mut PgConnection,
    current: &AttackExecutionRolloutRow,
) -> crate::Result<std::result::Result<CandidateCohortAttestation, String>> {
    let admission_cutoff: Option<i64> = sqlx::query_scalar(
        r#"SELECT MAX(admission_seq)
             FROM attack_execution_candidate_admissions
            WHERE attack_execution_contract=$1 AND rollout_rank=$2"#,
    )
    .bind(&current.contract)
    .bind(current.rank)
    .fetch_one(&mut *connection)
    .await?;
    let Some(admission_cutoff) = admission_cutoff else {
        return Ok(Err("candidate_cohort_empty".to_string()));
    };
    let gate = sqlx::query_as::<_, CandidateCohortGateRow>(
        r#"SELECT admission_count,candidate_unit_count,sample_count,ready,reason
             FROM attack_execution_candidate_cohort_gate($1,$2,$3)"#,
    )
    .bind(&current.contract)
    .bind(current.rank)
    .bind(admission_cutoff)
    .fetch_one(&mut *connection)
    .await?;
    if !gate.ready {
        return Ok(Err(gate.reason));
    }
    let aggregate = attack_execution_shadow::aggregate_for_candidate_cohort(
        connection,
        &current.contract,
        current.rank,
        admission_cutoff,
    )
    .await?;
    let sample_count = i64::try_from(aggregate.sample_count).map_err(|_| {
        crate::DbError::Other(anyhow::anyhow!(
            "attack rollout Candidate cohort sample count overflow"
        ))
    })?;
    if aggregate.mismatch_count != 0
        || aggregate.incomplete_count != 0
        || sample_count != gate.sample_count
        || gate.sample_count != gate.candidate_unit_count
        || gate.admission_count <= 0
    {
        return Ok(Err(format!(
            "candidate_shadow_canonical_rebuild_not_ready: sample_count={sample_count}, db_sample_count={}, candidate_unit_count={}, mismatch_count={}, incomplete_count={}",
            gate.sample_count,
            gate.candidate_unit_count,
            aggregate.mismatch_count,
            aggregate.incomplete_count
        )));
    }
    Ok(Ok(CandidateCohortAttestation {
        admission_cutoff,
        admission_count: gate.admission_count,
        candidate_unit_count: gate.candidate_unit_count,
        sample_count: gate.sample_count,
    }))
}

/// Promote the deployment default after complete, mismatch-free shadow comparison.
///
/// The repository loads and aggregates immutable persisted samples itself; a
/// caller cannot supply or forge counts. Rank zero to one only enables sample
/// production. Rejected later promotions return before issuing the CAS.
pub async fn promote_attack_execution_rollout(
    tx: &mut Transaction<'_, Postgres>,
    expected_version: i64,
    next: AttackExecutionContract,
) -> crate::Result<AttackExecutionRolloutRow> {
    lock_execution_rollout_pair(tx).await?;
    let current = get_for_update(tx).await?;
    if current.row_version != expected_version {
        return advance_attack_execution_rollout(tx, expected_version, next).await;
    }
    // Rank 0 -> 1 enables dual writing and therefore cannot have prior dual
    // samples. Every later promotion must consume the exact admitted cohort
    // frozen under the immediately preceding deployment contract.
    if next != AttackExecutionContract::DualWriteReadLegacy {
        let attestation = candidate_cohort_attestation(tx, &current).await?;
        let ready = attestation.map_err(|reason| {
            crate::DbError::Other(anyhow::anyhow!("attack_rollout_cohort_not_ready: {reason}"))
        })?;
        let _frozen_cohort = (
            ready.admission_cutoff,
            ready.admission_count,
            ready.candidate_unit_count,
            ready.sample_count,
        );
    }
    advance_attack_execution_rollout(tx, expected_version, next).await
}

/// Reconcile at most one adjacent deployment rank.
///
/// `NotReady` is an expected typed no-op. The transaction takes the rollout
/// singleton `FOR UPDATE`, freezing the admission cutoff while both PostgreSQL
/// and the Rust canonical read model validate the same cohort. The database
/// transition trigger recomputes the gate and writes its own receipt.
pub async fn reconcile_attack_execution_rollout(
    pool: &PgPool,
) -> crate::Result<AttackExecutionRolloutReconcileOutcome> {
    let mut tx = pool.begin().await?;
    lock_execution_rollout_pair(&mut tx).await?;
    let current = get_for_update(&mut tx).await?;
    let next = match current.rank {
        0 => AttackExecutionContract::DualWriteReadLegacy,
        1 => AttackExecutionContract::DualWriteReadV2Fallback,
        2 => AttackExecutionContract::V2Only,
        3 => {
            tx.commit().await?;
            return Ok(AttackExecutionRolloutReconcileOutcome::AlreadyCurrent(
                current,
            ));
        }
        _ => {
            return Err(crate::DbError::Other(anyhow::anyhow!(
                "attack rollout rank is outside the deployment contract"
            )));
        }
    };
    if current.rank > 0 {
        match candidate_cohort_attestation(&mut tx, &current).await? {
            Ok(attestation) => {
                tracing::debug!(
                    contract = %current.contract,
                    rank = current.rank,
                    admission_cutoff = attestation.admission_cutoff,
                    admission_count = attestation.admission_count,
                    candidate_unit_count = attestation.candidate_unit_count,
                    sample_count = attestation.sample_count,
                    "attack rollout Candidate cohort is ready"
                );
            }
            Err(reason) => {
                let outcome = AttackExecutionRolloutReconcileOutcome::NotReady {
                    contract: current.contract,
                    rank: current.rank,
                    row_version: current.row_version,
                    reason,
                };
                tx.commit().await?;
                return Ok(outcome);
            }
        }
    }
    let promoted = advance_attack_execution_rollout(&mut tx, current.row_version, next).await?;
    tx.commit().await?;
    Ok(AttackExecutionRolloutReconcileOutcome::Promoted(promoted))
}

/// Run one reconciliation attempt without coupling its availability to the
/// caller's business transaction. Promotion errors are observable but never
/// turn a committed final seal (or a new-operation request) back into failure.
pub async fn reconcile_attack_execution_rollout_best_effort(pool: &PgPool, trigger: &'static str) {
    match reconcile_attack_execution_rollout(pool).await {
        Ok(AttackExecutionRolloutReconcileOutcome::Promoted(row)) => {
            tracing::info!(
                trigger,
                contract = %row.contract,
                rank = row.rank,
                row_version = row.row_version,
                "attack execution rollout promoted"
            );
        }
        Ok(AttackExecutionRolloutReconcileOutcome::AlreadyCurrent(row)) => {
            tracing::debug!(
                trigger,
                contract = %row.contract,
                rank = row.rank,
                row_version = row.row_version,
                "attack execution rollout already current"
            );
        }
        Ok(AttackExecutionRolloutReconcileOutcome::NotReady {
            contract,
            rank,
            row_version,
            reason,
        }) => {
            tracing::debug!(
                trigger,
                %contract,
                rank,
                row_version,
                %reason,
                "attack execution rollout reconciliation is not ready"
            );
        }
        Err(error) => {
            tracing::warn!(
                trigger,
                error = %error,
                "attack execution rollout reconciliation failed after the business boundary"
            );
        }
    }
}
