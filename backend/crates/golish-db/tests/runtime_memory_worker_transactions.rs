use golish_db::models::{AgentType, NewSession, ToolcallStatus};
use golish_db::repo::{
    canonical_fact_refs, message_chains, operation_state, project_scopes, runtime_memory_rollout,
    runtime_memory_tx, sessions, stage_asset_waves, stage_deliverable_submissions, stage_handoffs,
    stage_run_units, stage_runs, stage_teams, stage_worker_runs, tasks, tool_calls,
};
use golish_db::{DbConfig, GolishDb};
use serial_test::serial;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const RUNTIME_MEMORY_V2_CUTOVER: &str =
    include_str!("../migrations/20260712000002_runtime_memory_v2_cutover.sql");

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
        database: format!("runtime_worker_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    };
    let db = GolishDb::start(config)
        .await
        .expect("start migrated embedded postgres");
    (db, data_dir)
}

/// Restore deployment defaults for tests that exercise pre-cutover contracts.
///
/// Both singletons are reset together so a future attack V2 cutover cannot make a legacy runtime
/// fixture freeze the invalid `(runtime=legacy, attack=v2_only)` contract combination.
async fn reset_deployment_rollouts_for_fixture(db: &GolishDb) {
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin rollout fixture reset");
    sqlx::raw_sql(
        r#"ALTER TABLE runtime_memory_rollout
               DISABLE TRIGGER runtime_memory_rollout_forward_only;
           ALTER TABLE runtime_memory_rollout
               DISABLE TRIGGER zz_runtime_memory_rollout_attestation_gate;
           ALTER TABLE runtime_memory_rollout
               DISABLE TRIGGER zz_runtime_memory_rollout_promotion_receipt;
           ALTER TABLE attack_execution_rollout
               DISABLE TRIGGER attack_execution_rollout_forward_only;
           ALTER TABLE attack_execution_rollout
               DISABLE TRIGGER zz_attack_runtime_rollout_compatibility;
           ALTER TABLE attack_execution_rollout
               DISABLE TRIGGER zz_attack_execution_rollout_promotion_receipt;
           UPDATE attack_execution_rollout
              SET contract='legacy', rank=0, row_version=0, updated_at=NOW()
            WHERE singleton=TRUE;
           UPDATE runtime_memory_rollout
              SET contract='legacy_v1', contract_rank=0, row_version=0, updated_at=NOW()
            WHERE singleton_id=1;
           ALTER TABLE runtime_memory_rollout
               ENABLE TRIGGER runtime_memory_rollout_forward_only;
           ALTER TABLE runtime_memory_rollout
               ENABLE TRIGGER zz_runtime_memory_rollout_attestation_gate;
           ALTER TABLE runtime_memory_rollout
               ENABLE TRIGGER zz_runtime_memory_rollout_promotion_receipt;
           ALTER TABLE attack_execution_rollout
               ENABLE TRIGGER attack_execution_rollout_forward_only;
           ALTER TABLE attack_execution_rollout
               ENABLE TRIGGER zz_attack_runtime_rollout_compatibility;
           ALTER TABLE attack_execution_rollout
               ENABLE TRIGGER zz_attack_execution_rollout_promotion_receipt;"#,
    )
    .execute(&mut *tx)
    .await
    .expect("reset rollout singletons for fixture");
    tx.commit().await.expect("commit rollout fixture reset");
}

/// Test-only deployment control for selector/state-machine coverage. Production
/// promotion is always attestation-gated; fixtures may freeze an operation at
/// each historical contract without manufacturing rollout evidence.
async fn set_runtime_rollout_for_fixture(
    db: &GolishDb,
    contract: runtime_memory_rollout::RuntimeMemoryContract,
) {
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin runtime rollout fixture");
    sqlx::raw_sql(
        r#"ALTER TABLE runtime_memory_rollout
               DISABLE TRIGGER runtime_memory_rollout_forward_only;
           ALTER TABLE runtime_memory_rollout
               DISABLE TRIGGER zz_runtime_memory_rollout_attestation_gate;
           ALTER TABLE runtime_memory_rollout
               DISABLE TRIGGER zz_runtime_memory_rollout_promotion_receipt;"#,
    )
    .execute(&mut *tx)
    .await
    .expect("disable production rollout gates for explicit fixture");
    sqlx::query(
        r#"UPDATE runtime_memory_rollout
              SET contract=$1,contract_rank=$2,row_version=$2,updated_at=NOW()
            WHERE singleton_id=1"#,
    )
    .bind(contract.as_str())
    .bind(contract.rank())
    .execute(&mut *tx)
    .await
    .expect("set explicit frozen runtime contract fixture");
    sqlx::raw_sql(
        r#"ALTER TABLE runtime_memory_rollout
               ENABLE TRIGGER runtime_memory_rollout_forward_only;
           ALTER TABLE runtime_memory_rollout
               ENABLE TRIGGER zz_runtime_memory_rollout_attestation_gate;
           ALTER TABLE runtime_memory_rollout
               ENABLE TRIGGER zz_runtime_memory_rollout_promotion_receipt;"#,
    )
    .execute(&mut *tx)
    .await
    .expect("restore production rollout gates after fixture setup");
    tx.commit().await.expect("commit runtime rollout fixture");
}

#[tokio::test]
#[serial]
async fn complete_migrations_enable_runtime_sampling_and_new_operation_freezes_it() {
    let (mut db, _data_dir) = fixture().await;

    let migrated_rollout = runtime_memory_rollout::get(db.pool())
        .await
        .expect("read fully migrated runtime rollout");
    assert_eq!(migrated_rollout.contract, "dual_write_legacy_read");
    assert_eq!(migrated_rollout.contract_rank, 1);
    assert_eq!(migrated_rollout.row_version, 1);

    let v2_session_id = sessions::create(
        db.pool(),
        NewSession {
            title: Some("post-cutover runtime operation".to_string()),
            workspace_path: Some("/tmp/runtime-worker-cutover-v2".to_string()),
            workspace_label: None,
            model: None,
            provider: None,
            project_path: Some("/tmp/runtime-worker-cutover-v2".to_string()),
        },
    )
    .await
    .expect("create post-cutover session")
    .id;
    let v2_project = project_scopes::register_first_open(
        db.pool(),
        "/tmp/runtime-worker-cutover-v2",
        "runtime-worker-cutover-v2-sha",
    )
    .await
    .expect("register post-cutover project");
    let v2_operation_id = Uuid::new_v4();
    let created_v2 = runtime_memory_tx::create_runtime_operation(
        db.pool(),
        &runtime_memory_tx::CreateRuntimeOperationRow {
            operation_id: v2_operation_id,
            initial_stage_execution_id: Uuid::new_v4(),
            session_id: v2_session_id,
            title: Some("post-cutover operation".to_string()),
            input: "freeze sampling runtime contract".to_string(),
            profile: "assessment".to_string(),
            entry_stage: "scoping".to_string(),
            application_model_contract: golish_core::ApplicationModelContract::LegacyNoModel,
            project_scope_id: v2_project.project_scope_id,
            cli_scope: None,
        },
    )
    .await
    .expect("new operation must freeze post-cutover rollout");
    assert_eq!(
        created_v2.operation.runtime_memory_contract,
        "dual_write_legacy_read"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn stage_team_schema_fences_owner_queue_output_and_request_epoch() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots(&db).await;
    let unit_id = Uuid::new_v4();
    stage_run_units::insert_with_executor(
        db.pool(),
        &stage_run_units::NewStageRunUnit {
            id: unit_id,
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            scope_snapshot_id: roots.snapshot_id,
            organization_id: roots.organization_id,
            stage_kind: "target_intel".to_string(),
            generation: 0,
            specialist: Some("target_intel".to_string()),
        },
    )
    .await
    .expect("seed team-owned stage unit");

    let plan_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO stage_team_plans (
               id,operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
               organization_id,stage_kind,unit_generation,schema_version,plan_version,
               plan_hash,leader_role,aggregator_kind,aggregator_role,allowed_worker_roles,
               max_workers_total,max_workers_active,dynamic_requests_allowed,
               dynamic_request_policy,dispatch_epoch,final_submitter_kind,
               created_from_stage_spec_hash
           ) VALUES (
               $1,$2,$3,$4,$5,$6,'target_intel',0,1,1,$7,'lead','worker','aggregator',
               $8,8,2,TRUE,$9,0,'worker',$10
           )"#,
    )
    .bind(plan_id)
    .bind(roots.operation_id)
    .bind(roots.stage_execution_id)
    .bind(unit_id)
    .bind(roots.snapshot_id)
    .bind(roots.organization_id)
    .bind("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind(serde_json::json!(["lead", "helper", "aggregator"]))
    .bind(serde_json::json!({"max_dynamic_requests": 2}))
    .bind("sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    .execute(db.pool())
    .await
    .expect("insert frozen team plan");

    let duplicate_plan = sqlx::query(
        r#"INSERT INTO stage_team_plans (
               id,operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
               organization_id,stage_kind,unit_generation,schema_version,plan_version,
               plan_hash,leader_role,aggregator_kind,aggregator_role,allowed_worker_roles,
               max_workers_total,max_workers_active,dynamic_requests_allowed,
               dynamic_request_policy,dispatch_epoch,final_submitter_kind,
               created_from_stage_spec_hash
           ) SELECT $1,operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
                    organization_id,stage_kind,unit_generation,schema_version,plan_version+1,
                    $2,leader_role,aggregator_kind,aggregator_role,allowed_worker_roles,
                    max_workers_total,max_workers_active,dynamic_requests_allowed,
                    dynamic_request_policy,dispatch_epoch,final_submitter_kind,
                    created_from_stage_spec_hash
               FROM stage_team_plans WHERE id=$3"#,
    )
    .bind(Uuid::new_v4())
    .bind("sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc")
    .bind(plan_id)
    .execute(db.pool())
    .await;
    assert!(
        duplicate_plan.is_err(),
        "one Unit must own one frozen TeamPlan"
    );

    let producer_item_id = Uuid::new_v4();
    let helper_item_id = Uuid::new_v4();
    for (item_id, stable_key, role, hash_char) in [
        (producer_item_id, "provider:primary", "lead", 'd'),
        (helper_item_id, "provider:helper", "helper", 'e'),
    ] {
        sqlx::query(
            r#"INSERT INTO stage_work_items (
                   id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                   scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
                   input_manifest_hash,input_refs,required_for_barrier,priority,status,
                   attempt_policy,budget,output_schema,created_by
               ) VALUES (
                   $1,$2,$3,$4,$5,$6,$7,0,'source_batch',$8,$9,$10,'[]'::jsonb,
                   TRUE,0,'queued','{}'::jsonb,'{}'::jsonb,
                   'stage_worker_output.v1','server_seed'
               )"#,
        )
        .bind(item_id)
        .bind(plan_id)
        .bind(roots.operation_id)
        .bind(roots.stage_execution_id)
        .bind(unit_id)
        .bind(roots.snapshot_id)
        .bind(roots.organization_id)
        .bind(stable_key)
        .bind(role)
        .bind(format!("sha256:{}", hash_char.to_string().repeat(64)))
        .execute(db.pool())
        .await
        .expect("insert stable server work item");
    }

    let duplicate_stable_key = sqlx::query(
        r#"INSERT INTO stage_work_items (
               id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
               input_manifest_hash,input_refs,required_for_barrier,priority,status,
               attempt_policy,budget,output_schema,created_by
           ) VALUES ($1,$2,$3,$4,$5,$6,$7,0,'source_batch','provider:primary','helper',
                     $8,'[]'::jsonb,FALSE,0,'queued','{}'::jsonb,'{}'::jsonb,
                     'stage_worker_output.v1','server_seed')"#,
    )
    .bind(Uuid::new_v4())
    .bind(plan_id)
    .bind(roots.operation_id)
    .bind(roots.stage_execution_id)
    .bind(unit_id)
    .bind(roots.snapshot_id)
    .bind(roots.organization_id)
    .bind("sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")
    .execute(db.pool())
    .await;
    assert!(
        duplicate_stable_key.is_err(),
        "stable work keys cannot duplicate"
    );

    let cross_owner_item = sqlx::query(
        r#"INSERT INTO stage_work_items (
               id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
               input_manifest_hash,input_refs,required_for_barrier,priority,status,
               attempt_policy,budget,output_schema,created_by
           ) VALUES ($1,$2,$3,$4,$5,$6,$7,0,'source_batch','foreign-org','helper',
                     $8,'[]'::jsonb,FALSE,0,'queued','{}'::jsonb,'{}'::jsonb,
                     'stage_worker_output.v1','server_seed')"#,
    )
    .bind(Uuid::new_v4())
    .bind(plan_id)
    .bind(roots.operation_id)
    .bind(roots.stage_execution_id)
    .bind(unit_id)
    .bind(roots.snapshot_id)
    .bind(Uuid::new_v4())
    .bind("sha256:1111111111111111111111111111111111111111111111111111111111111111")
    .execute(db.pool())
    .await;
    assert!(
        cross_owner_item.is_err(),
        "WorkItems cannot cross the owner tuple"
    );

    sqlx::query(
        r#"INSERT INTO stage_work_item_dependencies (
               team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,work_item_id,depends_on_work_item_id
           ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)"#,
    )
    .bind(plan_id)
    .bind(roots.operation_id)
    .bind(roots.stage_execution_id)
    .bind(unit_id)
    .bind(roots.snapshot_id)
    .bind(roots.organization_id)
    .bind(helper_item_id)
    .bind(producer_item_id)
    .execute(db.pool())
    .await
    .expect("insert same-owner dependency");
    let dependency_cycle = sqlx::query(
        r#"INSERT INTO stage_work_item_dependencies (
               team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,work_item_id,depends_on_work_item_id
           ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)"#,
    )
    .bind(plan_id)
    .bind(roots.operation_id)
    .bind(roots.stage_execution_id)
    .bind(unit_id)
    .bind(roots.snapshot_id)
    .bind(roots.organization_id)
    .bind(producer_item_id)
    .bind(helper_item_id)
    .execute(db.pool())
    .await;
    assert!(
        dependency_cycle.is_err(),
        "WorkItem dependencies cannot cycle"
    );

    let unbound_worker = sqlx::query(
        r#"INSERT INTO stage_worker_runs (
               id,operation_id,stage_execution_id,stage_run_unit_id,organization_id,
               worker_generation,specialist,work_item_kind,work_item_key,agent_path,status
           ) VALUES ($1,$2,$3,$4,$5,0,'lead','source_batch','unbound',
                     'main>team:unbound','queued')"#,
    )
    .bind(Uuid::new_v4())
    .bind(roots.operation_id)
    .bind(roots.stage_execution_id)
    .bind(unit_id)
    .bind(roots.organization_id)
    .execute(db.pool())
    .await;
    assert!(
        unbound_worker.is_err(),
        "team Workers must bind an exact WorkItem"
    );

    let producer_worker_id = Uuid::new_v4();
    let helper_worker_id = Uuid::new_v4();
    for (worker_id, item_id, specialist, stable_key) in [
        (
            producer_worker_id,
            producer_item_id,
            "lead",
            "provider:primary",
        ),
        (
            helper_worker_id,
            helper_item_id,
            "helper",
            "provider:helper",
        ),
    ] {
        sqlx::query(
            r#"INSERT INTO stage_worker_runs (
                   id,operation_id,stage_execution_id,stage_run_unit_id,organization_id,
                   worker_generation,specialist,work_item_kind,work_item_key,agent_path,
                   status,work_item_id
               ) VALUES ($1,$2,$3,$4,$5,0,$6,'source_batch',$7,$8,'queued',$9)"#,
        )
        .bind(worker_id)
        .bind(roots.operation_id)
        .bind(roots.stage_execution_id)
        .bind(unit_id)
        .bind(roots.organization_id)
        .bind(specialist)
        .bind(stable_key)
        .bind(format!("main>team:{specialist}"))
        .bind(item_id)
        .execute(db.pool())
        .await
        .expect("insert independently fenced sibling Worker");
    }
    let duplicate_live_worker = sqlx::query(
        r#"INSERT INTO stage_worker_runs (
               id,operation_id,stage_execution_id,stage_run_unit_id,organization_id,
               worker_generation,specialist,work_item_kind,work_item_key,agent_path,
               status,work_item_id
           ) VALUES ($1,$2,$3,$4,$5,1,'lead','source_batch','provider:primary',
                     'main>team:replacement','queued',$6)"#,
    )
    .bind(Uuid::new_v4())
    .bind(roots.operation_id)
    .bind(roots.stage_execution_id)
    .bind(unit_id)
    .bind(roots.organization_id)
    .bind(producer_item_id)
    .execute(db.pool())
    .await;
    assert!(
        duplicate_live_worker.is_err(),
        "one WorkItem cannot own two live Workers"
    );

    let dynamic_item_id = Uuid::new_v4();
    let mut request_tx = db.pool().begin().await.expect("begin accepted request");
    sqlx::query(
        r#"INSERT INTO stage_work_items (
               id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
               input_manifest_hash,input_refs,required_for_barrier,priority,status,
               attempt_policy,budget,output_schema,created_by
           ) VALUES ($1,$2,$3,$4,$5,$6,$7,0,'enrichment','dynamic:whois','helper',
                     $8,'[]'::jsonb,FALSE,1,'queued','{}'::jsonb,'{}'::jsonb,
                     'stage_worker_output.v1','accepted_worker_request')"#,
    )
    .bind(dynamic_item_id)
    .bind(plan_id)
    .bind(roots.operation_id)
    .bind(roots.stage_execution_id)
    .bind(unit_id)
    .bind(roots.snapshot_id)
    .bind(roots.organization_id)
    .bind("sha256:2222222222222222222222222222222222222222222222222222222222222222")
    .execute(&mut *request_tx)
    .await
    .expect("insert dynamic sibling WorkItem");
    sqlx::query(
        r#"INSERT INTO stage_worker_requests (
               id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,parent_work_item_id,parent_worker_run_id,
               dispatch_epoch,requested_role,request_kind,bounded_subject_refs,reason_code,
               expected_output_schema,budget_hint,dedupe_key,request_payload_hash,status,
               accepted_work_item_id
           ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,0,'helper','enrichment','[]'::jsonb,
                     'missing_whois','stage_worker_output.v1','{}'::jsonb,'whois:root',
                     $10,'accepted',$11)"#,
    )
    .bind(Uuid::new_v4())
    .bind(plan_id)
    .bind(roots.operation_id)
    .bind(roots.stage_execution_id)
    .bind(unit_id)
    .bind(roots.snapshot_id)
    .bind(roots.organization_id)
    .bind(producer_item_id)
    .bind(producer_worker_id)
    .bind("sha256:3333333333333333333333333333333333333333333333333333333333333333")
    .bind(dynamic_item_id)
    .execute(&mut *request_tx)
    .await
    .expect("record accepted dynamic WorkerRequest");
    request_tx
        .commit()
        .await
        .expect("accepted request and WorkItem commit together");

    let duplicate_request = sqlx::query(
        r#"INSERT INTO stage_worker_requests (
               id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,parent_work_item_id,parent_worker_run_id,
               dispatch_epoch,requested_role,request_kind,bounded_subject_refs,reason_code,
               expected_output_schema,budget_hint,dedupe_key,request_payload_hash,status
           ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,0,'helper','enrichment','[]'::jsonb,
                     'duplicate','stage_worker_output.v1','{}'::jsonb,'whois:root',
                     $10,'rejected')"#,
    )
    .bind(Uuid::new_v4())
    .bind(plan_id)
    .bind(roots.operation_id)
    .bind(roots.stage_execution_id)
    .bind(unit_id)
    .bind(roots.snapshot_id)
    .bind(roots.organization_id)
    .bind(producer_item_id)
    .bind(producer_worker_id)
    .bind("sha256:4444444444444444444444444444444444444444444444444444444444444444")
    .execute(db.pool())
    .await;
    assert!(
        duplicate_request.is_err(),
        "request dedupe is exact per epoch"
    );

    sqlx::query(
        "UPDATE stage_team_plans SET requests_closed_at=NOW(),row_version=row_version+1,updated_at=NOW() WHERE id=$1",
    )
    .bind(plan_id)
    .execute(db.pool())
    .await
    .expect("close current request epoch");
    let request_after_close = sqlx::query(
        r#"INSERT INTO stage_worker_requests (
               id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,parent_work_item_id,parent_worker_run_id,
               dispatch_epoch,requested_role,request_kind,bounded_subject_refs,reason_code,
               expected_output_schema,budget_hint,dedupe_key,request_payload_hash,status
           ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,0,'helper','dns','[]'::jsonb,
                     'late','stage_worker_output.v1','{}'::jsonb,'late:dns',$10,'accepted')"#,
    )
    .bind(Uuid::new_v4())
    .bind(plan_id)
    .bind(roots.operation_id)
    .bind(roots.stage_execution_id)
    .bind(unit_id)
    .bind(roots.snapshot_id)
    .bind(roots.organization_id)
    .bind(producer_item_id)
    .bind(producer_worker_id)
    .bind("sha256:5555555555555555555555555555555555555555555555555555555555555555")
    .execute(db.pool())
    .await;
    assert!(
        request_after_close.is_err(),
        "closed epochs reject new accepted requests"
    );
    let plan_drift = sqlx::query(
        "UPDATE stage_team_plans SET max_workers_active=max_workers_active+1,row_version=row_version+1,updated_at=NOW() WHERE id=$1",
    )
    .bind(plan_id)
    .execute(db.pool())
    .await;
    assert!(
        plan_drift.is_err(),
        "the frozen TeamPlan contract is immutable"
    );

    sqlx::query(
        "UPDATE stage_work_items SET status='running',started_at=NOW(),row_version=row_version+1,updated_at=NOW() WHERE id=$1",
    )
    .bind(producer_item_id)
    .execute(db.pool())
    .await
    .expect("start producer WorkItem");
    sqlx::query(
        "UPDATE stage_work_items SET status='completed',terminal_at=NOW(),row_version=row_version+1,updated_at=NOW() WHERE id=$1",
    )
    .bind(producer_item_id)
    .execute(db.pool())
    .await
    .expect("complete producer WorkItem");
    sqlx::query(
        "UPDATE stage_worker_runs SET status='passed',terminal_at=NOW(),updated_at=NOW() WHERE id=$1",
    )
    .bind(producer_worker_id)
    .execute(db.pool())
    .await
    .expect("terminalize producer Worker");
    let output_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO stage_worker_outputs (
               id,team_plan_id,work_item_id,worker_run_id,operation_id,stage_execution_id,
               stage_run_unit_id,scope_snapshot_id,organization_id,output_schema,
               output_version,business_disposition,canonical_output,canonical_fact_refs,evidence_ids,
               checked_empty_cells,blocker_codes,output_hash
           ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,'stage_worker_output.v1',1,'found',
                     '{"facts":[]}'::jsonb,'[]'::jsonb,ARRAY[42]::bigint[],
                     '[]'::jsonb,ARRAY[]::text[],$10)"#,
    )
    .bind(output_id)
    .bind(plan_id)
    .bind(producer_item_id)
    .bind(producer_worker_id)
    .bind(roots.operation_id)
    .bind(roots.stage_execution_id)
    .bind(unit_id)
    .bind(roots.snapshot_id)
    .bind(roots.organization_id)
    .bind("sha256:6666666666666666666666666666666666666666666666666666666666666666")
    .execute(db.pool())
    .await
    .expect("persist one terminal WorkerOutput");
    let output_mutation = sqlx::query(
        "UPDATE stage_worker_outputs SET blocker_codes=ARRAY['drift']::text[] WHERE id=$1",
    )
    .bind(output_id)
    .execute(db.pool())
    .await;
    assert!(output_mutation.is_err(), "WorkerOutput is immutable");
    let duplicate_output = sqlx::query(
        r#"INSERT INTO stage_worker_outputs (
               id,team_plan_id,work_item_id,worker_run_id,operation_id,stage_execution_id,
               stage_run_unit_id,scope_snapshot_id,organization_id,output_schema,
               output_version,business_disposition,canonical_output,canonical_fact_refs,evidence_ids,
               checked_empty_cells,blocker_codes,output_hash
           ) SELECT $1,team_plan_id,work_item_id,worker_run_id,operation_id,stage_execution_id,
                    stage_run_unit_id,scope_snapshot_id,organization_id,output_schema,
                    output_version,business_disposition,canonical_output,canonical_fact_refs,evidence_ids,
                    checked_empty_cells,blocker_codes,$2
               FROM stage_worker_outputs WHERE id=$3"#,
    )
    .bind(Uuid::new_v4())
    .bind("sha256:7777777777777777777777777777777777777777777777777777777777777777")
    .bind(output_id)
    .execute(db.pool())
    .await;
    assert!(
        duplicate_output.is_err(),
        "one terminal Worker has one output"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn stage_team_repo_closes_dynamic_queue_and_recovers_expired_aggregator() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    let hash = |character: char| format!("sha256:{}", character.to_string().repeat(64));
    let team_seed = runtime_memory_tx::SeedStageTeamRuntimeRow {
        base: runtime_memory_tx::SeedStageRuntimeRow {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_kind: "target_intel".to_string(),
            unit_generation: 0,
            specialist: "producer".to_string(),
            worker_generation: 0,
            work_item_kind: "stage_unit".to_string(),
            work_item_key: "team".to_string(),
            agent_path_prefix: "main>stage_run:target_intel".to_string(),
            organization_ids: Some(vec![roots.organization_id]),
        },
        plan: runtime_memory_tx::StageTeamPlanSeedRow {
            schema_version: 1,
            plan_version: 1,
            plan_hash: hash('a'),
            leader_role: "aggregator".to_string(),
            allowed_roles: vec![
                "producer".to_string(),
                "helper".to_string(),
                "aggregator".to_string(),
            ],
            aggregator_kind: "worker".to_string(),
            aggregator_role: Some("aggregator".to_string()),
            max_workers_total: 4,
            max_workers_active: 2,
            dynamic_requests_enabled: true,
            dynamic_request_policy: serde_json::json!({
                "allowed_request_kinds": ["enrichment"],
                "canonical_subject_refs_only": true,
                "max_requests": 1,
                "max_subject_refs": 1,
            }),
            final_submitter_kind: "worker".to_string(),
            created_from_stage_spec_hash: hash('b'),
        },
        work_items: vec![
            runtime_memory_tx::StageWorkItemSeedRow {
                stable_key: "producer:root".to_string(),
                work_item_kind: "stage_axis".to_string(),
                role: "producer".to_string(),
                input_manifest: serde_json::json!({"axis": "root"}),
                input_manifest_hash: hash('c'),
                conflict_key: None,
                priority: 0,
                required_for_barrier: true,
                is_aggregator: false,
                attempt_policy: serde_json::json!({"max_attempts": 2}),
                budget: serde_json::json!({}),
                output_schema: "stage_worker_output.v1".to_string(),
                created_by: "server_seed".to_string(),
            },
            runtime_memory_tx::StageWorkItemSeedRow {
                stable_key: "aggregator:final".to_string(),
                work_item_kind: "stage_aggregate".to_string(),
                role: "aggregator".to_string(),
                input_manifest: serde_json::json!({"aggregate": true}),
                input_manifest_hash: hash('d'),
                conflict_key: Some("stage_unit_finalizer".to_string()),
                priority: i32::MAX,
                required_for_barrier: false,
                is_aggregator: true,
                attempt_policy: serde_json::json!({"max_attempts": 2}),
                budget: serde_json::json!({}),
                output_schema: "stage_unit_aggregate.v1".to_string(),
                created_by: "server_seed".to_string(),
            },
        ],
    };
    let seeded = runtime_memory_tx::seed_stage_team_runtime(db.pool(), &team_seed)
        .await
        .expect("seed one durable team");
    let seeded = &seeded[0];
    let claim = || runtime_memory_tx::ClaimStageWorkItemRow {
        operation_id: roots.operation_id,
        stage_execution_id: roots.stage_execution_id,
        stage_run_unit_id: seeded.unit.id,
        stage_team_plan_id: seeded.plan.id,
        exact_work_item_id: None,
        lease_owner: "team-fixture".to_string(),
        lease_seconds: 60,
        session_id: roots.session_id,
        subtask_id: None,
        agent: AgentType::Pentester,
        model: None,
        provider: None,
        parent_chain_id: None,
        initial_chain: serde_json::json!([]),
        initial_checkpoint: serde_json::json!({"turn": 0}),
    };
    let producer = runtime_memory_tx::claim_stage_work_item(db.pool(), &claim())
        .await
        .expect("claim producer")
        .expect("producer queued");
    assert_eq!(producer.work_item.role, "producer");
    let producer_fence = runtime_memory_tx::RuntimeMemoryTxFence {
        operation_id: roots.operation_id,
        stage_execution_id: roots.stage_execution_id,
        stage_run_unit_id: seeded.unit.id,
        worker_run_id: producer.worker.id,
        lease_token: producer.worker.lease_token.expect("producer lease"),
        attempt_epoch: producer.worker.attempt_epoch,
        expected_checkpoint_version: producer.worker.checkpoint_version,
    };
    let running_unit = stage_run_units::get(db.pool(), seeded.unit.id)
        .await
        .expect("load claimed Team Unit")
        .expect("claimed Team Unit exists");
    for (next_worker_status, next_unit_status) in [
        (
            stage_worker_runs::StageWorkerRunStatus::GateBlocked,
            stage_run_units::StageRunUnitStatus::GateBlocked,
        ),
        (
            stage_worker_runs::StageWorkerRunStatus::Exhausted,
            stage_run_units::StageRunUnitStatus::Exhausted,
        ),
        (
            stage_worker_runs::StageWorkerRunStatus::Superseded,
            stage_run_units::StageRunUnitStatus::Superseded,
        ),
    ] {
        let error = runtime_memory_tx::finish_worker_attempt(
            db.pool(),
            &runtime_memory_tx::FinishWorkerAttemptRow {
                fence: producer_fence.clone(),
                expected_status: stage_worker_runs::StageWorkerRunStatus::Running,
                next_status: next_worker_status,
                expected_unit_status: stage_run_units::StageRunUnitStatus::Running,
                expected_unit_row_version: running_unit.row_version,
                next_unit_status,
                checkpoint: serde_json::json!({"forbidden": "generic-team-finish"}),
                evidence_watermark: None,
            },
        )
        .await
        .expect_err("generic finish must not terminalize a Team-owned Unit");
        assert!(matches!(
            error,
            runtime_memory_tx::RuntimeMemoryStoreError::Conflict {
                code: "stage_team_worker_requires_team_lifecycle"
            }
        ));
    }
    assert_eq!(
        stage_run_units::get(db.pool(), seeded.unit.id)
            .await
            .expect("reload Team Unit")
            .expect("Team Unit remains")
            .status,
        "running"
    );
    assert_eq!(
        stage_worker_runs::get(db.pool(), producer.worker.id)
            .await
            .expect("reload Team producer")
            .expect("Team producer remains")
            .status,
        "running"
    );
    let canonical_organization_ref =
        serde_json::to_value(canonical_fact_refs::CanonicalFactKey::Organization {
            organization_id: roots.organization_id,
        })
        .expect("serialize canonical organization request subject");
    let mut dynamic_request = runtime_memory_tx::RequestStageWorkerRow {
        fence: producer_fence.clone(),
        stage_team_plan_id: seeded.plan.id,
        parent_work_item_id: producer.work_item.id,
        expected_dispatch_epoch: seeded.plan.dispatch_epoch,
        requested_role: "helper".to_string(),
        requested_kind: "enrichment".to_string(),
        subject_refs: vec![canonical_organization_ref.clone()],
        reason: "resolve ownership".to_string(),
        output_schema: serde_json::json!("stage_worker_output.v1"),
        budget_hint: serde_json::json!({}),
        dedupe_key: "ownership:example.test".to_string(),
        request_sha256: String::new(),
    };
    dynamic_request.request_sha256 =
        runtime_memory_tx::stage_worker_request_payload_hash(&dynamic_request);
    let accepted = runtime_memory_tx::request_stage_worker(db.pool(), &dynamic_request)
        .await
        .expect("accept bounded dynamic sibling");
    assert_eq!(accepted.request.status, "accepted");
    assert!(accepted.work_item.is_some());
    let replayed = runtime_memory_tx::request_stage_worker(db.pool(), &dynamic_request)
        .await
        .expect("replay accepted request after response loss");
    assert!(replayed.replayed);
    assert_eq!(replayed.request.id, accepted.request.id);

    let restarted = runtime_memory_tx::seed_stage_team_runtime(db.pool(), &team_seed)
        .await
        .expect("restart replays static seed beside authorized dynamic WorkItem");
    assert_eq!(restarted.len(), 1);
    assert!(restarted[0].replayed);
    assert_eq!(restarted[0].work_items.len(), team_seed.work_items.len());
    assert!(restarted[0]
        .work_items
        .iter()
        .all(|item| item.created_by == "server_seed"));
    let replay_items = stage_teams::list_work_items_with_executor(db.pool(), restarted[0].plan.id)
        .await
        .expect("load static and dynamic replay items");
    assert_eq!(replay_items.len(), team_seed.work_items.len() + 1);
    assert!(replay_items.iter().any(|item| {
        item.id == accepted.work_item.as_ref().expect("accepted item").id
            && item.created_by == "accepted_worker_request"
    }));
    let mut mismatched_static_seed = team_seed.clone();
    mismatched_static_seed.work_items[0].budget = serde_json::json!({"timeout_ms": 1});
    let mismatch = runtime_memory_tx::seed_stage_team_runtime(db.pool(), &mismatched_static_seed)
        .await
        .expect_err("restart must exact-match every immutable server seed field");
    assert!(matches!(
        mismatch,
        runtime_memory_tx::RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_team_work_item_replay_mismatch"
        }
    ));

    let mut limited_request = runtime_memory_tx::RequestStageWorkerRow {
        dedupe_key: "ownership:second.test".to_string(),
        subject_refs: vec![canonical_organization_ref],
        request_sha256: String::new(),
        ..dynamic_request.clone()
    };
    limited_request.request_sha256 =
        runtime_memory_tx::stage_worker_request_payload_hash(&limited_request);
    let rejected = runtime_memory_tx::request_stage_worker(db.pool(), &limited_request)
        .await
        .expect("persist policy rejection");
    assert_eq!(rejected.request.status, "rejected");
    assert_eq!(
        rejected.request.decision_reason_code.as_deref(),
        Some("stage_team_dynamic_request_limit_reached")
    );

    let stage_worker_evidence_id = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO audit_log (
               action, category, details, project_path, source, audit_role,
               detail, run_id, created_at
           ) VALUES (
               'stage team worker evidence','harness','fresh exact worker evidence',
               '/tmp/runtime-worker','harness','evidence',$1,$2,NOW()
           ) RETURNING id"#,
    )
    .bind(serde_json::json!({"organization_id": roots.organization_id}))
    .bind(roots.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("insert fresh operation-owned Team worker evidence");

    let mut producer_completion = stage_teams::CompleteStageWorkerRow {
        fence: producer_fence,
        team_plan_id: seeded.plan.id,
        work_item_id: producer.work_item.id,
        expected_work_item_row_version: producer.work_item.row_version,
        output_schema: "stage_worker_output.v1".to_string(),
        business_disposition: "found".to_string(),
        canonical_output: serde_json::json!({"facts": []}),
        canonical_fact_refs: serde_json::json!([]),
        evidence_ids: vec![stage_worker_evidence_id],
        checked_empty_cells: serde_json::json!([]),
        blocker_codes: Vec::new(),
        output_hash: String::new(),
        terminal_checkpoint: serde_json::json!({"done": true}),
        evidence_watermark: Some(stage_worker_evidence_id),
    };
    refresh_stage_worker_output_hash(&mut producer_completion);
    let completed = stage_teams::complete_stage_worker(db.pool(), producer_completion.clone())
        .await
        .expect("producer completion keeps Unit running");
    assert_eq!(completed.unit.status, "running");
    assert!(!completed.replayed);
    assert!(
        stage_teams::complete_stage_worker(db.pool(), producer_completion)
            .await
            .expect("producer completion exact replay")
            .replayed
    );

    let closed = runtime_memory_tx::close_stage_request_epoch(
        db.pool(),
        &runtime_memory_tx::CloseStageRequestEpochRow {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: seeded.unit.id,
            stage_team_plan_id: seeded.plan.id,
            expected_dispatch_epoch: seeded.plan.dispatch_epoch,
            expected_plan_row_version: seeded.plan.row_version,
        },
    )
    .await
    .expect("close request epoch");
    assert!(!closed.replayed);
    let closed_replay = runtime_memory_tx::close_stage_request_epoch(
        db.pool(),
        &runtime_memory_tx::CloseStageRequestEpochRow {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: seeded.unit.id,
            stage_team_plan_id: seeded.plan.id,
            expected_dispatch_epoch: seeded.plan.dispatch_epoch,
            expected_plan_row_version: seeded.plan.row_version,
        },
    )
    .await
    .expect("close response-loss replay");
    assert!(closed_replay.replayed);

    let helper = runtime_memory_tx::claim_stage_work_item(db.pool(), &claim())
        .await
        .expect("claim accepted dynamic item after request epoch closes")
        .expect("dynamic item remains claimable");
    assert_eq!(helper.work_item.role, "helper");
    let helper_fence = runtime_memory_tx::RuntimeMemoryTxFence {
        operation_id: roots.operation_id,
        stage_execution_id: roots.stage_execution_id,
        stage_run_unit_id: seeded.unit.id,
        worker_run_id: helper.worker.id,
        lease_token: helper.worker.lease_token.expect("helper lease"),
        attempt_epoch: helper.worker.attempt_epoch,
        expected_checkpoint_version: helper.worker.checkpoint_version,
    };
    let mut helper_completion = stage_teams::CompleteStageWorkerRow {
        fence: helper_fence,
        team_plan_id: seeded.plan.id,
        work_item_id: helper.work_item.id,
        expected_work_item_row_version: helper.work_item.row_version,
        output_schema: "stage_worker_output.v1".to_string(),
        business_disposition: "checked_empty".to_string(),
        canonical_output: serde_json::json!({"facts": []}),
        canonical_fact_refs: serde_json::json!([]),
        evidence_ids: vec![stage_worker_evidence_id],
        checked_empty_cells: serde_json::json!([{"domain": "example.test"}]),
        blocker_codes: Vec::new(),
        output_hash: String::new(),
        terminal_checkpoint: serde_json::json!({"done": true}),
        evidence_watermark: Some(stage_worker_evidence_id),
    };
    refresh_stage_worker_output_hash(&mut helper_completion);
    stage_teams::complete_stage_worker(db.pool(), helper_completion)
        .await
        .expect("complete dynamic sibling");
    let barrier = runtime_memory_tx::load_stage_team_barrier(
        db.pool(),
        &runtime_memory_tx::LoadStageTeamBarrierRow {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: seeded.unit.id,
            stage_team_plan_id: seeded.plan.id,
            dispatch_epoch: seeded.plan.dispatch_epoch,
        },
    )
    .await
    .expect("load closed producer barrier");
    assert!(barrier.ready_to_finalize());
    assert_eq!(barrier.required_work_items, 2);

    let aggregator_claim = runtime_memory_tx::ClaimStageAggregatorRow {
        claim: claim(),
        expected_dispatch_epoch: seeded.plan.dispatch_epoch,
        expected_manifest_hash: barrier.manifest_hash.clone(),
    };
    let first_aggregator = runtime_memory_tx::claim_stage_aggregator(db.pool(), &aggregator_claim)
        .await
        .expect("claim unique final submitter");
    sqlx::query(
        "UPDATE stage_worker_runs
            SET lease_acquired_at=NOW()-INTERVAL '2 minutes',
                lease_expires_at=NOW()-INTERVAL '1 minute',
                heartbeat_at=NOW()-INTERVAL '1 minute'
          WHERE id=$1",
    )
    .bind(first_aggregator.worker.id)
    .execute(db.pool())
    .await
    .expect("expire aggregator without an active tool");
    let replacement = runtime_memory_tx::claim_stage_aggregator(db.pool(), &aggregator_claim)
        .await
        .expect("atomically resume the expired aggregator");
    assert_eq!(replacement.worker.id, first_aggregator.worker.id);
    assert_eq!(
        replacement.message_chain_id,
        first_aggregator.message_chain_id
    );
    assert_eq!(
        replacement.worker.attempt_epoch,
        first_aggregator.worker.attempt_epoch + 1
    );
    assert_eq!(
        replacement.plan.final_submitter_worker_run_id,
        Some(replacement.worker.id)
    );
    assert_eq!(
        stage_worker_runs::get(db.pool(), first_aggregator.worker.id)
            .await
            .expect("load resumed aggregator")
            .expect("aggregator remains auditable")
            .status,
        "running"
    );
    let after_recovery = runtime_memory_tx::load_stage_team_barrier(
        db.pool(),
        &runtime_memory_tx::LoadStageTeamBarrierRow {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: seeded.unit.id,
            stage_team_plan_id: seeded.plan.id,
            dispatch_epoch: seeded.plan.dispatch_epoch,
        },
    )
    .await
    .expect("barrier survives aggregator continuation");
    assert_eq!(after_recovery.manifest_hash, barrier.manifest_hash);
    assert!(after_recovery.ready_to_finalize());

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn stage_team_dynamic_request_replays_after_parent_work_item_retry() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    let hash = |character: char| format!("sha256:{}", character.to_string().repeat(64));
    let team_seed = runtime_memory_tx::SeedStageTeamRuntimeRow {
        base: runtime_memory_tx::SeedStageRuntimeRow {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_kind: "target_intel".to_string(),
            unit_generation: 0,
            specialist: "producer".to_string(),
            worker_generation: 0,
            work_item_kind: "stage_unit".to_string(),
            work_item_key: "team-request-replay".to_string(),
            agent_path_prefix: "main>stage_run:target_intel".to_string(),
            organization_ids: Some(vec![roots.organization_id]),
        },
        plan: runtime_memory_tx::StageTeamPlanSeedRow {
            schema_version: 1,
            plan_version: 1,
            plan_hash: hash('a'),
            leader_role: "aggregator".to_string(),
            allowed_roles: vec![
                "producer".to_string(),
                "helper".to_string(),
                "aggregator".to_string(),
            ],
            aggregator_kind: "worker".to_string(),
            aggregator_role: Some("aggregator".to_string()),
            max_workers_total: 4,
            max_workers_active: 2,
            dynamic_requests_enabled: true,
            dynamic_request_policy: serde_json::json!({
                "allowed_request_kinds": ["enrichment"],
                "canonical_subject_refs_only": true,
                "max_requests": 1,
                "max_subject_refs": 1,
            }),
            final_submitter_kind: "worker".to_string(),
            created_from_stage_spec_hash: hash('b'),
        },
        work_items: vec![
            runtime_memory_tx::StageWorkItemSeedRow {
                stable_key: "producer:root".to_string(),
                work_item_kind: "stage_axis".to_string(),
                role: "producer".to_string(),
                input_manifest: serde_json::json!({"axis": "root"}),
                input_manifest_hash: hash('c'),
                conflict_key: None,
                priority: 0,
                required_for_barrier: true,
                is_aggregator: false,
                attempt_policy: serde_json::json!({"max_attempts": 2}),
                budget: serde_json::json!({}),
                output_schema: "stage_worker_output.v1".to_string(),
                created_by: "server_seed".to_string(),
            },
            runtime_memory_tx::StageWorkItemSeedRow {
                stable_key: "aggregator:final".to_string(),
                work_item_kind: "stage_aggregate".to_string(),
                role: "aggregator".to_string(),
                input_manifest: serde_json::json!({"aggregate": true}),
                input_manifest_hash: hash('d'),
                conflict_key: Some("stage_unit_finalizer".to_string()),
                priority: i32::MAX,
                required_for_barrier: false,
                is_aggregator: true,
                attempt_policy: serde_json::json!({"max_attempts": 2}),
                budget: serde_json::json!({}),
                output_schema: "stage_unit_aggregate.v1".to_string(),
                created_by: "server_seed".to_string(),
            },
        ],
    };
    let seeded = runtime_memory_tx::seed_stage_team_runtime(db.pool(), &team_seed)
        .await
        .expect("seed Team request replay fixture")
        .remove(0);
    let claim = || runtime_memory_tx::ClaimStageWorkItemRow {
        operation_id: roots.operation_id,
        stage_execution_id: roots.stage_execution_id,
        stage_run_unit_id: seeded.unit.id,
        stage_team_plan_id: seeded.plan.id,
        exact_work_item_id: None,
        lease_owner: "team-request-replay-fixture".to_string(),
        lease_seconds: 60,
        session_id: roots.session_id,
        subtask_id: None,
        agent: AgentType::Primary,
        model: None,
        provider: None,
        parent_chain_id: None,
        initial_chain: serde_json::json!([]),
        initial_checkpoint: serde_json::json!({"turn": 0}),
    };
    let first_parent = runtime_memory_tx::claim_stage_work_item(db.pool(), &claim())
        .await
        .expect("claim first parent Worker")
        .expect("producer WorkItem is queued");
    assert_eq!(first_parent.work_item.stable_key, "producer:root");
    let first_fence = runtime_memory_tx::RuntimeMemoryTxFence {
        operation_id: roots.operation_id,
        stage_execution_id: roots.stage_execution_id,
        stage_run_unit_id: seeded.unit.id,
        worker_run_id: first_parent.worker.id,
        lease_token: first_parent.worker.lease_token.expect("first parent lease"),
        attempt_epoch: first_parent.worker.attempt_epoch,
        expected_checkpoint_version: first_parent.worker.checkpoint_version,
    };
    let subject_ref = serde_json::to_value(canonical_fact_refs::CanonicalFactKey::Organization {
        organization_id: roots.organization_id,
    })
    .expect("serialize canonical organization request subject");
    let mut initial_request = runtime_memory_tx::RequestStageWorkerRow {
        fence: first_fence.clone(),
        stage_team_plan_id: seeded.plan.id,
        parent_work_item_id: first_parent.work_item.id,
        expected_dispatch_epoch: seeded.plan.dispatch_epoch,
        requested_role: "helper".to_string(),
        requested_kind: "enrichment".to_string(),
        subject_refs: vec![subject_ref],
        reason: "resolve ownership".to_string(),
        output_schema: serde_json::json!("stage_worker_output.v1"),
        budget_hint: serde_json::json!({}),
        dedupe_key: "ownership:example.test".to_string(),
        request_sha256: String::new(),
    };
    initial_request.request_sha256 =
        runtime_memory_tx::stage_worker_request_payload_hash(&initial_request);
    let accepted = runtime_memory_tx::request_stage_worker(db.pool(), &initial_request)
        .await
        .expect("accept dynamic sibling before parent retry");
    let accepted_work_item_id = accepted
        .work_item
        .as_ref()
        .expect("accepted request owns a WorkItem")
        .id;

    let retried = stage_teams::retry_stage_worker(
        db.pool(),
        stage_teams::RetryStageWorkerRow {
            fence: first_fence,
            team_plan_id: seeded.plan.id,
            work_item_id: first_parent.work_item.id,
            expected_work_item_row_version: first_parent.work_item.row_version,
            failure_code: "provider_unavailable".to_string(),
            terminal_checkpoint: serde_json::json!({
                "stage_team_execution_failure": {"code": "provider_unavailable"}
            }),
        },
    )
    .await
    .expect("requeue stable parent WorkItem after execution failure");
    assert!(retried.retry_scheduled);
    assert_eq!(retried.work_item.id, first_parent.work_item.id);

    let recovered_parent = runtime_memory_tx::claim_stage_work_item(db.pool(), &claim())
        .await
        .expect("claim requeued parent WorkItem")
        .expect("parent WorkItem remains retryable");
    assert_eq!(recovered_parent.work_item.id, first_parent.work_item.id);
    assert_ne!(recovered_parent.worker.id, first_parent.worker.id);
    let recovered_lease = recovered_parent
        .worker
        .lease_token
        .expect("recovered parent lease");
    assert_ne!(Some(recovered_lease), first_parent.worker.lease_token);
    let mut replay_request = runtime_memory_tx::RequestStageWorkerRow {
        fence: runtime_memory_tx::RuntimeMemoryTxFence {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: seeded.unit.id,
            worker_run_id: recovered_parent.worker.id,
            lease_token: recovered_lease,
            attempt_epoch: recovered_parent.worker.attempt_epoch,
            expected_checkpoint_version: recovered_parent.worker.checkpoint_version,
        },
        request_sha256: String::new(),
        ..initial_request.clone()
    };
    replay_request.request_sha256 =
        runtime_memory_tx::stage_worker_request_payload_hash(&replay_request);
    let replayed = runtime_memory_tx::request_stage_worker(db.pool(), &replay_request)
        .await
        .expect("same logical request replays after parent WorkItem retry");
    assert!(replayed.replayed);
    assert_eq!(replayed.request.id, accepted.request.id);
    assert_eq!(
        replayed.work_item.as_ref().map(|item| item.id),
        Some(accepted_work_item_id)
    );
    let stale_fence = runtime_memory_tx::request_stage_worker(db.pool(), &initial_request)
        .await
        .expect_err("replay must still reject the superseded parent Worker fence");
    assert!(matches!(
        stale_fence,
        runtime_memory_tx::RuntimeMemoryStoreError::LeaseLost { .. }
    ));
    let request_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM stage_worker_requests
          WHERE team_plan_id=$1 AND dispatch_epoch=$2 AND dedupe_key=$3",
    )
    .bind(seeded.plan.id)
    .bind(seeded.plan.dispatch_epoch)
    .bind(&replay_request.dedupe_key)
    .fetch_one(db.pool())
    .await
    .expect("count exact dynamic request identity");
    assert_eq!(request_count, 1);
    let accepted_work_item_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM stage_work_items
          WHERE team_plan_id=$1 AND id=$2 AND created_by='accepted_worker_request'",
    )
    .bind(seeded.plan.id)
    .bind(accepted_work_item_id)
    .fetch_one(db.pool())
    .await
    .expect("count accepted dynamic WorkItem identity");
    assert_eq!(accepted_work_item_count, 1);

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn exact_resume_source_claim_allows_only_one_open_turn_contender() {
    let (mut db, _data_dir) = fixture().await;
    let session_id = sessions::create(
        db.pool(),
        NewSession {
            title: Some("atomic resume source claim".to_string()),
            workspace_path: Some("/tmp/runtime-resume-claim".to_string()),
            workspace_label: None,
            model: None,
            provider: None,
            project_path: Some("/tmp/runtime-resume-claim".to_string()),
        },
    )
    .await
    .expect("create resume-claim session")
    .id;
    let project_scope = project_scopes::register_first_open(
        db.pool(),
        "/tmp/runtime-resume-claim",
        "runtime-resume-claim-sha",
    )
    .await
    .expect("register resume-claim project scope");
    let operation_id = Uuid::new_v4();
    runtime_memory_tx::create_runtime_operation(
        db.pool(),
        &runtime_memory_tx::CreateRuntimeOperationRow {
            operation_id,
            initial_stage_execution_id: Uuid::new_v4(),
            session_id,
            title: Some("resume source claim".to_string()),
            input: "resume exact graph checkpoint".to_string(),
            profile: "assessment".to_string(),
            entry_stage: "scoping".to_string(),
            application_model_contract: golish_core::ApplicationModelContract::LegacyNoModel,
            project_scope_id: project_scope.project_scope_id,
            cli_scope: None,
        },
    )
    .await
    .expect("create resume-claim operation");
    sqlx::query(
        r#"UPDATE operation_state
              SET state_blob=jsonb_build_object(
                    'graph_flow',jsonb_build_object(
                        'state',jsonb_build_object(
                            'seeded',jsonb_build_object(),
                            'visited',jsonb_build_array(),
                            'applied',jsonb_build_object()
                        ),
                        'next_node','scoping'
                    )
                  )
            WHERE operation_id=$1"#,
    )
    .bind(operation_id)
    .execute(db.pool())
    .await
    .expect("install complete legacy graph checkpoint");
    sqlx::query("UPDATE tasks SET status='waiting',result=NULL WHERE id=$1")
        .bind(operation_id)
        .execute(db.pool())
        .await
        .expect("pause exact task before concurrent claim");

    let source = tasks::exact_resumable_runtime_source(db.pool(), operation_id, session_id)
        .await
        .expect("select complete runtime source")
        .expect("one complete runtime source");
    assert_eq!(source, runtime_memory_tx::RuntimeMemoryRecordSource::Legacy);
    let open_turn = golish_db::repo::operation_turns::get_open(db.pool(), operation_id)
        .await
        .expect("load exact open operation Turn")
        .expect("operation has one open Turn");
    let first = tasks::claim_exact_resumable_runtime_source(
        db.pool(),
        operation_id,
        session_id,
        source,
        open_turn.id,
        Uuid::new_v4(),
        "first concurrent continuation",
    );
    let second = tasks::claim_exact_resumable_runtime_source(
        db.pool(),
        operation_id,
        session_id,
        source,
        open_turn.id,
        Uuid::new_v4(),
        "second concurrent continuation",
    );
    let (first, second) = tokio::join!(first, second);
    let claims = [
        first.expect("first exact resume claim"),
        second.expect("second exact resume claim"),
    ];
    assert_eq!(claims.into_iter().filter(|claimed| *claimed).count(), 1);
    let task = tasks::get(db.pool(), operation_id)
        .await
        .expect("load claimed task")
        .expect("claimed task remains");
    assert_eq!(task.status, golish_db::models::TaskStatus::Running);
    let turns = golish_db::repo::operation_turns::list_for_operation(db.pool(), operation_id)
        .await
        .expect("load operation Turn timeline");
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].status, "interrupted");
    assert_eq!(turns[1].status, "running");
    assert_eq!(turns[1].ordinal, 2);
    let identity_rewrite =
        sqlx::query("UPDATE operation_turns SET trigger_input='rewritten witness' WHERE id=$1")
            .bind(turns[0].id)
            .execute(db.pool())
            .await;
    assert!(
        identity_rewrite.is_err(),
        "persisted Turn identity and input are immutable"
    );
    let terminal_reopen =
        sqlx::query("UPDATE operation_turns SET status='running',terminal_at=NULL WHERE id=$1")
            .bind(turns[0].id)
            .execute(db.pool())
            .await;
    assert!(
        terminal_reopen.is_err(),
        "a terminal predecessor Turn cannot be reopened"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn exact_resume_claims_running_v2_operation_without_reaper_delay() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    runtime_memory_tx::seed_stage_team_runtime(db.pool(), &stage_team_lifecycle_seed(&roots, 2))
        .await
        .expect("seed complete V2 Team runtime roots");
    tasks::update_status(
        db.pool(),
        roots.operation_id,
        golish_db::models::TaskStatus::Running,
    )
    .await
    .expect("leave operation in the pre-reaper running state");

    let source =
        tasks::exact_resumable_runtime_source(db.pool(), roots.operation_id, roots.session_id)
            .await
            .expect("select running V2 source")
            .expect("running V2 operation is immediately resumable");
    assert_eq!(source, runtime_memory_tx::RuntimeMemoryRecordSource::V2);
    let open_turn = golish_db::repo::operation_turns::get_open(db.pool(), roots.operation_id)
        .await
        .expect("load original V2 Turn")
        .expect("V2 operation has an open Turn");
    assert!(tasks::claim_exact_resumable_runtime_source(
        db.pool(),
        roots.operation_id,
        roots.session_id,
        source,
        open_turn.id,
        Uuid::new_v4(),
        "继续",
    )
    .await
    .expect("claim running V2 operation Turn"));
    let turns = golish_db::repo::operation_turns::list_for_operation(db.pool(), roots.operation_id)
        .await
        .expect("load V2 Turn timeline");
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].status, "interrupted");
    assert_eq!(turns[1].status, "running");
    assert_eq!(turns[1].trigger_input, "继续");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn raw_cutover_replay_preserves_an_existing_legacy_operation_contract() {
    let (mut db, _data_dir) = fixture().await;
    reset_deployment_rollouts_for_fixture(&db).await;
    let _legacy_session_id = sessions::create(
        db.pool(),
        NewSession {
            title: Some("pre-cutover runtime operation".to_string()),
            workspace_path: Some("/tmp/runtime-worker-cutover-legacy".to_string()),
            workspace_label: None,
            model: None,
            provider: None,
            project_path: Some("/tmp/runtime-worker-cutover-legacy".to_string()),
        },
    )
    .await
    .expect("create pre-cutover session")
    .id;
    let legacy_project = project_scopes::register_first_open(
        db.pool(),
        "/tmp/runtime-worker-cutover-legacy",
        "runtime-worker-cutover-legacy-sha",
    )
    .await
    .expect("register pre-cutover project");
    let legacy_operation_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,
               project_scope_id,attack_execution_contract
           ) VALUES($1,'assessment','scoping','legacy_v1',$2,'legacy')"#,
    )
    .bind(legacy_operation_id)
    .bind(legacy_project.project_scope_id)
    .execute(db.pool())
    .await
    .expect("install an operation frozen before the sampling cutover");

    // GolishDb already recorded 00002 in `_sqlx_migrations`. This raw replay intentionally tests
    // the SQL against a simulated pre-cutover singleton and must not be mistaken for a pending
    // migrator run.
    sqlx::raw_sql(RUNTIME_MEMORY_V2_CUTOVER)
        .execute(db.pool())
        .await
        .expect("replay cutover over an existing legacy operation");
    let replayed_rollout = runtime_memory_rollout::get(db.pool())
        .await
        .expect("read replayed cutover rollout");
    assert_eq!(replayed_rollout.contract, "dual_write_legacy_read");
    assert_eq!(replayed_rollout.contract_rank, 1);
    assert_eq!(replayed_rollout.row_version, 1);
    let legacy_after_cutover = operation_state::get(db.pool(), legacy_operation_id)
        .await
        .expect("read existing operation after cutover")
        .expect("existing legacy operation remains");
    assert_eq!(legacy_after_cutover.runtime_memory_contract, "legacy_v1");

    db.stop().await;
}

