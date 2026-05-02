//! Database adapters (Postgres-specific implementations of domain traits).
//!
//! Separates the main `golish` crate's knowledge of Postgres from the
//! domain crates. Each domain trait that lives in `golish-*-domain` (or
//! more colloquially in the relevant Layer 3 crate) gets a `Pg*Store`
//! adapter here that forwards to either `sqlx::query` directly or to
//! freestanding functions in the domain crate.
//!
//! See `.cursor/rules/refactor-roadmap.mdc` (P2-4) for the migration
//! plan.

pub mod pg_pentest_store;

pub use pg_pentest_store::PgPentestStore;
