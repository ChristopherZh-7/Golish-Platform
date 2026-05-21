//! Storage backend implementations.
//!
//! The trait [`crate::traits::StorageBackend`] is implemented by:
//!
//! - [`external_file::ExternalFileBackend`] (Phase 2) — renders fields
//!   into YAML / JSON merged onto an existing file
//!   (e.g. `~/.config/enscan/config.yaml`), with atomic write +
//!   rolling backups.
//! - `vault::VaultBackend` (Phase 2, separate module) — writes to the
//!   `vault_entries` table, one row per field, aggregated by
//!   `tags=["integration-group", <tool>, <group>]`. Also reads old
//!   `tags=["intel-provider", X]` rows as a backward-compatibility
//!   alias.
//! - `settings::SettingsBackend` (Phase 2, separate module) — writes
//!   through `SettingsManager` at the dotted path specified in
//!   [`crate::schema::SettingsStorage::key`].

pub mod external_file;
pub mod settings;
pub mod vault;

pub use external_file::ExternalFileBackend;
pub use settings::SettingsBackend;
pub use vault::VaultBackend;
