//! Agent-facing facade over the passive asset-intel engine.
//!
//! Wraps the inner pipeline used by the GUI commands
//! (`asset_intel_hydrate_subsidiaries` / `asset_intel_enrich_organization`) so an
//! agent `Tool` can drive subsidiary discovery + field enrichment without going
//! through the Tauri command layer (设计 2026-06-06-intel-stage-ai-driven-per-mode
//! §3.3). It scans the tools-config, selects the right provider phase, runs the
//! providers against one organization, and returns a small serializable summary
//! the agent tool can hand back (and the runtime can book to the evidence ledger).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use golish_app_core::GolishError;
use golish_pentest_domain::{canonical_asset_key, AssetClass};

use crate::organizations::OrganizationCandidates;

use super::{
    apply_ownership_threshold_override, auto_promote_discovered_children,
    enrichment_hydrate_config, parse_ownership_percent, run_providers_for_org,
    select_discovery_policy, select_enrichment_providers, select_subsidiary_providers,
    AssetIntelHydrateConfig, AssetIntelProviderRunState, AssetIntelProviderRunStatus,
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
    #[serde(rename = "observedTargets")]
    pub observed_targets: usize,
    pub targets: usize,
    pub providers: Vec<String>,
    /// Nested provider status for `source_query_log` with `target=<domain>`.
    #[serde(default, rename = "providerStatus")]
    pub provider_status: Vec<AssetIntelProviderRunStatus>,
    /// Technique-scoped non-found outcomes derived from the provider's declared
    /// capabilities. Unlike `providerStatus`, these rows may terminalize one
    /// exact coverage cell without signing for unrelated Intel techniques.
    #[serde(
        default,
        rename = "techniqueStatus",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub technique_status: Vec<IntelTechniqueRunStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// One exact Intel technique outcome from a provider that did not produce a
/// technique-specific DB `found` fact. This is deliberately produced by the
/// backend from provider capabilities and typed run status, never by the model.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IntelTechniqueRunStatus {
    pub source: String,
    pub technique: String,
    /// `found` | `empty` | `blocked` | `error`. A source-row `found` only clears
    /// an earlier retry marker; business-table facts remain coverage authority.
    pub status: String,
    pub message: String,
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
    /// Number of normalized target observations emitted by providers in this
    /// invocation. This is not a durable Target count.
    #[serde(rename = "observedTargets")]
    pub observed_targets: usize,
    /// Number of current-run domain/IP identities successfully upserted or
    /// reused in `targets`.
    pub targets: usize,
    #[serde(rename = "landedDomains")]
    pub landed_domains: usize,
    #[serde(rename = "landedIps")]
    pub landed_ips: usize,
    #[serde(rename = "dnsRecords")]
    pub dns_records: usize,
    #[serde(rename = "serviceAssets")]
    pub service_assets: usize,
    #[serde(rename = "subdomainAssets")]
    pub subdomain_assets: usize,
    /// Provider ids that were selected and run for this phase.
    pub providers: Vec<String>,
    /// Per-provider terminal status. This is source-level audit metadata for
    /// `source_query_log`; it is not a completeness proof by itself.
    #[serde(default, rename = "providerStatus")]
    pub provider_status: Vec<AssetIntelProviderRunStatus>,
    /// Exact technique-scoped non-found outcomes for `source_query_log`.
    #[serde(
        default,
        rename = "techniqueStatus",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub technique_status: Vec<IntelTechniqueRunStatus>,
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
    /// Present when provider data was obtained but gate-critical business/source
    /// persistence was incomplete. The runtime treats this as retryable failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn append_summary_error(error: &mut Option<String>, message: impl Into<String>) {
    let message = message.into();
    match error {
        Some(existing) => {
            existing.push_str("; ");
            existing.push_str(&message);
        }
        None => *error = Some(message),
    }
}

fn landing_technique_status(
    technique: &str,
    status: &str,
    message: &str,
) -> IntelTechniqueRunStatus {
    IntelTechniqueRunStatus {
        source: "golish-landing".to_string(),
        technique: technique.to_string(),
        status: status.to_string(),
        message: message.to_string(),
    }
}