struct RuntimeRoots {
    session_id: Uuid,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    snapshot_id: Uuid,
    organization_id: Uuid,
}

struct ClaimedCompoundRuntime {
    roots: RuntimeRoots,
    unit_id: Uuid,
    worker_id: Uuid,
    worker: stage_worker_runs::StageWorkerRunRow,
}

async fn create_sealed_runtime_roots(db: &GolishDb) -> RuntimeRoots {
    create_sealed_runtime_roots_with_contract(
        db,
        runtime_memory_rollout::RuntimeMemoryContract::DualWriteLegacyRead,
    )
    .await
}

async fn create_sealed_runtime_roots_with_contract(
    db: &GolishDb,
    target_contract: runtime_memory_rollout::RuntimeMemoryContract,
) -> RuntimeRoots {
    create_sealed_runtime_roots_with_contract_and_children(db, target_contract, 0)
        .await
        .0
}

async fn create_sealed_runtime_roots_with_contract_and_children(
    db: &GolishDb,
    target_contract: runtime_memory_rollout::RuntimeMemoryContract,
    child_count: usize,
) -> (RuntimeRoots, Vec<Uuid>) {
    create_sealed_runtime_roots_with_contract_stage_and_children(
        db,
        target_contract,
        "target_intel",
        child_count,
    )
    .await
}

async fn create_sealed_runtime_roots_with_contract_stage_and_children(
    db: &GolishDb,
    target_contract: runtime_memory_rollout::RuntimeMemoryContract,
    stage_kind: &str,
    child_count: usize,
) -> (RuntimeRoots, Vec<Uuid>) {
    reset_deployment_rollouts_for_fixture(db).await;
    let session_id = sessions::create(
        db.pool(),
        NewSession {
            title: Some("runtime worker transaction".to_string()),
            workspace_path: Some("/tmp/runtime-worker".to_string()),
            workspace_label: None,
            model: None,
            provider: None,
            project_path: Some("/tmp/runtime-worker".to_string()),
        },
    )
    .await
    .expect("create runtime session")
    .id;
    let project_scope = project_scopes::register_first_open(
        db.pool(),
        "/tmp/runtime-worker",
        "runtime-worker-path-sha",
    )
    .await
    .expect("register project scope");
    set_runtime_rollout_for_fixture(db, target_contract).await;

    let operation_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    runtime_memory_tx::create_runtime_operation(
        db.pool(),
        &runtime_memory_tx::CreateRuntimeOperationRow {
            operation_id,
            initial_stage_execution_id: stage_execution_id,
            session_id,
            title: Some("worker roots".to_string()),
            input: format!("run {stage_kind}"),
            profile: "assessment".to_string(),
            entry_stage: stage_kind.to_string(),
            application_model_contract: golish_core::ApplicationModelContract::LegacyNoModel,
            project_scope_id: project_scope.project_scope_id,
            cli_scope: None,
        },
    )
    .await
    .expect("create operation and initial stage execution");

    let decision_id = Uuid::new_v4();
    let snapshot_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let child_organization_ids = (0..child_count).map(|_| Uuid::new_v4()).collect::<Vec<_>>();
    let mut tx = db.pool().begin().await.expect("begin scope freeze fixture");
    sqlx::query(
        r#"INSERT INTO organizations (id, project_path, name)
           VALUES ($1, '/tmp/runtime-worker', 'Root Org')"#,
    )
    .bind(organization_id)
    .execute(&mut *tx)
    .await
    .expect("insert current organization owner");
    for (index, child_organization_id) in child_organization_ids.iter().enumerate() {
        sqlx::query(
            r#"INSERT INTO organizations (id, project_path, name, parent_id)
               VALUES ($1, '/tmp/runtime-worker', $2, $3)"#,
        )
        .bind(child_organization_id)
        .bind(format!("Child Org {}", index + 1))
        .bind(organization_id)
        .execute(&mut *tx)
        .await
        .expect("insert current child organization owner");
    }
    let decision_rows = std::iter::once(organization_id)
        .chain(child_organization_ids.iter().copied())
        .map(|organization_id| serde_json::json!({"organization_id": organization_id}))
        .collect::<Vec<_>>();
    sqlx::query(
        r#"INSERT INTO operation_scope_decisions (
               id, operation_id, project_scope_id, stage_execution_id,
               root_organization_id, mode, decision_rows, decision_hash
           ) VALUES ($1,$2,$3,$4,$5,'cli_flags',$6,$7)"#,
    )
    .bind(decision_id)
    .bind(operation_id)
    .bind(project_scope.project_scope_id)
    .bind(stage_execution_id)
    .bind(organization_id)
    .bind(serde_json::to_value(decision_rows).expect("serialize decision rows"))
    .bind("decision-sha")
    .execute(&mut *tx)
    .await
    .expect("insert trusted scope decision");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_snapshots (
               id, operation_id, project_scope_id, scope_decision_id,
               project_path_at_freeze, root_organization_id, mode, scope_hash
           ) VALUES ($1,$2,$3,$4,$5,$6,'cli_flags',$7)"#,
    )
    .bind(snapshot_id)
    .bind(operation_id)
    .bind(project_scope.project_scope_id)
    .bind(decision_id)
    .bind("/tmp/runtime-worker")
    .bind(organization_id)
    .bind("scope-sha")
    .execute(&mut *tx)
    .await
    .expect("insert scope snapshot header");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units (
               snapshot_id, organization_id, organization_name_at_freeze,
               role, depth, ordinal, decision_row_id, approval_source
           ) VALUES ($1,$2,'Root Org','root',0,0,'root-row',$3)"#,
    )
    .bind(snapshot_id)
    .bind(organization_id)
    .bind(serde_json::json!({"source": "cli_flags"}))
    .execute(&mut *tx)
    .await
    .expect("insert frozen root organization");
    for (index, child_organization_id) in child_organization_ids.iter().enumerate() {
        sqlx::query(
            r#"INSERT INTO operation_org_scope_units (
                   snapshot_id, organization_id, parent_organization_id,
                   organization_name_at_freeze, role, depth, ordinal,
                   ownership_percent, decision_row_id, approval_source
               ) VALUES ($1,$2,$3,$4,'subsidiary',1,$5,100,$6,$7)"#,
        )
        .bind(snapshot_id)
        .bind(child_organization_id)
        .bind(organization_id)
        .bind(format!("Child Org {}", index + 1))
        .bind(i32::try_from(index + 1).expect("child ordinal fits i32"))
        .bind(format!("child-row-{}", index + 1))
        .bind(serde_json::json!({"source": "cli_flags"}))
        .execute(&mut *tx)
        .await
        .expect("insert frozen child organization");
    }
    sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at = NOW() WHERE id = $1")
        .bind(snapshot_id)
        .execute(&mut *tx)
        .await
        .expect("seal scope snapshot");
    tx.commit().await.expect("commit sealed scope fixture");

    (
        RuntimeRoots {
            session_id,
            operation_id,
            stage_execution_id,
            snapshot_id,
            organization_id,
        },
        child_organization_ids,
    )
}

async fn create_claimed_compound_runtime(db: &GolishDb) -> ClaimedCompoundRuntime {
    create_claimed_compound_runtime_with_contract(
        db,
        runtime_memory_rollout::RuntimeMemoryContract::DualWriteLegacyRead,
    )
    .await
}

async fn create_claimed_compound_runtime_with_contract(
    db: &GolishDb,
    contract: runtime_memory_rollout::RuntimeMemoryContract,
) -> ClaimedCompoundRuntime {
    let roots = create_sealed_runtime_roots_with_contract(db, contract).await;
    let seeded = runtime_memory_tx::seed_stage_runtime(
        db.pool(),
        &runtime_memory_tx::SeedStageRuntimeRow {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_kind: "target_intel".to_string(),
            unit_generation: 0,
            specialist: "target_intel".to_string(),
            worker_generation: 0,
            work_item_kind: "stage_unit".to_string(),
            work_item_key: "primary".to_string(),
            agent_path_prefix: "main>stage_run:target_intel".to_string(),
            organization_ids: None,
        },
    )
    .await
    .expect("seed compound runtime fixture");
    let seeded = &seeded[0];
    assert_eq!(seeded.scope_hash, "scope-sha");
    let claimed = runtime_memory_tx::claim_worker_and_bind_chain(
        db.pool(),
        &runtime_memory_tx::ClaimWorkerAndBindChainRow {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: seeded.unit.id,
            worker_run_id: seeded.worker.id,
            expected_unit_status: stage_run_units::StageRunUnitStatus::Queued,
            expected_unit_row_version: seeded.unit.row_version,
            expected_worker_status: stage_worker_runs::StageWorkerRunStatus::Queued,
            expected_attempt_epoch: seeded.worker.attempt_epoch,
            session_id: roots.session_id,
            subtask_id: None,
            agent: AgentType::Primary,
            model: None,
            provider: None,
            parent_chain_id: None,
            lease_owner: "rollback-fixture".to_string(),
            lease_seconds: 60,
            initial_chain: serde_json::json!([]),
            initial_checkpoint: serde_json::json!({"turn": 0}),
        },
    )
    .await
    .expect("claim compound runtime fixture");
    ClaimedCompoundRuntime {
        roots,
        unit_id: seeded.unit.id,
        worker_id: seeded.worker.id,
        worker: claimed.worker,
    }
}

fn fence_for_claimed(runtime: &ClaimedCompoundRuntime) -> runtime_memory_tx::RuntimeMemoryTxFence {
    runtime_memory_tx::RuntimeMemoryTxFence {
        operation_id: runtime.roots.operation_id,
        stage_execution_id: runtime.roots.stage_execution_id,
        stage_run_unit_id: runtime.unit_id,
        worker_run_id: runtime.worker_id,
        lease_token: runtime.worker.lease_token.expect("claimed lease token"),
        attempt_epoch: runtime.worker.attempt_epoch,
        expected_checkpoint_version: runtime.worker.checkpoint_version,
    }
}

struct FinalSealFixture {
    runtime: ClaimedCompoundRuntime,
    fence: runtime_memory_tx::RuntimeMemoryTxFence,
    unit: stage_run_units::StageRunUnitRow,
    submission_id: Uuid,
    target_id: Uuid,
    finding_id: Uuid,
    evidence_id: i64,
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => {
            serde_json::to_string(value).expect("serialize JSON string")
        }
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
                        serde_json::to_string(key).expect("serialize object key"),
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

fn refresh_stage_worker_output_hash(input: &mut stage_teams::CompleteStageWorkerRow) {
    let material = serde_json::json!({
        "blocker_code": input.blocker_codes.first(),
        "canonical_output": input.canonical_output,
        "checked_empty_units": input.checked_empty_cells,
        "disposition": input.business_disposition,
        "evidence_ids": input.evidence_ids,
        "fact_refs": input.canonical_fact_refs,
        "output_schema": input.output_schema,
        "work_item_id": input.work_item_id,
        "worker_run_id": input.fence.worker_run_id,
    });
    input.output_hash = format!("sha256:{}", sha256_json(&material));
}

fn stage_team_lifecycle_seed(
    roots: &RuntimeRoots,
    max_attempts: i64,
) -> runtime_memory_tx::SeedStageTeamRuntimeRow {
    let hash = |character: char| format!("sha256:{}", character.to_string().repeat(64));
    runtime_memory_tx::SeedStageTeamRuntimeRow {
        base: runtime_memory_tx::SeedStageRuntimeRow {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_kind: "target_intel".to_string(),
            unit_generation: 0,
            // Team Worker identity is owned by StageWorkItem.role.  Deliberately
            // keep the Unit specialist different so startup recovery cannot
            // accidentally rely on the legacy specialist equality join.
            specialist: "stage_team".to_string(),
            worker_generation: 0,
            work_item_kind: "stage_unit".to_string(),
            work_item_key: "team-lifecycle".to_string(),
            agent_path_prefix: "main>stage_run:target_intel".to_string(),
            organization_ids: Some(vec![roots.organization_id]),
        },
        plan: runtime_memory_tx::StageTeamPlanSeedRow {
            schema_version: 1,
            plan_version: 1,
            plan_hash: hash('a'),
            leader_role: "aggregator".to_string(),
            allowed_roles: vec![
                "producer".to_string(),
                "helper".to_string(),
                "aggregator".to_string(),
            ],
            aggregator_kind: "worker".to_string(),
            aggregator_role: Some("aggregator".to_string()),
            max_workers_total: 8,
            max_workers_active: 3,
            dynamic_requests_enabled: false,
            dynamic_request_policy: serde_json::json!({}),
            final_submitter_kind: "worker".to_string(),
            created_from_stage_spec_hash: hash('b'),
        },
        work_items: vec![
            runtime_memory_tx::StageWorkItemSeedRow {
                stable_key: "producer:primary".to_string(),
                work_item_kind: "stage_axis".to_string(),
                role: "producer".to_string(),
                input_manifest: serde_json::json!({"axis": "primary"}),
                input_manifest_hash: hash('c'),
                conflict_key: None,
                priority: 0,
                required_for_barrier: true,
                is_aggregator: false,
                attempt_policy: serde_json::json!({"max_attempts": max_attempts}),
                budget: serde_json::json!({}),
                output_schema: "stage_worker_output.v1".to_string(),
                created_by: "server_seed".to_string(),
            },
            runtime_memory_tx::StageWorkItemSeedRow {
                stable_key: "helper:secondary".to_string(),
                work_item_kind: "stage_axis".to_string(),
                role: "helper".to_string(),
                input_manifest: serde_json::json!({"axis": "secondary"}),
                input_manifest_hash: hash('d'),
                conflict_key: None,
                priority: 1,
                required_for_barrier: true,
                is_aggregator: false,
                attempt_policy: serde_json::json!({"max_attempts": max_attempts}),
                budget: serde_json::json!({}),
                output_schema: "stage_worker_output.v1".to_string(),
                created_by: "server_seed".to_string(),
            },
            runtime_memory_tx::StageWorkItemSeedRow {
                stable_key: "aggregator:final".to_string(),
                work_item_kind: "stage_aggregate".to_string(),
                role: "aggregator".to_string(),
                input_manifest: serde_json::json!({"aggregate": true}),
                input_manifest_hash: hash('e'),
                conflict_key: Some("stage_unit_finalizer".to_string()),
                priority: i32::MAX,
                required_for_barrier: false,
                is_aggregator: true,
                attempt_policy: serde_json::json!({"max_attempts": max_attempts}),
                budget: serde_json::json!({}),
                output_schema: "stage_unit_aggregate.v1".to_string(),
                created_by: "server_seed".to_string(),
            },
        ],
    }
}

fn stage_team_controller_seed(roots: &RuntimeRoots) -> runtime_memory_tx::SeedStageTeamRuntimeRow {
    stage_team_controller_seed_for_stage(roots, "target_intel")
}

fn stage_team_controller_seed_for_stage(
    roots: &RuntimeRoots,
    stage_kind: &str,
) -> runtime_memory_tx::SeedStageTeamRuntimeRow {
    let hash = |character: char| format!("sha256:{}", character.to_string().repeat(64));
    runtime_memory_tx::SeedStageTeamRuntimeRow {
        base: runtime_memory_tx::SeedStageRuntimeRow {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_kind: stage_kind.to_string(),
            unit_generation: 0,
            specialist: "company_stage_controller".to_string(),
            worker_generation: 0,
            work_item_kind: "stage_unit".to_string(),
            work_item_key: "company-controller".to_string(),
            agent_path_prefix: format!("main>stage_run:{stage_kind}"),
            organization_ids: Some(vec![roots.organization_id]),
        },
        plan: runtime_memory_tx::StageTeamPlanSeedRow {
            schema_version: 1,
            plan_version: 1,
            plan_hash: hash('f'),
            leader_role: "company_stage_controller".to_string(),
            allowed_roles: vec![
                "company_stage_controller".to_string(),
                "intel_researcher".to_string(),
            ],
            aggregator_kind: "worker".to_string(),
            aggregator_role: Some("company_stage_controller".to_string()),
            max_workers_total: 5,
            // One Controller + at most two live children.
            max_workers_active: 3,
            dynamic_requests_enabled: true,
            dynamic_request_policy: serde_json::json!({
                "allowed_request_kinds": ["stage_axis"],
                "canonical_subject_refs_only": true,
                "child_budget": {},
                "child_output_schema": "stage_worker_output.v1",
                "coordination_mode": "company_controller",
                "max_controller_gate_repairs": 1,
                "max_repair_generations": 2,
                "max_requests": 3,
                "max_subject_refs": 1,
                "organization_scope_implicit": true,
            }),
            final_submitter_kind: "worker".to_string(),
            created_from_stage_spec_hash: hash('e'),
        },
        work_items: vec![runtime_memory_tx::StageWorkItemSeedRow {
            stable_key: "leader:primary".to_string(),
            work_item_kind: "stage_controller".to_string(),
            role: "company_stage_controller".to_string(),
            input_manifest: serde_json::json!({"controller": true}),
            input_manifest_hash: hash('d'),
            conflict_key: Some("stage_unit_finalizer".to_string()),
            priority: 0,
            required_for_barrier: false,
            is_aggregator: true,
            attempt_policy: serde_json::json!({"max_attempts": 3}),
            budget: serde_json::json!({"controller": true}),
            output_schema: "stage_unit_aggregate.v1".to_string(),
            created_by: "server_seed".to_string(),
        }],
    }
}

#[tokio::test]
#[serial]
async fn exact_resume_accepts_company_controller_pentester_chain_ownership() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    let seeded =
        runtime_memory_tx::seed_stage_team_runtime(db.pool(), &stage_team_controller_seed(&roots))
            .await
            .expect("seed Controller exact-resume Team")
            .remove(0);
    let controller = runtime_memory_tx::claim_stage_team_leader(
        db.pool(),
        &runtime_memory_tx::ClaimStageTeamLeaderRow {
            claim: stage_team_claim_input(&roots, &seeded, "controller-resume-ownership"),
        },
    )
    .await
    .expect("claim Controller exact-resume worker")
    .expect("Controller is runnable");

    let rows = message_chains::list_exact_resume_bound_chains(
        db.pool(),
        roots.operation_id,
        roots.session_id,
    )
    .await
    .expect("load Controller exact-resume chain ownership");
    let row = rows
        .iter()
        .find(|row| row.worker_run_id == controller.worker.id)
        .expect("Controller worker remains in current-stage resume set");
    assert_eq!(row.message_chain_id, Some(controller.message_chain_id));
    assert_eq!(
        row.exact_chain_id,
        Some(controller.message_chain_id),
        "company_stage_controller is persisted as the coarse pentester agent type"
    );
    assert!(row.chain.is_some());

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn company_controller_scope_bounded_admission_has_no_lifetime_request_quota() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    let seeded =
        runtime_memory_tx::seed_stage_team_runtime(db.pool(), &stage_team_controller_seed(&roots))
            .await
            .expect("seed scope-bounded Controller Team")
            .remove(0);
    assert_eq!(seeded.plan.max_workers_total, 5);
    assert_eq!(
        seeded
            .plan
            .dynamic_request_policy
            .get("max_requests")
            .and_then(serde_json::Value::as_i64),
        Some(3),
        "fixture keeps the historical quotas to prove existing plans stop enforcing them"
    );
    let controller = runtime_memory_tx::claim_stage_team_leader(
        db.pool(),
        &runtime_memory_tx::ClaimStageTeamLeaderRow {
            claim: stage_team_claim_input(&roots, &seeded, "scope-bounded-controller"),
        },
    )
    .await
    .expect("claim scope-bounded Company Controller")
    .expect("Controller is runnable");
    let controller_fence = stage_team_fence(&roots, &seeded, &controller);

    for index in 0..6 {
        let mut request = runtime_memory_tx::RequestStageWorkerRow {
            fence: controller_fence.clone(),
            stage_team_plan_id: seeded.plan.id,
            parent_work_item_id: controller.work_item.id,
            expected_dispatch_epoch: seeded.plan.dispatch_epoch,
            requested_role: "intel_researcher".to_string(),
            requested_kind: "stage_axis".to_string(),
            subject_refs: Vec::new(),
            reason: serde_json::json!({
                "schema": "stage_team_controller_request.v1",
                "objective": format!("Cover authoritative worklist shard {index}"),
                "parent_tool_request_id": format!("scope-bounded-dispatch-{index}"),
            })
            .to_string(),
            output_schema: serde_json::json!("stage_worker_output.v1"),
            budget_hint: serde_json::json!({}),
            dedupe_key: format!("authoritative-worklist-shard-{index}"),
            request_sha256: String::new(),
        };
        request.request_sha256 = runtime_memory_tx::stage_worker_request_payload_hash(&request);
        let accepted = runtime_memory_tx::request_stage_worker(db.pool(), &request)
            .await
            .expect("persist scope-bounded Controller request");
        assert_eq!(
            accepted.request.status, "accepted",
            "request {index} must not be rejected by historical lifetime counters: {:?}",
            accepted.request.decision_reason_code
        );
        assert!(accepted.work_item.is_some());
    }

