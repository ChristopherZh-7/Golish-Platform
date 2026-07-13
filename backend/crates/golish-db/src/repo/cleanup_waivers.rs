use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::Result;

pub const TABLE_NAME: &str = "cleanup_waivers";

#[derive(Clone, Debug, sqlx::FromRow, PartialEq)]
pub struct CleanupWaiverRow {
    pub id: Uuid,
    pub obligation_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub decided_by_principal_id: Uuid,
    pub reason: String,
    pub residual_risk: Value,
    pub row_version: i64,
    pub decided_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WaiveCleanupObligation {
    pub id: Uuid,
    pub obligation_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub expected_obligation_row_version: i64,
    pub principal_id: Uuid,
    pub reason: String,
    pub residual_risk: Value,
    pub evidence: Vec<(i64, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WaivedCleanupObligation {
    pub waiver: CleanupWaiverRow,
    pub obligation: super::cleanup_obligations::CleanupObligationRow,
}

pub async fn waive(
    pool: &PgPool,
    input: &WaiveCleanupObligation,
) -> Result<WaivedCleanupObligation> {
    if input.reason.trim().is_empty()
        || input.reason.len() > 4096
        || input.reason.chars().any(char::is_control)
        || !input.residual_risk.is_object()
        || input.evidence.is_empty()
        || input.evidence.len() > 1024
        || input.evidence.iter().any(|(id, role)| {
            *id <= 0 || !matches!(role.as_str(), "decision" | "residual" | "support")
        })
    {
        return Err(anyhow::anyhow!("cleanup_waiver_invalid").into());
    }
    let mut expected_evidence = input.evidence.clone();
    expected_evidence.sort_unstable();
    expected_evidence.dedup();
    if expected_evidence.len() != input.evidence.len() {
        return Err(anyhow::anyhow!("cleanup_waiver_evidence_duplicate").into());
    }
    let mut tx = pool.begin().await?;
    let principal_is_active: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM operator_principals
                WHERE id=$1 AND principal_kind='local_operator' AND active
           )"#,
    )
    .bind(input.principal_id)
    .fetch_one(&mut *tx)
    .await?;
    if !principal_is_active {
        return Err(anyhow::anyhow!("cleanup_operator_untrusted").into());
    }
    let current = sqlx::query_as::<_, super::cleanup_obligations::CleanupObligationRow>(
        "SELECT * FROM cleanup_obligations WHERE id=$1 FOR UPDATE",
    )
    .bind(input.obligation_id)
    .fetch_one(&mut *tx)
    .await?;
    if current.operation_id != input.operation_id
        || current.project_scope_id != input.project_scope_id
        || current.scope_snapshot_id != input.scope_snapshot_id
        || current.organization_id_at_time != input.organization_id_at_time
    {
        return Err(anyhow::anyhow!("cleanup_waiver_scope_not_authorized").into());
    }
    if let Some(existing) = sqlx::query_as::<_, CleanupWaiverRow>(
        "SELECT * FROM cleanup_waivers WHERE obligation_id=$1 FOR SHARE",
    )
    .bind(input.obligation_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        let stored_evidence = sqlx::query_as::<_, (i64, String)>(
            r#"SELECT evidence_id,role FROM cleanup_waiver_evidence
                WHERE waiver_id=$1 ORDER BY evidence_id,role"#,
        )
        .bind(existing.id)
        .fetch_all(&mut *tx)
        .await?;
        let mut expected = input.evidence.clone();
        expected.sort_unstable();
        if existing.id != input.id
            || existing.decided_by_principal_id != input.principal_id
            || existing.reason != input.reason
            || existing.residual_risk != input.residual_risk
            || stored_evidence != expected
            || current.status != "waived_by_user"
            || current.residual_risk.as_ref() != Some(&input.residual_risk)
        {
            return Err(anyhow::anyhow!("cleanup_waiver_replay_conflict").into());
        }
        super::cleanup_obligations::append_terminal_event_with_connection(
            &mut tx,
            current.id,
            super::cleanup_obligations::CleanupTerminalSource::OperatorWaiver {
                waiver_id: existing.id,
            },
        )
        .await?;
        tx.commit().await?;
        return Ok(WaivedCleanupObligation {
            waiver: existing,
            obligation: current,
        });
    }
    if !matches!(current.status.as_str(), "open" | "in_progress")
        || current.row_version != input.expected_obligation_row_version
    {
        return Err(anyhow::anyhow!("cleanup_waiver_cas_conflict").into());
    }
    let live_attempt_exists: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM cleanup_attempts
                WHERE obligation_id=$1
                  AND status IN ('claimed','executing','cleaned_pending_verification')
           )"#,
    )
    .bind(current.id)
    .fetch_one(&mut *tx)
    .await?;
    if live_attempt_exists {
        return Err(anyhow::anyhow!("cleanup_waiver_live_attempt_exists").into());
    }
    super::foothold_candidates::validate_evidence_authority(
        &mut tx,
        current.operation_id,
        current.scope_snapshot_id,
        current.organization_id_at_time,
        &input.evidence.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
    )
    .await?;
    let waiver = sqlx::query_as::<_, CleanupWaiverRow>(
        r#"INSERT INTO cleanup_waivers(
               id,obligation_id,operation_id,project_scope_id,scope_snapshot_id,
               organization_id_at_time,decided_by_principal_id,reason,residual_risk
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING *"#,
    )
    .bind(input.id)
    .bind(current.id)
    .bind(current.operation_id)
    .bind(current.project_scope_id)
    .bind(current.scope_snapshot_id)
    .bind(current.organization_id_at_time)
    .bind(input.principal_id)
    .bind(&input.reason)
    .bind(&input.residual_risk)
    .fetch_one(&mut *tx)
    .await?;
    for (evidence_id, role) in &input.evidence {
        sqlx::query(
            "INSERT INTO cleanup_waiver_evidence(waiver_id,evidence_id,role) VALUES($1,$2,$3)",
        )
        .bind(waiver.id)
        .bind(evidence_id)
        .bind(role)
        .execute(&mut *tx)
        .await?;
    }
    let obligation = sqlx::query_as::<_, super::cleanup_obligations::CleanupObligationRow>(
        r#"UPDATE cleanup_obligations
                  SET status='waived_by_user',residual_risk=$2,terminal_at=NOW(),
                      row_version=row_version+1,updated_at=NOW()
                WHERE id=$1 RETURNING *"#,
    )
    .bind(current.id)
    .bind(&input.residual_risk)
    .fetch_one(&mut *tx)
    .await?;
    super::cleanup_obligations::append_terminal_event_with_connection(
        &mut tx,
        obligation.id,
        super::cleanup_obligations::CleanupTerminalSource::OperatorWaiver {
            waiver_id: waiver.id,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(WaivedCleanupObligation { waiver, obligation })
}
