//! `attack_candidates` 读写（设计 2026-07-02-attack-stage-formulaic-candidate-exploit
//! §3.7）。attack_candidate 阶段产出的结构化攻击假设（[`AttackCandidate`]）的持久化，
//! 供 chain-wave 控制器跨波去重、追 a→b→c 血缘（`parent_finding_id`），并驱动
//! verification 消费的 disposition 状态机。
//!
//! 纯 runtime sqlx（无 `query!` 宏 → 无需编译期 DB）；SQL 抽成 `const` 便于单测。
//! **I2 IDOR**：一切读写按 `operation_id` +（org 场景）`organization_id` 过滤
//! （`organization_id IS NOT DISTINCT FROM $` 让 NULL=project 模式与具体 org 都精确
//! 隔离）。去重：`UNIQUE(operation_id, target, hypothesis_hash)`，`upsert_by_hash`
//! 冲突时刷新 disposition/技术/理由等可变字段但不堆叠新行，避免 a↔b 反复生成。

use anyhow::Result;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use std::collections::BTreeMap;
use uuid::Uuid;

const MAX_CANDIDATE_BATCH_ITEMS: usize = 100;
const MAX_CANDIDATE_HYPOTHESIS_BYTES: usize = 4 * 1024;
const MAX_CANDIDATE_RATIONALE_BYTES: usize = 8 * 1024;
const MAX_CANDIDATE_TECHNIQUE_BYTES: usize = 128;
const MAX_CANDIDATE_REASON_CODE_BYTES: usize = 64;
const MAX_CANDIDATE_DECISION_EVIDENCE_IDS: usize = 64;
const MAX_CANDIDATE_ACCEPTANCE_BYTES: usize = 256 * 1024;

/// upsert 一条攻击假设的入参。`hypothesis_hash` 由 [`hypothesis_hash`] 从
/// `(target, technique, hypothesis)` 确定性派生（MVP 语义去重 deferred，设计 §11
/// 开放问题 4）。
#[derive(Debug, Clone)]
pub struct AttackCandidateWrite {
    pub candidate_id: Uuid,
    pub operation_id: String,
    pub organization_id: Option<Uuid>,
    pub target: String,
    pub hypothesis: String,
    pub technique: Option<String>,
    pub rationale: String,
    /// wiki writeup / CVE id 等先验引用（存 JSONB）。
    pub prior_refs: Vec<String>,
    pub suggested_approach: String,
    /// `high` | `medium` | `low`（DB CHECK 约束）。
    pub priority: String,
    pub wave: i32,
    pub parent_finding_id: Option<Uuid>,
    /// `proposed` | `approved` | `rejected` | `verified` | `refuted` | `blocked`。
    pub disposition: String,
}

