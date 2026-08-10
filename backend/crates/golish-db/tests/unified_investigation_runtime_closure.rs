use golish_db::repo::unified_investigation_runtime::{
    BeginPentagiTaskPlanInput, InsertPentagiSubtaskInput, InsertPentagiTaskRunRequestInput,
    InvestigationStageIdentity, InvestigationUnitIdentity, PentagiSubjectKind,
    PgUnifiedInvestigationRuntimeRepository, RequestInvestigationStopInput,
    StartInvestigationRunInput,
};
use golish_db::{DbConfig, GolishDb};
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read local postgres port")
        .port()
}

#[tokio::test]
#[serial]
async fn stop_intent_loader_replays_the_exact_post_stop_authority() {
    let (db, _data_dir) = migrated_db("stop-response-loss").await;
    let f = fixture(db.pool(), "stop-response-loss").await;
    let repository = PgUnifiedInvestigationRuntimeRepository::new(Arc::new(db.pool().clone()));
    let identity = InvestigationStageIdentity {
        authority_id: f.authority_id,
        operation_id: f.operation_id,
        stage_execution_id: f.stage_execution_id,
        owning_stage_run_request_id: f.owning_request_id.clone(),
        scope_snapshot_id: f.scope_snapshot_id,
    };
    let head = repository
        .start_run(&StartInvestigationRunInput {
            identity: identity.clone(),
            stable_start_request_id: Uuid::new_v4(),
            initial_change_seq: 0,
        })
        .await
        .expect("start response-loss fixture run");
    let stop = repository
        .request_stop(&RequestInvestigationStopInput {
            identity: identity.clone(),
            stop_intent_id: Uuid::new_v4(),
            idempotency_key: Uuid::new_v4(),
            expected_run_head_sha256: head.head_sha256,
            expected_change_seq: 0,
        })
        .await
        .expect("commit the stop before the simulated response loss");

    let replayed = repository
        .load_stop_intent(&identity)
        .await
        .expect("load exact durable stop authority")
        .expect("stop authority exists");
    assert_eq!(replayed, stop);
}

