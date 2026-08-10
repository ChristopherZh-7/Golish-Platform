//! Provider-fact observations and the only IntelGoalV1 Target promotion path.

use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct TargetIntelAssetObservationRow {
    pub id: Uuid,
    pub stable_observation_key: String,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub team_plan_id: Uuid,
    pub goal_epoch_id: Uuid,
    pub goal_epoch: i64,
    pub producer_worker_run_id: Uuid,
    pub producer_tool_call_id: Option<Uuid>,
    pub semantic_receipt_audit_id: Option<i64>,
    pub evidence_id: i64,
    pub artifact_ref: String,
    pub artifact_sha256: String,
    pub provider_id: String,
    pub provider_query_type: String,
    pub adapter_version: String,
    pub stable_query_key: String,
    pub provider_record_ordinal: i32,
    pub provider_fetched_at: DateTime<Utc>,
    pub asset_kind: String,
    pub canonical_value: String,
    pub canonical_identity: Value,
    pub canonical_identity_sha256: String,
    pub typed_core: Value,
    pub provider_fields: Value,
    pub provider_metadata: Value,
    pub observation_sha256: String,
    pub attribution_disposition: String,
    pub attribution_method: Option<String>,
    pub attribution_basis: Option<Value>,
    pub attribution_decided_at: Option<DateTime<Utc>>,
    pub reachability_state: String,
    pub reachability_method: Option<String>,
    pub reachability_tool_call_id: Option<Uuid>,
    pub reachability_evidence_id: Option<i64>,
    pub reachability_checked_at: Option<DateTime<Utc>>,
    pub reachability_valid_until: Option<DateTime<Utc>>,
    pub promotion_target_id: Option<Uuid>,
    pub promoted_at: Option<DateTime<Utc>>,
    pub row_version: i64,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct RecordAttribution {
    pub observation_id: Uuid,
    pub expected_row_version: i64,
    pub disposition: String,
    pub method: String,
    pub basis: Value,
    pub evidence_refs: Value,
}

#[derive(Debug, Clone)]
pub struct RecordReachability {
    pub observation_id: Uuid,
    pub expected_row_version: i64,
    pub state: String,
    pub method: String,
    pub tool_call_id: Option<Uuid>,
    pub evidence_id: i64,
    pub checked_at: DateTime<Utc>,
    pub valid_until: Option<DateTime<Utc>>,
}

pub async fn insert(pool: &PgPool, row: &TargetIntelAssetObservationRow) -> Result<bool> {
    validate_observation(row)?;
    let mut tx = pool.begin().await?;
    validate_observation_ownership(&mut tx, row).await?;
    let result = sqlx::query(
        r#"INSERT INTO target_intel_asset_observations(
               id,stable_observation_key,operation_id,organization_id,team_plan_id,
               goal_epoch_id,goal_epoch,producer_worker_run_id,producer_tool_call_id,
               semantic_receipt_audit_id,evidence_id,artifact_ref,artifact_sha256,provider_id,
               provider_query_type,adapter_version,stable_query_key,provider_record_ordinal,
               provider_fetched_at,asset_kind,canonical_value,canonical_identity,
               canonical_identity_sha256,typed_core,provider_fields,provider_metadata,
               observation_sha256,attribution_disposition,attribution_method,attribution_basis,
               attribution_decided_at,reachability_state,reachability_method,
               reachability_tool_call_id,reachability_evidence_id,reachability_checked_at,
               reachability_valid_until,promotion_target_id,promoted_at,row_version,observed_at
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,
               $22,$23,$24,$25,$26,$27,$28,$29,$30,$31,$32,$33,$34,$35,$36,$37,$38,$39,$40,$41
           ) ON CONFLICT(operation_id,organization_id,stable_observation_key) DO NOTHING"#,
    )
    .bind(row.id)
    .bind(&row.stable_observation_key)
    .bind(row.operation_id)
    .bind(row.organization_id)
    .bind(row.team_plan_id)
    .bind(row.goal_epoch_id)
    .bind(row.goal_epoch)
    .bind(row.producer_worker_run_id)
    .bind(row.producer_tool_call_id)
    .bind(row.semantic_receipt_audit_id)
    .bind(row.evidence_id)
    .bind(&row.artifact_ref)
    .bind(&row.artifact_sha256)
    .bind(&row.provider_id)
    .bind(&row.provider_query_type)
    .bind(&row.adapter_version)
    .bind(&row.stable_query_key)
    .bind(row.provider_record_ordinal)
    .bind(row.provider_fetched_at)
    .bind(&row.asset_kind)
    .bind(&row.canonical_value)
    .bind(&row.canonical_identity)
    .bind(&row.canonical_identity_sha256)
    .bind(&row.typed_core)
    .bind(&row.provider_fields)
    .bind(&row.provider_metadata)
    .bind(&row.observation_sha256)
    .bind(&row.attribution_disposition)
    .bind(&row.attribution_method)
    .bind(&row.attribution_basis)
    .bind(row.attribution_decided_at)
    .bind(&row.reachability_state)
    .bind(&row.reachability_method)
    .bind(row.reachability_tool_call_id)
    .bind(row.reachability_evidence_id)
    .bind(row.reachability_checked_at)
    .bind(row.reachability_valid_until)
    .bind(row.promotion_target_id)
    .bind(row.promoted_at)
    .bind(row.row_version)
    .bind(row.observed_at)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(result.rows_affected() == 1)
}

