use std::sync::Arc;

use chrono::{Duration, Utc};
use golish_agent_app::ai::db_bridge::reporting::{
    current_reportable_source_snapshot, load_report_bundle, load_reporting_gate_truth_with_barrier,
    PgReportPublicationPort, PgReportTruthPort, ReportingProjectAuthority,
};
use golish_agent_app::ai::db_bridge::GolishDbRepoProvider;
use golish_agent_kit::db_traits::DbRepoProvider;
use golish_agent_kit::harness::{
    validate_reporting_gate_truth, CanonicalFactKey, CanonicalFactRef, StageHandoffPayload,
};
use golish_db::models::NewSession;
use golish_db::repo::{
    cleanup_obligations, operator_principals, project_scopes, runtime_memory_tx, sessions,
};
use golish_db::{DbConfig, GolishDb};
use golish_memory_domain::source_ref::CanonicalRowId;
use golish_reporting_app::{
    FinalizePublication, ReportPublicationPort, ReportReadModelBuilder, ReportTruthPort,
};
use golish_reporting_domain::ReportSourceKind;
use serde_json::{json, Value};
use serial_test::serial;
use sha2::{Digest, Sha256};
use tokio::sync::Notify;
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
        database: format!("reporting_authority_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    };
    let db = GolishDb::start(config)
        .await
        .expect("start migrated embedded postgres");
    (db, data_dir)
}

#[derive(Clone, Copy)]
struct FrozenScope {
    operation_id: Uuid,
    project_scope_id: Uuid,
    scope_snapshot_id: Uuid,
    stage_execution_id: Uuid,
    organization_id: Uuid,
}

async fn frozen_scope(db: &GolishDb, project_path: &str) -> FrozenScope {
    let session = sessions::create(
        db.pool(),
        NewSession {
            title: Some("reporting authority fixture".to_string()),
            workspace_path: Some(project_path.to_string()),
            workspace_label: None,
            model: None,
            provider: None,
            project_path: Some(project_path.to_string()),
        },
    )
    .await
    .expect("create reporting session");
    let project_scope =
        project_scopes::register_first_open(db.pool(), project_path, &"1".repeat(64))
            .await
            .expect("register project scope");
    let operation_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    runtime_memory_tx::create_runtime_operation(
        db.pool(),
        &runtime_memory_tx::CreateRuntimeOperationRow {
            operation_id,
            initial_stage_execution_id: stage_execution_id,
            session_id: session.id,
            title: Some("reporting authority operation".to_string()),
            input: "reporting authority fixture".to_string(),
            profile: "assessment".to_string(),
            entry_stage: "target_intel".to_string(),
            project_scope_id: project_scope.project_scope_id,
            cli_scope: None,
        },
    )
    .await
    .expect("create runtime operation");
    let organization_id = Uuid::new_v4();
    let decision_id = Uuid::new_v4();
    let scope_snapshot_id = Uuid::new_v4();
    let scope_hash = "3".repeat(64);
    let mut tx = db.pool().begin().await.expect("begin frozen scope");
    sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$2,'Report Org')")
        .bind(organization_id)
        .bind(project_path)
        .execute(&mut *tx)
        .await
        .expect("insert organization");
    sqlx::query(
        r#"INSERT INTO operation_scope_decisions(
               id,operation_id,project_scope_id,stage_execution_id,
               root_organization_id,mode,decision_rows,decision_hash
           ) VALUES($1,$2,$3,$4,$5,'cli_flags',$6,$7)"#,
    )
    .bind(decision_id)
    .bind(operation_id)
    .bind(project_scope.project_scope_id)
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
    .bind(scope_snapshot_id)
    .bind(operation_id)
    .bind(project_scope.project_scope_id)
    .bind(decision_id)
    .bind(project_path)
    .bind(organization_id)
    .bind(&scope_hash)
    .execute(&mut *tx)
    .await
    .expect("insert scope snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units(
               snapshot_id,organization_id,organization_name_at_freeze,
               role,depth,ordinal,decision_row_id,approval_source
           ) VALUES($1,$2,'Report Org','root',0,0,'root',$3)"#,
    )
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(serde_json::json!({"source": "cli_flags"}))
    .execute(&mut *tx)
    .await
    .expect("insert scope unit");
    sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
        .bind(scope_snapshot_id)
        .execute(&mut *tx)
        .await
        .expect("seal scope");
    tx.commit().await.expect("commit frozen scope");
    FrozenScope {
        operation_id,
        project_scope_id: project_scope.project_scope_id,
        scope_snapshot_id,
        stage_execution_id,
        organization_id,
    }
}

async fn reporting_project_authority(
    db: &GolishDb,
    scope: FrozenScope,
) -> ReportingProjectAuthority {
    let project = project_scopes::get_active_for_share(db.pool(), scope.project_scope_id)
        .await
        .expect("load active Reporting project authority")
        .expect("Reporting project remains active");
    let scope_hash: String =
        sqlx::query_scalar("SELECT scope_hash FROM operation_org_scope_snapshots WHERE id=$1")
            .bind(scope.scope_snapshot_id)
            .fetch_one(db.pool())
            .await
            .expect("load Reporting scope hash");
    ReportingProjectAuthority::new(
        project.project_scope_id,
        scope.scope_snapshot_id,
        scope_hash,
        project.canonical_project_path,
        project.path_sha256,
        project.row_version,
    )
}

async fn insert_episode(db: &GolishDb, scope: FrozenScope, label: &str) -> (Uuid, i64) {
    let evidence_id = insert_evidence(db, scope, label).await;
    let episode_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO stage_episodes(
               episode_id,project_scope_id,source_operation_id,
               organization_id_at_time,source_scope_snapshot_hash,
               stage_execution_id,stage_kind,verdict,reason_codes,fact_refs,
               evidence_refs,started_at,ended_at
           ) VALUES($1,$2,$3,$4,$5,$6,'enumeration','passed','[]','[]',$7,NOW(),NOW())"#,
    )
    .bind(episode_id)
    .bind(scope.project_scope_id)
    .bind(scope.operation_id)
    .bind(scope.organization_id)
    .bind("3".repeat(64))
    .bind(scope.stage_execution_id)
    .bind(vec![evidence_id])
    .execute(db.pool())
    .await
    .expect("insert canonical stage episode");
    (episode_id, evidence_id)
}

async fn insert_evidence(db: &GolishDb, scope: FrozenScope, label: &str) -> i64 {
    let project_path: String = sqlx::query_scalar(
        "SELECT project_path_at_freeze FROM operation_org_scope_snapshots WHERE id=$1",
    )
    .bind(scope.scope_snapshot_id)
    .fetch_one(db.pool())
    .await
    .expect("load frozen evidence project path");
    sqlx::query_scalar(
        r#"INSERT INTO audit_log(
               action,category,details,project_path,source,status,detail,run_id,audit_role
           ) VALUES($1,'harness',$2,$3,'harness',
                    'completed',$4,$5,'evidence') RETURNING id"#,
    )
    .bind(label)
    .bind(format!("{label} evidence"))
    .bind(project_path)
    .bind(serde_json::json!({"organization_id": scope.organization_id}))
    .bind(scope.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("insert exact evidence")
}

