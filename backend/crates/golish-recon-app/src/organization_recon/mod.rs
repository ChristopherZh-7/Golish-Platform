//! Organization-level recon orchestration primitives.
//!
//! The full staged runner builds on these modules. Artifact and normalized
//! record contracts live here so existing asset-intel adapters can adopt the
//! same evidence format before the asynchronous IPC surface is wired.

mod active;
pub(crate) mod artifacts;
mod commands;
mod export;
pub(crate) mod normalize;
mod persistence;
mod runner;
mod state;
pub(crate) mod types;

// Shared coverage-gate landing (design 2026-06-15 §5 PR1): reused by the agent
// enrich path (`asset_intel`) so passive intel lands in the gate-read tables.
pub(crate) use persistence::land_target_intel_coverage;
// Standalone RDAP WHOIS landing (plan 2026-06-18-slim-enrich): exposed to the
// agent as the `recon_lookup_whois` tool, separate from provider survey.
pub(crate) use persistence::land_whois;
// Scope ownership check reused by the asset_intel landing path
// (design 2026-06-17 passive-intel-pairing §2④) to keep third-party noise out
// of auto-promoted targets.
pub(crate) use persistence::value_belongs_to_organization;
// Per-asset landing refresh (fix 2026-06-17 enrich-timing): callable from the
// submit-gate read path to close the "enrich lands before targets are registered"
// ordering gap (DNS/SUBDOMAIN never reached the gate-read tables).
pub use persistence::{refresh_per_asset_landing, refresh_per_asset_landing_summary};

pub use commands::*;
pub use runner::ORGANIZATION_RECON_EVENT;
pub use state::OrganizationReconState;
pub use types::{
    NormalizedReconRecord, OrganizationReconEvent, OrganizationReconExportResult,
    OrganizationReconRunSnapshot, OrganizationReconRunStatus, OrganizationReconStageName,
    OrganizationReconStageSnapshot, OrganizationReconStartArgs, OrganizationReconTaskSnapshot,
    OrganizationReconTraceEvent, OrganizationReconTraceKind, ReconArtifactRef, ReconEvidenceRef,
    ReconRecordKind, ReconTaskError, ReconTaskManifest, ReconTaskStatus,
};

#[doc(hidden)]
pub use commands::{
    __cmd__organization_recon_export_assets, __cmd__organization_recon_export_current_assets,
    __cmd__organization_recon_get_run, __cmd__organization_recon_start_run,
    __cmd__recon_backfill_real_ip,
};