#[tokio::test]
#[serial]
async fn stop_freezes_every_runtime_work_class_and_fences_stage_team_admission() {
    let (db, _data_dir) = migrated_db("stop-denominator").await;
    let f = fixture(db.pool(), "stop-denominator").await;
    let start_request_id = Uuid::new_v4();
    let run: (i64, String) = sqlx::query_as(
        r#"SELECT change_seq,head_sha256
             FROM register_investigation_run_v1($1,$2,$3,$4,$5,$6,0)"#,
    )
    .bind(f.authority_id)
    .bind(start_request_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(&f.owning_request_id)
    .bind(f.scope_snapshot_id)
    .fetch_one(db.pool())
    .await
    .expect("register Investigation run");

    let open_task_plan_id = Uuid::new_v4();
    let run_request_id = Uuid::new_v4();
    let subject_id = Uuid::new_v4();
    let subject_fingerprint = digest('d');
    sqlx::query(
        r#"INSERT INTO pentagi_task_run_requests(
               run_request_id,stable_request_id,task_plan_id,authority_id,
               operation_id,stage_execution_id,owning_stage_run_request_id,
               stage_run_unit_id,organization_id,subject_kind,subject_id,
               subject_fingerprint_sha256,request_sha256
           ) VALUES($1,$2,NULL,$3,$4,$5,$6,$7,$8,'analysis_attempt',$9,$10,$11)"#,
    )
    .bind(run_request_id)
    .bind(Uuid::new_v4())
    .bind(f.authority_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(&f.owning_request_id)
    .bind(f.stage_run_unit_id)
    .bind(f.organization_id)
    .bind(subject_id)
    .bind(&subject_fingerprint)
    .bind(digest('c'))
    .execute(db.pool())
    .await
    .expect("seed request-first PentAGI admission");
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_task_plans(
               task_plan_id,stable_request_id,run_request_id,authority_id,stage_team_plan_id,
               operation_id,stage_execution_id,owning_stage_run_request_id,
               stage_run_unit_id,scope_snapshot_id,organization_id,subject_kind,
               subject_id,subject_fingerprint_sha256,task_plan_version,
               task_plan_sha256,allowed_role_catalog,cognitive_tool_envelope_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,'analysis_attempt',$12,$13,1,
                    $14,$15,$16)"#,
    )
    .bind(open_task_plan_id)
    .bind(Uuid::new_v4())
    .bind(run_request_id)
    .bind(f.authority_id)
    .bind(f.stage_team_plan_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(&f.owning_request_id)
    .bind(f.stage_run_unit_id)
    .bind(f.scope_snapshot_id)
    .bind(f.organization_id)
    .bind(subject_id)
    .bind(subject_fingerprint)
    .bind(digest('e'))
    .bind(serde_json::json!(["primary", "researcher"]))
    .bind(digest('f'))
    .execute(db.pool())
    .await
    .expect("seed open PentAGI plan before stop");

    let work_classes = [
        "analysis",
        "read_session",
        "query",
        "enrichment",
        "outbox",
        "verification_task",
        "pentagi_subtask",
        "worker_request",
        "campaign",
        "prepared_action",
        "action_execution",
        "fact_delta",
        "consolidation",
    ];
    for (ordinal, work_kind) in work_classes.iter().enumerate() {
        let nibble = char::from_digit(ordinal as u32, 16).expect("hex work ordinal");
        sqlx::query(
            r#"INSERT INTO investigation_run_work_items(
                   work_id,stable_work_key_sha256,authority_id,operation_id,
                   stage_execution_id,owning_stage_run_request_id,stage_run_unit_id,
                   scope_snapshot_id,organization_id,work_kind,external_identity_sha256,
                   current_state,observed_stop_epoch
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,'running',0)"#,
        )
        .bind(Uuid::new_v4())
        .bind(digest(nibble))
        .bind(f.authority_id)
        .bind(f.operation_id)
        .bind(f.stage_execution_id)
        .bind(&f.owning_request_id)
        .bind(f.stage_run_unit_id)
        .bind(f.scope_snapshot_id)
        .bind(f.organization_id)
        .bind(work_kind)
        .bind(digest(
            char::from_digit((ordinal + 1) as u32, 16).expect("hex source ordinal"),
        ))
        .execute(db.pool())
        .await
        .expect("register stop denominator member");
    }

    let stop_intent_id = Uuid::new_v4();
    let stop_key = Uuid::new_v4();
    let stop: (Uuid, i64, String, String) = sqlx::query_as(
        r#"SELECT stop_intent_id,frozen_work_count,frozen_work_set_sha256,receipt_sha256
             FROM investigation_request_stop_v1($1,$2,$3,$4,$5,$6,$7,$8)"#,
    )
    .bind(stop_intent_id)
    .bind(stop_key)
    .bind(f.authority_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(&f.owning_request_id)
    .bind(&run.1)
    .bind(run.0)
    .fetch_one(db.pool())
    .await
    .expect("freeze complete stop denominator");
    assert_eq!(stop.1, work_classes.len() as i64 + 1);

    let frozen_classes: Vec<String> = sqlx::query_scalar(
        r#"SELECT work_class
             FROM investigation_stop_denominator_members
            WHERE stop_intent_id=$1
            ORDER BY work_class"#,
    )
    .bind(stop_intent_id)
    .fetch_all(db.pool())
    .await
    .expect("load frozen denominator classes");
    let mut expected_classes = work_classes.map(str::to_string).to_vec();
    expected_classes.push("pentagi_plan".to_string());
    expected_classes.sort();
    assert_eq!(frozen_classes, expected_classes);

    let replay: (Uuid, i64, String, String) = sqlx::query_as(
        r#"SELECT stop_intent_id,frozen_work_count,frozen_work_set_sha256,receipt_sha256
             FROM investigation_request_stop_v1($1,$2,$3,$4,$5,$6,$7,$8)"#,
    )
    .bind(stop_intent_id)
    .bind(stop_key)
    .bind(f.authority_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(&f.owning_request_id)
    .bind(&run.1)
    .bind(run.0)
    .fetch_one(db.pool())
    .await
    .expect("replay complete stop denominator receipt");
    assert_eq!(replay, stop);

    let late_subtask_error = sqlx::query(
        r#"INSERT INTO investigation_pentagi_subtasks(
               subtask_id,task_plan_id,authority_id,operation_id,stage_execution_id,
               stage_run_unit_id,organization_id,subtask_ordinal,label,runnable,
               input_manifest_sha256,expected_output_schema,member_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,0,'late-after-stop',TRUE,$8,'{}',$9)"#,
    )
    .bind(Uuid::new_v4())
    .bind(open_task_plan_id)
    .bind(f.authority_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(f.stage_run_unit_id)
    .bind(f.organization_id)
    .bind(digest('a'))
    .bind(digest('b'))
    .execute(db.pool())
    .await
    .expect_err("PentAGI child admission must share the Investigation stop fence");
    assert!(
        late_subtask_error
            .to_string()
            .contains("INVESTIGATION_PENTAGI_CHILD_ADMISSION_CLOSED"),
        "{late_subtask_error}"
    );

    let late_stage_work_error = sqlx::query(
        r#"INSERT INTO stage_work_items(
               id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
               input_manifest_hash,status,output_schema,created_by
           )
           SELECT $1,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                  scope_snapshot_id,organization_id,dispatch_epoch,kind,$2,role,
                  input_manifest_hash,'queued',output_schema,'server_seed'
             FROM stage_work_items WHERE id=$3"#,
    )
    .bind(Uuid::new_v4())
    .bind(format!("late-after-stop-{}", Uuid::new_v4().simple()))
    .bind(f.primary_work_item_id)
    .execute(db.pool())
    .await
    .expect_err("StageTeam work admission must share the Investigation stop fence");
    assert!(
        late_stage_work_error
            .to_string()
            .contains("INVESTIGATION_CLOSURE_LATE_WORK_REJECTED"),
        "{late_stage_work_error}"
    );
}

fn digest(nibble: char) -> String {
    format!("sha256:{}", nibble.to_string().repeat(64))
}

async fn migrated_db(label: &str) -> (GolishDb, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("unified_runtime_{label}_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start isolated migrated postgres");
    (db, data_dir)
}

#[derive(Debug, Clone)]
struct Fixture {
    operation_id: Uuid,
    stage_execution_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    stage_run_unit_id: Uuid,
    authority_id: Uuid,
    owning_request_id: String,
    stage_team_plan_id: Uuid,
    primary_work_item_id: Uuid,
    primary_worker_run_id: Uuid,
    worker_work_item_id: Uuid,
    worker_request_id: Uuid,
    worker_run_id: Uuid,
    spare_primary_work_item_id: Uuid,
    spare_primary_worker_run_id: Uuid,
}

async fn fixture(pool: &PgPool, label: &str) -> Fixture {
    let operation_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    let project_path = format!("/tmp/unified-runtime-{label}-{}", Uuid::new_v4().simple());
    let organization_id = Uuid::new_v4();
    let au_execution_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    let scope_decision_id = Uuid::new_v4();
    let scope_snapshot_id = Uuid::new_v4();
    let stage_run_unit_id = Uuid::new_v4();
    let authority_id = Uuid::new_v4();
    let owning_request_id = format!("investigation-stage-request-{label}");

    sqlx::query(
        "INSERT INTO project_scopes(project_scope_id,canonical_project_path,path_sha256) VALUES($1,$2,$3)",
    )
    .bind(project_scope_id)
    .bind(&project_path)
    .bind(digest('1'))
    .execute(pool)
    .await
    .expect("insert project scope");
    let mut deployment = pool.begin().await.expect("begin rollout selection");
    for statement in [
        "ALTER TABLE tool_truth_rollout DISABLE TRIGGER tool_truth_rollout_direct_mutation_guard",
        "ALTER TABLE investigation_rollout DISABLE TRIGGER investigation_rollout_direct_mutation_guard",
    ] {
        sqlx::query(statement)
            .execute(&mut *deployment)
            .await
            .expect("disable isolated rollout guard");
    }
    sqlx::query(
        "UPDATE tool_truth_rollout SET new_operation_contract='receipt_v1',row_version=row_version+1 WHERE singleton=TRUE",
    )
    .execute(&mut *deployment)
    .await
    .expect("select receipt Tool Truth");
    sqlx::query(
        r#"UPDATE investigation_rollout
              SET contract_version='hypothesis_registry_v1',rollout_mode='new_only',
                  mode_rank=4,row_version=row_version+1 WHERE singleton=TRUE"#,
    )
    .execute(&mut *deployment)
    .await
    .expect("select unified Investigation");
    for statement in [
        "ALTER TABLE investigation_rollout ENABLE TRIGGER investigation_rollout_direct_mutation_guard",
        "ALTER TABLE tool_truth_rollout ENABLE TRIGGER tool_truth_rollout_direct_mutation_guard",
    ] {
        sqlx::query(statement)
            .execute(&mut *deployment)
            .await
            .expect("restore isolated rollout guard");
    }
    deployment.commit().await.expect("commit rollout selection");

    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,project_scope_id,
               tool_truth_contract,investigation_contract_version,investigation_rollout_mode
           ) VALUES($1,'red_team','application_understanding','v2_only',$2,
                    'receipt_v1','hypothesis_registry_v1','new_only')"#,
    )
    .bind(operation_id)
    .bind(project_scope_id)
    .execute(pool)
    .await
    .expect("insert unified operation");
    sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$2,'Runtime Org')")
        .bind(organization_id)
        .bind(&project_path)
        .execute(pool)
        .await
        .expect("insert organization");
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status) VALUES($1,$2,'application_understanding','started')",
    )
    .bind(au_execution_id)
    .bind(operation_id)
    .execute(pool)
    .await
    .expect("insert AU execution");
    sqlx::query(
        r#"INSERT INTO operation_scope_decisions(
               id,operation_id,project_scope_id,stage_execution_id,
               root_organization_id,mode,decision_rows,decision_hash
           ) VALUES($1,$2,$3,$4,$5,'cli_flags',$6,$7)"#,
    )
    .bind(scope_decision_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(au_execution_id)
    .bind(organization_id)
    .bind(serde_json::json!([{"organization_id": organization_id}]))
    .bind(digest('2'))
    .execute(pool)
    .await
    .expect("insert scope decision");
    let mut scope_tx = pool.begin().await.expect("begin scope snapshot");
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
    .expect("insert scope snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units(
               snapshot_id,organization_id,parent_organization_id,
               organization_name_at_freeze,role,depth,ordinal,
               decision_row_id,approval_source
           ) VALUES($1,$2,NULL,'Runtime Org','root',0,0,'root',$3)"#,
    )
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(serde_json::json!({"source":"runtime_closure_fixture"}))
    .execute(&mut *scope_tx)
    .await
    .expect("insert scope member");
    sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
        .bind(scope_snapshot_id)
        .execute(&mut *scope_tx)
        .await
        .expect("seal scope snapshot");
    scope_tx.commit().await.expect("commit scope snapshot");

    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status) VALUES($1,$2,'investigation','started')",
    )
    .bind(stage_execution_id)
    .bind(operation_id)
    .execute(pool)
    .await
    .expect("insert Investigation execution");
    sqlx::query("UPDATE operation_state SET current_stage='investigation' WHERE operation_id=$1")
        .bind(operation_id)
        .execute(pool)
        .await
        .expect("enter Investigation");
    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,
               organization_id,stage_kind,generation,status,started_at
           ) VALUES($1,$2,$3,$4,$5,'investigation',0,'running',NOW())"#,
    )
    .bind(stage_run_unit_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .execute(pool)
    .await
    .expect("insert Investigation unit");
    sqlx::query(
        r#"INSERT INTO investigation_stage_run_authorities(
               authority_id,operation_id,stage_execution_id,
               owning_stage_run_request_id,scope_snapshot_id
           ) VALUES($1,$2,$3,$4,$5)"#,
    )
    .bind(authority_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(&owning_request_id)
    .bind(scope_snapshot_id)
    .execute(pool)
    .await
    .expect("insert Investigation authority");

    let stage_team_plan_id = Uuid::new_v4();
    let primary_work_item_id = Uuid::new_v4();
    let primary_worker_run_id = Uuid::new_v4();
    let worker_work_item_id = Uuid::new_v4();
    let worker_request_id = Uuid::new_v4();
    let worker_run_id = Uuid::new_v4();
    let spare_primary_work_item_id = Uuid::new_v4();
    let spare_primary_worker_run_id = Uuid::new_v4();
    let mut actor_tx = pool.begin().await.expect("begin StageTeam adapter seed");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *actor_tx)
        .await
        .expect("isolate StageTeam adapter seed");
    sqlx::query(
        r#"INSERT INTO stage_team_plans(
               id,operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
               organization_id,stage_kind,unit_generation,schema_version,plan_version,
               plan_hash,leader_role,aggregator_kind,allowed_worker_roles,
               max_workers_total,max_workers_active,dynamic_requests_allowed,
               final_submitter_kind,created_from_stage_spec_hash,requests_closed_at
           ) VALUES($1,$2,$3,$4,$5,$6,'investigation',0,1,1,$7,'primary',
                    'deterministic',$8,8,4,TRUE,'deterministic',$9,NOW())"#,
    )
    .bind(stage_team_plan_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(digest('4'))
    .bind(serde_json::json!(["primary", "researcher"]))
    .bind(digest('5'))
    .execute(&mut *actor_tx)
    .await
    .expect("seed StageTeam plan");
    for (id, kind, key, role) in [
        (
            primary_work_item_id,
            "pentagi_primary",
            "primary",
            "primary",
        ),
        (
            worker_work_item_id,
            "pentagi_worker",
            "worker-0",
            "researcher",
        ),
        (
            spare_primary_work_item_id,
            "pentagi_primary",
            "spare-primary",
            "primary",
        ),
    ] {
        sqlx::query(
            r#"INSERT INTO stage_work_items(
                   id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                   scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
                   input_manifest_hash,status,output_schema,created_by,terminal_at
               ) VALUES($1,$2,$3,$4,$5,$6,$7,0,$8,$9,$10,$11,'completed',
                        'InvestigationPentagiWorkerResultV1','server_seed',NOW())"#,
        )
        .bind(id)
        .bind(stage_team_plan_id)
        .bind(operation_id)
        .bind(stage_execution_id)
        .bind(stage_run_unit_id)
        .bind(scope_snapshot_id)
        .bind(organization_id)
        .bind(kind)
        .bind(key)
        .bind(role)
        .bind(digest('6'))
        .execute(&mut *actor_tx)
        .await
        .expect("seed StageTeam work item");
    }
    for (id, work_item_id, specialist, kind, key, request_id) in [
        (
            primary_worker_run_id,
            primary_work_item_id,
            "primary",
            "pentagi_primary",
            "primary",
            None,
        ),
        (
            worker_run_id,
            worker_work_item_id,
            "researcher",
            "pentagi_worker",
            "worker-0",
            Some("dispatch-tool-0"),
        ),
        (
            spare_primary_worker_run_id,
            spare_primary_work_item_id,
            "primary",
            "pentagi_primary",
            "spare-primary",
            None,
        ),
    ] {
        sqlx::query(
            r#"INSERT INTO stage_worker_runs(
                   id,operation_id,stage_execution_id,stage_run_unit_id,organization_id,
                   worker_generation,specialist,work_item_kind,work_item_key,agent_path,
                   parent_request_id,status,terminal_at,work_item_id
               ) VALUES($1,$2,$3,$4,$5,0,$6,$7,$8,$9,$10,'passed',NOW(),$11)"#,
        )
        .bind(id)
        .bind(operation_id)
        .bind(stage_execution_id)
        .bind(stage_run_unit_id)
        .bind(organization_id)
        .bind(specialist)
        .bind(kind)
        .bind(key)
        .bind(format!("investigation/{key}"))
        .bind(request_id)
        .bind(work_item_id)
        .execute(&mut *actor_tx)
        .await
        .expect("seed StageTeam worker run");
    }
    sqlx::query(
        r#"INSERT INTO stage_worker_requests(
               id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,parent_work_item_id,parent_worker_run_id,
               dispatch_epoch,requested_role,request_kind,reason_code,
               expected_output_schema,dedupe_key,request_payload_hash,status,
               accepted_work_item_id
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,0,'researcher','pentagi_worker',
                    'dynamic_gap','InvestigationPentagiWorkerResultV1','worker-0',$10,
                    'accepted',$11)"#,
    )
    .bind(worker_request_id)
    .bind(stage_team_plan_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(primary_work_item_id)
    .bind(primary_worker_run_id)
    .bind(digest('7'))
    .bind(worker_work_item_id)
    .execute(&mut *actor_tx)
    .await
    .expect("seed StageTeam worker request");
    actor_tx
        .commit()
        .await
        .expect("commit StageTeam adapter seed");

    Fixture {
        operation_id,
        stage_execution_id,
        scope_snapshot_id,
        organization_id,
        stage_run_unit_id,
        authority_id,
        owning_request_id,
        stage_team_plan_id,
        primary_work_item_id,
        primary_worker_run_id,
        worker_work_item_id,
        worker_request_id,
        worker_run_id,
        spare_primary_work_item_id,
        spare_primary_worker_run_id,
    }
}

