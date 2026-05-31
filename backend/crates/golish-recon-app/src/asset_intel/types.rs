//! Data transfer objects (DTOs) for the asset-intel module: streaming events,
//! provider/run descriptors, hydrate config + args, lookup matches, and the
//! profile field entry. Pure serde data shared by the runtime, commands, and
//! submodules. Re-exported from the parent module so existing call sites keep
//! using the bare type names.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::organizations::{OrganizationCandidateKind, OrganizationCandidates};

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

pub(crate) fn enrichment_hydrate_config(
    mut config: AssetIntelHydrateConfig,
) -> AssetIntelHydrateConfig {
    config.create_candidates = Some(false);
    config
}

#[cfg(test)]
pub(crate) fn enrichment_hydrate_config_for_organization(
    args: &AssetIntelEnrichOrganizationArgs,
) -> AssetIntelHydrateConfig {
    enrichment_hydrate_config(args.config.clone())
}

pub(crate) fn discovery_hydrate_config(
    mut config: AssetIntelHydrateConfig,
) -> AssetIntelHydrateConfig {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetIntelEnrichOrganizationArgs {
    pub organization_id: String,
    #[serde(default)]
    pub provider_ids: Vec<String>,
    #[serde(default)]
    pub config: AssetIntelHydrateConfig,
}

/// Args for [`super::asset_intel_enrich_batch`].
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

/// Result of [`super::asset_intel_enrich_batch`].
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
