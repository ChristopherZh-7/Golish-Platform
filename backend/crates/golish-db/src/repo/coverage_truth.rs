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

use chrono::{DateTime, Utc};
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

/// Host-aware coverage 2c-3 IP-native techniques (per-asset, IP/CIDR only):
/// reverse DNS (PTR) + RIR/netblock IP-WHOIS.
pub const TECH_RDNS: &str = "GOLISH-INTEL-RDNS";
pub const TECH_IPWHOIS: &str = "GOLISH-INTEL-IPWHOIS";

/// SQL IN-list of IP/CIDR `targets.type` values — host-aware 2c-3 IP-native
/// techniques apply only to these (mirrors `technique_resolver` Ip/Cidr classes).
const IP_TYPE_IN_LIST: &str = "('ip', 'ipv4', 'ipv6', 'ip_address', 'cidr', 'range', 'netblock')";

/// 主动攻击面 technique id（external_attack_surface）。
pub const TECH_EAS_LIVENESS: &str = "GOLISH-EAS-LIVENESS";
pub const TECH_EAS_PORT: &str = "GOLISH-EAS-PORT";
pub const TECH_EAS_SERVICE_FP: &str = "GOLISH-EAS-SERVICE-FINGERPRINT";

/// 内容枚举 technique id（enumeration）。
/// JS 资产收集（design 2026-07-01 §4.1）：真值 = 该 host 已落 js_analysis_results 行。
pub const TECH_ENUM_JS: &str = "GOLISH-ENUM-JS";
pub const TECH_ENUM_DIR: &str = "GOLISH-ENUM-DIR";
pub const TECH_ENUM_PARAM: &str = "GOLISH-ENUM-PARAM";
/// JSAPI 收窄语义（design 2026-07-01 §4.1）：从 JS/爬虫抽取的 API 端点（SQL 不变）。
pub const TECH_ENUM_JSAPI: &str = "GOLISH-ENUM-JSAPI";

/// 某 JSONB 列「有内容」的 shape 无关判据：把 SQL NULL / `'null'` / `'[]'` /
/// `'{}'` 视为空，其余视为有内容（对齐 `has_whois` 的比较式判据）。
///
/// 关键约束：**绝不**对该列调 `jsonb_array_length`——后者遇非数组（例如
/// `contacts` 被 recon 写成对象 `{email:[...]}`）会抛
/// `cannot get array length of a non-array`，使整条 presence 查询失败、
/// `coverage_truth_facts` 返回 Err、db_truth 投影被整体丢弃，scoping 子公司
/// gate 因此误判 not_attempted。比较式判据对数组/对象/标量/NULL 都安全，且
/// Postgres 不保证 `AND` 短路，故不能用 `jsonb_typeof(x)='array' AND jsonb_array_length(x)`。
fn jsonb_non_empty(col: &str) -> String {
    format!(
        "({col} IS NOT NULL AND {col} <> 'null'::jsonb \
          AND {col} <> '[]'::jsonb AND {col} <> '{{}}'::jsonb)"
    )
}

/// org 级情报存量（一次查询返回五个 bool）：
/// - `has_asn` / `has_ct`：`asns` / `certificates` 列非空。
/// - `has_whois`：`whois` 专列非 NULL 且非 `'null'`/`'{}'`（Phase 1）。
/// - `has_osint`：`intel.records`（数组）/ `contacts`（对象）/ `social_accounts` /
///   `business_systems` 任一非空（OSINT 经 provider enrich 落这些列；Phase 1）。
/// - `has_subsidiary`：存在任意 child org（scoping 用；Phase 2）。
///
/// 列存量判定一律走 [`jsonb_non_empty`]（不裸调 `jsonb_array_length`）；唯一保留
/// 的 `jsonb_array_length` 是 `intel->'records'`，且有 `CASE WHEN jsonb_typeof =
/// 'array'` 守卫，对非数组安全。
fn build_org_intel_presence_sql(apply_window: bool) -> String {
    // Per-dimension freshness window (design 2026-06-22 §3.2): when `apply_window`,
    // each of the 4 org intel dims additionally requires its `<dim>_collected_at
    // >= $2` (this stage-run start, `operation_state.stage_started_at`). A NULL
    // collected_at (legacy row / never-collected) fails `>= $2`, and the
    // `(expr AND col >= $2) IS TRUE` wrapper maps the resulting NULL to `false`
    // ⇒ not projected (conservative: no stale Found, honors I8) **and** keeps the
    // column non-NULL so it still decodes into `bool`. `apply_window=false` emits
    // no `*_collected_at` predicate at all (freshness_window gray-switch off =
    // pre-2026-06-22 presence-only behavior); the existing presence tests cover
    // that the column reads / shape-agnostic empty checks are unchanged.
    let dim = |expr: &str, col: &str| -> String {
        if apply_window {
            format!("(({expr}) AND {col} >= $2) IS TRUE")
        } else {
            expr.to_string()
        }
    };
    let osint_expr = format!(
        "(COALESCE(jsonb_array_length(CASE WHEN jsonb_typeof(intel->'records') = 'array' \
                      THEN intel->'records' END), 0) > 0 \
         OR {has_contacts} \
         OR {has_social} \
         OR {has_business})",
        has_contacts = jsonb_non_empty("contacts"),
        has_social = jsonb_non_empty("social_accounts"),
        has_business = jsonb_non_empty("business_systems"),
    );
    let whois_expr = "(whois IS NOT NULL AND whois <> 'null'::jsonb AND whois <> '{}'::jsonb)";
    format!(
        "SELECT {has_asn} AS has_asn, \
                {has_ct} AS has_ct, \
                {has_whois} AS has_whois, \
                {has_osint} AS has_osint, \
                (EXISTS(SELECT 1 FROM organizations child \
                          WHERE child.parent_id = organizations.id)) AS has_subsidiary \
           FROM organizations WHERE id = $1",
        has_asn = dim(&jsonb_non_empty("asns"), "asns_collected_at"),
        has_ct = dim(
            &jsonb_non_empty("certificates"),
            "certificates_collected_at"
        ),
        has_whois = dim(whois_expr, "whois_collected_at"),
        has_osint = dim(&osint_expr, "osint_collected_at"),
    )
}

