#[path = "support/reporting_artifact_store.rs"]
mod reporting_artifact_store;

use golish_agent_app::ai::application_understanding_runtime::{
    run_application_understanding_unit, ApplicationModelProducerInput,
    ApplicationModelProposalDraft, ApplicationModelProposalProducer,
    ApplicationUnderstandingRuntimeError, ApplicationUnderstandingRuntimeOutcome,
    RunApplicationUnderstandingUnit,
};
use golish_agent_app::ai::candidate_submit_tool::SubmitCandidateAttemptTool;
#[cfg(any())]
use golish_agent_app::ai::commands::reporting::{
    build_reporting_read_model_for_local_channel, finalize_reporting_revision_for_local_channel,
    ReportingFinalizeFence,
};
use golish_agent_app::ai::db_bridge::GolishDbRepoProvider;
use golish_agent_kit::db_traits::{
    CheckpointBoundWorkerChain, CheckpointCandidateTerminalBarrier, ClaimCandidateAttempt,
    CloseAttackV2VerificationUnit, ControlCandidateAttempt, RuntimeMemoryRepository,
    RuntimeWorkerFence, TerminalizeCandidateIntent,
};
use golish_agent_kit::harness::application_model_gate::{
    ApplicationModelGateCode, ApplicationModelGateDisposition,
};
use golish_agent_kit::harness::attack_execution::{
    AttemptEvidenceRole, CandidateBudget, PlannedCandidateAction, SideEffectClass,
    CANDIDATE_CLASSIFIER_VERSION_V2, CANDIDATE_EXECUTOR_CONTRACT_DIRECTORY_ENTRY_REPLAY_V2,
    CANDIDATE_RECIPE_VERSION_DIRECTORY_ENTRY_REPLAY_V2,
};
use golish_agent_kit::task_orchestrator::{
    AgentExecutor, AgentResult, ApplicationModelAgentAttempt, ApplicationModelAgentBinding,
    ApplicationModelAgentOutcome, ApplicationModelAgentRunner, ApplicationModelDecisionContract,
    ApplicationModelEvidenceContract, ApplicationModelEvidenceRoleContract,
    ApplicationModelInputDispositionContract, ApplicationModelItemContract,
    ApplicationModelPartialItemKindContract, ApplicationModelProducerFailure,
    ApplicationModelProducerInputContract, ApplicationModelProposalContract,
    ApplicationModelSynthesisInputContract, ApplicationModelTruthStateContract,
    ApplicationModelWorkItemInputContract, ApplicationModelWorkItemOutputContract,
    ApplicationModelWorkItemPartialContract, ApplicationUnderstandingStageOutcome,
    ApplicationUnderstandingStageRequest, ApplicationUnderstandingStageRuntime, ExecutionContext,
};
use golish_app_core::domain::operator::{
    OperatorChannel, TrustedOperatorPrincipal, TrustedOperatorPrincipalProvider,
};
use golish_app_core::GolishError;
use golish_core::{AgentToolContext, CandidateAttemptContextRef, Tool};
use golish_db::models::{AgentType, NewSession};
use golish_db::repo::application_models::{
    ApplicationModelEvidenceRoleRow, ApplicationModelInputDecisionSeed,
    ApplicationModelInputDispositionRow, ApplicationModelItemEvidenceSeed,
    ApplicationModelItemSeed, ApplicationModelTruthStateRow, DeriveApplicationModelManifestSeed,
    LoadApplicationModelGateMaterial,
};
use golish_db::repo::attack_candidate_approvals::{
    list_candidate_reviews, review_wave_candidates, CandidateReviewDecision, ReviewCandidateBatch,
};
use golish_db::repo::attack_candidate_work_items::{
    self, SeedAttackObservation, SeedAttackWorkItems,
};
use golish_db::repo::attack_candidates::{
    canonical_execution_plan_hash, AcceptedCandidateDraft, CandidateAcceptanceInput,
    NoCandidateDecision,
};
use golish_db::repo::candidate_attempts::{
    begin_candidate_action, claim_next_candidate_attempt, finish_candidate_action,
    BeginCandidateAction, CandidateActionStart, CandidateClaimQuery, FinishCandidateAction,
};
use golish_db::repo::{
    application_models, attack_wave_consolidations, attack_waves, canonical_fact_refs,
    project_scopes, runtime_memory_tx, sessions, stage_deliverable_submissions, stage_run_units,
    stage_teams, tool_calls,
};
use golish_db::{DbConfig, GolishDb};
use golish_pentest_app::pentest_bridge::VerifyExecuteCandidateActionTool;
use serde_json::json;
use serial_test::serial;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

use reporting_artifact_store::{report_blob_path, TestProjectReportArtifactStoreFactory};

fn assert_send_sync<T: Send + Sync>() {}

struct LocalCliPrincipalProvider {
    principal_id: Uuid,
}

#[async_trait::async_trait]
impl TrustedOperatorPrincipalProvider for LocalCliPrincipalProvider {
    async fn current(
        &self,
        channel: OperatorChannel,
    ) -> Result<TrustedOperatorPrincipal, GolishError> {
        Ok(TrustedOperatorPrincipal::from_server_record(
            self.principal_id,
            channel,
        ))
    }
}

#[test]
fn public_runtime_contract_is_available() {
    assert_send_sync::<RunApplicationUnderstandingUnit>();
    assert_send_sync::<ApplicationModelProducerInput>();
    assert_send_sync::<ApplicationModelProposalDraft>();
    assert_send_sync::<ApplicationUnderstandingRuntimeError>();
    assert_send_sync::<ApplicationUnderstandingRuntimeOutcome>();

    fn assert_producer<T: ApplicationModelProposalProducer>() {}
    let _ = assert_producer::<NeverProducer>;
}

struct NeverProducer;

#[async_trait::async_trait]
impl ApplicationModelProposalProducer for NeverProducer {
    async fn produce(
        &self,
        _input: ApplicationModelProducerInput,
    ) -> Result<ApplicationModelProposalDraft, ApplicationUnderstandingRuntimeError> {
        unreachable!("the public contract test never invokes the producer")
    }
}

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read reserved local postgres port")
        .port()
}

struct ControlledHttpFixture {
    origin: String,
    request_count: Arc<AtomicUsize>,
    _task: tokio::task::JoinHandle<()>,
}

impl ControlledHttpFixture {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind controlled localhost HTTP fixture");
        let address = listener
            .local_addr()
            .expect("read localhost fixture address");
        let request_count = Arc::new(AtomicUsize::new(0));
        let count = request_count.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _peer)) = listener.accept().await else {
                    break;
                };
                let count = count.clone();
                tokio::spawn(async move {
                    let mut request = vec![0_u8; 4096];
                    let read = stream.read(&mut request).await.unwrap_or_default();
                    let request = String::from_utf8_lossy(&request[..read]);
                    if request.starts_with("GET /verified HTTP/1.1\r\n") {
                        count.fetch_add(1, Ordering::SeqCst);
                    }
                    let body = b"localhost-proof";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("write localhost response headers");
                    stream
                        .write_all(body)
                        .await
                        .expect("write localhost response body");
                });
            }
        });
        Self {
            origin: format!("http://{address}"),
            request_count,
            _task: task,
        }
    }
}

