#![allow(clippy::too_many_arguments)]

//! Scan-runner engine for Golish.
//!
//! Dispatch table for guarded reconnaissance scanners plus the read-only
//! fingerprint -> Nuclei template selector used by stage-owned adapters.
//!
//! This crate has **no** Tauri dependency — progress events are emitted
//! through [`golish_core::EventEmitterHandle`], and the frontend shell
//! provides the `TauriEventEmitter` adapter.
//!
//! ## Layout
//! - [`types`]       — small DTOs and Nuclei template selection rationale.
//! - [`helpers`]     — shared progress emission, audit logging, command lookup.
//! - [`whatweb`]     — WhatWeb fingerprinting.
//! - [`nuclei`]      — read-only fingerprint→safe-template selector.
//! - [`feroxbuster`] — directory busting over seed paths supplied by callers.

pub mod authorization;
pub mod error;
pub mod feroxbuster;
pub mod helpers;
pub mod nuclei;
pub mod storage;
pub mod types;
pub mod whatweb;

pub use authorization::{authorize_scan_target, AuthorizedScanTarget};
pub use error::{ScanRunnerError, ScanRunnerResult};
pub use feroxbuster::{run_feroxbuster, FeroxScanOptions};
pub use nuclei::select_nuclei_templates_for_origin;
pub use storage::ScanStorage;
pub use types::{NucleiTemplateRationale, NucleiTemplateSelection, ScanProgress, ScanResult};
pub use whatweb::{run_whatweb, WhatWebOptions};
