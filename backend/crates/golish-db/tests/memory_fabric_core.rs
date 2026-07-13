use chrono::{TimeZone, Utc};
use golish_db::repo::{
    knowledge_assertions, knowledge_documents, knowledge_embeddings, knowledge_outbox,
    project_scopes, stage_episodes,
};
use golish_db::{DbConfig, GolishDb};
use golish_memory_domain::assertion::{
    AssertionKind, AssertionObject, AssertionStatus, KnowledgeAssertion,
};
use golish_memory_domain::classification::{AssertionVisibility, KnowledgeClassification};
use golish_memory_domain::event_catalog::{
    KnowledgeEventEnvelopeV1, KnowledgeEventNameV1, KnowledgeEventPayloadV1, ProjectorId,
};
use golish_memory_domain::scope::{OperationScope, ProjectScopeId};
use golish_memory_domain::source_ref::{CanonicalRowId, CanonicalSourceKind, SourceRef};
use golish_memory_domain::{EpisodeVerdict, StageEpisode, EMBEDDING_DIMENSION_V1};
use serial_test::serial;
use uuid::Uuid;

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read local postgres port")
        .port()
}

async fn fixture() -> (GolishDb, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let config = DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("memory_fabric_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    };
    let db = GolishDb::start(config)
        .await
        .expect("start migrated embedded postgres");
    (db, data_dir)
}

fn source(
    source_kind: CanonicalSourceKind,
    row_id: CanonicalRowId,
    stream: &str,
    version: i64,
) -> SourceRef {
    SourceRef {
        source_kind,
        row_id,
        source_stream_key: stream.to_string(),
        version,
    }
}

fn event(
    event_id: Uuid,
    project_scope_id: Uuid,
    organization_id: Uuid,
    source: SourceRef,
    event_name: KnowledgeEventNameV1,
) -> KnowledgeEventEnvelopeV1 {
    KnowledgeEventEnvelopeV1 {
        event_id,
        project_scope_id: Some(ProjectScopeId(project_scope_id)),
        organization_id_at_time: Some(organization_id),
        source_operation_id: Uuid::from_u128(0x9000),
        event_name,
        schema_version: 1,
        payload: KnowledgeEventPayloadV1 {
            source_stream_key: source.source_stream_key.clone(),
            source_version: source.version,
            source,
            structured_payload: serde_json::json!({"fixture": "typed"}),
        },
        occurred_at: Utc
            .with_ymd_and_hms(2026, 7, 12, 12, 0, 0)
            .single()
            .expect("fixed timestamp"),
    }
}

#[tokio::test]
#[serial]
async fn memory_fabric_typed_ids_delivery_dag_and_projection_invalidation() {
    let (mut db, _data_dir) = fixture().await;
    let scope_a = project_scopes::register_first_open(db.pool(), "/fixture/memory-a", "a-sha")
        .await
        .expect("register scope a");
    let scope_b = project_scopes::register_first_open(db.pool(), "/fixture/memory-b", "b-sha")
        .await
        .expect("register scope b");
    let organization_id = Uuid::from_u128(0x100);

    let vector_type: String = sqlx::query_scalar(
        r#"SELECT format_type(attribute.atttypid, attribute.atttypmod)
           FROM pg_attribute attribute
           JOIN pg_class relation ON relation.oid = attribute.attrelid
           WHERE relation.relname = 'knowledge_embeddings'
             AND attribute.attname = 'embedding'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("read embedding physical type");
    assert_eq!(vector_type, "vector(1536)");

    let first = KnowledgeAssertion::new_for_test(
        AssertionVisibility::OrganizationLongTerm {
            project_scope_id: ProjectScopeId(scope_a.project_scope_id),
            organization_id_at_time: organization_id,
        },
        "target:example.com",
        "http.status",
        AssertionObject::Json(serde_json::json!(200)),
        AssertionKind::Observation,
        AssertionStatus::Active,
        source(
            CanonicalSourceKind::FactDelta,
            CanonicalRowId::Int64(42),
            "fact-delta:42",
            1,
        ),
        KnowledgeClassification::CustomerConfidential,
    )
    .expect("first assertion");
    let second = KnowledgeAssertion::new_for_test(
        AssertionVisibility::OrganizationLongTerm {
            project_scope_id: ProjectScopeId(scope_a.project_scope_id),
            organization_id_at_time: organization_id,
        },
        "target:example.com",
        "http.status",
        AssertionObject::Json(serde_json::json!(404)),
        AssertionKind::Observation,
        AssertionStatus::Active,
        source(
            CanonicalSourceKind::FactDelta,
            CanonicalRowId::Int64(42),
            "fact-delta:42",
            1,
        ),
        KnowledgeClassification::CustomerConfidential,
    )
    .expect("second assertion");
    let mut connection = db.pool().acquire().await.expect("assertion connection");
    let stored_first = knowledge_assertions::insert_with_connection(&mut connection, &first)
        .await
        .expect("insert first object");
    let stored_second = knowledge_assertions::insert_with_connection(&mut connection, &second)
        .await
        .expect("insert second object");
    assert_eq!(stored_first.source.row_id, CanonicalRowId::Int64(42));
    assert_eq!(stored_second.source.row_id, CanonicalRowId::Int64(42));
    assert_ne!(stored_first.assertion_id, stored_second.assertion_id);
    let mut assertion_drift = first.clone();
    assertion_drift.assertion_id = Uuid::from_u128(0x199);
    let assertion_drift_error =
        knowledge_assertions::insert_with_connection(&mut connection, &assertion_drift)
            .await
            .expect_err("same source identity with a different assertion id must conflict");
    assert_eq!(
        assertion_drift_error.code(),
        "memory_assertion_replay_conflict"
    );
    drop(connection);

    for projector in [
        ProjectorId::AssertionPromoterV1,
        ProjectorId::DocumentProjectorV1,
        ProjectorId::EmbeddingProjectorV1,
    ] {
        knowledge_outbox::set_projector_lifecycle(
            db.pool(),
            projector,
            knowledge_outbox::ProjectorLifecycle::Enabled,
            None,
        )
        .await
        .expect("enable projector");
    }

    // A newer version in another project scope must not stale this customer's
    // event even when the stream key happens to match.
    let cross_scope_b = event(
        Uuid::from_u128(0x201),
        scope_b.project_scope_id,
        organization_id,
        source(
            CanonicalSourceKind::FactDelta,
            CanonicalRowId::Int64(51),
            "shared-stream-name",
            2,
        ),
        KnowledgeEventNameV1::FactDeltaAccepted,
    );
    let mut tx = db.pool().begin().await.expect("cross scope b tx");
    knowledge_outbox::append_event_with_catalog_deliveries(&mut tx, &cross_scope_b)
        .await
        .expect("append scope b high version");
    tx.commit().await.expect("commit scope b event");
    let cross_scope_a = event(
        Uuid::from_u128(0x202),
        scope_a.project_scope_id,
        organization_id,
        source(
            CanonicalSourceKind::FactDelta,
            CanonicalRowId::Int64(52),
            "shared-stream-name",
            1,
        ),
        KnowledgeEventNameV1::FactDeltaAccepted,
    );
    let mut tx = db.pool().begin().await.expect("cross scope a tx");
    knowledge_outbox::append_event_with_catalog_deliveries(&mut tx, &cross_scope_a)
        .await
        .expect("append scope a low version");
    tx.commit().await.expect("commit scope a event");
    let cross_rows = knowledge_outbox::list_deliveries(db.pool(), cross_scope_a.event_id)
        .await
        .expect("scope a deliveries");
    assert_eq!(
        cross_rows
            .iter()
            .find(|row| row.projector_name == "assertion-promoter")
            .expect("assertion delivery")
            .status,
        knowledge_outbox::DeliveryStatus::Pending
    );

    let episode_id = Uuid::from_u128(0x301);
    let episode_source = source(
        CanonicalSourceKind::StageEpisode,
        CanonicalRowId::Uuid(episode_id),
        "stage-episode:301",
        1,
    );
    let episode_event = event(
        Uuid::from_u128(0x302),
        scope_a.project_scope_id,
        organization_id,
        episode_source.clone(),
        KnowledgeEventNameV1::StageEpisodeClosed,
    );
    let episode = StageEpisode {
        episode_id,
        scope: OperationScope {
            project_scope_id: ProjectScopeId(scope_a.project_scope_id),
            source_operation_id: episode_event.source_operation_id,
            organization_id_at_time: organization_id,
            scope_snapshot_hash: "scope-snapshot-sha".to_string(),
        },
        stage_execution_id: Uuid::from_u128(0x303),
        stage_run_unit_id: Some(Uuid::from_u128(0x304)),
        worker_run_id: Some(Uuid::from_u128(0x305)),
        candidate_attempt_id: None,
        stage_kind: "enumeration".to_string(),
        wave: Some(0),
        verdict: EpisodeVerdict::Passed,
        deliverable_submission_id: Some(Uuid::from_u128(0x306)),
        handoff_id: Some(Uuid::from_u128(0x307)),
        reason_codes: vec!["gate_passed".to_string()],
        fact_refs: vec![episode_source],
        evidence_ids: vec![91],
        started_at: episode_event.occurred_at - chrono::Duration::minutes(1),
        ended_at: episode_event.occurred_at,
    };
    stage_episodes::close_episode_and_emit(db.pool(), &episode, &episode_event)
        .await
        .expect("atomically close episode and append deliveries");
    assert_eq!(
        stage_episodes::get(db.pool(), episode_id)
            .await
            .expect("load episode")
            .verdict,
        "passed"
    );

    sqlx::query(
        r#"CREATE FUNCTION reject_memory_outbox_fixture()
           RETURNS trigger AS $$
           BEGIN
               RAISE EXCEPTION 'fixture outbox failure';
           END;
           $$ LANGUAGE plpgsql;"#,
    )
    .execute(db.pool())
    .await
    .expect("install outbox failure function");
    sqlx::query(
        r#"CREATE TRIGGER reject_memory_outbox_fixture
           BEFORE INSERT ON knowledge_outbox_events
           FOR EACH ROW EXECUTE FUNCTION reject_memory_outbox_fixture();"#,
    )
    .execute(db.pool())
    .await
    .expect("install outbox failure trigger");
    let rollback_episode_id = Uuid::from_u128(0x308);
    let mut rollback_episode = episode.clone();
    rollback_episode.episode_id = rollback_episode_id;
    let mut rollback_event = event(
        Uuid::from_u128(0x309),
        scope_a.project_scope_id,
        organization_id,
        source(
            CanonicalSourceKind::StageEpisode,
            CanonicalRowId::Uuid(rollback_episode_id),
            "stage-episode:rollback",
            1,
        ),
        KnowledgeEventNameV1::StageEpisodeClosed,
    );
    rollback_event.source_operation_id = rollback_episode.scope.source_operation_id;
    rollback_event.organization_id_at_time = Some(rollback_episode.scope.organization_id_at_time);
    stage_episodes::close_episode_and_emit(db.pool(), &rollback_episode, &rollback_event)
        .await
        .expect_err("outbox failure must roll back episode insert");
    let rolled_back_episode_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM stage_episodes WHERE episode_id = $1")
            .bind(rollback_episode_id)
            .fetch_one(db.pool())
            .await
            .expect("count rolled back episode");
    assert_eq!(rolled_back_episode_count, 0);
    sqlx::query("DROP TRIGGER reject_memory_outbox_fixture ON knowledge_outbox_events")
        .execute(db.pool())
        .await
        .expect("remove outbox failure trigger");
    sqlx::query("DROP FUNCTION reject_memory_outbox_fixture()")
        .execute(db.pool())
        .await
        .expect("remove outbox failure function");
    let mut episode_drift = episode.clone();
    episode_drift.reason_codes = vec!["drifted_reason".to_string()];
    let episode_drift_error =
        stage_episodes::close_episode_and_emit(db.pool(), &episode_drift, &episode_event)
            .await
            .expect_err("same episode id with changed payload must conflict");
    assert_eq!(episode_drift_error.code(), "memory_episode_replay_conflict");
    let mut wrong_kind_event = episode_event.clone();
    wrong_kind_event.event_name = KnowledgeEventNameV1::FactDeltaAccepted;
    let wrong_kind_error =
        stage_episodes::close_episode_and_emit(db.pool(), &episode, &wrong_kind_event)
            .await
            .expect_err("non-episode event must not close an episode");
    assert_eq!(
        wrong_kind_error.code(),
        "memory_episode_event_source_mismatch"
    );

    assert!(knowledge_outbox::claim_delivery_batch(
        db.pool(),
        ProjectorId::DocumentProjectorV1,
        "document-worker",
        10,
    )
    .await
    .expect("document preclaim")
    .iter()
    .all(|row| row.event_id != episode_event.event_id));
    let claimed = knowledge_outbox::claim_delivery_batch(
        db.pool(),
        ProjectorId::AssertionPromoterV1,
        "expired-worker",
        10,
    )
    .await
    .expect("claim assertion delivery");
    assert!(claimed
        .iter()
        .any(|row| row.event_id == episode_event.event_id));
    sqlx::query(
        r#"UPDATE knowledge_projection_deliveries
           SET lease_expires_at = NOW() - INTERVAL '1 second'
           WHERE event_id = $1 AND projector_name = 'assertion-promoter'"#,
    )
    .bind(episode_event.event_id)
    .execute(db.pool())
    .await
    .expect("expire assertion lease");
    let expired = knowledge_outbox::complete_delivery(
        db.pool(),
        episode_event.event_id,
        ProjectorId::AssertionPromoterV1,
        "expired-worker",
        knowledge_outbox::DeliveryStatus::Succeeded,
        None,
    )
    .await
    .expect_err("expired lease must lose its fence");
    assert_eq!(expired.code(), "memory_delivery_lease_fence_lost");
    let reclaimed = knowledge_outbox::claim_delivery_batch(
        db.pool(),
        ProjectorId::AssertionPromoterV1,
        "assertion-worker",
        10,
    )
    .await
    .expect("reclaim expired assertion delivery");
    assert!(reclaimed
        .iter()
        .any(|row| row.event_id == episode_event.event_id));
    knowledge_outbox::complete_delivery(
        db.pool(),
        episode_event.event_id,
        ProjectorId::AssertionPromoterV1,
        "assertion-worker",
        knowledge_outbox::DeliveryStatus::Succeeded,
        None,
    )
    .await
    .expect("complete assertion delivery");
    let documents = knowledge_outbox::claim_delivery_batch(
        db.pool(),
        ProjectorId::DocumentProjectorV1,
        "document-worker",
        10,
    )
    .await
    .expect("claim document after assertion");
    assert!(documents
        .iter()
        .any(|row| row.event_id == episode_event.event_id));
    assert!(knowledge_outbox::claim_delivery_batch(
        db.pool(),
        ProjectorId::EmbeddingProjectorV1,
        "embedding-worker",
        10,
    )
    .await
    .expect("embedding preclaim")
    .iter()
    .all(|row| row.event_id != episode_event.event_id));
    knowledge_outbox::complete_delivery(
        db.pool(),
        episode_event.event_id,
        ProjectorId::DocumentProjectorV1,
        "document-worker",
        knowledge_outbox::DeliveryStatus::Succeeded,
        None,
    )
    .await
    .expect("complete document delivery");
    let embeddings = knowledge_outbox::claim_delivery_batch(
        db.pool(),
        ProjectorId::EmbeddingProjectorV1,
        "embedding-worker",
        10,
    )
    .await
    .expect("claim embedding after document");
    assert!(embeddings
        .iter()
        .any(|row| row.event_id == episode_event.event_id));
    knowledge_outbox::complete_delivery(
        db.pool(),
        episode_event.event_id,
        ProjectorId::EmbeddingProjectorV1,
        "embedding-worker",
        knowledge_outbox::DeliveryStatus::SucceededSuppressed,
        Some("classification_policy"),
    )
    .await
    .expect("terminally suppress embedding");
    let final_rows = knowledge_outbox::list_deliveries(db.pool(), episode_event.event_id)
        .await
        .expect("final delivery rows");
    assert_eq!(
        final_rows
            .iter()
            .find(|row| row.projector_name == "embedding-projector")
            .expect("embedding delivery")
            .status,
        knowledge_outbox::DeliveryStatus::SucceededSuppressed
    );

    sqlx::query(
        r#"INSERT INTO knowledge_projector_registry (
               projector_name, projector_schema_version, lifecycle
           ) VALUES ('invalid-dependency-fixture', 1, 'enabled')"#,
    )
    .execute(db.pool())
    .await
    .expect("insert invalid dependency fixture projector");
    let dependency_constraint_error = sqlx::query(
        r#"INSERT INTO knowledge_projection_deliveries (
               event_id, projector_name, projector_schema_version, status
           ) VALUES ($1, 'invalid-dependency-fixture', 1, 'blocked_dependency')"#,
    )
    .bind(episode_event.event_id)
    .execute(db.pool())
    .await
    .expect_err("blocked dependency without dependency identity must fail");
    assert_eq!(
        dependency_constraint_error
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("23514")
    );

    let suppressed_predecessor = event(
        Uuid::from_u128(0x350),
        scope_a.project_scope_id,
        organization_id,
        source(
            CanonicalSourceKind::FactDelta,
            CanonicalRowId::Int64(350),
            "fact-delta:suppressed-predecessor",
            1,
        ),
        KnowledgeEventNameV1::FactDeltaAccepted,
    );
    let mut suppressed_tx = db.pool().begin().await.expect("suppressed event tx");
    knowledge_outbox::append_event_with_catalog_deliveries(
        &mut suppressed_tx,
        &suppressed_predecessor,
    )
    .await
    .expect("append suppressed predecessor event");
    suppressed_tx
        .commit()
        .await
        .expect("commit suppressed event");
    let suppressed_claim = knowledge_outbox::claim_delivery_batch(
        db.pool(),
        ProjectorId::AssertionPromoterV1,
        "suppressed-assertion-worker",
        10,
    )
    .await
    .expect("claim predecessor to suppress");
    assert!(suppressed_claim
        .iter()
        .any(|row| row.event_id == suppressed_predecessor.event_id));
    knowledge_outbox::complete_delivery(
        db.pool(),
        suppressed_predecessor.event_id,
        ProjectorId::AssertionPromoterV1,
        "suppressed-assertion-worker",
        knowledge_outbox::DeliveryStatus::SucceededSuppressed,
        Some("source_policy"),
    )
    .await
    .expect("suppress predecessor terminally");
    let document_after_suppression = knowledge_outbox::claim_delivery_batch(
        db.pool(),
        ProjectorId::DocumentProjectorV1,
        "suppressed-document-worker",
        10,
    )
    .await
    .expect("suppressed predecessor must satisfy dependency");
    assert!(document_after_suppression
        .iter()
        .any(|row| row.event_id == suppressed_predecessor.event_id));

    let mut duplicate_tx = db.pool().begin().await.expect("duplicate tx");
    let duplicate_id =
        knowledge_outbox::append_event_with_catalog_deliveries(&mut duplicate_tx, &episode_event)
            .await
            .expect("exact replay");
    assert_eq!(duplicate_id, episode_event.event_id);
    duplicate_tx.commit().await.expect("commit exact replay");
    let mut drifted = episode_event.clone();
    drifted.payload.structured_payload = serde_json::json!({"fixture": "drifted"});
    let mut drift_tx = db.pool().begin().await.expect("drift tx");
    let drift_error =
        knowledge_outbox::append_event_with_catalog_deliveries(&mut drift_tx, &drifted)
            .await
            .expect_err("same dedupe identity with changed payload must fail");
    assert_eq!(drift_error.code(), "memory_outbox_dedupe_conflict");
    drift_tx.rollback().await.expect("rollback drift tx");

    let document_id = Uuid::from_u128(0x401);
    let document = knowledge_documents::UpsertKnowledgeDocument {
        document_id,
        document_key: "d".repeat(64),
        project_scope_id: Some(scope_a.project_scope_id),
        source_stream_key: "fact-delta:42".to_string(),
        source_version: 1,
        projection_schema_version: 1,
        redaction_policy_version: 1,
        assertion_ids: vec![first.assertion_id, second.assertion_id],
        document_type: "assertion_bundle".to_string(),
        redacted_content: "structured-only".to_string(),
        content_hash: "e".repeat(64),
        classification: "customer_confidential".to_string(),
        valid_from: episode_event.occurred_at,
        valid_to: None,
    };
    knowledge_documents::upsert(db.pool(), &document)
        .await
        .expect("upsert deterministic document");
    let dimension_error = knowledge_embeddings::insert(
        db.pool(),
        &knowledge_embeddings::InsertKnowledgeEmbedding {
            embedding_id: Uuid::from_u128(0x402),
            document_id,
            source_stream_key: "fact-delta:42".to_string(),
            source_version: 1,
            provider: "fixture".to_string(),
            model: "fixture-1024".to_string(),
            embedding: vec![0.0; 1024],
            content_hash: "e".repeat(64),
            valid_from: episode_event.occurred_at,
            valid_to: None,
        },
    )
    .await
    .expect_err("1024 dimension must fail closed");
    assert_eq!(
        dimension_error.code(),
        "memory_embedding_dimension_mismatch"
    );
    let embedding_id = Uuid::from_u128(0x403);
    knowledge_embeddings::insert(
        db.pool(),
        &knowledge_embeddings::InsertKnowledgeEmbedding {
            embedding_id,
            document_id,
            source_stream_key: "fact-delta:42".to_string(),
            source_version: 1,
            provider: "fixture".to_string(),
            model: "fixture-1536".to_string(),
            embedding: vec![0.0; EMBEDDING_DIMENSION_V1],
            content_hash: "e".repeat(64),
            valid_from: episode_event.occurred_at,
            valid_to: None,
        },
    )
    .await
    .expect("insert 1536-dimensional embedding");
    knowledge_documents::invalidate_source(
        db.pool(),
        Some(scope_a.project_scope_id),
        "fact-delta:42",
        1,
        episode_event.occurred_at + chrono::Duration::minutes(5),
    )
    .await
    .expect("invalidate document and embedding together");
    assert_eq!(
        knowledge_documents::get(db.pool(), document_id)
            .await
            .expect("load invalidated document")
            .status,
        "invalidated"
    );
    assert_eq!(
        knowledge_embeddings::get(db.pool(), embedding_id)
            .await
            .expect("load invalidated embedding")
            .status,
        "invalidated"
    );

    let immutable_error =
        sqlx::query("UPDATE knowledge_outbox_events SET payload = '{}'::jsonb WHERE event_id = $1")
            .bind(episode_event.event_id)
            .execute(db.pool())
            .await
            .expect_err("outbox event must be immutable");
    assert!(immutable_error.as_database_error().is_some());

    db.stop().await;
}
