//! Per-domain Tauri command crate for the **vuln-intel** service.
//!
//! Holds the `#[tauri::command]` wrappers for vulnerability intelligence:
//! feed CRUD, NVD/CISA/RSS ingestion, local + remote search, target matching,
//! and per-CVE GitHub PoC + Nuclei enrichment. Extracted from
//! `golish/src/tools/vuln_intel/` as the first leaf of the crate-per-service
//! split (this service is the DAG leaf — out-degree 0).
//!
//! ## Boundary
//!
//! Commands take the narrow [`golish_app_core::DbState`] (+ `golish_settings`'s
//! `SettingsManager`), never the monolithic `golish::AppState`, so this crate
//! sits below the main `golish` binary. The `golish` aggregate
//! `generate_handler!` reaches these commands via the
//! `commands_facade::vuln_intel` re-export, which globs `vuln_intel::*` —
//! including the `__cmd__$name` macros re-exported below.

pub mod vuln_intel;
pub mod wiki;
