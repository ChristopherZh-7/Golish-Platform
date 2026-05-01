//! Vulnerability intelligence engine for Golish.
//!
//! This crate owns all vuln-feed ingestion (NVD, CISA KEV, RSS), GitHub PoC
//! search, and Nuclei template discovery/import. It has **no** Tauri
//! dependency — the application layer wires it up through thin command
//! wrappers, building `reqwest::Client` instances from settings and passing
//! `&PgPool` directly.
//!
//! ## Layout
//! - [`types`]           — public DTOs (`VulnFeed`, `VulnEntry`) + DB row helpers.
//! - [`fetch`]           — feed fetching (CISA KEV, NVD, RSS) + merge/enrich.
//! - [`github_client`]   — shared GitHub API client builder + header factory.
//! - [`github_poc`]      — GitHub PoC repository search.
//! - [`nuclei_search`]   — Nuclei template search (single + batch).
//! - [`nuclei_discover`] — bulk Nuclei template import via Git Trees/Contents API.

pub use golish_vuln_intel_domain::traits;

pub mod error;
pub mod fetch;
pub mod github_client;
pub mod github_poc;
pub mod nuclei_discover;
pub mod nuclei_search;
pub mod types;

pub use error::{VulnIntelError, VulnIntelResult};
pub use fetch::{enrich_missing_cvss, fetch_cisa_kev, fetch_nvd, fetch_rss, merge_and_enrich};
pub use github_client::{build_github_client, github_headers};
pub use github_poc::{search_github_poc, GithubPocResult};
pub use nuclei_discover::{discover_all_nuclei, NucleiDiscoverProgress, NucleiDiscoverResult};
pub use nuclei_search::{
    batch_search_nuclei_templates, extract_nuclei_severity, search_nuclei_templates,
    BatchNucleiResult, NucleiTemplateResult,
};
pub use types::{
    default_feeds, ensure_default_feeds, nvd_recent_url, upsert_entries, EntryRow, FeedRow,
    VulnEntry, VulnFeed,
};
