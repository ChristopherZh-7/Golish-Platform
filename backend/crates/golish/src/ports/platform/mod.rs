//! Platform service ports (vault / notes / terminal logs).

pub mod vault;

pub use vault::{PgVaultAdapter, VaultReadPort};