async fn insert_isolated_terminal_candidate_attempt(
    db: &GolishDb,
    scope: FrozenScope,
    project_path: &str,
) -> (Uuid, Uuid) {
    let target_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO targets(
               id,name,target_type,value,scope,project_path,organization_id
           ) VALUES($1,'Reporting Candidate target','url',
                    'https://reporting.example.test/login','in',$2,$3)"#,
    )
    .bind(target_id)
    .bind(project_path)
    .bind(scope.organization_id)
    .execute(db.pool())
    .await
    .expect("insert live Candidate target");

    let attempt_id = Uuid::new_v4();
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin isolated Candidate fixture");
    // Candidate authority/terminalization is covered by the attack-execution
    // integration suite.  This fixture deliberately isolates Reporting's
    // canonical-source projection while retaining the real live-target FK and
    // every frozen at-time field involved in deletion.
    sqlx::query("SET LOCAL session_replication_role = 'replica'")
        .execute(&mut *tx)
        .await
        .expect("isolate Candidate authority parents for Reporting fixture");
    sqlx::query(
        r#"INSERT INTO candidate_attempts(
               id,candidate_id,approval_id,operation_id,scope_snapshot_id,
               wave_run_id,wave_unit_id,organization_id,target_live_id,
               target_type_at_time,target_value_at_time,target_identity_hash,
               candidate_plan_hash,ordinal,status,result_json,result_hash,terminal_at
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,
               'url','https://reporting.example.test/login',
               'sha256:reporting-candidate-target','sha256:reporting-candidate-plan',
               0,'verified',$10,'sha256:reporting-candidate-result',NOW()
           )"#,
    )
    .bind(attempt_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(scope.operation_id)
    .bind(scope.scope_snapshot_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(scope.organization_id)
    .bind(target_id)
    .bind(json!({"disposition": "verified"}))
    .execute(&mut *tx)
    .await
    .expect("insert isolated terminal CandidateAttempt");
    sqlx::query("SET LOCAL session_replication_role = 'origin'")
        .execute(&mut *tx)
        .await
        .expect("restore Candidate fixture trigger authority");
    tx.commit()
        .await
        .expect("commit isolated terminal Candidate fixture");
    (attempt_id, target_id)
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).expect("serialize string"),
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
                        serde_json::to_string(key).expect("serialize key"),
                        canonical_json(&object[key])
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

fn sha256(value: &Value) -> String {
    Sha256::digest(canonical_json(value).as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

async fn insert_technique_outcome(
    db: &GolishDb,
    scope: FrozenScope,
    evidence_ids: &[i64],
) -> (i64, String) {
    let outcome_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO technique_outcomes(
               organization_id,run_id,asset,technique,outcome,source,query,
               result_count,confidence,evidence_ids,seq,collected_at
           ) VALUES($1,$2,'report.example','GOLISH-INTEL-DNS','found','fixture',
                    'fixture query',1,1.0,$3,1,NOW())
           RETURNING id"#,
    )
    .bind(scope.organization_id)
    .bind(scope.operation_id.to_string())
    .bind(evidence_ids)
    .fetch_one(db.pool())
    .await
    .expect("insert canonical technique outcome");
    let content: Value = sqlx::query_scalar(
        "SELECT to_jsonb(outcome.*) FROM technique_outcomes AS outcome WHERE id=$1",
    )
    .bind(outcome_id)
    .fetch_one(db.pool())
    .await
    .expect("load exact technique outcome body");
    (outcome_id, sha256(&content))
}

#[derive(sqlx::FromRow)]
struct OutcomeSetFixtureRow {
    organization_id: Uuid,
    run_id: String,
    asset: String,
    technique: String,
    outcome: String,
    observed_at: chrono::DateTime<chrono::Utc>,
    evidence_ids: Vec<i64>,
    content: Value,
}

async fn insert_technique_outcome_set(
    db: &GolishDb,
    scope: FrozenScope,
    evidence_id: i64,
) -> CanonicalFactRef {
    for asset_index in 0..36 {
        for technique_index in 0..10 {
            sqlx::query(
                r#"INSERT INTO technique_outcomes(
                       organization_id,run_id,asset,technique,outcome,source,query,
                       result_count,confidence,evidence_ids,seq,collected_at
                   ) VALUES($1,$2,$3,$4,'blocked','report-set-fixture',
                            'bounded fixture',0,1.0,$5,$6,NOW())"#,
            )
            .bind(scope.organization_id)
            .bind(scope.operation_id.to_string())
            .bind(format!("https://host-{asset_index:03}.example"))
            .bind(format!("GOLISH-VULN-{technique_index:02}"))
            .bind(vec![evidence_id])
            .bind(i64::from(asset_index * 10 + technique_index + 1))
            .execute(db.pool())
            .await
            .expect("insert aggregate technique outcome member");
        }
    }
    let rows = sqlx::query_as::<_, OutcomeSetFixtureRow>(
        r#"SELECT organization_id,run_id,asset,technique,outcome,
                  collected_at AS observed_at,evidence_ids,to_jsonb(outcome.*) AS content
             FROM technique_outcomes AS outcome
            WHERE organization_id=$1 AND run_id=$2
            ORDER BY asset,technique"#,
    )
    .bind(scope.organization_id)
    .bind(scope.operation_id.to_string())
    .fetch_all(db.pool())
    .await
    .expect("load aggregate technique outcome members");
    let members = rows
        .into_iter()
        .map(
            |row| golish_db::repo::canonical_fact_refs::TechniqueOutcomeSetMember {
                organization_id: row.organization_id,
                run_id: row.run_id,
                asset: row.asset,
                technique: row.technique,
                outcome: row.outcome,
                observed_at: row.observed_at,
                evidence_ids: row.evidence_ids,
                content: row.content,
            },
        )
        .collect::<Vec<_>>();
    let attestation = golish_db::repo::canonical_fact_refs::technique_outcome_set_attestation(
        "vuln_triage",
        scope.organization_id,
        &scope.operation_id.to_string(),
        &members,
    )
    .expect("attest aggregate technique outcome fixture");
    CanonicalFactRef {
        key: CanonicalFactKey::TechniqueOutcomeSet {
            organization_id: scope.organization_id,
            run_id: scope.operation_id.to_string(),
            stage: "vuln_triage".to_string(),
            terminal_cell_count: attestation.terminal_cell_count,
            outcome_set_sha256: attestation.outcome_set_sha256,
        },
        organization_id: scope.organization_id,
        observed_at: attestation.observed_at,
        content_sha256: attestation.content_sha256,
        evidence_ids: attestation.evidence_ids,
    }
}

async fn insert_final_handoff(
    db: &GolishDb,
    scope: FrozenScope,
    refs: Vec<CanonicalFactRef>,
    evidence_ids: Vec<i64>,
) -> Uuid {
    let stage_kind = if refs
        .iter()
        .any(|reference| matches!(&reference.key, CanonicalFactKey::TechniqueOutcomeSet { .. }))
    {
        "vuln_triage"
    } else {
        "enumeration"
    };
    let session_id: Uuid = sqlx::query_scalar("SELECT session_id FROM tasks WHERE id=$1")
        .bind(scope.operation_id)
        .fetch_one(db.pool())
        .await
        .expect("load operation session");
    let stage_execution_id = Uuid::new_v4();
    let stage_run_unit_id = Uuid::new_v4();
    let worker_run_id = Uuid::new_v4();
    let tool_call_id = Uuid::new_v4();
    let submission_id = Uuid::new_v4();
    let handoff_id = Uuid::new_v4();
    let lease_token = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status,completed_at) \
         VALUES($1,$2,$3,'completed',NOW())",
    )
    .bind(stage_execution_id)
    .bind(scope.operation_id)
    .bind(stage_kind)
    .execute(db.pool())
    .await
    .expect("insert final-seal stage run");
    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,
               organization_id,stage_kind,generation,specialist,status,
               started_at,terminal_at,pass_watermark
           ) VALUES($1,$2,$3,$4,$5,$6,0,'report-fixture','passed',
                    NOW()-INTERVAL '1 minute',NOW(),$7)"#,
    )
    .bind(stage_run_unit_id)
    .bind(scope.operation_id)
    .bind(stage_execution_id)
    .bind(scope.scope_snapshot_id)
    .bind(scope.organization_id)
    .bind(stage_kind)
    .bind(json!({"final_gate_passed": true, "deliverable_submission_id": submission_id}))
    .execute(db.pool())
    .await
    .expect("insert passed stage unit");
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,organization_id,
               worker_generation,specialist,work_item_kind,work_item_key,agent_path,
               status,lease_token,lease_owner,lease_acquired_at,lease_expires_at,
               heartbeat_at,attempt_epoch,terminal_at
           ) VALUES($1,$2,$3,$4,$5,0,'report-fixture','stage_unit',$6,$7,
                    'passed',$8,'report-fixture',NOW(),NOW()+INTERVAL '5 minutes',
                    NOW(),0,NOW())"#,
    )
    .bind(worker_run_id)
    .bind(scope.operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope.organization_id)
    .bind(format!("enumeration:{}", scope.organization_id))
    .bind("main>enumeration")
    .bind(lease_token)
    .execute(db.pool())
    .await
    .expect("insert passed source worker");
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,result,status,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token
           ) VALUES($1,$2,$3,$4,'primary','submit_stage_deliverable','{}','{}','finished',
                    $4,$5,$6,$7,$8,0,$9)"#,
    )
    .bind(tool_call_id)
    .bind(format!("report-fixture-{tool_call_id}"))
    .bind(session_id)
    .bind(scope.operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(worker_run_id)
    .bind(scope.organization_id)
    .bind(lease_token)
    .execute(db.pool())
    .await
    .expect("insert source tool call");
    sqlx::query(
        r#"INSERT INTO stage_deliverable_submissions(
               id,operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,tool_call_record_id,tool_request_id,stage_kind,
               attempt_epoch,lease_token,payload,payload_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,0,$10,$11,$12)"#,
    )
    .bind(submission_id)
    .bind(scope.operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(worker_run_id)
    .bind(scope.organization_id)
    .bind(tool_call_id)
    .bind(format!("report-fixture-{submission_id}"))
    .bind(stage_kind)
    .bind(lease_token)
    .bind(json!({"schema_version": 1}))
    .bind(format!("sha256:{submission_id}"))
    .execute(db.pool())
    .await
    .expect("insert source deliverable");
    let payload = serde_json::to_value(StageHandoffPayload {
        canonical_fact_refs: refs,
        typed_claims: Vec::new(),
        coverage_watermark: json!({}),
        evidence_ids: evidence_ids.clone(),
    })
    .expect("serialize final handoff payload");
    sqlx::query(
        r#"INSERT INTO stage_handoffs(
               id,operation_id,organization_id,scope_snapshot_id,from_stage_kind,
               stage_execution_id,source_stage_run_unit_id,deliverable_submission_id,
               scope_hash,payload,payload_sha256,evidence_ids,
               unit_gate_decision_hash,gate_passed_at
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,NOW())"#,
    )
    .bind(handoff_id)
    .bind(scope.operation_id)
    .bind(scope.organization_id)
    .bind(scope.scope_snapshot_id)
    .bind(stage_kind)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(submission_id)
    .bind("3".repeat(64))
    .bind(payload)
    .bind(format!("sha256:{handoff_id}"))
    .bind(evidence_ids)
    .bind(format!("sha256:{stage_execution_id}"))
    .execute(db.pool())
    .await
    .expect("insert final sealed handoff");
    handoff_id
}

