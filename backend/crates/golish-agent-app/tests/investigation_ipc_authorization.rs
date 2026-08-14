use async_trait::async_trait;
use golish_agent_app::ai::commands::investigation::{
    authorize_investigation_read_scope, authorize_investigation_scope,
    InvestigationActorTopologyNodeView, InvestigationAuthorityTimeViewV1,
    InvestigationCampaignDetailRequest, InvestigationCampaignDetailResponse,
    InvestigationCampaignDetailView, InvestigationCampaignListItemView,
    InvestigationCampaignListRequest, InvestigationCampaignPageResponse, InvestigationCommandError,
    InvestigationControlProjectionV1, InvestigationCoverageDenominatorView,
    InvestigationGenerationSummaryView, InvestigationHypothesisDetailView,
    InvestigationHypothesisGetRequest, InvestigationHypothesisListItemView,
    InvestigationHypothesisListRequest, InvestigationHypothesisListView,
    InvestigationModePolicyView, InvestigationOpenObligationSummaryView,
    InvestigationProjectionEnvelope, InvestigationRequestStopRequest,
    InvestigationRequestStopResponse, InvestigationScopeRequest,
    InvestigationSourceCensusMemberView, InvestigationSummaryView,
    InvestigationTemporalSnapshotView, InvestigationTimelineItemView,
    InvestigationTimelineListRequest, InvestigationTimelinePageResponse,
    InvestigationWaveSummaryView,
};
use golish_agent_app::AiState;
use golish_app_core::domain::operator::{
    OperatorChannel, TrustedOperatorPrincipal, TrustedOperatorPrincipalProvider,
};
use golish_app_core::GolishError;
use golish_core::investigation_projection::{
    ProjectionEntityKind, ProjectionInvalidationReason, ProjectionSourceTimeStatusV1,
    TimelineEventKind,
};
use golish_core::runtime::{ApprovalResult, GolishRuntime, RuntimeError, RuntimeEvent};
use golish_db::models::{NewSession, TaskStatus};
use golish_db::repo::{project_scopes, runtime_memory_tx, sessions, tasks};
use golish_db::{DbConfig, GolishDb};
use serial_test::serial;
use std::{any::Any, path::Path, sync::Arc};
use ts_rs::TS;
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
        database: format!("investigation_ipc_auth_{}", Uuid::new_v4().simple()),
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

#[derive(Debug)]
struct MockRuntime;

