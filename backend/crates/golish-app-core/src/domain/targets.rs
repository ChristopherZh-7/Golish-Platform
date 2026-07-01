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

fn collapse_attack_surface_seed_aliases(targets: Vec<Target>) -> (Vec<Target>, BTreeSet<String>) {
    let direct_ips: BTreeSet<String> = targets.iter().filter_map(direct_ip_seed_key).collect();
    let direct_ip_aliases = direct_ip_alias_keys(&targets);
    if direct_ips.is_empty() {
        return (targets, direct_ip_aliases);
    }
    let collapsed = targets
        .into_iter()
        .filter(|target| {
            if direct_ip_seed_key(target).is_some() {
                return true;
            }
            resolved_seed_ip_key(target).is_none_or(|ip| !direct_ips.contains(&ip))
        })
        .collect();
    (collapsed, direct_ip_aliases)
}

/// Rank in-scope targets into attack-surface seeds by descending priority
/// (stable tiebreak on value), optionally capping to `cap` (D3: per-org cap,
/// `None` = no cap / default off). Pure — the caller projects each ranked target
/// into the rich seed JSON the EAS handoff returns.
pub fn rank_attack_surface_seeds(targets: Vec<Target>, cap: Option<usize>) -> Vec<Target> {
    let (mut targets, direct_ip_aliases) = collapse_attack_surface_seed_aliases(targets);
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

/// EAS host-aware port/service delegation (design 2026-06-30-eas-domain-port-
/// delegation): the set of non-IP in-scope asset values whose resolved IP is
/// already an in-scope IP target. Their `GOLISH-EAS-PORT` /
/// `GOLISH-EAS-SERVICE-FINGERPRINT` coverage is delegated to that IP target, so
/// the EAS gate drops those two techniques for these assets (LIVENESS stays
/// required). Mirrors [`collapse_attack_surface_seed_aliases`]'s alias rule but
/// keeps the domain in the coverage denominator for liveness. Empty when there
/// are no in-scope IP targets to receive the delegated coverage.
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
    fn rank_attack_surface_seeds_collapses_domain_aliases_to_existing_ip_targets() {
        let ip = seed("115.28.135.55", "ip", "", None);
        let alias = seed("moresec.cn", "domain", "115.28.135.55", Some(200));
        let www_alias = seed("www.moresec.cn", "domain", "115.28.135.55", None);
        let sibling = seed("m.moresec.cn", "domain", "", None);

        let ranked = rank_attack_surface_seeds(vec![alias, www_alias, sibling, ip], None);

        assert_eq!(
            ranked
                .iter()
                .map(|target| target.value.as_str())
                .collect::<Vec<_>>(),
            vec!["115.28.135.55", "m.moresec.cn"]
        );
    }

    #[test]
    fn rank_attack_surface_seeds_collapses_ip_url_endpoint_aliases_before_cap() {
        let ip = seed("115.28.135.55", "ip", "", None);
        let endpoint = seed("http://115.28.135.55:8080/login", "url", "", Some(200));
        let bare_domain = seed("bare.example.com", "domain", "", None);

        let ranked = rank_attack_surface_seeds(vec![endpoint, bare_domain, ip], Some(1));

        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].value, "115.28.135.55");
    }

    #[test]
    fn eas_port_delegated_domain_values_delegates_alias_keeps_orphan_domain() {
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
        // … a domain whose real_ip is NOT an in-scope IP keeps its own PORT/SERVICE.
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
