use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::Result;

pub const TABLE_NAME: &str = "cleanup_attempts";

#[derive(Clone, Debug, sqlx::FromRow, PartialEq)]
pub struct CleanupAttemptRow {
    pub id: Uuid,
    pub obligation_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub ordinal: i32,
    pub status: String,
    pub lease_token: Uuid,
    pub lease_expires_at: DateTime<Utc>,
    pub worker_run_id: Option<Uuid>,
    pub result: Option<Value>,
    pub terminal_note: Option<String>,
    pub row_version: i64,
    pub claimed_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimCleanupAttempt {
    pub obligation_id: Uuid,
    pub lease_token: Uuid,
    pub lease_expires_at: DateTime<Utc>,
    pub worker_run_id: Option<Uuid>,
}

pub async fn claim(pool: &PgPool, input: &ClaimCleanupAttempt) -> Result<CleanupAttemptRow> {
    if input.lease_expires_at <= Utc::now() {
        return Err(anyhow::anyhow!("cleanup_attempt_lease_invalid").into());
    }
    let mut tx = pool.begin().await?;
    let obligation = sqlx::query_as::<_, super::cleanup_obligations::CleanupObligationRow>(
        "SELECT * FROM cleanup_obligations WHERE id=$1 FOR UPDATE",
    )
    .bind(input.obligation_id)
    .fetch_one(&mut *tx)
    .await?;
    if !matches!(obligation.status.as_str(), "open" | "in_progress") {
        return Err(anyhow::anyhow!("cleanup_obligation_terminal").into());
    }
    if let Some(live) = sqlx::query_as::<_, CleanupAttemptRow>(
        r#"SELECT * FROM cleanup_attempts
            WHERE obligation_id=$1
              AND status IN ('claimed','executing','cleaned_pending_verification')
            FOR UPDATE"#,
    )
    .bind(input.obligation_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        if live.lease_token == input.lease_token
            && live.worker_run_id == input.worker_run_id
            && live.lease_expires_at == input.lease_expires_at
        {
            tx.commit().await?;
            return Ok(live);
        }
        return Err(anyhow::anyhow!("cleanup_live_attempt_exists").into());
    }
    let ordinal: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(ordinal),0)+1 FROM cleanup_attempts WHERE obligation_id=$1",
    )
    .bind(input.obligation_id)
    .fetch_one(&mut *tx)
    .await?;
    let row = sqlx::query_as::<_, CleanupAttemptRow>(
        r#"INSERT INTO cleanup_attempts(
               obligation_id,operation_id,project_scope_id,scope_snapshot_id,
               organization_id_at_time,ordinal,lease_token,lease_expires_at,worker_run_id
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING *"#,
    )
    .bind(input.obligation_id)
    .bind(obligation.operation_id)
    .bind(obligation.project_scope_id)
    .bind(obligation.scope_snapshot_id)
    .bind(obligation.organization_id_at_time)
    .bind(ordinal)
    .bind(input.lease_token)
    .bind(input.lease_expires_at)
    .bind(input.worker_run_id)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        r#"UPDATE cleanup_obligations
              SET status='in_progress',row_version=row_version+1,updated_at=NOW()
            WHERE id=$1 AND status='open'"#,
    )
    .bind(input.obligation_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

#[derive(Clone, Debug, PartialEq)]
pub struct TransitionCleanupAttempt {
    pub attempt_id: Uuid,
    pub lease_token: Uuid,
    pub expected_row_version: i64,
    pub expected_status: String,
    pub next_status: String,
    pub result: Option<Value>,
    pub evidence: Vec<(i64, String)>,
    pub terminal_note: Option<String>,
}

pub async fn transition(
    pool: &PgPool,
    input: &TransitionCleanupAttempt,
) -> Result<CleanupAttemptRow> {
    let allowed = matches!(
        (input.expected_status.as_str(), input.next_status.as_str()),
        ("claimed", "executing")
            | ("executing", "cleaned_pending_verification")
            | ("claimed" | "executing", "execution_failed")
    );
    if !allowed
        || input.evidence.len() > 1024
        || input.evidence.iter().any(|(id, role)| {
            *id <= 0 || !matches!(role.as_str(), "execution" | "result" | "support")
        })
    {
        return Err(anyhow::anyhow!("cleanup_attempt_transition_invalid").into());
    }
    let mut unique = input.evidence.clone();
    unique.sort_unstable();
    unique.dedup();
    if unique.len() != input.evidence.len() {
        return Err(anyhow::anyhow!("cleanup_attempt_evidence_duplicate").into());
    }
    let mut tx = pool.begin().await?;
    let current = sqlx::query_as::<_, CleanupAttemptRow>(
        "SELECT * FROM cleanup_attempts WHERE id=$1 FOR UPDATE",
    )
    .bind(input.attempt_id)
    .fetch_one(&mut *tx)
    .await?;
    if current.lease_token != input.lease_token
        || current.row_version != input.expected_row_version
        || current.status != input.expected_status
    {
        return Err(anyhow::anyhow!("cleanup_attempt_fence_mismatch").into());
    }
    if !input.evidence.is_empty() {
        super::foothold_candidates::validate_evidence_authority(
            &mut tx,
            current.operation_id,
            current.scope_snapshot_id,
            current.organization_id_at_time,
            &input.evidence.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        )
        .await?;
        for (evidence_id, role) in &input.evidence {
            sqlx::query(
                "INSERT INTO cleanup_attempt_evidence(attempt_id,evidence_id,role) VALUES($1,$2,$3)",
            )
            .bind(current.id)
            .bind(evidence_id)
            .bind(role)
            .execute(&mut *tx)
            .await?;
        }
    }
    let terminal = input.next_status == "execution_failed";
    let row = sqlx::query_as::<_, CleanupAttemptRow>(
        r#"UPDATE cleanup_attempts
              SET status=$2,result=$3,terminal_note=$4,row_version=row_version+1,
                  started_at=CASE WHEN $2='executing' THEN COALESCE(started_at,NOW()) ELSE started_at END,
                  completed_at=CASE WHEN $5 THEN NOW() ELSE NULL END
            WHERE id=$1 RETURNING *"#,
    )
    .bind(current.id)
    .bind(&input.next_status)
    .bind(&input.result)
    .bind(&input.terminal_note)
    .bind(terminal)
    .fetch_one(&mut *tx)
    .await?;
    if terminal {
        sqlx::query(
            r#"UPDATE cleanup_obligations
                  SET status='open',row_version=row_version+1,updated_at=NOW()
                WHERE id=$1 AND status='in_progress'"#,
        )
        .bind(row.obligation_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(row)
}

pub async fn list_for_obligation(
    pool: &PgPool,
    obligation_id: Uuid,
) -> Result<Vec<CleanupAttemptRow>> {
    Ok(sqlx::query_as::<_, CleanupAttemptRow>(
        "SELECT * FROM cleanup_attempts WHERE obligation_id=$1 ORDER BY ordinal",
    )
    .bind(obligation_id)
    .fetch_all(pool)
    .await?)
}
