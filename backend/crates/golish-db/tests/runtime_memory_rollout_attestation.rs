use golish_db::models::NewSession;
use golish_db::repo::{
    project_scopes,
    runtime_memory_rollout::{self, RuntimeMemoryContract},
    runtime_memory_shadow, runtime_memory_tx, sessions, stage_run_units, stage_worker_runs,
};
use golish_db::{embedded::EmbeddedPg, DbConfig, GolishDb};
use serial_test::serial;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::borrow::Cow;
use tokio::time::{timeout, Duration};
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
        database: format!("runtime_rollout_{label}_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start migrated embedded postgres");
    (db, data_dir)
}

#[derive(Debug, Clone, Copy)]
struct AdmittedWorker {
    operation_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    worker_run_id: Uuid,
    organization_id: Uuid,
}

async fn create_admitted_worker(db: &GolishDb, label: &str) -> AdmittedWorker {
    create_admitted_worker_in_pool(db.pool(), label).await
}

async fn create_admitted_worker_in_pool(pool: &PgPool, label: &str) -> AdmittedWorker {
    let project_path = format!("/tmp/runtime-rollout-{label}-{}", Uuid::new_v4().simple());
    let session = sessions::create(
        pool,
        NewSession {
            title: Some(format!("runtime rollout {label}")),
            workspace_path: Some(project_path.clone()),
            workspace_label: None,
            model: None,
            provider: None,
            project_path: Some(project_path.clone()),
        },
    )
    .await
    .expect("create runtime rollout session");
    let project = project_scopes::register_first_open(
        pool,
        &project_path,
        &format!("runtime-rollout-{label}-sha"),
    )
    .await
    .expect("register runtime rollout project");
    let organization_id = Uuid::new_v4();
    sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$2,$3)")
        .bind(organization_id)
        .bind(&project_path)
        .bind(format!("Runtime rollout {label}"))
        .execute(pool)
        .await
        .expect("insert runtime rollout organization");

    let operation_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    runtime_memory_tx::create_runtime_operation(
        pool,
        &runtime_memory_tx::CreateRuntimeOperationRow {
            operation_id,
            initial_stage_execution_id: stage_execution_id,
            session_id: session.id,
            title: Some(format!("runtime rollout {label}")),
            input: "exercise retained runtime shadow".to_string(),
            profile: "assessment".to_string(),
            entry_stage: "target_intel".to_string(),
            project_scope_id: project.project_scope_id,
            cli_scope: Some(runtime_memory_tx::CliRuntimeScopeRow {
                root_organization_id: organization_id,
                include_subsidiaries: false,
                subsidiary_threshold: 51,
                units: vec![runtime_memory_tx::CliRuntimeScopeUnitRow {
                    organization_id,
                    parent_organization_id: None,
                    organization_name: format!("Runtime rollout {label}"),
                    depth: 0,
                    ordinal: 0,
                    ownership_percent: None,
                    approval_source: serde_json::json!({"source": "test"}),
                }],
            }),
        },
    )
    .await
    .expect("create rank-one runtime operation");
    let snapshot_id: Uuid =
        sqlx::query_scalar("SELECT id FROM operation_org_scope_snapshots WHERE operation_id=$1")
            .bind(operation_id)
            .fetch_one(pool)
            .await
            .expect("load sealed runtime scope");

    let stage_run_unit_id = Uuid::new_v4();
    let worker_run_id = Uuid::new_v4();
    let mut tx = pool.begin().await.expect("begin worker admission");
    stage_run_units::insert_with_executor(
        &mut *tx,
        &stage_run_units::NewStageRunUnit {
            id: stage_run_unit_id,
            operation_id,
            stage_execution_id,
            scope_snapshot_id: snapshot_id,
            organization_id,
            stage_kind: "target_intel".to_string(),
            generation: 0,
            specialist: Some("target-intel-specialist".to_string()),
        },
    )
    .await
    .expect("insert runtime unit");
    let has_stage_team_work_item: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (
               SELECT 1
                 FROM pg_attribute
                WHERE attrelid='stage_worker_runs'::regclass
                  AND attname='work_item_id' AND NOT attisdropped
           )"#,
    )
    .fetch_one(&mut *tx)
    .await
    .expect("inspect runtime Worker schema version");
    if has_stage_team_work_item {
        stage_worker_runs::insert_with_executor(
            &mut *tx,
            &stage_worker_runs::NewStageWorkerRun {
                id: worker_run_id,
                operation_id,
                stage_execution_id,
                stage_run_unit_id,
                work_item_id: None,
                organization_id,
                worker_generation: 0,
                specialist: "target-intel-specialist".to_string(),
                work_item_kind: "organization".to_string(),
                work_item_key: organization_id.to_string(),
                agent_path: format!("root>org:{organization_id}>target-intel-specialist"),
                parent_request_id: None,
            },
        )
        .await
        .expect("insert admitted runtime worker");
    } else {
        sqlx::query(
            r#"INSERT INTO stage_worker_runs(
                   id,operation_id,stage_execution_id,stage_run_unit_id,
                   organization_id,worker_generation,specialist,work_item_kind,
                   work_item_key,agent_path,parent_request_id,status
               ) VALUES($1,$2,$3,$4,$5,0,'target-intel-specialist','organization',
                        $6,$7,NULL,'queued')"#,
        )
        .bind(worker_run_id)
        .bind(operation_id)
        .bind(stage_execution_id)
        .bind(stage_run_unit_id)
        .bind(organization_id)
        .bind(organization_id.to_string())
        .bind(format!(
            "root>org:{organization_id}>target-intel-specialist"
        ))
        .execute(&mut *tx)
        .await
        .expect("insert pre-Stage-Team admitted runtime worker");
    }
    tx.commit().await.expect("commit worker admission");
    AdmittedWorker {
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        worker_run_id,
        organization_id,
    }
}

