//! Database adapters (Postgres-specific implementations of domain traits).
//!
//! Currently re-exports adapters that live alongside the domain traits in
//! their respective Layer-3 crates so the rest of the main crate has a
//! single import path:
//!
//! ```rust,ignore
//! use crate::db::PgPentestStore;
//! ```
//!
//! When more domain crates migrate to the trait + adapter pattern (P2-4),
//! their `Pg*Store` re-exports go here.

pub use golish_pentest::output_store::PgPentestStore;
