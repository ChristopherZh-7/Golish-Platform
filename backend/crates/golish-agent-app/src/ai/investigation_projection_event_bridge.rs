//! Commit-after-projection refresh events for the unified Investigation UI.
//!
//! The canonical writer and the read-model projector already communicate via
//! an immutable PostgreSQL outbox.  This bridge deliberately consumes only
//! committed projection receipts whose sequence is also the committed
//! projection head.  The emitted event is a refresh hint; it never replaces a
//! projection read and its in-memory delivery watermark is not an authority.

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use anyhow::Context;
use async_trait::async_trait;
use golish_core::{
    events::AiEvent,
    runtime::{GolishRuntime, RuntimeEvent},
};
use golish_db::repo::investigation_projection::InvestigationProjectionWorker;
use sqlx::PgPool;
use tokio::{
    sync::{watch, Mutex},
    task::JoinHandle,
};
use uuid::Uuid;

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// One DB-committed projection head and its exact frontend routing identity.
///
/// `session_id` is transport metadata for the shared `ai-event` channel.  The
/// [`AiEvent::InvestigationProjectionChanged`] body contains only the four
/// fields specified by the Investigation refresh contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedInvestigationProjectionRefresh {
    pub session_id: String,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_request_id: String,
    pub change_seq: i64,
}

#[async_trait]
pub trait InvestigationProjectionReceiptSource: Send + Sync + 'static {
    /// Return the latest committed projection receipt for each operation.
    /// Implementations may return duplicates; the bridge enforces monotonic
    /// delivery independently.
    async fn latest_committed_refreshes(
        &self,
    ) -> anyhow::Result<Vec<CommittedInvestigationProjectionRefresh>>;
}

#[derive(Clone)]
struct PgInvestigationProjectionReceiptSource {
    pool: Arc<PgPool>,
}

#[derive(sqlx::FromRow)]
struct CommittedRefreshRow {
    session_id: String,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_request_id: String,
    change_seq: i64,
}

#[async_trait]
impl InvestigationProjectionReceiptSource for PgInvestigationProjectionReceiptSource {
    async fn latest_committed_refreshes(
        &self,
    ) -> anyhow::Result<Vec<CommittedInvestigationProjectionRefresh>> {
        // The receipt and projection head are written by one projector
        // transaction. Requiring exact equality excludes both pre-commit
        // source batches and any stale receipt. One Investigation authority is
        // allowed per operation; an ambiguous/foreign identity is omitted
        // rather than guessed from a "latest" stage row.
        let rows = sqlx::query_as::<_, CommittedRefreshRow>(
            r#"WITH latest_receipt AS (
                   SELECT DISTINCT ON(receipt.operation_id)
                          receipt.operation_id,receipt.last_change_seq
                     FROM investigation_projection_batch_receipts receipt
                     JOIN investigation_projection_heads projection
                       ON projection.operation_id=receipt.operation_id
                      AND projection.change_seq=receipt.last_change_seq
                    ORDER BY receipt.operation_id,receipt.last_change_seq DESC
               ), exact_run_identity AS (
                   SELECT head.operation_id,
                          MIN(head.stage_execution_id::TEXT)::UUID AS stage_execution_id,
                          MIN(head.owning_stage_run_request_id) AS stage_run_request_id
                     FROM investigation_run_heads head
                    GROUP BY head.operation_id
                   HAVING COUNT(*)=1
               )
               SELECT session.chat_session_key AS session_id,
                      receipt.operation_id,identity.stage_execution_id,
                      identity.stage_run_request_id,
                      receipt.last_change_seq AS change_seq
                 FROM latest_receipt receipt
                 JOIN exact_run_identity identity USING(operation_id)
                 JOIN tasks task ON task.id=receipt.operation_id
                 JOIN sessions session ON session.id=task.session_id
                WHERE session.chat_session_key IS NOT NULL
                  AND btrim(session.chat_session_key)<>''
                ORDER BY receipt.operation_id"#,
        )
        .fetch_all(self.pool.as_ref())
        .await
        .context("load committed Investigation projection refresh heads")?;

