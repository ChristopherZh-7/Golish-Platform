//! Scan-runner subsystem: dispatch table for the various pentest scanners
//! invoked from the GUI/AI.
//!
//! - [`whatweb`]: WhatWeb fingerprinting.
//! - [`nuclei`]: Nuclei targeted scan + fingerprint → PoC matching engine.
//! - [`feroxbuster`]: directory busting over ZAP-discovered paths.
//! - [`helpers`]: shared progress emission, audit logging, command lookup.
//! - [`types`]: small DTOs (`ScanProgress`, `ScanResult`, `PocMatch`).

mod feroxbuster;
mod helpers;
mod nuclei;
mod types;
mod whatweb;

pub use feroxbuster::{get_zap_discovered_paths, scan_feroxbuster, FeroxScanOptions};
pub use nuclei::{match_pocs_for_target, scan_nuclei_targeted, NucleiScanOptions};
pub use types::{PocMatch, ScanProgress, ScanResult};
pub use whatweb::{scan_whatweb, WhatWebOptions};

// Re-export Tauri command macro items so `tauri::generate_handler!` in lib.rs
// resolves `scan_runner::__cmd__X` correctly even though `X` lives in a
// submodule.
#[doc(hidden)]
pub use feroxbuster::{__cmd__get_zap_discovered_paths, __cmd__scan_feroxbuster};
#[doc(hidden)]
pub use nuclei::{__cmd__match_pocs_for_target, __cmd__scan_nuclei_targeted};
#[doc(hidden)]
pub use whatweb::__cmd__scan_whatweb;

use std::sync::atomic::Ordering;

#[tauri::command]
pub async fn nuclei_cancel() -> Result<(), String> {
    helpers::NUCLEI_CANCELLED.store(true, Ordering::SeqCst);
    Ok(())
}
