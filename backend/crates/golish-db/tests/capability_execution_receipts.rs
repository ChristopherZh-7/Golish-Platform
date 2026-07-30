use golish_db::{DbConfig, GolishDb};
use serial_test::serial;
use sqlx::{Error as SqlxError, PgPool};
use uuid::Uuid;

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read local postgres port")
        .port()
}

fn digest_v1(nibble: char) -> String {
    assert!(nibble.is_ascii_hexdigit() && !nibble.is_ascii_uppercase());
    format!("sha256:{}", nibble.to_string().repeat(64))
}

fn assert_database_rejection(error: &SqlxError, sqlstate: &str, stable_marker: &str) {
    let database_error = error
        .as_database_error()
        .unwrap_or_else(|| panic!("expected PostgreSQL database error, got {error}"));
    assert_eq!(
        database_error.code().as_deref(),
        Some(sqlstate),
        "unexpected SQLSTATE for {stable_marker}: {error}"
    );
    assert!(
        database_error.message().contains(stable_marker)
            || database_error.constraint() == Some(stable_marker),
        "expected stable marker {stable_marker}, got message={} constraint={:?}",
        database_error.message(),
        database_error.constraint()
    );
}

async fn fixture(label: &str) -> (GolishDb, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("tool_truth_{label}_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start isolated migrated postgres");
    (db, data_dir)
}

#[derive(Debug, Clone)]
struct FrozenExecution {
    session_id: Uuid,
    operation_id: Uuid,
    project_scope_id: Uuid,
    project_path: String,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    outside_organization_id: Uuid,
    stage_execution_id: Uuid,
    other_stage_execution_id: Uuid,
    stage_kind: &'static str,
    stage_run_unit_id: Uuid,
    worker_run_id: Uuid,
    worker_attempt_epoch: i64,
    lease_token: Uuid,
    source_tool_call_id: Uuid,
}

