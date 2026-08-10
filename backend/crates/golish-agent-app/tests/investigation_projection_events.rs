use std::{
    any::Any,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use async_trait::async_trait;
use golish_agent_app::ai::{
    CommittedInvestigationProjectionRefresh, InvestigationProjectionEventBridge,
    InvestigationProjectionReceiptSource,
};
use golish_core::{
    events::AiEvent,
    runtime::{ApprovalResult, GolishRuntime, RuntimeError, RuntimeEvent},
};
use sqlx::{postgres::PgPoolOptions, PgPool};
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Default)]
struct FakeReceiptSource {
    rows: RwLock<Vec<CommittedInvestigationProjectionRefresh>>,
}

impl FakeReceiptSource {
    async fn replace(&self, rows: Vec<CommittedInvestigationProjectionRefresh>) {
        *self.rows.write().await = rows;
    }
}

#[async_trait]
impl InvestigationProjectionReceiptSource for FakeReceiptSource {
    async fn latest_committed_refreshes(
        &self,
    ) -> anyhow::Result<Vec<CommittedInvestigationProjectionRefresh>> {
        Ok(self.rows.read().await.clone())
    }
}

#[derive(Default)]
struct CaptureRuntime {
    events: Mutex<Vec<RuntimeEvent>>,
    fail_next_emit: AtomicBool,
}

impl CaptureRuntime {
    fn fail_next_emit(&self) {
        self.fail_next_emit.store(true, Ordering::SeqCst);
    }

    fn hints(&self) -> Vec<(String, String, String, String, i64)> {
        self.events
            .lock()
            .expect("capture runtime mutex")
            .iter()
            .filter_map(|event| {
                let RuntimeEvent::Ai { session_id, event } = event else {
                    return None;
                };
                let AiEvent::InvestigationProjectionChanged {
                    operation_id,
                    stage_execution_id,
                    stage_run_request_id,
                    change_seq,
                } = event.as_ref()
                else {
                    return None;
                };
                Some((
                    session_id.clone(),
                    operation_id.clone(),
                    stage_execution_id.clone(),
                    stage_run_request_id.clone(),
                    *change_seq,
                ))
            })
            .collect()
    }
}

#[async_trait]
impl GolishRuntime for CaptureRuntime {
    fn emit(&self, event: RuntimeEvent) -> Result<(), RuntimeError> {
        if self.fail_next_emit.swap(false, Ordering::SeqCst) {
            return Err(RuntimeError::EmitFailed(
                "simulated missed event".to_owned(),
            ));
        }
        self.events
            .lock()
            .map_err(|_| RuntimeError::EmitFailed("capture runtime mutex poisoned".to_owned()))?
            .push(event);
        Ok(())
    }

    async fn request_approval(
        &self,
        _request_id: String,
        _tool_name: String,
        _args: serde_json::Value,
        _risk_level: String,
    ) -> Result<ApprovalResult, RuntimeError> {
        Err(RuntimeError::NotInteractive)
    }

    fn is_interactive(&self) -> bool {
        false
    }

    fn auto_approve(&self) -> bool {
        false
    }

