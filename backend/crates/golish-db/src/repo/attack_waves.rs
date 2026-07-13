//! Operation/scope/org-scoped Wave repository for Candidate V2.

use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AttackWaveRunRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub generation: i32,
    pub status: String,
    pub policy_snapshot: serde_json::Value,
    pub policy_hash: String,
    pub max_waves: i32,
    pub max_candidates_total: i32,
    pub max_chain_depth: i32,
    pub max_attempts_total: i32,
    pub row_version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub terminal_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AttackWaveUnitRow {
    pub id: Uuid,
    pub wave_run_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub entry_stage_execution_id: Uuid,
    pub entry_stage_run_unit_id: Uuid,
    pub entry_deliverable_submission_id: Uuid,
    pub entry_stage_kind: String,
    pub ordinal: i32,
    pub status: String,
    pub review_closed: bool,
    pub verification_closed: bool,
    pub consolidation_status: String,
    pub manifest_hash: Option<String>,
    pub manifest_count: Option<i32>,
    pub manifest_frozen_at: Option<DateTime<Utc>>,
    pub row_version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub terminal_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct OpenAttackWaveUnit {
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub entry_stage_execution_id: Uuid,
    pub entry_stage_run_unit_id: Uuid,
    pub entry_deliverable_submission_id: Uuid,
    pub generation: i32,
    pub ordinal: i32,
    pub policy_snapshot: serde_json::Value,
    pub policy_hash: String,
    pub max_waves: i32,
    pub max_candidates_total: i32,
    pub max_chain_depth: i32,
    pub max_attempts_total: i32,
}

const WAVE_COLUMNS: &str = "id,operation_id,scope_snapshot_id,generation,status,policy_snapshot,\
    policy_hash,max_waves,max_candidates_total,max_chain_depth,max_attempts_total,row_version,\
    created_at,updated_at,terminal_at";
const UNIT_COLUMNS: &str = "id,wave_run_id,operation_id,scope_snapshot_id,organization_id,\
    entry_stage_execution_id,entry_stage_run_unit_id,entry_deliverable_submission_id,\
    entry_stage_kind,ordinal,status,review_closed,verification_closed,consolidation_status,\
    manifest_hash,manifest_count,manifest_frozen_at,row_version,created_at,updated_at,terminal_at";

/// Open/replay one WaveUnit from an exact upstream vuln_triage final handoff.
/// Natural-key replay compares every frozen policy and entry identity field;
/// drift fails closed rather than silently reusing a different wave.
pub async fn open_from_vuln_triage_handoff(
    tx: &mut Transaction<'_, Postgres>,
    input: &OpenAttackWaveUnit,
) -> crate::Result<(AttackWaveRunRow, AttackWaveUnitRow)> {
    if input.generation < 0
        || input.ordinal < 0
        || input.policy_hash.trim().is_empty()
        || !input.policy_snapshot.is_object()
    {
        return Err(crate::DbError::Other(anyhow::anyhow!(
            "invalid attack wave entry request"
        )));
    }
    let wave_insert = format!(
        "INSERT INTO attack_wave_runs(
             id,operation_id,scope_snapshot_id,generation,status,policy_snapshot,policy_hash,
             max_waves,max_candidates_total,max_chain_depth,max_attempts_total)
         VALUES($1,$2,$3,$4,'open',$5,$6,$7,$8,$9,$10)
         ON CONFLICT(operation_id,generation) DO NOTHING RETURNING {WAVE_COLUMNS}"
    );
    let inserted_wave = sqlx::query_as::<_, AttackWaveRunRow>(&wave_insert)
        .bind(input.wave_run_id)
        .bind(input.operation_id)
        .bind(input.scope_snapshot_id)
        .bind(input.generation)
        .bind(&input.policy_snapshot)
        .bind(&input.policy_hash)
        .bind(input.max_waves)
        .bind(input.max_candidates_total)
        .bind(input.max_chain_depth)
        .bind(input.max_attempts_total)
        .fetch_optional(&mut **tx)
        .await?;
    let wave = match inserted_wave {
        Some(row) => row,
        None => {
            let sql = format!(
                "SELECT {WAVE_COLUMNS} FROM attack_wave_runs
                 WHERE operation_id=$1 AND generation=$2 FOR UPDATE"
            );
            sqlx::query_as(&sql)
                .bind(input.operation_id)
                .bind(input.generation)
                .fetch_one(&mut **tx)
                .await?
        }
    };
    if wave.id != input.wave_run_id
        || wave.scope_snapshot_id != input.scope_snapshot_id
        || wave.policy_snapshot != input.policy_snapshot
        || wave.policy_hash != input.policy_hash
        || wave.max_waves != input.max_waves
        || wave.max_candidates_total != input.max_candidates_total
        || wave.max_chain_depth != input.max_chain_depth
        || wave.max_attempts_total != input.max_attempts_total
    {
        return Err(crate::DbError::Other(anyhow::anyhow!(
            "attack wave replay drift"
        )));
    }

    let unit_insert = format!(
        "INSERT INTO attack_wave_units(
             id,wave_run_id,operation_id,scope_snapshot_id,organization_id,
             entry_stage_execution_id,entry_stage_run_unit_id,
             entry_deliverable_submission_id,entry_stage_kind,ordinal,status)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,'vuln_triage',$9,'open')
         ON CONFLICT(wave_run_id,organization_id) DO NOTHING RETURNING {UNIT_COLUMNS}"
    );
    let inserted_unit = sqlx::query_as::<_, AttackWaveUnitRow>(&unit_insert)
        .bind(input.wave_unit_id)
        .bind(input.wave_run_id)
        .bind(input.operation_id)
        .bind(input.scope_snapshot_id)
        .bind(input.organization_id)
        .bind(input.entry_stage_execution_id)
        .bind(input.entry_stage_run_unit_id)
        .bind(input.entry_deliverable_submission_id)
        .bind(input.ordinal)
        .fetch_optional(&mut **tx)
        .await?;
    let unit = match inserted_unit {
        Some(row) => row,
        None => {
            let sql = format!(
                "SELECT {UNIT_COLUMNS} FROM attack_wave_units
                 WHERE wave_run_id=$1 AND organization_id=$2 FOR UPDATE"
            );
            sqlx::query_as(&sql)
                .bind(input.wave_run_id)
                .bind(input.organization_id)
                .fetch_one(&mut **tx)
                .await?
        }
    };
    if unit.id != input.wave_unit_id
        || unit.operation_id != input.operation_id
        || unit.scope_snapshot_id != input.scope_snapshot_id
        || unit.entry_stage_execution_id != input.entry_stage_execution_id
        || unit.entry_stage_run_unit_id != input.entry_stage_run_unit_id
        || unit.entry_deliverable_submission_id != input.entry_deliverable_submission_id
        || unit.entry_stage_kind != "vuln_triage"
        || unit.ordinal != input.ordinal
    {
        return Err(crate::DbError::Other(anyhow::anyhow!(
            "attack wave-unit replay drift"
        )));
    }
    Ok((wave, unit))
}

