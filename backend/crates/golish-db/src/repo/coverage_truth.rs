//! Coverage gate 的 DB 业务表真值查询（设计 2026-06-12 §5.3）。
//!
//! 只读地回答「某 org / in-scope 资产，在业务表里某类被动情报技术是否真有数据」，
//! 供 harness 外层 hook 转成 `Found` EvidenceFact 注入 coverage gate，使 coverage
//! 判定以 DB 真值为准（而非 agent 自报 / 命令派生）。
//!
//! 红线（设计 §4）：
//! - 只产「有数据」(Found 语义)；DB 无数据**绝不**推断 checked_empty (I8)。
//! - 只读 SELECT，不写库；gate 纯函数不变（查询在 golish-db，结果经 hook 注入）。
//! - org 维度过滤（`organization_id`）= coverage 资产盘按 organization 隔离
//!   （design 2026-06-09），避免跨 org 业务数据互相投影。

use std::collections::HashSet;

use sqlx::PgPool;
use uuid::Uuid;

use crate::Result;

/// 注册于 `technique_taxonomy.json` 的被动情报 technique id（已落点的四类）。
pub const TECH_ASN: &str = "GOLISH-INTEL-ASN";
pub const TECH_CT: &str = "GOLISH-INTEL-CT";
pub const TECH_SUBDOMAIN: &str = "GOLISH-INTEL-SUBDOMAIN";
pub const TECH_DNS: &str = "GOLISH-INTEL-DNS";

/// org 级情报存量：`asns` / `certificates` 专列是否非空（JSONB 数组长度 > 0）。
fn build_org_intel_presence_sql() -> String {
    "SELECT (jsonb_array_length(asns) > 0) AS has_asn, \
            (jsonb_array_length(certificates) > 0) AS has_ct \
       FROM organizations WHERE id = $1"
        .to_string()
}

/// 该 org 下 scope='in' 的 target 中，哪些 `value` 真有 `asset_type='subdomain'` 子资产行。
/// `$1 IS NULL` 时不按 org 过滤（退回全局 scope='in'，与 `targets::list_in_scope_values` 同款）。
fn build_subdomain_target_values_sql() -> String {
    "SELECT DISTINCT t.value FROM targets t \
       JOIN target_assets ta ON ta.target_id = t.id \
       WHERE t.scope::text = 'in' \
         AND ($1 IS NULL OR t.organization_id = $1) \
         AND ta.asset_type = 'subdomain'"
        .to_string()
}

/// 纯组装（与 IO 解耦，便于单测）：对每个 in-scope asset，按业务表存量产 `(asset, technique)`。
/// 顺序确定（每 asset 内 ASN→CT→SUBDOMAIN，外层按 `in_scope_assets` 顺序），便于断言。
pub(crate) fn assemble_truth_facts(
    in_scope_assets: &[String],
    has_asn: bool,
    has_ct: bool,
    subdomain_values: &HashSet<String>,
    dns_values: &HashSet<String>,
) -> Vec<(String, &'static str)> {
    let mut facts = Vec::new();
    for asset in in_scope_assets {
        if has_asn {
            facts.push((asset.clone(), TECH_ASN));
        }
        if has_ct {
            facts.push((asset.clone(), TECH_CT));
        }
        if subdomain_values.contains(asset) {
            facts.push((asset.clone(), TECH_SUBDOMAIN));
        }
        if dns_values.contains(asset) {
            facts.push((asset.clone(), TECH_DNS));
        }
    }
    facts
}

