//! Phase B of the passive-intel pairing closure (design 2026-06-17): turn
//! survey-discovered, scope-filtered assets into `targets` carrying the
//! surveyed `real_ip`.
//!
//! Pure planning (`plan_promotable_assets`, `pairs_from_candidates`) is
//! unit-tested without a DB; the writes (`promote_profile_assets_to_targets`)
//! are idempotent and non-fatal — a failure only warns and never rolls back the
//! committed enrich (sibling of the `land_*` coverage hooks, invariant D4).

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;

use serde_json::Value;
use uuid::Uuid;

use golish_app_core::GolishError;

use crate::asset_intel::extract_host_ip_pairs;
use crate::asset_intel::types::HostIpPair;
use crate::organization_recon::value_belongs_to_organization;
use crate::organizations::OrganizationCandidates;

/// Host-side fields tried (in priority order) when a provider declares no
/// `normalize.pairs` rule — covers the http_json record shapes (0.zone / quake)
/// so landing still pairs a domain with its surveyed IP.
const DEFAULT_HOST_FIELDS: &[&str] = &["domain", "hostname", "host", "url", "service.http.host"];
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
/// Each target candidate keeps the full provider record in `evidence.raw`; we
/// apply that provider's `normalize.pairs` field lists (or a default set) to the
/// single record and dedupe by host (first IP wins). Pure — no DB.
pub(crate) fn pairs_from_candidates(
    candidates: &OrganizationCandidates,
    rules_by_provider: &HashMap<String, Vec<golish_pentest::models::AssetIntelPairRule>>,
) -> Vec<HostIpPair> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for candidate in &candidates.targets {
        let Some(raw) = candidate.evidence.get("raw") else {
            continue;
        };
        let record_rules: Vec<golish_pentest::models::AssetIntelPairRule> =
            match rules_by_provider.get(&candidate.source) {
                Some(rules) if !rules.is_empty() => rules
                    .iter()
                    .map(|rule| single_record_rule(rule.host_field.clone(), rule.ip_field.clone()))
                    .collect(),
                _ => vec![single_record_rule(
                    DEFAULT_HOST_FIELDS.iter().map(|s| s.to_string()).collect(),
                    DEFAULT_IP_FIELDS.iter().map(|s| s.to_string()).collect(),
                )],
            };
        for rule in &record_rules {
            for pair in extract_host_ip_pairs(raw, rule) {
                if seen.insert(pair.host.clone()) {
                    out.push(pair);
                }
            }
        }
    }
    out
}

