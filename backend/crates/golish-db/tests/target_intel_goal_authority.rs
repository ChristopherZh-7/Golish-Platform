use chrono::{Duration, Utc};
use golish_db::models::{AgentType, NewSession};
use golish_db::repo::{
    message_chains, project_scopes, runtime_memory_tx, scoping_company_identities, sessions,
    stage_deliverable_submissions, stage_run_units, target_intel_asset_observations,
    target_intel_goal_contracts, target_intel_goal_frontier, target_intel_goal_reviews,
    target_intel_goal_work_journal, tool_calls,
};
use golish_db::{DbConfig, GolishDb};
use serde_json::{json, Value};
use serial_test::serial;
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read local postgres port")
        .port()
}

async fn migrated_db(label: &str) -> (GolishDb, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("target_intel_goal_{label}_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start isolated migrated postgres");
    (db, data_dir)
}

fn canonical_sha256(value: &Value) -> String {
    fn write(value: &Value, output: &mut Vec<u8>) {
        match value {
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
                output.extend(serde_json::to_vec(value).expect("serialize scalar"));
            }
            Value::Array(values) => {
                output.push(b'[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        output.push(b',');
                    }
                    write(value, output);
                }
                output.push(b']');
            }
            Value::Object(map) => {
                output.push(b'{');
                let mut keys = map.keys().collect::<Vec<_>>();
                keys.sort();
                for (index, key) in keys.into_iter().enumerate() {
                    if index > 0 {
                        output.push(b',');
                    }
                    output.extend(serde_json::to_vec(key).expect("serialize object key"));
                    output.push(b':');
                    write(&map[key], output);
                }
                output.push(b'}');
            }
        }
    }

    let mut bytes = Vec::new();
    write(value, &mut bytes);
    format!(
        "sha256:{}",
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

#[derive(Debug)]
struct GoalFixture {
    session_id: Uuid,
    operation_id: Uuid,
    organization_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    scope_snapshot_id: Uuid,
    team_plan_id: Uuid,
    controller_work_item_id: Uuid,
    controller_worker_run_id: Uuid,
    controller_message_chain_id: Uuid,
    goal_epoch_id: Uuid,
}

async fn goal_fixture_with_identity(
    db: &GolishDb,
    max_review_rounds: i32,
    identity_confirmed: bool,
) -> anyhow::Result<GoalFixture> {
    let path = format!("/tmp/target-intel-goal-{}", Uuid::new_v4().simple());
    let session_id = sessions::create(
        db.pool(),
        NewSession {
            title: Some("target intel goal authority".to_string()),
            workspace_path: Some(path.clone()),
            workspace_label: None,
            model: None,
            provider: None,
            project_path: Some(path.clone()),
        },
    )
    .await
    .expect("create session")
    .id;
    let project = project_scopes::register_first_open(db.pool(), &path, "target-intel-path-sha")
        .await
        .expect("register project scope");
    let operation_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    runtime_memory_tx::create_runtime_operation(
        db.pool(),
        &runtime_memory_tx::CreateRuntimeOperationRow {
            operation_id,
            initial_stage_execution_id: stage_execution_id,
            session_id,
            title: Some("target intel goal authority".to_string()),
            input: "collect target intelligence".to_string(),
            profile: "red_team".to_string(),
            entry_stage: "target_intel".to_string(),
            project_scope_id: project.project_scope_id,
            application_model_contract: golish_core::ApplicationModelContract::LegacyNoModel,
            cli_scope: None,
        },
    )
    .await
    .expect("create operation");

    let organization_id = Uuid::new_v4();
    let scope_decision_id = Uuid::new_v4();
    let scope_snapshot_id = Uuid::new_v4();
    let mut tx = db.pool().begin().await.expect("begin scope fixture");
    sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$2,'Goal Org')")
        .bind(organization_id)
        .bind(&path)
        .execute(&mut *tx)
        .await
        .expect("insert organization");
    sqlx::query(
        r#"INSERT INTO operation_scope_decisions(
               id,operation_id,project_scope_id,stage_execution_id,
               root_organization_id,mode,decision_rows,decision_hash
           ) VALUES($1,$2,$3,$4,$5,'cli_flags',$6,'target-intel-decision')"#,
    )
    .bind(scope_decision_id)
    .bind(operation_id)
    .bind(project.project_scope_id)
    .bind(stage_execution_id)
    .bind(organization_id)
    .bind(json!([{"organization_id": organization_id}]))
    .execute(&mut *tx)
    .await
    .expect("insert scope decision");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_snapshots(
               id,operation_id,project_scope_id,scope_decision_id,
               project_path_at_freeze,root_organization_id,mode,scope_hash
           ) VALUES($1,$2,$3,$4,$5,$6,'cli_flags','target-intel-scope')"#,
    )
    .bind(scope_snapshot_id)
    .bind(operation_id)
    .bind(project.project_scope_id)
    .bind(scope_decision_id)
    .bind(&path)
    .bind(organization_id)
    .execute(&mut *tx)
    .await
    .expect("insert scope snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units(
               snapshot_id,organization_id,organization_name_at_freeze,
               role,depth,ordinal,decision_row_id,approval_source
           ) VALUES($1,$2,'Goal Org','root',0,0,'root',$3)"#,
    )
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(json!({"source":"cli_flags"}))
    .execute(&mut *tx)
    .await
    .expect("insert scope unit");
    sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
        .bind(scope_snapshot_id)
        .execute(&mut *tx)
        .await
        .expect("seal scope snapshot");
    tx.commit().await.expect("commit scope fixture");
    let identity_payload = if identity_confirmed {
        json!({
            "canonical_legal_name": "Goal Org",
            "registration_identifiers": {"fixture": organization_id},
        })
    } else {
        json!({"subject_hint": "Goal Org", "resolution_status": "unresolved"})
    };
    let scope_policy = json!({"owned_only": true, "reachable_only": true});
    scoping_company_identities::insert_terminal_receipt(
        db.pool(),
        &scoping_company_identities::ScopingCompanyIdentityReceiptRow {
            id: Uuid::new_v4(),
            operation_id,
            stage_execution_id,
            resolution_attempt: 0,
            supersedes_receipt_id: None,
            organization_id: identity_confirmed.then_some(organization_id),
            subject_hint: "Goal Org".to_string(),
            canonical_legal_name: identity_confirmed.then(|| "Goal Org".to_string()),
            aliases: json!([]),
            brands: json!([]),
            registration_identifiers: json!({"fixture": organization_id}),
            disambiguation_fields: json!({}),
            confirmation_method: if identity_confirmed {
                "provider_corroborated"
            } else {
                "none"
            }
            .to_string(),
            resolution_status: if identity_confirmed {
                "confirmed"
            } else {
                "unresolved"
            }
            .to_string(),
            scope_policy: scope_policy.clone(),
            source_receipt_refs: if identity_confirmed {
                json!(["fixture:enterprise-registry"])
            } else {
                json!([])
            },
            artifact_refs: if identity_confirmed {
                json!([format!("fixture:{organization_id}")])
            } else {
                json!([])
            },
            evidence_refs: if identity_confirmed {
                json!(["audit:fixture"])
            } else {
                json!([])
            },
            identity_payload: identity_payload.clone(),
            identity_sha256: canonical_sha256(&identity_payload),
            scope_policy_sha256: canonical_sha256(&scope_policy),
        },
    )
    .await
    .expect("freeze terminal company identity fixture");

    let stage_run_unit_id = Uuid::new_v4();
    stage_run_units::insert_with_executor(
        db.pool(),
        &stage_run_units::NewStageRunUnit {
            id: stage_run_unit_id,
            operation_id,
            stage_execution_id,
            scope_snapshot_id,
            organization_id,
            stage_kind: "target_intel".to_string(),
            generation: 0,
            specialist: Some("company_stage_controller".to_string()),
        },
    )
    .await
    .expect("insert target intel unit");
    let team_plan_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO stage_team_plans(
               id,operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
               organization_id,stage_kind,unit_generation,schema_version,plan_version,
               plan_hash,leader_role,aggregator_kind,aggregator_role,allowed_worker_roles,
               max_workers_total,max_workers_active,dynamic_requests_allowed,
               dynamic_request_policy,dispatch_epoch,final_submitter_kind,
               created_from_stage_spec_hash
           ) VALUES(
               $1,$2,$3,$4,$5,$6,'target_intel',0,1,1,$7,
               'company_stage_controller','worker','company_stage_controller',$8,
               8,3,TRUE,$9,0,'worker',$10
           )"#,
    )
    .bind(team_plan_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(format!("sha256:{}", "a".repeat(64)))
    .bind(json!(["company_stage_controller", "intel_goal_reviewer"]))
    .bind(json!({"max_requests": 8}))
    .bind(format!("sha256:{}", "b".repeat(64)))
    .execute(db.pool())
    .await
    .expect("insert target intel team plan");
    let controller_work_item_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO stage_work_items(
               id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
               input_manifest_hash,input_refs,required_for_barrier,priority,status,
               attempt_policy,budget,output_schema,created_by,started_at
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,0,'company_controller','leader:primary',
               'company_stage_controller',$8,'[]'::jsonb,FALSE,100,'running',
               '{}'::jsonb,'{}'::jsonb,'stage_worker_output.v1','server_seed',NOW()
           )"#,
    )
    .bind(controller_work_item_id)
    .bind(team_plan_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(format!("sha256:{}", "c".repeat(64)))
    .execute(db.pool())
    .await
    .expect("insert controller work item");
    let controller_message_chain_id = Uuid::new_v4();
    message_chains::create_bound_with_executor(
        db.pool(),
        controller_message_chain_id,
        session_id,
        operation_id,
        None,
        AgentType::Primary,
        None,
        None,
        &json!([]),
    )
    .await
    .expect("create controller message chain");
    let controller_worker_run_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,organization_id,
               worker_generation,specialist,work_item_kind,work_item_key,agent_path,
               message_chain_id,status,work_item_id,started_at
           ) VALUES(
               $1,$2,$3,$4,$5,0,'company_stage_controller','company_controller',
               'leader:primary','main>target-intel-controller',$6,'running',$7,NOW()
           )"#,
    )
    .bind(controller_worker_run_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(organization_id)
    .bind(controller_message_chain_id)
    .bind(controller_work_item_id)
    .execute(db.pool())
    .await
    .expect("insert controller worker");

    let contract = target_intel_goal_contracts::TargetIntelGoalOperationContractRow {
        operation_id,
        profile_id: "red_team".to_string(),
        runtime_mode: "advisory_rework".to_string(),
        completion_authority: "legacy_six_axis_v1".to_string(),
        goal_contract_version: "target_intel_goal.v1".to_string(),
        canonical_goal_contract: json!({"goal":"collect bounded target intelligence"}),
        goal_contract_sha256: canonical_sha256(
            &json!({"goal":"collect bounded target intelligence"}),
        ),
        methodology_payload: json!({"version":"fixture.v1"}),
        methodology_sha256: canonical_sha256(&json!({"version":"fixture.v1"})),
        tool_manifest: json!({"semantic_pivot":true}),
        tool_manifest_sha256: canonical_sha256(&json!({"semantic_pivot":true})),
        provider_capability_manifest: json!({"company_name":"supported"}),
        provider_capability_sha256: canonical_sha256(&json!({"company_name":"supported"})),
        browser_policy: json!({"mode":"disabled"}),
        budget_policy: json!({"max_queries":8}),
        max_review_rounds,
        reviewer_retry_fuel: 1,
    };
    let goal_epoch_id = Uuid::new_v4();
    target_intel_goal_contracts::freeze_unit(
        db.pool(),
        &target_intel_goal_contracts::FreezeTargetIntelGoalUnit {
            contract,
            organization_id,
            team_plan_id,
            goal_epoch_id,
            controller_work_item_id,
            controller_worker_run_id,
            controller_message_chain_id,
        },
    )
    .await?;

    Ok(GoalFixture {
        session_id,
        operation_id,
        organization_id,
        stage_execution_id,
        stage_run_unit_id,
        scope_snapshot_id,
        team_plan_id,
        controller_work_item_id,
        controller_worker_run_id,
        controller_message_chain_id,
        goal_epoch_id,
    })
}

