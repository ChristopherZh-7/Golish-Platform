use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use sqlx::postgres::{PgPool, PgPoolOptions};
use tracing::{info, warn};

/// Result of pool creation including feature detection.
pub struct PoolInfo {
    pub pool: PgPool,
    pub has_pgvector: bool,
}

/// Build a **lazy** [`PgPool`] using the canonical golish-db pool config.
///
/// No TCP connection is opened here — the pool auto-connects on the first
/// query. This is the entry point for the GUI bootstrap, which has to hand a
/// pool to [`crate::AppState`] before the embedded PostgreSQL server has
/// finished starting; an eager [`create_pool`] would block the UI thread.
///
/// The connection-pool tuning (max/min connections, acquire timeout) lives
/// next to [`create_pool`] so both eager and lazy callers stay in lockstep
/// when those numbers are tweaked in the future. The lazy variant
/// intentionally keeps `min_connections = 0` so we don't try to connect
/// before the embedded server is up.
pub fn create_lazy_pool(connection_string: &str) -> Result<Arc<PgPool>> {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .min_connections(0)
        .acquire_timeout(Duration::from_secs(30))
        .connect_lazy(connection_string)
        .context("Failed to create lazy PG pool")?;
    Ok(Arc::new(pool))
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

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
struct MigrationMetadata {
    version: i64,
    description: String,
    success: bool,
    checksum: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChecksumRepair {
    version: i64,
    description: String,
    old_checksum: Vec<u8>,
    new_checksum: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
struct ChecksumRepairAllowance {
    version: i64,
    description: &'static str,
    old_checksum: &'static [u8],
    new_checksum: &'static [u8],
}

/// Checksum repair is a migration-specific emergency procedure, not a generic
/// recovery strategy. Every entry must name the exact old and new SHA-384
/// values after the corresponding schema postcondition has been audited.
///
/// The list is intentionally empty: the current persistent database has no
/// checksum drift. In particular, runtime-memory migrations are immutable once
/// successfully applied and must never be added here to hide a partial schema.
const CHECKSUM_REPAIR_ALLOWLIST: &[ChecksumRepairAllowance] = &[];

/// Plan only migration-specific checksum repairs whose exact old/new SHA-384
/// pair is explicitly allowlisted after schema postconditions were audited.
///
/// Missing rows are deliberately absent so the next SQLx run executes their
/// SQL. Failed/dirty rows and description mismatches remain hard migration
/// errors. Any unallowlisted checksum drift fails closed; matching version and
/// description alone is never permission to hide a partially applied schema.
fn plan_checksum_repairs(
    records: &[MigrationMetadata],
    migrator: &sqlx::migrate::Migrator,
) -> Result<Vec<ChecksumRepair>> {
    let mut repairs = Vec::new();
    for migration in migrator
        .iter()
        .filter(|migration| !migration.migration_type.is_down_migration())
    {
        let Some(record) = records.iter().find(|record| {
            record.version == migration.version
                && record.success
                && record.description == migration.description.as_ref()
        }) else {
            continue;
        };
        if record.checksum == migration.checksum.as_ref() {
            continue;
        }
        let Some(allowance) = CHECKSUM_REPAIR_ALLOWLIST.iter().find(|allowance| {
            allowance.version == record.version
                && allowance.description == record.description
                && allowance.old_checksum == record.checksum
                && allowance.new_checksum == migration.checksum.as_ref()
        }) else {
            anyhow::bail!(
                "migration checksum drift for version {} ({}) is not explicitly allowlisted",
                record.version,
                record.description
            );
        };
        repairs.push(ChecksumRepair {
            version: allowance.version,
            description: allowance.description.to_string(),
            old_checksum: allowance.old_checksum.to_vec(),
            new_checksum: allowance.new_checksum.to_vec(),
        });
    }
    Ok(repairs)
}

/// Repair checksum metadata for migrations SQLx has already recorded as
/// successfully applied. Never manufacture success metadata, promote dirty
/// rows, or remove unknown rows: the second `Migrator::run` must either execute
/// genuinely missing SQL or fail closed on dirty/schema-history drift.
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

    let records = sqlx::query_as::<_, MigrationMetadata>(
        "SELECT version, description, success, checksum FROM _sqlx_migrations",
    )
    .fetch_all(&mut *conn)
    .await?;

    for repair in plan_checksum_repairs(&records, migrator)? {
        let updated = sqlx::query(
            r#"UPDATE _sqlx_migrations
               SET checksum = $1
               WHERE version = $2
                 AND description = $3
                 AND success = true
                 AND checksum = $4
                 AND checksum != $1"#,
        )
        .bind(&repair.new_checksum)
        .bind(repair.version)
        .bind(&repair.description)
        .bind(&repair.old_checksum)
        .execute(&mut *conn)
        .await?;
        if updated.rows_affected() != 1 {
            anyhow::bail!(
                "migration metadata changed while repairing checksum for version {}",
                repair.version
            );
        }
        warn!(
            version = repair.version,
            description = %repair.description,
            "Repaired checksum for previously successful migration"
        );
    }

    Ok(())
}

/// Check whether the pgvector extension is loaded, attempting to create it if
/// the library files are available but the extension wasn't loaded yet (e.g.
/// the migration's `CREATE EXTENSION` failed on a previous run before the
/// library was correctly placed).
async fn detect_pgvector(pool: &PgPool) -> bool {
    let row: Option<(bool,)> =
        sqlx::query_as("SELECT EXISTS(SELECT 1 FROM pg_extension WHERE extname = 'vector')")
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();

    if row.is_some_and(|(exists,)| exists) {
        return true;
    }

    // Extension not loaded yet — try creating it now (the .dylib may have
    // been placed after the migration already ran and fell back to BYTEA).
    tracing::info!("pgvector extension not found, attempting CREATE EXTENSION vector");
    match sqlx::query("CREATE EXTENSION IF NOT EXISTS vector")
        .execute(pool)
        .await
    {
        Ok(_) => {
            tracing::info!("pgvector extension created successfully");
            true
        }
        Err(e) => {
            tracing::warn!(error = %e, "CREATE EXTENSION vector failed — pgvector unavailable");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::collections::HashSet;

    use sqlx::migrate::{Migration, MigrationType, Migrator};

    use super::{plan_checksum_repairs, ChecksumRepair, MigrationMetadata};

    fn migration(version: i64, description: &'static str, sql: &'static str) -> Migration {
        Migration::new(
            version,
            Cow::Borrowed(description),
            MigrationType::Simple,
            Cow::Borrowed(sql),
            false,
        )
    }

    fn migrator(migrations: Vec<Migration>) -> Migrator {
        Migrator {
            migrations: Cow::Owned(migrations),
            ignore_missing: false,
            locking: true,
            no_tx: false,
        }
    }

    fn metadata(migration: &Migration, success: bool, checksum: &[u8]) -> MigrationMetadata {
        MigrationMetadata {
            version: migration.version,
            description: migration.description.to_string(),
            success,
            checksum: checksum.to_vec(),
        }
    }

    fn apply_repairs(records: &mut [MigrationMetadata], repairs: &[ChecksumRepair]) {
        for repair in repairs {
            let record = records
                .iter_mut()
                .find(|record| record.version == repair.version)
                .expect("repair must reference an existing row");
            assert!(record.success);
            assert_eq!(record.description, repair.description);
            assert_eq!(record.checksum, repair.old_checksum);
            record.checksum.clone_from(&repair.new_checksum);
        }
    }

    /// Minimal SQLx-run simulation: dirty rows block; successful rows are
    /// checksum-validated; genuinely missing rows execute their SQL before a
    /// success record is appended.
    fn simulate_second_run(
        migrator: &Migrator,
        records: &mut Vec<MigrationMetadata>,
        schema_postconditions: &mut HashSet<String>,
    ) -> Result<Vec<i64>, String> {
        if let Some(dirty) = records.iter().find(|record| !record.success) {
            return Err(format!("dirty:{}", dirty.version));
        }

        let mut executed = Vec::new();
        for migration in migrator.iter() {
            if let Some(record) = records
                .iter()
                .find(|record| record.version == migration.version)
            {
                if record.checksum != migration.checksum.as_ref() {
                    return Err(format!("checksum:{}", migration.version));
                }
                continue;
            }

            // This represents the migration SQL/postcondition, and must happen
            // before SQLx writes the success metadata row.
            schema_postconditions.insert(migration.sql.to_string());
            executed.push(migration.version);
            records.push(metadata(migration, true, migration.checksum.as_ref()));
        }
        Ok(executed)
    }

    #[test]
    fn checksum_repair_leaves_missing_migration_for_second_run_to_execute_sql() {
        let old = migration(1, "old", "CREATE TABLE old_truth(id int)");
        let new = migration(2, "new", "CREATE TABLE new_truth(id int)");
        let migrator = migrator(vec![old.clone(), new.clone()]);
        let mut records = vec![metadata(&old, true, old.checksum.as_ref())];

        let repairs = plan_checksum_repairs(&records, &migrator)
            .expect("matching metadata needs no checksum repair");
        assert!(repairs.is_empty());
        apply_repairs(&mut records, &repairs);

        let mut postconditions = HashSet::new();
        let executed = simulate_second_run(&migrator, &mut records, &mut postconditions)
            .expect("second run should apply the missing migration");

        assert_eq!(executed, vec![new.version]);
        assert!(postconditions.contains(new.sql.as_ref()));
        assert!(records
            .iter()
            .any(|record| record.version == new.version && record.success));
    }

    #[test]
    fn dirty_migration_is_never_marked_success_or_checksum_repaired() {
        let dirty = migration(7, "dirty", "CREATE TABLE dirty_truth(id int)");
        let migrator = migrator(vec![dirty.clone()]);
        let mut records = vec![metadata(&dirty, false, b"partial-checksum")];

        let repairs = plan_checksum_repairs(&records, &migrator)
            .expect("dirty rows are not successful checksum candidates");
        assert!(repairs.is_empty());
        apply_repairs(&mut records, &repairs);

        let mut postconditions = HashSet::new();
        let error = simulate_second_run(&migrator, &mut records, &mut postconditions)
            .expect_err("dirty row must remain a hard failure");
        assert_eq!(error, "dirty:7");
        assert!(!records[0].success);
        assert!(postconditions.is_empty());
    }

    #[test]
    fn changed_successful_foundation_checksum_is_not_repaired() {
        let applied = migration(
            20260712000001,
            "runtime memory foundation",
            "ALTER TABLE operation_state ADD COLUMN runtime_memory_contract text",
        );
        let migrator = migrator(vec![applied.clone()]);
        let records = vec![metadata(&applied, true, b"old-prefix-checksum")];

        let error = plan_checksum_repairs(&records, &migrator)
            .expect_err("a changed successful foundation migration must fail closed");
        assert!(error.to_string().contains("20260712000001"));
    }

    #[test]
    fn description_mismatch_is_not_repaired() {
        let applied = migration(13, "expected description", "SELECT 13");
        let migrator = migrator(vec![applied.clone()]);
        let mut record = metadata(&applied, true, b"old-checksum");
        record.description = "different description".to_string();

        assert!(plan_checksum_repairs(&[record], &migrator)
            .expect("description mismatch is not a checksum repair candidate")
            .is_empty());
    }
}
