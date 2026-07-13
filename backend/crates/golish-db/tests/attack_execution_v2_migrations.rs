use golish_core::AttackExecutionContract;
use golish_db::repo::attack_candidate_approvals::{
    claim_candidate_review_resume, list_candidate_reviews, mark_candidate_review_resumed,
    reap_stale_candidate_review_dispatches, review_wave_candidates, CandidateReviewDecision,
    ReviewCandidateBatch,
};
use golish_db::repo::attack_candidates::{
    accept_gate_passed_candidate_batch, canonical_execution_plan_hash, AcceptCandidateBatch,
    AcceptedCandidateDraft, NoCandidateDecision,
};
use golish_db::repo::candidate_attempts::{
    claim_next_candidate_attempt, heartbeat_candidate_execution, record_attempt_submission,
    release_candidate_execution, AttemptEvidenceLink, CandidateClaimQuery,
    CandidateExecutionHeartbeat, CandidateExecutionRelease, RecordAttemptSubmission,
};
use golish_db::repo::finding_lineage::{
    terminalize_candidate_attempt, terminalize_verified_finding, TerminalizeCandidateAttempt,
    TerminalizeVerifiedFinding,
};
use golish_db::repo::{
    attack_execution_rollout, canonical_fact_refs, operation_state, runtime_memory_tx,
    stage_run_units,
};
use golish_db::{DbConfig, GolishDb};
use serial_test::serial;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use golish_pentest_domain::FindingWriteContext;

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

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read local postgres port")
        .port()
}

async fn migrated_db(label: &str) -> (GolishDb, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let config = DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("attack_v2_{label}_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    };
    let db = GolishDb::start(config)
        .await
        .expect("start migrated embedded postgres");
    (db, data_dir)
}

fn assert_sqlstate<T>(result: Result<T, sqlx::Error>, expected: &str, context: &str) {
    let error = match result {
        Ok(_) => panic!("{context} must fail"),
        Err(error) => error,
    };
    let sqlstate = match &error {
        sqlx::Error::Database(database_error) => {
            database_error.code().map(|code| code.into_owned())
        }
        _ => None,
    };
    assert_eq!(
        sqlstate.as_deref(),
        Some(expected),
        "{context} must fail with SQLSTATE {expected}, got {error}"
    );
}

#[tokio::test]
#[serial]
async fn wave_entry_and_candidate_decision_authorities_are_distinct() {
    let (mut db, _data_dir) = migrated_db("distinct_candidate_authority").await;
    let columns: Vec<String> = sqlx::query_scalar(
        r#"SELECT column_name
             FROM information_schema.columns
            WHERE table_schema='public'
              AND table_name IN ('attack_wave_units', 'attack_candidates')
              AND column_name IN (
                  'entry_stage_execution_id',
                  'entry_stage_run_unit_id',
                  'entry_deliverable_submission_id',
                  'entry_stage_kind',
                  'decision_stage_execution_id',
                  'decision_stage_run_unit_id',
                  'decision_deliverable_submission_id',
                  'decision_stage_kind'
              )
            ORDER BY column_name"#,
    )
    .fetch_all(db.pool())
    .await
    .expect("inspect Candidate V2 authority columns");
    assert_eq!(
        columns,
        vec![
            "decision_deliverable_submission_id",
            "decision_stage_execution_id",
            "decision_stage_kind",
            "decision_stage_run_unit_id",
            "entry_deliverable_submission_id",
            "entry_stage_execution_id",
            "entry_stage_kind",
            "entry_stage_run_unit_id",
        ],
        "Wave entry authority and Candidate decision authority must not reuse the same source columns"
    );
    db.stop().await;
}

#[derive(Clone, Copy)]
struct OrgFixture {
    organization_id: Uuid,
    target_id: Uuid,
    entry_stage_run_unit_id: Uuid,
    entry_worker_run_id: Uuid,
    entry_lease_token: Uuid,
    entry_submission_id: Uuid,
    stage_run_unit_id: Uuid,
    worker_run_id: Uuid,
    lease_token: Uuid,
    submission_id: Uuid,
    wave_unit_id: Uuid,
}

struct AttackFixture {
    session_id: Uuid,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    entry_stage_execution_id: Uuid,
    stage_execution_id: Uuid,
    wave_run_id: Uuid,
    org_a: OrgFixture,
    org_b: OrgFixture,
}

#[allow(clippy::too_many_arguments)]
async fn insert_final_passed_unit(
    pool: &PgPool,
    fixture: &AttackFixture,
    organization_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    worker_run_id: Uuid,
    lease_token: Uuid,
    submission_id: Uuid,
    stage_kind: &str,
    specialist: &str,
    ordinal: i32,
    publish_final_pass: bool,
) {
    let handoff_evidence_ids = if stage_kind == "vuln_triage" {
        let target_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM targets WHERE organization_id=$1 AND scope='in' ORDER BY id LIMIT 1",
        )
        .bind(organization_id)
        .fetch_one(pool)
        .await
        .expect("load formulaic entry target");
        let evidence_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO audit_log (
                   action,category,details,project_path,audit_role,run_id,target_id,detail
               ) VALUES (
                   'formulaic observation','attack','','/tmp/attack-v2','evidence',$1,$2,$3
               ) RETURNING id"#,
        )
        .bind(fixture.operation_id)
        .bind(target_id)
        .bind(serde_json::json!({"organization_id": organization_id}))
        .fetch_one(pool)
        .await
        .expect("insert formulaic entry evidence");
        vec![evidence_id]
    } else {
        vec![]
    };
    sqlx::query(
        r#"INSERT INTO stage_run_units (
               id, operation_id, stage_execution_id, scope_snapshot_id,
               organization_id, stage_kind, generation, specialist, status
           ) VALUES ($1,$2,$3,$4,$5,$6,0,$7,'running')"#,
    )
    .bind(stage_run_unit_id)
    .bind(fixture.operation_id)
    .bind(stage_execution_id)
    .bind(fixture.scope_snapshot_id)
    .bind(organization_id)
    .bind(stage_kind)
    .bind(specialist)
    .execute(pool)
    .await
    .expect("insert trusted stage run unit");

    sqlx::query(
        r#"INSERT INTO stage_worker_runs (
               id, operation_id, stage_execution_id, stage_run_unit_id,
               organization_id, worker_generation, specialist, work_item_kind,
               work_item_key, agent_path, status, lease_token, lease_owner,
               lease_acquired_at, lease_expires_at, heartbeat_at, attempt_epoch
           ) VALUES (
               $1,$2,$3,$4,$5,0,$6,'stage_unit',$7,
               $8,'running',$9,'attack-v2-fixture',NOW(),NOW()+INTERVAL '5 minutes',NOW(),0
           )"#,
    )
    .bind(worker_run_id)
    .bind(fixture.operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(organization_id)
    .bind(specialist)
    .bind(format!("org-{ordinal}"))
    .bind(format!("main>{stage_kind}:{ordinal}"))
    .bind(lease_token)
    .execute(pool)
    .await
    .expect("insert trusted worker run");

    let tool_call_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO tool_calls (
               id, call_id, session_id, task_id, agent, name, args, result, status,
               operation_id, stage_execution_id, stage_run_unit_id, worker_run_id,
               organization_id, attempt_epoch, lease_token
           ) VALUES (
               $1,$2,$3,$4,'primary','submit_stage_deliverable','{}','{}','finished',
               $4,$5,$6,$7,$8,0,$9
           )"#,
    )
    .bind(tool_call_id)
    .bind(format!("{stage_kind}-submit-{ordinal}"))
    .bind(fixture.session_id)
    .bind(fixture.operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(worker_run_id)
    .bind(organization_id)
    .bind(lease_token)
    .execute(pool)
    .await
    .expect("insert trusted submission tool call");

    sqlx::query(
        r#"INSERT INTO stage_deliverable_submissions (
               id, operation_id, stage_execution_id, stage_run_unit_id,
               worker_run_id, organization_id, tool_call_record_id,
               tool_request_id, stage_kind, attempt_epoch, lease_token,
               payload, payload_sha256
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,$9,0,$10,$11,$12
           )"#,
    )
    .bind(submission_id)
    .bind(fixture.operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(worker_run_id)
    .bind(organization_id)
    .bind(tool_call_id)
    .bind(format!("{stage_kind}-submit-{ordinal}"))
    .bind(stage_kind)
    .bind(lease_token)
    .bind(serde_json::json!({"schema_version": 1, "candidates": []}))
    .bind(format!("sha256:{stage_kind}-submission-{ordinal}"))
    .execute(pool)
    .await
    .expect("insert trusted deliverable submission");
    if !publish_final_pass {
        return;
    }
    sqlx::query(
        r#"UPDATE stage_run_units
           SET status='passed',terminal_at=NOW(),updated_at=NOW(),
               pass_watermark=$2
           WHERE id=$1"#,
    )
    .bind(stage_run_unit_id)
    .bind(serde_json::json!({
        "final_gate_passed": true,
        "deliverable_submission_id": submission_id
    }))
    .execute(pool)
    .await
    .expect("mark trusted source unit final passed");
    sqlx::query(
        r#"INSERT INTO stage_handoffs (
               id,operation_id,organization_id,scope_snapshot_id,from_stage_kind,
               stage_execution_id,source_stage_run_unit_id,deliverable_submission_id,
               scope_hash,payload,payload_sha256,evidence_ids,
               unit_gate_decision_hash,gate_passed_at
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,'sha256:scope',$9,$10,$11,$12,NOW()
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(fixture.operation_id)
    .bind(organization_id)
    .bind(fixture.scope_snapshot_id)
    .bind(stage_kind)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(submission_id)
    .bind(serde_json::json!({"accepted": true}))
    .bind(format!("sha256:{stage_kind}-handoff-{ordinal}"))
    .bind(handoff_evidence_ids)
    .bind(format!("sha256:{stage_kind}-gate-{ordinal}"))
    .execute(pool)
    .await
    .expect("insert immutable final-pass handoff");
}

async fn seed_attack_fixture_with_candidate_pass(
    pool: &PgPool,
    candidate_final_passed: bool,
) -> AttackFixture {
    let session_id = Uuid::new_v4();
    let operation_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    let scope_decision_id = Uuid::new_v4();
    let scope_snapshot_id = Uuid::new_v4();
    let entry_stage_execution_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    let wave_run_id = Uuid::new_v4();
    let root_organization_id = Uuid::new_v4();
    let org_a = OrgFixture {
        organization_id: Uuid::new_v4(),
        target_id: Uuid::new_v4(),
        entry_stage_run_unit_id: Uuid::new_v4(),
        entry_worker_run_id: Uuid::new_v4(),
        entry_lease_token: Uuid::new_v4(),
        entry_submission_id: Uuid::new_v4(),
        stage_run_unit_id: Uuid::new_v4(),
        worker_run_id: Uuid::new_v4(),
        lease_token: Uuid::new_v4(),
        submission_id: Uuid::new_v4(),
        wave_unit_id: Uuid::new_v4(),
    };
    let org_b = OrgFixture {
        organization_id: Uuid::new_v4(),
        target_id: Uuid::new_v4(),
        entry_stage_run_unit_id: Uuid::new_v4(),
        entry_worker_run_id: Uuid::new_v4(),
        entry_lease_token: Uuid::new_v4(),
        entry_submission_id: Uuid::new_v4(),
        stage_run_unit_id: Uuid::new_v4(),
        worker_run_id: Uuid::new_v4(),
        lease_token: Uuid::new_v4(),
        submission_id: Uuid::new_v4(),
        wave_unit_id: Uuid::new_v4(),
    };
    let fixture = AttackFixture {
        session_id,
        operation_id,
        scope_snapshot_id,
        entry_stage_execution_id,
        stage_execution_id,
        wave_run_id,
        org_a,
        org_b,
    };

    sqlx::query(
        r#"INSERT INTO sessions (id,title,status,project_path)
           VALUES ($1,'attack v2 fixture','running','/tmp/attack-v2')"#,
    )
    .bind(session_id)
    .execute(pool)
    .await
    .expect("insert fixture session");
    sqlx::query(
        r#"INSERT INTO tasks (id,session_id,title,input,status)
           VALUES ($1,$2,'attack v2 operation','verify candidates','running')"#,
    )
    .bind(operation_id)
    .bind(session_id)
    .execute(pool)
    .await
    .expect("insert fixture operation task");
    sqlx::query(
        r#"INSERT INTO project_scopes (
               project_scope_id,canonical_project_path,path_sha256
           ) VALUES ($1,'/tmp/attack-v2','sha256:attack-v2')"#,
    )
    .bind(project_scope_id)
    .execute(pool)
    .await
    .expect("insert fixture project scope");
    sqlx::query(
        r#"INSERT INTO operation_state (
               operation_id,profile,current_stage,runtime_memory_contract,
               attack_execution_contract,project_scope_id
           ) VALUES ($1,'red_team','attack_candidate','v2_only','v2_only',$2)"#,
    )
    .bind(operation_id)
    .bind(project_scope_id)
    .execute(pool)
    .await
    .expect("insert frozen v2 operation contracts");

    sqlx::query(
        r#"INSERT INTO organizations (id,project_path,name,parent_id) VALUES
               ($1,'/tmp/attack-v2','Root Org',NULL),
               ($2,'/tmp/attack-v2','Sibling A',$1),
               ($3,'/tmp/attack-v2','Sibling B',$1)"#,
    )
    .bind(root_organization_id)
    .bind(org_a.organization_id)
    .bind(org_b.organization_id)
    .execute(pool)
    .await
    .expect("insert live sibling organizations");
    sqlx::query(
        r#"INSERT INTO targets (
               id,name,target_type,value,scope,project_path,organization_id
           ) VALUES
               ($1,'Sibling A app','url','https://shared.example.test/login','in','/tmp/attack-v2',$2),
               ($3,'Sibling B app','url','https://shared.example.test/login','in','/tmp/attack-v2',$4)"#,
    )
    .bind(org_a.target_id)
    .bind(org_a.organization_id)
    .bind(org_b.target_id)
    .bind(org_b.organization_id)
    .execute(pool)
    .await
    .expect("insert sibling-owned live targets");
    sqlx::query(
        r#"INSERT INTO stage_runs (id,operation_id,stage_kind,status) VALUES
               ($1,$3,'vuln_triage','started'),
               ($2,$3,'attack_candidate','started')"#,
    )
    .bind(entry_stage_execution_id)
    .bind(stage_execution_id)
    .bind(operation_id)
    .execute(pool)
    .await
    .expect("insert attack candidate stage execution");
    sqlx::query(
        r#"INSERT INTO operation_scope_decisions (
               id,operation_id,project_scope_id,stage_execution_id,
               root_organization_id,mode,decision_rows,decision_hash
           ) VALUES ($1,$2,$3,$4,$5,'cli_flags',$6,'sha256:scope-decision')"#,
    )
    .bind(scope_decision_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(stage_execution_id)
    .bind(root_organization_id)
    .bind(serde_json::json!([
        {"organization_id": root_organization_id},
        {"organization_id": org_a.organization_id},
        {"organization_id": org_b.organization_id}
    ]))
    .execute(pool)
    .await
    .expect("insert trusted scope decision");
    let mut scope_tx = pool.begin().await.expect("begin frozen scope transaction");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_snapshots (
               id,operation_id,project_scope_id,scope_decision_id,
               project_path_at_freeze,root_organization_id,mode,scope_hash
           ) VALUES ($1,$2,$3,$4,'/tmp/attack-v2',$5,'cli_flags','sha256:scope')"#,
    )
    .bind(scope_snapshot_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(scope_decision_id)
    .bind(root_organization_id)
    .execute(&mut *scope_tx)
    .await
    .expect("insert frozen scope header");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units (
               snapshot_id,organization_id,parent_organization_id,
               organization_name_at_freeze,role,depth,ordinal,
               ownership_percent,decision_row_id,approval_source
           ) VALUES
               ($1,$2,NULL,'Root Org','root',0,0,NULL,'root',$5),
               ($1,$3,$2,'Sibling A','subsidiary',1,1,100,'sibling-a',$5),
               ($1,$4,$2,'Sibling B','subsidiary',1,2,100,'sibling-b',$5)"#,
    )
    .bind(scope_snapshot_id)
    .bind(root_organization_id)
    .bind(org_a.organization_id)
    .bind(org_b.organization_id)
    .bind(serde_json::json!({"source": "cli_flags"}))
    .execute(&mut *scope_tx)
    .await
    .expect("insert frozen sibling scope units");
    sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
        .bind(scope_snapshot_id)
        .execute(&mut *scope_tx)
        .await
        .expect("seal frozen attack scope");
    scope_tx
        .commit()
        .await
        .expect("commit frozen sibling scope");

    for (org, ordinal) in [(org_a, 1_i32), (org_b, 2_i32)] {
        insert_final_passed_unit(
            pool,
            &fixture,
            org.organization_id,
            fixture.entry_stage_execution_id,
            org.entry_stage_run_unit_id,
            org.entry_worker_run_id,
            org.entry_lease_token,
            org.entry_submission_id,
            "vuln_triage",
            "formulaic_scanner",
            ordinal,
            true,
        )
        .await;
        insert_final_passed_unit(
            pool,
            &fixture,
            org.organization_id,
            fixture.stage_execution_id,
            org.stage_run_unit_id,
            org.worker_run_id,
            org.lease_token,
            org.submission_id,
            "attack_candidate",
            "attack_analyst",
            ordinal,
            candidate_final_passed,
        )
        .await;
    }
    sqlx::query(
        "UPDATE stage_runs SET status='completed',completed_at=NOW() WHERE id=$1 AND status='started'",
    )
    .bind(entry_stage_execution_id)
    .execute(pool)
    .await
    .expect("complete predecessor vuln_triage stage execution");

    sqlx::query(
        r#"INSERT INTO attack_wave_runs (
               id,operation_id,scope_snapshot_id,generation,status,
               policy_snapshot,policy_hash,max_waves,max_candidates_total,
               max_chain_depth,max_attempts_total
           ) VALUES (
               $1,$2,$3,0,'open',$4,'sha256:policy',3,100,3,200
           )"#,
    )
    .bind(wave_run_id)
    .bind(operation_id)
    .bind(scope_snapshot_id)
    .bind(serde_json::json!({
        "max_waves": 3,
        "max_candidates_total": 100,
        "max_chain_depth": 3,
        "max_attempts_total": 200
    }))
    .execute(pool)
    .await
    .expect("insert attack wave run");
    for (org, ordinal) in [(org_a, 1_i32), (org_b, 2_i32)] {
        sqlx::query(
            r#"INSERT INTO attack_wave_units (
                   id,wave_run_id,operation_id,scope_snapshot_id,organization_id,
                   entry_stage_execution_id,entry_stage_run_unit_id,
                   entry_deliverable_submission_id,entry_stage_kind,ordinal,status
               ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'vuln_triage',$9,'open')"#,
        )
        .bind(org.wave_unit_id)
        .bind(wave_run_id)
        .bind(operation_id)
        .bind(scope_snapshot_id)
        .bind(org.organization_id)
        .bind(entry_stage_execution_id)
        .bind(org.entry_stage_run_unit_id)
        .bind(org.entry_submission_id)
        .bind(ordinal)
        .execute(pool)
        .await
        .expect("insert attack wave unit from trusted submission");
    }
    fixture
}

