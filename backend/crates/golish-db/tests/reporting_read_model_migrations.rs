use std::sync::Arc;
use std::time::Duration;

use golish_db::models::NewSession;
use golish_db::repo::{
    capability_execution_receipts, legacy_report_authority, legacy_security_verdict,
    project_scopes, report_input_authority, report_input_seals, report_revisions,
    report_source_manifest, runtime_memory_tx, sessions,
};
use golish_db::{DbConfig, GolishDb};
use golish_memory_domain::source_ref::CanonicalRowId;
use golish_reporting_domain::{
    ReportSourceKind, ReportSourceSnapshot, ReportSourceVersion, ReportValidationResult,
};
use serial_test::serial;
use tokio::sync::Notify;
use uuid::Uuid;

#[path = "support/tool_truth_authority_fixture.rs"]
mod tool_truth_authority_fixture;

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
        database: format!("reporting_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    };
    let db = GolishDb::start(config)
        .await
        .expect("start migrated embedded postgres");
    (db, data_dir)
}

#[derive(Clone, Copy)]
struct FrozenScope {
    operation_id: Uuid,
    project_scope_id: Uuid,
    snapshot_id: Uuid,
    organization_id: Uuid,
}

#[derive(Clone, Copy)]
struct ReportRevisionFixture {
    report_id: Uuid,
    revision_id: Uuid,
    section_id: Uuid,
    row_version: i64,
}

async fn frozen_scope(db: &GolishDb, project_path: &str) -> FrozenScope {
    let session = sessions::create(
        db.pool(),
        NewSession {
            title: Some("reporting fixture".to_string()),
            workspace_path: Some(project_path.to_string()),
            workspace_label: None,
            model: None,
            provider: None,
            project_path: Some(project_path.to_string()),
        },
    )
    .await
    .expect("create reporting session");
    let project_scope =
        project_scopes::register_first_open(db.pool(), project_path, &"1".repeat(64))
            .await
            .expect("register reporting project scope");
    let operation_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    runtime_memory_tx::create_runtime_operation(
        db.pool(),
        &runtime_memory_tx::CreateRuntimeOperationRow {
            operation_id,
            initial_stage_execution_id: stage_execution_id,
            session_id: session.id,
            title: Some("reporting operation".to_string()),
            input: "reporting fixture".to_string(),
            profile: "assessment".to_string(),
            entry_stage: "target_intel".to_string(),
            project_scope_id: project_scope.project_scope_id,
            application_model_contract: golish_core::ApplicationModelContract::LegacyNoModel,
            cli_scope: None,
        },
    )
    .await
    .expect("create reporting operation");
    let organization_id = Uuid::new_v4();
    let decision_id = Uuid::new_v4();
    let snapshot_id = Uuid::new_v4();
    let mut tx = db.pool().begin().await.expect("begin reporting scope");
    sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$2,'Report Org')")
        .bind(organization_id)
        .bind(project_path)
        .execute(&mut *tx)
        .await
        .expect("insert reporting organization");
    sqlx::query(
        r#"INSERT INTO operation_scope_decisions(
               id,operation_id,project_scope_id,stage_execution_id,
               root_organization_id,mode,decision_rows,decision_hash
           ) VALUES($1,$2,$3,$4,$5,'cli_flags',$6,$7)"#,
    )
    .bind(decision_id)
    .bind(operation_id)
    .bind(project_scope.project_scope_id)
    .bind(stage_execution_id)
    .bind(organization_id)
    .bind(serde_json::json!([{"organization_id": organization_id}]))
    .bind("2".repeat(64))
    .execute(&mut *tx)
    .await
    .expect("insert reporting scope decision");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_snapshots(
               id,operation_id,project_scope_id,scope_decision_id,
               project_path_at_freeze,root_organization_id,mode,scope_hash
           ) VALUES($1,$2,$3,$4,$5,$6,'cli_flags',$7)"#,
    )
    .bind(snapshot_id)
    .bind(operation_id)
    .bind(project_scope.project_scope_id)
    .bind(decision_id)
    .bind(project_path)
    .bind(organization_id)
    .bind("3".repeat(64))
    .execute(&mut *tx)
    .await
    .expect("insert reporting scope snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units(
               snapshot_id,organization_id,organization_name_at_freeze,
               role,depth,ordinal,decision_row_id,approval_source
           ) VALUES($1,$2,'Report Org','root',0,0,'root',$3)"#,
    )
    .bind(snapshot_id)
    .bind(organization_id)
    .bind(serde_json::json!({"source": "cli_flags"}))
    .execute(&mut *tx)
    .await
    .expect("insert reporting scope unit");
    sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
        .bind(snapshot_id)
        .execute(&mut *tx)
        .await
        .expect("seal reporting scope");
    tx.commit().await.expect("commit reporting scope");
    FrozenScope {
        operation_id,
        project_scope_id: project_scope.project_scope_id,
        snapshot_id,
        organization_id,
    }
}

fn tagged_digest(nibble: char) -> String {
    format!("sha256:{}", nibble.to_string().repeat(64))
}