#[async_trait]
impl GolishRuntime for MockRuntime {
    fn emit(&self, _event: RuntimeEvent) -> Result<(), RuntimeError> {
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

async fn install_live_bridge(ai_state: &AiState, session_id: &str, workspace_path: &Path) {
    let bridge = golish_agent_bridge::AgentBridge::new_openrouter_with_runtime(
        workspace_path.to_path_buf(),
        "investigation-auth-test",
        "test-key",
        None,
        Arc::new(MockRuntime),
    )
    .await
    .expect("construct investigation auth bridge");
    ai_state
        .install_session_bridge(session_id.to_owned(), bridge)
        .await
        .expect("install investigation auth bridge");
}

#[async_trait]
impl TrustedOperatorPrincipalProvider for StubPrincipalProvider {
    async fn current(
        &self,
        channel: OperatorChannel,
    ) -> Result<TrustedOperatorPrincipal, GolishError> {
        if channel != OperatorChannel::LocalDesktop {
            return Err(GolishError::Internal(
                "investigation requested a non-desktop principal".to_owned(),
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
                Err(GolishError::Internal("principal unavailable".to_owned()))
            }
        }
    }
}

async fn investigation_scope(
    db: &GolishDb,
    label: &str,
    workspace_path: &Path,
    sealed: bool,
) -> (Uuid, Uuid, Uuid, String) {
    let (project_path, path_sha256) =
        golish_agent_kit::runtime_memory::canonical_workspace_identity(workspace_path)
            .expect("canonical investigation workspace identity");
    let session_id = format!("investigation-auth-{label}");
    let session = sessions::upsert_by_chat_key(
        db.pool(),
        &session_id,
        NewSession {
            title: Some(format!("investigation IPC auth {label}")),
            workspace_path: Some(project_path.clone()),
            workspace_label: None,
            model: None,
            provider: None,
            project_path: Some(project_path.clone()),
        },
    )
    .await
    .expect("create investigation auth session");
    let project = project_scopes::register_first_open(db.pool(), &project_path, &path_sha256)
        .await
        .expect("register investigation project scope");
    let operation_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    runtime_memory_tx::create_runtime_operation(
        db.pool(),
        &runtime_memory_tx::CreateRuntimeOperationRow {
            operation_id,
            initial_stage_execution_id: stage_execution_id,
            session_id: session.id,
            title: Some(format!("investigation IPC auth operation {label}")),
            input: label.to_owned(),
            profile: "assessment".to_owned(),
            entry_stage: "target_intel".to_owned(),
            project_scope_id: project.project_scope_id,
            cli_scope: None,
            application_model_contract: golish_core::ApplicationModelContract::LegacyNoModel,
        },
    )
    .await
    .expect("create investigation operation");

    let decision_id = Uuid::new_v4();
    let snapshot_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin investigation auth scope");
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
    .bind(format!("Investigation Org {label}"))
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
    tx.commit().await.expect("commit investigation auth scope");
    (
        operation_id,
        project.project_scope_id,
        organization_id,
        session_id,
    )
}

fn assert_forbidden(error: &InvestigationCommandError) {
    assert_eq!(error.code, "INVESTIGATION_FORBIDDEN");
    assert_eq!(error.message, "investigation scope is not authorized");
    assert_eq!(error.current_change_seq, None);
    assert!(!error.restart_required);
}

#[tokio::test]
#[serial]
async fn investigation_auth_accepts_only_trusted_active_exact_sealed_scope() {
    let (mut db, _data_dir) = fixture().await;
    let active_workspace = tempfile::tempdir().expect("active workspace");
    let foreign_workspace = tempfile::tempdir().expect("foreign workspace");
    let retired_workspace = tempfile::tempdir().expect("retired workspace");
    let (active_operation, _, active_org, active_session) =
        investigation_scope(&db, "active", active_workspace.path(), true).await;
    let (foreign_operation, _, _, foreign_session) =
        investigation_scope(&db, "foreign-active", foreign_workspace.path(), true).await;
    let (unsealed_operation, _, _, unsealed_session) =
        investigation_scope(&db, "unsealed", active_workspace.path(), false).await;
    let (no_bridge_operation, _, _, no_bridge_session) =
        investigation_scope(&db, "no-bridge", active_workspace.path(), true).await;
    let (finished_operation, _, finished_org, finished_session) =
        investigation_scope(&db, "finished-no-bridge", active_workspace.path(), true).await;
    tasks::update_status(db.pool(), finished_operation, TaskStatus::Finished)
        .await
        .expect("finish historical Investigation task");
    let (retired_operation, retired_project, _, retired_session) =
        investigation_scope(&db, "retired", retired_workspace.path(), true).await;
    sqlx::query("UPDATE project_scopes SET retired_at=NOW() WHERE project_scope_id=$1")
        .bind(retired_project)
        .execute(db.pool())
        .await
        .expect("retire project scope");

    let ai_state = AiState::new();
    install_live_bridge(&ai_state, &active_session, active_workspace.path()).await;
    install_live_bridge(&ai_state, &foreign_session, foreign_workspace.path()).await;
    install_live_bridge(&ai_state, &unsealed_session, active_workspace.path()).await;
    install_live_bridge(&ai_state, &retired_session, retired_workspace.path()).await;

    let principal_id = golish_db::repo::operator_principals::current_local(db.pool())
        .await
        .expect("load active local principal")
        .id;
    let trusted = StubPrincipalProvider {
        mode: PrincipalMode::TrustedDesktop,
        principal_id,
    };
    let authority = authorize_investigation_scope(
        db.pool(),
        &trusted,
        &ai_state,
        &active_session,
        active_operation,
    )
    .await
    .expect("trusted exact frozen scope is authorized");
    authority
        .authorize_organization_selectors(&[active_org])
        .expect("at-time scope organization is authorized");
    assert_forbidden(
        &authority
            .authorize_organization_selectors(&[Uuid::new_v4()])
            .expect_err("cross-organization selector fails closed"),
    );

    let historical = authorize_investigation_read_scope(
        db.pool(),
        &trusted,
        &ai_state,
        &finished_session,
        finished_operation,
    )
    .await
    .expect("finished historical projection is readable without a live bridge");
    historical
        .authorize_organization_selectors(&[finished_org])
        .expect("historical read remains bound to the sealed organization set");
    assert_forbidden(
        &authorize_investigation_scope(
            db.pool(),
            &trusted,
            &ai_state,
            &finished_session,
            finished_operation,
        )
        .await
        .expect_err("control authority never falls back without a live bridge"),
    );
    assert_forbidden(
        &authorize_investigation_read_scope(
            db.pool(),
            &trusted,
            &ai_state,
            &no_bridge_session,
            no_bridge_operation,
        )
        .await
        .expect_err("nonterminal historical reads still require a live bridge"),
    );

    for (session_id, operation_id) in [
        (active_session.as_str(), Uuid::new_v4()),
        (active_session.as_str(), foreign_operation),
        (foreign_session.as_str(), active_operation),
        (no_bridge_session.as_str(), no_bridge_operation),
        (unsealed_session.as_str(), unsealed_operation),
        (retired_session.as_str(), retired_operation),
    ] {
        let error =
            authorize_investigation_scope(db.pool(), &trusted, &ai_state, session_id, operation_id)
                .await
                .expect_err("foreign, missing-bridge, unsealed, and retired scopes fail closed");
        assert_forbidden(&error);
    }
    for mode in [PrincipalMode::WrongChannel, PrincipalMode::ProviderFailure] {
        let provider = StubPrincipalProvider { mode, principal_id };
        let error = authorize_investigation_scope(
            db.pool(),
            &provider,
            &ai_state,
            &active_session,
            active_operation,
        )
        .await
        .expect_err("untrusted principal fails closed");
        assert_forbidden(&error);
    }
    let forged_principal = StubPrincipalProvider {
        mode: PrincipalMode::TrustedDesktop,
        principal_id: Uuid::new_v4(),
    };
    let error = authorize_investigation_scope(
        db.pool(),
        &forged_principal,
        &ai_state,
        &active_session,
        active_operation,
    )
    .await
    .expect_err("provider principal must equal the DB active local principal");
    assert_forbidden(&error);

    ai_state.remove_session_bridge(&active_session).await;
    let error = authorize_investigation_scope(
        db.pool(),
        &trusted,
        &ai_state,
        &active_session,
        active_operation,
    )
    .await
    .expect_err("retired live bridge fails closed");
    assert_forbidden(&error);
    db.stop().await;
}

#[test]
fn investigation_auth_precedes_selector_and_projection_reads_in_all_commands() {
    let source = include_str!("../src/ai/commands/investigation/mod.rs");
    let commands = [
        "investigation_get_summary",
        "investigation_list_hypotheses",
        "investigation_get_hypothesis",
    ];
    for command in commands {
        let start = source
            .find(&format!("pub async fn {command}"))
            .unwrap_or_else(|| panic!("missing command {command}"));
        let body = &source[start..];
        let auth = body
            .find("authorize_investigation_read_scope")
            .expect("command authorizes operation");
        for sensitive in [
            "read_investigation_summary(",
            "list_investigation_hypotheses(",
            "get_investigation_hypothesis(",
            "parse_uuid(&request.revision_id)",
            "authorize_organization_selectors",
        ] {
            if let Some(position) = body.find(sensitive) {
                assert!(auth < position, "{command} reaches {sensitive} before auth");
            }
        }
    }

    let authorizer = source
        .split("async fn authorize_investigation_scope_with_access")
        .nth(1)
        .expect("investigation authorizer");
    let ordered_authority_steps = [
        "principal_provider",
        "operation_state::get",
        "tasks::get",
        "sessions::get",
        "get_active_for_share",
        "get_session_bridge",
        "canonical_workspace_identity",
        "load_for_operation",
    ];
    let mut previous = 0;
    for step in ordered_authority_steps {
        let position = authorizer
            .find(step)
            .unwrap_or_else(|| panic!("missing investigation authority step {step}"));
        assert!(
            position >= previous,
            "investigation authority step {step} is out of order"
        );
        previous = position;
    }
}

#[test]
fn investigation_cursor_v2_command_boundary_verifies_before_materialized_query() {
    let source = include_str!("../src/ai/commands/investigation/mod.rs");
    let start = source
        .find("pub async fn investigation_list_hypotheses")
        .expect("list command");
    let body = &source[start..];
    let verify = body
        .find("continue_current_cursor(")
        .expect("HMAC verifier");
    let query = body
        .find("list_investigation_hypotheses(")
        .expect("materialized query");
    assert!(verify < query);
    assert!(body.contains("expected_page_authority"));

    let cursor = include_str!("../src/ai/commands/investigation/cursor.rs");
    assert!(cursor.contains("verify_slice(&signature)"));
    assert!(cursor.contains("URL_SAFE_NO_PAD"));
}

#[test]
fn investigation_cursor_v1_legacy_current_continuation_requires_restart() {
    let cursor = include_str!("../src/ai/commands/investigation/cursor.rs");
    assert!(cursor.contains("VerifiedInvestigationCursor::Historical(_)"));
    assert!(cursor.contains("InvestigationCursorFailure::Stale"));
    assert!(!cursor.contains("issue_v1"));
}

#[test]
fn investigation_temporal_snapshot_and_dtos_are_closed_and_redacted() {
    let config = ts_rs::Config::default();
    let temporal = InvestigationTemporalSnapshotView::decl(&config);
    for field in [
        "contractVersion",
        "asOfTemporalCutoff",
        "authorityEpochSetHash",
        "earliestEffectiveValidUntil",
    ] {
        assert!(temporal.contains(field), "temporal DTO missing {field}");
    }
    let envelope = InvestigationProjectionEnvelope::decl(&config);
    assert!(envelope.contains("temporalSnapshot"));

    let declarations = [
        InvestigationHypothesisListRequest::decl(&config),
        InvestigationScopeRequest::decl(&config),
        InvestigationHypothesisGetRequest::decl(&config),
        InvestigationRequestStopRequest::decl(&config),
        InvestigationControlProjectionV1::decl(&config),
        InvestigationRequestStopResponse::decl(&config),
        InvestigationTemporalSnapshotView::decl(&config),
        InvestigationProjectionEnvelope::decl(&config),
        InvestigationModePolicyView::decl(&config),
        InvestigationCommandError::decl(&config),
        InvestigationSummaryView::decl(&config),
        InvestigationSourceCensusMemberView::decl(&config),
        InvestigationActorTopologyNodeView::decl(&config),
        InvestigationHypothesisListItemView::decl(&config),
        InvestigationHypothesisListView::decl(&config),
        InvestigationHypothesisDetailView::decl(&config),
    ]
    .join("\n");
    for request in [
        InvestigationHypothesisListRequest::decl(&config),
        InvestigationScopeRequest::decl(&config),
        InvestigationHypothesisGetRequest::decl(&config),
        InvestigationRequestStopRequest::decl(&config),
    ] {
        assert!(request.contains("sessionId"));
        assert!(request.contains("operationId"));
    }
    for forbidden in [
        "rawPayload",
        "credential",
        "prompt",
        "proseArtifact",
        "leaseToken",
        "checkpoint",
        "cursorSalt",
        "workspacePath",
        "projectScopeId",
        "principalId",
    ] {
        assert!(
            !declarations.contains(forbidden),
            "leaked field {forbidden}"
        );
    }
}

#[test]
fn investigation_read_views_expose_bounded_census_and_host_verified_actor_identity() {
    let config = ts_rs::Config::default();
    let summary = InvestigationSummaryView::decl(&config);
    for field in [
        "sourceCensus",
        "mainActor",
        "actorTopology",
        "generationCount",
        "waveCount",
        "campaignCount",
        "openObligationCount",
    ] {
        assert!(summary.contains(field), "summary DTO missing {field}");
    }

    let actor = InvestigationActorTopologyNodeView::decl(&config);
    for field in [
        "organizationId",
        "hypothesisRevisionId",
        "taskId",
        "subtaskId",
        "workerRunId",
        "owningStageRunRequestId",
        "transcriptRequestId",
        "parentActorTranscriptRequestId",
        "parentDispatchToolRequestId",
        "status",
    ] {
        assert!(actor.contains(field), "actor DTO missing {field}");
    }
    assert!(InvestigationHypothesisDetailView::decl(&config).contains("actorTopology"));

    let source = include_str!("../../golish-db/src/repo/investigation_projection/summary.rs");
    assert!(source.contains("load_exact_source_census_on"));
    assert!(source.contains("load_exact_main_actor_on"));
    assert!(source.contains("load_exact_actor_topology_on"));
    for forbidden in ["raw_corpus_body", "credential_body", "prepared_action_args"] {
        assert!(
            !source.contains(forbidden),
            "raw authority leak: {forbidden}"
        );
    }
}

#[test]
fn six_read_requests_bind_one_exact_stage_run_and_explicit_snapshot_authority() {
    let config = ts_rs::Config::default();
    let bootstrap_reads = [
        InvestigationScopeRequest::decl(&config),
        InvestigationHypothesisListRequest::decl(&config),
        InvestigationCampaignListRequest::decl(&config),
        InvestigationTimelineListRequest::decl(&config),
    ];
    let detail_reads = [
        InvestigationHypothesisGetRequest::decl(&config),
        InvestigationCampaignDetailRequest::decl(&config),
    ];

    for request in bootstrap_reads.iter().chain(detail_reads.iter()) {
        for selector in ["operationId", "stageExecutionId", "stageRunRequestId"] {
            assert!(
                request.contains(selector),
                "read request is missing exact stage-run selector {selector}: {request}"
            );
        }
        for authority in [
            "expectedChangeSeq",
            "expectedTemporalCutoff",
            "expectedAuthorityEpochSetHash",
            "expectedEarliestEffectiveValidUntil",
        ] {
            assert!(
                request.contains(authority),
                "read request is missing snapshot authority {authority}: {request}"
            );
        }
    }

    for request in bootstrap_reads {
        assert!(
            request.contains("expectedChangeSeq: number | null"),
            "summary and first-page bootstrap must use a typed no-seq state: {request}"
        );
    }
    for request in detail_reads {
        assert!(
            request.contains("expectedChangeSeq: number"),
            "detail reads must require an exact non-null change sequence: {request}"
        );
        assert!(!request.contains("expectedChangeSeq: number | null"));
    }
}

#[test]
fn six_read_commands_validate_exact_run_before_materialized_projection_queries() {
    let source = include_str!("../src/ai/commands/investigation/mod.rs");
    let commands = [
        (
            "investigation_get_summary",
            "read_investigation_summary_for_stage_run(",
        ),
        (
            "investigation_list_hypotheses",
            "list_investigation_hypotheses_for_stage_run(",
        ),
        (
            "investigation_get_hypothesis",
            "get_investigation_hypothesis_for_stage_run(",
        ),
        (
            "investigation_list_campaigns",
            "list_investigation_campaigns_for_stage_run(",
        ),
        (
            "investigation_get_campaign",
            "get_investigation_campaign_for_stage_run(",
        ),
        (
            "investigation_list_timeline",
            "read_investigation_timeline_for_stage_run(",
        ),
    ];
    for (command, exact_query) in commands {
        let start = source
            .find(&format!("pub async fn {command}"))
            .unwrap_or_else(|| panic!("missing command {command}"));
        let body = &source[start..];
        let exact_head = body
            .find("exact_read_selector(")
            .unwrap_or_else(|| panic!("{command} does not validate an exact stage run"));
        let query = body
            .find(exact_query)
            .unwrap_or_else(|| panic!("{command} does not use exact-run projection query"));
        assert!(
            exact_head < query,
            "{command} queries before exact selector validation"
        );
    }

    assert!(!source.contains("latest_investigation"));
    assert!(!source.contains("ORDER BY stage_execution_id DESC LIMIT 1"));
}

#[test]
fn export_bindings() {
    let config = ts_rs::Config::default();
    ProjectionEntityKind::export(&config).expect("export ProjectionEntityKind");
    ProjectionInvalidationReason::export(&config).expect("export ProjectionInvalidationReason");
    TimelineEventKind::export(&config).expect("export TimelineEventKind");
    ProjectionSourceTimeStatusV1::export(&config).expect("export ProjectionSourceTimeStatusV1");
    InvestigationHypothesisListRequest::export(&config).expect("export list request");
    InvestigationScopeRequest::export(&config).expect("export scope request");
    InvestigationHypothesisGetRequest::export(&config).expect("export get request");
    InvestigationRequestStopRequest::export(&config).expect("export stop request");
    InvestigationControlProjectionV1::export(&config).expect("export control projection");
    InvestigationRequestStopResponse::export(&config).expect("export stop response");
    InvestigationTemporalSnapshotView::export(&config).expect("export temporal snapshot");
    InvestigationProjectionEnvelope::export(&config).expect("export projection envelope");
    InvestigationModePolicyView::export(&config).expect("export mode policy");
    InvestigationCommandError::export(&config).expect("export command error");
    InvestigationSummaryView::export(&config).expect("export summary view");
    InvestigationSourceCensusMemberView::export(&config).expect("export source census member");
    InvestigationActorTopologyNodeView::export(&config).expect("export actor topology node");
    InvestigationGenerationSummaryView::export(&config).expect("export generation summary");
    InvestigationWaveSummaryView::export(&config).expect("export wave summary");
    InvestigationOpenObligationSummaryView::export(&config).expect("export open obligation");
    InvestigationCoverageDenominatorView::export(&config).expect("export coverage denominator");
    InvestigationAuthorityTimeViewV1::export(&config).expect("export authority time");
    InvestigationCampaignListRequest::export(&config).expect("export campaign list request");
    InvestigationCampaignListItemView::export(&config).expect("export campaign list item");
    InvestigationCampaignPageResponse::export(&config).expect("export campaign page");
    InvestigationCampaignDetailRequest::export(&config).expect("export campaign detail request");
    InvestigationCampaignDetailView::export(&config).expect("export campaign detail");
    InvestigationCampaignDetailResponse::export(&config).expect("export campaign response");
    InvestigationTimelineListRequest::export(&config).expect("export timeline request");
    InvestigationTimelineItemView::export(&config).expect("export timeline item");
    InvestigationTimelinePageResponse::export(&config).expect("export timeline page");
    InvestigationHypothesisListItemView::export(&config).expect("export list item view");
    InvestigationHypothesisListView::export(&config).expect("export list view");
    InvestigationHypothesisDetailView::export(&config).expect("export detail view");
}
