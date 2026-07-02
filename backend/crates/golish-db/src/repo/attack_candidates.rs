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
use sqlx::PgPool;
use uuid::Uuid;

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
const UPSERT_SQL: &str = "\
INSERT INTO attack_candidates \
  (candidate_id, operation_id, organization_id, target, hypothesis, hypothesis_hash, \
   technique, rationale, prior_refs, suggested_approach, priority, wave, \
   parent_finding_id, disposition) \
VALUES \
  ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) \
ON CONFLICT (operation_id, target, hypothesis_hash) DO UPDATE SET \
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
       parent_finding_id, disposition, created_at, updated_at \
FROM attack_candidates \
WHERE operation_id = $1 AND organization_id IS NOT DISTINCT FROM $2 \
ORDER BY wave, created_at";

/// 列某 operation 某一波（org 隔离，IDOR）的候选。
const LIST_BY_WAVE_SQL: &str = "\
SELECT candidate_id, operation_id, organization_id, target, hypothesis, hypothesis_hash, \
       technique, rationale, prior_refs, suggested_approach, priority, wave, \
       parent_finding_id, disposition, created_at, updated_at \
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
pub async fn upsert_by_hash(pool: &PgPool, w: &AttackCandidateWrite) -> Result<Uuid> {
    let hash = hypothesis_hash(&w.target, w.technique.as_deref(), &w.hypothesis);
    let id: Uuid = sqlx::query_scalar(UPSERT_SQL)
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

/// 创建一条候选（等价 upsert：同假设重复提交返回既有 id，避免重复行）。
pub async fn create(pool: &PgPool, w: &AttackCandidateWrite) -> Result<Uuid> {
    upsert_by_hash(pool, w).await
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
        assert!(
            UPSERT_SQL.contains("ON CONFLICT (operation_id, target, hypothesis_hash) DO UPDATE")
        );
        assert!(UPSERT_SQL.contains("RETURNING candidate_id"));
        assert!(UPSERT_SQL.contains("updated_at = NOW()"));
        // 冲突时刷新 disposition（状态机推进）但不动 candidate_id / created_at。
        assert!(UPSERT_SQL.contains("disposition = EXCLUDED.disposition"));
        assert!(!UPSERT_SQL.contains("candidate_id = EXCLUDED.candidate_id"));
        assert!(!UPSERT_SQL.contains("created_at = EXCLUDED.created_at"));
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
