//! # golish-integrations
//!
//! Schema-driven external-service credential management for Golish.
//!
//! ## What this crate is for
//!
//! Golish needs to talk to many external services that each require their
//! own kind of credential:
//!
//! - **API key**: `0.zone` / FOFA / Quake / Hunter / Shodan
//! - **Cookie / Token**: ENScan_GO's 5 enterprise-intel sources
//!   (爱企查 / 天眼查 / 快查 / 风鸟 / MIIT)
//! - **Personal access token**: GitHub
//!
//! Each integration is described by a JSON / Rust [`IntegrationSchema`] that
//! says (a) what fields the user must fill, (b) where to store them
//! ([`Storage::Vault`] vs [`Storage::ExternalFile`] vs [`Storage::Settings`]),
//! and (c) how to test that the stored values actually work
//! ([`TestKind`]).
//!
//! The frontend renders a generic form per schema; the backend writes the
//! fields through a unified [`StorageBackend`] trait. New integrations are
//! a config-only change.
//!
//! ## Naming note
//!
//! We deliberately use "Integrations", not "Credentials", because the
//! `target.credentials` namespace inside `golish-pentest` is reserved for
//! credentials Golish **harvests during a penetration test** (account
//! dumps, leaked passwords, etc.). Integration credentials flow in the
//! opposite direction — they're what Golish uses to **access external
//! services**.

pub mod error;
pub mod resolver;
pub mod schema;
pub mod storage;
pub mod tester;
pub mod traits;
pub mod types;

pub use error::{IntegrationError, IntegrationResult};
pub use resolver::DefaultSchemaResolver;
pub use schema::{
    ExternalFileFormat, ExternalFileStorage, Field, FieldType, IntegrationGroup, IntegrationSchema,
    SettingsStorage, Storage, TestKind, VaultStorage,
};
pub use traits::{ResolvedIntegration, SchemaResolver, StorageBackend, Tester};
pub use types::{FieldValue, HealthStatus, IntegrationHealth};
