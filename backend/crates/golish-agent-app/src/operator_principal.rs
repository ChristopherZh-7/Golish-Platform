//! DB-backed trusted operator principal provider for local command surfaces.

use std::sync::Arc;

use async_trait::async_trait;
use golish_app_core::domain::operator::{
    OperatorChannel, TrustedOperatorPrincipal, TrustedOperatorPrincipalProvider,
};
use golish_app_core::GolishError;
use golish_db::DbReadyGate;
use sqlx::PgPool;

#[derive(Clone)]
pub struct DbTrustedOperatorPrincipalProvider {
    pool: Arc<PgPool>,
    ready: DbReadyGate,
}

impl DbTrustedOperatorPrincipalProvider {
    pub fn new(pool: Arc<PgPool>, ready: DbReadyGate) -> Self {
        Self { pool, ready }
    }

    async fn pool_ready(&self) -> Result<&PgPool, GolishError> {
        if self.ready.is_ready() {
            return Ok(&self.pool);
        }
        if self.ready.is_failed() {
            return Err(GolishError::Internal("Database failed to start".into()));
        }
        if !self
            .ready
            .wait_timeout(std::time::Duration::from_secs(15))
            .await
        {
            return Err(GolishError::Internal(
                "Database is still starting up, please retry".into(),
            ));
        }
        Ok(&self.pool)
    }
}

#[async_trait]
impl TrustedOperatorPrincipalProvider for DbTrustedOperatorPrincipalProvider {
    async fn current(
        &self,
        channel: OperatorChannel,
    ) -> Result<TrustedOperatorPrincipal, GolishError> {
        let row = golish_db::repo::operator_principals::current_local(self.pool_ready().await?)
            .await
            .map_err(|error| GolishError::Internal(error.to_string()))?;
        Ok(TrustedOperatorPrincipal::from_server_record(
            row.id, channel,
        ))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn provider_api_has_no_caller_selected_operator_id() {
        let source = include_str!("operator_principal.rs");
        assert!(source.contains("current_local(self.pool_ready().await?)"));
        assert!(source.contains("TrustedOperatorPrincipal::from_server_record(row.id, channel)"));
    }
}