/// Read the org profile's `ip_ranges` as plain IP strings. Only bare addresses
/// survive downstream (CIDRs / non-IP atoms are dropped by the parse filter in
/// [`plan_promotable_assets`]).
pub(crate) fn profile_ip_strings(org: &golish_db::models::Organization) -> Vec<String> {
    json_atom_strings(&org.ip_ranges)
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
/// targets: owned hosts keep their surveyed `real_ip`; profile IPs that parse as
/// bare addresses become IP targets. Third-party hosts (shared-tenant rDNS like
/// `*.163data.com.cn`) are dropped via `value_belongs_to_organization`. Pure.
pub(crate) fn plan_promotable_assets(
    org: &golish_db::models::Organization,
    pairs: &[HostIpPair],
    profile_ips: &[String],
) -> (Vec<(String, Option<String>)>, Vec<String>) {
    let mut domains = Vec::new();
    let mut seen_hosts = HashSet::new();
    for pair in pairs {
        if !value_belongs_to_organization(org, &pair.host) {
            continue;
        }
        if seen_hosts.insert(pair.host.clone()) {
            domains.push((pair.host.clone(), Some(pair.ip.clone())));
        }
    }
    let mut seen_ips = HashSet::new();
    let ips = profile_ips
        .iter()
        .filter(|ip| ip.parse::<IpAddr>().is_ok())
        .filter(|ip| seen_ips.insert((*ip).clone()))
        .cloned()
        .collect();
    (domains, ips)
}

/// Keep only well-formed CIDR networks (atoms carrying a `/prefix`) from the
/// org's `ip_ranges`, deduped. Bare IPs are handled by [`plan_promotable_assets`]
/// (they are NOT returned here); non-CIDR / malformed atoms are dropped so junk
/// never becomes a scan target (design 2026-06-24-intel-to-eas-handoff §4 L0a).
/// `target_type='cidr'` is an existing enum value (no schema change).
pub(crate) fn plan_promotable_cidrs(profile_ips: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    profile_ips
        .iter()
        .filter_map(|raw| {
            let s = raw.trim();
            let (addr, prefix) = s.split_once('/')?;
            let ip: IpAddr = addr.trim().parse().ok()?;
            let bits: u8 = prefix.trim().parse().ok()?;
            let max = if ip.is_ipv4() { 32 } else { 128 };
            if bits > max {
                return None;
            }
            Some(s.to_string())
        })
        .filter(|cidr| seen.insert(cidr.clone()))
        .collect()
}

/// Extract candidate hostnames from the org's `certificates` JSON (CT coverage),
/// **shape-agnostically**: walk every string leaf (`json_atom_strings`), pull
/// host-like tokens out of each (handles cert-subject DNs like `CN=*.pingan.com`,
/// strips `*.` wildcards to their parent, drops IPs and non-host tokens), dedupe.
/// The caller scope-filters via `value_belongs_to_organization` before
/// materialising (design 2026-06-24-intel-to-eas-handoff §4 L0b). Pure.
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
    out
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

/// Upsert one target, idempotent on `value` + `project_path`, tagging the
/// surveyed `real_ip` when known. Mirrors `persist_target_record` but adds the
/// `real_ip` column and `source='asset_intel'`. Returns the target's id so the
/// caller can directly land its provider-paired DNS record (design 2026-06-23).
async fn upsert_target(
    pool: &sqlx::PgPool,
    org: &golish_db::models::Organization,
    value: &str,
    target_type: &str,
    real_ip: Option<&str>,
) -> Result<Uuid, GolishError> {
    let existing: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT id FROM targets
           WHERE value = $1
             AND project_path IS NOT DISTINCT FROM $2
           LIMIT 1"#,
    )
    .bind(value)
    .bind(&org.project_path)
    .fetch_optional(pool)
    .await?;

    if let Some(id) = existing {
        sqlx::query(
            r#"UPDATE targets
               SET organization_id = COALESCE(organization_id, $2),
                   updated_at = NOW()
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(org.id)
        .execute(pool)
        .await?;
        if let Some(ip) = real_ip.map(str::trim).filter(|ip| !ip.is_empty()) {
            golish_db::repo::targets::set_real_ip_by_id(pool, id, ip).await?;
        }
        return Ok(id);
    }

    let new_id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO targets
              (name, target_type, value, tags, notes, scope, grp, owner,
               organization_id, project_path, source, parent_id, real_ip)
           VALUES
              ($1, $2::target_type, $3, '[]', '', 'in'::scope_type, 'default', '',
               $4, $5, 'asset_intel', NULL, $6)
           RETURNING id"#,
    )
    .bind(value)
    .bind(target_type)
    .bind(value)
    .bind(org.id)
    .bind(&org.project_path)
    .bind(real_ip.unwrap_or("").trim())
    .fetch_one(pool)
    .await?;
    Ok(new_id)
}

/// Classify a provider-paired `(host, ip)` into a DNS record tuple
/// `(record_type, name, value)` for direct landing into `dns_records` (design
/// 2026-06-23). Returns `None` for an empty host or an unparseable IP — so junk
/// never lands. IPv4 → `"A"`, IPv6 → `"AAAA"`.
fn provider_dns_record(host: &str, ip: &str) -> Option<(&'static str, String, String)> {
    let host = host.trim();
    let ip = ip.trim();
    if host.is_empty() {
        return None;
    }
    let parsed: IpAddr = ip.parse().ok()?;
    let record_type = if parsed.is_ipv4() { "A" } else { "AAAA" };
    Some((record_type, host.to_string(), ip.to_string()))
}

