use chrono::{DateTime, Utc};
use golish_db::models::NewSession;
use golish_db::repo::application_models::{
    self, ApplicationModelAuthorityKindRow, ApplicationModelEvidenceRoleRow,
    ApplicationModelInputDecisionSeed, ApplicationModelInputDispositionRow,
    ApplicationModelItemEvidenceSeed, ApplicationModelItemSeed, ApplicationModelManifestInputSeed,
    ApplicationModelTruthStateRow, ProposeApplicationModelRevision, SeedApplicationModelManifest,
};
use golish_db::repo::{project_scopes, runtime_memory_tx, sessions};
use golish_db::{DbConfig, GolishDb};
use serde_json::{json, Value};
use serial_test::serial;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use uuid::Uuid;

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read reserved local postgres port")
        .port()
}

struct ApplicationModelFinalizationDb {
    db: GolishDb,
    _data_dir: TempDir,
}

impl ApplicationModelFinalizationDb {
    async fn start(label: &str) -> Self {
        let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
        let config = DbConfig {
            pg_data_dir: data_dir.path().join("pgdata"),
            port: reserve_local_port(),
            database: format!(
                "application_model_finalization_{label}_{}",
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
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).expect("serialize JSON string"),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|key| format!(
                        "{}:{}",
                        serde_json::to_string(key).expect("serialize JSON key"),
                        canonical_json(&object[key])
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

fn sha256_json(value: &Value) -> String {
    Sha256::digest(canonical_json(value).as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn tagged_sha256_json(value: &Value) -> String {
    format!("sha256:{}", sha256_json(value))
}

struct RawApplicationModelFixture {
    database: ApplicationModelFinalizationDb,
    session_id: Uuid,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    scope_hash: String,
    organization_id: Uuid,
    source_handoff_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    worker_run_id: Uuid,
    lease_token: Uuid,
    submission_id: Uuid,
    manifest_id: Uuid,
    manifest_hash: String,
    revision_id: Uuid,
    model_hash: String,
    replay_material_hash: String,
    evidence_id: i64,
    team_plan_id: Option<Uuid>,
    leader_work_item_id: Option<Uuid>,
    sibling_work_item_id: Option<Uuid>,
}

impl RawApplicationModelFixture {
    async fn start(label: &str) -> Self {
        Self::start_with_team(label, false).await
    }

    async fn start_team(label: &str) -> Self {
        Self::start_with_team(label, true).await
    }

    async fn start_with_team(label: &str, team_mode: bool) -> Self {
        let database = ApplicationModelFinalizationDb::start(label).await;
        let workspace = format!("/tmp/application-model-raw-{}", Uuid::new_v4().simple());
        let project =
            project_scopes::register_first_open(database.db.pool(), &workspace, &"1".repeat(64))
                .await
                .expect("register raw publication project");
        let organization_id = Uuid::new_v4();
        sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$2,'Raw Org')")
            .bind(organization_id)
            .bind(&workspace)
            .execute(database.db.pool())
            .await
            .expect("insert raw publication organization");
        let session = sessions::create(
            database.db.pool(),
            NewSession {
                title: Some("Application Model raw publication fixture".to_string()),
                workspace_path: Some(workspace.clone()),
                workspace_label: None,
                model: Some("fixture-model".to_string()),
                provider: Some("fixture-provider".to_string()),
                project_path: Some(workspace.clone()),
            },
        )
        .await
        .expect("create raw publication session");
        let operation_id = Uuid::new_v4();
        let stage_execution_id = Uuid::new_v4();
        runtime_memory_tx::create_runtime_operation(
            database.db.pool(),
            &runtime_memory_tx::CreateRuntimeOperationRow {
                operation_id,
                initial_stage_execution_id: stage_execution_id,
                session_id: session.id,
                title: Some("Application Model raw publication fixture".to_string()),
                input: "raw publication fixture".to_string(),
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
                        organization_name: "Raw Org".to_string(),
                        depth: 0,
                        ordinal: 0,
                        ownership_percent: None,
                        approval_source: json!({"kind": "raw_fixture"}),
                    }],
                }),
            },
        )
        .await
        .expect("create raw publication operation");
        let (scope_snapshot_id, scope_hash) = sqlx::query_as::<_, (Uuid, String)>(
            "SELECT id,scope_hash FROM operation_org_scope_snapshots WHERE operation_id=$1",
        )
        .bind(operation_id)
        .fetch_one(database.db.pool())
        .await
        .expect("read raw publication scope");

        let source_execution_id = Uuid::new_v4();
        let source_unit_id = Uuid::new_v4();
        let source_worker_id = Uuid::new_v4();
        let source_lease = Uuid::new_v4();
        let source_tool_id = Uuid::new_v4();
        let source_submission_id = Uuid::new_v4();
        let source_handoff_id = Uuid::new_v4();
        let source_payload = json!({
            "schema_version": 1,
            "organization_id": organization_id,
            "routes": ["/orders/{id}"],
        });
        let evidence_id = sqlx::query_scalar::<_, i64>(
            r#"INSERT INTO audit_log(
                   action,category,details,project_path,audit_role,run_id,detail
               ) VALUES('application model raw source','attack','',$1,'evidence',$2,$3)
               RETURNING id"#,
        )
        .bind(&workspace)
        .bind(operation_id)
        .bind(json!({"organization_id": organization_id, "route": "/orders/{id}"}))
        .fetch_one(database.db.pool())
        .await
        .expect("insert raw inherited evidence");
        sqlx::query(
            "INSERT INTO stage_runs(id,operation_id,stage_kind,status) \
             VALUES($1,$2,'vuln_triage','completed')",
        )
        .bind(source_execution_id)
        .bind(operation_id)
        .execute(database.db.pool())
        .await
        .expect("insert raw source execution");
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
        .execute(database.db.pool())
        .await
        .expect("insert raw passed source unit");
        sqlx::query(
            r#"INSERT INTO stage_worker_runs(
                   id,operation_id,stage_execution_id,stage_run_unit_id,
                   organization_id,worker_generation,specialist,work_item_kind,
                   work_item_key,agent_path,status,lease_token,lease_owner,
                   lease_acquired_at,lease_expires_at,heartbeat_at,attempt_epoch
               ) VALUES($1,$2,$3,$4,$5,0,'vuln_triage','stage_unit','vuln_triage',
                        'main>vuln_triage','running',$6,'raw-source',
                        NOW(),NOW()+INTERVAL '5 minutes',NOW(),0)"#,
        )
        .bind(source_worker_id)
        .bind(operation_id)
        .bind(source_execution_id)
        .bind(source_unit_id)
        .bind(organization_id)
        .bind(source_lease)
        .execute(database.db.pool())
        .await
        .expect("insert raw source worker");
        sqlx::query(
            r#"INSERT INTO tool_calls(
                   id,call_id,session_id,task_id,agent,name,args,result,status,
                   operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
                   organization_id,attempt_epoch,lease_token
               ) VALUES($1,'raw-source-submit',$2,$3,'primary','submit_stage_deliverable',
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
        .execute(database.db.pool())
        .await
        .expect("insert raw source submit receipt");
        sqlx::query(
            r#"INSERT INTO stage_deliverable_submissions(
                   id,operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
                   organization_id,tool_call_record_id,tool_request_id,stage_kind,
                   attempt_epoch,lease_token,payload,payload_sha256
               ) VALUES($1,$2,$3,$4,$5,$6,$7,'raw-source-submit','vuln_triage',0,$8,$9,$10)"#,
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
        .execute(database.db.pool())
        .await
        .expect("insert raw source submission");
        sqlx::query(
            "UPDATE stage_worker_runs SET status='passed',terminal_at=NOW(),updated_at=NOW() \
             WHERE id=$1",
        )
        .bind(source_worker_id)
        .execute(database.db.pool())
        .await
        .expect("pass raw source worker");
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
        .execute(database.db.pool())
        .await
        .expect("insert raw source handoff");

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
        .execute(database.db.pool())
        .await
        .expect("insert raw Application Understanding unit");
        let seeded = application_models::seed_manifest(
            database.db.pool(),
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
        .expect("seed raw publication manifest");
        let worker_run_id = Uuid::new_v4();
        let lease_token = Uuid::new_v4();
        let team_plan_id = team_mode.then(Uuid::new_v4);
        let leader_work_item_id = team_mode.then(Uuid::new_v4);
        let sibling_work_item_id = team_mode.then(Uuid::new_v4);
        if let (Some(team_plan_id), Some(leader_work_item_id), Some(sibling_work_item_id)) =
            (team_plan_id, leader_work_item_id, sibling_work_item_id)
        {
            sqlx::query(
                r#"INSERT INTO stage_team_plans(
                       id,operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
                       organization_id,stage_kind,unit_generation,schema_version,plan_version,
                       plan_hash,leader_role,aggregator_kind,aggregator_role,allowed_worker_roles,
                       max_workers_total,max_workers_active,dynamic_requests_allowed,
                       dynamic_request_policy,dispatch_epoch,final_submitter_kind,
                       created_from_stage_spec_hash
                   ) VALUES($1,$2,$3,$4,$5,$6,'application_understanding',0,1,1,$7,
                            'application_model_synthesizer','worker',
                            'application_model_synthesizer',$8,1,1,FALSE,$9,0,'worker',$10)"#,
            )
            .bind(team_plan_id)
            .bind(operation_id)
            .bind(stage_execution_id)
            .bind(stage_run_unit_id)
            .bind(scope_snapshot_id)
            .bind(organization_id)
            .bind(tagged_sha256_json(
                &json!({"plan": "application-model-team"}),
            ))
            .bind(json!(["application_model_synthesizer"]))
            .bind(json!({"coordination_mode": "company_controller"}))
            .bind(tagged_sha256_json(
                &json!({"spec": "application-model-team"}),
            ))
            .execute(database.db.pool())
            .await
            .expect("insert Application Model TeamPlan");
            sqlx::query(
                r#"INSERT INTO stage_work_items(
                       id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                       scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
                       input_manifest_hash,input_refs,required_for_barrier,conflict_key,priority,
                       status,attempt_policy,budget,output_schema,created_by,started_at
                   ) VALUES($1,$2,$3,$4,$5,$6,$7,0,'application_model_synthesis',
                            'leader:primary','application_model_synthesizer',$8,$9,FALSE,
                            'stage_unit_finalizer',2147483647,'running',$10,'{}'::JSONB,
                            'application_model_proposal.v1','server_seed',NOW())"#,
            )
            .bind(leader_work_item_id)
            .bind(team_plan_id)
            .bind(operation_id)
            .bind(stage_execution_id)
            .bind(stage_run_unit_id)
            .bind(scope_snapshot_id)
            .bind(organization_id)
            .bind(tagged_sha256_json(&json!({"synthesis": true})))
            .bind(json!([{"schema": "application_model_synthesis_input.v1"}]))
            .bind(json!({"max_attempts": 2}))
            .execute(database.db.pool())
            .await
            .expect("insert Application Model synthesis WorkItem");
            sqlx::query(
                r#"INSERT INTO stage_work_items(
                       id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                       scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
                       input_manifest_hash,input_refs,required_for_barrier,conflict_key,priority,
                       status,attempt_policy,budget,output_schema,created_by
                   ) VALUES($1,$2,$3,$4,$5,$6,$7,0,'application_model_analysis',
                            'analysis:routes','application_model_synthesizer',$8,$9,TRUE,
                            'application_model_routes',0,'queued',$10,'{}'::JSONB,
                            'stage_worker_output.v1','server_seed')"#,
            )
            .bind(sibling_work_item_id)
            .bind(team_plan_id)
            .bind(operation_id)
            .bind(stage_execution_id)
            .bind(stage_run_unit_id)
            .bind(scope_snapshot_id)
            .bind(organization_id)
            .bind(tagged_sha256_json(&json!({"analysis": "routes"})))
            .bind(json!([{"schema": "application_model_work_item_input.v1"}]))
            .bind(json!({"max_attempts": 2}))
            .execute(database.db.pool())
            .await
            .expect("insert pending Application Model sibling WorkItem");
        }
        let submission_tool_id = Uuid::new_v4();
        let submission_id = Uuid::new_v4();
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
               ) VALUES($1,$2,$3,$4,$5,0,$7,$8,$9,
                        'main>application_understanding','running',
                        $6,'raw-model',NOW(),NOW()+INTERVAL '5 minutes',NOW(),0,$10)"#,
        )
        .bind(worker_run_id)
        .bind(operation_id)
        .bind(stage_execution_id)
        .bind(stage_run_unit_id)
        .bind(organization_id)
        .bind(lease_token)
        .bind(if team_mode {
            "application_model_synthesizer"
        } else {
            "application_understanding"
        })
        .bind(if team_mode {
            "application_model_synthesis"
        } else {
            "stage_unit"
        })
        .bind(if team_mode {
            "leader:primary"
        } else {
            "application_understanding"
        })
        .bind(leader_work_item_id)
        .execute(database.db.pool())
        .await
        .expect("insert raw model worker");
        if let Some(team_plan_id) = team_plan_id {
            sqlx::query(
                r#"UPDATE stage_team_plans
                      SET requests_closed_at=NOW(),final_submitter_worker_run_id=$2,
                          row_version=row_version+1,updated_at=NOW()
                    WHERE id=$1"#,
            )
            .bind(team_plan_id)
            .bind(worker_run_id)
            .execute(database.db.pool())
            .await
            .expect("bind exact Application Model final submitter");
        }
        sqlx::query(
            r#"INSERT INTO tool_calls(
                   id,call_id,session_id,task_id,agent,name,args,result,status,
                   operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
                   organization_id,attempt_epoch,lease_token
               ) VALUES($1,'raw-model-submit',$2,$3,'primary','submit_stage_deliverable',
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
        .execute(database.db.pool())
        .await
        .expect("insert raw model submit receipt");
        if team_mode {
            sqlx::query(
                "UPDATE stage_worker_runs SET active_tool_call_id=$2,active_tool_started_at=NOW() \
                 WHERE id=$1",
            )
            .bind(worker_run_id)
            .bind(submission_tool_id)
            .execute(database.db.pool())
            .await
            .expect("bind Team submit tool to finalizer Worker");
        }
        sqlx::query(
            r#"INSERT INTO stage_deliverable_submissions(
                   id,operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
                   organization_id,tool_call_record_id,tool_request_id,stage_kind,
                   attempt_epoch,lease_token,payload,payload_sha256
               ) VALUES($1,$2,$3,$4,$5,$6,$7,'raw-model-submit',
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
        .execute(database.db.pool())
        .await
        .expect("insert raw model submission");
        if team_mode {
            sqlx::query(
                "UPDATE stage_worker_runs SET active_tool_call_id=NULL,active_tool_started_at=NULL \
                 WHERE id=$1",
            )
                .bind(worker_run_id)
                .execute(database.db.pool())
                .await
                .expect("clear Team submit tool fence");
        }
        let proposed = application_models::propose_revision(
            database.db.pool(),
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
        .expect("propose raw model revision");
        Self {
            database,
            session_id: session.id,
            operation_id,
            scope_snapshot_id,
            scope_hash,
            organization_id,
            source_handoff_id,
            stage_execution_id,
            stage_run_unit_id,
            worker_run_id,
            lease_token,
            submission_id,
            manifest_id: seeded.manifest.id,
            manifest_hash: seeded.manifest.manifest_hash,
            revision_id: proposed.revision.id,
            model_hash: proposed.revision.model_hash,
            replay_material_hash: proposed.revision.replay_material_hash,
            evidence_id,
            team_plan_id,
            leader_work_item_id,
            sibling_work_item_id,
        }
    }

    fn pool(&self) -> &sqlx::PgPool {
        self.database.db.pool()
    }
}

#[derive(Clone, Copy)]
enum RawModelPublicationMutation {
    None,
    ExtraClaim,
    ExtraCanonicalRef,
    PayloadHashDrift,
    PayloadEvidenceDrift,
    PayloadCoverageDrift,
    UnitWatermarkDrift,
    CompletionDrift,
    RunningSubmitReceipt,
    LateSourceInvalidation,
    LatePredecessorHandoffInsert,
    LateSourceUnitStatusDrift,
}

async fn attempt_raw_model_publication(
    fixture: &RawApplicationModelFixture,
    mutation: RawModelPublicationMutation,
) -> Result<(), sqlx::Error> {
    let mut tx = fixture.pool().begin().await?;
    sqlx::query(
        "UPDATE application_model_revisions \
         SET status='final',row_version=1,finalized_at=transaction_timestamp() WHERE id=$1",
    )
    .bind(fixture.revision_id)
    .execute(&mut *tx)
    .await?;
    let (finalized_at, tagged_content_hash, ref_evidence_ids) =
        sqlx::query_as::<_, (DateTime<Utc>, String, Vec<i64>)>(
            r#"SELECT finalized_at,
                      application_model_sha256_jsonb(
                          application_model_revision_canonical_content(id)
                      ),
                      application_model_revision_evidence_ids(id)
                 FROM application_model_revisions WHERE id=$1"#,
        )
        .bind(fixture.revision_id)
        .fetch_one(&mut *tx)
        .await?;
    let handoff_id = Uuid::new_v4();
    let coverage_watermark = json!({
        "schema_version": "application_model_coverage.v1",
        "manifest_id": fixture.manifest_id,
        "revision_id": fixture.revision_id,
        "input_count": 1,
        "decision_count": 1,
        "item_count": 1,
        "manifest_hash": fixture.manifest_hash,
        "model_hash": fixture.model_hash,
        "replay_material_hash": fixture.replay_material_hash,
    });
    let authority_claim = json!({
        "kind": "application_model_authority",
        "payload": {
            "authority_kind": "model",
            "manifest_id": fixture.manifest_id,
            "revision_id": fixture.revision_id,
            "manifest_hash": fixture.manifest_hash,
            "model_hash": fixture.model_hash,
            "replay_material_hash": fixture.replay_material_hash,
            "deliverable_submission_id": fixture.submission_id,
        }
    });
    let terminal_checkpoint = json!({
        "schema_version": "application_model_terminal.v1",
        "manifest_id": fixture.manifest_id,
        "revision_id": fixture.revision_id,
        "manifest_hash": fixture.manifest_hash,
        "model_hash": fixture.model_hash,
        "replay_material_hash": fixture.replay_material_hash,
        "deliverable_submission_id": fixture.submission_id,
    });
    let gate_details = json!({
        "code": "APPLICATION_MODEL_GATE_PASS",
        "authority_kind": "model",
        "manifest_id": fixture.manifest_id,
        "revision_id": fixture.revision_id,
        "manifest_hash": fixture.manifest_hash,
        "model_hash": fixture.model_hash,
        "replay_material_hash": fixture.replay_material_hash,
    });
    let seal_material = json!({
        "canonical_fact_keys": [{
            "kind": "application_model_revision",
            "revision_id": fixture.revision_id,
        }],
        "typed_claims": [authority_claim.clone()],
        "coverage_watermark": coverage_watermark,
        "evidence_ids": [fixture.evidence_id],
        "terminal_checkpoint": terminal_checkpoint,
        "deterministic_gate_details": gate_details,
        "candidate_acceptance": null,
    });
    let gate_decision = json!({
        "outcome": "pass",
        "operation_id": fixture.operation_id,
        "stage_execution_id": fixture.stage_execution_id,
        "stage_run_unit_id": fixture.stage_run_unit_id,
        "deliverable_submission_id": fixture.submission_id,
        "scope_hash": fixture.scope_hash,
        "details": gate_details,
        "seal_material_sha256": sha256_json(&seal_material),
    });
    let gate_decision_hash = sha256_json(&gate_decision);
    let revision_ref = json!({
        "key": {
            "kind": "application_model_revision",
            "revision_id": fixture.revision_id,
        },
        "organization_id": fixture.organization_id,
        "observed_at": finalized_at,
        "content_sha256": tagged_content_hash
            .strip_prefix("sha256:")
            .expect("canonical revision hash is tagged"),
        "evidence_ids": ref_evidence_ids,
    });
    let mut typed_claims = vec![authority_claim];
    let mut canonical_refs = vec![revision_ref];
    if matches!(mutation, RawModelPublicationMutation::ExtraClaim) {
        typed_claims.push(json!({"kind": "foreign_authority", "payload": {"accepted": true}}));
    }
    if matches!(mutation, RawModelPublicationMutation::ExtraCanonicalRef) {
        canonical_refs.push(json!({
            "key": {"kind": "finding", "finding_id": Uuid::new_v4()},
            "organization_id": Uuid::new_v4(),
            "observed_at": finalized_at,
            "content_sha256": "c".repeat(64),
            "evidence_ids": [],
        }));
    }
    let payload_evidence_ids =
        if matches!(mutation, RawModelPublicationMutation::PayloadEvidenceDrift) {
            Vec::new()
        } else {
            vec![fixture.evidence_id]
        };
    let payload_coverage = if matches!(mutation, RawModelPublicationMutation::PayloadCoverageDrift)
    {
        json!({"drifted": true})
    } else {
        coverage_watermark.clone()
    };
    let payload = json!({
        "schema_version": 1,
        "canonical_fact_refs": canonical_refs,
        "typed_claims": typed_claims,
        "coverage_watermark": payload_coverage,
        "evidence_ids": payload_evidence_ids,
    });
    let persisted_payload_hash =
        if matches!(mutation, RawModelPublicationMutation::PayloadHashDrift) {
            "d".repeat(64)
        } else {
            sha256_json(&payload)
        };
    let pass_watermark = if matches!(mutation, RawModelPublicationMutation::UnitWatermarkDrift) {
        json!({"not_a_final_seal": true})
    } else {
        json!({
            "handoff_id": handoff_id,
            "deliverable_submission_id": fixture.submission_id,
            "scope_hash": fixture.scope_hash,
            "coverage_watermark": coverage_watermark,
            "gate_decision_hash": gate_decision_hash,
            "evidence_watermark": fixture.evidence_id,
        })
    };
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET status='passed',checkpoint_version=1,checkpoint=$2,
                  evidence_watermark=$3,lease_token=NULL,
                  lease_owner=NULL,lease_acquired_at=NULL,lease_expires_at=NULL,
                  heartbeat_at=NULL,terminal_at=transaction_timestamp(),updated_at=NOW()
            WHERE id=$1"#,
    )
    .bind(fixture.worker_run_id)
    .bind(terminal_checkpoint)
    .bind(fixture.evidence_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"UPDATE stage_run_units
              SET status='passed',row_version=row_version+1,pass_watermark=$2,
                  terminal_at=transaction_timestamp(),updated_at=NOW()
            WHERE id=$1"#,
    )
    .bind(fixture.stage_run_unit_id)
    .bind(pass_watermark)
    .execute(&mut *tx)
    .await?;
    if matches!(mutation, RawModelPublicationMutation::CompletionDrift) {
        sqlx::query(
            r#"INSERT INTO org_stage_completions(
                   organization_id,stage_kind,passed_at,stage_run_id,updated_at
               ) VALUES($1,'application_understanding',
                        transaction_timestamp()-INTERVAL '1 minute','foreign-operation',NOW())"#,
        )
        .bind(fixture.organization_id)
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query(
            r#"INSERT INTO org_stage_completions(
                   organization_id,stage_kind,passed_at,stage_run_id,updated_at
               ) VALUES($1,'application_understanding',transaction_timestamp(),$2,NOW())"#,
        )
        .bind(fixture.organization_id)
        .bind(fixture.operation_id.to_string())
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        r#"INSERT INTO stage_handoffs(
               id,operation_id,organization_id,scope_snapshot_id,from_stage_kind,
               stage_execution_id,source_stage_run_unit_id,deliverable_submission_id,
               scope_hash,payload,payload_sha256,evidence_ids,coverage_watermark,
               unit_gate_decision_hash,gate_passed_at
           ) VALUES($1,$2,$3,$4,'application_understanding',$5,$6,$7,$8,$9,$10,$11,$12,$13,
                    transaction_timestamp())"#,
    )
    .bind(handoff_id)
    .bind(fixture.operation_id)
    .bind(fixture.organization_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.stage_execution_id)
    .bind(fixture.stage_run_unit_id)
    .bind(fixture.submission_id)
    .bind(&fixture.scope_hash)
    .bind(payload)
    .bind(persisted_payload_hash)
    .bind(vec![fixture.evidence_id])
    .bind(coverage_watermark)
    .bind(&gate_decision_hash)
    .execute(&mut *tx)
    .await?;
    let late_predecessor = if matches!(
        mutation,
        RawModelPublicationMutation::LatePredecessorHandoffInsert
    ) {
        let execution_id = Uuid::new_v4();
        let unit_id = Uuid::new_v4();
        let worker_id = Uuid::new_v4();
        let lease_token = Uuid::new_v4();
        let tool_id = Uuid::new_v4();
        let submission_id = Uuid::new_v4();
        let handoff_id = Uuid::new_v4();
        let payload = json!({
            "schema_version": 1,
            "organization_id": fixture.organization_id,
            "routes": ["/late-predecessor"],
        });
        sqlx::query(
            "INSERT INTO stage_runs(id,operation_id,stage_kind,status) \
             VALUES($1,$2,'vuln_triage','completed')",
        )
        .bind(execution_id)
        .bind(fixture.operation_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"INSERT INTO stage_run_units(
                   id,operation_id,stage_execution_id,scope_snapshot_id,
                   organization_id,stage_kind,generation,specialist,status,
                   terminal_at,pass_watermark
               ) VALUES($1,$2,$3,$4,$5,'vuln_triage',0,'vuln_triage','passed',
                        transaction_timestamp(),'{"final_gate_passed":true}'::JSONB)"#,
        )
        .bind(unit_id)
        .bind(fixture.operation_id)
        .bind(execution_id)
        .bind(fixture.scope_snapshot_id)
        .bind(fixture.organization_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"INSERT INTO stage_worker_runs(
                   id,operation_id,stage_execution_id,stage_run_unit_id,
                   organization_id,worker_generation,specialist,work_item_kind,
                   work_item_key,agent_path,status,lease_token,lease_owner,
                   lease_acquired_at,lease_expires_at,heartbeat_at,attempt_epoch
               ) VALUES($1,$2,$3,$4,$5,0,'vuln_triage','stage_unit','vuln_triage',
                        'main>late-vuln-triage','running',$6,'raw-late-source',
                        NOW(),NOW()+INTERVAL '5 minutes',NOW(),0)"#,
        )
        .bind(worker_id)
        .bind(fixture.operation_id)
        .bind(execution_id)
        .bind(unit_id)
        .bind(fixture.organization_id)
        .bind(lease_token)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"INSERT INTO tool_calls(
                   id,call_id,session_id,task_id,agent,name,args,result,status,
                   operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
                   organization_id,attempt_epoch,lease_token
               ) VALUES($1,$2,$3,$4,'primary','submit_stage_deliverable','{}','{}',
                        'finished',$4,$5,$6,$7,$8,0,$9)"#,
        )
        .bind(tool_id)
        .bind(format!("raw-late-source-submit-{tool_id}"))
        .bind(fixture.session_id)
        .bind(fixture.operation_id)
        .bind(execution_id)
        .bind(unit_id)
        .bind(worker_id)
        .bind(fixture.organization_id)
        .bind(lease_token)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"INSERT INTO stage_deliverable_submissions(
                   id,operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
                   organization_id,tool_call_record_id,tool_request_id,stage_kind,
                   attempt_epoch,lease_token,payload,payload_sha256
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,'vuln_triage',0,$9,$10,$11)"#,
        )
        .bind(submission_id)
        .bind(fixture.operation_id)
        .bind(execution_id)
        .bind(unit_id)
        .bind(worker_id)
        .bind(fixture.organization_id)
        .bind(tool_id)
        .bind(format!("raw-late-source-submit-{tool_id}"))
        .bind(lease_token)
        .bind(&payload)
        .bind(sha256_json(&payload))
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE stage_worker_runs SET status='passed',terminal_at=NOW(),updated_at=NOW() \
             WHERE id=$1",
        )
        .bind(worker_id)
        .execute(&mut *tx)
        .await?;
        Some((execution_id, unit_id, submission_id, handoff_id, payload))
    } else {
        None
    };
    if matches!(mutation, RawModelPublicationMutation::RunningSubmitReceipt) {
        sqlx::query(
            "UPDATE tool_calls SET status='running',updated_at=NOW() \
             WHERE id=(SELECT tool_call_record_id FROM stage_deliverable_submissions WHERE id=$1)",
        )
        .bind(fixture.submission_id)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        r#"INSERT INTO application_model_current_revisions(
               manifest_id,revision_id,authority_kind,stage_handoff_id,
               deliverable_submission_id,manifest_hash,model_hash,
               replay_material_hash,gate_decision_hash
           ) VALUES($1,$2,'model',$3,$4,$5,$6,$7,$8)"#,
    )
    .bind(fixture.manifest_id)
    .bind(fixture.revision_id)
    .bind(handoff_id)
    .bind(fixture.submission_id)
    .bind(&fixture.manifest_hash)
    .bind(&fixture.model_hash)
    .bind(&fixture.replay_material_hash)
    .bind(format!("sha256:{gate_decision_hash}"))
    .execute(&mut *tx)
    .await?;
    if matches!(
        mutation,
        RawModelPublicationMutation::LateSourceInvalidation
    ) {
        sqlx::query("SET CONSTRAINTS application_model_current_revision_exact IMMEDIATE")
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE stage_handoffs SET invalidated_at=NOW() WHERE id=$1")
            .bind(fixture.source_handoff_id)
            .execute(&mut *tx)
            .await?;
    }
    if matches!(
        mutation,
        RawModelPublicationMutation::LateSourceUnitStatusDrift
    ) {
        sqlx::query("SET CONSTRAINTS application_model_current_revision_exact IMMEDIATE")
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE stage_run_units SET status='superseded' \
             WHERE id=(SELECT source_stage_run_unit_id FROM stage_handoffs WHERE id=$1)",
        )
        .bind(fixture.source_handoff_id)
        .execute(&mut *tx)
        .await?;
    }
    if let Some((execution_id, unit_id, submission_id, handoff_id, payload)) = late_predecessor {
        sqlx::query("SET CONSTRAINTS application_model_current_revision_exact IMMEDIATE")
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            r#"INSERT INTO stage_handoffs(
                   id,operation_id,organization_id,scope_snapshot_id,from_stage_kind,
                   stage_execution_id,source_stage_run_unit_id,deliverable_submission_id,
                   scope_hash,payload,payload_sha256,evidence_ids,coverage_watermark,
                   unit_gate_decision_hash,gate_passed_at
               ) VALUES($1,$2,$3,$4,'vuln_triage',$5,$6,$7,$8,$9,$10,$11,
                        '{"complete":true}'::JSONB,$12,transaction_timestamp())"#,
        )
        .bind(handoff_id)
        .bind(fixture.operation_id)
        .bind(fixture.organization_id)
        .bind(fixture.scope_snapshot_id)
        .bind(execution_id)
        .bind(unit_id)
        .bind(submission_id)
        .bind(&fixture.scope_hash)
        .bind(&payload)
        .bind(sha256_json(&payload))
        .bind(vec![fixture.evidence_id])
        .bind("6".repeat(64))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