async fn install_current_v2_as_legacy(
    db: &GolishDb,
    worker: AdmittedWorker,
    checkpoint_drift: bool,
) {
    let mut record: serde_json::Value =
        sqlx::query_scalar("SELECT runtime_memory_v2_worker_record($1)")
            .bind(worker.worker_run_id)
            .fetch_one(db.pool())
            .await
            .expect("rehydrate current V2 worker record");
    if checkpoint_drift {
        record["checkpoint"] = serde_json::json!({"hostile": "legacy-drift"});
    }
    sqlx::query(
        r#"UPDATE operation_state
              SET state_blob = COALESCE(state_blob,'{}'::jsonb) ||
                  jsonb_build_object(
                    'stage_run_workers',
                    jsonb_build_object(
                      'target_intel',
                      jsonb_build_object(
                        $2::text,
                        jsonb_build_object(
                          'worker_records', jsonb_build_object($3::text,$4::jsonb)
                        )
                      )
                    )
                  )
            WHERE operation_id=$1"#,
    )
    .bind(worker.operation_id)
    .bind(worker.organization_id)
    .bind(worker.worker_run_id)
    .bind(record)
    .execute(db.pool())
    .await
    .expect("install complete legacy worker record");
}

async fn install_pre_attestation_worker_as_legacy(
    pool: &PgPool,
    worker: AdmittedWorker,
    label: &str,
) {
    let worker_row: serde_json::Value =
        sqlx::query_scalar("SELECT TO_JSONB(worker) FROM stage_worker_runs AS worker WHERE id=$1")
            .bind(worker.worker_run_id)
            .fetch_optional(pool)
            .await
            .expect("load pre-attestation WorkerRun")
            .expect("pre-attestation WorkerRun must exist");
    let field = |name: &str| worker_row.get(name).cloned().unwrap_or_default();
    let record = serde_json::json!({
        "schema_v": 2,
        "id": field("id"),
        "operation_id": field("operation_id"),
        "stage_execution_id": field("stage_execution_id"),
        "stage_run_unit_id": field("stage_run_unit_id"),
        "worker_run_id": field("id"),
        "organization_id": field("organization_id"),
        "org_name": format!("Runtime rollout {label}"),
        "worker_generation": field("worker_generation"),
        "specialist": field("specialist"),
        "work_item_kind": field("work_item_kind"),
        "work_item_key": field("work_item_key"),
        "agent_path": field("agent_path"),
        "parent_request_id": field("parent_request_id"),
        "chain_id": field("message_chain_id"),
        "message_chain_id": field("message_chain_id"),
        "status": field("status"),
        "gate_attempt": field("gate_attempt"),
        "checkpoint": field("checkpoint"),
        "checkpoint_version": field("checkpoint_version"),
        "lease_token": field("lease_token"),
        "lease_owner": field("lease_owner"),
        "lease_acquired_at": field("lease_acquired_at"),
        "lease_expires_at": field("lease_expires_at"),
        "heartbeat_at": field("heartbeat_at"),
        "attempt_epoch": field("attempt_epoch"),
        "active_tool_call_id": field("active_tool_call_id"),
        "active_tool_started_at": field("active_tool_started_at"),
        "evidence_watermark": field("evidence_watermark"),
        "started_at": field("started_at"),
        "updated_at": field("updated_at"),
        "terminal_at": field("terminal_at"),
    });
    sqlx::query(
        r#"UPDATE operation_state
              SET state_blob = COALESCE(state_blob,'{}'::jsonb) ||
                  jsonb_build_object(
                    'stage_run_workers',
                    jsonb_build_object(
                      'target_intel',
                      jsonb_build_object(
                        $2::text,
                        jsonb_build_object(
                          'worker_records',jsonb_build_object($3::text,$4::jsonb)
                        )
                      )
                    )
                  )
            WHERE operation_id=$1"#,
    )
    .bind(worker.operation_id)
    .bind(worker.organization_id)
    .bind(worker.worker_run_id)
    .bind(record)
    .execute(pool)
    .await
    .expect("install pre-attestation legacy WorkerRun mirror");
}

fn migration_subset(min_version: i64, max_version: i64) -> Migrator {
    let all = sqlx::migrate!("./migrations");
    Migrator {
        migrations: Cow::Owned(
            all.iter()
                .filter(|migration| {
                    migration.version >= min_version && migration.version <= max_version
                })
                .cloned()
                .collect(),
        ),
        ignore_missing: true,
        locking: true,
        no_tx: false,
    }
}

