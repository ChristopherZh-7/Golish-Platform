#![allow(dead_code)] // Commit staging system implemented but not yet integrated
//! L2: Staged Commits using Git Format-Patch
//!
//! Stores commits as standard git patch files that can be applied with `git am`.
//!
//! ## File Format
//!
//! Each patch is a standard git format-patch file:
//!
//! ```patch
//! From 0000000000000000000000000000000000000000 Mon Sep 17 00:00:00 2001
//! From: Golish Agent <agent@golish.dev>
//! Date: Tue, 10 Dec 2025 14:30:00 +0000
//! Subject: [PATCH] feat(auth): add JWT authentication module
//!
//! Implements token generation and validation with configurable expiry.
//! ---
//!  src/auth.rs | 25 +++++++++++++++++++++++++
//!  src/lib.rs  |  1 +
//!  2 files changed, 26 insertions(+)
//!  create mode 100644 src/auth.rs
//!
//! diff --git a/src/auth.rs b/src/auth.rs
//! ...
//! --
//! 2.39.0
//! ```
//!
//! We also store a small metadata sidecar file for golish-specific info.
//!
//! ## Module layout
//!
//! - [`types`]   — patch types ([`PatchMeta`], [`BoundaryReason`], [`StagedPatch`]) and slug helper
//! - [`format`]  — git format-patch text formatting + parsing helpers
//! - [`diff`]    — diff generation helpers (string-based and git-backed)
//! - [`manager`] — [`PatchManager`]: filesystem CRUD + apply via `git am`

mod diff;
mod format;
mod manager;
mod types;

pub use manager::PatchManager;
pub use types::{BoundaryReason, PatchMeta, StagedPatch};