struct BlockedCleanupTruth {
    obligation_id: Uuid,
    obligation_evidence_id: i64,
    decision_id: Uuid,
    decision_evidence_id: i64,
    principal_id: Uuid,
    reason: String,
    residual_risk: Value,
}

async fn insert_blocked_cleanup_truth(db: &GolishDb, scope: FrozenScope) -> BlockedCleanupTruth {
    let principal = operator_principals::current_local(db.pool())
        .await
        .expect("load trusted local principal");
    let action_evidence_id = insert_evidence(db, scope, "blocked action plan").await;
    let obligation_evidence_id = insert_evidence(db, scope, "blocked obligation creation").await;
    let decision_evidence_id = insert_evidence(db, scope, "blocked operator decision").await;
    let obligation_id = Uuid::new_v4();
    cleanup_obligations::record_action_and_obligation(
        db.pool(),
        &cleanup_obligations::RecordActionAndObligation {
            action_id: Uuid::new_v4(),
            obligation_id,
            operation_id: scope.operation_id,
            project_scope_id: scope.project_scope_id,
            scope_snapshot_id: scope.scope_snapshot_id,
            organization_id_at_time: scope.organization_id,
            principal_id: principal.id,
            capability_id: "post_exploit.report_fixture".to_string(),
            side_effect_class: "remote_state_mutation".to_string(),
            action_plan: json!({"kind": "report_fixture"}),
            action_plan_hash: "a".repeat(64),
            action_evidence: vec![(action_evidence_id, "plan".to_string())],
            affected_resource_snapshot: json!({"kind": "fixture", "id": "blocked"}),
            resource_identity_hash: "b".repeat(64),
            cleanup_strategy: json!({"kind": "manual_owner_action"}),
            proof_requirements: json!([{"kind": "owner_confirmation"}]),
            deadline: Utc::now() + Duration::hours(1),
            obligation_evidence: vec![(obligation_evidence_id, "source".to_string())],
        },
    )
    .await
    .expect("record blocked cleanup obligation");
    let decision_id = Uuid::new_v4();
    let reason = "target owner denied cleanup during the authorized window".to_string();
    let residual_risk = json!({"summary": "fixture remains", "severity": "medium"});
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin blocked terminal truth");
    sqlx::query(
        r#"INSERT INTO cleanup_blocked_decisions(
               id,obligation_id,operation_id,project_scope_id,scope_snapshot_id,
               organization_id_at_time,decided_by_principal_id,reason,residual_risk
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)"#,
    )
    .bind(decision_id)
    .bind(obligation_id)
    .bind(scope.operation_id)
    .bind(scope.project_scope_id)
    .bind(scope.scope_snapshot_id)
    .bind(scope.organization_id)
    .bind(principal.id)
    .bind(&reason)
    .bind(&residual_risk)
    .execute(&mut *tx)
    .await
    .expect("insert blocked cleanup decision");
    sqlx::query(
        r#"INSERT INTO cleanup_blocked_decision_evidence(
               blocked_decision_id,evidence_id,role
           ) VALUES($1,$2,'decision')"#,
    )
    .bind(decision_id)
    .bind(decision_evidence_id)
    .execute(&mut *tx)
    .await
    .expect("link exact blocked decision evidence");
    sqlx::query(
        r#"UPDATE cleanup_obligations
              SET status='blocked',residual_risk=$2,terminal_at=NOW()
            WHERE id=$1"#,
    )
    .bind(obligation_id)
    .bind(&residual_risk)
    .execute(&mut *tx)
    .await
    .expect("mark cleanup obligation blocked after retaining decision evidence");
    sqlx::query("SET CONSTRAINTS ALL IMMEDIATE")
        .execute(&mut *tx)
        .await
        .expect("validate exact blocked terminal truth");
    tx.commit().await.expect("commit blocked terminal truth");
    BlockedCleanupTruth {
        obligation_id,
        obligation_evidence_id,
        decision_id,
        decision_evidence_id,
        principal_id: principal.id,
        reason,
        residual_risk,
    }
}

fn technique_ref(
    scope: FrozenScope,
    content_sha256: String,
    evidence_ids: Vec<i64>,
) -> CanonicalFactRef {
    CanonicalFactRef {
        key: CanonicalFactKey::TechniqueOutcome {
            organization_id: scope.organization_id,
            run_id: scope.operation_id.to_string(),
            asset: "report.example".to_string(),
            technique: "GOLISH-INTEL-DNS".to_string(),
        },
        organization_id: scope.organization_id,
        observed_at: Utc::now(),
        content_sha256,
        evidence_ids,
    }
}

