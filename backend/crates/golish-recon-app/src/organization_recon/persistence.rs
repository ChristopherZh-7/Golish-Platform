use std::collections::{BTreeSet, HashMap, HashSet};
use std::net::IpAddr;

use golish_app_core::GolishError;
use hickory_resolver::config::{NameServerConfig, ResolverConfig};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::proto::rr::{RData, RecordType};
use hickory_resolver::{Resolver, TokioResolver};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::types::{NormalizedReconRecord, ReconRecordKind};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PersistenceSummary {
    pub record_count: usize,
    pub target_inserted: usize,
    pub target_existing: usize,
    pub profile_updates: usize,
    pub unsupported_records: usize,
    pub record_results: Vec<PersistenceRecordResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PersistenceRecordResult {
    pub record_id: String,
    pub kind: ReconRecordKind,
    pub key: String,
    pub value: String,
    pub status: PersistenceRecordStatus,
    pub action: String,
    pub evidence_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PersistenceRecordStatus {
    Inserted,
    Existing,
    ProfileUpdated,
    Unsupported,
}

impl PersistenceSummary {
    fn push_result(
        &mut self,
        record: &NormalizedReconRecord,
        status: PersistenceRecordStatus,
        action: impl Into<String>,
        target_type: Option<&str>,
        error: Option<String>,
    ) {
        self.record_results.push(PersistenceRecordResult {
            record_id: record.record_id.clone(),
            kind: record.kind.clone(),
            key: record.key.clone(),
            value: record.value.clone(),
            status,
            action: action.into(),
            evidence_count: record.evidence.len(),
            target_type: target_type.map(str::to_string),
            error,
        });
    }
}

pub(crate) async fn persist_normalized_records(
    pool: &sqlx::PgPool,
    organization: &golish_db::models::Organization,
    run_id: &str,
    records: &[NormalizedReconRecord],
    manifest_path: &str,
) -> Result<PersistenceSummary, GolishError> {
    let mut tx = pool.begin().await?;
    let mut summary = PersistenceSummary {
        record_count: records.len(),
        ..PersistenceSummary::default()
    };

    let mut profile = ProfileAccumulator::from_organization(organization);
    for record in records {
        let target_type = if matches!(record.kind, ReconRecordKind::Ip) {
            ip_record_is_authorized(&mut tx, organization, record)
                .await?
                .then_some("ip")
        } else {
            target_type_for_record(organization, record)
        };
        if let Some(target_type) = target_type {
            let existed = persist_target_record(&mut tx, organization, record, target_type).await?;
            if existed {
                summary.target_existing += 1;
                summary.push_result(
                    record,
                    PersistenceRecordStatus::Existing,
                    "target_link_existing",
                    Some(target_type),
                    None,
                );
            } else {
                summary.target_inserted += 1;
                summary.push_result(
                    record,
                    PersistenceRecordStatus::Inserted,
                    "target_insert",
                    Some(target_type),
                    None,
                );
            }
            continue;
        }

        if matches!(record.kind, ReconRecordKind::Ip) {
            summary.unsupported_records += 1;
            summary.push_result(
                record,
                PersistenceRecordStatus::Unsupported,
                "passive_ip_observation_only",
                None,
                Some(
                    "IP observation has no confirmed organization IP/CIDR authorization"
                        .to_string(),
                ),
            );
            continue;
        }

        if profile.merge_record(record) {
            summary.profile_updates += 1;
            summary.push_result(
                record,
                PersistenceRecordStatus::ProfileUpdated,
                "organization_profile_merge",
                None,
                None,
            );
        } else {
            summary.unsupported_records += 1;
            summary.push_result(
                record,
                PersistenceRecordStatus::Unsupported,
                "unsupported_record",
                None,
                Some(format!(
                    "no persistence mapping for {} record",
                    record_kind_label(&record.kind)
                )),
            );
        }
    }

    profile.write(&mut tx, organization.id).await?;
    write_audit(&mut tx, organization, run_id, &summary, manifest_path).await?;
    tx.commit().await?;

    // Coverage-gate landing (design 2026-06-14-target-intel-landing-and-tools §2③,
    // unified in 2026-06-15-db-truth-single-source-deliverable §5 PR1): promote the
    // org-recon `Domain` records into the per-asset / org-level tables the
    // target_intel coverage gate actually reads. Shared with the agent enrich path
    // via `land_target_intel_coverage`. Non-fatal: a landing miss never rolls back
    // the recon persistence already committed above.
    let subdomain_hosts: Vec<String> = records
        .iter()
        .filter(|r| matches!(r.kind, ReconRecordKind::Domain))
        .map(|r| r.value.clone())
        .collect();
    match land_target_intel_coverage(pool, organization, run_id, &subdomain_hosts).await {
        Ok(landed) => tracing::info!(
            organization_id = %organization.id,
            subdomains = landed.subdomains,
            "target_intel coverage landing (org-recon path)"
        ),
        Err(error) => tracing::warn!(
            organization_id = %organization.id,
            %error,
            "target_intel coverage landing failed after org-recon persistence committed"
        ),
    }

    Ok(summary)
}

/// What a single coverage-landing pass wrote into the gate-read tables.
/// Slimmed 2026-06-18 (plan slim-enrich): the enrich / provider-survey landing only
/// owns subdomains (target_assets). DNS stays on the gate-refresh path
/// (`refresh_per_asset_landing`); CT/WHOIS moved to tools (`ctfr` / `recon_lookup_
/// whois`); reverse-DNS / IP-WHOIS dropped.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct CoverageLandingSummary {
    pub subdomains: usize,
}

/// Land one passive-intel collection's results into the business tables the
/// `target_intel` coverage gate reads (design 2026-06-15 §5 PR1). Shared by the GUI
/// org-recon path (`persist_normalized_records`) and the agent enrich path
/// (`asset_intel::run_passive_intel`). Each hook is independent and non-fatal: a
/// failure is logged and skipped, never rolling back already-committed recon data.
/// `subdomain_hosts` are candidate hosts to promote to `target_assets`
/// (org-recon: `Domain` record values; agent path: `organizations.domains`).
pub(crate) async fn land_target_intel_coverage(
    pool: &sqlx::PgPool,
    organization: &golish_db::models::Organization,
    run_id: &str,
    subdomain_hosts: &[String],
) -> Result<CoverageLandingSummary, GolishError> {
    let subdomains = land_subdomain_assets(pool, organization, run_id, subdomain_hosts).await?;
    Ok(CoverageLandingSummary { subdomains })
}

/// What a per-asset coverage refresh observed/wrote for target-intel.
#[derive(Debug, Default, Clone)]
pub struct PerAssetLandingSummary {
    pub subdomains: usize,
    pub dns_records: usize,
    pub dns_found_hosts: Vec<String>,
    pub dns_empty_hosts: Vec<String>,
    pub dns_partial_hosts: Vec<String>,
    pub dns_error_hosts: Vec<String>,
    pub dns_refresh_failed: bool,
}

/// Re-run ONLY the per-asset coverage landing (subdomains + DNS) for an org whose
/// in-scope targets are now registered. Closes the enrich-time ordering gap: the
/// agent calls `recon_map_assets` (which lands) BEFORE `manage_targets add`, so
/// at enrich time `targets WHERE scope='in'` is empty and nothing lands; the
/// gate-read path can call this once targets exist to refresh the gate-read tables
/// (`target_assets` / `dns_records`). Per-asset only — org-level WHOIS
/// (`land_whois`, RDAP) is the `recon_lookup_whois` tool's job and is NOT run here;
/// CT was removed from the enrich landing entirely (plan 2026-06-18-slim-enrich).
/// Idempotent (NOT EXISTS / upsert skip already-landed) and fully non-fatal.
/// Returns (subdomains, dns_records) landed this pass.
pub async fn refresh_per_asset_landing(pool: &sqlx::PgPool, org_id: Uuid) -> (usize, usize) {
    let summary = refresh_per_asset_landing_summary(pool, org_id).await;
    (summary.subdomains, summary.dns_records)
}

/// Same refresh as [`refresh_per_asset_landing`], plus the concrete domains whose
/// DNS lookup was actually attempted and returned no A/AAAA/CNAME/MX/TXT answers.
/// Callers with a real evidence id can persist those as `checked_empty` outcomes.
pub async fn refresh_per_asset_landing_summary(
    pool: &sqlx::PgPool,
    org_id: Uuid,
) -> PerAssetLandingSummary {
    let org = match golish_db::repo::organizations::get_one(pool, org_id).await {
        Ok(Some(org)) => org,
        Ok(None) => {
            tracing::warn!(%org_id, "per-asset landing organization no longer exists");
            return PerAssetLandingSummary {
                dns_refresh_failed: true,
                ..Default::default()
            };
        }
        Err(error) => {
            tracing::warn!(%org_id, %error, "per-asset landing organization lookup failed");
            return PerAssetLandingSummary {
                dns_refresh_failed: true,
                ..Default::default()
            };
        }
    };
    let subdomains = land_subdomain_assets(pool, &org, "gate-refresh", &[])
        .await
        .unwrap_or(0);
    let (dns, dns_refresh_failed) = match land_dns_records(pool, &org).await {
        Ok(summary) => (summary, false),
        Err(error) => {
            tracing::warn!(
                organization_id = %org.id,
                %error,
                "target_intel DNS refresh failed before per-host outcomes were available"
            );
            (DnsLandingSummary::default(), true)
        }
    };
    PerAssetLandingSummary {
        subdomains,
        dns_records: dns.records,
        dns_found_hosts: dns.found_hosts,
        dns_empty_hosts: dns.empty_hosts,
        dns_partial_hosts: dns.partial_hosts,
        dns_error_hosts: dns.error_hosts,
        dns_refresh_failed,
    }
}

/// Pair each candidate **host** with the org-owned **root** domain it belongs to
/// (longest owned suffix wins). Pure (no IO) for unit testing. Hosts that equal an
/// owned root, or that belong to no owned root, are dropped. Callers pre-filter to
/// the right values (org-recon path: `Domain`-kind record values; agent path:
/// `organizations.domains`).
fn collect_subdomain_pairs(
    organization: &golish_db::models::Organization,
    hosts: &[String],
) -> Vec<(String, String)> {
    let roots = organization_owned_domains(organization);
    if roots.is_empty() {
        return Vec::new();
    }
    let mut seen = HashSet::new();
    let mut pairs = Vec::new();
    for raw in hosts {
        let Some(host) = normalized_host(raw) else {
            continue;
        };
        let Some(root) = roots
            .iter()
            .filter(|root| strict_subdomain_of_scope_pattern(&host, root))
            .max_by_key(|root| root.trim_start_matches("*.").len())
        else {
            continue;
        };
        if seen.insert((root.clone(), host.clone())) {
            pairs.push((root.clone(), host));
        }
    }
    pairs
}

/// Pair each host with the longest OTHER host **in the same set** that it is a
/// strict subdomain of. Hosts that are nobody's subdomain (apex roots, IPs) yield
/// no pair. Pure (no IO) for unit testing.
///
/// Retained as a pure regression helper only. Production landing deliberately
/// does not run this over the cumulative target set because doing so would refresh
/// stale SUBDOMAIN observations without a current provider result.
#[cfg(test)]
fn pair_subdomains_within(hosts: &[String]) -> Vec<(String, String)> {
    let norm: Vec<String> = hosts.iter().filter_map(|h| normalized_host(h)).collect();
    let mut seen = HashSet::new();
    let mut pairs = Vec::new();
    for host in &norm {
        let Some(root) = norm
            .iter()
            .filter(|root| *root != host && strict_subdomain_of_scope_pattern(host, root))
            .max_by_key(|root| root.len())
        else {
            continue;
        };
        if seen.insert((root.clone(), host.clone())) {
            pairs.push((root.clone(), host.clone()));
        }
    }
    pairs
}

/// Persist `(root, subdomain)` pairs as `target_assets(asset_type='subdomain')`
/// children of the root's in-scope target. Returns how many landed. Roots with no
/// existing target row are skipped (we never invent an in-scope root here).
async fn land_subdomain_assets(
    pool: &sqlx::PgPool,
    organization: &golish_db::models::Organization,
    run_id: &str,
    subdomain_hosts: &[String],
) -> Result<usize, GolishError> {
    // Source 1 (legacy): the owned-domain list the caller passed (org-recon:
    // `Domain` records; agent enrich: `organizations.domains`).
    let pair_set: HashSet<(String, String)> =
        collect_subdomain_pairs(organization, subdomain_hosts)
            .into_iter()
            .collect();
    // Only the caller's current-run observations may create or refresh a
    // SUBDOMAIN relation. Re-pairing every historical in-scope target here used
    // to update `target_assets.updated_at` on every retry, falsely making an old
    // child satisfy the current stage freshness window.
    if pair_set.is_empty() {
        return Ok(0);
    }

    // Resolve the current observation's authorized root rows. This read is only
    // an identity lookup; it does not treat the full target set as observations.
    let in_scope: Vec<(Uuid, String)> = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT id, value FROM targets WHERE scope::text = 'in' AND organization_id = $1",
    )
    .bind(organization.id)
    .fetch_all(pool)
    .await?;
    let mut root_targets: HashMap<String, Option<Uuid>> = HashMap::new();
    for (id, value) in &in_scope {
        if let Some(host) = normalized_host(value) {
            root_targets.entry(host).or_insert(Some(*id));
        }
    }
    // HashSet iteration order is nondeterministic → sort for reproducible upserts/logs.
    let mut pairs: Vec<(String, String)> = pair_set.into_iter().collect();
    pairs.sort();
    let metadata = json!({ "source": "organization_recon", "run_id": run_id });
    // #4/E3 (设计 2026-06-23-technique-outcomes-provenance): enrich/landing 写点 provenance
    // 物化（**始终写**，无灰度开关；用户 2026-06-23 删除原 GOLISH_TECHNIQUE_OUTCOMES_WRITE
    // env 开关）。非致命 warn（写失败 / 表未 apply 只 warn，绝不影响 landing 主流程）。
    let mut landed = 0usize;
    for (root, subdomain) in pairs {
        let target_id = match root_targets.get(&root) {
            Some(cached) => *cached,
            None => {
                let resolved: Option<Uuid> = sqlx::query_scalar(
                    r#"SELECT id FROM targets
                       WHERE value = $1
                         AND target_type::text = 'domain'
                         AND organization_id = $2
                         AND project_path IS NOT DISTINCT FROM $3
                       ORDER BY (scope::text = 'in') DESC, updated_at DESC
                       LIMIT 1"#,
                )
                .bind(&root)
                .bind(organization.id)
                .bind(&organization.project_path)
                .fetch_optional(pool)
                .await?;
                root_targets.insert(root.clone(), resolved);
                resolved
            }
        };
        let Some(target_id) = target_id else {
            continue;
        };
        golish_db::repo::target_assets::upsert(
            pool,
            target_id,
            Some(organization.project_path.as_str()),
            "subdomain",
            &subdomain,
            None,
            None,
            None,
            None,
            &metadata,
        )
        .await?;
        landed += 1;
        // PR-C step2b: 同步 upsert technique_outcomes（SUBDOMAIN found provenance）。
        // asset 走 canonical_asset_key（E1）；evidence_ids 空（landing 无 ledger
        // 行）；collected_at None（landing 时刻非该维证据时刻）；非致命 warn。
        // coverage 仍由 db_truth 提供——本表此处仅记 provenance。
        let canonical = golish_pentest_domain::canonical_asset_key(&subdomain)
            .map(|k| k.key)
            .unwrap_or_else(|| subdomain.clone());
        let w = golish_db::repo::technique_outcomes::TechniqueOutcomeWrite {
            organization_id: organization.id,
            run_id: run_id.to_string(),
            asset: canonical,
            technique: "GOLISH-INTEL-SUBDOMAIN".to_string(),
            outcome: "found".to_string(),
            source: Some("organization_recon".to_string()),
            query: Some(root.clone()),
            result_count: None,
            confidence: None,
            evidence_ids: Vec::new(),
            collected_at: None,
        };
        golish_db::repo::technique_outcomes::upsert(pool, &w).await?;
    }
    Ok(landed)
}

