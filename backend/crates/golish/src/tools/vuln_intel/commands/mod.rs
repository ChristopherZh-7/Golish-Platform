//! Tauri command wrappers around `golish_vuln_intel::*`.
//!
//! Each command extracts the DB pool and settings from `AppState`, builds
//! the necessary HTTP client, and delegates to the library crate. Split here
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

pub use enrichment::{
    intel_batch_search_nuclei_templates, intel_discover_all_nuclei, intel_search_github_poc,
    intel_search_nuclei_templates,
};
pub use feeds::{intel_add_feed, intel_delete_feed, intel_list_feeds, intel_toggle_feed};
pub use fetching::{intel_fetch, intel_fetch_page, intel_get_cached};
pub use matching::intel_match_targets;
pub use search::{intel_search, intel_search_remote, intel_search_remote_page};
