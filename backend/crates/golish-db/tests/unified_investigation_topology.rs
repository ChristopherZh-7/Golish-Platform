use golish_db::repo::operation_state;
use golish_db::{DbConfig, GolishDb};
use serial_test::serial;
use uuid::Uuid;

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read reserved local postgres port")
        .port()
}

async fn migrated_db(label: &str) -> (GolishDb, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("stage_topology_{label}_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start isolated migrated postgres");
    (db, data_dir)
}

async fn select_unified_deployment(db: &GolishDb) {
    let mut transaction = db.pool().begin().await.expect("begin rollout fixture");
    sqlx::query(
        "ALTER TABLE tool_truth_rollout DISABLE TRIGGER tool_truth_rollout_direct_mutation_guard",
    )
    .execute(&mut *transaction)
    .await
    .expect("disable Tool Truth fixture guard");
    sqlx::query(
        "UPDATE tool_truth_rollout SET new_operation_contract='receipt_v1',row_version=row_version+1",
    )
    .execute(&mut *transaction)
    .await
    .expect("select receipt Tool Truth fixture contract");
    sqlx::query(
        "ALTER TABLE tool_truth_rollout ENABLE TRIGGER tool_truth_rollout_direct_mutation_guard",
    )
    .execute(&mut *transaction)
    .await
    .expect("restore Tool Truth fixture guard");

    sqlx::query(
        "ALTER TABLE investigation_rollout DISABLE TRIGGER investigation_rollout_direct_mutation_guard",
    )
    .execute(&mut *transaction)
    .await
    .expect("disable Investigation fixture guard");
    sqlx::query(
        r#"UPDATE investigation_rollout
              SET contract_version='hypothesis_registry_v1',
                  rollout_mode='new_only',mode_rank=4,row_version=row_version+1"#,
    )
    .execute(&mut *transaction)
    .await
    .expect("select unified Investigation fixture contract");
    sqlx::query(
        "ALTER TABLE investigation_rollout ENABLE TRIGGER investigation_rollout_direct_mutation_guard",
    )
    .execute(&mut *transaction)
    .await
    .expect("restore Investigation fixture guard");
    transaction.commit().await.expect("commit rollout fixture");
}

async fn current_runtime_memory_contract(db: &GolishDb) -> String {
    sqlx::query_scalar("SELECT contract FROM runtime_memory_rollout WHERE singleton_id=1")
        .fetch_one(db.pool())
        .await
        .expect("read current runtime-memory deployment contract")
}

