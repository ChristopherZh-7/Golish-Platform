use anyhow::{Context, Result};
use sqlx::postgres::{PgPool, PgPoolOptions};
use tracing::info;

/// Create a connection pool and run migrations.
pub async fn create_pool(connection_string: &str) -> Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .min_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(connection_string)
        .await
        .context("Failed to connect to PostgreSQL")?;

    info!("Running database migrations");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("Failed to run database migrations")?;

    info!("Database migrations complete");
    Ok(pool)
}
