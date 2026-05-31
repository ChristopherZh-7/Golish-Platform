//! Database adapters (Postgres-specific implementations of domain traits).
//!
//! Placeholder for `Pg*Store` adapter re-exports. The former
//! `PgPentestStore` re-export moved to `golish-pentest-app` together with the
//! pentest command surface (crate-per-service M3/M4-proper), so this module is
//! currently empty. When more domain crates that *stay* in `golish` adopt the
//! trait + adapter pattern, their `Pg*Store` re-exports go here.
