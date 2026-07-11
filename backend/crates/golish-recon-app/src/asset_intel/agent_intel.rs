//! Agent-facing facade over the passive asset-intel engine.
//!
//! Wraps the inner pipeline used by the GUI commands
//! (`asset_intel_hydrate_subsidiaries` / `asset_intel_enrich_organization`) so an
//! agent `Tool` can drive subsidiary discovery + field enrichment without going
//! through the Tauri command layer (设计 2026-06-06-intel-stage-ai-driven-per-mode
//! §3.3). It scans the tools-config, selects the right provider phase, runs the
//! providers against one organization, and returns a small serializable summary
//! the agent tool can hand back (and the runtime can book to the evidence ledger).

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use golish_app_core::GolishError;
use golish_pentest_domain::{canonical_asset_key, registrable_apex, AssetClass};

use crate::organizations::OrganizationCandidates;

use super::{
    apply_ownership_threshold_override, auto_promote_discovered_children, parse_ownership_percent,
    run_providers_for_org, select_discovery_policy, select_enrichment_providers,
    select_subsidiary_providers, AssetIntelHydrateConfig, AssetIntelProviderRunStatus,
    ToolsConfigState,
};

/// Keep automatic apex expansion bounded: it is passive/zero-touch, but each
/// root can fan out to several provider requests.
const AUTO_DOMAIN_EXPANSION_LIMIT: usize = 5;

/// Which passive provider phase to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassiveIntelPhase {
    /// Discovery: providers with a `subsidiaries` capability (enscan-go).
    Subsidiaries,
    /// Enrichment: providers without `subsidiaries` (0.zone / quake / fofa / …).
    Enrich,
}

impl PassiveIntelPhase {
    fn as_str(self) -> &'static str {
        match self {
            PassiveIntelPhase::Subsidiaries => "subsidiaries",
            PassiveIntelPhase::Enrich => "enrich",
        }
    }
}

/// One discovered subsidiary candidate surfaced for human review. With
/// `auto_promote` off, nothing is created until the user picks: the agent passes
/// these into `ask_human(unit_review)`. `meets_threshold` is computed against the
/// human-chosen `min_ownership_percent` (default 51) so the agent can pre-select.
#[derive(Debug, Clone, Serialize)]
pub struct SubsidiaryCandidate {
    pub name: String,
    pub ownership_percent: Option<String>,
    pub status: Option<String>,
    pub meets_threshold: bool,
}

/// One automatic domain-keyed expansion spawned by a normal `recon_map_assets`
/// org/company survey.
#[derive(Debug, Clone, Serialize)]
pub struct DomainExpansionSummary {
    pub domain: String,
    pub run_id: String,
    pub status: String,
    pub targets: usize,
    pub providers: Vec<String>,
    /// Nested provider status for `source_query_log` with `target=<domain>`.
    #[serde(default, rename = "providerStatus")]
    pub provider_status: Vec<AssetIntelProviderRunStatus>,
}

/// Serializable result of one passive-intel run, returned by the agent tool and
/// booked to the evidence ledger.
#[derive(Debug, Clone, Serialize)]
pub struct PassiveIntelSummary {
    pub run_id: String,
    pub company: String,
    pub phase: &'static str,
    /// `Completed` / `Partial` / `Failed`.
    pub status: String,
    pub organizations: usize,
    pub targets: usize,
    /// Provider ids that were selected and run for this phase.
    pub providers: Vec<String>,
    /// Per-provider terminal status. This is source-level audit metadata for
    /// `source_query_log`; it is not a completeness proof by itself.
    #[serde(default, rename = "providerStatus")]
    pub provider_status: Vec<AssetIntelProviderRunStatus>,
    /// Phase 2: number of discovered subsidiaries auto-promoted to child orgs
    /// (subsidiaries phase only; 0 for enrich or when no candidate qualified).
    pub promoted_children: usize,
    /// Subsidiaries phase with auto_promote OFF: the discovered candidates so the
    /// agent can pass them into `ask_human(unit_review)` for the user to pick.
    /// Empty for enrich or when candidates were auto-promoted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subsidiaries: Vec<SubsidiaryCandidate>,
    /// Enrich phase only: roots automatically expanded in domain-keyed mode
    /// after the broad org/company survey discovered owned domains.
    #[serde(
        default,
        rename = "domainExpansions",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub domain_expansions: Vec<DomainExpansionSummary>,
}

