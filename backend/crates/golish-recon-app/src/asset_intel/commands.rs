//! `#[tauri::command]` entry points for Asset Intel (Discover Assets).
//!
//! Split out of `mod.rs`; behaviour/signatures are unchanged. Relies on
//! `use super::*` to reach the runtime/service helpers + shared types.

use super::*;

#[tauri::command]
pub async fn asset_intel_list_providers(
    pentest: tauri::State<'_, ToolsConfigState>,
) -> Result<Vec<AssetIntelProviderDescriptor>, GolishError> {
    let pentest_config = pentest.0.get().await;
    let scan = golish_pentest::scan_asset_intel_sources(
        &pentest_config.toolsconfig_dir,
        &pentest_config.intel_providers_dir,
    );
    if !scan.success {
        return Err(GolishError::Internal(
            scan.error
                .unwrap_or_else(|| "toolsconfig scan failed".into()),
        ));
    }
    Ok(provider_descriptors_from_tools(&scan.tools))
}

#[tauri::command]
pub async fn asset_intel_lookup_company(
    state: tauri::State<'_, DbState>,
    pentest: tauri::State<'_, ToolsConfigState>,
    args: AssetIntelLookupRequest,
) -> Result<AssetIntelLookupResult, GolishError> {
    let pool = state.pool_ready().await?;
    if args.keyword.trim().is_empty() {
        return Err(GolishError::Validation(
            "keyword is required for asset intel lookup".into(),
        ));
    }

    let pentest_config = pentest.0.get().await;
    // Core moved to service/lookup_core.rs so the recon_lookup_company agent
    // tool (scoping 纠名, 设计 2026-06-13) shares the exact same path.
    lookup_company_matches(
        pool,
        &pentest_config,
        args.keyword.trim(),
        &args.provider_ids,
        args.limit,
    )
    .await
}

/// Legacy single-shot hydrate command.
///
/// Runs **every** auto-default provider against the given organization with
/// the same `company_name` input. Kept for backward compatibility with older
/// frontend callers and tests. New code should prefer the two-phase
/// orchestration commands:
/// - [`asset_intel_hydrate_subsidiaries`] for the discovery phase
/// - [`asset_intel_enrich_organization`] / [`asset_intel_enrich_batch`] for
///   the enrichment phase
///
/// See `docs/design/2026-05-22-asset-intel-two-phase-hydrate.md` for the
/// rationale.
#[tauri::command]
pub async fn asset_intel_hydrate(
    app: tauri::AppHandle,
    state: tauri::State<'_, DbState>,
    pentest: tauri::State<'_, ToolsConfigState>,
    args: AssetIntelHydrateArgs,
) -> Result<AssetIntelRun, GolishError> {
    let pool = state.pool_ready().await?;
    let organization_id: Uuid = args.organization_id.parse()?;
    let row = golish_db::repo::organizations::get_one(pool, organization_id)
        .await?
        .ok_or_else(|| GolishError::NotFound(format!("organization {}", args.organization_id)))?;
    let company_name = args.company_name.unwrap_or_else(|| row.name.clone());
    if company_name.trim().is_empty() {
        return Err(GolishError::Validation(
            "company_name is required for asset intel hydrate".into(),
        ));
    }

    let pentest_config = pentest.0.get().await;
    let scan = golish_pentest::scan_asset_intel_sources(
        &pentest_config.toolsconfig_dir,
        &pentest_config.intel_providers_dir,
    );
    if !scan.success {
        return Err(GolishError::Internal(
            scan.error
                .unwrap_or_else(|| "toolsconfig scan failed".into()),
        ));
    }

    let selected = select_asset_intel_providers(&scan.tools, &args.provider_ids)?;
    let sink = TauriEventEmitter::handle(app);

    run_providers_for_org(
        Some(&sink),
        pool,
        &pentest_config,
        &scan.tools,
        selected,
        &row,
        &company_name,
        &enrichment_hydrate_config(args.config),
    )
    .await
}

