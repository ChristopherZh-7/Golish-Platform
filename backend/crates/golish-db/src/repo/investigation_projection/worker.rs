//! Process-owned asynchronous projection worker.
//!
//! Canonical writers only append immutable outbox batches. This runtime is the
//! production bridge that eventually publishes those batches without coupling
//! canonical transaction success to projection availability.

use std::{sync::Arc, time::Duration};

use sqlx::{postgres::PgListener, PgPool};
use tokio::{
    sync::{watch, Mutex, Notify},
    task::JoinHandle,
};
use uuid::Uuid;

use super::{
    project_next_projection_batch, ProjectionProjectOutcome,
    INVESTIGATION_PROJECTION_NOTIFY_CHANNEL,
};

const POLL_INTERVAL: Duration = Duration::from_millis(250);
const OPERATION_WINDOW: i64 = 32;
const MAX_BATCHES_PER_OPERATION_PASS: usize = 8;

#[derive(Clone)]
pub struct InvestigationProjectionWorker {
    inner: Arc<WorkerInner>,
}

struct WorkerInner {
    pool: Arc<PgPool>,
    wake: Notify,
    running: Mutex<Option<RunningWorker>>,
}

struct RunningWorker {
    shutdown: watch::Sender<bool>,
    join: JoinHandle<()>,
}

struct ProjectionPass {
    last_operation_id: Option<Uuid>,
    made_progress: bool,
    had_failure: bool,
}

impl InvestigationProjectionWorker {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self {
            inner: Arc::new(WorkerInner {
                pool,
                wake: Notify::new(),
                running: Mutex::new(None),
            }),
        }
    }

    /// Start the singleton task owned by this runtime instance.
    ///
    /// Returns `false` when it was already running.
    pub async fn start(&self) -> bool {
        let mut running = self.inner.running.lock().await;
        if running.is_some() {
            return false;
        }
        let listener = connect_listener(&self.inner.pool).await;
        let (shutdown, shutdown_rx) = watch::channel(false);
        let inner = self.inner.clone();
        let join = tokio::spawn(async move {
            run_worker(inner, shutdown_rx, listener).await;
        });
        *running = Some(RunningWorker { shutdown, join });
        true
    }

    /// Wake a sleeping worker after a canonical commit. Correctness does not
    /// depend on the signal because the bounded poll loop also recovers work
    /// committed before process startup or after a lost notification.
    pub fn wake(&self) {
        self.inner.wake.notify_one();
    }

    /// Stop accepting work and join the active projection pass before its pool
    /// may be closed. Returns `false` when no task was running.
    pub async fn shutdown(&self) -> bool {
        let running = self.inner.running.lock().await.take();
        let Some(running) = running else {
            return false;
        };
        let _ = running.shutdown.send(true);
        self.inner.wake.notify_waiters();
        if let Err(error) = running.join.await {
            tracing::warn!(error = %error, "investigation projection worker join failed");
        }
        true
    }
}

async fn connect_listener(pool: &PgPool) -> Option<PgListener> {
    let mut listener = match PgListener::connect_with(pool).await {
        Ok(listener) => listener,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "investigation projection notification listener unavailable; polling remains active"
            );
            return None;
        }
    };
    if let Err(error) = listener
        .listen(INVESTIGATION_PROJECTION_NOTIFY_CHANNEL)
        .await
    {
        tracing::warn!(
            error = %error,
            "investigation projection notification subscription failed; polling remains active"
        );
        return None;
    }
    Some(listener)
}