async fn seed_attack_fixture(pool: &PgPool) -> AttackFixture {
    seed_attack_fixture_with_candidate_pass(pool, true).await
}

#[tokio::test]
#[serial]
async fn frozen_entry_evidence_requires_exact_live_handoff_membership() {
    use golish_db::repo::attack_candidate_work_items::{
        load_frozen_entry_evidence_ids_with_connection, seed_wave_work_items,
        SeedAttackObservation, SeedAttackWorkItems,
    };

    let (mut db, _data_dir) = migrated_db("frozen_entry_evidence").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let linked_evidence_id: i64 = sqlx::query_scalar(
        r#"SELECT evidence_ids[1] FROM stage_handoffs
            WHERE source_stage_run_unit_id=$1 AND invalidated_at IS NULL"#,
    )
    .bind(fixture.org_a.entry_stage_run_unit_id)
    .fetch_one(db.pool())
    .await
    .expect("load exact entry handoff evidence");
    sqlx::query("DELETE FROM targets WHERE id=$1 AND organization_id=$2")
        .bind(fixture.org_a.target_id)
        .bind(fixture.org_a.organization_id)
        .execute(db.pool())
        .await
        .expect("delete live target after predecessor final seal");
    let mut seed_tx = db.pool().begin().await.expect("begin linked manifest seed");
    seed_wave_work_items(
        &mut seed_tx,
        SeedAttackWorkItems {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_a.wave_unit_id,
            organization_id: fixture.org_a.organization_id,
            observations: vec![SeedAttackObservation {
                work_item_key: "formulaic:linked".to_string(),
                target_live_id: None,
                target_type_at_time: "url".to_string(),
                target_value_at_time: "https://shared.example.test/login".to_string(),
                target_identity_hash: "sha256:linked-target".to_string(),
                technique: "WSTG-INPV-05".to_string(),
                observation: serde_json::json!({"outcome": "found"}),
                observation_hash: "sha256:linked-observation".to_string(),
                evidence_ids: vec![linked_evidence_id],
            }],
        },
    )
    .await
    .expect("freeze linked Candidate manifest");
    seed_tx.commit().await.expect("commit linked manifest seed");

    let mut seal_tx = db.pool().begin().await.expect("begin entry evidence read");
    let exact = load_frozen_entry_evidence_ids_with_connection(
        &mut seal_tx,
        fixture.operation_id,
        fixture.scope_snapshot_id,
        fixture.wave_run_id,
        fixture.org_a.wave_unit_id,
        fixture.org_a.organization_id,
    )
    .await
    .expect("exact frozen entry evidence is authorized");
    assert_eq!(exact, vec![linked_evidence_id]);
    seal_tx
        .rollback()
        .await
        .expect("release entry evidence locks");

    let unlinked_evidence_id = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_b.organization_id,
        fixture.org_b.target_id,
        "evidence",
    )
    .await;
    let mut hostile_seed = db
        .pool()
        .begin()
        .await
        .expect("begin unlinked manifest seed");
    seed_wave_work_items(
        &mut hostile_seed,
        SeedAttackWorkItems {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_b.wave_unit_id,
            organization_id: fixture.org_b.organization_id,
            observations: vec![SeedAttackObservation {
                work_item_key: "formulaic:unlinked".to_string(),
                target_live_id: Some(fixture.org_b.target_id),
                target_type_at_time: "url".to_string(),
                target_value_at_time: "https://shared.example.test/login".to_string(),
                target_identity_hash: "sha256:unlinked-target".to_string(),
                technique: "WSTG-INPV-05".to_string(),
                observation: serde_json::json!({"outcome": "found"}),
                observation_hash: "sha256:unlinked-observation".to_string(),
                evidence_ids: vec![unlinked_evidence_id],
            }],
        },
    )
    .await
    .expect("freeze otherwise-owned but handoff-unlinked manifest");
    hostile_seed
        .commit()
        .await
        .expect("commit hostile manifest fixture");
    let mut hostile_read = db
        .pool()
        .begin()
        .await
        .expect("begin hostile evidence read");
    assert!(load_frozen_entry_evidence_ids_with_connection(
        &mut hostile_read,
        fixture.operation_id,
        fixture.scope_snapshot_id,
        fixture.wave_run_id,
        fixture.org_b.wave_unit_id,
        fixture.org_b.organization_id,
    )
    .await
    .is_err());
    hostile_read
        .rollback()
        .await
        .expect("release hostile evidence locks");

    sqlx::query("UPDATE stage_handoffs SET invalidated_at=NOW() WHERE source_stage_run_unit_id=$1")
        .bind(fixture.org_a.entry_stage_run_unit_id)
        .execute(db.pool())
        .await
        .expect("invalidate exact entry handoff");
    let mut invalidated = db.pool().begin().await.expect("begin invalidated read");
    assert!(load_frozen_entry_evidence_ids_with_connection(
        &mut invalidated,
        fixture.operation_id,
        fixture.scope_snapshot_id,
        fixture.wave_run_id,
        fixture.org_a.wave_unit_id,
        fixture.org_a.organization_id,
    )
    .await
    .is_err());
    invalidated
        .rollback()
        .await
        .expect("release invalidated evidence locks");
    db.stop().await;
}

