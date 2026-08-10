use golish_db::models::AgentType;
use golish_db::repo::investigation_nested_dispatch::{
    BeginInvestigationNestedDispatchRow, FinishInvestigationNestedDispatchRow,
    PgInvestigationNestedDispatchRepository,
};
use golish_db::repo::runtime_memory_tx::RuntimeMemoryTxFence;
use golish_db::repo::runtime_memory_tx::{
    claim_stage_team_leader, load_investigation_runtime_cursor, rearm_investigation_task_primary,
    recover_investigation_advisory_primary, ClaimStageTeamLeaderRow, ClaimStageWorkItemRow,
    InvestigationRuntimeCursorPhaseRow, LoadInvestigationRuntimeCursorRow,
    RearmInvestigationTaskPrimaryRow, RecoverInvestigationAdvisoryPrimaryRow,
};
use golish_db::repo::stage_teams::{
    complete_investigation_task_primary, CompleteInvestigationTaskPrimaryRow,
    CompleteStageWorkerRow,
};
use golish_db::repo::unified_investigation_runtime::PentagiDispatchOutcome;
use golish_db::{DbConfig, GolishDb};
use serde_json::{json, Value};
use serial_test::serial;
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read local postgres port")
        .port()
}

fn digest(nibble: char) -> String {
    format!("sha256:{}", nibble.to_string().repeat(64))
}

