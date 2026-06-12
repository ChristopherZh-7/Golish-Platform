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
    auto_promote_discovered_children, run_providers_for_org, select_discovery_policy,
    select_enrichment_providers, select_subsidiary_providers, AssetIntelHydrateConfig,
    ToolsConfigState,
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
        select_discovery_policy(
            selected
                .iter()
                .filter_map(|tool| tool.asset_intel.as_ref())
                .map(|asset| &asset.discovery),
        )
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

    Ok(PassiveIntelSummary {
        run_id: run.run_id,
        company: org.name,
        phase: phase.as_str(),
        status: format!("{:?}", run.status),
        organizations: run.candidates.organizations.len(),
        targets: run.candidates.targets.len(),
        providers: provider_ids,
        promoted_children,
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
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["company"], "Acme");
        assert_eq!(v["phase"], "enrich");
        assert_eq!(v["targets"], 5);
        assert_eq!(v["providers"][0], "0.zone");
    }
}