async fn goal_fixture(db: &GolishDb, max_review_rounds: i32) -> GoalFixture {
    goal_fixture_with_identity(db, max_review_rounds, true)
        .await
        .expect("freeze operation contract and first goal epoch")
}

async fn freeze_review(
    db: &GolishDb,
    fixture: &GoalFixture,
    expected_epoch: i64,
) -> (Uuid, Uuid, String) {
    let snapshot = target_intel_goal_reviews::load_freeze_snapshot(
        db.pool(),
        fixture.operation_id,
        fixture.organization_id,
        fixture.stage_execution_id,
        fixture.stage_run_unit_id,
        fixture.team_plan_id,
        fixture.controller_work_item_id,
        fixture.controller_worker_run_id,
        expected_epoch,
    )
    .await
    .expect("load review freeze snapshot");
    let frozen_chain_id = snapshot
        .durable_state
        .pointer("/controller_work_memory/message_chain_id")
        .and_then(Value::as_str)
        .expect("review freezes the exact Controller short-term memory chain");
    assert_eq!(
        frozen_chain_id,
        fixture.controller_message_chain_id.to_string()
    );
    assert!(
        snapshot
            .durable_state
            .pointer("/controller_work_memory/messages")
            .is_some_and(Value::is_array),
        "review material includes the auditable Controller work memory"
    );
    let review_id = Uuid::new_v4();
    let completion_claim = json!({"completion_claim":"all material paths exhausted"});
    let material_revision_vector = json!({
        "state_revision": snapshot.state_revision,
        "action_revision": snapshot.action_revision,
        "evidence_high_water": snapshot.evidence_high_water,
        "tool_high_water": snapshot.tool_high_water,
    });
    let durable_state_sha256 = canonical_sha256(&snapshot.durable_state);
    let observable_actions_sha256 = canonical_sha256(&snapshot.observable_actions);
    let frozen_contract_sha256 = canonical_sha256(&snapshot.frozen_contract);
    let completion_claim_sha256 = canonical_sha256(&completion_claim);
    let bundle_sha256 = canonical_sha256(&json!([
        {
            "review_id": review_id,
            "operation_id": snapshot.operation_id,
            "stage_execution_id": snapshot.stage_execution_id,
            "stage_run_unit_id": snapshot.stage_run_unit_id,
            "organization_id": snapshot.organization_id,
            "team_plan_id": snapshot.team_plan_id,
            "controller_work_item_id": snapshot.controller_work_item_id,
            "controller_worker_run_id": snapshot.controller_worker_run_id,
            "controller_message_chain_id": snapshot.controller_message_chain_id,
            "goal_epoch": snapshot.goal_epoch,
            "review_generation": snapshot.review_generation,
            "round": snapshot.round,
            "state_revision": snapshot.state_revision,
        },
        [
            {"kind":"durable_state","payload":snapshot.durable_state,"sha256":durable_state_sha256},
            {"kind":"observable_actions","payload":snapshot.observable_actions,"sha256":observable_actions_sha256},
            {"kind":"frozen_contract","payload":snapshot.frozen_contract,"sha256":frozen_contract_sha256},
            {"kind":"completion_claim","payload":completion_claim,"sha256":completion_claim_sha256}
        ]
    ]));
    let inserted = target_intel_goal_reviews::insert_frozen_review(
        db.pool(),
        &target_intel_goal_reviews::InsertFrozenTargetIntelReview {
            review_id,
            expected_plan_row_version: snapshot.plan_row_version,
            material_revision_vector,
            material_state_sha256: durable_state_sha256.clone(),
            material_actions_sha256: observable_actions_sha256.clone(),
            durable_state_sha256,
            observable_actions_sha256,
            frozen_contract_sha256,
            completion_claim_sha256,
            completion_claim,
            bundle_sha256: bundle_sha256.clone(),
            snapshot,
        },
    )
    .await
    .expect("freeze review before final submit closure");
    (
        review_id,
        inserted
            .reviewer_work_item_id
            .expect("advisory review creates a reviewer work item"),
        bundle_sha256,
    )
}

async fn bind_reviewer(
    db: &GolishDb,
    fixture: &GoalFixture,
    review_id: Uuid,
    reviewer_work_item_id: Uuid,
) -> Uuid {
    let reviewer_message_chain_id = Uuid::new_v4();
    message_chains::create_bound_with_executor(
        db.pool(),
        reviewer_message_chain_id,
        fixture.session_id,
        fixture.operation_id,
        None,
        AgentType::Pentester,
        None,
        None,
        &json!([]),
    )
    .await
    .expect("create reviewer message chain");
    let reviewer_worker_run_id = Uuid::new_v4();
    let (kind, stable_key): (String, String) =
        sqlx::query_as("SELECT kind,stable_key FROM stage_work_items WHERE id=$1")
            .bind(reviewer_work_item_id)
            .fetch_one(db.pool())
            .await
            .expect("read reviewer work item identity");
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,organization_id,
               worker_generation,specialist,work_item_kind,work_item_key,agent_path,
               message_chain_id,status,work_item_id,started_at
           ) VALUES($1,$2,$3,$4,$5,0,'intel_goal_reviewer',$6,$7,$8,$9,'running',$10,NOW())"#,
    )
    .bind(reviewer_worker_run_id)
    .bind(fixture.operation_id)
    .bind(fixture.stage_execution_id)
    .bind(fixture.stage_run_unit_id)
    .bind(fixture.organization_id)
    .bind(kind)
    .bind(stable_key)
    .bind(format!("main>target-intel-review:{review_id}"))
    .bind(reviewer_message_chain_id)
    .bind(reviewer_work_item_id)
    .execute(db.pool())
    .await
    .expect("insert read-only reviewer worker");
    sqlx::query(
        r#"UPDATE stage_work_items
              SET status='running',started_at=NOW(),row_version=row_version+1,updated_at=NOW()
            WHERE id=$1 AND status='queued'"#,
    )
    .bind(reviewer_work_item_id)
    .execute(db.pool())
    .await
    .expect("start reviewer work item");
    sqlx::query(
        r#"UPDATE target_intel_goal_reviews
              SET reviewer_worker_run_id=$2,row_version=row_version+1
            WHERE id=$1 AND reviewer_work_item_id=$3 AND status='frozen'"#,
    )
    .bind(review_id)
    .bind(reviewer_worker_run_id)
    .bind(reviewer_work_item_id)
    .execute(db.pool())
    .await
    .expect("bind exact reviewer worker");
    reviewer_worker_run_id
}

async fn read_all_sections(
    db: &GolishDb,
    review_id: Uuid,
    reviewer_worker_run_id: Uuid,
    bundle_sha256: &str,
) -> i64 {
    let mut version = 0;
    for kind in [
        "durable_state",
        "observable_actions",
        "frozen_contract",
        "completion_claim",
    ] {
        version = target_intel_goal_reviews::read_section(
            db.pool(),
            review_id,
            reviewer_worker_run_id,
            0,
            kind,
            bundle_sha256,
        )
        .await
        .expect("read review section in host order")
        .review_row_version;
    }
    version
}

async fn finish_reviewer(db: &GolishDb, reviewer_work_item_id: Uuid, reviewer_worker_run_id: Uuid) {
    sqlx::query(
        "UPDATE stage_worker_runs SET status='passed',terminal_at=NOW(),updated_at=NOW() WHERE id=$1",
    )
    .bind(reviewer_worker_run_id)
    .execute(db.pool())
    .await
    .expect("finish reviewer worker");
    sqlx::query(
        r#"UPDATE stage_work_items
              SET status='completed',terminal_at=NOW(),row_version=row_version+1,updated_at=NOW()
            WHERE id=$1"#,
    )
    .bind(reviewer_work_item_id)
    .execute(db.pool())
    .await
    .expect("finish reviewer work item");
}

async fn bind_exact_controller_final_submission(db: &GolishDb, fixture: &GoalFixture) {
    let lease_token = Uuid::new_v4();
    let (attempt_epoch, active_lease_token): (i64, Option<Uuid>) = sqlx::query_as(
        r#"UPDATE stage_worker_runs
              SET lease_token=COALESCE(lease_token,$2),
                  lease_owner=COALESCE(lease_owner,'target-intel-final-submit-test'),
                  lease_acquired_at=COALESCE(lease_acquired_at,NOW()),
                  lease_expires_at=GREATEST(COALESCE(lease_expires_at,NOW()),NOW())
                                   + INTERVAL '10 minutes',
                  heartbeat_at=NOW(),updated_at=NOW()
            WHERE id=$1
            RETURNING attempt_epoch,lease_token"#,
    )
    .bind(fixture.controller_worker_run_id)
    .bind(lease_token)
    .fetch_one(db.pool())
    .await
    .expect("lease exact Controller for final submission");
    let active_lease_token = active_lease_token.expect("Controller lease token is present");
    let tool_request_id = format!("target-intel-submit-{}", Uuid::new_v4());
    let tool_call_id = tool_calls::record_tracked_start(
        db.pool(),
        &tool_request_id,
        fixture.session_id,
        Some(fixture.operation_id),
        None,
        "submit_stage_deliverable",
        &json!({"stage_id": "target_intel"}),
        Some(&tool_calls::RuntimeToolIdentity {
            operation_id: fixture.operation_id,
            stage_execution_id: fixture.stage_execution_id,
            stage_run_unit_id: Some(fixture.stage_run_unit_id),
            worker_run_id: Some(fixture.controller_worker_run_id),
            organization_id: Some(fixture.organization_id),
            attempt_epoch: Some(attempt_epoch),
            lease_token: Some(active_lease_token),
        }),
    )
    .await
    .expect("record active Controller final-submission tool");
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET active_tool_call_id=$2,active_tool_started_at=NOW(),
                  heartbeat_at=NOW(),updated_at=NOW()
            WHERE id=$1 AND status='running'"#,
    )
    .bind(fixture.controller_worker_run_id)
    .bind(tool_call_id)
    .execute(db.pool())
    .await
    .expect("bind active final-submission tool to exact Controller");
    sqlx::query(
        r#"UPDATE stage_team_plans
              SET requests_closed_at=NOW(),final_submitter_worker_run_id=$2,
                  row_version=row_version+1,updated_at=NOW()
            WHERE id=$1 AND requests_closed_at IS NULL
              AND final_submitter_worker_run_id IS NULL"#,
    )
    .bind(fixture.team_plan_id)
    .bind(fixture.controller_worker_run_id)
    .execute(db.pool())
    .await
    .expect("bind exact Controller as final submitter");
    let payload = json!({
        "stage_id": "target_intel",
        "stage_run_id": fixture.stage_execution_id,
    });
    let canonical_payload_json = serde_json::to_string(&payload).expect("serialize deliverable");
    let payload_sha256 = Sha256::digest(canonical_payload_json.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    stage_deliverable_submissions::insert(
        db.pool(),
        &stage_deliverable_submissions::NewStageDeliverableSubmission {
            operation_id: fixture.operation_id,
            stage_execution_id: fixture.stage_execution_id,
            stage_run_unit_id: Some(fixture.stage_run_unit_id),
            worker_run_id: Some(fixture.controller_worker_run_id),
            organization_id: Some(fixture.organization_id),
            tool_call_record_id: tool_call_id,
            tool_request_id,
            stage_kind: "target_intel".to_string(),
            attempt_epoch: Some(attempt_epoch),
            lease_token: Some(active_lease_token),
            canonical_payload_json,
            payload_sha256,
        },
    )
    .await
    .expect("persist exact Controller Target Intel submission");
    tool_calls::record_tracked_finish(
        db.pool(),
        tool_call_id,
        fixture.session_id,
        "finished",
        r#"{"submitted":true}"#,
        1,
    )
    .await
    .expect("finish Controller final-submission tool");
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET active_tool_call_id=NULL,active_tool_started_at=NULL,
                  heartbeat_at=NOW(),updated_at=NOW()
            WHERE id=$1 AND active_tool_call_id=$2"#,
    )
    .bind(fixture.controller_worker_run_id)
    .bind(tool_call_id)
    .execute(db.pool())
    .await
    .expect("clear exact Controller final-submission tool fence");
}

