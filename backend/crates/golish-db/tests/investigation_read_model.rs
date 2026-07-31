use golish_core::{
    hypothesis_semantic_key::CanonicalJsonObject,
    investigation_projection::{
        GenerationProjectionRecordV1, HypothesisProjectionRecordV1,
        HypothesisVerificationPlanProjectionRecordV1, ProjectionChangeKind,
        ProjectionInvalidationReason, ProjectionSourceSnapshotV1, ProjectionSourceTimeStatusV1,
        ResidualProjectionRecordV1, TimelineEventKind,
    },
};
use golish_db::{
    repo::investigation_projection::{
        capture_investigation_read_authority, enqueue_projection_batch_on,
        get_investigation_hypothesis, list_investigation_hypotheses, project_projection_batch,
        read_investigation_summary, read_investigation_timeline, InvestigationHypothesisFilters,
        InvestigationHypothesisListQuery, InvestigationPageValidationInput,
        InvestigationProjectionError, InvestigationTimelineQuery, ProjectionOutboxBatchInput,
        ProjectionOutboxMemberInput, ProjectionSourceStorageV1,
    },
    DbConfig, GolishDb,
};
use serial_test::serial;
use sqlx::PgPool;
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
        database: format!("investigation_{label}_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start isolated migrated postgres");
    (db, data_dir)
}

async fn seed_operation(pool: &PgPool, operation_id: Uuid, organization_ids: &[Uuid]) {
    let project_scope_id = Uuid::new_v4();
    let project_path = format!("/tmp/investigation-read-model-{}", Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO project_scopes(project_scope_id,canonical_project_path,path_sha256) \
         VALUES($1,$2,$3)",
    )
    .bind(project_scope_id)
    .bind(&project_path)
    .bind(digest('0'))
    .execute(pool)
    .await
    .expect("insert read-model project scope");
    for (ordinal, organization_id) in organization_ids.iter().enumerate() {
        sqlx::query(
            "INSERT INTO organizations(id,project_path,name,sort_order) VALUES($1,$2,$3,$4)",
        )
        .bind(organization_id)
        .bind(&project_path)
        .bind(format!("read-model-org-{organization_id}"))
        .bind(ordinal as i32)
        .execute(pool)
        .await
        .expect("insert organization");
    }
    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,
               attack_execution_contract,tool_truth_contract,project_scope_id
           ) VALUES($1,'assessment','target_intel','legacy_v1','legacy','legacy_v1',$2)"#,
    )
    .bind(operation_id)
    .bind(project_scope_id)
    .execute(pool)
    .await
    .expect("insert operation");

    let Some(root_organization_id) = organization_ids.first().copied() else {
        return;
    };
    let stage_run_id = Uuid::new_v4();
    let scope_decision_id = Uuid::new_v4();
    let scope_snapshot_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status) \
         VALUES($1,$2,'target_intel','started')",
    )
    .bind(stage_run_id)
    .bind(operation_id)
    .execute(pool)
    .await
    .expect("insert read-model scope stage");
    sqlx::query(
        r#"INSERT INTO operation_scope_decisions(
               id,operation_id,project_scope_id,stage_execution_id,
               root_organization_id,mode,decision_rows,decision_hash
           ) VALUES($1,$2,$3,$4,$5,'cli_flags',$6,$7)"#,
    )
    .bind(scope_decision_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(stage_run_id)
    .bind(root_organization_id)
    .bind(serde_json::json!(organization_ids
        .iter()
        .map(|organization_id| serde_json::json!({"organization_id": organization_id}))
        .collect::<Vec<_>>()))
    .bind(digest('1'))
    .execute(pool)
    .await
    .expect("insert read-model scope decision");
    let mut tx = pool.begin().await.expect("begin read-model scope freeze");
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
    .bind(root_organization_id)
    .bind(digest('2'))
    .execute(&mut *tx)
    .await
    .expect("insert read-model scope snapshot");
    for (ordinal, organization_id) in organization_ids.iter().copied().enumerate() {
        sqlx::query(
            r#"INSERT INTO operation_org_scope_units(
                   snapshot_id,organization_id,parent_organization_id,
                   organization_name_at_freeze,role,depth,ordinal,
                   ownership_percent,decision_row_id,approval_source
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)"#,
        )
        .bind(scope_snapshot_id)
        .bind(organization_id)
        .bind((ordinal > 0).then_some(root_organization_id))
        .bind(format!("Read Model Org {ordinal}"))
        .bind(if ordinal == 0 { "root" } else { "subsidiary" })
        .bind(if ordinal == 0 { 0 } else { 1 })
        .bind(ordinal as i32)
        .bind((ordinal > 0).then_some(100_i32))
        .bind(format!("scope-{ordinal}"))
        .bind(serde_json::json!({"source": "fixture"}))
        .execute(&mut *tx)
        .await
        .expect("insert read-model scope unit");
    }
    sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
        .bind(scope_snapshot_id)
        .execute(&mut *tx)
        .await
        .expect("seal read-model scope snapshot");
    tx.commit().await.expect("commit read-model scope freeze");
}