/// 读出的一行（gate / 控制器 / reporting 用）。
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AttackCandidateRow {
    pub candidate_id: Uuid,
    pub operation_id: String,
    pub organization_id: Option<Uuid>,
    pub target: String,
    pub hypothesis: String,
    pub hypothesis_hash: String,
    pub technique: Option<String>,
    pub rationale: String,
    pub prior_refs: serde_json::Value,
    pub suggested_approach: String,
    pub priority: String,
    pub wave: i32,
    pub parent_finding_id: Option<Uuid>,
    pub disposition: String,
    pub operation_uuid: Option<Uuid>,
    pub scope_snapshot_id: Option<Uuid>,
    pub wave_run_id: Option<Uuid>,
    pub wave_unit_id: Option<Uuid>,
    pub source_work_item_id: Option<Uuid>,
    pub decision_stage_execution_id: Option<Uuid>,
    pub decision_stage_run_unit_id: Option<Uuid>,
    pub decision_deliverable_submission_id: Option<Uuid>,
    pub decision_stage_kind: Option<String>,
    pub target_live_id: Option<Uuid>,
    pub target_type_at_time: Option<String>,
    pub target_value_at_time: Option<String>,
    pub target_identity_hash: Option<String>,
    pub execution_plan: Option<serde_json::Value>,
    pub candidate_plan_hash: Option<String>,
    pub risk_class: Option<String>,
    pub row_version: i64,
    pub terminal_attempt_id: Option<Uuid>,
    pub terminal_finding_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// `(target, technique, normalize(hypothesis))` 的 sha256 十六进制（MVP 确定性去重）。
///
/// normalize = trim + 折叠内部连续空白为单空格 + 小写，容忍模型对同一假设的措辞抖动
/// （大小写 / 多空格 / 首尾空白）。语义相似度去重 deferred（设计 §11 开放问题 4）。
pub fn hypothesis_hash(target: &str, technique: Option<&str>, hypothesis: &str) -> String {
    let norm = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let mut hasher = Sha256::new();
    hasher.update(norm(target).as_bytes());
    hasher.update([0x1f]);
    hasher.update(norm(technique.unwrap_or("")).as_bytes());
    hasher.update([0x1f]);
    hasher.update(norm(hypothesis).as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        hex.push_str(&format!("{b:02x}"));
    }
    hex
}

/// upsert：`UNIQUE(operation_id, target, hypothesis_hash)` 冲突 → 更新可变字段
/// （technique/rationale/prior_refs/suggested_approach/priority/wave/
/// parent_finding_id/disposition/updated_at），**candidate_id / created_at 保持
/// 首插值**（幂等不堆叠），`RETURNING candidate_id` 返回该假设的稳定 id。
const UPSERT_LEGACY_SQL: &str = "\
INSERT INTO attack_candidates \
  (candidate_id, operation_id, organization_id, target, hypothesis, hypothesis_hash, \
   technique, rationale, prior_refs, suggested_approach, priority, wave, \
   parent_finding_id, disposition) \
VALUES \
  ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) \
ON CONFLICT (operation_id, target, hypothesis_hash) \
WHERE operation_uuid IS NULL DO UPDATE SET \
  technique = EXCLUDED.technique, \
  rationale = EXCLUDED.rationale, \
  prior_refs = EXCLUDED.prior_refs, \
  suggested_approach = EXCLUDED.suggested_approach, \
  priority = EXCLUDED.priority, \
  wave = EXCLUDED.wave, \
  parent_finding_id = EXCLUDED.parent_finding_id, \
  disposition = EXCLUDED.disposition, \
  updated_at = NOW() \
RETURNING candidate_id";

/// 列某 operation（org 隔离，IDOR）的全部候选，按波次 + 创建序稳定排序。
const LIST_BY_OPERATION_SQL: &str = "\
SELECT candidate_id, operation_id, organization_id, target, hypothesis, hypothesis_hash, \
       technique, rationale, prior_refs, suggested_approach, priority, wave, \
       parent_finding_id, disposition, operation_uuid, scope_snapshot_id, \
       wave_run_id, wave_unit_id, source_work_item_id, decision_stage_execution_id, \
       decision_stage_run_unit_id, decision_deliverable_submission_id, \
       decision_stage_kind, target_live_id, target_type_at_time, \
       target_value_at_time, target_identity_hash, execution_plan, \
       candidate_plan_hash, risk_class, row_version, terminal_attempt_id, \
       terminal_finding_id, created_at, updated_at \
FROM attack_candidates \
WHERE operation_id = $1 AND organization_id IS NOT DISTINCT FROM $2 \
ORDER BY wave, created_at";

/// 列某 operation 某一波（org 隔离，IDOR）的候选。
const LIST_BY_WAVE_SQL: &str = "\
SELECT candidate_id, operation_id, organization_id, target, hypothesis, hypothesis_hash, \
       technique, rationale, prior_refs, suggested_approach, priority, wave, \
       parent_finding_id, disposition, operation_uuid, scope_snapshot_id, \
       wave_run_id, wave_unit_id, source_work_item_id, decision_stage_execution_id, \
       decision_stage_run_unit_id, decision_deliverable_submission_id, \
       decision_stage_kind, target_live_id, target_type_at_time, \
       target_value_at_time, target_identity_hash, execution_plan, \
       candidate_plan_hash, risk_class, row_version, terminal_attempt_id, \
       terminal_finding_id, created_at, updated_at \
FROM attack_candidates \
WHERE operation_id = $1 AND organization_id IS NOT DISTINCT FROM $2 AND wave = $3 \
ORDER BY created_at";

/// 更新某候选的 disposition（IDOR：按 candidate_id + operation_id + org 三重限定，
/// 防跨 operation / 跨 org 改他人候选）。返回受影响行数（0 = 未命中/越权）。
const UPDATE_DISPOSITION_SQL: &str = "\
UPDATE attack_candidates SET disposition = $4, updated_at = NOW() \
WHERE candidate_id = $1 AND operation_id = $2 AND organization_id IS NOT DISTINCT FROM $3";

fn prior_refs_json(refs: &[String]) -> serde_json::Value {
    serde_json::Value::Array(
        refs.iter()
            .map(|r| serde_json::Value::String(r.clone()))
            .collect(),
    )
}

/// upsert 一条候选（去重键 = operation_id + target + hypothesis_hash），返回稳定
/// candidate_id（冲突时为既有行的 id）。
pub async fn upsert_legacy_by_hash(pool: &PgPool, w: &AttackCandidateWrite) -> Result<Uuid> {
    let hash = hypothesis_hash(&w.target, w.technique.as_deref(), &w.hypothesis);
    let id: Uuid = sqlx::query_scalar(UPSERT_LEGACY_SQL)
        .bind(w.candidate_id)
        .bind(&w.operation_id)
        .bind(w.organization_id)
        .bind(&w.target)
        .bind(&w.hypothesis)
        .bind(&hash)
        .bind(w.technique.as_deref())
        .bind(&w.rationale)
        .bind(prior_refs_json(&w.prior_refs))
        .bind(&w.suggested_approach)
        .bind(&w.priority)
        .bind(w.wave)
        .bind(w.parent_finding_id)
        .bind(&w.disposition)
        .fetch_one(pool)
        .await?;
    Ok(id)
}

/// Backward-compatible legacy alias. V2 callers must use
/// [`accept_gate_passed_candidate_batch`] and never enter this UPSERT path.
pub async fn upsert_by_hash(pool: &PgPool, w: &AttackCandidateWrite) -> Result<Uuid> {
    upsert_legacy_by_hash(pool, w).await
}

/// 创建一条候选（等价 upsert：同假设重复提交返回既有 id，避免重复行）。
pub async fn create(pool: &PgPool, w: &AttackCandidateWrite) -> Result<Uuid> {
    upsert_legacy_by_hash(pool, w).await
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AcceptedCandidateDraft {
    pub candidate_id: Uuid,
    pub work_item_id: Uuid,
    pub hypothesis: String,
    pub technique: Option<String>,
    pub rationale: String,
    pub prior_refs: Vec<String>,
    pub suggested_approach: String,
    pub priority: String,
    pub execution_plan: serde_json::Value,
    pub candidate_plan_hash: String,
    pub risk_class: String,
    pub evidence_ids: Vec<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct NoCandidateDecision {
    pub work_item_id: Uuid,
    pub reason_code: String,
    pub detail: String,
    pub evidence_ids: Vec<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AcceptCandidateBatch {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub decision_stage_execution_id: Uuid,
    pub decision_stage_run_unit_id: Uuid,
    pub decision_deliverable_submission_id: Uuid,
    pub manifest_hash: String,
    pub expected_work_item_ids: Vec<Uuid>,
    pub candidates: Vec<AcceptedCandidateDraft>,
    pub no_candidate_decisions: Vec<NoCandidateDecision>,
}

/// Server-classified Candidate payload carried into the runtime final-seal
/// transaction. Operation/scope/org/current decision authority are deliberately
/// absent and are rebound from the locked Unit + trusted submission.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CandidateAcceptanceInput {
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub manifest_hash: String,
    pub expected_work_item_ids: Vec<Uuid>,
    pub candidates: Vec<AcceptedCandidateDraft>,
    pub no_candidate_decisions: Vec<NoCandidateDecision>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedCandidateBatch {
    pub candidate_ids: Vec<Uuid>,
    pub no_candidate_work_item_ids: Vec<Uuid>,
    pub replayed: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct LockedWorkItem {
    id: Uuid,
    seed_id: Uuid,
    work_item_key: String,
    technique: String,
    organization_id: Uuid,
    target_live_id: Option<Uuid>,
    target_type_at_time: String,
    target_value_at_time: String,
    target_identity_hash: String,
    decision_kind: Option<String>,
    candidate_id: Option<Uuid>,
    no_candidate_reason_code: Option<String>,
    no_candidate_detail: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct ReplayCandidateRow {
    candidate_id: Uuid,
    hypothesis: String,
    hypothesis_hash: String,
    technique: Option<String>,
    rationale: String,
    prior_refs: serde_json::Value,
    suggested_approach: String,
    priority: String,
    operation_uuid: Uuid,
    scope_snapshot_id: Uuid,
    wave_run_id: Uuid,
    wave_unit_id: Uuid,
    source_work_item_id: Uuid,
    decision_stage_execution_id: Uuid,
    decision_stage_run_unit_id: Uuid,
    decision_deliverable_submission_id: Uuid,
    decision_stage_kind: String,
    organization_id: Uuid,
    target_live_id: Option<Uuid>,
    target_type_at_time: String,
    target_value_at_time: String,
    target_identity_hash: String,
    execution_plan: serde_json::Value,
    candidate_plan_hash: String,
    risk_class: String,
}

#[derive(Debug, sqlx::FromRow)]
struct LockedWaveUnit {
    generation: i32,
    wave_status: String,
    unit_status: String,
    review_closed: bool,
    terminal_at: Option<DateTime<Utc>>,
    manifest_hash: Option<String>,
    manifest_count: Option<i32>,
    manifest_frozen_at: Option<DateTime<Utc>>,
}

fn attack_conflict(message: impl Into<String>) -> crate::DbError {
    crate::DbError::Other(anyhow::anyhow!(message.into()))
}

fn sorted_unique_ids(ids: &[Uuid]) -> Option<Vec<Uuid>> {
    let mut sorted = ids.to_vec();
    sorted.sort_unstable();
    let original_len = sorted.len();
    sorted.dedup();
    (sorted.len() == original_len).then_some(sorted)
}

fn sorted_unique_evidence_ids(ids: &[i64]) -> Option<Vec<i64>> {
    let mut sorted = ids.to_vec();
    sorted.sort_unstable();
    let original_len = sorted.len();
    sorted.dedup();
    (sorted.len() == original_len && sorted.iter().all(|id| *id > 0)).then_some(sorted)
}

fn bounded_nonempty(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max_bytes
}

fn stable_reason_code(value: &str) -> bool {
    bounded_nonempty(value, MAX_CANDIDATE_REASON_CODE_BYTES)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

async fn exact_linked_evidence(
    connection: &mut sqlx::PgConnection,
    table: &str,
    owner_column: &str,
    owner_id: Uuid,
    role: &str,
) -> crate::Result<Vec<i64>> {
    let allowed = matches!(
        (table, owner_column),
        ("attack_candidate_evidence", "candidate_id")
            | ("attack_candidate_work_item_evidence", "work_item_id")
    );
    if !allowed {
        return Err(attack_conflict("unsupported Candidate evidence projection"));
    }
    let sql = format!(
        "SELECT evidence_id FROM {table} WHERE {owner_column}=$1 AND role=$2 ORDER BY evidence_id"
    );
    Ok(sqlx::query_scalar(&sql)
        .bind(owner_id)
        .bind(role)
        .fetch_all(&mut *connection)
        .await?)
}

fn canonicalize_json(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), canonicalize_json(value)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(canonicalize_json).collect())
        }
        _ => value.clone(),
    }
}

pub fn canonical_execution_plan_hash(plan: &serde_json::Value) -> crate::Result<String> {
    let bytes = serde_json::to_vec(&canonicalize_json(plan))?;
    let digest = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("sha256:{digest}"))
}

async fn evidence_is_grounded_in_work_item(
    connection: &mut sqlx::PgConnection,
    item: &LockedWorkItem,
    evidence_ids: &[i64],
) -> crate::Result<bool> {
    let unique = {
        let mut ids = evidence_ids.to_vec();
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    if unique.len() != evidence_ids.len() || unique.iter().any(|id| *id <= 0) {
        return Ok(false);
    }
    let count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(DISTINCT evidence_id) FROM (
               SELECT evidence_id FROM attack_candidate_seed_evidence
                WHERE seed_id=$1 AND evidence_id=ANY($3)
               UNION ALL
               SELECT evidence_id FROM attack_candidate_work_item_evidence
                WHERE work_item_id=$2 AND role='support' AND evidence_id=ANY($3)
           ) AS grounded"#,
    )
    .bind(item.seed_id)
    .bind(item.id)
    .bind(&unique)
    .fetch_one(&mut *connection)
    .await?;
    Ok(count == unique.len() as i64)
}

async fn canonical_manifest_hash(
    connection: &mut sqlx::PgConnection,
    items: &[LockedWorkItem],
) -> crate::Result<String> {
    let mut projection = Vec::with_capacity(items.len());
    for item in items {
        let evidence_ids: Vec<i64> = sqlx::query_scalar(
            r#"SELECT evidence_id FROM (
                   SELECT evidence_id FROM attack_candidate_seed_evidence WHERE seed_id=$1
                   UNION
                   SELECT evidence_id FROM attack_candidate_work_item_evidence
                    WHERE work_item_id=$2 AND role IN ('observation','support')
               ) evidence ORDER BY evidence_id"#,
        )
        .bind(item.seed_id)
        .bind(item.id)
        .fetch_all(&mut *connection)
        .await?;
        projection.push(serde_json::json!({
            "evidence_ids": evidence_ids,
            "target_identity_hash": item.target_identity_hash,
            "technique": item.technique,
            "work_item_id": item.id,
            "work_item_key": item.work_item_key,
        }));
    }
    canonical_execution_plan_hash(&serde_json::Value::Array(projection))
}

/// Accept a complete server-seeded manifest after the trusted caller reports
/// final Gate PASS. The repository re-reads and locks the exact WaveUnit and
/// every work item; model-provided ownership/source fields are never accepted.
pub async fn accept_gate_passed_candidate_batch(
    tx: &mut Transaction<'_, Postgres>,
    command: AcceptCandidateBatch,
) -> crate::Result<AcceptedCandidateBatch> {
    accept_gate_passed_candidate_batch_with_connection(tx, command).await
}

pub async fn accept_gate_passed_candidate_batch_with_connection(
    connection: &mut sqlx::PgConnection,
    command: AcceptCandidateBatch,
) -> crate::Result<AcceptedCandidateBatch> {
    if command.expected_work_item_ids.len() > MAX_CANDIDATE_BATCH_ITEMS
        || command.candidates.len() + command.no_candidate_decisions.len()
            > MAX_CANDIDATE_BATCH_ITEMS
        || serde_json::to_vec(&command)?.len() > MAX_CANDIDATE_ACCEPTANCE_BYTES
        || command.candidates.iter().any(|draft| {
            draft.candidate_id.is_nil()
                || !bounded_nonempty(&draft.hypothesis, MAX_CANDIDATE_HYPOTHESIS_BYTES)
                || !bounded_nonempty(&draft.rationale, MAX_CANDIDATE_RATIONALE_BYTES)
                || draft.technique.as_deref().is_some_and(|technique| {
                    !bounded_nonempty(technique, MAX_CANDIDATE_TECHNIQUE_BYTES)
                })
                || draft.evidence_ids.is_empty()
                || draft.evidence_ids.len() > MAX_CANDIDATE_DECISION_EVIDENCE_IDS
        })
        || command.no_candidate_decisions.iter().any(|decision| {
            !stable_reason_code(&decision.reason_code)
                || !bounded_nonempty(&decision.detail, MAX_CANDIDATE_RATIONALE_BYTES)
                || decision.evidence_ids.is_empty()
                || decision.evidence_ids.len() > MAX_CANDIDATE_DECISION_EVIDENCE_IDS
        })
    {
        return Err(attack_conflict(
            "Candidate acceptance exceeds the bounded final-seal contract",
        ));
    }
    let expected = sorted_unique_ids(&command.expected_work_item_ids)
        .ok_or_else(|| attack_conflict("duplicate expected work item id"))?;
    if expected.is_empty() {
        return Err(attack_conflict("candidate manifest cannot be empty"));
    }

    let contracts: Option<(String, String)> = sqlx::query_as(
        r#"SELECT runtime_memory_contract,attack_execution_contract
           FROM operation_state WHERE operation_id=$1 FOR UPDATE"#,
    )
    .bind(command.operation_id)
    .fetch_optional(&mut *connection)
    .await?;
    let (runtime_contract, attack_contract) =
        contracts.ok_or_else(|| crate::DbError::NotFound("operation_state".to_string()))?;
    if attack_contract == "legacy"
        || (attack_contract == "v2_only" && runtime_contract != "v2_only")
    {
        return Err(attack_conflict(
            "operation does not permit Candidate V2 writes",
        ));
    }

    let wave = sqlx::query_as::<_, LockedWaveUnit>(
        r#"SELECT run.generation,run.status AS wave_status,
                  unit.status AS unit_status,unit.review_closed,unit.terminal_at,
                  unit.manifest_hash,unit.manifest_count,unit.manifest_frozen_at
           FROM attack_wave_runs AS run
           JOIN attack_wave_units AS unit
             ON unit.wave_run_id=run.id AND unit.operation_id=run.operation_id
            AND unit.scope_snapshot_id=run.scope_snapshot_id
           JOIN stage_run_units AS entry_unit
             ON entry_unit.id=unit.entry_stage_run_unit_id
            AND entry_unit.operation_id=unit.operation_id
            AND entry_unit.stage_execution_id=unit.entry_stage_execution_id
            AND entry_unit.organization_id=unit.organization_id
            AND entry_unit.stage_kind=unit.entry_stage_kind
           JOIN stage_handoffs AS handoff
             ON handoff.operation_id=unit.operation_id
            AND handoff.scope_snapshot_id=unit.scope_snapshot_id
            AND handoff.organization_id=unit.organization_id
            AND handoff.stage_execution_id=unit.entry_stage_execution_id
            AND handoff.source_stage_run_unit_id=unit.entry_stage_run_unit_id
            AND handoff.deliverable_submission_id=unit.entry_deliverable_submission_id
            AND handoff.from_stage_kind=unit.entry_stage_kind
            AND handoff.invalidated_at IS NULL
           WHERE run.id=$1 AND run.operation_id=$2 AND run.scope_snapshot_id=$3
             AND unit.id=$4 AND unit.organization_id=$5
             AND unit.entry_stage_kind='vuln_triage'
             AND entry_unit.status='passed' AND entry_unit.terminal_at IS NOT NULL
           FOR UPDATE OF run,unit,entry_unit,handoff"#,
    )
    .bind(command.wave_run_id)
    .bind(command.operation_id)
    .bind(command.scope_snapshot_id)
    .bind(command.wave_unit_id)
    .bind(command.organization_id)
    .fetch_optional(&mut *connection)
    .await?
    .ok_or_else(|| crate::DbError::NotFound("attack_wave_units".to_string()))?;

    let decision_authority: Option<bool> = sqlx::query_scalar(
        r#"SELECT TRUE
             FROM stage_run_units AS decision_unit
             JOIN stage_deliverable_submissions AS submission
               ON submission.id=$1
              AND submission.operation_id=decision_unit.operation_id
              AND submission.stage_execution_id=decision_unit.stage_execution_id
              AND submission.stage_run_unit_id=decision_unit.id
              AND submission.organization_id=decision_unit.organization_id
              AND submission.stage_kind=decision_unit.stage_kind
             JOIN stage_handoffs AS handoff
               ON handoff.deliverable_submission_id=submission.id
              AND handoff.operation_id=decision_unit.operation_id
              AND handoff.stage_execution_id=decision_unit.stage_execution_id
              AND handoff.source_stage_run_unit_id=decision_unit.id
              AND handoff.organization_id=decision_unit.organization_id
              AND handoff.from_stage_kind=decision_unit.stage_kind
              AND handoff.scope_snapshot_id=decision_unit.scope_snapshot_id
              AND handoff.invalidated_at IS NULL
            WHERE decision_unit.id=$2
              AND decision_unit.operation_id=$3
              AND decision_unit.stage_execution_id=$4
              AND decision_unit.scope_snapshot_id=$5
              AND decision_unit.organization_id=$6
              AND decision_unit.stage_kind='attack_candidate'
              AND decision_unit.status='passed'
              AND decision_unit.terminal_at IS NOT NULL
            FOR UPDATE OF decision_unit,submission,handoff"#,
    )
    .bind(command.decision_deliverable_submission_id)
    .bind(command.decision_stage_run_unit_id)
    .bind(command.operation_id)
    .bind(command.decision_stage_execution_id)
    .bind(command.scope_snapshot_id)
    .bind(command.organization_id)
    .fetch_optional(&mut *connection)
    .await?;
    if decision_authority.is_none() {
        return Err(attack_conflict(
            "Candidate acceptance requires the exact current attack_candidate final-pass submission",
        ));
    }
    let work_items = sqlx::query_as::<_, LockedWorkItem>(
        r#"SELECT item.id,item.seed_id,item.work_item_key,seed.technique,
                  item.organization_id,item.target_live_id,item.target_type_at_time,
                  item.target_value_at_time,item.target_identity_hash,item.decision_kind,
                  item.candidate_id,item.no_candidate_reason_code,item.no_candidate_detail
           FROM attack_candidate_work_items AS item
           JOIN attack_candidate_seeds AS seed ON seed.id=item.seed_id
           WHERE item.wave_unit_id=$1 AND item.operation_id=$2 AND item.scope_snapshot_id=$3
             AND item.organization_id=$4
           ORDER BY item.work_item_key,item.id FOR UPDATE OF item,seed"#,
    )
    .bind(command.wave_unit_id)
    .bind(command.operation_id)
    .bind(command.scope_snapshot_id)
    .bind(command.organization_id)
    .fetch_all(&mut *connection)
    .await?;
    let actual_manifest_hash = canonical_manifest_hash(connection, &work_items).await?;
    if command.manifest_hash != actual_manifest_hash
        || wave.manifest_hash.as_deref() != Some(actual_manifest_hash.as_str())
        || wave.manifest_count != i32::try_from(work_items.len()).ok()
        || wave.manifest_frozen_at.is_none()
    {
        return Err(attack_conflict("Candidate manifest hash drift"));
    }
    let mut actual = work_items.iter().map(|item| item.id).collect::<Vec<_>>();
    actual.sort_unstable();
    if actual != expected {
        return Err(attack_conflict(
            "expected work-item manifest does not match DB truth",
        ));
    }
    let mut terminal_ids = command
        .candidates
        .iter()
        .map(|draft| draft.work_item_id)
        .chain(
            command
                .no_candidate_decisions
                .iter()
                .map(|decision| decision.work_item_id),
        )
        .collect::<Vec<_>>();
    terminal_ids.sort_unstable();
    let terminal_len = terminal_ids.len();
    terminal_ids.dedup();
    if terminal_len != terminal_ids.len() || terminal_ids != expected {
        return Err(attack_conflict(
            "every work item must terminate exactly once as candidate or no_candidate",
        ));
    }
    if command
        .candidates
        .iter()
        .any(|draft| draft.evidence_ids.is_empty())
        || command
            .no_candidate_decisions
            .iter()
            .any(|decision| decision.evidence_ids.is_empty())
    {
        return Err(attack_conflict(
            "terminal work-item decisions require evidence",
        ));
    }

    let by_id = work_items
        .iter()
        .map(|item| (item.id, item))
        .collect::<std::collections::HashMap<_, _>>();
    let terminal_count = work_items
        .iter()
        .filter(|item| item.decision_kind.is_some())
        .count();
    if terminal_count != 0 && terminal_count != work_items.len() {
        return Err(attack_conflict(
            "work-item manifest cannot be partially terminal on replay",
        ));
    }
    if terminal_count == work_items.len() {
        let mut candidate_ids = Vec::with_capacity(command.candidates.len());
        for draft in &command.candidates {
            let item = by_id
                .get(&draft.work_item_id)
                .ok_or_else(|| attack_conflict("candidate replay is outside manifest"))?;
            if item.decision_kind.as_deref() != Some("candidate")
                || item.candidate_id != Some(draft.candidate_id)
                || canonical_execution_plan_hash(&draft.execution_plan)?
                    != draft.candidate_plan_hash
            {
                return Err(attack_conflict("candidate replay decision drift"));
            }
            let persisted = sqlx::query_as::<_, ReplayCandidateRow>(
                r#"SELECT candidate_id,hypothesis,hypothesis_hash,technique,rationale,
                          prior_refs,suggested_approach,priority,operation_uuid,
                          scope_snapshot_id,wave_run_id,wave_unit_id,source_work_item_id,
                          decision_stage_execution_id,decision_stage_run_unit_id,
                          decision_deliverable_submission_id,decision_stage_kind,
                          organization_id,target_live_id,target_type_at_time,
                          target_value_at_time,target_identity_hash,execution_plan,
                          candidate_plan_hash,risk_class
                     FROM attack_candidates
                    WHERE candidate_id=$1 AND source_work_item_id=$2
                      AND operation_uuid=$3 AND scope_snapshot_id=$4 AND wave_run_id=$5
                      AND wave_unit_id=$6 AND organization_id=$7
                    FOR UPDATE"#,
            )
            .bind(draft.candidate_id)
            .bind(draft.work_item_id)
            .bind(command.operation_id)
            .bind(command.scope_snapshot_id)
            .bind(command.wave_run_id)
            .bind(command.wave_unit_id)
            .bind(command.organization_id)
            .fetch_optional(&mut *connection)
            .await?
            .ok_or_else(|| attack_conflict("persisted Candidate replay row missing"))?;
            let expected_hypothesis_hash = hypothesis_hash(
                &item.target_value_at_time,
                draft.technique.as_deref(),
                &draft.hypothesis,
            );
            if persisted.candidate_id != draft.candidate_id
                || persisted.hypothesis != draft.hypothesis
                || persisted.hypothesis_hash != expected_hypothesis_hash
                || persisted.technique != draft.technique
                || persisted.rationale != draft.rationale
                || persisted.prior_refs != prior_refs_json(&draft.prior_refs)
                || persisted.suggested_approach != draft.suggested_approach
                || persisted.priority != draft.priority
                || persisted.operation_uuid != command.operation_id
                || persisted.scope_snapshot_id != command.scope_snapshot_id
                || persisted.wave_run_id != command.wave_run_id
                || persisted.wave_unit_id != command.wave_unit_id
                || persisted.source_work_item_id != draft.work_item_id
                || persisted.decision_stage_execution_id != command.decision_stage_execution_id
                || persisted.decision_stage_run_unit_id != command.decision_stage_run_unit_id
                || persisted.decision_deliverable_submission_id
                    != command.decision_deliverable_submission_id
                || persisted.decision_stage_kind != "attack_candidate"
                || persisted.organization_id != command.organization_id
                || persisted.target_live_id != item.target_live_id
                || persisted.target_type_at_time != item.target_type_at_time
                || persisted.target_value_at_time != item.target_value_at_time
                || persisted.target_identity_hash != item.target_identity_hash
                || persisted.execution_plan != draft.execution_plan
                || persisted.candidate_plan_hash != draft.candidate_plan_hash
                || persisted.risk_class != draft.risk_class
            {
                return Err(attack_conflict("persisted Candidate replay payload drift"));
            }
            let expected_evidence = sorted_unique_evidence_ids(&draft.evidence_ids)
                .ok_or_else(|| attack_conflict("invalid Candidate replay evidence"))?;
            let actual_evidence = exact_linked_evidence(
                connection,
                "attack_candidate_evidence",
                "candidate_id",
                draft.candidate_id,
                "support",
            )
            .await?;
            if actual_evidence != expected_evidence {
                return Err(attack_conflict("Candidate replay evidence drift"));
            }
            candidate_ids.push(draft.candidate_id);
        }
        let mut no_candidate_ids = Vec::with_capacity(command.no_candidate_decisions.len());
        for decision in &command.no_candidate_decisions {
            let item = by_id
                .get(&decision.work_item_id)
                .ok_or_else(|| attack_conflict("no-candidate replay is outside manifest"))?;
            if item.decision_kind.as_deref() != Some("no_candidate")
                || item.candidate_id.is_some()
                || item.no_candidate_reason_code.as_deref() != Some(decision.reason_code.as_str())
                || item.no_candidate_detail.as_deref() != Some(decision.detail.as_str())
            {
                return Err(attack_conflict("no-candidate replay decision drift"));
            }
            let expected_evidence = sorted_unique_evidence_ids(&decision.evidence_ids)
                .ok_or_else(|| attack_conflict("invalid no-candidate replay evidence"))?;
            let actual_evidence = exact_linked_evidence(
                connection,
                "attack_candidate_work_item_evidence",
                "work_item_id",
                decision.work_item_id,
                "decision",
            )
            .await?;
            if actual_evidence != expected_evidence {
                return Err(attack_conflict("no-candidate replay evidence drift"));
            }
            no_candidate_ids.push(decision.work_item_id);
        }
        return Ok(AcceptedCandidateBatch {
            candidate_ids,
            no_candidate_work_item_ids: no_candidate_ids,
            replayed: true,
        });
    }
    if wave.wave_status != "open"
        || !matches!(wave.unit_status.as_str(), "open" | "reasoning")
        || wave.review_closed
        || wave.terminal_at.is_some()
    {
        return Err(attack_conflict(
            "fresh Candidate acceptance requires an open reasoning WaveUnit",
        ));
    }
    let mut candidate_ids = Vec::with_capacity(command.candidates.len());
    for draft in &command.candidates {
        let item = by_id
            .get(&draft.work_item_id)
            .ok_or_else(|| attack_conflict("candidate work item is outside manifest"))?;
        if item.organization_id != command.organization_id
            || draft.hypothesis.trim().is_empty()
            || draft.rationale.trim().is_empty()
            || draft.candidate_plan_hash.trim().is_empty()
            || !draft.execution_plan.is_object()
        {
            return Err(attack_conflict("invalid candidate draft"));
        }
        if canonical_execution_plan_hash(&draft.execution_plan)? != draft.candidate_plan_hash {
            return Err(attack_conflict(
                "candidate plan hash does not match canonical plan",
            ));
        }
        if !evidence_is_grounded_in_work_item(connection, item, &draft.evidence_ids).await? {
            return Err(attack_conflict(
                "candidate evidence is not grounded in its work item",
            ));
        }
        let hypothesis_hash = hypothesis_hash(
            &item.target_value_at_time,
            draft.technique.as_deref(),
            &draft.hypothesis,
        );
        sqlx::query(
            r#"INSERT INTO attack_candidates (
                   candidate_id,operation_id,organization_id,target,hypothesis,
                   hypothesis_hash,technique,rationale,prior_refs,suggested_approach,
                   priority,wave,disposition,operation_uuid,scope_snapshot_id,
                   wave_run_id,wave_unit_id,source_work_item_id,
                   decision_stage_execution_id,decision_stage_run_unit_id,
                   decision_deliverable_submission_id,decision_stage_kind,
                   target_live_id,target_type_at_time,target_value_at_time,
                   target_identity_hash,execution_plan,candidate_plan_hash,risk_class
               ) VALUES (
                   $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,'proposed',$13,$14,
                   $15,$16,$17,$18,$19,$20,'attack_candidate',$21,$22,$23,$24,
                   $25,$26,$27
               )"#,
        )
        .bind(draft.candidate_id)
        .bind(command.operation_id.to_string())
        .bind(command.organization_id)
        .bind(&item.target_value_at_time)
        .bind(&draft.hypothesis)
        .bind(hypothesis_hash)
        .bind(&draft.technique)
        .bind(&draft.rationale)
        .bind(prior_refs_json(&draft.prior_refs))
        .bind(&draft.suggested_approach)
        .bind(&draft.priority)
        .bind(wave.generation)
        .bind(command.operation_id)
        .bind(command.scope_snapshot_id)
        .bind(command.wave_run_id)
        .bind(command.wave_unit_id)
        .bind(draft.work_item_id)
        .bind(command.decision_stage_execution_id)
        .bind(command.decision_stage_run_unit_id)
        .bind(command.decision_deliverable_submission_id)
        .bind(item.target_live_id)
        .bind(&item.target_type_at_time)
        .bind(&item.target_value_at_time)
        .bind(&item.target_identity_hash)
        .bind(&draft.execution_plan)
        .bind(&draft.candidate_plan_hash)
        .bind(&draft.risk_class)
        .execute(&mut *connection)
        .await?;
        for evidence_id in &draft.evidence_ids {
            sqlx::query(
                r#"INSERT INTO attack_candidate_evidence(candidate_id,evidence_id,role)
                   VALUES ($1,$2,'support') ON CONFLICT DO NOTHING"#,
            )
            .bind(draft.candidate_id)
            .bind(evidence_id)
            .execute(&mut *connection)
            .await?;
        }
        sqlx::query(
            r#"UPDATE attack_candidate_work_items
               SET decision_kind='candidate',candidate_id=$2,decided_at=NOW(),
                   row_version=row_version+1,updated_at=NOW()
               WHERE id=$1 AND decision_kind IS NULL"#,
        )
        .bind(draft.work_item_id)
        .bind(draft.candidate_id)
        .execute(&mut *connection)
        .await?;
        candidate_ids.push(draft.candidate_id);
    }

    let mut no_candidate_ids = Vec::with_capacity(command.no_candidate_decisions.len());
    for decision in &command.no_candidate_decisions {
        if decision.reason_code.trim().is_empty() || decision.detail.trim().is_empty() {
            return Err(attack_conflict(
                "no_candidate requires stable reason and detail",
            ));
        }
        let item = by_id
            .get(&decision.work_item_id)
            .ok_or_else(|| attack_conflict("no_candidate work item is outside manifest"))?;
        if !evidence_is_grounded_in_work_item(connection, item, &decision.evidence_ids).await? {
            return Err(attack_conflict(
                "no_candidate evidence is not grounded in its work item",
            ));
        }
        for evidence_id in &decision.evidence_ids {
            sqlx::query(
                r#"INSERT INTO attack_candidate_work_item_evidence(work_item_id,evidence_id,role)
                   VALUES ($1,$2,'decision') ON CONFLICT DO NOTHING"#,
            )
            .bind(decision.work_item_id)
            .bind(evidence_id)
            .execute(&mut *connection)
            .await?;
        }
        sqlx::query(
            r#"UPDATE attack_candidate_work_items
               SET decision_kind='no_candidate',no_candidate_reason_code=$2,
                   no_candidate_detail=$3,decided_at=NOW(),
                   row_version=row_version+1,updated_at=NOW()
               WHERE id=$1 AND decision_kind IS NULL"#,
        )
        .bind(decision.work_item_id)
        .bind(&decision.reason_code)
        .bind(&decision.detail)
        .execute(&mut *connection)
        .await?;
        no_candidate_ids.push(decision.work_item_id);
    }
    let moved = sqlx::query(
        r#"UPDATE attack_wave_units
              SET status='review',row_version=row_version+1,updated_at=NOW()
            WHERE id=$1 AND wave_run_id=$2 AND operation_id=$3
              AND scope_snapshot_id=$4 AND organization_id=$5
              AND status IN ('open','reasoning') AND NOT review_closed
              AND terminal_at IS NULL"#,
    )
    .bind(command.wave_unit_id)
    .bind(command.wave_run_id)
    .bind(command.operation_id)
    .bind(command.scope_snapshot_id)
    .bind(command.organization_id)
    .execute(&mut *connection)
    .await?;
    if moved.rows_affected() != 1 {
        return Err(attack_conflict("Candidate WaveUnit review transition lost"));
    }
    Ok(AcceptedCandidateBatch {
        candidate_ids,
        no_candidate_work_item_ids: no_candidate_ids,
        replayed: false,
    })
}