fn directory_entry_row_hash(
    directory_entry_id: Uuid,
    target_id: Uuid,
    url: &str,
    status_code: i32,
    content_length: i32,
) -> String {
    format!(
        "sha256:{}",
        sha256_json(&json!({
            "content_length": content_length,
            "content_type": "",
            "id": directory_entry_id,
            "status_code": status_code,
            "target_id": target_id,
            "tool": "route_probe",
            "url": url,
        }))
    )
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => serde_json::to_string(value).unwrap(),
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
                        serde_json::to_string(key).unwrap(),
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

fn target_identity_hash(target_type: &str, target_value: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(target_type.as_bytes());
    digest.update([0]);
    digest.update(target_value.as_bytes());
    format!(
        "sha256:{}",
        digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

struct RuntimeFixture {
    db: GolishDb,
    _data_dir: TempDir,
    session_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    additional_organization_id: Option<Uuid>,
    workspace: String,
    scope_hash: String,
    fence: runtime_memory_tx::RuntimeMemoryTxFence,
    source: Option<RuntimeSourceFixture>,
}

#[derive(Debug, Clone, Copy)]
struct RuntimeSourceFixture {
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    deliverable_submission_id: Uuid,
    handoff_id: Uuid,
    evidence_id: i64,
}

impl RuntimeFixture {
    async fn start(label: &str, with_source: bool) -> Self {
        Self::start_inner(label, with_source, false, false, false).await
    }

    async fn start_v2(label: &str, with_source: bool) -> Self {
        Self::start_inner(label, with_source, false, true, false).await
    }

    async fn start_v2_two_companies(label: &str, with_source: bool) -> Self {
        Self::start_inner(label, with_source, false, true, true).await
    }

    async fn start_with_pending_work_item(label: &str, with_source: bool) -> Self {
        Self::start_inner(label, with_source, true, false, false).await
    }

    async fn start_inner(
        label: &str,
        with_source: bool,
        pending_work_item: bool,
        force_v2_rollouts: bool,
        include_second_company: bool,
    ) -> Self {
        let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
        let config = DbConfig {
            pg_data_dir: data_dir.path().join("pgdata"),
            port: reserve_local_port(),
            database: format!(
                "application_understanding_runtime_{label}_{}",
                Uuid::new_v4().simple()
            ),
            ..DbConfig::default()
        };
        let db = GolishDb::start(config)
            .await
            .expect("start fresh migrated embedded postgres");
        if force_v2_rollouts {
            let mut tx = db.pool().begin().await.expect("begin V2 rollout fixture");
            sqlx::raw_sql(
                r#"ALTER TABLE runtime_memory_rollout
                       DISABLE TRIGGER runtime_memory_rollout_forward_only;
                   ALTER TABLE runtime_memory_rollout
                       DISABLE TRIGGER zz_runtime_memory_rollout_attestation_gate;
                   ALTER TABLE runtime_memory_rollout
                       DISABLE TRIGGER zz_runtime_memory_rollout_promotion_receipt;
                   UPDATE runtime_memory_rollout
                      SET contract='v2_only',contract_rank=3,row_version=3,updated_at=NOW()
                    WHERE singleton_id=1;
                   ALTER TABLE runtime_memory_rollout
                       ENABLE TRIGGER runtime_memory_rollout_forward_only;
                   ALTER TABLE runtime_memory_rollout
                       ENABLE TRIGGER zz_runtime_memory_rollout_attestation_gate;
                   ALTER TABLE runtime_memory_rollout
                       ENABLE TRIGGER zz_runtime_memory_rollout_promotion_receipt;
                   ALTER TABLE attack_execution_rollout
                       DISABLE TRIGGER attack_execution_rollout_forward_only;
                   ALTER TABLE attack_execution_rollout
                       DISABLE TRIGGER zz_attack_runtime_rollout_compatibility;
                   ALTER TABLE attack_execution_rollout
                       DISABLE TRIGGER zz_attack_execution_rollout_promotion_receipt;
                   UPDATE attack_execution_rollout
                      SET contract='v2_only',rank=3,row_version=3,updated_at=NOW()
                    WHERE singleton=TRUE;
                   ALTER TABLE attack_execution_rollout
                       ENABLE TRIGGER attack_execution_rollout_forward_only;
                   ALTER TABLE attack_execution_rollout
                       ENABLE TRIGGER zz_attack_runtime_rollout_compatibility;
                   ALTER TABLE attack_execution_rollout
                       ENABLE TRIGGER zz_attack_execution_rollout_promotion_receipt;
                   ALTER TABLE investigation_rollout
                       DISABLE TRIGGER investigation_rollout_direct_mutation_guard;
                   UPDATE investigation_rollout
                      SET contract_version='hypothesis_registry_v1',
                          rollout_mode='new_only',mode_rank=4,
                          row_version=row_version+1
                    WHERE singleton=TRUE;
                   ALTER TABLE investigation_rollout
                       ENABLE TRIGGER investigation_rollout_direct_mutation_guard;
                   ALTER TABLE tool_truth_rollout
                       DISABLE TRIGGER tool_truth_rollout_direct_mutation_guard;
                   UPDATE tool_truth_rollout
                      SET new_operation_contract='receipt_v1',
                          row_version=row_version+1
                    WHERE singleton=TRUE;
                   ALTER TABLE tool_truth_rollout
                       ENABLE TRIGGER tool_truth_rollout_direct_mutation_guard;"#,
            )
            .execute(&mut *tx)
            .await
            .expect("align isolated rollout singletons to V2");
            tx.commit().await.expect("commit V2 rollout fixture");
        }
        let workspace_dir = data_dir.path().join("workspace");
        std::fs::create_dir(&workspace_dir).expect("create isolated runtime workspace");
        let workspace = std::fs::canonicalize(&workspace_dir)
            .expect("canonical isolated runtime workspace")
            .to_string_lossy()
            .into_owned();
        let workspace_sha256 = Sha256::digest(workspace.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let project = project_scopes::register_first_open(db.pool(), &workspace, &workspace_sha256)
            .await
            .expect("register runtime project");
        let organization_id = Uuid::new_v4();
        sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$2,'Runtime Org')")
            .bind(organization_id)
            .bind(&workspace)
            .execute(db.pool())
            .await
            .expect("insert runtime organization");
        let additional_organization_id = if include_second_company {
            let subsidiary_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO organizations(id,project_path,name,parent_id) \
                 VALUES($1,$2,'Runtime Subsidiary',$3)",
            )
            .bind(subsidiary_id)
            .bind(&workspace)
            .bind(organization_id)
            .execute(db.pool())
            .await
            .expect("insert runtime subsidiary organization");
            Some(subsidiary_id)
        } else {
            None
        };
        let session = sessions::create(
            db.pool(),
            NewSession {
                title: Some("Application Understanding runtime fixture".to_string()),
                workspace_path: Some(workspace.clone()),
                workspace_label: None,
                model: Some("fixture-model".to_string()),
                provider: Some("fixture-provider".to_string()),
                project_path: Some(workspace.clone()),
            },
        )
        .await
        .expect("create runtime session");
        let operation_id = Uuid::new_v4();
        let mut stage_execution_id = Uuid::new_v4();
        runtime_memory_tx::create_runtime_operation(
            db.pool(),
            &runtime_memory_tx::CreateRuntimeOperationRow {
                operation_id,
                initial_stage_execution_id: stage_execution_id,
                session_id: session.id,
                title: Some("Application Understanding runtime fixture".to_string()),
                input: "fixture".to_string(),
                profile: "red_team".to_string(),
                entry_stage: "scoping".to_string(),
                application_model_contract:
                    golish_core::ApplicationModelContract::ApplicationModelV1,
                project_scope_id: project.project_scope_id,
                cli_scope: Some(runtime_memory_tx::CliRuntimeScopeRow {
                    root_organization_id: organization_id,
                    include_subsidiaries: additional_organization_id.is_some(),
                    subsidiary_threshold: 51,
                    units: std::iter::once(runtime_memory_tx::CliRuntimeScopeUnitRow {
                        organization_id,
                        parent_organization_id: None,
                        organization_name: "Runtime Org".to_string(),
                        depth: 0,
                        ordinal: 0,
                        ownership_percent: None,
                        approval_source: json!({"kind": "fixture"}),
                    })
                    .chain(additional_organization_id.map(|subsidiary_id| {
                        runtime_memory_tx::CliRuntimeScopeUnitRow {
                            organization_id: subsidiary_id,
                            parent_organization_id: Some(organization_id),
                            organization_name: "Runtime Subsidiary".to_string(),
                            depth: 1,
                            ordinal: 1,
                            ownership_percent: Some("100".to_string()),
                            approval_source: json!({"kind": "fixture"}),
                        }
                    }))
                    .collect(),
                }),
            },
        )
        .await
        .expect("create exact V2 runtime operation");
        for next_stage in [
            "target_intel",
            "external_attack_surface",
            "enumeration",
            "vuln_triage",
            "application_understanding",
        ] {
            let next_stage_execution_id = Uuid::new_v4();
            runtime_memory_tx::transition_stage_execution(
                db.pool(),
                &runtime_memory_tx::TransitionStageExecutionRow {
                    operation_id,
                    current_stage_execution_id: stage_execution_id,
                    next_stage_execution_id,
                    next_stage: next_stage.to_string(),
                },
            )
            .await
            .unwrap_or_else(|error| panic!("advance AU fixture to {next_stage}: {error}"));
            stage_execution_id = next_stage_execution_id;
        }
        let (scope_snapshot_id, scope_hash) = sqlx::query_as::<_, (Uuid, String)>(
            "SELECT id,scope_hash FROM operation_org_scope_snapshots WHERE operation_id=$1",
        )
        .bind(operation_id)
        .fetch_one(db.pool())
        .await
        .expect("read runtime scope");

        let source = if with_source {
            let source_execution_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO stage_runs(id,operation_id,stage_kind,status) \
                 VALUES($1,$2,'vuln_triage','completed')",
            )
            .bind(source_execution_id)
            .bind(operation_id)
            .execute(db.pool())
            .await
            .expect("insert source execution");
            let source_unit_id = Uuid::new_v4();
            let source_worker_id = Uuid::new_v4();
            let source_lease = Uuid::new_v4();
            let source_tool_id = Uuid::new_v4();
            let source_submission_id = Uuid::new_v4();
            let source_handoff_id = Uuid::new_v4();
            let evidence_id: i64 = sqlx::query_scalar(
                r#"INSERT INTO audit_log(
                       action,category,details,project_path,audit_role,run_id,detail
                   ) VALUES('application model source','attack','',$1,'evidence',$2,$3)
                   RETURNING id"#,
            )
            .bind(&workspace)
            .bind(operation_id)
            .bind(json!({"organization_id": organization_id, "route": "/orders/{id}"}))
            .fetch_one(db.pool())
            .await
            .expect("insert source evidence");
            let source_payload = json!({
                "schema_version": 1,
                "organization_id": organization_id,
                "routes": ["/orders/{id}"],
            });
            let source_handoff_payload = json!({
                "schema_version": 1,
                "canonical_fact_refs": [],
                "typed_claims": [{
                    "kind": "vuln_source_fixture",
                    "payload": {
                        "organization_id": organization_id,
                        "routes": ["/orders/{id}"]
                    }
                }],
                "coverage_watermark": {"complete": true},
                "evidence_ids": [evidence_id]
            });
            sqlx::query(
                r#"INSERT INTO stage_run_units(
                       id,operation_id,stage_execution_id,scope_snapshot_id,
                       organization_id,stage_kind,generation,specialist,status,
                       terminal_at,pass_watermark
                   ) VALUES($1,$2,$3,$4,$5,'vuln_triage',0,'vuln_triage','passed',NOW(),$6)"#,
            )
            .bind(source_unit_id)
            .bind(operation_id)
            .bind(source_execution_id)
            .bind(scope_snapshot_id)
            .bind(organization_id)
            .bind(json!({"final_gate_passed": true}))
            .execute(db.pool())
            .await
            .expect("insert passed source unit");
            sqlx::query(
                r#"INSERT INTO stage_worker_runs(
                       id,operation_id,stage_execution_id,stage_run_unit_id,
                       organization_id,worker_generation,specialist,work_item_kind,
                       work_item_key,agent_path,status,lease_token,lease_owner,
                       lease_acquired_at,lease_expires_at,heartbeat_at,attempt_epoch
                   ) VALUES($1,$2,$3,$4,$5,0,'vuln_triage','stage_unit','vuln_triage',
                            'main>vuln_triage','passed',$6,'source-fixture',
                            NOW(),NOW()+INTERVAL '5 minutes',NOW(),0)"#,
            )
            .bind(source_worker_id)
            .bind(operation_id)
            .bind(source_execution_id)
            .bind(source_unit_id)
            .bind(organization_id)
            .bind(source_lease)
            .execute(db.pool())
            .await
            .expect("insert passed source worker");
            sqlx::query(
                r#"INSERT INTO tool_calls(
                       id,call_id,session_id,task_id,agent,name,args,result,status,
                       operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
                       organization_id,attempt_epoch,lease_token
                   ) VALUES($1,'runtime-source-submit',$2,$3,'primary','submit_stage_deliverable',
                            '{}','{}','finished',$3,$4,$5,$6,$7,0,$8)"#,
            )
            .bind(source_tool_id)
            .bind(session.id)
            .bind(operation_id)
            .bind(source_execution_id)
            .bind(source_unit_id)
            .bind(source_worker_id)
            .bind(organization_id)
            .bind(source_lease)
            .execute(db.pool())
            .await
            .expect("insert source tool receipt");
            sqlx::query(
                r#"INSERT INTO stage_deliverable_submissions(
                       id,operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
                       organization_id,tool_call_record_id,tool_request_id,stage_kind,
                       attempt_epoch,lease_token,payload,payload_sha256
                   ) VALUES($1,$2,$3,$4,$5,$6,$7,'runtime-source-submit',
                            'vuln_triage',0,$8,$9,$10)"#,
            )
            .bind(source_submission_id)
            .bind(operation_id)
            .bind(source_execution_id)
            .bind(source_unit_id)
            .bind(source_worker_id)
            .bind(organization_id)
            .bind(source_tool_id)
            .bind(source_lease)
            .bind(&source_payload)
            .bind(sha256_json(&source_payload))
            .execute(db.pool())
            .await
            .expect("insert source submission");
            sqlx::query(
                r#"INSERT INTO stage_handoffs(
                       id,operation_id,organization_id,scope_snapshot_id,from_stage_kind,
                       stage_execution_id,source_stage_run_unit_id,deliverable_submission_id,
                       scope_hash,payload,payload_sha256,evidence_ids,coverage_watermark,
                       unit_gate_decision_hash,gate_passed_at
                   ) VALUES($1,$2,$3,$4,'vuln_triage',$5,$6,$7,$8,$9,$10,$11,$12,$13,NOW())"#,
            )
            .bind(source_handoff_id)
            .bind(operation_id)
            .bind(organization_id)
            .bind(scope_snapshot_id)
            .bind(source_execution_id)
            .bind(source_unit_id)
            .bind(source_submission_id)
            .bind(&scope_hash)
            .bind(&source_handoff_payload)
            .bind(sha256_json(&source_handoff_payload))
            .bind(vec![evidence_id])
            .bind(json!({"complete": true}))
            .bind("5".repeat(64))
            .execute(db.pool())
            .await
            .expect("insert source handoff");
            Some(RuntimeSourceFixture {
                stage_execution_id: source_execution_id,
                stage_run_unit_id: source_unit_id,
                deliverable_submission_id: source_submission_id,
                handoff_id: source_handoff_id,
                evidence_id,
            })
        } else {
            None
        };

        if let (Some(subsidiary_id), Some(primary_source)) =
            (additional_organization_id, source.as_ref())
        {
            let source_unit_id = Uuid::new_v4();
            let source_worker_id = Uuid::new_v4();
            let source_lease = Uuid::new_v4();
            let source_tool_id = Uuid::new_v4();
            let source_submission_id = Uuid::new_v4();
            let source_handoff_id = Uuid::new_v4();
            let evidence_id: i64 = sqlx::query_scalar(
                r#"INSERT INTO audit_log(
                       action,category,details,project_path,audit_role,run_id,detail
                   ) VALUES('application model subsidiary source','attack','',$1,
                            'evidence',$2,$3) RETURNING id"#,
            )
            .bind(&workspace)
            .bind(operation_id)
            .bind(json!({"organization_id": subsidiary_id, "route": "/subsidiary"}))
            .fetch_one(db.pool())
            .await
            .expect("insert subsidiary source evidence");
            let source_payload = json!({
                "schema_version": 1,
                "organization_id": subsidiary_id,
                "routes": ["/subsidiary"],
            });
            let source_handoff_payload = json!({
                "schema_version": 1,
                "canonical_fact_refs": [],
                "typed_claims": [{
                    "kind": "vuln_source_fixture",
                    "payload": {
                        "organization_id": subsidiary_id,
                        "routes": ["/subsidiary"]
                    }
                }],
                "coverage_watermark": {"complete": true},
                "evidence_ids": [evidence_id]
            });
            sqlx::query(
                r#"INSERT INTO stage_run_units(
                       id,operation_id,stage_execution_id,scope_snapshot_id,
                       organization_id,stage_kind,generation,specialist,status,
                       terminal_at,pass_watermark
                   ) VALUES($1,$2,$3,$4,$5,'vuln_triage',0,'vuln_triage','passed',
                            NOW(),$6)"#,
            )
            .bind(source_unit_id)
            .bind(operation_id)
            .bind(primary_source.stage_execution_id)
            .bind(scope_snapshot_id)
            .bind(subsidiary_id)
            .bind(json!({"final_gate_passed": true}))
            .execute(db.pool())
            .await
            .expect("insert passed subsidiary source unit");
            sqlx::query(
                r#"INSERT INTO stage_worker_runs(
                       id,operation_id,stage_execution_id,stage_run_unit_id,
                       organization_id,worker_generation,specialist,work_item_kind,
                       work_item_key,agent_path,status,lease_token,lease_owner,
                       lease_acquired_at,lease_expires_at,heartbeat_at,attempt_epoch
                   ) VALUES($1,$2,$3,$4,$5,0,'vuln_triage','stage_unit','vuln_triage',
                            'main>vuln_triage','passed',$6,'source-fixture',
                            NOW(),NOW()+INTERVAL '5 minutes',NOW(),0)"#,
            )
            .bind(source_worker_id)
            .bind(operation_id)
            .bind(primary_source.stage_execution_id)
            .bind(source_unit_id)
            .bind(subsidiary_id)
            .bind(source_lease)
            .execute(db.pool())
            .await
            .expect("insert passed subsidiary source worker");
            sqlx::query(
                r#"INSERT INTO tool_calls(
                       id,call_id,session_id,task_id,agent,name,args,result,status,
                       operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
                       organization_id,attempt_epoch,lease_token
                   ) VALUES($1,$2,$3,$4,'primary','submit_stage_deliverable','{}','{}',
                            'finished',$4,$5,$6,$7,$8,0,$9)"#,
            )
            .bind(source_tool_id)
            .bind(format!("runtime-source-submit-{subsidiary_id}"))
            .bind(session.id)
            .bind(operation_id)
            .bind(primary_source.stage_execution_id)
            .bind(source_unit_id)
            .bind(source_worker_id)
            .bind(subsidiary_id)
            .bind(source_lease)
            .execute(db.pool())
            .await
            .expect("insert subsidiary source tool receipt");
            sqlx::query(
                r#"INSERT INTO stage_deliverable_submissions(
                       id,operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
                       organization_id,tool_call_record_id,tool_request_id,stage_kind,
                       attempt_epoch,lease_token,payload,payload_sha256
                   ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,'vuln_triage',0,$9,$10,$11)"#,
            )
            .bind(source_submission_id)
            .bind(operation_id)
            .bind(primary_source.stage_execution_id)
            .bind(source_unit_id)
            .bind(source_worker_id)
            .bind(subsidiary_id)
            .bind(source_tool_id)
            .bind(format!("runtime-source-submit-{subsidiary_id}"))
            .bind(source_lease)
            .bind(&source_payload)
            .bind(sha256_json(&source_payload))
            .execute(db.pool())
            .await
            .expect("insert subsidiary source submission");
            sqlx::query(
                r#"INSERT INTO stage_handoffs(
                       id,operation_id,organization_id,scope_snapshot_id,from_stage_kind,
                       stage_execution_id,source_stage_run_unit_id,deliverable_submission_id,
                       scope_hash,payload,payload_sha256,evidence_ids,coverage_watermark,
                       unit_gate_decision_hash,gate_passed_at
                   ) VALUES($1,$2,$3,$4,'vuln_triage',$5,$6,$7,$8,$9,$10,$11,$12,$13,NOW())"#,
            )
            .bind(source_handoff_id)
            .bind(operation_id)
            .bind(subsidiary_id)
            .bind(scope_snapshot_id)
            .bind(primary_source.stage_execution_id)
            .bind(source_unit_id)
            .bind(source_submission_id)
            .bind(&scope_hash)
            .bind(&source_handoff_payload)
            .bind(sha256_json(&source_handoff_payload))
            .bind(vec![evidence_id])
            .bind(json!({"complete": true}))
            .bind("6".repeat(64))
            .execute(db.pool())
            .await
            .expect("insert subsidiary source handoff");
        }

        let stage_run_unit_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO stage_run_units(
                   id,operation_id,stage_execution_id,scope_snapshot_id,
                   organization_id,stage_kind,generation,specialist,status,started_at
               ) VALUES($1,$2,$3,$4,$5,'application_understanding',0,
                        'application_understanding','running',NOW())"#,
        )
        .bind(stage_run_unit_id)
        .bind(operation_id)
        .bind(stage_execution_id)
        .bind(scope_snapshot_id)
        .bind(organization_id)
        .execute(db.pool())
        .await
        .expect("insert running Application Understanding unit");
        let team_work_item_id = if pending_work_item {
            let plan_id = Uuid::new_v4();
            let submitter_item_id = Uuid::new_v4();
            sqlx::query(
                r#"INSERT INTO stage_team_plans(
                       id,operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
                       organization_id,stage_kind,unit_generation,schema_version,plan_version,
                       plan_hash,leader_role,aggregator_kind,aggregator_role,allowed_worker_roles,
                       max_workers_total,max_workers_active,dynamic_requests_allowed,
                       dynamic_request_policy,dispatch_epoch,final_submitter_kind,
                       created_from_stage_spec_hash
                   ) VALUES($1,$2,$3,$4,$5,$6,'application_understanding',0,1,1,$7,
                            'application_understanding','worker','application_understanding',$8,
                            2,1,FALSE,'{}'::JSONB,0,'worker',$9)"#,
            )
            .bind(plan_id)
            .bind(operation_id)
            .bind(stage_execution_id)
            .bind(stage_run_unit_id)
            .bind(scope_snapshot_id)
            .bind(organization_id)
            .bind(format!("sha256:{}", "7".repeat(64)))
            .bind(json!(["application_understanding"]))
            .bind(format!("sha256:{}", "8".repeat(64)))
            .execute(db.pool())
            .await
            .expect("insert pending-work TeamPlan");
            sqlx::query(
                r#"INSERT INTO stage_work_items(
                       id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                       scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
                       input_manifest_hash,input_refs,required_for_barrier,priority,status,
                       attempt_policy,budget,output_schema,created_by,started_at
                   ) VALUES($1,$2,$3,$4,$5,$6,$7,0,'stage_unit',
                            'application_understanding','application_understanding',$8,
                            '[]'::JSONB,FALSE,0,'running','{}'::JSONB,'{}'::JSONB,
                            'application_model.v1','server_seed',NOW())"#,
            )
            .bind(submitter_item_id)
            .bind(plan_id)
            .bind(operation_id)
            .bind(stage_execution_id)
            .bind(stage_run_unit_id)
            .bind(scope_snapshot_id)
            .bind(organization_id)
            .bind(format!("sha256:{}", "a".repeat(64)))
            .execute(db.pool())
            .await
            .expect("insert Team submitter WorkItem");
            sqlx::query(
                r#"INSERT INTO stage_work_items(
                       id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                       scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
                       input_manifest_hash,input_refs,required_for_barrier,priority,status,
                       attempt_policy,budget,output_schema,created_by
                   ) VALUES($1,$2,$3,$4,$5,$6,$7,0,'analysis','pending-analysis',
                            'application_understanding',$8,'[]'::JSONB,TRUE,0,'queued',
                            '{}'::JSONB,'{}'::JSONB,'application_model_item.v1','server_seed')"#,
            )
            .bind(Uuid::new_v4())
            .bind(plan_id)
            .bind(operation_id)
            .bind(stage_execution_id)
            .bind(stage_run_unit_id)
            .bind(scope_snapshot_id)
            .bind(organization_id)
            .bind(format!("sha256:{}", "9".repeat(64)))
            .execute(db.pool())
            .await
            .expect("insert pending producer WorkItem");
            Some(submitter_item_id)
        } else {
            None
        };
        let worker_run_id = Uuid::new_v4();
        let lease_token = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO stage_worker_runs(
                   id,operation_id,stage_execution_id,stage_run_unit_id,
                   organization_id,worker_generation,specialist,work_item_kind,
                   work_item_key,agent_path,status,lease_token,lease_owner,
                   lease_acquired_at,lease_expires_at,heartbeat_at,attempt_epoch,work_item_id
               ) VALUES($1,$2,$3,$4,$5,0,'application_understanding','stage_unit',
                        'application_understanding',$6,'running',
                        $7,'runtime-fixture',NOW(),NOW()+INTERVAL '5 minutes',NOW(),0,$8)"#,
        )
        .bind(worker_run_id)
        .bind(operation_id)
        .bind(stage_execution_id)
        .bind(stage_run_unit_id)
        .bind(organization_id)
        .bind(format!(
            "main>org:{organization_id}>application_understanding"
        ))
        .bind(lease_token)
        .bind(team_work_item_id)
        .execute(db.pool())
        .await
        .expect("insert live Application Understanding worker");
        if let Some(subsidiary_id) = additional_organization_id {
            let subsidiary_unit_id = Uuid::new_v4();
            sqlx::query(
                r#"INSERT INTO stage_run_units(
                       id,operation_id,stage_execution_id,scope_snapshot_id,
                       organization_id,stage_kind,generation,specialist,status
                   ) VALUES($1,$2,$3,$4,$5,'application_understanding',0,
                            'application_understanding','queued')"#,
            )
            .bind(subsidiary_unit_id)
            .bind(operation_id)
            .bind(stage_execution_id)
            .bind(scope_snapshot_id)
            .bind(subsidiary_id)
            .execute(db.pool())
            .await
            .expect("insert queued subsidiary Application Understanding unit");
            sqlx::query(
                r#"INSERT INTO stage_worker_runs(
                       id,operation_id,stage_execution_id,stage_run_unit_id,
                       organization_id,worker_generation,specialist,work_item_kind,
                       work_item_key,agent_path,status,attempt_epoch
                   ) VALUES($1,$2,$3,$4,$5,0,'application_understanding','stage_unit',
                            'application_understanding',$6,'queued',0)"#,
            )
            .bind(Uuid::new_v4())
            .bind(operation_id)
            .bind(stage_execution_id)
            .bind(subsidiary_unit_id)
            .bind(subsidiary_id)
            .bind(format!(
                "main>org:{subsidiary_id}>application_understanding"
            ))
            .execute(db.pool())
            .await
            .expect("insert queued subsidiary Application Understanding worker");
        }
        Self {
            db,
            _data_dir: data_dir,
            session_id: session.id,
            scope_snapshot_id,
            organization_id,
            additional_organization_id,
            workspace,
            scope_hash,
            fence: runtime_memory_tx::RuntimeMemoryTxFence {
                operation_id,
                stage_execution_id,
                stage_run_unit_id,
                worker_run_id,
                lease_token,
                attempt_epoch: 0,
                expected_checkpoint_version: 0,
            },
            source,
        }
    }

    fn command(&self) -> RunApplicationUnderstandingUnit {
        RunApplicationUnderstandingUnit {
            session_id: self.session_id,
            fence: self.fence.clone(),
            expected_unit_row_version: 0,
            scope_hash: self.scope_hash.clone(),
        }
    }

    async fn prepare_for_formal_stage_controller(&self) {
        sqlx::query(
            r#"UPDATE stage_run_units
                  SET status='queued',started_at=NULL,updated_at=NOW()
                WHERE id=$1"#,
        )
        .bind(self.fence.stage_run_unit_id)
        .execute(self.db.pool())
        .await
        .expect("reset AU Unit to controller-owned queued state");
        sqlx::query(
            r#"UPDATE stage_worker_runs
                  SET status='queued',agent_path=$2,message_chain_id=NULL,
                      checkpoint='{}'::JSONB,checkpoint_version=0,
                      lease_token=NULL,lease_owner=NULL,lease_acquired_at=NULL,
                      lease_expires_at=NULL,heartbeat_at=NULL,attempt_epoch=0,
                      active_tool_call_id=NULL,active_tool_started_at=NULL,
                      started_at=NULL,terminal_at=NULL,updated_at=NOW()
                WHERE id=$1"#,
        )
        .bind(self.fence.worker_run_id)
        .bind(format!(
            "main>org:{}>application_understanding",
            self.organization_id
        ))
        .execute(self.db.pool())
        .await
        .expect("reset AU Worker to controller-owned queued state");
    }

    async fn insert_safe_projection_surface(&self) {
        let target_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO targets(id,name,target_type,value,scope,project_path,organization_id)
               VALUES($1,'Application target','domain','app.example.test','in',$2,$3)"#,
        )
        .bind(target_id)
        .bind(&self.workspace)
        .bind(self.organization_id)
        .execute(self.db.pool())
        .await
        .expect("insert normalized application target");
        let origin_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO web_origins(
                   id,organization_id,project_path,scheme,host,host_type,port,origin,source
               ) VALUES($1,$2,$3,'https','app.example.test','domain',443,
                        'https://app.example.test','fixture')"#,
        )
        .bind(origin_id)
        .bind(self.organization_id)
        .bind(&self.workspace)
        .execute(self.db.pool())
        .await
        .expect("insert normalized web origin");
        sqlx::query(
            r#"INSERT INTO web_origin_observations(
                   organization_id,project_path,web_origin_id,target_id,source,
                   capture_path,final_url,raw
               ) VALUES($1,$2,$3,$4,'fixture','/private/Cookie-capture.txt',
                        'https://app.example.test/?token=do-not-forward',
                        '{"headers":{"Cookie":"session=do-not-forward"}}'::JSONB)"#,
        )
        .bind(self.organization_id)
        .bind(&self.workspace)
        .bind(origin_id)
        .bind(target_id)
        .execute(self.db.pool())
        .await
        .expect("insert observation containing forbidden capture material");
        sqlx::query(
            r#"INSERT INTO network_endpoints(
                   id,organization_id,project_path,ip,port,transport,state,
                   service_name,service_product,service_version,banner,tls_detected,source
               ) VALUES($1,$2,$3,'192.0.2.10',22,'tcp','open','ssh','OpenSSH','9.0',
                        'private banner do-not-forward',FALSE,'fixture')"#,
        )
        .bind(Uuid::new_v4())
        .bind(self.organization_id)
        .bind(&self.workspace)
        .execute(self.db.pool())
        .await
        .expect("insert uncovered normalized service");
    }

    async fn insert_running_sibling(&self) {
        sqlx::query(
            r#"INSERT INTO stage_worker_runs(
                   id,operation_id,stage_execution_id,stage_run_unit_id,
                   organization_id,worker_generation,specialist,work_item_kind,
                   work_item_key,agent_path,status,lease_token,lease_owner,
                   lease_acquired_at,lease_expires_at,heartbeat_at,attempt_epoch
               ) SELECT $1,operation_id,stage_execution_id,stage_run_unit_id,
                        organization_id,worker_generation,'application_understanding',
                        'analysis','sibling-analysis','main>application_understanding>sibling',
                        'running',$2,'runtime-sibling',NOW(),
                        NOW()+INTERVAL '5 minutes',NOW(),0
                   FROM stage_worker_runs WHERE id=$3"#,
        )
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(self.fence.worker_run_id)
        .execute(self.db.pool())
        .await
        .expect("insert running sibling producer");
    }

    #[allow(dead_code)]
    async fn insert_terminal_sibling(&self) {
        sqlx::query(
            r#"INSERT INTO stage_worker_runs(
                   id,operation_id,stage_execution_id,stage_run_unit_id,
                   organization_id,worker_generation,specialist,work_item_kind,
                   work_item_key,agent_path,status,attempt_epoch,terminal_at
               ) SELECT $1,operation_id,stage_execution_id,stage_run_unit_id,
                        organization_id,worker_generation,'application_understanding',
                        'analysis','terminal-sibling-analysis',
                        'main>application_understanding>terminal-sibling',
                        'failed',0,NOW()
                   FROM stage_worker_runs WHERE id=$2"#,
        )
        .bind(Uuid::new_v4())
        .bind(self.fence.worker_run_id)
        .execute(self.db.pool())
        .await
        .expect("insert terminal sibling producer");
    }

    async fn insert_standalone_model_submission(&self, finish_tool: bool) -> Uuid {
        let seeded = application_models::seed_manifest_from_current_predecessors(
            self.db.pool(),
            &DeriveApplicationModelManifestSeed {
                operation_id: self.fence.operation_id,
                scope_snapshot_id: self.scope_snapshot_id,
                stage_execution_id: self.fence.stage_execution_id,
                stage_run_unit_id: self.fence.stage_run_unit_id,
                organization_id: self.organization_id,
            },
        )
        .await
        .expect("seed manifest before simulating response loss");
        let material = application_models::load_gate_material(
            self.db.pool(),
            &LoadApplicationModelGateMaterial {
                manifest_id: seeded.manifest.id,
                operation_id: self.fence.operation_id,
                scope_snapshot_id: self.scope_snapshot_id,
                stage_execution_id: self.fence.stage_execution_id,
                stage_run_unit_id: self.fence.stage_run_unit_id,
                organization_id: self.organization_id,
            },
        )
        .await
        .expect("load manifest inputs for standalone payload");
        let draft = CountingProducer::valid()
            .produce(ApplicationModelProducerInput {
                manifest_id: seeded.manifest.id,
                organization_id: self.organization_id,
                inputs: material.inputs,
            })
            .await
            .expect("build a valid standalone payload");
        let payload = json!({
            "stage_id": "application_understanding",
            "stage_run_id": self.fence.stage_execution_id,
            "schema_version": 1,
            "manifest_id": seeded.manifest.id,
            "structured_model": draft.structured_model,
            "decisions": draft.decisions.iter().map(|decision| json!({
                "input_key": decision.input_key,
                "disposition": decision.disposition.as_str(),
                "item_keys": decision.item_keys,
                "duplicate_input_key": decision.duplicate_input_key,
                "reason_code": decision.reason_code,
            })).collect::<Vec<_>>(),
            "items": draft.items.iter().map(|item| json!({
                "item_key": item.item_key,
                "item_kind": item.item_kind,
                "truth_state": item.truth_state.as_str(),
                "source_input_keys": item.source_input_keys,
                "referenced_item_keys": item.referenced_item_keys,
                "payload": item.payload,
                "evidence": item.evidence.iter().map(|evidence| json!({
                    "evidence_id": evidence.evidence_id,
                    "role": evidence.role.as_str(),
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        });
        let request_id = format!(
            "application-understanding-submit:{}:{}:{}",
            self.fence.worker_run_id, self.fence.attempt_epoch, seeded.manifest.id
        );
        let runtime = tool_calls::RuntimeToolIdentity {
            operation_id: self.fence.operation_id,
            stage_execution_id: self.fence.stage_execution_id,
            stage_run_unit_id: Some(self.fence.stage_run_unit_id),
            worker_run_id: Some(self.fence.worker_run_id),
            organization_id: Some(self.organization_id),
            attempt_epoch: Some(self.fence.attempt_epoch),
            lease_token: Some(self.fence.lease_token),
        };
        let tool_call_record_id = tool_calls::record_tracked_start(
            self.db.pool(),
            &request_id,
            self.session_id,
            Some(self.fence.operation_id),
            None,
            "submit_stage_deliverable",
            &json!({"manifest_id": seeded.manifest.id, "server_owned": true}),
            Some(&runtime),
        )
        .await
        .expect("start standalone submit receipt");
        runtime_memory_tx::begin_worker_tool(self.db.pool(), &self.fence, tool_call_record_id)
            .await
            .expect("bind standalone submit receipt");
        let canonical_payload_json = canonical_json(&payload);
        let submission = stage_deliverable_submissions::insert(
            self.db.pool(),
            &stage_deliverable_submissions::NewStageDeliverableSubmission {
                operation_id: self.fence.operation_id,
                stage_execution_id: self.fence.stage_execution_id,
                stage_run_unit_id: Some(self.fence.stage_run_unit_id),
                worker_run_id: Some(self.fence.worker_run_id),
                organization_id: Some(self.organization_id),
                tool_call_record_id,
                tool_request_id: request_id,
                stage_kind: "application_understanding".to_string(),
                attempt_epoch: Some(self.fence.attempt_epoch),
                lease_token: Some(self.fence.lease_token),
                payload_sha256: sha256_json(&payload),
                canonical_payload_json,
            },
        )
        .await
        .expect("commit standalone deliverable before revision");
        runtime_memory_tx::finish_worker_tool(self.db.pool(), &self.fence, tool_call_record_id)
            .await
            .expect("clear standalone submit active-tool fence");
        if finish_tool {
            tool_calls::record_tracked_finish(
                self.db.pool(),
                tool_call_record_id,
                self.session_id,
                "finished",
                &canonical_json(&json!({
                    "accepted": true,
                    "deliverable_submission_id": submission.id,
                })),
                0,
            )
            .await
            .expect("finish standalone submit receipt");
        }
        submission.id
    }

    async fn freeze_candidate_manifest(&self) -> (Uuid, Uuid) {
        let (wave_run_id, wave_unit_id, _) = self
            .freeze_candidate_manifest_for_target("https://runtime.example.test/orders/{id}")
            .await;
        (wave_run_id, wave_unit_id)
    }

    async fn freeze_candidate_manifest_for_target(&self, target_value: &str) -> (Uuid, Uuid, Uuid) {
        let source = self.source.expect("Candidate fixture requires vuln source");
        let wave_run_id = Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!("{}:candidate-wave:0", self.fence.operation_id).as_bytes(),
        );
        let wave_unit_id = Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!("{wave_run_id}:{}", self.organization_id).as_bytes(),
        );
        let policy_snapshot = json!({
            "max_attempts_total": 200,
            "max_candidates_total": 100,
            "max_chain_depth": 3,
            "max_waves": 3,
        });
        let observation = json!({
            "schema": "candidate_application_model_test_v1",
            "route": "/orders/{id}",
        });
        let target_type = "url";
        let target_id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("{}:{target_value}", self.organization_id).as_bytes(),
        );
        let project_path: String =
            sqlx::query_scalar("SELECT project_path FROM organizations WHERE id=$1")
                .bind(self.organization_id)
                .fetch_one(self.db.pool())
                .await
                .expect("read Candidate target project path");
        sqlx::query(
            r#"INSERT INTO targets(id,name,target_type,value,scope,project_path,organization_id)
               VALUES($1,'Runtime candidate target','url',$2,'in',$3,$4)"#,
        )
        .bind(target_id)
        .bind(target_value)
        .bind(project_path)
        .bind(self.organization_id)
        .execute(self.db.pool())
        .await
        .expect("insert live Candidate target");
        let mut tx = self
            .db
            .pool()
            .begin()
            .await
            .expect("begin Candidate freeze");
        attack_waves::open_from_vuln_triage_handoff(
            &mut tx,
            &attack_waves::OpenAttackWaveUnit {
                wave_run_id,
                wave_unit_id,
                operation_id: self.fence.operation_id,
                scope_snapshot_id: self.scope_snapshot_id,
                organization_id: self.organization_id,
                entry_stage_execution_id: source.stage_execution_id,
                entry_stage_run_unit_id: source.stage_run_unit_id,
                entry_deliverable_submission_id: source.deliverable_submission_id,
                generation: 0,
                ordinal: 0,
                policy_snapshot,
                policy_hash:
                    "sha256:66e50329b4bb217eb060bcccb38f78f4b0eafc163471bebd6554d271d1a6b326"
                        .to_string(),
                max_waves: 3,
                max_candidates_total: 100,
                max_chain_depth: 3,
                max_attempts_total: 200,
            },
        )
        .await
        .expect("open exact Candidate WaveUnit");
        attack_candidate_work_items::seed_wave_work_items(
            &mut tx,
            SeedAttackWorkItems {
                operation_id: self.fence.operation_id,
                scope_snapshot_id: self.scope_snapshot_id,
                wave_run_id,
                wave_unit_id,
                organization_id: self.organization_id,
                observations: vec![SeedAttackObservation {
                    work_item_key: "route:/orders/{id}".to_string(),
                    target_live_id: Some(target_id),
                    target_type_at_time: target_type.to_string(),
                    target_value_at_time: target_value.to_string(),
                    target_identity_hash: target_identity_hash(target_type, target_value),
                    technique: "WSTG-INPV-05".to_string(),
                    observation_hash: format!("sha256:{}", sha256_json(&observation)),
                    observation,
                    source_fact_delta_id: None,
                    delta_kind: None,
                    observation_kind: "candidate_application_model_test_v1".to_string(),
                    allowed_techniques: vec!["WSTG-INPV-05".to_string()],
                    enrichment_required: false,
                    evidence_ids: vec![source.evidence_id],
                }],
            },
        )
        .await
        .expect("freeze exact Candidate manifest");
        tx.commit().await.expect("commit Candidate manifest freeze");
        (wave_run_id, wave_unit_id, target_id)
    }

    async fn start_candidate_runtime_unit(&self) -> CandidateRuntimeFixture {
        let stage_execution_id = Uuid::new_v4();
        runtime_memory_tx::transition_stage_execution(
            self.db.pool(),
            &runtime_memory_tx::TransitionStageExecutionRow {
                operation_id: self.fence.operation_id,
                current_stage_execution_id: self.fence.stage_execution_id,
                next_stage_execution_id: stage_execution_id,
                next_stage: "attack_candidate".to_string(),
            },
        )
        .await
        .expect("transition Application Understanding to Candidate");

        let stage_run_unit_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO stage_run_units(
                   id,operation_id,stage_execution_id,scope_snapshot_id,
                   organization_id,stage_kind,generation,specialist,status,started_at
               ) VALUES($1,$2,$3,$4,$5,'attack_candidate',0,
                        'attack_candidate','running',NOW())"#,
        )
        .bind(stage_run_unit_id)
        .bind(self.fence.operation_id)
        .bind(stage_execution_id)
        .bind(self.scope_snapshot_id)
        .bind(self.organization_id)
        .execute(self.db.pool())
        .await
        .expect("insert running Candidate Unit");

        let worker_run_id = Uuid::new_v4();
        let lease_token = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO stage_worker_runs(
                   id,operation_id,stage_execution_id,stage_run_unit_id,
                   organization_id,worker_generation,specialist,work_item_kind,
                   work_item_key,agent_path,status,lease_token,lease_owner,
                   lease_acquired_at,lease_expires_at,heartbeat_at,attempt_epoch
               ) VALUES($1,$2,$3,$4,$5,0,'attack_candidate','stage_unit',
                        'attack_candidate','main>attack_candidate','running',$6,
                        'candidate-fixture',NOW(),NOW()+INTERVAL '5 minutes',NOW(),0)"#,
        )
        .bind(worker_run_id)
        .bind(self.fence.operation_id)
        .bind(stage_execution_id)
        .bind(stage_run_unit_id)
        .bind(self.organization_id)
        .bind(lease_token)
        .execute(self.db.pool())
        .await
        .expect("insert live Candidate worker");

        let tool_call_record_id = Uuid::new_v4();
        let tool_request_id = format!("candidate-submit-{tool_call_record_id}");
        sqlx::query(
            r#"INSERT INTO tool_calls(
                   id,call_id,session_id,task_id,agent,name,args,result,status,
                   operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
                   organization_id,attempt_epoch,lease_token
               ) VALUES($1,$2,$3,$4,'primary','submit_stage_deliverable','{}','{}',
                        'finished',$4,$5,$6,$7,$8,0,$9)"#,
        )
        .bind(tool_call_record_id)
        .bind(&tool_request_id)
        .bind(self.session_id)
        .bind(self.fence.operation_id)
        .bind(stage_execution_id)
        .bind(stage_run_unit_id)
        .bind(worker_run_id)
        .bind(self.organization_id)
        .bind(lease_token)
        .execute(self.db.pool())
        .await
        .expect("insert Candidate submission receipt");

        let deliverable_submission_id = Uuid::new_v4();
        let payload = json!({"schema_version": 1, "stage": "attack_candidate"});
        sqlx::query(
            r#"INSERT INTO stage_deliverable_submissions(
                   id,operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
                   organization_id,tool_call_record_id,tool_request_id,stage_kind,
                   attempt_epoch,lease_token,payload,payload_sha256
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,'attack_candidate',0,$9,$10,$11)"#,
        )
        .bind(deliverable_submission_id)
        .bind(self.fence.operation_id)
        .bind(stage_execution_id)
        .bind(stage_run_unit_id)
        .bind(worker_run_id)
        .bind(self.organization_id)
        .bind(tool_call_record_id)
        .bind(&tool_request_id)
        .bind(lease_token)
        .bind(&payload)
        .bind(sha256_json(&payload))
        .execute(self.db.pool())
        .await
        .expect("insert Candidate deliverable submission");

        CandidateRuntimeFixture {
            stage_execution_id,
            stage_run_unit_id,
            worker_run_id,
            lease_token,
            deliverable_submission_id,
        }
    }
}

struct TypedStageProducer {
    legacy_calls: AtomicUsize,
    shard_calls: AtomicUsize,
    synthesis_calls: AtomicUsize,
    valid: bool,
    fail_shards: bool,
    omit_shard_evidence: bool,
    fail_organization_id: Option<Uuid>,
}

impl TypedStageProducer {
    fn new() -> Self {
        Self {
            legacy_calls: AtomicUsize::new(0),
            shard_calls: AtomicUsize::new(0),
            synthesis_calls: AtomicUsize::new(0),
            valid: true,
            fail_shards: false,
            omit_shard_evidence: false,
            fail_organization_id: None,
        }
    }

    fn invalid() -> Self {
        Self {
            legacy_calls: AtomicUsize::new(0),
            shard_calls: AtomicUsize::new(0),
            synthesis_calls: AtomicUsize::new(0),
            valid: false,
            fail_shards: false,
            omit_shard_evidence: false,
            fail_organization_id: None,
        }
    }

    fn shard_failure() -> Self {
        Self {
            legacy_calls: AtomicUsize::new(0),
            shard_calls: AtomicUsize::new(0),
            synthesis_calls: AtomicUsize::new(0),
            valid: true,
            fail_shards: true,
            omit_shard_evidence: false,
            fail_organization_id: None,
        }
    }

    fn evidenceless_shards() -> Self {
        Self {
            legacy_calls: AtomicUsize::new(0),
            shard_calls: AtomicUsize::new(0),
            synthesis_calls: AtomicUsize::new(0),
            valid: true,
            fail_shards: false,
            omit_shard_evidence: true,
            fail_organization_id: None,
        }
    }

    fn fail_one_company(organization_id: Uuid) -> Self {
        Self {
            legacy_calls: AtomicUsize::new(0),
            shard_calls: AtomicUsize::new(0),
            synthesis_calls: AtomicUsize::new(0),
            valid: true,
            fail_shards: false,
            omit_shard_evidence: false,
            fail_organization_id: Some(organization_id),
        }
    }
}

#[async_trait::async_trait]
impl AgentExecutor for TypedStageProducer {
    async fn execute_subtask(
        &self,
        _subtask_title: &str,
        _subtask_description: &str,
        _execution_context: &ExecutionContext,
        _agent_type: Option<&str>,
    ) -> anyhow::Result<AgentResult> {
        panic!("formal AU controller must not enter the generic agentic loop")
    }

    async fn produce_application_model(
        &self,
        input: ApplicationModelProducerInputContract,
    ) -> anyhow::Result<ApplicationModelProposalContract> {
        self.legacy_calls.fetch_add(1, Ordering::SeqCst);
        if !self.valid {
            return Ok(ApplicationModelProposalContract {
                structured_model: json!({"schema_version": "application_model.v1"}),
                decisions: Vec::new(),
                items: Vec::new(),
            });
        }
        let item_key = "workflow:formal-controller".to_string();
        let mut source_input_keys = input
            .inputs
            .iter()
            .map(|source| source.input_key.clone())
            .collect::<Vec<_>>();
        source_input_keys.sort();
        let mut evidence_ids = input
            .inputs
            .iter()
            .flat_map(|source| source.evidence_ids.iter().copied())
            .collect::<Vec<_>>();
        evidence_ids.sort_unstable();
        evidence_ids.dedup();
        Ok(ApplicationModelProposalContract {
            structured_model: json!({
                "organization_id": input.organization_id,
                "summary": "formal controller fixture",
                "technologies": [],
                "routes_and_pages": [],
                "api_surfaces": [],
                "roles_and_identities": [],
                "business_entities": [],
                "workflows": [item_key],
                "state_transitions": [],
                "ownership_rules": [],
                "sensitive_operations": [],
                "trust_boundaries": [],
                "unknowns": [],
            }),
            decisions: source_input_keys
                .iter()
                .map(|input_key| ApplicationModelDecisionContract {
                    input_key: input_key.clone(),
                    disposition: ApplicationModelInputDispositionContract::Incorporated,
                    item_keys: vec![item_key.clone()],
                    duplicate_input_key: None,
                    reason_code: None,
                })
                .collect(),
            items: vec![ApplicationModelItemContract {
                item_key,
                item_kind: "workflow".to_string(),
                truth_state: ApplicationModelTruthStateContract::Observed,
                source_input_keys,
                referenced_item_keys: Vec::new(),
                payload: json!({"path": "/orders/{id}"}),
                evidence: evidence_ids
                    .into_iter()
                    .map(|evidence_id| ApplicationModelEvidenceContract {
                        evidence_id,
                        role: ApplicationModelEvidenceRoleContract::Observation,
                    })
                    .collect(),
            }],
        })
    }

    async fn analyze_application_model_work_item(
        &self,
        input: ApplicationModelWorkItemInputContract,
    ) -> anyhow::Result<ApplicationModelWorkItemOutputContract> {
        self.shard_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_shards || self.fail_organization_id == Some(input.organization_id) {
            return Err(ApplicationModelProducerFailure::ResponseNonContract.into());
        }
        let encoded_projection = serde_json::to_string(&input.projection)
            .expect("serialize closed shard projection in fixture");
        for forbidden in ["do-not-forward", "cookie", "capture_path", "private banner"] {
            assert!(
                !encoded_projection.to_ascii_lowercase().contains(forbidden),
                "forbidden raw material reached shard: {forbidden}"
            );
        }
        let mut source_input_keys = input
            .projection
            .manifest_inputs
            .iter()
            .map(|source| source.input_key.clone())
            .collect::<Vec<_>>();
        source_input_keys.sort();
        source_input_keys.dedup();
        let mut evidence_ids = input
            .projection
            .manifest_inputs
            .iter()
            .flat_map(|source| source.evidence_ids.iter().copied())
            .collect::<Vec<_>>();
        evidence_ids.sort_unstable();
        evidence_ids.dedup();
        if self.omit_shard_evidence {
            evidence_ids.clear();
        }
        let truth_state = if evidence_ids.is_empty() {
            ApplicationModelTruthStateContract::Inferred
        } else {
            ApplicationModelTruthStateContract::Observed
        };
        Ok(ApplicationModelWorkItemOutputContract {
            organization_id: input.organization_id,
            work_item_id: input.work_item_id,
            work_item_key: input.work_item_key,
            projection_hash: input.projection_hash,
            summary: "formal static shard fixture".to_string(),
            items: vec![ApplicationModelWorkItemPartialContract {
                item_key: "workflow:formal-controller".to_string(),
                item_kind: ApplicationModelPartialItemKindContract::Workflow,
                truth_state,
                summary: "Evidence-bound workflow inferred from a redacted shard".to_string(),
                source_input_keys,
                evidence: evidence_ids
                    .into_iter()
                    .map(|evidence_id| ApplicationModelEvidenceContract {
                        evidence_id,
                        role: ApplicationModelEvidenceRoleContract::Observation,
                    })
                    .collect(),
            }],
            unknowns: Vec::new(),
        })
    }

