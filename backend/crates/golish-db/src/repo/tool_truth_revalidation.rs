//! Deterministic, bounded Tool Truth revalidation obligations.
//!
//! Consumer reads may only record/deduplicate obligations. Provider I/O is
//! owned by the background orchestrator, which must claim through the held
//! dispatch head and an expiring CAS lease before it can execute anything.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{DbError, Result};

const CONTRACT_INVALID: &str = "TOOL_TRUTH_REVALIDATION_CONTRACT_INVALID";
const AUTHORITY_STALE: &str = "TOOL_TRUTH_REVALIDATION_AUTHORITY_STALE";
const DISPATCH_HELD: &str = "TOOL_TRUTH_REVALIDATION_DISPATCH_HELD";
const POLICY_BLOCKED: &str = "TOOL_TRUTH_REVALIDATION_POLICY_BLOCKED";
const CLAIM_STALE: &str = "TOOL_TRUTH_REVALIDATION_CLAIM_STALE";
const REPLACEMENT_INVALID: &str = "TOOL_TRUTH_REVALIDATION_REPLACEMENT_INVALID";

fn fail(code: &'static str) -> DbError {
    DbError::Other(anyhow::anyhow!(code))
}

fn sha256_json(value: &serde_json::Value) -> Result<String> {
    let bytes = serde_json::to_vec(value).map_err(|error| DbError::Other(error.into()))?;
    let digest = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("sha256:{digest}"))
}

#[derive(Debug, Clone)]
pub struct RecordRevalidationObligation {
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub source_receipt_id: Uuid,
    pub source_receipt_input_id: Uuid,
    pub source_input_key: String,
    pub fact_class: String,
    pub temporal_policy_id: Uuid,
    pub reason_code: String,
    pub risk_tier: String,
    pub mandatory_axis: bool,
    pub consumer_kind: String,
    pub consumer_key: String,
}

#[derive(Debug, Clone)]
pub struct ClaimRevalidationObligation {
    pub operation_id: Uuid,
    pub owner: String,
    pub expected_dispatch_generation: i64,
    pub expected_head_row_version: i64,
}

#[derive(Debug, Clone)]
pub struct CompleteRevalidationObligation {
    pub obligation_id: Uuid,
    pub owner: String,
    pub claim_token: Uuid,
    pub expected_row_version: i64,
    pub replacement_denominator_id: Uuid,
    pub replacement_receipt_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct FailRevalidationObligation {
    pub obligation_id: Uuid,
    pub owner: String,
    pub claim_token: Uuid,
    pub expected_row_version: i64,
    pub progress_fingerprint: String,
    pub reason_code: String,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize, PartialEq)]
pub struct RevalidationObligationRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub source_receipt_id: Uuid,
    pub source_receipt_input_id: Uuid,
    pub source_input_key: String,
    pub fact_class: String,
    pub temporal_policy_id: Uuid,
    pub reason_code: String,
    pub risk_tier: String,
    pub mandatory_axis: bool,
    pub obligation_hash: String,
    pub status: String,
    pub attempt_count: i32,
    pub retry_count: i32,
    pub no_progress_count: i32,
    pub last_progress_fingerprint: Option<String>,
    pub claim_owner: Option<String>,
    pub claim_token: Option<Uuid>,
    pub claim_expires_at: Option<DateTime<Utc>>,
    pub claimed_dispatch_generation: Option<i64>,
    pub deadline_at: DateTime<Utc>,
    pub replacement_denominator_id: Option<Uuid>,
    pub replacement_receipt_id: Option<Uuid>,
    pub residual: Option<serde_json::Value>,
    pub row_version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const OBLIGATION_COLUMNS: &str = r#"id,operation_id,organization_id,source_receipt_id,
    source_receipt_input_id,source_input_key,fact_class,temporal_policy_id,reason_code,
    risk_tier,mandatory_axis,obligation_hash,status,attempt_count,retry_count,
    no_progress_count,last_progress_fingerprint,claim_owner,claim_token,claim_expires_at,
    claimed_dispatch_generation,deadline_at,replacement_denominator_id,replacement_receipt_id,
    residual,row_version,created_at,updated_at"#;

