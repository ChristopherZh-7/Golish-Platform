//! Indexer state management with a pluggable backend.
//!
//! `IndexerState` wraps a `dyn IndexerBackend` behind a `RwLock` so that
//! golish-ai never depends on a concrete indexer crate. The app layer
//! injects the real implementation (e.g. vtcode-indexer's `SimpleIndexer`).

use parking_lot::RwLock;
use std::path::{Path, PathBuf};

/// Trait for the code-indexer backend injected at application startup.
///
/// Implementors wrap a concrete indexer (e.g. `SimpleIndexer`) and expose
/// the subset of operations used by the rest of the platform.
pub trait IndexerBackend: Send + Sync {
    fn index_file(&mut self, path: &Path) -> anyhow::Result<()>;
    fn index_directory(&mut self, path: &Path) -> anyhow::Result<()>;
    fn all_files(&self) -> Vec<String>;
    fn search(
        &self,
        pattern: &str,
        path_filter: Option<&str>,
    ) -> anyhow::Result<Vec<CodeSearchResult>>;
    fn find_files(&self, pattern: &str) -> anyhow::Result<Vec<String>>;
}

/// A single search hit returned by [`IndexerBackend::search`].
#[derive(Debug, Clone)]
pub struct CodeSearchResult {
    pub file_path: String,
    pub line_number: usize,
    pub line_content: String,
    pub matches: Vec<String>,
}

/// Manages the code indexer state.
///
/// The backend is injected via [`Self::set_backend`]; until that is called
/// every accessor returns "not initialized".
pub struct IndexerState {
    backend: RwLock<Option<Box<dyn IndexerBackend>>>,
    workspace_root: RwLock<Option<PathBuf>>,
}

impl IndexerState {
    pub fn new() -> Self {
        Self {
            backend: RwLock::new(None),
            workspace_root: RwLock::new(None),
        }
    }

    /// Inject a ready-to-use indexer backend.
    pub fn set_backend(&self, backend: Box<dyn IndexerBackend>, workspace_root: PathBuf) {
        *self.backend.write() = Some(backend);
        *self.workspace_root.write() = Some(workspace_root);
    }

    pub fn is_initialized(&self) -> bool {
        self.backend.read().is_some()
    }

    pub fn workspace_root(&self) -> Option<PathBuf> {
        self.workspace_root.read().clone()
    }

    /// Access the indexer for read operations.
    pub fn with_indexer<F, R>(&self, f: F) -> anyhow::Result<R>
    where
        F: FnOnce(&dyn IndexerBackend) -> anyhow::Result<R>,
    {
        let guard = self.backend.read();
        match guard.as_ref() {
            Some(backend) => f(backend.as_ref()),
            None => anyhow::bail!("Indexer not initialized"),
        }
    }

    /// Access the indexer for write operations.
    pub fn with_indexer_mut<F, R>(&self, f: F) -> anyhow::Result<R>
    where
        F: FnOnce(&mut dyn IndexerBackend) -> anyhow::Result<R>,
    {
        let mut guard = self.backend.write();
        match guard.as_mut() {
            Some(backend) => f(backend.as_mut()),
            None => anyhow::bail!("Indexer not initialized"),
        }
    }

    pub fn shutdown(&self) {
        tracing::info!("Shutting down indexer");
        *self.backend.write() = None;
        *self.workspace_root.write() = None;
    }
}

impl Default for IndexerState {
    fn default() -> Self {
        Self::new()
    }
}
