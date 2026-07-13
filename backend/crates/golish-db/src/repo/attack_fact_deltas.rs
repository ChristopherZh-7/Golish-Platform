//! Scoped FactDelta writes produced by terminal Candidate attempts.

use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttackFactDeltaKind {
    Created,
    Updated,
    Refuted,
    NewSurface,
}

impl AttackFactDeltaKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
            Self::Refuted => "refuted",
            Self::NewSurface => "new_surface",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "created" => Some(Self::Created),
            "updated" => Some(Self::Updated),
            "refuted" => Some(Self::Refuted),
            "new_surface" => Some(Self::NewSurface),
            _ => None,
        }
    }
}

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
    created_at: DateTime<Utc>,
    terminal_at: DateTime<Utc>,
}

const COLUMNS: &str = "id,source_attempt_id,candidate_id,operation_id,scope_snapshot_id,\
    wave_run_id,wave_unit_id,organization_id,target_live_id,target_type_at_time,\
    target_value_at_time,target_identity_hash,candidate_plan_hash,canonical_ref_kind,\
    canonical_ref_id,canonical_ref_version,canonical_ref_hash,delta_kind,dedupe_hash,status,\
    consumed_by_wave_run_id,created_at,updated_at,consumed_at";

fn conflict(message: &str) -> crate::DbError {
    crate::DbError::Other(anyhow::anyhow!(message.to_string()))
}

