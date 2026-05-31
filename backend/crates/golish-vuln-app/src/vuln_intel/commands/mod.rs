//! Tauri command wrappers around `golish_vuln_intel::*`.
//!
//! Each command extracts the DB pool from the narrow `golish_app_core::DbState`
//! (and settings from `golish_settings`'s `SettingsManager`), builds the
//! necessary HTTP client, and delegates to the library crate. Split here
//! by concern:
//!
//! - [`feeds`]      — `vuln_feeds` table CRUD (list / add / toggle / delete).
//! - [`fetching`]   — NVD/CISA/RSS feed ingestion + cached entries.
//! - [`search`]     — local + remote (NVD) keyword/CVE search.
//! - [`matching`]   — match cached CVEs against user-defined targets.
//! - [`enrichment`] — per-CVE GitHub PoC + Nuclei template lookups.
//! - [`shared`]     — common helpers (GitHub client builder).

mod enrichment;
mod feeds;
mod fetching;
mod matching;
mod search;
mod shared;

// Re-export both the function AND the matching `__cmd__$name` macro generated
// by `#[tauri::command]`. golish's aggregate `generate_handler!` resolves each
// command's `__cmd__$name` macro through the `commands_facade::vuln_intel`
// glob, which only reaches it if the macro is re-exported alongside the fn.
// (Same rationale as the `wiki` module — see its mod.rs.)
pub use enrichment::{
    __cmd__intel_batch_search_nuclei_templates, __cmd__intel_discover_all_nuclei,
    __cmd__intel_search_github_poc, __cmd__intel_search_nuclei_templates,
    intel_batch_search_nuclei_templates, intel_discover_all_nuclei, intel_search_github_poc,
    intel_search_nuclei_templates,
};
pub use feeds::{
    __cmd__intel_add_feed, __cmd__intel_delete_feed, __cmd__intel_list_feeds,
    __cmd__intel_toggle_feed, intel_add_feed, intel_delete_feed, intel_list_feeds,
    intel_toggle_feed,
};
pub use fetching::{
    __cmd__intel_fetch, __cmd__intel_fetch_page, __cmd__intel_get_cached, intel_fetch,
    intel_fetch_page, intel_get_cached,
};
pub use matching::{__cmd__intel_match_targets, intel_match_targets};
pub use search::{
    __cmd__intel_search, __cmd__intel_search_remote, __cmd__intel_search_remote_page, intel_search,
    intel_search_remote, intel_search_remote_page,
};