async fn attempt_false_terminal_no_input_publication(
    fixture: &RawApplicationModelFixture,
) -> Result<(), sqlx::Error> {
    let stage_execution_id = Uuid::new_v4();
    let stage_run_unit_id = Uuid::new_v4();
    let worker_run_id = Uuid::new_v4();
    let lease_token = Uuid::new_v4();
    let tool_call_id = Uuid::new_v4();
    let submission_id = Uuid::new_v4();
    let manifest_id = Uuid::new_v4();
    let handoff_id = Uuid::new_v4();
    let gate_decision_hash = "e".repeat(64);
    let manifest_material = json!({
        "schema_version": "application_model_manifest.v1",
        "manifest_id": manifest_id,
        "operation_id": fixture.operation_id,
        "scope_snapshot_id": fixture.scope_snapshot_id,
        "stage_execution_id": stage_execution_id,
        "stage_run_unit_id": stage_run_unit_id,
        "organization_id": fixture.organization_id,
        "authority_kind": "terminal_no_input",
        "inputs": [],
    });
    let manifest_hash = tagged_sha256_json(&manifest_material);
    let coverage_watermark = json!({
        "schema_version": "application_model_coverage.v1",
        "manifest_id": manifest_id,
        "revision_id": null,
        "input_count": 0,
        "decision_count": 0,
        "item_count": 0,
        "manifest_hash": manifest_hash,
        "model_hash": null,
        "replay_material_hash": manifest_hash,
    });
    let terminal_checkpoint = json!({
        "schema_version": "application_model_terminal.v1",
        "manifest_id": manifest_id,
        "revision_id": null,
        "manifest_hash": manifest_hash,
        "model_hash": null,
        "replay_material_hash": manifest_hash,
        "deliverable_submission_id": submission_id,
    });
    let payload = json!({
        "schema_version": 1,
        "canonical_fact_refs": [],
        "typed_claims": [{
            "kind": "application_model_authority",
            "payload": {
                "authority_kind": "terminal_no_input",
                "manifest_id": manifest_id,
                "revision_id": null,
                "manifest_hash": manifest_hash,
                "model_hash": null,
                "replay_material_hash": manifest_hash,
                "deliverable_submission_id": submission_id,
            }
        }],
        "coverage_watermark": coverage_watermark,
        "evidence_ids": [],
    });
    let submission_payload = json!({
        "stage_id": "application_understanding",
        "stage_run_id": stage_execution_id,
        "schema_version": 1,
        "manifest_id": manifest_id,
        "authority_kind": "terminal_no_input",
    });
    let pass_watermark = json!({
        "handoff_id": handoff_id,
        "deliverable_submission_id": submission_id,
        "scope_hash": fixture.scope_hash,
        "coverage_watermark": coverage_watermark,
        "gate_decision_hash": gate_decision_hash,
        "evidence_watermark": null,
    });

    let mut tx = fixture.pool().begin().await?;
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status) \
         VALUES($1,$2,'application_understanding','started')",
    )
    .bind(stage_execution_id)
    .bind(fixture.operation_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,
               organization_id,stage_kind,generation,specialist,status,started_at
           ) VALUES($1,$2,$3,$4,$5,'application_understanding',0,
                    'application_understanding','running',NOW())"#,
    )
    .bind(stage_run_unit_id)
    .bind(fixture.operation_id)
    .bind(stage_execution_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.organization_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,
               organization_id,worker_generation,specialist,work_item_kind,
               work_item_key,agent_path,status,lease_token,lease_owner,
               lease_acquired_at,lease_expires_at,heartbeat_at,attempt_epoch
           ) VALUES($1,$2,$3,$4,$5,0,'application_understanding','stage_unit',
                    'application_understanding','main>terminal-no-input','running',
                    $6,'raw-terminal',NOW(),NOW()+INTERVAL '5 minutes',NOW(),0)"#,
    )
    .bind(worker_run_id)
    .bind(fixture.operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(fixture.organization_id)
    .bind(lease_token)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,result,status,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token
           ) VALUES($1,$2,$3,$4,'primary','submit_stage_deliverable',
                    '{}','{}','finished',$4,$5,$6,$7,$8,0,$9)"#,
    )
    .bind(tool_call_id)
    .bind(format!("raw-terminal-submit-{tool_call_id}"))
    .bind(fixture.session_id)
    .bind(fixture.operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(worker_run_id)
    .bind(fixture.organization_id)
    .bind(lease_token)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO stage_deliverable_submissions(
               id,operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,tool_call_record_id,tool_request_id,stage_kind,
               attempt_epoch,lease_token,payload,payload_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,'application_understanding',0,$9,$10,$11)"#,
    )
    .bind(submission_id)
    .bind(fixture.operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(worker_run_id)
    .bind(fixture.organization_id)
    .bind(tool_call_id)
    .bind(format!("raw-terminal-submit-{tool_call_id}"))
    .bind(lease_token)
    .bind(&submission_payload)
    .bind(sha256_json(&submission_payload))
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO application_model_manifests(
               id,operation_id,scope_snapshot_id,stage_execution_id,
               stage_run_unit_id,organization_id,stage_kind,authority_kind,
               input_count,manifest_hash,replay_material_hash
           ) VALUES($1,$2,$3,$4,$5,$6,'application_understanding','terminal_no_input',
                    0,$7,$7)"#,
    )
    .bind(manifest_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(fixture.organization_id)
    .bind(&manifest_hash)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET status='passed',checkpoint_version=1,checkpoint=$2,lease_token=NULL,
                  lease_owner=NULL,lease_acquired_at=NULL,lease_expires_at=NULL,
                  heartbeat_at=NULL,terminal_at=transaction_timestamp(),updated_at=NOW()
            WHERE id=$1"#,
    )
    .bind(worker_run_id)
    .bind(terminal_checkpoint)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"UPDATE stage_run_units
              SET status='passed',row_version=row_version+1,pass_watermark=$2,
                  terminal_at=transaction_timestamp(),updated_at=NOW()
            WHERE id=$1"#,
    )
    .bind(stage_run_unit_id)
    .bind(pass_watermark)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO org_stage_completions(
               organization_id,stage_kind,passed_at,stage_run_id,updated_at
           ) VALUES($1,'application_understanding',transaction_timestamp(),$2,NOW())"#,
    )
    .bind(fixture.organization_id)
    .bind(fixture.operation_id.to_string())
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO stage_handoffs(
               id,operation_id,organization_id,scope_snapshot_id,from_stage_kind,
               stage_execution_id,source_stage_run_unit_id,deliverable_submission_id,
               scope_hash,payload,payload_sha256,evidence_ids,coverage_watermark,
               unit_gate_decision_hash,gate_passed_at
           ) VALUES($1,$2,$3,$4,'application_understanding',$5,$6,$7,$8,$9,$10,
                    '{}'::BIGINT[],$11,$12,transaction_timestamp())"#,
    )
    .bind(handoff_id)
    .bind(fixture.operation_id)
    .bind(fixture.organization_id)
    .bind(fixture.scope_snapshot_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(submission_id)
    .bind(&fixture.scope_hash)
    .bind(&payload)
    .bind(sha256_json(&payload))
    .bind(coverage_watermark)
    .bind(&gate_decision_hash)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO application_model_current_revisions(
               manifest_id,revision_id,authority_kind,stage_handoff_id,
               deliverable_submission_id,manifest_hash,model_hash,
               replay_material_hash,gate_decision_hash
           ) VALUES($1,NULL,'terminal_no_input',$2,$3,$4,NULL,$4,$5)"#,
    )
    .bind(manifest_id)
    .bind(handoff_id)
    .bind(submission_id)
    .bind(manifest_hash)
    .bind(format!("sha256:{gate_decision_hash}"))
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

