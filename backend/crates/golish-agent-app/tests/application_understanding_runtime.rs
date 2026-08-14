use golish_agent_app::ai::application_understanding_runtime::{
    run_application_understanding_unit, ApplicationModelProducerInput,
    ApplicationModelProposalDraft, ApplicationModelProposalProducer,
    ApplicationUnderstandingRuntimeError, ApplicationUnderstandingRuntimeOutcome,
    RunApplicationUnderstandingUnit,
};
use golish_agent_kit::harness::application_model_gate::{
    ApplicationModelGateCode, ApplicationModelGateDisposition,
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
use golish_db::models::{AgentType, NewSession};
use golish_db::repo::application_models::{
    ApplicationModelEvidenceRoleRow, ApplicationModelInputDecisionSeed,
    ApplicationModelInputDispositionRow, ApplicationModelItemEvidenceSeed,
    ApplicationModelItemSeed, ApplicationModelTruthStateRow, DeriveApplicationModelManifestSeed,
    LoadApplicationModelGateMaterial,
};
use golish_db::repo::{
    application_models, project_scopes, runtime_memory_tx, sessions, stage_deliverable_submissions,
    stage_teams, tool_calls,
};
use golish_db::{DbConfig, GolishDb};
use serde_json::json;
use serial_test::serial;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tempfile::TempDir;
use uuid::Uuid;

fn assert_send_sync<T: Send + Sync>() {}

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
}

#[derive(Debug, Clone, Copy)]
struct RuntimeSourceFixture {
    stage_execution_id: Uuid,
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