/// DB 业务表真值事实 `(asset, technique)`：业务表里 `asset` 上 `technique` 真有数据。
///
/// `in_scope_assets` 是 coverage gate 实际遍历的权威资产集（org 已隔离），保证与
/// `coverage_complete` 的 asset 维度对齐。`org_id=None` 时不查 org 级情报（ASN/CT 不
/// 投影），SUBDOMAIN 退回全局 scope='in'。空 in-scope → 直接返回空（D1）。
pub async fn coverage_truth_facts(
    pool: &PgPool,
    org_id: Option<Uuid>,
    in_scope_assets: &[String],
) -> Result<Vec<(String, &'static str)>> {
    if in_scope_assets.is_empty() {
        return Ok(Vec::new());
    }
    let (has_asn, has_ct) = match org_id {
        Some(id) => sqlx::query_as::<_, (bool, bool)>(&build_org_intel_presence_sql())
            .bind(id)
            .fetch_optional(pool)
            .await?
            .unwrap_or((false, false)),
        None => (false, false),
    };
    let subdomain_values: HashSet<String> =
        sqlx::query_scalar::<_, String>(&build_subdomain_target_values_sql())
            .bind(org_id)
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect();
    // DNS 维度（PR-B）：复用 dns_records repo 的存在查询（DRY），org 隔离。
    let dns_values: HashSet<String> = crate::repo::dns_records::present_target_values(pool, org_id)
        .await?
        .into_iter()
        .collect();
    Ok(assemble_truth_facts(
        in_scope_assets,
        has_asn,
        has_ct,
        &subdomain_values,
        &dns_values,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subs(values: &[&str]) -> HashSet<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn org_intel_presence_sql_reads_asn_and_cert_columns() {
        let sql = build_org_intel_presence_sql();
        assert!(sql.contains("jsonb_array_length(asns) > 0"));
        assert!(sql.contains("jsonb_array_length(certificates) > 0"));
        assert!(sql.contains("FROM organizations WHERE id = $1"));
    }

    #[test]
    fn subdomain_sql_filters_scope_org_and_asset_type() {
        let sql = build_subdomain_target_values_sql();
        assert!(sql.contains("t.scope::text = 'in'"));
        assert!(sql.contains("($1 IS NULL OR t.organization_id = $1)"));
        assert!(sql.contains("ta.asset_type = 'subdomain'"));
        assert!(sql.contains("JOIN target_assets ta ON ta.target_id = t.id"));
    }

    #[test]
    fn assemble_empty_in_scope_yields_no_facts() {
        let out = assemble_truth_facts(&[], true, true, &subs(&["a.com"]), &subs(&["a.com"]));
        assert!(out.is_empty(), "no in-scope asset → no fact (D1 维度对齐)");
    }

    #[test]
    fn assemble_org_intel_applies_to_every_in_scope_asset() {
        let assets = vec!["moresec.cn".to_string(), "sub.moresec.cn".to_string()];
        let out = assemble_truth_facts(&assets, true, false, &HashSet::new(), &HashSet::new());
        // has_asn=true → 每个 in-scope asset 产 ASN；has_ct=false → 无 CT。
        assert_eq!(
            out,
            vec![
                ("moresec.cn".to_string(), TECH_ASN),
                ("sub.moresec.cn".to_string(), TECH_ASN),
            ]
        );
    }

    #[test]
    fn assemble_subdomain_only_for_targets_with_children() {
        let assets = vec!["moresec.cn".to_string(), "other.cn".to_string()];
        let out = assemble_truth_facts(
            &assets,
            false,
            false,
            &subs(&["moresec.cn"]),
            &HashSet::new(),
        );
        // 只有 moresec.cn 有子域资产行 → 只它产 SUBDOMAIN。
        assert_eq!(out, vec![("moresec.cn".to_string(), TECH_SUBDOMAIN)]);
    }

    #[test]
    fn assemble_dns_only_for_targets_with_records() {
        let assets = vec!["moresec.cn".to_string(), "other.cn".to_string()];
        let out = assemble_truth_facts(
            &assets,
            false,
            false,
            &HashSet::new(),
            &subs(&["moresec.cn"]),
        );
        // 只有 moresec.cn 有 DNS 记录行 → 只它产 DNS。
        assert_eq!(out, vec![("moresec.cn".to_string(), TECH_DNS)]);
    }

    #[test]
    fn assemble_combines_all_dimensions_in_stable_order() {
        let out = assemble_truth_facts(
            &["a.com".to_string()],
            true,
            true,
            &subs(&["a.com"]),
            &subs(&["a.com"]),
        );
        assert_eq!(
            out,
            vec![
                ("a.com".to_string(), TECH_ASN),
                ("a.com".to_string(), TECH_CT),
                ("a.com".to_string(), TECH_SUBDOMAIN),
                ("a.com".to_string(), TECH_DNS),
            ]
        );
    }
}
