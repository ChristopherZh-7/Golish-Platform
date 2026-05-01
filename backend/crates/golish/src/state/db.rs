use std::sync::Arc;

use golish_db::DbReadyGate;
use sqlx::PgPool;

/// Managed database state: lazy connection pool + readiness gate.
///
/// Register with `app.manage(DbState::new(pool, gate))`.
/// Commands that need DB access take `db: tauri::State<'_, DbState>`.
pub struct DbState {
    pool: Arc<PgPool>,
    ready: DbReadyGate,
}

impl DbState {
    pub fn new(pool: Arc<PgPool>, ready: DbReadyGate) -> Self {
        Self { pool, ready }
    }

    /// Block (with timeout) until the embedded PG is ready, then return the pool.
    pub async fn pool_ready(&self) -> Result<&PgPool, String> {
        if self.ready.is_ready() {
            return Ok(&self.pool);
        }
        if self.ready.is_failed() {
            return Err("Database failed to start".to_string());
        }
        if !self
            .ready
            .wait_timeout(std::time::Duration::from_secs(15))
            .await
        {
            return Err("Database is still starting up, please retry".to_string());
        }
        Ok(&self.pool)
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub fn ready_gate(&self) -> &DbReadyGate {
        &self.ready
    }
}