#[tokio::test]
#[serial]
async fn repository_inserts_pentagi_subtask_with_exact_task_and_org_identity() {
    let (db, _data_dir) = migrated_db("pentagi-subtask-bind-order").await;
    let f = fixture(db.pool(), "pentagi-subtask-bind-order").await;
    let repository = PgUnifiedInvestigationRuntimeRepository::new(Arc::new(db.pool().clone()));
    let stage_identity = InvestigationStageIdentity {
        authority_id: f.authority_id,
        operation_id: f.operation_id,
        stage_execution_id: f.stage_execution_id,
        owning_stage_run_request_id: f.owning_request_id.clone(),
        scope_snapshot_id: f.scope_snapshot_id,
    };
    let identity = InvestigationUnitIdentity {
        stage: stage_identity.clone(),
        stage_run_unit_id: f.stage_run_unit_id,
        organization_id: f.organization_id,
    };
    repository
        .start_run(&StartInvestigationRunInput {
            identity: stage_identity,
            stable_start_request_id: Uuid::new_v4(),
            initial_change_seq: 0,
        })
        .await
        .expect("start repository-backed Investigation run");

    let subject_id = Uuid::new_v4();
    let run_request_id = Uuid::new_v4();
    repository
        .insert_pentagi_run_request(&InsertPentagiTaskRunRequestInput {
            identity: identity.clone(),
            run_request_id,
            stable_request_id: Uuid::new_v4(),
            subject_kind: PentagiSubjectKind::AnalysisAttempt,
            subject_id,
            subject_fingerprint_sha256: digest('6'),
            request_sha256: digest('7'),
        })
        .await
        .expect("insert repository-backed PentAGI request");

    let task_plan_id = Uuid::new_v4();
    repository
        .begin_pentagi_plan(&BeginPentagiTaskPlanInput {
            identity: identity.clone(),
            task_plan_id,
            stable_request_id: Uuid::new_v4(),
            run_request_id,
            stage_team_plan_id: f.stage_team_plan_id,
            subject_kind: PentagiSubjectKind::AnalysisAttempt,
            subject_id,
            subject_fingerprint_sha256: digest('6'),
            task_plan_version: 1,
            task_plan_sha256: digest('8'),
            allowed_role_catalog: serde_json::json!(["primary", "researcher"]),
            cognitive_tool_envelope_sha256: digest('9'),
        })
        .await
        .expect("begin repository-backed PentAGI plan");

    let subtask_id = Uuid::new_v4();
    let request = InsertPentagiSubtaskInput {
        identity,
        task_plan_id,
        subtask_id,
        subtask_ordinal: 3,
        label: "identity-bind-regression".to_string(),
        runnable: true,
        input_manifest_sha256: digest('a'),
        expected_output_schema: "InvestigationPentagiWorkerResultV1".to_string(),
        member_sha256: digest('b'),
    };
    let inserted = repository
        .insert_pentagi_subtask(&request)
        .await
        .expect("insert PentAGI subtask without shifting UUID and ordinal binds");
    assert_eq!(inserted.task_plan_id, task_plan_id);
    assert_eq!(inserted.organization_id, f.organization_id);
    assert_eq!(inserted.subtask_ordinal, 3);
    assert_eq!(
        inserted,
        repository.insert_pentagi_subtask(&request).await.unwrap()
    );
}

async fn exact_set_hash(
    pool: &PgPool,
    domain: &str,
    table: &str,
    value_column: &str,
    order_column: &str,
    where_column: &str,
    id: Uuid,
) -> (i64, String) {
    let statement = format!(
        "SELECT COUNT(*),unified_investigation_exact_set_hash($1,COALESCE(array_agg({value_column} ORDER BY {order_column}),ARRAY[]::TEXT[])) FROM {table} WHERE {where_column}=$2"
    );
    sqlx::query_as(&statement)
        .bind(domain)
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("compute exact set hash")
}

async fn seed_refiner_plan_ledger(
    pool: &PgPool,
    task_plan_id: Uuid,
    runnable_subtask_ids: Vec<Uuid>,
) -> (i64, String) {
    let ledger_id = Uuid::new_v4();
    let ledger_request_id = Uuid::new_v4();
    let generator_event_id = Uuid::new_v4();
    let generator_manifest = serde_json::json!({
        "contract_version": "investigation_refiner_generator_manifest.v1",
        "strategy": "bounded_parallel_analysis"
    });
    let ledger: (String, i64) = sqlx::query_as(
        r#"SELECT ledger_sha256,generator_subtask_count
             FROM create_investigation_refiner_plan_ledger_v1($1,$2,$3,$4,$5)"#,
    )
    .bind(ledger_id)
    .bind(ledger_request_id)
    .bind(task_plan_id)
    .bind(generator_event_id)
    .bind(&generator_manifest)
    .fetch_one(pool)
    .await
    .expect("create DB-derived Generator manifest ledger");
    let replay: (String, i64) = sqlx::query_as(
        r#"SELECT ledger_sha256,generator_subtask_count
             FROM create_investigation_refiner_plan_ledger_v1($1,$2,$3,$4,$5)"#,
    )
    .bind(ledger_id)
    .bind(ledger_request_id)
    .bind(task_plan_id)
    .bind(generator_event_id)
    .bind(&generator_manifest)
    .fetch_one(pool)
    .await
    .expect("replay Generator manifest ledger");
    assert_eq!(ledger, replay);

    let stale_patch_error = sqlx::query(
        r#"SELECT patch_id FROM append_investigation_refiner_plan_patch_v1(
               $1,$2,$3,$4,$5,$6,$7,$8)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(ledger_id)
    .bind(task_plan_id)
    .bind(Uuid::new_v4())
    .bind(digest('0'))
    .bind(serde_json::json!({"remaining": ["worker-result"]}))
    .bind(&runnable_subtask_ids)
    .execute(pool)
    .await
    .expect_err("stale Refiner previous-state CAS must fail");
    assert!(stale_patch_error
        .to_string()
        .contains("INVESTIGATION_REFINER_PATCH_PREVIOUS_STATE_CAS_MISMATCH"));

    let patch_id = Uuid::new_v4();
    let patch_request_id = Uuid::new_v4();
    let patch_event_id = Uuid::new_v4();
    let remaining_plan = serde_json::json!({
        "contract_version": "investigation_refiner_remaining_plan_patch.v1",
        "remaining": []
    });
    let patch: (String, i64, i64) = sqlx::query_as(
        r#"SELECT patch_sha256,patch_ordinal,active_realized_subtask_count
             FROM append_investigation_refiner_plan_patch_v1($1,$2,$3,$4,$5,$6,$7,$8)"#,
    )
    .bind(patch_id)
    .bind(patch_request_id)
    .bind(ledger_id)
    .bind(task_plan_id)
    .bind(patch_event_id)
    .bind(&ledger.0)
    .bind(&remaining_plan)
    .bind(&runnable_subtask_ids)
    .fetch_one(pool)
    .await
    .expect("append exact Refiner remaining-plan patch");
    assert_eq!(patch.1, 0);
    assert_eq!(patch.2, runnable_subtask_ids.len() as i64);
    let patch_replay: (String, i64, i64) = sqlx::query_as(
        r#"SELECT patch_sha256,patch_ordinal,active_realized_subtask_count
             FROM append_investigation_refiner_plan_patch_v1($1,$2,$3,$4,$5,$6,$7,$8)"#,
    )
    .bind(patch_id)
    .bind(patch_request_id)
    .bind(ledger_id)
    .bind(task_plan_id)
    .bind(patch_event_id)
    .bind(&ledger.0)
    .bind(&remaining_plan)
    .bind(&runnable_subtask_ids)
    .fetch_one(pool)
    .await
    .expect("replay exact Refiner patch");
    assert_eq!(patch, patch_replay);

    let mut forged = pool.begin().await.expect("begin forged Refiner seal probe");
    let (primary_dispatch_id, primary_worker_run_id): (Uuid, Uuid) = sqlx::query_as(
        r#"SELECT dispatch_receipt_id,worker_run_id
             FROM pentagi_logical_dispatch_receipts
            WHERE task_plan_id=$1 AND actor_kind='primary'"#,
    )
    .bind(task_plan_id)
    .fetch_one(&mut *forged)
    .await
    .expect("load Primary for forged seal probe");
    let forged_barrier_event_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_pipeline_events(
               pipeline_event_id,stable_request_id,task_plan_id,event_ordinal,event_kind,
               actor_worker_run_id,parent_dispatch_receipt_id,event_sha256
           ) SELECT $1,$2,$3,COALESCE(MAX(event_ordinal)+1,0),'result_barrier',$4,$5,$6
               FROM investigation_pentagi_pipeline_events WHERE task_plan_id=$3"#,
    )
    .bind(forged_barrier_event_id)
    .bind(Uuid::new_v4())
    .bind(task_plan_id)
    .bind(primary_worker_run_id)
    .bind(primary_dispatch_id)
    .bind(digest('0'))
    .execute(&mut *forged)
    .await
    .expect("seed forged result barrier in rollback-only probe");
    let forged_seal_error = sqlx::query(
        r#"INSERT INTO investigation_refiner_plan_ledger_seals(
               seal_id,stable_request_id,ledger_id,task_plan_id,
               result_barrier_pipeline_event_id,patch_count,patch_set_sha256,
               final_patch_id,final_patch_sha256,final_active_realized_subtask_count,
               final_active_realized_subtask_set_sha256,generator_subtask_count,
               generator_subtask_set_sha256,seal_sha256
           ) VALUES($1,$2,$3,$4,$5,1,$6,$7,$8,$9,$10,$11,$12,$13)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(ledger_id)
    .bind(task_plan_id)
    .bind(forged_barrier_event_id)
    .bind(digest('1'))
    .bind(patch_id)
    .bind(&patch.0)
    .bind(runnable_subtask_ids.len() as i64)
    .bind(digest('2'))
    .bind(ledger.1)
    .bind(digest('3'))
    .bind(digest('4'))
    .execute(&mut *forged)
    .await
    .expect_err("direct forged Refiner seal must fail DB-authoritative validation");
    assert!(forged_seal_error
        .to_string()
        .contains("INVESTIGATION_REFINER_SEAL_AUTHORITY_INVALID"));
    forged
        .rollback()
        .await
        .expect("rollback forged Refiner seal probe");

    let seal: (i64, i64, String) = sqlx::query_as(
        r#"SELECT patch_count,final_active_realized_subtask_count,seal_sha256
             FROM seal_investigation_refiner_plan_ledger_v1($1,$2,$3,$4,$5,$6)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(ledger_id)
    .bind(task_plan_id)
    .bind(Uuid::new_v4())
    .bind(&patch.0)
    .fetch_one(pool)
    .await
    .expect("seal Refiner ledger at exact result barrier");
    assert_eq!(seal.0, 1);
    assert_eq!(seal.1, runnable_subtask_ids.len() as i64);
    exact_set_hash(
        pool,
        "investigation_pentagi_pipeline_events.v1",
        "investigation_pentagi_pipeline_events",
        "event_sha256",
        "pipeline_event_id",
        "task_plan_id",
        task_plan_id,
    )
    .await
}

