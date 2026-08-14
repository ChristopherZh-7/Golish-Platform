use std::borrow::Cow;

use golish_core::{
    hypothesis_semantic_key::CanonicalJsonObject,
    investigation_projection::{
        HypothesisProjectionRecordV1, ProjectionChangeKind, ProjectionSourceSnapshotV1,
        ProjectionSourceTimeStatusV1,
    },
    InvestigationContractVersion, InvestigationRolloutMode,
};
use golish_db::{embedded::EmbeddedPg, DbConfig, GolishDb};
use golish_pentest_domain::tool_truth::ToolTruthContract;
use serial_test::serial;
use sqlx::{
    migrate::Migrator,
    postgres::{PgConnection, PgPool, PgPoolOptions},
    Error as SqlxError,
};
use uuid::Uuid;

// Task 5 repository contract: these modules must be registered as public, typed ports.
// Keeping this as a compile-time dependency ensures callers never fall back to raw SQL.
use golish_db::repo::{
    attack_candidates::{
        precheck_legacy_candidate_mutation,
        ATTACK_LEGACY_MUTATION_FORBIDDEN_BY_INVESTIGATION_CONTRACT,
    },
    candidate_analysis::{
        freeze_candidate_snapshot, load_candidate_gate_material, CandidateSnapshotDispositionRow,
        FreezeCandidateSnapshotInput, LoadCandidateGateMaterialInput,
    },
    hypothesis_legacy_projection::ProjectionSourceBatchView,
    hypothesis_registry::ApplyCandidateGatePassInput,
    investigation_projection::{
        capture_projection_head, compare_and_record_v1, enqueue_projection_batch_on,
        project_projection_batch, read_projection_at_head, CompareAndRecordV1Input,
        ProjectionOutboxBatchInput, ProjectionOutboxMemberInput, ProjectionProjectOutcome,
        ProjectionSourceStorageV1,
    },
};

const PLAN_A_MIGRATION_VERSION: i64 = 20260729000005;
const PLAN_B_MIGRATION_VERSION: i64 = 20260729000006;

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read local postgres port")
        .port()
}

fn migration_subset(min_version: i64, max_version: i64) -> Migrator {
    let all = sqlx::migrate!("./migrations");
    Migrator {
        migrations: Cow::Owned(
            all.iter()
                .filter(|migration| {
                    migration.version >= min_version && migration.version <= max_version
                })
                .cloned()
                .collect(),
        ),
        ignore_missing: true,
        locking: true,
        no_tx: false,
    }
}

async fn fixture(label: &str) -> (GolishDb, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("hr_{label}_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start isolated migrated postgres");
    (db, data_dir)
}

fn assert_database_rejection(error: &SqlxError, stable_marker: &str) {
    let database_error = error
        .as_database_error()
        .unwrap_or_else(|| panic!("expected PostgreSQL database error, got {error}"));
    assert!(
        database_error.message().contains(stable_marker)
            || database_error.constraint() == Some(stable_marker),
        "expected stable marker {stable_marker}, got message={} constraint={:?}",
        database_error.message(),
        database_error.constraint()
    );
}

async fn insert_operation(pool: &PgPool, operation_id: Uuid) {
    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,
               attack_execution_contract,tool_truth_contract
           ) VALUES($1,'assessment','target_intel','legacy_v1','legacy','legacy_v1')"#,
    )
    .bind(operation_id)
    .execute(pool)
    .await
    .expect("insert legacy operation");
}

fn projection_hypothesis_source(
    entity_id: Uuid,
    entity_version: u64,
    label: &str,
) -> ProjectionSourceSnapshotV1 {
    let body = CanonicalJsonObject::try_from_value(serde_json::json!({
        "label": label,
        "entity_version": entity_version,
    }))
    .expect("canonical projection body");
    ProjectionSourceSnapshotV1::Hypothesis(
        HypothesisProjectionRecordV1::try_new(entity_id.to_string(), entity_version, 1, body)
            .expect("bounded typed projection source"),
    )
}

fn projection_batch_input(
    operation_id: Uuid,
    batch_id: Uuid,
    stable_request_id: Uuid,
    entity_id: Uuid,
    entity_version: u64,
    label: &str,
    storage: ProjectionSourceStorageV1,
) -> ProjectionOutboxBatchInput {
    ProjectionOutboxBatchInput {
        batch_id,
        operation_id,
        project_scope_id: None,
        stable_request_id,
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
            source: projection_hypothesis_source(entity_id, entity_version, label),
            source_occurred_at: None,
            source_time_status: ProjectionSourceTimeStatusV1::HistoricalUnknown,
            invalidation_reason: None,
            storage,
        }],
    }
}

async fn enqueue_projection_batch(
    pool: &PgPool,
    input: ProjectionOutboxBatchInput,
) -> golish_db::repo::investigation_projection::ProjectionBatchEnqueueReceipt {
    let mut tx = pool
        .begin()
        .await
        .expect("begin projection source transaction");
    let receipt = enqueue_projection_batch_on(&mut tx, input)
        .await
        .expect("append immutable projection source batch");
    tx.commit()
        .await
        .expect("commit projection source transaction");
    receipt
}

fn digest(nibble: char) -> String {
    format!("sha256:{}", nibble.to_string().repeat(64))
}

#[test]
fn registry_repository_modules_are_public_typed_ports() {
    // Reaching this test proves the compile-time imports above resolved through
    // `golish_db::repo`; runtime callers do not need raw SQL escape hatches.
    let ports = [
        std::any::type_name::<FreezeCandidateSnapshotInput>(),
        std::any::type_name::<ApplyCandidateGatePassInput>(),
        std::any::type_name::<ProjectionSourceBatchView>(),
    ];
    assert!(ports.iter().all(|port| port.contains("golish_db::repo")));
}

#[tokio::test]
#[serial]
async fn projection_batch_projects_atomically_and_replays_exact_once() {
    let (db, _data_dir) = fixture("projection-batch-replay").await;
    let operation_id = Uuid::new_v4();
    insert_operation(db.pool(), operation_id).await;
    let batch_id = Uuid::new_v4();
    let source = projection_batch_input(
        operation_id,
        batch_id,
        Uuid::new_v4(),
        Uuid::new_v4(),
        1,
        "initial",
        ProjectionSourceStorageV1::Inline,
    );
    let source_receipt = enqueue_projection_batch(db.pool(), source).await;
    assert_eq!(source_receipt.source_batch_seq, 1);

    let applied = project_projection_batch(db.pool(), operation_id, batch_id)
        .await
        .expect("project complete source batch");
    let first = match applied {
        ProjectionProjectOutcome::Applied(receipt) => receipt,
        other => panic!("expected applied batch, got {other:?}"),
    };
    assert_eq!((first.first_change_seq, first.last_change_seq), (1, 1));

    let replayed = project_projection_batch(db.pool(), operation_id, batch_id)
        .await
        .expect("replay projected batch");
    assert!(matches!(replayed, ProjectionProjectOutcome::Replay(_)));
    let head = capture_projection_head(db.pool(), operation_id)
        .await
        .expect("capture projection head");
    let page = read_projection_at_head(db.pool(), &head)
        .await
        .expect("read materialized projection");
    assert_eq!(head.change_seq, 1);
    assert_eq!(page.entities.len(), 1);
    assert_eq!(page.changes.len(), 1);
}

#[tokio::test]
#[serial]
async fn projection_registry_source_derives_typed_compatibility_invalidations_atomically() {
    let (db, _data_dir) = fixture("projection-registry-compatibility").await;
    for statement in [
        "ALTER TABLE tool_truth_rollout DISABLE TRIGGER tool_truth_rollout_direct_mutation_guard",
        "ALTER TABLE investigation_rollout DISABLE TRIGGER investigation_rollout_direct_mutation_guard",
    ] {
        sqlx::query(statement)
            .execute(db.pool())
            .await
            .expect("disable rollout guard in isolated compatibility fixture");
    }
    sqlx::query(
        "UPDATE tool_truth_rollout SET new_operation_contract='receipt_v1',row_version=row_version+1 WHERE singleton=TRUE",
    )
    .execute(db.pool())
    .await
    .expect("install isolated receipt Tool Truth default");
    sqlx::query(
        r#"UPDATE investigation_rollout
              SET contract_version='hypothesis_registry_v1',
                  rollout_mode='registry_authoritative_legacy_projection',
                  mode_rank=3,row_version=row_version+1
            WHERE singleton=TRUE"#,
    )
    .execute(db.pool())
    .await
    .expect("install isolated compatibility Investigation default");
    let operation_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    let project_path = format!(
        "/tmp/projection-registry-compatibility-{}",
        Uuid::new_v4().simple()
    );
    sqlx::query(
        "INSERT INTO project_scopes(project_scope_id,canonical_project_path,path_sha256) VALUES($1,$2,$3)",
    )
    .bind(project_scope_id)
    .bind(project_path)
    .bind(digest('a'))
    .execute(db.pool())
    .await
    .expect("insert compatibility project scope");
    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,
               attack_execution_contract,tool_truth_contract,project_scope_id,
               investigation_contract_version,investigation_rollout_mode
           ) VALUES($1,'assessment','target_intel','legacy_v1','legacy','receipt_v1',
                    $2,'hypothesis_registry_v1','registry_authoritative_legacy_projection')"#,
    )
    .bind(operation_id)
    .bind(project_scope_id)
    .execute(db.pool())
    .await
    .expect("insert registry-authoritative operation");

    let root_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let source = ProjectionSourceSnapshotV1::Hypothesis(
        HypothesisProjectionRecordV1::try_new(
            root_id.to_string(),
            1,
            1,
            CanonicalJsonObject::try_from_value(serde_json::json!({
                "root_id": root_id,
                "revision_id": revision_id,
                "state": "proposed",
            }))
            .expect("canonical registry hypothesis body"),
        )
        .expect("bounded registry hypothesis source"),
    );
    let batch_id = Uuid::new_v4();
    let outbox_member_id = Uuid::new_v4();
    let enqueue = enqueue_projection_batch(
        db.pool(),
        ProjectionOutboxBatchInput {
            batch_id,
            operation_id,
            project_scope_id: Some(project_scope_id),
            stable_request_id: Uuid::new_v4(),
            source_transaction_id: Uuid::new_v4(),
            source_occurred_at: None,
            source_time_status: ProjectionSourceTimeStatusV1::HistoricalUnknown,
            members: vec![ProjectionOutboxMemberInput {
                outbox_member_id,
                change_kind: ProjectionChangeKind::Insert,
                source,
                source_occurred_at: None,
                source_time_status: ProjectionSourceTimeStatusV1::HistoricalUnknown,
                invalidation_reason: None,
                storage: ProjectionSourceStorageV1::Inline,
            }],
        },
    )
    .await;
    assert!(!enqueue.replayed);
    let receipt = match project_projection_batch(db.pool(), operation_id, batch_id)
        .await
        .expect("project canonical source plus compatibility invalidations")
    {
        ProjectionProjectOutcome::Applied(receipt) => receipt,
        other => panic!("expected applied compatibility batch, got {other:?}"),
    };
    assert_eq!((receipt.first_change_seq, receipt.last_change_seq), (1, 3));
    let replay = project_projection_batch(db.pool(), operation_id, batch_id)
        .await
        .expect("replay expanded compatibility receipt");
    assert!(matches!(
        replay,
        ProjectionProjectOutcome::Replay(ref replayed) if replayed == &receipt
    ));

    let head = capture_projection_head(db.pool(), operation_id)
        .await
        .expect("capture expanded projection head");
    let page = read_projection_at_head(db.pool(), &head)
        .await
        .expect("read canonical and compatibility projections");
    assert_eq!(head.change_seq, 3);
    assert_eq!(page.entities.len(), 3);
    assert_eq!(page.changes.len(), 3);
    assert_eq!(
        page.entities
            .iter()
            .map(|entity| entity.entity_kind.as_str())
            .collect::<Vec<_>>(),
        vec![
            "hypothesis",
            "legacy_candidate_projection",
            "legacy_attempt_projection",
        ]
    );
    for compatibility in &page.entities[1..] {
        assert_eq!(compatibility.entity_id, root_id.to_string());
        assert_eq!(compatibility.entity_version, 1);
        assert_eq!(
            compatibility.invalidation_reason,
            Some(
                golish_core::investigation_projection::ProjectionInvalidationReason::LegacyProjectionDerivationFailed
            )
        );
    }
    let event_ids = page
        .changes
        .iter()
        .map(|change| change.event_id)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(event_ids.len(), 3);
    for change in &page.changes {
        assert_eq!(change.outbox_member_id, outbox_member_id);
        assert!(!change.event_id.is_nil());
        assert!(change.change_hash.starts_with("sha256:"));
    }
    let compatibility_rows: i64 = sqlx::query_scalar(
        r#"SELECT
             (SELECT COUNT(*) FROM hypothesis_legacy_candidate_projection_versions
               WHERE operation_id=$1)
           + (SELECT COUNT(*) FROM hypothesis_legacy_attempt_projection_versions
               WHERE operation_id=$1)"#,
    )
    .bind(operation_id)
    .fetch_one(db.pool())
    .await
    .expect("count authority-bound compatibility rows");
    assert_eq!(
        compatibility_rows, 0,
        "missing canonical authority cannot be forged"
    );
}

#[tokio::test]
#[serial]
async fn projection_source_response_loss_replays_exact_envelope_and_rejects_drift() {
    let (db, _data_dir) = fixture("projection-source-response-loss").await;
    let operation_id = Uuid::new_v4();
    insert_operation(db.pool(), operation_id).await;
    let input = projection_batch_input(
        operation_id,
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        1,
        "response-loss",
        ProjectionSourceStorageV1::Inline,
    );
    let first = enqueue_projection_batch(db.pool(), input.clone()).await;
    assert!(!first.replayed);
    let replay = enqueue_projection_batch(db.pool(), input.clone()).await;
    assert!(replay.replayed);
    assert_eq!(
        (
            replay.batch_id,
            replay.source_batch_seq,
            replay.member_count,
            replay.member_set_hash.as_str(),
        ),
        (
            first.batch_id,
            first.source_batch_seq,
            first.member_count,
            first.member_set_hash.as_str(),
        )
    );

    let mut drifted = input;
    drifted.project_scope_id = Some(Uuid::new_v4());
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin replay drift transaction");
    let error = enqueue_projection_batch_on(&mut tx, drifted)
        .await
        .expect_err("same stable request cannot change the source envelope");
    assert!(error
        .to_string()
        .contains("INVESTIGATION_SOURCE_BATCH_REPLAY_DRIFT"));
    tx.rollback().await.expect("rollback rejected replay drift");
}

