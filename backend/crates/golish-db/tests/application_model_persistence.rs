use golish_db::models::NewSession;
use golish_db::repo::application_models::{
    self, ApplicationModelAuthorityKindRow, ApplicationModelEvidenceRoleRow,
    ApplicationModelInputDecisionSeed, ApplicationModelInputDispositionRow,
    ApplicationModelItemEvidenceSeed, ApplicationModelItemSeed, ApplicationModelManifestInputSeed,
    ApplicationModelTruthStateRow, DeriveApplicationModelManifestSeed,
    LoadApplicationModelGateMaterial, ProposeApplicationModelRevision,
    SeedApplicationModelManifest,
};
use golish_db::repo::{project_scopes, runtime_memory_tx, sessions};
use golish_db::{DbConfig, GolishDb};
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

#[derive(Debug, Clone)]
struct ApplicationModelOwner {
    organization_id: Uuid,
    stage_run_unit_id: Uuid,
    source_handoff_id: Uuid,
    source_payload: serde_json::Value,
    evidence_id: i64,
}

struct ApplicationModelAuthorityFixture {
    db: GolishDb,
    _data_dir: TempDir,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    stage_execution_id: Uuid,
    owner_a: ApplicationModelOwner,
    owner_b: ApplicationModelOwner,
    terminal_organization_id: Uuid,
    terminal_stage_run_unit_id: Uuid,
}

