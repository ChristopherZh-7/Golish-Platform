use golish_db::models::NewSession;
use golish_db::repo::{
    attack_candidates, attack_waves, operation_stage_forks, operator_principals,
    organization_deletion_jobs, project_scopes, runtime_memory_tx, sessions, stage_handoffs,
};
use golish_db::{DbConfig, GolishDb};
use serial_test::serial;
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read reserved local postgres port")
        .port()
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => serde_json::to_string(value).unwrap(),
        serde_json::Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        serde_json::Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|key| format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap(),
                        canonical_json(&object[key])
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

fn sha256_json(value: &serde_json::Value) -> String {
    Sha256::digest(canonical_json(value).as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

struct FinalStageHandoffFixture<'a> {
    session_id: Uuid,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    stage_kind: &'a str,
    generation: i32,
    evidence_ids: &'a [i64],
    started_at_sql: &'a str,
    completed_at_sql: &'a str,
}

async fn insert_final_stage_handoff(db: &GolishDb, fixture: FinalStageHandoffFixture<'_>) {
    let FinalStageHandoffFixture {
        session_id,
        operation_id,
        scope_snapshot_id,
        organization_id,
        stage_kind,
        generation,
        evidence_ids,
        started_at_sql,
        completed_at_sql,
    } = fixture;
    let stage_execution_id = Uuid::new_v4();
    let stage_run_unit_id = Uuid::new_v4();
    let worker_run_id = Uuid::new_v4();
    let lease_token = Uuid::new_v4();
    let tool_call_id = Uuid::new_v4();
    let submission_id = Uuid::new_v4();
    let payload = serde_json::json!({
        "schema_version": 1,
        "stage_kind": stage_kind,
        "organization_id": organization_id,
    });

    let stage_sql = format!(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status,started_at,completed_at) \
         VALUES($1,$2,$3,'completed',{started_at_sql},{completed_at_sql})"
    );
    sqlx::query(&stage_sql)
        .bind(stage_execution_id)
        .bind(operation_id)
        .bind(stage_kind)
        .execute(db.pool())
        .await
        .expect("insert completed source stage execution");
    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,
               organization_id,stage_kind,generation,specialist,status,
               terminal_at,pass_watermark
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$6,'passed',NOW(),$8)"#,
    )
    .bind(stage_run_unit_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(stage_kind)
    .bind(generation)
    .bind(serde_json::json!({"final_gate_passed": true}))
    .execute(db.pool())
    .await
    .expect("insert passed source stage unit");
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,
               organization_id,worker_generation,specialist,work_item_kind,
               work_item_key,agent_path,status,lease_token,lease_owner,
               lease_acquired_at,lease_expires_at,heartbeat_at,attempt_epoch
           ) VALUES($1,$2,$3,$4,$5,0,$6,'stage_unit',$6,$7,'running',$8,
                    'stage-fork-fixture',NOW(),NOW()+INTERVAL '5 minutes',NOW(),0)"#,
    )
    .bind(worker_run_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(organization_id)
    .bind(stage_kind)
    .bind(format!("main>{stage_kind}"))
    .bind(lease_token)
    .execute(db.pool())
    .await
    .expect("insert passed source worker");
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,result,status,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token
           ) VALUES($1,$2,$3,$4,'primary','submit_stage_deliverable','{}','{}',
                    'finished',$4,$5,$6,$7,$8,0,$9)"#,
    )
    .bind(tool_call_id)
    .bind(format!("{stage_kind}-fork-fixture"))
    .bind(session_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(worker_run_id)
    .bind(organization_id)
    .bind(lease_token)
    .execute(db.pool())
    .await
    .expect("insert source final submission tool call");
    sqlx::query(
        r#"INSERT INTO stage_deliverable_submissions(
               id,operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,tool_call_record_id,tool_request_id,stage_kind,
               attempt_epoch,lease_token,payload,payload_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,0,$10,$11,$12)"#,
    )
    .bind(submission_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(worker_run_id)
    .bind(organization_id)
    .bind(tool_call_id)
    .bind(format!("{stage_kind}-fork-fixture"))
    .bind(stage_kind)
    .bind(lease_token)
    .bind(&payload)
    .bind(sha256_json(&payload))
    .execute(db.pool())
    .await
    .expect("insert source final submission");
    sqlx::query(
        "UPDATE stage_worker_runs SET status='passed',terminal_at=NOW(),updated_at=NOW() \
         WHERE id=$1",
    )
    .bind(worker_run_id)
    .execute(db.pool())
    .await
    .expect("finish source worker before final handoff");
    sqlx::query(
        r#"INSERT INTO stage_handoffs(
               id,operation_id,organization_id,scope_snapshot_id,from_stage_kind,
               stage_execution_id,source_stage_run_unit_id,deliverable_submission_id,
               scope_hash,payload,payload_sha256,evidence_ids,coverage_watermark,
               unit_gate_decision_hash,gate_passed_at
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,NOW())"#,
    )
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(organization_id)
    .bind(scope_snapshot_id)
    .bind(stage_kind)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(submission_id)
    .bind("a".repeat(64))
    .bind(&payload)
    .bind(sha256_json(&payload))
    .bind(evidence_ids)
    .bind(serde_json::json!({"complete": true, "stage_kind": stage_kind}))
    .bind("b".repeat(64))
    .execute(db.pool())
    .await
    .expect("insert exact source final handoff");
}

