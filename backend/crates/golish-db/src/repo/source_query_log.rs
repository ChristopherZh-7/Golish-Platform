//! `source_query_log` 物化表写入（#5，设计 `docs/design/2026-06-23-source-query-log.md`）。
//!
//! 被动情报「逐源查询日志」：每 `(run × source × query × target)` 一行，记录数据源
//! 查询的结果态 + 计数 + 用时 + 证据。比 `technique_outcomes`（每 `asset × technique`
//! 单行）更细——一个 technique 被多个源覆盖时各源各一行，用来证明「查过 CT / WHOIS /
//! OSINT / 代码平台——但为空 / 失败 / 无凭证」。
//!
//! 消费模型 A（审计 / provenance-only）：命令路径 / enrich 落库点 **upsert** 这里（写路径，
//! gray-switch）；reviewer / 报告 / `run_tree.py` 直接读 DB（**coverage gate 不读本表**）。
//!
//! 纯 runtime sqlx（无 `query!` 宏 → 无需编译期 DB）；SQL 抽成 `const` 便于单测。
//! I2：写按 `organization_id`。I8：`status=empty` 只来自真「跑了→空」；`error` = 失败
//! 阻断（承接 T2），二者不混。I7：`evidence_ids` 指 audit_log 真实行。

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// 一条 source_query_log 的写入参数。`target` 由调用方过 `canonical_asset_key().key`
/// 归一（E1）；org 级/非资产专属查询取 `""`（空串，UNIQUE 行为确定）。
#[derive(Debug, Clone)]
pub struct SourceQueryLogWrite {
    pub organization_id: Uuid,
    pub run_id: String,
    /// 数据源 / provider（`crt.sh` / `rdap` / `subfinder` / `ENScan_GO` …）。
    pub source: String,
    /// 实际查询 / 命令文本。
    pub query: String,
    /// 被查资产 canonical_asset_key；`""` = org 级 / 非资产专属。
    pub target: String,
    /// 贡献的 technique id（`GOLISH-INTEL-*`）；`None` = 未映射。
    pub technique: Option<String>,
    /// `found` | `empty` | `error` | `blocked`（与 `EvidenceOutcome` + T2 对齐）。
    pub status: String,
    pub result_count: Option<i32>,
    pub evidence_ids: Vec<i64>,
    pub detail: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// upsert：`UNIQUE(run_id, source, query, target)` 冲突 → 更新可变态（status / 计数 /
/// 证据 / timing / detail / updated_at），幂等不堆叠（重跑同 (源,查询,目标) 只刷新最新态）。
/// 键列（org/run/source/query/target）与 `created_at` 不在 SET 子句里。
const UPSERT_SQL: &str = "\
INSERT INTO source_query_log \
  (organization_id, run_id, source, query, target, technique, status, \
   result_count, evidence_ids, detail, started_at, finished_at) \
VALUES \
  ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) \
ON CONFLICT (run_id, source, query, target) DO UPDATE SET \
  technique = EXCLUDED.technique, \
  status = EXCLUDED.status, \
  result_count = EXCLUDED.result_count, \
  evidence_ids = EXCLUDED.evidence_ids, \
  detail = EXCLUDED.detail, \
  started_at = EXCLUDED.started_at, \
  finished_at = EXCLUDED.finished_at, \
  updated_at = NOW()";

/// upsert 一条 source_query_log（#5 写路径，gray-switch；调用方 warn-only、非致命）。
pub async fn upsert(pool: &PgPool, w: &SourceQueryLogWrite) -> Result<()> {
    sqlx::query(UPSERT_SQL)
        .bind(w.organization_id)
        .bind(&w.run_id)
        .bind(&w.source)
        .bind(&w.query)
        .bind(&w.target)
        .bind(w.technique.as_deref())
        .bind(&w.status)
        .bind(w.result_count)
        .bind(w.evidence_ids.as_slice())
        .bind(w.detail.as_deref())
        .bind(w.started_at)
        .bind(w.finished_at)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_sql_targets_unique_key() {
        assert!(UPSERT_SQL.contains("ON CONFLICT (run_id, source, query, target) DO UPDATE"));
        assert!(UPSERT_SQL.contains("updated_at = NOW()"));
    }

    #[test]
    fn upsert_sql_refreshes_mutable_columns_on_conflict() {
        for col in [
            "technique",
            "status",
            "result_count",
            "evidence_ids",
            "detail",
            "started_at",
            "finished_at",
        ] {
            assert!(
                UPSERT_SQL.contains(&format!("{col} = EXCLUDED.{col}")),
                "conflict update must refresh {col}"
            );
        }
    }

    #[test]
    fn upsert_sql_does_not_mutate_immutable_key_columns() {
        // 冲突键列 + created_at 不参与更新（幂等只刷新可变态）。
        for frag in [
            "organization_id = EXCLUDED",
            "run_id = EXCLUDED",
            "source = EXCLUDED",
            "query = EXCLUDED",
            "target = EXCLUDED",
            "created_at = EXCLUDED",
        ] {
            assert!(!UPSERT_SQL.contains(frag), "{frag} must NOT be in the conflict SET");
        }
    }

    #[test]
    fn upsert_sql_binds_exactly_twelve_columns() {
        for n in 1..=12 {
            assert!(UPSERT_SQL.contains(&format!("${n}")), "missing bind ${n}");
        }
        assert!(!UPSERT_SQL.contains("$13"), "must not bind a 13th column");
    }
}
