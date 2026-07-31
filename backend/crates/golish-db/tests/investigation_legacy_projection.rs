use golish_core::hypothesis_semantic_key::CanonicalJsonObject;
use golish_core::investigation_comparison::{
    CheckedAuthorityComparisonV1, ComparisonAuthorityBasisInputV1,
    ComparisonHypothesisDispositionV1, ComparisonHypothesisReadinessV1, GenerationComparisonV1,
    InvestigationComparisonRecordInputV1, InvestigationComparisonRecordV1,
    KnowledgeFeedComparisonV1, PlanBCheckedComparisonAuthorityInputV1,
    PlanCComparisonAuthorityInputV1,
};
use golish_core::investigation_projection::{
    HypothesisProjectionRecordV1, LegacyAttemptProjectionRecordV1,
    LegacyCandidateProjectionRecordV1, ProjectionChangeKind, ProjectionSourceSnapshotV1,
    ProjectionSourceTimeStatusV1,
};
use golish_db::repo::attack_candidates::{
    create, update_disposition, upsert_by_hash, upsert_legacy_by_hash, AttackCandidateWrite,
    ATTACK_LEGACY_MUTATION_FORBIDDEN_BY_INVESTIGATION_CONTRACT,
};
use golish_db::repo::hypothesis_legacy_projection::freeze_comparison_projection_source_body_v1;
use golish_db::repo::investigation_projection::{
    claim_next_projection_batch, compare_and_record_v1, enqueue_projection_batch_on,
    project_next_projection_batch, project_projection_batch, read_legacy_candidate_projection,
    CompareAndRecordV1Input, LegacyCompatibilityReadDisposition, ProjectionOutboxBatchInput,
    ProjectionOutboxMemberInput, ProjectionProjectOutcome, ProjectionSourceStorageV1,
};
use golish_db::{DbConfig, GolishDb};
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
        database: format!("legacy_projection_{label}_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start isolated migrated postgres");
    (db, data_dir)
}

async fn insert_operation(
    pool: &PgPool,
    operation_id: Uuid,
    project_scope_id: Option<Uuid>,
    tool_truth: &str,
    mode: &str,
) {
    let mode_rank = match mode {
        "legacy_only" => 0,
        "shadow_registry" => 1,
        "dual_read_compare" => 2,
        "registry_authoritative_legacy_projection" => 3,
        "new_only" => 4,
        other => panic!("unsupported fixture rollout mode: {other}"),
    };
    for statement in [
        "ALTER TABLE tool_truth_rollout DISABLE TRIGGER tool_truth_rollout_direct_mutation_guard",
        "ALTER TABLE investigation_rollout DISABLE TRIGGER investigation_rollout_direct_mutation_guard",
    ] {
        sqlx::query(statement)
            .execute(pool)
            .await
            .expect("disable immutable rollout guard in isolated fixture");
    }
    sqlx::query(
        "UPDATE tool_truth_rollout SET new_operation_contract=$1,row_version=row_version+1 WHERE singleton=TRUE",
    )
    .bind(tool_truth)
    .execute(pool)
    .await
    .expect("install isolated Tool Truth default");
    sqlx::query(
        r#"UPDATE investigation_rollout
              SET contract_version='hypothesis_registry_v1',rollout_mode=$1,
                  mode_rank=$2,row_version=row_version+1
            WHERE singleton=TRUE"#,
    )
    .bind(mode)
    .bind(mode_rank)
    .execute(pool)
    .await
    .expect("install isolated Investigation default");
    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,
               attack_execution_contract,tool_truth_contract,project_scope_id,
               investigation_contract_version,investigation_rollout_mode
           ) VALUES($1,'assessment','target_intel','legacy_v1','legacy',$2,$3,
                    'hypothesis_registry_v1',$4)"#,
    )
    .bind(operation_id)
    .bind(tool_truth)
    .bind(project_scope_id)
    .bind(mode)
    .execute(pool)
    .await
    .expect("insert frozen Investigation operation");
}

