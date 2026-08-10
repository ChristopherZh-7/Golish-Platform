use chrono::{Duration, Utc};
use golish_db::models::NewSession;
use golish_db::repo::{
    cleanup_absence_checks, cleanup_attempts, cleanup_obligations, cleanup_waivers,
    operator_principals, organization_deletion_jobs, post_exploit_actions, post_exploit_approvals,
    project_scopes, runtime_memory_tx, sessions, sitemap_store,
};
use golish_db::{DbConfig, GolishDb, PgPool};
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
        database: format!("cleanup_kernel_{}", Uuid::new_v4().simple()),
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
    stage_execution_id: Uuid,
    project_scope_id: Uuid,
    snapshot_id: Uuid,
    organization_id: Uuid,
}

async fn frozen_scope(db: &GolishDb, project_path: &str, label: &str) -> FrozenScope {
    let session = sessions::create(
        db.pool(),
        NewSession {
            title: Some(format!("cleanup {label}")),
            workspace_path: Some(project_path.to_string()),
            workspace_label: None,
            model: None,
            provider: None,
            project_path: Some(project_path.to_string()),
        },
    )
    .await
    .expect("create cleanup fixture session");
    let project_scope =
        project_scopes::register_first_open(db.pool(), project_path, &format!("{label}-path-sha"))
            .await
            .expect("register cleanup project scope");
    let operation_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    runtime_memory_tx::create_runtime_operation(
        db.pool(),
        &runtime_memory_tx::CreateRuntimeOperationRow {
            operation_id,
            initial_stage_execution_id: stage_execution_id,
            session_id: session.id,
            title: Some(format!("cleanup operation {label}")),
            input: "cleanup fixture".to_string(),
            profile: "assessment".to_string(),
            entry_stage: "target_intel".to_string(),
            project_scope_id: project_scope.project_scope_id,
            application_model_contract: golish_core::ApplicationModelContract::LegacyNoModel,
            cli_scope: None,
        },
    )
    .await
    .expect("create cleanup runtime operation");
    let organization_id = Uuid::new_v4();
    let decision_id = Uuid::new_v4();
    let snapshot_id = Uuid::new_v4();
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin cleanup scope fixture");
    sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$2,$3)")
        .bind(organization_id)
        .bind(project_path)
        .bind(format!("{label} organization"))
        .execute(&mut *tx)
        .await
        .expect("insert cleanup organization");
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
    .bind(format!("{label}-decision"))
    .execute(&mut *tx)
    .await
    .expect("insert cleanup scope decision");
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
    .bind(format!("{label}-scope-hash"))
    .execute(&mut *tx)
    .await
    .expect("insert cleanup scope snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units(
               snapshot_id,organization_id,organization_name_at_freeze,
               role,depth,ordinal,decision_row_id,approval_source
           ) VALUES($1,$2,$3,'root',0,0,'root',$4)"#,
    )
    .bind(snapshot_id)
    .bind(organization_id)
    .bind(format!("{label} organization"))
    .bind(serde_json::json!({"source": "cli_flags"}))
    .execute(&mut *tx)
    .await
    .expect("insert cleanup frozen organization");
    sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
        .bind(snapshot_id)
        .execute(&mut *tx)
        .await
        .expect("seal cleanup scope");
    tx.commit().await.expect("commit cleanup frozen scope");
    FrozenScope {
        operation_id,
        stage_execution_id,
        project_scope_id: project_scope.project_scope_id,
        snapshot_id,
        organization_id,
    }
}

async fn evidence(db: &GolishDb, scope: FrozenScope, project_path: &str, label: &str) -> i64 {
    sqlx::query_scalar(
        r#"INSERT INTO audit_log(
               action,category,details,project_path,source,status,detail,run_id,audit_role
           ) VALUES($1,'harness',$2,$3,'harness','completed',$4,$5,'evidence')
           RETURNING id"#,
    )
    .bind(label)
    .bind(format!("{label} evidence"))
    .bind(project_path)
    .bind(serde_json::json!({"organization_id": scope.organization_id}))
    .bind(scope.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("insert exact cleanup evidence")
}

fn command(
    scope: FrozenScope,
    principal_id: Uuid,
    action_evidence: i64,
    obligation_evidence: i64,
) -> cleanup_obligations::RecordActionAndObligation {
    cleanup_obligations::RecordActionAndObligation {
        action_id: Uuid::new_v4(),
        obligation_id: Uuid::new_v4(),
        operation_id: scope.operation_id,
        project_scope_id: scope.project_scope_id,
        scope_snapshot_id: scope.snapshot_id,
        organization_id_at_time: scope.organization_id,
        principal_id,
        capability_id: "post_exploit.create_test_account".to_string(),
        side_effect_class: "remote_state_mutation".to_string(),
        action_plan: serde_json::json!({"kind": "create_test_account"}),
        action_plan_hash: "a".repeat(64),
        action_evidence: vec![(action_evidence, "plan".to_string())],
        affected_resource_snapshot: serde_json::json!({"kind": "account", "name": "test"}),
        resource_identity_hash: "b".repeat(64),
        cleanup_strategy: serde_json::json!({"kind": "delete_account"}),
        proof_requirements: serde_json::json!([{
            "kind": "independent_lookup",
            "independent_verifier_required": true
        }]),
        deadline: Utc::now() + Duration::hours(1),
        obligation_evidence: vec![(obligation_evidence, "source".to_string())],
    }
}

struct PreparedCleanupAttempt {
    obligation: cleanup_obligations::RecordActionAndObligation,
    pending_verification: cleanup_attempts::CleanupAttemptRow,
    verifier_worker_run_id: Uuid,
    absence_evidence_id: i64,
    terminal_evidence_ids: Vec<i64>,
}

#[derive(sqlx::FromRow)]
struct CleanupTerminalEventRow {
    event_id: Uuid,
    project_scope_id: Option<Uuid>,
    organization_id_at_time: Option<Uuid>,
    source_operation_id: Uuid,
    source_stream_key: String,
    source_version: i64,
    structured_payload: serde_json::Value,
    occurred_at: chrono::DateTime<Utc>,
}

async fn prepare_cleanup_attempt(
    db: &GolishDb,
    scope: FrozenScope,
    project_path: &str,
    label: &str,
    principal_id: Uuid,
) -> PreparedCleanupAttempt {
    let stage_run_unit_id = Uuid::new_v4();
    let executor_worker_run_id = Uuid::new_v4();
    let verifier_worker_run_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,
               organization_id,stage_kind,generation,specialist,status
           ) VALUES($1,$2,$3,$4,$5,'target_intel',0,'cleanup','running')"#,
    )
    .bind(stage_run_unit_id)
    .bind(scope.operation_id)
    .bind(scope.stage_execution_id)
    .bind(scope.snapshot_id)
    .bind(scope.organization_id)
    .execute(db.pool())
    .await
    .expect("insert cleanup worker fixture stage unit");
    for (worker_run_id, work_item_key, agent_path) in [
        (
            executor_worker_run_id,
            "cleanup-executor",
            "main>cleanup-executor",
        ),
        (
            verifier_worker_run_id,
            "cleanup-verifier",
            "main>cleanup-verifier",
        ),
    ] {
        sqlx::query(
            r#"INSERT INTO stage_worker_runs(
                   id,operation_id,stage_execution_id,stage_run_unit_id,
                   organization_id,worker_generation,specialist,work_item_kind,
                   work_item_key,agent_path,status
               ) VALUES($1,$2,$3,$4,$5,0,'cleanup','cleanup',$6,$7,'queued')"#,
        )
        .bind(worker_run_id)
        .bind(scope.operation_id)
        .bind(scope.stage_execution_id)
        .bind(stage_run_unit_id)
        .bind(scope.organization_id)
        .bind(work_item_key)
        .bind(agent_path)
        .execute(db.pool())
        .await
        .expect("insert independent cleanup worker fixture");
    }
    let action_evidence = evidence(db, scope, project_path, &format!("{label}-action")).await;
    let obligation_evidence =
        evidence(db, scope, project_path, &format!("{label}-obligation")).await;
    let execution_evidence = evidence(db, scope, project_path, &format!("{label}-execution")).await;
    let absence_evidence_id = evidence(db, scope, project_path, &format!("{label}-absence")).await;
    let obligation = command(scope, principal_id, action_evidence, obligation_evidence);
    cleanup_obligations::record_action_and_obligation(db.pool(), &obligation)
        .await
        .expect("record cleanup-attempt fixture obligation");
    let claimed = cleanup_attempts::claim(
        db.pool(),
        &cleanup_attempts::ClaimCleanupAttempt {
            obligation_id: obligation.obligation_id,
            lease_token: Uuid::new_v4(),
            lease_expires_at: Utc::now() + Duration::minutes(5),
            worker_run_id: Some(executor_worker_run_id),
        },
    )
    .await
    .expect("claim cleanup-attempt fixture");
    let executing = cleanup_attempts::transition(
        db.pool(),
        &cleanup_attempts::TransitionCleanupAttempt {
            attempt_id: claimed.id,
            lease_token: claimed.lease_token,
            expected_row_version: claimed.row_version,
            expected_status: "claimed".to_string(),
            next_status: "executing".to_string(),
            result: None,
            evidence: Vec::new(),
            terminal_note: None,
        },
    )
    .await
    .expect("start cleanup-attempt fixture");
    let pending_verification = cleanup_attempts::transition(
        db.pool(),
        &cleanup_attempts::TransitionCleanupAttempt {
            attempt_id: executing.id,
            lease_token: executing.lease_token,
            expected_row_version: executing.row_version,
            expected_status: "executing".to_string(),
            next_status: "cleaned_pending_verification".to_string(),
            result: Some(serde_json::json!({"cleanup": "submitted"})),
            evidence: vec![(execution_evidence, "result".to_string())],
            terminal_note: None,
        },
    )
    .await
    .expect("mark cleanup-attempt fixture pending verification");
    PreparedCleanupAttempt {
        obligation,
        pending_verification,
        verifier_worker_run_id,
        absence_evidence_id,
        terminal_evidence_ids: vec![obligation_evidence, execution_evidence, absence_evidence_id],
    }
}

fn cleanup_terminal_event_id(obligation_id: Uuid, source_version: i64) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("CleanupObligationTerminal.v1:{obligation_id}:{source_version}").as_bytes(),
    )
}

async fn assert_terminal_evidence_insert_rejected(
    pool: &PgPool,
    statement: &str,
    relation_id: Uuid,
    evidence_id: i64,
) {
    let error = sqlx::query(statement)
        .bind(relation_id)
        .bind(evidence_id)
        .execute(pool)
        .await
        .expect_err("terminal Cleanup evidence membership cannot grow");
    assert!(
        error
            .to_string()
            .contains("CLEANUP_TERMINAL_EVIDENCE_IMMUTABLE"),
        "unexpected terminal evidence rejection: {error}"
    );
}