fn hash_json(value: &Value) -> String {
    fn canonical(value: &Value) -> String {
        match value {
            Value::Null => "null".to_string(),
            Value::Bool(value) => value.to_string(),
            Value::Number(value) => value.to_string(),
            Value::String(value) => serde_json::to_string(value).expect("serialize JSON string"),
            Value::Array(values) => format!(
                "[{}]",
                values.iter().map(canonical).collect::<Vec<_>>().join(",")
            ),
            Value::Object(object) => {
                let mut keys = object.keys().collect::<Vec<_>>();
                keys.sort_unstable();
                format!(
                    "{{{}}}",
                    keys.into_iter()
                        .map(|key| format!(
                            "{}:{}",
                            serde_json::to_string(key).expect("serialize JSON key"),
                            canonical(&object[key])
                        ))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
        }
    }
    format!(
        "sha256:{}",
        Sha256::digest(canonical(value).as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

async fn migrated_db() -> (GolishDb, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("nested_dispatch_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start isolated migrated postgres");
    (db, data_dir)
}

#[tokio::test]
#[serial]
async fn runtime_cursor_starts_from_the_durable_analysis_primary() {
    let (mut db, _data_dir) = migrated_db().await;
    let f = fixture(&db).await;
    let cursor = load_investigation_runtime_cursor(
        db.pool(),
        &LoadInvestigationRuntimeCursorRow {
            operation_id: f.operation_id,
            stage_execution_id: f.stage_execution_id,
            stage_run_unit_id: f.stage_run_unit_id,
            stage_team_plan_id: f.stage_team_plan_id,
        },
    )
    .await
    .expect("derive initial Analysis cursor from the active Primary");
    assert_eq!(cursor.phase, InvestigationRuntimeCursorPhaseRow::Analysis);
    assert_eq!(cursor.verification_task_id, None);
    assert!(!cursor.analysis_read_session_sealed);
    assert_eq!(cursor.dispatch_epoch, 0);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn investigation_task_primary_completion_keeps_the_closed_plan_and_unit_running() {
    let (mut db, _data_dir) = migrated_db().await;
    let f = fixture(&db).await;
    let primary_lease_token = Uuid::new_v4();
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin Primary completion fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("isolate exact terminal fixture state");
    sqlx::query(
        r#"UPDATE stage_work_items
              SET status='completed',row_version=row_version+1,
                  terminal_at=NOW(),updated_at=NOW()
            WHERE id=$1"#,
    )
    .bind(f.parent_work_item_id)
    .execute(&mut *tx)
    .await
    .expect("terminalize required cognitive WorkItem");
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET status='passed',checkpoint_version=checkpoint_version+1,
                  lease_token=NULL,lease_owner=NULL,lease_acquired_at=NULL,
                  lease_expires_at=NULL,heartbeat_at=NULL,terminal_at=NOW(),updated_at=NOW()
            WHERE id=$1"#,
    )
    .bind(f.parent_worker_run_id)
    .execute(&mut *tx)
    .await
    .expect("terminalize required cognitive WorkerRun");
    sqlx::query(
        r#"INSERT INTO stage_worker_outputs(
               id,team_plan_id,work_item_id,worker_run_id,operation_id,stage_execution_id,
               stage_run_unit_id,scope_snapshot_id,organization_id,output_schema,output_version,
               business_disposition,canonical_output,canonical_fact_refs,evidence_ids,
               checked_empty_cells,blocker_codes,output_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'investigation_cognitive_output.v1',1,
                  'checked_empty','{}'::JSONB,'[]'::JSONB,'{}'::BIGINT[],'[]'::JSONB,
                  '{}'::TEXT[],$10)"#,
    )
    .bind(Uuid::new_v4())
    .bind(f.stage_team_plan_id)
    .bind(f.parent_work_item_id)
    .bind(f.parent_worker_run_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(f.stage_run_unit_id)
    .bind(f.scope_snapshot_id)
    .bind(f.organization_id)
    .bind(digest('1'))
    .execute(&mut *tx)
    .await
    .expect("persist required cognitive output");
    sqlx::query(
        r#"UPDATE stage_work_items
              SET status='running',started_at=NOW(),terminal_at=NULL,updated_at=NOW()
            WHERE id=$1"#,
    )
    .bind(f.leader_work_item_id)
    .execute(&mut *tx)
    .await
    .expect("make planning Primary current");
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET status='running',lease_token=$2,lease_owner='primary-completion-fixture',
                  lease_acquired_at=NOW(),lease_expires_at=NOW()+INTERVAL '10 minutes',
                  heartbeat_at=NOW(),terminal_at=NULL,updated_at=NOW()
            WHERE id=$1"#,
    )
    .bind(f.primary_worker_run_id)
    .bind(primary_lease_token)
    .execute(&mut *tx)
    .await
    .expect("make planning Primary WorkerRun current");
    tx.commit()
        .await
        .expect("commit Primary completion fixture");

    let closed = golish_db::repo::runtime_memory_tx::close_stage_request_epoch(
        db.pool(),
        &golish_db::repo::runtime_memory_tx::CloseStageRequestEpochRow {
            operation_id: f.operation_id,
            stage_execution_id: f.stage_execution_id,
            stage_run_unit_id: f.stage_run_unit_id,
            stage_team_plan_id: f.stage_team_plan_id,
            expected_dispatch_epoch: 0,
            expected_plan_row_version: 0,
        },
    )
    .await
    .expect("close exact Investigation request epoch");
    assert!(closed.barrier.ready_to_finalize());
    let input = CompleteInvestigationTaskPrimaryRow {
        fence: RuntimeMemoryTxFence {
            operation_id: f.operation_id,
            stage_execution_id: f.stage_execution_id,
            stage_run_unit_id: f.stage_run_unit_id,
            worker_run_id: f.primary_worker_run_id,
            lease_token: primary_lease_token,
            attempt_epoch: 1,
            expected_checkpoint_version: 1,
        },
        team_plan_id: f.stage_team_plan_id,
        primary_work_item_id: f.leader_work_item_id,
        expected_work_item_row_version: 0,
        expected_plan_row_version: closed.plan.row_version,
        expected_dispatch_epoch: closed.plan.dispatch_epoch,
        expected_barrier_manifest_hash: closed.barrier.manifest_hash.clone(),
        terminal_checkpoint: json!({"primary_synthesis":"complete"}),
    };
    let completed = complete_investigation_task_primary(db.pool(), input.clone())
        .await
        .expect("terminalize exact planning Primary without a nonexistent plan status");
    assert!(!completed.replayed);
    assert_eq!(completed.work_item.status, "completed");
    assert_eq!(completed.worker.status, "passed");
    assert_eq!(completed.plan.row_version, closed.plan.row_version);
    assert_eq!(completed.unit.status, "running");

    let replay = complete_investigation_task_primary(db.pool(), input)
        .await
        .expect("replay exact planning Primary terminalization");
    assert!(replay.replayed);
    assert_eq!(replay.plan.row_version, completed.plan.row_version);
    assert_eq!(replay.unit.status, "running");
    db.stop().await;
}

#[derive(Debug, Clone)]
struct Fixture {
    session_id: Uuid,
    authority_id: Uuid,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    owning_request_id: String,
    stage_run_unit_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    stage_team_plan_id: Uuid,
    leader_work_item_id: Uuid,
    primary_worker_run_id: Uuid,
    parent_work_item_id: Uuid,
    parent_worker_run_id: Uuid,
    parent_lease_token: Uuid,
    task_plan_id: Uuid,
    subtask_id: Uuid,
    parent_dispatch_receipt_id: Uuid,
}

async fn fixture(db: &GolishDb) -> Fixture {
    let session_id = Uuid::new_v4();
    let authority_id = Uuid::new_v4();
    let operation_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    let owning_request_id = "nested-fixture-stage-run".to_string();
    let stage_run_unit_id = Uuid::new_v4();
    let scope_snapshot_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    let scope_decision_id = Uuid::new_v4();
    let stage_team_plan_id = Uuid::new_v4();
    let leader_work_item_id = Uuid::new_v4();
    let primary_worker_run_id = Uuid::new_v4();
    let parent_worker_request_id = Uuid::new_v4();
    let parent_work_item_id = Uuid::new_v4();
    let parent_worker_run_id = Uuid::new_v4();
    let parent_lease_token = Uuid::new_v4();
    let task_plan_id = Uuid::new_v4();
    let subtask_id = Uuid::new_v4();
    let primary_dispatch_receipt_id = Uuid::new_v4();
    let parent_dispatch_receipt_id = Uuid::new_v4();
    let subject_id = Uuid::new_v4();

    let mut tx = db.pool().begin().await.expect("begin fixture seed");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("isolate fixture seed");
    sqlx::query("INSERT INTO sessions(id,title,status) VALUES($1,'nested fixture','running')")
        .bind(session_id)
        .execute(&mut *tx)
        .await
        .expect("insert session");
    sqlx::query(
        "INSERT INTO tasks(id,session_id,input,status) VALUES($1,$2,'nested fixture','running')",
    )
    .bind(operation_id)
    .bind(session_id)
    .execute(&mut *tx)
    .await
    .expect("insert task");
    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,project_scope_id,
               enumeration_analysis_contract,stage_topology_contract,
               stage_topology_canonical_json,stage_topology_sha256,
               stage_topology_freeze_source,investigation_contract_version,
               investigation_rollout_mode,tool_truth_contract
           ) VALUES($1,'red_team','investigation','v2_only',$2,'legacy_v1',
                    'unified_investigation_v1',
                    stage_topology_canonical_json('unified_investigation_v1'),
                    stage_topology_contract_sha256('unified_investigation_v1'),
                    'deployment_pair_v1','hypothesis_registry_v1','new_only','receipt_v1')"#,
    )
    .bind(operation_id)
    .bind(project_scope_id)
    .execute(&mut *tx)
    .await
    .expect("insert operation");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_snapshots(
               id,operation_id,project_scope_id,scope_decision_id,project_path_at_freeze,
               root_organization_id,mode,scope_hash,sealed_at
           ) VALUES($1,$2,$3,$4,'/tmp/nested-fixture',$5,'cli_flags',$6,NOW())"#,
    )
    .bind(scope_snapshot_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(scope_decision_id)
    .bind(organization_id)
    .bind(digest('1'))
    .execute(&mut *tx)
    .await
    .expect("insert scope snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units(
               snapshot_id,organization_id,organization_name_at_freeze,role,depth,ordinal,
               decision_row_id,approval_source
           ) VALUES($1,$2,'Nested Fixture Org','root',0,0,'root','{}'::jsonb)"#,
    )
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .execute(&mut *tx)
    .await
    .expect("insert scope unit");
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status,stage_topology_contract) VALUES($1,$2,'investigation','started','unified_investigation_v1')",
    )
    .bind(stage_execution_id)
    .bind(operation_id)
    .execute(&mut *tx)
    .await
    .expect("insert stage execution");
    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,organization_id,
               stage_kind,generation,status,started_at
           ) VALUES($1,$2,$3,$4,$5,'investigation',0,'running',NOW())"#,
    )
    .bind(stage_run_unit_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .execute(&mut *tx)
    .await
    .expect("insert stage unit");
    sqlx::query(
        r#"INSERT INTO investigation_stage_run_authorities(
               authority_id,operation_id,stage_execution_id,owning_stage_run_request_id,
               scope_snapshot_id
           ) VALUES($1,$2,$3,$4,$5)"#,
    )
    .bind(authority_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(&owning_request_id)
    .bind(scope_snapshot_id)
    .execute(&mut *tx)
    .await
    .expect("insert stage authority");
    sqlx::query(
        r#"INSERT INTO investigation_run_heads(
               authority_id,stable_start_request_id,operation_id,stage_execution_id,
               owning_stage_run_request_id,scope_snapshot_id,run_state,admission_open,
               stop_epoch,change_seq,head_version,head_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,'running',TRUE,0,0,0,
                    unified_investigation_runtime_head_sha256($1,'running',TRUE,0,0,0))"#,
    )
    .bind(authority_id)
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(&owning_request_id)
    .bind(scope_snapshot_id)
    .execute(&mut *tx)
    .await
    .expect("insert run head");
    sqlx::query(
        r#"INSERT INTO stage_team_plans(
               id,operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
               organization_id,stage_kind,unit_generation,schema_version,plan_version,
               plan_hash,leader_role,aggregator_kind,aggregator_role,allowed_worker_roles,
               max_workers_total,max_workers_active,dynamic_requests_allowed,
               dynamic_request_policy,dispatch_epoch,final_submitter_kind,
               created_from_stage_spec_hash
           ) VALUES($1,$2,$3,$4,$5,$6,'investigation',0,1,1,$7,'investigation','worker',
                    'investigation',$8,16,8,TRUE,$9,0,'worker',$10)"#,
    )
    .bind(stage_team_plan_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(digest('2'))
    .bind(json!(["investigation", "researcher", "coder"]))
    .bind(json!({
        "allowed_request_kinds": ["analysis_task"],
        "canonical_subject_refs_only": true,
        "child_budget": {},
        "child_output_schema": "investigation_cognitive_output.v1",
        "coordination_mode": "investigation_task_orchestrator",
        "max_requests": 8,
        "max_subject_refs": 8,
        "organization_scope_implicit": true,
        "attempt_policy": {"max_attempts": 3}
    }))
    .bind(digest('3'))
    .execute(&mut *tx)
    .await
    .expect("insert StageTeam plan");
    sqlx::query(
        r#"INSERT INTO stage_work_items(
               id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
               input_manifest_hash,required_for_barrier,priority,status,attempt_policy,budget,
               output_schema,created_by,started_at
           ) VALUES
              ($1,$3,$4,$5,$6,$7,$8,0,'investigation_primary','leader:primary',
               'investigation',$9,FALSE,0,'waiting_dependency','{"max_attempts":3}'::jsonb,
               '{}'::jsonb,'stage_unit_aggregate.v1','server_seed',NOW()),
              ($2,$3,$4,$5,$6,$7,$8,0,'analysis_task','dynamic:parent','researcher',$10,
               TRUE,1,'running','{"max_attempts":3}'::jsonb,'{}'::jsonb,
               'investigation_cognitive_output.v1','accepted_worker_request',NOW())"#,
    )
    .bind(leader_work_item_id)
    .bind(parent_work_item_id)
    .bind(stage_team_plan_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(digest('4'))
    .bind(digest('5'))
    .execute(&mut *tx)
    .await
    .expect("insert StageTeam work items");
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,work_item_id,
               organization_id,worker_generation,specialist,work_item_kind,work_item_key,
               agent_path,parent_request_id,status,checkpoint,checkpoint_version,
               lease_token,lease_owner,lease_acquired_at,lease_expires_at,heartbeat_at,
               attempt_epoch,started_at,terminal_at
           ) VALUES
              ($1,$3,$4,$5,$6,$7,0,'investigation','investigation_primary','leader:primary',
               'fixture>primary','primary-transcript','passed','[]'::jsonb,1,NULL,NULL,NULL,NULL,
               NULL,1,NOW(),NOW()),
              ($2,$3,$4,$5,$8,$7,0,'researcher','analysis_task','dynamic:parent',
               'fixture>worker','parent-transcript','running','[]'::jsonb,1,$9,
               'fixture-parent',NOW(),NOW()+INTERVAL '10 minutes',NOW(),1,NOW(),NULL)"#,
    )
    .bind(primary_worker_run_id)
    .bind(parent_worker_run_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(leader_work_item_id)
    .bind(organization_id)
    .bind(parent_work_item_id)
    .bind(parent_lease_token)
    .execute(&mut *tx)
    .await
    .expect("insert parent workers");
    sqlx::query(
        r#"INSERT INTO stage_worker_requests(
               id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,parent_work_item_id,parent_worker_run_id,
               dispatch_epoch,requested_role,request_kind,bounded_subject_refs,reason_code,
               expected_output_schema,budget_hint,dedupe_key,request_payload_hash,status,
               accepted_work_item_id
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,0,'researcher','analysis_task','[]'::jsonb,
                    $10,'investigation_cognitive_output.v1','{}'::jsonb,'parent',$11,
                    'accepted',$12)"#,
    )
    .bind(parent_worker_request_id)
    .bind(stage_team_plan_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(leader_work_item_id)
    .bind(primary_worker_run_id)
    .bind(
        json!({
            "schema": "investigation_task_orchestrator_request.v1",
            "parent_tool_request_id": "parent-transcript",
            "objective": "direct cognitive worker"
        })
        .to_string(),
    )
    .bind(digest('6'))
    .bind(parent_work_item_id)
    .execute(&mut *tx)
    .await
    .expect("insert parent worker request");
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_task_plans(
               task_plan_id,stable_request_id,authority_id,stage_team_plan_id,operation_id,
               stage_execution_id,owning_stage_run_request_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,subject_kind,subject_id,
               subject_fingerprint_sha256,task_plan_version,task_plan_sha256,
               allowed_role_catalog,cognitive_tool_envelope_sha256,status
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'analysis_attempt',$11,$12,1,$13,$14,$15,'open')"#,
    )
    .bind(task_plan_id)
    .bind(Uuid::new_v4())
    .bind(authority_id)
    .bind(stage_team_plan_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(&owning_request_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(subject_id)
    .bind(digest('7'))
    .bind(digest('8'))
    .bind(json!(["investigation", "researcher", "coder"]))
    .bind(digest('9'))
    .execute(&mut *tx)
    .await
    .expect("insert PentAGI task plan");
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_subtasks(
               subtask_id,task_plan_id,authority_id,operation_id,stage_execution_id,
               stage_run_unit_id,organization_id,subtask_ordinal,label,runnable,
               input_manifest_sha256,expected_output_schema,member_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,0,'bounded analysis',TRUE,$8,
                    'investigation_cognitive_output.v1',$9)"#,
    )
    .bind(subtask_id)
    .bind(task_plan_id)
    .bind(authority_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(organization_id)
    .bind(digest('a'))
    .bind(digest('b'))
    .execute(&mut *tx)
    .await
    .expect("insert PentAGI subtask");
    sqlx::query(
        r#"INSERT INTO pentagi_logical_dispatch_receipts(
               dispatch_receipt_id,stable_request_id,logical_dispatch_key_sha256,task_plan_id,
               subtask_id,parent_dispatch_receipt_id,dispatch_ordinal,actor_kind,
               stage_work_item_id,stage_worker_request_id,worker_run_id,operation_id,
               stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id,
               transcript_request_id,parent_actor_transcript_request_id,
               parent_dispatch_tool_request_id,snapshot_sha256,receipt_sha256
           ) VALUES
              ($1,$3,$4,$5,NULL,NULL,0,'primary',$6,NULL,$7,$8,$9,$10,$11,$12,
               'primary-transcript',NULL,NULL,$13,$14),
              ($2,$15,$16,$5,$17,$1,1,'worker',$18,$19,$20,$8,$9,$10,$11,$12,
               'parent-transcript','primary-transcript','parent-transcript',$13,$21)"#,
    )
    .bind(primary_dispatch_receipt_id)
    .bind(parent_dispatch_receipt_id)
    .bind(Uuid::new_v4())
    .bind(digest('c'))
    .bind(task_plan_id)
    .bind(leader_work_item_id)
    .bind(primary_worker_run_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(digest('d'))
    .bind(digest('e'))
    .bind(Uuid::new_v4())
    .bind(digest('f'))
    .bind(subtask_id)
    .bind(parent_work_item_id)
    .bind(parent_worker_request_id)
    .bind(parent_worker_run_id)
    .bind(digest('0'))
    .execute(&mut *tx)
    .await
    .expect("insert parent dispatch chain");
    tx.commit().await.expect("commit fixture seed");

    Fixture {
        session_id,
        authority_id,
        operation_id,
        stage_execution_id,
        owning_request_id,
        stage_run_unit_id,
        scope_snapshot_id,
        organization_id,
        stage_team_plan_id,
        leader_work_item_id,
        primary_worker_run_id,
        parent_work_item_id,
        parent_worker_run_id,
        parent_lease_token,
        task_plan_id,
        subtask_id,
        parent_dispatch_receipt_id,
    }
}

async fn seed_verification_task_stub(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    f: &Fixture,
    project_scope_id: Uuid,
    task_id: Uuid,
) -> (Uuid, String) {
    let revision_id = Uuid::new_v4();
    let plan_id = Uuid::new_v4();
    let assignment_set_id = Uuid::new_v4();
    let campaign_id = Uuid::new_v4();
    let semantic_attempt_fingerprint = digest('7');
    sqlx::query(
        r#"INSERT INTO hypothesis_verification_tasks(
               task_id,stable_task_key_sha256,operation_id,project_scope_id,
               stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id,
               hypothesis_revision_id,hypothesis_revision_sha256,verification_plan_id,
               verification_plan_sha256,relevant_evidence_snapshot_id,
               semantic_evidence_set_sha256,open_obligation_set_sha256,
               semantic_attempt_fingerprint,task_contract_version,
               first_admission_generation_id
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,
                    'hypothesis_verification_task.v1',$17)"#,
    )
    .bind(task_id)
    .bind(hash_json(&json!({"task_id": task_id})))
    .bind(f.operation_id)
    .bind(project_scope_id)
    .bind(f.stage_execution_id)
    .bind(f.stage_run_unit_id)
    .bind(f.scope_snapshot_id)
    .bind(f.organization_id)
    .bind(revision_id)
    .bind(digest('8'))
    .bind(plan_id)
    .bind(digest('9'))
    .bind(Uuid::new_v4())
    .bind(digest('a'))
    .bind(digest('b'))
    .bind(&semantic_attempt_fingerprint)
    .bind(Uuid::new_v4())
    .execute(&mut **tx)
    .await
    .expect("insert VerificationTask stub");
    sqlx::query(
        r#"INSERT INTO hypothesis_verification_task_assignment_sets(
               assignment_set_id,stable_request_id,task_id,hypothesis_revision_id,
               verification_plan_id,status,member_count,member_set_sha256,row_version,sealed_at
           ) VALUES($1,$2,$3,$4,$5,'sealed',1,$6,1,NOW())"#,
    )
    .bind(assignment_set_id)
    .bind(Uuid::new_v4())
    .bind(task_id)
    .bind(revision_id)
    .bind(plan_id)
    .bind(digest('c'))
    .execute(&mut **tx)
    .await
    .expect("insert VerificationTask assignment stub");
    sqlx::query(
        r#"INSERT INTO hypothesis_verification_task_campaigns(
               campaign_id,assignment_set_id,task_id,hypothesis_revision_id,
               verification_plan_id,plan_objective_id,verification_objective_id,
               reservation_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
    )
    .bind(campaign_id)
    .bind(assignment_set_id)
    .bind(task_id)
    .bind(revision_id)
    .bind(plan_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(digest('d'))
    .execute(&mut **tx)
    .await
    .expect("insert VerificationTask campaign stub");
    (assignment_set_id, semantic_attempt_fingerprint)
}