async fn seed_report_finalization_authority(
    db: &GolishDb,
    scope: FrozenScope,
    revision_id: Uuid,
) -> ReportSourceSnapshot {
    let project_path: String = sqlx::query_scalar(
        "SELECT project_path_at_freeze FROM operation_org_scope_snapshots WHERE id=$1",
    )
    .bind(scope.snapshot_id)
    .fetch_one(db.pool())
    .await
    .expect("load report finalization fixture project path");
    let _ = tool_truth_authority_fixture::seed_all_fresh_tool_truth_roots(
        db.pool(),
        scope.operation_id,
        scope.organization_id,
        &project_path,
    )
    .await;
    let authority_request = capability_execution_receipts::CheckToolTruthAuthorityBundle {
        stable_consumer_request_id: Uuid::new_v5(
            &revision_id,
            format!("report-tool-truth:{}", scope.organization_id).as_bytes(),
        ),
        operation_id: scope.operation_id,
        organization_id: scope.organization_id,
        consumer_kind:
            capability_execution_receipts::ToolTruthAuthorityBundleConsumerV1::CurrentReport,
    };
    capability_execution_receipts::with_all_fresh_tool_truth_authority_bundle(
        db.pool(),
        &authority_request,
        |_tx, _authority| Box::pin(async { Ok::<(), golish_db::DbError>(()) }),
    )
    .await
    .expect("seal all-fresh CurrentReport Tool Truth authority");

    let evidence_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO audit_log(
               action,category,details,project_path,source,status,detail,run_id,audit_role
           ) VALUES(
               'report-finalization-authority','harness','retained report authority',$1,
               'harness','completed',$2,$3,'evidence'
           ) RETURNING id"#,
    )
    .bind(&project_path)
    .bind(serde_json::json!({"organization_id":scope.organization_id}))
    .bind(scope.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("insert retained report authority evidence");
    let target_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO targets(
               id,name,target_type,value,scope,project_path,organization_id,source
           ) VALUES($1,'Legacy report target','url',
                    'https://legacy-report.example.test/login','in',$2,$3,
                    'legacy_report_authority_fixture')"#,
    )
    .bind(target_id)
    .bind(&project_path)
    .bind(scope.organization_id)
    .execute(db.pool())
    .await
    .expect("insert legacy report authority target");
    let candidate_id = Uuid::new_v4();
    let attempt_id = Uuid::new_v4();
    let hypothesis_root_id = Uuid::new_v4();
    let hypothesis_revision_id = Uuid::new_v4();
    let target_identity_hash = tagged_digest('6');
    let candidate_plan_hash = tagged_digest('7');
    let mut legacy_tx = db
        .pool()
        .begin()
        .await
        .expect("begin legacy report authority");
    sqlx::query("SET LOCAL session_replication_role = 'replica'")
        .execute(&mut *legacy_tx)
        .await
        .expect("isolate legacy authority parent fixture");
    sqlx::query(
        r#"INSERT INTO attack_hypotheses(
               root_id,operation_id,organization_id,root_kind,
               identity_ingredients,identity_ingredients_hash
           ) VALUES($1,$2,$3,'initial',$4,$5)"#,
    )
    .bind(hypothesis_root_id)
    .bind(scope.operation_id)
    .bind(scope.organization_id)
    .bind(serde_json::json!({"fixture":"report-finalization-authority"}))
    .bind(tagged_digest('b'))
    .execute(&mut *legacy_tx)
    .await
    .expect("insert report finalization hypothesis root");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_revisions(
               revision_id,root_id,operation_id,organization_id,revision_ordinal,
               semantic_key,semantic_key_hash,subject_kind,subject_identity_hash,
               target_live_id,target_type_at_time,target_value_at_time,
               predicate_schema,predicate_version,normalized_arguments,trust_boundary,
               polarity,epistemic_state,lifecycle_state,planning_readiness,
               structured_claim,assumptions,missing_facts,priority,risk_impact,
               origin_decision_hash,revision_ingredients_hash,revision_hash
           ) VALUES(
               $1,$2,$3,$4,0,$5,$6,'url',$7,$8,'url',
               'https://legacy-report.example.test/login','legacy_report_fixture',1,$9,
               'external','positive','refuted','closed','deferred',$10,'[]','[]',0,$11,
               $12,$13,$14
           )"#,
    )
    .bind(hypothesis_revision_id)
    .bind(hypothesis_root_id)
    .bind(scope.operation_id)
    .bind(scope.organization_id)
    .bind(serde_json::json!({
        "target":"https://legacy-report.example.test/login",
        "predicate":"legacy_report_fixture",
    }))
    .bind(tagged_digest('c'))
    .bind(&target_identity_hash)
    .bind(target_id)
    .bind(serde_json::json!({}))
    .bind(serde_json::json!({"disposition":"refuted"}))
    .bind(serde_json::json!({"impact":"none"}))
    .bind(tagged_digest('d'))
    .bind(tagged_digest('e'))
    .bind(tagged_digest('f'))
    .execute(&mut *legacy_tx)
    .await
    .expect("insert report finalization hypothesis revision");
    sqlx::query(
        r#"INSERT INTO attack_candidates(
               candidate_id,operation_id,organization_id,target,hypothesis,
               hypothesis_hash,technique,rationale,priority,wave,disposition,
               operation_uuid,scope_snapshot_id,wave_run_id,wave_unit_id,
               source_work_item_id,decision_stage_execution_id,
               decision_stage_run_unit_id,decision_deliverable_submission_id,
               decision_stage_kind,target_live_id,target_id_at_time,live_target_id,
               canonical_target_snapshot,target_type_at_time,
               target_value_at_time,target_identity_hash,execution_plan,
               candidate_plan_hash,risk_class,hypothesis_revision_id
           ) VALUES(
               $1,$2,$3,'https://legacy-report.example.test/login',
               'legacy report authority hypothesis',$4,'WSTG-INFO-01',
               'report finalization fixture','medium',0,'refuted',$5,$6,$7,$8,
               $9,$10,$11,$12,'attack_candidate',$13,$13,$13,$18,'url',
               'https://legacy-report.example.test/login',$14,$15,$16,
               'deterministic_safe',$17
           )"#,
    )
    .bind(candidate_id)
    .bind(scope.operation_id.to_string())
    .bind(scope.organization_id)
    .bind(tagged_digest('8'))
    .bind(scope.operation_id)
    .bind(scope.snapshot_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(target_id)
    .bind(&target_identity_hash)
    .bind(serde_json::json!({"schema_version":"report-finalization-fixture-v1"}))
    .bind(&candidate_plan_hash)
    .bind(hypothesis_revision_id)
    .bind(serde_json::json!({
        "targetIdAtTime":target_id,
        "targetTypeAtTime":"url",
        "targetValueAtTime":"https://legacy-report.example.test/login",
        "targetIdentityHash":target_identity_hash,
    }))
    .execute(&mut *legacy_tx)
    .await
    .expect("insert retained legacy Candidate authority parent");
    sqlx::query(
        r#"INSERT INTO candidate_attempts(
               id,candidate_id,approval_id,operation_id,scope_snapshot_id,
               wave_run_id,wave_unit_id,organization_id,target_live_id,
               target_type_at_time,target_value_at_time,target_identity_hash,
               candidate_plan_hash,ordinal,status,result_json,result_hash,terminal_at
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,'url',
               'https://legacy-report.example.test/login',$10,$11,0,'refuted',
               $12,$13,NOW()
           )"#,
    )
    .bind(attempt_id)
    .bind(candidate_id)
    .bind(Uuid::new_v4())
    .bind(scope.operation_id)
    .bind(scope.snapshot_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(scope.organization_id)
    .bind(target_id)
    .bind(&target_identity_hash)
    .bind(&candidate_plan_hash)
    .bind(serde_json::json!({"disposition":"refuted"}))
    .bind(tagged_digest('9'))
    .execute(&mut *legacy_tx)
    .await
    .expect("insert retained legacy terminal Attempt");
    sqlx::query(
        "INSERT INTO candidate_attempt_evidence(attempt_id,evidence_id,role) VALUES($1,$2,'proof')",
    )
    .bind(attempt_id)
    .bind(evidence_id)
    .execute(&mut *legacy_tx)
    .await
    .expect("link exact evidence to legacy Attempt");
    sqlx::query("SET LOCAL session_replication_role = 'origin'")
        .execute(&mut *legacy_tx)
        .await
        .expect("restore legacy authority fixture guards");
    legacy_security_verdict::seal_legacy_attempt_authority_on(
        &mut legacy_tx,
        legacy_security_verdict::SealLegacyAttemptAuthorityV1 {
            operation_id: scope.operation_id,
            project_scope_id: scope.project_scope_id,
            organization_id: scope.organization_id,
            attempt_id,
            hypothesis_revision_id,
            adapter_version: "report-finalization-test-v1".to_owned(),
            adapter_digest: tagged_digest('a'),
        },
    )
    .await
    .expect("seal retained legacy Attempt authority");
    legacy_report_authority::seal_legacy_report_authority_on(
        &mut legacy_tx,
        legacy_report_authority::SealLegacyReportAuthorityV1 {
            operation_id: scope.operation_id,
            project_scope_id: scope.project_scope_id,
            adapter_version: "report-finalization-test-v1".to_owned(),
            adapter_digest: tagged_digest('a'),
        },
    )
    .await
    .expect("seal operation-wide legacy report authority");
    legacy_tx
        .commit()
        .await
        .expect("commit retained legacy report authority");

    ReportSourceSnapshot::freeze(
        "fixture",
        vec![ReportSourceVersion {
            kind: ReportSourceKind::EvidenceAudit,
            authority_class: Default::default(),
            id: CanonicalRowId::Int64(evidence_id),
            row_version: 0,
            content_hash: [0x55; 32],
        }],
    )
    .expect("freeze a non-empty report source snapshot")
}