pub async fn accept_v2_batch_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    command: AcceptCandidateBatch,
) -> crate::Result<AcceptedCandidateBatch> {
    accept_gate_passed_candidate_batch(tx, command).await
}

/// 列某 operation 的全部候选（org 隔离）。
pub async fn list_by_operation(
    pool: &PgPool,
    operation_id: &str,
    organization_id: Option<Uuid>,
) -> Result<Vec<AttackCandidateRow>> {
    let rows = sqlx::query_as::<_, AttackCandidateRow>(LIST_BY_OPERATION_SQL)
        .bind(operation_id)
        .bind(organization_id)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// 列某 operation 某一波的候选（org 隔离）。
pub async fn list_by_wave(
    pool: &PgPool,
    operation_id: &str,
    organization_id: Option<Uuid>,
    wave: i32,
) -> Result<Vec<AttackCandidateRow>> {
    let rows = sqlx::query_as::<_, AttackCandidateRow>(LIST_BY_WAVE_SQL)
        .bind(operation_id)
        .bind(organization_id)
        .bind(wave)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// 更新一条候选的 disposition（IDOR 三重限定）。返回是否命中一行。
pub async fn update_disposition(
    pool: &PgPool,
    candidate_id: Uuid,
    operation_id: &str,
    organization_id: Option<Uuid>,
    disposition: &str,
) -> Result<bool> {
    let res = sqlx::query(UPDATE_DISPOSITION_SQL)
        .bind(candidate_id)
        .bind(operation_id)
        .bind(organization_id)
        .bind(disposition)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hypothesis_hash_is_deterministic_and_normalizes() {
        let a = hypothesis_hash(
            "api.example.com",
            Some("WSTG-ATHZ-04"),
            "IDOR on /orders/{id}",
        );
        let b = hypothesis_hash(
            "api.example.com",
            Some("WSTG-ATHZ-04"),
            "IDOR on /orders/{id}",
        );
        assert_eq!(a, b, "same inputs → same hash");
        // 64 hex chars = sha256.
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        // 措辞抖动（大小写 / 多空格 / 首尾空白）归一后同 hash。
        let c = hypothesis_hash(
            "api.example.com",
            Some("WSTG-ATHZ-04"),
            "  IDOR  ON   /orders/{id}  ",
        );
        assert_eq!(a, c, "whitespace/case drift must collapse to the same hash");
    }

    #[test]
    fn hypothesis_hash_distinguishes_target_technique_and_text() {
        let base = hypothesis_hash("a", Some("T1"), "h");
        assert_ne!(
            base,
            hypothesis_hash("b", Some("T1"), "h"),
            "target matters"
        );
        assert_ne!(
            base,
            hypothesis_hash("a", Some("T2"), "h"),
            "technique matters"
        );
        assert_ne!(base, hypothesis_hash("a", Some("T1"), "h2"), "text matters");
        // None technique ≠ empty-string technique collision guard: both normalize
        // to "" so they SHOULD collide (documented MVP behavior).
        assert_eq!(
            hypothesis_hash("a", None, "h"),
            hypothesis_hash("a", Some(""), "h")
        );
    }

    #[test]
    fn upsert_sql_dedupes_on_op_target_hash_and_returns_id() {
        assert!(UPSERT_LEGACY_SQL.contains("ON CONFLICT (operation_id, target, hypothesis_hash)"));
        assert!(UPSERT_LEGACY_SQL.contains("WHERE operation_uuid IS NULL DO UPDATE"));
        assert!(UPSERT_LEGACY_SQL.contains("RETURNING candidate_id"));
        assert!(UPSERT_LEGACY_SQL.contains("updated_at = NOW()"));
        // 冲突时刷新 disposition（状态机推进）但不动 candidate_id / created_at。
        assert!(UPSERT_LEGACY_SQL.contains("disposition = EXCLUDED.disposition"));
        assert!(!UPSERT_LEGACY_SQL.contains("candidate_id = EXCLUDED.candidate_id"));
        assert!(!UPSERT_LEGACY_SQL.contains("created_at = EXCLUDED.created_at"));
    }

    #[test]
    fn reads_are_org_isolated() {
        // I2：operation_id + organization_id IS NOT DISTINCT FROM（NULL=project 与
        // 具体 org 都精确隔离）。
        for sql in [LIST_BY_OPERATION_SQL, LIST_BY_WAVE_SQL] {
            assert!(sql.contains("operation_id = $1"));
            assert!(sql.contains("organization_id IS NOT DISTINCT FROM $2"));
        }
        assert!(LIST_BY_WAVE_SQL.contains("wave = $3"));
    }

    #[test]
    fn update_disposition_sql_is_idor_scoped() {
        // 越权防护：改 disposition 必须匹配 candidate_id + operation_id + org 三者。
        assert!(UPDATE_DISPOSITION_SQL.contains("WHERE candidate_id = $1"));
        assert!(UPDATE_DISPOSITION_SQL.contains("operation_id = $2"));
        assert!(UPDATE_DISPOSITION_SQL.contains("organization_id IS NOT DISTINCT FROM $3"));
        assert!(UPDATE_DISPOSITION_SQL.contains("disposition = $4"));
    }

    #[test]
    fn prior_refs_json_serializes_to_array() {
        let v = prior_refs_json(&["CVE-2024-1".to_string(), "wiki:foo".to_string()]);
        assert_eq!(v, serde_json::json!(["CVE-2024-1", "wiki:foo"]));
        assert_eq!(prior_refs_json(&[]), serde_json::json!([]));
    }
}
