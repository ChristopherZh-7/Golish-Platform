//! API authorization probe — Stage 2 of the API security pipeline.
//!
//! Consumes the structured `Endpoint` rows produced by
//! [`golish_js_analyzer`] and runs 3 deterministic HTTP rounds per
//! endpoint to detect:
//!
//! - **Anonymous access** — endpoint serves data without auth (Critical)
//! - **Cross-user IDOR** — user A's token reads user B's resource (High)
//! - **Privilege escalation** — low-priv token reaches admin endpoint (High)
//!
//! See `docs/auth-probe-contract.md` for the full spec.
//!
//! ## Scope (P0 scaffold)
//!
//! This first commit defines the public types (`Scenario`, `Round`,
//! `Verdict`, `Finding`, `ProbeConfig`, `ProbeReport`). The actual HTTP
//! orchestrator (`probe()`) is implemented in the next commit so this
//! crate compiles cleanly without committing untested HTTP logic.

#![forbid(unsafe_code)]
#![deny(warnings)]
#![allow(clippy::result_large_err)]

mod compare;
mod orchestrator;
mod request;
mod substitute;
mod types;

pub use compare::compare_rounds;
pub use orchestrator::probe;
pub use substitute::{substitute_id, SubstituteKind};
pub use types::{
    Evidence, Finding, ProbeConfig, ProbeReport, ProbeSummary, Round, RoundOutcome, Scenario,
    Severity, TokenSource, Verdict,
};

// Re-export Endpoint et al. so callers don't have to depend on the
// analyzer crate directly.
pub use golish_js_analyzer::{AuthHint, CallSiteKind, Endpoint, UrlKind};