async fn report_revision_fixture(
    db: &GolishDb,
    scope: FrozenScope,
    validated: bool,
) -> ReportRevisionFixture {
    let report_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let section_id = Uuid::new_v4();
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin report revision fixture");
    sqlx::query(
        r#"INSERT INTO reports(
               report_id,operation_id,project_scope_id,scope_snapshot_id,scope_snapshot_hash
           ) VALUES($1,$2,$3,$4,$5)"#,
    )
    .bind(report_id)
    .bind(scope.operation_id)
    .bind(scope.project_scope_id)
    .bind(scope.snapshot_id)
    .bind("3".repeat(64))
    .execute(&mut *tx)
    .await
    .expect("insert report fixture");
    sqlx::query(
        r#"INSERT INTO report_revisions(
               revision_id,report_id,revision_number,transaction_snapshot,source_set_hash
           ) VALUES($1,$2,1,'fixture',$3)"#,
    )
    .bind(revision_id)
    .bind(report_id)
    .bind("4".repeat(64))
    .execute(&mut *tx)
    .await
    .expect("insert report revision fixture");
    sqlx::query(
        r#"INSERT INTO report_sections(
               section_id,revision_id,organization_id_at_time,
               organization_name_at_snapshot,section_kind,ordinal,content_hash
           ) VALUES($1,$2,$3,'Report Org','findings',0,$4)"#,
    )
    .bind(section_id)
    .bind(revision_id)
    .bind(scope.organization_id)
    .bind("5".repeat(64))
    .execute(&mut *tx)
    .await
    .expect("insert report section fixture");
    sqlx::query("UPDATE report_revisions SET validation_status='draft' WHERE revision_id=$1")
        .bind(revision_id)
        .execute(&mut *tx)
        .await
        .expect("move report revision fixture to draft");
    if validated {
        sqlx::query(
            r#"UPDATE report_revisions
                  SET validation_status='validated',validation_result=$2,validated_at=NOW()
                WHERE revision_id=$1"#,
        )
        .bind(revision_id)
        .bind(serde_json::json!({
            "revision_id": revision_id,
            "claim_count": 0,
            "citation_count": 0,
            "source_count": 0,
        }))
        .execute(&mut *tx)
        .await
        .expect("validate report revision fixture");
        sqlx::query("UPDATE reports SET current_revision_id=$2 WHERE report_id=$1")
            .bind(report_id)
            .bind(revision_id)
            .execute(&mut *tx)
            .await
            .expect("make validated report revision current");
    }
    let row_version: i64 =
        sqlx::query_scalar("SELECT row_version FROM report_revisions WHERE revision_id=$1")
            .bind(revision_id)
            .fetch_one(&mut *tx)
            .await
            .expect("load report revision fixture version");
    tx.commit().await.expect("commit report revision fixture");
    ReportRevisionFixture {
        report_id,
        revision_id,
        section_id,
        row_version,
    }
}

async fn authority_complete_report_revision_fixture(
    db: &GolishDb,
    scope: FrozenScope,
) -> (ReportRevisionFixture, ReportSourceSnapshot) {
    let report_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let section_id = Uuid::new_v4();
    let snapshot = seed_report_finalization_authority(db, scope, revision_id).await;
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin authority-complete report revision fixture");
    sqlx::query(
        r#"INSERT INTO reports(
               report_id,operation_id,project_scope_id,scope_snapshot_id,scope_snapshot_hash
           ) VALUES($1,$2,$3,$4,$5)"#,
    )
    .bind(report_id)
    .bind(scope.operation_id)
    .bind(scope.project_scope_id)
    .bind(scope.snapshot_id)
    .bind("3".repeat(64))
    .execute(&mut *tx)
    .await
    .expect("insert authority-complete report");
    let building = report_revisions::begin_revision(
        &mut tx,
        &report_revisions::BeginReportRevision {
            revision_id,
            report_id,
            revision_number: 1,
            expected_report_current_revision_id: None,
            snapshot: snapshot.clone(),
        },
    )
    .await
    .expect("begin report revision with a non-empty canonical manifest");
    sqlx::query(
        r#"INSERT INTO report_sections(
               section_id,revision_id,organization_id_at_time,
               organization_name_at_snapshot,section_kind,ordinal,content_hash
           ) VALUES($1,$2,$3,'Report Org','findings',0,$4)"#,
    )
    .bind(section_id)
    .bind(revision_id)
    .bind(scope.organization_id)
    .bind("5".repeat(64))
    .execute(&mut *tx)
    .await
    .expect("insert authority-complete report section");
    let draft_row_version: i64 = sqlx::query_scalar(
        "UPDATE report_revisions SET validation_status='draft' WHERE revision_id=$1 RETURNING row_version",
    )
    .bind(revision_id)
    .fetch_one(&mut *tx)
    .await
    .expect("move authority-complete revision to draft");
    assert!(draft_row_version > building.row_version);
    let seal = report_input_authority::seal_current_report_input_authority_on(
        &mut tx,
        scope.operation_id,
        revision_id,
        &snapshot,
    )
    .await
    .expect("freeze current report input authority");
    report_input_seals::seal_report_input_on(
        &mut tx,
        scope.operation_id,
        revision_id,
        &snapshot,
        &seal,
    )
    .await
    .expect("persist the immutable report input seal");
    let validated = report_revisions::validate_revision(
        &mut tx,
        &report_revisions::ValidateReportRevision {
            report_id,
            revision_id,
            expected_row_version: draft_row_version,
            expected_source_set_hash: snapshot.source_set_hash,
            validation_result: ReportValidationResult {
                revision_id,
                claim_count: 0,
                citation_count: 0,
                source_count: snapshot.ordered_sources.len(),
            },
        },
    )
    .await
    .expect("validate authority-complete report revision");
    tx.commit()
        .await
        .expect("commit authority-complete report revision fixture");
    (
        ReportRevisionFixture {
            report_id,
            revision_id,
            section_id,
            row_version: validated.row_version,
        },
        snapshot,
    )
}

