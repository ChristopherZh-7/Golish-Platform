//! Asset-map landing: turn one provider invocation's normalized domain/IP
//! observations into org-bound `targets`, complete DNS edges, and service/
//! subdomain relationships. The current invocation is the freshness boundary.
//!
//! Pure planning (`plan_current_run_targets`, `pairs_from_candidates`) is
//! unit-tested without a DB; writes are exact org/type/value upserts and do not
//! reactivate a pre-existing `scope=out` row.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::net::IpAddr;

use serde_json::Value;
use uuid::Uuid;

use golish_app_core::GolishError;

use crate::asset_intel::types::HostIpPair;
use crate::asset_intel::{extract_host_ip_pairs, resolve_field_ref};
use crate::organization_recon::{normalized_host, value_belongs_to_organization};
use crate::organizations::OrganizationCandidates;

/// Host-side fields tried (in priority order) when a provider declares no
/// `normalize.pairs` rule — covers the http_json record shapes (0.zone / quake)
/// so landing still pairs a domain with its surveyed IP.
const DEFAULT_HOST_FIELDS: &[&str] = &["service.http.host", "host", "domain", "hostname", "url"];
/// IP-side fields tried (in priority order); mirrors the providers' IP keys.
const DEFAULT_IP_FIELDS: &[&str] = &["ip", "msg.ip", "ip_addr"];

/// Build a pair rule that runs against a *single record* (a candidate's
/// `evidence.raw`), so the provider's `host_field` / `ip_field` are applied
/// directly without the document-level `$..data[*]` descent.
fn single_record_rule(
    host_field: Vec<String>,
    ip_field: Vec<String>,
) -> golish_pentest::models::AssetIntelPairRule {
    golish_pentest::models::AssetIntelPairRule {
        path: "$".to_string(),
        host_field,
        ip_field,
    }
}

/// Lift `(host, ip)` pairs out of the in-memory candidates of a provider run.
///
/// Each target candidate keeps the full provider record in `evidence.raw`.
/// Cross-provider candidate dedupe moves every contributing evidence object
/// into `evidence.sources`, so all of those records must be inspected with the
/// rule belonging to their own provider. Results are deduped by the exact
/// `(hostname, canonical IP)` pair. Pure — no DB. A hostname may legitimately
/// have multiple A/AAAA observations.
pub(crate) fn pairs_from_candidates(
    candidates: &OrganizationCandidates,
    rules_by_provider: &HashMap<String, Vec<golish_pentest::models::AssetIntelPairRule>>,
) -> Vec<HostIpPair> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for candidate in &candidates.targets {
        let evidence_sources = candidate
            .evidence
            .get("sources")
            .and_then(Value::as_array)
            .filter(|sources| !sources.is_empty());
        let evidence_records: Vec<&Value> = match evidence_sources {
            Some(sources) => sources.iter().collect(),
            None => vec![&candidate.evidence],
        };

        for evidence in evidence_records {
            let Some(raw) = evidence.get("raw") else {
                continue;
            };
            let provider = evidence
                .get("provider")
                .and_then(Value::as_str)
                .unwrap_or(&candidate.source);
            let record_rules: Vec<golish_pentest::models::AssetIntelPairRule> =
                match rules_by_provider.get(provider) {
                    Some(rules) if !rules.is_empty() => rules
                        .iter()
                        .map(|rule| {
                            single_record_rule(rule.host_field.clone(), rule.ip_field.clone())
                        })
                        .collect(),
                    _ => vec![single_record_rule(
                        DEFAULT_HOST_FIELDS.iter().map(|s| s.to_string()).collect(),
                        DEFAULT_IP_FIELDS.iter().map(|s| s.to_string()).collect(),
                    )],
                };
            for rule in &record_rules {
                for pair in extract_host_ip_pairs(raw, rule) {
                    let Some(host) = normalize_concrete_landing_host(&pair.host) else {
                        continue;
                    };
                    let Ok(ip) = pair.ip.trim().parse::<IpAddr>() else {
                        continue;
                    };
                    let ip = ip.to_string();
                    if seen.insert((host.clone(), ip.clone())) {
                        out.push(HostIpPair { host, ip });
                    }
                }
            }
        }
    }
    out
}

fn normalize_landing_identity(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_end_matches('.');
    if let Ok(ip) = trimmed.parse::<IpAddr>() {
        return Some(ip.to_string());
    }
    normalized_host(trimmed)
}

pub(crate) fn normalize_concrete_landing_host(value: &str) -> Option<String> {
    let host = normalize_landing_identity(value)?;
    (!host.starts_with("*.") && host.parse::<IpAddr>().is_err()).then_some(host)
}

fn is_ip_literal(value: &str) -> bool {
    normalize_landing_identity(value).is_some_and(|identity| identity.parse::<IpAddr>().is_ok())
}

fn target_accepts_real_ip(target_type: &str, value: &str) -> bool {
    !matches!(target_type, "ip" | "ipv4" | "ip_address" | "cidr") && !is_ip_literal(value)
}

fn dedupe_landing_hosts<I>(hosts: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for host in hosts {
        let Some(host) = normalize_concrete_landing_host(&host) else {
            continue;
        };
        if seen.insert(host.clone()) {
            out.push(host);
        }
    }
    out
}

fn canonical_ip(raw: &str) -> Option<String> {
    raw.trim().parse::<IpAddr>().ok().map(|ip| ip.to_string())
}

fn deterministic_primary_ip<I>(ips: I) -> Option<String>
where
    I: IntoIterator<Item = String>,
{
    let mut ips: Vec<IpAddr> = ips
        .into_iter()
        .filter_map(|ip| ip.parse::<IpAddr>().ok())
        .collect();
    ips.sort_by(|left, right| match (left, right) {
        (IpAddr::V4(_), IpAddr::V6(_)) => std::cmp::Ordering::Less,
        (IpAddr::V6(_), IpAddr::V4(_)) => std::cmp::Ordering::Greater,
        _ => left.to_string().cmp(&right.to_string()),
    });
    ips.dedup();
    ips.first().map(ToString::to_string)
}

/// One canonical Target identity planned from a single provider invocation.
/// This is an execution handoff, not the legacy durable candidate-review row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CurrentRunTarget {
    pub value: String,
    pub target_type: &'static str,
    pub real_ip: Option<String>,
}

/// Turn only the current provider invocation's normalized observations into a
/// deterministic domain/IP Target plan. No organization profile or historical
/// candidate queue is accepted as input, so an old observation cannot become
/// fresh merely because another survey ran.
#[allow(dead_code)]
pub(crate) fn plan_current_run_targets(
    candidates: &OrganizationCandidates,
    observed_domain_hosts: &[String],
    pairs: &[HostIpPair],
) -> Vec<CurrentRunTarget> {
    plan_current_run_targets_with_ip_policy(candidates, observed_domain_hosts, pairs, true)
}

