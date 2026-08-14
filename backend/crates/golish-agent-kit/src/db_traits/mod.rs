//! Trait abstractions and local model types for database operations.
//!
//! This module fully decouples `golish-ai` from `golish-db` and `sqlx`.
//! The application layer provides concrete implementations via:
//! - [`DbRepoProvider`] — repository operations (CRUD for tasks, subtasks, plans, wiki, etc.)
//! - [`DbTrackingBackend`] — fire-and-forget recording + memory store/search operations
//! - [`DbReadinessGate`] — PG startup readiness gate
//! - [`TextEmbedder`] — text embedding for semantic memory search

pub mod hypothesis_registry;
pub mod investigation_analysis_host;
pub mod investigation_asset_queue;
pub mod investigation_asset_verification;
pub mod investigation_nested_dispatch;
pub mod memory;
pub mod repo;
pub mod runtime_memory;
pub mod tracking;
pub mod types;
pub mod unified_investigation;

pub use hypothesis_registry::*;
pub use investigation_analysis_host::*;
pub use investigation_asset_queue::*;
pub use investigation_asset_verification::*;
pub use investigation_nested_dispatch::*;
pub use memory::*;
pub use repo::*;
pub use runtime_memory::*;
pub use tracking::*;
pub use types::*;
pub use unified_investigation::*;

use async_trait::async_trait;

// ── Database readiness gate ─────────────────────────────────────────────

/// Readiness gate for the database connection pool.
///
/// The application layer provides a concrete implementation that wraps
/// the embedded-PG startup signal.
#[async_trait]
pub trait DbReadinessGate: Send + Sync {
    fn is_ready(&self) -> bool;
    fn is_failed(&self) -> bool;
    async fn wait(&mut self) -> bool;
    fn clone_box(&self) -> Box<dyn DbReadinessGate>;
}

impl Clone for Box<dyn DbReadinessGate> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

// ── Embedder ────────────────────────────────────────────────────────────

/// Text embedding for semantic search.
#[async_trait]
pub trait TextEmbedder: Send + Sync {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>>;
    async fn embed_batch(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        let mut results = Vec::with_capacity(texts.len());
        for text in texts {
            results.push(self.embed(text).await?);
        }
        Ok(results)
    }
    fn dimension(&self) -> usize;
    fn model_name(&self) -> &str;
}
