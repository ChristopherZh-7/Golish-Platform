//! Pure provider selection + descriptor helpers for asset intel.
//!
//! Expands multi-provider tool configs, builds provider descriptors, and
//! partitions the provider set into discovery (subsidiaries) vs enrichment
//! phases. No DB / IO. Re-exported from the parent module so existing call
//! sites keep using the bare function names.

use golish_pentest::models::ToolConfig;

use golish_app_core::GolishError;

use super::{
    AssetIntelCapability, AssetIntelIntegrationRequirement, AssetIntelProviderDescriptor,
    AssetIntelProviderRunState, AssetIntelProviderRunStatus, AssetIntelProviderStatus,
};

fn capability_from_str(value: &str) -> Option<AssetIntelCapability> {
    match value {
        "subsidiaries" => Some(AssetIntelCapability::Subsidiaries),
        "domains" => Some(AssetIntelCapability::Domains),
        "icp" => Some(AssetIntelCapability::Icp),
        "apps" => Some(AssetIntelCapability::Apps),
        "mini_programs" => Some(AssetIntelCapability::MiniPrograms),
        "social_accounts" => Some(AssetIntelCapability::SocialAccounts),
        "contacts" => Some(AssetIntelCapability::Contacts),
        _ => None,
    }
}

/// Expand tools with multi-provider declarations into per-provider virtual
/// `ToolConfig`s so the downstream selection / runtime code can keep working
/// against the existing "one tool → one `tool.asset_intel`" assumption.
///
/// - tools with `asset_intel_providers: Some(vec)`  → cloned once per enabled
///   provider; each virtual clone shares the parent's executable / install /
///   runtime metadata but has `asset_intel = Some(provider)` and
///   `asset_intel_providers = None`.
/// - tools with `asset_intel: Some(_)` (legacy single provider) → cloned 1:1
///   when the provider is enabled.
/// - tools with neither (regular pentest tools) → omitted (Asset Intel
///   selectors must only see Asset Intel-aware tools anyway).
///
/// The tool manager UI keeps using the raw `scan_toolsconfig` output, so it
/// still sees a single parent tool entry per JSON file; only the Asset Intel
/// pipeline calls this expander.
pub(crate) fn expand_provider_tools(tools: &[ToolConfig]) -> Vec<ToolConfig> {
    let mut out = Vec::new();
    for tool in tools {
        if let Some(providers) = tool.asset_intel_providers.as_ref() {
            for provider in providers {
                if !provider.enabled {
                    continue;
                }
                let mut virtual_tool = tool.clone();
                virtual_tool.asset_intel = Some(provider.clone());
                virtual_tool.asset_intel_providers = None;
                out.push(virtual_tool);
            }
        } else if let Some(asset) = tool.asset_intel.as_ref() {
            if !asset.enabled {
                continue;
            }
            out.push(tool.clone());
        }
    }
    out
}

pub(crate) fn provider_descriptors_from_tools(
    tools: &[ToolConfig],
) -> Vec<AssetIntelProviderDescriptor> {
    let expanded = expand_provider_tools(tools);
    expanded
        .iter()
        .filter_map(|tool| {
            let asset = tool.asset_intel.as_ref()?;
            if !asset.enabled {
                return None;
            }
            let id = if asset.provider_id.trim().is_empty() {
                tool.id.clone()
            } else {
                asset.provider_id.clone()
            };
            let display_name = if asset.display_name.trim().is_empty() {
                tool.name.clone()
            } else {
                asset.display_name.clone()
            };
            let capabilities = asset
                .capabilities
                .iter()
                .filter_map(|capability| capability_from_str(capability))
                .collect();
            let requires_integration = asset.requires_integration.as_ref().map(|requirement| {
                AssetIntelIntegrationRequirement {
                    tool_id: requirement.tool_id.clone(),
                    group_ids: requirement.group_ids.clone(),
                }
            });

            Some(AssetIntelProviderDescriptor {
                id,
                display_name,
                requires_integration,
                capabilities,
                status: AssetIntelProviderStatus::Available,
            })
        })
        .collect()
}

pub(crate) fn provider_id_for_tool(tool: &ToolConfig) -> Option<String> {
    let asset = tool.asset_intel.as_ref()?;
    if !asset.enabled {
        return None;
    }
    Some(if asset.provider_id.trim().is_empty() {
        tool.id.clone()
    } else {
        asset.provider_id.clone()
    })
}

/// Capability literal used to distinguish discovery (subsidiaries) providers
/// from enrichment providers when wiring the two-phase hydrate flow.
///
/// Kept as a single source of truth so we don't end up with `"subsidiaries"`
/// string literals scattered across the file; pair with
/// [`provider_has_subsidiaries`] for the actual check.
const SUBSIDIARIES_CAPABILITY: &str = "subsidiaries";

/// Returns `true` when the tool's asset intel descriptor declares a
/// "subsidiaries" capability — i.e. it is suitable for the **discovery**
/// phase (finding child companies of a master org).
///
/// Used by [`select_subsidiary_providers`] and [`select_enrichment_providers`]
/// to partition the global provider set into the two phases described in
/// `docs/design/2026-05-22-asset-intel-two-phase-hydrate.md`.
pub(crate) fn provider_has_subsidiaries(tool: &ToolConfig) -> bool {
    tool.asset_intel
        .as_ref()
        .map(|asset| {
            asset
                .capabilities
                .iter()
                .any(|cap| cap.eq_ignore_ascii_case(SUBSIDIARIES_CAPABILITY))
        })
        .unwrap_or(false)
}

