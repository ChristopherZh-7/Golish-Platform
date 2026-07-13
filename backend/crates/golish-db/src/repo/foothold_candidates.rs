use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::Result;

pub const TABLE_NAME: &str = "foothold_candidates";

#[derive(Clone, Debug, sqlx::FromRow, PartialEq, Eq)]
pub struct FootholdCandidateRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub source: String,
    pub source_id: Uuid,
    pub target_live_id: Option<Uuid>,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub target_identity_hash: String,
    pub status: String,
    pub row_version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub terminal_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewFootholdCandidate {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub source: String,
    pub source_id: Uuid,
    pub target_live_id: Option<Uuid>,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub target_identity_hash: String,
    pub evidence: Vec<(i64, String)>,
}

fn validate(input: &NewFootholdCandidate) -> Result<()> {
    if input.source.trim().is_empty()
        || input.target_type_at_time.trim().is_empty()
        || input.target_value_at_time.trim().is_empty()
        || input.target_identity_hash.len() != 64
        || !input
            .target_identity_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || input.evidence.is_empty()
        || input.evidence.len() > 1024
        || input.evidence.iter().any(|(id, role)| {
            *id <= 0 || !matches!(role.as_str(), "observation" | "support" | "validation")
        })
    {
        return Err(anyhow::anyhow!("post_exploit_candidate_invalid").into());
    }
    let mut evidence = input.evidence.clone();
    evidence.sort_unstable();
    evidence.dedup();
    if evidence.len() != input.evidence.len() {
        return Err(anyhow::anyhow!("post_exploit_candidate_evidence_duplicate").into());
    }
    Ok(())
}

pub(crate) async fn validate_evidence_authority(
    connection: &mut PgConnection,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id_at_time: Uuid,
    evidence_ids: &[i64],
) -> Result<()> {
    let mut unique = evidence_ids.to_vec();
    unique.sort_unstable();
    unique.dedup();
    if unique.len() != evidence_ids.len() || unique.is_empty() || unique.len() > 1024 {
        return Err(anyhow::anyhow!("post_exploit_evidence_identity_invalid").into());
    }
    let project_path = sqlx::query_scalar::<_, String>(
        r#"SELECT project_path_at_freeze
             FROM operation_org_scope_snapshots
            WHERE id=$1 AND operation_id=$2 AND sealed_at IS NOT NULL
            FOR SHARE"#,
    )
    .bind(scope_snapshot_id)
    .bind(operation_id)
    .fetch_optional(&mut *connection)
    .await?
    .ok_or_else(|| anyhow::anyhow!("post_exploit_scope_snapshot_invalid"))?;
    let rows = sqlx::query_as::<_, (i64,)>(
        r#"SELECT id FROM audit_log
            WHERE id=ANY($1) AND audit_role='evidence' AND run_id=$2
              AND project_path=$3
              AND detail ->> 'organization_id'=$4
            FOR SHARE"#,
    )
    .bind(&unique)
    .bind(operation_id)
    .bind(project_path)
    .bind(organization_id_at_time.to_string())
    .fetch_all(&mut *connection)
    .await?;
    if rows.len() != unique.len() {
        return Err(anyhow::anyhow!("post_exploit_evidence_stale_or_foreign").into());
    }
    Ok(())
}

pub async fn insert_with_connection(
    connection: &mut PgConnection,
    input: &NewFootholdCandidate,
) -> Result<FootholdCandidateRow> {
    validate(input)?;
    validate_evidence_authority(
        connection,
        input.operation_id,
        input.scope_snapshot_id,
        input.organization_id_at_time,
        &input.evidence.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
    )
    .await?;
    let row = sqlx::query_as::<_, FootholdCandidateRow>(
        r#"INSERT INTO foothold_candidates(
               id,operation_id,project_scope_id,scope_snapshot_id,
               organization_id_at_time,source,source_id,target_live_id,
               target_type_at_time,target_value_at_time,target_identity_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
           ON CONFLICT(operation_id,source,source_id) DO NOTHING
           RETURNING *"#,
    )
    .bind(input.id)
    .bind(input.operation_id)
    .bind(input.project_scope_id)
    .bind(input.scope_snapshot_id)
    .bind(input.organization_id_at_time)
    .bind(&input.source)
    .bind(input.source_id)
    .bind(input.target_live_id)
    .bind(&input.target_type_at_time)
    .bind(&input.target_value_at_time)
    .bind(&input.target_identity_hash)
    .fetch_optional(&mut *connection)
    .await?;
    let (row, inserted_new) = match row {
        Some(row) => (row, true),
        None => {
            let existing = sqlx::query_as::<_, FootholdCandidateRow>(
                r#"SELECT * FROM foothold_candidates
                    WHERE operation_id=$1 AND source=$2 AND source_id=$3
                    FOR SHARE"#,
            )
            .bind(input.operation_id)
            .bind(&input.source)
            .bind(input.source_id)
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
            {
                return Err(anyhow::anyhow!("post_exploit_candidate_replay_conflict").into());
            }
            (existing, false)
        }
    };
    if inserted_new {
        for (evidence_id, role) in &input.evidence {
            sqlx::query(
                r#"INSERT INTO foothold_candidate_evidence(
                       foothold_candidate_id,evidence_id,role
                   ) VALUES($1,$2,$3)"#,
            )
            .bind(row.id)
            .bind(evidence_id)
            .bind(role)
            .execute(&mut *connection)
            .await?;
        }
    }
    let persisted = sqlx::query_as::<_, (i64, String)>(
        r#"SELECT evidence_id,role FROM foothold_candidate_evidence
            WHERE foothold_candidate_id=$1 ORDER BY evidence_id,role"#,
    )
    .bind(row.id)
    .fetch_all(&mut *connection)
    .await?;
    let mut expected = input.evidence.clone();
    expected.sort_unstable();
    if persisted != expected {
        return Err(anyhow::anyhow!("post_exploit_candidate_evidence_replay_conflict").into());
    }
    Ok(row)
}

pub async fn create(pool: &PgPool, input: &NewFootholdCandidate) -> Result<FootholdCandidateRow> {
    let mut tx = pool.begin().await?;
    let row = insert_with_connection(&mut tx, input).await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<FootholdCandidateRow>> {
    Ok(
        sqlx::query_as::<_, FootholdCandidateRow>("SELECT * FROM foothold_candidates WHERE id=$1")
            .bind(id)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn list_for_scope(
    pool: &PgPool,
    project_scope_id: Uuid,
    organization_id_at_time: Uuid,
) -> Result<Vec<FootholdCandidateRow>> {
    Ok(sqlx::query_as::<_, FootholdCandidateRow>(
        r#"SELECT * FROM foothold_candidates
            WHERE project_scope_id=$1 AND organization_id_at_time=$2
            ORDER BY created_at,id"#,
    )
    .bind(project_scope_id)
    .bind(organization_id_at_time)
    .fetch_all(pool)
    .await?)
}

pub async fn evidence_payload(pool: &PgPool, id: Uuid) -> Result<Value> {
    let rows = sqlx::query_as::<_, (i64, String)>(
        r#"SELECT evidence_id,role FROM foothold_candidate_evidence
            WHERE foothold_candidate_id=$1 ORDER BY evidence_id,role"#,
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    Ok(serde_json::json!(rows
        .into_iter()
        .map(|(evidence_id, role)| serde_json::json!({
            "evidence_id": evidence_id,
            "role": role,
        }))
        .collect::<Vec<_>>()))
}
