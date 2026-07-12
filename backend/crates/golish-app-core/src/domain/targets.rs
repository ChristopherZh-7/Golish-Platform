//! Shared target / recon domain DTOs (servitization S1-3).
//!
//! These are the remote-ready contract types for the recon `targets` surface:
//! every field is serializable, no `PgPool`/closures, no crate-private deps.
//! They live here (not in `golish-recon-app`) so consuming services — pentest,
//! agent, … — can hold them without an upward sibling-crate dependency. The
//! `sqlx::FromRow` row adapters (`TargetRow` / `DirEntryRow`) and their `From`
//! conversions stay private inside `golish-recon-app` (DB-layer detail).

use std::collections::BTreeSet;
use std::net::IpAddr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct Target {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub target_type: TargetType,
    pub value: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub notes: String,
    pub scope: Scope,
    pub status: TargetStatus,
    #[serde(default)]
    pub grp: String,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    #[ts(type = "number | null")]
    pub time_window_start: Option<u64>,
    #[serde(default)]
    #[ts(type = "number | null")]
    pub time_window_end: Option<u64>,
    #[serde(default)]
    pub organization_id: Option<String>,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    // The frontend view-model replaces this with a structured `PortInfo[]`; the
    // backend stores it as opaque JSON. Skip it from the generated type so the
    // binding stays free of the serde_json `JsonValue` import (the FE layers its
    // own `ports` + `technologies` on top via intersection).
    #[serde(default)]
    #[ts(skip)]
    pub ports: Vec<serde_json::Value>,
    #[serde(default)]
    pub real_ip: String,
    #[serde(default)]
    pub cdn_waf: String,
    #[serde(default)]
    pub http_title: String,
    #[serde(default)]
    pub http_status: Option<i32>,
    #[serde(default)]
    pub webserver: String,
    #[serde(default)]
    pub os_info: String,
    #[serde(default)]
    pub content_type: String,
    /// Persistent liveness verdict stamped by EAS probing: `alive` / `dead` /
    /// `unreachable`; `None` = not probed yet (design 2026-07-02-dead-asset-
    /// liveness-state). Downstream stages exclude confirmed-dead assets from the
    /// coverage denominator.
    #[serde(default)]
    #[ts(optional)]
    pub liveness_state: Option<String>,
    /// Failure detail behind a non-alive `liveness_state`
    /// (`dns_fail` / `timeout` / `conn_refused` / `no_service` / `probe_error`).
    #[serde(default)]
    #[ts(optional)]
    pub liveness_reason: Option<String>,
    #[ts(type = "number")]
    pub created_at: u64,
    #[ts(type = "number")]
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub enum TargetType {
    Domain,
    Ip,
    Cidr,
    Url,
    Wildcard,
}

impl TargetType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Domain => "domain",
            Self::Ip => "ip",
            Self::Cidr => "cidr",
            Self::Url => "url",
            Self::Wildcard => "wildcard",
        }
    }
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "ip" => Self::Ip,
            "cidr" => Self::Cidr,
            "url" => Self::Url,
            "wildcard" => Self::Wildcard,
            _ => Self::Domain,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub enum Scope {
    #[serde(rename = "in")]
    InScope,
    #[serde(rename = "out")]
    OutOfScope,
}

impl Scope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::InScope => "in",
            Self::OutOfScope => "out",
        }
    }
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "out" => Self::OutOfScope,
            _ => Self::InScope,
        }
    }
}

/// A target's furthest-completed pentest stage, aligned to the harness pipeline
/// (design 2026-06-14-target-status-stage-aligned). Doubles as the AI's
/// per-target resume/skip signal: before running a stage on a target, the agent
/// skips it when its status is already at/after that stage. Ordered
/// `new < passive < active < enumerated < vuln_scan < verified`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub enum TargetStatus {
    /// Discovered, no work done yet.
    New,
    /// Passive recon done (no direct contact) — recon `PassiveInternet`.
    Passive,
    /// Active recon done (ports/services/tech) — recon `ActiveCollection`.
    Active,
    /// Enumeration done (dirs/params/JS-API) — stage `Enumeration`.
    Enumerated,
    /// Vulnerability scan done — stage `VulnTriage`.
    VulnScan,
    /// Verified / actively tested — stage `Verification`.
    Verified,
}

