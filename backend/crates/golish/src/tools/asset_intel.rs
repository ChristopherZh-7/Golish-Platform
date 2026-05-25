//! Asset Intel service for Discover Assets engagements.
//!
//! Phase 1 keeps this layer provider-agnostic: the workspace asks for
//! candidates, providers return normalized records, and only approved
//! candidates become scope in later phases.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::Mutex as TokioMutex;
use uuid::Uuid;

use golish_core::{emit_opt, EventEmitterHandle};
use golish_pentest::models::ToolConfig;

use crate::error::GolishError;
use crate::event_emitter::TauriEventEmitter;
use crate::state::DbState;
use crate::tools::organizations::{
    upsert_organization_candidates_for_org, OrganizationCandidate, OrganizationCandidateKind,
    OrganizationCandidates,
};
use crate::tools::pentest::PentestState;

/// Tauri event name used for all Asset Intel streaming events.
///
/// The frontend listens once on this channel and filters payloads by `runId`.
/// Kept as a constant so backend + frontend share a single source of truth
/// (frontend re-imports the literal in `lib/api/asset-intel.ts`).
pub const ASSET_INTEL_EVENT: &str = "asset-intel:event";

/// Provider stream source — where a progress line came from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetIntelStreamSource {
    Stdout,
    Stderr,
    System,
}

/// Provider batch source — where a candidate batch was normalized from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetIntelBatchSource {
    Stdout,
    Artifact,
    Http,
}

/// Provider runtime kind exposed in `provider_started` events.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetIntelProviderRuntimeKind {
    CliJson,
    HttpJson,
}

/// Streaming event payload — generic across runtime kinds.
///
/// Emitted via `EventEmitterHandle` on `ASSET_INTEL_EVENT`. Frontend matches
/// on `kind` to decide rendering. Carries `runId` + `providerId` so multiple
/// concurrent runs / providers can multiplex on a single channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AssetIntelStreamEvent {
    ProviderStarted {
        run_id: String,
        provider_id: String,
        display_name: String,
        runtime: AssetIntelProviderRuntimeKind,
    },
    ProviderProgress {
        run_id: String,
        provider_id: String,
        message: String,
        stream: AssetIntelStreamSource,
    },
    ProviderBatch {
        run_id: String,
        provider_id: String,
        candidates: OrganizationCandidates,
        source: AssetIntelBatchSource,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artifact: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
    },
    ProviderCompleted {
        run_id: String,
        provider_id: String,
        status: AssetIntelProviderRunStatus,
        candidate_count: usize,
    },
}