async fn seed_materialized_campaign_authority(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    f: &Fixture,
    project_scope_id: Uuid,
    task_id: Uuid,
) {
    let (
        campaign_id,
        revision_id,
        plan_id,
        plan_sha256,
        plan_objective_id,
        verification_objective_id,
    ): (Uuid, Uuid, Uuid, String, Uuid, Uuid) = sqlx::query_as(
        r#"SELECT reservation.campaign_id,task.hypothesis_revision_id,
                  task.verification_plan_id,task.verification_plan_sha256,
                  reservation.plan_objective_id,reservation.verification_objective_id
             FROM hypothesis_verification_tasks task
             JOIN hypothesis_verification_task_campaigns reservation
               ON reservation.task_id=task.task_id
            WHERE task.task_id=$1"#,
    )
    .bind(task_id)
    .fetch_one(&mut **tx)
    .await
    .expect("load reserved Campaign authority");
    let assessment_set_seal_id = Uuid::new_v4();
    let verification_contract_id = Uuid::new_v4();
    let verification_contract_hash = digest('e');
    let wave_denominator_id = Uuid::new_v4();
    let tool_truth_bundle_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO verification_capability_assessment_set_seals(
               assessment_set_seal_id,stable_request_id,operation_id,project_scope_id,
               organization_id,hypothesis_revision_id,verification_objective_id,
               verification_contract_hash,policy_snapshot_hash,source_snapshot_hash,
               registry_contract_hash,member_count,member_set_hash,seal_hash,sealed_at
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,1,$12,$13,NOW())"#,
    )
    .bind(assessment_set_seal_id)
    .bind(Uuid::new_v4())
    .bind(f.operation_id)
    .bind(project_scope_id)
    .bind(f.organization_id)
    .bind(revision_id)
    .bind(verification_objective_id)
    .bind(&verification_contract_hash)
    .bind(digest('f'))
    .bind(digest('0'))
    .bind(digest('1'))
    .bind(digest('2'))
    .bind(digest('3'))
    .execute(&mut **tx)
    .await
    .expect("seal materialized capability authority");
    sqlx::query(
        r#"INSERT INTO verification_wave_coverage_denominators(
               wave_denominator_id,stable_request_id,operation_id,project_scope_id,
               organization_id,generation_seal_id,contract_version,source_snapshot_hash,
               member_set_hash,member_count,sealed_at
           ) VALUES($1,$2,$3,$4,$5,$6,'verification-wave.v1',$7,$8,1,NOW())"#,
    )
    .bind(wave_denominator_id)
    .bind(Uuid::new_v4())
    .bind(f.operation_id)
    .bind(project_scope_id)
    .bind(f.organization_id)
    .bind(Uuid::new_v4())
    .bind(digest('a'))
    .bind(digest('b'))
    .execute(&mut **tx)
    .await
    .expect("seal materialized Wave authority");
    sqlx::query(
        r#"INSERT INTO tool_truth_authority_bundle_seals(
               id,operation_id,project_scope_id,project_path_at_freeze,scope_snapshot_id,
               organization_id,consumer_kind,stable_consumer_request_id,
               relevant_root_count,relevant_root_set_hash,member_count,member_set_hash,
               sealed_empty,semantic_authority_bundle_hash,freshness_attestation_bundle_hash,
               temporal_validity_bundle_hash,temporal_validity_policy_set_hash,
               target_state_epoch_set_hash,observation_window_completed_at,
               effective_valid_until,consistent_fresh_count,stale_or_invalid_count,sealed_at
           ) VALUES($1,$2,$3,'/fixture',$4,$5,'verification_campaign',$6,
                    4,$7,4,$8,FALSE,$9,$10,$11,$12,$13,NOW(),
                    NOW()+INTERVAL '1 hour',4,0,NOW())"#,
    )
    .bind(tool_truth_bundle_id)
    .bind(f.operation_id)
    .bind(project_scope_id)
    .bind(f.scope_snapshot_id)
    .bind(f.organization_id)
    .bind(Uuid::new_v4())
    .bind(digest('4'))
    .bind(digest('5'))
    .bind(digest('6'))
    .bind(digest('7'))
    .bind(digest('8'))
    .bind(digest('c'))
    .bind(digest('d'))
    .execute(&mut **tx)
    .await
    .expect("seal materialized Tool Truth authority");
    sqlx::query(
        r#"INSERT INTO verification_campaigns(
               campaign_id,stable_request_id,operation_id,project_scope_id,organization_id,
               hypothesis_revision_id,verification_plan_id,verification_plan_hash,
               plan_objective_id,verification_objective_id,verification_contract_id,
               verification_contract_hash,capability_assessment_set_seal_id,wave_denominator_id,
               tool_truth_authority_bundle_seal_id,relevant_root_set_hash,
               authority_member_set_hash,semantic_authority_bundle_hash,
               freshness_attestation_bundle_hash,temporal_validity_bundle_hash,
               effective_valid_until,campaign_version,state,source_snapshot_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,
                    NOW()+INTERVAL '1 hour',1,'admitted',$21)"#,
    )
    .bind(campaign_id)
    .bind(Uuid::new_v4())
    .bind(f.operation_id)
    .bind(project_scope_id)
    .bind(f.organization_id)
    .bind(revision_id)
    .bind(plan_id)
    .bind(plan_sha256)
    .bind(plan_objective_id)
    .bind(verification_objective_id)
    .bind(verification_contract_id)
    .bind(verification_contract_hash)
    .bind(assessment_set_seal_id)
    .bind(wave_denominator_id)
    .bind(tool_truth_bundle_id)
    .bind(digest('4'))
    .bind(digest('5'))
    .bind(digest('6'))
    .bind(digest('7'))
    .bind(digest('8'))
    .bind(digest('9'))
    .execute(&mut **tx)
    .await
    .expect("materialize reserved Campaign authority");
}

