//! Immutable Company Identity receipts frozen by Scoping.

use anyhow::{bail, Result};
use chrono::Utc;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use super::audit;

#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct ScopingCompanyIdentityReceiptRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub resolution_attempt: i64,
    pub supersedes_receipt_id: Option<Uuid>,
    pub organization_id: Option<Uuid>,
    pub subject_hint: String,
    pub canonical_legal_name: Option<String>,
    pub aliases: Value,
    pub brands: Value,
    pub registration_identifiers: Value,
    pub disambiguation_fields: Value,
    pub confirmation_method: String,
    pub resolution_status: String,
    pub scope_policy: Value,
    pub source_receipt_refs: Value,
    pub artifact_refs: Value,
    pub evidence_refs: Value,
    pub identity_payload: Value,
    pub identity_sha256: String,
    pub scope_policy_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedCompanyIdentityIntake {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub organization_id: Uuid,
    pub canonical_legal_name: String,
    pub session_id: Option<String>,
}

/// Freeze an adapter-confirmed root company before Scoping invokes the model.
///
/// This path is deliberately narrower than reusing an arbitrary organization
/// row: the operation and exact active Scoping execution must already exist,
/// the organization must be the root row in the operation's canonical project,
/// and an evidence-ledger row is committed with the immutable receipt. Replays
/// return the one existing confirmed receipt; conflicting authority fails.
pub async fn freeze_trusted_intake(
    pool: &PgPool,
    input: &TrustedCompanyIdentityIntake,
) -> Result<ScopingCompanyIdentityReceiptRow> {
    if input.operation_id.is_nil()
        || input.stage_execution_id.is_nil()
        || input.organization_id.is_nil()
        || input.canonical_legal_name.trim().is_empty()
    {
        bail!("SCOPING_TRUSTED_COMPANY_IDENTITY_INPUT_INVALID");
    }

    let mut tx = pool.begin().await?;
    let operation: Option<(String, Option<Uuid>)> = sqlx::query_as(
        r#"SELECT current_stage,project_scope_id FROM operation_state
            WHERE operation_id=$1 AND superseded_by IS NULL FOR SHARE"#,
    )
    .bind(input.operation_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (current_stage, project_scope_id) =
        operation.ok_or_else(|| anyhow::anyhow!("SCOPING_TRUSTED_OPERATION_MISSING"))?;
    if current_stage != "scoping" {
        bail!("SCOPING_TRUSTED_OPERATION_NOT_SCOPING");
    }
    let project_scope_id =
        project_scope_id.ok_or_else(|| anyhow::anyhow!("SCOPING_TRUSTED_PROJECT_SCOPE_MISSING"))?;
    let canonical_project_path: Option<String> = sqlx::query_scalar(
        "SELECT canonical_project_path FROM project_scopes WHERE project_scope_id=$1 AND retired_at IS NULL",
    )
    .bind(project_scope_id)
    .fetch_optional(&mut *tx)
    .await?;
    let canonical_project_path = canonical_project_path
        .ok_or_else(|| anyhow::anyhow!("SCOPING_TRUSTED_PROJECT_SCOPE_MISSING"))?;
    let active_stage: Option<(Uuid,)> = sqlx::query_as(
        r#"SELECT id FROM stage_runs
            WHERE operation_id=$1 AND stage_kind='scoping' AND status='started'
            FOR SHARE"#,
    )
    .bind(input.operation_id)
    .fetch_optional(&mut *tx)
    .await?;
    if active_stage.map(|row| row.0) != Some(input.stage_execution_id) {
        bail!("SCOPING_TRUSTED_STAGE_EXECUTION_MISMATCH");
    }
    let organization: Option<(String, Option<Uuid>)> = sqlx::query_as(
        r#"SELECT name,parent_id FROM organizations
            WHERE id=$1 AND project_path=$2 FOR SHARE"#,
    )
    .bind(input.organization_id)
    .bind(&canonical_project_path)
    .fetch_optional(&mut *tx)
    .await?;
    let (organization_name, parent_id) =
        organization.ok_or_else(|| anyhow::anyhow!("SCOPING_TRUSTED_ORGANIZATION_MISSING"))?;
    if parent_id.is_some()
        || !organization_name
            .trim()
            .eq_ignore_ascii_case(input.canonical_legal_name.trim())
    {
        bail!("SCOPING_TRUSTED_ORGANIZATION_MISMATCH");
    }

    if let Some(existing) = sqlx::query_as::<_, ScopingCompanyIdentityReceiptRow>(
        r#"SELECT id,operation_id,stage_execution_id,resolution_attempt,supersedes_receipt_id,
                  organization_id,subject_hint,canonical_legal_name,aliases,brands,
                  registration_identifiers,disambiguation_fields,confirmation_method,
                  resolution_status,scope_policy,source_receipt_refs,artifact_refs,evidence_refs,
                  identity_payload,identity_sha256,scope_policy_sha256
             FROM scoping_company_identity_receipts
            WHERE operation_id=$1 AND resolution_status='confirmed' FOR SHARE"#,
    )
    .bind(input.operation_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        if existing.stage_execution_id != input.stage_execution_id
            || existing.organization_id != Some(input.organization_id)
            || existing.canonical_legal_name.as_deref() != Some(input.canonical_legal_name.trim())
            || existing.confirmation_method != "exact_reuse"
        {
            bail!("SCOPING_TRUSTED_COMPANY_IDENTITY_REPLAY_MISMATCH");
        }
        tx.commit().await?;
        return Ok(existing);
    }

    let identity_payload = serde_json::json!({
        "canonical_legal_name": input.canonical_legal_name.trim(),
        "authority": "trusted_confirmed_organization_intake_v1",
        "trusted_roots": [],
    });
    let scope_policy = serde_json::json!({
        "owned_only": true,
        "reachable_only": true,
        "trusted_roots": [],
        "third_party_default": "exclude",
    });
    let identity_sha256 = prefixed_sha256(&identity_payload)?;
    let scope_policy_sha256 = prefixed_sha256(&scope_policy)?;
    let created_at = Utc::now();
    let evidence = audit::log_evidence_in_transaction(
        &mut tx,
        "scoping_trusted_company_identity",
        "scoping",
        "scoping.company_identity.trusted_intake.v1",
        Some(&canonical_project_path),
        "trusted_operation_intake",
        None,
        input.session_id.as_deref(),
        Some("trusted_company_identity_intake"),
        &serde_json::json!({
            "kind": "scoping.company_identity.trusted_intake",
            "operation_id": input.operation_id,
            "stage_execution_id": input.stage_execution_id,
            "organization_id": input.organization_id,
            "identity_sha256": identity_sha256,
            "scope_policy_sha256": scope_policy_sha256,
        }),
        Some(input.operation_id),
        None,
        Some(input.canonical_legal_name.trim()),
        Some("confirmed"),
        created_at,
    )
    .await?;
    let evidence_ref = format!("audit:{}", evidence.id);
    let receipt = ScopingCompanyIdentityReceiptRow {
        id: Uuid::new_v5(
            &input.operation_id,
            format!(
                "trusted-company-identity:{}:{}",
                input.stage_execution_id, identity_sha256
            )
            .as_bytes(),
        ),
        operation_id: input.operation_id,
        stage_execution_id: input.stage_execution_id,
        resolution_attempt: 0,
        supersedes_receipt_id: None,
        organization_id: Some(input.organization_id),
        subject_hint: input.canonical_legal_name.trim().to_string(),
        canonical_legal_name: Some(input.canonical_legal_name.trim().to_string()),
        aliases: serde_json::json!([]),
        brands: serde_json::json!([]),
        registration_identifiers: serde_json::json!({}),
        disambiguation_fields: serde_json::json!({
            "authority": "trusted_confirmed_organization_intake_v1"
        }),
        confirmation_method: "exact_reuse".to_string(),
        resolution_status: "confirmed".to_string(),
        scope_policy,
        source_receipt_refs: serde_json::json!([evidence_ref.clone()]),
        artifact_refs: serde_json::json!([evidence_ref.clone()]),
        evidence_refs: serde_json::json!([evidence_ref]),
        identity_payload,
        identity_sha256,
        scope_policy_sha256,
    };
    validate_receipt(&receipt)?;
    sqlx::query(
        r#"INSERT INTO scoping_company_identity_receipts(
               id,operation_id,stage_execution_id,resolution_attempt,supersedes_receipt_id,
               organization_id,subject_hint,canonical_legal_name,aliases,brands,
               registration_identifiers,disambiguation_fields,confirmation_method,
               resolution_status,scope_policy,source_receipt_refs,artifact_refs,evidence_refs,
               identity_payload,identity_sha256,scope_policy_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21)"#,
    )
    .bind(receipt.id)
    .bind(receipt.operation_id)
    .bind(receipt.stage_execution_id)
    .bind(receipt.resolution_attempt)
    .bind(receipt.supersedes_receipt_id)
    .bind(receipt.organization_id)
    .bind(&receipt.subject_hint)
    .bind(&receipt.canonical_legal_name)
    .bind(&receipt.aliases)
    .bind(&receipt.brands)
    .bind(&receipt.registration_identifiers)
    .bind(&receipt.disambiguation_fields)
    .bind(&receipt.confirmation_method)
    .bind(&receipt.resolution_status)
    .bind(&receipt.scope_policy)
    .bind(&receipt.source_receipt_refs)
    .bind(&receipt.artifact_refs)
    .bind(&receipt.evidence_refs)
    .bind(&receipt.identity_payload)
    .bind(&receipt.identity_sha256)
    .bind(&receipt.scope_policy_sha256)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(receipt)
}

