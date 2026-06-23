//! `technique_outcomes` 物化表读写（#4 / E3，设计
//! `docs/design/2026-06-23-technique-outcomes-provenance.md`）。
//!
//! coverage gate 的单一真值源 + provenance：每 `(run × asset × technique)` 一行，
//! 带 outcome + source/query/confidence/evidence_ids/collected_at。命令路径与
//! enrich/landing 落库点都 **upsert** 这里（PR-C step2 写路径）；gate 后续从这里投影
//! `EvidenceFact`（PR-D 读路径，灰度）。
//!
//! 纯 runtime sqlx（无 `query!` 宏 → 无需编译期 DB）；SQL 抽成 `const` 便于单测。
//! I2：一切读写按 `organization_id` 过滤。I8：`outcome=empty` 只来自真「跑了→空」；
//! 缺行 = not_attempted（gate 照旧 BLOCK）。I7：`evidence_ids` 指 audit_log 真实行。

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// 一条 technique_outcome 的写入参数（provenance 全量）。`asset` 必须由调用方先过
/// `canonical_asset_key().key` 归一（E1），否则 gate join 漂移。
#[derive(Debug, Clone)]
pub struct TechniqueOutcomeWrite {
    pub organization_id: Uuid,
    pub run_id: String,
    pub asset: String,
    pub technique: String,
    /// `found` | `empty` | `error` | `blocked`（与 `EvidenceOutcome` + T2 对齐）。
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

/// upsert：`UNIQUE(run_id, asset, technique)` 冲突 → 更新 outcome/provenance/
/// evidence_ids/collected_at/updated_at，**seq 保持首插值**（幂等不堆叠）。首插
/// `seq = COALESCE(MAX(seq),0)+1 WHERE run_id`（D2：每 run 从 1 自增；并发以 UNIQUE
/// 兜底，seq 仅排序提示）。
const UPSERT_SQL: &str = "\
INSERT INTO technique_outcomes \
  (organization_id, run_id, asset, technique, outcome, source, query, \
   result_count, confidence, evidence_ids, seq, collected_at) \
VALUES \
  ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, \
   (SELECT COALESCE(MAX(seq), 0) + 1 FROM technique_outcomes WHERE run_id = $2), \
   $11) \
ON CONFLICT (run_id, asset, technique) DO UPDATE SET \
  outcome = EXCLUDED.outcome, \
  source = EXCLUDED.source, \
  query = EXCLUDED.query, \
  result_count = EXCLUDED.result_count, \
  confidence = EXCLUDED.confidence, \
  evidence_ids = EXCLUDED.evidence_ids, \
  collected_at = EXCLUDED.collected_at, \
  updated_at = NOW()";

/// 读某 run 的全部维（org 隔离，IDOR），按 seq 排序。供 PR-D gate 投影 + 诊断。
const LIST_FOR_RUN_SQL: &str = "\
SELECT asset, technique, outcome, source, evidence_ids, collected_at \
FROM technique_outcomes \
WHERE organization_id = $1 AND run_id = $2 \
ORDER BY seq";

/// upsert 一条 technique_outcome（PR-C step2 写路径）。
pub async fn upsert(pool: &PgPool, w: &TechniqueOutcomeWrite) -> Result<()> {
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
        .execute(pool)
        .await?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_sql_targets_unique_key_and_keeps_seq_on_conflict() {
        // 冲突键 = (run_id, asset, technique)；冲突时更新 provenance 但不动 seq。
        assert!(UPSERT_SQL.contains("ON CONFLICT (run_id, asset, technique) DO UPDATE"));
        assert!(!UPSERT_SQL.contains("seq = EXCLUDED.seq"), "seq must NOT be updated on conflict");
        assert!(UPSERT_SQL.contains("updated_at = NOW()"));
    }

    #[test]
    fn upsert_sql_seq_is_per_run_autoincrement() {
        // D2：首插 seq = 该 run 内 MAX(seq)+1。
        assert!(UPSERT_SQL.contains("COALESCE(MAX(seq), 0) + 1 FROM technique_outcomes WHERE run_id = $2"));
    }

    #[test]
    fn upsert_sql_writes_provenance_columns() {
        for col in ["outcome", "source", "query", "result_count", "confidence", "evidence_ids", "collected_at"] {
            assert!(UPSERT_SQL.contains(col), "upsert must write {col}");
            assert!(
                UPSERT_SQL.contains(&format!("{col} = EXCLUDED.{col}")),
                "conflict update must refresh {col}"
            );
        }
    }

    #[test]
    fn list_for_run_sql_is_org_isolated_and_ordered() {
        // I2：org 过滤；按 seq 稳定排序。
        assert!(LIST_FOR_RUN_SQL.contains("WHERE organization_id = $1 AND run_id = $2"));
        assert!(LIST_FOR_RUN_SQL.contains("ORDER BY seq"));
    }
}