/// 通用模板：该 org 下 scope='in' 的 target 中，满足 `extra` 条件的 `value` 集合。
/// `$1 IS NULL` 时不按 org 过滤（退回全局 scope='in'）。`extra` 形如
/// `"AND jsonb_typeof(t.ports) = 'array' AND t.ports <> '[]'::jsonb"` 或
/// `"JOIN fingerprints f ON ..."`。
///
/// Per-asset row-level freshness window (design 2026-06-22 §3.3): when
/// `window = Some(col)`, additionally require `col >= $2` (this stage-run start,
/// `operation_state.stage_started_at`), so a row left by a previous stage-run no
/// longer satisfies the dimension this run. `col` is a fixed literal chosen by the
/// caller (never user input — injection-safe). `None` = presence-only ($1 only;
/// freshness_window gray-switch off, or this dimension not yet windowed). When
/// `Some`, the caller MUST bind `$2` in [`fetch_values`] (gated by the same flag).
fn build_in_scope_values_sql(join: &str, filter: &str, window: Option<&str>) -> String {
    let window_clause = match window {
        Some(col) => format!("AND {col} >= $2"),
        None => String::new(),
    };
    format!(
        "SELECT DISTINCT t.value FROM targets t {join} \
           WHERE t.scope::text = 'in' \
             AND ($1 IS NULL OR t.organization_id = $1) {filter} {window_clause}"
    )
}

/// 该 org 下 scope='in' 的 target 中，哪些 `value` 真有 `asset_type='subdomain'` 子资产行。
/// `apply_window` (Phase B, freshness_window on) ⇒ 只数本次 stage-run 期间发现的子域
/// 子资产行（`target_assets.discovered_at >= $2`）。
fn build_subdomain_target_values_sql(apply_window: bool) -> String {
    build_in_scope_values_sql(
        "JOIN target_assets ta ON ta.target_id = t.id",
        "AND ta.asset_type = 'subdomain'",
        apply_window.then_some("ta.discovered_at"),
    )
}

fn ports_non_empty_sql(alias: &str) -> String {
    format!("jsonb_typeof({alias}.ports) = 'array' AND {alias}.ports <> '[]'::jsonb")
}

fn ports_array_expr(alias: &str) -> String {
    format!(
        "CASE WHEN jsonb_typeof({alias}.ports) = 'array' THEN {alias}.ports ELSE '[]'::jsonb END"
    )
}

fn port_has_technologies_sql(port_alias: &str) -> String {
    format!(
        "({port_alias} ? 'technologies' AND {port_alias}->'technologies' <> 'null'::jsonb
                                 AND {port_alias}->'technologies' <> '[]'::jsonb
                                 AND {port_alias}->'technologies' <> '{{}}'::jsonb)"
    )
}

fn informative_service_sql(port_alias: &str) -> String {
    format!(
        "NULLIF(trim({port_alias}->>'service'), '') IS NOT NULL
            AND lower(split_part(trim({port_alias}->>'service'), ' ', 1))
                NOT IN ('tcpwrapped', 'unknown', 'open', 'filtered', 'closed')
            AND {port_alias}->>'port' <> '53'"
    )
}

fn ports_have_service_hint_sql(alias: &str) -> String {
    let ports = ports_array_expr(alias);
    let technologies = port_has_technologies_sql("p");
    let service = informative_service_sql("p");
    format!(
        "EXISTS (
        SELECT 1
          FROM jsonb_array_elements({ports}) p
         WHERE ({service})
            OR NULLIF(p->>'version', '') IS NOT NULL
            OR NULLIF(p->>'webserver', '') IS NOT NULL
            OR {technologies}
    )"
    )
}

fn fresh_ports_sql(alias: &str, apply_window: bool) -> String {
    let ports = ports_non_empty_sql(alias);
    if apply_window {
        format!("({ports} AND {alias}.ports_scanned_at >= $2)")
    } else {
        format!("({ports})")
    }
}

fn real_ip_fresh_ports_exists_sql(apply_window: bool) -> String {
    format!(
        "EXISTS (
            SELECT 1
              FROM targets ip
             WHERE t.real_ip <> ''
               AND ip.value = t.real_ip
               AND ip.scope::text = 'in'
               AND ($1 IS NULL OR ip.organization_id = $1)
               AND ip.target_type::text IN {IP_TYPE_IN_LIST}
               AND {fresh_ports}
        )",
        fresh_ports = fresh_ports_sql("ip", apply_window)
    )
}

fn fingerprint_exists_sql(alias: &str, apply_window: bool) -> String {
    if apply_window {
        format!(
            "EXISTS (SELECT 1 FROM fingerprints f WHERE f.target_id = {alias}.id AND f.detected_at >= $2)"
        )
    } else {
        format!("EXISTS (SELECT 1 FROM fingerprints f WHERE f.target_id = {alias}.id)")
    }
}

fn service_from_ports_sql(alias: &str, apply_window: bool) -> String {
    format!(
        "({fresh_ports} AND {hints})",
        fresh_ports = fresh_ports_sql(alias, apply_window),
        hints = ports_have_service_hint_sql(alias)
    )
}

fn real_ip_service_exists_sql(apply_window: bool) -> String {
    format!(
        "EXISTS (
            SELECT 1
              FROM targets ip
             WHERE t.real_ip <> ''
               AND ip.value = t.real_ip
               AND ip.scope::text = 'in'
               AND ($1 IS NULL OR ip.organization_id = $1)
               AND ip.target_type::text IN {IP_TYPE_IN_LIST}
               AND ({fp_exists} OR {service_from_ports})
        )",
        fp_exists = fingerprint_exists_sql("ip", apply_window),
        service_from_ports = service_from_ports_sql("ip", apply_window)
    )
}

