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
const CLOSE_BARRIER_WAVE_LOCK_SQL: &str =
    "LOCK TABLE stage_asset_waves IN SHARE ROW EXCLUSIVE MODE";
const CLOSE_BARRIER_TARGET_LOCK_SQL: &str = "LOCK TABLE targets IN SHARE MODE";

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

fn build_all_items_created_at_or_before_sql() -> String {
    "SELECT COUNT(*) > 0 \
            AND BOOL_AND(t.created_at IS NOT NULL AND t.created_at <= $2) \
       FROM stage_asset_wave_items i \
       LEFT JOIN targets t ON t.id = i.target_id \
      WHERE i.wave_id = $1"
        .to_string()
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

fn build_sealed_completion_upsert_sql() -> &'static str {
    "INSERT INTO org_stage_completions \
         (organization_id, stage_kind, passed_at, stage_run_id, updated_at) \
     VALUES ($1, $2, statement_timestamp(), $3, statement_timestamp()) \
     ON CONFLICT (organization_id, stage_kind) DO UPDATE SET \
         passed_at = statement_timestamp(), \
         stage_run_id = EXCLUDED.stage_run_id, \
         updated_at = statement_timestamp()"
}

fn effective_parent_wave_id(
    requested_parent_wave_id: Option<Uuid>,
    latest_wave_id: Option<Uuid>,
) -> Option<Uuid> {
    requested_parent_wave_id.or(latest_wave_id)
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

fn stable_wave_item_hash(items: &[StageAssetWaveItemRow]) -> String {
    let candidates = items
        .iter()
        .map(|item| WaveTargetCandidate {
            target_id: item.target_id,
            asset_value: item.asset_value.clone(),
            asset_type: item.asset_type.clone(),
            source: item.source.clone(),
        })
        .collect::<Vec<_>>();
    stable_asset_hash(&candidates)
}

fn validate_wave_items(wave: &StageAssetWaveRow, items: &[StageAssetWaveItemRow]) -> Result<()> {
    if items.is_empty() {
        return Err(anyhow::anyhow!(
            "running asset wave {} has no items; refusing denominator fallback",
            wave.id
        )
        .into());
    }
    let actual_hash = stable_wave_item_hash(items);
    if actual_hash != wave.asset_hash {
        return Err(anyhow::anyhow!(
            "running asset wave {} item hash mismatch: stored={}, actual={}; wave items may have been deleted",
            wave.id,
            wave.asset_hash,
            actual_hash
        )
        .into());
    }
    Ok(())
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
    validate_wave_items(&wave, &items)?;
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

pub async fn all_items_created_at_or_before(
    pool: &PgPool,
    wave_id: Uuid,
    cutoff: DateTime<Utc>,
) -> Result<bool> {
    let covered = sqlx::query_scalar::<_, bool>(&build_all_items_created_at_or_before_sql())
        .bind(wave_id)
        .bind(cutoff)
        .fetch_one(pool)
        .await?;
    Ok(covered)
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

/// Atomically decide whether a wave-aware stage has more work or can publish
/// its per-org completion watermark.
///
/// The table locks deliberately make the final unassigned-target read and the
/// completion-ledger write one serialization point. A target writer that was
/// already in flight completes before the candidate SELECT; a later writer is
/// ordered after the durable completion watermark and belongs to a later
/// operation/stage lifecycle. This closes the SELECT-empty -> pass-token race
/// without changing the schema.
pub async fn create_next_or_seal_completion(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
    stage_kind: &str,
    parent_wave_id: Option<Uuid>,
    limit: i64,
    stage_run_id: Option<&str>,
) -> Result<Option<StageAssetWaveWithItems>> {
    let mut tx = pool.begin().await?;
    sqlx::query(CLOSE_BARRIER_WAVE_LOCK_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CLOSE_BARRIER_TARGET_LOCK_SQL)
        .execute(&mut *tx)
        .await?;

    if let Some(wave) = sqlx::query_as::<_, StageAssetWaveRow>(&build_current_running_sql())
        .bind(operation_id)
        .bind(organization_id)
        .bind(stage_kind)
        .fetch_optional(&mut *tx)
        .await?
    {
        let items = sqlx::query_as::<_, StageAssetWaveItemRow>(&build_items_for_wave_sql())
            .bind(wave.id)
            .fetch_all(&mut *tx)
            .await?;
        validate_wave_items(&wave, &items)?;
        tx.commit().await?;
        return Ok(Some(StageAssetWaveWithItems { wave, items }));
    }

    let candidates = sqlx::query_as::<_, WaveTargetCandidate>(&build_next_candidates_sql())
        .bind(operation_id)
        .bind(organization_id)
        .bind(stage_kind)
        .bind(limit.max(0))
        .fetch_all(&mut *tx)
        .await?;

    if candidates.is_empty() {
        sqlx::query(build_sealed_completion_upsert_sql())
            .bind(organization_id)
            .bind(stage_kind)
            .bind(stage_run_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Ok(None);
    }

    // Resume-compatible backlog repair may arrive without the just-completed
    // wave in the runtime's request-local map (for example, a fresh legacy
    // completion with no running wave). If this operation already has any wave,
    // the new batch is necessarily supplemental and must carry that parent;
    // otherwise the legacy parentless resume rule could skip the backlog.
    let latest_wave_id = if parent_wave_id.is_none() {
        sqlx::query_as::<_, StageAssetWaveRow>(&build_latest_wave_sql())
            .bind(operation_id)
            .bind(organization_id)
            .bind(stage_kind)
            .fetch_optional(&mut *tx)
            .await?
            .map(|wave| wave.id)
    } else {
        None
    };
    let effective_parent_wave_id = effective_parent_wave_id(parent_wave_id, latest_wave_id);

    let wave_index = sqlx::query_scalar::<_, i32>(&build_next_wave_index_sql())
        .bind(operation_id)
        .bind(organization_id)
        .bind(stage_kind)
        .fetch_one(&mut *tx)
        .await?;
    let started_at = Utc::now();
    let asset_hash = stable_asset_hash(&candidates);
    let wave = sqlx::query_as::<_, StageAssetWaveRow>(&build_insert_wave_sql())
        .bind(operation_id)
        .bind(organization_id)
        .bind(stage_kind)
        .bind(wave_index)
        .bind(started_at)
        .bind(effective_parent_wave_id)
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

    fn item(target_id: u128, value: &str) -> StageAssetWaveItemRow {
        StageAssetWaveItemRow {
            id: target_id as i64,
            wave_id: Uuid::from_u128(99),
            target_id: Uuid::from_u128(target_id),
            asset_value: value.to_string(),
            asset_type: "domain".to_string(),
            source: Some("test".to_string()),
            created_at: Utc::now(),
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
    fn stage_asset_wave_hash_detects_cascade_deleted_item() {
        let full = vec![item(1, "a.example.com"), item(2, "b.example.com")];
        let stored_hash = stable_wave_item_hash(&full);

        let remaining_hash = stable_wave_item_hash(&full[1..]);

        assert_ne!(stored_hash, remaining_hash);
        let wave = StageAssetWaveRow {
            id: Uuid::from_u128(90),
            operation_id: Uuid::from_u128(91),
            organization_id: Uuid::from_u128(92),
            stage_kind: "enumeration".to_string(),
            wave_index: 0,
            status: "running".to_string(),
            started_at: Utc::now(),
            completed_at: None,
            parent_wave_id: None,
            asset_hash: stored_hash,
        };
        assert!(validate_wave_items(&wave, &full[1..]).is_err());
    }

    #[test]
    fn stage_asset_wave_next_candidates_exclude_prior_wave_items() {
        let sql = build_next_candidates_sql();
        assert!(sql.contains("NOT EXISTS"));
        assert!(sql.contains("stage_asset_wave_items"));
        assert!(sql.contains("w.operation_id = $1"));
        assert!(sql.contains("w.organization_id = $2"));
        assert!(sql.contains("w.stage_kind = $3"));
        assert!(sql.contains("LIMIT $4"));
    }

    #[test]
    fn stage_asset_wave_next_candidates_include_unassigned_backlog_without_time_floor() {
        let sql = build_next_candidates_sql();
        assert!(!sql.contains("parent.started_at"));
        assert!(!sql.contains("org_stage_completions"));
        assert!(!sql.contains("t.created_at >"));
        assert!(sql.contains("ORDER BY t.created_at ASC, t.value ASC, t.id ASC"));
    }

    #[test]
    fn stage_asset_wave_close_barrier_serializes_target_writers_and_completion() {
        assert!(CLOSE_BARRIER_WAVE_LOCK_SQL.contains("stage_asset_waves"));
        assert!(CLOSE_BARRIER_TARGET_LOCK_SQL.contains("targets IN SHARE MODE"));
        let completion = build_sealed_completion_upsert_sql();
        assert!(completion.contains("org_stage_completions"));
        assert!(completion.contains("statement_timestamp()"));
        assert!(completion.contains("ON CONFLICT (organization_id, stage_kind)"));
    }

    #[test]
    fn stage_asset_wave_backlog_without_request_parent_attaches_to_latest_wave() {
        let latest = Uuid::from_u128(41);
        let explicit = Uuid::from_u128(42);
        assert_eq!(effective_parent_wave_id(None, Some(latest)), Some(latest));
        assert_eq!(
            effective_parent_wave_id(Some(explicit), Some(latest)),
            Some(explicit),
            "the caller's current completed wave remains authoritative"
        );
        assert_eq!(effective_parent_wave_id(None, None), None);
    }

    #[test]
    fn stage_asset_wave_item_coverage_query_requires_all_targets_before_cutoff() {
        let sql = build_all_items_created_at_or_before_sql();
        assert!(sql.contains("COUNT(*) > 0"));
        assert!(sql.contains("BOOL_AND"));
        assert!(sql.contains("t.created_at <= $2"));
        assert!(sql.contains("LEFT JOIN targets"));
    }

    #[test]
    fn stage_asset_wave_initial_candidates_freeze_at_started_at() {
        let sql = build_initial_candidates_sql();
        assert!(sql.contains("created_at <= $2"));
        assert!(sql.contains("scope::text = 'in'"));
        assert!(sql.contains("organization_id = $1"));
    }
}