/// Run one passive-intel phase (subsidiary discovery or field enrichment)
/// against a single organization, reusing the production asset-intel engine.
///
/// Candidates + master-record profile fields are written back to the org by the
/// engine (`run_providers_for_org`); this returns only a summary.
pub async fn run_passive_intel(
    pool: Arc<sqlx::PgPool>,
    tools: ToolsConfigState,
    organization_id: Uuid,
    phase: PassiveIntelPhase,
    config: AssetIntelHydrateConfig,
) -> Result<PassiveIntelSummary, GolishError> {
    let should_auto_expand_domains = phase == PassiveIntelPhase::Enrich
        && config
            .domain
            .as_deref()
            .map(str::trim)
            .is_none_or(str::is_empty);

    let mut summary = run_passive_intel_once(
        Arc::clone(&pool),
        tools.clone(),
        organization_id,
        phase,
        config.clone(),
    )
    .await?;

    if should_auto_expand_domains {
        let roots =
            match golish_db::repo::organizations::get_one(pool.as_ref(), organization_id).await {
                Ok(Some(fresh)) => domain_expansion_roots(&fresh, AUTO_DOMAIN_EXPANSION_LIMIT),
                Ok(None) => Vec::new(),
                Err(error) => {
                    tracing::warn!(%error, "reload org for passive-intel domain expansion failed");
                    Vec::new()
                }
            };

        for domain in roots {
            let mut expansion_config = config.clone();
            expansion_config.domain = Some(domain.clone());
            match run_passive_intel_once(
                Arc::clone(&pool),
                tools.clone(),
                organization_id,
                phase,
                expansion_config,
            )
            .await
            {
                Ok(expansion) => {
                    tracing::info!(
                        domain = %domain,
                        run_id = %expansion.run_id,
                        targets = expansion.targets,
                        "passive-intel automatic domain expansion complete"
                    );
                    summary.domain_expansions.push(DomainExpansionSummary {
                        domain,
                        run_id: expansion.run_id,
                        status: expansion.status,
                        targets: expansion.targets,
                        providers: expansion.providers,
                        provider_status: expansion.provider_status,
                    });
                }
                Err(error) => {
                    tracing::warn!(
                        domain = %domain,
                        %error,
                        "passive-intel automatic domain expansion failed"
                    );
                }
            }
        }
    }

    Ok(summary)
}