/// Promote scope-filtered survey assets into `targets` with their surveyed
/// `real_ip`. Non-fatal: each failure only warns. Returns how many rows were
/// inserted or updated.
pub(crate) async fn promote_profile_assets_to_targets(
    pool: &sqlx::PgPool,
    org: &golish_db::models::Organization,
    pairs: &[HostIpPair],
) -> usize {
    let profile_ips = profile_ip_strings(org);
    let (domains, ips) = plan_promotable_assets(org, pairs, &profile_ips);
    let mut landed = 0usize;
    for (domain, real_ip) in domains {
        match upsert_target(pool, org, &domain, "domain", real_ip.as_deref()).await {
            Ok(target_id) => {
                landed += 1;
                // Phase A (design 2026-06-23): the provider already paired this
                // host→IP; land it DIRECTLY as a DNS A/AAAA record (the gate-read
                // table) so the DNS coverage cell no longer depends on gate-time
                // live re-resolution. Non-fatal (I9): a failure only warns and
                // never rolls back the committed enrich. `dns_records.upsert` is
                // idempotent (unique key DO NOTHING).
                if let Some(ip) = real_ip.as_deref() {
                    if let Some((rt, name, value)) = provider_dns_record(&domain, ip) {
                        if let Err(error) = golish_db::repo::dns_records::upsert(
                            pool,
                            target_id,
                            org.project_path.as_str(),
                            rt,
                            &name,
                            &value,
                            "provider",
                        )
                        .await
                        {
                            tracing::warn!(
                                %domain, %error,
                                "provider dns_records direct-land failed (non-fatal)"
                            );
                        }
                    }
                }
            }
            Err(error) => {
                tracing::warn!(%domain, %error, "promote domain→target failed (non-fatal)")
            }
        }
    }
    for ip in ips {
        match upsert_target(pool, org, &ip, "ip", None).await {
            Ok(_) => landed += 1,
            Err(error) => tracing::warn!(%ip, %error, "promote ip→target failed (non-fatal)"),
        }
    }
    // L0a (design 2026-06-24): owned CIDR/ASN ranges become VISIBLE `cidr` scope
    // targets so EAS sees them (today they were dropped by the bare-IP parse
    // filter). Active port-scanning a whole netblock stays behind EAS
    // human_approval (D1: "看得见，但不乱炸"). Non-fatal (I9).
    for cidr in plan_promotable_cidrs(&profile_ips) {
        match upsert_target(pool, org, &cidr, "cidr", None).await {
            Ok(_) => landed += 1,
            Err(error) => tracing::warn!(%cidr, %error, "promote cidr→target failed (non-fatal)"),
        }
    }
    // L0b (design 2026-06-24): CT-discovered owned hosts (cert SAN/CN) become
    // `domain` scope targets so EAS can probe them (today they only set the
    // coverage has_ct flag, locked in organizations.certificates JSON). Scope-
    // filtered (drop third-party / non-owned) + idempotent upsert. Non-fatal (I9).
    for host in hostnames_from_certificates(&org.certificates) {
        if !value_belongs_to_organization(org, &host) {
            continue;
        }
        match upsert_target(pool, org, &host, "domain", None).await {
            Ok(_) => landed += 1,
            Err(error) => {
                tracing::warn!(%host, %error, "promote cert host→target failed (non-fatal)")
            }
        }
    }
    landed
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
            source: source.into(),
            confidence: 0.7,
            status: "needs_review".into(),
            evidence: serde_json::json!({ "provider": source, "raw": raw }),
            created_at: 1,
        }
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
        // trims whitespace.
        assert_eq!(
            provider_dns_record("  a.com  ", " 9.9.9.9 "),
            Some(("A", "a.com".to_string(), "9.9.9.9".to_string()))
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
        ];
        let profile_ips = vec!["1.2.3.4".to_string(), "not-an-ip".to_string()];
        let (domains, ips) = plan_promotable_assets(&org, &pairs, &profile_ips);
        assert_eq!(
            domains,
            vec![(
                "bank.pingan.com".to_string(),
                Some("221.11.190.218".to_string())
            )]
        );
        assert_eq!(ips, vec!["1.2.3.4".to_string()]);
    }

    #[test]
    fn plan_promotable_cidrs_keeps_networks_drops_bare_and_garbage() {
        // L0a (design 2026-06-24): only well-formed CIDR networks survive; bare
        // IPs (handled elsewhere), non-CIDR atoms, and over-wide prefixes drop.
        let v = plan_promotable_cidrs(&[
            "203.0.113.0/24".to_string(),
            "10.0.0.0/8".to_string(),
            "1.2.3.4".to_string(),        // bare IP — not a CIDR
            "not-a-net".to_string(),      // garbage
            "1.2.3.4/99".to_string(),     // invalid prefix (>32)
            "203.0.113.0/24".to_string(), // dup — collapsed
            "2001:db8::/32".to_string(),  // IPv6 CIDR
        ]);
        assert_eq!(
            v,
            vec![
                "203.0.113.0/24".to_string(),
                "10.0.0.0/8".to_string(),
                "2001:db8::/32".to_string(),
            ]
        );
    }

    #[test]
    fn hostnames_from_certificates_extracts_hosts_shape_agnostic() {
        // L0b (design 2026-06-24): pull hosts out of cert-subject strings + nested
        // objects; strip wildcards; drop IPs and non-host DN tokens (O=/C=).
        let certs = serde_json::json!([
            "CN=*.pingan.com",
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
}
