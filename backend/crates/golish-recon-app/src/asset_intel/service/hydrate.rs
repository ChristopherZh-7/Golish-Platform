//! Hydrate orchestration: per-org provider runs + engagement promotion.
//! Moved out of `mod.rs` verbatim.

use std::sync::Arc;

use futures::{stream, StreamExt};
use tokio::sync::Semaphore;

use super::super::*;

const CLI_PROVIDER_CONCURRENCY: usize = 2;
const HTTP_PROVIDER_CONCURRENCY: usize = 4;

pub(crate) async fn clear_engagement_candidates_for_org(
    pool: &sqlx::PgPool,
    organization_id: Uuid,
) -> Result<(), GolishError> {
    let Some(row) = golish_db::repo::organizations::get_one(pool, organization_id).await? else {
        return Err(GolishError::NotFound(format!(
            "organization {organization_id}"
        )));
    };
    let intel = clear_engagement_candidates_from_intel(row.intel)?;
    let patch = golish_db::repo::organizations::ProfilePatch {
        intel: Some(intel),
        ..Default::default()
    };
    golish_db::repo::organizations::update_profile(pool, organization_id, &patch)
        .await?
        .ok_or_else(|| GolishError::NotFound(format!("organization {organization_id}")))?;
    Ok(())
}

pub(crate) async fn auto_promote_discovered_children(
    pool: &sqlx::PgPool,
    parent: &golish_db::models::Organization,
    candidates: &OrganizationCandidates,
    policy: &golish_pentest::models::AssetIntelDiscoveryConfig,
) -> Result<Value, GolishError> {
    let existing = golish_db::repo::organizations::list(pool, &parent.project_path).await?;
    let existing_child_names: HashSet<String> = existing
        .iter()
        .filter(|org| org.parent_id == Some(parent.id))
        .map(|org| org.name.trim().to_lowercase())
        .collect();
    let decisions = auto_promote_child_decisions(candidates, policy, &existing_child_names);

    let mut created = Vec::new();
    let mut skipped = Vec::new();
    for decision in decisions {
        let name = decision.candidate.value.trim();
        if decision.promote {
            let child = golish_db::repo::organizations::create(
                pool,
                &parent.project_path,
                name,
                Some(parent.id),
                &format!(
                    "Auto-promoted from {} investment discovery",
                    decision.candidate.source
                ),
                "",
            )
            .await?;
            let mut intel = serde_json::Map::new();
            intel.insert(
                "asset_intel_discovery".into(),
                serde_json::json!({
                    "parentOrganizationId": parent.id.to_string(),
                    "source": decision.candidate.source,
                    "ownershipPercent": decision.ownership_percent,
                    "evidence": decision.candidate.evidence,
                }),
            );
            let patch = golish_db::repo::organizations::ProfilePatch {
                intel: Some(Value::Object(intel)),
                ..Default::default()
            };
            golish_db::repo::organizations::update_profile(pool, child.id, &patch).await?;
            created.push(serde_json::json!({
                "organizationId": child.id.to_string(),
                "name": child.name,
                "ownershipPercent": decision.ownership_percent,
                "source": decision.candidate.source,
            }));
        } else {
            skipped.push(serde_json::json!({
                "name": name,
                "source": decision.candidate.source,
                "ownershipPercent": decision.ownership_percent,
                "reason": decision.reason,
            }));
        }
    }
    clear_engagement_candidates_for_org(pool, parent.id).await?;

    Ok(serde_json::json!({
        "kind": "auto_promote_children",
        "policy": policy,
        "clearedCandidates": true,
        "created": created,
        "skipped": skipped,
    }))
}

/// b1 (design 2026-06-24-intel-to-eas-handoff): a provider supports the
/// domain-keyed survey iff any of its query/request templates reference
/// `{{domain}}`. Shape-agnostic — inspects the serialized runtime config so it
/// works across native_provider / http_json / cli_json without per-kind code.
fn provider_supports_domain(tool: &ToolConfig) -> bool {
    tool.asset_intel
        .as_ref()
        .and_then(|a| serde_json::to_string(&a.runtime).ok())
        .is_some_and(|s| s.contains("{{domain}}"))
}

/// Passive surface providers observe addresses behind hostnames; they do not
/// grant direct network-scan authorization. Keep those values in pair evidence /
/// `dns_records`, but never merge provider-authored `ip_ranges` into the org's
/// authorized profile. Derived ASN entries and all non-scope profile fields stay.
fn remove_passive_ip_scope_entries(entries: &mut Vec<ProfileFieldEntry>) {
    entries.retain(|entry| entry.target_field != "ip_ranges");
}