async fn append_event(
    tx: &mut Transaction<'_, Postgres>,
    obligation: &RevalidationObligationRow,
    event_type: &str,
    claim_token: Option<Uuid>,
    progress_fingerprint: Option<&str>,
    residual: Option<&serde_json::Value>,
) -> Result<()> {
    let ordinal: i32 = sqlx::query_scalar(
        "SELECT count(*)::int FROM tool_truth_revalidation_events WHERE obligation_id=$1",
    )
    .bind(obligation.id)
    .fetch_one(&mut **tx)
    .await?;
    let payload = serde_json::json!({
        "obligation_id": obligation.id,
        "operation_id": obligation.operation_id,
        "event_ordinal": ordinal,
        "event_type": event_type,
        "claim_token": claim_token,
        "progress_fingerprint": progress_fingerprint,
        "replacement_denominator_id": obligation.replacement_denominator_id,
        "replacement_receipt_id": obligation.replacement_receipt_id,
        "residual": residual,
    });
    let event_hash = sha256_json(&payload)?;
    let event_id = Uuid::new_v5(
        &obligation.id,
        format!("event:{ordinal}:{event_hash}").as_bytes(),
    );
    sqlx::query(
        r#"INSERT INTO tool_truth_revalidation_events(
               id,obligation_id,operation_id,event_ordinal,event_type,claim_token,
               progress_fingerprint,replacement_denominator_id,replacement_receipt_id,
               residual,event_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
    )
    .bind(event_id)
    .bind(obligation.id)
    .bind(obligation.operation_id)
    .bind(ordinal)
    .bind(event_type)
    .bind(claim_token)
    .bind(progress_fingerprint)
    .bind(obligation.replacement_denominator_id)
    .bind(obligation.replacement_receipt_id)
    .bind(residual)
    .bind(&event_hash)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO tool_truth_revalidation_outbox(
               id,obligation_id,operation_id,event_ordinal,event_type,payload,payload_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7)"#,
    )
    .bind(Uuid::new_v5(&event_id, b"outbox:v1"))
    .bind(obligation.id)
    .bind(obligation.operation_id)
    .bind(ordinal)
    .bind(event_type)
    .bind(&payload)
    .bind(event_hash)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn record_obligation(
    pool: &PgPool,
    command: &RecordRevalidationObligation,
) -> Result<RevalidationObligationRow> {
    let mut tx = pool.begin().await?;
    let row = record_obligation_on(&mut tx, command).await?;
    tx.commit().await?;
    Ok(row)
}