#[tokio::test]
#[serial]
async fn comparison_record_missing_side_is_incomplete_and_replays_whole_record_only() {
    let (mut db, _data_dir) = fixture("comparison-record-incomplete").await;
    let operation_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    let project_path = format!("/tmp/comparison-{}", Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO project_scopes(project_scope_id,canonical_project_path,path_sha256) VALUES($1,$2,$3)",
    )
    .bind(project_scope_id)
    .bind(project_path)
    .bind(digest('a'))
    .execute(db.pool())
    .await
    .expect("insert comparison project scope");
    for statement in [
        "ALTER TABLE tool_truth_rollout DISABLE TRIGGER tool_truth_rollout_direct_mutation_guard",
        "ALTER TABLE investigation_rollout DISABLE TRIGGER investigation_rollout_direct_mutation_guard",
    ] {
        sqlx::query(statement)
            .execute(db.pool())
            .await
            .expect("disable rollout promotion guard in isolated comparison fixture");
    }
    sqlx::query(
        "UPDATE tool_truth_rollout SET new_operation_contract='shadow_v1',row_version=row_version+1 WHERE singleton=TRUE",
    )
    .execute(db.pool())
    .await
    .expect("install isolated shadow Tool Truth default");
    sqlx::query(
        r#"UPDATE investigation_rollout
              SET contract_version='hypothesis_registry_v1',rollout_mode='dual_read_compare',
                  mode_rank=2,row_version=row_version+1
            WHERE singleton=TRUE"#,
    )
    .execute(db.pool())
    .await
    .expect("install isolated dual-read Investigation default");
    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,
               attack_execution_contract,tool_truth_contract,project_scope_id,
               investigation_contract_version,investigation_rollout_mode
           ) VALUES($1,'assessment','target_intel','legacy_v1','legacy','shadow_v1',$2,
                    'hypothesis_registry_v1','dual_read_compare')"#,
    )
    .bind(operation_id)
    .bind(project_scope_id)
    .execute(db.pool())
    .await
    .expect("insert dual-read comparison operation");

    let input = CompareAndRecordV1Input {
        operation_id,
        organization_id: None,
        as_of_change_seq: 0,
        record_kind: "hypothesis".into(),
        record_key: "root:one".into(),
        legacy: None,
        registry: None,
    };
    let first = compare_and_record_v1(db.pool(), input.clone())
        .await
        .expect("record incomplete whole-record comparison");
    let replay = compare_and_record_v1(db.pool(), input)
        .await
        .expect("replay exact comparison sample");
    assert_eq!(first, replay);
    assert_eq!(first.comparison_state, "incomplete");
    assert_eq!(first.legacy_hash, None);
    assert_eq!(first.registry_hash, None);
    assert_eq!(
        first.diff_summary,
        serde_json::json!({
            "schema": "whole_record_comparison.v1",
            "field_fallback": false,
            "legacy_complete": false,
            "registry_complete": false,
        })
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM investigation_projection_compare_samples WHERE operation_id=$1",
    )
    .bind(operation_id)
    .fetch_one(db.pool())
    .await
    .expect("count exact comparison sample");
    assert_eq!(count, 1);

    let shadow_entity_id = Uuid::new_v4();
    let shadow_batch_id = Uuid::new_v4();
    let mut shadow_source = projection_batch_input(
        operation_id,
        shadow_batch_id,
        Uuid::new_v4(),
        shadow_entity_id,
        1,
        "legacy-shadow-source",
        ProjectionSourceStorageV1::Blob {
            redaction_contract_version: "legacy_candidate_shadow.v1".to_owned(),
        },
    );
    shadow_source.project_scope_id = Some(project_scope_id);
    enqueue_projection_batch(db.pool(), shadow_source).await;
    project_projection_batch(db.pool(), operation_id, shadow_batch_id)
        .await
        .expect("whole-batch projector invokes the comparison seam");
    let automatic: (String, serde_json::Value) = sqlx::query_as(
        r#"SELECT comparison_state,diff_summary
             FROM investigation_projection_compare_samples
            WHERE operation_id=$1 AND as_of_change_seq=1
              AND record_kind='hypothesis' AND record_key=$2"#,
    )
    .bind(operation_id)
    .bind(format!("{shadow_entity_id}:v1"))
    .fetch_one(db.pool())
    .await
    .expect("load projector-owned incomplete comparison sample");
    assert_eq!(automatic.0, "incomplete");
    assert_eq!(automatic.1["field_fallback"], false);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn projection_rebuild_stability_preserves_identity_and_canonical_manifests() {
    let (first_db, _first_data_dir) = fixture("projection-rebuild-first").await;
    let (second_db, _second_data_dir) = fixture("projection-rebuild-second").await;
    let operation_id = Uuid::new_v4();
    insert_operation(first_db.pool(), operation_id).await;
    insert_operation(second_db.pool(), operation_id).await;
    let batch_id = Uuid::new_v4();
    let source = projection_batch_input(
        operation_id,
        batch_id,
        Uuid::new_v4(),
        Uuid::new_v4(),
        1,
        "stable-rebuild",
        ProjectionSourceStorageV1::Inline,
    );
    enqueue_projection_batch(first_db.pool(), source.clone()).await;
    enqueue_projection_batch(second_db.pool(), source).await;
    let first = match project_projection_batch(first_db.pool(), operation_id, batch_id)
        .await
        .expect("project first materialization")
    {
        ProjectionProjectOutcome::Applied(receipt) => receipt,
        other => panic!("expected first applied batch, got {other:?}"),
    };
    let rebuilt = match project_projection_batch(second_db.pool(), operation_id, batch_id)
        .await
        .expect("project rebuilt materialization")
    {
        ProjectionProjectOutcome::Applied(receipt) => receipt,
        other => panic!("expected rebuilt applied batch, got {other:?}"),
    };
    assert_eq!(first.batch_id, rebuilt.batch_id);
    assert_eq!(first.source_batch_seq, rebuilt.source_batch_seq);
    assert_eq!(first.first_change_seq, rebuilt.first_change_seq);
    assert_eq!(first.last_change_seq, rebuilt.last_change_seq);
    assert_eq!(
        first.entity_version_manifest_hash,
        rebuilt.entity_version_manifest_hash
    );
    assert_eq!(first.change_manifest_hash, rebuilt.change_manifest_hash);
    assert_eq!(first.timeline_manifest_hash, rebuilt.timeline_manifest_hash);
    let first_head = capture_projection_head(first_db.pool(), operation_id)
        .await
        .expect("capture first rebuilt head");
    let rebuilt_head = capture_projection_head(second_db.pool(), operation_id)
        .await
        .expect("capture second rebuilt head");
    let first_page = read_projection_at_head(first_db.pool(), &first_head)
        .await
        .expect("read first materialization");
    let rebuilt_page = read_projection_at_head(second_db.pool(), &rebuilt_head)
        .await
        .expect("read rebuilt materialization");
    assert_eq!(first_page.entities, rebuilt_page.entities);
    let canonical_change =
        |change: &golish_db::repo::investigation_projection::InvestigationProjectionChange| {
            format!(
                "{}|{}|{}|{}|{}|{:?}|{}|{}|{:?}|{:?}|{:?}|{}|{:?}|{:?}",
                change.change_seq,
                change.event_id,
                change.batch_id,
                change.source_batch_seq,
                change.outbox_member_id,
                change.entity_kind,
                change.entity_id.clone(),
                change.entity_version,
                change.change_kind,
                change.timeline_event_kind,
                change.invalidation_reason,
                change.change_hash.clone(),
                change.source_occurred_at,
                change.source_time_status,
            )
        };
    assert_eq!(
        first_page
            .changes
            .iter()
            .map(canonical_change)
            .collect::<Vec<_>>(),
        rebuilt_page
            .changes
            .iter()
            .map(canonical_change)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
#[serial]
async fn projection_entity_predecessor_failure_rolls_back_whole_batch() {
    let (db, _data_dir) = fixture("projection-predecessor").await;
    let operation_id = Uuid::new_v4();
    let entity_id = Uuid::new_v4();
    insert_operation(db.pool(), operation_id).await;
    let first_batch = Uuid::new_v4();
    enqueue_projection_batch(
        db.pool(),
        projection_batch_input(
            operation_id,
            first_batch,
            Uuid::new_v4(),
            entity_id,
            1,
            "v1",
            ProjectionSourceStorageV1::Inline,
        ),
    )
    .await;
    project_projection_batch(db.pool(), operation_id, first_batch)
        .await
        .expect("project predecessor");

    let invalid_batch = Uuid::new_v4();
    enqueue_projection_batch(
        db.pool(),
        projection_batch_input(
            operation_id,
            invalid_batch,
            Uuid::new_v4(),
            entity_id,
            3,
            "forged-v3",
            ProjectionSourceStorageV1::Inline,
        ),
    )
    .await;
    let error = project_projection_batch(db.pool(), operation_id, invalid_batch)
        .await
        .expect_err("version three cannot skip predecessor version two");
    assert_eq!(
        error.code(),
        "INVESTIGATION_PROJECTION_ENTITY_PREDECESSOR_INVALID"
    );
    let head = capture_projection_head(db.pool(), operation_id)
        .await
        .expect("capture unchanged head");
    assert_eq!(head.change_seq, 1);
    let materialized: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM investigation_projection_entity_versions WHERE batch_id=$1",
    )
    .bind(invalid_batch)
    .fetch_one(db.pool())
    .await
    .expect("count invalid batch entity versions");
    let receipts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM investigation_projection_batch_receipts WHERE batch_id=$1",
    )
    .bind(invalid_batch)
    .fetch_one(db.pool())
    .await
    .expect("count invalid batch receipts");
    assert_eq!((materialized, receipts), (0, 0));
}

#[tokio::test]
#[serial]
async fn projection_batch_source_order_concurrent_later_worker_drives_predecessor_without_deadlock()
{
    let (db, _data_dir) = fixture("projection-source-order").await;
    let operation_id = Uuid::new_v4();
    let entity_id = Uuid::new_v4();
    insert_operation(db.pool(), operation_id).await;
    let first_batch = Uuid::new_v4();
    let second_batch = Uuid::new_v4();
    enqueue_projection_batch(
        db.pool(),
        projection_batch_input(
            operation_id,
            first_batch,
            Uuid::new_v4(),
            entity_id,
            1,
            "v1",
            ProjectionSourceStorageV1::Inline,
        ),
    )
    .await;
    enqueue_projection_batch(
        db.pool(),
        projection_batch_input(
            operation_id,
            second_batch,
            Uuid::new_v4(),
            entity_id,
            2,
            "v2",
            ProjectionSourceStorageV1::Inline,
        ),
    )
    .await;

    let (later, predecessor) = tokio::join!(
        project_projection_batch(db.pool(), operation_id, second_batch),
        project_projection_batch(db.pool(), operation_id, first_batch),
    );
    let later = later.expect("later worker completes after predecessor outside its transaction");
    let predecessor = predecessor.expect("concurrent predecessor worker completes");
    assert!(matches!(later, ProjectionProjectOutcome::Applied(_)));
    assert!(matches!(
        predecessor,
        ProjectionProjectOutcome::Applied(_) | ProjectionProjectOutcome::Replay(_)
    ));
    assert_eq!(
        capture_projection_head(db.pool(), operation_id)
            .await
            .expect("capture ordered head")
            .change_seq,
        2
    );
}

#[tokio::test]
#[serial]
async fn projection_source_snapshot_blob_is_self_contained_and_immutable() {
    let (db, _data_dir) = fixture("projection-source-blob").await;
    let operation_id = Uuid::new_v4();
    insert_operation(db.pool(), operation_id).await;
    let batch_id = Uuid::new_v4();
    enqueue_projection_batch(
        db.pool(),
        projection_batch_input(
            operation_id,
            batch_id,
            Uuid::new_v4(),
            Uuid::new_v4(),
            1,
            "blob-backed",
            ProjectionSourceStorageV1::Blob {
                redaction_contract_version: "projection_redaction.v1".into(),
            },
        ),
    )
    .await;
    let storage: (bool, bool) = sqlx::query_as(
        "SELECT immutable_source_body IS NULL,source_blob_id IS NOT NULL FROM investigation_projection_outbox WHERE batch_id=$1",
    )
    .bind(batch_id)
    .fetch_one(db.pool())
    .await
    .expect("inspect immutable blob storage");
    assert_eq!(storage, (true, true));
    project_projection_batch(db.pool(), operation_id, batch_id)
        .await
        .expect("project exclusively from outbox-owned blob");
    let head = capture_projection_head(db.pool(), operation_id)
        .await
        .expect("capture blob projection head");
    assert_eq!(
        read_projection_at_head(db.pool(), &head)
            .await
            .expect("read blob-backed entity")
            .entities
            .len(),
        1
    );
}

#[tokio::test]
#[serial]
async fn projection_head_isolation_keeps_captured_old_or_complete_new_head() {
    let (db, _data_dir) = fixture("projection-head-isolation").await;
    let operation_id = Uuid::new_v4();
    insert_operation(db.pool(), operation_id).await;
    let old_head = capture_projection_head(db.pool(), operation_id)
        .await
        .expect("capture old head");
    let batch_id = Uuid::new_v4();
    enqueue_projection_batch(
        db.pool(),
        projection_batch_input(
            operation_id,
            batch_id,
            Uuid::new_v4(),
            Uuid::new_v4(),
            1,
            "new",
            ProjectionSourceStorageV1::Inline,
        ),
    )
    .await;
    project_projection_batch(db.pool(), operation_id, batch_id)
        .await
        .expect("publish complete batch");
    let new_head = capture_projection_head(db.pool(), operation_id)
        .await
        .expect("capture new head");
    let old_page = read_projection_at_head(db.pool(), &old_head)
        .await
        .expect("old captured head remains readable");
    let new_page = read_projection_at_head(db.pool(), &new_head)
        .await
        .expect("new captured head is complete");
    assert_eq!((old_page.entities.len(), old_page.changes.len()), (0, 0));
    assert_eq!((new_page.entities.len(), new_page.changes.len()), (1, 1));
}

#[derive(Debug, Clone, Copy)]
struct CandidateAuthorityFixture {
    operation_id: Uuid,
    organization_id: Uuid,
    scope_snapshot_id: Uuid,
    asset_lane_id: Uuid,
    snapshot_id: Uuid,
    analysis_attempt_id: Uuid,
}

