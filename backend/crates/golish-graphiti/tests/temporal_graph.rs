use chrono::{DateTime, TimeZone, Utc};
use golish_db::repo::{knowledge_assertions, knowledge_graph, knowledge_outbox, project_scopes};
use golish_db::{DbConfig, GolishDb};
use golish_graphiti::{
    identity_hash, GraphClient, GraphScopeKey, GraphVisibility, ProjectionWriteDisposition,
    ScopedGraphQuery, TemporalEntityProjection, TemporalEntityType, TemporalGraphClient,
    TemporalGraphInvalidation, TemporalGraphProjection, TemporalLineageProjection,
    TemporalRelationLineageProjection, TemporalRelationProjection, TemporalRelationType,
    TEMPORAL_GRAPH_SCHEMA_V1,
};
use golish_memory_domain::assertion::{
    AssertionIdentity, AssertionKind, AssertionObject, AssertionStatus, KnowledgeAssertion,
    KnowledgeAssertionDraft,
};
use golish_memory_domain::classification::{AssertionVisibility, KnowledgeClassification};
use golish_memory_domain::event_catalog::{
    KnowledgeEventEnvelopeV1, KnowledgeEventNameV1, KnowledgeEventPayloadV1, ProjectorId,
};
use golish_memory_domain::scope::ProjectScopeId;
use golish_memory_domain::source_ref::{CanonicalRowId, CanonicalSourceKind, SourceRef};
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
        database: format!("temporal_graph_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    };
    let db = GolishDb::start(config)
        .await
        .expect("start migrated embedded postgres");
    (db, data_dir)
}

fn timestamp(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 12, hour, 0, 0)
        .single()
        .expect("fixed timestamp")
}

#[allow(clippy::too_many_arguments)]
fn assertion(
    assertion_id: Uuid,
    visibility: AssertionVisibility,
    subject: &str,
    predicate: &str,
    object: serde_json::Value,
    kind: AssertionKind,
    stream: &str,
    version: i64,
    fresh_until: Option<DateTime<Utc>>,
) -> KnowledgeAssertion {
    let object = AssertionObject::Json(object);
    KnowledgeAssertionDraft {
        assertion_id,
        visibility,
        source_operation_id: Uuid::from_u128(0x9000),
        source_scope_snapshot_hash: "temporal-graph-scope-snapshot".to_string(),
        source: SourceRef {
            source_kind: CanonicalSourceKind::FactDelta,
            row_id: CanonicalRowId::Text(format!("{stream}:{version}:{assertion_id}")),
            source_stream_key: stream.to_string(),
            version,
        },
        identity: AssertionIdentity::derive(subject, predicate, &object)
            .expect("derive assertion identity"),
        kind,
        status: AssertionStatus::Active,
        object,
        classification: match kind {
            AssertionKind::TechniqueExperience => KnowledgeClassification::Internal,
            _ => KnowledgeClassification::CustomerConfidential,
        },
        evidence_ids: vec![100 + i64::from(assertion_id.as_bytes()[15])],
        valid_from: timestamp(10),
        valid_to: None,
        fresh_until,
    }
    .validate()
    .expect("valid assertion")
}

async fn insert_assertion(db: &GolishDb, assertion: &KnowledgeAssertion) {
    let mut connection = db.pool().acquire().await.expect("assertion connection");
    knowledge_assertions::insert_with_connection(&mut connection, assertion)
        .await
        .expect("insert canonical assertion");
}

fn scope_parts(
    visibility: &AssertionVisibility,
) -> (
    GraphScopeKey,
    GraphVisibility,
    Option<ProjectScopeId>,
    Option<Uuid>,
) {
    match visibility {
        AssertionVisibility::OrganizationLongTerm {
            project_scope_id,
            organization_id_at_time,
        } => (
            GraphScopeKey::organization(*project_scope_id, *organization_id_at_time),
            GraphVisibility::OrganizationLongTerm,
            Some(*project_scope_id),
            Some(*organization_id_at_time),
        ),
        AssertionVisibility::GlobalSanitized => (
            GraphScopeKey::global_sanitized(),
            GraphVisibility::GlobalSanitized,
            None,
            None,
        ),
    }
}