impl ApplicationModelAuthorityFixture {
    async fn start(label: &str) -> Self {
        let ApplicationModelDbFixture { db, _data_dir } =
            ApplicationModelDbFixture::start(label).await;
        let workspace = format!("/tmp/application-model-{}", Uuid::new_v4().simple());
        let project = project_scopes::register_first_open(db.pool(), &workspace, &"1".repeat(64))
            .await
            .expect("register application model project scope");
        let session = sessions::create(
            db.pool(),
            NewSession {
                title: Some("Application Model persistence fixture".to_string()),
                workspace_path: Some(workspace.clone()),
                workspace_label: None,
                model: Some("fixture-model".to_string()),
                provider: Some("fixture-provider".to_string()),
                project_path: Some(workspace.clone()),
            },
        )
        .await
        .expect("create application model fixture session");
        let operation_id = Uuid::new_v4();
        let scoping_execution_id = Uuid::new_v4();
        runtime_memory_tx::create_runtime_operation(
            db.pool(),
            &runtime_memory_tx::CreateRuntimeOperationRow {
                operation_id,
                initial_stage_execution_id: scoping_execution_id,
                session_id: session.id,
                title: Some("Application Model persistence fixture".to_string()),
                input: "fixture".to_string(),
                profile: "red_team".to_string(),
                entry_stage: "scoping".to_string(),
                application_model_contract:
                    golish_core::ApplicationModelContract::ApplicationModelV1,
                project_scope_id: project.project_scope_id,
                cli_scope: None,
            },
        )
        .await
        .expect("create application model fixture operation");

        let organization_a = Uuid::new_v4();
        let organization_b = Uuid::new_v4();
        let terminal_organization_id = Uuid::new_v4();
        let scope_decision_id = Uuid::new_v4();
        let scope_snapshot_id = Uuid::new_v4();
        let mut tx = db.pool().begin().await.expect("begin scope fixture");
        sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$2,'Org A')")
            .bind(organization_a)
            .bind(&workspace)
            .execute(&mut *tx)
            .await
            .expect("insert root organization");
        sqlx::query(
            "INSERT INTO organizations(id,project_path,name,parent_id) VALUES($1,$2,'Org B',$3)",
        )
        .bind(organization_b)
        .bind(&workspace)
        .bind(organization_a)
        .execute(&mut *tx)
        .await
        .expect("insert subsidiary organization");
        sqlx::query(
            "INSERT INTO organizations(id,project_path,name,parent_id) VALUES($1,$2,'Org C',$3)",
        )
        .bind(terminal_organization_id)
        .bind(&workspace)
        .bind(organization_a)
        .execute(&mut *tx)
        .await
        .expect("insert zero-input subsidiary organization");
        sqlx::query(
            r#"INSERT INTO operation_scope_decisions(
                   id,operation_id,project_scope_id,stage_execution_id,
                   root_organization_id,mode,decision_rows,decision_hash
               ) VALUES($1,$2,$3,$4,$5,'cli_flags',$6,$7)"#,
        )
        .bind(scope_decision_id)
        .bind(operation_id)
        .bind(project.project_scope_id)
        .bind(scoping_execution_id)
        .bind(organization_a)
        .bind(serde_json::json!([
            {"organization_id": organization_a},
            {"organization_id": organization_b},
            {"organization_id": terminal_organization_id}
        ]))
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
        .bind(scope_snapshot_id)
        .bind(operation_id)
        .bind(project.project_scope_id)
        .bind(scope_decision_id)
        .bind(&workspace)
        .bind(organization_a)
        .bind("3".repeat(64))
        .execute(&mut *tx)
        .await
        .expect("insert scope snapshot");
        sqlx::query(
            r#"INSERT INTO operation_org_scope_units(
                   snapshot_id,organization_id,organization_name_at_freeze,
                   role,parent_organization_id,depth,ordinal,decision_row_id,approval_source
               ) VALUES
                   ($1,$2,'Org A','root',NULL,0,0,'org-a',$5),
                   ($1,$3,'Org B','subsidiary',$2,1,1,'org-b',$5),
                   ($1,$4,'Org C','subsidiary',$2,1,2,'org-c',$5)"#,
        )
        .bind(scope_snapshot_id)
        .bind(organization_a)
        .bind(organization_b)
        .bind(terminal_organization_id)
        .bind(serde_json::json!({"source": "application_model_fixture"}))
        .execute(&mut *tx)
        .await
        .expect("insert frozen scope units");
        sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
            .bind(scope_snapshot_id)
            .execute(&mut *tx)
            .await
            .expect("seal scope snapshot");
        tx.commit().await.expect("commit scope fixture");

        let source_execution_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO stage_runs(id,operation_id,stage_kind,status) \
             VALUES($1,$2,'vuln_triage','completed')",
        )
        .bind(source_execution_id)
        .bind(operation_id)
        .execute(db.pool())
        .await
        .expect("insert source stage run");
        let owner_a = insert_source_handoff(
            &db,
            session.id,
            operation_id,
            scope_snapshot_id,
            source_execution_id,
            organization_a,
            &workspace,
            "api-a",
        )
        .await;
        let owner_b = insert_source_handoff(
            &db,
            session.id,
            operation_id,
            scope_snapshot_id,
            source_execution_id,
            organization_b,
            &workspace,
            "api-b",
        )
        .await;

        let stage_execution_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO stage_runs(id,operation_id,stage_kind,status) \
             VALUES($1,$2,'application_understanding','started')",
        )
        .bind(stage_execution_id)
        .bind(operation_id)
        .execute(db.pool())
        .await
        .expect("insert dormant Application Understanding stage run");
        for owner in [&owner_a, &owner_b] {
            sqlx::query(
                r#"INSERT INTO stage_run_units(
                       id,operation_id,stage_execution_id,scope_snapshot_id,
                       organization_id,stage_kind,generation,specialist,status
                   ) VALUES($1,$2,$3,$4,$5,'application_understanding',0,
                            'application_understanding','queued')"#,
            )
            .bind(owner.stage_run_unit_id)
            .bind(operation_id)
            .bind(stage_execution_id)
            .bind(scope_snapshot_id)
            .bind(owner.organization_id)
            .execute(db.pool())
            .await
            .expect("insert dormant Application Understanding unit");
        }
        let terminal_stage_run_unit_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO stage_run_units(
                   id,operation_id,stage_execution_id,scope_snapshot_id,
                   organization_id,stage_kind,generation,specialist,status
               ) VALUES($1,$2,$3,$4,$5,'application_understanding',0,
                        'application_understanding','queued')"#,
        )
        .bind(terminal_stage_run_unit_id)
        .bind(operation_id)
        .bind(stage_execution_id)
        .bind(scope_snapshot_id)
        .bind(terminal_organization_id)
        .execute(db.pool())
        .await
        .expect("insert zero-input Application Understanding unit");

        Self {
            db,
            _data_dir,
            operation_id,
            scope_snapshot_id,
            stage_execution_id,
            owner_a,
            owner_b,
            terminal_organization_id,
            terminal_stage_run_unit_id,
        }
    }

    fn model_seed(&self, owner: &ApplicationModelOwner) -> SeedApplicationModelManifest {
        SeedApplicationModelManifest {
            operation_id: self.operation_id,
            scope_snapshot_id: self.scope_snapshot_id,
            stage_execution_id: self.stage_execution_id,
            stage_run_unit_id: owner.stage_run_unit_id,
            organization_id: owner.organization_id,
            authority_kind: ApplicationModelAuthorityKindRow::Model,
            inputs: vec![ApplicationModelManifestInputSeed {
                input_key: "vuln-handoff".to_string(),
                input_kind: "stage_handoff".to_string(),
                source_handoff_id: owner.source_handoff_id,
                source_kind: "vuln_triage".to_string(),
                source_id: owner.source_handoff_id.to_string(),
                source_version: 1,
                source_payload: owner.source_payload.clone(),
                evidence_ids: vec![owner.evidence_id],
            }],
        }
    }

    async fn insert_application_submission(
        &self,
        owner: &ApplicationModelOwner,
        payload: &serde_json::Value,
    ) -> Uuid {
        let worker_run_id = Uuid::new_v4();
        let lease_token = Uuid::new_v4();
        let tool_call_id = Uuid::new_v4();
        let submission_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO stage_worker_runs(
                   id,operation_id,stage_execution_id,stage_run_unit_id,
                   organization_id,worker_generation,specialist,work_item_kind,
                   work_item_key,agent_path,status,lease_token,lease_owner,
                   lease_acquired_at,lease_expires_at,heartbeat_at,attempt_epoch
               ) VALUES($1,$2,$3,$4,$5,0,'application_understanding','stage_unit',
                        'application_understanding','main>application_understanding','running',
                        $6,'application-model-fixture',NOW(),NOW()+INTERVAL '5 minutes',NOW(),0)"#,
        )
        .bind(worker_run_id)
        .bind(self.operation_id)
        .bind(self.stage_execution_id)
        .bind(owner.stage_run_unit_id)
        .bind(owner.organization_id)
        .bind(lease_token)
        .execute(self.db.pool())
        .await
        .expect("insert Application Understanding worker");
        sqlx::query(
            r#"INSERT INTO tool_calls(
                   id,call_id,session_id,task_id,agent,name,args,result,status,
                   operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
                   organization_id,attempt_epoch,lease_token
               ) SELECT $1,$2,session_id,$3,'primary','submit_stage_deliverable','{}','{}',
                        'finished',$3,$4,$5,$6,$7,0,$8
                   FROM tasks WHERE id=$3"#,
        )
        .bind(tool_call_id)
        .bind(format!(
            "application-model-proposal-{}",
            owner.organization_id
        ))
        .bind(self.operation_id)
        .bind(self.stage_execution_id)
        .bind(owner.stage_run_unit_id)
        .bind(worker_run_id)
        .bind(owner.organization_id)
        .bind(lease_token)
        .execute(self.db.pool())
        .await
        .expect("insert Application Understanding submission tool call");
        sqlx::query(
            r#"INSERT INTO stage_deliverable_submissions(
                   id,operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
                   organization_id,tool_call_record_id,tool_request_id,stage_kind,
                   attempt_epoch,lease_token,payload,payload_sha256
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,'application_understanding',0,$9,$10,$11)"#,
        )
        .bind(submission_id)
        .bind(self.operation_id)
        .bind(self.stage_execution_id)
        .bind(owner.stage_run_unit_id)
        .bind(worker_run_id)
        .bind(owner.organization_id)
        .bind(tool_call_id)
        .bind(format!(
            "application-model-proposal-{}",
            owner.organization_id
        ))
        .bind(lease_token)
        .bind(payload)
        .bind(sha256_json(payload))
        .execute(self.db.pool())
        .await
        .expect("insert Application Understanding submission");
        submission_id
    }

    async fn valid_proposal(
        &self,
        owner: &ApplicationModelOwner,
    ) -> (Uuid, ProposeApplicationModelRevision) {
        let seeded = application_models::seed_manifest(self.db.pool(), &self.model_seed(owner))
            .await
            .expect("seed manifest before valid proposal");
        let structured_model = serde_json::json!({
            "organization_id": owner.organization_id,
            "summary": "Fixture application with an observed API route",
            "technologies": [],
            "routes_and_pages": ["route:api-a"],
            "api_surfaces": [],
            "roles_and_identities": [],
            "business_entities": [],
            "workflows": [],
            "state_transitions": [],
            "ownership_rules": [],
            "sensitive_operations": [],
            "trust_boundaries": [],
            "unknowns": [],
        });
        let decisions = vec![ApplicationModelInputDecisionSeed {
            input_key: "vuln-handoff".to_string(),
            disposition: ApplicationModelInputDispositionRow::Incorporated,
            item_keys: vec!["route:api-a".to_string()],
            duplicate_input_key: None,
            reason_code: None,
        }];
        let items = vec![ApplicationModelItemSeed {
            item_key: "route:api-a".to_string(),
            item_kind: "business_route".to_string(),
            truth_state: ApplicationModelTruthStateRow::Observed,
            source_input_keys: vec!["vuln-handoff".to_string()],
            referenced_item_keys: Vec::new(),
            payload: serde_json::json!({"method": "GET", "path": "/api-a"}),
            evidence: vec![ApplicationModelItemEvidenceSeed {
                evidence_id: owner.evidence_id,
                role: ApplicationModelEvidenceRoleRow::Observation,
            }],
        }];
        let submission_payload = serde_json::json!({
            "schema_version": 1,
            "manifest_id": seeded.manifest.id,
            "structured_model": structured_model.clone(),
            "decisions": [{
                "input_key": "vuln-handoff",
                "disposition": "incorporated",
                "item_keys": ["route:api-a"],
                "duplicate_input_key": null,
                "reason_code": null,
            }],
            "items": [{
                "item_key": "route:api-a",
                "item_kind": "business_route",
                "truth_state": "observed",
                "source_input_keys": ["vuln-handoff"],
                "referenced_item_keys": [],
                "payload": {"method": "GET", "path": "/api-a"},
                "evidence": [{
                    "evidence_id": owner.evidence_id,
                    "role": "observation",
                }],
            }],
        });
        let source_submission_id = self
            .insert_application_submission(owner, &submission_payload)
            .await;
        let proposal = ProposeApplicationModelRevision {
            manifest_id: seeded.manifest.id,
            operation_id: self.operation_id,
            scope_snapshot_id: self.scope_snapshot_id,
            stage_execution_id: self.stage_execution_id,
            stage_run_unit_id: owner.stage_run_unit_id,
            organization_id: owner.organization_id,
            source_submission_id,
            structured_model,
            decisions,
            items,
        };
        (seeded.manifest.id, proposal)
    }
}

