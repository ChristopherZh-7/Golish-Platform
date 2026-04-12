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

pub mod config;
pub mod embedded;
pub mod embeddings;
pub mod gatekeeper;
pub mod models;
pub mod pool;
pub mod ready_gate;
pub mod repo;

use anyhow::Result;
use sqlx::PgPool;

pub use config::DbConfig;
pub use models::*;
pub use ready_gate::DbReadyGate;

/// Top-level database handle. Owns the embedded PG server and connection pool.
pub struct GolishDb {
    embedded: embedded::EmbeddedPg,
    pool: PgPool,
    pub has_pgvector: bool,
}

impl GolishDb {
    /// Start the embedded PostgreSQL server, run migrations, and return a ready handle.
    pub async fn start(config: DbConfig) -> Result<Self> {
        let embedded = embedded::EmbeddedPg::start(config).await?;
        let info = pool::create_pool(&embedded.connection_string()).await?;
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