    async fn synthesize_application_model(
        &self,
        input: ApplicationModelSynthesisInputContract,
    ) -> anyhow::Result<ApplicationModelProposalContract> {
        self.synthesis_calls.fetch_add(1, Ordering::SeqCst);
        if !self.valid {
            return Ok(ApplicationModelProposalContract {
                structured_model: json!({"schema_version": "application_model.v1"}),
                decisions: Vec::new(),
                items: Vec::new(),
            });
        }
        let item_key = "workflow:formal-controller".to_string();
        let mut source_input_keys = input
            .manifest_inputs
            .iter()
            .map(|source| source.input_key.clone())
            .collect::<Vec<_>>();
        source_input_keys.sort();
        source_input_keys.dedup();
        let mut evidence_ids = input
            .partial_outputs
            .iter()
            .flat_map(|output| output.items.iter())
            .flat_map(|item| item.evidence.iter().map(|evidence| evidence.evidence_id))
            .collect::<Vec<_>>();
        evidence_ids.sort_unstable();
        evidence_ids.dedup();
        Ok(ApplicationModelProposalContract {
            structured_model: json!({
                "organization_id": input.organization_id,
                "summary": "formal hierarchical controller fixture",
                "technologies": [],
                "routes_and_pages": [],
                "api_surfaces": [],
                "roles_and_identities": [],
                "business_entities": [],
                "workflows": [item_key],
                "state_transitions": [],
                "ownership_rules": [],
                "sensitive_operations": [],
                "trust_boundaries": [],
                "unknowns": [],
            }),
            decisions: source_input_keys
                .iter()
                .map(|input_key| ApplicationModelDecisionContract {
                    input_key: input_key.clone(),
                    disposition: ApplicationModelInputDispositionContract::Incorporated,
                    item_keys: vec![item_key.clone()],
                    duplicate_input_key: None,
                    reason_code: None,
                })
                .collect(),
            items: vec![ApplicationModelItemContract {
                item_key,
                item_kind: "workflow".to_string(),
                truth_state: ApplicationModelTruthStateContract::Observed,
                source_input_keys,
                referenced_item_keys: Vec::new(),
                payload: json!({"summary": "redacted static shard synthesis"}),
                evidence: evidence_ids
                    .into_iter()
                    .map(|evidence_id| ApplicationModelEvidenceContract {
                        evidence_id,
                        role: ApplicationModelEvidenceRoleContract::Observation,
                    })
                    .collect(),
            }],
        })
    }

    async fn generate_report(
        &self,
        _execution_context: &ExecutionContext,
    ) -> anyhow::Result<AgentResult> {
        Ok(AgentResult::new(String::new()))
    }

    async fn reflect(&self, _subtask_title: &str, _agent_response: &str) -> anyhow::Result<String> {
        Ok(String::new())
    }
}

#[async_trait::async_trait]
impl ApplicationModelAgentRunner for TypedStageProducer {
    async fn run_work_item(
        &self,
        binding: ApplicationModelAgentBinding,
        input: ApplicationModelWorkItemInputContract,
    ) -> anyhow::Result<ApplicationModelAgentAttempt<ApplicationModelWorkItemOutputContract>> {
        assert_eq!(binding.operation_id, input.operation_id);
        assert_eq!(binding.stage_run_unit_id, input.stage_run_unit_id);
        assert_eq!(binding.organization_id, input.organization_id);
        assert_eq!(binding.work_item_id, input.work_item_id);
        assert_eq!(binding.work_item_key, input.work_item_key);
        assert_eq!(binding.work_item_role, "application_model_worker");
        assert_eq!(
            binding.parent_request_id,
            format!(
                "formal-au-stage-run::team::{}::worker:{}",
                binding.organization_id, binding.worker_run_id
            )
        );
        let outcome = match AgentExecutor::analyze_application_model_work_item(self, input).await {
            Ok(output) => ApplicationModelAgentOutcome::Completed(output),
            Err(error) => ApplicationModelAgentOutcome::Failed(
                error
                    .downcast_ref::<ApplicationModelProducerFailure>()
                    .copied()
                    .unwrap_or(ApplicationModelProducerFailure::Unavailable),
            ),
        };
        Ok(ApplicationModelAgentAttempt {
            outcome,
            checkpoint_version: binding.checkpoint_version,
            checkpoint_body: binding.checkpoint_body,
        })
    }

    async fn run_synthesis(
        &self,
        binding: ApplicationModelAgentBinding,
        input: ApplicationModelSynthesisInputContract,
    ) -> anyhow::Result<ApplicationModelAgentAttempt<ApplicationModelProposalContract>> {
        assert_eq!(binding.operation_id, input.operation_id);
        assert_eq!(binding.stage_run_unit_id, input.stage_run_unit_id);
        assert_eq!(binding.organization_id, input.organization_id);
        assert_eq!(binding.work_item_role, "application_model_synthesizer");
        assert_eq!(
            binding.parent_request_id,
            format!(
                "formal-au-stage-run::team::{}::lead:{}",
                binding.organization_id, binding.worker_run_id
            )
        );
        let outcome = match AgentExecutor::synthesize_application_model(self, input).await {
            Ok(proposal) if self.valid => ApplicationModelAgentOutcome::Completed(proposal),
            Ok(_) => ApplicationModelAgentOutcome::Failed(
                ApplicationModelProducerFailure::ResponseNonContract,
            ),
            Err(error) => ApplicationModelAgentOutcome::Failed(
                error
                    .downcast_ref::<ApplicationModelProducerFailure>()
                    .copied()
                    .unwrap_or(ApplicationModelProducerFailure::Unavailable),
            ),
        };
        Ok(ApplicationModelAgentAttempt {
            outcome,
            checkpoint_version: binding.checkpoint_version,
            checkpoint_body: binding.checkpoint_body,
        })
    }
}

struct SynthesisRunnerError {
    producer: TypedStageProducer,
}

#[async_trait::async_trait]
impl ApplicationModelAgentRunner for SynthesisRunnerError {
    async fn run_work_item(
        &self,
        binding: ApplicationModelAgentBinding,
        input: ApplicationModelWorkItemInputContract,
    ) -> anyhow::Result<ApplicationModelAgentAttempt<ApplicationModelWorkItemOutputContract>> {
        ApplicationModelAgentRunner::run_work_item(&self.producer, binding, input).await
    }

    async fn run_synthesis(
        &self,
        binding: ApplicationModelAgentBinding,
        _input: ApplicationModelSynthesisInputContract,
    ) -> anyhow::Result<ApplicationModelAgentAttempt<ApplicationModelProposalContract>> {
        assert_eq!(binding.work_item_role, "application_model_synthesizer");
        assert_eq!(
            binding.parent_request_id,
            format!(
                "formal-au-stage-run::team::{}::lead:{}",
                binding.organization_id, binding.worker_run_id
            )
        );
        self.producer.synthesis_calls.fetch_add(1, Ordering::SeqCst);
        Err(anyhow::anyhow!("synthetic runner infrastructure failure"))
    }
}

struct FinalizationCommitFailureRunner {
    producer: TypedStageProducer,
    pool: sqlx::PgPool,
    armed: AtomicBool,
}

#[async_trait::async_trait]
impl ApplicationModelAgentRunner for FinalizationCommitFailureRunner {
    async fn run_work_item(
        &self,
        binding: ApplicationModelAgentBinding,
        input: ApplicationModelWorkItemInputContract,
    ) -> anyhow::Result<ApplicationModelAgentAttempt<ApplicationModelWorkItemOutputContract>> {
        ApplicationModelAgentRunner::run_work_item(&self.producer, binding, input).await
    }

    async fn run_synthesis(
        &self,
        binding: ApplicationModelAgentBinding,
        input: ApplicationModelSynthesisInputContract,
    ) -> anyhow::Result<ApplicationModelAgentAttempt<ApplicationModelProposalContract>> {
        if !self.armed.swap(true, Ordering::SeqCst) {
            sqlx::query(
                r#"CREATE FUNCTION test_fail_application_model_current_insert()
                   RETURNS trigger AS $$
                   BEGIN
                       RAISE EXCEPTION 'TEST_APPLICATION_MODEL_CURRENT_COMMIT_FAILED';
                   END;
                   $$ LANGUAGE plpgsql"#,
            )
            .execute(&self.pool)
            .await
            .expect("create deterministic Application Model finalization failure function");
            sqlx::query(
                r#"CREATE TRIGGER test_fail_application_model_current_insert
                   BEFORE INSERT ON application_model_current_revisions
                   FOR EACH ROW EXECUTE FUNCTION test_fail_application_model_current_insert()"#,
            )
            .execute(&self.pool)
            .await
            .expect("arm deterministic Application Model finalization failure trigger");
        }
        ApplicationModelAgentRunner::run_synthesis(&self.producer, binding, input).await
    }
}

struct WorkItemRunnerError {
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl ApplicationModelAgentRunner for WorkItemRunnerError {
    async fn run_work_item(
        &self,
        binding: ApplicationModelAgentBinding,
        _input: ApplicationModelWorkItemInputContract,
    ) -> anyhow::Result<ApplicationModelAgentAttempt<ApplicationModelWorkItemOutputContract>> {
        assert_eq!(binding.work_item_role, "application_model_worker");
        assert_eq!(
            binding.parent_request_id,
            format!(
                "formal-au-stage-run::team::{}::worker:{}",
                binding.organization_id, binding.worker_run_id
            )
        );
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(anyhow::anyhow!("synthetic work-item runner failure"))
    }

    async fn run_synthesis(
        &self,
        _binding: ApplicationModelAgentBinding,
        _input: ApplicationModelSynthesisInputContract,
    ) -> anyhow::Result<ApplicationModelAgentAttempt<ApplicationModelProposalContract>> {
        panic!("work-item runner exhaustion must prevent synthesis")
    }
}

#[derive(Debug, Clone, Copy)]
struct CandidateRuntimeFixture {
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    worker_run_id: Uuid,
    lease_token: Uuid,
    deliverable_submission_id: Uuid,
}

async fn insert_follow_on_candidate_runtime_unit(
    fixture: &RuntimeFixture,
    current_stage_execution_id: Uuid,
    generation: i32,
) -> CandidateRuntimeFixture {
    let stage_execution_id = Uuid::new_v4();
    runtime_memory_tx::transition_stage_execution(
        fixture.db.pool(),
        &runtime_memory_tx::TransitionStageExecutionRow {
            operation_id: fixture.fence.operation_id,
            current_stage_execution_id,
            next_stage_execution_id: stage_execution_id,
            next_stage: "attack_candidate".to_string(),
        },
    )
    .await
    .expect("transition Verification to follow-on Candidate");
    let stage_run_unit_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,
               organization_id,stage_kind,generation,specialist,status,started_at
           ) VALUES($1,$2,$3,$4,$5,'attack_candidate',$6,
                    'attack_candidate','running',NOW())"#,
    )
    .bind(stage_run_unit_id)
    .bind(fixture.fence.operation_id)
    .bind(stage_execution_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.organization_id)
    .bind(generation)
    .execute(fixture.db.pool())
    .await
    .expect("insert follow-on Candidate Unit");
    let worker_run_id = Uuid::new_v4();
    let lease_token = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,
               organization_id,worker_generation,specialist,work_item_kind,
               work_item_key,agent_path,status,lease_token,lease_owner,
               lease_acquired_at,lease_expires_at,heartbeat_at,attempt_epoch
           ) VALUES($1,$2,$3,$4,$5,0,'attack_candidate','stage_unit',
                    'attack_candidate','main>attack_candidate','running',$6,
                    'candidate-follow-on-fixture',NOW(),NOW()+INTERVAL '5 minutes',NOW(),0)"#,
    )
    .bind(worker_run_id)
    .bind(fixture.fence.operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(fixture.organization_id)
    .bind(lease_token)
    .execute(fixture.db.pool())
    .await
    .expect("insert follow-on Candidate Worker");
    let tool_call_record_id = Uuid::new_v4();
    let tool_request_id = format!("candidate-follow-on-submit-{tool_call_record_id}");
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,result,status,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token
           ) VALUES($1,$2,$3,$4,'primary','submit_stage_deliverable','{}','{}',
                    'finished',$4,$5,$6,$7,$8,0,$9)"#,
    )
    .bind(tool_call_record_id)
    .bind(&tool_request_id)
    .bind(fixture.session_id)
    .bind(fixture.fence.operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(worker_run_id)
    .bind(fixture.organization_id)
    .bind(lease_token)
    .execute(fixture.db.pool())
    .await
    .expect("insert follow-on Candidate submit receipt");
    let deliverable_submission_id = Uuid::new_v4();
    let payload = json!({"schema_version": 1, "stage": "attack_candidate"});
    sqlx::query(
        r#"INSERT INTO stage_deliverable_submissions(
               id,operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,tool_call_record_id,tool_request_id,stage_kind,
               attempt_epoch,lease_token,payload,payload_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,'attack_candidate',0,$9,$10,$11)"#,
    )
    .bind(deliverable_submission_id)
    .bind(fixture.fence.operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(worker_run_id)
    .bind(fixture.organization_id)
    .bind(tool_call_record_id)
    .bind(&tool_request_id)
    .bind(lease_token)
    .bind(&payload)
    .bind(sha256_json(&payload))
    .execute(fixture.db.pool())
    .await
    .expect("insert follow-on Candidate deliverable submission");
    CandidateRuntimeFixture {
        stage_execution_id,
        stage_run_unit_id,
        worker_run_id,
        lease_token,
        deliverable_submission_id,
    }
}

#[cfg(any())]
fn candidate_final_seal_input(
    fixture: &RuntimeFixture,
    runtime: CandidateRuntimeFixture,
    acceptance: CandidateAcceptanceInput,
) -> runtime_memory_tx::FinalizeUnitPassRow {
    let mut canonical_fact_keys = acceptance
        .expected_work_item_ids
        .iter()
        .copied()
        .map(
            |work_item_id| canonical_fact_refs::CanonicalFactKey::AttackCandidateWorkItem {
                work_item_id,
            },
        )
        .collect::<Vec<_>>();
    if let Some(authority) = acceptance.application_model_authority.as_ref() {
        canonical_fact_keys.push(
            canonical_fact_refs::CanonicalFactKey::ApplicationModelRevision {
                revision_id: authority.revision_id,
            },
        );
    }
    canonical_fact_keys.sort_by_key(|key| canonical_json(&serde_json::to_value(key).unwrap()));

    let mut evidence_ids = acceptance
        .candidates
        .iter()
        .flat_map(|decision| decision.evidence_ids.iter().copied())
        .chain(
            acceptance
                .no_candidate_decisions
                .iter()
                .flat_map(|decision| decision.evidence_ids.iter().copied()),
        )
        .collect::<Vec<_>>();
    evidence_ids.sort_unstable();
    evidence_ids.dedup();
    let mut expected_work_item_ids = acceptance.expected_work_item_ids.clone();
    expected_work_item_ids.sort_unstable();
    let mut candidate_ids = acceptance
        .candidates
        .iter()
        .map(|decision| decision.candidate_id)
        .collect::<Vec<_>>();
    candidate_ids.sort_unstable();
    let mut no_candidate_work_item_ids = acceptance
        .no_candidate_decisions
        .iter()
        .map(|decision| decision.work_item_id)
        .collect::<Vec<_>>();
    no_candidate_work_item_ids.sort_unstable();
    let mut typed_claims = acceptance
        .candidates
        .iter()
        .map(|decision| {
            json!({
                "kind": "attack_candidate_decision",
                "payload": {
                    "candidate_id": decision.candidate_id,
                    "work_item_id": decision.work_item_id,
                    "hypothesis": decision.hypothesis,
                    "technique": decision.technique,
                    "rationale": decision.rationale,
                    "candidate_plan_hash": decision.candidate_plan_hash,
                    "risk_class": decision.risk_class,
                    "evidence_ids": decision.evidence_ids,
                }
            })
        })
        .chain(acceptance.no_candidate_decisions.iter().map(|decision| {
            json!({
                "kind": "attack_no_candidate_decision",
                "payload": {
                    "work_item_id": decision.work_item_id,
                    "reason_code": decision.reason_code,
                    "detail": decision.detail,
                    "evidence_ids": decision.evidence_ids,
                }
            })
        }))
        .collect::<Vec<_>>();
    typed_claims.sort_by_key(canonical_json);
    let mut coverage_watermark = json!({
        "kind": "candidate_manifest_v1",
        "stage": "attack_candidate",
        "organization_id": fixture.organization_id,
        "wave_run_id": acceptance.wave_run_id,
        "wave_unit_id": acceptance.wave_unit_id,
        "manifest_hash": acceptance.manifest_hash,
        "expected_work_item_ids": expected_work_item_ids,
        "candidate_ids": candidate_ids,
        "no_candidate_work_item_ids": no_candidate_work_item_ids,
        "decision_evidence_ids": evidence_ids,
        "terminal_count": acceptance.candidates.len() + acceptance.no_candidate_decisions.len(),
        "canonical_ref_total": canonical_fact_keys.len(),
        "canonical_ref_included": canonical_fact_keys.len(),
        "canonical_ref_truncated": false,
        "typed_claim_total": typed_claims.len(),
        "typed_claim_included": typed_claims.len(),
        "typed_claim_truncated": false,
        "evidence_id_total": evidence_ids.len(),
        "evidence_id_included": evidence_ids.len(),
        "evidence_id_truncated": false,
    });
    if let Some(authority) = acceptance.application_model_authority.as_ref() {
        coverage_watermark["kind"] = json!("candidate_manifest_application_model_v1");
        coverage_watermark["application_model_revision_id"] = json!(authority.revision_id);
        coverage_watermark["candidate_input_authority_hash"] =
            json!(authority.input_authority_hash);
    }
    let terminal_checkpoint = json!({"terminal": true});
    let details = json!({
        "source": "authoritative_org_gate",
        "stage": "attack_candidate",
        "organization_id": fixture.organization_id,
    });
    let seal_material = json!({
        "canonical_fact_keys": canonical_fact_keys,
        "typed_claims": typed_claims,
        "coverage_watermark": coverage_watermark,
        "evidence_ids": evidence_ids,
        "terminal_checkpoint": terminal_checkpoint,
        "deterministic_gate_details": details,
        "candidate_acceptance": acceptance,
    });
    let gate_decision = json!({
        "outcome": "pass",
        "operation_id": fixture.fence.operation_id,
        "stage_execution_id": runtime.stage_execution_id,
        "stage_run_unit_id": runtime.stage_run_unit_id,
        "deliverable_submission_id": runtime.deliverable_submission_id,
        "scope_hash": fixture.scope_hash,
        "details": details,
        "seal_material_sha256": sha256_json(&seal_material),
    });
    runtime_memory_tx::FinalizeUnitPassRow {
        fence: runtime_memory_tx::RuntimeMemoryTxFence {
            operation_id: fixture.fence.operation_id,
            stage_execution_id: runtime.stage_execution_id,
            stage_run_unit_id: runtime.stage_run_unit_id,
            worker_run_id: runtime.worker_run_id,
            lease_token: runtime.lease_token,
            attempt_epoch: 0,
            expected_checkpoint_version: 0,
        },
        deliverable_submission_id: runtime.deliverable_submission_id,
        expected_unit_status: stage_run_units::StageRunUnitStatus::Running,
        expected_unit_row_version: 0,
        scope_hash: fixture.scope_hash.clone(),
        gate_decision_hash: sha256_json(&gate_decision),
        gate_decision,
        aggregate_pass_token_hash: None,
        canonical_fact_keys,
        typed_claims,
        coverage_watermark,
        evidence_ids,
        terminal_checkpoint,
        candidate_acceptance: Some(acceptance),
    }
}

#[cfg(any())]
struct LocalhostActionHarness {
    fixture: RuntimeFixture,
    wave_run_id: Uuid,
    wave_unit_id: Uuid,
    verification_stage_execution_id: Uuid,
    verification_stage_run_unit_id: Uuid,
    attempt_id: Uuid,
    worker_run_id: Uuid,
    lease_token: Uuid,
    action_context: AgentToolContext,
    operator_grant: CandidateOperatorGrant,
    workspace: String,
}

#[cfg(any())]
impl LocalhostActionHarness {
    async fn execute_action(&self) -> anyhow::Result<serde_json::Value> {
        let action_tool = VerifyExecuteCandidateActionTool::new(
            Arc::new(golish_pentest::ConfigManager::with_defaults()),
            Arc::new(self.fixture.db.pool().clone()),
        );
        golish_core::with_agent_session(
            Some("candidate-localhost-response-loss".to_string()),
            golish_core::with_agent_tool_context(
                Some(self.action_context.clone()),
                golish_core::with_candidate_operator_grant(
                    Some(self.operator_grant.clone()),
                    action_tool.execute(
                        json!({"action_ordinal": 0}),
                        std::path::Path::new(&self.workspace),
                    ),
                ),
            ),
        )
        .await
    }
}

#[cfg(any())]
async fn prepare_localhost_action_harness(
    label: &str,
    http: &ControlledHttpFixture,
) -> LocalhostActionHarness {
    let fixture = RuntimeFixture::start_v2(label, true).await;
    let outcome = run_application_understanding_unit(
        fixture.db.pool(),
        &fixture.command(),
        &CountingProducer::valid(),
    )
    .await
    .expect("run strict Application Understanding for response-loss fixture");
    assert!(matches!(
        outcome,
        ApplicationUnderstandingRuntimeOutcome::Passed(_)
    ));

    let (wave_run_id, wave_unit_id, target_id) = fixture
        .freeze_candidate_manifest_for_target(&http.origin)
        .await;
    let application_model =
        attack_candidate_work_items::bind_frozen_manifest_to_current_application_model(
            fixture.db.pool(),
            fixture.fence.operation_id,
            fixture.scope_snapshot_id,
            wave_run_id,
            wave_unit_id,
            fixture.organization_id,
        )
        .await
        .expect("bind response-loss Candidate to strict Application Model");
    let work_item_id = application_model.manifest.items[0].work_item.id;
    let work_target_identity_hash = application_model.manifest.items[0]
        .work_item
        .target_identity_hash
        .clone();
    let source_evidence_id = fixture.source.expect("vuln source").evidence_id;
    let verified_url = format!("{}/verified", http.origin);
    let content_length = i32::try_from(b"localhost-proof".len()).unwrap();
    let directory_entry_id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO directory_entries(
               target_id,url,status_code,content_length,lines,words,tool,project_path)
           SELECT $1,$2,200,$3,1,1,'route_probe',project_path
             FROM targets WHERE id=$1
           RETURNING id"#,
    )
    .bind(target_id)
    .bind(&verified_url)
    .bind(content_length)
    .fetch_one(fixture.db.pool())
    .await
    .expect("insert response-loss localhost directory observation");
    let directory_row_hash = directory_entry_row_hash(
        directory_entry_id,
        target_id,
        &verified_url,
        200,
        content_length,
    );
    let observation = json!({
        "schema": "directory_entry_observation_v1",
        "target_id": target_id,
        "directory_entry_id": directory_entry_id,
        "directory_entry_row_sha256": directory_row_hash,
        "url": verified_url,
        "method": "GET",
        "status_code": 200,
        "content_length": content_length,
        "content_type": "",
        "source_tool": "route_probe",
        "source_evidence_id": source_evidence_id,
        "network_attempted": true,
        "authority_current_after": true,
    });
    let candidate_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!(
            "{}:{work_item_id}:response-loss",
            fixture.fence.operation_id
        )
        .as_bytes(),
    );
    let budget = CandidateBudget {
        max_actions: 1,
        max_requests: 1,
        max_runtime_ms: 30_000,
    };
    let action_recipe = PlannedCandidateAction {
        ordinal: 0,
        capability_id: "verify.directory_entry_replay".to_string(),
        action_kind: "directory_entry_replay".to_string(),
        recipe_version: CANDIDATE_RECIPE_VERSION_DIRECTORY_ENTRY_REPLAY_V2.to_string(),
        executor_contract_version: CANDIDATE_EXECUTOR_CONTRACT_DIRECTORY_ENTRY_REPLAY_V2
            .to_string(),
        canonical_args: json!({
            "authority_current_after": true,
            "background": false,
            "content_length": content_length,
            "content_type": "",
            "directory_entry_id": directory_entry_id,
            "directory_entry_row_sha256": directory_row_hash,
            "executor_contract_version": CANDIDATE_EXECUTOR_CONTRACT_DIRECTORY_ENTRY_REPLAY_V2,
            "follow_redirects": false,
            "method": "GET",
            "network_attempted": true,
            "no_auth": true,
            "observation": observation,
            "observation_hash": format!("sha256:{}", sha256_json(&observation)),
            "recipe_version": CANDIDATE_RECIPE_VERSION_DIRECTORY_ENTRY_REPLAY_V2,
            "source_evidence_id": source_evidence_id,
            "source_tool": "route_probe",
            "status_code": 200,
            "target": http.origin,
            "target_id": target_id,
            "technique": "WSTG-INFO",
            "url": verified_url,
        }),
        side_effect_class: SideEffectClass::ReadOnly,
        required_evidence_role: AttemptEvidenceRole::Proof,
    };
    let execution_plan = serde_json::to_value(CandidateHypothesisAuthority {
        schema_version: CANDIDATE_HYPOTHESIS_SCHEMA_V1.to_string(),
        classifier_version: CANDIDATE_CLASSIFIER_VERSION_V2.to_string(),
        candidate_id,
        target_identity_hash: work_target_identity_hash,
        allowed_techniques: vec!["WSTG-INFO".to_string()],
        allowed_capability_ids: vec!["verify.directory_entry_replay".to_string()],
        allowed_action_kinds: vec!["directory_entry_replay".to_string()],
        max_side_effect_class: SideEffectClass::ReadOnly,
        budget,
        foreground_only: true,
        credential_policy: "frozen_candidate_credentials_only".to_string(),
        scope_policy: "exact_candidate_target_only".to_string(),
        stop_policy: "new_target_service_credential_technique_or_parameters_requires_fact_delta"
            .to_string(),
    })
    .expect("serialize response-loss Candidate hypothesis authority");
    assert!(execution_plan.get("actions").is_none());
    assert!(execution_plan.get("canonical_args").is_none());
    let action_recipes =
        vec![serde_json::to_value(action_recipe).expect("serialize response-loss private recipe")];
    let candidate_plan_hash =
        canonical_execution_plan_hash(&execution_plan).expect("hash response-loss plan");
    let authority = &application_model.authority;
    let acceptance = CandidateAcceptanceInput {
        wave_run_id,
        wave_unit_id,
        manifest_hash: authority.candidate_manifest_hash.clone(),
        application_model_authority: Some(CandidateApplicationModelAcceptance {
            manifest_id: authority.application_model_manifest_id,
            revision_id: authority.application_model_revision_id,
            manifest_hash: authority.application_model_manifest_hash.clone(),
            model_hash: authority.application_model_model_hash.clone(),
            replay_material_hash: authority.application_model_replay_material_hash.clone(),
            stage_handoff_id: authority.application_model_handoff_id,
            stage_execution_id: authority.application_model_stage_execution_id,
            stage_run_unit_id: authority.application_model_stage_run_unit_id,
            deliverable_submission_id: authority.application_model_deliverable_submission_id,
            gate_decision_hash: authority.application_model_gate_decision_hash.clone(),
            input_authority_hash: authority.input_authority_hash.clone(),
        }),
        expected_work_item_ids: vec![work_item_id],
        candidates: vec![AcceptedCandidateDraft {
            candidate_id,
            work_item_id,
            hypothesis: "response-loss localhost path remains reachable".to_string(),
            technique: Some("WSTG-INFO".to_string()),
            rationale: "exact typed response-loss observation".to_string(),
            prior_refs: vec![format!("audit:{source_evidence_id}")],
            suggested_approach: "dynamic_verification_strategy".to_string(),
            priority: "medium".to_string(),
            execution_plan,
            action_recipes,
            candidate_plan_hash: candidate_plan_hash.clone(),
            risk_class: "deterministic_safe".to_string(),
            evidence_ids: vec![source_evidence_id],
        }],
        no_candidate_decisions: Vec::new(),
    };
    let candidate_runtime = fixture.start_candidate_runtime_unit().await;
    runtime_memory_tx::finalize_unit_pass(
        fixture.db.pool(),
        &candidate_final_seal_input(&fixture, candidate_runtime, acceptance),
    )
    .await
    .expect("seal response-loss Candidate");
    let machine_authorization =
        authorize_wave_candidates_by_machine_policy(fixture.db.pool(), fixture.fence.operation_id)
            .await
            .expect("machine-authorize response-loss Candidate hypothesis");
    assert_eq!(machine_authorization.approvals.len(), 1);

    let verification_stage_execution_id = Uuid::new_v4();
    let verification_stage_run_unit_id = Uuid::new_v4();
    runtime_memory_tx::transition_stage_execution(
        fixture.db.pool(),
        &runtime_memory_tx::TransitionStageExecutionRow {
            operation_id: fixture.fence.operation_id,
            current_stage_execution_id: candidate_runtime.stage_execution_id,
            next_stage_execution_id: verification_stage_execution_id,
            next_stage: "verification".to_string(),
        },
    )
    .await
    .expect("transition response-loss Candidate to Verification");
    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,organization_id,
               stage_kind,generation,specialist,status)
           VALUES($1,$2,$3,$4,$5,'verification',0,'candidate_verifier','queued')"#,
    )
    .bind(verification_stage_run_unit_id)
    .bind(fixture.fence.operation_id)
    .bind(verification_stage_execution_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.organization_id)
    .execute(fixture.db.pool())
    .await
    .expect("insert response-loss Verification Unit");
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,organization_id,
               worker_generation,specialist,work_item_kind,work_item_key,agent_path,status)
           VALUES($1,$2,$3,$4,$5,0,'candidate_verifier','organization','verification',
                  'main>stage_run:candidate_verifier','queued')"#,
    )
    .bind(Uuid::new_v4())
    .bind(fixture.fence.operation_id)
    .bind(verification_stage_execution_id)
    .bind(verification_stage_run_unit_id)
    .bind(fixture.organization_id)
    .execute(fixture.db.pool())
    .await
    .expect("insert response-loss Verification primary Worker");
    let claimed = claim_next_candidate_attempt(
        fixture.db.pool(),
        CandidateClaimQuery {
            operation_id: fixture.fence.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id,
            wave_unit_id,
            organization_id: fixture.organization_id,
            verification_stage_execution_id,
            verification_stage_run_unit_id,
            lease_owner: "candidate-response-loss-fixture".to_string(),
            lease_seconds: 300,
        },
    )
    .await
    .expect("claim response-loss Candidate")
    .expect("response-loss Candidate is claimable");
    let lease_token = claimed.worker.lease_token.expect("response-loss lease");
    let worker_lease = golish_core::WorkerLeaseContext {
        worker_run_id: claimed.worker.id,
        stage_run_unit_id: verification_stage_run_unit_id,
        lease_token,
        attempt_epoch: claimed.worker.attempt_epoch,
    };
    let candidate_attempt = CandidateAttemptContextRef {
        candidate_id,
        approval_id: claimed.attempt.approval_id,
        attempt_id: claimed.attempt.id,
        candidate_plan_hash: claimed.attempt.candidate_plan_hash.clone(),
    };
    let repository: Arc<dyn RuntimeMemoryRepository> = Arc::new(GolishDbRepoProvider::new(
        Arc::new(fixture.db.pool().clone()),
    ));
    let proposed = repository
        .propose_candidate_action(ProposeCandidateAction {
            control: ControlCandidateAttempt {
                candidate_attempt: candidate_attempt.clone(),
                fence: RuntimeWorkerFence {
                    operation_id: fixture.fence.operation_id,
                    stage_execution_id: verification_stage_execution_id,
                    stage_run_unit_id: verification_stage_run_unit_id,
                    worker_run_id: claimed.worker.id,
                    lease_token,
                    attempt_epoch: claimed.worker.attempt_epoch,
                    expected_checkpoint_version: claimed.worker.checkpoint_version,
                },
                organization_id: fixture.organization_id,
                lease_owner: "candidate-response-loss-fixture".to_string(),
            },
            proposal_request_id: "candidate-response-loss-jit-action-0".to_string(),
            capability_id: "verify.directory_entry_replay".to_string(),
            rationale: "Use the allowlisted typed replay capability before testing finish loss"
                .to_string(),
        })
        .await
        .expect("propose response-loss JIT Candidate action");
    assert_eq!(proposed.route.action_ordinal, 0);
    assert_eq!(
        proposed.route.capability_id,
        "verify.directory_entry_replay"
    );
    assert_eq!(proposed.route.action_kind, "directory_entry_replay");
    let action_context = AgentToolContext {
        request_id: "localhost-response-loss-action-0".to_string(),
        tool_call_record_id: None,
        tool_name: "verify_execute_candidate_action".to_string(),
        source: golish_core::events::ToolSource::SubAgent {
            agent_id: "candidate_http_operator".to_string(),
            agent_name: "Candidate HTTP Operator".to_string(),
        },
        operation_id: Some(fixture.fence.operation_id),
        stage_execution_id: Some(verification_stage_execution_id),
        stage_run_unit_id: Some(verification_stage_run_unit_id),
        organization_id: Some(fixture.organization_id),
        worker_lease: Some(worker_lease.clone()),
        candidate_attempt: Some(candidate_attempt.clone()),
    };
    let operator_grant = CandidateOperatorGrant {
        operation_id: fixture.fence.operation_id,
        stage_execution_id: verification_stage_execution_id,
        organization_id: fixture.organization_id,
        worker_lease,
        candidate_attempt,
        action_ordinal: proposed.route.action_ordinal,
        capability_id: proposed.route.capability_id,
        action_kind: proposed.route.action_kind,
        operator_agent_id: "candidate_http_operator".to_string(),
    };
    let workspace: String =
        sqlx::query_scalar("SELECT project_path FROM organizations WHERE id=$1")
            .bind(fixture.organization_id)
            .fetch_one(fixture.db.pool())
            .await
            .expect("read response-loss isolated workspace");
    LocalhostActionHarness {
        fixture,
        wave_run_id,
        wave_unit_id,
        verification_stage_execution_id,
        verification_stage_run_unit_id,
        attempt_id: claimed.attempt.id,
        worker_run_id: claimed.worker.id,
        lease_token,
        action_context,
        operator_grant,
        workspace,
    }
}