fn emit_event(sink: Option<&EventEmitterHandle>, event: AssetIntelStreamEvent) {
    emit_opt(sink, ASSET_INTEL_EVENT, &event);
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssetIntelCapability {
    Subsidiaries,
    Domains,
    Icp,
    Apps,
    MiniPrograms,
    SocialAccounts,
    Contacts,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssetIntelProviderStatus {
    Available,
    Unavailable,
    Deprecated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AssetIntelIntegrationRequirement {
    pub tool_id: String,
    pub group_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AssetIntelProviderDescriptor {
    pub id: String,
    pub display_name: String,
    pub requires_integration: Option<AssetIntelIntegrationRequirement>,
    pub capabilities: Vec<AssetIntelCapability>,
    pub status: AssetIntelProviderStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetIntelProviderRecord {
    pub kind: OrganizationCandidateKind,
    pub label: String,
    pub value: String,
    pub confidence: f64,
    #[serde(default)]
    pub evidence: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AssetIntelHydrateConfig {
    #[serde(default)]
    pub min_ownership_percent: Option<String>,
    #[serde(default)]
    pub depth: Option<String>,
    #[serde(default)]
    pub include_branches: Option<bool>,
    #[serde(default)]
    pub create_candidates: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetIntelHydrateArgs {
    pub organization_id: String,
    #[serde(default)]
    pub company_name: Option<String>,
    #[serde(default)]
    pub provider_ids: Vec<String>,
    #[serde(default)]
    pub config: AssetIntelHydrateConfig,
}

fn enrichment_hydrate_config(mut config: AssetIntelHydrateConfig) -> AssetIntelHydrateConfig {
    config.create_candidates = Some(false);
    config
}

#[cfg(test)]
fn enrichment_hydrate_config_for_organization(
    args: &AssetIntelEnrichOrganizationArgs,
) -> AssetIntelHydrateConfig {
    enrichment_hydrate_config(args.config.clone())
}

fn discovery_hydrate_config(mut config: AssetIntelHydrateConfig) -> AssetIntelHydrateConfig {
    config.create_candidates = Some(false);
    config
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssetIntelRunStatus {
    Completed,
    Partial,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssetIntelProviderRunState {
    Completed,
    CheckedEmpty,
    Unavailable,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetIntelProviderRunStatus {
    pub provider_id: String,
    pub status: AssetIntelProviderRunState,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetIntelRun {
    pub run_id: String,
    pub status: AssetIntelRunStatus,
    pub provider_status: Vec<AssetIntelProviderRunStatus>,
    pub candidates: OrganizationCandidates,
    pub evidence: Vec<Value>,
}

/// One disambiguation hit returned by `asset_intel_lookup_company`.
///
/// Lookups intentionally stop at "enterprise_info" (no invest / branch
/// traversal) so they're cheap and fit in a single UI modal: the user picks
/// one canonical company → its `name` + `credit_code` then drive the full
/// hydrate run. Every field except `name` and `provider_id` is optional
/// because different upstream sources expose different subsets.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LookupCompanyMatch {
    pub provider_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub industry: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legal_representative: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registered_at: Option<String>,
    pub confidence: f64,
    #[serde(default)]
    pub evidence: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetIntelLookupRequest {
    pub keyword: String,
    /// Restrict the lookup to specific providers (by id). When empty, every
    /// provider whose `asset_intel.lookup` is enabled runs sequentially and
    /// results are merged.
    #[serde(default)]
    pub provider_ids: Vec<String>,
    /// Cap on returned matches across providers. UI uses a default modal
    /// list size, but the backend hard-caps at 25 to avoid runaway lists.
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetIntelLookupResult {
    pub run_id: String,
    pub matches: Vec<LookupCompanyMatch>,
    pub provider_status: Vec<AssetIntelProviderRunStatus>,
}

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
fn expand_provider_tools(tools: &[ToolConfig]) -> Vec<ToolConfig> {
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

fn provider_descriptors_from_tools(tools: &[ToolConfig]) -> Vec<AssetIntelProviderDescriptor> {
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

pub fn normalize_provider_records(
    provider_id: &str,
    run_id: &str,
    fetched_at: u64,
    records: Vec<AssetIntelProviderRecord>,
) -> OrganizationCandidates {
    let mut candidates = OrganizationCandidates::default();
    for record in records {
        let candidate = OrganizationCandidate {
            id: format!(
                "{}:{}:{}",
                match record.kind {
                    OrganizationCandidateKind::Organization => "org",
                    OrganizationCandidateKind::Target => "target",
                },
                provider_id,
                record.value.trim()
            ),
            kind: record.kind,
            label: record.label,
            value: record.value,
            source: provider_id.to_string(),
            confidence: record.confidence,
            status: "needs_review".to_string(),
            evidence: serde_json::json!({
                "provider": provider_id,
                "runId": run_id,
                "raw": record.evidence,
            }),
            created_at: fetched_at,
        };
        match candidate.kind {
            OrganizationCandidateKind::Organization => candidates.organizations.push(candidate),
            OrganizationCandidateKind::Target => candidates.targets.push(candidate),
        }
    }
    candidates
}

fn merge_candidate_evidence(
    existing: &mut OrganizationCandidate,
    incoming: &OrganizationCandidate,
) {
    fn evidence_sources(evidence: &Value) -> Vec<Value> {
        evidence
            .get("sources")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| vec![evidence.clone()])
    }

    let mut sources = evidence_sources(&existing.evidence);
    for source in evidence_sources(&incoming.evidence) {
        if !sources.iter().any(|item| item == &source) {
            sources.push(source);
        }
    }

    if incoming.confidence > existing.confidence {
        existing.confidence = incoming.confidence;
    }
    if let Some(obj) = existing.evidence.as_object_mut() {
        obj.insert("sources".into(), Value::Array(sources));
    } else {
        existing.evidence = serde_json::json!({
            "primary": existing.evidence,
            "sources": sources,
        });
    }
}

fn dedupe_candidates(candidates: OrganizationCandidates) -> OrganizationCandidates {
    fn dedupe_bucket(items: Vec<OrganizationCandidate>) -> Vec<OrganizationCandidate> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for item in items {
            let key = item.value.trim().to_lowercase();
            if seen.insert(key) {
                out.push(item);
            } else if let Some(existing) = out.iter_mut().find(|existing| {
                existing
                    .value
                    .trim()
                    .eq_ignore_ascii_case(item.value.trim())
            }) {
                merge_candidate_evidence(existing, &item);
            }
        }
        out
    }

    OrganizationCandidates {
        organizations: dedupe_bucket(candidates.organizations),
        targets: dedupe_bucket(candidates.targets),
    }
}

fn merge_candidates(target: &mut OrganizationCandidates, next: OrganizationCandidates) {
    target.organizations.extend(next.organizations);
    target.targets.extend(next.targets);
    let deduped = dedupe_candidates(std::mem::take(target));
    *target = deduped;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AutoPromoteSkipReason {
    EmptyName,
    MissingOwnership,
    OwnershipBelowThreshold,
    InactiveStatus,
    Duplicate,
    PolicyFilterFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AutoPromoteChildDecision {
    candidate: OrganizationCandidate,
    promote: bool,
    reason: Option<AutoPromoteSkipReason>,
    ownership_percent: Option<f64>,
}

fn parse_ownership_percent(raw: &str) -> Option<f64> {
    let cleaned = raw.trim().trim_end_matches('%').replace(',', "");
    if cleaned.is_empty() {
        return None;
    }
    cleaned.parse::<f64>().ok()
}

fn discovery_policy_threshold(
    policy: &golish_pentest::models::AssetIntelDiscoveryConfig,
) -> Option<f64> {
    use golish_pentest::models::AssetIntelNormalizeFilterOp as Op;
    policy
        .promote_when
        .iter()
        .find(|filter| {
            filter.field == policy.ownership_field && matches!(filter.op, Op::Gte | Op::Gt | Op::Eq)
        })
        .and_then(|filter| parse_ownership_percent(&filter.value))
}

fn candidate_raw_field<'a>(candidate: &'a OrganizationCandidate, field: &str) -> Option<&'a str> {
    candidate
        .evidence
        .get("raw")
        .and_then(|raw| raw.get(field))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "-")
}

fn auto_promote_child_decisions(
    candidates: &OrganizationCandidates,
    policy: &golish_pentest::models::AssetIntelDiscoveryConfig,
    existing_child_names: &HashSet<String>,
) -> Vec<AutoPromoteChildDecision> {
    let mut seen = existing_child_names.clone();
    let threshold = discovery_policy_threshold(policy);
    let mut decisions = Vec::new();
    for candidate in &candidates.organizations {
        let name = candidate.value.trim();
        if name.is_empty() {
            decisions.push(AutoPromoteChildDecision {
                candidate: candidate.clone(),
                promote: false,
                reason: Some(AutoPromoteSkipReason::EmptyName),
                ownership_percent: None,
            });
            continue;
        }

        let raw = candidate.evidence.get("raw").unwrap_or(&Value::Null);
        if !filter_passes(raw, &policy.promote_when) {
            let status = candidate_raw_field(candidate, "status");
            let ownership = candidate_raw_field(candidate, &policy.ownership_field)
                .and_then(parse_ownership_percent);
            let reason = if threshold.is_some_and(|min| ownership.is_some_and(|value| value < min))
            {
                AutoPromoteSkipReason::OwnershipBelowThreshold
            } else if status.is_some_and(|value| value != "开业" && value != "存续") {
                AutoPromoteSkipReason::InactiveStatus
            } else {
                AutoPromoteSkipReason::PolicyFilterFailed
            };
            decisions.push(AutoPromoteChildDecision {
                candidate: candidate.clone(),
                promote: false,
                reason: Some(reason),
                ownership_percent: ownership,
            });
            continue;
        }

        let ownership = candidate_raw_field(candidate, &policy.ownership_field)
            .and_then(parse_ownership_percent);
        if ownership.is_none() {
            decisions.push(AutoPromoteChildDecision {
                candidate: candidate.clone(),
                promote: false,
                reason: Some(AutoPromoteSkipReason::MissingOwnership),
                ownership_percent: None,
            });
            continue;
        };
        let percent = ownership.expect("checked is_some above");

        if existing_child_names.contains(&name.to_lowercase()) {
            decisions.push(AutoPromoteChildDecision {
                candidate: candidate.clone(),
                promote: false,
                reason: Some(AutoPromoteSkipReason::Duplicate),
                ownership_percent: Some(percent),
            });
            continue;
        }

        let dedupe_key = policy
            .dedupe_by
            .iter()
            .filter_map(|field| candidate_raw_field(candidate, field))
            .next()
            .unwrap_or(name)
            .to_lowercase();
        if !seen.insert(dedupe_key) {
            decisions.push(AutoPromoteChildDecision {
                candidate: candidate.clone(),
                promote: false,
                reason: Some(AutoPromoteSkipReason::Duplicate),
                ownership_percent: Some(percent),
            });
            continue;
        }

        decisions.push(AutoPromoteChildDecision {
            candidate: candidate.clone(),
            promote: true,
            reason: None,
            ownership_percent: Some(percent),
        });
    }
    decisions
}

fn clear_engagement_candidates_from_intel(mut intel: Value) -> Result<Value, GolishError> {
    if !intel.is_object() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    let root = intel.as_object_mut().ok_or_else(|| {
        GolishError::Internal("organization intel must be a JSON object".to_string())
    })?;
    if let Some(engagement) = root.get_mut("engagement").and_then(Value::as_object_mut) {
        engagement.remove("candidates");
    }
    Ok(intel)
}

async fn clear_engagement_candidates_for_org(
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

async fn auto_promote_discovered_children(
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

/// Run the descriptor's candidate rules + profile_fields rules against a
/// single raw JSON document. Returns the candidate bucket (for the review
/// queue) plus the profile field entries (master record write). Callers
/// always get both — even when one or the other is empty — so call sites
/// don't have to remember to extract twice.
fn normalize_json_with_descriptor(
    provider_id: &str,
    run_id: &str,
    fetched_at: u64,
    normalize: &golish_pentest::models::AssetIntelNormalizeConfig,
    raw: &Value,
) -> (OrganizationCandidates, Vec<ProfileFieldEntry>) {
    fn collect_rule_records(
        kind: OrganizationCandidateKind,
        rules: &[golish_pentest::models::AssetIntelNormalizeRule],
        raw: &Value,
        out: &mut Vec<AssetIntelProviderRecord>,
    ) {
        for rule in rules {
            for item in select_json_values(raw, &rule.path) {
                // `when` clauses are AND'd; an empty when always keeps the
                // match (legacy behaviour). This is where descriptor-driven
                // filters like `invest.scale >= 51` cut down noise without
                // touching Rust.
                if !filter_passes(item, &rule.when) {
                    continue;
                }
                let Some(label) = resolve_field_ref(item, &rule.label) else {
                    continue;
                };
                let Some(value) = resolve_field_ref(item, &rule.value) else {
                    continue;
                };
                out.push(enscan_record(
                    kind.clone(),
                    &label,
                    &value,
                    rule.confidence,
                    item,
                ));
            }
        }
    }

    let mut records = Vec::new();
    collect_rule_records(
        OrganizationCandidateKind::Organization,
        &normalize.organization,
        raw,
        &mut records,
    );
    collect_rule_records(
        OrganizationCandidateKind::Target,
        &normalize.target,
        raw,
        &mut records,
    );

    let candidates = normalize_provider_records(provider_id, run_id, fetched_at, records);
    let profile_entries = extract_profile_field_entries(&normalize.profile_fields, raw);
    (candidates, profile_entries)
}

/// One value lifted out of provider raw JSON destined for the organization
/// profile (master record), not for the candidate review queue.
///
/// Producers populate this; the hydrate top-level merges all entries from
/// every provider into a single `OrganizationProfilePatch` and writes once
/// via `update_profile`. We keep target_kind + target_field separate so we
/// can route values to scalar columns, `intel` json keys, and contact lists
/// without spreading per-target logic across every call site.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileFieldEntry {
    pub target_kind: golish_pentest::models::AssetIntelProfileFieldTarget,
    pub target_field: String,
    pub value: String,
}

fn apply_profile_transform(
    raw: &str,
    transform: &golish_pentest::models::AssetIntelProfileFieldTransform,
) -> String {
    use golish_pentest::models::AssetIntelProfileFieldTransform as T;
    match transform {
        T::None => raw.to_string(),
        T::Trim => raw.trim().to_string(),
        T::Lower => raw.trim().to_lowercase(),
        T::Upper => raw.trim().to_uppercase(),
        T::Asn => normalize_asn(raw),
    }
}

fn normalize_asn(raw: &str) -> String {
    let upper = raw.trim().to_uppercase();
    let digits = upper.strip_prefix("AS").unwrap_or(&upper).trim();
    if digits.is_empty() || digits.len() > 10 || !digits.chars().all(|ch| ch.is_ascii_digit()) {
        return String::new();
    }
    format!("AS{digits}")
}

const TEAM_CYMRU_WHOIS_ADDR: &str = "whois.cymru.com:43";
const TEAM_CYMRU_ASN_LOOKUP_TIMEOUT_SECS: u64 = 8;
const TEAM_CYMRU_ASN_LOOKUP_IP_LIMIT: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq)]
struct IpAsnMapping {
    asn: String,
}

fn parse_ip_for_asn_lookup(raw: &str) -> Option<IpAddr> {
    let without_cidr = raw.trim().split_once('/').map_or(raw.trim(), |(ip, _)| ip);
    without_cidr.parse::<IpAddr>().ok()
}

fn is_public_ipv4_for_asn_lookup(ip: &Ipv4Addr) -> bool {
    let [a, b, c, _d] = ip.octets();
    !matches!(
        (a, b, c),
        (0, _, _)
            | (10, _, _)
            | (100, 64..=127, _)
            | (127, _, _)
            | (169, 254, _)
            | (172, 16..=31, _)
            | (192, 0, 0)
            | (192, 0, 2)
            | (192, 168, _)
            | (198, 18..=19, _)
            | (198, 51, 100)
            | (203, 0, 113)
            | (224..=255, _, _)
    )
}

fn is_public_ipv6_for_asn_lookup(ip: &Ipv6Addr) -> bool {
    let segments = ip.segments();
    let first = segments[0];
    if ip.is_unspecified() || ip.is_loopback() || ip.is_multicast() {
        return false;
    }
    if (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80 {
        return false;
    }
    segments[0] != 0x2001 || segments[1] != 0x0db8
}

fn is_public_ip_for_asn_lookup(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_ipv4_for_asn_lookup(ip),
        IpAddr::V6(ip) => is_public_ipv6_for_asn_lookup(ip),
    }
}

fn collect_public_ips_for_asn_lookup(entries: &[ProfileFieldEntry]) -> Vec<IpAddr> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for entry in entries {
        if entry.target_kind != golish_pentest::models::AssetIntelProfileFieldTarget::Scalar
            || entry.target_field != "ip_ranges"
        {
            continue;
        }
        let Some(ip) = parse_ip_for_asn_lookup(&entry.value) else {
            continue;
        };
        if !is_public_ip_for_asn_lookup(&ip) || !seen.insert(ip) {
            continue;
        }
        out.push(ip);
        if out.len() >= TEAM_CYMRU_ASN_LOOKUP_IP_LIMIT {
            break;
        }
    }
    out
}

fn parse_team_cymru_asn_response(raw: &str) -> Vec<IpAsnMapping> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for line in raw.lines() {
        let mut cols = line.split('|').map(str::trim);
        let Some(asn_raw) = cols.next() else {
            continue;
        };
        let Some(ip_raw) = cols.next() else {
            continue;
        };
        if asn_raw.eq_ignore_ascii_case("as") {
            continue;
        }
        let asn = normalize_asn(asn_raw);
        let Some(ip) = parse_ip_for_asn_lookup(ip_raw) else {
            continue;
        };
        if asn.is_empty() || !seen.insert((ip, asn.clone())) {
            continue;
        }
        out.push(IpAsnMapping { asn });
    }
    out
}

fn profile_asn_entries_from_mappings(mappings: &[IpAsnMapping]) -> Vec<ProfileFieldEntry> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for mapping in mappings {
        if seen.insert(mapping.asn.to_ascii_uppercase()) {
            out.push(ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "asns".into(),
                value: mapping.asn.clone(),
            });
        }
    }
    out
}

async fn lookup_team_cymru_asns(ips: &[IpAddr]) -> Result<Vec<IpAsnMapping>, String> {
    if ips.is_empty() {
        return Ok(Vec::new());
    }
    let timeout = Duration::from_secs(TEAM_CYMRU_ASN_LOOKUP_TIMEOUT_SECS);
    let mut stream = tokio::time::timeout(
        timeout,
        tokio::net::TcpStream::connect(TEAM_CYMRU_WHOIS_ADDR),
    )
    .await
    .map_err(|_| "timed out connecting to Team Cymru whois".to_string())?
    .map_err(|err| format!("connect failed: {err}"))?;
    let query = format!(
        "begin\nverbose\n{}\nend\n",
        ips.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    );
    tokio::time::timeout(timeout, stream.write_all(query.as_bytes()))
        .await
        .map_err(|_| "timed out writing Team Cymru query".to_string())?
        .map_err(|err| format!("write failed: {err}"))?;
    let mut response = String::new();
    tokio::time::timeout(timeout, stream.read_to_string(&mut response))
        .await
        .map_err(|_| "timed out reading Team Cymru response".to_string())?
        .map_err(|err| format!("read failed: {err}"))?;
    Ok(parse_team_cymru_asn_response(&response))
}

async fn enrich_0zone_asns_from_ip_ranges(
    provider_id: &str,
    run_id: &str,
    profile_entries: &mut Vec<ProfileFieldEntry>,
    sink: Option<&EventEmitterHandle>,
) -> Option<Value> {
    if provider_id != "0.zone"
        || profile_entries
            .iter()
            .any(|entry| entry.target_field == "asns" && !entry.value.trim().is_empty())
    {
        return None;
    }
    let ips = collect_public_ips_for_asn_lookup(profile_entries);
    if ips.is_empty() {
        return None;
    }
    emit_event(
        sink,
        AssetIntelStreamEvent::ProviderProgress {
            run_id: run_id.to_string(),
            provider_id: provider_id.to_string(),
            message: format!("deriving ASN from {} public IP(s)", ips.len()),
            stream: AssetIntelStreamSource::System,
        },
    );
    match lookup_team_cymru_asns(&ips).await {
        Ok(mappings) => {
            let derived = profile_asn_entries_from_mappings(&mappings);
            let asn_count = derived.len();
            profile_entries.extend(derived);
            Some(serde_json::json!({
                "requestId": "team-cymru-ip-to-asn",
                "state": if asn_count == 0 { "checked_empty" } else { "completed" },
                "queriedIpCount": ips.len(),
                "asnCount": asn_count,
            }))
        }
        Err(error) => {
            tracing::warn!(
                provider = %provider_id,
                run_id,
                error,
                "asset_intel derived ASN lookup failed"
            );
            Some(serde_json::json!({
                "requestId": "team-cymru-ip-to-asn",
                "state": "failed",
                "queriedIpCount": ips.len(),
                "error": error,
            }))
        }
    }
}

/// Walk the descriptor's `lookup.normalize` mapping over a raw JSON
/// document and produce `LookupCompanyMatch` entries — one per item at
/// `normalize.path` that has a usable `name`. Missing optional fields stay
/// `None`; the static `default_confidence` is used unless `score` resolves
/// to a parseable f64.
pub fn extract_lookup_matches(
    provider_id: &str,
    config: &golish_pentest::models::AssetIntelLookupConfig,
    raw: &Value,
) -> Vec<LookupCompanyMatch> {
    let normalize = &config.normalize;
    let mut out = Vec::new();
    for item in select_json_values(raw, &normalize.path) {
        let Some(name) = resolve_field_ref(item, &normalize.name) else {
            continue;
        };
        let credit_code = normalize
            .credit_code
            .as_ref()
            .and_then(|field_ref| resolve_field_ref(item, field_ref));
        let industry = normalize
            .industry
            .as_ref()
            .and_then(|field_ref| resolve_field_ref(item, field_ref));
        let legal_representative = normalize
            .legal_representative
            .as_ref()
            .and_then(|field_ref| resolve_field_ref(item, field_ref));
        let address = normalize
            .address
            .as_ref()
            .and_then(|field_ref| resolve_field_ref(item, field_ref));
        let registered_at = normalize
            .registered_at
            .as_ref()
            .and_then(|field_ref| resolve_field_ref(item, field_ref));
        let confidence = normalize
            .score
            .as_ref()
            .and_then(|field_ref| resolve_field_ref(item, field_ref))
            .and_then(|raw_score| raw_score.parse::<f64>().ok())
            .unwrap_or(normalize.default_confidence);
        out.push(LookupCompanyMatch {
            provider_id: provider_id.to_string(),
            name,
            credit_code,
            industry,
            legal_representative,
            address,
            registered_at,
            confidence,
            evidence: item.clone(),
        });
    }
    out
}

/// Walk the descriptor's `profile_fields` rules over a single raw JSON
/// document and return every resolved (target_kind, target_field, value)
/// triple. Caller is responsible for deduping / merging across providers.
pub fn extract_profile_field_entries(
    rules: &[golish_pentest::models::AssetIntelProfileFieldRule],
    raw: &Value,
) -> Vec<ProfileFieldEntry> {
    let mut out = Vec::new();
    for rule in rules {
        for item in select_json_values(raw, &rule.path) {
            // `when` clauses are AND'd; empty = always keep. Typical use:
            // drop ENScan's "-" placeholder before it lands in contacts.
            if !filter_passes(item, &rule.when) {
                continue;
            }
            let Some(raw_value) = resolve_field_ref(item, &rule.source_field) else {
                continue;
            };
            let value = apply_profile_transform(&raw_value, &rule.transform);
            if value.trim().is_empty() {
                continue;
            }
            out.push(ProfileFieldEntry {
                target_kind: rule.target_kind.clone(),
                target_field: rule.target_field.clone(),
                value,
            });
        }
    }
    out
}

/// Returns true when every filter clause matches the given JSON item.
///
/// Operators apply via [`apply_filter_op`]:
/// numeric ops (`gte`, `gt`, `lte`, `lt`) try f64 first then string compare;
/// equality ops (`eq`, `ne`) try f64 first then case-insensitive string compare;
/// `exists` / `missing` only check field presence + non-empty value;
/// `contains` does case-insensitive substring compare.
fn filter_passes(
    item: &Value,
    filters: &[golish_pentest::models::AssetIntelNormalizeFilter],
) -> bool {
    filters.iter().all(|clause| {
        let raw = resolve_value_field(item, &clause.field);
        apply_filter_op(&clause.op, raw.as_deref(), &clause.value)
    })
}

fn apply_filter_op(
    op: &golish_pentest::models::AssetIntelNormalizeFilterOp,
    raw: Option<&str>,
    compare_to: &str,
) -> bool {
    use golish_pentest::models::AssetIntelNormalizeFilterOp as Op;
    let value = raw.unwrap_or("").trim();
    let compare_to_trimmed = compare_to.trim();
    let parse = |s: &str| -> Option<f64> {
        s.trim()
            .trim_end_matches('%')
            .replace(',', "")
            .parse::<f64>()
            .ok()
    };

    match op {
        Op::Exists => !value.is_empty(),
        Op::Missing => value.is_empty(),
        Op::Contains => {
            !value.is_empty()
                && value
                    .to_lowercase()
                    .contains(&compare_to_trimmed.to_lowercase())
        }
        Op::Eq | Op::Ne => {
            let equal = match (parse(value), parse(compare_to_trimmed)) {
                (Some(a), Some(b)) => (a - b).abs() < f64::EPSILON,
                _ => value.eq_ignore_ascii_case(compare_to_trimmed),
            };
            matches!(op, Op::Eq) == equal
        }
        Op::Gte | Op::Gt | Op::Lte | Op::Lt => {
            let (Some(a), Some(b)) = (parse(value), parse(compare_to_trimmed)) else {
                // Non-numeric comparison falls through to string ordering so
                // descriptors that compare e.g. dates "2024-01-01" still work.
                let ord = value.cmp(compare_to_trimmed);
                return match op {
                    Op::Gte => !matches!(ord, std::cmp::Ordering::Less),
                    Op::Gt => matches!(ord, std::cmp::Ordering::Greater),
                    Op::Lte => !matches!(ord, std::cmp::Ordering::Greater),
                    Op::Lt => matches!(ord, std::cmp::Ordering::Less),
                    _ => unreachable!(),
                };
            };
            match op {
                Op::Gte => a >= b,
                Op::Gt => a > b,
                Op::Lte => a <= b,
                Op::Lt => a < b,
                _ => unreachable!(),
            }
        }
    }
}

fn select_json_values<'a>(raw: &'a Value, path: &str) -> Vec<&'a Value> {
    if path == "$" {
        return vec![raw];
    }
    let Some(field) = path
        .strip_prefix("$..")
        .and_then(|rest| rest.strip_suffix("[*]"))
    else {
        return Vec::new();
    };

    fn visit<'a>(value: &'a Value, field: &str, out: &mut Vec<&'a Value>) {
        match value {
            Value::Object(map) => {
                if let Some(found) = map.get(field) {
                    match found {
                        Value::Array(items) => out.extend(items),
                        other => out.push(other),
                    }
                }
                for child in map.values() {
                    visit(child, field, out);
                }
            }
            Value::Array(items) => {
                for item in items {
                    visit(item, field, out);
                }
            }
            _ => {}
        }
    }

    let mut out = Vec::new();
    visit(raw, field, &mut out);
    out
}

fn resolve_field_ref(
    value: &Value,
    field_ref: &golish_pentest::models::AssetIntelFieldRef,
) -> Option<String> {
    match field_ref {
        golish_pentest::models::AssetIntelFieldRef::Field(field) => {
            resolve_value_field(value, field)
        }
        golish_pentest::models::AssetIntelFieldRef::FirstOf(fields) => fields
            .iter()
            .find_map(|field| resolve_value_field(value, field)),
    }
}

fn resolve_value_field(value: &Value, field: &str) -> Option<String> {
    let resolved = if field.is_empty() {
        match value {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            Value::Bool(flag) => Some(flag.to_string()),
            _ => None,
        }
    } else {
        golish_core::utils::resolve_json_path(value, field)
    }?;
    let trimmed = resolved.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn provider_id_for_tool(tool: &ToolConfig) -> Option<String> {
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
fn provider_has_subsidiaries(tool: &ToolConfig) -> bool {
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
fn select_subsidiary_providers(
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
fn select_enrichment_providers(
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

fn select_asset_intel_providers(
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

fn render_asset_intel_skill_args(
    skill_args: &str,
    company_name: &str,
    out_dir: &Path,
    config: &AssetIntelHydrateConfig,
    arg_bindings: &std::collections::HashMap<String, String>,
) -> String {
    fn render_template(
        template: &str,
        company_name: &str,
        out_dir: &Path,
        config: &AssetIntelHydrateConfig,
    ) -> String {
        template
            .replace("{{org}}", company_name)
            .replace("{{company_name}}", company_name)
            .replace("{{out_dir}}", &out_dir.to_string_lossy())
            .replace(
                "{{config.min_ownership_percent}}",
                config.min_ownership_percent.as_deref().unwrap_or_default(),
            )
            .replace(
                "{{config.depth}}",
                config.depth.as_deref().unwrap_or_default(),
            )
            .replace(
                "{{config.include_branches}}",
                if config.include_branches.unwrap_or(false) {
                    "true"
                } else {
                    "false"
                },
            )
    }

    let mut rendered = render_template(skill_args, company_name, out_dir, config);
    let mut binding_keys: Vec<&String> = arg_bindings.keys().collect();
    binding_keys.sort_by(|a, b| {
        fn order(key: &str) -> usize {
            match key {
                "min_ownership_percent" => 0,
                "depth" => 1,
                "include_branches" => 2,
                _ => 100,
            }
        }
        order(a).cmp(&order(b)).then_with(|| a.cmp(b))
    });
    for key in binding_keys {
        let enabled = match key.as_str() {
            "min_ownership_percent" => config
                .min_ownership_percent
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()),
            "depth" => config
                .depth
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()),
            "include_branches" => config.include_branches.unwrap_or(false),
            _ => false,
        };
        if enabled {
            let binding = render_template(&arg_bindings[key], company_name, out_dir, config);
            if !binding.trim().is_empty() {
                rendered.push(' ');
                rendered.push_str(binding.trim());
            }
        }
    }
    rendered
}

fn provider_output_is_trusted(status: &AssetIntelProviderRunStatus) -> bool {
    matches!(
        status.status,
        AssetIntelProviderRunState::Completed | AssetIntelProviderRunState::CheckedEmpty
    )
}

fn split_command_args(rendered: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    for ch in rendered.chars() {
        match ch {
            '"' => in_quote = !in_quote,
            c if c.is_whitespace() && !in_quote => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

fn extract_secret_refs_from_str(value: &str, out: &mut HashSet<String>) {
    let mut rest = value;
    while let Some(start) = rest.find("{{secret:") {
        let after_start = &rest[start + "{{secret:".len()..];
        let Some(end) = after_start.find("}}") else {
            break;
        };
        let key = after_start[..end].trim();
        if !key.is_empty() {
            out.insert(key.to_string());
        }
        rest = &after_start[end + "}}".len()..];
    }
}

fn extract_secret_refs_from_json(value: &Value, out: &mut HashSet<String>) {
    match value {
        Value::String(text) => extract_secret_refs_from_str(text, out),
        Value::Array(items) => {
            for item in items {
                extract_secret_refs_from_json(item, out);
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                extract_secret_refs_from_json(item, out);
            }
        }
        _ => {}
    }
}

fn collect_http_secret_refs(
    requests: &[golish_pentest::models::AssetIntelHttpRequest],
) -> HashSet<String> {
    let mut refs = HashSet::new();
    for request in requests {
        extract_secret_refs_from_str(&request.url, &mut refs);
        for value in request.headers.values() {
            extract_secret_refs_from_str(value, &mut refs);
        }
        for value in request.form.values() {
            extract_secret_refs_from_str(value, &mut refs);
        }
        extract_secret_refs_from_json(&request.json, &mut refs);
    }
    refs
}

async fn read_vault_secret(
    pool: &sqlx::PgPool,
    tool_id: &str,
    group_id: &str,
    field_key: &str,
) -> Result<Option<String>, GolishError> {
    let name = format!("{tool_id}.{group_id}.{field_key}");
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT value FROM vault_entries \
         WHERE name = $1 \
         ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(&name)
    .fetch_optional(pool)
    .await?;
    if let Some((value,)) = row {
        return golish_core::vault::deobfuscate(&value)
            .map(Some)
            .map_err(|err| GolishError::Internal(format!("vault deobfuscate failed: {err}")));
    }

    if group_id == "default" {
        let legacy: Option<(String,)> = sqlx::query_as(
            "SELECT value FROM vault_entries \
             WHERE name = $1 AND entry_type = 'api_key' \
             ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(tool_id)
        .fetch_optional(pool)
        .await?;
        if let Some((value,)) = legacy {
            return golish_core::vault::deobfuscate(&value)
                .map(Some)
                .map_err(|err| GolishError::Internal(format!("vault deobfuscate failed: {err}")));
        }
    }

    Ok(None)
}

async fn resolve_http_secrets(
    pool: &sqlx::PgPool,
    asset: &golish_pentest::models::AssetIntelToolConfig,
    requests: &[golish_pentest::models::AssetIntelHttpRequest],
) -> Result<Result<HashMap<String, String>, Vec<String>>, GolishError> {
    let refs = collect_http_secret_refs(requests);
    if refs.is_empty() {
        return Ok(Ok(HashMap::new()));
    }
    let Some(requirement) = asset.requires_integration.as_ref() else {
        return Ok(Err(refs.into_iter().collect()));
    };

    let mut values = HashMap::new();
    let mut missing = Vec::new();
    for key in refs {
        let mut found = None;
        for group_id in &requirement.group_ids {
            if let Some(value) =
                read_vault_secret(pool, &requirement.tool_id, group_id, &key).await?
            {
                found = Some(value);
                break;
            }
        }
        if let Some(value) = found {
            values.insert(key, value);
        } else {
            missing.push(key);
        }
    }

    if missing.is_empty() {
        Ok(Ok(values))
    } else {
        Ok(Err(missing))
    }
}

fn render_http_template(
    template: &str,
    company_name: &str,
    config: &AssetIntelHydrateConfig,
    secrets: &HashMap<String, String>,
) -> String {
    let mut rendered = template
        .replace("{{org}}", company_name)
        .replace("{{company_name}}", company_name)
        .replace(
            "{{config.min_ownership_percent}}",
            config.min_ownership_percent.as_deref().unwrap_or_default(),
        )
        .replace(
            "{{config.depth}}",
            config.depth.as_deref().unwrap_or_default(),
        );
    for (key, value) in secrets {
        rendered = rendered.replace(&format!("{{{{secret:{key}}}}}"), value);
    }
    rendered
}

fn render_http_json_value(
    value: &Value,
    company_name: &str,
    config: &AssetIntelHydrateConfig,
    secrets: &HashMap<String, String>,
) -> Value {
    match value {
        Value::String(text) => {
            Value::String(render_http_template(text, company_name, config, secrets))
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| render_http_json_value(item, company_name, config, secrets))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| {
                    (
                        key.clone(),
                        render_http_json_value(item, company_name, config, secrets),
                    )
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

async fn run_http_json_provider(
    pool: &sqlx::PgPool,
    tool: &ToolConfig,
    run_id: &str,
    company_name: &str,
    config: &AssetIntelHydrateConfig,
    sink: Option<&EventEmitterHandle>,
) -> Result<
    (
        AssetIntelProviderRunStatus,
        OrganizationCandidates,
        Value,
        Vec<ProfileFieldEntry>,
    ),
    GolishError,
> {
    let Some(asset) = tool.asset_intel.as_ref() else {
        return Err(GolishError::Validation(format!(
            "tool '{}' has no asset_intel descriptor",
            tool.id
        )));
    };
    let provider_id = provider_id_for_tool(tool).unwrap_or_else(|| tool.id.clone());
    let display_name = if asset.display_name.trim().is_empty() {
        tool.name.clone()
    } else {
        asset.display_name.clone()
    };
    let golish_pentest::models::AssetIntelRuntimeConfig::HttpJson { requests } = &asset.runtime
    else {
        return Err(GolishError::Validation(format!(
            "tool '{}' is not an http_json provider",
            tool.id
        )));
    };

    // Sequential per-request loop; accumulator stays here so every early
    // return path can hand whatever has already been hydrated up to the
    // hydrate orchestrator (we don't want to drop master-record fields
    // because a later request 500'd).
    let mut profile_entries: Vec<ProfileFieldEntry> = Vec::new();

    emit_event(
        sink,
        AssetIntelStreamEvent::ProviderStarted {
            run_id: run_id.to_string(),
            provider_id: provider_id.clone(),
            display_name,
            runtime: AssetIntelProviderRuntimeKind::HttpJson,
        },
    );

    if requests.is_empty() {
        let status = AssetIntelProviderRunStatus {
            provider_id: provider_id.clone(),
            status: AssetIntelProviderRunState::Unavailable,
            message: "http_json provider has no requests".into(),
        };
        emit_event(
            sink,
            AssetIntelStreamEvent::ProviderCompleted {
                run_id: run_id.to_string(),
                provider_id: provider_id.clone(),
                status: status.clone(),
                candidate_count: 0,
            },
        );
        return Ok((
            status,
            OrganizationCandidates::default(),
            serde_json::json!({
                "provider": provider_id,
                "runId": run_id,
                "state": "unavailable",
                "reason": "no_requests",
            }),
            profile_entries,
        ));
    }

    let secrets = match resolve_http_secrets(pool, asset, requests).await? {
        Ok(values) => values,
        Err(missing) => {
            let status = AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::Unavailable,
                message: format!("missing integration secret(s): {}", missing.join(", ")),
            };
            emit_event(
                sink,
                AssetIntelStreamEvent::ProviderCompleted {
                    run_id: run_id.to_string(),
                    provider_id: provider_id.clone(),
                    status: status.clone(),
                    candidate_count: 0,
                },
            );
            return Ok((
                status,
                OrganizationCandidates::default(),
                serde_json::json!({
                    "provider": provider_id,
                    "runId": run_id,
                    "state": "unavailable",
                    "reason": "missing_secrets",
                    "missing": missing,
                }),
                profile_entries,
            ));
        }
    };

    let client = reqwest::Client::builder()
        .user_agent(concat!("golish/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|err| GolishError::Internal(format!("http client build failed: {err}")))?;
    let mut candidates = OrganizationCandidates::default();
    let mut request_evidence = Vec::new();
    for request in requests {
        emit_event(
            sink,
            AssetIntelStreamEvent::ProviderProgress {
                run_id: run_id.to_string(),
                provider_id: provider_id.clone(),
                message: format!("requesting '{}' ({})", request.id, request.method),
                stream: AssetIntelStreamSource::System,
            },
        );
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|err| GolishError::Validation(format!("bad HTTP method: {err}")))?;
        let url = render_http_template(&request.url, company_name, config, &secrets);
        let timeout_secs = request.timeout_secs.clamp(1, 120);
        tracing::info!(
            provider = %provider_id,
            run_id,
            request_id = %request.id,
            timeout_secs,
            "running asset_intel http_json request"
        );
        let mut builder = client
            .request(method, &url)
            .timeout(Duration::from_secs(timeout_secs));
        for (name, value) in &request.headers {
            builder = builder.header(
                name,
                render_http_template(value, company_name, config, &secrets),
            );
        }
        if !request.form.is_empty() {
            let form: HashMap<String, String> = request
                .form
                .iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        render_http_template(value, company_name, config, &secrets),
                    )
                })
                .collect();
            let mut encoded = url::form_urlencoded::Serializer::new(String::new());
            for (key, value) in &form {
                encoded.append_pair(key, value);
            }
            builder = builder
                .header(
                    reqwest::header::CONTENT_TYPE,
                    "application/x-www-form-urlencoded",
                )
                .body(encoded.finish());
        } else if !request.json.is_null() {
            builder = builder.json(&render_http_json_value(
                &request.json,
                company_name,
                config,
                &secrets,
            ));
        }

        let response = match builder.send().await {
            Ok(response) => response,
            Err(err) => {
                tracing::warn!(
                    provider = %provider_id,
                    run_id,
                    request_id = %request.id,
                    error = %err,
                    "asset_intel http_json request failed"
                );
                let status = AssetIntelProviderRunStatus {
                    provider_id: provider_id.clone(),
                    status: AssetIntelProviderRunState::Failed,
                    message: format!("request '{}' failed: {err}", request.id),
                };
                let count = candidates.organizations.len() + candidates.targets.len();
                emit_event(
                    sink,
                    AssetIntelStreamEvent::ProviderCompleted {
                        run_id: run_id.to_string(),
                        provider_id: provider_id.clone(),
                        status: status.clone(),
                        candidate_count: count,
                    },
                );
                return Ok((
                    status,
                    candidates,
                    serde_json::json!({
                        "provider": provider_id,
                        "runId": run_id,
                        "state": "failed",
                        "reason": "request_failed",
                        "requestId": request.id,
                        "error": err.to_string(),
                        "candidateCount": count,
                    }),
                    profile_entries,
                ));
            }
        };
        let http_status = response.status();
        let body = response.text().await.unwrap_or_default();
        let preview: String = body.chars().take(512).collect();
        if !http_status.is_success() {
            tracing::warn!(
                provider = %provider_id,
                run_id,
                request_id = %request.id,
                status = http_status.as_u16(),
                "asset_intel http_json request returned non-success status"
            );
            let status = AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::Failed,
                message: format!("request '{}' returned HTTP {http_status}", request.id),
            };
            let count = candidates.organizations.len() + candidates.targets.len();
            emit_event(
                sink,
                AssetIntelStreamEvent::ProviderCompleted {
                    run_id: run_id.to_string(),
                    provider_id: provider_id.clone(),
                    status: status.clone(),
                    candidate_count: count,
                },
            );
            return Ok((
                status,
                candidates,
                serde_json::json!({
                    "provider": provider_id,
                    "runId": run_id,
                    "state": "failed",
                    "reason": "http_status",
                    "requestId": request.id,
                    "status": http_status.as_u16(),
                    "preview": preview,
                    "candidateCount": count,
                }),
                profile_entries,
            ));
        }

        if let Some((next, profile)) =
            normalize_json_document(&provider_id, run_id, &asset.normalize, &body)
        {
            profile_entries.extend(profile);
            let added_total = next.organizations.len() + next.targets.len();
            if added_total > 0 {
                let mut delta = OrganizationCandidates::default();
                for item in next.organizations.iter() {
                    delta.organizations.push(item.clone());
                }
                for item in next.targets.iter() {
                    delta.targets.push(item.clone());
                }
                merge_candidates(&mut candidates, next);
                emit_event(
                    sink,
                    AssetIntelStreamEvent::ProviderBatch {
                        run_id: run_id.to_string(),
                        provider_id: provider_id.clone(),
                        candidates: delta,
                        source: AssetIntelBatchSource::Http,
                        artifact: None,
                        request_id: Some(request.id.clone()),
                    },
                );
            }
        }
        request_evidence.push(serde_json::json!({
            "requestId": request.id,
            "status": http_status.as_u16(),
        }));
    }

    if let Some(evidence) =
        enrich_0zone_asns_from_ip_ranges(&provider_id, run_id, &mut profile_entries, sink).await
    {
        request_evidence.push(evidence);
    }

    let total = candidates.organizations.len() + candidates.targets.len();
    let state = if total == 0 {
        AssetIntelProviderRunState::CheckedEmpty
    } else {
        AssetIntelProviderRunState::Completed
    };
    tracing::info!(
        provider = %provider_id,
        run_id,
        candidate_count = total,
        state = ?state,
        "asset_intel http_json provider completed"
    );
    let status = AssetIntelProviderRunStatus {
        provider_id: provider_id.clone(),
        status: state,
        message: if total == 0 {
            format!("{provider_id} completed with no candidates")
        } else {
            format!("{provider_id} normalized {total} candidate(s)")
        },
    };
    emit_event(
        sink,
        AssetIntelStreamEvent::ProviderCompleted {
            run_id: run_id.to_string(),
            provider_id: provider_id.clone(),
            status: status.clone(),
            candidate_count: total,
        },
    );
    Ok((
        status,
        candidates,
        serde_json::json!({
            "provider": provider_id,
            "runId": run_id,
            "state": if total == 0 { "checked_empty" } else { "completed" },
            "candidateCount": total,
            "requests": request_evidence,
        }),
        profile_entries,
    ))
}

fn normalize_json_document(
    provider_id: &str,
    run_id: &str,
    normalize: &golish_pentest::models::AssetIntelNormalizeConfig,
    raw: &str,
) -> Option<(OrganizationCandidates, Vec<ProfileFieldEntry>)> {
    let value = serde_json::from_str::<Value>(raw.trim()).ok()?;
    Some(normalize_json_with_descriptor(
        provider_id,
        run_id,
        now_millis(),
        normalize,
        &value,
    ))
}

fn asset_intel_provider_output_dir(
    project_root: &Path,
    run_id: &str,
    provider_id: &str,
) -> PathBuf {
    golish_projects::file_storage::tool_output_dir(project_root, "asset-intel")
        .join(run_id)
        .join(provider_id)
}

/// Max characters of any single stdout/stderr line forwarded to the frontend.
///
/// Long PTY/OSC dumps (terminal control sequences) can balloon individual
/// lines into multi-kilobyte chunks; truncating here keeps the event stream
/// useful and bounds memory cost per emit.
const PROVIDER_PROGRESS_LINE_LIMIT: usize = 512;

/// Polling interval for the `out_dir` artifact watcher (cli_json runtime).
///
/// The frontend's perceived "first candidate in N seconds" is bounded by
/// this interval. Tuned to a sweet spot: small enough to feel live (<1s),
/// large enough to avoid hot-looping `read_dir` during long scans.
const ARTIFACT_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Shared, normalize-and-emit-once accumulator used by the cli_json runner.
///
/// Keeping the accumulator + the cancel flag in a single Arc-wrapped struct
/// lets us hand a cheap clone to every background task (stdout reader,
/// stderr reader, artifact watcher) without juggling individual Arcs.
#[derive(Debug)]
struct CliJsonStreamShared {
    candidates: TokioMutex<OrganizationCandidates>,
    /// Profile field entries lifted out of the same raw JSON documents.
    /// Stored separately from candidates because they target the master
    /// record (credit_code / industry / contacts / intel keys), not the
    /// review queue. The hydrate top-level merges these into a single
    /// `OrganizationProfilePatch` after the provider finishes.
    profile_entries: TokioMutex<Vec<ProfileFieldEntry>>,
    progress_buffer: TokioMutex<String>,
    cancel: AtomicBool,
}

impl CliJsonStreamShared {
    fn new() -> Self {
        Self {
            candidates: TokioMutex::new(OrganizationCandidates::default()),
            profile_entries: TokioMutex::new(Vec::new()),
            progress_buffer: TokioMutex::new(String::new()),
            cancel: AtomicBool::new(false),
        }
    }
}

fn truncate_progress_line(raw: &str) -> String {
    let cleaned = raw.trim_end_matches(['\r', '\n']).trim();
    if cleaned.chars().count() <= PROVIDER_PROGRESS_LINE_LIMIT {
        cleaned.to_string()
    } else {
        let mut out: String = cleaned.chars().take(PROVIDER_PROGRESS_LINE_LIMIT).collect();
        out.push_str(" … (truncated)");
        out
    }
}

/// Try to normalize a single stdout line as JSON; emit a Batch if it yields
/// candidates. Non-JSON or empty-result lines are returned to the caller so
/// they can be emitted as Progress instead.
async fn handle_stdout_line(
    line: &str,
    provider_id: &str,
    run_id: &str,
    normalize: &golish_pentest::models::AssetIntelNormalizeConfig,
    shared: &CliJsonStreamShared,
    sink: Option<&EventEmitterHandle>,
) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return false;
    }
    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        return false;
    };
    let (next, profile) =
        normalize_json_with_descriptor(provider_id, run_id, now_millis(), normalize, &value);
    if !profile.is_empty() {
        shared.profile_entries.lock().await.extend(profile);
    }
    let added_total = next.organizations.len() + next.targets.len();
    if added_total == 0 {
        return false;
    }
    let mut guard = shared.candidates.lock().await;
    let mut delta = OrganizationCandidates::default();
    for item in next.organizations.iter() {
        delta.organizations.push(item.clone());
    }
    for item in next.targets.iter() {
        delta.targets.push(item.clone());
    }
    merge_candidates(&mut *guard, next);
    drop(guard);
    emit_event(
        sink,
        AssetIntelStreamEvent::ProviderBatch {
            run_id: run_id.to_string(),
            provider_id: provider_id.to_string(),
            candidates: delta,
            source: AssetIntelBatchSource::Stdout,
            artifact: None,
            request_id: None,
        },
    );
    true
}

/// Scan `out_dir` for JSON artifacts that have not been emitted yet; for any
/// newly-seen file, normalize its contents and emit a Batch with source =
/// artifact. Mutates `seen` so repeated calls are idempotent.
async fn scan_new_artifacts(
    out_dir: &Path,
    provider_id: &str,
    run_id: &str,
    normalize: &golish_pentest::models::AssetIntelNormalizeConfig,
    seen: &mut HashSet<PathBuf>,
    shared: &CliJsonStreamShared,
    sink: Option<&EventEmitterHandle>,
) -> Result<(), GolishError> {
    let mut files = Vec::new();
    collect_json_files(out_dir, &mut files)?;
    files.sort();
    for path in files {
        if !seen.insert(path.clone()) {
            continue;
        }
        let raw = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) => {
                tracing::debug!(
                    provider = %provider_id,
                    run_id,
                    artifact = %path.display(),
                    error = %err,
                    "asset_intel cli_json artifact read failed (skipping)"
                );
                continue;
            }
        };
        let Some((next, profile)) = normalize_json_document(provider_id, run_id, normalize, &raw)
        else {
            continue;
        };
        if !profile.is_empty() {
            shared.profile_entries.lock().await.extend(profile);
        }
        let added_total = next.organizations.len() + next.targets.len();
        if added_total == 0 {
            continue;
        }
        let mut delta = OrganizationCandidates::default();
        for item in next.organizations.iter() {
            delta.organizations.push(item.clone());
        }
        for item in next.targets.iter() {
            delta.targets.push(item.clone());
        }
        let mut guard = shared.candidates.lock().await;
        merge_candidates(&mut *guard, next);
        drop(guard);
        emit_event(
            sink,
            AssetIntelStreamEvent::ProviderBatch {
                run_id: run_id.to_string(),
                provider_id: provider_id.to_string(),
                candidates: delta,
                source: AssetIntelBatchSource::Artifact,
                artifact: Some(path.display().to_string()),
                request_id: None,
            },
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_cli_json_provider(
    tool: &ToolConfig,
    tools: &[ToolConfig],
    tools_dir: &Path,
    project_root: &Path,
    run_id: &str,
    company_name: &str,
    config: &AssetIntelHydrateConfig,
    sink: Option<&EventEmitterHandle>,
) -> Result<
    (
        AssetIntelProviderRunStatus,
        OrganizationCandidates,
        Value,
        Vec<ProfileFieldEntry>,
    ),
    GolishError,
> {
    let Some(asset) = tool.asset_intel.as_ref() else {
        return Err(GolishError::Validation(format!(
            "tool '{}' has no asset_intel descriptor",
            tool.id
        )));
    };
    let provider_id = provider_id_for_tool(tool).unwrap_or_else(|| tool.id.clone());
    let display_name = if asset.display_name.trim().is_empty() {
        tool.name.clone()
    } else {
        asset.display_name.clone()
    };
    let golish_pentest::models::AssetIntelRuntimeConfig::CliJson {
        skill_id,
        timeout_secs,
        artifact_globs: _,
        arg_bindings,
    } = &asset.runtime
    else {
        return Err(GolishError::Validation(format!(
            "tool '{}' is not a cli_json provider",
            tool.id
        )));
    };

    emit_event(
        sink,
        AssetIntelStreamEvent::ProviderStarted {
            run_id: run_id.to_string(),
            provider_id: provider_id.clone(),
            display_name: display_name.clone(),
            runtime: AssetIntelProviderRuntimeKind::CliJson,
        },
    );

    let Some(exec) = golish_pentest::resolve_tool_executable(&tool.id, tools, tools_dir) else {
        let status = AssetIntelProviderRunStatus {
            provider_id: provider_id.clone(),
            status: AssetIntelProviderRunState::Unavailable,
            message: format!("tool '{}' executable is unavailable", tool.id),
        };
        emit_event(
            sink,
            AssetIntelStreamEvent::ProviderCompleted {
                run_id: run_id.to_string(),
                provider_id: provider_id.clone(),
                status: status.clone(),
                candidate_count: 0,
            },
        );
        return Ok((
            status,
            OrganizationCandidates::default(),
            serde_json::json!({
                "provider": provider_id,
                "runId": run_id,
                "state": "unavailable",
                "reason": "tool_executable_unavailable",
            }),
            Vec::new(),
        ));
    };
    let Some(skill) = tool.skills.iter().find(|skill| skill.id == *skill_id) else {
        let status = AssetIntelProviderRunStatus {
            provider_id: provider_id.clone(),
            status: AssetIntelProviderRunState::Unavailable,
            message: format!("asset intel skill '{skill_id}' is not declared"),
        };
        emit_event(
            sink,
            AssetIntelStreamEvent::ProviderCompleted {
                run_id: run_id.to_string(),
                provider_id: provider_id.clone(),
                status: status.clone(),
                candidate_count: 0,
            },
        );
        return Ok((
            status,
            OrganizationCandidates::default(),
            serde_json::json!({
                "provider": provider_id,
                "runId": run_id,
                "state": "unavailable",
                "reason": "skill_not_found",
                "skillId": skill_id,
            }),
            Vec::new(),
        ));
    };

    let out_dir = asset_intel_provider_output_dir(project_root, run_id, &provider_id);
    fs::create_dir_all(&out_dir)?;
    let rendered_args =
        render_asset_intel_skill_args(&skill.args, company_name, &out_dir, config, arg_bindings);
    let args = split_command_args(&rendered_args);
    let mut command = Command::new(&exec);
    command.args(&args);
    command.current_dir(&out_dir);
    command.kill_on_drop(true);
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());

    let timeout = Duration::from_secs((*timeout_secs).clamp(1, 900));
    tracing::info!(
        provider = %provider_id,
        run_id,
        timeout_secs = timeout.as_secs(),
        out_dir = %out_dir.display(),
        "running asset_intel cli_json provider"
    );

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            tracing::warn!(
                provider = %provider_id,
                run_id,
                error = %err,
                "asset_intel cli_json provider failed to spawn"
            );
            let status = AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::Unavailable,
                message: format!("spawn failed: {err}"),
            };
            emit_event(
                sink,
                AssetIntelStreamEvent::ProviderCompleted {
                    run_id: run_id.to_string(),
                    provider_id: provider_id.clone(),
                    status: status.clone(),
                    candidate_count: 0,
                },
            );
            return Ok((
                status,
                OrganizationCandidates::default(),
                serde_json::json!({
                    "provider": provider_id,
                    "runId": run_id,
                    "state": "unavailable",
                    "reason": "spawn_failed",
                    "error": err.to_string(),
                }),
                Vec::new(),
            ));
        }
    };

    let shared = Arc::new(CliJsonStreamShared::new());
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let normalize = asset.normalize.clone();

    let stdout_handle = stdout.map(|stream| {
        let shared = shared.clone();
        let sink = sink.cloned();
        let normalize = normalize.clone();
        let provider_id = provider_id.clone();
        let run_id = run_id.to_string();
        tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stream).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                {
                    let mut buf = shared.progress_buffer.lock().await;
                    buf.push_str(&line);
                    buf.push('\n');
                }
                let emitted = handle_stdout_line(
                    &line,
                    &provider_id,
                    &run_id,
                    &normalize,
                    &shared,
                    sink.as_ref(),
                )
                .await;
                if !emitted {
                    let msg = truncate_progress_line(&line);
                    if !msg.is_empty() {
                        emit_event(
                            sink.as_ref(),
                            AssetIntelStreamEvent::ProviderProgress {
                                run_id: run_id.clone(),
                                provider_id: provider_id.clone(),
                                message: msg,
                                stream: AssetIntelStreamSource::Stdout,
                            },
                        );
                    }
                }
            }
        })
    });

    let stderr_handle = stderr.map(|stream| {
        let shared = shared.clone();
        let sink = sink.cloned();
        let provider_id = provider_id.clone();
        let run_id = run_id.to_string();
        tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stream).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                {
                    let mut buf = shared.progress_buffer.lock().await;
                    buf.push_str(&line);
                    buf.push('\n');
                }
                let msg = truncate_progress_line(&line);
                if msg.is_empty() {
                    continue;
                }
                emit_event(
                    sink.as_ref(),
                    AssetIntelStreamEvent::ProviderProgress {
                        run_id: run_id.clone(),
                        provider_id: provider_id.clone(),
                        message: msg,
                        stream: AssetIntelStreamSource::Stderr,
                    },
                );
            }
        })
    });

    let watcher_handle = {
        let shared = shared.clone();
        let sink = sink.cloned();
        let normalize = normalize.clone();
        let provider_id = provider_id.clone();
        let run_id = run_id.to_string();
        let out_dir = out_dir.clone();
        tokio::spawn(async move {
            let mut seen: HashSet<PathBuf> = HashSet::new();
            while !shared.cancel.load(Ordering::Acquire) {
                if let Err(err) = scan_new_artifacts(
                    &out_dir,
                    &provider_id,
                    &run_id,
                    &normalize,
                    &mut seen,
                    &shared,
                    sink.as_ref(),
                )
                .await
                {
                    tracing::debug!(
                        provider = %provider_id,
                        run_id,
                        error = %err,
                        "asset_intel cli_json artifact watcher scan failed"
                    );
                }
                tokio::time::sleep(ARTIFACT_POLL_INTERVAL).await;
            }
        })
    };

    let exit_result = tokio::time::timeout(timeout, child.wait()).await;
    shared.cancel.store(true, Ordering::Release);
    if let Some(handle) = stdout_handle {
        let _ = handle.await;
    }
    if let Some(handle) = stderr_handle {
        let _ = handle.await;
    }
    let _ = watcher_handle.await;

    let exit_status = match exit_result {
        Ok(Ok(status)) => status,
        Ok(Err(err)) => {
            tracing::warn!(
                provider = %provider_id,
                run_id,
                error = %err,
                "asset_intel cli_json provider wait failed"
            );
            let _ = child.kill().await;
            let status = AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::Failed,
                message: format!("wait failed: {err}"),
            };
            emit_event(
                sink,
                AssetIntelStreamEvent::ProviderCompleted {
                    run_id: run_id.to_string(),
                    provider_id: provider_id.clone(),
                    status: status.clone(),
                    candidate_count: 0,
                },
            );
            let profile_entries = std::mem::take(&mut *shared.profile_entries.lock().await);
            return Ok((
                status,
                OrganizationCandidates::default(),
                serde_json::json!({
                    "provider": provider_id,
                    "runId": run_id,
                    "state": "failed",
                    "reason": "wait_failed",
                    "error": err.to_string(),
                }),
                profile_entries,
            ));
        }
        Err(_) => {
            tracing::warn!(
                provider = %provider_id,
                run_id,
                timeout_secs = timeout.as_secs(),
                "asset_intel cli_json provider timed out"
            );
            let _ = child.kill().await;
            let candidates = std::mem::take(&mut *shared.candidates.lock().await);
            let profile_entries = std::mem::take(&mut *shared.profile_entries.lock().await);
            let candidate_count = candidates.organizations.len() + candidates.targets.len();
            let status = AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::Failed,
                message: format!("command timed out after {}s", timeout.as_secs()),
            };
            emit_event(
                sink,
                AssetIntelStreamEvent::ProviderCompleted {
                    run_id: run_id.to_string(),
                    provider_id: provider_id.clone(),
                    status: status.clone(),
                    candidate_count,
                },
            );
            return Ok((
                status,
                candidates,
                serde_json::json!({
                    "provider": provider_id,
                    "runId": run_id,
                    "state": "failed",
                    "reason": "timeout",
                    "timeoutSecs": timeout.as_secs(),
                    "candidateCount": candidate_count,
                }),
                profile_entries,
            ));
        }
    };

    let mut final_seen: HashSet<PathBuf> = HashSet::new();
    if let Err(err) = scan_new_artifacts(
        &out_dir,
        &provider_id,
        run_id,
        &normalize,
        &mut final_seen,
        shared.as_ref(),
        sink,
    )
    .await
    {
        tracing::debug!(
            provider = %provider_id,
            run_id,
            error = %err,
            "asset_intel cli_json final artifact scan failed"
        );
    }

    let candidates = std::mem::take(&mut *shared.candidates.lock().await);
    let profile_entries = std::mem::take(&mut *shared.profile_entries.lock().await);
    let progress_buffer = std::mem::take(&mut *shared.progress_buffer.lock().await);
    let preview: String = progress_buffer.chars().take(512).collect();

    if !exit_status.success() {
        tracing::warn!(
            provider = %provider_id,
            run_id,
            exit_code = exit_status.code(),
            "asset_intel cli_json provider exited unsuccessfully"
        );
        let candidate_count = candidates.organizations.len() + candidates.targets.len();
        let status = AssetIntelProviderRunStatus {
            provider_id: provider_id.clone(),
            status: AssetIntelProviderRunState::Failed,
            message: format!("command failed: {preview}"),
        };
        emit_event(
            sink,
            AssetIntelStreamEvent::ProviderCompleted {
                run_id: run_id.to_string(),
                provider_id: provider_id.clone(),
                status: status.clone(),
                candidate_count,
            },
        );
        return Ok((
            status,
            candidates,
            serde_json::json!({
                "provider": provider_id,
                "runId": run_id,
                "state": "failed",
                "reason": "command_failed",
                "exitCode": exit_status.code(),
                "preview": preview,
                "candidateCount": candidate_count,
            }),
            profile_entries,
        ));
    }

    let total = candidates.organizations.len() + candidates.targets.len();
    let state = if total == 0 {
        AssetIntelProviderRunState::CheckedEmpty
    } else {
        AssetIntelProviderRunState::Completed
    };
    tracing::info!(
        provider = %provider_id,
        run_id,
        candidate_count = total,
        state = ?state,
        "asset_intel cli_json provider completed"
    );
    let status = AssetIntelProviderRunStatus {
        provider_id: provider_id.clone(),
        status: state,
        message: if total == 0 {
            format!("{provider_id} completed with no candidates")
        } else {
            format!("{provider_id} normalized {total} candidate(s)")
        },
    };
    emit_event(
        sink,
        AssetIntelStreamEvent::ProviderCompleted {
            run_id: run_id.to_string(),
            provider_id: provider_id.clone(),
            status: status.clone(),
            candidate_count: total,
        },
    );
    Ok((
        status,
        candidates,
        serde_json::json!({
            "provider": provider_id,
            "runId": run_id,
            "state": if total == 0 { "checked_empty" } else { "completed" },
            "candidateCount": total,
            "outDir": out_dir,
        }),
        profile_entries,
    ))
}