pub async fn record_attribution(pool: &PgPool, input: &RecordAttribution) -> Result<i64> {
    if !matches!(
        input.disposition.as_str(),
        "owned" | "shared" | "third_party" | "ambiguous" | "rejected"
    ) || input.method.trim().is_empty()
        || !input.basis.is_object()
    {
        bail!("TARGET_INTEL_ATTRIBUTION_INPUT_INVALID");
    }
    let mut tx = pool.begin().await?;
    let before = lock_observation(&mut tx, input.observation_id).await?;
    if before.row_version != input.expected_row_version || before.promotion_target_id.is_some() {
        bail!("TARGET_INTEL_ATTRIBUTION_CAS_FAILED");
    }
    let changed: i64 = sqlx::query_scalar(
        r#"UPDATE target_intel_asset_observations
              SET attribution_disposition=$3,attribution_method=$4,attribution_basis=$5,
                  attribution_decided_at=NOW(),row_version=row_version+1,updated_at=NOW()
            WHERE id=$1 AND row_version=$2 RETURNING row_version"#,
    )
    .bind(input.observation_id)
    .bind(input.expected_row_version)
    .bind(&input.disposition)
    .bind(&input.method)
    .bind(&input.basis)
    .fetch_one(&mut *tx)
    .await?;
    append_event(
        &mut tx,
        &before,
        "attribution",
        input.expected_row_version,
        json!({"disposition": input.disposition, "method": input.method, "basis": input.basis}),
        input.evidence_refs.clone(),
        json!([]),
    )
    .await?;
    tx.commit().await?;
    Ok(changed)
}

pub async fn record_reachability(pool: &PgPool, input: &RecordReachability) -> Result<i64> {
    if !matches!(
        input.state.as_str(),
        "reachable" | "unreachable" | "failed" | "blocked"
    ) || !is_authoritative_reachability_method(&input.method)
        || (input.state == "reachable"
            && (input.tool_call_id.is_none()
                || input
                    .valid_until
                    .is_none_or(|until| until <= input.checked_at)))
        || (input.state != "reachable" && input.valid_until.is_some())
    {
        bail!("TARGET_INTEL_REACHABILITY_INPUT_INVALID");
    }
    let mut tx = pool.begin().await?;
    let before = lock_observation(&mut tx, input.observation_id).await?;
    if before.row_version != input.expected_row_version || before.promotion_target_id.is_some() {
        bail!("TARGET_INTEL_REACHABILITY_CAS_FAILED");
    }
    let tool_call_id = input
        .tool_call_id
        .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_REACHABILITY_TOOL_AUTHORITY_MISSING"))?;
    ensure_tool_call_owner(
        &mut tx,
        tool_call_id,
        before.operation_id,
        before.organization_id,
        before.producer_worker_run_id,
    )
    .await?;
    ensure_evidence_owner(
        &mut tx,
        input.evidence_id,
        before.operation_id,
        before.organization_id,
    )
    .await?;
    let changed: i64 = sqlx::query_scalar(
        r#"UPDATE target_intel_asset_observations
              SET reachability_state=$3,reachability_method=$4,reachability_tool_call_id=$5,
                  reachability_evidence_id=$6,reachability_checked_at=$7,
                  reachability_valid_until=$8,row_version=row_version+1,updated_at=NOW()
            WHERE id=$1 AND row_version=$2 RETURNING row_version"#,
    )
    .bind(input.observation_id)
    .bind(input.expected_row_version)
    .bind(&input.state)
    .bind(&input.method)
    .bind(input.tool_call_id)
    .bind(input.evidence_id)
    .bind(input.checked_at)
    .bind(input.valid_until)
    .fetch_one(&mut *tx)
    .await?;
    append_event(
        &mut tx,
        &before,
        "reachability",
        input.expected_row_version,
        json!({"state": input.state, "method": input.method, "checked_at": input.checked_at,
               "valid_until": input.valid_until}),
        json!([format!("audit:{}", input.evidence_id)]),
        input
            .tool_call_id
            .map_or_else(|| json!([]), |id| json!([id])),
    )
    .await?;
    tx.commit().await?;
    Ok(changed)
}

