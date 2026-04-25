//! Session persistence module for Golish AI conversations.
//!
//! Provides session archiving, conversation logs, and transcript export by
//! integrating with `golish_core::session`. Supports dual persistence:
//! file-based (via `golish-core`) and PostgreSQL (via `golish-db`).
//!
//! # Submodules
//!
//! - [`types`]: public DTOs ([`GolishMessageRole`], [`GolishSessionMessage`],
//!   [`GolishSessionSnapshot`], [`SessionListingInfo`]).
//! - [`manager`]: [`GolishSessionManager`] — in-memory active session plus
//!   dual-write to disk + optional Postgres handle.
//! - [`archive`]: read-side helpers ([`list_recent_sessions`],
//!   [`find_session`], [`load_session`]) plus sidecar metadata extraction
//!   for the session picker.
//! - [`db`]: existing PostgreSQL persistence companion module.

pub mod db;

mod archive;
mod manager;
mod types;

#[cfg(test)]
mod tests;

pub use archive::{find_session, list_recent_sessions, load_session};
pub use manager::GolishSessionManager;
pub use types::{
    GolishMessageRole, GolishSessionMessage, GolishSessionSnapshot, SessionListingInfo,
};