/// Stable identity of one semantic fact change. Attempt/Candidate/Wave prose
/// and evidence are provenance of the first observation, not dedupe inputs.
pub fn semantic_dedupe_hash(
    target_identity_hash: &str,
    canonical_ref_kind: &str,
    canonical_ref_id: Uuid,
    canonical_ref_version: i64,
    canonical_ref_hash: &str,
    delta_kind: &str,
) -> crate::Result<String> {
    let kind = AttackFactDeltaKind::parse(delta_kind)
        .ok_or_else(|| conflict("invalid FactDelta proposal kind"))?;
    if target_identity_hash.trim().is_empty()
        || target_identity_hash.trim() != target_identity_hash
        || canonical_ref_kind.trim().is_empty()
        || canonical_ref_kind.trim() != canonical_ref_kind
        || canonical_ref_id.is_nil()
        || canonical_ref_version <= 0
        || canonical_ref_hash.trim().is_empty()
        || canonical_ref_hash.trim() != canonical_ref_hash
    {
        return Err(conflict("invalid FactDelta semantic identity"));
    }
    let material = serde_json::json!({
        "schema_version": "attack-fact-delta-semantic-v1",
        "target_identity_hash": target_identity_hash,
        "canonical_ref_kind": canonical_ref_kind,
        "canonical_ref_id": canonical_ref_id,
        "canonical_ref_version": canonical_ref_version,
        "canonical_ref_hash": canonical_ref_hash,
        "delta_kind": kind.as_str(),
    });
    Ok(format!(
        "sha256:{}",
        super::operation_scope_decisions::sha256_json(&material)
    ))
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
        || command.dedupe_hash.trim().is_empty()
        || command.canonical_ref_version <= 0
        || command.evidence_ids.is_empty()
    {
        return Err(conflict("invalid FactDelta proposal"));
    }
    AttackFactDeltaKind::parse(&command.delta_kind)
        .ok_or_else(|| conflict("invalid FactDelta proposal kind"))?;
    let mut evidence_ids = command.evidence_ids.clone();
    evidence_ids.sort_unstable();
    let original_len = evidence_ids.len();
    evidence_ids.dedup();
    if evidence_ids.len() != original_len || evidence_ids.iter().any(|id| *id <= 0) {
        return Err(conflict("invalid FactDelta evidence ids"));
    }
    let target = sqlx::query_as::<_, TerminalAttemptTarget>(
        r#"SELECT target_live_id,target_type_at_time,target_value_at_time,target_identity_hash,
                  created_at,terminal_at
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
    let expected_dedupe_hash = semantic_dedupe_hash(
        &target.target_identity_hash,
        &command.canonical_ref_kind,
        command.canonical_ref_id,
        command.canonical_ref_version,
        &command.canonical_ref_hash,
        &command.delta_kind,
    )?;
    if command.dedupe_hash != expected_dedupe_hash {
        return Err(conflict("FactDelta semantic dedupe hash drift"));
    }
    let source_attempt_evidence: Vec<(i64, DateTime<Utc>)> = sqlx::query_as(
        "SELECT link.evidence_id,evidence.created_at
           FROM candidate_attempt_evidence AS link
           JOIN audit_log AS evidence ON evidence.id=link.evidence_id
          WHERE link.attempt_id=$1 AND link.role='fact_delta'
            AND link.evidence_id=ANY($2)
          ORDER BY link.evidence_id",
    )
    .bind(command.source_attempt_id)
    .bind(&evidence_ids)
    .fetch_all(&mut **tx)
    .await?;
    let source_attempt_evidence_ids = source_attempt_evidence
        .iter()
        .map(|(evidence_id, _)| *evidence_id)
        .collect::<Vec<_>>();
    if source_attempt_evidence_ids != evidence_ids {
        return Err(conflict(
            "FactDelta proposal evidence is not frozen on the source Attempt",
        ));
    }
    if source_attempt_evidence.iter().any(|(_, observed_at)| {
        *observed_at < target.created_at || *observed_at > target.terminal_at
    }) {
        return Err(conflict(
            "FactDelta proposal evidence is outside the source Attempt interval",
        ));
    }
    let select_sql = format!(
        "SELECT {COLUMNS} FROM attack_fact_deltas
         WHERE operation_id=$1 AND organization_id=$2 AND dedupe_hash=$3 FOR UPDATE"
    );
    let existing = sqlx::query_as::<_, AttackFactDeltaRow>(&select_sql)
        .bind(command.operation_id)
        .bind(command.organization_id)
        .bind(&command.dedupe_hash)
        .fetch_optional(&mut **tx)
        .await?;
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
    let inserted = if existing.is_none() {
        sqlx::query_as::<_, AttackFactDeltaRow>(&insert_sql)
            .bind(Uuid::new_v4())
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
            .await?
    } else {
        None
    };
    let (row, inserted_now) = if let Some(row) = existing {
        (row, false)
    } else if let Some(row) = inserted {
        (row, true)
    } else {
        let row = sqlx::query_as::<_, AttackFactDeltaRow>(&select_sql)
            .bind(command.operation_id)
            .bind(command.organization_id)
            .bind(&command.dedupe_hash)
            .fetch_one(&mut **tx)
            .await?;
        (row, false)
    };
    if row.target_identity_hash != target.target_identity_hash
        || row.canonical_ref_kind != command.canonical_ref_kind
        || row.canonical_ref_id != command.canonical_ref_id
        || row.canonical_ref_version != command.canonical_ref_version
        || row.canonical_ref_hash != command.canonical_ref_hash
        || row.delta_kind != command.delta_kind
        || row.dedupe_hash != expected_dedupe_hash
    {
        return Err(conflict("FactDelta idempotency payload drift"));
    }
    if inserted_now {
        for evidence_id in evidence_ids {
            sqlx::query(
                "INSERT INTO attack_fact_delta_evidence(fact_delta_id,evidence_id,role)
                 VALUES($1,$2,'fact_delta')",
            )
            .bind(row.id)
            .bind(evidence_id)
            .execute(&mut **tx)
            .await?;
        }
    } else if row.source_attempt_id == command.source_attempt_id {
        if row.candidate_id != command.candidate_id
            || row.scope_snapshot_id != command.scope_snapshot_id
            || row.wave_run_id != command.wave_run_id
            || row.wave_unit_id != command.wave_unit_id
            || row.candidate_plan_hash != command.candidate_plan_hash
        {
            return Err(conflict("FactDelta idempotency payload drift"));
        }
        let persisted_evidence_ids: Vec<i64> = sqlx::query_scalar(
            "SELECT evidence_id FROM attack_fact_delta_evidence
              WHERE fact_delta_id=$1 AND role='fact_delta' ORDER BY evidence_id",
        )
        .bind(row.id)
        .fetch_all(&mut **tx)
        .await?;
        if persisted_evidence_ids != evidence_ids {
            return Err(conflict("FactDelta idempotency payload drift"));
        }
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