async fn seed_frozen_execution(pool: &PgPool, label: &str) -> FrozenExecution {
    let session_id = Uuid::new_v4();
    let operation_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    let project_path = format!("/tmp/tool-truth-{label}-{}", Uuid::new_v4().simple());
    let scope_decision_id = Uuid::new_v4();
    let scope_snapshot_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let outside_organization_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    let other_stage_execution_id = Uuid::new_v4();
    let stage_run_unit_id = Uuid::new_v4();
    let worker_run_id = Uuid::new_v4();
    let worker_attempt_epoch = 0_i64;
    let lease_token = Uuid::new_v4();
    let source_tool_call_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO sessions(id,title,status,project_path) \
         VALUES($1,'tool truth fixture','running',$2)",
    )
    .bind(session_id)
    .bind(&project_path)
    .execute(pool)
    .await
    .expect("insert isolated fixture session");
    sqlx::query(
        "INSERT INTO tasks(id,session_id,title,input,status) \
         VALUES($1,$2,'tool truth operation','fixture','running')",
    )
    .bind(operation_id)
    .bind(session_id)
    .execute(pool)
    .await
    .expect("insert isolated fixture task");
    sqlx::query(
        "INSERT INTO project_scopes(project_scope_id,canonical_project_path,path_sha256) \
         VALUES($1,$2,$3)",
    )
    .bind(project_scope_id)
    .bind(&project_path)
    .bind(digest_v1('1'))
    .execute(pool)
    .await
    .expect("insert frozen project scope");
    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,project_scope_id
           ) VALUES($1,'assessment','enumeration','legacy_v1',$2)"#,
    )
    .bind(operation_id)
    .bind(project_scope_id)
    .execute(pool)
    .await
    .expect("insert operation with deployment-owned Tool Truth default");
    sqlx::query(
        "INSERT INTO organizations(id,project_path,name) VALUES \
             ($1,$3,'Tool Truth Scoped Org'), \
             ($2,$3,'Tool Truth Outside Org')",
    )
    .bind(organization_id)
    .bind(outside_organization_id)
    .bind(&project_path)
    .execute(pool)
    .await
    .expect("insert scoped and outside organizations");
    sqlx::query(
        r#"INSERT INTO stage_runs(id,operation_id,stage_kind,status) VALUES
               ($1,$3,'enumeration','started'),
               ($2,$3,'vuln_triage','started')"#,
    )
    .bind(stage_execution_id)
    .bind(other_stage_execution_id)
    .bind(operation_id)
    .execute(pool)
    .await
    .expect("insert fixture stage executions");
    sqlx::query(
        r#"INSERT INTO operation_scope_decisions(
               id,operation_id,project_scope_id,stage_execution_id,
               root_organization_id,mode,decision_rows,decision_hash
           ) VALUES($1,$2,$3,$4,$5,'cli_flags',$6,$7)"#,
    )
    .bind(scope_decision_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(stage_execution_id)
    .bind(organization_id)
    .bind(serde_json::json!([{"organization_id": organization_id}]))
    .bind(digest_v1('2'))
    .execute(pool)
    .await
    .expect("insert fixture scope decision");

    let mut scope_tx = pool.begin().await.expect("begin frozen scope transaction");
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
    .bind(digest_v1('3'))
    .execute(&mut *scope_tx)
    .await
    .expect("insert frozen scope snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units(
               snapshot_id,organization_id,organization_name_at_freeze,
               role,depth,ordinal,decision_row_id,approval_source
           ) VALUES($1,$2,'Tool Truth Scoped Org','root',0,0,'root',$3)"#,
    )
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(serde_json::json!({"source": "fixture"}))
    .execute(&mut *scope_tx)
    .await
    .expect("insert frozen scope member");
    sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
        .bind(scope_snapshot_id)
        .execute(&mut *scope_tx)
        .await
        .expect("seal fixture scope snapshot");
    scope_tx
        .commit()
        .await
        .expect("commit frozen scope transaction");

    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,
               organization_id,stage_kind,generation,specialist,status,started_at
           ) VALUES($1,$2,$3,$4,$5,'enumeration',0,'tool_truth_fixture','running',NOW())"#,
    )
    .bind(stage_run_unit_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .execute(pool)
    .await
    .expect("insert stage Unit bound to frozen scope");
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,
               organization_id,worker_generation,specialist,work_item_kind,
               work_item_key,agent_path,status,lease_token,lease_owner,
               lease_acquired_at,lease_expires_at,heartbeat_at,attempt_epoch
           ) VALUES(
               $1,$2,$3,$4,$5,0,'tool_truth_fixture','stage_unit','fixture',
               'main>enumeration','running',$6,'tool-truth-fixture',NOW(),
               NOW()+INTERVAL '5 minutes',NOW(),$7
           )"#,
    )
    .bind(worker_run_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(organization_id)
    .bind(lease_token)
    .bind(worker_attempt_epoch)
    .execute(pool)
    .await
    .expect("insert live worker fence");
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,status,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token
           ) VALUES(
               $1,'tool-truth-source',$2,$3,'primary','tool_truth_fixture','{}','running',
               $3,$4,$5,$6,$7,$8,$9
           )"#,
    )
    .bind(source_tool_call_id)
    .bind(session_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(worker_run_id)
    .bind(organization_id)
    .bind(worker_attempt_epoch)
    .bind(lease_token)
    .execute(pool)
    .await
    .expect("insert immutable source tool-call fence");

    FrozenExecution {
        session_id,
        operation_id,
        project_scope_id,
        project_path,
        scope_snapshot_id,
        organization_id,
        outside_organization_id,
        stage_execution_id,
        other_stage_execution_id,
        stage_kind: "enumeration",
        stage_run_unit_id,
        worker_run_id,
        worker_attempt_epoch,
        lease_token,
        source_tool_call_id,
    }
}