/// Select providers eligible for the **discovery** phase
/// (`asset_intel_hydrate_subsidiaries`).
///
/// Reuses [`select_asset_intel_providers`] for the auto/priority/explicit-id
/// semantics, then keeps only those whose capability set contains
/// `subsidiaries`. When `requested` is non-empty we explicitly reject any
/// requested provider that does **not** have the capability, rather than
/// silently dropping it — callers should know they asked for the wrong tool.
pub(crate) fn select_subsidiary_providers(
    tools: &[ToolConfig],
    requested: &[String],
) -> Result<Vec<ToolConfig>, GolishError> {
    let base = select_asset_intel_providers(tools, requested)?;
    if requested.is_empty() {
        return Ok(base.into_iter().filter(provider_has_subsidiaries).collect());
    }
    let mut out = Vec::with_capacity(base.len());
    for tool in base {
        if !provider_has_subsidiaries(&tool) {
            let id = provider_id_for_tool(&tool).unwrap_or_else(|| tool.id.clone());
            return Err(GolishError::Validation(format!(
                "asset intel provider '{id}' does not declare a 'subsidiaries' capability"
            )));
        }
        out.push(tool);
    }
    Ok(out)
}

/// Select providers eligible for the **enrichment** phase
/// (`asset_intel_enrich_organization` and `asset_intel_enrich_batch`).
///
/// Mirror of [`select_subsidiary_providers`] but keeps providers whose
/// capability set does **not** include `subsidiaries`. enscan-go has both
/// `subsidiaries` and `domains/apps/...`, but we still treat it as
/// discovery-only because it already collected those other fields during
/// the discovery phase — re-running it during enrichment would double the
/// cost without adding new data.
pub(crate) fn select_enrichment_providers(
    tools: &[ToolConfig],
    requested: &[String],
) -> Result<Vec<ToolConfig>, GolishError> {
    let base = select_asset_intel_providers(tools, requested)?;
    if requested.is_empty() {
        return Ok(base
            .into_iter()
            .filter(|t| !provider_has_subsidiaries(t))
            .collect());
    }
    let mut out = Vec::with_capacity(base.len());
    for tool in base {
        if provider_has_subsidiaries(&tool) {
            let id = provider_id_for_tool(&tool).unwrap_or_else(|| tool.id.clone());
            return Err(GolishError::Validation(format!(
                "asset intel provider '{id}' is a discovery provider; use asset_intel_hydrate_subsidiaries instead"
            )));
        }
        out.push(tool);
    }
    Ok(out)
}

pub(crate) fn select_asset_intel_providers(
    tools: &[ToolConfig],
    requested: &[String],
) -> Result<Vec<ToolConfig>, GolishError> {
    let mut providers: Vec<ToolConfig> = expand_provider_tools(tools)
        .into_iter()
        .filter(|tool| provider_id_for_tool(tool).is_some())
        .collect();

    if requested.is_empty() {
        providers.retain(|tool| {
            tool.asset_intel
                .as_ref()
                .is_some_and(|asset| asset.auto.default)
        });
        providers.sort_by(|a, b| {
            let a_asset = a.asset_intel.as_ref().expect("asset_intel descriptor");
            let b_asset = b.asset_intel.as_ref().expect("asset_intel descriptor");
            b_asset
                .auto
                .priority
                .cmp(&a_asset.auto.priority)
                .then_with(|| {
                    provider_id_for_tool(a)
                        .unwrap_or_default()
                        .cmp(&provider_id_for_tool(b).unwrap_or_default())
                })
        });
        return Ok(providers);
    }

    let mut selected = Vec::new();
    for provider_id in requested {
        let Some(tool) = providers
            .iter()
            .find(|tool| provider_id_for_tool(tool).as_deref() == Some(provider_id.as_str()))
        else {
            return Err(GolishError::NotFound(format!(
                "asset intel provider '{provider_id}'"
            )));
        };
        selected.push(tool.clone());
    }
    Ok(selected)
}

pub(crate) fn provider_output_is_trusted(status: &AssetIntelProviderRunStatus) -> bool {
    matches!(
        status.status,
        AssetIntelProviderRunState::Completed | AssetIntelProviderRunState::CheckedEmpty
    )
}

/// Decide whether normalized records may be landed independently of the
/// provider-wide terminal state. Native multi-query providers can return real
/// records from successful queries while a sibling query fails; those records
/// remain valid observations even though the provider must stay retryable.
/// Other failed runtimes do not get this exception because they lack typed
/// successful-query evidence.
pub(crate) fn provider_output_has_landable_records(
    status: &AssetIntelProviderRunStatus,
    evidence: &serde_json::Value,
) -> bool {
    provider_output_is_trusted(status)
        || (status.status == AssetIntelProviderRunState::Failed
            && evidence
                .get("succeededQueries")
                .and_then(serde_json::Value::as_u64)
                .is_some_and(|count| count > 0))
}
