use std::time::Duration;

use golish_core::{
    hypothesis_semantic_key::CanonicalJsonObject,
    investigation_projection::{
        HypothesisProjectionRecordV1, ProjectionChangeKind, ProjectionSourceSnapshotV1,
        ProjectionSourceTimeStatusV1,
    },
};
use golish_db::{
    repo::investigation_projection::{
        capture_projection_head, enqueue_projection_batch_on, read_projection_at_head,
        InvestigationProjectionWorker, ProjectionOutboxBatchInput, ProjectionOutboxMemberInput,
        ProjectionSourceStorageV1, INVESTIGATION_PROJECTION_NOTIFY_CHANNEL,
    },
    DbConfig, GolishDb,
};
use serial_test::serial;
use sqlx::{postgres::PgListener, PgPool};
use uuid::Uuid;

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read local postgres port")
        .port()
}

async fn fixture(label: &str) -> (GolishDb, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("projection_worker_{label}_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start isolated migrated postgres");
    (db, data_dir)
}

async fn seed_operation(pool: &PgPool, operation_id: Uuid) {
    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,
               attack_execution_contract,tool_truth_contract
           ) VALUES($1,'assessment','target_intel','legacy_v1','legacy','legacy_v1')"#,
    )
    .bind(operation_id)
    .execute(pool)
    .await
    .expect("insert operation");
}

fn hypothesis_source(entity_id: &str, entity_version: u64) -> ProjectionSourceSnapshotV1 {
    let body = CanonicalJsonObject::try_from_value(serde_json::json!({
        "fixture": "projection-worker",
        "entity_version": entity_version,
    }))
    .expect("canonical projection body");
    ProjectionSourceSnapshotV1::Hypothesis(
        HypothesisProjectionRecordV1::try_new(entity_id, entity_version, 1, body)
            .expect("typed projection source"),
    )
}

fn batch_input(
    operation_id: Uuid,
    entity_id: &str,
    entity_version: u64,
) -> ProjectionOutboxBatchInput {
    let batch_id = Uuid::new_v4();
    ProjectionOutboxBatchInput {
        batch_id,
        operation_id,
        project_scope_id: None,
        stable_request_id: Uuid::new_v4(),
        source_transaction_id: Uuid::new_v4(),
        source_occurred_at: None,
        source_time_status: ProjectionSourceTimeStatusV1::HistoricalUnknown,
        members: vec![ProjectionOutboxMemberInput {
            outbox_member_id: Uuid::new_v4(),
            change_kind: if entity_version == 1 {
                ProjectionChangeKind::Insert
            } else {
                ProjectionChangeKind::Supersede
            },
            source: hypothesis_source(entity_id, entity_version),
            source_occurred_at: None,
            source_time_status: ProjectionSourceTimeStatusV1::HistoricalUnknown,
            invalidation_reason: None,
            storage: ProjectionSourceStorageV1::Inline,
        }],
    }
}

async fn enqueue(pool: &PgPool, operation_id: Uuid, entity_id: &str, entity_version: u64) -> Uuid {
    let input = batch_input(operation_id, entity_id, entity_version);
    let batch_id = input.batch_id;
    let mut tx = pool.begin().await.expect("begin canonical transaction");
    enqueue_projection_batch_on(&mut tx, input)
        .await
        .expect("append immutable projection batch");
    tx.commit().await.expect("commit canonical transaction");
    batch_id
}

#[tokio::test]
#[serial]
async fn projection_notification_is_delivered_only_after_canonical_commit() {
    let (db, _data_dir) = fixture("transactional-notify").await;
    let operation_id = Uuid::new_v4();
    seed_operation(db.pool(), operation_id).await;
    let mut listener = PgListener::connect_with(db.pool())
        .await
        .expect("connect projection notification listener");
    listener
        .listen(INVESTIGATION_PROJECTION_NOTIFY_CHANNEL)
        .await
        .expect("listen for committed source batches");

    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin canonical transaction");
    enqueue_projection_batch_on(
        &mut tx,
        batch_input(operation_id, "hypothesis:opaque:notify", 1),
    )
    .await
    .expect("append source batch before commit");
    assert!(
        tokio::time::timeout(Duration::from_millis(100), listener.recv())
            .await
            .is_err(),
        "transactional notification must not escape before commit"
    );

    tx.commit().await.expect("commit canonical source batch");
    let notification = tokio::time::timeout(Duration::from_secs(2), listener.recv())
        .await
        .expect("notification after commit")
        .expect("receive committed notification");
    assert_eq!(notification.payload(), operation_id.to_string());
}

