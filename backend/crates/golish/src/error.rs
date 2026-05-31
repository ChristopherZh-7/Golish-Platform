//! Unified application-boundary error type.
//!
//! `GolishError`, its `From` conversions, the stable `code()` mapping, the
//! `{ code, message }` `Serialize` impl, and the `Result` / `IpcError` aliases
//! now live in the `golish-app-core` crate (L5) so per-domain app crates can
//! return them without depending on this monolithic application crate. They are
//! re-exported here unchanged so existing `crate::error::*` paths keep working.
//!
//! Only conversions from `golish`-internal error types stay here, next to the
//! type they convert (the orphan rule permits `impl From<LocalType> for
//! GolishError` in this downstream crate).

// `IpcError` (alias for `GolishError`) is intentionally not re-exported here:
// nothing in `golish` references `crate::error::IpcError`, and re-exporting an
// unused alias trips `-D warnings`. Its canonical home is `golish_app_core`.
pub use golish_app_core::{GolishError, Result};

impl From<crate::history::HistoryError> for GolishError {
    fn from(err: crate::history::HistoryError) -> Self {
        GolishError::Internal(err.to_string())
    }
}