async fn run_passive_intel_once(
    pool: Arc<sqlx::PgPool>,
    tools: ToolsConfigState,
    organization_id: Uuid,
    phase: PassiveIntelPhase,
    config: AssetIntelHydrateConfig,
) -> Result<PassiveIntelSummary, GolishError> {
    let org = golish_db::repo::organizations::get_one(pool.as_ref(), organization_id)
        .await?
        .ok_or_else(|| GolishError::NotFound(format!("organization {organization_id}")))?;

    let pentest_config = tools.0.get().await;
    let scan = golish_pentest::scan_asset_intel_sources_with_status(
        &pentest_config.toolsconfig_dir,
        &pentest_config.intel_providers_dir,
        pentest_config.tools_dir(),
    );
    if !scan.success {
        return Err(GolishError::Internal(
            scan.error
                .unwrap_or_else(|| "toolsconfig scan failed".into()),
        ));
    }

    let selected = match phase {
        PassiveIntelPhase::Subsidiaries => select_subsidiary_providers(&scan.tools, &[])?,
        PassiveIntelPhase::Enrich => select_enrichment_providers(&scan.tools, &[])?,
    };
    if selected.is_empty() {
        return Err(GolishError::Validation(format!(
            "no asset-intel provider available for the '{}' phase (configure one in Integrations)",
            phase.as_str()
        )));
    }
    let provider_ids: Vec<String> = selected.iter().map(|tool| tool.id.clone()).collect();

    // Phase 2 (G1/G2): in the subsidiaries phase, capture the discovery policy
    // BEFORE `selected` is moved into `run_providers_for_org`, so we can promote
    // qualifying candidates to child orgs afterwards (mirrors the GUI path
    // `asset_intel_hydrate_subsidiaries`). Enrich phase keeps `None` (no promotion).
    let discovery_policy = (phase == PassiveIntelPhase::Subsidiaries).then(|| {
        let mut policy = select_discovery_policy(
            selected
                .iter()
                .filter_map(|tool| tool.asset_intel.as_ref())
                .map(|asset| &asset.discovery),
        );
        // Human-chosen ownership threshold (from the discover_subsidiaries tool
        // args) overrides the provider-config default so the scoping decision
        // actually drives auto-promotion.
        if let Some(threshold) = config.min_ownership_percent.as_deref() {
            apply_ownership_threshold_override(&mut policy, threshold);
        }
        policy
    });

    let mut run = run_providers_for_org(
        None,
        pool.as_ref(),
        &pentest_config,
        &scan.tools,
        selected,
        &org,
        &org.name,
        &config,
    )
    .await?;

    // Promote discovered subsidiaries that clear the ownership-percent threshold
    // into child organizations (parent_id = this org). The pure decision logic
    // (`auto_promote_child_decisions`) keeps the I8 distinction (ran-but-filtered
    // vs never-ran); a DB with no matching candidate never promotes.
    let promoted_children = match discovery_policy {
        Some(policy) if policy.auto_promote => {
            let promotion =
                auto_promote_discovered_children(pool.as_ref(), &org, &run.candidates, &policy)
                    .await?;
            run.candidates = OrganizationCandidates::default();
            promotion
                .get("created")
                .and_then(|c| c.as_array())
                .map(|a| a.len())
                .unwrap_or(0)
        }
        _ => 0,
    };

    // With auto_promote OFF the discovered subsidiaries stay as candidates; surface
    // them (name + ownership% + status, flagged against the human's threshold) so
    // the agent can show them in ask_human(unit_review) for the user to pick.
    let threshold = config
        .min_ownership_percent
        .as_deref()
        .and_then(parse_ownership_percent)
        .unwrap_or(51.0);
    let subsidiaries: Vec<SubsidiaryCandidate> = run
        .candidates
        .organizations
        .iter()
        .map(|c| {
            let raw = c.evidence.get("raw");
            let ownership_percent = raw
                .and_then(|r| r.get("scale"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let status = raw
                .and_then(|r| r.get("status"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let meets_threshold = ownership_percent
                .as_deref()
                .and_then(parse_ownership_percent)
                .is_some_and(|v| v >= threshold);
            SubsidiaryCandidate {
                name: c.value.trim().to_string(),
                ownership_percent,
                status,
                meets_threshold,
            }
        })
        .collect();

    // Coverage-gate landing (design 2026-06-15-db-truth-single-source-deliverable
    // §5 PR1): enrich wrote domains/subdomains into `organizations.domains`, but the
    // target_intel coverage gate reads per-asset / org-level business tables. Reuse
    // the org-recon landing hooks so techniques that actually ran land where the gate
    // looks. Enrich phase only (subsidiaries phase produces no per-asset coverage);
    // reload the org first to pick up the freshly-written domains; fully non-fatal.
    if phase == PassiveIntelPhase::Enrich {
        match golish_db::repo::organizations::get_one(pool.as_ref(), organization_id).await {
            Ok(Some(fresh)) => {
                // Passive-intel pairing closure (design 2026-06-17 §2 ③–⑤):
                // recover (host, ip) pairs from the survey records FIRST — their
                // hosts are the REAL provider-discovered subdomains. Descriptor-
                // driven via each provider's `normalize.pairs`; fully non-fatal.
                let pairs_rules_by_provider: HashMap<
                    String,
                    Vec<golish_pentest::models::AssetIntelPairRule>,
                > = scan
                    .tools
                    .iter()
                    .filter_map(|candidate_tool| {
                        candidate_tool.asset_intel.as_ref().map(|asset| {
                            let provider_id = super::provider_id_for_tool(candidate_tool)
                                .unwrap_or_else(|| candidate_tool.id.clone());
                            (provider_id, asset.normalize.pairs.clone())
                        })
                    })
                    .collect();
                let pairs = crate::asset_intel::landing::pairs_from_candidates(
                    &run.candidates,
                    &pairs_rules_by_provider,
                );

                // SUBDOMAIN landing (design 2026-06-23 Phase B): feed the provider
                // pair hosts (real subdomains) to the coverage landing so
                // `collect_subdomain_pairs` maps each to its owned root →
                // `target_assets(asset_type='subdomain')`. The old input
                // (`organizations.domains` alone) self-cancels — every entry IS an
                // owned root, so collect_subdomain_pairs skipped them all and
                // landed 0 subdomains on the agent path.
                let mut subdomain_hosts: Vec<String> = fresh
                    .domains
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                subdomain_hosts.extend(pairs.iter().map(|p| p.host.clone()));
                let landed = crate::organization_recon::land_target_intel_coverage(
                    pool.as_ref(),
                    &fresh,
                    &run.run_id,
                    &subdomain_hosts,
                )
                .await;
                tracing::info!(
                    run_id = %run.run_id,
                    subdomains = landed.subdomains,
                    subdomain_hosts = subdomain_hosts.len(),
                    "target_intel coverage landing (agent path)"
                );

                let promoted = crate::asset_intel::landing::promote_profile_assets_to_targets(
                    pool.as_ref(),
                    &fresh,
                    &pairs,
                )
                .await;
                tracing::info!(
                    run_id = %run.run_id,
                    pairs = pairs.len(),
                    promoted,
                    "passive-intel auto-landing to targets (agent path)"
                );

                // P1 (design 2026-06-26): land per-host service assets
                // (port/protocol/service/version) from the survey raw records.
                // Host targets now exist (promoted above), so services attach to
                // them. Cyberspace-mapping providers return open ports + service
                // banners per host; they were previously dropped (only the bare
                // subdomain landed, the 4 target_assets columns stayed NULL).
                // Non-fatal — a miss only warns.
                let services =
                    crate::asset_intel::landing::service_assets_from_candidates(&run.candidates);
                let service_assets = crate::asset_intel::landing::land_service_assets(
                    pool.as_ref(),
                    &fresh,
                    &services,
                )
                .await;
                tracing::info!(
                    run_id = %run.run_id,
                    services = services.len(),
                    service_assets,
                    "passive-intel service landing to target_assets (agent path)"
                );
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%error, "reload org for coverage landing failed")
            }
        }
    }

    Ok(PassiveIntelSummary {
        run_id: run.run_id,
        company: org.name,
        phase: phase.as_str(),
        status: format!("{:?}", run.status),
        organizations: run.candidates.organizations.len(),
        targets: run.candidates.targets.len(),
        providers: provider_ids,
        provider_status: run.provider_status,
        promoted_children,
        subsidiaries,
        domain_expansions: vec![],
    })
}

fn domain_expansion_roots(
    organization: &golish_db::models::Organization,
    limit: usize,
) -> Vec<String> {
    let mut roots = BTreeSet::new();
    collect_domain_roots_from_json(&organization.domains, &mut roots);
    if let Some(intel) = organization.intel.as_object() {
        if let Some(app_domains) = intel.get("app_domains") {
            collect_domain_roots_from_json(app_domains, &mut roots);
        }
    }
    roots.into_iter().take(limit).collect()
}

fn collect_domain_roots_from_json(value: &Value, roots: &mut BTreeSet<String>) {
    match value {
        Value::Null => {}
        Value::String(text) => push_domain_root(text, roots),
        Value::Array(items) => {
            for item in items {
                collect_domain_roots_from_json(item, roots);
            }
        }
        Value::Object(map) => {
            let mut matched = false;
            for key in ["domain", "host", "hostname", "url", "value", "name"] {
                if let Some(text) = map.get(key).and_then(Value::as_str) {
                    push_domain_root(text, roots);
                    matched = true;
                }
            }
            if !matched {
                for item in map.values() {
                    collect_domain_roots_from_json(item, roots);
                }
            }
        }
        other => push_domain_root(&other.to_string(), roots),
    }
}

fn push_domain_root(value: &str, roots: &mut BTreeSet<String>) {
    let value = value
        .trim()
        .trim_start_matches("*.")
        .trim_start_matches('.')
        .trim_end_matches('.');
    let Some(asset) = canonical_asset_key(value) else {
        return;
    };
    if asset.class != AssetClass::Domain {
        return;
    }
    let host = asset
        .key
        .trim_start_matches("*.")
        .trim_start_matches("www.")
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if !looks_like_domain(&host) || is_known_public_non_asset_host(&host) {
        return;
    }
    let apex = registrable_apex(&host)
        .trim_start_matches("www.")
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if looks_like_domain(&apex) && !is_known_public_non_asset_host(&apex) {
        roots.insert(apex);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_as_str_matches_wire_values() {
        assert_eq!(PassiveIntelPhase::Subsidiaries.as_str(), "subsidiaries");
        assert_eq!(PassiveIntelPhase::Enrich.as_str(), "enrich");
    }

    #[test]
    fn summary_serializes_with_camel_friendly_fields() {
        let s = PassiveIntelSummary {
            run_id: "r1".into(),
            company: "Acme".into(),
            phase: "enrich",
            status: "Completed".into(),
            organizations: 2,
            targets: 5,
            providers: vec!["0.zone".into()],
            provider_status: vec![],
            promoted_children: 0,
            subsidiaries: vec![],
            domain_expansions: vec![],
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["company"], "Acme");
        assert_eq!(v["phase"], "enrich");
        assert_eq!(v["targets"], 5);
        assert_eq!(v["providers"][0], "0.zone");
        assert!(v["providerStatus"].as_array().unwrap().is_empty());
        // Empty subsidiaries is skipped from the JSON so enrich stays clean.
        assert!(v.get("subsidiaries").is_none());
        assert!(v.get("domainExpansions").is_none());
    }

    #[test]
    fn subsidiary_candidate_serializes_with_threshold_flag() {
        let s = SubsidiaryCandidate {
            name: "平安银行股份有限公司".into(),
            ownership_percent: Some("58%".into()),
            status: Some("在营".into()),
            meets_threshold: true,
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["name"], "平安银行股份有限公司");
        assert_eq!(v["ownership_percent"], "58%");
        assert_eq!(v["status"], "在营");
        assert_eq!(v["meets_threshold"], true);
    }

    #[test]
    fn domain_expansion_roots_extracts_apexes_and_skips_noise() {
        let org = org_with_domains(
            serde_json::json!([
                "www.moresec.cn",
                {"domain": "mail.moresec.cn"},
                {"url": "https://api.moresec.com.cn:443/path"},
                "*.portal.moresec.cn.",
                "1.2.3.4",
                "github.com",
                "not a domain"
            ]),
            serde_json::json!({
                "app_domains": [
                    {"host": "console.moresec.com"},
                    {"host": "foo.github.com"}
                ]
            }),
        );

        assert_eq!(
            domain_expansion_roots(&org, 10),
            vec![
                "moresec.cn".to_string(),
                "moresec.com".to_string(),
                "moresec.com.cn".to_string(),
            ]
        );
        assert_eq!(
            domain_expansion_roots(&org, 1),
            vec!["moresec.cn".to_string()]
        );
    }

    fn org_with_domains(
        domains: serde_json::Value,
        intel: serde_json::Value,
    ) -> golish_db::models::Organization {
        let now = chrono::Utc::now();
        golish_db::models::Organization {
            id: Uuid::new_v4(),
            project_path: ".".into(),
            name: "Acme".into(),
            parent_id: None,
            description: String::new(),
            owner: String::new(),
            sort_order: 0,
            aliases: vec![],
            industry: String::new(),
            tier: String::new(),
            credit_code: String::new(),
            domains,
            ip_ranges: serde_json::json!([]),
            asns: serde_json::json!([]),
            email_domains: serde_json::json!([]),
            scope_rules: serde_json::json!({}),
            intel,
            notes: String::new(),
            certificates: serde_json::json!([]),
            subsidiaries: serde_json::json!([]),
            business_systems: serde_json::json!([]),
            cloud_assets: serde_json::json!([]),
            github_orgs: serde_json::json!([]),
            social_accounts: serde_json::json!([]),
            historical_vulns: serde_json::json!([]),
            contacts: serde_json::json!([]),
            created_at: now,
            updated_at: now,
        }
    }
}
