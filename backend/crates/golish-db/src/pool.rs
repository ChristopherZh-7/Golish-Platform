use std::time::Duration;

use anyhow::{Context, Result};
use sqlx::postgres::{PgPool, PgPoolOptions};
use tracing::info;

/// Result of pool creation including feature detection.
pub struct PoolInfo {
    pub pool: PgPool,
    pub has_pgvector: bool,
}

/// Create a connection pool, run migrations, and detect available extensions.
///
/// This pool is created during `GolishDb::start()` after PG is confirmed running,
/// so the connect should succeed quickly. The acquire_timeout here is mainly a
/// safety net for slow migrations or transient hiccups.
pub async fn create_pool(connection_string: &str) -> Result<PoolInfo> {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(30))
        .idle_timeout(Duration::from_secs(300))
        .max_lifetime(Duration::from_secs(1800))
        .connect(connection_string)
        .await
        .context("Failed to connect to PostgreSQL")?;

    info!("Running database migrations");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("Failed to run database migrations")?;

    info!("Database migrations complete");

    let has_pgvector = detect_pgvector(&pool).await;
    info!(has_pgvector, "Extension detection complete");

    Ok(PoolInfo { pool, has_pgvector })
}

/// Check whether the pgvector extension was successfully loaded.
async fn detect_pgvector(pool: &PgPool) -> bool {
    let row: Option<(bool,)> = sqlx::query_as(
        "SELECT EXISTS(SELECT 1 FROM pg_extension WHERE extname = 'vector')",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    row.map_or(false, |(exists,)| exists)
}