fn lineage(assertion: &KnowledgeAssertion, canonical_ref: &str) -> TemporalLineageProjection {
    TemporalLineageProjection {
        canonical_ref: canonical_ref.to_string(),
        assertion_id: assertion.assertion_id,
        source_stream_key: assertion.source.source_stream_key.clone(),
        source_version: assertion.source.version,
        evidence_refs: assertion.evidence_ids.clone(),
        status: assertion.status.as_str().to_string(),
        valid_from: assertion.valid_from,
        valid_to: assertion.valid_to,
        fresh_until: assertion.fresh_until,
        classification: assertion.classification,
        projection_schema_version: TEMPORAL_GRAPH_SCHEMA_V1,
    }
}

fn entity_projection(
    assertion: &KnowledgeAssertion,
    canonical_ref: &str,
    entity_type: TemporalEntityType,
    display_name: &str,
) -> TemporalGraphProjection {
    let (scope_key, visibility, project_scope_id, organization_id_at_time) =
        scope_parts(&assertion.visibility);
    TemporalGraphProjection {
        entities: vec![TemporalEntityProjection {
            identity_hash: identity_hash(&[
                scope_key.as_str(),
                canonical_ref,
                entity_type.as_str(),
            ]),
            scope_key,
            visibility,
            project_scope_id,
            organization_id_at_time,
            canonical_ref: canonical_ref.to_string(),
            entity_type,
            display_name: display_name.to_string(),
            properties: serde_json::json!({"fixture": true}),
        }],
        entity_lineages: vec![lineage(assertion, canonical_ref)],
        relations: Vec::new(),
        relation_lineages: Vec::new(),
    }
}

fn relation_projection(
    assertion: &KnowledgeAssertion,
    from_ref: &str,
    from_type: TemporalEntityType,
    to_ref: &str,
    to_type: TemporalEntityType,
    relation_type: TemporalRelationType,
) -> TemporalGraphProjection {
    let (scope_key, visibility, project_scope_id, organization_id_at_time) =
        scope_parts(&assertion.visibility);
    let entity = |canonical_ref: &str, entity_type: TemporalEntityType| TemporalEntityProjection {
        identity_hash: identity_hash(&[scope_key.as_str(), canonical_ref, entity_type.as_str()]),
        scope_key: scope_key.clone(),
        visibility,
        project_scope_id,
        organization_id_at_time,
        canonical_ref: canonical_ref.to_string(),
        entity_type,
        display_name: canonical_ref.to_string(),
        properties: serde_json::json!({"fixture": true}),
    };
    TemporalGraphProjection {
        entities: vec![entity(from_ref, from_type), entity(to_ref, to_type)],
        entity_lineages: vec![lineage(assertion, from_ref), lineage(assertion, to_ref)],
        relations: vec![TemporalRelationProjection {
            scope_key,
            from_canonical_ref: from_ref.to_string(),
            to_canonical_ref: to_ref.to_string(),
            relation_type,
            identity_hash: identity_hash(&[
                scope_parts(&assertion.visibility).0.as_str(),
                from_ref,
                relation_type.as_str(),
                to_ref,
            ]),
            properties: serde_json::json!({"protocol": "tcp"}),
        }],
        relation_lineages: vec![TemporalRelationLineageProjection {
            from_canonical_ref: from_ref.to_string(),
            to_canonical_ref: to_ref.to_string(),
            relation_type,
            assertion_id: assertion.assertion_id,
            source_stream_key: assertion.source.source_stream_key.clone(),
            source_version: assertion.source.version,
            evidence_refs: assertion.evidence_ids.clone(),
            status: assertion.status.as_str().to_string(),
            valid_from: assertion.valid_from,
            valid_to: assertion.valid_to,
            fresh_until: assertion.fresh_until,
            classification: assertion.classification,
            projection_schema_version: TEMPORAL_GRAPH_SCHEMA_V1,
        }],
    }
}

