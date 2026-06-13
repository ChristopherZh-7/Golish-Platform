//! Asset Intel service for Discover Assets engagements.
//!
//! Phase 1 keeps this layer provider-agnostic: the workspace asks for
//! candidates, providers return normalized records, and only approved
//! candidates become scope in later phases.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::Mutex as TokioMutex;
use uuid::Uuid;

use golish_core::{emit_opt, EventEmitterHandle};
use golish_pentest::models::ToolConfig;

use crate::organizations::{upsert_organization_candidates_for_org, OrganizationCandidates};
#[cfg(test)]
use crate::organizations::{OrganizationCandidate, OrganizationCandidateKind};
use golish_app_core::DbState;
use golish_app_core::GolishError;
use golish_app_core::TauriEventEmitter;

/// Shared handle to the pentest **tools-config** manager, managed by the host
/// binary (`golish`) so recon-app's asset-intel commands can resolve the
/// `toolsconfig_dir` (and run `golish_pentest::scan_toolsconfig`) without
/// depending on golish's monolithic `PentestState`. The host clones the same
/// `Arc<ConfigManager>` it hands `PentestState`, so behaviour is identical.
#[derive(Clone)]
pub struct ToolsConfigState(pub Arc<golish_pentest::ConfigManager>);

mod agent_intel;
mod asn;
mod availability;
mod capability;
mod commands;
mod merge;
mod normalize;
mod profile_patch;
mod promote;
mod records;
mod runtime;
mod service;
mod template;
mod types;
pub use agent_intel::{run_passive_intel, PassiveIntelPhase, PassiveIntelSummary};
pub(crate) use asn::{
    collect_public_ips_for_asn_lookup, normalize_asn, parse_team_cymru_asn_response,
    profile_asn_entries_from_mappings, IpAsnMapping, TEAM_CYMRU_ASN_LOOKUP_TIMEOUT_SECS,
    TEAM_CYMRU_WHOIS_ADDR,
};
pub use availability::{list_provider_availability, ProviderAvailability};
#[cfg(test)]
pub(crate) use capability::{expand_provider_tools, provider_has_subsidiaries};
pub(crate) use capability::{
    provider_descriptors_from_tools, provider_id_for_tool, provider_output_is_trusted,
    select_asset_intel_providers, select_enrichment_providers, select_subsidiary_providers,
};
pub(crate) use merge::{flatten_candidates, merge_candidates};
pub(crate) use normalize::{
    extract_profile_field_entries, filter_passes, resolve_field_ref, select_json_values,
};
pub(crate) use profile_patch::{
    build_profile_patch_from_entries, merge_profile_patch_with_existing,
};
#[cfg(test)]
pub(crate) use promote::AutoPromoteSkipReason;
pub(crate) use promote::{
    apply_ownership_threshold_override, auto_promote_child_decisions,
    clear_engagement_candidates_from_intel, parse_ownership_percent, select_discovery_policy,
};
#[cfg(test)]
pub(crate) use records::normalize_provider_records;
pub(crate) use records::{
    dedupe_lookup_matches, extract_lookup_matches, normalize_json_document,
    normalize_json_with_descriptor,
};
pub(crate) use template::{
    collect_http_secret_refs, render_asset_intel_skill_args, render_http_json_value,
    render_http_template, render_lookup_skill_args, split_command_args,
};
#[cfg(test)]
pub(crate) use types::enrichment_hydrate_config_for_organization;
pub(crate) use types::{discovery_hydrate_config, enrichment_hydrate_config};
pub use types::{
    AssetIntelBatchSource, AssetIntelCapability, AssetIntelEnrichBatchArgs,
    AssetIntelEnrichBatchResult, AssetIntelEnrichBatchSkip, AssetIntelEnrichOrganizationArgs,
    AssetIntelHydrateArgs, AssetIntelHydrateConfig, AssetIntelIntegrationRequirement,
    AssetIntelLookupRequest, AssetIntelLookupResult, AssetIntelProviderDescriptor,
    AssetIntelProviderRecord, AssetIntelProviderRunState, AssetIntelProviderRunStatus,
    AssetIntelProviderRuntimeKind, AssetIntelProviderStatus, AssetIntelRun, AssetIntelRunStatus,
    AssetIntelStreamEvent, AssetIntelStreamSource, LookupCompanyMatch, ProfileFieldEntry,
};

pub use commands::*;
#[doc(hidden)]
pub use commands::{
    __cmd__asset_intel_enrich_batch, __cmd__asset_intel_enrich_organization,
    __cmd__asset_intel_hydrate, __cmd__asset_intel_hydrate_subsidiaries,
    __cmd__asset_intel_list_providers, __cmd__asset_intel_lookup_company,
};

pub(crate) use runtime::*;
pub(crate) use service::*;

/// Tauri event name used for all Asset Intel streaming events.
///
/// The frontend listens once on this channel and filters payloads by `runId`.
/// Kept as a constant so backend + frontend share a single source of truth
/// (frontend re-imports the literal in `lib/api/asset-intel.ts`).
pub const ASSET_INTEL_EVENT: &str = "asset-intel:event";

fn emit_event(sink: Option<&EventEmitterHandle>, event: AssetIntelStreamEvent) {
    emit_opt(sink, ASSET_INTEL_EVENT, &event);
}

#[cfg(test)]
mod tests;