/// Resolve in-scope **domain** targets of this org and
/// land their A/AAAA/CNAME/MX/TXT answers into `dns_records` — the table the
/// target_intel DNS coverage cell reads (`dns_records::present_target_values` /
/// `coverage_truth`). enrich records carry domains and IPs unpaired, so we do a
/// bounded best-effort resolve (the honest way to produce real records). When
/// hickory is available A and AAAA are queried explicitly, so an authoritative
/// no-record response can become `checked_empty`; the OS resolver is only a
/// positive fallback for unusable/failed hickory resolution and never proves a
/// negative. CNAME/MX/TXT use the same typed hickory resolver. No stage drives `dig`
/// (target_intel forbids scan-tool fallback, EAS reuses inherited DNS), so this
/// landing is the only place those record types can be collected. DNS queries
/// hit the resolver, not the target's own hosts, so it stays zero-touch.
/// failures/timeouts are non-fatal but remain typed `error`; they are never
/// projected as checked-empty.
#[derive(Debug, Default)]
struct DnsLandingSummary {
    records: usize,
    found_hosts: Vec<String>,
    empty_hosts: Vec<String>,
    partial_hosts: Vec<String>,
    error_hosts: Vec<String>,
}

#[derive(Debug)]
struct DnsResolveOutcome {
    target_id: Uuid,
    host: String,
    records: Vec<(&'static str, String, String)>,
    state: DnsAttemptState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DnsAttemptState {
    Found,
    Empty,
    Partial,
    Error,
}

const DNS_QUERY_GROUP_COUNT: usize = 4;
const DNS_MAX_RESOLVE_CONCURRENCY: usize = 128;

fn dns_target_query_sql() -> &'static str {
    r#"SELECT t.id, t.value FROM targets t
       WHERE t.organization_id = $1
         AND t.scope::text = 'in'
         AND t.target_type::text = 'domain'
       ORDER BY t.created_at ASC, t.value ASC, t.id ASC"#
}

fn resolv_conf_nameservers(contents: &str) -> Vec<IpAddr> {
    let mut out = Vec::new();
    for line in contents.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some(raw) = line.strip_prefix("nameserver").map(str::trim) else {
            continue;
        };
        // A scoped link-local address needs an interface id that hickory's
        // NameServerConfig cannot represent. Skip it; another ordinary resolver
        // from the same system file remains usable.
        if raw.contains('%') {
            continue;
        }
        if let Ok(ip) = raw.parse::<IpAddr>() {
            if !out.contains(&ip) {
                out.push(ip);
            }
        }
    }
    out
}

fn fallback_resolver_from_resolv_conf() -> Option<TokioResolver> {
    let contents = std::fs::read_to_string("/etc/resolv.conf").ok()?;
    let nameservers = resolv_conf_nameservers(&contents)
        .into_iter()
        .map(NameServerConfig::udp_and_tcp)
        .collect::<Vec<_>>();
    if nameservers.is_empty() {
        return None;
    }
    Resolver::builder_with_config(
        ResolverConfig::from_parts(None, Vec::new(), nameservers),
        TokioRuntimeProvider::default(),
    )
    .build()
    .ok()
}

fn classify_dns_host_attempt(attempts: &[DnsAttemptState]) -> DnsAttemptState {
    let found =
        attempts.contains(&DnsAttemptState::Found) || attempts.contains(&DnsAttemptState::Partial);
    let incomplete = attempts.len() != DNS_QUERY_GROUP_COUNT
        || attempts.contains(&DnsAttemptState::Error)
        || attempts.contains(&DnsAttemptState::Partial);
    if found && incomplete {
        DnsAttemptState::Partial
    } else if found {
        DnsAttemptState::Found
    } else if incomplete {
        DnsAttemptState::Error
    } else {
        DnsAttemptState::Empty
    }
}