async fn run_worker(
    inner: Arc<WorkerInner>,
    mut shutdown: watch::Receiver<bool>,
    mut listener: Option<PgListener>,
) {
    let mut after_operation_id = None;
    loop {
        if *shutdown.borrow() {
            return;
        }

        let pass = project_pending_window(&inner.pool, after_operation_id).await;
        let (made_progress, had_failure) = match pass {
            Ok(pass) => {
                after_operation_id = pass.last_operation_id;
                (pass.made_progress, pass.had_failure)
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    error_code = error.code(),
                    "investigation projection worker could not enumerate backlog"
                );
                (false, true)
            }
        };
        if made_progress && !had_failure {
            tokio::task::yield_now().await;
            continue;
        }

        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
            () = inner.wake.notified() => {}
            notification = receive_notification(&mut listener), if listener.is_some() => {
                if let Err(error) = notification {
                    tracing::warn!(
                        error = %error,
                        "investigation projection notification listener disconnected; polling remains active"
                    );
                    listener = None;
                }
            }
            () = tokio::time::sleep(POLL_INTERVAL) => {}
        }
        if listener.is_none() {
            listener = connect_listener(&inner.pool).await;
        }
    }
}

async fn receive_notification(listener: &mut Option<PgListener>) -> Result<(), sqlx::Error> {
    let listener = listener
        .as_mut()
        .expect("notification branch is disabled without a listener");
    listener.recv().await.map(|_| ())
}

async fn project_pending_window(
    pool: &PgPool,
    after_operation_id: Option<Uuid>,
) -> super::InvestigationProjectionResult<ProjectionPass> {
    let mut operation_ids = pending_operation_ids(pool, after_operation_id).await?;
    if operation_ids.is_empty() && after_operation_id.is_some() {
        operation_ids = pending_operation_ids(pool, None).await?;
    }
    let last_operation_id = operation_ids.last().copied();
    let mut made_progress = false;
    let mut had_failure = false;

    for operation_id in operation_ids {
        for _ in 0..MAX_BATCHES_PER_OPERATION_PASS {
            match project_next_projection_batch(pool, operation_id).await {
                Ok(Some(ProjectionProjectOutcome::Applied(_))) => made_progress = true,
                Ok(Some(ProjectionProjectOutcome::Replay(_))) => made_progress = true,
                Ok(Some(ProjectionProjectOutcome::PredecessorPending(_))) | Ok(None) => break,
                Err(error) => {
                    // A malformed or temporarily unavailable operation is
                    // isolated to this pass. The keyset scan continues so it
                    // cannot prevent another operation from becoming visible.
                    tracing::warn!(
                        %operation_id,
                        error = %error,
                        error_code = error.code(),
                        "investigation projection batch failed; retaining backlog for retry"
                    );
                    had_failure = true;
                    break;
                }
            }
        }
    }

    Ok(ProjectionPass {
        last_operation_id,
        made_progress,
        had_failure,
    })
}

async fn pending_operation_ids(
    pool: &PgPool,
    after_operation_id: Option<Uuid>,
) -> super::InvestigationProjectionResult<Vec<Uuid>> {
    Ok(sqlx::query_scalar(
        r#"SELECT batch.operation_id
             FROM investigation_projection_outbox_batches batch
             LEFT JOIN investigation_projection_batch_receipts receipt
               ON receipt.batch_id=batch.batch_id
             JOIN operation_state state ON state.operation_id=batch.operation_id
            WHERE (receipt.batch_id IS NULL OR (
                    state.investigation_rollout_mode IN (
                      'shadow_registry','dual_read_compare',
                      'registry_authoritative_legacy_projection'
                    ) AND EXISTS(
                      SELECT 1 FROM investigation_projection_outbox member
                       WHERE member.batch_id=batch.batch_id
                         AND NOT EXISTS(
                           SELECT 1 FROM investigation_projection_compare_samples sample
                            WHERE sample.operation_id=batch.operation_id
                              AND sample.as_of_change_seq=receipt.last_change_seq
                              AND sample.record_kind=member.entity_kind
                              AND sample.record_key=(member.source_entity_id || ':v' ||
                                                     member.source_entity_version::TEXT)
                         )
                    )
                  ))
              AND ($1::UUID IS NULL OR batch.operation_id>$1)
            GROUP BY batch.operation_id
            ORDER BY batch.operation_id
            LIMIT $2"#,
    )
    .bind(after_operation_id)
    .bind(OPERATION_WINDOW)
    .fetch_all(pool)
    .await?)
}