pub(crate) fn plan_current_run_targets_with_ip_policy(
    candidates: &OrganizationCandidates,
    observed_domain_hosts: &[String],
    pairs: &[HostIpPair],
    promote_ip_targets: bool,
) -> Vec<CurrentRunTarget> {
    let mut domains: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut ips = BTreeSet::new();

    {
        let mut add_identity = |raw: &str| match normalize_landing_identity(raw) {
            Some(identity) if promote_ip_targets && identity.parse::<IpAddr>().is_ok() => {
                ips.insert(identity);
            }
            Some(identity) if identity.parse::<IpAddr>().is_ok() => {}
            Some(identity) if !identity.starts_with("*.") => {
                domains.entry(identity).or_default();
            }
            _ => {}
        };

        for candidate in &candidates.targets {
            add_identity(&candidate.value);
        }
        for host in observed_domain_hosts {
            add_identity(host);
        }
    }

    for pair in pairs {
        let Some(host) = normalize_concrete_landing_host(&pair.host) else {
            continue;
        };
        let Some(ip) = canonical_ip(&pair.ip) else {
            continue;
        };
        let resolved = domains.entry(host).or_default();
        if !resolved.contains(&ip) {
            resolved.push(ip.clone());
        }
        if promote_ip_targets {
            ips.insert(ip);
        }
    }

    let mut planned = domains
        .into_iter()
        .map(|(value, resolved)| CurrentRunTarget {
            value,
            target_type: "domain",
            real_ip: deterministic_primary_ip(resolved),
        })
        .collect::<Vec<_>>();
    planned.extend(ips.into_iter().map(|value| CurrentRunTarget {
        value,
        target_type: "ip",
        real_ip: None,
    }));
    planned
}

fn json_atom_strings(value: &Value) -> Vec<String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                Vec::new()
            } else {
                vec![trimmed.to_string()]
            }
        }
        Value::Array(items) => items.iter().flat_map(json_atom_strings).collect(),
        Value::Object(map) => map.values().flat_map(json_atom_strings).collect(),
        _ => Vec::new(),
    }
}

/// Decide which discovered assets belong to `org` and should be upserted as
/// targets: owned exact hosts keep a deterministic primary `real_ip` cache;
/// provider pair IPs — whether carried as the pair value or as an IP-literal host
/// — remain relationship evidence and do not become executable targets. The
/// organization profile is metadata, not an authorization source: explicit
/// IP/CIDR scope must already exist as a trusted target row from scoping/CLI.
/// Third-party hosts (shared-tenant rDNS like `*.163data.com.cn`) are dropped via
/// `value_belongs_to_organization`. Pure.
#[allow(dead_code)]
pub(crate) fn plan_promotable_assets(
    org: &golish_db::models::Organization,
    pairs: &[HostIpPair],
) -> Vec<(String, Option<String>)> {
    let mut domains: Vec<(String, Option<String>)> = Vec::new();
    let mut host_order = Vec::new();
    let mut resolved_by_host: HashMap<String, Vec<String>> = HashMap::new();
    for pair in pairs {
        let Some(host) = normalize_concrete_landing_host(&pair.host) else {
            continue;
        };
        let Some(ip) = canonical_ip(&pair.ip) else {
            continue;
        };
        if !value_belongs_to_organization(org, &host) {
            continue;
        }
        let entry = resolved_by_host.entry(host.clone()).or_insert_with(|| {
            host_order.push(host.clone());
            Vec::new()
        });
        if !entry.contains(&ip) {
            entry.push(ip);
        }
    }
    for host in host_order {
        let primary = resolved_by_host
            .remove(&host)
            .and_then(deterministic_primary_ip);
        domains.push((host, primary));
    }
    domains
}

/// Merge provider-discovered hostnames into the pair-backed promotion plan.
/// A concrete owned hostname is an asset identity even when the provider did not
/// return A/AAAA data; it becomes a domain target with `real_ip=None`. Valid
/// host/IP pairs still supply the deterministic cache and all DNS relationships.
/// Wildcard apexes, third-party hosts, IP literals and malformed names remain
/// excluded by the same organization-scope predicate.
#[allow(dead_code)]
pub(crate) fn plan_promotable_assets_with_hosts(
    org: &golish_db::models::Organization,
    pairs: &[HostIpPair],
    discovered_hosts: &[String],
) -> Vec<(String, Option<String>)> {
    let mut planned = plan_promotable_assets(org, pairs);
    let mut seen: HashSet<String> = planned.iter().map(|(host, _)| host.clone()).collect();
    for raw in discovered_hosts {
        let Some(host) = normalize_concrete_landing_host(raw) else {
            continue;
        };
        if !value_belongs_to_organization(org, &host) || !seen.insert(host.clone()) {
            continue;
        }
        planned.push((host, None));
    }
    planned
}

/// Extract candidate hostnames from the org's `certificates` JSON (CT coverage),
/// **shape-agnostically**: walk every string leaf (`json_atom_strings`), pull
/// host-like tokens out of each (handles cert-subject DNs like `CN=*.pingan.com`,
/// strips `*.` wildcards to their parent, drops IPs and non-host tokens), dedupe.
/// The caller scope-filters via `value_belongs_to_organization` before
/// materialising (design 2026-06-24-intel-to-eas-handoff §4 L0b). Pure.
#[allow(dead_code)]
pub(crate) fn hostnames_from_certificates(certificates: &Value) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for leaf in json_atom_strings(certificates) {
        for host in hostnames_in_text(&leaf) {
            if seen.insert(host.clone()) {
                out.push(host);
            }
        }
    }
    dedupe_landing_hosts(out)
}

/// Pull dotted-hostname tokens out of free text (a cert subject DN, SAN list,
/// etc.). Splits on non-host chars, strips a leading `*.` wildcard, lowercases,
/// keeps only well-formed hostnames (≥2 labels, alpha TLD → IPs excluded).
fn hostnames_in_text(text: &str) -> Vec<String> {
    text.split(|c: char| {
        !(c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '*' || c == '_')
    })
    .filter_map(|raw| {
        let tok = raw
            .trim()
            .trim_start_matches("*.")
            .trim_matches('.')
            .to_ascii_lowercase();
        if is_hostname_like(&tok) {
            Some(tok)
        } else {
            None
        }
    })
    .collect()
}