#[tokio::test]
#[serial]
async fn application_model_barrier_excludes_only_exact_bound_company_synthesizer() {
    let mut fixture = RawApplicationModelFixture::start_team("team-final-submitter-barrier").await;
    assert!(fixture.team_plan_id.is_some());
    assert!(fixture.leader_work_item_id.is_some());
    let sibling_work_item_id = fixture
        .sibling_work_item_id
        .expect("Team fixture owns one pending sibling WorkItem");
    let mut tx = fixture
        .pool()
        .begin()
        .await
        .expect("begin barrier transaction");
    let barrier = application_models::lock_finalize_authority_with_transaction(
        &mut tx,
        &application_models::LockApplicationModelFinalizeAuthority {
            gate: application_models::LoadApplicationModelGateMaterial {
                manifest_id: fixture.manifest_id,
                operation_id: fixture.operation_id,
                scope_snapshot_id: fixture.scope_snapshot_id,
                stage_execution_id: fixture.stage_execution_id,
                stage_run_unit_id: fixture.stage_run_unit_id,
                organization_id: fixture.organization_id,
            },
            fence: runtime_memory_tx::RuntimeMemoryTxFence {
                operation_id: fixture.operation_id,
                stage_execution_id: fixture.stage_execution_id,
                stage_run_unit_id: fixture.stage_run_unit_id,
                worker_run_id: fixture.worker_run_id,
                lease_token: fixture.lease_token,
                attempt_epoch: 0,
                expected_checkpoint_version: 0,
            },
            deliverable_submission_id: fixture.submission_id,
        },
    )
    .await
    .expect("exact bound company synthesizer owns finalization");

    assert!(barrier.forbidden_activity_refs.is_empty());
    assert_eq!(
        barrier.pending_producer_refs,
        vec![format!("work_item:{sibling_work_item_id}:queued")],
        "only the exact bound leader WorkItem is excluded; an ordinary sibling still blocks"
    );
    tx.rollback()
        .await
        .expect("rollback read-only barrier proof");
    fixture.database.db.stop().await;
}

