use std::sync::Arc;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use golish_agent_app::ai::db_bridge::knowledge_memory::{
    KnowledgeMemoryRuntime, PgKnowledgeMemory,
};
use golish_db::repo::{knowledge_assertions, knowledge_outbox, project_scopes};
use golish_db::{DbConfig, GolishDb};
use golish_memory_app::ports::{
    CloseEpisodeAndEmit, InvalidateProjectionChainAndEmit, PromoteAssertionAndEmit,
};
use golish_memory_app::{KnowledgeUnitOfWork, SupervisorStartOutcome};
use golish_memory_domain::assertion::{
    AssertionIdentity, AssertionKind, AssertionObject, AssertionStatus, KnowledgeAssertionDraft,
};
use golish_memory_domain::classification::{AssertionVisibility, KnowledgeClassification};
use golish_memory_domain::event_catalog::{
    KnowledgeEventEnvelopeV1, KnowledgeEventNameV1, KnowledgeEventPayloadV1, ProjectorId,
};
use golish_memory_domain::scope::{OperationScope, ProjectScopeId};
use golish_memory_domain::source_ref::{CanonicalRowId, CanonicalSourceKind, SourceRef};
use golish_memory_domain::{EpisodeVerdict, StageEpisode};
use serial_test::serial;
use uuid::Uuid;

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read local postgres port")
        .port()
}

async fn fixture(name: &str) -> (GolishDb, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let config = DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("{name}_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    };
    let db = GolishDb::start(config)
        .await
        .expect("start migrated embedded postgres");
    (db, data_dir)
}

fn at(hour: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 13, hour, 0, 0)
        .single()
        .expect("fixed timestamp")
}

fn event(
    event_id: Uuid,
    project_scope_id: Uuid,
    organization_id: Uuid,
    operation_id: Uuid,
    event_name: KnowledgeEventNameV1,
    source: SourceRef,
) -> KnowledgeEventEnvelopeV1 {
    KnowledgeEventEnvelopeV1 {
        event_id,
        project_scope_id: Some(ProjectScopeId(project_scope_id)),
        organization_id_at_time: Some(organization_id),
        source_operation_id: operation_id,
        event_name,
        schema_version: 1,
        payload: KnowledgeEventPayloadV1 {
            source_stream_key: source.source_stream_key.clone(),
            source_version: source.version,
            source,
            structured_payload: serde_json::json!({"schema": "fixture.v1"}),
        },
        occurred_at: at(12),
    }
}

fn reason_only_candidate_event(
    event_id: Uuid,
    project_scope_id: Uuid,
    organization_id: Uuid,
    operation_id: Uuid,
    attempt_id: Uuid,
) -> KnowledgeEventEnvelopeV1 {
    let source = SourceRef {
        source_kind: CanonicalSourceKind::CandidateAttempt,
        row_id: CanonicalRowId::Uuid(attempt_id),
        source_stream_key: format!("candidate-attempt:{attempt_id}"),
        version: 1,
    };
    let mut typed_event = event(
        event_id,
        project_scope_id,
        organization_id,
        operation_id,
        KnowledgeEventNameV1::CandidateAttemptTerminal,
        source,
    );
    typed_event.payload.structured_payload = serde_json::json!({
        "attempt_id": attempt_id,
        "candidate_id": Uuid::new_v4(),
        "approval_id": Uuid::new_v4(),
        "disposition": "blocked",
        "candidate_plan_hash": "candidate-plan",
        "result_hash": "result-hash",
        "finding_id": null,
        "target_type_at_time": "domain",
        "target_value_at_time": "example.test",
        "target_identity_hash": "target-hash",
        "evidence_ids": [],
        "proof_evidence_ids": [],
        "refutation_evidence_ids": [],
        "blocker_evidence_ids": [],
        "blocker_reason_code": "approval_expired",
        "fact_delta_count": 0,
    });
    typed_event
}

