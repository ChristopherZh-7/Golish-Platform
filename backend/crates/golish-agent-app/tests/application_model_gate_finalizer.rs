use chrono::Utc;
use golish_agent_app::ai::application_model_gate::{
    build_application_model_gate_snapshot, evaluate_application_model_content_precheck,
    finalize_application_model_gate_pass, ApplicationModelFinalizationOutcome,
    ApplicationModelGatePrecheckEvaluation, FinalizeApplicationModelGatePass,
};
use golish_agent_kit::harness::{
    validate_application_model_gate_truth, ApplicationModelAuthorityKind,
    ApplicationModelInputDisposition, ApplicationModelTruthState,
};
use golish_db::models::NewSession;
use golish_db::repo::application_models::{
    self, ApplicationModelAuthorityKindRow, ApplicationModelEvidenceRoleRow,
    ApplicationModelGateMaterial, ApplicationModelInputDecisionRow,
    ApplicationModelInputDecisionSeed, ApplicationModelInputDispositionRow,
    ApplicationModelItemEvidenceRow, ApplicationModelItemEvidenceSeed, ApplicationModelItemRow,
    ApplicationModelItemSeed, ApplicationModelManifestInputRow, ApplicationModelManifestInputSeed,
    ApplicationModelManifestRow, ApplicationModelRevisionRow, ApplicationModelTruthStateRow,
    ProposeApplicationModelRevision, SeedApplicationModelManifest,
};
use golish_db::repo::{project_scopes, runtime_memory_tx, sessions};
use golish_db::{DbConfig, GolishDb};
use serde_json::json;
use serial_test::serial;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use uuid::Uuid;

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn application_model_gate_public_adapter_and_finalizer_contract_exists() {
    assert_send_sync::<ApplicationModelGatePrecheckEvaluation>();
    assert_send_sync::<FinalizeApplicationModelGatePass>();
    let _ = evaluate_application_model_content_precheck;
    let _ = finalize_application_model_gate_pass;
}

fn model_material() -> ApplicationModelGateMaterial {
    let manifest_id = Uuid::new_v4();
    let operation_id = Uuid::new_v4();
    let scope_snapshot_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    let stage_run_unit_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let source_handoff_id = Uuid::new_v4();
    ApplicationModelGateMaterial {
        manifest: ApplicationModelManifestRow {
            id: manifest_id,
            operation_id,
            scope_snapshot_id,
            stage_execution_id,
            stage_run_unit_id,
            organization_id,
            stage_kind: "application_understanding".to_string(),
            authority_kind: "model".to_string(),
            input_count: 1,
            manifest_hash:
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
            replay_material_hash:
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
            row_version: 0,
            frozen_at: Utc::now(),
        },
        inputs: vec![ApplicationModelManifestInputRow {
            input_key: "api:/orders/{id}".to_string(),
            ordinal: 0,
            input_kind: "stage_handoff".to_string(),
            source_handoff_id,
            source_kind: "vulnerability_analysis".to_string(),
            source_id: source_handoff_id.to_string(),
            source_version: 1,
            source_payload: json!({"routes": ["/orders/{id}"]}),
            source_payload_hash:
                "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                    .to_string(),
            evidence_ids: vec![41],
        }],
        revision: Some(ApplicationModelRevisionRow {
            id: revision_id,
            manifest_id,
            operation_id,
            scope_snapshot_id,
            stage_execution_id,
            stage_run_unit_id,
            organization_id,
            revision_ordinal: 1,
            stage_kind: "application_understanding".to_string(),
            schema_version: "application_model.v1".to_string(),
            status: "proposed".to_string(),
            structured_model: json!({"workflows": ["order_read"]}),
            model_hash: "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                .to_string(),
            replay_material_hash:
                "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
                    .to_string(),
            source_submission_id: Uuid::new_v4(),
            row_version: 0,
            created_at: Utc::now(),
            finalized_at: None,
        }),
        decisions: vec![ApplicationModelInputDecisionRow {
            revision_id,
            manifest_id,
            input_key: "api:/orders/{id}".to_string(),
            disposition: "incorporated".to_string(),
            item_keys: vec!["workflow:order_read".to_string()],
            duplicate_input_key: None,
            reason_code: None,
        }],
        items: vec![ApplicationModelItemRow {
            revision_id,
            manifest_id,
            item_key: "workflow:order_read".to_string(),
            ordinal: 0,
            item_kind: "workflow".to_string(),
            truth_state: "observed".to_string(),
            source_input_keys: vec!["api:/orders/{id}".to_string()],
            referenced_item_keys: Vec::new(),
            payload: json!({"name": "order_read"}),
            payload_hash: "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
                .to_string(),
        }],
        item_evidence: vec![ApplicationModelItemEvidenceRow {
            revision_id,
            manifest_id,
            item_key: "workflow:order_read".to_string(),
            evidence_id: 41,
            role: "observation".to_string(),
        }],
        forbidden_activity_refs: Vec::new(),
        pending_producer_refs: Vec::new(),
    }
}

#[test]
fn application_model_gate_adapter_recomputes_hashes_and_maps_truth_roles() {
    let mut material = model_material();
    let recomputed = build_application_model_gate_snapshot(&material).unwrap();
    assert_ne!(
        recomputed.manifest_hash, recomputed.expected_manifest_hash,
        "the adapter must not copy the persisted manifest hash into expected truth"
    );
    assert_ne!(
        recomputed.model_hash, recomputed.expected_model_hash,
        "the adapter must recompute the model hash"
    );
    material.manifest.manifest_hash = recomputed.expected_manifest_hash;
    material.manifest.replay_material_hash = material.manifest.manifest_hash.clone();
    let revision = material.revision.as_mut().unwrap();
    revision.model_hash = recomputed.expected_model_hash.unwrap();
    revision.replay_material_hash = recomputed.expected_replay_material_hash;

    let snapshot = build_application_model_gate_snapshot(&material).unwrap();

    assert_eq!(
        snapshot.authority_kind,
        ApplicationModelAuthorityKind::Model
    );
    assert_eq!(
        snapshot.decisions[0].disposition,
        ApplicationModelInputDisposition::Incorporated
    );
    assert_eq!(
        snapshot.items[0].truth_state,
        ApplicationModelTruthState::Observed
    );
    assert_eq!(snapshot.items[0].observed_evidence_ids, vec![41]);
    assert_eq!(validate_application_model_gate_truth(&snapshot), Ok(()));
}