fn enscan_record(
    kind: OrganizationCandidateKind,
    label: &str,
    value: &str,
    confidence: f64,
    raw: &Value,
) -> AssetIntelProviderRecord {
    AssetIntelProviderRecord {
        kind,
        label: label.to_string(),
        value: value.to_string(),
        confidence,
        evidence: raw.clone(),
    }
}

fn collect_json_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), GolishError> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(&path, files)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            files.push(path);
        }
    }
    Ok(())
}

fn flatten_candidates(candidates: &OrganizationCandidates) -> Vec<OrganizationCandidate> {
    candidates
        .organizations
        .iter()
        .cloned()
        .chain(candidates.targets.iter().cloned())
        .collect()
}

fn now_millis() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

/// Per-provider output directory used by the lookup runtime, scoped under
/// `<project>/.golish/tool-output/asset-intel-lookup/<runId>/<providerId>`.
/// Keeps lookup artifacts separate from full hydrate runs so cleanup is
/// trivial and there's no risk of mixing canonical vs. discovery output.
fn lookup_provider_output_dir(project_root: &Path, run_id: &str, provider_id: &str) -> PathBuf {
    golish_projects::file_storage::tool_output_dir(project_root, "asset-intel-lookup")
        .join(run_id)
        .join(provider_id)
}

