//! `technique_outcomes` 物化表读写（#4 / E3，设计
//! `docs/design/2026-06-23-technique-outcomes-provenance.md`）。
//!
//! coverage gate 的单一真值源 + provenance：每 `(org × run × asset × technique)` 一行，
//! 带 outcome + source/query/confidence/evidence_ids/collected_at。命令路径与
//! enrich/landing 落库点都 **upsert** 这里（PR-C step2 写路径）；gate 后续从这里投影
//! `EvidenceFact`（PR-D 读路径，灰度）。
//!
//! 纯 runtime sqlx（无 `query!` 宏 → 无需编译期 DB）；SQL 抽成 `const` 便于单测。
//! I2：一切读写按 `organization_id` 过滤。I8：`outcome=empty` 只来自真「跑了→空」；
//! 缺行 = not_attempted（gate 照旧 BLOCK）。I7：`evidence_ids` 指 audit_log 真实行。

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{Executor, PgConnection, PgPool, Postgres};
use uuid::Uuid;

use super::scoped::{lock_target_write_guard, TargetWriteGuard};

/// Trusted stage-attempt and per-origin generation witness used when a
/// long-running producer replaces its non-terminal attempt markers with terminal
/// sibling outcomes. The generation is stored in `technique_outcomes.query` so a
/// newer attempt can invalidate an older in-flight request without holding a DB
/// transaction across network I/O.
#[derive(Debug, Clone)]
pub struct TechniqueOutcomeAttemptGuard {
    pub operation_id: Uuid,
    pub stage: String,
    pub stage_started_at: DateTime<Utc>,
    pub engagement_org_id: Option<Uuid>,
    pub organization_id: Uuid,
    pub source: String,
    pub generation: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionalBatchUpsertResult {
    Applied,
    Superseded,
}

fn state_slot_clear_outcome_is_terminal(outcome: &str) -> bool {
    matches!(outcome, "found" | "empty" | "blocked")
}

/// 一条 technique_outcome 的写入参数（provenance 全量）。`asset` 必须由调用方先过
/// `canonical_asset_key().key` 归一（E1），否则 gate join 漂移。
#[derive(Debug, Clone)]
pub struct TechniqueOutcomeWrite {
    pub organization_id: Uuid,
    pub run_id: String,
    pub asset: String,
    pub technique: String,
    /// `found` | `empty` | `partial` | `error` | `blocked` | `not_applicable`
    /// （与 producer/gate 合同对齐）。
    pub outcome: String,
    pub source: Option<String>,
    pub query: Option<String>,
    pub result_count: Option<i32>,
    pub confidence: Option<f32>,
    pub evidence_ids: Vec<i64>,
    pub collected_at: Option<DateTime<Utc>>,
}

/// gate 投影 / 诊断读出的一行（PR-D 用）。
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TechniqueOutcomeRow {
    pub asset: String,
    pub technique: String,
    pub outcome: String,
    pub source: Option<String>,
    pub evidence_ids: Vec<i64>,
    pub collected_at: Option<DateTime<Utc>>,
}

/// upsert：`UNIQUE(organization_id, run_id, asset, technique)` 冲突 → 更新 outcome/provenance/
/// evidence_ids/collected_at/updated_at，**seq 保持首插值**（幂等不堆叠）。首插
/// `seq = COALESCE(MAX(seq),0)+1 WHERE organization_id + run_id`（D2：每 org/run
/// 从 1 自增；并发以 UNIQUE 兜底，seq 仅排序提示）。
const UPSERT_SQL: &str = "\
INSERT INTO technique_outcomes \
  (organization_id, run_id, asset, technique, outcome, source, query, \
   result_count, confidence, evidence_ids, seq, collected_at) \
VALUES \
  ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, \
   (SELECT COALESCE(MAX(seq), 0) + 1 FROM technique_outcomes \
     WHERE organization_id = $1 AND run_id = $2), \
   $11) \
ON CONFLICT (organization_id, run_id, asset, technique) DO UPDATE SET \
  outcome = EXCLUDED.outcome, \
  source = EXCLUDED.source, \
  query = EXCLUDED.query, \
  result_count = EXCLUDED.result_count, \
  confidence = EXCLUDED.confidence, \
  evidence_ids = EXCLUDED.evidence_ids, \
  collected_at = EXCLUDED.collected_at, \
  updated_at = NOW()";

/// Gate-PASS terminal materialization. A model-authored `blocked` /
/// `not_applicable` cell may fill a missing or unfinished slot, but must never
/// downgrade producer-owned `found` / `empty` or an already-terminal exception.
/// The conflict predicate makes the snapshot-check → write race safe.
const UPSERT_TERMINAL_IF_UNFINISHED_SQL: &str = "\
INSERT INTO technique_outcomes \
  (organization_id, run_id, asset, technique, outcome, source, query, \
   result_count, confidence, evidence_ids, seq, collected_at) \
VALUES \
  ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, \
   (SELECT COALESCE(MAX(seq), 0) + 1 FROM technique_outcomes \
     WHERE organization_id = $1 AND run_id = $2), \
   $11) \
ON CONFLICT (organization_id, run_id, asset, technique) DO UPDATE SET \
  outcome = EXCLUDED.outcome, \
  source = EXCLUDED.source, \
  query = EXCLUDED.query, \
  result_count = EXCLUDED.result_count, \
  confidence = EXCLUDED.confidence, \
  evidence_ids = EXCLUDED.evidence_ids, \
  collected_at = EXCLUDED.collected_at, \
  updated_at = NOW() \
WHERE technique_outcomes.outcome IN ('partial', 'error')";

/// Attempt-start marker publication is monotonic: a retry may replace a prior
/// unfinished marker, but it must never demote durable producer truth. The
/// caller treats a zero-row conflict update as a superseded attempt and rolls
/// back the whole sibling batch before any scanner work starts.
const UPSERT_ATTEMPT_MARKER_IF_UNFINISHED_SQL: &str = "\
INSERT INTO technique_outcomes \
  (organization_id, run_id, asset, technique, outcome, source, query, \
   result_count, confidence, evidence_ids, seq, collected_at) \
VALUES \
  ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, \
   (SELECT COALESCE(MAX(seq), 0) + 1 FROM technique_outcomes \
     WHERE organization_id = $1 AND run_id = $2), \
   $11) \
ON CONFLICT (organization_id, run_id, asset, technique) DO UPDATE SET \
  outcome = EXCLUDED.outcome, \
  source = EXCLUDED.source, \
  query = EXCLUDED.query, \
  result_count = EXCLUDED.result_count, \
  confidence = EXCLUDED.confidence, \
  evidence_ids = EXCLUDED.evidence_ids, \
  collected_at = EXCLUDED.collected_at, \
  updated_at = NOW() \
WHERE technique_outcomes.outcome IN ('partial', 'error')";

/// Epoch-guarded stage attempts may replace terminal truth only when it is
/// older than the current stage epoch. This is the append-only replacement
/// execution case: the old evidence remains in the ledger, while the mutable
/// coverage cell becomes an unfinished marker for the fresh stage execution.
/// A terminal result collected in the current epoch still wins the race.
const UPSERT_ATTEMPT_MARKER_IF_UNFINISHED_OR_STALE_SQL: &str = "\
INSERT INTO technique_outcomes \
  (organization_id, run_id, asset, technique, outcome, source, query, \
   result_count, confidence, evidence_ids, seq, collected_at) \
VALUES \
  ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, \
   (SELECT COALESCE(MAX(seq), 0) + 1 FROM technique_outcomes \
     WHERE organization_id = $1 AND run_id = $2), \
   $11) \
ON CONFLICT (organization_id, run_id, asset, technique) DO UPDATE SET \
  outcome = EXCLUDED.outcome, \
  source = EXCLUDED.source, \
  query = EXCLUDED.query, \
  result_count = EXCLUDED.result_count, \
  confidence = EXCLUDED.confidence, \
  evidence_ids = EXCLUDED.evidence_ids, \
  collected_at = EXCLUDED.collected_at, \
  updated_at = NOW() \
WHERE technique_outcomes.outcome IN ('partial', 'error') \
   OR technique_outcomes.collected_at IS NULL \
   OR technique_outcomes.collected_at < $12";

/// 读某 run 的全部维（org 隔离，IDOR）。`seq` 只是并发写入下的排序提示；
/// asset/technique 是确定性 tie-breaker，避免两个并发首插拿到同一 seq 时读序漂移。
const LIST_FOR_RUN_SQL: &str = "\
SELECT asset, technique, outcome, source, evidence_ids, collected_at \
FROM technique_outcomes \
WHERE organization_id = $1 AND run_id = $2 \
ORDER BY seq, asset, technique";

/// 同 `LIST_FOR_RUN_SQL`，但套 freshness cutoff（护栏 4，设计
/// `docs/superpowers/plans/2026-07-02-gate-capability-ledger.md` Phase 1）：
/// `$3 IS NULL` → 旧的 presence-only 行为；`$3 = Some(cutoff)` → 只算本 stage-run
/// 采集的行（`collected_at >= cutoff`）。`collected_at` 为 NULL 的行在 `$3=Some`
/// 时被排除（保守，对齐 `db_truth_facts` 的 `>= cutoff` NULL→false 语义）。
const LIST_FOR_RUN_FRESH_SQL: &str = "\
SELECT asset, technique, outcome, source, evidence_ids, collected_at \
FROM technique_outcomes \
WHERE organization_id = $1 AND run_id = $2 \
  AND ($3::timestamptz IS NULL OR collected_at >= $3) \
ORDER BY seq, asset, technique";

/// upsert 一条 technique_outcome（PR-C step2 写路径）。
pub async fn upsert(pool: &PgPool, w: &TechniqueOutcomeWrite) -> Result<()> {
    execute_upsert(pool, w).await
}

/// Insert a gate-approved `blocked` / `not_applicable` terminal cell only when
/// no row exists, or replace an unfinished `partial` / `error` row. Returns
/// `true` when the row was inserted/updated and `false` when an existing terminal
/// producer/gate row won the race.
pub async fn upsert_terminal_if_unfinished(
    pool: &PgPool,
    w: &TechniqueOutcomeWrite,
) -> Result<bool> {
    if !matches!(w.outcome.as_str(), "blocked" | "not_applicable") {
        return Err(anyhow::anyhow!(
            "conditional gate terminal write accepts only blocked/not_applicable"
        ));
    }
    let result = sqlx::query(UPSERT_TERMINAL_IF_UNFINISHED_SQL)
        .bind(w.organization_id)
        .bind(&w.run_id)
        .bind(&w.asset)
        .bind(&w.technique)
        .bind(&w.outcome)
        .bind(w.source.as_deref())
        .bind(w.query.as_deref())
        .bind(w.result_count)
        .bind(w.confidence)
        .bind(w.evidence_ids.as_slice())
        .bind(w.collected_at)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() == 1)
}

