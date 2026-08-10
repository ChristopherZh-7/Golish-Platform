use golish_core::ApplicationModelContract;
use golish_db::models::NewSession;
use golish_db::repo::{operation_state, project_scopes, runtime_memory_tx, sessions};
use golish_db::{DbConfig, GolishDb};
use serial_test::serial;
use tempfile::TempDir;
use uuid::Uuid;

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read reserved local postgres port")
        .port()
}

struct ApplicationModelOperationDb {
    db: GolishDb,
    _data_dir: TempDir,
}

impl ApplicationModelOperationDb {
    async fn start(label: &str) -> Self {
        let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
        let config = DbConfig {
            pg_data_dir: data_dir.path().join("pgdata"),
            port: reserve_local_port(),
            database: format!(
                "application_model_operation_{label}_{}",
                Uuid::new_v4().simple()
            ),
            ..DbConfig::default()
        };
        let db = GolishDb::start(config)
            .await
            .expect("start fresh migrated embedded postgres");
        Self {
            db,
            _data_dir: data_dir,
        }
    }

    async fn create_task_root(&self, label: &str) -> Uuid {
        let session_id = Uuid::new_v4();
        let operation_id = Uuid::new_v4();
        let project_path = format!("/tmp/application-model-operation-{operation_id}");
        sqlx::query(
            r#"INSERT INTO sessions(id,title,status,project_path)
               VALUES($1,$2,'running',$3)"#,
        )
        .bind(session_id)
        .bind(label)
        .bind(project_path)
        .execute(self.db.pool())
        .await
        .expect("insert operation-contract session");
        sqlx::query(
            r#"INSERT INTO tasks(id,session_id,title,input,status)
               VALUES($1,$2,$3,'contract fixture','running')"#,
        )
        .bind(operation_id)
        .bind(session_id)
        .bind(label)
        .execute(self.db.pool())
        .await
        .expect("insert operation-contract task");
        operation_id
    }

    async fn create_runtime_operation(&self, label: &str, entry_stage: &str) -> Uuid {
        let workspace = format!("/tmp/application-model-runtime-{}", Uuid::new_v4().simple());
        let project =
            project_scopes::register_first_open(self.db.pool(), &workspace, &"1".repeat(64))
                .await
                .expect("register application-model project scope");
        let session = sessions::create(
            self.db.pool(),
            NewSession {
                title: Some(label.to_string()),
                workspace_path: Some(workspace.clone()),
                workspace_label: None,
                model: Some("fixture-model".to_string()),
                provider: Some("fixture-provider".to_string()),
                project_path: Some(workspace),
            },
        )
        .await
        .expect("create application-model session");
        let operation_id = Uuid::new_v4();
        runtime_memory_tx::create_runtime_operation(
            self.db.pool(),
            &runtime_memory_tx::CreateRuntimeOperationRow {
                operation_id,
                initial_stage_execution_id: Uuid::new_v4(),
                session_id: session.id,
                title: Some(label.to_string()),
                input: "application-model contract fixture".to_string(),
                profile: "red_team".to_string(),
                entry_stage: entry_stage.to_string(),
                application_model_contract: ApplicationModelContract::ApplicationModelV1,
                project_scope_id: project.project_scope_id,
                cli_scope: None,
            },
        )
        .await
        .expect("create runtime operation");
        operation_id
    }
}

#[tokio::test]
#[serial]
async fn fresh_scoping_operation_freezes_application_model_v1_and_restores_it_typed() {
    let database = ApplicationModelOperationDb::start("fresh-scoping").await;

    let operation_id = database
        .create_runtime_operation("fresh Scoping operation", "scoping")
        .await;

    assert_eq!(
        operation_state::get_application_model_contract(database.db.pool(), operation_id)
            .await
            .expect("restore frozen application-model contract"),
        Some(ApplicationModelContract::ApplicationModelV1)
    );
}

#[tokio::test]
#[serial]
async fn historical_default_remains_legacy_and_restores_without_upgrade() {
    let database = ApplicationModelOperationDb::start("historical-legacy").await;
    let operation_id = database.create_task_root("historical operation").await;

    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,
               attack_execution_contract
           ) VALUES($1,'red_team','attack_candidate','v2_only','v2_only')"#,
    )
    .bind(operation_id)
    .execute(database.db.pool())
    .await
    .expect("insert migration-compatible historical operation");

    assert_eq!(
        operation_state::get_application_model_contract(database.db.pool(), operation_id)
            .await
            .expect("restore historical application-model contract"),
        Some(ApplicationModelContract::LegacyNoModel)
    );
}

#[tokio::test]
#[serial]
async fn stage_fork_target_explicitly_inherits_source_contract_across_candidate_entry() {
    let database = ApplicationModelOperationDb::start("fork-inheritance").await;
    let source_operation_id = database.create_task_root("source operation").await;
    let target_operation_id = database.create_task_root("fork target operation").await;

    operation_state::insert(
        database.db.pool(),
        source_operation_id,
        "red_team",
        "scoping",
        "v2_only",
        ApplicationModelContract::ApplicationModelV1,
    )
    .await
    .expect("insert source operation with an explicit contract");
    let source_contract =
        operation_state::get_application_model_contract(database.db.pool(), source_operation_id)
            .await
            .expect("read source contract")
            .expect("source operation contract");

    operation_state::insert(
        database.db.pool(),
        target_operation_id,
        "red_team",
        "attack_candidate",
        "v2_only",
        source_contract,
    )
    .await
    .expect("insert fork target with the exact source contract");

    assert_eq!(
        operation_state::get_application_model_contract(database.db.pool(), target_operation_id)
            .await
            .expect("restore fork target contract"),
        Some(ApplicationModelContract::ApplicationModelV1),
        "a Candidate entry stage must not silently downgrade a v1 source fork to legacy"
    );
}

#[tokio::test]
#[serial]
async fn unknown_or_mutated_application_model_contract_fails_closed() {
    let database = ApplicationModelOperationDb::start("fail-closed").await;
    let unknown_operation_id = database.create_task_root("unknown contract").await;

    let unknown = sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,
               attack_execution_contract,application_model_contract
           ) VALUES($1,'red_team','scoping','v2_only','v2_only','latest_if_available')"#,
    )
    .bind(unknown_operation_id)
    .execute(database.db.pool())
    .await;
    assert!(unknown.is_err(), "unknown contracts must fail closed");

    let frozen_operation_id = database.create_task_root("immutable contract").await;
    operation_state::insert(
        database.db.pool(),
        frozen_operation_id,
        "red_team",
        "scoping",
        "v2_only",
        ApplicationModelContract::ApplicationModelV1,
    )
    .await
    .expect("insert frozen operation");
    let update = sqlx::query(
        "UPDATE operation_state SET application_model_contract='legacy_no_model' \
         WHERE operation_id=$1",
    )
    .bind(frozen_operation_id)
    .execute(database.db.pool())
    .await;
    assert!(update.is_err(), "a frozen contract must reject mutation");
    assert_eq!(
        operation_state::get_application_model_contract(database.db.pool(), frozen_operation_id)
            .await
            .expect("restore contract after rejected update"),
        Some(ApplicationModelContract::ApplicationModelV1)
    );
}
