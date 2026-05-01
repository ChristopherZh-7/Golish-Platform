//! Tauri command wrappers for vuln-intel operations.
//!
//! The pure business logic (feed ingestion, NVD/CISA/RSS fetching, GitHub PoC
//! search, Nuclei template discovery) now lives in the `golish-vuln-intel`
//! crate. This module provides thin `#[tauri::command]` wrappers that adapt
//! `AppState` to the library's API.

mod commands;

pub use golish_vuln_intel::{GithubPocResult, VulnEntry, VulnFeed};
pub use commands::*;