/// Run one passive-intel phase (subsidiary discovery or field enrichment)
/// against a single organization, reusing the production asset-intel engine.
///
/// Master-record profile fields are written by `run_providers_for_org`; enrich
/// keeps normalized target observations in memory long enough to land them
/// directly and returns only a summary. Subsidiary discovery may still persist
/// organization candidates for the separate `ask_human(unit_review)` boundary.
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
        let roots = match authorized_domain_scope_hosts(pool.as_ref(), organization_id).await {
            Ok(hosts) => {
                domain_expansion_roots_from_authorized_hosts(&hosts, AUTO_DOMAIN_EXPANSION_LIMIT)
            }
            Err(error) => {
                tracing::warn!(%error, "load authorized roots for passive-intel domain expansion failed");
                summary.status = "Partial".to_string();
                append_summary_error(
                    &mut summary.error,
                    "authorized root lookup for domain expansion failed",
                );
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
                    if expansion.error.is_some() {
                        summary.status = "Partial".to_string();
                        append_summary_error(
                            &mut summary.error,
                            format!("domain-keyed landing was incomplete for {domain}"),
                        );
                    }
                    summary.domain_expansions.push(DomainExpansionSummary {
                        domain,
                        run_id: expansion.run_id,
                        status: expansion.status,
                        observed_targets: expansion.observed_targets,
                        targets: expansion.targets,
                        providers: expansion.providers,
                        provider_status: expansion.provider_status,
                        technique_status: expansion.technique_status,
                        error: expansion.error,
                    });
                }
                Err(error) => {
                    tracing::warn!(
                        domain = %domain,
                        %error,
                        "passive-intel automatic domain expansion failed"
                    );
                    summary.status = "Partial".to_string();
                    append_summary_error(
                        &mut summary.error,
                        format!("domain-keyed provider expansion failed for {domain}"),
                    );
                    summary.domain_expansions.push(DomainExpansionSummary {
                        domain,
                        run_id: String::new(),
                        status: "Failed".to_string(),
                        observed_targets: 0,
                        targets: 0,
                        providers: Vec::new(),
                        provider_status: Vec::new(),
                        technique_status: vec![
                            landing_technique_status(
                                "GOLISH-INTEL-SUBDOMAIN",
                                "error",
                                "domain-keyed provider expansion failed",
                            ),
                            landing_technique_status(
                                "GOLISH-INTEL-DNS",
                                "error",
                                "domain-keyed provider expansion failed",
                            ),
                        ],
                        error: Some("domain-keyed provider expansion failed".to_string()),
                    });
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
    let provider_capabilities: HashMap<String, Vec<String>> = selected
        .iter()
        .filter_map(|tool| {
            let asset = tool.asset_intel.as_ref()?;
            let provider_id = super::provider_id_for_tool(tool).unwrap_or_else(|| tool.id.clone());
            Some((provider_id, asset.capabilities.clone()))
        })
        .collect();

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

    let provider_config = if phase == PassiveIntelPhase::Enrich {
        enrichment_hydrate_config(config.clone())
    } else {
        config.clone()
    };
    let mut run = run_providers_for_org(
        None,
        pool.as_ref(),
        &pentest_config,
        &scan.tools,
        selected,
        &org,
        &org.name,
        &provider_config,
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

    // Current-run provider observations are the deterministic Target Intel -> EAS
    // handoff. Keep observation and actual business-write counts separate so a
    // normalized provider result can never masquerade as a durable Target.
    let observed_targets = run.candidates.targets.len();
    let mut target_landing = crate::asset_intel::landing::TargetLandingSummary::default();
    let mut landed_subdomain_assets = 0usize;
    let mut landed_service_assets = 0usize;
    let mut landing_statuses = Vec::new();
    let mut landing_errors = Vec::new();
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

                let mut current_run_hosts: Vec<String> = run.observed_domain_hosts.clone();
                current_run_hosts.extend(
                    run.candidates
                        .targets
                        .iter()
                        .map(|candidate| candidate.value.clone()),
                );
                current_run_hosts.extend(pairs.iter().map(|pair| pair.host.clone()));

                let current_run_plan = crate::asset_intel::landing::plan_current_run_targets(
                    &run.candidates,
                    &current_run_hosts,
                    &pairs,
                );
                let mut landing_org = fresh.clone();
                landing_org.domains = serde_json::json!(current_run_plan
                    .iter()
                    .filter(|target| target.target_type == "domain")
                    .map(|target| target.value.clone())
                    .collect::<Vec<_>>());
                if let Some(intel) = landing_org.intel.as_object_mut() {
                    intel.remove("app_domains");
                }

                match crate::asset_intel::landing::land_current_run_targets(
                    pool.as_ref(),
                    &fresh,
                    &run.candidates,
                    &pairs,
                    &current_run_hosts,
                )
                .await
                {
                    Ok(landed) => {
                        target_landing = landed;
                        tracing::info!(
                            run_id = %run.run_id,
                            pairs = pairs.len(),
                            observed_targets,
                            targets = landed.targets,
                            domains = landed.domains,
                            ips = landed.ips,
                            dns_records = landed.dns_records,
                            "passive-intel direct landing to targets (agent path)"
                        );
                        if observed_targets > 0 && landed.targets == 0 {
                            landing_errors.push(
                                "provider observations produced zero durable targets".to_string(),
                            );
                            landing_statuses.push(landing_technique_status(
                                "GOLISH-INTEL-DNS",
                                "error",
                                "provider observations produced zero durable targets",
                            ));
                        } else if !pairs.is_empty() && landed.dns_records == 0 {
                            landing_errors.push(
                                "provider host/IP pairs produced zero DNS records".to_string(),
                            );
                            landing_statuses.push(landing_technique_status(
                                "GOLISH-INTEL-DNS",
                                "error",
                                "provider host/IP pairs produced zero DNS records",
                            ));
                        } else if landed.dns_records > 0 {
                            landing_statuses.push(landing_technique_status(
                                "GOLISH-INTEL-DNS",
                                "found",
                                "provider DNS business landing completed",
                            ));
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "provider target/DNS business landing failed");
                        landing_errors
                            .push("provider target/DNS business landing failed".to_string());
                        landing_statuses.push(landing_technique_status(
                            "GOLISH-INTEL-DNS",
                            "error",
                            "provider target/DNS business landing failed",
                        ));
                    }
                }

                // Targets must exist before relationship rows can resolve their
                // parent identity. Pair only this invocation's concrete domains.
                match crate::organization_recon::land_target_intel_coverage(
                    pool.as_ref(),
                    &landing_org,
                    &run.run_id,
                    &current_run_hosts,
                )
                .await
                {
                    Ok(landed) => {
                        landed_subdomain_assets = landed.subdomains;
                        tracing::info!(
                            run_id = %run.run_id,
                            subdomains = landed.subdomains,
                            current_run_hosts = current_run_hosts.len(),
                            "target_intel subdomain relationship landing (agent path)"
                        );
                        if landed.subdomains > 0 {
                            landing_statuses.push(landing_technique_status(
                                "GOLISH-INTEL-SUBDOMAIN",
                                "found",
                                "subdomain business landing completed",
                            ));
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "target_intel subdomain business landing failed");
                        landing_errors.push("subdomain business landing failed".to_string());
                        landing_statuses.push(landing_technique_status(
                            "GOLISH-INTEL-SUBDOMAIN",
                            "error",
                            "subdomain business landing failed",
                        ));
                    }
                }

                // P1 (design 2026-06-26): land per-host service assets
                // (port/protocol/service/version) from the survey raw records.
                // Host targets now exist (promoted above), so services attach to
                // them. Cyberspace-mapping providers return open ports + service
                // banners per host; they were previously dropped (only the bare
                // subdomain landed, the 4 target_assets columns stayed NULL).
                // Non-fatal — a miss only warns.
                let services =
                    crate::asset_intel::landing::service_assets_from_candidates(&run.candidates);
                match crate::asset_intel::landing::land_service_assets(
                    pool.as_ref(),
                    &landing_org,
                    &services,
                )
                .await
                {
                    Ok(service_assets) => {
                        landed_service_assets = service_assets;
                        tracing::info!(
                            run_id = %run.run_id,
                            services = services.len(),
                            service_assets,
                            "passive-intel service landing to target_assets (agent path)"
                        );
                        if !services.is_empty() && service_assets == 0 {
                            landing_errors.push(
                                "provider service observations produced zero service assets"
                                    .to_string(),
                            );
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "provider service observation landing failed");
                        landing_errors
                            .push("provider service observation landing failed".to_string());
                    }
                }
            }
            Ok(None) => {
                landing_errors.push("organization disappeared before business landing".to_string());
                landing_statuses.push(landing_technique_status(
                    "GOLISH-INTEL-DNS",
                    "error",
                    "organization disappeared before business landing",
                ));
            }
            Err(error) => {
                tracing::warn!(%error, "reload org for coverage landing failed");
                landing_errors
                    .push("organization reload before business landing failed".to_string());
                landing_statuses.push(landing_technique_status(
                    "GOLISH-INTEL-DNS",
                    "error",
                    "organization reload before business landing failed",
                ));
            }
        }
    }
    let mut technique_status = derive_intel_technique_statuses(
        &provider_capabilities,
        &run.provider_status,
        &run.evidence,
    );
    technique_status.extend(landing_statuses);
    technique_status.sort_by(|left, right| {
        (&left.source, &left.technique).cmp(&(&right.source, &right.technique))
    });
    technique_status
        .dedup_by(|left, right| left.source == right.source && left.technique == right.technique);
    let landing_error = (!landing_errors.is_empty()).then(|| landing_errors.join("; "));
    Ok(PassiveIntelSummary {
        run_id: run.run_id,
        company: org.name,
        phase: phase.as_str(),
        status: if landing_error.is_some() {
            "Partial".to_string()
        } else {
            format!("{:?}", run.status)
        },
        organizations: run.candidates.organizations.len(),
        observed_targets,
        targets: target_landing.targets,
        landed_domains: target_landing.domains,
        landed_ips: target_landing.ips,
        dns_records: target_landing.dns_records,
        service_assets: landed_service_assets,
        subdomain_assets: landed_subdomain_assets,
        providers: provider_ids,
        provider_status: run.provider_status,
        technique_status,
        promoted_children,
        subsidiaries,
        domain_expansions: vec![],
        error: landing_error,
    })
}