fn digest(byte: char) -> String {
    format!("sha256:{}", byte.to_string().repeat(64))
}

fn hypothesis_source(
    root_id: Uuid,
    revision_id: Uuid,
    organization_id: Uuid,
    revision_ordinal: i32,
    predecessor_revision_id: Option<Uuid>,
    state: &str,
    identity_byte: char,
) -> ProjectionSourceSnapshotV1 {
    let identity_hash = digest(identity_byte);
    let body = CanonicalJsonObject::try_from_value(serde_json::json!({
        "source_generation_id": Uuid::new_v4(),
        "root_id": root_id,
        "revision_id": revision_id,
        "revision_ordinal": revision_ordinal,
        "predecessor_revision_id": predecessor_revision_id,
        "revision_hash": digest('a'),
        "revision_ingredients_hash": digest('b'),
        "semantic_key": {
            "schema": "hypothesis_semantic_key.v1",
            "organization_id": organization_id,
            "subject": {"kind": "domain", "identity_hash": identity_hash},
            "predicate": {
                "schema": "dns_takeover.v1",
                "version": 1,
                "normalized_arguments": {"record_type": "CNAME"}
            },
            "trust_boundary": "public_dns",
            "polarity": "positive"
        },
        "semantic_key_hash": digest(identity_byte),
        "state": state,
        "lifecycle_state": "current",
        "planning_readiness": "ready_for_strategy",
        "target_type_at_time": "subject_identity_hash",
        "target_value_at_time": identity_hash,
        "origin_decision_hash": digest('c'),
        "proposal": {
            "kind": "hypothesis_proposal",
            "proposal_id": Uuid::new_v4(),
            "subject_kind": "domain",
            "subject_identity_hash": identity_hash,
            "predicate": {
                "schema": "dns_takeover.v1",
                "version": 1,
                "normalized_arguments": {"record_type": "CNAME"}
            },
            "trust_boundary": "public_dns",
            "polarity": "positive",
            "prose": "must not cross the read-model boundary",
            "confidence": 70,
            "priority": 2,
            "tags": ["dns"],
            "evidence_refs": ["evidence-a"]
        },
        "proof_refs": [
            {"ToolTruthEvidence": "receipt-a"},
            {"ApplicationContext": "context-a"}
        ],
        "refutation_refs": [
            {"Finding": "finding-a"},
            {"Gap": "gap-a"}
        ],
        "relation_sources": []
    }))
    .expect("canonical Hypothesis body");
    ProjectionSourceSnapshotV1::Hypothesis(
        HypothesisProjectionRecordV1::try_new(
            root_id.to_string(),
            u64::try_from(revision_ordinal + 1).expect("positive version"),
            1,
            body,
        )
        .expect("typed Hypothesis source"),
    )
}

fn generation_source(generation_id: Uuid, snapshot_id: Uuid) -> ProjectionSourceSnapshotV1 {
    let body = CanonicalJsonObject::try_from_value(serde_json::json!({
        "generation_id": generation_id,
        "candidate_snapshot_id": snapshot_id,
        "generation_hash": digest('d')
    }))
    .expect("canonical generation body");
    ProjectionSourceSnapshotV1::Generation(
        GenerationProjectionRecordV1::try_new(generation_id.to_string(), 1, 1, body)
            .expect("typed generation source"),
    )
}

fn invalidated_residual_source(residual_id: Uuid) -> ProjectionSourceSnapshotV1 {
    let body = CanonicalJsonObject::try_from_value(serde_json::json!({
        "residual_id": residual_id,
        "residual_hash": digest('e'),
        "reason": "source_deleted"
    }))
    .expect("canonical residual body");
    ProjectionSourceSnapshotV1::Residual(
        ResidualProjectionRecordV1::try_new(residual_id.to_string(), 1, 1, body)
            .expect("typed residual source"),
    )
}