#[test]
fn application_model_gate_adapter_keeps_terminal_no_input_explicit() {
    let mut material = model_material();
    material.manifest.authority_kind = "terminal_no_input".to_string();
    material.manifest.input_count = 0;
    material.inputs.clear();
    material.revision = None;
    material.decisions.clear();
    material.items.clear();
    material.item_evidence.clear();
    let recomputed = build_application_model_gate_snapshot(&material).unwrap();
    material.manifest.manifest_hash = recomputed.expected_manifest_hash;
    material.manifest.replay_material_hash = material.manifest.manifest_hash.clone();

    let snapshot = build_application_model_gate_snapshot(&material).unwrap();

    assert_eq!(
        snapshot.authority_kind,
        ApplicationModelAuthorityKind::TerminalNoInput
    );
    assert!(snapshot.schema_version.is_none());
    assert!(snapshot.model_hash.is_none());
    assert_eq!(validate_application_model_gate_truth(&snapshot), Ok(()));
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

struct FinalizerFixture {
    db: GolishDb,
    _data_dir: TempDir,
    organization_id: Uuid,
    source_handoff_id: Uuid,
    manifest_id: Uuid,
    revision_id: Uuid,
    submission_id: Uuid,
    scope_hash: String,
    fence: runtime_memory_tx::RuntimeMemoryTxFence,
}

impl FinalizerFixture {
    async fn start(label: &str) -> Self {
        Self::start_inner(label, false, None).await
    }

    async fn start_with_pending_work_item(label: &str) -> Self {
        Self::start_inner(label, true, None).await
    }

    async fn start_with_au_submit_result(
        label: &str,
        contract: AuSubmitResultWorkerContract<'_>,
    ) -> Self {
        Self::start_inner(label, false, Some(contract)).await
    }

    async fn start_inner(
        label: &str,
        seed_pending_work_item: bool,
        au_submit_result_contract: Option<AuSubmitResultWorkerContract<'_>>,
    ) -> Self {
        let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
        let config = DbConfig {
            pg_data_dir: data_dir.path().join("pgdata"),
            port: reserve_local_port(),
            database: format!("application_model_app_{label}_{}", Uuid::new_v4().simple()),
            ..DbConfig::default()
        };
        let db = GolishDb::start(config)
            .await
            .expect("start fresh migrated embedded postgres");
        if seed_pending_work_item || au_submit_result_contract.is_some() {
            let mut rollout_tx = db.pool().begin().await.expect("begin V2 rollout fixture");
            sqlx::query("SET LOCAL session_replication_role = 'replica'")
                .execute(&mut *rollout_tx)
                .await
                .expect("isolate rollout promotion fixture");
            sqlx::query(
                "UPDATE runtime_memory_rollout \
                 SET contract='v2_only',contract_rank=3,row_version=3,updated_at=NOW() \
                 WHERE singleton_id=1",
            )
            .execute(&mut *rollout_tx)
            .await
            .expect("position runtime rollout at V2-only");
            sqlx::query(
                "UPDATE attack_execution_rollout \
                 SET contract='v2_only',rank=3,row_version=3,updated_at=NOW() \
                 WHERE singleton=TRUE",
            )
            .execute(&mut *rollout_tx)
            .await
            .expect("position attack rollout at V2-only");
            rollout_tx
                .commit()
                .await
                .expect("commit V2 rollout fixture");
        }
        let workspace = format!("/tmp/application-model-app-{}", Uuid::new_v4().simple());
        let project = project_scopes::register_first_open(db.pool(), &workspace, &"1".repeat(64))
            .await
            .expect("register finalizer project");
        let organization_id = Uuid::new_v4();
        sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$2,'Model Org')")
            .bind(organization_id)
            .bind(&workspace)
            .execute(db.pool())
            .await
            .expect("insert finalizer organization");
        let session = sessions::create(
            db.pool(),
            NewSession {
                title: Some("Application Model finalizer fixture".to_string()),
                workspace_path: Some(workspace.clone()),
                workspace_label: None,
                model: Some("fixture-model".to_string()),
                provider: Some("fixture-provider".to_string()),
                project_path: Some(workspace.clone()),
            },
        )
        .await
        .expect("create finalizer session");
        let operation_id = Uuid::new_v4();
        let stage_execution_id = Uuid::new_v4();
        runtime_memory_tx::create_runtime_operation(
            db.pool(),
            &runtime_memory_tx::CreateRuntimeOperationRow {
                operation_id,
                initial_stage_execution_id: stage_execution_id,
                session_id: session.id,
                title: Some("Application Model finalizer fixture".to_string()),
                input: "fixture".to_string(),
                profile: "red_team".to_string(),
                entry_stage: "application_understanding".to_string(),
                application_model_contract:
                    golish_core::ApplicationModelContract::ApplicationModelV1,
                project_scope_id: project.project_scope_id,
                cli_scope: Some(runtime_memory_tx::CliRuntimeScopeRow {
                    root_organization_id: organization_id,
                    include_subsidiaries: false,
                    subsidiary_threshold: 51,
                    units: vec![runtime_memory_tx::CliRuntimeScopeUnitRow {
                        organization_id,
                        parent_organization_id: None,
                        organization_name: "Model Org".to_string(),
                        depth: 0,
                        ordinal: 0,
                        ownership_percent: None,
                        approval_source: json!({"kind": "fixture"}),
                    }],
                }),
            },
        )
        .await
        .expect("create exact V2 finalizer operation");
        let (scope_snapshot_id, scope_hash) = sqlx::query_as::<_, (Uuid, String)>(
            "SELECT id,scope_hash FROM operation_org_scope_snapshots WHERE operation_id=$1",
        )
        .bind(operation_id)
        .fetch_one(db.pool())
        .await
        .expect("read frozen finalizer scope");

        let source_execution_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO stage_runs(id,operation_id,stage_kind,status) \
             VALUES($1,$2,'vuln_triage','completed')",
        )
        .bind(source_execution_id)
        .bind(operation_id)
        .execute(db.pool())
        .await
        .expect("insert completed source execution");
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
        .expect("insert inherited source evidence");
        let source_payload = json!({
            "schema_version": 1,
            "organization_id": organization_id,
            "routes": ["/orders/{id}"],
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
                        'main>vuln_triage','running',$6,'source-fixture',
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
               ) VALUES($1,'source-submit',$2,$3,'primary','submit_stage_deliverable',
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
        .expect("insert source submission tool call");
        sqlx::query(
            r#"INSERT INTO stage_deliverable_submissions(
                   id,operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
                   organization_id,tool_call_record_id,tool_request_id,stage_kind,
                   attempt_epoch,lease_token,payload,payload_sha256
               ) VALUES($1,$2,$3,$4,$5,$6,$7,'source-submit','vuln_triage',0,$8,$9,$10)"#,
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
            "UPDATE stage_worker_runs SET status='passed',terminal_at=NOW(),updated_at=NOW() \
             WHERE id=$1",
        )
        .bind(source_worker_id)
        .execute(db.pool())
        .await
        .expect("pass source worker");
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
        .bind(&source_payload)
        .bind(sha256_json(&source_payload))
        .bind(vec![evidence_id])
        .bind(json!({"complete": true}))
        .bind("5".repeat(64))
        .execute(db.pool())
        .await
        .expect("insert source handoff");

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
        let stage_team = if seed_pending_work_item {
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
                            2,1,FALSE,'{"coordination_mode":"company_controller"}'::JSONB,
                            0,'worker',$9)"#,
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
            .expect("insert pending-work fixture TeamPlan");
            sqlx::query(
                r#"INSERT INTO stage_work_items(
                       id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                       scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
                       input_manifest_hash,input_refs,required_for_barrier,conflict_key,priority,status,
                       attempt_policy,budget,output_schema,created_by,started_at
                   ) VALUES($1,$2,$3,$4,$5,$6,$7,0,'stage_unit','leader:primary',
                            'application_understanding',$8,'[]'::JSONB,FALSE,
                            'stage_unit_finalizer',0,'running',
                            '{}'::JSONB,'{}'::JSONB,'application_model.v1','server_seed',NOW())"#,
            )
            .bind(submitter_item_id)
            .bind(plan_id)
            .bind(operation_id)
            .bind(stage_execution_id)
            .bind(stage_run_unit_id)
            .bind(scope_snapshot_id)
            .bind(organization_id)
            .bind(format!("sha256:{}", "9".repeat(64)))
            .execute(db.pool())
            .await
            .expect("insert submitter WorkItem");
            sqlx::query(
                r#"INSERT INTO stage_work_items(
                       id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                       scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
                       input_manifest_hash,input_refs,required_for_barrier,priority,status,
                       attempt_policy,budget,output_schema,created_by
                   ) VALUES($1,$2,$3,$4,$5,$6,$7,0,'late_analysis','pending-analysis',
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
            .bind(format!("sha256:{}", "a".repeat(64)))
            .execute(db.pool())
            .await
            .expect("insert pending WorkItem");
            Some((plan_id, submitter_item_id))
        } else if let Some(contract) = au_submit_result_contract {
            let plan_id = Uuid::new_v4();
            let submitter_item_id = Uuid::new_v4();
            let contract_item_id = Uuid::new_v4();
            let contract_worker_id = Uuid::new_v4();
            let contract_worker_lease = Uuid::new_v4();
            let contract_tool_call_id = Uuid::new_v4();
            let contract_kind = if contract.role == "application_model_synthesizer" {
                "application_model_synthesis"
            } else {
                "application_model_analysis"
            };
            let contract_stable_key = format!("au-submit-result-{}", Uuid::new_v4().simple());
            let mut allowed_worker_roles = vec![
                "application_understanding".to_string(),
                contract.role.to_string(),
            ];
            allowed_worker_roles.sort();
            allowed_worker_roles.dedup();
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
                            2,2,FALSE,$9,0,'worker',$10)"#,
            )
            .bind(plan_id)
            .bind(operation_id)
            .bind(stage_execution_id)
            .bind(stage_run_unit_id)
            .bind(scope_snapshot_id)
            .bind(organization_id)
            .bind(format!("sha256:{}", "b".repeat(64)))
            .bind(json!(allowed_worker_roles))
            .bind(json!({
                "coordination_mode": "company_controller",
                "formulaic_worklist_executor": "application_model_v1",
            }))
            .bind(format!("sha256:{}", "c".repeat(64)))
            .execute(db.pool())
            .await
            .expect("insert AU submit_result TeamPlan");
            sqlx::query(
                r#"INSERT INTO stage_work_items(
                       id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                       scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
                       input_manifest_hash,input_refs,required_for_barrier,conflict_key,priority,status,
                       attempt_policy,budget,output_schema,created_by,started_at
                   ) VALUES($1,$2,$3,$4,$5,$6,$7,0,'stage_unit','leader:primary',
                            'application_understanding',$8,'[]'::JSONB,FALSE,
                            'stage_unit_finalizer',0,'running',
                            '{}'::JSONB,'{}'::JSONB,'application_model.v1','server_seed',NOW())"#,
            )
            .bind(submitter_item_id)
            .bind(plan_id)
            .bind(operation_id)
            .bind(stage_execution_id)
            .bind(stage_run_unit_id)
            .bind(scope_snapshot_id)
            .bind(organization_id)
            .bind(format!("sha256:{}", "d".repeat(64)))
            .execute(db.pool())
            .await
            .expect("insert AU final submitter WorkItem");
            sqlx::query(
                r#"INSERT INTO stage_work_items(
                       id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                       scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
                       input_manifest_hash,input_refs,required_for_barrier,priority,status,
                       attempt_policy,budget,output_schema,created_by,started_at,terminal_at
                   ) VALUES($1,$2,$3,$4,$5,$6,$7,0,$8,$9,$10,$11,'[]'::JSONB,TRUE,0,
                            'completed','{}'::JSONB,'{}'::JSONB,$12,'server_seed',NOW(),NOW())"#,
            )
            .bind(contract_item_id)
            .bind(plan_id)
            .bind(operation_id)
            .bind(stage_execution_id)
            .bind(stage_run_unit_id)
            .bind(scope_snapshot_id)
            .bind(organization_id)
            .bind(contract_kind)
            .bind(&contract_stable_key)
            .bind(contract.role)
            .bind(format!("sha256:{}", "e".repeat(64)))
            .bind(contract.output_schema)
            .execute(db.pool())
            .await
            .expect("insert AU submit_result WorkItem");
            sqlx::query(
                r#"INSERT INTO stage_worker_runs(
                       id,operation_id,stage_execution_id,stage_run_unit_id,
                       organization_id,worker_generation,specialist,work_item_kind,
                       work_item_key,agent_path,status,lease_token,lease_owner,
                       lease_acquired_at,lease_expires_at,heartbeat_at,attempt_epoch,
                       work_item_id,started_at,terminal_at
                   ) VALUES($1,$2,$3,$4,$5,0,$6,$7,$8,
                            'main>application-understanding-contract','passed',$9,
                            'au-submit-result-fixture',NOW(),NOW()+INTERVAL '5 minutes',NOW(),
                            0,$10,NOW(),NOW())"#,
            )
            .bind(contract_worker_id)
            .bind(operation_id)
            .bind(stage_execution_id)
            .bind(stage_run_unit_id)
            .bind(organization_id)
            .bind(contract.role)
            .bind(contract_kind)
            .bind(&contract_stable_key)
            .bind(contract_worker_lease)
            .bind(contract_item_id)
            .execute(db.pool())
            .await
            .expect("insert AU submit_result Worker");
            sqlx::query(
                r#"INSERT INTO tool_calls(
                       id,call_id,session_id,task_id,agent,name,args,result,status,
                       operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
                       organization_id,attempt_epoch,lease_token
                   ) VALUES($1,$2,$3,$4,'primary','submit_result','{}','{}','finished',
                            $4,$5,$6,$7,$8,0,$9)"#,
            )
            .bind(contract_tool_call_id)
            .bind(format!(
                "application-model-submit-result-{contract_tool_call_id}"
            ))
            .bind(session.id)
            .bind(operation_id)
            .bind(stage_execution_id)
            .bind(stage_run_unit_id)
            .bind(contract_worker_id)
            .bind(organization_id)
            .bind(contract_worker_lease)
            .execute(db.pool())
            .await
            .expect("insert AU submit_result receipt");
            sqlx::query(
                r#"INSERT INTO stage_worker_outputs(
                       id,team_plan_id,work_item_id,worker_run_id,operation_id,
                       stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id,
                       output_schema,output_version,business_disposition,canonical_output,
                       canonical_fact_refs,evidence_ids,checked_empty_cells,blocker_codes,output_hash
                   ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,1,'found',$11,
                            '[]'::JSONB,ARRAY[]::BIGINT[],'[]'::JSONB,ARRAY[]::TEXT[],$12)"#,
            )
            .bind(Uuid::new_v4())
            .bind(plan_id)
            .bind(contract_item_id)
            .bind(contract_worker_id)
            .bind(operation_id)
            .bind(stage_execution_id)
            .bind(stage_run_unit_id)
            .bind(scope_snapshot_id)
            .bind(organization_id)
            .bind(contract.output_schema)
            .bind(json!({"status": "completed"}))
            .bind(format!("sha256:{}", "f".repeat(64)))
            .execute(db.pool())
            .await
            .expect("insert AU submit_result WorkerOutput");
            Some((plan_id, submitter_item_id))
        } else {
            None
        };
        let seeded = application_models::seed_manifest(
            db.pool(),
            &SeedApplicationModelManifest {
                operation_id,
                scope_snapshot_id,
                stage_execution_id,
                stage_run_unit_id,
                organization_id,
                authority_kind: ApplicationModelAuthorityKindRow::Model,
                inputs: vec![ApplicationModelManifestInputSeed {
                    input_key: "vuln-handoff".to_string(),
                    input_kind: "stage_handoff".to_string(),
                    source_handoff_id,
                    source_kind: "vuln_triage".to_string(),
                    source_id: source_handoff_id.to_string(),
                    source_version: 1,
                    source_payload: source_payload.clone(),
                    evidence_ids: vec![evidence_id],
                }],
            },
        )
        .await
        .expect("seed finalizer manifest");
        let worker_run_id = Uuid::new_v4();
        let lease_token = Uuid::new_v4();
        let submission_tool_id = Uuid::new_v4();
        let submission_id = Uuid::new_v4();
        let final_submitter_work_item_key = stage_team
            .map(|_| "leader:primary")
            .unwrap_or("application_understanding");
        let structured_model = json!({
            "organization_id": organization_id,
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
        });
        let submission_payload = json!({
            "stage_id": "application_understanding",
            "stage_run_id": stage_execution_id,
            "schema_version": 1,
            "manifest_id": seeded.manifest.id,
            "structured_model": structured_model,
            "decisions": [{
                "input_key": "vuln-handoff",
                "disposition": "incorporated",
                "item_keys": ["workflow:order_read"],
                "duplicate_input_key": null,
                "reason_code": null,
            }],
            "items": [{
                "item_key": "workflow:order_read",
                "item_kind": "workflow",
                "truth_state": "observed",
                "source_input_keys": ["vuln-handoff"],
                "referenced_item_keys": [],
                "payload": {"method": "GET", "path": "/orders/{id}"},
                "evidence": [{"evidence_id": evidence_id, "role": "observation"}],
            }],
        });
        sqlx::query(
            r#"INSERT INTO stage_worker_runs(
                   id,operation_id,stage_execution_id,stage_run_unit_id,
                   organization_id,worker_generation,specialist,work_item_kind,
                   work_item_key,agent_path,status,lease_token,lease_owner,
                   lease_acquired_at,lease_expires_at,heartbeat_at,attempt_epoch,work_item_id
               ) VALUES($1,$2,$3,$4,$5,0,'application_understanding','stage_unit',
                        $7,'main>application_understanding','running',
                        $6,'application-model-fixture',NOW(),NOW()+INTERVAL '5 minutes',NOW(),0,$8)"#,
        )
        .bind(worker_run_id)
        .bind(operation_id)
        .bind(stage_execution_id)
        .bind(stage_run_unit_id)
        .bind(organization_id)
        .bind(lease_token)
        .bind(final_submitter_work_item_key)
        .bind(stage_team.map(|(_, work_item_id)| work_item_id))
        .execute(db.pool())
        .await
        .expect("insert live finalizer worker");
        sqlx::query(
            r#"INSERT INTO tool_calls(
                   id,call_id,session_id,task_id,agent,name,args,result,status,
                   operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
                   organization_id,attempt_epoch,lease_token
               ) VALUES($1,'model-submit',$2,$3,'primary','submit_stage_deliverable',
                        '{}','{}','finished',$3,$4,$5,$6,$7,0,$8)"#,
        )
        .bind(submission_tool_id)
        .bind(session.id)
        .bind(operation_id)
        .bind(stage_execution_id)
        .bind(stage_run_unit_id)
        .bind(worker_run_id)
        .bind(organization_id)
        .bind(lease_token)
        .execute(db.pool())
        .await
        .expect("insert model submission tool call");
        if let Some((plan_id, _)) = stage_team {
            sqlx::query(
                "UPDATE stage_worker_runs \
                 SET active_tool_call_id=$2,active_tool_started_at=NOW(),updated_at=NOW() \
                 WHERE id=$1",
            )
            .bind(worker_run_id)
            .bind(submission_tool_id)
            .execute(db.pool())
            .await
            .expect("mark Team submit tool active");
            sqlx::query(
                r#"UPDATE stage_team_plans
                      SET requests_closed_at=NOW(),final_submitter_worker_run_id=$2,
                          row_version=row_version+1,updated_at=NOW()
                    WHERE id=$1"#,
            )
            .bind(plan_id)
            .bind(worker_run_id)
            .execute(db.pool())
            .await
            .expect("bind Team final submitter");
        }
        sqlx::query(
            r#"INSERT INTO stage_deliverable_submissions(
                   id,operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
                   organization_id,tool_call_record_id,tool_request_id,stage_kind,
                   attempt_epoch,lease_token,payload,payload_sha256
               ) VALUES($1,$2,$3,$4,$5,$6,$7,'model-submit',
                        'application_understanding',0,$8,$9,$10)"#,
        )
        .bind(submission_id)
        .bind(operation_id)
        .bind(stage_execution_id)
        .bind(stage_run_unit_id)
        .bind(worker_run_id)
        .bind(organization_id)
        .bind(submission_tool_id)
        .bind(lease_token)
        .bind(&submission_payload)
        .bind(sha256_json(&submission_payload))
        .execute(db.pool())
        .await
        .expect("insert model submission");
        if stage_team.is_some() {
            sqlx::query(
                "UPDATE stage_worker_runs \
                 SET active_tool_call_id=NULL,active_tool_started_at=NULL,updated_at=NOW() \
                 WHERE id=$1",
            )
            .bind(worker_run_id)
            .execute(db.pool())
            .await
            .expect("clear Team submit tool activity");
        }
        let proposed = application_models::propose_revision(
            db.pool(),
            &ProposeApplicationModelRevision {
                manifest_id: seeded.manifest.id,
                operation_id,
                scope_snapshot_id,
                stage_execution_id,
                stage_run_unit_id,
                organization_id,
                source_submission_id: submission_id,
                structured_model,
                decisions: vec![ApplicationModelInputDecisionSeed {
                    input_key: "vuln-handoff".to_string(),
                    disposition: ApplicationModelInputDispositionRow::Incorporated,
                    item_keys: vec!["workflow:order_read".to_string()],
                    duplicate_input_key: None,
                    reason_code: None,
                }],
                items: vec![ApplicationModelItemSeed {
                    item_key: "workflow:order_read".to_string(),
                    item_kind: "workflow".to_string(),
                    truth_state: ApplicationModelTruthStateRow::Observed,
                    source_input_keys: vec!["vuln-handoff".to_string()],
                    referenced_item_keys: Vec::new(),
                    payload: json!({"method": "GET", "path": "/orders/{id}"}),
                    evidence: vec![ApplicationModelItemEvidenceSeed {
                        evidence_id,
                        role: ApplicationModelEvidenceRoleRow::Observation,
                    }],
                }],
            },
        )
        .await
        .expect("propose exact model revision");
        Self {
            db,
            _data_dir: data_dir,
            organization_id,
            source_handoff_id,
            manifest_id: seeded.manifest.id,
            revision_id: proposed.revision.id,
            submission_id,
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

    fn command(&self) -> FinalizeApplicationModelGatePass {
        FinalizeApplicationModelGatePass {
            fence: self.fence.clone(),
            deliverable_submission_id: self.submission_id,
            manifest_id: self.manifest_id,
            expected_unit_row_version: 0,
            scope_hash: self.scope_hash.clone(),
        }
    }
}