fn candidate_final_seal_input(
    fixture: &AttackFixture,
    unit: &stage_run_units::StageRunUnitRow,
    acceptance: golish_db::repo::attack_candidates::CandidateAcceptanceInput,
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
    let mut expected_work_item_ids = acceptance.expected_work_item_ids.clone();
    expected_work_item_ids.sort_unstable();
    let mut typed_claims = acceptance
        .candidates
        .iter()
        .map(|decision| {
            serde_json::json!({
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
            serde_json::json!({
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
    let coverage_watermark = serde_json::json!({
        "kind": "candidate_manifest_v1",
        "stage": "attack_candidate",
        "organization_id": fixture.org_a.organization_id,
        "wave_run_id": acceptance.wave_run_id,
        "wave_unit_id": acceptance.wave_unit_id,
        "manifest_hash": acceptance.manifest_hash,
        "expected_work_item_ids": expected_work_item_ids,
        "candidate_ids": candidate_ids,
        "no_candidate_work_item_ids": no_candidate_work_item_ids,
        "decision_evidence_ids": evidence_ids,
        "terminal_count": acceptance.candidates.len()
            + acceptance.no_candidate_decisions.len(),
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
    let terminal_checkpoint = serde_json::json!({"terminal": true});
    let details = serde_json::json!({
        "source": "authoritative_org_gate",
        "stage": "attack_candidate",
        "organization_id": fixture.org_a.organization_id,
    });
    let seal_material = serde_json::json!({
        "canonical_fact_keys": canonical_fact_keys,
        "typed_claims": typed_claims,
        "coverage_watermark": coverage_watermark,
        "evidence_ids": evidence_ids,
        "terminal_checkpoint": terminal_checkpoint,
        "deterministic_gate_details": details,
        "candidate_acceptance": acceptance,
    });
    let gate_decision = serde_json::json!({
        "outcome": "pass",
        "operation_id": fixture.operation_id,
        "stage_execution_id": fixture.stage_execution_id,
        "stage_run_unit_id": fixture.org_a.stage_run_unit_id,
        "deliverable_submission_id": fixture.org_a.submission_id,
        "scope_hash": "sha256:scope",
        "seal_material_sha256": sha256_json(&seal_material),
        "details": details,
    });
    runtime_memory_tx::FinalizeUnitPassRow {
        fence: runtime_memory_tx::RuntimeMemoryTxFence {
            operation_id: fixture.operation_id,
            stage_execution_id: fixture.stage_execution_id,
            stage_run_unit_id: fixture.org_a.stage_run_unit_id,
            worker_run_id: fixture.org_a.worker_run_id,
            lease_token: fixture.org_a.lease_token,
            attempt_epoch: 0,
            expected_checkpoint_version: 0,
        },
        deliverable_submission_id: fixture.org_a.submission_id,
        expected_unit_status: stage_run_units::StageRunUnitStatus::Running,
        expected_unit_row_version: unit.row_version,
        scope_hash: "sha256:scope".to_string(),
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

#[tokio::test]
#[serial]
async fn candidate_final_seal_is_atomic_uses_exact_predecessor_evidence_and_keeps_ref_hash() {
    use golish_db::repo::attack_candidate_work_items::{
        canonical_manifest_hash, seed_wave_work_items, SeedAttackObservation, SeedAttackWorkItems,
    };

    let (mut db, _data_dir) = migrated_db("candidate_final_seal").await;
    let fixture = seed_attack_fixture_with_candidate_pass(db.pool(), false).await;
    sqlx::query("UPDATE stage_run_units SET started_at=NOW(),updated_at=NOW() WHERE id=$1")
        .bind(fixture.org_a.stage_run_unit_id)
        .execute(db.pool())
        .await
        .expect("start exact Candidate Unit");
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET status='running',checkpoint='{}',checkpoint_version=0,
                  lease_expires_at=NOW()+INTERVAL '5 minutes',heartbeat_at=NOW()
            WHERE id=$1"#,
    )
    .bind(fixture.org_a.worker_run_id)
    .execute(db.pool())
    .await
    .expect("restore exact Candidate Worker lease");
    let linked_evidence_id: i64 = sqlx::query_scalar(
        "SELECT evidence_ids[1] FROM stage_handoffs WHERE source_stage_run_unit_id=$1",
    )
    .bind(fixture.org_a.entry_stage_run_unit_id)
    .fetch_one(db.pool())
    .await
    .expect("load predecessor handoff evidence");
    let mut seed_tx = db
        .pool()
        .begin()
        .await
        .expect("begin Candidate manifest seed");
    let seeded = seed_wave_work_items(
        &mut seed_tx,
        SeedAttackWorkItems {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_a.wave_unit_id,
            organization_id: fixture.org_a.organization_id,
            observations: vec![SeedAttackObservation {
                work_item_key: "formulaic:atomic-final-seal".to_string(),
                target_live_id: Some(fixture.org_a.target_id),
                target_type_at_time: "url".to_string(),
                target_value_at_time: "https://shared.example.test/login".to_string(),
                target_identity_hash: "sha256:atomic-target".to_string(),
                technique: "WSTG-INPV-05".to_string(),
                observation: serde_json::json!({"outcome": "found"}),
                observation_hash: "sha256:atomic-observation".to_string(),
                evidence_ids: vec![linked_evidence_id],
            }],
        },
    )
    .await
    .expect("freeze exact Candidate manifest");
    seed_tx
        .commit()
        .await
        .expect("commit Candidate manifest seed");
    let work_item_id = seeded.items[0].work_item.id;
    let manifest = golish_db::repo::attack_candidate_work_items::load_for_wave_unit(
        db.pool(),
        fixture.operation_id,
        fixture.scope_snapshot_id,
        fixture.wave_run_id,
        fixture.org_a.wave_unit_id,
        fixture.org_a.organization_id,
    )
    .await
    .expect("reload frozen Candidate manifest");
    let candidate_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{}:{work_item_id}", fixture.operation_id).as_bytes(),
    );
    let execution_plan = serde_json::json!({
        "schema_version": "candidate-plan-v1",
        "classifier_version": "candidate-classifier-v1",
        "candidate_id": candidate_id,
        "target_identity_hash": "sha256:atomic-target",
        "actions": [],
        "budget": {"max_actions": 1, "max_requests": 1, "max_runtime_ms": 1000},
        "foreground_only": true,
    });
    let candidate_plan_hash =
        canonical_execution_plan_hash(&execution_plan).expect("hash immutable Candidate plan");
    let base_acceptance = golish_db::repo::attack_candidates::CandidateAcceptanceInput {
        wave_run_id: fixture.wave_run_id,
        wave_unit_id: fixture.org_a.wave_unit_id,
        manifest_hash: canonical_manifest_hash(&manifest),
        expected_work_item_ids: vec![work_item_id],
        candidates: vec![AcceptedCandidateDraft {
            candidate_id,
            work_item_id,
            hypothesis: "bounded SQL injection hypothesis".to_string(),
            technique: Some("WSTG-INPV-05".to_string()),
            rationale: "grounded by exact predecessor evidence".to_string(),
            prior_refs: vec![format!("audit:{linked_evidence_id}")],
            suggested_approach: "bounded_sql_injection_probe".to_string(),
            priority: "high".to_string(),
            execution_plan,
            candidate_plan_hash,
            risk_class: "exploit".to_string(),
            evidence_ids: vec![linked_evidence_id],
        }],
        no_candidate_decisions: vec![],
    };
    let unit = sqlx::query_as::<_, stage_run_units::StageRunUnitRow>(
        "SELECT * FROM stage_run_units WHERE id=$1",
    )
    .bind(fixture.org_a.stage_run_unit_id)
    .fetch_one(db.pool())
    .await
    .expect("load running Candidate Unit");
    let keys =
        vec![canonical_fact_refs::CanonicalFactKey::AttackCandidateWorkItem { work_item_id }];
    let mut before_tx = db.pool().begin().await.expect("begin pre-accept resolve");
    let before = canonical_fact_refs::resolve_for_handoff(
        &mut before_tx,
        fixture.operation_id,
        fixture.org_a.organization_id,
        "/tmp/attack-v2",
        chrono::Utc::now() + chrono::Duration::days(1),
        &keys,
    )
    .await
    .expect("frozen Candidate work item resolves despite predecessor age");
    before_tx
        .rollback()
        .await
        .expect("release pre-accept ref lock");

    let unlinked_old_evidence_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO audit_log (
               action,category,details,project_path,audit_role,run_id,target_id,detail,created_at
           ) VALUES (
               'unlinked old evidence','attack','','/tmp/attack-v2','evidence',$1,$2,$3,$4
           ) RETURNING id"#,
    )
    .bind(fixture.operation_id)
    .bind(fixture.org_a.target_id)
    .bind(serde_json::json!({"organization_id": fixture.org_a.organization_id}))
    .bind(unit.started_at.expect("running Unit started") - chrono::Duration::hours(1))
    .fetch_one(db.pool())
    .await
    .expect("insert same-owner but manifest-unlinked old evidence");
    let mut hostile_acceptance = base_acceptance.clone();
    hostile_acceptance.candidates[0].evidence_ids = vec![unlinked_old_evidence_id];
    hostile_acceptance.candidates[0].prior_refs = vec![format!("audit:{unlinked_old_evidence_id}")];
    let hostile = candidate_final_seal_input(&fixture, &unit, hostile_acceptance);
    let hostile_error = runtime_memory_tx::finalize_unit_pass(db.pool(), &hostile)
        .await
        .expect_err("unlinked predecessor evidence must fail closed");
    assert!(
        hostile_error
            .to_string()
            .contains("final_seal_evidence_stale_or_foreign"),
        "unexpected hostile final-seal error: {hostile_error}"
    );
    let partial: (i64, i64, String) = sqlx::query_as(
        r#"SELECT
               (SELECT COUNT(*) FROM stage_handoffs WHERE source_stage_run_unit_id=$1),
               (SELECT COUNT(*) FROM attack_candidates WHERE decision_stage_run_unit_id=$1),
               (SELECT status FROM stage_run_units WHERE id=$1)"#,
    )
    .bind(fixture.org_a.stage_run_unit_id)
    .fetch_one(db.pool())
    .await
    .expect("inspect hostile final-seal rollback");
    assert_eq!(partial, (0, 0, "running".to_string()));

    let valid = candidate_final_seal_input(&fixture, &unit, base_acceptance.clone());
    let finalized = runtime_memory_tx::finalize_unit_pass(db.pool(), &valid)
        .await
        .expect("Candidate Unit, handoff, and acceptance finalize atomically");
    assert!(!finalized.replayed);
    let committed: (i64, i64, String) = sqlx::query_as(
        r#"SELECT
               (SELECT COUNT(*) FROM stage_handoffs WHERE source_stage_run_unit_id=$1),
               (SELECT COUNT(*) FROM attack_candidates WHERE decision_stage_run_unit_id=$1),
               (SELECT status FROM stage_run_units WHERE id=$1)"#,
    )
    .bind(fixture.org_a.stage_run_unit_id)
    .fetch_one(db.pool())
    .await
    .expect("inspect atomic Candidate final seal");
    assert_eq!(committed, (1, 1, "passed".to_string()));

    let mut after_tx = db.pool().begin().await.expect("begin post-accept resolve");
    let after = canonical_fact_refs::resolve_for_handoff(
        &mut after_tx,
        fixture.operation_id,
        fixture.org_a.organization_id,
        "/tmp/attack-v2",
        chrono::Utc::now() + chrono::Duration::days(1),
        &keys,
    )
    .await
    .expect("accepted Candidate work item remains resolvable");
    after_tx
        .rollback()
        .await
        .expect("release post-accept ref lock");
    assert_eq!(before[0].content_sha256, after[0].content_sha256);
    assert_eq!(before[0].evidence_ids, after[0].evidence_ids);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn frozen_manifest_blocks_raw_drift_and_replay_requires_exact_evidence_set() {
    use golish_db::repo::attack_candidate_work_items::{
        seed_wave_work_items, SeedAttackObservation, SeedAttackWorkItems,
    };

    let (mut db, _data_dir) = migrated_db("manifest_freeze_hostile").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let linked_evidence_id: i64 = sqlx::query_scalar(
        "SELECT evidence_ids[1] FROM stage_handoffs WHERE source_stage_run_unit_id=$1",
    )
    .bind(fixture.org_a.entry_stage_run_unit_id)
    .fetch_one(db.pool())
    .await
    .expect("load linked predecessor evidence");
    let second_evidence_id = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_a.organization_id,
        fixture.org_a.target_id,
        "evidence",
    )
    .await;
    let third_evidence_id = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_a.organization_id,
        fixture.org_a.target_id,
        "evidence",
    )
    .await;
    let command = SeedAttackWorkItems {
        operation_id: fixture.operation_id,
        scope_snapshot_id: fixture.scope_snapshot_id,
        wave_run_id: fixture.wave_run_id,
        wave_unit_id: fixture.org_a.wave_unit_id,
        organization_id: fixture.org_a.organization_id,
        observations: vec![SeedAttackObservation {
            work_item_key: "formulaic:freeze".to_string(),
            target_live_id: Some(fixture.org_a.target_id),
            target_type_at_time: "url".to_string(),
            target_value_at_time: "https://shared.example.test/login".to_string(),
            target_identity_hash: "sha256:freeze-target".to_string(),
            technique: "WSTG-INPV-05".to_string(),
            observation: serde_json::json!({"outcome": "found"}),
            observation_hash: "sha256:freeze-observation".to_string(),
            evidence_ids: vec![linked_evidence_id, second_evidence_id],
        }],
    };
    let mut seed_tx = db.pool().begin().await.expect("begin manifest freeze");
    let seeded = seed_wave_work_items(&mut seed_tx, command.clone())
        .await
        .expect("freeze exact manifest");
    seed_tx.commit().await.expect("commit manifest freeze");

    let raw_append = sqlx::query(
        r#"INSERT INTO attack_candidate_seeds (
               id,wave_unit_id,operation_id,scope_snapshot_id,organization_id,
               target_live_id,target_type_at_time,target_value_at_time,target_identity_hash,
               technique,observation,observation_hash
           ) VALUES ($1,$2,$3,$4,$5,$6,'url','https://shared.example.test/other',
                     'sha256:hostile-target','WSTG-INPV-01',$7,'sha256:hostile')"#,
    )
    .bind(Uuid::new_v4())
    .bind(fixture.org_a.wave_unit_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.org_a.organization_id)
    .bind(fixture.org_a.target_id)
    .bind(serde_json::json!({"outcome": "found"}))
    .execute(db.pool())
    .await;
    assert_sqlstate(raw_append.map(|_| ()), "P0001", "append frozen seed");

    let raw_mutation = sqlx::query(
        "UPDATE attack_candidate_work_items SET work_item_key='formulaic:drift' WHERE id=$1",
    )
    .bind(seeded.items[0].work_item.id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        raw_mutation.map(|_| ()),
        "P0001",
        "mutate frozen work-item identity",
    );
    let raw_evidence_append = sqlx::query(
        "INSERT INTO attack_candidate_work_item_evidence(work_item_id,evidence_id,role)
         VALUES($1,$2,'support')",
    )
    .bind(seeded.items[0].work_item.id)
    .bind(third_evidence_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        raw_evidence_append.map(|_| ()),
        "P0001",
        "append frozen manifest evidence",
    );

    let mut replay_tx = db.pool().begin().await.expect("begin exact seed replay");
    seed_wave_work_items(&mut replay_tx, command.clone())
        .await
        .expect("exact seed replay remains idempotent");
    replay_tx.commit().await.expect("commit exact seed replay");

    let mut missing = command.clone();
    missing.observations[0].evidence_ids = vec![linked_evidence_id];
    let mut missing_tx = db.pool().begin().await.expect("begin missing replay");
    assert!(seed_wave_work_items(&mut missing_tx, missing)
        .await
        .is_err());
    missing_tx
        .rollback()
        .await
        .expect("rollback missing replay");

    let mut extra = command;
    extra.observations[0].evidence_ids.push(third_evidence_id);
    let mut extra_tx = db.pool().begin().await.expect("begin extra replay");
    assert!(seed_wave_work_items(&mut extra_tx, extra).await.is_err());
    extra_tx.rollback().await.expect("rollback extra replay");
    db.stop().await;
}

#[derive(Clone, Copy)]
struct CandidateFixture {
    seed_id: Uuid,
    work_item_id: Uuid,
    candidate_id: Uuid,
}

async fn seed_candidate(
    pool: &PgPool,
    fixture: &AttackFixture,
    org: OrgFixture,
) -> CandidateFixture {
    let seed_id = Uuid::new_v4();
    let work_item_id = Uuid::new_v4();
    let candidate_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO attack_candidate_seeds (
               id,wave_unit_id,operation_id,scope_snapshot_id,organization_id,
               target_live_id,target_type_at_time,target_value_at_time,
               target_identity_hash,technique,observation,observation_hash
           ) VALUES (
               $1,$2,$3,$4,$5,$6,'url','https://shared.example.test/login',
               'sha256:shared-target','WSTG-INPV-05',$7,'sha256:observation'
           )"#,
    )
    .bind(seed_id)
    .bind(org.wave_unit_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(org.organization_id)
    .bind(org.target_id)
    .bind(serde_json::json!({"parameter": "username"}))
    .execute(pool)
    .await
    .expect("insert formulaic observation seed");
    sqlx::query(
        r#"INSERT INTO attack_candidate_work_items (
               id,seed_id,wave_unit_id,operation_id,scope_snapshot_id,organization_id,
               target_live_id,target_type_at_time,target_value_at_time,
               target_identity_hash,work_item_key
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,'url','https://shared.example.test/login',
               'sha256:shared-target',$8
           )"#,
    )
    .bind(work_item_id)
    .bind(seed_id)
    .bind(org.wave_unit_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(org.organization_id)
    .bind(org.target_id)
    .bind(format!("seed:{seed_id}:v1:sha256:observation"))
    .execute(pool)
    .await
    .expect("insert candidate reasoning work item");
    let mut acceptance_tx = pool
        .begin()
        .await
        .expect("begin candidate/work-item terminalization transaction");
    sqlx::query(
        r#"INSERT INTO attack_candidates (
               candidate_id,operation_id,organization_id,target,hypothesis,
               hypothesis_hash,technique,rationale,prior_refs,suggested_approach,
               priority,wave,disposition,operation_uuid,scope_snapshot_id,
               wave_run_id,wave_unit_id,source_work_item_id,
               decision_stage_execution_id,decision_stage_run_unit_id,
               decision_deliverable_submission_id,decision_stage_kind,
               target_live_id,target_type_at_time,target_value_at_time,
               target_identity_hash,execution_plan,candidate_plan_hash,risk_class
           ) VALUES (
               $1,$2,$3,'https://shared.example.test/login','SQL injection hypothesis',
               'sha256:same-hypothesis','WSTG-INPV-05','evidence grounded','[]',
               'bounded verifier','high',0,'proposed',$4,$5,$6,$7,$8,$9,$10,$11,
               'attack_candidate',$12,
               'url','https://shared.example.test/login','sha256:shared-target',$13,
               'sha256:candidate-plan','exploit'
           )"#,
    )
    .bind(candidate_id)
    .bind(fixture.operation_id.to_string())
    .bind(org.organization_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.wave_run_id)
    .bind(org.wave_unit_id)
    .bind(work_item_id)
    .bind(fixture.stage_execution_id)
    .bind(org.stage_run_unit_id)
    .bind(org.submission_id)
    .bind(org.target_id)
    .bind(serde_json::json!({
        "schema_version": "candidate-plan-v1",
        "classifier_version": "candidate-classifier-v1",
        "candidate_id": candidate_id,
        "target_identity_hash": "sha256:shared-target",
        "foreground_only": true,
        "actions": [{
            "ordinal": 0,
            "capability_id": "verify.sql_injection",
            "action_kind": "bounded_sql_injection_probe",
            "canonical_args": {"target": "https://shared.example.test/login"},
            "side_effect_class": "exploit",
            "required_evidence_role": "proof"
        }],
        "budget": {"max_actions": 1, "max_requests": 8, "max_runtime_ms": 120000}
    }))
    .execute(&mut *acceptance_tx)
    .await
    .expect("accept candidate after final gate pass");
    sqlx::query(
        r#"UPDATE attack_candidate_work_items
           SET decision_kind='candidate',candidate_id=$2,decided_at=NOW()
           WHERE id=$1"#,
    )
    .bind(work_item_id)
    .bind(candidate_id)
    .execute(&mut *acceptance_tx)
    .await
    .expect("terminalize work item as candidate");
    acceptance_tx
        .commit()
        .await
        .expect("commit candidate/work-item terminalization");
    CandidateFixture {
        seed_id,
        work_item_id,
        candidate_id,
    }
}

async fn seed_pending_work_item(
    pool: &PgPool,
    fixture: &AttackFixture,
    org: OrgFixture,
    suffix: &str,
) -> CandidateFixture {
    let seed_id = Uuid::new_v4();
    let work_item_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO attack_candidate_seeds (
               id,wave_unit_id,operation_id,scope_snapshot_id,organization_id,
               target_live_id,target_type_at_time,target_value_at_time,
               target_identity_hash,technique,observation,observation_hash
           ) VALUES (
               $1,$2,$3,$4,$5,$6,'url','https://shared.example.test/login',
               'sha256:shared-target','WSTG-INPV-05',$7,$8
           )"#,
    )
    .bind(seed_id)
    .bind(org.wave_unit_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(org.organization_id)
    .bind(org.target_id)
    .bind(serde_json::json!({"fixture": suffix}))
    .bind(format!("sha256:observation-{suffix}"))
    .execute(pool)
    .await
    .expect("insert pending formulaic seed");
    sqlx::query(
        r#"INSERT INTO attack_candidate_work_items (
               id,seed_id,wave_unit_id,operation_id,scope_snapshot_id,organization_id,
               target_live_id,target_type_at_time,target_value_at_time,
               target_identity_hash,work_item_key
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,'url','https://shared.example.test/login',
               'sha256:shared-target',$8
           )"#,
    )
    .bind(work_item_id)
    .bind(seed_id)
    .bind(org.wave_unit_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(org.organization_id)
    .bind(org.target_id)
    .bind(format!("fixture:{suffix}"))
    .execute(pool)
    .await
    .expect("insert pending reasoning work item");
    CandidateFixture {
        seed_id,
        work_item_id,
        candidate_id: Uuid::new_v4(),
    }
}

async fn mark_candidate_wave_review_ready(pool: &PgPool, fixture: &AttackFixture) {
    sqlx::query(
        "UPDATE attack_wave_units SET status='review',updated_at=NOW()
         WHERE operation_id=$1 AND scope_snapshot_id=$2 AND wave_run_id=$3
           AND status IN ('open','reasoning')",
    )
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.wave_run_id)
    .execute(pool)
    .await
    .expect("simulate every Candidate WaveUnit reaching durable review");
}

async fn insert_approval(
    pool: &PgPool,
    fixture: &AttackFixture,
    candidate: CandidateFixture,
    owner: OrgFixture,
) -> Result<Uuid, sqlx::Error> {
    let approval_id = Uuid::new_v4();
    let operator_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM operator_principals WHERE principal_kind='local_operator' AND active",
    )
    .fetch_one(pool)
    .await?;
    sqlx::query(
        r#"INSERT INTO attack_candidate_approvals (
               id,candidate_id,operation_id,scope_snapshot_id,wave_run_id,wave_unit_id,
               organization_id,target_live_id,target_type_at_time,target_value_at_time,
               target_identity_hash,candidate_plan_hash,source_work_item_id,execution_plan,
               allowed_capability_ids,allowed_action_kinds,budget,expires_at,
               decision_version,status,decided_by
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,'url','https://shared.example.test/login',
               'sha256:shared-target','sha256:candidate-plan',$9,$10,$11,$12,$13,
               NOW()+INTERVAL '1 hour',1,'approved',$14
           )"#,
    )
    .bind(approval_id)
    .bind(candidate.candidate_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.wave_run_id)
    .bind(owner.wave_unit_id)
    .bind(owner.organization_id)
    .bind(owner.target_id)
    .bind(candidate.work_item_id)
    .bind(serde_json::json!({"schema_version": "candidate-plan-v1"}))
    .bind(vec!["verify.sql_injection"])
    .bind(vec!["bounded_sql_injection_probe"])
    .bind(serde_json::json!({"max_actions": 1, "max_requests": 8}))
    .bind(operator_id)
    .execute(pool)
    .await?;
    Ok(approval_id)
}

async fn insert_attempt(
    pool: &PgPool,
    fixture: &AttackFixture,
    candidate: CandidateFixture,
    approval_id: Uuid,
    owner: OrgFixture,
    status: &str,
) -> Result<Uuid, sqlx::Error> {
    insert_attempt_with_ordinal(pool, fixture, candidate, approval_id, owner, status, 0).await
}

async fn insert_attempt_with_ordinal(
    pool: &PgPool,
    fixture: &AttackFixture,
    candidate: CandidateFixture,
    approval_id: Uuid,
    owner: OrgFixture,
    status: &str,
    ordinal: i32,
) -> Result<Uuid, sqlx::Error> {
    let attempt_id = Uuid::new_v4();
    let terminal = matches!(status, "verified" | "refuted" | "blocked" | "abandoned");
    let verification_execution_id = Uuid::new_v4();
    let verification_unit_id = Uuid::new_v4();
    let verification_worker_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO stage_runs(id,operation_id,stage_kind,status)
           VALUES ($1,$2,'verification','started')"#,
    )
    .bind(verification_execution_id)
    .bind(fixture.operation_id)
    .execute(pool)
    .await?;
    sqlx::query(
        r#"INSERT INTO stage_run_units (
               id,operation_id,stage_execution_id,scope_snapshot_id,
               organization_id,stage_kind,generation,specialist,status
           ) VALUES ($1,$2,$3,$4,$5,'verification',0,'candidate_verifier','running')"#,
    )
    .bind(verification_unit_id)
    .bind(fixture.operation_id)
    .bind(verification_execution_id)
    .bind(fixture.scope_snapshot_id)
    .bind(owner.organization_id)
    .execute(pool)
    .await?;
    sqlx::query(
        r#"INSERT INTO stage_worker_runs (
               id,operation_id,stage_execution_id,stage_run_unit_id,
               organization_id,worker_generation,specialist,work_item_kind,
               work_item_key,agent_path,status
           ) VALUES (
               $1,$2,$3,$4,$5,0,'candidate_verifier','candidate_attempt',
               $6,$7,'queued'
           )"#,
    )
    .bind(verification_worker_id)
    .bind(fixture.operation_id)
    .bind(verification_execution_id)
    .bind(verification_unit_id)
    .bind(owner.organization_id)
    .bind(attempt_id.to_string())
    .bind(format!("main>candidate_verifier:{attempt_id}"))
    .execute(pool)
    .await?;
    sqlx::query(
        r#"INSERT INTO candidate_attempts (
               id,candidate_id,approval_id,operation_id,scope_snapshot_id,
               wave_run_id,wave_unit_id,organization_id,target_live_id,
               target_type_at_time,target_value_at_time,target_identity_hash,
               candidate_plan_hash,ordinal,status,stage_worker_run_id,
               result_json,result_hash,terminal_at
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,$9,'url',
               'https://shared.example.test/login','sha256:shared-target',
               'sha256:candidate-plan',$10,$11,$12,$13,$14,
               CASE WHEN $15 THEN NOW() ELSE NULL END
           )"#,
    )
    .bind(attempt_id)
    .bind(candidate.candidate_id)
    .bind(approval_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.wave_run_id)
    .bind(owner.wave_unit_id)
    .bind(owner.organization_id)
    .bind(owner.target_id)
    .bind(ordinal)
    .bind(status)
    .bind(verification_worker_id)
    .bind(terminal.then(|| serde_json::json!({"disposition": status})))
    .bind(terminal.then_some("sha256:attempt-result"))
    .bind(terminal)
    .execute(pool)
    .await?;
    Ok(attempt_id)
}

async fn seed_verification_unit(
    pool: &PgPool,
    fixture: &AttackFixture,
    owner: OrgFixture,
) -> (Uuid, Uuid) {
    let stage_execution_id = Uuid::new_v4();
    let stage_run_unit_id = Uuid::new_v4();
    sqlx::query("UPDATE attack_wave_runs SET status='verification',updated_at=NOW() WHERE id=$1")
        .bind(fixture.wave_run_id)
        .execute(pool)
        .await
        .expect("advance exact WaveRun to verification");
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status)
         VALUES($1,$2,'verification','started')",
    )
    .bind(stage_execution_id)
    .bind(fixture.operation_id)
    .execute(pool)
    .await
    .expect("insert verification StageRun");
    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,organization_id,
               stage_kind,generation,specialist,status)
           VALUES($1,$2,$3,$4,$5,'verification',0,'candidate_verifier','running')"#,
    )
    .bind(stage_run_unit_id)
    .bind(fixture.operation_id)
    .bind(stage_execution_id)
    .bind(fixture.scope_snapshot_id)
    .bind(owner.organization_id)
    .execute(pool)
    .await
    .expect("insert verification StageRunUnit");
    (stage_execution_id, stage_run_unit_id)
}

async fn insert_audit(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
    target_id: Uuid,
    audit_role: &str,
) -> i64 {
    sqlx::query_scalar(
        r#"INSERT INTO audit_log (
               action,category,details,project_path,audit_role,run_id,target_id,detail
           ) VALUES (
               'candidate evidence','attack','','/tmp/attack-v2',$1,$2,$3,$4
           ) RETURNING id"#,
    )
    .bind(audit_role)
    .bind(operation_id)
    .bind(target_id)
    .bind(serde_json::json!({"organization_id": organization_id}))
    .fetch_one(pool)
    .await
    .expect("insert audit fixture")
}

