//! Vulnerability intelligence domain layer — pure types and I/O boundary traits.
//!
//! This crate has **no** I/O dependencies (`reqwest`, `sqlx`, etc.).
//! Feed fetching and database storage are abstracted behind traits.

pub mod traits;
pub mod types;

pub use types::{default_feeds, nvd_recent_url, VulnEntry, VulnFeed};
