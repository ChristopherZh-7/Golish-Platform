//! Narrow managed state handles for Tauri commands.
//!
//! New commands should take narrow sub-states (e.g. [`DbState`]) instead of the
//! monolithic `AppState` (which stays in the `golish` crate because it
//! aggregates golish-internal subsystems). `DbState` lives here so per-domain
//! app crates can receive it via `tauri::State<'_, DbState>` without depending
//! on `golish`.

use crate::error::GolishError;
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
    pub async fn pool_ready(&self) -> Result<&PgPool, GolishError> {
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

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Return a cheap Arc clone of the underlying pool (zero-cost; matches
    /// the `AppState::db_pool.clone()` pattern used before A4).
    pub fn pool_arc(&self) -> Arc<PgPool> {
        self.pool.clone()
    }

    pub fn ready_gate(&self) -> &DbReadyGate {
        &self.ready
    }
}