#[tokio::test]
#[serial]
async fn v2_same_candidate_hash_is_isolated_by_frozen_org() {
    let (mut db, _data_dir) = migrated_db("org_identity").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    seed_candidate(db.pool(), &fixture, fixture.org_b).await;

    let count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM attack_candidates
           WHERE operation_uuid=$1 AND target_identity_hash='sha256:shared-target'
             AND hypothesis_hash='sha256:same-hypothesis'"#,
    )
    .bind(fixture.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("count sibling candidates");
    assert_eq!(count, 2);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn deleting_live_org_and_target_retains_attack_audit_rows_and_nulls_live_target_ref() {
    let (mut db, _data_dir) = migrated_db("retention").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let candidate = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    let evidence_id = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_a.organization_id,
        fixture.org_a.target_id,
        "evidence",
    )
    .await;
    sqlx::query(
        "INSERT INTO attack_candidate_evidence(candidate_id,evidence_id,role) VALUES ($1,$2,'support')",
    )
    .bind(candidate.candidate_id)
    .bind(evidence_id)
    .execute(db.pool())
    .await
    .expect("link candidate evidence");
    sqlx::query(
        "INSERT INTO attack_candidate_seed_evidence(seed_id,evidence_id,role) VALUES ($1,$2,'observation')",
    )
    .bind(candidate.seed_id)
    .bind(evidence_id)
    .execute(db.pool())
    .await
    .expect("link seed evidence");
    sqlx::query(
        "INSERT INTO attack_candidate_work_item_evidence(work_item_id,evidence_id,role) VALUES ($1,$2,'support')",
    )
    .bind(candidate.work_item_id)
    .bind(evidence_id)
    .execute(db.pool())
    .await
    .expect("link work-item evidence");
    let approval_id = insert_approval(db.pool(), &fixture, candidate, fixture.org_a)
        .await
        .expect("insert retained approval");
    let attempt_id = insert_attempt(
        db.pool(),
        &fixture,
        candidate,
        approval_id,
        fixture.org_a,
        "running",
    )
    .await
    .expect("insert retained running attempt");
    sqlx::query(
        "INSERT INTO candidate_attempt_evidence(attempt_id,evidence_id,role) VALUES ($1,$2,'proof')",
    )
    .bind(attempt_id)
    .bind(evidence_id)
    .execute(db.pool())
    .await
    .expect("link attempt proof evidence");
    sqlx::query(
        "UPDATE candidate_attempts SET status='verified',
             result_json=$2,result_hash='sha256:verified',terminal_at=NOW()
         WHERE id=$1",
    )
    .bind(attempt_id)
    .bind(serde_json::json!({"disposition": "verified"}))
    .execute(db.pool())
    .await
    .expect("terminalize retained Attempt after freezing proof membership");
    let finding_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO findings (
               id,title,sev,target,description,steps,remediation,project_path,target_id,
               source,evidence
           ) VALUES (
               $1,'Verified SQL injection','high','https://shared.example.test/login',
               'verified','replay evidence','parameterize','/tmp/attack-v2',$2,
               'candidate_v2',$3
           )"#,
    )
    .bind(finding_id)
    .bind(fixture.org_a.target_id)
    .bind(serde_json::json!([evidence_id]))
    .execute(db.pool())
    .await
    .expect("insert retained finding");
    sqlx::query(
        r#"INSERT INTO finding_lineage (
               id,finding_id,candidate_attempt_id,candidate_id,operation_id,
               scope_snapshot_id,wave_run_id,wave_unit_id,organization_id,
               target_live_id,target_type_at_time,target_value_at_time,
               target_identity_hash,candidate_plan_hash
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'url',
               'https://shared.example.test/login','sha256:shared-target',
               'sha256:candidate-plan'
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(finding_id)
    .bind(attempt_id)
    .bind(candidate.candidate_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.wave_run_id)
    .bind(fixture.org_a.wave_unit_id)
    .bind(fixture.org_a.organization_id)
    .bind(fixture.org_a.target_id)
    .execute(db.pool())
    .await
    .expect("insert retained finding lineage");
    let fact_delta_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO attack_fact_deltas (
               id,source_attempt_id,candidate_id,operation_id,scope_snapshot_id,
               wave_run_id,wave_unit_id,organization_id,target_live_id,
               target_type_at_time,target_value_at_time,target_identity_hash,
               candidate_plan_hash,canonical_ref_kind,canonical_ref_id,
               canonical_ref_version,canonical_ref_hash,delta_kind,dedupe_hash,status
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,$9,'url',
               'https://shared.example.test/login','sha256:shared-target',
               'sha256:candidate-plan','web_origin',$10,1,'sha256:canonical-ref',
               'new_parameter','sha256:delta','accepted'
           )"#,
    )
    .bind(fact_delta_id)
    .bind(attempt_id)
    .bind(candidate.candidate_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.wave_run_id)
    .bind(fixture.org_a.wave_unit_id)
    .bind(fixture.org_a.organization_id)
    .bind(fixture.org_a.target_id)
    .bind(Uuid::new_v4())
    .execute(db.pool())
    .await
    .expect("insert retained fact delta");
    sqlx::query(
        "INSERT INTO attack_fact_delta_evidence(fact_delta_id,evidence_id,role) VALUES ($1,$2,'fact_delta')",
    )
    .bind(fact_delta_id)
    .bind(evidence_id)
    .execute(db.pool())
    .await
    .expect("link fact delta evidence");
    let residual_risk_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO attack_residual_risks (
               id,operation_id,scope_snapshot_id,wave_run_id,wave_unit_id,
               organization_id,target_live_id,target_type_at_time,target_value_at_time,
               target_identity_hash,reason_code,reason_detail,policy_hash,
               wave_count,candidate_count,chain_depth,attempt_count
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,'url','https://shared.example.test/login',
               'sha256:shared-target','attempt_cap','cap reached','sha256:policy',1,1,0,1
           )"#,
    )
    .bind(residual_risk_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.wave_run_id)
    .bind(fixture.org_a.wave_unit_id)
    .bind(fixture.org_a.organization_id)
    .bind(fixture.org_a.target_id)
    .execute(db.pool())
    .await
    .expect("insert retained residual risk");
    sqlx::query(
        "INSERT INTO attack_residual_risk_evidence(residual_risk_id,evidence_id,role) VALUES ($1,$2,'residual')",
    )
    .bind(residual_risk_id)
    .bind(evidence_id)
    .execute(db.pool())
    .await
    .expect("link residual risk evidence");
    sqlx::query(
        r#"UPDATE attack_candidates
           SET disposition='verified',terminal_attempt_id=$2,terminal_finding_id=$3
           WHERE candidate_id=$1"#,
    )
    .bind(candidate.candidate_id)
    .bind(attempt_id)
    .bind(finding_id)
    .execute(db.pool())
    .await
    .expect("terminalize candidate lineage");

    let attempt_row_version_before_delete: i64 =
        sqlx::query_scalar("SELECT row_version FROM candidate_attempts WHERE id=$1")
            .bind(attempt_id)
            .fetch_one(db.pool())
            .await
            .expect("read retained Attempt source version before live target deletion");
    let attempt_canonical_row_before_delete: serde_json::Value = sqlx::query_scalar(
        "SELECT to_jsonb(attempt) - 'target_live_id' \
         FROM candidate_attempts AS attempt WHERE id=$1",
    )
    .bind(attempt_id)
    .fetch_one(db.pool())
    .await
    .expect("read retained Attempt canonical row before live target deletion");

    let direct_attempt_live_pointer_tamper =
        sqlx::query("UPDATE candidate_attempts SET target_live_id=NULL WHERE id=$1")
            .bind(attempt_id)
            .execute(db.pool())
            .await;
    assert_sqlstate(
        direct_attempt_live_pointer_tamper.map(|_| ()),
        "23514",
        "direct terminal Attempt live target pointer tamper",
    );

    let direct_live_pointer_tamper = sqlx::query("UPDATE findings SET target_id=NULL WHERE id=$1")
        .bind(finding_id)
        .execute(db.pool())
        .await;
    assert_sqlstate(
        direct_live_pointer_tamper.map(|_| ()),
        "P0001",
        "direct lineage-bound Finding target pointer tamper",
    );

    sqlx::query("DELETE FROM organizations WHERE id=$1")
        .bind(fixture.org_a.organization_id)
        .execute(db.pool())
        .await
        .expect("delete live organization after application-level invalidation fixture");

    for (table, id_column, id) in [
        ("attack_candidate_seeds", "id", candidate.seed_id),
        ("attack_candidate_work_items", "id", candidate.work_item_id),
        ("attack_candidates", "candidate_id", candidate.candidate_id),
        ("attack_candidate_approvals", "id", approval_id),
        ("candidate_attempts", "id", attempt_id),
        ("attack_fact_deltas", "id", fact_delta_id),
        ("attack_residual_risks", "id", residual_risk_id),
    ] {
        let retained: bool = sqlx::query_scalar(&format!(
            "SELECT EXISTS(SELECT 1 FROM {table} WHERE {id_column}=$1)"
        ))
        .bind(id)
        .fetch_one(db.pool())
        .await
        .expect("read retained attack row");
        assert!(retained, "{table} must survive live organization deletion");
    }
    for (table, id_column, id) in [
        ("attack_candidate_seeds", "id", candidate.seed_id),
        ("attack_candidate_work_items", "id", candidate.work_item_id),
        ("attack_candidates", "candidate_id", candidate.candidate_id),
        ("attack_candidate_approvals", "id", approval_id),
        ("candidate_attempts", "id", attempt_id),
        ("finding_lineage", "candidate_attempt_id", attempt_id),
        ("attack_fact_deltas", "id", fact_delta_id),
        ("attack_residual_risks", "id", residual_risk_id),
    ] {
        let live_target: Option<Uuid> = sqlx::query_scalar(&format!(
            "SELECT target_live_id FROM {table} WHERE {id_column}=$1"
        ))
        .bind(id)
        .fetch_one(db.pool())
        .await
        .expect("read nulled live target reference");
        assert_eq!(live_target, None, "{table}.target_live_id must be nulled");
    }
    let attempt_row_version_after_delete: i64 =
        sqlx::query_scalar("SELECT row_version FROM candidate_attempts WHERE id=$1")
            .bind(attempt_id)
            .fetch_one(db.pool())
            .await
            .expect("read retained Attempt source version after live target deletion");
    assert_eq!(
        attempt_row_version_after_delete, attempt_row_version_before_delete,
        "nulling a non-canonical live pointer must not advance the frozen source version"
    );
    let attempt_canonical_row_after_delete: serde_json::Value = sqlx::query_scalar(
        "SELECT to_jsonb(attempt) - 'target_live_id' \
         FROM candidate_attempts AS attempt WHERE id=$1",
    )
    .bind(attempt_id)
    .fetch_one(db.pool())
    .await
    .expect("read retained Attempt canonical row after live target deletion");
    assert_eq!(
        attempt_canonical_row_after_delete, attempt_canonical_row_before_delete,
        "live target deletion must not alter any canonical Attempt field"
    );
    let audit_target: Option<Uuid> =
        sqlx::query_scalar("SELECT target_id FROM audit_log WHERE id=$1")
            .bind(evidence_id)
            .fetch_one(db.pool())
            .await
            .expect("read retained audit evidence");
    assert_eq!(audit_target, None);
    let finding_target: Option<Uuid> =
        sqlx::query_scalar("SELECT target_id FROM findings WHERE id=$1")
            .bind(finding_id)
            .fetch_one(db.pool())
            .await
            .expect("read retained finding");
    assert_eq!(finding_target, None);
    let evidence_link_retained: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM attack_candidate_evidence WHERE candidate_id=$1 AND evidence_id=$2)",
    )
    .bind(candidate.candidate_id)
    .bind(evidence_id)
    .fetch_one(db.pool())
    .await
    .expect("read retained evidence link");
    assert!(evidence_link_retained);
    let retained_join_count: i64 = sqlx::query_scalar(
        r#"SELECT
               (SELECT COUNT(*) FROM attack_candidate_seed_evidence WHERE evidence_id=$1)
             + (SELECT COUNT(*) FROM attack_candidate_work_item_evidence WHERE evidence_id=$1)
             + (SELECT COUNT(*) FROM attack_candidate_evidence WHERE evidence_id=$1)
             + (SELECT COUNT(*) FROM candidate_attempt_evidence WHERE evidence_id=$1)
             + (SELECT COUNT(*) FROM attack_fact_delta_evidence WHERE evidence_id=$1)
             + (SELECT COUNT(*) FROM attack_residual_risk_evidence WHERE evidence_id=$1)"#,
    )
    .bind(evidence_id)
    .fetch_one(db.pool())
    .await
    .expect("count retained relational evidence links");
    assert_eq!(retained_join_count, 6);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn hostile_sibling_approval_attempt_and_lineage_inserts_fail_in_db() {
    let (mut db, _data_dir) = migrated_db("hostile_sibling").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let candidate_a = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    seed_candidate(db.pool(), &fixture, fixture.org_b).await;

    assert_sqlstate(
        insert_approval(db.pool(), &fixture, candidate_a, fixture.org_b)
            .await
            .map(|_| ()),
        "23503",
        "sibling approval",
    );
    let approval_a = insert_approval(db.pool(), &fixture, candidate_a, fixture.org_a)
        .await
        .expect("insert correctly owned approval");
    assert_sqlstate(
        insert_attempt(
            db.pool(),
            &fixture,
            candidate_a,
            approval_a,
            fixture.org_b,
            "running",
        )
        .await
        .map(|_| ()),
        "P0001",
        "sibling attempt",
    );
    let attempt_a = insert_attempt(
        db.pool(),
        &fixture,
        candidate_a,
        approval_a,
        fixture.org_a,
        "verified",
    )
    .await
    .expect("insert correctly owned attempt");
    let finding_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO findings (id,title,sev,project_path,target_id)
           VALUES ($1,'hostile lineage fixture','high','/tmp/attack-v2',$2)"#,
    )
    .bind(finding_id)
    .bind(fixture.org_b.target_id)
    .execute(db.pool())
    .await
    .expect("insert sibling finding fixture");
    let hostile_lineage = sqlx::query(
        r#"INSERT INTO finding_lineage (
               id,finding_id,candidate_attempt_id,candidate_id,operation_id,
               scope_snapshot_id,wave_run_id,wave_unit_id,organization_id,
               target_live_id,target_type_at_time,target_value_at_time,
               target_identity_hash,candidate_plan_hash
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'url',
               'https://shared.example.test/login','sha256:shared-target',
               'sha256:candidate-plan'
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(finding_id)
    .bind(attempt_a)
    .bind(candidate_a.candidate_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.wave_run_id)
    .bind(fixture.org_b.wave_unit_id)
    .bind(fixture.org_b.organization_id)
    .bind(fixture.org_b.target_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(hostile_lineage.map(|_| ()), "P0001", "sibling lineage");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn foreign_or_non_evidence_audit_id_cannot_be_linked() {
    let (mut db, _data_dir) = migrated_db("evidence_owner").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let candidate = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    let foreign_evidence = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_b.organization_id,
        fixture.org_b.target_id,
        "evidence",
    )
    .await;
    let non_evidence = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_a.organization_id,
        fixture.org_a.target_id,
        "action",
    )
    .await;
    let valid_evidence = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_a.organization_id,
        fixture.org_a.target_id,
        "evidence",
    )
    .await;

    for (evidence_id, context) in [
        (foreign_evidence, "foreign organization evidence"),
        (non_evidence, "non-evidence audit row"),
    ] {
        let linked = sqlx::query(
            "INSERT INTO attack_candidate_seed_evidence(seed_id,evidence_id,role) VALUES ($1,$2,'observation')",
        )
        .bind(candidate.seed_id)
        .bind(evidence_id)
        .execute(db.pool())
        .await;
        assert_sqlstate(linked.map(|_| ()), "P0001", context);
    }
    sqlx::query(
        "INSERT INTO attack_candidate_seed_evidence(seed_id,evidence_id,role) VALUES ($1,$2,'observation')",
    )
    .bind(candidate.seed_id)
    .bind(valid_evidence)
    .execute(db.pool())
    .await
    .expect("link same-operation same-org evidence");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn v2_contract_requires_runtime_memory_v2_and_is_immutable() {
    let (mut db, _data_dir) = migrated_db("contract").await;
    let invalid_operation = Uuid::new_v4();
    let invalid = sqlx::query(
        r#"INSERT INTO operation_state (
               operation_id,profile,current_stage,runtime_memory_contract,
               attack_execution_contract
           ) VALUES ($1,'red_team','scoping','dual_write_v2_preferred','v2_only')"#,
    )
    .bind(invalid_operation)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        invalid.map(|_| ()),
        "23514",
        "v2 attack contract on non-v2 runtime memory",
    );

    let valid_operation = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO operation_state (
               operation_id,profile,current_stage,runtime_memory_contract,
               attack_execution_contract
           ) VALUES ($1,'red_team','scoping','v2_only','v2_only')"#,
    )
    .bind(valid_operation)
    .execute(db.pool())
    .await
    .expect("insert compatible frozen contracts");
    let changed = sqlx::query(
        "UPDATE operation_state SET attack_execution_contract='legacy' WHERE operation_id=$1",
    )
    .bind(valid_operation)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        changed.map(|_| ()),
        "P0001",
        "immutable operation attack contract",
    );

    let skipped = sqlx::query(
        r#"UPDATE attack_execution_rollout
           SET contract='dual_write_read_v2_fallback',rank=2,row_version=1
           WHERE singleton=TRUE"#,
    )
    .execute(db.pool())
    .await;
    assert_sqlstate(skipped.map(|_| ()), "P0001", "skipped rollout rank");
    sqlx::query(
        r#"UPDATE attack_execution_rollout
           SET contract='dual_write_read_legacy',rank=1,row_version=1,updated_at=NOW()
           WHERE singleton=TRUE"#,
    )
    .execute(db.pool())
    .await
    .expect("advance rollout one state");
    let stale = sqlx::query(
        r#"UPDATE attack_execution_rollout
           SET contract='dual_write_read_v2_fallback',rank=2,row_version=3
           WHERE singleton=TRUE"#,
    )
    .execute(db.pool())
    .await;
    assert_sqlstate(stale.map(|_| ()), "P0001", "stale rollout row version");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn attack_execution_schema_is_relational_and_attempt_does_not_own_recovery() {
    let (mut db, _data_dir) = migrated_db("schema_shape").await;
    for table in [
        "attack_execution_rollout",
        "attack_wave_runs",
        "attack_wave_units",
        "attack_candidate_seeds",
        "attack_candidate_work_items",
        "attack_candidate_approvals",
        "candidate_attempts",
        "candidate_attempt_actions",
        "candidate_review_barriers",
        "attack_execution_lanes",
        "finding_lineage",
        "attack_fact_deltas",
        "attack_residual_risks",
        "candidate_attempt_evidence",
    ] {
        let exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
            .bind(format!("public.{table}"))
            .fetch_one(db.pool())
            .await
            .expect("inspect attack execution table");
        assert!(exists, "missing attack execution table {table}");
    }

    let evidence_array_columns: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
           FROM information_schema.columns
           WHERE table_schema='public'
             AND table_name IN (
                 'attack_candidate_seed_evidence',
                 'attack_candidate_work_item_evidence',
                 'attack_candidate_evidence',
                 'candidate_attempt_evidence',
                 'attack_fact_delta_evidence',
                 'attack_residual_risk_evidence'
             )
             AND data_type='ARRAY'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect evidence join column types");
    assert_eq!(
        evidence_array_columns, 0,
        "evidence IDs must be relational joins"
    );

    let attempt_recovery_columns: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
           FROM information_schema.columns
           WHERE table_schema='public' AND table_name='candidate_attempts'
             AND column_name IN (
                 'lease_token','lease_owner','lease_expires_at','heartbeat_at',
                 'checkpoint','checkpoint_version','background_job_id'
             )"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect attempt ownership columns");
    assert_eq!(
        attempt_recovery_columns, 0,
        "P1 WorkerRun owns recovery state"
    );

    let evidence_owner_triggers: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM pg_trigger
           WHERE NOT tgisinternal AND tgname IN (
               'attack_candidate_seed_evidence_owner',
               'attack_candidate_work_item_evidence_owner',
               'attack_candidate_evidence_owner',
               'candidate_attempt_evidence_owner',
               'attack_fact_delta_evidence_owner',
               'attack_residual_risk_evidence_owner'
           )"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect evidence ownership triggers");
    assert_eq!(evidence_owner_triggers, 6);

    let live_organization_fks: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
           FROM pg_constraint AS constraint_row
           WHERE constraint_row.contype='f'
             AND constraint_row.confrelid='organizations'::regclass
             AND constraint_row.conrelid::regclass::text IN (
                 'attack_candidates',
                 'attack_candidate_seeds',
                 'attack_candidate_work_items',
                 'attack_candidate_approvals',
                 'candidate_attempts',
                 'finding_lineage',
                 'attack_fact_deltas',
                 'attack_residual_risks'
             )"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect retained organization references");
    assert_eq!(
        live_organization_fks, 0,
        "frozen organization UUIDs are retained facts"
    );

    let partial_indexes: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM pg_indexes
           WHERE schemaname='public'
             AND indexname IN (
                 'uq_attack_candidates_legacy_op_target_hash',
                 'uq_attack_candidates_v2_identity'
             )
             AND indexdef LIKE '% WHERE %operation_uuid IS %NULL%'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect Candidate partial indexes");
    assert_eq!(partial_indexes, 2);
    let old_index_exists: bool =
        sqlx::query_scalar("SELECT to_regclass('uq_attack_candidates_op_target_hash') IS NOT NULL")
            .fetch_one(db.pool())
            .await
            .expect("inspect removed legacy global index");
    assert!(!old_index_exists);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn accept_candidate_batch_requires_final_pass_and_complete_manifest() {
    let (mut db, _data_dir) = migrated_db("accept_manifest").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let evidence_id: i64 = sqlx::query_scalar(
        "SELECT evidence_ids[1] FROM stage_handoffs WHERE source_stage_run_unit_id=$1",
    )
    .bind(fixture.org_a.entry_stage_run_unit_id)
    .fetch_one(db.pool())
    .await
    .expect("load exact predecessor evidence");
    let mut seed_tx = db.pool().begin().await.expect("begin exact manifest seed");
    let seeded = golish_db::repo::attack_candidate_work_items::seed_wave_work_items(
        &mut seed_tx,
        golish_db::repo::attack_candidate_work_items::SeedAttackWorkItems {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_a.wave_unit_id,
            organization_id: fixture.org_a.organization_id,
            observations: vec![
                golish_db::repo::attack_candidate_work_items::SeedAttackObservation {
                    work_item_key: "formulaic:candidate".to_string(),
                    target_live_id: Some(fixture.org_a.target_id),
                    target_type_at_time: "url".to_string(),
                    target_value_at_time: "https://shared.example.test/login".to_string(),
                    target_identity_hash: "sha256:candidate-target".to_string(),
                    technique: "WSTG-INPV-05".to_string(),
                    observation: serde_json::json!({"outcome": "found"}),
                    observation_hash: "sha256:candidate-observation".to_string(),
                    evidence_ids: vec![evidence_id],
                },
                golish_db::repo::attack_candidate_work_items::SeedAttackObservation {
                    work_item_key: "formulaic:checked-empty".to_string(),
                    target_live_id: Some(fixture.org_a.target_id),
                    target_type_at_time: "url".to_string(),
                    target_value_at_time: "https://shared.example.test/login".to_string(),
                    target_identity_hash: "sha256:checked-empty-target".to_string(),
                    technique: "WSTG-INPV-01".to_string(),
                    observation: serde_json::json!({"outcome": "empty"}),
                    observation_hash: "sha256:checked-empty-observation".to_string(),
                    evidence_ids: vec![evidence_id],
                },
            ],
        },
    )
    .await
    .expect("seed and freeze exact Candidate manifest");
    seed_tx.commit().await.expect("commit exact manifest seed");
    let candidate_item = CandidateFixture {
        seed_id: seeded.items[0].seed.id,
        work_item_id: seeded.items[0].work_item.id,
        candidate_id: Uuid::new_v4(),
    };
    let no_candidate_item = CandidateFixture {
        seed_id: seeded.items[1].seed.id,
        work_item_id: seeded.items[1].work_item.id,
        candidate_id: Uuid::new_v4(),
    };
    let execution_plan = serde_json::json!({
        "schema_version": "candidate-plan-v1",
        "foreground_only": true,
        "actions": [{"ordinal": 0, "capability_id": "verify.sql_injection"}]
    });
    let draft = AcceptedCandidateDraft {
        candidate_id: candidate_item.candidate_id,
        work_item_id: candidate_item.work_item_id,
        hypothesis: "bounded SQL injection hypothesis".to_string(),
        technique: Some("WSTG-INPV-05".to_string()),
        rationale: "grounded by formulaic observation".to_string(),
        prior_refs: vec![format!("audit:{evidence_id}")],
        suggested_approach: "run immutable verifier plan".to_string(),
        priority: "high".to_string(),
        candidate_plan_hash: canonical_execution_plan_hash(&execution_plan)
            .expect("hash canonical execution plan"),
        execution_plan,
        risk_class: "exploit".to_string(),
        evidence_ids: vec![evidence_id],
    };
    let no_candidate = NoCandidateDecision {
        work_item_id: no_candidate_item.work_item_id,
        reason_code: "checked_empty".to_string(),
        detail: "bounded formulaic check produced no actionable hypothesis".to_string(),
        evidence_ids: vec![evidence_id],
    };
    let manifest = golish_db::repo::attack_candidate_work_items::load_for_wave_unit(
        db.pool(),
        fixture.operation_id,
        fixture.scope_snapshot_id,
        fixture.wave_run_id,
        fixture.org_a.wave_unit_id,
        fixture.org_a.organization_id,
    )
    .await
    .expect("load exact Candidate manifest");
    let command = AcceptCandidateBatch {
        operation_id: fixture.operation_id,
        scope_snapshot_id: fixture.scope_snapshot_id,
        wave_run_id: fixture.wave_run_id,
        wave_unit_id: fixture.org_a.wave_unit_id,
        organization_id: fixture.org_a.organization_id,
        decision_stage_execution_id: fixture.stage_execution_id,
        decision_stage_run_unit_id: fixture.org_a.stage_run_unit_id,
        decision_deliverable_submission_id: fixture.org_a.submission_id,
        manifest_hash: golish_db::repo::attack_candidate_work_items::canonical_manifest_hash(
            &manifest,
        ),
        expected_work_item_ids: vec![candidate_item.work_item_id, no_candidate_item.work_item_id],
        candidates: vec![draft.clone()],
        no_candidate_decisions: vec![no_candidate.clone()],
    };

    let mut oversized = command.clone();
    oversized.candidates[0].rationale = "x".repeat(8193);
    let mut oversized_tx = db
        .pool()
        .begin()
        .await
        .expect("begin oversized Candidate acceptance");
    assert!(
        accept_gate_passed_candidate_batch(&mut oversized_tx, oversized)
            .await
            .is_err()
    );
    oversized_tx
        .rollback()
        .await
        .expect("rollback oversized Candidate acceptance");

    let mut unstable_reason = command.clone();
    unstable_reason.no_candidate_decisions[0].reason_code = "Checked Empty".to_string();
    let mut unstable_reason_tx = db
        .pool()
        .begin()
        .await
        .expect("begin unstable no-candidate reason");
    assert!(
        accept_gate_passed_candidate_batch(&mut unstable_reason_tx, unstable_reason)
            .await
            .is_err()
    );
    unstable_reason_tx
        .rollback()
        .await
        .expect("rollback unstable no-candidate reason");

    sqlx::query("UPDATE stage_run_units SET status='gate_blocked',terminal_at=NULL WHERE id=$1")
        .bind(fixture.org_a.stage_run_unit_id)
        .execute(db.pool())
        .await
        .expect("make source unit non-passed hostile fixture");
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin rejected gate transaction");
    assert!(accept_gate_passed_candidate_batch(&mut tx, command.clone())
        .await
        .is_err());
    tx.rollback()
        .await
        .expect("rollback rejected gate transaction");
    sqlx::query("UPDATE stage_run_units SET status='passed',terminal_at=NOW() WHERE id=$1")
        .bind(fixture.org_a.stage_run_unit_id)
        .execute(db.pool())
        .await
        .expect("restore source unit final pass");

    let mut entry_authority_is_not_decision_authority = command.clone();
    entry_authority_is_not_decision_authority.decision_stage_execution_id =
        fixture.entry_stage_execution_id;
    entry_authority_is_not_decision_authority.decision_stage_run_unit_id =
        fixture.org_a.entry_stage_run_unit_id;
    entry_authority_is_not_decision_authority.decision_deliverable_submission_id =
        fixture.org_a.entry_submission_id;
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin wrong decision authority transaction");
    assert!(
        accept_gate_passed_candidate_batch(&mut tx, entry_authority_is_not_decision_authority)
            .await
            .is_err()
    );
    tx.rollback()
        .await
        .expect("rollback wrong decision authority transaction");

    let mut incomplete = command.clone();
    incomplete.no_candidate_decisions.clear();
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin incomplete manifest transaction");
    assert!(accept_gate_passed_candidate_batch(&mut tx, incomplete)
        .await
        .is_err());
    tx.rollback()
        .await
        .expect("rollback incomplete manifest transaction");

    let mut closed_wave_tx = db
        .pool()
        .begin()
        .await
        .expect("begin closed Wave rejection");
    sqlx::query("UPDATE attack_wave_runs SET status='terminal',terminal_at=NOW() WHERE id=$1")
        .bind(fixture.wave_run_id)
        .execute(&mut *closed_wave_tx)
        .await
        .expect("close Wave in hostile transaction");
    assert!(
        accept_gate_passed_candidate_batch(&mut closed_wave_tx, command.clone())
            .await
            .is_err()
    );
    closed_wave_tx
        .rollback()
        .await
        .expect("rollback closed Wave fixture");

    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin accepted manifest transaction");
    let result = accept_gate_passed_candidate_batch(&mut tx, command.clone())
        .await
        .expect("accept complete final-gate manifest");
    tx.commit().await.expect("commit accepted manifest");
    assert_eq!(result.candidate_ids, vec![candidate_item.candidate_id]);
    assert_eq!(
        result.no_candidate_work_item_ids,
        vec![no_candidate_item.work_item_id]
    );

    let decisions: Vec<(String, Option<Uuid>)> = sqlx::query_as(
        r#"SELECT decision_kind,candidate_id FROM attack_candidate_work_items
           WHERE wave_unit_id=$1 ORDER BY work_item_key"#,
    )
    .bind(fixture.org_a.wave_unit_id)
    .fetch_all(db.pool())
    .await
    .expect("read terminal work-item manifest");
    assert_eq!(decisions.len(), 2);
    assert!(decisions
        .iter()
        .all(|(kind, _)| kind == "candidate" || kind == "no_candidate"));

    let mut replay_tx = db.pool().begin().await.expect("begin exact replay");
    let replay = accept_gate_passed_candidate_batch(&mut replay_tx, command.clone())
        .await
        .expect("exact response-loss replay is idempotent");
    replay_tx.commit().await.expect("commit exact replay");
    assert!(replay.replayed);
    assert_eq!(replay.candidate_ids, vec![candidate_item.candidate_id]);
    assert_eq!(
        replay.no_candidate_work_item_ids,
        vec![no_candidate_item.work_item_id]
    );

    let mut drifted = command;
    drifted.no_candidate_decisions[0].detail = "drifted terminal decision".to_string();
    let mut drift_tx = db.pool().begin().await.expect("begin drifted replay");
    assert!(accept_gate_passed_candidate_batch(&mut drift_tx, drifted)
        .await
        .is_err());
    drift_tx.rollback().await.expect("rollback drifted replay");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn attack_rollout_cas_and_operation_creation_freeze_exact_contract() {
    let (mut db, _data_dir) = migrated_db("rollout_repo").await;
    let mut tx = db.pool().begin().await.expect("begin rollout transaction");
    let dual = attack_execution_rollout::advance_attack_execution_rollout(
        &mut tx,
        0,
        AttackExecutionContract::DualWriteReadLegacy,
    )
    .await
    .expect("advance attack rollout one rank");
    tx.commit().await.expect("commit rollout transaction");
    assert_eq!(dual.contract, "dual_write_read_legacy");
    assert_eq!(dual.row_version, 1);

    let stale = db
        .pool()
        .begin()
        .await
        .expect("begin stale rollout transaction");
    let mut stale = stale;
    assert!(attack_execution_rollout::advance_attack_execution_rollout(
        &mut stale,
        0,
        AttackExecutionContract::DualWriteReadV2Fallback,
    )
    .await
    .is_err());
    stale
        .rollback()
        .await
        .expect("rollback stale rollout transaction");

    let operation_id = Uuid::new_v4();
    operation_state::insert(
        db.pool(),
        operation_id,
        "red_team",
        "scoping",
        "dual_write_legacy_read",
    )
    .await
    .expect("freeze current attack rollout on operation insert");
    let frozen: String = sqlx::query_scalar(
        "SELECT attack_execution_contract FROM operation_state WHERE operation_id=$1",
    )
    .bind(operation_id)
    .fetch_one(db.pool())
    .await
    .expect("read frozen attack operation contract");
    assert_eq!(frozen, "dual_write_read_legacy");

    for (expected_version, next) in [
        (1, AttackExecutionContract::DualWriteReadV2Fallback),
        (2, AttackExecutionContract::V2Only),
    ] {
        let mut tx = db
            .pool()
            .begin()
            .await
            .expect("begin adjacent rollout transaction");
        attack_execution_rollout::advance_attack_execution_rollout(&mut tx, expected_version, next)
            .await
            .expect("advance adjacent attack rollout");
        tx.commit().await.expect("commit adjacent rollout");
    }
    let invalid_operation = operation_state::insert(
        db.pool(),
        Uuid::new_v4(),
        "red_team",
        "scoping",
        "dual_write_v2_preferred",
    )
    .await;
    assert!(invalid_operation.is_err());
    let still_frozen: String = sqlx::query_scalar(
        "SELECT attack_execution_contract FROM operation_state WHERE operation_id=$1",
    )
    .bind(operation_id)
    .fetch_one(db.pool())
    .await
    .expect("re-read existing frozen operation");
    assert_eq!(still_frozen, "dual_write_read_legacy");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn review_batch_is_plan_bound_org_scoped_and_reopens_after_expiry() {
    let (mut db, _data_dir) = migrated_db("review_plan_scope_expiry").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let candidate = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    mark_candidate_wave_review_ready(db.pool(), &fixture).await;
    let decision = CandidateReviewDecision {
        candidate_id: candidate.candidate_id,
        expected_candidate_plan_hash: "sha256:candidate-plan".to_string(),
        expected_candidate_row_version: 0,
        approve: true,
        expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
    };

    let foreign_result = review_wave_candidates(
        db.pool(),
        ReviewCandidateBatch {
            operation_id: fixture.operation_id,
            wave_run_id: fixture.wave_run_id,
            decisions: vec![CandidateReviewDecision {
                candidate_id: Uuid::new_v4(),
                ..decision.clone()
            }],
        },
    )
    .await;
    assert!(foreign_result.is_err(), "sibling review must fail closed");

    let approved = review_wave_candidates(
        db.pool(),
        ReviewCandidateBatch {
            operation_id: fixture.operation_id,
            wave_run_id: fixture.wave_run_id,
            decisions: vec![decision],
        },
    )
    .await
    .expect("approve exact candidate plan");
    assert!(approved.state.review_closed);
    assert_eq!(approved.approvals.len(), 1);

    let current_operator: Uuid = sqlx::query_scalar(
        "SELECT id FROM operator_principals WHERE principal_kind='local_operator' AND active",
    )
    .fetch_one(db.pool())
    .await
    .expect("read trusted local operator");
    assert_eq!(approved.approvals[0].decided_by, current_operator);
    sqlx::query(
        "UPDATE attack_candidate_approvals SET expires_at=NOW()-INTERVAL '1 second' WHERE id=$1",
    )
    .bind(approved.approvals[0].id)
    .execute(db.pool())
    .await
    .expect("expire approval before Attempt");

    let reopened = list_candidate_reviews(db.pool(), fixture.operation_id, fixture.wave_run_id)
        .await
        .expect("expire stale approval and reopen review");
    assert!(!reopened.review_closed);
    assert_eq!(
        reopened.proposed_candidate_count, 1,
        "expired approval without an Attempt reopens the exact candidate"
    );
    let state: (String, String, bool, String) = sqlx::query_as(
        r#"SELECT candidate.disposition,approval.status,unit.review_closed,barrier.status
           FROM attack_candidates candidate
           JOIN attack_candidate_approvals approval ON approval.candidate_id=candidate.candidate_id
           JOIN attack_wave_units unit ON unit.id=candidate.wave_unit_id
           JOIN candidate_review_barriers barrier ON barrier.wave_run_id=candidate.wave_run_id
           WHERE candidate.candidate_id=$1"#,
    )
    .bind(candidate.candidate_id)
    .fetch_one(db.pool())
    .await
    .expect("read reopened review state");
    assert_eq!(
        state,
        ("proposed".into(), "expired".into(), false, "open".into())
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn review_response_loss_replays_the_exact_durable_decision() {
    let (mut db, _data_dir) = migrated_db("review_response_loss_replay").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let candidate = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    mark_candidate_wave_review_ready(db.pool(), &fixture).await;
    let command = ReviewCandidateBatch {
        operation_id: fixture.operation_id,
        wave_run_id: fixture.wave_run_id,
        decisions: vec![CandidateReviewDecision {
            candidate_id: candidate.candidate_id,
            expected_candidate_plan_hash: "sha256:candidate-plan".to_string(),
            expected_candidate_row_version: 0,
            approve: true,
            expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
        }],
    };

    let first = review_wave_candidates(db.pool(), command.clone())
        .await
        .expect("first exact review");

    let replay = review_wave_candidates(db.pool(), command)
        .await
        .expect("the exact durable decision must replay after response loss");

    assert_eq!(replay.approvals, first.approvals);
    assert!(replay.state.review_closed);
    let approval_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM attack_candidate_approvals WHERE candidate_id=$1")
            .bind(candidate.candidate_id)
            .fetch_one(db.pool())
            .await
            .expect("count replayed approvals");
    assert_eq!(
        approval_count, 1,
        "response loss must not duplicate decisions"
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn review_barrier_waits_for_the_exact_complete_wave_snapshot() {
    let (mut db, _data_dir) = migrated_db("review_exact_wave_barrier").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let candidate_a = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    let _candidate_b = seed_candidate(db.pool(), &fixture, fixture.org_b).await;
    mark_candidate_wave_review_ready(db.pool(), &fixture).await;

    let partial = review_wave_candidates(
        db.pool(),
        ReviewCandidateBatch {
            operation_id: fixture.operation_id,
            wave_run_id: fixture.wave_run_id,
            decisions: vec![CandidateReviewDecision {
                candidate_id: candidate_a.candidate_id,
                expected_candidate_plan_hash: "sha256:candidate-plan".to_string(),
                expected_candidate_row_version: 0,
                approve: false,
                expires_at: None,
            }],
        },
    )
    .await
    .expect_err("a partial sibling-org snapshot must fail closed");
    assert!(partial.to_string().contains("ATTACK_REVIEW_SCOPE_MISMATCH"));

    let state = list_candidate_reviews(db.pool(), fixture.operation_id, fixture.wave_run_id)
        .await
        .expect("read durable wave barrier");
    assert_eq!(
        state.barrier.status, "open",
        "an unreviewed sibling org keeps the exact wave DB snapshot open"
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn review_rejects_sibling_candidate_stale_plan_row_and_expired_budget() {
    let (mut db, _data_dir) = migrated_db("review_hostile_fences").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let candidate = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    mark_candidate_wave_review_ready(db.pool(), &fixture).await;

    let base = CandidateReviewDecision {
        candidate_id: candidate.candidate_id,
        expected_candidate_plan_hash: "sha256:candidate-plan".to_string(),
        expected_candidate_row_version: 0,
        approve: true,
        expires_at: Some(chrono::Utc::now() + chrono::Duration::minutes(30)),
    };
    for (label, decision, expected_code) in [
        (
            "sibling",
            CandidateReviewDecision {
                candidate_id: Uuid::new_v4(),
                ..base.clone()
            },
            "ATTACK_REVIEW_SCOPE_MISMATCH",
        ),
        (
            "stale plan",
            CandidateReviewDecision {
                expected_candidate_plan_hash: "sha256:changed".to_string(),
                ..base.clone()
            },
            "ATTACK_CANDIDATE_PLAN_CHANGED",
        ),
        (
            "stale row",
            CandidateReviewDecision {
                expected_candidate_row_version: 9,
                ..base.clone()
            },
            "ATTACK_CANDIDATE_PLAN_CHANGED",
        ),
        (
            "expired budget",
            CandidateReviewDecision {
                expires_at: Some(chrono::Utc::now() - chrono::Duration::seconds(1)),
                ..base.clone()
            },
            "ATTACK_APPROVAL_EXPIRED",
        ),
    ] {
        let error = review_wave_candidates(
            db.pool(),
            ReviewCandidateBatch {
                operation_id: fixture.operation_id,
                wave_run_id: fixture.wave_run_id,
                decisions: vec![decision],
            },
        )
        .await
        .expect_err(label);
        assert!(
            error.to_string().contains(expected_code),
            "{label} returned unexpected error: {error}"
        );
    }
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn review_close_sets_durable_resume_pending_and_survives_process_restart() {
    let (mut db, _data_dir) = migrated_db("review_durable_resume_pending").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let candidate = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    mark_candidate_wave_review_ready(db.pool(), &fixture).await;
    let reviewed = review_wave_candidates(
        db.pool(),
        ReviewCandidateBatch {
            operation_id: fixture.operation_id,
            wave_run_id: fixture.wave_run_id,
            decisions: vec![CandidateReviewDecision {
                candidate_id: candidate.candidate_id,
                expected_candidate_plan_hash: "sha256:candidate-plan".to_string(),
                expected_candidate_row_version: 0,
                approve: false,
                expires_at: None,
            }],
        },
    )
    .await
    .expect("close exact review");
    assert_eq!(reviewed.state.barrier.status, "resume_pending");

    let reloaded = list_candidate_reviews(db.pool(), fixture.operation_id, fixture.wave_run_id)
        .await
        .expect("a fresh process reloads the durable review");
    assert_eq!(reloaded.barrier.status, "resume_pending");
    assert_eq!(reloaded.candidates.len(), 1);
    assert_eq!(reloaded.candidates[0].disposition, "rejected");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn stale_dispatching_wakeup_reopens_without_reopening_review_decisions() {
    let (mut db, _data_dir) = migrated_db("review_stale_dispatch_reaper").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let candidate = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    mark_candidate_wave_review_ready(db.pool(), &fixture).await;
    let reviewed = review_wave_candidates(
        db.pool(),
        ReviewCandidateBatch {
            operation_id: fixture.operation_id,
            wave_run_id: fixture.wave_run_id,
            decisions: vec![CandidateReviewDecision {
                candidate_id: candidate.candidate_id,
                expected_candidate_plan_hash: "sha256:candidate-plan".to_string(),
                expected_candidate_row_version: 0,
                approve: false,
                expires_at: None,
            }],
        },
    )
    .await
    .expect("close review before resume dispatch");
    sqlx::query(
        "UPDATE sessions SET chat_session_key='candidate-review-test'
         WHERE id=(SELECT session_id FROM tasks WHERE id=$1)",
    )
    .bind(fixture.operation_id)
    .execute(db.pool())
    .await
    .expect("bind the fixture operation to a durable chat session");
    let claim = claim_candidate_review_resume(
        db.pool(),
        fixture.operation_id,
        fixture.wave_run_id,
        reviewed.state.barrier.resume_version,
    )
    .await
    .expect("claim durable resume");
    assert!(claim.dispatch_required);
    sqlx::query(
        "UPDATE candidate_review_barriers
         SET dispatch_started_at=NOW()-INTERVAL '10 minutes' WHERE wave_run_id=$1",
    )
    .bind(fixture.wave_run_id)
    .execute(db.pool())
    .await
    .expect("simulate a process dying during dispatch");
    assert_eq!(
        reap_stale_candidate_review_dispatches(db.pool(), chrono::Duration::minutes(5))
            .await
            .expect("startup reaper resets stale dispatch"),
        1
    );
    let reopened = list_candidate_reviews(db.pool(), fixture.operation_id, fixture.wave_run_id)
        .await
        .expect("reload reaped resume state");
    assert_eq!(reopened.barrier.status, "resume_pending");
    assert_eq!(reopened.candidates[0].disposition, "rejected");
    assert_eq!(
        reopened.candidates[0]
            .latest_approval
            .as_ref()
            .expect("review decision remains durable")
            .status,
        "rejected"
    );

    let retry = claim_candidate_review_resume(
        db.pool(),
        fixture.operation_id,
        fixture.wave_run_id,
        reopened.barrier.resume_version,
    )
    .await
    .expect("retry resume after restart");
    mark_candidate_review_resumed(db.pool(), &retry)
        .await
        .expect("mark trusted resume started");
    let replay = claim_candidate_review_resume(
        db.pool(),
        fixture.operation_id,
        fixture.wave_run_id,
        reopened.barrier.resume_version,
    )
    .await
    .expect("response-loss resume retry is idempotent");
    assert!(replay.replayed);
    assert!(!replay.dispatch_required);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn compound_claim_owns_worker_and_lane_with_one_lease_token() {
    let (mut db, _data_dir) = migrated_db("compound_candidate_claim").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let candidate_a = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    insert_approval(db.pool(), &fixture, candidate_a, fixture.org_a)
        .await
        .expect("approve org A candidate");
    sqlx::query("UPDATE attack_candidates SET disposition='approved' WHERE candidate_id=$1")
        .bind(candidate_a.candidate_id)
        .execute(db.pool())
        .await
        .expect("mark org A candidate approved");
    sqlx::query(
        "UPDATE attack_wave_units SET review_closed=TRUE,status='verification' WHERE id=$1",
    )
    .bind(fixture.org_a.wave_unit_id)
    .execute(db.pool())
    .await
    .expect("close org A review");
    let (stage_execution_a, stage_unit_a) =
        seed_verification_unit(db.pool(), &fixture, fixture.org_a).await;
    let claimed = claim_next_candidate_attempt(
        db.pool(),
        CandidateClaimQuery {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_a.wave_unit_id,
            organization_id: fixture.org_a.organization_id,
            verification_stage_execution_id: stage_execution_a,
            verification_stage_run_unit_id: stage_unit_a,
            lease_owner: "claim-test-a".to_string(),
            lease_seconds: 60,
        },
    )
    .await
    .expect("claim exact Candidate")
    .expect("Candidate available");
    let lane: (Uuid, Uuid, String) = sqlx::query_as(
        "SELECT stage_worker_run_id,lease_token,lease_owner
         FROM attack_execution_lanes WHERE lane_key='global:exploit'",
    )
    .fetch_one(db.pool())
    .await
    .expect("read exploit lane");
    assert_eq!(lane.0, claimed.worker.id);
    assert_eq!(Some(lane.1), claimed.worker.lease_token);
    assert_eq!(lane.2, "claim-test-a");
    assert_eq!(claimed.attempt.stage_worker_run_id, Some(claimed.worker.id));
    assert_eq!(claimed.worker.work_item_kind, "candidate_attempt");
    assert_eq!(claimed.worker.work_item_key, claimed.attempt.id.to_string());

    let candidate_b = seed_candidate(db.pool(), &fixture, fixture.org_b).await;
    insert_approval(db.pool(), &fixture, candidate_b, fixture.org_b)
        .await
        .expect("approve org B candidate");
    sqlx::query("UPDATE attack_candidates SET disposition='approved' WHERE candidate_id=$1")
        .bind(candidate_b.candidate_id)
        .execute(db.pool())
        .await
        .expect("mark org B candidate approved");
    sqlx::query(
        "UPDATE attack_wave_units SET review_closed=TRUE,status='verification' WHERE id=$1",
    )
    .bind(fixture.org_b.wave_unit_id)
    .execute(db.pool())
    .await
    .expect("close org B review");
    let (stage_execution_b, stage_unit_b) =
        seed_verification_unit(db.pool(), &fixture, fixture.org_b).await;
    let blocked = claim_next_candidate_attempt(
        db.pool(),
        CandidateClaimQuery {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_b.wave_unit_id,
            organization_id: fixture.org_b.organization_id,
            verification_stage_execution_id: stage_execution_b,
            verification_stage_run_unit_id: stage_unit_b,
            lease_owner: "claim-test-b".to_string(),
            lease_seconds: 60,
        },
    )
    .await
    .expect("second claimant does not error");
    assert!(blocked.is_none(), "global exploit lane must serialize orgs");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn heartbeat_and_retry_release_terminalizes_old_attempt_and_claims_new_ordinal() {
    let (mut db, _data_dir) = migrated_db("candidate_heartbeat_release").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let candidate = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    insert_approval(db.pool(), &fixture, candidate, fixture.org_a)
        .await
        .expect("approve Candidate");
    sqlx::query("UPDATE attack_candidates SET disposition='approved' WHERE candidate_id=$1")
        .bind(candidate.candidate_id)
        .execute(db.pool())
        .await
        .expect("mark Candidate approved");
    sqlx::query(
        "UPDATE attack_wave_units SET review_closed=TRUE,status='verification' WHERE id=$1",
    )
    .bind(fixture.org_a.wave_unit_id)
    .execute(db.pool())
    .await
    .expect("close Candidate review");
    let (stage_execution_id, stage_run_unit_id) =
        seed_verification_unit(db.pool(), &fixture, fixture.org_a).await;
    let claimed = claim_next_candidate_attempt(
        db.pool(),
        CandidateClaimQuery {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_a.wave_unit_id,
            organization_id: fixture.org_a.organization_id,
            verification_stage_execution_id: stage_execution_id,
            verification_stage_run_unit_id: stage_run_unit_id,
            lease_owner: "heartbeat-test".to_string(),
            lease_seconds: 30,
        },
    )
    .await
    .expect("claim Candidate")
    .expect("Candidate available");
    let lease_token = claimed.worker.lease_token.expect("worker lease token");
    let heartbeat = CandidateExecutionHeartbeat {
        operation_id: fixture.operation_id,
        scope_snapshot_id: fixture.scope_snapshot_id,
        wave_run_id: fixture.wave_run_id,
        wave_unit_id: fixture.org_a.wave_unit_id,
        organization_id: fixture.org_a.organization_id,
        attempt_id: claimed.attempt.id,
        worker_run_id: claimed.worker.id,
        stage_execution_id,
        stage_run_unit_id,
        lease_token,
        lease_owner: "heartbeat-test".to_string(),
        attempt_epoch: claimed.worker.attempt_epoch,
        expected_checkpoint_version: claimed.worker.checkpoint_version,
        extend_seconds: 120,
    };
    let heartbeated = heartbeat_candidate_execution(db.pool(), heartbeat.clone())
        .await
        .expect("heartbeat worker and lane");
    let expiries: (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
        r#"SELECT worker.lease_expires_at,lane.lease_expires_at
               FROM stage_worker_runs worker
               JOIN attack_execution_lanes lane ON lane.stage_worker_run_id=worker.id
               WHERE worker.id=$1"#,
    )
    .bind(claimed.worker.id)
    .fetch_one(db.pool())
    .await
    .expect("read compound heartbeat");
    assert_eq!(expiries.0, expiries.1);
    assert_eq!(heartbeated.lease_expires_at, expiries.0);

    let released = release_candidate_execution(
        db.pool(),
        CandidateExecutionRelease {
            operation_id: heartbeat.operation_id,
            scope_snapshot_id: heartbeat.scope_snapshot_id,
            wave_run_id: heartbeat.wave_run_id,
            wave_unit_id: heartbeat.wave_unit_id,
            organization_id: heartbeat.organization_id,
            attempt_id: heartbeat.attempt_id,
            worker_run_id: heartbeat.worker_run_id,
            stage_execution_id: heartbeat.stage_execution_id,
            stage_run_unit_id: heartbeat.stage_run_unit_id,
            lease_token: heartbeat.lease_token,
            lease_owner: heartbeat.lease_owner,
            attempt_epoch: heartbeat.attempt_epoch,
            expected_checkpoint_version: heartbeat.expected_checkpoint_version,
        },
    )
    .await
    .expect("release worker and lane");
    assert!(!released.requeued);
    type ReleasedOwnership = (
        Option<Uuid>,
        Option<Uuid>,
        String,
        String,
        Option<serde_json::Value>,
        Option<chrono::DateTime<chrono::Utc>>,
    );
    let ownership: ReleasedOwnership = sqlx::query_as(
        r#"SELECT lane.stage_worker_run_id,worker.lease_token,worker.status,attempt.status,
                  attempt.result_json,attempt.terminal_at
           FROM attack_execution_lanes lane
           JOIN stage_worker_runs worker ON worker.id=$1
           JOIN candidate_attempts attempt ON attempt.id=$2
           WHERE lane.lane_key='global:exploit'"#,
    )
    .bind(claimed.worker.id)
    .bind(claimed.attempt.id)
    .fetch_one(db.pool())
    .await
    .expect("read compound release");
    assert_eq!(ownership.0, None);
    assert_eq!(ownership.1, None);
    assert_eq!(ownership.2, "failed");
    assert_eq!(ownership.3, "retryable_failed");
    assert_eq!(
        ownership.4,
        Some(serde_json::json!({
            "disposition": "retryable_failed",
            "reason_code": "worker_released_for_retry",
            "schema_version": 1
        }))
    );
    assert!(ownership.5.is_some());

    let retried = claim_next_candidate_attempt(
        db.pool(),
        CandidateClaimQuery {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_a.wave_unit_id,
            organization_id: fixture.org_a.organization_id,
            verification_stage_execution_id: stage_execution_id,
            verification_stage_run_unit_id: stage_run_unit_id,
            lease_owner: "retry-test".to_string(),
            lease_seconds: 30,
        },
    )
    .await
    .expect("claim retry ordinal")
    .expect("retry Candidate remains claimable");
    assert_ne!(retried.attempt.id, claimed.attempt.id);
    assert_eq!(retried.attempt.ordinal, claimed.attempt.ordinal + 1);
    let old_after_retry: (String, Option<serde_json::Value>, Option<String>) =
        sqlx::query_as("SELECT status,result_json,result_hash FROM candidate_attempts WHERE id=$1")
            .bind(claimed.attempt.id)
            .fetch_one(db.pool())
            .await
            .expect("re-read terminal old attempt");
    assert_eq!(old_after_retry.0, "retryable_failed");
    assert_eq!(old_after_retry.1, ownership.4);
    assert!(old_after_retry.2.is_some());
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn terminalizer_replay_returns_same_finding_and_lineage() {
    let (mut db, _data_dir) = migrated_db("terminalize_verified").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let candidate = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    let approval_id = insert_approval(db.pool(), &fixture, candidate, fixture.org_a)
        .await
        .expect("approve Candidate");
    sqlx::query("UPDATE attack_candidates SET disposition='approved' WHERE candidate_id=$1")
        .bind(candidate.candidate_id)
        .execute(db.pool())
        .await
        .expect("mark Candidate approved");
    sqlx::query(
        "UPDATE attack_wave_units SET review_closed=TRUE,status='verification' WHERE id=$1",
    )
    .bind(fixture.org_a.wave_unit_id)
    .execute(db.pool())
    .await
    .expect("close Candidate review");
    let (stage_execution_id, stage_run_unit_id) =
        seed_verification_unit(db.pool(), &fixture, fixture.org_a).await;
    let claimed = claim_next_candidate_attempt(
        db.pool(),
        CandidateClaimQuery {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_a.wave_unit_id,
            organization_id: fixture.org_a.organization_id,
            verification_stage_execution_id: stage_execution_id,
            verification_stage_run_unit_id: stage_run_unit_id,
            lease_owner: "terminalizer-test".to_string(),
            lease_seconds: 120,
        },
    )
    .await
    .expect("claim Candidate")
    .expect("Candidate available");
    let proof_evidence_id = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_a.organization_id,
        fixture.org_a.target_id,
        "evidence",
    )
    .await;
    let foreign_proof_id = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_b.organization_id,
        fixture.org_b.target_id,
        "evidence",
    )
    .await;
    let result_json = serde_json::json!({
        "disposition": "verified",
        "proof_evidence_ids": [proof_evidence_id],
        "finding": {
            "title": "Verified bounded SQL injection",
            "severity": "high",
            "cvss": 8.1,
            "affected_target": "https://shared.example.test/login",
            "description": "Deterministic verifier reproduced the bounded condition.",
            "steps": "Replay the evidence-backed bounded action journal.",
            "remediation": "Use parameterized queries and least privilege."
        }
    });
    let lease_token = claimed.worker.lease_token.expect("claimed lease token");
    let submission_command = RecordAttemptSubmission {
        operation_id: fixture.operation_id,
        scope_snapshot_id: fixture.scope_snapshot_id,
        wave_run_id: fixture.wave_run_id,
        wave_unit_id: fixture.org_a.wave_unit_id,
        organization_id: fixture.org_a.organization_id,
        candidate_id: candidate.candidate_id,
        approval_id,
        attempt_id: claimed.attempt.id,
        candidate_plan_hash: "sha256:candidate-plan".to_string(),
        worker_run_id: claimed.worker.id,
        stage_execution_id,
        stage_run_unit_id,
        lease_token,
        lease_owner: "terminalizer-test".to_string(),
        attempt_epoch: claimed.worker.attempt_epoch,
        expected_checkpoint_version: claimed.worker.checkpoint_version,
        result_json,
        evidence: vec![AttemptEvidenceLink {
            evidence_id: proof_evidence_id,
            role: "proof".to_string(),
        }],
    };
    let mut planned_tx = db.pool().begin().await.expect("begin planned submission");
    let planned_error = record_attempt_submission(&mut planned_tx, submission_command.clone())
        .await
        .expect_err("an unexecuted planned action must block Attempt submission");
    assert!(
        planned_error
            .to_string()
            .contains("action journal is not terminal"),
        "unexpected planned action error: {planned_error}"
    );
    planned_tx
        .rollback()
        .await
        .expect("rollback planned submission");
    let finished_action = sqlx::query(
        "INSERT INTO candidate_attempt_actions(
             attempt_id,action_ordinal,capability_id,action_kind,canonical_args,status,
             outcome,outcome_hash,started_at,completed_at)
         VALUES($1,0,'verify.sql_injection','bounded_sql_injection_probe',
                '{\"target\":\"https://shared.example.test/login\"}'::jsonb,
                'completed','{}'::jsonb,'sha256:test',NOW(),NOW())",
    )
    .bind(claimed.attempt.id)
    .execute(db.pool())
    .await
    .expect("finish approved action for terminalizer fixture");
    assert_eq!(finished_action.rows_affected(), 1);
    let action_state: (i32, String) = sqlx::query_as(
        "SELECT action_ordinal,status FROM candidate_attempt_actions
         WHERE attempt_id=$1",
    )
    .bind(claimed.attempt.id)
    .fetch_one(db.pool())
    .await
    .expect("reload finished approved action");
    assert_eq!(action_state, (0, "completed".to_string()));

    let mut submission_tx = db.pool().begin().await.expect("begin submission");
    let submission = record_attempt_submission(&mut submission_tx, submission_command)
        .await
        .expect("record immutable submitted result");
    submission_tx.commit().await.expect("commit submission");
    let expected_result_hash = submission
        .attempt
        .result_hash
        .clone()
        .expect("server-derived result hash");
    let base = TerminalizeVerifiedFinding {
        operation_id: fixture.operation_id,
        scope_snapshot_id: fixture.scope_snapshot_id,
        wave_run_id: fixture.wave_run_id,
        wave_unit_id: fixture.org_a.wave_unit_id,
        organization_id: fixture.org_a.organization_id,
        candidate_id: candidate.candidate_id,
        approval_id,
        attempt_id: claimed.attempt.id,
        candidate_plan_hash: "sha256:candidate-plan".to_string(),
        expected_result_hash,
        proof_evidence_ids: vec![foreign_proof_id],
        worker_run_id: claimed.worker.id,
        stage_execution_id,
        stage_run_unit_id,
        lease_token,
        lease_owner: "terminalizer-test".to_string(),
        attempt_epoch: claimed.worker.attempt_epoch,
        expected_checkpoint_version: claimed.worker.checkpoint_version,
    };
    let mut foreign_tx = db.pool().begin().await.expect("begin foreign proof");
    assert!(terminalize_verified_finding(&mut foreign_tx, base.clone())
        .await
        .is_err());
    foreign_tx.rollback().await.expect("rollback foreign proof");

    let mut exact = base;
    exact.proof_evidence_ids = vec![proof_evidence_id];
    sqlx::raw_sql(
        r#"CREATE FUNCTION reject_candidate_terminal_outbox_fixture()
           RETURNS trigger AS $$
           BEGIN
               IF NEW.event_name='CandidateAttemptTerminal.v1' THEN
                   RAISE EXCEPTION 'fixture rejects Candidate terminal outbox';
               END IF;
               RETURN NEW;
           END;
           $$ LANGUAGE plpgsql;
           CREATE TRIGGER reject_candidate_terminal_outbox_fixture
           BEFORE INSERT ON knowledge_outbox_events
           FOR EACH ROW EXECUTE FUNCTION reject_candidate_terminal_outbox_fixture();"#,
    )
    .execute(db.pool())
    .await
    .expect("install Candidate terminal outbox failure fixture");
    let mut rejected_terminal_tx = db
        .pool()
        .begin()
        .await
        .expect("begin rejected terminalization");
    terminalize_verified_finding(&mut rejected_terminal_tx, exact.clone())
        .await
        .expect_err("outbox failure must reject Candidate terminalization");
    rejected_terminal_tx
        .rollback()
        .await
        .expect("rollback rejected terminalization");
    type RolledBackTerminalState = (
        String,
        String,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        String,
    );
    let rolled_back: RolledBackTerminalState = sqlx::query_as(
        r#"SELECT attempt.status,candidate.disposition,candidate.terminal_attempt_id,
                  candidate.terminal_finding_id,lane.stage_worker_run_id,worker.status
             FROM candidate_attempts attempt
             JOIN attack_candidates candidate ON candidate.candidate_id=attempt.candidate_id
             JOIN attack_execution_lanes lane ON lane.lane_key='global:exploit'
             JOIN stage_worker_runs worker ON worker.id=attempt.stage_worker_run_id
            WHERE attempt.id=$1"#,
    )
    .bind(claimed.attempt.id)
    .fetch_one(db.pool())
    .await
    .expect("read rolled-back Candidate terminal state");
    assert_eq!(rolled_back.0, "submitted");
    assert_eq!(rolled_back.1, "approved");
    assert_eq!(rolled_back.2, None);
    assert_eq!(rolled_back.3, None);
    assert_eq!(rolled_back.4, Some(claimed.worker.id));
    assert_eq!(rolled_back.5, "running");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM findings WHERE source='candidate_v2'")
            .fetch_one(db.pool())
            .await
            .expect("count rolled-back Candidate Findings"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM finding_lineage WHERE candidate_attempt_id=$1"
        )
        .bind(claimed.attempt.id)
        .fetch_one(db.pool())
        .await
        .expect("count rolled-back Finding lineage"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM knowledge_outbox_events
             WHERE event_name='CandidateAttemptTerminal.v1' AND source_id_value=$1"
        )
        .bind(claimed.attempt.id.hyphenated().to_string())
        .fetch_one(db.pool())
        .await
        .expect("count rolled-back Candidate terminal events"),
        0
    );
    sqlx::raw_sql(
        r#"DROP TRIGGER reject_candidate_terminal_outbox_fixture ON knowledge_outbox_events;
           DROP FUNCTION reject_candidate_terminal_outbox_fixture();"#,
    )
    .execute(db.pool())
    .await
    .expect("remove Candidate terminal outbox failure fixture");
    let mut terminal_tx = db.pool().begin().await.expect("begin terminalization");
    let terminal = terminalize_verified_finding(&mut terminal_tx, exact.clone())
        .await
        .expect("terminalize exact verified proof");
    terminal_tx.commit().await.expect("commit terminalization");
    assert!(!terminal.replayed);

    let mut replay_tx = db.pool().begin().await.expect("begin terminal replay");
    let replay = terminalize_verified_finding(&mut replay_tx, exact.clone())
        .await
        .expect("terminalization response-loss replay");
    replay_tx.commit().await.expect("commit terminal replay");
    assert!(replay.replayed);
    assert_eq!(replay.finding_id, terminal.finding_id);
    let state: (String, String, Option<Uuid>, Option<Uuid>, Option<Uuid>) = sqlx::query_as(
        r#"SELECT attempt.status,candidate.disposition,candidate.terminal_attempt_id,
                  candidate.terminal_finding_id,lane.stage_worker_run_id
             FROM candidate_attempts attempt
             JOIN attack_candidates candidate ON candidate.candidate_id=attempt.candidate_id
             JOIN attack_execution_lanes lane ON lane.lane_key='global:exploit'
            WHERE attempt.id=$1"#,
    )
    .bind(claimed.attempt.id)
    .fetch_one(db.pool())
    .await
    .expect("read atomic terminal state");
    assert_eq!(state.0, "verified");
    assert_eq!(state.1, "verified");
    assert_eq!(state.2, Some(claimed.attempt.id));
    assert_eq!(state.3, Some(terminal.finding_id));
    assert_eq!(state.4, None);
    type CandidateTerminalEvent = (Uuid, String, String, String, i64, serde_json::Value);
    let terminal_event: CandidateTerminalEvent = sqlx::query_as(
        r#"SELECT event_id,source_kind,source_id_value,source_stream_key,
                  source_version,payload
             FROM knowledge_outbox_events
            WHERE event_name='CandidateAttemptTerminal.v1'
              AND source_operation_id=$1 AND source_id_value=$2"#,
    )
    .bind(fixture.operation_id)
    .bind(claimed.attempt.id.hyphenated().to_string())
    .fetch_one(db.pool())
    .await
    .expect("load canonical Candidate terminal event");
    assert_eq!(
        terminal_event.0,
        Uuid::new_v5(&claimed.attempt.id, b"CandidateAttemptTerminal.v1")
    );
    assert_eq!(terminal_event.1, "candidate_attempt");
    assert_eq!(
        terminal_event.2,
        claimed.attempt.id.hyphenated().to_string()
    );
    assert_eq!(
        terminal_event.3,
        format!("candidate-attempt:{}", claimed.attempt.id)
    );
    let terminal_row_version: i64 =
        sqlx::query_scalar("SELECT row_version FROM candidate_attempts WHERE id=$1")
            .bind(claimed.attempt.id)
            .fetch_one(db.pool())
            .await
            .expect("load terminal Attempt source version");
    assert_eq!(terminal_event.4, terminal_row_version);
    assert_eq!(
        terminal_event.5["structured_payload"]["attempt_id"],
        serde_json::json!(claimed.attempt.id)
    );
    assert_eq!(
        terminal_event.5["structured_payload"]["candidate_id"],
        serde_json::json!(candidate.candidate_id)
    );
    assert_eq!(
        terminal_event.5["structured_payload"]["disposition"],
        "verified"
    );
    assert_eq!(
        terminal_event.5["structured_payload"]["finding_id"],
        serde_json::json!(terminal.finding_id)
    );
    assert_eq!(
        terminal_event.5["structured_payload"]["evidence_ids"],
        serde_json::json!([proof_evidence_id])
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            r#"SELECT COUNT(*)
                 FROM knowledge_projection_deliveries
                WHERE event_id=$1"#,
        )
        .bind(terminal_event.0)
        .fetch_one(db.pool())
        .await
        .expect("count Candidate terminal projector deliveries"),
        4
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM knowledge_outbox_events
             WHERE event_name='CandidateAttemptTerminal.v1'
               AND source_operation_id=$1 AND source_id_value=$2"
        )
        .bind(fixture.operation_id)
        .bind(claimed.attempt.id.hyphenated().to_string())
        .fetch_one(db.pool())
        .await
        .expect("count replay-safe Candidate terminal events"),
        1
    );
    let update_error =
        sqlx::query("UPDATE candidate_attempts SET result_json=result_json WHERE id=$1")
            .bind(claimed.attempt.id)
            .execute(db.pool())
            .await
            .expect_err("terminal CandidateAttempt source cannot bump its version");
    assert!(update_error
        .to_string()
        .contains("TERMINAL_CANONICAL_SOURCE_IMMUTABLE"));
    let delete_error = sqlx::query("DELETE FROM candidate_attempts WHERE id=$1")
        .bind(claimed.attempt.id)
        .execute(db.pool())
        .await
        .expect_err("terminal CandidateAttempt source cannot be deleted");
    assert!(delete_error
        .to_string()
        .contains("TERMINAL_CANONICAL_SOURCE_IMMUTABLE"));
    let mut retained_replay_tx = db
        .pool()
        .begin()
        .await
        .expect("begin retained terminal replay");
    let retained_replay = terminalize_verified_finding(&mut retained_replay_tx, exact)
        .await
        .expect("terminal source remains exactly replayable after blocked mutations");
    retained_replay_tx
        .commit()
        .await
        .expect("commit retained terminal replay");
    assert!(retained_replay.replayed);
    assert_eq!(retained_replay.finding_id, terminal.finding_id);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM knowledge_outbox_events
             WHERE event_name='CandidateAttemptTerminal.v1'
               AND source_operation_id=$1 AND source_id_value=$2",
        )
        .bind(fixture.operation_id)
        .bind(claimed.attempt.id.hyphenated().to_string())
        .fetch_one(db.pool())
        .await
        .expect("terminal replay still owns one Candidate event"),
        1
    );
    let truth = golish_db::repo::verification_truth::load_for_operation(
        db.pool(),
        fixture.operation_id,
        Some(fixture.org_a.organization_id),
    )
    .await
    .expect("load exact Verification truth after terminalization");
    assert_eq!(truth.expected_units.len(), 1);
    assert_eq!(truth.snapshots.len(), 1);
    assert!(truth.snapshots[0].review_closed);
    assert_eq!(truth.snapshots[0].pending_work_items, 0);
    assert_eq!(truth.snapshots[0].approved_ever, 1);
    assert_eq!(truth.snapshots[0].attempts.len(), 1);
    assert_eq!(
        truth.snapshots[0].attempts[0].attempt_id,
        claimed.attempt.id
    );
    assert_eq!(
        truth.snapshots[0].attempts[0].proof_evidence_ids,
        vec![proof_evidence_id]
    );
    assert_eq!(
        truth.snapshots[0].attempts[0].finding_id,
        Some(terminal.finding_id)
    );
    assert!(truth.snapshots[0].attempts[0].finding_lineage_exact);
    let late_evidence_id = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_a.organization_id,
        fixture.org_a.target_id,
        "evidence",
    )
    .await;
    let late_insert = sqlx::query(
        "INSERT INTO candidate_attempt_evidence(attempt_id,evidence_id,role)
         VALUES($1,$2,'proof')",
    )
    .bind(claimed.attempt.id)
    .bind(late_evidence_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        late_insert.map(|_| ()),
        "P0001",
        "terminal Attempt evidence insertion",
    );
    let late_delete = sqlx::query(
        "DELETE FROM candidate_attempt_evidence
         WHERE attempt_id=$1 AND evidence_id=$2 AND role='proof'",
    )
    .bind(claimed.attempt.id)
    .bind(proof_evidence_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        late_delete.map(|_| ()),
        "P0001",
        "terminal Attempt evidence deletion",
    );
    let late_role_change = sqlx::query(
        "UPDATE candidate_attempt_evidence SET role='refutation'
         WHERE attempt_id=$1 AND evidence_id=$2 AND role='proof'",
    )
    .bind(claimed.attempt.id)
    .bind(proof_evidence_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        late_role_change.map(|_| ()),
        "P0001",
        "terminal Attempt evidence role mutation",
    );
    let persisted_cvss: Option<f64> = sqlx::query_scalar("SELECT cvss FROM findings WHERE id=$1")
        .bind(terminal.finding_id)
        .fetch_one(db.pool())
        .await
        .expect("load terminalizer Finding CVSS");
    assert_eq!(persisted_cvss, Some(8.1));

    let empty_array = serde_json::json!([]);
    let tamper = golish_db::repo::findings::FindingUpsert {
        id: terminal.finding_id,
        title: "Tampered finding",
        sev: "low",
        cvss: Some(1.0),
        url: "https://foreign.example.test",
        target: "https://foreign.example.test",
        target_id: None,
        description: "overwrite immutable proof",
        steps: "none",
        remediation: "none",
        tags: &empty_array,
        tool: "user",
        template: "",
        refs: &empty_array,
        evidence: &empty_array,
        status: "open",
        source: "user",
        project_path: Some("/tmp/attack-v2"),
        created_at: 0.0,
        updated_at: 1.0,
    };
    assert!(
        golish_db::repo::findings::upsert_full(db.pool(), FindingWriteContext::UserCrud, &tamper)
            .await
            .is_err(),
        "UserCrud must not rewrite proof-bearing fields of a lineage-bound Finding"
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn verification_truth_rejects_a_partial_current_wave_instead_of_hiding_siblings() {
    let (mut db, _data_dir) = migrated_db("verification_truth_partial_wave").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    sqlx::query("UPDATE attack_wave_runs SET status='verification' WHERE id=$1")
        .bind(fixture.wave_run_id)
        .execute(db.pool())
        .await
        .expect("advance current wave");
    sqlx::query(
        "UPDATE attack_wave_units
         SET status='verification',review_closed=TRUE
         WHERE id=$1",
    )
    .bind(fixture.org_a.wave_unit_id)
    .execute(db.pool())
    .await
    .expect("advance one sibling only");

    let error = golish_db::repo::verification_truth::load_for_operation(
        db.pool(),
        fixture.operation_id,
        None,
    )
    .await
    .expect_err("a non-verification sibling must make exact truth unavailable");
    assert!(
        error
            .to_string()
            .contains("wave unit is not verification-ready"),
        "unexpected partial-wave error: {error}"
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn reason_only_blocked_attempt_persists_and_terminalizes_without_fake_evidence() {
    let (mut db, _data_dir) = migrated_db("reason_only_blocked_attempt").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let candidate = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    let approval_id = insert_approval(db.pool(), &fixture, candidate, fixture.org_a)
        .await
        .expect("approve Candidate");
    sqlx::query("UPDATE attack_candidates SET disposition='approved' WHERE candidate_id=$1")
        .bind(candidate.candidate_id)
        .execute(db.pool())
        .await
        .expect("mark Candidate approved");
    sqlx::query(
        "UPDATE attack_wave_units SET review_closed=TRUE,status='verification' WHERE id=$1",
    )
    .bind(fixture.org_a.wave_unit_id)
    .execute(db.pool())
    .await
    .expect("close Candidate review");
    let (stage_execution_id, stage_run_unit_id) =
        seed_verification_unit(db.pool(), &fixture, fixture.org_a).await;
    let claimed = claim_next_candidate_attempt(
        db.pool(),
        CandidateClaimQuery {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_a.wave_unit_id,
            organization_id: fixture.org_a.organization_id,
            verification_stage_execution_id: stage_execution_id,
            verification_stage_run_unit_id: stage_run_unit_id,
            lease_owner: "reason-only-test".to_string(),
            lease_seconds: 120,
        },
    )
    .await
    .expect("claim Candidate")
    .expect("Candidate available");
    sqlx::query(
        "INSERT INTO candidate_attempt_actions(
             attempt_id,action_ordinal,capability_id,action_kind,canonical_args,status,
             outcome,outcome_hash,started_at,completed_at)
         VALUES($1,0,'verify.sql_injection','bounded_sql_injection_probe',
                '{\"target\":\"https://shared.example.test/login\"}'::jsonb,
                'failed','{}'::jsonb,'sha256:blocked',NOW(),NOW())",
    )
    .bind(claimed.attempt.id)
    .execute(db.pool())
    .await
    .expect("record terminal failed action");
    let lease_token = claimed.worker.lease_token.expect("claimed lease token");
    let mut submission_tx = db.pool().begin().await.expect("begin submission");
    let submitted = record_attempt_submission(
        &mut submission_tx,
        RecordAttemptSubmission {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_a.wave_unit_id,
            organization_id: fixture.org_a.organization_id,
            candidate_id: candidate.candidate_id,
            approval_id,
            attempt_id: claimed.attempt.id,
            candidate_plan_hash: "sha256:candidate-plan".to_string(),
            worker_run_id: claimed.worker.id,
            stage_execution_id,
            stage_run_unit_id,
            lease_token,
            lease_owner: "reason-only-test".to_string(),
            attempt_epoch: claimed.worker.attempt_epoch,
            expected_checkpoint_version: claimed.worker.checkpoint_version,
            result_json: serde_json::json!({
                "disposition": "blocked",
                "blocker_reason_code": "approval_expired",
            }),
            evidence: Vec::new(),
        },
    )
    .await
    .expect("stable reason must be sufficient for blocked submission");
    submission_tx.commit().await.expect("commit submission");
    let mut terminal_tx = db.pool().begin().await.expect("begin terminalization");
    let terminal = terminalize_candidate_attempt(
        &mut terminal_tx,
        TerminalizeCandidateAttempt {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_a.wave_unit_id,
            organization_id: fixture.org_a.organization_id,
            candidate_id: candidate.candidate_id,
            approval_id,
            attempt_id: claimed.attempt.id,
            candidate_plan_hash: "sha256:candidate-plan".to_string(),
            expected_result_hash: submitted.attempt.result_hash.expect("result hash"),
            worker_run_id: claimed.worker.id,
            stage_execution_id,
            stage_run_unit_id,
            lease_token,
            lease_owner: "reason-only-test".to_string(),
            attempt_epoch: claimed.worker.attempt_epoch,
            expected_checkpoint_version: claimed.worker.checkpoint_version,
        },
    )
    .await
    .expect("terminalize reason-only blocked Attempt");
    terminal_tx.commit().await.expect("commit terminalization");
    assert_eq!(terminal.disposition, "blocked");
    assert_eq!(terminal.finding_id, None);
    let blocked_event: (String, serde_json::Value) = sqlx::query_as(
        r#"SELECT source_kind,payload
             FROM knowledge_outbox_events
            WHERE event_name='CandidateAttemptTerminal.v1'
              AND source_operation_id=$1 AND source_id_value=$2"#,
    )
    .bind(fixture.operation_id)
    .bind(claimed.attempt.id.hyphenated().to_string())
    .fetch_one(db.pool())
    .await
    .expect("load reason-only blocked Candidate terminal event");
    assert_eq!(blocked_event.0, "candidate_attempt");
    assert_eq!(
        blocked_event.1["structured_payload"]["disposition"],
        "blocked"
    );
    assert_eq!(
        blocked_event.1["structured_payload"]["evidence_ids"],
        serde_json::json!([])
    );
    assert_eq!(
        blocked_event.1["structured_payload"]["blocker_reason_code"],
        "approval_expired",
        "the canonical event must retain the persisted reason used for intentional projector suppression"
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn non_passed_source_unit_cannot_back_attack_wave_unit() {
    let (mut db, _data_dir) = migrated_db("wave_source_authority").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    sqlx::query("DELETE FROM attack_wave_units WHERE id=$1")
        .bind(fixture.org_a.wave_unit_id)
        .execute(db.pool())
        .await
        .expect("remove accepted wave unit for hostile replay");
    sqlx::query("UPDATE stage_run_units SET status='gate_blocked',terminal_at=NULL WHERE id=$1")
        .bind(fixture.org_a.entry_stage_run_unit_id)
        .execute(db.pool())
        .await
        .expect("make trusted source non-terminal");
    let inserted = sqlx::query(
        r#"INSERT INTO attack_wave_units (
               id,wave_run_id,operation_id,scope_snapshot_id,organization_id,
               entry_stage_execution_id,entry_stage_run_unit_id,
               entry_deliverable_submission_id,entry_stage_kind,ordinal,status
           ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'vuln_triage',1,'open')"#,
    )
    .bind(fixture.org_a.wave_unit_id)
    .bind(fixture.wave_run_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.org_a.organization_id)
    .bind(fixture.entry_stage_execution_id)
    .bind(fixture.org_a.entry_stage_run_unit_id)
    .bind(fixture.org_a.entry_submission_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(inserted.map(|_| ()), "P0001", "non-passed wave source unit");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn candidate_attempt_requires_exact_verification_worker_and_approved_plan() {
    let (mut db, _data_dir) = migrated_db("attempt_authority").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let candidate = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    let approval_id = insert_approval(db.pool(), &fixture, candidate, fixture.org_a)
        .await
        .expect("insert approved candidate plan");
    let wrong_worker_attempt = sqlx::query(
        r#"INSERT INTO candidate_attempts (
               id,candidate_id,approval_id,operation_id,scope_snapshot_id,
               wave_run_id,wave_unit_id,organization_id,target_live_id,
               target_type_at_time,target_value_at_time,target_identity_hash,
               candidate_plan_hash,ordinal,status,stage_worker_run_id
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,$9,'url',
               'https://shared.example.test/login','sha256:shared-target',
               'sha256:candidate-plan',0,'running',$10
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(candidate.candidate_id)
    .bind(approval_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.wave_run_id)
    .bind(fixture.org_a.wave_unit_id)
    .bind(fixture.org_a.organization_id)
    .bind(fixture.org_a.target_id)
    .bind(fixture.org_a.worker_run_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        wrong_worker_attempt.map(|_| ()),
        "P0001",
        "non-verification candidate worker",
    );

    sqlx::query("UPDATE attack_candidate_approvals SET status='revoked' WHERE id=$1")
        .bind(approval_id)
        .execute(db.pool())
        .await
        .expect("revoke approval for hostile attempt");
    let revoked_attempt = sqlx::query(
        r#"INSERT INTO candidate_attempts (
               id,candidate_id,approval_id,operation_id,scope_snapshot_id,
               wave_run_id,wave_unit_id,organization_id,target_live_id,
               target_type_at_time,target_value_at_time,target_identity_hash,
               candidate_plan_hash,ordinal,status
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,$9,'url',
               'https://shared.example.test/login','sha256:shared-target',
               'sha256:candidate-plan',1,'queued'
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(candidate.candidate_id)
    .bind(approval_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.wave_run_id)
    .bind(fixture.org_a.wave_unit_id)
    .bind(fixture.org_a.organization_id)
    .bind(fixture.org_a.target_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        revoked_attempt.map(|_| ()),
        "P0001",
        "revoked approval attempt",
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn candidate_and_work_item_terminalize_together_and_only_one_attempt_stays_live() {
    let (mut db, _data_dir) = migrated_db("manifest_commit").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let item = seed_pending_work_item(db.pool(), &fixture, fixture.org_a, "orphan").await;
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin orphan candidate transaction");
    sqlx::query(
        r#"INSERT INTO attack_candidates (
               candidate_id,operation_id,organization_id,target,hypothesis,
               hypothesis_hash,technique,rationale,priority,wave,disposition,
               operation_uuid,scope_snapshot_id,wave_run_id,wave_unit_id,
               source_work_item_id,decision_stage_execution_id,
               decision_stage_run_unit_id,decision_deliverable_submission_id,
               decision_stage_kind,target_live_id,target_type_at_time,
               target_value_at_time,target_identity_hash,execution_plan,
               candidate_plan_hash,risk_class
           ) VALUES (
               $1,$2,$3,'https://shared.example.test/login','orphan hypothesis',
               'sha256:orphan','WSTG-INPV-05','orphan','high',0,'proposed',
               $4,$5,$6,$7,$8,$9,$10,$11,'attack_candidate',$12,'url',
               'https://shared.example.test/login','sha256:shared-target',$13,
               'sha256:orphan-plan','exploit'
           )"#,
    )
    .bind(item.candidate_id)
    .bind(fixture.operation_id.to_string())
    .bind(fixture.org_a.organization_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.wave_run_id)
    .bind(fixture.org_a.wave_unit_id)
    .bind(item.work_item_id)
    .bind(fixture.stage_execution_id)
    .bind(fixture.org_a.stage_run_unit_id)
    .bind(fixture.org_a.submission_id)
    .bind(fixture.org_a.target_id)
    .bind(serde_json::json!({"schema_version": "candidate-plan-v1"}))
    .execute(&mut *tx)
    .await
    .expect("stage orphan candidate row");
    assert_sqlstate(
        tx.commit().await,
        "P0001",
        "candidate without same-transaction work-item terminalization",
    );

    let candidate = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    let approval_id = insert_approval(db.pool(), &fixture, candidate, fixture.org_a)
        .await
        .expect("insert approval for live-attempt uniqueness");
    for ordinal in [0_i32, 1_i32] {
        let inserted = sqlx::query(
            r#"INSERT INTO candidate_attempts (
                   id,candidate_id,approval_id,operation_id,scope_snapshot_id,
                   wave_run_id,wave_unit_id,organization_id,target_live_id,
                   target_type_at_time,target_value_at_time,target_identity_hash,
                   candidate_plan_hash,ordinal,status
               ) VALUES (
                   $1,$2,$3,$4,$5,$6,$7,$8,$9,'url',
                   'https://shared.example.test/login','sha256:shared-target',
                   'sha256:candidate-plan',$10,'queued'
               )"#,
        )
        .bind(Uuid::new_v4())
        .bind(candidate.candidate_id)
        .bind(approval_id)
        .bind(fixture.operation_id)
        .bind(fixture.scope_snapshot_id)
        .bind(fixture.wave_run_id)
        .bind(fixture.org_a.wave_unit_id)
        .bind(fixture.org_a.organization_id)
        .bind(fixture.org_a.target_id)
        .bind(ordinal)
        .execute(db.pool())
        .await;
        if ordinal == 0 {
            inserted.expect("insert first live attempt");
        } else {
            assert_sqlstate(
                inserted.map(|_| ()),
                "23505",
                "second live Candidate attempt",
            );
        }
    }
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn finding_lineage_requires_verified_attempt_and_unique_finding() {
    let (mut db, _data_dir) = migrated_db("lineage_authority").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let candidate = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    let approval_id = insert_approval(db.pool(), &fixture, candidate, fixture.org_a)
        .await
        .expect("insert lineage approval");
    let refuted_attempt = insert_attempt(
        db.pool(),
        &fixture,
        candidate,
        approval_id,
        fixture.org_a,
        "refuted",
    )
    .await
    .expect("insert refuted attempt fixture");
    let finding_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO findings(id,title,sev,project_path,target_id)
           VALUES ($1,'refuted lineage','high','/tmp/attack-v2',$2)"#,
    )
    .bind(finding_id)
    .bind(fixture.org_a.target_id)
    .execute(db.pool())
    .await
    .expect("insert hostile finding fixture");
    let lineage = sqlx::query(
        r#"INSERT INTO finding_lineage (
               id,finding_id,candidate_attempt_id,candidate_id,operation_id,
               scope_snapshot_id,wave_run_id,wave_unit_id,organization_id,
               target_live_id,target_type_at_time,target_value_at_time,
               target_identity_hash,candidate_plan_hash
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'url',
               'https://shared.example.test/login','sha256:shared-target',
               'sha256:candidate-plan'
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(finding_id)
    .bind(refuted_attempt)
    .bind(candidate.candidate_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.wave_run_id)
    .bind(fixture.org_a.wave_unit_id)
    .bind(fixture.org_a.organization_id)
    .bind(fixture.org_a.target_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(lineage.map(|_| ()), "P0001", "lineage from refuted attempt");
    let verified_attempt = insert_attempt_with_ordinal(
        db.pool(),
        &fixture,
        candidate,
        approval_id,
        fixture.org_a,
        "running",
        1,
    )
    .await
    .expect("insert running Attempt before linking proof evidence");
    let proof_evidence_id = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_a.organization_id,
        fixture.org_a.target_id,
        "evidence",
    )
    .await;
    sqlx::query(
        "INSERT INTO candidate_attempt_evidence(attempt_id,evidence_id,role)
         VALUES($1,$2,'proof')",
    )
    .bind(verified_attempt)
    .bind(proof_evidence_id)
    .execute(db.pool())
    .await
    .expect("link exact proof evidence");
    sqlx::query(
        "UPDATE candidate_attempts SET status='verified',
             result_json=$2,result_hash='sha256:verified',terminal_at=NOW()
         WHERE id=$1",
    )
    .bind(verified_attempt)
    .bind(serde_json::json!({"disposition": "verified"}))
    .execute(db.pool())
    .await
    .expect("make exact Attempt verified for Finding authority test");
    sqlx::query("UPDATE findings SET evidence=$2 WHERE id=$1")
        .bind(finding_id)
        .bind(serde_json::json!([proof_evidence_id]))
        .execute(db.pool())
        .await
        .expect("attach exact proof projection to legacy Finding");
    let legacy_finding_lineage = sqlx::query(
        r#"INSERT INTO finding_lineage (
               id,finding_id,candidate_attempt_id,candidate_id,operation_id,
               scope_snapshot_id,wave_run_id,wave_unit_id,organization_id,
               target_live_id,target_type_at_time,target_value_at_time,
               target_identity_hash,candidate_plan_hash
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'url',
               'https://shared.example.test/login','sha256:shared-target',
               'sha256:candidate-plan'
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(finding_id)
    .bind(verified_attempt)
    .bind(candidate.candidate_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.wave_run_id)
    .bind(fixture.org_a.wave_unit_id)
    .bind(fixture.org_a.organization_id)
    .bind(fixture.org_a.target_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        legacy_finding_lineage.map(|_| ()),
        "P0001",
        "legacy Finding cannot be reused for Candidate lineage",
    );
    db.stop().await;
}
