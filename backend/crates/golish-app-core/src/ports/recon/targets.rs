//! `ReconTargetsPort` — recon `targets` lookups as a service port.
//!
//! The in-proc adapter mirrors `golish_db::repo::targets` exactly (same SQL /
//! IDOR project-scope semantics). It is the ONLY place the consuming pentest /
//! vuln services reach the recon `targets` repo; it lives under the recon port
//! domain so the ownership guard treats it as recon-owned. Remote-ready: only
//! serializable params/returns (`Uuid` / `bool` / `(String, Value)`), no pool
//! leaks across the boundary.

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

/// Outbound port for recon target lookups (read-only).
#[async_trait]
pub trait ReconTargetsPort: Send + Sync {
    async fn targets_find_id_by_value_pair(
        &self,
        value_a: &str,
        value_b: &str,
        project_path: &str,
    ) -> anyhow::Result<Option<Uuid>>;

    async fn targets_find_id_by_value_or_name(
        &self,
        value_or_name: &str,
        project_path: &str,
    ) -> anyhow::Result<Option<Uuid>>;

    async fn targets_exists_by_value_exact(
        &self,
        value: &str,
        project_path: Option<&str>,
    ) -> anyhow::Result<bool>;

    async fn targets_match_rows_legacy(
        &self,
        project_path: Option<&str>,
    ) -> anyhow::Result<Vec<(String, serde_json::Value)>>;
}

/// In-proc adapter backed by the embedded Postgres pool.
pub struct PgReconTargetsAdapter {
    pool: Arc<PgPool>,
}

impl PgReconTargetsAdapter {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ReconTargetsPort for PgReconTargetsAdapter {
    async fn targets_find_id_by_value_pair(
        &self,
        value_a: &str,
        value_b: &str,
        project_path: &str,
    ) -> anyhow::Result<Option<Uuid>> {
        Ok(golish_db::repo::targets::find_id_by_value_pair(
            self.pool.as_ref(),
            value_a,
            value_b,
            project_path,
        )
        .await?)
    }

    async fn targets_find_id_by_value_or_name(
        &self,
        value_or_name: &str,
        project_path: &str,
    ) -> anyhow::Result<Option<Uuid>> {
        Ok(golish_db::repo::targets::find_id_by_value_or_name(
            self.pool.as_ref(),
            value_or_name,
            project_path,
        )
        .await?)
    }

    async fn targets_exists_by_value_exact(
        &self,
        value: &str,
        project_path: Option<&str>,
    ) -> anyhow::Result<bool> {
        Ok(
            golish_db::repo::targets::exists_by_value_exact(
                self.pool.as_ref(),
                value,
                project_path,
            )
            .await?,
        )
    }

    async fn targets_match_rows_legacy(
        &self,
        project_path: Option<&str>,
    ) -> anyhow::Result<Vec<(String, serde_json::Value)>> {
        Ok(golish_db::repo::targets::match_rows_legacy(self.pool.as_ref(), project_path).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recon_targets_port_is_object_safe() {
        fn _assert(_: &dyn ReconTargetsPort) {}
    }
}