pub(crate) async fn record_obligation_on(
    tx: &mut Transaction<'_, Postgres>,
    command: &RecordRevalidationObligation,
) -> Result<RevalidationObligationRow> {
    if command.operation_id.is_nil()
        || command.organization_id.is_nil()
        || command.source_receipt_id.is_nil()
        || command.source_receipt_input_id.is_nil()
        || command.source_input_key.trim().is_empty()
        || command.fact_class.trim().is_empty()
        || command.reason_code.trim().is_empty()
        || command.consumer_key.trim().is_empty()
        || !matches!(command.risk_tier.as_str(), "T0" | "T1" | "T2" | "T3")
        || !matches!(
            command.consumer_kind.as_str(),
            "candidate" | "campaign" | "reporting" | "ui" | "report_download"
        )
    {
        return Err(fail(CONTRACT_INVALID));
    }
    let deadline_seconds: i32 = sqlx::query_scalar(
        "SELECT deadline_seconds FROM tool_truth_revalidation_dispatch_policies WHERE operation_id=$1 FOR SHARE",
    )
    .bind(command.operation_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| fail(AUTHORITY_STALE))?;
    let source_exact: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
              SELECT 1
                FROM capability_execution_receipts r
                JOIN coverage_denominators d ON d.id=r.denominator_id
                JOIN capability_execution_receipt_inputs i
                  ON i.id=$4 AND i.receipt_id=r.id
                 AND i.input_key=$5 AND i.sealed_at IS NOT NULL
               WHERE r.id=$3 AND d.operation_id=$1 AND d.organization_id=$2
                 AND r.temporal_validity_policy_id=$6 AND r.finalized_at IS NOT NULL
           )"#,
    )
    .bind(command.operation_id)
    .bind(command.organization_id)
    .bind(command.source_receipt_id)
    .bind(command.source_receipt_input_id)
    .bind(&command.source_input_key)
    .bind(command.temporal_policy_id)
    .fetch_one(&mut **tx)
    .await?;
    if !source_exact {
        return Err(fail(AUTHORITY_STALE));
    }
    let obligation_hash = sha256_json(&serde_json::json!({
        "operation_id": command.operation_id,
        "organization_id": command.organization_id,
        "source_receipt_id": command.source_receipt_id,
        "source_receipt_input_id": command.source_receipt_input_id,
        "source_input_key": command.source_input_key,
        "fact_class": command.fact_class,
        "temporal_policy_id": command.temporal_policy_id,
        "reason_code": command.reason_code,
        "risk_tier": command.risk_tier,
        "mandatory_axis": command.mandatory_axis,
    }))?;
    let obligation_id = Uuid::new_v5(&command.operation_id, obligation_hash.as_bytes());
    let existing = sqlx::query_as::<_, RevalidationObligationRow>(&format!(
        "SELECT {OBLIGATION_COLUMNS} FROM tool_truth_revalidation_obligations WHERE operation_id=$1 AND organization_id=$2 AND source_receipt_id=$3 AND source_receipt_input_id=$4 AND source_input_key=$5 AND fact_class=$6 AND temporal_policy_id=$7 AND reason_code=$8 FOR SHARE"
    ))
    .bind(command.operation_id)
    .bind(command.organization_id)
    .bind(command.source_receipt_id)
    .bind(command.source_receipt_input_id)
    .bind(&command.source_input_key)
    .bind(&command.fact_class)
    .bind(command.temporal_policy_id)
    .bind(&command.reason_code)
    .fetch_optional(&mut **tx)
    .await?;
    let row = if let Some(existing) = existing {
        if existing.id != obligation_id || existing.obligation_hash != obligation_hash {
            return Err(fail(AUTHORITY_STALE));
        }
        existing
    } else {
        let row = sqlx::query_as::<_, RevalidationObligationRow>(&format!(
            r#"INSERT INTO tool_truth_revalidation_obligations(
                   id,operation_id,organization_id,source_receipt_id,source_receipt_input_id,
                   source_input_key,fact_class,temporal_policy_id,reason_code,risk_tier,
                   mandatory_axis,obligation_hash,deadline_at
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,
                        statement_timestamp()+make_interval(secs=>$13))
               RETURNING {OBLIGATION_COLUMNS}"#
        ))
        .bind(obligation_id)
        .bind(command.operation_id)
        .bind(command.organization_id)
        .bind(command.source_receipt_id)
        .bind(command.source_receipt_input_id)
        .bind(&command.source_input_key)
        .bind(&command.fact_class)
        .bind(command.temporal_policy_id)
        .bind(&command.reason_code)
        .bind(&command.risk_tier)
        .bind(command.mandatory_axis)
        .bind(&obligation_hash)
        .bind(deadline_seconds)
        .fetch_one(&mut **tx)
        .await?;
        append_event(tx, &row, "opened", None, None, None).await?;
        row
    };
    let consumer_hash = sha256_json(&serde_json::json!({
        "obligation_id": row.id,
        "consumer_kind": command.consumer_kind,
        "consumer_key": command.consumer_key,
    }))?;
    sqlx::query(
        r#"INSERT INTO tool_truth_revalidation_consumers(
               id,obligation_id,operation_id,consumer_kind,consumer_key,consumer_hash
           ) VALUES($1,$2,$3,$4,$5,$6)
           ON CONFLICT(obligation_id,consumer_kind,consumer_key) DO NOTHING"#,
    )
    .bind(Uuid::new_v5(&row.id, consumer_hash.as_bytes()))
    .bind(row.id)
    .bind(row.operation_id)
    .bind(&command.consumer_kind)
    .bind(&command.consumer_key)
    .bind(consumer_hash)
    .execute(&mut **tx)
    .await?;
    Ok(row)
}