/// A token is hostname-like when it has ≥2 dot-separated labels, each label is a
/// valid DNS label, and the TLD is alphabetic (so dotted IPs like `1.2.3.4` are
/// rejected — their numeric TLD fails).
fn is_hostname_like(s: &str) -> bool {
    if s.len() < 4 || s.len() > 253 || !s.contains('.') {
        return false;
    }
    let labels: Vec<&str> = s.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    if labels.iter().any(|l| {
        l.is_empty()
            || l.len() > 63
            || l.starts_with('-')
            || l.ends_with('-')
            || !l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    }) {
        return false;
    }
    let tld = labels.last().expect("len>=2 checked");
    tld.len() >= 2 && tld.bytes().all(|b| b.is_ascii_alphabetic())
}

/// Serialize Intel writers for one exact stored Target identity. `targets` has
/// no database uniqueness constraint beyond its UUID primary key, while Stage
/// Team may run several provider producers concurrently. The transaction lock
/// closes the SELECT-then-INSERT race without changing the schema. It is keyed
/// by project + exact target type + exact canonical value (not apex or IP), so
/// sibling vhosts remain independent assets. Organization is deliberately not
/// part of the key: a legacy `organization_id=NULL` row can be claimed by only
/// one org before another org decides whether it needs its own row.
fn target_identity_lock_key(project_path: &str, target_type: &str, value: &str) -> String {
    serde_json::json!(["asset_intel_target_v1", project_path, target_type, value]).to_string()
}

fn build_target_identity_lock_sql() -> &'static str {
    "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))"
}

fn build_target_real_ip_update_sql() -> &'static str {
    r#"UPDATE targets
       SET real_ip = $1, updated_at = NOW()
       WHERE id = $2
         AND target_type::text NOT IN ('ip', 'ipv4', 'ip_address', 'cidr')"#
}

/// Upsert one target, idempotent on the storage identity
/// `(project_path, organization, target_type, canonical value)`, tagging the
/// surveyed `real_ip` when known. An unowned legacy row in the same project may
/// first be claimed by the organization. Returns the target's id so the caller
/// can directly land its provider-paired DNS record (design 2026-06-23).
async fn upsert_target(
    pool: &sqlx::PgPool,
    org: &golish_db::models::Organization,
    value: &str,
    target_type: &str,
    real_ip: Option<&str>,
) -> Result<Uuid, GolishError> {
    let mut tx = pool.begin().await?;
    sqlx::query(build_target_identity_lock_sql())
        .bind(target_identity_lock_key(
            &org.project_path,
            target_type,
            value,
        ))
        .execute(&mut *tx)
        .await?;

    let existing: Option<(Uuid, String)> = sqlx::query_as(build_target_lookup_sql())
        .bind(value)
        .bind(target_type)
        .bind(&org.project_path)
        .bind(org.id)
        .fetch_optional(&mut *tx)
        .await?;

    if let Some((id, existing_target_type)) = existing {
        let claimed = sqlx::query(build_target_claim_sql())
            .bind(id)
            .bind(org.id)
            .execute(&mut *tx)
            .await?;
        if claimed.rows_affected() == 0 {
            return Err(GolishError::Validation(format!(
                "target ownership changed while claiming {target_type}:{value}"
            )));
        }
        if target_accepts_real_ip(&existing_target_type, value) {
            if let Some(ip) = real_ip.map(str::trim).filter(|ip| !ip.is_empty()) {
                sqlx::query(build_target_real_ip_update_sql())
                    .bind(ip)
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        tx.commit().await?;
        return Ok(id);
    }

    let landed_real_ip = if target_accepts_real_ip(target_type, value) {
        real_ip.unwrap_or("").trim()
    } else {
        ""
    };
    let new_id: Uuid = sqlx::query_scalar(build_target_insert_sql())
        .bind(value)
        .bind(target_type)
        .bind(value)
        .bind(org.id)
        .bind(&org.project_path)
        .bind(landed_real_ip)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(new_id)
}

fn build_target_lookup_sql() -> &'static str {
    r#"SELECT id, target_type::text FROM targets
       WHERE value = $1
         AND target_type::text = $2
         AND project_path IS NOT DISTINCT FROM $3
         AND (organization_id = $4 OR organization_id IS NULL)
       ORDER BY (organization_id = $4) DESC NULLS LAST, updated_at DESC
       LIMIT 1"#
}

fn build_target_claim_sql() -> &'static str {
    r#"UPDATE targets
       SET organization_id = $2,
           updated_at = NOW()
       WHERE id = $1
         AND (organization_id = $2 OR organization_id IS NULL)"#
}

fn build_target_insert_sql() -> &'static str {
    r#"INSERT INTO targets
          (name, target_type, value, tags, notes, scope, grp, owner,
           organization_id, project_path, source, parent_id, real_ip)
       VALUES
          ($1, $2::target_type, $3, '[]', '', 'in'::scope_type, 'default', '',
           $4, $5, 'asset_intel', NULL, $6)
       RETURNING id"#
}

fn build_service_target_lookup_sql() -> &'static str {
    r#"SELECT id FROM targets
       WHERE value = $1
         AND target_type::text = $2
         AND organization_id = $3
         AND project_path IS NOT DISTINCT FROM $4
         AND scope::text = 'in'
       ORDER BY updated_at DESC
       LIMIT 1"#
}

/// Classify a provider-paired `(host, ip)` into a DNS record tuple
/// `(record_type, name, value)` for direct landing into `dns_records` (design
/// 2026-06-23). Returns `None` for an empty host or an unparseable IP — so junk
/// never lands. IPv4 → `"A"`, IPv6 → `"AAAA"`.
fn provider_dns_record(host: &str, ip: &str) -> Option<(&'static str, String, String)> {
    let host = normalize_concrete_landing_host(host)?;
    let parsed: IpAddr = ip.trim().parse().ok()?;
    let record_type = if parsed.is_ipv4() { "A" } else { "AAAA" };
    Some((record_type, host, parsed.to_string()))
}

fn provider_dns_records_for_pairs(
    org: &golish_db::models::Organization,
    pairs: &[HostIpPair],
) -> Vec<(&'static str, String, String)> {
    let mut seen = HashSet::new();
    pairs
        .iter()
        .filter_map(|pair| provider_dns_record(&pair.host, &pair.ip))
        .filter(|(_, name, value)| {
            !name.starts_with("*.")
                && value_belongs_to_organization(org, name)
                && seen.insert((name.clone(), value.clone()))
        })
        .collect()
}

/// Business-table counts written by one current-run asset-map landing pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TargetLandingSummary {
    pub targets: usize,
    pub domains: usize,
    pub ips: usize,
    pub dns_records: usize,
}

