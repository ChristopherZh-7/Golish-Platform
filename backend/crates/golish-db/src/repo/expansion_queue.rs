//! `expansion_queue` 物化表写入（#6，设计 `docs/design/2026-06-23-expansion-queue.md`）。
//!
//! 被动收集「待扩展线索」队列：每 `(run × lead_type × lead_value)` 一行——发现的子公司 /
//! 新域名 / github org / email 域 … 入队作 `pending`，reviewer / `run_tree.py` 据此报
//! 「高置信 pending 线索没追完」，堵「发现了子域/子公司却没递归深挖」。
//!
//! 消费模型 A（审计 / reviewer-only）：发现点 **enqueue** 这里（写路径，gray-switch）；
//! reviewer / 报告 / `run_tree.py` 直接读 DB（**coverage gate 不读 / 不 block 本表**）。
//! `status` / `processed_at` 列为 future B（gate 强制）预留。
//!
//! 纯 runtime sqlx（无 `query!` 宏 → 无需编译期 DB）；SQL 抽成 `const` 便于单测。
//! I2：写按 `organization_id`。I7：`evidence_ids` 指 audit_log 真实行。

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// 一条 expansion_queue 的入队参数。`lead_value` 是线索主体（公司名 / 域名等，**不**过
/// canonical_asset_key——子公司线索是公司名，非 in-scope 主机）。
#[derive(Debug, Clone)]
pub struct ExpansionLeadWrite {
    pub organization_id: Uuid,
    pub run_id: String,
    /// `new_domain` | `brand` | `app` | `github_org` | `subsidiary` | `email_domain`。
    pub lead_type: String,
    pub lead_value: String,
    pub source: Option<String>,
    pub confidence: Option<f32>,
    /// 入队恒 `pending`；`processed`/`skipped`/`blocked` 为 future B 预留。
    pub status: String,
    pub evidence_ids: Vec<i64>,
    pub detail: Option<String>,
    pub discovered_at: Option<DateTime<Utc>>,
}

/// enqueue：`UNIQUE(run_id, lead_type, lead_value)` 冲突 → 只刷新 provenance
/// （source/confidence/evidence_ids/updated_at），**不重置 status**（已处理的线索重复
/// 发现不退回 pending，保 future B 语义）、不动 discovered_at/detail/created_at。
const ENQUEUE_SQL: &str = "\
INSERT INTO expansion_queue \
  (organization_id, run_id, lead_type, lead_value, source, confidence, status, \
   evidence_ids, detail, discovered_at) \
VALUES \
  ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
ON CONFLICT (run_id, lead_type, lead_value) DO UPDATE SET \
  source = EXCLUDED.source, \
  confidence = EXCLUDED.confidence, \
  evidence_ids = EXCLUDED.evidence_ids, \
  updated_at = NOW()";

/// enqueue 一条待扩展线索（#6 写路径，gray-switch；调用方 warn-only、非致命）。
pub async fn enqueue(pool: &PgPool, w: &ExpansionLeadWrite) -> Result<()> {
    sqlx::query(ENQUEUE_SQL)
        .bind(w.organization_id)
        .bind(&w.run_id)
        .bind(&w.lead_type)
        .bind(&w.lead_value)
        .bind(w.source.as_deref())
        .bind(w.confidence)
        .bind(&w.status)
        .bind(w.evidence_ids.as_slice())
        .bind(w.detail.as_deref())
        .bind(w.discovered_at)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enqueue_sql_targets_unique_key() {
        assert!(ENQUEUE_SQL.contains("ON CONFLICT (run_id, lead_type, lead_value) DO UPDATE"));
        assert!(ENQUEUE_SQL.contains("updated_at = NOW()"));
    }

    #[test]
    fn enqueue_sql_does_not_reset_status_on_conflict() {
        // future B 语义：已处理线索重复发现不退回 pending（status 不在 SET）。
        assert!(
            !ENQUEUE_SQL.contains("status = EXCLUDED"),
            "status must NOT be reset on conflict"
        );
        // 键列 + discovered_at/detail/created_at 也不更新。
        for frag in [
            "run_id = EXCLUDED",
            "lead_type = EXCLUDED",
            "lead_value = EXCLUDED",
            "discovered_at = EXCLUDED",
            "created_at = EXCLUDED",
        ] {
            assert!(
                !ENQUEUE_SQL.contains(frag),
                "{frag} must NOT be in the conflict SET"
            );
        }
    }

    #[test]
    fn enqueue_sql_refreshes_provenance_on_conflict() {
        for col in ["source", "confidence", "evidence_ids"] {
            assert!(
                ENQUEUE_SQL.contains(&format!("{col} = EXCLUDED.{col}")),
                "conflict update must refresh {col}"
            );
        }
    }

    #[test]
    fn enqueue_sql_binds_exactly_ten_columns() {
        for n in 1..=10 {
            assert!(ENQUEUE_SQL.contains(&format!("${n}")), "missing bind ${n}");
        }
        assert!(!ENQUEUE_SQL.contains("$11"), "must not bind an 11th column");
    }
}