/// The only IntelGoalV1 path that may create an in-scope Target.
pub async fn promote_owned_reachable(
    pool: &PgPool,
    observation_id: Uuid,
    expected_row_version: i64,
) -> Result<Uuid> {
    let mut tx = pool.begin().await?;
    let before = lock_observation(&mut tx, observation_id).await?;
    if before.row_version != expected_row_version
        || before.attribution_disposition != "owned"
        || before.reachability_state != "reachable"
        || before.reachability_tool_call_id.is_none()
        || before.reachability_evidence_id.is_none()
        || before.reachability_checked_at.is_none()
        || before
            .reachability_valid_until
            .is_none_or(|valid_until| valid_until <= Utc::now())
        || before
            .reachability_method
            .as_deref()
            .is_none_or(|method| !is_authoritative_reachability_method(method))
    {
        bail!("TARGET_INTEL_PROMOTION_AUTHORITY_INVALID");
    }
    if let Some(target_id) = before.promotion_target_id {
        tx.commit().await?;
        return Ok(target_id);
    }
    let target_type = target_type_for(&before.asset_kind)?;
    let project_path: String =
        sqlx::query_scalar("SELECT project_path FROM organizations WHERE id=$1 FOR SHARE")
            .bind(before.organization_id)
            .fetch_one(&mut *tx)
            .await?;
    let target_id = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT id FROM targets WHERE organization_id=$1 AND project_path=$2
              AND target_type::text=$3 AND value=$4 ORDER BY created_at LIMIT 1 FOR UPDATE"#,
    )
    .bind(before.organization_id)
    .bind(&project_path)
    .bind(target_type)
    .bind(&before.canonical_value)
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or(Uuid::nil());
    let target_id = if target_id.is_nil() {
        sqlx::query_scalar(
            r#"INSERT INTO targets(name,target_type,value,tags,notes,scope,grp,owner,
                   organization_id,project_path,source,parent_id,liveness_state,liveness_checked_at)
               VALUES($1,$2::target_type,$1,'[]','', 'in'::scope_type,'default','',$3,$4,
                      'target_intel_goal',NULL,'alive',$5) RETURNING id"#,
        )
        .bind(&before.canonical_value)
        .bind(target_type)
        .bind(before.organization_id)
        .bind(&project_path)
        .bind(before.reachability_checked_at)
        .fetch_one(&mut *tx)
        .await?
    } else {
        sqlx::query(
            r#"UPDATE targets SET scope='in'::scope_type,source='target_intel_goal',
                   liveness_state='alive',liveness_checked_at=$2,updated_at=NOW()
                 WHERE id=$1 AND organization_id=$3 AND project_path=$4"#,
        )
        .bind(target_id)
        .bind(before.reachability_checked_at)
        .bind(before.organization_id)
        .bind(&project_path)
        .execute(&mut *tx)
        .await?;
        target_id
    };
    let promoted_at: DateTime<Utc> = sqlx::query_scalar(
        r#"UPDATE target_intel_asset_observations
              SET promotion_target_id=$3,promoted_at=NOW(),row_version=row_version+1,updated_at=NOW()
            WHERE id=$1 AND row_version=$2 RETURNING promoted_at"#,
    ).bind(observation_id).bind(expected_row_version).bind(target_id)
     .fetch_one(&mut *tx).await?;
    append_event(
        &mut tx,
        &before,
        "promotion",
        expected_row_version,
        json!({"target_id": target_id, "promoted_at": promoted_at}),
        json!([format!(
            "audit:{}",
            before
                .reachability_evidence_id
                .unwrap_or(before.evidence_id)
        )]),
        json!([]),
    )
    .await?;
    tx.commit().await?;
    Ok(target_id)
}