/// Record exact stale TargetIntel inputs for a consumer. This function never
/// claims or executes work; repeated consumers converge on the same open rows.
pub async fn record_expired_target_intel_obligations(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
    consumer_kind: &str,
    consumer_key: &str,
) -> Result<Vec<RevalidationObligationRow>> {
    let stage_ids = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT id FROM stage_runs
            WHERE operation_id=$1 AND stage_kind='target_intel' AND status='started'
            ORDER BY started_at,id"#,
    )
    .bind(operation_id)
    .fetch_all(pool)
    .await?;
    let stage_id = match stage_ids.as_slice() {
        [] => return Ok(Vec::new()),
        [stage_id] => *stage_id,
        _ => return Err(fail(AUTHORITY_STALE)),
    };
    let stale = sqlx::query_as::<_, (Uuid, Uuid, String, String, Uuid)>(
        r#"SELECT r.id,i.id,i.input_key,d.technique,r.temporal_validity_policy_id
             FROM coverage_denominators denominator
             JOIN capability_execution_receipts r ON r.denominator_id=denominator.id
             JOIN capability_execution_receipt_inputs i ON i.receipt_id=r.id
             JOIN coverage_denominator_items d ON d.id=i.denominator_item_id
            WHERE denominator.operation_id=$1 AND denominator.organization_id=$2
              AND denominator.stage_execution_id=$3 AND denominator.stage_kind='target_intel'
              AND denominator.denominator_kind='root' AND denominator.sealed_at IS NOT NULL
              AND r.attempt_ordinal=(
                  SELECT max(current.attempt_ordinal)
                    FROM capability_execution_receipts current
                   WHERE current.denominator_id=denominator.id
              )
              AND r.reconciliation_state='consistent' AND r.coverage_extent='complete'
              AND r.finalized_at IS NOT NULL AND r.valid_until<=statement_timestamp()
              AND i.sealed_at IS NOT NULL
            ORDER BY i.input_key,r.id"#,
    )
    .bind(operation_id)
    .bind(organization_id)
    .bind(stage_id)
    .fetch_all(pool)
    .await?;
    let mut obligations = Vec::with_capacity(stale.len());
    for (receipt_id, input_id, input_key, technique, temporal_policy_id) in stale {
        obligations.push(
            record_obligation(
                pool,
                &RecordRevalidationObligation {
                    operation_id,
                    organization_id,
                    source_receipt_id: receipt_id,
                    source_receipt_input_id: input_id,
                    source_input_key: input_key,
                    fact_class: technique,
                    temporal_policy_id,
                    reason_code: "temporal_expired".to_string(),
                    risk_tier: "T1".to_string(),
                    mandatory_axis: true,
                    consumer_kind: consumer_kind.to_string(),
                    consumer_key: consumer_key.to_string(),
                },
            )
            .await?,
        );
    }
    Ok(obligations)
}

pub async fn claim_next(
    pool: &PgPool,
    command: &ClaimRevalidationObligation,
) -> Result<Option<RevalidationObligationRow>> {
    if command.operation_id.is_nil()
        || command.owner.trim().is_empty()
        || command.expected_dispatch_generation < 0
        || command.expected_head_row_version < 0
    {
        return Err(fail(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    let head = sqlx::query_as::<_, (String, i64, i64)>(
        r#"SELECT dispatch_state,generation,row_version
             FROM tool_truth_revalidation_dispatch_heads
            WHERE operation_id=$1 FOR SHARE"#,
    )
    .bind(command.operation_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| fail(AUTHORITY_STALE))?;
    if head.0 != "released"
        || head.1 != command.expected_dispatch_generation
        || head.2 != command.expected_head_row_version
    {
        return Err(fail(DISPATCH_HELD));
    }
    let policy = sqlx::query_as::<_, (String, String, i32, i32, i32, i32)>(
        r#"SELECT dispatch_mode,max_risk_tier,max_attempts,max_retries,
                  max_no_progress,lease_seconds
             FROM tool_truth_revalidation_dispatch_policies
            WHERE operation_id=$1 FOR SHARE"#,
    )
    .bind(command.operation_id)
    .fetch_one(&mut *tx)
    .await?;
    if policy.0 != "auto_passive_t0_t1" {
        return Err(fail(POLICY_BLOCKED));
    }
    let active: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
              SELECT 1 FROM operation_state o
              JOIN stage_runs s ON s.operation_id=o.operation_id AND s.status='started'
               WHERE o.operation_id=$1 AND o.superseded_by IS NULL
           )"#,
    )
    .bind(command.operation_id)
    .fetch_one(&mut *tx)
    .await?;
    if !active {
        return Err(fail(POLICY_BLOCKED));
    }
    let Some(candidate) = sqlx::query_as::<_, RevalidationObligationRow>(&format!(
        r#"SELECT {OBLIGATION_COLUMNS}
             FROM tool_truth_revalidation_obligations
            WHERE operation_id=$1
              AND (status='open' OR (status='claimed' AND claim_expires_at<=statement_timestamp()))
              AND deadline_at>statement_timestamp()
              AND attempt_count<$2 AND retry_count<=$3 AND no_progress_count<$4
              AND risk_tier IN ('T0','T1')
              AND CASE $5 WHEN 'T0' THEN risk_tier='T0' ELSE TRUE END
            ORDER BY created_at,id FOR UPDATE SKIP LOCKED LIMIT 1"#
    ))
    .bind(command.operation_id)
    .bind(policy.2)
    .bind(policy.3)
    .bind(policy.4)
    .bind(&policy.1)
    .fetch_optional(&mut *tx)
    .await?
    else {
        tx.commit().await?;
        return Ok(None);
    };
    let reclaimed = candidate.status == "claimed";
    let claim_token = Uuid::new_v4();
    let row = sqlx::query_as::<_, RevalidationObligationRow>(&format!(
        r#"UPDATE tool_truth_revalidation_obligations
              SET status='claimed',attempt_count=attempt_count+1,claim_owner=$2,
                  claim_token=$3,claim_expires_at=statement_timestamp()+make_interval(secs=>$4),
                  claimed_dispatch_generation=$5,row_version=row_version+1,
                  updated_at=statement_timestamp()
            WHERE id=$1 AND row_version=$6
            RETURNING {OBLIGATION_COLUMNS}"#
    ))
    .bind(candidate.id)
    .bind(&command.owner)
    .bind(claim_token)
    .bind(policy.5)
    .bind(head.1)
    .bind(candidate.row_version)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| fail(CLAIM_STALE))?;
    append_event(
        &mut tx,
        &row,
        if reclaimed { "reclaimed" } else { "claimed" },
        Some(claim_token),
        None,
        None,
    )
    .await?;
    tx.commit().await?;
    Ok(Some(row))
}