async fn seed_candidate_authority_fixture(pool: &PgPool, label: &str) -> CandidateAuthorityFixture {
    let operation_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    let scope_decision_id = Uuid::new_v4();
    let scope_snapshot_id = Uuid::new_v4();
    let asset_lane_id = Uuid::new_v4();
    let target_id = Uuid::new_v4();
    let project_path = format!("/tmp/{label}-{}", Uuid::new_v4().simple());
    let stages = [
        ("eas", "external_attack_surface"),
        ("enum", "enumeration"),
        ("vuln", "vuln_triage"),
    ];
    let stage_runs = stages.map(|_| Uuid::new_v4());

    sqlx::query(
        "INSERT INTO project_scopes(project_scope_id,canonical_project_path,path_sha256) \
         VALUES($1,$2,$3)",
    )
    .bind(project_scope_id)
    .bind(&project_path)
    .bind(digest('1'))
    .execute(pool)
    .await
    .expect("insert Candidate authority project scope");

    // Test-only deployment-state construction in this brand-new embedded
    // database. Restore both mutation guards before creating the operation;
    // no production promotion path is invoked or exercised here.
    let mut deployment_tx = pool
        .begin()
        .await
        .expect("begin Candidate deployment fixture");
    for statement in [
        "ALTER TABLE tool_truth_rollout DISABLE TRIGGER tool_truth_rollout_direct_mutation_guard",
        "ALTER TABLE investigation_rollout DISABLE TRIGGER investigation_rollout_direct_mutation_guard",
    ] {
        sqlx::query(statement)
            .execute(&mut *deployment_tx)
            .await
            .expect("disable Candidate deployment fixture guard");
    }
    sqlx::query(
        "UPDATE tool_truth_rollout SET new_operation_contract='receipt_v1',row_version=row_version+1 WHERE singleton=TRUE",
    )
    .execute(&mut *deployment_tx)
    .await
    .expect("install Candidate Tool Truth deployment fixture");
    sqlx::query(
        r#"UPDATE investigation_rollout
              SET contract_version='hypothesis_registry_v1',
                  rollout_mode='registry_authoritative_legacy_projection',mode_rank=3,
                  row_version=row_version+1
            WHERE singleton=TRUE"#,
    )
    .execute(&mut *deployment_tx)
    .await
    .expect("install Candidate Investigation deployment fixture");
    for statement in [
        "ALTER TABLE investigation_rollout ENABLE TRIGGER investigation_rollout_direct_mutation_guard",
        "ALTER TABLE tool_truth_rollout ENABLE TRIGGER tool_truth_rollout_direct_mutation_guard",
    ] {
        sqlx::query(statement)
            .execute(&mut *deployment_tx)
            .await
            .expect("restore Candidate deployment fixture guard");
    }
    deployment_tx
        .commit()
        .await
        .expect("commit Candidate deployment fixture");

    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,
               attack_execution_contract,tool_truth_contract,project_scope_id,
               investigation_contract_version,investigation_rollout_mode
           ) VALUES($1,'assessment','target_intel','legacy_v1','legacy','receipt_v1',$2,
                    'hypothesis_registry_v1','registry_authoritative_legacy_projection')"#,
    )
    .bind(operation_id)
    .bind(project_scope_id)
    .execute(pool)
    .await
    .expect("insert Candidate authority operation");
    sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$2,'Authority Org')")
        .bind(organization_id)
        .bind(&project_path)
        .execute(pool)
        .await
        .expect("insert Candidate authority organization");
    for ((_, stage_kind), stage_run_id) in stages.iter().zip(stage_runs) {
        sqlx::query(
            "INSERT INTO stage_runs(id,operation_id,stage_kind,status) VALUES($1,$2,$3,'started')",
        )
        .bind(stage_run_id)
        .bind(operation_id)
        .bind(*stage_kind)
        .execute(pool)
        .await
        .expect("insert Candidate authority stage run");
    }
    sqlx::query(
        r#"INSERT INTO operation_scope_decisions(
               id,operation_id,project_scope_id,stage_execution_id,
               root_organization_id,mode,decision_rows,decision_hash
           ) VALUES($1,$2,$3,$4,$5,'cli_flags',$6,$7)"#,
    )
    .bind(scope_decision_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(stage_runs[0])
    .bind(organization_id)
    .bind(serde_json::json!([{"organization_id": organization_id}]))
    .bind(digest('2'))
    .execute(pool)
    .await
    .expect("insert Candidate authority scope decision");

    let mut scope_tx = pool.begin().await.expect("begin Candidate scope seal");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_snapshots(
               id,operation_id,project_scope_id,scope_decision_id,
               project_path_at_freeze,root_organization_id,mode,scope_hash
           ) VALUES($1,$2,$3,$4,$5,$6,'cli_flags',$7)"#,
    )
    .bind(scope_snapshot_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(scope_decision_id)
    .bind(&project_path)
    .bind(organization_id)
    .bind(digest('3'))
    .execute(&mut *scope_tx)
    .await
    .expect("insert Candidate scope snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units(
               snapshot_id,organization_id,organization_name_at_freeze,
               role,depth,ordinal,decision_row_id,approval_source
           ) VALUES($1,$2,'Authority Org','root',0,0,'root',$3)"#,
    )
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(serde_json::json!({"source": "fixture"}))
    .execute(&mut *scope_tx)
    .await
    .expect("insert Candidate scope member");
    sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
        .bind(scope_snapshot_id)
        .execute(&mut *scope_tx)
        .await
        .expect("seal Candidate scope snapshot");
    scope_tx
        .commit()
        .await
        .expect("commit Candidate scope seal");

    let mut asset_tx = pool
        .begin()
        .await
        .expect("begin Candidate asset lane fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *asset_tx)
        .await
        .expect("disable fixture-only asset relationship triggers");
    sqlx::query(
        r#"INSERT INTO targets(
               id,name,target_type,value,project_path,organization_id,scope,source)
           VALUES($1,'authority.example','domain','authority.example',$2,$3,'in','fixture')"#,
    )
    .bind(target_id)
    .bind(&project_path)
    .bind(organization_id)
    .execute(&mut *asset_tx)
    .await
    .expect("insert Candidate authority live target");
    sqlx::query(
        r#"INSERT INTO investigation_asset_lanes(
               asset_lane_id,asset_queue_id,company_queue_id,company_member_id,authority_id,
               operation_id,stage_execution_id,scope_snapshot_id,organization_id,target_id,
               target_type_at_freeze,target_value_at_freeze,target_source_at_freeze,
               target_created_at,target_identity_sha256,ordinal,state,evolution_epoch,
               max_evolution_epochs)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'domain','authority.example','fixture',
                  NOW(),$11,0,'analyzing',0,2)"#,
    )
    .bind(asset_lane_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(stage_runs[2])
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(target_id)
    .bind(digest('4'))
    .execute(&mut *asset_tx)
    .await
    .expect("insert Candidate authority asset lane");
    asset_tx
        .commit()
        .await
        .expect("commit Candidate asset lane fixture");

    let bundle_id = Uuid::new_v4();
    let stable_consumer_request_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO tool_truth_authority_bundle_seals(
               id,operation_id,project_scope_id,project_path_at_freeze,
               scope_snapshot_id,organization_id,consumer_kind,stable_consumer_request_id
           ) VALUES($1,$2,$3,$4,$5,$6,'candidate_analysis',$7)"#,
    )
    .bind(bundle_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(&project_path)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(stable_consumer_request_id)
    .execute(pool)
    .await
    .expect("insert unsealed three-root execution authority bundle");

    for (ordinal, (((root_family, stage_kind), stage_run_id), hash_nibble)) in stages
        .iter()
        .zip(stage_runs)
        .zip(['5', '6', '7'])
        .enumerate()
    {
        let execution_authority_id = Uuid::new_v4();
        let authority_hash: String = sqlx::query_scalar(
            r#"INSERT INTO tool_truth_execution_authorities(
                   id,stable_authority_request_id,operation_id,project_scope_id,
                   project_path_at_freeze,scope_snapshot_id,organization_id,
                   stage_execution_id,stage_kind,execution_source_kind,
                   execution_owner_kind,authority_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'stage_execution','host_stage',$10)
               RETURNING authority_hash"#,
        )
        .bind(execution_authority_id)
        .bind(Uuid::new_v4())
        .bind(operation_id)
        .bind(project_scope_id)
        .bind(&project_path)
        .bind(scope_snapshot_id)
        .bind(organization_id)
        .bind(stage_run_id)
        .bind(*stage_kind)
        .bind(digest(hash_nibble))
        .fetch_one(pool)
        .await
        .expect("insert root execution authority");

        let denominator_id = Uuid::new_v4();
        let denominator_hash = digest(hash_nibble);
        sqlx::query(
            r#"INSERT INTO coverage_denominators(
                   id,stable_seal_request_id,execution_authority_id,operation_id,
                   project_scope_id,project_path_at_freeze,scope_snapshot_id,
                   organization_id,stage_execution_id,stage_kind,execution_authority_hash,
                   contract,input_manifest_hash,denominator_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,'receipt_v1',$12,$13)"#,
        )
        .bind(denominator_id)
        .bind(Uuid::new_v4())
        .bind(execution_authority_id)
        .bind(operation_id)
        .bind(project_scope_id)
        .bind(&project_path)
        .bind(scope_snapshot_id)
        .bind(organization_id)
        .bind(stage_run_id)
        .bind(*stage_kind)
        .bind(&authority_hash)
        .bind(digest('8'))
        .bind(&denominator_hash)
        .execute(pool)
        .await
        .expect("insert empty root denominator");
        sqlx::query("UPDATE coverage_denominators SET sealed_at=NOW() WHERE id=$1")
            .bind(denominator_id)
            .execute(pool)
            .await
            .expect("seal empty root denominator");

        let authority_set_id = Uuid::new_v4();
        let semantic_hash = digest(hash_nibble);
        let graph_hash = digest('9');
        let freshness_hash = digest('a');
        sqlx::query(
            r#"INSERT INTO tool_truth_authority_set_seals(
                   id,stable_consumer_request_id,execution_authority_id,denominator_id,
                   denominator_hash,consumer_kind,graph_hash,semantic_hash,freshness_hash
               ) VALUES($1,$2,$3,$4,$5,'candidate_analysis',$6,$7,$8)"#,
        )
        .bind(authority_set_id)
        .bind(Uuid::new_v4())
        .bind(execution_authority_id)
        .bind(denominator_id)
        .bind(&denominator_hash)
        .bind(&graph_hash)
        .bind(&semantic_hash)
        .bind(&freshness_hash)
        .execute(pool)
        .await
        .expect("insert empty authority set");
        sqlx::query("UPDATE tool_truth_authority_set_seals SET sealed_at=NOW() WHERE id=$1")
            .bind(authority_set_id)
            .execute(pool)
            .await
            .expect("seal empty authority set");

        sqlx::query(
            r#"INSERT INTO tool_truth_authority_bundle_members(
                   id,bundle_seal_id,operation_id,organization_id,ordinal,root_family,
                   root_execution_authority_id,root_denominator_id,root_denominator_hash,
                   authority_set_seal_id,authority_set_semantic_hash,
                   authority_set_graph_hash,authority_set_freshness_hash,
                   temporal_validity_policy_set_hash,target_state_epoch_set_hash,
                   observation_window_started_at,observation_window_completed_at,
                   effective_valid_until,semantic_status,temporal_validity_status,
                   member_status,member_hash
               ) VALUES(
                   $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,
                   NOW()-INTERVAL '2 minutes',NOW()-INTERVAL '1 minute',
                   NOW()+INTERVAL '10 minutes','consistent','fresh','consistent_fresh',$16
               )"#,
        )
        .bind(Uuid::new_v4())
        .bind(bundle_id)
        .bind(operation_id)
        .bind(organization_id)
        .bind(ordinal as i32)
        .bind(*root_family)
        .bind(execution_authority_id)
        .bind(denominator_id)
        .bind(&denominator_hash)
        .bind(authority_set_id)
        .bind(&semantic_hash)
        .bind(&graph_hash)
        .bind(&freshness_hash)
        .bind(digest('b'))
        .bind(digest('c'))
        .bind(digest(hash_nibble))
        .execute(pool)
        .await
        .expect("insert fresh authority bundle member");
    }
    sqlx::query("UPDATE tool_truth_authority_bundle_seals SET sealed_at=NOW() WHERE id=$1")
        .bind(bundle_id)
        .execute(pool)
        .await
        .expect("seal three-root execution authority bundle");

    let snapshot_id = Uuid::new_v4();
    let analysis_attempt_id = Uuid::new_v4();
    let mut candidate_tx = pool
        .begin()
        .await
        .expect("begin Candidate snapshot fixture");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_snapshots(
               snapshot_id,operation_id,organization_id,wave_ordinal,scope_snapshot_id,
               genesis,source_set_hash,capability_revision_hash,policy_revision_hash,
               credential_revision_hash,snapshot_status,tool_truth_authority_bundle_seal_id,
               stable_consumer_request_id,relevant_root_count,relevant_root_set_hash,
               bundle_member_count,bundle_member_set_hash,semantic_authority_bundle_hash,
               freshness_attestation_bundle_hash,temporal_validity_bundle_hash,
               temporal_validity_policy_set_hash,target_state_epoch_set_hash,
               observation_window_hash,bundle_sealed_at,candidate_snapshot_authority_hash,
               asset_lane_id
           ) SELECT $1,operation_id,organization_id,0,scope_snapshot_id,TRUE,$2,$3,$4,$5,
                    'sealed_ready',id,stable_consumer_request_id,relevant_root_count,
                    relevant_root_set_hash,member_count,member_set_hash,
                    semantic_authority_bundle_hash,freshness_attestation_bundle_hash,
                    temporal_validity_bundle_hash,temporal_validity_policy_set_hash,
                    target_state_epoch_set_hash,$6,sealed_at,$7,$9
               FROM tool_truth_authority_bundle_seals WHERE id=$8"#,
    )
    .bind(snapshot_id)
    .bind(digest('d'))
    .bind(digest('e'))
    .bind(digest('f'))
    .bind(digest('0'))
    .bind(digest('1'))
    .bind(digest('2'))
    .bind(bundle_id)
    .bind(asset_lane_id)
    .execute(&mut *candidate_tx)
    .await
    .expect("insert ready Candidate snapshot");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_snapshot_authority_bundle_members(
               snapshot_member_id,snapshot_id,operation_id,organization_id,bundle_seal_id,
               tool_truth_authority_bundle_member_id,ordinal,root_family,
               root_execution_authority_id,root_denominator_id,root_denominator_hash,
               authority_set_seal_id,authority_set_semantic_hash,authority_set_graph_hash,
               authority_set_freshness_hash,temporal_validity_policy_set_hash,
               target_state_epoch_set_hash,semantic_status,temporal_validity_status,
               member_status,member_hash
           ) SELECT gen_random_uuid(),$1,operation_id,organization_id,bundle_seal_id,id,
                    ordinal,root_family,root_execution_authority_id,root_denominator_id,
                    root_denominator_hash,authority_set_seal_id,authority_set_semantic_hash,
                    authority_set_graph_hash,authority_set_freshness_hash,
                    temporal_validity_policy_set_hash,target_state_epoch_set_hash,
                    semantic_status,temporal_validity_status,member_status,member_hash
               FROM tool_truth_authority_bundle_members WHERE bundle_seal_id=$2"#,
    )
    .bind(snapshot_id)
    .bind(bundle_id)
    .execute(&mut *candidate_tx)
    .await
    .expect("copy exact three-root Candidate snapshot members");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_attempts(
               analysis_attempt_id,snapshot_id,operation_id,organization_id,attempt_ordinal,
               attempt_input_hash,attack_class_checklist_version,
               attack_class_checklist_digest,trust_boundary_checklist_version,
               trust_boundary_checklist_digest,coverage_sampling_contract_version,
               coverage_sampling_contract_digest,retry_limit,asset_lane_id
           ) VALUES($1,$2,$3,$4,0,$5,'1',$6,'1',$7,'1',$8,1,$9)"#,
    )
    .bind(analysis_attempt_id)
    .bind(snapshot_id)
    .bind(operation_id)
    .bind(organization_id)
    .bind(digest('3'))
    .bind(digest('4'))
    .bind(digest('5'))
    .bind(digest('6'))
    .bind(asset_lane_id)
    .execute(&mut *candidate_tx)
    .await
    .expect("insert Candidate analysis attempt");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_attempt_state_events(
               attempt_event_id,analysis_attempt_id,event_ordinal,event_kind,event_hash
           ) VALUES($1,$2,0,'opened',tool_truth_sha256(jsonb_build_object(
               'attempt',$2::UUID,'ordinal',0,'event','opened'
           )::TEXT))"#,
    )
    .bind(Uuid::new_v4())
    .bind(analysis_attempt_id)
    .execute(&mut *candidate_tx)
    .await
    .expect("open Candidate analysis attempt");
    candidate_tx
        .commit()
        .await
        .expect("commit legal Candidate snapshot and attempt");

    CandidateAuthorityFixture {
        operation_id,
        organization_id,
        scope_snapshot_id,
        asset_lane_id,
        snapshot_id,
        analysis_attempt_id,
    }
}

#[derive(Debug, Clone, Copy)]
struct VerificationFactDeltaAuthorityFixture {
    fact_delta_bundle_id: Uuid,
    objective_outcome_receipt_id: Uuid,
    revision_adjudication_id: Uuid,
    revision_terminal_decision_id: Uuid,
    capability_execution_receipt_id: Uuid,
    oracle_assessment_id: Uuid,
}

