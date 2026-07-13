//! Exact persisted Verification Gate snapshot projection.

use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

fn unavailable(message: &str) -> crate::DbError {
    crate::DbError::Other(anyhow::anyhow!(message.to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptTerminalTruthRow {
    pub candidate_id: Uuid,
    pub attempt_id: Uuid,
    pub candidate_plan_hash: String,
    pub status: String,
    pub proof_evidence_ids: Vec<i64>,
    pub refutation_evidence_ids: Vec<i64>,
    pub blocker_evidence_ids: Vec<i64>,
    pub blocker_reason_code: Option<String>,
    pub finding_id: Option<Uuid>,
    pub finding_lineage_exact: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResidualRiskTruthRow {
    pub residual_risk_id: Uuid,
    pub reason_code: String,
    pub disclosure_status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationTruthRow {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub review_closed: bool,
    pub pending_work_items: u32,
    pub approved_ever: u32,
    pub attempts: Vec<AttemptTerminalTruthRow>,
    pub residual_risks: Vec<ResidualRiskTruthRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationUnitAuthorityRow {
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationTruthSetRow {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub expected_units: Vec<VerificationUnitAuthorityRow>,
    pub snapshots: Vec<VerificationTruthRow>,
}

#[derive(Debug, sqlx::FromRow)]
struct VerificationWaveAuthority {
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    wave_run_id: Uuid,
}

#[derive(Debug, sqlx::FromRow)]
struct VerificationAuthority {
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    wave_run_id: Uuid,
    wave_unit_id: Uuid,
    organization_id: Uuid,
    review_closed: bool,
    status: String,
    terminal_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, sqlx::FromRow)]
struct AttemptTruthRow {
    candidate_id: Uuid,
    attempt_id: Uuid,
    candidate_plan_hash: String,
    status: String,
    blocker_reason_code: Option<String>,
    finding_id: Option<Uuid>,
    finding_lineage_exact: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct ResidualRiskRow {
    residual_risk_id: Uuid,
    reason_code: String,
    disclosure_status: String,
}

/// Load every exact current-wave unit for a V2-only operation. Optional org
/// narrowing is server-owned routing context, never a model-supplied key.
pub async fn load_for_operation(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Option<Uuid>,
) -> crate::Result<VerificationTruthSetRow> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let wave = sqlx::query_as::<_, VerificationWaveAuthority>(
        r#"SELECT operation.operation_id,wave.scope_snapshot_id,wave.id AS wave_run_id
             FROM operation_state operation
             JOIN attack_wave_runs wave ON wave.operation_id=operation.operation_id
            WHERE operation.operation_id=$1
              AND operation.runtime_memory_contract='v2_only'
              AND operation.attack_execution_contract='v2_only'
              AND wave.status='verification' AND wave.terminal_at IS NULL
              AND wave.generation=(
                    SELECT MAX(current_wave.generation)
                      FROM attack_wave_runs current_wave
                     WHERE current_wave.operation_id=operation.operation_id
                       AND current_wave.status='verification'
                       AND current_wave.terminal_at IS NULL
                  )
            ORDER BY wave.generation DESC,wave.id
            LIMIT 1"#,
    )
    .bind(operation_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| unavailable("exact Verification wave authority is missing"))?;
    let authorities = sqlx::query_as::<_, VerificationAuthority>(
        r#"SELECT $1::UUID AS operation_id,$2::UUID AS scope_snapshot_id,
                  $3::UUID AS wave_run_id,wave_unit.id AS wave_unit_id,
                  wave_unit.organization_id,wave_unit.review_closed,
                  wave_unit.status,wave_unit.terminal_at
             FROM attack_wave_units wave_unit
            WHERE wave_unit.wave_run_id=$3
              AND wave_unit.operation_id=$1
              AND wave_unit.scope_snapshot_id=$2
              AND ($4::UUID IS NULL OR wave_unit.organization_id=$4)
            ORDER BY wave_unit.ordinal,wave_unit.organization_id"#,
    )
    .bind(wave.operation_id)
    .bind(wave.scope_snapshot_id)
    .bind(wave.wave_run_id)
    .bind(organization_id)
    .fetch_all(&mut *tx)
    .await?;
    if authorities.is_empty() {
        return Err(unavailable(
            "exact Verification wave unit authority is missing",
        ));
    }
    if authorities
        .iter()
        .any(|unit| unit.status != "verification" || unit.terminal_at.is_some())
    {
        return Err(unavailable(
            "Verification wave unit is not verification-ready",
        ));
    }
    let expected_units = authorities
        .iter()
        .map(|authority| VerificationUnitAuthorityRow {
            wave_unit_id: authority.wave_unit_id,
            organization_id: authority.organization_id,
        })
        .collect();
    let mut snapshots = Vec::with_capacity(authorities.len());
    for authority in authorities {
        snapshots.push(load_exact(&mut tx, authority).await?);
    }
    let truth = VerificationTruthSetRow {
        operation_id: wave.operation_id,
        scope_snapshot_id: wave.scope_snapshot_id,
        wave_run_id: wave.wave_run_id,
        expected_units,
        snapshots,
    };
    tx.commit().await?;
    Ok(truth)
}

async fn load_exact(
    tx: &mut Transaction<'_, Postgres>,
    authority: VerificationAuthority,
) -> crate::Result<VerificationTruthRow> {
    let pending_work_items: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM attack_candidate_work_items work
            WHERE work.operation_id=$1 AND work.scope_snapshot_id=$2
              AND work.wave_unit_id=$3
              AND work.organization_id=$4
              AND (
                work.decision_kind IS NULL
                OR (
                  work.decision_kind='no_candidate'
                  AND NOT EXISTS(
                    SELECT 1 FROM attack_candidate_work_item_evidence evidence
                     WHERE evidence.work_item_id=work.id AND evidence.role='decision'
                  )
                )
                OR (
                  work.decision_kind='candidate'
                  AND NOT EXISTS(
                    SELECT 1
                      FROM attack_candidates candidate
                      JOIN attack_candidate_approvals approval
                        ON approval.candidate_id=candidate.candidate_id
                       AND approval.operation_id=candidate.operation_uuid
                       AND approval.scope_snapshot_id=candidate.scope_snapshot_id
                       AND approval.wave_run_id=candidate.wave_run_id
                       AND approval.wave_unit_id=candidate.wave_unit_id
                       AND approval.organization_id=candidate.organization_id
                       AND approval.status<>'rejected'
                     WHERE candidate.source_work_item_id=work.id
                       AND candidate.terminal_attempt_id IS NOT NULL
                       AND candidate.disposition IN ('verified','refuted','blocked')
                  )
                )
              )"#,
    )
    .bind(authority.operation_id)
    .bind(authority.scope_snapshot_id)
    .bind(authority.wave_unit_id)
    .bind(authority.organization_id)
    .fetch_one(&mut **tx)
    .await?;
    let approved_ever: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(DISTINCT approval.candidate_id)
             FROM attack_candidate_approvals approval
            WHERE approval.operation_id=$1 AND approval.scope_snapshot_id=$2
              AND approval.wave_run_id=$3 AND approval.wave_unit_id=$4
              AND approval.organization_id=$5 AND approval.status<>'rejected'"#,
    )
    .bind(authority.operation_id)
    .bind(authority.scope_snapshot_id)
    .bind(authority.wave_run_id)
    .bind(authority.wave_unit_id)
    .bind(authority.organization_id)
    .fetch_one(&mut **tx)
    .await?;
    let attempt_rows = sqlx::query_as::<_, AttemptTruthRow>(
        r#"SELECT candidate.candidate_id,attempt.id AS attempt_id,
                  attempt.candidate_plan_hash,attempt.status,
                  NULLIF(BTRIM(attempt.result_json->>'blocker_reason_code'),'')
                    AS blocker_reason_code,
                  candidate.terminal_finding_id AS finding_id,
                  EXISTS(
                    SELECT 1 FROM finding_lineage lineage
                     WHERE lineage.finding_id=candidate.terminal_finding_id
                       AND lineage.candidate_attempt_id=attempt.id
                       AND lineage.candidate_id=candidate.candidate_id
                       AND lineage.operation_id=candidate.operation_uuid
                       AND lineage.scope_snapshot_id=candidate.scope_snapshot_id
                       AND lineage.wave_run_id=candidate.wave_run_id
                       AND lineage.wave_unit_id=candidate.wave_unit_id
                       AND lineage.organization_id=candidate.organization_id
                       AND lineage.candidate_plan_hash=candidate.candidate_plan_hash
                  ) AS finding_lineage_exact
             FROM attack_candidates candidate
             JOIN candidate_attempts attempt
               ON attempt.id=candidate.terminal_attempt_id
              AND attempt.candidate_id=candidate.candidate_id
              AND attempt.operation_id=candidate.operation_uuid
              AND attempt.scope_snapshot_id=candidate.scope_snapshot_id
              AND attempt.wave_run_id=candidate.wave_run_id
              AND attempt.wave_unit_id=candidate.wave_unit_id
              AND attempt.organization_id=candidate.organization_id
              AND attempt.candidate_plan_hash=candidate.candidate_plan_hash
            WHERE candidate.operation_uuid=$1 AND candidate.scope_snapshot_id=$2
              AND candidate.wave_run_id=$3 AND candidate.wave_unit_id=$4
              AND candidate.organization_id=$5
              AND candidate.disposition IN ('verified','refuted','blocked')
              AND EXISTS(
                SELECT 1 FROM attack_candidate_approvals approval
                 WHERE approval.candidate_id=candidate.candidate_id
                   AND approval.operation_id=candidate.operation_uuid
                   AND approval.scope_snapshot_id=candidate.scope_snapshot_id
                   AND approval.wave_run_id=candidate.wave_run_id
                   AND approval.wave_unit_id=candidate.wave_unit_id
                   AND approval.organization_id=candidate.organization_id
                   AND approval.status<>'rejected'
              )
            ORDER BY candidate.candidate_id"#,
    )
    .bind(authority.operation_id)
    .bind(authority.scope_snapshot_id)
    .bind(authority.wave_run_id)
    .bind(authority.wave_unit_id)
    .bind(authority.organization_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut attempts = Vec::with_capacity(attempt_rows.len());
    for row in attempt_rows {
        let links: Vec<(i64, String)> = sqlx::query_as(
            "SELECT evidence_id,role FROM candidate_attempt_evidence
             WHERE attempt_id=$1 ORDER BY evidence_id,role",
        )
        .bind(row.attempt_id)
        .fetch_all(&mut **tx)
        .await?;
        let ids_for = |role: &str| {
            links
                .iter()
                .filter(|(_, actual_role)| actual_role == role)
                .map(|(id, _)| *id)
                .collect::<Vec<_>>()
        };
        attempts.push(AttemptTerminalTruthRow {
            candidate_id: row.candidate_id,
            attempt_id: row.attempt_id,
            candidate_plan_hash: row.candidate_plan_hash,
            status: row.status,
            proof_evidence_ids: ids_for("proof"),
            refutation_evidence_ids: ids_for("refutation"),
            blocker_evidence_ids: ids_for("blocker"),
            blocker_reason_code: row.blocker_reason_code,
            finding_id: row.finding_id,
            finding_lineage_exact: row.finding_lineage_exact,
        });
    }
    let residual_risks = sqlx::query_as::<_, ResidualRiskRow>(
        r#"SELECT id AS residual_risk_id,reason_code,disclosure_status
             FROM attack_residual_risks
            WHERE operation_id=$1 AND scope_snapshot_id=$2 AND wave_run_id=$3
              AND wave_unit_id=$4 AND organization_id=$5
            ORDER BY id"#,
    )
    .bind(authority.operation_id)
    .bind(authority.scope_snapshot_id)
    .bind(authority.wave_run_id)
    .bind(authority.wave_unit_id)
    .bind(authority.organization_id)
    .fetch_all(&mut **tx)
    .await?
    .into_iter()
    .map(|row| ResidualRiskTruthRow {
        residual_risk_id: row.residual_risk_id,
        reason_code: row.reason_code,
        disclosure_status: row.disclosure_status,
    })
    .collect();
    Ok(VerificationTruthRow {
        operation_id: authority.operation_id,
        scope_snapshot_id: authority.scope_snapshot_id,
        wave_run_id: authority.wave_run_id,
        wave_unit_id: authority.wave_unit_id,
        organization_id: authority.organization_id,
        review_closed: authority.review_closed,
        pending_work_items: u32::try_from(pending_work_items).map_err(|_| {
            crate::DbError::Other(anyhow::anyhow!("pending work-item count overflow"))
        })?,
        approved_ever: u32::try_from(approved_ever).map_err(|_| {
            crate::DbError::Other(anyhow::anyhow!("approved Candidate count overflow"))
        })?,
        attempts,
        residual_risks,
    })
}