#[allow(clippy::too_many_arguments)]
async fn insert_source_handoff(
    db: &GolishDb,
    session_id: Uuid,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    stage_execution_id: Uuid,
    organization_id: Uuid,
    workspace: &str,
    route: &str,
) -> ApplicationModelOwner {
    let stage_run_unit_id = Uuid::new_v4();
    let worker_run_id = Uuid::new_v4();
    let lease_token = Uuid::new_v4();
    let tool_call_id = Uuid::new_v4();
    let submission_id = Uuid::new_v4();
    let source_handoff_id = Uuid::new_v4();
    let evidence_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO audit_log(
               action,category,details,project_path,audit_role,run_id,detail
           ) VALUES('application model source','attack','',$1,'evidence',$2,$3)
           RETURNING id"#,
    )
    .bind(workspace)
    .bind(operation_id)
    .bind(serde_json::json!({"organization_id": organization_id, "route": route}))
    .fetch_one(db.pool())
    .await
    .expect("insert source evidence");
    let source_payload = serde_json::json!({
        "schema_version": 1,
        "organization_id": organization_id,
        "routes": [route],
    });
    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,
               organization_id,stage_kind,generation,specialist,status,
               terminal_at,pass_watermark
           ) VALUES($1,$2,$3,$4,$5,'vuln_triage',0,'vuln_triage','passed',NOW(),$6)"#,
    )
    .bind(stage_run_unit_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(serde_json::json!({"final_gate_passed": true}))
    .execute(db.pool())
    .await
    .expect("insert final source unit");
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,
               organization_id,worker_generation,specialist,work_item_kind,
               work_item_key,agent_path,status,lease_token,lease_owner,
               lease_acquired_at,lease_expires_at,heartbeat_at,attempt_epoch
           ) VALUES($1,$2,$3,$4,$5,0,'vuln_triage','stage_unit','vuln_triage',
                    'main>vuln_triage','running',$6,'application-model-fixture',
                    NOW(),NOW()+INTERVAL '5 minutes',NOW(),0)"#,
    )
    .bind(worker_run_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(organization_id)
    .bind(lease_token)
    .execute(db.pool())
    .await
    .expect("insert source worker");
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,result,status,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token
           ) VALUES($1,$2,$3,$4,'primary','submit_stage_deliverable','{}','{}',
                    'finished',$4,$5,$6,$7,$8,0,$9)"#,
    )
    .bind(tool_call_id)
    .bind(format!("application-model-source-{organization_id}"))
    .bind(session_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(worker_run_id)
    .bind(organization_id)
    .bind(lease_token)
    .execute(db.pool())
    .await
    .expect("insert source tool call");
    sqlx::query(
        r#"INSERT INTO stage_deliverable_submissions(
               id,operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,tool_call_record_id,tool_request_id,stage_kind,
               attempt_epoch,lease_token,payload,payload_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,'vuln_triage',0,$9,$10,$11)"#,
    )
    .bind(submission_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(worker_run_id)
    .bind(organization_id)
    .bind(tool_call_id)
    .bind(format!("application-model-source-{organization_id}"))
    .bind(lease_token)
    .bind(&source_payload)
    .bind(sha256_json(&source_payload))
    .execute(db.pool())
    .await
    .expect("insert source submission");
    sqlx::query(
        "UPDATE stage_worker_runs SET status='passed',terminal_at=NOW(),updated_at=NOW() \
         WHERE id=$1",
    )
    .bind(worker_run_id)
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
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(submission_id)
    .bind("4".repeat(64))
    .bind(&source_payload)
    .bind(sha256_json(&source_payload))
    .bind(vec![evidence_id])
    .bind(serde_json::json!({"complete": true}))
    .bind("5".repeat(64))
    .execute(db.pool())
    .await
    .expect("insert final source handoff");

    ApplicationModelOwner {
        organization_id,
        stage_run_unit_id: Uuid::new_v4(),
        source_handoff_id,
        source_payload,
        evidence_id,
    }
}