async fn seed_verification_fact_delta_authority(
    pool: &PgPool,
    authority: CandidateAuthorityFixture,
) -> VerificationFactDeltaAuthorityFixture {
    let project_scope_id: Uuid =
        sqlx::query_scalar("SELECT project_scope_id FROM operation_state WHERE operation_id=$1")
            .bind(authority.operation_id)
            .fetch_one(pool)
            .await
            .expect("load Candidate fixture project scope");
    let generation_id = Uuid::new_v4();
    let generation_seal_id = Uuid::new_v4();
    let hypothesis_revision_id = Uuid::new_v4();
    let verification_plan_id = Uuid::new_v4();
    let verification_objective_id = Uuid::new_v4();
    let campaign_id = Uuid::new_v4();
    let campaign_terminal_decision_id = Uuid::new_v4();
    let campaign_adjudication_id = Uuid::new_v4();
    let campaign_coverage_receipt_id = Uuid::new_v4();
    let oracle_census_seal_id = Uuid::new_v4();
    let claim_component_outcome_seal_id = Uuid::new_v4();
    let fact_delta_bundle_id = Uuid::new_v4();
    let objective_outcome_receipt_id = Uuid::new_v4();
    let objective_outcome_set_seal_id = Uuid::new_v4();
    let revision_adjudication_id = Uuid::new_v4();
    let revision_terminal_decision_id = Uuid::new_v4();
    let terminal_successor_revision_id = Uuid::new_v4();
    let capability_execution_receipt_id = Uuid::new_v4();
    let oracle_assessment_id = Uuid::new_v4();
    let campaign_coverage_member_id = Uuid::new_v4();

    // These rows exercise the snapshot reader, not the already-covered Plan C
    // writers.  Replica mode is scoped to this one fixture connection so the
    // test can install a compact, internally consistent retained authority
    // graph without restating every upstream Campaign constructor.
    let mut connection = pool.acquire().await.expect("acquire fixture connection");
    sqlx::query("SET session_replication_role='replica'")
        .execute(&mut *connection)
        .await
        .expect("disable fixture relationship triggers");
    sqlx::query(
        r#"INSERT INTO hypothesis_generations(
               generation_id,operation_id,organization_id,generation_ordinal,
               candidate_snapshot_id,candidate_gate_decision_id,
               candidate_snapshot_authority_hash,previous_generation_id)
           VALUES($1,$2,$3,0,$4,$5,$6,NULL)"#,
    )
    .bind(generation_id)
    .bind(authority.operation_id)
    .bind(authority.organization_id)
    .bind(authority.snapshot_id)
    .bind(Uuid::new_v4())
    .bind(digest('7'))
    .execute(&mut *connection)
    .await
    .expect("insert retained generation");
    sqlx::query(
        r#"INSERT INTO hypothesis_generation_seals(
               seal_id,generation_id,member_count,member_set_hash,event_count,
               event_set_hash,open_obligation_set_hash,controller_worker_run_id,generation_hash)
           VALUES($1,$2,0,$3,0,$4,$5,$6,$7)"#,
    )
    .bind(generation_seal_id)
    .bind(generation_id)
    .bind(digest('8'))
    .bind(digest('9'))
    .bind(digest('a'))
    .bind(Uuid::new_v4())
    .bind(digest('b'))
    .execute(&mut *connection)
    .await
    .expect("seal retained generation");
    sqlx::query(
        r#"INSERT INTO verification_fact_delta_bundles(
               fact_delta_bundle_id,stable_request_id,campaign_id,
               campaign_terminal_decision_id,operation_id,project_scope_id,
               organization_id,hypothesis_revision_id,verification_objective_id,
               delta_kind,typed_delta,evidence_ref_set_hash,source_authority_hash,
               fact_delta_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'support',$10,$11,$12,$13)"#,
    )
    .bind(fact_delta_bundle_id)
    .bind(Uuid::new_v4())
    .bind(campaign_id)
    .bind(campaign_terminal_decision_id)
    .bind(authority.operation_id)
    .bind(project_scope_id)
    .bind(authority.organization_id)
    .bind(hypothesis_revision_id)
    .bind(verification_objective_id)
    .bind(serde_json::json!({
        "contract_version":"verification-fact-delta.v1",
        "objective_id":verification_objective_id,
        "outcome":"proof",
        "semantic_material_hash":{"secret":"must-also-not-enter-candidate-context"},
        "credential":"must-not-enter-candidate-context"
    }))
    .bind(digest('c'))
    .bind(digest('d'))
    .bind(digest('e'))
    .execute(&mut *connection)
    .await
    .expect("insert retained Verification FactDelta");
    sqlx::query(
        r#"INSERT INTO fact_delta_consumptions(
               fact_delta_consumption_id,stable_request_id,fact_delta_bundle_id,
               operation_id,project_scope_id,organization_id,generation_id,
               disposition,consumption_hash,residual_id)
           VALUES($1,$2,$3,$4,$5,$6,$7,'applied',$8,NULL)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(fact_delta_bundle_id)
    .bind(authority.operation_id)
    .bind(project_scope_id)
    .bind(authority.organization_id)
    .bind(generation_id)
    .bind(digest('f'))
    .execute(&mut *connection)
    .await
    .expect("consume retained Verification FactDelta");
    sqlx::query(
        r#"INSERT INTO hypothesis_objective_outcome_receipts(
               objective_outcome_receipt_id,stable_request_id,verification_plan_id,
               hypothesis_revision_id,verification_objective_id,operation_id,
               project_scope_id,organization_id,outcome_ordinal,predecessor_outcome_id,
               outcome,campaign_terminal_decision_id,campaign_adjudication_id,
               campaign_coverage_receipt_id,oracle_census_seal_id,
               claim_component_outcome_seal_id,claim_component_outcome_seal_hash,
               fact_delta_bundle_id,residual_id,source_authority_hash,outcome_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,1,NULL,'proof',$9,$10,$11,$12,$13,
                  $14,$15,NULL,$16,$17)"#,
    )
    .bind(objective_outcome_receipt_id)
    .bind(Uuid::new_v4())
    .bind(verification_plan_id)
    .bind(hypothesis_revision_id)
    .bind(verification_objective_id)
    .bind(authority.operation_id)
    .bind(project_scope_id)
    .bind(authority.organization_id)
    .bind(campaign_terminal_decision_id)
    .bind(campaign_adjudication_id)
    .bind(campaign_coverage_receipt_id)
    .bind(oracle_census_seal_id)
    .bind(claim_component_outcome_seal_id)
    .bind(digest('1'))
    .bind(fact_delta_bundle_id)
    .bind(digest('2'))
    .bind(digest('3'))
    .execute(&mut *connection)
    .await
    .expect("insert exact Objective outcome");
    sqlx::query(
        r#"INSERT INTO hypothesis_objective_outcome_set_seals(
               objective_outcome_set_seal_id,stable_request_id,verification_plan_id,
               hypothesis_revision_id,operation_id,project_scope_id,organization_id,
               cutoff_at,head_set_hash,member_count,member_set_hash,seal_hash,sealed_at)
           VALUES($1,$2,$3,$4,$5,$6,$7,NOW()-INTERVAL '1 minute',$8,1,$9,$10,NOW())"#,
    )
    .bind(objective_outcome_set_seal_id)
    .bind(Uuid::new_v4())
    .bind(verification_plan_id)
    .bind(hypothesis_revision_id)
    .bind(authority.operation_id)
    .bind(project_scope_id)
    .bind(authority.organization_id)
    .bind(digest('4'))
    .bind(digest('5'))
    .bind(digest('6'))
    .execute(&mut *connection)
    .await
    .expect("insert exact Objective outcome-set seal");
    sqlx::query(
        r#"INSERT INTO hypothesis_objective_outcome_set_members(
               objective_outcome_set_seal_id,verification_plan_id,operation_id,
               project_scope_id,organization_id,member_ordinal,
               verification_objective_id,selected_current_outcome_id,
               selected_current_ordinal,selected_current_outcome_hash,member_hash)
           VALUES($1,$2,$3,$4,$5,0,$6,$7,1,$8,$9)"#,
    )
    .bind(objective_outcome_set_seal_id)
    .bind(verification_plan_id)
    .bind(authority.operation_id)
    .bind(project_scope_id)
    .bind(authority.organization_id)
    .bind(verification_objective_id)
    .bind(objective_outcome_receipt_id)
    .bind(digest('3'))
    .bind(digest('7'))
    .execute(&mut *connection)
    .await
    .expect("bind exact Objective outcome to adjudication set");
    sqlx::query(
        r#"INSERT INTO hypothesis_revision_adjudications(
               revision_adjudication_id,stable_request_id,verification_plan_id,
               hypothesis_revision_id,objective_outcome_set_seal_id,operation_id,
               project_scope_id,organization_id,tool_truth_authority_bundle_seal_id,
               relevant_root_set_hash,member_set_hash,semantic_authority_bundle_hash,
               freshness_attestation_bundle_hash,temporal_validity_bundle_hash,
               temporal_census_hash,temporal_policy_hash,target_epoch_set_hash,
               observation_window_start,observation_window_end,effective_valid_until,
               outcome,unresolved_set_hash,adjudication_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,
                  NOW()-INTERVAL '2 minutes',NOW()-INTERVAL '1 minute',
                  NOW()+INTERVAL '10 minutes','verified',NULL,$18)"#,
    )
    .bind(revision_adjudication_id)
    .bind(Uuid::new_v4())
    .bind(verification_plan_id)
    .bind(hypothesis_revision_id)
    .bind(objective_outcome_set_seal_id)
    .bind(authority.operation_id)
    .bind(project_scope_id)
    .bind(authority.organization_id)
    .bind(Uuid::new_v4())
    .bind(digest('4'))
    .bind(digest('5'))
    .bind(digest('6'))
    .bind(digest('7'))
    .bind(digest('8'))
    .bind(digest('9'))
    .bind(digest('a'))
    .bind(digest('b'))
    .bind(digest('c'))
    .execute(&mut *connection)
    .await
    .expect("insert exact revision adjudication");
    sqlx::query(
        r#"INSERT INTO hypothesis_revision_terminal_decisions(
               revision_terminal_decision_id,stable_request_id,revision_adjudication_id,
               hypothesis_revision_id,terminal_successor_revision_id,operation_id,
               project_scope_id,organization_id,decision,finding_id,
               refutation_lineage_id,state_event_id,decision_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,'verified',$9,NULL,$10,$11)"#,
    )
    .bind(revision_terminal_decision_id)
    .bind(Uuid::new_v4())
    .bind(revision_adjudication_id)
    .bind(hypothesis_revision_id)
    .bind(terminal_successor_revision_id)
    .bind(authority.operation_id)
    .bind(project_scope_id)
    .bind(authority.organization_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(digest('d'))
    .execute(&mut *connection)
    .await
    .expect("insert exact terminal decision");
    sqlx::query(
        r#"INSERT INTO capability_execution_receipts(
               id,denominator_id,execution_authority_id,capability,attempt_ordinal,
               receipt_authority_hash,input_manifest_hash,destination_policy_id,
               destination_policy_hash,temporal_validity_policy_id,
               temporal_validity_policy_hash,attempt_state,landing_state,
               observation_state,coverage_extent,coverage_gap_reason,
               reconciliation_state,security_interpretation,typed_landing)
           VALUES($1,$2,$3,'verification_http_observation',1,$4,$5,$6,$7,$8,$9,
                  'succeeded','committed','found','sampled','none','consistent',
                  'signal',$10)"#,
    )
    .bind(capability_execution_receipt_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(digest('e'))
    .bind(digest('f'))
    .bind(Uuid::new_v4())
    .bind(digest('0'))
    .bind(Uuid::new_v4())
    .bind(digest('1'))
    .bind(serde_json::json!({"contract_version":"capability_landing.v1"}))
    .execute(&mut *connection)
    .await
    .expect("insert exact capability receipt");
    sqlx::query(
        r#"INSERT INTO verification_oracle_assessments(
               oracle_assessment_id,stable_request_id,campaign_id,prepared_action_id,
               action_execution_id,campaign_coverage_member_id,operation_id,
               project_scope_id,organization_id,oracle_revision_ordinal,
               oracle_contract_version,oracle_contract_hash,observation_receipt_hash,
               precondition_validity,control_validity,verdict,assessment_body,
               assessment_hash,residual_id)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,1,'verification_oracle.v1',$10,$11,
                  'valid','valid','proof',$12,$13,NULL)"#,
    )
    .bind(oracle_assessment_id)
    .bind(Uuid::new_v4())
    .bind(campaign_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(campaign_coverage_member_id)
    .bind(authority.operation_id)
    .bind(project_scope_id)
    .bind(authority.organization_id)
    .bind(digest('2'))
    .bind(digest('3'))
    .bind(serde_json::json!({"verdict":"proof"}))
    .bind(digest('4'))
    .execute(&mut *connection)
    .await
    .expect("insert exact Oracle assessment");
    sqlx::query(
        r#"INSERT INTO verification_campaign_coverage_results(
               campaign_coverage_receipt_id,campaign_coverage_member_id,
               coverage_disposition,epistemic_outcome,control_binding_kind,
               control_validity,prepared_action_id,capability_execution_receipt_id,
               oracle_assessment_id,residual_id,result_hash)
           VALUES($1,$2,'tested_complete','proof','required','valid',$3,$4,$5,NULL,$6)"#,
    )
    .bind(campaign_coverage_receipt_id)
    .bind(campaign_coverage_member_id)
    .bind(Uuid::new_v4())
    .bind(capability_execution_receipt_id)
    .bind(oracle_assessment_id)
    .bind(digest('5'))
    .execute(&mut *connection)
    .await
    .expect("insert exact Campaign coverage result");
    sqlx::query("SET session_replication_role='origin'")
        .execute(&mut *connection)
        .await
        .expect("restore fixture relationship triggers");

    VerificationFactDeltaAuthorityFixture {
        fact_delta_bundle_id,
        objective_outcome_receipt_id,
        revision_adjudication_id,
        revision_terminal_decision_id,
        capability_execution_receipt_id,
        oracle_assessment_id,
    }
}

#[tokio::test]
#[serial]
async fn successor_snapshot_freezes_redacted_verification_fact_delta_authority() {
    let (db, _data_dir) = fixture("verification-fact-delta-source").await;
    let authority =
        seed_candidate_authority_fixture(db.pool(), "verification-fact-delta-source").await;
    let fact_delta = seed_verification_fact_delta_authority(db.pool(), authority).await;

    let stable_consumer_request_id = Uuid::new_v4();
    let snapshot = freeze_candidate_snapshot(
        db.pool(),
        FreezeCandidateSnapshotInput {
            stable_consumer_request_id,
            operation_id: authority.operation_id,
            scope_snapshot_id: authority.scope_snapshot_id,
            organization_id: authority.organization_id,
            asset_lane_id: authority.asset_lane_id,
        },
    )
    .await
    .expect("freeze successor Candidate snapshot");
    let (source_identity, source_hash, input_content_hash, input_kind, body): (
        String,
        String,
        String,
        String,
        serde_json::Value,
    ) = sqlx::query_as(
        r#"SELECT member.source_identity,member.source_hash,input.source_content_hash,
                  input.source_kind,
                  convert_from(decode(string_agg(
                      chunk.immutable_redacted_body->>'canonical_source_fragment',''
                      ORDER BY chunk.ordinal),'hex'),'UTF8')::JSONB
             FROM candidate_analysis_snapshot_source_sets source_set
             JOIN candidate_analysis_snapshot_source_set_members member
               USING(source_set_id,snapshot_id)
             JOIN candidate_analysis_snapshot_inputs input
               ON input.snapshot_id=source_set.snapshot_id
              AND input.stable_input_key=
                  'source-set:'||source_set.source_kind||':'||member.source_identity
             JOIN candidate_analysis_input_chunk_censuses census
               ON census.snapshot_input_id=input.snapshot_input_id
             JOIN candidate_analysis_input_chunk_census_members chunk
               ON chunk.chunk_census_id=census.chunk_census_id
            WHERE source_set.snapshot_id=$1
              AND source_set.source_kind='verification_fact_deltas'
            GROUP BY member.source_identity,member.source_hash,
                     input.source_content_hash,input.source_kind"#,
    )
    .bind(snapshot.snapshot_id)
    .fetch_one(db.pool())
    .await
    .expect("read frozen Verification FactDelta source");

    assert_eq!(source_identity, fact_delta.fact_delta_bundle_id.to_string());
    assert_eq!(source_hash, input_content_hash);
    assert_eq!(input_kind, "verification_fact_deltas");
    assert_eq!(
        body["schema"],
        "candidate_verification_fact_delta_source.v1"
    );
    assert_eq!(body["instruction_authority"], false);
    assert_eq!(
        body["fact_delta"]["fact_delta_bundle_id"],
        fact_delta.fact_delta_bundle_id.to_string()
    );
    assert_eq!(body["fact_delta"]["evidence_ref_set_hash"], digest('c'));
    assert_eq!(body["fact_delta"]["source_authority_hash"], digest('d'));
    assert_eq!(body["fact_delta"]["fact_delta_hash"], digest('e'));
    assert_eq!(
        body["objective_outcome"]["objective_outcome_receipt_id"],
        fact_delta.objective_outcome_receipt_id.to_string()
    );
    assert_eq!(body["objective_outcome"]["outcome_hash"], digest('3'));
    assert_eq!(
        body["revision_adjudication"]["revision_adjudication_id"],
        fact_delta.revision_adjudication_id.to_string()
    );
    assert_eq!(
        body["revision_adjudication"]["adjudication_hash"],
        digest('c')
    );
    assert_eq!(
        body["terminal_decision"]["revision_terminal_decision_id"],
        fact_delta.revision_terminal_decision_id.to_string()
    );
    assert_eq!(body["terminal_decision"]["decision_hash"], digest('d'));
    assert_eq!(
        body["capability_execution_receipts"][0]["id"],
        fact_delta.capability_execution_receipt_id.to_string()
    );
    assert_eq!(
        body["capability_execution_receipts"][0]["receipt_authority_hash"],
        digest('e')
    );
    assert_eq!(
        body["oracle_assessments"][0]["id"],
        fact_delta.oracle_assessment_id.to_string()
    );
    assert_eq!(
        body["oracle_assessments"][0]["assessment_hash"],
        digest('4')
    );
    assert_eq!(body["typed_fact_delta"]["outcome"], "proof");
    assert!(body["typed_fact_delta"].get("credential").is_none());
    assert!(body["typed_fact_delta"]
        .get("semantic_material_hash")
        .is_none());
    assert_eq!(
        body["typed_fact_delta"]["redacted_field_names"],
        serde_json::json!(["credential", "semantic_material_hash"])
    );

    let replay = freeze_candidate_snapshot(
        db.pool(),
        FreezeCandidateSnapshotInput {
            stable_consumer_request_id,
            operation_id: authority.operation_id,
            scope_snapshot_id: authority.scope_snapshot_id,
            organization_id: authority.organization_id,
            asset_lane_id: authority.asset_lane_id,
        },
    )
    .await
    .expect("replay exact successor Candidate snapshot");
    assert_eq!(replay.snapshot_id, snapshot.snapshot_id);
    let verification_source_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM candidate_analysis_snapshot_source_set_members member
             JOIN candidate_analysis_snapshot_source_sets source_set
               USING(source_set_id,snapshot_id)
            WHERE source_set.snapshot_id=$1
              AND source_set.source_kind='verification_fact_deltas'"#,
    )
    .bind(snapshot.snapshot_id)
    .fetch_one(db.pool())
    .await
    .expect("count replayed Verification FactDelta sources");
    assert_eq!(verification_source_count, 1);
}

#[derive(Debug, Clone, Copy)]
struct HypothesisCompoundInput<'a> {
    identity_nibble: char,
    semantic_nibble: char,
    revision_nibble: char,
    epistemic_state: &'a str,
    lifecycle_state: &'a str,
    planning_readiness: &'a str,
    event_kind: &'a str,
    origin_authority: &'a str,
    authority_receipt_kind: &'a str,
}