async fn record_finished_reviewer_submit_result(
    db: &GolishDb,
    fixture: &GoalFixture,
    reviewer_worker_run_id: Uuid,
) {
    let lease_token = Uuid::new_v4();
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET lease_token=$2,lease_owner='target-intel-reviewer-submit-result-test',
                  lease_acquired_at=NOW(),lease_expires_at=NOW()+INTERVAL '10 minutes',
                  heartbeat_at=NOW(),updated_at=NOW()
            WHERE id=$1 AND status='running'"#,
    )
    .bind(reviewer_worker_run_id)
    .bind(lease_token)
    .execute(db.pool())
    .await
    .expect("lease the exact reviewer for its generic terminal protocol call");
    let tool_call_id = tool_calls::record_tracked_start(
        db.pool(),
        &format!("reviewer-submit-result-{}", Uuid::new_v4()),
        fixture.session_id,
        Some(fixture.operation_id),
        None,
        "submit_result",
        &json!({"success": true}),
        Some(&tool_calls::RuntimeToolIdentity {
            operation_id: fixture.operation_id,
            stage_execution_id: fixture.stage_execution_id,
            stage_run_unit_id: Some(fixture.stage_run_unit_id),
            worker_run_id: Some(reviewer_worker_run_id),
            organization_id: Some(fixture.organization_id),
            attempt_epoch: Some(0),
            lease_token: Some(lease_token),
        }),
    )
    .await
    .expect("record the reviewer's generic submit_result call");
    tool_calls::record_tracked_finish(
        db.pool(),
        tool_call_id,
        fixture.session_id,
        "finished",
        r#"{"success":true}"#,
        1,
    )
    .await
    .expect("finish the reviewer's generic submit_result call");
}

async fn insert_intel_evidence(db: &GolishDb, fixture: &GoalFixture, label: &str) -> i64 {
    sqlx::query_scalar(
        r#"INSERT INTO audit_log(
               session_id,action,category,details,project_path,source,audit_role,detail,run_id
           ) VALUES($1,$2,'target_intel','provider fact','/tmp','target_intel_goal',
                    'evidence',$3,$4) RETURNING id"#,
    )
    .bind(fixture.session_id)
    .bind(label)
    .bind(json!({"organization_id": fixture.organization_id, "label": label}))
    .bind(fixture.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("insert target intel evidence")
}

async fn insert_reachability_tool_call(db: &GolishDb, fixture: &GoalFixture, label: &str) -> Uuid {
    insert_finished_controller_tool_call(db, fixture, label, "http_probe").await
}

async fn insert_finished_controller_tool_call(
    db: &GolishDb,
    fixture: &GoalFixture,
    label: &str,
    tool_name: &str,
) -> Uuid {
    let tool_call_id = insert_active_controller_tool_call(db, fixture, label, tool_name).await;
    tool_calls::record_tracked_finish(
        db.pool(),
        tool_call_id,
        fixture.session_id,
        "finished",
        r#"{"reachable":true}"#,
        1,
    )
    .await
    .expect("record exact reachability tool finish");
    tool_call_id
}

async fn insert_active_controller_tool_call(
    db: &GolishDb,
    fixture: &GoalFixture,
    label: &str,
    tool_name: &str,
) -> Uuid {
    let lease_token = Uuid::new_v4();
    let (attempt_epoch, active_lease_token): (i64, Option<Uuid>) = sqlx::query_as(
        r#"UPDATE stage_worker_runs
              SET lease_token=COALESCE(lease_token,$2),
                  lease_owner=COALESCE(lease_owner,'target-intel-db-contract-test'),
                  lease_acquired_at=COALESCE(lease_acquired_at,NOW()),
                  lease_expires_at=GREATEST(COALESCE(lease_expires_at,NOW()),NOW())
                                   + INTERVAL '10 minutes',
                  heartbeat_at=NOW(),updated_at=NOW()
            WHERE id=$1
            RETURNING attempt_epoch,lease_token"#,
    )
    .bind(fixture.controller_worker_run_id)
    .bind(lease_token)
    .fetch_one(db.pool())
    .await
    .expect("lease the exact Controller worker for a runtime-fenced tool call");
    let active_lease_token = active_lease_token.expect("Controller lease token is present");
    let tool_call_id = tool_calls::record_tracked_start(
        db.pool(),
        &format!("reachability-{label}-{}", Uuid::new_v4()),
        fixture.session_id,
        Some(fixture.operation_id),
        None,
        tool_name,
        &json!({"target": label}),
        Some(&tool_calls::RuntimeToolIdentity {
            operation_id: fixture.operation_id,
            stage_execution_id: fixture.stage_execution_id,
            stage_run_unit_id: Some(fixture.stage_run_unit_id),
            worker_run_id: Some(fixture.controller_worker_run_id),
            organization_id: Some(fixture.organization_id),
            attempt_epoch: Some(attempt_epoch),
            lease_token: Some(active_lease_token),
        }),
    )
    .await
    .expect("record exact reachability tool start");
    tool_call_id
}

