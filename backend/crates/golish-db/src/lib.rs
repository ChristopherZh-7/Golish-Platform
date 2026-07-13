//! golish-db: PostgreSQL-backed persistence layer for the Golish platform.
//!
//! Provides:
//! - Embedded PostgreSQL via pg_embed (auto-download + lifecycle management)
//! - pgvector for semantic memory / similarity search
//! - Structured session → task → subtask → tool_call hierarchy
//! - Migrated pentest data (targets, findings, vault, etc.)
//! - Token usage analytics per agent type
//!
//! # Quick Start
//! ```rust,ignore
//! let db = GolishDb::start(DbConfig::default()).await?;
//! let pool = db.pool();
//!
//! // Create a session
//! let session = repo::sessions::create(pool, NewSession { ... }).await?;
//!
//! // Track a tool call
//! let tc = repo::tool_calls::create(pool, NewToolCall { ... }).await?;
//!
//! // Search vector memory
//! let similar = repo::memories::search_similar(pool, &embedding, None, 10).await?;
//!
//! // Graceful shutdown
//! db.stop().await;
//! ```

#![allow(clippy::too_many_arguments)]

pub mod config;
pub mod embedded;
pub mod embeddings;
pub mod error;
pub mod gatekeeper;
pub mod models;
pub mod pool;
pub mod repo;

use chrono::Duration;

pub use config::DbConfig;
pub use error::{DbError, Result};
pub use golish_core::DbReadyGate;
pub use models::*;
pub use pool::create_lazy_pool;
pub use repo::audit::{reclaim_abandoned_audits, DEFAULT_RECLAIM_THRESHOLD_HOURS};
/// The canonical connection-pool type golish-db hands to consumers that share
/// its embedded PostgreSQL instance (e.g. `golish-graphiti`). Re-exported so
/// those crates can express the dependency in their own type signatures instead
/// of pinning a second `sqlx` independently.
pub use sqlx::PgPool;

/// Top-level database handle. Owns the embedded PG server and connection pool.
pub struct GolishDb {
    embedded: embedded::EmbeddedPg,
    pool: PgPool,
    pub has_pgvector: bool,
}

impl GolishDb {
    /// Start the embedded PostgreSQL server, run migrations, and return a ready handle.
    ///
    /// After migrations land, reclaim any abandoned `audit_log` rows (status='started'
    /// but older than `DEFAULT_RECLAIM_THRESHOLD_HOURS` hours). Reclaim failure is
    /// logged but does NOT abort startup — the platform must come up even if the
    /// audit table has a transient issue (Doc 1 §5.3 fire-and-forget semantics).
    pub async fn start(config: DbConfig) -> anyhow::Result<Self> {
        let embedded = embedded::EmbeddedPg::start(config).await?;
        let info = pool::create_pool(&embedded.connection_string()).await?;

        match reclaim_abandoned_audits(&info.pool, Duration::hours(DEFAULT_RECLAIM_THRESHOLD_HOURS))
            .await
        {
            Ok(n) if n > 0 => {
                tracing::info!(
                    reclaimed = n,
                    "Reclaimed abandoned audit_log rows on startup"
                );
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = %e, "Failed to reclaim abandoned audit_log rows on startup");
            }
        }

        // Same fire-and-forget reclaim for tasks: a process killed mid-run can't
        // finalize its own `tasks` rows, so they leak forever as `running`.
        //
        // Runtime-aware compound reaper: first fence expired relational workers
        // (requeue only when no tool is active; otherwise recovery_required),
        // then pause complete resumable operations and fail malformed remainder.
        // Live leases are left untouched. All four outcomes commit together.
        match repo::tasks::startup_reap_abandoned(
            &info.pool,
            Duration::hours(DEFAULT_RECLAIM_THRESHOLD_HOURS),
        )
        .await
        {
            Ok(stats)
                if stats.paused > 0
                    || stats.failed > 0
                    || stats.workers_requeued > 0
                    || stats.workers_recovery_required > 0 =>
            {
                tracing::info!(
                    paused = stats.paused,
                    failed = stats.failed,
                    workers_requeued = stats.workers_requeued,
                    workers_recovery_required = stats.workers_recovery_required,
                    "Reconciled abandoned runtime operations on startup"
                );
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = %e, "Failed to reconcile abandoned runtime operations on startup");
            }
        }

        match repo::attack_candidate_approvals::reap_stale_candidate_review_dispatches(
            &info.pool,
            Duration::seconds(
                repo::attack_candidate_approvals::DEFAULT_REVIEW_DISPATCH_STALE_SECONDS,
            ),
        )
        .await
        {
            Ok(n) if n > 0 => {
                tracing::info!(
                    reclaimed = n,
                    "Reset stale Candidate review resume dispatches on startup"
                );
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = %e, "Failed to reset Candidate review dispatches on startup");
            }
        }

        Ok(Self {
            embedded,
            pool: info.pool,
            has_pgvector: info.has_pgvector,
        })
    }

    /// Get a reference to the connection pool for query operations.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Gracefully shut down the database.
    pub async fn stop(&mut self) {
        self.pool.close().await;
        self.embedded.stop().await;
    }
}
