//! `ReconSitemapPort` — recon `sitemap_store` (ZAP sitemap) as a service port.
//!
//! The in-proc adapter mirrors `golish_db::repo::sitemap_store` exactly. It is
//! the ONLY place the consuming pentest service reaches the recon `sitemap_store`
//! repo; it lives under the recon port domain so the ownership guard treats it
//! as recon-owned. Remote-ready: `serde_json::Value` / `u64` returns, no pool
//! leaks across the boundary.

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;

/// Outbound port for the recon ZAP sitemap store (read + delete).
#[async_trait]
pub trait ReconSitemapPort: Send + Sync {
    async fn sitemap_read_zap_sitemap(
        &self,
        project_path: Option<&str>,
    ) -> anyhow::Result<Option<serde_json::Value>>;

    async fn sitemap_delete_zap_sitemap(&self, project_path: Option<&str>) -> anyhow::Result<u64>;
}

/// In-proc adapter backed by the embedded Postgres pool.
pub struct PgReconSitemapAdapter {
    pool: Arc<PgPool>,
}

impl PgReconSitemapAdapter {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ReconSitemapPort for PgReconSitemapAdapter {
    async fn sitemap_read_zap_sitemap(
        &self,
        project_path: Option<&str>,
    ) -> anyhow::Result<Option<serde_json::Value>> {
        Ok(
            golish_db::repo::sitemap_store::read_zap_sitemap(self.pool.as_ref(), project_path)
                .await?,
        )
    }

    async fn sitemap_delete_zap_sitemap(&self, project_path: Option<&str>) -> anyhow::Result<u64> {
        Ok(
            golish_db::repo::sitemap_store::delete_zap_sitemap(self.pool.as_ref(), project_path)
                .await?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recon_sitemap_port_is_object_safe() {
        fn _assert(_: &dyn ReconSitemapPort) {}
    }
}