pub async fn complete_success(
    pool: &PgPool,
    command: &CompleteRevalidationObligation,
) -> Result<RevalidationObligationRow> {
    let mut tx = pool.begin().await?;
    let current = sqlx::query_as::<_, RevalidationObligationRow>(&format!(
        "SELECT {OBLIGATION_COLUMNS} FROM tool_truth_revalidation_obligations WHERE id=$1 FOR UPDATE"
    ))
    .bind(command.obligation_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| fail(AUTHORITY_STALE))?;
    if current.status != "claimed"
        || current.claim_owner.as_deref() != Some(command.owner.as_str())
        || current.claim_token != Some(command.claim_token)
        || current.row_version != command.expected_row_version
        || current
            .claim_expires_at
            .is_none_or(|expiry| expiry <= Utc::now())
        || command.replacement_receipt_id == current.source_receipt_id
    {
        return Err(fail(CLAIM_STALE));
    }
    let replacement_exact: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
              SELECT 1
                FROM capability_execution_receipts replacement
                JOIN coverage_denominators rd ON rd.id=replacement.denominator_id
                JOIN capability_execution_receipt_inputs ri
                  ON ri.receipt_id=replacement.id AND ri.sealed_at IS NOT NULL
                JOIN coverage_denominator_items rdi ON rdi.id=ri.denominator_item_id
                JOIN capability_execution_receipts source ON source.id=$3
                JOIN capability_execution_receipt_inputs si ON si.id=$4
                JOIN coverage_denominator_items sdi ON sdi.id=si.denominator_item_id
               WHERE replacement.id=$1 AND replacement.denominator_id=$2
                 AND rd.operation_id=$5 AND rd.organization_id=$6
                 AND replacement.finalized_at IS NOT NULL
                 AND replacement.reconciliation_state='consistent'
                 AND replacement.coverage_extent='complete'
                 AND replacement.denominator_id<>source.denominator_id
                 AND replacement.attempt_ordinal>source.attempt_ordinal
                 AND rdi.exact_asset=sdi.exact_asset AND rdi.technique=sdi.technique
           )"#,
    )
    .bind(command.replacement_receipt_id)
    .bind(command.replacement_denominator_id)
    .bind(current.source_receipt_id)
    .bind(current.source_receipt_input_id)
    .bind(current.operation_id)
    .bind(current.organization_id)
    .fetch_one(&mut *tx)
    .await?;
    if !replacement_exact {
        return Err(fail(REPLACEMENT_INVALID));
    }
    let row = sqlx::query_as::<_, RevalidationObligationRow>(&format!(
        r#"UPDATE tool_truth_revalidation_obligations
              SET status='succeeded',claim_owner=NULL,claim_token=NULL,claim_expires_at=NULL,
                  claimed_dispatch_generation=NULL,replacement_denominator_id=$2,
                  replacement_receipt_id=$3,residual=NULL,row_version=row_version+1,
                  updated_at=statement_timestamp()
            WHERE id=$1 AND row_version=$4 RETURNING {OBLIGATION_COLUMNS}"#
    ))
    .bind(current.id)
    .bind(command.replacement_denominator_id)
    .bind(command.replacement_receipt_id)
    .bind(current.row_version)
    .fetch_one(&mut *tx)
    .await?;
    append_event(
        &mut tx,
        &row,
        "succeeded",
        Some(command.claim_token),
        None,
        None,
    )
    .await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn record_failure(
    pool: &PgPool,
    command: &FailRevalidationObligation,
) -> Result<RevalidationObligationRow> {
    if command.progress_fingerprint.trim().is_empty() || command.reason_code.trim().is_empty() {
        return Err(fail(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    let current = sqlx::query_as::<_, RevalidationObligationRow>(&format!(
        "SELECT {OBLIGATION_COLUMNS} FROM tool_truth_revalidation_obligations WHERE id=$1 FOR UPDATE"
    ))
    .bind(command.obligation_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| fail(AUTHORITY_STALE))?;
    if current.status != "claimed"
        || current.claim_owner.as_deref() != Some(command.owner.as_str())
        || current.claim_token != Some(command.claim_token)
        || current.row_version != command.expected_row_version
    {
        return Err(fail(CLAIM_STALE));
    }
    let (max_attempts, max_retries, max_no_progress): (i32, i32, i32) = sqlx::query_as(
        r#"SELECT max_attempts,max_retries,max_no_progress
             FROM tool_truth_revalidation_dispatch_policies WHERE operation_id=$1 FOR SHARE"#,
    )
    .bind(current.operation_id)
    .fetch_one(&mut *tx)
    .await?;
    let no_progress = if current.last_progress_fingerprint.as_deref()
        == Some(command.progress_fingerprint.as_str())
    {
        current.no_progress_count.saturating_add(1)
    } else {
        0
    };
    let retry_count = current.retry_count.saturating_add(1);
    let exhausted = current.attempt_count >= max_attempts
        || retry_count > max_retries
        || no_progress >= max_no_progress
        || current.deadline_at <= Utc::now();
    let residual = exhausted.then(|| {
        serde_json::json!({
            "code": "TOOL_TRUTH_REVALIDATION_EXHAUSTED",
            "reason_code": command.reason_code,
            "progress_fingerprint": command.progress_fingerprint,
            "attempt_count": current.attempt_count,
            "retry_count": retry_count,
            "no_progress_count": no_progress,
            "mandatory_axis": current.mandatory_axis,
        })
    });
    let row = sqlx::query_as::<_, RevalidationObligationRow>(&format!(
        r#"UPDATE tool_truth_revalidation_obligations
              SET status=$2,retry_count=$3,no_progress_count=$4,
                  last_progress_fingerprint=$5,claim_owner=NULL,claim_token=NULL,
                  claim_expires_at=NULL,claimed_dispatch_generation=NULL,residual=$6,
                  row_version=row_version+1,updated_at=statement_timestamp()
            WHERE id=$1 AND row_version=$7 RETURNING {OBLIGATION_COLUMNS}"#
    ))
    .bind(current.id)
    .bind(if exhausted { "exhausted" } else { "open" })
    .bind(retry_count)
    .bind(no_progress)
    .bind(&command.progress_fingerprint)
    .bind(&residual)
    .bind(current.row_version)
    .fetch_one(&mut *tx)
    .await?;
    append_event(
        &mut tx,
        &row,
        if exhausted { "exhausted" } else { "failed" },
        Some(command.claim_token),
        Some(&command.progress_fingerprint),
        residual.as_ref(),
    )
    .await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<RevalidationObligationRow>> {
    sqlx::query_as::<_, RevalidationObligationRow>(&format!(
        "SELECT {OBLIGATION_COLUMNS} FROM tool_truth_revalidation_obligations WHERE id=$1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}