async fn wait_for_receipt(pool: &PgPool, batch_id: Uuid) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let projected: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM investigation_projection_batch_receipts WHERE batch_id=$1)",
            )
            .bind(batch_id)
            .fetch_one(pool)
            .await
            .expect("query projection receipt");
            if projected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("projection worker did not publish batch before timeout");
}

#[tokio::test]
#[serial]
async fn canonical_commit_becomes_visible_asynchronously_and_restart_drains_backlog() {
    let (db, _data_dir) = fixture("restart").await;
    let operation_id = Uuid::new_v4();
    seed_operation(db.pool(), operation_id).await;
    let worker = InvestigationProjectionWorker::new(std::sync::Arc::new(db.pool().clone()));

    assert!(worker.start().await);
    assert!(!worker.start().await, "one runtime owns one worker task");
    assert!(worker.shutdown().await);

    let entity_id = "hypothesis:opaque:restart";
    let batch_id = enqueue(db.pool(), operation_id, entity_id, 1).await;
    let old_head = capture_projection_head(db.pool(), operation_id)
        .await
        .expect("capture old projection head");
    assert_eq!(old_head.change_seq, 0);
    assert!(!sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM investigation_projection_batch_receipts WHERE batch_id=$1)",
    )
    .bind(batch_id)
    .fetch_one(db.pool())
    .await
    .expect("query stopped worker receipt"));

    assert!(worker.start().await);
    wait_for_receipt(db.pool(), batch_id).await;

    let new_head = capture_projection_head(db.pool(), operation_id)
        .await
        .expect("capture published projection head");
    assert_eq!(new_head.change_seq, 1);
    let page = read_projection_at_head(db.pool(), &new_head)
        .await
        .expect("read asynchronously published projection");
    assert_eq!(page.entities.len(), 1);
    assert_eq!(page.entities[0].entity_id, entity_id);
    assert!(worker.shutdown().await);
}

#[tokio::test]
#[serial]
async fn corrupt_operation_backlog_does_not_block_another_operation() {
    let (db, _data_dir) = fixture("failure-isolation").await;
    let invalid_operation_id = Uuid::new_v4();
    let valid_operation_id = Uuid::new_v4();
    seed_operation(db.pool(), invalid_operation_id).await;
    seed_operation(db.pool(), valid_operation_id).await;

    // Version two without version one deterministically fails the direct
    // predecessor contract and remains available for a later repair/retry.
    let invalid_batch = enqueue(
        db.pool(),
        invalid_operation_id,
        "hypothesis:opaque:invalid-predecessor",
        2,
    )
    .await;
    let valid_batch = enqueue(db.pool(), valid_operation_id, "hypothesis:opaque:valid", 1).await;

    let worker = InvestigationProjectionWorker::new(std::sync::Arc::new(db.pool().clone()));
    worker.start().await;
    wait_for_receipt(db.pool(), valid_batch).await;

    let invalid_was_published: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM investigation_projection_batch_receipts WHERE batch_id=$1)",
    )
    .bind(invalid_batch)
    .fetch_one(db.pool())
    .await
    .expect("query corrupt-operation receipt");
    assert!(!invalid_was_published);
    assert_eq!(
        capture_projection_head(db.pool(), invalid_operation_id)
            .await
            .expect("capture corrupt operation head")
            .change_seq,
        0
    );
    assert_eq!(
        capture_projection_head(db.pool(), valid_operation_id)
            .await
            .expect("capture healthy operation head")
            .change_seq,
        1
    );
    worker.shutdown().await;
}