#[tokio::test]
#[serial]
async fn application_model_finalization_migration_replaces_dormant_blockers_with_bundle_gates() {
    let fixture = ApplicationModelFinalizationDb::start("schema").await;
    let old_triggers = sqlx::query_scalar::<_, String>(
        r#"SELECT tgname
             FROM pg_trigger
            WHERE NOT tgisinternal
              AND tgname IN (
                  'application_model_current_revisions_dormant',
                  'application_model_stage_handoffs_dormant'
              )
            ORDER BY tgname"#,
    )
    .fetch_all(fixture.db.pool())
    .await
    .expect("query old dormant triggers");
    assert!(old_triggers.is_empty(), "old publication blockers remain");

    let expected = [
        "application_model_current_revision_exact",
        "application_model_final_revision_has_pointer",
        "application_model_passed_unit_bundle_exact",
        "application_model_predecessor_handoff_denominator_exact",
        "application_model_predecessor_invalidation_exact",
        "application_model_predecessor_source_unit_exact",
        "application_model_stage_handoff_bundle_exact",
    ];
    for trigger in expected {
        let is_deferred_constraint = sqlx::query_scalar::<_, bool>(
            r#"SELECT EXISTS(
                   SELECT 1
                     FROM pg_trigger
                    WHERE tgname=$1
                      AND NOT tgisinternal
                      AND tgconstraint <> 0
                      AND tgdeferrable
                      AND tginitdeferred
               )"#,
        )
        .bind(trigger)
        .fetch_one(fixture.db.pool())
        .await
        .expect("query application model publication constraint trigger");
        assert!(
            is_deferred_constraint,
            "missing deferred exact-bundle trigger {trigger}"
        );
    }
}