/// Promote this invocation's normalized provider observations directly into
/// org-bound Targets, then persist every exact hostname/IP edge. The transient
/// candidate DTO is only an adapter input; no approval queue participates.
pub(crate) async fn land_current_run_targets(
    pool: &sqlx::PgPool,
    org: &golish_db::models::Organization,
    candidates: &OrganizationCandidates,
    pairs: &[HostIpPair],
    discovered_hosts: &[String],
    promote_ip_targets: bool,
) -> Result<TargetLandingSummary, GolishError> {
    let planned = plan_current_run_targets_with_ip_policy(
        candidates,
        discovered_hosts,
        pairs,
        promote_ip_targets,
    );
    let mut summary = TargetLandingSummary::default();
    let mut domain_target_ids = HashMap::new();
    for target in &planned {
        let target_id = upsert_target(
            pool,
            org,
            &target.value,
            target.target_type,
            target.real_ip.as_deref(),
        )
        .await?;
        summary.targets += 1;
        match target.target_type {
            "domain" => {
                summary.domains += 1;
                domain_target_ids.insert(target.value.clone(), target_id);
            }
            "ip" => summary.ips += 1,
            _ => {}
        }
    }

    // Reuse the hardened DNS normalizer/filter, but scope it to the current
    // invocation's concrete domain plan rather than the cumulative org profile.
    let mut current_run_org = org.clone();
    current_run_org.domains = serde_json::json!(planned
        .iter()
        .filter(|target| target.target_type == "domain")
        .map(|target| target.value.clone())
        .collect::<Vec<_>>());
    if let Some(intel) = current_run_org.intel.as_object_mut() {
        intel.remove("app_domains");
    }
    // Every provider-observed A/AAAA pair is relationship truth. Keep all of
    // them on the exact hostname target; `real_ip` above is only one stable
    // cache. Whether a pair IP is also promoted to an explicit Target is the
    // caller's policy (`promote_ip_targets`); domain-pivot passes keep it only
    // as relationship evidence.
    for (record_type, name, value) in provider_dns_records_for_pairs(&current_run_org, pairs) {
        let Some(target_id) = domain_target_ids.get(&name).copied() else {
            continue;
        };
        golish_db::repo::dns_records::upsert(
            pool,
            target_id,
            org.project_path.as_str(),
            record_type,
            &name,
            &value,
            "provider",
        )
        .await?;
        summary.dns_records += 1;
    }
    Ok(summary)
}

/// A per-host service observed by a provider survey (port + optional
/// protocol/service/version), lifted out of a candidate's `evidence.raw`.
/// Lands as `target_assets(asset_type='service')` so the cyberspace-mapping
/// port/service intel (quake/fofa/0.zone) becomes queryable instead of dying in
/// raw JSON — providers return open ports + service banners per host, but the
/// landing previously only wrote the bare subdomain and dropped all of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostServiceAsset {
    pub host: String,
    pub port: i32,
    pub protocol: Option<String>,
    pub service: Option<String>,
    pub version: Option<String>,
}

/// Host-owner fields tried (priority order) when lifting a service from a
/// provider record — mirrors survey record shapes (quake/fofa/0.zone flat
/// `domain`/`host`, quake nested `service.http.host`, shodan `ip_str`). Prefer
/// the most concrete HTTP/host identity over a registrable `domain`; Quake
/// `hostname` can be PTR/rDNS noise and stays a late fallback.
const SERVICE_HOST_FIELDS: &[&str] = &[
    "service.http.host",
    "host",
    "domain",
    "hostname",
    "ip",
    "ip_str",
];
const SERVICE_PORT_FIELDS: &[&str] = &["port", "service.port"];
const SERVICE_PROTOCOL_FIELDS: &[&str] = &["transport", "protocol", "service.transport"];
const SERVICE_NAME_FIELDS: &[&str] = &[
    "service.name",
    "service.http.server",
    "webserver",
    "service",
];
const SERVICE_VERSION_FIELDS: &[&str] = &["service.version", "version"];

fn record_field(record: &Value, fields: &[&str]) -> Option<String> {
    fields.iter().find_map(|field| {
        resolve_field_ref(
            record,
            &golish_pentest::models::AssetIntelFieldRef::Field((*field).to_string()),
        )
    })
}

/// Lift a single `(host, port, protocol, service, version)` service from one
/// provider record. Returns `None` unless BOTH an owner host/IP and a valid
/// port (1..=65535) resolve — a service without a port is not a service row.
/// Pure.
fn service_asset_from_record(record: &Value) -> Option<HostServiceAsset> {
    let host = record_field(record, SERVICE_HOST_FIELDS)
        .and_then(|value| normalize_landing_identity(&value))
        .filter(|host| !host.starts_with("*."))?;
    let port = record_field(record, SERVICE_PORT_FIELDS)
        .and_then(|raw| raw.trim().parse::<i32>().ok())
        .filter(|port| (1..=65535).contains(port))?;
    let protocol = record_field(record, SERVICE_PROTOCOL_FIELDS).map(|p| p.to_ascii_lowercase());
    let service = record_field(record, SERVICE_NAME_FIELDS);
    let version = record_field(record, SERVICE_VERSION_FIELDS);
    Some(HostServiceAsset {
        host,
        port,
        protocol,
        service,
        version,
    })
}

/// Lift per-host service assets out of the in-memory survey candidates (each
/// keeps the full provider record in `evidence.raw`). Deduped by
/// `(host, port, protocol)`. Pure — no DB.
pub(crate) fn service_assets_from_candidates(
    candidates: &OrganizationCandidates,
) -> Vec<HostServiceAsset> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for candidate in &candidates.targets {
        let sources = candidate
            .evidence
            .get("sources")
            .and_then(Value::as_array)
            .filter(|sources| !sources.is_empty());
        let evidence_records: Vec<&Value> = match sources {
            Some(sources) => sources.iter().collect(),
            None => vec![&candidate.evidence],
        };
        for evidence in evidence_records {
            let Some(raw) = evidence.get("raw") else {
                continue;
            };
            let Some(asset) = service_asset_from_record(raw) else {
                continue;
            };
            if seen.insert((asset.host.clone(), asset.port, asset.protocol.clone())) {
                out.push(asset);
            }
        }
    }
    out
}

