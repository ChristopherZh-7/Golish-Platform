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
        evidence.push(next_evidence);
        if provider_output_is_trusted(&status) {
            merge_candidates(&mut candidates, next_candidates);
            profile_entries.extend(next_profile);
        }
        provider_status.push(status);
    }

    // Master record write happens *before* candidate upsert. If the patch is
    // empty (no descriptor profile_fields fired) we skip the DB roundtrip to
    // avoid noise. The patch is the merged view across every provider —
    // duplicate values are collapsed so the master record stays canonical.
    if !profile_entries.is_empty() {
        if let Some(mut patch) = build_profile_patch_from_entries(&org_row.intel, &profile_entries)?
        {
            merge_profile_patch_with_existing(org_row, &mut patch);
            golish_db::repo::organizations::update_profile(pool, organization_id, &patch).await?;
        }
    }

    if config.create_candidates.unwrap_or(true) {
        let flat = flatten_candidates(&candidates);
        if !flat.is_empty() {
            candidates =
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

    Ok(AssetIntelRun {
        run_id,
        status,
        provider_status,
        candidates,
        evidence,
    })
}