async fn seeded_verification_subject_fingerprint(
    db: &GolishDb,
    task_id: Uuid,
    assignment_set_id: Uuid,
    semantic_attempt_fingerprint: &str,
) -> String {
    let (revision_sha256, plan_sha256, assignment_sha256): (String, String, String) =
        sqlx::query_as(
            r#"SELECT task.hypothesis_revision_sha256,task.verification_plan_sha256,
                      assignment.member_set_sha256
                 FROM hypothesis_verification_tasks task
                 JOIN hypothesis_verification_task_assignment_sets assignment
                   ON assignment.task_id=task.task_id
                WHERE task.task_id=$1 AND assignment.assignment_set_id=$2"#,
        )
        .bind(task_id)
        .bind(assignment_set_id)
        .fetch_one(db.pool())
        .await
        .expect("load VerificationTask fingerprint material");
    let campaign_authority_sha256s: Vec<String> = sqlx::query_scalar(
        r#"SELECT unified_investigation_campaign_authority_sha256_v4(
                   reservation.campaign_id,reservation.reservation_sha256
               )
             FROM hypothesis_verification_task_campaigns reservation
             JOIN verification_campaigns campaign
               ON campaign.campaign_id=reservation.campaign_id
             JOIN verification_capability_assessment_set_seals assessment_set
               ON assessment_set.assessment_set_seal_id=campaign.capability_assessment_set_seal_id
              AND assessment_set.sealed_at IS NOT NULL
            WHERE reservation.task_id=$1 AND reservation.assignment_set_id=$2
            ORDER BY reservation.campaign_id"#,
    )
    .bind(task_id)
    .bind(assignment_set_id)
    .fetch_all(db.pool())
    .await
    .expect("load campaign denominator");
    let campaign_denominator_sha256: String = sqlx::query_scalar(
        "SELECT unified_investigation_exact_set_hash('verification_task_campaigns.v4',$1::TEXT[])",
    )
    .bind(campaign_authority_sha256s)
    .fetch_one(db.pool())
    .await
    .expect("hash campaign denominator");
    sqlx::query_scalar(
        r#"SELECT tool_truth_sha256(jsonb_build_object(
               'task_id',$1,'revision_sha256',$2,'plan_sha256',$3,
               'assignment_sha256',$4,'campaign_denominator_sha256',$5,
               'semantic_attempt_fingerprint',$6
           )::TEXT)"#,
    )
    .bind(task_id)
    .bind(revision_sha256)
    .bind(plan_sha256)
    .bind(assignment_sha256)
    .bind(campaign_denominator_sha256)
    .bind(semantic_attempt_fingerprint)
    .fetch_one(db.pool())
    .await
    .expect("hash VerificationTask subject")
}