async fn lock_observation(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> Result<TargetIntelAssetObservationRow> {
    sqlx::query_as::<_, TargetIntelAssetObservationRow>(
        r#"SELECT id,stable_observation_key,operation_id,organization_id,team_plan_id,
                  goal_epoch_id,goal_epoch,producer_worker_run_id,producer_tool_call_id,
                  semantic_receipt_audit_id,evidence_id,artifact_ref,artifact_sha256,provider_id,
                  provider_query_type,adapter_version,stable_query_key,provider_record_ordinal,
                  provider_fetched_at,asset_kind,canonical_value,canonical_identity,
                  canonical_identity_sha256,typed_core,provider_fields,provider_metadata,
                  observation_sha256,attribution_disposition,attribution_method,attribution_basis,
                  attribution_decided_at,reachability_state,reachability_method,
                  reachability_tool_call_id,reachability_evidence_id,reachability_checked_at,
                  reachability_valid_until,promotion_target_id,promoted_at,row_version,observed_at
             FROM target_intel_asset_observations WHERE id=$1 FOR UPDATE"#,
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await
    .map_err(Into::into)
}

async fn append_event(
    tx: &mut Transaction<'_, Postgres>,
    before: &TargetIntelAssetObservationRow,
    event_kind: &str,
    expected_row_version: i64,
    after_state: Value,
    evidence_refs: Value,
    tool_call_refs: Value,
) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO target_intel_asset_observation_events(
               id,observation_id,operation_id,organization_id,event_kind,expected_row_version,
               before_state,after_state,evidence_refs,tool_call_refs)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)"#,
    )
    .bind(Uuid::new_v4())
    .bind(before.id)
    .bind(before.operation_id)
    .bind(before.organization_id)
    .bind(event_kind)
    .bind(expected_row_version)
    .bind(json!({"attribution": before.attribution_disposition,
                  "reachability": before.reachability_state,
                  "promotion_target_id": before.promotion_target_id,
                  "row_version": before.row_version}))
    .bind(after_state)
    .bind(evidence_refs)
    .bind(tool_call_refs)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn target_type_for(asset_kind: &str) -> Result<&'static str> {
    match asset_kind {
        "domain" | "hostname" | "email_domain" => Ok("domain"),
        "ip" => Ok("ip"),
        "cidr" => Ok("cidr"),
        "web_origin" | "network_endpoint" => Ok("url"),
        _ => bail!("TARGET_INTEL_ASSET_KIND_NOT_PROMOTABLE"),
    }
}

fn is_authoritative_reachability_method(method: &str) -> bool {
    matches!(
        method,
        "bounded_http_probe_v1" | "bounded_tcp_protocol_probe_v1"
    )
}

fn validate_observation(row: &TargetIntelAssetObservationRow) -> Result<()> {
    if row.id.is_nil()
        || row.operation_id.is_nil()
        || row.organization_id.is_nil()
        || row.team_plan_id.is_nil()
        || row.goal_epoch_id.is_nil()
        || row.producer_worker_run_id.is_nil()
        || row.evidence_id <= 0
        || row.stable_observation_key.trim().is_empty()
        || !row.artifact_ref.starts_with("intel-artifact:sha256:")
        || !row.observation_sha256.starts_with("sha256:")
    {
        bail!("TARGET_INTEL_OBSERVATION_INPUT_INVALID");
    }
    Ok(())
}