#[tokio::test]
#[serial]
async fn snapshot_tool_truth_authority_repo_freezes_unavailable_feed_as_residuals() {
    let (db, _data_dir) = fixture("snapshot-blocked-feed").await;
    let authority = seed_candidate_authority_fixture(db.pool(), "snapshot-blocked-feed").await;
    let snapshot = freeze_candidate_snapshot(
        db.pool(),
        FreezeCandidateSnapshotInput {
            stable_consumer_request_id: Uuid::new_v4(),
            operation_id: authority.operation_id,
            scope_snapshot_id: authority.scope_snapshot_id,
            organization_id: authority.organization_id,
            asset_lane_id: authority.asset_lane_id,
        },
    )
    .await
    .expect("freeze fail-closed Candidate authority snapshot");
    assert_eq!(
        snapshot.disposition,
        CandidateSnapshotDispositionRow::SealedAnalysisReadyWithResiduals
    );
    assert_eq!(snapshot.tool_truth_authority_root_count, 3);
    assert_eq!(snapshot.authority_roots.len(), 3);
    let (attempts, inputs, feed_members, obligations): (i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT COUNT(*) FROM candidate_analysis_attempts WHERE snapshot_id=$1),
             (SELECT COUNT(*) FROM candidate_analysis_snapshot_inputs WHERE snapshot_id=$1),
             (SELECT COUNT(*) FROM candidate_analysis_knowledge_feed_snapshot_members WHERE snapshot_id=$1),
             (SELECT COUNT(*) FROM candidate_analysis_enrichment_obligations WHERE snapshot_id=$1)"#,
    )
    .bind(snapshot.snapshot_id)
    .fetch_one(db.pool())
    .await
    .expect("inspect blocked snapshot closure");
    assert_eq!((attempts, inputs, feed_members, obligations), (1, 4, 5, 5));
}

#[tokio::test]
#[serial]
async fn gate_material_rejects_unavailable_feed_before_any_canonical_or_outbox_write() {
    let (db, _data_dir) = fixture("finalizer-feed-reevaluation").await;
    let authority =
        seed_candidate_authority_fixture(db.pool(), "finalizer-feed-reevaluation").await;
    let before: (i64, i64, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT COUNT(*) FROM attack_hypotheses WHERE operation_id=$1),
             (SELECT COUNT(*) FROM hypothesis_generations WHERE operation_id=$1),
             (SELECT COUNT(*) FROM investigation_projection_outbox_batches WHERE operation_id=$1)"#,
    )
    .bind(authority.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("count canonical rows before rejected apply");
    let error = load_candidate_gate_material(
        db.pool(),
        LoadCandidateGateMaterialInput {
            operation_id: authority.operation_id,
            scope_snapshot_id: authority.scope_snapshot_id,
            organization_id: authority.organization_id,
            snapshot_id: authority.snapshot_id,
            analysis_attempt_id: authority.analysis_attempt_id,
            analysis_attempt_ordinal: 0,
            expected_snapshot_row_version: 0,
            expected_attempt_row_version: 0,
        },
    )
    .await
    .expect_err("unavailable managed feeds must fail Gate-time material reevaluation");
    let error_text = error.to_string();
    assert!(
        error_text.contains("HYPOTHESIS_REGISTRY_AUTHORITY_MISMATCH"),
        "unexpected apply rejection: {error_text}"
    );
    let after: (i64, i64, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT COUNT(*) FROM attack_hypotheses WHERE operation_id=$1),
             (SELECT COUNT(*) FROM hypothesis_generations WHERE operation_id=$1),
             (SELECT COUNT(*) FROM investigation_projection_outbox_batches WHERE operation_id=$1)"#,
    )
    .bind(authority.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("count canonical rows after rejected apply");
    assert_eq!(after, before);
}

async fn insert_hypothesis_compound(
    connection: &mut PgConnection,
    authority: CandidateAuthorityFixture,
    input: HypothesisCompoundInput<'_>,
) -> (Uuid, Uuid) {
    let root_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let semantic_key_hash = digest(input.semantic_nibble);
    let revision_hash = digest(input.revision_nibble);
    let revision_ingredients_hash = digest('d');
    let origin_decision_hash = digest('e');
    let component_id = Uuid::new_v4();
    let component_member_hash = digest('1');
    let objective_id = Uuid::new_v4();
    let objective_hash = digest('2');
    let stopping_criteria_hash = digest('3');
    let contract_id = Uuid::new_v4();
    let contract_hash = digest('4');
    let predicate_component_id = Uuid::new_v4();
    let predicate_member_hash = digest('5');
    let plan_id = Uuid::new_v4();
    let plan_objective_id = Uuid::new_v4();
    let plan_objective_member_hash = digest('6');
    let path_id = Uuid::new_v4();
    let path_hash = digest('7');
    let path_member_hash = digest('8');
    let gate_decision_id = Uuid::new_v4();
    let mutation_id = Uuid::new_v4();
    let gate_member_hash = digest('9');
    let gate_transition_hash = digest('0');

    let (predicate_set_hash, control_set_hash, pair_set_hash, ordered_set_hash): (
        String,
        String,
        String,
        String,
    ) = sqlx::query_as(
        r#"SELECT
               verification_contract_exact_member_set_hash(
                   'verification_predicate_set.v1',ARRAY[$1]::TEXT[]),
               verification_contract_exact_member_set_hash(
                   'verification_control_set.v1',ARRAY[]::TEXT[]),
               verification_contract_exact_member_set_hash(
                   'verification_paired_differential_set.v1',ARRAY[]::TEXT[]),
               verification_contract_exact_member_set_hash(
                   'verification_ordered_step_set.v1',ARRAY[]::TEXT[])"#,
    )
    .bind(&predicate_member_hash)
    .fetch_one(&mut *connection)
    .await
    .expect("derive VerificationContract exact-set hashes");
    let (
        required_component_set_hash,
        objective_component_set_hash,
        objective_set_hash,
        falsifier_set_hash,
        path_member_set_hash,
        proof_path_set_hash,
        gate_mutation_set_hash,
        gate_transition_set_hash,
    ): (
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
    ) = sqlx::query_as(
        r#"SELECT
               investigation_exact_member_set_hash(
                   'hypothesis_verification_plan_required_components.v1',ARRAY[$1]::TEXT[]),
               investigation_exact_member_set_hash(
                   'hypothesis_verification_plan_objective_components.v1',ARRAY[$1]::TEXT[]),
               investigation_exact_member_set_hash(
                   'hypothesis_verification_plan_objectives.v1',ARRAY[$2]::TEXT[]),
               investigation_exact_member_set_hash(
                   'hypothesis_verification_plan_path_falsifiers.v1',ARRAY[$1]::TEXT[]),
               investigation_exact_member_set_hash(
                   'hypothesis_verification_plan_path_members.v1',ARRAY[$3]::TEXT[]),
               investigation_exact_member_set_hash(
                   'hypothesis_verification_plan_paths.v1',ARRAY[$4]::TEXT[]),
               candidate_gate_exact_member_set_hash(
                   'candidate_mutations.v1',ARRAY[$5]::TEXT[]),
               candidate_gate_exact_member_set_hash(
                   'candidate_generation_transitions.v1',ARRAY[$6]::TEXT[])"#,
    )
    .bind(&component_member_hash)
    .bind(&plan_objective_member_hash)
    .bind(&path_member_hash)
    .bind(&path_hash)
    .bind(&gate_member_hash)
    .bind(&gate_transition_hash)
    .fetch_one(&mut *connection)
    .await
    .expect("derive HypothesisPlan and Gate exact-set hashes");

    sqlx::query(
        r#"INSERT INTO attack_hypotheses(
               root_id,operation_id,organization_id,root_kind,
               identity_ingredients,identity_ingredients_hash
           ) VALUES($1,$2,$3,'initial','{}'::JSONB,$4)"#,
    )
    .bind(root_id)
    .bind(authority.operation_id)
    .bind(authority.organization_id)
    .bind(digest(input.identity_nibble))
    .execute(&mut *connection)
    .await
    .expect("insert hypothesis root in canonical transaction");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_revisions(
               revision_id,root_id,operation_id,organization_id,revision_ordinal,
               semantic_key,semantic_key_hash,subject_kind,subject_identity_hash,
               target_type_at_time,target_value_at_time,predicate_schema,predicate_version,
               normalized_arguments,trust_boundary,polarity,epistemic_state,lifecycle_state,
               planning_readiness,structured_claim,priority,risk_impact,origin_decision_hash,
               revision_ingredients_hash,revision_hash
           ) VALUES(
               $1,$2,$3,$4,0,'{}'::JSONB,$5,'origin',$6,'domain','example.test',
               'predicate.v1',1,'{}'::JSONB,'internet','positive',$7,$8,$9,
               '{}'::JSONB,1,'{}'::JSONB,$10,$11,$12
           )"#,
    )
    .bind(revision_id)
    .bind(root_id)
    .bind(authority.operation_id)
    .bind(authority.organization_id)
    .bind(&semantic_key_hash)
    .bind(digest('a'))
    .bind(input.epistemic_state)
    .bind(input.lifecycle_state)
    .bind(input.planning_readiness)
    .bind(&origin_decision_hash)
    .bind(&revision_ingredients_hash)
    .bind(&revision_hash)
    .execute(&mut *connection)
    .await
    .expect("insert hypothesis revision in canonical transaction");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_claim_components(
               component_id,revision_id,revision_hash,component_ordinal,component_key,kind,
               canonical_fragment_hash,canonical_condition_hash,required,
               derivation_contract_version,derivation_contract_digest,member_hash
           ) VALUES($1,$2,$3,0,'claim','claim_clause',$4,$5,TRUE,1,$6,$7)"#,
    )
    .bind(component_id)
    .bind(revision_id)
    .bind(&revision_hash)
    .bind(digest('b'))
    .bind(digest('c'))
    .bind(digest('d'))
    .bind(&component_member_hash)
    .execute(&mut *connection)
    .await
    .expect("insert required claim component");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_verification_objectives(
               objective_id,revision_id,objective_ordinal,objective_intent,
               stopping_criteria,stopping_criteria_hash,objective_hash
           ) VALUES($1,$2,0,'{}'::JSONB,'{}'::JSONB,$3,$4)"#,
    )
    .bind(objective_id)
    .bind(revision_id)
    .bind(&stopping_criteria_hash)
    .bind(&objective_hash)
    .execute(&mut *connection)
    .await
    .expect("insert verification objective");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_verification_contracts(
               contract_id,revision_id,revision_hash,objective_id,combinator,
               predicate_count,predicate_set_hash,required_control_count,
               required_control_set_hash,explicit_no_required_control,
               paired_differential_count,paired_differential_set_hash,
               ordered_step_count,ordered_step_set_hash,stopping_criteria_hash,
               compiler_digest,rule_digest,policy_snapshot_hash,contract_hash
           ) VALUES(
               $1,$2,$3,$4,'all_of',1,$5,0,$6,TRUE,0,$7,0,$8,$9,$10,$11,$12,$13
           )"#,
    )
    .bind(contract_id)
    .bind(revision_id)
    .bind(&revision_hash)
    .bind(objective_id)
    .bind(&predicate_set_hash)
    .bind(&control_set_hash)
    .bind(&pair_set_hash)
    .bind(&ordered_set_hash)
    .bind(&stopping_criteria_hash)
    .bind(digest('e'))
    .bind(digest('f'))
    .bind(digest('0'))
    .bind(&contract_hash)
    .execute(&mut *connection)
    .await
    .expect("insert host-owned VerificationContract header");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_verification_predicate_components(
               predicate_component_id,contract_id,ordinal,semantic_key,predicate_schema,
               predicate_version,normalized_arguments,normalized_arguments_hash,
               expected_polarity,prerequisite_hash,member_hash
           ) VALUES($1,$2,0,'claim','predicate.v1',1,'{}'::JSONB,$3,'positive',$4,$5)"#,
    )
    .bind(predicate_component_id)
    .bind(contract_id)
    .bind(digest('1'))
    .bind(digest('2'))
    .bind(&predicate_member_hash)
    .execute(&mut *connection)
    .await
    .expect("insert VerificationContract predicate component");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_verification_objective_claim_components(
               binding_id,contract_id,revision_id,objective_id,claim_component_id,
               ordinal,component_member_hash,binding_member_hash
           ) VALUES($1,$2,$3,$4,$5,0,$6,$7)"#,
    )
    .bind(Uuid::new_v4())
    .bind(contract_id)
    .bind(revision_id)
    .bind(objective_id)
    .bind(component_id)
    .bind(&component_member_hash)
    .bind(digest('3'))
    .execute(&mut *connection)
    .await
    .expect("bind objective to required claim component");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_verification_plans(
               plan_id,revision_id,revision_hash,revision_ingredients_hash,
               required_claim_component_count,required_claim_component_set_hash,
               objective_count,objective_set_hash,proof_path_count,proof_path_set_hash,
               outer_aggregation_policy_version,outer_aggregation_policy_digest,
               plan_hash,sealed_at
           ) VALUES($1,$2,$3,$4,1,$5,1,$6,1,$7,1,$8,$9,NOW())"#,
    )
    .bind(plan_id)
    .bind(revision_id)
    .bind(&revision_hash)
    .bind(&revision_ingredients_hash)
    .bind(&required_component_set_hash)
    .bind(&objective_set_hash)
    .bind(&proof_path_set_hash)
    .bind(digest('4'))
    .bind(digest('5'))
    .execute(&mut *connection)
    .await
    .expect("insert immutable verification plan header");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_verification_plan_objectives(
               plan_objective_id,plan_id,revision_id,objective_id,
               verification_contract_id,ordinal,objective_hash,
               verification_contract_version,verification_contract_hash,
               claim_component_count,claim_component_set_hash,stopping_criteria_hash,
               outcome_requirement,member_hash
           ) VALUES($1,$2,$3,$4,$5,0,$6,1,$7,1,$8,$9,
                    'satisfy_or_falsify_bound_required_components',$10)"#,
    )
    .bind(plan_objective_id)
    .bind(plan_id)
    .bind(revision_id)
    .bind(objective_id)
    .bind(contract_id)
    .bind(&objective_hash)
    .bind(&contract_hash)
    .bind(&objective_component_set_hash)
    .bind(&stopping_criteria_hash)
    .bind(&plan_objective_member_hash)
    .execute(&mut *connection)
    .await
    .expect("insert verification plan objective");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_verification_plan_paths(
               path_id,plan_id,path_ordinal,path_key,member_count,member_set_hash,path_hash
           ) VALUES($1,$2,0,'primary',1,$3,$4)"#,
    )
    .bind(path_id)
    .bind(plan_id)
    .bind(&path_member_set_hash)
    .bind(&path_hash)
    .execute(&mut *connection)
    .await
    .expect("insert verification plan proof path");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_verification_plan_path_members(
               path_member_id,path_id,plan_id,plan_objective_id,
               plan_objective_member_hash,revision_id,member_ordinal,
               verification_contract_hash,claim_component_set_hash,role,
               falsifier_claim_component_member_hashes,falsifier_claim_component_count,
               falsifier_claim_component_set_hash,member_hash
           ) VALUES($1,$2,$3,$4,$5,$6,0,$7,$8,
                    'required_proof_and_path_falsifier',ARRAY[$9]::TEXT[],1,$10,$11)"#,
    )
    .bind(Uuid::new_v4())
    .bind(path_id)
    .bind(plan_id)
    .bind(plan_objective_id)
    .bind(&plan_objective_member_hash)
    .bind(revision_id)
    .bind(&contract_hash)
    .bind(&objective_component_set_hash)
    .bind(&component_member_hash)
    .bind(&falsifier_set_hash)
    .bind(&path_member_hash)
    .execute(&mut *connection)
    .await
    .expect("insert verification plan path member");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_heads(
               root_id,operation_id,organization_id,head_revision_id,head_revision_hash,
               head_semantic_key_hash,head_epistemic_state,head_lifecycle_state
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
    )
    .bind(root_id)
    .bind(authority.operation_id)
    .bind(authority.organization_id)
    .bind(revision_id)
    .bind(&revision_hash)
    .bind(&semantic_key_hash)
    .bind(input.epistemic_state)
    .bind(input.lifecycle_state)
    .execute(&mut *connection)
    .await
    .expect("insert current hypothesis head");
    sqlx::query(
        r#"INSERT INTO hypothesis_candidate_gate_decisions(
               decision_id,stable_request_id,operation_id,organization_id,
               candidate_snapshot_id,analysis_attempt_id,mutation_count,
               mutation_set_hash,generation_transition_count,generation_transition_set_hash,
               gate_authority_hash,decision_hash
           ) VALUES($1,$2,$3,$4,$5,$6,1,$7,1,$8,$9,$10)"#,
    )
    .bind(gate_decision_id)
    .bind(Uuid::new_v4())
    .bind(authority.operation_id)
    .bind(authority.organization_id)
    .bind(authority.snapshot_id)
    .bind(authority.analysis_attempt_id)
    .bind(&gate_mutation_set_hash)
    .bind(&gate_transition_set_hash)
    .bind(digest('a'))
    .bind(digest('b'))
    .execute(&mut *connection)
    .await
    .expect("insert Candidate Gate decision header");
    sqlx::query(
        r#"INSERT INTO hypothesis_candidate_gate_decision_members(
               mutation_id,decision_id,operation_id,organization_id,ordinal,route_kind,
               root_id,successor_revision_id,semantic_key_hash,successor_epistemic_state,
               origin_decision_hash,generation_transition_hash,member_hash
           ) VALUES($1,$2,$3,$4,0,'create_initial',$5,$6,$7,'proposed',$8,$9,$10)"#,
    )
    .bind(mutation_id)
    .bind(gate_decision_id)
    .bind(authority.operation_id)
    .bind(authority.organization_id)
    .bind(root_id)
    .bind(revision_id)
    .bind(&semantic_key_hash)
    .bind(&origin_decision_hash)
    .bind(&gate_transition_hash)
    .bind(&gate_member_hash)
    .execute(&mut *connection)
    .await
    .expect("insert Candidate Gate decision member");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_state_events(
               event_id,operation_id,organization_id,root_id,successor_revision_id,
               event_kind,origin_authority,successor_epistemic_state,
               authority_receipt_kind,authority_receipt_id,authority_receipt_hash,
               event_hash,server_decision_id,server_decision_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$10,$11)"#,
    )
    .bind(Uuid::new_v4())
    .bind(authority.operation_id)
    .bind(authority.organization_id)
    .bind(root_id)
    .bind(revision_id)
    .bind(input.event_kind)
    .bind(input.origin_authority)
    .bind(input.epistemic_state)
    .bind(input.authority_receipt_kind)
    .bind(mutation_id)
    .bind(&origin_decision_hash)
    .bind(digest('c'))
    .execute(&mut *connection)
    .await
    .expect("insert exact creating state event");

    (root_id, revision_id)
}

