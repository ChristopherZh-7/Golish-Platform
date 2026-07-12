//! Coverage gate 的 DB 业务表真值查询（设计 2026-06-12 §5.3 + Phase 1 §5）。
//!
//! 只读地回答「某 org / in-scope 资产，在业务表里某类技术是否真有数据」，供
//! harness 外层 hook 转成 `Found` EvidenceFact 注入 coverage gate，使 coverage
//! 判定以 DB 真值为准（而非 agent 自报 / 命令派生）。
//!
//! 覆盖技术（Phase 0 被动 4 类 + Phase 1 被动 2 类 + 主动 7 类 = 13 维）：
//! - 被动情报（target_intel）：ASN / CT / WHOIS（org 级专列）、OSINT（org 级
//!   intel/contacts/social/business 任一非空）、SUBDOMAIN / DNS（per-asset）。
//! - 主动攻击面（external_attack_surface）：LIVENESS / PORT / SERVICE-FINGERPRINT / WEB-FINGERPRINT。
//! - 内容枚举（enumeration）：DIR / PARAM / JSAPI。
//!
//! 红线（设计 §4）：
//! - 只产「有数据」(Found 语义)；DB 无数据**绝不**推断 checked_empty (I8)。
//! - 只读 SELECT，不写库；gate 纯函数不变（查询在 golish-db，结果经 hook 注入）。
//! - org 维度过滤（`organization_id`）= coverage 资产盘按 organization 隔离
//!   （design 2026-06-09），避免跨 org 业务数据互相投影。

use std::collections::{BTreeMap, BTreeSet, HashSet};

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
pub const TECH_EAS_WEB_FP: &str = "GOLISH-EAS-WEB-FINGERPRINT";

/// Confirmed open TCP-ish ports currently stored on an EAS asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmedOpenServicePorts {
    pub asset: String,
    pub ports: Vec<u16>,
}

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
/// `apply_window` (Phase B, freshness_window on) ⇒ 只数本次 stage-run 期间首次发现
/// 或重新观察的子域资产行。
fn build_subdomain_target_values_sql(apply_window: bool) -> String {
    build_in_scope_values_sql(
        "JOIN target_assets ta ON ta.target_id = t.id",
        "AND ta.project_path IS NOT DISTINCT FROM t.project_path
         AND ta.asset_type = 'subdomain'
         AND EXISTS (
           SELECT 1
           FROM targets child
           WHERE child.organization_id = t.organization_id
             AND child.project_path IS NOT DISTINCT FROM t.project_path
             AND child.scope::text = 'in'
             AND child.target_type::text = 'domain'
             AND lower(trim(trailing '.' FROM child.value)) =
                 lower(trim(trailing '.' FROM ta.value))
         )",
        apply_window.then_some("GREATEST(ta.discovered_at, ta.updated_at)"),
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

fn open_port_sql(port_alias: &str) -> String {
    format!(
        "NULLIF({port_alias}->>'port', '') IS NOT NULL
            AND lower(COALESCE(NULLIF({port_alias}->>'state', ''), 'open')) = 'open'"
    )
}

fn service_fingerprint_required_port_sql(port_alias: &str) -> String {
    let open_port = open_port_sql(port_alias);
    format!("({open_port} AND COALESCE(NULLIF({port_alias}->>'port', ''), '') <> '53')")
}

fn port_has_service_surface_sql(port_alias: &str) -> String {
    let technologies = port_has_technologies_sql(port_alias);
    let service = informative_service_sql(port_alias);
    format!(
        "(({service})
            OR NULLIF({port_alias}->>'version', '') IS NOT NULL
            OR NULLIF({port_alias}->>'product', '') IS NOT NULL
            OR NULLIF({port_alias}->>'service_product', '') IS NOT NULL
            OR NULLIF({port_alias}->>'service_version', '') IS NOT NULL
            OR NULLIF({port_alias}->>'banner', '') IS NOT NULL
            OR NULLIF({port_alias}->>'webserver', '') IS NOT NULL
            OR {technologies})"
    )
}

fn port_number_from_json(entry: &serde_json::Value) -> Option<u16> {
    let value = entry.get("port")?;
    let raw = value
        .as_u64()
        .map(|n| n.to_string())
        .or_else(|| value.as_str().map(|s| s.trim().to_string()))?;
    let port = raw.parse::<u16>().ok()?;
    (port > 0).then_some(port)
}

fn port_state_is_open_json(entry: &serde_json::Value) -> bool {
    entry
        .get("state")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|state| !state.is_empty())
        .map(|state| state.eq_ignore_ascii_case("open"))
        .unwrap_or(true)
}

