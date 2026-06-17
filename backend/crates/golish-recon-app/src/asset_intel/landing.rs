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

/// Upsert one target, idempotent on `value` + `project_path`, tagging the
/// surveyed `real_ip` when known. Mirrors `persist_target_record` but adds the
/// `real_ip` column and `source='asset_intel'`. Returns `true` if the row
/// already existed.
async fn upsert_target(
    pool: &sqlx::PgPool,
    org: &golish_db::models::Organization,
    value: &str,
    target_type: &str,
    real_ip: Option<&str>,
) -> Result<bool, GolishError> {
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
        return Ok(true);
    }

    sqlx::query(
        r#"INSERT INTO targets
              (name, target_type, value, tags, notes, scope, grp, owner,
               organization_id, project_path, source, parent_id, real_ip)
           VALUES
              ($1, $2::target_type, $3, '[]', '', 'in'::scope_type, 'default', '',
               $4, $5, 'asset_intel', NULL, $6)"#,
    )
    .bind(value)
    .bind(target_type)
    .bind(value)
    .bind(org.id)
    .bind(&org.project_path)
    .bind(real_ip.unwrap_or("").trim())
    .execute(pool)
    .await?;
    Ok(false)
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
            Ok(_) => landed += 1,
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
