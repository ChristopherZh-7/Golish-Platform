//! Per-domain Tauri command crate for the **recon** service.
//!
//! Holds the `#[tauri::command]` wrappers for the reconnaissance / attack-surface
//! domain, extracted from `golish/src/tools/` as the second service of the
//! crate-per-service split (M2). Recon owns the targets / asset-surface tables
//! (`targets`, `target_assets`, `organizations`, `directory_entries`,
//! `custom_rules`, `sensitive_scan`, …).
//!
//! ## Boundary
//!
//! Commands take the narrow [`golish_app_core::DbState`] (+ domain crates like
//! `golish_pentest`, `golish_scan_runner`, `golish_intel_providers`), never the
//! monolithic `golish::AppState`, so this crate sits below the main `golish`
//! binary. The `golish` aggregate `generate_handler!` reaches these commands via
//! the `commands_facade` re-exports, which glob each module — including the
//! `__cmd__$name` macros each `#[tauri::command]` emits.

pub mod agent_tools;
pub mod asset_intel;
pub mod custom_rules;
pub mod integrations;
pub mod intel_providers;
pub mod organization_recon;
pub mod organizations;
pub mod scan_queue;
pub mod scan_runner;
pub mod sensitive_scan;
pub mod targets;
pub mod wordlists;