fn derive_intel_technique_statuses(
    provider_capabilities: &HashMap<String, Vec<String>>,
    provider_statuses: &[AssetIntelProviderRunStatus],
    provider_evidence: &[Value],
) -> Vec<IntelTechniqueRunStatus> {
    let mut exact_native =
        derive_native_query_technique_statuses(provider_capabilities, provider_evidence);
    let mut rows = Vec::new();
    for provider in provider_statuses {
        let Some(capabilities) = provider_capabilities.get(&provider.provider_id) else {
            continue;
        };
        for technique in intel_techniques_for_capabilities(capabilities) {
            // Native and HTTP providers expose each query/request's typed
            // outcome. Prefer that narrower result over the provider-wide state
            // so one failed sibling cannot poison an independently successful
            // technique (and one success cannot sign for an unrelated failure).
            if let Some(exact) =
                exact_native.remove(&(provider.provider_id.clone(), technique.to_string()))
            {
                rows.push(exact);
                continue;
            }
            // Source-row found is attempt/recovery state only; business-table
            // landing remains the sole coverage authority. Emitting it also
            // overwrites an earlier exact error after a successful retry.
            let status = match provider.status {
                AssetIntelProviderRunState::Completed => "found",
                AssetIntelProviderRunState::CheckedEmpty => "empty",
                AssetIntelProviderRunState::Unavailable => "blocked",
                AssetIntelProviderRunState::Failed => "error",
            };
            rows.push(IntelTechniqueRunStatus {
                source: provider.provider_id.clone(),
                technique: technique.to_string(),
                status: status.to_string(),
                message: provider.message.clone(),
            });
        }
    }
    rows.sort_by(|left, right| {
        (&left.source, &left.technique).cmp(&(&right.source, &right.technique))
    });
    rows.dedup_by(|left, right| left.source == right.source && left.technique == right.technique);
    rows
}