impl TargetStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Passive => "passive",
            Self::Active => "active",
            Self::Enumerated => "enumerated",
            Self::VulnScan => "vuln_scan",
            Self::Verified => "verified",
        }
    }
    /// Parse from the DB/tool string. Also accepts the legacy coarse values
    /// (`recon`/`recon_done`/`scanning`/`tested`) so any pre-migration string
    /// still maps onto the stage-aligned lifecycle.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "passive" | "recon" => Self::Passive,
            "active" | "recon_done" => Self::Active,
            "enumerated" => Self::Enumerated,
            "vuln_scan" | "scanning" => Self::VulnScan,
            "verified" | "tested" => Self::Verified,
            _ => Self::New,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TargetStore {
    pub targets: Vec<Target>,
}

/// Attack-surface seed priority for a target (design 2026-06-24-intel-to-eas-
/// handoff §4 L1c / D3). Higher = scan sooner. Lets EAS prioritise instead of
/// flat-scanning a large in-scope set: resolved hosts (have a real_ip) and
/// already-alive hosts (have an http_status) rank highest, web-capable classes
/// (domain/url) above bare IPs, bare IPs above whole netblocks (cidr/wildcard).
pub fn attack_surface_priority(t: &Target) -> i32 {
    let mut score = 0;
    if !t.real_ip.trim().is_empty() {
        score += 40;
    }
    if t.http_status.is_some() {
        score += 20;
    }
    score += match t.target_type {
        TargetType::Domain | TargetType::Url => 30,
        TargetType::Ip => 20,
        TargetType::Cidr | TargetType::Wildcard => 5,
    };
    if !t.source.trim().is_empty() && t.source != "manual" {
        score += 10;
    }
    score
}

/// Build a canonical web root URL from a host, optional port, and service hint.
/// Default ports are omitted from the suffix; TLS-looking services and common
/// TLS web ports choose `https`.
pub fn web_root_url(host: &str, port: Option<u16>, service: &str) -> (String, String, Option<u16>) {
    let service = service.to_ascii_lowercase();
    let scheme = if service.contains("https")
        || service.contains("ssl")
        || matches!(port, Some(443 | 8443 | 9443))
    {
        "https"
    } else {
        "http"
    };
    let port_suffix = match (scheme, port) {
        ("http", Some(80)) | ("https", Some(443)) | (_, None) => String::new(),
        (_, Some(port)) => format!(":{port}"),
    };
    (
        format!("{scheme}://{host}{port_suffix}/"),
        scheme.to_string(),
        port,
    )
}

/// Derive a target's persistent liveness verdict from its EAS probe outcome
/// (design 2026-07-02-dead-asset-liveness-state §1.2 / §4). The `alive`
/// predicate mirrors `coverage_truth::build_liveness_values_sql` exactly so the
/// stamped state and the coverage-gate truth never drift: a target is `alive`
/// when it resolved (`real_ip`), answered HTTP (`http_status`), or exposed at
/// least one open port. A probe that actively errored (DNS failure, refused
/// connection, timeout — ledger outcome `error`) → `unreachable`; a probe that
/// completed but found nothing → `dead` (I8: "checked-empty" ≠ "unchecked", so
/// this is only called after a real probe). Returns `(state, reason)`.
pub fn compute_liveness_state(
    http_status: Option<i32>,
    real_ip: &str,
    open_ports: usize,
    probe_errored: bool,
) -> (&'static str, Option<&'static str>) {
    if http_status.is_some() || !real_ip.trim().is_empty() || open_ports > 0 {
        ("alive", None)
    } else if probe_errored {
        ("unreachable", Some("probe_error"))
    } else {
        ("dead", Some("no_service"))
    }
}