fn begin_request(f: &Fixture, stable_request_id: Uuid) -> BeginInvestigationNestedDispatchRow {
    BeginInvestigationNestedDispatchRow {
        authority_id: f.authority_id,
        operation_id: f.operation_id,
        stage_execution_id: f.stage_execution_id,
        owning_stage_run_request_id: f.owning_request_id.clone(),
        stage_run_unit_id: f.stage_run_unit_id,
        scope_snapshot_id: f.scope_snapshot_id,
        organization_id: f.organization_id,
        stable_request_id,
        task_plan_id: f.task_plan_id,
        subtask_id: f.subtask_id,
        parent_dispatch_receipt_id: f.parent_dispatch_receipt_id,
        parent_fence: RuntimeMemoryTxFence {
            operation_id: f.operation_id,
            stage_execution_id: f.stage_execution_id,
            stage_run_unit_id: f.stage_run_unit_id,
            worker_run_id: f.parent_worker_run_id,
            lease_token: f.parent_lease_token,
            attempt_epoch: 1,
            expected_checkpoint_version: 1,
        },
        stage_team_plan_id: f.stage_team_plan_id,
        parent_work_item_id: f.parent_work_item_id,
        expected_dispatch_epoch: 0,
        nested_tool_request_id: "nested-tool-request-1".to_string(),
        requested_role: "coder".to_string(),
        objective: "Review the bounded analysis from another cognitive angle".to_string(),
        args_sha256: digest('1'),
        snapshot_sha256: digest('2'),
        dispatch_ordinal: 2,
        session_id: f.session_id,
        agent: AgentType::Coder,
        model: Some("fixture-model".to_string()),
        provider: Some("fixture-provider".to_string()),
        lease_owner: "nested-fixture".to_string(),
        lease_seconds: 600,
        initial_chain: json!([]),
        initial_checkpoint: json!([]),
    }
}