#[tokio::test]
#[serial]
async fn application_model_finalization_raw_exact_bundle_baseline_commits() {
    let fixture = RawApplicationModelFixture::start("raw_exact_baseline").await;

    attempt_raw_model_publication(&fixture, RawModelPublicationMutation::None)
        .await
        .expect("exact raw publication baseline must commit before mutation tests are meaningful");
}

#[tokio::test]
#[serial]
async fn application_model_finalization_freezes_published_runtime_authority() {
    let fixture = RawApplicationModelFixture::start("published_runtime_freeze").await;
    attempt_raw_model_publication(&fixture, RawModelPublicationMutation::None)
        .await
        .expect("publish exact raw baseline");

    let unit_drift =
        sqlx::query("UPDATE stage_run_units SET pass_watermark='{}'::JSONB WHERE id=$1")
            .bind(fixture.stage_run_unit_id)
            .execute(fixture.pool())
            .await;
    assert!(
        unit_drift.is_err(),
        "published Unit authority remained mutable"
    );

    let worker_drift =
        sqlx::query("UPDATE stage_worker_runs SET checkpoint='{}'::JSONB WHERE id=$1")
            .bind(fixture.worker_run_id)
            .execute(fixture.pool())
            .await;
    assert!(
        worker_drift.is_err(),
        "published Worker authority remained mutable"
    );

    let submit_tool_drift = sqlx::query(
        "UPDATE tool_calls SET status='running' \
         WHERE id=(SELECT tool_call_record_id FROM stage_deliverable_submissions WHERE id=$1)",
    )
    .bind(fixture.submission_id)
    .execute(fixture.pool())
    .await;
    assert!(
        submit_tool_drift.is_err(),
        "published submit-tool authority remained mutable"
    );

    let late_worker = sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,
               organization_id,worker_generation,specialist,work_item_kind,
               work_item_key,agent_path,status,attempt_epoch
           ) VALUES($1,$2,$3,$4,$5,0,'application_understanding','late_worker',
                    'late-worker','main>late-worker','queued',0)"#,
    )
    .bind(Uuid::new_v4())
    .bind(fixture.operation_id)
    .bind(fixture.stage_execution_id)
    .bind(fixture.stage_run_unit_id)
    .bind(fixture.organization_id)
    .execute(fixture.pool())
    .await;
    assert!(
        late_worker.is_err(),
        "published Unit accepted a new sibling Worker"
    );

    let late_tool = sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,result,status,
               operation_id,stage_execution_id,stage_run_unit_id,organization_id
           ) VALUES($1,'late-tool',$2,$3,'primary','browser_open','{}','{}','received',
                    $3,$4,$5,$6)"#,
    )
    .bind(Uuid::new_v4())
    .bind(fixture.session_id)
    .bind(fixture.operation_id)
    .bind(fixture.stage_execution_id)
    .bind(fixture.stage_run_unit_id)
    .bind(fixture.organization_id)
    .execute(fixture.pool())
    .await;
    assert!(late_tool.is_err(), "published Unit accepted a new tool row");
}