        Ok(rows
            .into_iter()
            .filter(|row| row.change_seq > 0 && !row.stage_run_request_id.trim().is_empty())
            .map(|row| CommittedInvestigationProjectionRefresh {
                session_id: row.session_id,
                operation_id: row.operation_id,
                stage_execution_id: row.stage_execution_id,
                stage_run_request_id: row.stage_run_request_id,
                change_seq: row.change_seq,
            })
            .collect())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InvestigationProjectionPublishOutcome {
    pub emitted: usize,
    pub duplicate_or_out_of_order: usize,
    pub foreign_identity: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeliveredHead {
    stage_execution_id: Uuid,
    stage_run_request_id: String,
    change_seq: i64,
}

struct RunningPublisher {
    shutdown: watch::Sender<bool>,
    join: JoinHandle<()>,
}

struct BridgeInner {
    projection_worker: InvestigationProjectionWorker,
    source: Arc<dyn InvestigationProjectionReceiptSource>,
    delivered: Mutex<BTreeMap<Uuid, DeliveredHead>>,
    running: Mutex<Option<RunningPublisher>>,
}

/// Process-owned lifecycle for projection materialization plus refresh events.
#[derive(Clone)]
pub struct InvestigationProjectionEventBridge {
    inner: Arc<BridgeInner>,
}

impl InvestigationProjectionEventBridge {
    /// Compose the production PostgreSQL receipt source and existing
    /// whole-batch projection worker over the same pool.
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self::with_receipt_source(
            pool.clone(),
            Arc::new(PgInvestigationProjectionReceiptSource { pool }),
        )
    }

    /// Explicit source seam used by deterministic producer tests. Production
    /// composition should call [`Self::new`].
    pub fn with_receipt_source(
        pool: Arc<PgPool>,
        source: Arc<dyn InvestigationProjectionReceiptSource>,
    ) -> Self {
        Self {
            inner: Arc::new(BridgeInner {
                projection_worker: InvestigationProjectionWorker::new(pool),
                source,
                delivered: Mutex::new(BTreeMap::new()),
                running: Mutex::new(None),
            }),
        }
    }

    /// Publish one bounded snapshot of latest committed heads.
    ///
    /// The delivery watermark advances only after the runtime accepts the
    /// event. A failed emission is therefore retried on the next pass. A
    /// changed stage identity for an already-observed operation is rejected as
    /// foreign instead of being rebound to a new frontend session.
    pub async fn publish_once(
        &self,
        runtime: &dyn GolishRuntime,
    ) -> anyhow::Result<InvestigationProjectionPublishOutcome> {
        let mut refreshes = self.inner.source.latest_committed_refreshes().await?;
        refreshes.sort_by_key(|refresh| (refresh.operation_id, refresh.change_seq));

        let mut outcome = InvestigationProjectionPublishOutcome::default();
        let mut delivered = self.inner.delivered.lock().await;
        for refresh in refreshes {
            if refresh.change_seq <= 0
                || refresh.session_id.trim().is_empty()
                || refresh.stage_run_request_id.trim().is_empty()
            {
                outcome.foreign_identity += 1;
                continue;
            }
            if let Some(previous) = delivered.get(&refresh.operation_id) {
                if previous.stage_execution_id != refresh.stage_execution_id
                    || previous.stage_run_request_id != refresh.stage_run_request_id
                {
                    outcome.foreign_identity += 1;
                    continue;
                }
                if refresh.change_seq <= previous.change_seq {
                    outcome.duplicate_or_out_of_order += 1;
                    continue;
                }
            }

            runtime
                .emit(RuntimeEvent::Ai {
                    session_id: refresh.session_id.clone(),
                    event: Box::new(AiEvent::InvestigationProjectionChanged {
                        operation_id: refresh.operation_id.to_string(),
                        stage_execution_id: refresh.stage_execution_id.to_string(),
                        stage_run_request_id: refresh.stage_run_request_id.clone(),
                        change_seq: refresh.change_seq,
                    }),
                })
                .context("emit committed Investigation projection refresh")?;
            delivered.insert(
                refresh.operation_id,
                DeliveredHead {
                    stage_execution_id: refresh.stage_execution_id,
                    stage_run_request_id: refresh.stage_run_request_id,
                    change_seq: refresh.change_seq,
                },
            );
            outcome.emitted += 1;
        }
        Ok(outcome)
    }

    /// Start the process singleton after database readiness. The projection
    /// worker performs cold outbox replay; the publisher independently observes
    /// only committed receipts and therefore cannot emit before projection
    /// commit.
    pub async fn start(&self, runtime: Arc<dyn GolishRuntime>) -> bool {
        let mut running = self.inner.running.lock().await;
        if running.is_some() {
            return false;
        }
        self.inner.projection_worker.start().await;
        let (shutdown, shutdown_rx) = watch::channel(false);
        let bridge = self.clone();
        let join = tokio::spawn(async move {
            bridge.run_publisher(runtime, shutdown_rx).await;
        });
        *running = Some(RunningPublisher { shutdown, join });
        true
    }

    /// Wake the projection side after a local canonical commit. Correctness
    /// still relies on PostgreSQL NOTIFY plus bounded polling.
    pub fn wake(&self) {
        self.inner.projection_worker.wake();
    }

    /// Drain the projector first, then run one final committed-receipt pass and
    /// join the publisher before the database pool is closed.
    pub async fn shutdown(&self) -> bool {
        let running = self.inner.running.lock().await.take();
        let Some(running) = running else {
            return false;
        };
        self.inner.projection_worker.shutdown().await;
        let _ = running.shutdown.send(true);
        if let Err(error) = running.join.await {
            tracing::warn!(error = %error, "Investigation projection event bridge join failed");
        }
        true
    }

    async fn run_publisher(
        &self,
        runtime: Arc<dyn GolishRuntime>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        loop {
            if let Err(error) = self.publish_once(runtime.as_ref()).await {
                tracing::warn!(
                    error = %error,
                    "Investigation projection refresh pass failed; retaining watermark for retry"
                );
            }
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        // The projection worker has already joined, so this
                        // final read observes every batch committed before
                        // shutdown without guessing a pre-commit sequence.
                        if let Err(error) = self.publish_once(runtime.as_ref()).await {
                            tracing::warn!(
                                error = %error,
                                "final Investigation projection refresh drain failed"
                            );
                        }
                        return;
                    }
                }
                () = tokio::time::sleep(EVENT_POLL_INTERVAL) => {}
            }
        }
    }
}