#[derive(Debug, Default)]
struct NativeTechniqueAggregate {
    attempted: usize,
    succeeded: usize,
    found: bool,
    errors: usize,
}

/// Derive non-found Intel outcomes from native-provider query evidence. The
/// provider status remains useful as coarse provenance, but a query-scoped row
/// is the stronger source when a provider batch mixes success and failure.
fn derive_native_query_technique_statuses(
    provider_capabilities: &HashMap<String, Vec<String>>,
    provider_evidence: &[Value],
) -> BTreeMap<(String, String), IntelTechniqueRunStatus> {
    let mut aggregates = BTreeMap::<(String, String), NativeTechniqueAggregate>::new();
    for evidence in provider_evidence {
        let Some(source) = evidence
            .get("provider")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|source| !source.is_empty())
        else {
            continue;
        };
        let Some(capabilities) = provider_capabilities.get(source) else {
            continue;
        };
        let Some(queries) = evidence
            .get("queries")
            .or_else(|| evidence.get("requests"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for query in queries {
            let Some(query_type) = query.get("queryType").and_then(Value::as_str) else {
                // Older manifests did not carry the query discriminator. They
                // remain on the conservative provider-wide fallback path.
                continue;
            };
            let status = query
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("error");
            let records = query.get("records").and_then(Value::as_u64).unwrap_or(0);
            for technique in intel_techniques_for_native_query(query_type, capabilities) {
                let aggregate = aggregates
                    .entry((source.to_string(), technique.to_string()))
                    .or_default();
                aggregate.attempted += 1;
                match status {
                    "found" if records > 0 => {
                        aggregate.succeeded += 1;
                        aggregate.found = true;
                    }
                    "empty" => aggregate.succeeded += 1,
                    _ => aggregate.errors += 1,
                }
            }
        }
    }

    aggregates
        .into_iter()
        .filter_map(|((source, technique), aggregate)| {
            if aggregate.attempted == 0 {
                return None;
            }
            let (status, message) = if aggregate.errors > 0 {
                (
                    "error",
                    format!(
                        "{source} completed {}/{} native query attempt(s) for {technique}; {} errored",
                        aggregate.succeeded, aggregate.attempted, aggregate.errors
                    ),
                )
            } else if aggregate.found {
                (
                    "found",
                    format!(
                        "{source} completed all {}/{} native query attempt(s) for {technique} with records",
                        aggregate.succeeded, aggregate.attempted
                    ),
                )
            } else if aggregate.succeeded == aggregate.attempted {
                (
                    "empty",
                    format!(
                        "{source} completed all {}/{} native query attempt(s) for {technique} with no records",
                        aggregate.succeeded, aggregate.attempted
                    ),
                )
            } else {
                (
                    "error",
                    format!(
                        "{source} completed {}/{} native query attempt(s) for {technique}; {} errored",
                        aggregate.succeeded, aggregate.attempted, aggregate.errors
                    ),
                )
            };
            Some((
                (source.clone(), technique.clone()),
                IntelTechniqueRunStatus {
                    source,
                    technique,
                    status: status.to_string(),
                    message,
                },
            ))
        })
        .collect()
}

fn intel_techniques_for_native_query(
    query_type: &str,
    capabilities: &[String],
) -> Vec<&'static str> {
    let declared = intel_techniques_for_capabilities(capabilities);
    match query_type.trim().to_ascii_lowercase().as_str() {
        // HTTP request ids share this discriminator with native query types.
        // Keep common request families deliberately narrow: a domain lookup
        // does not prove DNS resolution, while corporate/app/contact requests
        // do not sign for every other capability declared by the provider.
        "domain" | "domain_root" => declared
            .into_iter()
            .filter(|technique| *technique == "GOLISH-INTEL-SUBDOMAIN")
            .collect(),
        "cert" => declared
            .into_iter()
            .filter(|technique| *technique == "GOLISH-INTEL-CT")
            .collect(),
        "asn" => declared
            .into_iter()
            .filter(|technique| *technique == "GOLISH-INTEL-ASN")
            .collect(),
        "apk" | "org" | "email" | "member" | "icp_unit" => declared
            .into_iter()
            .filter(|technique| *technique == "GOLISH-INTEL-OSINT")
            .collect(),
        "cidr" => Vec::new(),
        // `parse_query_type` treats unknown values as the provider's broad Site
        // query. Mirror that behavior here and bind it to declared capabilities.
        _ => declared,
    }
}

