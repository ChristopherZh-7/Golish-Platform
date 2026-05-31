//! Asset Intel commands facade.
//!
//! Exposes:
//! - `asset_intel_list_providers` · list business-level discovery providers
//! - `asset_intel_lookup_company` · quick disambiguation lookup
//! - `asset_intel_hydrate` · legacy single-shot hydrate (every auto provider)
//! - `asset_intel_hydrate_subsidiaries` · two-phase discovery (subsidiaries only)
//! - `asset_intel_enrich_organization` · two-phase enrichment (single org)
//! - `asset_intel_enrich_batch` · two-phase enrichment (parent + children)

// Extracted to the golish-recon-app crate (crate-per-service split M2b).
pub use golish_recon_app::asset_intel::*;