async fn seed_sealed_scope_authority(
    pool: &sqlx::PgPool,
    project_path: &str,
    project_scope_id: Uuid,
    operation_id: Uuid,
    organization_id: Uuid,
) {
    let session_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO sessions(id,title,status,project_path) \
         VALUES($1,'Memory authority fixture','running',$2)",
    )
    .bind(session_id)
    .bind(project_path)
    .execute(pool)
    .await
    .expect("insert authority fixture session");
    sqlx::query(
        "INSERT INTO tasks(id,session_id,title,input,status) \
         VALUES($1,$2,'Memory authority operation','reason-only Candidate','running')",
    )
    .bind(operation_id)
    .bind(session_id)
    .execute(pool)
    .await
    .expect("insert authority fixture operation task");
    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,
               attack_execution_contract,project_scope_id
           ) VALUES($1,'red_team','attack_candidate','v2_only','v2_only',$2)"#,
    )
    .bind(operation_id)
    .bind(project_scope_id)
    .execute(pool)
    .await
    .expect("insert authority fixture operation state");
    sqlx::query(
        "INSERT INTO organizations(id,project_path,name) VALUES($1,$2,'Memory Authority Org')",
    )
    .bind(organization_id)
    .bind(project_path)
    .execute(pool)
    .await
    .expect("insert authority fixture organization");

    let stage_execution_id = Uuid::new_v4();
    let scope_decision_id = Uuid::new_v4();
    let scope_snapshot_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status,completed_at) \
         VALUES($1,$2,'scoping','completed',NOW())",
    )
    .bind(stage_execution_id)
    .bind(operation_id)
    .execute(pool)
    .await
    .expect("insert authority scope decision stage");
    sqlx::query(
        r#"INSERT INTO operation_scope_decisions(
               id,operation_id,project_scope_id,stage_execution_id,
               root_organization_id,mode,decision_rows,decision_hash
           ) VALUES($1,$2,$3,$4,$5,'cli_flags',$6,$7)"#,
    )
    .bind(scope_decision_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(stage_execution_id)
    .bind(organization_id)
    .bind(serde_json::json!([{"organization_id": organization_id}]))
    .bind("authority-decision-hash")
    .execute(pool)
    .await
    .expect("insert authority scope decision");
    let mut tx = pool.begin().await.expect("begin authority scope freeze");
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
    .bind(project_path)
    .bind(organization_id)
    .bind("authority-scope-hash")
    .execute(&mut *tx)
    .await
    .expect("insert authority scope snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units(
               snapshot_id,organization_id,organization_name_at_freeze,role,
               depth,ordinal,decision_row_id,approval_source
           ) VALUES($1,$2,'Memory Authority Org','root',0,0,'root',$3)"#,
    )
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(serde_json::json!({"source": "cli_flags"}))
    .execute(&mut *tx)
    .await
    .expect("insert authority scope unit");
    sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
        .bind(scope_snapshot_id)
        .execute(&mut *tx)
        .await
        .expect("seal authority scope snapshot");
    tx.commit().await.expect("commit authority scope freeze");
}

