//! Company-lookup core (disambiguation / 纠名), extracted from the
//! `asset_intel_lookup_company` Tauri command body so the same logic serves
//! both the GUI modal and the `recon_lookup_company` agent tool
//! (设计 2026-06-13-engagement-scoping-fanout §6.2: scoping 纠名前置子步骤).
//! Behaviour is a verbatim move — provider selection, dedupe, sort, cap.

use super::super::capability::expand_provider_tools;
use super::super::types::{AssetIntelLookupNextStep, AssetIntelLookupResolutionStatus};
use super::super::*;

/// Hard cap so lookup result lists stay scannable (UI modal / agent tool
/// alike). Per-provider lookups can exceed this individually; we trim after
/// dedupe.
pub(crate) const LOOKUP_RESULTS_HARD_CAP: usize = 25;
const SCOPING_STRUCTURED_FALLBACK_PROVIDER_ID: &str = "0.zone";

/// Resolve company candidates in strict structured-source order.
///
/// Enterprise registry providers run first. The 0.zone `org` adapter runs only
/// when that first tier cannot produce one deterministic identity candidate.
/// The result remains a candidate outcome: only the upper Scoping receipt flow
/// may turn it into a confirmed Company Identity.
pub(crate) async fn lookup_company_matches(
    pool: &sqlx::PgPool,
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
    let mut selected: Vec<ToolConfig> = if provider_ids.is_empty() {
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
    selected.sort_by_key(|tool| provider_id_for_tool(tool).unwrap_or_else(|| tool.id.clone()));

    let (fallback_providers, enterprise_providers): (Vec<_>, Vec<_>) =
        selected.iter().partition(is_structured_fallback_provider);

    let run_id = Uuid::new_v4().to_string();
    // Lookup writes nothing to organizations.intel; output is a per-call
    // scratch dir keyed by run_id so concurrent lookups don't collide.
    let project_root = pentest_config.tools_dir.clone();

    let mut provider_status = Vec::new();
    let mut all_matches = Vec::new();
    run_lookup_tier(
        pool,
        &enterprise_providers,
        &scan.tools,
        &pentest_config.tools_dir,
        &project_root,
        &run_id,
        keyword,
        &mut provider_status,
        &mut all_matches,
    )
    .await;

    let enterprise_matches = dedupe_lookup_matches(all_matches.clone());
    if needs_structured_fallback(keyword, enterprise_providers.len(), &enterprise_matches) {
        run_lookup_tier(
            pool,
            &fallback_providers,
            &scan.tools,
            &pentest_config.tools_dir,
            &project_root,
            &run_id,
            keyword,
            &mut provider_status,
            &mut all_matches,
        )
        .await;
    }

    let mut deduped = dedupe_lookup_matches(all_matches);
    deduped.sort_by(|a, b| {
        b.confidence
            .total_cmp(&a.confidence)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.credit_code.cmp(&b.credit_code))
            .then_with(|| a.provider_id.cmp(&b.provider_id))
    });
    let unique = has_unique_lookup_candidate(keyword, &deduped);
    let limit = limit
        .unwrap_or(LOOKUP_RESULTS_HARD_CAP)
        .min(LOOKUP_RESULTS_HARD_CAP);
    deduped.truncate(limit);

    Ok(AssetIntelLookupResult {
        run_id,
        matches: deduped,
        provider_status,
        resolution_status: if unique {
            AssetIntelLookupResolutionStatus::UniqueCandidate
        } else {
            AssetIntelLookupResolutionStatus::Unresolved
        },
        next_step: if unique {
            AssetIntelLookupNextStep::None
        } else {
            AssetIntelLookupNextStep::NeedsPublicSearch
        },
    })
}

fn is_structured_fallback_provider(tool: &&ToolConfig) -> bool {
    provider_id_for_tool(tool).as_deref() == Some(SCOPING_STRUCTURED_FALLBACK_PROVIDER_ID)
}

#[allow(clippy::too_many_arguments)]
async fn run_lookup_tier(
    pool: &sqlx::PgPool,
    selected: &[&ToolConfig],
    tools: &[ToolConfig],
    tools_dir: &std::path::Path,
    project_root: &std::path::Path,
    run_id: &str,
    keyword: &str,
    provider_status: &mut Vec<AssetIntelProviderRunStatus>,
    all_matches: &mut Vec<LookupCompanyMatch>,
) {
    for tool in selected {
        let provider_id = provider_id_for_tool(tool).unwrap_or_else(|| tool.id.clone());
        match run_lookup_provider(pool, tool, tools, tools_dir, project_root, run_id, keyword).await
        {
            Ok((status, matches)) => {
                provider_status.push(status);
                all_matches.extend(matches);
            }
            Err(error) => provider_status.push(AssetIntelProviderRunStatus {
                provider_id,
                status: AssetIntelProviderRunState::Failed,
                message: format!("company lookup provider failed: {error}"),
            }),
        }
    }
}

/// Confidence and provider order are deliberately absent from this decision.
/// A single merged identity is deterministic only when it carries a legal
/// registration code or exactly matches the requested legal name.
fn has_unique_lookup_candidate(keyword: &str, matches: &[LookupCompanyMatch]) -> bool {
    let [candidate] = matches else {
        return false;
    };
    candidate
        .credit_code
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || candidate
            .name
            .trim()
            .to_lowercase()
            .eq(&keyword.trim().to_lowercase())
}

fn needs_structured_fallback(
    keyword: &str,
    enterprise_provider_count: usize,
    enterprise_matches: &[LookupCompanyMatch],
) -> bool {
    enterprise_provider_count == 0 || !has_unique_lookup_candidate(keyword, enterprise_matches)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(name: &str, credit_code: Option<&str>, confidence: f64) -> LookupCompanyMatch {
        LookupCompanyMatch {
            provider_id: "enterprise".into(),
            name: name.into(),
            credit_code: credit_code.map(str::to_string),
            industry: None,
            legal_representative: None,
            address: None,
            registered_at: None,
            confidence,
            evidence: serde_json::json!({}),
        }
    }

    #[test]
    fn unique_lookup_candidate_never_uses_first_or_highest_confidence() {
        assert!(!has_unique_lookup_candidate(
            "Acme",
            &[candidate("Possible Acme Group", None, 0.99)]
        ));
        assert!(!has_unique_lookup_candidate(
            "Acme",
            &[
                candidate("Acme", Some("CODE-1"), 0.99),
                candidate("Acme Holdings", Some("CODE-2"), 0.1),
            ]
        ));
    }

    #[test]
    fn unique_lookup_candidate_requires_one_legal_identity_signal() {
        assert!(has_unique_lookup_candidate(
            "Acme",
            &[candidate("ACME", None, 0.1)]
        ));
        assert!(has_unique_lookup_candidate(
            "Acme",
            &[candidate("Acme Technology Co Ltd", Some("CODE-1"), 0.1)]
        ));
        assert!(!has_unique_lookup_candidate("Acme", &[]));
    }

    #[test]
    fn structured_fallback_runs_only_when_enterprise_tier_is_not_unique() {
        assert!(!needs_structured_fallback(
            "Acme",
            1,
            &[candidate("Acme Technology Co Ltd", Some("CODE-1"), 0.1)]
        ));
        assert!(needs_structured_fallback(
            "Acme",
            1,
            &[candidate("Possible Acme Group", None, 0.99)]
        ));
        assert!(needs_structured_fallback("Acme", 0, &[]));
    }
}