#[tokio::test]
#[serial]
async fn pg_truth_builds_cited_revision_and_new_source_rejects_finalize() {
    let (mut db, _data_dir) = fixture().await;
    let scope = frozen_scope(&db, "/fixture/reporting-authority").await;
    let (episode_id, evidence_id) = insert_episode(&db, scope, "first episode").await;
    let pool = Arc::new(db.pool().clone());
    let authority = reporting_project_authority(&db, scope).await;
    let builder = ReportReadModelBuilder::new(PgReportTruthPort::with_project_authority(
        pool.clone(),
        authority.clone(),
    ));
    let built = builder
        .build_and_validate(scope.operation_id)
        .await
        .expect("build validated canonical report");

    assert_eq!(built.model.source_snapshot.ordered_sources.len(), 2);
    assert!(built
        .model
        .source_snapshot
        .ordered_sources
        .iter()
        .any(|source| source.kind == golish_reporting_domain::ReportSourceKind::EvidenceAudit));
    assert_eq!(built.model.scope_snapshot_id, scope.scope_snapshot_id);
    assert_eq!(built.model.organization_sections.len(), 1);
    let claim = &built.model.organization_sections[0].section.claims[0];
    assert_eq!(claim.subject_ref, format!("stage_episode:{episode_id}"));
    let citation = built
        .model
        .citations
        .iter()
        .find(|citation| citation.claim_id == claim.claim_id)
        .expect("claim has canonical citation");
    assert_eq!(citation.evidence_audit_id, Some(evidence_id));

    let stored = load_report_bundle(&pool, scope.operation_id)
        .await
        .expect("load stored report")
        .expect("report exists");
    let revision = stored.current_revision.expect("current revision");
    assert_eq!(revision.validation_status, "validated");
    assert_eq!(revision.publication_status, "unpublished");
    let provider = GolishDbRepoProvider::new(pool.clone());
    let gate_truth = provider
        .reporting_gate_truth(scope.operation_id)
        .await
        .expect("load Reporting Gate truth")
        .expect("current Reporting truth exists");
    validate_reporting_gate_truth(&gate_truth).expect("current validated revision passes Gate");

    insert_episode(&db, scope, "second episode").await;
    let current = current_reportable_source_snapshot(&pool, scope.operation_id)
        .await
        .expect("reload complete source set");
    assert_ne!(
        current.source_set_hash,
        built.model.source_snapshot.source_set_hash
    );
    let stale_gate_truth = provider
        .reporting_gate_truth(scope.operation_id)
        .await
        .expect("reload stale Reporting Gate truth")
        .expect("historical current revision still exists");
    assert_eq!(
        validate_reporting_gate_truth(&stale_gate_truth)
            .expect_err("new canonical source must block Reporting Gate")
            .code,
        "REPORT_SOURCE_SNAPSHOT_STALE"
    );

    let principal = golish_db::repo::operator_principals::current_local(&pool)
        .await
        .expect("load trusted local principal");
    let publication = PgReportPublicationPort::with_project_authority(pool.clone(), authority);
    let error = publication
        .finalize_publication(FinalizePublication {
            operation_id: scope.operation_id,
            report_id: built.model.report_id,
            revision_id: built.model.revision_id,
            expected_row_version: built.expected_row_version,
            expected_source_snapshot: built.model.source_snapshot.clone(),
            principal_id: principal.id,
            artifacts: Vec::new(),
        })
        .await
        .expect_err("new canonical source must reject finalization");
    assert_eq!(error.code(), "report_source_snapshot_stale");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn candidate_attempt_report_source_survives_live_target_and_organization_deletion() {
    let project_path = "/fixture/reporting-candidate-live-target";
    let (mut db, _data_dir) = fixture().await;
    let scope = frozen_scope(&db, project_path).await;
    let (attempt_id, target_id) =
        insert_isolated_terminal_candidate_attempt(&db, scope, project_path).await;
    let pool = Arc::new(db.pool().clone());

    let before = current_reportable_source_snapshot(&pool, scope.operation_id)
        .await
        .expect("load CandidateAttempt source before live deletion");
    let source_before = before
        .ordered_sources
        .iter()
        .find(|source| {
            source.kind == ReportSourceKind::CandidateAttempt
                && source.id == CanonicalRowId::Uuid(attempt_id)
        })
        .cloned()
        .expect("CandidateAttempt is a canonical Reporting source");
    let canonical_before: (Option<Uuid>, String, String, String, i64) = sqlx::query_as(
        r#"SELECT target_live_id,target_type_at_time,target_value_at_time,
                  target_identity_hash,row_version
             FROM candidate_attempts WHERE id=$1"#,
    )
    .bind(attempt_id)
    .fetch_one(db.pool())
    .await
    .expect("load frozen CandidateAttempt target identity before deletion");
    assert_eq!(canonical_before.0, Some(target_id));

    let deleted_target = sqlx::query("DELETE FROM targets WHERE id=$1")
        .bind(target_id)
        .execute(db.pool())
        .await
        .expect("delete live target and null only its non-canonical pointer");
    assert_eq!(deleted_target.rows_affected(), 1);
    let deleted_organization = sqlx::query("DELETE FROM organizations WHERE id=$1")
        .bind(scope.organization_id)
        .execute(db.pool())
        .await
        .expect("delete the retired live organization after its target");
    assert_eq!(deleted_organization.rows_affected(), 1);

    let after = current_reportable_source_snapshot(&pool, scope.operation_id)
        .await
        .expect("load CandidateAttempt source after live deletion");
    let source_after = after
        .ordered_sources
        .iter()
        .find(|source| {
            source.kind == ReportSourceKind::CandidateAttempt
                && source.id == CanonicalRowId::Uuid(attempt_id)
        })
        .cloned()
        .expect("retained CandidateAttempt remains a canonical Reporting source");
    assert_eq!(
        source_after.row_version, source_before.row_version,
        "nulling target_live_id must not advance the canonical source version"
    );
    assert_eq!(
        source_after.content_hash, source_before.content_hash,
        "target_live_id is a nullable live pointer, not canonical report content"
    );
    assert_eq!(
        after.source_set_hash, before.source_set_hash,
        "live target and organization deletion must not stale the report source set"
    );

    let canonical_after: (Option<Uuid>, String, String, String, i64) = sqlx::query_as(
        r#"SELECT target_live_id,target_type_at_time,target_value_at_time,
                  target_identity_hash,row_version
             FROM candidate_attempts WHERE id=$1"#,
    )
    .bind(attempt_id)
    .fetch_one(db.pool())
    .await
    .expect("load retained CandidateAttempt target identity after deletion");
    assert_eq!(canonical_after.0, None);
    assert_eq!(canonical_after.1, canonical_before.1);
    assert_eq!(canonical_after.2, canonical_before.2);
    assert_eq!(canonical_after.3, canonical_before.3);
    assert_eq!(canonical_after.4, canonical_before.4);

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn authorized_build_rejects_project_path_rebind_before_persistence() {
    let (mut db, _data_dir) = fixture().await;
    let original_path = "/fixture/reporting-authorized-build";
    let scope = frozen_scope(&db, original_path).await;
    insert_episode(&db, scope, "authorized build episode").await;
    let authority = reporting_project_authority(&db, scope).await;
    let pool = Arc::new(db.pool().clone());
    let truth = PgReportTruthPort::with_project_authority(pool, authority);
    let built = truth
        .build_repeatable_read_snapshot(scope.operation_id)
        .await
        .expect("build one authorized repeatable-read snapshot");
    let validation_result =
        golish_reporting_domain::validate_report(&built.model, &built.validation_truth)
            .expect("validate authorized snapshot before persistence");
    sqlx::query(
        r#"UPDATE project_scopes
              SET canonical_project_path=$2,path_sha256=$3,
                  row_version=row_version+1,updated_at=NOW()
            WHERE project_scope_id=$1"#,
    )
    .bind(scope.project_scope_id)
    .bind("/fixture/reporting-authorized-build-renamed")
    .bind("9".repeat(64))
    .execute(db.pool())
    .await
    .expect("rename project after Reporting authorization");

    let error = truth
        .persist_validated_revision(&built, &validation_result)
        .await
        .expect_err("stale project-path authority must reject the build persistence transaction");
    assert_eq!(error.code(), "report_source_snapshot_stale");
    let report_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM reports WHERE operation_id=$1")
            .bind(scope.operation_id)
            .fetch_one(db.pool())
            .await
            .expect("count reports after stale authorized build");
    assert_eq!(report_count, 0);

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn production_stage_entry_rejects_retired_and_prebound_project_authority() {
    let (mut db, _data_dir) = fixture().await;
    let retired = frozen_scope(&db, "/fixture/reporting-stage-entry-retired").await;
    insert_episode(&db, retired, "retired stage-entry episode").await;
    sqlx::query(
        r#"UPDATE project_scopes
              SET retired_at=NOW(),row_version=row_version+1,updated_at=NOW()
            WHERE project_scope_id=$1"#,
    )
    .bind(retired.project_scope_id)
    .execute(db.pool())
    .await
    .expect("retire project before Reporting stage entry");

    let rebound = frozen_scope(&db, "/fixture/reporting-stage-entry-rebound").await;
    insert_episode(&db, rebound, "rebound stage-entry episode").await;
    sqlx::query(
        r#"UPDATE project_scopes
              SET canonical_project_path=$2,path_sha256=$3,
                  row_version=row_version+1,updated_at=NOW()
            WHERE project_scope_id=$1"#,
    )
    .bind(rebound.project_scope_id)
    .bind("/fixture/reporting-stage-entry-rebound-new")
    .bind("9".repeat(64))
    .execute(db.pool())
    .await
    .expect("rebind project before Reporting stage entry");

    let provider = GolishDbRepoProvider::new(Arc::new(db.pool().clone()));
    for (label, scope) in [("retired", retired), ("rebound", rebound)] {
        provider
            .reporting_build_validated_revision(scope.operation_id)
            .await
            .expect_err(label);
        let report_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM reports WHERE operation_id=$1")
                .bind(scope.operation_id)
                .fetch_one(db.pool())
                .await
                .expect("count reports after rejected production stage entry");
        assert_eq!(report_count, 0, "{label} authority persisted a report");
    }

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn authorized_finalize_rejects_project_retirement_before_publish() {
    let (mut db, _data_dir) = fixture().await;
    let scope = frozen_scope(&db, "/fixture/reporting-authorized-finalize").await;
    insert_episode(&db, scope, "authorized finalize episode").await;
    let authority = reporting_project_authority(&db, scope).await;
    let pool = Arc::new(db.pool().clone());
    let built = ReportReadModelBuilder::new(PgReportTruthPort::new(pool.clone()))
        .build_and_validate(scope.operation_id)
        .await
        .expect("build report before project retirement");
    let principal = operator_principals::current_local(db.pool())
        .await
        .expect("load publication principal");
    sqlx::query(
        r#"UPDATE project_scopes
              SET retired_at=NOW(),row_version=row_version+1,updated_at=NOW()
            WHERE project_scope_id=$1"#,
    )
    .bind(scope.project_scope_id)
    .execute(db.pool())
    .await
    .expect("retire project after Reporting authorization");

    let error = PgReportPublicationPort::with_project_authority(pool, authority)
        .finalize_publication(FinalizePublication {
            operation_id: scope.operation_id,
            report_id: built.model.report_id,
            revision_id: built.model.revision_id,
            expected_row_version: built.expected_row_version,
            expected_source_snapshot: built.model.source_snapshot,
            principal_id: principal.id,
            artifacts: Vec::new(),
        })
        .await
        .expect_err("retired project authority must reject publication");
    assert_eq!(error.code(), "report_source_snapshot_stale");
    let publication_status: String =
        sqlx::query_scalar("SELECT publication_status FROM report_revisions WHERE revision_id=$1")
            .bind(built.model.revision_id)
            .fetch_one(db.pool())
            .await
            .expect("load revision after rejected stale-authority publish");
    assert_eq!(publication_status, "unpublished");
    let artifact_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM report_revision_artifacts WHERE revision_id=$1")
            .bind(built.model.revision_id)
            .fetch_one(db.pool())
            .await
            .expect("count artifacts after rejected stale-authority publish");
    assert_eq!(artifact_count, 0);

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn sibling_org_evidence_is_rejected_before_a_validated_revision_is_persisted() {
    let (mut db, _data_dir) = fixture().await;
    let scope = frozen_scope(&db, "/fixture/reporting-sibling-evidence").await;
    let (_, evidence_id) = insert_episode(&db, scope, "sibling evidence").await;
    let sibling_organization_id = Uuid::new_v4();
    sqlx::query("UPDATE audit_log SET detail=$2 WHERE id=$1")
        .bind(evidence_id)
        .bind(serde_json::json!({"organization_id": sibling_organization_id}))
        .execute(db.pool())
        .await
        .expect("move evidence metadata to sibling organization");

    let pool = Arc::new(db.pool().clone());
    let error = ReportReadModelBuilder::new(PgReportTruthPort::new(pool.clone()))
        .build_and_validate(scope.operation_id)
        .await
        .expect_err("sibling organization evidence must fail before persistence");
    assert_eq!(error.code(), "report_validation_failed");
    let report_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM reports WHERE operation_id=$1")
            .bind(scope.operation_id)
            .fetch_one(db.pool())
            .await
            .expect("count reports after rejected build");
    assert_eq!(report_count, 0, "rejected build must not leave a report");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn foreign_run_or_non_evidence_role_is_rejected_before_persistence() {
    let (mut db, _data_dir) = fixture().await;
    let scope = frozen_scope(&db, "/fixture/reporting-foreign-evidence").await;
    let (_, evidence_id) = insert_episode(&db, scope, "foreign evidence").await;
    sqlx::query("UPDATE audit_log SET run_id=$2,audit_role='action' WHERE id=$1")
        .bind(evidence_id)
        .bind(Uuid::new_v4())
        .execute(db.pool())
        .await
        .expect("move evidence authority to another run and role");

    let pool = Arc::new(db.pool().clone());
    let error = ReportReadModelBuilder::new(PgReportTruthPort::new(pool))
        .build_and_validate(scope.operation_id)
        .await
        .expect_err("foreign non-evidence audit row must fail before persistence");
    assert_eq!(error.code(), "report_validation_failed");
    let report_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM reports WHERE operation_id=$1")
            .bind(scope.operation_id)
            .fetch_one(db.pool())
            .await
            .expect("count reports after rejected evidence authority");
    assert_eq!(report_count, 0, "rejected build must not leave a report");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn evidence_body_or_organization_drift_rejects_finalize_as_a_stale_source() {
    let (mut db, _data_dir) = fixture().await;
    let scope = frozen_scope(&db, "/fixture/reporting-evidence-drift").await;
    let (_, evidence_id) = insert_episode(&db, scope, "drifting evidence").await;
    let pool = Arc::new(db.pool().clone());
    let built = ReportReadModelBuilder::new(PgReportTruthPort::new(pool.clone()))
        .build_and_validate(scope.operation_id)
        .await
        .expect("build validated report");

    sqlx::query("UPDATE audit_log SET details=$2,detail=$3 WHERE id=$1")
        .bind(evidence_id)
        .bind("tampered evidence body")
        .bind(serde_json::json!({"organization_id": Uuid::new_v4()}))
        .execute(db.pool())
        .await
        .expect("drift evidence after validation");

    let current = current_reportable_source_snapshot(&pool, scope.operation_id)
        .await
        .expect("reload complete source set");
    assert_ne!(
        current.source_set_hash, built.model.source_snapshot.source_set_hash,
        "evidence body and ownership metadata are canonical source material"
    );
    let principal = golish_db::repo::operator_principals::current_local(&pool)
        .await
        .expect("load trusted local principal");
    let error = PgReportPublicationPort::new(pool.clone())
        .finalize_publication(FinalizePublication {
            operation_id: scope.operation_id,
            report_id: built.model.report_id,
            revision_id: built.model.revision_id,
            expected_row_version: built.expected_row_version,
            expected_source_snapshot: built.model.source_snapshot,
            principal_id: principal.id,
            artifacts: Vec::new(),
        })
        .await
        .expect_err("evidence drift must reject finalization");
    assert_eq!(error.code(), "report_source_snapshot_stale");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn finalize_revalidates_stored_citations_and_validation_attestation() {
    let (mut db, _data_dir) = fixture().await;
    let scope = frozen_scope(&db, "/fixture/reporting-finalize-integrity").await;
    insert_episode(&db, scope, "finalize integrity").await;
    let pool = Arc::new(db.pool().clone());
    let builder = ReportReadModelBuilder::new(PgReportTruthPort::new(pool.clone()));
    let built = builder
        .build_and_validate(scope.operation_id)
        .await
        .expect("build validated report");
    let principal = golish_db::repo::operator_principals::current_local(&pool)
        .await
        .expect("load trusted local principal");

    let direct_mutation_error = sqlx::query(
        "UPDATE report_claim_citations SET organization_id_at_time=$2 WHERE revision_id=$1",
    )
    .bind(built.model.revision_id)
    .bind(Uuid::new_v4())
    .execute(db.pool())
    .await
    .expect_err("validated citation ownership is frozen in the database");
    assert!(direct_mutation_error
        .to_string()
        .contains("FINAL_HISTORY_IMMUTABLE"));
    let mut legacy_corruption = db
        .pool()
        .begin()
        .await
        .expect("begin legacy corruption fixture");
    sqlx::query(
        "ALTER TABLE report_claim_citations DISABLE TRIGGER report_claim_citations_immutable",
    )
    .execute(&mut *legacy_corruption)
    .await
    .expect("disable validated citation guard inside legacy fixture");
    sqlx::query(
        "UPDATE report_claim_citations SET organization_id_at_time=$2 WHERE revision_id=$1",
    )
    .bind(built.model.revision_id)
    .bind(Uuid::new_v4())
    .execute(&mut *legacy_corruption)
    .await
    .expect("inject legacy citation ownership corruption");
    sqlx::query(
        "ALTER TABLE report_claim_citations ENABLE TRIGGER report_claim_citations_immutable",
    )
    .execute(&mut *legacy_corruption)
    .await
    .expect("restore validated citation guard");
    legacy_corruption
        .commit()
        .await
        .expect("commit legacy citation corruption fixture");
    let error = PgReportPublicationPort::new(pool.clone())
        .finalize_publication(FinalizePublication {
            operation_id: scope.operation_id,
            report_id: built.model.report_id,
            revision_id: built.model.revision_id,
            expected_row_version: built.expected_row_version,
            expected_source_snapshot: built.model.source_snapshot.clone(),
            principal_id: principal.id,
            artifacts: Vec::new(),
        })
        .await
        .expect_err("tampered citation must reject finalization");
    assert_eq!(error.code(), "report_revision_not_validated");

    let rebuilt = builder
        .build_and_validate(scope.operation_id)
        .await
        .expect("build a clean successor revision");
    let attestation_mutation_error = sqlx::query(
        r#"UPDATE report_revisions
              SET validation_result=jsonb_set(validation_result,'{claim_count}','999'::jsonb)
            WHERE revision_id=$1"#,
    )
    .bind(rebuilt.model.revision_id)
    .execute(db.pool())
    .await
    .expect_err("validated attestation is frozen before finalization can observe tampering");
    assert!(attestation_mutation_error
        .to_string()
        .contains("REPORT_VALIDATED_REVISION_IMMUTABLE"));

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn blocked_residual_is_projected_only_from_the_retained_operator_decision() {
    let (mut db, _data_dir) = fixture().await;
    let scope = frozen_scope(&db, "/fixture/reporting-blocked-decision").await;
    let blocked = insert_blocked_cleanup_truth(&db, scope).await;
    let pool = Arc::new(db.pool().clone());
    let built = ReportReadModelBuilder::new(PgReportTruthPort::new(pool.clone()))
        .build_and_validate(scope.operation_id)
        .await
        .expect("build report from exact blocked-decision authority");

    assert!(built
        .model
        .source_snapshot
        .ordered_sources
        .iter()
        .any(|source| {
            source.kind.as_str() == "cleanup_blocked_decision"
                && source.id
                    == golish_memory_domain::source_ref::CanonicalRowId::Uuid(blocked.decision_id)
        }));
    let claim = built
        .model
        .organization_sections
        .iter()
        .flat_map(|section| &section.section.claims)
        .find(|claim| claim.subject_ref == format!("cleanup_obligation:{}", blocked.obligation_id))
        .expect("blocked residual claim");
    assert_eq!(
        claim.value,
        json!({
            "status": "blocked",
            "decidedByPrincipalId": blocked.principal_id,
            "reason": blocked.reason,
            "residualRisk": blocked.residual_risk,
        })
    );
    let citations = built
        .model
        .citations
        .iter()
        .filter(|citation| citation.claim_id == claim.claim_id)
        .collect::<Vec<_>>();
    assert_eq!(citations.len(), 1);
    assert_eq!(
        citations[0].evidence_audit_id,
        Some(blocked.decision_evidence_id)
    );
    assert_eq!(
        citations[0].source.kind.as_str(),
        "cleanup_blocked_decision"
    );
    assert_ne!(
        citations[0].evidence_audit_id,
        Some(blocked.obligation_evidence_id),
        "obligation-creation evidence cannot authorize a blocked residual"
    );

    let obligation_source: (String, String, i64, Vec<u8>) = sqlx::query_as(
        r#"SELECT source_id_kind,source_id_value,source_row_version,content_hash
             FROM report_source_manifest
            WHERE revision_id=$1 AND source_kind='cleanup_obligation'
              AND source_id_value=$2"#,
    )
    .bind(built.model.revision_id)
    .bind(blocked.obligation_id.to_string())
    .fetch_one(db.pool())
    .await
    .expect("load frozen obligation source for tamper regression");
    let direct_mutation_error = sqlx::query(
        r#"UPDATE report_claim_citations
              SET source_kind='cleanup_obligation',source_id_kind=$2,
                  source_id_value=$3,source_row_version=$4,source_hash=$5,
                  evidence_audit_id=$6
            WHERE citation_id=$1"#,
    )
    .bind(citations[0].citation_id)
    .bind(&obligation_source.0)
    .bind(&obligation_source.1)
    .bind(obligation_source.2)
    .bind(&obligation_source.3)
    .bind(blocked.obligation_evidence_id)
    .execute(db.pool())
    .await
    .expect_err("validated blocked-decision citation is frozen in the database");
    assert!(direct_mutation_error
        .to_string()
        .contains("FINAL_HISTORY_IMMUTABLE"));
    let mut legacy_corruption = db
        .pool()
        .begin()
        .await
        .expect("begin legacy corruption fixture");
    sqlx::query(
        "ALTER TABLE report_claim_citations DISABLE TRIGGER report_claim_citations_immutable",
    )
    .execute(&mut *legacy_corruption)
    .await
    .expect("disable validated citation guard inside legacy fixture");
    sqlx::query(
        r#"UPDATE report_claim_citations
              SET source_kind='cleanup_obligation',source_id_kind=$2,
                  source_id_value=$3,source_row_version=$4,source_hash=$5,
                  evidence_audit_id=$6
            WHERE citation_id=$1"#,
    )
    .bind(citations[0].citation_id)
    .bind(&obligation_source.0)
    .bind(&obligation_source.1)
    .bind(obligation_source.2)
    .bind(&obligation_source.3)
    .bind(blocked.obligation_evidence_id)
    .execute(&mut *legacy_corruption)
    .await
    .expect("inject legacy blocked-decision citation corruption");
    sqlx::query(
        "ALTER TABLE report_claim_citations ENABLE TRIGGER report_claim_citations_immutable",
    )
    .execute(&mut *legacy_corruption)
    .await
    .expect("restore validated citation guard");
    legacy_corruption
        .commit()
        .await
        .expect("commit legacy blocked-decision corruption fixture");
    let error = PgReportPublicationPort::new(pool)
        .finalize_publication(FinalizePublication {
            operation_id: scope.operation_id,
            report_id: built.model.report_id,
            revision_id: built.model.revision_id,
            expected_row_version: built.expected_row_version,
            expected_source_snapshot: built.model.source_snapshot,
            principal_id: blocked.principal_id,
            artifacts: Vec::new(),
        })
        .await
        .expect_err("finalize must revalidate exact blocked-decision citations");
    assert_eq!(error.code(), "report_revision_not_validated");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn aggregate_technique_outcome_set_expands_to_all_report_sources() {
    let (mut db, _data_dir) = fixture().await;
    let scope = frozen_scope(&db, "/fixture/reporting-technique-outcome-set").await;
    let evidence_id = insert_evidence(&db, scope, "aggregate outcome set").await;
    let set_ref = insert_technique_outcome_set(&db, scope, evidence_id).await;
    let handoff_id = insert_final_handoff(&db, scope, vec![set_ref], vec![evidence_id]).await;

    let snapshot = current_reportable_source_snapshot(db.pool(), scope.operation_id)
        .await
        .expect("aggregate set expands to exact outcome rows");
    assert_eq!(
        snapshot
            .ordered_sources
            .iter()
            .filter(|source| source.kind == ReportSourceKind::TechniqueOutcome)
            .count(),
        360
    );
    assert!(snapshot.ordered_sources.iter().any(|source| {
        source.kind == ReportSourceKind::StageHandoff
            && source.id == CanonicalRowId::Uuid(handoff_id)
    }));

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn technique_outcome_authority_rejects_missing_duplicate_and_drifted_sealed_refs() {
    let (mut db, _data_dir) = fixture().await;

    let missing_scope = frozen_scope(&db, "/fixture/reporting-technique-missing").await;
    let missing_evidence = insert_evidence(&db, missing_scope, "missing outcome ref").await;
    insert_final_handoff(
        &db,
        missing_scope,
        vec![technique_ref(
            missing_scope,
            "0".repeat(64),
            vec![missing_evidence],
        )],
        vec![missing_evidence],
    )
    .await;
    let missing = current_reportable_source_snapshot(db.pool(), missing_scope.operation_id).await;
    assert!(missing
        .expect_err("a sealed ref without its canonical row must fail closed")
        .to_string()
        .contains("report_technique_handoff_row_missing"));

    let duplicate_scope = frozen_scope(&db, "/fixture/reporting-technique-duplicate").await;
    let duplicate_evidence = insert_evidence(&db, duplicate_scope, "duplicate outcome ref").await;
    let (_, duplicate_hash) =
        insert_technique_outcome(&db, duplicate_scope, &[duplicate_evidence]).await;
    insert_final_handoff(
        &db,
        duplicate_scope,
        vec![
            technique_ref(duplicate_scope, duplicate_hash, vec![duplicate_evidence]),
            technique_ref(duplicate_scope, "f".repeat(64), vec![duplicate_evidence]),
        ],
        vec![duplicate_evidence],
    )
    .await;
    let duplicate =
        current_reportable_source_snapshot(db.pool(), duplicate_scope.operation_id).await;
    assert!(duplicate
        .expect_err("duplicate sealed refs must fail the bijection")
        .to_string()
        .contains("report_technique_handoff_ref_duplicate"));

    let drift_scope = frozen_scope(&db, "/fixture/reporting-technique-drift").await;
    let drift_evidence = insert_evidence(&db, drift_scope, "drifted outcome ref").await;
    insert_technique_outcome(&db, drift_scope, &[drift_evidence]).await;
    insert_final_handoff(
        &db,
        drift_scope,
        vec![technique_ref(
            drift_scope,
            "e".repeat(64),
            vec![drift_evidence],
        )],
        vec![drift_evidence],
    )
    .await;
    let drift = current_reportable_source_snapshot(db.pool(), drift_scope.operation_id).await;
    assert!(drift
        .expect_err("sealed ref hash drift must fail closed")
        .to_string()
        .contains("report_technique_outcome_source_changed"));

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn exact_technique_ref_is_frozen_and_cannot_be_invalidated_after_validation() {
    let (mut db, _data_dir) = fixture().await;
    let scope = frozen_scope(&db, "/fixture/reporting-technique-retention").await;
    let evidence_id = insert_evidence(&db, scope, "exact outcome ref").await;
    let (_, content_hash) = insert_technique_outcome(&db, scope, &[evidence_id]).await;
    let handoff_id = insert_final_handoff(
        &db,
        scope,
        vec![technique_ref(scope, content_hash, vec![evidence_id])],
        vec![evidence_id],
    )
    .await;
    let built = ReportReadModelBuilder::new(PgReportTruthPort::new(Arc::new(db.pool().clone())))
        .build_and_validate(scope.operation_id)
        .await
        .expect("build exact sealed-ref report");
    assert!(built
        .model
        .source_snapshot
        .ordered_sources
        .iter()
        .any(|source| {
            source.kind.as_str() == "stage_handoff"
                && source.id == golish_memory_domain::source_ref::CanonicalRowId::Uuid(handoff_id)
        }));
    let technique_claim = built
        .model
        .organization_sections
        .iter()
        .flat_map(|section| &section.section.claims)
        .find(|claim| {
            claim.claim_kind == golish_reporting_domain::ReportClaimKind::TechniqueOutcome
        })
        .expect("sealed exact row projects one claim");
    let technique_citation = built
        .model
        .citations
        .iter()
        .find(|citation| citation.claim_id == technique_claim.claim_id)
        .expect("technique claim cites its canonical row");
    let handoff_source: (String, String, i64, Vec<u8>) = sqlx::query_as(
        r#"SELECT source_id_kind,source_id_value,source_row_version,content_hash
             FROM report_source_manifest
            WHERE revision_id=$1 AND source_kind='stage_handoff'
              AND source_id_value=$2"#,
    )
    .bind(built.model.revision_id)
    .bind(handoff_id.to_string())
    .fetch_one(db.pool())
    .await
    .expect("load retained handoff source for tamper regression");
    let direct_mutation_error = sqlx::query(
        r#"UPDATE report_claim_citations
              SET source_kind='stage_handoff',source_id_kind=$2,
                  source_id_value=$3,source_row_version=$4,source_hash=$5
            WHERE citation_id=$1"#,
    )
    .bind(technique_citation.citation_id)
    .bind(&handoff_source.0)
    .bind(&handoff_source.1)
    .bind(handoff_source.2)
    .bind(&handoff_source.3)
    .execute(db.pool())
    .await
    .expect_err("validated technique citation is frozen in the database");
    assert!(direct_mutation_error
        .to_string()
        .contains("FINAL_HISTORY_IMMUTABLE"));
    let mut legacy_corruption = db
        .pool()
        .begin()
        .await
        .expect("begin legacy corruption fixture");
    sqlx::query(
        "ALTER TABLE report_claim_citations DISABLE TRIGGER report_claim_citations_immutable",
    )
    .execute(&mut *legacy_corruption)
    .await
    .expect("disable validated citation guard inside legacy fixture");
    sqlx::query(
        r#"UPDATE report_claim_citations
              SET source_kind='stage_handoff',source_id_kind=$2,
                  source_id_value=$3,source_row_version=$4,source_hash=$5
            WHERE citation_id=$1"#,
    )
    .bind(technique_citation.citation_id)
    .bind(&handoff_source.0)
    .bind(&handoff_source.1)
    .bind(handoff_source.2)
    .bind(&handoff_source.3)
    .execute(&mut *legacy_corruption)
    .await
    .expect("inject legacy technique citation corruption");
    sqlx::query(
        "ALTER TABLE report_claim_citations ENABLE TRIGGER report_claim_citations_immutable",
    )
    .execute(&mut *legacy_corruption)
    .await
    .expect("restore validated citation guard");
    legacy_corruption
        .commit()
        .await
        .expect("commit legacy technique corruption fixture");
    let principal = operator_principals::current_local(db.pool())
        .await
        .expect("load publication principal");
    let error = PgReportPublicationPort::new(Arc::new(db.pool().clone()))
        .finalize_publication(FinalizePublication {
            operation_id: scope.operation_id,
            report_id: built.model.report_id,
            revision_id: built.model.revision_id,
            expected_row_version: built.expected_row_version,
            expected_source_snapshot: built.model.source_snapshot.clone(),
            principal_id: principal.id,
            artifacts: Vec::new(),
        })
        .await
        .expect_err("finalize must revalidate exact technique-row citations");
    assert_eq!(error.code(), "report_revision_not_validated");

    let invalidation = sqlx::query("UPDATE stage_handoffs SET invalidated_at=NOW() WHERE id=$1")
        .bind(handoff_id)
        .execute(db.pool())
        .await;
    assert!(invalidation
        .expect_err("a handoff retained by validated Reporting history is immutable")
        .to_string()
        .contains("REPORT_SEALED_REF_RETAINED"));

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn gate_repeatable_read_snapshot_cannot_synthesize_a_pass_that_never_existed() {
    let (mut db, _data_dir) = fixture().await;
    let scope = frozen_scope(&db, "/fixture/reporting-gate-rr").await;
    let (_, evidence_id) = insert_episode(&db, scope, "gate rr evidence").await;
    let pool = Arc::new(db.pool().clone());
    let builder = ReportReadModelBuilder::new(PgReportTruthPort::new(pool.clone()));
    let built = builder
        .build_and_validate(scope.operation_id)
        .await
        .expect("build validated report before concurrency switch");
    let invalid_successor = builder
        .build_and_validate(scope.operation_id)
        .await
        .expect("build successor used for the legacy-invalid state");
    let direct_attestation_error = sqlx::query(
        r#"UPDATE report_revisions
              SET validation_result=jsonb_set(validation_result,'{claim_count}','999'::jsonb)
            WHERE revision_id=$1"#,
    )
    .bind(invalid_successor.model.revision_id)
    .execute(db.pool())
    .await
    .expect_err("validated successor attestation is immutable");
    assert!(direct_attestation_error
        .to_string()
        .contains("REPORT_VALIDATED_REVISION_IMMUTABLE"));
    let mut legacy_invalid = db
        .pool()
        .begin()
        .await
        .expect("begin legacy invalid-attestation fixture");
    sqlx::query("ALTER TABLE report_revisions DISABLE TRIGGER report_revisions_guard")
        .execute(&mut *legacy_invalid)
        .await
        .expect("disable revision guard for the legacy fixture");
    sqlx::query(
        r#"UPDATE report_revisions
              SET validation_result=jsonb_set(validation_result,'{claim_count}','999'::jsonb)
            WHERE revision_id=$1"#,
    )
    .bind(invalid_successor.model.revision_id)
    .execute(&mut *legacy_invalid)
    .await
    .expect("inject a legacy invalid successor attestation");
    sqlx::query("ALTER TABLE report_revisions ENABLE TRIGGER report_revisions_guard")
        .execute(&mut *legacy_invalid)
        .await
        .expect("restore revision guard after the legacy fixture");
    legacy_invalid
        .commit()
        .await
        .expect("commit legacy invalid successor");
    sqlx::query("UPDATE reports SET current_revision_id=$2,updated_at=NOW() WHERE report_id=$1")
        .bind(built.model.report_id)
        .bind(built.model.revision_id)
        .execute(db.pool())
        .await
        .expect("restore original revision as state-B current");
    let original_details: String = sqlx::query_scalar("SELECT details FROM audit_log WHERE id=$1")
        .bind(evidence_id)
        .fetch_one(db.pool())
        .await
        .expect("load original evidence body");

    // Visible state B: attestation is valid, but its canonical evidence source
    // is stale. This state cannot pass Reporting Gate.
    sqlx::query("UPDATE audit_log SET details='state-b-stale-evidence' WHERE id=$1")
        .bind(evidence_id)
        .execute(db.pool())
        .await
        .expect("enter stale-source state B");

    let reached_barrier = Arc::new(Notify::new());
    let release_barrier = Arc::new(Notify::new());
    let gate_pool = pool.clone();
    let gate_reached = reached_barrier.clone();
    let gate_release = release_barrier.clone();
    let gate_task = tokio::spawn(async move {
        load_reporting_gate_truth_with_barrier(&gate_pool, scope.operation_id, move || {
            let reached = gate_reached.clone();
            let release = gate_release.clone();
            async move {
                reached.notify_one();
                release.notified().await;
            }
        })
        .await
    });
    reached_barrier.notified().await;

    // Atomically switch to visible state A: canonical sources are exact again,
    // but validation attestation is invalid. There is no committed instant at
    // which source exactness and the old valid attestation coexist.
    let mut switch = db.pool().begin().await.expect("begin state switch");
    sqlx::query("UPDATE audit_log SET details=$2 WHERE id=$1")
        .bind(evidence_id)
        .bind(original_details)
        .execute(&mut *switch)
        .await
        .expect("restore exact source in state A");
    sqlx::query("UPDATE reports SET current_revision_id=$2,updated_at=NOW() WHERE report_id=$1")
        .bind(built.model.report_id)
        .bind(invalid_successor.model.revision_id)
        .execute(&mut *switch)
        .await
        .expect("switch to the retained legacy-invalid successor in state A");
    switch.commit().await.expect("commit atomic state switch");
    release_barrier.notify_one();

    let barrier_truth = gate_task
        .await
        .expect("join barrier Gate read")
        .expect("load barrier Gate truth")
        .expect("report truth exists");
    assert_eq!(
        validate_reporting_gate_truth(&barrier_truth)
            .expect_err("one Gate read cannot combine state-B bundle with state-A sources")
            .code,
        "REPORT_SOURCE_SNAPSHOT_STALE"
    );

    let current_truth = GolishDbRepoProvider::new(pool)
        .reporting_gate_truth(scope.operation_id)
        .await
        .expect("load current state-A Gate truth")
        .expect("current report exists");
    assert!(current_truth.source_snapshot_exact);
    assert!(!current_truth.validation_attestation_valid);
    assert_eq!(
        validate_reporting_gate_truth(&current_truth)
            .expect_err("state A also never passes")
            .code,
        "REPORT_VALIDATION_ATTESTATION_INVALID"
    );

    db.stop().await;
}
