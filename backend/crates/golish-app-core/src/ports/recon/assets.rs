//! `ReconAssetsPort` — recon `target_assets` reads as a service port.
//!
//! The in-proc adapter mirrors `golish_db::repo::target_assets` exactly. It is
//! the ONLY place the consuming services reach the recon `target_assets` repo;
//! it lives under the recon port domain so the ownership guard treats it as
//! recon-owned. Remote-ready: `TargetAsset` derives Serde, no pool leaks.

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use golish_db::models::TargetAsset;

/// Outbound port for reading recon target assets.
#[async_trait]
pub trait ReconAssetsPort: Send + Sync {
    async fn target_assets_list_by_target(
        &self,
        target_id: Uuid,
    ) -> anyhow::Result<Vec<TargetAsset>>;

    async fn target_assets_count_by_target(&self, target_id: Uuid) -> anyhow::Result<i64>;
}

/// In-proc adapter backed by the embedded Postgres pool.
pub struct PgReconAssetsAdapter {
    pool: Arc<PgPool>,
}

impl PgReconAssetsAdapter {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ReconAssetsPort for PgReconAssetsAdapter {
    async fn target_assets_list_by_target(
        &self,
        target_id: Uuid,
    ) -> anyhow::Result<Vec<TargetAsset>> {
        Ok(
            golish_db::repo::target_assets::list_by_current_target_owner(
                self.pool.as_ref(),
                target_id,
            )
            .await?,
        )
    }

    async fn target_assets_count_by_target(&self, target_id: Uuid) -> anyhow::Result<i64> {
        Ok(
            golish_db::repo::target_assets::count_by_current_target_owner(
                self.pool.as_ref(),
                target_id,
            )
            .await?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recon_assets_port_is_object_safe() {
        fn _assert(_: &dyn ReconAssetsPort) {}
    }
}