fn json_text_field_non_empty(entry: &serde_json::Value, key: &str) -> bool {
    entry
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
}

fn json_value_non_empty(value: Option<&serde_json::Value>) -> bool {
    match value {
        Some(serde_json::Value::Array(items)) => !items.is_empty(),
        Some(serde_json::Value::Object(items)) => !items.is_empty(),
        Some(serde_json::Value::String(value)) => !value.trim().is_empty(),
        Some(serde_json::Value::Null) | None => false,
        Some(_) => true,
    }
}

fn port_has_service_surface_json(entry: &serde_json::Value) -> bool {
    let informative_service = entry
        .get("service")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|service| !service.is_empty())
        .map(|service| {
            let first = service
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            !matches!(
                first.as_str(),
                "tcpwrapped" | "unknown" | "open" | "filtered" | "closed"
            ) && port_number_from_json(entry) != Some(53)
        })
        .unwrap_or(false);
    informative_service
        || [
            "version",
            "product",
            "service_product",
            "service_version",
            "banner",
            "webserver",
        ]
        .iter()
        .any(|key| json_text_field_non_empty(entry, key))
        || json_value_non_empty(entry.get("technologies"))
}

/// Parse the current confirmed-open service ports from a target `ports` JSONB
/// array. This mirrors the gate's SERVICE-FINGERPRINT denominator closely: an
/// absent/empty state is treated as open, and DNS/53 does not require nmap
/// service fingerprinting for multi-port hosts.
pub fn confirmed_open_service_ports_from_ports_json(ports: &serde_json::Value) -> Vec<u16> {
    let mut out = BTreeSet::new();
    let Some(items) = ports.as_array() else {
        return Vec::new();
    };
    for entry in items {
        let Some(port) = port_number_from_json(entry) else {
            continue;
        };
        if port == 53 || !port_state_is_open_json(entry) {
            continue;
        }
        out.insert(port);
    }
    out.into_iter().collect()
}

/// Return confirmed-open ports that still lack a terminal service surface in
/// `targets.ports[]`. This is used for read-model diagnostics; full PASS/BLOCK
/// authority still lives in `coverage_truth_facts` and the gate.
pub fn missing_service_fingerprint_ports_from_ports_json(ports: &serde_json::Value) -> Vec<u16> {
    let mut out = BTreeSet::new();
    let Some(items) = ports.as_array() else {
        return Vec::new();
    };
    for entry in items {
        let Some(port) = port_number_from_json(entry) else {
            continue;
        };
        if port == 53 || !port_state_is_open_json(entry) || port_has_service_surface_json(entry) {
            continue;
        }
        out.insert(port);
    }
    out.into_iter().collect()
}

