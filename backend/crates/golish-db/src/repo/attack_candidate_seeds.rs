//! Frozen observation seeds from which Candidate reasoning work-items are made.

use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]
pub struct AttackCandidateSeedRow {
    pub id: Uuid,
    pub wave_unit_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub target_live_id: Option<Uuid>,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub target_identity_hash: String,
    pub technique: String,
    pub observation: serde_json::Value,
    pub observation_hash: String,
    pub source_fact_delta_id: Option<Uuid>,
    pub delta_kind: Option<String>,
    pub observation_kind: String,
    pub allowed_techniques: Vec<String>,
    pub enrichment_required: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewAttackCandidateSeed {
    pub id: Uuid,
    pub target_live_id: Option<Uuid>,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub target_identity_hash: String,
    pub technique: String,
    pub observation: serde_json::Value,
    pub observation_hash: String,
    pub source_fact_delta_id: Option<Uuid>,
    pub delta_kind: Option<String>,
    pub observation_kind: String,
    pub allowed_techniques: Vec<String>,
    pub enrichment_required: bool,
}

const COLUMNS: &str = "id,wave_unit_id,operation_id,scope_snapshot_id,organization_id,\
    target_live_id,target_type_at_time,target_value_at_time,target_identity_hash,technique,\
    observation,observation_hash,source_fact_delta_id,delta_kind,observation_kind,\
    allowed_techniques,enrichment_required,created_at";

pub(crate) async fn insert_or_get_exact(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    wave_unit_id: Uuid,
    organization_id: Uuid,
    seed: &NewAttackCandidateSeed,
) -> crate::Result<AttackCandidateSeedRow> {
    let insert_sql = format!(
        "INSERT INTO attack_candidate_seeds(
             id,wave_unit_id,operation_id,scope_snapshot_id,organization_id,target_live_id,
             target_type_at_time,target_value_at_time,target_identity_hash,technique,
             observation,observation_hash,source_fact_delta_id,delta_kind,observation_kind,
             allowed_techniques,enrichment_required)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)
         ON CONFLICT(wave_unit_id,target_identity_hash,technique,observation_hash) DO NOTHING
         RETURNING {COLUMNS}"
    );
    let inserted = sqlx::query_as::<_, AttackCandidateSeedRow>(&insert_sql)
        .bind(seed.id)
        .bind(wave_unit_id)
        .bind(operation_id)
        .bind(scope_snapshot_id)
        .bind(organization_id)
        .bind(seed.target_live_id)
        .bind(&seed.target_type_at_time)
        .bind(&seed.target_value_at_time)
        .bind(&seed.target_identity_hash)
        .bind(&seed.technique)
        .bind(&seed.observation)
        .bind(&seed.observation_hash)
        .bind(seed.source_fact_delta_id)
        .bind(&seed.delta_kind)
        .bind(&seed.observation_kind)
        .bind(&seed.allowed_techniques)
        .bind(seed.enrichment_required)
        .fetch_optional(&mut **tx)
        .await?;
    let row = if let Some(row) = inserted {
        row
    } else {
        let select_sql = format!(
            "SELECT {COLUMNS} FROM attack_candidate_seeds
             WHERE wave_unit_id=$1 AND target_identity_hash=$2
               AND technique=$3 AND observation_hash=$4 FOR UPDATE"
        );
        sqlx::query_as::<_, AttackCandidateSeedRow>(&select_sql)
            .bind(wave_unit_id)
            .bind(&seed.target_identity_hash)
            .bind(&seed.technique)
            .bind(&seed.observation_hash)
            .fetch_one(&mut **tx)
            .await?
    };
    if row.operation_id != operation_id
        || row.scope_snapshot_id != scope_snapshot_id
        || row.organization_id != organization_id
        || row.target_type_at_time != seed.target_type_at_time
        || row.target_value_at_time != seed.target_value_at_time
        || row.observation != seed.observation
        || row.source_fact_delta_id != seed.source_fact_delta_id
        || row.delta_kind != seed.delta_kind
        || row.observation_kind != seed.observation_kind
        || row.allowed_techniques != seed.allowed_techniques
        || row.enrichment_required != seed.enrichment_required
    {
        return Err(crate::DbError::Other(anyhow::anyhow!(
            "attack seed idempotency identity mismatch"
        )));
    }
    Ok(row)
}