/// Render the lookup skill template. Mirrors `render_asset_intel_skill_args`
/// but without the optional `arg_bindings` for ownership / depth / branches
/// — lookup is intentionally lightweight.
fn render_lookup_skill_args(skill_args: &str, keyword: &str, out_dir: &Path) -> String {
    skill_args
        .replace("{{org}}", keyword)
        .replace("{{keyword}}", keyword)
        .replace("{{company_name}}", keyword)
        .replace("{{out_dir}}", &out_dir.to_string_lossy())
}

/// Run a tool's `asset_intel.lookup` skill in synchronous "wait once, parse,
/// return matches" mode. Used by `asset_intel_lookup_company` to give the
/// UI a fast disambiguation list before a real hydrate.
///
/// Differences vs. `run_cli_json_provider`:
/// - No streaming events (`provider_started` / `provider_progress` / batch).
///   UI shows a single spinner, then a candidate list.
/// - No candidate / profile_fields output; only `LookupCompanyMatch` rows.
/// - Hard timeout from the descriptor, clamped to `[1, 300]` seconds.
async fn run_lookup_cli_provider(
    tool: &ToolConfig,
    tools: &[ToolConfig],
    tools_dir: &Path,
    project_root: &Path,
    run_id: &str,
    keyword: &str,
) -> Result<(AssetIntelProviderRunStatus, Vec<LookupCompanyMatch>), GolishError> {
    let asset = tool.asset_intel.as_ref().ok_or_else(|| {
        GolishError::Validation(format!("tool '{}' has no asset_intel descriptor", tool.id))
    })?;
    let provider_id = provider_id_for_tool(tool).unwrap_or_else(|| tool.id.clone());
    let lookup = match asset.lookup.as_ref() {
        Some(l) if l.enabled => l,
        _ => {
            return Ok((
                AssetIntelProviderRunStatus {
                    provider_id: provider_id.clone(),
                    status: AssetIntelProviderRunState::Unavailable,
                    message: format!("'{provider_id}' does not declare a lookup runtime"),
                },
                Vec::new(),
            ));
        }
    };

    if !matches!(
        asset.runtime,
        golish_pentest::models::AssetIntelRuntimeConfig::CliJson { .. }
    ) {
        return Ok((
            AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::Unavailable,
                message: "lookup is only supported for cli_json providers in this release".into(),
            },
            Vec::new(),
        ));
    }

    let Some(exec) = golish_pentest::resolve_tool_executable(&tool.id, tools, tools_dir) else {
        return Ok((
            AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::Unavailable,
                message: format!("tool '{}' executable is unavailable", tool.id),
            },
            Vec::new(),
        ));
    };
    let Some(skill) = tool.skills.iter().find(|s| s.id == lookup.skill_id) else {
        return Ok((
            AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::Unavailable,
                message: format!("lookup skill '{}' is not declared", lookup.skill_id),
            },
            Vec::new(),
        ));
    };

    let out_dir = lookup_provider_output_dir(project_root, run_id, &provider_id);
    fs::create_dir_all(&out_dir)?;
    let rendered_args = render_lookup_skill_args(&skill.args, keyword, &out_dir);
    let args = split_command_args(&rendered_args);
    let mut command = Command::new(&exec);
    command.args(&args);
    command.current_dir(&out_dir);
    command.kill_on_drop(true);

    let timeout = Duration::from_secs(lookup.timeout_secs.clamp(1, 300));
    tracing::info!(
        provider = %provider_id,
        run_id,
        timeout_secs = timeout.as_secs(),
        keyword,
        "running asset_intel lookup cli provider"
    );

    let output_result = tokio::time::timeout(timeout, command.output()).await;
    let output = match output_result {
        Ok(Ok(output)) => output,
        Ok(Err(err)) => {
            tracing::warn!(provider = %provider_id, error = %err, "lookup spawn failed");
            return Ok((
                AssetIntelProviderRunStatus {
                    provider_id: provider_id.clone(),
                    status: AssetIntelProviderRunState::Unavailable,
                    message: format!("spawn failed: {err}"),
                },
                Vec::new(),
            ));
        }
        Err(_) => {
            tracing::warn!(
                provider = %provider_id,
                timeout_secs = timeout.as_secs(),
                "lookup timed out"
            );
            return Ok((
                AssetIntelProviderRunStatus {
                    provider_id: provider_id.clone(),
                    status: AssetIntelProviderRunState::Failed,
                    message: format!("lookup timed out after {}s", timeout.as_secs()),
                },
                Vec::new(),
            ));
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let preview: String = stdout.chars().take(512).collect();
    let mut matches: Vec<LookupCompanyMatch> = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            matches.extend(extract_lookup_matches(&provider_id, lookup, &value));
        }
    }
    let mut files = Vec::new();
    collect_json_files(&out_dir, &mut files)?;
    files.sort();
    for path in files {
        if let Ok(raw) = fs::read_to_string(&path) {
            if let Ok(value) = serde_json::from_str::<Value>(&raw) {
                matches.extend(extract_lookup_matches(&provider_id, lookup, &value));
            }
        }
    }

    if !output.status.success() {
        return Ok((
            AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::Failed,
                message: format!(
                    "lookup exited with code {:?}: {preview}",
                    output.status.code()
                ),
            },
            matches,
        ));
    }

    if matches.is_empty() {
        return Ok((
            AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::CheckedEmpty,
                message: format!("'{provider_id}' lookup found no matches"),
            },
            matches,
        ));
    }

    Ok((
        AssetIntelProviderRunStatus {
            provider_id: provider_id.clone(),
            status: AssetIntelProviderRunState::Completed,
            message: format!(
                "'{provider_id}' lookup returned {} match(es)",
                matches.len()
            ),
        },
        matches,
    ))
}

/// Dedupe lookup matches across providers. Key is `lower(credit_code)` when
/// present (most reliable), else `lower(trim(name))`. Keeps the first hit's
/// confidence + evidence; subsequent duplicates are silently dropped.
fn dedupe_lookup_matches(input: Vec<LookupCompanyMatch>) -> Vec<LookupCompanyMatch> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for m in input {
        let key = m
            .credit_code
            .as_deref()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| m.name.trim().to_lowercase());
        if seen.insert(key) {
            out.push(m);
        }
    }
    out
}

#[tauri::command]
pub async fn asset_intel_list_providers(
    pentest: tauri::State<'_, PentestState>,
) -> Result<Vec<AssetIntelProviderDescriptor>, GolishError> {
    let pentest_config = pentest.config_manager.get().await;
    let scan = golish_pentest::scan_toolsconfig(&pentest_config.toolsconfig_dir);
    if !scan.success {
        return Err(GolishError::Internal(
            scan.error
                .unwrap_or_else(|| "toolsconfig scan failed".into()),
        ));
    }
    Ok(provider_descriptors_from_tools(&scan.tools))
}

/// Hard cap so frontend lookup modals stay scannable. Per-provider lookups
/// can exceed this individually; we trim after dedupe.
const LOOKUP_RESULTS_HARD_CAP: usize = 25;