struct ApplicationModelDbFixture {
    db: GolishDb,
    _data_dir: TempDir,
}

impl ApplicationModelDbFixture {
    async fn start(label: &str) -> Self {
        let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
        let config = DbConfig {
            pg_data_dir: data_dir.path().join("pgdata"),
            port: reserve_local_port(),
            database: format!("application_model_{label}_{}", Uuid::new_v4().simple()),
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

#[tokio::test]
#[serial]
async fn application_model_persistence_migration_exposes_dormant_authority_tables() {
    let fixture = ApplicationModelDbFixture::start("schema").await;
    let expected = [
        "application_model_manifests",
        "application_model_manifest_inputs",
        "application_model_revisions",
        "application_model_input_decisions",
        "application_model_items",
        "application_model_item_evidence",
        "application_model_current_revisions",
    ];

    for table in expected {
        let exists = sqlx::query_scalar::<_, bool>("SELECT to_regclass($1) IS NOT NULL")
            .bind(format!("public.{table}"))
            .fetch_one(fixture.db.pool())
            .await
            .expect("query application model table registration");
        assert!(exists, "expected migrated table {table}");
    }
}

#[test]
fn application_model_persistence_public_seed_contract_distinguishes_terminal_no_input() {
    let seed = SeedApplicationModelManifest {
        operation_id: Uuid::new_v4(),
        scope_snapshot_id: Uuid::new_v4(),
        stage_execution_id: Uuid::new_v4(),
        stage_run_unit_id: Uuid::new_v4(),
        organization_id: Uuid::new_v4(),
        authority_kind: ApplicationModelAuthorityKindRow::TerminalNoInput,
        inputs: Vec::new(),
    };

    assert_eq!(seed.authority_kind.as_str(), "terminal_no_input");
    assert!(seed.inputs.is_empty());
}

#[tokio::test]
#[serial]
async fn application_model_persistence_seeds_server_owned_manifest_and_replays_exactly() {
    let fixture = ApplicationModelAuthorityFixture::start("manifest_seed").await;
    let seed = fixture.model_seed(&fixture.owner_a);

    let created = application_models::seed_manifest(fixture.db.pool(), &seed)
        .await
        .expect("seed exact Application Model manifest");
    let replayed = application_models::seed_manifest(fixture.db.pool(), &seed)
        .await
        .expect("replay exact Application Model manifest");

    assert!(!created.replayed);
    assert!(replayed.replayed);
    assert_eq!(created.manifest, replayed.manifest);
    assert_eq!(created.manifest.input_count, 1);
    assert_eq!(created.manifest.authority_kind, "model");
    assert!(created.manifest.manifest_hash.starts_with("sha256:"));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_model_manifests WHERE stage_run_unit_id=$1",
        )
        .bind(fixture.owner_a.stage_run_unit_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count exact replay rows"),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_model_current_revisions WHERE manifest_id=$1",
        )
        .bind(created.manifest.id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count dormant publication rows"),
        0,
        "S1 seed must not publish a Gate result",
    );
}

#[tokio::test]
#[serial]
async fn application_model_persistence_runtime_manifest_is_derived_from_current_predecessors() {
    let fixture = ApplicationModelAuthorityFixture::start("runtime_manifest").await;

    let seeded = application_models::seed_manifest_from_current_predecessors(
        fixture.db.pool(),
        &DeriveApplicationModelManifestSeed {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            stage_execution_id: fixture.stage_execution_id,
            stage_run_unit_id: fixture.owner_a.stage_run_unit_id,
            organization_id: fixture.owner_a.organization_id,
        },
    )
    .await
    .expect("derive exact model manifest from current predecessor handoffs");
    let rows = sqlx::query_as::<_, (String, String, Uuid, Vec<i64>)>(
        r#"SELECT input_key,input_kind,source_handoff_id,evidence_ids
             FROM application_model_manifest_inputs
            WHERE manifest_id=$1
            ORDER BY ordinal"#,
    )
    .bind(seeded.manifest.id)
    .fetch_all(fixture.db.pool())
    .await
    .expect("load derived manifest inputs");

    assert_eq!(seeded.manifest.authority_kind, "model");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].0,
        format!("vuln_triage:{}", fixture.owner_a.source_handoff_id)
    );
    assert_eq!(rows[0].1, "vuln_triage");
    assert_eq!(rows[0].2, fixture.owner_a.source_handoff_id);
    assert_eq!(rows[0].3, vec![fixture.owner_a.evidence_id]);
}

