use std::{
    any::Any,
    path::Path,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use golish_agent_app::{
    ai::commands::investigation::{
        request_investigation_stop_authorized, InvestigationRequestStopRequest,
    },
    AiState,
};
use golish_app_core::{
    domain::operator::{
        OperatorChannel, TrustedOperatorPrincipal, TrustedOperatorPrincipalProvider,
    },
    GolishError,
};
use golish_core::{
    events::AiEvent,
    runtime::{ApprovalResult, GolishRuntime, RuntimeError, RuntimeEvent},
};
use golish_db::{
    models::NewSession,
    repo::{
        project_scopes, runtime_memory_tx, sessions,
        unified_investigation_runtime::{
            InvestigationStageIdentity, PgUnifiedInvestigationRuntimeRepository,
            StartInvestigationRunInput,
        },
    },
    DbConfig, GolishDb,
};
use serial_test::serial;
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

#[derive(Default)]
struct CaptureRuntime {
    events: Mutex<Vec<RuntimeEvent>>,
}

impl CaptureRuntime {
    fn projection_hints(&self) -> Vec<(String, String, String, i64)> {
        self.events
            .lock()
            .expect("capture runtime mutex")
            .iter()
            .filter_map(|event| {
                let ai_event = match event {
                    RuntimeEvent::Ai { event, .. } => Some(event.as_ref()),
                    RuntimeEvent::AiEnvelope { envelope, .. } => Some(&envelope.event),
                    _ => None,
                }?;
                match ai_event {
                    AiEvent::InvestigationProjectionChanged {
                        operation_id,
                        stage_execution_id,
                        stage_run_request_id,
                        change_seq,
                    } => Some((
                        operation_id.clone(),
                        stage_execution_id.clone(),
                        stage_run_request_id.clone(),
                        *change_seq,
                    )),
                    _ => None,
                }
            })
            .collect()
    }
}

#[async_trait]
impl GolishRuntime for CaptureRuntime {
    fn emit(&self, event: RuntimeEvent) -> Result<(), RuntimeError> {
        self.events
            .lock()
            .map_err(|_| RuntimeError::EmitFailed("capture runtime mutex poisoned".to_owned()))?
            .push(event);
        Ok(())
    }

    async fn request_approval(
        &self,
        _request_id: String,
        _tool_name: String,
        _args: serde_json::Value,
        _risk_level: String,
    ) -> Result<ApprovalResult, RuntimeError> {
        Err(RuntimeError::ApprovalTimeout(0))
    }

    fn is_interactive(&self) -> bool {
        false
    }

    fn auto_approve(&self) -> bool {
        false
    }

    async fn shutdown(&self) -> Result<(), RuntimeError> {
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct TrustedDesktopPrincipal {
    id: Uuid,
}

#[async_trait]
impl TrustedOperatorPrincipalProvider for TrustedDesktopPrincipal {
    async fn current(
        &self,
        channel: OperatorChannel,
    ) -> Result<TrustedOperatorPrincipal, GolishError> {
        if channel != OperatorChannel::LocalDesktop {
            return Err(GolishError::Internal(
                "unexpected operator channel".to_owned(),
            ));
        }
        Ok(TrustedOperatorPrincipal::from_server_record(
            self.id,
            OperatorChannel::LocalDesktop,
        ))
    }
}

struct StopFixture {
    operation_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_request_id: String,
    session_id: String,
    head_sha256: String,
    change_seq: i64,
}

async fn select_unified_rollout(db: &GolishDb) {
    let mut tx = db.pool().begin().await.expect("begin rollout selection");
    sqlx::query(
        "ALTER TABLE tool_truth_rollout DISABLE TRIGGER tool_truth_rollout_direct_mutation_guard",
    )
    .execute(&mut *tx)
    .await
    .expect("disable isolated Tool Truth rollout mutation guard");
    sqlx::query(
        r#"UPDATE tool_truth_rollout
              SET new_operation_contract='receipt_v1',row_version=row_version+1,
                  updated_at=statement_timestamp()
            WHERE singleton=TRUE"#,
    )
    .execute(&mut *tx)
    .await
    .expect("select receipt Tool Truth rollout");
    sqlx::query(
        "ALTER TABLE tool_truth_rollout ENABLE TRIGGER tool_truth_rollout_direct_mutation_guard",
    )
    .execute(&mut *tx)
    .await
    .expect("restore Tool Truth rollout mutation guard");
    sqlx::query(
        "ALTER TABLE investigation_rollout DISABLE TRIGGER investigation_rollout_direct_mutation_guard",
    )
    .execute(&mut *tx)
    .await
    .expect("disable isolated rollout mutation guard");
    sqlx::query(
        r#"UPDATE investigation_rollout
              SET contract_version='hypothesis_registry_v1',rollout_mode='new_only',
                  mode_rank=4,row_version=row_version+1 WHERE singleton=TRUE"#,
    )
    .execute(&mut *tx)
    .await
    .expect("select unified Investigation rollout");
    sqlx::query(
        "ALTER TABLE investigation_rollout ENABLE TRIGGER investigation_rollout_direct_mutation_guard",
    )
    .execute(&mut *tx)
    .await
    .expect("restore rollout mutation guard");
    tx.commit().await.expect("commit rollout selection");
}

async fn install_bridge(
    ai_state: &AiState,
    session_id: &str,
    workspace: &Path,
    runtime: Arc<CaptureRuntime>,
) {
    let bridge = golish_agent_bridge::AgentBridge::new_openrouter_with_runtime(
        workspace.to_path_buf(),
        "investigation-stop-test",
        "test-key",
        None,
        runtime,
    )
    .await
    .expect("construct stop-control bridge");
    ai_state
        .install_session_bridge(session_id.to_owned(), bridge)
        .await
        .expect("install stop-control bridge");
    ai_state
        .get_session_bridge(session_id)
        .await
        .expect("reload stop-control bridge")
        .mark_frontend_ready()
        .await;
}

async fn stop_fixture(db: &GolishDb, workspace: &Path) -> StopFixture {
    select_unified_rollout(db).await;
    let (canonical_path, path_sha256) =
        golish_agent_kit::runtime_memory::canonical_workspace_identity(workspace)
            .expect("canonical workspace identity");
    let session_id = format!("investigation-stop-{}", Uuid::new_v4().simple());
    let session = sessions::upsert_by_chat_key(
        db.pool(),
        &session_id,
        NewSession {
            title: Some("Investigation stop control".to_owned()),
            workspace_path: Some(canonical_path.clone()),
            workspace_label: None,
            model: None,
            provider: None,
            project_path: Some(canonical_path.clone()),
        },
    )
    .await
    .expect("create stop-control session");
    let project = project_scopes::register_first_open(db.pool(), &canonical_path, &path_sha256)
        .await
        .expect("register stop-control project");
    let operation_id = Uuid::new_v4();
    let initial_stage_execution_id = Uuid::new_v4();
    runtime_memory_tx::create_runtime_operation(
        db.pool(),
        &runtime_memory_tx::CreateRuntimeOperationRow {
            operation_id,
            initial_stage_execution_id,
            session_id: session.id,
            title: Some("Investigation stop operation".to_owned()),
            input: "stop control".to_owned(),
            profile: "red_team".to_owned(),
            entry_stage: "target_intel".to_owned(),
            project_scope_id: project.project_scope_id,
            cli_scope: None,
            application_model_contract: golish_core::ApplicationModelContract::LegacyNoModel,
        },
    )
    .await
    .expect("create stop-control operation");
    let organization_id = Uuid::new_v4();
    sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$2,'Stop Org')")
        .bind(organization_id)
        .bind(&canonical_path)
        .execute(db.pool())
        .await
        .expect("insert stop-control organization");
    let decision_id = Uuid::new_v4();
    let scope_snapshot_id = Uuid::new_v4();
    let mut scope_tx = db.pool().begin().await.expect("begin stop scope");
    sqlx::query(
        r#"INSERT INTO operation_scope_decisions(
               id,operation_id,project_scope_id,stage_execution_id,
               root_organization_id,mode,decision_rows,decision_hash
           ) VALUES($1,$2,$3,$4,$5,'cli_flags',$6,$7)"#,
    )
    .bind(decision_id)
    .bind(operation_id)
    .bind(project.project_scope_id)
    .bind(initial_stage_execution_id)
    .bind(organization_id)
    .bind(serde_json::json!([{"organization_id": organization_id}]))
    .bind(digest('2'))
    .execute(&mut *scope_tx)
    .await
    .expect("insert stop scope decision");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_snapshots(
               id,operation_id,project_scope_id,scope_decision_id,
               project_path_at_freeze,root_organization_id,mode,scope_hash
           ) VALUES($1,$2,$3,$4,$5,$6,'cli_flags',$7)"#,
    )
    .bind(scope_snapshot_id)
    .bind(operation_id)
    .bind(project.project_scope_id)
    .bind(decision_id)
    .bind(&canonical_path)
    .bind(organization_id)
    .bind(digest('3'))
    .execute(&mut *scope_tx)
    .await
    .expect("insert stop scope snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units(
               snapshot_id,organization_id,organization_name_at_freeze,
               role,depth,ordinal,decision_row_id,approval_source
           ) VALUES($1,$2,'Stop Org','root',0,0,'root',$3)"#,
    )
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(serde_json::json!({"source":"stop_control_fixture"}))
    .execute(&mut *scope_tx)
    .await
    .expect("insert stop scope member");
    sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
        .bind(scope_snapshot_id)
        .execute(&mut *scope_tx)
        .await
        .expect("seal stop scope");
    scope_tx.commit().await.expect("commit stop scope");

    for stage in [
        "external_attack_surface",
        "enumeration",
        "vuln_triage",
        "application_understanding",
        "investigation",
    ] {
        sqlx::query("UPDATE operation_state SET current_stage=$2 WHERE operation_id=$1")
            .bind(operation_id)
            .bind(stage)
            .execute(db.pool())
            .await
            .unwrap_or_else(|error| panic!("advance operation to {stage}: {error}"));
    }
    let stage_execution_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status) VALUES($1,$2,'investigation','started')",
    )
    .bind(stage_execution_id)
    .bind(operation_id)
    .execute(db.pool())
    .await
    .expect("insert Investigation stage execution");
    let authority_id = Uuid::new_v4();
    let stage_run_request_id = format!("investigation-stage-run-{}", Uuid::new_v4().simple());
    sqlx::query(
        r#"INSERT INTO investigation_stage_run_authorities(
               authority_id,operation_id,stage_execution_id,
               owning_stage_run_request_id,scope_snapshot_id
           ) VALUES($1,$2,$3,$4,$5)"#,
    )
    .bind(authority_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(&stage_run_request_id)
    .bind(scope_snapshot_id)
    .execute(db.pool())
    .await
    .expect("insert Investigation stage authority");
    let repository = PgUnifiedInvestigationRuntimeRepository::new(Arc::new(db.pool().clone()));
    let head = repository
        .start_run(&StartInvestigationRunInput {
            identity: InvestigationStageIdentity {
                authority_id,
                operation_id,
                stage_execution_id,
                owning_stage_run_request_id: stage_run_request_id.clone(),
                scope_snapshot_id,
            },
            stable_start_request_id: Uuid::new_v4(),
            initial_change_seq: 0,
        })
        .await
        .expect("start unified Investigation run");
    StopFixture {
        operation_id,
        stage_execution_id,
        stage_run_request_id,
        session_id,
        head_sha256: head.head_sha256,
        change_seq: head.change_seq,
    }
}

