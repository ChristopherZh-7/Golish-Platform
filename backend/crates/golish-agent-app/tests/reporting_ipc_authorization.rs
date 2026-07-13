use async_trait::async_trait;
use golish_agent_app::ai::commands::reporting::authorize_reporting_scope;
use golish_app_core::domain::operator::{
    OperatorChannel, TrustedOperatorPrincipal, TrustedOperatorPrincipalProvider,
};
use golish_app_core::GolishError;
use golish_db::models::NewSession;
use golish_db::repo::{project_scopes, runtime_memory_tx, sessions};
use golish_db::{DbConfig, GolishDb};
use serial_test::serial;
use uuid::Uuid;

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
        database: format!("reporting_ipc_auth_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    };
    let db = GolishDb::start(config)
        .await
        .expect("start migrated embedded postgres");
    (db, data_dir)
}

#[derive(Clone, Copy)]
enum PrincipalMode {
    TrustedDesktop,
    WrongChannel,
    ProviderFailure,
}

struct StubPrincipalProvider {
    mode: PrincipalMode,
    principal_id: Uuid,
}

#[async_trait]
impl TrustedOperatorPrincipalProvider for StubPrincipalProvider {
    async fn current(
        &self,
        channel: OperatorChannel,
    ) -> Result<TrustedOperatorPrincipal, GolishError> {
        if channel != OperatorChannel::LocalDesktop {
            return Err(GolishError::Internal(
                "reporting requested a non-desktop principal".to_string(),
            ));
        }
        match self.mode {
            PrincipalMode::TrustedDesktop => Ok(TrustedOperatorPrincipal::from_server_record(
                self.principal_id,
                OperatorChannel::LocalDesktop,
            )),
            PrincipalMode::WrongChannel => Ok(TrustedOperatorPrincipal::from_server_record(
                self.principal_id,
                OperatorChannel::LocalCli,
            )),
            PrincipalMode::ProviderFailure => {
                Err(GolishError::Internal("principal unavailable".to_string()))
            }
        }
    }
}

async fn reporting_scope(db: &GolishDb, label: &str, sealed: bool) -> (Uuid, Uuid) {
    let project_path = format!("/fixture/reporting-ipc-auth/{label}");
    let session = sessions::create(
        db.pool(),
        NewSession {
            title: Some(format!("reporting IPC auth {label}")),
            workspace_path: Some(project_path.clone()),
            workspace_label: None,
            model: None,
            provider: None,
            project_path: Some(project_path.clone()),
        },
    )
    .await
    .expect("create reporting auth session");
    let project = project_scopes::register_first_open(db.pool(), &project_path, &"1".repeat(64))
        .await
        .expect("register reporting project scope");
    let operation_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    runtime_memory_tx::create_runtime_operation(
        db.pool(),
        &runtime_memory_tx::CreateRuntimeOperationRow {
            operation_id,
            initial_stage_execution_id: stage_execution_id,
            session_id: session.id,
            title: Some(format!("reporting IPC auth operation {label}")),
            input: label.to_string(),
            profile: "assessment".to_string(),
            entry_stage: "target_intel".to_string(),
            project_scope_id: project.project_scope_id,
            cli_scope: None,
        },
    )
    .await
    .expect("create reporting operation");

    let decision_id = Uuid::new_v4();
    let snapshot_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let mut tx = db.pool().begin().await.expect("begin reporting auth scope");
    sqlx::query(
        r#"INSERT INTO operation_scope_decisions(
               id,operation_id,project_scope_id,stage_execution_id,
               root_organization_id,mode,decision_rows,decision_hash
           ) VALUES($1,$2,$3,$4,$5,'cli_flags',$6,$7)"#,
    )
    .bind(decision_id)
    .bind(operation_id)
    .bind(project.project_scope_id)
    .bind(stage_execution_id)
    .bind(organization_id)
    .bind(serde_json::json!([{"organization_id": organization_id}]))
    .bind("2".repeat(64))
    .execute(&mut *tx)
    .await
    .expect("insert scope decision");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_snapshots(
               id,operation_id,project_scope_id,scope_decision_id,
               project_path_at_freeze,root_organization_id,mode,scope_hash
           ) VALUES($1,$2,$3,$4,$5,$6,'cli_flags',$7)"#,
    )
    .bind(snapshot_id)
    .bind(operation_id)
    .bind(project.project_scope_id)
    .bind(decision_id)
    .bind(&project_path)
    .bind(organization_id)
    .bind("3".repeat(64))
    .execute(&mut *tx)
    .await
    .expect("insert scope snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units(
               snapshot_id,organization_id,organization_name_at_freeze,
               role,depth,ordinal,decision_row_id,approval_source
           ) VALUES($1,$2,$3,'root',0,0,'root',$4)"#,
    )
    .bind(snapshot_id)
    .bind(organization_id)
    .bind(format!("Reporting Org {label}"))
    .bind(serde_json::json!({"source": "cli_flags"}))
    .execute(&mut *tx)
    .await
    .expect("insert frozen scope unit");
    if sealed {
        sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
            .bind(snapshot_id)
            .execute(&mut *tx)
            .await
            .expect("seal frozen scope");
    }
    tx.commit().await.expect("commit reporting auth scope");
    (operation_id, project.project_scope_id)
}

