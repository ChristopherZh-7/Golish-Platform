use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::Result;

pub const TABLE_NAME: &str = "internal_asset_observations";

#[derive(Clone, Debug, sqlx::FromRow, PartialEq)]
pub struct InternalAssetObservationRow {
    pub id: Uuid,
    pub foothold_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub asset_type: String,
    pub asset_value_at_time: String,
    pub asset_identity_hash: String,
    pub observation_kind: String,
    pub observation: Value,
    pub observation_hash: String,
    pub observed_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NewInternalAssetObservation {
    pub id: Uuid,
    pub foothold_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub asset_type: String,
    pub asset_value_at_time: String,
    pub asset_identity_hash: String,
    pub observation_kind: String,
    pub observation: Value,
    pub observation_hash: String,
    pub observed_at: DateTime<Utc>,
    pub evidence: Vec<(i64, String)>,
}

fn is_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate(input: &NewInternalAssetObservation) -> Result<()> {
    if input.asset_type.trim().is_empty()
        || input.asset_value_at_time.trim().is_empty()
        || input.observation_kind.trim().is_empty()
        || !input.observation.is_object()
        || !is_hash(&input.asset_identity_hash)
        || !is_hash(&input.observation_hash)
        || input.evidence.is_empty()
        || input.evidence.len() > 1024
        || input
            .evidence
            .iter()
            .any(|(id, role)| *id <= 0 || !matches!(role.as_str(), "observation" | "support"))
    {
        return Err(anyhow::anyhow!("post_exploit_internal_observation_invalid").into());
    }
    let mut evidence = input.evidence.clone();
    evidence.sort_unstable();
    evidence.dedup();
    if evidence.len() != input.evidence.len() {
        return Err(anyhow::anyhow!("post_exploit_internal_evidence_duplicate").into());
    }
    Ok(())
}

pub async fn insert_with_connection(
    connection: &mut PgConnection,
    input: &NewInternalAssetObservation,
) -> Result<InternalAssetObservationRow> {
    validate(input)?;
    super::foothold_candidates::validate_evidence_authority(
        connection,
        input.operation_id,
        input.scope_snapshot_id,
        input.organization_id_at_time,
        &input.evidence.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
    )
    .await?;
    let row = sqlx::query_as::<_, InternalAssetObservationRow>(
        r#"INSERT INTO internal_asset_observations(
               id,foothold_id,operation_id,project_scope_id,scope_snapshot_id,
               organization_id_at_time,asset_type,asset_value_at_time,
               asset_identity_hash,observation_kind,observation,observation_hash,observed_at
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
           ON CONFLICT(operation_id,foothold_id,asset_identity_hash,observation_hash)
           DO NOTHING RETURNING *"#,
    )
    .bind(input.id)
    .bind(input.foothold_id)
    .bind(input.operation_id)
    .bind(input.project_scope_id)
    .bind(input.scope_snapshot_id)
    .bind(input.organization_id_at_time)
    .bind(&input.asset_type)
    .bind(&input.asset_value_at_time)
    .bind(&input.asset_identity_hash)
    .bind(&input.observation_kind)
    .bind(&input.observation)
    .bind(&input.observation_hash)
    .bind(input.observed_at)
    .fetch_optional(&mut *connection)
    .await?;
    let (row, inserted_new) = match row {
        Some(row) => (row, true),
        None => {
            let existing = sqlx::query_as::<_, InternalAssetObservationRow>(
                r#"SELECT * FROM internal_asset_observations
                    WHERE operation_id=$1 AND foothold_id=$2
                      AND asset_identity_hash=$3 AND observation_hash=$4 FOR SHARE"#,
            )
            .bind(input.operation_id)
            .bind(input.foothold_id)
            .bind(&input.asset_identity_hash)
            .bind(&input.observation_hash)
            .fetch_one(&mut *connection)
            .await?;
            if existing.id != input.id
                || existing.project_scope_id != input.project_scope_id
                || existing.scope_snapshot_id != input.scope_snapshot_id
                || existing.organization_id_at_time != input.organization_id_at_time
                || existing.asset_type != input.asset_type
                || existing.asset_value_at_time != input.asset_value_at_time
                || existing.observation_kind != input.observation_kind
                || existing.observation != input.observation
            {
                return Err(
                    anyhow::anyhow!("post_exploit_internal_observation_replay_conflict").into(),
                );
            }
            (existing, false)
        }
    };
    if inserted_new {
        for (evidence_id, role) in &input.evidence {
            sqlx::query(
                r#"INSERT INTO internal_asset_observation_evidence(
                       observation_id,evidence_id,role
                   ) VALUES($1,$2,$3)"#,
            )
            .bind(row.id)
            .bind(evidence_id)
            .bind(role)
            .execute(&mut *connection)
            .await?;
        }
    }
    let stored_evidence = sqlx::query_as::<_, (i64, String)>(
        r#"SELECT evidence_id,role FROM internal_asset_observation_evidence
            WHERE observation_id=$1 ORDER BY evidence_id,role"#,
    )
    .bind(row.id)
    .fetch_all(&mut *connection)
    .await?;
    let mut expected_evidence = input.evidence.clone();
    expected_evidence.sort_unstable();
    if stored_evidence != expected_evidence {
        return Err(anyhow::anyhow!("post_exploit_internal_evidence_replay_conflict").into());
    }
    Ok(row)
}

pub async fn insert_batch(
    pool: &PgPool,
    inputs: &[NewInternalAssetObservation],
) -> Result<Vec<InternalAssetObservationRow>> {
    if inputs.is_empty() || inputs.len() > 256 {
        return Err(anyhow::anyhow!("post_exploit_internal_batch_invalid").into());
    }
    let mut tx = pool.begin().await?;
    let mut rows = Vec::with_capacity(inputs.len());
    for input in inputs {
        rows.push(insert_with_connection(&mut tx, input).await?);
    }
    tx.commit().await?;
    Ok(rows)
}

pub async fn list_for_foothold(
    pool: &PgPool,
    operation_id: Uuid,
    foothold_id: Uuid,
) -> Result<Vec<InternalAssetObservationRow>> {
    Ok(sqlx::query_as::<_, InternalAssetObservationRow>(
        r#"SELECT * FROM internal_asset_observations
            WHERE operation_id=$1 AND foothold_id=$2 ORDER BY observed_at,id"#,
    )
    .bind(operation_id)
    .bind(foothold_id)
    .fetch_all(pool)
    .await?)
}
