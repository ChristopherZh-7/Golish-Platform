//! `dns_records` repository (design 2026-06-12 §5.2). Write-side upsert for
//! dig/dnsx ANSWER-SECTION rows; read-side presence query backing
//! `coverage_truth`'s GOLISH-INTEL-DNS projection. Owner: recon (DNS = recon
//! asset data).
//!
//! 红线：写只幂等 upsert；读只回答「哪些 in-scope target 真有 DNS 记录」(Found
//! 语义)，无记录 ≠ checked_empty (I8) —— 由 coverage gate 的缺口 BLOCK 体现。

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::Result;

fn build_dns_upsert_sql() -> String {
    "INSERT INTO dns_records \
       (target_id, project_path, record_type, name, value, source) \
       VALUES ($1, $2, $3, $4, $5, $6) \
       ON CONFLICT (target_id, record_type, name, value) DO NOTHING"
        .to_string()
}

/// 哪些 in-scope target `value` 真有 DNS 记录（org 隔离，与 coverage_truth 的
/// subdomain 查询同款）。`org_id=None` → 全局 scope='in'。
///
/// Phase B row-level freshness (design 2026-06-22 §3.3): `apply_window` ⇒ only
/// count DNS records created this stage-run (`dr.created_at >= $2`), so records
/// landed by a previous run don't satisfy the GOLISH-INTEL-DNS cell this run.
/// `created_at` is NOT NULL (schema default NOW()), so no NULL-coalescing needed.
fn build_dns_present_target_values_sql(apply_window: bool) -> String {
    let window = if apply_window {
        " AND dr.created_at >= $2"
    } else {
        ""
    };
    format!(
        "SELECT DISTINCT t.value FROM targets t \
           JOIN dns_records dr ON dr.target_id = t.id \
           WHERE t.scope::text = 'in' \
             AND ($1 IS NULL OR t.organization_id = $1) \
             AND dr.project_path IS NOT DISTINCT FROM t.project_path{window}"
    )
}

/// 写入一条 DNS 记录（幂等，唯一键冲突即跳过）。
pub async fn upsert(
    pool: &PgPool,
    target_id: Uuid,
    project_path: &str,
    record_type: &str,
    name: &str,
    value: &str,
    source: &str,
) -> Result<()> {
    sqlx::query(&build_dns_upsert_sql())
        .bind(target_id)
        .bind(project_path)
        .bind(record_type)
        .bind(name)
        .bind(value)
        .bind(source)
        .execute(pool)
        .await?;
    Ok(())
}

/// in-scope target value 中真有 DNS 记录的集合（org 隔离）。供 `coverage_truth`
/// 的 DNS 维度投影使用。`run_start = Some` 启用 Phase B 行级新鲜度窗（只数本次
/// stage-run 期间落库的记录），`None` 退回 presence-only。
pub async fn present_target_values(
    pool: &PgPool,
    org_id: Option<Uuid>,
    run_start: Option<DateTime<Utc>>,
) -> Result<Vec<String>> {
    let sql = build_dns_present_target_values_sql(run_start.is_some());
    let mut query = sqlx::query_scalar::<_, String>(&sql).bind(org_id);
    if let Some(rs) = run_start {
        query = query.bind(rs);
    }
    let rows = query.fetch_all(pool).await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_sql_targets_dns_records_with_conflict_noop() {
        let sql = build_dns_upsert_sql();
        assert!(sql.contains("INSERT INTO dns_records"));
        assert!(sql.contains("target_id, project_path, record_type, name, value, source"));
        assert!(sql.contains("ON CONFLICT") && sql.contains("DO NOTHING"));
    }

    #[test]
    fn present_sql_filters_scope_and_org_and_joins_targets() {
        let sql = build_dns_present_target_values_sql(false);
        assert!(sql.contains("t.scope::text = 'in'"));
        assert!(sql.contains("($1 IS NULL OR t.organization_id = $1)"));
        assert!(sql.contains("JOIN dns_records dr ON dr.target_id = t.id"));
        assert!(sql.contains("dr.project_path IS NOT DISTINCT FROM t.project_path"));
    }

    #[test]
    fn present_sql_off_omits_row_level_window() {
        // freshness_window OFF (Phase B): presence-only, no `$2` / `created_at >=`
        // ⇒ a DNS record landed by a previous stage-run still counts (pre-change).
        let sql = build_dns_present_target_values_sql(false);
        assert!(!sql.contains("$2"), "off must bind only $1: {sql}");
        assert!(!sql.contains("created_at >="), "off must not window: {sql}");
    }

    #[test]
    fn present_sql_on_windows_dns_records_created_at() {
        // freshness_window ON (Phase B, design 2026-06-22 §3.3): the GOLISH-INTEL-DNS
        // dimension only counts records created this stage-run (`dr.created_at >= $2`).
        let sql = build_dns_present_target_values_sql(true);
        assert!(
            sql.contains("dr.created_at >= $2"),
            "on must window dns_records.created_at: {sql}"
        );
        assert!(sql.contains("JOIN dns_records dr ON dr.target_id = t.id"));
        assert!(sql.contains("t.scope::text = 'in'"));
    }
}