async fn seed_full_closure_authority(pool: &PgPool, f: &Fixture, with_residual: bool) {
    let snapshot_id = Uuid::new_v4();
    let session_set_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    let session_member_hash = digest('b');
    let session_set_hash: String = sqlx::query_scalar(
        "SELECT unified_investigation_exact_set_hash('investigation_main_read_sessions.v1',ARRAY[$1]::TEXT[])",
    )
    .bind(format!("{session_id}:{session_member_hash}"))
    .fetch_one(pool)
    .await
    .expect("compute main read-session set hash");
    let generation_id = Uuid::new_v4();
    let generation_hash = digest('c');
    let generation_member_id = Uuid::new_v4();
    let generation_member_hash = digest('e');
    let generation_member_hashes = if with_residual {
        vec![generation_member_hash.clone()]
    } else {
        Vec::new()
    };
    let generation_member_set_hash: String = sqlx::query_scalar(
        "SELECT investigation_exact_member_set_hash('hypothesis_generation_members.v1',$1::TEXT[])",
    )
    .bind(generation_member_hashes)
    .fetch_one(pool)
    .await
    .expect("compute generation-member set hash");
    let revision_id = Uuid::new_v4();
    let admission_set_id = Uuid::new_v4();
    let admission_member_hash = digest('d');
    let admission_hash_members = if with_residual {
        vec![admission_member_hash.clone()]
    } else {
        Vec::new()
    };
    let admission_hash: String = sqlx::query_scalar(
        "SELECT unified_investigation_exact_set_hash('verification_admission_members.v1',$1::TEXT[])",
    )
    .bind(admission_hash_members)
    .fetch_one(pool)
    .await
    .expect("compute empty admission hash");

    let mut tx = pool
        .begin()
        .await
        .expect("begin full closure authority seed");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("isolate full closure authority seed");
    sqlx::query(
        r#"INSERT INTO investigation_analysis_snapshot_authorities(
               snapshot_id,authority_id,operation_id,stage_execution_id,
               owning_stage_run_request_id,stage_run_unit_id,scope_snapshot_id,
               organization_id,snapshot_sha256,context_item_count,
               context_item_set_sha256,methodology_hit_count,
               methodology_result_set_sha256,omission_count,omission_set_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,0,$10,0,$11,0,$12)"#,
    )
    .bind(snapshot_id)
    .bind(f.authority_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(&f.owning_request_id)
    .bind(f.stage_run_unit_id)
    .bind(f.scope_snapshot_id)
    .bind(f.organization_id)
    .bind(digest('1'))
    .bind(digest('2'))
    .bind(digest('3'))
    .bind(digest('4'))
    .execute(&mut *tx)
    .await
    .expect("seed analysis snapshot authority");
    sqlx::query(
        r#"INSERT INTO investigation_main_session_sets(
               session_set_id,stable_request_id,authority_id,operation_id,
               stage_execution_id,owning_stage_run_request_id,scope_snapshot_id,
               session_set_ordinal,status,member_count,member_set_sha256,row_version,sealed_at
           ) VALUES($1,$2,$3,$4,$5,$6,$7,0,'sealed',1,$8,1,NOW())"#,
    )
    .bind(session_set_id)
    .bind(Uuid::new_v4())
    .bind(f.authority_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(&f.owning_request_id)
    .bind(f.scope_snapshot_id)
    .bind(&session_set_hash)
    .execute(&mut *tx)
    .await
    .expect("seed sealed main session set");
    sqlx::query(
        r#"INSERT INTO investigation_main_read_sessions(
               main_read_session_id,session_set_id,authority_id,operation_id,
               stage_execution_id,owning_stage_run_request_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,snapshot_id,snapshot_sha256,
               context_chain_id,transcript_partition_id,session_contract_version,member_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,
                    'investigation_main_organization_read_session.v1',$14)"#,
    )
    .bind(session_id)
    .bind(session_set_id)
    .bind(f.authority_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(&f.owning_request_id)
    .bind(f.stage_run_unit_id)
    .bind(f.scope_snapshot_id)
    .bind(f.organization_id)
    .bind(snapshot_id)
    .bind(digest('1'))
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(session_member_hash)
    .execute(&mut *tx)
    .await
    .expect("seed main read session");
    sqlx::query(
        r#"INSERT INTO investigation_main_read_session_receipts(
               receipt_id,main_read_session_id,operation_id,stage_execution_id,
               stage_run_unit_id,organization_id,snapshot_id,snapshot_sha256,
               context_item_count,context_item_set_sha256,methodology_hit_count,
               methodology_result_set_sha256,omission_count,omission_set_sha256,receipt_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,0,$9,0,$10,0,$11,$12)"#,
    )
    .bind(Uuid::new_v4())
    .bind(session_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(f.stage_run_unit_id)
    .bind(f.organization_id)
    .bind(snapshot_id)
    .bind(digest('1'))
    .bind(digest('2'))
    .bind(digest('3'))
    .bind(digest('4'))
    .bind(digest('5'))
    .execute(&mut *tx)
    .await
    .expect("seed main read-session receipt");
    sqlx::query(
        r#"INSERT INTO hypothesis_generations(
               generation_id,operation_id,organization_id,generation_ordinal,
               candidate_snapshot_id,candidate_gate_decision_id,
               candidate_snapshot_authority_hash
           ) VALUES($1,$2,$3,0,$4,$5,$6)"#,
    )
    .bind(generation_id)
    .bind(f.operation_id)
    .bind(f.organization_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(digest('6'))
    .execute(&mut *tx)
    .await
    .expect("seed current generation");
    if with_residual {
        sqlx::query(
            r#"INSERT INTO hypothesis_generation_members(
                   generation_member_id,generation_id,operation_id,organization_id,
                   revision_id,ordinal,member_hash
               ) VALUES($1,$2,$3,$4,$5,0,$6)"#,
        )
        .bind(generation_member_id)
        .bind(generation_id)
        .bind(f.operation_id)
        .bind(f.organization_id)
        .bind(revision_id)
        .bind(generation_member_hash)
        .execute(&mut *tx)
        .await
        .expect("seed residual generation member");
    }
    sqlx::query(
        r#"INSERT INTO hypothesis_generation_seals(
               seal_id,generation_id,member_count,member_set_hash,event_count,
               event_set_hash,open_obligation_set_hash,controller_worker_run_id,generation_hash
           ) VALUES($1,$2,$3,$4,0,$5,$6,$7,$8)"#,
    )
    .bind(Uuid::new_v4())
    .bind(generation_id)
    .bind(if with_residual { 1_i64 } else { 0_i64 })
    .bind(generation_member_set_hash)
    .bind(digest('8'))
    .bind(digest('9'))
    .bind(f.primary_worker_run_id)
    .bind(generation_hash)
    .execute(&mut *tx)
    .await
    .expect("seed generation seal");
    sqlx::query(
        r#"INSERT INTO verification_admission_sets(
               admission_set_id,stable_request_id,operation_id,stage_execution_id,
               stage_run_unit_id,scope_snapshot_id,organization_id,generation_id,
               status,member_count,member_set_sha256,row_version,sealed_at
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,'sealed',$9,$10,1,NOW())"#,
    )
    .bind(admission_set_id)
    .bind(Uuid::new_v4())
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(f.stage_run_unit_id)
    .bind(f.scope_snapshot_id)
    .bind(f.organization_id)
    .bind(generation_id)
    .bind(if with_residual { 1_i64 } else { 0_i64 })
    .bind(admission_hash)
    .execute(&mut *tx)
    .await
    .expect("seed sealed admission set");
    if with_residual {
        sqlx::query(
            r#"INSERT INTO verification_admission_members(
                   admission_member_id,admission_set_id,operation_id,stage_execution_id,
                   stage_run_unit_id,scope_snapshot_id,organization_id,generation_member_id,
                   hypothesis_revision_id,disposition,reason_code,
                   semantic_attempt_fingerprint,member_sha256
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'needs_enrichment',
                        'fixture_residual',$10,$11)"#,
        )
        .bind(Uuid::new_v4())
        .bind(admission_set_id)
        .bind(f.operation_id)
        .bind(f.stage_execution_id)
        .bind(f.stage_run_unit_id)
        .bind(f.scope_snapshot_id)
        .bind(f.organization_id)
        .bind(generation_member_id)
        .bind(revision_id)
        .bind(digest('f'))
        .bind(admission_member_hash)
        .execute(&mut *tx)
        .await
        .expect("seed typed admission residual");
    }
    sqlx::query(
        r#"INSERT INTO hypothesis_fixed_point_receipts(
               fixed_point_receipt_id,stable_request_id,consolidation_receipt_id,
               generation_id,open_obligation_set_hash,residual_set_hash,fixed_point_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(generation_id)
    .bind(digest('a'))
    .bind(digest('b'))
    .bind(digest('c'))
    .execute(&mut *tx)
    .await
    .expect("seed fixed-point receipt");
    tx.commit()
        .await
        .expect("commit full closure authority seed");
}

async fn start_and_stop_empty_run(pool: &PgPool, f: &Fixture, with_residual: bool) -> String {
    let running_head: String = sqlx::query_scalar(
        r#"SELECT head_sha256
             FROM register_investigation_run_v1($1,$2,$3,$4,$5,$6,0)"#,
    )
    .bind(f.authority_id)
    .bind(Uuid::new_v4())
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(&f.owning_request_id)
    .bind(f.scope_snapshot_id)
    .fetch_one(pool)
    .await
    .expect("start empty closure fixture run");
    seed_full_closure_authority(pool, f, with_residual).await;
    sqlx::query("SELECT stop_intent_id FROM investigation_request_stop_v1($1,$2,$3,$4,$5,$6,$7,0)")
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(f.authority_id)
        .bind(f.operation_id)
        .bind(f.stage_execution_id)
        .bind(&f.owning_request_id)
        .bind(running_head)
        .execute(pool)
        .await
        .expect("stop empty closure fixture run");
    sqlx::query_scalar("SELECT head_sha256 FROM investigation_run_heads WHERE authority_id=$1")
        .bind(f.authority_id)
        .fetch_one(pool)
        .await
        .expect("load stopped fixture head")
}