struct CountingProducer {
    calls: AtomicUsize,
    mode: ProducerMode,
}

#[derive(Debug, Clone, Copy)]
enum ProducerMode {
    Valid,
    Empty,
    DuplicateDecision,
    ObservedWithoutEvidence,
    BadInternalReference,
    MismatchedOrganization,
}

impl CountingProducer {
    fn valid() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            mode: ProducerMode::Valid,
        }
    }

    fn invalid() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            mode: ProducerMode::Empty,
        }
    }

    fn with_mode(mode: ProducerMode) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            mode,
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl ApplicationModelProposalProducer for CountingProducer {
    async fn produce(
        &self,
        input: ApplicationModelProducerInput,
    ) -> Result<ApplicationModelProposalDraft, ApplicationUnderstandingRuntimeError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if matches!(self.mode, ProducerMode::Empty) {
            return Ok(ApplicationModelProposalDraft {
                structured_model: json!({"schema_version": "application_model.v1"}),
                decisions: Vec::new(),
                items: Vec::new(),
            });
        }
        let source = input
            .inputs
            .first()
            .expect("model producer receives one input");
        let evidence_id = *source
            .evidence_ids
            .first()
            .expect("source input carries evidence");
        let mut draft = ApplicationModelProposalDraft {
            structured_model: json!({
                "organization_id": input.organization_id,
                "summary": "Observed order-reading workflow",
                "technologies": [],
                "routes_and_pages": [],
                "api_surfaces": [],
                "roles_and_identities": [],
                "business_entities": [],
                "workflows": ["workflow:order_read"],
                "state_transitions": [],
                "ownership_rules": [],
                "sensitive_operations": [],
                "trust_boundaries": [],
                "unknowns": [],
            }),
            decisions: vec![ApplicationModelInputDecisionSeed {
                input_key: source.input_key.clone(),
                disposition: ApplicationModelInputDispositionRow::Incorporated,
                item_keys: vec!["workflow:order_read".to_string()],
                duplicate_input_key: None,
                reason_code: None,
            }],
            items: vec![ApplicationModelItemSeed {
                item_key: "workflow:order_read".to_string(),
                item_kind: "workflow".to_string(),
                truth_state: ApplicationModelTruthStateRow::Observed,
                source_input_keys: vec![source.input_key.clone()],
                referenced_item_keys: Vec::new(),
                payload: json!({"method": "GET", "path": "/orders/{id}"}),
                evidence: vec![ApplicationModelItemEvidenceSeed {
                    evidence_id,
                    role: ApplicationModelEvidenceRoleRow::Observation,
                }],
            }],
        };
        match self.mode {
            ProducerMode::Valid | ProducerMode::Empty => {}
            ProducerMode::DuplicateDecision => {
                draft.decisions.push(draft.decisions[0].clone());
            }
            ProducerMode::ObservedWithoutEvidence => {
                draft.items[0].evidence.clear();
            }
            ProducerMode::BadInternalReference => {
                draft.items[0].referenced_item_keys = vec!["workflow:missing".to_string()];
            }
            ProducerMode::MismatchedOrganization => {
                draft.structured_model["organization_id"] = json!(Uuid::new_v4());
            }
        }
        Ok(draft)
    }
}

#[tokio::test]
#[serial]
async fn runtime_passes_model_and_exact_replay_does_not_call_producer_twice() {
    let fixture = RuntimeFixture::start("model_pass", true).await;
    let producer = CountingProducer::valid();

    let first =
        run_application_understanding_unit(fixture.db.pool(), &fixture.command(), &producer)
            .await
            .expect("run model authority to Gate PASS");
    let ApplicationUnderstandingRuntimeOutcome::Passed(first) = first else {
        panic!("expected model runtime PASS");
    };
    let replay =
        run_application_understanding_unit(fixture.db.pool(), &fixture.command(), &producer)
            .await
            .expect("replay exact model authority");
    let ApplicationUnderstandingRuntimeOutcome::Passed(replay) = replay else {
        panic!("expected exact runtime replay PASS, got {replay:?}");
    };

    assert_eq!(producer.calls(), 1);
    assert_eq!(first.manifest_id, replay.manifest_id);
    assert_eq!(first.revision_id, replay.revision_id);
    assert_eq!(first.final_seal.unit.id, replay.final_seal.unit.id);
    assert_eq!(
        first.final_seal.unit.row_version,
        replay.final_seal.unit.row_version
    );
    assert_eq!(first.final_seal.worker.id, replay.final_seal.worker.id);
    assert_eq!(
        first.final_seal.worker.checkpoint_version,
        replay.final_seal.worker.checkpoint_version
    );
    assert_eq!(first.final_seal.handoff, replay.final_seal.handoff);
    assert!(!first.replayed);
    assert!(replay.replayed);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM tool_calls WHERE operation_id=$1 AND name='submit_stage_deliverable'",
        )
        .bind(fixture.fence.operation_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count runtime submit receipts"),
        2,
        "one source receipt plus one Application Understanding control receipt",
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_model_current_revisions WHERE manifest_id=$1",
        )
        .bind(first.manifest_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count current model rows"),
        1,
    );
    assert_eq!(
        sqlx::query_as::<_, (i64, i64)>(
            r#"SELECT count(*),count(*) FILTER (
                       WHERE status::text='finished'
                         AND name='submit_stage_deliverable'
                         AND operation_id=$1
                         AND stage_execution_id=$2
                         AND stage_run_unit_id=$3
                         AND worker_run_id=$4
                         AND organization_id IS NOT NULL
                         AND attempt_epoch=$5
                         AND lease_token=$6
                   )
                  FROM tool_calls WHERE stage_run_unit_id=$3"#,
        )
        .bind(fixture.fence.operation_id)
        .bind(fixture.fence.stage_execution_id)
        .bind(fixture.fence.stage_run_unit_id)
        .bind(fixture.fence.worker_run_id)
        .bind(fixture.fence.attempt_epoch)
        .bind(fixture.fence.lease_token)
        .fetch_one(fixture.db.pool())
        .await
        .expect("audit exact runtime control receipt"),
        (1, 1),
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            r#"SELECT count(*) FROM tool_calls
                WHERE stage_run_unit_id=$1
                  AND name<>'submit_stage_deliverable'"#,
        )
        .bind(fixture.fence.stage_run_unit_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count forbidden runtime tools"),
        0,
    );
    for table in [
        "targets",
        "target_assets",
        "findings",
        "attack_candidates",
        "candidate_attempts",
    ] {
        let count = sqlx::query_scalar::<_, i64>(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(fixture.db.pool())
            .await
            .expect("count forbidden business side effects");
        assert_eq!(count, 0, "{table} must remain untouched by modeling");
    }
}

#[tokio::test]
#[serial]
async fn runtime_resumes_proposed_revision_after_gate_hold_without_duplicate_submission() {
    let fixture = RuntimeFixture::start("resume_proposed", true).await;
    let producer = CountingProducer::valid();
    let mut stale_command = fixture.command();
    stale_command.expected_unit_row_version = 99;

    let first = run_application_understanding_unit(fixture.db.pool(), &stale_command, &producer)
        .await
        .expect("stale Unit CAS becomes a typed Gate HOLD after proposal persistence");
    let ApplicationUnderstandingRuntimeOutcome::Blocked(first_gate) = first else {
        panic!("expected first finalizer attempt to HOLD");
    };
    assert_eq!(
        first_gate.disposition,
        ApplicationModelGateDisposition::Hold
    );
    assert_eq!(producer.calls(), 1);

    let resumed =
        run_application_understanding_unit(fixture.db.pool(), &fixture.command(), &producer)
            .await
            .expect("resume persisted proposal through the atomic Gate");
    let ApplicationUnderstandingRuntimeOutcome::Passed(_) = resumed else {
        panic!("expected persisted proposal resume PASS, got {resumed:?}");
    };

    assert_eq!(
        producer.calls(),
        1,
        "resume must reuse the persisted proposal"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM stage_deliverable_submissions WHERE stage_run_unit_id=$1",
        )
        .bind(fixture.fence.stage_run_unit_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count resumed submissions"),
        1,
    );
}

#[tokio::test]
#[serial]
async fn runtime_reauthorizes_proposed_revision_after_worker_retry_without_second_completion() {
    let fixture = RuntimeFixture::start("retry_proposed", true).await;
    let producer = CountingProducer::valid();
    let mut stale_command = fixture.command();
    stale_command.expected_unit_row_version = 99;

    let first = run_application_understanding_unit(fixture.db.pool(), &stale_command, &producer)
        .await
        .expect("persist proposal before the simulated finalizer interruption");
    assert!(matches!(
        first,
        ApplicationUnderstandingRuntimeOutcome::Blocked(_)
    ));
    assert_eq!(producer.calls(), 1);

    let retry_lease_token = Uuid::new_v4();
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET attempt_epoch=attempt_epoch+1,lease_token=$2,
                  lease_owner='runtime-retry-fixture',
                  lease_acquired_at=NOW(),lease_expires_at=NOW()+INTERVAL '5 minutes',
                  heartbeat_at=NOW(),updated_at=NOW()
            WHERE id=$1 AND status='running' AND attempt_epoch=0"#,
    )
    .bind(fixture.fence.worker_run_id)
    .bind(retry_lease_token)
    .execute(fixture.db.pool())
    .await
    .expect("advance the Application Model worker to a new exact retry fence");
    let mut retry_command = fixture.command();
    retry_command.fence.attempt_epoch = 1;
    retry_command.fence.lease_token = retry_lease_token;

    let resumed = run_application_understanding_unit(fixture.db.pool(), &retry_command, &producer)
        .await
        .expect("reauthorize the persisted proposal under the current retry fence");
    let ApplicationUnderstandingRuntimeOutcome::Passed(pass) = resumed else {
        panic!("expected cross-attempt proposal recovery PASS, got {resumed:?}");
    };

    assert_eq!(
        producer.calls(),
        1,
        "retry must reconstruct the persisted proposal without another completion"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM stage_deliverable_submissions WHERE stage_run_unit_id=$1",
        )
        .bind(fixture.fence.stage_run_unit_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count retry submissions"),
        2,
        "the retry writes one fresh fence-bound receipt while retaining the old receipt"
    );
    let current_source = sqlx::query_as::<_, (Uuid, Option<i64>)>(
        r#"SELECT revision.source_submission_id,submission.attempt_epoch
             FROM application_model_revisions AS revision
             JOIN stage_deliverable_submissions AS submission
               ON submission.id=revision.source_submission_id
            WHERE revision.id=$1"#,
    )
    .bind(pass.revision_id.expect("model revision"))
    .fetch_one(fixture.db.pool())
    .await
    .expect("load reauthorized proposal receipt");
    assert_eq!(
        current_source.0,
        pass.final_seal.handoff.deliverable_submission_id
    );
    assert_eq!(current_source.1, Some(1));
}

#[tokio::test]
#[serial]
async fn runtime_prefers_persisted_proposal_over_multiple_historical_receipts() {
    let fixture = RuntimeFixture::start("retry_proposed_history", true).await;
    let producer = CountingProducer::valid();
    let mut command = fixture.command();
    command.expected_unit_row_version = 99;

    let first = run_application_understanding_unit(fixture.db.pool(), &command, &producer)
        .await
        .expect("persist the initial proposal before closeout interruption");
    assert!(matches!(
        first,
        ApplicationUnderstandingRuntimeOutcome::Blocked(_)
    ));

    for attempt_epoch in 1..=2 {
        let lease_token = Uuid::new_v4();
        sqlx::query(
            r#"UPDATE stage_worker_runs
                  SET attempt_epoch=$2,lease_token=$3,
                      lease_owner='proposal-history-fixture',
                      lease_acquired_at=NOW(),lease_expires_at=NOW()+INTERVAL '5 minutes',
                      heartbeat_at=NOW(),updated_at=NOW()
                WHERE id=$1 AND status='running'"#,
        )
        .bind(fixture.fence.worker_run_id)
        .bind(attempt_epoch)
        .bind(lease_token)
        .execute(fixture.db.pool())
        .await
        .expect("advance the interrupted finalizer fence");
        command.fence.attempt_epoch = attempt_epoch;
        command.fence.lease_token = lease_token;
        let retry = run_application_understanding_unit(fixture.db.pool(), &command, &producer)
            .await
            .expect("reauthorize the proposal while closeout remains interrupted");
        assert!(matches!(
            retry,
            ApplicationUnderstandingRuntimeOutcome::Blocked(_)
        ));
    }

    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM stage_deliverable_submissions WHERE stage_run_unit_id=$1",
        )
        .bind(fixture.fence.stage_run_unit_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count retained closeout receipts"),
        3,
    );

    let final_lease_token = Uuid::new_v4();
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET attempt_epoch=3,lease_token=$2,
                  lease_owner='proposal-history-final-fixture',
                  lease_acquired_at=NOW(),lease_expires_at=NOW()+INTERVAL '5 minutes',
                  heartbeat_at=NOW(),updated_at=NOW()
            WHERE id=$1 AND status='running'"#,
    )
    .bind(fixture.fence.worker_run_id)
    .bind(final_lease_token)
    .execute(fixture.db.pool())
    .await
    .expect("advance to the successful closeout fence");
    let mut final_command = fixture.command();
    final_command.fence.attempt_epoch = 3;
    final_command.fence.lease_token = final_lease_token;

    let resumed = run_application_understanding_unit(fixture.db.pool(), &final_command, &producer)
        .await
        .expect("prefer the durable proposal over unreferenced historical receipts");
    assert!(matches!(
        resumed,
        ApplicationUnderstandingRuntimeOutcome::Passed(_)
    ));
    assert_eq!(
        producer.calls(),
        1,
        "closeout retries must never repeat provider completion",
    );
}

#[tokio::test]
#[serial]
async fn runtime_reconstructs_completed_standalone_submission_without_second_completion() {
    let fixture = RuntimeFixture::start("standalone_completed", true).await;
    let standalone_submission_id = fixture.insert_standalone_model_submission(true).await;
    let producer = CountingProducer::valid();

    let outcome =
        run_application_understanding_unit(fixture.db.pool(), &fixture.command(), &producer)
            .await
            .expect("recover completed standalone submission from canonical payload");
    let ApplicationUnderstandingRuntimeOutcome::Passed(pass) = outcome else {
        panic!("completed standalone submission must recover to PASS, got {outcome:?}");
    };

    assert_eq!(producer.calls(), 0, "provider completion must not repeat");
    assert_eq!(
        pass.final_seal.handoff.deliverable_submission_id,
        standalone_submission_id
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM stage_deliverable_submissions WHERE stage_run_unit_id=$1",
        )
        .bind(fixture.fence.stage_run_unit_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count recovered standalone submissions"),
        1,
    );
    assert_eq!(
        sqlx::query_scalar::<_, Uuid>(
            "SELECT source_submission_id FROM application_model_revisions WHERE manifest_id=$1",
        )
        .bind(pass.manifest_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("read reconstructed revision source"),
        standalone_submission_id,
    );
}

#[tokio::test]
#[serial]
async fn runtime_reauthorizes_completed_standalone_submission_after_worker_retry() {
    let fixture = RuntimeFixture::start("standalone_retry", true).await;
    sqlx::query("UPDATE stage_run_units SET generation=1 WHERE id=$1")
        .bind(fixture.fence.stage_run_unit_id)
        .execute(fixture.db.pool())
        .await
        .expect("model a replacement Unit generation independent from Worker attempts");
    let prior_submission_id = fixture.insert_standalone_model_submission(true).await;
    let producer = CountingProducer::valid();
    let retry_lease_token = Uuid::new_v4();
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET attempt_epoch=attempt_epoch+1,lease_token=$2,
                  lease_owner='standalone-retry-fixture',
                  lease_acquired_at=NOW(),lease_expires_at=NOW()+INTERVAL '5 minutes',
                  heartbeat_at=NOW(),updated_at=NOW()
            WHERE id=$1 AND status='running' AND attempt_epoch=0"#,
    )
    .bind(fixture.fence.worker_run_id)
    .bind(retry_lease_token)
    .execute(fixture.db.pool())
    .await
    .expect("advance standalone receipt owner to a new exact retry fence");
    let mut retry_command = fixture.command();
    retry_command.fence.attempt_epoch = 1;
    retry_command.fence.lease_token = retry_lease_token;

    let outcome = run_application_understanding_unit(fixture.db.pool(), &retry_command, &producer)
        .await
        .expect("reauthorize completed standalone receipt under the current retry fence");
    let ApplicationUnderstandingRuntimeOutcome::Passed(pass) = outcome else {
        panic!("cross-attempt standalone recovery must PASS, got {outcome:?}");
    };

    assert_eq!(producer.calls(), 0, "provider completion must not repeat");
    let current_submission_id = pass.final_seal.handoff.deliverable_submission_id;
    assert_ne!(current_submission_id, prior_submission_id);
    let receipt_authority: (i64, Option<i64>) = sqlx::query_as(
        r#"SELECT count(*),max(attempt_epoch)
             FROM stage_deliverable_submissions
            WHERE stage_run_unit_id=$1 AND worker_run_id=$2"#,
    )
    .bind(fixture.fence.stage_run_unit_id)
    .bind(fixture.fence.worker_run_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("read old and reauthorized standalone receipts");
    assert_eq!(receipt_authority, (2, Some(1)));
    assert_eq!(
        sqlx::query_scalar::<_, Uuid>(
            "SELECT source_submission_id FROM application_model_revisions WHERE manifest_id=$1",
        )
        .bind(pass.manifest_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("read proposal source after standalone reauthorization"),
        current_submission_id,
    );
}

#[tokio::test]
#[serial]
async fn runtime_holds_nonterminal_standalone_submission_without_second_completion() {
    let fixture = RuntimeFixture::start("standalone_nonterminal", true).await;
    let standalone_submission_id = fixture.insert_standalone_model_submission(false).await;
    let producer = CountingProducer::valid();

    let outcome =
        run_application_understanding_unit(fixture.db.pool(), &fixture.command(), &producer)
            .await
            .expect("outcome-unknown standalone submission returns deterministic HOLD");
    let ApplicationUnderstandingRuntimeOutcome::Blocked(block) = outcome else {
        panic!("nonterminal standalone submission must HOLD");
    };

    assert_eq!(producer.calls(), 0, "provider completion must not repeat");
    assert_eq!(block.code, ApplicationModelGateCode::ProducerBarrierOpen);
    assert_eq!(block.disposition, ApplicationModelGateDisposition::Hold);
    assert!(block
        .refs
        .contains(&format!("standalone_submission:{standalone_submission_id}")));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_model_revisions WHERE manifest_id IN (SELECT id FROM application_model_manifests WHERE stage_run_unit_id=$1)",
        )
        .bind(fixture.fence.stage_run_unit_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count outcome-unknown revisions"),
        0,
    );
}

#[tokio::test]
#[serial]
async fn runtime_passes_terminal_no_input_without_calling_producer() {
    let fixture = RuntimeFixture::start("terminal_pass", false).await;

    let outcome =
        run_application_understanding_unit(fixture.db.pool(), &fixture.command(), &NeverProducer)
            .await
            .expect("run true zero-input authority to Gate PASS");
    let ApplicationUnderstandingRuntimeOutcome::Passed(pass) = outcome else {
        panic!("expected terminal-no-input runtime PASS");
    };
    let replay =
        run_application_understanding_unit(fixture.db.pool(), &fixture.command(), &NeverProducer)
            .await
            .expect("replay true zero-input authority");
    let ApplicationUnderstandingRuntimeOutcome::Passed(replay) = replay else {
        panic!("expected terminal-no-input replay PASS");
    };

    assert_eq!(pass.revision_id, None);
    assert_eq!(pass.manifest_id, replay.manifest_id);
    assert!(replay.replayed);
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT authority_kind FROM application_model_current_revisions WHERE manifest_id=$1",
        )
        .bind(pass.manifest_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("read terminal current row"),
        "terminal_no_input",
    );
}

#[tokio::test]
#[serial]
async fn runtime_reworks_invalid_producer_draft_before_submission() {
    let fixture = RuntimeFixture::start("invalid_draft", true).await;
    let producer = CountingProducer::invalid();

    let outcome =
        run_application_understanding_unit(fixture.db.pool(), &fixture.command(), &producer)
            .await
            .expect("invalid draft is a typed Gate outcome");
    let ApplicationUnderstandingRuntimeOutcome::Blocked(gate) = outcome else {
        panic!("expected invalid draft REWORK");
    };

    assert_eq!(producer.calls(), 1);
    assert_eq!(gate.code, ApplicationModelGateCode::SchemaInvalid);
    assert_eq!(gate.disposition, ApplicationModelGateDisposition::Rework);
    for table in [
        "application_model_revisions",
        "application_model_current_revisions",
    ] {
        let count = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT count(*) FROM {table} WHERE manifest_id IN (SELECT id FROM application_model_manifests WHERE stage_run_unit_id=$1)"
        ))
        .bind(fixture.fence.stage_run_unit_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count invalid-draft publication rows");
        assert_eq!(count, 0, "{table} must remain empty on REWORK");
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM tool_calls WHERE stage_run_unit_id=$1",)
            .bind(fixture.fence.stage_run_unit_id)
            .fetch_one(fixture.db.pool())
            .await
            .expect("count invalid-draft receipts"),
        0,
    );
}

#[tokio::test]
#[serial]
async fn runtime_reworks_mismatched_producer_organization_before_submission() {
    let fixture = RuntimeFixture::start("mismatched_model_organization", true).await;
    let producer = CountingProducer::with_mode(ProducerMode::MismatchedOrganization);

    let outcome =
        run_application_understanding_unit(fixture.db.pool(), &fixture.command(), &producer)
            .await
            .expect("mismatched model organization is a typed Gate outcome");
    let ApplicationUnderstandingRuntimeOutcome::Blocked(gate) = outcome else {
        panic!("expected organization identity REWORK");
    };

    assert_eq!(producer.calls(), 1);
    assert_eq!(gate.code, ApplicationModelGateCode::IdentityMismatch);
    assert_eq!(gate.disposition, ApplicationModelGateDisposition::Rework);
    assert_eq!(gate.refs, vec!["proposed_model_organization_mismatch"]);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM tool_calls WHERE stage_run_unit_id=$1",)
            .bind(fixture.fence.stage_run_unit_id)
            .fetch_one(fixture.db.pool())
            .await
            .expect("count mismatched-identity tool calls"),
        0,
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM stage_deliverable_submissions WHERE stage_run_unit_id=$1",
        )
        .bind(fixture.fence.stage_run_unit_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count mismatched-identity submissions"),
        0,
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_model_revisions WHERE manifest_id IN (SELECT id FROM application_model_manifests WHERE stage_run_unit_id=$1)",
        )
        .bind(fixture.fence.stage_run_unit_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count mismatched-identity revisions"),
        0,
    );
}

#[tokio::test]
#[serial]
async fn runtime_reworks_duplicate_evidenceless_and_bad_reference_drafts_before_submission() {
    for (label, mode) in [
        ("duplicate_decision", ProducerMode::DuplicateDecision),
        (
            "observed_no_evidence",
            ProducerMode::ObservedWithoutEvidence,
        ),
        ("bad_internal_ref", ProducerMode::BadInternalReference),
    ] {
        let fixture = RuntimeFixture::start(label, true).await;
        let producer = CountingProducer::with_mode(mode);

        let outcome =
            run_application_understanding_unit(fixture.db.pool(), &fixture.command(), &producer)
                .await
                .expect("invalid draft is a typed Gate outcome");
        let ApplicationUnderstandingRuntimeOutcome::Blocked(gate) = outcome else {
            panic!("expected invalid draft REWORK for {label}");
        };

        assert_eq!(producer.calls(), 1, "producer call count for {label}");
        assert_eq!(gate.disposition, ApplicationModelGateDisposition::Rework);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM tool_calls WHERE stage_run_unit_id=$1",
            )
            .bind(fixture.fence.stage_run_unit_id)
            .fetch_one(fixture.db.pool())
            .await
            .expect("count invalid-draft receipts"),
            0,
            "{label} must not create a submission receipt",
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM application_model_revisions WHERE manifest_id IN (SELECT id FROM application_model_manifests WHERE stage_run_unit_id=$1)",
            )
            .bind(fixture.fence.stage_run_unit_id)
            .fetch_one(fixture.db.pool())
            .await
            .expect("count invalid-draft revisions"),
            0,
            "{label} must not persist a proposal revision",
        );
    }
}