/// Tauri command · two-phase hydrate **discovery** entrypoint.
///
/// Runs only the providers that declare a `subsidiaries` capability
/// (currently enscan-go) against the master organization, then writes the
/// resulting child-org / target candidates back under the **master
/// organization's** candidate list — exactly like the legacy single-shot
/// hydrate, but with 0.zone-style enrichment providers held back for the
/// later enrich phase.
///
/// Frontend wires this to the master row's "查子公司" button.
#[tauri::command]
pub async fn asset_intel_hydrate_subsidiaries(
    app: tauri::AppHandle,
    state: tauri::State<'_, DbState>,
    pentest: tauri::State<'_, ToolsConfigState>,
    args: AssetIntelHydrateArgs,
) -> Result<AssetIntelRun, GolishError> {
    let pool = state.pool_ready().await?;
    let organization_id: Uuid = args.organization_id.parse()?;
    let row = golish_db::repo::organizations::get_one(pool, organization_id)
        .await?
        .ok_or_else(|| GolishError::NotFound(format!("organization {}", args.organization_id)))?;
    let company_name = args.company_name.unwrap_or_else(|| row.name.clone());
    if company_name.trim().is_empty() {
        return Err(GolishError::Validation(
            "company_name is required for asset intel hydrate subsidiaries".into(),
        ));
    }

    let pentest_config = pentest.0.get().await;
    let scan = golish_pentest::scan_asset_intel_sources(
        &pentest_config.toolsconfig_dir,
        &pentest_config.intel_providers_dir,
    );
    if !scan.success {
        return Err(GolishError::Internal(
            scan.error
                .unwrap_or_else(|| "toolsconfig scan failed".into()),
        ));
    }

    let selected = select_subsidiary_providers(&scan.tools, &args.provider_ids)?;
    if selected.is_empty() {
        return Err(GolishError::Validation(
            "no asset intel provider with a 'subsidiaries' capability is available".into(),
        ));
    }
    let sink = TauriEventEmitter::handle(app);

    let discovery_config = discovery_hydrate_config(args.config);
    let discovery_policy = selected
        .iter()
        .filter_map(|tool| tool.asset_intel.as_ref())
        .find(|asset| asset.discovery.auto_promote)
        .map(|asset| asset.discovery.clone())
        .unwrap_or_default();
    let mut run = run_providers_for_org(
        Some(&sink),
        pool,
        &pentest_config,
        &scan.tools,
        selected,
        &row,
        &company_name,
        &discovery_config,
    )
    .await?;
    if discovery_policy.auto_promote {
        let promotion =
            auto_promote_discovered_children(pool, &row, &run.candidates, &discovery_policy)
                .await?;
        run.evidence.push(promotion);
        run.candidates = OrganizationCandidates::default();
    }
    Ok(run)
}

/// Args for [`asset_intel_enrich_organization`].
///
/// Differences vs. [`AssetIntelHydrateArgs`]: no `company_name` override —
/// enrichment always uses the canonical `organization.name` so that
/// querying 0.zone for "中国平安" enriches the master org, while querying
/// for "平安银行" enriches that specific child. Letting callers override
/// the name would defeat the whole purpose of the two-phase split.
/// Tauri command · two-phase hydrate **enrichment** entrypoint (single org).
///
/// Runs only providers that do **not** declare a `subsidiaries` capability
/// (currently 0.zone et al.) against the given organization, using
/// `organization.name` as the query input. Candidates and master-record
/// profile updates land on the targeted org — not on its parent.
///
/// Frontend wires this to per-org "补字段" buttons.
#[tauri::command]
pub async fn asset_intel_enrich_organization(
    app: tauri::AppHandle,
    state: tauri::State<'_, DbState>,
    pentest: tauri::State<'_, ToolsConfigState>,
    args: AssetIntelEnrichOrganizationArgs,
) -> Result<AssetIntelRun, GolishError> {
    let pool = state.pool_ready().await?;
    let organization_id: Uuid = args.organization_id.parse()?;
    let row = golish_db::repo::organizations::get_one(pool, organization_id)
        .await?
        .ok_or_else(|| GolishError::NotFound(format!("organization {}", args.organization_id)))?;
    let company_name = row.name.clone();
    if company_name.trim().is_empty() {
        return Err(GolishError::Validation(
            "organization name is empty; cannot run enrichment".into(),
        ));
    }

    let pentest_config = pentest.0.get().await;
    let scan = golish_pentest::scan_asset_intel_sources(
        &pentest_config.toolsconfig_dir,
        &pentest_config.intel_providers_dir,
    );
    if !scan.success {
        return Err(GolishError::Internal(
            scan.error
                .unwrap_or_else(|| "toolsconfig scan failed".into()),
        ));
    }

    let selected = select_enrichment_providers(&scan.tools, &args.provider_ids)?;
    if selected.is_empty() {
        return Err(GolishError::Validation(
            "no asset intel enrichment provider (non-subsidiaries) is available".into(),
        ));
    }
    let sink = TauriEventEmitter::handle(app);

    run_providers_for_org(
        Some(&sink),
        pool,
        &pentest_config,
        &scan.tools,
        selected,
        &row,
        &company_name,
        &enrichment_hydrate_config(args.config),
    )
    .await
}

