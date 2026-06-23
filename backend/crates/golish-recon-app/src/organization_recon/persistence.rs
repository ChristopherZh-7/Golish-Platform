use std::collections::{HashMap, HashSet};

use golish_app_core::GolishError;
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
        if let Some(target_type) = target_type_for_record(organization, record) {
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
    let landed = land_target_intel_coverage(pool, organization, run_id, &subdomain_hosts).await;
    tracing::info!(
        organization_id = %organization.id,
        subdomains = landed.subdomains,
        "target_intel coverage landing (org-recon path)"
    );

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
) -> CoverageLandingSummary {
    let subdomains = land_subdomain_assets(pool, organization, run_id, subdomain_hosts)
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(
                organization_id = %organization.id,
                %error,
                "subdomain target_assets landing failed (recon persistence already committed)"
            );
            0
        });
    CoverageLandingSummary { subdomains }
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
    let Ok(Some(org)) = golish_db::repo::organizations::get_one(pool, org_id).await else {
        return (0, 0);
    };
    let subdomains = land_subdomain_assets(pool, &org, "gate-refresh", &[])
        .await
        .unwrap_or(0);
    let dns_records = land_dns_records(pool, &org).await.unwrap_or(0);
    (subdomains, dns_records)
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
        // Host that *is* an owned root is the asset itself, not a subdomain of it.
        if roots.iter().any(|root| root == &host) {
            continue;
        }
        let Some(root) = roots
            .iter()
            .filter(|root| host.ends_with(&format!(".{root}")))
            .max_by_key(|root| root.len())
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
/// This is the in-scope-`targets` source for subdomain landing. The agent's
/// subfinder/amass discoveries are registered as `scope='in'` target rows (not
/// into the junk `organizations.domains` OSINT list), so `collect_subdomain_pairs`
/// — whose roots AND candidate hosts both come from `organizations.domains` — pairs
/// nothing for the agent enrich path (every host equals an owned root ⇒ skipped:
/// the "same-source" landing gap). Pairing within the in-scope target set recovers
/// the real `(root, subdomain)` edges the coverage gate reads from `target_assets`.
fn pair_subdomains_within(hosts: &[String]) -> Vec<(String, String)> {
    let norm: Vec<String> = hosts.iter().filter_map(|h| normalized_host(h)).collect();
    let mut seen = HashSet::new();
    let mut pairs = Vec::new();
    for host in &norm {
        let Some(root) = norm
            .iter()
            .filter(|root| *root != host && host.ends_with(&format!(".{root}")))
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
    let mut pair_set: HashSet<(String, String)> =
        collect_subdomain_pairs(organization, subdomain_hosts)
            .into_iter()
            .collect();
    // Source 2 (fix 2026-06-16 enrich-same-source): the in-scope `targets` the
    // agent actually registered — subfinder/amass discoveries land there as
    // scope='in' rows, NOT into the junk `organizations.domains` OSINT list that
    // self-cancels in `collect_subdomain_pairs`. Seed the root→target_id cache from
    // them, then pair within the set so each discovered subdomain lands as a
    // `target_assets(asset_type='subdomain')` child of its in-scope root.
    let in_scope: Vec<(Uuid, String)> = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT id, value FROM targets WHERE scope::text = 'in' AND organization_id = $1",
    )
    .bind(organization.id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let mut root_targets: HashMap<String, Option<Uuid>> = HashMap::new();
    for (id, value) in &in_scope {
        if let Some(host) = normalized_host(value) {
            root_targets.entry(host).or_insert(Some(*id));
        }
    }
    let in_scope_values: Vec<String> = in_scope.into_iter().map(|(_, value)| value).collect();
    for pair in pair_subdomains_within(&in_scope_values) {
        pair_set.insert(pair);
    }
    if pair_set.is_empty() {
        return Ok(0);
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
                         AND project_path IS NOT DISTINCT FROM $2
                       ORDER BY (scope::text = 'in') DESC, updated_at DESC
                       LIMIT 1"#,
                )
                .bind(&root)
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
        match golish_db::repo::target_assets::upsert(
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
        .await
        {
            Ok(_) => {
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
                if let Err(e) = golish_db::repo::technique_outcomes::upsert(pool, &w).await {
                    tracing::warn!(
                        %subdomain,
                        error = %e,
                        "technique_outcomes upsert (enrich subdomain landing) failed"
                    );
                }
            }
            Err(error) => tracing::warn!(
                %root,
                %subdomain,
                %error,
                "target_assets subdomain upsert failed"
            ),
        }
    }
    Ok(landed)
}

/// Resolve in-scope **domain** targets of this org that have no DNS record yet and
/// land their A/AAAA answers into `dns_records` — the table the target_intel DNS
/// coverage cell reads (`dns_records::present_target_values` /
/// `coverage_truth`). enrich records carry domains and IPs unpaired, so we do a
/// bounded best-effort resolve (the honest way to produce real A records). DNS
/// resolution is standard `recon/dns`; failures/timeouts are skipped, never fatal.
async fn land_dns_records(
    pool: &sqlx::PgPool,
    organization: &golish_db::models::Organization,
) -> Result<usize, GolishError> {
    const MAX_RESOLVE: i64 = 128;
    let targets: Vec<(Uuid, String)> = sqlx::query_as(
        r#"SELECT t.id, t.value FROM targets t
           WHERE t.organization_id = $1
             AND t.scope::text = 'in'
             AND t.target_type::text = 'domain'
             AND NOT EXISTS (SELECT 1 FROM dns_records dr WHERE dr.target_id = t.id)
           ORDER BY t.updated_at DESC
           LIMIT $2"#,
    )
    .bind(organization.id)
    .bind(MAX_RESOLVE)
    .fetch_all(pool)
    .await?;
    if targets.is_empty() {
        return Ok(0);
    }
    let mut set = tokio::task::JoinSet::new();
    for (target_id, value) in targets {
        set.spawn(async move {
            let host = normalized_host(&value)?;
            let lookup = tokio::time::timeout(
                std::time::Duration::from_secs(3),
                tokio::net::lookup_host(format!("{host}:0")),
            )
            .await
            .ok()?
            .ok()?;
            let records: Vec<(Uuid, &'static str, String, String)> = lookup
                .map(|addr| {
                    let ip = addr.ip();
                    let record_type = if ip.is_ipv4() { "A" } else { "AAAA" };
                    (target_id, record_type, host.clone(), ip.to_string())
                })
                .collect();
            Some(records)
        });
    }
    let mut landed = 0usize;
    while let Some(joined) = set.join_next().await {
        let Ok(Some(records)) = joined else {
            continue;
        };
        // Primary IP for the host tree (design 2026-06-15 Phase 0): first IPv4 (A)
        // answer, else the first answer. Captured before the consuming upsert loop
        // below moves `records`.
        let primary_ip: Option<(Uuid, String)> = records
            .iter()
            .find(|(_, record_type, _, _)| *record_type == "A")
            .or_else(|| records.first())
            .map(|(target_id, _, _, ip)| (*target_id, ip.clone()));
        for (target_id, record_type, name, value) in records {
            if golish_db::repo::dns_records::upsert(
                pool,
                target_id,
                organization.project_path.as_str(),
                record_type,
                &name,
                &value,
                "resolver",
            )
            .await
            .is_ok()
            {
                landed += 1;
            }
        }
        if let Some((target_id, ip)) = primary_ip {
            let _ = golish_db::repo::targets::set_real_ip_by_id(pool, target_id, &ip).await;
        }
    }
    Ok(landed)
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

/// Unique registrable apex domains owned by the org (capped), for CT/WHOIS queries.
fn registrable_domains(organization: &golish_db::models::Organization) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for host in organization_owned_domains(organization) {
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
        if !apex.is_empty() && !out.contains(&apex) {
            out.push(apex);
        }
        if out.len() >= 3 {
            break;
        }
    }
    out
}

/// Land WHOIS (RDAP) → `organizations.whois` — the org-level column the
/// target_intel WHOIS coverage cell reads (`coverage_truth::build_org_intel_
/// presence_sql`). HTTP, bounded, best-effort: only fills when the column is
/// currently empty; failures are skipped, never fatal. (`whois` is a schema-ahead
/// column not on the model, so it is read/written via direct SQL.) Returns whether
/// a whois object was landed.
///
/// CT (crt.sh) is intentionally NOT done here anymore: crt.sh was the 300s-timeout
/// culprit and only produced junk for polluted registrable domains. CT now comes
/// from the `ctfr` tool (crt.sh) + fofa native cert. WHOIS is exposed to the agent
/// as the standalone `recon_lookup_whois` tool. (plan 2026-06-18-slim-enrich)
pub(crate) async fn land_whois(
    pool: &sqlx::PgPool,
    organization: &golish_db::models::Organization,
) -> Result<bool, GolishError> {
    let whois_existing: Option<Value> =
        sqlx::query_scalar::<_, Option<Value>>("SELECT whois FROM organizations WHERE id = $1")
            .bind(organization.id)
            .fetch_one(pool)
            .await?;
    if whois_existing
        .as_ref()
        .is_some_and(|v| !json_value_is_empty(v))
    {
        return Ok(false);
    }
    let domains = registrable_domains(organization);
    if domains.is_empty() {
        return Ok(false);
    }
    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("golish-recon/1.0")
        .build()
    else {
        return Ok(false);
    };

    let mut whois_value: Option<Value> = None;
    for domain in &domains {
        let url = format!("https://rdap.org/domain/{domain}");
        let Ok(resp) = client.get(&url).send().await else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        let Ok(text) = resp.text().await else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<Value>(&text) {
            if value.is_object() && !json_value_is_empty(&value) {
                whois_value = Some(value);
                break;
            }
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
    Ok(whois_landed)
}

fn target_type_for_record(
    organization: &golish_db::models::Organization,
    record: &NormalizedReconRecord,
) -> Option<&'static str> {
    match record.kind {
        ReconRecordKind::Domain if record_belongs_to_organization(organization, record) => {
            Some("domain")
        }
        ReconRecordKind::Ip => Some("ip"),
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
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
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
    if let Ok(url) = url::Url::parse(&value) {
        return url
            .host_str()
            .map(|host| host.trim_start_matches("www.").to_string());
    }
    if looks_like_domain(&value) {
        return Some(value.trim_start_matches("www.").to_string());
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
    let existing: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT id FROM targets
           WHERE value = $1
             AND project_path IS NOT DISTINCT FROM $2
           LIMIT 1"#,
    )
    .bind(&record.value)
    .bind(&organization.project_path)
    .fetch_optional(&mut **tx)
    .await?;

    if let Some(id) = existing {
        sqlx::query(
            r#"UPDATE targets
               SET organization_id = COALESCE(organization_id, $2),
                   updated_at = NOW()
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(organization.id)
        .execute(&mut **tx)
        .await?;
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
            ReconRecordKind::Ip => push_json_array(&mut self.ip_ranges, &record.value),
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
            // www is normalized to the root → dropped
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
    fn pair_subdomains_within_maps_subdomains_to_in_scope_root() {
        // The agent registers roots AND discovered subdomains as scope='in'
        // targets; pairing within that set recovers the (root, subdomain) edges
        // the enrich path can't (same-source skip-all — see fn doc). IPs/apexes
        // are nobody's subdomain → no pair; `www.` is normalized to the apex
        // (consistent with collect_subdomain_pairs) → dropped, not an asset.
        let hosts: Vec<String> = [
            "pa18.com",
            "pingan.com",
            "pingan.cn",
            "pingan.com.cn",
            "um.pa18.com",
            "act.pa18.com",
            "sub.pingan.com",
            "www.pingan.com", // normalizes to pingan.com (apex) → dropped
            "202.69.26.13",   // IP → never a subdomain → no pair
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
            ]
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
        // Regression: a domains list polluted with URL-wrapped IPs made
        // `registrable_domains` return IP fragments (`124.196.77.48` -> `77.48`),
        // so crt.sh was queried for junk and CT never landed. IP hosts must be
        // skipped, leaving the real owned roots for the CT/WHOIS query.
        let org = org_with_domains(json!([
            "http://124.196.77.48", // url-wrapped IP -> host 124.196.77.48 -> skip
            "https://61.241.22.10", // url-wrapped IP -> skip
            "pingan.com",
            "life.pingan.com", // -> apex pingan.com (deduped)
        ]));
        let domains = registrable_domains(&org);
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
}
