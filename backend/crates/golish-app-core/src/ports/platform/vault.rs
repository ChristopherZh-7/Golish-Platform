//! `VaultReadPort` — platform's credential vault reads as a service port.
//!
//! The in-proc adapter mirrors `golish_db::repo::vault` exactly (same SQL,
//! same project-scope / IDOR semantics). Encryption stays in the caller; the
//! port only moves already-obfuscated values, identical to today's behaviour.
//! Writes (the `credential_vault` `store` action) keep their raw INSERT for now
//! (it relies on `ON CONFLICT DO NOTHING`); draining that is P0-3 scoped-SQL
//! work, not S1-2. See `docs/design/2026-05-30-s1-2-port-horizontal-coupling.md` §5.

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;

/// Outbound port for reading the platform credential vault. Remote-ready: only
/// serializable params/returns — no pool/closures leak across the boundary.
#[async_trait]
pub trait VaultReadPort: Send + Sync {
    /// `(name, entry_type, username, notes)` for a project, alphabetical.
    async fn list_name_meta_by_project(
        &self,
        project_path: &str,
    ) -> anyhow::Result<Vec<(String, String, String, String)>>;

    /// `(enc_value, username, entry_type)` for the first entry matching `name`.
    async fn get_secret_by_name_project(
        &self,
        name: &str,
        project_path: &str,
    ) -> anyhow::Result<Option<(String, String, String)>>;
}

/// In-proc adapter backed by the embedded Postgres pool. The ONLY place in the
/// pentest service allowed to call `golish_db::repo::vault` — it lives under the
/// platform port domain, so the ownership guard treats it as platform-owned.
pub struct PgVaultAdapter {
    pool: Arc<PgPool>,
}

impl PgVaultAdapter {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl VaultReadPort for PgVaultAdapter {
    async fn list_name_meta_by_project(
        &self,
        project_path: &str,
    ) -> anyhow::Result<Vec<(String, String, String, String)>> {
        Ok(
            golish_db::repo::vault::list_name_meta_by_project(self.pool.as_ref(), project_path)
                .await?,
        )
    }

    async fn get_secret_by_name_project(
        &self,
        name: &str,
        project_path: &str,
    ) -> anyhow::Result<Option<(String, String, String)>> {
        Ok(golish_db::repo::vault::get_secret_by_name_project(
            self.pool.as_ref(),
            name,
            project_path,
        )
        .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Compile-time guarantee the port stays object-safe (consumers store
    // `Arc<dyn VaultReadPort>`).
    #[test]
    fn vault_read_port_is_object_safe() {
        fn _assert(_: &dyn VaultReadPort) {}
    }
}