async fn validate_observation_ownership(
    tx: &mut Transaction<'_, Postgres>,
    row: &TargetIntelAssetObservationRow,
) -> Result<()> {
    let worker_owned: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1
                 FROM stage_worker_runs worker
                 JOIN stage_work_items item ON item.id=worker.work_item_id
                WHERE worker.id=$1 AND worker.operation_id=$2
                  AND worker.organization_id=$3 AND item.team_plan_id=$4
           )"#,
    )
    .bind(row.producer_worker_run_id)
    .bind(row.operation_id)
    .bind(row.organization_id)
    .bind(row.team_plan_id)
    .fetch_one(&mut **tx)
    .await?;
    if !worker_owned {
        bail!("TARGET_INTEL_OBSERVATION_WORKER_OWNER_MISMATCH");
    }
    if let Some(tool_call_id) = row.producer_tool_call_id {
        ensure_tool_call_owner(
            tx,
            tool_call_id,
            row.operation_id,
            row.organization_id,
            row.producer_worker_run_id,
        )
        .await?;
    }
    ensure_evidence_owner(tx, row.evidence_id, row.operation_id, row.organization_id).await?;
    if let Some(receipt_id) = row.semantic_receipt_audit_id {
        let receipt_owned: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1 FROM audit_log
                    WHERE id=$1
                      AND detail->>'operation_id'=$2
                      AND detail->>'organization_id'=$3
               )"#,
        )
        .bind(receipt_id)
        .bind(row.operation_id.to_string())
        .bind(row.organization_id.to_string())
        .fetch_one(&mut **tx)
        .await?;
        if !receipt_owned {
            bail!("TARGET_INTEL_OBSERVATION_RECEIPT_OWNER_MISMATCH");
        }
    }
    Ok(())
}

async fn ensure_tool_call_owner(
    tx: &mut Transaction<'_, Postgres>,
    tool_call_id: Uuid,
    operation_id: Uuid,
    organization_id: Uuid,
    worker_run_id: Uuid,
) -> Result<()> {
    let owned: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM tool_calls
                WHERE id=$1 AND operation_id=$2 AND organization_id=$3
                  AND worker_run_id=$4
                  AND status IN ('received','running','finished')
           )"#,
    )
    .bind(tool_call_id)
    .bind(operation_id)
    .bind(organization_id)
    .bind(worker_run_id)
    .fetch_one(&mut **tx)
    .await?;
    if !owned {
        bail!("TARGET_INTEL_TOOL_CALL_OWNER_MISMATCH");
    }
    Ok(())
}

async fn ensure_evidence_owner(
    tx: &mut Transaction<'_, Postgres>,
    evidence_id: i64,
    operation_id: Uuid,
    organization_id: Uuid,
) -> Result<()> {
    // Observation and reachability landing happen inside the tool executor,
    // before the outer runtime can transition the durable call to `finished`.
    // Accept only the exact owner tuple while the call is active (or its
    // finished replay); failed calls never retain mutation authority.
    let owned: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM audit_log
                WHERE id=$1 AND audit_role='evidence' AND run_id=$2
                  AND detail->>'organization_id'=$3
           )"#,
    )
    .bind(evidence_id)
    .bind(operation_id)
    .bind(organization_id.to_string())
    .fetch_one(&mut **tx)
    .await?;
    if !owned {
        bail!("TARGET_INTEL_EVIDENCE_OWNER_MISMATCH");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{is_authoritative_reachability_method, target_type_for};

    #[test]
    fn only_reachable_asset_identity_kinds_are_promotable() {
        assert_eq!(target_type_for("hostname").unwrap(), "domain");
        assert_eq!(target_type_for("ip").unwrap(), "ip");
        assert!(target_type_for("certificate").is_err());
        assert!(target_type_for("asn").is_err());
    }

    #[test]
    fn dns_resolution_is_not_authoritative_reachability() {
        assert!(is_authoritative_reachability_method(
            "bounded_http_probe_v1"
        ));
        assert!(is_authoritative_reachability_method(
            "bounded_tcp_protocol_probe_v1"
        ));
        assert!(!is_authoritative_reachability_method("dns"));
        assert!(!is_authoritative_reachability_method("dns_resolution_v1"));
    }
}