async fn assert_application_model_unpublished(fixture: &FinalizerFixture) {
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT status FROM application_model_revisions WHERE id=$1",
        )
        .bind(fixture.revision_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("read unpublished revision"),
        "proposed",
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_model_current_revisions WHERE manifest_id=$1",
        )
        .bind(fixture.manifest_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count unpublished current pointers"),
        0,
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM stage_handoffs \
             WHERE source_stage_run_unit_id=$1 AND from_stage_kind='application_understanding'",
        )
        .bind(fixture.fence.stage_run_unit_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count unpublished Application Understanding handoffs"),
        0,
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM stage_run_units WHERE id=$1")
            .bind(fixture.fence.stage_run_unit_id)
            .fetch_one(fixture.db.pool())
            .await
            .expect("read unpublished Unit status"),
        "running",
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM stage_worker_runs WHERE id=$1")
            .bind(fixture.fence.worker_run_id)
            .fetch_one(fixture.db.pool())
            .await
            .expect("read unpublished Worker status"),
        "running",
    );
}

async fn load_publication_snapshot(fixture: &FinalizerFixture) -> serde_json::Value {
    sqlx::query_scalar::<_, serde_json::Value>(
        r#"SELECT jsonb_build_object(
               'revision',(
                   SELECT to_jsonb(revision.*)
                     FROM application_model_revisions AS revision
                    WHERE revision.id=$1
               ),
               'current',(
                   SELECT to_jsonb(current_revision.*)
                     FROM application_model_current_revisions AS current_revision
                    WHERE current_revision.manifest_id=$2
               ),
               'handoff',(
                   SELECT to_jsonb(handoff.*)
                     FROM stage_handoffs AS handoff
                    WHERE handoff.source_stage_run_unit_id=$3
                      AND handoff.from_stage_kind='application_understanding'
               ),
               'unit',(
                   SELECT to_jsonb(unit.*) FROM stage_run_units AS unit WHERE unit.id=$3
               ),
               'worker',(
                   SELECT to_jsonb(worker.*) FROM stage_worker_runs AS worker WHERE worker.id=$4
               ),
               'completion',(
                   SELECT to_jsonb(completion.*)
                     FROM org_stage_completions AS completion
                    WHERE completion.organization_id=$5
                      AND completion.stage_kind='application_understanding'
               )
           )"#,
    )
    .bind(fixture.revision_id)
    .bind(fixture.manifest_id)
    .bind(fixture.fence.stage_run_unit_id)
    .bind(fixture.fence.worker_run_id)
    .bind(fixture.organization_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("load exact Application Model publication snapshot")
}