#[tokio::test]
#[serial]
async fn shared_db_candidate_fork_materializes_scoping_prefix_targets_and_wave_entry() {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let workspace = format!("/tmp/stage-fork-{}", Uuid::new_v4().simple());
    let config = DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("stage_fork_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    };
    let mut db = GolishDb::start(config)
        .await
        .expect("start fresh migrated embedded postgres");
    let project = project_scopes::register_first_open(db.pool(), &workspace, &"1".repeat(64))
        .await
        .expect("register shared project scope");
    let source_session = sessions::create(
        db.pool(),
        NewSession {
            title: Some("GUI source run".to_string()),
            workspace_path: Some(workspace.clone()),
            workspace_label: None,
            model: Some("source-model".to_string()),
            provider: Some("source-provider".to_string()),
            project_path: Some(workspace.clone()),
        },
    )
    .await
    .expect("create GUI-shaped source session");
    let source_operation_id = Uuid::new_v4();
    let scoping_execution_id = Uuid::new_v4();
    runtime_memory_tx::create_runtime_operation(
        db.pool(),
        &runtime_memory_tx::CreateRuntimeOperationRow {
            operation_id: source_operation_id,
            initial_stage_execution_id: scoping_execution_id,
            session_id: source_session.id,
            title: Some("GUI source run".to_string()),
            input: "full GUI run".to_string(),
            profile: "red_team".to_string(),
            entry_stage: "scoping".to_string(),
            project_scope_id: project.project_scope_id,
            cli_scope: None,
        },
    )
    .await
    .expect("create source operation through shared runtime repository");

    let root_organization_id = Uuid::new_v4();
    let scope_decision_id = Uuid::new_v4();
    let source_scope_snapshot_id = Uuid::new_v4();
    let scoping_unit_id = Uuid::new_v4();
    let scoping_tool_call_id = Uuid::new_v4();
    let scoping_submission_id = Uuid::new_v4();
    let scope_hash = "a".repeat(64);
    let decision_hash = "b".repeat(64);
    let mut scope_tx = db.pool().begin().await.expect("begin source scope fixture");
    sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$2,'Root Org')")
        .bind(root_organization_id)
        .bind(&workspace)
        .execute(&mut *scope_tx)
        .await
        .expect("insert source root organization");
    sqlx::query(
        r#"INSERT INTO operation_scope_decisions(
               id,operation_id,project_scope_id,stage_execution_id,
               root_organization_id,mode,decision_rows,decision_hash
           ) VALUES($1,$2,$3,$4,$5,'cli_flags',$6,$7)"#,
    )
    .bind(scope_decision_id)
    .bind(source_operation_id)
    .bind(project.project_scope_id)
    .bind(scoping_execution_id)
    .bind(root_organization_id)
    .bind(serde_json::json!([{"organization_id": root_organization_id}]))
    .bind(&decision_hash)
    .execute(&mut *scope_tx)
    .await
    .expect("insert source scope decision");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_snapshots(
               id,operation_id,project_scope_id,scope_decision_id,
               project_path_at_freeze,root_organization_id,mode,scope_hash
           ) VALUES($1,$2,$3,$4,$5,$6,'cli_flags',$7)"#,
    )
    .bind(source_scope_snapshot_id)
    .bind(source_operation_id)
    .bind(project.project_scope_id)
    .bind(scope_decision_id)
    .bind(&workspace)
    .bind(root_organization_id)
    .bind(&scope_hash)
    .execute(&mut *scope_tx)
    .await
    .expect("insert source scope snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units(
               snapshot_id,organization_id,organization_name_at_freeze,
               role,depth,ordinal,decision_row_id,approval_source
           ) VALUES($1,$2,'Root Org','root',0,0,'root',$3)"#,
    )
    .bind(source_scope_snapshot_id)
    .bind(root_organization_id)
    .bind(serde_json::json!({"source": "gui_scoping"}))
    .execute(&mut *scope_tx)
    .await
    .expect("insert source frozen root unit");
    sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
        .bind(source_scope_snapshot_id)
        .execute(&mut *scope_tx)
        .await
        .expect("seal source organization scope");
    scope_tx
        .commit()
        .await
        .expect("commit source scope fixture");

    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,
               organization_id,stage_kind,generation,status,terminal_at,pass_watermark
           ) VALUES($1,$2,$3,$4,$5,'scoping',0,'passed',NOW(),$6)"#,
    )
    .bind(scoping_unit_id)
    .bind(source_operation_id)
    .bind(scoping_execution_id)
    .bind(source_scope_snapshot_id)
    .bind(root_organization_id)
    .bind(serde_json::json!({"scope_snapshot_id": source_scope_snapshot_id}))
    .execute(db.pool())
    .await
    .expect("insert passed Scoping root unit");
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,result,status,
               operation_id,stage_execution_id,stage_run_unit_id,organization_id
           ) VALUES($1,'scoping-fork-fixture',$2,$3,'primary',
                    'submit_stage_deliverable','{}','{}','finished',$3,$4,$5,$6)"#,
    )
    .bind(scoping_tool_call_id)
    .bind(source_session.id)
    .bind(source_operation_id)
    .bind(scoping_execution_id)
    .bind(scoping_unit_id)
    .bind(root_organization_id)
    .execute(db.pool())
    .await
    .expect("insert Scoping submission tool call");
    let scoping_payload = serde_json::json!({"schema_version": 1, "scope": "approved"});
    sqlx::query(
        r#"INSERT INTO stage_deliverable_submissions(
               id,operation_id,stage_execution_id,stage_run_unit_id,organization_id,
               tool_call_record_id,tool_request_id,stage_kind,payload,payload_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,'scoping-fork-fixture','scoping',$7,$8)"#,
    )
    .bind(scoping_submission_id)
    .bind(source_operation_id)
    .bind(scoping_execution_id)
    .bind(scoping_unit_id)
    .bind(root_organization_id)
    .bind(scoping_tool_call_id)
    .bind(&scoping_payload)
    .bind(sha256_json(&scoping_payload))
    .execute(db.pool())
    .await
    .expect("insert workerless Scoping submission");
    sqlx::query("UPDATE stage_runs SET status='completed',completed_at=NOW() WHERE id=$1")
        .bind(scoping_execution_id)
        .execute(db.pool())
        .await
        .expect("complete source Scoping execution");

    let target_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO targets(id,name,target_type,value,scope,project_path,organization_id)
           VALUES($1,'Root app','url','https://fork.example.test','in',$2,$3)"#,
    )
    .bind(target_id)
    .bind(&workspace)
    .bind(root_organization_id)
    .execute(db.pool())
    .await
    .expect("insert current Scoping target");
    let enumeration_evidence_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO audit_log(
               action,category,details,project_path,audit_role,run_id,target_id,detail
           ) VALUES('enumerated origin','recon','',$1,'evidence',$2,$3,$4)
           RETURNING id"#,
    )
    .bind(&workspace)
    .bind(source_operation_id)
    .bind(target_id)
    .bind(serde_json::json!({"organization_id": root_organization_id}))
    .fetch_one(db.pool())
    .await
    .expect("insert Enumeration source evidence");
    let vuln_evidence_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO audit_log(
               action,category,details,project_path,audit_role,run_id,target_id,detail
           ) VALUES('formulaic observation','attack','',$1,'evidence',$2,$3,$4)
           RETURNING id"#,
    )
    .bind(&workspace)
    .bind(source_operation_id)
    .bind(target_id)
    .bind(serde_json::json!({"organization_id": root_organization_id}))
    .fetch_one(db.pool())
    .await
    .expect("insert Vuln source evidence");

    insert_final_stage_handoff(
        &db,
        FinalStageHandoffFixture {
            session_id: source_session.id,
            operation_id: source_operation_id,
            scope_snapshot_id: source_scope_snapshot_id,
            organization_id: root_organization_id,
            stage_kind: "target_intel",
            generation: 0,
            evidence_ids: &[],
            started_at_sql: "NOW()-INTERVAL '5 minutes'",
            completed_at_sql: "NOW()-INTERVAL '4 minutes'",
        },
    )
    .await;
    insert_final_stage_handoff(
        &db,
        FinalStageHandoffFixture {
            session_id: source_session.id,
            operation_id: source_operation_id,
            scope_snapshot_id: source_scope_snapshot_id,
            organization_id: root_organization_id,
            stage_kind: "external_attack_surface",
            generation: 0,
            evidence_ids: &[],
            started_at_sql: "NOW()-INTERVAL '4 minutes'",
            completed_at_sql: "NOW()-INTERVAL '3 minutes'",
        },
    )
    .await;
    insert_final_stage_handoff(
        &db,
        FinalStageHandoffFixture {
            session_id: source_session.id,
            operation_id: source_operation_id,
            scope_snapshot_id: source_scope_snapshot_id,
            organization_id: root_organization_id,
            stage_kind: "enumeration",
            generation: 0,
            evidence_ids: &[enumeration_evidence_id],
            started_at_sql: "NOW()-INTERVAL '3 minutes'",
            completed_at_sql: "NOW()-INTERVAL '2 minutes'",
        },
    )
    .await;
    insert_final_stage_handoff(
        &db,
        FinalStageHandoffFixture {
            session_id: source_session.id,
            operation_id: source_operation_id,
            scope_snapshot_id: source_scope_snapshot_id,
            organization_id: root_organization_id,
            stage_kind: "vuln_triage",
            generation: 0,
            evidence_ids: &[vuln_evidence_id],
            started_at_sql: "NOW()-INTERVAL '2 minutes'",
            completed_at_sql: "NOW()-INTERVAL '1 minute'",
        },
    )
    .await;

    let source_before: (String, Option<Uuid>, i64, i64) = sqlx::query_as(
        r#"SELECT operation.current_stage,operation.superseded_by,
                  (SELECT COUNT(*) FROM stage_handoffs
                    WHERE operation_id=operation.operation_id),
                  (SELECT COUNT(*) FROM tool_calls
                    WHERE operation_id=operation.operation_id)
             FROM operation_state AS operation
            WHERE operation.operation_id=$1"#,
    )
    .bind(source_operation_id)
    .fetch_one(db.pool())
    .await
    .expect("snapshot source operation before fork");

    let target_session = sessions::create(
        db.pool(),
        NewSession {
            title: Some("CLI Candidate-only fork".to_string()),
            workspace_path: Some(workspace.clone()),
            workspace_label: None,
            model: Some("source-model".to_string()),
            provider: Some("source-provider".to_string()),
            project_path: Some(workspace.clone()),
        },
    )
    .await
    .expect("create CLI fork session");
    let target_operation_id = Uuid::new_v4();
    let created = runtime_memory_tx::create_runtime_operation_with_stage_fork(
        db.pool(),
        &runtime_memory_tx::CreateRuntimeOperationRow {
            operation_id: target_operation_id,
            initial_stage_execution_id: Uuid::new_v4(),
            session_id: target_session.id,
            title: Some("CLI Candidate-only fork".to_string()),
            input: "run Candidate only".to_string(),
            profile: "red_team".to_string(),
            entry_stage: "attack_candidate".to_string(),
            project_scope_id: project.project_scope_id,
            cli_scope: Some(runtime_memory_tx::CliRuntimeScopeRow {
                root_organization_id,
                include_subsidiaries: false,
                subsidiary_threshold: 51,
                units: vec![runtime_memory_tx::CliRuntimeScopeUnitRow {
                    organization_id: root_organization_id,
                    parent_organization_id: None,
                    organization_name: "Root Org".to_string(),
                    depth: 0,
                    ordinal: 0,
                    ownership_percent: None,
                    approval_source: serde_json::json!({"kind": "stage_fork_source_scope"}),
                }],
            }),
        },
        &runtime_memory_tx::StageForkCreateRow {
            source_operation_id,
            source_scope_snapshot_id,
            entry_stage: "attack_candidate".to_string(),
            terminal_stage: "attack_candidate".to_string(),
            adopted_stage_kinds: vec![
                "scoping".to_string(),
                "target_intel".to_string(),
                "external_attack_surface".to_string(),
                "enumeration".to_string(),
                "vuln_triage".to_string(),
            ],
        },
    )
    .await
    .expect("atomically create CLI fork from GUI-shaped source DB truth");
    assert_eq!(created.operation.operation_id, target_operation_id);
    let target_scope_snapshot_id: Uuid =
        sqlx::query_scalar("SELECT id FROM operation_org_scope_snapshots WHERE operation_id=$1")
            .bind(target_operation_id)
            .fetch_one(db.pool())
            .await
            .expect("stage fork freezes target scope in creation transaction");
    let fork = operation_stage_forks::get(db.pool(), target_operation_id)
        .await
        .expect("load stage fork header")
        .expect("stage fork header exists");
    assert_eq!(fork.source_operation_id, source_operation_id);
    assert_eq!(fork.expected_input_count, 5);
    assert_eq!(fork.expected_target_count, 1);
    let inputs = operation_stage_forks::list_inputs(db.pool(), target_operation_id)
        .await
        .expect("load immutable fork inputs");
    assert_eq!(inputs.len(), 5);
    let scoping_input = inputs
        .iter()
        .find(|input| input.source_stage_kind == "scoping")
        .expect("Scoping is adopted from sealed scope truth");
    assert_eq!(scoping_input.source_handoff_id, None);
    assert_eq!(scoping_input.source_worker_run_id, None);
    let targets = operation_stage_forks::list_targets(db.pool(), target_operation_id)
        .await
        .expect("load immutable current Target snapshot");
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].live_target_id, target_id);

    let deletion_principal = operator_principals::current_local(db.pool())
        .await
        .expect("load trusted organization deletion principal");
    let delete_error = organization_deletion_jobs::request(
        db.pool(),
        &organization_deletion_jobs::RequestOrganizationDeletion {
            job_id: Uuid::new_v4(),
            root_organization_id,
            principal_id: deletion_principal.id,
            expected_project_path: workspace.clone(),
        },
    )
    .await
    .expect_err("an active stage fork must block deletion before artifact cleanup starts");
    assert!(
        delete_error
            .to_string()
            .contains(&target_operation_id.to_string()),
        "blocker must identify the exact active stage fork: {delete_error}"
    );
    assert!(
        organization_deletion_jobs::list_active(db.pool())
            .await
            .expect("list deletion jobs after active-fork rejection")
            .is_empty(),
        "active-fork rejection must not commit a deletion job or artifact cleanup"
    );
    let target_stages: Vec<String> = sqlx::query_scalar(
        "SELECT stage_kind FROM stage_runs WHERE operation_id=$1 ORDER BY started_at,id",
    )
    .bind(target_operation_id)
    .fetch_all(db.pool())
    .await
    .expect("load target fork executions");
    assert_eq!(target_stages, vec!["attack_candidate"]);

    let inherited = stage_handoffs::list_latest_final_sealed_for_sources(
        db.pool(),
        target_operation_id,
        root_organization_id,
        &fork.adopted_stage_kinds,
    )
    .await
    .expect("resolve all adopted source truth under the target operation");
    assert_eq!(inherited.len(), 5);
    assert!(inherited
        .iter()
        .all(|handoff| handoff.authority_kind == "stage_fork_final_seal"));

    let vuln_input = inputs
        .iter()
        .find(|input| input.source_stage_kind == "vuln_triage")
        .expect("Candidate fork has exact Vuln input");
    let policy_snapshot = serde_json::json!({
        "max_waves": 3,
        "max_candidates_total": 100,
        "max_chain_depth": 3,
        "max_attempts_total": 200,
    });
    let policy_hash =
        "sha256:66e50329b4bb217eb060bcccb38f78f4b0eafc163471bebd6554d271d1a6b326".to_string();
    let wave_run_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{target_operation_id}:candidate-wave:0").as_bytes(),
    );
    let wave_unit_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{wave_run_id}:{root_organization_id}").as_bytes(),
    );
    let mut wave_tx = db.pool().begin().await.expect("begin fork Wave entry");
    let (_, wave_unit) = attack_waves::open_from_stage_fork_input(
        &mut wave_tx,
        &attack_waves::OpenAttackWaveForkUnit {
            wave_run_id,
            wave_unit_id,
            operation_id: target_operation_id,
            scope_snapshot_id: target_scope_snapshot_id,
            organization_id: root_organization_id,
            stage_fork_input_id: vuln_input.id,
            generation: 0,
            ordinal: 0,
            policy_snapshot,
            policy_hash,
            max_waves: 3,
            max_candidates_total: 100,
            max_chain_depth: 3,
            max_attempts_total: 200,
        },
    )
    .await
    .expect("open Candidate Wave from exact fork input");
    wave_tx.commit().await.expect("commit fork Wave entry");
    assert_eq!(
        wave_unit.entry,
        attack_waves::AttackWaveEntry::ForkedVulnHandoff {
            stage_fork_input_id: vuln_input.id,
        }
    );
    let mut acceptance_probe = db
        .pool()
        .begin()
        .await
        .expect("begin Candidate fork acceptance authority probe");
    let acceptance_error = attack_candidates::accept_gate_passed_candidate_batch(
        &mut acceptance_probe,
        attack_candidates::AcceptCandidateBatch {
            operation_id: target_operation_id,
            scope_snapshot_id: target_scope_snapshot_id,
            wave_run_id,
            wave_unit_id: wave_unit.id,
            organization_id: root_organization_id,
            decision_stage_execution_id: Uuid::new_v4(),
            decision_stage_run_unit_id: Uuid::new_v4(),
            decision_deliverable_submission_id: Uuid::new_v4(),
            manifest_hash: "sha256:probe".to_string(),
            expected_work_item_ids: vec![Uuid::new_v4()],
            candidates: Vec::new(),
            no_candidate_decisions: Vec::new(),
        },
    )
    .await
    .expect_err("missing Candidate final-pass authority must fail after resolving fork Wave entry");
    assert!(
        acceptance_error
            .to_string()
            .contains("exact current-generation attack_candidate final-pass submission"),
        "fork Wave entry must be accepted by the shared Candidate gate query before the missing decision seal is reported: {acceptance_error}"
    );
    acceptance_probe
        .rollback()
        .await
        .expect("rollback Candidate authority probe");

    let active_drift = sqlx::query("UPDATE targets SET scope='out' WHERE id=$1")
        .bind(target_id)
        .execute(db.pool())
        .await
        .expect_err("active EAS-or-later fork freezes its snapshotted Target identity/scope");
    assert!(active_drift
        .to_string()
        .contains("active stage fork Target identity/scope is frozen"));
    let target_stage_execution_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM stage_runs WHERE operation_id=$1 AND stage_kind='attack_candidate'",
    )
    .bind(target_operation_id)
    .fetch_one(db.pool())
    .await
    .expect("load target fork stage execution for parent tool fixture");
    let live_stage_unit_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,
               organization_id,stage_kind,generation,specialist,status
           ) VALUES($1,$2,$3,$4,$5,'attack_candidate',0,'attack_analyst','running')"#,
    )
    .bind(live_stage_unit_id)
    .bind(target_operation_id)
    .bind(target_stage_execution_id)
    .bind(target_scope_snapshot_id)
    .bind(root_organization_id)
    .execute(db.pool())
    .await
    .expect("insert active fork unit for live-lease delete blocker regression");
    let live_worker_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,
               organization_id,worker_generation,specialist,work_item_kind,
               work_item_key,agent_path,status,lease_token,lease_owner,
               lease_acquired_at,lease_expires_at,heartbeat_at,attempt_epoch
           ) VALUES($1,$2,$3,$4,$5,0,'attack_analyst','stage_unit',
                    'attack_candidate','main>attack_candidate','running',$6,
                    'live-delete-blocker',NOW(),NOW()+INTERVAL '5 minutes',NOW(),0)"#,
    )
    .bind(live_worker_id)
    .bind(target_operation_id)
    .bind(target_stage_execution_id)
    .bind(live_stage_unit_id)
    .bind(root_organization_id)
    .bind(Uuid::new_v4())
    .execute(db.pool())
    .await
    .expect("insert live Worker lease for delete blocker regression");
    sqlx::query("UPDATE tasks SET status='waiting',updated_at=NOW() WHERE id=$1")
        .bind(target_operation_id)
        .execute(db.pool())
        .await
        .expect("pause fork while its Worker still holds live authority");
    let live_delete_error = organization_deletion_jobs::request(
        db.pool(),
        &organization_deletion_jobs::RequestOrganizationDeletion {
            job_id: Uuid::new_v4(),
            root_organization_id,
            principal_id: deletion_principal.id,
            expected_project_path: workspace.clone(),
        },
    )
    .await
    .expect_err("a waiting fork with a live Worker lease must still block deletion");
    assert!(
        live_delete_error
            .to_string()
            .contains(&target_operation_id.to_string()),
        "live authority blocker must preserve exact operation identity: {live_delete_error}"
    );
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET status='gate_blocked',lease_token=NULL,lease_owner=NULL,
                  lease_acquired_at=NULL,lease_expires_at=NULL,heartbeat_at=NULL,
                  updated_at=NOW()
            WHERE id=$1"#,
    )
    .bind(live_worker_id)
    .execute(db.pool())
    .await
    .expect("release fixture Worker authority before quiescent deletion");
    let stale_stage_run_tool_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,status,
               operation_id,stage_execution_id
           ) VALUES($1,'quiescent-delete-stage-run',$2,$3,'primary','stage_run','{}',
                    'running',$3,$4)"#,
    )
    .bind(stale_stage_run_tool_id)
    .bind(target_session.id)
    .bind(target_operation_id)
    .bind(target_stage_execution_id)
    .execute(db.pool())
    .await
    .expect("insert stale parent stage_run tool for quiescent delete regression");
    let accepted_deletion = organization_deletion_jobs::request(
        db.pool(),
        &organization_deletion_jobs::RequestOrganizationDeletion {
            job_id: Uuid::new_v4(),
            root_organization_id,
            principal_id: deletion_principal.id,
            expected_project_path: workspace.clone(),
        },
    )
    .await
    .expect("a quiescent waiting stage fork is stopped by explicit organization deletion");
    assert_eq!(accepted_deletion.state, "waiting_for_invalidation_delivery");
    let stopped_task: (String, Option<String>) =
        sqlx::query_as("SELECT status::TEXT,result FROM tasks WHERE id=$1")
            .bind(target_operation_id)
            .fetch_one(db.pool())
            .await
            .expect("read organization-delete-stopped fork task");
    assert_eq!(stopped_task.0, "failed");
    assert_eq!(
        stopped_task.1.as_deref(),
        Some("Stopped: organization deletion closed a quiescent stage task.")
    );
    let stopped_tool: (String, Option<String>) =
        sqlx::query_as("SELECT status::TEXT,result FROM tool_calls WHERE id=$1")
            .bind(stale_stage_run_tool_id)
            .fetch_one(db.pool())
            .await
            .expect("read organization-delete-stopped parent tool");
    assert_eq!(stopped_tool.0, "failed");
    assert_eq!(
        stopped_tool.1.as_deref(),
        Some("Stopped: organization deletion closed a quiescent stage task.")
    );
    let open_turn_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM operation_turns \
         WHERE operation_id=$1 AND status IN ('running','waiting')",
    )
    .bind(target_operation_id)
    .fetch_one(db.pool())
    .await
    .expect("count open turns after quiescent task closure");
    assert_eq!(open_turn_count, 0);
    let stopped_history_detail: serde_json::Value = sqlx::query_scalar(
        r#"SELECT detail FROM organization_deletion_job_state_history
            WHERE job_id=$1 AND state='deleting_db_committed'"#,
    )
    .bind(accepted_deletion.id)
    .fetch_one(db.pool())
    .await
    .expect("read deletion admission history");
    assert_eq!(
        stopped_history_detail["stoppedQuiescentStageTaskIds"],
        serde_json::json!([target_operation_id])
    );

    let blocked_fork_session = sessions::create(
        db.pool(),
        NewSession {
            title: Some("CLI fork rejected by deletion".to_string()),
            workspace_path: Some(workspace.clone()),
            workspace_label: None,
            model: Some("source-model".to_string()),
            provider: Some("source-provider".to_string()),
            project_path: Some(workspace.clone()),
        },
    )
    .await
    .expect("create second CLI fork session");
    let blocked_fork_operation_id = Uuid::new_v4();
    let blocked_fork_error = runtime_memory_tx::create_runtime_operation_with_stage_fork(
        db.pool(),
        &runtime_memory_tx::CreateRuntimeOperationRow {
            operation_id: blocked_fork_operation_id,
            initial_stage_execution_id: Uuid::new_v4(),
            session_id: blocked_fork_session.id,
            title: Some("CLI fork rejected by deletion".to_string()),
            input: "must not start after deletion".to_string(),
            profile: "red_team".to_string(),
            entry_stage: "attack_candidate".to_string(),
            project_scope_id: project.project_scope_id,
            cli_scope: Some(runtime_memory_tx::CliRuntimeScopeRow {
                root_organization_id,
                include_subsidiaries: false,
                subsidiary_threshold: 51,
                units: vec![runtime_memory_tx::CliRuntimeScopeUnitRow {
                    organization_id: root_organization_id,
                    parent_organization_id: None,
                    organization_name: "Root Org".to_string(),
                    depth: 0,
                    ordinal: 0,
                    ownership_percent: None,
                    approval_source: serde_json::json!({
                        "kind": "stage_fork_source_scope"
                    }),
                }],
            }),
        },
        &runtime_memory_tx::StageForkCreateRow {
            source_operation_id,
            source_scope_snapshot_id,
            entry_stage: "attack_candidate".to_string(),
            terminal_stage: "attack_candidate".to_string(),
            adopted_stage_kinds: vec![
                "scoping".to_string(),
                "target_intel".to_string(),
                "external_attack_surface".to_string(),
                "enumeration".to_string(),
                "vuln_triage".to_string(),
            ],
        },
    )
    .await
    .expect_err("an active organization deletion must reject a new stage fork");
    assert!(
        blocked_fork_error
            .to_string()
            .contains("stage_fork_target_organization_deleting"),
        "reverse admission fence must return its stable blocker: {blocked_fork_error}"
    );
    assert!(
        operation_stage_forks::get(db.pool(), blocked_fork_operation_id)
            .await
            .expect("read rolled-back second fork")
            .is_none(),
        "rejected stage-fork creation must roll back its immutable manifest"
    );
    let source_after: (String, Option<Uuid>, i64, i64) = sqlx::query_as(
        r#"SELECT operation.current_stage,operation.superseded_by,
                  (SELECT COUNT(*) FROM stage_handoffs
                    WHERE operation_id=operation.operation_id),
                  (SELECT COUNT(*) FROM tool_calls
                    WHERE operation_id=operation.operation_id)
             FROM operation_state AS operation
            WHERE operation.operation_id=$1"#,
    )
    .bind(source_operation_id)
    .fetch_one(db.pool())
    .await
    .expect("snapshot source operation after fork");
    assert_eq!(
        source_after, source_before,
        "fork must not mutate source run"
    );

    db.stop().await;
}