fn scoped_residual_source(
    residual_id: Uuid,
    root_id: Uuid,
    revision_id: Uuid,
) -> ProjectionSourceSnapshotV1 {
    let body = CanonicalJsonObject::try_from_value(serde_json::json!({
        "residual_id": residual_id,
        "residual_hash": digest('f'),
        "reason": "scoped_residual",
        "root_id": root_id,
        "revision_id": revision_id
    }))
    .expect("canonical scoped residual body");
    ProjectionSourceSnapshotV1::Residual(
        ResidualProjectionRecordV1::try_new(residual_id.to_string(), 1, 1, body)
            .expect("typed scoped residual source"),
    )
}

fn malformed_unrelated_residual_source(
    residual_id: Uuid,
    unrelated_root_id: Uuid,
    unrelated_revision_id: Uuid,
) -> ProjectionSourceSnapshotV1 {
    let body = CanonicalJsonObject::try_from_value(serde_json::json!({
        "root_id": unrelated_root_id,
        "revision_id": unrelated_revision_id,
        "malformed": true
    }))
    .expect("canonical malformed unrelated residual body");
    ProjectionSourceSnapshotV1::Residual(
        ResidualProjectionRecordV1::try_new(residual_id.to_string(), 1, 1, body)
            .expect("typed malformed unrelated residual source"),
    )
}

fn malformed_unrelated_plan_source(
    plan_id: Uuid,
    unrelated_revision_id: Uuid,
) -> ProjectionSourceSnapshotV1 {
    let body = CanonicalJsonObject::try_from_value(serde_json::json!({
        "revision_id": unrelated_revision_id,
        "malformed": true
    }))
    .expect("canonical malformed unrelated verification-plan body");
    ProjectionSourceSnapshotV1::HypothesisVerificationPlan(
        HypothesisVerificationPlanProjectionRecordV1::try_new(plan_id.to_string(), 1, 1, body)
            .expect("typed malformed unrelated verification-plan source"),
    )
}

fn member(
    source: ProjectionSourceSnapshotV1,
    change_kind: ProjectionChangeKind,
    invalidation_reason: Option<ProjectionInvalidationReason>,
) -> ProjectionOutboxMemberInput {
    ProjectionOutboxMemberInput {
        outbox_member_id: Uuid::new_v4(),
        change_kind,
        source,
        source_occurred_at: None,
        source_time_status: ProjectionSourceTimeStatusV1::HistoricalUnknown,
        invalidation_reason,
        storage: ProjectionSourceStorageV1::Inline,
    }
}

async fn enqueue_and_project(
    pool: &PgPool,
    operation_id: Uuid,
    members: Vec<ProjectionOutboxMemberInput>,
) {
    let batch_id = Uuid::new_v4();
    let mut tx = pool.begin().await.expect("begin source transaction");
    enqueue_projection_batch_on(
        &mut tx,
        ProjectionOutboxBatchInput {
            batch_id,
            operation_id,
            project_scope_id: None,
            stable_request_id: Uuid::new_v4(),
            source_transaction_id: Uuid::new_v4(),
            source_occurred_at: None,
            source_time_status: ProjectionSourceTimeStatusV1::HistoricalUnknown,
            members,
        },
    )
    .await
    .expect("enqueue source batch");
    tx.commit().await.expect("commit source batch");
    project_projection_batch(pool, operation_id, batch_id)
        .await
        .expect("project source batch");
}

fn first_page_query(page_size: u32) -> InvestigationHypothesisListQuery {
    InvestigationHypothesisListQuery {
        filters: InvestigationHypothesisFilters::default(),
        after: None,
        expected_page_authority: None,
        page_size,
    }
}

