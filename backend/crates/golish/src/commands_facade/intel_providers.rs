//! ASM intel-provider commands facade.
//!
//! Exposes:
//! - `intel_list_providers` · list all known ASM platforms + metadata
//! - `intel_test_connection` · verify a configured API key
//! - `intel_query_provider` · run a query, persist into `organizations`

// Extracted to the golish-recon-app crate (crate-per-service split M2a).
pub use golish_recon_app::intel_providers::*;