fn normalized_ip(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_matches(['[', ']']);
    trimmed.parse::<IpAddr>().ok().map(|ip| ip.to_string())
}

fn url_host_ip(value: &str) -> Option<String> {
    let raw = value.trim();
    let rest = raw
        .strip_prefix("http://")
        .or_else(|| raw.strip_prefix("https://"))?;
    let authority = rest
        .split('/')
        .next()
        .unwrap_or(rest)
        .split('@')
        .next_back()
        .unwrap_or(rest);
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        stripped.split(']').next().unwrap_or(stripped)
    } else {
        authority.split(':').next().unwrap_or(authority)
    };
    normalized_ip(host)
}

fn direct_ip_seed_key(target: &Target) -> Option<String> {
    if target.target_type != TargetType::Ip {
        return None;
    }
    normalized_ip(&target.value)
}

fn resolved_seed_ip_key(target: &Target) -> Option<String> {
    normalized_ip(&target.real_ip).or_else(|| url_host_ip(&target.value))
}

fn direct_ip_alias_keys(targets: &[Target]) -> BTreeSet<String> {
    let direct_ips: BTreeSet<String> = targets.iter().filter_map(direct_ip_seed_key).collect();
    if direct_ips.is_empty() {
        return BTreeSet::new();
    }
    targets
        .iter()
        .filter(|target| direct_ip_seed_key(target).is_none())
        .filter_map(resolved_seed_ip_key)
        .filter(|ip| direct_ips.contains(ip))
        .collect()
}

fn attack_surface_sort_priority(t: &Target, direct_ip_aliases: &BTreeSet<String>) -> i32 {
    let mut score = attack_surface_priority(t);
    if direct_ip_seed_key(t).is_some_and(|ip| direct_ip_aliases.contains(&ip)) {
        score += 50;
    }
    score
}

/// Rank in-scope targets into attack-surface seeds by descending priority
/// (stable tiebreak on value), optionally capping to `cap` (D3: per-org cap,
/// `None` = no cap / default off). Wildcards are passive authorization patterns,
/// not executable hosts; concrete children remain ordinary domain seeds. Pure —
/// the caller projects each ranked target into the rich seed JSON the EAS
/// handoff returns.
pub fn rank_attack_surface_seeds(mut targets: Vec<Target>, cap: Option<usize>) -> Vec<Target> {
    targets.retain(|target| target.target_type != TargetType::Wildcard);
    // A concrete IP may own PORT/SERVICE work for every vhost resolving to it,
    // but it cannot replace those vhosts' Host/SNI-specific LIVENESS/WEB work.
    // Keep every asset in the seed list and only boost the direct IP's ordering.
    let direct_ip_aliases = direct_ip_alias_keys(&targets);
    targets.sort_by(|a, b| {
        attack_surface_sort_priority(b, &direct_ip_aliases)
            .cmp(&attack_surface_sort_priority(a, &direct_ip_aliases))
            .then_with(|| a.value.cmp(&b.value))
    });
    if let Some(n) = cap {
        targets.truncate(n);
    }
    targets
}

/// Rank enumeration web roots so a cut-short enumeration pass spends its budget
/// on the highest-value roots first (design 2026-07-03-enumeration-throughput-
/// optimization PR-C). Alive roots (confirmed `http_status`) rank above unproven
/// ones, then by [`attack_surface_priority`], stable tiebreak on `value`. Unlike
/// `rank_attack_surface_seeds` this does **not** collapse same-IP aliases — two
/// vhosts on one IP can serve different apps, so both remain enumeration targets.
/// `cap = Some(n)` truncates to the top-n (the tail is a caller-side next-wave
/// backlog, never silently dropped); `None` = full set, order only.
pub fn rank_enumeration_web_roots(mut roots: Vec<Target>, cap: Option<usize>) -> Vec<Target> {
    roots.sort_by(|a, b| {
        let a_alive = a.http_status.is_some();
        let b_alive = b.http_status.is_some();
        b_alive
            .cmp(&a_alive)
            .then_with(|| attack_surface_priority(b).cmp(&attack_surface_priority(a)))
            .then_with(|| a.value.cmp(&b.value))
    });
    if let Some(n) = cap {
        roots.truncate(n);
    }
    roots
}

