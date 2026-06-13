//! Company-lookup core (disambiguation / 纠名), extracted from the
//! `asset_intel_lookup_company` Tauri command body so the same logic serves
//! both the GUI modal and the `recon_lookup_company` agent tool
//! (设计 2026-06-13-engagement-scoping-fanout §6.2: scoping 纠名前置子步骤).
//! Behaviour is a verbatim move — provider selection, dedupe, sort, cap.

use super::super::capability::expand_provider_tools;
use super::super::*;

/// Hard cap so lookup result lists stay scannable (UI modal / agent tool
/// alike). Per-provider lookups can exceed this individually; we trim after
/// dedupe.
pub(crate) const LOOKUP_RESULTS_HARD_CAP: usize = 25;

/// Run the company lookup against every provider with an enabled lookup
/// descriptor (or the explicit `provider_ids` subset), then dedupe + sort by
/// confidence + cap. `keyword` must be non-empty (caller validates).
pub(crate) async fn lookup_company_matches(
    pentest_config: &golish_pentest::PentestConfig,
    keyword: &str,
    provider_ids: &[String],
    limit: Option<usize>,
) -> Result<AssetIntelLookupResult, GolishError> {
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

    // Expand multi-provider tools FIRST. ENScan declares its providers under
    // `asset_intel_providers` (not the single `asset_intel` field), so the lookup
    // descriptor lives inside a provider entry. Every other asset-intel path
    // (availability / discovery / descriptors) calls `expand_provider_tools`;
    // this one used to read the raw `scan.tools`, never saw ENScan's lookup, and
    // failed with "no provider with an enabled lookup descriptor" on every call.
    let expanded = expand_provider_tools(&scan.tools);
    // Select providers: explicit ids if given (must exist + have lookup),
    // otherwise every tool with a lookup descriptor regardless of `auto`.
    // Lookup is meant for "I want to disambiguate" so we don't apply the
    // auto.priority filter — the caller has already opted in.
    let selected: Vec<ToolConfig> = if provider_ids.is_empty() {
        expanded
            .into_iter()
            .filter(|t| {
                t.asset_intel
                    .as_ref()
                    .and_then(|a| a.lookup.as_ref())
                    .is_some_and(|l| l.enabled)
            })
            .collect()
    } else {
        let mut out = Vec::new();
        for provider_id in provider_ids {
            let Some(tool) = expanded
                .iter()
                .find(|t| provider_id_for_tool(t).as_deref() == Some(provider_id.as_str()))
            else {
                return Err(GolishError::NotFound(format!(
                    "asset intel provider '{provider_id}' is not registered"
                )));
            };
            out.push(tool.clone());
        }
        out
    };

    if selected.is_empty() {
        return Err(GolishError::Validation(
            "no asset_intel provider with an enabled lookup descriptor is available".into(),
        ));
    }

    let run_id = Uuid::new_v4().to_string();
    // Lookup writes nothing to organizations.intel; output is a per-call
    // scratch dir keyed by run_id so concurrent lookups don't collide.
    let project_root = pentest_config.tools_dir.clone();

    let mut provider_status = Vec::new();
    let mut all_matches = Vec::new();
    for tool in &selected {
        let (status, matches) = run_lookup_cli_provider(
            tool,
            &scan.tools,
            &pentest_config.tools_dir,
            &project_root,
            &run_id,
            keyword,
        )
        .await?;
        provider_status.push(status);
        all_matches.extend(matches);
    }

    let mut deduped = dedupe_lookup_matches(all_matches);
    deduped.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let limit = limit
        .unwrap_or(LOOKUP_RESULTS_HARD_CAP)
        .min(LOOKUP_RESULTS_HARD_CAP);
    deduped.truncate(limit);

    Ok(AssetIntelLookupResult {
        run_id,
        matches: deduped,
        provider_status,
    })
}
