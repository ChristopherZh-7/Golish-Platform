use std::collections::BTreeSet;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::FutureExt;
use golish_memory_domain::event_catalog::ProjectorId;

use crate::ports::MemoryError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectorRunState {
    Idle,
    Processed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupervisorStartOutcome {
    Started,
    AlreadyRunning,
}

/// One independently fenced delivery consumer. `register` may activate a
/// migration-time paused registry row, but must preserve an administrative
/// disabled state. `run_once` owns at most one leased delivery.
#[async_trait]
pub trait KnowledgeProjectorWorker: Send + Sync {
    fn projector_id(&self) -> ProjectorId;
    async fn register(&self) -> Result<(), MemoryError>;
    async fn run_once(&self) -> Result<ProjectorRunState, MemoryError>;
}

/// Shared process owner for all Memory Fabric projector loops.
///
/// Clones share the same lifecycle gate, cancellation channel and join set.
/// `start` is idempotent under hostile concurrent calls. Shutdown stops taking
/// new deliveries, waits for the current batch, and only then releases the
/// owner so a later process-level restart cannot overlap the old generation.
#[derive(Clone)]
pub struct KnowledgeProjectorSupervisor {
    inner: Arc<SupervisorInner>,
}

struct SupervisorInner {
    workers: Vec<Arc<dyn KnowledgeProjectorWorker>>,
    lifecycle: tokio::sync::Mutex<()>,
    state: Mutex<SupervisorState>,
}

#[derive(Default)]
struct SupervisorState {
    running: bool,
    start_count: u64,
    cancel: Option<tokio::sync::watch::Sender<bool>>,
    joins: Vec<tokio::task::JoinHandle<()>>,
}

impl KnowledgeProjectorSupervisor {
    pub fn new(workers: Vec<Arc<dyn KnowledgeProjectorWorker>>) -> Result<Self, MemoryError> {
        if workers.is_empty() {
            return Err(MemoryError::Policy(
                "memory_supervisor_workers_empty".to_string(),
            ));
        }
        let mut keys = BTreeSet::new();
        for worker in &workers {
            if !keys.insert(worker.projector_id().key()) {
                return Err(MemoryError::Policy(
                    "memory_supervisor_duplicate_projector".to_string(),
                ));
            }
        }
        Ok(Self {
            inner: Arc::new(SupervisorInner {
                workers,
                lifecycle: tokio::sync::Mutex::new(()),
                state: Mutex::new(SupervisorState::default()),
            }),
        })
    }

    pub async fn start(&self) -> Result<SupervisorStartOutcome, MemoryError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        if self.state().running {
            return Ok(SupervisorStartOutcome::AlreadyRunning);
        }
        for worker in &self.inner.workers {
            worker.register().await?;
        }

        let (cancel, _) = tokio::sync::watch::channel(false);
        let joins = self
            .inner
            .workers
            .iter()
            .cloned()
            .map(|worker| {
                let cancel = cancel.subscribe();
                tokio::spawn(run_worker_loop(worker, cancel))
            })
            .collect::<Vec<_>>();
        let mut state = self.state();
        state.running = true;
        state.start_count += 1;
        state.cancel = Some(cancel);
        state.joins = joins;
        Ok(SupervisorStartOutcome::Started)
    }

    pub async fn shutdown(&self) -> Result<(), MemoryError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        let (cancel, joins) = {
            let mut state = self.state();
            if !state.running {
                return Ok(());
            }
            (state.cancel.take(), std::mem::take(&mut state.joins))
        };
        if let Some(cancel) = cancel {
            let _ = cancel.send(true);
        }
        for join in joins {
            if let Err(error) = join.await {
                tracing::warn!(error = %error, "memory projector worker join failed");
            }
        }
        self.state().running = false;
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.state().running
    }

    pub fn owner_count(&self) -> usize {
        let state = self.state();
        if state.running {
            state.joins.len()
        } else {
            0
        }
    }

    pub fn start_count(&self) -> u64 {
        self.state().start_count
    }

    pub fn worker_count(&self) -> usize {
        self.inner.workers.len()
    }

    fn state(&self) -> std::sync::MutexGuard<'_, SupervisorState> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

async fn run_worker_loop(
    worker: Arc<dyn KnowledgeProjectorWorker>,
    mut cancel: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        if *cancel.borrow() {
            break;
        }
        let delay = match AssertUnwindSafe(worker.run_once()).catch_unwind().await {
            Ok(Ok(ProjectorRunState::Processed)) => Duration::ZERO,
            Ok(Ok(ProjectorRunState::Idle)) => Duration::from_millis(100),
            Ok(Err(error)) => {
                tracing::warn!(
                    projector = worker.projector_id().key(),
                    error_code = error.code(),
                    "memory projector delivery failed; retry remains fenced by outbox"
                );
                Duration::from_secs(1)
            }
            Err(_) => {
                // A panic after claim deliberately leaves the lease untouched.
                // This loop survives; the same delivery becomes claimable after
                // the DB lease expires, preserving crash replay semantics.
                tracing::error!(
                    projector = worker.projector_id().key(),
                    "memory projector panicked; leased delivery will retry after expiry"
                );
                Duration::from_secs(1)
            }
        };
        if *cancel.borrow() {
            break;
        }
        if delay.is_zero() {
            tokio::task::yield_now().await;
            continue;
        }
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() {
                    break;
                }
            }
        }
    }
}

