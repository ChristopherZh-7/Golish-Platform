//! Coverage gate 的 DB 业务表真值查询（设计 2026-06-12 §5.3 + Phase 1 §5）。
//!
//! 只读地回答「某 org / in-scope 资产，在业务表里某类技术是否真有数据」，供
//! harness 外层 hook 转成 `Found` EvidenceFact 注入 coverage gate，使 coverage
//! 判定以 DB 真值为准（而非 agent 自报 / 命令派生）。
//!
//! 覆盖技术（Phase 0 被动 4 类 + Phase 1 被动 2 类 + 主动 6 类 = 12 维）：
//! - 被动情报（target_intel）：ASN / CT / WHOIS（org 级专列）、OSINT（org 级
//!   intel/contacts/social/business 任一非空）、SUBDOMAIN / DNS（per-asset）。
//! - 主动攻击面（external_attack_surface）：LIVENESS / PORT / SERVICE-FINGERPRINT。
//! - 内容枚举（enumeration）：DIR / PARAM / JSAPI。
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

/// 被动情报 technique id（target_intel）。
pub const TECH_ASN: &str = "GOLISH-INTEL-ASN";
pub const TECH_CT: &str = "GOLISH-INTEL-CT";
pub const TECH_SUBDOMAIN: &str = "GOLISH-INTEL-SUBDOMAIN";
pub const TECH_DNS: &str = "GOLISH-INTEL-DNS";
pub const TECH_WHOIS: &str = "GOLISH-INTEL-WHOIS";
pub const TECH_OSINT: &str = "GOLISH-INTEL-OSINT";
/// 子公司 / org 树发现 technique（scoping 阶段；Phase 2 2026-06-12-redteam-phase2）。
pub const TECH_SUBSIDIARY: &str = "GOLISH-INTEL-SUBSIDIARY";

/// 主动攻击面 technique id（external_attack_surface）。
pub const TECH_EAS_LIVENESS: &str = "GOLISH-EAS-LIVENESS";
pub const TECH_EAS_PORT: &str = "GOLISH-EAS-PORT";
pub const TECH_EAS_SERVICE_FP: &str = "GOLISH-EAS-SERVICE-FINGERPRINT";

/// 内容枚举 technique id（enumeration）。
pub const TECH_ENUM_DIR: &str = "GOLISH-ENUM-DIR";
pub const TECH_ENUM_PARAM: &str = "GOLISH-ENUM-PARAM";
pub const TECH_ENUM_JSAPI: &str = "GOLISH-ENUM-JSAPI";

/// org 级情报存量（一次查询返回四个 bool）：
/// - `has_asn` / `has_ct`：`asns` / `certificates` JSONB 数组非空。
/// - `has_whois`：`whois` 专列非 NULL 且非 `'null'`/`'{}'`（Phase 1）。
/// - `has_osint`：`intel.records` / `contacts` / `social_accounts` /
///   `business_systems` 任一非空（OSINT 经 provider enrich 落这些列；Phase 1）。
fn build_org_intel_presence_sql() -> String {
    "SELECT (jsonb_array_length(asns) > 0) AS has_asn, \
            (jsonb_array_length(certificates) > 0) AS has_ct, \
            (whois IS NOT NULL AND whois <> 'null'::jsonb AND whois <> '{}'::jsonb) AS has_whois, \
            (COALESCE(jsonb_array_length(CASE WHEN jsonb_typeof(intel->'records') = 'array' \
                          THEN intel->'records' END), 0) > 0 \
             OR jsonb_array_length(contacts) > 0 \
             OR jsonb_array_length(social_accounts) > 0 \
             OR jsonb_array_length(business_systems) > 0) AS has_osint, \
            (EXISTS(SELECT 1 FROM organizations child \
                      WHERE child.parent_id = organizations.id)) AS has_subsidiary \
       FROM organizations WHERE id = $1"
        .to_string()
}

/// 通用模板：该 org 下 scope='in' 的 target 中，满足 `extra` 条件的 `value` 集合。
/// `$1 IS NULL` 时不按 org 过滤（退回全局 scope='in'）。`extra` 形如
/// `"AND jsonb_array_length(t.ports) > 0"` 或 `"JOIN fingerprints f ON ..."`。
fn build_in_scope_values_sql(join: &str, filter: &str) -> String {
    format!(
        "SELECT DISTINCT t.value FROM targets t {join} \
           WHERE t.scope::text = 'in' \
             AND ($1 IS NULL OR t.organization_id = $1) {filter}"
    )
}

