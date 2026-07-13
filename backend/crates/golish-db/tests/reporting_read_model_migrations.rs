use std::sync::Arc;
use std::time::Duration;

use golish_db::models::NewSession;
use golish_db::repo::{project_scopes, report_revisions, runtime_memory_tx, sessions};
use golish_db::{DbConfig, GolishDb};
use golish_reporting_domain::ReportSourceSnapshot;
use serial_test::serial;
use tokio::sync::Notify;
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
    let revision = report_revision_fixture(&db, scope, true).await;
    let principal_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM operator_principals WHERE principal_kind='local_operator' AND active",
    )
    .fetch_one(db.pool())
    .await
    .expect("load active local principal");
    let snapshot = ReportSourceSnapshot {
        transaction_snapshot: "fixture".to_string(),
        ordered_sources: Vec::new(),
        source_set_hash: [0x44; 32],
    };
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
    for (revision_id, number) in [(revision_one, 1), (revision_two, 2)] {
        sqlx::query(
            r#"INSERT INTO report_revisions(
                   revision_id,report_id,revision_number,transaction_snapshot,
                   source_set_hash,validation_status,publication_status
               ) VALUES($1,$2,$3,'fixture',$4,'building','unpublished')"#,
        )
        .bind(revision_id)
        .bind(report_id)
        .bind(number)
        .bind("4".repeat(64))
        .execute(&mut *tx)
        .await
        .expect("insert report revision");
    }
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
    sqlx::query(
        r#"UPDATE report_revisions
              SET validation_status='validated',validated_at=NOW()
            WHERE revision_id IN ($1,$2)"#,
    )
    .bind(revision_one)
    .bind(revision_two)
    .execute(&mut *tx)
    .await
    .expect("terminalize both report validations");
    sqlx::query("UPDATE reports SET current_revision_id=$2 WHERE report_id=$1")
        .bind(report_id)
        .bind(revision_one)
        .execute(&mut *tx)
        .await
        .expect("make first revision current");
    let revision_one_version: i64 =
        sqlx::query_scalar("SELECT row_version FROM report_revisions WHERE revision_id=$1")
            .bind(revision_one)
            .fetch_one(&mut *tx)
            .await
            .expect("load validated first revision version");
    let snapshot = ReportSourceSnapshot {
        transaction_snapshot: "fixture".to_string(),
        ordered_sources: Vec::new(),
        source_set_hash: [0x44; 32],
    };
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
