//! Vulnerability intelligence commands (feeds, search, matching, POC discovery).
//!
//! Expected commands exposed here (documentation only):
//! - **Feeds**: `intel_list_feeds`, `intel_add_feed`,
//!   `intel_toggle_feed`, `intel_delete_feed`
//! - **Fetch**: `intel_fetch`, `intel_fetch_page`, `intel_get_cached`
//! - **Search**: `intel_search`, `intel_search_remote`,
//!   `intel_search_remote_page`
//! - **Match**: `intel_match_targets`, `intel_search_github_poc`
//! - **Nuclei templates**: `intel_search_nuclei_templates`,
//!   `intel_batch_search_nuclei_templates`, `intel_discover_all_nuclei`

// Extracted to the `golish-vuln-app` crate (crate-per-service split M1).
// The glob re-exports both the command fns and their `__cmd__$name` macros so
// the aggregate `generate_handler!` in `commands_registry.rs` resolves them.
pub use golish_vuln_app::vuln_intel::*;
