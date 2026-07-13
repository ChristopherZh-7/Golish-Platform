use chrono::{DateTime, Utc};
use golish_memory_domain::event_catalog::{
    KnowledgeEventEnvelopeV1, KnowledgeEventNameV1, KnowledgeEventPayloadV1,
};
use golish_memory_domain::scope::ProjectScopeId;
use golish_memory_domain::source_ref::{CanonicalRowId, CanonicalSourceKind, SourceRef};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::Result;

pub const TABLE_NAME: &str = "footholds";

#[derive(Clone, Debug, sqlx::FromRow, PartialEq, Eq)]
pub struct FootholdRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub validation_unit_kind: String,
    pub validation_unit_id: Uuid,
    pub target_live_id: Option<Uuid>,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub target_identity_hash: String,
    pub vault_credential_ref: Option<Uuid>,
    pub status: String,
    pub row_version: i64,
    pub validated_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub terminal_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidateFoothold {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub validation_unit_kind: String,
    pub validation_unit_id: Uuid,
    pub target_live_id: Option<Uuid>,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub target_identity_hash: String,
    pub vault_credential_ref: Option<Uuid>,
    pub evidence: Vec<(i64, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessValidationSourceSnapshot {
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub target_identity_hash: String,
}

/// Reload the immutable target snapshot for one exact allowed access-validation
/// unit. Model-visible input supplies only the opaque unit id; it cannot replace
/// the target value/hash that will be persisted into the Foothold.
pub async fn load_access_validation_source(
    pool: &PgPool,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id_at_time: Uuid,
    validation_unit_kind: &str,
    validation_unit_id: Uuid,
) -> Result<AccessValidationSourceSnapshot> {
    let row = match validation_unit_kind {
        "candidate_attempt" => {
            sqlx::query_as::<_, (String, String, String)>(
                r#"SELECT target_type_at_time,target_value_at_time,target_identity_hash
                 FROM candidate_attempts
                WHERE id=$1 AND operation_id=$2 AND scope_snapshot_id=$3
                  AND organization_id=$4 AND status='verified'"#,
            )
            .bind(validation_unit_id)
            .bind(operation_id)
            .bind(scope_snapshot_id)
            .bind(organization_id_at_time)
            .fetch_optional(pool)
            .await?
        }
        "foothold_candidate" => {
            sqlx::query_as::<_, (String, String, String)>(
                r#"SELECT target_type_at_time,target_value_at_time,target_identity_hash
                 FROM foothold_candidates
                WHERE id=$1 AND operation_id=$2 AND scope_snapshot_id=$3
                  AND organization_id_at_time=$4 AND status IN ('pending','validated')"#,
            )
            .bind(validation_unit_id)
            .bind(operation_id)
            .bind(scope_snapshot_id)
            .bind(organization_id_at_time)
            .fetch_optional(pool)
            .await?
        }
        _ => return Err(anyhow::anyhow!("post_exploit_access_source_kind_invalid").into()),
    }
    .ok_or_else(|| anyhow::anyhow!("post_exploit_access_source_not_authorized"))?;
    Ok(AccessValidationSourceSnapshot {
        target_type_at_time: row.0,
        target_value_at_time: row.1,
        target_identity_hash: row.2,
    })
}

fn validate(input: &ValidateFoothold) -> Result<()> {
    if !matches!(
        input.validation_unit_kind.as_str(),
        "candidate_attempt" | "foothold_candidate"
    ) || input.target_type_at_time.trim().is_empty()
        || input.target_value_at_time.trim().is_empty()
        || input.target_identity_hash.len() != 64
        || !input
            .target_identity_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || input.evidence.is_empty()
        || input.evidence.len() > 1024
        || input
            .evidence
            .iter()
            .any(|(id, role)| *id <= 0 || !matches!(role.as_str(), "validation" | "support"))
    {
        return Err(anyhow::anyhow!("post_exploit_foothold_invalid").into());
    }
    let mut evidence = input.evidence.clone();
    evidence.sort_unstable();
    evidence.dedup();
    if evidence.len() != input.evidence.len() {
        return Err(anyhow::anyhow!("post_exploit_foothold_evidence_duplicate").into());
    }
    Ok(())
}

pub async fn validate_with_connection(
    connection: &mut PgConnection,
    input: &ValidateFoothold,
) -> Result<FootholdRow> {
    validate(input)?;
    super::foothold_candidates::validate_evidence_authority(
        connection,
        input.operation_id,
        input.scope_snapshot_id,
        input.organization_id_at_time,
        &input.evidence.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
    )
    .await?;
    let source_snapshot = match input.validation_unit_kind.as_str() {
        "candidate_attempt" => {
            sqlx::query_as::<_, (String, String, String)>(
                r#"SELECT target_type_at_time,target_value_at_time,target_identity_hash
                 FROM candidate_attempts
                WHERE id=$1 AND operation_id=$2 AND scope_snapshot_id=$3
                  AND organization_id=$4 AND status='verified'
                FOR SHARE"#,
            )
            .bind(input.validation_unit_id)
            .bind(input.operation_id)
            .bind(input.scope_snapshot_id)
            .bind(input.organization_id_at_time)
            .fetch_optional(&mut *connection)
            .await?
        }
        "foothold_candidate" => {
            sqlx::query_as::<_, (String, String, String)>(
                r#"SELECT target_type_at_time,target_value_at_time,target_identity_hash
                 FROM foothold_candidates
                WHERE id=$1 AND operation_id=$2 AND scope_snapshot_id=$3
                  AND organization_id_at_time=$4 AND status IN ('pending','validated')
                FOR SHARE"#,
            )
            .bind(input.validation_unit_id)
            .bind(input.operation_id)
            .bind(input.scope_snapshot_id)
            .bind(input.organization_id_at_time)
            .fetch_optional(&mut *connection)
            .await?
        }
        _ => None,
    }
    .ok_or_else(|| anyhow::anyhow!("post_exploit_access_source_not_authorized"))?;
    if source_snapshot
        != (
            input.target_type_at_time.clone(),
            input.target_value_at_time.clone(),
            input.target_identity_hash.clone(),
        )
    {
        return Err(anyhow::anyhow!("post_exploit_access_source_snapshot_mismatch").into());
    }
    let row = sqlx::query_as::<_, FootholdRow>(
        r#"INSERT INTO footholds(
               id,operation_id,project_scope_id,scope_snapshot_id,
               organization_id_at_time,validation_unit_kind,validation_unit_id,
               target_live_id,target_type_at_time,target_value_at_time,
               target_identity_hash,vault_credential_ref
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
           ON CONFLICT(operation_id,validation_unit_kind,validation_unit_id) DO NOTHING
           RETURNING *"#,
    )
    .bind(input.id)
    .bind(input.operation_id)
    .bind(input.project_scope_id)
    .bind(input.scope_snapshot_id)
    .bind(input.organization_id_at_time)
    .bind(&input.validation_unit_kind)
    .bind(input.validation_unit_id)
    .bind(input.target_live_id)
    .bind(&input.target_type_at_time)
    .bind(&input.target_value_at_time)
    .bind(&input.target_identity_hash)
    .bind(input.vault_credential_ref)
    .fetch_optional(&mut *connection)
    .await?;
    let (row, inserted_new) = match row {
        Some(row) => (row, true),
        None => {
            let existing = sqlx::query_as::<_, FootholdRow>(
                r#"SELECT * FROM footholds
                    WHERE operation_id=$1 AND validation_unit_kind=$2
                      AND validation_unit_id=$3 FOR SHARE"#,
            )
            .bind(input.operation_id)
            .bind(&input.validation_unit_kind)
            .bind(input.validation_unit_id)
            .fetch_one(&mut *connection)
            .await?;
            if existing.id != input.id
                || existing.project_scope_id != input.project_scope_id
                || existing.scope_snapshot_id != input.scope_snapshot_id
                || existing.organization_id_at_time != input.organization_id_at_time
                || existing.target_live_id != input.target_live_id
                || existing.target_type_at_time != input.target_type_at_time
                || existing.target_value_at_time != input.target_value_at_time
                || existing.target_identity_hash != input.target_identity_hash
                || existing.vault_credential_ref != input.vault_credential_ref
            {
                return Err(anyhow::anyhow!("post_exploit_foothold_replay_conflict").into());
            }
            (existing, false)
        }
    };
    if inserted_new {
        for (evidence_id, role) in &input.evidence {
            sqlx::query(
                r#"INSERT INTO foothold_evidence(foothold_id,evidence_id,role)
                   VALUES($1,$2,$3)"#,
            )
            .bind(row.id)
            .bind(evidence_id)
            .bind(role)
            .execute(&mut *connection)
            .await?;
        }
    }
    let persisted = sqlx::query_as::<_, (i64, String)>(
        r#"SELECT evidence_id,role FROM foothold_evidence
            WHERE foothold_id=$1 ORDER BY evidence_id,role"#,
    )
    .bind(row.id)
    .fetch_all(&mut *connection)
    .await?;
    let mut expected = input.evidence.clone();
    expected.sort_unstable();
    if persisted != expected {
        return Err(anyhow::anyhow!("post_exploit_foothold_evidence_replay_conflict").into());
    }
    if input.validation_unit_kind == "foothold_candidate" {
        let updated = sqlx::query(
            r#"UPDATE foothold_candidates
                  SET status='validated',terminal_at=NOW(),row_version=row_version+1,
                      updated_at=NOW()
                WHERE id=$1 AND operation_id=$2 AND project_scope_id=$3
                  AND scope_snapshot_id=$4 AND organization_id_at_time=$5
                  AND target_identity_hash=$6 AND status='pending'"#,
        )
        .bind(input.validation_unit_id)
        .bind(input.operation_id)
        .bind(input.project_scope_id)
        .bind(input.scope_snapshot_id)
        .bind(input.organization_id_at_time)
        .bind(&input.target_identity_hash)
        .execute(&mut *connection)
        .await?;
        if updated.rows_affected() == 0 {
            let already_validated: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM foothold_candidates WHERE id=$1 AND status='validated')",
            )
            .bind(input.validation_unit_id)
            .fetch_one(&mut *connection)
            .await?;
            if !already_validated {
                return Err(anyhow::anyhow!("post_exploit_candidate_validation_cas_failed").into());
            }
        }
    }
    Ok(row)
}

