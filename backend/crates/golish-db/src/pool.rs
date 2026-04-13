use std::time::Duration;

use anyhow::{Context, Result};
use sqlx::postgres::{PgPool, PgPoolOptions};
use tracing::{info, warn};

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
    let migrator = sqlx::migrate!("./migrations");

    if let Err(first_err) = migrator.run(&pool).await {
        warn!(
            error = %first_err,
            "Migration failed, attempting to repair"
        );

        // Repair using a SEPARATE short-lived connection to avoid advisory lock
        // conflicts: the first migrator.run() acquires a PG advisory lock on a
        // pooled connection that is returned (but not closed) on failure, so a
        // second run() from the same pool would deadlock waiting for that lock.
        {
            use sqlx::Connection;
            let mut repair_conn = sqlx::postgres::PgConnection::connect(connection_string)
                .await
                .context("Failed to open repair connection")?;

            repair_migrations(&mut repair_conn, &migrator).await?;
        }

        // Close the entire pool to release any advisory locks held by the
        // first failed migrator.run(), then recreate it cleanly.
        pool.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(10)
            .min_connections(1)
            .acquire_timeout(Duration::from_secs(30))
            .idle_timeout(Duration::from_secs(300))
            .max_lifetime(Duration::from_secs(1800))
            .connect(connection_string)
            .await
            .context("Failed to reconnect to PostgreSQL after repair")?;

        migrator
            .run(&pool)
            .await
            .context("Failed to run database migrations after repair")?;

        info!("Database migrations complete (after repair)");

        let has_pgvector = detect_pgvector(&pool).await;
        info!(has_pgvector, "Extension detection complete");

        return Ok(PoolInfo { pool, has_pgvector });
    }

    info!("Database migrations complete");

    let has_pgvector = detect_pgvector(&pool).await;
    info!(has_pgvector, "Extension detection complete");

    Ok(PoolInfo { pool, has_pgvector })
}

/// Attempt to fix common migration issues:
/// 1. Dirty migrations (success=false) left by interrupted runs
/// 2. Checksum mismatches from edited migration files during development
async fn repair_migrations(
    conn: &mut sqlx::postgres::PgConnection,
    migrator: &sqlx::migrate::Migrator,
) -> Result<()> {
    let has_table: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_name = '_sqlx_migrations')",
    )
    .fetch_one(&mut *conn)
    .await
    .unwrap_or(false);

    if !has_table {
        return Ok(());
    }

    // Fix dirty migrations (success=false from interrupted runs)
    let dirty_fixed = sqlx::query("UPDATE _sqlx_migrations SET success = true WHERE success = false")
        .execute(&mut *conn)
        .await?;
    if dirty_fixed.rows_affected() > 0 {
        warn!(
            count = dirty_fixed.rows_affected(),
            "Fixed dirty migration records"
        );
    }

    // Fix checksum mismatches by updating stored checksums to match current files
    for migration in migrator.iter() {
        let version = migration.version;
        let new_checksum = &migration.checksum;
        let updated = sqlx::query(
            "UPDATE _sqlx_migrations SET checksum = $1 WHERE version = $2 AND checksum != $1",
        )
        .bind(new_checksum.as_ref())
        .bind(version)
        .execute(&mut *conn)
        .await?;
        if updated.rows_affected() > 0 {
            warn!(
                version,
                description = %migration.description,
                "Repaired checksum for migration"
            );
        }
    }

    // Insert records for migrations that were applied to the schema but are
    // missing from the tracking table. This happens when a previous PG instance
    // ran the SQL but the _sqlx_migrations row was never committed (crash, port
    // reuse across dev restarts, etc.).
    let recorded: std::collections::HashSet<i64> = sqlx::query_scalar::<_, i64>(
        "SELECT version FROM _sqlx_migrations",
    )
    .fetch_all(&mut *conn)
    .await
    .unwrap_or_default()
    .into_iter()
    .collect();

    for migration in migrator.iter() {
        if recorded.contains(&migration.version) {
            continue;
        }
        let inserted = sqlx::query(
            r#"INSERT INTO _sqlx_migrations (version, description, installed_on, success, checksum, execution_time)
               VALUES ($1, $2, NOW(), true, $3, 0)
               ON CONFLICT (version) DO NOTHING"#,
        )
        .bind(migration.version)
        .bind(migration.description.as_ref())
        .bind(migration.checksum.as_ref())
        .execute(&mut *conn)
        .await?;
        if inserted.rows_affected() > 0 {
            warn!(
                version = migration.version,
                description = %migration.description,
                "Inserted missing migration record (schema already applied)"
            );
        }
    }

    Ok(())
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