#[tokio::test]
#[serial]
async fn application_model_finalization_rejects_running_submit_receipt() {
    let fixture = RawApplicationModelFixture::start("running_submit_receipt").await;

    let commit =
        attempt_raw_model_publication(&fixture, RawModelPublicationMutation::RunningSubmitReceipt)
            .await;

    assert!(commit.is_err(), "running submit receipt became authority");
}

#[tokio::test]
#[serial]
async fn application_model_finalization_rechecks_late_source_invalidation() {
    let fixture = RawApplicationModelFixture::start("late_source_invalidation").await;

    let commit = attempt_raw_model_publication(
        &fixture,
        RawModelPublicationMutation::LateSourceInvalidation,
    )
    .await;

    assert!(
        commit.is_err(),
        "source invalidation bypassed an already-immediate current constraint"
    );
}

#[tokio::test]
#[serial]
async fn application_model_finalization_rechecks_late_predecessor_handoff_insert() {
    let fixture = RawApplicationModelFixture::start("late_predecessor_handoff").await;

    let commit = attempt_raw_model_publication(
        &fixture,
        RawModelPublicationMutation::LatePredecessorHandoffInsert,
    )
    .await;

    assert!(
        commit.is_err(),
        "new closed-predecessor Handoff bypassed the frozen manifest denominator"
    );
}