fn only_dns_port_without_service_surface_sql(alias: &str, apply_window: bool) -> String {
    let ports = ports_array_expr(alias);
    let fresh_ports = fresh_ports_sql(alias, apply_window);
    let fp_exists = fingerprint_exists_sql(alias, apply_window);
    let technologies = port_has_technologies_sql("p");
    format!(
        "({fresh_ports}
            AND NOT {fp_exists}
            AND EXISTS (
                SELECT 1 FROM jsonb_array_elements({ports}) p
                 WHERE p->>'port' = '53'
            )
            AND NOT EXISTS (
                SELECT 1 FROM jsonb_array_elements({ports}) p
                 WHERE COALESCE(NULLIF(p->>'port', ''), '') <> '53'
            )
            AND NOT EXISTS (
                SELECT 1 FROM jsonb_array_elements({ports}) p
                 WHERE NULLIF(p->>'version', '') IS NOT NULL
                    OR NULLIF(p->>'webserver', '') IS NOT NULL
                    OR NULLIF(p->>'product', '') IS NOT NULL
                    OR NULLIF(p->>'banner', '') IS NOT NULL
                    OR {technologies}
            ))"
    )
}

/// EAS-LIVENESS：httpx 探活/解析 IP（`http_status` 非空或 `real_ip` 非空）；
/// 端口扫描得到新鲜端口也证明 host 存活。Phase D：`apply_window` ⇒ 只数本次
/// stage-run 探的活性（`t.liveness_checked_at` / `t.ports_scanned_at >= $2`）。
fn build_liveness_values_sql(apply_window: bool) -> String {
    let filter = if apply_window {
        format!(
            "AND (((t.http_status IS NOT NULL OR t.real_ip <> '') AND t.liveness_checked_at >= $2) \
              OR {fresh_ports} OR {real_ip_ports})",
            fresh_ports = fresh_ports_sql("t", true),
            real_ip_ports = real_ip_fresh_ports_exists_sql(true)
        )
    } else {
        format!(
            "AND (t.http_status IS NOT NULL OR t.real_ip <> '' OR {fresh_ports} OR {real_ip_ports})",
            fresh_ports = fresh_ports_sql("t", false),
            real_ip_ports = real_ip_fresh_ports_exists_sql(false)
        )
    };
    build_in_scope_values_sql("", &filter, None)
}

/// Enumeration IP-web denominator: in-scope IP/CIDR assets that EAS/httpx has
/// proven to be HTTP services (`targets.http_status IS NOT NULL`). This is more
/// specific than EAS-LIVENESS, which may also be satisfied by ping/port evidence.
fn build_web_capable_ip_values_sql() -> String {
    build_in_scope_values_sql(
        "",
        &format!("AND t.http_status IS NOT NULL AND t.target_type::text IN {IP_TYPE_IN_LIST}"),
        None,
    )
}

/// EAS-PORT：端口扫描结果（`ports` 为非空 JSONB 数组）。判空走 `jsonb_typeof =
/// 'array'` + 比较式（不裸调 `jsonb_array_length`，否则非数组 `ports` 会抛
/// `cannot get array length of a non-array`），与 `engagement_truth` 同款守卫。
fn build_port_values_sql(apply_window: bool) -> String {
    let filter = format!(
        "AND ({fresh_ports} OR {real_ip_ports})",
        fresh_ports = fresh_ports_sql("t", apply_window),
        real_ip_ports = real_ip_fresh_ports_exists_sql(apply_window)
    );
    build_in_scope_values_sql("", &filter, None)
}

/// EAS-SERVICE-FINGERPRINT：该 host 有服务/版本指纹行，或端口扫描结果里已经
/// 带 service/version/webserver/technology hint。Phase D 行级窗：`apply_window`
/// ⇒ 只数本次 stage-run 探到的指纹/端口服务（`f.detected_at` /
/// `t.ports_scanned_at >= $2`）。
fn build_service_fp_values_sql(apply_window: bool) -> String {
    let fp_exists = fingerprint_exists_sql("t", apply_window);
    let ports_clause = service_from_ports_sql("t", apply_window);
    let real_ip_service = real_ip_service_exists_sql(apply_window);
    build_in_scope_values_sql(
        "",
        &format!("AND ({fp_exists} OR {ports_clause} OR {real_ip_service})"),
        None,
    )
}

fn build_eas_service_not_applicable_values_sql(apply_window: bool) -> String {
    let dns_only_no_service = only_dns_port_without_service_surface_sql("t", apply_window);
    build_in_scope_values_sql(
        "",
        &format!("AND t.target_type::text IN {IP_TYPE_IN_LIST} AND {dns_only_no_service}"),
        None,
    )
}

/// ENUM-JS：该 host 已收集到 JS 资产（browser_collect_js_api → js_analysis_results）。
/// Phase D 行级窗：`apply_window` ⇒ 只数本次 stage-run 落库（`jar.analyzed_at >= $2`）。
fn build_js_values_sql(apply_window: bool) -> String {
    build_in_scope_values_sql(
        "JOIN js_analysis_results jar ON jar.target_id = t.id",
        "",
        apply_window.then_some("jar.analyzed_at"),
    )
}

/// ENUM-DIR：该 host 有目录枚举产物（ffuf/gobuster → directory_entries）。Phase D
/// 行级窗：`apply_window` ⇒ 只数本次 stage-run 落库的条目（`de.created_at >= $2`）。
fn build_dir_values_sql(apply_window: bool) -> String {
    build_in_scope_values_sql(
        "JOIN directory_entries de ON de.target_id = t.id",
        "",
        apply_window.then_some("de.created_at"),
    )
}

/// ENUM-PARAM：该 host 有带参端点（arjun/katana → api_endpoints.params 非空）。
/// 判空同 [`build_port_values_sql`]：`jsonb_typeof = 'array'` + 比较式，避免对
/// 非数组 `params` 调 `jsonb_array_length` 抛错。
fn build_param_values_sql(apply_window: bool) -> String {
    build_in_scope_values_sql(
        "JOIN api_endpoints ae ON ae.target_id = t.id",
        "AND jsonb_typeof(ae.params) = 'array' AND ae.params <> '[]'::jsonb",
        apply_window.then_some("ae.discovered_at"),
    )
}

/// ENUM-JSAPI：该 host 有 JS/爬虫抽取的端点（api_endpoints.source）。
fn build_jsapi_values_sql(apply_window: bool) -> String {
    build_in_scope_values_sql(
        "JOIN api_endpoints ae ON ae.target_id = t.id",
        "AND ae.source IN ('js_analysis', 'crawler')",
        apply_window.then_some("ae.discovered_at"),
    )
}