async fn insert_finished_stage_tool_receipt(fixture: &FinalizerFixture, name: &str) {
    let tool_call_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,result,status,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token
           ) SELECT $1,$2,task.session_id,$3,'primary',$4,
                    '{}','{}','finished',$3,$5,$6,$7,$8,$9,$10
               FROM tasks AS task WHERE task.id=$3"#,
    )
    .bind(tool_call_id)
    .bind(format!("application-model-{name}-{tool_call_id}"))
    .bind(fixture.fence.operation_id)
    .bind(name)
    .bind(fixture.fence.stage_execution_id)
    .bind(fixture.fence.stage_run_unit_id)
    .bind(fixture.fence.worker_run_id)
    .bind(fixture.organization_id)
    .bind(fixture.fence.attempt_epoch)
    .bind(fixture.fence.lease_token)
    .execute(fixture.db.pool())
    .await
    .expect("insert synthetic stage tool receipt");
}

#[derive(Clone, Copy)]
struct AuSubmitResultWorkerContract<'a> {
    role: &'a str,
    output_schema: &'a str,
}

#[tokio::test]
#[serial]
async fn application_model_gate_finalizer_atomically_publishes_model_and_runtime_pass() {
    let fixture = FinalizerFixture::start("pass").await;

    let outcome = finalize_application_model_gate_pass(fixture.db.pool(), &fixture.command())
        .await
        .expect("finalize exact Application Model Gate PASS");
    let ApplicationModelFinalizationOutcome::Passed(passed) = outcome else {
        panic!("exact model should pass the Gate");
    };

    assert_eq!(passed.manifest_id, fixture.manifest_id);
    assert_eq!(passed.revision_id, Some(fixture.revision_id));
    assert!(!passed.replayed);
    assert_eq!(passed.final_seal.unit.status, "passed");
    assert_eq!(passed.final_seal.worker.status, "passed");
    assert!(matches!(
        passed.final_seal.canonical_fact_refs.as_slice(),
        [golish_db::repo::canonical_fact_refs::CanonicalFactRef {
            key: golish_db::repo::canonical_fact_refs::CanonicalFactKey::ApplicationModelRevision {
                revision_id
            },
            ..
        }] if *revision_id == fixture.revision_id
    ));
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT status FROM application_model_revisions WHERE id=$1",
        )
        .bind(fixture.revision_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("read finalized revision"),
        "final",
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_model_current_revisions WHERE manifest_id=$1",
        )
        .bind(fixture.manifest_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count current Application Model pointer"),
        1,
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM org_stage_completions \
             WHERE organization_id=$1 AND stage_kind='application_understanding'",
        )
        .bind(fixture.organization_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count Application Understanding completion"),
        1,
    );
}