fn port_has_nmap_fingerprint_sql(
    target_alias: &str,
    port_alias: &str,
    apply_window: bool,
) -> String {
    let window = if apply_window {
        "AND f.detected_at >= $2"
    } else {
        ""
    };
    format!(
        "EXISTS (
            SELECT 1
              FROM fingerprints f
             WHERE f.target_id = {target_alias}.id
               AND f.project_path IS NOT DISTINCT FROM {target_alias}.project_path
               AND lower(COALESCE(f.source, '')) = 'nmap'
               AND COALESCE(f.evidence->>'port', '') = {port_alias}->>'port'
               AND (NULLIF(trim(f.name), '') IS NOT NULL
                    OR NULLIF(trim(f.version), '') IS NOT NULL)
               {window}
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

fn fingerprint_exists_sql(alias: &str, apply_window: bool) -> String {
    if apply_window {
        format!(
            "EXISTS (SELECT 1 FROM fingerprints f WHERE f.target_id = {alias}.id AND f.project_path IS NOT DISTINCT FROM {alias}.project_path AND f.detected_at >= $2)"
        )
    } else {
        format!("EXISTS (SELECT 1 FROM fingerprints f WHERE f.target_id = {alias}.id AND f.project_path IS NOT DISTINCT FROM {alias}.project_path)")
    }
}

fn service_from_ports_sql(alias: &str, apply_window: bool) -> String {
    let ports = ports_array_expr(alias);
    let open_port = open_port_sql("p");
    let required_port = service_fingerprint_required_port_sql("p");
    let service_surface = port_has_service_surface_sql("p");
    let nmap_port_fingerprint = port_has_nmap_fingerprint_sql(alias, "p", apply_window);
    let terminal_port = format!("(COALESCE({service_surface}, false) OR {nmap_port_fingerprint})");
    format!(
        "({fresh_ports}
            AND EXISTS (
                SELECT 1 FROM jsonb_array_elements({ports}) p
                 WHERE {open_port}
            )
            AND (
                (
                    EXISTS (
                        SELECT 1 FROM jsonb_array_elements({ports}) p
                         WHERE {required_port}
                    )
                    AND NOT EXISTS (
                        SELECT 1 FROM jsonb_array_elements({ports}) p
                         WHERE {required_port}
                           AND NOT {terminal_port}
                    )
                )
                OR (
                    NOT EXISTS (
                        SELECT 1 FROM jsonb_array_elements({ports}) p
                         WHERE {required_port}
                    )
                    AND NOT EXISTS (
                        SELECT 1 FROM jsonb_array_elements({ports}) p
                         WHERE {open_port}
                           AND NOT {terminal_port}
                    )
                )
            ))",
        fresh_ports = fresh_ports_sql(alias, apply_window),
        ports = ports,
        open_port = open_port,
        required_port = required_port,
        terminal_port = terminal_port
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

/// EAS-LIVENESS：httpx 响应、nmap explicit `Host is up`, or an open port proves
/// host liveness; passive `real_ip` cache never participates. Phase D:
/// `apply_window` only consumes this stage-run's liveness/port timestamps.
fn build_liveness_values_sql(apply_window: bool) -> String {
    let filter = if apply_window {
        format!(
            "AND (((t.http_status IS NOT NULL OR t.liveness_state = 'alive') \
                    AND t.liveness_checked_at >= $2) \
              OR {fresh_ports})",
            fresh_ports = fresh_ports_sql("t", true),
        )
    } else {
        format!(
            "AND (t.http_status IS NOT NULL OR t.liveness_state = 'alive' OR {fresh_ports})",
            fresh_ports = fresh_ports_sql("t", false),
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

fn http_port_surface_sql(port_alias: &str) -> String {
    format!(
        "(lower(COALESCE({port_alias}->>'service', '')) IN ('http', 'https', 'http-alt', 'http-proxy')
            OR lower(COALESCE({port_alias}->>'service', '')) LIKE '%http%'
            OR NULLIF({port_alias}->>'url', '') IS NOT NULL
            OR NULLIF({port_alias}->>'http_status', '') IS NOT NULL
            OR NULLIF({port_alias}->>'webserver', '') IS NOT NULL)"
    )
}

/// EAS web-stack denominator: assets with a freshly confirmed HTTP(S) surface.
/// `targets.http_status` comes from httpx/WhatWeb landing; `targets.ports[]`
/// covers nmap/http service detection on concrete IP:port surfaces.
fn build_eas_web_capable_values_sql(apply_window: bool) -> String {
    let ports = ports_array_expr("t");
    let open_port = open_port_sql("p");
    let http_port = http_port_surface_sql("p");
    let port_window = if apply_window {
        "AND t.ports_scanned_at >= $2"
    } else {
        ""
    };
    let http_status_clause = if apply_window {
        "t.http_status IS NOT NULL AND t.liveness_checked_at >= $2"
    } else {
        "t.http_status IS NOT NULL"
    };
    build_in_scope_values_sql(
        "",
        &format!(
            "AND (({http_status_clause})
                OR (jsonb_typeof(t.ports) = 'array' {port_window}
                    AND EXISTS (
                        SELECT 1 FROM jsonb_array_elements({ports}) p
                         WHERE {open_port} AND {http_port}
                    )))"
        ),
        None,
    )
}

/// EAS-PORT：端口扫描结果（`ports` 为非空 JSONB 数组）。判空走 `jsonb_typeof =
/// 'array'` + 比较式（不裸调 `jsonb_array_length`，否则非数组 `ports` 会抛
/// `cannot get array length of a non-array`），与 `engagement_truth` 同款守卫。
fn build_port_values_sql(apply_window: bool) -> String {
    // PORT belongs to the concrete target row that was scanned. `real_ip` is a
    // passive convenience cache for domain→IP display and must never project an
    // IP target's active result onto another domain/URL identity.
    let filter = format!("AND {}", fresh_ports_sql("t", apply_window));
    build_in_scope_values_sql("", &filter, None)
}

/// EAS-SERVICE-FINGERPRINT：该 host 的每个 SERVICE-applicable confirmed-open
/// port 都已有 service/version/product/banner/webserver/technology 之类的端口级
/// 服务面，或有同 target、同 port 的 nmap service fingerprint 行。弱服务名
///（tcpwrapped/unknown/open/...）不算强服务面，但 nmap 对该 port 的 terminal
/// fingerprint 行可以关闭该端口，避免不可进一步识别的服务被无限重扫。泛化
/// `fingerprints` 不再足够：WhatWeb 是 web-origin 技术栈补充，不能替代
/// IP:port 的服务指纹。DNS/53 只有在 DNS-only 主机且有强表面/nmap 结果时
/// 才作为 SERVICE found；多端口主机上的 bare DNS/53 不阻塞其它服务闭环。
/// Phase D 行级窗：`apply_window` ⇒ 只数本次 stage-run 探到的端口服务
///（`t.ports_scanned_at >= $2` / `f.detected_at >= $2`）。
fn build_service_fp_values_sql(apply_window: bool) -> String {
    let ports_clause = service_from_ports_sql("t", apply_window);
    // SERVICE is likewise target+port identity, never a relation-cache
    // projection through `targets.real_ip`.
    build_in_scope_values_sql("", &format!("AND {ports_clause}"), None)
}

/// EAS-WEB-FINGERPRINT：WhatWeb web-origin stack facts. This is deliberately
/// separate from SERVICE-FINGERPRINT: web-origin technologies enrich UI/targets
/// but do not prove every IP:port has nmap-style service/version coverage.
fn build_web_fp_values_sql(apply_window: bool) -> String {
    build_in_scope_values_sql(
        "JOIN fingerprints f ON f.target_id = t.id",
        "AND f.project_path IS NOT DISTINCT FROM t.project_path
         AND lower(COALESCE(f.source, '')) = 'whatweb'
         AND f.category IN ('web_server', 'technology')
         AND NULLIF(trim(f.name), '') IS NOT NULL",
        apply_window.then_some("f.detected_at"),
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
        "AND jar.project_path IS NOT DISTINCT FROM t.project_path",
        apply_window.then_some("jar.analyzed_at"),
    )
}

/// ENUM-DIR：该 host 有目录枚举产物（ffuf/gobuster → directory_entries）。Phase D
/// 行级窗：`apply_window` ⇒ 只数本次 stage-run 落库的条目（`de.created_at >= $2`）。
fn build_dir_values_sql(apply_window: bool) -> String {
    build_in_scope_values_sql(
        "JOIN directory_entries de ON de.target_id = t.id",
        "AND de.project_path IS NOT DISTINCT FROM t.project_path",
        apply_window.then_some("de.created_at"),
    )
}

/// ENUM-PARAM：该 host 有带参端点（arjun/katana → api_endpoints.params 非空）。
/// 判空同 [`build_port_values_sql`]：`jsonb_typeof = 'array'` + 比较式，避免对
/// 非数组 `params` 调 `jsonb_array_length` 抛错。
fn build_param_values_sql(apply_window: bool) -> String {
    build_in_scope_values_sql(
        "JOIN api_endpoints ae ON ae.target_id = t.id",
        "AND ae.project_path IS NOT DISTINCT FROM t.project_path
         AND jsonb_typeof(ae.params) = 'array' AND ae.params <> '[]'::jsonb",
        apply_window.then_some("ae.discovered_at"),
    )
}

/// ENUM-JSAPI：该 host 有 JS/爬虫抽取的端点（api_endpoints.source）。
fn build_jsapi_values_sql(apply_window: bool) -> String {
    build_in_scope_values_sql(
        "JOIN api_endpoints ae ON ae.target_id = t.id",
        "AND ae.project_path IS NOT DISTINCT FROM t.project_path
         AND ae.source IN ('js_analysis', 'crawler')",
        apply_window.then_some("ae.discovered_at"),
    )
}

/// RDNS (host-aware 2c-3): in-scope IP/CIDR targets that have a 'PTR' dns_records
/// row (reverse DNS landed). Reuses `dns_records` (no schema change) + IP-type filter.
fn build_rdns_values_sql() -> String {
    build_in_scope_values_sql(
        "JOIN dns_records dr ON dr.target_id = t.id",
        &format!(
            "AND dr.project_path IS NOT DISTINCT FROM t.project_path
             AND dr.record_type = 'PTR' AND t.target_type::text IN {IP_TYPE_IN_LIST}"
        ),
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
    pub web_fp_values: &'a HashSet<String>,
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
        if inputs.web_fp_values.contains(asset) {
            facts.push((asset.clone(), TECH_EAS_WEB_FP));
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
    let web_fp_values = fetch_values(pool, &build_web_fp_values_sql(aw), org_id, run_start).await?;
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
            web_fp_values: &web_fp_values,
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

/// In-scope assets that currently have an HTTP(S) surface and therefore need
/// per-origin web-stack fingerprinting in EAS.
pub async fn eas_web_capable_assets(
    pool: &PgPool,
    org_id: Option<Uuid>,
    run_start: Option<DateTime<Utc>>,
) -> Result<HashSet<String>> {
    let aw = run_start.is_some();
    fetch_values(
        pool,
        &build_eas_web_capable_values_sql(aw),
        org_id,
        run_start,
    )
    .await
}

/// Current confirmed-open service ports for a set of in-scope assets. This is a
/// read-model helper for EAS tooling, not a PASS/BLOCK shortcut: it lets wrapper
/// tools and background listeners stay consistent with the `targets.ports[]`
/// denominator the gate already uses.
const CONFIRMED_OPEN_SERVICE_PORTS_FOR_ASSETS_SQL: &str = r#"
        SELECT t.value,
               CASE
                 WHEN jsonb_typeof(t.ports) = 'array' THEN t.ports
                 ELSE '[]'::jsonb
               END AS ports
          FROM targets t
         WHERE t.scope::text = 'in'
           AND t.value = ANY($1)
           AND ($2::uuid IS NULL OR t.organization_id = $2)
           AND ($3::text IS NULL OR t.project_path = $3)
        "#;

pub async fn confirmed_open_service_ports_for_assets(
    pool: &PgPool,
    org_id: Option<Uuid>,
    project_path: Option<&str>,
    assets: &[String],
) -> Result<Vec<ConfirmedOpenServicePorts>> {
    if assets.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<(String, serde_json::Value)> =
        sqlx::query_as(CONFIRMED_OPEN_SERVICE_PORTS_FOR_ASSETS_SQL)
            .bind(assets)
            .bind(org_id)
            .bind(project_path)
            .fetch_all(pool)
            .await?;

    let mut by_asset: BTreeMap<String, BTreeSet<u16>> = BTreeMap::new();
    for (asset, ports) in rows {
        by_asset
            .entry(asset)
            .or_default()
            .extend(confirmed_open_service_ports_from_ports_json(&ports));
    }

    Ok(by_asset
        .into_iter()
        .filter_map(|(asset, ports)| {
            (!ports.is_empty()).then(|| ConfirmedOpenServicePorts {
                asset,
                ports: ports.into_iter().collect(),
            })
        })
        .collect())
}

fn build_dead_asset_values_sql() -> String {
    build_in_scope_values_sql("", "AND t.liveness_state = 'dead'", None)
}

/// In-scope assets EAS has confirmed dead (`targets.liveness_state = 'dead'`),
/// for the downstream coverage gate to drop from its denominator (design
/// 2026-07-02-dead-asset-liveness-state §5.1). Only `'dead'` is returned, never
/// `'unreachable'` — an unreachable verdict may be a transient network / WAF
/// condition, so it stays in scope (conservative). `org_id = None` = whole-DB
/// in-scope set (asset-axis isolation off).
pub async fn dead_asset_values(pool: &PgPool, org_id: Option<Uuid>) -> Result<HashSet<String>> {
    fetch_values(pool, &build_dead_asset_values_sql(), org_id, None).await
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
            web_fp_values: empty,
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
        assert!(sql.contains("dr.project_path IS NOT DISTINCT FROM t.project_path"));
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
        assert!(sql.contains("ta.project_path IS NOT DISTINCT FROM t.project_path"));
        assert!(sql.contains("FROM targets child"));
        assert!(sql.contains("child.organization_id = t.organization_id"));
        assert!(sql.contains("child.project_path IS NOT DISTINCT FROM t.project_path"));
        assert!(sql.contains("child.scope::text = 'in'"));
        assert!(sql.contains("child.target_type::text = 'domain'"));
        assert!(sql.contains("trim(trailing '.' FROM ta.value)"));
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
    fn subdomain_sql_on_windows_target_assets_latest_observation() {
        // freshness_window ON (Phase B, design 2026-06-22 §3.3): the SUBDOMAIN
        // dimension only counts in-scope targets whose subdomain child rows were
        // discovered this stage-run (`target_assets.discovered_at >= $2`).
        let sql = build_subdomain_target_values_sql(true);
        assert!(
            sql.contains("GREATEST(ta.discovered_at, ta.updated_at) >= $2"),
            "on must window the latest target_assets observation: {sql}"
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
        assert!(live.contains("t.http_status IS NOT NULL"));
        assert!(
            !live.contains("t.http_status IS NOT NULL OR t.real_ip"),
            "passive real_ip cache must not close EAS liveness: {live}"
        );
        assert!(live.contains("jsonb_typeof(t.ports) = 'array' AND t.ports <> '[]'::jsonb"));
        assert!(build_port_values_sql(false)
            .contains("jsonb_typeof(t.ports) = 'array' AND t.ports <> '[]'::jsonb"));
        assert!(
            !build_port_values_sql(false).contains("t.real_ip"),
            "passive real_ip cache must not close another target's PORT cell"
        );
        let service = build_service_fp_values_sql(false);
        assert!(
            !service.contains("t.real_ip"),
            "passive real_ip cache must not close another target's SERVICE cell"
        );
        assert!(service.contains("FROM fingerprints f"));
        assert!(service.contains("f.project_path IS NOT DISTINCT FROM t.project_path"));
        assert!(service.contains("lower(COALESCE(f.source, '')) = 'nmap'"));
        assert!(service.contains("COALESCE(f.evidence->>'port', '') = p->>'port'"));
        assert!(service.contains("NOT EXISTS"));
        assert!(service.contains("COALESCE("));
        assert!(service.contains(", false)"));
        assert!(service.contains("jsonb_array_elements(CASE WHEN jsonb_typeof(t.ports)"));
        assert!(service.contains("lower(COALESCE(NULLIF(p->>'state', ''), 'open')) = 'open'"));
        assert!(service.contains("p->>'service'"));
        assert!(service.contains("NOT IN ('tcpwrapped', 'unknown', 'open', 'filtered', 'closed')"));
        assert!(service.contains("p->>'version'"));
        assert!(service.contains("p->>'product'"));
        assert!(service.contains("p->>'banner'"));
        assert!(build_dir_values_sql(false)
            .contains("JOIN directory_entries de ON de.target_id = t.id"));
        assert!(build_dir_values_sql(false)
            .contains("de.project_path IS NOT DISTINCT FROM t.project_path"));
        let param = build_param_values_sql(false);
        assert!(param.contains("JOIN api_endpoints ae ON ae.target_id = t.id"));
        assert!(param.contains("ae.project_path IS NOT DISTINCT FROM t.project_path"));
        assert!(param.contains("jsonb_typeof(ae.params) = 'array' AND ae.params <> '[]'::jsonb"));
        let jsapi = build_jsapi_values_sql(false);
        assert!(jsapi.contains("JOIN api_endpoints ae ON ae.target_id = t.id"));
        assert!(jsapi.contains("ae.project_path IS NOT DISTINCT FROM t.project_path"));
        assert!(jsapi.contains("ae.source IN ('js_analysis', 'crawler')"));
    }

    #[test]
    fn nmap_port_fingerprint_is_terminal_even_for_weak_service_names() {
        let nmap = port_has_nmap_fingerprint_sql("t", "p", false);
        assert!(nmap.contains("lower(COALESCE(f.source, '')) = 'nmap'"));
        assert!(nmap.contains("COALESCE(f.evidence->>'port', '') = p->>'port'"));
        assert!(nmap.contains("f.project_path IS NOT DISTINCT FROM t.project_path"));
        assert!(nmap.contains("NULLIF(trim(f.name), '') IS NOT NULL"));
        assert!(
            !nmap.contains("NOT IN ('tcpwrapped', 'unknown', 'open', 'filtered', 'closed')"),
            "port-scoped nmap attempts must close tcpwrapped/unknown style terminal results: {nmap}"
        );

        let strong_surface = informative_service_sql("p");
        assert!(strong_surface
            .contains("NOT IN ('tcpwrapped', 'unknown', 'open', 'filtered', 'closed')"));
    }

    #[test]
    fn service_fp_sql_does_not_require_bare_dns_53_on_multi_service_hosts() {
        let required = service_fingerprint_required_port_sql("p");
        assert!(required.contains("lower(COALESCE(NULLIF(p->>'state', ''), 'open')) = 'open'"));
        assert!(required.contains("COALESCE(NULLIF(p->>'port', ''), '') <> '53'"));

        let sql = service_from_ports_sql("t", false);
        assert!(sql.contains("WHERE (NULLIF(p->>'port', '') IS NOT NULL"));
        assert!(sql.contains("COALESCE(NULLIF(p->>'port', ''), '') <> '53'"));
        assert!(sql.contains("OR (\n                    NOT EXISTS"));
    }

    #[test]
    fn confirmed_open_service_ports_json_matches_eas_service_denominator() {
        let ports = serde_json::json!([
            {"port": "22", "state": "open", "service": "ssh"},
            {"port": 53, "state": "open", "service": "domain"},
            {"port": "82", "state": "open", "service": ""},
            {"port": "443", "state": "filtered"},
            {"port": "50002"}
        ]);

        assert_eq!(
            confirmed_open_service_ports_from_ports_json(&ports),
            vec![22, 82, 50002]
        );
        assert_eq!(
            missing_service_fingerprint_ports_from_ports_json(&ports),
            vec![82, 50002]
        );
    }

    #[test]
    fn confirmed_ports_exact_workspace_rejects_legacy_project_rows() {
        let sql = CONFIRMED_OPEN_SERVICE_PORTS_FOR_ASSETS_SQL;

        assert!(sql.contains("($3::text IS NULL OR t.project_path = $3)"));
        assert!(!sql.contains("t.project_path = ''"));
        assert!(!sql.contains("t.project_path IS NULL"));
    }

    #[test]
    fn weak_service_names_are_missing_service_fingerprint_json() {
        let ports = serde_json::json!([
            {"port": "80", "state": "open", "service": "http"},
            {"port": "81", "state": "open", "service": "open"},
            {"port": "82", "state": "open", "service": "tcpwrapped"},
            {"port": "83", "state": "open", "technologies": ["nginx"]},
            {"port": "84", "state": "open", "version": "1.2.3"}
        ]);

        assert_eq!(
            missing_service_fingerprint_ports_from_ports_json(&ports),
            vec![81, 82]
        );
    }

    #[test]
    fn js_values_sql_joins_js_analysis_results_and_scoping() {
        // ENUM-JS 真值（design 2026-07-01 §4.1）：join js_analysis_results + org/scope 隔离。
        let sql = build_js_values_sql(false);
        assert!(sql.contains("JOIN js_analysis_results jar ON jar.target_id = t.id"));
        assert!(sql.contains("jar.project_path IS NOT DISTINCT FROM t.project_path"));
        assert!(sql.contains("t.scope::text = 'in'"));
        assert!(sql.contains("($1 IS NULL OR t.organization_id = $1)"));
        assert!(!sql.contains("$2"), "off must bind only $1: {sql}");
        // freshness window on ⇒ 只数本次 stage-run 落库（jar.analyzed_at >= $2）。
        assert!(build_js_values_sql(true).contains("jar.analyzed_at >= $2"));
    }

    #[test]
    fn web_fingerprint_truth_requires_current_target_project() {
        let sql = build_web_fp_values_sql(false);
        assert!(sql.contains("JOIN fingerprints f ON f.target_id = t.id"));
        assert!(sql.contains("f.project_path IS NOT DISTINCT FROM t.project_path"));
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
        assert!(
            !sql.contains("$2"),
            "presence-only mode must not bind cutoff: {sql}"
        );

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
        assert!(!build_service_fp_values_sql(false).contains("jsonb_array_length(t.ports)"));
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
    fn dead_asset_values_sql_filters_scope_and_dead_only() {
        // Dead-asset P3: denominator-exclusion query selects only in-scope,
        // liveness_state='dead' rows (never 'unreachable'), org-narrowable via $1.
        let sql = build_dead_asset_values_sql();
        assert!(sql.contains("t.scope::text = 'in'"));
        assert!(sql.contains("t.liveness_state = 'dead'"));
        assert!(!sql.contains("unreachable"));
        assert!(sql.contains("($1 IS NULL OR t.organization_id = $1)"));
        // presence-only: no freshness window binds $2
        assert!(!sql.contains("$2"));
    }

    #[test]
    fn active_dimension_sqls_on_window_their_collection_timestamp() {
        // freshness_window ON (Phase D, design 2026-06-22 §3.3/D3): each EAS/ENUM dim
        // only counts rows collected this stage-run, via its per-dim timestamp >= $2.
        // PORT/LIVENESS/IPWHOIS = targets columns (migration 20260623000001);
        // SERVICE-FP/DIR/PARAM/JSAPI = existing child-table row timestamps.
        let live = build_liveness_values_sql(true);
        assert!(live.contains("t.liveness_checked_at >= $2"));
        assert!(
            live.contains("t.liveness_state = 'alive'"),
            "an explicit nmap Host-is-up observation must corroborate LIVENESS without an open port"
        );
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
        let web_fp = subs(&["b.com"]);
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
                web_fp_values: &web_fp,
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
                ("b.com".to_string(), TECH_EAS_WEB_FP),
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
                web_fp_values: &one,
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
                ("a.com".to_string(), TECH_EAS_WEB_FP),
                ("a.com".to_string(), TECH_ENUM_JS),
                ("a.com".to_string(), TECH_ENUM_DIR),
                ("a.com".to_string(), TECH_ENUM_PARAM),
                ("a.com".to_string(), TECH_ENUM_JSAPI),
            ]
        );
    }
}