/// EAS host-aware alias exclusion (design 2026-06-30-eas-domain-port-
/// delegation, tightened 2026-07-02): the set of non-IP in-scope asset values
/// whose resolved IP is already an in-scope IP target. These rows are explanatory
/// aliases for the concrete IP host; the EAS gate/read model can drop them from
/// the direct denominator instead of scanning the same host once per domain or
/// URL alias. Domains without a matching IP are not returned here; they remain
/// liveness/vhost-only assets, and PORT/SERVICE still belongs to a concrete
/// IP/CIDR target via `technique_resolver`.
pub fn eas_port_delegated_domain_values(targets: &[Target]) -> BTreeSet<String> {
    let direct_ips: BTreeSet<String> = targets.iter().filter_map(direct_ip_seed_key).collect();
    if direct_ips.is_empty() {
        return BTreeSet::new();
    }
    targets
        .iter()
        .filter(|t| direct_ip_seed_key(t).is_none())
        .filter(|t| resolved_seed_ip_key(t).is_some_and(|ip| direct_ips.contains(&ip)))
        .map(|t| t.value.clone())
        .collect()
}

/// Infer a [`TargetType`] from a raw target value (URL > CIDR > wildcard > IP >
/// domain).
pub fn detect_type(value: &str) -> TargetType {
    let v = value.trim();
    if v.starts_with("http://") || v.starts_with("https://") {
        return TargetType::Url;
    }
    if v.contains('/') {
        return TargetType::Cidr;
    }
    if v.starts_with("*.") {
        return TargetType::Wildcard;
    }
    if v.parse::<std::net::IpAddr>().is_ok() {
        return TargetType::Ip;
    }
    TargetType::Domain
}

/// Parse an ISO 8601 datetime string (with timezone) into UTC.
/// Returns None for empty/whitespace/malformed inputs so callers can pass
/// through to SQL as NULL.
pub fn parse_iso8601(s: Option<&str>) -> Option<chrono::DateTime<chrono::Utc>> {
    let trimmed = s?.trim();
    if trimmed.is_empty() {
        return None;
    }
    chrono::DateTime::parse_from_rfc3339(trimmed)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

/// Fields for an extended recon update. Only non-empty values overwrite
/// existing data when applied.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReconUpdate {
    #[serde(default)]
    pub real_ip: String,
    #[serde(default)]
    pub cdn_waf: String,
    #[serde(default)]
    pub http_title: String,
    #[serde(default)]
    pub http_status: Option<i32>,
    #[serde(default)]
    pub webserver: String,
    #[serde(default)]
    pub os_info: String,
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub ports: serde_json::Value,
}

