//! Nuclei targeted scan + fingerprint → PoC matching engine.
//!
//! Layout:
//! - [`runner`]    — spawns `nuclei`, parses output, persists results.
//! - [`poc_match`] — fingerprint → cached PoC matching used to seed runner
//!   templates.
//!
//! `severity_rank` is shared between the two and lives here to avoid a
//! one-function `helpers.rs`.

mod poc_match;
mod runner;

pub use poc_match::match_pocs_for_target;
pub use runner::{run_nuclei_targeted, NucleiScanOptions};

/// Numeric ordering for severity strings (higher = more severe).
pub(super) fn severity_rank(s: &str) -> u8 {
    match s.to_lowercase().as_str() {
        "critical" => 4,
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
}