pub async fn validate_and_create(pool: &PgPool, input: &ValidateFoothold) -> Result<FootholdRow> {
    let mut tx = pool.begin().await?;
    let row = validate_with_connection(&mut tx, input).await?;
    let candidate_source = if input.validation_unit_kind == "candidate_attempt" {
        "candidate_attempt".to_string()
    } else {
        sqlx::query_scalar::<_, String>(
            "SELECT source FROM foothold_candidates WHERE id=$1 FOR SHARE",
        )
        .bind(input.validation_unit_id)
        .fetch_one(&mut *tx)
        .await?
    };
    let source = SourceRef {
        source_kind: CanonicalSourceKind::Foothold,
        row_id: CanonicalRowId::Uuid(row.id),
        source_stream_key: format!("foothold:{}", row.id),
        version: row.row_version + 1,
    };
    let evidence_ids = input
        .evidence
        .iter()
        .map(|(evidence_id, _)| *evidence_id)
        .collect::<Vec<_>>();
    let event = KnowledgeEventEnvelopeV1 {
        event_id: Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!("PostExploitFactTerminal.v1:foothold:{}", row.id).as_bytes(),
        ),
        project_scope_id: Some(ProjectScopeId(row.project_scope_id)),
        organization_id_at_time: Some(row.organization_id_at_time),
        source_operation_id: row.operation_id,
        event_name: KnowledgeEventNameV1::PostExploitFactTerminal,
        schema_version: 1,
        payload: KnowledgeEventPayloadV1 {
            source: source.clone(),
            source_stream_key: source.source_stream_key.clone(),
            source_version: source.version,
            structured_payload: serde_json::json!({
                "fact_kind": "foothold",
                "foothold_id": row.id,
                "candidate_source": candidate_source,
                "target_type_at_time": &row.target_type_at_time,
                "target_value_at_time": &row.target_value_at_time,
                "target_identity_hash": &row.target_identity_hash,
                "evidence_ids": evidence_ids,
            }),
        },
        occurred_at: row.validated_at,
    };
    super::knowledge_outbox::append_event_with_catalog_deliveries(&mut tx, &event)
        .await
        .map_err(|error| anyhow::anyhow!("post_exploit_foothold_outbox_failed: {error}"))?;
    tx.commit().await?;
    Ok(row)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<FootholdRow>> {
    Ok(
        sqlx::query_as::<_, FootholdRow>("SELECT * FROM footholds WHERE id=$1")
            .bind(id)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn active_exists(
    pool: &PgPool,
    operation_id: Uuid,
    project_scope_id: Uuid,
    organization_id_at_time: Uuid,
    foothold_id: Uuid,
) -> Result<bool> {
    Ok(sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM footholds
                WHERE id=$1 AND operation_id=$2 AND project_scope_id=$3
                  AND organization_id_at_time=$4 AND status='active'
           )"#,
    )
    .bind(foothold_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id_at_time)
    .fetch_one(pool)
    .await?)
}

pub async fn resolve_exact_scope_snapshot(
    pool: &PgPool,
    operation_id: Uuid,
    project_scope_id: Uuid,
    organization_id_at_time: Uuid,
) -> Result<Option<Uuid>> {
    Ok(
        super::operation_org_scope::sealed_snapshot_id_for_exact_scope(
            pool,
            operation_id,
            project_scope_id,
            organization_id_at_time,
        )
        .await?,
    )
}