impl ReconUpdate {
    pub fn new() -> Self {
        Self {
            ports: serde_json::json!([]),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub id: String,
    pub target_id: Option<String>,
    pub url: String,
    pub status_code: Option<i32>,
    pub content_length: Option<i32>,
    pub lines: Option<i32>,
    pub words: Option<i32>,
    pub content_type: String,
    pub tool: String,
    pub created_at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(value: &str, ty: &str, real_ip: &str, http_status: Option<i32>) -> Target {
        serde_json::from_value(serde_json::json!({
            "id": value, "name": value, "type": ty, "value": value,
            "scope": "in", "status": "new", "source": "asset_intel",
            "real_ip": real_ip, "http_status": http_status,
            "created_at": 0, "updated_at": 0,
        }))
        .expect("build test target")
    }

    #[test]
    fn rank_attack_surface_seeds_orders_by_priority_and_caps() {
        // L1c / D3 (design 2026-06-24): resolved+alive web host ranks top, whole
        // netblock bottom; cap truncates.
        let resolved_domain = seed("resolved.example.com", "domain", "1.2.3.4", Some(200));
        let bare_domain = seed("bare.example.com", "domain", "", None);
        let bare_ip = seed("9.9.9.9", "ip", "", None);
        let netblock = seed("10.0.0.0/8", "cidr", "", None);
        let all = vec![
            netblock.clone(),
            bare_ip,
            bare_domain,
            resolved_domain.clone(),
        ];

        let ranked = rank_attack_surface_seeds(all.clone(), None);
        assert_eq!(ranked[0].value, "resolved.example.com");
        assert_eq!(ranked.last().unwrap().value, "10.0.0.0/8");

        let capped = rank_attack_surface_seeds(all, Some(2));
        assert_eq!(capped.len(), 2);
        assert_eq!(capped[0].value, "resolved.example.com");
    }

    #[test]
    fn rank_attack_surface_seeds_excludes_wildcard_pattern_but_keeps_concrete_child() {
        let wildcard = seed("*.example.com", "wildcard", "", None);
        let child = seed("app.example.com", "domain", "", None);

        let ranked = rank_attack_surface_seeds(vec![wildcard, child.clone()], None);

        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].value, child.value);
        assert_eq!(ranked[0].target_type, TargetType::Domain);
    }

    #[test]
    fn web_root_url_derives_scheme_and_default_port_suffix() {
        assert_eq!(
            web_root_url("app.example.com", Some(443), "https"),
            (
                "https://app.example.com/".to_string(),
                "https".to_string(),
                Some(443)
            )
        );
        assert_eq!(
            web_root_url("app.example.com", Some(8443), "http/ssl"),
            (
                "https://app.example.com:8443/".to_string(),
                "https".to_string(),
                Some(8443)
            )
        );
        assert_eq!(
            web_root_url("plain.example.com", Some(80), "http"),
            (
                "http://plain.example.com/".to_string(),
                "http".to_string(),
                Some(80)
            )
        );
    }