#[tokio::test]
#[serial]
async fn application_model_finalization_rechecks_late_source_unit_status_drift() {
    let fixture = RawApplicationModelFixture::start("late_source_status").await;

    let commit = attempt_raw_model_publication(
        &fixture,
        RawModelPublicationMutation::LateSourceUnitStatusDrift,
    )
    .await;

    assert!(
        commit.is_err(),
        "adopted predecessor Unit drifted away from passed after publication"
    );
}

#[tokio::test]
#[serial]
async fn application_model_finalization_raw_bundle_rejects_extra_typed_claim() {
    let fixture = RawApplicationModelFixture::start("extra_claim").await;
    let commit =
        attempt_raw_model_publication(&fixture, RawModelPublicationMutation::ExtraClaim).await;
    assert!(commit.is_err(), "deferred authority accepted extra claim");
}

#[tokio::test]
#[serial]
async fn application_model_finalization_raw_bundle_rejects_extra_canonical_ref() {
    let fixture = RawApplicationModelFixture::start("extra_ref").await;
    let commit =
        attempt_raw_model_publication(&fixture, RawModelPublicationMutation::ExtraCanonicalRef)
            .await;
    assert!(
        commit.is_err(),
        "deferred authority accepted extra canonical ref"
    );
}

#[tokio::test]
#[serial]
async fn application_model_finalization_raw_bundle_rejects_payload_hash_drift() {
    let fixture = RawApplicationModelFixture::start("payload_hash_drift").await;
    let commit =
        attempt_raw_model_publication(&fixture, RawModelPublicationMutation::PayloadHashDrift)
            .await;
    assert!(
        commit.is_err(),
        "deferred authority accepted payload hash drift"
    );
}

