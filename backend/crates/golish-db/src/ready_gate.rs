use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::watch;

/// Signaling primitive that lets consumers wait for the embedded PostgreSQL to
/// become ready. The background startup task calls `mark_ready()` once PG is
/// up and migrations have run. Fire-and-forget DB tasks call `is_ready()` to
/// skip work when PG is still starting, and blocking callers use `wait()`.
#[derive(Clone)]
pub struct DbReadyGate {
    ready: Arc<AtomicBool>,
    has_pgvector: Arc<AtomicBool>,
    tx: Arc<watch::Sender<bool>>,
    rx: watch::Receiver<bool>,
}

impl DbReadyGate {
    pub fn new() -> Self {
        let (tx, rx) = watch::channel(false);
        Self {
            ready: Arc::new(AtomicBool::new(false)),
            has_pgvector: Arc::new(AtomicBool::new(false)),
            tx: Arc::new(tx),
            rx,
        }
    }

    /// Non-blocking check — returns `true` once PG is ready.
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    /// Whether the pgvector extension is available (only meaningful after ready).
    pub fn has_pgvector(&self) -> bool {
        self.has_pgvector.load(Ordering::Acquire)
    }

    /// Async wait until PG is ready. Returns immediately if already ready.
    pub async fn wait(&mut self) {
        if self.is_ready() {
            return;
        }
        let _ = self.rx.wait_for(|&v| v).await;
    }

    /// Called by the background startup task once PG is fully up.
    pub fn mark_ready(&self) {
        self.ready.store(true, Ordering::Release);
        let _ = self.tx.send(true);
    }

    /// Set pgvector availability (call before mark_ready).
    pub fn set_pgvector_available(&self, available: bool) {
        self.has_pgvector.store(available, Ordering::Release);
    }

    /// Called if the background startup task fails.
    pub fn mark_failed(&self) {
        let _ = self.tx.send(false);
    }
}

impl Default for DbReadyGate {
    fn default() -> Self {
        Self::new()
    }
}
