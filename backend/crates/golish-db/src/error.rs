//! Typed error for golish-db's persistence API.
//!
//! Repo functions return [`Result`] (= `Result<T, DbError>`) so callers can
//! distinguish database failures — e.g. a typed [`DbError::NotFound`] or the
//! underlying [`sqlx::Error`] — instead of pattern-matching on a type-erased
//! `anyhow::Error`. This addresses architecture audit item P2-2
//! ("golish-db 无本地 Error enum / #[from] 不全").
//!
//! The [`DbError::Other`] variant carries an `anyhow::Error`, so existing call
//! sites that attach context with `anyhow`'s `.context(...)` keep compiling: the
//! resulting `anyhow::Error` converts into `DbError::Other` through the `?`
//! operator. Bare `?` on a `sqlx::Error` / `serde_json::Error` / `std::io::Error`
//! lands in the corresponding typed variant.

use thiserror::Error;

/// Result alias for golish-db operations. Defaults to [`DbError`] but keeps the
/// error type parameter so callers can still name a concrete error when needed.
pub type Result<T, E = DbError> = std::result::Result<T, E>;

/// The typed error returned by golish-db's persistence layer.
#[derive(Debug, Error)]
pub enum DbError {
    /// Underlying SQL / connection / row-decoding failure.
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),

    /// JSON (de)serialization failure on a JSONB column payload.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// Filesystem / IO failure (embedded PG data dir, file-backed ops, ...).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// A required row / record was not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// Any other failure, typically carrying context added upstream via
    /// `anyhow`'s `.context(...)`.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
