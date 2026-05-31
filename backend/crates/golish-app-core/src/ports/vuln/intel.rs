//! `VulnIntelPort` — vuln-intel CVE search as a service port (S1-2c).
//!
//! The in-proc adapter mirrors `golish_db::repo::vuln_intel` exactly. It is the
//! ONLY place the consuming agent service reaches the vuln `vuln_intel` repo; it
//! lives under the vuln port domain so the ownership guard treats it as
//! vuln-owned. Remote-ready: `VulnEntry` derives Serde, no pool leaks.

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;

use golish_db::models::VulnEntry;

/// Outbound port for vuln-intel CVE entry search (read-only).
#[async_trait]
pub trait VulnIntelPort: Send + Sync {
    async fn vuln_intel_search_entries(
        &self,
        query: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<VulnEntry>>;
}

/// In-proc adapter backed by the embedded Postgres pool.
pub struct PgVulnIntelAdapter {
    pool: Arc<PgPool>,
}

impl PgVulnIntelAdapter {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl VulnIntelPort for PgVulnIntelAdapter {
    async fn vuln_intel_search_entries(
        &self,
        query: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<VulnEntry>> {
        Ok(golish_db::repo::vuln_intel::search_entries(self.pool.as_ref(), query, limit).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vuln_intel_port_is_object_safe() {
        fn _assert(_: &dyn VulnIntelPort) {}
    }
}
