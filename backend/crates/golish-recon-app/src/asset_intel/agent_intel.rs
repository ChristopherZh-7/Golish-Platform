//! Agent-facing facade over the passive asset-intel engine.
//!
//! Wraps the inner pipeline used by the GUI commands
//! (`asset_intel_hydrate_subsidiaries` / `asset_intel_enrich_organization`) so an
//! agent `Tool` can drive subsidiary discovery + field enrichment without going
//! through the Tauri command layer (设计 2026-06-06-intel-stage-ai-driven-per-mode
//! §3.3). It scans the tools-config, selects the right provider phase, runs the
//! providers against one organization, and returns a small serializable summary
//! the agent tool can hand back (and the runtime can book to the evidence ledger).

use std::sync::Arc;

use serde::Serialize;
use uuid::Uuid;

use golish_app_core::GolishError;

use crate::organizations::OrganizationCandidates;

use super::{
    apply_ownership_threshold_override, auto_promote_discovered_children, parse_ownership_percent,
    run_providers_for_org, select_discovery_policy, select_enrichment_providers,
    select_subsidiary_providers, AssetIntelHydrateConfig, ToolsConfigState,
};

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
    /// Phase 2: number of discovered subsidiaries auto-promoted to child orgs
    /// (subsidiaries phase only; 0 for enrich or when no candidate qualified).
    pub promoted_children: usize,
    /// Subsidiaries phase with auto_promote OFF: the discovered candidates so the
    /// agent can pass them into `ask_human(unit_review)` for the user to pick.
    /// Empty for enrich or when candidates were auto-promoted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subsidiaries: Vec<SubsidiaryCandidate>,
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
                let subdomain_hosts: Vec<String> = fresh
                    .domains
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
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
                    dns_records = landed.dns_records,
                    certificates = landed.certificates,
                    whois = landed.whois,
                    "target_intel coverage landing (agent path)"
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
        promoted_children,
        subsidiaries,
    })
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
            promoted_children: 0,
            subsidiaries: vec![],
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["company"], "Acme");
        assert_eq!(v["phase"], "enrich");
        assert_eq!(v["targets"], 5);
        assert_eq!(v["providers"][0], "0.zone");
        // Empty subsidiaries is skipped from the JSON so enrich stays clean.
        assert!(v.get("subsidiaries").is_none());
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
}