fn assert_forbidden(error: &golish_agent_app::ai::commands::reporting::ReportingCommandError) {
    assert_eq!(error.code, "REPORT_FORBIDDEN");
    assert_eq!(error.message, "reporting scope is not authorized");
}

#[tokio::test]
#[serial]
async fn reporting_authorizer_accepts_only_trusted_active_exact_sealed_scope() {
    let (mut db, _data_dir) = fixture().await;
    let (active_operation, _) = reporting_scope(&db, "active", true).await;
    let (unsealed_operation, _) = reporting_scope(&db, "unsealed", false).await;
    let (retired_operation, retired_project) = reporting_scope(&db, "retired", true).await;
    sqlx::query("UPDATE project_scopes SET retired_at=NOW() WHERE project_scope_id=$1")
        .bind(retired_project)
        .execute(db.pool())
        .await
        .expect("retire project scope");

    let trusted = StubPrincipalProvider {
        mode: PrincipalMode::TrustedDesktop,
        principal_id: Uuid::new_v4(),
    };
    authorize_reporting_scope(db.pool(), &trusted, active_operation)
        .await
        .expect("trusted exact frozen scope is authorized");

    for operation_id in [Uuid::new_v4(), unsealed_operation, retired_operation] {
        let error = authorize_reporting_scope(db.pool(), &trusted, operation_id)
            .await
            .expect_err("foreign, unsealed, and retired scopes fail closed");
        assert_forbidden(&error);
    }

    for mode in [PrincipalMode::WrongChannel, PrincipalMode::ProviderFailure] {
        let untrusted = StubPrincipalProvider {
            mode,
            principal_id: Uuid::new_v4(),
        };
        let error = authorize_reporting_scope(db.pool(), &untrusted, active_operation)
            .await
            .expect_err("untrusted principals fail closed");
        assert_forbidden(&error);
    }

    db.stop().await;
}

#[test]
fn all_reporting_commands_authorize_before_any_report_read_or_exists_branch() {
    let source = include_str!("../src/ai/commands/reporting.rs");
    let commands = [
        "reporting_get_read_model",
        "reporting_list_revisions",
        "reporting_get_artifacts",
        "reporting_build_read_model",
        "reporting_finalize_revision",
    ];
    for (index, command) in commands.iter().enumerate() {
        let declaration = format!("pub async fn {command}");
        let start = source
            .find(&declaration)
            .unwrap_or_else(|| panic!("missing production command {command}"));
        let tail = &source[start..];
        let end = commands
            .iter()
            .filter_map(|other| {
                let other_declaration = format!("pub async fn {other}");
                tail.find(&other_declaration)
                    .filter(|position| *position > 0)
            })
            .min()
            .unwrap_or(tail.len());
        let body = &tail[..end];
        let authorization = body
            .find("authorize_reporting_scope")
            .unwrap_or_else(|| panic!("{command} does not reuse reporting authorization"));
        for read in [
            "load_bundle(",
            "build_or_reuse_validated_report(",
            "get_active_for_share(",
        ] {
            if let Some(read_position) = body.find(read) {
                assert!(
                    authorization < read_position,
                    "{command} performs {read} before authorization"
                );
            }
        }
        if index == commands.len() - 1 {
            let confirmation = body
                .find("if !request.confirm_final_publish")
                .expect("finalize keeps explicit confirmation");
            assert!(
                authorization < confirmation,
                "finalize must reject untrusted callers before request-specific branches"
            );
        }
    }
}