#[tokio::test]
#[serial]
async fn full_closure_rejects_a_current_generation_without_an_admission_disposition() {
    let (db, _data_dir) = migrated_db("full-closure-current-admission").await;
    let f = fixture(db.pool(), "full-closure-current-admission").await;
    let running_head: String = sqlx::query_scalar(
        r#"SELECT head_sha256
             FROM register_investigation_run_v1($1,$2,$3,$4,$5,$6,0)"#,
    )
    .bind(f.authority_id)
    .bind(Uuid::new_v4())
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(&f.owning_request_id)
    .bind(f.scope_snapshot_id)
    .fetch_one(db.pool())
    .await
    .expect("start current-admission fixture run");
    seed_full_closure_authority(db.pool(), &f, false).await;

    let newer_generation_id = Uuid::new_v4();
    let empty_member_hash: String = sqlx::query_scalar(
        "SELECT investigation_exact_member_set_hash('hypothesis_generation_members.v1',ARRAY[]::TEXT[])",
    )
    .fetch_one(db.pool())
    .await
    .expect("compute empty generation-member hash");
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin newer generation seed");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("isolate newer generation seed");
    sqlx::query(
        r#"INSERT INTO hypothesis_generations(
               generation_id,operation_id,organization_id,generation_ordinal,
               candidate_snapshot_id,candidate_gate_decision_id,
               candidate_snapshot_authority_hash
           ) VALUES($1,$2,$3,1,$4,$5,$6)"#,
    )
    .bind(newer_generation_id)
    .bind(f.operation_id)
    .bind(f.organization_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(digest('1'))
    .execute(&mut *tx)
    .await
    .expect("seed newer current generation");
    sqlx::query(
        r#"INSERT INTO hypothesis_generation_seals(
               seal_id,generation_id,member_count,member_set_hash,event_count,
               event_set_hash,open_obligation_set_hash,controller_worker_run_id,generation_hash
           ) VALUES($1,$2,0,$3,0,$4,$5,$6,$7)"#,
    )
    .bind(Uuid::new_v4())
    .bind(newer_generation_id)
    .bind(empty_member_hash)
    .bind(digest('2'))
    .bind(digest('3'))
    .bind(f.primary_worker_run_id)
    .bind(digest('4'))
    .execute(&mut *tx)
    .await
    .expect("seal newer current generation without admission");
    tx.commit().await.expect("commit newer generation seed");

    sqlx::query("SELECT stop_intent_id FROM investigation_request_stop_v1($1,$2,$3,$4,$5,$6,$7,0)")
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(f.authority_id)
        .bind(f.operation_id)
        .bind(f.stage_execution_id)
        .bind(&f.owning_request_id)
        .bind(running_head)
        .execute(db.pool())
        .await
        .expect("stop current-admission fixture run");
    let stopped_head: String =
        sqlx::query_scalar("SELECT head_sha256 FROM investigation_run_heads WHERE authority_id=$1")
            .bind(f.authority_id)
            .fetch_one(db.pool())
            .await
            .expect("load stopped current-admission head");
    let error =
        sqlx::query("SELECT closure_id FROM seal_investigation_run_closure_v1($1,$2,$3,$4)")
            .bind(Uuid::new_v4())
            .bind(Uuid::new_v4())
            .bind(f.authority_id)
            .bind(stopped_head)
            .fetch_one(db.pool())
            .await
            .expect_err("every current generation requires an exact admission disposition");
    assert!(error
        .to_string()
        .contains("INVESTIGATION_CLOSURE_GENERATION_ADMISSION_SET_INCOMPLETE"));
}

#[tokio::test]
#[serial]
async fn full_closure_derives_pass_with_gaps_from_typed_residual_rows() {
    let (db, _data_dir) = migrated_db("full-closure-residual").await;
    let f = fixture(db.pool(), "full-closure-residual").await;
    let stopped_head = start_and_stop_empty_run(db.pool(), &f, true).await;
    let closure: (String, i64) = sqlx::query_as(
        r#"SELECT disposition,residual_member_count
             FROM seal_investigation_run_closure_v1($1,$2,$3,$4)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(f.authority_id)
    .bind(stopped_head)
    .fetch_one(db.pool())
    .await
    .expect("DB derives PASS_WITH_GAPS from residual authority");
    assert_eq!(closure, ("pass_with_gaps".to_string(), 1));
}

#[tokio::test]
#[serial]
async fn full_closure_accepts_the_unified_compiler_generation_member_hash_contract() {
    let (db, _data_dir) = migrated_db("full-closure-unified-generation-hash").await;
    let f = fixture(db.pool(), "full-closure-unified-generation-hash").await;
    let stopped_head = start_and_stop_empty_run(db.pool(), &f, true).await;
    let generation_id: Uuid = sqlx::query_scalar(
        r#"SELECT generation_id
              FROM hypothesis_generations
             WHERE operation_id=$1 AND organization_id=$2
             ORDER BY generation_ordinal DESC,generation_id
             LIMIT 1"#,
    )
    .bind(f.operation_id)
    .bind(f.organization_id)
    .fetch_one(db.pool())
    .await
    .expect("load current generation");
    let unified_member_set_hash: String = sqlx::query_scalar(
        r#"SELECT unified_investigation_exact_set_hash(
                       'hypothesis_generation_members.v1',
                       COALESCE(array_agg(member_hash ORDER BY member_hash),ARRAY[]::TEXT[])
                   )
              FROM hypothesis_generation_members
             WHERE generation_id=$1"#,
    )
    .bind(generation_id)
    .fetch_one(db.pool())
    .await
    .expect("compute unified compiler generation-member hash");
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin hash compatibility fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("isolate immutable hash compatibility fixture");
    sqlx::query("UPDATE hypothesis_generation_seals SET member_set_hash=$2 WHERE generation_id=$1")
        .bind(generation_id)
        .bind(unified_member_set_hash)
        .execute(&mut *tx)
        .await
        .expect("represent the unified compiler seal format");
    tx.commit()
        .await
        .expect("commit unified compiler hash fixture");

    let closure: (String, i64) = sqlx::query_as(
        r#"SELECT disposition,residual_member_count
             FROM seal_investigation_run_closure_v1($1,$2,$3,$4)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(f.authority_id)
    .bind(stopped_head)
    .fetch_one(db.pool())
    .await
    .expect("closure accepts the unified compiler exact-set envelope");
    assert_eq!(closure, ("pass_with_gaps".to_string(), 1));
}

#[tokio::test]
#[serial]
async fn full_closure_rejects_open_fuel_reservation() {
    let (db, _data_dir) = migrated_db("full-closure-open-fuel").await;
    let f = fixture(db.pool(), "full-closure-open-fuel").await;
    let running_head: String = sqlx::query_scalar(
        r#"SELECT head_sha256
             FROM register_investigation_run_v1($1,$2,$3,$4,$5,$6,0)"#,
    )
    .bind(f.authority_id)
    .bind(Uuid::new_v4())
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(&f.owning_request_id)
    .bind(f.scope_snapshot_id)
    .fetch_one(db.pool())
    .await
    .expect("start open-fuel fixture run");
    seed_full_closure_authority(db.pool(), &f, false).await;
    let budget_id = Uuid::new_v4();
    let reservation_id = Uuid::new_v4();
    let mut tx = db.pool().begin().await.expect("begin open fuel fixture");
    sqlx::query(
        r#"INSERT INTO investigation_fuel_budgets(
               budget_id,stable_request_id,authority_id,operation_id,
               stage_execution_id,owning_stage_run_request_id,scope_kind,owner_id
           ) VALUES($1,$2,$3,$4,$5,$6,'operation',$4)"#,
    )
    .bind(budget_id)
    .bind(Uuid::new_v4())
    .bind(f.authority_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(&f.owning_request_id)
    .execute(&mut *tx)
    .await
    .expect("insert operation fuel budget");
    sqlx::query(
        "INSERT INTO investigation_fuel_budget_heads(budget_id,axis,limit_amount,reserved_amount) VALUES($1,'campaign',1,1)",
    )
    .bind(budget_id)
    .execute(&mut *tx)
    .await
    .expect("insert reserved fuel head");
    sqlx::query(
        r#"INSERT INTO investigation_fuel_reservations(
               reservation_id,budget_id,axis,amount,work_key_sha256,state,reservation_epoch
           ) VALUES($1,$2,'campaign',1,$3,'reserved',1)"#,
    )
    .bind(reservation_id)
    .bind(budget_id)
    .bind(digest('1'))
    .execute(&mut *tx)
    .await
    .expect("insert open reservation");
    sqlx::query(
        r#"INSERT INTO investigation_fuel_reservation_events(
               event_id,stable_request_id,reservation_id,budget_id,axis,
               event_ordinal,from_state,to_state,amount,event_sha256
           ) VALUES($1,$2,$3,$4,'campaign',0,NULL,'reserved',1,$5)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(reservation_id)
    .bind(budget_id)
    .bind(digest('2'))
    .execute(&mut *tx)
    .await
    .expect("insert open reservation event");
    tx.commit().await.expect("commit open fuel fixture");
    sqlx::query("SELECT stop_intent_id FROM investigation_request_stop_v1($1,$2,$3,$4,$5,$6,$7,0)")
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(f.authority_id)
        .bind(f.operation_id)
        .bind(f.stage_execution_id)
        .bind(&f.owning_request_id)
        .bind(running_head)
        .execute(db.pool())
        .await
        .expect("stop open-fuel fixture run");
    let stopped_head: String =
        sqlx::query_scalar("SELECT head_sha256 FROM investigation_run_heads WHERE authority_id=$1")
            .bind(f.authority_id)
            .fetch_one(db.pool())
            .await
            .expect("read stopped open-fuel head");
    let error =
        sqlx::query("SELECT closure_id FROM seal_investigation_run_closure_v1($1,$2,$3,$4)")
            .bind(Uuid::new_v4())
            .bind(Uuid::new_v4())
            .bind(f.authority_id)
            .bind(stopped_head)
            .fetch_one(db.pool())
            .await
            .expect_err("open fuel must reject closure");
    assert!(error
        .to_string()
        .contains("INVESTIGATION_CLOSURE_FUEL_NOT_SETTLED"));
}