#[tokio::test]
#[serial]
async fn public_rollout_cas_cannot_promote_without_retained_runtime_samples() {
    let (mut db, _data_dir) = fixture("public_gate").await;
    let initial = runtime_memory_rollout::get(db.pool())
        .await
        .expect("read sampling rollout");
    assert_eq!(initial.contract, "dual_write_legacy_read");
    assert_eq!(initial.contract_rank, 1);

    let result = runtime_memory_rollout::advance(
        db.pool(),
        RuntimeMemoryContract::DualWriteLegacyRead,
        RuntimeMemoryContract::DualWriteV2Preferred,
        initial.row_version,
    )
    .await;
    assert!(
        result.is_err(),
        "an adjacent repository CAS must not bypass runtime shadow readiness"
    );

    let unchanged = runtime_memory_rollout::get(db.pool())
        .await
        .expect("read unchanged sampling rollout");
    assert_eq!(unchanged.contract, "dual_write_legacy_read");
    assert_eq!(unchanged.contract_rank, 1);
    assert_eq!(unchanged.row_version, initial.row_version);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn raw_rollout_update_cannot_promote_without_retained_runtime_samples() {
    let (mut db, _data_dir) = fixture("raw_gate").await;
    let raw_transition = sqlx::query(
        r#"UPDATE runtime_memory_rollout
              SET contract='dual_write_v2_preferred', contract_rank=2,
                  row_version=row_version+1, updated_at=NOW()
            WHERE singleton_id=1"#,
    )
    .execute(db.pool())
    .await;
    assert!(
        raw_transition.is_err(),
        "raw adjacent SQL must run the same retained runtime shadow gate"
    );

    let unchanged = runtime_memory_rollout::get(db.pool())
        .await
        .expect("read unchanged sampling rollout");
    assert_eq!(unchanged.contract, "dual_write_legacy_read");
    assert_eq!(unchanged.contract_rank, 1);
    assert_eq!(unchanged.row_version, 1);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn rollout_updates_require_read_committed_snapshots() {
    let (mut db, _data_dir) = fixture("rollout_isolation").await;
    let cases = [
        (
            "runtime",
            "UPDATE runtime_memory_rollout SET contract='dual_write_v2_preferred', \
             contract_rank=2,row_version=row_version+1,updated_at=NOW() \
             WHERE singleton_id=1",
        ),
        (
            "attack",
            "UPDATE attack_execution_rollout SET contract='dual_write_read_v2_fallback', \
             rank=2,row_version=row_version+1,updated_at=NOW() WHERE singleton=TRUE",
        ),
    ];
    for (label, statement) in cases {
        let mut tx = db.pool().begin().await.expect("begin isolation attack");
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await
            .expect("select hostile fixed-snapshot isolation");
        let error = sqlx::query(statement)
            .execute(&mut *tx)
            .await
            .expect_err("fixed-snapshot rollout updates must fail closed");
        assert!(
            error
                .to_string()
                .contains("EXECUTION_ROLLOUT_REQUIRES_READ_COMMITTED"),
            "unexpected {label} fixed-snapshot error: {error}"
        );
        tx.rollback().await.expect("rollback isolation attack");
    }
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn attestation_install_preflight_rejects_advanced_or_incompatible_singletons() {
    let (mut db, _data_dir) = fixture("attestation_install_preflight").await;

    let mut advanced_runtime = db
        .pool()
        .begin()
        .await
        .expect("begin advanced runtime fixture");
    sqlx::raw_sql(
        r#"ALTER TABLE runtime_memory_rollout
               DISABLE TRIGGER runtime_memory_rollout_forward_only;
           ALTER TABLE runtime_memory_rollout
               DISABLE TRIGGER zz_runtime_memory_rollout_attestation_gate;
           ALTER TABLE runtime_memory_rollout
               DISABLE TRIGGER zz_runtime_memory_rollout_promotion_receipt;
           UPDATE runtime_memory_rollout
              SET contract='dual_write_v2_preferred',contract_rank=2,
                  row_version=2,updated_at=NOW()
            WHERE singleton_id=1"#,
    )
    .execute(&mut *advanced_runtime)
    .await
    .expect("install unsupported pre-attestation runtime rank");
    let runtime_error =
        sqlx::query("SELECT assert_runtime_memory_shadow_attestation_installable()")
            .execute(&mut *advanced_runtime)
            .await
            .expect_err("migration preflight must reject an already-advanced runtime singleton");
    assert!(runtime_error
        .to_string()
        .contains("RUNTIME_MEMORY_ATTESTATION_REQUIRES_RANK_ONE"));
    advanced_runtime
        .rollback()
        .await
        .expect("rollback advanced runtime fixture");

    let mut incompatible_pair = db
        .pool()
        .begin()
        .await
        .expect("begin incompatible pair fixture");
    sqlx::raw_sql(
        r#"ALTER TABLE attack_execution_rollout
               DISABLE TRIGGER attack_execution_rollout_forward_only;
           ALTER TABLE attack_execution_rollout
               DISABLE TRIGGER zz_attack_runtime_rollout_compatibility;
           ALTER TABLE attack_execution_rollout
               DISABLE TRIGGER zz_attack_execution_rollout_promotion_receipt;
           UPDATE attack_execution_rollout
              SET contract='v2_only',rank=3,row_version=3,updated_at=NOW()
            WHERE singleton=TRUE"#,
    )
    .execute(&mut *incompatible_pair)
    .await
    .expect("install incompatible pre-attestation singleton pair");
    let pair_error = sqlx::query("SELECT assert_runtime_memory_shadow_attestation_installable()")
        .execute(&mut *incompatible_pair)
        .await
        .expect_err("migration preflight must reject an incompatible singleton pair");
    assert!(pair_error
        .to_string()
        .contains("EXECUTION_ROLLOUT_PAIR_INCOMPATIBLE_EXISTING"));
    incompatible_pair
        .rollback()
        .await
        .expect("rollback incompatible pair fixture");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn dual_worker_is_database_admitted_and_missing_sample_blocks_promotion() {
    let (mut db, _data_dir) = fixture("admission_missing_sample").await;
    let worker = create_admitted_worker(&db, "admission-missing-sample").await;
    let admission: (i64, Uuid, Uuid, Uuid, Uuid, String, i16) = sqlx::query_as(
        r#"SELECT admission_seq,operation_id,stage_execution_id,stage_run_unit_id,
                  organization_id,runtime_memory_contract,rollout_rank
             FROM runtime_memory_rollout_admissions
            WHERE worker_run_id=$1"#,
    )
    .bind(worker.worker_run_id)
    .fetch_one(db.pool())
    .await
    .expect("the DB trigger admits every dual-contract worker");
    assert_eq!(admission.1, worker.operation_id);
    assert_eq!(admission.2, worker.stage_execution_id);
    assert_eq!(admission.3, worker.stage_run_unit_id);
    assert_eq!(admission.4, worker.organization_id);
    assert_eq!(admission.5, "dual_write_legacy_read");
    assert_eq!(admission.6, 1);
    let direct_admission = sqlx::query(
        r#"INSERT INTO runtime_memory_rollout_admissions(
               worker_run_id,operation_id,stage_execution_id,stage_run_unit_id,
               organization_id,runtime_memory_contract,rollout_rank,rollout_row_version
           ) SELECT worker_run_id,operation_id,stage_execution_id,stage_run_unit_id,
                    organization_id,runtime_memory_contract,rollout_rank,rollout_row_version
               FROM runtime_memory_rollout_admissions WHERE worker_run_id=$1"#,
    )
    .bind(worker.worker_run_id)
    .execute(db.pool())
    .await;
    assert!(
        direct_admission.is_err(),
        "admission rows can only be emitted by the WorkerRun trigger"
    );

    let outcome = runtime_memory_rollout::reconcile(db.pool())
        .await
        .expect("missing samples are an expected typed no-op");
    assert!(matches!(
        outcome,
        runtime_memory_rollout::RuntimeMemoryRolloutReconcileOutcome::NotReady {
            reason,
            ..
        } if reason == "runtime_shadow_sample_missing"
    ));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn migration_backfill_rehydrates_missing_dual_worker_samples() {
    let data_dir = tempfile::tempdir().expect("temporary upgrade postgres data directory");
    let config = DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("runtime_upgrade_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    };
    let connection_string = config.connection_string();
    let mut embedded = EmbeddedPg::start(config)
        .await
        .expect("start pre-attestation embedded postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&connection_string)
        .await
        .expect("connect pre-attestation upgrade pool");
    migration_subset(i64::MIN, 20260712000016)
        .run(&pool)
        .await
        .expect("apply migrations through Candidate cohort rollout");

    // Current repository code takes the pair lock before operation creation;
    // the real function arrives in 00017, so this pre-upgrade fixture supplies
    // only the old binary's no-op equivalent and removes it before migration.
    sqlx::raw_sql(
        r#"CREATE FUNCTION lock_execution_rollout_pair()
           RETURNS VOID AS $$ BEGIN RETURN; END; $$ LANGUAGE plpgsql;
           CREATE TABLE operation_turns (
               id UUID PRIMARY KEY,
               operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE CASCADE,
               ordinal BIGINT NOT NULL,
               trigger_input TEXT NOT NULL,
               status TEXT NOT NULL,
               started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
               terminal_at TIMESTAMPTZ
           );"#,
    )
    .execute(&pool)
    .await
    .expect("install pre-attestation repository compatibility stubs");
    let matching = create_admitted_worker_in_pool(&pool, "upgrade-matching").await;
    install_pre_attestation_worker_as_legacy(&pool, matching, "upgrade-matching").await;
    let missing = create_admitted_worker_in_pool(&pool, "upgrade-missing").await;
    sqlx::query("DROP FUNCTION lock_execution_rollout_pair()")
        .execute(&pool)
        .await
        .expect("remove compatibility stub before installing real authority");

    migration_subset(20260712000017, 20260712000017)
        .run(&pool)
        .await
        .expect("upgrade existing dual WorkerRuns through attestation migration");
    let samples: Vec<(Uuid, String, String)> = sqlx::query_as(
        r#"SELECT worker_run_id,mutation_kind,comparison
             FROM runtime_memory_shadow_samples
            ORDER BY worker_run_id"#,
    )
    .fetch_all(&pool)
    .await
    .expect("load immutable migration observations");
    assert_eq!(samples.len(), 2);
    assert!(samples.iter().any(|sample| {
        sample.0 == matching.worker_run_id
            && sample.1 == "migration_backfill"
            && sample.2 == "match"
    }));
    assert!(samples.iter().any(|sample| {
        sample.0 == missing.worker_run_id
            && sample.1 == "migration_backfill"
            && sample.2 == "legacy_missing"
    }));
    let outcome = runtime_memory_rollout::reconcile(&pool)
        .await
        .expect("migration mismatch is a typed durable blocker");
    assert!(matches!(
        outcome,
        runtime_memory_rollout::RuntimeMemoryRolloutReconcileOutcome::NotReady {
            reason,
            ..
        } if reason == "runtime_shadow_retained_mismatch"
    ));

    migration_subset(20260712000017, 20260712000017)
        .run(&pool)
        .await
        .expect("exact migration replay must be idempotent");
    let replay_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM runtime_memory_shadow_samples")
            .fetch_one(&pool)
            .await
            .expect("count migration samples after replay");
    assert_eq!(replay_count, 2);
    let inserted_again: i64 = sqlx::query_scalar("SELECT backfill_runtime_memory_shadow_samples()")
        .fetch_one(&pool)
        .await
        .expect("explicit exact backfill replay is supported");
    assert_eq!(inserted_again, 0);

    pool.close().await;
    embedded.stop().await;
}

#[tokio::test]
#[serial]
async fn migration_preflight_rejects_existing_v2_only_legacy_checkpoint_state() {
    let data_dir = tempfile::tempdir().expect("temporary preflight postgres directory");
    let config = DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("runtime_preflight_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    };
    let connection_string = config.connection_string();
    let mut embedded = EmbeddedPg::start(config)
        .await
        .expect("start pre-attestation embedded postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&connection_string)
        .await
        .expect("connect pre-attestation pool");
    migration_subset(i64::MIN, 20260712000016)
        .run(&pool)
        .await
        .expect("apply migrations through Candidate cohort rollout");

    let session = sessions::create(
        &pool,
        NewSession {
            title: Some("V2-only preflight fixture".to_string()),
            workspace_path: Some("/tmp/runtime-v2-preflight".to_string()),
            workspace_label: None,
            model: None,
            provider: None,
            project_path: Some("/tmp/runtime-v2-preflight".to_string()),
        },
    )
    .await
    .expect("create preflight session");
    let operation_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO tasks(id,session_id,title,input,status)
           VALUES($1,$2,'V2-only preflight','reject legacy checkpoint','running')"#,
    )
    .bind(operation_id)
    .bind(session.id)
    .execute(&pool)
    .await
    .expect("insert preflight task");
    sqlx::query(
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
    .bind(operation_id)
    .execute(&pool)
    .await
    .expect("seed pre-attestation V2-only legacy checkpoint");

    let error = migration_subset(20260712000017, 20260712000017)
        .run(&pool)
        .await
        .expect_err("attestation install must fail closed on V2-only legacy checkpoints");
    assert!(
        error
            .to_string()
            .contains("V2_ONLY_LEGACY_CHECKPOINT_EXISTING"),
        "unexpected preflight error: {error}"
    );

    pool.close().await;
    embedded.stop().await;
}

#[tokio::test]
#[serial]
async fn nested_trigger_cannot_forge_runtime_admission_fields() {
    let (mut db, _data_dir) = fixture("nested_admission_owner").await;
    let owner = create_admitted_worker(&db, "nested-admission-owner").await;
    let hostile_worker_id = Uuid::new_v4();

    sqlx::raw_sql(
        "ALTER TABLE stage_worker_runs
             DISABLE TRIGGER runtime_memory_worker_admission",
    )
    .execute(db.pool())
    .await
    .expect("disable owner admission trigger for hostile fixture only");
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,organization_id,
               worker_generation,specialist,work_item_kind,work_item_key,agent_path,status
           ) VALUES($1,$2,$3,$4,$5,91,'target-intel-specialist','organization',
                    'hostile-admission','root>hostile-admission','queued')"#,
    )
    .bind(hostile_worker_id)
    .bind(owner.operation_id)
    .bind(owner.stage_execution_id)
    .bind(owner.stage_run_unit_id)
    .bind(owner.organization_id)
    .execute(db.pool())
    .await
    .expect("install an unadmitted WorkerRun in disposable hostile fixture");
    sqlx::raw_sql(
        "ALTER TABLE stage_worker_runs
             ENABLE TRIGGER runtime_memory_worker_admission",
    )
    .execute(db.pool())
    .await
    .expect("restore owner admission trigger");

    sqlx::raw_sql(
        r#"CREATE TABLE runtime_admission_hostile_input(
               worker_run_id UUID NOT NULL,
               operation_id UUID NOT NULL,
               stage_execution_id UUID NOT NULL,
               stage_run_unit_id UUID NOT NULL,
               organization_id UUID NOT NULL
           );
           CREATE FUNCTION forge_runtime_admission_from_nested_trigger()
           RETURNS trigger AS $$
           BEGIN
               INSERT INTO runtime_memory_rollout_admissions(
                   admission_seq,worker_run_id,operation_id,stage_execution_id,stage_run_unit_id,
                   organization_id,runtime_memory_contract,rollout_rank,
                   rollout_row_version,admitted_at
               ) VALUES(
                   999999999,NEW.worker_run_id,NEW.operation_id,NEW.stage_execution_id,
                   NEW.stage_run_unit_id,NEW.organization_id,
                   'dual_write_legacy_read',1,999,'2000-01-01T00:00:00Z'
               );
               RETURN NEW;
           END;
           $$ LANGUAGE plpgsql;
           CREATE TRIGGER runtime_admission_hostile_nested
           AFTER INSERT ON runtime_admission_hostile_input
           FOR EACH ROW EXECUTE FUNCTION forge_runtime_admission_from_nested_trigger();"#,
    )
    .execute(db.pool())
    .await
    .expect("install nested admission forgery trigger");
    sqlx::query(
        r#"INSERT INTO runtime_admission_hostile_input(
               worker_run_id,operation_id,stage_execution_id,stage_run_unit_id,organization_id
           ) VALUES($1,$2,$3,$4,$5)"#,
    )
    .bind(hostile_worker_id)
    .bind(owner.operation_id)
    .bind(owner.stage_execution_id)
    .bind(owner.stage_run_unit_id)
    .bind(owner.organization_id)
    .execute(db.pool())
    .await
    .expect("nested insert may only persist server-derived admission truth");

    let admission: (i64, Uuid, Uuid, Uuid, Uuid, String, i16, i64, bool) = sqlx::query_as(
        r#"SELECT admission_seq,operation_id,stage_execution_id,stage_run_unit_id,organization_id,
                  runtime_memory_contract,rollout_rank,rollout_row_version,
                  admitted_at > NOW()-INTERVAL '1 minute'
             FROM runtime_memory_rollout_admissions
            WHERE worker_run_id=$1"#,
    )
    .bind(hostile_worker_id)
    .fetch_one(db.pool())
    .await
    .expect("load nested admission");
    let rollout = runtime_memory_rollout::get(db.pool())
        .await
        .expect("load current rollout authority");
    assert_ne!(admission.0, 999999999);
    assert_eq!(admission.1, owner.operation_id);
    assert_eq!(admission.2, owner.stage_execution_id);
    assert_eq!(admission.3, owner.stage_run_unit_id);
    assert_eq!(admission.4, owner.organization_id);
    assert_eq!(admission.5, "dual_write_legacy_read");
    assert_eq!(admission.6, 1);
    assert_eq!(admission.7, rollout.row_version);
    assert!(admission.8, "admitted_at must be database-authored now");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn nested_trigger_cannot_preseed_runtime_promotion_receipt() {
    let (mut db, _data_dir) = fixture("nested_receipt_owner").await;
    create_admitted_worker(&db, "nested-receipt-owner").await;
    sqlx::raw_sql(
        r#"CREATE TABLE runtime_receipt_hostile_input(id INTEGER PRIMARY KEY);
           CREATE FUNCTION forge_runtime_receipt_from_nested_trigger()
           RETURNS trigger AS $$
           BEGIN
               INSERT INTO runtime_memory_rollout_promotions(
                   from_rank,to_rank,from_contract,to_contract,
                   from_row_version,to_row_version,admission_cutoff,
                   admission_count,sample_count,aggregate_digest
               )
               SELECT 1,2,'dual_write_legacy_read','dual_write_v2_preferred',
                      rollout.row_version,rollout.row_version+1,
                      (SELECT MAX(admission_seq)
                         FROM runtime_memory_rollout_admissions),
                      1,1,repeat('0',64)
                 FROM runtime_memory_rollout AS rollout
                WHERE rollout.singleton_id=1;
               RETURN NEW;
           END;
           $$ LANGUAGE plpgsql;
           CREATE TRIGGER runtime_receipt_hostile_nested
           AFTER INSERT ON runtime_receipt_hostile_input
           FOR EACH ROW EXECUTE FUNCTION forge_runtime_receipt_from_nested_trigger();"#,
    )
    .execute(db.pool())
    .await
    .expect("install nested promotion-receipt forgery trigger");

    let forged = sqlx::query("INSERT INTO runtime_receipt_hostile_input(id) VALUES(1)")
        .execute(db.pool())
        .await;
    assert!(
        forged.is_err(),
        "a nested trigger cannot create a receipt before its rollout transition"
    );
    let receipt_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM runtime_memory_rollout_promotions")
            .fetch_one(db.pool())
            .await
            .expect("count receipts after hostile nested trigger");
    assert_eq!(receipt_count, 0);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn explicit_shadow_sample_sequence_is_database_owned() {
    let (mut db, _data_dir) = fixture("sample_sequence_owner").await;
    let worker = create_admitted_worker(&db, "sample-sequence-owner").await;
    install_current_v2_as_legacy(&db, worker, false).await;

    let supplied = i64::MAX - 1;
    let retained: i64 = sqlx::query_scalar(
        r#"INSERT INTO runtime_memory_shadow_samples(
               sample_seq,worker_run_id,mutation_kind
           ) VALUES($1,$2,'hostile_explicit_sample_seq')
           RETURNING sample_seq"#,
    )
    .bind(supplied)
    .bind(worker.worker_run_id)
    .fetch_one(db.pool())
    .await
    .expect("database may retain only a server-ordered sample");
    assert_ne!(
        retained, supplied,
        "caller-owned ordinals can permanently preoccupy the latest sample"
    );

    sqlx::query(
        "UPDATE stage_worker_runs SET checkpoint=jsonb_build_object('server','newer'), \
         checkpoint_version=checkpoint_version+1 WHERE id=$1",
    )
    .bind(worker.worker_run_id)
    .execute(db.pool())
    .await
    .expect("advance current V2 truth");
    install_current_v2_as_legacy(&db, worker, false).await;
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin later legitimate sample");
    let latest = runtime_memory_shadow::persist_worker_sample(
        &mut tx,
        worker.worker_run_id,
        "test_legitimate_sample_after_explicit_sequence",
    )
    .await
    .expect("persist later legitimate sample");
    tx.commit().await.expect("commit later legitimate sample");
    assert!(latest.sample_seq > retained);

    assert!(matches!(
        runtime_memory_rollout::reconcile(db.pool())
            .await
            .expect("server-ordered latest sample permits promotion"),
        runtime_memory_rollout::RuntimeMemoryRolloutReconcileOutcome::Promoted(_)
    ));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn matching_whole_record_sample_promotes_once_and_receipts_are_immutable() {
    let (mut db, _data_dir) = fixture("matching_sample").await;
    let worker = create_admitted_worker(&db, "matching-sample").await;
    install_current_v2_as_legacy(&db, worker, false).await;
    let mut tx = db.pool().begin().await.expect("begin retained sample");
    let sample = runtime_memory_shadow::persist_worker_sample(
        &mut tx,
        worker.worker_run_id,
        "test_matching_whole_record",
    )
    .await
    .expect("persist complete matching sample");
    tx.commit().await.expect("commit retained sample");
    assert_eq!(sample.comparison, "match");
    assert_eq!(sample.selected_source, "legacy");
    assert_eq!(sample.legacy_record_hash, sample.v2_record_hash);

    let promoted = runtime_memory_rollout::reconcile(db.pool())
        .await
        .expect("matching cohort may promote");
    let promoted = match promoted {
        runtime_memory_rollout::RuntimeMemoryRolloutReconcileOutcome::Promoted(row) => row,
        other => panic!("matching cohort did not promote: {other:?}"),
    };
    assert_eq!(promoted.contract, "dual_write_v2_preferred");
    assert_eq!(promoted.contract_rank, 2);
    assert_eq!(promoted.row_version, 2);

    let receipt: (i16, i16, i64, i64, String) = sqlx::query_as(
        r#"SELECT from_rank,to_rank,admission_cutoff,admission_count,aggregate_digest
             FROM runtime_memory_rollout_promotions WHERE from_rank=1"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("DB transition trigger owns a promotion receipt");
    assert_eq!((receipt.0, receipt.1), (1, 2));
    assert!(receipt.2 > 0);
    assert_eq!(receipt.3, 1);
    assert_eq!(receipt.4.len(), 64);

    for result in [
        sqlx::query(
            "UPDATE runtime_memory_shadow_samples SET mutation_kind='tampered' WHERE sample_seq=$1",
        )
        .bind(sample.sample_seq)
        .execute(db.pool())
        .await,
        sqlx::query("DELETE FROM runtime_memory_rollout_admissions WHERE worker_run_id=$1")
            .bind(worker.worker_run_id)
            .execute(db.pool())
            .await,
        sqlx::query("DELETE FROM runtime_memory_rollout_promotions WHERE from_rank=1")
            .execute(db.pool())
            .await,
    ] {
        assert!(result.is_err(), "attestation authority must be immutable");
    }
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn rank_two_requires_its_own_cohort_before_v2_only_and_selects_v2() {
    let (mut db, _data_dir) = fixture("rank_two_cohort").await;
    let rank_one = create_admitted_worker(&db, "rank-one-cohort").await;
    install_current_v2_as_legacy(&db, rank_one, false).await;
    let mut first_sample = db.pool().begin().await.expect("begin rank-one sample");
    runtime_memory_shadow::persist_worker_sample(
        &mut first_sample,
        rank_one.worker_run_id,
        "test_rank_one_match",
    )
    .await
    .expect("persist rank-one sample");
    first_sample.commit().await.expect("commit rank-one sample");
    assert!(matches!(
        runtime_memory_rollout::reconcile(db.pool())
            .await
            .expect("promote to rank two"),
        runtime_memory_rollout::RuntimeMemoryRolloutReconcileOutcome::Promoted(ref row)
            if row.contract_rank == 2
    ));

    let rank_two_without_cohort = runtime_memory_rollout::get(db.pool())
        .await
        .expect("read rank-two default");
    let public = runtime_memory_rollout::advance(
        db.pool(),
        RuntimeMemoryContract::DualWriteV2Preferred,
        RuntimeMemoryContract::V2Only,
        rank_two_without_cohort.row_version,
    )
    .await;
    assert!(public.is_err(), "rank two cannot reuse the rank-one cohort");
    let raw = sqlx::query(
        "UPDATE runtime_memory_rollout SET contract='v2_only',contract_rank=3,row_version=3,updated_at=NOW() WHERE singleton_id=1",
    )
    .execute(db.pool())
    .await;
    assert!(
        raw.is_err(),
        "raw rank-two promotion also needs a new cohort"
    );

    let rank_two = create_admitted_worker(&db, "rank-two-cohort").await;
    install_current_v2_as_legacy(&db, rank_two, false).await;
    let mut second_sample = db.pool().begin().await.expect("begin rank-two sample");
    let sample = runtime_memory_shadow::persist_worker_sample(
        &mut second_sample,
        rank_two.worker_run_id,
        "test_rank_two_match",
    )
    .await
    .expect("persist rank-two sample");
    second_sample
        .commit()
        .await
        .expect("commit rank-two sample");
    assert_eq!(sample.runtime_memory_contract, "dual_write_v2_preferred");
    assert_eq!(sample.selected_source, "v2");

    let promoted = runtime_memory_rollout::reconcile(db.pool())
        .await
        .expect("rank-two cohort promotes to V2-only");
    assert!(matches!(
        promoted,
        runtime_memory_rollout::RuntimeMemoryRolloutReconcileOutcome::Promoted(ref row)
            if row.contract == "v2_only" && row.contract_rank == 3 && row.row_version == 3
    ));
    let receipt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM runtime_memory_rollout_promotions WHERE from_rank IN (1,2)",
    )
    .fetch_one(db.pool())
    .await
    .expect("count both database-owned receipts");
    assert_eq!(receipt_count, 2);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn retained_mismatch_cannot_be_replaced_by_a_later_matching_sample() {
    let (mut db, _data_dir) = fixture("retained_mismatch").await;
    let worker = create_admitted_worker(&db, "retained-mismatch").await;
    install_current_v2_as_legacy(&db, worker, true).await;
    let mut first = db.pool().begin().await.expect("begin mismatch sample");
    let mismatch = runtime_memory_shadow::persist_worker_sample(
        &mut first,
        worker.worker_run_id,
        "test_retained_mismatch",
    )
    .await
    .expect("retain a mismatch rather than losing the observation");
    first.commit().await.expect("commit mismatch sample");
    assert_eq!(mismatch.comparison, "mismatch");

    install_current_v2_as_legacy(&db, worker, false).await;
    let mut second = db.pool().begin().await.expect("begin repaired sample");
    let repaired = runtime_memory_shadow::persist_worker_sample(
        &mut second,
        worker.worker_run_id,
        "test_later_match",
    )
    .await
    .expect("retain later matching observation");
    second.commit().await.expect("commit repaired sample");
    assert_eq!(repaired.comparison, "match");

    let outcome = runtime_memory_rollout::reconcile(db.pool())
        .await
        .expect("historical mismatch is a typed not-ready outcome");
    assert!(matches!(
        outcome,
        runtime_memory_rollout::RuntimeMemoryRolloutReconcileOutcome::NotReady {
            reason,
            ..
        } if reason == "runtime_shadow_retained_mismatch"
    ));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn current_truth_drift_after_sampling_blocks_raw_and_repository_promotion() {
    let (mut db, _data_dir) = fixture("current_truth_drift").await;
    let worker = create_admitted_worker(&db, "current-truth-drift").await;
    install_current_v2_as_legacy(&db, worker, false).await;
    let mut sample_tx = db.pool().begin().await.expect("begin current sample");
    runtime_memory_shadow::persist_worker_sample(
        &mut sample_tx,
        worker.worker_run_id,
        "test_pre_drift_match",
    )
    .await
    .expect("persist pre-drift match");
    sample_tx.commit().await.expect("commit pre-drift match");

    sqlx::query(
        "UPDATE stage_worker_runs SET checkpoint=jsonb_build_object('raw','drift'), checkpoint_version=checkpoint_version+1 WHERE id=$1",
    )
    .bind(worker.worker_run_id)
    .execute(db.pool())
    .await
    .expect("hostile raw V2 mutation");
    let outcome = runtime_memory_rollout::reconcile(db.pool())
        .await
        .expect("current truth drift is a typed no-op");
    assert!(matches!(
        outcome,
        runtime_memory_rollout::RuntimeMemoryRolloutReconcileOutcome::NotReady {
            reason,
            ..
        } if reason == "runtime_shadow_latest_sample_stale"
    ));
    let raw = sqlx::query(
        "UPDATE runtime_memory_rollout SET contract='dual_write_v2_preferred',contract_rank=2,row_version=2,updated_at=NOW() WHERE singleton_id=1",
    )
    .execute(db.pool())
    .await;
    assert!(
        raw.is_err(),
        "raw SQL must rehydrate the same current truth"
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn runtime_attack_compatibility_matrix_is_database_enforced() {
    let (mut db, _data_dir) = fixture("compatibility_matrix").await;
    let pair_constraint_validated: bool = sqlx::query_scalar(
        r#"SELECT convalidated
             FROM pg_constraint
            WHERE conname='operation_rollout_contract_pair_compatible'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("operation contract-pair constraint must be validated after migration scan");
    assert!(pair_constraint_validated);
    for (runtime_rank, attack_rank, expected) in [
        (0_i16, 0_i16, true),
        (0, 1, false),
        (1, 1, true),
        (1, 2, true),
        (1, 3, false),
        (2, 3, false),
        (3, 3, true),
    ] {
        let actual: bool = sqlx::query_scalar("SELECT execution_rollout_pair_is_compatible($1,$2)")
            .bind(runtime_rank)
            .bind(attack_rank)
            .fetch_one(db.pool())
            .await
            .expect("evaluate DB compatibility matrix");
        assert_eq!(
            actual, expected,
            "runtime={runtime_rank}, attack={attack_rank}"
        );
    }

    sqlx::raw_sql(
        "ALTER TABLE attack_execution_rollout DISABLE TRIGGER attack_execution_rollout_forward_only",
    )
    .execute(db.pool())
    .await
    .expect("disable unrelated attack cohort gate in hostile fixture");
    let incompatible = sqlx::query(
        "UPDATE attack_execution_rollout SET contract='v2_only',rank=3,row_version=row_version+1,updated_at=NOW() WHERE singleton=TRUE",
    )
    .execute(db.pool())
    .await;
    assert!(
        incompatible.is_err(),
        "the independent compatibility trigger must reject attack V2-only on runtime rank one"
    );
    sqlx::raw_sql(
        "ALTER TABLE attack_execution_rollout ENABLE TRIGGER attack_execution_rollout_forward_only",
    )
    .execute(db.pool())
    .await
    .expect("restore attack cohort gate after hostile fixture");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn concurrent_late_old_contract_admission_lands_after_promotion_cutoff() {
    let (mut db, _data_dir) = fixture("late_admission").await;
    let worker = create_admitted_worker(&db, "late-admission").await;
    install_current_v2_as_legacy(&db, worker, false).await;
    let mut sample_tx = db.pool().begin().await.expect("begin ready sample");
    runtime_memory_shadow::persist_worker_sample(
        &mut sample_tx,
        worker.worker_run_id,
        "test_ready_before_late_admission",
    )
    .await
    .expect("persist ready sample");
    sample_tx.commit().await.expect("commit ready sample");

    let mut promotion = db.pool().begin().await.expect("begin held promotion");
    sqlx::query(
        "UPDATE runtime_memory_rollout SET contract='dual_write_v2_preferred',contract_rank=2,row_version=2,updated_at=NOW() WHERE singleton_id=1",
    )
    .execute(&mut *promotion)
    .await
    .expect("gate ready promotion while holding commit");
    let late_worker_id = Uuid::new_v4();
    let pool = db.pool().clone();
    let mut late = tokio::spawn(async move {
        sqlx::query(
            r#"INSERT INTO stage_worker_runs(
                   id,operation_id,stage_execution_id,stage_run_unit_id,organization_id,
                   worker_generation,specialist,work_item_kind,work_item_key,agent_path,status
               ) VALUES($1,$2,$3,$4,$5,1,'target-intel-specialist','organization',$6,$7,'queued')"#,
        )
        .bind(late_worker_id)
        .bind(worker.operation_id)
        .bind(worker.stage_execution_id)
        .bind(worker.stage_run_unit_id)
        .bind(worker.organization_id)
        .bind(format!("late-{}", worker.organization_id))
        .bind(format!("root>late:{late_worker_id}"))
        .execute(&pool)
        .await
    });
    assert!(
        timeout(Duration::from_millis(150), &mut late)
            .await
            .is_err(),
        "late admission must wait for the rollout cutoff lock"
    );
    promotion.commit().await.expect("commit ready promotion");
    late.await
        .expect("late admission task remains alive after lock observation")
        .expect("old frozen operation may admit a late worker after promotion");
    let (late_seq, late_contract): (i64, String) = sqlx::query_as(
        "SELECT admission_seq,runtime_memory_contract FROM runtime_memory_rollout_admissions WHERE worker_run_id=$1",
    )
    .bind(late_worker_id)
    .fetch_one(db.pool())
    .await
    .expect("read late old-contract admission");
    let cutoff: i64 = sqlx::query_scalar(
        "SELECT admission_cutoff FROM runtime_memory_rollout_promotions WHERE from_rank=1",
    )
    .fetch_one(db.pool())
    .await
    .expect("read frozen promotion cutoff");
    assert!(late_seq > cutoff);
    assert_eq!(late_contract, "dual_write_legacy_read");
    db.stop().await;
}