#[tokio::test]
#[serial]
async fn application_model_gate_finalizer_exact_response_loss_replay_is_idempotent() {
    let fixture = FinalizerFixture::start("replay").await;
    let first = finalize_application_model_gate_pass(fixture.db.pool(), &fixture.command())
        .await
        .expect("first exact Application Model finalization");
    let ApplicationModelFinalizationOutcome::Passed(first) = first else {
        panic!("first exact Application Model finalization should pass");
    };
    let before_replay = load_publication_snapshot(&fixture).await;
    assert_eq!(before_replay["revision"]["row_version"], json!(1));
    assert_eq!(before_replay["unit"]["row_version"], json!(1));
    assert_eq!(before_replay["worker"]["checkpoint_version"], json!(1));
    assert!(!before_replay["revision"]["finalized_at"].is_null());
    assert!(!before_replay["current"]["published_at"].is_null());
    assert!(!before_replay["handoff"]["gate_passed_at"].is_null());

    let replayed = finalize_application_model_gate_pass(fixture.db.pool(), &fixture.command())
        .await
        .expect("replay exact Application Model finalization");
    let ApplicationModelFinalizationOutcome::Passed(replayed) = replayed else {
        panic!("exact response-loss replay should pass");
    };

    assert!(replayed.replayed);
    assert_eq!(replayed.manifest_id, first.manifest_id);
    assert_eq!(replayed.revision_id, first.revision_id);
    assert_eq!(replayed.final_seal.handoff.id, first.final_seal.handoff.id);
    assert_eq!(replayed.final_seal.unit.id, first.final_seal.unit.id);
    assert_eq!(replayed.final_seal.worker.id, first.final_seal.worker.id);
    let after_replay = load_publication_snapshot(&fixture).await;
    assert_eq!(
        after_replay, before_replay,
        "response-loss replay must be a field-for-field no-op, including timestamps and row versions"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM stage_handoffs \
             WHERE source_stage_run_unit_id=$1 AND from_stage_kind='application_understanding'",
        )
        .bind(fixture.fence.stage_run_unit_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count replayed handoffs"),
        1,
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_model_current_revisions WHERE manifest_id=$1",
        )
        .bind(fixture.manifest_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count replayed current pointers"),
        1,
    );
}