fn prefixed_sha256(value: &Value) -> Result<String> {
    let encoded = serde_json::to_vec(value)?;
    let digest = Sha256::digest(encoded);
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("sha256:{hex}"))
}

pub async fn insert_terminal_receipt(
    pool: &PgPool,
    row: &ScopingCompanyIdentityReceiptRow,
) -> Result<ScopingCompanyIdentityReceiptRow> {
    validate_receipt(row)?;
    sqlx::query(
        r#"INSERT INTO scoping_company_identity_receipts(
               id,operation_id,stage_execution_id,resolution_attempt,supersedes_receipt_id,
               organization_id,subject_hint,canonical_legal_name,aliases,brands,
               registration_identifiers,disambiguation_fields,confirmation_method,
               resolution_status,scope_policy,source_receipt_refs,artifact_refs,evidence_refs,
               identity_payload,identity_sha256,scope_policy_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21)
           ON CONFLICT(operation_id,resolution_attempt) DO NOTHING"#,
    )
    .bind(row.id)
    .bind(row.operation_id)
    .bind(row.stage_execution_id)
    .bind(row.resolution_attempt)
    .bind(row.supersedes_receipt_id)
    .bind(row.organization_id)
    .bind(&row.subject_hint)
    .bind(&row.canonical_legal_name)
    .bind(&row.aliases)
    .bind(&row.brands)
    .bind(&row.registration_identifiers)
    .bind(&row.disambiguation_fields)
    .bind(&row.confirmation_method)
    .bind(&row.resolution_status)
    .bind(&row.scope_policy)
    .bind(&row.source_receipt_refs)
    .bind(&row.artifact_refs)
    .bind(&row.evidence_refs)
    .bind(&row.identity_payload)
    .bind(&row.identity_sha256)
    .bind(&row.scope_policy_sha256)
    .execute(pool)
    .await?;
    let persisted = get_by_attempt(pool, row.operation_id, row.resolution_attempt)
        .await?
        .ok_or_else(|| anyhow::anyhow!("SCOPING_COMPANY_IDENTITY_RECEIPT_MISSING"))?;
    if &persisted != row {
        bail!("SCOPING_COMPANY_IDENTITY_REPLAY_MISMATCH");
    }
    Ok(persisted)
}

