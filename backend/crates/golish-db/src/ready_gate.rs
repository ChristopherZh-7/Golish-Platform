use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::watch;

/// Gate state: `None` = still starting, `Some(true)` = ready, `Some(false)` = failed.
type GateState = Option<bool>;

/// Signaling primitive that lets consumers wait for the embedded PostgreSQL to
/// become ready. The background startup task calls `mark_ready()` once PG is
/// up and migrations have run. Fire-and-forget DB tasks call `is_ready()` to
/// skip work when PG is still starting, and blocking callers use `wait()`.
#[derive(Clone)]
pub struct DbReadyGate {
    ready: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    has_pgvector: Arc<AtomicBool>,
    tx: Arc<watch::Sender<GateState>>,
    rx: watch::Receiver<GateState>,
}

impl DbReadyGate {
    pub fn new() -> Self {
        let (tx, rx) = watch::channel(None);
        Self {
            ready: Arc::new(AtomicBool::new(false)),
            failed: Arc::new(AtomicBool::new(false)),
            has_pgvector: Arc::new(AtomicBool::new(false)),
            tx: Arc::new(tx),
            rx,
        }
    }

    /// Non-blocking check — returns `true` once PG is ready.
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    /// Non-blocking check — returns `true` if startup failed.
    pub fn is_failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    /// Whether the pgvector extension is available (only meaningful after ready).
    pub fn has_pgvector(&self) -> bool {
        self.has_pgvector.load(Ordering::Acquire)
    }

    /// Async wait until PG is ready or failed. Returns `true` if ready, `false`
    /// if startup failed.
    pub async fn wait(&mut self) -> bool {
        if self.is_ready() {
            return true;
        }
        if self.is_failed() {
            return false;
        }
        // Wait for any resolved state (Some(true) or Some(false))
        let _ = self.rx.wait_for(|v| v.is_some()).await;
        self.is_ready()
    }

    /// Wait with a timeout. Returns `true` if PG became ready, `false` on
    /// timeout or failure.
    pub async fn wait_timeout(&self, timeout: std::time::Duration) -> bool {
        if self.is_ready() {
            return true;
        }
        if self.is_failed() {
            return false;
        }
        let mut rx = self.rx.clone();
        let result = tokio::time::timeout(timeout, rx.wait_for(|v| v.is_some())).await;
        match result {
            Ok(Ok(state)) => state.unwrap_or(false),
            _ => false,
        }
    }

    /// Called by the background startup task once PG is fully up.
    pub fn mark_ready(&self) {
        self.ready.store(true, Ordering::Release);
        let _ = self.tx.send(Some(true));
    }

    /// Set pgvector availability (call before mark_ready).
    pub fn set_pgvector_available(&self, available: bool) {
        self.has_pgvector.store(available, Ordering::Release);
    }

    /// Called if the background startup task fails.
    pub fn mark_failed(&self) {
        self.failed.store(true, Ordering::Release);
        let _ = self.tx.send(Some(false));
    }
}

impl Default for DbReadyGate {
    fn default() -> Self {
        Self::new()
    }
}