#[tokio::test]
#[serial]
async fn application_model_gate_finalizer_forbidden_activity_holds_with_zero_publication() {
    let fixture = FinalizerFixture::start("forbidden").await;
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,result,status,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token
           ) SELECT $1,'forbidden-browser',task.session_id,$2,'primary','browser_open',
                    '{}','{}','finished',$2,$3,$4,$5,$6,$7,$8
               FROM tasks AS task WHERE task.id=$2"#,
    )
    .bind(Uuid::new_v4())
    .bind(fixture.fence.operation_id)
    .bind(fixture.fence.stage_execution_id)
    .bind(fixture.fence.stage_run_unit_id)
    .bind(fixture.fence.worker_run_id)
    .bind(fixture.organization_id)
    .bind(fixture.fence.attempt_epoch)
    .bind(fixture.fence.lease_token)
    .execute(fixture.db.pool())
    .await
    .expect("record synthetic forbidden activity");

    let outcome = finalize_application_model_gate_pass(fixture.db.pool(), &fixture.command())
        .await
        .expect("forbidden activity should be a typed Gate HOLD");
    let ApplicationModelFinalizationOutcome::Blocked(block) = outcome else {
        panic!("forbidden activity must not publish");
    };

    assert_eq!(
        block.code,
        golish_agent_kit::harness::ApplicationModelGateCode::ForbiddenToolActivity
    );
    assert_application_model_unpublished(&fixture).await;
}

#[tokio::test]
#[serial]
async fn application_model_gate_finalizer_pending_work_item_holds_with_zero_publication() {
    let fixture = FinalizerFixture::start_with_pending_work_item("pending_work_item").await;

    let outcome = finalize_application_model_gate_pass(fixture.db.pool(), &fixture.command())
        .await
        .expect("pending WorkItem should be a typed Gate HOLD");
    let ApplicationModelFinalizationOutcome::Blocked(block) = outcome else {
        panic!("pending WorkItem must not publish");
    };

    assert_eq!(
        block.code,
        golish_agent_kit::harness::ApplicationModelGateCode::ProducerBarrierOpen,
        "pending WorkItem must remain the first blocker: {block:?}"
    );
    assert!(
        block
            .refs
            .iter()
            .any(|reference| reference.starts_with("work_item:")),
        "producer barrier must identify the pending WorkItem"
    );
    assert_application_model_unpublished(&fixture).await;
}

#[tokio::test]
#[serial]
async fn application_model_gate_finalizer_submitting_worker_active_tool_holds_with_zero_publication(
) {
    let fixture = FinalizerFixture::start("active_submitter_tool").await;
    let active_tool_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,result,status,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token
           ) SELECT $1,'active-update-plan',task.session_id,$2,'primary','update_plan',
                    '{}','{}','running',$2,$3,$4,$5,$6,$7,$8
               FROM tasks AS task WHERE task.id=$2"#,
    )
    .bind(active_tool_id)
    .bind(fixture.fence.operation_id)
    .bind(fixture.fence.stage_execution_id)
    .bind(fixture.fence.stage_run_unit_id)
    .bind(fixture.fence.worker_run_id)
    .bind(fixture.organization_id)
    .bind(fixture.fence.attempt_epoch)
    .bind(fixture.fence.lease_token)
    .execute(fixture.db.pool())
    .await
    .expect("insert active submitter tool receipt");
    sqlx::query(
        "UPDATE stage_worker_runs \
         SET active_tool_call_id=$2,active_tool_started_at=NOW(),updated_at=NOW() WHERE id=$1",
    )
    .bind(fixture.fence.worker_run_id)
    .bind(active_tool_id)
    .execute(fixture.db.pool())
    .await
    .expect("mark submitting Worker tool active");

    let outcome = finalize_application_model_gate_pass(fixture.db.pool(), &fixture.command())
        .await
        .expect("active submitter tool should be a typed Gate HOLD");
    let ApplicationModelFinalizationOutcome::Blocked(block) = outcome else {
        panic!("submitting Worker active tool must not publish");
    };

    assert_eq!(
        block.code,
        golish_agent_kit::harness::ApplicationModelGateCode::ProducerBarrierOpen
    );
    assert!(
        block
            .refs
            .contains(&format!("active_tool:{active_tool_id}")),
        "producer barrier must identify the submitter's active tool"
    );
    assert_application_model_unpublished(&fixture).await;
}

#[tokio::test]
#[serial]
async fn application_model_gate_finalizer_allows_submit_and_update_plan_receipts() {
    let fixture = FinalizerFixture::start("allowlisted_receipts").await;
    insert_finished_stage_tool_receipt(&fixture, "update_plan").await;

    let outcome = finalize_application_model_gate_pass(fixture.db.pool(), &fixture.command())
        .await
        .expect("submit/update_plan-only history should reach Gate PASS");

    assert!(
        matches!(outcome, ApplicationModelFinalizationOutcome::Passed(_)),
        "submit_stage_deliverable and update_plan receipts must not cause a false HOLD"
    );
}

#[tokio::test]
#[serial]
async fn application_model_gate_finalizer_allows_exact_au_modeler_submit_result() {
    let fixture = FinalizerFixture::start_with_au_submit_result(
        "exact_au_modeler_submit_result",
        AuSubmitResultWorkerContract {
            role: "application_model_worker",
            output_schema: "application_model_work_item_output.v1",
        },
    )
    .await;

    let outcome = finalize_application_model_gate_pass(fixture.db.pool(), &fixture.command())
        .await
        .expect("exact AU Modeler submit_result should reach Gate PASS");

    assert!(
        matches!(outcome, ApplicationModelFinalizationOutcome::Passed(_)),
        "the AU Modeler internal submit_result control receipt must not be forbidden: {outcome:?}"
    );
}

#[tokio::test]
#[serial]
async fn application_model_gate_finalizer_allows_exact_au_synthesizer_submit_result() {
    let fixture = FinalizerFixture::start_with_au_submit_result(
        "exact_au_synthesizer_submit_result",
        AuSubmitResultWorkerContract {
            role: "application_model_synthesizer",
            output_schema: "application_model_proposal.v1",
        },
    )
    .await;

    let outcome = finalize_application_model_gate_pass(fixture.db.pool(), &fixture.command())
        .await
        .expect("exact AU Synthesizer submit_result should reach Gate PASS");

    assert!(
        matches!(outcome, ApplicationModelFinalizationOutcome::Passed(_)),
        "the AU Synthesizer internal submit_result control receipt must not be forbidden: {outcome:?}"
    );
}

#[tokio::test]
#[serial]
async fn application_model_gate_finalizer_rejects_inexact_submit_result_with_wrong_schema() {
    let fixture = FinalizerFixture::start_with_au_submit_result(
        "wrong_schema_submit_result",
        AuSubmitResultWorkerContract {
            role: "application_model_worker",
            output_schema: "stage_worker_output.v1",
        },
    )
    .await;

    let outcome = finalize_application_model_gate_pass(fixture.db.pool(), &fixture.command())
        .await
        .expect("wrong-schema submit_result should be a typed Gate HOLD");
    let ApplicationModelFinalizationOutcome::Blocked(block) = outcome else {
        panic!("wrong-schema submit_result must not publish");
    };

    assert_eq!(
        block.code,
        golish_agent_kit::harness::ApplicationModelGateCode::ForbiddenToolActivity
    );
    assert_application_model_unpublished(&fixture).await;
}