async fn insert_host_authority(pool: &PgPool, fixture: &FrozenExecution) -> (Uuid, String) {
    let id = Uuid::new_v4();
    sqlx::query_as::<_, (Uuid, String)>(
        r#"INSERT INTO tool_truth_execution_authorities(
               id,stable_authority_request_id,operation_id,project_scope_id,project_path_at_freeze,
               scope_snapshot_id,organization_id,stage_execution_id,stage_kind,
               execution_source_kind,execution_owner_kind,authority_hash
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,
               'stage_execution','host_stage',$10
           )
           RETURNING id,authority_hash"#,
    )
    .bind(id)
    .bind(Uuid::new_v4())
    .bind(fixture.operation_id)
    .bind(fixture.project_scope_id)
    .bind(&fixture.project_path)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.organization_id)
    .bind(fixture.stage_execution_id)
    .bind(fixture.stage_kind)
    .bind(digest_v1('4'))
    .fetch_one(pool)
    .await
    .expect("insert server-validated host-stage execution authority")
}

#[tokio::test]
#[serial]
async fn operation_insert_defaults_tool_truth_contract_to_legacy_v1() {
    let (mut db, _data_dir) = fixture("operation_contract_default").await;
    let frozen = seed_frozen_execution(db.pool(), "operation-default").await;

    let contract: String =
        sqlx::query_scalar("SELECT tool_truth_contract FROM operation_state WHERE operation_id=$1")
            .bind(frozen.operation_id)
            .fetch_one(db.pool())
            .await
            .expect("read operation-frozen Tool Truth contract");
    assert_eq!(contract, "legacy_v1");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn persisted_operation_tool_truth_contract_is_db_immutable() {
    let (mut db, _data_dir) = fixture("operation_contract_immutable").await;
    let frozen = seed_frozen_execution(db.pool(), "operation-immutable").await;

    let error = sqlx::query(
        "UPDATE operation_state SET tool_truth_contract='receipt_v1' WHERE operation_id=$1",
    )
    .bind(frozen.operation_id)
    .execute(db.pool())
    .await
    .expect_err("operation-frozen contract must reject direct SQL UPDATE");
    assert_database_rejection(&error, "23514", "operation_tool_truth_contract_immutable");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn rollout_rejects_direct_update_and_delete() {
    let (mut db, _data_dir) = fixture("rollout_guard").await;

    let update_error = sqlx::query(
        "UPDATE tool_truth_rollout SET new_operation_contract='shadow_v1' WHERE singleton=TRUE",
    )
    .execute(db.pool())
    .await
    .expect_err("frozen rollout must reject direct SQL UPDATE");
    assert_database_rejection(
        &update_error,
        "23514",
        "tool_truth_rollout_direct_mutation_forbidden",
    );

    let delete_error = sqlx::query("DELETE FROM tool_truth_rollout WHERE singleton=TRUE")
        .execute(db.pool())
        .await
        .expect_err("frozen rollout must reject direct SQL DELETE");
    assert_database_rejection(
        &delete_error,
        "23514",
        "tool_truth_rollout_direct_mutation_forbidden",
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn catalog_uses_stage_execution_uuid_and_bigint_worker_attempt_identity() {
    let (mut db, _data_dir) = fixture("identity_catalog").await;

    let forbidden_attempt_epoch_columns: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM information_schema.columns
            WHERE table_schema=current_schema()
              AND table_name IN (
                  'coverage_denominators',
                  'capability_execution_destination_policies',
                  'capability_execution_receipts'
              )
              AND column_name='attempt_epoch'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect stage-owned Tool Truth identity columns");
    assert_eq!(
        forbidden_attempt_epoch_columns, 0,
        "stage attempt identity must only be stage_execution_id"
    );

    let worker_epoch_type: Option<String> = sqlx::query_scalar(
        r#"SELECT data_type
             FROM information_schema.columns
            WHERE table_schema=current_schema()
              AND table_name='tool_truth_execution_authorities'
              AND column_name='worker_attempt_epoch'"#,
    )
    .fetch_optional(db.pool())
    .await
    .expect("inspect worker attempt identity type");
    assert_eq!(worker_epoch_type.as_deref(), Some("bigint"));

    for table_name in ["stage_worker_runs", "tool_calls"] {
        let data_type: Option<String> = sqlx::query_scalar(
            r#"SELECT data_type
                 FROM information_schema.columns
                WHERE table_schema=current_schema()
                  AND table_name=$1
                  AND column_name='attempt_epoch'"#,
        )
        .bind(table_name)
        .fetch_optional(db.pool())
        .await
        .expect("inspect existing worker fence epoch type");
        assert_eq!(data_type.as_deref(), Some("bigint"), "table={table_name}");
    }

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn execution_authority_rejects_cross_organization_scope_member() {
    let (mut db, _data_dir) = fixture("authority_cross_org").await;
    let frozen = seed_frozen_execution(db.pool(), "authority-cross-org").await;

    let error = sqlx::query(
        r#"INSERT INTO tool_truth_execution_authorities(
               id,stable_authority_request_id,operation_id,project_scope_id,project_path_at_freeze,
               scope_snapshot_id,organization_id,stage_execution_id,stage_kind,
               execution_source_kind,execution_owner_kind,authority_hash
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,
               'stage_execution','host_stage',$10
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(frozen.operation_id)
    .bind(frozen.project_scope_id)
    .bind(&frozen.project_path)
    .bind(frozen.scope_snapshot_id)
    .bind(frozen.outside_organization_id)
    .bind(frozen.stage_execution_id)
    .bind(frozen.stage_kind)
    .bind(digest_v1('5'))
    .execute(db.pool())
    .await
    .expect_err("outside organization must not join a frozen scope authority");
    assert_database_rejection(&error, "23514", "tool_truth_authority_scope_org_mismatch");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn execution_authority_rejects_cross_scope_snapshot() {
    let (mut db, _data_dir) = fixture("authority_cross_scope").await;
    let frozen = seed_frozen_execution(db.pool(), "authority-cross-scope-a").await;
    let foreign = seed_frozen_execution(db.pool(), "authority-cross-scope-b").await;

    let error = sqlx::query(
        r#"INSERT INTO tool_truth_execution_authorities(
               id,stable_authority_request_id,operation_id,project_scope_id,project_path_at_freeze,
               scope_snapshot_id,organization_id,stage_execution_id,stage_kind,
               execution_source_kind,execution_owner_kind,authority_hash
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,
               'stage_execution','host_stage',$10
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(frozen.operation_id)
    .bind(frozen.project_scope_id)
    .bind(&frozen.project_path)
    .bind(foreign.scope_snapshot_id)
    .bind(frozen.organization_id)
    .bind(frozen.stage_execution_id)
    .bind(frozen.stage_kind)
    .bind(digest_v1('6'))
    .execute(db.pool())
    .await
    .expect_err("foreign snapshot must not join an operation authority");
    assert_database_rejection(
        &error,
        "23514",
        "tool_truth_authority_scope_snapshot_mismatch",
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn execution_authority_rejects_cross_stage_execution() {
    let (mut db, _data_dir) = fixture("authority_cross_stage").await;
    let frozen = seed_frozen_execution(db.pool(), "authority-cross-stage").await;

    let error = sqlx::query(
        r#"INSERT INTO tool_truth_execution_authorities(
               id,stable_authority_request_id,operation_id,project_scope_id,project_path_at_freeze,
               scope_snapshot_id,organization_id,stage_execution_id,stage_kind,
               execution_source_kind,execution_owner_kind,authority_hash
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,
               'stage_execution','host_stage',$10
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(frozen.operation_id)
    .bind(frozen.project_scope_id)
    .bind(&frozen.project_path)
    .bind(frozen.scope_snapshot_id)
    .bind(frozen.organization_id)
    .bind(frozen.other_stage_execution_id)
    .bind(frozen.stage_kind)
    .bind(digest_v1('7'))
    .execute(db.pool())
    .await
    .expect_err("stage execution and stage kind must be the same frozen parent");
    assert_database_rejection(&error, "23514", "tool_truth_authority_stage_mismatch");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn worker_execution_rejects_old_epoch_with_new_lease() {
    let (mut db, _data_dir) = fixture("authority_worker_fence").await;
    let frozen = seed_frozen_execution(db.pool(), "authority-worker-fence").await;
    let forged_lease = Uuid::new_v4();
    assert_ne!(forged_lease, frozen.lease_token);

    let error = sqlx::query(
        r#"INSERT INTO tool_truth_execution_authorities(
               id,stable_authority_request_id,operation_id,project_scope_id,project_path_at_freeze,
               scope_snapshot_id,organization_id,stage_execution_id,stage_kind,
               execution_source_kind,stage_run_unit_id,execution_owner_kind,
               worker_run_id,worker_attempt_epoch,lease_token,source_tool_call_id,
               authority_hash
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,
               'stage_unit',$10,'worker_tool',$11,$12,$13,$14,$15
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(frozen.operation_id)
    .bind(frozen.project_scope_id)
    .bind(&frozen.project_path)
    .bind(frozen.scope_snapshot_id)
    .bind(frozen.organization_id)
    .bind(frozen.stage_execution_id)
    .bind(frozen.stage_kind)
    .bind(frozen.stage_run_unit_id)
    .bind(frozen.worker_run_id)
    .bind(frozen.worker_attempt_epoch)
    .bind(forged_lease)
    .bind(frozen.source_tool_call_id)
    .bind(digest_v1('8'))
    .execute(db.pool())
    .await
    .expect_err("old worker epoch cannot be paired with a forged new lease");
    assert_database_rejection(&error, "23514", "tool_truth_worker_fence_mismatch");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn evidence_adapter_rejects_non_evidence_audit_role() {
    let (mut db, _data_dir) = fixture("evidence_role").await;
    let frozen = seed_frozen_execution(db.pool(), "evidence-role").await;
    let (execution_authority_id, execution_authority_hash) =
        insert_host_authority(db.pool(), &frozen).await;

    let audit_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO audit_log(
               action,category,details,project_path,run_id,audit_role,detail,status
           ) VALUES(
               'tool_truth_fixture','test','not evidence',$1,$2,'action',$3,'completed'
           ) RETURNING id"#,
    )
    .bind(&frozen.project_path)
    .bind(frozen.operation_id)
    .bind(serde_json::json!({"organization_id": frozen.organization_id}))
    .fetch_one(db.pool())
    .await
    .expect("insert non-evidence audit row");
    let classification_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO evidence_classifications(
               evidence_audit_id,classification,scope_version,reason,
               classified_by_session,producing_stage_run_id
           ) VALUES($1,'in_scope',1,'fixture',$2,$3)
           RETURNING id"#,
    )
    .bind(audit_id)
    .bind(frozen.session_id.to_string())
    .bind(frozen.stage_execution_id)
    .fetch_one(db.pool())
    .await
    .expect("insert current classification for non-evidence row");

    let error = sqlx::query(
        r#"INSERT INTO tool_truth_evidence_production_bindings(
               id,execution_authority_id,operation_id,project_scope_id,
               project_path_at_freeze,scope_snapshot_id,organization_id,
               stage_execution_id,stage_kind,execution_authority_hash,
               evidence_audit_id,evidence_classification_id,production_binding_hash
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(execution_authority_id)
    .bind(frozen.operation_id)
    .bind(frozen.project_scope_id)
    .bind(&frozen.project_path)
    .bind(frozen.scope_snapshot_id)
    .bind(frozen.organization_id)
    .bind(frozen.stage_execution_id)
    .bind(frozen.stage_kind)
    .bind(execution_authority_hash)
    .bind(audit_id)
    .bind(classification_id)
    .bind(digest_v1('9'))
    .execute(db.pool())
    .await
    .expect_err("action audit row must not be normalized as Evidence");
    assert_database_rejection(&error, "23514", "tool_truth_evidence_role_invalid");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn business_ref_adapter_rejects_typed_id_shape_confusion() {
    let (mut db, _data_dir) = fixture("business_ref_shape").await;

    let dns_uuid_error = sqlx::query(
        r#"INSERT INTO tool_truth_business_ref_authorities(
               id,execution_authority_id,evidence_authority_id,ref_kind,
               ref_uuid,ref_bigint,source_hash,authority_hash
           ) VALUES($1,$2,$3,'dns_record',$4,NULL,$5,$6)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(digest_v1('d'))
    .bind(digest_v1('e'))
    .execute(db.pool())
    .await
    .expect_err("DNS reference must use the BIGINT typed identity column");
    assert_database_rejection(
        &dns_uuid_error,
        "23514",
        "tool_truth_business_ref_id_shape_invalid",
    );

    let target_bigint_error = sqlx::query(
        r#"INSERT INTO tool_truth_business_ref_authorities(
               id,execution_authority_id,evidence_authority_id,ref_kind,
               ref_uuid,ref_bigint,source_hash,authority_hash
           ) VALUES($1,$2,$3,'target_asset',NULL,42,$4,$5)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(digest_v1('f'))
    .bind(digest_v1('0'))
    .execute(db.pool())
    .await
    .expect_err("UUID reference kind must not use the BIGINT identity column");
    assert_database_rejection(
        &target_bigint_error,
        "23514",
        "tool_truth_business_ref_id_shape_invalid",
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn raw_witness_rejects_missing_reciprocal_receipt_binding() {
    let (mut db, _data_dir) = fixture("raw_reciprocal").await;
    let frozen = seed_frozen_execution(db.pool(), "raw-reciprocal").await;
    let (execution_authority_id, _) = insert_host_authority(db.pool(), &frozen).await;

    let raw_to_receipt_fk_defs: Vec<String> = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(oid)
             FROM pg_constraint
            WHERE conrelid='capability_raw_witness_artifacts'::regclass
              AND contype='f'"#,
    )
    .fetch_all(db.pool())
    .await
    .expect("inspect raw witness foreign keys");
    assert!(
        raw_to_receipt_fk_defs.iter().any(|definition| {
            definition.contains(
                "FOREIGN KEY (receipt_id, execution_authority_id, receipt_authority_hash)",
            ) && definition.contains(
                "capability_execution_receipts(id, execution_authority_id, receipt_authority_hash)",
            )
        }),
        "raw witness must bind the exact receipt authority tuple: {raw_to_receipt_fk_defs:?}"
    );

    let receipt_to_raw_fk_defs: Vec<String> = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(oid)
             FROM pg_constraint
            WHERE conrelid='capability_execution_receipts'::regclass
              AND contype='f'"#,
    )
    .fetch_all(db.pool())
    .await
    .expect("inspect receipt raw-witness back-reference");
    assert!(
        receipt_to_raw_fk_defs.iter().any(|definition| {
            definition.contains("FOREIGN KEY (raw_witness_artifact_id, id, execution_authority_id)")
                && definition.contains(
                    "capability_raw_witness_artifacts(id, receipt_id, execution_authority_id)",
                )
        }),
        "receipt must point back to its own exact raw witness: {receipt_to_raw_fk_defs:?}"
    );

    let error = sqlx::query(
        r#"INSERT INTO capability_raw_witness_artifacts(
               id,receipt_id,execution_authority_id,receipt_authority_hash,
               content_key,vault_object_ref_token,vault_object_ref_token_hash,
               sha256,ciphertext_sha256,encryption_contract_version,
               operation_key_ref_hash,key_generation,retention_policy_id,
               retention_policy_hash,sensitivity_disposition,
               original_byte_count,stored_byte_count,truncated
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,'raw_witness_envelope.v1',
               $10,1,$11,$12,'typed_derivative_ready',1,1,FALSE
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(execution_authority_id)
    .bind(digest_v1('1'))
    .bind(digest_v1('2'))
    .bind(vec![0_u8; 32])
    .bind(digest_v1('3'))
    .bind(digest_v1('4'))
    .bind(digest_v1('5'))
    .bind(digest_v1('6'))
    .bind(Uuid::new_v4())
    .bind(digest_v1('7'))
    .execute(db.pool())
    .await
    .expect_err("raw witness without an exact reciprocal receipt must be rejected");
    assert_database_rejection(
        &error,
        "23503",
        "capability_raw_witness_receipt_authority_fk",
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn sealed_header_rejects_late_denominator_member_insert() {
    let (mut db, _data_dir) = fixture("denominator_late_member").await;
    let frozen = seed_frozen_execution(db.pool(), "denominator-late-member").await;
    let (execution_authority_id, execution_authority_hash) =
        insert_host_authority(db.pool(), &frozen).await;
    let denominator_id = Uuid::new_v4();

    let denominator_hash: String = sqlx::query_scalar(
        r#"INSERT INTO coverage_denominators(
               id,stable_seal_request_id,execution_authority_id,
               operation_id,project_scope_id,project_path_at_freeze,
               scope_snapshot_id,organization_id,stage_execution_id,stage_kind,
               execution_authority_hash,denominator_kind,contract,
               denominator_hash,input_manifest_hash
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,
               'root','shadow_v1',$12,$13
           ) RETURNING denominator_hash"#,
    )
    .bind(denominator_id)
    .bind(Uuid::new_v4())
    .bind(execution_authority_id)
    .bind(frozen.operation_id)
    .bind(frozen.project_scope_id)
    .bind(&frozen.project_path)
    .bind(frozen.scope_snapshot_id)
    .bind(frozen.organization_id)
    .bind(frozen.stage_execution_id)
    .bind(frozen.stage_kind)
    .bind(execution_authority_hash)
    .bind(digest_v1('8'))
    .bind(digest_v1('9'))
    .fetch_one(db.pool())
    .await
    .expect("insert open denominator header");
    sqlx::query(
        r#"INSERT INTO coverage_denominator_items(
               id,denominator_id,execution_authority_id,denominator_hash,
               ordinal,input_key,exact_asset,technique,expected_capability,member_hash
           ) VALUES($1,$2,$3,$4,0,'root','example.test','enumerate','dns',$5)"#,
    )
    .bind(Uuid::new_v4())
    .bind(denominator_id)
    .bind(execution_authority_id)
    .bind(&denominator_hash)
    .bind(digest_v1('b'))
    .execute(db.pool())
    .await
    .expect("insert denominator member before seal");
    sqlx::query("UPDATE coverage_denominators SET sealed_at=statement_timestamp() WHERE id=$1")
        .bind(denominator_id)
        .execute(db.pool())
        .await
        .expect("seal denominator from its exact member set");

    let error = sqlx::query(
        r#"INSERT INTO coverage_denominator_items(
               id,denominator_id,execution_authority_id,denominator_hash,
               ordinal,input_key,exact_asset,technique,expected_capability,member_hash
           ) VALUES($1,$2,$3,$4,1,'late','late.example','enumerate','dns',$5)"#,
    )
    .bind(Uuid::new_v4())
    .bind(denominator_id)
    .bind(execution_authority_id)
    .bind(denominator_hash)
    .bind(digest_v1('c'))
    .execute(db.pool())
    .await
    .expect_err("sealed denominator must reject late direct-SQL members");
    assert_database_rejection(&error, "23514", "tool_truth_sealed_parent_immutable");

    db.stop().await;
}
