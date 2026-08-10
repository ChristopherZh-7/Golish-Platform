use std::{any::Any, sync::Arc};

use async_trait::async_trait;
use golish_core::runtime::{ApprovalResult, GolishRuntime, RuntimeError, RuntimeEvent};
use golish_lib::ai::{
    CommittedInvestigationProjectionRefresh, InvestigationProjectionEventBridge,
    InvestigationProjectionReceiptSource,
};
use parking_lot::Mutex;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

struct OneCommittedReceipt(CommittedInvestigationProjectionRefresh);

#[async_trait]
impl InvestigationProjectionReceiptSource for OneCommittedReceipt {
    async fn latest_committed_refreshes(
        &self,
    ) -> anyhow::Result<Vec<CommittedInvestigationProjectionRefresh>> {
        Ok(vec![self.0.clone()])
    }
}

#[derive(Default)]
struct FrontendChannel {
    events: Mutex<Vec<RuntimeEvent>>,
}

#[async_trait]
impl GolishRuntime for FrontendChannel {
    fn emit(&self, event: RuntimeEvent) -> Result<(), RuntimeError> {
        self.events.lock().push(event);
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

#[tokio::test]
async fn production_composition_emits_exact_committed_seq_once() {
    // Lock the real desktop lifecycle to one named composition seam: AppState
    // owns it, bootstrap starts it only after DB readiness, and exit drains it.
    let state_source = include_str!("../src/state/mod.rs");
    let bootstrap_source = include_str!("../src/app/bootstrap.rs");
    let lifecycle_source = include_str!("../src/app/window_lifecycle.rs");
    assert!(state_source.contains("compose_investigation_projection_event_bridge"));
    assert!(bootstrap_source.contains(".start(investigation_event_runtime)"));
    assert!(lifecycle_source.contains("investigation_projection_event_bridge.shutdown().await"));

    let operation_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    let source = Arc::new(OneCommittedReceipt(
        CommittedInvestigationProjectionRefresh {
            session_id: "frontend-session".to_owned(),
            operation_id,
            stage_execution_id,
            stage_run_request_id: "stage-run-request".to_owned(),
            change_seq: 37,
        },
    ));
    let pool = Arc::new(
        PgPoolOptions::new()
            .connect_lazy("postgres://golish:golish@127.0.0.1:1/golish")
            .expect("syntactically valid lazy pool"),
    );
    let bridge = InvestigationProjectionEventBridge::with_receipt_source(pool, source);
    let frontend = FrontendChannel::default();

    assert_eq!(bridge.publish_once(&frontend).await.unwrap().emitted, 1);
    assert_eq!(
        bridge
            .publish_once(&frontend)
            .await
            .unwrap()
            .duplicate_or_out_of_order,
        1
    );

    let events = frontend.events.lock();
    assert_eq!(events.len(), 1);
    let RuntimeEvent::Ai { session_id, event } = &events[0] else {
        panic!("expected frontend AI event");
    };
    assert_eq!(session_id, "frontend-session");
    let golish_core::events::AiEvent::InvestigationProjectionChanged {
        operation_id: emitted_operation_id,
        stage_execution_id: emitted_stage_execution_id,
        stage_run_request_id,
        change_seq,
    } = event.as_ref()
    else {
        panic!("expected Investigation projection event");
    };
    assert_eq!(emitted_operation_id, &operation_id.to_string());
    assert_eq!(emitted_stage_execution_id, &stage_execution_id.to_string());
    assert_eq!(stage_run_request_id, "stage-run-request");
    assert_eq!(*change_seq, 37);
    let serialized = serde_json::to_value(event.as_ref()).expect("serialize AI event");
    let keys = serialized
        .as_object()
        .expect("tagged AI event object")
        .keys()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        keys,
        std::collections::BTreeSet::from([
            "type",
            "operation_id",
            "stage_execution_id",
            "stage_run_request_id",
            "change_seq",
        ])
    );
}