fn terminal_assertion(
    active: &KnowledgeAssertion,
    status: AssertionStatus,
    valid_to: DateTime<Utc>,
) -> KnowledgeAssertion {
    KnowledgeAssertionDraft {
        assertion_id: active.assertion_id,
        visibility: active.visibility.clone(),
        source_operation_id: active.source_operation_id,
        source_scope_snapshot_hash: active.source_scope_snapshot_hash.clone(),
        source: active.source.clone(),
        identity: active.identity.clone(),
        kind: active.kind,
        status,
        object: active.object.clone(),
        classification: active.classification,
        evidence_ids: active.evidence_ids.clone(),
        valid_from: active.valid_from,
        valid_to: Some(valid_to),
        fresh_until: active.fresh_until,
    }
    .validate()
    .expect("terminal assertion")
}

async fn close_canonical_and_graph(
    db: &GolishDb,
    client: &TemporalGraphClient,
    active: &KnowledgeAssertion,
    valid_to: DateTime<Utc>,
) {
    let terminal = terminal_assertion(active, AssertionStatus::Superseded, valid_to);
    sqlx::query(
        r#"UPDATE knowledge_assertions
           SET status = $2, valid_to = $3, content_hash = $4
           WHERE assertion_id = $1"#,
    )
    .bind(active.assertion_id)
    .bind(terminal.status.as_str())
    .bind(valid_to)
    .bind(&terminal.content_hash)
    .execute(db.pool())
    .await
    .expect("close canonical assertion");
    client
        .close_assertion_lineage(&TemporalGraphInvalidation {
            close_assertion_id: active.assertion_id,
            valid_to,
        })
        .await
        .expect("close graph lineage");
}