/// Tauri command · two-phase hydrate **enrichment** entrypoint (batch).
///
/// Resolves the parent organization, optionally includes it as the first
/// run, then iterates over every direct child (matched by
/// `parent_id = parent_organization_id`) and runs the enrichment provider
/// set against each in turn. Failures in one org don't abort the batch —
/// they just produce a `Failed` / `Partial` `AssetIntelRun` in the result
/// vector. Orgs with an empty `name` are skipped, not failed.
///
/// Frontend wires this to the master row's "批量补字段" button.
#[tauri::command]
pub async fn asset_intel_enrich_batch(
    app: tauri::AppHandle,
    state: tauri::State<'_, DbState>,
    pentest: tauri::State<'_, ToolsConfigState>,
    args: AssetIntelEnrichBatchArgs,
) -> Result<AssetIntelEnrichBatchResult, GolishError> {
    let pool = state.pool_ready().await?;
    let parent_id: Uuid = args.parent_organization_id.parse()?;
    let parent_row = golish_db::repo::organizations::get_one(pool, parent_id)
        .await?
        .ok_or_else(|| {
            GolishError::NotFound(format!("organization {}", args.parent_organization_id))
        })?;

    let pentest_config = pentest.0.get().await;
    let scan = golish_pentest::scan_asset_intel_sources(
        &pentest_config.toolsconfig_dir,
        &pentest_config.intel_providers_dir,
    );
    if !scan.success {
        return Err(GolishError::Internal(
            scan.error
                .unwrap_or_else(|| "toolsconfig scan failed".into()),
        ));
    }

    let selected_for_check = select_enrichment_providers(&scan.tools, &args.provider_ids)?;
    if selected_for_check.is_empty() {
        return Err(GolishError::Validation(
            "no asset intel enrichment provider (non-subsidiaries) is available".into(),
        ));
    }

    // Build target list: parent first (when requested), then children in the
    // same order organizations::list returns (parent_id NULLS FIRST → sort_order
    // → name). We re-fetch the parent row from the same list to keep IDs and
    // intel snapshots fresh inside the loop.
    let all_orgs = golish_db::repo::organizations::list(pool, &parent_row.project_path).await?;
    let include_self = args.include_self.unwrap_or(true);
    let mut targets: Vec<golish_db::models::Organization> = Vec::new();
    if include_self {
        targets.push(parent_row.clone());
    }
    for org in &all_orgs {
        if org.parent_id == Some(parent_id) {
            targets.push(org.clone());
        }
    }

    if targets.is_empty() {
        return Err(GolishError::Validation(format!(
            "organization {} has no children and include_self=false; nothing to enrich",
            args.parent_organization_id
        )));
    }

    let sink = TauriEventEmitter::handle(app);
    let mut runs: Vec<AssetIntelRun> = Vec::new();
    let mut skipped: Vec<AssetIntelEnrichBatchSkip> = Vec::new();
    for org in targets {
        let company_name = org.name.trim().to_string();
        if company_name.is_empty() {
            skipped.push(AssetIntelEnrichBatchSkip {
                organization_id: org.id.to_string(),
                reason: "empty_name".into(),
            });
            continue;
        }
        // Re-select providers per iteration so that hot-reloading toolsconfig
        // (e.g. operator disabling 0.zone mid-batch) doesn't keep firing the
        // disabled provider. Conservative but cheap: the scan was already
        // performed once above.
        let selected = match select_enrichment_providers(&scan.tools, &args.provider_ids) {
            Ok(p) if !p.is_empty() => p,
            Ok(_) => {
                skipped.push(AssetIntelEnrichBatchSkip {
                    organization_id: org.id.to_string(),
                    reason: "no_enrichment_provider".into(),
                });
                continue;
            }
            Err(err) => {
                skipped.push(AssetIntelEnrichBatchSkip {
                    organization_id: org.id.to_string(),
                    reason: format!("provider_select_error: {err}"),
                });
                continue;
            }
        };
        match run_providers_for_org(
            Some(&sink),
            pool,
            &pentest_config,
            &scan.tools,
            selected,
            &org,
            &company_name,
            &enrichment_hydrate_config(args.config.clone()),
        )
        .await
        {
            Ok(run) => runs.push(run),
            Err(err) => {
                // Don't abort the batch; record skip and continue. This keeps
                // a 0.zone quota-exhausted error from killing enrichment for
                // every later org.
                skipped.push(AssetIntelEnrichBatchSkip {
                    organization_id: org.id.to_string(),
                    reason: format!("run_failed: {err}"),
                });
            }
        }
    }

    Ok(AssetIntelEnrichBatchResult { runs, skipped })
}
