//! API key storage abstraction.
//!
//! Production implementations (e.g. backed by `vault_entries`) live in the
//! consumer crate so this crate has zero DB dependency. Tests and CLI
//! tooling can use [`EnvKeyStore`].

use async_trait::async_trait;

use crate::error::IntelResult;

/// Abstraction over the place where provider API keys live.
///
/// Implementors decide where keys are read from (vault DB, env vars, file,
/// settings.json, ...) and are responsible for any masking / encryption.
///
/// **Invariant**: implementors MUST NOT log the key itself in any tracing
/// event. Provider IDs are safe to log; values are not.
#[async_trait]
pub trait KeyStore: Send + Sync {
    /// Fetch the API key for `provider_id` (e.g. `"0.zone"`).
    ///
    /// Returns `Ok(None)` when no key is configured (caller should ask the
    /// user to configure one), `Err(_)` when the read itself fails.
    async fn get_key(&self, provider_id: &str) -> IntelResult<Option<String>>;
}

/// Read keys from environment variables.
///
/// Naming convention: `INTEL_KEY_<PROVIDER_ID_UPPER>` where dots and dashes
/// in the provider id are replaced with underscores.
///
/// Examples:
/// - `"0.zone"`  → `INTEL_KEY_0_ZONE`
/// - `"fofa"`    → `INTEL_KEY_FOFA`
/// - `"360-quake"` → `INTEL_KEY_360_QUAKE`
///
/// This is primarily for tests and CLI tools. The desktop app injects a
/// vault-backed implementation from the facade layer.
#[derive(Debug, Clone, Default)]
pub struct EnvKeyStore;

impl EnvKeyStore {
    fn env_var_for(provider_id: &str) -> String {
        format!(
            "INTEL_KEY_{}",
            provider_id.to_uppercase().replace(['.', '-'], "_")
        )
    }
}

#[async_trait]
impl KeyStore for EnvKeyStore {
    async fn get_key(&self, provider_id: &str) -> IntelResult<Option<String>> {
        Ok(std::env::var(EnvKeyStore::env_var_for(provider_id)).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    fn env_var_naming() {
        assert_eq!(EnvKeyStore::env_var_for("0.zone"), "INTEL_KEY_0_ZONE");
        assert_eq!(EnvKeyStore::env_var_for("fofa"), "INTEL_KEY_FOFA");
        assert_eq!(EnvKeyStore::env_var_for("360-quake"), "INTEL_KEY_360_QUAKE");
        assert_eq!(EnvKeyStore::env_var_for("hunter"), "INTEL_KEY_HUNTER");
    }

    #[tokio::test]
    #[serial]
    async fn env_key_store_reads_env_var() {
        std::env::set_var("INTEL_KEY_0_ZONE", "test-key-xyz");
        let store = EnvKeyStore;
        let key = store.get_key("0.zone").await.unwrap();
        assert_eq!(key.as_deref(), Some("test-key-xyz"));
        std::env::remove_var("INTEL_KEY_0_ZONE");
    }

    #[tokio::test]
    #[serial]
    async fn env_key_store_returns_none_when_unset() {
        std::env::remove_var("INTEL_KEY_NONEXISTENT_PROVIDER_TEST");
        let store = EnvKeyStore;
        let key = store.get_key("nonexistent-provider-test").await.unwrap();
        assert!(key.is_none());
    }
}