fn legacy_write(operation_id: Uuid) -> AttackCandidateWrite {
    AttackCandidateWrite {
        candidate_id: Uuid::new_v4(),
        operation_id: operation_id.to_string(),
        organization_id: None,
        target: "example.test".to_owned(),
        hypothesis: "A bounded legacy hypothesis".to_owned(),
        technique: Some("GOLISH-TEST".to_owned()),
        rationale: "test rationale".to_owned(),
        prior_refs: Vec::new(),
        suggested_approach: "read only".to_owned(),
        priority: "low".to_owned(),
        wave: 0,
        parent_finding_id: None,
        disposition: "proposed".to_owned(),
    }
}

fn hash(index: u64) -> String {
    format!("sha256:{index:064x}")
}

#[derive(sqlx::FromRow)]
struct ComparisonSampleFixture {
    record_key: String,
    comparison_state: String,
    legacy_hash: Option<String>,
    registry_hash: Option<String>,
    diff_summary: serde_json::Value,
}

fn comparison_record_input(semantic_index: u64) -> InvestigationComparisonRecordInputV1 {
    let authority = (10..23).map(hash).collect::<Vec<_>>();
    let feed = (30..41).map(hash).collect::<Vec<_>>();
    InvestigationComparisonRecordInputV1 {
        semantic_key_hash: hash(semantic_index),
        revision_ingredients_hash: hash(2),
        authority_basis: ComparisonAuthorityBasisInputV1::PlanBChecked {
            authority: Box::new(PlanBCheckedComparisonAuthorityInputV1 {
                checked_authority: CheckedAuthorityComparisonV1 {
                    bundle_seal_hash: authority[0].clone(),
                    root_set_hash: authority[1].clone(),
                    bundle_member_set_hash: authority[2].clone(),
                    receipt_set_hash: authority[3].clone(),
                    denominator_graph_bundle_hash: authority[4].clone(),
                    semantic_authority_bundle_hash: authority[5].clone(),
                    freshness_attestation_bundle_hash: authority[6].clone(),
                    temporal_validity_bundle_hash: authority[7].clone(),
                    temporal_validity_policy_set_hash: authority[8].clone(),
                    temporal_validity_decision_set_hash: authority[9].clone(),
                    target_state_epoch_set_hash: authority[10].clone(),
                    observation_window_hash: authority[11].clone(),
                    gate_temporal_reevaluation_hash: authority[12].clone(),
                },
                knowledge_feed: KnowledgeFeedComparisonV1 {
                    catalog_policy_seal_hash: feed[0].clone(),
                    required_member_set_hash: feed[1].clone(),
                    signature_algorithm_set_hash: feed[2].clone(),
                    trust_store_hash: feed[3].clone(),
                    key_revocation_epoch_hash: feed[4].clone(),
                    snapshot_set_hash: feed[5].clone(),
                    product_version_census_hash: feed[6].clone(),
                    match_census_hash: feed[7].clone(),
                    source_set_hash: feed[8].clone(),
                    gate_reevaluation_hash: feed[9].clone(),
                    obligation_set_hash: feed[10].clone(),
                },
                claim_component_member_hashes: vec![hash(60)],
                verification_contract_member_hashes: vec![hash(61)],
                verification_plan_member_hashes: vec![hash(62)],
                verification_plan_objective_member_hashes: vec![hash(63)],
                verification_plan_path_member_hashes: vec![hash(64)],
                coverage_subreview_member_hashes: vec![hash(65)],
                coverage_synthesis_member_hashes: vec![hash(66)],
                coverage_final_review_member_hashes: vec![hash(67)],
                coverage_checklist_member_hashes: vec![hash(68)],
                sampling_degraded_residual_member_hashes: vec![hash(69)],
            }),
        },
        generation: GenerationComparisonV1 {
            generation_ordinal: 1,
            generation_seal_hash: hash(50),
            generation_member_set_hash: hash(51),
            generation_event_set_hash: hash(52),
            open_obligation_set_hash: hash(53),
        },
        disposition: ComparisonHypothesisDispositionV1::Supported,
        readiness: ComparisonHypothesisReadinessV1::ReportingOnlyPlanCUnavailable,
        plan_c: PlanCComparisonAuthorityInputV1::not_available_plan_c(),
        finding_lineage_member_hashes: Vec::new(),
        refutation_lineage_member_hashes: Vec::new(),
        residual_member_hashes: vec![hash(69)],
        coverage_member_hashes: vec![hash(70)],
    }
}

fn comparison_record(semantic_index: u64) -> InvestigationComparisonRecordV1 {
    InvestigationComparisonRecordV1::compile(comparison_record_input(semantic_index))
        .expect("compile complete comparison record")
}