#[tokio::test]
#[serial]
async fn topology_sql_catalog_matches_rust_contract_hash_and_closed_ranks() {
    let (db, _data_dir) = migrated_db("catalog").await;
    let values: (
        String,
        String,
        Option<String>,
        i16,
        i16,
        Option<i16>,
        Option<i16>,
    ) = sqlx::query_as(
        r#"SELECT
                stage_topology_contract_sha256('legacy_candidate_verification_v1'),
                stage_topology_contract_sha256('unified_investigation_v1'),
                stage_topology_contract_sha256('future_topology'),
                operation_stage_rank_for_topology(
                    'legacy_candidate_verification_v1','attack_candidate'
                ),
                operation_stage_rank_for_topology(
                    'unified_investigation_v1','application_understanding'
                ),
                operation_stage_rank_for_topology(
                    'unified_investigation_v1','attack_candidate'
                ),
                operation_stage_rank_for_topology('future_topology','investigation')"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("read topology catalog");
    assert_eq!(
        values.0,
        "sha256:bd1f6afec2091958c9806eed93e9764bf5df8df855f03ab6f9d8d8fbf6d725e5"
    );
    assert_eq!(
        values.1,
        "sha256:611faa1101253676becfdb776b95905363737fa2be3116da6d23ab401d9f5859"
    );
    assert_eq!(values.2, None);
    assert_eq!(values.3, 6);
    assert_eq!(values.4, 6);
    assert_eq!(values.5, None);
    assert_eq!(values.6, None);

    let pairs: Vec<(String, String)> = sqlx::query_as(
        r#"SELECT mode,stage_topology_for_investigation_rollout(mode)
             FROM unnest(ARRAY[
                'legacy_only','shadow_registry','dual_read_compare',
                'registry_authoritative_legacy_projection','new_only'
             ]) AS mode
            ORDER BY mode"#,
    )
    .fetch_all(db.pool())
    .await
    .expect("read closed rollout topology pairs");
    assert_eq!(pairs.len(), 5);
    assert_eq!(
        pairs
            .iter()
            .filter(|(_, topology)| topology == "unified_investigation_v1")
            .count(),
        2
    );
}

#[tokio::test]
#[serial]
async fn new_operations_freeze_deployment_topology_and_old_rows_do_not_drift() {
    let (db, _data_dir) = migrated_db("freeze").await;
    let runtime_memory_contract = current_runtime_memory_contract(&db).await;
    let legacy_operation_id = Uuid::new_v4();
    operation_state::insert(
        db.pool(),
        legacy_operation_id,
        "red_team",
        "scoping",
        &runtime_memory_contract,
        golish_core::ApplicationModelContract::LegacyNoModel,
    )
    .await
    .expect("create legacy operation");
    let legacy_before = operation_state::get_stage_topology(db.pool(), legacy_operation_id)
        .await
        .expect("read legacy topology")
        .expect("legacy topology exists");
    assert_eq!(
        legacy_before.stage_topology_contract,
        "legacy_candidate_verification_v1"
    );

    select_unified_deployment(&db).await;
    let unified_operation_id = Uuid::new_v4();
    operation_state::insert(
        db.pool(),
        unified_operation_id,
        "red_team",
        "application_understanding",
        &runtime_memory_contract,
        golish_core::ApplicationModelContract::ApplicationModelV1,
    )
    .await
    .expect("create unified operation");
    let unified = operation_state::get_stage_topology(db.pool(), unified_operation_id)
        .await
        .expect("read unified topology")
        .expect("unified topology exists");
    assert_eq!(unified.stage_topology_contract, "unified_investigation_v1");
    assert_eq!(unified.stage_topology_freeze_source, "deployment_pair_v1");

    let legacy_after = operation_state::get_stage_topology(db.pool(), legacy_operation_id)
        .await
        .expect("re-read legacy topology")
        .expect("legacy topology remains");
    assert_eq!(legacy_before, legacy_after);
    assert!(sqlx::query(
        "UPDATE operation_state SET stage_topology_contract='unified_investigation_v1' WHERE operation_id=$1",
    )
    .bind(legacy_operation_id)
    .execute(db.pool())
    .await
    .is_err());
}

#[tokio::test]
#[serial]
async fn unified_operation_rejects_legacy_stages_and_non_graph_transitions() {
    let (db, _data_dir) = migrated_db("runtime_guard").await;
    let runtime_memory_contract = current_runtime_memory_contract(&db).await;
    select_unified_deployment(&db).await;
    let operation_id = Uuid::new_v4();
    operation_state::insert(
        db.pool(),
        operation_id,
        "red_team",
        "application_understanding",
        "legacy_v1",
        golish_core::ApplicationModelContract::ApplicationModelV1,
    )
    .await
    .expect("create unified operation");

    assert!(sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind) VALUES($1,$2,'attack_candidate')",
    )
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .execute(db.pool())
    .await
    .is_err());
    sqlx::query("INSERT INTO stage_runs(id,operation_id,stage_kind) VALUES($1,$2,'investigation')")
        .bind(Uuid::new_v4())
        .bind(operation_id)
        .execute(db.pool())
        .await
        .expect("insert unified Investigation execution");

    assert!(sqlx::query(
        "UPDATE operation_state SET current_stage='reporting' WHERE operation_id=$1",
    )
    .bind(operation_id)
    .execute(db.pool())
    .await
    .is_err());
    sqlx::query("UPDATE operation_state SET current_stage='investigation' WHERE operation_id=$1")
        .bind(operation_id)
        .execute(db.pool())
        .await
        .expect("advance through exact AU to Investigation edge");
    sqlx::query("UPDATE operation_state SET current_stage='reporting' WHERE operation_id=$1")
        .bind(operation_id)
        .execute(db.pool())
        .await
        .expect("advance through exact Investigation to Reporting edge");
}
