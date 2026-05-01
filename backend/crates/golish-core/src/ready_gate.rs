//! Database readiness gate.
//!
//! A signaling primitive that lets consumers wait for the embedded PostgreSQL
//! to become ready. Lives in golish-core so both golish-ai (consumer) and
//! golish-db (producer) can share the same type without a dependency cycle.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::watch;

type GateState = Option<bool>;

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

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    pub fn is_failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    pub fn has_pgvector(&self) -> bool {
        self.has_pgvector.load(Ordering::Acquire)
    }

    pub async fn wait(&mut self) -> bool {
        if self.is_ready() {
            return true;
        }
        if self.is_failed() {
            return false;
        }
        let _ = self.rx.wait_for(|v| v.is_some()).await;
        self.is_ready()
    }

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

    pub fn mark_ready(&self) {
        self.ready.store(true, Ordering::Release);
        let _ = self.tx.send(Some(true));
    }

    pub fn set_pgvector_available(&self, available: bool) {
        self.has_pgvector.store(available, Ordering::Release);
    }

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
