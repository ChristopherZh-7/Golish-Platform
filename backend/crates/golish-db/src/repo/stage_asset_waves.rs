//! Repository for durable stage asset waves.
//!
//! A wave freezes the target set for one `(operation, organization, stage)` run.
//! New targets discovered while that wave is running are left in `targets`, but
//! become candidates for a later wave instead of moving the current gate
//! denominator.

use crate::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct StageAssetWaveRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub stage_kind: String,
    pub wave_index: i32,
    pub status: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub parent_wave_id: Option<Uuid>,
    pub asset_hash: String,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct StageAssetWaveItemRow {
    pub id: i64,
    pub wave_id: Uuid,
    pub target_id: Uuid,
    pub asset_value: String,
    pub asset_type: String,
    pub source: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct StageAssetWaveWithItems {
    pub wave: StageAssetWaveRow,
    pub items: Vec<StageAssetWaveItemRow>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct WaveTargetCandidate {
    target_id: Uuid,
    asset_value: String,
    asset_type: String,
    source: Option<String>,
}

const WAVE_ROW_COLS: &str = "id, operation_id, organization_id, stage_kind, wave_index, status, started_at, completed_at, parent_wave_id, asset_hash";
const ITEM_ROW_COLS: &str = "id, wave_id, target_id, asset_value, asset_type, source, created_at";

fn build_current_running_sql() -> String {
    format!(
        "SELECT {WAVE_ROW_COLS} FROM stage_asset_waves \
         WHERE operation_id = $1 \
           AND organization_id = $2 \
           AND stage_kind = $3 \
           AND status = 'running' \
         ORDER BY wave_index DESC \
         LIMIT 1"
    )
}

fn build_latest_wave_sql() -> String {
    format!(
        "SELECT {WAVE_ROW_COLS} FROM stage_asset_waves \
         WHERE operation_id = $1 \
           AND organization_id = $2 \
           AND stage_kind = $3 \
         ORDER BY wave_index DESC \
         LIMIT 1"
    )
}

fn build_items_for_wave_sql() -> String {
    format!("SELECT {ITEM_ROW_COLS} FROM stage_asset_wave_items WHERE wave_id = $1 ORDER BY id")
}

fn build_initial_candidates_sql() -> String {
    "SELECT id AS target_id, value AS asset_value, target_type::text AS asset_type, source \
       FROM targets \
      WHERE scope::text = 'in' \
        AND organization_id = $1 \
        AND created_at <= $2 \
      ORDER BY created_at ASC, value ASC, id ASC \
      LIMIT $3"
        .to_string()
}

fn build_next_candidates_sql() -> String {
    "SELECT t.id AS target_id, t.value AS asset_value, t.target_type::text AS asset_type, t.source \
       FROM targets t \
      WHERE t.scope::text = 'in' \
        AND t.organization_id = $2 \
        AND NOT EXISTS ( \
            SELECT 1 \
              FROM stage_asset_wave_items i \
              JOIN stage_asset_waves w ON w.id = i.wave_id \
             WHERE w.operation_id = $1 \
               AND w.organization_id = $2 \
               AND w.stage_kind = $3 \
               AND i.target_id = t.id \
        ) \
      ORDER BY t.created_at ASC, t.value ASC, t.id ASC \
      LIMIT $4"
        .to_string()
}

fn build_next_wave_index_sql() -> String {
    "SELECT COALESCE(MAX(wave_index), -1) + 1 \
       FROM stage_asset_waves \
      WHERE operation_id = $1 \
        AND organization_id = $2 \
        AND stage_kind = $3"
        .to_string()
}

fn build_insert_wave_sql() -> String {
    format!(
        "INSERT INTO stage_asset_waves \
             (operation_id, organization_id, stage_kind, wave_index, status, started_at, parent_wave_id, asset_hash, updated_at) \
         VALUES ($1, $2, $3, $4, 'running', $5, $6, $7, NOW()) \
         RETURNING {WAVE_ROW_COLS}"
    )
}

fn build_insert_item_sql() -> String {
    format!(
        "INSERT INTO stage_asset_wave_items \
             (wave_id, target_id, asset_value, asset_type, source) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (wave_id, target_id) DO UPDATE SET \
             asset_value = EXCLUDED.asset_value, \
             asset_type = EXCLUDED.asset_type, \
             source = EXCLUDED.source \
         RETURNING {ITEM_ROW_COLS}"
    )
}

fn stable_asset_hash(candidates: &[WaveTargetCandidate]) -> String {
    let mut parts: Vec<String> = candidates
        .iter()
        .map(|c| {
            format!(
                "{}\x1f{}\x1f{}\x1f{}",
                c.target_id,
                c.asset_value,
                c.asset_type,
                c.source.as_deref().unwrap_or("")
            )
        })
        .collect();
    parts.sort();

    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for part in parts {
        for byte in part.as_bytes().iter().chain(std::iter::once(&0)) {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    format!("{hash:016x}")
}

pub async fn current_running(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
    stage_kind: &str,
) -> Result<Option<StageAssetWaveWithItems>> {
    let Some(wave) = sqlx::query_as::<_, StageAssetWaveRow>(&build_current_running_sql())
        .bind(operation_id)
        .bind(organization_id)
        .bind(stage_kind)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };
    let items = list_items(pool, wave.id).await?;
    Ok(Some(StageAssetWaveWithItems { wave, items }))
}

async fn latest_wave(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
    stage_kind: &str,
) -> Result<Option<StageAssetWaveRow>> {
    let row = sqlx::query_as::<_, StageAssetWaveRow>(&build_latest_wave_sql())
        .bind(operation_id)
        .bind(organization_id)
        .bind(stage_kind)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

pub async fn list_items(pool: &PgPool, wave_id: Uuid) -> Result<Vec<StageAssetWaveItemRow>> {
    let rows = sqlx::query_as::<_, StageAssetWaveItemRow>(&build_items_for_wave_sql())
        .bind(wave_id)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

pub async fn current_or_create_initial(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
    stage_kind: &str,
    started_at: DateTime<Utc>,
    limit: i64,
) -> Result<Option<StageAssetWaveWithItems>> {
    if let Some(wave) = current_running(pool, operation_id, organization_id, stage_kind).await? {
        return Ok(Some(wave));
    }
    if let Some(latest) = latest_wave(pool, operation_id, organization_id, stage_kind).await? {
        return create_next(
            pool,
            operation_id,
            organization_id,
            stage_kind,
            Some(latest.id),
            limit,
        )
        .await;
    }

    let candidates = sqlx::query_as::<_, WaveTargetCandidate>(&build_initial_candidates_sql())
        .bind(organization_id)
        .bind(started_at)
        .bind(limit.max(0))
        .fetch_all(pool)
        .await?;
    insert_wave_with_items(
        pool,
        operation_id,
        organization_id,
        stage_kind,
        0,
        started_at,
        None,
        candidates,
    )
    .await
}

pub async fn create_next(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
    stage_kind: &str,
    parent_wave_id: Option<Uuid>,
    limit: i64,
) -> Result<Option<StageAssetWaveWithItems>> {
    let candidates = sqlx::query_as::<_, WaveTargetCandidate>(&build_next_candidates_sql())
        .bind(operation_id)
        .bind(organization_id)
        .bind(stage_kind)
        .bind(limit.max(0))
        .fetch_all(pool)
        .await?;

    let wave_index = sqlx::query_scalar::<_, i32>(&build_next_wave_index_sql())
        .bind(operation_id)
        .bind(organization_id)
        .bind(stage_kind)
        .fetch_one(pool)
        .await?;

    insert_wave_with_items(
        pool,
        operation_id,
        organization_id,
        stage_kind,
        wave_index,
        Utc::now(),
        parent_wave_id,
        candidates,
    )
    .await
}

async fn insert_wave_with_items(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
    stage_kind: &str,
    wave_index: i32,
    started_at: DateTime<Utc>,
    parent_wave_id: Option<Uuid>,
    candidates: Vec<WaveTargetCandidate>,
) -> Result<Option<StageAssetWaveWithItems>> {
    if candidates.is_empty() {
        return Ok(None);
    }

    let asset_hash = stable_asset_hash(&candidates);
    let mut tx = pool.begin().await?;
    let wave = sqlx::query_as::<_, StageAssetWaveRow>(&build_insert_wave_sql())
        .bind(operation_id)
        .bind(organization_id)
        .bind(stage_kind)
        .bind(wave_index)
        .bind(started_at)
        .bind(parent_wave_id)
        .bind(asset_hash)
        .fetch_one(&mut *tx)
        .await?;

    let mut items = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let row = sqlx::query_as::<_, StageAssetWaveItemRow>(&build_insert_item_sql())
            .bind(wave.id)
            .bind(candidate.target_id)
            .bind(candidate.asset_value)
            .bind(candidate.asset_type)
            .bind(candidate.source)
            .fetch_one(&mut *tx)
            .await?;
        items.push(row);
    }
    tx.commit().await?;
    Ok(Some(StageAssetWaveWithItems { wave, items }))
}

pub async fn complete(pool: &PgPool, wave_id: Uuid) -> Result<()> {
    sqlx::query(
        "UPDATE stage_asset_waves \
            SET status = 'completed', \
                completed_at = COALESCE(completed_at, NOW()), \
                updated_at = NOW() \
          WHERE id = $1",
    )
    .bind(wave_id)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(target_id: u128, value: &str) -> WaveTargetCandidate {
        WaveTargetCandidate {
            target_id: Uuid::from_u128(target_id),
            asset_value: value.to_string(),
            asset_type: "domain".to_string(),
            source: Some("test".to_string()),
        }
    }

    #[test]
    fn stage_asset_wave_hash_is_order_independent_and_content_bound() {
        let a = candidate(1, "a.example.com");
        let b = candidate(2, "b.example.com");

        assert_eq!(
            stable_asset_hash(&[a.clone(), b.clone()]),
            stable_asset_hash(&[b, a.clone()])
        );
        assert_ne!(
            stable_asset_hash(&[a]),
            stable_asset_hash(&[candidate(1, "changed.example.com")])
        );
    }

    #[test]
    fn stage_asset_wave_next_candidates_exclude_prior_wave_items() {
        let sql = build_next_candidates_sql();
        assert!(sql.contains("NOT EXISTS"));
        assert!(sql.contains("stage_asset_wave_items"));
        assert!(sql.contains("w.operation_id = $1"));
        assert!(sql.contains("w.organization_id = $2"));
        assert!(sql.contains("w.stage_kind = $3"));
    }

    #[test]
    fn stage_asset_wave_initial_candidates_freeze_at_started_at() {
        let sql = build_initial_candidates_sql();
        assert!(sql.contains("created_at <= $2"));
        assert!(sql.contains("scope::text = 'in'"));
        assert!(sql.contains("organization_id = $1"));
    }
}