fn classify_dns_address_attempts(attempts: &[DnsAttemptState]) -> DnsAttemptState {
    let found =
        attempts.contains(&DnsAttemptState::Found) || attempts.contains(&DnsAttemptState::Partial);
    let incomplete = attempts.len() != 2
        || attempts.contains(&DnsAttemptState::Error)
        || attempts.contains(&DnsAttemptState::Partial);
    if found && incomplete {
        DnsAttemptState::Partial
    } else if found {
        DnsAttemptState::Found
    } else if attempts.len() == 2
        && attempts
            .iter()
            .all(|state| *state == DnsAttemptState::Empty)
    {
        DnsAttemptState::Empty
    } else {
        DnsAttemptState::Error
    }
}

async fn resolve_address_records(
    resolver: Option<&TokioResolver>,
    fqdn: &str,
    host: &str,
    os_timeout: std::time::Duration,
) -> (Vec<(&'static str, String, String)>, DnsAttemptState) {
    let mut records = Vec::new();
    let mut typed_attempts = Vec::with_capacity(2);

    if let Some(resolver) = resolver {
        for record_type in [RecordType::A, RecordType::AAAA] {
            let expected = if record_type == RecordType::A {
                "A"
            } else {
                "AAAA"
            };
            let before = records.len();
            let state = match tokio::time::timeout(
                std::time::Duration::from_secs(3),
                resolver.lookup(fqdn.to_string(), record_type),
            )
            .await
            {
                Ok(Ok(lookup)) => {
                    for record in lookup.answers() {
                        if let Some((actual, value)) = rdata_to_dns_record(&record.data) {
                            if actual == expected {
                                records.push((actual, host.to_string(), value));
                            }
                        }
                    }
                    if records.len() > before {
                        DnsAttemptState::Found
                    } else {
                        DnsAttemptState::Empty
                    }
                }
                Ok(Err(error)) if error.is_no_records_found() => DnsAttemptState::Empty,
                Ok(Err(error)) => {
                    tracing::warn!(%host, %record_type, %error, "typed DNS address lookup failed");
                    DnsAttemptState::Error
                }
                Err(_) => {
                    tracing::warn!(%host, %record_type, "typed DNS address lookup timed out");
                    DnsAttemptState::Error
                }
            };
            typed_attempts.push(state);
        }
    }

    let mut state = classify_dns_address_attempts(&typed_attempts);
    let needs_positive_os_fallback =
        resolver.is_none() || typed_attempts.contains(&DnsAttemptState::Error);
    if needs_positive_os_fallback {
        match tokio::time::timeout(os_timeout, tokio::net::lookup_host((host, 0))).await {
            Ok(Ok(lookup)) => {
                let before = records.len();
                for socket in lookup {
                    let ip = socket.ip();
                    let record_type = if ip.is_ipv4() { "A" } else { "AAAA" };
                    records.push((record_type, host.to_string(), ip.to_string()));
                }
                if records.len() > before {
                    state =
                        if resolver.is_some() && typed_attempts.contains(&DnsAttemptState::Error) {
                            DnsAttemptState::Partial
                        } else {
                            DnsAttemptState::Found
                        };
                }
            }
            Ok(Err(error)) => {
                tracing::warn!(%host, %error, "OS DNS positive fallback failed");
            }
            Err(_) => {
                tracing::warn!(%host, "OS DNS positive fallback timed out");
            }
        }
    }

    records.sort_by(|left, right| left.0.cmp(right.0).then(left.2.cmp(&right.2)));
    records.dedup();
    (records, state)
}

async fn land_dns_records(
    pool: &sqlx::PgPool,
    organization: &golish_db::models::Organization,
) -> Result<DnsLandingSummary, GolishError> {
    // macOS SystemConfiguration may need several seconds to traverse scoped
    // resolvers after hickory rejects a `%en0` link-local nameserver. Three
    // seconds caused real public A records to be mislabeled as transport errors
    // in the MoreSec acceptance run. Keep this bounded, but leave enough room
    // for the platform resolver's blocking-pool hop and first lookup.
    const ADDRESS_RESOLVE_TIMEOUT_SECS: u64 = 10;
    let targets: Vec<(Uuid, String)> = sqlx::query_as(dns_target_query_sql())
        .bind(organization.id)
        .fetch_all(pool)
        .await?;
    if targets.is_empty() {
        return Ok(DnsLandingSummary::default());
    }
    // Built once from the system resolver config and cloned into each task (the
    // resolver is cheap to clone — Arc inside). Construction failure is a typed
    // error for every host, never a fabricated checked-empty.
    let dns_resolver: Option<TokioResolver> = match hickory_resolver::Resolver::builder_tokio() {
        Ok(builder) => match builder.build() {
            Ok(resolver) => Some(resolver),
            Err(error) => {
                let fallback = fallback_resolver_from_resolv_conf();
                tracing::warn!(
                    %error,
                    fallback_available = fallback.is_some(),
                    "DNS system resolver build failed; tried filtered /etc/resolv.conf fallback"
                );
                fallback
            }
        },
        Err(error) => {
            // macOS may expose a scoped link-local nameserver (`fe80::...%en0`)
            // that hickory 0.26 cannot parse as `IpAddr`. Do not lose ordinary
            // address resolution just because the auxiliary resolver is absent.
            let fallback = fallback_resolver_from_resolv_conf();
            tracing::warn!(
                %error,
                fallback_available = fallback.is_some(),
                "DNS system resolver config failed; tried filtered /etc/resolv.conf fallback"
            );
            fallback
        }
    };
    let mut summary = DnsLandingSummary::default();
    // Target Intel has no asset-wave denominator of its own. Resolve every
    // current in-scope domain, but in bounded chunks so >128 targets cannot
    // starve forever behind a fixed newest-only LIMIT or explode concurrency.
    for chunk in targets.chunks(DNS_MAX_RESOLVE_CONCURRENCY) {
        let mut set = tokio::task::JoinSet::new();
        for (target_id, value) in chunk.iter().cloned() {
            let resolver = dns_resolver.clone();
            set.spawn(async move {
                let Some(host) = normalized_host(&value) else {
                    return DnsResolveOutcome {
                        target_id,
                        host: value,
                        records: Vec::new(),
                        state: DnsAttemptState::Error,
                    };
                };
                let mut records: Vec<(&'static str, String, String)> = Vec::new();
                let mut attempts = Vec::with_capacity(DNS_QUERY_GROUP_COUNT);
                let fqdn = format!("{host}.");

                let (address_records, address_state) = resolve_address_records(
                    resolver.as_ref(),
                    &fqdn,
                    &host,
                    std::time::Duration::from_secs(ADDRESS_RESOLVE_TIMEOUT_SECS),
                )
                .await;
                records.extend(address_records);
                attempts.push(address_state);

                // CNAME / MX / TXT are additive and independently bounded. A successful
                // no-record response is Empty; only usable persisted answers are Found.
                for record_type in [RecordType::CNAME, RecordType::MX, RecordType::TXT] {
                    let state = if let Some(resolver) = resolver.as_ref() {
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(3),
                            resolver.lookup(fqdn.clone(), record_type),
                        )
                        .await
                        {
                            Ok(Ok(lookup)) => {
                                let before = records.len();
                                let expected = match record_type {
                                    RecordType::CNAME => "CNAME",
                                    RecordType::MX => "MX",
                                    RecordType::TXT => "TXT",
                                    _ => unreachable!("fixed auxiliary DNS record types"),
                                };
                                for record in lookup.answers() {
                                    if let Some((rt, value)) = rdata_to_dns_record(&record.data) {
                                        if rt == expected {
                                            records.push((rt, host.clone(), value));
                                        }
                                    }
                                }
                                if records.len() > before {
                                    DnsAttemptState::Found
                                } else {
                                    DnsAttemptState::Empty
                                }
                            }
                            Ok(Err(error)) if error.is_no_records_found() => DnsAttemptState::Empty,
                            Ok(Err(_)) | Err(_) => DnsAttemptState::Error,
                        }
                    } else {
                        DnsAttemptState::Error
                    };
                    attempts.push(state);
                }
                DnsResolveOutcome {
                    target_id,
                    host,
                    records,
                    state: classify_dns_host_attempt(&attempts),
                }
            });
        }
        while let Some(joined) = set.join_next().await {
            let Ok(outcome) = joined else {
                tracing::warn!("target_intel DNS resolver task failed to join");
                continue;
            };
            match outcome.state {
                DnsAttemptState::Empty => {
                    summary.empty_hosts.push(outcome.host);
                    continue;
                }
                DnsAttemptState::Error => {
                    summary.error_hosts.push(outcome.host);
                    continue;
                }
                DnsAttemptState::Found | DnsAttemptState::Partial => {}
            }
            // Primary IP is a deterministic cache only: IPv4 first, then lexical
            // canonical address. Full multi-address truth remains in dns_records.
            let primary_ip = deterministic_primary_dns_ip(&outcome.records);
            let mut write_failed = false;
            let mut host_records_stored = 0usize;
            for (record_type, name, value) in &outcome.records {
                match golish_db::repo::dns_records::upsert(
                    pool,
                    outcome.target_id,
                    organization.project_path.as_str(),
                    record_type,
                    name,
                    value,
                    "resolver",
                )
                .await
                {
                    Ok(_) => {
                        summary.records += 1;
                        host_records_stored += 1;
                    }
                    Err(error) => {
                        write_failed = true;
                        tracing::warn!(
                            host = %outcome.host,
                            %record_type,
                            %value,
                            %error,
                            "target_intel DNS record upsert failed"
                        );
                    }
                }
            }
            if !write_failed {
                match outcome.state {
                    DnsAttemptState::Found => summary.found_hosts.push(outcome.host.clone()),
                    DnsAttemptState::Partial => summary.partial_hosts.push(outcome.host.clone()),
                    DnsAttemptState::Empty | DnsAttemptState::Error => unreachable!(),
                }
            } else if host_records_stored > 0 {
                summary.partial_hosts.push(outcome.host.clone());
            } else {
                summary.error_hosts.push(outcome.host.clone());
            }
            if !write_failed {
                if let Some(ip) = primary_ip {
                    let _ =
                        golish_db::repo::targets::set_real_ip_by_id(pool, outcome.target_id, &ip)
                            .await;
                }
            }
        }
    }
    summary.empty_hosts.sort();
    summary.empty_hosts.dedup();
    summary.found_hosts.sort();
    summary.found_hosts.dedup();
    summary.partial_hosts.sort();
    summary.partial_hosts.dedup();
    summary.error_hosts.sort();
    summary.error_hosts.dedup();
    Ok(summary)
}

fn deterministic_primary_dns_ip(records: &[(&'static str, String, String)]) -> Option<String> {
    let mut ips: Vec<std::net::IpAddr> = records
        .iter()
        .filter(|(record_type, _, _)| matches!(*record_type, "A" | "AAAA"))
        .filter_map(|(_, _, value)| value.parse().ok())
        .collect();
    ips.sort_by(|left, right| match (left, right) {
        (std::net::IpAddr::V4(_), std::net::IpAddr::V6(_)) => std::cmp::Ordering::Less,
        (std::net::IpAddr::V6(_), std::net::IpAddr::V4(_)) => std::cmp::Ordering::Greater,
        _ => left.to_string().cmp(&right.to_string()),
    });
    ips.dedup();
    ips.first().map(ToString::to_string)
}

/// Map a hickory `RData` answer to a `(record_type, value)` tuple for
/// `dns_records`. Returns `None` for record types we do not persist or an empty
/// TXT payload.
fn rdata_to_dns_record(rdata: &RData) -> Option<(&'static str, String)> {
    match rdata {
        RData::A(address) => Some(("A", address.0.to_string())),
        RData::AAAA(address) => Some(("AAAA", address.0.to_string())),
        RData::CNAME(name) => Some(("CNAME", normalize_dns_target(&name.0.to_utf8()))),
        RData::MX(mx) => Some(("MX", format_mx_value(mx.preference, &mx.exchange.to_utf8()))),
        RData::TXT(txt) => {
            let joined: String = txt
                .txt_data
                .iter()
                .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
                .collect::<Vec<_>>()
                .concat();
            let trimmed = joined.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(("TXT", trimmed.to_string()))
            }
        }
        _ => None,
    }
}

/// Normalize a DNS target name (CNAME target / MX exchange): strip the trailing
/// root dot and lowercase so duplicate answers collapse on the `dns_records`
/// unique key.
fn normalize_dns_target(name: &str) -> String {
    name.trim().trim_end_matches('.').to_ascii_lowercase()
}

/// Render an MX answer as `"<preference> <exchange>"` so the single `value`
/// column keeps both the priority and the mail host.
fn format_mx_value(preference: u16, exchange: &str) -> String {
    format!("{preference} {}", normalize_dns_target(exchange))
}

/// JSONB "has content" predicate matching `coverage_truth`'s emptiness semantics
/// (NULL / `null` / `[]` / `{}` / blank string = empty).
fn json_value_is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(items) => items.is_empty(),
        Value::Object(map) => map.is_empty(),
        Value::String(text) => text.trim().is_empty(),
        _ => false,
    }
}