async fn assert_terminal_cleanup_attempt_is_immutable(pool: &PgPool, attempt_id: Uuid) {
    for statement in [
        "UPDATE cleanup_attempts SET status=status WHERE id=$1",
        "DELETE FROM cleanup_attempts WHERE id=$1",
    ] {
        let error = sqlx::query(statement)
            .bind(attempt_id)
            .execute(pool)
            .await
            .expect_err("terminal CleanupAttempt row must be immutable");
        assert!(
            error
                .to_string()
                .contains("CLEANUP_TERMINAL_ATTEMPT_IMMUTABLE"),
            "unexpected terminal attempt rejection: {error}"
        );
    }
}

async fn install_cleanup_terminal_outbox_failure(db: &GolishDb) {
    sqlx::query(
        r#"CREATE FUNCTION reject_cleanup_terminal_outbox_fixture()
           RETURNS trigger AS $$
           BEGIN
               IF NEW.event_name = 'CleanupObligationTerminal.v1' THEN
                   RAISE EXCEPTION 'fixture cleanup terminal outbox failure';
               END IF;
               RETURN NEW;
           END;
           $$ LANGUAGE plpgsql"#,
    )
    .execute(db.pool())
    .await
    .expect("install cleanup terminal outbox failure function");
    sqlx::query(
        r#"CREATE TRIGGER reject_cleanup_terminal_outbox_fixture
           BEFORE INSERT ON knowledge_outbox_events
           FOR EACH ROW EXECUTE FUNCTION reject_cleanup_terminal_outbox_fixture()"#,
    )
    .execute(db.pool())
    .await
    .expect("install cleanup terminal outbox failure fixture");
}