pub async fn lock_wave(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    wave_run_id: Uuid,
) -> crate::Result<AttackWaveRunRow> {
    let sql = format!(
        "SELECT {WAVE_COLUMNS} FROM attack_wave_runs
         WHERE id=$1 AND operation_id=$2 AND scope_snapshot_id=$3 FOR UPDATE"
    );
    sqlx::query_as(&sql)
        .bind(wave_run_id)
        .bind(operation_id)
        .bind(scope_snapshot_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| crate::DbError::NotFound("attack_wave_run".to_string()))
}

pub async fn lock_wave_unit(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    wave_run_id: Uuid,
    wave_unit_id: Uuid,
    organization_id: Uuid,
) -> crate::Result<AttackWaveUnitRow> {
    let sql = format!(
        "SELECT {UNIT_COLUMNS} FROM attack_wave_units
         WHERE id=$1 AND wave_run_id=$2 AND operation_id=$3
           AND scope_snapshot_id=$4 AND organization_id=$5 FOR UPDATE"
    );
    sqlx::query_as(&sql)
        .bind(wave_unit_id)
        .bind(wave_run_id)
        .bind(operation_id)
        .bind(scope_snapshot_id)
        .bind(organization_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| crate::DbError::NotFound("attack_wave_unit".to_string()))
}

pub async fn set_review_closed(
    tx: &mut Transaction<'_, Postgres>,
    wave_unit: &AttackWaveUnitRow,
    closed: bool,
) -> crate::Result<AttackWaveUnitRow> {
    let sql = format!(
        "UPDATE attack_wave_units SET review_closed=$2,
             status=CASE WHEN $2 THEN 'verification' ELSE 'review' END,
             row_version=row_version+1,updated_at=NOW()
         WHERE id=$1 AND row_version=$3 RETURNING {UNIT_COLUMNS}"
    );
    sqlx::query_as(&sql)
        .bind(wave_unit.id)
        .bind(closed)
        .bind(wave_unit.row_version)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| crate::DbError::Other(anyhow::anyhow!("stale attack wave unit")))
}