#[tokio::test]
#[serial]
async fn legacy_public_mutations_lock_and_reject_new_authority_operations() {
    let (mut db, _data_dir) = fixture("mutation_guard").await;
    let operation_id = Uuid::new_v4();
    insert_operation(db.pool(), operation_id, None, "receipt_v1", "new_only").await;
    let write = legacy_write(operation_id);

    for result in [
        upsert_legacy_by_hash(db.pool(), &write).await,
        upsert_by_hash(db.pool(), &write).await,
        create(db.pool(), &write).await,
    ] {
        let error = result.expect_err("new authority must reject legacy Candidate writes");
        assert!(error
            .to_string()
            .contains(ATTACK_LEGACY_MUTATION_FORBIDDEN_BY_INVESTIGATION_CONTRACT));
    }
    let update_error = update_disposition(
        db.pool(),
        write.candidate_id,
        &write.operation_id,
        None,
        "rejected",
    )
    .await
    .expect_err("new authority must reject legacy Candidate updates");
    assert!(update_error
        .to_string()
        .contains(ATTACK_LEGACY_MUTATION_FORBIDDEN_BY_INVESTIGATION_CONTRACT));

    let rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM attack_candidates WHERE operation_id=$1 AND operation_uuid IS NULL",
    )
    .bind(operation_id.to_string())
    .fetch_one(db.pool())
    .await
    .expect("count forbidden legacy rows");
    assert_eq!(rows, 0);

    let raw_error = sqlx::query(
        r#"INSERT INTO attack_candidates(
               candidate_id,operation_id,target,hypothesis,hypothesis_hash
           ) VALUES($1,$2,'raw.example.test','raw bypass attempt',$3)"#,
    )
    .bind(Uuid::new_v4())
    .bind(operation_id.to_string())
    .bind(hash(400))
    .execute(db.pool())
    .await
    .expect_err("database trigger must reject raw legacy Candidate mutation");
    assert!(raw_error
        .to_string()
        .contains(ATTACK_LEGACY_MUTATION_FORBIDDEN_BY_INVESTIGATION_CONTRACT));
    let guarded_tables: Vec<String> = sqlx::query_scalar(
        r#"SELECT event_object_table
             FROM information_schema.triggers
            WHERE trigger_name LIKE '%investigation_legacy_mutation_guard'
            ORDER BY event_object_table"#,
    )
    .fetch_all(db.pool())
    .await
    .expect("load database legacy mutation guards");
    assert_eq!(
        guarded_tables,
        vec![
            "attack_candidate_approvals".to_owned(),
            "attack_candidate_approvals".to_owned(),
            "attack_candidate_approvals".to_owned(),
            "attack_candidates".to_owned(),
            "attack_candidates".to_owned(),
            "attack_candidates".to_owned(),
            "candidate_attempts".to_owned(),
            "candidate_attempts".to_owned(),
            "candidate_attempts".to_owned(),
        ]
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn new_only_missing_compatibility_is_a_hold_not_a_fallback() {
    let (mut db, _data_dir) = fixture("missing_hold").await;
    let operation_id = Uuid::new_v4();
    insert_operation(db.pool(), operation_id, None, "receipt_v1", "new_only").await;

    let read = read_legacy_candidate_projection(db.pool(), operation_id, Uuid::new_v4())
        .await
        .expect("read missing compatibility projection");
    assert_eq!(read.disposition, LegacyCompatibilityReadDisposition::Hold);
    assert!(read.projection.is_none());
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn compatibility_reads_hold_unsupported_diverged_and_stale_versions() {
    let (mut db, _data_dir) = fixture("compatibility_hold_states").await;
    let operation_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO project_scopes(project_scope_id,canonical_project_path,path_sha256) VALUES($1,$2,$3)",
    )
    .bind(project_scope_id)
    .bind(format!("/tmp/compatibility-hold-{project_scope_id}"))
    .bind(hash(490))
    .execute(db.pool())
    .await
    .expect("insert compatibility HOLD project scope");
    insert_operation(
        db.pool(),
        operation_id,
        Some(project_scope_id),
        "receipt_v1",
        "registry_authoritative_legacy_projection",
    )
    .await;
    let cases = [
        (
            Uuid::new_v4(),
            "unsupported",
            "legacy_projection_unsupported",
        ),
        (Uuid::new_v4(), "invalidated", "legacy_projection_diverged"),
        (Uuid::new_v4(), "invalidated", "authority_stale"),
    ];
    let batch_id = Uuid::new_v4();
    let members = cases
        .iter()
        .enumerate()
        .map(|(index, (entity_id, status, _))| {
            let body = CanonicalJsonObject::try_from_value(serde_json::json!({
                "fixture_status": status,
            }))
            .expect("canonical compatibility fixture body");
            ProjectionOutboxMemberInput {
                outbox_member_id: Uuid::new_v4(),
                change_kind: ProjectionChangeKind::Insert,
                source: ProjectionSourceSnapshotV1::LegacyCandidateProjection(
                    LegacyCandidateProjectionRecordV1::try_new(entity_id.to_string(), 1, 1, body)
                        .expect("bounded compatibility fixture source"),
                ),
                source_occurred_at: None,
                source_time_status: ProjectionSourceTimeStatusV1::HistoricalUnknown,
                invalidation_reason: None,
                storage: if index % 2 == 0 {
                    ProjectionSourceStorageV1::Inline
                } else {
                    ProjectionSourceStorageV1::Blob {
                        redaction_contract_version: "compatibility-read-fixture.v1".to_owned(),
                    }
                },
            }
        })
        .collect();
    let mut source_tx = db
        .pool()
        .begin()
        .await
        .expect("begin compatibility source batch");
    enqueue_projection_batch_on(
        &mut source_tx,
        ProjectionOutboxBatchInput {
            batch_id,
            operation_id,
            project_scope_id: Some(project_scope_id),
            stable_request_id: Uuid::new_v4(),
            source_transaction_id: Uuid::new_v4(),
            source_occurred_at: None,
            source_time_status: ProjectionSourceTimeStatusV1::HistoricalUnknown,
            members,
        },
    )
    .await
    .expect("append compatibility fixture batch");
    source_tx
        .commit()
        .await
        .expect("commit compatibility fixture batch");
    assert!(matches!(
        project_projection_batch(db.pool(), operation_id, batch_id)
            .await
            .expect("project compatibility fixture batch"),
        ProjectionProjectOutcome::Applied(_)
    ));

    // These rows model historical projector output. The canonical authority
    // identities are deliberately opaque to this read-contract fixture, so
    // only FK triggers are suppressed inside the isolated transaction; all
    // table checks, joins, append-only behavior and production reader SQL run.
    let mut fixture_tx = db
        .pool()
        .begin()
        .await
        .expect("begin compatibility row fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *fixture_tx)
        .await
        .expect("suppress opaque authority FKs in isolated read fixture");
    for (index, (entity_id, status, reason)) in cases.iter().enumerate() {
        sqlx::query(
            r#"INSERT INTO hypothesis_legacy_candidate_projection_versions(
                   legacy_candidate_projection_id,operation_id,entity_id,entity_version,
                   source_generation_id,source_revision_id,source_contract_hash,
                   projection_status,projection_body,projection_hash,batch_id,change_seq,
                   invalidation_reason,projected_at
               )
               SELECT $1,$2,$3,entity.entity_version,$4,$5,$6,$7,NULL,$8,
                      entity.batch_id,entity.change_seq,$9,entity.projected_at
                 FROM investigation_projection_entity_versions entity
                WHERE entity.operation_id=$2
                  AND entity.entity_kind='legacy_candidate_projection'
                  AND entity.entity_id=$10"#,
        )
        .bind(Uuid::new_v4())
        .bind(operation_id)
        .bind(entity_id)
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(hash(500 + index as u64))
        .bind(status)
        .bind(hash(510 + index as u64))
        .bind(reason)
        .bind(entity_id.to_string())
        .execute(&mut *fixture_tx)
        .await
        .expect("insert historical compatibility status fixture");
    }
    fixture_tx
        .commit()
        .await
        .expect("commit historical compatibility status fixture");

    for (entity_id, expected_status, expected_reason) in cases {
        let read = read_legacy_candidate_projection(db.pool(), operation_id, entity_id)
            .await
            .expect("read fail-closed compatibility status");
        assert_eq!(read.disposition, LegacyCompatibilityReadDisposition::Hold);
        let projection = read
            .projection
            .expect("historical status remains inspectable");
        assert_eq!(projection.projection_status, expected_status);
        assert_eq!(
            projection.invalidation_reason.as_deref(),
            Some(expected_reason)
        );
    }
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn dual_read_complete_records_compare_whole_hash_without_field_fallback() {
    let (mut db, _data_dir) = fixture("complete_dual_compare").await;
    let operation_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO project_scopes(project_scope_id,canonical_project_path,path_sha256) VALUES($1,$2,$3)",
    )
    .bind(project_scope_id)
    .bind(format!("/tmp/legacy-projection-{project_scope_id}"))
    .bind(hash(99))
    .execute(db.pool())
    .await
    .expect("insert comparison project scope");
    insert_operation(
        db.pool(),
        operation_id,
        Some(project_scope_id),
        "shadow_v1",
        "dual_read_compare",
    )
    .await;

    let sample = compare_and_record_v1(
        db.pool(),
        CompareAndRecordV1Input {
            operation_id,
            organization_id: None,
            as_of_change_seq: 0,
            record_kind: "hypothesis".to_owned(),
            record_key: "complete:v1".to_owned(),
            legacy: Some(comparison_record(1)),
            registry: Some(comparison_record(101)),
        },
    )
    .await
    .expect("record complete dual comparison");
    assert_eq!(sample.comparison_state, "mismatch");
    assert!(sample.legacy_hash.is_some());
    assert!(sample.registry_hash.is_some());
    assert_eq!(sample.diff_summary["field_fallback"], false);
    assert_eq!(sample.diff_summary["legacy_complete"], true);
    assert_eq!(sample.diff_summary["registry_complete"], true);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn projector_assembles_frozen_producer_match_mismatch_and_incomplete_records() {
    let (mut db, _data_dir) = fixture("projector_comparison_assembler").await;
    let operation_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO project_scopes(project_scope_id,canonical_project_path,path_sha256) VALUES($1,$2,$3)",
    )
    .bind(project_scope_id)
    .bind(format!("/tmp/projector-comparison-{project_scope_id}"))
    .bind(hash(600))
    .execute(db.pool())
    .await
    .expect("insert projector comparison scope");
    insert_operation(
        db.pool(),
        operation_id,
        Some(project_scope_id),
        "shadow_v1",
        "dual_read_compare",
    )
    .await;

    let match_id = Uuid::new_v5(&operation_id, b"comparison-match");
    let mismatch_id = Uuid::new_v5(&operation_id, b"comparison-mismatch");
    let incomplete_id = Uuid::new_v5(&operation_id, b"comparison-incomplete");
    let missing_field_id = Uuid::new_v5(&operation_id, b"comparison-missing-field");
    let complete = comparison_record_input(1);
    let mut missing_field_body = freeze_comparison_projection_source_body_v1(
        serde_json::json!({"producer_case":"missing_field"}),
        Some(comparison_record_input(5)),
        Some(comparison_record_input(5)),
    )
    .expect("freeze complete record before historical field-loss fixture")
    .as_value()
    .clone();
    missing_field_body["comparison_record_v1"]["registry"]
        .as_object_mut()
        .expect("registry comparison input object")
        .remove("semantic_key_hash");
    let missing_field_body = CanonicalJsonObject::try_from_value(missing_field_body)
        .expect("canonical historical missing-field source");
    let source_specs = vec![
        (
            match_id,
            freeze_comparison_projection_source_body_v1(
                serde_json::json!({"producer_case":"match"}),
                Some(complete.clone()),
                Some(complete),
            )
            .expect("freeze complete matching producer records"),
        ),
        (
            mismatch_id,
            freeze_comparison_projection_source_body_v1(
                serde_json::json!({"producer_case":"mismatch"}),
                Some(comparison_record_input(2)),
                Some(comparison_record_input(102)),
            )
            .expect("freeze complete mismatching producer records"),
        ),
        (
            incomplete_id,
            freeze_comparison_projection_source_body_v1(
                serde_json::json!({"producer_case":"incomplete"}),
                Some(comparison_record_input(3)),
                None,
            )
            .expect("freeze independently absent registry record"),
        ),
        (missing_field_id, missing_field_body),
    ];
    let members = source_specs
        .into_iter()
        .map(|(entity_id, body)| ProjectionOutboxMemberInput {
            outbox_member_id: Uuid::new_v5(&entity_id, b"comparison-member"),
            change_kind: ProjectionChangeKind::Insert,
            source: ProjectionSourceSnapshotV1::Hypothesis(
                HypothesisProjectionRecordV1::try_new(entity_id.to_string(), 1, 1, body)
                    .expect("build typed comparison source"),
            ),
            source_occurred_at: None,
            source_time_status: ProjectionSourceTimeStatusV1::HistoricalUnknown,
            invalidation_reason: None,
            storage: ProjectionSourceStorageV1::Inline,
        })
        .collect();
    let batch_id = Uuid::new_v5(&operation_id, b"comparison-batch");
    let mut tx = db.pool().begin().await.expect("begin comparison source");
    enqueue_projection_batch_on(
        &mut tx,
        ProjectionOutboxBatchInput {
            batch_id,
            operation_id,
            project_scope_id: Some(project_scope_id),
            stable_request_id: Uuid::new_v5(&batch_id, b"stable-request"),
            source_transaction_id: Uuid::new_v5(&batch_id, b"source-transaction"),
            source_occurred_at: None,
            source_time_status: ProjectionSourceTimeStatusV1::HistoricalUnknown,
            members,
        },
    )
    .await
    .expect("enqueue frozen comparison producer batch");
    tx.commit().await.expect("commit comparison source");
    assert!(matches!(
        project_projection_batch(db.pool(), operation_id, batch_id)
            .await
            .expect("project complete comparison batch"),
        ProjectionProjectOutcome::Applied(_)
    ));

    let samples: Vec<ComparisonSampleFixture> = sqlx::query_as(
        r#"SELECT record_key,comparison_state,legacy_hash,registry_hash,diff_summary
                 FROM investigation_projection_compare_samples
                WHERE operation_id=$1 ORDER BY record_key"#,
    )
    .bind(operation_id)
    .fetch_all(db.pool())
    .await
    .expect("load projector comparison samples");
    assert_eq!(samples.len(), 4);
    let by_key = samples
        .into_iter()
        .map(|sample| (sample.record_key.clone(), sample))
        .collect::<std::collections::BTreeMap<_, _>>();
    let matched = &by_key[&format!("{match_id}:v1")];
    assert_eq!(matched.comparison_state, "match");
    assert_eq!(matched.legacy_hash, matched.registry_hash);
    assert_eq!(matched.diff_summary["field_fallback"], false);
    let mismatched = &by_key[&format!("{mismatch_id}:v1")];
    assert_eq!(mismatched.comparison_state, "mismatch");
    assert!(mismatched.legacy_hash.is_some());
    assert!(mismatched.registry_hash.is_some());
    assert_ne!(mismatched.legacy_hash, mismatched.registry_hash);
    let incomplete = &by_key[&format!("{incomplete_id}:v1")];
    assert_eq!(incomplete.comparison_state, "incomplete");
    assert!(incomplete.legacy_hash.is_some());
    assert!(incomplete.registry_hash.is_none());
    assert_eq!(incomplete.diff_summary["field_fallback"], false);
    let missing_field = &by_key[&format!("{missing_field_id}:v1")];
    assert_eq!(missing_field.comparison_state, "incomplete");
    assert!(missing_field.legacy_hash.is_some());
    assert!(missing_field.registry_hash.is_none());
    assert_eq!(missing_field.diff_summary["field_fallback"], false);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn audit_only_compare_and_missing_sample_are_recovered_by_worker_claim_path() {
    let (mut db, _data_dir) = fixture("audit_compare_recovery").await;
    let operation_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO project_scopes(project_scope_id,canonical_project_path,path_sha256) VALUES($1,$2,$3)",
    )
    .bind(project_scope_id)
    .bind(format!("/tmp/audit-comparison-{project_scope_id}"))
    .bind(hash(700))
    .execute(db.pool())
    .await
    .expect("insert audit comparison scope");
    insert_operation(
        db.pool(),
        operation_id,
        Some(project_scope_id),
        "receipt_v1",
        "registry_authoritative_legacy_projection",
    )
    .await;
    let entity_id = Uuid::new_v5(&operation_id, b"audit-comparison-attempt");
    let body = freeze_comparison_projection_source_body_v1(
        serde_json::json!({"producer_case":"audit_only"}),
        Some(comparison_record_input(4)),
        Some(comparison_record_input(4)),
    )
    .expect("freeze audit-only producer records");
    let batch_id = Uuid::new_v5(&operation_id, b"audit-comparison-batch");
    let mut tx = db.pool().begin().await.expect("begin audit source");
    enqueue_projection_batch_on(
        &mut tx,
        ProjectionOutboxBatchInput {
            batch_id,
            operation_id,
            project_scope_id: Some(project_scope_id),
            stable_request_id: Uuid::new_v5(&batch_id, b"stable-request"),
            source_transaction_id: Uuid::new_v5(&batch_id, b"source-transaction"),
            source_occurred_at: None,
            source_time_status: ProjectionSourceTimeStatusV1::HistoricalUnknown,
            members: vec![ProjectionOutboxMemberInput {
                outbox_member_id: Uuid::new_v5(&entity_id, b"audit-comparison-member"),
                change_kind: ProjectionChangeKind::Insert,
                source: ProjectionSourceSnapshotV1::LegacyAttemptProjection(
                    LegacyAttemptProjectionRecordV1::try_new(entity_id.to_string(), 1, 1, body)
                        .expect("build typed audit comparison source"),
                ),
                source_occurred_at: None,
                source_time_status: ProjectionSourceTimeStatusV1::HistoricalUnknown,
                invalidation_reason: None,
                storage: ProjectionSourceStorageV1::Inline,
            }],
        },
    )
    .await
    .expect("enqueue audit comparison source");
    tx.commit().await.expect("commit audit comparison source");
    sqlx::query(
        r#"CREATE FUNCTION investigation_test_reject_compare_insert() RETURNS trigger
            LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'comparison_write_unavailable'; END $$"#,
    )
    .execute(db.pool())
    .await
    .expect("install comparison failure function");
    sqlx::query(
        r#"CREATE TRIGGER investigation_test_reject_compare_insert
            BEFORE INSERT ON investigation_projection_compare_samples
            FOR EACH ROW EXECUTE FUNCTION investigation_test_reject_compare_insert()"#,
    )
    .execute(db.pool())
    .await
    .expect("install comparison failure trigger");
    project_projection_batch(db.pool(), operation_id, batch_id)
        .await
        .expect_err("comparison failure must surface after the projection commit");
    let persisted_without_sample: (bool, i64) = sqlx::query_as(
        r#"SELECT EXISTS(
               SELECT 1 FROM investigation_projection_batch_receipts WHERE batch_id=$1
           ),(
               SELECT COUNT(*) FROM investigation_projection_compare_samples
                WHERE operation_id=$2
           )"#,
    )
    .bind(batch_id)
    .bind(operation_id)
    .fetch_one(db.pool())
    .await
    .expect("inspect post-head comparison failure");
    assert_eq!(persisted_without_sample, (true, 0));
    sqlx::query(
        "DROP TRIGGER investigation_test_reject_compare_insert ON investigation_projection_compare_samples",
    )
    .execute(db.pool())
    .await
    .expect("remove comparison failure trigger");
    sqlx::query("DROP FUNCTION investigation_test_reject_compare_insert()")
        .execute(db.pool())
        .await
        .expect("remove comparison failure function");

    let claim = claim_next_projection_batch(db.pool(), operation_id)
        .await
        .expect("claim missing comparison recovery")
        .expect("receipt with missing comparison remains worker backlog");
    assert_eq!(claim.batch_id, batch_id);
    assert!(matches!(
        project_next_projection_batch(db.pool(), operation_id)
            .await
            .expect("recover missing comparison")
            .expect("recovery projects receipt replay"),
        ProjectionProjectOutcome::Replay(_)
    ));
    let recovered: (String, i64) = sqlx::query_as(
        "SELECT comparison_state,COUNT(*) OVER() FROM investigation_projection_compare_samples WHERE operation_id=$1",
    )
    .bind(operation_id)
    .fetch_one(db.pool())
    .await
    .expect("load recovered audit comparison");
    assert_eq!(recovered, ("match".to_owned(), 1));
    assert!(claim_next_projection_batch(db.pool(), operation_id)
        .await
        .expect("check recovered worker backlog")
        .is_none());
    db.stop().await;
}
