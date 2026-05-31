//! `ReconDirectoryPort` — recon `directory_entries` existence checks as a port.
//!
//! The in-proc adapter mirrors `golish_db::repo::directory_entries` exactly. It
//! is the ONLY place the consuming pentest service reaches the recon
//! `directory_entries` repo; it lives under the recon port domain so the
//! ownership guard treats it as recon-owned.

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;

/// Outbound port for recon directory-entry existence checks (read-only).
#[async_trait]
pub trait ReconDirectoryPort: Send + Sync {
    async fn directory_entries_exists_by_url_project(
        &self,
        url: &str,
        project_path: Option<&str>,
    ) -> anyhow::Result<bool>;
}

/// In-proc adapter backed by the embedded Postgres pool.
pub struct PgReconDirectoryAdapter {
    pool: Arc<PgPool>,
}

impl PgReconDirectoryAdapter {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ReconDirectoryPort for PgReconDirectoryAdapter {
    async fn directory_entries_exists_by_url_project(
        &self,
        url: &str,
        project_path: Option<&str>,
    ) -> anyhow::Result<bool> {
        Ok(golish_db::repo::directory_entries::exists_by_url_project(
            self.pool.as_ref(),
            url,
            project_path,
        )
        .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recon_directory_port_is_object_safe() {
        fn _assert(_: &dyn ReconDirectoryPort) {}
    }
}