#[tokio::test]
#[serial]
async fn application_model_persistence_runtime_manifest_derives_true_zero_input() {
    let fixture = ApplicationModelAuthorityFixture::start("runtime_manifest_empty").await;

    let seeded = application_models::seed_manifest_from_current_predecessors(
        fixture.db.pool(),
        &DeriveApplicationModelManifestSeed {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            stage_execution_id: fixture.stage_execution_id,
            stage_run_unit_id: fixture.terminal_stage_run_unit_id,
            organization_id: fixture.terminal_organization_id,
        },
    )
    .await
    .expect("derive terminal-no-input only from a true empty predecessor set");

    assert_eq!(seeded.manifest.authority_kind, "terminal_no_input");
    assert_eq!(seeded.manifest.input_count, 0);
}

#[tokio::test]
#[serial]
async fn application_model_persistence_rejects_cross_org_source_and_rolls_back_manifest() {
    let fixture = ApplicationModelAuthorityFixture::start("cross_org").await;
    let mut seed = fixture.model_seed(&fixture.owner_a);
    seed.inputs[0].source_handoff_id = fixture.owner_b.source_handoff_id;
    seed.inputs[0].source_id = fixture.owner_b.source_handoff_id.to_string();
    seed.inputs[0].source_payload = fixture.owner_b.source_payload.clone();
    seed.inputs[0].evidence_ids = vec![fixture.owner_b.evidence_id];

    let error = application_models::seed_manifest(fixture.db.pool(), &seed)
        .await
        .expect_err("cross-organization source must fail closed");

    assert_eq!(error.code(), "manifest_source_denominator_mismatch");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_model_manifests WHERE stage_run_unit_id=$1",
        )
        .bind(fixture.owner_a.stage_run_unit_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count rolled-back cross-org manifest"),
        0,
    );
}

