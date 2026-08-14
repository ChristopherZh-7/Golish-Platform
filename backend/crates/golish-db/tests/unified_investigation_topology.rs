use golish_db::repo::operation_state;
use golish_db::{DbConfig, GolishDb};
use serial_test::serial;
use uuid::Uuid;

#[derive(Debug, PartialEq, Eq, sqlx::FromRow)]
struct FreshDefaultsRow {
    runtime_contract: String,
    runtime_contract_rank: i16,
    runtime_row_version: i64,
    attack_contract: String,
    attack_rank: i16,
    attack_row_version: i64,
    enumeration_contract: String,
    enumeration_generation: i64,
    tool_truth_contract: String,
    tool_truth_row_version: i64,
    investigation_contract: String,
    investigation_rollout_mode: String,
    investigation_mode_rank: i16,
    investigation_row_version: i64,
    joint_contract_rank: Option<i16>,
}

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

async fn select_legacy_authority_deployment(db: &GolishDb) {
    let mut transaction = db
        .pool()
        .begin()
        .await
        .expect("begin legacy rollout fixture");
    sqlx::query(
        "ALTER TABLE tool_truth_rollout DISABLE TRIGGER tool_truth_rollout_direct_mutation_guard",
    )
    .execute(&mut *transaction)
    .await
    .expect("disable Tool Truth fixture guard");
    sqlx::query("UPDATE tool_truth_rollout SET new_operation_contract='legacy_v1',row_version=0")
        .execute(&mut *transaction)
        .await
        .expect("select legacy Tool Truth fixture contract");
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
              SET contract_version='legacy_candidate_v1',
                  rollout_mode='legacy_only',mode_rank=0,row_version=0"#,
    )
    .execute(&mut *transaction)
    .await
    .expect("select legacy Investigation fixture contract");
    sqlx::query(
        "ALTER TABLE investigation_rollout ENABLE TRIGGER investigation_rollout_direct_mutation_guard",
    )
    .execute(&mut *transaction)
    .await
    .expect("restore Investigation fixture guard");
    transaction
        .commit()
        .await
        .expect("commit legacy rollout fixture");
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
async fn fresh_install_selects_the_accepted_full_chain_defaults() {
    let (db, _data_dir) = migrated_db("fresh_defaults").await;
    let selected = sqlx::query_as::<_, FreshDefaultsRow>(
        r#"SELECT runtime.contract AS runtime_contract,
                      runtime.contract_rank AS runtime_contract_rank,
                      runtime.row_version AS runtime_row_version,
                      attack.contract AS attack_contract,
                      attack.rank AS attack_rank,
                      attack.row_version AS attack_row_version,
                      enumeration.new_operation_contract AS enumeration_contract,
                      enumeration.generation AS enumeration_generation,
                      tool.new_operation_contract AS tool_truth_contract,
                      tool.row_version AS tool_truth_row_version,
                      investigation.contract_version AS investigation_contract,
                      investigation.rollout_mode AS investigation_rollout_mode,
                      investigation.mode_rank AS investigation_mode_rank,
                      investigation.row_version AS investigation_row_version,
                      operation_joint_contract_rank(
                          tool.new_operation_contract,
                          investigation.contract_version,
                          investigation.rollout_mode
                      ) AS joint_contract_rank
                 FROM runtime_memory_rollout runtime
                 CROSS JOIN attack_execution_rollout attack
                 CROSS JOIN enumeration_analysis_rollout enumeration
                 CROSS JOIN tool_truth_rollout tool
                 CROSS JOIN investigation_rollout investigation
                WHERE runtime.singleton_id=1 AND attack.singleton=TRUE
                  AND enumeration.singleton=TRUE AND tool.singleton=TRUE
                  AND investigation.singleton=TRUE"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("read fresh full-chain defaults");
    assert_eq!(
        selected,
        FreshDefaultsRow {
            runtime_contract: "v2_only".to_owned(),
            runtime_contract_rank: 3,
            runtime_row_version: 3,
            attack_contract: "v2_only".to_owned(),
            attack_rank: 3,
            attack_row_version: 3,
            enumeration_contract: "agent_team_v2".to_owned(),
            enumeration_generation: 2,
            tool_truth_contract: "receipt_v1".to_owned(),
            tool_truth_row_version: 1,
            investigation_contract: "hypothesis_registry_v1".to_owned(),
            investigation_rollout_mode: "new_only".to_owned(),
            investigation_mode_rank: 4,
            investigation_row_version: 1,
            joint_contract_rank: Some(6),
        }
    );
    let receipt: (String, String) = sqlx::query_as(
        r#"SELECT bootstrap_mode,receipt_sha256
             FROM fresh_install_full_chain_bootstrap_receipts
            WHERE singleton=TRUE"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("read fresh-install bootstrap receipt");
    assert_eq!(receipt.0, "selected");
    assert!(receipt.1.starts_with("sha256:"));
}

#[tokio::test]
#[serial]
async fn new_operations_freeze_deployment_topology_and_old_rows_do_not_drift() {
    let (db, _data_dir) = migrated_db("freeze").await;
    select_legacy_authority_deployment(&db).await;
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
    select_unified_deployment(&db).await;
    let runtime_memory_contract = current_runtime_memory_contract(&db).await;
    let operation_id = Uuid::new_v4();
    operation_state::insert(
        db.pool(),
        operation_id,
        "red_team",
        "application_understanding",
        &runtime_memory_contract,
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

#[tokio::test]
#[serial]
async fn vuln_formulaic_controller_recovery_migration_keeps_an_exact_trigger_guard() {
    let (db, _data_dir) = migrated_db("vuln_formulaic_recovery").await;
    let applied: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM _sqlx_migrations WHERE version=20260811000003 AND success)",
    )
    .fetch_one(db.pool())
    .await
    .expect("read applied migration");
    assert!(applied);
    let definition: String = sqlx::query_scalar(
        "SELECT pg_get_functiondef('enforce_stage_work_item_contract()'::regprocedure)",
    )
    .fetch_one(db.pool())
    .await
    .expect("read Stage WorkItem trigger definition");
    for witness in [
        "vuln_formulaic_controller_recovery",
        "_runtime_vuln_formulaic_controller_recovery",
        "formulaic_worklist_executor",
        "vuln_v1",
        "outcome IN ('pending','partial','error')",
    ] {
        assert!(
            definition.contains(witness),
            "migration trigger lost exact witness {witness}"
        );
    }
}