fn intel_techniques_for_capabilities(capabilities: &[String]) -> Vec<&'static str> {
    let has = |expected: &[&str]| {
        capabilities.iter().any(|capability| {
            expected
                .iter()
                .any(|item| capability.eq_ignore_ascii_case(item))
        })
    };
    let mut techniques = Vec::new();
    if has(&["domains", "subdomains"]) {
        techniques.push("GOLISH-INTEL-SUBDOMAIN");
    }
    if has(&["dns", "dns_records"]) {
        techniques.push("GOLISH-INTEL-DNS");
    }
    if has(&["asn", "asns"]) {
        techniques.push("GOLISH-INTEL-ASN");
    }
    if has(&["certificates", "certificate_transparency", "ct"]) {
        techniques.push("GOLISH-INTEL-CT");
    }
    if has(&[
        "contacts",
        "social_accounts",
        "apps",
        "mini_programs",
        "icp",
        "business_systems",
    ]) {
        techniques.push("GOLISH-INTEL-OSINT");
    }
    techniques
}

pub(crate) async fn authorized_domain_scope_hosts(
    pool: &sqlx::PgPool,
    organization_id: Uuid,
) -> Result<Vec<String>, GolishError> {
    let rows = domain_scope_rows(pool, organization_id).await?;
    Ok(authorized_domain_scope_hosts_from_rows(&rows))
}

/// WHOIS is passive registration enrichment for Targets already materialized
/// by the deterministic asset-map backend. It may consume `source=asset_intel`
/// domain rows without making those rows recursive provider-query roots.
pub(crate) async fn whois_domain_scope_hosts(
    pool: &sqlx::PgPool,
    organization_id: Uuid,
) -> Result<Vec<String>, GolishError> {
    let rows = domain_scope_rows(pool, organization_id).await?;
    Ok(whois_domain_scope_hosts_from_rows(&rows))
}

