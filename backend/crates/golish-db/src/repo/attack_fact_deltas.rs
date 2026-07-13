//! Scoped FactDelta writes produced by terminal Candidate attempts.

use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]
pub struct AttackFactDeltaRow {
    pub id: Uuid,
    pub source_attempt_id: Uuid,
    pub candidate_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub target_live_id: Option<Uuid>,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub target_identity_hash: String,
    pub candidate_plan_hash: String,
    pub canonical_ref_kind: String,
    pub canonical_ref_id: Uuid,
    pub canonical_ref_version: i64,
    pub canonical_ref_hash: String,
    pub delta_kind: String,
    pub dedupe_hash: String,
    pub status: String,
    pub consumed_by_wave_run_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct ProposeAttackFactDelta {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub source_attempt_id: Uuid,
    pub candidate_id: Uuid,
    pub candidate_plan_hash: String,
    pub canonical_ref_kind: String,
    pub canonical_ref_id: Uuid,
    pub canonical_ref_version: i64,
    pub canonical_ref_hash: String,
    pub delta_kind: String,
    pub dedupe_hash: String,
    pub evidence_ids: Vec<i64>,
}

#[derive(Debug, sqlx::FromRow)]
struct TerminalAttemptTarget {
    target_live_id: Option<Uuid>,
    target_type_at_time: String,
    target_value_at_time: String,
    target_identity_hash: String,
}

const COLUMNS: &str = "id,source_attempt_id,candidate_id,operation_id,scope_snapshot_id,\
    wave_run_id,wave_unit_id,organization_id,target_live_id,target_type_at_time,\
    target_value_at_time,target_identity_hash,candidate_plan_hash,canonical_ref_kind,\
    canonical_ref_id,canonical_ref_version,canonical_ref_hash,delta_kind,dedupe_hash,status,\
    consumed_by_wave_run_id,created_at,updated_at,consumed_at";

fn conflict(message: &str) -> crate::DbError {
    crate::DbError::Other(anyhow::anyhow!(message.to_string()))
}

/// Propose or exactly replay one terminal Attempt's FactDelta. Frozen target
/// identity is derived from the Attempt, never accepted from a caller.
pub async fn propose_fact_delta(
    tx: &mut Transaction<'_, Postgres>,
    command: ProposeAttackFactDelta,
) -> crate::Result<AttackFactDeltaRow> {
    if command.candidate_plan_hash.trim().is_empty()
        || command.canonical_ref_kind.trim().is_empty()
        || command.canonical_ref_hash.trim().is_empty()
        || command.delta_kind.trim().is_empty()
        || command.dedupe_hash.trim().is_empty()
        || command.canonical_ref_version <= 0
        || command.evidence_ids.is_empty()
    {
        return Err(conflict("invalid FactDelta proposal"));
    }
    let mut evidence_ids = command.evidence_ids.clone();
    evidence_ids.sort_unstable();
    let original_len = evidence_ids.len();
    evidence_ids.dedup();
    if evidence_ids.len() != original_len || evidence_ids.iter().any(|id| *id <= 0) {
        return Err(conflict("invalid FactDelta evidence ids"));
    }
    let target = sqlx::query_as::<_, TerminalAttemptTarget>(
        r#"SELECT target_live_id,target_type_at_time,target_value_at_time,target_identity_hash
             FROM candidate_attempts
            WHERE id=$1 AND candidate_id=$2 AND operation_id=$3 AND scope_snapshot_id=$4
              AND wave_run_id=$5 AND wave_unit_id=$6 AND organization_id=$7
              AND candidate_plan_hash=$8 AND status IN ('verified','refuted','blocked')
              AND terminal_at IS NOT NULL
            FOR UPDATE"#,
    )
    .bind(command.source_attempt_id)
    .bind(command.candidate_id)
    .bind(command.operation_id)
    .bind(command.scope_snapshot_id)
    .bind(command.wave_run_id)
    .bind(command.wave_unit_id)
    .bind(command.organization_id)
    .bind(&command.candidate_plan_hash)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| crate::DbError::NotFound("terminal_candidate_attempt".to_string()))?;
    let id = Uuid::new_v4();
    let insert_sql = format!(
        "INSERT INTO attack_fact_deltas(
             id,source_attempt_id,candidate_id,operation_id,scope_snapshot_id,wave_run_id,
             wave_unit_id,organization_id,target_live_id,target_type_at_time,
             target_value_at_time,target_identity_hash,candidate_plan_hash,canonical_ref_kind,
             canonical_ref_id,canonical_ref_version,canonical_ref_hash,delta_kind,dedupe_hash)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19)
         ON CONFLICT(operation_id,organization_id,dedupe_hash) DO NOTHING
         RETURNING {COLUMNS}"
    );
    let inserted = sqlx::query_as::<_, AttackFactDeltaRow>(&insert_sql)
        .bind(id)
        .bind(command.source_attempt_id)
        .bind(command.candidate_id)
        .bind(command.operation_id)
        .bind(command.scope_snapshot_id)
        .bind(command.wave_run_id)
        .bind(command.wave_unit_id)
        .bind(command.organization_id)
        .bind(target.target_live_id)
        .bind(&target.target_type_at_time)
        .bind(&target.target_value_at_time)
        .bind(&target.target_identity_hash)
        .bind(&command.candidate_plan_hash)
        .bind(&command.canonical_ref_kind)
        .bind(command.canonical_ref_id)
        .bind(command.canonical_ref_version)
        .bind(&command.canonical_ref_hash)
        .bind(&command.delta_kind)
        .bind(&command.dedupe_hash)
        .fetch_optional(&mut **tx)
        .await?;
    let row = if let Some(row) = inserted {
        row
    } else {
        let select_sql = format!(
            "SELECT {COLUMNS} FROM attack_fact_deltas
             WHERE operation_id=$1 AND organization_id=$2 AND dedupe_hash=$3 FOR UPDATE"
        );
        sqlx::query_as::<_, AttackFactDeltaRow>(&select_sql)
            .bind(command.operation_id)
            .bind(command.organization_id)
            .bind(&command.dedupe_hash)
            .fetch_one(&mut **tx)
            .await?
    };
    if row.source_attempt_id != command.source_attempt_id
        || row.candidate_id != command.candidate_id
        || row.scope_snapshot_id != command.scope_snapshot_id
        || row.wave_run_id != command.wave_run_id
        || row.wave_unit_id != command.wave_unit_id
        || row.candidate_plan_hash != command.candidate_plan_hash
        || row.canonical_ref_kind != command.canonical_ref_kind
        || row.canonical_ref_id != command.canonical_ref_id
        || row.canonical_ref_version != command.canonical_ref_version
        || row.canonical_ref_hash != command.canonical_ref_hash
        || row.delta_kind != command.delta_kind
    {
        return Err(conflict("FactDelta idempotency payload drift"));
    }
    for evidence_id in evidence_ids {
        sqlx::query(
            "INSERT INTO attack_fact_delta_evidence(fact_delta_id,evidence_id,role)
             VALUES($1,$2,'fact_delta') ON CONFLICT DO NOTHING",
        )
        .bind(row.id)
        .bind(evidence_id)
        .execute(&mut **tx)
        .await?;
    }
    Ok(row)
}

/// Consume one exact accepted FactDelta into a same-operation/scope Wave.
pub async fn consume_fact_delta(
    tx: &mut Transaction<'_, Postgres>,
    fact_delta_id: Uuid,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    consumed_by_wave_run_id: Uuid,
) -> crate::Result<AttackFactDeltaRow> {
    let sql = format!(
        "UPDATE attack_fact_deltas SET status='consumed',consumed_by_wave_run_id=$5,
             consumed_at=NOW(),updated_at=NOW()
         WHERE id=$1 AND operation_id=$2 AND scope_snapshot_id=$3
           AND organization_id=$4 AND status='accepted'
         RETURNING {COLUMNS}"
    );
    sqlx::query_as(&sql)
        .bind(fact_delta_id)
        .bind(operation_id)
        .bind(scope_snapshot_id)
        .bind(organization_id)
        .bind(consumed_by_wave_run_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| conflict("FactDelta consume CAS or ownership mismatch"))
}