#[test]
fn operation_repository_joint_contract_rank_is_closed() {
    use golish_db::repo::operation_rollout::joint_contract_rank;

    assert_eq!(
        joint_contract_rank(
            ToolTruthContract::LegacyV1,
            InvestigationContractVersion::LegacyCandidateV1,
            InvestigationRolloutMode::LegacyOnly,
        ),
        Some(0)
    );
    assert_eq!(
        joint_contract_rank(
            ToolTruthContract::ReceiptV1,
            InvestigationContractVersion::HypothesisRegistryV1,
            InvestigationRolloutMode::NewOnly,
        ),
        Some(6)
    );
    assert_eq!(
        joint_contract_rank(
            ToolTruthContract::LegacyV1,
            InvestigationContractVersion::HypothesisRegistryV1,
            InvestigationRolloutMode::ShadowRegistry,
        ),
        None
    );
}

#[tokio::test]
#[serial]
async fn legacy_mutation_guard_uses_operation_frozen_mode_not_deployment_default() {
    let (mut db, _data_dir) = fixture("legacy_mutation_frozen_mode").await;
    let runtime_contract: String =
        sqlx::query_scalar("SELECT contract FROM runtime_memory_rollout WHERE singleton_id=1")
            .fetch_one(db.pool())
            .await
            .expect("read compatible runtime deployment contract");
    let legacy_operation_id = Uuid::new_v4();
    golish_db::repo::operation_state::insert(
        db.pool(),
        legacy_operation_id,
        "assessment",
        "target_intel",
        &runtime_contract,
        golish_core::ApplicationModelContract::LegacyNoModel,
    )
    .await
    .expect("freeze legacy operation mode");

    for statement in [
        "ALTER TABLE tool_truth_rollout DISABLE TRIGGER tool_truth_rollout_direct_mutation_guard",
        "ALTER TABLE investigation_rollout DISABLE TRIGGER investigation_rollout_direct_mutation_guard",
    ] {
        sqlx::query(statement)
            .execute(db.pool())
            .await
            .expect("disable immutable rollout guard in isolated fixture");
    }
    let mut promotion = db
        .pool()
        .begin()
        .await
        .expect("begin isolated default change");
    sqlx::query(
        "UPDATE tool_truth_rollout SET new_operation_contract='receipt_v1',row_version=row_version+1 WHERE singleton=TRUE",
    )
    .execute(&mut *promotion)
    .await
    .expect("set isolated Tool Truth default");
    sqlx::query(
        r#"UPDATE investigation_rollout
              SET contract_version='hypothesis_registry_v1',
                  rollout_mode='registry_authoritative_legacy_projection',
                  mode_rank=3,row_version=row_version+1
            WHERE singleton=TRUE"#,
    )
    .execute(&mut *promotion)
    .await
    .expect("set isolated Investigation default");
    promotion
        .commit()
        .await
        .expect("commit isolated complete joint pair");

    precheck_legacy_candidate_mutation(db.pool(), legacy_operation_id)
        .await
        .expect("legacy operation remains mutable after defaults change");

    let registry_operation_id = Uuid::new_v4();
    golish_db::repo::operation_state::insert(
        db.pool(),
        registry_operation_id,
        "assessment",
        "target_intel",
        &runtime_contract,
        golish_core::ApplicationModelContract::LegacyNoModel,
    )
    .await
    .expect("freeze registry-authoritative operation mode");
    let forbidden = precheck_legacy_candidate_mutation(db.pool(), registry_operation_id)
        .await
        .expect_err("registry-authoritative operation rejects legacy mutation");
    assert!(forbidden
        .to_string()
        .starts_with(ATTACK_LEGACY_MUTATION_FORBIDDEN_BY_INVESTIGATION_CONTRACT));

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn operation_repository_freezes_and_resumes_complete_joint_pair() {
    let (mut db, _data_dir) = fixture("operation_repo_freeze").await;
    let operation_id = Uuid::new_v4();
    let runtime_contract: String =
        sqlx::query_scalar("SELECT contract FROM runtime_memory_rollout WHERE singleton_id=1")
            .fetch_one(db.pool())
            .await
            .expect("read compatible runtime deployment contract");

    golish_db::repo::operation_state::insert(
        db.pool(),
        operation_id,
        "assessment",
        "target_intel",
        &runtime_contract,
        golish_core::ApplicationModelContract::LegacyNoModel,
    )
    .await
    .expect("freeze deployment defaults at operation creation");

    let created = golish_db::repo::operation_state::get(db.pool(), operation_id)
        .await
        .expect("load created operation")
        .expect("created operation exists");
    assert_eq!(created.tool_truth_contract, "legacy_v1");
    assert_eq!(
        created.investigation_contract_version,
        "legacy_candidate_v1"
    );
    assert_eq!(created.investigation_rollout_mode, "legacy_only");

    let resumed = golish_db::repo::operation_state::get(db.pool(), operation_id)
        .await
        .expect("resume operation")
        .expect("resumed operation exists");
    assert_eq!(
        (
            resumed.tool_truth_contract,
            resumed.investigation_contract_version,
            resumed.investigation_rollout_mode,
        ),
        (
            created.tool_truth_contract,
            created.investigation_contract_version,
            created.investigation_rollout_mode,
        )
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn operation_repository_concurrent_default_commit_never_freezes_torn_pair() {
    let (mut db, _data_dir) = fixture("operation_repo_concurrent").await;
    let runtime_contract: String =
        sqlx::query_scalar("SELECT contract FROM runtime_memory_rollout WHERE singleton_id=1")
            .fetch_one(db.pool())
            .await
            .expect("read compatible runtime deployment contract");
    let old_operation_id = Uuid::new_v4();
    golish_db::repo::operation_state::insert(
        db.pool(),
        old_operation_id,
        "assessment",
        "target_intel",
        &runtime_contract,
        golish_core::ApplicationModelContract::LegacyNoModel,
    )
    .await
    .expect("freeze the old complete joint pair");

    for statement in [
        "ALTER TABLE tool_truth_rollout DISABLE TRIGGER tool_truth_rollout_direct_mutation_guard",
        "ALTER TABLE investigation_rollout DISABLE TRIGGER investigation_rollout_direct_mutation_guard",
    ] {
        sqlx::query(statement)
            .execute(db.pool())
            .await
            .expect("disable immutable rollout guard in isolated fixture");
    }

    let promoter_pool = db.pool().clone();
    let (tool_locked_tx, tool_locked_rx) = tokio::sync::oneshot::channel();
    let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
    let promoter = tokio::spawn(async move {
        let mut tx = promoter_pool
            .begin()
            .await
            .expect("begin simulated promotion");
        sqlx::query(
            r#"UPDATE tool_truth_rollout
                  SET new_operation_contract='shadow_v1',row_version=row_version+1,updated_at=NOW()
                WHERE singleton=TRUE"#,
        )
        .execute(&mut *tx)
        .await
        .expect("lock and update Tool Truth first");
        tool_locked_tx.send(()).expect("signal Tool Truth lock");
        finish_rx.await.expect("release simulated promotion");
        sqlx::query(
            r#"UPDATE investigation_rollout
                  SET contract_version='hypothesis_registry_v1',rollout_mode='shadow_registry',
                      mode_rank=1,row_version=row_version+1,updated_at=NOW()
                WHERE singleton=TRUE"#,
        )
        .execute(&mut *tx)
        .await
        .expect("update Investigation second");
        tx.commit().await.expect("commit complete simulated pair");
    });
    tool_locked_rx
        .await
        .expect("observe simulated promotion between singleton writes");

    let creator_pool = db.pool().clone();
    let creator_runtime_contract = runtime_contract.clone();
    let new_operation_id = Uuid::new_v4();
    let mut creator = tokio::spawn(async move {
        golish_db::repo::operation_state::insert(
            &creator_pool,
            new_operation_id,
            "assessment",
            "target_intel",
            &creator_runtime_contract,
            golish_core::ApplicationModelContract::LegacyNoModel,
        )
        .await
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(150), &mut creator)
            .await
            .is_err(),
        "creator must wait for the Tool Truth share lock, not read a torn pair"
    );
    finish_tx
        .send(())
        .expect("finish simulated complete deployment commit");
    promoter.await.expect("join simulated promoter");
    creator
        .await
        .expect("join concurrent operation creator")
        .expect("create from the complete new pair");

    let old = golish_db::repo::operation_state::get(db.pool(), old_operation_id)
        .await
        .expect("read old operation")
        .expect("old operation exists");
    assert_eq!(old.tool_truth_contract, "legacy_v1");
    assert_eq!(old.investigation_contract_version, "legacy_candidate_v1");
    assert_eq!(old.investigation_rollout_mode, "legacy_only");
    let new = golish_db::repo::operation_state::get(db.pool(), new_operation_id)
        .await
        .expect("read new operation")
        .expect("new operation exists");
    assert_eq!(new.tool_truth_contract, "shadow_v1");
    assert_eq!(new.investigation_contract_version, "hypothesis_registry_v1");
    assert_eq!(new.investigation_rollout_mode, "shadow_registry");

    for statement in [
        "ALTER TABLE investigation_rollout ENABLE TRIGGER investigation_rollout_direct_mutation_guard",
        "ALTER TABLE tool_truth_rollout ENABLE TRIGGER tool_truth_rollout_direct_mutation_guard",
    ] {
        sqlx::query(statement)
            .execute(db.pool())
            .await
            .expect("restore immutable rollout guard");
    }
    db.stop().await;
}

async fn assert_tables_exist(pool: &PgPool, tables: &[&str]) {
    for table in tables {
        let exists: Option<String> = sqlx::query_scalar("SELECT to_regclass($1)::text")
            .bind(table)
            .fetch_one(pool)
            .await
            .unwrap_or_else(|error| panic!("inspect table {table}: {error}"));
        assert_eq!(exists.as_deref(), Some(*table), "missing table {table}");
    }
}

#[tokio::test]
#[serial]
async fn hypothesis_registry_schema_defaults_existing_operations_to_legacy() {
    let data_dir = tempfile::tempdir().expect("temporary upgrade postgres directory");
    let config = DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("hypothesis_upgrade_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    };
    let connection_string = config.connection_string();
    let mut embedded = EmbeddedPg::start(config)
        .await
        .expect("start pre-Plan-B embedded postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&connection_string)
        .await
        .expect("connect pre-Plan-B pool");
    migration_subset(i64::MIN, PLAN_A_MIGRATION_VERSION)
        .run(&pool)
        .await
        .expect("apply migrations through Plan A");

    let operation_id = Uuid::new_v4();
    insert_operation(&pool, operation_id).await;

    migration_subset(PLAN_B_MIGRATION_VERSION, PLAN_B_MIGRATION_VERSION)
        .run(&pool)
        .await
        .expect("apply the unique Plan B migration");

    let defaults: (String, String, i16) = sqlx::query_as(
        "SELECT contract_version,rollout_mode,mode_rank FROM investigation_rollout WHERE singleton=TRUE",
    )
    .fetch_one(&pool)
    .await
    .expect("read investigation rollout singleton");
    assert_eq!(
        defaults,
        ("legacy_candidate_v1".into(), "legacy_only".into(), 0)
    );

    let frozen: (String, String) = sqlx::query_as(
        "SELECT investigation_contract_version,investigation_rollout_mode FROM operation_state WHERE operation_id=$1",
    )
    .bind(operation_id)
    .fetch_one(&pool)
    .await
    .expect("read historical operation backfill");
    assert_eq!(frozen, ("legacy_candidate_v1".into(), "legacy_only".into()));

    let head_counts: (i64, i64) = sqlx::query_as(
        r#"SELECT
               (SELECT COUNT(*) FROM investigation_projection_source_heads WHERE operation_id=$1),
               (SELECT COUNT(*) FROM investigation_projection_heads WHERE operation_id=$1)"#,
    )
    .bind(operation_id)
    .fetch_one(&pool)
    .await
    .expect("read exact projection heads");
    assert_eq!(head_counts, (1, 1));

    pool.close().await;
    embedded.stop().await;
}

#[tokio::test]
#[serial]
async fn candidate_headers_freeze_snapshot_and_exact_member_sets_after_commit() {
    let (mut db, _data_dir) = fixture("candidate_header_freeze").await;
    let mut connection = db
        .pool()
        .acquire()
        .await
        .expect("acquire isolated connection");
    let snapshot_id = Uuid::new_v4();
    let attempt_id = Uuid::new_v4();
    let proposal_census_id = Uuid::new_v4();
    let subreview_census_id = Uuid::new_v4();
    let critic_census_id = Uuid::new_v4();
    let conflict_component_id = Uuid::new_v4();
    let input_id = Uuid::new_v4();
    let digest = format!("sha256:{}", "a".repeat(64));
    sqlx::query("SET session_replication_role='replica'")
        .execute(&mut *connection)
        .await
        .expect("disable fixture relationship triggers");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_snapshots(
               snapshot_id,operation_id,organization_id,wave_ordinal,scope_snapshot_id,
               genesis,source_set_hash,capability_revision_hash,policy_revision_hash,
               credential_revision_hash,snapshot_status,tool_truth_authority_bundle_seal_id,
               stable_consumer_request_id,relevant_root_count,relevant_root_set_hash,
               bundle_member_count,bundle_member_set_hash,semantic_authority_bundle_hash,
               freshness_attestation_bundle_hash,temporal_validity_bundle_hash,
               temporal_validity_policy_set_hash,target_state_epoch_set_hash,
               observation_window_hash,bundle_sealed_at,candidate_snapshot_authority_hash)
           VALUES($1,$2,$3,0,$4,TRUE,$5,$5,$5,$5,'sealed_ready',$6,$7,4,$5,4,$5,
                  $5,$5,$5,$5,$5,$5,statement_timestamp(),$5)"#,
    )
    .bind(snapshot_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(&digest)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .execute(&mut *connection)
    .await
    .expect("seed committed snapshot header");
    sqlx::query(
        "INSERT INTO candidate_analysis_proposal_censuses VALUES($1,$2,1,$3,$3,statement_timestamp())",
    )
    .bind(proposal_census_id)
    .bind(attempt_id)
    .bind(&digest)
    .execute(&mut *connection)
    .await
    .expect("seed committed proposal census header");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_hypothesis_coverage_subreview_censuses(
               subreview_census_id,analysis_attempt_id,snapshot_input_id,
               checklist_member_count,checklist_member_set_hash,chunk_partition_count,
               chunk_partition_set_hash,expected_member_count,member_set_hash,census_hash)
           VALUES($1,$2,$3,1,$4,1,$4,1,$4,$4)"#,
    )
    .bind(subreview_census_id)
    .bind(attempt_id)
    .bind(input_id)
    .bind(&digest)
    .execute(&mut *connection)
    .await
    .expect("seed committed subreview census header");
    sqlx::query(
        "INSERT INTO candidate_analysis_critic_censuses VALUES($1,$2,1,$3,$3,statement_timestamp())",
    )
    .bind(critic_census_id)
    .bind(attempt_id)
    .bind(&digest)
    .execute(&mut *connection)
    .await
    .expect("seed committed critic census header");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_conflict_components(
               conflict_component_id,analysis_attempt_id,ordinal,proposal_count,
               proposal_set_hash,component_hash) VALUES($1,$2,0,1,$3,$3)"#,
    )
    .bind(conflict_component_id)
    .bind(attempt_id)
    .bind(&digest)
    .execute(&mut *connection)
    .await
    .expect("seed committed conflict component header");
    sqlx::query("SET session_replication_role='origin'")
        .execute(&mut *connection)
        .await
        .expect("restore all guards");

    let snapshot_error = sqlx::query(
        r#"INSERT INTO candidate_analysis_snapshot_source_sets(
               source_set_id,snapshot_id,source_kind,member_count,member_set_hash,sealed_empty)
           VALUES($1,$2,'relations',0,$3,TRUE)"#,
    )
    .bind(Uuid::new_v4())
    .bind(snapshot_id)
    .bind(&digest)
    .execute(&mut *connection)
    .await
    .expect_err("committed snapshot cannot accept a late child");
    assert_database_rejection(&snapshot_error, "FROZEN");

    let proposal_error = sqlx::query(
        r#"INSERT INTO candidate_analysis_proposal_census_members(
               census_member_id,proposal_census_id,analysis_attempt_id,proposal_id,
               ordinal,proposal_hash,member_hash) VALUES($1,$2,$3,$4,0,$5,$5)"#,
    )
    .bind(Uuid::new_v4())
    .bind(proposal_census_id)
    .bind(attempt_id)
    .bind(Uuid::new_v4())
    .bind(&digest)
    .execute(&mut *connection)
    .await
    .expect_err("committed proposal census cannot accept a late member");
    assert_database_rejection(&proposal_error, "FROZEN");

    let subreview_error = sqlx::query(
        r#"INSERT INTO candidate_analysis_hypothesis_coverage_subreview_census_members(
               subreview_census_member_id,subreview_census_id,analysis_attempt_id,
               snapshot_input_id,checklist_member_id,chunk_partition_id,checklist_ordinal,
               partition_ordinal,designated_stage_work_item_id,disposition,member_hash)
           VALUES($1,$2,$3,$4,$5,$6,0,0,$7,'required',$8)"#,
    )
    .bind(Uuid::new_v4())
    .bind(subreview_census_id)
    .bind(attempt_id)
    .bind(input_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(&digest)
    .execute(&mut *connection)
    .await
    .expect_err("committed subreview census cannot accept a late member");
    assert_database_rejection(&subreview_error, "FROZEN");

    let critic_error = sqlx::query(
        r#"INSERT INTO candidate_analysis_critic_census_members(
               critic_member_id,critic_census_id,analysis_attempt_id,ordinal,member_kind,
               source_identity,source_hash,member_hash)
           VALUES($1,$2,$3,0,'proposal_conflict_review',$4,$5,$5)"#,
    )
    .bind(Uuid::new_v4())
    .bind(critic_census_id)
    .bind(attempt_id)
    .bind(Uuid::new_v4())
    .bind(&digest)
    .execute(&mut *connection)
    .await
    .expect_err("committed critic census cannot accept a late member");
    assert_database_rejection(&critic_error, "FROZEN");

    let conflict_error = sqlx::query(
        r#"INSERT INTO candidate_analysis_conflict_component_members(
               conflict_member_id,conflict_component_id,analysis_attempt_id,
               proposal_id,ordinal,member_hash) VALUES($1,$2,$3,$4,0,$5)"#,
    )
    .bind(Uuid::new_v4())
    .bind(conflict_component_id)
    .bind(attempt_id)
    .bind(Uuid::new_v4())
    .bind(&digest)
    .execute(&mut *connection)
    .await
    .expect_err("committed conflict component cannot accept a late member");
    assert_database_rejection(&conflict_error, "FROZEN");
    drop(connection);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn hypothesis_registry_schema_joint_pairs_and_operation_freeze_are_closed() {
    let (mut db, _data_dir) = fixture("joint_pairs").await;
    let ranks: Vec<Option<i16>> = sqlx::query_scalar(
        r#"SELECT operation_joint_contract_rank(tool_truth,contract_version,rollout_mode)
           FROM (VALUES
             ('legacy_v1','legacy_candidate_v1','legacy_only'),
             ('shadow_v1','legacy_candidate_v1','legacy_only'),
             ('shadow_v1','hypothesis_registry_v1','shadow_registry'),
             ('shadow_v1','hypothesis_registry_v1','dual_read_compare'),
             ('receipt_v1','hypothesis_registry_v1','dual_read_compare'),
             ('receipt_v1','hypothesis_registry_v1','registry_authoritative_legacy_projection'),
             ('receipt_v1','hypothesis_registry_v1','new_only'),
             ('legacy_v1','hypothesis_registry_v1','shadow_registry')
           ) AS pairs(tool_truth,contract_version,rollout_mode)"#,
    )
    .fetch_all(db.pool())
    .await
    .expect("evaluate the single joint-pair function");
    assert_eq!(
        ranks,
        vec![
            Some(0),
            Some(1),
            Some(2),
            Some(3),
            Some(4),
            Some(5),
            Some(6),
            None
        ]
    );

    let operation_id = Uuid::new_v4();
    insert_operation(db.pool(), operation_id).await;
    let immutable = sqlx::query(
        "UPDATE operation_state SET investigation_rollout_mode='shadow_registry' WHERE operation_id=$1",
    )
    .bind(operation_id)
    .execute(db.pool())
    .await
    .expect_err("operation-frozen investigation mode must reject mutation");
    assert_database_rejection(&immutable, "OPERATION_INVESTIGATION_CONTRACT_IMMUTABLE");

    let invalid_id = Uuid::new_v4();
    let invalid = sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,
               attack_execution_contract,tool_truth_contract,
               investigation_contract_version,investigation_rollout_mode
           ) VALUES($1,'assessment','target_intel','legacy_v1','legacy','shadow_v1',
                    'legacy_candidate_v1','legacy_only')"#,
    )
    .bind(invalid_id)
    .execute(db.pool())
    .await
    .expect_err("a legal but neither deployed nor adopted pair must fail at the database boundary");
    assert_database_rejection(&invalid, "operation_joint_contract_not_deployed_or_adopted");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn hypothesis_registry_schema_adoption_is_adjacent_and_append_only() {
    let (mut db, _data_dir) = fixture("adoption").await;
    let source_operation_id = Uuid::new_v4();
    insert_operation(db.pool(), source_operation_id).await;
    let target_operation_id = Uuid::new_v4();
    let adoption_id = Uuid::new_v4();

    let mut tx = db.pool().begin().await.expect("begin adjacent adoption");
    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,
               attack_execution_contract,tool_truth_contract,
               investigation_contract_version,investigation_rollout_mode
           ) VALUES($1,'assessment','target_intel','legacy_v1','legacy','shadow_v1',
                    'legacy_candidate_v1','legacy_only')"#,
    )
    .bind(target_operation_id)
    .execute(&mut *tx)
    .await
    .expect("insert the exact adopted target pair");
    sqlx::query(
        r#"INSERT INTO operation_contract_adoptions(
               adoption_id,source_operation_id,target_operation_id,
               source_tool_truth_contract,source_investigation_contract_version,
               source_investigation_rollout_mode,source_joint_rank,
               target_tool_truth_contract,target_investigation_contract_version,
               target_investigation_rollout_mode,target_joint_rank,
               source_final_seal_hash,adoption_set_hash,stable_request_id,receipt_hash
           ) VALUES(
               $1,$2,$3,'legacy_v1','legacy_candidate_v1','legacy_only',0,
               'shadow_v1','legacy_candidate_v1','legacy_only',1,$4,$5,$6,$7
           )"#,
    )
    .bind(adoption_id)
    .bind(source_operation_id)
    .bind(target_operation_id)
    .bind(digest('a'))
    .bind(digest('b'))
    .bind(Uuid::new_v4())
    .bind(digest('c'))
    .execute(&mut *tx)
    .await
    .expect("insert adjacent adoption before deferred target");
    let frozen: (String, String, String) = sqlx::query_as(
        r#"SELECT tool_truth_contract,investigation_contract_version,investigation_rollout_mode
             FROM operation_state WHERE operation_id=$1"#,
    )
    .bind(target_operation_id)
    .fetch_one(&mut *tx)
    .await
    .expect("read adopted operation contract");
    assert_eq!(
        frozen,
        (
            "shadow_v1".into(),
            "legacy_candidate_v1".into(),
            "legacy_only".into()
        )
    );

    let mutation =
        sqlx::query("UPDATE operation_contract_adoptions SET receipt_hash=$2 WHERE adoption_id=$1")
            .bind(adoption_id)
            .bind(digest('d'))
            .execute(&mut *tx)
            .await
            .expect_err("adoption receipts are append-only");
    assert_database_rejection(&mutation, "investigation_append_only");
    tx.rollback()
        .await
        .expect("rollback schema-only adoption fixture without a stage-fork edge");

    let jump = sqlx::query(
        r#"INSERT INTO operation_contract_adoptions(
               adoption_id,source_operation_id,target_operation_id,
               source_tool_truth_contract,source_investigation_contract_version,
               source_investigation_rollout_mode,source_joint_rank,
               target_tool_truth_contract,target_investigation_contract_version,
               target_investigation_rollout_mode,target_joint_rank,
               source_final_seal_hash,adoption_set_hash,stable_request_id,receipt_hash
           ) VALUES(
               $1,$2,$3,'legacy_v1','legacy_candidate_v1','legacy_only',0,
               'shadow_v1','hypothesis_registry_v1','shadow_registry',2,$4,$5,$6,$7
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(source_operation_id)
    .bind(Uuid::new_v4())
    .bind(digest('e'))
    .bind(digest('f'))
    .bind(Uuid::new_v4())
    .bind(digest('9'))
    .execute(db.pool())
    .await
    .expect_err("joint-rank adoption cannot skip a state");
    assert_database_rejection(&jump, "operation_contract_adoption_adjacent_check");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn hypothesis_registry_schema_stage_team_extensions_are_exact() {
    let (mut db, _data_dir) = fixture("stage_extensions").await;
    let work_item_check: String = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(oid)
             FROM pg_constraint
            WHERE conname='stage_work_items_created_by_check'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("load work-item authority constraint");
    assert!(work_item_check.contains("server_phase_transition"));

    let output_check: String = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(oid)
             FROM pg_constraint
            WHERE conname='stage_worker_outputs_business_disposition_check'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("load worker-output disposition constraint");
    assert!(output_check.contains("artifact_recorded"));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn hypothesis_registry_schema_installs_plan_b_and_follow_on_plan_c_authority() {
    let (mut db, _data_dir) = fixture("plan_boundaries").await;
    for table in [
        "attack_hypotheses",
        "attack_hypothesis_revisions",
        "attack_hypothesis_verification_contracts",
        "attack_hypothesis_verification_plans",
        "candidate_analysis_snapshots",
        "candidate_analysis_attempts",
        "investigation_projection_outbox_batches",
        "investigation_projection_entity_versions",
        "investigation_projection_batch_receipts",
    ] {
        let exists: Option<String> = sqlx::query_scalar("SELECT to_regclass($1)::text")
            .bind(table)
            .fetch_one(db.pool())
            .await
            .expect("inspect Plan B table");
        assert_eq!(
            exists.as_deref(),
            Some(table),
            "missing Plan B table {table}"
        );
    }
    for table in [
        "verification_capability_assessments",
        "hypothesis_revision_adjudications",
        "hypothesis_revision_terminal_decisions",
    ] {
        let exists: Option<String> = sqlx::query_scalar("SELECT to_regclass($1)::text")
            .bind(table)
            .fetch_one(db.pool())
            .await
            .expect("inspect Plan C-owned table");
        assert_eq!(
            exists.as_deref(),
            Some(table),
            "missing follow-on Plan C table {table}"
        );
    }
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn candidate_snapshot_tool_truth_authority_schema_has_compound_plan_a_binding() {
    let (mut db, _data_dir) = fixture("bundle_binding").await;
    let columns: Vec<String> = sqlx::query_scalar(
        r#"SELECT column_name
             FROM information_schema.columns
            WHERE table_name='candidate_analysis_snapshots'
              AND column_name IN (
                  'tool_truth_authority_bundle_seal_id','operation_id','organization_id',
                  'relevant_root_set_hash','bundle_member_set_hash',
                  'semantic_authority_bundle_hash','freshness_attestation_bundle_hash',
                  'temporal_validity_bundle_hash','temporal_validity_policy_set_hash',
                  'target_state_epoch_set_hash','stable_consumer_request_id','snapshot_status'
              )
            ORDER BY column_name"#,
    )
    .fetch_all(db.pool())
    .await
    .expect("load snapshot authority columns");
    assert_eq!(
        columns.len(),
        12,
        "snapshot authority must not collapse the A bundle"
    );

    let member_fk: String = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(c.oid)
             FROM pg_constraint c
             JOIN pg_class child ON child.oid=c.conrelid
             JOIN pg_class parent ON parent.oid=c.confrelid
            WHERE c.contype='f'
              AND child.relname='candidate_analysis_snapshot_authority_bundle_members'
              AND parent.relname='tool_truth_authority_bundle_members'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect exact Plan A member binding");
    for column in [
        "root_execution_authority_id",
        "root_denominator_hash",
        "authority_set_semantic_hash",
        "authority_set_graph_hash",
        "authority_set_freshness_hash",
        "temporal_validity_policy_set_hash",
        "target_state_epoch_set_hash",
        "member_status",
        "member_hash",
    ] {
        assert!(
            member_fk.contains(column),
            "compound member FK omits {column}"
        );
    }
    let header_fk: String = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(c.oid)
             FROM pg_constraint c
             JOIN pg_class child ON child.oid=c.conrelid
             JOIN pg_class parent ON parent.oid=c.confrelid
            WHERE c.contype='f'
              AND child.relname='candidate_analysis_snapshots'
              AND parent.relname='tool_truth_authority_bundle_seals'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect exact Plan A bundle header binding");
    for column in [
        "stable_consumer_request_id",
        "relevant_root_set_hash",
        "bundle_member_set_hash",
        "semantic_authority_bundle_hash",
        "freshness_attestation_bundle_hash",
        "temporal_validity_bundle_hash",
        "temporal_validity_policy_set_hash",
        "target_state_epoch_set_hash",
        "bundle_sealed_at",
    ] {
        assert!(
            header_fk.contains(column),
            "compound header FK omits {column}"
        );
    }
    let exact_set_trigger: String = sqlx::query_scalar(
        r#"SELECT pg_get_triggerdef(oid) FROM pg_trigger
            WHERE tgname='candidate_analysis_snapshot_exact_authority_bundle'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("load deferred exact bundle trigger");
    assert!(exact_set_trigger.contains("DEFERRABLE INITIALLY DEFERRED"));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn hypothesis_state_authority_schema_rejects_terminal_forgery() {
    let (mut db, _data_dir) = fixture("terminal_authority").await;
    let authority = seed_candidate_authority_fixture(db.pool(), "terminal-authority").await;

    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin forged terminal transaction");
    let (_, revision_id) = insert_hypothesis_compound(
        &mut tx,
        authority,
        HypothesisCompoundInput {
            identity_nibble: '1',
            semantic_nibble: '2',
            revision_nibble: '4',
            epistemic_state: "verified",
            lifecycle_state: "closed",
            planning_readiness: "deferred",
            event_kind: "verified",
            origin_authority: "hypothesis_revision_adjudication",
            authority_receipt_kind: "revision_transition_decision",
        },
    )
    .await;
    let commit_error = tx
        .commit()
        .await
        .expect_err("verified revision without adjudication authority must fail at commit");
    assert_database_rejection(
        &commit_error,
        "HYPOTHESIS_REVISION_ADJUDICATION_AUTHORITY_REQUIRED",
    );

    let retained: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM attack_hypothesis_revisions WHERE revision_id=$1")
            .bind(revision_id)
            .fetch_one(db.pool())
            .await
            .expect("confirm forged revision rollback");
    assert_eq!(retained, 0);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn hypothesis_state_authority_schema_rejects_partial_candidate_compound_and_terminal() {
    let (mut db, _data_dir) = fixture("state_events").await;
    let authority = seed_candidate_authority_fixture(db.pool(), "state-events").await;
    let mut legal = db.pool().begin().await.expect("begin legal creating event");
    let (_, revision_id) = insert_hypothesis_compound(
        &mut legal,
        authority,
        HypothesisCompoundInput {
            identity_nibble: '1',
            semantic_nibble: '2',
            revision_nibble: '4',
            epistemic_state: "proposed",
            lifecycle_state: "current",
            planning_readiness: "ready_for_strategy",
            event_kind: "created",
            origin_authority: "candidate_analysis",
            authority_receipt_kind: "candidate_gate_decision",
        },
    )
    .await;
    let partial_error = legal
        .commit()
        .await
        .expect_err("Candidate compound without generation/outbox apply receipt must fail");
    assert_database_rejection(
        &partial_error,
        "HYPOTHESIS_CANDIDATE_CANONICAL_APPLY_RECEIPT_REQUIRED",
    );
    let retained: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM attack_hypothesis_revisions WHERE revision_id=$1")
            .bind(revision_id)
            .fetch_one(db.pool())
            .await
            .expect("confirm legal nonterminal revision committed");
    assert_eq!(retained, 0);

    let mut forged = db
        .pool()
        .begin()
        .await
        .expect("begin forged terminal event");
    insert_hypothesis_compound(
        &mut forged,
        authority,
        HypothesisCompoundInput {
            identity_nibble: '6',
            semantic_nibble: '7',
            revision_nibble: '9',
            epistemic_state: "verified",
            lifecycle_state: "closed",
            planning_readiness: "deferred",
            event_kind: "verified",
            origin_authority: "candidate_analysis",
            authority_receipt_kind: "candidate_gate_decision",
        },
    )
    .await;
    let error = forged
        .commit()
        .await
        .expect_err("Candidate Analysis cannot authorize verified terminal state");
    assert_database_rejection(&error, "HYPOTHESIS_CANDIDATE_TERMINAL_FORBIDDEN");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn candidate_analysis_attempt_schema_has_immutable_two_wave_spine() {
    let (mut db, _data_dir) = fixture("attempt_spine").await;
    assert_tables_exist(
        db.pool(),
        &[
            "candidate_analysis_attempts",
            "candidate_analysis_attempt_state_events",
            "candidate_analysis_page_receipts",
            "candidate_analysis_work_items",
            "candidate_analysis_artifacts",
            "candidate_analysis_host_compilation_seals",
            "hypothesis_proposals",
            "candidate_analysis_proposal_censuses",
            "candidate_analysis_proposal_census_members",
            "candidate_analysis_critic_censuses",
            "candidate_analysis_critic_census_members",
        ],
    )
    .await;
    let append_only: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM pg_trigger trigger
            JOIN pg_class table_ref ON table_ref.oid=trigger.tgrelid
            JOIN pg_proc function_ref ON function_ref.oid=trigger.tgfoid
           WHERE NOT trigger.tgisinternal
             AND table_ref.relname IN (
                 'candidate_analysis_attempts','candidate_analysis_attempt_state_events',
                 'candidate_analysis_page_receipts','candidate_analysis_artifacts',
                 'candidate_analysis_host_compilation_seals'
             )
             AND function_ref.proname='investigation_reject_append_only'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect attempt append-only triggers");
    assert_eq!(append_only, 5);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn candidate_knowledge_feed_schema_freezes_expected_and_observed_members() {
    let (mut db, _data_dir) = fixture("knowledge_feed").await;
    assert_tables_exist(
        db.pool(),
        &[
            "candidate_analysis_knowledge_feed_denominators",
            "candidate_analysis_knowledge_feed_denominator_members",
            "candidate_analysis_knowledge_feed_snapshots",
            "candidate_analysis_knowledge_feed_snapshot_members",
            "candidate_analysis_product_version_censuses",
            "candidate_analysis_product_version_census_members",
            "candidate_analysis_feed_match_censuses",
            "candidate_analysis_feed_match_census_members",
            "candidate_analysis_enrichment_obligations",
        ],
    )
    .await;
    let disposition_check: String = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(c.oid)
             FROM pg_constraint c JOIN pg_class t ON t.oid=c.conrelid
            WHERE t.relname='candidate_analysis_knowledge_feed_snapshot_members'
              AND pg_get_constraintdef(c.oid) LIKE '%signature_invalid%'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("load closed feed disposition constraint");
    for value in [
        "current",
        "stale",
        "signature_invalid",
        "signer_revoked",
        "unavailable",
    ] {
        assert!(disposition_check.contains(value));
    }
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn candidate_input_chunk_census_schema_keeps_replayable_exact_sets() {
    let (mut db, _data_dir) = fixture("input_chunks").await;
    assert_tables_exist(
        db.pool(),
        &[
            "candidate_analysis_snapshot_inputs",
            "candidate_analysis_input_chunk_censuses",
            "candidate_analysis_input_chunk_census_members",
            "candidate_analysis_snapshot_source_sets",
            "candidate_analysis_input_proposal_dispositions",
        ],
    )
    .await;
    let forbidden_instruction_authority: String = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(c.oid)
             FROM pg_constraint c JOIN pg_class t ON t.oid=c.conrelid
            WHERE t.relname='candidate_analysis_snapshot_inputs'
              AND pg_get_constraintdef(c.oid) LIKE '%instruction_authority%'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("load untrusted-input instruction constraint");
    assert!(forbidden_instruction_authority.contains("NOT instruction_authority"));
    let body_columns: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM information_schema.columns
            WHERE table_name='candidate_analysis_input_chunk_census_members'
              AND column_name IN (
                  'immutable_redacted_body','content_blob_id','chunk_hash',
                  'source_range_start','source_range_end'
              )"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect replayable chunk body columns");
    assert_eq!(body_columns, 5);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn hypothesis_coverage_review_schema_has_recursive_exact_reducer_tables() {
    let (mut db, _data_dir) = fixture("coverage_review").await;
    assert_tables_exist(
        db.pool(),
        &[
            "candidate_analysis_hypothesis_coverage_checklist_members",
            "candidate_analysis_hypothesis_coverage_chunk_partitions",
            "candidate_analysis_hypothesis_coverage_subreview_censuses",
            "candidate_analysis_hypothesis_coverage_subreview_census_members",
            "candidate_analysis_hypothesis_coverage_subreviews",
            "candidate_analysis_hypothesis_coverage_synthesis_censuses",
            "candidate_analysis_hypothesis_coverage_synthesis_census_members",
            "candidate_analysis_hypothesis_coverage_synthesis_reviews",
            "candidate_analysis_hypothesis_coverage_global_reviews",
            "candidate_analysis_hypothesis_coverage_reviews",
        ],
    )
    .await;
    let outcome_constraint_tables: Vec<String> = sqlx::query_scalar(
        r#"SELECT DISTINCT t.relname
             FROM pg_constraint c JOIN pg_class t ON t.oid=c.conrelid
            WHERE t.relname IN (
                'candidate_analysis_hypothesis_coverage_subreviews',
                'candidate_analysis_hypothesis_coverage_global_reviews',
                'candidate_analysis_hypothesis_coverage_reviews'
            ) AND pg_get_constraintdef(c.oid) LIKE '%missed_hypothesis%'"#,
    )
    .fetch_all(db.pool())
    .await
    .expect("load coverage outcome constraints");
    assert_eq!(outcome_constraint_tables.len(), 3);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn verification_contract_schema_has_closed_component_control_shapes() {
    let (mut db, _data_dir) = fixture("verification_contract").await;
    assert_tables_exist(
        db.pool(),
        &[
            "attack_hypothesis_verification_objectives",
            "attack_hypothesis_verification_contracts",
            "attack_hypothesis_verification_objective_claim_components",
            "attack_hypothesis_verification_predicate_components",
            "attack_hypothesis_verification_required_controls",
            "attack_hypothesis_verification_pair_bindings",
            "attack_hypothesis_verification_ordered_steps",
        ],
    )
    .await;
    let combinator_check: String = sqlx::query_scalar(
        r#"SELECT string_agg(pg_get_constraintdef(c.oid),' ' ORDER BY c.oid)
             FROM pg_constraint c JOIN pg_class t ON t.oid=c.conrelid
            WHERE t.relname='attack_hypothesis_verification_contracts'
              AND pg_get_constraintdef(c.oid) LIKE '%paired_differential%'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("load VerificationContract combinator constraint");
    assert!(combinator_check.contains("ordered_sequence"));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn hypothesis_claim_component_schema_is_closed_and_revision_scoped() {
    let (mut db, _data_dir) = fixture("claim_component").await;
    let component_check: String = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(c.oid)
             FROM pg_constraint c JOIN pg_class t ON t.oid=c.conrelid
            WHERE t.relname='attack_hypothesis_claim_components'
              AND pg_get_constraintdef(c.oid) LIKE '%trust_boundary_condition%'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("load claim component kind constraint");
    for value in [
        "claim_clause",
        "impact_qualifier",
        "trust_boundary_condition",
        "identity_condition",
    ] {
        assert!(component_check.contains(value));
    }
    let objective_component_fk: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM pg_constraint c
            JOIN pg_class child ON child.oid=c.conrelid
            JOIN pg_class parent ON parent.oid=c.confrelid
           WHERE c.contype='f'
             AND child.relname='attack_hypothesis_verification_objective_claim_components'
             AND parent.relname='attack_hypothesis_claim_components'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect objective claim-component authority");
    assert_eq!(objective_component_fk, 1);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn hypothesis_verification_plan_schema_has_objective_and_path_exact_sets() {
    let (mut db, _data_dir) = fixture("verification_plan").await;
    assert_tables_exist(
        db.pool(),
        &[
            "attack_hypothesis_verification_plans",
            "attack_hypothesis_verification_plan_objectives",
            "attack_hypothesis_verification_plan_paths",
            "attack_hypothesis_verification_plan_path_members",
        ],
    )
    .await;
    let plan_shape_columns: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM information_schema.columns
            WHERE table_name='attack_hypothesis_verification_plans'
              AND column_name IN (
                  'required_claim_component_count','required_claim_component_set_hash',
                  'objective_count','objective_set_hash','proof_path_count','proof_path_set_hash',
                  'outer_aggregation_policy_version','outer_aggregation_policy_digest','plan_hash'
              )"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect frozen verification plan exact sets");
    assert_eq!(plan_shape_columns, 9);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn projection_catalog_schema_is_closed_and_rejects_unknown_mapping() {
    let (mut db, _data_dir) = fixture("projection_catalog").await;
    let valid: bool = sqlx::query_scalar(
        "SELECT projection_timeline_mapping_is_valid('hypothesis','insert','hypothesis_inserted')",
    )
    .fetch_one(db.pool())
    .await
    .expect("evaluate known projection mapping");
    assert!(valid);
    let unknown: bool = sqlx::query_scalar(
        "SELECT projection_timeline_mapping_is_valid('future_entity','insert','hypothesis_inserted')",
    )
    .fetch_one(db.pool())
    .await
    .expect("evaluate unknown projection mapping");
    assert!(!unknown);
    assert_tables_exist(db.pool(), &["investigation_projection_changes"]).await;
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn projection_plan_b_verification_plan_route_is_exact_one() {
    let (mut db, _data_dir) = fixture("plan_b_route").await;
    let plan_route: bool = sqlx::query_scalar(
        "SELECT projection_timeline_mapping_is_valid('hypothesis_verification_plan','close','hypothesis_verification_plan_sealed')",
    )
    .fetch_one(db.pool())
    .await
    .expect("evaluate Plan B plan-seal route");
    assert!(plan_route);
    let campaign_substitution: bool = sqlx::query_scalar(
        "SELECT projection_timeline_mapping_is_valid('campaign_terminal','close','hypothesis_verification_plan_sealed')",
    )
    .fetch_one(db.pool())
    .await
    .expect("evaluate forbidden Campaign substitution");
    assert!(!campaign_substitution);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn projection_plan_c_route_catalog_is_backed_by_installed_authority_tables() {
    let (mut db, _data_dir) = fixture("plan_c_routes").await;
    for (entity, change, event) in [
        (
            "hypothesis_revision_adjudication",
            "close",
            "hypothesis_revision_adjudication_closed",
        ),
        (
            "hypothesis_revision_terminal_decision",
            "close",
            "hypothesis_revision_terminal_decision_closed",
        ),
        ("finding", "insert", "finding_inserted"),
        (
            "hypothesis_state_event",
            "insert",
            "hypothesis_state_event_inserted",
        ),
        ("hypothesis", "insert", "hypothesis_inserted"),
    ] {
        let valid: bool =
            sqlx::query_scalar("SELECT projection_timeline_mapping_is_valid($1,$2,$3)")
                .bind(entity)
                .bind(change)
                .bind(event)
                .fetch_one(db.pool())
                .await
                .expect("evaluate frozen Plan C route vocabulary");
        assert!(valid, "missing future route {entity}/{change}/{event}");
    }
    for table in [
        "hypothesis_revision_adjudications",
        "hypothesis_revision_terminal_decisions",
    ] {
        let exists: Option<String> = sqlx::query_scalar("SELECT to_regclass($1)::text")
            .bind(table)
            .fetch_one(db.pool())
            .await
            .expect("inspect installed Plan C authority table");
        assert_eq!(
            exists.as_deref(),
            Some(table),
            "missing installed authority table for frozen route catalog"
        );
    }
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn projection_source_snapshot_schema_freezes_inline_or_blob_payload() {
    let (mut db, _data_dir) = fixture("projection_snapshot").await;
    assert_tables_exist(
        db.pool(),
        &[
            "investigation_projection_source_blobs",
            "investigation_projection_outbox",
        ],
    )
    .await;
    let source_columns: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM information_schema.columns
            WHERE table_name='investigation_projection_outbox'
              AND column_name IN (
                  'source_snapshot_schema','source_snapshot_version','source_snapshot_hash',
                  'immutable_source_body','source_blob_id','source_blob_hash',
                  'source_occurred_at','source_time_status'
              )"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect frozen projection source snapshot");
    assert_eq!(source_columns, 8);
    let live_locator_columns: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM information_schema.columns
            WHERE table_name='investigation_projection_outbox'
              AND column_name IN ('source_table','source_path','loader','live_locator')"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect forbidden live source locators");
    assert_eq!(live_locator_columns, 0);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn projection_entity_predecessor_schema_binds_direct_version_and_hash() {
    let (mut db, _data_dir) = fixture("entity_predecessor").await;
    let self_fk_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM pg_constraint c
            JOIN pg_class child ON child.oid=c.conrelid
            JOIN pg_class parent ON parent.oid=c.confrelid
           WHERE c.contype='f'
             AND child.relname='investigation_projection_entity_versions'
             AND parent.relname='investigation_projection_entity_versions'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect direct predecessor self-FK");
    assert_eq!(self_fk_count, 1);
    let predecessor_check: String = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(c.oid)
             FROM pg_constraint c JOIN pg_class t ON t.oid=c.conrelid
            WHERE t.relname='investigation_projection_entity_versions'
              AND pg_get_constraintdef(c.oid) LIKE '%predecessor_absent%'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("load entity predecessor shape constraint");
    assert!(predecessor_check.contains("entity_version"));
    assert!(predecessor_check.contains("predecessor_projection_hash"));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn projection_batch_schema_has_receipt_truth_and_no_processed_flag() {
    let (mut db, _data_dir) = fixture("projection_batch").await;
    assert_tables_exist(
        db.pool(),
        &[
            "investigation_projection_source_heads",
            "investigation_projection_heads",
            "investigation_projection_outbox_batches",
            "investigation_projection_outbox",
            "investigation_projection_entity_versions",
            "investigation_projection_changes",
            "investigation_projection_batch_receipts",
        ],
    )
    .await;
    let processed_columns: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM information_schema.columns
            WHERE table_name='investigation_projection_outbox'
              AND column_name IN ('processed','processed_at','is_processed')"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect forbidden per-member processing markers");
    assert_eq!(processed_columns, 0);
    let receipt_unique: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM pg_constraint c JOIN pg_class t ON t.oid=c.conrelid
            WHERE t.relname='investigation_projection_batch_receipts'
              AND c.contype='u' AND pg_get_constraintdef(c.oid) LIKE '%batch_id%'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect exact-one batch receipt");
    assert!(receipt_unique >= 1);
    db.stop().await;
}