#[tokio::test]
#[serial]
async fn runtime_holds_stale_fence_before_manifest_or_producer() {
    let fixture = RuntimeFixture::start("stale_fence", true).await;
    let producer = CountingProducer::valid();
    let mut command = fixture.command();
    command.fence.lease_token = Uuid::new_v4();

    let outcome = run_application_understanding_unit(fixture.db.pool(), &command, &producer)
        .await
        .expect("stale fence is a typed Gate outcome");
    let ApplicationUnderstandingRuntimeOutcome::Blocked(gate) = outcome else {
        panic!("expected stale fence HOLD");
    };

    assert_eq!(producer.calls(), 0);
    assert_eq!(gate.code, ApplicationModelGateCode::IdentityMismatch);
    assert_eq!(gate.disposition, ApplicationModelGateDisposition::Hold);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_model_manifests WHERE stage_run_unit_id=$1",
        )
        .bind(fixture.fence.stage_run_unit_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count stale-fence manifests"),
        0,
    );
}

#[tokio::test]
#[serial]
async fn runtime_holds_active_tool_before_manifest_or_producer() {
    let fixture = RuntimeFixture::start("active_tool", true).await;
    let producer = CountingProducer::valid();
    let tool_call_record_id = tool_calls::record_tracked_start(
        fixture.db.pool(),
        "runtime-active-tool",
        fixture.session_id,
        Some(fixture.fence.operation_id),
        None,
        "update_plan",
        &json!({"server_owned": true}),
        Some(&tool_calls::RuntimeToolIdentity {
            operation_id: fixture.fence.operation_id,
            stage_execution_id: fixture.fence.stage_execution_id,
            stage_run_unit_id: Some(fixture.fence.stage_run_unit_id),
            worker_run_id: Some(fixture.fence.worker_run_id),
            organization_id: Some(
                sqlx::query_scalar::<_, Uuid>(
                    "SELECT organization_id FROM stage_run_units WHERE id=$1",
                )
                .bind(fixture.fence.stage_run_unit_id)
                .fetch_one(fixture.db.pool())
                .await
                .expect("read active-tool organization"),
            ),
            attempt_epoch: Some(fixture.fence.attempt_epoch),
            lease_token: Some(fixture.fence.lease_token),
        }),
    )
    .await
    .expect("record active control tool");
    runtime_memory_tx::begin_worker_tool(fixture.db.pool(), &fixture.fence, tool_call_record_id)
        .await
        .expect("bind active control tool to worker");

    let outcome =
        run_application_understanding_unit(fixture.db.pool(), &fixture.command(), &producer)
            .await
            .expect("active tool is a typed Gate outcome");
    let ApplicationUnderstandingRuntimeOutcome::Blocked(gate) = outcome else {
        panic!("expected active-tool HOLD");
    };

    assert_eq!(producer.calls(), 0);
    assert_eq!(gate.code, ApplicationModelGateCode::ProducerBarrierOpen);
    assert_eq!(gate.disposition, ApplicationModelGateDisposition::Hold);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_model_manifests WHERE stage_run_unit_id=$1",
        )
        .bind(fixture.fence.stage_run_unit_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count active-tool manifests"),
        0,
    );
}

#[tokio::test]
#[serial]
async fn runtime_holds_running_sibling_before_calling_producer() {
    let fixture = RuntimeFixture::start("running_sibling", true).await;
    fixture.insert_running_sibling().await;
    let producer = CountingProducer::valid();

    let outcome =
        run_application_understanding_unit(fixture.db.pool(), &fixture.command(), &producer)
            .await
            .expect("running sibling is a typed Gate outcome");
    let ApplicationUnderstandingRuntimeOutcome::Blocked(gate) = outcome else {
        panic!("expected producer barrier HOLD");
    };

    assert_eq!(producer.calls(), 0);
    assert_eq!(gate.code, ApplicationModelGateCode::ProducerBarrierOpen);
    assert_eq!(gate.disposition, ApplicationModelGateDisposition::Hold);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_model_current_revisions WHERE manifest_id IN (SELECT id FROM application_model_manifests WHERE stage_run_unit_id=$1)",
        )
        .bind(fixture.fence.stage_run_unit_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count sibling-blocked current rows"),
        0,
    );
}

#[tokio::test]
#[serial]
async fn runtime_holds_pending_work_item_before_calling_producer() {
    let fixture = RuntimeFixture::start_with_pending_work_item("pending_work_item", true).await;
    let producer = CountingProducer::valid();

    let outcome =
        run_application_understanding_unit(fixture.db.pool(), &fixture.command(), &producer)
            .await
            .expect("pending WorkItem is a typed Gate outcome");
    let ApplicationUnderstandingRuntimeOutcome::Blocked(gate) = outcome else {
        panic!("expected pending WorkItem HOLD");
    };

    assert_eq!(producer.calls(), 0);
    assert_eq!(gate.code, ApplicationModelGateCode::ProducerBarrierOpen);
    assert_eq!(gate.disposition, ApplicationModelGateDisposition::Hold);
    assert!(gate
        .refs
        .iter()
        .any(|value| value.starts_with("work_item:")));
}

#[cfg(any())]
#[tokio::test]
#[serial]
async fn candidate_application_model_authority_binds_current_final_model_and_exact_replays() {
    let fixture = RuntimeFixture::start_v2("candidate-model-authority", true).await;
    let producer = CountingProducer::valid();
    let outcome =
        run_application_understanding_unit(fixture.db.pool(), &fixture.command(), &producer)
            .await
            .expect("run Application Understanding before Candidate binding");
    assert!(
        matches!(outcome, ApplicationUnderstandingRuntimeOutcome::Passed(_)),
        "Application Understanding must pass before Candidate binding, got {outcome:?}"
    );
    let (wave_run_id, wave_unit_id, target_id) = fixture
        .freeze_candidate_manifest_for_target("https://runtime.example.test/orders/{id}")
        .await;

    let first = attack_candidate_work_items::bind_frozen_manifest_to_current_application_model(
        fixture.db.pool(),
        fixture.fence.operation_id,
        fixture.scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        fixture.organization_id,
    )
    .await
    .expect("bind exact current Application Model");
    let replay = attack_candidate_work_items::bind_frozen_manifest_to_current_application_model(
        fixture.db.pool(),
        fixture.fence.operation_id,
        fixture.scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        fixture.organization_id,
    )
    .await
    .expect("exact Candidate authority replay");

    assert_eq!(first.authority, replay.authority);
    assert_eq!(first.application_model, replay.application_model);
    assert_eq!(first.manifest, replay.manifest);
    assert_eq!(
        first.authority.application_model_revision_id,
        first.application_model.revision_id
    );
    assert_eq!(
        first.authority.source_vuln_handoff_id,
        fixture.source.expect("source fixture").handoff_id
    );
    assert_eq!(first.application_model.items.len(), 1);
    assert!(first.authority.input_authority_hash.starts_with("sha256:"));
    assert!(!attack_candidate_work_items::frozen_manifest_owns_target(
        fixture.db.pool(),
        fixture.fence.operation_id,
        fixture.organization_id,
        target_id,
    )
    .await
    .expect("reject frozen target before Candidate stage"));
    let authority_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM attack_wave_application_model_authorities \
         WHERE wave_unit_id=$1",
    )
    .bind(wave_unit_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("count Candidate Application Model authorities");
    assert_eq!(authority_count, 1);

    let work_item = &first.manifest.items[0];
    let work_item_id = work_item.work_item.id;
    let evidence_id = fixture.source.expect("source fixture").evidence_id;
    let candidate_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{}:{work_item_id}:candidate", fixture.fence.operation_id).as_bytes(),
    );
    let execution_plan = json!({
        "schema_version": "candidate-plan-v1",
        "classifier_version": "candidate-classifier-v1",
        "candidate_id": candidate_id,
        "target_identity_hash": work_item.work_item.target_identity_hash,
        "actions": [{
            "ordinal": 0,
            "capability_id": "verify.controlled_http_probe",
            "action_kind": "http_request",
            "canonical_args": {
                "background": false,
                "method": "GET",
                "url": "https://runtime.example.test/orders/1"
            }
        }, {
            "ordinal": 1,
            "capability_id": "verify.controlled_http_probe",
            "action_kind": "http_request",
            "canonical_args": {
                "background": false,
                "method": "GET",
                "url": "https://runtime.example.test/orders/2"
            }
        }],
        "budget": {"max_actions": 2, "max_requests": 2, "max_runtime_ms": 1000},
        "foreground_only": true,
    });
    let candidate_plan_hash =
        canonical_execution_plan_hash(&execution_plan).expect("hash Candidate execution plan");
    let authority = CandidateApplicationModelAcceptance {
        manifest_id: first.authority.application_model_manifest_id,
        revision_id: first.authority.application_model_revision_id,
        manifest_hash: first.authority.application_model_manifest_hash.clone(),
        model_hash: first.authority.application_model_model_hash.clone(),
        replay_material_hash: first
            .authority
            .application_model_replay_material_hash
            .clone(),
        stage_handoff_id: first.authority.application_model_handoff_id,
        stage_execution_id: first.authority.application_model_stage_execution_id,
        stage_run_unit_id: first.authority.application_model_stage_run_unit_id,
        deliverable_submission_id: first.authority.application_model_deliverable_submission_id,
        gate_decision_hash: first.authority.application_model_gate_decision_hash.clone(),
        input_authority_hash: first.authority.input_authority_hash.clone(),
    };
    let acceptance = CandidateAcceptanceInput {
        wave_run_id,
        wave_unit_id,
        manifest_hash: first.authority.candidate_manifest_hash.clone(),
        application_model_authority: Some(authority),
        expected_work_item_ids: vec![work_item_id],
        candidates: vec![AcceptedCandidateDraft {
            candidate_id,
            work_item_id,
            hypothesis: "orders endpoint may accept an injectable identifier".to_string(),
            technique: Some("WSTG-INPV-05".to_string()),
            rationale: "exact frozen observation interpreted with the bound Application Model"
                .to_string(),
            prior_refs: vec![format!("audit:{evidence_id}")],
            suggested_approach: "bounded_sql_injection_probe".to_string(),
            priority: "high".to_string(),
            execution_plan,
            action_recipes: Vec::new(),
            candidate_plan_hash,
            risk_class: "exploit".to_string(),
            evidence_ids: vec![evidence_id],
        }],
        no_candidate_decisions: Vec::new(),
    };
    let runtime = fixture.start_candidate_runtime_unit().await;
    assert!(attack_candidate_work_items::frozen_manifest_owns_target(
        fixture.db.pool(),
        fixture.fence.operation_id,
        fixture.organization_id,
        target_id,
    )
    .await
    .expect("authorize exact frozen Candidate target"));
    assert!(!attack_candidate_work_items::frozen_manifest_owns_target(
        fixture.db.pool(),
        fixture.fence.operation_id,
        fixture.organization_id,
        Uuid::new_v4(),
    )
    .await
    .expect("reject target outside frozen Candidate manifest"));

    let mut drifted_acceptance = acceptance.clone();
    drifted_acceptance
        .application_model_authority
        .as_mut()
        .expect("strict authority")
        .input_authority_hash = format!("sha256:{}", "2".repeat(64));
    let drifted = candidate_final_seal_input(&fixture, runtime, drifted_acceptance);
    assert!(
        runtime_memory_tx::finalize_unit_pass(fixture.db.pool(), &drifted)
            .await
            .is_err(),
        "drifted Candidate Application Model authority must fail closed"
    );
    for (table, predicate) in [
        ("stage_handoffs", "source_stage_run_unit_id"),
        ("attack_candidates", "candidate_id"),
        ("attack_candidate_application_model_refs", "candidate_id"),
    ] {
        let value = if predicate == "source_stage_run_unit_id" {
            runtime.stage_run_unit_id
        } else {
            candidate_id
        };
        let count = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT count(*) FROM {table} WHERE {predicate}=$1"
        ))
        .bind(value)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count rolled-back Candidate final-seal rows");
        assert_eq!(count, 0, "{table} must roll back on authority drift");
    }
    let still_running: String =
        sqlx::query_scalar("SELECT status FROM stage_run_units WHERE id=$1")
            .bind(runtime.stage_run_unit_id)
            .fetch_one(fixture.db.pool())
            .await
            .expect("read Candidate Unit after rejected seal");
    assert_eq!(still_running, "running");

    let final_input = candidate_final_seal_input(&fixture, runtime, acceptance);
    let sealed = runtime_memory_tx::finalize_unit_pass(fixture.db.pool(), &final_input)
        .await
        .expect("atomically seal Candidate with Application Model provenance");
    assert!(!sealed.replayed);
    assert_eq!(sealed.unit.status, "passed");
    assert_eq!(sealed.handoff.from_stage_kind, "attack_candidate");
    let persisted_ref: (Uuid, String) = sqlx::query_as(
        r#"SELECT application_model_revision_id,input_authority_hash
             FROM attack_candidate_application_model_refs WHERE candidate_id=$1"#,
    )
    .bind(candidate_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("read Candidate Application Model provenance");
    assert_eq!(
        persisted_ref.0,
        first.authority.application_model_revision_id
    );
    assert_eq!(persisted_ref.1, first.authority.input_authority_hash);
    let candidate_authority_hash: String = sqlx::query_scalar(
        "SELECT candidate_authority_hash FROM attack_candidates WHERE candidate_id=$1",
    )
    .bind(candidate_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("read server-owned Candidate authority hash");
    assert!(candidate_authority_hash.starts_with("sha256:"));
    assert_eq!(candidate_authority_hash.len(), 71);

    let exact_replay = runtime_memory_tx::finalize_unit_pass(fixture.db.pool(), &final_input)
        .await
        .expect("exact Candidate final-seal replay");
    assert!(exact_replay.replayed);
    assert_eq!(exact_replay.handoff.id, sealed.handoff.id);

    let candidate_row_version: i64 =
        sqlx::query_scalar("SELECT row_version FROM attack_candidates WHERE candidate_id=$1")
            .bind(candidate_id)
            .fetch_one(fixture.db.pool())
            .await
            .expect("read Candidate review version");
    let review = review_wave_candidates(
        fixture.db.pool(),
        ReviewCandidateBatch {
            operation_id: fixture.fence.operation_id,
            wave_run_id,
            decisions: vec![CandidateReviewDecision {
                candidate_id,
                expected_candidate_plan_hash: final_input
                    .candidate_acceptance
                    .as_ref()
                    .expect("Candidate acceptance")
                    .candidates[0]
                    .candidate_plan_hash
                    .clone(),
                expected_candidate_row_version: candidate_row_version,
                approve: true,
                start_before: Some(chrono::Utc::now() + chrono::Duration::minutes(5)),
                expires_at: Some(chrono::Utc::now() + chrono::Duration::minutes(10)),
            }],
        },
    )
    .await
    .expect("approve exact current Candidate authority");
    assert_eq!(review.approvals.len(), 1);
    assert_eq!(
        review.approvals[0].candidate_authority_hash.as_deref(),
        Some(candidate_authority_hash.as_str())
    );

    let verification_stage_execution_id = Uuid::new_v4();
    let verification_stage_run_unit_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status) \
         VALUES($1,$2,'verification','started')",
    )
    .bind(verification_stage_execution_id)
    .bind(fixture.fence.operation_id)
    .execute(fixture.db.pool())
    .await
    .expect("insert Verification StageRun");
    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,organization_id,
               stage_kind,generation,specialist,status)
           VALUES($1,$2,$3,$4,$5,'verification',0,'candidate_verifier','queued')"#,
    )
    .bind(verification_stage_run_unit_id)
    .bind(fixture.fence.operation_id)
    .bind(verification_stage_execution_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.organization_id)
    .execute(fixture.db.pool())
    .await
    .expect("insert Verification StageRunUnit");
    let claimed = claim_next_candidate_attempt(
        fixture.db.pool(),
        CandidateClaimQuery {
            operation_id: fixture.fence.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id,
            wave_unit_id,
            organization_id: fixture.organization_id,
            verification_stage_execution_id,
            verification_stage_run_unit_id,
            lease_owner: "candidate-authority-fixture".to_string(),
            lease_seconds: 300,
        },
    )
    .await
    .expect("claim exact current Candidate Attempt")
    .expect("one Candidate must be claimable");
    assert_eq!(
        claimed.attempt.candidate_authority_hash.as_deref(),
        Some(candidate_authority_hash.as_str())
    );
    let workspace_path_sha256: String = sqlx::query_scalar(
        r#"SELECT project.path_sha256
             FROM operation_state operation
             JOIN project_scopes project
               ON project.project_scope_id=operation.project_scope_id
            WHERE operation.operation_id=$1"#,
    )
    .bind(fixture.fence.operation_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("read workspace path authority");
    let lease_token = claimed.worker.lease_token.expect("claimed lease token");
    let begin = |action_ordinal| BeginCandidateAction {
        operation_id: fixture.fence.operation_id,
        stage_execution_id: verification_stage_execution_id,
        stage_run_unit_id: verification_stage_run_unit_id,
        organization_id: fixture.organization_id,
        worker_run_id: claimed.worker.id,
        lease_token,
        attempt_epoch: claimed.worker.attempt_epoch,
        candidate_id,
        approval_id: claimed.attempt.approval_id,
        attempt_id: claimed.attempt.id,
        candidate_plan_hash: claimed.attempt.candidate_plan_hash.clone(),
        workspace_path_sha256: workspace_path_sha256.clone(),
        action_ordinal,
    };
    let first_action = begin_candidate_action(fixture.db.pool(), begin(0))
        .await
        .expect("begin first action under current Candidate authority");
    let first_action_id = match first_action {
        CandidateActionStart::Authorized(action) => action.action_id,
        other => panic!("expected a newly-authorized action, got {other:?}"),
    };

    sqlx::query("UPDATE stage_handoffs SET invalidated_at=NOW() WHERE id=$1")
        .bind(first.authority.application_model_handoff_id)
        .execute(fixture.db.pool())
        .await
        .expect("invalidate superseded Application Model Handoff");
    let stale_review = golish_db::repo::attack_candidate_approvals::list_candidate_reviews(
        fixture.db.pool(),
        fixture.fence.operation_id,
        wave_run_id,
    )
    .await
    .expect("load stale Candidate authority projection");
    assert_eq!(
        stale_review.candidates[0].candidate_authority_status,
        "stale"
    );
    assert_eq!(
        stale_review.candidates[0]
            .candidate_authority_hash
            .as_deref(),
        Some(candidate_authority_hash.as_str())
    );
    finish_candidate_action(
        fixture.db.pool(),
        FinishCandidateAction {
            operation_id: fixture.fence.operation_id,
            stage_execution_id: verification_stage_execution_id,
            stage_run_unit_id: verification_stage_run_unit_id,
            organization_id: fixture.organization_id,
            worker_run_id: claimed.worker.id,
            lease_token,
            attempt_epoch: claimed.worker.attempt_epoch,
            candidate_id,
            approval_id: claimed.attempt.approval_id,
            attempt_id: claimed.attempt.id,
            candidate_plan_hash: claimed.attempt.candidate_plan_hash.clone(),
            action_id: first_action_id,
            success: true,
            outcome: json!({"controlled_fixture": true, "network_used": false}),
            error_code: None,
        },
    )
    .await
    .expect("finish already-begun action after authority becomes stale");
    let stale_begin = begin_candidate_action(fixture.db.pool(), begin(1))
        .await
        .expect_err("a new action must not begin under stale Candidate authority");
    assert!(
        stale_begin
            .to_string()
            .contains("CANDIDATE_AUTHORITY_STALE"),
        "unexpected stale Candidate action error: {stale_begin}"
    );
    let completed_status: String =
        sqlx::query_scalar("SELECT status FROM candidate_attempt_actions WHERE id=$1")
            .bind(first_action_id)
            .fetch_one(fixture.db.pool())
            .await
            .expect("read completed historical action");
    assert_eq!(completed_status, "completed");
}