#[tokio::test]
#[serial]
async fn validated_unpublished_revision_rejects_attestation_source_and_metadata_updates() {
    let (mut db, _data_dir) = fixture().await;
    let scope = frozen_scope(&db, "/fixture/validated-revision-freeze").await;
    let revision = report_revision_fixture(&db, scope, true).await;
    for (label, statement) in [
        (
            "validation attestation",
            "UPDATE report_revisions SET validation_result='{}'::jsonb WHERE revision_id=$1",
        ),
        (
            "source identity",
            "UPDATE report_revisions SET source_set_hash=repeat('a',64) WHERE revision_id=$1",
        ),
        (
            "revision metadata",
            "UPDATE report_revisions SET transaction_snapshot='tampered' WHERE revision_id=$1",
        ),
    ] {
        let error = sqlx::query(statement)
            .bind(revision.revision_id)
            .execute(db.pool())
            .await
            .expect_err(label);
        assert!(
            error
                .to_string()
                .contains("REPORT_VALIDATED_REVISION_IMMUTABLE"),
            "{label} returned the wrong rejection: {error}"
        );
    }
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn plain_sql_cannot_finalize_a_validated_current_revision() {
    let (mut db, _data_dir) = fixture().await;
    let scope = frozen_scope(&db, "/fixture/direct-final-bypass").await;
    let revision = report_revision_fixture(&db, scope, true).await;
    let principal_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM operator_principals WHERE principal_kind='local_operator' AND active",
    )
    .fetch_one(db.pool())
    .await
    .expect("load active local principal");
    let mut artifact_tx = db.pool().begin().await.expect("begin preattached artifact");
    sqlx::query(
        r#"INSERT INTO report_artifact_blobs(content_key,sha256,storage_path,byte_len)
           VALUES('sha256/aa/direct-final.md',$1,'.golish/reports/blobs/direct-final.md',12)"#,
    )
    .bind("b".repeat(64))
    .execute(&mut *artifact_tx)
    .await
    .expect("insert verified direct-final blob reference");
    sqlx::query(
        r#"INSERT INTO report_revision_artifacts(
               revision_id,artifact_kind,content_key,redaction_version
           ) VALUES($1,'markdown','sha256/aa/direct-final.md',1)"#,
    )
    .bind(revision.revision_id)
    .execute(&mut *artifact_tx)
    .await
    .expect("attach verified direct-final artifact reference");
    artifact_tx
        .commit()
        .await
        .expect("commit preattached direct-final artifact");
    let error = sqlx::query(
        r#"UPDATE report_revisions
              SET publication_status='final',finalized_at=NOW(),
                  finalized_by_principal_id=$2
            WHERE revision_id=$1"#,
    )
    .bind(revision.revision_id)
    .bind(principal_id)
    .execute(db.pool())
    .await
    .expect_err("plain SQL finalization must not bypass artifacts and outbox authority");
    assert!(
        error
            .to_string()
            .contains("REPORT_FINALIZATION_AUTHORITY_REQUIRED"),
        "direct final returned the wrong rejection: {error}"
    );
    let status: String =
        sqlx::query_scalar("SELECT publication_status FROM report_revisions WHERE revision_id=$1")
            .bind(revision.revision_id)
            .fetch_one(db.pool())
            .await
            .expect("reload revision after rejected direct final");
    assert_eq!(status, "unpublished");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn repository_finalize_commits_artifact_and_exact_outbox_with_final_transition() {
    let (mut db, _data_dir) = fixture().await;
    let scope = frozen_scope(&db, "/fixture/repository-final").await;
    let (revision, snapshot) = authority_complete_report_revision_fixture(&db, scope).await;
    let principal_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM operator_principals WHERE principal_kind='local_operator' AND active",
    )
    .fetch_one(db.pool())
    .await
    .expect("load active local principal");
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin repository finalization");
    let finalized = report_revisions::finalize_revision_with_artifacts_and_outbox(
        &mut tx,
        &report_revisions::FinalizeReportRevision {
            report_id: revision.report_id,
            revision_id: revision.revision_id,
            operation_id: scope.operation_id,
            project_scope_id: scope.project_scope_id,
            principal_id,
            expected_row_version: revision.row_version,
            expected_source_snapshot: snapshot.clone(),
            current_source_snapshot: snapshot,
            artifacts: vec![report_revisions::FinalizedArtifactRef {
                artifact_kind: "markdown".to_string(),
                content_key: "sha256/aa/repository-final.md".to_string(),
                sha256: "a".repeat(64),
                storage_path: ".golish/reports/blobs/repository-final.md".to_string(),
                byte_len: 12,
                redaction_version: 1,
            }],
        },
    )
    .await
    .expect("repository finalization succeeds");
    tx.commit()
        .await
        .expect("commit repository finalization protocol");
    assert_eq!(finalized.publication_status, "final");
    let outbox_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM knowledge_outbox_events
            WHERE event_name='ReportRevisionFinalized.v1'
              AND source_operation_id=$1
              AND source_id_value=$2
              AND source_version=$3"#,
    )
    .bind(scope.operation_id)
    .bind(revision.revision_id.to_string())
    .bind(finalized.row_version)
    .fetch_one(db.pool())
    .await
    .expect("count exact report finalization outbox event");
    assert_eq!(outbox_count, 1);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn child_insert_update_delete_serialize_before_draft_validation() {
    let (mut db, _data_dir) = fixture().await;
    for mutation in ["insert", "update", "delete"] {
        let scope =
            frozen_scope(&db, &format!("/fixture/report-child-validation-{mutation}")).await;
        let revision = report_revision_fixture(&db, scope, false).await;
        let mut child_tx = db.pool().begin().await.expect("begin child mutation");
        match mutation {
            "insert" => {
                sqlx::query(
                    r#"INSERT INTO report_sections(
                           section_id,revision_id,organization_id_at_time,
                           organization_name_at_snapshot,section_kind,ordinal,content_hash
                       ) VALUES($1,$2,$3,'Report Org','limitations',1,$4)"#,
                )
                .bind(Uuid::new_v4())
                .bind(revision.revision_id)
                .bind(scope.organization_id)
                .bind("6".repeat(64))
                .execute(&mut *child_tx)
                .await
                .expect("insert child while revision is draft");
            }
            "update" => {
                sqlx::query("UPDATE report_sections SET content_hash=$2 WHERE section_id=$1")
                    .bind(revision.section_id)
                    .bind("6".repeat(64))
                    .execute(&mut *child_tx)
                    .await
                    .expect("update child while revision is draft");
            }
            "delete" => {
                sqlx::query("DELETE FROM report_sections WHERE section_id=$1")
                    .bind(revision.section_id)
                    .execute(&mut *child_tx)
                    .await
                    .expect("delete child while revision is draft");
            }
            _ => unreachable!(),
        }

        let validation_started = Arc::new(Notify::new());
        let task_started = validation_started.clone();
        let pool = db.pool().clone();
        let revision_id = revision.revision_id;
        let mut validation_task = tokio::spawn(async move {
            let mut connection = pool.acquire().await.expect("acquire validation connection");
            task_started.notify_one();
            sqlx::query(
                r#"UPDATE report_revisions
                      SET validation_status='validated',validation_result=$2,validated_at=NOW()
                    WHERE revision_id=$1 AND validation_status='draft'"#,
            )
            .bind(revision_id)
            .bind(serde_json::json!({
                "revision_id": revision_id,
                "claim_count": 0,
                "citation_count": 0,
                "source_count": 0,
            }))
            .execute(&mut *connection)
            .await
        });
        validation_started.notified().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(300), &mut validation_task)
                .await
                .is_err(),
            "draft validation completed before the {mutation} child transaction committed"
        );
        child_tx
            .commit()
            .await
            .expect("commit serialized child mutation");
        let validated = validation_task
            .await
            .expect("join validation task")
            .expect("validate after child transaction");
        assert_eq!(validated.rows_affected(), 1);
    }
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn reporting_schema_installs_retained_read_model_and_blob_tables() {
    let (mut db, _data_dir) = fixture().await;
    for table in [
        "reports",
        "report_revisions",
        "report_source_manifest",
        "report_sections",
        "report_claims",
        "report_claim_citations",
        "report_artifact_blobs",
        "report_revision_artifacts",
        "report_input_tool_truth_authority_sets",
        "report_input_tool_truth_authority_members",
        "report_input_revision_adjudication_sets",
        "report_input_revision_adjudication_members",
        "report_input_open_headers",
        "report_input_seals",
        "report_input_seal_members",
        "report_authority_invalidation_events",
        "historical_report_artifact_receipts",
        "historical_report_artifact_read_attestations",
    ] {
        let exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
            .bind(format!("public.{table}"))
            .fetch_one(db.pool())
            .await
            .expect("query reporting table");
        assert!(exists, "missing reporting table {table}");
    }
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn every_mutable_canonical_report_source_has_a_row_version() {
    let (mut db, _data_dir) = fixture().await;
    for table in [
        "findings",
        "technique_outcomes",
        "attack_candidates",
        "candidate_attempts",
        "finding_lineage",
        "footholds",
        "internal_asset_observations",
        "attack_paths",
        "objective_attempts",
        "cleanup_obligations",
        "cleanup_waivers",
    ] {
        let exists: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1 FROM information_schema.columns
                    WHERE table_schema='public' AND table_name=$1
                      AND column_name='row_version'
               )"#,
        )
        .bind(table)
        .fetch_one(db.pool())
        .await
        .expect("query reportable row version");
        assert!(exists, "missing row_version on {table}");
    }
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn validated_reporting_children_are_immutable_for_insert_update_and_delete() {
    let (mut db, _data_dir) = fixture().await;
    let project_path = "/fixture/validated-report-history";
    let scope = frozen_scope(&db, project_path).await;
    let report_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let section_id = Uuid::new_v4();
    let claim_id = Uuid::new_v4();
    let citation_id = Uuid::new_v4();
    let source_id = Uuid::new_v4();
    let evidence_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO audit_log(
               action,category,details,project_path,source,status,detail,run_id,audit_role
           ) VALUES(
               'validated-report-evidence','harness','validated report evidence',$1,
               'harness','completed',$2,$3,'evidence'
           ) RETURNING id"#,
    )
    .bind(project_path)
    .bind(serde_json::json!({"organization_id": scope.organization_id}))
    .bind(scope.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("insert retained report evidence");

    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin validated report fixture");
    sqlx::query(
        r#"INSERT INTO reports(
               report_id,operation_id,project_scope_id,scope_snapshot_id,scope_snapshot_hash
           ) VALUES($1,$2,$3,$4,$5)"#,
    )
    .bind(report_id)
    .bind(scope.operation_id)
    .bind(scope.project_scope_id)
    .bind(scope.snapshot_id)
    .bind("3".repeat(64))
    .execute(&mut *tx)
    .await
    .expect("insert validated report aggregate");
    sqlx::query(
        r#"INSERT INTO report_revisions(
               revision_id,report_id,revision_number,transaction_snapshot,source_set_hash
           ) VALUES($1,$2,1,'validated-fixture',$3)"#,
    )
    .bind(revision_id)
    .bind(report_id)
    .bind("4".repeat(64))
    .execute(&mut *tx)
    .await
    .expect("insert building report revision");
    sqlx::query(
        r#"INSERT INTO report_source_manifest(
               revision_id,ordinal,source_kind,source_id_kind,source_id_value,
               source_row_version,content_hash
           ) VALUES($1,0,'finding','uuid',$2,1,decode($3,'hex'))"#,
    )
    .bind(revision_id)
    .bind(source_id.to_string())
    .bind("5".repeat(64))
    .execute(&mut *tx)
    .await
    .expect("insert building source manifest");
    sqlx::query(
        r#"INSERT INTO report_sections(
               section_id,revision_id,organization_id_at_time,
               organization_name_at_snapshot,section_kind,ordinal,content_hash
           ) VALUES($1,$2,$3,'Report Org','findings',0,$4)"#,
    )
    .bind(section_id)
    .bind(revision_id)
    .bind(scope.organization_id)
    .bind("6".repeat(64))
    .execute(&mut *tx)
    .await
    .expect("insert building report section");
    sqlx::query(
        r#"INSERT INTO report_claims(
               claim_id,revision_id,section_id,organization_id_at_time,claim_kind,
               subject_ref,predicate,object_value,claim_hash,ordinal
           ) VALUES($1,$2,$3,$4,'finding','finding-1','verified','{}',$5,0)"#,
    )
    .bind(claim_id)
    .bind(revision_id)
    .bind(section_id)
    .bind(scope.organization_id)
    .bind("7".repeat(64))
    .execute(&mut *tx)
    .await
    .expect("insert building report claim");
    sqlx::query(
        r#"INSERT INTO report_claim_citations(
               citation_id,revision_id,claim_id,citation_ordinal,source_type,
               source_kind,source_id_kind,source_id_value,source_row_version,
               source_hash,evidence_audit_id,organization_id_at_time,display_label
           ) VALUES(
               $1,$2,$3,0,'canonical_fact','finding','uuid',$4,1,
               decode($5,'hex'),$6,$7,'retained evidence'
           )"#,
    )
    .bind(citation_id)
    .bind(revision_id)
    .bind(claim_id)
    .bind(source_id.to_string())
    .bind("5".repeat(64))
    .bind(evidence_id)
    .bind(scope.organization_id)
    .execute(&mut *tx)
    .await
    .expect("insert building report citation");
    sqlx::query(
        "UPDATE report_revisions SET validation_status='validated',validated_at=NOW() WHERE revision_id=$1",
    )
    .bind(revision_id)
    .execute(&mut *tx)
    .await
    .expect("terminalize report validation");
    tx.commit().await.expect("commit validated report fixture");

    let mutations = [
        (
            "manifest insert",
            format!(
                "INSERT INTO report_source_manifest(revision_id,ordinal,source_kind,source_id_kind,source_id_value,source_row_version,content_hash) VALUES('{revision_id}',1,'finding','uuid','{}',1,decode('{}','hex'))",
                Uuid::new_v4(),
                "8".repeat(64)
            ),
        ),
        (
            "manifest update",
            format!(
                "UPDATE report_source_manifest SET ordinal=ordinal WHERE revision_id='{revision_id}'"
            ),
        ),
        (
            "manifest delete",
            format!("DELETE FROM report_source_manifest WHERE revision_id='{revision_id}'"),
        ),
        (
            "section insert",
            format!(
                "INSERT INTO report_sections(section_id,revision_id,section_kind,ordinal,content_hash) VALUES('{}','{revision_id}','limitations',1,'{}')",
                Uuid::new_v4(),
                "9".repeat(64)
            ),
        ),
        (
            "section update",
            format!("UPDATE report_sections SET ordinal=ordinal WHERE section_id='{section_id}'"),
        ),
        (
            "section delete",
            format!("DELETE FROM report_sections WHERE section_id='{section_id}'"),
        ),
        (
            "claim insert",
            format!(
                "INSERT INTO report_claims(claim_id,revision_id,section_id,claim_kind,subject_ref,predicate,object_value,claim_hash,ordinal) VALUES('{}','{revision_id}','{section_id}','finding','finding-2','verified','{{}}','{}',1)",
                Uuid::new_v4(),
                "a".repeat(64)
            ),
        ),
        (
            "claim update",
            format!("UPDATE report_claims SET ordinal=ordinal WHERE claim_id='{claim_id}'"),
        ),
        (
            "claim delete",
            format!("DELETE FROM report_claims WHERE claim_id='{claim_id}'"),
        ),
        (
            "citation insert",
            format!(
                "INSERT INTO report_claim_citations(citation_id,revision_id,claim_id,citation_ordinal,source_type,source_kind,source_id_kind,source_id_value,source_row_version,source_hash,evidence_audit_id,organization_id_at_time,display_label) VALUES('{}','{revision_id}','{claim_id}',1,'canonical_fact','finding','uuid','{source_id}',1,decode('{}','hex'),{evidence_id},'{}','late evidence')",
                Uuid::new_v4(),
                "5".repeat(64),
                scope.organization_id
            ),
        ),
        (
            "citation update",
            format!(
                "UPDATE report_claim_citations SET citation_ordinal=citation_ordinal WHERE citation_id='{citation_id}'"
            ),
        ),
        (
            "citation delete",
            format!("DELETE FROM report_claim_citations WHERE citation_id='{citation_id}'"),
        ),
    ];
    for (label, statement) in mutations {
        let error = sqlx::query(&statement)
            .execute(db.pool())
            .await
            .expect_err(label);
        assert!(
            error.to_string().contains("FINAL_HISTORY_IMMUTABLE"),
            "{label} returned the wrong rejection: {error}"
        );
    }

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn finalized_content_is_immutable_and_blob_is_shareable_across_revisions() {
    let (mut db, _data_dir) = fixture().await;
    let scope = frozen_scope(&db, "/fixture/report-history").await;
    let principal_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM operator_principals WHERE principal_kind='local_operator' AND active",
    )
    .fetch_one(db.pool())
    .await
    .expect("load server-owned principal");
    let report_id = Uuid::new_v4();
    let revision_one = Uuid::new_v4();
    let revision_two = Uuid::new_v4();
    let section_id = Uuid::new_v4();
    let claim_id = Uuid::new_v4();
    let snapshot = seed_report_finalization_authority(&db, scope, revision_one).await;
    let source_evidence_id = match snapshot.ordered_sources[0].id {
        CanonicalRowId::Int64(value) => value,
        _ => panic!("report finalization fixture source must be audit evidence"),
    };
    let revision_one_source_hash = snapshot
        .source_set_hash
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();

    let mut tx = db.pool().begin().await.expect("begin report history");
    sqlx::query(
        r#"INSERT INTO reports(
               report_id,operation_id,project_scope_id,scope_snapshot_id,scope_snapshot_hash
           ) VALUES($1,$2,$3,$4,$5)"#,
    )
    .bind(report_id)
    .bind(scope.operation_id)
    .bind(scope.project_scope_id)
    .bind(scope.snapshot_id)
    .bind("3".repeat(64))
    .execute(&mut *tx)
    .await
    .expect("insert report");
    for (revision_id, number, source_set_hash) in [
        (revision_one, 1, revision_one_source_hash.clone()),
        (revision_two, 2, "4".repeat(64)),
    ] {
        sqlx::query(
            r#"INSERT INTO report_revisions(
                   revision_id,report_id,revision_number,transaction_snapshot,
                   source_set_hash,validation_status,publication_status
               ) VALUES($1,$2,$3,'fixture',$4,'building','unpublished')"#,
        )
        .bind(revision_id)
        .bind(report_id)
        .bind(number)
        .bind(source_set_hash)
        .execute(&mut *tx)
        .await
        .expect("insert report revision");
    }
    report_source_manifest::insert_snapshot(&mut tx, revision_one, &snapshot)
        .await
        .expect("insert first revision's non-empty canonical manifest");
    sqlx::query(
        r#"INSERT INTO report_sections(
               section_id,revision_id,organization_id_at_time,
               organization_name_at_snapshot,section_kind,ordinal,content_hash
           ) VALUES($1,$2,$3,'Report Org','findings',0,$4)"#,
    )
    .bind(section_id)
    .bind(revision_one)
    .bind(Uuid::new_v4())
    .bind("5".repeat(64))
    .execute(&mut *tx)
    .await
    .expect("insert report section");
    sqlx::query(
        r#"INSERT INTO report_claims(
               claim_id,revision_id,section_id,claim_kind,subject_ref,
               predicate,object_value,claim_hash,ordinal
           ) VALUES($1,$2,$3,'finding','finding-1','verified',$4,$5,0)"#,
    )
    .bind(claim_id)
    .bind(revision_one)
    .bind(section_id)
    .bind(serde_json::json!({"severity": "high"}))
    .bind("6".repeat(64))
    .execute(&mut *tx)
    .await
    .expect("insert report claim");
    sqlx::query(
        r#"INSERT INTO report_claim_citations(
               citation_id,revision_id,claim_id,citation_ordinal,source_type,
               source_kind,source_id_kind,source_id_value,source_row_version,
               source_hash,evidence_audit_id,organization_id_at_time,display_label
           ) VALUES(
               $1,$2,$3,0,'canonical_fact','evidence_audit','int64',$4,0,
               decode($5,'hex'),$6,$7,'retained report authority evidence'
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(revision_one)
    .bind(claim_id)
    .bind(source_evidence_id.to_string())
    .bind("55".repeat(32))
    .bind(source_evidence_id)
    .bind(scope.organization_id)
    .execute(&mut *tx)
    .await
    .expect("cite the retained non-empty report source");
    sqlx::query(
        r#"INSERT INTO report_artifact_blobs(
               content_key,sha256,storage_path,byte_len
           ) VALUES('sha256/aa/shared.md',$1,'.golish/reports/blobs/shared.md',12)"#,
    )
    .bind("a".repeat(64))
    .execute(&mut *tx)
    .await
    .expect("insert shared blob");
    sqlx::query(
        r#"INSERT INTO report_revision_artifacts(
               revision_id,artifact_kind,content_key,redaction_version
           ) VALUES($1,'markdown','sha256/aa/shared.md',1)"#,
    )
    .bind(revision_two)
    .execute(&mut *tx)
    .await
    .expect("attach shared blob to the unpublished successor");
    let draft_row_version: i64 = sqlx::query_scalar(
        "UPDATE report_revisions SET validation_status='draft' WHERE revision_id=$1 RETURNING row_version",
    )
    .bind(revision_one)
    .fetch_one(&mut *tx)
    .await
    .expect("move first revision to draft");
    let authority_seal = report_input_authority::seal_current_report_input_authority_on(
        &mut tx,
        scope.operation_id,
        revision_one,
        &snapshot,
    )
    .await
    .expect("freeze first revision's current authority");
    report_input_seals::seal_report_input_on(
        &mut tx,
        scope.operation_id,
        revision_one,
        &snapshot,
        &authority_seal,
    )
    .await
    .expect("seal first revision's canonical input set");
    let validated = report_revisions::validate_revision(
        &mut tx,
        &report_revisions::ValidateReportRevision {
            report_id,
            revision_id: revision_one,
            expected_row_version: draft_row_version,
            expected_source_set_hash: snapshot.source_set_hash,
            validation_result: ReportValidationResult {
                revision_id: revision_one,
                claim_count: 1,
                citation_count: 1,
                source_count: snapshot.ordered_sources.len(),
            },
        },
    )
    .await
    .expect("validate first revision through the repository protocol");
    let revision_one_version = validated.row_version;
    report_revisions::finalize_revision_with_artifacts_and_outbox(
        &mut tx,
        &report_revisions::FinalizeReportRevision {
            report_id,
            revision_id: revision_one,
            operation_id: scope.operation_id,
            project_scope_id: scope.project_scope_id,
            principal_id,
            expected_row_version: revision_one_version,
            expected_source_snapshot: snapshot.clone(),
            current_source_snapshot: snapshot,
            artifacts: vec![report_revisions::FinalizedArtifactRef {
                artifact_kind: "markdown".to_string(),
                content_key: "sha256/aa/shared.md".to_string(),
                sha256: "a".repeat(64),
                storage_path: ".golish/reports/blobs/shared.md".to_string(),
                byte_len: 12,
                redaction_version: 1,
            }],
        },
    )
    .await
    .expect("finalize first revision through the repository protocol");
    tx.commit().await.expect("commit final report");

    let mut reuse_tx = db.pool().begin().await.expect("begin shared blob reuse");
    let reused = golish_db::repo::report_artifact_blobs::put(
        &mut reuse_tx,
        &golish_db::repo::report_artifact_blobs::PutReportArtifactBlob {
            content_key: "sha256/aa/shared.md".to_string(),
            sha256: "a".repeat(64),
            storage_path: ".golish/reports/blobs/shared.md".to_string(),
            byte_len: 12,
        },
    )
    .await
    .expect("reuse referenced blob without updating immutable identity");
    reuse_tx.commit().await.expect("commit shared blob reuse");
    assert_eq!(reused.content_key, "sha256/aa/shared.md");

    let claim_error = sqlx::query("UPDATE report_claims SET object_value=$2 WHERE claim_id=$1")
        .bind(claim_id)
        .bind(serde_json::json!({"severity": "critical"}))
        .execute(db.pool())
        .await
        .expect_err("finalized claim must be immutable");
    assert!(claim_error.to_string().contains("FINAL_HISTORY_IMMUTABLE"));

    let moved_section_error =
        sqlx::query("UPDATE report_sections SET revision_id=$2 WHERE section_id=$1")
            .bind(section_id)
            .bind(revision_two)
            .execute(db.pool())
            .await
            .expect_err("finalized child cannot be moved to an unpublished revision");
    assert!(moved_section_error
        .to_string()
        .contains("FINAL_HISTORY_IMMUTABLE"));

    let revision_error =
        sqlx::query("UPDATE report_revisions SET source_set_hash=$2 WHERE revision_id=$1")
            .bind(revision_one)
            .bind("f".repeat(64))
            .execute(db.pool())
            .await
            .expect_err("finalized revision content must be immutable");
    assert!(revision_error
        .to_string()
        .contains("FINAL_HISTORY_IMMUTABLE"));

    let manifest_insert = sqlx::query(
        r#"INSERT INTO report_source_manifest(
               revision_id,ordinal,source_kind,source_id_kind,source_id_value,
               source_row_version,content_hash
           ) VALUES($1,0,'finding','uuid',$2,1,decode($3,'hex'))"#,
    )
    .bind(revision_one)
    .bind(Uuid::new_v4().to_string())
    .bind("7".repeat(64))
    .execute(db.pool())
    .await
    .expect_err("finalized manifest must reject inserts");
    assert!(manifest_insert
        .to_string()
        .contains("FINAL_HISTORY_IMMUTABLE"));

    let section_insert = sqlx::query(
        r#"INSERT INTO report_sections(
               section_id,revision_id,section_kind,ordinal,content_hash
           ) VALUES($1,$2,'limitations',99,$3)"#,
    )
    .bind(Uuid::new_v4())
    .bind(revision_one)
    .bind("8".repeat(64))
    .execute(db.pool())
    .await
    .expect_err("finalized section must reject inserts");
    assert!(section_insert
        .to_string()
        .contains("FINAL_HISTORY_IMMUTABLE"));

    let claim_insert = sqlx::query(
        r#"INSERT INTO report_claims(
               claim_id,revision_id,section_id,claim_kind,subject_ref,
               predicate,object_value,claim_hash,ordinal
           ) VALUES($1,$2,$3,'finding','late-finding','verified','{}',$4,1)"#,
    )
    .bind(Uuid::new_v4())
    .bind(revision_one)
    .bind(section_id)
    .bind("9".repeat(64))
    .execute(db.pool())
    .await
    .expect_err("finalized claim must reject inserts");
    assert!(claim_insert.to_string().contains("FINAL_HISTORY_IMMUTABLE"));

    let citation_insert = sqlx::query(
        r#"INSERT INTO report_claim_citations(
               citation_id,revision_id,claim_id,citation_ordinal,source_type,
               source_kind,source_id_kind,source_id_value,source_row_version,
               source_hash,evidence_audit_id,organization_id_at_time,display_label
           ) VALUES(
               $1,$2,$3,1,'canonical_fact','finding','uuid',$4,1,
               decode($5,'hex'),9223372036854775807,$6,'late citation'
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(revision_one)
    .bind(claim_id)
    .bind(Uuid::new_v4().to_string())
    .bind("a".repeat(64))
    .bind(Uuid::new_v4())
    .execute(db.pool())
    .await
    .expect_err("finalized citation must reject inserts before foreign-key resolution");
    assert!(citation_insert
        .to_string()
        .contains("FINAL_HISTORY_IMMUTABLE"));

    let artifact_insert = sqlx::query(
        r#"INSERT INTO report_revision_artifacts(
               revision_id,artifact_kind,content_key,redaction_version
           ) VALUES($1,'json','sha256/aa/shared.md',1)"#,
    )
    .bind(revision_one)
    .execute(db.pool())
    .await
    .expect_err("finalized revision artifact must reject inserts");
    assert!(artifact_insert
        .to_string()
        .contains("FINAL_HISTORY_IMMUTABLE"));

    let blob_update =
        sqlx::query("UPDATE report_artifact_blobs SET byte_len=byte_len+1 WHERE content_key=$1")
            .bind("sha256/aa/shared.md")
            .execute(db.pool())
            .await
            .expect_err("a referenced report blob identity must be immutable");
    assert!(blob_update
        .to_string()
        .contains("REPORT_ARTIFACT_BLOB_IMMUTABLE"));
    let blob_noop_update = sqlx::query(
        "UPDATE report_artifact_blobs SET content_key=content_key WHERE content_key=$1",
    )
    .bind("sha256/aa/shared.md")
    .execute(db.pool())
    .await
    .expect_err("a referenced report blob rejects even identity-preserving UPDATE");
    assert!(blob_noop_update
        .to_string()
        .contains("REPORT_ARTIFACT_BLOB_IMMUTABLE"));

    let final_row_version: i64 =
        sqlx::query_scalar("SELECT row_version FROM report_revisions WHERE revision_id=$1")
            .bind(revision_one)
            .fetch_one(db.pool())
            .await
            .expect("load final revision version");
    let combined_supersede_error = sqlx::query(
        r#"UPDATE report_revisions
              SET publication_status='superseded',validation_result='{}'::jsonb
            WHERE revision_id=$1"#,
    )
    .bind(revision_one)
    .execute(db.pool())
    .await
    .expect_err("final to superseded may change only publication status");
    assert!(combined_supersede_error
        .to_string()
        .contains("FINAL_HISTORY_IMMUTABLE"));

    let superseded_row_version: i64 = sqlx::query_scalar(
        "UPDATE report_revisions SET publication_status='superseded' WHERE revision_id=$1 RETURNING row_version",
    )
        .bind(revision_one)
        .fetch_one(db.pool())
        .await
        .expect("supersede retained final revision");
    assert_eq!(superseded_row_version, final_row_version + 1);
    let superseded_insert = sqlx::query(
        r#"INSERT INTO report_sections(
               section_id,revision_id,section_kind,ordinal,content_hash
           ) VALUES($1,$2,'limitations',100,$3)"#,
    )
    .bind(Uuid::new_v4())
    .bind(revision_one)
    .bind("b".repeat(64))
    .execute(db.pool())
    .await
    .expect_err("superseded section must reject inserts");
    assert!(superseded_insert
        .to_string()
        .contains("FINAL_HISTORY_IMMUTABLE"));

    let references: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM report_revision_artifacts WHERE content_key='sha256/aa/shared.md'",
    )
    .fetch_one(db.pool())
    .await
    .expect("count shared blob references");
    assert_eq!(references, 2);
    db.stop().await;
}