/// Compatibility port retained for callers that explicitly register one
/// implemented projector before starting the shared owner.
#[async_trait]
pub trait KnowledgeProjectorSupervisorPort: Send + Sync {
    async fn register(&self, projector: ProjectorId) -> Result<(), MemoryError>;
    async fn shutdown(&self) -> Result<(), MemoryError>;
}

#[async_trait]
impl KnowledgeProjectorSupervisorPort for KnowledgeProjectorSupervisor {
    async fn register(&self, projector: ProjectorId) -> Result<(), MemoryError> {
        let worker = self
            .inner
            .workers
            .iter()
            .find(|worker| worker.projector_id() == projector)
            .ok_or_else(|| {
                MemoryError::Policy("memory_supervisor_projector_unavailable".to_string())
            })?;
        worker.register().await
    }

    async fn shutdown(&self) -> Result<(), MemoryError> {
        KnowledgeProjectorSupervisor::shutdown(self).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use super::*;

    struct FakeWorker {
        projector: ProjectorId,
        registrations: AtomicUsize,
        runs: AtomicUsize,
        block_first: AtomicBool,
        started: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }

    impl FakeWorker {
        fn new(projector: ProjectorId) -> Self {
            Self {
                projector,
                registrations: AtomicUsize::new(0),
                runs: AtomicUsize::new(0),
                block_first: AtomicBool::new(false),
                started: tokio::sync::Notify::new(),
                release: tokio::sync::Notify::new(),
            }
        }
    }

    #[async_trait]
    impl KnowledgeProjectorWorker for FakeWorker {
        fn projector_id(&self) -> ProjectorId {
            self.projector
        }

        async fn register(&self) -> Result<(), MemoryError> {
            self.registrations.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn run_once(&self) -> Result<ProjectorRunState, MemoryError> {
            let run = self.runs.fetch_add(1, Ordering::SeqCst);
            if run == 0 && self.block_first.load(Ordering::SeqCst) {
                self.started.notify_one();
                self.release.notified().await;
                return Ok(ProjectorRunState::Processed);
            }
            Ok(ProjectorRunState::Idle)
        }
    }

    #[tokio::test]
    async fn desktop_two_sessions_observe_one_process_owner() {
        let worker = Arc::new(FakeWorker::new(ProjectorId::AssertionPromoterV1));
        let supervisor =
            KnowledgeProjectorSupervisor::new(vec![worker.clone()]).expect("valid supervisor");
        let session_a = supervisor.clone();
        let session_b = supervisor.clone();
        let (left, right) = tokio::join!(session_a.start(), session_b.start());
        let outcomes = [
            left.expect("session a start"),
            right.expect("session b start"),
        ];
        assert!(outcomes.contains(&SupervisorStartOutcome::Started));
        assert!(outcomes.contains(&SupervisorStartOutcome::AlreadyRunning));
        assert_eq!(supervisor.start_count(), 1);
        assert_eq!(supervisor.owner_count(), 1);
        assert_eq!(worker.registrations.load(Ordering::SeqCst), 1);
        supervisor.shutdown().await.expect("shutdown supervisor");
    }

    #[tokio::test]
    async fn cli_shutdown_awaits_the_inflight_projector_batch() {
        let worker = Arc::new(FakeWorker::new(ProjectorId::DocumentProjectorV1));
        worker.block_first.store(true, Ordering::SeqCst);
        let supervisor =
            KnowledgeProjectorSupervisor::new(vec![worker.clone()]).expect("valid supervisor");
        supervisor.start().await.expect("start supervisor");
        tokio::time::timeout(Duration::from_secs(1), worker.started.notified())
            .await
            .expect("worker started first batch");

        let shutdown = tokio::spawn({
            let supervisor = supervisor.clone();
            async move { supervisor.shutdown().await }
        });
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert!(
            !shutdown.is_finished(),
            "shutdown returned before batch ended"
        );
        worker.release.notify_waiters();
        shutdown
            .await
            .expect("join shutdown")
            .expect("graceful shutdown");
        assert!(!supervisor.is_running());
    }

    #[test]
    fn duplicate_projector_owner_is_rejected() {
        let first: Arc<dyn KnowledgeProjectorWorker> =
            Arc::new(FakeWorker::new(ProjectorId::GraphProjectorV1));
        let second: Arc<dyn KnowledgeProjectorWorker> =
            Arc::new(FakeWorker::new(ProjectorId::GraphProjectorV1));
        let error = KnowledgeProjectorSupervisor::new(vec![first, second])
            .err()
            .expect("duplicate projector rejected");
        assert_eq!(error.code(), "memory_policy_rejected");
    }
}