#[tokio::test]
#[serial]
async fn nested_begin_and_finish_are_atomic_exact_replays() {
    let (mut db, _data_dir) = migrated_db().await;
    let f = fixture(&db).await;
    let repository =
        PgInvestigationNestedDispatchRepository::new(std::sync::Arc::new(db.pool().clone()));
    let begin_id = Uuid::new_v4();
    let first = repository
        .begin(&begin_request(&f, begin_id))
        .await
        .expect("commit nested begin");
    assert!(!first.replayed);
    assert_eq!(first.task_plan_id, f.task_plan_id);
    assert_eq!(first.subtask_id, f.subtask_id);
    assert_eq!(
        first.parent_dispatch_receipt_id,
        f.parent_dispatch_receipt_id
    );
    assert_eq!(first.worker.status, "running");
    assert_eq!(first.work_item.status, "running");
    assert_eq!(first.dispatch.actor_kind, "nested_worker");
    assert_eq!(first.dispatch.worker_run_id, first.worker.id);
    assert_eq!(
        first.dispatch.stage_worker_request_id,
        Some(first.stage_worker_request_id)
    );

    let replay = repository
        .begin(&begin_request(&f, begin_id))
        .await
        .expect("replay exact nested begin");
    assert!(replay.replayed);
    assert_eq!(replay.worker.id, first.worker.id);
    assert_eq!(replay.worker.lease_token, first.worker.lease_token);
    assert_eq!(replay.message_chain_id, first.message_chain_id);
    assert_eq!(
        replay.dispatch.dispatch_receipt_id,
        first.dispatch.dispatch_receipt_id
    );
    let begin_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM investigation_nested_dispatch_begins WHERE stable_request_id=$1",
    )
    .bind(begin_id)
    .fetch_one(db.pool())
    .await
    .expect("count begin receipts");
    assert_eq!(begin_count, 1);
    let dependency_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM stage_work_item_dependencies WHERE work_item_id=$1 AND depends_on_work_item_id=$2",
    )
    .bind(f.leader_work_item_id)
    .bind(first.work_item.id)
    .fetch_one(db.pool())
    .await
    .expect("count leader dependency");
    assert_eq!(dependency_count, 1);

    let advisory_begin_id = Uuid::new_v4();
    let mut advisory_begin_request = begin_request(&f, advisory_begin_id);
    advisory_begin_request.nested_tool_request_id = "nested-tool-request-advisory".to_string();
    advisory_begin_request.dispatch_ordinal = 3;
    advisory_begin_request.objective = "Return bounded advisory planning material".to_string();
    let advisory = repository
        .begin(&advisory_begin_request)
        .await
        .expect("commit advisory nested begin");
    let advisory_fence = RuntimeMemoryTxFence {
        operation_id: f.operation_id,
        stage_execution_id: f.stage_execution_id,
        stage_run_unit_id: f.stage_run_unit_id,
        worker_run_id: advisory.worker.id,
        lease_token: advisory.worker.lease_token.expect("advisory child lease"),
        attempt_epoch: advisory.worker.attempt_epoch,
        expected_checkpoint_version: advisory.worker.checkpoint_version,
    };
    let advisory_output = json!({
        "kind": "bounded_nested_cognitive_result",
        "status": "found",
        "summary": "advisory-only planning material"
    });
    let advisory_output_hash = hash_json(&json!({
        "blocker_code": null,
        "canonical_output": advisory_output,
        "checked_empty_units": [],
        "disposition": "found",
        "evidence_ids": [],
        "fact_refs": [],
        "output_schema": "investigation_cognitive_output.v1",
        "work_item_id": advisory.work_item.id,
        "worker_run_id": advisory.worker.id,
    }));
    let advisory_finish = repository
        .finish(&FinishInvestigationNestedDispatchRow {
            authority_id: f.authority_id,
            operation_id: f.operation_id,
            stage_execution_id: f.stage_execution_id,
            owning_stage_run_request_id: f.owning_request_id.clone(),
            stage_run_unit_id: f.stage_run_unit_id,
            scope_snapshot_id: f.scope_snapshot_id,
            organization_id: f.organization_id,
            stable_request_id: Uuid::new_v4(),
            begin_receipt_id: advisory.begin_receipt_id,
            task_plan_id: f.task_plan_id,
            subtask_id: f.subtask_id,
            parent_dispatch_receipt_id: f.parent_dispatch_receipt_id,
            dispatch_receipt_id: advisory.dispatch.dispatch_receipt_id,
            child_fence: advisory_fence.clone(),
            stage_team_plan_id: f.stage_team_plan_id,
            work_item_id: advisory.work_item.id,
            expected_work_item_row_version: advisory.work_item.row_version,
            output: CompleteStageWorkerRow {
                fence: advisory_fence,
                team_plan_id: f.stage_team_plan_id,
                work_item_id: advisory.work_item.id,
                expected_work_item_row_version: advisory.work_item.row_version,
                output_schema: "investigation_cognitive_output.v1".to_string(),
                business_disposition: "found".to_string(),
                canonical_output: advisory_output,
                canonical_fact_refs: json!([]),
                evidence_ids: vec![],
                checked_empty_cells: json!([]),
                blocker_codes: vec![],
                output_hash: advisory_output_hash,
                terminal_checkpoint: json!({"nested_dispatch": "advisory_complete"}),
                evidence_watermark: None,
            },
            outcome: PentagiDispatchOutcome::Completed,
            result_sha256: digest('5'),
            fence_sha256: digest('6'),
        })
        .await
        .expect("commit advisory-only nested finish");
    assert_eq!(
        advisory_finish.completion.output.business_disposition,
        "found"
    );
    assert!(advisory_finish.completion.output.canonical_fact_refs == json!([]));
    assert!(advisory_finish.completion.output.evidence_ids.is_empty());
    assert_eq!(advisory_finish.dispatch_attempt.outcome, "completed");

    let child_fence = RuntimeMemoryTxFence {
        operation_id: f.operation_id,
        stage_execution_id: f.stage_execution_id,
        stage_run_unit_id: f.stage_run_unit_id,
        worker_run_id: first.worker.id,
        lease_token: first.worker.lease_token.expect("child lease token"),
        attempt_epoch: first.worker.attempt_epoch,
        expected_checkpoint_version: first.worker.checkpoint_version,
    };
    let canonical_output = json!({
        "kind": "bounded_nested_cognitive_result",
        "status": "blocked",
        "summary": "fixture intentionally has no evidence authority"
    });
    let output_hash = hash_json(&json!({
        "blocker_code": "INVESTIGATION_NESTED_FIXTURE_BLOCKED",
        "canonical_output": canonical_output,
        "checked_empty_units": [],
        "disposition": "blocked",
        "evidence_ids": [],
        "fact_refs": [],
        "output_schema": "investigation_cognitive_output.v1",
        "work_item_id": first.work_item.id,
        "worker_run_id": first.worker.id,
    }));
    let finish_id = Uuid::new_v4();
    let finish_input = FinishInvestigationNestedDispatchRow {
        authority_id: f.authority_id,
        operation_id: f.operation_id,
        stage_execution_id: f.stage_execution_id,
        owning_stage_run_request_id: f.owning_request_id.clone(),
        stage_run_unit_id: f.stage_run_unit_id,
        scope_snapshot_id: f.scope_snapshot_id,
        organization_id: f.organization_id,
        stable_request_id: finish_id,
        begin_receipt_id: first.begin_receipt_id,
        task_plan_id: f.task_plan_id,
        subtask_id: f.subtask_id,
        parent_dispatch_receipt_id: f.parent_dispatch_receipt_id,
        dispatch_receipt_id: first.dispatch.dispatch_receipt_id,
        child_fence: child_fence.clone(),
        stage_team_plan_id: f.stage_team_plan_id,
        work_item_id: first.work_item.id,
        expected_work_item_row_version: first.work_item.row_version,
        output: CompleteStageWorkerRow {
            fence: child_fence,
            team_plan_id: f.stage_team_plan_id,
            work_item_id: first.work_item.id,
            expected_work_item_row_version: first.work_item.row_version,
            output_schema: "investigation_cognitive_output.v1".to_string(),
            business_disposition: "blocked".to_string(),
            canonical_output,
            canonical_fact_refs: json!([]),
            evidence_ids: vec![],
            checked_empty_cells: json!([]),
            blocker_codes: vec!["INVESTIGATION_NESTED_FIXTURE_BLOCKED".to_string()],
            output_hash,
            terminal_checkpoint: json!({"nested_dispatch": "blocked"}),
            evidence_watermark: None,
        },
        outcome: PentagiDispatchOutcome::Blocked,
        result_sha256: digest('3'),
        fence_sha256: digest('4'),
    };
    let finished = repository
        .finish(&finish_input)
        .await
        .expect("commit nested finish");
    assert!(!finished.replayed);
    assert_eq!(finished.completion.worker.status, "passed");
    assert_eq!(finished.completion.work_item.status, "completed");
    assert_eq!(
        finished.dispatch_attempt.dispatch_receipt_id,
        first.dispatch.dispatch_receipt_id
    );
    assert_eq!(finished.dispatch_attempt.outcome, "blocked");

    let finish_replay = repository
        .finish(&finish_input)
        .await
        .expect("replay exact nested finish");
    assert!(finish_replay.replayed);
    assert_eq!(finish_replay.finish_receipt_id, finished.finish_receipt_id);
    assert_eq!(
        finish_replay.dispatch_attempt.dispatch_attempt_id,
        finished.dispatch_attempt.dispatch_attempt_id
    );
    let exact_counts: (i64, i64, i64) = sqlx::query_as(
        r#"SELECT
              (SELECT COUNT(*) FROM investigation_nested_dispatch_finishes WHERE stable_request_id=$1),
              (SELECT COUNT(*) FROM pentagi_logical_dispatch_attempts WHERE dispatch_receipt_id=$2),
              (SELECT COUNT(*) FROM stage_worker_outputs WHERE work_item_id=$3)"#,
    )
    .bind(finish_id)
    .bind(first.dispatch.dispatch_receipt_id)
    .bind(first.work_item.id)
    .fetch_one(db.pool())
    .await
    .expect("count exact terminal rows");
    assert_eq!(exact_counts, (1, 1, 1));

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn next_verification_task_primary_rearms_once_and_is_exactly_claimable() {
    let (mut db, _data_dir) = migrated_db().await;
    let f = fixture(&db).await;
    let task_id = Uuid::new_v4();
    let conflicting_task_id = Uuid::new_v4();
    let primary_chain_id = Uuid::new_v4();
    let project_scope_id: Uuid =
        sqlx::query_scalar("SELECT project_scope_id FROM operation_state WHERE operation_id=$1")
            .bind(f.operation_id)
            .fetch_one(db.pool())
            .await
            .expect("load project scope");
    let mut tx = db.pool().begin().await.expect("begin rearm fixture seed");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("isolate rearm fixture seed");
    sqlx::query(
        r#"INSERT INTO message_chains(id,session_id,task_id,agent,chain)
           VALUES($1,$2,$3,'primary','[]'::jsonb)"#,
    )
    .bind(primary_chain_id)
    .bind(f.session_id)
    .bind(f.operation_id)
    .execute(&mut *tx)
    .await
    .expect("insert prior Primary chain");
    sqlx::query(
        "UPDATE stage_worker_runs SET message_chain_id=$2
          WHERE id=$1 AND status='passed'",
    )
    .bind(f.primary_worker_run_id)
    .bind(primary_chain_id)
    .execute(&mut *tx)
    .await
    .expect("bind prior Primary chain");
    sqlx::query(
        "UPDATE stage_work_items
            SET status='completed',row_version=1,terminal_at=NOW(),updated_at=NOW()
          WHERE id=$1",
    )
    .bind(f.leader_work_item_id)
    .execute(&mut *tx)
    .await
    .expect("complete prior Primary item");
    sqlx::query(
        "UPDATE stage_work_items
            SET status='completed',row_version=1,terminal_at=NOW(),updated_at=NOW()
          WHERE id=$1",
    )
    .bind(f.parent_work_item_id)
    .execute(&mut *tx)
    .await
    .expect("complete prior cognitive item");
    sqlx::query(
        "UPDATE stage_worker_runs
            SET status='passed',checkpoint_version=checkpoint_version+1,
                lease_token=NULL,lease_owner=NULL,lease_acquired_at=NULL,
                lease_expires_at=NULL,heartbeat_at=NULL,terminal_at=NOW(),updated_at=NOW()
          WHERE id=$1",
    )
    .bind(f.parent_worker_run_id)
    .execute(&mut *tx)
    .await
    .expect("pass prior cognitive worker");
    sqlx::query(
        r#"INSERT INTO stage_worker_outputs(
               id,team_plan_id,work_item_id,worker_run_id,operation_id,stage_execution_id,
               stage_run_unit_id,scope_snapshot_id,organization_id,output_schema,output_version,
               business_disposition,canonical_output,canonical_fact_refs,evidence_ids,
               checked_empty_cells,blocker_codes,output_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'investigation_cognitive_output.v1',1,
                    'blocked','{}'::jsonb,'[]'::jsonb,'{}'::BIGINT[],'[]'::jsonb,
                    ARRAY['FIXTURE_TERMINAL'],$10)"#,
    )
    .bind(Uuid::new_v4())
    .bind(f.stage_team_plan_id)
    .bind(f.parent_work_item_id)
    .bind(f.parent_worker_run_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(f.stage_run_unit_id)
    .bind(f.scope_snapshot_id)
    .bind(f.organization_id)
    .bind(digest('e'))
    .execute(&mut *tx)
    .await
    .expect("insert prior cognitive output");
    sqlx::query(
        "UPDATE stage_team_plans
            SET requests_closed_at=NOW(),row_version=1,updated_at=NOW()
          WHERE id=$1",
    )
    .bind(f.stage_team_plan_id)
    .execute(&mut *tx)
    .await
    .expect("close prior task epoch");
    let (assignment_set_id, semantic_fingerprint) =
        seed_verification_task_stub(&mut tx, &f, project_scope_id, task_id).await;
    let (conflicting_assignment_set_id, conflicting_semantic_fingerprint) =
        seed_verification_task_stub(&mut tx, &f, project_scope_id, conflicting_task_id).await;
    seed_materialized_campaign_authority(&mut tx, &f, project_scope_id, task_id).await;
    seed_materialized_campaign_authority(&mut tx, &f, project_scope_id, conflicting_task_id).await;
    tx.commit().await.expect("commit rearm fixture seed");
    let subject_fingerprint = seeded_verification_subject_fingerprint(
        &db,
        task_id,
        assignment_set_id,
        &semantic_fingerprint,
    )
    .await;
    let conflicting_subject_fingerprint = seeded_verification_subject_fingerprint(
        &db,
        conflicting_task_id,
        conflicting_assignment_set_id,
        &conflicting_semantic_fingerprint,
    )
    .await;
    let cursor_request = LoadInvestigationRuntimeCursorRow {
        operation_id: f.operation_id,
        stage_execution_id: f.stage_execution_id,
        stage_run_unit_id: f.stage_run_unit_id,
        stage_team_plan_id: f.stage_team_plan_id,
    };
    let pending_cursor = load_investigation_runtime_cursor(db.pool(), &cursor_request)
        .await
        .expect("derive pending VerificationTask from durable state");
    assert_eq!(
        pending_cursor.phase,
        InvestigationRuntimeCursorPhaseRow::VerificationTask
    );
    assert!(matches!(
        pending_cursor.verification_task_id,
        Some(id) if id == task_id || id == conflicting_task_id
    ));
    let prior_fence = RuntimeMemoryTxFence {
        operation_id: f.operation_id,
        stage_execution_id: f.stage_execution_id,
        stage_run_unit_id: f.stage_run_unit_id,
        worker_run_id: f.primary_worker_run_id,
        lease_token: Uuid::new_v4(),
        attempt_epoch: 1,
        expected_checkpoint_version: 0,
    };
    let request = RearmInvestigationTaskPrimaryRow {
        previous_primary_fence: prior_fence.clone(),
        stage_team_plan_id: f.stage_team_plan_id,
        previous_primary_work_item_id: f.leader_work_item_id,
        verification_task_id: task_id,
        subject_fingerprint_sha256: subject_fingerprint,
        expected_plan_row_version: 1,
        expected_previous_work_item_row_version: 1,
    };
    let rearmed = rearm_investigation_task_primary(db.pool(), &request)
        .await
        .expect("atomically rearm next VerificationTask Primary");
    assert!(!rearmed.replayed);
    assert_eq!(rearmed.plan.dispatch_epoch, 1);
    assert!(rearmed.plan.requests_closed_at.is_none());
    assert_eq!(
        rearmed.primary_work_item.stable_key,
        format!("task:{task_id}:primary")
    );
    assert_eq!(rearmed.primary_work_item.status, "queued");
    assert_eq!(rearmed.primary_worker.status, "queued");
    assert_ne!(rearmed.primary_worker.id, f.primary_worker_run_id);
    assert_eq!(
        rearmed.primary_worker.message_chain_id,
        Some(rearmed.message_chain_id)
    );
    let rearmed_cursor = load_investigation_runtime_cursor(db.pool(), &cursor_request)
        .await
        .expect("resume the exact rearmed VerificationTask Primary");
    assert_eq!(
        rearmed_cursor.phase,
        InvestigationRuntimeCursorPhaseRow::VerificationTask
    );
    assert_eq!(rearmed_cursor.verification_task_id, Some(task_id));
    assert_eq!(rearmed_cursor.dispatch_epoch, rearmed.plan.dispatch_epoch);
    let replay = rearm_investigation_task_primary(db.pool(), &request)
        .await
        .expect("replay exact VerificationTask rearm");
    assert!(replay.replayed);
    assert_eq!(replay.primary_work_item.id, rearmed.primary_work_item.id);
    assert_eq!(replay.primary_worker.id, rearmed.primary_worker.id);
    let conflict = rearm_investigation_task_primary(
        db.pool(),
        &RearmInvestigationTaskPrimaryRow {
            verification_task_id: conflicting_task_id,
            subject_fingerprint_sha256: conflicting_subject_fingerprint,
            ..request.clone()
        },
    )
    .await
    .expect_err("a different VerificationTask cannot replay the open epoch");
    assert!(format!("{conflict}").contains("rearm_receipt_mismatch"));
    let task_plan_id = Uuid::new_v4();
    let primary_dispatch_receipt_id = Uuid::new_v4();
    let delegation_census_seal_id = Uuid::new_v4();
    let empty_residual_set_hash: String = sqlx::query_scalar(
        "SELECT investigation_exact_member_set_hash('investigation_primary_residual.v1',ARRAY[]::TEXT[])",
    )
    .fetch_one(db.pool())
    .await
    .expect("compute empty Primary residual set hash");
    let mut advisory_tx = db
        .pool()
        .begin()
        .await
        .expect("begin frozen advisory fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *advisory_tx)
        .await
        .expect("isolate frozen advisory fixture");
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_task_plans(
               task_plan_id,stable_request_id,run_request_id,authority_id,stage_team_plan_id,
               operation_id,stage_execution_id,owning_stage_run_request_id,
               stage_run_unit_id,scope_snapshot_id,organization_id,subject_kind,
               subject_id,subject_fingerprint_sha256,task_plan_version,task_plan_sha256,
               allowed_role_catalog,cognitive_tool_envelope_sha256,status,
               subtask_count,subtask_set_sha256,row_version,sealed_at
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,'verification_task',$12,$13,1,$14,
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
    .bind(task_id)
    .bind(&request.subject_fingerprint_sha256)
    .bind(digest('1'))
    .bind(digest('2'))
    .bind(digest('3'))
    .execute(&mut *advisory_tx)
    .await
    .expect("seed sealed VerificationTask PentAGI plan");
    sqlx::query(
        r#"INSERT INTO pentagi_logical_dispatch_receipts(
               dispatch_receipt_id,stable_request_id,logical_dispatch_key_sha256,
               task_plan_id,dispatch_ordinal,actor_kind,stage_work_item_id,worker_run_id,
               operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
               organization_id,transcript_request_id,snapshot_sha256,receipt_sha256
           ) VALUES($1,$2,$3,$4,0,'primary',$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"#,
    )
    .bind(primary_dispatch_receipt_id)
    .bind(Uuid::new_v4())
    .bind(digest('4'))
    .bind(task_plan_id)
    .bind(rearmed.primary_work_item.id)
    .bind(rearmed.primary_worker.id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(f.stage_run_unit_id)
    .bind(f.scope_snapshot_id)
    .bind(f.organization_id)
    .bind("verification-advisory-recovery-primary")
    .bind(digest('5'))
    .bind(digest('6'))
    .execute(&mut *advisory_tx)
    .await
    .expect("seed frozen Primary dispatch");
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_delegation_census_seals(
               census_seal_id,stable_request_id,task_plan_id,primary_dispatch_receipt_id,
               primary_worker_run_id,runnable_subtask_count,runnable_subtask_set_sha256,
               dispatch_count,dispatch_set_sha256,pipeline_event_count,
               pipeline_event_set_sha256,seal_sha256
           ) VALUES($1,$2,$3,$4,$5,0,$6,1,$7,0,$8,$9)"#,
    )
    .bind(delegation_census_seal_id)
    .bind(Uuid::new_v4())
    .bind(task_plan_id)
    .bind(primary_dispatch_receipt_id)
    .bind(rearmed.primary_worker.id)
    .bind(digest('7'))
    .bind(digest('8'))
    .bind(digest('9'))
    .bind(digest('a'))
    .execute(&mut *advisory_tx)
    .await
    .expect("seed frozen delegation census");
    sqlx::query(
        r#"INSERT INTO investigation_verification_task_advisory_receipts(
               advisory_receipt_id,stable_request_id,verification_task_id,authority_id,
               operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
               organization_id,hypothesis_revision_id,hypothesis_revision_sha256,
               verification_plan_id,verification_plan_sha256,assignment_set_id,
               assignment_set_sha256,campaign_denominator_sha256,
               subject_fingerprint_sha256,task_plan_id,delegation_census_seal_id,
               primary_worker_run_id,accepted_output_count,accepted_output_set_sha256,
               primary_residual_sha256,primary_residual_count,primary_residual_set_sha256,
               campaign_member_count,campaign_member_set_sha256,envelope_sha256,status)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,
                  1,$21,ARRAY[]::TEXT[],0,$22,1,$23,$24,'building')"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(task_id)
    .bind(f.authority_id)
    .bind(f.operation_id)
    .bind(f.stage_execution_id)
    .bind(f.stage_run_unit_id)
    .bind(f.scope_snapshot_id)
    .bind(f.organization_id)
    .bind(Uuid::new_v4())
    .bind(digest('b'))
    .bind(Uuid::new_v4())
    .bind(digest('c'))
    .bind(assignment_set_id)
    .bind(digest('d'))
    .bind(digest('e'))
    .bind(&request.subject_fingerprint_sha256)
    .bind(task_plan_id)
    .bind(delegation_census_seal_id)
    .bind(rearmed.primary_worker.id)
    .bind(digest('f'))
    .bind(&empty_residual_set_hash)
    .bind(digest('1'))
    .bind(digest('2'))
    .execute(&mut *advisory_tx)
    .await
    .expect("seed frozen VerificationTask advisory");
    sqlx::query(
        "UPDATE stage_team_plans SET requests_closed_at=NOW(),updated_at=NOW() WHERE id=$1",
    )
    .bind(f.stage_team_plan_id)
    .execute(&mut *advisory_tx)
    .await
    .expect("close plan after frozen advisory");
    advisory_tx
        .commit()
        .await
        .expect("commit frozen advisory recovery fixture");

    let ordinary_claim = claim_stage_team_leader(
        db.pool(),
        &ClaimStageTeamLeaderRow {
            claim: ClaimStageWorkItemRow {
                operation_id: f.operation_id,
                stage_execution_id: f.stage_execution_id,
                stage_run_unit_id: f.stage_run_unit_id,
                stage_team_plan_id: f.stage_team_plan_id,
                exact_work_item_id: Some(rearmed.primary_work_item.id),
                lease_owner: "verification-task-primary-fixture".to_string(),
                lease_seconds: 600,
                session_id: f.session_id,
                subtask_id: None,
                agent: AgentType::Primary,
                model: None,
                provider: None,
                parent_chain_id: None,
                initial_chain: json!([]),
                initial_checkpoint: json!([]),
            },
        },
    )
    .await
    .expect_err("ordinary leader claim must stay closed after advisory freeze");
    assert!(format!("{ordinary_claim}").contains("leader_not_claimable"));
    let claimed = recover_investigation_advisory_primary(
        db.pool(),
        &RecoverInvestigationAdvisoryPrimaryRow {
            verification_task_id: task_id,
            subject_fingerprint_sha256: request.subject_fingerprint_sha256.clone(),
            claim: ClaimStageWorkItemRow {
                operation_id: f.operation_id,
                stage_execution_id: f.stage_execution_id,
                stage_run_unit_id: f.stage_run_unit_id,
                stage_team_plan_id: f.stage_team_plan_id,
                exact_work_item_id: Some(rearmed.primary_work_item.id),
                lease_owner: "verification-advisory-recovery-fixture".to_string(),
                lease_seconds: 600,
                session_id: f.session_id,
                subtask_id: None,
                agent: AgentType::Primary,
                model: None,
                provider: None,
                parent_chain_id: None,
                initial_chain: json!([]),
                initial_checkpoint: json!([]),
            },
        },
    )
    .await
    .expect("claim exact closed-plan advisory recovery Primary")
    .expect("rearmed Primary is claimable");
    assert_eq!(claimed.work_item.id, rearmed.primary_work_item.id);
    assert_eq!(claimed.worker.id, rearmed.primary_worker.id);
    assert_eq!(claimed.message_chain_id, rearmed.message_chain_id);
    assert_eq!(claimed.plan.dispatch_epoch, 1);
    let exact_counts: (i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT
              (SELECT COUNT(*) FROM stage_work_items WHERE id=$1),
              (SELECT COUNT(*) FROM stage_worker_runs WHERE id=$2),
              (SELECT COUNT(*) FROM message_chains WHERE id=$3),
              (SELECT COUNT(*) FROM investigation_task_primary_rearms
                WHERE verification_task_id=$4 AND status='applied')"#,
    )
    .bind(rearmed.primary_work_item.id)
    .bind(rearmed.primary_worker.id)
    .bind(rearmed.message_chain_id)
    .bind(task_id)
    .fetch_one(db.pool())
    .await
    .expect("count exact rearm rows");
    assert_eq!(exact_counts, (1, 1, 1, 1));

    db.stop().await;
}