/// 该 org 下 scope='in' 的 target 中，哪些 `value` 真有 `asset_type='subdomain'` 子资产行。
fn build_subdomain_target_values_sql() -> String {
    build_in_scope_values_sql(
        "JOIN target_assets ta ON ta.target_id = t.id",
        "AND ta.asset_type = 'subdomain'",
    )
}

/// EAS-LIVENESS：httpx 探活/解析 IP（`http_status` 非空或 `real_ip` 非空）。
fn build_liveness_values_sql() -> String {
    build_in_scope_values_sql("", "AND (t.http_status IS NOT NULL OR t.real_ip <> '')")
}

/// EAS-PORT：端口扫描结果（`ports` JSONB 数组非空）。
fn build_port_values_sql() -> String {
    build_in_scope_values_sql("", "AND jsonb_array_length(t.ports) > 0")
}

/// EAS-SERVICE-FINGERPRINT：该 host 有服务/版本指纹行。
fn build_service_fp_values_sql() -> String {
    build_in_scope_values_sql("JOIN fingerprints f ON f.target_id = t.id", "")
}

/// ENUM-DIR：该 host 有目录枚举产物（ffuf/gobuster → directory_entries）。
fn build_dir_values_sql() -> String {
    build_in_scope_values_sql("JOIN directory_entries de ON de.target_id = t.id", "")
}

/// ENUM-PARAM：该 host 有带参端点（arjun/katana → api_endpoints.params 非空）。
fn build_param_values_sql() -> String {
    build_in_scope_values_sql(
        "JOIN api_endpoints ae ON ae.target_id = t.id",
        "AND jsonb_array_length(ae.params) > 0",
    )
}

/// ENUM-JSAPI：该 host 有 JS/爬虫抽取的端点（api_endpoints.source）。
fn build_jsapi_values_sql() -> String {
    build_in_scope_values_sql(
        "JOIN api_endpoints ae ON ae.target_id = t.id",
        "AND ae.source IN ('js_analysis', 'crawler')",
    )
}

/// 纯组装入参（与 IO 解耦，便于单测）。bool = org 级存量；HashSet = per-asset 命中集。
pub(crate) struct TruthInputs<'a> {
    pub has_asn: bool,
    pub has_ct: bool,
    pub has_whois: bool,
    pub has_osint: bool,
    /// org 级：该 engagement org 是否有任意 child org（子公司已落 org 树）。
    pub has_subsidiary: bool,
    pub subdomain_values: &'a HashSet<String>,
    pub dns_values: &'a HashSet<String>,
    pub liveness_values: &'a HashSet<String>,
    pub port_values: &'a HashSet<String>,
    pub service_fp_values: &'a HashSet<String>,
    pub dir_values: &'a HashSet<String>,
    pub param_values: &'a HashSet<String>,
    pub jsapi_values: &'a HashSet<String>,
}

/// 纯组装（与 IO 解耦，便于单测）：对每个 in-scope asset，按业务表存量产 `(asset, technique)`。
/// 顺序确定（每 asset 内固定 12 维顺序，外层按 `in_scope_assets` 顺序），便于断言。
pub(crate) fn assemble_truth_facts(
    in_scope_assets: &[String],
    inputs: &TruthInputs<'_>,
) -> Vec<(String, &'static str)> {
    let mut facts = Vec::new();
    for asset in in_scope_assets {
        // org 级存量：命中即对每个 in-scope 资产产同一 technique。
        if inputs.has_asn {
            facts.push((asset.clone(), TECH_ASN));
        }
        if inputs.has_ct {
            facts.push((asset.clone(), TECH_CT));
        }
        if inputs.has_whois {
            facts.push((asset.clone(), TECH_WHOIS));
        }
        if inputs.has_osint {
            facts.push((asset.clone(), TECH_OSINT));
        }
        // org 级：该 engagement org 有任意 child org（子公司发现已落 org 树）→
        // 对每个 in-scope asset 标 SUBSIDIARY found（scoping 用；Phase 2）。
        if inputs.has_subsidiary {
            facts.push((asset.clone(), TECH_SUBSIDIARY));
        }
        // per-asset 命中集。
        if inputs.subdomain_values.contains(asset) {
            facts.push((asset.clone(), TECH_SUBDOMAIN));
        }
        if inputs.dns_values.contains(asset) {
            facts.push((asset.clone(), TECH_DNS));
        }
        if inputs.liveness_values.contains(asset) {
            facts.push((asset.clone(), TECH_EAS_LIVENESS));
        }
        if inputs.port_values.contains(asset) {
            facts.push((asset.clone(), TECH_EAS_PORT));
        }
        if inputs.service_fp_values.contains(asset) {
            facts.push((asset.clone(), TECH_EAS_SERVICE_FP));
        }
        if inputs.dir_values.contains(asset) {
            facts.push((asset.clone(), TECH_ENUM_DIR));
        }
        if inputs.param_values.contains(asset) {
            facts.push((asset.clone(), TECH_ENUM_PARAM));
        }
        if inputs.jsapi_values.contains(asset) {
            facts.push((asset.clone(), TECH_ENUM_JSAPI));
        }
    }
    facts
}