#[tokio::test]
#[serial]
async fn projection_read_head_isolation_materialized_at_time_identity_and_typed_timeline() {
    let (db, _data_dir) = fixture("head-isolation").await;
    let operation_id = Uuid::new_v4();
    let organizations = [Uuid::new_v4(), Uuid::new_v4()];
    seed_operation(db.pool(), operation_id, &organizations).await;
    let roots = [Uuid::new_v4(), Uuid::new_v4()];
    let revisions = [Uuid::new_v4(), Uuid::new_v4()];
    let generation_id = Uuid::new_v4();
    let candidate_snapshot_id = Uuid::new_v4();

    let before = list_investigation_hypotheses(db.pool(), operation_id, first_page_query(100))
        .await
        .expect("read empty materialized head");
    assert_eq!(before.authority.temporal.as_of_change_seq, 0);
    assert!(before.hypotheses.is_empty());

    enqueue_and_project(
        db.pool(),
        operation_id,
        vec![
            member(
                generation_source(generation_id, candidate_snapshot_id),
                ProjectionChangeKind::Insert,
                None,
            ),
            member(
                hypothesis_source(
                    roots[0],
                    revisions[0],
                    organizations[0],
                    0,
                    None,
                    "supported",
                    '1',
                ),
                ProjectionChangeKind::Insert,
                None,
            ),
            member(
                hypothesis_source(
                    roots[1],
                    revisions[1],
                    organizations[1],
                    0,
                    None,
                    "contested",
                    '2',
                ),
                ProjectionChangeKind::Insert,
                None,
            ),
            member(
                invalidated_residual_source(Uuid::new_v4()),
                ProjectionChangeKind::Invalidate,
                Some(ProjectionInvalidationReason::SourceDeleted),
            ),
            member(
                scoped_residual_source(Uuid::new_v4(), roots[0], revisions[0]),
                ProjectionChangeKind::Insert,
                None,
            ),
        ],
    )
    .await;

    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM attack_hypothesis_revisions WHERE operation_id=$1",
        )
        .bind(operation_id)
        .fetch_one(db.pool())
        .await
        .expect("count canonical revisions"),
        0,
        "read model fixture intentionally has no canonical rows"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM targets")
            .fetch_one(db.pool())
            .await
            .expect("count live targets"),
        0,
        "at-time identity must not depend on a live target"
    );

    let page = list_investigation_hypotheses(db.pool(), operation_id, first_page_query(100))
        .await
        .expect("read materialized Hypotheses");
    assert_eq!(page.hypotheses.len(), 2);
    assert!(page
        .hypotheses
        .iter()
        .all(|item| item.target_type_at_time == "subject_identity_hash"));
    assert!(page.hypotheses.iter().all(|item| {
        item.target_value_at_time == item.subject_identity_hash
            && item.predicate_summary.contains("dns_takeover.v1")
    }));
    assert_eq!(
        page.hypotheses
            .iter()
            .find(|item| item.revision_id == revisions[0])
            .expect("scoped residual Hypothesis")
            .residual_codes,
        ["scoped_residual"]
    );
    assert!(page
        .hypotheses
        .iter()
        .find(|item| item.revision_id == revisions[1])
        .expect("unrelated Hypothesis")
        .residual_codes
        .is_empty());

    let detail = get_investigation_hypothesis(db.pool(), operation_id, revisions[0])
        .await
        .expect("read materialized detail")
        .expect("detail exists");
    assert_eq!(detail.hypothesis.revision_id, revisions[0]);
    assert_eq!(detail.support_ref_ids, ["receipt-a"]);
    assert_eq!(detail.contradiction_ref_ids, ["finding-a"]);
    assert_eq!(detail.application_context_ref_ids, ["context-a"]);
    assert_eq!(detail.gap_ref_ids, ["gap-a"]);
    assert!(detail
        .legacy_unavailable_fields
        .contains(&"legacy_candidate".to_owned()));

    let timeline = read_investigation_timeline(
        db.pool(),
        operation_id,
        InvestigationTimelineQuery {
            after: None,
            page_size: 100,
        },
    )
    .await
    .expect("read typed Timeline");
    assert_eq!(timeline.events.len(), 5);
    assert!(timeline.events.iter().all(|event| {
        event.source_occurred_at.is_none()
            && event.source_time_status == ProjectionSourceTimeStatusV1::HistoricalUnknown
    }));
    let invalidation = timeline
        .events
        .iter()
        .find(|event| event.event_kind == TimelineEventKind::ResidualInvalidated)
        .expect("typed residual invalidation event");
    assert_eq!(
        invalidation.invalidation_reason,
        Some(ProjectionInvalidationReason::SourceDeleted)
    );

    let summary = read_investigation_summary(db.pool(), operation_id)
        .await
        .expect("read materialized summary");
    assert_eq!(summary.active_generation_id, Some(generation_id));
    assert_eq!(summary.active_generation_seal_hash, Some(digest('d')));
    assert_eq!(summary.current_hypothesis_count, 2);
    assert_eq!(summary.contested_hypothesis_count, 1);
    assert_eq!(
        summary.residual_count, 1,
        "invalidated residual is excluded"
    );

    let persisted_snapshot_id: Uuid = sqlx::query_scalar(
        r#"SELECT (projection_body #>>
                   '{record,canonicalRedactedBody,candidate_snapshot_id}')::UUID
             FROM investigation_projection_entity_versions
            WHERE operation_id=$1 AND entity_kind='generation'"#,
    )
    .bind(operation_id)
    .fetch_one(db.pool())
    .await
    .expect("read generation dependency path");
    assert_eq!(persisted_snapshot_id, candidate_snapshot_id);
}

