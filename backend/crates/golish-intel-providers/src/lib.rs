//! ASM/threat intel provider abstraction layer.
//!
//! This crate provides a uniform `IntelProvider` trait for integrating
//! external ASM (Attack Surface Management) and threat intelligence
//! platforms (0.zone, FOFA, 360 Quake, Hunter, Shodan, ...).
//!
//! ## Architecture
//!
//! - [`IntelProvider`] · async trait that every platform implements
//! - [`ProviderRecord`] · uniform result format (provider-agnostic field map)
//! - [`ProviderMeta`] · static metadata (id / display name / signup url)
//! - [`QueryType`] · enum of query categories (site / domain / email / ...)
//! - [`error`] · `IntelError` + `IntelResult<T>`
//!
//! ## Adding a new provider
//!
//! 1. Create `src/<name>/{mod,client,types,mapper}.rs`
//! 2. Implement `IntelProvider` in `<name>/mod.rs`
//! 3. Add `pub mod <name>;` here
//! 4. Register the provider in the consumer crate (Tauri facade)
//!
//! See `docs/design/2026-05-20-asm-intel-providers.md` for the full design.

pub mod error;
pub mod shared;
pub mod types;

pub mod fofa;
pub mod hunter;
pub mod quake;
pub mod shodan;
pub mod zone;

pub use error::{IntelError, IntelResult};
pub use types::{ConnectionStatus, ProviderMeta, ProviderRecord, QueryType};

use async_trait::async_trait;

/// Uniform interface for any ASM / threat intel platform.
///
/// Each provider implementation is responsible for:
/// - Translating a generic [`QueryType`] + query string into the
///   platform-specific request (e.g. POST to `https://0.zone/api/data/`)
/// - Holding the API key (passed in via `key`, never persisted here)
/// - Rate-limiting requests (per-provider, owned by the impl)
/// - Mapping the raw response into a list of [`ProviderRecord`]s with
///   `fields` keys that match what `output_store::store_organization_update`
///   expects (`domain` / `cidr` / `asn` / `cert` / `email` / `github_org` /
///   `contact_name` / ...)
#[async_trait]
pub trait IntelProvider: Send + Sync {
    /// Stable identifier (used for vault lookup key, e.g. `"0.zone"`).
    fn id(&self) -> &str;

    /// Static metadata for UI rendering.
    fn meta(&self) -> ProviderMeta;

    /// Execute a query against the provider.
    ///
    /// `key` is the API key (or whatever the provider needs for auth).
    /// Implementations must NOT log the key in any tracing event.
    async fn query(
        &self,
        query_type: QueryType,
        query: &str,
        key: &str,
    ) -> IntelResult<Vec<ProviderRecord>>;

    /// Check whether the given `key` is valid by issuing a cheap request.
    /// Used by the Settings UI "Test Connection" button.
    async fn test_connection(&self, key: &str) -> IntelResult<ConnectionStatus>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intel_provider_is_object_safe() {
        // Compile-time check: the trait must be object-safe so the consumer
        // can hold `Box<dyn IntelProvider>` in a registry.
        fn _box_dyn(p: Box<dyn IntelProvider>) -> Box<dyn IntelProvider> {
            p
        }
    }
}