pub async fn get_confirmed_for_operation(
    pool: &PgPool,
    operation_id: Uuid,
) -> Result<Option<ScopingCompanyIdentityReceiptRow>> {
    load_one(
        pool,
        "operation_id=$1 AND resolution_status='confirmed'",
        operation_id,
        None,
    )
    .await
}

async fn get_by_attempt(
    pool: &PgPool,
    operation_id: Uuid,
    attempt: i64,
) -> Result<Option<ScopingCompanyIdentityReceiptRow>> {
    load_one(
        pool,
        "operation_id=$1 AND resolution_attempt=$2",
        operation_id,
        Some(attempt),
    )
    .await
}

async fn load_one(
    pool: &PgPool,
    predicate: &str,
    operation_id: Uuid,
    attempt: Option<i64>,
) -> Result<Option<ScopingCompanyIdentityReceiptRow>> {
    let sql = format!(
        r#"SELECT id,operation_id,stage_execution_id,resolution_attempt,supersedes_receipt_id,
                  organization_id,subject_hint,canonical_legal_name,aliases,brands,
                  registration_identifiers,disambiguation_fields,confirmation_method,
                  resolution_status,scope_policy,source_receipt_refs,artifact_refs,evidence_refs,
                  identity_payload,identity_sha256,scope_policy_sha256
             FROM scoping_company_identity_receipts WHERE {predicate}"#
    );
    let mut query = sqlx::query_as::<_, ScopingCompanyIdentityReceiptRow>(&sql).bind(operation_id);
    if let Some(attempt) = attempt {
        query = query.bind(attempt);
    }
    query.fetch_optional(pool).await.map_err(Into::into)
}

fn validate_receipt(row: &ScopingCompanyIdentityReceiptRow) -> Result<()> {
    if row.id.is_nil()
        || row.operation_id.is_nil()
        || row.stage_execution_id.is_nil()
        || row.resolution_attempt < 0
        || row.subject_hint.trim().is_empty()
        || !row.identity_sha256.starts_with("sha256:")
        || !row.scope_policy_sha256.starts_with("sha256:")
    {
        bail!("SCOPING_COMPANY_IDENTITY_INPUT_INVALID");
    }
    Ok(())
}