async fn execute_upsert<'e, E>(executor: E, w: &TechniqueOutcomeWrite) -> Result<()>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query(UPSERT_SQL)
        .bind(w.organization_id)
        .bind(&w.run_id)
        .bind(&w.asset)
        .bind(&w.technique)
        .bind(&w.outcome)
        .bind(w.source.as_deref())
        .bind(w.query.as_deref())
        .bind(w.result_count)
        .bind(w.confidence)
        .bind(w.evidence_ids.as_slice())
        .bind(w.collected_at)
        .execute(executor)
        .await?;
    Ok(())
}

async fn execute_attempt_marker<'e, E>(executor: E, w: &TechniqueOutcomeWrite) -> Result<bool>
where
    E: Executor<'e, Database = Postgres>,
{
    let result = sqlx::query(UPSERT_ATTEMPT_MARKER_IF_UNFINISHED_SQL)
        .bind(w.organization_id)
        .bind(&w.run_id)
        .bind(&w.asset)
        .bind(&w.technique)
        .bind(&w.outcome)
        .bind(w.source.as_deref())
        .bind(w.query.as_deref())
        .bind(w.result_count)
        .bind(w.confidence)
        .bind(w.evidence_ids.as_slice())
        .bind(w.collected_at)
        .execute(executor)
        .await?;
    Ok(result.rows_affected() == 1)
}