    let accepted_requests: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM stage_worker_requests WHERE team_plan_id=$1 AND status='accepted'",
    )
    .bind(seeded.plan.id)
    .fetch_one(db.pool())
    .await
    .expect("count scope-bounded accepted requests");
    let work_items: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM stage_work_items WHERE team_plan_id=$1")
            .bind(seeded.plan.id)
            .fetch_one(db.pool())
            .await
            .expect("count scope-bounded WorkItems");
    assert_eq!(accepted_requests, 6);
    assert_eq!(work_items, 7, "one Controller plus all six child WorkItems");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn company_controller_keeps_live_concurrency_and_retries_without_a_lifetime_total() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    let mut seed = stage_team_controller_seed(&roots);
    seed.plan.max_workers_total = 3;
    seed.plan.max_workers_active = 3;
    let hash = |character: char| format!("sha256:{}", character.to_string().repeat(64));
    for index in 0..3 {
        seed.work_items
            .push(runtime_memory_tx::StageWorkItemSeedRow {
                stable_key: format!("scope-shard:{index}"),
                work_item_kind: "stage_axis".to_string(),
                role: "intel_researcher".to_string(),
                input_manifest: serde_json::json!({"scope_shard": index}),
                input_manifest_hash: hash(char::from(b'a' + index as u8)),
                conflict_key: None,
                priority: index + 1,
                required_for_barrier: true,
                is_aggregator: false,
                attempt_policy: serde_json::json!({"max_attempts": 2}),
                budget: serde_json::json!({}),
                output_schema: "stage_worker_output.v1".to_string(),
                created_by: "server_seed".to_string(),
            });
    }
    let seeded = runtime_memory_tx::seed_stage_team_runtime(db.pool(), &seed)
        .await
        .expect("seed concurrency-bounded Controller Team")
        .remove(0);
    let claim = stage_team_claim_input(&roots, &seeded, "scope-bounded-claim");
    let _controller = runtime_memory_tx::claim_stage_team_leader(
        db.pool(),
        &runtime_memory_tx::ClaimStageTeamLeaderRow {
            claim: claim.clone(),
        },
    )
    .await
    .expect("claim Company Controller")
    .expect("Controller is runnable");
    let first = runtime_memory_tx::claim_stage_work_item(db.pool(), &claim)
        .await
        .expect("claim first live child")
        .expect("first child is runnable");
    let _second = runtime_memory_tx::claim_stage_work_item(db.pool(), &claim)
        .await
        .expect("claim second live child")
        .expect("second child is runnable");
    assert!(
        runtime_memory_tx::claim_stage_work_item(db.pool(), &claim)
            .await
            .expect("active concurrency check")
            .is_none(),
        "K=3 includes the Controller and blocks a third simultaneous child"
    );

    let retried = stage_teams::retry_stage_worker(
        db.pool(),
        stage_teams::RetryStageWorkerRow {
            fence: stage_team_fence(&roots, &seeded, &first),
            team_plan_id: seeded.plan.id,
            work_item_id: first.work_item.id,
            expected_work_item_row_version: first.work_item.row_version,
            failure_code: "provider_unavailable".to_string(),
            terminal_checkpoint: serde_json::json!({
                "stage_team_execution_failure": {"code": "provider_unavailable"}
            }),
        },
    )
    .await
    .expect("retry Company Controller child at historical total");
    assert!(
        retried.retry_scheduled,
        "the WorkItem's own attempt policy, not a Team lifetime total, owns retry fuel"
    );
    let retry = runtime_memory_tx::claim_stage_work_item(db.pool(), &claim)
        .await
        .expect("claim retry beyond historical WorkerRun total")
        .expect("a freed live slot admits the retry");
    assert_eq!(retry.work_item.id, first.work_item.id);
    assert_ne!(retry.worker.id, first.worker.id);
    let worker_runs: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM stage_worker_runs WHERE stage_run_unit_id=$1")
            .bind(seeded.unit.id)
            .fetch_one(db.pool())
            .await
            .expect("count WorkerRuns beyond compatibility total");
    assert_eq!(worker_runs, 4);
    assert!(worker_runs > i64::from(seeded.plan.max_workers_total));

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn company_controller_parks_for_dynamic_child_and_resumes_same_worker_chain() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    let seeded =
        runtime_memory_tx::seed_stage_team_runtime(db.pool(), &stage_team_controller_seed(&roots))
            .await
            .expect("seed controller-only Team")
            .remove(0);
    assert_eq!(seeded.work_items.len(), 1);
    assert_eq!(seeded.work_items[0].stable_key, "leader:primary");

    let claim = stage_team_claim_input(&roots, &seeded, "controller-fixture");
    let controller = runtime_memory_tx::claim_stage_team_leader(
        db.pool(),
        &runtime_memory_tx::ClaimStageTeamLeaderRow {
            claim: claim.clone(),
        },
    )
    .await
    .expect("claim Company Controller")
    .expect("Controller is immediately runnable");
    assert_eq!(controller.unit.id, seeded.unit.id);
    assert_eq!(controller.unit.status, "running");
    assert_eq!(controller.unit.row_version, seeded.unit.row_version + 1);
    assert_eq!(controller.work_item.id, seeded.work_items[0].id);
    assert_eq!(controller.work_item.role, seeded.plan.leader_role);
    assert_eq!(controller.plan.final_submitter_worker_run_id, None);
    let original_worker_id = controller.worker.id;
    let original_chain_id = controller.message_chain_id;
    let original_attempt_epoch = controller.worker.attempt_epoch;
    let controller_fence = stage_team_fence(&roots, &seeded, &controller);

    let mut request = runtime_memory_tx::RequestStageWorkerRow {
        fence: controller_fence.clone(),
        stage_team_plan_id: seeded.plan.id,
        parent_work_item_id: controller.work_item.id,
        expected_dispatch_epoch: seeded.plan.dispatch_epoch,
        requested_role: "intel_researcher".to_string(),
        requested_kind: "stage_axis".to_string(),
        subject_refs: Vec::new(),
        reason: serde_json::json!({
            "schema": "stage_team_controller_request.v1",
            "objective": "Check DNS and CT obligations for this frozen organization",
            "parent_tool_request_id": "lead-tool-call-1",
        })
        .to_string(),
        // These caller values must not become the durable child contract.
        output_schema: serde_json::json!("caller-owned-schema"),
        budget_hint: serde_json::json!({"caller_owned": true}),
        dedupe_key: "controller-round-1:dns-ct".to_string(),
        request_sha256: String::new(),
    };
    request.request_sha256 = runtime_memory_tx::stage_worker_request_payload_hash(&request);
    let accepted = runtime_memory_tx::request_stage_worker(db.pool(), &request)
        .await
        .expect("Controller request is durably decided");
    assert_eq!(accepted.request.status, "accepted");
    let child_item = accepted.work_item.expect("accepted child WorkItem");
    assert_eq!(child_item.output_schema, "stage_worker_output.v1");
    assert_eq!(child_item.budget, serde_json::json!({}));

    let replayed =
        runtime_memory_tx::seed_stage_team_runtime(db.pool(), &stage_team_controller_seed(&roots))
            .await
            .expect("replay accepts the canonical Controller child assignment");
    let replayed = replayed
        .into_iter()
        .find(|row| row.unit.id == seeded.unit.id)
        .expect("replay returns the original Controller unit");
    assert_eq!(replayed.plan.id, seeded.plan.id);
    let child_still_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM stage_work_items WHERE id=$1)")
            .bind(child_item.id)
            .fetch_one(db.pool())
            .await
            .expect("read the dynamically accepted child after replay");
    assert!(
        child_still_exists,
        "replay preserves the dynamically accepted child WorkItem"
    );

    let parked = runtime_memory_tx::park_stage_team_leader(
        db.pool(),
        &runtime_memory_tx::ParkStageTeamLeaderRow {
            fence: controller_fence,
            stage_team_plan_id: seeded.plan.id,
            leader_work_item_id: controller.work_item.id,
            expected_work_item_row_version: controller.work_item.row_version,
            checkpoint: serde_json::json!({"controller_round": 1, "waiting": true}),
        },
    )
    .await
    .expect("atomically park Controller behind accepted children");
    assert_eq!(parked.work_item.status, "waiting_dependency");
    assert_eq!(parked.worker.status, "waiting_background");
    assert_eq!(parked.dependency_count, 1);

    let not_ready = runtime_memory_tx::claim_stage_team_leader(
        db.pool(),
        &runtime_memory_tx::ClaimStageTeamLeaderRow {
            claim: claim.clone(),
        },
    )
    .await
    .expect("not-ready Controller claim is a stable queue result");
    assert!(not_ready.is_none(), "live child keeps Controller parked");

    let child = runtime_memory_tx::claim_stage_work_item(db.pool(), &claim)
        .await
        .expect("claim Controller child")
        .expect("accepted child is queued");
    assert_eq!(child.work_item.id, child_item.id);
    assert_ne!(child.worker.id, original_worker_id);
    assert_eq!(
        child.worker.parent_request_id.as_deref(),
        Some("lead-tool-call-1")
    );
    let evidence_id = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO audit_log (
               action, category, details, project_path, source, audit_role,
               detail, run_id, created_at
           ) VALUES (
               'controller child evidence','harness','fresh controller child evidence',
               '/tmp/runtime-worker','harness','evidence',$1,$2,NOW()
           ) RETURNING id"#,
    )
    .bind(serde_json::json!({"organization_id": roots.organization_id}))
    .bind(roots.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("insert Controller child evidence");
    let mut completion = stage_teams::CompleteStageWorkerRow {
        fence: stage_team_fence(&roots, &seeded, &child),
        team_plan_id: seeded.plan.id,
        work_item_id: child.work_item.id,
        expected_work_item_row_version: child.work_item.row_version,
        output_schema: "stage_worker_output.v1".to_string(),
        business_disposition: "found".to_string(),
        canonical_output: serde_json::json!({"facts": []}),
        canonical_fact_refs: serde_json::json!([]),
        evidence_ids: vec![evidence_id],
        checked_empty_cells: serde_json::json!([]),
        blocker_codes: Vec::new(),
        output_hash: String::new(),
        terminal_checkpoint: serde_json::json!({"done": true}),
        evidence_watermark: Some(evidence_id),
    };
    refresh_stage_worker_output_hash(&mut completion);
    stage_teams::complete_stage_worker(db.pool(), completion)
        .await
        .expect("complete Controller child with immutable output");

    let resumed = runtime_memory_tx::claim_stage_team_leader(
        db.pool(),
        &runtime_memory_tx::ClaimStageTeamLeaderRow { claim },
    )
    .await
    .expect("resume Controller after child barrier")
    .expect("Controller dependencies are ready");
    assert_eq!(resumed.worker.id, original_worker_id);
    assert_eq!(resumed.message_chain_id, original_chain_id);
    assert_eq!(resumed.worker.attempt_epoch, original_attempt_epoch + 1);
    assert_eq!(resumed.work_item.status, "running");
    assert_eq!(resumed.unit.id, controller.unit.id);
    assert_eq!(resumed.unit.status, "running");
    assert_eq!(resumed.unit.row_version, controller.unit.row_version);

    let closed = runtime_memory_tx::close_stage_request_epoch(
        db.pool(),
        &runtime_memory_tx::CloseStageRequestEpochRow {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: seeded.unit.id,
            stage_team_plan_id: seeded.plan.id,
            expected_dispatch_epoch: seeded.plan.dispatch_epoch,
            expected_plan_row_version: resumed.plan.row_version,
        },
    )
    .await
    .expect("Controller closes its request epoch before final submission");
    assert!(closed.barrier.ready_to_finalize());
    let bound = runtime_memory_tx::bind_stage_team_leader_final_submitter(
        db.pool(),
        &runtime_memory_tx::BindStageTeamLeaderFinalSubmitterRow {
            fence: stage_team_fence(&roots, &seeded, &resumed),
            stage_team_plan_id: seeded.plan.id,
            leader_work_item_id: resumed.work_item.id,
            expected_plan_row_version: closed.plan.row_version,
            expected_dispatch_epoch: closed.plan.dispatch_epoch,
            expected_manifest_hash: closed.barrier.manifest_hash.clone(),
        },
    )
    .await
    .expect("bind the same Controller as sole final submitter");
    assert_eq!(
        bound.plan.final_submitter_worker_run_id,
        Some(original_worker_id)
    );
    assert!(!bound.replayed);

    let (submission, controller_after_tool) = persist_stage_team_submission(
        &db,
        &roots,
        &seeded,
        &resumed,
        "controller-dynamic-child-gate-submit",
    )
    .await;
    let gate_decision_hash = format!(
        "sha256:{}",
        sha256_json(&serde_json::json!({"decision": "block", "round": 1}))
    );
    let gap_manifest = serde_json::json!({
        "gate_decision_hash": gate_decision_hash,
        "reasons": ["missing_exact_web_origin"],
        "schema_version": 1,
    });
    let reopened = runtime_memory_tx::reopen_stage_team_leader_after_gate_block(
        db.pool(),
        &runtime_memory_tx::ReopenStageTeamLeaderAfterGateBlockRow {
            request_id: "controller-dynamic-child-gate-reopen".to_string(),
            fence: runtime_memory_tx::RuntimeMemoryTxFence {
                operation_id: roots.operation_id,
                stage_execution_id: roots.stage_execution_id,
                stage_run_unit_id: seeded.unit.id,
                worker_run_id: original_worker_id,
                lease_token: controller_after_tool
                    .lease_token
                    .expect("Controller lease remains after Gate tool"),
                attempt_epoch: controller_after_tool.attempt_epoch,
                expected_checkpoint_version: controller_after_tool.checkpoint_version,
            },
            stage_team_plan_id: seeded.plan.id,
            leader_work_item_id: resumed.work_item.id,
            deliverable_submission_id: submission.id,
            expected_dispatch_epoch: bound.plan.dispatch_epoch,
            expected_manifest_hash: closed.barrier.manifest_hash,
            gate_decision_hash: gate_decision_hash.clone(),
            gap_manifest_hash: format!("sha256:{}", sha256_json(&gap_manifest)),
            gap_manifest,
            checkpoint: serde_json::json!({"resume_after_gate_block": true}),
        },
    )
    .await
    .expect("reopen Controller after dynamic child output reached Gate");
    assert_eq!(reopened.plan.dispatch_epoch, seeded.plan.dispatch_epoch + 1);

    let restarted_after_epoch_advance =
        runtime_memory_tx::seed_stage_team_runtime(db.pool(), &stage_team_controller_seed(&roots))
            .await
            .expect("stage_run reentry accepts historical static and dynamic WorkItem epochs");
    assert_eq!(restarted_after_epoch_advance.len(), 1);
    assert!(restarted_after_epoch_advance[0].replayed);
    assert_eq!(
        restarted_after_epoch_advance[0].plan.dispatch_epoch,
        reopened.plan.dispatch_epoch
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn investigation_primary_reentry_reads_the_parked_worker_before_claiming_its_child() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract_stage_and_children(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
        "investigation",
        0,
    )
    .await
    .0;
    let mut seed = stage_team_controller_seed_for_stage(&roots, "investigation");
    seed.base.specialist = "investigation".to_string();
    seed.base.agent_path_prefix = "main>stage_run:investigation".to_string();
    seed.plan.leader_role = "investigation".to_string();
    seed.plan.allowed_roles = vec!["investigation".to_string(), "adviser".to_string()];
    seed.plan.aggregator_role = Some("investigation".to_string());
    seed.plan.dynamic_request_policy = serde_json::json!({
        "allowed_request_kinds": ["analysis_task"],
        "canonical_subject_refs_only": true,
        "child_budget": {},
        "child_output_schema": "investigation_cognitive_output.v1",
        "coordination_mode": "investigation_task_orchestrator",
        "max_requests": 8,
        "max_subject_refs": 8,
        "organization_scope_implicit": true,
        "attempt_policy": {"max_attempts": 3}
    });
    seed.work_items[0].work_item_kind = "investigation_primary".to_string();
    seed.work_items[0].role = "investigation".to_string();
    seed.work_items[0].conflict_key = None;
    let seeded = runtime_memory_tx::seed_stage_team_runtime(db.pool(), &seed)
        .await
        .expect("seed Investigation Primary")
        .remove(0);
    let claim = stage_team_claim_input(&roots, &seeded, "investigation-primary-reentry");
    let primary = runtime_memory_tx::claim_stage_team_leader(
        db.pool(),
        &runtime_memory_tx::ClaimStageTeamLeaderRow {
            claim: claim.clone(),
        },
    )
    .await
    .expect("claim Investigation Primary")
    .expect("Investigation Primary is runnable");
    let primary_worker_id = primary.worker.id;
    let primary_chain_id = primary.message_chain_id;
    let mut request = runtime_memory_tx::RequestStageWorkerRow {
        fence: stage_team_fence(&roots, &seeded, &primary),
        stage_team_plan_id: seeded.plan.id,
        parent_work_item_id: primary.work_item.id,
        expected_dispatch_epoch: seeded.plan.dispatch_epoch,
        requested_role: "adviser".to_string(),
        requested_kind: "analysis_task".to_string(),
        subject_refs: Vec::new(),
        reason: serde_json::json!({
            "schema": "stage_team_controller_request.v1",
            "objective": "Continue the exact durable Investigation subtask",
            "parent_tool_request_id": "investigation-child-1"
        })
        .to_string(),
        output_schema: serde_json::json!("investigation_cognitive_output.v1"),
        budget_hint: serde_json::json!({}),
        dedupe_key: "investigation-child-1".to_string(),
        request_sha256: String::new(),
    };
    request.request_sha256 = runtime_memory_tx::stage_worker_request_payload_hash(&request);
    let accepted = runtime_memory_tx::request_stage_worker(db.pool(), &request)
        .await
        .expect("persist exact Investigation child request");
    let child = accepted.work_item.expect("accepted child WorkItem");
    let parked = runtime_memory_tx::park_stage_team_leader(
        db.pool(),
        &runtime_memory_tx::ParkStageTeamLeaderRow {
            fence: stage_team_fence(&roots, &seeded, &primary),
            stage_team_plan_id: seeded.plan.id,
            leader_work_item_id: primary.work_item.id,
            expected_work_item_row_version: primary.work_item.row_version,
            checkpoint: serde_json::json!({"waiting_for": child.id}),
        },
    )
    .await
    .expect("park Investigation Primary behind its child");
    assert_eq!(parked.work_item.status, "waiting_dependency");
    assert_eq!(parked.worker.status, "waiting_background");

    let reentered = runtime_memory_tx::claim_stage_team_leader(
        db.pool(),
        &runtime_memory_tx::ClaimStageTeamLeaderRow { claim },
    )
    .await
    .expect("read exact parked Investigation Primary")
    .expect("Investigation host must regain the parked Primary to schedule its durable child");
    assert_eq!(reentered.work_item.id, primary.work_item.id);
    assert_eq!(reentered.work_item.status, "waiting_dependency");
    assert_eq!(reentered.worker.id, primary_worker_id);
    assert_eq!(reentered.worker.status, "waiting_background");
    assert_eq!(reentered.worker.lease_token, None);
    assert_eq!(reentered.message_chain_id, primary_chain_id);

    let restarted = runtime_memory_tx::seed_stage_team_runtime(db.pool(), &seed)
        .await
        .expect("replay the static Investigation denominator")
        .remove(0);
    assert!(
        restarted.work_items.iter().all(|item| item.id != child.id),
        "dynamic child remains outside the immutable static seed response"
    );
    let loaded_child = runtime_memory_tx::load_stage_work_item(
        db.pool(),
        &runtime_memory_tx::LoadStageWorkItemRow {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: seeded.unit.id,
            stage_team_plan_id: seeded.plan.id,
            organization_id: roots.organization_id,
            work_item_id: child.id,
        },
    )
    .await
    .expect("load exact durable Investigation child")
    .expect("accepted dynamic child remains readable after restart");
    assert_eq!(loaded_child.work_item.id, child.id);
    assert_eq!(loaded_child.work_item.created_by, "accepted_worker_request");
    assert_eq!(loaded_child.work_item.status, "queued");
    assert_eq!(
        loaded_child.aggregator_role.as_deref(),
        Some("investigation")
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn company_controller_continue_reclaims_interrupted_eas_child_on_same_chain() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    let seeded =
        runtime_memory_tx::seed_stage_team_runtime(db.pool(), &stage_team_controller_seed(&roots))
            .await
            .expect("seed interrupted-child Controller Team")
            .remove(0);
    let claim = stage_team_claim_input(&roots, &seeded, "controller-interrupted-eas-child");
    let controller = runtime_memory_tx::claim_stage_team_leader(
        db.pool(),
        &runtime_memory_tx::ClaimStageTeamLeaderRow {
            claim: claim.clone(),
        },
    )
    .await
    .expect("claim Controller before interrupted child")
    .expect("Controller is runnable");
    let mut request = runtime_memory_tx::RequestStageWorkerRow {
        fence: stage_team_fence(&roots, &seeded, &controller),
        stage_team_plan_id: seeded.plan.id,
        parent_work_item_id: controller.work_item.id,
        expected_dispatch_epoch: seeded.plan.dispatch_epoch,
        requested_role: "intel_researcher".to_string(),
        requested_kind: "stage_axis".to_string(),
        subject_refs: Vec::new(),
        reason: serde_json::json!({
            "schema": "stage_team_controller_request.v1",
            "objective": "Fingerprint only the exact current EAS service worklist gaps",
            "parent_tool_request_id": "controller-interrupted-eas-request",
        })
        .to_string(),
        output_schema: serde_json::json!("ignored"),
        budget_hint: serde_json::json!({}),
        dedupe_key: "controller-interrupted-eas".to_string(),
        request_sha256: String::new(),
    };
    request.request_sha256 = runtime_memory_tx::stage_worker_request_payload_hash(&request);
    let child_item = runtime_memory_tx::request_stage_worker(db.pool(), &request)
        .await
        .expect("accept interrupted EAS child")
        .work_item
        .expect("child WorkItem is accepted");
    runtime_memory_tx::park_stage_team_leader(
        db.pool(),
        &runtime_memory_tx::ParkStageTeamLeaderRow {
            fence: stage_team_fence(&roots, &seeded, &controller),
            stage_team_plan_id: seeded.plan.id,
            leader_work_item_id: controller.work_item.id,
            expected_work_item_row_version: controller.work_item.row_version,
            checkpoint: serde_json::json!({"waiting_for": child_item.id}),
        },
    )
    .await
    .expect("park Controller behind EAS child");
    let child = runtime_memory_tx::claim_stage_work_item(db.pool(), &claim)
        .await
        .expect("claim EAS child")
        .expect("EAS child is queued");
    assert_eq!(child.work_item.id, child_item.id);
    let original_worker_id = child.worker.id;
    let original_chain_id = child.message_chain_id;
    let original_attempt_epoch = child.worker.attempt_epoch;
    let active_tool_id = tool_calls::record_tracked_start(
        db.pool(),
        "controller-child-interrupted-eas-service-fingerprint",
        roots.session_id,
        Some(roots.operation_id),
        None,
        "eas_fingerprint_services",
        &serde_json::json!({
            "targets": [{
                "target_id": Uuid::new_v4(),
                "target_ip": "192.0.2.10",
                "ports": [443]
            }]
        }),
        Some(&tool_calls::RuntimeToolIdentity {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: Some(seeded.unit.id),
            worker_run_id: Some(child.worker.id),
            organization_id: Some(roots.organization_id),
            attempt_epoch: Some(child.worker.attempt_epoch),
            lease_token: child.worker.lease_token,
        }),
    )
    .await
    .expect("record Controller child EAS service fingerprint tool");
    runtime_memory_tx::begin_worker_tool(
        db.pool(),
        &stage_team_fence(&roots, &seeded, &child),
        active_tool_id,
    )
    .await
    .expect("bind Controller child EAS service fingerprint tool");
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET lease_acquired_at=NOW()-INTERVAL '2 hours',
                  lease_expires_at=NOW()-INTERVAL '1 hour',
                  heartbeat_at=NOW()-INTERVAL '1 hour'
            WHERE id=$1"#,
    )
    .bind(child.worker.id)
    .execute(db.pool())
    .await
    .expect("expire Controller child EAS lease");
    sqlx::query(
        "UPDATE tasks SET status='running',updated_at=NOW()-INTERVAL '7 hours' WHERE id=$1",
    )
    .bind(roots.operation_id)
    .execute(db.pool())
    .await
    .expect("age Controller EAS task before restart");
    let startup = tasks::startup_reap_abandoned(db.pool(), chrono::Duration::zero())
        .await
        .expect("park interrupted Controller EAS child on startup");
    assert_eq!(startup.workers_recovery_required, 1);

    let controller_wait = runtime_memory_tx::claim_stage_team_leader(
        db.pool(),
        &runtime_memory_tx::ClaimStageTeamLeaderRow {
            claim: claim.clone(),
        },
    )
    .await
    .expect("Controller continue reconciles its interrupted EAS child");
    assert!(
        controller_wait.is_none(),
        "Controller stays parked until the reconciled child actually completes"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status::text FROM stage_work_items WHERE id=$1")
            .bind(child_item.id)
            .fetch_one(db.pool())
            .await
            .expect("load Controller-reconciled child status"),
        "queued",
        "Controller continuation must reconcile a safe child before inspecting the barrier"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status::text FROM tool_calls WHERE id=$1")
            .bind(active_tool_id)
            .fetch_one(db.pool())
            .await
            .expect("load Controller-reconciled tool status"),
        "failed"
    );
    let resumed_child = runtime_memory_tx::claim_stage_work_item(db.pool(), &claim)
        .await
        .expect("claim reconciled Controller EAS child")
        .expect("same Controller EAS child is runnable again");
    assert_eq!(resumed_child.work_item.id, child_item.id);
    assert_eq!(resumed_child.worker.id, original_worker_id);
    assert_eq!(resumed_child.message_chain_id, original_chain_id);
    assert_eq!(
        resumed_child.worker.attempt_epoch,
        original_attempt_epoch + 1
    );
    assert_eq!(
        resumed_child.worker.checkpoint["stage_team_interrupted_tool_recovery"]["kind"],
        "resume_after_reconcile"
    );
    assert_eq!(
        resumed_child.worker.checkpoint["stage_team_interrupted_tool_recovery"]["tool_name"],
        "eas_fingerprint_services"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status::text FROM tool_calls WHERE id=$1")
            .bind(active_tool_id)
            .fetch_one(db.pool())
            .await
            .expect("load reconciled child tool status"),
        "failed"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn expired_company_controller_reclaims_same_worker_and_message_chain() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    let seeded =
        runtime_memory_tx::seed_stage_team_runtime(db.pool(), &stage_team_controller_seed(&roots))
            .await
            .expect("seed controller-only Team")
            .remove(0);
    let claim = stage_team_claim_input(&roots, &seeded, "expired-controller-resume");
    let controller = runtime_memory_tx::claim_stage_team_leader(
        db.pool(),
        &runtime_memory_tx::ClaimStageTeamLeaderRow {
            claim: claim.clone(),
        },
    )
    .await
    .expect("claim Company Controller")
    .expect("Controller is runnable");
    let original_worker_id = controller.worker.id;
    let original_chain_id = controller.message_chain_id;
    let original_attempt_epoch = controller.worker.attempt_epoch;

    sqlx::query(
        "UPDATE stage_worker_runs
            SET lease_acquired_at=NOW() - INTERVAL '2 hours',
                lease_expires_at=NOW() - INTERVAL '1 hour',
                heartbeat_at=NOW() - INTERVAL '1 hour'
          WHERE id=$1 AND status='running' AND active_tool_call_id IS NULL",
    )
    .bind(original_worker_id)
    .execute(db.pool())
    .await
    .expect("expire idle Controller lease");

    let resumed = runtime_memory_tx::claim_stage_team_leader(
        db.pool(),
        &runtime_memory_tx::ClaimStageTeamLeaderRow { claim },
    )
    .await
    .expect("reclaim expired Company Controller")
    .expect("expired idle Controller is immediately resumable");
    assert_eq!(resumed.worker.id, original_worker_id);
    assert_eq!(resumed.message_chain_id, original_chain_id);
    assert_eq!(resumed.worker.attempt_epoch, original_attempt_epoch + 1);
    assert_eq!(resumed.worker.status, "running");

    let controller_worker_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM stage_worker_runs WHERE work_item_id=$1")
            .bind(controller.work_item.id)
            .fetch_one(db.pool())
            .await
            .expect("count logical Controller workers");
    assert_eq!(
        controller_worker_count, 1,
        "lease recovery must not mint a second logical Controller"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn interrupted_company_controller_browser_reconciles_on_exact_chain() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    let seeded =
        runtime_memory_tx::seed_stage_team_runtime(db.pool(), &stage_team_controller_seed(&roots))
            .await
            .expect("seed browser-recovery Controller Team")
            .remove(0);
    let claim = stage_team_claim_input(&roots, &seeded, "interrupted-controller-browser");
    let controller = runtime_memory_tx::claim_stage_team_leader(
        db.pool(),
        &runtime_memory_tx::ClaimStageTeamLeaderRow {
            claim: claim.clone(),
        },
    )
    .await
    .expect("claim Company Controller")
    .expect("Controller is runnable");
    let active_tool_id = tool_calls::record_tracked_start(
        db.pool(),
        "controller-interrupted-browser",
        roots.session_id,
        Some(roots.operation_id),
        None,
        "browser_collect_js_api",
        &serde_json::json!({
            "target_id": Uuid::new_v4(),
            "target_url": "https://browser-recovery.example.test",
            "max_actions": 0
        }),
        Some(&tool_calls::RuntimeToolIdentity {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: Some(seeded.unit.id),
            worker_run_id: Some(controller.worker.id),
            organization_id: Some(roots.organization_id),
            attempt_epoch: Some(controller.worker.attempt_epoch),
            lease_token: controller.worker.lease_token,
        }),
    )
    .await
    .expect("record exact browser collection tool");
    runtime_memory_tx::begin_worker_tool(
        db.pool(),
        &stage_team_fence(&roots, &seeded, &controller),
        active_tool_id,
    )
    .await
    .expect("bind browser tool to Controller");
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET lease_acquired_at=NOW()-INTERVAL '2 hours',
                  lease_expires_at=NOW()-INTERVAL '1 hour',
                  heartbeat_at=NOW()-INTERVAL '1 hour'
            WHERE id=$1"#,
    )
    .bind(controller.worker.id)
    .execute(db.pool())
    .await
    .expect("expire interrupted Controller browser lease");
    sqlx::query(
        "UPDATE tasks SET status='running',updated_at=NOW()-INTERVAL '7 hours' WHERE id=$1",
    )
    .bind(roots.operation_id)
    .execute(db.pool())
    .await
    .expect("age interrupted Controller browser task");

    let startup = tasks::startup_reap_abandoned(db.pool(), chrono::Duration::zero())
        .await
        .expect("park interrupted Controller browser during startup reconciliation");
    assert_eq!(startup.workers_recovery_required, 1);

    let resumed = runtime_memory_tx::claim_stage_team_leader(
        db.pool(),
        &runtime_memory_tx::ClaimStageTeamLeaderRow { claim },
    )
    .await
    .expect("reconcile interrupted Controller browser")
    .expect("same Controller is claimable after browser reconciliation");
    assert_eq!(resumed.work_item.id, controller.work_item.id);
    assert_eq!(resumed.worker.id, controller.worker.id);
    assert_eq!(resumed.message_chain_id, controller.message_chain_id);
    assert_eq!(
        resumed.worker.attempt_epoch,
        controller.worker.attempt_epoch + 1
    );
    assert_eq!(resumed.worker.active_tool_call_id, None);
    assert_eq!(
        resumed.worker.checkpoint["stage_team_interrupted_tool_recovery"]["tool_name"],
        "browser_collect_js_api"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status::text FROM tool_calls WHERE id=$1")
            .bind(active_tool_id)
            .fetch_one(db.pool())
            .await
            .expect("load reconciled browser tool status"),
        "failed"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn stage_team_repair_epoch_dispatch_accepts_child_from_stable_company_controller() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    let seeded =
        runtime_memory_tx::seed_stage_team_runtime(db.pool(), &stage_team_controller_seed(&roots))
            .await
            .expect("seed repair-dispatch Controller Team")
            .remove(0);
    let claim = stage_team_claim_input(&roots, &seeded, "repair-epoch-dispatch-controller");
    let controller = runtime_memory_tx::claim_stage_team_leader(
        db.pool(),
        &runtime_memory_tx::ClaimStageTeamLeaderRow {
            claim: claim.clone(),
        },
    )
    .await
    .expect("claim Controller before repair")
    .expect("Controller is runnable");
    let original_controller_epoch = controller.work_item.dispatch_epoch;

    let closed = runtime_memory_tx::close_stage_request_epoch(
        db.pool(),
        &runtime_memory_tx::CloseStageRequestEpochRow {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: seeded.unit.id,
            stage_team_plan_id: seeded.plan.id,
            expected_dispatch_epoch: controller.plan.dispatch_epoch,
            expected_plan_row_version: controller.plan.row_version,
        },
    )
    .await
    .expect("close initial Controller epoch");
    let bound = runtime_memory_tx::bind_stage_team_leader_final_submitter(
        db.pool(),
        &runtime_memory_tx::BindStageTeamLeaderFinalSubmitterRow {
            fence: stage_team_fence(&roots, &seeded, &controller),
            stage_team_plan_id: seeded.plan.id,
            leader_work_item_id: controller.work_item.id,
            expected_plan_row_version: closed.plan.row_version,
            expected_dispatch_epoch: closed.plan.dispatch_epoch,
            expected_manifest_hash: closed.barrier.manifest_hash.clone(),
        },
    )
    .await
    .expect("bind Controller before Gate BLOCK");
    let (submission, controller_after_tool) = persist_stage_team_submission(
        &db,
        &roots,
        &seeded,
        &controller,
        "repair-epoch-dispatch-submit",
    )
    .await;
    let gate_decision_hash = format!(
        "sha256:{}",
        sha256_json(&serde_json::json!({
            "decision": "block",
            "reason": "retry exact origins",
        }))
    );
    let gap_manifest = serde_json::json!({
        "gate_decision_hash": gate_decision_hash,
        "reasons": ["retry_exact_web_origins"],
        "schema_version": 1,
    });
    let reopened = runtime_memory_tx::reopen_stage_team_leader_after_gate_block(
        db.pool(),
        &runtime_memory_tx::ReopenStageTeamLeaderAfterGateBlockRow {
            request_id: "repair-epoch-dispatch-gate-block".to_string(),
            fence: runtime_memory_tx::RuntimeMemoryTxFence {
                operation_id: roots.operation_id,
                stage_execution_id: roots.stage_execution_id,
                stage_run_unit_id: seeded.unit.id,
                worker_run_id: controller.worker.id,
                lease_token: controller_after_tool
                    .lease_token
                    .expect("Controller lease remains after submit tool"),
                attempt_epoch: controller_after_tool.attempt_epoch,
                expected_checkpoint_version: controller_after_tool.checkpoint_version,
            },
            stage_team_plan_id: seeded.plan.id,
            leader_work_item_id: controller.work_item.id,
            deliverable_submission_id: submission.id,
            expected_dispatch_epoch: bound.plan.dispatch_epoch,
            expected_manifest_hash: closed.barrier.manifest_hash,
            gate_decision_hash: gate_decision_hash.clone(),
            gap_manifest_hash: format!("sha256:{}", sha256_json(&gap_manifest)),
            gap_manifest,
            checkpoint: serde_json::json!({"repair": "retry exact origins"}),
        },
    )
    .await
    .expect("open exact Controller repair epoch");
    let resumed = runtime_memory_tx::claim_stage_team_leader(
        db.pool(),
        &runtime_memory_tx::ClaimStageTeamLeaderRow { claim },
    )
    .await
    .expect("claim reopened Controller")
    .expect("reopened Controller is runnable");
    assert_eq!(resumed.work_item.id, controller.work_item.id);
    assert_eq!(resumed.work_item.dispatch_epoch, original_controller_epoch);
    assert_eq!(resumed.plan.dispatch_epoch, reopened.plan.dispatch_epoch);
    assert_eq!(resumed.plan.dispatch_epoch, original_controller_epoch + 1);

    let mut request = runtime_memory_tx::RequestStageWorkerRow {
        fence: stage_team_fence(&roots, &seeded, &resumed),
        stage_team_plan_id: resumed.plan.id,
        parent_work_item_id: resumed.work_item.id,
        expected_dispatch_epoch: resumed.plan.dispatch_epoch,
        requested_role: "intel_researcher".to_string(),
        requested_kind: "stage_axis".to_string(),
        subject_refs: Vec::new(),
        reason: serde_json::json!({
            "schema": "stage_team_controller_request.v1",
            "objective": "Retry the five exact origins from the durable Gate gap",
            "parent_tool_request_id": "repair-epoch-dispatch-tool",
        })
        .to_string(),
        output_schema: serde_json::json!("ignored-by-controller-policy"),
        budget_hint: serde_json::json!({}),
        dedupe_key: "repair-epoch:retry-exact-origins".to_string(),
        request_sha256: String::new(),
    };
    request.request_sha256 = runtime_memory_tx::stage_worker_request_payload_hash(&request);
    let accepted = runtime_memory_tx::request_stage_worker(db.pool(), &request)
        .await
        .expect("repair epoch Controller dispatch is accepted");
    assert_eq!(accepted.request.status, "accepted");
    assert_eq!(accepted.request.dispatch_epoch, resumed.plan.dispatch_epoch);
    assert_eq!(
        accepted
            .work_item
            .as_ref()
            .expect("accepted repair child")
            .dispatch_epoch,
        resumed.plan.dispatch_epoch
    );
    assert_eq!(accepted.request.parent_work_item_id, resumed.work_item.id);

    let mut authority_tx = db
        .pool()
        .begin()
        .await
        .expect("begin authority-loss fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *authority_tx)
        .await
        .expect("disable immutable repair-generation trigger in fixture");
    sqlx::query(
        "DELETE FROM stage_team_repair_generations WHERE team_plan_id=$1 AND dispatch_epoch=$2",
    )
    .bind(resumed.plan.id)
    .bind(resumed.plan.dispatch_epoch)
    .execute(&mut *authority_tx)
    .await
    .expect("remove only the server-owned cross-epoch authority in fixture");
    authority_tx
        .commit()
        .await
        .expect("commit authority-loss fixture");

    let mut unauthorized = request.clone();
    unauthorized.dedupe_key = "repair-epoch:unauthorized-followup".to_string();
    unauthorized.request_sha256 =
        runtime_memory_tx::stage_worker_request_payload_hash(&unauthorized);
    assert!(matches!(
        runtime_memory_tx::request_stage_worker(db.pool(), &unauthorized).await,
        Err(runtime_memory_tx::RuntimeMemoryStoreError::Conflict {
            code: "stage_team_controller_parent_epoch_not_authorized"
        })
    ));

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn company_controller_gate_block_reopens_same_worker_chain_until_repair_fuel_is_exhausted() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    let seeded =
        runtime_memory_tx::seed_stage_team_runtime(db.pool(), &stage_team_controller_seed(&roots))
            .await
            .expect("seed controller-only Team")
            .remove(0);
    let claim = stage_team_claim_input(&roots, &seeded, "gate-repair-controller");
    let controller = runtime_memory_tx::claim_stage_team_leader(
        db.pool(),
        &runtime_memory_tx::ClaimStageTeamLeaderRow {
            claim: claim.clone(),
        },
    )
    .await
    .expect("claim Company Controller")
    .expect("Controller is runnable");
    let original_worker_id = controller.worker.id;
    let original_chain_id = controller.message_chain_id;

    let closed = runtime_memory_tx::close_stage_request_epoch(
        db.pool(),
        &runtime_memory_tx::CloseStageRequestEpochRow {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: seeded.unit.id,
            stage_team_plan_id: seeded.plan.id,
            expected_dispatch_epoch: controller.plan.dispatch_epoch,
            expected_plan_row_version: controller.plan.row_version,
        },
    )
    .await
    .expect("close initial Controller request epoch");
    let bound = runtime_memory_tx::bind_stage_team_leader_final_submitter(
        db.pool(),
        &runtime_memory_tx::BindStageTeamLeaderFinalSubmitterRow {
            fence: stage_team_fence(&roots, &seeded, &controller),
            stage_team_plan_id: seeded.plan.id,
            leader_work_item_id: controller.work_item.id,
            expected_plan_row_version: closed.plan.row_version,
            expected_dispatch_epoch: closed.plan.dispatch_epoch,
            expected_manifest_hash: closed.barrier.manifest_hash.clone(),
        },
    )
    .await
    .expect("bind Controller before first Gate attempt");
    let (submission, controller_after_tool) = persist_stage_team_submission(
        &db,
        &roots,
        &seeded,
        &controller,
        "controller-gate-submit-1",
    )
    .await;
    let gate_decision_hash = format!(
        "sha256:{}",
        sha256_json(&serde_json::json!({"decision": "block", "round": 1}))
    );
    let gap_manifest = serde_json::json!({
        "gate_decision_hash": gate_decision_hash,
        "reasons": ["missing_dns_attestation"],
        "schema_version": 1,
    });
    let full_checkpoint = serde_json::json!({
        "controller": {
            "current_round": 3,
            "provider_chain_length": 17,
            "resume_after_gate_block": true,
        },
        "stage_team_gate_block": gap_manifest,
    });
    let reopen_input = runtime_memory_tx::ReopenStageTeamLeaderAfterGateBlockRow {
        request_id: "controller-gate-reopen-1".to_string(),
        fence: runtime_memory_tx::RuntimeMemoryTxFence {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: seeded.unit.id,
            worker_run_id: original_worker_id,
            lease_token: controller_after_tool
                .lease_token
                .expect("Controller lease remains after tool completion"),
            attempt_epoch: controller_after_tool.attempt_epoch,
            expected_checkpoint_version: controller_after_tool.checkpoint_version,
        },
        stage_team_plan_id: seeded.plan.id,
        leader_work_item_id: controller.work_item.id,
        deliverable_submission_id: submission.id,
        expected_dispatch_epoch: bound.plan.dispatch_epoch,
        expected_manifest_hash: closed.barrier.manifest_hash.clone(),
        gate_decision_hash: gate_decision_hash.clone(),
        gap_manifest_hash: format!("sha256:{}", sha256_json(&gap_manifest)),
        gap_manifest,
        checkpoint: full_checkpoint.clone(),
    };
    let reopened =
        runtime_memory_tx::reopen_stage_team_leader_after_gate_block(db.pool(), &reopen_input)
            .await
            .expect("reopen the same Controller after Gate BLOCK");
    assert!(!reopened.replayed);
    assert!(!reopened.fuel_exhausted);
    assert_eq!(reopened.plan.dispatch_epoch, bound.plan.dispatch_epoch + 1);
    assert_eq!(reopened.plan.requests_closed_at, None);
    assert_eq!(reopened.plan.final_submitter_worker_run_id, None);
    assert_eq!(reopened.unit.status, "running");
    assert_eq!(reopened.leader_work_item.status, "waiting_dependency");
    assert_eq!(reopened.leader_worker.id, original_worker_id);
    assert_eq!(reopened.leader_worker.status, "waiting_background");
    assert_eq!(
        reopened.leader_worker.message_chain_id,
        Some(original_chain_id)
    );
    assert_eq!(
        reopened.leader_worker.checkpoint.get("controller"),
        full_checkpoint.get("controller")
    );
    assert_eq!(
        reopened
            .leader_worker
            .checkpoint
            .get("stage_team_gate_block"),
        full_checkpoint.get("stage_team_gate_block")
    );
    assert_eq!(
        reopened
            .leader_worker
            .checkpoint
            .pointer("/_runtime_stage_team_gate_block/request_id")
            .and_then(serde_json::Value::as_str),
        Some("controller-gate-reopen-1")
    );
    assert_eq!(
        reopened
            .leader_worker
            .checkpoint
            .pointer("/_runtime_stage_team_gate_block/gap_manifest/reasons/0")
            .and_then(serde_json::Value::as_str),
        Some("missing_dns_attestation"),
        "the resumed Controller receives the server-authored durable gap, not hashes alone"
    );
    let workers_after_reopen: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM stage_worker_runs WHERE stage_run_unit_id=$1")
            .bind(seeded.unit.id)
            .fetch_one(db.pool())
            .await
            .expect("count Controller workers after reopen");
    assert_eq!(workers_after_reopen, 1, "no fresh Aggregator is created");
    assert!(
        runtime_memory_tx::reopen_stage_team_leader_after_gate_block(db.pool(), &reopen_input)
            .await
            .expect("exact reopen response-loss replay")
            .replayed
    );

    let resumed = runtime_memory_tx::claim_stage_team_leader(
        db.pool(),
        &runtime_memory_tx::ClaimStageTeamLeaderRow {
            claim: claim.clone(),
        },
    )
    .await
    .expect("resume Controller immediately after durable reopen")
    .expect("all old dependencies are already terminal");
    assert_eq!(resumed.worker.id, original_worker_id);
    assert_eq!(resumed.message_chain_id, original_chain_id);
    assert_eq!(
        resumed.worker.attempt_epoch,
        controller_after_tool.attempt_epoch + 1
    );
    assert_eq!(
        resumed.worker.checkpoint.get("controller"),
        full_checkpoint.get("controller")
    );
    assert_eq!(
        resumed.worker.checkpoint.get("stage_team_gate_block"),
        full_checkpoint.get("stage_team_gate_block")
    );

    let second_closed = runtime_memory_tx::close_stage_request_epoch(
        db.pool(),
        &runtime_memory_tx::CloseStageRequestEpochRow {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: seeded.unit.id,
            stage_team_plan_id: seeded.plan.id,
            expected_dispatch_epoch: resumed.plan.dispatch_epoch,
            expected_plan_row_version: resumed.plan.row_version,
        },
    )
    .await
    .expect("close second Controller request epoch");
    let second_bound = runtime_memory_tx::bind_stage_team_leader_final_submitter(
        db.pool(),
        &runtime_memory_tx::BindStageTeamLeaderFinalSubmitterRow {
            fence: stage_team_fence(&roots, &seeded, &resumed),
            stage_team_plan_id: seeded.plan.id,
            leader_work_item_id: resumed.work_item.id,
            expected_plan_row_version: second_closed.plan.row_version,
            expected_dispatch_epoch: second_closed.plan.dispatch_epoch,
            expected_manifest_hash: second_closed.barrier.manifest_hash.clone(),
        },
    )
    .await
    .expect("bind same Controller before second Gate attempt");
    let (second_submission, second_controller_after_tool) =
        persist_stage_team_submission(&db, &roots, &seeded, &resumed, "controller-gate-submit-2")
            .await;
    let second_gate_decision_hash = format!(
        "sha256:{}",
        sha256_json(&serde_json::json!({"decision": "block", "round": 2}))
    );
    let second_gap_manifest = serde_json::json!({
        "gate_decision_hash": second_gate_decision_hash,
        "reasons": ["repair_fuel_exhausted"],
        "schema_version": 1,
    });
    let second_request_id = format!(
        "stage-team-repair:{}:{}:{}",
        seeded.plan.id, second_bound.plan.dispatch_epoch, second_gate_decision_hash
    );
    let exhausted_input = runtime_memory_tx::ReopenStageTeamLeaderAfterGateBlockRow {
        request_id: second_request_id,
        fence: runtime_memory_tx::RuntimeMemoryTxFence {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: seeded.unit.id,
            worker_run_id: original_worker_id,
            lease_token: second_controller_after_tool
                .lease_token
                .expect("Controller lease remains after second tool"),
            attempt_epoch: second_controller_after_tool.attempt_epoch,
            expected_checkpoint_version: second_controller_after_tool.checkpoint_version,
        },
        stage_team_plan_id: seeded.plan.id,
        leader_work_item_id: resumed.work_item.id,
        deliverable_submission_id: second_submission.id,
        expected_dispatch_epoch: second_bound.plan.dispatch_epoch,
        expected_manifest_hash: second_closed.barrier.manifest_hash,
        gate_decision_hash: second_gate_decision_hash.clone(),
        gap_manifest_hash: format!("sha256:{}", sha256_json(&second_gap_manifest)),
        gap_manifest: second_gap_manifest,
        checkpoint: serde_json::json!([
            {"role": "system", "content": "frozen Company Controller prompt"},
            {"role": "assistant", "content": "durable provider-chain checkpoint"}
        ]),
    };
    let exhausted =
        runtime_memory_tx::reopen_stage_team_leader_after_gate_block(db.pool(), &exhausted_input)
            .await
            .expect("exhaust Controller repair fuel deterministically");
    assert!(exhausted.fuel_exhausted);
    assert_eq!(exhausted.unit.status, "gate_blocked");
    assert_eq!(exhausted.leader_worker.status, "gate_blocked");
    assert_eq!(exhausted.leader_work_item.status, "superseded");
    assert_eq!(
        exhausted.plan.dispatch_epoch,
        second_bound.plan.dispatch_epoch
    );
    let opened_gap = reopened.gap.as_ref().expect("first Gate gap is durable");
    let exhausted_gap = exhausted
        .gap
        .as_ref()
        .expect("fuel-exhausted Gate gap is durable");
    assert_ne!(opened_gap.id, exhausted_gap.id);
    assert_eq!(opened_gap.disposition, "opened");
    assert_eq!(exhausted_gap.disposition, "fuel_exhausted");
    assert_eq!(
        opened_gap.source_aggregator_worker_run_id,
        original_worker_id
    );
    assert_eq!(
        exhausted_gap.source_aggregator_worker_run_id,
        original_worker_id
    );
    assert_eq!(
        exhausted_gap.source_dispatch_epoch,
        opened_gap.source_dispatch_epoch + 1
    );
    assert!(
        runtime_memory_tx::reopen_stage_team_leader_after_gate_block(db.pool(), &exhausted_input,)
            .await
            .expect("exact fuel-exhausted response-loss replay")
            .replayed
    );

    // Upgrade compatibility: historical Company Controller fuel exhaustion
    // predates durable `fuel_exhausted` gaps.  Preserve the real checkpoint
    // shape (hash witnesses only) and remove only the newly-created row so the
    // successor-Turn path is exercised against that exact legacy boundary.
    let mut historical_tx = db.pool().begin().await.expect("begin legacy-gap fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *historical_tx)
        .await
        .expect("disable immutable trigger inside compatibility fixture");
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET checkpoint=jsonb_set(
                  checkpoint,
                  '{_runtime_stage_team_gate_block,schema_version}',
                  '1'::jsonb,
                  FALSE
              )
            WHERE id=$1"#,
    )
    .bind(original_worker_id)
    .execute(&mut *historical_tx)
    .await
    .expect("model the legacy v1 Gate checkpoint marker");
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET legacy_controller_gap_checkpoint_hash=
                  'sha256:' || attack_fact_delta_sha256_jsonb(checkpoint)
            WHERE id=$1"#,
    )
    .bind(original_worker_id)
    .execute(&mut *historical_tx)
    .await
    .expect("model the migration-time frozen legacy checkpoint witness");
    sqlx::query("DELETE FROM stage_team_unit_gaps WHERE id=$1")
        .bind(exhausted_gap.id)
        .execute(&mut *historical_tx)
        .await
        .expect("model the pre-gap fuel-exhausted checkpoint");
    historical_tx
        .commit()
        .await
        .expect("commit historical compatibility fixture");

    let unauthorized_plan_reopen = sqlx::query(
        r#"UPDATE stage_team_plans
              SET dispatch_epoch=dispatch_epoch+1,requests_closed_at=NULL,
                  final_submitter_worker_run_id=NULL,row_version=row_version+1,updated_at=NOW()
            WHERE id=$1"#,
    )
    .bind(seeded.plan.id)
    .execute(db.pool())
    .await;
    assert!(
        unauthorized_plan_reopen.is_err(),
        "a successor-Turn authority is required to reopen the terminal plan"
    );
    let unauthorized_item_reopen = sqlx::query(
        r#"UPDATE stage_work_items
              SET status='waiting_dependency',terminal_at=NULL,
                  row_version=row_version+1,updated_at=NOW()
            WHERE id=$1"#,
    )
    .bind(controller.work_item.id)
    .execute(db.pool())
    .await;
    assert!(
        unauthorized_item_reopen.is_err(),
        "a successor-Turn authority is required to resurrect the Controller item"
    );
    let unauthorized_worker_reopen = sqlx::query(
        r#"UPDATE stage_worker_runs
              SET status='waiting_background',checkpoint_version=checkpoint_version+1,
                  terminal_at=NULL,updated_at=NOW()
            WHERE id=$1"#,
    )
    .bind(original_worker_id)
    .execute(db.pool())
    .await;
    assert!(
        unauthorized_worker_reopen.is_err(),
        "a successor-Turn authority is required to resume the Controller worker"
    );

    let restarted =
        runtime_memory_tx::seed_stage_team_runtime(db.pool(), &stage_team_controller_seed(&roots))
            .await
            .expect("a separate continuation can replay the Gate-blocked Team seed");
    assert!(restarted[0].replayed);
    tasks::update_status(
        db.pool(),
        roots.operation_id,
        golish_db::models::TaskStatus::Running,
    )
    .await
    .expect("model the live task status before successor-Turn continuation");

    let source =
        tasks::exact_resumable_runtime_source(db.pool(), roots.operation_id, roots.session_id)
            .await
            .expect("select exact Gate-blocked V2 continuation source")
            .expect("fuel-exhausted Controller remains resumable");
    assert_eq!(source, runtime_memory_tx::RuntimeMemoryRecordSource::V2);
    let prior_turn = golish_db::repo::operation_turns::get_open(db.pool(), roots.operation_id)
        .await
        .expect("load prior open operation Turn")
        .expect("Gate-blocked operation retains one open Turn");
    let successor_turn_id = Uuid::new_v4();
    assert!(tasks::claim_exact_resumable_runtime_source(
        db.pool(),
        roots.operation_id,
        roots.session_id,
        source,
        prior_turn.id,
        successor_turn_id,
        "continue exact Controller after terminal Gate BLOCK",
    )
    .await
    .expect("claim successor operation Turn and reopen exact Controller"));

    let successor_plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(
        "SELECT * FROM stage_team_plans WHERE id=$1",
    )
    .bind(seeded.plan.id)
    .fetch_one(db.pool())
    .await
    .expect("load successor-Turn Team plan");
    let successor_unit = stage_run_units::get(db.pool(), seeded.unit.id)
        .await
        .expect("load successor-Turn Unit")
        .expect("successor-Turn Unit remains");
    let successor_item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
        "SELECT * FROM stage_work_items WHERE id=$1",
    )
    .bind(controller.work_item.id)
    .fetch_one(db.pool())
    .await
    .expect("load successor-Turn Controller item");
    let successor_worker = stage_worker_runs::get(db.pool(), original_worker_id)
        .await
        .expect("load successor-Turn Controller worker")
        .expect("successor-Turn Controller worker remains");
    assert_eq!(
        successor_plan.dispatch_epoch,
        second_bound.plan.dispatch_epoch + 1
    );
    assert_eq!(successor_plan.requests_closed_at, None);
    assert_eq!(successor_plan.final_submitter_worker_run_id, None);
    assert_eq!(successor_unit.status, "running");
    assert_eq!(successor_item.status, "waiting_dependency");
    assert_eq!(successor_item.terminal_at, None);
    assert_eq!(successor_worker.id, original_worker_id);
    assert_eq!(successor_worker.status, "waiting_background");
    assert_eq!(successor_worker.message_chain_id, Some(original_chain_id));
    assert_eq!(successor_worker.terminal_at, None);
    assert_eq!(
        successor_worker.checkpoint_version,
        exhausted.leader_worker.checkpoint_version + 1
    );
    let successor_turn_id_text = successor_turn_id.to_string();
    assert_eq!(
        successor_worker
            .checkpoint
            .pointer("/_runtime_stage_team_turn_resume/resume_turn_id")
            .and_then(serde_json::Value::as_str),
        Some(successor_turn_id_text.as_str())
    );
    let authority =
        sqlx::query_as::<_, (Uuid, String, Uuid, Uuid, i64, i64, Uuid, Uuid, Option<Uuid>)>(
            r#"SELECT id,status,prior_turn_id,resume_turn_id,
                  source_dispatch_epoch,resume_dispatch_epoch,
                  leader_worker_run_id,message_chain_id,source_gap_id
             FROM stage_team_controller_turn_resumes
            WHERE team_plan_id=$1"#,
        )
        .bind(seeded.plan.id)
        .fetch_one(db.pool())
        .await
        .expect("load applied successor-Turn Controller authority");
    assert_eq!(authority.1, "applied");
    assert_eq!(authority.2, prior_turn.id);
    assert_eq!(authority.3, successor_turn_id);
    assert_eq!(authority.4, exhausted.plan.dispatch_epoch);
    assert_eq!(authority.5, successor_plan.dispatch_epoch);
    assert_eq!(authority.6, original_worker_id);
    assert_eq!(authority.7, original_chain_id);
    assert_eq!(
        authority.8, None,
        "legacy recovery records hashes without inventing a historical gap"
    );

    let resumed_after_turn = runtime_memory_tx::claim_stage_team_leader(
        db.pool(),
        &runtime_memory_tx::ClaimStageTeamLeaderRow { claim },
    )
    .await
    .expect("claim exact Controller after successor-Turn reopen")
    .expect("successor Turn made the Controller runnable");
    assert_eq!(resumed_after_turn.worker.id, original_worker_id);
    assert_eq!(resumed_after_turn.message_chain_id, original_chain_id);
    assert_eq!(
        resumed_after_turn.worker.attempt_epoch,
        exhausted.leader_worker.attempt_epoch + 1
    );

    db.stop().await;
}

fn stage_team_claim_input(
    roots: &RuntimeRoots,
    seeded: &runtime_memory_tx::SeededStageTeamRuntimeRow,
    owner: &str,
) -> runtime_memory_tx::ClaimStageWorkItemRow {
    runtime_memory_tx::ClaimStageWorkItemRow {
        operation_id: roots.operation_id,
        stage_execution_id: roots.stage_execution_id,
        stage_run_unit_id: seeded.unit.id,
        stage_team_plan_id: seeded.plan.id,
        exact_work_item_id: None,
        lease_owner: owner.to_string(),
        lease_seconds: 60,
        session_id: roots.session_id,
        subtask_id: None,
        agent: AgentType::Pentester,
        model: None,
        provider: None,
        parent_chain_id: None,
        initial_chain: serde_json::json!([]),
        initial_checkpoint: serde_json::json!({"turn": 0}),
    }
}

fn stage_team_fence(
    roots: &RuntimeRoots,
    seeded: &runtime_memory_tx::SeededStageTeamRuntimeRow,
    claimed: &runtime_memory_tx::ClaimedStageWorkItemRow,
) -> runtime_memory_tx::RuntimeMemoryTxFence {
    runtime_memory_tx::RuntimeMemoryTxFence {
        operation_id: roots.operation_id,
        stage_execution_id: roots.stage_execution_id,
        stage_run_unit_id: seeded.unit.id,
        worker_run_id: claimed.worker.id,
        lease_token: claimed.worker.lease_token.expect("claimed Team lease"),
        attempt_epoch: claimed.worker.attempt_epoch,
        expected_checkpoint_version: claimed.worker.checkpoint_version,
    }
}

async fn persist_stage_team_submission(
    db: &GolishDb,
    roots: &RuntimeRoots,
    seeded: &runtime_memory_tx::SeededStageTeamRuntimeRow,
    controller: &runtime_memory_tx::ClaimedStageWorkItemRow,
    request_id: &str,
) -> (
    stage_deliverable_submissions::StageDeliverableSubmissionRow,
    stage_worker_runs::StageWorkerRunRow,
) {
    let fence = stage_team_fence(roots, seeded, controller);
    let tool_call_id = tool_calls::record_tracked_start(
        db.pool(),
        request_id,
        roots.session_id,
        Some(roots.operation_id),
        None,
        "submit_stage_deliverable",
        &serde_json::json!({"stage_id": &seeded.unit.stage_kind}),
        Some(&tool_calls::RuntimeToolIdentity {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: Some(seeded.unit.id),
            worker_run_id: Some(controller.worker.id),
            organization_id: Some(roots.organization_id),
            attempt_epoch: Some(controller.worker.attempt_epoch),
            lease_token: controller.worker.lease_token,
        }),
    )
    .await
    .expect("record Controller submission tool");
    runtime_memory_tx::begin_worker_tool(db.pool(), &fence, tool_call_id)
        .await
        .expect("fence Controller submission tool");
    let payload = serde_json::json!({
        "stage_id": &seeded.unit.stage_kind,
        "stage_run_id": roots.stage_execution_id,
        "claims": [],
    });
    let canonical_payload_json = canonical_json(&payload);
    let submission = stage_deliverable_submissions::insert(
        db.pool(),
        &stage_deliverable_submissions::NewStageDeliverableSubmission {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: Some(seeded.unit.id),
            worker_run_id: Some(controller.worker.id),
            organization_id: Some(roots.organization_id),
            tool_call_record_id: tool_call_id,
            tool_request_id: request_id.to_string(),
            stage_kind: seeded.unit.stage_kind.clone(),
            attempt_epoch: Some(controller.worker.attempt_epoch),
            lease_token: controller.worker.lease_token,
            payload_sha256: Sha256::digest(canonical_payload_json.as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            canonical_payload_json,
        },
    )
    .await
    .expect("persist Controller deliverable submission");
    let worker = runtime_memory_tx::finish_worker_tool(db.pool(), &fence, tool_call_id)
        .await
        .expect("clear Controller submission fence");
    tool_calls::record_tracked_finish(
        db.pool(),
        tool_call_id,
        roots.session_id,
        "finished",
        &serde_json::json!({
            "deliverable_submission_id": submission.id,
            "status": "accepted",
        })
        .to_string(),
        1,
    )
    .await
    .expect("finish Controller submission tool");
    (submission, worker)
}

#[tokio::test]
#[serial]
async fn runtime_tool_start_uses_unit_before_stage_lock_order() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    let seeded = runtime_memory_tx::seed_stage_team_runtime(
        db.pool(),
        &stage_team_lifecycle_seed(&roots, 2),
    )
    .await
    .expect("seed lock-order Team")
    .remove(0);
    let claimed = runtime_memory_tx::claim_stage_work_item(
        db.pool(),
        &stage_team_claim_input(&roots, &seeded, "tool-start-lock-order"),
    )
    .await
    .expect("claim lock-order worker")
    .expect("lock-order worker exists");

    let mut heartbeat_order_tx = db.pool().begin().await.expect("begin lock-order tx");
    sqlx::query("SELECT id FROM stage_run_units WHERE id=$1 FOR UPDATE")
        .bind(seeded.unit.id)
        .fetch_one(&mut *heartbeat_order_tx)
        .await
        .expect("hold unit lock before tool start");

    let pool = db.pool().clone();
    let runtime = tool_calls::RuntimeToolIdentity {
        operation_id: roots.operation_id,
        stage_execution_id: roots.stage_execution_id,
        stage_run_unit_id: Some(seeded.unit.id),
        worker_run_id: Some(claimed.worker.id),
        organization_id: Some(roots.organization_id),
        attempt_epoch: Some(claimed.worker.attempt_epoch),
        lease_token: claimed.worker.lease_token,
    };
    let session_id = roots.session_id;
    let operation_id = roots.operation_id;
    let start = tokio::spawn(async move {
        tool_calls::record_tracked_start(
            &pool,
            "runtime-tool-lock-order",
            session_id,
            Some(operation_id),
            None,
            "query_target_data",
            &serde_json::json!({}),
            Some(&runtime),
        )
        .await
    });

    // The runtime-aware start must be waiting on the already-held Unit before
    // it can touch StageExecution/Worker FK rows. A legacy direct INSERT took
    // the StageExecution lock first and deadlocked at this exact seam.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    sqlx::query("SELECT id FROM stage_runs WHERE id=$1 FOR UPDATE")
        .bind(roots.stage_execution_id)
        .fetch_one(&mut *heartbeat_order_tx)
        .await
        .expect("unit owner can lock stage without a reverse-lock deadlock");
    heartbeat_order_tx
        .commit()
        .await
        .expect("release ordered runtime locks");

    let tool_call_id = tokio::time::timeout(std::time::Duration::from_secs(5), start)
        .await
        .expect("runtime tool start did not stall")
        .expect("runtime tool start task joined")
        .expect("runtime tool start committed");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM tool_calls WHERE id=$1 AND status='running'"
        )
        .bind(tool_call_id)
        .fetch_one(db.pool())
        .await
        .expect("count exact running tool"),
        1
    );
    tool_calls::record_tracked_finish(
        db.pool(),
        tool_call_id,
        roots.session_id,
        "finished",
        "{}",
        1,
    )
    .await
    .expect("finish lock-order tool");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn startup_reaper_recognizes_team_workers_by_work_item_identity() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    let seeded = runtime_memory_tx::seed_stage_team_runtime(
        db.pool(),
        &stage_team_lifecycle_seed(&roots, 2),
    )
    .await
    .expect("seed lifecycle Team")
    .remove(0);
    let producer = runtime_memory_tx::claim_stage_work_item(
        db.pool(),
        &stage_team_claim_input(&roots, &seeded, "startup-producer"),
    )
    .await
    .expect("claim producer")
    .expect("producer WorkItem exists");
    let helper = runtime_memory_tx::claim_stage_work_item(
        db.pool(),
        &stage_team_claim_input(&roots, &seeded, "startup-helper"),
    )
    .await
    .expect("claim helper")
    .expect("helper WorkItem exists");
    assert_eq!(producer.work_item.role, "producer");
    assert_eq!(helper.work_item.role, "helper");

    let helper_fence = stage_team_fence(&roots, &seeded, &helper);
    let active_tool_id = tool_calls::record_tracked_start(
        db.pool(),
        "stage-team-startup-active-tool",
        roots.session_id,
        Some(roots.operation_id),
        None,
        "query_target_data",
        &serde_json::json!({}),
        Some(&tool_calls::RuntimeToolIdentity {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: Some(seeded.unit.id),
            worker_run_id: Some(helper.worker.id),
            organization_id: Some(roots.organization_id),
            attempt_epoch: Some(helper.worker.attempt_epoch),
            lease_token: helper.worker.lease_token,
        }),
    )
    .await
    .expect("record exact Team active tool");
    runtime_memory_tx::begin_worker_tool(db.pool(), &helper_fence, active_tool_id)
        .await
        .expect("fence Team active tool");
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET lease_acquired_at=NOW()-INTERVAL '2 hours',
                  lease_expires_at=NOW()-INTERVAL '1 hour',
                  heartbeat_at=NOW()-INTERVAL '1 hour'
            WHERE id = ANY($1)"#,
    )
    .bind(vec![producer.worker.id, helper.worker.id])
    .execute(db.pool())
    .await
    .expect("expire producer and helper leases");
    sqlx::query(
        "UPDATE tasks SET status='running',updated_at=NOW()-INTERVAL '7 hours' WHERE id=$1",
    )
    .bind(roots.operation_id)
    .execute(db.pool())
    .await
    .expect("age Team task for startup reaper");

    let reaped = tasks::startup_reap_abandoned(db.pool(), chrono::Duration::zero())
        .await
        .expect("reap exact Team workers");
    assert_eq!(reaped.workers_requeued, 1);
    assert_eq!(reaped.workers_recovery_required, 1);
    let producer_worker = stage_worker_runs::get(db.pool(), producer.worker.id)
        .await
        .expect("load producer Worker")
        .expect("producer Worker exists");
    let helper_worker = stage_worker_runs::get(db.pool(), helper.worker.id)
        .await
        .expect("load helper Worker")
        .expect("helper Worker exists");
    let items = stage_teams::list_work_items_with_executor(db.pool(), seeded.plan.id)
        .await
        .expect("load Team WorkItems after startup");
    let producer_item = items
        .iter()
        .find(|item| item.id == producer.work_item.id)
        .expect("producer WorkItem remains");
    let helper_item = items
        .iter()
        .find(|item| item.id == helper.work_item.id)
        .expect("helper WorkItem remains");
    assert_eq!(producer_worker.status, "queued");
    assert_eq!(
        producer_worker.message_chain_id,
        Some(producer.message_chain_id)
    );
    assert_eq!(producer_item.status, "queued");
    assert_eq!(helper_worker.status, "recovery_required");
    assert_eq!(helper_worker.active_tool_call_id, Some(active_tool_id));
    assert_eq!(helper_item.status, "recovery_required");
    assert_eq!(
        tasks::get(db.pool(), roots.operation_id)
            .await
            .expect("load startup-reaped Team task")
            .expect("Team task remains")
            .status,
        golish_db::models::TaskStatus::Waiting,
        "valid Team recovery truth must remain resumable"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn expired_clean_child_resumes_same_worker_and_message_chain() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    let seeded = runtime_memory_tx::seed_stage_team_runtime(
        db.pool(),
        &stage_team_lifecycle_seed(&roots, 2),
    )
    .await
    .expect("seed exact-chain recovery Team")
    .remove(0);
    let first = runtime_memory_tx::claim_stage_work_item(
        db.pool(),
        &stage_team_claim_input(&roots, &seeded, "exact-chain-child-first-turn"),
    )
    .await
    .expect("claim child first turn")
    .expect("child WorkItem exists");
    let first_chain_id = first.message_chain_id;
    let first_attempt_epoch = first.worker.attempt_epoch;
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET lease_acquired_at=NOW()-INTERVAL '2 hours',
                  lease_expires_at=NOW()-INTERVAL '1 hour',
                  heartbeat_at=NOW()-INTERVAL '1 hour'
            WHERE id=$1 AND active_tool_call_id IS NULL"#,
    )
    .bind(first.worker.id)
    .execute(db.pool())
    .await
    .expect("expire clean child lease");

    let resumed = runtime_memory_tx::claim_stage_work_item(
        db.pool(),
        &stage_team_claim_input(&roots, &seeded, "exact-chain-child-second-turn"),
    )
    .await
    .expect("resume clean child")
    .expect("same child WorkItem is claimable");

    assert_eq!(resumed.work_item.id, first.work_item.id);
    assert_eq!(
        resumed.worker.id, first.worker.id,
        "a lease retry is a new Turn on the same logical child, not a replacement child"
    );
    assert_eq!(resumed.message_chain_id, first_chain_id);
    assert_eq!(
        resumed.worker.attempt_epoch,
        first_attempt_epoch + 1,
        "the same WorkerRun advances its Turn fence"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM stage_worker_runs WHERE work_item_id=$1"
        )
        .bind(first.work_item.id)
        .fetch_one(db.pool())
        .await
        .expect("count logical child WorkerRuns"),
        1,
        "resume must not insert a fresh WorkerRun"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn queued_same_worker_resume_bypasses_distinct_worker_lifetime_cap() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    let mut seed = stage_team_lifecycle_seed(&roots, 2);
    seed.plan.max_workers_total = 1;
    seed.plan.max_workers_active = 1;
    let seeded = runtime_memory_tx::seed_stage_team_runtime(db.pool(), &seed)
        .await
        .expect("seed lifetime-capped continuation Team")
        .remove(0);
    let claim = stage_team_claim_input(&roots, &seeded, "lifetime-capped-continuation");
    let first = runtime_memory_tx::claim_stage_work_item(db.pool(), &claim)
        .await
        .expect("claim first Turn")
        .expect("first WorkItem is runnable");
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET lease_acquired_at=NOW()-INTERVAL '2 hours',
                  lease_expires_at=NOW()-INTERVAL '1 hour',
                  heartbeat_at=NOW()-INTERVAL '1 hour'
            WHERE id=$1 AND active_tool_call_id IS NULL"#,
    )
    .bind(first.worker.id)
    .execute(db.pool())
    .await
    .expect("expire lifetime-capped Worker lease");

    let resumed = runtime_memory_tx::claim_stage_work_item(db.pool(), &claim)
        .await
        .expect("same Worker continuation does not spend distinct-worker lifetime budget")
        .expect("queued same Worker remains claimable at the frozen caps");

    assert_eq!(resumed.work_item.id, first.work_item.id);
    assert_eq!(resumed.worker.id, first.worker.id);
    assert_eq!(resumed.message_chain_id, first.message_chain_id);
    assert_eq!(resumed.worker.attempt_epoch, first.worker.attempt_epoch + 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM stage_worker_runs WHERE stage_run_unit_id=$1"
        )
        .bind(seeded.unit.id)
        .fetch_one(db.pool())
        .await
        .expect("count distinct lifetime Workers"),
        1,
        "continuation must not insert a distinct WorkerRun"
    );
    assert!(
        runtime_memory_tx::claim_stage_work_item(db.pool(), &claim)
            .await
            .expect("fresh-worker active-cap check remains deterministic")
            .is_none(),
        "the active cap must still block a fresh sibling while the resumed Worker is live"
    );
    let evidence_id = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO audit_log (
               action, category, details, project_path, source, audit_role,
               detail, run_id, created_at
           ) VALUES (
               'lifetime-capped worker evidence','harness','fresh continuation evidence',
               '/tmp/runtime-worker','harness','evidence',$1,$2,NOW()
           ) RETURNING id"#,
    )
    .bind(serde_json::json!({"organization_id": roots.organization_id}))
    .bind(roots.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("insert lifetime-capped continuation evidence");
    let mut completion = stage_teams::CompleteStageWorkerRow {
        fence: stage_team_fence(&roots, &seeded, &resumed),
        team_plan_id: seeded.plan.id,
        work_item_id: resumed.work_item.id,
        expected_work_item_row_version: resumed.work_item.row_version,
        output_schema: resumed.work_item.output_schema.clone(),
        business_disposition: "found".to_string(),
        canonical_output: serde_json::json!({"facts": []}),
        canonical_fact_refs: serde_json::json!([]),
        evidence_ids: vec![evidence_id],
        checked_empty_cells: serde_json::json!([]),
        blocker_codes: Vec::new(),
        output_hash: String::new(),
        terminal_checkpoint: serde_json::json!({"done": true}),
        evidence_watermark: Some(evidence_id),
    };
    refresh_stage_worker_output_hash(&mut completion);
    stage_teams::complete_stage_worker(db.pool(), completion)
        .await
        .expect("complete resumed Worker before testing fresh lifetime cap");
    let fresh_worker_error = runtime_memory_tx::claim_stage_work_item(db.pool(), &claim)
        .await
        .expect_err("a distinct sibling Worker must still obey the lifetime cap");
    assert!(matches!(
        fresh_worker_error,
        runtime_memory_tx::RuntimeMemoryStoreError::Conflict {
            code: "stage_team_worker_lifetime_budget_exhausted"
        }
    ));

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn startup_reaper_exhausts_clean_team_worker_when_attempt_budget_is_spent() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    let seeded = runtime_memory_tx::seed_stage_team_runtime(
        db.pool(),
        &stage_team_lifecycle_seed(&roots, 1),
    )
    .await
    .expect("seed startup exhaustion Team")
    .remove(0);
    let producer = runtime_memory_tx::claim_stage_work_item(
        db.pool(),
        &stage_team_claim_input(&roots, &seeded, "startup-exhausted-producer"),
    )
    .await
    .expect("claim startup exhaustion producer")
    .expect("producer WorkItem exists");
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET lease_acquired_at=NOW()-INTERVAL '2 hours',
                  lease_expires_at=NOW()-INTERVAL '1 hour',
                  heartbeat_at=NOW()-INTERVAL '1 hour'
            WHERE id=$1"#,
    )
    .bind(producer.worker.id)
    .execute(db.pool())
    .await
    .expect("expire final producer attempt");
    sqlx::query(
        "UPDATE tasks SET status='running',updated_at=NOW()-INTERVAL '7 hours' WHERE id=$1",
    )
    .bind(roots.operation_id)
    .execute(db.pool())
    .await
    .expect("age final-attempt Team task");

    let reaped = tasks::startup_reap_abandoned(db.pool(), chrono::Duration::zero())
        .await
        .expect("exhaust final Team attempt on startup");
    assert_eq!(reaped.workers_requeued, 0);
    assert_eq!(reaped.workers_recovery_required, 0);
    let worker = stage_worker_runs::get(db.pool(), producer.worker.id)
        .await
        .expect("load exhausted startup Worker")
        .expect("startup Worker remains");
    assert_eq!(worker.status, "failed");
    assert_eq!(worker.active_tool_call_id, None);
    assert_eq!(
        worker.checkpoint["stage_team_execution_failure"]["code"],
        "stage_team_worker_lease_expired"
    );
    let items = stage_teams::list_work_items_with_executor(db.pool(), seeded.plan.id)
        .await
        .expect("load exhausted startup WorkItem");
    assert_eq!(
        items
            .iter()
            .find(|item| item.id == producer.work_item.id)
            .expect("producer WorkItem remains")
            .status,
        "exhausted"
    );
    let outputs = stage_teams::list_outputs_with_executor(db.pool(), seeded.plan.id)
        .await
        .expect("load startup exhaustion output");
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].work_item_id, producer.work_item.id);
    assert_eq!(outputs[0].business_disposition, "blocked");
    assert_eq!(
        outputs[0].canonical_output["failure_code"],
        "stage_team_worker_lease_expired"
    );
    assert_eq!(
        outputs[0].blocker_codes,
        vec!["STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED"]
    );

    let replay = tasks::startup_reap_abandoned(db.pool(), chrono::Duration::zero())
        .await
        .expect("startup exhaustion replay is a no-op");
    assert_eq!(replay.workers_requeued, 0);
    assert_eq!(replay.workers_recovery_required, 0);
    assert_eq!(
        stage_teams::list_outputs_with_executor(db.pool(), seeded.plan.id)
            .await
            .expect("reload one startup exhaustion output")
            .len(),
        1
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn stage_team_active_tool_recovery_requires_exact_operator_cas() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    let seeded = runtime_memory_tx::seed_stage_team_runtime(
        db.pool(),
        &stage_team_lifecycle_seed(&roots, 2),
    )
    .await
    .expect("seed active-tool recovery Team")
    .remove(0);
    let claimed = runtime_memory_tx::claim_stage_work_item(
        db.pool(),
        &stage_team_claim_input(&roots, &seeded, "active-tool-recovery-producer"),
    )
    .await
    .expect("claim active-tool recovery producer")
    .expect("producer WorkItem exists");
    let fence = stage_team_fence(&roots, &seeded, &claimed);
    let active_tool_id = tool_calls::record_tracked_start(
        db.pool(),
        "stage-team-active-tool-unknown-outcome",
        roots.session_id,
        Some(roots.operation_id),
        None,
        "query_target_data",
        &serde_json::json!({}),
        Some(&tool_calls::RuntimeToolIdentity {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: Some(seeded.unit.id),
            worker_run_id: Some(claimed.worker.id),
            organization_id: Some(roots.organization_id),
            attempt_epoch: Some(claimed.worker.attempt_epoch),
            lease_token: claimed.worker.lease_token,
        }),
    )
    .await
    .expect("record exact active tool");
    runtime_memory_tx::begin_worker_tool(db.pool(), &fence, active_tool_id)
        .await
        .expect("bind exact active tool");
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET lease_acquired_at=NOW()-INTERVAL '2 hours',
                  lease_expires_at=NOW()-INTERVAL '1 hour',
                  heartbeat_at=NOW()-INTERVAL '1 hour'
            WHERE id=$1"#,
    )
    .bind(claimed.worker.id)
    .execute(db.pool())
    .await
    .expect("expire active-tool worker");
    sqlx::query(
        "UPDATE tasks SET status='running',updated_at=NOW()-INTERVAL '7 hours' WHERE id=$1",
    )
    .bind(roots.operation_id)
    .execute(db.pool())
    .await
    .expect("age active-tool Team task");
    let reaped = tasks::startup_reap_abandoned(db.pool(), chrono::Duration::zero())
        .await
        .expect("park active-tool Worker for recovery");
    assert_eq!(reaped.workers_recovery_required, 1);
    let parked_worker = stage_worker_runs::get(db.pool(), claimed.worker.id)
        .await
        .expect("load parked Worker")
        .expect("parked Worker remains");
    let parked_item = stage_teams::list_work_items_with_executor(db.pool(), seeded.plan.id)
        .await
        .expect("load parked WorkItem")
        .into_iter()
        .find(|item| item.id == claimed.work_item.id)
        .expect("parked WorkItem remains");
    assert_eq!(parked_worker.status, "recovery_required");
    assert_eq!(parked_item.status, "recovery_required");

    // The immutable decision table is also safe against a bypass of the Rust
    // repository: a valid plan/item/worker/tool tuple cannot be rebound to a
    // sibling (or invented) frozen scope by direct SQL.
    let cross_scope = sqlx::query(
        r#"INSERT INTO stage_team_recovery_decisions(
               id,request_id,team_plan_id,work_item_id,worker_run_id,tool_call_record_id,
               operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
               organization_id,expected_work_item_row_version,expected_checkpoint_version,
               expected_attempt_epoch,resolution_kind,resolution_payload,resolution_hash,resolved_by
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,
               'mark_blocked_outcome_unknown',$15,$16,$17
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind("cross-scope-recovery-must-fail")
    .bind(seeded.plan.id)
    .bind(parked_item.id)
    .bind(parked_worker.id)
    .bind(active_tool_id)
    .bind(roots.operation_id)
    .bind(roots.stage_execution_id)
    .bind(seeded.unit.id)
    .bind(Uuid::new_v4())
    .bind(roots.organization_id)
    .bind(parked_item.row_version)
    .bind(parked_worker.checkpoint_version)
    .bind(parked_worker.attempt_epoch)
    .bind(serde_json::json!({"kind": "mark_blocked_outcome_unknown"}))
    .bind(format!("sha256:{}", "a".repeat(64)))
    .bind("local-operator")
    .execute(db.pool())
    .await;
    assert!(matches!(
        cross_scope,
        Err(sqlx::Error::Database(error)) if error.code().as_deref() == Some("23503")
    ));

    let resolve = stage_teams::ResolveStageTeamRecoveryRow {
        request_id: "operator-resolution-1".to_string(),
        operation_id: roots.operation_id,
        stage_execution_id: roots.stage_execution_id,
        stage_run_unit_id: seeded.unit.id,
        scope_snapshot_id: seeded.plan.scope_snapshot_id,
        team_plan_id: seeded.plan.id,
        work_item_id: parked_item.id,
        worker_run_id: parked_worker.id,
        tool_call_record_id: active_tool_id,
        expected_work_item_row_version: parked_item.row_version,
        expected_checkpoint_version: parked_worker.checkpoint_version,
        expected_attempt_epoch: parked_worker.attempt_epoch,
        resolved_by: "local-operator".to_string(),
    };
    let resolved = stage_teams::resolve_stage_team_recovery(db.pool(), &resolve)
        .await
        .expect("resolve unknown external outcome without replay");
    assert!(!resolved.replayed);
    assert_eq!(resolved.worker.status, "failed");
    assert_eq!(resolved.worker.active_tool_call_id, None);
    assert_eq!(resolved.work_item.status, "exhausted");
    assert_eq!(resolved.output.business_disposition, "blocked");
    assert_eq!(
        resolved.output.blocker_codes,
        vec!["STAGE_TEAM_ACTIVE_TOOL_RECOVERY_BLOCKED"]
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status::text FROM tool_calls WHERE id=$1")
            .bind(active_tool_id)
            .fetch_one(db.pool())
            .await
            .expect("load locally resolved tool status"),
        "failed"
    );
    let replayed = stage_teams::resolve_stage_team_recovery(db.pool(), &resolve)
        .await
        .expect("exact recovery response-loss replay");
    assert!(replayed.replayed);
    assert_eq!(replayed.decision.id, resolved.decision.id);
    assert_eq!(replayed.output.id, resolved.output.id);
    let mut drifted = resolve;
    drifted.request_id = "operator-resolution-drift".to_string();
    assert!(matches!(
        stage_teams::resolve_stage_team_recovery(db.pool(), &drifted).await,
        Err(runtime_memory_tx::RuntimeMemoryStoreError::Conflict {
            code: "stage_team_recovery_resolution_replay_mismatch"
        })
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM stage_team_recovery_decisions WHERE worker_run_id=$1"
        )
        .bind(claimed.worker.id)
        .fetch_one(db.pool())
        .await
        .expect("count one immutable recovery decision"),
        1
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn stage_team_reclaims_terminal_failed_local_provider_list_fence() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    let seeded = runtime_memory_tx::seed_stage_team_runtime(
        db.pool(),
        &stage_team_lifecycle_seed(&roots, 2),
    )
    .await
    .expect("seed local-tool recovery Team")
    .remove(0);
    let claimed = runtime_memory_tx::claim_stage_work_item(
        db.pool(),
        &stage_team_claim_input(&roots, &seeded, "local-tool-recovery-producer"),
    )
    .await
    .expect("claim local-tool recovery producer")
    .expect("producer WorkItem exists");
    let fence = stage_team_fence(&roots, &seeded, &claimed);
    let active_tool_id = tool_calls::record_tracked_start(
        db.pool(),
        "stage-team-local-provider-list",
        roots.session_id,
        Some(roots.operation_id),
        None,
        "recon_list_providers",
        &serde_json::json!({}),
        Some(&tool_calls::RuntimeToolIdentity {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: Some(seeded.unit.id),
            worker_run_id: Some(claimed.worker.id),
            organization_id: Some(roots.organization_id),
            attempt_epoch: Some(claimed.worker.attempt_epoch),
            lease_token: claimed.worker.lease_token,
        }),
    )
    .await
    .expect("record exact local provider-list tool");
    runtime_memory_tx::begin_worker_tool(db.pool(), &fence, active_tool_id)
        .await
        .expect("bind local provider-list tool");
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET status='recovery_required',
                  lease_acquired_at=NOW()-INTERVAL '2 hours',
                  lease_expires_at=NOW()-INTERVAL '1 hour',
                  heartbeat_at=NOW()-INTERVAL '1 hour'
            WHERE id=$1"#,
    )
    .bind(claimed.worker.id)
    .execute(db.pool())
    .await
    .expect("park local-tool Worker");
    sqlx::query(
        "UPDATE stage_work_items SET status='recovery_required',row_version=row_version+1 WHERE id=$1",
    )
    .bind(claimed.work_item.id)
    .execute(db.pool())
    .await
    .expect("park local-tool WorkItem");
    tool_calls::record_tracked_finish(
        db.pool(),
        active_tool_id,
        roots.session_id,
        "failed",
        "worker tool result rejected by lease fence: runtime memory storage failure: deadlock detected",
        1,
    )
    .await
    .expect("record terminal local lifecycle failure");

    let resumed = runtime_memory_tx::claim_stage_work_item(
        db.pool(),
        &stage_team_claim_input(&roots, &seeded, "local-tool-recovery-continuation"),
    )
    .await
    .expect("reclaim retry-safe terminal local tool")
    .expect("same producer Thread remains claimable");
    assert_eq!(resumed.work_item.id, claimed.work_item.id);
    assert_eq!(resumed.worker.id, claimed.worker.id);
    assert_eq!(resumed.message_chain_id, claimed.message_chain_id);
    assert_eq!(
        resumed.worker.attempt_epoch,
        claimed.worker.attempt_epoch + 1
    );
    assert_eq!(resumed.worker.status, "running");
    let same_worker = stage_worker_runs::get(db.pool(), claimed.worker.id)
        .await
        .expect("load resumed local-tool Worker")
        .expect("same local-tool Worker remains");
    assert_eq!(same_worker.status, "running");
    assert_eq!(same_worker.active_tool_call_id, None);

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn stage_team_reconciles_interrupted_eas_service_fingerprint_on_same_worker_chain() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    let seeded = runtime_memory_tx::seed_stage_team_runtime(
        db.pool(),
        &stage_team_lifecycle_seed(&roots, 2),
    )
    .await
    .expect("seed interrupted crawler Team")
    .remove(0);
    let claimed = runtime_memory_tx::claim_stage_work_item(
        db.pool(),
        &stage_team_claim_input(&roots, &seeded, "interrupted-crawler-first-turn"),
    )
    .await
    .expect("claim crawler producer")
    .expect("crawler producer WorkItem exists");
    let fence = stage_team_fence(&roots, &seeded, &claimed);
    let active_tool_id = tool_calls::record_tracked_start(
        db.pool(),
        "stage-team-interrupted-eas-service-fingerprint",
        roots.session_id,
        Some(roots.operation_id),
        None,
        "eas_fingerprint_services",
        &serde_json::json!({
            "targets": [{
                "target_id": Uuid::new_v4(),
                "target_ip": "192.0.2.10",
                "ports": [443]
            }]
        }),
        Some(&tool_calls::RuntimeToolIdentity {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: Some(seeded.unit.id),
            worker_run_id: Some(claimed.worker.id),
            organization_id: Some(roots.organization_id),
            attempt_epoch: Some(claimed.worker.attempt_epoch),
            lease_token: claimed.worker.lease_token,
        }),
    )
    .await
    .expect("record exact EAS service fingerprint tool");
    runtime_memory_tx::begin_worker_tool(db.pool(), &fence, active_tool_id)
        .await
        .expect("bind EAS service fingerprint tool");
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET lease_acquired_at=NOW()-INTERVAL '2 hours',
                  lease_expires_at=NOW()-INTERVAL '1 hour',
                  heartbeat_at=NOW()-INTERVAL '1 hour'
            WHERE id=$1"#,
    )
    .bind(claimed.worker.id)
    .execute(db.pool())
    .await
    .expect("expire interrupted EAS service fingerprint lease");
    sqlx::query(
        "UPDATE tasks SET status='running',updated_at=NOW()-INTERVAL '7 hours' WHERE id=$1",
    )
    .bind(roots.operation_id)
    .execute(db.pool())
    .await
    .expect("age interrupted EAS service fingerprint task");

    let startup = tasks::startup_reap_abandoned(db.pool(), chrono::Duration::zero())
        .await
        .expect("park interrupted EAS service fingerprint during startup reconciliation");
    assert_eq!(startup.workers_recovery_required, 1);

    let resumed = runtime_memory_tx::claim_stage_work_item(
        db.pool(),
        &stage_team_claim_input(&roots, &seeded, "interrupted-crawler-second-turn"),
    )
    .await
    .expect("reconcile interrupted EAS service fingerprint")
    .expect("same EAS producer is claimable");
    assert_eq!(resumed.work_item.id, claimed.work_item.id);
    assert_eq!(resumed.worker.id, claimed.worker.id);
    assert_eq!(resumed.message_chain_id, claimed.message_chain_id);
    assert_eq!(
        resumed.worker.attempt_epoch,
        claimed.worker.attempt_epoch + 1
    );
    assert_eq!(resumed.worker.status, "running");
    assert_eq!(resumed.worker.active_tool_call_id, None);
    assert_eq!(
        resumed.worker.checkpoint["stage_team_interrupted_tool_recovery"]["kind"],
        "resume_after_reconcile"
    );
    assert_eq!(
        resumed.worker.checkpoint["stage_team_interrupted_tool_recovery"]["tool_name"],
        "eas_fingerprint_services"
    );
    let (tool_status, tool_result) = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT status::text,result FROM tool_calls WHERE id=$1",
    )
    .bind(active_tool_id)
    .fetch_one(db.pool())
    .await
    .expect("load reconciled EAS service fingerprint tool");
    assert_eq!(tool_status, "failed");
    assert!(tool_result
        .as_deref()
        .is_some_and(|result| result.contains("stage_team_interrupted_tool_reconciled")));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM stage_worker_runs WHERE work_item_id=$1"
        )
        .bind(claimed.work_item.id)
        .fetch_one(db.pool())
        .await
        .expect("count logical crawler workers"),
        1,
        "interrupted EAS recovery must not create a replacement Agent"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM stage_team_recovery_decisions WHERE worker_run_id=$1"
        )
        .bind(claimed.worker.id)
        .fetch_one(db.pool())
        .await
        .expect("count operator recovery decisions"),
        0,
        "server-owned safe reconciliation must not masquerade as an operator decision"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn startup_reaper_requeues_expired_aggregator_on_exact_chain() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract_stage_and_children(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
        "external_attack_surface",
        0,
    )
    .await
    .0;
    let mut controller_seed =
        stage_team_controller_seed_for_stage(&roots, "external_attack_surface");
    controller_seed.work_items[0].attempt_policy = serde_json::json!({"max_attempts": 1});
    let seeded = runtime_memory_tx::seed_stage_team_runtime(db.pool(), &controller_seed)
        .await
        .expect("seed Company Controller recovery Team")
        .remove(0);
    let controller_claim = stage_team_claim_input(&roots, &seeded, "stable-aggregator");
    let leader = runtime_memory_tx::claim_stage_team_leader(
        db.pool(),
        &runtime_memory_tx::ClaimStageTeamLeaderRow {
            claim: controller_claim.clone(),
        },
    )
    .await
    .expect("claim Company Controller leader")
    .expect("Company Controller leader is runnable");
    let closed = runtime_memory_tx::close_stage_request_epoch(
        db.pool(),
        &runtime_memory_tx::CloseStageRequestEpochRow {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: seeded.unit.id,
            stage_team_plan_id: seeded.plan.id,
            expected_dispatch_epoch: seeded.plan.dispatch_epoch,
            expected_plan_row_version: leader.plan.row_version,
        },
    )
    .await
    .expect("close Company Controller request epoch");
    assert!(closed.barrier.ready_to_finalize());
    let bound = runtime_memory_tx::bind_stage_team_leader_final_submitter(
        db.pool(),
        &runtime_memory_tx::BindStageTeamLeaderFinalSubmitterRow {
            fence: stage_team_fence(&roots, &seeded, &leader),
            stage_team_plan_id: seeded.plan.id,
            leader_work_item_id: leader.work_item.id,
            expected_plan_row_version: closed.plan.row_version,
            expected_dispatch_epoch: closed.barrier.dispatch_epoch,
            expected_manifest_hash: closed.barrier.manifest_hash.clone(),
        },
    )
    .await
    .expect("bind Company Controller as exact final submitter");
    let aggregator_claim = runtime_memory_tx::ClaimStageAggregatorRow {
        claim: controller_claim,
        expected_dispatch_epoch: closed.barrier.dispatch_epoch,
        expected_manifest_hash: closed.barrier.manifest_hash.clone(),
    };
    let first = runtime_memory_tx::claim_stage_aggregator(db.pool(), &aggregator_claim)
        .await
        .expect("claim first Aggregator");
    assert_eq!(
        bound.plan.final_submitter_worker_run_id,
        Some(first.worker.id)
    );
    let (first_submission, controller_after_submission) = persist_stage_team_submission(
        &db,
        &roots,
        &seeded,
        &first,
        "aggregator-finalization-retry-submit-1",
    )
    .await;
    let parked = runtime_memory_tx::park_stage_team_finalizer_after_failure(
        db.pool(),
        &runtime_memory_tx::ParkStageTeamFinalizerAfterFailureRow {
            fence: runtime_memory_tx::RuntimeMemoryTxFence {
                operation_id: roots.operation_id,
                stage_execution_id: roots.stage_execution_id,
                stage_run_unit_id: seeded.unit.id,
                worker_run_id: first.worker.id,
                lease_token: controller_after_submission
                    .lease_token
                    .expect("finalizer lease remains after submission"),
                attempt_epoch: controller_after_submission.attempt_epoch,
                expected_checkpoint_version: controller_after_submission.checkpoint_version,
            },
            stage_team_plan_id: seeded.plan.id,
            leader_work_item_id: first.work_item.id,
            deliverable_submission_id: first_submission.id,
            expected_work_item_row_version: first.work_item.row_version,
            expected_dispatch_epoch: closed.barrier.dispatch_epoch,
            expected_manifest_hash: closed.barrier.manifest_hash.clone(),
            checkpoint: serde_json::json!([
                {"role": "system", "content": "controller"},
                {"role": "assistant", "content": "final"}
            ]),
            failure_detail: "injected deterministic final seal failure".to_string(),
        },
    )
    .await
    .expect("park exact finalizer after deterministic closeout failure");
    assert_eq!(parked.work_item.status, "queued");
    assert_eq!(parked.worker.status, "queued");
    assert_eq!(parked.worker.lease_token, None);
    assert_eq!(parked.worker.message_chain_id, Some(first.message_chain_id));
    assert_eq!(
        parked.worker.checkpoint["_runtime_stage_team_finalization_retry"]
            ["deliverable_submission_id"],
        serde_json::json!(first_submission.id)
    );
    assert_eq!(
        parked.worker.checkpoint["_runtime_company_finalizer_recovery"]
            ["deliverable_submission_id"],
        serde_json::json!(first_submission.id)
    );
    assert_eq!(
        parked.worker.checkpoint["_runtime_company_finalizer_recovery"]["retry_count"],
        serde_json::json!(0)
    );
    assert_eq!(
        parked.worker.checkpoint["source_terminal_checkpoint"],
        serde_json::json!([
            {"role": "system", "content": "controller"},
            {"role": "assistant", "content": "final"}
        ])
    );
    let resumed_after_finalization_failure =
        runtime_memory_tx::claim_stage_aggregator(db.pool(), &aggregator_claim)
            .await
            .expect("resume parked finalizer on exact chain");
    assert_eq!(
        resumed_after_finalization_failure.worker.id,
        first.worker.id
    );
    assert_eq!(
        resumed_after_finalization_failure.message_chain_id,
        first.message_chain_id
    );
    let (_second_submission, controller_after_second_submission) = persist_stage_team_submission(
        &db,
        &roots,
        &seeded,
        &resumed_after_finalization_failure,
        "aggregator-finalization-retry-submit-2",
    )
    .await;
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET lease_acquired_at=NOW()-INTERVAL '2 hours',
                  lease_expires_at=NOW()-INTERVAL '1 hour',
                  heartbeat_at=NOW()-INTERVAL '1 hour'
            WHERE id=$1"#,
    )
    .bind(controller_after_second_submission.id)
    .execute(db.pool())
    .await
    .expect("expire first Aggregator");
    sqlx::query(
        "UPDATE tasks SET status='running',updated_at=NOW()-INTERVAL '7 hours' WHERE id=$1",
    )
    .bind(roots.operation_id)
    .execute(db.pool())
    .await
    .expect("age Aggregator task");
    let reaped = tasks::startup_reap_abandoned(db.pool(), chrono::Duration::zero())
        .await
        .expect("startup reap expired Aggregator");
    assert_eq!(reaped.workers_requeued, 1);
    let parked_worker = stage_worker_runs::get(db.pool(), first.worker.id)
        .await
        .expect("load parked Aggregator")
        .expect("parked Aggregator remains auditable");
    assert_eq!(parked_worker.status, "queued");
    assert_eq!(parked_worker.message_chain_id, Some(first.message_chain_id));
    assert_eq!(
        parked_worker.checkpoint["_runtime_company_finalizer_recovery"]["retry_count"],
        serde_json::json!(1),
        "startup reaping advances the durable replay generation"
    );
    assert_eq!(
        tasks::get(db.pool(), roots.operation_id)
            .await
            .expect("load startup-reaped Aggregator task")
            .expect("Aggregator task remains")
            .status,
        golish_db::models::TaskStatus::Waiting,
        "a clean Aggregator requeue must remain resumable"
    );
    let items = stage_teams::list_work_items_with_executor(db.pool(), seeded.plan.id)
        .await
        .expect("load Aggregator WorkItem");
    assert_eq!(
        items
            .iter()
            .find(|item| item.id == first.work_item.id)
            .expect("Aggregator WorkItem remains")
            .status,
        "queued"
    );
    assert!(
        stage_teams::list_outputs_with_executor(db.pool(), seeded.plan.id)
            .await
            .expect("load finalizer outputs after startup retry")
            .is_empty(),
        "deterministic closeout retry must not forge a producer exhaustion output"
    );
    let resumed = runtime_memory_tx::claim_stage_aggregator(db.pool(), &aggregator_claim)
        .await
        .expect("resume startup-reaped Aggregator");
    assert_eq!(resumed.worker.id, first.worker.id);
    assert_eq!(resumed.message_chain_id, first.message_chain_id);
    assert_eq!(
        resumed.worker.checkpoint["_runtime_company_finalizer_recovery"]["retry_count"],
        serde_json::json!(1)
    );
    assert_eq!(
        resumed.worker.attempt_epoch,
        resumed_after_finalization_failure.worker.attempt_epoch + 1
    );
    assert_eq!(
        resumed.plan.final_submitter_worker_run_id,
        Some(resumed.worker.id)
    );

    // Model the already-shipped bug: an older binary let startup consume the
    // exact finalizer's producer fuel and wrote an immutable exhausted output.
    // Recovery must preserve that history and replace only the runtime shell.
    let (third_submission, controller_after_third_submission) = persist_stage_team_submission(
        &db,
        &roots,
        &seeded,
        &resumed,
        "aggregator-legacy-terminalized-submit",
    )
    .await;
    // Model the response-loss shape from the retained EAS run: a later
    // final-submitter turn can fail without writing another deliverable, so
    // the last accepted submission belongs to the same Worker but to the
    // immediately preceding attempt.
    let attempts_used = controller_after_third_submission.attempt_epoch + 1;
    let canonical_output = serde_json::json!({
        "attempts_used": attempts_used,
        "failure_code": "stage_team_worker_lease_expired",
        "kind": "stage_team_attempts_exhausted",
        "max_attempts": 1,
        "schema_version": 1,
        "stable_work_key": resumed.work_item.stable_key,
    });
    let mut terminal_output = stage_teams::CompleteStageWorkerRow {
        fence: runtime_memory_tx::RuntimeMemoryTxFence {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: seeded.unit.id,
            worker_run_id: resumed.worker.id,
            lease_token: controller_after_third_submission
                .lease_token
                .expect("legacy finalizer lease"),
            attempt_epoch: controller_after_third_submission.attempt_epoch,
            expected_checkpoint_version: controller_after_third_submission.checkpoint_version,
        },
        team_plan_id: seeded.plan.id,
        work_item_id: resumed.work_item.id,
        expected_work_item_row_version: resumed.work_item.row_version,
        output_schema: resumed.work_item.output_schema.clone(),
        business_disposition: "blocked".to_string(),
        canonical_output: canonical_output.clone(),
        canonical_fact_refs: serde_json::json!([]),
        evidence_ids: Vec::new(),
        checked_empty_cells: serde_json::json!([]),
        blocker_codes: vec!["STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED".to_string()],
        output_hash: String::new(),
        terminal_checkpoint: serde_json::json!({}),
        evidence_watermark: None,
    };
    refresh_stage_worker_output_hash(&mut terminal_output);
    let immutable_output_id = Uuid::new_v4();
    let mut historical_tx = db.pool().begin().await.expect("begin historical fixture");
    sqlx::query("UPDATE stage_worker_runs SET attempt_epoch=$2 WHERE id=$1 AND status='running'")
        .bind(resumed.worker.id)
        .bind(attempts_used)
        .execute(&mut *historical_tx)
        .await
        .expect("advance finalizer after response loss without another submission");
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET status='failed',checkpoint=$2,checkpoint_version=checkpoint_version+1,
                  lease_token=NULL,lease_owner=NULL,lease_acquired_at=NULL,
                  lease_expires_at=NULL,heartbeat_at=NULL,terminal_at=NOW(),updated_at=NOW()
            WHERE id=$1 AND status='running'"#,
    )
    .bind(resumed.worker.id)
    .bind(serde_json::json!({
        "stage_team_execution_failure": {
            "attempts_used": attempts_used,
            "code": "stage_team_worker_lease_expired",
            "max_attempts": 1,
            "schema_version": 1,
        }
    }))
    .execute(&mut *historical_tx)
    .await
    .expect("model legacy failed finalizer");
    sqlx::query(
        r#"UPDATE stage_work_items
              SET status='exhausted',row_version=row_version+1,terminal_at=NOW(),updated_at=NOW()
            WHERE id=$1 AND status='running'"#,
    )
    .bind(resumed.work_item.id)
    .execute(&mut *historical_tx)
    .await
    .expect("model legacy exhausted finalizer item");
    sqlx::query(
        r#"INSERT INTO stage_worker_outputs(
               id,team_plan_id,work_item_id,worker_run_id,operation_id,
               stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id,
               output_schema,output_version,business_disposition,canonical_output,
               canonical_fact_refs,evidence_ids,checked_empty_cells,blocker_codes,output_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,1,'blocked',$11,'[]',
                    ARRAY[]::BIGINT[],'[]',$12,$13)"#,
    )
    .bind(immutable_output_id)
    .bind(seeded.plan.id)
    .bind(resumed.work_item.id)
    .bind(resumed.worker.id)
    .bind(roots.operation_id)
    .bind(roots.stage_execution_id)
    .bind(seeded.unit.id)
    .bind(roots.snapshot_id)
    .bind(roots.organization_id)
    .bind(&resumed.work_item.output_schema)
    .bind(&canonical_output)
    .bind(vec!["STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED".to_string()])
    .bind(&terminal_output.output_hash)
    .execute(&mut *historical_tx)
    .await
    .expect("persist immutable legacy exhaustion output");
    historical_tx
        .commit()
        .await
        .expect("commit historical finalizer fixture");
    let stage_started_before_recovery = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
        "SELECT stage_started_at FROM operation_state WHERE operation_id=$1",
    )
    .bind(roots.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("load source stage freshness epoch");

    let recovered = runtime_memory_tx::claim_stage_aggregator(db.pool(), &aggregator_claim)
        .await
        .expect("historical terminal finalizer must recover in place");
    assert_eq!(recovered.worker.id, resumed.worker.id);
    assert_eq!(recovered.work_item.id, resumed.work_item.id);
    assert_eq!(recovered.message_chain_id, resumed.message_chain_id);
    assert_eq!(recovered.worker.status, "running");
    assert_eq!(recovered.work_item.status, "running");
    assert_eq!(recovered.worker.attempt_epoch, attempts_used + 1);
    assert_eq!(
        recovered.worker.checkpoint["_runtime_company_finalizer_recovery"]
            ["deliverable_submission_id"],
        serde_json::json!(third_submission.id)
    );
    assert_eq!(
        recovered.worker.checkpoint["_runtime_company_finalizer_recovery"]["payload_sha256"],
        serde_json::json!(third_submission.payload_sha256)
    );
    assert_eq!(
        recovered.plan.final_submitter_worker_run_id,
        Some(recovered.worker.id)
    );
    let active = stage_runs::list_active_for_operation_with_executor(db.pool(), roots.operation_id)
        .await
        .expect("load active execution after in-place recovery");
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, roots.stage_execution_id);
    assert_eq!(active[0].stage_kind, seeded.unit.stage_kind);
    let recovered_stage_started_at = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
        "SELECT stage_started_at FROM operation_state WHERE operation_id=$1",
    )
    .bind(roots.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("load operation after in-place finalizer recovery");
    assert_eq!(
        recovered_stage_started_at, stage_started_before_recovery,
        "in-place finalizer recovery must preserve the producer freshness epoch"
    );
    assert_eq!(
        stage_teams::list_outputs_with_executor(db.pool(), seeded.plan.id)
            .await
            .expect("load immutable output after rollover")[0]
            .id,
        immutable_output_id
    );

    // A retained Controller can have several accepted submissions from prior
    // turns. Recovery authority names exactly one immutable submission; the
    // startup reaper must filter to that ID before applying its singleton
    // check instead of counting unrelated historical submissions.
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET lease_acquired_at=NOW()-INTERVAL '2 hours',
                  lease_expires_at=NOW()-INTERVAL '1 hour',
                  heartbeat_at=NOW()-INTERVAL '1 hour'
            WHERE id=$1"#,
    )
    .bind(recovered.worker.id)
    .execute(db.pool())
    .await
    .expect("expire recovered finalizer with submission history");
    sqlx::query(
        "UPDATE tasks SET status='running',updated_at=NOW()-INTERVAL '7 hours' WHERE id=$1",
    )
    .bind(roots.operation_id)
    .execute(db.pool())
    .await
    .expect("age recovered finalizer task");
    let reaped_again = tasks::startup_reap_abandoned(db.pool(), chrono::Duration::zero())
        .await
        .expect("reap exact recovered finalizer despite older submissions");
    assert_eq!(reaped_again.workers_requeued, 1);
    let recovered_again = runtime_memory_tx::claim_stage_aggregator(db.pool(), &aggregator_claim)
        .await
        .expect("claim exact recovered finalizer after historical-submission reap");
    assert_eq!(recovered_again.worker.id, recovered.worker.id);
    assert_eq!(recovered_again.work_item.id, recovered.work_item.id);
    assert_eq!(
        recovered_again.worker.checkpoint["_runtime_company_finalizer_recovery"]
            ["deliverable_submission_id"],
        serde_json::json!(third_submission.id)
    );
    assert_eq!(
        stage_teams::list_outputs_with_executor(db.pool(), seeded.plan.id)
            .await
            .expect("immutable output remains singleton after exact reap")
            .len(),
        1
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn terminalized_final_submitter_without_durable_submission_fails_closed() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    let mut controller_seed = stage_team_controller_seed(&roots);
    controller_seed.work_items[0].attempt_policy = serde_json::json!({"max_attempts": 1});
    let seeded = runtime_memory_tx::seed_stage_team_runtime(db.pool(), &controller_seed)
        .await
        .expect("seed no-submission Controller")
        .remove(0);
    let claim = stage_team_claim_input(&roots, &seeded, "no-submission-finalizer");
    let leader = runtime_memory_tx::claim_stage_team_leader(
        db.pool(),
        &runtime_memory_tx::ClaimStageTeamLeaderRow {
            claim: claim.clone(),
        },
    )
    .await
    .expect("claim no-submission Controller")
    .expect("Controller is runnable");
    let closed = runtime_memory_tx::close_stage_request_epoch(
        db.pool(),
        &runtime_memory_tx::CloseStageRequestEpochRow {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: seeded.unit.id,
            stage_team_plan_id: seeded.plan.id,
            expected_dispatch_epoch: seeded.plan.dispatch_epoch,
            expected_plan_row_version: leader.plan.row_version,
        },
    )
    .await
    .expect("close no-submission Controller plan");
    runtime_memory_tx::bind_stage_team_leader_final_submitter(
        db.pool(),
        &runtime_memory_tx::BindStageTeamLeaderFinalSubmitterRow {
            fence: stage_team_fence(&roots, &seeded, &leader),
            stage_team_plan_id: seeded.plan.id,
            leader_work_item_id: leader.work_item.id,
            expected_plan_row_version: closed.plan.row_version,
            expected_dispatch_epoch: closed.barrier.dispatch_epoch,
            expected_manifest_hash: closed.barrier.manifest_hash.clone(),
        },
    )
    .await
    .expect("bind no-submission finalizer");
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET lease_acquired_at=NOW()-INTERVAL '2 hours',
                  lease_expires_at=NOW()-INTERVAL '1 hour',
                  heartbeat_at=NOW()-INTERVAL '1 hour'
            WHERE id=$1"#,
    )
    .bind(leader.worker.id)
    .execute(db.pool())
    .await
    .expect("expire no-submission finalizer");
    sqlx::query(
        "UPDATE tasks SET status='running',updated_at=NOW()-INTERVAL '7 hours' WHERE id=$1",
    )
    .bind(roots.operation_id)
    .execute(db.pool())
    .await
    .expect("age no-submission task");
    let reaped = tasks::startup_reap_abandoned(db.pool(), chrono::Duration::zero())
        .await
        .expect("terminalize no-submission finalizer");
    assert_eq!(reaped.workers_requeued, 0);

    let error = runtime_memory_tx::claim_stage_aggregator(
        db.pool(),
        &runtime_memory_tx::ClaimStageAggregatorRow {
            claim,
            expected_dispatch_epoch: closed.barrier.dispatch_epoch,
            expected_manifest_hash: closed.barrier.manifest_hash,
        },
    )
    .await
    .expect_err("missing durable submission must not authorize runtime replacement");
    assert!(matches!(
        error,
        runtime_memory_tx::RuntimeMemoryStoreError::Conflict {
            code: "stage_team_final_submitter_not_replaceable"
        }
    ));
    let active = stage_runs::list_active_for_operation_with_executor(db.pool(), roots.operation_id)
        .await
        .expect("load unchanged active execution");
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, roots.stage_execution_id);

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn stage_team_gate_repair_advances_epoch_and_blocks_only_fresh_aggregator() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    let seeded = runtime_memory_tx::seed_stage_team_runtime(
        db.pool(),
        &stage_team_lifecycle_seed(&roots, 2),
    )
    .await
    .expect("seed repair Team")
    .remove(0);

    for owner in ["repair-producer", "repair-helper"] {
        let claimed = runtime_memory_tx::claim_stage_work_item(
            db.pool(),
            &stage_team_claim_input(&roots, &seeded, owner),
        )
        .await
        .expect("claim required producer")
        .expect("required producer remains");
        let evidence_id = sqlx::query_scalar::<_, i64>(
            r#"INSERT INTO audit_log (
                   action,category,details,project_path,source,audit_role,detail,run_id,created_at
               ) VALUES (
                   'repair producer evidence','harness','fresh producer evidence',
                   '/tmp/runtime-worker','harness','evidence',$1,$2,NOW()
               ) RETURNING id"#,
        )
        .bind(serde_json::json!({"organization_id": roots.organization_id}))
        .bind(roots.operation_id)
        .fetch_one(db.pool())
        .await
        .expect("insert repair producer evidence");
        let mut completion = stage_teams::CompleteStageWorkerRow {
            fence: stage_team_fence(&roots, &seeded, &claimed),
            team_plan_id: seeded.plan.id,
            work_item_id: claimed.work_item.id,
            expected_work_item_row_version: claimed.work_item.row_version,
            output_schema: claimed.work_item.output_schema.clone(),
            business_disposition: "found".to_string(),
            canonical_output: serde_json::json!({"summary": "producer complete"}),
            canonical_fact_refs: serde_json::json!([]),
            evidence_ids: vec![evidence_id],
            checked_empty_cells: serde_json::json!([]),
            blocker_codes: Vec::new(),
            output_hash: String::new(),
            terminal_checkpoint: serde_json::json!({"done": true}),
            evidence_watermark: Some(evidence_id),
        };
        refresh_stage_worker_output_hash(&mut completion);
        stage_teams::complete_stage_worker(db.pool(), completion)
            .await
            .expect("complete required producer");
    }

    let first_closed = runtime_memory_tx::close_stage_request_epoch(
        db.pool(),
        &runtime_memory_tx::CloseStageRequestEpochRow {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: seeded.unit.id,
            stage_team_plan_id: seeded.plan.id,
            expected_dispatch_epoch: seeded.plan.dispatch_epoch,
            expected_plan_row_version: seeded.plan.row_version,
        },
    )
    .await
    .expect("close first producer epoch");
    assert!(first_closed.barrier.ready_to_finalize());
    let first_aggregator = runtime_memory_tx::claim_stage_aggregator(
        db.pool(),
        &runtime_memory_tx::ClaimStageAggregatorRow {
            claim: stage_team_claim_input(&roots, &seeded, "repair-first-aggregator"),
            expected_dispatch_epoch: first_closed.barrier.dispatch_epoch,
            expected_manifest_hash: first_closed.barrier.manifest_hash.clone(),
        },
    )
    .await
    .expect("claim first Aggregator");
    let first_aggregator_fence = stage_team_fence(&roots, &seeded, &first_aggregator);
    let tool_request_id = "repair-first-aggregator-submit";
    let tool_call_id = tool_calls::record_tracked_start(
        db.pool(),
        tool_request_id,
        roots.session_id,
        Some(roots.operation_id),
        None,
        "submit_stage_deliverable",
        &serde_json::json!({"stage_id": "target_intel"}),
        Some(&tool_calls::RuntimeToolIdentity {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: Some(seeded.unit.id),
            worker_run_id: Some(first_aggregator.worker.id),
            organization_id: Some(roots.organization_id),
            attempt_epoch: Some(first_aggregator.worker.attempt_epoch),
            lease_token: first_aggregator.worker.lease_token,
        }),
    )
    .await
    .expect("record first Aggregator submission tool");
    runtime_memory_tx::begin_worker_tool(db.pool(), &first_aggregator_fence, tool_call_id)
        .await
        .expect("fence first Aggregator submission");
    let payload = serde_json::json!({
        "stage_id": "target_intel",
        "stage_run_id": roots.stage_execution_id,
        "claims": [],
    });
    let canonical_payload_json = canonical_json(&payload);
    let submission = stage_deliverable_submissions::insert(
        db.pool(),
        &stage_deliverable_submissions::NewStageDeliverableSubmission {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: Some(seeded.unit.id),
            worker_run_id: Some(first_aggregator.worker.id),
            organization_id: Some(roots.organization_id),
            tool_call_record_id: tool_call_id,
            tool_request_id: tool_request_id.to_string(),
            stage_kind: "target_intel".to_string(),
            attempt_epoch: Some(first_aggregator.worker.attempt_epoch),
            lease_token: first_aggregator.worker.lease_token,
            payload_sha256: Sha256::digest(canonical_payload_json.as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            canonical_payload_json,
        },
    )
    .await
    .expect("persist first Aggregator submission");
    let first_aggregator_worker =
        runtime_memory_tx::finish_worker_tool(db.pool(), &first_aggregator_fence, tool_call_id)
            .await
            .expect("clear first Aggregator submission fence");
    tool_calls::record_tracked_finish(
        db.pool(),
        tool_call_id,
        roots.session_id,
        "finished",
        "{}",
        1,
    )
    .await
    .expect("finish first Aggregator submission tool");

    let gate_decision_hash = format!(
        "sha256:{}",
        sha256_json(&serde_json::json!({"decision": "block", "schema_version": 1}))
    );
    let gap_manifest = serde_json::json!({
        "gate_decision_hash": gate_decision_hash,
        "reasons": ["missing_required_fact"],
        "recovery_actions": ["collect_missing_fact"],
        "schema_version": 1,
    });
    let repair_input = runtime_memory_tx::OpenStageTeamRepairRow {
        request_id: "stage-team-gate-repair-epoch-1".to_string(),
        fence: runtime_memory_tx::RuntimeMemoryTxFence {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: seeded.unit.id,
            worker_run_id: first_aggregator.worker.id,
            lease_token: first_aggregator
                .worker
                .lease_token
                .expect("Aggregator lease"),
            attempt_epoch: first_aggregator.worker.attempt_epoch,
            expected_checkpoint_version: first_aggregator_worker.checkpoint_version,
        },
        stage_team_plan_id: seeded.plan.id,
        aggregator_work_item_id: first_aggregator.work_item.id,
        deliverable_submission_id: submission.id,
        expected_dispatch_epoch: first_closed.barrier.dispatch_epoch,
        expected_manifest_hash: first_closed.barrier.manifest_hash.clone(),
        gate_decision_hash,
        gap_manifest_hash: format!("sha256:{}", sha256_json(&gap_manifest)),
        gap_manifest,
    };
    let opened = runtime_memory_tx::open_stage_team_repair(db.pool(), &repair_input)
        .await
        .expect("open next repair generation");
    assert!(!opened.replayed);
    assert_eq!(opened.unit.status, "running");
    assert_eq!(opened.plan.dispatch_epoch, seeded.plan.dispatch_epoch + 1);
    assert_eq!(opened.aggregator_worker.status, "gate_blocked");
    let restarted_after_repair_epoch = runtime_memory_tx::seed_stage_team_runtime(
        db.pool(),
        &stage_team_lifecycle_seed(&roots, 2),
    )
    .await
    .expect("stage_run reentry replays the immutable seed after repair epoch advance");
    assert_eq!(restarted_after_repair_epoch.len(), 1);
    assert!(restarted_after_repair_epoch[0].replayed);
    assert_eq!(
        restarted_after_repair_epoch[0].plan.dispatch_epoch,
        opened.plan.dispatch_epoch
    );
    let repair_item = opened
        .repair_work_item
        .as_ref()
        .expect("repair producer WorkItem");
    let fresh_aggregator_item = opened
        .aggregator_work_item
        .as_ref()
        .expect("fresh Aggregator WorkItem");
    assert_eq!(repair_item.dispatch_epoch, opened.plan.dispatch_epoch);
    assert_eq!(
        fresh_aggregator_item.dispatch_epoch,
        opened.plan.dispatch_epoch
    );
    assert_ne!(fresh_aggregator_item.id, first_aggregator.work_item.id);
    assert!(
        runtime_memory_tx::open_stage_team_repair(db.pool(), &repair_input)
            .await
            .expect("exact repair response-loss replay")
            .replayed
    );

    let repair_attempt_one = runtime_memory_tx::claim_stage_work_item(
        db.pool(),
        &stage_team_claim_input(&roots, &seeded, "repair-attempt-one"),
    )
    .await
    .expect("claim repair producer")
    .expect("repair producer remains");
    assert_eq!(repair_attempt_one.work_item.id, repair_item.id);
    let first_retry = stage_teams::retry_stage_worker(
        db.pool(),
        stage_teams::RetryStageWorkerRow {
            fence: stage_team_fence(&roots, &seeded, &repair_attempt_one),
            team_plan_id: seeded.plan.id,
            work_item_id: repair_attempt_one.work_item.id,
            expected_work_item_row_version: repair_attempt_one.work_item.row_version,
            failure_code: "provider_unavailable".to_string(),
            terminal_checkpoint: serde_json::json!({
                "stage_team_execution_failure": {"code": "provider_unavailable"}
            }),
        },
    )
    .await
    .expect("schedule second repair attempt");
    assert!(first_retry.retry_scheduled);
    let repair_attempt_two = runtime_memory_tx::claim_stage_work_item(
        db.pool(),
        &stage_team_claim_input(&roots, &seeded, "repair-attempt-two"),
    )
    .await
    .expect("claim second repair producer")
    .expect("second repair attempt remains");
    assert_eq!(repair_attempt_two.work_item.id, repair_item.id);
    let exhausted = stage_teams::retry_stage_worker(
        db.pool(),
        stage_teams::RetryStageWorkerRow {
            fence: stage_team_fence(&roots, &seeded, &repair_attempt_two),
            team_plan_id: seeded.plan.id,
            work_item_id: repair_attempt_two.work_item.id,
            expected_work_item_row_version: repair_attempt_two.work_item.row_version,
            failure_code: "provider_unavailable".to_string(),
            terminal_checkpoint: serde_json::json!({
                "stage_team_execution_failure": {"code": "provider_unavailable"}
            }),
        },
    )
    .await
    .expect("exhaust repair producer");
    assert!(!exhausted.retry_scheduled);
    assert_eq!(exhausted.work_item.status, "exhausted");

    let repair_closed = runtime_memory_tx::close_stage_request_epoch(
        db.pool(),
        &runtime_memory_tx::CloseStageRequestEpochRow {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: seeded.unit.id,
            stage_team_plan_id: seeded.plan.id,
            expected_dispatch_epoch: opened.plan.dispatch_epoch,
            expected_plan_row_version: opened.plan.row_version,
        },
    )
    .await
    .expect("close repair epoch");
    assert!(repair_closed.barrier.ready_to_finalize());
    let blocked = runtime_memory_tx::block_stage_team_unit(
        db.pool(),
        &runtime_memory_tx::BlockStageTeamUnitRow {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: seeded.unit.id,
            stage_team_plan_id: seeded.plan.id,
            expected_dispatch_epoch: opened.plan.dispatch_epoch,
            expected_manifest_hash: repair_closed.barrier.manifest_hash,
        },
    )
    .await
    .expect("block only the fresh repair Aggregator");
    assert_eq!(blocked.unit.status, "gate_blocked");
    assert_eq!(blocked.aggregator_work_item.id, fresh_aggregator_item.id);
    assert_eq!(blocked.aggregator_work_item.status, "superseded");
    assert_ne!(
        blocked.aggregator_work_item.id,
        first_aggregator.work_item.id
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn stage_team_exhausted_attempt_lands_explicit_blocked_output() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    let seeded = runtime_memory_tx::seed_stage_team_runtime(
        db.pool(),
        &stage_team_lifecycle_seed(&roots, 1),
    )
    .await
    .expect("seed exhaustion Team")
    .remove(0);
    let producer = runtime_memory_tx::claim_stage_work_item(
        db.pool(),
        &stage_team_claim_input(&roots, &seeded, "exhausted-producer"),
    )
    .await
    .expect("claim producer")
    .expect("producer WorkItem exists");
    let retried = stage_teams::retry_stage_worker(
        db.pool(),
        stage_teams::RetryStageWorkerRow {
            fence: stage_team_fence(&roots, &seeded, &producer),
            team_plan_id: seeded.plan.id,
            work_item_id: producer.work_item.id,
            expected_work_item_row_version: producer.work_item.row_version,
            failure_code: "provider_unavailable".to_string(),
            terminal_checkpoint: serde_json::json!({
                "stage_team_execution_failure": {"code": "provider_unavailable"}
            }),
        },
    )
    .await
    .expect("terminalize exhausted execution attempt");
    assert!(!retried.retry_scheduled);
    assert_eq!(retried.work_item.status, "exhausted");
    assert_eq!(retried.worker.status, "failed");
    let outputs = stage_teams::list_outputs_with_executor(db.pool(), seeded.plan.id)
        .await
        .expect("load deterministic exhaustion output");
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].work_item_id, producer.work_item.id);
    assert_eq!(outputs[0].business_disposition, "blocked");
    assert_eq!(
        outputs[0].blocker_codes,
        vec!["STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED"]
    );

    let helper = runtime_memory_tx::claim_stage_work_item(
        db.pool(),
        &stage_team_claim_input(&roots, &seeded, "exhausted-helper"),
    )
    .await
    .expect("claim helper")
    .expect("helper WorkItem exists");
    let helper_retried = stage_teams::retry_stage_worker(
        db.pool(),
        stage_teams::RetryStageWorkerRow {
            fence: stage_team_fence(&roots, &seeded, &helper),
            team_plan_id: seeded.plan.id,
            work_item_id: helper.work_item.id,
            expected_work_item_row_version: helper.work_item.row_version,
            failure_code: "provider_unavailable".to_string(),
            terminal_checkpoint: serde_json::json!({
                "stage_team_execution_failure": {"code": "provider_unavailable"}
            }),
        },
    )
    .await
    .expect("terminalize exhausted helper attempt");
    assert!(!helper_retried.retry_scheduled);
    let closed = runtime_memory_tx::close_stage_request_epoch(
        db.pool(),
        &runtime_memory_tx::CloseStageRequestEpochRow {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: seeded.unit.id,
            stage_team_plan_id: seeded.plan.id,
            expected_dispatch_epoch: seeded.plan.dispatch_epoch,
            expected_plan_row_version: seeded.plan.row_version,
        },
    )
    .await
    .expect("close exhausted producer epoch");
    assert!(closed.barrier.ready_to_finalize());
    assert_eq!(closed.barrier.required_work_items, 2);
    assert_eq!(closed.barrier.terminal_required_work_items, 2);
    assert_eq!(closed.barrier.missing_outputs, 0);
    let blocked = runtime_memory_tx::block_stage_team_unit(
        db.pool(),
        &runtime_memory_tx::BlockStageTeamUnitRow {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: seeded.unit.id,
            stage_team_plan_id: seeded.plan.id,
            expected_dispatch_epoch: closed.barrier.dispatch_epoch,
            expected_manifest_hash: closed.barrier.manifest_hash.clone(),
        },
    )
    .await
    .expect("deterministically block Unit from immutable outputs");
    assert_eq!(blocked.unit.status, "gate_blocked");
    assert_eq!(blocked.aggregator_work_item.status, "superseded");
    assert!(!blocked.replayed);
    assert!(
        runtime_memory_tx::block_stage_team_unit(
            db.pool(),
            &runtime_memory_tx::BlockStageTeamUnitRow {
                operation_id: roots.operation_id,
                stage_execution_id: roots.stage_execution_id,
                stage_run_unit_id: seeded.unit.id,
                stage_team_plan_id: seeded.plan.id,
                expected_dispatch_epoch: closed.barrier.dispatch_epoch,
                expected_manifest_hash: closed.barrier.manifest_hash,
            },
        )
        .await
        .expect("block terminalizer exact replay")
        .replayed
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn stage_team_producer_cannot_persist_unit_deliverable() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    let seeded = runtime_memory_tx::seed_stage_team_runtime(
        db.pool(),
        &stage_team_lifecycle_seed(&roots, 2),
    )
    .await
    .expect("seed submit-authority Team")
    .remove(0);
    let producer = runtime_memory_tx::claim_stage_work_item(
        db.pool(),
        &stage_team_claim_input(&roots, &seeded, "producer-submit-attempt"),
    )
    .await
    .expect("claim producer")
    .expect("producer WorkItem exists");
    let fence = stage_team_fence(&roots, &seeded, &producer);
    let tool_call_id = tool_calls::record_tracked_start(
        db.pool(),
        "team-producer-submit",
        roots.session_id,
        Some(roots.operation_id),
        None,
        "submit_stage_deliverable",
        &serde_json::json!({"stage_id": "target_intel"}),
        Some(&tool_calls::RuntimeToolIdentity {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: Some(seeded.unit.id),
            worker_run_id: Some(producer.worker.id),
            organization_id: Some(roots.organization_id),
            attempt_epoch: Some(producer.worker.attempt_epoch),
            lease_token: producer.worker.lease_token,
        }),
    )
    .await
    .expect("record producer submit call");
    runtime_memory_tx::begin_worker_tool(db.pool(), &fence, tool_call_id)
        .await
        .expect("fence producer submit call");
    let payload = serde_json::json!({
        "stage_id": "target_intel",
        "stage_run_id": roots.stage_execution_id,
        "claims": [],
    });
    let canonical_payload_json = canonical_json(&payload);
    let error = stage_deliverable_submissions::insert(
        db.pool(),
        &stage_deliverable_submissions::NewStageDeliverableSubmission {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: Some(seeded.unit.id),
            worker_run_id: Some(producer.worker.id),
            organization_id: Some(roots.organization_id),
            tool_call_record_id: tool_call_id,
            tool_request_id: "team-producer-submit".to_string(),
            stage_kind: "target_intel".to_string(),
            attempt_epoch: Some(producer.worker.attempt_epoch),
            lease_token: producer.worker.lease_token,
            payload_sha256: Sha256::digest(canonical_payload_json.as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            canonical_payload_json,
        },
    )
    .await
    .expect_err("only the claimed Team Aggregator may submit the Unit deliverable");
    assert_eq!(
        error.code(),
        "stage_team_submission_requires_unique_aggregator"
    );

    db.stop().await;
}

async fn create_final_seal_fixture(db: &GolishDb) -> FinalSealFixture {
    let runtime = create_claimed_compound_runtime(db).await;
    let fence = fence_for_claimed(&runtime);
    let unit = stage_run_units::get(db.pool(), runtime.unit_id)
        .await
        .expect("load running final-seal unit")
        .expect("final-seal unit exists");
    let target_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO targets (
               id, name, target_type, value, scope, project_path, organization_id
           ) VALUES ($1,'seal.example','domain','seal.example','in',
                     '/tmp/runtime-worker',$2)"#,
    )
    .bind(target_id)
    .bind(runtime.roots.organization_id)
    .execute(db.pool())
    .await
    .expect("insert fresh owned canonical target");
    let finding_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO findings (
               id, title, sev, target, target_id, project_path, source
           ) VALUES ($1,'Fresh finding','medium','seal.example',$2,
                     '/tmp/runtime-worker','harness')"#,
    )
    .bind(finding_id)
    .bind(target_id)
    .execute(db.pool())
    .await
    .expect("insert fresh target-owned finding");
    let evidence_id = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO audit_log (
               action, category, details, project_path, source, target_id,
               session_id, tool_name, status, detail, run_id, audit_role,
               created_at
           ) VALUES (
               'final seal evidence','harness','fresh exact evidence',
               '/tmp/runtime-worker','harness',$1,$2,'submit_stage_deliverable',
               'completed',$3,$4,'evidence',NOW()
           ) RETURNING id"#,
    )
    .bind(target_id)
    .bind(runtime.roots.session_id.to_string())
    .bind(serde_json::json!({"organization_id": runtime.roots.organization_id}))
    .bind(runtime.roots.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("insert exact owned evidence");

    let tool_call_id = tool_calls::record_tracked_start(
        db.pool(),
        "final-seal-submit",
        runtime.roots.session_id,
        Some(runtime.roots.operation_id),
        None,
        "submit_stage_deliverable",
        &serde_json::json!({"stage_id": "target_intel"}),
        Some(&tool_calls::RuntimeToolIdentity {
            operation_id: runtime.roots.operation_id,
            stage_execution_id: runtime.roots.stage_execution_id,
            stage_run_unit_id: Some(runtime.unit_id),
            worker_run_id: Some(runtime.worker_id),
            organization_id: Some(runtime.roots.organization_id),
            attempt_epoch: Some(runtime.worker.attempt_epoch),
            lease_token: runtime.worker.lease_token,
        }),
    )
    .await
    .expect("record exact submit tool call");
    runtime_memory_tx::begin_worker_tool(db.pool(), &fence, tool_call_id)
        .await
        .expect("fence submit tool dispatch");
    let canonical_deliverable = serde_json::json!({
        "stage_id": "target_intel",
        "stage_run_id": runtime.roots.stage_execution_id,
        "claims": [{"kind": "target_intel_complete"}],
    });
    let canonical_deliverable_json = canonical_json(&canonical_deliverable);
    let payload_sha256 = Sha256::digest(canonical_deliverable_json.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let submission = stage_deliverable_submissions::insert(
        db.pool(),
        &stage_deliverable_submissions::NewStageDeliverableSubmission {
            operation_id: runtime.roots.operation_id,
            stage_execution_id: runtime.roots.stage_execution_id,
            stage_run_unit_id: Some(runtime.unit_id),
            worker_run_id: Some(runtime.worker_id),
            organization_id: Some(runtime.roots.organization_id),
            tool_call_record_id: tool_call_id,
            tool_request_id: "final-seal-submit".to_string(),
            stage_kind: "target_intel".to_string(),
            attempt_epoch: Some(runtime.worker.attempt_epoch),
            lease_token: runtime.worker.lease_token,
            canonical_payload_json: canonical_deliverable_json,
            payload_sha256,
        },
    )
    .await
    .expect("persist immutable worker submission");
    runtime_memory_tx::finish_worker_tool(db.pool(), &fence, tool_call_id)
        .await
        .expect("clear exact submit tool fence");
    tool_calls::record_tracked_finish(
        db.pool(),
        tool_call_id,
        runtime.roots.session_id,
        "finished",
        "{}",
        1,
    )
    .await
    .expect("finish exact submit tool call");

    FinalSealFixture {
        runtime,
        fence,
        unit,
        submission_id: submission.id,
        target_id,
        finding_id,
        evidence_id,
    }
}

fn final_seal_input(
    fixture: &FinalSealFixture,
    canonical_fact_keys: Vec<canonical_fact_refs::CanonicalFactKey>,
) -> runtime_memory_tx::FinalizeUnitPassRow {
    let typed_claims = vec![serde_json::json!({
        "kind": "target_intel_complete",
        "target_id": fixture.target_id,
    })];
    let coverage_watermark = serde_json::json!({"terminal_cells": 1});
    let evidence_ids = vec![fixture.evidence_id];
    let terminal_checkpoint = serde_json::json!({"terminal": true});
    let details = serde_json::json!({});
    let seal_material = serde_json::json!({
        "canonical_fact_keys": canonical_fact_keys,
        "typed_claims": typed_claims,
        "coverage_watermark": coverage_watermark,
        "evidence_ids": evidence_ids,
        "terminal_checkpoint": terminal_checkpoint,
        "deterministic_gate_details": details,
        "candidate_acceptance": serde_json::Value::Null,
    });
    let gate_decision = serde_json::json!({
        "outcome": "pass",
        "operation_id": fixture.runtime.roots.operation_id,
        "stage_execution_id": fixture.runtime.roots.stage_execution_id,
        "stage_run_unit_id": fixture.runtime.unit_id,
        "deliverable_submission_id": fixture.submission_id,
        "scope_hash": "scope-sha",
        "seal_material_sha256": sha256_json(&seal_material),
        "details": details,
    });
    runtime_memory_tx::FinalizeUnitPassRow {
        fence: fixture.fence.clone(),
        deliverable_submission_id: fixture.submission_id,
        expected_unit_status: stage_run_units::StageRunUnitStatus::Running,
        expected_unit_row_version: fixture.unit.row_version,
        scope_hash: "scope-sha".to_string(),
        gate_decision_hash: sha256_json(&gate_decision),
        gate_decision,
        aggregate_pass_token_hash: None,
        canonical_fact_keys,
        typed_claims,
        coverage_watermark,
        evidence_ids,
        terminal_checkpoint,
        candidate_acceptance: None,
    }
}

async fn create_initial_final_seal_wave(
    db: &GolishDb,
    fixture: &FinalSealFixture,
) -> stage_asset_waves::StageAssetWaveWithItems {
    stage_asset_waves::current_or_create_initial(
        db.pool(),
        fixture.runtime.roots.operation_id,
        fixture.runtime.roots.organization_id,
        "target_intel",
        chrono::Utc::now() + chrono::Duration::seconds(1),
        100,
    )
    .await
    .expect("create exact initial V2 wave")
    .expect("initial V2 wave has the fixture target")
}

fn close_wave_input(
    fixture: &FinalSealFixture,
    wave: &stage_asset_waves::StageAssetWaveWithItems,
) -> runtime_memory_tx::CloseWaveGatePassRow {
    let mut final_seal = final_seal_input(
        fixture,
        vec![canonical_fact_refs::CanonicalFactKey::Target {
            target_id: fixture.target_id,
        }],
    );
    final_seal.coverage_watermark = serde_json::json!({
        "stage": "target_intel",
        "organization_id": fixture.runtime.roots.organization_id,
        "terminal_cells": 1,
        "waves": [{
            "id": wave.wave.id,
            "wave_index": wave.wave.wave_index,
            "asset_count": wave.items.len(),
            "asset_hash": wave.wave.asset_hash,
        }],
        "wave_count": 1,
        "wave_asset_count": wave.items.len(),
    });
    let seal_material = serde_json::json!({
        "canonical_fact_keys": final_seal.canonical_fact_keys,
        "typed_claims": final_seal.typed_claims,
        "coverage_watermark": final_seal.coverage_watermark,
        "evidence_ids": final_seal.evidence_ids,
        "terminal_checkpoint": final_seal.terminal_checkpoint,
        "deterministic_gate_details": final_seal.gate_decision["details"],
        "candidate_acceptance": final_seal.candidate_acceptance,
    });
    final_seal.gate_decision["seal_material_sha256"] =
        serde_json::json!(sha256_json(&seal_material));
    final_seal.gate_decision_hash = sha256_json(&final_seal.gate_decision);
    runtime_memory_tx::CloseWaveGatePassRow {
        final_seal,
        wave_id: wave.wave.id,
        next_wave_limit: 100,
        continuation_pass_watermark: serde_json::json!({
            "pending_v2_final_seal": {
                "deliverable_submission_id": fixture.submission_id,
                "material": {
                    "cells": [{
                        "asset": "seal.example",
                        "technique": "GOLISH-INTEL-DNS",
                        "state": "found",
                        "evidence_ids": [fixture.evidence_id],
                    }],
                    "waves": [{
                        "id": wave.wave.id,
                        "wave_index": wave.wave.wave_index,
                        "asset_count": wave.items.len(),
                        "asset_hash": wave.wave.asset_hash,
                    }]
                }
            }
        }),
    }
}

fn refresh_final_seal_material_hash(input: &mut runtime_memory_tx::FinalizeUnitPassRow) {
    let seal_material = serde_json::json!({
        "canonical_fact_keys": input.canonical_fact_keys,
        "typed_claims": input.typed_claims,
        "coverage_watermark": input.coverage_watermark,
        "evidence_ids": input.evidence_ids,
        "terminal_checkpoint": input.terminal_checkpoint,
        "deterministic_gate_details": input.gate_decision["details"],
        "candidate_acceptance": input.candidate_acceptance,
    });
    input.gate_decision["seal_material_sha256"] = serde_json::json!(sha256_json(&seal_material));
    input.gate_decision_hash = sha256_json(&input.gate_decision);
}

#[tokio::test]
#[serial]
async fn unit_worker_chain_checkpoint_and_tool_fence_are_one_typed_state_machine() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots(&db).await;
    let unit_id = Uuid::new_v4();
    let queued_unit = stage_run_units::insert_with_executor(
        db.pool(),
        &stage_run_units::NewStageRunUnit {
            id: unit_id,
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            scope_snapshot_id: roots.snapshot_id,
            organization_id: roots.organization_id,
            stage_kind: "target_intel".to_string(),
            generation: 0,
            specialist: Some("target_intel".to_string()),
        },
    )
    .await
    .expect("seed queued stage unit");
    let running_unit = stage_run_units::transition_cas(
        db.pool(),
        unit_id,
        roots.operation_id,
        roots.stage_execution_id,
        roots.organization_id,
        stage_run_units::StageRunUnitStatus::Queued,
        queued_unit.row_version,
        stage_run_units::StageRunUnitStatus::Running,
        None,
    )
    .await
    .expect("start exact stage unit");

    let worker_run_id = Uuid::new_v4();
    let queued_worker = stage_worker_runs::insert_with_executor(
        db.pool(),
        &stage_worker_runs::NewStageWorkerRun {
            id: worker_run_id,
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: unit_id,
            work_item_id: None,
            organization_id: roots.organization_id,
            worker_generation: 0,
            specialist: "target_intel".to_string(),
            work_item_kind: "stage_unit".to_string(),
            work_item_key: unit_id.to_string(),
            agent_path: "main>target_intel".to_string(),
            parent_request_id: None,
        },
    )
    .await
    .expect("seed queued worker");
    let lease_token = Uuid::new_v4();
    let claimed = stage_worker_runs::claim_cas(
        db.pool(),
        worker_run_id,
        unit_id,
        stage_worker_runs::StageWorkerRunStatus::Queued,
        queued_worker.attempt_epoch,
        lease_token,
        "test-worker",
        60,
    )
    .await
    .expect("claim worker lease");
    assert_eq!(claimed.attempt_epoch, 1);
    assert_eq!(claimed.lease_token, Some(lease_token));

    let chain_id = Uuid::new_v4();
    message_chains::create_bound_with_executor(
        db.pool(),
        chain_id,
        roots.session_id,
        roots.operation_id,
        None,
        AgentType::Primary,
        None,
        None,
        &serde_json::json!([]),
    )
    .await
    .expect("create provider-safe prebound chain");
    stage_worker_runs::bind_message_chain_cas(
        db.pool(),
        worker_run_id,
        unit_id,
        lease_token,
        claimed.attempt_epoch,
        chain_id,
    )
    .await
    .expect("bind exact worker chain");

    let runtime_identity = tool_calls::RuntimeToolIdentity {
        operation_id: roots.operation_id,
        stage_execution_id: roots.stage_execution_id,
        stage_run_unit_id: Some(unit_id),
        worker_run_id: Some(worker_run_id),
        organization_id: Some(roots.organization_id),
        attempt_epoch: Some(claimed.attempt_epoch),
        lease_token: Some(lease_token),
    };
    let tool_call_id = tool_calls::record_tracked_start(
        db.pool(),
        "worker-tool-request",
        roots.session_id,
        Some(roots.operation_id),
        None,
        "query_target_data",
        &serde_json::json!({"section": "targets"}),
        Some(&runtime_identity),
    )
    .await
    .expect("persist worker-fenced tool call before dispatch");
    let fence = runtime_memory_tx::RuntimeMemoryTxFence {
        operation_id: roots.operation_id,
        stage_execution_id: roots.stage_execution_id,
        stage_run_unit_id: unit_id,
        worker_run_id,
        lease_token,
        attempt_epoch: claimed.attempt_epoch,
        expected_checkpoint_version: claimed.checkpoint_version,
    };
    stage_worker_runs::begin_tool_cas(db.pool(), &fence, tool_call_id)
        .await
        .expect("mark exact active tool");
    stage_worker_runs::finish_tool_cas(db.pool(), &fence, tool_call_id)
        .await
        .expect("clear exact active tool");
    tool_calls::record_tracked_finish(
        db.pool(),
        tool_call_id,
        roots.session_id,
        "finished",
        "{}",
        1,
    )
    .await
    .expect("finish exact tracked tool call");

    let checkpointed =
        stage_worker_runs::checkpoint_cas(db.pool(), &fence, &serde_json::json!({"turn": 1}))
            .await
            .expect("checkpoint exact leased worker");
    assert_eq!(checkpointed.checkpoint_version, 1);
    let stale = stage_worker_runs::checkpoint_cas(
        db.pool(),
        &runtime_memory_tx::RuntimeMemoryTxFence {
            lease_token: Uuid::new_v4(),
            ..fence.clone()
        },
        &serde_json::json!({"turn": 2}),
    )
    .await;
    assert!(matches!(
        stale,
        Err(runtime_memory_tx::RuntimeMemoryStoreError::LeaseLost { .. })
    ));

    let final_fence = runtime_memory_tx::RuntimeMemoryTxFence {
        expected_checkpoint_version: checkpointed.checkpoint_version,
        ..fence
    };
    let direct_worker_pass = stage_worker_runs::finish_attempt_cas(
        db.pool(),
        &final_fence,
        stage_worker_runs::StageWorkerRunStatus::Running,
        stage_worker_runs::StageWorkerRunStatus::Passed,
        &serde_json::json!({"turn": 1, "terminal": true}),
        Some(42),
    )
    .await;
    assert!(matches!(
        direct_worker_pass,
        Err(runtime_memory_tx::RuntimeMemoryStoreError::Conflict {
            code: "worker_pass_requires_final_seal"
        })
    ));

    let direct_unit_pass = stage_run_units::transition_cas(
        db.pool(),
        unit_id,
        roots.operation_id,
        roots.stage_execution_id,
        roots.organization_id,
        stage_run_units::StageRunUnitStatus::Running,
        running_unit.row_version,
        stage_run_units::StageRunUnitStatus::Passed,
        Some(&serde_json::json!({"evidence_watermark": 42})),
    )
    .await;
    assert!(matches!(
        direct_unit_pass,
        Err(runtime_memory_tx::RuntimeMemoryStoreError::Conflict {
            code: "unit_pass_requires_final_seal"
        })
    ));

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn seed_stage_runtime_respects_explicit_organization_subset() {
    let (mut db, _data_dir) = fixture().await;
    let (roots, child_organization_ids) = create_sealed_runtime_roots_with_contract_and_children(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::DualWriteLegacyRead,
        2,
    )
    .await;
    let selected_organization_ids = vec![roots.organization_id, child_organization_ids[1]];
    let seed_input = runtime_memory_tx::SeedStageRuntimeRow {
        operation_id: roots.operation_id,
        stage_execution_id: roots.stage_execution_id,
        stage_kind: "target_intel".to_string(),
        unit_generation: 0,
        specialist: "target_intel".to_string(),
        worker_generation: 0,
        work_item_kind: "stage_unit".to_string(),
        work_item_key: "primary".to_string(),
        agent_path_prefix: "main>stage_run:target_intel".to_string(),
        organization_ids: Some(selected_organization_ids.clone()),
    };

    let seeded = runtime_memory_tx::seed_stage_runtime(db.pool(), &seed_input)
        .await
        .expect("seed only explicitly selected frozen organizations");
    let mut seeded_organization_ids = seeded
        .iter()
        .map(|row| row.unit.organization_id)
        .collect::<Vec<_>>();
    seeded_organization_ids.sort_unstable();
    let mut expected_organization_ids = selected_organization_ids.clone();
    expected_organization_ids.sort_unstable();
    assert_eq!(seeded_organization_ids, expected_organization_ids);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM stage_run_units WHERE operation_id=$1 AND stage_execution_id=$2"
        )
        .bind(roots.operation_id)
        .bind(roots.stage_execution_id)
        .fetch_one(db.pool())
        .await
        .expect("count subset runtime units"),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM stage_worker_runs WHERE operation_id=$1 AND stage_execution_id=$2"
        )
        .bind(roots.operation_id)
        .bind(roots.stage_execution_id)
        .fetch_one(db.pool())
        .await
        .expect("count subset runtime workers"),
        2
    );

    let mut duplicate = seed_input.clone();
    duplicate.organization_ids = Some(vec![roots.organization_id, roots.organization_id]);
    assert!(matches!(
        runtime_memory_tx::seed_stage_runtime(db.pool(), &duplicate).await,
        Err(
            runtime_memory_tx::RuntimeMemoryStoreError::IdentityMismatch {
                code: "duplicate_stage_runtime_seed_organization"
            }
        )
    ));

    let mut empty = seed_input.clone();
    empty.organization_ids = Some(Vec::new());
    assert!(matches!(
        runtime_memory_tx::seed_stage_runtime(db.pool(), &empty).await,
        Err(
            runtime_memory_tx::RuntimeMemoryStoreError::IdentityMismatch {
                code: "empty_stage_runtime_seed_organizations"
            }
        )
    ));

    let mut outside_scope = seed_input.clone();
    outside_scope.organization_ids = Some(vec![Uuid::new_v4()]);
    assert!(matches!(
        runtime_memory_tx::seed_stage_runtime(db.pool(), &outside_scope).await,
        Err(
            runtime_memory_tx::RuntimeMemoryStoreError::IdentityMismatch {
                code: "stage_runtime_seed_organization_outside_frozen_scope"
            }
        )
    ));

    let replayed = runtime_memory_tx::seed_stage_runtime(db.pool(), &seed_input)
        .await
        .expect("exact subset seed replay is idempotent");
    assert_eq!(
        replayed.iter().map(|row| row.unit.id).collect::<Vec<_>>(),
        seeded.iter().map(|row| row.unit.id).collect::<Vec<_>>()
    );

    let mut all_organizations = seed_input;
    all_organizations.organization_ids = None;
    let seeded_all = runtime_memory_tx::seed_stage_runtime(db.pool(), &all_organizations)
        .await
        .expect("None preserves all-frozen-organization seeding");
    assert_eq!(seeded_all.len(), 3);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM stage_run_units WHERE operation_id=$1 AND stage_execution_id=$2"
        )
        .bind(roots.operation_id)
        .bind(roots.stage_execution_id)
        .fetch_one(db.pool())
        .await
        .expect("count all runtime units after default seed"),
        3
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn compound_seed_claim_and_checkpoint_keep_v2_and_legacy_mirror_atomic() {
    let (mut db, _data_dir) = fixture().await;
    let roots = create_sealed_runtime_roots(&db).await;

    let seed_input = runtime_memory_tx::SeedStageRuntimeRow {
        operation_id: roots.operation_id,
        stage_execution_id: roots.stage_execution_id,
        stage_kind: "target_intel".to_string(),
        unit_generation: 0,
        specialist: "target_intel".to_string(),
        worker_generation: 0,
        work_item_kind: "stage_unit".to_string(),
        work_item_key: "primary".to_string(),
        agent_path_prefix: "main>stage_run:target_intel".to_string(),
        organization_ids: None,
    };
    let seeded = runtime_memory_tx::seed_stage_runtime(db.pool(), &seed_input)
        .await
        .expect("seed exact scope units and primary workers");
    assert_eq!(seeded.len(), 1);
    let seeded = &seeded[0];
    assert_eq!(seeded.unit.organization_id, roots.organization_id);
    assert_eq!(seeded.unit.status, "queued");
    assert_eq!(seeded.worker.status, "queued");
    let replayed = runtime_memory_tx::seed_stage_runtime(db.pool(), &seed_input)
        .await
        .expect("exact seed replay is idempotent");
    assert_eq!(replayed[0].unit.id, seeded.unit.id);
    assert_eq!(replayed[0].worker.id, seeded.worker.id);

    let claimed = runtime_memory_tx::claim_worker_and_bind_chain(
        db.pool(),
        &runtime_memory_tx::ClaimWorkerAndBindChainRow {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: seeded.unit.id,
            worker_run_id: seeded.worker.id,
            expected_unit_status: stage_run_units::StageRunUnitStatus::Queued,
            expected_unit_row_version: seeded.unit.row_version,
            expected_worker_status: stage_worker_runs::StageWorkerRunStatus::Queued,
            expected_attempt_epoch: seeded.worker.attempt_epoch,
            session_id: roots.session_id,
            subtask_id: None,
            agent: AgentType::Primary,
            model: Some("fixture-model".to_string()),
            provider: Some("fixture-provider".to_string()),
            parent_chain_id: None,
            lease_owner: "fixture-worker".to_string(),
            lease_seconds: 60,
            initial_chain: serde_json::json!([]),
            initial_checkpoint: serde_json::json!({"turn": 0}),
        },
    )
    .await
    .expect("claim worker and commit its provider-safe chain before dispatch");
    assert_eq!(claimed.unit.status, "running");
    assert_eq!(claimed.worker.status, "running");
    assert_eq!(
        claimed.worker.message_chain_id,
        Some(claimed.message_chain_id)
    );
    assert_eq!(claimed.worker.checkpoint, serde_json::json!({"turn": 0}));

    let fence = runtime_memory_tx::RuntimeMemoryTxFence {
        operation_id: roots.operation_id,
        stage_execution_id: roots.stage_execution_id,
        stage_run_unit_id: seeded.unit.id,
        worker_run_id: seeded.worker.id,
        lease_token: claimed.worker.lease_token.expect("claimed lease token"),
        attempt_epoch: claimed.worker.attempt_epoch,
        expected_checkpoint_version: claimed.worker.checkpoint_version,
    };
    let checkpointed = runtime_memory_tx::checkpoint_worker(
        db.pool(),
        &fence,
        &serde_json::json!({"turn": 1, "provider_safe": true}),
    )
    .await
    .expect("checkpoint exact leased worker with atomic legacy mirror");
    assert_eq!(
        checkpointed.checkpoint_version,
        fence.expected_checkpoint_version + 1
    );

    let operation = golish_db::repo::operation_state::get(db.pool(), roots.operation_id)
        .await
        .expect("read dual-write operation")
        .expect("operation exists");
    let mirror = &operation.state_blob["stage_run_workers"]["target_intel"]
        [&roots.organization_id.to_string()];
    assert_eq!(mirror["stage_run_unit_id"], seeded.unit.id.to_string());
    assert_eq!(mirror["worker_run_id"], seeded.worker.id.to_string());
    assert_eq!(mirror["chain_id"], claimed.message_chain_id.to_string());
    assert_eq!(mirror["checkpoint"], checkpointed.checkpoint);
    assert_eq!(
        mirror["checkpoint_version"],
        checkpointed.checkpoint_version
    );
    let loaded = runtime_memory_tx::load_worker_checkpoint(
        db.pool(),
        &runtime_memory_tx::LoadWorkerCheckpointRow {
            operation_id: roots.operation_id,
            stage_execution_id: roots.stage_execution_id,
            stage_run_unit_id: seeded.unit.id,
            worker_run_id: seeded.worker.id,
            selected_source: None,
        },
    )
    .await
    .expect("dual-write legacy-read selects one complete legacy record");
    assert_eq!(
        loaded.source,
        runtime_memory_tx::RuntimeMemoryRecordSource::Legacy
    );
    assert_eq!(loaded.worker.checkpoint, checkpointed.checkpoint);
    assert_eq!(
        loaded.worker.message_chain_id,
        Some(claimed.message_chain_id)
    );
    let samples: Vec<(String, String)> = sqlx::query_as(
        r#"SELECT mutation_kind,comparison
             FROM runtime_memory_shadow_samples
            WHERE worker_run_id=$1
            ORDER BY sample_seq"#,
    )
    .bind(seeded.worker.id)
    .fetch_all(db.pool())
    .await
    .expect("load retained dual-mutation observations");
    assert_eq!(
        samples,
        vec![
            ("seed".to_string(), "match".to_string()),
            ("seed".to_string(), "match".to_string()),
            ("claim_and_bind_chain".to_string(), "match".to_string()),
            ("checkpoint".to_string(), "match".to_string()),
        ],
        "every production dual mirror boundary retains its whole-record comparison"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn compound_checkpoint_rolls_back_v2_when_legacy_mirror_write_fails() {
    let (mut db, _data_dir) = fixture().await;
    let runtime = create_claimed_compound_runtime(&db).await;
    let fence = fence_for_claimed(&runtime);
    let samples_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM runtime_memory_shadow_samples WHERE worker_run_id=$1",
    )
    .bind(runtime.worker_id)
    .fetch_one(db.pool())
    .await
    .expect("count samples before immediate failure");
    sqlx::query(
        r#"CREATE FUNCTION reject_runtime_legacy_mirror() RETURNS trigger AS $$
           BEGIN
             RAISE EXCEPTION 'injected legacy mirror failure';
           END;
           $$ LANGUAGE plpgsql"#,
    )
    .execute(db.pool())
    .await
    .expect("install immediate mirror failure function");
    sqlx::query(
        r#"CREATE TRIGGER reject_runtime_legacy_mirror
           BEFORE UPDATE OF state_blob ON operation_state
           FOR EACH ROW EXECUTE FUNCTION reject_runtime_legacy_mirror()"#,
    )
    .execute(db.pool())
    .await
    .expect("install immediate mirror failure trigger");

    let result =
        runtime_memory_tx::checkpoint_worker(db.pool(), &fence, &serde_json::json!({"turn": 1}))
            .await;
    assert!(result.is_err());
    let worker = stage_worker_runs::get(db.pool(), runtime.worker_id)
        .await
        .expect("reload worker after rollback")
        .expect("worker remains");
    assert_eq!(worker.checkpoint, serde_json::json!({"turn": 0}));
    assert_eq!(worker.checkpoint_version, fence.expected_checkpoint_version);
    let samples_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM runtime_memory_shadow_samples WHERE worker_run_id=$1",
    )
    .bind(runtime.worker_id)
    .fetch_one(db.pool())
    .await
    .expect("count samples after immediate rollback");
    assert_eq!(samples_after, samples_before);

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn compound_checkpoint_rolls_back_both_sources_when_commit_fails() {
    let (mut db, _data_dir) = fixture().await;
    let runtime = create_claimed_compound_runtime(&db).await;
    let fence = fence_for_claimed(&runtime);
    let samples_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM runtime_memory_shadow_samples WHERE worker_run_id=$1",
    )
    .bind(runtime.worker_id)
    .fetch_one(db.pool())
    .await
    .expect("count samples before deferred failure");
    let before = golish_db::repo::operation_state::get(db.pool(), runtime.roots.operation_id)
        .await
        .expect("load mirror before deferred failure")
        .expect("operation exists")
        .state_blob;
    sqlx::query(
        r#"CREATE FUNCTION reject_runtime_commit() RETURNS trigger AS $$
           BEGIN
             RAISE EXCEPTION 'injected deferred runtime commit failure';
           END;
           $$ LANGUAGE plpgsql"#,
    )
    .execute(db.pool())
    .await
    .expect("install deferred commit failure function");
    sqlx::query(
        r#"CREATE CONSTRAINT TRIGGER reject_runtime_commit
           AFTER UPDATE ON operation_state
           DEFERRABLE INITIALLY DEFERRED
           FOR EACH ROW EXECUTE FUNCTION reject_runtime_commit()"#,
    )
    .execute(db.pool())
    .await
    .expect("install deferred commit failure trigger");

    let result =
        runtime_memory_tx::checkpoint_worker(db.pool(), &fence, &serde_json::json!({"turn": 1}))
            .await;
    assert!(result.is_err());
    let worker = stage_worker_runs::get(db.pool(), runtime.worker_id)
        .await
        .expect("reload worker after deferred rollback")
        .expect("worker remains");
    assert_eq!(worker.checkpoint, serde_json::json!({"turn": 0}));
    assert_eq!(worker.checkpoint_version, fence.expected_checkpoint_version);
    let after = golish_db::repo::operation_state::get(db.pool(), runtime.roots.operation_id)
        .await
        .expect("reload mirror after deferred rollback")
        .expect("operation exists")
        .state_blob;
    assert_eq!(after, before, "legacy mirror must roll back at commit too");
    let samples_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM runtime_memory_shadow_samples WHERE worker_run_id=$1",
    )
    .bind(runtime.worker_id)
    .fetch_one(db.pool())
    .await
    .expect("count samples after deferred rollback");
    assert_eq!(samples_after, samples_before);

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn startup_reaper_reconciles_committed_dual_sample_and_replay_is_idempotent() {
    let (mut db, _data_dir) = fixture().await;
    let runtime = create_claimed_compound_runtime(&db).await;

    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET lease_acquired_at=NOW()-INTERVAL '2 hours',
                  lease_expires_at=NOW()-INTERVAL '1 hour',
                  heartbeat_at=NOW()-INTERVAL '1 hour'
            WHERE id=$1"#,
    )
    .bind(runtime.worker_id)
    .execute(db.pool())
    .await
    .expect("expire claimed worker lease");
    sqlx::query(
        r#"UPDATE tasks
              SET status='running',updated_at=NOW()-INTERVAL '2 hours'
            WHERE id=$1"#,
    )
    .bind(runtime.roots.operation_id)
    .execute(db.pool())
    .await
    .expect("age runtime task for startup reaper");

    // Earlier dual mutations may already have reconciled the deployment row.
    // Give this response-loss fixture a fresh row version at sampling rank so
    // only the startup reaper can produce the transition under assertion.
    let mut rollout_fixture = db.pool().begin().await.expect("begin rollout fixture");
    sqlx::raw_sql(
        r#"ALTER TABLE runtime_memory_rollout
               DISABLE TRIGGER runtime_memory_rollout_forward_only;
           ALTER TABLE runtime_memory_rollout
               DISABLE TRIGGER zz_runtime_memory_rollout_attestation_gate;
           ALTER TABLE runtime_memory_rollout
               DISABLE TRIGGER zz_runtime_memory_rollout_promotion_receipt;
           ALTER TABLE runtime_memory_rollout_promotions
               DISABLE TRIGGER runtime_memory_rollout_promotion_receipt_immutable;
           DELETE FROM runtime_memory_rollout_promotions WHERE from_rank=1;
           UPDATE runtime_memory_rollout
              SET contract='dual_write_legacy_read',contract_rank=1,
                  row_version=101,updated_at=NOW()
            WHERE singleton_id=1;
           ALTER TABLE runtime_memory_rollout_promotions
               ENABLE TRIGGER runtime_memory_rollout_promotion_receipt_immutable;
           ALTER TABLE runtime_memory_rollout
               ENABLE TRIGGER runtime_memory_rollout_forward_only;
           ALTER TABLE runtime_memory_rollout
               ENABLE TRIGGER zz_runtime_memory_rollout_attestation_gate;
           ALTER TABLE runtime_memory_rollout
               ENABLE TRIGGER zz_runtime_memory_rollout_promotion_receipt;"#,
    )
    .execute(&mut *rollout_fixture)
    .await
    .expect("install fresh sampling-rank rollout fixture");
    rollout_fixture
        .commit()
        .await
        .expect("commit rollout fixture");

    let samples_before: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM runtime_memory_shadow_samples
            WHERE worker_run_id=$1 AND mutation_kind='startup_reaper'"#,
    )
    .bind(runtime.worker_id)
    .fetch_one(db.pool())
    .await
    .expect("count startup samples before reaping");

    let first = tasks::startup_reap_abandoned(db.pool(), chrono::Duration::zero())
        .await
        .expect("commit startup reaper and reconcile deployment rollouts");
    assert_eq!(first.workers_requeued, 1);
    assert_eq!(first.workers_recovery_required, 0);
    assert_eq!(first.runtime_shadow_samples_written, 1);
    let promoted = runtime_memory_rollout::get(db.pool())
        .await
        .expect("read rollout after startup reaper commit");
    assert_eq!(promoted.contract, "dual_write_v2_preferred");
    assert_eq!(promoted.contract_rank, 2);
    assert_eq!(promoted.row_version, 102);
    let samples_after: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM runtime_memory_shadow_samples
            WHERE worker_run_id=$1 AND mutation_kind='startup_reaper'"#,
    )
    .bind(runtime.worker_id)
    .fetch_one(db.pool())
    .await
    .expect("count committed startup samples");
    assert_eq!(samples_after, samples_before + 1);
    let receipts_after_first: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM runtime_memory_rollout_promotions
            WHERE from_row_version=101 AND to_row_version=102"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("count first startup promotion receipt");
    assert_eq!(receipts_after_first, 1);

    // Simulate the caller losing the first response and replaying startup.
    let replay = tasks::startup_reap_abandoned(db.pool(), chrono::Duration::zero())
        .await
        .expect("replay startup after response loss");
    assert_eq!(replay.workers_requeued, 0);
    assert_eq!(replay.workers_recovery_required, 0);
    assert_eq!(replay.runtime_shadow_samples_written, 0);
    let replayed_rollout = runtime_memory_rollout::get(db.pool())
        .await
        .expect("read rollout after replay");
    assert_eq!(replayed_rollout, promoted);
    let receipts_after_replay: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM runtime_memory_rollout_promotions
            WHERE from_row_version=101 AND to_row_version=102"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("count startup promotion receipt after replay");
    assert_eq!(receipts_after_replay, 1);

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn exhaustive_eas_port_producer_is_reserved_once_per_exact_epoch_manifest() {
    let (mut db, _data_dir) = fixture().await;
    let runtime = create_claimed_compound_runtime_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    operation_state::advance_stage(
        db.pool(),
        runtime.roots.operation_id,
        "external_attack_surface",
    )
    .await
    .expect("move fixture to active EAS");
    let epoch = operation_state::get_epoch(db.pool(), runtime.roots.operation_id)
        .await
        .expect("read operation epoch")
        .expect("operation remains");
    let input = operation_state::EasPortScanAttemptInput {
        operation_id: runtime.roots.operation_id,
        stage_started_at: epoch.stage_started_at,
        slot_key: "exact-port-manifest".to_string(),
        organization_id: runtime.roots.organization_id,
        target_ids_sha256: "sha256:target-members".to_string(),
        profile_version: 3,
        target_manifest_sha256: "sha256:port-manifest".to_string(),
        producer_deadline_secs: 300,
    };

    let first = operation_state::reserve_eas_port_scan_attempt(db.pool(), &input)
        .await
        .expect("reserve exhaustive producer before network");
    assert_eq!(
        first,
        Some(operation_state::EasPortScanAttemptReservation::Reserved { attempt: 1 })
    );
    let replay = operation_state::reserve_eas_port_scan_attempt(db.pool(), &input)
        .await
        .expect("response-loss replay reads exhausted authority");
    assert_eq!(
        replay,
        Some(operation_state::EasPortScanAttemptReservation::Exhausted { attempts: 1 })
    );

    let mut foreign = input.clone();
    foreign.target_manifest_sha256 = "sha256:foreign-manifest".to_string();
    let error = operation_state::reserve_eas_port_scan_attempt(db.pool(), &foreign)
        .await
        .expect_err("same slot with foreign authority must fail closed");
    assert!(
        error
            .to_string()
            .contains("EAS_PORT_SCAN_ATTEMPT_AUTHORITY_MISMATCH"),
        "unexpected error: {error}"
    );

    let stored = operation_state::get(db.pool(), runtime.roots.operation_id)
        .await
        .expect("load operation state")
        .expect("operation remains");
    assert_eq!(
        stored.state_blob["eas_port_scan_attempts"]["exact-port-manifest"]["attempts"],
        1
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn v2_only_developer_reset_ignores_caller_legacy_checkpoint_namespaces() {
    let (mut db, _data_dir) = fixture().await;
    let runtime = create_claimed_compound_runtime_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;

    operation_state::advance_stage(
        db.pool(),
        runtime.roots.operation_id,
        "external_attack_surface",
    )
    .await
    .expect("move fixture to the EAS server namespace owner");
    let eas_epoch = operation_state::get(db.pool(), runtime.roots.operation_id)
        .await
        .expect("load EAS epoch")
        .expect("operation remains");
    let attempts = operation_state::increment_eas_web_transport_failure(
        db.pool(),
        &operation_state::EasWebTransportFailureInput {
            operation_id: runtime.roots.operation_id,
            stage_started_at: eas_epoch.stage_started_at,
            slot_key: "reserved-slot".to_string(),
            organization_id: runtime.roots.organization_id,
            target_id: Uuid::new_v4(),
            origin: "https://example.test".to_string(),
            technique: "GOLISH-EAS-WEB-FINGERPRINT".to_string(),
            failure_class: "timeout".to_string(),
        },
    )
    .await
    .expect("dedicated EAS namespace writer remains allowed");
    assert_eq!(attempts, Some(1));
    operation_state::advance_stage(db.pool(), runtime.roots.operation_id, "target_intel")
        .await
        .expect("restore relational stage for developer reset");

    let replacement_stage_execution_id = Uuid::new_v4();
    runtime_memory_tx::supersede_stage_checkpoint(
        db.pool(),
        &runtime_memory_tx::SupersedeStageCheckpointRow {
            operation_id: runtime.roots.operation_id,
            expected_active_stage_execution_id: None,
            expected_current_stage: "target_intel".to_string(),
            selected_stage: "target_intel".to_string(),
            affected_stage_kinds: vec!["target_intel".to_string()],
            next_state_blob: serde_json::json!({
                "caller_only": "must-not-cross-v2-boundary",
                "graph_flow": {"next_node": "caller-injected"},
                "profile": "caller-profile",
                "current_stage": "caller-stage",
                "current_stage_run_id": Uuid::new_v4(),
                "queue_titles": ["caller-queue"],
                "completed_count": 99,
                "continuity_adoption": {"caller": true},
                "schema_v": 1,
                "stage_run_workers": {"target_intel": {"caller": true}},
                "stage_run_handoffs": {"target_intel": {"caller": true}},
                "agent_run": {"caller": true}
            }),
            replacement_specialist: Some("target_intel".to_string()),
            replacement_stage_execution_id: Some(replacement_stage_execution_id),
            fact_purge: None,
            finalizer_recovery_witness: None,
        },
    )
    .await
    .expect("reset V2-only runtime from relational truth");

    let reset = operation_state::get(db.pool(), runtime.roots.operation_id)
        .await
        .expect("load reset operation")
        .expect("reset operation remains");
    assert_eq!(
        reset.state_blob["eas_web_transport_failures"]["reserved-slot"]["attempts"], 1,
        "server-owned non-checkpoint state must survive the reset"
    );
    for forbidden in [
        "caller_only",
        "graph_flow",
        "profile",
        "current_stage",
        "current_stage_run_id",
        "queue_titles",
        "completed_count",
        "continuity_adoption",
        "schema_v",
        "stage_run_workers",
        "stage_run_handoffs",
        "agent_run",
    ] {
        assert!(
            reset.state_blob.get(forbidden).is_none(),
            "V2-only reset retained forbidden checkpoint namespace {forbidden}: {}",
            reset.state_blob
        );
    }
    assert_eq!(
        reset.state_blob["runtime_v2_dev_reset"]["replacement_stage_execution_id"],
        serde_json::json!(replacement_stage_execution_id),
        "only the server-authored reset marker may describe the replacement"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn v2_only_company_stage_reset_rejects_active_tool_until_exact_finish() {
    let (mut db, _data_dir) = fixture().await;
    let runtime = create_claimed_compound_runtime_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    let tool_call_id = tool_calls::record_tracked_start(
        db.pool(),
        "reset-active-company-stage-tool",
        runtime.roots.session_id,
        Some(runtime.roots.operation_id),
        None,
        "intel_search",
        &serde_json::json!({"query": "authorized fixture"}),
        Some(&tool_calls::RuntimeToolIdentity {
            operation_id: runtime.roots.operation_id,
            stage_execution_id: runtime.roots.stage_execution_id,
            stage_run_unit_id: Some(runtime.unit_id),
            worker_run_id: Some(runtime.worker_id),
            organization_id: Some(runtime.roots.organization_id),
            attempt_epoch: Some(runtime.worker.attempt_epoch),
            lease_token: runtime.worker.lease_token,
        }),
    )
    .await
    .expect("record reset fixture tool");
    runtime_memory_tx::begin_worker_tool(db.pool(), &fence_for_claimed(&runtime), tool_call_id)
        .await
        .expect("bind reset fixture tool");

    let active_tool_lease = runtime.worker.lease_token.expect("claimed tool lease");
    let mismatched_worker_lease = Uuid::new_v4();
    sqlx::query("UPDATE stage_worker_runs SET lease_token=$2 WHERE id=$1")
        .bind(runtime.worker_id)
        .bind(mismatched_worker_lease)
        .execute(db.pool())
        .await
        .expect("inject worker/tool lease mismatch");
    let mismatch = runtime_memory_tx::supersede_stage_checkpoint(
        db.pool(),
        &runtime_memory_tx::SupersedeStageCheckpointRow {
            operation_id: runtime.roots.operation_id,
            expected_active_stage_execution_id: None,
            expected_current_stage: "target_intel".to_string(),
            selected_stage: "vuln_triage".to_string(),
            affected_stage_kinds: vec!["target_intel".to_string(), "vuln_triage".to_string()],
            next_state_blob: serde_json::json!({}),
            replacement_specialist: Some("vuln_scanner".to_string()),
            replacement_stage_execution_id: Some(Uuid::new_v4()),
            fact_purge: None,
            finalizer_recovery_witness: None,
        },
    )
    .await
    .expect_err("stale tool lease must abort the whole reset transaction");
    assert!(matches!(
        mismatch,
        runtime_memory_tx::RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_checkpoint_reset_active_tool_identity_mismatch"
        }
    ));
    let unchanged_worker = stage_worker_runs::get(db.pool(), runtime.worker_id)
        .await
        .expect("load worker after rejected reset")
        .expect("worker remains");
    assert_eq!(unchanged_worker.status, "running");
    assert_eq!(unchanged_worker.active_tool_call_id, Some(tool_call_id));
    assert_eq!(unchanged_worker.lease_token, Some(mismatched_worker_lease));
    assert_eq!(
        stage_runs::list_active_for_operation_with_executor(db.pool(), runtime.roots.operation_id,)
            .await
            .expect("load execution after rejected reset")
            .iter()
            .map(|run| run.id)
            .collect::<Vec<_>>(),
        vec![runtime.roots.stage_execution_id]
    );
    sqlx::query("UPDATE stage_worker_runs SET lease_token=$2 WHERE id=$1")
        .bind(runtime.worker_id)
        .bind(active_tool_lease)
        .execute(db.pool())
        .await
        .expect("restore exact worker/tool lease identity");

    let active_tool_reset = runtime_memory_tx::supersede_stage_checkpoint(
        db.pool(),
        &runtime_memory_tx::SupersedeStageCheckpointRow {
            operation_id: runtime.roots.operation_id,
            expected_active_stage_execution_id: None,
            expected_current_stage: "target_intel".to_string(),
            selected_stage: "vuln_triage".to_string(),
            affected_stage_kinds: vec!["target_intel".to_string(), "vuln_triage".to_string()],
            next_state_blob: serde_json::json!({}),
            replacement_specialist: Some("vuln_scanner".to_string()),
            replacement_stage_execution_id: Some(Uuid::new_v4()),
            fact_purge: None,
            finalizer_recovery_witness: None,
        },
    )
    .await
    .expect_err("an external tool may still land facts after reset commit");
    assert!(matches!(
        active_tool_reset,
        runtime_memory_tx::RuntimeMemoryStoreError::Conflict {
            code: "stage_checkpoint_reset_active_tool_in_flight"
        }
    ));
    assert_eq!(
        stage_runs::list_active_for_operation_with_executor(db.pool(), runtime.roots.operation_id,)
            .await
            .expect("load execution after active-tool rejection")
            .iter()
            .map(|run| run.id)
            .collect::<Vec<_>>(),
        vec![runtime.roots.stage_execution_id]
    );
    runtime_memory_tx::finish_worker_tool(db.pool(), &fence_for_claimed(&runtime), tool_call_id)
        .await
        .expect("clear exact completed tool fence before reset");
    tool_calls::record_tracked_finish(
        db.pool(),
        tool_call_id,
        runtime.roots.session_id,
        "finished",
        "{}",
        1,
    )
    .await
    .expect("record exact completed tool before reset");

    let replacement_stage_execution_id = Uuid::new_v4();
    runtime_memory_tx::supersede_stage_checkpoint(
        db.pool(),
        &runtime_memory_tx::SupersedeStageCheckpointRow {
            operation_id: runtime.roots.operation_id,
            expected_active_stage_execution_id: None,
            expected_current_stage: "target_intel".to_string(),
            selected_stage: "vuln_triage".to_string(),
            affected_stage_kinds: vec!["target_intel".to_string(), "vuln_triage".to_string()],
            next_state_blob: serde_json::json!({}),
            replacement_specialist: Some("vuln_scanner".to_string()),
            replacement_stage_execution_id: Some(replacement_stage_execution_id),
            fact_purge: None,
            finalizer_recovery_witness: None,
        },
    )
    .await
    .expect("reset V2-only Company stage");

    let replacement_roots = RuntimeRoots {
        session_id: runtime.roots.session_id,
        operation_id: runtime.roots.operation_id,
        stage_execution_id: replacement_stage_execution_id,
        snapshot_id: runtime.roots.snapshot_id,
        organization_id: runtime.roots.organization_id,
    };
    let mut team_seed = stage_team_controller_seed(&replacement_roots);
    team_seed.base.stage_kind = "vuln_triage".to_string();
    team_seed.base.unit_generation = 1;
    team_seed.base.specialist = "vuln_scanner".to_string();
    team_seed.base.worker_generation = 1;
    team_seed.base.work_item_kind = "organization".to_string();
    team_seed.base.work_item_key = "vuln_triage".to_string();
    team_seed.base.agent_path_prefix = "main>stage_run:vuln_triage".to_string();
    team_seed.plan.allowed_roles = vec![
        "company_stage_controller".to_string(),
        "vuln_scanner".to_string(),
    ];
    let seeded = runtime_memory_tx::seed_stage_team_runtime(db.pool(), &team_seed)
        .await
        .expect("canonical Company Controller Team seed must follow reset")
        .remove(0);

    let workers_before_claim = stage_worker_runs::list_for_execution(
        db.pool(),
        runtime.roots.operation_id,
        replacement_stage_execution_id,
    )
    .await
    .expect("load replacement workers before claim");
    assert!(
        workers_before_claim.is_empty(),
        "Team seed must remain Worker-free until a WorkItem is claimed"
    );
    let superseded_worker = stage_worker_runs::get(db.pool(), runtime.worker_id)
        .await
        .expect("load superseded worker")
        .expect("superseded worker remains as history");
    assert_eq!(superseded_worker.status, "superseded");
    assert_eq!(superseded_worker.active_tool_call_id, None);
    assert_eq!(superseded_worker.active_tool_started_at, None);
    let completed_tool = tool_calls::get(db.pool(), tool_call_id)
        .await
        .expect("load completed tool after reset")
        .expect("tool history remains");
    assert_eq!(completed_tool.status, ToolcallStatus::Finished);

    let controller = runtime_memory_tx::claim_stage_team_leader(
        db.pool(),
        &runtime_memory_tx::ClaimStageTeamLeaderRow {
            claim: stage_team_claim_input(&replacement_roots, &seeded, "reset-controller"),
        },
    )
    .await
    .expect("claim Controller after reset")
    .expect("reset Controller WorkItem is runnable");
    assert_eq!(controller.worker.specialist, "company_stage_controller");
    assert_eq!(controller.work_item.stable_key, "leader:primary");
    assert_eq!(
        controller.worker.work_item_id,
        Some(controller.work_item.id)
    );
    let workers_after_claim = stage_worker_runs::list_for_execution(
        db.pool(),
        runtime.roots.operation_id,
        replacement_stage_execution_id,
    )
    .await
    .expect("load replacement workers after claim");
    assert_eq!(workers_after_claim.len(), 1);

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn v2_only_state_blob_rejects_legacy_checkpoint_writes_at_repo_and_raw_sql() {
    let (mut db, _data_dir) = fixture().await;
    let runtime = create_claimed_compound_runtime_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only,
    )
    .await;
    let operation_id = runtime.roots.operation_id;
    let before = operation_state::get(db.pool(), operation_id)
        .await
        .expect("load clean V2-only state")
        .expect("operation remains")
        .state_blob;

    operation_state::write_state_blob(
        db.pool(),
        operation_id,
        serde_json::json!({
            "graph_flow": {"next_node": "reporting", "state": {}},
            "agent_run": {"status": "running"},
            "stage_run_workers": {"hostile": true}
        }),
    )
    .await
    .expect("generic repository checkpoint is an idempotent no-op for V2-only");
    assert_eq!(
        operation_state::get(db.pool(), operation_id)
            .await
            .expect("reload after generic checkpoint")
            .expect("operation remains")
            .state_blob,
        before
    );

    let raw_error = sqlx::query(
        r#"UPDATE operation_state
              SET state_blob=state_blob || jsonb_build_object(
                    'graph_flow',jsonb_build_object('next_node','reporting'),
                    'agent_run',jsonb_build_object('status','running')
                  )
            WHERE operation_id=$1"#,
    )
    .bind(operation_id)
    .execute(db.pool())
    .await
    .expect_err("raw DML cannot recreate a V2-only legacy checkpoint");
    assert!(
        raw_error
            .to_string()
            .contains("V2_ONLY_LEGACY_CHECKPOINT_FORBIDDEN"),
        "unexpected raw checkpoint error: {raw_error}"
    );
    assert_eq!(
        operation_state::get(db.pool(), operation_id)
            .await
            .expect("reload after rejected raw checkpoint")
            .expect("operation remains")
            .state_blob,
        before
    );

    let inserted_operation_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO tasks(id,session_id,title,input,status)
           VALUES($1,$2,'hostile V2 insert','must reject checkpoint','running')"#,
    )
    .bind(inserted_operation_id)
    .bind(runtime.roots.session_id)
    .execute(db.pool())
    .await
    .expect("insert task prerequisite for hostile operation");
    let insert_error = sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,
               attack_execution_contract,state_blob
           ) VALUES(
               $1,'red_team','verification','v2_only','v2_only',
               jsonb_build_object(
                   'graph_flow',jsonb_build_object('next_node','reporting'),
                   'agent_run',jsonb_build_object('status','running')
               )
           )"#,
    )
    .bind(inserted_operation_id)
    .execute(db.pool())
    .await
    .expect_err("raw INSERT cannot create a V2-only legacy checkpoint");
    assert!(
        insert_error
            .to_string()
            .contains("V2_ONLY_LEGACY_CHECKPOINT_FORBIDDEN"),
        "unexpected raw insert error: {insert_error}"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn v2_preferred_selects_one_complete_record_then_whole_legacy_fallback() {
    let (mut db, _data_dir) = fixture().await;
    let runtime = create_claimed_compound_runtime_with_contract(
        &db,
        runtime_memory_rollout::RuntimeMemoryContract::DualWriteV2Preferred,
    )
    .await;
    let input = runtime_memory_tx::LoadWorkerCheckpointRow {
        operation_id: runtime.roots.operation_id,
        stage_execution_id: runtime.roots.stage_execution_id,
        stage_run_unit_id: runtime.unit_id,
        worker_run_id: runtime.worker_id,
        selected_source: None,
    };

    let authoritative = runtime_memory_tx::load_worker_checkpoint(db.pool(), &input)
        .await
        .expect("complete V2 record is authoritative");
    assert_eq!(
        authoritative.source,
        runtime_memory_tx::RuntimeMemoryRecordSource::V2
    );
    assert_eq!(authoritative.worker.id, runtime.worker_id);

    let retained_delete = sqlx::query("DELETE FROM stage_worker_runs WHERE id=$1")
        .bind(runtime.worker_id)
        .execute(db.pool())
        .await;
    assert!(
        retained_delete.is_err(),
        "a rollout admission must normally retain its V2 identity"
    );
    // Selector fallback still needs a hostile-corruption fixture. Temporarily
    // suppress FK enforcement in this disposable embedded database only; the
    // preceding assertion proves production SQL cannot create this state.
    let mut corruption = db.pool().begin().await.expect("begin corruption fixture");
    sqlx::raw_sql("ALTER TABLE stage_worker_runs DISABLE TRIGGER ALL")
        .execute(&mut *corruption)
        .await
        .expect("disable FK triggers in hostile fixture");
    sqlx::query("DELETE FROM stage_worker_runs WHERE id=$1")
        .bind(runtime.worker_id)
        .execute(&mut *corruption)
        .await
        .expect("simulate an entirely missing V2 record");
    sqlx::raw_sql("ALTER TABLE stage_worker_runs ENABLE TRIGGER ALL")
        .execute(&mut *corruption)
        .await
        .expect("restore FK triggers after hostile fixture");
    corruption
        .commit()
        .await
        .expect("commit hostile selector fixture");
    let fallback = runtime_memory_tx::load_worker_checkpoint(db.pool(), &input)
        .await
        .expect("whole legacy record is the only permitted fallback");
    assert_eq!(
        fallback.source,
        runtime_memory_tx::RuntimeMemoryRecordSource::LegacyFallback
    );
    assert_eq!(fallback.worker.id, runtime.worker_id);
    assert_eq!(fallback.worker.checkpoint, runtime.worker.checkpoint);

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn bound_chain_body_and_worker_checkpoint_commit_or_roll_back_together() {
    let (mut db, _data_dir) = fixture().await;
    let runtime = create_claimed_compound_runtime(&db).await;
    let fence = fence_for_claimed(&runtime);
    let chain_id = runtime
        .worker
        .message_chain_id
        .expect("claimed worker has prebound chain");
    let chain = serde_json::json!([{"role": "user", "content": "resume safely"}]);
    let checkpoint = serde_json::json!({"turn": 1, "chain_messages": 1});

    let checkpointed = runtime_memory_tx::checkpoint_bound_worker_chain(
        db.pool(),
        &runtime_memory_tx::CheckpointBoundWorkerChainRow {
            fence: fence.clone(),
            message_chain_id: chain_id,
            chain: chain.clone(),
            checkpoint: checkpoint.clone(),
        },
    )
    .await
    .expect("chain and checkpoint commit under one worker fence");
    assert_eq!(checkpointed.checkpoint, checkpoint);
    let loaded = runtime_memory_tx::load_bound_worker_chain(
        db.pool(),
        &runtime_memory_tx::LoadBoundWorkerChainRow {
            operation_id: runtime.roots.operation_id,
            stage_execution_id: runtime.roots.stage_execution_id,
            stage_run_unit_id: runtime.unit_id,
            worker_run_id: runtime.worker_id,
            message_chain_id: chain_id,
            session_id: runtime.roots.session_id,
            agent: AgentType::Primary,
            selected_source: None,
        },
    )
    .await
    .expect("load exact bound chain and selected complete worker record");
    assert_eq!(loaded.chain, chain);
    assert_eq!(loaded.worker.checkpoint, checkpoint);

    sqlx::query(
        r#"CREATE FUNCTION reject_bound_worker_checkpoint() RETURNS trigger AS $$
           BEGIN
             RAISE EXCEPTION 'injected bound checkpoint failure';
           END;
           $$ LANGUAGE plpgsql"#,
    )
    .execute(db.pool())
    .await
    .expect("install bound checkpoint failure function");
    sqlx::query(
        r#"CREATE TRIGGER reject_bound_worker_checkpoint
           BEFORE UPDATE OF checkpoint ON stage_worker_runs
           FOR EACH ROW EXECUTE FUNCTION reject_bound_worker_checkpoint()"#,
    )
    .execute(db.pool())
    .await
    .expect("install bound checkpoint failure trigger");
    let next_fence = runtime_memory_tx::RuntimeMemoryTxFence {
        expected_checkpoint_version: checkpointed.checkpoint_version,
        ..fence
    };
    let rejected_chain = serde_json::json!([{"role": "user", "content": "must roll back"}]);
    let rejected = runtime_memory_tx::checkpoint_bound_worker_chain(
        db.pool(),
        &runtime_memory_tx::CheckpointBoundWorkerChainRow {
            fence: next_fence,
            message_chain_id: chain_id,
            chain: rejected_chain,
            checkpoint: serde_json::json!({"turn": 2}),
        },
    )
    .await;
    assert!(rejected.is_err());
    let persisted_chain = sqlx::query_scalar::<_, Option<serde_json::Value>>(
        "SELECT chain FROM message_chains WHERE id=$1",
    )
    .bind(chain_id)
    .fetch_one(db.pool())
    .await
    .expect("reload chain after rollback")
    .expect("chain body remains");
    assert_eq!(persisted_chain, chain);

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn expired_worker_without_active_tool_requeues_and_can_be_reclaimed() {
    let (mut db, _data_dir) = fixture().await;
    let runtime = create_claimed_compound_runtime(&db).await;
    sqlx::query(
        "UPDATE stage_worker_runs
            SET lease_acquired_at = NOW() - INTERVAL '2 minutes',
                lease_expires_at = NOW() - INTERVAL '1 minute'
          WHERE id=$1",
    )
    .bind(runtime.worker_id)
    .execute(db.pool())
    .await
    .expect("expire worker lease");

    let input = runtime_memory_tx::LoadWorkerCheckpointRow {
        operation_id: runtime.roots.operation_id,
        stage_execution_id: runtime.roots.stage_execution_id,
        stage_run_unit_id: runtime.unit_id,
        worker_run_id: runtime.worker_id,
        selected_source: None,
    };
    let (disposition, requeued) = runtime_memory_tx::reap_expired_worker(db.pool(), &input)
        .await
        .expect("requeue an expired worker with no in-flight tool");
    assert_eq!(
        disposition,
        stage_worker_runs::ExpiredWorkerDisposition::Requeued
    );
    assert_eq!(requeued.status, "queued");
    assert_eq!(requeued.lease_token, None);

    let mirrored = runtime_memory_tx::load_worker_checkpoint(db.pool(), &input)
        .await
        .expect("legacy read observes the compound requeue");
    assert_eq!(
        mirrored.source,
        runtime_memory_tx::RuntimeMemoryRecordSource::Legacy
    );
    assert_eq!(mirrored.worker.status, "queued");
    assert_eq!(mirrored.worker.lease_token, None);

    let unit = stage_run_units::get(db.pool(), runtime.unit_id)
        .await
        .expect("reload running unit")
        .expect("unit remains");
    let reclaimed = runtime_memory_tx::claim_worker_and_bind_chain(
        db.pool(),
        &runtime_memory_tx::ClaimWorkerAndBindChainRow {
            operation_id: runtime.roots.operation_id,
            stage_execution_id: runtime.roots.stage_execution_id,
            stage_run_unit_id: runtime.unit_id,
            worker_run_id: runtime.worker_id,
            expected_unit_status: stage_run_units::StageRunUnitStatus::Running,
            expected_unit_row_version: unit.row_version,
            expected_worker_status: stage_worker_runs::StageWorkerRunStatus::Queued,
            expected_attempt_epoch: requeued.attempt_epoch,
            session_id: runtime.roots.session_id,
            subtask_id: None,
            agent: AgentType::Primary,
            model: None,
            provider: None,
            parent_chain_id: None,
            lease_owner: "reclaimed-worker".to_string(),
            lease_seconds: 60,
            initial_chain: serde_json::json!([{"ignored": "bound chain already exists"}]),
            initial_checkpoint: serde_json::json!({"ignored": "resume existing checkpoint"}),
        },
    )
    .await
    .expect("reclaim the exact logical worker and its prebound chain");
    assert_eq!(reclaimed.worker.status, "running");
    assert_eq!(reclaimed.worker.attempt_epoch, requeued.attempt_epoch + 1);
    assert_eq!(
        reclaimed.message_chain_id,
        runtime
            .worker
            .message_chain_id
            .expect("original bound chain")
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn expired_worker_with_active_tool_requires_recovery_and_is_not_reclaimable() {
    let (mut db, _data_dir) = fixture().await;
    let runtime = create_claimed_compound_runtime(&db).await;
    let fence = fence_for_claimed(&runtime);
    let tool_call_id = tool_calls::record_tracked_start(
        db.pool(),
        "expired-active-tool",
        runtime.roots.session_id,
        Some(runtime.roots.operation_id),
        None,
        "query_target_data",
        &serde_json::json!({"section": "targets"}),
        Some(&tool_calls::RuntimeToolIdentity {
            operation_id: runtime.roots.operation_id,
            stage_execution_id: runtime.roots.stage_execution_id,
            stage_run_unit_id: Some(runtime.unit_id),
            worker_run_id: Some(runtime.worker_id),
            organization_id: Some(runtime.roots.organization_id),
            attempt_epoch: Some(runtime.worker.attempt_epoch),
            lease_token: runtime.worker.lease_token,
        }),
    )
    .await
    .expect("persist exact worker-fenced tool call");
    runtime_memory_tx::begin_worker_tool(db.pool(), &fence, tool_call_id)
        .await
        .expect("mark external tool in flight through compound mirror");
    sqlx::query(
        "UPDATE stage_worker_runs
            SET lease_acquired_at = NOW() - INTERVAL '2 minutes',
                lease_expires_at = NOW() - INTERVAL '1 minute'
          WHERE id=$1",
    )
    .bind(runtime.worker_id)
    .execute(db.pool())
    .await
    .expect("expire active worker lease");

    let input = runtime_memory_tx::LoadWorkerCheckpointRow {
        operation_id: runtime.roots.operation_id,
        stage_execution_id: runtime.roots.stage_execution_id,
        stage_run_unit_id: runtime.unit_id,
        worker_run_id: runtime.worker_id,
        selected_source: None,
    };
    let (disposition, parked) = runtime_memory_tx::reap_expired_worker(db.pool(), &input)
        .await
        .expect("park unknown external side effect for recovery");
    assert_eq!(
        disposition,
        stage_worker_runs::ExpiredWorkerDisposition::RecoveryRequired
    );
    assert_eq!(parked.status, "recovery_required");
    assert_eq!(parked.active_tool_call_id, Some(tool_call_id));
    assert_eq!(parked.lease_token, runtime.worker.lease_token);

    let mirrored = runtime_memory_tx::load_worker_checkpoint(db.pool(), &input)
        .await
        .expect("legacy read observes recovery-required atomically");
    assert_eq!(mirrored.worker.status, "recovery_required");
    assert_eq!(mirrored.worker.active_tool_call_id, Some(tool_call_id));
    assert_eq!(mirrored.worker.lease_token, runtime.worker.lease_token);

    let repeated = runtime_memory_tx::reap_expired_worker(db.pool(), &input)
        .await
        .expect("reaping recovery-required worker is idempotent");
    assert_eq!(
        repeated.0,
        stage_worker_runs::ExpiredWorkerDisposition::RecoveryRequired
    );
    assert_eq!(repeated.1.attempt_epoch, parked.attempt_epoch);

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn worker_and_unit_non_pass_gate_outcome_commit_or_roll_back_together() {
    let (mut db, _data_dir) = fixture().await;
    let runtime = create_claimed_compound_runtime(&db).await;
    let unit = stage_run_units::get(db.pool(), runtime.unit_id)
        .await
        .expect("reload running unit")
        .expect("unit remains");
    let pass_without_handoff = runtime_memory_tx::finish_worker_attempt(
        db.pool(),
        &runtime_memory_tx::FinishWorkerAttemptRow {
            fence: fence_for_claimed(&runtime),
            expected_status: stage_worker_runs::StageWorkerRunStatus::Running,
            next_status: stage_worker_runs::StageWorkerRunStatus::Passed,
            expected_unit_status: stage_run_units::StageRunUnitStatus::Running,
            expected_unit_row_version: unit.row_version,
            next_unit_status: stage_run_units::StageRunUnitStatus::Passed,
            checkpoint: serde_json::json!({"terminal": true}),
            evidence_watermark: Some(42),
        },
    )
    .await;
    assert!(matches!(
        pass_without_handoff,
        Err(runtime_memory_tx::RuntimeMemoryStoreError::Conflict {
            code: "unit_pass_requires_final_seal"
        })
    ));
    let finished = runtime_memory_tx::finish_worker_attempt(
        db.pool(),
        &runtime_memory_tx::FinishWorkerAttemptRow {
            fence: fence_for_claimed(&runtime),
            expected_status: stage_worker_runs::StageWorkerRunStatus::Running,
            next_status: stage_worker_runs::StageWorkerRunStatus::GateBlocked,
            expected_unit_status: stage_run_units::StageRunUnitStatus::Running,
            expected_unit_row_version: unit.row_version,
            next_unit_status: stage_run_units::StageRunUnitStatus::GateBlocked,
            checkpoint: serde_json::json!({"gate": "blocked"}),
            evidence_watermark: None,
        },
    )
    .await
    .expect("commit worker and unit gate-blocked outcome in one transaction");
    assert_eq!(finished.worker.status, "gate_blocked");
    assert_eq!(finished.worker.lease_token, None);
    assert_eq!(finished.unit.status, "gate_blocked");
    assert_eq!(finished.unit.pass_watermark, serde_json::json!({}));

    db.stop().await;

    let (mut rollback_db, _rollback_data_dir) = fixture().await;
    let rollback_runtime = create_claimed_compound_runtime(&rollback_db).await;
    let rollback_unit = stage_run_units::get(rollback_db.pool(), rollback_runtime.unit_id)
        .await
        .expect("reload second running unit")
        .expect("second unit remains");
    sqlx::query(
        r#"CREATE FUNCTION reject_unit_gate_outcome() RETURNS trigger AS $$
           BEGIN
             RAISE EXCEPTION 'injected unit gate outcome failure';
           END;
           $$ LANGUAGE plpgsql"#,
    )
    .execute(rollback_db.pool())
    .await
    .expect("install unit failure function");
    sqlx::query(
        r#"CREATE TRIGGER reject_unit_gate_outcome
           BEFORE UPDATE OF status ON stage_run_units
           FOR EACH ROW EXECUTE FUNCTION reject_unit_gate_outcome()"#,
    )
    .execute(rollback_db.pool())
    .await
    .expect("install unit failure trigger");
    let rejected = runtime_memory_tx::finish_worker_attempt(
        rollback_db.pool(),
        &runtime_memory_tx::FinishWorkerAttemptRow {
            fence: fence_for_claimed(&rollback_runtime),
            expected_status: stage_worker_runs::StageWorkerRunStatus::Running,
            next_status: stage_worker_runs::StageWorkerRunStatus::Exhausted,
            expected_unit_status: stage_run_units::StageRunUnitStatus::Running,
            expected_unit_row_version: rollback_unit.row_version,
            next_unit_status: stage_run_units::StageRunUnitStatus::Exhausted,
            checkpoint: serde_json::json!({"gate": "exhausted"}),
            evidence_watermark: None,
        },
    )
    .await;
    assert!(rejected.is_err());
    let worker_after = stage_worker_runs::get(rollback_db.pool(), rollback_runtime.worker_id)
        .await
        .expect("reload worker after rollback")
        .expect("worker remains");
    assert_eq!(worker_after.status, "running");
    assert_eq!(
        worker_after.lease_token,
        rollback_runtime.worker.lease_token
    );
    let unit_after = stage_run_units::get(rollback_db.pool(), rollback_runtime.unit_id)
        .await
        .expect("reload unit after rollback")
        .expect("unit remains");
    assert_eq!(unit_after.status, "running");
    assert_eq!(unit_after.row_version, rollback_unit.row_version);

    rollback_db.stop().await;
}

#[tokio::test]
#[serial]
async fn compound_wave_close_finalizes_without_early_completion_and_replays_exact_terminal_wave() {
    let (mut db, _data_dir) = fixture().await;
    let fixture = create_final_seal_fixture(&db).await;
    let wave = create_initial_final_seal_wave(&db, &fixture).await;
    let input = close_wave_input(&fixture, &wave);

    let closed = runtime_memory_tx::close_wave_gate_pass(db.pool(), &input)
        .await
        .expect("complete exact terminal wave and final seal in one transaction");
    let finalized = match closed {
        runtime_memory_tx::ClosedWaveGatePassRow::Finalized(finalized) => finalized,
        runtime_memory_tx::ClosedWaveGatePassRow::WaitingBackground { .. } => {
            panic!("no unassigned target should create a next wave")
        }
    };
    assert!(!finalized.replayed);
    assert_eq!(finalized.unit.status, "passed");
    assert_eq!(finalized.worker.status, "passed");
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM stage_asset_waves WHERE id=$1")
            .bind(wave.wave.id)
            .fetch_one(db.pool())
            .await
            .expect("reload final wave"),
        "completed"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM org_stage_completions WHERE organization_id=$1 AND stage_kind='target_intel' AND stage_run_id=$2"
        )
        .bind(fixture.runtime.roots.organization_id)
        .bind(fixture.runtime.roots.operation_id.to_string())
        .fetch_one(db.pool())
        .await
        .expect("count only final-seal completion"),
        1
    );

    let replay = runtime_memory_tx::close_wave_gate_pass(db.pool(), &input)
        .await
        .expect("response-loss replay recognizes the exact terminal wave");
    assert!(matches!(
        replay,
        runtime_memory_tx::ClosedWaveGatePassRow::Finalized(
            runtime_memory_tx::FinalizedUnitPassRow { replayed: true, .. }
        )
    ));

    let hostile_child_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO stage_asset_waves (
               id,operation_id,organization_id,stage_kind,wave_index,status,
               started_at,completed_at,parent_wave_id,asset_hash,updated_at
           ) VALUES ($1,$2,$3,'target_intel',$4,'completed',NOW(),NOW(),$5,'hostile',NOW())"#,
    )
    .bind(hostile_child_id)
    .bind(fixture.runtime.roots.operation_id)
    .bind(fixture.runtime.roots.organization_id)
    .bind(wave.wave.wave_index + 1)
    .bind(wave.wave.id)
    .execute(db.pool())
    .await
    .expect("inject a hostile later child wave");
    let hostile_replay = runtime_memory_tx::close_wave_gate_pass(db.pool(), &input).await;
    assert!(matches!(
        hostile_replay,
        Err(
            runtime_memory_tx::RuntimeMemoryStoreError::IdentityMismatch {
                code: "stage_asset_wave_final_replay_mismatch"
            }
        )
    ));

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn compound_wave_close_waiting_background_is_exactly_replayable_without_false_pass() {
    let (mut db, _data_dir) = fixture().await;
    let fixture = create_final_seal_fixture(&db).await;
    let wave = create_initial_final_seal_wave(&db, &fixture).await;
    let next_target_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO targets (
               id,name,target_type,value,scope,project_path,organization_id
           ) VALUES ($1,'next.example','domain','next.example','in',
                     '/tmp/runtime-worker',$2)"#,
    )
    .bind(next_target_id)
    .bind(fixture.runtime.roots.organization_id)
    .execute(db.pool())
    .await
    .expect("insert an unassigned supplemental target");
    let input = close_wave_input(&fixture, &wave);
    let closed = runtime_memory_tx::close_wave_gate_pass(db.pool(), &input)
        .await
        .expect("complete wave, create child and park worker atomically");
    let next_wave_id = match closed {
        runtime_memory_tx::ClosedWaveGatePassRow::WaitingBackground {
            unit,
            worker,
            next_wave,
        } => {
            assert_eq!(unit.status, "running");
            assert_eq!(unit.row_version, fixture.unit.row_version + 1);
            assert_eq!(unit.pass_watermark, input.continuation_pass_watermark);
            assert_eq!(worker.status, "waiting_background");
            assert_eq!(worker.lease_token, None);
            assert_eq!(next_wave.wave.parent_wave_id, Some(wave.wave.id));
            next_wave.wave.id
        }
        runtime_memory_tx::ClosedWaveGatePassRow::Finalized(_) => {
            panic!("supplemental target must withhold final PASS")
        }
    };
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM org_stage_completions WHERE organization_id=$1 AND stage_kind='target_intel' AND stage_run_id=$2"
        )
        .bind(fixture.runtime.roots.organization_id)
        .bind(fixture.runtime.roots.operation_id.to_string())
        .fetch_one(db.pool())
        .await
        .expect("count false completions"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM stage_handoffs WHERE operation_id=$1")
            .bind(fixture.runtime.roots.operation_id)
            .fetch_one(db.pool())
            .await
            .expect("count premature handoffs"),
        0
    );

    let replay = runtime_memory_tx::close_wave_gate_pass(db.pool(), &input)
        .await
        .expect("response-loss retry returns the exact persisted child wave");
    assert!(matches!(
        replay,
        runtime_memory_tx::ClosedWaveGatePassRow::WaitingBackground { next_wave, .. }
            if next_wave.wave.id == next_wave_id
    ));
    let mut drifted = input.clone();
    drifted.continuation_pass_watermark["pending_v2_final_seal"]["material"]["cells"] =
        serde_json::json!([]);
    assert!(runtime_memory_tx::close_wave_gate_pass(db.pool(), &drifted)
        .await
        .is_err());

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn compound_wave_close_failure_injection_rolls_back_both_pause_and_final_alternatives() {
    let (mut db, _data_dir) = fixture().await;
    let pause_fixture = create_final_seal_fixture(&db).await;
    let pause_wave = create_initial_final_seal_wave(&db, &pause_fixture).await;
    sqlx::query(
        r#"INSERT INTO targets (
               id,name,target_type,value,scope,project_path,organization_id
           ) VALUES ($1,'pause-next.example','domain','pause-next.example','in',
                     '/tmp/runtime-worker',$2)"#,
    )
    .bind(Uuid::new_v4())
    .bind(pause_fixture.runtime.roots.organization_id)
    .execute(db.pool())
    .await
    .expect("insert pause-path supplemental target");
    sqlx::query(
        r#"CREATE FUNCTION reject_wave_worker_pause() RETURNS trigger AS $$
           BEGIN
             IF NEW.status='waiting_background' THEN
               RAISE EXCEPTION 'injected wave worker pause failure';
             END IF;
             RETURN NEW;
           END;
           $$ LANGUAGE plpgsql"#,
    )
    .execute(db.pool())
    .await
    .expect("install pause failure function");
    sqlx::query(
        r#"CREATE TRIGGER reject_wave_worker_pause
           BEFORE UPDATE OF status ON stage_worker_runs
           FOR EACH ROW EXECUTE FUNCTION reject_wave_worker_pause()"#,
    )
    .execute(db.pool())
    .await
    .expect("install pause failure trigger");
    assert!(runtime_memory_tx::close_wave_gate_pass(
        db.pool(),
        &close_wave_input(&pause_fixture, &pause_wave),
    )
    .await
    .is_err());
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM stage_asset_waves WHERE id=$1")
            .bind(pause_wave.wave.id)
            .fetch_one(db.pool())
            .await
            .expect("reload pause wave after rollback"),
        "running"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM stage_asset_waves WHERE parent_wave_id=$1"
        )
        .bind(pause_wave.wave.id)
        .fetch_one(db.pool())
        .await
        .expect("count rolled-back child waves"),
        0
    );
    sqlx::query("DROP TRIGGER reject_wave_worker_pause ON stage_worker_runs")
        .execute(db.pool())
        .await
        .expect("remove pause failure trigger");

    db.stop().await;
    let (mut db, _final_data_dir) = fixture().await;
    let final_fixture = create_final_seal_fixture(&db).await;
    let final_wave = create_initial_final_seal_wave(&db, &final_fixture).await;
    sqlx::query(
        r#"CREATE FUNCTION reject_wave_final_handoff() RETURNS trigger AS $$
           BEGIN
             RAISE EXCEPTION 'injected wave final handoff failure';
           END;
           $$ LANGUAGE plpgsql"#,
    )
    .execute(db.pool())
    .await
    .expect("install final failure function");
    sqlx::query(
        r#"CREATE TRIGGER reject_wave_final_handoff
           BEFORE INSERT ON stage_handoffs
           FOR EACH ROW EXECUTE FUNCTION reject_wave_final_handoff()"#,
    )
    .execute(db.pool())
    .await
    .expect("install final failure trigger");
    assert!(runtime_memory_tx::close_wave_gate_pass(
        db.pool(),
        &close_wave_input(&final_fixture, &final_wave),
    )
    .await
    .is_err());
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM stage_asset_waves WHERE id=$1")
            .bind(final_wave.wave.id)
            .fetch_one(db.pool())
            .await
            .expect("reload final wave after rollback"),
        "running"
    );
    let unit = stage_run_units::get(db.pool(), final_fixture.runtime.unit_id)
        .await
        .expect("reload final unit")
        .expect("final unit exists");
    let worker = stage_worker_runs::get(db.pool(), final_fixture.runtime.worker_id)
        .await
        .expect("reload final worker")
        .expect("final worker exists");
    assert_eq!(unit.status, "running");
    assert_eq!(worker.status, "running");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM org_stage_completions WHERE organization_id=$1 AND stage_kind='target_intel' AND stage_run_id=$2"
        )
        .bind(final_fixture.runtime.roots.organization_id)
        .bind(final_fixture.runtime.roots.operation_id.to_string())
        .fetch_one(db.pool())
        .await
        .expect("count rolled-back final completions"),
        0
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn stage_handoff_accepts_exact_operation_chat_session_outcome() {
    let (mut db, _data_dir) = fixture().await;
    let fixture = create_final_seal_fixture(&db).await;
    let chat_session_key = format!("stage-run-{}", Uuid::new_v4());
    sqlx::query("UPDATE sessions SET chat_session_key=$1 WHERE id=$2")
        .bind(&chat_session_key)
        .bind(fixture.runtime.roots.session_id)
        .execute(db.pool())
        .await
        .expect("bind exact chat-session key to the operation session");
    sqlx::query(
        r#"INSERT INTO technique_outcomes (
               organization_id, run_id, asset, technique, outcome, evidence_ids,
               seq, collected_at
           ) VALUES ($1,$2,'seal.example','GOLISH-INTEL-DNS','found',$3,1,NOW())"#,
    )
    .bind(fixture.runtime.roots.organization_id)
    .bind(&chat_session_key)
    .bind(vec![fixture.evidence_id])
    .execute(db.pool())
    .await
    .expect("insert exact chat-session canonical outcome");

    let finalized = runtime_memory_tx::finalize_unit_pass(
        db.pool(),
        &final_seal_input(
            &fixture,
            vec![canonical_fact_refs::CanonicalFactKey::TechniqueOutcome {
                organization_id: fixture.runtime.roots.organization_id,
                run_id: chat_session_key.clone(),
                asset: "seal.example".to_string(),
                technique: "GOLISH-INTEL-DNS".to_string(),
            }],
        ),
    )
    .await
    .expect("the exact operation-owned chat-session outcome must be sealable");

    assert!(matches!(
        &finalized.canonical_fact_refs[0].key,
        canonical_fact_refs::CanonicalFactKey::TechniqueOutcome { run_id, .. }
            if run_id == &chat_session_key
    ));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn final_seal_resolver_attests_complete_large_vuln_outcome_set() {
    let (mut db, _data_dir) = fixture().await;
    let fixture = create_final_seal_fixture(&db).await;
    for asset_index in 0..36 {
        for technique_index in 0..10 {
            sqlx::query(
                r#"INSERT INTO technique_outcomes (
                       organization_id,run_id,asset,technique,outcome,source,query,
                       result_count,confidence,evidence_ids,seq,collected_at
                   ) VALUES ($1,$2,$3,$4,'blocked','vuln_terminal_materializer',
                             'bounded fixture',0,1.0,$5,$6,NOW())"#,
            )
            .bind(fixture.runtime.roots.organization_id)
            .bind(fixture.runtime.roots.operation_id.to_string())
            .bind(format!("https://host-{asset_index:03}.example"))
            .bind(format!("GOLISH-VULN-{technique_index:02}"))
            .bind(vec![fixture.evidence_id])
            .bind(i64::from(asset_index * 10 + technique_index + 1))
            .execute(db.pool())
            .await
            .expect("insert exact Vuln outcome-set member");
        }
    }
    let rows = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            String,
            String,
            String,
            chrono::DateTime<chrono::Utc>,
            Vec<i64>,
            serde_json::Value,
        ),
    >(
        r#"SELECT outcome.organization_id,outcome.run_id,outcome.asset,outcome.technique,
                  outcome.outcome,outcome.collected_at,outcome.evidence_ids,
                  to_jsonb(outcome.*)
             FROM technique_outcomes AS outcome
            WHERE outcome.organization_id=$1 AND outcome.run_id=$2
            ORDER BY outcome.asset,outcome.technique"#,
    )
    .bind(fixture.runtime.roots.organization_id)
    .bind(fixture.runtime.roots.operation_id.to_string())
    .fetch_all(db.pool())
    .await
    .expect("load exact Vuln outcome-set members");
    let members = rows
        .into_iter()
        .map(
            |(
                organization_id,
                run_id,
                asset,
                technique,
                outcome,
                observed_at,
                evidence_ids,
                content,
            )| canonical_fact_refs::TechniqueOutcomeSetMember {
                organization_id,
                run_id,
                asset,
                technique,
                outcome,
                observed_at,
                evidence_ids,
                content,
            },
        )
        .collect::<Vec<_>>();
    let attestation = canonical_fact_refs::technique_outcome_set_attestation(
        "vuln_triage",
        fixture.runtime.roots.organization_id,
        &fixture.runtime.roots.operation_id.to_string(),
        &members,
    )
    .expect("compute expected outcome-set identity");
    let key = canonical_fact_refs::CanonicalFactKey::TechniqueOutcomeSet {
        organization_id: fixture.runtime.roots.organization_id,
        run_id: fixture.runtime.roots.operation_id.to_string(),
        stage: "vuln_triage".to_string(),
        terminal_cell_count: attestation.terminal_cell_count,
        outcome_set_sha256: attestation.outcome_set_sha256.clone(),
    };
    let mut tx = db.pool().begin().await.expect("begin set resolution");
    let resolved = canonical_fact_refs::resolve_for_final_seal(
        &mut tx,
        fixture.runtime.roots.operation_id,
        fixture.runtime.roots.organization_id,
        "/tmp/runtime-worker",
        fixture
            .unit
            .started_at
            .expect("running Unit has start time"),
        chrono::Utc::now() + chrono::Duration::seconds(1),
        std::slice::from_ref(&key),
    )
    .await
    .expect("resolve complete final-seal outcome set");
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].content_sha256, attestation.content_sha256);
    assert_eq!(resolved[0].evidence_ids, vec![fixture.evidence_id]);
    assert!(matches!(
        canonical_fact_refs::resolve_for_handoff(
            &mut tx,
            fixture.runtime.roots.operation_id,
            fixture.runtime.roots.organization_id,
            "/tmp/runtime-worker",
            fixture
                .unit
                .started_at
                .expect("running Unit has start time"),
            &[key],
        )
        .await,
        Err(canonical_fact_refs::CanonicalFactRefError::Rejected {
            code: "technique_outcome_set_final_seal_only"
        })
    ));
    tx.rollback().await.expect("finish set resolution");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn stage_handoff_rejects_unknown_stale_or_foreign_canonical_refs() {
    let (mut db, _data_dir) = fixture().await;
    let fixture = create_final_seal_fixture(&db).await;

    let unknown = runtime_memory_tx::finalize_unit_pass(
        db.pool(),
        &final_seal_input(
            &fixture,
            vec![canonical_fact_refs::CanonicalFactKey::Target {
                target_id: Uuid::new_v4(),
            }],
        ),
    )
    .await;
    assert!(matches!(
        unknown,
        Err(
            runtime_memory_tx::RuntimeMemoryStoreError::IdentityMismatch {
                code: "canonical_fact_unknown_or_foreign"
            }
        )
    ));

    let sibling_operation_id = Uuid::new_v4();
    let sibling_evidence_id = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO audit_log (
               action, category, details, project_path, source, target_id,
               audit_role, detail, run_id, created_at
           ) VALUES ('sibling evidence','harness','foreign operation',
                     '/tmp/runtime-worker','harness',$1,'evidence',$2,$3,NOW())
           RETURNING id"#,
    )
    .bind(fixture.target_id)
    .bind(serde_json::json!({"organization_id": fixture.runtime.roots.organization_id}))
    .bind(sibling_operation_id)
    .fetch_one(db.pool())
    .await
    .expect("insert sibling-operation evidence");
    let mut foreign_evidence = final_seal_input(
        &fixture,
        vec![canonical_fact_refs::CanonicalFactKey::Target {
            target_id: fixture.target_id,
        }],
    );
    foreign_evidence.evidence_ids = vec![sibling_evidence_id];
    refresh_final_seal_material_hash(&mut foreign_evidence);
    let foreign_evidence =
        runtime_memory_tx::finalize_unit_pass(db.pool(), &foreign_evidence).await;
    assert!(matches!(
        foreign_evidence,
        Err(
            runtime_memory_tx::RuntimeMemoryStoreError::IdentityMismatch {
                code: "final_seal_evidence_stale_or_foreign"
            }
        )
    ));

    sqlx::query(
        r#"INSERT INTO technique_outcomes (
               organization_id, run_id, asset, technique, outcome, evidence_ids,
               seq, collected_at
           ) VALUES ($1,$2,'seal.example','GOLISH-INTEL-DNS','found','{}',1,NOW())"#,
    )
    .bind(fixture.runtime.roots.organization_id)
    .bind(sibling_operation_id.to_string())
    .execute(db.pool())
    .await
    .expect("insert sibling-operation canonical outcome");
    let foreign_outcome = runtime_memory_tx::finalize_unit_pass(
        db.pool(),
        &final_seal_input(
            &fixture,
            vec![canonical_fact_refs::CanonicalFactKey::TechniqueOutcome {
                organization_id: fixture.runtime.roots.organization_id,
                run_id: sibling_operation_id.to_string(),
                asset: "seal.example".to_string(),
                technique: "GOLISH-INTEL-DNS".to_string(),
            }],
        ),
    )
    .await;
    assert!(matches!(
        foreign_outcome,
        Err(
            runtime_memory_tx::RuntimeMemoryStoreError::IdentityMismatch {
                code: "canonical_fact_foreign_operation"
            }
        )
    ));

    let foreign_organization_id = Uuid::new_v4();
    let foreign_target_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO organizations (id, project_path, name)
           VALUES ($1,'/tmp/runtime-worker','Foreign Org')"#,
    )
    .bind(foreign_organization_id)
    .execute(db.pool())
    .await
    .expect("insert foreign org");
    sqlx::query(
        r#"INSERT INTO targets (
               id, name, target_type, value, scope, project_path, organization_id
           ) VALUES ($1,'foreign.example','domain','foreign.example','in',
                     '/tmp/runtime-worker',$2)"#,
    )
    .bind(foreign_target_id)
    .bind(foreign_organization_id)
    .execute(db.pool())
    .await
    .expect("insert foreign target");
    let foreign = runtime_memory_tx::finalize_unit_pass(
        db.pool(),
        &final_seal_input(
            &fixture,
            vec![canonical_fact_refs::CanonicalFactKey::Target {
                target_id: foreign_target_id,
            }],
        ),
    )
    .await;
    assert!(matches!(
        foreign,
        Err(
            runtime_memory_tx::RuntimeMemoryStoreError::IdentityMismatch {
                code: "canonical_fact_unknown_or_foreign"
            }
        )
    ));

    let stale_target_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO targets (
               id, name, target_type, value, scope, project_path, organization_id,
               created_at, updated_at
           ) VALUES ($1,'stale.example','domain','stale.example','in',
                     '/tmp/runtime-worker',$2,NOW() - INTERVAL '1 day',
                     NOW() - INTERVAL '1 day')"#,
    )
    .bind(stale_target_id)
    .bind(fixture.runtime.roots.organization_id)
    .execute(db.pool())
    .await
    .expect("insert stale owned target");
    let stale_finding_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO findings (
               id, title, sev, target, target_id, project_path, source,
               created_at, updated_at
           ) VALUES ($1,'Stale finding','low','stale.example',$2,
                     '/tmp/runtime-worker','harness',NOW() - INTERVAL '1 day',
                     NOW() - INTERVAL '1 day')"#,
    )
    .bind(stale_finding_id)
    .bind(stale_target_id)
    .execute(db.pool())
    .await
    .expect("insert stale target-owned finding");
    let stale = runtime_memory_tx::finalize_unit_pass(
        db.pool(),
        &final_seal_input(
            &fixture,
            vec![canonical_fact_refs::CanonicalFactKey::Finding {
                finding_id: stale_finding_id,
            }],
        ),
    )
    .await;
    assert!(matches!(
        stale,
        Err(
            runtime_memory_tx::RuntimeMemoryStoreError::IdentityMismatch {
                code: "canonical_fact_stale"
            }
        )
    ));

    let unit = stage_run_units::get(db.pool(), fixture.runtime.unit_id)
        .await
        .expect("reload rejected unit")
        .expect("unit remains");
    let worker = stage_worker_runs::get(db.pool(), fixture.runtime.worker_id)
        .await
        .expect("reload rejected worker")
        .expect("worker remains");
    assert_eq!(unit.status, "running");
    assert_eq!(worker.status, "running");
    assert_eq!(worker.lease_token, fixture.runtime.worker.lease_token);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn final_seal_unit_worker_handoff_completion_and_legacy_mirror_are_atomic() {
    let (mut db, _data_dir) = fixture().await;
    let fixture = create_final_seal_fixture(&db).await;
    let before_blob = operation_state::get(db.pool(), fixture.runtime.roots.operation_id)
        .await
        .expect("load pre-seal legacy mirror")
        .expect("operation exists")
        .state_blob;
    sqlx::query("UPDATE targets SET updated_at=NOW() - INTERVAL '1 day' WHERE id=$1")
        .bind(fixture.target_id)
        .execute(db.pool())
        .await
        .expect("make target a pre-existing locked identity seed");
    sqlx::query(
        r#"CREATE FUNCTION reject_final_seal_legacy_mirror() RETURNS trigger AS $$
           BEGIN
             RAISE EXCEPTION 'injected final seal legacy mirror failure';
           END;
           $$ LANGUAGE plpgsql"#,
    )
    .execute(db.pool())
    .await
    .expect("install final seal failure function");
    sqlx::query(
        r#"CREATE TRIGGER reject_final_seal_legacy_mirror
           BEFORE UPDATE OF state_blob ON operation_state
           FOR EACH ROW EXECUTE FUNCTION reject_final_seal_legacy_mirror()"#,
    )
    .execute(db.pool())
    .await
    .expect("install final seal failure trigger");

    let input = final_seal_input(
        &fixture,
        vec![
            canonical_fact_refs::CanonicalFactKey::Target {
                target_id: fixture.target_id,
            },
            canonical_fact_refs::CanonicalFactKey::Finding {
                finding_id: fixture.finding_id,
            },
        ],
    );
    let rejected = runtime_memory_tx::finalize_unit_pass(db.pool(), &input).await;
    assert!(rejected.is_err());
    let unit_after = stage_run_units::get(db.pool(), fixture.runtime.unit_id)
        .await
        .expect("reload rolled-back unit")
        .expect("unit remains");
    let worker_after = stage_worker_runs::get(db.pool(), fixture.runtime.worker_id)
        .await
        .expect("reload rolled-back worker")
        .expect("worker remains");
    assert_eq!(unit_after.status, "running");
    assert_eq!(unit_after.row_version, fixture.unit.row_version);
    assert_eq!(worker_after.status, "running");
    assert_eq!(worker_after.lease_token, fixture.runtime.worker.lease_token);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM stage_handoffs WHERE operation_id=$1")
            .bind(fixture.runtime.roots.operation_id)
            .fetch_one(db.pool())
            .await
            .expect("count rolled-back handoffs"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM org_stage_completions WHERE organization_id=$1 AND stage_kind='target_intel'"
        )
        .bind(fixture.runtime.roots.organization_id)
        .fetch_one(db.pool())
        .await
        .expect("count rolled-back compatibility completions"),
        0
    );
    for (label, sql) in [
        (
            "stage_episodes",
            "SELECT COUNT(*) FROM stage_episodes WHERE source_operation_id=$1",
        ),
        (
            "knowledge_outbox_events",
            "SELECT COUNT(*) FROM knowledge_outbox_events WHERE source_operation_id=$1",
        ),
        (
            "knowledge_projection_deliveries",
            r#"SELECT COUNT(*)
                 FROM knowledge_projection_deliveries delivery
                 JOIN knowledge_outbox_events event ON event.event_id=delivery.event_id
                WHERE event.source_operation_id=$1"#,
        ),
    ] {
        let count = sqlx::query_scalar::<_, i64>(sql)
            .bind(fixture.runtime.roots.operation_id)
            .fetch_one(db.pool())
            .await
            .unwrap_or_else(|error| panic!("count rolled-back {label}: {error}"));
        assert_eq!(count, 0, "table={label}");
    }
    let rolled_back_blob = operation_state::get(db.pool(), fixture.runtime.roots.operation_id)
        .await
        .expect("reload rolled-back legacy mirror")
        .expect("operation remains")
        .state_blob;
    assert_eq!(rolled_back_blob, before_blob);

    sqlx::query("DROP TRIGGER reject_final_seal_legacy_mirror ON operation_state")
        .execute(db.pool())
        .await
        .expect("remove injected final seal failure");

    sqlx::query(
        r#"CREATE FUNCTION reject_final_seal_memory_event() RETURNS trigger AS $$
           BEGIN
             RAISE EXCEPTION 'injected final seal memory event failure';
           END;
           $$ LANGUAGE plpgsql"#,
    )
    .execute(db.pool())
    .await
    .expect("install memory event failure function");
    sqlx::query(
        r#"CREATE TRIGGER reject_final_seal_memory_event
           BEFORE INSERT ON knowledge_outbox_events
           FOR EACH ROW EXECUTE FUNCTION reject_final_seal_memory_event()"#,
    )
    .execute(db.pool())
    .await
    .expect("install memory event failure trigger");
    let memory_rejected = runtime_memory_tx::finalize_unit_pass(db.pool(), &input).await;
    assert!(memory_rejected.is_err());
    assert_eq!(
        stage_run_units::get(db.pool(), fixture.runtime.unit_id)
            .await
            .expect("reload unit after memory failure")
            .expect("unit remains")
            .status,
        "running"
    );
    assert_eq!(
        stage_worker_runs::get(db.pool(), fixture.runtime.worker_id)
            .await
            .expect("reload worker after memory failure")
            .expect("worker remains")
            .status,
        "running"
    );
    for (label, sql) in [
        (
            "stage_handoffs",
            "SELECT COUNT(*) FROM stage_handoffs WHERE operation_id=$1",
        ),
        (
            "stage_episodes",
            "SELECT COUNT(*) FROM stage_episodes WHERE source_operation_id=$1",
        ),
        (
            "knowledge_outbox_events",
            "SELECT COUNT(*) FROM knowledge_outbox_events WHERE source_operation_id=$1",
        ),
        (
            "knowledge_projection_deliveries",
            r#"SELECT COUNT(*)
                 FROM knowledge_projection_deliveries delivery
                 JOIN knowledge_outbox_events event ON event.event_id=delivery.event_id
                WHERE event.source_operation_id=$1"#,
        ),
    ] {
        let count = sqlx::query_scalar::<_, i64>(sql)
            .bind(fixture.runtime.roots.operation_id)
            .fetch_one(db.pool())
            .await
            .unwrap_or_else(|error| panic!("count memory-failure rollback {label}: {error}"));
        assert_eq!(count, 0, "table={label}");
    }
    assert_eq!(
        operation_state::get(db.pool(), fixture.runtime.roots.operation_id)
            .await
            .expect("reload memory-failure operation")
            .expect("operation remains")
            .state_blob,
        before_blob
    );
    sqlx::query("DROP TRIGGER reject_final_seal_memory_event ON knowledge_outbox_events")
        .execute(db.pool())
        .await
        .expect("remove memory event failure trigger");

    let sealed = runtime_memory_tx::finalize_unit_pass(db.pool(), &input)
        .await
        .expect("publish one exact final seal");
    assert_eq!(sealed.unit.status, "passed");
    assert_eq!(sealed.worker.status, "passed");
    assert_eq!(sealed.canonical_fact_refs.len(), 2);
    assert_eq!(sealed.handoff.evidence_ids, vec![fixture.evidence_id]);
    assert_eq!(
        sealed.handoff.payload["canonical_fact_refs"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    let replayed = runtime_memory_tx::finalize_unit_pass(db.pool(), &input)
        .await
        .expect("response-loss replay returns the exact committed seal");
    assert!(replayed.replayed);
    assert_eq!(replayed.handoff.id, sealed.handoff.id);
    assert_eq!(replayed.worker.id, sealed.worker.id);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM stage_episodes WHERE source_operation_id=$1"
        )
        .bind(fixture.runtime.roots.operation_id)
        .fetch_one(db.pool())
        .await
        .expect("count final-seal StageEpisode"),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM knowledge_outbox_events WHERE source_operation_id=$1 AND event_name='StageEpisodeClosed.v1'"
        )
        .bind(fixture.runtime.roots.operation_id)
        .fetch_one(db.pool())
        .await
        .expect("count final-seal Memory event"),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            r#"SELECT COUNT(*)
                 FROM knowledge_projection_deliveries delivery
                 JOIN knowledge_outbox_events event ON event.event_id=delivery.event_id
                WHERE event.source_operation_id=$1"#,
        )
        .bind(fixture.runtime.roots.operation_id)
        .fetch_one(db.pool())
        .await
        .expect("count final-seal projector deliveries"),
        4
    );
    let mut drifted_replay = input.clone();
    drifted_replay
        .typed_claims
        .push(serde_json::json!({"kind": "drift"}));
    assert!(matches!(
        runtime_memory_tx::finalize_unit_pass(db.pool(), &drifted_replay).await,
        Err(runtime_memory_tx::RuntimeMemoryStoreError::Conflict {
            code: "final_seal_replay_mismatch"
        })
    ));

    let inherited = stage_handoffs::list_latest_final_sealed_for_sources(
        db.pool(),
        fixture.runtime.roots.operation_id,
        fixture.runtime.roots.organization_id,
        &["target_intel".to_string()],
    )
    .await
    .expect("load downstream inherited final-sealed handoff");
    assert_eq!(inherited.len(), 1);
    assert_eq!(inherited[0].id, sealed.handoff.id);
    let completion_run_id = sqlx::query_scalar::<_, Option<String>>(
        "SELECT stage_run_id FROM org_stage_completions WHERE organization_id=$1 AND stage_kind='target_intel'",
    )
    .bind(fixture.runtime.roots.organization_id)
    .fetch_one(db.pool())
    .await
    .expect("load compatibility completion");
    assert_eq!(
        completion_run_id,
        Some(fixture.runtime.roots.operation_id.to_string())
    );
    let final_blob = operation_state::get(db.pool(), fixture.runtime.roots.operation_id)
        .await
        .expect("load final legacy mirror")
        .expect("operation remains")
        .state_blob;
    assert_eq!(
        final_blob["stage_run_workers"]["target_intel"]
            [fixture.runtime.roots.organization_id.to_string()]["status"],
        "passed"
    );
    assert_eq!(
        final_blob["stage_run_handoffs"]["target_intel"]
            [fixture.runtime.roots.organization_id.to_string()]["handoff_id"],
        sealed.handoff.id.to_string()
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn target_intel_empty_final_seal_emits_replayable_memory_attestation() {
    let (mut db, _data_dir) = fixture().await;
    let fixture = create_final_seal_fixture(&db).await;
    let mut input = final_seal_input(&fixture, Vec::new());
    input.evidence_ids.clear();
    refresh_final_seal_material_hash(&mut input);

    let sealed = runtime_memory_tx::finalize_unit_pass(db.pool(), &input)
        .await
        .expect("empty Target Intel truth still publishes an evidence-backed episode");
    assert!(sealed.canonical_fact_refs.is_empty());
    assert_eq!(sealed.handoff.evidence_ids.len(), 1);
    let attestation_id = sealed.handoff.evidence_ids[0];
    assert_ne!(attestation_id, fixture.evidence_id);

    let attestation = sqlx::query_as::<
        _,
        (
            String,
            String,
            Option<Uuid>,
            Option<Uuid>,
            serde_json::Value,
        ),
    >(
        r#"SELECT action,tool_name,run_id,target_id,detail
             FROM audit_log
            WHERE id=$1 AND audit_role='evidence'"#,
    )
    .bind(attestation_id)
    .fetch_one(db.pool())
    .await
    .expect("load exact server final-seal attestation");
    assert_eq!(attestation.0, "stage_final_seal_attested");
    assert_eq!(attestation.1, "runtime_memory_final_seal_attestation");
    assert_eq!(attestation.2, Some(fixture.runtime.roots.operation_id));
    assert_eq!(attestation.3, None);
    assert_eq!(
        attestation.4["organization_id"],
        fixture.runtime.roots.organization_id.to_string()
    );
    assert_eq!(
        attestation.4["deliverable_submission_id"],
        fixture.submission_id.to_string()
    );
    assert_eq!(
        sqlx::query_scalar::<_, Vec<i64>>(
            "SELECT evidence_refs FROM stage_episodes WHERE source_operation_id=$1"
        )
        .bind(fixture.runtime.roots.operation_id)
        .fetch_one(db.pool())
        .await
        .expect("load evidence-backed StageEpisode"),
        vec![attestation_id]
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM knowledge_outbox_events WHERE source_operation_id=$1 AND event_name='StageEpisodeClosed.v1'"
        )
        .bind(fixture.runtime.roots.operation_id)
        .fetch_one(db.pool())
        .await
        .expect("count StageEpisode event"),
        1
    );

    let replayed = runtime_memory_tx::finalize_unit_pass(db.pool(), &input)
        .await
        .expect("response-loss replay resolves the same server attestation");
    assert!(replayed.replayed);
    assert_eq!(replayed.handoff.id, sealed.handoff.id);
    assert_eq!(replayed.handoff.evidence_ids, vec![attestation_id]);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM audit_log WHERE run_id=$1 AND tool_name='runtime_memory_final_seal_attestation'"
        )
        .bind(fixture.runtime.roots.operation_id)
        .fetch_one(db.pool())
        .await
        .expect("count idempotent server attestations"),
        1
    );

    db.stop().await;
}