#[tokio::test]
#[serial]
async fn canonical_episode_rolls_back_when_outbox_route_fails() {
    let (mut db, _data_dir) = fixture("memory_uow_rollback").await;
    let project = project_scopes::register_first_open(db.pool(), "/fixture/uow", "uow-sha")
        .await
        .expect("register stable project scope");
    let operation_id = Uuid::from_u128(0x1001);
    let organization_id = Uuid::from_u128(0x1002);
    let episode_id = Uuid::from_u128(0x1003);
    let source = SourceRef {
        source_kind: CanonicalSourceKind::StageEpisode,
        row_id: CanonicalRowId::Uuid(episode_id),
        source_stream_key: format!("stage-episode:{episode_id}"),
        version: 1,
    };
    let command = CloseEpisodeAndEmit {
        episode: StageEpisode {
            episode_id,
            scope: OperationScope {
                project_scope_id: ProjectScopeId(project.project_scope_id),
                source_operation_id: operation_id,
                organization_id_at_time: organization_id,
                scope_snapshot_hash: "fixture-scope-snapshot".to_string(),
            },
            stage_execution_id: Uuid::from_u128(0x1004),
            stage_run_unit_id: None,
            worker_run_id: None,
            candidate_attempt_id: None,
            stage_kind: "enumeration".to_string(),
            wave: None,
            verdict: EpisodeVerdict::Blocked,
            deliverable_submission_id: None,
            handoff_id: None,
            reason_codes: vec!["fixture_block".to_string()],
            fact_refs: Vec::new(),
            evidence_ids: Vec::new(),
            started_at: at(10),
            ended_at: at(11),
        },
        event: event(
            Uuid::from_u128(0x1005),
            project.project_scope_id,
            organization_id,
            operation_id,
            KnowledgeEventNameV1::StageEpisodeClosed,
            source,
        ),
    };

    sqlx::query(
        "DELETE FROM knowledge_projector_registry WHERE projector_name = 'assertion-promoter'",
    )
    .execute(db.pool())
    .await
    .expect("inject missing outbox route");
    let adapter = PgKnowledgeMemory::new(Arc::new(db.pool().clone()), None);
    let error = adapter
        .close_episode_and_emit(command)
        .await
        .expect_err("outbox failure must roll back canonical episode");
    assert_eq!(error.code(), "memory_port_failure");

    let episodes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stage_episodes")
        .fetch_one(db.pool())
        .await
        .expect("count episodes");
    let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM knowledge_outbox_events")
        .fetch_one(db.pool())
        .await
        .expect("count events");
    let deliveries: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM knowledge_projection_deliveries")
            .fetch_one(db.pool())
            .await
            .expect("count deliveries");
    assert_eq!((episodes, events, deliveries), (0, 0, 0));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn source_invalidation_preserves_assertion_hash_integrity_for_the_projector() {
    let (mut db, _data_dir) = fixture("memory_invalidation_hash_integrity").await;
    let project = project_scopes::register_first_open(
        db.pool(),
        "/fixture/invalidation-hash-integrity",
        "invalidation-hash-integrity-sha",
    )
    .await
    .expect("register stable project scope");
    let operation_id = Uuid::from_u128(0x1101);
    let organization_id = Uuid::from_u128(0x1102);
    let assertion_id = Uuid::from_u128(0x1103);
    let source = SourceRef {
        source_kind: CanonicalSourceKind::FactDelta,
        row_id: CanonicalRowId::Text("fact-delta:invalidation-hash".to_string()),
        source_stream_key: "fact-delta:invalidation-hash".to_string(),
        version: 1,
    };
    let object = AssertionObject::Json(serde_json::json!({
        "canonical_ref": "host:invalidation.example",
        "display_name": "invalidation.example",
        "properties": {"hostname": "invalidation.example"}
    }));
    let assertion = KnowledgeAssertionDraft {
        assertion_id,
        visibility: AssertionVisibility::OrganizationLongTerm {
            project_scope_id: ProjectScopeId(project.project_scope_id),
            organization_id_at_time: organization_id,
        },
        source_operation_id: operation_id,
        source_scope_snapshot_hash: "fixture-invalidation-scope".to_string(),
        source: source.clone(),
        identity: AssertionIdentity::derive(
            "host:invalidation.example",
            "graph.entity.host",
            &object,
        )
        .expect("derive invalidation assertion identity"),
        kind: AssertionKind::Observation,
        status: AssertionStatus::Active,
        object,
        classification: KnowledgeClassification::CustomerConfidential,
        evidence_ids: vec![211],
        valid_from: at(10),
        valid_to: None,
        fresh_until: Some(at(23)),
    }
    .validate()
    .expect("valid active assertion");
    let adapter = PgKnowledgeMemory::new(Arc::new(db.pool().clone()), None);
    adapter
        .promote_assertion_and_emit(PromoteAssertionAndEmit {
            assertion,
            event: event(
                Uuid::from_u128(0x1104),
                project.project_scope_id,
                organization_id,
                operation_id,
                KnowledgeEventNameV1::FactDeltaAccepted,
                source.clone(),
            ),
        })
        .await
        .expect("store active assertion and producer event");

    let mut invalidation_event = event(
        Uuid::from_u128(0x1105),
        project.project_scope_id,
        organization_id,
        operation_id,
        KnowledgeEventNameV1::SourceScopeInvalidated,
        source,
    );
    invalidation_event.occurred_at = at(13);
    adapter
        .invalidate_projection_chain_and_emit(InvalidateProjectionChainAndEmit {
            source: invalidation_event.payload.source.clone(),
            invalidated_at: invalidation_event.occurred_at,
            reason_code: "organization_deleted".to_string(),
            event: invalidation_event.clone(),
        })
        .await
        .expect("atomically invalidate assertion and append event");

    let stored = knowledge_assertions::get(db.pool(), assertion_id)
        .await
        .expect("invalidated assertion must remain hash-valid and readable");
    assert_eq!(stored.status, AssertionStatus::Expired);
    assert_eq!(stored.valid_to, Some(at(13)));

    let event_assertions =
        knowledge_assertions::list_for_event_source(db.pool(), &invalidation_event)
            .await
            .expect("assertion projector must be able to read the invalidated source");
    assert_eq!(event_assertions, vec![stored.clone()]);

    adapter
        .invalidate_projection_chain_and_emit(InvalidateProjectionChainAndEmit {
            source: invalidation_event.payload.source.clone(),
            invalidated_at: invalidation_event.occurred_at,
            reason_code: "organization_deleted".to_string(),
            event: invalidation_event,
        })
        .await
        .expect("exact invalidation replay must remain idempotent");
    assert_eq!(
        knowledge_assertions::get(db.pool(), assertion_id)
            .await
            .expect("replayed invalidation must keep the row hash-valid"),
        stored
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn crashed_projector_retries_and_two_sessions_keep_one_owner() {
    let (mut db, _data_dir) = fixture("memory_projector_replay").await;
    let project = project_scopes::register_first_open(db.pool(), "/fixture/replay", "replay-sha")
        .await
        .expect("register stable project scope");
    let operation_id = Uuid::from_u128(0x2001);
    let organization_id = Uuid::from_u128(0x2002);
    let source = SourceRef {
        source_kind: CanonicalSourceKind::FactDelta,
        row_id: CanonicalRowId::Text("fact-delta:replay".to_string()),
        source_stream_key: "fact-delta:replay".to_string(),
        version: 1,
    };
    let object = AssertionObject::Json(serde_json::json!({
        "canonical_ref": "host:example.com",
        "display_name": "example.com",
        "properties": {"hostname": "example.com"}
    }));
    let assertion = KnowledgeAssertionDraft {
        assertion_id: Uuid::from_u128(0x2003),
        visibility: AssertionVisibility::OrganizationLongTerm {
            project_scope_id: ProjectScopeId(project.project_scope_id),
            organization_id_at_time: organization_id,
        },
        source_operation_id: operation_id,
        source_scope_snapshot_hash: "fixture-scope-snapshot".to_string(),
        source: source.clone(),
        identity: AssertionIdentity::derive("host:example.com", "graph.entity.host", &object)
            .expect("derive graph assertion identity"),
        kind: AssertionKind::Observation,
        status: AssertionStatus::Active,
        object,
        classification: KnowledgeClassification::CustomerConfidential,
        evidence_ids: vec![201],
        valid_from: at(10),
        valid_to: None,
        fresh_until: Some(at(23)),
    }
    .validate()
    .expect("valid assertion");
    let typed_event = event(
        Uuid::from_u128(0x2004),
        project.project_scope_id,
        organization_id,
        operation_id,
        KnowledgeEventNameV1::FactDeltaAccepted,
        source,
    );
    let adapter = PgKnowledgeMemory::new(Arc::new(db.pool().clone()), None);
    adapter
        .promote_assertion_and_emit(PromoteAssertionAndEmit {
            assertion,
            event: typed_event.clone(),
        })
        .await
        .expect("atomic assertion + outbox");

    for projector in [
        ProjectorId::AssertionPromoterV1,
        ProjectorId::DocumentProjectorV1,
    ] {
        knowledge_outbox::set_projector_lifecycle(
            db.pool(),
            projector,
            knowledge_outbox::ProjectorLifecycle::Enabled,
            None,
        )
        .await
        .expect("enable fixture projector");
    }
    let assertion_delivery = knowledge_outbox::claim_delivery_batch(
        db.pool(),
        ProjectorId::AssertionPromoterV1,
        "fixture-assertion",
        1,
    )
    .await
    .expect("claim assertion delivery")
    .pop()
    .expect("assertion delivery present");
    knowledge_outbox::complete_delivery(
        db.pool(),
        assertion_delivery.event_id,
        ProjectorId::AssertionPromoterV1,
        "fixture-assertion",
        knowledge_outbox::DeliveryStatus::Succeeded,
        None,
    )
    .await
    .expect("complete assertion predecessor");
    let crashed = knowledge_outbox::claim_delivery_batch(
        db.pool(),
        ProjectorId::DocumentProjectorV1,
        "fixture-crashed-document",
        1,
    )
    .await
    .expect("claim document before simulated crash")
    .pop()
    .expect("document delivery present");
    sqlx::query(
        r#"UPDATE knowledge_projection_deliveries
           SET lease_expires_at = NOW() - INTERVAL '1 second'
           WHERE event_id = $1 AND projector_name = 'document-projector'"#,
    )
    .bind(crashed.event_id)
    .execute(db.pool())
    .await
    .expect("expire crashed projector lease");

    let runtime = KnowledgeMemoryRuntime::new(Arc::new(db.pool().clone()), None);
    let session_a_runtime = runtime.clone();
    let session_b_runtime = runtime.clone();
    let (session_a, session_b) = tokio::join!(session_a_runtime.start(), session_b_runtime.start());
    let outcomes = [
        session_a.expect("desktop session start"),
        session_b.expect("second session start"),
    ];
    assert!(outcomes.contains(&SupervisorStartOutcome::Started));
    assert!(outcomes.contains(&SupervisorStartOutcome::AlreadyRunning));
    assert_eq!(runtime.start_count(), 1);
    assert_eq!(runtime.owner_count(), 4);

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let deliveries = knowledge_outbox::list_deliveries(db.pool(), typed_event.event_id)
                .await
                .expect("load delivery states");
            let document_succeeded = deliveries.iter().any(|delivery| {
                delivery.projector_name == "document-projector"
                    && delivery.status == knowledge_outbox::DeliveryStatus::Succeeded
            });
            let graph_succeeded = deliveries.iter().any(|delivery| {
                delivery.projector_name == "graph-projector"
                    && delivery.status == knowledge_outbox::DeliveryStatus::Succeeded
            });
            if document_succeeded && graph_succeeded {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("expired crash lease was replayed");
    let documents: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM knowledge_documents")
        .fetch_one(db.pool())
        .await
        .expect("count deterministic documents");
    assert_eq!(documents, 1);
    let active_graph_entities: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM knowledge_graph_entities entity
             JOIN knowledge_graph_generations generation
               ON generation.generation_id=entity.generation_id
            WHERE generation.status='active'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("count active temporal graph entities");
    assert_eq!(active_graph_entities, 1);

    runtime.shutdown().await.expect("graceful CLI shutdown");
    assert!(!runtime.is_running());
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn bare_fact_delta_event_fails_closed_until_the_accepted_transition_exists() {
    let (mut db, _data_dir) = fixture("memory_fact_delta_fail_closed").await;
    let project = project_scopes::register_first_open(
        db.pool(),
        "/fixture/fact-delta-fail-closed",
        "fact-delta-fail-closed-sha",
    )
    .await
    .expect("register stable project scope");
    let operation_id = Uuid::from_u128(0x3001);
    let organization_id = Uuid::from_u128(0x3002);
    let fact_delta_id = Uuid::from_u128(0x3003);
    let source = SourceRef {
        source_kind: CanonicalSourceKind::FactDelta,
        row_id: CanonicalRowId::Uuid(fact_delta_id),
        source_stream_key: format!("fact-delta:{fact_delta_id}"),
        version: 1,
    };
    let typed_event = event(
        Uuid::from_u128(0x3004),
        project.project_scope_id,
        organization_id,
        operation_id,
        KnowledgeEventNameV1::FactDeltaAccepted,
        source,
    );
    let mut tx = db.pool().begin().await.expect("begin bare event append");
    knowledge_outbox::append_event_with_catalog_deliveries(&mut tx, &typed_event)
        .await
        .expect("append routed event without inventing an accepted assertion");
    tx.commit().await.expect("commit bare event append");

    let runtime = KnowledgeMemoryRuntime::new(Arc::new(db.pool().clone()), None);
    runtime
        .start()
        .await
        .expect("start production assertion promoter");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status = knowledge_outbox::get_delivery_status(
                db.pool(),
                typed_event.event_id,
                ProjectorId::AssertionPromoterV1,
            )
            .await
            .expect("load assertion delivery")
            .expect("routed assertion delivery exists");
            if status == knowledge_outbox::DeliveryStatus::RetryableFailed {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("bare FactDeltaAccepted must fail rather than acknowledge suppression");

    let (status, attempt_count, last_error): (String, i32, Option<String>) = sqlx::query_as(
        r#"SELECT status,attempt_count,last_error
              FROM knowledge_projection_deliveries
             WHERE event_id=$1
               AND projector_name='assertion-promoter'
               AND projector_schema_version=1"#,
    )
    .bind(typed_event.event_id)
    .fetch_one(db.pool())
    .await
    .expect("load fail-closed delivery evidence");
    assert_eq!(status, "retryable_failed");
    assert_eq!(attempt_count, 1);
    assert_eq!(last_error.as_deref(), Some("memory_policy_rejected"));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM knowledge_assertions WHERE source_stream_key=$1",
        )
        .bind(&typed_event.payload.source_stream_key)
        .fetch_one(db.pool())
        .await
        .expect("count assertions for a bare accepted-delta event"),
        0,
        "the projector must not invent FactDelta authority"
    );

    runtime.shutdown().await.expect("stop projector supervisor");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn reason_only_blocked_candidate_without_sealed_authority_fails_closed() {
    let (mut db, _data_dir) = fixture("memory_reason_only_candidate").await;
    let project = project_scopes::register_first_open(
        db.pool(),
        "/fixture/reason-only-candidate",
        "reason-only-candidate-sha",
    )
    .await
    .expect("register stable project scope");
    let operation_id = Uuid::from_u128(0x4001);
    let organization_id = Uuid::from_u128(0x4002);
    let attempt_id = Uuid::from_u128(0x4003);
    let typed_event = reason_only_candidate_event(
        Uuid::from_u128(0x4004),
        project.project_scope_id,
        organization_id,
        operation_id,
        attempt_id,
    );
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin Candidate event append");
    knowledge_outbox::append_event_with_catalog_deliveries(&mut tx, &typed_event)
        .await
        .expect("append reason-only blocked Candidate event");
    tx.commit().await.expect("commit Candidate event append");

    let runtime = KnowledgeMemoryRuntime::new(Arc::new(db.pool().clone()), None);
    runtime
        .start()
        .await
        .expect("start production assertion promoter");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status = knowledge_outbox::get_delivery_status(
                db.pool(),
                typed_event.event_id,
                ProjectorId::AssertionPromoterV1,
            )
            .await
            .expect("load Candidate assertion delivery")
            .expect("routed Candidate assertion delivery exists");
            if matches!(
                status,
                knowledge_outbox::DeliveryStatus::RetryableFailed
                    | knowledge_outbox::DeliveryStatus::SucceededSuppressed
            ) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("reason-only blocked Candidate must reach a terminal first attempt");

    let (status, attempt_count, last_error): (String, i32, Option<String>) = sqlx::query_as(
        r#"SELECT status,attempt_count,last_error
              FROM knowledge_projection_deliveries
             WHERE event_id=$1
               AND projector_name='assertion-promoter'
               AND projector_schema_version=1"#,
    )
    .bind(typed_event.event_id)
    .fetch_one(db.pool())
    .await
    .expect("load fail-closed authority evidence");
    assert_eq!(status, "retryable_failed");
    assert_eq!(attempt_count, 1);
    assert_eq!(last_error.as_deref(), Some("memory_policy_rejected"));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM knowledge_assertions WHERE source_stream_key=$1",
        )
        .bind(&typed_event.payload.source_stream_key)
        .fetch_one(db.pool())
        .await
        .expect("count reason-only Candidate assertions"),
        0,
        "a reason-only Candidate without sealed authority must not project"
    );

    runtime.shutdown().await.expect("stop projector supervisor");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn reason_only_blocked_candidate_with_sealed_authority_is_intentionally_suppressed() {
    let (mut db, _data_dir) = fixture("memory_authorized_reason_only_candidate").await;
    let project_path = "/fixture/authorized-reason-only-candidate";
    let project = project_scopes::register_first_open(
        db.pool(),
        project_path,
        "authorized-reason-only-candidate-sha",
    )
    .await
    .expect("register stable project scope");
    let operation_id = Uuid::from_u128(0x5001);
    let organization_id = Uuid::from_u128(0x5002);
    let attempt_id = Uuid::from_u128(0x5003);
    seed_sealed_scope_authority(
        db.pool(),
        project_path,
        project.project_scope_id,
        operation_id,
        organization_id,
    )
    .await;
    let typed_event = reason_only_candidate_event(
        Uuid::from_u128(0x5004),
        project.project_scope_id,
        organization_id,
        operation_id,
        attempt_id,
    );
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin authorized Candidate event append");
    knowledge_outbox::append_event_with_catalog_deliveries(&mut tx, &typed_event)
        .await
        .expect("append authorized reason-only blocked Candidate event");
    tx.commit()
        .await
        .expect("commit authorized Candidate event append");

    let runtime = KnowledgeMemoryRuntime::new(Arc::new(db.pool().clone()), None);
    runtime
        .start()
        .await
        .expect("start production assertion promoter");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status = knowledge_outbox::get_delivery_status(
                db.pool(),
                typed_event.event_id,
                ProjectorId::AssertionPromoterV1,
            )
            .await
            .expect("load authorized Candidate assertion delivery")
            .expect("routed authorized Candidate assertion delivery exists");
            if matches!(
                status,
                knowledge_outbox::DeliveryStatus::RetryableFailed
                    | knowledge_outbox::DeliveryStatus::SucceededSuppressed
            ) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("authorized reason-only Candidate must reach a terminal first attempt");

    let (status, attempt_count, terminal_reason, last_error): (
        String,
        i32,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        r#"SELECT status,attempt_count,terminal_reason,last_error
              FROM knowledge_projection_deliveries
             WHERE event_id=$1
               AND projector_name='assertion-promoter'
               AND projector_schema_version=1"#,
    )
    .bind(typed_event.event_id)
    .fetch_one(db.pool())
    .await
    .expect("load authorized intentional suppression evidence");
    assert_eq!(status, "succeeded_suppressed");
    assert_eq!(attempt_count, 1);
    assert_eq!(last_error, None);
    assert_eq!(
        terminal_reason.as_deref(),
        Some("memory_candidate_reason_only_blocked_no_audit_evidence")
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM knowledge_assertions WHERE source_stream_key=$1",
        )
        .bind(&typed_event.payload.source_stream_key)
        .fetch_one(db.pool())
        .await
        .expect("count authorized reason-only Candidate assertions"),
        0,
        "sealed authority allows suppression but blocker reason is still not audit evidence"
    );

    runtime.shutdown().await.expect("stop projector supervisor");
    db.stop().await;
}