fn candidate_queue_enabled(config: &AssetIntelHydrateConfig) -> bool {
    config.create_candidates.unwrap_or(false)
}

/// Run a set of asset-intel providers against a single organization, writing
/// candidates + master-record profile fields back to **that org's** id.
///
/// This is the shared backbone behind every hydrate / enrich command:
/// - legacy [`asset_intel_hydrate`] passes the full provider list,
/// - [`asset_intel_hydrate_subsidiaries`] passes the discovery subset,
/// - [`asset_intel_enrich_organization`] and [`asset_intel_enrich_batch`]
///   pass the enrichment subset (and use the org's own name as the query).
///
/// The function intentionally takes already-filtered `providers` so the
/// caller controls phase semantics; this body only knows "run these tools
/// for this org with this name".
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_providers_for_org(
    sink: Option<&EventEmitterHandle>,
    pool: &sqlx::PgPool,
    pentest_config: &golish_pentest::config::PentestConfig,
    scan_tools: &[ToolConfig],
    providers: Vec<ToolConfig>,
    org_row: &golish_db::models::Organization,
    company_name: &str,
    config: &AssetIntelHydrateConfig,
) -> Result<AssetIntelRun, GolishError> {
    let run_id = Uuid::new_v4().to_string();
    let project_root = PathBuf::from(&org_row.project_path);
    let organization_id = org_row.id;

    // b1 (design 2026-06-24): in domain-keyed mode, drop providers that have no
    // {{domain}} query so a domain expansion never re-fires a company-name survey
    // of the parent org. Legacy company survey (domain=None) keeps all providers.
    let providers: Vec<ToolConfig> = if config
        .domain
        .as_deref()
        .map(str::trim)
        .is_some_and(|d| !d.is_empty())
    {
        providers
            .into_iter()
            .filter(provider_supports_domain)
            .collect()
    } else {
        providers
    };

    let mut provider_status = Vec::new();
    let mut evidence = Vec::new();
    let mut candidates = OrganizationCandidates::default();
    let mut profile_entries: Vec<ProfileFieldEntry> = Vec::new();
    let cli_limit = Arc::new(Semaphore::new(CLI_PROVIDER_CONCURRENCY));
    let http_limit = Arc::new(Semaphore::new(HTTP_PROVIDER_CONCURRENCY));
    let provider_count = providers.len();
    let provider_runs = stream::iter(providers.into_iter().map(|tool| {
        let cli_limit = Arc::clone(&cli_limit);
        let http_limit = Arc::clone(&http_limit);
        let project_root = &project_root;
        let run_id = &run_id;
        async move {
            let asset = tool.asset_intel.as_ref().ok_or_else(|| {
                GolishError::Validation(format!("tool '{}' has no asset_intel descriptor", tool.id))
            })?;
            match &asset.runtime {
                golish_pentest::models::AssetIntelRuntimeConfig::CliJson { .. } => {
                    let _permit = cli_limit.acquire_owned().await.map_err(|error| {
                        GolishError::Internal(format!(
                            "asset intel CLI concurrency limiter closed: {error}"
                        ))
                    })?;
                    run_cli_json_provider(
                        &tool,
                        scan_tools,
                        &pentest_config.tools_dir,
                        project_root,
                        run_id,
                        company_name,
                        config,
                        sink,
                    )
                    .await
                }
                golish_pentest::models::AssetIntelRuntimeConfig::HttpJson { .. } => {
                    let _permit = http_limit.acquire_owned().await.map_err(|error| {
                        GolishError::Internal(format!(
                            "asset intel HTTP concurrency limiter closed: {error}"
                        ))
                    })?;
                    run_http_json_provider(
                        pool,
                        &tool,
                        project_root,
                        run_id,
                        company_name,
                        config,
                        sink,
                    )
                    .await
                }
                golish_pentest::models::AssetIntelRuntimeConfig::NativeProvider { .. } => {
                    let _permit = http_limit.acquire_owned().await.map_err(|error| {
                        GolishError::Internal(format!(
                            "asset intel HTTP concurrency limiter closed: {error}"
                        ))
                    })?;
                    run_native_provider(
                        pool,
                        &tool,
                        project_root,
                        run_id,
                        company_name,
                        config,
                        sink,
                    )
                    .await
                }
            }
        }
    }))
    .buffered(provider_count.max(1))
    .collect::<Vec<_>>()
    .await;
    for provider_run in provider_runs {
        let (status, next_candidates, next_evidence, next_profile) = provider_run?;
        if provider_output_has_landable_records(&status, &next_evidence) {
            merge_candidates(&mut candidates, next_candidates);
            profile_entries.extend(next_profile);
        }
        evidence.push(next_evidence);
        provider_status.push(status);
    }
    remove_passive_ip_scope_entries(&mut profile_entries);

    // Master record write happens *before* candidate upsert. If the patch is
    // empty (no descriptor profile_fields fired) we skip the DB roundtrip to
    // avoid noise. The patch is the merged view across every provider —
    // duplicate values are collapsed so the master record stays canonical.
    if !profile_entries.is_empty() {
        if let Some(mut patch) = build_profile_patch_from_entries(&org_row.intel, &profile_entries)?
        {
            merge_profile_patch_with_existing(org_row, &mut patch);
            golish_db::repo::organizations::update_profile(pool, organization_id, &patch).await?;
            // Per-dimension freshness (design 2026-06-22 §3.2): genuine collection
            // site, so stamp `*_collected_at` for the intel coverage dimensions
            // this patch actually carried. The patch is a delta — a dimension is
            // Some only when entries for it were collected this run — so this
            // never over-stamps. WHOIS is not in ProfilePatch (separate whois
            // write path). OSINT presence = contacts / social_accounts /
            // business_systems (mirrors coverage_truth `has_osint`).
            use golish_db::repo::organizations::IntelDim;
            let mut dims: Vec<IntelDim> = Vec::new();
            if patch.asns.is_some() {
                dims.push(IntelDim::Asn);
            }
            if patch.certificates.is_some() {
                dims.push(IntelDim::Ct);
            }
            if patch.contacts.is_some()
                || patch.social_accounts.is_some()
                || patch.business_systems.is_some()
            {
                dims.push(IntelDim::Osint);
            }
            golish_db::repo::organizations::stamp_intel_collected_at(pool, organization_id, &dims)
                .await?;
        }
    }

    if candidate_queue_enabled(config) {
        let flat = flatten_candidates(&candidates);
        if !flat.is_empty() {
            // Persist into the cumulative review queue, but keep the returned
            // candidates scoped to this provider run. Landing/freshness must not
            // mistake historical queue entries for fresh observations.
            let _persisted =
                upsert_organization_candidates_for_org(pool, organization_id, flat).await?;
        }
    }
    let failed = provider_status
        .iter()
        .filter(|item| {
            matches!(
                item.status,
                AssetIntelProviderRunState::Failed | AssetIntelProviderRunState::Unavailable
            )
        })
        .count();
    let status = if failed == 0 {
        AssetIntelRunStatus::Completed
    } else if failed == provider_status.len() {
        AssetIntelRunStatus::Failed
    } else {
        AssetIntelRunStatus::Partial
    };
    let observed_domain_hosts = profile_entries
        .iter()
        .filter(|entry| {
            matches!(
                &entry.target_kind,
                golish_pentest::models::AssetIntelProfileFieldTarget::Scalar
            ) && entry.target_field == "domains"
        })
        .map(|entry| entry.value.clone())
        .collect();

    Ok(AssetIntelRun {
        run_id,
        status,
        provider_status,
        candidates,
        evidence,
        observed_domain_hosts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_queue_is_opt_in_for_legacy_compatibility() {
        assert!(!candidate_queue_enabled(&AssetIntelHydrateConfig::default()));
        assert!(candidate_queue_enabled(&AssetIntelHydrateConfig {
            create_candidates: Some(true),
            ..Default::default()
        }));
    }

    #[test]
    fn passive_provider_ip_ranges_do_not_expand_authorized_profile() {
        use golish_pentest::models::AssetIntelProfileFieldTarget as T;
        let mut entries = vec![
            ProfileFieldEntry {
                target_kind: T::Scalar,
                target_field: "ip_ranges".to_string(),
                value: "203.0.113.10".to_string(),
            },
            ProfileFieldEntry {
                target_kind: T::Scalar,
                target_field: "asns".to_string(),
                value: "AS64500".to_string(),
            },
        ];

        remove_passive_ip_scope_entries(&mut entries);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].target_field, "asns");
    }
}
