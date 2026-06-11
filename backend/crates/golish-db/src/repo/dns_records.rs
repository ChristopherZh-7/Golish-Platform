//! `dns_records` repository (design 2026-06-12 §5.2). Write-side upsert for
//! dig/dnsx ANSWER-SECTION rows; read-side presence query backing
//! `coverage_truth`'s GOLISH-INTEL-DNS projection. Owner: recon (DNS = recon
//! asset data).
//!
//! 红线：写只幂等 upsert；读只回答「哪些 in-scope target 真有 DNS 记录」(Found
//! 语义)，无记录 ≠ checked_empty (I8) —— 由 coverage gate 的缺口 BLOCK 体现。

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
fn build_dns_present_target_values_sql() -> String {
    "SELECT DISTINCT t.value FROM targets t \
       JOIN dns_records dr ON dr.target_id = t.id \
       WHERE t.scope::text = 'in' \
         AND ($1 IS NULL OR t.organization_id = $1)"
        .to_string()
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
/// 的 DNS 维度投影使用。
pub async fn present_target_values(pool: &PgPool, org_id: Option<Uuid>) -> Result<Vec<String>> {
    let rows = sqlx::query_scalar::<_, String>(&build_dns_present_target_values_sql())
        .bind(org_id)
        .fetch_all(pool)
        .await?;
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
        let sql = build_dns_present_target_values_sql();
        assert!(sql.contains("t.scope::text = 'in'"));
        assert!(sql.contains("($1 IS NULL OR t.organization_id = $1)"));
        assert!(sql.contains("JOIN dns_records dr ON dr.target_id = t.id"));
    }
}