/// Registrable ("apex") domain — delegates to the single source
/// [`golish_pentest_domain::registrable_apex`] (best-effort two-level-TLD table,
/// no full PSL): keep the last 2 labels, or the last 3 under a known two-level TLD
/// (`a.pingan.com.cn` → `pingan.com.cn`, `life.pingan.com` → `pingan.com`). The
/// table is shared with the gate's `is_registrable_apex` so CT/WHOIS target
/// selection here and the SUBDOMAIN coverage gate can never drift on ccTLDs.
fn registrable_domain(host: &str) -> String {
    golish_pentest_domain::registrable_apex(host)
}

/// Unique registrable apex domains from the pre-provider authorized target
/// snapshot (capped), for WHOIS queries. Organization profile fields are not
/// accepted here: providers write `domains`/`app_domains`, so consuming those
/// fields would let a third-party observation authorize its own RDAP request on
/// this or a later retry.
fn registrable_domains_from_authorized_hosts(authorized_hosts: &[String]) -> Vec<String> {
    let mut out = BTreeSet::new();
    for host in authorized_hosts
        .iter()
        .filter_map(|value| normalized_host(value))
    {
        let host = host.trim_start_matches("*.").to_string();
        // Never CT/WHOIS-query an IP literal: a bare IP has no registrable apex or
        // CT log, and `registrable_domain` would mangle it into a junk 2-label
        // fragment (e.g. `124.196.77.48` -> `77.48`) that consumes the limited
        // query slots and crowds out the real owned roots — leaving CT permanently
        // unlanded (`organizations.certificates` empty -> the target_intel CT
        // coverage cell never reaches a terminal state). The polluting IPs come
        // from URL-wrapped IPs that `normalized_host` extracts host-only.
        if host.parse::<std::net::IpAddr>().is_ok() {
            continue;
        }
        let apex = registrable_domain(&host);
        if !apex.is_empty() {
            out.insert(apex);
        }
    }
    out.into_iter().take(3).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WhoisLandingState {
    Found,
    Empty,
    Error,
    Blocked,
}

impl WhoisLandingState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Found => "found",
            Self::Empty => "empty",
            Self::Error => "error",
            Self::Blocked => "blocked",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WhoisLandingOutcome {
    pub(crate) state: WhoisLandingState,
    pub(crate) attempted: usize,
    pub(crate) succeeded: usize,
    pub(crate) reason: Option<String>,
}

fn classify_whois_landing(attempted: usize, succeeded: usize, found: bool) -> WhoisLandingState {
    if attempted == 0 {
        WhoisLandingState::Blocked
    } else if succeeded < attempted {
        WhoisLandingState::Error
    } else if found {
        WhoisLandingState::Found
    } else {
        WhoisLandingState::Empty
    }
}

/// Land WHOIS (RDAP) → `organizations.whois` — the org-level column the
/// target_intel WHOIS coverage cell reads (`coverage_truth::build_org_intel_
/// presence_sql`). HTTP, bounded, best-effort, and always performs a fresh query;
/// an old stored value cannot turn this run into a fabricated empty result.
///
/// CT (crt.sh) is intentionally NOT done here anymore: crt.sh was the 300s-timeout
/// culprit and only produced junk for polluted registrable domains. CT now comes
/// from the `ctfr` tool (crt.sh) + fofa native cert. WHOIS is exposed to the agent
/// as the standalone `recon_lookup_whois` tool. (plan 2026-06-18-slim-enrich)
pub(crate) async fn land_whois(
    pool: &sqlx::PgPool,
    organization: &golish_db::models::Organization,
    authorized_hosts: &[String],
) -> Result<WhoisLandingOutcome, GolishError> {
    let domains = registrable_domains_from_authorized_hosts(authorized_hosts);
    if domains.is_empty() {
        return Ok(WhoisLandingOutcome {
            state: WhoisLandingState::Blocked,
            attempted: 0,
            succeeded: 0,
            reason: Some("no registrable domain".to_string()),
        });
    }
    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("golish-recon/1.0")
        .build()
    else {
        return Ok(WhoisLandingOutcome {
            state: WhoisLandingState::Error,
            attempted: domains.len(),
            succeeded: 0,
            reason: Some("failed to construct RDAP client".to_string()),
        });
    };

    let mut whois_value: Option<Value> = None;
    let attempted = domains.len();
    let mut succeeded = 0usize;
    let mut last_error = None;
    for domain in &domains {
        let url = format!("https://rdap.org/domain/{domain}");
        let resp = match client.get(&url).send().await {
            Ok(resp) => resp,
            Err(error) => {
                last_error = Some(error.to_string());
                continue;
            }
        };
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            succeeded += 1;
            continue;
        }
        if !resp.status().is_success() {
            last_error = Some(format!("RDAP HTTP {}", resp.status()));
            continue;
        }
        let text = match resp.text().await {
            Ok(text) => text,
            Err(error) => {
                last_error = Some(error.to_string());
                continue;
            }
        };
        let value = match serde_json::from_str::<Value>(&text) {
            Ok(value) if value.is_object() => value,
            Ok(_) => {
                last_error = Some("RDAP response was not an object".to_string());
                continue;
            }
            Err(error) => {
                last_error = Some(error.to_string());
                continue;
            }
        };
        succeeded += 1;
        if whois_value.is_none() && !json_value_is_empty(&value) {
            whois_value = Some(value);
        }
    }

    let whois_landed = whois_value.is_some();
    if let Some(value) = whois_value {
        // whois_collected_at bump (design 2026-06-22 §3.2): genuine WHOIS
        // collection site (RDAP fetch), so stamp per-dimension freshness in the
        // same write for the coverage gate's time-windowed read.
        sqlx::query(
            "UPDATE organizations SET whois = $1, whois_collected_at = NOW(), updated_at = NOW() WHERE id = $2",
        )
            .bind(value)
            .bind(organization.id)
            .execute(pool)
            .await?;
    }
    let state = classify_whois_landing(attempted, succeeded, whois_landed);
    Ok(WhoisLandingOutcome {
        state,
        attempted,
        succeeded,
        reason: matches!(state, WhoisLandingState::Error)
            .then(|| last_error.unwrap_or_else(|| "all RDAP requests failed".to_string())),
    })
}

const AUTHORIZED_IP_CONTEXT_SQL: &str = r#"SELECT target_type::text, value
       FROM targets
       WHERE organization_id = $1
         AND project_path IS NOT DISTINCT FROM $2
         AND scope::text = 'in'
         AND target_type::text IN ('ip', 'cidr')
       ORDER BY id"#;

async fn ip_record_is_authorized(
    tx: &mut Transaction<'_, Postgres>,
    organization: &golish_db::models::Organization,
    record: &NormalizedReconRecord,
) -> Result<bool, GolishError> {
    let Ok(candidate) = record.value.trim().parse::<IpAddr>() else {
        return Ok(false);
    };

    if organization_ip_ranges_contain_ip(organization, candidate) {
        return Ok(true);
    }

    let authorized_targets: Vec<(String, String)> = sqlx::query_as(AUTHORIZED_IP_CONTEXT_SQL)
        .bind(organization.id)
        .bind(&organization.project_path)
        .fetch_all(&mut **tx)
        .await?;
    Ok(authorized_target_rows_contain_ip(
        candidate,
        &authorized_targets,
    ))
}