async fn insert_observation(
    db: &GolishDb,
    fixture: &GoalFixture,
    label: &str,
) -> target_intel_asset_observations::TargetIntelAssetObservationRow {
    let evidence_id = insert_intel_evidence(db, fixture, label).await;
    let artifact_sha256 = Sha256::digest(format!("artifact:{label}").as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let canonical_identity = json!({"kind": "hostname", "value": format!("{label}.example.test")});
    let observed_at = Utc::now();
    let row = target_intel_asset_observations::TargetIntelAssetObservationRow {
        id: Uuid::new_v4(),
        stable_observation_key: format!("fixture:{label}"),
        operation_id: fixture.operation_id,
        organization_id: fixture.organization_id,
        team_plan_id: fixture.team_plan_id,
        goal_epoch_id: fixture.goal_epoch_id,
        goal_epoch: 0,
        producer_worker_run_id: fixture.controller_worker_run_id,
        producer_tool_call_id: None,
        semantic_receipt_audit_id: None,
        evidence_id,
        artifact_ref: format!("intel-artifact:sha256:{artifact_sha256}"),
        artifact_sha256,
        provider_id: "fixture-provider".to_string(),
        provider_query_type: "domain_search".to_string(),
        adapter_version: "fixture.v1".to_string(),
        stable_query_key: format!("query:{label}"),
        provider_record_ordinal: 0,
        provider_fetched_at: observed_at,
        asset_kind: "hostname".to_string(),
        canonical_value: format!("{label}.example.test"),
        canonical_identity_sha256: canonical_sha256(&canonical_identity),
        canonical_identity,
        typed_core: json!({"hostname": format!("{label}.example.test")}),
        provider_fields: json!({"provider_rank": 1, "label": label}),
        provider_metadata: json!({"query": label}),
        observation_sha256: canonical_sha256(&json!({"label": label})),
        attribution_disposition: "unassessed".to_string(),
        attribution_method: None,
        attribution_basis: None,
        attribution_decided_at: None,
        reachability_state: "unverified".to_string(),
        reachability_method: None,
        reachability_tool_call_id: None,
        reachability_evidence_id: None,
        reachability_checked_at: None,
        reachability_valid_until: None,
        promotion_target_id: None,
        promoted_at: None,
        row_version: 0,
        observed_at,
    };
    assert!(
        target_intel_asset_observations::insert(db.pool(), &row)
            .await
            .expect("insert provider-fact observation"),
        "a novel provider record lands as an Observation"
    );
    row
}

async fn record_attribution(
    db: &GolishDb,
    observation_id: Uuid,
    expected_row_version: i64,
    disposition: &str,
) -> i64 {
    target_intel_asset_observations::record_attribution(
        db.pool(),
        &target_intel_asset_observations::RecordAttribution {
            observation_id,
            expected_row_version,
            disposition: disposition.to_string(),
            method: "company_identity_corroboration".to_string(),
            basis: json!({"company_identity_receipt": "fixture", "disposition": disposition}),
            evidence_refs: json!(["audit:fixture-attribution"]),
        },
    )
    .await
    .expect("record typed attribution")
}

async fn record_reachability(
    db: &GolishDb,
    fixture: &GoalFixture,
    observation_id: Uuid,
    expected_row_version: i64,
    state: &str,
    method: &str,
    tool_call_id: Option<Uuid>,
) -> anyhow::Result<i64> {
    let evidence_id = insert_intel_evidence(db, fixture, &format!("reach-{observation_id}")).await;
    let checked_at = Utc::now();
    target_intel_asset_observations::record_reachability(
        db.pool(),
        &target_intel_asset_observations::RecordReachability {
            observation_id,
            expected_row_version,
            state: state.to_string(),
            method: method.to_string(),
            tool_call_id,
            evidence_id,
            checked_at,
            valid_until: (state == "reachable").then_some(checked_at + Duration::minutes(10)),
        },
    )
    .await
}

async fn append_journal(db: &GolishDb, fixture: &GoalFixture, ordinal: i64, entry_kind: &str) {
    let payload = json!({"ordinal": ordinal, "kind": entry_kind});
    target_intel_goal_work_journal::append(
        db.pool(),
        &target_intel_goal_work_journal::TargetIntelGoalWorkJournalEntryRow {
            id: Uuid::new_v4(),
            stable_request_id: Uuid::new_v4(),
            operation_id: fixture.operation_id,
            organization_id: fixture.organization_id,
            team_plan_id: fixture.team_plan_id,
            goal_epoch_id: fixture.goal_epoch_id,
            goal_epoch: 0,
            controller_worker_run_id: fixture.controller_worker_run_id,
            controller_message_chain_id: fixture.controller_message_chain_id,
            ordinal,
            entry_kind: entry_kind.to_string(),
            payload: payload.clone(),
            related_frontier_refs: json!([]),
            evidence_refs: json!([]),
            tool_call_refs: json!([]),
            observation_refs: json!([]),
            entry_sha256: canonical_sha256(&payload),
        },
    )
    .await
    .expect("append structured Controller work memory");
}

fn make_finalizer_preconditions_ready(
    snapshot: &mut target_intel_goal_reviews::TargetIntelGoalFinalizerSnapshot,
) {
    snapshot.operation_contract_valid = true;
    snapshot.review_is_fresh_pass = true;
    snapshot.all_four_sections_read = true;
    snapshot.verdict_sha256 = format!("sha256:{}", "d".repeat(64));
    snapshot.active_authoritative_workers = 0;
    snapshot.active_authoritative_tools = 0;
    snapshot.current_run_terminal_receipt_count = 1;
    snapshot.valid_evidence_artifact_closure_count = 1;
    snapshot.pending_or_retryable_frontier_count = 0;
    snapshot.unwaived_blocked_or_unsupported_count = 0;
    snapshot.unresolved_material_contradiction_count = 0;
    snapshot.open_material_finding_count = 0;
    snapshot.unauthorized_scope_promotion_count = 0;
    snapshot.confirmed_company_identity_count = 1;
    snapshot.structured_journal_entry_count = snapshot.structured_journal_entry_count.max(1);
    snapshot.completion_checkpoint_count = 1;
    snapshot.unassessed_observation_count = 0;
    snapshot.invalid_promotion_count = 0;
    snapshot.needs_human_count = 0;
}

fn clone_observation_for_label(
    source: &target_intel_asset_observations::TargetIntelAssetObservationRow,
    label: &str,
) -> target_intel_asset_observations::TargetIntelAssetObservationRow {
    let mut row = source.clone();
    let artifact_sha256 = Sha256::digest(format!("artifact:{label}").as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let canonical_identity = json!({"kind": "hostname", "value": format!("{label}.example.test")});
    row.id = Uuid::new_v4();
    row.stable_observation_key = format!("fixture:{label}");
    row.artifact_ref = format!("intel-artifact:sha256:{artifact_sha256}");
    row.artifact_sha256 = artifact_sha256;
    row.stable_query_key = format!("query:{label}");
    row.canonical_value = format!("{label}.example.test");
    row.canonical_identity_sha256 = canonical_sha256(&canonical_identity);
    row.canonical_identity = canonical_identity;
    row.typed_core = json!({"hostname": format!("{label}.example.test")});
    row.provider_fields = json!({"provider_rank": 1, "label": label});
    row.provider_metadata = json!({"query": label});
    row.observation_sha256 = canonical_sha256(&json!({"label": label}));
    row
}

#[tokio::test]
#[serial]
async fn unresolved_scoping_identity_cannot_freeze_target_intel_goal() {
    let (mut db, _data_dir) = migrated_db("unresolved-identity").await;
    let error = goal_fixture_with_identity(&db, 3, false)
        .await
        .expect_err("an unresolved company-name receipt cannot open Intel");
    assert!(
        format!("{error:#}").contains("TARGET_INTEL_CONFIRMED_COMPANY_IDENTITY_MISSING"),
        "freeze fails at the confirmed Company Identity boundary: {error:#}"
    );
    let frozen_contracts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM target_intel_goal_operation_contracts")
            .fetch_one(db.pool())
            .await
            .expect("count rolled-back goal contracts");
    let epochs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM target_intel_goal_epochs")
        .fetch_one(db.pool())
        .await
        .expect("count rolled-back goal epochs");
    assert_eq!((frozen_contracts, epochs), (0, 0));

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn observations_precede_targets_and_only_owned_fresh_reachable_promotes_once() {
    let (mut db, _data_dir) = migrated_db("observation-promotion").await;
    let fixture = goal_fixture(&db, 3).await;
    let shared = insert_observation(&db, &fixture, "shared").await;
    let ambiguous = insert_observation(&db, &fixture, "ambiguous").await;
    let unreachable = insert_observation(&db, &fixture, "unreachable").await;
    let owned = insert_observation(&db, &fixture, "owned").await;
    let target_count_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM targets WHERE organization_id=$1 AND source='target_intel_goal'",
    )
    .bind(fixture.organization_id)
    .fetch_one(db.pool())
    .await
    .expect("count pre-promotion Targets");
    assert_eq!(
        target_count_before, 0,
        "provider records land as Observations before any formal Target exists"
    );

    let reachability_tool = insert_reachability_tool_call(&db, &fixture, "http-probe").await;
    for (row, disposition) in [(&shared, "shared"), (&ambiguous, "ambiguous")] {
        let version = record_attribution(&db, row.id, 0, disposition).await;
        let version = record_reachability(
            &db,
            &fixture,
            row.id,
            version,
            "reachable",
            "bounded_http_probe_v1",
            Some(reachability_tool),
        )
        .await
        .expect("record fresh HTTP reachability");
        assert!(
            target_intel_asset_observations::promote_owned_reachable(db.pool(), row.id, version)
                .await
                .is_err(),
            "{disposition} attribution never authorizes formal scope promotion"
        );
    }

    let unreachable_version = record_attribution(&db, unreachable.id, 0, "owned").await;
    let unreachable_version = record_reachability(
        &db,
        &fixture,
        unreachable.id,
        unreachable_version,
        "unreachable",
        "bounded_http_probe_v1",
        Some(reachability_tool),
    )
    .await
    .expect("record terminal unreachable result");
    assert!(
        target_intel_asset_observations::promote_owned_reachable(
            db.pool(),
            unreachable.id,
            unreachable_version,
        )
        .await
        .is_err(),
        "owned but unreachable observations remain outside formal Targets"
    );

    let owned_version = record_attribution(&db, owned.id, 0, "owned").await;
    let owned_version = record_reachability(
        &db,
        &fixture,
        owned.id,
        owned_version,
        "reachable",
        "http_probe",
        Some(reachability_tool),
    )
    .await
    .expect("record owned asset fresh reachability");
    let target_id = target_intel_asset_observations::promote_owned_reachable(
        db.pool(),
        owned.id,
        owned_version,
    )
    .await
    .expect("promote the sole owned and freshly reachable Observation");
    let promoted_row_version: i64 =
        sqlx::query_scalar("SELECT row_version FROM target_intel_asset_observations WHERE id=$1")
            .bind(owned.id)
            .fetch_one(db.pool())
            .await
            .expect("read promoted Observation row version");
    let replay_target_id = target_intel_asset_observations::promote_owned_reachable(
        db.pool(),
        owned.id,
        promoted_row_version,
    )
    .await
    .expect("promotion replay returns the existing Target");
    assert_eq!(replay_target_id, target_id);
    let target_count_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM targets WHERE organization_id=$1 AND source='target_intel_goal'",
    )
    .bind(fixture.organization_id)
    .fetch_one(db.pool())
    .await
    .expect("count post-promotion Targets");
    let promotion_events: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM target_intel_asset_observation_events WHERE observation_id=$1 AND event_kind='promotion'",
    )
    .bind(owned.id)
    .fetch_one(db.pool())
    .await
    .expect("count promotion events");
    assert_eq!((target_count_after, promotion_events), (1, 1));

    let mutate_provider_fact = sqlx::query(
        r#"UPDATE target_intel_asset_observations
              SET provider_fields='{"forged":true}'::jsonb,row_version=row_version+1
            WHERE id=$1"#,
    )
    .bind(owned.id)
    .execute(db.pool())
    .await;
    assert!(
        mutate_provider_fact.is_err(),
        "landed provider facts are immutable even with a syntactically valid CAS increment"
    );
    let mutate_event = sqlx::query(
        r#"UPDATE target_intel_asset_observation_events
              SET after_state='{"forged":true}'::jsonb
            WHERE observation_id=$1 AND event_kind='promotion'"#,
    )
    .bind(owned.id)
    .execute(db.pool())
    .await;
    assert!(
        mutate_event.is_err(),
        "Observation transition events are append-only"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn dns_only_reachability_cannot_promote_an_owned_observation() {
    let (mut db, _data_dir) = migrated_db("dns-only").await;
    let fixture = goal_fixture(&db, 3).await;
    let observation = insert_observation(&db, &fixture, "dns-only").await;
    let owned_version = record_attribution(&db, observation.id, 0, "owned").await;
    let dns_result = record_reachability(
        &db,
        &fixture,
        observation.id,
        owned_version,
        "reachable",
        "dns",
        None,
    )
    .await;
    if let Ok(dns_version) = dns_result {
        assert!(
            target_intel_asset_observations::promote_owned_reachable(
                db.pool(),
                observation.id,
                dns_version,
            )
            .await
            .is_err(),
            "DNS resolution alone is discovery evidence, not verified service reachability"
        );
    }

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn observation_producer_evidence_and_tool_refs_are_closed_to_the_owner_tuple() {
    let (mut db, _data_dir) = migrated_db("observation-owner-closure").await;
    let owner = goal_fixture(&db, 3).await;
    let foreign = goal_fixture(&db, 3).await;
    let owner_observation = insert_observation(&db, &owner, "owner-base").await;
    let foreign_evidence_id = insert_intel_evidence(&db, &foreign, "foreign-evidence").await;
    let foreign_tool_call_id = insert_reachability_tool_call(&db, &foreign, "foreign-tool").await;

    let mut foreign_producer = clone_observation_for_label(&owner_observation, "foreign-producer");
    foreign_producer.producer_worker_run_id = foreign.controller_worker_run_id;
    let foreign_producer_rejected =
        target_intel_asset_observations::insert(db.pool(), &foreign_producer)
            .await
            .is_err();

    let mut foreign_evidence = clone_observation_for_label(&owner_observation, "foreign-evidence");
    foreign_evidence.evidence_id = foreign_evidence_id;
    let foreign_evidence_rejected =
        target_intel_asset_observations::insert(db.pool(), &foreign_evidence)
            .await
            .is_err();

    let mut foreign_producer_tool =
        clone_observation_for_label(&owner_observation, "foreign-producer-tool");
    foreign_producer_tool.producer_tool_call_id = Some(foreign_tool_call_id);
    let foreign_producer_tool_rejected =
        target_intel_asset_observations::insert(db.pool(), &foreign_producer_tool)
            .await
            .is_err();

    let reachability_observation = insert_observation(&db, &owner, "foreign-reachability").await;
    let owned_version = record_attribution(&db, reachability_observation.id, 0, "owned").await;
    let checked_at = Utc::now();
    let foreign_reachability_rejected = target_intel_asset_observations::record_reachability(
        db.pool(),
        &target_intel_asset_observations::RecordReachability {
            observation_id: reachability_observation.id,
            expected_row_version: owned_version,
            state: "reachable".to_string(),
            method: "bounded_http_probe_v1".to_string(),
            tool_call_id: Some(foreign_tool_call_id),
            evidence_id: foreign_evidence_id,
            checked_at,
            valid_until: Some(checked_at + Duration::minutes(10)),
        },
    )
    .await
    .is_err();

    assert_eq!(
        (
            foreign_producer_rejected,
            foreign_evidence_rejected,
            foreign_producer_tool_rejected,
            foreign_reachability_rejected,
        ),
        (true, true, true, true),
        "foreign refs must be rejected in order: producer worker, provider evidence, producer tool, reachability tool+evidence"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn active_exact_recon_tool_may_land_its_observation_before_tool_finish() {
    let (mut db, _data_dir) = migrated_db("active-observation-producer").await;
    let fixture = goal_fixture(&db, 3).await;
    let base = insert_observation(&db, &fixture, "active-producer-base").await;
    let tool_call_id =
        insert_active_controller_tool_call(&db, &fixture, "active-producer", "recon_search_intel")
            .await;
    let mut landed = clone_observation_for_label(&base, "active-producer");
    landed.producer_tool_call_id = Some(tool_call_id);

    assert!(
        target_intel_asset_observations::insert(db.pool(), &landed)
            .await
            .expect("the exact active recon tool owns its in-flight landing"),
        "candidate observations land before the enclosing tool can become finished"
    );

    tool_calls::record_tracked_finish(
        db.pool(),
        tool_call_id,
        fixture.session_id,
        "finished",
        r#"{"status":"landed"}"#,
        1,
    )
    .await
    .expect("finish exact producer tool after observation landing");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn reviewer_reads_structured_work_and_finalizer_rejects_drift_and_orphan_targets() {
    let (mut db, _data_dir) = migrated_db("review-material-closure").await;
    let fixture = goal_fixture(&db, 3).await;
    append_journal(&db, &fixture, 0, "plan_snapshot").await;
    append_journal(&db, &fixture, 1, "completion_checkpoint").await;
    let observation = insert_observation(&db, &fixture, "reviewed-rejected").await;
    record_attribution(&db, observation.id, 0, "rejected").await;

    let reviewer_material = target_intel_goal_reviews::load_freeze_snapshot(
        db.pool(),
        fixture.operation_id,
        fixture.organization_id,
        fixture.stage_execution_id,
        fixture.stage_run_unit_id,
        fixture.team_plan_id,
        fixture.controller_work_item_id,
        fixture.controller_worker_run_id,
        0,
    )
    .await
    .expect("load the exact reviewer material");
    assert_eq!(
        reviewer_material
            .durable_state
            .pointer("/work_journal")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2),
        "reviewer receives structured Controller work memory"
    );
    assert_eq!(
        reviewer_material
            .durable_state
            .pointer("/asset_observations")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1),
        "reviewer sees landed Observations and their attribution state"
    );

    let (review_id, _, bundle_sha256) = freeze_review(&db, &fixture, 0).await;
    append_journal(&db, &fixture, 2, "plan_changed").await;
    sqlx::query(
        r#"INSERT INTO targets(
               name,target_type,value,tags,notes,scope,grp,owner,
               organization_id,project_path,source,liveness_state,liveness_checked_at)
           SELECT 'orphan.example.test','domain','orphan.example.test','[]','',
                  'in'::scope_type,'default','',id,project_path,
                  'target_intel_goal','alive',NOW()
             FROM organizations WHERE id=$1"#,
    )
    .bind(fixture.organization_id)
    .execute(db.pool())
    .await
    .expect("insert a forged orphan formal Target after review freeze");
    let operation_contract_sha256: String = sqlx::query_scalar(
        "SELECT goal_contract_sha256 FROM target_intel_goal_operation_contracts WHERE operation_id=$1",
    )
    .bind(fixture.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("read operation contract seal");
    let review_row_version: i64 =
        sqlx::query_scalar("SELECT row_version FROM target_intel_goal_reviews WHERE id=$1")
            .bind(review_id)
            .fetch_one(db.pool())
            .await
            .expect("read frozen review row version");
    let mut finalizer = target_intel_goal_reviews::load_finalizer_snapshot(
        db.pool(),
        fixture.operation_id,
        fixture.organization_id,
        review_id,
        &bundle_sha256,
        "",
        &operation_contract_sha256,
        review_row_version,
    )
    .await
    .expect("finalizer re-reads locked material instead of trusting reviewer prose");
    assert!(
        !finalizer.material_revision_matches,
        "post-freeze journal drift is visible"
    );
    assert_eq!(
        finalizer.orphan_formal_target_count, 1,
        "orphan Target is visible"
    );

    make_finalizer_preconditions_ready(&mut finalizer);
    assert_eq!(
        finalizer.pass_block_code(),
        Some("INTEL_GOAL_MATERIAL_DRIFT")
    );
    finalizer.material_revision_matches = true;
    assert_eq!(
        finalizer.pass_block_code(),
        Some("INTEL_GOAL_PROMOTION_CLOSURE_INVALID")
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn final_seal_attestation_is_not_post_review_goal_material_drift() {
    let (mut db, _data_dir) = migrated_db("final-seal-attestation-boundary").await;
    let fixture = goal_fixture(&db, 3).await;
    append_journal(&db, &fixture, 0, "plan_snapshot").await;
    append_journal(&db, &fixture, 1, "completion_checkpoint").await;
    let (review_id, _, bundle_sha256) = freeze_review(&db, &fixture, 0).await;
    let operation_contract_sha256: String = sqlx::query_scalar(
        "SELECT goal_contract_sha256 FROM target_intel_goal_operation_contracts WHERE operation_id=$1",
    )
    .bind(fixture.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("read operation contract seal");
    let review_row_version: i64 =
        sqlx::query_scalar("SELECT row_version FROM target_intel_goal_reviews WHERE id=$1")
            .bind(review_id)
            .fetch_one(db.pool())
            .await
            .expect("read frozen review row version");

    sqlx::query(
        r#"INSERT INTO audit_log(
               session_id,action,category,details,project_path,source,tool_name,
               audit_role,detail,run_id,evidence_outcome
           ) VALUES(
               $1,'stage_final_seal_attested','harness',
               'Server attested the exact deterministic Target Intel final seal',
               '/tmp','runtime_memory_final_seal','runtime_memory_final_seal_attestation',
               'evidence',$2,$3,'found'
           )"#,
    )
    .bind(fixture.session_id)
    .bind(json!({
        "kind": "stage_final_seal_attestation",
        "schema_version": 1,
        "operation_id": fixture.operation_id,
        "organization_id": fixture.organization_id,
        "stage_kind": "target_intel",
        "stage_execution_id": fixture.stage_execution_id,
        "stage_run_unit_id": fixture.stage_run_unit_id,
        "worker_run_id": fixture.controller_worker_run_id,
        "deliverable_submission_id": Uuid::new_v4(),
        "scope_hash": "target-intel-scope",
        "gate_decision_hash": format!("sha256:{}", "d".repeat(64)),
        "coverage_watermark_sha256": format!("sha256:{}", "e".repeat(64)),
    }))
    .bind(fixture.operation_id)
    .execute(db.pool())
    .await
    .expect("insert the server-owned final-seal attestation");

    let finalizer = target_intel_goal_reviews::load_finalizer_snapshot(
        db.pool(),
        fixture.operation_id,
        fixture.organization_id,
        review_id,
        &bundle_sha256,
        "",
        &operation_contract_sha256,
        review_row_version,
    )
    .await
    .expect("load finalizer after the ordinary final seal created its attestation");
    assert!(
        finalizer.material_revision_matches,
        "the finalizer's own server attestation is output packaging, not reviewed Goal material"
    );

    sqlx::query(
        r#"INSERT INTO audit_log(
               session_id,action,category,details,project_path,source,
               audit_role,detail,run_id,evidence_outcome
           ) VALUES(
               $1,'post_review_business_evidence','target_intel','fixture drift',
               '/tmp','target_intel_goal','evidence',$2,$3,'found'
           )"#,
    )
    .bind(fixture.session_id)
    .bind(json!({"organization_id": fixture.organization_id, "kind": "business_material"}))
    .bind(fixture.operation_id)
    .execute(db.pool())
    .await
    .expect("insert unrelated post-review business evidence");
    let drifted = target_intel_goal_reviews::load_finalizer_snapshot(
        db.pool(),
        fixture.operation_id,
        fixture.organization_id,
        review_id,
        &bundle_sha256,
        "",
        &operation_contract_sha256,
        review_row_version,
    )
    .await
    .expect("re-read finalizer after actual business evidence drift");
    assert!(
        !drifted.material_revision_matches,
        "unrelated post-review evidence must remain fail-closed material drift"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn checked_empty_query_receipt_is_reviewable_and_closes_finalizer_receipt_authority() {
    let (mut db, _data_dir) = migrated_db("checked-empty-query-receipt").await;
    let fixture = goal_fixture(&db, 3).await;
    append_journal(&db, &fixture, 0, "plan_snapshot").await;
    append_journal(&db, &fixture, 1, "completion_checkpoint").await;
    let tool_call_id = insert_finished_controller_tool_call(
        &db,
        &fixture,
        "company-name-query",
        "recon_search_intel",
    )
    .await;
    let evidence_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO audit_log(
               session_id,action,category,details,project_path,source,audit_role,
               detail,run_id,evidence_outcome
           ) VALUES(
               $1,'target_intel_semantic_query','target_intel',
               'intel.semantic_query_receipt.v1','/tmp','harness','evidence',
               $2,$3,'checked_empty'
           ) RETURNING id"#,
    )
    .bind(fixture.session_id)
    .bind(json!({
        "kind": "target_intel.semantic_query",
        "operation_id": fixture.operation_id,
        "organization_id": fixture.organization_id,
        "goal_epoch_id": fixture.goal_epoch_id,
        "producer_worker_run_id": fixture.controller_worker_run_id,
        "producer_tool_call_id": tool_call_id,
        "provider_run_id": Uuid::new_v4(),
        "pivot_kind": "company_name",
        "pivot_value_sha256": "0".repeat(64),
        "result_status": "complete",
        "provider_status": {"fixture": "empty"},
        "technique_status": {"semantic_search": "checked_empty"},
        "counts": {"candidate_targets": 0, "profile_fields": 0}
    }))
    .bind(fixture.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("insert exact checked-empty semantic query receipt");
    let failed_tool_call_id =
        insert_active_controller_tool_call(&db, &fixture, "failed-query", "recon_search_intel")
            .await;
    tool_calls::record_tracked_finish(
        db.pool(),
        failed_tool_call_id,
        fixture.session_id,
        "failed",
        r#"{"error":"provider landing failed"}"#,
        1,
    )
    .await
    .expect("record failed semantic query tool");
    sqlx::query(
        r#"INSERT INTO audit_log(
               session_id,action,category,details,project_path,source,audit_role,
               detail,run_id,evidence_outcome
           ) VALUES(
               $1,'target_intel_semantic_query','target_intel',
               'intel.semantic_query_receipt.v1','/tmp','harness','evidence',
               $2,$3,'observed'
           )"#,
    )
    .bind(fixture.session_id)
    .bind(json!({
        "kind": "target_intel.semantic_query",
        "operation_id": fixture.operation_id,
        "organization_id": fixture.organization_id,
        "goal_epoch_id": fixture.goal_epoch_id,
        "producer_worker_run_id": fixture.controller_worker_run_id,
        "producer_tool_call_id": failed_tool_call_id,
        "provider_run_id": Uuid::new_v4(),
        "pivot_kind": "brand",
        "pivot_value_sha256": "1".repeat(64),
        "result_status": "partial",
        "provider_status": {"fixture": "observed"},
        "technique_status": {"semantic_search": "observed"},
        "counts": {"candidate_targets": 1, "profile_fields": 0}
    }))
    .bind(fixture.operation_id)
    .execute(db.pool())
    .await
    .expect("insert immutable partial receipt left by failed tool");

    let material = target_intel_goal_reviews::load_freeze_snapshot(
        db.pool(),
        fixture.operation_id,
        fixture.organization_id,
        fixture.stage_execution_id,
        fixture.stage_run_unit_id,
        fixture.team_plan_id,
        fixture.controller_work_item_id,
        fixture.controller_worker_run_id,
        0,
    )
    .await
    .expect("load review material with query receipts");
    let expected_evidence_ref = format!("audit:{evidence_id}");
    assert_eq!(
        material
            .observable_actions
            .pointer("/query_receipts/0/evidence_ref")
            .and_then(Value::as_str),
        Some(expected_evidence_ref.as_str())
    );
    assert_eq!(
        material
            .observable_actions
            .pointer("/query_receipts")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1),
        "a failed tool's partial evidence is immutable history, not a terminal query receipt"
    );

    let (review_id, reviewer_work_item_id, bundle_sha256) = freeze_review(&db, &fixture, 0).await;
    let reviewer_worker_run_id =
        bind_reviewer(&db, &fixture, review_id, reviewer_work_item_id).await;
    let version = read_all_sections(&db, review_id, reviewer_worker_run_id, &bundle_sha256).await;
    let verdict = json!({
        "schema": "intel_review.v1",
        "decision": "PASS",
        "findings": [],
        "inherited_dispositions": [],
        "residuals": ["authorized semantic company-name query checked empty"],
        "human_requirement": null
    });
    let verdict_sha256 = canonical_sha256(&verdict);
    let recorded = target_intel_goal_reviews::record_terminal_verdict(
        db.pool(),
        review_id,
        reviewer_worker_run_id,
        0,
        version,
        &bundle_sha256,
        "pass",
        &verdict,
        &verdict_sha256,
    )
    .await
    .expect("record reviewer PASS for checked-empty authority");
    finish_reviewer(&db, reviewer_work_item_id, reviewer_worker_run_id).await;
    let operation_contract_sha256: String = sqlx::query_scalar(
        "SELECT goal_contract_sha256 FROM target_intel_goal_operation_contracts WHERE operation_id=$1",
    )
    .bind(fixture.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("read operation contract seal");
    let finalizer = target_intel_goal_reviews::load_finalizer_snapshot(
        db.pool(),
        fixture.operation_id,
        fixture.organization_id,
        review_id,
        &bundle_sha256,
        &verdict_sha256,
        &operation_contract_sha256,
        recorded.review_row_version,
    )
    .await
    .expect("load deterministic finalizer snapshot");
    assert!(finalizer.material_revision_matches);
    assert_eq!(finalizer.current_run_terminal_receipt_count, 1);
    assert_eq!(finalizer.valid_evidence_artifact_closure_count, 1);

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn candidate_receipt_without_landed_observation_cannot_close_finalizer_authority() {
    let (mut db, _data_dir) = migrated_db("orphan-semantic-receipt").await;
    let fixture = goal_fixture(&db, 3).await;
    append_journal(&db, &fixture, 0, "plan_snapshot").await;
    append_journal(&db, &fixture, 1, "completion_checkpoint").await;
    let artifact_sha256 = "a".repeat(64);
    let artifact_ref = format!("intel-artifact:sha256:{artifact_sha256}");
    sqlx::query(
        r#"INSERT INTO target_intel_semantic_artifacts(
               artifact_ref,operation_id,organization_id,session_id,
               artifact_sha256,redacted_payload
           ) VALUES($1,$2,$3,$4,$5,$6)"#,
    )
    .bind(&artifact_ref)
    .bind(fixture.operation_id)
    .bind(fixture.organization_id)
    .bind(fixture.session_id)
    .bind(&artifact_sha256)
    .bind(json!({"candidate": "orphan.example.test"}))
    .execute(db.pool())
    .await
    .expect("insert exact semantic artifact");
    let evidence_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO audit_log(
               session_id,action,category,details,project_path,source,tool_name,
               status,audit_role,detail,run_id,evidence_outcome
           ) VALUES(
               $1,'target_intel_observation','target_intel',
               'intel.semantic_observation.v1','/tmp','harness','recon_search_intel',
               'completed','evidence',$2,$3,'observed'
           ) RETURNING id"#,
    )
    .bind(fixture.session_id)
    .bind(json!({
        "kind": "target_intel.semantic_pivot",
        "organization_id": fixture.organization_id,
        "raw_output": {"artifact_ref": artifact_ref, "artifact_sha256": artifact_sha256}
    }))
    .bind(fixture.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("insert candidate evidence without an Observation");
    let semantic_receipt_audit_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO audit_log(
               session_id,action,category,details,project_path,source,tool_name,status,detail
           ) VALUES(
               $1,'target_intel_semantic_receipt','target_intel',
               'intel.semantic_pivot_receipt.v1','/tmp','target_intel_goal',
               'recon_search_intel','succeeded',$2
           ) RETURNING id"#,
    )
    .bind(fixture.session_id)
    .bind(json!({
        "operation_id": fixture.operation_id,
        "organization_id": fixture.organization_id,
        "stable_query_key": "orphan-semantic:v1",
        "provider_id": "fixture-provider",
        "query_type": "brand",
        "adapter_version": "fixture.v1",
        "artifact_ref": artifact_ref,
        "artifact_sha256": artifact_sha256,
        "evidence_ref": format!("audit:{evidence_id}"),
        "unauthorized_promotion_refs": []
    }))
    .fetch_one(db.pool())
    .await
    .expect("insert candidate-bearing terminal receipt without Observation landing");

    let (review_id, reviewer_work_item_id, bundle_sha256) = freeze_review(&db, &fixture, 0).await;
    let reviewer_worker_run_id =
        bind_reviewer(&db, &fixture, review_id, reviewer_work_item_id).await;
    let version = read_all_sections(&db, review_id, reviewer_worker_run_id, &bundle_sha256).await;
    let verdict = json!({
        "schema": "intel_review.v1",
        "decision": "PASS",
        "findings": [],
        "inherited_dispositions": [],
        "residuals": [],
        "human_requirement": null
    });
    let verdict_sha256 = canonical_sha256(&verdict);
    let recorded = target_intel_goal_reviews::record_terminal_verdict(
        db.pool(),
        review_id,
        reviewer_worker_run_id,
        0,
        version,
        &bundle_sha256,
        "pass",
        &verdict,
        &verdict_sha256,
    )
    .await
    .expect("record reviewer PASS to exercise deterministic closure");
    finish_reviewer(&db, reviewer_work_item_id, reviewer_worker_run_id).await;
    let operation_contract_sha256: String = sqlx::query_scalar(
        "SELECT goal_contract_sha256 FROM target_intel_goal_operation_contracts WHERE operation_id=$1",
    )
    .bind(fixture.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("read operation contract seal");
    let mut finalizer = target_intel_goal_reviews::load_finalizer_snapshot(
        db.pool(),
        fixture.operation_id,
        fixture.organization_id,
        review_id,
        &bundle_sha256,
        &verdict_sha256,
        &operation_contract_sha256,
        recorded.review_row_version,
    )
    .await
    .expect("load deterministic finalizer snapshot");

    assert_eq!(finalizer.current_run_terminal_receipt_count, 1);
    assert_eq!(finalizer.valid_evidence_artifact_closure_count, 0);
    make_finalizer_preconditions_ready(&mut finalizer);
    finalizer.current_run_terminal_receipt_count = 1;
    finalizer.valid_evidence_artifact_closure_count = 0;
    assert_eq!(
        finalizer.pass_block_code(),
        Some("INTEL_GOAL_NON_VACUITY_FAILED"),
        "an artifact/evidence receipt is not closed until the typed Observation exists"
    );

    let template = insert_observation(&db, &fixture, "production-source-template").await;
    let mut linked = clone_observation_for_label(&template, "production-source-linked");
    linked.semantic_receipt_audit_id = Some(semantic_receipt_audit_id);
    linked.evidence_id = evidence_id;
    linked.artifact_ref = artifact_ref;
    linked.artifact_sha256 = artifact_sha256;
    assert!(
        target_intel_asset_observations::insert(db.pool(), &linked)
            .await
            .expect("land the exact production-source Observation"),
        "the linked production-source Observation must be novel"
    );
    let production_closed = target_intel_goal_reviews::load_finalizer_snapshot(
        db.pool(),
        fixture.operation_id,
        fixture.organization_id,
        review_id,
        &bundle_sha256,
        &verdict_sha256,
        &operation_contract_sha256,
        recorded.review_row_version,
    )
    .await
    .expect("re-read deterministic finalizer after production-source Observation landing");
    assert_eq!(production_closed.current_run_terminal_receipt_count, 1);
    assert_eq!(
        production_closed.valid_evidence_artifact_closure_count, 1,
        "the production source closes only through the same exact artifact/evidence/Observation joins as shadow"
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn review_freeze_seals_epoch_and_creates_only_the_authorized_reviewer() {
    let (mut db, _data_dir) = migrated_db("freeze").await;
    let fixture = goal_fixture(&db, 3).await;
    let (review_id, reviewer_work_item_id, _) = freeze_review(&db, &fixture, 0).await;

    let plan: (bool, Option<Uuid>, i64) = sqlx::query_as(
        "SELECT requests_closed_at IS NOT NULL,final_submitter_worker_run_id,row_version FROM stage_team_plans WHERE id=$1",
    )
    .bind(fixture.team_plan_id)
    .fetch_one(db.pool())
    .await
    .expect("read sealed review plan");
    assert!(plan.0, "review freeze closes ordinary requests atomically");
    assert_eq!(plan.1, None, "review must precede final-submitter binding");
    let epoch: (String, i64) =
        sqlx::query_as("SELECT status,row_version FROM target_intel_goal_epochs WHERE id=$1")
            .bind(fixture.goal_epoch_id)
            .fetch_one(db.pool())
            .await
            .expect("read sealed goal epoch");
    assert_eq!(epoch, ("sealed_for_review".to_string(), 1));
    sqlx::query(
        r#"UPDATE stage_team_plans
              SET final_submitter_worker_run_id=$2,row_version=row_version+1
            WHERE id=$1 AND requests_closed_at IS NOT NULL
              AND final_submitter_worker_run_id IS NULL"#,
    )
    .bind(fixture.team_plan_id)
    .bind(fixture.controller_worker_run_id)
    .execute(db.pool())
    .await
    .expect("bind the exact Controller as final submitter");
    let contract = target_intel_goal_contracts::get_by_operation(db.pool(), fixture.operation_id)
        .await
        .expect("read frozen Goal contract")
        .expect("Goal contract must exist");
    assert!(
        target_intel_goal_contracts::freeze_unit(
            db.pool(),
            &target_intel_goal_contracts::FreezeTargetIntelGoalUnit {
                contract,
                organization_id: fixture.organization_id,
                team_plan_id: fixture.team_plan_id,
                goal_epoch_id: fixture.goal_epoch_id,
                controller_work_item_id: fixture.controller_work_item_id,
                controller_worker_run_id: fixture.controller_worker_run_id,
                controller_message_chain_id: fixture.controller_message_chain_id,
            },
        )
        .await
        .expect("replay the frozen contract from the sealed final-submitter state"),
        "the sealed final-submitter path must replay rather than create a second contract"
    );
    let reviewer: (String, String, String, bool) = sqlx::query_as(
        r#"SELECT created_by,execution_profile,terminal_contract,required_for_barrier
             FROM stage_work_items WHERE id=$1"#,
    )
    .bind(reviewer_work_item_id)
    .fetch_one(db.pool())
    .await
    .expect("read reviewer work item");
    assert_eq!(
        reviewer,
        (
            "target_intel_review_freeze".to_string(),
            "read_only_reviewer".to_string(),
            "intel_review_v1".to_string(),
            false,
        )
    );
    let authority_status: String = sqlx::query_scalar(
        "SELECT status FROM target_intel_goal_review_freeze_authorities WHERE review_id=$1",
    )
    .bind(review_id)
    .fetch_one(db.pool())
    .await
    .expect("read applied reviewer authority");
    assert_eq!(authority_status, "applied");

    let forged = sqlx::query(
        r#"INSERT INTO stage_work_items(
               id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
               input_manifest_hash,input_refs,required_for_barrier,priority,status,
               attempt_policy,budget,output_schema,created_by,
               execution_profile,terminal_contract
           ) SELECT $1,id,operation_id,stage_execution_id,stage_run_unit_id,
                    scope_snapshot_id,organization_id,dispatch_epoch,
                    'target_intel_read_only_review','forged-reviewer','intel_goal_reviewer',
                    $2,'[]'::jsonb,FALSE,-100,'queued','{}'::jsonb,'{}'::jsonb,
                    'intel_review.v1','target_intel_review_freeze',
                    'read_only_reviewer','intel_review_v1'
               FROM stage_team_plans WHERE id=$3"#,
    )
    .bind(Uuid::new_v4())
    .bind(format!("sha256:{}", "f".repeat(64)))
    .bind(fixture.team_plan_id)
    .execute(db.pool())
    .await;
    assert!(
        forged.is_err(),
        "created_by alone cannot forge reviewer authority"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn reviewer_verdict_accepts_the_exact_prebound_controller_submission() {
    let (mut db, _data_dir) = migrated_db("prebound_controller_submission").await;
    let fixture = goal_fixture(&db, 3).await;
    bind_exact_controller_final_submission(&db, &fixture).await;

    let (review_id, reviewer_work_item_id, bundle_sha256) = freeze_review(&db, &fixture, 0).await;
    let reviewer_worker_run_id =
        bind_reviewer(&db, &fixture, review_id, reviewer_work_item_id).await;
    let version = read_all_sections(&db, review_id, reviewer_worker_run_id, &bundle_sha256).await;
    let verdict = json!({
        "schema": "intel_review.v1",
        "decision": "PASS",
        "findings": [],
        "inherited_dispositions": [],
        "residuals": [],
        "human_requirement": null
    });
    let verdict_sha256 = canonical_sha256(&verdict);

    let recorded = target_intel_goal_reviews::record_terminal_verdict(
        db.pool(),
        review_id,
        reviewer_worker_run_id,
        0,
        version,
        &bundle_sha256,
        "pass",
        &verdict,
        &verdict_sha256,
    )
    .await
    .expect("exact prebound Controller submission remains valid verdict authority");
    assert_eq!(recorded.effective_decision, "pass");

    record_finished_reviewer_submit_result(&db, &fixture, reviewer_worker_run_id).await;
    finish_reviewer(&db, reviewer_work_item_id, reviewer_worker_run_id).await;
    let operation_contract_sha256: String = sqlx::query_scalar(
        "SELECT goal_contract_sha256 FROM target_intel_goal_operation_contracts WHERE operation_id=$1",
    )
    .bind(fixture.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("read operation contract seal");
    let finalizer = target_intel_goal_reviews::load_finalizer_snapshot(
        db.pool(),
        fixture.operation_id,
        fixture.organization_id,
        review_id,
        &bundle_sha256,
        &verdict_sha256,
        &operation_contract_sha256,
        recorded.review_row_version,
    )
    .await
    .expect("load finalizer after the reviewer terminal protocol call");
    assert!(
        finalizer.material_revision_matches,
        "the exact read-only reviewer submit_result is protocol output, not Controller material"
    );
    assert_eq!(
        target_intel_goal_reviews::find_exact_freeze_replay(
            db.pool(),
            fixture.operation_id,
            fixture.organization_id,
            fixture.team_plan_id,
            0,
            fixture.controller_work_item_id,
            fixture.controller_worker_run_id,
            "all material paths exhausted",
        )
        .await
        .expect("find the exact frozen review after reviewer protocol output"),
        Some((1, 1)),
        "response-loss replay ignores only the bound read-only reviewer protocol"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn reviewer_rework_reopens_the_exact_prebound_controller_submission() {
    let (mut db, _data_dir) = migrated_db("prebound_controller_rework").await;
    let fixture = goal_fixture(&db, 3).await;
    bind_exact_controller_final_submission(&db, &fixture).await;
    let evidence_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO audit_log(
               action,category,details,project_path,source,audit_role,detail,run_id
           ) VALUES(
               'target intel review evidence','harness','material gap','/tmp',
               'harness','evidence',$1,$2
           ) RETURNING id"#,
    )
    .bind(json!({"organization_id": fixture.organization_id}))
    .bind(fixture.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("insert exact review evidence");

    let (review_id, reviewer_work_item_id, bundle_sha256) = freeze_review(&db, &fixture, 0).await;
    let reviewer_worker_run_id =
        bind_reviewer(&db, &fixture, review_id, reviewer_work_item_id).await;
    let version = read_all_sections(&db, review_id, reviewer_worker_run_id, &bundle_sha256).await;
    let mut finding = json!({
        "finding_id": Uuid::new_v4(),
        "fingerprint": "",
        "materiality": "major",
        "subject_refs": ["org:fixture"],
        "reason": "a discovered candidate lacks a terminal disposition",
        "evidence_refs": [format!("audit:{evidence_id}")],
        "action_kind": "promote_or_disposition",
        "capability_ref": "target_intel.promote_candidate",
        "close_condition": "record one terminal candidate disposition"
    });
    finding["fingerprint"] = Value::String(
        target_intel_goal_reviews::compute_finding_fingerprint(&finding)
            .expect("host computes finding fingerprint"),
    );
    let verdict = json!({
        "schema":"intel_review.v1",
        "decision":"REWORK",
        "findings":[finding],
        "inherited_dispositions":[],
        "residuals":[],
        "human_requirement":null
    });
    let verdict_sha256 = canonical_sha256(&verdict);

    let recorded = target_intel_goal_reviews::record_terminal_verdict(
        db.pool(),
        review_id,
        reviewer_worker_run_id,
        0,
        version,
        &bundle_sha256,
        "rework",
        &verdict,
        &verdict_sha256,
    )
    .await
    .expect("rework clears the exact Controller submission and resumes its same chain");
    assert_eq!(recorded.effective_decision, "rework");
    assert!(recorded.successor_goal_epoch_id.is_some());
    let plan: (i64, bool, Option<Uuid>) = sqlx::query_as(
        r#"SELECT dispatch_epoch,requests_closed_at IS NULL,
                  final_submitter_worker_run_id
             FROM stage_team_plans WHERE id=$1"#,
    )
    .bind(fixture.team_plan_id)
    .fetch_one(db.pool())
    .await
    .expect("read same-chain reopened plan");
    assert_eq!(plan, (1, true, None));

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn rework_advances_the_same_chain_and_fixed_point_requires_typed_human_fulfillment() {
    let (mut db, _data_dir) = migrated_db("rework").await;
    let fixture = goal_fixture(&db, 3).await;
    let evidence_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO audit_log(
               action,category,details,project_path,source,audit_role,detail,run_id
           ) VALUES(
               'target intel review evidence','harness','material gap','/tmp',
               'harness','evidence',$1,$2
           ) RETURNING id"#,
    )
    .bind(json!({"organization_id": fixture.organization_id}))
    .bind(fixture.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("insert exact review evidence");

    let (review_id, reviewer_work_item_id, bundle_sha256) = freeze_review(&db, &fixture, 0).await;
    let reviewer_worker_run_id =
        bind_reviewer(&db, &fixture, review_id, reviewer_work_item_id).await;
    let version = read_all_sections(&db, review_id, reviewer_worker_run_id, &bundle_sha256).await;
    let finding_id = Uuid::new_v4();
    let mut finding = json!({
        "finding_id": finding_id,
        "fingerprint": "",
        "materiality": "major",
        "subject_refs": ["org:fixture"],
        "reason": "a supported company pivot was not executed",
        "evidence_refs": [format!("audit:{evidence_id}")],
        "action_kind": "semantic_pivot",
        "capability_ref": "company_name",
        "close_condition": "record one terminal receipt"
    });
    finding["fingerprint"] = Value::String(
        target_intel_goal_reviews::compute_finding_fingerprint(&finding)
            .expect("host computes finding fingerprint"),
    );
    let verdict = json!({
        "schema":"intel_review.v1",
        "decision":"REWORK",
        "findings":[finding],
        "inherited_dispositions":[],
        "residuals":[],
        "human_requirement":null
    });
    let verdict_sha256 = canonical_sha256(&verdict);
    let recorded = target_intel_goal_reviews::record_terminal_verdict(
        db.pool(),
        review_id,
        reviewer_worker_run_id,
        0,
        version,
        &bundle_sha256,
        "rework",
        &verdict,
        &verdict_sha256,
    )
    .await
    .expect("record actionable rework and resume exact controller");
    assert_eq!(recorded.effective_decision, "rework");
    assert!(recorded.hold_id.is_none());
    assert!(recorded.successor_goal_epoch_id.is_some());
    let plan: (i64, bool) = sqlx::query_as(
        "SELECT dispatch_epoch,requests_closed_at IS NULL FROM stage_team_plans WHERE id=$1",
    )
    .bind(fixture.team_plan_id)
    .fetch_one(db.pool())
    .await
    .expect("read reopened plan");
    assert_eq!(plan, (1, true));
    let successor: (i64, String, Uuid, Uuid, Option<Uuid>) = sqlx::query_as(
        r#"SELECT epoch,status,controller_work_item_id,controller_worker_run_id,
                  controller_message_chain_id
             FROM target_intel_goal_epochs
            WHERE team_plan_id=$1 AND epoch=1"#,
    )
    .bind(fixture.team_plan_id)
    .fetch_one(db.pool())
    .await
    .expect("read successor goal epoch");
    assert_eq!(successor.0, 1);
    assert_eq!(successor.1, "open");
    assert_eq!(successor.2, fixture.controller_work_item_id);
    assert_eq!(successor.3, fixture.controller_worker_run_id);
    assert_eq!(successor.4, Some(fixture.controller_message_chain_id));
    let resumed_controller: (String, i64, String, Option<Uuid>, Value) = sqlx::query_as(
        r#"SELECT item.status,item.dispatch_epoch,worker.status,worker.message_chain_id,chain.chain
             FROM stage_work_items item
             JOIN stage_worker_runs worker ON worker.id=$2 AND worker.work_item_id=item.id
             JOIN message_chains chain ON chain.id=worker.message_chain_id
            WHERE item.id=$1"#,
    )
    .bind(fixture.controller_work_item_id)
    .bind(fixture.controller_worker_run_id)
    .fetch_one(db.pool())
    .await
    .expect("read same-chain REWORK continuation state");
    assert_eq!(resumed_controller.0, "waiting_dependency");
    assert_eq!(resumed_controller.1, 1);
    assert_eq!(resumed_controller.2, "waiting_background");
    assert_eq!(
        resumed_controller.3,
        Some(fixture.controller_message_chain_id)
    );
    let continuation = resumed_controller
        .4
        .as_array()
        .and_then(|messages| messages.last())
        .and_then(|message| message.pointer("/content/0/text"))
        .and_then(Value::as_str)
        .expect("trusted review continuation is appended to the exact chain");
    assert!(continuation.contains("trusted_target_intel_review_continuation"));
    assert!(continuation.contains(&review_id.to_string()));
    assert!(continuation.contains("a supported company pivot was not executed"));
    finish_reviewer(&db, reviewer_work_item_id, reviewer_worker_run_id).await;

    let (second_review_id, second_reviewer_item_id, second_bundle_sha256) =
        freeze_review(&db, &fixture, 1).await;
    let second_reviewer_worker_id =
        bind_reviewer(&db, &fixture, second_review_id, second_reviewer_item_id).await;
    let second_version = read_all_sections(
        &db,
        second_review_id,
        second_reviewer_worker_id,
        &second_bundle_sha256,
    )
    .await;
    let second_finding_id = Uuid::new_v4();
    let mut same_finding = verdict["findings"][0].clone();
    same_finding["finding_id"] = Value::String(second_finding_id.to_string());
    let second_verdict = json!({
        "schema":"intel_review.v1",
        "decision":"REWORK",
        "findings":[same_finding],
        "inherited_dispositions":[{
            "finding_id":finding_id,
            "disposition":"still_open",
            "resolution_refs":[],
            "reason":"no material state or action delta"
        }],
        "residuals":["same material gap"],
        "human_requirement":null
    });
    let second_verdict_sha256 = canonical_sha256(&second_verdict);
    let fixed_point = target_intel_goal_reviews::record_terminal_verdict(
        db.pool(),
        second_review_id,
        second_reviewer_worker_id,
        0,
        second_version,
        &second_bundle_sha256,
        "rework",
        &second_verdict,
        &second_verdict_sha256,
    )
    .await
    .expect("same finding without material delta atomically becomes a hold");
    assert_eq!(fixed_point.effective_decision, "needs_human");
    assert!(fixed_point.successor_goal_epoch_id.is_none());
    let hold_id = fixed_point.hold_id.expect("fixed point creates typed hold");
    let hold: (String, String, i64) = sqlx::query_as(
        "SELECT requirement_kind,status,row_version FROM target_intel_goal_holds WHERE id=$1",
    )
    .bind(hold_id)
    .fetch_one(db.pool())
    .await
    .expect("read fixed-point hold");
    assert_eq!(
        hold,
        ("review_fixed_point".to_string(), "open".to_string(), 0)
    );

    let resumed = target_intel_goal_reviews::fulfill_hold_and_resume(
        db.pool(),
        &target_intel_goal_reviews::FulfillTargetIntelGoalHold {
            fulfillment_id: Uuid::new_v4(),
            hold_id,
            expected_hold_row_version: 0,
            fulfillment_kind: "operator_override".to_string(),
            authority_ref: "operator:fixture-approved".to_string(),
            material_input: json!({"decision":"continue bounded review"}),
        },
    )
    .await
    .expect("typed operator authority resumes exact held controller");
    assert_eq!(resumed.successor_goal_epoch, 2);
    assert_eq!(
        resumed.controller_work_item_id,
        fixture.controller_work_item_id
    );
    assert_eq!(
        resumed.controller_worker_run_id,
        fixture.controller_worker_run_id
    );
    assert_eq!(
        resumed.controller_message_chain_id,
        fixture.controller_message_chain_id
    );
    let immutable_fulfillment = sqlx::query(
        "UPDATE target_intel_goal_hold_fulfillments SET authority_ref='forged' WHERE id=$1",
    )
    .bind(resumed.fulfillment_id)
    .execute(db.pool())
    .await;
    assert!(
        immutable_fulfillment.is_err(),
        "fulfillment authority is append-only"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn frontier_claim_lease_terminal_materiality_and_waiver_are_database_authority() {
    let (mut db, _data_dir) = migrated_db("frontier").await;
    let fixture = goal_fixture(&db, 3).await;
    let frontier_id = Uuid::new_v4();
    let inserted = target_intel_goal_frontier::insert_pending(
        db.pool(),
        &target_intel_goal_frontier::InsertTargetIntelFrontier {
            id: frontier_id,
            operation_id: fixture.operation_id,
            organization_id: fixture.organization_id,
            stage_execution_id: fixture.stage_execution_id,
            stage_run_unit_id: fixture.stage_run_unit_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            team_plan_id: fixture.team_plan_id,
            goal_epoch_id: fixture.goal_epoch_id,
            semantic_pivot_key: "company-name:goal-org".to_string(),
            pivot_kind: "company_name".to_string(),
            pivot_value_sha256: format!("sha256:{}", "d".repeat(64)),
            intent: "discover_related_assets".to_string(),
            materiality: "material".to_string(),
            provenance: json!({"source":"controller"}),
        },
    )
    .await
    .expect("insert exact material frontier");
    assert_eq!(inserted.row_version, 0);
    let lease_token = Uuid::new_v4();
    let claimed = target_intel_goal_frontier::claim(
        db.pool(),
        &target_intel_goal_frontier::ClaimTargetIntelFrontier {
            frontier_id,
            operation_id: fixture.operation_id,
            organization_id: fixture.organization_id,
            expected_row_version: 0,
            claimed_by_worker_run_id: fixture.controller_worker_run_id,
            claim_attempt_epoch: 0,
            lease_token,
            lease_seconds: 60,
        },
    )
    .await
    .expect("claim frontier with exact worker fence");
    assert_eq!(claimed.row_version, 1);
    let wrong_lease = target_intel_goal_frontier::transition_claimed(
        db.pool(),
        &target_intel_goal_frontier::TransitionClaimedTargetIntelFrontier {
            frontier_id,
            operation_id: fixture.operation_id,
            organization_id: fixture.organization_id,
            expected_row_version: 1,
            claimed_by_worker_run_id: fixture.controller_worker_run_id,
            claim_attempt_epoch: 0,
            lease_token: Uuid::new_v4(),
            to_status: "unsupported".to_string(),
            terminal_refs: json!([]),
            capability_ref: Some("adapter:icp".to_string()),
            reason: Some("no frozen adapter".to_string()),
        },
    )
    .await;
    assert!(
        wrong_lease.is_err(),
        "a foreign lease cannot terminalize frontier"
    );
    let terminal = target_intel_goal_frontier::transition_claimed(
        db.pool(),
        &target_intel_goal_frontier::TransitionClaimedTargetIntelFrontier {
            frontier_id,
            operation_id: fixture.operation_id,
            organization_id: fixture.organization_id,
            expected_row_version: 1,
            claimed_by_worker_run_id: fixture.controller_worker_run_id,
            claim_attempt_epoch: 0,
            lease_token,
            to_status: "unsupported".to_string(),
            terminal_refs: json!([]),
            capability_ref: Some("adapter:icp".to_string()),
            reason: Some("no frozen adapter".to_string()),
        },
    )
    .await
    .expect("lease owner records typed unsupported terminal state");
    assert_eq!(terminal.row_version, 2);
    let waiver_evidence_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO audit_log(
               action,category,details,project_path,source,audit_role,detail,run_id
           ) VALUES(
               'operator frontier waiver','authorization','bounded unsupported residual',
               '/tmp','operator','evidence',$1,$2
           ) RETURNING id"#,
    )
    .bind(json!({"organization_id": fixture.organization_id}))
    .bind(fixture.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("insert exact operator waiver evidence");
    let revision_before_waiver: i64 = sqlx::query_scalar(
        "SELECT state_revision FROM target_intel_goal_material_revisions WHERE operation_id=$1 AND organization_id=$2",
    )
    .bind(fixture.operation_id)
    .bind(fixture.organization_id)
    .fetch_one(db.pool())
    .await
    .expect("read material revision before waiver");
    let waiver = target_intel_goal_frontier::waive_terminal_gap(
        db.pool(),
        &target_intel_goal_frontier::WaiveTargetIntelFrontierGap {
            waiver_id: Uuid::new_v4(),
            frontier_id,
            operation_id: fixture.operation_id,
            organization_id: fixture.organization_id,
            expected_frontier_row_version: 2,
            authority_kind: "human_operator".to_string(),
            authority_ref: "operator:fixture-scope".to_string(),
            evidence_refs: json!([format!("audit:{waiver_evidence_id}")]),
            reason: "operator accepts the bounded unsupported residual".to_string(),
        },
    )
    .await
    .expect("human authority waives exact material capability gap");
    assert!(!waiver.replayed);
    let revision_after_waiver: i64 = sqlx::query_scalar(
        "SELECT state_revision FROM target_intel_goal_material_revisions WHERE operation_id=$1 AND organization_id=$2",
    )
    .bind(fixture.operation_id)
    .bind(fixture.organization_id)
    .fetch_one(db.pool())
    .await
    .expect("read material revision after waiver");
    assert_eq!(revision_after_waiver, revision_before_waiver + 1);
    let mutate_event = sqlx::query(
        "UPDATE target_intel_goal_frontier_events SET to_status='resolved' WHERE frontier_id=$1",
    )
    .bind(frontier_id)
    .execute(db.pool())
    .await;
    assert!(mutate_event.is_err(), "frontier history is append-only");

    db.stop().await;
}