#[tauri::command]
pub async fn asset_intel_lookup_company(
    state: tauri::State<'_, DbState>,
    pentest: tauri::State<'_, PentestState>,
    args: AssetIntelLookupRequest,
) -> Result<AssetIntelLookupResult, GolishError> {
    let _ = state.pool_ready().await?;
    if args.keyword.trim().is_empty() {
        return Err(GolishError::Validation(
            "keyword is required for asset intel lookup".into(),
        ));
    }

    let pentest_config = pentest.config_manager.get().await;
    let scan = golish_pentest::scan_toolsconfig(&pentest_config.toolsconfig_dir);
    if !scan.success {
        return Err(GolishError::Internal(
            scan.error
                .unwrap_or_else(|| "toolsconfig scan failed".into()),
        ));
    }

    // Select providers: explicit ids if given (must exist + have lookup),
    // otherwise every tool with a lookup descriptor regardless of `auto`.
    // Lookup is meant for "I want to disambiguate" so we don't apply the
    // auto.priority filter — the user has already opted in by clicking
    // "Look up company".
    let selected: Vec<&ToolConfig> = if args.provider_ids.is_empty() {
        scan.tools
            .iter()
            .filter(|t| {
                t.asset_intel
                    .as_ref()
                    .and_then(|a| a.lookup.as_ref())
                    .is_some_and(|l| l.enabled)
            })
            .collect()
    } else {
        let mut out = Vec::new();
        for provider_id in &args.provider_ids {
            let Some(tool) = scan
                .tools
                .iter()
                .find(|t| provider_id_for_tool(t).as_deref() == Some(provider_id.as_str()))
            else {
                return Err(GolishError::NotFound(format!(
                    "asset intel provider '{provider_id}' is not registered"
                )));
            };
            out.push(tool);
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
    for tool in selected {
        let (status, matches) = run_lookup_cli_provider(
            tool,
            &scan.tools,
            &pentest_config.tools_dir,
            &project_root,
            &run_id,
            args.keyword.trim(),
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
    let limit = args
        .limit
        .unwrap_or(LOOKUP_RESULTS_HARD_CAP)
        .min(LOOKUP_RESULTS_HARD_CAP);
    deduped.truncate(limit);

    Ok(AssetIntelLookupResult {
        run_id,
        matches: deduped,
        provider_status,
    })
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
    pentest: tauri::State<'_, PentestState>,
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

    let pentest_config = pentest.config_manager.get().await;
    let scan = golish_pentest::scan_toolsconfig(&pentest_config.toolsconfig_dir);
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
    pentest: tauri::State<'_, PentestState>,
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

    let pentest_config = pentest.config_manager.get().await;
    let scan = golish_pentest::scan_toolsconfig(&pentest_config.toolsconfig_dir);
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetIntelEnrichOrganizationArgs {
    pub organization_id: String,
    #[serde(default)]
    pub provider_ids: Vec<String>,
    #[serde(default)]
    pub config: AssetIntelHydrateConfig,
}

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
    pentest: tauri::State<'_, PentestState>,
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

    let pentest_config = pentest.config_manager.get().await;
    let scan = golish_pentest::scan_toolsconfig(&pentest_config.toolsconfig_dir);
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
        &args.config,
    )
    .await
}

/// Args for [`asset_intel_enrich_batch`].
///
/// `include_self` defaults to `true` so a single click on the master org's
/// "批量补字段" button enriches the master + every promoted child.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetIntelEnrichBatchArgs {
    pub parent_organization_id: String,
    #[serde(default)]
    pub include_self: Option<bool>,
    #[serde(default)]
    pub provider_ids: Vec<String>,
    #[serde(default)]
    pub config: AssetIntelHydrateConfig,
}

/// One entry in [`AssetIntelEnrichBatchResult::skipped`]; carries the org id
/// we declined to enrich and a short machine-readable reason
/// (`empty_name` / `no_children` / `provider_error`). Frontend can render a
/// localized message via `mapErr` instead of dumping the raw string.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetIntelEnrichBatchSkip {
    pub organization_id: String,
    pub reason: String,
}

/// Result of [`asset_intel_enrich_batch`].
///
/// `runs` is sequenced in execution order (parent first when
/// `include_self=true`, then children in `parent_id, sort_order, name`
/// order — same as `organizations::list`). `skipped` captures empty-name /
/// missing-provider cases so the UI can present a complete activity log.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetIntelEnrichBatchResult {
    pub runs: Vec<AssetIntelRun>,
    pub skipped: Vec<AssetIntelEnrichBatchSkip>,
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
    pentest: tauri::State<'_, PentestState>,
    args: AssetIntelEnrichBatchArgs,
) -> Result<AssetIntelEnrichBatchResult, GolishError> {
    let pool = state.pool_ready().await?;
    let parent_id: Uuid = args.parent_organization_id.parse()?;
    let parent_row = golish_db::repo::organizations::get_one(pool, parent_id)
        .await?
        .ok_or_else(|| {
            GolishError::NotFound(format!("organization {}", args.parent_organization_id))
        })?;

    let pentest_config = pentest.config_manager.get().await;
    let scan = golish_pentest::scan_toolsconfig(&pentest_config.toolsconfig_dir);
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
async fn run_providers_for_org(
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
    for tool in &providers {
        let asset = tool.asset_intel.as_ref().ok_or_else(|| {
            GolishError::Validation(format!("tool '{}' has no asset_intel descriptor", tool.id))
        })?;
        let (status, next_candidates, next_evidence, next_profile) = match &asset.runtime {
            golish_pentest::models::AssetIntelRuntimeConfig::CliJson { .. } => {
                run_cli_json_provider(
                    tool,
                    scan_tools,
                    &pentest_config.tools_dir,
                    &project_root,
                    &run_id,
                    company_name,
                    config,
                    sink,
                )
                .await?
            }
            golish_pentest::models::AssetIntelRuntimeConfig::HttpJson { .. } => {
                run_http_json_provider(pool, tool, &run_id, company_name, config, sink).await?
            }
        };
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

fn merge_string_vec(existing: &[String], incoming: &mut Vec<String>) {
    let mut out = existing.to_vec();
    for item in std::mem::take(incoming) {
        let key = item.trim().to_lowercase();
        if key.is_empty() || out.iter().any(|value| value.trim().to_lowercase() == key) {
            continue;
        }
        out.push(item);
    }
    *incoming = out;
}

fn merge_json_array(existing: &Value, incoming: &mut Option<Value>) {
    let Some(Value::Array(next)) = incoming else {
        return;
    };
    let mut out = existing.as_array().cloned().unwrap_or_default();
    for item in std::mem::take(next) {
        let key = display_json_atom(&item).trim().to_lowercase();
        if key.is_empty()
            || out
                .iter()
                .any(|value| display_json_atom(value).trim().to_lowercase() == key)
        {
            continue;
        }
        out.push(item);
    }
    *incoming = Some(Value::Array(out));
}

fn merge_contacts_object(existing: &Value, incoming: &mut Option<Value>) {
    let Some(Value::Object(next)) = incoming else {
        return;
    };
    let mut out = existing.as_object().cloned().unwrap_or_default();
    for (channel, value) in std::mem::take(next) {
        let target = out
            .entry(channel)
            .or_insert_with(|| Value::Array(Vec::new()));
        if !target.is_array() {
            *target = Value::Array(Vec::new());
        }
        let Some(target_array) = target.as_array_mut() else {
            continue;
        };
        for item in value.as_array().cloned().unwrap_or_else(|| vec![value]) {
            let key = display_json_atom(&item).trim().to_lowercase();
            if key.is_empty()
                || target_array
                    .iter()
                    .any(|existing| display_json_atom(existing).trim().to_lowercase() == key)
            {
                continue;
            }
            target_array.push(item);
        }
    }
    *incoming = Some(Value::Object(out));
}

fn display_json_atom(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Object(map) => map
            .get("domain")
            .or_else(|| map.get("name"))
            .or_else(|| map.get("value"))
            .or_else(|| map.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        other => other.to_string(),
    }
}

fn merge_profile_patch_with_existing(
    org: &golish_db::models::Organization,
    patch: &mut golish_db::repo::organizations::ProfilePatch,
) {
    if let Some(aliases) = patch.aliases.as_mut() {
        merge_string_vec(&org.aliases, aliases);
    }
    merge_json_array(&org.domains, &mut patch.domains);
    merge_json_array(&org.ip_ranges, &mut patch.ip_ranges);
    merge_json_array(&org.asns, &mut patch.asns);
    merge_json_array(&org.email_domains, &mut patch.email_domains);
    merge_json_array(&org.scope_rules, &mut patch.scope_rules);
    merge_json_array(&org.certificates, &mut patch.certificates);
    merge_json_array(&org.subsidiaries, &mut patch.subsidiaries);
    merge_json_array(&org.business_systems, &mut patch.business_systems);
    merge_json_array(&org.cloud_assets, &mut patch.cloud_assets);
    merge_json_array(&org.github_orgs, &mut patch.github_orgs);
    merge_json_array(&org.social_accounts, &mut patch.social_accounts);
    merge_json_array(&org.historical_vulns, &mut patch.historical_vulns);
    match patch.contacts {
        Some(Value::Array(_)) => merge_json_array(&org.contacts, &mut patch.contacts),
        Some(Value::Object(_)) => merge_contacts_object(&org.contacts, &mut patch.contacts),
        _ => {}
    }
}

/// Fold a flat list of `ProfileFieldEntry` into a single
/// `ProfilePatch`, layered on top of the organization's existing `intel`
/// JSON.
///
/// Returns `Ok(None)` when there's nothing meaningful to write (no scalar
/// entries, no intel mutations, no contact additions) — avoiding a noisy
/// `update_profile` roundtrip on every hydrate run.
///
/// Conflict policy when multiple providers (or multiple paths in one
/// descriptor) supply the same key:
/// - Scalar: keep the **first non-empty** value seen (later providers don't
///   silently overwrite).
/// - Intel key: same — first wins (use a more specific descriptor to break
///   ties at config time).
/// - Contact channel: append unique values; duplicates from raw input are
///   dropped via lowercase-trim compare.
fn build_profile_patch_from_entries(
    existing_intel: &Value,
    entries: &[ProfileFieldEntry],
) -> Result<Option<golish_db::repo::organizations::ProfilePatch>, GolishError> {
    use golish_pentest::models::AssetIntelProfileFieldTarget as Target;

    let mut scalars: HashMap<String, String> = HashMap::new();
    let mut aliases: Vec<String> = Vec::new();
    let mut json_array_fields: HashMap<String, Vec<String>> = HashMap::new();
    let mut intel_overrides: HashMap<String, Value> = HashMap::new();
    let mut intel_array_fields: HashMap<String, Vec<String>> = HashMap::new();
    let mut contact_additions: HashMap<String, Vec<String>> = HashMap::new();

    fn push_unique(values: &mut Vec<String>, value: String) {
        let key = value.trim().to_lowercase();
        if !values.iter().any(|item| item.trim().to_lowercase() == key) {
            values.push(value);
        }
    }

    fn is_json_array_profile_field(field: &str) -> bool {
        matches!(
            field,
            "domains"
                | "ip_ranges"
                | "asns"
                | "email_domains"
                | "scope_rules"
                | "certificates"
                | "subsidiaries"
                | "business_systems"
                | "cloud_assets"
                | "github_orgs"
                | "social_accounts"
                | "historical_vulns"
                | "contacts"
        )
    }

    fn is_intel_array_profile_field(field: &str) -> bool {
        matches!(
            field,
            "icp_records"
                | "mobile_apps"
                | "mini_programs"
                | "app_domains"
                | "exposed_emails"
                | "code_leaks"
                | "mail_mx"
        )
    }

    for entry in entries {
        let value = entry.value.trim().to_string();
        if value.is_empty() {
            continue;
        }
        match entry.target_kind {
            Target::Scalar => {
                if entry.target_field == "aliases" {
                    push_unique(&mut aliases, value);
                } else if is_json_array_profile_field(&entry.target_field) {
                    push_unique(
                        json_array_fields
                            .entry(entry.target_field.clone())
                            .or_default(),
                        value,
                    );
                } else {
                    scalars.entry(entry.target_field.clone()).or_insert(value);
                }
            }
            Target::Intel => {
                if is_intel_array_profile_field(&entry.target_field) {
                    push_unique(
                        intel_array_fields
                            .entry(entry.target_field.clone())
                            .or_default(),
                        value,
                    );
                } else {
                    intel_overrides
                        .entry(entry.target_field.clone())
                        .or_insert_with(|| Value::String(value));
                }
            }
            Target::Contact => {
                let bucket = contact_additions
                    .entry(entry.target_field.clone())
                    .or_default();
                let lower = value.to_lowercase();
                if !bucket.iter().any(|item| item.to_lowercase() == lower) {
                    bucket.push(value);
                }
            }
        }
    }

    let mut patch = golish_db::repo::organizations::ProfilePatch::default();
    let mut touched = false;

    if !aliases.is_empty() {
        patch.aliases = Some(aliases);
        touched = true;
    }

    for (field, value) in &scalars {
        match field.as_str() {
            "industry" => {
                patch.industry = Some(value.clone());
                touched = true;
            }
            "credit_code" => {
                patch.credit_code = Some(value.clone());
                touched = true;
            }
            "notes" => {
                patch.notes = Some(value.clone());
                touched = true;
            }
            // tier is technically scalar but constrained to enum; let users
            // promote tier manually rather than via auto-hydrate
            other => {
                tracing::debug!(
                    field = other,
                    value,
                    "asset_intel profile scalar field is not wired to ProfilePatch — ignoring"
                );
            }
        }
    }

    for (field, values) in json_array_fields {
        if values.is_empty() {
            continue;
        }
        let json = Some(Value::Array(
            values.into_iter().map(Value::String).collect(),
        ));
        match field.as_str() {
            "domains" => patch.domains = json,
            "ip_ranges" => patch.ip_ranges = json,
            "asns" => patch.asns = json,
            "email_domains" => patch.email_domains = json,
            "scope_rules" => patch.scope_rules = json,
            "certificates" => patch.certificates = json,
            "subsidiaries" => patch.subsidiaries = json,
            "business_systems" => patch.business_systems = json,
            "cloud_assets" => patch.cloud_assets = json,
            "github_orgs" => patch.github_orgs = json,
            "social_accounts" => patch.social_accounts = json,
            "historical_vulns" => patch.historical_vulns = json,
            "contacts" => patch.contacts = json,
            _ => continue,
        }
        touched = true;
    }

    let mut intel_value = if existing_intel.is_object() {
        existing_intel.clone()
    } else {
        Value::Object(serde_json::Map::new())
    };
    let intel_object = intel_value
        .as_object_mut()
        .expect("intel_value initialized as object above");

    let mut intel_touched = false;
    for (key, value) in intel_overrides {
        intel_object.entry(key).or_insert(value);
        intel_touched = true;
    }

    for (key, values) in intel_array_fields {
        if values.is_empty() {
            continue;
        }
        let entry = intel_object
            .entry(key)
            .or_insert_with(|| Value::Array(Vec::new()));
        if !entry.is_array() {
            *entry = Value::Array(vec![entry.clone()]);
        }
        let existing = entry
            .as_array_mut()
            .expect("intel array field initialized above");
        let mut seen: HashSet<String> = existing
            .iter()
            .map(display_json_atom)
            .map(|item| item.trim().to_lowercase())
            .filter(|item| !item.is_empty())
            .collect();
        for value in values {
            let key = value.trim().to_lowercase();
            if key.is_empty() || !seen.insert(key) {
                continue;
            }
            existing.push(Value::String(value));
        }
        intel_touched = true;
    }

    if !contact_additions.is_empty() {
        let contacts_entry = intel_object
            .entry("contacts")
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if !contacts_entry.is_object() {
            *contacts_entry = Value::Object(serde_json::Map::new());
        }
        let contacts_map = contacts_entry
            .as_object_mut()
            .expect("contacts initialized as object above");
        for (channel, mut values) in contact_additions {
            let existing_list = match contacts_map.entry(channel.clone()) {
                serde_json::map::Entry::Occupied(o) => o.into_mut(),
                serde_json::map::Entry::Vacant(v) => v.insert(Value::Array(Vec::new())),
            };
            if !existing_list.is_array() {
                *existing_list = Value::Array(Vec::new());
            }
            let list = existing_list
                .as_array_mut()
                .expect("contacts channel initialized as array above");
            let already: HashSet<String> = list
                .iter()
                .filter_map(|item| item.as_str().map(|s| s.trim().to_lowercase()))
                .collect();
            values.retain(|item| !already.contains(&item.trim().to_lowercase()));
            for value in values {
                list.push(Value::String(value));
            }
        }
        patch.contacts = Some(contacts_entry.clone());
        intel_touched = true;
    }

    if intel_touched {
        patch.intel = Some(intel_value);
        touched = true;
    }

    if touched {
        Ok(Some(patch))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_runtime() -> golish_pentest::models::AssetIntelRuntimeConfig {
        golish_pentest::models::AssetIntelRuntimeConfig::CliJson {
            skill_id: "company-default-json".into(),
            timeout_secs: 30,
            artifact_globs: vec![],
            arg_bindings: std::collections::HashMap::new(),
        }
    }

    fn fake_normalize_config() -> golish_pentest::models::AssetIntelNormalizeConfig {
        golish_pentest::models::AssetIntelNormalizeConfig {
            organization: vec![golish_pentest::models::AssetIntelNormalizeRule {
                path: "$..invest[*]".into(),
                label: golish_pentest::models::AssetIntelFieldRef::Field("name".into()),
                value: golish_pentest::models::AssetIntelFieldRef::Field("name".into()),
                confidence: 0.82,
                when: vec![],
            }],
            target: vec![golish_pentest::models::AssetIntelNormalizeRule {
                path: "$..icp[*]".into(),
                label: golish_pentest::models::AssetIntelFieldRef::Field("domain".into()),
                value: golish_pentest::models::AssetIntelFieldRef::FirstOf(vec![
                    "domain".into(),
                    "url".into(),
                ]),
                confidence: 0.78,
                when: vec![],
            }],
            profile_fields: vec![],
        }
    }

    fn org_candidate_with_raw(name: &str, scale: &str, status: &str) -> OrganizationCandidate {
        OrganizationCandidate {
            id: format!("org:enscan-go:{name}"),
            kind: OrganizationCandidateKind::Organization,
            label: name.into(),
            value: name.into(),
            source: "enscan-go".into(),
            confidence: 0.82,
            status: "needs_review".into(),
            evidence: serde_json::json!({
                "provider": "enscan-go",
                "runId": "run-test",
                "raw": {
                    "name": name,
                    "scale": scale,
                    "status": status,
                    "pid": format!("pid-{name}")
                }
            }),
            created_at: 1,
        }
    }

    fn auto_promote_policy() -> golish_pentest::models::AssetIntelDiscoveryConfig {
        use golish_pentest::models::{AssetIntelNormalizeFilter, AssetIntelNormalizeFilterOp};
        golish_pentest::models::AssetIntelDiscoveryConfig {
            auto_promote: true,
            promote_when: vec![
                AssetIntelNormalizeFilter {
                    field: "scale".into(),
                    op: AssetIntelNormalizeFilterOp::Gte,
                    value: "51".into(),
                },
                AssetIntelNormalizeFilter {
                    field: "status".into(),
                    op: AssetIntelNormalizeFilterOp::Contains,
                    value: "开业".into(),
                },
            ],
            ownership_field: "scale".into(),
            dedupe_by: vec!["pid".into(), "name".into()],
        }
    }

    #[test]
    fn provider_output_is_trusted_only_for_successful_terminal_states() {
        assert!(provider_output_is_trusted(&AssetIntelProviderRunStatus {
            provider_id: "enscan-go".into(),
            status: AssetIntelProviderRunState::Completed,
            message: "ok".into(),
        }));
        assert!(provider_output_is_trusted(&AssetIntelProviderRunStatus {
            provider_id: "enscan-go".into(),
            status: AssetIntelProviderRunState::CheckedEmpty,
            message: "empty".into(),
        }));
        assert!(!provider_output_is_trusted(&AssetIntelProviderRunStatus {
            provider_id: "enscan-go-tyc-discovery".into(),
            status: AssetIntelProviderRunState::Failed,
            message: "command failed after emitting partial stdout".into(),
        }));
        assert!(!provider_output_is_trusted(&AssetIntelProviderRunStatus {
            provider_id: "enscan-go-kc-discovery".into(),
            status: AssetIntelProviderRunState::Unavailable,
            message: "missing credentials".into(),
        }));
    }

    #[test]
    fn asset_intel_provider_descriptors_load_from_tool_configs() {
        let tool = golish_pentest::models::ToolConfig {
            id: "fake-intel".into(),
            name: "Fake Intel".into(),
            asset_intel: Some(golish_pentest::models::AssetIntelToolConfig {
                enabled: true,
                provider_id: "fake-provider".into(),
                display_name: "Fake Provider".into(),
                capabilities: vec!["domains".into(), "apps".into()],
                requires_integration: Some(
                    golish_pentest::models::AssetIntelIntegrationRequirement {
                        tool_id: "fake-intel".into(),
                        group_ids: vec!["default".into()],
                    },
                ),
                auto: golish_pentest::models::AssetIntelAutoConfig {
                    default: true,
                    priority: 10,
                },
                runtime: fake_runtime(),
                normalize: golish_pentest::models::AssetIntelNormalizeConfig::default(),
                discovery: golish_pentest::models::AssetIntelDiscoveryConfig::default(),
                lookup: None,
            }),
            ..Default::default()
        };

        let providers = provider_descriptors_from_tools(&[tool]);

        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].id, "fake-provider");
        assert_eq!(providers[0].display_name, "Fake Provider");
        assert_eq!(
            providers[0].requires_integration,
            Some(AssetIntelIntegrationRequirement {
                tool_id: "fake-intel".into(),
                group_ids: vec!["default".into()],
            })
        );
        assert!(providers[0]
            .capabilities
            .contains(&AssetIntelCapability::Domains));
        assert!(providers[0]
            .capabilities
            .contains(&AssetIntelCapability::Apps));
    }

    #[test]
    fn normalize_provider_records_splits_candidates_and_preserves_evidence() {
        let candidates = normalize_provider_records(
            "mock",
            "run-1",
            123,
            vec![
                AssetIntelProviderRecord {
                    kind: OrganizationCandidateKind::Organization,
                    label: "Acme Subsidiary".into(),
                    value: "Acme Subsidiary".into(),
                    confidence: 0.86,
                    evidence: serde_json::json!({"raw": {"ownership": 51}}),
                },
                AssetIntelProviderRecord {
                    kind: OrganizationCandidateKind::Target,
                    label: "api.acme.test".into(),
                    value: "api.acme.test".into(),
                    confidence: 0.72,
                    evidence: serde_json::json!({"raw": {"type": "domain"}}),
                },
            ],
        );

        assert_eq!(candidates.organizations.len(), 1);
        assert_eq!(candidates.targets.len(), 1);
        assert_eq!(candidates.organizations[0].source, "mock");
        assert_eq!(candidates.organizations[0].status, "needs_review");
        assert_eq!(candidates.organizations[0].created_at, 123);
        assert_eq!(candidates.organizations[0].evidence["provider"], "mock");
        assert_eq!(candidates.organizations[0].evidence["runId"], "run-1");
        assert_eq!(candidates.targets[0].id, "target:mock:api.acme.test");
    }

    #[test]
    fn auto_promote_child_decisions_only_promote_active_controlled_investments() {
        let candidates = OrganizationCandidates {
            organizations: vec![
                org_candidate_with_raw("平安信托有限责任公司", "99.880923%", "开业"),
                org_candidate_with_raw("平安证券股份有限公司", "40.9596%", "开业"),
                org_candidate_with_raw("注销分支", "100%", "注销"),
                org_candidate_with_raw("已存在子公司", "100%", "开业"),
            ],
            targets: vec![],
        };
        let existing = HashSet::from(["已存在子公司".to_string()]);
        let policy = auto_promote_policy();

        let decisions = auto_promote_child_decisions(&candidates, &policy, &existing);

        assert_eq!(decisions.iter().filter(|item| item.promote).count(), 1);
        assert_eq!(decisions[0].candidate.value, "平安信托有限责任公司");
        assert_eq!(decisions[0].ownership_percent, Some(99.880923));
        assert_eq!(
            decisions
                .iter()
                .filter_map(|item| item.reason.as_ref())
                .collect::<Vec<_>>(),
            vec![
                &AutoPromoteSkipReason::OwnershipBelowThreshold,
                &AutoPromoteSkipReason::InactiveStatus,
                &AutoPromoteSkipReason::Duplicate,
            ]
        );
    }

    #[test]
    fn clear_engagement_candidates_preserves_engagement_metadata() {
        let intel = serde_json::json!({
            "engagement": {
                "mode": "discover_assets",
                "lookup_match": { "name": "中国平安保险（集团）股份有限公司" },
                "candidates": {
                    "organizations": [{ "id": "org:enscan-go:old", "value": "old" }],
                    "targets": [{ "id": "target:enscan-go:old", "value": "old.example" }]
                }
            },
            "contacts": {
                "email": ["ir@example.test"]
            }
        });

        let cleared = clear_engagement_candidates_from_intel(intel).unwrap();

        assert_eq!(cleared["engagement"]["mode"], "discover_assets");
        assert_eq!(
            cleared["engagement"]["lookup_match"]["name"],
            "中国平安保险（集团）股份有限公司"
        );
        assert!(cleared["engagement"].get("candidates").is_none());
        assert_eq!(cleared["contacts"]["email"][0], "ir@example.test");
    }

    #[test]
    fn json_descriptor_normalizer_maps_nested_candidate_buckets() {
        let normalize = fake_normalize_config();
        let raw = serde_json::json!({
            "payload": {
                "invest": [{ "name": "小米科技有限责任公司" }],
                "icp": [{ "domain": "mi.com" }]
            }
        });

        let (candidates, profile) =
            normalize_json_with_descriptor("fake", "run-1", 123, &normalize, &raw);

        assert_eq!(candidates.organizations.len(), 1);
        assert_eq!(candidates.organizations[0].label, "小米科技有限责任公司");
        assert_eq!(candidates.organizations[0].source, "fake");
        assert_eq!(candidates.targets.len(), 1);
        assert_eq!(candidates.targets[0].value, "mi.com");
        assert_eq!(candidates.targets[0].confidence, 0.78);
        assert_eq!(candidates.targets[0].evidence["provider"], "fake");
        assert!(profile.is_empty(), "no profile_fields rules in fake config");
    }

    #[test]
    fn fake_provider_json_data_dedupes_across_sources() {
        let normalize = fake_normalize_config();
        let first_raw = serde_json::json!({
            "payload": {
                "invest": [{ "name": "小米科技有限责任公司" }],
                "icp": [{ "domain": "mi.com" }, { "domain": "api.mi.com" }]
            }
        });
        let second_raw = serde_json::json!({
            "data": {
                "invest": [{ "name": "小米科技有限责任公司" }],
                "icp": [{ "domain": "MI.COM" }, { "domain": "store.mi.com" }]
            }
        });

        let (mut merged, _) =
            normalize_json_with_descriptor("fake-cli", "run-1", 1, &normalize, &first_raw);
        let (http_candidates, _) =
            normalize_json_with_descriptor("fake-http", "run-1", 2, &normalize, &second_raw);
        merge_candidates(&mut merged, http_candidates);

        assert_eq!(merged.organizations.len(), 1);
        assert_eq!(merged.organizations[0].source, "fake-cli");
        assert_eq!(
            merged
                .targets
                .iter()
                .map(|item| item.value.as_str())
                .collect::<Vec<_>>(),
            vec!["mi.com", "api.mi.com", "store.mi.com"]
        );
    }

    #[tokio::test]
    async fn http_json_runtime_posts_fake_data_and_normalizes_candidates() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let mut bytes = Vec::new();
            loop {
                let n = stream.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buf[..n]);
                let req = String::from_utf8_lossy(&bytes);
                if req.contains("query_type=domain") {
                    break;
                }
            }
            let req = String::from_utf8_lossy(&bytes);
            assert!(req.starts_with("POST / HTTP/1.1"));
            assert!(req.contains("query=%E5%B0%8F%E7%B1%B3"));
            assert!(req.contains("query_type=domain"));

            let body = serde_json::json!({
                "code": 0,
                "data": [
                    { "domain": "mi.com", "title": "Xiaomi" },
                    { "domain": "api.mi.com", "title": "Xiaomi API" }
                ],
                "message": "ok"
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let mut form = std::collections::HashMap::new();
        form.insert("query".to_string(), "{{company_name}}".to_string());
        form.insert("query_type".to_string(), "domain".to_string());
        let tool = ToolConfig {
            id: "fake-http".into(),
            name: "Fake HTTP".into(),
            asset_intel: Some(golish_pentest::models::AssetIntelToolConfig {
                enabled: true,
                provider_id: "fake-http".into(),
                display_name: "Fake HTTP".into(),
                capabilities: vec!["domains".into()],
                requires_integration: None,
                auto: golish_pentest::models::AssetIntelAutoConfig {
                    default: true,
                    priority: 1,
                },
                runtime: golish_pentest::models::AssetIntelRuntimeConfig::HttpJson {
                    requests: vec![golish_pentest::models::AssetIntelHttpRequest {
                        id: "domains".into(),
                        method: "POST".into(),
                        url,
                        headers: std::collections::HashMap::new(),
                        form,
                        json: Value::Null,
                        timeout_secs: 5,
                    }],
                },
                normalize: golish_pentest::models::AssetIntelNormalizeConfig {
                    organization: vec![],
                    target: vec![golish_pentest::models::AssetIntelNormalizeRule {
                        path: "$..data[*]".into(),
                        label: golish_pentest::models::AssetIntelFieldRef::Field("title".into()),
                        value: golish_pentest::models::AssetIntelFieldRef::Field("domain".into()),
                        confidence: 0.72,
                        when: vec![],
                    }],
                    profile_fields: vec![],
                },
                discovery: golish_pentest::models::AssetIntelDiscoveryConfig::default(),
                lookup: None,
            }),
            ..Default::default()
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://golish:golish@127.0.0.1:1/golish")
            .unwrap();

        let (status, candidates, evidence, _profile) = run_http_json_provider(
            &pool,
            &tool,
            "run-1",
            "小米",
            &AssetIntelHydrateConfig::default(),
            None,
        )
        .await
        .unwrap();
        server.await.unwrap();

        assert_eq!(status.status, AssetIntelProviderRunState::Completed);
        assert_eq!(candidates.targets.len(), 2);
        assert_eq!(candidates.targets[0].label, "Xiaomi");
        assert_eq!(candidates.targets[0].value, "mi.com");
        assert_eq!(evidence["candidateCount"], 2);
    }

    #[derive(Debug, Default, Clone)]
    struct RecordingEmitter {
        events: std::sync::Arc<std::sync::Mutex<Vec<(String, Value)>>>,
    }

    impl golish_core::EventEmitter for RecordingEmitter {
        fn emit_json(&self, event: &str, payload: Value) {
            self.events
                .lock()
                .unwrap()
                .push((event.to_string(), payload));
        }
    }

    impl RecordingEmitter {
        fn snapshot(&self) -> Vec<(String, Value)> {
            self.events.lock().unwrap().clone()
        }

        fn handle(&self) -> EventEmitterHandle {
            EventEmitterHandle::new(self.clone())
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cli_json_runtime_streams_progress_and_artifact_batches() {
        use std::os::unix::fs::PermissionsExt;

        let tools_dir = tempfile::tempdir().unwrap();
        let project_root = tempfile::tempdir().unwrap();
        let executable = tools_dir.path().join("fake-asset-intel.sh");
        // Fake CLI:
        //   1) emit a progress line on stdout (non-JSON → progress event)
        //   2) write icp.json → artifact watcher should observe it
        //   3) sleep > ARTIFACT_POLL_INTERVAL so the watcher polls
        //   4) write app.json → another artifact batch
        //   5) emit another progress line + exit 0
        fs::write(
            &executable,
            r#"#!/bin/sh
echo "[stage] collecting icp"
printf '%s' '{"payload":{"icp":[{"domain":"a.example"}]}}' > "$(pwd)/icp.json"
sleep 0.8
echo "[stage] collecting app"
printf '%s' '{"payload":{"icp":[{"domain":"b.example"}]}}' > "$(pwd)/app.json"
sleep 0.8
echo "[stage] done"
"#,
        )
        .unwrap();
        let mut perms = fs::metadata(&executable).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&executable, perms).unwrap();

        let tool = ToolConfig {
            id: "fake-stream".into(),
            name: "Fake Stream".into(),
            executable: "fake-asset-intel.sh".into(),
            runtime: "native".into(),
            skills: vec![golish_pentest::models::ToolSkill {
                id: "company-default-json".into(),
                name: "Company JSON".into(),
                description: String::new(),
                args: String::new(),
                tags: vec![],
            }],
            asset_intel: Some(golish_pentest::models::AssetIntelToolConfig {
                enabled: true,
                provider_id: "fake-stream".into(),
                display_name: "Fake Stream".into(),
                capabilities: vec!["domains".into()],
                requires_integration: None,
                auto: golish_pentest::models::AssetIntelAutoConfig {
                    default: true,
                    priority: 1,
                },
                runtime: fake_runtime(),
                normalize: fake_normalize_config(),
                discovery: golish_pentest::models::AssetIntelDiscoveryConfig::default(),
                lookup: None,
            }),
            ..Default::default()
        };

        let recorder = RecordingEmitter::default();
        let handle = recorder.handle();
        let (status, candidates, _evidence, _profile) = run_cli_json_provider(
            &tool,
            std::slice::from_ref(&tool),
            tools_dir.path(),
            project_root.path(),
            "run-stream",
            "Acme",
            &AssetIntelHydrateConfig::default(),
            Some(&handle),
        )
        .await
        .unwrap();

        assert_eq!(status.status, AssetIntelProviderRunState::Completed);
        // dedup of a.example + b.example
        assert_eq!(candidates.targets.len(), 2);

        let events = recorder.snapshot();
        let names: Vec<&str> = events
            .iter()
            .filter_map(|(name, payload)| {
                if name == ASSET_INTEL_EVENT {
                    payload.get("kind").and_then(|v| v.as_str())
                } else {
                    None
                }
            })
            .collect();

        assert!(
            names.iter().any(|name| *name == "provider_started"),
            "expected provider_started in {:?}",
            names
        );
        assert!(
            names
                .iter()
                .filter(|name| **name == "provider_progress")
                .count()
                >= 2,
            "expected at least 2 progress events (saw {:?})",
            names
        );
        let batch_events: Vec<&(String, Value)> = events
            .iter()
            .filter(|(_, payload)| {
                payload.get("kind").and_then(|v| v.as_str()) == Some("provider_batch")
            })
            .collect();
        assert!(
            !batch_events.is_empty(),
            "expected at least one provider_batch event (got events: {:?})",
            names
        );
        // every batch should carry source = "artifact" with an artifact path
        for (_, payload) in &batch_events {
            assert_eq!(
                payload.get("source").and_then(|v| v.as_str()),
                Some("artifact"),
                "batch should originate from artifact (payload={:?})",
                payload
            );
            assert!(
                payload
                    .get("artifact")
                    .and_then(|v| v.as_str())
                    .map(|p| p.ends_with(".json"))
                    .unwrap_or(false),
                "artifact path should be set (payload={:?})",
                payload
            );
        }
        assert!(
            names.iter().any(|name| *name == "provider_completed"),
            "expected provider_completed in {:?}",
            names
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cli_json_runtime_runs_in_project_tool_output_dir() {
        use std::os::unix::fs::PermissionsExt;

        let tools_dir = tempfile::tempdir().unwrap();
        let project_root = tempfile::tempdir().unwrap();
        let executable = tools_dir.path().join("fake-asset-intel.sh");
        fs::write(
            &executable,
            r#"#!/bin/sh
case "$(pwd)" in
  */.golish/tool-output/asset-intel/run-cwd/fake-cli)
    printf '{"payload":{"icp":[{"domain":"cwd.example","title":"CWD OK"}]}}'
    ;;
  *)
    echo "bad cwd: $(pwd)" >&2
    exit 2
    ;;
esac
"#,
        )
        .unwrap();
        let mut perms = fs::metadata(&executable).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&executable, perms).unwrap();

        let tool = ToolConfig {
            id: "fake-cli".into(),
            name: "Fake CLI".into(),
            executable: "fake-asset-intel.sh".into(),
            runtime: "native".into(),
            skills: vec![golish_pentest::models::ToolSkill {
                id: "company-default-json".into(),
                name: "Company JSON".into(),
                description: String::new(),
                args: String::new(),
                tags: vec![],
            }],
            asset_intel: Some(golish_pentest::models::AssetIntelToolConfig {
                enabled: true,
                provider_id: "fake-cli".into(),
                display_name: "Fake CLI".into(),
                capabilities: vec!["domains".into()],
                requires_integration: None,
                auto: golish_pentest::models::AssetIntelAutoConfig {
                    default: true,
                    priority: 1,
                },
                runtime: fake_runtime(),
                normalize: fake_normalize_config(),
                discovery: golish_pentest::models::AssetIntelDiscoveryConfig::default(),
                lookup: None,
            }),
            ..Default::default()
        };

        let (status, candidates, evidence, _profile) = run_cli_json_provider(
            &tool,
            std::slice::from_ref(&tool),
            tools_dir.path(),
            project_root.path(),
            "run-cwd",
            "Acme",
            &AssetIntelHydrateConfig::default(),
            None,
        )
        .await
        .unwrap();

        assert_eq!(status.status, AssetIntelProviderRunState::Completed);
        assert_eq!(candidates.targets.len(), 1);
        assert_eq!(candidates.targets[0].value, "cwd.example");
        assert!(
            evidence["outDir"].as_str().is_some_and(
                |path| path.ends_with(".golish/tool-output/asset-intel/run-cwd/fake-cli")
            )
        );
    }

    #[test]
    fn asset_intel_skill_args_render_config_bindings() {
        let mut bindings = std::collections::HashMap::new();
        bindings.insert(
            "min_ownership_percent".to_string(),
            "-invest {{config.min_ownership_percent}}".to_string(),
        );
        bindings.insert("depth".to_string(), "-deep {{config.depth}}".to_string());
        bindings.insert("include_branches".to_string(), "-branch".to_string());

        let rendered = render_asset_intel_skill_args(
            "-n \"{{org}}\" -json -out-dir \"{{out_dir}}\"",
            "小米",
            &PathBuf::from("/tmp/golish-enscan"),
            &AssetIntelHydrateConfig {
                min_ownership_percent: Some("51".into()),
                depth: Some("2".into()),
                include_branches: Some(true),
                create_candidates: Some(true),
            },
            &bindings,
        );

        assert_eq!(
            split_command_args(&rendered),
            vec![
                "-n",
                "小米",
                "-json",
                "-out-dir",
                "/tmp/golish-enscan",
                "-invest",
                "51",
                "-deep",
                "2",
                "-branch",
            ]
        );
    }

    #[test]
    fn select_asset_intel_providers_uses_json_auto_priority() {
        fn tool(id: &str, priority: i32, enabled: bool) -> ToolConfig {
            ToolConfig {
                id: id.into(),
                name: id.into(),
                asset_intel: Some(golish_pentest::models::AssetIntelToolConfig {
                    enabled: true,
                    provider_id: id.into(),
                    display_name: id.into(),
                    capabilities: vec!["domains".into()],
                    requires_integration: None,
                    auto: golish_pentest::models::AssetIntelAutoConfig {
                        default: enabled,
                        priority,
                    },
                    runtime: fake_runtime(),
                    normalize: golish_pentest::models::AssetIntelNormalizeConfig::default(),
                    discovery: golish_pentest::models::AssetIntelDiscoveryConfig::default(),
                    lookup: None,
                }),
                ..Default::default()
            }
        }

        let tools = vec![
            tool("low", 10, true),
            tool("high", 100, true),
            tool("off", 200, false),
        ];
        let selected = select_asset_intel_providers(&tools, &[]).unwrap();

        assert_eq!(
            selected
                .iter()
                .map(|tool| provider_id_for_tool(tool).unwrap())
                .collect::<Vec<_>>(),
            vec!["high".to_string(), "low".to_string()]
        );
    }

    /// Shared fixture for two-phase selector tests: 3 providers covering
    /// the realistic mix we ship today.
    /// - `enscan-go`: subsidiaries + domains (discovery-capable)
    /// - `0.zone`:   domains + apps (enrichment-only)
    /// - `legacy`:   domains, auto.default=false (excluded by auto filter)
    fn two_phase_fixture_tools() -> Vec<ToolConfig> {
        fn tool(id: &str, caps: &[&str], priority: i32, auto_default: bool) -> ToolConfig {
            ToolConfig {
                id: id.into(),
                name: id.into(),
                asset_intel: Some(golish_pentest::models::AssetIntelToolConfig {
                    enabled: true,
                    provider_id: id.into(),
                    display_name: id.into(),
                    capabilities: caps.iter().map(|s| (*s).to_string()).collect(),
                    requires_integration: None,
                    auto: golish_pentest::models::AssetIntelAutoConfig {
                        default: auto_default,
                        priority,
                    },
                    runtime: fake_runtime(),
                    normalize: golish_pentest::models::AssetIntelNormalizeConfig::default(),
                    discovery: golish_pentest::models::AssetIntelDiscoveryConfig::default(),
                    lookup: None,
                }),
                ..Default::default()
            }
        }

        vec![
            tool("enscan-go", &["subsidiaries", "domains", "icp"], 100, true),
            tool("0.zone", &["domains", "apps", "contacts"], 90, true),
            tool("legacy", &["domains"], 50, false),
        ]
    }

    #[test]
    fn select_subsidiary_providers_keeps_only_subsidiaries_capable_tools() {
        let tools = two_phase_fixture_tools();
        let selected = select_subsidiary_providers(&tools, &[]).unwrap();
        assert_eq!(
            selected
                .iter()
                .map(|t| provider_id_for_tool(t).unwrap())
                .collect::<Vec<_>>(),
            vec!["enscan-go".to_string()],
            "only enscan-go declares the subsidiaries capability"
        );
    }

    fn multi_provider_tool(id: &str, providers: &[(&str, &[&str], bool, i32)]) -> ToolConfig {
        ToolConfig {
            id: id.into(),
            name: id.into(),
            executable: format!("{id}/bin"),
            asset_intel_providers: Some(
                providers
                    .iter()
                    .map(|(pid, caps, default, priority)| {
                        golish_pentest::models::AssetIntelToolConfig {
                            enabled: true,
                            provider_id: (*pid).into(),
                            display_name: (*pid).into(),
                            capabilities: caps.iter().map(|c| (*c).to_string()).collect(),
                            requires_integration: None,
                            auto: golish_pentest::models::AssetIntelAutoConfig {
                                default: *default,
                                priority: *priority,
                            },
                            runtime: fake_runtime(),
                            normalize: golish_pentest::models::AssetIntelNormalizeConfig::default(),
                            discovery: golish_pentest::models::AssetIntelDiscoveryConfig::default(),
                            lookup: None,
                        }
                    })
                    .collect(),
            ),
            ..Default::default()
        }
    }

    #[test]
    fn select_subsidiary_providers_expands_multi_provider_tool() {
        let tool = multi_provider_tool(
            "multi",
            &[
                ("multi-hi", &["subsidiaries"], true, 100),
                ("multi-lo", &["subsidiaries"], true, 50),
            ],
        );
        let selected = select_subsidiary_providers(&[tool], &[]).unwrap();
        assert_eq!(selected.len(), 2);
        let ids: Vec<String> = selected
            .iter()
            .map(|t| provider_id_for_tool(t).unwrap())
            .collect();
        assert_eq!(ids, vec!["multi-hi".to_string(), "multi-lo".to_string()]);
    }

    #[test]
    fn select_asset_intel_providers_treats_multi_provider_tool_as_single_pool() {
        // Tool A has two providers (priority 50 / 100); tool B has one (priority 75).
        // Expected sort across both tools: [100, 75, 50].
        let tool_a = multi_provider_tool(
            "multi",
            &[
                ("multi-low", &["subsidiaries"], true, 50),
                ("multi-high", &["subsidiaries"], true, 100),
            ],
        );
        let tool_b = ToolConfig {
            id: "single".into(),
            name: "single".into(),
            asset_intel: Some(golish_pentest::models::AssetIntelToolConfig {
                enabled: true,
                provider_id: "single-mid".into(),
                display_name: "single".into(),
                capabilities: vec!["subsidiaries".into()],
                requires_integration: None,
                auto: golish_pentest::models::AssetIntelAutoConfig {
                    default: true,
                    priority: 75,
                },
                runtime: fake_runtime(),
                normalize: Default::default(),
                discovery: Default::default(),
                lookup: None,
            }),
            ..Default::default()
        };
        let selected = select_asset_intel_providers(&[tool_a, tool_b], &[]).unwrap();
        let ids: Vec<String> = selected
            .iter()
            .map(|t| provider_id_for_tool(t).unwrap())
            .collect();
        assert_eq!(
            ids,
            vec![
                "multi-high".to_string(),
                "single-mid".to_string(),
                "multi-low".to_string(),
            ]
        );
    }

    #[test]
    fn provider_descriptors_from_tools_unpacks_multi_provider_tool() {
        let tool = multi_provider_tool(
            "multi",
            &[
                ("multi-a", &["subsidiaries"], true, 100),
                ("multi-b", &["domains"], false, 50),
            ],
        );
        let descriptors = provider_descriptors_from_tools(&[tool]);
        assert_eq!(descriptors.len(), 2);
        let ids: Vec<&str> = descriptors.iter().map(|d| d.id.as_str()).collect();
        assert!(ids.contains(&"multi-a"));
        assert!(ids.contains(&"multi-b"));
    }

    #[test]
    fn expand_provider_tools_clones_each_provider_into_virtual_tool() {
        let tool = multi_provider_tool(
            "shared",
            &[
                ("shared", &["subsidiaries"], true, 100),
                ("shared-alt", &["subsidiaries"], false, 50),
            ],
        );
        let expanded = expand_provider_tools(&[tool]);
        assert_eq!(expanded.len(), 2);
        assert_eq!(provider_id_for_tool(&expanded[0]).unwrap(), "shared");
        assert_eq!(provider_id_for_tool(&expanded[1]).unwrap(), "shared-alt");
        assert_eq!(expanded[0].executable, "shared/bin");
        assert_eq!(expanded[1].executable, "shared/bin");
        assert!(
            expanded[0].asset_intel_providers.is_none(),
            "virtual tool must not carry providers vec"
        );
        assert!(
            expanded[1].asset_intel_providers.is_none(),
            "virtual tool must not carry providers vec"
        );
    }

    #[test]
    fn expand_provider_tools_passes_single_asset_intel_tool_through_unchanged() {
        let tools = two_phase_fixture_tools();
        let expanded = expand_provider_tools(&tools);
        assert_eq!(
            expanded
                .iter()
                .map(|t| provider_id_for_tool(t).unwrap())
                .collect::<Vec<_>>(),
            vec![
                "enscan-go".to_string(),
                "0.zone".to_string(),
                "legacy".to_string(),
            ],
            "single-provider tools must be cloned 1:1 in scan order"
        );
    }

    #[test]
    fn expand_provider_tools_skips_disabled_providers() {
        let mut tool = multi_provider_tool(
            "shared",
            &[
                ("off", &["subsidiaries"], true, 1),
                ("on", &["subsidiaries"], true, 1),
            ],
        );
        // Mark the first provider disabled so the helper exercises the enabled filter.
        if let Some(providers) = tool.asset_intel_providers.as_mut() {
            providers[0].enabled = false;
        }
        let expanded = expand_provider_tools(&[tool]);
        assert_eq!(expanded.len(), 1);
        assert_eq!(provider_id_for_tool(&expanded[0]).unwrap(), "on");
    }

    #[test]
    fn fixture_discovery_auto_defaults_skip_tyc_until_upstream_is_stable() {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let toolsconfig_dir = std::path::PathBuf::from(manifest_dir)
            .join("..")
            .join("..")
            .join("..")
            .join("resources")
            .join("toolsconfig");
        if !toolsconfig_dir.exists() {
            eprintln!(
                "fixture skipped: toolsconfig dir not found at {}",
                toolsconfig_dir.display()
            );
            return;
        }
        let scan = golish_pentest::scan_toolsconfig(&toolsconfig_dir);
        assert!(
            scan.success,
            "toolsconfig scan failed: {:?}",
            scan.error.as_deref()
        );

        let selected = select_subsidiary_providers(&scan.tools, &[]).unwrap();

        assert_eq!(
            selected
                .iter()
                .map(|tool| provider_id_for_tool(tool).unwrap())
                .collect::<Vec<_>>(),
            vec![
                "enscan-go".to_string(),
                "enscan-go-kc-discovery".to_string(),
                "enscan-go-rb-discovery".to_string(),
            ],
            "default discovery should skip TYC while ENScan_GO v2.0.5 TYC discovery is unstable"
        );
    }

    #[test]
    fn fixture_enrichment_profile_fields_cover_observed_provider_keys() {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let toolsconfig_dir = std::path::PathBuf::from(manifest_dir)
            .join("..")
            .join("..")
            .join("..")
            .join("resources")
            .join("toolsconfig");
        if !toolsconfig_dir.exists() {
            eprintln!(
                "fixture skipped: toolsconfig dir not found at {}",
                toolsconfig_dir.display()
            );
            return;
        }
        let scan = golish_pentest::scan_toolsconfig(&toolsconfig_dir);
        assert!(
            scan.success,
            "toolsconfig scan failed: {:?}",
            scan.error.as_deref()
        );
        fn has_rule(
            asset: &golish_pentest::models::AssetIntelToolConfig,
            path: &str,
            source: &str,
            target: &str,
            kind: golish_pentest::models::AssetIntelProfileFieldTarget,
        ) -> bool {
            asset.normalize.profile_fields.iter().any(|rule| {
                rule.path == path
                    && rule.target_field == target
                    && rule.target_kind == kind
                    && matches!(
                        &rule.source_field,
                        golish_pentest::models::AssetIntelFieldRef::Field(field) if field == source
                    )
            })
        }

        let expanded = expand_provider_tools(&scan.tools);
        let zone = expanded
            .iter()
            .find(|tool| provider_id_for_tool(tool).as_deref() == Some("0.zone"))
            .and_then(|tool| tool.asset_intel.as_ref())
            .expect("0.zone provider fixture");
        let enscan = expanded
            .iter()
            .find(|tool| provider_id_for_tool(tool).as_deref() == Some("enscan-go-enrichment"))
            .and_then(|tool| tool.asset_intel.as_ref())
            .expect("ENScan enrichment provider fixture");

        assert!(
            has_rule(
                zone,
                "$..data[*]",
                "ip",
                "ip_ranges",
                golish_pentest::models::AssetIntelProfileFieldTarget::Scalar
            ),
            "0.zone site.ip should hydrate organization ip_ranges"
        );
        assert!(
            has_rule(
                zone,
                "$..data[*]",
                "asn",
                "asns",
                golish_pentest::models::AssetIntelProfileFieldTarget::Scalar
            ),
            "0.zone site.asn should hydrate organization asns"
        );
        assert!(
            has_rule(
                zone,
                "$..data[*]",
                "msg.code",
                "credit_code",
                golish_pentest::models::AssetIntelProfileFieldTarget::Scalar
            ),
            "0.zone org.msg.code should hydrate credit_code"
        );
        assert!(
            has_rule(
                enscan,
                "$..enterprise_info[*]",
                "scope",
                "business_scope",
                golish_pentest::models::AssetIntelProfileFieldTarget::Intel
            ),
            "ENScan enterprise scope should be preserved in intel"
        );
        assert!(
            has_rule(
                enscan,
                "$..icp[*]",
                "icp",
                "icp_records",
                golish_pentest::models::AssetIntelProfileFieldTarget::Intel
            ),
            "ENScan ICP license number should be preserved in intel"
        );

        let credit_rule = zone
            .normalize
            .profile_fields
            .iter()
            .find(|rule| {
                rule.target_field == "credit_code"
                    && matches!(
                        &rule.source_field,
                        golish_pentest::models::AssetIntelFieldRef::Field(field) if field == "msg.code"
                    )
            })
            .expect("0.zone msg.code -> credit_code rule must exist");
        assert!(
            credit_rule.when.iter().any(|clause| {
                clause.field == "name_cn"
                    && matches!(
                        clause.op,
                        golish_pentest::models::AssetIntelNormalizeFilterOp::Exists
                    )
            }),
            "0.zone msg.code -> credit_code must require name_cn presence to avoid pulling \
             apk/site/domain msg.code values into the master organization profile"
        );

        for target_kind in [
            golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
            golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
            golish_pentest::models::AssetIntelProfileFieldTarget::Contact,
        ] {
            for rule in zone.normalize.profile_fields.iter() {
                if rule.target_kind != target_kind {
                    continue;
                }
                if !matches!(
                    &rule.source_field,
                    golish_pentest::models::AssetIntelFieldRef::Field(field)
                        if matches!(
                            field.as_str(),
                            "msg.industry"
                            | "msg.legal_person"
                            | "msg.reg_address"
                            | "msg.reg_time"
                            | "msg.capital"
                            | "msg.business"
                            | "msg.email[0]"
                            | "msg.contact_number"
                            | "msg.website[0]"
                        )
                ) {
                    continue;
                }
                assert!(
                    rule.when.iter().any(|clause| {
                        clause.field == "name_cn"
                            && matches!(
                                clause.op,
                                golish_pentest::models::AssetIntelNormalizeFilterOp::Exists
                            )
                    }),
                    "0.zone {:?} -> {} rule must require name_cn presence (org-only field), \
                     otherwise apk/site/domain records can pollute the master record",
                    rule.source_field,
                    rule.target_field
                );
            }
        }

        assert!(
            !zone
                .normalize
                .profile_fields
                .iter()
                .any(|rule| rule.target_field == "certificates"),
            "0.zone must not map ssl_certificate (a static-asset URL) into organization \
             certificates; revisit when we add a real cert subject extractor"
        );

        let apk_rule = zone
            .normalize
            .profile_fields
            .iter()
            .find(|rule| {
                rule.target_field == "mobile_apps"
                    && matches!(
                        &rule.source_field,
                        golish_pentest::models::AssetIntelFieldRef::FirstOf(items)
                            if items.iter().any(|s| s == "msg.app_url")
                    )
            })
            .expect("0.zone apk -> mobile_apps rule must exist");
        if let golish_pentest::models::AssetIntelFieldRef::FirstOf(items) = &apk_rule.source_field {
            assert!(
                !items.iter().any(|s| s == "title"),
                "0.zone apk -> mobile_apps must NOT fall back to `title` \
                 (网页 SEO 标题被误塞进 business systems 是上轮发现的 bug)"
            );
        }
    }

    #[test]
    fn select_enrichment_providers_excludes_subsidiaries_capable_tools() {
        let tools = two_phase_fixture_tools();
        let selected = select_enrichment_providers(&tools, &[]).unwrap();
        assert_eq!(
            selected
                .iter()
                .map(|t| provider_id_for_tool(t).unwrap())
                .collect::<Vec<_>>(),
            vec!["0.zone".to_string()],
            "0.zone is the only auto-default non-subsidiaries provider"
        );
    }

    #[test]
    fn enrichment_config_disables_candidate_queue_writes() {
        let config = enrichment_hydrate_config(AssetIntelHydrateConfig {
            min_ownership_percent: Some("35".into()),
            depth: Some("2".into()),
            include_branches: Some(true),
            create_candidates: Some(true),
        });

        assert_eq!(config.min_ownership_percent.as_deref(), Some("35"));
        assert_eq!(config.depth.as_deref(), Some("2"));
        assert_eq!(config.include_branches, Some(true));
        assert_eq!(config.create_candidates, Some(false));
    }

    #[test]
    fn enrich_organization_config_disables_candidate_queue_writes() {
        let args = AssetIntelEnrichOrganizationArgs {
            organization_id: Uuid::new_v4().to_string(),
            provider_ids: Vec::new(),
            config: AssetIntelHydrateConfig {
                min_ownership_percent: Some("35".into()),
                depth: Some("2".into()),
                include_branches: Some(true),
                create_candidates: Some(true),
            },
        };

        let config = enrichment_hydrate_config_for_organization(&args);

        assert_eq!(config.min_ownership_percent.as_deref(), Some("35"));
        assert_eq!(config.depth.as_deref(), Some("2"));
        assert_eq!(config.include_branches, Some(true));
        assert_eq!(config.create_candidates, Some(false));
    }

    #[test]
    fn select_subsidiary_providers_rejects_explicit_request_for_enrichment_tool() {
        let tools = two_phase_fixture_tools();
        let err = select_subsidiary_providers(&tools, &["0.zone".to_string()])
            .expect_err("requesting 0.zone for discovery must reject");
        let msg = format!("{err}");
        assert!(
            msg.contains("subsidiaries") && msg.contains("0.zone"),
            "error should mention both the missing capability and the offending provider, got: {msg}"
        );
    }

    #[test]
    fn select_enrichment_providers_rejects_explicit_request_for_subsidiaries_tool() {
        let tools = two_phase_fixture_tools();
        let err = select_enrichment_providers(&tools, &["enscan-go".to_string()])
            .expect_err("requesting enscan-go for enrichment must reject");
        let msg = format!("{err}");
        assert!(
            msg.contains("discovery") && msg.contains("enscan-go"),
            "error should direct caller to hydrate_subsidiaries, got: {msg}"
        );
    }

    #[test]
    fn provider_has_subsidiaries_is_case_insensitive() {
        let tool = ToolConfig {
            id: "casing".into(),
            name: "casing".into(),
            asset_intel: Some(golish_pentest::models::AssetIntelToolConfig {
                enabled: true,
                provider_id: "casing".into(),
                display_name: "casing".into(),
                capabilities: vec!["Subsidiaries".into()],
                requires_integration: None,
                auto: golish_pentest::models::AssetIntelAutoConfig {
                    default: true,
                    priority: 1,
                },
                runtime: fake_runtime(),
                normalize: golish_pentest::models::AssetIntelNormalizeConfig::default(),
                discovery: golish_pentest::models::AssetIntelDiscoveryConfig::default(),
                lookup: None,
            }),
            ..Default::default()
        };
        assert!(
            provider_has_subsidiaries(&tool),
            "capability matching must be case-insensitive so JSON authors don't get bit"
        );
    }

    #[test]
    fn normalize_when_filter_drops_low_ownership_invest_rows() {
        let mut normalize = fake_normalize_config();
        // The org rule covers `$..invest[*]` already; layer a numeric filter
        // that only keeps rows with `scale >= 51`. Anything below should drop
        // out of the candidate pool entirely.
        normalize.organization[0].when = vec![golish_pentest::models::AssetIntelNormalizeFilter {
            field: "scale".into(),
            op: golish_pentest::models::AssetIntelNormalizeFilterOp::Gte,
            value: "51".into(),
        }];
        let raw = serde_json::json!({
            "payload": {
                "invest": [
                    { "name": "全资子公司", "scale": "100" },
                    { "name": "少数股权",   "scale": "5"   },
                    { "name": "缺字段公司"                  },
                ]
            }
        });

        let (candidates, _profile) =
            normalize_json_with_descriptor("filter-provider", "run-filter", 99, &normalize, &raw);

        assert_eq!(
            candidates
                .organizations
                .iter()
                .map(|c| c.label.as_str())
                .collect::<Vec<_>>(),
            vec!["全资子公司"],
            "only rows passing scale>=51 should remain"
        );
    }

    #[test]
    fn normalize_when_filter_contains_op_keeps_matching_rows() {
        let mut normalize = fake_normalize_config();
        normalize.organization[0].when = vec![golish_pentest::models::AssetIntelNormalizeFilter {
            field: "entity_type".into(),
            op: golish_pentest::models::AssetIntelNormalizeFilterOp::Contains,
            value: "公司".into(),
        }];
        let raw = serde_json::json!({
            "data": {
                "invest": [
                    { "name": "测试有限公司", "entity_type": "有限责任公司" },
                    { "name": "个体张三",      "entity_type": "个体工商户"   },
                ]
            }
        });

        let (candidates, _profile) =
            normalize_json_with_descriptor("filter-provider", "run-contains", 1, &normalize, &raw);

        assert_eq!(candidates.organizations.len(), 1);
        assert_eq!(candidates.organizations[0].label, "测试有限公司");
    }

    #[test]
    fn normalize_when_filter_exists_drops_empty_fields() {
        let mut normalize = fake_normalize_config();
        normalize.organization[0].when = vec![golish_pentest::models::AssetIntelNormalizeFilter {
            field: "pid".into(),
            op: golish_pentest::models::AssetIntelNormalizeFilterOp::Exists,
            value: String::new(),
        }];
        let raw = serde_json::json!({
            "data": {
                "invest": [
                    { "name": "已知 pid", "pid": "abc" },
                    { "name": "缺 pid"                 },
                    { "name": "空 pid",   "pid": ""    },
                ]
            }
        });

        let (candidates, _profile) =
            normalize_json_with_descriptor("filter-provider", "run-exists", 1, &normalize, &raw);

        assert_eq!(candidates.organizations.len(), 1);
        assert_eq!(candidates.organizations[0].label, "已知 pid");
    }

    #[test]
    fn extract_profile_field_entries_scalar_intel_contact_buckets() {
        let rules = vec![
            golish_pentest::models::AssetIntelProfileFieldRule {
                path: "$..enterprise_info[*]".into(),
                source_field: golish_pentest::models::AssetIntelFieldRef::Field("reg_code".into()),
                target_field: "credit_code".into(),
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                transform: golish_pentest::models::AssetIntelProfileFieldTransform::None,
                when: vec![],
            },
            golish_pentest::models::AssetIntelProfileFieldRule {
                path: "$..enterprise_info[*]".into(),
                source_field: golish_pentest::models::AssetIntelFieldRef::Field("legal".into()),
                target_field: "legal_representative".into(),
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
                transform: golish_pentest::models::AssetIntelProfileFieldTransform::Trim,
                when: vec![],
            },
            golish_pentest::models::AssetIntelProfileFieldRule {
                path: "$..enterprise_info[*]".into(),
                source_field: golish_pentest::models::AssetIntelFieldRef::Field("email".into()),
                target_field: "email".into(),
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Contact,
                transform: golish_pentest::models::AssetIntelProfileFieldTransform::Lower,
                when: vec![],
            },
            golish_pentest::models::AssetIntelProfileFieldRule {
                path: "$..enterprise_info[*]".into(),
                source_field: golish_pentest::models::AssetIntelFieldRef::Field("phone".into()),
                target_field: "phone".into(),
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Contact,
                transform: golish_pentest::models::AssetIntelProfileFieldTransform::None,
                when: vec![],
            },
        ];
        let raw = serde_json::json!({
            "payload": {
                "enterprise_info": [
                    {
                        "name": "小米科技",
                        "reg_code": "91110108551385082Q",
                        "legal": "  雷军  ",
                        "email": "Press@MI.com",
                        "phone": "010-12345678"
                    }
                ]
            }
        });

        let entries = extract_profile_field_entries(&rules, &raw);

        assert_eq!(entries.len(), 4);
        let by_field: HashMap<_, _> = entries
            .iter()
            .map(|e| (e.target_field.as_str(), e.value.as_str()))
            .collect();
        assert_eq!(by_field["credit_code"], "91110108551385082Q");
        assert_eq!(by_field["legal_representative"], "雷军"); // trim
        assert_eq!(by_field["email"], "press@mi.com"); // lower
        assert_eq!(by_field["phone"], "010-12345678");
    }

    #[test]
    fn extract_profile_field_entries_when_filter_drops_placeholder_values() {
        // ENScan AQC returns "-" (single dash) as a placeholder for missing
        // email / phone. Without a `when` filter that placeholder would land
        // in organizations.intel.contacts.email and pollute the master record.
        let rules = vec![
            golish_pentest::models::AssetIntelProfileFieldRule {
                path: "$..enterprise_info[*]".into(),
                source_field: golish_pentest::models::AssetIntelFieldRef::Field("email".into()),
                target_field: "email".into(),
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Contact,
                transform: golish_pentest::models::AssetIntelProfileFieldTransform::Lower,
                when: vec![golish_pentest::models::AssetIntelNormalizeFilter {
                    field: "email".into(),
                    op: golish_pentest::models::AssetIntelNormalizeFilterOp::Ne,
                    value: "-".into(),
                }],
            },
            golish_pentest::models::AssetIntelProfileFieldRule {
                path: "$..enterprise_info[*]".into(),
                source_field: golish_pentest::models::AssetIntelFieldRef::Field("phone".into()),
                target_field: "phone".into(),
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Contact,
                transform: golish_pentest::models::AssetIntelProfileFieldTransform::Trim,
                when: vec![golish_pentest::models::AssetIntelNormalizeFilter {
                    field: "phone".into(),
                    op: golish_pentest::models::AssetIntelNormalizeFilterOp::Ne,
                    value: "-".into(),
                }],
            },
        ];
        let raw = serde_json::json!({
            "enterprise_info": [
                {
                    // dash placeholders — both must drop out
                    "email": "-",
                    "phone": "-"
                },
                {
                    // real values — must pass through
                    "email": "Press@MI.com",
                    "phone": "010-12345678"
                }
            ]
        });

        let entries = extract_profile_field_entries(&rules, &raw);

        assert_eq!(entries.len(), 2, "only the real-value row survives");
        assert_eq!(entries[0].target_field, "email");
        assert_eq!(entries[0].value, "press@mi.com");
        assert_eq!(entries[1].target_field, "phone");
        assert_eq!(entries[1].value, "010-12345678");
    }

    #[test]
    fn build_profile_patch_first_wins_for_scalar_intel_contact_dedupes() {
        let entries = vec![
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "credit_code".into(),
                value: "AAA".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "credit_code".into(),
                value: "BBB".into(), // duplicate from another provider — must NOT overwrite
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "industry".into(),
                value: "互联网".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
                target_field: "legal_representative".into(),
                value: "雷军".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Contact,
                target_field: "email".into(),
                value: "a@example.com".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Contact,
                target_field: "email".into(),
                value: "A@example.com".into(), // case-only diff → must dedupe
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Contact,
                target_field: "email".into(),
                value: "b@example.com".into(),
            },
        ];
        let existing_intel = serde_json::json!({
            "contacts": { "email": ["preexisting@example.com"] },
            "engagement": { "mode": "discover_assets" }
        });

        let patch = build_profile_patch_from_entries(&existing_intel, &entries)
            .expect("patch build ok")
            .expect("patch is Some when entries present");

        assert_eq!(patch.credit_code.as_deref(), Some("AAA"));
        assert_eq!(patch.industry.as_deref(), Some("互联网"));
        let intel = patch.intel.expect("intel patched");
        assert_eq!(
            intel["legal_representative"],
            serde_json::Value::String("雷军".into())
        );
        let emails = intel["contacts"]["email"].as_array().expect("email array");
        let strs: Vec<&str> = emails.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(
            strs,
            vec!["preexisting@example.com", "a@example.com", "b@example.com"]
        );
        // engagement metadata must survive
        assert_eq!(
            intel["engagement"]["mode"],
            serde_json::Value::String("discover_assets".into())
        );
    }

    #[test]
    fn build_profile_patch_dedupes_multi_value_intel_fields() {
        let entries = vec![
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
                target_field: "icp_records".into(),
                value: "粤ICP备06118290号-2".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
                target_field: "icp_records".into(),
                value: "粤ICP备06118290号-2".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
                target_field: "icp_records".into(),
                value: "粤ICP备06118290号-16".into(),
            },
        ];
        let existing_intel = serde_json::json!({
            "icp_records": ["粤ICP备06118290号-1"]
        });

        let patch = build_profile_patch_from_entries(&existing_intel, &entries)
            .expect("patch build ok")
            .expect("patch is Some when entries present");

        assert_eq!(
            patch.intel.expect("intel patched")["icp_records"],
            serde_json::json!([
                "粤ICP备06118290号-1",
                "粤ICP备06118290号-2",
                "粤ICP备06118290号-16"
            ])
        );
    }

    #[test]
    fn build_profile_patch_dedupes_app_intel_array_fields() {
        let entries = vec![
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
                target_field: "mobile_apps".into(),
                value: "小米实况麻将".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
                target_field: "mobile_apps".into(),
                value: "小米实况麻将".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
                target_field: "mini_programs".into(),
                value: "小米商城".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
                target_field: "app_domains".into(),
                value: "https://com.dfwe".into(),
            },
        ];

        let patch = build_profile_patch_from_entries(&serde_json::json!({}), &entries)
            .expect("patch build ok")
            .expect("app intel entries should produce a patch");
        let intel = patch.intel.expect("intel patched");

        assert_eq!(intel["mobile_apps"], serde_json::json!(["小米实况麻将"]));
        assert_eq!(intel["mini_programs"], serde_json::json!(["小米商城"]));
        assert_eq!(
            intel["app_domains"],
            serde_json::json!(["https://com.dfwe"])
        );
    }

    #[test]
    fn build_profile_patch_writes_visible_profile_array_fields() {
        let entries = vec![
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "domains".into(),
                value: "example.com".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "domains".into(),
                value: "EXAMPLE.com".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "email_domains".into(),
                value: "example.com".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "business_systems".into(),
                value: "Example App".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "social_accounts".into(),
                value: "wechat:example".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "contacts".into(),
                value: "ir@example.com".into(),
            },
        ];

        let patch = build_profile_patch_from_entries(&serde_json::json!({}), &entries)
            .expect("patch build ok")
            .expect("patch is Some when profile fields are present");

        assert_eq!(patch.domains, Some(serde_json::json!(["example.com"])));
        assert_eq!(
            patch.email_domains,
            Some(serde_json::json!(["example.com"]))
        );
        assert_eq!(
            patch.business_systems,
            Some(serde_json::json!(["Example App"]))
        );
        assert_eq!(
            patch.social_accounts,
            Some(serde_json::json!(["wechat:example"]))
        );
        assert_eq!(patch.contacts, Some(serde_json::json!(["ir@example.com"])));
    }

    #[test]
    fn extract_profile_fields_normalizes_asn_values() {
        let rules = vec![golish_pentest::models::AssetIntelProfileFieldRule {
            path: "$..data[*]".into(),
            source_field: golish_pentest::models::AssetIntelFieldRef::Field("asn".into()),
            target_field: "asns".into(),
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
            transform: golish_pentest::models::AssetIntelProfileFieldTransform::Asn,
            when: vec![],
        }];
        let raw = serde_json::json!({
            "data": [
                { "asn": 4134 },
                { "asn": " as37963 " },
                { "asn": "not-an-asn" }
            ]
        });

        let entries = extract_profile_field_entries(&rules, &raw);
        let patch = build_profile_patch_from_entries(&serde_json::json!({}), &entries)
            .expect("patch build ok")
            .expect("asn entries should produce a patch");

        assert_eq!(patch.asns, Some(serde_json::json!(["AS4134", "AS37963"])));
    }

    #[test]
    fn team_cymru_asn_lookup_builds_profile_entries_from_public_ips() {
        let entries = vec![
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "ip_ranges".into(),
                value: "183.62.123.10".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "ip_ranges".into(),
                value: "182.92.121.121".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "ip_ranges".into(),
                value: "10.0.0.1".into(),
            },
        ];
        let response = "\
AS      | IP               | BGP Prefix          | CC | Registry | Allocated  | AS Name
4134    | 183.62.123.10    | 183.56.0.0/13       | CN | apnic    | 2009-09-29 | CHINANET-BACKBONE
37963   | 182.92.121.121   | 182.92.0.0/16       | CN | apnic    | 2013-08-16 | ALIBABA-CN-NET
";

        let ips = collect_public_ips_for_asn_lookup(&entries);
        let mappings = parse_team_cymru_asn_response(response);
        let derived = profile_asn_entries_from_mappings(&mappings);
        let patch = build_profile_patch_from_entries(&serde_json::json!({}), &derived)
            .expect("patch build ok")
            .expect("derived ASN entries should produce a patch");

        assert_eq!(
            ips.iter().map(ToString::to_string).collect::<Vec<_>>(),
            vec!["183.62.123.10", "182.92.121.121"]
        );
        assert_eq!(patch.asns, Some(serde_json::json!(["AS4134", "AS37963"])));
    }

    #[test]
    fn build_profile_patch_returns_none_for_empty_or_blank_entries() {
        let entries = vec![ProfileFieldEntry {
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
            target_field: "credit_code".into(),
            value: "   ".into(),
        }];
        let intel = serde_json::json!({});
        let patch = build_profile_patch_from_entries(&intel, &entries).unwrap();
        assert!(
            patch.is_none(),
            "all-blank entries should not produce a patch"
        );
    }

    #[test]
    fn extract_lookup_matches_maps_enterprise_info_into_disambiguation_rows() {
        let config = golish_pentest::models::AssetIntelLookupConfig {
            enabled: true,
            skill_id: "company-lookup-json".into(),
            timeout_secs: 60,
            normalize: golish_pentest::models::AssetIntelLookupNormalize {
                path: "$..enterprise_info[*]".into(),
                name: golish_pentest::models::AssetIntelFieldRef::Field("name".into()),
                credit_code: Some(golish_pentest::models::AssetIntelFieldRef::Field(
                    "reg_code".into(),
                )),
                industry: Some(golish_pentest::models::AssetIntelFieldRef::Field(
                    "industry".into(),
                )),
                legal_representative: Some(golish_pentest::models::AssetIntelFieldRef::FirstOf(
                    vec!["legal_person".into(), "legal".into()],
                )),
                address: Some(golish_pentest::models::AssetIntelFieldRef::FirstOf(vec![
                    "reg_address".into(),
                    "addr".into(),
                ])),
                registered_at: Some(golish_pentest::models::AssetIntelFieldRef::Field(
                    "reg_date".into(),
                )),
                score: None,
                default_confidence: 0.68,
            },
        };
        let raw = serde_json::json!({
            "payload": {
                "enterprise_info": [
                    {
                        "name": "小米科技有限责任公司",
                        "reg_code": "91110108551385082Q",
                        "industry": "互联网",
                        "legal_person": "雷军",
                        "reg_address": "北京市海淀区清河中街68号",
                        "reg_date": "2010-03-03"
                    },
                    {
                        "name": "小米通讯技术有限公司",
                        "reg_code": "91440300325990618B",
                        "legal": "回退法人字段",
                        "addr": "回退地址字段"
                    }
                ]
            }
        });

        let matches = extract_lookup_matches("enscan-go", &config, &raw);

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].provider_id, "enscan-go");
        assert_eq!(matches[0].name, "小米科技有限责任公司");
        assert_eq!(
            matches[0].credit_code.as_deref(),
            Some("91110108551385082Q")
        );
        assert_eq!(matches[0].industry.as_deref(), Some("互联网"));
        assert_eq!(matches[0].legal_representative.as_deref(), Some("雷军"));
        assert_eq!(
            matches[0].address.as_deref(),
            Some("北京市海淀区清河中街68号")
        );
        assert_eq!(matches[0].registered_at.as_deref(), Some("2010-03-03"));
        assert!((matches[0].confidence - 0.68).abs() < f64::EPSILON);

        assert_eq!(matches[1].name, "小米通讯技术有限公司");
        assert_eq!(
            matches[1].legal_representative.as_deref(),
            Some("回退法人字段")
        );
        assert_eq!(matches[1].address.as_deref(), Some("回退地址字段"));
        assert!(
            matches[1].industry.is_none(),
            "missing field should stay None"
        );
        assert!(matches[1].registered_at.is_none());
    }

    #[test]
    fn dedupe_lookup_matches_prefers_credit_code_for_uniqueness() {
        let m1 = LookupCompanyMatch {
            provider_id: "enscan-go".into(),
            name: "小米科技有限责任公司".into(),
            credit_code: Some("91110108551385082Q".into()),
            industry: None,
            legal_representative: None,
            address: None,
            registered_at: None,
            confidence: 0.68,
            evidence: serde_json::json!({}),
        };
        let m2 = LookupCompanyMatch {
            provider_id: "another".into(),
            // Different display name but same credit code → must dedupe.
            name: "Xiaomi Inc".into(),
            credit_code: Some("91110108551385082q".into()), // case differs
            industry: None,
            legal_representative: None,
            address: None,
            registered_at: None,
            confidence: 0.5,
            evidence: serde_json::json!({}),
        };
        let m3 = LookupCompanyMatch {
            provider_id: "enscan-go".into(),
            name: "Acme Inc".into(),
            credit_code: None,
            industry: None,
            legal_representative: None,
            address: None,
            registered_at: None,
            confidence: 0.42,
            evidence: serde_json::json!({}),
        };
        let m4 = LookupCompanyMatch {
            provider_id: "another".into(),
            name: "  acme inc  ".into(), // case + whitespace only diff → must dedupe
            credit_code: None,
            industry: None,
            legal_representative: None,
            address: None,
            registered_at: None,
            confidence: 0.3,
            evidence: serde_json::json!({}),
        };

        let deduped = dedupe_lookup_matches(vec![m1.clone(), m2, m3.clone(), m4]);

        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].provider_id, "enscan-go");
        assert_eq!(deduped[0].name, m1.name);
        assert_eq!(deduped[1].name, "Acme Inc");
    }

    #[test]
    fn merge_candidates_dedupes_same_value_across_providers() {
        let mut merged = normalize_provider_records(
            "first-provider",
            "run-1",
            1,
            vec![AssetIntelProviderRecord {
                kind: OrganizationCandidateKind::Target,
                label: "api.example.com".into(),
                value: "api.example.com".into(),
                confidence: 0.8,
                evidence: serde_json::json!({"provider": "enscan"}),
            }],
        );
        let zone = normalize_provider_records(
            "second-provider",
            "run-1",
            1,
            vec![AssetIntelProviderRecord {
                kind: OrganizationCandidateKind::Target,
                label: "duplicate".into(),
                value: "API.EXAMPLE.COM".into(),
                confidence: 0.7,
                evidence: serde_json::json!({"provider": "zone"}),
            }],
        );

        merge_candidates(&mut merged, zone);

        assert_eq!(merged.targets.len(), 1);
        assert_eq!(merged.targets[0].source, "first-provider");
        assert_eq!(
            merged.targets[0]
                .evidence
                .get("sources")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(2)
        );
    }
}