    async fn shutdown(&self) -> Result<(), RuntimeError> {
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn lazy_pool() -> Arc<PgPool> {
    Arc::new(
        PgPoolOptions::new()
            .connect_lazy("postgres://golish:golish@127.0.0.1:1/golish")
            .expect("syntactically valid lazy pool"),
    )
}

fn refresh(
    operation_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_request_id: &str,
    change_seq: i64,
) -> CommittedInvestigationProjectionRefresh {
    CommittedInvestigationProjectionRefresh {
        session_id: "chat-session-1".to_owned(),
        operation_id,
        stage_execution_id,
        stage_run_request_id: stage_run_request_id.to_owned(),
        change_seq,
    }
}

fn bridge(source: Arc<FakeReceiptSource>) -> InvestigationProjectionEventBridge {
    InvestigationProjectionEventBridge::with_receipt_source(lazy_pool(), source)
}

#[tokio::test]
async fn projection_refresh_is_not_emitted_before_committed_receipt_is_visible() {
    let source = Arc::new(FakeReceiptSource::default());
    let bridge = bridge(source.clone());
    let runtime = CaptureRuntime::default();

    let before_commit = bridge.publish_once(&runtime).await.expect("empty pass");
    assert_eq!(before_commit.emitted, 0);
    assert!(runtime.hints().is_empty());

    let operation_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    source
        .replace(vec![refresh(
            operation_id,
            stage_execution_id,
            "stage-run-request-1",
            7,
        )])
        .await;
    let after_commit = bridge.publish_once(&runtime).await.expect("committed pass");
    assert_eq!(after_commit.emitted, 1);
    assert_eq!(
        runtime.hints(),
        vec![(
            "chat-session-1".to_owned(),
            operation_id.to_string(),
            stage_execution_id.to_string(),
            "stage-run-request-1".to_owned(),
            7,
        )]
    );
}

#[tokio::test]
async fn duplicate_committed_projection_refresh_is_idempotent() {
    let operation_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    let source = Arc::new(FakeReceiptSource::default());
    source
        .replace(vec![refresh(
            operation_id,
            stage_execution_id,
            "request",
            11,
        )])
        .await;
    let bridge = bridge(source);
    let runtime = CaptureRuntime::default();

    assert_eq!(bridge.publish_once(&runtime).await.unwrap().emitted, 1);
    let replay = bridge.publish_once(&runtime).await.unwrap();
    assert_eq!(replay.emitted, 0);
    assert_eq!(replay.duplicate_or_out_of_order, 1);
    assert_eq!(runtime.hints().len(), 1);
}

#[tokio::test]
async fn out_of_order_projection_refresh_does_not_regress_watermark() {
    let operation_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    let source = Arc::new(FakeReceiptSource::default());
    let bridge = bridge(source.clone());
    let runtime = CaptureRuntime::default();

    source
        .replace(vec![refresh(
            operation_id,
            stage_execution_id,
            "request",
            12,
        )])
        .await;
    assert_eq!(bridge.publish_once(&runtime).await.unwrap().emitted, 1);
    source
        .replace(vec![refresh(
            operation_id,
            stage_execution_id,
            "request",
            9,
        )])
        .await;
    let stale = bridge.publish_once(&runtime).await.unwrap();
    assert_eq!(stale.emitted, 0);
    assert_eq!(stale.duplicate_or_out_of_order, 1);
    assert_eq!(runtime.hints()[0].4, 12);
}

#[tokio::test]
async fn missed_delivery_is_retried_without_advancing_delivery_watermark() {
    let source = Arc::new(FakeReceiptSource::default());
    source
        .replace(vec![refresh(Uuid::new_v4(), Uuid::new_v4(), "request", 3)])
        .await;
    let bridge = bridge(source);
    let runtime = CaptureRuntime::default();
    runtime.fail_next_emit();

    assert!(bridge.publish_once(&runtime).await.is_err());
    assert!(runtime.hints().is_empty());
    assert_eq!(bridge.publish_once(&runtime).await.unwrap().emitted, 1);
    assert_eq!(runtime.hints().len(), 1);
}

#[tokio::test]
async fn cold_restore_replays_latest_committed_head_once_per_process() {
    let source = Arc::new(FakeReceiptSource::default());
    source
        .replace(vec![refresh(Uuid::new_v4(), Uuid::new_v4(), "request", 21)])
        .await;
    let first_runtime = CaptureRuntime::default();
    assert_eq!(
        bridge(source.clone())
            .publish_once(&first_runtime)
            .await
            .unwrap()
            .emitted,
        1
    );

    let restored_runtime = CaptureRuntime::default();
    let restored_bridge = bridge(source);
    assert_eq!(
        restored_bridge
            .publish_once(&restored_runtime)
            .await
            .unwrap()
            .emitted,
        1
    );
    assert_eq!(
        restored_bridge
            .publish_once(&restored_runtime)
            .await
            .unwrap()
            .duplicate_or_out_of_order,
        1
    );
    assert_eq!(restored_runtime.hints().len(), 1);
}

#[tokio::test]
async fn foreign_stage_identity_for_observed_operation_is_rejected() {
    let operation_id = Uuid::new_v4();
    let source = Arc::new(FakeReceiptSource::default());
    let bridge = bridge(source.clone());
    let runtime = CaptureRuntime::default();

    source
        .replace(vec![refresh(operation_id, Uuid::new_v4(), "request-a", 1)])
        .await;
    assert_eq!(bridge.publish_once(&runtime).await.unwrap().emitted, 1);
    source
        .replace(vec![refresh(operation_id, Uuid::new_v4(), "request-b", 2)])
        .await;
    let foreign = bridge.publish_once(&runtime).await.unwrap();
    assert_eq!(foreign.emitted, 0);
    assert_eq!(foreign.foreign_identity, 1);
    assert_eq!(runtime.hints().len(), 1);
}
