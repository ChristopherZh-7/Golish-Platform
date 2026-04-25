//! Tauri commands for the simplified sidecar system.
//!
//! Provides interface between frontend and sidecar session/patch/artifact management.
//!
//! Consumers continue to use `crate::sidecar::commands::*` (a glob in `lib.rs`);
//! all 32 `sidecar_*` functions are re-exported flat from this module.
//!
//! ## Submodules
//!
//! - [`status`]    — sidecar status / initialize / shutdown
//! - [`lifecycle`] — session start / end / current / resume
//! - [`content`]   — read state.md, log.md, metadata, session listings
//! - [`config`]    — read/write `SidecarConfig`
//! - [`patches`]   — L2 staged-patch operations (list/get/discard/apply/regenerate)
//! - [`artifacts`] — L3 project-artifact operations (list/get/preview/apply/regenerate)

mod artifacts;
mod config;
mod content;
mod lifecycle;
mod patches;
mod status;

pub use artifacts::*;
pub use config::*;
pub use content::*;
pub use lifecycle::*;
pub use patches::*;
pub use status::*;
