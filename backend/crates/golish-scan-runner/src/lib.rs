#![allow(clippy::too_many_arguments)]

//! Scan-runner engine for Golish.
//!
//! Dispatch table for the various pentest scanners invoked from the GUI/AI:
//! WhatWeb fingerprinting, Nuclei targeted scanning with fingerprint→PoC
//! matching, and feroxbuster directory busting.
//!
//! This crate has **no** Tauri dependency — progress events are emitted
//! through [`golish_core::EventEmitterHandle`], and the frontend shell
//! provides the `TauriEventEmitter` adapter.
//!
//! ## Layout
//! - [`types`]       — small DTOs (`ScanProgress`, `ScanResult`, `PocMatch`).
//! - [`helpers`]     — shared progress emission, audit logging, command lookup.
//! - [`whatweb`]     — WhatWeb fingerprinting.
//! - [`nuclei`]      — Nuclei targeted scan + fingerprint→PoC matching engine.
//! - [`feroxbuster`] — directory busting over seed paths supplied by callers.

pub mod error;
pub mod feroxbuster;
pub mod helpers;
pub mod nuclei;
pub mod storage;
pub mod types;
pub mod whatweb;

pub use error::{ScanRunnerError, ScanRunnerResult};
pub use feroxbuster::{run_feroxbuster, FeroxScanOptions};
pub use helpers::NUCLEI_CANCELLED;
pub use nuclei::{match_pocs_for_target, run_nuclei_targeted, NucleiScanOptions};
pub use storage::ScanStorage;
pub use types::{PocMatch, ScanProgress, ScanResult};
pub use whatweb::{run_whatweb, WhatWebOptions};