/// Persist provider-surveyed services as `target_assets(asset_type='service')`
/// children of each owner's existing target (resolved after host promotion).
/// `value` is `"<port>/<protocol>"` so the unique `(target_id, asset_type,
/// value)` key dedupes per port/proto while `upsert` COALESCE-fills
/// port/protocol/service/version. Scope-filtered (owned host or IP) and fully
/// non-fatal — a miss only warns, never rolls back the committed enrich. Returns
/// how many service rows landed.
pub(crate) async fn land_service_assets(
    pool: &sqlx::PgPool,
    org: &golish_db::models::Organization,
    assets: &[HostServiceAsset],
) -> Result<usize, GolishError> {
    if assets.is_empty() {
        return Ok(0);
    }
    let metadata = serde_json::json!({ "source": "asset_intel" });
    let mut target_cache: HashMap<String, Option<Uuid>> = HashMap::new();
    let mut landed = 0usize;
    for asset in assets {
        if !value_belongs_to_organization(org, &asset.host) {
            continue;
        }
        let target_id = match target_cache.get(&asset.host) {
            Some(cached) => *cached,
            None => {
                let resolved: Option<Uuid> = sqlx::query_scalar(build_service_target_lookup_sql())
                    .bind(&asset.host)
                    .bind(if is_ip_literal(&asset.host) {
                        "ip"
                    } else {
                        "domain"
                    })
                    .bind(org.id)
                    .bind(&org.project_path)
                    .fetch_optional(pool)
                    .await?;
                target_cache.insert(asset.host.clone(), resolved);
                resolved
            }
        };
        let Some(target_id) = target_id else {
            continue;
        };
        let protocol = asset.protocol.as_deref().unwrap_or("tcp");
        let value = format!("{}/{}", asset.port, protocol);
        golish_db::repo::target_assets::upsert(
            pool,
            target_id,
            Some(org.project_path.as_str()),
            "service",
            &value,
            Some(asset.port),
            Some(protocol),
            asset.service.as_deref(),
            asset.version.as_deref(),
            &metadata,
        )
        .await?;
        landed += 1;
    }
    Ok(landed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::organizations::{OrganizationCandidate, OrganizationCandidateKind};

    fn org_with_domains(domains: Value) -> golish_db::models::Organization {
        serde_json::from_value(serde_json::json!({
            "id": "00000000-0000-0000-0000-000000000001",
            "project_path": "/tmp/passive-intel-test",
            "name": "Test Org",
            "parent_id": null,
            "description": "",
            "owner": "",
            "sort_order": 0,
            "domains": domains,
            "created_at": "2026-06-17T00:00:00Z",
            "updated_at": "2026-06-17T00:00:00Z"
        }))
        .expect("construct test organization")
    }

    fn target_candidate(source: &str, raw: Value) -> OrganizationCandidate {
        OrganizationCandidate {
            id: format!("target:{source}"),
            kind: OrganizationCandidateKind::Target,
            label: "t".into(),
            value: "t".into(),
            organization_id: None,
            ownership_percent: None,
            source: source.into(),
            confidence: 0.7,
            status: "needs_review".into(),
            evidence: serde_json::json!({ "provider": source, "raw": raw }),
            created_at: 1,
        }
    }

    fn target_candidate_value(value: &str) -> OrganizationCandidate {
        OrganizationCandidate {
            id: format!("target:test:{value}"),
            kind: OrganizationCandidateKind::Target,
            label: value.to_string(),
            value: value.to_string(),
            organization_id: None,
            ownership_percent: None,
            source: "test".to_string(),
            confidence: 0.9,
            status: "needs_review".to_string(),
            evidence: serde_json::json!({"provider": "test", "raw": {"value": value}}),
            created_at: 1,
        }
    }

    #[test]
    fn plan_current_run_targets_promotes_domains_and_ips_without_authorized_roots() {
        let candidates = OrganizationCandidates {
            targets: vec![
                target_candidate_value("Api.MoreSec.CN."),
                target_candidate_value("https://Portal.MoreSec.CN/login"),
                target_candidate_value("203.0.113.30"),
                target_candidate_value("api.moresec.cn"),
            ],
            ..Default::default()
        };
        let observed_domain_hosts = vec![
            "portal.moresec.cn".to_string(),
            "Shop.MoreSec.CN.".to_string(),
        ];
        let pairs = vec![
            HostIpPair {
                host: "api.moresec.cn".to_string(),
                ip: "2001:db8::2".to_string(),
            },
            HostIpPair {
                host: "API.MoreSec.CN.".to_string(),
                ip: "203.0.113.20".to_string(),
            },
        ];

        let planned = plan_current_run_targets(&candidates, &observed_domain_hosts, &pairs);

        assert_eq!(
            planned
                .iter()
                .filter(|target| target.target_type == "domain")
                .map(|target| target.value.as_str())
                .collect::<HashSet<_>>(),
            HashSet::from(["api.moresec.cn", "portal.moresec.cn", "shop.moresec.cn"])
        );
        assert_eq!(
            planned
                .iter()
                .filter(|target| target.target_type == "ip")
                .map(|target| target.value.as_str())
                .collect::<HashSet<_>>(),
            HashSet::from(["203.0.113.20", "203.0.113.30", "2001:db8::2"])
        );
        assert_eq!(
            planned
                .iter()
                .find(|target| target.value == "api.moresec.cn")
                .and_then(|target| target.real_ip.as_deref()),
            Some("203.0.113.20"),
            "domain primary cache must be deterministic while every pair IP remains a target"
        );
    }

    #[test]
    fn plan_current_run_targets_rejects_wildcard_and_malformed_values() {
        let candidates = OrganizationCandidates {
            targets: vec![
                target_candidate_value("*.moresec.cn"),
                target_candidate_value("not-a-host"),
                target_candidate_value("203.0.113.0/24"),
                target_candidate_value("valid.moresec.cn"),
            ],
            ..Default::default()
        };

        let planned = plan_current_run_targets(&candidates, &[], &[]);

        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].target_type, "domain");
        assert_eq!(planned[0].value, "valid.moresec.cn");
    }

    #[test]
    fn domain_pivot_keeps_distinct_vhosts_without_promoting_relation_ips() {
        let pairs = vec![
            HostIpPair {
                host: "app.example.com".to_string(),
                ip: "203.0.113.10".to_string(),
            },
            HostIpPair {
                host: "admin.example.com".to_string(),
                ip: "203.0.113.10".to_string(),
            },
        ];

        let planned = plan_current_run_targets_with_ip_policy(
            &OrganizationCandidates::default(),
            &[],
            &pairs,
            false,
        );

        assert_eq!(
            planned
                .iter()
                .map(|target| (target.target_type, target.value.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("domain", "admin.example.com"),
                ("domain", "app.example.com"),
            ],
            "same-IP vhosts remain separate assets while the observed IP stays a DNS/service relation"
        );
        assert!(planned
            .iter()
            .all(|target| target.real_ip.as_deref() == Some("203.0.113.10")));
    }

    #[test]
    fn provider_dns_record_classifies_a_aaaa_and_rejects_garbage() {
        // Phase A (design 2026-06-23): provider host↔IP → direct DNS record.
        assert_eq!(
            provider_dns_record("bank.pingan.com", "1.2.3.4"),
            Some(("A", "bank.pingan.com".to_string(), "1.2.3.4".to_string()))
        );
        assert_eq!(
            provider_dns_record("x.com", "2400:cb00::1").map(|t| t.0),
            Some("AAAA")
        );
        // junk IP / empty host never lands.
        assert_eq!(provider_dns_record("x.com", "not-an-ip"), None);
        assert_eq!(provider_dns_record("", "1.2.3.4"), None);
        assert_eq!(provider_dns_record("116.62.45.225", "1.94.38.88"), None);
        // trims whitespace.
        assert_eq!(
            provider_dns_record("  a.com  ", " 9.9.9.9 "),
            Some(("A", "a.com".to_string(), "9.9.9.9".to_string()))
        );
    }

    #[test]
    fn provider_dns_plan_keeps_all_multi_address_edges_without_authorizing_ips() {
        let org = org_with_domains(serde_json::json!(["moresec.cn"]));
        let pairs = vec![
            HostIpPair {
                host: "WWW.MoreSec.CN.".into(),
                ip: "203.0.113.10".into(),
            },
            HostIpPair {
                host: "www.moresec.cn".into(),
                ip: "203.0.113.11".into(),
            },
            HostIpPair {
                host: "www.moresec.cn".into(),
                ip: "2001:db8::1".into(),
            },
        ];

        let records = provider_dns_records_for_pairs(&org, &pairs);
        assert_eq!(records.len(), 3);
        assert_eq!(records.iter().filter(|(ty, _, _)| *ty == "A").count(), 2);
        assert_eq!(records.iter().filter(|(ty, _, _)| *ty == "AAAA").count(), 1);
        assert!(records.iter().all(|(_, host, _)| host == "www.moresec.cn"));
        let domains = plan_promotable_assets(&org, &pairs);
        assert_eq!(domains.len(), 1);
    }

    #[test]
    fn provider_and_profile_ips_never_become_active_targets_in_intel_landing() {
        let mut org = org_with_domains(serde_json::json!(["moresec.cn"]));
        org.ip_ranges = serde_json::json!(["203.0.113.7", "203.0.113.0/24"]);
        let pairs = vec![HostIpPair {
            host: "116.62.45.225".into(),
            ip: "1.94.38.88".into(),
        }];

        let domains = plan_promotable_assets(&org, &pairs);
        assert!(
            domains.is_empty(),
            "provider IP observations and profile ip_ranges are metadata, not scan authorization"
        );
    }

    #[test]
    fn plan_promotable_assets_keeps_owned_pairs_drops_thirdparty_and_bad_ip() {
        let org = org_with_domains(serde_json::json!(["pingan.com"]));
        let pairs = vec![
            HostIpPair {
                host: "bank.pingan.com".into(),
                ip: "221.11.190.218".into(),
            },
            // Third-party shared-tenant rDNS — not an owned domain → dropped.
            HostIpPair {
                host: "194.1.broad.ha.dynamic.163data.com.cn".into(),
                ip: "61.241.22.62".into(),
            },
            // Provider IP-only observations are evidence, not scan authorization.
            HostIpPair {
                host: "116.62.45.225".into(),
                ip: "1.94.38.88".into(),
            },
            // Invalid pair IP is ignored instead of poisoning target.real_ip.
            HostIpPair {
                host: "api.pingan.com".into(),
                ip: "not-an-ip".into(),
            },
        ];
        let domains = plan_promotable_assets(&org, &pairs);
        assert_eq!(
            domains,
            vec![(
                "bank.pingan.com".to_string(),
                Some("221.11.190.218".to_string())
            )]
        );
    }

    #[test]
    fn plan_promotable_assets_preserves_www_and_apex_as_distinct_hosts() {
        let org = org_with_domains(serde_json::json!(["moresec.cn"]));
        let pairs = vec![
            HostIpPair {
                host: "www.moresec.cn".into(),
                ip: "115.28.135.55".into(),
            },
            HostIpPair {
                host: "moresec.cn".into(),
                ip: "115.28.135.55".into(),
            },
            HostIpPair {
                host: "m.moresec.cn".into(),
                ip: "115.28.135.55".into(),
            },
            HostIpPair {
                host: "www.moresec.cn".into(),
                ip: "203.0.113.10".into(),
            },
        ];

        let domains = plan_promotable_assets(&org, &pairs);
        assert_eq!(
            domains,
            vec![
                (
                    "www.moresec.cn".to_string(),
                    Some("115.28.135.55".to_string())
                ),
                ("moresec.cn".to_string(), Some("115.28.135.55".to_string())),
                (
                    "m.moresec.cn".to_string(),
                    Some("115.28.135.55".to_string())
                ),
            ]
        );
    }

    #[test]
    fn host_only_wildcard_discovery_promotes_concrete_child_without_authorizing_ip_or_apex() {
        let org = org_with_domains(serde_json::json!(["*.moresec.cn"]));
        let wildcard_pair = HostIpPair {
            host: "*.moresec.cn".to_string(),
            ip: "203.0.113.8".to_string(),
        };
        let discovered = vec![
            "app.moresec.cn".to_string(),
            "moresec.cn".to_string(),
            "*.moresec.cn".to_string(),
            "*.sub.moresec.cn".to_string(),
            "cdn.vendor.net".to_string(),
            "203.0.113.9".to_string(),
        ];

        assert_eq!(
            plan_promotable_assets_with_hosts(
                &org,
                std::slice::from_ref(&wildcard_pair),
                &discovered,
            ),
            vec![("app.moresec.cn".to_string(), None)]
        );
        assert!(provider_dns_records_for_pairs(&org, &[wildcard_pair]).is_empty());
    }

    #[test]
    fn url_wrapped_pair_and_host_only_candidate_promote_concrete_domain_hosts() {
        let org = org_with_domains(serde_json::json!(["moresec.cn"]));
        let pairs = vec![HostIpPair {
            host: "https://App.MoreSec.CN:8443/path?q=1".to_string(),
            ip: "203.0.113.8".to_string(),
        }];

        assert_eq!(
            plan_promotable_assets_with_hosts(
                &org,
                &pairs,
                &["https://Portal.MoreSec.CN/login".to_string()],
            ),
            vec![
                (
                    "app.moresec.cn".to_string(),
                    Some("203.0.113.8".to_string()),
                ),
                ("portal.moresec.cn".to_string(), None),
            ]
        );
        assert_eq!(
            provider_dns_records_for_pairs(&org, &pairs),
            vec![("A", "app.moresec.cn".to_string(), "203.0.113.8".to_string(),)]
        );
    }

    #[test]
    fn hostnames_from_certificates_extracts_hosts_shape_agnostic() {
        // L0b (design 2026-06-24): pull hosts out of cert-subject strings + nested
        // objects; strip wildcards; drop IPs and non-host DN tokens (O=/C=).
        let certs = serde_json::json!([
            "CN=*.pingan.com",
            "CN=www.pingan.com",
            "CN=bank.pingan.com, O=Ping An, C=CN",
            "mail.pingan.com",
            "1.2.3.4",
            { "subject": "CN=vpn.pingan.com", "san": ["api.pingan.com"] },
        ]);
        let mut v = hostnames_from_certificates(&certs);
        v.sort();
        assert_eq!(
            v,
            vec![
                "api.pingan.com",
                "bank.pingan.com",
                "mail.pingan.com",
                "pingan.com",
                "vpn.pingan.com",
                "www.pingan.com",
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn pairs_from_candidates_extracts_paired_host_ip_with_defaults() {
        let candidates = OrganizationCandidates {
            targets: vec![
                target_candidate(
                    "0.zone",
                    serde_json::json!({"domain":"bank.pingan.com","ip":"221.11.190.218"}),
                ),
                // host empty → skipped (no pair without a host).
                target_candidate("0.zone", serde_json::json!({"domain":"","ip":"1.2.3.4"})),
            ],
            ..Default::default()
        };
        let pairs = pairs_from_candidates(&candidates, &HashMap::new());
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].host, "bank.pingan.com");
        assert_eq!(pairs[0].ip, "221.11.190.218");
    }

    #[test]
    fn pairs_from_candidates_prefer_concrete_host_over_apex_domain() {
        let candidates = OrganizationCandidates {
            targets: vec![target_candidate(
                "fofa",
                serde_json::json!({
                    "host": "https://Api.Example.COM:8443/login",
                    "domain": "example.com",
                    "ip": "203.0.113.10"
                }),
            )],
            ..Default::default()
        };

        let pairs = pairs_from_candidates(&candidates, &HashMap::new());

        assert_eq!(
            pairs,
            vec![HostIpPair {
                host: "api.example.com".to_string(),
                ip: "203.0.113.10".to_string(),
            }],
            "the apex is provenance, not a replacement for the exact observed vhost"
        );
    }

    #[test]
    fn pairs_from_candidates_normalizes_url_field_to_hostname() {
        let candidates = OrganizationCandidates {
            targets: vec![target_candidate(
                "fofa",
                serde_json::json!({
                    "url": "https://App.MoreSec.CN:8443/path?q=1",
                    "ip": "203.0.113.8"
                }),
            )],
            ..Default::default()
        };

        let pairs = pairs_from_candidates(&candidates, &HashMap::new());

        assert_eq!(
            pairs,
            vec![HostIpPair {
                host: "app.moresec.cn".to_string(),
                ip: "203.0.113.8".to_string(),
            }]
        );
    }

    #[test]
    fn pairs_from_candidates_uses_provider_rules_for_nested_fields() {
        let candidates = OrganizationCandidates {
            targets: vec![target_candidate(
                "quake",
                serde_json::json!({"service":{"http":{"host":"www.pingan.com"}},"ip":"61.241.22.62"}),
            )],
            ..Default::default()
        };
        let mut rules = HashMap::new();
        rules.insert(
            "quake".to_string(),
            vec![golish_pentest::models::AssetIntelPairRule {
                path: "$..data[*]".into(),
                host_field: vec!["service.http.host".into()],
                ip_field: vec!["ip".into()],
            }],
        );
        let pairs = pairs_from_candidates(&candidates, &rules);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].host, "www.pingan.com");
        assert_eq!(pairs[0].ip, "61.241.22.62");
    }

    #[test]
    fn pairs_from_candidates_keeps_multiple_ips_for_same_exact_host() {
        let candidates = OrganizationCandidates {
            targets: vec![
                target_candidate(
                    "quake",
                    serde_json::json!({"domain":"www.pingan.com","ip":"203.0.113.11"}),
                ),
                target_candidate(
                    "fofa",
                    serde_json::json!({"domain":"www.pingan.com","ip":"203.0.113.10"}),
                ),
            ],
            ..Default::default()
        };

        let pairs = pairs_from_candidates(&candidates, &HashMap::new());
        assert_eq!(pairs.len(), 2, "host-level first-IP-wins loses DNS truth");
        assert_eq!(pairs[0].host, "www.pingan.com");
        assert_eq!(pairs[1].host, "www.pingan.com");
        assert_ne!(pairs[0].ip, pairs[1].ip);
    }

    #[test]
    fn pairs_from_candidates_keeps_all_pairs_after_cross_provider_merge() {
        let mut candidate = target_candidate(
            "quake",
            serde_json::json!({
                "service": {"http": {"host": "www.pingan.com"}},
                "ip": "203.0.113.11"
            }),
        );
        candidate.evidence["sources"] = serde_json::json!([
            {
                "provider": "quake",
                "raw": {
                    "service": {"http": {"host": "www.pingan.com"}},
                    "ip": "203.0.113.11"
                }
            },
            {
                "provider": "fofa",
                "raw": {"fqdn": "www.pingan.com", "addr": "203.0.113.10"}
            }
        ]);
        let candidates = OrganizationCandidates {
            targets: vec![candidate],
            ..Default::default()
        };
        let rules = HashMap::from([
            (
                "quake".to_string(),
                vec![golish_pentest::models::AssetIntelPairRule {
                    path: "$".into(),
                    host_field: vec!["service.http.host".into()],
                    ip_field: vec!["ip".into()],
                }],
            ),
            (
                "fofa".to_string(),
                vec![golish_pentest::models::AssetIntelPairRule {
                    path: "$".into(),
                    host_field: vec!["fqdn".into()],
                    ip_field: vec!["addr".into()],
                }],
            ),
        ]);

        let pairs = pairs_from_candidates(&candidates, &rules);

        assert_eq!(pairs.len(), 2, "merged evidence must retain both DNS edges");
        assert!(pairs.iter().all(|pair| pair.host == "www.pingan.com"));
        assert_eq!(
            pairs
                .iter()
                .map(|pair| pair.ip.as_str())
                .collect::<HashSet<_>>(),
            HashSet::from(["203.0.113.10", "203.0.113.11"])
        );
    }

    #[test]
    fn plan_promotable_assets_chooses_deterministic_primary_without_authorizing_dns_ips() {
        let org = org_with_domains(serde_json::json!(["moresec.cn"]));
        let pairs = vec![
            HostIpPair {
                host: "www.moresec.cn".into(),
                ip: "2001:db8::2".into(),
            },
            HostIpPair {
                host: "www.moresec.cn".into(),
                ip: "203.0.113.20".into(),
            },
            HostIpPair {
                host: "www.moresec.cn".into(),
                ip: "203.0.113.10".into(),
            },
        ];

        let domains = plan_promotable_assets(&org, &pairs);
        assert_eq!(
            domains,
            vec![(
                "www.moresec.cn".to_string(),
                Some("203.0.113.10".to_string())
            )],
            "primary cache must be IPv4-first and stable, independent of provider order"
        );
    }

    #[test]
    fn target_lookup_is_org_type_and_exact_value_scoped() {
        let sql = build_target_lookup_sql();
        assert!(
            sql.contains("organization_id"),
            "missing org ownership: {sql}"
        );
        assert!(
            sql.contains("target_type::text"),
            "missing type identity: {sql}"
        );
        assert!(
            sql.contains("organization_id IS NULL"),
            "legacy claim path missing: {sql}"
        );
        assert!(
            sql.contains("DESC NULLS LAST"),
            "owned row must beat legacy row: {sql}"
        );
        assert!(
            !sql.contains("value = $1 OR"),
            "lookup must be exact: {sql}"
        );
    }

    #[test]
    fn intel_target_lock_dedupes_only_the_exact_stored_identity() {
        let project = "/tmp/project";
        let api = target_identity_lock_key(project, "domain", "api.example.com");

        assert_eq!(
            api,
            target_identity_lock_key(project, "domain", "api.example.com"),
            "the same exact Target must share one Intel writer lock"
        );
        assert_ne!(
            api,
            target_identity_lock_key(project, "domain", "admin.example.com"),
            "sibling vhosts are different Target assets even when they later resolve to one IP"
        );
        assert_ne!(
            api,
            target_identity_lock_key(project, "ip", "api.example.com"),
            "Target type is part of stored identity"
        );
        assert_ne!(
            api,
            target_identity_lock_key("/tmp/other", "domain", "api.example.com"),
            "project ownership is part of stored identity"
        );

        let sql = build_target_identity_lock_sql();
        assert!(
            sql.contains("pg_advisory_xact_lock"),
            "lock must be transaction-scoped: {sql}"
        );
        assert!(
            sql.contains("hashtextextended"),
            "lock key must be stable inside Postgres: {sql}"
        );
    }

    #[test]
    fn current_run_target_write_contract_is_org_bound_in_scope_and_preserves_scope_out() {
        let insert = build_target_insert_sql();
        assert!(
            insert.contains("organization_id"),
            "missing org bind: {insert}"
        );
        assert!(
            insert.contains("project_path"),
            "missing project bind: {insert}"
        );
        assert!(
            insert.contains("'in'::scope_type"),
            "new targets must enter scope: {insert}"
        );
        assert!(
            insert.contains("'asset_intel'"),
            "missing landing provenance: {insert}"
        );

        let claim = build_target_claim_sql();
        assert!(claim.contains("organization_id = $2"));
        assert!(
            !claim.contains("scope =") && !claim.contains("scope="),
            "provider retry must not reactivate an existing scope=out row: {claim}"
        );
    }

    #[test]
    fn service_lookup_is_org_type_and_exact_value_scoped() {
        let sql = build_service_target_lookup_sql();
        assert!(
            sql.contains("organization_id"),
            "missing org ownership: {sql}"
        );
        assert!(
            sql.contains("target_type::text"),
            "missing type identity: {sql}"
        );
        assert!(
            !sql.contains("value = $1 OR"),
            "www must not fall back to apex: {sql}"
        );
    }

    #[test]
    fn service_assets_extract_port_protocol_service_from_quake_record() {
        // P1 (design 2026-06-26): cyberspace-mapping records carry per-host
        // port/protocol/service; lift them so they land in target_assets columns
        // instead of dying in evidence.raw / org-flat intel arrays.
        let candidates = OrganizationCandidates {
            targets: vec![target_candidate(
                "quake",
                serde_json::json!({
                    "domain": "App.PingAn.com.",
                    "ip": "1.2.3.4",
                    "port": 443,
                    "transport": "TCP",
                    "service": { "name": "http", "http": { "server": "nginx" } }
                }),
            )],
            ..Default::default()
        };
        let assets = service_assets_from_candidates(&candidates);
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].host, "app.pingan.com");
        assert_eq!(assets[0].port, 443);
        assert_eq!(assets[0].protocol.as_deref(), Some("tcp"));
        assert_eq!(assets[0].service.as_deref(), Some("http"));
    }

    #[test]
    fn service_assets_prefer_http_host_over_quake_hostname_noise() {
        let candidates = OrganizationCandidates {
            targets: vec![target_candidate(
                "quake",
                serde_json::json!({
                    "domain": "moresec.cn",
                    "hostname": "mail.bimlmvcg.cfd",
                    "ip": "1.2.3.4",
                    "port": 443,
                    "transport": "tcp",
                    "service": { "http": { "host": "ai-sales.moresec.cn" } }
                }),
            )],
            ..Default::default()
        };
        let assets = service_assets_from_candidates(&candidates);
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].host, "ai-sales.moresec.cn");
    }

    #[test]
    fn service_assets_skip_records_without_a_port() {
        // A host with no port is not a service row (it is already covered by the
        // subdomain / host↔IP landing).
        let candidates = OrganizationCandidates {
            targets: vec![target_candidate(
                "0.zone",
                serde_json::json!({ "domain": "no-port.pingan.com", "ip": "1.2.3.4" }),
            )],
            ..Default::default()
        };
        assert!(service_assets_from_candidates(&candidates).is_empty());
    }

    #[test]
    fn service_assets_dedupe_same_host_port_protocol() {
        let record = serde_json::json!({"domain":"a.pingan.com","port":80,"transport":"tcp"});
        let candidates = OrganizationCandidates {
            targets: vec![
                target_candidate("quake", record.clone()),
                target_candidate("fofa", record),
            ],
            ..Default::default()
        };
        let assets = service_assets_from_candidates(&candidates);
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].port, 80);
        assert!(assets[0].service.is_none());
    }

    #[test]
    fn service_assets_keep_every_port_after_candidate_evidence_merge() {
        let mut candidate = target_candidate(
            "quake",
            serde_json::json!({
                "domain": "api.pingan.com",
                "port": 80,
                "transport": "tcp",
                "service": {"name": "http"}
            }),
        );
        candidate.evidence["sources"] = serde_json::json!([
            {"provider":"quake","raw":{"domain":"api.pingan.com","port":80,"transport":"tcp","service":{"name":"http"}}},
            {"provider":"fofa","raw":{"domain":"api.pingan.com","port":443,"transport":"tcp","service":"https"}},
            {"provider":"shodan","raw":{"domain":"api.pingan.com","port":8443,"transport":"tcp","service":"https-alt"}}
        ]);
        let candidates = OrganizationCandidates {
            targets: vec![candidate],
            ..Default::default()
        };

        let assets = service_assets_from_candidates(&candidates);
        assert_eq!(
            assets.iter().map(|asset| asset.port).collect::<Vec<_>>(),
            vec![80, 443, 8443]
        );
    }
}