#[tokio::test]
#[serial]
async fn application_model_persistence_rejects_replay_drift_without_mutating_frozen_rows() {
    let fixture = ApplicationModelAuthorityFixture::start("replay_drift").await;
    let seed = fixture.model_seed(&fixture.owner_a);
    let created = application_models::seed_manifest(fixture.db.pool(), &seed)
        .await
        .expect("seed exact manifest before drift");
    let mut drifted = seed;
    drifted.inputs[0].evidence_ids.clear();

    let error = application_models::seed_manifest(fixture.db.pool(), &drifted)
        .await
        .expect_err("same manifest identity with changed material must fail");

    assert_eq!(error.code(), "manifest_replay_drift");
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT manifest_hash FROM application_model_manifests WHERE id=$1",
        )
        .bind(created.manifest.id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("read immutable manifest hash after drift"),
        created.manifest.manifest_hash,
    );
}

#[tokio::test]
#[serial]
async fn application_model_persistence_closes_terminal_no_input_without_fake_model_revision() {
    let fixture = ApplicationModelAuthorityFixture::start("terminal_no_input").await;
    let seed = SeedApplicationModelManifest {
        operation_id: fixture.operation_id,
        scope_snapshot_id: fixture.scope_snapshot_id,
        stage_execution_id: fixture.stage_execution_id,
        stage_run_unit_id: fixture.terminal_stage_run_unit_id,
        organization_id: fixture.terminal_organization_id,
        authority_kind: ApplicationModelAuthorityKindRow::TerminalNoInput,
        inputs: Vec::new(),
    };

    let created = application_models::seed_manifest(fixture.db.pool(), &seed)
        .await
        .expect("seed terminal-no-input manifest");

    assert_eq!(created.manifest.authority_kind, "terminal_no_input");
    assert_eq!(created.manifest.input_count, 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_model_revisions WHERE manifest_id=$1",
        )
        .bind(created.manifest.id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count forbidden terminal-no-input revisions"),
        0,
    );
}

#[tokio::test]
#[serial]
async fn application_model_persistence_rejects_false_terminal_no_input_when_source_exists() {
    let fixture = ApplicationModelAuthorityFixture::start("false_terminal_no_input").await;
    let seed = SeedApplicationModelManifest {
        operation_id: fixture.operation_id,
        scope_snapshot_id: fixture.scope_snapshot_id,
        stage_execution_id: fixture.stage_execution_id,
        stage_run_unit_id: fixture.owner_a.stage_run_unit_id,
        organization_id: fixture.owner_a.organization_id,
        authority_kind: ApplicationModelAuthorityKindRow::TerminalNoInput,
        inputs: Vec::new(),
    };

    let error = application_models::seed_manifest(fixture.db.pool(), &seed)
        .await
        .expect_err("caller cannot hide a final-sealed source behind terminal-no-input");

    assert_eq!(error.code(), "manifest_source_denominator_mismatch");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_model_manifests WHERE stage_run_unit_id=$1",
        )
        .bind(fixture.owner_a.stage_run_unit_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count false terminal manifests"),
        0,
    );
}

#[tokio::test]
#[serial]
async fn application_model_persistence_proposes_evidence_linked_revision_for_gate_readback() {
    let fixture = ApplicationModelAuthorityFixture::start("proposed_revision").await;
    let (manifest_id, proposal) = fixture.valid_proposal(&fixture.owner_a).await;

    let created = application_models::propose_revision(fixture.db.pool(), &proposal)
        .await
        .expect("persist proposed model revision");
    let replayed = application_models::propose_revision(fixture.db.pool(), &proposal)
        .await
        .expect("replay exact proposed model revision");
    let gate_material = application_models::load_gate_material(
        fixture.db.pool(),
        &LoadApplicationModelGateMaterial {
            manifest_id,
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            stage_execution_id: fixture.stage_execution_id,
            stage_run_unit_id: fixture.owner_a.stage_run_unit_id,
            organization_id: fixture.owner_a.organization_id,
        },
    )
    .await
    .expect("load exact Gate material");

    assert!(!created.replayed);
    assert!(replayed.replayed);
    assert_eq!(created.revision, replayed.revision);
    assert_eq!(created.revision.status, "proposed");
    assert_eq!(gate_material.revision, Some(created.revision));
    assert_eq!(gate_material.decisions.len(), 1);
    assert_eq!(gate_material.items.len(), 1);
    assert_eq!(gate_material.item_evidence.len(), 1);
    assert_eq!(gate_material.item_evidence[0].role, "observation");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_model_current_revisions WHERE manifest_id=$1",
        )
        .bind(manifest_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count unpublished proposal pointers"),
        0,
    );
}

#[tokio::test]
#[serial]
async fn application_model_persistence_rejects_foreign_evidence_and_rolls_back_proposal_batch() {
    let fixture = ApplicationModelAuthorityFixture::start("proposal_rollback").await;
    let (manifest_id, mut proposal) = fixture.valid_proposal(&fixture.owner_a).await;
    proposal.items[0].evidence[0].evidence_id = fixture.owner_b.evidence_id;

    let error = application_models::propose_revision(fixture.db.pool(), &proposal)
        .await
        .expect_err("foreign evidence must reject the complete proposal batch");

    assert_eq!(error.code(), "proposal_source_payload_mismatch");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_model_revisions WHERE manifest_id=$1",
        )
        .bind(manifest_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count rolled-back revisions"),
        0,
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_model_items WHERE manifest_id=$1",
        )
        .bind(manifest_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count rolled-back model items"),
        0,
    );
}