#[tokio::test]
#[serial]
async fn application_model_gate_finalizer_rejects_inexact_submit_result_for_ordinary_role() {
    let fixture = FinalizerFixture::start_with_au_submit_result(
        "ordinary_role_submit_result",
        AuSubmitResultWorkerContract {
            role: "application_understanding",
            output_schema: "application_model_work_item_output.v1",
        },
    )
    .await;

    let outcome = finalize_application_model_gate_pass(fixture.db.pool(), &fixture.command())
        .await
        .expect("ordinary-role submit_result should be a typed Gate HOLD");
    let ApplicationModelFinalizationOutcome::Blocked(block) = outcome else {
        panic!("ordinary-role submit_result must not publish");
    };

    assert_eq!(
        block.code,
        golish_agent_kit::harness::ApplicationModelGateCode::ForbiddenToolActivity
    );
    assert_application_model_unpublished(&fixture).await;
}

#[tokio::test]
#[serial]
async fn application_model_gate_finalizer_query_target_data_receipt_is_forbidden() {
    let fixture = FinalizerFixture::start("forbidden_query_target_data").await;
    insert_finished_stage_tool_receipt(&fixture, "query_target_data").await;

    let outcome = finalize_application_model_gate_pass(fixture.db.pool(), &fixture.command())
        .await
        .expect("query_target_data should be a typed Gate HOLD");
    let ApplicationModelFinalizationOutcome::Blocked(block) = outcome else {
        panic!("query_target_data activity must not publish");
    };

    assert_eq!(
        block.code,
        golish_agent_kit::harness::ApplicationModelGateCode::ForbiddenToolActivity
    );
    assert!(
        block
            .refs
            .iter()
            .any(|reference| reference.contains(":query_target_data:")),
        "forbidden receipt must identify query_target_data"
    );
    assert_application_model_unpublished(&fixture).await;
}

#[tokio::test]
#[serial]
async fn application_model_gate_finalizer_source_invalidation_holds_with_zero_publication() {
    let fixture = FinalizerFixture::start("source_invalidated").await;
    sqlx::query("UPDATE stage_handoffs SET invalidated_at=NOW() WHERE id=$1")
        .bind(fixture.source_handoff_id)
        .execute(fixture.db.pool())
        .await
        .expect("invalidate exact upstream Handoff");

    let outcome = finalize_application_model_gate_pass(fixture.db.pool(), &fixture.command())
        .await
        .expect("source invalidation should be a typed Gate HOLD");
    let ApplicationModelFinalizationOutcome::Blocked(block) = outcome else {
        panic!("invalidated source must not publish");
    };

    assert_eq!(
        block.code,
        golish_agent_kit::harness::ApplicationModelGateCode::IdentityMismatch
    );
    assert_application_model_unpublished(&fixture).await;
}

#[tokio::test]
#[serial]
async fn application_model_gate_finalizer_pending_sibling_worker_holds_with_zero_publication() {
    let fixture = FinalizerFixture::start("pending_worker").await;
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,
               organization_id,worker_generation,specialist,work_item_kind,
               work_item_key,agent_path,status,attempt_epoch
           ) VALUES($1,$2,$3,$4,$5,0,'application_understanding','late_producer',
                    'late-producer','main>late-producer','queued',0)"#,
    )
    .bind(Uuid::new_v4())
    .bind(fixture.fence.operation_id)
    .bind(fixture.fence.stage_execution_id)
    .bind(fixture.fence.stage_run_unit_id)
    .bind(fixture.organization_id)
    .execute(fixture.db.pool())
    .await
    .expect("insert synthetic pending sibling producer");

    let outcome = finalize_application_model_gate_pass(fixture.db.pool(), &fixture.command())
        .await
        .expect("pending producer should be a typed Gate HOLD");
    let ApplicationModelFinalizationOutcome::Blocked(block) = outcome else {
        panic!("pending producer must not publish");
    };

    assert_eq!(
        block.code,
        golish_agent_kit::harness::ApplicationModelGateCode::ProducerBarrierOpen
    );
    assert_application_model_unpublished(&fixture).await;
}

#[tokio::test]
#[serial]
async fn application_model_gate_finalizer_lost_lease_holds_with_zero_publication() {
    let fixture = FinalizerFixture::start("lost_lease").await;
    let mut command = fixture.command();
    command.fence.lease_token = Uuid::new_v4();

    let outcome = finalize_application_model_gate_pass(fixture.db.pool(), &command)
        .await
        .expect("lost lease should route to a typed Gate HOLD");
    let ApplicationModelFinalizationOutcome::Blocked(block) = outcome else {
        panic!("lost lease must not publish");
    };
    assert_eq!(
        block.disposition,
        golish_agent_kit::harness::ApplicationModelGateDisposition::Hold
    );
    assert_application_model_unpublished(&fixture).await;
}

#[tokio::test]
#[serial]
async fn application_model_gate_finalizer_wrong_submission_holds_with_zero_publication() {
    let fixture = FinalizerFixture::start("wrong_submission").await;
    let mut command = fixture.command();
    command.deliverable_submission_id = Uuid::new_v4();

    let outcome = finalize_application_model_gate_pass(fixture.db.pool(), &command)
        .await
        .expect("wrong submission should route to a typed Gate HOLD");
    let ApplicationModelFinalizationOutcome::Blocked(block) = outcome else {
        panic!("wrong submission must not publish");
    };
    assert_eq!(
        block.disposition,
        golish_agent_kit::harness::ApplicationModelGateDisposition::Hold
    );
    assert_application_model_unpublished(&fixture).await;
}

#[tokio::test]
#[serial]
async fn application_model_gate_finalizer_runtime_failure_rolls_back_revision_transition() {
    let fixture = FinalizerFixture::start("rollback").await;
    let mut command = fixture.command();
    command.expected_unit_row_version = 99;

    let outcome = finalize_application_model_gate_pass(fixture.db.pool(), &command)
        .await
        .expect("stale Unit CAS must route to a typed Gate HOLD");
    let ApplicationModelFinalizationOutcome::Blocked(block) = outcome else {
        panic!("stale Unit CAS must not publish");
    };
    assert_eq!(
        block.disposition,
        golish_agent_kit::harness::ApplicationModelGateDisposition::Hold
    );

    assert_application_model_unpublished(&fixture).await;
}

#[tokio::test]
#[serial]
async fn application_model_gate_finalizer_current_publish_failure_rolls_back_runtime_writes() {
    let fixture = FinalizerFixture::start("late_publish_rollback").await;
    sqlx::query(
        r#"CREATE FUNCTION test_reject_application_model_current()
           RETURNS trigger AS $$
           BEGIN
               RAISE EXCEPTION 'TEST_REJECT_APPLICATION_MODEL_CURRENT';
           END;
           $$ LANGUAGE plpgsql"#,
    )
    .execute(fixture.db.pool())
    .await
    .expect("install test-only late publication failure function");
    sqlx::query(
        r#"CREATE TRIGGER test_reject_application_model_current
           BEFORE INSERT ON application_model_current_revisions
           FOR EACH ROW EXECUTE FUNCTION test_reject_application_model_current()"#,
    )
    .execute(fixture.db.pool())
    .await
    .expect("install test-only late publication failure trigger");

    finalize_application_model_gate_pass(fixture.db.pool(), &fixture.command())
        .await
        .expect_err("current publication must fail after runtime final-seal writes");

    assert_application_model_unpublished(&fixture).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM org_stage_completions \
             WHERE organization_id=$1 AND stage_kind='application_understanding'",
        )
        .bind(fixture.organization_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count rolled-back completion"),
        0,
    );
}

