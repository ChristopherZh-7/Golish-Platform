//! Indexer state management with a pluggable backend.
//!
//! `IndexerState` wraps a `dyn IndexerBackend` behind a `RwLock` so that
//! upstream consumers (e.g. `golish-ai`) never depend on a concrete
//! indexer crate. The application layer injects the real implementation
//! (e.g. vtcode-indexer's `SimpleIndexer` via [`crate::vtcode_bridge`]).

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Mock backend that records every call so tests can assert on it.
    #[derive(Default)]
    struct MockBackend {
        files_indexed: Arc<AtomicUsize>,
        dirs_indexed: Arc<AtomicUsize>,
        search_calls: Arc<AtomicUsize>,
    }

    impl IndexerBackend for MockBackend {
        fn index_file(&mut self, _path: &Path) -> anyhow::Result<()> {
            self.files_indexed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn index_directory(&mut self, _path: &Path) -> anyhow::Result<()> {
            self.dirs_indexed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn all_files(&self) -> Vec<String> {
            vec!["mock.rs".into()]
        }
        fn search(
            &self,
            _pattern: &str,
            _path_filter: Option<&str>,
        ) -> anyhow::Result<Vec<CodeSearchResult>> {
            self.search_calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![CodeSearchResult {
                file_path: "mock.rs".into(),
                line_number: 1,
                line_content: "fn main()".into(),
                matches: vec!["fn".into()],
            }])
        }
        fn find_files(&self, _pattern: &str) -> anyhow::Result<Vec<String>> {
            Ok(vec!["mock.rs".into()])
        }
    }

    #[test]
    fn state_starts_uninitialized() {
        let s = IndexerState::new();
        assert!(!s.is_initialized());
        assert!(s.workspace_root().is_none());
        assert!(s.with_indexer(|_| Ok(())).is_err());
    }

    #[test]
    fn set_backend_marks_initialized() {
        let s = IndexerState::new();
        s.set_backend(Box::<MockBackend>::default(), PathBuf::from("/tmp/ws"));
        assert!(s.is_initialized());
        assert_eq!(s.workspace_root(), Some(PathBuf::from("/tmp/ws")));
    }

    #[test]
    fn with_indexer_dispatches_to_backend() {
        let s = IndexerState::new();
        let mock = MockBackend::default();
        let calls = mock.search_calls.clone();
        s.set_backend(Box::new(mock), PathBuf::from("/tmp/ws"));

        let hits = s
            .with_indexer(|b| b.search("fn", None))
            .expect("search should succeed");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file_path, "mock.rs");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn with_indexer_mut_records_writes() {
        let s = IndexerState::new();
        let mock = MockBackend::default();
        let files = mock.files_indexed.clone();
        let dirs = mock.dirs_indexed.clone();
        s.set_backend(Box::new(mock), PathBuf::from("/tmp/ws"));

        s.with_indexer_mut(|b| b.index_file(Path::new("/x/a.rs")))
            .unwrap();
        s.with_indexer_mut(|b| b.index_directory(Path::new("/x")))
            .unwrap();

        assert_eq!(files.load(Ordering::SeqCst), 1);
        assert_eq!(dirs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn shutdown_clears_backend() {
        let s = IndexerState::new();
        s.set_backend(Box::<MockBackend>::default(), PathBuf::from("/tmp/ws"));
        assert!(s.is_initialized());
        s.shutdown();
        assert!(!s.is_initialized());
        assert!(s.workspace_root().is_none());
    }
}