#[tokio::test]
#[serial]
async fn application_model_persistence_freezes_manifest_and_proposed_model_rows() {
    let fixture = ApplicationModelAuthorityFixture::start("immutable_rows").await;
    let (manifest_id, proposal) = fixture.valid_proposal(&fixture.owner_a).await;
    let created = application_models::propose_revision(fixture.db.pool(), &proposal)
        .await
        .expect("persist proposal before immutability probes");

    let manifest_error =
        sqlx::query("UPDATE application_model_manifests SET manifest_hash=$1 WHERE id=$2")
            .bind(format!("sha256:{}", "a".repeat(64)))
            .bind(manifest_id)
            .execute(fixture.db.pool())
            .await
            .expect_err("frozen manifest must reject update");
    let item_error =
        sqlx::query("UPDATE application_model_items SET item_kind='changed' WHERE revision_id=$1")
            .bind(created.revision.id)
            .execute(fixture.db.pool())
            .await
            .expect_err("frozen model item must reject update");
    let revision_error = sqlx::query(
        "UPDATE application_model_revisions SET structured_model='{}'::jsonb WHERE id=$1",
    )
    .bind(created.revision.id)
    .execute(fixture.db.pool())
    .await
    .expect_err("proposed revision content must reject update");

    for error in [manifest_error, item_error, revision_error] {
        assert_eq!(
            error
                .as_database_error()
                .and_then(|database| database.code())
                .as_deref(),
            Some("P0001"),
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT status FROM application_model_revisions WHERE id=$1",
        )
        .bind(created.revision.id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("read proposal status after rejected updates"),
        "proposed",
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_model_current_revisions WHERE manifest_id=$1",
        )
        .bind(manifest_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count publication pointers after immutability probes"),
        0,
    );

    let append_error = sqlx::query(
        r#"INSERT INTO application_model_item_evidence(
               revision_id,manifest_id,item_key,evidence_id,role
           ) VALUES($1,$2,'route:api-a',$3,'support')"#,
    )
    .bind(created.revision.id)
    .bind(manifest_id)
    .bind(fixture.owner_b.evidence_id)
    .execute(fixture.db.pool())
    .await
    .expect_err("proposed revision must reject appended child evidence");
    assert!(append_error
        .as_database_error()
        .is_some_and(|database| database.message().contains("REVISION_CHILDREN_FROZEN")),);

    let publication_error = sqlx::query(
        "UPDATE application_model_revisions \
         SET status='final',row_version=1,finalized_at=NOW() WHERE id=$1",
    )
    .bind(created.revision.id)
    .execute(fixture.db.pool())
    .await
    .expect_err("raw SQL cannot commit a final revision without the publication bundle");
    assert!(publication_error
        .as_database_error()
        .is_some_and(|database| {
            database
                .message()
                .contains("FINAL_REVISION_REQUIRES_CURRENT_POINTER")
        }));

    let current_error = sqlx::query(
        r#"INSERT INTO application_model_current_revisions(
               manifest_id,revision_id,authority_kind,stage_handoff_id,
               deliverable_submission_id,manifest_hash,model_hash,
               replay_material_hash,gate_decision_hash
           ) VALUES($1,$2,'model',$3,$4,$5,$6,$7,$8)"#,
    )
    .bind(manifest_id)
    .bind(created.revision.id)
    .bind(Uuid::new_v4())
    .bind(proposal.source_submission_id)
    .bind(
        sqlx::query_scalar::<_, String>(
            "SELECT manifest_hash FROM application_model_manifests WHERE id=$1",
        )
        .bind(manifest_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("read manifest hash for dormant current probe"),
    )
    .bind(&created.revision.model_hash)
    .bind(&created.revision.replay_material_hash)
    .bind(format!("sha256:{}", "b".repeat(64)))
    .execute(fixture.db.pool())
    .await
    .expect_err("raw SQL cannot insert a current pointer without its exact Handoff");
    assert!(current_error.as_database_error().is_some());

    let handoff_payload = serde_json::json!({
        "schema_version": 1,
        "manifest_id": manifest_id,
    });
    let handoff_error = sqlx::query(
        r#"INSERT INTO stage_handoffs(
               id,operation_id,organization_id,scope_snapshot_id,from_stage_kind,
               stage_execution_id,source_stage_run_unit_id,deliverable_submission_id,
               scope_hash,payload,payload_sha256,evidence_ids,coverage_watermark,
               unit_gate_decision_hash,gate_passed_at
           ) VALUES($1,$2,$3,$4,'application_understanding',$5,$6,$7,$8,$9,$10,$11,$12,$13,NOW())"#,
    )
    .bind(Uuid::new_v4())
    .bind(fixture.operation_id)
    .bind(fixture.owner_a.organization_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.stage_execution_id)
    .bind(fixture.owner_a.stage_run_unit_id)
    .bind(proposal.source_submission_id)
    .bind("c".repeat(64))
    .bind(&handoff_payload)
    .bind(sha256_json(&handoff_payload))
    .bind(vec![fixture.owner_a.evidence_id])
    .bind(serde_json::json!({"complete": true}))
    .bind("d".repeat(64))
    .execute(fixture.db.pool())
    .await
    .expect_err("raw SQL cannot commit an Application Understanding Handoff alone");
    assert!(handoff_error.as_database_error().is_some_and(|database| {
        database
            .message()
            .contains("HANDOFF_REQUIRES_CURRENT_POINTER")
    }));
}

