use golish_db::models::{AgentType, NewSession};
use golish_db::repo::{
    canonical_fact_refs, message_chains, operation_state, project_scopes, runtime_memory_rollout,
    runtime_memory_tx, sessions, stage_asset_waves, stage_deliverable_submissions, stage_handoffs,
    stage_run_units, stage_worker_runs, tasks, tool_calls,
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
async fn exact_resume_source_claim_allows_only_one_waiting_contender() {
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
    let first =
        tasks::claim_exact_resumable_runtime_source(db.pool(), operation_id, session_id, source);
    let second =
        tasks::claim_exact_resumable_runtime_source(db.pool(), operation_id, session_id, source);
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
            input: "run target intelligence".to_string(),
            profile: "assessment".to_string(),
            entry_stage: "target_intel".to_string(),
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