#[tokio::test]
#[serial]
async fn structured_temporal_graph_is_scoped_lineaged_fresh_and_rebuildable() {
    let (db, _data_dir) = fixture().await;
    let scope_a = project_scopes::register_first_open(db.pool(), "/fixture/graph-a", "a-sha")
        .await
        .expect("register scope a");
    let scope_b = project_scopes::register_first_open(db.pool(), "/fixture/graph-b", "b-sha")
        .await
        .expect("register scope b");
    let project_a = ProjectScopeId(scope_a.project_scope_id);
    let project_b = ProjectScopeId(scope_b.project_scope_id);
    let organization_a = Uuid::from_u128(0xa1);
    let organization_b = Uuid::from_u128(0xb1);
    let visibility_a = AssertionVisibility::OrganizationLongTerm {
        project_scope_id: project_a,
        organization_id_at_time: organization_a,
    };
    let visibility_b = AssertionVisibility::OrganizationLongTerm {
        project_scope_id: project_b,
        organization_id_at_time: organization_b,
    };
    let temporal = TemporalGraphClient::new(db.pool().clone());
    sqlx::query(
        r#"INSERT INTO organizations (id, project_path, name)
           VALUES ($1, '/fixture/graph-a', 'A'),
                  ($2, '/fixture/graph-b', 'B')"#,
    )
    .bind(organization_a)
    .bind(organization_b)
    .execute(db.pool())
    .await
    .expect("insert live organization bindings");
    assert!(knowledge_graph::organization_scope_is_registered_and_bound(
        db.pool(),
        project_a.0,
        organization_a,
    )
    .await
    .expect("authorize exact scope"));
    assert!(
        !knowledge_graph::organization_scope_is_registered_and_bound(
            db.pool(),
            project_a.0,
            organization_b,
        )
        .await
        .expect("reject sibling project binding")
    );

    for projector in [
        ProjectorId::AssertionPromoterV1,
        ProjectorId::DocumentProjectorV1,
        ProjectorId::GraphProjectorV1,
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
    let delivery_source = SourceRef {
        source_kind: CanonicalSourceKind::FactDelta,
        row_id: CanonicalRowId::Text("graph-delivery-fixture".to_string()),
        source_stream_key: "graph-delivery-fixture".to_string(),
        version: 1,
    };
    let delivery_event = KnowledgeEventEnvelopeV1 {
        event_id: Uuid::from_u128(0x50),
        project_scope_id: Some(project_a),
        organization_id_at_time: Some(organization_a),
        source_operation_id: Uuid::from_u128(0x9000),
        event_name: KnowledgeEventNameV1::FactDeltaAccepted,
        schema_version: 1,
        payload: KnowledgeEventPayloadV1 {
            source_stream_key: delivery_source.source_stream_key.clone(),
            source_version: delivery_source.version,
            source: delivery_source,
            structured_payload: serde_json::json!({"typed": true}),
        },
        occurred_at: timestamp(12),
    };
    let mut delivery_tx = db.pool().begin().await.expect("delivery tx");
    knowledge_outbox::append_event_with_catalog_deliveries(&mut delivery_tx, &delivery_event)
        .await
        .expect("append graph delivery event");
    delivery_tx.commit().await.expect("commit delivery event");
    assert!(knowledge_outbox::claim_delivery_batch(
        db.pool(),
        ProjectorId::GraphProjectorV1,
        "graph-predecessor-blocked",
        10,
    )
    .await
    .expect("graph predecessor claim")
    .is_empty());
    let assertion_claim = knowledge_outbox::claim_delivery_batch(
        db.pool(),
        ProjectorId::AssertionPromoterV1,
        "assertion-suppression-worker",
        10,
    )
    .await
    .expect("claim assertion predecessor");
    assert!(assertion_claim
        .iter()
        .any(|row| row.event_id == delivery_event.event_id));
    knowledge_outbox::complete_delivery(
        db.pool(),
        delivery_event.event_id,
        ProjectorId::AssertionPromoterV1,
        "assertion-suppression-worker",
        knowledge_outbox::DeliveryStatus::SucceededSuppressed,
        Some("fixture_policy"),
    )
    .await
    .expect("suppressed predecessor unblocks dependents");
    let graph_claim = knowledge_outbox::claim_delivery_batch(
        db.pool(),
        ProjectorId::GraphProjectorV1,
        "graph-worker",
        10,
    )
    .await
    .expect("claim graph independently");
    let document_claim = knowledge_outbox::claim_delivery_batch(
        db.pool(),
        ProjectorId::DocumentProjectorV1,
        "document-worker",
        10,
    )
    .await
    .expect("claim document independently");
    assert!(graph_claim
        .iter()
        .any(|row| row.event_id == delivery_event.event_id));
    assert!(document_claim
        .iter()
        .any(|row| row.event_id == delivery_event.event_id));
    knowledge_outbox::complete_delivery(
        db.pool(),
        delivery_event.event_id,
        ProjectorId::GraphProjectorV1,
        "graph-worker",
        knowledge_outbox::DeliveryStatus::Succeeded,
        None,
    )
    .await
    .expect("complete graph delivery");
    let independent_rows = knowledge_outbox::list_deliveries(db.pool(), delivery_event.event_id)
        .await
        .expect("read independent deliveries");
    assert_eq!(
        independent_rows
            .iter()
            .find(|row| row.projector_name == "document-projector")
            .expect("document delivery")
            .status,
        knowledge_outbox::DeliveryStatus::Leased
    );
    assert_eq!(
        knowledge_outbox::fail_delivery(
            db.pool(),
            delivery_event.event_id,
            ProjectorId::DocumentProjectorV1,
            "document-worker",
            "fixture_unknown_schema",
            1,
        )
        .await
        .expect("terminally fail exhausted delivery"),
        knowledge_outbox::DeliveryStatus::DeadLetter
    );

    // Legacy GraphClient storage remains usable and is not a temporal source.
    let legacy = GraphClient::new(db.pool().clone());
    legacy
        .upsert_entity(
            "host",
            "legacy-only.example",
            serde_json::json!({"legacy": true}),
            Some("legacy-project"),
        )
        .await
        .expect("legacy upsert remains compatible");
    let temporal_legacy = temporal
        .query(ScopedGraphQuery::for_organization(
            project_a,
            organization_a,
            "legacy-only",
            timestamp(12),
        ))
        .await
        .expect("temporal query");
    assert!(temporal_legacy.entities.is_empty());
    assert!(temporal_legacy.relations.is_empty());

    let host_a1 = assertion(
        Uuid::from_u128(0x101),
        visibility_a.clone(),
        "host:10.0.0.5",
        "graph.entity.host",
        serde_json::json!({"canonical_ref": "host:10.0.0.5"}),
        AssertionKind::Observation,
        "host-observation",
        1,
        None,
    );
    let host_a2 = assertion(
        Uuid::from_u128(0x102),
        visibility_a.clone(),
        "host:10.0.0.5",
        "graph.entity.host",
        serde_json::json!({"canonical_ref": "host:10.0.0.5"}),
        AssertionKind::Observation,
        "dns-observation",
        1,
        None,
    );
    insert_assertion(&db, &host_a1).await;
    insert_assertion(&db, &host_a2).await;
    let host_projection_a1 = entity_projection(
        &host_a1,
        "host:10.0.0.5",
        TemporalEntityType::Host,
        "10.0.0.5",
    );
    let host_projection_a2 = entity_projection(
        &host_a2,
        "host:10.0.0.5",
        TemporalEntityType::Host,
        "10.0.0.5",
    );
    temporal
        .apply_projection(&host_projection_a1)
        .await
        .expect("apply first host lineage");
    temporal
        .apply_projection(&host_projection_a2)
        .await
        .expect("apply second host lineage");
    let replay = temporal
        .apply_projection(&host_projection_a2)
        .await
        .expect("exact replay");
    assert_eq!(replay.disposition, ProjectionWriteDisposition::ExactReplay);

    let host_query = temporal
        .query(ScopedGraphQuery::for_organization(
            project_a,
            organization_a,
            "10.0.0.5",
            timestamp(12),
        ))
        .await
        .expect("query shared host");
    assert_eq!(host_query.entities.len(), 1);
    assert_eq!(host_query.entities[0].lineages.len(), 2);
    let active_generation_id = host_query.entities[0].generation_id;

    let active_scope = knowledge_graph::GraphScopeRecord::organization(project_a.0, organization_a);
    let active_attestation =
        knowledge_graph::active_generation(db.pool(), &active_scope, TEMPORAL_GRAPH_SCHEMA_V1)
            .await
            .expect("active generation")
            .expect("active generation exists");
    assert_eq!(active_attestation.entity_count, Some(1));
    assert_ne!(
        active_attestation.build_hash.as_deref(),
        Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
    );

    close_canonical_and_graph(&db, &temporal, &host_a1, timestamp(13)).await;
    let retained_host = temporal
        .query(ScopedGraphQuery::for_organization(
            project_a,
            organization_a,
            "10.0.0.5",
            timestamp(14),
        ))
        .await
        .expect("query retained host lineage");
    assert_eq!(retained_host.entities[0].lineages.len(), 1);
    assert_eq!(
        retained_host.entities[0].lineages[0].assertion_id,
        host_a2.assertion_id
    );

    let relation_a1 = assertion(
        Uuid::from_u128(0x201),
        visibility_a.clone(),
        "host:10.0.0.5/service:443",
        "graph.relation.runs_service",
        serde_json::json!({"from": "host:10.0.0.5", "to": "service:443"}),
        AssertionKind::Observation,
        "service-observation-a",
        1,
        None,
    );
    let relation_a2 = assertion(
        Uuid::from_u128(0x202),
        visibility_a.clone(),
        "host:10.0.0.5/service:443",
        "graph.relation.runs_service",
        serde_json::json!({"from": "host:10.0.0.5", "to": "service:443"}),
        AssertionKind::Observation,
        "service-observation-b",
        1,
        None,
    );
    insert_assertion(&db, &relation_a1).await;
    insert_assertion(&db, &relation_a2).await;
    let relation_projection_a1 = relation_projection(
        &relation_a1,
        "host:10.0.0.5",
        TemporalEntityType::Host,
        "service:443",
        TemporalEntityType::Service,
        TemporalRelationType::RunsService,
    );
    let relation_projection_a2 = relation_projection(
        &relation_a2,
        "host:10.0.0.5",
        TemporalEntityType::Host,
        "service:443",
        TemporalEntityType::Service,
        TemporalRelationType::RunsService,
    );
    temporal
        .apply_projection(&relation_projection_a1)
        .await
        .expect("apply first relation lineage");
    temporal
        .apply_projection(&relation_projection_a2)
        .await
        .expect("apply second relation lineage");
    let relation_query = temporal
        .query(ScopedGraphQuery::for_organization(
            project_a,
            organization_a,
            "runs_service",
            timestamp(12),
        ))
        .await
        .expect("query relation lineages");
    assert_eq!(relation_query.relations.len(), 1);
    assert_eq!(relation_query.relations[0].lineages.len(), 2);
    close_canonical_and_graph(&db, &temporal, &relation_a1, timestamp(13)).await;
    let retained_relation = temporal
        .query(ScopedGraphQuery::for_organization(
            project_a,
            organization_a,
            "runs_service",
            timestamp(14),
        ))
        .await
        .expect("query retained relation");
    assert_eq!(retained_relation.relations.len(), 1);
    assert_eq!(retained_relation.relations[0].lineages.len(), 1);
    assert_eq!(
        retained_relation.relations[0].lineages[0].assertion_id,
        relation_a2.assertion_id
    );

    // An active relation lineage is still hidden if either endpoint has no
    // current lineage.
    sqlx::query(
        r#"DELETE FROM knowledge_graph_entity_assertions
           WHERE generation_id = $1
             AND entity_id = $2"#,
    )
    .bind(retained_relation.relations[0].generation_id)
    .bind(retained_relation.relations[0].to_entity_id)
    .execute(db.pool())
    .await
    .expect("remove endpoint lineages hostile fixture");
    let hidden_without_endpoint = temporal
        .query(ScopedGraphQuery::for_organization(
            project_a,
            organization_a,
            "runs_service",
            timestamp(14),
        ))
        .await
        .expect("query relation without current endpoint");
    assert!(hidden_without_endpoint.relations.is_empty());

    let expired = assertion(
        Uuid::from_u128(0x301),
        visibility_a.clone(),
        "host:expired",
        "graph.entity.host",
        serde_json::json!({"canonical_ref": "host:expired"}),
        AssertionKind::Observation,
        "freshness-stream",
        1,
        Some(timestamp(11)),
    );
    insert_assertion(&db, &expired).await;
    temporal
        .apply_projection(&entity_projection(
            &expired,
            "host:expired",
            TemporalEntityType::Host,
            "expired-host",
        ))
        .await
        .expect("apply expiring entity");
    assert!(temporal
        .query(ScopedGraphQuery::for_organization(
            project_a,
            organization_a,
            "expired-host",
            timestamp(12),
        ))
        .await
        .expect("freshness query")
        .entities
        .is_empty());

    let sibling = assertion(
        Uuid::from_u128(0x401),
        visibility_b.clone(),
        "host:10.0.0.5",
        "graph.entity.host",
        serde_json::json!({"canonical_ref": "host:10.0.0.5"}),
        AssertionKind::Observation,
        "host-observation",
        1,
        None,
    );
    insert_assertion(&db, &sibling).await;
    let sibling_projection = entity_projection(
        &sibling,
        "host:10.0.0.5",
        TemporalEntityType::Host,
        "10.0.0.5",
    );
    temporal
        .apply_projection(&sibling_projection)
        .await
        .expect("apply sibling scope projection");
    assert_ne!(
        host_projection_a2.entities[0].identity_hash,
        sibling_projection.entities[0].identity_hash
    );
    let sibling_query = temporal
        .query(ScopedGraphQuery::for_organization(
            project_b,
            organization_b,
            "10.0.0.5",
            timestamp(12),
        ))
        .await
        .expect("query sibling scope");
    assert_eq!(sibling_query.entities.len(), 1);
    assert_eq!(sibling_query.entities[0].project_scope_id, Some(project_b));

    let cross_scope_relation_error = sqlx::query(
        r#"INSERT INTO knowledge_graph_relations (
               relation_id, generation_id, scope_key, from_entity_id,
               to_entity_id, relation_type, identity_hash, properties
           ) VALUES ($1,$2,$3,$4,$5,'contains',$6,'{}'::jsonb)"#,
    )
    .bind(Uuid::from_u128(0x402))
    .bind(active_generation_id)
    .bind(active_scope.scope_key.clone())
    .bind(retained_host.entities[0].entity_id)
    .bind(sibling_query.entities[0].entity_id)
    .bind("4".repeat(64))
    .execute(db.pool())
    .await
    .expect_err("cross-generation/scope relation must fail");
    assert_eq!(
        cross_scope_relation_error
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("23503")
    );

    let technique = assertion(
        Uuid::from_u128(0x501),
        AssertionVisibility::GlobalSanitized,
        "technique:T1190",
        "graph.entity.technique",
        serde_json::json!({"canonical_ref": "technique:T1190"}),
        AssertionKind::TechniqueExperience,
        "global-technique",
        1,
        None,
    );
    insert_assertion(&db, &technique).await;
    temporal
        .apply_projection(&entity_projection(
            &technique,
            "technique:T1190",
            TemporalEntityType::Technique,
            "T1190",
        ))
        .await
        .expect("apply safe global technique");
    let global_query = temporal
        .query(ScopedGraphQuery::global_sanitized("T1190", timestamp(12)))
        .await
        .expect("query global technique");
    assert_eq!(global_query.entities.len(), 1);
    let global_host_error = sqlx::query(
        r#"INSERT INTO knowledge_graph_entities (
               entity_id, generation_id, scope_key, visibility,
               canonical_ref, identity_hash, entity_type, display_name, properties
           ) VALUES ($1,$2,'global_sanitized','global_sanitized',
                     'host:forbidden',$3,'host','forbidden','{}'::jsonb)"#,
    )
    .bind(Uuid::from_u128(0x502))
    .bind(global_query.entities[0].generation_id)
    .bind("5".repeat(64))
    .execute(db.pool())
    .await
    .expect_err("global host identity must fail closed");
    assert_eq!(
        global_host_error
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("23514")
    );

    let v5 = assertion(
        Uuid::from_u128(0x601),
        visibility_a.clone(),
        "host:versioned",
        "graph.entity.host",
        serde_json::json!({"canonical_ref": "host:versioned"}),
        AssertionKind::Observation,
        "versioned-stream",
        5,
        None,
    );
    let v4 = assertion(
        Uuid::from_u128(0x602),
        visibility_a.clone(),
        "host:versioned",
        "graph.entity.host",
        serde_json::json!({"canonical_ref": "host:versioned"}),
        AssertionKind::Observation,
        "versioned-stream",
        4,
        None,
    );
    insert_assertion(&db, &v5).await;
    insert_assertion(&db, &v4).await;
    temporal
        .apply_projection(&entity_projection(
            &v5,
            "host:versioned",
            TemporalEntityType::Host,
            "versioned-host",
        ))
        .await
        .expect("apply higher source version");
    let stale = temporal
        .apply_projection(&entity_projection(
            &v4,
            "host:versioned",
            TemporalEntityType::Host,
            "versioned-host",
        ))
        .await
        .expect("stale disposition");
    assert_eq!(
        stale.disposition,
        ProjectionWriteDisposition::Stale { current_version: 5 }
    );
    let mut mixed = entity_projection(
        &v5,
        "host:versioned",
        TemporalEntityType::Host,
        "versioned-host",
    );
    let mut mixed_lineage = mixed.entity_lineages[0].clone();
    mixed_lineage.source_stream_key = "smuggled-stream".to_string();
    mixed_lineage.source_version = 1;
    mixed.entity_lineages.push(mixed_lineage);
    assert_eq!(
        temporal
            .apply_projection(&mixed)
            .await
            .expect_err("mixed source tuple must fail")
            .code(),
        "graph_invalid_argument"
    );

    let forged = assertion(
        Uuid::from_u128(0x701),
        visibility_a.clone(),
        "host:forged",
        "graph.entity.host",
        serde_json::json!({"canonical_ref": "host:forged"}),
        AssertionKind::Observation,
        "forged-stream",
        1,
        None,
    );
    insert_assertion(&db, &forged).await;
    let forged_lineage_error = sqlx::query(
        r#"INSERT INTO knowledge_graph_entity_assertions (
               entity_id, generation_id, assertion_id, source_stream_key,
               source_version, evidence_refs, status, valid_from,
               classification, projection_schema_version
           ) VALUES ($1,$2,$3,'wrong-stream',1,$4,'active',$5,
                     'customer_confidential',1)"#,
    )
    .bind(retained_host.entities[0].entity_id)
    .bind(active_generation_id)
    .bind(forged.assertion_id)
    .bind(&forged.evidence_ids)
    .bind(forged.valid_from)
    .execute(db.pool())
    .await
    .expect_err("forged lineage must fail trigger");
    assert_eq!(
        forged_lineage_error
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("P0001")
    );

    let malformed_generation_error = sqlx::query(
        r#"INSERT INTO knowledge_graph_generations (
               generation_id, scope_key, visibility, project_scope_id,
               organization_id_at_time, projection_schema_version,
               status, completed_at
           ) VALUES ($1,$2,'organization_long_term',$3,$4,1,'building',NOW())"#,
    )
    .bind(Uuid::from_u128(0x702))
    .bind(format!("org:{}:{}", project_a.0, Uuid::from_u128(0xa2)))
    .bind(project_a.0)
    .bind(Uuid::from_u128(0xa2))
    .execute(db.pool())
    .await
    .expect_err("invalid building state shape must fail");
    assert_eq!(
        malformed_generation_error
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("23514")
    );

    // Rebuild writes remain invisible, one-building fencing prevents a second
    // owner, and generation-independent manifests hash identically.
    let generation_before_rebuild = temporal
        .query(ScopedGraphQuery::for_organization(
            project_a,
            organization_a,
            "10.0.0.5",
            timestamp(14),
        ))
        .await
        .expect("query before rebuild")
        .entities[0]
        .generation_id;
    let building = temporal
        .begin_rebuild(
            &GraphScopeKey::organization(project_a, organization_a),
            Some(project_a),
            Some(organization_a),
            TEMPORAL_GRAPH_SCHEMA_V1,
        )
        .await
        .expect("begin rebuild");
    assert_eq!(
        temporal
            .begin_rebuild(
                &GraphScopeKey::organization(project_a, organization_a),
                Some(project_a),
                Some(organization_a),
                TEMPORAL_GRAPH_SCHEMA_V1,
            )
            .await
            .expect_err("concurrent rebuild must be fenced")
            .code(),
        "knowledge_graph_database_error"
    );
    temporal
        .apply_projection_to_generation(building.generation_id, &host_projection_a2)
        .await
        .expect("populate building generation");
    let still_old = temporal
        .query(ScopedGraphQuery::for_organization(
            project_a,
            organization_a,
            "10.0.0.5",
            timestamp(14),
        ))
        .await
        .expect("building remains invisible");
    assert_eq!(
        still_old.entities[0].generation_id,
        generation_before_rebuild
    );
    let first_attestation = temporal
        .generation_attestation(building.generation_id)
        .await
        .expect("attest first rebuild");
    let activated = temporal
        .activate_rebuild(building.generation_id, &first_attestation)
        .await
        .expect("activate first rebuild");
    assert_ne!(activated.generation_id, generation_before_rebuild);
    let after_cutover = temporal
        .query(ScopedGraphQuery::for_organization(
            project_a,
            organization_a,
            "10.0.0.5",
            timestamp(14),
        ))
        .await
        .expect("query after cutover");
    assert_eq!(
        after_cutover.entities[0].generation_id,
        activated.generation_id
    );

    let failed_build = temporal
        .begin_rebuild(
            &GraphScopeKey::organization(project_a, organization_a),
            Some(project_a),
            Some(organization_a),
            TEMPORAL_GRAPH_SCHEMA_V1,
        )
        .await
        .expect("begin second rebuild");
    temporal
        .apply_projection_to_generation(failed_build.generation_id, &host_projection_a2)
        .await
        .expect("populate identical second rebuild");
    let second_attestation = temporal
        .generation_attestation(failed_build.generation_id)
        .await
        .expect("attest second rebuild");
    assert_eq!(first_attestation, second_attestation);
    temporal
        .fail_rebuild(failed_build.generation_id, "fixture_failure")
        .await
        .expect("fail second rebuild");
    assert!(temporal
        .activate_rebuild(failed_build.generation_id, &second_attestation)
        .await
        .is_err());
    let active_after_failure = temporal
        .query(ScopedGraphQuery::for_organization(
            project_a,
            organization_a,
            "10.0.0.5",
            timestamp(14),
        ))
        .await
        .expect("failed rebuild preserves active generation");
    assert_eq!(
        active_after_failure.entities[0].generation_id,
        activated.generation_id
    );
    temporal
        .discard_rebuild(failed_build.generation_id)
        .await
        .expect("discard failed rebuild");
}