fn organization_ip_ranges_contain_ip(
    organization: &golish_db::models::Organization,
    candidate: IpAddr,
) -> bool {
    json_atom_values(&organization.ip_ranges)
        .iter()
        .any(|value| ip_authorization_value_contains(value, candidate))
}

fn ip_authorization_value_contains(value: &str, candidate: IpAddr) -> bool {
    value
        .trim()
        .parse::<IpAddr>()
        .is_ok_and(|authorized| authorized == candidate)
        || cidr_contains_ip(value, candidate)
}

fn authorized_target_rows_contain_ip(candidate: IpAddr, rows: &[(String, String)]) -> bool {
    rows.iter()
        .any(|(target_type, value)| match target_type.as_str() {
            "ip" => value
                .trim()
                .parse::<IpAddr>()
                .is_ok_and(|authorized| authorized == candidate),
            "cidr" => cidr_contains_ip(value, candidate),
            _ => false,
        })
}

fn cidr_contains_ip(cidr: &str, candidate: IpAddr) -> bool {
    let Some((network, prefix)) = cidr.trim().split_once('/') else {
        return false;
    };
    let Ok(network) = network.trim().parse::<IpAddr>() else {
        return false;
    };
    let Ok(prefix) = prefix.trim().parse::<u8>() else {
        return false;
    };

    match (network, candidate) {
        (IpAddr::V4(network), IpAddr::V4(candidate)) if prefix <= 32 => {
            let mask = if prefix == 0 {
                0
            } else {
                u32::MAX << (32 - prefix)
            };
            u32::from(network) & mask == u32::from(candidate) & mask
        }
        (IpAddr::V6(network), IpAddr::V6(candidate)) if prefix <= 128 => {
            let mask = if prefix == 0 {
                0
            } else {
                u128::MAX << (128 - prefix)
            };
            u128::from(network) & mask == u128::from(candidate) & mask
        }
        _ => false,
    }
}

fn target_type_for_record(
    organization: &golish_db::models::Organization,
    record: &NormalizedReconRecord,
) -> Option<&'static str> {
    match record.kind {
        ReconRecordKind::Domain if record_belongs_to_organization(organization, record) => {
            Some("domain")
        }
        ReconRecordKind::Ip => None,
        ReconRecordKind::Url if record_belongs_to_organization(organization, record) => Some("url"),
        ReconRecordKind::Site
            if url::Url::parse(&record.value).is_ok()
                && record_belongs_to_organization(organization, record) =>
        {
            Some("url")
        }
        _ => None,
    }
}

fn record_belongs_to_organization(
    organization: &golish_db::models::Organization,
    record: &NormalizedReconRecord,
) -> bool {
    match record.kind {
        ReconRecordKind::Domain | ReconRecordKind::Url | ReconRecordKind::Site => {
            value_belongs_to_organization(organization, &record.value)
        }
        _ => true,
    }
}

pub(crate) fn value_belongs_to_organization(
    organization: &golish_db::models::Organization,
    value: &str,
) -> bool {
    if value.trim().parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    let Some(host) = normalized_host(value) else {
        return false;
    };
    if is_known_public_non_asset_host(&host) {
        return false;
    }
    let domains = organization_owned_domains(organization);
    if domains.is_empty() {
        return false;
    }
    domains
        .iter()
        .any(|domain| host_matches_scope_pattern(&host, domain))
}

fn strict_subdomain_of_scope_pattern(host: &str, pattern: &str) -> bool {
    // Wildcards are authorization patterns, never concrete discovered hosts.
    // Reject both a pattern echo (`*.base`) and nested wildcard values before
    // suffix matching so they cannot become `target_assets(subdomain)` rows.
    if host.starts_with("*.") {
        return false;
    }
    let base = pattern.strip_prefix("*.").unwrap_or(pattern);
    host != base && host.ends_with(&format!(".{base}"))
}

fn host_matches_scope_pattern(host: &str, pattern: &str) -> bool {
    if pattern.starts_with("*.") {
        strict_subdomain_of_scope_pattern(host, pattern)
    } else {
        host == pattern || strict_subdomain_of_scope_pattern(host, pattern)
    }
}

pub(crate) fn organization_owned_domains(
    organization: &golish_db::models::Organization,
) -> Vec<String> {
    let mut domains = Vec::new();
    collect_owned_domain_values(&mut domains, &organization.domains);
    if let Some(intel) = organization.intel.as_object() {
        if let Some(value) = intel.get("app_domains") {
            collect_owned_domain_values(&mut domains, value);
        }
    }
    domains.sort();
    domains.dedup();
    domains
}

fn collect_owned_domain_values(domains: &mut Vec<String>, value: &Value) {
    for item in json_atom_values(value) {
        if let Some(host) = normalized_host(&item) {
            if !is_known_public_non_asset_host(&host) {
                domains.push(host);
            }
        }
    }
}

fn json_atom_values(value: &Value) -> Vec<String> {
    match value {
        Value::Null => Vec::new(),
        Value::String(text) => {
            if text.trim().is_empty() {
                Vec::new()
            } else {
                vec![text.trim().to_string()]
            }
        }
        Value::Array(items) => items.iter().flat_map(json_atom_values).collect(),
        Value::Object(map) => {
            for key in ["domain", "url", "host", "value", "name"] {
                if let Some(value) = map.get(key).and_then(Value::as_str) {
                    if !value.trim().is_empty() {
                        return vec![value.trim().to_string()];
                    }
                }
            }
            map.values().flat_map(json_atom_values).collect()
        }
        other => vec![other.to_string()],
    }
}

pub(crate) fn normalized_host(value: &str) -> Option<String> {
    let value = value.trim().trim_end_matches('.').to_ascii_lowercase();
    if value.is_empty() || value.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }
    if let Some(base) = value.strip_prefix("*.") {
        return looks_like_domain(base).then(|| format!("*.{base}"));
    }
    if let Ok(url) = url::Url::parse(&value) {
        return url.host_str().map(str::to_string);
    }
    if looks_like_domain(&value) {
        return Some(value);
    }
    None
}

fn looks_like_domain(value: &str) -> bool {
    let value = value.trim().trim_end_matches('.');
    if value.contains(char::is_whitespace) || !value.contains('.') {
        return false;
    }
    value.split('.').all(|part| {
        !part.is_empty()
            && part
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    })
}

fn is_known_public_non_asset_host(host: &str) -> bool {
    const PUBLIC_HOSTS: &[&str] = &[
        "github.com",
        "gitlab.com",
        "bitbucket.org",
        "gitee.com",
        "126.com",
        "163.com",
        "gmail.com",
        "hotmail.com",
        "outlook.com",
        "qq.com",
    ];
    PUBLIC_HOSTS
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

fn record_kind_label(kind: &ReconRecordKind) -> &'static str {
    match kind {
        ReconRecordKind::Organization => "organization",
        ReconRecordKind::Domain => "domain",
        ReconRecordKind::Ip => "ip",
        ReconRecordKind::Port => "port",
        ReconRecordKind::Service => "service",
        ReconRecordKind::Url => "url",
        ReconRecordKind::Site => "site",
        ReconRecordKind::App => "app",
        ReconRecordKind::MiniProgram => "mini_program",
        ReconRecordKind::Wechat => "wechat",
        ReconRecordKind::Certificate => "certificate",
        ReconRecordKind::Contact => "contact",
        ReconRecordKind::Leak => "leak",
    }
}

async fn persist_target_record(
    tx: &mut Transaction<'_, Postgres>,
    organization: &golish_db::models::Organization,
    record: &NormalizedReconRecord,
    target_type: &str,
) -> Result<bool, GolishError> {
    let existing: Option<Uuid> = sqlx::query_scalar(build_persist_target_lookup_sql())
        .bind(&record.value)
        .bind(target_type)
        .bind(&organization.project_path)
        .bind(organization.id)
        .fetch_optional(&mut **tx)
        .await?;

    if let Some(id) = existing {
        let claimed = sqlx::query(
            r#"UPDATE targets
               SET organization_id = $2,
                   updated_at = NOW()
               WHERE id = $1
                 AND (organization_id = $2 OR organization_id IS NULL)"#,
        )
        .bind(id)
        .bind(organization.id)
        .execute(&mut **tx)
        .await?;
        if claimed.rows_affected() == 0 {
            return Err(GolishError::Validation(format!(
                "target ownership changed while claiming {target_type}:{}",
                record.value
            )));
        }
        return Ok(true);
    }

    sqlx::query(
        r#"INSERT INTO targets
              (name, target_type, value, tags, notes, scope, grp, owner,
               organization_id, project_path, source, parent_id)
           VALUES
              ($1, $2::target_type, $3, '[]', '', 'in'::scope_type, 'default', '',
               $4, $5, 'organization_recon', NULL)"#,
    )
    .bind(&record.value)
    .bind(target_type)
    .bind(&record.value)
    .bind(organization.id)
    .bind(&organization.project_path)
    .execute(&mut **tx)
    .await?;
    Ok(false)
}

fn build_persist_target_lookup_sql() -> &'static str {
    r#"SELECT id FROM targets
       WHERE value = $1
         AND target_type::text = $2
         AND project_path IS NOT DISTINCT FROM $3
         AND (organization_id = $4 OR organization_id IS NULL)
       ORDER BY (organization_id = $4) DESC NULLS LAST, updated_at DESC
       LIMIT 1"#
}