#[tokio::test]
#[serial]
async fn local_operator_stop_is_exact_idempotent_and_emits_one_committed_hint() {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let mut db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("investigation_stop_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start isolated migrated postgres");
    let workspace = tempfile::tempdir().expect("stop-control workspace");
    let fixture = stop_fixture(&db, workspace.path()).await;
    let ai_state = AiState::new();
    let runtime = Arc::new(CaptureRuntime::default());
    install_bridge(
        &ai_state,
        &fixture.session_id,
        workspace.path(),
        runtime.clone(),
    )
    .await;
    let principal_id = golish_db::repo::operator_principals::current_local(db.pool())
        .await
        .expect("load local principal")
        .id;
    let principal = TrustedDesktopPrincipal { id: principal_id };
    let idempotency_key = Uuid::new_v4();
    let request = InvestigationRequestStopRequest {
        session_id: fixture.session_id.clone(),
        operation_id: fixture.operation_id.to_string(),
        stage_execution_id: fixture.stage_execution_id.to_string(),
        stage_run_request_id: fixture.stage_run_request_id.clone(),
        expected_investigation_run_state_head: fixture.head_sha256.clone(),
        expected_change_seq: fixture.change_seq,
        idempotency_key: idempotency_key.to_string(),
    };
    let denied = request_investigation_stop_authorized(
        db.pool(),
        &TrustedDesktopPrincipal { id: Uuid::new_v4() },
        &ai_state,
        request.clone(),
    )
    .await
    .expect_err("an unrelated local principal cannot mutate the run");
    assert_eq!(denied.code, "INVESTIGATION_FORBIDDEN");
    assert!(runtime.projection_hints().is_empty());

    let first =
        request_investigation_stop_authorized(db.pool(), &principal, &ai_state, request.clone())
            .await
            .expect("exact local operator stop succeeds");
    assert_eq!(first.idempotency_key, idempotency_key.to_string());
    assert_eq!(first.stop_epoch, 1);
    assert_eq!(
        first.control_projection.investigation_run_state,
        "stop_pending"
    );
    assert!(!first.control_projection.stop_allowed);
    assert!(first.control_projection.change_seq > fixture.change_seq);

    let replay = request_investigation_stop_authorized(db.pool(), &principal, &ai_state, request)
        .await
        .expect("response-loss replay returns the same stop receipt");
    assert_eq!(replay.stop_intent_id, first.stop_intent_id);
    assert_eq!(replay.receipt_sha256, first.receipt_sha256);
    assert_eq!(replay.control_projection, first.control_projection);
    assert_eq!(
        runtime.projection_hints(),
        vec![(
            fixture.operation_id.to_string(),
            fixture.stage_execution_id.to_string(),
            fixture.stage_run_request_id,
            first.control_projection.change_seq,
        )]
    );

    let stale = request_investigation_stop_authorized(
        db.pool(),
        &principal,
        &ai_state,
        InvestigationRequestStopRequest {
            session_id: fixture.session_id,
            operation_id: fixture.operation_id.to_string(),
            stage_execution_id: fixture.stage_execution_id.to_string(),
            stage_run_request_id: first.control_projection.stage_run_request_id.clone(),
            expected_investigation_run_state_head: fixture.head_sha256,
            expected_change_seq: fixture.change_seq,
            idempotency_key: Uuid::new_v4().to_string(),
        },
    )
    .await
    .expect_err("a new request with a stale run head fails closed");
    assert_eq!(stale.code, "INVESTIGATION_PROJECTION_STALE");
    assert!(stale.restart_required);
    assert_eq!(runtime.projection_hints().len(), 1);
    db.stop().await;
}