async fn domain_scope_rows(
    pool: &sqlx::PgPool,
    organization_id: Uuid,
) -> Result<Vec<(String, String, String)>, GolishError> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        r#"SELECT value, target_type::text, COALESCE(source, '')
           FROM targets
           WHERE organization_id = $1
             AND scope::text = 'in'
             AND target_type::text IN ('domain', 'url', 'wildcard')
           ORDER BY created_at ASC, id ASC"#,
    )
    .bind(organization_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

fn authorized_domain_scope_hosts_from_rows(rows: &[(String, String, String)]) -> Vec<String> {
    domain_scope_hosts_from_rows(rows, false)
}

fn whois_domain_scope_hosts_from_rows(rows: &[(String, String, String)]) -> Vec<String> {
    domain_scope_hosts_from_rows(rows, true)
}

fn domain_scope_hosts_from_rows(
    rows: &[(String, String, String)],
    include_asset_intel: bool,
) -> Vec<String> {
    let mut hosts = BTreeSet::new();
    for (value, target_type, source) in rows {
        let source = source.trim().to_ascii_lowercase();
        let trusted_intake = matches!(
            source.as_str(),
            "manual" | "imported" | "customer_provided" | "stage-run-seed" | "seed" | "cli"
        );
        if !(trusted_intake || (include_asset_intel && source == "asset_intel"))
            || !matches!(target_type.as_str(), "domain" | "url" | "wildcard")
        {
            continue;
        }
        let (candidate, wildcard) = if target_type == "wildcard" {
            let Some(base) = value.trim().trim_end_matches('.').strip_prefix("*.") else {
                continue;
            };
            (base, true)
        } else {
            (value.as_str(), false)
        };
        let Some(key) = canonical_asset_key(candidate) else {
            continue;
        };
        if key.class == AssetClass::Domain && !is_known_public_non_asset_host(&key.key) {
            hosts.insert(if wildcard {
                format!("*.{}", key.key)
            } else {
                key.key
            });
        }
    }
    hosts.into_iter().collect()
}

fn domain_expansion_roots_from_authorized_hosts(hosts: &[String], limit: usize) -> Vec<String> {
    let mut roots = BTreeSet::new();
    for host in hosts {
        push_authorized_domain_query(host, &mut roots);
    }
    roots.into_iter().take(limit).collect()
}

fn push_authorized_domain_query(value: &str, roots: &mut BTreeSet<String>) {
    let value = value
        .trim()
        .trim_start_matches("*.")
        .trim_start_matches('.')
        .trim_end_matches('.');
    let value = value.strip_prefix("*.").unwrap_or(value);
    let Some(asset) = canonical_asset_key(value) else {
        return;
    };
    if asset.class != AssetClass::Domain {
        return;
    }
    let host = asset
        .key
        .trim_start_matches("*.")
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if !looks_like_domain(&host) || is_known_public_non_asset_host(&host) {
        return;
    }
    // Provider query scope preserves the exact approved host. Folding an
    // approved `www.example.com` target to `example.com` would silently widen
    // authorization to its apex and siblings. WHOIS may separately derive a
    // registrable apex because it is a registration lookup, not scan scope.
    roots.insert(host);
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
            observed_targets: 9,
            targets: 5,
            landed_domains: 3,
            landed_ips: 2,
            dns_records: 4,
            service_assets: 6,
            subdomain_assets: 1,
            providers: vec!["0.zone".into()],
            provider_status: vec![],
            technique_status: vec![],
            promoted_children: 0,
            subsidiaries: vec![],
            domain_expansions: vec![],
            error: None,
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["company"], "Acme");
        assert_eq!(v["phase"], "enrich");
        assert_eq!(v["targets"], 5);
        assert_eq!(v["observedTargets"], 9);
        assert_eq!(v["landedDomains"], 3);
        assert_eq!(v["landedIps"], 2);
        assert_eq!(v["dnsRecords"], 4);
        assert_eq!(v["serviceAssets"], 6);
        assert_eq!(v["subdomainAssets"], 1);
        assert_eq!(v["providers"][0], "0.zone");
        assert!(v["providerStatus"].as_array().unwrap().is_empty());
        assert!(v.get("techniqueStatus").is_none());
        // Empty subsidiaries is skipped from the JSON so enrich stays clean.
        assert!(v.get("subsidiaries").is_none());
        assert!(v.get("domainExpansions").is_none());
    }

    #[test]
    fn checked_empty_provider_capabilities_emit_exact_intel_technique_statuses() {
        let capabilities = HashMap::from([
            (
                "enscan-go-enrichment".to_string(),
                vec![
                    "domains".to_string(),
                    "contacts".to_string(),
                    "social_accounts".to_string(),
                ],
            ),
            ("domain-only".to_string(), vec!["domains".to_string()]),
        ]);
        let statuses = vec![
            AssetIntelProviderRunStatus {
                provider_id: "enscan-go-enrichment".to_string(),
                status: crate::asset_intel::AssetIntelProviderRunState::CheckedEmpty,
                message: "no candidates".to_string(),
            },
            AssetIntelProviderRunStatus {
                provider_id: "domain-only".to_string(),
                status: crate::asset_intel::AssetIntelProviderRunState::CheckedEmpty,
                message: "no domains".to_string(),
            },
        ];

        let derived = derive_intel_technique_statuses(&capabilities, &statuses, &[]);
        assert!(derived.iter().any(|row| {
            row.source == "enscan-go-enrichment"
                && row.technique == "GOLISH-INTEL-OSINT"
                && row.status == "empty"
        }));
        assert!(derived.iter().any(|row| {
            row.source == "enscan-go-enrichment"
                && row.technique == "GOLISH-INTEL-SUBDOMAIN"
                && row.status == "empty"
        }));
        assert!(!derived
            .iter()
            .any(|row| { row.source == "domain-only" && row.technique == "GOLISH-INTEL-OSINT" }));
    }

    #[test]
    fn native_mixed_query_results_prefer_exact_status_over_provider_wide_empty() {
        let capabilities = HashMap::from([(
            "native-mixed".to_string(),
            vec!["domains".to_string(), "certificates".to_string()],
        )]);
        // Simulate a stale/coarse provider state to prove the query-scoped
        // projection is independently fail-closed: the broad query was empty,
        // while the CT-specific query errored.
        let statuses = vec![AssetIntelProviderRunStatus {
            provider_id: "native-mixed".to_string(),
            status: crate::asset_intel::AssetIntelProviderRunState::CheckedEmpty,
            message: "1/2 queries succeeded".to_string(),
        }];
        let evidence = vec![serde_json::json!({
            "provider": "native-mixed",
            "queries": [
                {"queryType": "site", "status": "empty", "records": 0},
                {"queryType": "cert", "status": "error", "error": "upstream timeout"}
            ]
        })];

        let derived = derive_intel_technique_statuses(&capabilities, &statuses, &evidence);
        let subdomain = derived
            .iter()
            .find(|row| row.technique == "GOLISH-INTEL-SUBDOMAIN")
            .expect("subdomain status");
        let ct = derived
            .iter()
            .find(|row| row.technique == "GOLISH-INTEL-CT")
            .expect("CT status");
        assert_eq!(subdomain.status, "empty");
        assert_eq!(ct.status, "error");
        assert!(!derived.iter().all(|row| row.status == "empty"));
    }

    #[test]
    fn http_request_ids_map_to_minimal_exact_intel_techniques() {
        let capabilities = vec![
            "domains".to_string(),
            "dns_records".to_string(),
            "asns".to_string(),
            "certificates".to_string(),
            "apps".to_string(),
            "contacts".to_string(),
        ];

        for request_id in ["domain", "domain_root"] {
            assert_eq!(
                intel_techniques_for_native_query(request_id, &capabilities),
                vec!["GOLISH-INTEL-SUBDOMAIN"],
                "{request_id} must not sign for DNS or unrelated organization intel"
            );
        }
        for request_id in ["apk", "org", "email", "member", "icp_unit"] {
            assert_eq!(
                intel_techniques_for_native_query(request_id, &capabilities),
                vec!["GOLISH-INTEL-OSINT"],
                "{request_id} must affect only OSINT"
            );
        }
    }

    #[test]
    fn http_found_request_is_not_polluted_by_a_sibling_request_error() {
        let capabilities = HashMap::from([(
            "http-mixed".to_string(),
            vec![
                "domains".to_string(),
                "apps".to_string(),
                "contacts".to_string(),
            ],
        )]);
        let statuses = vec![AssetIntelProviderRunStatus {
            provider_id: "http-mixed".to_string(),
            status: crate::asset_intel::AssetIntelProviderRunState::Failed,
            message: "1/2 requests succeeded".to_string(),
        }];
        let evidence = vec![serde_json::json!({
            "provider": "http-mixed",
            "requests": [
                {"queryType": "domain_root", "status": "found", "records": 2},
                {"queryType": "apk", "status": "error", "records": 0}
            ]
        })];

        let derived = derive_intel_technique_statuses(&capabilities, &statuses, &evidence);
        let subdomain = derived
            .iter()
            .find(|row| row.technique == "GOLISH-INTEL-SUBDOMAIN")
            .expect("domain_root status");
        let osint = derived
            .iter()
            .find(|row| row.technique == "GOLISH-INTEL-OSINT")
            .expect("apk status");

        assert_eq!(subdomain.status, "found");
        assert_eq!(osint.status, "error");
    }

    #[test]
    fn native_success_retry_emits_exact_found_to_replace_an_old_error_marker() {
        let capabilities =
            HashMap::from([("native-retry".to_string(), vec!["domains".to_string()])]);
        let failed = vec![serde_json::json!({
            "provider": "native-retry",
            "queries": [{"queryType": "domain", "status": "error"}]
        })];
        let recovered = vec![serde_json::json!({
            "provider": "native-retry",
            "queries": [{"queryType": "domain", "status": "found", "records": 2}]
        })];

        let failed_rows = derive_native_query_technique_statuses(&capabilities, &failed);
        let recovered_rows = derive_native_query_technique_statuses(&capabilities, &recovered);
        let key = (
            "native-retry".to_string(),
            "GOLISH-INTEL-SUBDOMAIN".to_string(),
        );
        assert_eq!(failed_rows[&key].status, "error");
        assert_eq!(recovered_rows[&key].status, "found");
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
    fn provider_discovered_domains_never_become_authorization_roots_on_retry() {
        let rows = vec![
            (
                "moresec.cn".to_string(),
                "domain".to_string(),
                "manual".to_string(),
            ),
            (
                "https://portal.moresec.cn/login".to_string(),
                "url".to_string(),
                "cli".to_string(),
            ),
            (
                "customer.moresec.cn".to_string(),
                "domain".to_string(),
                "customer_provided".to_string(),
            ),
            (
                "cdn.vendor.net".to_string(),
                "domain".to_string(),
                "asset_intel".to_string(),
            ),
            (
                "a.moresec.cn".to_string(),
                "domain".to_string(),
                "asset_intel".to_string(),
            ),
            (
                "asserted.moresec.cn".to_string(),
                "domain".to_string(),
                "discovered".to_string(),
            ),
            (
                "198.51.100.10".to_string(),
                "ip".to_string(),
                "manual".to_string(),
            ),
            (
                "*.wild.moresec.cn".to_string(),
                "wildcard".to_string(),
                "manual".to_string(),
            ),
        ];
        let hosts = authorized_domain_scope_hosts_from_rows(&rows);
        assert_eq!(
            hosts,
            vec![
                "*.wild.moresec.cn",
                "customer.moresec.cn",
                "moresec.cn",
                "portal.moresec.cn",
            ]
        );
        let scope_org = org_with_domains(serde_json::json!(hosts), serde_json::json!({}));
        assert!(crate::organization_recon::value_belongs_to_organization(
            &scope_org,
            "api.moresec.cn"
        ));
        assert!(!crate::organization_recon::value_belongs_to_organization(
            &scope_org,
            "cdn.vendor.net"
        ));
    }

    #[test]
    fn asset_intel_targets_are_whois_inputs_without_becoming_recursive_provider_roots() {
        let rows = vec![
            (
                "manual.moresec.cn".to_string(),
                "domain".to_string(),
                "manual".to_string(),
            ),
            (
                "api.moresec.cn".to_string(),
                "domain".to_string(),
                "asset_intel".to_string(),
            ),
            (
                "model-asserted.moresec.cn".to_string(),
                "domain".to_string(),
                "discovered".to_string(),
            ),
        ];

        assert_eq!(
            authorized_domain_scope_hosts_from_rows(&rows),
            vec!["manual.moresec.cn"]
        );
        assert_eq!(
            whois_domain_scope_hosts_from_rows(&rows),
            vec!["api.moresec.cn", "manual.moresec.cn"]
        );
    }

    #[test]
    fn domain_expansion_uses_only_authorized_target_hosts() {
        let hosts = vec![
            "www.moresec.cn".to_string(),
            "api.moresec.com.cn".to_string(),
            "console.moresec.com".to_string(),
            "moresec.cn".to_string(),
            "*.wild.moresec.cn".to_string(),
        ];

        assert_eq!(
            domain_expansion_roots_from_authorized_hosts(&hosts, 10),
            vec![
                "api.moresec.com.cn".to_string(),
                "console.moresec.com".to_string(),
                "moresec.cn".to_string(),
                "wild.moresec.cn".to_string(),
                "www.moresec.cn".to_string(),
            ]
        );
        assert_eq!(
            domain_expansion_roots_from_authorized_hosts(&hosts, 1),
            vec!["api.moresec.com.cn".to_string()]
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
