use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::Result;

pub const TABLE_NAME: &str = "cleanup_absence_checks";

#[derive(Clone, Debug, sqlx::FromRow, PartialEq, Eq)]
pub struct CleanupAbsenceCheckRow {
    pub id: Uuid,
    pub obligation_id: Uuid,
    pub cleanup_attempt_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub verifier_worker_run_id: Option<Uuid>,
    pub verifier_key: String,
    pub resource_identity_hash: String,
    pub disposition: String,
    pub checked_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordAbsenceCheck {
    pub id: Uuid,
    pub cleanup_attempt_id: Uuid,
    pub verifier_worker_run_id: Option<Uuid>,
    pub verifier_key: String,
    pub resource_identity_hash: String,
    pub disposition: String,
    pub evidence: Vec<(i64, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppliedAbsenceCheck {
    pub check: CleanupAbsenceCheckRow,
    pub attempt: super::cleanup_attempts::CleanupAttemptRow,
    pub obligation: super::cleanup_obligations::CleanupObligationRow,
}

fn is_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub async fn record_and_apply(
    pool: &PgPool,
    input: &RecordAbsenceCheck,
) -> Result<AppliedAbsenceCheck> {
    if input.verifier_key.trim().is_empty()
        || input.verifier_key.len() > 256
        || input.verifier_key.chars().any(char::is_control)
        || !is_hash(&input.resource_identity_hash)
        || !matches!(
            input.disposition.as_str(),
            "absent" | "still_present" | "inconclusive"
        )
        || input.evidence.is_empty()
        || input.evidence.len() > 1024
        || input.evidence.iter().any(|(id, role)| {
            *id <= 0
                || !matches!(
                    role.as_str(),
                    "absence" | "presence" | "inconclusive" | "support"
                )
        })
    {
        return Err(anyhow::anyhow!("cleanup_absence_check_invalid").into());
    }
    let mut expected_evidence = input.evidence.clone();
    expected_evidence.sort_unstable();
    expected_evidence.dedup();
    if expected_evidence.len() != input.evidence.len() {
        return Err(anyhow::anyhow!("cleanup_absence_evidence_duplicate").into());
    }
    let mut tx = pool.begin().await?;
    let current_attempt = sqlx::query_as::<_, super::cleanup_attempts::CleanupAttemptRow>(
        "SELECT * FROM cleanup_attempts WHERE id=$1 FOR UPDATE",
    )
    .bind(input.cleanup_attempt_id)
    .fetch_one(&mut *tx)
    .await?;
    let current_obligation = sqlx::query_as::<_, super::cleanup_obligations::CleanupObligationRow>(
        "SELECT * FROM cleanup_obligations WHERE id=$1 FOR UPDATE",
    )
    .bind(current_attempt.obligation_id)
    .fetch_one(&mut *tx)
    .await?;
    if let Some(existing) = sqlx::query_as::<_, CleanupAbsenceCheckRow>(
        "SELECT * FROM cleanup_absence_checks WHERE cleanup_attempt_id=$1 FOR SHARE",
    )
    .bind(current_attempt.id)
    .fetch_optional(&mut *tx)
    .await?
    {
        let stored_evidence = sqlx::query_as::<_, (i64, String)>(
            r#"SELECT evidence_id,role FROM cleanup_absence_check_evidence
                WHERE absence_check_id=$1 ORDER BY evidence_id,role"#,
        )
        .bind(existing.id)
        .fetch_all(&mut *tx)
        .await?;
        let mut expected = input.evidence.clone();
        expected.sort_unstable();
        if existing.id != input.id
            || existing.verifier_worker_run_id != input.verifier_worker_run_id
            || existing.verifier_key != input.verifier_key
            || existing.resource_identity_hash != input.resource_identity_hash
            || existing.disposition != input.disposition
            || stored_evidence != expected
        {
            return Err(anyhow::anyhow!("cleanup_absence_replay_conflict").into());
        }
        if existing.disposition == "absent" {
            super::cleanup_obligations::append_terminal_event_with_connection(
                &mut tx,
                current_obligation.id,
                super::cleanup_obligations::CleanupTerminalSource::IndependentAbsence {
                    cleanup_attempt_id: current_attempt.id,
                    absence_check_id: existing.id,
                },
            )
            .await?;
        }
        tx.commit().await?;
        return Ok(AppliedAbsenceCheck {
            check: existing,
            attempt: current_attempt,
            obligation: current_obligation,
        });
    }
    if current_attempt.status != "cleaned_pending_verification"
        || current_obligation.status != "in_progress"
        || current_obligation.resource_identity_hash != input.resource_identity_hash
    {
        return Err(anyhow::anyhow!("cleanup_absence_state_mismatch").into());
    }
    super::foothold_candidates::validate_evidence_authority(
        &mut tx,
        current_attempt.operation_id,
        current_attempt.scope_snapshot_id,
        current_attempt.organization_id_at_time,
        &input.evidence.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
    )
    .await?;
    let execution_evidence: Vec<i64> =
        sqlx::query_scalar("SELECT evidence_id FROM cleanup_attempt_evidence WHERE attempt_id=$1")
            .bind(current_attempt.id)
            .fetch_all(&mut *tx)
            .await?;
    if input
        .evidence
        .iter()
        .any(|(evidence_id, _)| execution_evidence.contains(evidence_id))
    {
        return Err(anyhow::anyhow!("cleanup_absence_evidence_not_independent").into());
    }
    let check = sqlx::query_as::<_, CleanupAbsenceCheckRow>(
        r#"INSERT INTO cleanup_absence_checks(
               id,obligation_id,cleanup_attempt_id,operation_id,project_scope_id,
               scope_snapshot_id,organization_id_at_time,verifier_worker_run_id,
               verifier_key,resource_identity_hash,disposition
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) RETURNING *"#,
    )
    .bind(input.id)
    .bind(current_obligation.id)
    .bind(current_attempt.id)
    .bind(current_attempt.operation_id)
    .bind(current_attempt.project_scope_id)
    .bind(current_attempt.scope_snapshot_id)
    .bind(current_attempt.organization_id_at_time)
    .bind(input.verifier_worker_run_id)
    .bind(&input.verifier_key)
    .bind(&input.resource_identity_hash)
    .bind(&input.disposition)
    .fetch_one(&mut *tx)
    .await?;
    for (evidence_id, role) in &input.evidence {
        sqlx::query(
            r#"INSERT INTO cleanup_absence_check_evidence(
                   absence_check_id,evidence_id,role
               ) VALUES($1,$2,$3)"#,
        )
        .bind(check.id)
        .bind(evidence_id)
        .bind(role)
        .execute(&mut *tx)
        .await?;
    }
    let absent = input.disposition == "absent";
    let attempt = sqlx::query_as::<_, super::cleanup_attempts::CleanupAttemptRow>(
        r#"UPDATE cleanup_attempts
              SET status=CASE WHEN $2 THEN 'verified_absent' ELSE 'verification_failed' END,
                  completed_at=NOW(),row_version=row_version+1
            WHERE id=$1 RETURNING *"#,
    )
    .bind(current_attempt.id)
    .bind(absent)
    .fetch_one(&mut *tx)
    .await?;
    let obligation = sqlx::query_as::<_, super::cleanup_obligations::CleanupObligationRow>(
        r#"UPDATE cleanup_obligations
                  SET status=CASE WHEN $2 THEN 'verified_absent' ELSE 'open' END,
                      terminal_at=CASE WHEN $2 THEN NOW() ELSE NULL END,
                      row_version=row_version+1,updated_at=NOW()
                WHERE id=$1 RETURNING *"#,
    )
    .bind(current_obligation.id)
    .bind(absent)
    .fetch_one(&mut *tx)
    .await?;
    if absent {
        super::cleanup_obligations::append_terminal_event_with_connection(
            &mut tx,
            obligation.id,
            super::cleanup_obligations::CleanupTerminalSource::IndependentAbsence {
                cleanup_attempt_id: attempt.id,
                absence_check_id: check.id,
            },
        )
        .await?;
    }
    tx.commit().await?;
    Ok(AppliedAbsenceCheck {
        check,
        attempt,
        obligation,
    })
}