async fn closure_error_for_unsettled_dispatch(label: &str, outcome: Option<&str>) -> String {
    let (db, _data_dir) = migrated_db(label).await;
    let f = fixture(db.pool(), label).await;
    let running_head: String = sqlx::query_scalar(
        r#"SELECT head_sha256
             FROM register_investigation_run_v1($1,$2,$3,$4,$5,$6,0)"#,
    )
    .bind(f.authority_id)
    .bind(Uuid::new_v4())
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(&f.owning_request_id)
    .bind(f.scope_snapshot_id)
    .fetch_one(db.pool())
    .await
    .expect("start dispatch fixture run");
    seed_full_closure_authority(db.pool(), &f, false).await;
    let task_plan_id = Uuid::new_v4();
    let subtask_id = Uuid::new_v4();
    let subtask_member_hash = digest('a');
    let dispatch_id = Uuid::new_v4();
    let dispatch_receipt_hash = digest('3');
    let subtask_set_hash: String = sqlx::query_scalar(
        "SELECT unified_investigation_exact_set_hash('investigation_pentagi_subtasks.v1',ARRAY[$1]::TEXT[])",
    )
    .bind(&subtask_member_hash)
    .fetch_one(db.pool())
    .await
    .expect("compute empty subtask hash");
    let dispatch_set_hash: String = sqlx::query_scalar(
        "SELECT unified_investigation_exact_set_hash('pentagi_logical_dispatch_receipts.v1',ARRAY[$1]::TEXT[])",
    )
    .bind(&dispatch_receipt_hash)
    .fetch_one(db.pool())
    .await
    .expect("compute dispatch set hash");
    let empty_pipeline_hash: String = sqlx::query_scalar(
        "SELECT unified_investigation_exact_set_hash('investigation_pentagi_pipeline_events.v1',ARRAY[]::TEXT[])",
    )
    .fetch_one(db.pool())
    .await
    .expect("compute empty pipeline hash");
    let mut tx = db.pool().begin().await.expect("begin dispatch fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("isolate dispatch fixture");
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_task_plans(
               task_plan_id,stable_request_id,run_request_id,authority_id,stage_team_plan_id,
               operation_id,stage_execution_id,owning_stage_run_request_id,
               stage_run_unit_id,scope_snapshot_id,organization_id,subject_kind,
               subject_id,subject_fingerprint_sha256,task_plan_version,task_plan_sha256,
               allowed_role_catalog,cognitive_tool_envelope_sha256,status,
               subtask_count,subtask_set_sha256,row_version,sealed_at
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,'analysis_attempt',$12,$13,1,$14,
                    '["primary"]'::JSONB,$15,'sealed',1,$16,1,NOW())"#,
    )
    .bind(task_plan_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(f.authority_id)
    .bind(f.stage_team_plan_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(&f.owning_request_id)
    .bind(f.stage_run_unit_id)
    .bind(f.scope_snapshot_id)
    .bind(f.organization_id)
    .bind(Uuid::new_v4())
    .bind(digest('1'))
    .bind(digest('2'))
    .bind(digest('4'))
    .bind(subtask_set_hash)
    .execute(&mut *tx)
    .await
    .expect("seed sealed task plan");
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_subtasks(
               subtask_id,task_plan_id,authority_id,operation_id,stage_execution_id,
               stage_run_unit_id,organization_id,subtask_ordinal,label,runnable,
               input_manifest_sha256,expected_output_schema,member_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,0,'non-runnable fixture',FALSE,$8,
                    'InvestigationFixtureV1',$9)"#,
    )
    .bind(subtask_id)
    .bind(task_plan_id)
    .bind(f.authority_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(f.stage_run_unit_id)
    .bind(f.organization_id)
    .bind(digest('b'))
    .bind(subtask_member_hash)
    .execute(&mut *tx)
    .await
    .expect("seed non-runnable subtask denominator");
    sqlx::query(
        r#"INSERT INTO pentagi_logical_dispatch_receipts(
               dispatch_receipt_id,stable_request_id,logical_dispatch_key_sha256,
               task_plan_id,dispatch_ordinal,actor_kind,stage_work_item_id,worker_run_id,
               operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
               organization_id,transcript_request_id,snapshot_sha256,receipt_sha256
           ) VALUES($1,$2,$3,$4,0,'primary',$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"#,
    )
    .bind(dispatch_id)
    .bind(Uuid::new_v4())
    .bind(digest('5'))
    .bind(task_plan_id)
    .bind(f.primary_work_item_id)
    .bind(f.primary_worker_run_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(f.stage_run_unit_id)
    .bind(f.scope_snapshot_id)
    .bind(f.organization_id)
    .bind(format!("dispatch-{label}"))
    .bind(digest('6'))
    .bind(&dispatch_receipt_hash)
    .execute(&mut *tx)
    .await
    .expect("seed logical primary dispatch");
    if let Some(outcome) = outcome {
        sqlx::query(
            r#"INSERT INTO pentagi_logical_dispatch_attempts(
                   dispatch_attempt_id,stable_request_id,dispatch_receipt_id,attempt_epoch,
                   lease_token,fence_sha256,outcome,result_sha256
               ) VALUES($1,$2,$3,0,$4,$5,$6,$7)"#,
        )
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(dispatch_id)
        .bind(Uuid::new_v4())
        .bind(digest('7'))
        .bind(outcome)
        .bind(digest('8'))
        .execute(&mut *tx)
        .await
        .expect("seed dispatch attempt outcome");
    }
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_delegation_census_seals(
               census_seal_id,stable_request_id,task_plan_id,primary_dispatch_receipt_id,
               primary_worker_run_id,runnable_subtask_count,runnable_subtask_set_sha256,
               dispatch_count,dispatch_set_sha256,pipeline_event_count,
               pipeline_event_set_sha256,seal_sha256
           ) VALUES($1,$2,$3,$4,$5,0,$6,1,$7,0,$8,$9)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(task_plan_id)
    .bind(dispatch_id)
    .bind(f.primary_worker_run_id)
    .bind(sqlx::query_scalar::<_, String>(
        "SELECT unified_investigation_exact_set_hash('investigation_pentagi_subtasks.v1',ARRAY[]::TEXT[])",
    )
    .fetch_one(db.pool())
    .await
    .expect("compute runnable set hash"))
    .bind(dispatch_set_hash)
    .bind(empty_pipeline_hash)
    .bind(digest('9'))
    .execute(&mut *tx)
    .await
    .expect("seed sealed delegation census");
    tx.commit().await.expect("commit dispatch fixture");
    sqlx::query("SELECT stop_intent_id FROM investigation_request_stop_v1($1,$2,$3,$4,$5,$6,$7,0)")
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(f.authority_id)
        .bind(f.operation_id)
        .bind(f.stage_execution_id)
        .bind(&f.owning_request_id)
        .bind(running_head)
        .execute(db.pool())
        .await
        .expect("stop dispatch fixture run");
    let stopped_head: String =
        sqlx::query_scalar("SELECT head_sha256 FROM investigation_run_heads WHERE authority_id=$1")
            .bind(f.authority_id)
            .fetch_one(db.pool())
            .await
            .expect("read stopped dispatch head");
    sqlx::query("SELECT closure_id FROM seal_investigation_run_closure_v1($1,$2,$3,$4)")
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(f.authority_id)
        .bind(stopped_head)
        .fetch_one(db.pool())
        .await
        .expect_err("unsettled dispatch must reject closure")
        .to_string()
}

#[tokio::test]
#[serial]
async fn full_closure_rejects_missing_and_unknown_dispatch_attempts() {
    let missing = closure_error_for_unsettled_dispatch("full-closure-missing-dispatch", None).await;
    assert!(missing.contains("INVESTIGATION_CLOSURE_DELEGATION_NOT_CLOSED"));
    let unknown =
        closure_error_for_unsettled_dispatch("full-closure-unknown-dispatch", Some("unknown_held"))
            .await;
    assert!(unknown.contains("INVESTIGATION_CLOSURE_DELEGATION_NOT_CLOSED"));
}

#[tokio::test]
#[serial]
async fn pentagi_census_stop_and_closure_are_exact_cas_fenced_and_replay_stable() {
    let (db, _data_dir) = migrated_db("pentagi-stop-closure").await;
    let f = fixture(db.pool(), "pentagi-stop-closure").await;
    let start_request_id = Uuid::new_v4();
    let run: (Uuid, String, i64) = sqlx::query_as(
        r#"SELECT authority_id,head_sha256,head_version
             FROM register_investigation_run_v1($1,$2,$3,$4,$5,$6,0)"#,
    )
    .bind(f.authority_id)
    .bind(start_request_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(&f.owning_request_id)
    .bind(f.scope_snapshot_id)
    .fetch_one(db.pool())
    .await
    .expect("register durable Investigation run");
    let replay: (Uuid, String, i64) = sqlx::query_as(
        r#"SELECT authority_id,head_sha256,head_version
             FROM register_investigation_run_v1($1,$2,$3,$4,$5,$6,0)"#,
    )
    .bind(f.authority_id)
    .bind(start_request_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(&f.owning_request_id)
    .bind(f.scope_snapshot_id)
    .fetch_one(db.pool())
    .await
    .expect("replay durable Investigation run");
    assert_eq!(run, replay);
    seed_full_closure_authority(db.pool(), &f, false).await;

    let task_plan_id = Uuid::new_v4();
    let subject_id = Uuid::new_v4();
    let task_run_request_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO pentagi_task_run_requests(
               run_request_id,stable_request_id,task_plan_id,authority_id,operation_id,
               stage_execution_id,owning_stage_run_request_id,stage_run_unit_id,
               organization_id,subject_kind,subject_id,subject_fingerprint_sha256,
               request_sha256
           ) VALUES($1,$2,NULL,$3,$4,$5,$6,$7,$8,'analysis_attempt',$9,$10,$11)"#,
    )
    .bind(task_run_request_id)
    .bind(Uuid::new_v4())
    .bind(f.authority_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(&f.owning_request_id)
    .bind(f.stage_run_unit_id)
    .bind(f.organization_id)
    .bind(subject_id)
    .bind(digest('8'))
    .bind(digest('b'))
    .execute(db.pool())
    .await
    .expect("insert scheduler-owned PentAGI run request before its plan");
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_task_plans(
               task_plan_id,stable_request_id,run_request_id,authority_id,stage_team_plan_id,
               operation_id,stage_execution_id,owning_stage_run_request_id,
               stage_run_unit_id,scope_snapshot_id,organization_id,subject_kind,
               subject_id,subject_fingerprint_sha256,task_plan_version,task_plan_sha256,
               allowed_role_catalog,cognitive_tool_envelope_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,'analysis_attempt',$12,$13,1,$14,$15,$16)"#,
    )
    .bind(task_plan_id)
    .bind(Uuid::new_v4())
    .bind(task_run_request_id)
    .bind(f.authority_id)
    .bind(f.stage_team_plan_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(&f.owning_request_id)
    .bind(f.stage_run_unit_id)
    .bind(f.scope_snapshot_id)
    .bind(f.organization_id)
    .bind(subject_id)
    .bind(digest('8'))
    .bind(digest('9'))
    .bind(serde_json::json!(["primary", "researcher"]))
    .bind(digest('a'))
    .execute(db.pool())
    .await
    .expect("insert tagged PentAGI plan");
    let subtask_id = Uuid::new_v4();
    let primary_transcript_request_id = format!(
        "{}::team:{}::lead:{}",
        f.owning_request_id, f.organization_id, f.primary_worker_run_id
    );
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_subtasks(
               subtask_id,task_plan_id,authority_id,operation_id,stage_execution_id,
               stage_run_unit_id,organization_id,subtask_ordinal,label,runnable,
               input_manifest_sha256,expected_output_schema,member_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,0,'Analyze exact bounded snapshot',TRUE,$8,
                    'InvestigationPentagiWorkerResultV1',$9)"#,
    )
    .bind(subtask_id)
    .bind(task_plan_id)
    .bind(f.authority_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(f.stage_run_unit_id)
    .bind(f.organization_id)
    .bind(digest('c'))
    .bind(digest('d'))
    .execute(db.pool())
    .await
    .expect("insert runnable subtask");

    let primary_dispatch_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO pentagi_logical_dispatch_receipts(
               dispatch_receipt_id,stable_request_id,logical_dispatch_key_sha256,
               task_plan_id,dispatch_ordinal,actor_kind,stage_work_item_id,
               worker_run_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,transcript_request_id,
               snapshot_sha256,receipt_sha256
           ) VALUES($1,$2,$3,$4,0,'primary',$5,$6,$7,$8,$9,$10,$11,
                    $12,$13,$14)"#,
    )
    .bind(primary_dispatch_id)
    .bind(Uuid::new_v4())
    .bind(digest('e'))
    .bind(task_plan_id)
    .bind(f.primary_work_item_id)
    .bind(f.primary_worker_run_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(f.stage_run_unit_id)
    .bind(f.scope_snapshot_id)
    .bind(f.organization_id)
    .bind(&primary_transcript_request_id)
    .bind(digest('f'))
    .bind(digest('1'))
    .execute(db.pool())
    .await
    .expect("insert the single Primary dispatch");
    let worker_transcript_request_id = format!("dispatch-tool-0::worker:{}", f.worker_run_id);
    sqlx::query(
        r#"INSERT INTO pentagi_logical_dispatch_attempts(
               dispatch_attempt_id,stable_request_id,dispatch_receipt_id,attempt_epoch,
               lease_token,fence_sha256,outcome,result_sha256
           ) VALUES($1,$2,$3,0,$4,$5,'completed',$6)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(primary_dispatch_id)
    .bind(Uuid::new_v4())
    .bind(digest('2'))
    .bind(digest('3'))
    .execute(db.pool())
    .await
    .expect("terminalize Primary dispatch attempt");

    let second_primary_error = sqlx::query(
        r#"INSERT INTO pentagi_logical_dispatch_receipts(
               dispatch_receipt_id,stable_request_id,logical_dispatch_key_sha256,
               task_plan_id,dispatch_ordinal,actor_kind,stage_work_item_id,
               worker_run_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,transcript_request_id,
               snapshot_sha256,receipt_sha256
           ) VALUES($1,$2,$3,$4,1,'primary',$5,$6,$7,$8,$9,$10,$11,
                    $12,$13,$14)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(digest('4'))
    .bind(task_plan_id)
    .bind(f.spare_primary_work_item_id)
    .bind(f.spare_primary_worker_run_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(f.stage_run_unit_id)
    .bind(f.scope_snapshot_id)
    .bind(f.organization_id)
    .bind(format!(
        "{}::team:{}::lead:{}",
        f.owning_request_id, f.organization_id, f.spare_primary_worker_run_id
    ))
    .bind(digest('f'))
    .bind(digest('5'))
    .execute(db.pool())
    .await
    .expect_err("a task cannot have a second Primary");
    assert!(second_primary_error
        .to_string()
        .contains("pentagi_one_primary_per_task_plan"));

    let (runnable_count, runnable_hash) = exact_set_hash(
        db.pool(),
        "investigation_pentagi_runnable_subtasks.v1",
        "investigation_pentagi_subtasks",
        "member_sha256",
        "subtask_ordinal",
        "task_plan_id",
        task_plan_id,
    )
    .await;
    let (primary_dispatch_count, primary_dispatch_hash) = exact_set_hash(
        db.pool(),
        "pentagi_logical_dispatch_receipts.v1",
        "pentagi_logical_dispatch_receipts",
        "receipt_sha256",
        "dispatch_receipt_id",
        "task_plan_id",
        task_plan_id,
    )
    .await;
    let (_, empty_pipeline_hash): (i64, String) = sqlx::query_as(
        "SELECT 0::BIGINT,unified_investigation_exact_set_hash('investigation_pentagi_pipeline_events.v1',ARRAY[]::TEXT[])",
    )
    .fetch_one(db.pool())
    .await
    .expect("compute empty pipeline census");
    let missing_worker_error = sqlx::query(
        r#"INSERT INTO investigation_pentagi_delegation_census_seals(
               census_seal_id,stable_request_id,task_plan_id,primary_dispatch_receipt_id,
               primary_worker_run_id,runnable_subtask_count,runnable_subtask_set_sha256,
               dispatch_count,dispatch_set_sha256,pipeline_event_count,
               pipeline_event_set_sha256,seal_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(task_plan_id)
    .bind(primary_dispatch_id)
    .bind(f.primary_worker_run_id)
    .bind(runnable_count)
    .bind(&runnable_hash)
    .bind(primary_dispatch_count)
    .bind(primary_dispatch_hash)
    .bind(0_i64)
    .bind(&empty_pipeline_hash)
    .bind(digest('6'))
    .execute(db.pool())
    .await
    .expect_err("Primary-only task cannot seal its delegation census");
    assert!(missing_worker_error
        .to_string()
        .contains("PENTAGI_RUNNABLE_SUBTASK_REQUIRES_DISTINCT_WORKER"));

    let worker_dispatch_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO pentagi_logical_dispatch_receipts(
               dispatch_receipt_id,stable_request_id,logical_dispatch_key_sha256,
               task_plan_id,subtask_id,parent_dispatch_receipt_id,dispatch_ordinal,
               actor_kind,stage_work_item_id,stage_worker_request_id,worker_run_id,
               operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
               organization_id,transcript_request_id,parent_actor_transcript_request_id,
               parent_dispatch_tool_request_id,snapshot_sha256,receipt_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,0,'worker',$7,$8,$9,$10,$11,$12,$13,$14,
                    $15,$16,'dispatch-tool-0',$17,$18)"#,
    )
    .bind(worker_dispatch_id)
    .bind(Uuid::new_v4())
    .bind(digest('7'))
    .bind(task_plan_id)
    .bind(subtask_id)
    .bind(primary_dispatch_id)
    .bind(f.worker_work_item_id)
    .bind(f.worker_request_id)
    .bind(f.worker_run_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(f.stage_run_unit_id)
    .bind(f.scope_snapshot_id)
    .bind(f.organization_id)
    .bind(&worker_transcript_request_id)
    .bind(&primary_transcript_request_id)
    .bind(digest('f'))
    .bind(digest('8'))
    .execute(db.pool())
    .await
    .expect("insert independent worker delegation");
    sqlx::query(
        r#"INSERT INTO pentagi_logical_dispatch_attempts(
               dispatch_attempt_id,stable_request_id,dispatch_receipt_id,attempt_epoch,
               lease_token,fence_sha256,outcome,result_sha256
           ) VALUES($1,$2,$3,0,$4,$5,'completed',$6)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(worker_dispatch_id)
    .bind(Uuid::new_v4())
    .bind(digest('9'))
    .bind(digest('a'))
    .execute(db.pool())
    .await
    .expect("terminalize worker dispatch attempt");

    let (pipeline_count, pipeline_hash) =
        seed_refiner_plan_ledger(db.pool(), task_plan_id, vec![subtask_id]).await;
    assert_eq!(pipeline_count, 3);

    let duplicate_dispatch_error = sqlx::query(
        r#"INSERT INTO pentagi_logical_dispatch_receipts(
               dispatch_receipt_id,stable_request_id,logical_dispatch_key_sha256,
               task_plan_id,subtask_id,parent_dispatch_receipt_id,dispatch_ordinal,
               actor_kind,stage_work_item_id,stage_worker_request_id,worker_run_id,
               operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
               organization_id,transcript_request_id,parent_actor_transcript_request_id,
               parent_dispatch_tool_request_id,snapshot_sha256,receipt_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,0,'worker',$7,$8,$9,$10,$11,$12,$13,$14,
                    $15,$16,'dispatch-tool-0',$17,$18)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(digest('7'))
    .bind(task_plan_id)
    .bind(subtask_id)
    .bind(primary_dispatch_id)
    .bind(f.worker_work_item_id)
    .bind(f.worker_request_id)
    .bind(f.worker_run_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(f.stage_run_unit_id)
    .bind(f.scope_snapshot_id)
    .bind(f.organization_id)
    .bind(&worker_transcript_request_id)
    .bind(&primary_transcript_request_id)
    .bind(digest('f'))
    .bind(digest('b'))
    .execute(db.pool())
    .await
    .expect_err("duplicate logical dispatch must be rejected");
    assert!(
        duplicate_dispatch_error
            .to_string()
            .contains("logical_dispatch_key_sha256"),
        "{duplicate_dispatch_error}"
    );

    let (dispatch_count, dispatch_hash) = exact_set_hash(
        db.pool(),
        "pentagi_logical_dispatch_receipts.v1",
        "pentagi_logical_dispatch_receipts",
        "receipt_sha256",
        "dispatch_receipt_id",
        "task_plan_id",
        task_plan_id,
    )
    .await;
    let exact_set_error = sqlx::query(
        r#"INSERT INTO investigation_pentagi_delegation_census_seals(
               census_seal_id,stable_request_id,task_plan_id,primary_dispatch_receipt_id,
               primary_worker_run_id,runnable_subtask_count,runnable_subtask_set_sha256,
               dispatch_count,dispatch_set_sha256,pipeline_event_count,
               pipeline_event_set_sha256,seal_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(task_plan_id)
    .bind(primary_dispatch_id)
    .bind(f.primary_worker_run_id)
    .bind(runnable_count)
    .bind(&runnable_hash)
    .bind(dispatch_count + 1)
    .bind(&dispatch_hash)
    .bind(pipeline_count)
    .bind(&pipeline_hash)
    .bind(digest('c'))
    .execute(db.pool())
    .await
    .expect_err("caller cannot forge delegation census cardinality");
    assert!(exact_set_error
        .to_string()
        .contains("PENTAGI_DELEGATION_CENSUS_EXACT_SET_MISMATCH"));

    let census_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_delegation_census_seals(
               census_seal_id,stable_request_id,task_plan_id,primary_dispatch_receipt_id,
               primary_worker_run_id,runnable_subtask_count,runnable_subtask_set_sha256,
               dispatch_count,dispatch_set_sha256,pipeline_event_count,
               pipeline_event_set_sha256,seal_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)"#,
    )
    .bind(census_id)
    .bind(Uuid::new_v4())
    .bind(task_plan_id)
    .bind(primary_dispatch_id)
    .bind(f.primary_worker_run_id)
    .bind(runnable_count)
    .bind(&runnable_hash)
    .bind(dispatch_count)
    .bind(&dispatch_hash)
    .bind(pipeline_count)
    .bind(&pipeline_hash)
    .bind(digest('d'))
    .execute(db.pool())
    .await
    .expect("seal exact delegation census");
    let (subtask_count, subtask_hash) = exact_set_hash(
        db.pool(),
        "investigation_pentagi_subtasks.v1",
        "investigation_pentagi_subtasks",
        "member_sha256",
        "subtask_ordinal",
        "task_plan_id",
        task_plan_id,
    )
    .await;
    sqlx::query(
        r#"UPDATE investigation_pentagi_task_plans
              SET status='sealed',subtask_count=$2,subtask_set_sha256=$3,
                  row_version=1,sealed_at=NOW()
            WHERE task_plan_id=$1 AND status='open' AND row_version=0"#,
    )
    .bind(task_plan_id)
    .bind(subtask_count)
    .bind(subtask_hash)
    .execute(db.pool())
    .await
    .expect("seal task plan after exact delegation census");

    let completed_before_stop_work_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO investigation_run_work_items(
               work_id,stable_work_key_sha256,authority_id,operation_id,
               stage_execution_id,owning_stage_run_request_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,work_kind,external_identity_sha256,
               current_state,observed_stop_epoch
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'read_session',$10,'queued',0)"#,
    )
    .bind(completed_before_stop_work_id)
    .bind(digest('1'))
    .bind(f.authority_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(&f.owning_request_id)
    .bind(f.stage_run_unit_id)
    .bind(f.scope_snapshot_id)
    .bind(f.organization_id)
    .bind(digest('2'))
    .execute(db.pool())
    .await
    .expect("register read work before stop");
    for (ordinal, from, to, nibble) in [
        (1_i64, "queued", "running", '3'),
        (2_i64, "running", "completed", '4'),
    ] {
        sqlx::query(
            r#"INSERT INTO investigation_run_work_state_events(
                   event_id,stable_request_id,work_id,expected_head_version,event_ordinal,
                   from_state,to_state,observed_stop_epoch,reason_code,event_sha256
               ) VALUES($1,$2,$3,$4,$5,$6,$7,0,'read_session_terminal',$8)"#,
        )
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(completed_before_stop_work_id)
        .bind(ordinal - 1)
        .bind(ordinal)
        .bind(from)
        .bind(to)
        .bind(digest(nibble))
        .execute(db.pool())
        .await
        .expect("terminalize read work before stop");
    }

    let work_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO investigation_run_work_items(
               work_id,stable_work_key_sha256,authority_id,operation_id,
               stage_execution_id,owning_stage_run_request_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,work_kind,external_identity_sha256,
               current_state,observed_stop_epoch
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'pentagi_subtask',$10,'running',0)"#,
    )
    .bind(work_id)
    .bind(digest('e'))
    .bind(f.authority_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(&f.owning_request_id)
    .bind(f.stage_run_unit_id)
    .bind(f.scope_snapshot_id)
    .bind(f.organization_id)
    .bind(digest('f'))
    .execute(db.pool())
    .await
    .expect("register active work before stop");

    let stop_intent_id = Uuid::new_v4();
    let stop_key = Uuid::new_v4();
    let stop: (Uuid, i64, i64, String) = sqlx::query_as(
        r#"SELECT stop_intent_id,stop_epoch,frozen_work_count,receipt_sha256
             FROM investigation_request_stop_v1($1,$2,$3,$4,$5,$6,$7,0)"#,
    )
    .bind(stop_intent_id)
    .bind(stop_key)
    .bind(f.authority_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(&f.owning_request_id)
    .bind(&run.1)
    .fetch_one(db.pool())
    .await
    .expect("freeze exact open-work set and stop admission");
    assert_eq!(stop.1, 1);
    assert_eq!(stop.2, 1);
    let stop_replay: (Uuid, i64, i64, String) = sqlx::query_as(
        r#"SELECT stop_intent_id,stop_epoch,frozen_work_count,receipt_sha256
             FROM investigation_request_stop_v1($1,$2,$3,$4,$5,$6,$7,0)"#,
    )
    .bind(stop_intent_id)
    .bind(stop_key)
    .bind(f.authority_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(&f.owning_request_id)
    .bind(&run.1)
    .fetch_one(db.pool())
    .await
    .expect("replay returns the same stop receipt");
    assert_eq!(stop, stop_replay);
    let start_after_stop: (Uuid, String, bool, i64, i64) = sqlx::query_as(
        r#"SELECT authority_id,run_state,admission_open,stop_epoch,change_seq
             FROM register_investigation_run_v1($1,$2,$3,$4,$5,$6,0)"#,
    )
    .bind(f.authority_id)
    .bind(start_request_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(&f.owning_request_id)
    .bind(f.scope_snapshot_id)
    .fetch_one(db.pool())
    .await
    .expect("start replay after stop returns the mutable current head");
    assert_eq!(start_after_stop.0, f.authority_id);
    assert_eq!(start_after_stop.1, "stop_pending");
    assert!(!start_after_stop.2);
    assert_eq!(start_after_stop.3, 1);
    assert!(start_after_stop.4 > 0);

    let late_work_error = sqlx::query(
        r#"INSERT INTO investigation_run_work_items(
               work_id,stable_work_key_sha256,authority_id,operation_id,
               stage_execution_id,owning_stage_run_request_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,work_kind,external_identity_sha256,
               current_state,observed_stop_epoch
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'campaign',$10,'queued',1)"#,
    )
    .bind(Uuid::new_v4())
    .bind(digest('1'))
    .bind(f.authority_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(&f.owning_request_id)
    .bind(f.stage_run_unit_id)
    .bind(f.scope_snapshot_id)
    .bind(f.organization_id)
    .bind(digest('2'))
    .execute(db.pool())
    .await
    .expect_err("stop fence prevents concurrent late work");
    assert!(late_work_error
        .to_string()
        .contains("INVESTIGATION_WORK_ADMISSION_CLOSED"));

    let stopped_head: String =
        sqlx::query_scalar("SELECT head_sha256 FROM investigation_run_heads WHERE authority_id=$1")
            .bind(f.authority_id)
            .fetch_one(db.pool())
            .await
            .expect("read stopped head");
    let closure_id = Uuid::new_v4();
    let closure_request_id = Uuid::new_v4();
    let early_closure_error =
        sqlx::query("SELECT closure_id FROM seal_investigation_run_closure_v1($1,$2,$3,$4)")
            .bind(closure_id)
            .bind(closure_request_id)
            .bind(f.authority_id)
            .bind(&stopped_head)
            .fetch_one(db.pool())
            .await
            .expect_err("closure cannot pass while frozen work is active");
    assert!(early_closure_error
        .to_string()
        .contains("INVESTIGATION_CLOSURE_WORK_NOT_DRAINED"));

    for (ordinal, from, to) in [
        (1_i64, "running", "stop_pending"),
        (2_i64, "stop_pending", "draining"),
        (3_i64, "draining", "cancelled"),
    ] {
        sqlx::query(
            r#"INSERT INTO investigation_run_work_state_events(
                   event_id,stable_request_id,work_id,expected_head_version,event_ordinal,
                   from_state,to_state,observed_stop_epoch,reason_code,event_sha256
               ) VALUES($1,$2,$3,$4,$5,$6,$7,1,'stage_stop',$8)"#,
        )
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(work_id)
        .bind(ordinal - 1)
        .bind(ordinal)
        .bind(from)
        .bind(to)
        .bind(digest(
            char::from_digit((ordinal + 3) as u32, 16).expect("hash nibble"),
        ))
        .execute(db.pool())
        .await
        .expect("drain frozen work through CAS events");
    }
    let closure: (Uuid, i64, i64, String, String) = sqlx::query_as(
        r#"SELECT closure_id,work_total_count,delegation_logical_dispatch_count,
                  disposition,closure_sha256
             FROM seal_investigation_run_closure_v1($1,$2,$3,$4)"#,
    )
    .bind(closure_id)
    .bind(closure_request_id)
    .bind(f.authority_id)
    .bind(&stopped_head)
    .bind(digest('3'))
    .fetch_one(db.pool())
    .await
    .expect("seal deterministic closure after exact drain");
    assert_eq!(closure.0, closure_id);
    assert_eq!(closure.1, 2);
    assert_eq!(closure.2, 2);
    assert_eq!(closure.3, "pass");
    let closure_replay: (Uuid, i64, i64, String, String) = sqlx::query_as(
        r#"SELECT closure_id,work_total_count,delegation_logical_dispatch_count,
                  disposition,closure_sha256
             FROM seal_investigation_run_closure_v1($1,$2,$3,$4)"#,
    )
    .bind(closure_id)
    .bind(closure_request_id)
    .bind(f.authority_id)
    .bind(&stopped_head)
    .bind(digest('3'))
    .fetch_one(db.pool())
    .await
    .expect("closure replay returns the same receipt");
    assert_eq!(closure, closure_replay);
    let final_state: (String, bool) = sqlx::query_as(
        "SELECT run_state,admission_open FROM investigation_run_heads WHERE authority_id=$1",
    )
    .bind(f.authority_id)
    .fetch_one(db.pool())
    .await
    .expect("read closed run head");
    assert_eq!(final_state, ("closed".to_string(), false));

    let publication_id = Uuid::new_v4();
    let publication_request_id = Uuid::new_v4();
    let publication: (Uuid, Uuid, i64, String, String) = sqlx::query_as(
        r#"SELECT publication_id,closure_id,member_count,disposition,publication_sha256
             FROM publish_investigation_stage_closure_v1($1,$2,$3)"#,
    )
    .bind(publication_id)
    .bind(publication_request_id)
    .bind(closure_id)
    .fetch_one(db.pool())
    .await
    .expect("publish closure into the generic stage runtime");
    assert_eq!(publication.0, publication_id);
    assert_eq!(publication.1, closure_id);
    assert_eq!(publication.2, 1);
    assert_eq!(publication.3, "pass");
    assert!(publication.4.starts_with("sha256:"));
    let published_unit: (String, serde_json::Value, i64) =
        sqlx::query_as("SELECT status,pass_watermark,row_version FROM stage_run_units WHERE id=$1")
            .bind(f.stage_run_unit_id)
            .fetch_one(db.pool())
            .await
            .expect("load closure-published Unit");
    assert_eq!(published_unit.0, "passed");
    assert_eq!(
        published_unit.1["publication_id"],
        publication_id.to_string()
    );
    let completion: (String, String) = sqlx::query_as(
        "SELECT stage_kind,stage_run_id FROM org_stage_completions
          WHERE organization_id=$1 AND stage_kind='investigation'",
    )
    .bind(f.organization_id)
    .fetch_one(db.pool())
    .await
    .expect("load Investigation org completion");
    assert_eq!(
        completion,
        ("investigation".to_string(), f.operation_id.to_string())
    );
    let reloaded = golish_db::repo::unified_investigation_runtime::PgUnifiedInvestigationRuntimeRepository::new(
        std::sync::Arc::new(db.pool().clone()),
    )
    .load_closure_publication_for_operation(f.operation_id)
    .await
    .expect("reload validated Investigation closure publication")
    .expect("published closure must exist");
    assert_eq!(reloaded.publication.publication_id, publication_id);
    assert_eq!(reloaded.publication.closure_id, closure_id);
    assert_eq!(reloaded.members.len(), 1);
    assert_eq!(reloaded.members[0].organization_id, f.organization_id);
    let replay: (Uuid, Uuid, i64, String, String) = sqlx::query_as(
        r#"SELECT publication_id,closure_id,member_count,disposition,publication_sha256
             FROM publish_investigation_stage_closure_v1($1,$2,$3)"#,
    )
    .bind(publication_id)
    .bind(publication_request_id)
    .bind(closure_id)
    .fetch_one(db.pool())
    .await
    .expect("replay exact closure publication");
    assert_eq!(publication, replay);
    let replayed_unit_version: i64 =
        sqlx::query_scalar("SELECT row_version FROM stage_run_units WHERE id=$1")
            .bind(f.stage_run_unit_id)
            .fetch_one(db.pool())
            .await
            .expect("load replayed Unit version");
    assert_eq!(replayed_unit_version, published_unit.2);
}