    #[test]
    fn rank_attack_surface_seeds_preserves_ip_and_same_ip_vhosts() {
        let ip = seed("115.28.135.55", "ip", "", None);
        let alias = seed("moresec.cn", "domain", "115.28.135.55", Some(200));
        let www_alias = seed("www.moresec.cn", "domain", "115.28.135.55", None);
        let sibling = seed("m.moresec.cn", "domain", "", None);

        let ranked = rank_attack_surface_seeds(vec![alias, www_alias, sibling, ip], None);

        let values = ranked
            .iter()
            .map(|target| target.value.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(ranked.len(), 4);
        assert_eq!(
            values,
            BTreeSet::from([
                "115.28.135.55",
                "moresec.cn",
                "www.moresec.cn",
                "m.moresec.cn",
            ])
        );
    }

    #[test]
    fn rank_attack_surface_seeds_preserves_ip_url_endpoint_alias() {
        let ip = seed("115.28.135.55", "ip", "", None);
        let endpoint = seed("http://115.28.135.55:8080/login", "url", "", Some(200));
        let bare_domain = seed("bare.example.com", "domain", "", None);

        let ranked = rank_attack_surface_seeds(vec![endpoint, bare_domain, ip], None);

        assert_eq!(ranked.len(), 3);
        assert!(ranked.iter().any(|target| target.value == "115.28.135.55"));
        assert!(ranked
            .iter()
            .any(|target| target.value == "http://115.28.135.55:8080/login"));
    }

    #[test]
    fn eas_port_delegated_domain_values_delegates_alias_keeps_orphan_domain_liveness() {
        let ip = seed("115.28.135.55", "ip", "", None);
        let alias = seed("moresec.cn", "domain", "115.28.135.55", Some(200));
        let www_alias = seed("www.moresec.cn", "domain", "115.28.135.55", None);
        // real_ip resolves to an IP that is NOT an in-scope IP target.
        let orphan = seed("m.moresec.cn", "domain", "203.0.113.9", None);
        let no_ip = seed("bare.example.com", "domain", "", None);

        let delegated = eas_port_delegated_domain_values(&[ip, alias, www_alias, orphan, no_ip]);

        // Domains resolving to the in-scope IP delegate PORT/SERVICE to it …
        assert!(delegated.contains("moresec.cn"));
        assert!(delegated.contains("www.moresec.cn"));
        // … a domain whose real_ip is NOT an in-scope IP is not an alias of that
        // IP. It may still carry LIVENESS, but not PORT/SERVICE.
        assert!(!delegated.contains("m.moresec.cn"));
        assert!(!delegated.contains("bare.example.com"));
        // The IP target itself is never delegated (it carries the coverage).
        assert!(!delegated.contains("115.28.135.55"));
    }

    #[test]
    fn eas_port_delegated_domain_values_empty_without_ip_targets() {
        let a = seed("a.example.com", "domain", "1.2.3.4", None);
        let b = seed("b.example.com", "domain", "", None);
        assert!(eas_port_delegated_domain_values(&[a, b]).is_empty());
    }

    #[test]
    fn eas_port_delegated_domain_values_delegates_ip_url_endpoint() {
        // A bare-IP URL endpoint resolves (by host) to the in-scope IP target.
        let ip = seed("115.28.135.55", "ip", "", None);
        let endpoint = seed("http://115.28.135.55:8080/login", "url", "", Some(200));
        let delegated = eas_port_delegated_domain_values(&[ip, endpoint]);
        assert!(delegated.contains("http://115.28.135.55:8080/login"));
    }

    #[test]
    fn rank_enumeration_web_roots_orders_alive_then_priority_and_caps() {
        // PR-C: alive (http_status) roots first, then priority, stable on value;
        // same-IP vhosts are NOT collapsed (both remain enumeration targets).
        let alive_low = seed("z-alive.example.com", "domain", "", Some(200));
        let alive_high = seed("a-alive.example.com", "domain", "1.2.3.4", Some(200));
        let dead_domain = seed("bare.example.com", "domain", "", None);
        let same_ip_a = seed("app1.example.com", "domain", "9.9.9.9", Some(200));
        let same_ip_b = seed("app2.example.com", "domain", "9.9.9.9", Some(200));

        let ranked = rank_enumeration_web_roots(
            vec![
                dead_domain,
                alive_low,
                alive_high.clone(),
                same_ip_a.clone(),
                same_ip_b.clone(),
            ],
            None,
        );
        // Unproven (no http_status) sinks to the bottom.
        assert_eq!(ranked.last().unwrap().value, "bare.example.com");
        // Same-IP vhosts both survive (no alias collapse).
        assert!(ranked.iter().any(|t| t.value == "app1.example.com"));
        assert!(ranked.iter().any(|t| t.value == "app2.example.com"));
        assert_eq!(ranked.len(), 5);

        // Cap keeps the top-N alive/high-priority roots.
        let capped = rank_enumeration_web_roots(
            vec![seed("bare.example.com", "domain", "", None), alive_high],
            Some(1),
        );
        assert_eq!(capped.len(), 1);
        assert_eq!(capped[0].value, "a-alive.example.com");
    }

    #[test]
    fn compute_liveness_state_alive_on_any_signal() {
        // Any one of http_status / real_ip / open_ports proves the host is alive.
        assert_eq!(
            compute_liveness_state(Some(200), "", 0, false),
            ("alive", None)
        );
        assert_eq!(
            compute_liveness_state(None, "1.2.3.4", 0, false),
            ("alive", None)
        );
        assert_eq!(compute_liveness_state(None, "", 3, false), ("alive", None));
        // Alive signal wins even when the probe also reported an error.
        assert_eq!(
            compute_liveness_state(Some(200), "", 0, true),
            ("alive", None)
        );
        // Whitespace-only real_ip is not a signal.
        assert_eq!(
            compute_liveness_state(None, "  ", 0, false),
            ("dead", Some("no_service"))
        );
    }

    #[test]
    fn compute_liveness_state_unreachable_on_probe_error() {
        // Probe actively errored (DNS fail / refused) and found no signal.
        assert_eq!(
            compute_liveness_state(None, "", 0, true),
            ("unreachable", Some("probe_error"))
        );
    }

    #[test]
    fn compute_liveness_state_dead_when_probed_empty() {
        // Probe completed, no signal, no error → confirmed dead (checked-empty).
        assert_eq!(
            compute_liveness_state(None, "", 0, false),
            ("dead", Some("no_service"))
        );
    }

    #[test]
    fn compute_liveness_state_matches_check_constraint_domain() {
        // Every state this returns must be a value the migration's CHECK allows.
        let allowed = ["alive", "dead", "unreachable"];
        for (hs, ip, ports, err) in [
            (Some(200), "", 0, false),
            (None, "", 0, true),
            (None, "", 0, false),
        ] {
            let (state, _) = compute_liveness_state(hs, ip, ports, err);
            assert!(
                allowed.contains(&state),
                "state {state} not in CHECK domain"
            );
        }
    }

    #[test]
    fn target_status_as_str_matches_db_enum_values() {
        // as_str() must equal the Postgres `target_status` enum members exactly
        // (the migration's CREATE TYPE list); they are bound as `$1::target_status`.
        assert_eq!(TargetStatus::New.as_str(), "new");
        assert_eq!(TargetStatus::Passive.as_str(), "passive");
        assert_eq!(TargetStatus::Active.as_str(), "active");
        assert_eq!(TargetStatus::Enumerated.as_str(), "enumerated");
        assert_eq!(TargetStatus::VulnScan.as_str(), "vuln_scan");
        assert_eq!(TargetStatus::Verified.as_str(), "verified");
    }

    #[test]
    fn target_status_serde_wire_form_equals_as_str() {
        // rename_all = "snake_case" → the serialized wire form (what the frontend
        // sees) equals the DB value, so there is one representation everywhere.
        for s in [
            TargetStatus::New,
            TargetStatus::Passive,
            TargetStatus::Active,
            TargetStatus::Enumerated,
            TargetStatus::VulnScan,
            TargetStatus::Verified,
        ] {
            let wire = serde_json::to_value(&s).unwrap();
            assert_eq!(wire, serde_json::Value::String(s.as_str().to_string()));
        }
    }

    #[test]
    fn target_status_from_str_roundtrips_new_values() {
        for s in [
            "new",
            "passive",
            "active",
            "enumerated",
            "vuln_scan",
            "verified",
        ] {
            assert_eq!(TargetStatus::from_str(s).as_str(), s);
        }
    }

    #[test]
    fn target_status_from_str_maps_legacy_values() {
        // Pre-migration strings still map onto the stage-aligned lifecycle so any
        // lingering old value (transcripts/cached JSON) resolves correctly.
        assert_eq!(TargetStatus::from_str("recon"), TargetStatus::Passive);
        assert_eq!(TargetStatus::from_str("recon_done"), TargetStatus::Active);
        assert_eq!(TargetStatus::from_str("scanning"), TargetStatus::VulnScan);
        assert_eq!(TargetStatus::from_str("tested"), TargetStatus::Verified);
        // Unknown / empty falls back to New.
        assert_eq!(TargetStatus::from_str("bogus"), TargetStatus::New);
        assert_eq!(TargetStatus::from_str(""), TargetStatus::New);
    }
}