#[tokio::test]
#[serial]
async fn projection_read_v2_temporal_authority_and_six_field_keyset_are_stable_or_stale() {
    let (db, _data_dir) = fixture("stable-keyset").await;
    let operation_id = Uuid::new_v4();
    let organizations = [Uuid::new_v4(), Uuid::new_v4()];
    seed_operation(db.pool(), operation_id, &organizations).await;
    enqueue_and_project(
        db.pool(),
        operation_id,
        vec![
            member(
                hypothesis_source(
                    Uuid::new_v4(),
                    Uuid::new_v4(),
                    organizations[0],
                    0,
                    None,
                    "supported",
                    '3',
                ),
                ProjectionChangeKind::Insert,
                None,
            ),
            member(
                hypothesis_source(
                    Uuid::new_v4(),
                    Uuid::new_v4(),
                    organizations[1],
                    0,
                    None,
                    "proposed",
                    '4',
                ),
                ProjectionChangeKind::Insert,
                None,
            ),
        ],
    )
    .await;

    let first = list_investigation_hypotheses(db.pool(), operation_id, first_page_query(1))
        .await
        .expect("first keyset page");
    assert_eq!(first.hypotheses.len(), 1);
    let next_key = first.next_key.clone().expect("next key");
    assert_eq!(
        next_key, first.hypotheses[0].sort_key,
        "cursor freezes all six sort fields"
    );
    let expected = InvestigationPageValidationInput {
        as_of_change_seq: first.authority.temporal.as_of_change_seq,
        as_of_temporal_cutoff: first.authority.temporal.as_of_temporal_cutoff,
        authority_epoch_set_hash: first.authority.temporal.authority_epoch_set_hash.clone(),
        earliest_effective_valid_until: first.authority.temporal.earliest_effective_valid_until,
    };
    let second = list_investigation_hypotheses(
        db.pool(),
        operation_id,
        InvestigationHypothesisListQuery {
            filters: InvestigationHypothesisFilters::default(),
            after: Some(next_key),
            expected_page_authority: Some(expected.clone()),
            page_size: 1,
        },
    )
    .await
    .expect("same-snapshot continuation");
    assert_eq!(second.hypotheses.len(), 1);
    assert_eq!(second.authority.temporal, first.authority.temporal);

    let captured = capture_investigation_read_authority(db.pool(), operation_id)
        .await
        .expect("capture cursor-verification authority");
    assert_eq!(
        captured.temporal.as_of_change_seq,
        first.authority.temporal.as_of_change_seq
    );
    assert_eq!(captured.operation.cursor_salt.len(), 32);

    enqueue_and_project(
        db.pool(),
        operation_id,
        vec![member(
            hypothesis_source(
                Uuid::new_v4(),
                Uuid::new_v4(),
                organizations[0],
                0,
                None,
                "supported",
                '5',
            ),
            ProjectionChangeKind::Insert,
            None,
        )],
    )
    .await;
    let error = list_investigation_hypotheses(
        db.pool(),
        operation_id,
        InvestigationHypothesisListQuery {
            filters: InvestigationHypothesisFilters::default(),
            after: Some(first.hypotheses[0].sort_key.clone()),
            expected_page_authority: Some(expected),
            page_size: 1,
        },
    )
    .await
    .expect_err("head drift requires restart");
    assert!(matches!(error, InvestigationProjectionError::Stale { .. }));
    assert_eq!(error.code(), "INVESTIGATION_PROJECTION_STALE");
    assert!(error.restart_required());
}