#[cfg(any())]
async fn run_strict_localhost_fact_delta_generation_loop() {
    let http = ControlledHttpFixture::start().await;
    let fixture = RuntimeFixture::start_v2("candidate-localhost-http", true).await;
    let producer = CountingProducer::valid();
    let outcome =
        run_application_understanding_unit(fixture.db.pool(), &fixture.command(), &producer)
            .await
            .expect("run strict Application Understanding");
    assert!(matches!(
        outcome,
        ApplicationUnderstandingRuntimeOutcome::Passed(_)
    ));

    let (wave_run_id, wave_unit_id, target_id) = fixture
        .freeze_candidate_manifest_for_target(&http.origin)
        .await;
    let application_model =
        attack_candidate_work_items::bind_frozen_manifest_to_current_application_model(
            fixture.db.pool(),
            fixture.fence.operation_id,
            fixture.scope_snapshot_id,
            wave_run_id,
            wave_unit_id,
            fixture.organization_id,
        )
        .await
        .expect("bind current strict Application Model");
    let work_item = &application_model.manifest.items[0];
    let source_evidence_id = fixture.source.expect("vuln source").evidence_id;
    let verified_url = format!("{}/verified", http.origin);
    let content_length = i32::try_from(b"localhost-proof".len()).unwrap();
    let directory_entry_id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO directory_entries(
               target_id,url,status_code,content_length,lines,words,tool,project_path)
           SELECT $1,$2,200,$3,1,1,'route_probe',project_path
             FROM targets WHERE id=$1
           RETURNING id"#,
    )
    .bind(target_id)
    .bind(&verified_url)
    .bind(content_length)
    .fetch_one(fixture.db.pool())
    .await
    .expect("insert frozen localhost directory observation");
    let directory_row_hash = directory_entry_row_hash(
        directory_entry_id,
        target_id,
        &verified_url,
        200,
        content_length,
    );
    let observation = json!({
        "schema": "directory_entry_observation_v1",
        "target_id": target_id,
        "directory_entry_id": directory_entry_id,
        "directory_entry_row_sha256": directory_row_hash,
        "url": verified_url,
        "method": "GET",
        "status_code": 200,
        "content_length": content_length,
        "content_type": "",
        "source_tool": "route_probe",
        "source_evidence_id": source_evidence_id,
        "network_attempted": true,
        "authority_current_after": true,
    });
    let candidate_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!(
            "{}:{}:localhost-directory-candidate",
            fixture.fence.operation_id, work_item.work_item.id
        )
        .as_bytes(),
    );
    let budget = CandidateBudget {
        max_actions: 1,
        max_requests: 1,
        max_runtime_ms: 30_000,
    };
    let action_recipe = PlannedCandidateAction {
        ordinal: 0,
        capability_id: "verify.directory_entry_replay".to_string(),
        action_kind: "directory_entry_replay".to_string(),
        recipe_version: CANDIDATE_RECIPE_VERSION_DIRECTORY_ENTRY_REPLAY_V2.to_string(),
        executor_contract_version: CANDIDATE_EXECUTOR_CONTRACT_DIRECTORY_ENTRY_REPLAY_V2
            .to_string(),
        canonical_args: json!({
            "authority_current_after": true,
            "background": false,
            "content_length": content_length,
            "content_type": "",
            "directory_entry_id": directory_entry_id,
            "directory_entry_row_sha256": directory_row_hash,
            "executor_contract_version": CANDIDATE_EXECUTOR_CONTRACT_DIRECTORY_ENTRY_REPLAY_V2,
            "follow_redirects": false,
            "method": "GET",
            "network_attempted": true,
            "no_auth": true,
            "observation": observation,
            "observation_hash": format!("sha256:{}", sha256_json(&observation)),
            "recipe_version": CANDIDATE_RECIPE_VERSION_DIRECTORY_ENTRY_REPLAY_V2,
            "source_evidence_id": source_evidence_id,
            "source_tool": "route_probe",
            "status_code": 200,
            "target": http.origin,
            "target_id": target_id,
            "technique": "WSTG-INFO",
            "url": verified_url,
        }),
        side_effect_class: SideEffectClass::ReadOnly,
        required_evidence_role: AttemptEvidenceRole::Proof,
    };
    let execution_plan = serde_json::to_value(CandidateHypothesisAuthority {
        schema_version: CANDIDATE_HYPOTHESIS_SCHEMA_V1.to_string(),
        classifier_version: CANDIDATE_CLASSIFIER_VERSION_V2.to_string(),
        candidate_id,
        target_identity_hash: work_item.work_item.target_identity_hash.clone(),
        allowed_techniques: vec!["WSTG-INFO".to_string()],
        allowed_capability_ids: vec!["verify.directory_entry_replay".to_string()],
        allowed_action_kinds: vec!["directory_entry_replay".to_string()],
        max_side_effect_class: SideEffectClass::ReadOnly,
        budget,
        foreground_only: true,
        credential_policy: "frozen_candidate_credentials_only".to_string(),
        scope_policy: "exact_candidate_target_only".to_string(),
        stop_policy: "new_target_service_credential_technique_or_parameters_requires_fact_delta"
            .to_string(),
    })
    .expect("serialize localhost Candidate hypothesis authority");
    assert!(execution_plan.get("actions").is_none());
    assert!(execution_plan.get("canonical_args").is_none());
    let action_recipes =
        vec![serde_json::to_value(action_recipe)
            .expect("serialize private localhost Candidate recipe")];
    let candidate_plan_hash =
        canonical_execution_plan_hash(&execution_plan).expect("hash localhost Candidate plan");
    let model_authority = CandidateApplicationModelAcceptance {
        manifest_id: application_model.authority.application_model_manifest_id,
        revision_id: application_model.authority.application_model_revision_id,
        manifest_hash: application_model
            .authority
            .application_model_manifest_hash
            .clone(),
        model_hash: application_model
            .authority
            .application_model_model_hash
            .clone(),
        replay_material_hash: application_model
            .authority
            .application_model_replay_material_hash
            .clone(),
        stage_handoff_id: application_model.authority.application_model_handoff_id,
        stage_execution_id: application_model
            .authority
            .application_model_stage_execution_id,
        stage_run_unit_id: application_model
            .authority
            .application_model_stage_run_unit_id,
        deliverable_submission_id: application_model
            .authority
            .application_model_deliverable_submission_id,
        gate_decision_hash: application_model
            .authority
            .application_model_gate_decision_hash
            .clone(),
        input_authority_hash: application_model.authority.input_authority_hash.clone(),
    };
    let acceptance = CandidateAcceptanceInput {
        wave_run_id,
        wave_unit_id,
        manifest_hash: application_model.authority.candidate_manifest_hash.clone(),
        application_model_authority: Some(model_authority),
        expected_work_item_ids: vec![work_item.work_item.id],
        candidates: vec![AcceptedCandidateDraft {
            candidate_id,
            work_item_id: work_item.work_item.id,
            hypothesis: "frozen localhost path remains reachable".to_string(),
            technique: Some("WSTG-INFO".to_string()),
            rationale: "exact typed directory observation".to_string(),
            prior_refs: vec![format!("audit:{source_evidence_id}")],
            suggested_approach: "dynamic_verification_strategy".to_string(),
            priority: "medium".to_string(),
            execution_plan,
            action_recipes,
            candidate_plan_hash: candidate_plan_hash.clone(),
            risk_class: "deterministic_safe".to_string(),
            evidence_ids: vec![source_evidence_id],
        }],
        no_candidate_decisions: Vec::new(),
    };
    let candidate_runtime = fixture.start_candidate_runtime_unit().await;
    let final_input = candidate_final_seal_input(&fixture, candidate_runtime, acceptance);
    runtime_memory_tx::finalize_unit_pass(fixture.db.pool(), &final_input)
        .await
        .expect("seal strict localhost Candidate");
    let frozen_candidate: (serde_json::Value, i64) = sqlx::query_as(
        r#"SELECT execution_plan,
                  (SELECT COUNT(*) FROM candidate_action_recipe_options
                    WHERE candidate_id=attack_candidates.candidate_id)
             FROM attack_candidates WHERE candidate_id=$1"#,
    )
    .bind(candidate_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("read hypothesis-only localhost Candidate");
    assert_eq!(
        frozen_candidate.0["schema_version"],
        CANDIDATE_HYPOTHESIS_SCHEMA_V1
    );
    assert!(frozen_candidate.0.get("actions").is_none());
    assert!(frozen_candidate.0.get("canonical_args").is_none());
    assert_eq!(frozen_candidate.1, 1, "one private recipe is registered");
    let review =
        authorize_wave_candidates_by_machine_policy(fixture.db.pool(), fixture.fence.operation_id)
            .await
            .expect("machine-authorize localhost Candidate hypothesis");
    assert_eq!(review.approvals.len(), 1);
    let machine_authority: (String, String, serde_json::Value, i64) = sqlx::query_as(
        r#"SELECT authorization_source,verification_contract,authorization_envelope,
                  (SELECT COUNT(*) FROM operator_principals principal
                    WHERE principal.id=approval.decided_by
                      AND principal.principal_kind='machine_policy')
             FROM attack_candidate_approvals approval
            WHERE approval.candidate_id=$1"#,
    )
    .bind(candidate_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("read machine-policy Candidate authorization");
    assert_eq!(machine_authority.0, "machine_policy");
    assert_eq!(machine_authority.1, "hypothesis_jit_v1");
    assert_eq!(machine_authority.2, frozen_candidate.0);
    assert_eq!(machine_authority.3, 1);

    let verification_stage_execution_id = Uuid::new_v4();
    let verification_stage_run_unit_id = Uuid::new_v4();
    runtime_memory_tx::transition_stage_execution(
        fixture.db.pool(),
        &runtime_memory_tx::TransitionStageExecutionRow {
            operation_id: fixture.fence.operation_id,
            current_stage_execution_id: candidate_runtime.stage_execution_id,
            next_stage_execution_id: verification_stage_execution_id,
            next_stage: "verification".to_string(),
        },
    )
    .await
    .expect("transition Candidate to localhost Verification");
    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,organization_id,
               stage_kind,generation,specialist,status)
           VALUES($1,$2,$3,$4,$5,'verification',0,'candidate_verifier','queued')"#,
    )
    .bind(verification_stage_run_unit_id)
    .bind(fixture.fence.operation_id)
    .bind(verification_stage_execution_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.organization_id)
    .execute(fixture.db.pool())
    .await
    .expect("insert localhost Verification StageRunUnit");
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,organization_id,
               worker_generation,specialist,work_item_kind,work_item_key,agent_path,status)
           VALUES($1,$2,$3,$4,$5,0,'candidate_verifier','organization','verification',
                  'main>stage_run:candidate_verifier','queued')"#,
    )
    .bind(Uuid::new_v4())
    .bind(fixture.fence.operation_id)
    .bind(verification_stage_execution_id)
    .bind(verification_stage_run_unit_id)
    .bind(fixture.organization_id)
    .execute(fixture.db.pool())
    .await
    .expect("insert Verification logical primary Worker");
    let repository: Arc<dyn RuntimeMemoryRepository> = Arc::new(GolishDbRepoProvider::new(
        Arc::new(fixture.db.pool().clone()),
    ));
    let claimed = repository
        .claim_candidate_attempt(ClaimCandidateAttempt {
            operation_id: fixture.fence.operation_id,
            organization_id: fixture.organization_id,
            verification_stage_execution_id,
            verification_stage_run_unit_id,
            lease_owner: "candidate-localhost-fixture".to_string(),
            lease_seconds: 300,
        })
        .await
        .expect("claim localhost Candidate Attempt through runtime repository")
        .expect("localhost Candidate is claimable");
    assert_eq!(
        claimed.planning_context["schema_version"],
        "candidate_verification_planning_context.v1"
    );
    assert_eq!(
        claimed.planning_context["allowed_capability_ids"],
        json!(["verify.directory_entry_replay"])
    );
    assert!(claimed.planning_context.get("actions").is_none());
    assert!(claimed.planning_context.get("canonical_args").is_none());
    let lease_token = claimed.worker.lease_token.expect("Candidate lease token");
    let candidate_attempt = claimed.candidate_attempt.clone();
    let worker_lease = golish_core::WorkerLeaseContext {
        worker_run_id: claimed.worker.id,
        stage_run_unit_id: verification_stage_run_unit_id,
        lease_token,
        attempt_epoch: claimed.worker.attempt_epoch,
    };
    let proposed = repository
        .propose_candidate_action(ProposeCandidateAction {
            control: ControlCandidateAttempt {
                candidate_attempt: candidate_attempt.clone(),
                fence: RuntimeWorkerFence {
                    operation_id: fixture.fence.operation_id,
                    stage_execution_id: verification_stage_execution_id,
                    stage_run_unit_id: verification_stage_run_unit_id,
                    worker_run_id: claimed.worker.id,
                    lease_token,
                    attempt_epoch: claimed.worker.attempt_epoch,
                    expected_checkpoint_version: claimed.worker.checkpoint_version,
                },
                organization_id: fixture.organization_id,
                lease_owner: "candidate-localhost-fixture".to_string(),
            },
            proposal_request_id: "candidate-localhost-jit-action-0".to_string(),
            capability_id: "verify.directory_entry_replay".to_string(),
            rationale: "Use the allowlisted typed replay capability to validate the hypothesis"
                .to_string(),
        })
        .await
        .expect("propose one machine-authorized JIT Candidate action");
    assert!(!proposed.replayed);
    assert_eq!(proposed.route.action_ordinal, 0);
    assert_eq!(
        proposed.route.capability_id,
        "verify.directory_entry_replay"
    );
    assert_eq!(proposed.route.action_kind, "directory_entry_replay");
    let proposal_authority: (i64, i64, String) = sqlx::query_as(
        r#"SELECT
              (SELECT COUNT(*) FROM candidate_attempt_actions WHERE attempt_id=$1),
              (SELECT COUNT(*) FROM candidate_action_proposals WHERE attempt_id=$1),
              authority_kind
             FROM candidate_attempt_actions
            WHERE attempt_id=$1 AND action_ordinal=0"#,
    )
    .bind(candidate_attempt.attempt_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("read one persisted JIT proposal and action");
    assert_eq!(proposal_authority, (1, 1, "jit_proposal".to_string()));
    let action_context = AgentToolContext {
        request_id: "localhost-action-0".to_string(),
        tool_call_record_id: None,
        tool_name: "verify_execute_candidate_action".to_string(),
        source: golish_core::events::ToolSource::SubAgent {
            agent_id: "candidate_http_operator".to_string(),
            agent_name: "Candidate HTTP Operator".to_string(),
        },
        operation_id: Some(fixture.fence.operation_id),
        stage_execution_id: Some(verification_stage_execution_id),
        stage_run_unit_id: Some(verification_stage_run_unit_id),
        organization_id: Some(fixture.organization_id),
        worker_lease: Some(worker_lease.clone()),
        candidate_attempt: Some(candidate_attempt.clone()),
    };
    let operator_grant = CandidateOperatorGrant {
        operation_id: fixture.fence.operation_id,
        stage_execution_id: verification_stage_execution_id,
        organization_id: fixture.organization_id,
        worker_lease: worker_lease.clone(),
        candidate_attempt: candidate_attempt.clone(),
        action_ordinal: proposed.route.action_ordinal,
        capability_id: proposed.route.capability_id,
        action_kind: proposed.route.action_kind,
        operator_agent_id: "candidate_http_operator".to_string(),
    };
    let workspace: String =
        sqlx::query_scalar("SELECT project_path FROM organizations WHERE id=$1")
            .bind(fixture.organization_id)
            .fetch_one(fixture.db.pool())
            .await
            .expect("read isolated workspace");
    let action_tool = VerifyExecuteCandidateActionTool::new(
        Arc::new(golish_pentest::ConfigManager::with_defaults()),
        Arc::new(fixture.db.pool().clone()),
    );
    let run_action = || {
        golish_core::with_agent_session(
            Some("candidate-localhost-http".to_string()),
            golish_core::with_agent_tool_context(
                Some(action_context.clone()),
                golish_core::with_candidate_operator_grant(
                    Some(operator_grant.clone()),
                    action_tool.execute(
                        json!({"action_ordinal": 0}),
                        std::path::Path::new(&workspace),
                    ),
                ),
            ),
        )
    };
    let action_result = run_action()
        .await
        .expect("execute real localhost Candidate action");
    assert_eq!(action_result["status"], "completed");
    assert_eq!(http.request_count.load(Ordering::SeqCst), 1);
    let proof_evidence_id = action_result["outcome"]["evidence_id"]
        .as_i64()
        .expect("typed HTTP replay writes proof evidence");
    let directory_canonical_ref_hash = action_result["outcome"]["result"]["canonical_ref_hash"]
        .as_str()
        .expect("typed HTTP replay refreshes the canonical directory fact")
        .to_string();
    assert_eq!(action_result["outcome"]["evidence_role"], "proof");
    assert_eq!(
        action_result["outcome"]["result"]["network_attempted"],
        true
    );

    let replay = run_action()
        .await
        .expect("response-loss replay reads the terminal journal");
    assert_eq!(replay["replayed_terminal"], true);
    assert_eq!(http.request_count.load(Ordering::SeqCst), 1);

    let submit_args = json!({
        "disposition": "verified",
        "proof_evidence_ids": [proof_evidence_id],
        "finding": {
            "title": "Controlled localhost path verified",
            "severity": "info",
            "cvss": 0.0,
            "affected_target": format!("{}/model-supplied-extra-path", http.origin),
            "description": "The frozen localhost path returned the exact approved response.",
            "reproduction_steps": ["Replay the frozen GET action ordinal through the typed Candidate wrapper."],
            "remediation": "Keep the path intentional and covered by regression tests."
        },
        "fact_deltas": [{
            "fact_kind": "new_surface",
            "canonical_ref_kind": "directory_entry",
            "canonical_ref_id": directory_entry_id,
            "canonical_ref_version": 1,
            "canonical_ref_hash": directory_canonical_ref_hash,
            "summary": "Controlled localhost path is currently reachable.",
            "evidence_ids": [proof_evidence_id]
        }]
    });
    let submit_request_id = format!(
        "candidate-localhost-submit:{}",
        candidate_attempt.attempt_id
    );
    let submit_tool_call_id = tool_calls::record_tracked_start(
        fixture.db.pool(),
        &submit_request_id,
        fixture.session_id,
        Some(fixture.fence.operation_id),
        None,
        "submit_candidate_attempt",
        &submit_args,
        Some(&tool_calls::RuntimeToolIdentity {
            operation_id: fixture.fence.operation_id,
            stage_execution_id: verification_stage_execution_id,
            stage_run_unit_id: Some(verification_stage_run_unit_id),
            worker_run_id: Some(claimed.worker.id),
            organization_id: Some(fixture.organization_id),
            attempt_epoch: Some(claimed.worker.attempt_epoch),
            lease_token: Some(lease_token),
        }),
    )
    .await
    .expect("start Candidate submit tool receipt");
    let worker_fence = runtime_memory_tx::RuntimeMemoryTxFence {
        operation_id: fixture.fence.operation_id,
        stage_execution_id: verification_stage_execution_id,
        stage_run_unit_id: verification_stage_run_unit_id,
        worker_run_id: claimed.worker.id,
        lease_token,
        attempt_epoch: claimed.worker.attempt_epoch,
        expected_checkpoint_version: claimed.worker.checkpoint_version,
    };
    runtime_memory_tx::begin_worker_tool(fixture.db.pool(), &worker_fence, submit_tool_call_id)
        .await
        .expect("bind Candidate submit tool to verifier Worker");
    let submit_context = AgentToolContext {
        request_id: submit_request_id,
        tool_call_record_id: Some(submit_tool_call_id),
        tool_name: "submit_candidate_attempt".to_string(),
        source: golish_core::events::ToolSource::SubAgent {
            agent_id: "candidate_verifier".to_string(),
            agent_name: "Candidate Verifier".to_string(),
        },
        operation_id: Some(fixture.fence.operation_id),
        stage_execution_id: Some(verification_stage_execution_id),
        stage_run_unit_id: Some(verification_stage_run_unit_id),
        organization_id: Some(fixture.organization_id),
        worker_lease: Some(worker_lease),
        candidate_attempt: Some(candidate_attempt.clone()),
    };
    let submit_tool = SubmitCandidateAttemptTool::new(repository.clone());
    let submit_result = golish_core::with_agent_tool_context(
        Some(submit_context),
        submit_tool.execute(submit_args, std::path::Path::new(&workspace)),
    )
    .await
    .expect("persist exact Candidate TerminalIntent");
    assert_eq!(submit_result["status"], "terminal_intent_persisted");
    runtime_memory_tx::finish_worker_tool(fixture.db.pool(), &worker_fence, submit_tool_call_id)
        .await
        .expect("land Candidate submit Worker result");
    tool_calls::record_tracked_finish(
        fixture.db.pool(),
        submit_tool_call_id,
        fixture.session_id,
        "finished",
        &serde_json::to_string(&submit_result).unwrap(),
        0,
    )
    .await
    .expect("finish Candidate submit tool receipt");

    let intent = repository
        .next_candidate_terminal_intent(fixture.fence.operation_id)
        .await
        .expect("load Candidate TerminalIntent")
        .expect("TerminalIntent is pending");
    let frozen_target: String =
        sqlx::query_scalar("SELECT target_value_at_time FROM candidate_attempts WHERE id=$1")
            .bind(candidate_attempt.attempt_id)
            .fetch_one(fixture.db.pool())
            .await
            .expect("load frozen Candidate target");
    let submitted_result: serde_json::Value = sqlx::query_scalar(
        "SELECT submitted_result FROM candidate_attempt_terminal_intents WHERE id=$1",
    )
    .bind(intent.id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("load server-normalized Candidate TerminalIntent");
    assert_eq!(
        submitted_result["finding"]["affected_target"],
        frozen_target
    );
    let message_chain_id = claimed
        .worker
        .message_chain_id
        .expect("claimed Candidate Worker owns a message chain");
    let chain: serde_json::Value =
        sqlx::query_scalar("SELECT chain FROM message_chains WHERE id=$1")
            .bind(message_chain_id)
            .fetch_one(fixture.db.pool())
            .await
            .expect("load Candidate verifier message chain");
    let checkpoint = claimed.worker.checkpoint.clone();
    let barrier = repository
        .checkpoint_candidate_terminal_barrier(CheckpointCandidateTerminalBarrier {
            checkpoint: CheckpointBoundWorkerChain {
                fence: RuntimeWorkerFence {
                    operation_id: fixture.fence.operation_id,
                    stage_execution_id: verification_stage_execution_id,
                    stage_run_unit_id: verification_stage_run_unit_id,
                    worker_run_id: claimed.worker.id,
                    lease_token,
                    attempt_epoch: claimed.worker.attempt_epoch,
                    expected_checkpoint_version: claimed.worker.checkpoint_version,
                },
                message_chain_id,
                chain,
                checkpoint,
            },
            terminal_intent_id: intent.id,
            expected_intent_hash: intent.intent_hash.clone(),
        })
        .await
        .expect("checkpoint Candidate terminal barrier");
    let terminal = repository
        .terminalize_candidate_intent(TerminalizeCandidateIntent {
            operation_id: fixture.fence.operation_id,
            terminal_intent_id: intent.id,
            barrier_id: barrier.id,
            expected_intent_hash: intent.intent_hash,
            expected_barrier_hash: barrier.barrier_hash,
        })
        .await
        .expect("terminalize Candidate from exact barrier");
    assert_eq!(terminal.status, "verified");
    assert_eq!(terminal.evidence_count, 1);
    assert_eq!(terminal.fact_delta_count, 1);
    assert!(terminal.finding_id.is_some());

    let gate_diagnostic: serde_json::Value = sqlx::query_scalar(
        r#"SELECT jsonb_build_object(
                  'bundle_exact', verification_attempt_terminal_bundle_exact($1,$2,$3,$4,$5,$6),
                  'pending_work_items', (SELECT COUNT(*) FROM attack_candidate_work_items
                    WHERE wave_unit_id=$5 AND decision_kind IS NULL),
                  'approval_count', (SELECT COUNT(DISTINCT candidate_id) FROM attack_candidate_approvals
                    WHERE wave_unit_id=$5 AND status<>'rejected'),
                  'terminal_attempt_count', (SELECT COUNT(*) FROM candidate_attempts
                    WHERE wave_unit_id=$5 AND status IN ('verified','refuted','blocked')),
                  'finding_lineage', EXISTS(SELECT 1 FROM finding_lineage
                    WHERE candidate_attempt_id=$1),
                  'terminal_event', EXISTS(SELECT 1 FROM knowledge_outbox_events
                    WHERE event_id=uuid_generate_v5($1,'CandidateAttemptTerminal.v1')),
                  'attempt', (SELECT jsonb_build_object(
                      'status', status,
                      'row_version', row_version,
                      'result_hash_exact', result_hash =
                          'sha256:' || verification_sha256_jsonb(result_json),
                      'result_disposition_exact', result_json->>'disposition'=status)
                    FROM candidate_attempts WHERE id=$1),
                  'candidate_exact', EXISTS(
                    SELECT 1 FROM candidate_attempts attempt
                    JOIN attack_candidates candidate
                      ON candidate.terminal_attempt_id=attempt.id
                     AND candidate.candidate_id=attempt.candidate_id
                     AND candidate.operation_uuid=attempt.operation_id
                     AND candidate.scope_snapshot_id=attempt.scope_snapshot_id
                     AND candidate.wave_run_id=attempt.wave_run_id
                     AND candidate.wave_unit_id=attempt.wave_unit_id
                     AND candidate.organization_id=attempt.organization_id
                     AND candidate.target_identity_hash=attempt.target_identity_hash
                     AND candidate.candidate_plan_hash=attempt.candidate_plan_hash
                     AND candidate.disposition=attempt.status
                    WHERE attempt.id=$1),
                  'worker', (SELECT jsonb_build_object(
                      'status', worker.status,
                      'terminal', worker.terminal_at IS NOT NULL,
                      'lease_clear', worker.lease_token IS NULL AND worker.lease_owner IS NULL
                          AND worker.lease_acquired_at IS NULL AND worker.lease_expires_at IS NULL
                          AND worker.heartbeat_at IS NULL,
                      'active_tool_clear', worker.active_tool_call_id IS NULL
                          AND worker.active_tool_started_at IS NULL,
                      'lane_count', (SELECT COUNT(*) FROM attack_execution_lanes lane
                          WHERE lane.stage_worker_run_id=worker.id))
                    FROM candidate_attempts attempt
                    JOIN stage_worker_runs worker ON worker.id=attempt.stage_worker_run_id
                    WHERE attempt.id=$1),
                  'event', (SELECT jsonb_build_object(
                      'source_version', event.source_version,
                      'attempt_row_version', attempt.row_version,
                      'occurred_exact', event.occurred_at=attempt.terminal_at,
                      'payload_result_hash', event.payload#>>'{structured_payload,result_hash}',
                      'attempt_result_hash', attempt.result_hash,
                      'payload_evidence_ids', event.payload#>'{structured_payload,evidence_ids}',
                      'relational_evidence_ids', to_jsonb(COALESCE(ARRAY(
                          SELECT DISTINCT evidence.evidence_id
                          FROM candidate_attempt_evidence evidence
                          WHERE evidence.attempt_id=attempt.id ORDER BY evidence.evidence_id
                      ),'{}'::BIGINT[])),
                      'payload_fact_delta_count', event.payload#>>'{structured_payload,fact_delta_count}',
                      'relational_fact_delta_count', (SELECT COUNT(*) FROM attack_fact_deltas delta
                          WHERE delta.source_attempt_id=attempt.id
                            AND delta.candidate_id=attempt.candidate_id
                            AND delta.operation_id=attempt.operation_id
                            AND delta.scope_snapshot_id=attempt.scope_snapshot_id
                            AND delta.wave_run_id=attempt.wave_run_id
                            AND delta.wave_unit_id=attempt.wave_unit_id
                            AND delta.organization_id=attempt.organization_id),
                      'projection_delivery_count', (SELECT COUNT(*)
                          FROM knowledge_projection_deliveries delivery
                          WHERE delivery.event_id=event.event_id))
                    FROM candidate_attempts attempt
                    JOIN knowledge_outbox_events event
                      ON event.event_id=uuid_generate_v5(attempt.id,'CandidateAttemptTerminal.v1')
                    WHERE attempt.id=$1),
                  'actions', (SELECT jsonb_build_object(
                      'planned_count', jsonb_array_length(candidate.execution_plan->'actions'),
                      'journal_count', (SELECT COUNT(*) FROM candidate_attempt_actions action
                          WHERE action.attempt_id=attempt.id),
                      'exact_count', (SELECT COUNT(*)
                          FROM jsonb_array_elements(candidate.execution_plan->'actions') planned(value)
                          WHERE EXISTS(SELECT 1 FROM candidate_attempt_actions action
                              WHERE action.attempt_id=attempt.id
                                AND action.action_ordinal=(planned.value->>'ordinal')::INTEGER
                                AND action.capability_id=planned.value->>'capability_id'
                                AND action.action_kind=planned.value->>'action_kind'
                                AND action.canonical_args=planned.value->'canonical_args'
                                AND action.status IN ('completed','failed')
                                AND action.completed_at IS NOT NULL)))
                    FROM candidate_attempts attempt
                    JOIN attack_candidates candidate ON candidate.candidate_id=attempt.candidate_id
                    WHERE attempt.id=$1)
              )"#,
    )
    .bind(candidate_attempt.attempt_id)
    .bind(fixture.fence.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(wave_run_id)
    .bind(wave_unit_id)
    .bind(fixture.organization_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("read Verification Gate diagnostic");
    assert_eq!(
        gate_diagnostic["bundle_exact"], true,
        "terminal Candidate authority must be exact before Verification close: {gate_diagnostic}"
    );

    let closed = repository
        .close_attack_v2_verification_unit(CloseAttackV2VerificationUnit {
            operation_id: fixture.fence.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id,
            wave_unit_id,
            organization_id: fixture.organization_id,
            verification_stage_execution_id,
            verification_stage_run_unit_id,
        })
        .await
        .expect("close Verification Unit through relational Gate");
    assert!(closed.verification_closed);
    assert_eq!(closed.consolidation_status, "ready");
    assert_eq!(closed.verification_stage_run_unit_status, "passed");
    assert_eq!(closed.verification_primary_worker_status, "passed");
    assert_eq!(http.request_count.load(Ordering::SeqCst), 1);

    let terminal_state: (String, String, String, i64, i64, i64) = sqlx::query_as(
        r#"SELECT attempt.status,candidate.disposition,delta.status,
                  (SELECT COUNT(*) FROM findings WHERE id=$4),
                  (SELECT COUNT(*) FROM candidate_attempt_evidence
                    WHERE attempt_id=attempt.id AND evidence_id=$5),
                  (SELECT COUNT(*) FROM candidate_attempt_actions
                    WHERE attempt_id=attempt.id AND status='completed')
             FROM candidate_attempts attempt
             JOIN attack_candidates candidate ON candidate.candidate_id=attempt.candidate_id
             JOIN attack_fact_deltas delta ON delta.source_attempt_id=attempt.id
            WHERE attempt.id=$1 AND candidate.candidate_id=$2 AND delta.canonical_ref_id=$3"#,
    )
    .bind(candidate_attempt.attempt_id)
    .bind(candidate_id)
    .bind(directory_entry_id)
    .bind(terminal.finding_id.expect("verified Finding"))
    .bind(proof_evidence_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("read terminal Candidate/Finding/FactDelta lineage");
    assert_eq!(terminal_state.0, "verified");
    assert_eq!(terminal_state.1, "verified");
    assert_eq!(terminal_state.2, "proposed");
    assert_eq!(terminal_state.3, 1);
    assert_eq!(terminal_state.4, 2, "proof and fact_delta roles are frozen");
    assert_eq!(terminal_state.5, 1);

    let mut consolidation_tx = fixture
        .db
        .pool()
        .begin()
        .await
        .expect("begin strict localhost FactDelta consolidation");
    let consolidation = attack_wave_consolidations::consolidate_attack_wave(
        &mut consolidation_tx,
        attack_wave_consolidations::ConsolidateAttackWave {
            operation_id: fixture.fence.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            source_wave_run_id: wave_run_id,
        },
    )
    .await
    .expect("typed localhost FactDelta opens generation one");
    consolidation_tx
        .commit()
        .await
        .expect("commit strict localhost generation one");
    let consolidation_decisions: Vec<(String, String, Option<i64>, Option<String>)> =
        sqlx::query_as(
            r#"SELECT disposition,reason_code,resolved_ref_version,resolved_ref_hash
                 FROM attack_fact_delta_decisions
                WHERE operation_id=$1 AND source_wave_run_id=$2
                ORDER BY fact_delta_id"#,
        )
        .bind(fixture.fence.operation_id)
        .bind(wave_run_id)
        .fetch_all(fixture.db.pool())
        .await
        .expect("read strict localhost FactDelta decisions");
    assert_eq!(
        consolidation.decision_kind, "opened_next_wave",
        "consolidation={consolidation:?} decisions={consolidation_decisions:?}"
    );
    let generation_one_wave_run_id = consolidation
        .target_wave_run_id
        .expect("typed FactDelta creates generation one");
    let generation_one: (i32, Uuid, String, Uuid, String, i64, Uuid) = sqlx::query_as(
        r#"SELECT run.generation,authority.source_consolidation_id,
                  authority.parent_input_authority_hash,authority.parent_wave_unit_id,
                  authority.input_authority_hash,
                  (SELECT COUNT(*) FROM attack_candidate_work_items AS work_item
                    WHERE work_item.wave_unit_id=unit.id
                      AND work_item.decision_kind IS NULL),
                  unit.id
             FROM attack_wave_runs AS run
             JOIN attack_wave_units AS unit
               ON unit.wave_run_id=run.id
              AND unit.operation_id=run.operation_id
              AND unit.scope_snapshot_id=run.scope_snapshot_id
              AND unit.organization_id=$3
             JOIN attack_wave_application_model_authorities AS authority
               ON authority.wave_unit_id=unit.id
              AND authority.wave_run_id=run.id
              AND authority.operation_id=run.operation_id
              AND authority.scope_snapshot_id=run.scope_snapshot_id
              AND authority.organization_id=unit.organization_id
            WHERE run.id=$1 AND run.operation_id=$2"#,
    )
    .bind(generation_one_wave_run_id)
    .bind(fixture.fence.operation_id)
    .bind(fixture.organization_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("load strict generation-one Application Model authority");
    assert_eq!(generation_one.0, 1);
    assert_eq!(generation_one.1, consolidation.consolidation_id);
    assert_eq!(generation_one.3, wave_unit_id);
    assert_eq!(
        generation_one.2,
        application_model.authority.input_authority_hash
    );
    assert!(generation_one.4.starts_with("sha256:"));
    assert_ne!(generation_one.4, generation_one.2);
    assert_eq!(generation_one.5, 1);
    assert_eq!(http.request_count.load(Ordering::SeqCst), 1);

    let generation_one_candidate_runtime =
        insert_follow_on_candidate_runtime_unit(&fixture, verification_stage_execution_id, 1).await;
    let generation_one_candidate_stage_execution_id =
        generation_one_candidate_runtime.stage_execution_id;
    let generation_one_model =
        attack_candidate_work_items::load_bound_manifest_with_application_model_for_runtime_unit(
            fixture.db.pool(),
            fixture.fence.operation_id,
            generation_one_candidate_runtime.stage_run_unit_id,
            fixture.organization_id,
        )
        .await
        .expect("load generation-one manifest with inherited current Application Model");
    assert_eq!(
        generation_one_model.manifest.wave_run_id,
        generation_one_wave_run_id
    );
    assert_eq!(generation_one_model.manifest.wave_unit_id, generation_one.6);
    assert_eq!(generation_one_model.manifest.items.len(), 1);
    assert_eq!(
        generation_one_model
            .authority
            .parent_input_authority_hash
            .as_deref(),
        Some(application_model.authority.input_authority_hash.as_str())
    );
    let generation_one_item = &generation_one_model.manifest.items[0];
    assert_eq!(generation_one_item.evidence_ids, vec![proof_evidence_id]);
    assert_eq!(
        generation_one_item.observation_kind,
        "directory_entry_observation_v1"
    );
    let mut generation_one_evidence_tx = fixture
        .db
        .pool()
        .begin()
        .await
        .expect("begin generation-one inherited evidence read");
    let generation_one_inherited_evidence =
        attack_candidate_work_items::load_frozen_entry_evidence_ids_with_connection(
            &mut generation_one_evidence_tx,
            fixture.fence.operation_id,
            fixture.scope_snapshot_id,
            generation_one_wave_run_id,
            generation_one.6,
            fixture.organization_id,
        )
        .await
        .expect("load generation-one exact FactDelta evidence denominator");
    generation_one_evidence_tx
        .rollback()
        .await
        .expect("finish generation-one inherited evidence read");
    assert!(generation_one_inherited_evidence.contains(&proof_evidence_id));
    assert!(generation_one_inherited_evidence.len() >= 2);
    let generation_one_acceptance = CandidateAcceptanceInput {
        wave_run_id: generation_one_wave_run_id,
        wave_unit_id: generation_one.6,
        manifest_hash: generation_one_model
            .authority
            .candidate_manifest_hash
            .clone(),
        application_model_authority: Some(CandidateApplicationModelAcceptance {
            manifest_id: generation_one_model.authority.application_model_manifest_id,
            revision_id: generation_one_model.authority.application_model_revision_id,
            manifest_hash: generation_one_model
                .authority
                .application_model_manifest_hash
                .clone(),
            model_hash: generation_one_model
                .authority
                .application_model_model_hash
                .clone(),
            replay_material_hash: generation_one_model
                .authority
                .application_model_replay_material_hash
                .clone(),
            stage_handoff_id: generation_one_model.authority.application_model_handoff_id,
            stage_execution_id: generation_one_model
                .authority
                .application_model_stage_execution_id,
            stage_run_unit_id: generation_one_model
                .authority
                .application_model_stage_run_unit_id,
            deliverable_submission_id: generation_one_model
                .authority
                .application_model_deliverable_submission_id,
            gate_decision_hash: generation_one_model
                .authority
                .application_model_gate_decision_hash
                .clone(),
            input_authority_hash: generation_one_model.authority.input_authority_hash.clone(),
        }),
        expected_work_item_ids: vec![generation_one_item.work_item.id],
        candidates: Vec::new(),
        no_candidate_decisions: vec![NoCandidateDecision {
            work_item_id: generation_one_item.work_item.id,
            reason_code: "fact_delta_checked_empty".to_string(),
            detail: "The typed localhost delta produced no new approved action.".to_string(),
            evidence_ids: vec![proof_evidence_id],
        }],
    };
    runtime_memory_tx::finalize_unit_pass(
        fixture.db.pool(),
        &candidate_final_seal_input(
            &fixture,
            generation_one_candidate_runtime,
            generation_one_acceptance,
        ),
    )
    .await
    .expect("final-seal generation-one Candidate as exact no-candidate");
    let generation_one_review = list_candidate_reviews(
        fixture.db.pool(),
        fixture.fence.operation_id,
        generation_one_wave_run_id,
    )
    .await
    .expect("close generation-one zero-Candidate review from DB truth");
    assert!(generation_one_review.review_closed);
    assert_eq!(generation_one_review.candidate_count, 0);

    let generation_one_verification_stage_execution_id = Uuid::new_v4();
    runtime_memory_tx::transition_stage_execution(
        fixture.db.pool(),
        &runtime_memory_tx::TransitionStageExecutionRow {
            operation_id: fixture.fence.operation_id,
            current_stage_execution_id: generation_one_candidate_stage_execution_id,
            next_stage_execution_id: generation_one_verification_stage_execution_id,
            next_stage: "verification".to_string(),
        },
    )
    .await
    .expect("transition generation-one Candidate to Verification");
    let generation_one_verification_stage_run_unit_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,organization_id,
               stage_kind,generation,specialist,status)
           VALUES($1,$2,$3,$4,$5,'verification',1,'candidate_verifier','queued')"#,
    )
    .bind(generation_one_verification_stage_run_unit_id)
    .bind(fixture.fence.operation_id)
    .bind(generation_one_verification_stage_execution_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.organization_id)
    .execute(fixture.db.pool())
    .await
    .expect("insert generation-one Verification Unit");
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,organization_id,
               worker_generation,specialist,work_item_kind,work_item_key,agent_path,status)
           VALUES($1,$2,$3,$4,$5,1,'candidate_verifier','organization','verification',
                  'main>stage_run:candidate_verifier','queued')"#,
    )
    .bind(Uuid::new_v4())
    .bind(fixture.fence.operation_id)
    .bind(generation_one_verification_stage_execution_id)
    .bind(generation_one_verification_stage_run_unit_id)
    .bind(fixture.organization_id)
    .execute(fixture.db.pool())
    .await
    .expect("insert generation-one Verification primary Worker");
    let generation_one_closed = repository
        .close_attack_v2_verification_unit(CloseAttackV2VerificationUnit {
            operation_id: fixture.fence.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: generation_one_wave_run_id,
            wave_unit_id: generation_one.6,
            organization_id: fixture.organization_id,
            verification_stage_execution_id: generation_one_verification_stage_execution_id,
            verification_stage_run_unit_id: generation_one_verification_stage_run_unit_id,
        })
        .await
        .expect("close generation-one Verification through the zero-Candidate Gate");
    assert!(generation_one_closed.verification_closed);
    assert_eq!(generation_one_closed.consolidation_status, "ready");
    let mut generation_one_consolidation_tx = fixture
        .db
        .pool()
        .begin()
        .await
        .expect("begin generation-one no-delta consolidation");
    let generation_one_consolidation = attack_wave_consolidations::consolidate_attack_wave(
        &mut generation_one_consolidation_tx,
        attack_wave_consolidations::ConsolidateAttackWave {
            operation_id: fixture.fence.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            source_wave_run_id: generation_one_wave_run_id,
        },
    )
    .await
    .expect("generation-one Verification closes without another delta");
    generation_one_consolidation_tx
        .commit()
        .await
        .expect("commit generation-one closed-no-delta result");
    assert_eq!(
        generation_one_consolidation.decision_kind,
        "closed_no_delta"
    );
    assert_eq!(generation_one_consolidation.target_wave_run_id, None);
    assert_eq!(http.request_count.load(Ordering::SeqCst), 1);
    let final_lineage: (i64, i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT
              (SELECT COUNT(*) FROM attack_wave_runs WHERE operation_id=$1),
              (SELECT COUNT(*) FROM attack_wave_application_model_authorities
                WHERE operation_id=$1),
              (SELECT COUNT(*) FROM attack_candidates WHERE operation_uuid=$1),
              (SELECT COUNT(*) FROM candidate_attempts WHERE operation_id=$1),
              (SELECT COUNT(*) FROM attack_fact_deltas WHERE operation_id=$1)"#,
    )
    .bind(fixture.fence.operation_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("read two-generation localhost lineage counts");
    assert_eq!(final_lineage, (2, 2, 1, 1, 1));

    let reporting_snapshot =
        golish_agent_app::ai::db_bridge::reporting::current_reportable_source_snapshot(
            fixture.db.pool(),
            fixture.fence.operation_id,
        )
        .await
        .expect("freeze Reporting source snapshot after Candidate generation closure");
    let reporting_source_kinds = reporting_snapshot
        .ordered_sources
        .iter()
        .map(|source| source.kind.as_str())
        .collect::<BTreeSet<_>>();
    for expected in [
        "attack_wave_run",
        "attack_wave_unit",
        "attack_wave_consolidation",
        "attack_candidate",
        "attack_candidate_approval",
        "candidate_attempt",
        "finding",
        "attack_fact_delta",
        "finding_lineage",
        "evidence_audit",
    ] {
        assert!(
            reporting_source_kinds.contains(expected),
            "Reporting source manifest must contain {expected}: {reporting_source_kinds:?}"
        );
    }

    let principal = golish_db::repo::operator_principals::current_local(fixture.db.pool())
        .await
        .expect("load server-owned local CLI principal");
    let principal_provider = LocalCliPrincipalProvider {
        principal_id: principal.id,
    };
    let artifact_store_factory = TestProjectReportArtifactStoreFactory;
    let reporting_pool = Arc::new(fixture.db.pool().clone());
    let built = build_reporting_read_model_for_local_channel(
        reporting_pool.clone(),
        &principal_provider,
        fixture.fence.operation_id,
        OperatorChannel::LocalCli,
    )
    .await
    .expect("build validated report through the shared Reporting kernel");
    let current = built
        .current
        .as_ref()
        .expect("Reporting build publishes a current revision");
    assert_eq!(current.validation_status, "validated");
    assert_eq!(current.publication_status, "unpublished");
    let reporting_gate_truth =
        golish_agent_app::ai::db_bridge::reporting::load_reporting_gate_truth_with_barrier(
            &reporting_pool,
            fixture.fence.operation_id,
            || async {},
        )
        .await
        .expect("load Reporting Gate truth from one repeatable-read snapshot")
        .expect("validated report exposes Reporting Gate truth");
    golish_agent_kit::harness::validate_reporting_gate_truth(&reporting_gate_truth)
        .expect("Reporting Gate passes before LocalCli publication");

    let finalized = finalize_reporting_revision_for_local_channel(
        reporting_pool,
        &principal_provider,
        &artifact_store_factory,
        OperatorChannel::LocalCli,
        ReportingFinalizeFence {
            operation_id: fixture.fence.operation_id,
            revision_id: Uuid::parse_str(&current.revision_id).expect("current revision UUID"),
            expected_source_hash: current.source_set_hash.clone(),
            expected_revision_version: current.row_version,
            confirm_final_publish: true,
        },
    )
    .await
    .expect("explicit LocalCli publication produces real report artifacts");
    assert_eq!(
        finalized
            .current
            .as_ref()
            .expect("final revision remains current")
            .publication_status,
        "final"
    );
    assert_eq!(finalized.artifacts.len(), 2);
    let citation_count = finalized
        .sections
        .iter()
        .flat_map(|section| &section.claims)
        .flat_map(|claim| &claim.citations)
        .inspect(|citation| assert!(citation.evidence_audit_id > 0))
        .count();
    assert!(
        citation_count > 0,
        "Finding claims must retain evidence citations"
    );

    let project_root = std::path::Path::new(&fixture.workspace);
    let mut json_artifact = None;
    for artifact in &finalized.artifacts {
        let blob_path = report_blob_path(project_root, &artifact.content_key);
        let bytes = std::fs::read(&blob_path).expect("read content-addressed report blob");
        assert_eq!(
            i64::try_from(bytes.len()).expect("artifact byte length fits i64"),
            artifact.byte_len
        );
        assert_eq!(
            Sha256::digest(&bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
            artifact.sha256
        );
        if artifact.artifact_kind == "json" {
            json_artifact = Some(
                serde_json::from_slice::<serde_json::Value>(&bytes)
                    .expect("parse Reporting JSON artifact"),
            );
        } else {
            let markdown = String::from_utf8(bytes).expect("Reporting Markdown is UTF-8");
            assert!(markdown.contains("## Lineage Manifest"));
            assert!(markdown.contains("Evidence "));
        }
    }
    let json_artifact = json_artifact.expect("JSON report artifact exists");
    assert_eq!(json_artifact["schema"], "golish.report_artifact.v1");
    let lineage = json_artifact["lineage"]
        .as_array()
        .expect("artifact carries a lineage manifest");
    let lineage_kind_counts = lineage.iter().fold(
        std::collections::BTreeMap::<&str, usize>::new(),
        |mut counts, source| {
            *counts
                .entry(source["kind"].as_str().expect("lineage kind"))
                .or_default() += 1;
            counts
        },
    );
    for (kind, expected) in [
        ("attack_wave_run", 2),
        ("attack_wave_unit", 2),
        ("attack_wave_consolidation", 2),
        ("attack_candidate", 1),
        ("attack_candidate_approval", 1),
        ("candidate_attempt", 1),
        ("finding", 1),
        ("attack_fact_delta", 1),
        ("finding_lineage", 1),
    ] {
        assert_eq!(
            lineage_kind_counts.get(kind).copied().unwrap_or_default(),
            expected,
            "artifact lineage count for {kind}"
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM knowledge_outbox_events \
             WHERE event_name='ReportRevisionFinalized.v1' AND source_id_value=$1",
        )
        .bind(
            finalized
                .current
                .as_ref()
                .expect("final revision")
                .revision_id
                .clone(),
        )
        .fetch_one(fixture.db.pool())
        .await
        .expect("count report finalization outbox"),
        1
    );
    assert_eq!(http.request_count.load(Ordering::SeqCst), 1);
}

#[cfg(any())]
#[tokio::test]
#[serial]
async fn strict_candidate_localhost_http_reaches_finding_fact_delta_and_verification_gate() {
    run_strict_localhost_fact_delta_generation_loop().await;
}

#[cfg(any())]
#[tokio::test]
#[serial]
async fn strict_localhost_fact_delta_opens_generation_one_then_closes_no_delta() {
    run_strict_localhost_fact_delta_generation_loop().await;
}

#[cfg(any())]
#[tokio::test]
#[serial]
async fn strict_localhost_generation_loop_publishes_cli_reporting_artifacts() {
    run_strict_localhost_fact_delta_generation_loop().await;
}

#[cfg(any())]
#[tokio::test]
#[serial]
async fn candidate_verification_response_loss_never_replays_localhost_request() {
    let http = ControlledHttpFixture::start().await;
    let harness = prepare_localhost_action_harness("candidate-response-loss", &http).await;

    sqlx::raw_sql(
        r#"CREATE FUNCTION test_reject_candidate_action_finish()
           RETURNS trigger AS $$
           BEGIN
               IF OLD.status='started' AND NEW.status IN ('completed','failed') THEN
                   RAISE EXCEPTION 'TEST_CANDIDATE_FINISH_RESPONSE_LOST';
               END IF;
               RETURN NEW;
           END;
           $$ LANGUAGE plpgsql;
           CREATE TRIGGER aa_test_reject_candidate_action_finish
           BEFORE UPDATE OF status ON candidate_attempt_actions
           FOR EACH ROW EXECUTE FUNCTION test_reject_candidate_action_finish();"#,
    )
    .execute(harness.fixture.db.pool())
    .await
    .expect("install isolated response-loss finish failpoint");

    let first = tokio::time::timeout(std::time::Duration::from_secs(15), harness.execute_action())
        .await
        .expect("first response-loss wrapper call must not hang")
        .expect_err("finish response loss must surface after the real request");
    assert!(first
        .to_string()
        .contains("TEST_CANDIDATE_FINISH_RESPONSE_LOST"));
    assert_eq!(http.request_count.load(Ordering::SeqCst), 1);
    let first_state: (String, bool, i64) = sqlx::query_as(
        r#"SELECT action.status,action.authorization_receipt_id IS NOT NULL,
                  (SELECT COUNT(*) FROM audit_log evidence
                    WHERE evidence.run_id=$2 AND evidence.audit_role='evidence'
                      AND evidence.detail->>'kind'='verification.directory_entry_replay_v1')
             FROM candidate_attempt_actions action
            WHERE action.attempt_id=$1 AND action.action_ordinal=0"#,
    )
    .bind(harness.attempt_id)
    .bind(harness.fixture.fence.operation_id)
    .fetch_one(harness.fixture.db.pool())
    .await
    .expect("read response-loss journal and evidence");
    assert_eq!(first_state.0, "started");
    assert!(
        first_state.1,
        "authorization receipt must commit before HTTP"
    );
    assert_eq!(first_state.2, 1, "HTTP evidence must survive finish loss");

    let replay = tokio::time::timeout(std::time::Duration::from_secs(15), harness.execute_action())
        .await
        .expect("outcome-unknown replay must not hang")
        .expect("same ordinal must become outcome_unknown without replay");
    assert_eq!(replay["status"], "outcome_unknown");
    assert_eq!(replay["code"], "ATTACK_ACTION_OUTCOME_UNKNOWN");
    assert_eq!(replay["review_required"], true);
    assert_eq!(http.request_count.load(Ordering::SeqCst), 1);

    sqlx::query(
        "UPDATE stage_worker_runs
            SET lease_acquired_at=NOW()-INTERVAL '2 minutes',
                heartbeat_at=NOW()-INTERVAL '1 minute',
                lease_expires_at=NOW()-INTERVAL '1 second'
          WHERE id=$1",
    )
    .bind(harness.worker_run_id)
    .execute(harness.fixture.db.pool())
    .await
    .expect("expire isolated response-loss Worker lease");
    sqlx::query(
        "UPDATE attack_execution_lanes SET lease_expires_at=NOW()-INTERVAL '1 second' \
         WHERE lane_key='global:exploit' AND stage_worker_run_id=$1 AND lease_token=$2",
    )
    .bind(harness.worker_run_id)
    .bind(harness.lease_token)
    .execute(harness.fixture.db.pool())
    .await
    .expect("expire isolated response-loss lane");
    let reclaimed = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        claim_next_candidate_attempt(
            harness.fixture.db.pool(),
            CandidateClaimQuery {
                operation_id: harness.fixture.fence.operation_id,
                scope_snapshot_id: harness.fixture.scope_snapshot_id,
                wave_run_id: harness.wave_run_id,
                wave_unit_id: harness.wave_unit_id,
                organization_id: harness.fixture.organization_id,
                verification_stage_execution_id: harness.verification_stage_execution_id,
                verification_stage_run_unit_id: harness.verification_stage_run_unit_id,
                lease_owner: "candidate-response-loss-recovery-probe".to_string(),
                lease_seconds: 300,
            },
        ),
    )
    .await
    .expect("expired response-loss lane reconciliation must not hang")
    .expect("reconcile expired outcome-unknown lane");
    assert!(
        reclaimed.is_none(),
        "recovery-required Attempt is not executable"
    );
    let recovery_state: (String, String, i64) = sqlx::query_as(
        r#"SELECT action.status,worker.status,
                  (SELECT COUNT(*) FROM candidate_recovery_cases recovery
                    WHERE recovery.attempt_id=$1 AND recovery.action_id=action.id
                      AND recovery.case_kind='outcome_unknown' AND recovery.status='open')
             FROM candidate_attempt_actions action
             JOIN candidate_attempts attempt ON attempt.id=action.attempt_id
             JOIN stage_worker_runs worker ON worker.id=attempt.stage_worker_run_id
            WHERE action.attempt_id=$1 AND action.action_ordinal=0"#,
    )
    .bind(harness.attempt_id)
    .fetch_one(harness.fixture.db.pool())
    .await
    .expect("read durable outcome-unknown recovery authority");
    assert_eq!(recovery_state.0, "outcome_unknown");
    assert_eq!(recovery_state.1, "recovery_required");
    assert_eq!(recovery_state.2, 1);
    assert_eq!(http.request_count.load(Ordering::SeqCst), 1);
}

#[cfg(any())]
#[tokio::test]
#[serial]
async fn candidate_application_model_authority_missing_current_fails_closed() {
    let fixture = RuntimeFixture::start("candidate-model-missing", true).await;
    let (wave_run_id, wave_unit_id) = fixture.freeze_candidate_manifest().await;
    let result = attack_candidate_work_items::bind_frozen_manifest_to_current_application_model(
        fixture.db.pool(),
        fixture.fence.operation_id,
        fixture.scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        fixture.organization_id,
    )
    .await;
    assert!(result.is_err());
    let authority_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM attack_wave_application_model_authorities \
         WHERE wave_unit_id=$1",
    )
    .bind(wave_unit_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("count rejected Candidate authorities");
    assert_eq!(authority_count, 0);
}

#[tokio::test]
#[serial]
async fn formal_stage_controller_runs_typed_no_tool_model_and_aggregate_gate() {
    let fixture = RuntimeFixture::start_v2("formal-stage-controller", true).await;
    fixture.prepare_for_formal_stage_controller().await;
    fixture.insert_safe_projection_surface().await;
    let producer = TypedStageProducer::new();
    let runtime = golish_agent_app::ai::application_understanding_runtime::PgApplicationUnderstandingStageRuntime::new(
        std::sync::Arc::new(fixture.db.pool().clone()),
    );
    let replacement = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: fixture.fence.stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &producer,
        )
        .await
        .expect("replace legacy direct AU runtime");
    let ApplicationUnderstandingStageOutcome::Blocked { code, refs } = replacement else {
        panic!("legacy direct AU runtime must be replaced before model dispatch")
    };
    assert_eq!(
        code,
        "APPLICATION_UNDERSTANDING_RUNTIME_REPLACED_CONTINUE_REQUIRED"
    );
    let replacement_stage_execution_id = refs
        .iter()
        .find_map(|reference| reference.strip_prefix("replacement_stage_execution:"))
        .and_then(|value| Uuid::parse_str(value).ok())
        .expect("replacement execution identity");
    assert_eq!(producer.legacy_calls.load(Ordering::SeqCst), 0);
    assert_eq!(producer.shard_calls.load(Ordering::SeqCst), 0);
    assert_eq!(producer.synthesis_calls.load(Ordering::SeqCst), 0);

    let outcome = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: replacement_stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &producer,
        )
        .await
        .expect("run hierarchical AU stage controller");

    assert_eq!(
        outcome,
        ApplicationUnderstandingStageOutcome::Passed {
            completed_units: 1,
            total_units: 1,
        }
    );
    assert_eq!(producer.legacy_calls.load(Ordering::SeqCst), 0);
    assert_eq!(producer.shard_calls.load(Ordering::SeqCst), 2);
    assert_eq!(producer.synthesis_calls.load(Ordering::SeqCst), 1);
    let persisted: (i64, i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT
              (SELECT count(*) FROM stage_run_units
                WHERE operation_id=$1 AND stage_execution_id=$2
                  AND stage_kind='application_understanding' AND status='passed'),
              (SELECT count(*) FROM application_model_current_revisions AS current_revision
                 JOIN application_model_manifests AS manifest
                   ON manifest.id=current_revision.manifest_id
                WHERE manifest.operation_id=$1 AND manifest.stage_execution_id=$2),
              (SELECT count(*) FROM stage_handoffs
                WHERE operation_id=$1 AND stage_execution_id=$2
                  AND from_stage_kind='application_understanding'),
              (SELECT count(*) FROM stage_team_plans
                WHERE operation_id=$1 AND stage_execution_id=$2),
              (SELECT count(*) FROM stage_work_items
                WHERE operation_id=$1 AND stage_execution_id=$2
                  AND required_for_barrier),
              (SELECT count(*) FROM stage_worker_runs
                WHERE operation_id=$1 AND stage_execution_id=$2
                  AND work_item_id IS NULL),
              (SELECT count(*) FROM stage_worker_runs AS worker
                 JOIN message_chains AS chain ON chain.id=worker.message_chain_id
                WHERE worker.operation_id=$1 AND worker.stage_execution_id=$2
                  AND jsonb_typeof(chain.chain)<>'array')"#,
    )
    .bind(fixture.fence.operation_id)
    .bind(replacement_stage_execution_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("read formal AU aggregate truth");
    assert_eq!(persisted, (1, 1, 1, 1, 2, 0, 0));

    let replay_producer = TypedStageProducer::new();
    let replay = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: replacement_stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &replay_producer,
        )
        .await
        .expect("replay formal AU aggregate");
    assert_eq!(replay, outcome);
    assert_eq!(replay_producer.legacy_calls.load(Ordering::SeqCst), 0);
    assert_eq!(replay_producer.shard_calls.load(Ordering::SeqCst), 0);
    assert_eq!(replay_producer.synthesis_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
#[serial]
async fn formal_stage_controller_recovers_terminalized_post_submission_finalizer_in_place() {
    let fixture = RuntimeFixture::start_v2("formal-stage-finalization-retry", true).await;
    fixture.prepare_for_formal_stage_controller().await;
    fixture.insert_safe_projection_surface().await;
    let runner = FinalizationCommitFailureRunner {
        producer: TypedStageProducer::new(),
        pool: fixture.db.pool().clone(),
        armed: AtomicBool::new(false),
    };
    let runtime = golish_agent_app::ai::application_understanding_runtime::PgApplicationUnderstandingStageRuntime::new(
        std::sync::Arc::new(fixture.db.pool().clone()),
    );
    let replacement = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: fixture.fence.stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &runner,
        )
        .await
        .expect("replace legacy direct AU runtime");
    let ApplicationUnderstandingStageOutcome::Blocked { refs, .. } = replacement else {
        panic!("legacy direct AU runtime must be replaced")
    };
    let replacement_stage_execution_id = refs
        .iter()
        .find_map(|reference| reference.strip_prefix("replacement_stage_execution:"))
        .and_then(|value| Uuid::parse_str(value).ok())
        .expect("replacement execution identity");

    let failure = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: replacement_stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &runner,
        )
        .await
        .expect_err("injected current-authority commit failure must surface");
    assert!(format!("{failure:#}").contains("TEST_APPLICATION_MODEL_CURRENT_COMMIT_FAILED"));
    let parked: (String, String, String, i64, i64, i64) = sqlx::query_as(
        r#"SELECT unit.status,item.status,worker.status,
                  (SELECT count(*) FROM stage_deliverable_submissions submission
                    WHERE submission.stage_run_unit_id=unit.id),
                  (SELECT count(*) FROM application_model_revisions revision
                     JOIN application_model_manifests manifest ON manifest.id=revision.manifest_id
                    WHERE manifest.stage_run_unit_id=unit.id AND revision.status='proposed'),
                  (SELECT count(*) FROM application_model_current_revisions current_revision
                     JOIN application_model_manifests manifest ON manifest.id=current_revision.manifest_id
                    WHERE manifest.stage_run_unit_id=unit.id)
             FROM stage_run_units unit
             JOIN stage_team_plans plan ON plan.stage_run_unit_id=unit.id
             JOIN stage_work_items item
               ON item.team_plan_id=plan.id AND item.stable_key='leader:primary'
             JOIN stage_worker_runs worker ON worker.id=plan.final_submitter_worker_run_id
            WHERE unit.stage_execution_id=$1"#,
    )
    .bind(replacement_stage_execution_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("read parked Application Model finalizer");
    assert_eq!(
        parked,
        (
            "running".to_string(),
            "queued".to_string(),
            "queued".to_string(),
            1,
            1,
            0,
        ),
        "post-submission closeout failure must preserve the proposal and park the exact finalizer",
    );
    let shard_calls_after_failure = runner.producer.shard_calls.load(Ordering::SeqCst);

    let (terminal_unit_id, terminal_plan_id) = sqlx::query_as::<_, (Uuid, Uuid)>(
        r#"SELECT unit.id,plan.id
             FROM stage_run_units AS unit
             JOIN stage_team_plans AS plan ON plan.stage_run_unit_id=unit.id
            WHERE unit.stage_execution_id=$1"#,
    )
    .bind(replacement_stage_execution_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("load parked Application Model finalizer authority");
    let mut barrier_connection = fixture
        .db
        .pool()
        .acquire()
        .await
        .expect("acquire Application Model barrier connection");
    let terminal_barrier =
        stage_teams::load_barrier_with_connection(&mut barrier_connection, terminal_plan_id)
            .await
            .expect("load parked Application Model sibling barrier");
    drop(barrier_connection);
    let reclaimed = runtime_memory_tx::claim_stage_aggregator(
        fixture.db.pool(),
        &runtime_memory_tx::ClaimStageAggregatorRow {
            claim: runtime_memory_tx::ClaimStageWorkItemRow {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: replacement_stage_execution_id,
                stage_run_unit_id: terminal_unit_id,
                stage_team_plan_id: terminal_plan_id,
                exact_work_item_id: None,
                lease_owner: "terminalized-application-model-fixture".to_string(),
                lease_seconds: 300,
                session_id: fixture.session_id,
                subtask_id: None,
                agent: AgentType::Pentester,
                model: None,
                provider: None,
                parent_chain_id: None,
                initial_chain: json!([]),
                initial_checkpoint: json!({
                    "phase": "application_model_synthesis",
                    "schema_version": 1,
                }),
            },
            expected_dispatch_epoch: terminal_barrier.dispatch_epoch,
            expected_manifest_hash: terminal_barrier.manifest_hash,
        },
    )
    .await
    .expect("claim the parked Application Model finalizer before historical exhaustion");
    assert_eq!(reclaimed.work_item.status, "running");
    assert_eq!(reclaimed.worker.status, "running");

    // Model the shipped failure sequence: a later retry lost sight of the
    // earlier finished receipt, consumed the remaining producer attempt, and
    // terminalized the exact finalizer even though its durable submission and
    // proposed revision were still recoverable.
    let mut terminalized = fixture
        .db
        .pool()
        .begin()
        .await
        .expect("begin terminalized Application Model finalizer fixture");
    sqlx::query(
        r#"UPDATE stage_worker_runs AS worker
              SET status='failed',
                  checkpoint=jsonb_build_object(
                      'chain',worker.checkpoint,
                      'stage_team_execution_failure',jsonb_build_object(
                          'attempts_used',worker.attempt_epoch,
                          'code','application_model_closeout_blocked_before_submission',
                          'max_attempts',2,
                          'schema_version',1,
                          'stage','application_understanding_synthesis'
                      )
                  ),
                  checkpoint_version=checkpoint_version+1,
                  lease_token=NULL,lease_owner=NULL,lease_acquired_at=NULL,
                  lease_expires_at=NULL,heartbeat_at=NULL,terminal_at=NOW(),updated_at=NOW()
             FROM stage_team_plans AS plan
            WHERE plan.stage_run_unit_id IN (
                      SELECT id FROM stage_run_units WHERE stage_execution_id=$1
                  )
              AND worker.id=plan.final_submitter_worker_run_id
              AND worker.status='running'"#,
    )
    .bind(replacement_stage_execution_id)
    .execute(&mut *terminalized)
    .await
    .expect("terminalize exact Application Model finalizer Worker");
    sqlx::query(
        r#"UPDATE stage_work_items AS item
              SET status='exhausted',row_version=item.row_version+1,
                  terminal_at=NOW(),updated_at=NOW()
             FROM stage_team_plans AS plan
            WHERE plan.stage_run_unit_id IN (
                      SELECT id FROM stage_run_units WHERE stage_execution_id=$1
                  )
              AND item.team_plan_id=plan.id
              AND item.stable_key='leader:primary'
              AND item.status='running'"#,
    )
    .bind(replacement_stage_execution_id)
    .execute(&mut *terminalized)
    .await
    .expect("terminalize exact Application Model finalizer WorkItem");
    sqlx::query(
        r#"UPDATE stage_run_units
              SET status='gate_blocked',row_version=row_version+1,
                  terminal_at=NOW(),updated_at=NOW()
            WHERE stage_execution_id=$1 AND status='running'"#,
    )
    .bind(replacement_stage_execution_id)
    .execute(&mut *terminalized)
    .await
    .expect("terminalize exact Application Model Unit");
    terminalized
        .commit()
        .await
        .expect("commit terminalized Application Model finalizer fixture");

    sqlx::query(
        "DROP TRIGGER test_fail_application_model_current_insert ON application_model_current_revisions",
    )
    .execute(fixture.db.pool())
    .await
    .expect("drop deterministic finalization failure trigger");
    sqlx::query("DROP FUNCTION test_fail_application_model_current_insert()")
        .execute(fixture.db.pool())
        .await
        .expect("drop deterministic finalization failure function");

    let resumed = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: replacement_stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &runner,
        )
        .await
        .expect("recover the exact terminalized Application Model finalizer");
    assert_eq!(
        resumed,
        ApplicationUnderstandingStageOutcome::Passed {
            completed_units: 1,
            total_units: 1,
        }
    );
    assert_eq!(
        runner.producer.shard_calls.load(Ordering::SeqCst),
        shard_calls_after_failure,
        "closeout retry must not repeat any Application Model shard",
    );
    assert_eq!(runner.producer.synthesis_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
#[serial]
async fn formal_stage_controller_continues_other_companies_after_one_company_blocks() {
    let fixture = RuntimeFixture::start_v2_two_companies("formal-stage-two-companies", true).await;
    fixture.prepare_for_formal_stage_controller().await;
    let subsidiary_id = fixture
        .additional_organization_id
        .expect("two-company fixture subsidiary");
    let producer = TypedStageProducer::fail_one_company(fixture.organization_id);
    let runtime = golish_agent_app::ai::application_understanding_runtime::PgApplicationUnderstandingStageRuntime::new(
        std::sync::Arc::new(fixture.db.pool().clone()),
    );
    let replacement = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: fixture.fence.stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &producer,
        )
        .await
        .expect("replace two-company legacy direct AU runtime");
    let ApplicationUnderstandingStageOutcome::Blocked { refs, .. } = replacement else {
        panic!("legacy direct AU runtime must be replaced")
    };
    let replacement_stage_execution_id = refs
        .iter()
        .find_map(|reference| reference.strip_prefix("replacement_stage_execution:"))
        .and_then(|value| Uuid::parse_str(value).ok())
        .expect("replacement execution identity");

    let outcome = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: replacement_stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &producer,
        )
        .await
        .expect("drive every independent company before returning blockers");
    let ApplicationUnderstandingStageOutcome::Blocked { code, .. } = outcome else {
        panic!("one blocked company must block the aggregate stage")
    };
    assert_eq!(code, "APPLICATION_MODEL_WORK_ITEM_ATTEMPTS_EXHAUSTED");
    assert_eq!(producer.shard_calls.load(Ordering::SeqCst), 3);
    assert_eq!(producer.synthesis_calls.load(Ordering::SeqCst), 1);

    let unit_states = sqlx::query_as::<_, (Uuid, String)>(
        r#"SELECT organization_id,status
             FROM stage_run_units
            WHERE operation_id=$1 AND stage_execution_id=$2
            ORDER BY organization_id"#,
    )
    .bind(fixture.fence.operation_id)
    .bind(replacement_stage_execution_id)
    .fetch_all(fixture.db.pool())
    .await
    .expect("read independent company Unit states");
    assert_eq!(unit_states.len(), 2);
    assert!(unit_states
        .iter()
        .any(
            |(organization_id, status)| *organization_id == fixture.organization_id
                && status == "gate_blocked"
        ));
    assert!(unit_states
        .iter()
        .any(|(organization_id, status)| *organization_id == subsidiary_id && status == "passed"));
    let persisted: (i64, i64) = sqlx::query_as(
        r#"SELECT
              (SELECT count(*) FROM stage_team_plans
                WHERE operation_id=$1 AND stage_execution_id=$2),
              (SELECT count(*) FROM application_model_current_revisions AS current_revision
                 JOIN application_model_manifests AS manifest
                   ON manifest.id=current_revision.manifest_id
                WHERE manifest.operation_id=$1 AND manifest.stage_execution_id=$2)"#,
    )
    .bind(fixture.fence.operation_id)
    .bind(replacement_stage_execution_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("read per-company TeamPlan and publication counts");
    assert_eq!(persisted, (2, 1));

    let replay_producer = TypedStageProducer::new();
    let replay = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: replacement_stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &replay_producer,
        )
        .await
        .expect("replay terminal per-company states without provider work");
    let ApplicationUnderstandingStageOutcome::Blocked { code, .. } = replay else {
        panic!("persisted blocked company must continue to block aggregate PASS")
    };
    assert_eq!(code, "APPLICATION_MODEL_WORK_ITEM_BLOCKED");
    assert_eq!(replay_producer.shard_calls.load(Ordering::SeqCst), 0);
    assert_eq!(replay_producer.synthesis_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
#[serial]
async fn formal_stage_controller_rejects_missing_vuln_handoff_before_typed_producer() {
    let fixture = RuntimeFixture::start_v2("formal-stage-missing-handoff", false).await;
    fixture.prepare_for_formal_stage_controller().await;
    let producer = TypedStageProducer::new();
    let runtime = golish_agent_app::ai::application_understanding_runtime::PgApplicationUnderstandingStageRuntime::new(
        std::sync::Arc::new(fixture.db.pool().clone()),
    );
    let error = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: fixture.fence.stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &producer,
        )
        .await
        .expect_err("missing predecessor closure must fail closed");

    assert!(
        format!("{error:#}").contains("APPLICATION_UNDERSTANDING_PREDECESSOR_HANDOFF_INCOMPLETE")
    );
    assert_eq!(producer.legacy_calls.load(Ordering::SeqCst), 0);
    assert_eq!(producer.shard_calls.load(Ordering::SeqCst), 0);
    assert_eq!(producer.synthesis_calls.load(Ordering::SeqCst), 0);
    let authorities: i64 =
        sqlx::query_scalar("SELECT count(*) FROM application_model_current_revisions")
            .fetch_one(fixture.db.pool())
            .await
            .expect("count rejected formal AU authorities");
    assert_eq!(authorities, 0);
}

#[tokio::test]
#[serial]
async fn formal_stage_controller_retries_then_exhausts_invalid_synthesis_without_publication() {
    let fixture = RuntimeFixture::start_v2("formal-stage-rework", true).await;
    fixture.prepare_for_formal_stage_controller().await;
    let producer = TypedStageProducer::invalid();
    let runtime = golish_agent_app::ai::application_understanding_runtime::PgApplicationUnderstandingStageRuntime::new(
        std::sync::Arc::new(fixture.db.pool().clone()),
    );
    let replacement = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: fixture.fence.stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &producer,
        )
        .await
        .expect("replace legacy direct AU runtime");
    let ApplicationUnderstandingStageOutcome::Blocked { refs, .. } = replacement else {
        panic!("legacy direct AU runtime must be replaced")
    };
    let replacement_stage_execution_id = refs
        .iter()
        .find_map(|reference| reference.strip_prefix("replacement_stage_execution:"))
        .and_then(|value| Uuid::parse_str(value).ok())
        .expect("replacement execution identity");
    let outcome = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: replacement_stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &producer,
        )
        .await
        .expect("invalid typed synthesis returns deterministic BLOCK");

    let ApplicationUnderstandingStageOutcome::Blocked { code, refs } = outcome else {
        panic!("invalid typed draft must not pass")
    };
    assert_eq!(code, "APPLICATION_MODEL_SYNTHESIS_REWORK");
    assert!(refs
        .iter()
        .any(|reference| reference == "attempt_disposition:exhausted"));
    let exhausted_state: (String, String, String, i64, bool, i64) = sqlx::query_as(
        r#"SELECT unit.status,item.status,worker.status,
                  (SELECT count(*) FROM stage_worker_runs AS live
                    WHERE live.work_item_id=item.id
                      AND live.status='running' AND live.lease_token IS NOT NULL),
                  plan.final_submitter_worker_run_id=worker.id,
                  (SELECT count(*) FROM stage_deliverable_submissions AS submission
                    WHERE submission.stage_run_unit_id=unit.id)
             FROM stage_run_units AS unit
             JOIN stage_team_plans AS plan ON plan.stage_run_unit_id=unit.id
             JOIN stage_work_items AS item ON item.team_plan_id=plan.id
             JOIN stage_worker_runs AS worker ON worker.work_item_id=item.id
            WHERE unit.stage_execution_id=$1 AND item.stable_key='leader:primary'"#,
    )
    .bind(replacement_stage_execution_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("read exhausted AU synthesis attempt");
    assert_eq!(
        exhausted_state,
        (
            "gate_blocked".to_string(),
            "exhausted".to_string(),
            "failed".to_string(),
            0,
            true,
            0,
        ),
        "attempt exhaustion must terminalize the exact finalizer and Unit without publication",
    );
    assert_eq!(producer.legacy_calls.load(Ordering::SeqCst), 0);
    assert_eq!(producer.shard_calls.load(Ordering::SeqCst), 1);
    assert_eq!(producer.synthesis_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
#[serial]
async fn formal_stage_controller_retries_then_exhausts_synthesis_runner_error() {
    let fixture = RuntimeFixture::start_v2("formal-stage-runner-error", true).await;
    fixture.prepare_for_formal_stage_controller().await;
    let runner = SynthesisRunnerError {
        producer: TypedStageProducer::new(),
    };
    let runtime = golish_agent_app::ai::application_understanding_runtime::PgApplicationUnderstandingStageRuntime::new(
        std::sync::Arc::new(fixture.db.pool().clone()),
    );
    let replacement = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: fixture.fence.stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &runner,
        )
        .await
        .expect("replace legacy direct AU runtime");
    let ApplicationUnderstandingStageOutcome::Blocked { refs, .. } = replacement else {
        panic!("legacy direct AU runtime must be replaced")
    };
    let replacement_stage_execution_id = refs
        .iter()
        .find_map(|reference| reference.strip_prefix("replacement_stage_execution:"))
        .and_then(|value| Uuid::parse_str(value).ok())
        .expect("replacement execution identity");

    let outcome = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: replacement_stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &runner,
        )
        .await
        .expect("runner infrastructure failure must return a stable blocked outcome");
    let ApplicationUnderstandingStageOutcome::Blocked { code, refs } = outcome else {
        panic!("runner infrastructure failure must not pass")
    };
    assert_eq!(code, "APPLICATION_MODEL_SYNTHESIS_INFRASTRUCTURE");
    assert!(refs
        .iter()
        .any(|reference| reference == "attempt_disposition:exhausted"));
    assert!(refs
        .iter()
        .any(|reference| reference == "failure_code:application_model_producer_failed"));

    let state: (String, String, i64, i64, bool, i64) = sqlx::query_as(
        r#"SELECT unit.status,item.status,
                  (SELECT count(*) FROM stage_worker_runs AS worker
                    WHERE worker.work_item_id=item.id),
                  (SELECT count(*) FROM stage_worker_runs AS worker
                    WHERE worker.work_item_id=item.id
                      AND worker.status='running'
                      AND worker.lease_token IS NOT NULL),
                  plan.final_submitter_worker_run_id IS NOT NULL,
                  (SELECT count(*) FROM stage_deliverable_submissions AS submission
                    WHERE submission.stage_run_unit_id=unit.id)
             FROM stage_run_units AS unit
             JOIN stage_team_plans AS plan ON plan.stage_run_unit_id=unit.id
             JOIN stage_work_items AS item ON item.team_plan_id=plan.id
            WHERE unit.stage_execution_id=$1 AND item.stable_key='leader:primary'"#,
    )
    .bind(replacement_stage_execution_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("read synthesis runner failure recovery state");
    assert_eq!(
        state,
        (
            "gate_blocked".to_string(),
            "exhausted".to_string(),
            1,
            0,
            true,
            0,
        ),
        "runner error must not leave a live lease or publish a submission",
    );
    assert_eq!(runner.producer.shard_calls.load(Ordering::SeqCst), 1);
    assert_eq!(runner.producer.synthesis_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
#[serial]
async fn formal_stage_controller_exhausts_invalid_shard_with_durable_blocked_output() {
    let fixture = RuntimeFixture::start_v2("formal-stage-shard-exhaustion", true).await;
    fixture.prepare_for_formal_stage_controller().await;
    let producer = TypedStageProducer::shard_failure();
    let runtime = golish_agent_app::ai::application_understanding_runtime::PgApplicationUnderstandingStageRuntime::new(
        std::sync::Arc::new(fixture.db.pool().clone()),
    );
    let replacement = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: fixture.fence.stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &producer,
        )
        .await
        .expect("replace legacy direct AU runtime");
    let ApplicationUnderstandingStageOutcome::Blocked { refs, .. } = replacement else {
        panic!("legacy direct AU runtime must be replaced")
    };
    let replacement_stage_execution_id = refs
        .iter()
        .find_map(|reference| reference.strip_prefix("replacement_stage_execution:"))
        .and_then(|value| Uuid::parse_str(value).ok())
        .expect("replacement execution identity");

    let outcome = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: replacement_stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &producer,
        )
        .await
        .expect("exhaust invalid shard attempts");
    let ApplicationUnderstandingStageOutcome::Blocked { code, .. } = outcome else {
        panic!("exhausted shard must block synthesis")
    };
    assert_eq!(code, "APPLICATION_MODEL_WORK_ITEM_ATTEMPTS_EXHAUSTED");
    assert_eq!(producer.legacy_calls.load(Ordering::SeqCst), 0);
    assert_eq!(producer.shard_calls.load(Ordering::SeqCst), 2);
    assert_eq!(producer.synthesis_calls.load(Ordering::SeqCst), 0);
    let state: (String, i64, String) = sqlx::query_as(
        r#"SELECT item.status,
                  (SELECT count(*) FROM stage_worker_runs AS worker
                    WHERE worker.work_item_id=item.id),
                  output.business_disposition
             FROM stage_work_items AS item
             JOIN stage_worker_outputs AS output ON output.work_item_id=item.id
            WHERE item.stage_execution_id=$1 AND item.required_for_barrier"#,
    )
    .bind(replacement_stage_execution_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("read exhausted shard state");
    assert_eq!(state, ("exhausted".to_string(), 2, "blocked".to_string()));
}

#[tokio::test]
#[serial]
async fn formal_stage_controller_recovers_response_non_contract_exhaustion_with_bounded_no_purge_generations(
) {
    let fixture = RuntimeFixture::start_v2("formal-stage-shard-exhaustion-recovery", true).await;
    fixture.prepare_for_formal_stage_controller().await;
    let failing = TypedStageProducer::shard_failure();
    let runtime = golish_agent_app::ai::application_understanding_runtime::PgApplicationUnderstandingStageRuntime::new(
        std::sync::Arc::new(fixture.db.pool().clone()),
    );
    let legacy_replacement = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: fixture.fence.stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &failing,
        )
        .await
        .expect("replace legacy direct AU runtime");
    let ApplicationUnderstandingStageOutcome::Blocked { refs, .. } = legacy_replacement else {
        panic!("legacy direct AU runtime must be replaced")
    };
    let exhausted_stage_execution_id = refs
        .iter()
        .find_map(|reference| reference.strip_prefix("replacement_stage_execution:"))
        .and_then(|value| Uuid::parse_str(value).ok())
        .expect("legacy replacement execution identity");

    let exhausted = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: exhausted_stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &failing,
        )
        .await
        .expect("exhaust response-non-contract shard attempts");
    let ApplicationUnderstandingStageOutcome::Blocked { code, .. } = exhausted else {
        panic!("response-non-contract shard exhaustion must block")
    };
    assert_eq!(code, "APPLICATION_MODEL_WORK_ITEM_ATTEMPTS_EXHAUSTED");
    assert_eq!(failing.shard_calls.load(Ordering::SeqCst), 2);

    let recovered_runner = TypedStageProducer::new();
    let recovered = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: exhausted_stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &recovered_runner,
        )
        .await
        .expect("roll over exact exhausted AU runtime without purging facts");
    let ApplicationUnderstandingStageOutcome::Blocked { code, refs } = recovered else {
        panic!("runtime rollover must stop the current request")
    };
    assert_eq!(
        code,
        "APPLICATION_UNDERSTANDING_EXHAUSTED_RUNTIME_RECOVERED_CONTINUE_REQUIRED"
    );
    assert_eq!(recovered_runner.shard_calls.load(Ordering::SeqCst), 0);
    assert_eq!(recovered_runner.synthesis_calls.load(Ordering::SeqCst), 0);
    let replacement_stage_execution_id = refs
        .iter()
        .find_map(|reference| reference.strip_prefix("replacement_stage_execution:"))
        .and_then(|value| Uuid::parse_str(value).ok())
        .expect("exhausted runtime replacement execution identity");
    assert_ne!(replacement_stage_execution_id, exhausted_stage_execution_id);
    let rollover_state: (String, String, i32, i64, i64, i64) = sqlx::query_as(
        r#"SELECT source.status,replacement.status,replacement.generation,
                  (SELECT count(*) FROM stage_runs execution
                    WHERE execution.operation_id=$1 AND execution.status='started'),
                  (SELECT count(*) FROM stage_team_plans plan
                    WHERE plan.stage_execution_id=$3),
                  (SELECT count(*) FROM stage_worker_outputs output
                    WHERE output.stage_execution_id=$2)
             FROM stage_run_units source
             JOIN stage_run_units replacement
               ON replacement.operation_id=source.operation_id
              AND replacement.organization_id=source.organization_id
              AND replacement.stage_execution_id=$3
            WHERE source.operation_id=$1 AND source.stage_execution_id=$2"#,
    )
    .bind(fixture.fence.operation_id)
    .bind(exhausted_stage_execution_id)
    .bind(replacement_stage_execution_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("read no-purge AU runtime rollover state");
    assert_eq!(
        rollover_state,
        ("superseded".to_string(), "queued".to_string(), 1, 1, 0, 1,),
        "source output history is retained while one generation-1 runtime becomes active",
    );

    let compatibility_failure = TypedStageProducer::shard_failure();
    let replacement_exhausted = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: replacement_stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &compatibility_failure,
        )
        .await
        .expect("exhaust the first replacement after a provider-compatibility failure");
    let ApplicationUnderstandingStageOutcome::Blocked { code, .. } = replacement_exhausted else {
        panic!("first replacement exhaustion must remain blocked")
    };
    assert_eq!(code, "APPLICATION_MODEL_WORK_ITEM_ATTEMPTS_EXHAUSTED");
    assert_eq!(compatibility_failure.shard_calls.load(Ordering::SeqCst), 2);

    let compatibility_recovered = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: replacement_stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &compatibility_failure,
        )
        .await
        .expect("roll over the exact first replacement once for compatibility recovery");
    let ApplicationUnderstandingStageOutcome::Blocked { code, refs } = compatibility_recovered
    else {
        panic!("compatibility recovery must stop the current request")
    };
    assert_eq!(
        code,
        "APPLICATION_UNDERSTANDING_EXHAUSTED_RUNTIME_RECOVERED_CONTINUE_REQUIRED"
    );
    assert_eq!(compatibility_failure.shard_calls.load(Ordering::SeqCst), 2);
    let compatibility_stage_execution_id = refs
        .iter()
        .find_map(|reference| reference.strip_prefix("replacement_stage_execution:"))
        .and_then(|value| Uuid::parse_str(value).ok())
        .expect("compatibility replacement execution identity");
    assert_ne!(
        compatibility_stage_execution_id,
        replacement_stage_execution_id
    );
    let recovery_marker: (i32, i64, bool, String) = sqlx::query_as(
        r#"SELECT unit.generation,
                  (operation.state_blob #>> '{application_understanding_response_non_contract_recovery,recovery_count}')::BIGINT,
                  (operation.state_blob #>> '{application_understanding_response_non_contract_recovery,facts_purged}')::BOOLEAN,
                  operation.state_blob #>> '{application_understanding_response_non_contract_recovery,replacement_stage_execution_id}'
             FROM operation_state operation
             JOIN stage_run_units unit ON unit.operation_id=operation.operation_id
            WHERE operation.operation_id=$1 AND unit.stage_execution_id=$2"#,
    )
    .bind(fixture.fence.operation_id)
    .bind(compatibility_stage_execution_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("read bounded compatibility recovery marker");
    assert_eq!(
        recovery_marker,
        (2, 2, false, compatibility_stage_execution_id.to_string(),),
        "the second no-purge rollover consumes the final durable recovery fuel",
    );

    sqlx::query(
        "UPDATE stage_run_units SET generation=1
          WHERE operation_id=$1 AND stage_execution_id=$2
            AND status='queued' AND row_version=0",
    )
    .bind(fixture.fence.operation_id)
    .bind(compatibility_stage_execution_id)
    .execute(fixture.db.pool())
    .await
    .expect("simulate the pre-fix second-rollover generation");
    sqlx::query(
        "UPDATE operation_state
            SET state_blob=state_blob-'application_understanding_response_non_contract_recovery'
          WHERE operation_id=$1",
    )
    .bind(fixture.fence.operation_id)
    .execute(fixture.db.pool())
    .await
    .expect("simulate the pre-fix missing recovery marker");

    let repaired_exhausted = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: compatibility_stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &compatibility_failure,
        )
        .await
        .expect("repair and exhaust the pre-fix second-rollover generation");
    let ApplicationUnderstandingStageOutcome::Blocked { code, .. } = repaired_exhausted else {
        panic!("repaired second replacement exhaustion must remain blocked")
    };
    assert_eq!(code, "APPLICATION_MODEL_WORK_ITEM_ATTEMPTS_EXHAUSTED");
    assert_eq!(compatibility_failure.shard_calls.load(Ordering::SeqCst), 4);
    let repaired_generation: (i32, i64, bool) = sqlx::query_as(
        r#"SELECT unit.generation,
                  (operation.state_blob #>> '{application_understanding_response_non_contract_recovery,recovery_count}')::BIGINT,
                  (operation.state_blob #>> '{application_understanding_response_non_contract_recovery,legacy_generation_repaired}')::BOOLEAN
             FROM stage_run_units unit
             JOIN operation_state operation ON operation.operation_id=unit.operation_id
            WHERE unit.operation_id=$1 AND unit.stage_execution_id=$2"#,
    )
    .bind(fixture.fence.operation_id)
    .bind(compatibility_stage_execution_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("read repaired pre-fix second-rollover generation");
    assert_eq!(repaired_generation, (2, 2, true));

    let final_recovery = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: compatibility_stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &compatibility_failure,
        )
        .await
        .expect("roll over the exact second replacement for the final bounded recovery");
    let ApplicationUnderstandingStageOutcome::Blocked { code, refs } = final_recovery else {
        panic!("final bounded recovery must stop the current request")
    };
    assert_eq!(
        code,
        "APPLICATION_UNDERSTANDING_EXHAUSTED_RUNTIME_RECOVERED_CONTINUE_REQUIRED"
    );
    assert_eq!(compatibility_failure.shard_calls.load(Ordering::SeqCst), 4);
    let final_stage_execution_id = refs
        .iter()
        .find_map(|reference| reference.strip_prefix("replacement_stage_execution:"))
        .and_then(|value| Uuid::parse_str(value).ok())
        .expect("final bounded replacement execution identity");
    let final_marker: (i32, i64, i64, bool) = sqlx::query_as(
        r#"SELECT unit.generation,
                  (operation.state_blob #>> '{application_understanding_response_non_contract_recovery,recovery_count}')::BIGINT,
                  (operation.state_blob #>> '{application_understanding_response_non_contract_recovery,max_recoveries}')::BIGINT,
                  (operation.state_blob #>> '{application_understanding_response_non_contract_recovery,facts_purged}')::BOOLEAN
             FROM stage_run_units unit
             JOIN operation_state operation ON operation.operation_id=unit.operation_id
            WHERE unit.operation_id=$1 AND unit.stage_execution_id=$2"#,
    )
    .bind(fixture.fence.operation_id)
    .bind(final_stage_execution_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("read final bounded recovery marker");
    assert_eq!(final_marker, (3, 3, 3, false));

    let passed = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: final_stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &recovered_runner,
        )
        .await
        .expect("run the final bounded AU generation");
    assert_eq!(
        passed,
        ApplicationUnderstandingStageOutcome::Passed {
            completed_units: 1,
            total_units: 1,
        }
    );
    assert_eq!(recovered_runner.shard_calls.load(Ordering::SeqCst), 1);
    assert_eq!(recovered_runner.synthesis_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
#[serial]
async fn formal_stage_controller_exhausts_work_item_runner_error_without_live_lease() {
    let fixture = RuntimeFixture::start_v2("formal-stage-shard-runner-error", true).await;
    fixture.prepare_for_formal_stage_controller().await;
    let runner = WorkItemRunnerError {
        calls: AtomicUsize::new(0),
    };
    let runtime = golish_agent_app::ai::application_understanding_runtime::PgApplicationUnderstandingStageRuntime::new(
        std::sync::Arc::new(fixture.db.pool().clone()),
    );
    let replacement = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: fixture.fence.stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &runner,
        )
        .await
        .expect("replace legacy direct AU runtime");
    let ApplicationUnderstandingStageOutcome::Blocked { refs, .. } = replacement else {
        panic!("legacy direct AU runtime must be replaced")
    };
    let replacement_stage_execution_id = refs
        .iter()
        .find_map(|reference| reference.strip_prefix("replacement_stage_execution:"))
        .and_then(|value| Uuid::parse_str(value).ok())
        .expect("replacement execution identity");

    let outcome = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: replacement_stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &runner,
        )
        .await
        .expect("work-item runner errors must consume their bounded attempts");
    let ApplicationUnderstandingStageOutcome::Blocked { code, .. } = outcome else {
        panic!("work-item runner exhaustion must block synthesis")
    };
    assert_eq!(code, "APPLICATION_MODEL_WORK_ITEM_ATTEMPTS_EXHAUSTED");
    assert_eq!(runner.calls.load(Ordering::SeqCst), 2);
    let state: (String, String, i64, i64) = sqlx::query_as(
        r#"SELECT unit.status,item.status,
                  (SELECT count(*) FROM stage_worker_runs AS worker
                    WHERE worker.work_item_id=item.id),
                  (SELECT count(*) FROM stage_worker_runs AS worker
                    WHERE worker.work_item_id=item.id
                      AND worker.status='running' AND worker.lease_token IS NOT NULL)
             FROM stage_run_units AS unit
             JOIN stage_work_items AS item ON item.stage_run_unit_id=unit.id
            WHERE unit.stage_execution_id=$1 AND item.required_for_barrier"#,
    )
    .bind(replacement_stage_execution_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("read exhausted runner-error shard state");
    assert_eq!(
        state,
        ("gate_blocked".to_string(), "exhausted".to_string(), 2, 0),
        "runner errors must not escape the child retry transaction or leave a live lease",
    );
}

#[tokio::test]
#[serial]
async fn formal_stage_controller_exhausts_evidenceless_shard_without_leaving_a_live_lease() {
    let fixture = RuntimeFixture::start_v2("formal-stage-evidenceless-shard", true).await;
    fixture.prepare_for_formal_stage_controller().await;
    let producer = TypedStageProducer::evidenceless_shards();
    let runtime = golish_agent_app::ai::application_understanding_runtime::PgApplicationUnderstandingStageRuntime::new(
        std::sync::Arc::new(fixture.db.pool().clone()),
    );
    let replacement = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: fixture.fence.stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &producer,
        )
        .await
        .expect("replace legacy direct AU runtime");
    let ApplicationUnderstandingStageOutcome::Blocked { refs, .. } = replacement else {
        panic!("legacy direct AU runtime must be replaced")
    };
    let replacement_stage_execution_id = refs
        .iter()
        .find_map(|reference| reference.strip_prefix("replacement_stage_execution:"))
        .and_then(|value| Uuid::parse_str(value).ok())
        .expect("replacement execution identity");

    let outcome = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: replacement_stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &producer,
        )
        .await
        .expect("exhaust evidenceless shard attempts");
    let ApplicationUnderstandingStageOutcome::Blocked { code, refs } = outcome else {
        panic!("evidenceless shard must block synthesis")
    };
    assert_eq!(code, "APPLICATION_MODEL_WORK_ITEM_ATTEMPTS_EXHAUSTED");
    assert!(refs
        .iter()
        .any(|reference| reference == "failure_code:application_model_work_item_evidence_missing"));
    assert_eq!(producer.shard_calls.load(Ordering::SeqCst), 2);
    assert_eq!(producer.synthesis_calls.load(Ordering::SeqCst), 0);
    let state: (String, i64, i64, String) = sqlx::query_as(
        r#"SELECT item.status,
                  (SELECT count(*) FROM stage_worker_runs AS worker
                    WHERE worker.work_item_id=item.id),
                  (SELECT count(*) FROM stage_worker_runs AS worker
                    WHERE worker.work_item_id=item.id AND worker.status='running'),
                  output.business_disposition
             FROM stage_work_items AS item
             JOIN stage_worker_outputs AS output ON output.work_item_id=item.id
            WHERE item.stage_execution_id=$1 AND item.required_for_barrier"#,
    )
    .bind(replacement_stage_execution_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("read evidenceless shard terminal state");
    assert_eq!(
        state,
        ("exhausted".to_string(), 2, 0, "blocked".to_string())
    );
}

#[tokio::test]
#[serial]
async fn formal_stage_controller_holds_existing_live_lease_without_second_producer() {
    let fixture = RuntimeFixture::start_v2("formal-stage-live-lease", true).await;
    let producer = TypedStageProducer::new();
    let runtime = golish_agent_app::ai::application_understanding_runtime::PgApplicationUnderstandingStageRuntime::new(
        std::sync::Arc::new(fixture.db.pool().clone()),
    );
    let error = runtime
        .run(
            ApplicationUnderstandingStageRequest {
                operation_id: fixture.fence.operation_id,
                stage_execution_id: fixture.fence.stage_execution_id,
                session_id: fixture.session_id,
                stage_run_parent_request_id: "formal-au-stage-run".to_string(),
            },
            &producer,
        )
        .await
        .expect_err("live worker lease must be owned by only one controller");

    assert!(format!("{error:#}").contains("legacy_application_understanding_worker_not_quiescent"));
    assert_eq!(producer.legacy_calls.load(Ordering::SeqCst), 0);
    assert_eq!(producer.shard_calls.load(Ordering::SeqCst), 0);
    assert_eq!(producer.synthesis_calls.load(Ordering::SeqCst), 0);
}