/// per-asset host 级查询的小封装：跑一条 `build_in_scope_values_sql` 派生的 SQL，
/// 绑定 `org_id`，收 distinct `value` 成集合。
async fn fetch_values(pool: &PgPool, sql: &str, org_id: Option<Uuid>) -> Result<HashSet<String>> {
    Ok(sqlx::query_scalar::<_, String>(sql)
        .bind(org_id)
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect())
}

/// DB 业务表真值事实 `(asset, technique)`：业务表里 `asset` 上 `technique` 真有数据。
///
/// `in_scope_assets` 是 coverage gate 实际遍历的权威资产集（org 已隔离），保证与
/// `coverage_complete` 的 asset 维度对齐。`org_id=None` 时不查 org 级情报（ASN/CT/
/// WHOIS/OSINT 不投影），per-asset 维度退回全局 scope='in'。空 in-scope → 直接返回空。
pub async fn coverage_truth_facts(
    pool: &PgPool,
    org_id: Option<Uuid>,
    in_scope_assets: &[String],
) -> Result<Vec<(String, &'static str)>> {
    if in_scope_assets.is_empty() {
        return Ok(Vec::new());
    }
    let (has_asn, has_ct, has_whois, has_osint, has_subsidiary) = match org_id {
        Some(id) => {
            sqlx::query_as::<_, (bool, bool, bool, bool, bool)>(&build_org_intel_presence_sql())
                .bind(id)
                .fetch_optional(pool)
                .await?
                .unwrap_or((false, false, false, false, false))
        }
        None => (false, false, false, false, false),
    };
    let subdomain_values = fetch_values(pool, &build_subdomain_target_values_sql(), org_id).await?;
    // DNS 维度（PR-B）：复用 dns_records repo 的存在查询（DRY），org 隔离。
    let dns_values: HashSet<String> = crate::repo::dns_records::present_target_values(pool, org_id)
        .await?
        .into_iter()
        .collect();
    let liveness_values = fetch_values(pool, &build_liveness_values_sql(), org_id).await?;
    let port_values = fetch_values(pool, &build_port_values_sql(), org_id).await?;
    let service_fp_values = fetch_values(pool, &build_service_fp_values_sql(), org_id).await?;
    let dir_values = fetch_values(pool, &build_dir_values_sql(), org_id).await?;
    let param_values = fetch_values(pool, &build_param_values_sql(), org_id).await?;
    let jsapi_values = fetch_values(pool, &build_jsapi_values_sql(), org_id).await?;
    Ok(assemble_truth_facts(
        in_scope_assets,
        &TruthInputs {
            has_asn,
            has_ct,
            has_whois,
            has_osint,
            has_subsidiary,
            subdomain_values: &subdomain_values,
            dns_values: &dns_values,
            liveness_values: &liveness_values,
            port_values: &port_values,
            service_fp_values: &service_fp_values,
            dir_values: &dir_values,
            param_values: &param_values,
            jsapi_values: &jsapi_values,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subs(values: &[&str]) -> HashSet<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    /// 全空输入的基线 `TruthInputs`：测试只翻自己关心的那几维。
    fn empty_inputs<'a>(empty: &'a HashSet<String>) -> TruthInputs<'a> {
        TruthInputs {
            has_asn: false,
            has_ct: false,
            has_whois: false,
            has_osint: false,
            has_subsidiary: false,
            subdomain_values: empty,
            dns_values: empty,
            liveness_values: empty,
            port_values: empty,
            service_fp_values: empty,
            dir_values: empty,
            param_values: empty,
            jsapi_values: empty,
        }
    }

    #[test]
    fn org_intel_presence_sql_includes_subsidiary_child_exists() {
        let sql = build_org_intel_presence_sql();
        assert!(sql.contains("child.parent_id = organizations.id"));
        assert!(sql.contains("AS has_subsidiary"));
    }

    #[test]
    fn assemble_projects_subsidiary_to_every_in_scope_asset() {
        let empty = subs(&[]);
        let mut inputs = empty_inputs(&empty);
        inputs.has_subsidiary = true;
        let assets = vec!["moresec.cn".to_string(), "moresec.com".to_string()];
        let facts = assemble_truth_facts(&assets, &inputs);
        // org 级事实投影：每个 in-scope asset 都拿到一条 SUBSIDIARY found。
        assert_eq!(
            facts.iter().filter(|(_, t)| *t == TECH_SUBSIDIARY).count(),
            2
        );
        assert!(facts.contains(&("moresec.cn".to_string(), TECH_SUBSIDIARY)));
    }

    #[test]
    fn assemble_no_subsidiary_when_flag_false() {
        let empty = subs(&[]);
        let inputs = empty_inputs(&empty);
        let assets = vec!["moresec.cn".to_string()];
        let facts = assemble_truth_facts(&assets, &inputs);
        assert!(!facts.iter().any(|(_, t)| *t == TECH_SUBSIDIARY));
    }

    #[test]
    fn org_intel_presence_sql_reads_all_four_org_columns() {
        let sql = build_org_intel_presence_sql();
        assert!(sql.contains("jsonb_array_length(asns) > 0"));
        assert!(sql.contains("jsonb_array_length(certificates) > 0"));
        assert!(sql.contains("whois IS NOT NULL"));
        // OSINT 多源任一非空。
        assert!(sql.contains("intel->'records'"));
        assert!(sql.contains("jsonb_array_length(contacts) > 0"));
        assert!(sql.contains("jsonb_array_length(social_accounts) > 0"));
        assert!(sql.contains("jsonb_array_length(business_systems) > 0"));
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
    fn active_dimension_sqls_filter_scope_and_org() {
        // 6 个主动维度 SQL 都必须带 scope='in' + org 隔离（不串 org / 不漏 scope）。
        for sql in [
            build_liveness_values_sql(),
            build_port_values_sql(),
            build_service_fp_values_sql(),
            build_dir_values_sql(),
            build_param_values_sql(),
            build_jsapi_values_sql(),
        ] {
            assert!(sql.contains("t.scope::text = 'in'"), "missing scope: {sql}");
            assert!(
                sql.contains("($1 IS NULL OR t.organization_id = $1)"),
                "missing org filter: {sql}"
            );
        }
    }

    #[test]
    fn active_dimension_sqls_target_the_right_tables() {
        assert!(build_liveness_values_sql().contains("t.http_status IS NOT NULL OR t.real_ip"));
        assert!(build_port_values_sql().contains("jsonb_array_length(t.ports) > 0"));
        assert!(build_service_fp_values_sql().contains("JOIN fingerprints f ON f.target_id = t.id"));
        assert!(build_dir_values_sql().contains("JOIN directory_entries de ON de.target_id = t.id"));
        let param = build_param_values_sql();
        assert!(param.contains("JOIN api_endpoints ae ON ae.target_id = t.id"));
        assert!(param.contains("jsonb_array_length(ae.params) > 0"));
        let jsapi = build_jsapi_values_sql();
        assert!(jsapi.contains("JOIN api_endpoints ae ON ae.target_id = t.id"));
        assert!(jsapi.contains("ae.source IN ('js_analysis', 'crawler')"));
    }

    #[test]
    fn assemble_empty_in_scope_yields_no_facts() {
        let empty = HashSet::new();
        let out = assemble_truth_facts(
            &[],
            &TruthInputs {
                has_asn: true,
                has_ct: true,
                ..empty_inputs(&empty)
            },
        );
        assert!(out.is_empty(), "no in-scope asset → no fact (维度对齐)");
    }

    #[test]
    fn assemble_org_intel_applies_to_every_in_scope_asset() {
        let assets = vec!["moresec.cn".to_string(), "sub.moresec.cn".to_string()];
        let empty = HashSet::new();
        let out = assemble_truth_facts(
            &assets,
            &TruthInputs {
                has_asn: true,
                ..empty_inputs(&empty)
            },
        );
        assert_eq!(
            out,
            vec![
                ("moresec.cn".to_string(), TECH_ASN),
                ("sub.moresec.cn".to_string(), TECH_ASN),
            ]
        );
    }

    #[test]
    fn assemble_whois_and_osint_are_org_level() {
        let assets = vec!["a.com".to_string(), "b.com".to_string()];
        let empty = HashSet::new();
        let out = assemble_truth_facts(
            &assets,
            &TruthInputs {
                has_whois: true,
                has_osint: true,
                ..empty_inputs(&empty)
            },
        );
        // WHOIS/OSINT 是 org 级：每个 in-scope 资产都产，顺序 WHOIS→OSINT。
        assert_eq!(
            out,
            vec![
                ("a.com".to_string(), TECH_WHOIS),
                ("a.com".to_string(), TECH_OSINT),
                ("b.com".to_string(), TECH_WHOIS),
                ("b.com".to_string(), TECH_OSINT),
            ]
        );
    }

    #[test]
    fn assemble_subdomain_only_for_targets_with_children() {
        let assets = vec!["moresec.cn".to_string(), "other.cn".to_string()];
        let sub = subs(&["moresec.cn"]);
        let empty = HashSet::new();
        let out = assemble_truth_facts(
            &assets,
            &TruthInputs {
                subdomain_values: &sub,
                ..empty_inputs(&empty)
            },
        );
        assert_eq!(out, vec![("moresec.cn".to_string(), TECH_SUBDOMAIN)]);
    }

    #[test]
    fn assemble_dns_only_for_targets_with_records() {
        let assets = vec!["moresec.cn".to_string(), "other.cn".to_string()];
        let dns = subs(&["moresec.cn"]);
        let empty = HashSet::new();
        let out = assemble_truth_facts(
            &assets,
            &TruthInputs {
                dns_values: &dns,
                ..empty_inputs(&empty)
            },
        );
        assert_eq!(out, vec![("moresec.cn".to_string(), TECH_DNS)]);
    }

    #[test]
    fn assemble_each_active_dimension_only_for_matching_asset() {
        let assets = vec!["a.com".to_string(), "b.com".to_string()];
        let liveness = subs(&["a.com"]);
        let port = subs(&["a.com"]);
        let service_fp = subs(&["b.com"]);
        let dir = subs(&["a.com"]);
        let param = subs(&["b.com"]);
        let jsapi = subs(&["a.com"]);
        let out = assemble_truth_facts(
            &assets,
            &TruthInputs {
                has_asn: false,
                has_ct: false,
                has_whois: false,
                has_osint: false,
                has_subsidiary: false,
                subdomain_values: &HashSet::new(),
                dns_values: &HashSet::new(),
                liveness_values: &liveness,
                port_values: &port,
                service_fp_values: &service_fp,
                dir_values: &dir,
                param_values: &param,
                jsapi_values: &jsapi,
            },
        );
        assert_eq!(
            out,
            vec![
                ("a.com".to_string(), TECH_EAS_LIVENESS),
                ("a.com".to_string(), TECH_EAS_PORT),
                ("a.com".to_string(), TECH_ENUM_DIR),
                ("a.com".to_string(), TECH_ENUM_JSAPI),
                ("b.com".to_string(), TECH_EAS_SERVICE_FP),
                ("b.com".to_string(), TECH_ENUM_PARAM),
            ]
        );
    }

    #[test]
    fn assemble_combines_all_dimensions_in_stable_order() {
        let one = subs(&["a.com"]);
        let out = assemble_truth_facts(
            &["a.com".to_string()],
            &TruthInputs {
                has_asn: true,
                has_ct: true,
                has_whois: true,
                has_osint: true,
                has_subsidiary: true,
                subdomain_values: &one,
                dns_values: &one,
                liveness_values: &one,
                port_values: &one,
                service_fp_values: &one,
                dir_values: &one,
                param_values: &one,
                jsapi_values: &one,
            },
        );
        assert_eq!(
            out,
            vec![
                ("a.com".to_string(), TECH_ASN),
                ("a.com".to_string(), TECH_CT),
                ("a.com".to_string(), TECH_WHOIS),
                ("a.com".to_string(), TECH_OSINT),
                ("a.com".to_string(), TECH_SUBSIDIARY),
                ("a.com".to_string(), TECH_SUBDOMAIN),
                ("a.com".to_string(), TECH_DNS),
                ("a.com".to_string(), TECH_EAS_LIVENESS),
                ("a.com".to_string(), TECH_EAS_PORT),
                ("a.com".to_string(), TECH_EAS_SERVICE_FP),
                ("a.com".to_string(), TECH_ENUM_DIR),
                ("a.com".to_string(), TECH_ENUM_PARAM),
                ("a.com".to_string(), TECH_ENUM_JSAPI),
            ]
        );
    }
}