#[tokio::test]
#[serial]
async fn cleanup_obligation_kernel_schema_is_installed() {
    let (mut db, _data_dir) = fixture().await;
    for table in [
        "cleanup_obligations",
        "cleanup_attempts",
        "cleanup_absence_checks",
        "cleanup_waivers",
    ] {
        let exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
            .bind(format!("public.{table}"))
            .fetch_one(db.pool())
            .await
            .expect("query cleanup table");
        assert!(exists, "missing cleanup table {table}");
    }
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn side_effect_action_and_obligation_commit_atomically_and_replay_exactly() {
    let (mut db, _data_dir) = fixture().await;
    let project_path = "/fixture/cleanup-atomic";
    let scope = frozen_scope(&db, project_path, "atomic").await;
    let principal = operator_principals::current_local(db.pool())
        .await
        .expect("load trusted cleanup principal");
    let action_evidence = evidence(&db, scope, project_path, "action").await;
    let obligation_evidence = evidence(&db, scope, project_path, "obligation").await;
    let input = command(scope, principal.id, action_evidence, obligation_evidence);

    let created = cleanup_obligations::record_action_and_obligation(db.pool(), &input)
        .await
        .expect("atomically record action and cleanup obligation");
    assert_eq!(created.action.id, input.action_id);
    assert_eq!(created.obligation.id, input.obligation_id);
    assert_eq!(created.obligation.status, "open");
    let prepared_event_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM knowledge_outbox_events
            WHERE event_name='PostExploitActionPrepared.v1'
              AND source_kind='post_exploit_action'
              AND source_id_value=$1"#,
    )
    .bind(input.action_id.to_string())
    .fetch_one(db.pool())
    .await
    .expect("count action-prepared memory event");
    assert_eq!(prepared_event_count, 1);
    let prepared_delivery_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM knowledge_projection_deliveries AS delivery
             JOIN knowledge_outbox_events AS event ON event.event_id=delivery.event_id
            WHERE event.event_name='PostExploitActionPrepared.v1'
              AND event.source_id_value=$1"#,
    )
    .bind(input.action_id.to_string())
    .fetch_one(db.pool())
    .await
    .expect("count action-prepared projector deliveries");
    assert_eq!(prepared_delivery_count, 4);
    let replay = cleanup_obligations::record_action_and_obligation(db.pool(), &input)
        .await
        .expect("exact response-loss replay");
    assert_eq!(replay, created);

    let extra_evidence = evidence(&db, scope, project_path, "extra").await;
    let mut drifted = input.clone();
    drifted
        .obligation_evidence
        .push((extra_evidence, "support".to_string()));
    assert!(
        cleanup_obligations::record_action_and_obligation(db.pool(), &drifted)
            .await
            .is_err(),
        "replay cannot append evidence"
    );
    let evidence_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM cleanup_obligation_evidence WHERE obligation_id=$1",
    )
    .bind(input.obligation_id)
    .fetch_one(db.pool())
    .await
    .expect("count immutable obligation evidence");
    assert_eq!(evidence_count, 1);

    let raw_unpaired = sqlx::query(
        r#"INSERT INTO post_exploit_actions(
               operation_id,project_scope_id,scope_snapshot_id,organization_id_at_time,
               capability_id,side_effect_class,plan,plan_hash
           ) VALUES($1,$2,$3,$4,'post_exploit.unpaired','remote_state_mutation',$5,$6)"#,
    )
    .bind(scope.operation_id)
    .bind(scope.project_scope_id)
    .bind(scope.snapshot_id)
    .bind(scope.organization_id)
    .bind(serde_json::json!({"kind": "unpaired"}))
    .bind("c".repeat(64))
    .execute(db.pool())
    .await;
    assert!(
        raw_unpaired.is_err(),
        "unpaired side effect must fail closed"
    );

    let read_only_action_id = Uuid::new_v4();
    let forged_obligation_id = Uuid::new_v4();
    let mut forged_tx = db.pool().begin().await.expect("begin forged cleanup pair");
    sqlx::query(
        r#"INSERT INTO post_exploit_actions(
               id,operation_id,project_scope_id,scope_snapshot_id,organization_id_at_time,
               capability_id,side_effect_class,plan,plan_hash
           ) VALUES($1,$2,$3,$4,$5,'post_exploit.inspect','none',$6,$7)"#,
    )
    .bind(read_only_action_id)
    .bind(scope.operation_id)
    .bind(scope.project_scope_id)
    .bind(scope.snapshot_id)
    .bind(scope.organization_id)
    .bind(serde_json::json!({"kind": "read_only"}))
    .bind("e".repeat(64))
    .execute(&mut *forged_tx)
    .await
    .expect("insert side-effect-free action fixture");
    sqlx::query(
        r#"INSERT INTO cleanup_obligations(
               id,operation_id,project_scope_id,scope_snapshot_id,organization_id_at_time,
               source_action_id,source_action_plan_hash,affected_resource_snapshot,
               resource_identity_hash,cleanup_strategy,proof_requirements,deadline
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)"#,
    )
    .bind(forged_obligation_id)
    .bind(scope.operation_id)
    .bind(scope.project_scope_id)
    .bind(scope.snapshot_id)
    .bind(scope.organization_id)
    .bind(read_only_action_id)
    .bind("e".repeat(64))
    .bind(serde_json::json!({"kind": "none"}))
    .bind("f".repeat(64))
    .bind(serde_json::json!({"kind": "none"}))
    .bind(serde_json::json!([{"kind": "lookup"}]))
    .bind(Utc::now() + Duration::hours(1))
    .execute(&mut *forged_tx)
    .await
    .expect("deferred action back-reference checks at constraint boundary");
    assert!(
        sqlx::query("SET CONSTRAINTS ALL IMMEDIATE")
            .execute(&mut *forged_tx)
            .await
            .is_err(),
        "a side-effect-free action cannot acquire a cleanup obligation"
    );
    forged_tx.rollback().await.ok();

    sqlx::raw_sql(
        r#"CREATE FUNCTION reject_cleanup_obligation_fixture()
           RETURNS trigger AS $$
           BEGIN
               RAISE EXCEPTION 'fixture rejects cleanup obligation';
           END;
           $$ LANGUAGE plpgsql;
           CREATE TRIGGER reject_cleanup_obligation_fixture
           BEFORE INSERT ON cleanup_obligations
           FOR EACH ROW EXECUTE FUNCTION reject_cleanup_obligation_fixture();"#,
    )
    .execute(db.pool())
    .await
    .expect("install cleanup failure fixture");
    let mut failed = command(scope, principal.id, action_evidence, obligation_evidence);
    failed.action_plan = serde_json::json!({"kind": "create_second_test_account"});
    failed.action_plan_hash = "d".repeat(64);
    assert!(
        cleanup_obligations::record_action_and_obligation(db.pool(), &failed)
            .await
            .is_err(),
        "obligation failure must abort its paired action"
    );
    let action_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM post_exploit_actions WHERE id=$1")
            .bind(failed.action_id)
            .fetch_one(db.pool())
            .await
            .expect("count rolled-back action");
    assert_eq!(action_count, 0);

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn approved_action_consumes_exact_approval_once_and_never_replays_execution() {
    let (mut db, _data_dir) = fixture().await;
    let project_path = "/fixture/post-exploit-approved-execution";
    let scope = frozen_scope(&db, project_path, "approved-execution").await;
    let principal = operator_principals::current_local(db.pool())
        .await
        .expect("load trusted execution principal");
    let action_evidence = evidence(&db, scope, project_path, "execution-plan").await;
    let obligation_evidence = evidence(&db, scope, project_path, "execution-obligation").await;
    let result_evidence = evidence(&db, scope, project_path, "execution-result").await;
    let input = command(scope, principal.id, action_evidence, obligation_evidence);
    cleanup_obligations::record_action_and_obligation(db.pool(), &input)
        .await
        .expect("prepare exact cleanup-bound action");

    let approval_id = Uuid::new_v4();
    let pending = post_exploit_approvals::create_pending(
        db.pool(),
        &post_exploit_approvals::NewPostExploitApproval {
            id: approval_id,
            action_id: input.action_id,
            operation_id: scope.operation_id,
            project_scope_id: scope.project_scope_id,
            scope_snapshot_id: scope.snapshot_id,
            organization_id_at_time: scope.organization_id,
            action_plan_hash: input.action_plan_hash.clone(),
        },
    )
    .await
    .expect("create pending post-exploit approval");
    let approved = post_exploit_approvals::decide(
        db.pool(),
        pending.id,
        pending.row_version,
        post_exploit_approvals::ApprovalDecision::Approve,
        principal.id,
        Some(Utc::now() + Duration::minutes(15)),
    )
    .await
    .expect("approve exact post-exploit plan");
    let begin = post_exploit_actions::BeginApprovedExecution {
        action_id: input.action_id,
        approval_id,
        expected_approval_row_version: approved.row_version,
        operation_id: scope.operation_id,
        project_scope_id: scope.project_scope_id,
        scope_snapshot_id: scope.snapshot_id,
        organization_id_at_time: scope.organization_id,
    };
    let executing = post_exploit_actions::begin_approved_execution(db.pool(), begin)
        .await
        .expect("consume exact approval and fence action");
    assert_eq!(executing.status, "executing");
    assert!(
        post_exploit_actions::begin_approved_execution(db.pool(), begin)
            .await
            .is_err(),
        "response loss must never replay the external action"
    );
    let consumed = post_exploit_approvals::get(db.pool(), approval_id)
        .await
        .expect("load consumed approval")
        .expect("approval exists");
    assert_eq!(consumed.status, "consumed");

    let succeeded = post_exploit_actions::finish_execution(
        db.pool(),
        executing.id,
        executing.row_version,
        post_exploit_actions::ExecutionDisposition::Succeeded,
        &[(result_evidence, "result".to_string())],
    )
    .await
    .expect("close execution with authoritative result evidence");
    assert_eq!(succeeded.status, "succeeded");
    assert!(succeeded.terminal_at.is_some());

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn inconclusive_absence_closes_attempt_and_allows_the_next_ordinal() {
    let (mut db, _data_dir) = fixture().await;
    let project_path = "/fixture/cleanup-retry";
    let scope = frozen_scope(&db, project_path, "retry").await;
    let principal = operator_principals::current_local(db.pool())
        .await
        .expect("load trusted cleanup principal");
    let action_evidence = evidence(&db, scope, project_path, "retry-action").await;
    let obligation_evidence = evidence(&db, scope, project_path, "retry-obligation").await;
    let execution_evidence = evidence(&db, scope, project_path, "retry-execution").await;
    let absence_evidence = evidence(&db, scope, project_path, "retry-absence").await;
    let input = command(scope, principal.id, action_evidence, obligation_evidence);
    cleanup_obligations::record_action_and_obligation(db.pool(), &input)
        .await
        .expect("record cleanup obligation");

    let first_claim = cleanup_attempts::ClaimCleanupAttempt {
        obligation_id: input.obligation_id,
        lease_token: Uuid::new_v4(),
        lease_expires_at: Utc::now() + Duration::minutes(5),
        worker_run_id: None,
    };
    let claimed = cleanup_attempts::claim(db.pool(), &first_claim)
        .await
        .expect("claim first cleanup attempt");
    assert_eq!(claimed.ordinal, 1);
    let competing = cleanup_attempts::ClaimCleanupAttempt {
        lease_token: Uuid::new_v4(),
        ..first_claim.clone()
    };
    assert!(
        cleanup_attempts::claim(db.pool(), &competing)
            .await
            .is_err(),
        "one obligation cannot have two live attempts"
    );
    let executing = cleanup_attempts::transition(
        db.pool(),
        &cleanup_attempts::TransitionCleanupAttempt {
            attempt_id: claimed.id,
            lease_token: claimed.lease_token,
            expected_row_version: claimed.row_version,
            expected_status: "claimed".to_string(),
            next_status: "executing".to_string(),
            result: None,
            evidence: Vec::new(),
            terminal_note: None,
        },
    )
    .await
    .expect("start cleanup execution");
    let pending_verification = cleanup_attempts::transition(
        db.pool(),
        &cleanup_attempts::TransitionCleanupAttempt {
            attempt_id: executing.id,
            lease_token: executing.lease_token,
            expected_row_version: executing.row_version,
            expected_status: "executing".to_string(),
            next_status: "cleaned_pending_verification".to_string(),
            result: Some(serde_json::json!({"cleanup": "submitted"})),
            evidence: vec![(execution_evidence, "result".to_string())],
            terminal_note: None,
        },
    )
    .await
    .expect("mark cleanup pending verification");
    let applied = cleanup_absence_checks::record_and_apply(
        db.pool(),
        &cleanup_absence_checks::RecordAbsenceCheck {
            id: Uuid::new_v4(),
            cleanup_attempt_id: pending_verification.id,
            verifier_worker_run_id: None,
            verifier_key: "independent-db-lookup".to_string(),
            resource_identity_hash: input.resource_identity_hash.clone(),
            disposition: "inconclusive".to_string(),
            evidence: vec![(absence_evidence, "inconclusive".to_string())],
        },
    )
    .await
    .expect("record inconclusive independent absence result");
    assert_eq!(applied.attempt.status, "verification_failed");
    assert_eq!(applied.obligation.status, "open");
    assert_terminal_cleanup_attempt_is_immutable(db.pool(), applied.attempt.id).await;

    let second = cleanup_attempts::claim(
        db.pool(),
        &cleanup_attempts::ClaimCleanupAttempt {
            obligation_id: input.obligation_id,
            lease_token: Uuid::new_v4(),
            lease_expires_at: Utc::now() + Duration::minutes(5),
            worker_run_id: None,
        },
    )
    .await
    .expect("claim retry cleanup attempt");
    assert_eq!(second.ordinal, 2);

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn execution_failed_attempt_is_immutable_and_retry_uses_a_new_ordinal() {
    let (mut db, _data_dir) = fixture().await;
    let project_path = "/fixture/cleanup-execution-failed-retry";
    let scope = frozen_scope(&db, project_path, "execution-failed-retry").await;
    let principal = operator_principals::current_local(db.pool())
        .await
        .expect("load trusted cleanup principal");
    let action_evidence = evidence(&db, scope, project_path, "execution-failed-action").await;
    let obligation_evidence =
        evidence(&db, scope, project_path, "execution-failed-obligation").await;
    let failure_evidence = evidence(&db, scope, project_path, "execution-failed-result").await;
    let input = command(scope, principal.id, action_evidence, obligation_evidence);
    cleanup_obligations::record_action_and_obligation(db.pool(), &input)
        .await
        .expect("record execution-failed cleanup obligation");

    let claimed = cleanup_attempts::claim(
        db.pool(),
        &cleanup_attempts::ClaimCleanupAttempt {
            obligation_id: input.obligation_id,
            lease_token: Uuid::new_v4(),
            lease_expires_at: Utc::now() + Duration::minutes(5),
            worker_run_id: None,
        },
    )
    .await
    .expect("claim execution-failed cleanup attempt");
    let failed = cleanup_attempts::transition(
        db.pool(),
        &cleanup_attempts::TransitionCleanupAttempt {
            attempt_id: claimed.id,
            lease_token: claimed.lease_token,
            expected_row_version: claimed.row_version,
            expected_status: "claimed".to_string(),
            next_status: "execution_failed".to_string(),
            result: Some(serde_json::json!({"error": "fixture failure"})),
            evidence: vec![(failure_evidence, "result".to_string())],
            terminal_note: Some("fixture execution failed before cleanup".to_string()),
        },
    )
    .await
    .expect("terminalize failed cleanup execution once");
    assert_eq!(failed.status, "execution_failed");
    assert_terminal_cleanup_attempt_is_immutable(db.pool(), failed.id).await;
    assert_eq!(
        cleanup_obligations::get(db.pool(), input.obligation_id)
            .await
            .expect("reload execution-failed obligation")
            .expect("execution-failed obligation exists")
            .status,
        "open"
    );

    let retry = cleanup_attempts::claim(
        db.pool(),
        &cleanup_attempts::ClaimCleanupAttempt {
            obligation_id: input.obligation_id,
            lease_token: Uuid::new_v4(),
            lease_expires_at: Utc::now() + Duration::minutes(5),
            worker_run_id: None,
        },
    )
    .await
    .expect("retry uses a new cleanup attempt row");
    assert_eq!(retry.ordinal, 2);
    assert_ne!(retry.id, failed.id);

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn verified_absence_emits_one_exact_replayable_cleanup_terminal_event() {
    let (mut db, _data_dir) = fixture().await;
    let project_path = "/fixture/cleanup-absence-terminal-event";
    let scope = frozen_scope(&db, project_path, "absence-terminal-event").await;
    let principal = operator_principals::current_local(db.pool())
        .await
        .expect("load trusted cleanup principal");
    let prepared = prepare_cleanup_attempt(
        &db,
        scope,
        project_path,
        "absence-terminal-event",
        principal.id,
    )
    .await;
    let absence_check_id = Uuid::new_v4();
    let check = cleanup_absence_checks::RecordAbsenceCheck {
        id: absence_check_id,
        cleanup_attempt_id: prepared.pending_verification.id,
        verifier_worker_run_id: Some(prepared.verifier_worker_run_id),
        verifier_key: "independent-db-lookup".to_string(),
        resource_identity_hash: prepared.obligation.resource_identity_hash.clone(),
        disposition: "absent".to_string(),
        evidence: vec![(prepared.absence_evidence_id, "absence".to_string())],
    };

    let applied = cleanup_absence_checks::record_and_apply(db.pool(), &check)
        .await
        .expect("terminalize cleanup with independent absence proof");
    assert_eq!(applied.attempt.status, "verified_absent");
    assert_eq!(applied.obligation.status, "verified_absent");
    assert_terminal_cleanup_attempt_is_immutable(db.pool(), applied.attempt.id).await;
    let source_version = applied.obligation.row_version;
    let event_id = cleanup_terminal_event_id(prepared.obligation.obligation_id, source_version);
    let event = sqlx::query_as::<_, CleanupTerminalEventRow>(
        r#"SELECT event_id,project_scope_id,organization_id_at_time,
                  source_operation_id,source_stream_key,source_version,
                  payload->'structured_payload' AS structured_payload,occurred_at
             FROM knowledge_outbox_events
            WHERE event_name='CleanupObligationTerminal.v1'
              AND source_kind='cleanup_obligation'
              AND source_id_kind='uuid'
              AND source_id_value=$1"#,
    )
    .bind(prepared.obligation.obligation_id.to_string())
    .fetch_one(db.pool())
    .await
    .expect("load exact cleanup-terminal event");
    assert_eq!(event.event_id, event_id);
    assert_eq!(event.project_scope_id, Some(scope.project_scope_id));
    assert_eq!(event.organization_id_at_time, Some(scope.organization_id));
    assert_eq!(event.source_operation_id, scope.operation_id);
    assert_eq!(
        event.source_stream_key,
        format!("cleanup-obligation:{}", prepared.obligation.obligation_id)
    );
    assert_eq!(event.source_version, source_version);
    assert_eq!(
        event.structured_payload,
        serde_json::json!({
            "obligation_id": prepared.obligation.obligation_id,
            "terminal_kind": "independent_absence",
            "terminal_status": "verified_absent",
            "resource_identity_hash": prepared.obligation.resource_identity_hash,
            "cleanup_attempt_id": prepared.pending_verification.id,
            "absence_check_id": absence_check_id,
            "evidence_ids": prepared.terminal_evidence_ids,
        })
    );
    assert_eq!(
        event.occurred_at.timestamp_micros(),
        applied
            .obligation
            .terminal_at
            .expect("terminal timestamp")
            .timestamp_micros()
    );

    let late_evidence = evidence(
        &db,
        scope,
        project_path,
        "absence-terminal-event-late-obligation-evidence",
    )
    .await;
    assert_terminal_evidence_insert_rejected(
        db.pool(),
        "INSERT INTO cleanup_obligation_evidence(obligation_id,evidence_id,role) \
         VALUES($1,$2,'support')",
        prepared.obligation.obligation_id,
        late_evidence,
    )
    .await;
    assert_terminal_evidence_insert_rejected(
        db.pool(),
        "INSERT INTO cleanup_attempt_evidence(attempt_id,evidence_id,role) \
         VALUES($1,$2,'support')",
        prepared.pending_verification.id,
        late_evidence,
    )
    .await;
    assert_terminal_evidence_insert_rejected(
        db.pool(),
        "INSERT INTO cleanup_absence_check_evidence(absence_check_id,evidence_id,role) \
         VALUES($1,$2,'support')",
        absence_check_id,
        late_evidence,
    )
    .await;

    let replayed = cleanup_absence_checks::record_and_apply(db.pool(), &check)
        .await
        .expect("exact absence response-loss replay");
    assert_eq!(replayed, applied);
    let counts: (i64, i64) = sqlx::query_as(
        r#"SELECT
               (SELECT COUNT(*) FROM knowledge_outbox_events WHERE event_id=$1),
               (SELECT COUNT(*) FROM knowledge_projection_deliveries WHERE event_id=$1)"#,
    )
    .bind(event_id)
    .fetch_one(db.pool())
    .await
    .expect("count replay-safe cleanup terminal outbox rows");
    assert_eq!(counts, (1, 4));

    let update_error =
        sqlx::query("UPDATE cleanup_obligations SET updated_at=updated_at WHERE id=$1")
            .bind(prepared.obligation.obligation_id)
            .execute(db.pool())
            .await
            .expect_err("terminal CleanupObligation source cannot bump its version");
    assert!(update_error
        .to_string()
        .contains("TERMINAL_CANONICAL_SOURCE_IMMUTABLE"));
    let delete_error = sqlx::query("DELETE FROM cleanup_obligations WHERE id=$1")
        .bind(prepared.obligation.obligation_id)
        .execute(db.pool())
        .await
        .expect_err("terminal CleanupObligation source cannot be deleted");
    assert!(delete_error
        .to_string()
        .contains("TERMINAL_CANONICAL_SOURCE_IMMUTABLE"));
    let retained_replay = cleanup_absence_checks::record_and_apply(db.pool(), &check)
        .await
        .expect("terminal Cleanup source remains exactly replayable after blocked mutations");
    assert_eq!(retained_replay, applied);
    let retained_counts: (i64, i64) = sqlx::query_as(
        r#"SELECT
               (SELECT COUNT(*) FROM knowledge_outbox_events WHERE event_id=$1),
               (SELECT COUNT(*) FROM knowledge_projection_deliveries WHERE event_id=$1)"#,
    )
    .bind(event_id)
    .fetch_one(db.pool())
    .await
    .expect("terminal Cleanup replay still owns one event and four deliveries");
    assert_eq!(retained_counts, (1, 4));

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn verified_absence_rolls_back_when_cleanup_terminal_outbox_insert_fails() {
    let (mut db, _data_dir) = fixture().await;
    let project_path = "/fixture/cleanup-absence-terminal-rollback";
    let scope = frozen_scope(&db, project_path, "absence-terminal-rollback").await;
    let principal = operator_principals::current_local(db.pool())
        .await
        .expect("load trusted cleanup principal");
    let prepared = prepare_cleanup_attempt(
        &db,
        scope,
        project_path,
        "absence-terminal-rollback",
        principal.id,
    )
    .await;
    install_cleanup_terminal_outbox_failure(&db).await;

    let error = cleanup_absence_checks::record_and_apply(
        db.pool(),
        &cleanup_absence_checks::RecordAbsenceCheck {
            id: Uuid::new_v4(),
            cleanup_attempt_id: prepared.pending_verification.id,
            verifier_worker_run_id: Some(prepared.verifier_worker_run_id),
            verifier_key: "independent-db-lookup".to_string(),
            resource_identity_hash: prepared.obligation.resource_identity_hash.clone(),
            disposition: "absent".to_string(),
            evidence: vec![(prepared.absence_evidence_id, "absence".to_string())],
        },
    )
    .await
    .expect_err("outbox failure must abort cleanup terminal truth");
    assert!(error
        .to_string()
        .contains("fixture cleanup terminal outbox failure"));
    let attempts =
        cleanup_attempts::list_for_obligation(db.pool(), prepared.obligation.obligation_id)
            .await
            .expect("reload rolled-back cleanup attempt");
    assert_eq!(attempts, vec![prepared.pending_verification]);
    let obligation = cleanup_obligations::get(db.pool(), prepared.obligation.obligation_id)
        .await
        .expect("reload rolled-back cleanup obligation")
        .expect("cleanup obligation remains");
    assert_eq!(obligation.status, "in_progress");
    assert!(obligation.terminal_at.is_none());
    let counts: (i64, i64) = sqlx::query_as(
        r#"SELECT
               (SELECT COUNT(*) FROM cleanup_absence_checks WHERE obligation_id=$1),
               (SELECT COUNT(*) FROM knowledge_outbox_events
                 WHERE event_name='CleanupObligationTerminal.v1'
                   AND source_id_value=$2)"#,
    )
    .bind(prepared.obligation.obligation_id)
    .bind(prepared.obligation.obligation_id.to_string())
    .fetch_one(db.pool())
    .await
    .expect("count rolled-back cleanup terminal rows");
    assert_eq!(counts, (0, 0));

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn waiver_requires_the_server_principal_exact_cas_and_residual_evidence() {
    let (mut db, _data_dir) = fixture().await;
    let project_path = "/fixture/cleanup-waiver";
    let scope = frozen_scope(&db, project_path, "waiver").await;
    let principal = operator_principals::current_local(db.pool())
        .await
        .expect("load trusted cleanup principal");
    let action_evidence = evidence(&db, scope, project_path, "waiver-action").await;
    let obligation_evidence = evidence(&db, scope, project_path, "waiver-obligation").await;
    let waiver_evidence = evidence(&db, scope, project_path, "waiver-decision").await;
    let input = command(scope, principal.id, action_evidence, obligation_evidence);
    let created = cleanup_obligations::record_action_and_obligation(db.pool(), &input)
        .await
        .expect("record waiver fixture obligation");
    let waiver = cleanup_waivers::WaiveCleanupObligation {
        id: Uuid::new_v4(),
        obligation_id: input.obligation_id,
        operation_id: scope.operation_id,
        project_scope_id: scope.project_scope_id,
        scope_snapshot_id: scope.snapshot_id,
        organization_id_at_time: scope.organization_id,
        expected_obligation_row_version: created.obligation.row_version,
        principal_id: principal.id,
        reason: "target owner retains the account under documented control".to_string(),
        residual_risk: serde_json::json!({"summary": "owner retained account", "severity": "medium"}),
        evidence: vec![(waiver_evidence, "decision".to_string())],
    };
    let mut forged = waiver.clone();
    forged.principal_id = Uuid::new_v4();
    assert!(
        cleanup_waivers::waive(db.pool(), &forged).await.is_err(),
        "request-selected actor cannot authorize a waiver"
    );
    let mut cross_scope = waiver.clone();
    cross_scope.organization_id_at_time = Uuid::new_v4();
    assert!(
        cleanup_waivers::waive(db.pool(), &cross_scope)
            .await
            .is_err(),
        "an obligation UUID cannot authorize a waiver in a different frozen organization scope"
    );
    let accepted = cleanup_waivers::waive(db.pool(), &waiver)
        .await
        .expect("trusted operator waives cleanup with residual risk");
    assert_eq!(accepted.obligation.status, "waived_by_user");
    let terminal_event_id =
        cleanup_terminal_event_id(accepted.obligation.id, accepted.obligation.row_version);
    let terminal_event: (Uuid, i64, serde_json::Value, chrono::DateTime<Utc>) = sqlx::query_as(
        r#"SELECT event_id,source_version,payload->'structured_payload',occurred_at
                 FROM knowledge_outbox_events
                WHERE event_name='CleanupObligationTerminal.v1'
                  AND project_scope_id=$1 AND organization_id_at_time=$2
                  AND source_operation_id=$3 AND source_kind='cleanup_obligation'
                  AND source_id_kind='uuid' AND source_id_value=$4
                  AND source_stream_key=$5"#,
    )
    .bind(scope.project_scope_id)
    .bind(scope.organization_id)
    .bind(scope.operation_id)
    .bind(input.obligation_id.to_string())
    .bind(format!("cleanup-obligation:{}", input.obligation_id))
    .fetch_one(db.pool())
    .await
    .expect("load exact waiver terminal memory event");
    assert_eq!(terminal_event.0, terminal_event_id);
    assert_eq!(terminal_event.1, accepted.obligation.row_version);
    assert_eq!(
        terminal_event.2,
        serde_json::json!({
            "obligation_id": input.obligation_id,
            "terminal_kind": "operator_waiver",
            "terminal_status": "waived_by_user",
            "resource_identity_hash": input.resource_identity_hash,
            "waiver_id": waiver.id,
            "residual_risk": waiver.residual_risk,
            "evidence_ids": [obligation_evidence, waiver_evidence],
        })
    );
    assert_eq!(
        terminal_event.3.timestamp_micros(),
        accepted
            .obligation
            .terminal_at
            .expect("waiver terminal timestamp")
            .timestamp_micros()
    );
    let late_evidence = evidence(&db, scope, project_path, "waiver-late-evidence").await;
    assert_terminal_evidence_insert_rejected(
        db.pool(),
        "INSERT INTO cleanup_obligation_evidence(obligation_id,evidence_id,role) \
         VALUES($1,$2,'support')",
        input.obligation_id,
        late_evidence,
    )
    .await;
    assert_terminal_evidence_insert_rejected(
        db.pool(),
        "INSERT INTO cleanup_waiver_evidence(waiver_id,evidence_id,role) \
         VALUES($1,$2,'support')",
        waiver.id,
        late_evidence,
    )
    .await;
    let closeout = organization_deletion_jobs::cleanup_closeout_gate(
        db.pool(),
        scope.operation_id,
        scope.organization_id,
    )
    .await
    .expect("load Cleanup-owned closeout truth");
    assert!(closeout.allows_closeout());
    assert_eq!(
        closeout.residual_obligation_ids,
        [input.obligation_id].into_iter().collect()
    );
    assert_eq!(
        cleanup_waivers::waive(db.pool(), &waiver)
            .await
            .expect("exact waiver response-loss replay"),
        accepted
    );
    let terminal_counts: (i64, i64) = sqlx::query_as(
        r#"SELECT
               (SELECT COUNT(*) FROM knowledge_outbox_events WHERE event_id=$1),
               (SELECT COUNT(*) FROM knowledge_projection_deliveries WHERE event_id=$1)"#,
    )
    .bind(terminal_event_id)
    .fetch_one(db.pool())
    .await
    .expect("count exact waiver terminal event and deliveries");
    assert_eq!(terminal_counts, (1, 4));
    let mut drifted = waiver;
    drifted.reason = "different reason".to_string();
    assert!(
        cleanup_waivers::waive(db.pool(), &drifted).await.is_err(),
        "terminal waiver history is immutable"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn blocked_terminal_freezes_evidence_after_exact_same_transaction_closeout() {
    let (mut db, _data_dir) = fixture().await;
    let project_path = "/fixture/cleanup-blocked-evidence-freeze";
    let scope = frozen_scope(&db, project_path, "blocked-evidence-freeze").await;
    let principal = operator_principals::current_local(db.pool())
        .await
        .expect("load trusted cleanup principal");
    let action_evidence = evidence(&db, scope, project_path, "blocked-action").await;
    let obligation_evidence = evidence(&db, scope, project_path, "blocked-obligation").await;
    let decision_evidence = evidence(&db, scope, project_path, "blocked-decision").await;
    let input = command(scope, principal.id, action_evidence, obligation_evidence);
    cleanup_obligations::record_action_and_obligation(db.pool(), &input)
        .await
        .expect("record blocked cleanup obligation");

    let decision_id = Uuid::new_v4();
    let residual_risk = serde_json::json!({
        "summary": "cleanup cannot be completed within the authorized window",
        "severity": "high"
    });
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin exact blocked closeout");
    sqlx::query(
        r#"INSERT INTO cleanup_blocked_decisions(
               id,obligation_id,operation_id,project_scope_id,scope_snapshot_id,
               organization_id_at_time,decided_by_principal_id,reason,residual_risk
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)"#,
    )
    .bind(decision_id)
    .bind(input.obligation_id)
    .bind(scope.operation_id)
    .bind(scope.project_scope_id)
    .bind(scope.snapshot_id)
    .bind(scope.organization_id)
    .bind(principal.id)
    .bind("authorized cleanup window expired")
    .bind(&residual_risk)
    .execute(&mut *tx)
    .await
    .expect("insert retained blocked decision");
    sqlx::query(
        "INSERT INTO cleanup_blocked_decision_evidence( \
             blocked_decision_id,evidence_id,role \
         ) VALUES($1,$2,'decision')",
    )
    .bind(decision_id)
    .bind(decision_evidence)
    .execute(&mut *tx)
    .await
    .expect("insert blocked decision evidence before terminal transition");
    sqlx::query(
        r#"UPDATE cleanup_obligations
              SET status='blocked',residual_risk=$2,terminal_at=NOW(),
                  row_version=row_version+1,updated_at=NOW()
            WHERE id=$1"#,
    )
    .bind(input.obligation_id)
    .bind(&residual_risk)
    .execute(&mut *tx)
    .await
    .expect("transition exact blocked terminal truth in the same transaction");
    sqlx::query("SET CONSTRAINTS ALL IMMEDIATE")
        .execute(&mut *tx)
        .await
        .expect("exact blocked relational truth satisfies the deferred boundary");
    tx.commit().await.expect("commit exact blocked closeout");

    let (status, stored_residual): (String, Option<serde_json::Value>) =
        sqlx::query_as("SELECT status,residual_risk FROM cleanup_obligations WHERE id=$1")
            .bind(input.obligation_id)
            .fetch_one(db.pool())
            .await
            .expect("load exact blocked terminal obligation");
    assert_eq!(status, "blocked");
    assert_eq!(stored_residual.as_ref(), Some(&residual_risk));
    let closeout = organization_deletion_jobs::cleanup_closeout_gate(
        db.pool(),
        scope.operation_id,
        scope.organization_id,
    )
    .await
    .expect("read exact blocked Cleanup closeout truth");
    assert!(closeout.allows_closeout());
    assert_eq!(
        closeout.residual_obligation_ids,
        [input.obligation_id].into_iter().collect()
    );

    let late_evidence = evidence(&db, scope, project_path, "blocked-late-evidence").await;
    assert_terminal_evidence_insert_rejected(
        db.pool(),
        "INSERT INTO cleanup_obligation_evidence(obligation_id,evidence_id,role) \
         VALUES($1,$2,'support')",
        input.obligation_id,
        late_evidence,
    )
    .await;
    assert_terminal_evidence_insert_rejected(
        db.pool(),
        "INSERT INTO cleanup_blocked_decision_evidence( \
             blocked_decision_id,evidence_id,role \
         ) VALUES($1,$2,'support')",
        decision_id,
        late_evidence,
    )
    .await;

    for statement in [
        "UPDATE cleanup_blocked_decision_evidence SET role=role \
         WHERE blocked_decision_id=$1 AND evidence_id=$2",
        "DELETE FROM cleanup_blocked_decision_evidence \
         WHERE blocked_decision_id=$1 AND evidence_id=$2",
    ] {
        let error = sqlx::query(statement)
            .bind(decision_id)
            .bind(decision_evidence)
            .execute(db.pool())
            .await
            .expect_err("terminal blocked decision evidence is retained immutable history");
        assert!(error
            .to_string()
            .contains("cleanup fact history is immutable"));
    }

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn waiver_rolls_back_when_cleanup_terminal_outbox_insert_fails() {
    let (mut db, _data_dir) = fixture().await;
    let project_path = "/fixture/cleanup-waiver-terminal-rollback";
    let scope = frozen_scope(&db, project_path, "waiver-terminal-rollback").await;
    let principal = operator_principals::current_local(db.pool())
        .await
        .expect("load trusted cleanup principal");
    let action_evidence = evidence(&db, scope, project_path, "waiver-rollback-action").await;
    let obligation_evidence =
        evidence(&db, scope, project_path, "waiver-rollback-obligation").await;
    let waiver_evidence = evidence(&db, scope, project_path, "waiver-rollback-decision").await;
    let input = command(scope, principal.id, action_evidence, obligation_evidence);
    let created = cleanup_obligations::record_action_and_obligation(db.pool(), &input)
        .await
        .expect("record waiver rollback fixture obligation");
    install_cleanup_terminal_outbox_failure(&db).await;
    let request = cleanup_waivers::WaiveCleanupObligation {
        id: Uuid::new_v4(),
        obligation_id: input.obligation_id,
        operation_id: scope.operation_id,
        project_scope_id: scope.project_scope_id,
        scope_snapshot_id: scope.snapshot_id,
        organization_id_at_time: scope.organization_id,
        expected_obligation_row_version: created.obligation.row_version,
        principal_id: principal.id,
        reason: "documented retained account".to_string(),
        residual_risk: serde_json::json!({"summary": "retained", "severity": "medium"}),
        evidence: vec![(waiver_evidence, "decision".to_string())],
    };

    let error = cleanup_waivers::waive(db.pool(), &request)
        .await
        .expect_err("outbox failure must abort waiver terminal truth");
    assert!(error
        .to_string()
        .contains("fixture cleanup terminal outbox failure"));
    let obligation = cleanup_obligations::get(db.pool(), input.obligation_id)
        .await
        .expect("reload rolled-back waiver obligation")
        .expect("waiver obligation remains");
    assert_eq!(obligation, created.obligation);
    let counts: (i64, i64) = sqlx::query_as(
        r#"SELECT
               (SELECT COUNT(*) FROM cleanup_waivers WHERE obligation_id=$1),
               (SELECT COUNT(*) FROM knowledge_outbox_events
                 WHERE event_name='CleanupObligationTerminal.v1'
                   AND source_id_value=$2)"#,
    )
    .bind(input.obligation_id)
    .bind(input.obligation_id.to_string())
    .fetch_one(db.pool())
    .await
    .expect("count rolled-back waiver terminal rows");
    assert_eq!(counts, (0, 0));

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn cleanup_terminal_state_requires_exact_relational_truth_at_deferred_boundary() {
    let (mut db, _data_dir) = fixture().await;
    let project_path = "/fixture/cleanup-terminal-truth";
    let scope = frozen_scope(&db, project_path, "terminal-truth").await;
    let principal = operator_principals::current_local(db.pool())
        .await
        .expect("load trusted cleanup principal");
    let action_evidence = evidence(&db, scope, project_path, "terminal-action").await;
    let obligation_evidence = evidence(&db, scope, project_path, "terminal-obligation").await;
    let input = command(scope, principal.id, action_evidence, obligation_evidence);
    cleanup_obligations::record_action_and_obligation(db.pool(), &input)
        .await
        .expect("record cleanup obligation");

    let mut forged = db
        .pool()
        .begin()
        .await
        .expect("begin forged terminal update");
    sqlx::query(
        r#"UPDATE cleanup_obligations
              SET status='verified_absent',terminal_at=NOW(),row_version=row_version+1
            WHERE id=$1"#,
    )
    .bind(input.obligation_id)
    .execute(&mut *forged)
    .await
    .expect("deferred terminal truth is checked at the constraint boundary");
    assert!(
        sqlx::query("SET CONSTRAINTS ALL IMMEDIATE")
            .execute(&mut *forged)
            .await
            .is_err(),
        "verified_absent without an exact terminal attempt, independent absence check, and absence evidence must fail closed"
    );
    forged.rollback().await.ok();

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn gate_and_deletion_precheck_reject_legacy_invalid_terminal_truth() {
    let (mut db, _data_dir) = fixture().await;
    let project_path = "/fixture/cleanup-legacy-invalid-terminal";
    let scope = frozen_scope(&db, project_path, "legacy-invalid-terminal").await;
    let principal = operator_principals::current_local(db.pool())
        .await
        .expect("load trusted cleanup principal");
    let action_evidence = evidence(&db, scope, project_path, "legacy-action").await;
    let obligation_evidence = evidence(&db, scope, project_path, "legacy-obligation").await;
    let input = command(scope, principal.id, action_evidence, obligation_evidence);
    cleanup_obligations::record_action_and_obligation(db.pool(), &input)
        .await
        .expect("record cleanup obligation");

    // Simulate a legacy/replica-imported row that pre-dates the deferred
    // relational constraint. Gate and deletion must still re-read exact truth.
    sqlx::query("ALTER TABLE cleanup_obligations DISABLE TRIGGER USER")
        .execute(db.pool())
        .await
        .expect("disable user triggers for legacy corruption fixture");
    sqlx::query(
        r#"UPDATE cleanup_obligations
              SET status='verified_absent',terminal_at=NOW(),row_version=row_version+1
            WHERE id=$1"#,
    )
    .bind(input.obligation_id)
    .execute(db.pool())
    .await
    .expect("seed legacy invalid terminal row");
    sqlx::query("ALTER TABLE cleanup_obligations ENABLE TRIGGER USER")
        .execute(db.pool())
        .await
        .expect("restore cleanup obligation triggers");

    let gate = organization_deletion_jobs::cleanup_closeout_gate(
        db.pool(),
        scope.operation_id,
        scope.organization_id,
    )
    .await
    .expect("read cleanup closeout truth");
    assert!(
        !gate.allows_closeout(),
        "a terminal label without exact relational truth cannot pass Cleanup"
    );

    let delete_error = organization_deletion_jobs::request(
        db.pool(),
        &organization_deletion_jobs::RequestOrganizationDeletion {
            job_id: Uuid::new_v4(),
            root_organization_id: scope.organization_id,
            principal_id: principal.id,
            expected_project_path: project_path.to_string(),
        },
    )
    .await
    .expect_err("organization deletion must independently reject forged terminal truth");
    assert!(delete_error
        .to_string()
        .contains("organization_delete_cleanup_terminal_truth_invalid"));

    db.stop().await;
}

async fn seed_deletion_waiter(
    db: &GolishDb,
    principal_id: Uuid,
    requested_at: chrono::DateTime<Utc>,
    delivery_status: &str,
) -> (Uuid, Uuid) {
    let job_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let operation_id = Uuid::new_v4();
    let source_id = Uuid::new_v4();
    let stream_key = format!("cleanup-obligation:{source_id}");
    let project_path = format!("/fixture/deletion-{job_id}");
    let project_scope = project_scopes::register_first_open(
        db.pool(),
        &project_path,
        &format!("deletion-{job_id}-path-sha"),
    )
    .await
    .expect("register deletion waiter project scope");
    sqlx::query(
        r#"INSERT INTO organization_deletion_jobs(
               id,root_organization_id_at_time,project_scope_id,project_path_at_time,
               requested_by_principal_id,state,organization_snapshot,target_snapshot,
               required_invalidation_count,requested_at
           ) VALUES($1,$2,$3,$4,$5,'waiting_for_invalidation_delivery',$6,'[]'::jsonb,1,$7)"#,
    )
    .bind(job_id)
    .bind(organization_id)
    .bind(project_scope.project_scope_id)
    .bind(&project_path)
    .bind(principal_id)
    .bind(serde_json::json!([{
        "organizationIdAtTime": organization_id,
        "projectPathAtTime": project_path
    }]))
    .bind(requested_at)
    .execute(db.pool())
    .await
    .expect("insert deletion waiter");
    sqlx::query(
        r#"INSERT INTO knowledge_outbox_events(
               event_id,event_name,schema_version,project_scope_id,
               organization_id_at_time,source_operation_id,source_kind,
               source_id_kind,source_id_value,source_stream_key,source_version,
               payload,occurred_at,dedupe_key
           ) VALUES(
               $1,'SourceScopeInvalidated.v1',1,NULL,$2,$3,'cleanup_obligation',
               'uuid',$4,$5,1,$6,$7,$8
           )"#,
    )
    .bind(event_id)
    .bind(organization_id)
    .bind(operation_id)
    .bind(source_id.to_string())
    .bind(&stream_key)
    .bind(serde_json::json!({"source": source_id}))
    .bind(requested_at)
    .bind(format!("deletion-test:{event_id}"))
    .execute(db.pool())
    .await
    .expect("insert deletion invalidation event");
    sqlx::query(
        r#"INSERT INTO knowledge_projection_deliveries(
               event_id,projector_name,projector_schema_version,status,completed_at
           ) VALUES($1,'assertion-promoter',1,$2,
               CASE WHEN $2='succeeded' THEN NOW() ELSE NULL END)"#,
    )
    .bind(event_id)
    .bind(delivery_status)
    .execute(db.pool())
    .await
    .expect("insert deletion invalidation delivery");
    sqlx::query(
        r#"INSERT INTO organization_deletion_job_invalidations(
               job_id,event_id,source_stream_key,source_version,
               required_delivery_manifest
           ) VALUES($1,$2,$3,1,$4)"#,
    )
    .bind(job_id)
    .bind(event_id)
    .bind(stream_key)
    .bind(serde_json::json!([{
        "projector_name": "assertion-promoter",
        "projector_schema_version": 1
    }]))
    .execute(db.pool())
    .await
    .expect("insert deletion invalidation manifest");
    (job_id, event_id)
}

#[tokio::test]
#[serial]
async fn deletion_claim_skips_older_unready_job_and_uses_frozen_delivery_manifest() {
    let (mut db, _data_dir) = fixture().await;
    let principal = operator_principals::current_local(db.pool())
        .await
        .expect("load trusted deletion principal");
    let (older_unready, _) = seed_deletion_waiter(
        &db,
        principal.id,
        Utc::now() - Duration::minutes(2),
        "pending",
    )
    .await;
    let (newer_ready, newer_ready_event) = seed_deletion_waiter(
        &db,
        principal.id,
        Utc::now() - Duration::minutes(1),
        "succeeded",
    )
    .await;
    sqlx::query(
        r#"INSERT INTO knowledge_projector_registry(
               projector_name,projector_schema_version,lifecycle
           ) VALUES('future-projector-not-in-frozen-manifest',999,'enabled')"#,
    )
    .execute(db.pool())
    .await
    .expect("register projector outside the frozen manifest");
    sqlx::query(
        r#"INSERT INTO knowledge_projection_deliveries(
               event_id,projector_name,projector_schema_version,status
           ) VALUES($1,'future-projector-not-in-frozen-manifest',999,'pending')"#,
    )
    .bind(newer_ready_event)
    .execute(db.pool())
    .await
    .expect("insert delivery outside the frozen manifest");

    let (claimed, plan) = organization_deletion_jobs::claim_next_artifact_cleanup(
        db.pool(),
        &organization_deletion_jobs::ClaimOrganizationArtifactCleanup {
            worker_id: "cleanup-test-worker".to_string(),
            lease_seconds: 60,
        },
    )
    .await
    .expect("claim ready deletion job")
    .expect("a later ready job must not be starved");

    assert_eq!(claimed.id, newer_ready);
    assert_ne!(claimed.id, older_unready);
    assert_eq!(plan.job_id, newer_ready);
    assert!(plan.targets.is_empty());
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn deletion_claim_orders_ready_retries_and_new_waiters_without_starvation() {
    let (mut db, _data_dir) = fixture().await;
    let principal = operator_principals::current_local(db.pool())
        .await
        .expect("load trusted deletion principal");
    let (older_retry, _) = seed_deletion_waiter(
        &db,
        principal.id,
        Utc::now() - Duration::minutes(2),
        "succeeded",
    )
    .await;
    sqlx::query(
        "UPDATE organization_deletion_jobs SET state='pending_artifact_cleanup' WHERE id=$1",
    )
    .bind(older_retry)
    .execute(db.pool())
    .await
    .expect("make the older ready job an expired retry");
    let (newer_waiter, _) = seed_deletion_waiter(
        &db,
        principal.id,
        Utc::now() - Duration::minutes(1),
        "succeeded",
    )
    .await;

    let (claimed, _) = organization_deletion_jobs::claim_next_artifact_cleanup(
        db.pool(),
        &organization_deletion_jobs::ClaimOrganizationArtifactCleanup {
            worker_id: "cleanup-test-worker".to_string(),
            lease_seconds: 60,
        },
    )
    .await
    .expect("claim oldest ready deletion job")
    .expect("a ready deletion job exists");

    assert_eq!(claimed.id, older_retry);
    assert_ne!(claimed.id, newer_waiter);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn sitemap_compare_and_swap_rejects_a_stale_prune_without_losing_concurrent_data() {
    let (mut db, _data_dir) = fixture().await;
    let project_path = "/fixture/sitemap-cas";
    let original = serde_json::json!({
        "entries": {
            "GET:https://remove.example.test/": {
                "url": "https://remove.example.test/"
            }
        }
    });
    sitemap_store::upsert_zap_sitemap(db.pool(), Some(project_path), &original)
        .await
        .expect("seed sitemap");
    let stale = sitemap_store::read_zap_sitemap(db.pool(), Some(project_path))
        .await
        .expect("read sitemap for stale prune")
        .expect("stored sitemap");
    let concurrently_extended = serde_json::json!({
        "entries": {
            "GET:https://remove.example.test/": {
                "url": "https://remove.example.test/"
            },
            "GET:https://keep.example.test/new": {
                "url": "https://keep.example.test/new"
            }
        }
    });
    sitemap_store::upsert_zap_sitemap(db.pool(), Some(project_path), &concurrently_extended)
        .await
        .expect("commit concurrent sitemap extension");
    let stale_pruned = serde_json::json!({"entries": {}});

    assert!(
        !sitemap_store::compare_and_swap_zap_sitemap(
            db.pool(),
            project_path,
            &stale,
            Some(&stale_pruned),
        )
        .await
        .expect("attempt stale sitemap CAS"),
        "a stale prune must retry instead of replacing a concurrently extended sitemap"
    );
    assert_eq!(
        sitemap_store::read_zap_sitemap(db.pool(), Some(project_path))
            .await
            .expect("read sitemap after rejected stale prune"),
        Some(concurrently_extended)
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn deletion_request_freezes_targets_before_external_cleanup_and_hard_delete() {
    let (mut db, _data_dir) = fixture().await;
    let principal = operator_principals::current_local(db.pool())
        .await
        .expect("load trusted deletion principal");
    let job_id = Uuid::new_v4();
    let project_path = format!("/fixture/two-phase-delete-{job_id}");
    let scope = frozen_scope(&db, &project_path, "two-phase-delete").await;
    let organization_id = scope.organization_id;
    let child_organization_id = Uuid::new_v4();
    let target_id = Uuid::new_v4();
    let external_organization_id = Uuid::new_v4();
    sqlx::query("INSERT INTO organizations(id,project_path,name,parent_id) VALUES($1,$2,$3,$4)")
        .bind(child_organization_id)
        .bind(&project_path)
        .bind("two phase child")
        .bind(organization_id)
        .execute(db.pool())
        .await
        .expect("insert deletion child organization");
    sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$2,$3)")
        .bind(external_organization_id)
        .bind(&project_path)
        .bind("external organization")
        .execute(db.pool())
        .await
        .expect("insert organization outside deletion subtree");
    sqlx::query(
        r#"INSERT INTO targets(
               id,name,target_type,value,scope,project_path,organization_id
           ) VALUES($1,'delete target','url','https://delete.example.test','in',$2,$3)"#,
    )
    .bind(target_id)
    .bind(&project_path)
    .bind(organization_id)
    .execute(db.pool())
    .await
    .expect("insert deletion target");

    let web_origin_id = Uuid::new_v4();
    let fingerprint_id = Uuid::new_v4();
    let endpoint_id = Uuid::new_v4();
    let fingerprint_observation_id = Uuid::new_v4();
    let endpoint_observation_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO web_origins(
               id,organization_id,project_path,scheme,host,host_type,port,origin,source
           ) VALUES(
               $1,$2,$3,'https','delete.example.test','domain',443,
               'https://delete.example.test:443','cleanup-test'
           )"#,
    )
    .bind(web_origin_id)
    .bind(organization_id)
    .bind(&project_path)
    .execute(db.pool())
    .await
    .expect("insert deletion manifest origin");
    sqlx::query(
        r#"INSERT INTO web_origin_observations(
               organization_id,project_path,web_origin_id,target_id,status_code,source
           ) VALUES($1,$2,$3,$4,200,'cleanup-test')"#,
    )
    .bind(organization_id)
    .bind(&project_path)
    .bind(web_origin_id)
    .bind(target_id)
    .execute(db.pool())
    .await
    .expect("link deletion Target to its exact origin");
    sqlx::query(
        r#"INSERT INTO fingerprints(
               id,target_id,project_path,category,name,version,source
           ) VALUES($1,$2,$3,'technology','Delete Fixture','1.0','cleanup-test')"#,
    )
    .bind(fingerprint_id)
    .bind(target_id)
    .bind(&project_path)
    .execute(db.pool())
    .await
    .expect("insert deletion fingerprint");
    sqlx::query(
        r#"INSERT INTO api_endpoints(
               id,target_id,project_path,url,method,path,source
           ) VALUES(
               $1,$2,$3,'https://delete.example.test:443/api/search?term=fixture',
               'GET','/api/search','cleanup-test'
           )"#,
    )
    .bind(endpoint_id)
    .bind(target_id)
    .bind(&project_path)
    .execute(db.pool())
    .await
    .expect("insert deletion endpoint");
    sqlx::query(
        r#"INSERT INTO fingerprint_origin_observations(
               id,fingerprint_id,web_origin_id,organization_id,target_id,project_path,source
           ) VALUES($1,$2,$3,$4,$5,$6,'cleanup-test')"#,
    )
    .bind(fingerprint_observation_id)
    .bind(fingerprint_id)
    .bind(web_origin_id)
    .bind(organization_id)
    .bind(target_id)
    .bind(&project_path)
    .execute(db.pool())
    .await
    .expect("publish deletion fingerprint-origin observation");
    sqlx::query(
        r#"INSERT INTO enumeration_endpoint_observations(
               id,operation_id,organization_id,target_id,web_origin_id,
               endpoint_id,project_path,source
           ) VALUES($1,$2,$3,$4,$5,$6,$7,'cleanup-test')"#,
    )
    .bind(endpoint_observation_id)
    .bind(scope.operation_id)
    .bind(organization_id)
    .bind(target_id)
    .bind(web_origin_id)
    .bind(endpoint_id)
    .bind(&project_path)
    .execute(db.pool())
    .await
    .expect("publish deletion endpoint observation");
    sqlx::query(
        r#"INSERT INTO enumeration_endpoint_parameters(
               endpoint_observation_id,name,location,value_type,source
           ) VALUES($1,'term','query','string','cleanup-test')"#,
    )
    .bind(endpoint_observation_id)
    .execute(db.pool())
    .await
    .expect("publish deletion endpoint parameter");

    let requested = organization_deletion_jobs::request(
        db.pool(),
        &organization_deletion_jobs::RequestOrganizationDeletion {
            job_id,
            root_organization_id: organization_id,
            principal_id: principal.id,
            expected_project_path: project_path.clone(),
        },
    )
    .await
    .expect("commit deletion precheck and frozen snapshots");
    assert_eq!(requested.state, "waiting_for_invalidation_delivery");
    let overlapping_child = organization_deletion_jobs::request(
        db.pool(),
        &organization_deletion_jobs::RequestOrganizationDeletion {
            job_id: Uuid::new_v4(),
            root_organization_id: child_organization_id,
            principal_id: principal.id,
            expected_project_path: project_path.clone(),
        },
    )
    .await
    .expect_err("an active parent deletion owns the whole frozen subtree");
    assert!(overlapping_child
        .to_string()
        .contains("organization_delete_subtree_already_deleting"));

    let reparent_drift =
        sqlx::query("UPDATE organizations SET parent_id=$1,updated_at=NOW() WHERE id=$2")
            .bind(organization_id)
            .bind(external_organization_id)
            .execute(db.pool())
            .await
            .expect_err("an external organization cannot be moved into an active deletion subtree");
    assert_eq!(
        reparent_drift
            .as_database_error()
            .and_then(|error| error.code())
            .map(|code| code.into_owned())
            .as_deref(),
        Some("55000")
    );

    let target_drift = sqlx::query(
        r#"INSERT INTO targets(
               id,name,target_type,value,scope,project_path,organization_id
           ) VALUES($1,'late target','url','https://late.example.test','in',$2,$3)"#,
    )
    .bind(Uuid::new_v4())
    .bind(&project_path)
    .bind(organization_id)
    .execute(db.pool())
    .await
    .expect_err("deleting subtree must reject target drift");
    assert_eq!(
        target_drift
            .as_database_error()
            .and_then(|error| error.code())
            .map(|code| code.into_owned())
            .as_deref(),
        Some("55000")
    );

    let worker_id = "cleanup-two-phase-test-worker".to_string();
    let (claimed, plan) = organization_deletion_jobs::claim_next_artifact_cleanup(
        db.pool(),
        &organization_deletion_jobs::ClaimOrganizationArtifactCleanup {
            worker_id: worker_id.clone(),
            lease_seconds: 60,
        },
    )
    .await
    .expect("claim committed deletion job")
    .expect("deletion job is ready without source invalidations");
    assert_eq!(plan.targets.len(), 1);
    assert_eq!(plan.targets[0].target_id_at_time, target_id);
    let completed = organization_deletion_jobs::complete_artifact_cleanup(
        db.pool(),
        &organization_deletion_jobs::CompleteOrganizationArtifactCleanup {
            job_id,
            worker_id,
            lease_token: claimed.lease_token.expect("fenced deletion claim"),
            expected_row_version: claimed.row_version,
            result: Ok(()),
        },
    )
    .await
    .expect("persist successful external cleanup result");
    assert_eq!(completed.state, "artifact_cleanup_succeeded");
    let committed = organization_deletion_jobs::hard_delete(db.pool(), job_id)
        .await
        .expect("commit independent hard delete");
    assert_eq!(committed.state, "hard_delete_committed");
    let live_orgs: (bool, bool, bool) = sqlx::query_as(
        r#"SELECT
               EXISTS(SELECT 1 FROM organizations WHERE id=$1),
               EXISTS(SELECT 1 FROM organizations WHERE id=$2),
               EXISTS(SELECT 1 FROM organizations WHERE id=$3)"#,
    )
    .bind(organization_id)
    .bind(child_organization_id)
    .bind(external_organization_id)
    .fetch_one(db.pool())
    .await
    .expect("read organization identities after hard delete");
    assert!(!live_orgs.0);
    assert!(!live_orgs.1);
    assert!(
        live_orgs.2,
        "organization outside the frozen subtree remains"
    );
    let live_target: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM targets WHERE id=$1)")
        .bind(target_id)
        .fetch_one(db.pool())
        .await
        .expect("read Target after hard delete");
    assert!(!live_target);
    let manifest_rows: (i64, i64) = sqlx::query_as(
        r#"SELECT
               (SELECT COUNT(*) FROM fingerprint_origin_observations WHERE id=$1),
               (SELECT COUNT(*) FROM enumeration_endpoint_observations WHERE id=$2)"#,
    )
    .bind(fingerprint_observation_id)
    .bind(endpoint_observation_id)
    .fetch_one(db.pool())
    .await
    .expect("count deleted manifest observations");
    assert_eq!(manifest_rows, (0, 0));
    let retained_target_snapshot: (Uuid, Option<Uuid>) = sqlx::query_as(
        r#"SELECT target_id_at_time,live_target_id
             FROM organization_deletion_job_targets
            WHERE job_id=$1 AND target_id_at_time=$2"#,
    )
    .bind(job_id)
    .bind(target_id)
    .fetch_one(db.pool())
    .await
    .expect("load retained deletion Target snapshot");
    assert_eq!(retained_target_snapshot, (target_id, None));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn deletion_request_requires_the_matching_active_server_project_scope() {
    let (mut db, _data_dir) = fixture().await;
    let principal = operator_principals::current_local(db.pool())
        .await
        .expect("load trusted deletion principal");
    let actual_path = format!("/fixture/delete-idor-actual-{}", Uuid::new_v4());
    let scope = frozen_scope(&db, &actual_path, "delete-idor-actual").await;
    let foreign_path = format!("/fixture/delete-idor-foreign-{}", Uuid::new_v4());
    project_scopes::register_first_open(
        db.pool(),
        &foreign_path,
        &format!("delete-idor-foreign-{}-path-sha", Uuid::new_v4()),
    )
    .await
    .expect("register another active server project scope");

    for expected_project_path in [
        foreign_path,
        format!("/fixture/delete-idor-unregistered-{}", Uuid::new_v4()),
    ] {
        let error = organization_deletion_jobs::request(
            db.pool(),
            &organization_deletion_jobs::RequestOrganizationDeletion {
                job_id: Uuid::new_v4(),
                root_organization_id: scope.organization_id,
                principal_id: principal.id,
                expected_project_path,
            },
        )
        .await
        .expect_err("foreign or unregistered workspace cannot authorize deletion");
        assert!(error
            .to_string()
            .contains("organization_delete_project_scope_not_authorized"));
    }
    assert!(
        organization_deletion_jobs::list_active(db.pool())
            .await
            .expect("list deletion jobs after denied requests")
            .is_empty(),
        "an IDOR attempt must not create any durable deletion job"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn deletion_artifact_plan_uses_only_the_registered_project_scope_path() {
    let (mut db, _data_dir) = fixture().await;
    let project_path = format!("/fixture/canonical-delete-root-{}", Uuid::new_v4());
    let scope = frozen_scope(&db, &project_path, "canonical-delete-root").await;
    let principal = operator_principals::current_local(db.pool())
        .await
        .expect("load trusted deletion principal");
    let target_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO targets(
               id,name,target_type,value,scope,project_path,organization_id
           ) VALUES($1,'forged path target','url','https://scoped.example.test','in',$2,$3)"#,
    )
    .bind(target_id)
    .bind("/tmp/caller-controlled-artifact-root")
    .bind(scope.organization_id)
    .execute(db.pool())
    .await
    .expect("insert target with historical/caller-controlled project path");

    let requested = organization_deletion_jobs::request(
        db.pool(),
        &organization_deletion_jobs::RequestOrganizationDeletion {
            job_id: Uuid::new_v4(),
            root_organization_id: scope.organization_id,
            principal_id: principal.id,
            expected_project_path: project_path.clone(),
        },
    )
    .await
    .expect("request deletion from registered project scope");
    let (_, plan) = organization_deletion_jobs::claim_next_artifact_cleanup(
        db.pool(),
        &organization_deletion_jobs::ClaimOrganizationArtifactCleanup {
            worker_id: "canonical-project-root-test".to_string(),
            lease_seconds: 60,
        },
    )
    .await
    .expect("claim artifact plan")
    .expect("deletion job is ready");
    assert_eq!(plan.job_id, requested.id);
    assert_eq!(plan.project_path_at_time, project_path);
    assert_eq!(plan.targets.len(), 1);
    assert_eq!(plan.targets[0].target_id_at_time, target_id);
    assert_eq!(
        plan.targets[0].project_path_at_time, plan.project_path_at_time,
        "target-owned path text must never override the registered project scope root"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn deletion_request_rechecks_subtree_after_acquiring_row_locks() {
    let (mut db, _data_dir) = fixture().await;
    let principal = operator_principals::current_local(db.pool())
        .await
        .expect("load trusted deletion principal");
    // The lock query orders UUIDs. Make the child sort before the root so a
    // blocker on the child opens a deterministic pre-root-lock race window.
    let child_id = Uuid::from_u128(1);
    let external_id = Uuid::from_u128(2);
    let root_id = Uuid::from_u128(u128::MAX - 1);
    let project_path = format!("/fixture/subtree-race-{}", Uuid::new_v4());
    project_scopes::register_first_open(
        db.pool(),
        &project_path,
        &format!("subtree-race-{}-path-sha", Uuid::new_v4()),
    )
    .await
    .expect("register subtree-race project scope");
    sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$2,'race root')")
        .bind(root_id)
        .bind(&project_path)
        .execute(db.pool())
        .await
        .expect("insert race root");
    sqlx::query(
        "INSERT INTO organizations(id,project_path,name,parent_id) VALUES($1,$2,'race child',$3)",
    )
    .bind(child_id)
    .bind(&project_path)
    .bind(root_id)
    .execute(db.pool())
    .await
    .expect("insert race child");
    sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$2,'race external')")
        .bind(external_id)
        .bind(&project_path)
        .execute(db.pool())
        .await
        .expect("insert race external organization");

    let mut blocker = db.pool().begin().await.expect("begin subtree lock blocker");
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(child_id)
        .execute(&mut *blocker)
        .await
        .expect("lock first-sorted child");

    let request_pool = db.pool().clone();
    let request_project_path = project_path.clone();
    let request = tokio::spawn(async move {
        organization_deletion_jobs::request(
            &request_pool,
            &organization_deletion_jobs::RequestOrganizationDeletion {
                job_id: Uuid::new_v4(),
                root_organization_id: root_id,
                principal_id: principal.id,
                expected_project_path: request_project_path,
            },
        )
        .await
    });

    let mut waiting_on_lock = false;
    for _ in 0..100 {
        waiting_on_lock = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1 FROM pg_stat_activity
                    WHERE datname=current_database()
                      AND pid<>pg_backend_pid()
                      AND wait_event_type='Lock'
                      AND query LIKE '%SELECT id FROM organizations WHERE id=ANY%'
               )"#,
        )
        .fetch_one(db.pool())
        .await
        .expect("observe blocked deletion request");
        if waiting_on_lock {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(waiting_on_lock, "deletion request reached its subtree lock");

    sqlx::query("UPDATE organizations SET parent_id=$1,updated_at=NOW() WHERE id=$2")
        .bind(root_id)
        .bind(external_id)
        .execute(db.pool())
        .await
        .expect("commit concurrent subtree expansion before root lock");
    blocker
        .commit()
        .await
        .expect("release subtree lock blocker");

    let error = request
        .await
        .expect("join deletion request")
        .expect_err("post-lock subtree re-read must detect the concurrent expansion");
    assert!(error
        .to_string()
        .contains("organization_delete_subtree_changed"));

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn artifact_cleanup_success_remains_recoverable_until_hard_delete_commits() {
    let (mut db, _data_dir) = fixture().await;
    let principal = operator_principals::current_local(db.pool())
        .await
        .expect("load trusted deletion principal");
    let organization_id = Uuid::new_v4();
    let job_id = Uuid::new_v4();
    let project_path = format!("/fixture/hard-delete-{job_id}");
    let project_scope = project_scopes::register_first_open(
        db.pool(),
        &project_path,
        &format!("hard-delete-{job_id}-path-sha"),
    )
    .await
    .expect("register hard-delete project scope");
    sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$2,$3)")
        .bind(organization_id)
        .bind(&project_path)
        .bind("recoverable hard delete")
        .execute(db.pool())
        .await
        .expect("insert hard-delete organization");
    sqlx::query(
        r#"INSERT INTO organization_deletion_jobs(
               id,root_organization_id_at_time,project_scope_id,project_path_at_time,
               requested_by_principal_id,state,organization_snapshot,target_snapshot,
               artifact_cleanup_completed_at
           ) VALUES($1,$2,$3,$4,$5,'artifact_cleanup_succeeded',$6,'[]'::jsonb,NOW())"#,
    )
    .bind(job_id)
    .bind(organization_id)
    .bind(project_scope.project_scope_id)
    .bind(&project_path)
    .bind(principal.id)
    .bind(serde_json::json!([{
        "organizationIdAtTime": organization_id,
        "projectPathAtTime": project_path
    }]))
    .execute(db.pool())
    .await
    .expect("insert recoverable hard-delete job");
    sqlx::query(
        r#"INSERT INTO organization_deletion_job_units(
               job_id,organization_id_at_time,organization_name_at_time,depth,ordinal
           ) VALUES($1,$2,'recoverable hard delete',0,0)"#,
    )
    .bind(job_id)
    .bind(organization_id)
    .execute(db.pool())
    .await
    .expect("insert frozen deletion unit");

    assert_eq!(
        organization_deletion_jobs::next_hard_delete_ready(db.pool())
            .await
            .expect("load recoverable hard-delete job"),
        Some(job_id)
    );
    let committed = organization_deletion_jobs::hard_delete(db.pool(), job_id)
        .await
        .expect("commit recovered hard delete");
    assert_eq!(committed.state, "hard_delete_committed");
    let organization_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM organizations WHERE id=$1)")
            .bind(organization_id)
            .fetch_one(db.pool())
            .await
            .expect("query deleted organization");
    assert!(!organization_exists);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn failed_artifact_cleanup_backs_off_without_starving_a_ready_job() {
    let (mut db, _data_dir) = fixture().await;
    let principal = operator_principals::current_local(db.pool())
        .await
        .expect("load trusted deletion principal");
    let older_scope = frozen_scope(
        &db,
        &format!("/fixture/artifact-backoff-old-{}", Uuid::new_v4()),
        "artifact-backoff-old",
    )
    .await;
    let newer_scope = frozen_scope(
        &db,
        &format!("/fixture/artifact-backoff-new-{}", Uuid::new_v4()),
        "artifact-backoff-new",
    )
    .await;
    let older_job = Uuid::new_v4();
    let newer_job = Uuid::new_v4();
    for (job_id, scope, age) in [
        (older_job, older_scope, "2 hours"),
        (newer_job, newer_scope, "1 hour"),
    ] {
        sqlx::query(
            r#"INSERT INTO organization_deletion_jobs(
                   id,root_organization_id_at_time,project_scope_id,project_path_at_time,
                   requested_by_principal_id,state,organization_snapshot,target_snapshot,
                   requested_at
               )
               SELECT $1,$2,$3,canonical_project_path,$4,'pending_artifact_cleanup',
                      jsonb_build_array(jsonb_build_object(
                          'organizationIdAtTime',$2,
                          'projectPathAtTime',canonical_project_path
                      )),
                      '[]'::jsonb,NOW()-$5::interval
                 FROM project_scopes WHERE project_scope_id=$3"#,
        )
        .bind(job_id)
        .bind(scope.organization_id)
        .bind(scope.project_scope_id)
        .bind(principal.id)
        .bind(age)
        .execute(db.pool())
        .await
        .expect("insert pending artifact-cleanup job");
    }

    let worker_id = "artifact-backoff-fairness".to_string();
    let (claimed, _) = organization_deletion_jobs::claim_next_artifact_cleanup(
        db.pool(),
        &organization_deletion_jobs::ClaimOrganizationArtifactCleanup {
            worker_id: worker_id.clone(),
            lease_seconds: 60,
        },
    )
    .await
    .expect("claim oldest artifact cleanup")
    .expect("oldest artifact cleanup is ready");
    assert_eq!(claimed.id, older_job);
    let failed = organization_deletion_jobs::complete_artifact_cleanup(
        db.pool(),
        &organization_deletion_jobs::CompleteOrganizationArtifactCleanup {
            job_id: claimed.id,
            worker_id: worker_id.clone(),
            lease_token: claimed.lease_token.expect("fenced artifact claim"),
            expected_row_version: claimed.row_version,
            result: Err(organization_deletion_jobs::ArtifactCleanupFailure {
                code: "artifact_io_failed".to_string(),
                message: "temporary filesystem failure".to_string(),
            }),
        },
    )
    .await
    .expect("persist artifact cleanup failure");
    assert_eq!(failed.state, "pending_artifact_cleanup");
    assert!(failed.artifact_retry_not_before > failed.updated_at);
    assert_eq!(failed.attempt_count, 1);

    let (next, _) = organization_deletion_jobs::claim_next_artifact_cleanup(
        db.pool(),
        &organization_deletion_jobs::ClaimOrganizationArtifactCleanup {
            worker_id,
            lease_seconds: 60,
        },
    )
    .await
    .expect("claim another ready artifact cleanup")
    .expect("newer job must not be starved by the failed oldest job");
    assert_eq!(
        next.id, newer_job,
        "a failed job must enter durable backoff before it is claimable again"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn failed_hard_delete_backs_off_without_starving_a_ready_job() {
    let (mut db, _data_dir) = fixture().await;
    let principal = operator_principals::current_local(db.pool())
        .await
        .expect("load trusted deletion principal");
    let older_scope = frozen_scope(
        &db,
        &format!("/fixture/hard-backoff-old-{}", Uuid::new_v4()),
        "hard-backoff-old",
    )
    .await;
    let newer_scope = frozen_scope(
        &db,
        &format!("/fixture/hard-backoff-new-{}", Uuid::new_v4()),
        "hard-backoff-new",
    )
    .await;
    let older_job = Uuid::new_v4();
    let newer_job = Uuid::new_v4();
    for (job_id, scope, age) in [
        (older_job, older_scope, "2 hours"),
        (newer_job, newer_scope, "1 hour"),
    ] {
        sqlx::query(
            r#"INSERT INTO organization_deletion_jobs(
                   id,root_organization_id_at_time,project_scope_id,project_path_at_time,
                   requested_by_principal_id,state,organization_snapshot,target_snapshot,
                   requested_at,artifact_cleanup_completed_at
               )
               SELECT $1,$2,$3,canonical_project_path,$4,'artifact_cleanup_succeeded',
                      jsonb_build_array(jsonb_build_object(
                          'organizationIdAtTime',$2,
                          'projectPathAtTime',canonical_project_path
                      )),
                      '[]'::jsonb,NOW()-$5::interval,NOW()-$5::interval
                 FROM project_scopes WHERE project_scope_id=$3"#,
        )
        .bind(job_id)
        .bind(scope.organization_id)
        .bind(scope.project_scope_id)
        .bind(principal.id)
        .bind(age)
        .execute(db.pool())
        .await
        .expect("insert hard-delete-ready job");
    }
    assert_eq!(
        organization_deletion_jobs::next_hard_delete_ready(db.pool())
            .await
            .expect("load oldest hard-delete job"),
        Some(older_job)
    );
    organization_deletion_jobs::record_hard_delete_error(
        db.pool(),
        older_job,
        "organization_hard_delete_failed",
        "temporary database failure",
    )
    .await
    .expect("persist hard-delete failure");
    let failed = organization_deletion_jobs::get(db.pool(), older_job)
        .await
        .expect("read hard-delete retry state")
        .expect("hard-delete retry job remains durable");
    assert_eq!(failed.hard_delete_attempt_count, 1);
    assert!(failed.hard_delete_retry_not_before > failed.updated_at);
    assert_eq!(
        organization_deletion_jobs::next_hard_delete_ready(db.pool())
            .await
            .expect("load another ready hard-delete job"),
        Some(newer_job),
        "a failed hard delete must enter durable backoff before retry"
    );

    db.stop().await;
}
