//! Managed database state.
//!
//! `DbState` moved to the `golish-app-core` crate (L5) so per-domain app crates
//! can receive it via `tauri::State<'_, DbState>` without depending on this
//! monolithic application crate. It is re-exported here unchanged so existing
//! `crate::state::{db::DbState, DbState}` paths keep working.

pub use golish_app_core::DbState;