#[tokio::test]
#[serial]
async fn projection_read_unknown_hypothesis_payload_fails_closed() {
    let (db, _data_dir) = fixture("unknown-payload").await;
    let operation_id = Uuid::new_v4();
    seed_operation(db.pool(), operation_id, &[]).await;
    let root_id = Uuid::new_v4();
    let body = CanonicalJsonObject::try_from_value(serde_json::json!({"label": "not-authority"}))
        .expect("canonical malformed body");
    enqueue_and_project(
        db.pool(),
        operation_id,
        vec![member(
            ProjectionSourceSnapshotV1::Hypothesis(
                HypothesisProjectionRecordV1::try_new(root_id.to_string(), 1, 1, body)
                    .expect("bounded malformed body"),
            ),
            ProjectionChangeKind::Insert,
            None,
        )],
    )
    .await;
    let error = list_investigation_hypotheses(db.pool(), operation_id, first_page_query(100))
        .await
        .expect_err("unknown Hypothesis payload must fail closed");
    assert_eq!(error.code(), "INVESTIGATION_PROJECTION_PAYLOAD_INVALID");
}

#[tokio::test]
#[serial]
async fn projection_read_auxiliary_queries_ignore_many_unrelated_bad_rows() {
    let (db, _data_dir) = fixture("bounded-auxiliary").await;
    let operation_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    seed_operation(db.pool(), operation_id, &[organization_id]).await;
    let root_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let unrelated_root_id = Uuid::new_v4();
    let unrelated_revision_id = Uuid::new_v4();
    let mut members = vec![member(
        hypothesis_source(
            root_id,
            revision_id,
            organization_id,
            0,
            None,
            "supported",
            '6',
        ),
        ProjectionChangeKind::Insert,
        None,
    )];
    for _ in 0..64 {
        members.push(member(
            malformed_unrelated_residual_source(
                Uuid::new_v4(),
                unrelated_root_id,
                unrelated_revision_id,
            ),
            ProjectionChangeKind::Insert,
            None,
        ));
        members.push(member(
            malformed_unrelated_plan_source(Uuid::new_v4(), unrelated_revision_id),
            ProjectionChangeKind::Close,
            None,
        ));
    }
    enqueue_and_project(db.pool(), operation_id, members).await;

    let page = list_investigation_hypotheses(db.pool(), operation_id, first_page_query(1))
        .await
        .expect("unrelated malformed auxiliary rows are outside the bounded page query");
    assert_eq!(page.hypotheses.len(), 1);
    assert_eq!(page.hypotheses[0].root_id, root_id);
    assert!(page.hypotheses[0].residual_codes.is_empty());

    let detail = get_investigation_hypothesis(db.pool(), operation_id, revision_id)
        .await
        .expect("unrelated malformed plans are outside the direct revision selector")
        .expect("authorized Hypothesis detail exists");
    assert!(detail.verification_objective_summaries.is_empty());
}

#[tokio::test]
#[serial]
async fn projection_read_rejects_a_corrupt_operation_joint_contract_pair() {
    let (db, _data_dir) = fixture("joint-contract").await;
    let operation_id = Uuid::new_v4();
    seed_operation(db.pool(), operation_id, &[]).await;
    // This is an intentionally corrupt, disposable test database. Production
    // constraints prevent the row; the read boundary must still defend itself
    // against preexisting/corrupted storage rather than trusting DDL alone.
    sqlx::query(
        "ALTER TABLE operation_state DROP CONSTRAINT operation_state_joint_contract_pair_check",
    )
    .execute(db.pool())
    .await
    .expect("remove pair check in isolated corruption fixture");
    sqlx::query("ALTER TABLE operation_state DISABLE TRIGGER operation_state_investigation_contract_immutable")
        .execute(db.pool())
        .await
        .expect("disable immutable trigger in isolated corruption fixture");
    sqlx::query(
        r#"UPDATE operation_state
              SET investigation_contract_version='hypothesis_registry_v1',
                  investigation_rollout_mode='shadow_registry'
            WHERE operation_id=$1"#,
    )
    .bind(operation_id)
    .execute(db.pool())
    .await
    .expect("inject individually valid but jointly invalid pair");

    let error = capture_investigation_read_authority(db.pool(), operation_id)
        .await
        .expect_err("read authority must reject invalid joint pair");
    assert_eq!(error.code(), "INVESTIGATION_PROJECTION_PAYLOAD_INVALID");
}