/// RDNS (host-aware 2c-3): in-scope IP/CIDR targets that have a 'PTR' dns_records
/// row (reverse DNS landed). Reuses `dns_records` (no schema change) + IP-type filter.
fn build_rdns_values_sql() -> String {
    build_in_scope_values_sql(
        "JOIN dns_records dr ON dr.target_id = t.id",
        &format!("AND dr.record_type = 'PTR' AND t.target_type::text IN {IP_TYPE_IN_LIST}"),
        None,
    )
}

/// IP-WHOIS (host-aware 2c-3): in-scope IP/CIDR targets with non-empty
/// `targets.ip_whois` (RIR RDAP). Shape-agnostic empty check (no `jsonb_array_length`).
/// Phase D：`apply_window` ⇒ 只数本次 stage-run 采的 IP-WHOIS（`t.ip_whois_collected_at >= $2`）。
fn build_ipwhois_values_sql(apply_window: bool) -> String {
    build_in_scope_values_sql(
        "",
        &format!(
            "AND {} AND t.target_type::text IN {IP_TYPE_IN_LIST}",
            jsonb_non_empty("t.ip_whois")
        ),
        apply_window.then_some("t.ip_whois_collected_at"),
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
    /// Host-aware 2c-3 (per-asset, IP-only via SQL): reverse-DNS (PTR) + RIR IP-WHOIS.
    pub rdns_values: &'a HashSet<String>,
    pub ipwhois_values: &'a HashSet<String>,
    pub liveness_values: &'a HashSet<String>,
    pub port_values: &'a HashSet<String>,
    pub service_fp_values: &'a HashSet<String>,
    /// ENUM-JS（design 2026-07-01 §4.1）：该 host 已收集 JS 资产（js_analysis_results 有行）。
    pub js_values: &'a HashSet<String>,
    pub dir_values: &'a HashSet<String>,
    pub param_values: &'a HashSet<String>,
    pub jsapi_values: &'a HashSet<String>,
}

/// 纯组装（与 IO 解耦，便于单测）：对每个 in-scope asset，按业务表存量产 `(asset, technique)`。
/// 顺序确定（每 asset 内固定 12 维顺序，外层按 `in_scope_assets` 顺序），便于断言。
///
/// 2c-2 (设计 2026-06-15-host-aware-coverage-2c §4.3): type-aware projection.
/// `types[i]` is `in_scope_assets[i]` 的 `targets.type`；域名专属 org 事实（CT）
/// **不**盖到 IP/CIDR 资产上（cert transparency 对裸 IP 无意义）。`types` 为空（或某
/// 索引缺失/未知类型）⇒ 当作非 IP（保留全部事实——fail-safe 倾向多报、绝不少报，不放松 gate）。
pub(crate) fn assemble_truth_facts_typed(
    in_scope_assets: &[String],
    types: &[String],
    inputs: &TruthInputs<'_>,
) -> Vec<(String, &'static str)> {
    let mut facts = Vec::new();
    for (i, asset) in in_scope_assets.iter().enumerate() {
        // 2c-2: 该资产是否 IP/CIDR（按权威 type）；缺/未知 ⇒ 非 IP（保留全部，fail-safe）。
        let ip_like = matches!(
            types.get(i).map(String::as_str),
            Some("ip" | "ipv4" | "ipv6" | "ip_address" | "cidr" | "range" | "netblock")
        );
        // org 级存量：命中即对每个 in-scope 资产产同一 technique。
        if inputs.has_asn {
            facts.push((asset.clone(), TECH_ASN));
        }
        // CT 是域名专属（cert transparency 对裸 IP 无意义）→ 不盖 IP/CIDR。
        if inputs.has_ct && !ip_like {
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
        // Host-aware 2c-3: per-asset IP-native (the SQL already restricts to
        // IP/CIDR assets, so pushing is harmless on a domain that has none).
        if inputs.rdns_values.contains(asset) {
            facts.push((asset.clone(), TECH_RDNS));
        }
        if inputs.ipwhois_values.contains(asset) {
            facts.push((asset.clone(), TECH_IPWHOIS));
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
        if inputs.js_values.contains(asset) {
            facts.push((asset.clone(), TECH_ENUM_JS));
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
async fn fetch_values(
    pool: &PgPool,
    sql: &str,
    org_id: Option<Uuid>,
    run_start: Option<DateTime<Utc>>,
) -> Result<HashSet<String>> {
    // `$1` = org_id always; `$2` = run_start bound only when the SQL was built
    // with a row-level window (caller passes the same `run_start` that drove
    // `apply_window`), keeping placeholder count and bind count in lockstep.
    let mut query = sqlx::query_scalar::<_, String>(sql).bind(org_id);
    if let Some(rs) = run_start {
        query = query.bind(rs);
    }
    Ok(query.fetch_all(pool).await?.into_iter().collect())
}

/// DB 业务表真值事实 `(asset, technique)`：业务表里 `asset` 上 `technique` 真有数据。
///
/// `in_scope_assets` 是 coverage gate 实际遍历的权威资产集（org 已隔离），保证与
/// `coverage_complete` 的 asset 维度对齐。`org_id=None` 时不查 org 级情报（ASN/CT/
/// WHOIS/OSINT 不投影），per-asset 维度退回全局 scope='in'。空 in-scope → 直接返回空。
///
/// 2c-2: `types[i]` = `in_scope_assets[i]` 的 `targets.type`；CT 等域名专属 org 事实
/// 不投影到 IP/CIDR（缺/未知类型 ⇒ 当作非 IP，保留全部，fail-safe）。
pub async fn coverage_truth_facts(
    pool: &PgPool,
    org_id: Option<Uuid>,
    in_scope_assets: &[String],
    types: &[String],
    run_start: Option<DateTime<Utc>>,
) -> Result<Vec<(String, &'static str)>> {
    if in_scope_assets.is_empty() {
        return Ok(Vec::new());
    }
    // `run_start = Some(stage-run start)` applies the per-dimension freshness
    // window to the 4 org intel dims (design 2026-06-22 §3.2); `None` keeps the
    // presence-only behavior (freshness_window gray-switch off). Per-asset dims
    // (DNS/SUBDOMAIN/EAS/ENUM) are unaffected here — they get their own row-level
    // window in Phase B/D.
    let (has_asn, has_ct, has_whois, has_osint, has_subsidiary) = match org_id {
        Some(id) => {
            let sql = build_org_intel_presence_sql(run_start.is_some());
            let mut query = sqlx::query_as::<_, (bool, bool, bool, bool, bool)>(&sql).bind(id);
            if let Some(rs) = run_start {
                query = query.bind(rs);
            }
            query
                .fetch_optional(pool)
                .await?
                .unwrap_or((false, false, false, false, false))
        }
        None => (false, false, false, false, false),
    };
    // Per-dimension freshness window (design 2026-06-22): when `run_start = Some`
    // (freshness_window on), each per-asset dim only counts rows collected this
    // stage-run. `aw` (= `run_start.is_some()`) keeps the SQL placeholder ($2) and
    // the `fetch_values` bind in lockstep. Phase B = SUBDOMAIN + DNS; Phase D =
    // EAS/ENUM (LIVENESS/PORT/SERVICE-FP/DIR/PARAM/JSAPI) + IP-WHOIS. RDNS stays
    // presence-only (out of the 2026-06-22 scope; niche IP-only PTR dim).
    let aw = run_start.is_some();
    let subdomain_values = fetch_values(
        pool,
        &build_subdomain_target_values_sql(aw),
        org_id,
        run_start,
    )
    .await?;
    // DNS 维度（PR-B）：复用 dns_records repo 的存在查询（DRY），org 隔离；Phase B
    // 行级窗 `dns_records.created_at >= run_start`（受 freshness_window 控）。
    let dns_values: HashSet<String> =
        crate::repo::dns_records::present_target_values(pool, org_id, run_start)
            .await?
            .into_iter()
            .collect();
    let rdns_values = fetch_values(pool, &build_rdns_values_sql(), org_id, None).await?;
    let ipwhois_values =
        fetch_values(pool, &build_ipwhois_values_sql(aw), org_id, run_start).await?;
    let liveness_values =
        fetch_values(pool, &build_liveness_values_sql(aw), org_id, run_start).await?;
    let port_values = fetch_values(pool, &build_port_values_sql(aw), org_id, run_start).await?;
    let service_fp_values =
        fetch_values(pool, &build_service_fp_values_sql(aw), org_id, run_start).await?;
    let js_values = fetch_values(pool, &build_js_values_sql(aw), org_id, run_start).await?;
    let dir_values = fetch_values(pool, &build_dir_values_sql(aw), org_id, run_start).await?;
    let param_values = fetch_values(pool, &build_param_values_sql(aw), org_id, run_start).await?;
    let jsapi_values = fetch_values(pool, &build_jsapi_values_sql(aw), org_id, run_start).await?;
    Ok(assemble_truth_facts_typed(
        in_scope_assets,
        types,
        &TruthInputs {
            has_asn,
            has_ct,
            has_whois,
            has_osint,
            has_subsidiary,
            subdomain_values: &subdomain_values,
            dns_values: &dns_values,
            rdns_values: &rdns_values,
            ipwhois_values: &ipwhois_values,
            liveness_values: &liveness_values,
            port_values: &port_values,
            service_fp_values: &service_fp_values,
            js_values: &js_values,
            dir_values: &dir_values,
            param_values: &param_values,
            jsapi_values: &jsapi_values,
        },
    ))
}

/// In-scope IP/CIDR target values that are content-enumeration capable because
/// EAS/httpx observed an HTTP response (`targets.http_status` is non-null).
pub async fn web_capable_ip_assets(pool: &PgPool, org_id: Option<Uuid>) -> Result<HashSet<String>> {
    fetch_values(pool, &build_web_capable_ip_values_sql(), org_id, None).await
}

/// EAS IP/CIDR assets whose SERVICE-FINGERPRINT technique is deterministically
/// not applicable: the only open port observed in this wave is DNS/53, and no
/// strong service/version surface (fingerprint row, version, product, banner,
/// webserver, technologies) exists. This keeps shared DNS/CDN real_ip rows from
/// wedging the SERVICE gate after pseudo-services such as tcpwrapped are excluded.
pub async fn eas_service_not_applicable_assets(
    pool: &PgPool,
    org_id: Option<Uuid>,
    run_start: Option<DateTime<Utc>>,
) -> Result<HashSet<String>> {
    let aw = run_start.is_some();
    fetch_values(
        pool,
        &build_eas_service_not_applicable_values_sql(aw),
        org_id,
        run_start,
    )
    .await
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
            rdns_values: empty,
            ipwhois_values: empty,
            liveness_values: empty,
            port_values: empty,
            service_fp_values: empty,
            js_values: empty,
            dir_values: empty,
            param_values: empty,
            jsapi_values: empty,
        }
    }

    #[test]
    fn org_intel_presence_sql_includes_subsidiary_child_exists() {
        let sql = build_org_intel_presence_sql(false);
        assert!(sql.contains("child.parent_id = organizations.id"));
        assert!(sql.contains("AS has_subsidiary"));
    }

    #[test]
    fn assemble_projects_subsidiary_to_every_in_scope_asset() {
        let empty = subs(&[]);
        let mut inputs = empty_inputs(&empty);
        inputs.has_subsidiary = true;
        let assets = vec!["moresec.cn".to_string(), "moresec.com".to_string()];
        let facts = assemble_truth_facts_typed(&assets, &[], &inputs);
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
        let facts = assemble_truth_facts_typed(&assets, &[], &inputs);
        assert!(!facts.iter().any(|(_, t)| *t == TECH_SUBSIDIARY));
    }

    #[test]
    fn assemble_skips_ct_for_ip_assets() {
        // 2c-2: CT 是域名专属 org 事实——域名拿到 CT，IP 不拿；ASN 两者都拿（org 级）。
        let empty = subs(&[]);
        let mut inputs = empty_inputs(&empty);
        inputs.has_ct = true;
        inputs.has_asn = true;
        let assets = vec!["a.com".to_string(), "1.2.3.4".to_string()];
        let types = vec!["domain".to_string(), "ip".to_string()];
        let facts = assemble_truth_facts_typed(&assets, &types, &inputs);
        assert!(facts.contains(&("a.com".to_string(), TECH_CT)));
        assert!(!facts.contains(&("1.2.3.4".to_string(), TECH_CT)));
        assert!(facts.contains(&("1.2.3.4".to_string(), TECH_ASN)));
        // 缺类型 ⇒ 当作非 IP，保留 CT（fail-safe，绝不少报）。
        let facts_untyped = assemble_truth_facts_typed(&assets, &[], &inputs);
        assert!(facts_untyped.contains(&("1.2.3.4".to_string(), TECH_CT)));
    }

    #[test]
    fn rdns_values_sql_filters_ptr_and_ip_types() {
        let sql = build_rdns_values_sql();
        assert!(sql.contains("JOIN dns_records dr ON dr.target_id = t.id"));
        assert!(sql.contains("dr.record_type = 'PTR'"));
        assert!(sql.contains("t.target_type::text IN ('ip', 'ipv4'"));
        assert!(sql.contains("t.scope::text = 'in'"));
        assert!(sql.contains("($1 IS NULL OR t.organization_id = $1)"));
    }

    #[test]
    fn ipwhois_values_sql_filters_nonempty_and_ip_types() {
        let sql = build_ipwhois_values_sql(false);
        assert!(sql.contains("t.ip_whois IS NOT NULL"));
        assert!(sql.contains("t.ip_whois <> '{}'::jsonb"));
        assert!(sql.contains("t.target_type::text IN ('ip', 'ipv4'"));
        assert!(!sql.contains("jsonb_array_length"));
    }

    #[test]
    fn assemble_projects_rdns_and_ipwhois_per_asset() {
        let empty = subs(&[]);
        let rdns = subs(&["1.2.3.4"]);
        let ipw = subs(&["1.2.3.4"]);
        let mut inputs = empty_inputs(&empty);
        inputs.rdns_values = &rdns;
        inputs.ipwhois_values = &ipw;
        let assets = vec!["a.com".to_string(), "1.2.3.4".to_string()];
        let types = vec!["domain".to_string(), "ip".to_string()];
        let facts = assemble_truth_facts_typed(&assets, &types, &inputs);
        assert!(facts.contains(&("1.2.3.4".to_string(), TECH_RDNS)));
        assert!(facts.contains(&("1.2.3.4".to_string(), TECH_IPWHOIS)));
        // domain has no rdns/ipwhois truth → not projected.
        assert!(!facts
            .iter()
            .any(|(a, t)| a == "a.com" && (*t == TECH_RDNS || *t == TECH_IPWHOIS)));
    }

    #[test]
    fn org_intel_presence_sql_reads_all_four_org_columns() {
        let sql = build_org_intel_presence_sql(false);
        // 五个 org 级列都参与判定（列名出现）。
        for col in [
            "asns",
            "certificates",
            "contacts",
            "social_accounts",
            "business_systems",
        ] {
            assert!(sql.contains(col), "missing column {col}: {sql}");
        }
        assert!(sql.contains("whois IS NOT NULL"));
        // OSINT 多源任一非空（intel->records 走 jsonb_typeof 守卫的 array_length）。
        assert!(sql.contains("intel->'records'"));
        assert!(sql.contains("FROM organizations WHERE id = $1"));
    }

    #[test]
    fn org_intel_presence_sql_off_omits_freshness_window() {
        // freshness_window gray-switch OFF (design 2026-06-22): presence-only, no
        // `*_collected_at` predicate and no $2 placeholder ⇒ pre-change behavior.
        let sql = build_org_intel_presence_sql(false);
        assert!(!sql.contains("collected_at"), "off must not window: {sql}");
        assert!(!sql.contains("$2"), "off must bind only $1: {sql}");
        assert!(!sql.contains("IS TRUE"));
    }

    #[test]
    fn org_intel_presence_sql_on_windows_each_org_dim() {
        // freshness_window ON: every org intel dim requires its per-dimension
        // `<dim>_collected_at >= $2`, wrapped `IS TRUE` so a NULL collected_at
        // becomes `false` (never a stale Found; stays a non-NULL bool).
        let sql = build_org_intel_presence_sql(true);
        for col in [
            "asns_collected_at",
            "certificates_collected_at",
            "whois_collected_at",
            "osint_collected_at",
        ] {
            assert!(
                sql.contains(&format!("{col} >= $2")),
                "missing window for {col}: {sql}"
            );
        }
        assert!(sql.contains("IS TRUE"), "window must coalesce NULL: {sql}");
        // subsidiary is scoping-only (no collected_at) — never windowed.
        assert!(!sql.contains("has_subsidiary >= $2"));
        // org-column reads + shape-agnostic empty checks survive (windowing is
        // additive, not a rewrite).
        assert!(sql.contains("asns <> '[]'::jsonb"));
        assert!(sql.contains("intel->'records'"));
    }

    /// shape 无关空判据自身的守卫：永不调 `jsonb_array_length`，且把
    /// NULL/`'null'`/`'[]'`/`'{}'` 全判空（数组/对象/标量列共用）。
    #[test]
    fn jsonb_non_empty_avoids_array_length_and_guards_all_empty_shapes() {
        let pred = jsonb_non_empty("contacts");
        assert!(
            !pred.contains("jsonb_array_length"),
            "must not call jsonb_array_length: {pred}"
        );
        assert!(pred.contains("contacts IS NOT NULL"));
        assert!(pred.contains("contacts <> 'null'::jsonb"));
        assert!(pred.contains("contacts <> '[]'::jsonb"));
        assert!(pred.contains("contacts <> '{}'::jsonb"));
    }

    /// 回归（既有 bug `cannot get array length of a non-array`）：org-level
    /// presence 查询绝不能对可能为非数组的列裸调 `jsonb_array_length`。`contacts`
    /// 经 recon 写成对象 `{email:[...]}`，裸调会让整条查询抛错 → db_truth 投影被丢
    /// → scoping 子公司 gate 误判 not_attempted → BLOCK。唯一允许的 array_length 是
    /// `intel->'records'`（带 `jsonb_typeof = 'array'` 守卫，对非数组安全）。
    #[test]
    fn org_intel_presence_sql_never_array_length_on_unguarded_columns() {
        let sql = build_org_intel_presence_sql(false);
        for col in [
            "asns",
            "certificates",
            "contacts",
            "social_accounts",
            "business_systems",
        ] {
            let needle = format!("jsonb_array_length({col})");
            assert!(!sql.contains(&needle), "unguarded {needle}: {sql}");
            // 每列都用 shape 无关比较式判空（对象/数组/NULL 均安全）。
            assert!(
                sql.contains(&format!("{col} <> '[]'::jsonb"))
                    && sql.contains(&format!("{col} <> '{{}}'::jsonb")),
                "missing shape-agnostic empty check for {col}: {sql}"
            );
        }
        // 唯一允许的 array_length 必须带 jsonb_typeof 守卫。
        assert!(sql.contains("jsonb_typeof(intel->'records') = 'array'"));
    }

    #[test]
    fn subdomain_sql_filters_scope_org_and_asset_type() {
        let sql = build_subdomain_target_values_sql(false);
        assert!(sql.contains("t.scope::text = 'in'"));
        assert!(sql.contains("($1 IS NULL OR t.organization_id = $1)"));
        assert!(sql.contains("ta.asset_type = 'subdomain'"));
        assert!(sql.contains("JOIN target_assets ta ON ta.target_id = t.id"));
    }

    #[test]
    fn subdomain_sql_off_omits_row_level_window() {
        // freshness_window OFF (Phase B): presence-only, no `$2` / `discovered_at >=`
        // ⇒ a subdomain child landed by a previous stage-run still counts (pre-change).
        let sql = build_subdomain_target_values_sql(false);
        assert!(!sql.contains("$2"), "off must bind only $1: {sql}");
        assert!(
            !sql.contains("discovered_at >="),
            "off must not window: {sql}"
        );
    }

    #[test]
    fn subdomain_sql_on_windows_target_assets_discovered_at() {
        // freshness_window ON (Phase B, design 2026-06-22 §3.3): the SUBDOMAIN
        // dimension only counts in-scope targets whose subdomain child rows were
        // discovered this stage-run (`target_assets.discovered_at >= $2`).
        let sql = build_subdomain_target_values_sql(true);
        assert!(
            sql.contains("ta.discovered_at >= $2"),
            "on must window target_assets.discovered_at: {sql}"
        );
        // windowing is additive — scope/org/asset_type predicates survive.
        assert!(sql.contains("ta.asset_type = 'subdomain'"));
        assert!(sql.contains("t.scope::text = 'in'"));
    }

    #[test]
    fn active_dimension_sqls_filter_scope_and_org() {
        // 6 个主动维度 SQL 都必须带 scope='in' + org 隔离（不串 org / 不漏 scope）。
        for sql in [
            build_liveness_values_sql(false),
            build_port_values_sql(false),
            build_service_fp_values_sql(false),
            build_dir_values_sql(false),
            build_param_values_sql(false),
            build_jsapi_values_sql(false),
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
        let live = build_liveness_values_sql(false);
        assert!(live.contains("t.http_status IS NOT NULL OR t.real_ip"));
        assert!(live.contains("jsonb_typeof(t.ports) = 'array' AND t.ports <> '[]'::jsonb"));
        assert!(build_port_values_sql(false)
            .contains("jsonb_typeof(t.ports) = 'array' AND t.ports <> '[]'::jsonb"));
        let service = build_service_fp_values_sql(false);
        assert!(service.contains("EXISTS (SELECT 1 FROM fingerprints f WHERE f.target_id = t.id)"));
        assert!(service.contains("jsonb_array_elements("));
        assert!(service.contains("p->>'service'"));
        assert!(service.contains("NOT IN ('tcpwrapped', 'unknown', 'open', 'filtered', 'closed')"));
        assert!(service.contains("p->>'port' <> '53'"));
        assert!(service.contains("p->>'version'"));
        assert!(build_dir_values_sql(false)
            .contains("JOIN directory_entries de ON de.target_id = t.id"));
        let param = build_param_values_sql(false);
        assert!(param.contains("JOIN api_endpoints ae ON ae.target_id = t.id"));
        assert!(param.contains("jsonb_typeof(ae.params) = 'array' AND ae.params <> '[]'::jsonb"));
        let jsapi = build_jsapi_values_sql(false);
        assert!(jsapi.contains("JOIN api_endpoints ae ON ae.target_id = t.id"));
        assert!(jsapi.contains("ae.source IN ('js_analysis', 'crawler')"));
    }

    #[test]
    fn js_values_sql_joins_js_analysis_results_and_scoping() {
        // ENUM-JS 真值（design 2026-07-01 §4.1）：join js_analysis_results + org/scope 隔离。
        let sql = build_js_values_sql(false);
        assert!(sql.contains("JOIN js_analysis_results jar ON jar.target_id = t.id"));
        assert!(sql.contains("t.scope::text = 'in'"));
        assert!(sql.contains("($1 IS NULL OR t.organization_id = $1)"));
        assert!(!sql.contains("$2"), "off must bind only $1: {sql}");
        // freshness window on ⇒ 只数本次 stage-run 落库（jar.analyzed_at >= $2）。
        assert!(build_js_values_sql(true).contains("jar.analyzed_at >= $2"));
    }

    #[test]
    fn web_capable_ip_values_sql_uses_http_status_and_ip_types() {
        let sql = build_web_capable_ip_values_sql();
        assert!(sql.contains("t.http_status IS NOT NULL"));
        assert!(sql.contains("t.target_type::text IN"));
        assert!(sql.contains("'ip'"));
        assert!(sql.contains("'cidr'"));
        assert!(sql.contains("t.scope::text = 'in'"));
        assert!(sql.contains("($1 IS NULL OR t.organization_id = $1)"));
        assert!(
            !sql.contains("$2"),
            "web-capable IP lookup is presence-only: {sql}"
        );
    }

    #[test]
    fn eas_service_not_applicable_sql_is_dns_only_and_ip_scoped() {
        let sql = build_eas_service_not_applicable_values_sql(false);
        assert!(sql.contains("t.target_type::text IN"));
        assert!(sql.contains("p->>'port' = '53'"));
        assert!(sql.contains("<> '53'"));
        assert!(sql.contains("NOT EXISTS (SELECT 1 FROM fingerprints"));
        assert!(sql.contains("p->>'version'"));
        assert!(sql.contains("p->>'product'"));
        assert!(!sql.contains("$2"), "presence-only mode must not bind cutoff: {sql}");

        let fresh = build_eas_service_not_applicable_values_sql(true);
        assert!(fresh.contains("t.ports_scanned_at >= $2"));
        assert!(fresh.contains("f.detected_at >= $2"));
    }

    #[test]
    fn assemble_projects_js_per_asset() {
        // 只有真收集到 JS 的 host 才产 GOLISH-ENUM-JS fact（per-asset）。
        let empty = subs(&[]);
        let js = subs(&["a.com"]);
        let mut inputs = empty_inputs(&empty);
        inputs.js_values = &js;
        let assets = vec!["a.com".to_string(), "b.com".to_string()];
        let facts = assemble_truth_facts_typed(&assets, &[], &inputs);
        assert!(facts.contains(&("a.com".to_string(), TECH_ENUM_JS)));
        assert!(!facts
            .iter()
            .any(|(a, t)| a == "b.com" && *t == TECH_ENUM_JS));
    }

    /// 回归：per-asset 维度 SQL 也不能对可能为非数组的 `ports`/`params` 裸调
    /// `jsonb_array_length`（同 org-level presence 的 crash 类），否则会让整条
    /// `coverage_truth_facts` 抛错、db_truth 投影整体失效。
    #[test]
    fn active_dimension_sqls_never_array_length_unguarded() {
        assert!(!build_port_values_sql(false).contains("jsonb_array_length(t.ports)"));
        assert!(!build_service_fp_values_sql(false).contains("jsonb_array_elements(t.ports)"));
        assert!(!build_param_values_sql(false).contains("jsonb_array_length(ae.params)"));
    }

    #[test]
    fn active_dimension_sqls_off_omit_row_level_window() {
        // freshness_window OFF (Phase D): presence-only — no `$2` window predicate
        // ⇒ EAS/ENUM data left by a previous stage-run still counts (pre-change).
        for sql in [
            build_liveness_values_sql(false),
            build_port_values_sql(false),
            build_service_fp_values_sql(false),
            build_dir_values_sql(false),
            build_param_values_sql(false),
            build_jsapi_values_sql(false),
            build_ipwhois_values_sql(false),
        ] {
            assert!(!sql.contains("$2"), "off must bind only $1: {sql}");
            assert!(!sql.contains(">= $2"), "off must not window: {sql}");
        }
    }

    #[test]
    fn active_dimension_sqls_on_window_their_collection_timestamp() {
        // freshness_window ON (Phase D, design 2026-06-22 §3.3/D3): each EAS/ENUM dim
        // only counts rows collected this stage-run, via its per-dim timestamp >= $2.
        // PORT/LIVENESS/IPWHOIS = targets columns (migration 20260623000001);
        // SERVICE-FP/DIR/PARAM/JSAPI = existing child-table row timestamps.
        let live = build_liveness_values_sql(true);
        assert!(live.contains("t.liveness_checked_at >= $2"));
        assert!(live.contains("t.ports_scanned_at >= $2"));
        assert!(build_port_values_sql(true).contains("t.ports_scanned_at >= $2"));
        assert!(build_ipwhois_values_sql(true).contains("t.ip_whois_collected_at >= $2"));
        let service = build_service_fp_values_sql(true);
        assert!(service.contains("f.detected_at >= $2"));
        assert!(service.contains("t.ports_scanned_at >= $2"));
        assert!(build_dir_values_sql(true).contains("de.created_at >= $2"));
        assert!(build_param_values_sql(true).contains("ae.discovered_at >= $2"));
        assert!(build_jsapi_values_sql(true).contains("ae.discovered_at >= $2"));
    }

    #[test]
    fn assemble_empty_in_scope_yields_no_facts() {
        let empty = HashSet::new();
        let out = assemble_truth_facts_typed(
            &[],
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
        let out = assemble_truth_facts_typed(
            &assets,
            &[],
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
        let out = assemble_truth_facts_typed(
            &assets,
            &[],
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
        let out = assemble_truth_facts_typed(
            &assets,
            &[],
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
        let out = assemble_truth_facts_typed(
            &assets,
            &[],
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
        let js = subs(&["a.com"]);
        let out = assemble_truth_facts_typed(
            &assets,
            &[],
            &TruthInputs {
                has_asn: false,
                has_ct: false,
                has_whois: false,
                has_osint: false,
                has_subsidiary: false,
                subdomain_values: &HashSet::new(),
                dns_values: &HashSet::new(),
                rdns_values: &HashSet::new(),
                ipwhois_values: &HashSet::new(),
                liveness_values: &liveness,
                port_values: &port,
                service_fp_values: &service_fp,
                js_values: &js,
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
                ("a.com".to_string(), TECH_ENUM_JS),
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
        let out = assemble_truth_facts_typed(
            &["a.com".to_string()],
            &[],
            &TruthInputs {
                has_asn: true,
                has_ct: true,
                has_whois: true,
                has_osint: true,
                has_subsidiary: true,
                subdomain_values: &one,
                dns_values: &one,
                rdns_values: &one,
                ipwhois_values: &one,
                liveness_values: &one,
                port_values: &one,
                service_fp_values: &one,
                js_values: &one,
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
                ("a.com".to_string(), TECH_RDNS),
                ("a.com".to_string(), TECH_IPWHOIS),
                ("a.com".to_string(), TECH_EAS_LIVENESS),
                ("a.com".to_string(), TECH_EAS_PORT),
                ("a.com".to_string(), TECH_EAS_SERVICE_FP),
                ("a.com".to_string(), TECH_ENUM_JS),
                ("a.com".to_string(), TECH_ENUM_DIR),
                ("a.com".to_string(), TECH_ENUM_PARAM),
                ("a.com".to_string(), TECH_ENUM_JSAPI),
            ]
        );
    }
}