async fn write_audit(
    tx: &mut Transaction<'_, Postgres>,
    organization: &golish_db::models::Organization,
    run_id: &str,
    summary: &PersistenceSummary,
    manifest_path: &str,
) -> Result<(), GolishError> {
    let run_uuid = Uuid::parse_str(run_id).ok();
    let detail = json!({
        "runId": run_id,
        "organizationId": organization.id,
        "recordCount": summary.record_count,
        "targetInserted": summary.target_inserted,
        "targetExisting": summary.target_existing,
        "profileUpdates": summary.profile_updates,
        "unsupportedRecords": summary.unsupported_records,
        "recordResults": summary.record_results,
        "manifestPath": manifest_path,
    });
    sqlx::query(
        r#"INSERT INTO audit_log
              (action, category, details, project_path, source,
               target_id, session_id, tool_name, status, detail, run_id)
           VALUES
              ('organization_recon_persisted', 'recon',
               'Organization recon records persisted',
               $1, 'organization_recon', NULL, $2, 'organization_recon',
               'completed', $3, $4)"#,
    )
    .bind(&organization.project_path)
    .bind(run_id)
    .bind(detail)
    .bind(run_uuid)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

struct ProfileAccumulator {
    domains: Value,
    ip_ranges: Value,
    email_domains: Value,
    intel: Value,
    certificates: Value,
    business_systems: Value,
    social_accounts: Value,
    historical_vulns: Value,
    contacts: Value,
    /// Intel coverage dimensions this accumulation actually collected this run
    /// (design 2026-06-22 §3.2) — drives per-dimension `*_collected_at` stamping
    /// in [`ProfileAccumulator::write`]. Set on record-kind match (not per-value
    /// dedup), so re-finding known values still marks the dimension fresh.
    touched_ct: bool,
    touched_osint: bool,
}

impl ProfileAccumulator {
    fn from_organization(organization: &golish_db::models::Organization) -> Self {
        Self {
            domains: array_or_empty(&organization.domains),
            ip_ranges: array_or_empty(&organization.ip_ranges),
            email_domains: array_or_empty(&organization.email_domains),
            intel: object_or_empty(&organization.intel),
            certificates: array_or_empty(&organization.certificates),
            business_systems: array_or_empty(&organization.business_systems),
            social_accounts: array_or_empty(&organization.social_accounts),
            historical_vulns: array_or_empty(&organization.historical_vulns),
            contacts: object_or_empty(&organization.contacts),
            touched_ct: false,
            touched_osint: false,
        }
    }

    fn merge_record(&mut self, record: &NormalizedReconRecord) -> bool {
        let field = record
            .attributes
            .get("profileField")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match record.kind {
            ReconRecordKind::App => push_intel_array(&mut self.intel, "mobile_apps", &record.value),
            ReconRecordKind::MiniProgram => {
                push_intel_array(&mut self.intel, "mini_programs", &record.value)
            }
            ReconRecordKind::Wechat => {
                self.touched_osint = true;
                push_json_array(&mut self.social_accounts, &record.value)
            }
            ReconRecordKind::Certificate => {
                self.touched_ct = true;
                push_json_array(&mut self.certificates, &record.value)
            }
            ReconRecordKind::Contact => {
                self.touched_osint = true;
                let channel = contact_channel(field, &record.value);
                push_contact(&mut self.contacts, channel, &record.value)
            }
            ReconRecordKind::Leak => {
                if field.contains("historical_vulns") {
                    push_json_array(&mut self.historical_vulns, &record.value)
                } else {
                    push_intel_array(&mut self.intel, "leaks", &record.value)
                }
            }
            ReconRecordKind::Domain => {
                if field.contains("email_domains") {
                    push_json_array(&mut self.email_domains, &record.value)
                } else if field.contains("mail_mx") {
                    push_intel_array(&mut self.intel, "mail_mx", &record.value)
                } else {
                    push_json_array(&mut self.domains, &record.value)
                }
            }
            // IP observations never expand authorization. Confirmed organization
            // `ip_ranges` are read above by `ip_record_is_authorized`; an
            // unconfirmed passive IP must remain evidence-only instead of
            // authorizing itself for the next run.
            ReconRecordKind::Ip => false,
            ReconRecordKind::Url | ReconRecordKind::Site => {
                self.touched_osint = true;
                push_json_array(&mut self.business_systems, &record.value)
            }
            ReconRecordKind::Organization | ReconRecordKind::Port | ReconRecordKind::Service => {
                false
            }
        }
    }

    async fn write(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        organization_id: Uuid,
    ) -> Result<(), GolishError> {
        // Per-dimension freshness stamps (design 2026-06-22 §3.2): append
        // `*_collected_at = NOW()` only for intel coverage dimensions this
        // accumulation collected this run (touched_*). Column names are fixed
        // literals mirroring golish-db `IntelDim::collected_at_column` (no
        // injection). CT ← Certificate records; OSINT ← Wechat / Contact /
        // Url|Site records (social_accounts / contacts / business_systems).
        let mut freshness = String::new();
        if self.touched_ct {
            freshness.push_str(", certificates_collected_at = NOW()");
        }
        if self.touched_osint {
            freshness.push_str(", osint_collected_at = NOW()");
        }
        let sql = format!(
            r#"UPDATE organizations
               SET domains = $1,
                   ip_ranges = $2,
                   email_domains = $3,
                   intel = $4,
                   certificates = $5,
                   business_systems = $6,
                   social_accounts = $7,
                   historical_vulns = $8,
                   contacts = $9,
                   updated_at = NOW(){freshness}
               WHERE id = $10"#
        );
        sqlx::query(&sql)
            .bind(&self.domains)
            .bind(&self.ip_ranges)
            .bind(&self.email_domains)
            .bind(&self.intel)
            .bind(&self.certificates)
            .bind(&self.business_systems)
            .bind(&self.social_accounts)
            .bind(&self.historical_vulns)
            .bind(&self.contacts)
            .bind(organization_id)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }
}

fn array_or_empty(value: &Value) -> Value {
    if value.is_array() {
        value.clone()
    } else {
        Value::Array(Vec::new())
    }
}

fn object_or_empty(value: &Value) -> Value {
    if value.is_object() {
        value.clone()
    } else {
        Value::Object(Map::new())
    }
}

fn push_json_array(target: &mut Value, value: &str) -> bool {
    if !target.is_array() {
        *target = Value::Array(Vec::new());
    }
    let Some(items) = target.as_array_mut() else {
        return false;
    };
    push_unique_string(items, value)
}

fn push_intel_array(intel: &mut Value, key: &str, value: &str) -> bool {
    if !intel.is_object() {
        *intel = Value::Object(Map::new());
    }
    let Some(map) = intel.as_object_mut() else {
        return false;
    };
    let entry = map
        .entry(key.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !entry.is_array() {
        *entry = Value::Array(vec![entry.clone()]);
    }
    let Some(items) = entry.as_array_mut() else {
        return false;
    };
    push_unique_string(items, value)
}

fn push_contact(contacts: &mut Value, channel: &str, value: &str) -> bool {
    if !contacts.is_object() {
        *contacts = Value::Object(Map::new());
    }
    let Some(map) = contacts.as_object_mut() else {
        return false;
    };
    let entry = map
        .entry(channel.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !entry.is_array() {
        *entry = Value::Array(Vec::new());
    }
    let Some(items) = entry.as_array_mut() else {
        return false;
    };
    push_unique_string(items, value)
}

fn push_unique_string(items: &mut Vec<Value>, value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    let key = value.to_lowercase();
    if items
        .iter()
        .filter_map(Value::as_str)
        .any(|existing| existing.trim().to_lowercase() == key)
    {
        return true;
    }
    items.push(Value::String(value.into()));
    true
}

fn contact_channel(field: &str, value: &str) -> &'static str {
    if field.contains("phone") || value.chars().filter(|ch| ch.is_ascii_digit()).count() >= 7 {
        "phone"
    } else if field.contains("email") || value.contains('@') {
        "email"
    } else {
        "other"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::organization_recon::types::ReconEvidenceRef;

    fn record(kind: ReconRecordKind, value: &str, field: &str) -> NormalizedReconRecord {
        NormalizedReconRecord {
            record_id: format!("id:{value}"),
            kind,
            key: format!("key:{value}"),
            value: value.into(),
            attributes: json!({ "profileField": field }),
            evidence: vec![ReconEvidenceRef {
                source_id: "fixture".into(),
                run_id: "run".into(),
                task_id: "processing".into(),
                raw_artifact_path: "raw/profile.json".into(),
            }],
        }
    }

    #[test]
    fn profile_merge_routes_asset_record_types() {
        let org = golish_db::models::Organization {
            id: Uuid::new_v4(),
            project_path: "/tmp/project".into(),
            name: "Org".into(),
            parent_id: None,
            description: String::new(),
            owner: String::new(),
            sort_order: 0,
            aliases: Vec::new(),
            industry: String::new(),
            tier: String::new(),
            credit_code: String::new(),
            domains: json!(["example.com"]),
            ip_ranges: json!([]),
            asns: json!([]),
            email_domains: json!([]),
            scope_rules: json!([]),
            intel: json!({}),
            notes: String::new(),
            certificates: json!([]),
            subsidiaries: json!([]),
            business_systems: json!([]),
            cloud_assets: json!([]),
            github_orgs: json!([]),
            social_accounts: json!([]),
            historical_vulns: json!([]),
            contacts: json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let mut profile = ProfileAccumulator::from_organization(&org);

        assert!(profile.merge_record(&record(
            ReconRecordKind::App,
            "平安金管家",
            "intel.mobile_apps"
        )));
        assert!(profile.merge_record(&record(
            ReconRecordKind::MiniProgram,
            "平安好车主",
            "intel.mini_programs",
        )));
        assert!(profile.merge_record(&record(
            ReconRecordKind::Wechat,
            "pingan",
            "social_accounts",
        )));
        assert!(profile.merge_record(&record(
            ReconRecordKind::Contact,
            "security@example.com",
            "contacts.email",
        )));

        assert_eq!(profile.intel["mobile_apps"], json!(["平安金管家"]));
        assert_eq!(profile.intel["mini_programs"], json!(["平安好车主"]));
        assert_eq!(profile.social_accounts, json!(["pingan"]));
        assert_eq!(profile.contacts["email"], json!(["security@example.com"]));
    }

    #[test]
    fn target_type_rejects_public_code_host_outside_org_domains() {
        let org = golish_db::models::Organization {
            id: Uuid::new_v4(),
            project_path: "/tmp/project".into(),
            name: "Org".into(),
            parent_id: None,
            description: String::new(),
            owner: String::new(),
            sort_order: 0,
            aliases: Vec::new(),
            industry: String::new(),
            tier: String::new(),
            credit_code: String::new(),
            domains: json!(["pingan.com.cn"]),
            ip_ranges: json!([]),
            asns: json!([]),
            email_domains: json!(["126.com"]),
            scope_rules: json!({}),
            intel: json!({ "app_domains": ["app.pingan.com.cn"] }),
            notes: String::new(),
            certificates: json!([]),
            subsidiaries: json!([]),
            business_systems: json!([]),
            cloud_assets: json!([]),
            github_orgs: json!([]),
            social_accounts: json!([]),
            historical_vulns: json!([]),
            contacts: json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        assert_eq!(
            target_type_for_record(
                &org,
                &record(
                    ReconRecordKind::Url,
                    "https://github.com/example/leak/blob/main/key.txt",
                    ""
                )
            ),
            None
        );
        assert_eq!(
            target_type_for_record(&org, &record(ReconRecordKind::Domain, "126.com", "")),
            None
        );
        assert_eq!(
            target_type_for_record(
                &org,
                &record(ReconRecordKind::Url, "https://www.pingan.com.cn/", "")
            ),
            Some("url")
        );

        let passive_ip = record(ReconRecordKind::Ip, "203.0.113.42", "");
        assert_eq!(
            target_type_for_record(&org, &passive_ip),
            None,
            "a passive IP observation must not become an executable in-scope target"
        );
        let mut profile = ProfileAccumulator::from_organization(&org);
        assert!(
            !profile.merge_record(&passive_ip),
            "a rejected passive IP must not backfill organization.ip_ranges and authorize itself later"
        );
    }

    #[test]
    fn confirmed_org_ip_ranges_authorize_only_exact_or_contained_ips() {
        let mut org = org_with_domains(json!(["example.com"]));
        org.ip_ranges = json!([
            "203.0.113.42",
            "198.51.100.0/24",
            "2001:db8::/32",
            "malformed"
        ]);

        assert!(organization_ip_ranges_contain_ip(
            &org,
            "203.0.113.42".parse().unwrap()
        ));
        assert!(organization_ip_ranges_contain_ip(
            &org,
            "198.51.100.255".parse().unwrap()
        ));
        assert!(organization_ip_ranges_contain_ip(
            &org,
            "2001:db8:abcd::1".parse().unwrap()
        ));
        assert!(!organization_ip_ranges_contain_ip(
            &org,
            "198.51.101.1".parse().unwrap()
        ));
        assert!(!organization_ip_ranges_contain_ip(
            &org,
            "2001:db9::1".parse().unwrap()
        ));
    }

    #[test]
    fn authorized_target_rows_require_exact_ip_or_containing_cidr() {
        let rows = vec![
            ("ip".to_string(), "203.0.113.42".to_string()),
            ("cidr".to_string(), "198.51.100.0/24".to_string()),
            ("domain".to_string(), "192.0.2.10".to_string()),
            ("cidr".to_string(), "2001:db8::/32".to_string()),
            ("cidr".to_string(), "malformed".to_string()),
        ];

        assert!(authorized_target_rows_contain_ip(
            "203.0.113.42".parse().unwrap(),
            &rows
        ));
        assert!(authorized_target_rows_contain_ip(
            "198.51.100.88".parse().unwrap(),
            &rows
        ));
        assert!(authorized_target_rows_contain_ip(
            "2001:db8::42".parse().unwrap(),
            &rows
        ));
        assert!(!authorized_target_rows_contain_ip(
            "203.0.113.43".parse().unwrap(),
            &rows
        ));
        assert!(!authorized_target_rows_contain_ip(
            "192.0.2.10".parse().unwrap(),
            &rows
        ));
    }

    #[test]
    fn authorized_ip_context_query_is_strictly_owned_scoped_and_project_bound() {
        assert!(AUTHORIZED_IP_CONTEXT_SQL.contains("organization_id = $1"));
        assert!(AUTHORIZED_IP_CONTEXT_SQL.contains("project_path IS NOT DISTINCT FROM $2"));
        assert!(AUTHORIZED_IP_CONTEXT_SQL.contains("scope::text = 'in'"));
        assert!(AUTHORIZED_IP_CONTEXT_SQL.contains("target_type::text IN ('ip', 'cidr')"));
        assert!(
            !AUTHORIZED_IP_CONTEXT_SQL.contains("organization_id IS NULL"),
            "legacy/null-org rows must never authorize active IP promotion"
        );
    }

    #[test]
    fn persistence_summary_serializes_per_record_results() {
        let domain = record(ReconRecordKind::Domain, "PingAn.COM", "domains");
        let app = record(ReconRecordKind::App, "平安金管家", "intel.mobile_apps");
        let port = record(ReconRecordKind::Port, "example.com:443/tcp", "");
        let mut summary = PersistenceSummary {
            record_count: 3,
            ..PersistenceSummary::default()
        };

        summary.target_inserted += 1;
        summary.push_result(
            &domain,
            PersistenceRecordStatus::Inserted,
            "target_insert",
            Some("domain"),
            None,
        );
        summary.profile_updates += 1;
        summary.push_result(
            &app,
            PersistenceRecordStatus::ProfileUpdated,
            "organization_profile_merge",
            None,
            None,
        );
        summary.unsupported_records += 1;
        summary.push_result(
            &port,
            PersistenceRecordStatus::Unsupported,
            "unsupported_record",
            None,
            Some("no persistence mapping for port record".into()),
        );

        let json = serde_json::to_value(&summary).unwrap();

        assert_eq!(summary.record_results.len(), summary.record_count);
        assert_eq!(json["recordResults"][0]["status"], "inserted");
        assert_eq!(json["recordResults"][0]["targetType"], "domain");
        assert_eq!(json["recordResults"][1]["status"], "profile_updated");
        assert_eq!(json["recordResults"][2]["status"], "unsupported");
        assert_eq!(
            json["recordResults"][2]["error"],
            "no persistence mapping for port record"
        );
    }

    fn org_with_domains(domains: Value) -> golish_db::models::Organization {
        golish_db::models::Organization {
            id: Uuid::new_v4(),
            project_path: "/tmp/project".into(),
            name: "Org".into(),
            parent_id: None,
            description: String::new(),
            owner: String::new(),
            sort_order: 0,
            aliases: Vec::new(),
            industry: String::new(),
            tier: String::new(),
            credit_code: String::new(),
            domains,
            ip_ranges: json!([]),
            asns: json!([]),
            email_domains: json!([]),
            scope_rules: json!([]),
            intel: json!({}),
            notes: String::new(),
            certificates: json!([]),
            subsidiaries: json!([]),
            business_systems: json!([]),
            cloud_assets: json!([]),
            github_orgs: json!([]),
            social_accounts: json!([]),
            historical_vulns: json!([]),
            contacts: json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn collect_subdomain_pairs_maps_owned_subdomains_to_root() {
        let org = org_with_domains(json!(["pingan.com"]));
        let hosts: Vec<String> = [
            "life.pingan.com",
            "stock.pingan.com",
            // root itself → not a subdomain of itself
            "pingan.com",
            // www remains a distinct exact hostname below the authorized root.
            "www.pingan.com",
            // duplicate → deduped
            "life.pingan.com",
            // unrelated apex (not a subdomain of an owned root) → dropped
            "notpingan.com",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let mut pairs = collect_subdomain_pairs(&org, &hosts);
        pairs.sort();
        assert_eq!(
            pairs,
            vec![
                ("pingan.com".to_string(), "life.pingan.com".to_string()),
                ("pingan.com".to_string(), "stock.pingan.com".to_string()),
                ("pingan.com".to_string(), "www.pingan.com".to_string()),
            ]
        );
    }

    #[test]
    fn collect_subdomain_pairs_prefers_longest_owned_root() {
        let org = org_with_domains(json!(["pingan.com", "sub.pingan.com"]));
        let hosts = vec!["a.sub.pingan.com".to_string()];
        assert_eq!(
            collect_subdomain_pairs(&org, &hosts),
            vec![("sub.pingan.com".to_string(), "a.sub.pingan.com".to_string())]
        );
    }

    #[test]
    fn collect_subdomain_pairs_empty_without_owned_roots() {
        let org = org_with_domains(json!([]));
        let hosts = vec!["life.pingan.com".to_string()];
        assert!(collect_subdomain_pairs(&org, &hosts).is_empty());
    }

    #[test]
    fn wildcard_scope_matches_children_but_never_authorizes_apex() {
        let org = org_with_domains(json!(["*.moresec.cn"]));
        assert!(value_belongs_to_organization(&org, "www.moresec.cn"));
        assert!(value_belongs_to_organization(&org, "a.www.moresec.cn"));
        assert!(!value_belongs_to_organization(&org, "moresec.cn"));
        assert_eq!(
            collect_subdomain_pairs(
                &org,
                &[
                    "*.moresec.cn".to_string(),
                    "*.sub.moresec.cn".to_string(),
                    "moresec.cn".to_string(),
                    "www.moresec.cn".to_string(),
                ]
            ),
            vec![("*.moresec.cn".to_string(), "www.moresec.cn".to_string())]
        );
    }

    #[test]
    fn pair_subdomains_within_maps_subdomains_to_in_scope_root() {
        // The agent registers roots AND discovered subdomains as scope='in'
        // targets; pairing within that set recovers the (root, subdomain) edges
        // the enrich path can't (same-source skip-all — see fn doc). IPs/apexes
        // are nobody's subdomain → no pair; `www.` remains an exact host asset.
        let hosts: Vec<String> = [
            "pa18.com",
            "pingan.com",
            "pingan.cn",
            "pingan.com.cn",
            "um.pa18.com",
            "act.pa18.com",
            "sub.pingan.com",
            "www.pingan.com",
            "202.69.26.13", // IP → never a subdomain → no pair
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let mut pairs = pair_subdomains_within(&hosts);
        pairs.sort();
        assert_eq!(
            pairs,
            vec![
                ("pa18.com".to_string(), "act.pa18.com".to_string()),
                ("pa18.com".to_string(), "um.pa18.com".to_string()),
                ("pingan.com".to_string(), "sub.pingan.com".to_string()),
                ("pingan.com".to_string(), "www.pingan.com".to_string()),
            ]
        );
    }

    #[test]
    fn normalized_host_preserves_www_identity() {
        assert_eq!(
            normalized_host("WWW.MoreSec.CN."),
            Some("www.moresec.cn".to_string())
        );
        assert_eq!(
            normalized_host("https://WWW.MoreSec.CN/path"),
            Some("www.moresec.cn".to_string())
        );
        assert_eq!(
            normalized_host("*.MoreSec.CN."),
            Some("*.moresec.cn".to_string())
        );
    }

    #[test]
    fn persistence_target_lookup_is_org_type_scoped_with_legacy_claim() {
        let sql = build_persist_target_lookup_sql();
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
    }

    #[test]
    fn pair_subdomains_within_prefers_longest_parent_and_dedupes() {
        let hosts: Vec<String> = ["a.com", "sub.a.com", "x.sub.a.com", "x.sub.a.com"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut pairs = pair_subdomains_within(&hosts);
        pairs.sort();
        assert_eq!(
            pairs,
            vec![
                ("a.com".to_string(), "sub.a.com".to_string()),
                ("sub.a.com".to_string(), "x.sub.a.com".to_string()),
            ]
        );
    }

    #[test]
    fn pair_subdomains_within_empty_when_only_apexes() {
        let hosts = vec!["a.com".to_string(), "b.com".to_string()];
        assert!(pair_subdomains_within(&hosts).is_empty());
    }

    #[test]
    fn registrable_domains_skips_ip_garbage_and_keeps_real_roots() {
        // Regression: an authorized-host input polluted with URL-wrapped IPs made
        // `registrable_domains` return IP fragments (`124.196.77.48` -> `77.48`),
        // so RDAP was queried for junk. IP hosts must be skipped, leaving the
        // real authorized roots for the WHOIS query.
        let domains = registrable_domains_from_authorized_hosts(
            &[
                "http://124.196.77.48", // url-wrapped IP -> host 124.196.77.48 -> skip
                "https://61.241.22.10", // url-wrapped IP -> skip
                "pingan.com",
                "life.pingan.com", // -> apex pingan.com (deduped)
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>(),
        );
        assert!(
            domains.contains(&"pingan.com".to_string()),
            "real root must be queried: {domains:?}"
        );
        assert!(
            !domains.iter().any(|d| d == "77.48" || d == "22.10"),
            "IP fragments must not appear: {domains:?}"
        );
        assert!(
            !domains
                .iter()
                .any(|d| d.parse::<std::net::IpAddr>().is_ok()),
            "no IP literal may be CT-queried: {domains:?}"
        );
    }

    #[test]
    fn registrable_domains_include_only_authorized_target_values() {
        let domains = registrable_domains_from_authorized_hosts(&[
            "moresec.cn".to_string(),
            "https://www.moresec.cn/login".to_string(),
            "115.28.135.55".to_string(),
        ]);
        assert_eq!(domains, vec!["moresec.cn".to_string()]);
    }

    #[test]
    fn registrable_domain_handles_two_level_tlds() {
        assert_eq!(registrable_domain("life.pingan.com"), "pingan.com");
        assert_eq!(registrable_domain("pingan.com"), "pingan.com");
        assert_eq!(registrable_domain("a.b.pingan.com.cn"), "pingan.com.cn");
        assert_eq!(registrable_domain("pingan.com.cn"), "pingan.com.cn");
        assert_eq!(registrable_domain("example.org"), "example.org");
        // ③ 修复回归：ccTLD 组织类二级域（`.ne.jp`）现在正确折到注册 apex，
        // CT/WHOIS 查询目标与 gate 的 SUBDOMAIN 判定共用同一套表、不再漂移。
        assert_eq!(registrable_domain("s.example.ne.jp"), "example.ne.jp");
        assert_eq!(registrable_domain("example.ne.jp"), "example.ne.jp");
    }

    #[test]
    fn json_value_is_empty_matches_coverage_semantics() {
        assert!(json_value_is_empty(&json!(null)));
        assert!(json_value_is_empty(&json!([])));
        assert!(json_value_is_empty(&json!({})));
        assert!(json_value_is_empty(&json!("  ")));
        assert!(!json_value_is_empty(&json!(["AS1"])));
        assert!(!json_value_is_empty(&json!({ "handle": "X" })));
    }

    #[test]
    fn whois_terminal_state_distinguishes_empty_error_and_blocked() {
        assert_eq!(
            classify_whois_landing(0, 0, false),
            WhoisLandingState::Blocked
        );
        assert_eq!(
            classify_whois_landing(2, 0, false),
            WhoisLandingState::Error
        );
        assert_eq!(
            classify_whois_landing(2, 1, false),
            WhoisLandingState::Error
        );
        assert_eq!(classify_whois_landing(2, 1, true), WhoisLandingState::Error);
        assert_eq!(
            classify_whois_landing(2, 2, false),
            WhoisLandingState::Empty
        );
        assert_eq!(classify_whois_landing(2, 2, true), WhoisLandingState::Found);
    }

    #[test]
    fn normalize_dns_target_strips_root_dot_and_lowercases() {
        // CNAME/MX answers arrive FQDN-style (trailing root dot) and mixed-case;
        // collapse them so duplicate rows hit the dns_records unique key.
        assert_eq!(normalize_dns_target("CDN.Example.COM."), "cdn.example.com");
        assert_eq!(
            normalize_dns_target("  alias.example.net  "),
            "alias.example.net"
        );
        assert_eq!(normalize_dns_target("."), "");
    }

    #[test]
    fn deterministic_primary_dns_ip_is_ipv4_first_and_order_independent() {
        let records = vec![
            (
                "AAAA",
                "www.moresec.cn".to_string(),
                "2001:db8::2".to_string(),
            ),
            (
                "A",
                "www.moresec.cn".to_string(),
                "203.0.113.20".to_string(),
            ),
            (
                "A",
                "www.moresec.cn".to_string(),
                "203.0.113.10".to_string(),
            ),
        ];
        assert_eq!(
            deterministic_primary_dns_ip(&records),
            Some("203.0.113.10".to_string())
        );
    }

    #[test]
    fn dns_host_attempt_does_not_treat_query_error_as_empty() {
        assert_eq!(
            classify_dns_host_attempt(&[
                DnsAttemptState::Empty,
                DnsAttemptState::Empty,
                DnsAttemptState::Error,
                DnsAttemptState::Empty,
            ]),
            DnsAttemptState::Error,
            "one resolver/transport error makes an otherwise empty host non-terminal"
        );
        assert_eq!(
            classify_dns_host_attempt(&[DnsAttemptState::Empty; DNS_QUERY_GROUP_COUNT]),
            DnsAttemptState::Empty
        );
        assert_eq!(
            classify_dns_host_attempt(&[
                DnsAttemptState::Found,
                DnsAttemptState::Empty,
                DnsAttemptState::Error,
                DnsAttemptState::Empty,
            ]),
            DnsAttemptState::Partial,
            "real A/AAAA data must land without hiding an auxiliary query failure"
        );
    }

    #[test]
    fn typed_a_and_aaaa_negatives_can_close_the_address_group() {
        assert_eq!(
            classify_dns_address_attempts(&[DnsAttemptState::Empty, DnsAttemptState::Empty]),
            DnsAttemptState::Empty,
            "two explicit no-record answers prove the address group was checked empty"
        );
        assert_eq!(
            classify_dns_address_attempts(&[DnsAttemptState::Empty, DnsAttemptState::Error]),
            DnsAttemptState::Error,
            "a transport error cannot be promoted to a negative DNS fact"
        );
        assert_eq!(
            classify_dns_address_attempts(&[]),
            DnsAttemptState::Error,
            "an unavailable typed resolver leaves OS lookup as positive-only fallback"
        );
        assert_eq!(
            classify_dns_address_attempts(&[DnsAttemptState::Found, DnsAttemptState::Error]),
            DnsAttemptState::Partial,
            "a real address lands, but its failed sibling family remains retryable"
        );
    }

    #[test]
    fn resolv_conf_fallback_skips_unrepresentable_scoped_nameserver() {
        let servers = resolv_conf_nameservers(
            "nameserver fe80::5e7d:aeff:fe2d:4f3d%en0\n\
             nameserver 192.168.0.1\n\
             nameserver 2001:4860:4860::8888\n",
        );
        assert_eq!(
            servers,
            vec![
                "192.168.0.1".parse::<IpAddr>().unwrap(),
                "2001:4860:4860::8888".parse::<IpAddr>().unwrap()
            ]
        );
    }

    #[test]
    fn dns_refresh_has_no_fixed_target_limit_and_batches_every_domain() {
        let sql = dns_target_query_sql();
        assert!(!sql.to_ascii_uppercase().contains("LIMIT"));
        assert!(sql.contains("ORDER BY t.created_at ASC, t.value ASC, t.id ASC"));

        let targets = (0..401).collect::<Vec<_>>();
        let batch_sizes = targets
            .chunks(DNS_MAX_RESOLVE_CONCURRENCY)
            .map(<[_]>::len)
            .collect::<Vec<_>>();
        assert_eq!(batch_sizes, vec![128, 128, 128, 17]);
        assert_eq!(batch_sizes.iter().sum::<usize>(), targets.len());
    }

    #[test]
    fn format_mx_value_keeps_preference_and_normalizes_host() {
        assert_eq!(
            format_mx_value(10, "MAIL.Example.com."),
            "10 mail.example.com"
        );
        // RFC 7505 null MX ("0 .") — exchange normalizes to empty, preference kept.
        assert_eq!(format_mx_value(0, "."), "0 ");
    }
}