async fn execute_epoch_attempt_marker<'e, E>(
    executor: E,
    w: &TechniqueOutcomeWrite,
    stage_started_at: DateTime<Utc>,
) -> Result<bool>
where
    E: Executor<'e, Database = Postgres>,
{
    let result = sqlx::query(UPSERT_ATTEMPT_MARKER_IF_UNFINISHED_OR_STALE_SQL)
        .bind(w.organization_id)
        .bind(&w.run_id)
        .bind(&w.asset)
        .bind(&w.technique)
        .bind(&w.outcome)
        .bind(w.source.as_deref())
        .bind(w.query.as_deref())
        .bind(w.result_count)
        .bind(w.confidence)
        .bind(w.evidence_ids.as_slice())
        .bind(w.collected_at)
        .bind(stage_started_at)
        .execute(executor)
        .await?;
    Ok(result.rows_affected() == 1)
}

/// Atomically upsert several outcomes. Enumeration producers use this for the
/// attempt-start `partial` markers across sibling axes: cancellation must never
/// update only one cell and leave another cell's prior terminal value behind.
pub async fn upsert_batch(pool: &PgPool, writes: &[TechniqueOutcomeWrite]) -> Result<()> {
    if writes.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for write in writes {
        execute_upsert(&mut *tx, write).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Atomically upsert a target producer's outcome group only while the exact
/// target authorization witness is unchanged. Outcome rows do not carry a
/// target id, so the organization must also match the locked target owner.
pub async fn upsert_batch_guarded(
    pool: &PgPool,
    guard: &TargetWriteGuard,
    writes: &[TechniqueOutcomeWrite],
) -> Result<()> {
    if writes.is_empty() {
        return Ok(());
    }
    let Some(organization_id) = guard.organization_id else {
        return Err(anyhow::anyhow!(
            "target-bound technique outcomes require an organization"
        ));
    };
    if writes
        .iter()
        .any(|write| write.organization_id != organization_id)
    {
        return Err(anyhow::anyhow!(
            "technique outcome organization does not match target write guard"
        ));
    }

    let mut tx = pool.begin().await?;
    lock_target_write_guard(&mut tx, guard).await?;
    for write in writes {
        execute_upsert(&mut *tx, write).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Target-guarded producer publication that may advance an unfinished cell but
/// never replace an existing terminal cell. EAS port discovery uses this so a
/// later quick/standard observation cannot demote prior full-scan truth.
pub async fn upsert_batch_guarded_monotonic(
    pool: &PgPool,
    guard: &TargetWriteGuard,
    writes: &[TechniqueOutcomeWrite],
) -> Result<()> {
    if writes.is_empty() {
        return Ok(());
    }
    let Some(organization_id) = guard.organization_id else {
        return Err(anyhow::anyhow!(
            "target-bound technique outcomes require an organization"
        ));
    };
    if writes
        .iter()
        .any(|write| write.organization_id != organization_id)
    {
        return Err(anyhow::anyhow!(
            "technique outcome organization does not match target write guard"
        ));
    }

    let mut tx = pool.begin().await?;
    lock_target_write_guard(&mut tx, guard).await?;
    for write in writes {
        let _applied = execute_attempt_marker(&mut *tx, write).await?;
    }
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct AttemptStateRow {
    technique: String,
    outcome: String,
    source: Option<String>,
    query: Option<String>,
}

const LOCK_OPERATION_EPOCH_SQL: &str = r#"SELECT 1
FROM operation_state
WHERE operation_id = $1
  AND current_stage = $2
  AND stage_started_at = $3
  AND superseded_by IS NULL
  AND engagement_org_id IS NOT DISTINCT FROM $4
FOR UPDATE"#;

const ACTIVE_ORG_IN_ENGAGEMENT_SUBTREE_SQL: &str = r#"WITH RECURSIVE subtree AS (
  SELECT id FROM organizations WHERE id = $1
  UNION ALL
  SELECT o.id FROM organizations o JOIN subtree s ON o.parent_id = s.id
)
SELECT EXISTS(SELECT 1 FROM subtree WHERE id = $2)"#;

const LOCK_ATTEMPT_STATE_SQL: &str = r#"SELECT technique, outcome, source, query
FROM technique_outcomes
WHERE organization_id = $1
  AND run_id = $2
  AND asset = $3
  AND technique = ANY($4::text[])
FOR UPDATE"#;

const CLEAR_OPERATION_STATE_SLOT_SQL: &str = r#"UPDATE operation_state
SET state_blob = jsonb_set(
    COALESCE(state_blob, '{}'::jsonb),
    ARRAY[$2]::text[],
    COALESCE(state_blob -> $2, '{}'::jsonb) - $3,
    true
)
WHERE operation_id = $1
  AND current_stage = $4
  AND stage_started_at = $5
  AND superseded_by IS NULL
  AND engagement_org_id IS NOT DISTINCT FROM $6"#;

fn attempt_generation_matches(
    rows: &[AttemptStateRow],
    expected_techniques: &[String],
    source: &str,
    generation: &str,
) -> bool {
    if rows.len() != expected_techniques.len() {
        return false;
    }
    let mut observed = rows
        .iter()
        .filter(|row| {
            row.outcome == "partial"
                && row.source.as_deref() == Some(source)
                && row.query.as_deref() == Some(generation)
        })
        .map(|row| row.technique.as_str())
        .collect::<Vec<_>>();
    let mut expected = expected_techniques
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    observed.sort_unstable();
    observed.dedup();
    expected.sort_unstable();
    expected.dedup();
    observed == expected
}

async fn lock_operation_epoch_and_validate_org(
    connection: &mut PgConnection,
    attempt_guard: &TechniqueOutcomeAttemptGuard,
) -> Result<bool> {
    let epoch_matches = sqlx::query_scalar::<_, i32>(LOCK_OPERATION_EPOCH_SQL)
        .bind(attempt_guard.operation_id)
        .bind(&attempt_guard.stage)
        .bind(attempt_guard.stage_started_at)
        .bind(attempt_guard.engagement_org_id)
        .fetch_optional(&mut *connection)
        .await?
        .is_some();
    if !epoch_matches {
        return Ok(false);
    }
    if let Some(engagement_org_id) = attempt_guard.engagement_org_id {
        let active_org_in_subtree =
            sqlx::query_scalar::<_, bool>(ACTIVE_ORG_IN_ENGAGEMENT_SUBTREE_SQL)
                .bind(engagement_org_id)
                .bind(attempt_guard.organization_id)
                .fetch_one(&mut *connection)
                .await?;
        if !active_org_in_subtree {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Lock and validate the exact operation epoch plus producer generation.
/// Sibling repositories use this after locking the same target guard so a
/// late business write cannot land after a newer attempt has replaced the
/// producer's partial marker.
pub(super) async fn lock_attempt_generation_current(
    connection: &mut PgConnection,
    attempt_guard: &TechniqueOutcomeAttemptGuard,
    run_id: &str,
    asset: &str,
    techniques: &[String],
) -> Result<bool> {
    if techniques.is_empty()
        || attempt_guard.source.trim().is_empty()
        || attempt_guard.generation.trim().is_empty()
    {
        return Ok(false);
    }
    if !lock_operation_epoch_and_validate_org(connection, attempt_guard).await? {
        return Ok(false);
    }
    let current = sqlx::query_as::<_, AttemptStateRow>(LOCK_ATTEMPT_STATE_SQL)
        .bind(attempt_guard.organization_id)
        .bind(run_id)
        .bind(asset)
        .bind(techniques)
        .fetch_all(&mut *connection)
        .await?;
    Ok(attempt_generation_matches(
        &current,
        techniques,
        &attempt_guard.source,
        &attempt_guard.generation,
    ))
}

/// Start a generated attempt only while its operation epoch is still current.
/// This closes the revalidate→restart→marker race: the operation row is locked
/// and compared in the same short transaction as the four partial upserts.
pub async fn upsert_attempt_markers_guarded_if_epoch_current(
    pool: &PgPool,
    target_guard: &TargetWriteGuard,
    attempt_guard: &TechniqueOutcomeAttemptGuard,
    writes: &[TechniqueOutcomeWrite],
) -> Result<ConditionalBatchUpsertResult> {
    if writes.is_empty() || writes.iter().any(|write| write.outcome != "partial") {
        return Err(anyhow::anyhow!(
            "conditional attempt-start batch requires non-empty partial writes"
        ));
    }
    if target_guard.organization_id != Some(attempt_guard.organization_id) {
        return Err(anyhow::anyhow!(
            "attempt organization does not match target write guard"
        ));
    }
    let run_id = &writes[0].run_id;
    let asset = &writes[0].asset;
    let mut techniques = Vec::with_capacity(writes.len());
    for write in writes {
        if write.organization_id != attempt_guard.organization_id
            || &write.run_id != run_id
            || &write.asset != asset
            || write.source.as_deref() != Some(attempt_guard.source.as_str())
            || write.query.as_deref() != Some(attempt_guard.generation.as_str())
        {
            return Err(anyhow::anyhow!(
                "conditional attempt-start batch does not share the attempt witness"
            ));
        }
        if !techniques.contains(&write.technique) {
            techniques.push(write.technique.clone());
        }
    }
    if techniques.len() != writes.len() {
        return Err(anyhow::anyhow!(
            "conditional attempt-start batch contains duplicate techniques"
        ));
    }

    let mut tx = pool.begin().await?;
    lock_target_write_guard(&mut tx, target_guard).await?;
    if !lock_operation_epoch_and_validate_org(&mut tx, attempt_guard).await? {
        tx.rollback().await?;
        return Ok(ConditionalBatchUpsertResult::Superseded);
    }
    for write in writes {
        if !execute_epoch_attempt_marker(&mut *tx, write, attempt_guard.stage_started_at).await? {
            tx.rollback().await?;
            return Ok(ConditionalBatchUpsertResult::Superseded);
        }
    }
    tx.commit().await?;
    Ok(ConditionalBatchUpsertResult::Applied)
}

/// Conditionally publish a terminal sibling group while both the trusted
/// operation epoch and this origin's attempt generation are still current.
///
/// All locks and writes live in one short transaction. Callers must complete
/// network work and evidence preparation before entering this function (I9).
async fn upsert_batch_guarded_if_attempt_current_inner(
    pool: &PgPool,
    target_guard: &TargetWriteGuard,
    attempt_guard: &TechniqueOutcomeAttemptGuard,
    writes: &[TechniqueOutcomeWrite],
    clear_operation_state_slot: Option<(&str, &str)>,
) -> Result<ConditionalBatchUpsertResult> {
    if writes.is_empty() {
        return Err(anyhow::anyhow!(
            "conditional technique outcome batch must not be empty"
        ));
    }
    if clear_operation_state_slot.is_some()
        && writes
            .iter()
            .any(|write| !state_slot_clear_outcome_is_terminal(&write.outcome))
    {
        return Err(anyhow::anyhow!(
            "conditional state-slot cleanup requires terminal found/empty/blocked outcomes"
        ));
    }
    if target_guard.organization_id != Some(attempt_guard.organization_id) {
        return Err(anyhow::anyhow!(
            "attempt organization does not match target write guard"
        ));
    }
    let run_id = &writes[0].run_id;
    let asset = &writes[0].asset;
    let mut techniques = Vec::with_capacity(writes.len());
    for write in writes {
        if write.organization_id != attempt_guard.organization_id
            || &write.run_id != run_id
            || &write.asset != asset
            || write.source.as_deref() != Some(attempt_guard.source.as_str())
            || write.query.as_deref() != Some(attempt_guard.generation.as_str())
        {
            return Err(anyhow::anyhow!(
                "conditional technique outcome batch does not share the attempt witness"
            ));
        }
        if !techniques.contains(&write.technique) {
            techniques.push(write.technique.clone());
        }
    }
    if techniques.len() != writes.len() {
        return Err(anyhow::anyhow!(
            "conditional technique outcome batch contains duplicate techniques"
        ));
    }

    let mut tx = pool.begin().await?;
    lock_target_write_guard(&mut tx, target_guard).await?;
    if !lock_attempt_generation_current(&mut tx, attempt_guard, run_id, asset, &techniques).await? {
        tx.rollback().await?;
        return Ok(ConditionalBatchUpsertResult::Superseded);
    }
    for write in writes {
        execute_upsert(&mut *tx, write).await?;
    }
    if let Some((namespace, slot)) = clear_operation_state_slot {
        if namespace.trim().is_empty() || slot.trim().is_empty() {
            tx.rollback().await?;
            return Err(anyhow::anyhow!(
                "conditional terminal checkpoint cleanup requires a namespace and slot"
            ));
        }
        let cleared = sqlx::query(CLEAR_OPERATION_STATE_SLOT_SQL)
            .bind(attempt_guard.operation_id)
            .bind(namespace)
            .bind(slot)
            .bind(&attempt_guard.stage)
            .bind(attempt_guard.stage_started_at)
            .bind(attempt_guard.engagement_org_id)
            .execute(&mut *tx)
            .await?;
        if cleared.rows_affected() != 1 {
            tx.rollback().await?;
            return Ok(ConditionalBatchUpsertResult::Superseded);
        }
    }
    tx.commit().await?;
    Ok(ConditionalBatchUpsertResult::Applied)
}

pub async fn upsert_batch_guarded_if_attempt_current(
    pool: &PgPool,
    target_guard: &TargetWriteGuard,
    attempt_guard: &TechniqueOutcomeAttemptGuard,
    writes: &[TechniqueOutcomeWrite],
) -> Result<ConditionalBatchUpsertResult> {
    upsert_batch_guarded_if_attempt_current_inner(pool, target_guard, attempt_guard, writes, None)
        .await
}

/// Publish the terminal outcome and remove its operation-state recovery slot
/// in the same generation-guarded transaction. A crash can therefore leave
/// either the prior partial marker + cursor or the terminal row without the
/// cursor, never a terminal row paired with a replayable cursor.
pub async fn upsert_batch_guarded_if_attempt_current_and_clear_state_slot(
    pool: &PgPool,
    target_guard: &TargetWriteGuard,
    attempt_guard: &TechniqueOutcomeAttemptGuard,
    writes: &[TechniqueOutcomeWrite],
    state_namespace: &str,
    state_slot: &str,
) -> Result<ConditionalBatchUpsertResult> {
    upsert_batch_guarded_if_attempt_current_inner(
        pool,
        target_guard,
        attempt_guard,
        writes,
        Some((state_namespace, state_slot)),
    )
    .await
}

/// 读某 `(org, run)` 的全部 technique_outcome 行（org 隔离）。
pub async fn list_for_run(
    pool: &PgPool,
    organization_id: Uuid,
    run_id: &str,
) -> Result<Vec<TechniqueOutcomeRow>> {
    let rows = sqlx::query_as::<_, TechniqueOutcomeRow>(LIST_FOR_RUN_SQL)
        .bind(organization_id)
        .bind(run_id)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// 读某 `(org, run)` 的 technique_outcome 行，可选 freshness cutoff（护栏 4）。
/// `since = None` 等价 [`list_for_run`]（presence-only）；`since = Some(cutoff)`
/// 只返回 `collected_at >= cutoff` 的行，`collected_at IS NULL` 的行被排除。
pub async fn list_for_run_fresh(
    pool: &PgPool,
    organization_id: Uuid,
    run_id: &str,
    since: Option<DateTime<Utc>>,
) -> Result<Vec<TechniqueOutcomeRow>> {
    let rows = sqlx::query_as::<_, TechniqueOutcomeRow>(LIST_FOR_RUN_FRESH_SQL)
        .bind(organization_id)
        .bind(run_id)
        .bind(since)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_sql_targets_unique_key_and_keeps_seq_on_conflict() {
        // 冲突键必须带 organization_id，避免同一 stage_run 的 sibling org 互相覆盖。
        assert!(UPSERT_SQL
            .contains("ON CONFLICT (organization_id, run_id, asset, technique) DO UPDATE"));
        assert!(
            !UPSERT_SQL.contains("seq = EXCLUDED.seq"),
            "seq must NOT be updated on conflict"
        );
        assert!(UPSERT_SQL.contains("updated_at = NOW()"));
    }

    #[test]
    fn upsert_sql_seq_is_per_run_autoincrement() {
        // D2：首插 seq = 该 org/run 内 MAX(seq)+1。
        assert!(UPSERT_SQL.contains(
            "COALESCE(MAX(seq), 0) + 1 FROM technique_outcomes WHERE organization_id = $1 AND run_id = $2"
        ));
    }

    #[test]
    fn upsert_sql_writes_provenance_columns() {
        for col in [
            "outcome",
            "source",
            "query",
            "result_count",
            "confidence",
            "evidence_ids",
            "collected_at",
        ] {
            assert!(UPSERT_SQL.contains(col), "upsert must write {col}");
            assert!(
                UPSERT_SQL.contains(&format!("{col} = EXCLUDED.{col}")),
                "conflict update must refresh {col}"
            );
        }
    }

    #[test]
    fn terminal_materialization_sql_cannot_downgrade_terminal_truth() {
        assert!(UPSERT_TERMINAL_IF_UNFINISHED_SQL
            .contains("ON CONFLICT (organization_id, run_id, asset, technique) DO UPDATE"));
        assert!(UPSERT_TERMINAL_IF_UNFINISHED_SQL
            .contains("WHERE technique_outcomes.outcome IN ('partial', 'error')"));
        assert!(!UPSERT_TERMINAL_IF_UNFINISHED_SQL.contains("'found'"));
        assert!(!UPSERT_TERMINAL_IF_UNFINISHED_SQL.contains("'empty'"));
    }

    #[test]
    fn attempt_start_sql_cannot_downgrade_terminal_truth() {
        assert!(UPSERT_ATTEMPT_MARKER_IF_UNFINISHED_SQL
            .contains("ON CONFLICT (organization_id, run_id, asset, technique) DO UPDATE"));
        assert!(UPSERT_ATTEMPT_MARKER_IF_UNFINISHED_SQL
            .contains("WHERE technique_outcomes.outcome IN ('partial', 'error')"));
        assert!(!UPSERT_ATTEMPT_MARKER_IF_UNFINISHED_SQL.contains("'found'"));
        assert!(!UPSERT_ATTEMPT_MARKER_IF_UNFINISHED_SQL.contains("'empty'"));
    }

    #[test]
    fn epoch_guarded_attempt_start_refreshes_only_stale_terminal_truth() {
        assert!(UPSERT_ATTEMPT_MARKER_IF_UNFINISHED_OR_STALE_SQL
            .contains("ON CONFLICT (organization_id, run_id, asset, technique) DO UPDATE"));
        assert!(UPSERT_ATTEMPT_MARKER_IF_UNFINISHED_OR_STALE_SQL
            .contains("technique_outcomes.collected_at < $12"));
        assert!(UPSERT_ATTEMPT_MARKER_IF_UNFINISHED_OR_STALE_SQL
            .contains("technique_outcomes.collected_at IS NULL"));
        assert!(LOCK_OPERATION_EPOCH_SQL.contains("stage_started_at = $3"));
    }

    #[test]
    fn batch_upsert_reuses_the_same_org_scoped_conflict_contract() {
        assert!(UPSERT_SQL
            .contains("ON CONFLICT (organization_id, run_id, asset, technique) DO UPDATE"));
    }

    #[test]
    fn guarded_batch_reuses_org_scoped_conflict_contract() {
        assert!(UPSERT_SQL
            .contains("ON CONFLICT (organization_id, run_id, asset, technique) DO UPDATE"));
    }

    #[test]
    fn monotonic_guarded_batch_cannot_downgrade_terminal_truth() {
        assert!(UPSERT_ATTEMPT_MARKER_IF_UNFINISHED_SQL
            .contains("WHERE technique_outcomes.outcome IN ('partial', 'error')"));
    }

    #[test]
    fn attempt_generation_a_cannot_publish_after_b_replaced_markers() {
        let techniques = vec!["JS".to_string(), "DIR".to_string()];
        let rows = techniques
            .iter()
            .map(|technique| AttemptStateRow {
                technique: technique.clone(),
                outcome: "partial".to_string(),
                source: Some("enum_preflight_web_origins".to_string()),
                query: Some("attempt-b".to_string()),
            })
            .collect::<Vec<_>>();

        assert!(!attempt_generation_matches(
            &rows,
            &techniques,
            "enum_preflight_web_origins",
            "attempt-a"
        ));
        assert!(attempt_generation_matches(
            &rows,
            &techniques,
            "enum_preflight_web_origins",
            "attempt-b"
        ));
    }

    #[test]
    fn conditional_batch_sql_locks_epoch_and_attempt_rows_before_publish() {
        assert!(LOCK_OPERATION_EPOCH_SQL.contains("stage_started_at = $3"));
        assert!(LOCK_OPERATION_EPOCH_SQL.contains("superseded_by IS NULL"));
        assert!(LOCK_OPERATION_EPOCH_SQL.contains("FOR UPDATE"));
        assert!(ACTIVE_ORG_IN_ENGAGEMENT_SUBTREE_SQL.contains("WITH RECURSIVE subtree"));
        assert!(LOCK_ATTEMPT_STATE_SQL.contains("organization_id = $1"));
        assert!(LOCK_ATTEMPT_STATE_SQL.contains("run_id = $2"));
        assert!(LOCK_ATTEMPT_STATE_SQL.contains("asset = $3"));
        assert!(LOCK_ATTEMPT_STATE_SQL.contains("FOR UPDATE"));
        assert!(CLEAR_OPERATION_STATE_SLOT_SQL.contains("state_blob"));
        assert!(CLEAR_OPERATION_STATE_SLOT_SQL.contains("stage_started_at = $5"));
        assert!(CLEAR_OPERATION_STATE_SLOT_SQL.contains("superseded_by IS NULL"));
    }

    #[test]
    fn attempt_start_uses_the_same_epoch_lock_as_terminal_publish() {
        assert!(LOCK_OPERATION_EPOCH_SQL.contains("stage_started_at = $3"));
        assert!(LOCK_OPERATION_EPOCH_SQL.contains("engagement_org_id IS NOT DISTINCT FROM $4"));
        assert!(LOCK_OPERATION_EPOCH_SQL.contains("FOR UPDATE"));
    }

    #[test]
    fn state_slot_cleanup_accepts_only_route_terminal_outcomes() {
        for outcome in ["found", "empty", "blocked"] {
            assert!(state_slot_clear_outcome_is_terminal(outcome));
        }
        for outcome in ["partial", "error", "pending", ""] {
            assert!(!state_slot_clear_outcome_is_terminal(outcome));
        }
    }

    #[test]
    fn list_for_run_sql_is_org_isolated_and_ordered() {
        // I2：org 过滤；同 seq 的并发首插仍按业务键稳定排序。
        assert!(LIST_FOR_RUN_SQL.contains("WHERE organization_id = $1 AND run_id = $2"));
        assert!(LIST_FOR_RUN_SQL.contains("ORDER BY seq, asset, technique"));
    }

    #[test]
    fn list_for_run_fresh_sql_applies_cutoff_and_stays_org_isolated() {
        // 护栏 4：org 过滤保持；$3 NULL → presence-only，否则 collected_at >= $3。
        assert!(LIST_FOR_RUN_FRESH_SQL.contains("WHERE organization_id = $1 AND run_id = $2"));
        assert!(
            LIST_FOR_RUN_FRESH_SQL.contains("$3::timestamptz IS NULL OR collected_at >= $3"),
            "fresh query must gate on collected_at cutoff with a NULL passthrough"
        );
        assert!(LIST_FOR_RUN_FRESH_SQL.contains("ORDER BY seq, asset, technique"));
    }
}