#[tokio::test]
#[serial]
async fn application_model_persistence_accepts_independent_unit_and_worker_generations() {
    let fixture = ApplicationModelAuthorityFixture::start("independent_generations").await;
    let (manifest_id, proposal) = fixture.valid_proposal(&fixture.owner_a).await;
    sqlx::query("UPDATE stage_run_units SET generation=1 WHERE id=$1")
        .bind(fixture.owner_a.stage_run_unit_id)
        .execute(fixture.db.pool())
        .await
        .expect("model a no-purge replacement Unit generation");

    let proposed = application_models::propose_revision(fixture.db.pool(), &proposal)
        .await
        .expect("the exact live Worker lease must authorize its proposal");

    assert_eq!(proposed.revision.status, "proposed");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_model_revisions WHERE manifest_id=$1",
        )
        .bind(manifest_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count exact proposal revision"),
        1,
    );
}

#[tokio::test]
#[serial]
async fn application_model_persistence_gate_readback_rejects_sibling_org_identity() {
    let fixture = ApplicationModelAuthorityFixture::start("gate_cross_org").await;
    let (manifest_id, proposal) = fixture.valid_proposal(&fixture.owner_a).await;
    application_models::propose_revision(fixture.db.pool(), &proposal)
        .await
        .expect("persist proposal before cross-org readback");

    let error = application_models::load_gate_material(
        fixture.db.pool(),
        &LoadApplicationModelGateMaterial {
            manifest_id,
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            stage_execution_id: fixture.stage_execution_id,
            stage_run_unit_id: fixture.owner_b.stage_run_unit_id,
            organization_id: fixture.owner_b.organization_id,
        },
    )
    .await
    .expect_err("sibling organization must not read another model snapshot");

    assert_eq!(error.code(), "gate_manifest_owner_mismatch");
}

#[tokio::test]
#[serial]
async fn application_model_persistence_s1_never_publishes_or_runs_attack_tools() {
    let fixture = ApplicationModelAuthorityFixture::start("dormant_boundary").await;
    let (manifest_id, proposal) = fixture.valid_proposal(&fixture.owner_a).await;
    application_models::propose_revision(fixture.db.pool(), &proposal)
        .await
        .expect("persist dormant proposal before boundary audit");

    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_model_current_revisions WHERE manifest_id=$1",
        )
        .bind(manifest_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count current revision publications"),
        0,
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM stage_handoffs WHERE operation_id=$1 \
             AND from_stage_kind='application_understanding'",
        )
        .bind(fixture.operation_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count Application Understanding handoffs"),
        0,
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM org_stage_completions WHERE organization_id=$1 \
             AND stage_kind='application_understanding'",
        )
        .bind(fixture.owner_a.organization_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count Application Understanding completion rows"),
        0,
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM stage_run_units WHERE id=$1")
            .bind(fixture.owner_a.stage_run_unit_id)
            .fetch_one(fixture.db.pool())
            .await
            .expect("read dormant unit status"),
        "queued",
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            r#"SELECT count(*) FROM tool_calls
                WHERE operation_id=$1
                  AND name NOT IN ('submit_stage_deliverable')"#,
        )
        .bind(fixture.operation_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count forbidden active tool calls"),
        0,
        "persistence must not invoke browser, network, shell, or pentest tools",
    );
}

#[tokio::test]
#[serial]
async fn application_model_persistence_gate_readback_fails_after_source_handoff_invalidation() {
    let fixture = ApplicationModelAuthorityFixture::start("source_invalidation").await;
    let (manifest_id, proposal) = fixture.valid_proposal(&fixture.owner_a).await;
    application_models::propose_revision(fixture.db.pool(), &proposal)
        .await
        .expect("persist proposal before source invalidation");
    sqlx::query("UPDATE stage_handoffs SET invalidated_at=NOW() WHERE id=$1")
        .bind(fixture.owner_a.source_handoff_id)
        .execute(fixture.db.pool())
        .await
        .expect("invalidate upstream handoff in isolated fixture");

    let error = application_models::load_gate_material(
        fixture.db.pool(),
        &LoadApplicationModelGateMaterial {
            manifest_id,
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            stage_execution_id: fixture.stage_execution_id,
            stage_run_unit_id: fixture.owner_a.stage_run_unit_id,
            organization_id: fixture.owner_a.organization_id,
        },
    )
    .await
    .expect_err("Gate readback must fail closed after source invalidation");

    assert_eq!(error.code(), "gate_source_handoff_authority_mismatch");
}