#[tokio::test]
#[serial]
async fn application_model_gate_finalizer_publishes_terminal_no_input_without_revision() {
    let fixture = FinalizerFixture::start("terminal_base").await;
    let workspace =
        sqlx::query_scalar::<_, String>("SELECT project_path FROM organizations WHERE id=$1")
            .bind(fixture.organization_id)
            .fetch_one(fixture.db.pool())
            .await
            .expect("read terminal fixture workspace");
    let project_scope_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT project_scope_id FROM operation_state WHERE operation_id=$1",
    )
    .bind(fixture.fence.operation_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("read terminal fixture project");
    let session = sessions::create(
        fixture.db.pool(),
        NewSession {
            title: Some("Application Model terminal fixture".to_string()),
            workspace_path: Some(workspace.clone()),
            workspace_label: None,
            model: Some("fixture-model".to_string()),
            provider: Some("fixture-provider".to_string()),
            project_path: Some(workspace),
        },
    )
    .await
    .expect("create terminal fixture session");
    let operation_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    runtime_memory_tx::create_runtime_operation(
        fixture.db.pool(),
        &runtime_memory_tx::CreateRuntimeOperationRow {
            operation_id,
            initial_stage_execution_id: stage_execution_id,
            session_id: session.id,
            title: Some("Application Model terminal fixture".to_string()),
            input: "terminal fixture".to_string(),
            profile: "red_team".to_string(),
            entry_stage: "application_understanding".to_string(),
            application_model_contract: golish_core::ApplicationModelContract::ApplicationModelV1,
            project_scope_id,
            cli_scope: Some(runtime_memory_tx::CliRuntimeScopeRow {
                root_organization_id: fixture.organization_id,
                include_subsidiaries: false,
                subsidiary_threshold: 51,
                units: vec![runtime_memory_tx::CliRuntimeScopeUnitRow {
                    organization_id: fixture.organization_id,
                    parent_organization_id: None,
                    organization_name: "Model Org".to_string(),
                    depth: 0,
                    ordinal: 0,
                    ownership_percent: None,
                    approval_source: json!({"kind": "terminal_fixture"}),
                }],
            }),
        },
    )
    .await
    .expect("create terminal operation");
    let (scope_snapshot_id, scope_hash) = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT id,scope_hash FROM operation_org_scope_snapshots WHERE operation_id=$1",
    )
    .bind(operation_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("read terminal frozen scope");
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
    .bind(fixture.organization_id)
    .execute(fixture.db.pool())
    .await
    .expect("insert terminal Unit");
    let seeded = application_models::seed_manifest(
        fixture.db.pool(),
        &SeedApplicationModelManifest {
            operation_id,
            scope_snapshot_id,
            stage_execution_id,
            stage_run_unit_id,
            organization_id: fixture.organization_id,
            authority_kind: ApplicationModelAuthorityKindRow::TerminalNoInput,
            inputs: Vec::new(),
        },
    )
    .await
    .expect("seed terminal-no-input manifest");
    let worker_run_id = Uuid::new_v4();
    let lease_token = Uuid::new_v4();
    let tool_call_id = Uuid::new_v4();
    let submission_id = Uuid::new_v4();
    let payload = json!({
        "stage_id": "application_understanding",
        "stage_run_id": stage_execution_id,
        "schema_version": 1,
        "manifest_id": seeded.manifest.id,
        "authority_kind": "terminal_no_input",
    });
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,
               organization_id,worker_generation,specialist,work_item_kind,
               work_item_key,agent_path,status,lease_token,lease_owner,
               lease_acquired_at,lease_expires_at,heartbeat_at,attempt_epoch
           ) VALUES($1,$2,$3,$4,$5,0,'application_understanding','stage_unit',
                    'application_understanding','main>application_understanding','running',
                    $6,'terminal-fixture',NOW(),NOW()+INTERVAL '5 minutes',NOW(),0)"#,
    )
    .bind(worker_run_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(fixture.organization_id)
    .bind(lease_token)
    .execute(fixture.db.pool())
    .await
    .expect("insert terminal Worker");
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,result,status,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token
           ) VALUES($1,'terminal-submit',$2,$3,'primary','submit_stage_deliverable',
                    '{}','{}','finished',$3,$4,$5,$6,$7,0,$8)"#,
    )
    .bind(tool_call_id)
    .bind(session.id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(worker_run_id)
    .bind(fixture.organization_id)
    .bind(lease_token)
    .execute(fixture.db.pool())
    .await
    .expect("insert terminal submit tool");
    sqlx::query(
        r#"INSERT INTO stage_deliverable_submissions(
               id,operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,tool_call_record_id,tool_request_id,stage_kind,
               attempt_epoch,lease_token,payload,payload_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,'terminal-submit',
                    'application_understanding',0,$8,$9,$10)"#,
    )
    .bind(submission_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(worker_run_id)
    .bind(fixture.organization_id)
    .bind(tool_call_id)
    .bind(lease_token)
    .bind(&payload)
    .bind(sha256_json(&payload))
    .execute(fixture.db.pool())
    .await
    .expect("insert terminal submission");

    let outcome = finalize_application_model_gate_pass(
        fixture.db.pool(),
        &FinalizeApplicationModelGatePass {
            fence: runtime_memory_tx::RuntimeMemoryTxFence {
                operation_id,
                stage_execution_id,
                stage_run_unit_id,
                worker_run_id,
                lease_token,
                attempt_epoch: 0,
                expected_checkpoint_version: 0,
            },
            deliverable_submission_id: submission_id,
            manifest_id: seeded.manifest.id,
            expected_unit_row_version: 0,
            scope_hash,
        },
    )
    .await
    .expect("finalize terminal-no-input authority");
    let ApplicationModelFinalizationOutcome::Passed(passed) = outcome else {
        panic!("terminal-no-input DB truth should pass");
    };

    assert_eq!(passed.revision_id, None);
    assert_eq!(
        sqlx::query_as::<_, (Option<Uuid>, Option<String>, String)>(
            "SELECT revision_id,model_hash,authority_kind \
             FROM application_model_current_revisions WHERE manifest_id=$1",
        )
        .bind(seeded.manifest.id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("read terminal current pointer"),
        (None, None, "terminal_no_input".to_string()),
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_model_revisions WHERE manifest_id=$1",
        )
        .bind(seeded.manifest.id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count terminal revisions"),
        0,
    );
    assert_eq!(
        sqlx::query_as::<_, (serde_json::Value, Vec<i64>, String, String)>(
            r#"SELECT payload -> 'canonical_fact_refs',evidence_ids,
                      current_revision.manifest_hash,current_revision.replay_material_hash
                 FROM stage_handoffs AS handoff
                 JOIN application_model_current_revisions AS current_revision
                   ON current_revision.stage_handoff_id=handoff.id
                WHERE current_revision.manifest_id=$1"#,
        )
        .bind(seeded.manifest.id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("read exact terminal authority"),
        (
            json!([]),
            Vec::new(),
            seeded.manifest.manifest_hash.clone(),
            seeded.manifest.manifest_hash,
        ),
    );
}