#[tokio::test]
#[serial]
async fn application_model_finalization_raw_bundle_rejects_payload_evidence_drift() {
    let fixture = RawApplicationModelFixture::start("payload_evidence_drift").await;
    let commit =
        attempt_raw_model_publication(&fixture, RawModelPublicationMutation::PayloadEvidenceDrift)
            .await;
    assert!(
        commit.is_err(),
        "deferred authority accepted payload evidence drift"
    );
}

#[tokio::test]
#[serial]
async fn application_model_finalization_raw_bundle_rejects_payload_coverage_drift() {
    let fixture = RawApplicationModelFixture::start("payload_coverage_drift").await;
    let commit =
        attempt_raw_model_publication(&fixture, RawModelPublicationMutation::PayloadCoverageDrift)
            .await;
    assert!(
        commit.is_err(),
        "deferred authority accepted payload coverage drift"
    );
}

#[tokio::test]
#[serial]
async fn application_model_finalization_raw_bundle_rejects_unit_watermark_drift() {
    let fixture = RawApplicationModelFixture::start("unit_watermark_drift").await;
    let commit =
        attempt_raw_model_publication(&fixture, RawModelPublicationMutation::UnitWatermarkDrift)
            .await;
    assert!(
        commit.is_err(),
        "deferred authority accepted Unit watermark drift"
    );
}

#[tokio::test]
#[serial]
async fn application_model_finalization_raw_bundle_rejects_completion_drift() {
    let fixture = RawApplicationModelFixture::start("completion_drift").await;
    let commit =
        attempt_raw_model_publication(&fixture, RawModelPublicationMutation::CompletionDrift).await;
    assert!(
        commit.is_err(),
        "deferred authority accepted completion drift"
    );
}

#[tokio::test]
#[serial]
async fn application_model_finalization_rejects_false_terminal_no_input_with_source_handoff() {
    let fixture = RawApplicationModelFixture::start("false_terminal_no_input").await;
    assert_ne!(
        fixture.source_handoff_id,
        Uuid::nil(),
        "fixture must contain a final source Handoff"
    );

    let commit = attempt_false_terminal_no_input_publication(&fixture).await;

    assert!(
        commit.is_err(),
        "deferred authority accepted terminal_no_input despite a final source Handoff"
    );
}
