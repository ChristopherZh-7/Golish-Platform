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
    begin_candidate_action, candidate_execution_continuation, claim_next_candidate_attempt,
    finish_candidate_action, heartbeat_candidate_execution, record_attempt_submission,
    release_candidate_execution, AttemptEvidenceLink, BeginCandidateAction, CandidateActionStart,
    CandidateClaimQuery, CandidateExecutionContinuation, CandidateExecutionHeartbeat,
    CandidateExecutionRelease, FinishCandidateAction, RecordAttemptSubmission,
};
use golish_db::repo::candidate_recovery::{
    checkpoint_candidate_terminal_barrier, converge_candidate_recovery,
    expire_candidate_starts_before_claim, next_candidate_terminal_intent,
    record_candidate_terminal_intent, recover_candidate_terminal_intent_barrier,
    resolve_candidate_recovery, terminalize_candidate_terminal_intent, CandidateRecoveryResolution,
    CheckpointCandidateTerminalBarrier, RecordCandidateTerminalIntent,
    RecoverCandidateTerminalIntent, ResolveCandidateRecovery, TerminalizeCandidateTerminalIntent,
};
use golish_db::repo::finding_lineage::{
    terminalize_candidate_attempt, terminalize_verified_finding, TerminalizeCandidateAttempt,
    TerminalizeVerifiedFinding,
};
use golish_db::repo::{
    attack_execution_rollout, attack_execution_shadow, attack_wave_consolidations, attack_waves,
    canonical_fact_refs, operation_state, runtime_memory_tx, stage_run_units,
};
use golish_db::{DbConfig, GolishDb};
use serial_test::serial;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool};
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

fn verification_truth_hash_for_payload(
    handoff: &golish_db::repo::stage_handoffs::VerificationStageHandoffRow,
    payload: &serde_json::Value,
) -> String {
    sha256_json(&serde_json::json!({
        "schema_version": 1,
        "operation_id": handoff.operation_id,
        "scope_snapshot_id": handoff.scope_snapshot_id,
        "wave_run_id": handoff.wave_run_id,
        "wave_unit_id": handoff.wave_unit_id,
        "organization_id": handoff.organization_id,
        "canonical_fact_refs": payload["canonical_fact_refs"],
        "typed_claims": payload["typed_claims"],
        "coverage_watermark": payload["coverage_watermark"],
        "evidence_ids": payload["evidence_ids"],
    }))
}

fn refresh_verification_handoff_hashes(
    handoff: &golish_db::repo::stage_handoffs::VerificationStageHandoffRow,
    payload: &mut serde_json::Value,
) -> String {
    let truth_hash = verification_truth_hash_for_payload(handoff, payload);
    payload["verification_truth_hash"] = serde_json::json!(truth_hash);
    sha256_json(payload)
}

async fn try_raw_verification_handoff_insert(
    pool: &PgPool,
    handoff: &golish_db::repo::stage_handoffs::VerificationStageHandoffRow,
    payload: &serde_json::Value,
    payload_sha256: &str,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await.expect("begin raw typed-handoff forgery");
    let result = try_raw_verification_handoff_insert_on_connection(
        &mut tx,
        handoff,
        payload,
        payload_sha256,
        None,
    )
    .await
    .map(|_| ());
    tx.rollback()
        .await
        .expect("rollback raw typed-handoff forgery");
    result
}

async fn try_raw_verification_handoff_insert_with_gate_time(
    pool: &PgPool,
    handoff: &golish_db::repo::stage_handoffs::VerificationStageHandoffRow,
    supplied_gate_passed_at: chrono::DateTime<chrono::Utc>,
) -> Result<chrono::DateTime<chrono::Utc>, sqlx::Error> {
    let mut tx = pool
        .begin()
        .await
        .expect("begin caller-timestamp typed-handoff fixture");
    let result = try_raw_verification_handoff_insert_on_connection(
        &mut tx,
        handoff,
        &handoff.payload,
        &handoff.payload_sha256,
        Some(supplied_gate_passed_at),
    )
    .await;
    tx.rollback()
        .await
        .expect("rollback caller-timestamp typed-handoff fixture");
    result
}

async fn try_unready_verification_handoff_preseed(
    pool: &PgPool,
    handoff: &golish_db::repo::stage_handoffs::VerificationStageHandoffRow,
) -> Result<(), sqlx::Error> {
    let mut tx = pool
        .begin()
        .await
        .expect("begin unready typed-handoff preseed");
    let result = insert_raw_verification_handoff_on_connection(
        &mut tx,
        handoff,
        &handoff.payload,
        &handoff.payload_sha256,
        handoff.wave_unit_row_version_after_close,
        Some(
            chrono::DateTime::parse_from_rfc3339("2000-01-01T00:00:00Z")
                .expect("fixed hostile handoff timestamp")
                .with_timezone(&chrono::Utc),
        ),
    )
    .await
    .map(|_| ());
    tx.rollback()
        .await
        .expect("rollback unready typed-handoff preseed");
    result
}

async fn try_raw_verification_handoff_insert_on_connection(
    connection: &mut PgConnection,
    handoff: &golish_db::repo::stage_handoffs::VerificationStageHandoffRow,
    payload: &serde_json::Value,
    payload_sha256: &str,
    gate_passed_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<chrono::DateTime<chrono::Utc>, sqlx::Error> {
    let row_version: i64 = sqlx::query_scalar(
        "UPDATE attack_wave_units
         SET verification_closed=TRUE,consolidation_status='ready',
             row_version=row_version+1,updated_at=NOW()
         WHERE id=$1 AND wave_run_id=$2 AND operation_id=$3
           AND scope_snapshot_id=$4 AND organization_id=$5
           AND status='verification' AND review_closed
           AND NOT verification_closed AND consolidation_status='pending'
         RETURNING row_version",
    )
    .bind(handoff.wave_unit_id)
    .bind(handoff.wave_run_id)
    .bind(handoff.operation_id)
    .bind(handoff.scope_snapshot_id)
    .bind(handoff.organization_id)
    .fetch_one(&mut *connection)
    .await
    .expect("stage raw WaveUnit close");
    sqlx::query(
        "UPDATE stage_worker_runs SET status='passed',terminal_at=NOW(),updated_at=NOW()
         WHERE id=$1",
    )
    .bind(handoff.primary_worker_run_id)
    .execute(&mut *connection)
    .await
    .expect("stage raw aggregate WorkerRun close");
    sqlx::query(
        "UPDATE stage_run_units SET status='passed',terminal_at=NOW(),updated_at=NOW()
         WHERE id=$1",
    )
    .bind(handoff.source_stage_run_unit_id)
    .execute(&mut *connection)
    .await
    .expect("stage raw Verification StageRunUnit close");
    insert_raw_verification_handoff_on_connection(
        connection,
        handoff,
        payload,
        payload_sha256,
        row_version,
        gate_passed_at,
    )
    .await
}

async fn insert_raw_verification_handoff_on_connection(
    connection: &mut PgConnection,
    handoff: &golish_db::repo::stage_handoffs::VerificationStageHandoffRow,
    payload: &serde_json::Value,
    payload_sha256: &str,
    row_version: i64,
    gate_passed_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<chrono::DateTime<chrono::Utc>, sqlx::Error> {
    sqlx::query_scalar(
        r#"INSERT INTO verification_stage_handoffs(
               id,operation_id,scope_snapshot_id,wave_run_id,wave_unit_id,
               organization_id,stage_execution_id,source_stage_run_unit_id,
               primary_worker_run_id,wave_generation,
               wave_unit_row_version_after_close,payload,payload_sha256,
               evidence_ids,coverage_watermark,verification_truth_hash,gate_passed_at
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,
                    COALESCE($17,NOW()))
           RETURNING gate_passed_at"#,
    )
    .bind(handoff.id)
    .bind(handoff.operation_id)
    .bind(handoff.scope_snapshot_id)
    .bind(handoff.wave_run_id)
    .bind(handoff.wave_unit_id)
    .bind(handoff.organization_id)
    .bind(handoff.stage_execution_id)
    .bind(handoff.source_stage_run_unit_id)
    .bind(handoff.primary_worker_run_id)
    .bind(handoff.wave_generation)
    .bind(row_version)
    .bind(payload)
    .bind(payload_sha256)
    .bind(&handoff.evidence_ids)
    .bind(&handoff.coverage_watermark)
    .bind(payload["verification_truth_hash"].as_str().unwrap())
    .bind(gate_passed_at)
    .fetch_one(&mut *connection)
    .await
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

/// Recreate the pre-cutover rollout state for the three repository tests that
/// exercise each adjacent transition explicitly. Production migrations remain
/// forward-only; this helper is confined to a fresh, isolated embedded test DB.
async fn reset_attack_rollout_to_legacy_for_transition_fixture(pool: &PgPool) {
    sqlx::query(
        "ALTER TABLE attack_execution_rollout
         DISABLE TRIGGER attack_execution_rollout_forward_only",
    )
    .execute(pool)
    .await
    .expect("disable rollout transition trigger for isolated fixture reset");
    sqlx::query(
        "ALTER TABLE attack_execution_rollout
         DISABLE TRIGGER zz_attack_execution_rollout_promotion_receipt",
    )
    .execute(pool)
    .await
    .expect("disable promotion receipt owner for isolated fixture reset");
    sqlx::query(
        "UPDATE attack_execution_rollout
            SET contract='legacy',rank=0,row_version=0,updated_at=NOW()
          WHERE singleton=TRUE",
    )
    .execute(pool)
    .await
    .expect("reset isolated attack rollout fixture");
    sqlx::query(
        "ALTER TABLE attack_execution_rollout
         ENABLE TRIGGER attack_execution_rollout_forward_only",
    )
    .execute(pool)
    .await
    .expect("restore rollout transition trigger after isolated fixture reset");
    sqlx::query(
        "ALTER TABLE attack_execution_rollout
         ENABLE TRIGGER zz_attack_execution_rollout_promotion_receipt",
    )
    .execute(pool)
    .await
    .expect("restore promotion receipt owner after isolated fixture reset");
}

/// Match the isolated deployment singleton to the frozen operation contract
/// used by a fixture. Candidate cohort admission intentionally rejects a Wave
/// whose contract is stale relative to the deployment singleton.
async fn align_attack_rollout_for_fixture(pool: &PgPool, contract: &str) {
    let rank = match contract {
        "legacy" => 0_i16,
        "dual_write_read_legacy" => 1_i16,
        "dual_write_read_v2_fallback" => 2_i16,
        "v2_only" => 3_i16,
        other => panic!("unsupported attack rollout fixture contract: {other}"),
    };
    if rank == 3 {
        // Test-only singleton alignment for pre-existing fixtures. Production
        // promotion must keep the forward, attestation, and receipt gates on.
        sqlx::query(
            "ALTER TABLE runtime_memory_rollout
             DISABLE TRIGGER runtime_memory_rollout_forward_only",
        )
        .execute(pool)
        .await
        .expect("disable runtime rollout transition trigger for v2-only attack fixture");
        sqlx::query(
            "ALTER TABLE runtime_memory_rollout
             DISABLE TRIGGER zz_runtime_memory_rollout_attestation_gate",
        )
        .execute(pool)
        .await
        .expect("disable runtime attestation gate for v2-only attack fixture");
        sqlx::query(
            "ALTER TABLE runtime_memory_rollout
             DISABLE TRIGGER zz_runtime_memory_rollout_promotion_receipt",
        )
        .execute(pool)
        .await
        .expect("disable runtime receipt owner for v2-only attack fixture");
        sqlx::query(
            "UPDATE runtime_memory_rollout
                SET contract='v2_only',contract_rank=3,row_version=3,updated_at=NOW()
              WHERE singleton_id=1",
        )
        .execute(pool)
        .await
        .expect("align runtime rollout required by v2-only attack fixture");
        sqlx::query(
            "ALTER TABLE runtime_memory_rollout
             ENABLE TRIGGER zz_runtime_memory_rollout_attestation_gate",
        )
        .execute(pool)
        .await
        .expect("restore runtime attestation gate after v2-only attack fixture");
        sqlx::query(
            "ALTER TABLE runtime_memory_rollout
             ENABLE TRIGGER zz_runtime_memory_rollout_promotion_receipt",
        )
        .execute(pool)
        .await
        .expect("restore runtime receipt owner after v2-only attack fixture");
        sqlx::query(
            "ALTER TABLE runtime_memory_rollout
             ENABLE TRIGGER runtime_memory_rollout_forward_only",
        )
        .execute(pool)
        .await
        .expect("restore runtime rollout transition trigger after v2-only attack fixture");
    }
    sqlx::query(
        "ALTER TABLE attack_execution_rollout
         DISABLE TRIGGER attack_execution_rollout_forward_only",
    )
    .execute(pool)
    .await
    .expect("disable rollout transition trigger for contract-aligned fixture");
    sqlx::query(
        "ALTER TABLE attack_execution_rollout
         DISABLE TRIGGER zz_attack_execution_rollout_promotion_receipt",
    )
    .execute(pool)
    .await
    .expect("disable receipt owner for contract-aligned fixture");
    sqlx::query(
        "UPDATE attack_execution_rollout
            SET contract=$1,rank=$2,row_version=$2,updated_at=NOW()
          WHERE singleton=TRUE",
    )
    .bind(contract)
    .bind(rank)
    .execute(pool)
    .await
    .expect("align isolated attack rollout fixture contract");
    sqlx::query(
        "ALTER TABLE attack_execution_rollout
         ENABLE TRIGGER attack_execution_rollout_forward_only",
    )
    .execute(pool)
    .await
    .expect("restore rollout transition trigger after contract-aligned fixture");
    sqlx::query(
        "ALTER TABLE attack_execution_rollout
         ENABLE TRIGGER zz_attack_execution_rollout_promotion_receipt",
    )
    .execute(pool)
    .await
    .expect("restore receipt owner after contract-aligned fixture");
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
    generation: i32,
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
           ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'running')"#,
    )
    .bind(stage_run_unit_id)
    .bind(fixture.operation_id)
    .bind(stage_execution_id)
    .bind(fixture.scope_snapshot_id)
    .bind(organization_id)
    .bind(stage_kind)
    .bind(generation)
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

async fn seed_attack_fixture_with_candidate_pass_and_contract(
    pool: &PgPool,
    candidate_final_passed: bool,
    attack_execution_contract: &str,
) -> AttackFixture {
    align_attack_rollout_for_fixture(pool, attack_execution_contract).await;
    let session_id = Uuid::new_v4();
    let operation_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    let scope_decision_id = Uuid::new_v4();
    let scope_snapshot_id = Uuid::new_v4();
    let entry_stage_execution_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    let wave_run_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{operation_id}:candidate-wave:0").as_bytes(),
    );
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
           ) VALUES ($1,'red_team','attack_candidate','v2_only',$3,$2)"#,
    )
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(attack_execution_contract)
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
            1,
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
            0,
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
               $1,$2,$3,0,'open',$4,
               'sha256:66e50329b4bb217eb060bcccb38f78f4b0eafc163471bebd6554d271d1a6b326',
               3,100,3,200
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

async fn seed_attack_fixture_with_candidate_pass(
    pool: &PgPool,
    candidate_final_passed: bool,
) -> AttackFixture {
    seed_attack_fixture_with_candidate_pass_and_contract(pool, candidate_final_passed, "v2_only")
        .await
}

async fn seed_attack_fixture(pool: &PgPool) -> AttackFixture {
    seed_attack_fixture_with_candidate_pass(pool, true).await
}

async fn insert_exact_enumeration_predecessor(pool: &PgPool, fixture: &AttackFixture) {
    let enumeration_stage_execution_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO stage_runs (
               id,operation_id,stage_kind,status,started_at,completed_at
           )
           SELECT $1,$2,'enumeration','completed',
                  started_at-INTERVAL '1 minute',started_at
             FROM stage_runs
            WHERE id=$3 AND operation_id=$2 AND stage_kind='vuln_triage'"#,
    )
    .bind(enumeration_stage_execution_id)
    .bind(fixture.operation_id)
    .bind(fixture.entry_stage_execution_id)
    .execute(pool)
    .await
    .expect("insert exact Enumeration predecessor stage");
    let organizations: Vec<Uuid> = sqlx::query_scalar(
        r#"SELECT organization_id
              FROM stage_run_units
             WHERE operation_id=$1 AND stage_execution_id=$2
               AND stage_kind='vuln_triage' AND status='passed'
             ORDER BY organization_id"#,
    )
    .bind(fixture.operation_id)
    .bind(fixture.entry_stage_execution_id)
    .fetch_all(pool)
    .await
    .expect("load exact Vuln predecessor organizations");
    for (index, organization_id) in organizations.into_iter().enumerate() {
        insert_final_passed_unit(
            pool,
            fixture,
            organization_id,
            enumeration_stage_execution_id,
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            "enumeration",
            "enumerator",
            2,
            i32::try_from(index + 1).expect("Enumeration fixture ordinal fits i32"),
            true,
        )
        .await;
    }
    sqlx::query(
        r#"UPDATE stage_runs AS candidate
              SET started_at=vuln.completed_at
             FROM stage_runs AS vuln
            WHERE candidate.id=$1
              AND candidate.operation_id=$2
              AND candidate.stage_kind='attack_candidate'
              AND vuln.id=$3
              AND vuln.operation_id=candidate.operation_id
              AND vuln.stage_kind='vuln_triage'
              AND vuln.status='completed'
              AND vuln.completed_at IS NOT NULL"#,
    )
    .bind(fixture.stage_execution_id)
    .bind(fixture.operation_id)
    .bind(fixture.entry_stage_execution_id)
    .execute(pool)
    .await
    .expect("align Candidate start with exact Vuln predecessor completion");
}

#[tokio::test]
#[serial]
async fn current_wave_authority_separates_wave_zero_from_predecessor_generation() {
    let (mut db, _data_dir) = migrated_db("current_wave_initial_authority").await;
    let fixture = seed_attack_fixture_with_candidate_pass_and_contract(
        db.pool(),
        true,
        "dual_write_read_legacy",
    )
    .await;

    sqlx::query(
        "ALTER TABLE attack_execution_candidate_admissions
         DISABLE TRIGGER attack_execution_candidate_admission_internal_only",
    )
    .execute(db.pool())
    .await
    .expect("disable Candidate admission guard for isolated pre-Wave fixture rewind");
    sqlx::query("DELETE FROM attack_execution_candidate_admissions WHERE operation_id=$1")
        .bind(fixture.operation_id)
        .execute(db.pool())
        .await
        .expect("remove isolated prebuilt Candidate admission");
    sqlx::query(
        "ALTER TABLE attack_execution_candidate_admissions
         ENABLE TRIGGER attack_execution_candidate_admission_internal_only",
    )
    .execute(db.pool())
    .await
    .expect("restore Candidate admission guard after isolated fixture rewind");
    sqlx::query("DELETE FROM attack_wave_units WHERE wave_run_id=$1")
        .bind(fixture.wave_run_id)
        .execute(db.pool())
        .await
        .expect("remove prebuilt WaveUnits to exercise initial authority");
    sqlx::query("DELETE FROM attack_wave_runs WHERE id=$1")
        .bind(fixture.wave_run_id)
        .execute(db.pool())
        .await
        .expect("remove prebuilt Wave to exercise initial authority");

    let root_organization_id: Uuid = sqlx::query_scalar(
        "SELECT root_organization_id FROM operation_org_scope_snapshots WHERE id=$1",
    )
    .bind(fixture.scope_snapshot_id)
    .fetch_one(db.pool())
    .await
    .expect("load frozen root organization");
    let root_target_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO targets (
               id,name,target_type,value,scope,project_path,organization_id
           ) VALUES (
               $1,'Root app','url','https://root.example.test/','in','/tmp/attack-v2',$2
           )"#,
    )
    .bind(root_target_id)
    .bind(root_organization_id)
    .execute(db.pool())
    .await
    .expect("insert root predecessor target");
    let root_stage_run_unit_id = Uuid::new_v4();
    let root_submission_id = Uuid::new_v4();
    insert_final_passed_unit(
        db.pool(),
        &fixture,
        root_organization_id,
        fixture.entry_stage_execution_id,
        root_stage_run_unit_id,
        Uuid::new_v4(),
        Uuid::new_v4(),
        root_submission_id,
        "vuln_triage",
        "formulaic_scanner",
        1,
        0,
        true,
    )
    .await;

    let authority = attack_waves::load_current_authority(db.pool(), fixture.operation_id)
        .await
        .expect("sealed generation-one predecessor truth must derive initial Wave authority");
    let initial = match authority {
        attack_waves::AttackWaveAuthority::Initial(initial) => initial,
        other => panic!("expected initial Wave authority, got {other:?}"),
    };
    assert_eq!(initial.operation_id, fixture.operation_id);
    assert_eq!(initial.scope_snapshot_id, fixture.scope_snapshot_id);
    assert_eq!(initial.generation, 0, "initial generation is DB-derived");
    assert_eq!(initial.predecessor_generation, 1);
    assert_eq!(
        initial.predecessor_stage_execution_id,
        fixture.entry_stage_execution_id
    );
    assert_eq!(initial.units.len(), 3);
    assert_eq!(
        initial
            .units
            .iter()
            .map(|unit| (unit.ordinal, unit.organization_id))
            .collect::<Vec<_>>(),
        vec![
            (0, root_organization_id),
            (1, fixture.org_a.organization_id),
            (2, fixture.org_b.organization_id),
        ]
    );
    assert!(initial.units.iter().all(|unit| {
        matches!(
            unit.entry,
            attack_waves::AttackWaveEntry::VulnTriageHandoff { .. }
        ) && !unit.evidence_ids.is_empty()
    }));

    let wave_run_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{}:candidate-wave:0", fixture.operation_id).as_bytes(),
    );
    let mut open_tx = db.pool().begin().await.expect("begin initial Wave open");
    for unit in &initial.units {
        let attack_waves::AttackWaveEntry::VulnTriageHandoff {
            stage_execution_id,
            stage_run_unit_id,
            deliverable_submission_id,
        } = unit.entry
        else {
            panic!("initial authority must contain only vuln_triage handoffs");
        };
        attack_waves::open_from_vuln_triage_handoff(
            &mut open_tx,
            &attack_waves::OpenAttackWaveUnit {
                wave_run_id,
                wave_unit_id: Uuid::new_v4(),
                operation_id: fixture.operation_id,
                scope_snapshot_id: fixture.scope_snapshot_id,
                organization_id: unit.organization_id,
                entry_stage_execution_id: stage_execution_id,
                entry_stage_run_unit_id: stage_run_unit_id,
                entry_deliverable_submission_id: deliverable_submission_id,
                generation: initial.generation,
                ordinal: unit.ordinal,
                policy_snapshot: serde_json::json!({
                    "max_waves": 3,
                    "max_candidates_total": 100,
                    "max_chain_depth": 3,
                    "max_attempts_total": 200
                }),
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
        .expect("open one exact initial WaveUnit");
    }
    open_tx.commit().await.expect("commit initial Wave open");

    let authority = attack_waves::load_current_authority(db.pool(), fixture.operation_id)
        .await
        .expect("opened Wave must become current authority");
    let current = match authority {
        attack_waves::AttackWaveAuthority::Current(current) => current,
        other => panic!("expected current Wave authority, got {other:?}"),
    };
    assert_eq!(current.wave.id, wave_run_id);
    assert_eq!(current.wave.generation, 0);
    assert_eq!(current.units.len(), 3);
    assert!(current.units.iter().all(|unit| matches!(
        unit.state,
        attack_waves::CurrentAttackWaveUnitState::AwaitingManifest
    )));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn partial_initial_wave_with_frozen_manifest_recovers_and_replay_completes_scope() {
    use golish_db::repo::attack_candidate_work_items::{
        seed_wave_work_items, SeedAttackObservation, SeedAttackWorkItems,
    };

    let (mut db, _data_dir) = migrated_db("partial_initial_wave_recovery").await;
    let fixture = seed_attack_fixture(db.pool()).await;

    sqlx::query("DELETE FROM attack_wave_units WHERE wave_run_id=$1")
        .bind(fixture.wave_run_id)
        .execute(db.pool())
        .await
        .expect("remove prebuilt WaveUnits to exercise partial initial recovery");
    sqlx::query("DELETE FROM attack_wave_runs WHERE id=$1")
        .bind(fixture.wave_run_id)
        .execute(db.pool())
        .await
        .expect("remove prebuilt Wave to exercise partial initial recovery");

    let root_organization_id: Uuid = sqlx::query_scalar(
        "SELECT root_organization_id FROM operation_org_scope_snapshots WHERE id=$1",
    )
    .bind(fixture.scope_snapshot_id)
    .fetch_one(db.pool())
    .await
    .expect("load frozen root organization");
    let root_target_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO targets (
               id,name,target_type,value,scope,project_path,organization_id
           ) VALUES (
               $1,'Recovery root app','url','https://recovery-root.example.test/',
               'in','/tmp/attack-v2',$2
           )"#,
    )
    .bind(root_target_id)
    .bind(root_organization_id)
    .execute(db.pool())
    .await
    .expect("insert root recovery target");
    insert_final_passed_unit(
        db.pool(),
        &fixture,
        root_organization_id,
        fixture.entry_stage_execution_id,
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        "vuln_triage",
        "formulaic_scanner",
        1,
        0,
        true,
    )
    .await;
    insert_final_passed_unit(
        db.pool(),
        &fixture,
        root_organization_id,
        fixture.stage_execution_id,
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        "attack_candidate",
        "attack_analyst",
        0,
        0,
        false,
    )
    .await;
    insert_exact_enumeration_predecessor(db.pool(), &fixture).await;

    let initial = match attack_waves::load_current_authority(db.pool(), fixture.operation_id)
        .await
        .expect("complete predecessor truth derives initial authority")
    {
        attack_waves::AttackWaveAuthority::Initial(initial) => initial,
        other => panic!("expected initial authority, got {other:?}"),
    };
    assert_eq!(initial.units.len(), 3);

    let wave_run_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{}:candidate-wave:0", fixture.operation_id).as_bytes(),
    );
    let policy_snapshot = serde_json::json!({
        "max_attempts_total": 200,
        "max_candidates_total": 100,
        "max_chain_depth": 3,
        "max_waves": 3,
    });
    let policy_hash = format!("sha256:{}", sha256_json(&policy_snapshot));
    let seed_one = |unit: &attack_waves::InitialAttackWaveUnitAuthority| {
        let (target_live_id, target_value_at_time) = if unit.organization_id == root_organization_id
        {
            (
                root_target_id,
                "https://recovery-root.example.test/".to_string(),
            )
        } else if unit.organization_id == fixture.org_a.organization_id {
            (
                fixture.org_a.target_id,
                "https://shared.example.test/login".to_string(),
            )
        } else if unit.organization_id == fixture.org_b.organization_id {
            (
                fixture.org_b.target_id,
                "https://shared.example.test/login".to_string(),
            )
        } else {
            panic!("unexpected frozen organization in initial recovery authority");
        };
        let wave_unit_id = Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!("{wave_run_id}:{}", unit.organization_id).as_bytes(),
        );
        let attack_waves::AttackWaveEntry::VulnTriageHandoff {
            stage_execution_id,
            stage_run_unit_id,
            deliverable_submission_id,
        } = unit.entry
        else {
            panic!("initial recovery authority must contain vuln_triage handoffs");
        };
        (
            wave_unit_id,
            attack_waves::OpenAttackWaveUnit {
                wave_run_id,
                wave_unit_id,
                operation_id: fixture.operation_id,
                scope_snapshot_id: fixture.scope_snapshot_id,
                organization_id: unit.organization_id,
                entry_stage_execution_id: stage_execution_id,
                entry_stage_run_unit_id: stage_run_unit_id,
                entry_deliverable_submission_id: deliverable_submission_id,
                generation: 0,
                ordinal: unit.ordinal,
                policy_snapshot: policy_snapshot.clone(),
                policy_hash: policy_hash.clone(),
                max_waves: 3,
                max_candidates_total: 100,
                max_chain_depth: 3,
                max_attempts_total: 200,
            },
            SeedAttackWorkItems {
                operation_id: fixture.operation_id,
                scope_snapshot_id: fixture.scope_snapshot_id,
                wave_run_id,
                wave_unit_id,
                organization_id: unit.organization_id,
                observations: vec![SeedAttackObservation {
                    work_item_key: format!("recovery:{}", unit.organization_id),
                    target_live_id: Some(target_live_id),
                    target_type_at_time: "url".to_string(),
                    target_value_at_time,
                    target_identity_hash: format!("sha256:{}", unit.organization_id.simple()),
                    technique: "WSTG-INPV-05".to_string(),
                    observation: serde_json::json!({"outcome": "found"}),
                    observation_hash: format!(
                        "sha256:recovery-observation-{}",
                        unit.organization_id.simple()
                    ),
                    source_fact_delta_id: None,
                    delta_kind: None,
                    observation_kind: "legacy_observation".to_string(),
                    allowed_techniques: vec!["WSTG-INPV-05".to_string()],
                    enrichment_required: false,
                    evidence_ids: vec![unit.evidence_ids[0]],
                }],
            },
        )
    };

    let (_, first_open, first_manifest) = seed_one(&initial.units[0]);
    let mut first_tx = db
        .pool()
        .begin()
        .await
        .expect("begin first per-organization manifest transaction");
    attack_waves::open_from_vuln_triage_handoff(&mut first_tx, &first_open)
        .await
        .expect("open first deterministic initial WaveUnit");
    seed_wave_work_items(&mut first_tx, first_manifest)
        .await
        .expect("freeze first non-empty manifest atomically");
    first_tx
        .commit()
        .await
        .expect("commit first partial initial WaveUnit and manifest");

    let recovered = match attack_waves::load_current_authority(db.pool(), fixture.operation_id)
        .await
        .expect("strict partial initial state must recover full initial authority")
    {
        attack_waves::AttackWaveAuthority::Initial(initial) => initial,
        other => panic!("expected recovered initial authority, got {other:?}"),
    };
    assert_eq!(
        recovered
            .units
            .iter()
            .map(|unit| (unit.ordinal, unit.organization_id))
            .collect::<Vec<_>>(),
        initial
            .units
            .iter()
            .map(|unit| (unit.ordinal, unit.organization_id))
            .collect::<Vec<_>>()
    );

    for unit in &recovered.units {
        let (_, open, manifest) = seed_one(unit);
        let mut replay_tx = db
            .pool()
            .begin()
            .await
            .expect("begin replayed per-organization manifest transaction");
        attack_waves::open_from_vuln_triage_handoff(&mut replay_tx, &open)
            .await
            .expect("replay or create deterministic initial WaveUnit");
        seed_wave_work_items(&mut replay_tx, manifest)
            .await
            .expect("replay or freeze deterministic non-empty manifest");
        replay_tx
            .commit()
            .await
            .expect("commit replayed initial organization seed");
    }

    let current = match attack_waves::load_current_authority(db.pool(), fixture.operation_id)
        .await
        .expect("completed replay must expose full current Wave")
    {
        attack_waves::AttackWaveAuthority::Current(current) => current,
        other => panic!("expected complete current Wave, got {other:?}"),
    };
    assert_eq!(current.wave.id, wave_run_id);
    assert_eq!(current.units.len(), 3);
    assert!(current.units.iter().all(|unit| matches!(
        unit.state,
        attack_waves::CurrentAttackWaveUnitState::Runnable { .. }
    )));

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn current_wave_authority_rejects_incomplete_frozen_org_coverage() {
    let (mut db, _data_dir) = migrated_db("current_wave_incomplete_scope").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let error = attack_waves::load_current_authority(db.pool(), fixture.operation_id)
        .await
        .expect_err("a current Wave missing the frozen root unit must fail closed");
    assert!(
        error
            .to_string()
            .contains("attack_wave_initial_predecessor_scope_mismatch"),
        "incomplete Wave must return a stable fail-closed reason: {error}"
    );
    db.stop().await;
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
    insert_exact_enumeration_predecessor(db.pool(), &fixture).await;
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
                source_fact_delta_id: None,
                delta_kind: None,
                observation_kind: "legacy_observation".to_string(),
                allowed_techniques: vec!["WSTG-INPV-05".to_string()],
                enrichment_required: false,
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
                source_fact_delta_id: None,
                delta_kind: None,
                observation_kind: "legacy_observation".to_string(),
                allowed_techniques: vec!["WSTG-INPV-05".to_string()],
                enrichment_required: false,
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
    let fixture = seed_attack_fixture_with_candidate_pass_and_contract(
        db.pool(),
        false,
        "dual_write_read_legacy",
    )
    .await;
    // The frozen Candidate identity may be a contextual URL while the live
    // pointer is the in-scope base asset that produced the observation.
    sqlx::query("UPDATE targets SET target_type='domain',value='shared.example.test' WHERE id=$1")
        .bind(fixture.org_a.target_id)
        .execute(db.pool())
        .await
        .expect("model contextual URL on an in-scope base target");
    insert_exact_enumeration_predecessor(db.pool(), &fixture).await;
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
                target_identity_hash:
                    "sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963"
                        .to_string(),
                technique: "WSTG-INPV-05".to_string(),
                observation: serde_json::json!({"outcome": "found"}),
                observation_hash: "sha256:atomic-observation".to_string(),
                source_fact_delta_id: None,
                delta_kind: None,
                observation_kind: "legacy_observation".to_string(),
                allowed_techniques: vec!["WSTG-INPV-05".to_string()],
                enrichment_required: false,
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
        "target_identity_hash": "sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963",
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

    let sample = attack_execution_shadow::load_unit_sample(
        db.pool(),
        fixture.operation_id,
        fixture.org_a.stage_run_unit_id,
    )
    .await
    .expect("load dual-write shadow sample")
    .expect("dual-write final seal must persist one whole-record shadow sample");
    assert_eq!(sample.contract, "dual_write_read_legacy");
    let legacy_record = sample
        .legacy_record
        .as_ref()
        .expect("dual-write sample has one complete legacy record");
    assert!(matches!(
        sample.v2_record,
        attack_execution_shadow::AttackShadowV2ReadRow::Complete(ref record)
            if record == legacy_record
    ));
    assert_eq!(legacy_record.review_counts.wave_unit_count, 1);
    assert_eq!(legacy_record.decisions.len(), 1);
    assert_eq!(sample.comparison.as_deref(), Some("match"));
    assert_eq!(sample.selected_source.as_deref(), Some("legacy"));

    let mut wrong_source_tx = db.pool().begin().await.expect("begin hostile source");
    let wrong_source = attack_execution_shadow::record_unit_selection_with_connection(
        &mut wrong_source_tx,
        fixture.operation_id,
        fixture.org_a.stage_run_unit_id,
        "match",
        "v2",
    )
    .await
    .expect_err("dual-read-legacy cannot attest a caller-selected V2 source");
    assert!(wrong_source
        .to_string()
        .contains("selected source violates the frozen contract"));
    wrong_source_tx
        .rollback()
        .await
        .expect("rollback hostile source");

    let mut selection_tx = db.pool().begin().await.expect("begin shadow selection");
    attack_execution_shadow::record_unit_selection_with_connection(
        &mut selection_tx,
        fixture.operation_id,
        fixture.org_a.stage_run_unit_id,
        "match",
        "legacy",
    )
    .await
    .expect("persist server-validated whole-record match");
    selection_tx
        .commit()
        .await
        .expect("commit shadow selection");

    let rewrite = sqlx::query(
        "UPDATE attack_execution_shadow_reads
            SET comparison='mismatch',updated_at=NOW()
          WHERE operation_id=$1 AND stage_run_unit_id=$2",
    )
    .bind(fixture.operation_id)
    .bind(fixture.org_a.stage_run_unit_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        rewrite.map(|_| ()),
        "23514",
        "rewrite closed shadow attestation",
    );
    let touch_only = sqlx::query(
        "UPDATE attack_execution_shadow_reads
            SET updated_at=updated_at+INTERVAL '1 second'
          WHERE operation_id=$1 AND stage_run_unit_id=$2",
    )
    .bind(fixture.operation_id)
    .bind(fixture.org_a.stage_run_unit_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        touch_only.map(|_| ()),
        "23514",
        "touch closed shadow attestation timestamp",
    );
    let direct_delete = sqlx::query(
        "DELETE FROM attack_execution_shadow_reads
          WHERE operation_id=$1 AND stage_run_unit_id=$2",
    )
    .bind(fixture.operation_id)
    .bind(fixture.org_a.stage_run_unit_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        direct_delete.map(|_| ()),
        "23514",
        "delete retained shadow sample directly",
    );

    let legacy_json = serde_json::to_value(legacy_record).expect("serialize legacy mirror");
    let wrong_org = sqlx::query(
        r#"INSERT INTO attack_execution_shadow_reads (
               stage_run_unit_id,operation_id,stage_execution_id,organization_id,
               attack_execution_contract,legacy_record,legacy_record_hash
           ) VALUES ($1,$2,$3,$4,'dual_write_read_legacy',$5,$6)"#,
    )
    .bind(fixture.org_b.stage_run_unit_id)
    .bind(fixture.operation_id)
    .bind(fixture.stage_execution_id)
    .bind(fixture.org_a.organization_id)
    .bind(&legacy_json)
    .bind(sha256_json(&legacy_json))
    .execute(db.pool())
    .await;
    assert_sqlstate(
        wrong_org.map(|_| ()),
        "23514",
        "bind shadow sample to a sibling Unit's organization",
    );

    let wrong_contract = sqlx::query(
        r#"INSERT INTO attack_execution_shadow_reads (
               stage_run_unit_id,operation_id,stage_execution_id,organization_id,
               attack_execution_contract,legacy_record,legacy_record_hash
           ) VALUES ($1,$2,$3,$4,'dual_write_read_v2_fallback',$5,$6)"#,
    )
    .bind(fixture.org_b.stage_run_unit_id)
    .bind(fixture.operation_id)
    .bind(fixture.stage_execution_id)
    .bind(fixture.org_b.organization_id)
    .bind(&legacy_json)
    .bind(sha256_json(&legacy_json))
    .execute(db.pool())
    .await;
    assert_sqlstate(
        wrong_contract.map(|_| ()),
        "23514",
        "bind shadow sample to a non-frozen operation contract",
    );

    let root_organization_id: Uuid = sqlx::query_scalar(
        "SELECT root_organization_id FROM operation_org_scope_snapshots WHERE id=$1",
    )
    .bind(fixture.scope_snapshot_id)
    .fetch_one(db.pool())
    .await
    .expect("load root organization for cascade fixture");
    let cleanup_unit_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO stage_run_units (
               id,operation_id,stage_execution_id,scope_snapshot_id,
               organization_id,stage_kind,generation,specialist,status,
               started_at,terminal_at
           ) VALUES (
               $1,$2,$3,$4,$5,'attack_candidate',0,'attack_analyst','passed',NOW(),NOW()
           )"#,
    )
    .bind(cleanup_unit_id)
    .bind(fixture.operation_id)
    .bind(fixture.stage_execution_id)
    .bind(fixture.scope_snapshot_id)
    .bind(root_organization_id)
    .execute(db.pool())
    .await
    .expect("insert isolated final-passed Candidate Unit for cleanup cascade");
    let legacy_record_hash = sha256_json(&legacy_json);
    let pre_attested_insert = sqlx::query(
        r#"INSERT INTO attack_execution_shadow_reads (
               stage_run_unit_id,operation_id,stage_execution_id,organization_id,
               attack_execution_contract,legacy_record,legacy_record_hash,
               comparison,selected_source,selected_record_hash,compared_at
           ) VALUES (
               $1,$2,$3,$4,'dual_write_read_legacy',$5,$6,
               'match','legacy',$6,NOW()
           )"#,
    )
    .bind(cleanup_unit_id)
    .bind(fixture.operation_id)
    .bind(fixture.stage_execution_id)
    .bind(root_organization_id)
    .bind(&legacy_json)
    .bind(&legacy_record_hash)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        pre_attested_insert.map(|_| ()),
        "23514",
        "insert an already-attested shadow row without the repository selector",
    );
    let status_only_preseed = sqlx::query(
        r#"INSERT INTO attack_execution_shadow_reads (
               stage_run_unit_id,operation_id,stage_execution_id,organization_id,
               attack_execution_contract,legacy_record,legacy_record_hash
           ) VALUES ($1,$2,$3,$4,'dual_write_read_legacy',$5,$6)"#,
    )
    .bind(cleanup_unit_id)
    .bind(fixture.operation_id)
    .bind(fixture.stage_execution_id)
    .bind(root_organization_id)
    .bind(&legacy_json)
    .bind(&legacy_record_hash)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        status_only_preseed.map(|_| ()),
        "23514",
        "preseed a shadow owner using only a raw passed Unit without exact handoff authority",
    );
    sqlx::query("DELETE FROM stage_run_units WHERE id=$1 AND operation_id=$2")
        .bind(cleanup_unit_id)
        .bind(fixture.operation_id)
        .execute(db.pool())
        .await
        .expect("cleanup status-only Unit after rejected shadow preseed");
    let cleanup_sample_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM attack_execution_shadow_reads WHERE stage_run_unit_id=$1",
    )
    .bind(cleanup_unit_id)
    .fetch_one(db.pool())
    .await
    .expect("count rejected deployment sample");
    assert_eq!(cleanup_sample_count, 0);

    reset_attack_rollout_to_legacy_for_transition_fixture(db.pool()).await;
    let mut enable_tx = db.pool().begin().await.expect("begin dual rollout enable");
    let dual = attack_execution_rollout::promote_attack_execution_rollout(
        &mut enable_tx,
        0,
        AttackExecutionContract::DualWriteReadLegacy,
    )
    .await
    .expect("enable dual sample production");
    enable_tx
        .commit()
        .await
        .expect("commit dual rollout enable");
    let mut promote_tx = db.pool().begin().await.expect("begin persisted promotion");
    let cohort_error = attack_execution_rollout::promote_attack_execution_rollout(
        &mut promote_tx,
        dual.row_version,
        AttackExecutionContract::DualWriteReadV2Fallback,
    )
    .await
    .expect_err("an open Candidate Wave cannot be promoted despite a matching partial sample");
    assert!(
        cohort_error
            .to_string()
            .contains("attack_rollout_cohort_not_ready"),
        "unexpected open-cohort promotion error: {cohort_error}"
    );
    promote_tx
        .rollback()
        .await
        .expect("rollback blocked open-cohort promotion");
    let retained_contract: String = sqlx::query_scalar(
        "SELECT attack_execution_contract FROM attack_execution_shadow_reads
          WHERE operation_id=$1 AND stage_run_unit_id=$2",
    )
    .bind(fixture.operation_id)
    .bind(fixture.org_a.stage_run_unit_id)
    .fetch_one(db.pool())
    .await
    .expect("read historical sample contract after default promotion");
    assert_eq!(retained_contract, "dual_write_read_legacy");

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
            source_fact_delta_id: None,
            delta_kind: None,
            observation_kind: "legacy_observation".to_string(),
            allowed_techniques: vec!["WSTG-INPV-05".to_string()],
            enrichment_required: false,
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
    seed_candidate_with_support(pool, fixture, org, &[]).await
}

async fn seed_candidate_with_support(
    pool: &PgPool,
    fixture: &AttackFixture,
    org: OrgFixture,
    support_evidence_ids: &[i64],
) -> CandidateFixture {
    let seed_id = Uuid::new_v4();
    let work_item_id = Uuid::new_v4();
    let candidate_id = Uuid::new_v4();
    let execution_plan = serde_json::json!({
        "schema_version": "candidate-plan-v1",
        "classifier_version": "candidate-classifier-v1",
        "target_identity_hash": "sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963",
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
    });
    assert_eq!(
        canonical_execution_plan_hash(&execution_plan).expect("hash fixture Candidate plan"),
        "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5"
    );
    sqlx::query(
        r#"INSERT INTO attack_candidate_seeds (
               id,wave_unit_id,operation_id,scope_snapshot_id,organization_id,
               target_live_id,target_type_at_time,target_value_at_time,
               target_identity_hash,technique,observation,observation_hash
           ) VALUES (
               $1,$2,$3,$4,$5,$6,'url','https://shared.example.test/login',
               'sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963','WSTG-INPV-05',$7,'sha256:observation'
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
               'sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963',$8
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
               'url','https://shared.example.test/login','sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963',$13,
               'sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5','exploit'
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
    .bind(execution_plan)
    .execute(&mut *acceptance_tx)
    .await
    .expect("accept candidate after final gate pass");
    for evidence_id in support_evidence_ids {
        sqlx::query(
            "INSERT INTO attack_candidate_evidence(candidate_id,evidence_id,role)
             VALUES($1,$2,'support')",
        )
        .bind(candidate_id)
        .bind(evidence_id)
        .execute(&mut *acceptance_tx)
        .await
        .expect("attach Candidate support before decision freeze");
    }
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

#[derive(Debug)]
struct PendingShadowFixture {
    legacy_record_hash: String,
    v2_record_hash: String,
}

async fn freeze_shadow_fixture_manifest(pool: &PgPool, fixture: &AttackFixture, org: OrgFixture) {
    let (manifest_projection, manifest_count): (serde_json::Value, i64) = sqlx::query_as(
        r#"SELECT COALESCE(jsonb_agg(
                   jsonb_build_object(
                       'evidence_ids',item_source.evidence_ids,
                       'observation',item_source.observation,
                       'observation_hash',item_source.observation_hash,
                       'target_identity_hash',item_source.target_identity_hash,
                       'technique',item_source.technique,
                       'work_item_id',item_source.work_item_id,
                       'work_item_key',item_source.work_item_key
                   ) ORDER BY item_source.work_item_key,item_source.work_item_id
               ),'[]'::jsonb),COUNT(*)
             FROM (
                 SELECT item.id AS work_item_id,item.work_item_key,
                        item.target_identity_hash,seed.technique,
                        seed.observation,seed.observation_hash,
                        COALESCE((
                            SELECT jsonb_agg(source.evidence_id ORDER BY source.evidence_id)
                              FROM (
                                  SELECT evidence_id FROM attack_candidate_seed_evidence
                                   WHERE seed_id=item.seed_id
                                  UNION
                                  SELECT evidence_id FROM attack_candidate_work_item_evidence
                                   WHERE work_item_id=item.id
                                     AND role IN ('observation','support')
                              ) AS source
                        ),'[]'::jsonb) AS evidence_ids
                   FROM attack_candidate_work_items AS item
                   JOIN attack_candidate_seeds AS seed ON seed.id=item.seed_id
                  WHERE item.operation_id=$1 AND item.wave_unit_id=$2
                    AND item.organization_id=$3
             ) AS item_source"#,
    )
    .bind(fixture.operation_id)
    .bind(org.wave_unit_id)
    .bind(org.organization_id)
    .fetch_one(pool)
    .await
    .expect("rebuild exact shadow fixture manifest");
    let manifest_hash = format!("sha256:{}", sha256_json(&manifest_projection));
    sqlx::query(
        "UPDATE attack_wave_units
            SET manifest_count=$2,manifest_hash=$3,
                manifest_frozen_at=NOW(),updated_at=NOW()
          WHERE id=$1 AND manifest_count IS NULL",
    )
    .bind(org.wave_unit_id)
    .bind(i32::try_from(manifest_count).expect("bounded shadow manifest"))
    .bind(manifest_hash)
    .execute(pool)
    .await
    .expect("freeze shadow fixture manifest count");
}

async fn legacy_record_for_seeded_candidate(
    pool: &PgPool,
    candidate: CandidateFixture,
) -> serde_json::Value {
    type PersistedCandidateProjection = (
        String,
        Option<String>,
        String,
        serde_json::Value,
        String,
        String,
        serde_json::Value,
        String,
        String,
        Vec<i64>,
    );

    let work_item_key: String =
        sqlx::query_scalar("SELECT work_item_key FROM attack_candidate_work_items WHERE id=$1")
            .bind(candidate.work_item_id)
            .fetch_one(pool)
            .await
            .expect("load shadow work-item key");
    let persisted: PersistedCandidateProjection = sqlx::query_as(
        r#"SELECT hypothesis,technique,rationale,prior_refs,suggested_approach,
                  priority,execution_plan,candidate_plan_hash,risk_class,
                  COALESCE((
                      SELECT array_agg(link.evidence_id ORDER BY link.evidence_id)
                        FROM attack_candidate_evidence AS link
                       WHERE link.candidate_id=candidate.candidate_id AND link.role='support'
                  ),ARRAY[]::BIGINT[])
             FROM attack_candidates AS candidate WHERE candidate_id=$1"#,
    )
    .bind(candidate.candidate_id)
    .fetch_one(pool)
    .await
    .expect("load shadow Candidate projection");
    let payload = serde_json::json!({
        "work_item_id": candidate.work_item_id,
        "candidate_id": candidate.candidate_id,
        "hypothesis": persisted.0,
        "technique": persisted.1,
        "rationale": persisted.2,
        "prior_refs": persisted.3,
        "suggested_approach": persisted.4,
        "priority": persisted.5,
        "execution_plan": persisted.6,
        "candidate_plan_hash": persisted.7,
        "risk_class": persisted.8,
        "evidence_ids": persisted.9,
    });
    serde_json::json!({
        "decisions": [{
            "work_item_key": work_item_key,
            "kind": "candidate",
            "semantic_hash": sha256_json(&payload),
        }],
        "review_counts": {
            "wave_unit_count": 1,
            "review_closed_unit_count": 0,
            "candidate_decision_count": 1,
            "no_candidate_decision_count": 0,
        }
    })
}

async fn seed_pending_shadow_fixture_from_current_v2(
    pool: &PgPool,
    fixture: &AttackFixture,
    org: OrgFixture,
    contract: &str,
    comparison: &str,
) -> PendingShadowFixture {
    let evidence_id: i64 = sqlx::query_scalar(
        "SELECT evidence_ids[1] FROM stage_handoffs WHERE source_stage_run_unit_id=$1",
    )
    .bind(org.entry_stage_run_unit_id)
    .fetch_one(pool)
    .await
    .expect("load exact predecessor evidence for shadow fixture");
    seed_candidate_with_support(pool, fixture, org, &[evidence_id]).await;
    freeze_shadow_fixture_manifest(pool, fixture, org).await;
    let mut probe_tx = pool.begin().await.expect("begin V2 projection probe");
    let loaded = attack_execution_shadow::load_v2_record_with_connection(
        &mut probe_tx,
        fixture.operation_id,
        org.stage_run_unit_id,
    )
    .await
    .expect("load authoritative V2 shadow fixture");
    let attack_execution_shadow::AttackShadowV2ReadRow::Complete(mut mirror) = loaded else {
        panic!("seeded V2 fixture must be complete");
    };
    let v2_record_hash =
        sha256_json(&serde_json::to_value(&mirror).expect("serialize authoritative V2 projection"));
    probe_tx
        .rollback()
        .await
        .expect("rollback temporary projection probe");
    if comparison == "mismatch" {
        mirror.decisions[0].semantic_hash = "0".repeat(64);
    }
    let mirror_json = serde_json::to_value(&mirror).expect("serialize semantic mirror");
    let legacy_record_hash = sha256_json(&mirror_json);
    let wrong_hash = sqlx::query(
        r#"INSERT INTO attack_execution_shadow_reads (
               stage_run_unit_id,operation_id,stage_execution_id,organization_id,
               attack_execution_contract,legacy_record,legacy_record_hash
           ) VALUES ($1,$2,$3,$4,$5,$6,$7)"#,
    )
    .bind(org.stage_run_unit_id)
    .bind(fixture.operation_id)
    .bind(fixture.stage_execution_id)
    .bind(org.organization_id)
    .bind(contract)
    .bind(&mirror_json)
    .bind("0".repeat(64))
    .execute(pool)
    .await;
    assert_sqlstate(
        wrong_hash.map(|_| ()),
        "23514",
        "submit a caller hash that differs from canonical legacy JSON",
    );
    sqlx::query(
        r#"INSERT INTO attack_execution_shadow_reads (
               stage_run_unit_id,operation_id,stage_execution_id,organization_id,
               attack_execution_contract,legacy_record,legacy_record_hash,
               created_at,updated_at
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,
               '2000-01-01 00:00:00+00'::TIMESTAMPTZ,
               '2100-01-01 00:00:00+00'::TIMESTAMPTZ
           )"#,
    )
    .bind(org.stage_run_unit_id)
    .bind(fixture.operation_id)
    .bind(fixture.stage_execution_id)
    .bind(org.organization_id)
    .bind(contract)
    .bind(&mirror_json)
    .bind(&legacy_record_hash)
    .execute(pool)
    .await
    .expect("insert exact deployment semantic mirror");
    let chronology: (
        String,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
    ) = sqlx::query_as(
        "SELECT legacy_record_hash,created_at,updated_at,compared_at,NOW()
           FROM attack_execution_shadow_reads WHERE stage_run_unit_id=$1",
    )
    .bind(org.stage_run_unit_id)
    .fetch_one(pool)
    .await
    .expect("read DB-owned shadow chronology and canonical hash");
    assert_eq!(chronology.0, legacy_record_hash);
    assert_eq!(chronology.1, chronology.2);
    assert_eq!(chronology.1, chronology.3);
    assert!(chronology.1 <= chronology.4);
    assert!(chronology.1 >= chronology.4 - chrono::Duration::minutes(1));
    PendingShadowFixture {
        legacy_record_hash,
        v2_record_hash,
    }
}

async fn attest_shadow_fixture_from_current_v2(
    pool: &PgPool,
    fixture: &AttackFixture,
    org: OrgFixture,
    contract: &str,
    comparison: &str,
) {
    seed_pending_shadow_fixture_from_current_v2(pool, fixture, org, contract, comparison).await;
    let source = match contract {
        "dual_write_read_legacy" => "legacy",
        "dual_write_read_v2_fallback" => "v2",
        _ => panic!("unsupported test contract"),
    };
    let mut selection_tx = pool.begin().await.expect("begin fixture selection");
    attack_execution_shadow::record_unit_selection_with_connection(
        &mut selection_tx,
        fixture.operation_id,
        org.stage_run_unit_id,
        comparison,
        source,
    )
    .await
    .expect("persist DB-verified shadow fixture selection");
    selection_tx
        .commit()
        .await
        .expect("commit fixture selection");
}

#[tokio::test]
#[serial]
async fn fallback_shadow_samples_gate_v2only_promotion_and_retain_frozen_contract() {
    let (mut db, _data_dir) = migrated_db("fallback_shadow_promotion").await;
    let fixture = seed_attack_fixture_with_candidate_pass_and_contract(
        db.pool(),
        true,
        "dual_write_read_v2_fallback",
    )
    .await;
    attest_shadow_fixture_from_current_v2(
        db.pool(),
        &fixture,
        fixture.org_a,
        "dual_write_read_v2_fallback",
        "match",
    )
    .await;

    let mut promote_tx = db.pool().begin().await.expect("begin v2only promotion");
    let cohort_error = attack_execution_rollout::promote_attack_execution_rollout(
        &mut promote_tx,
        2,
        AttackExecutionContract::V2Only,
    )
    .await
    .expect_err("an open fallback Candidate Wave cannot promote to v2-only");
    assert!(
        cohort_error
            .to_string()
            .contains("attack_rollout_cohort_not_ready"),
        "unexpected open fallback cohort error: {cohort_error}"
    );
    promote_tx
        .rollback()
        .await
        .expect("rollback open fallback cohort promotion");
    let sample_contract: String = sqlx::query_scalar(
        "SELECT attack_execution_contract FROM attack_execution_shadow_reads
          WHERE operation_id=$1 AND stage_run_unit_id=$2",
    )
    .bind(fixture.operation_id)
    .bind(fixture.org_a.stage_run_unit_id)
    .fetch_one(db.pool())
    .await
    .expect("read frozen fallback sample after promotion");
    assert_eq!(sample_contract, "dual_write_read_v2_fallback");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn raw_shadow_attestation_cannot_claim_server_owned_selection_authority() {
    let (mut db, _data_dir) = migrated_db("raw_shadow_attestation").await;
    let fixture = seed_attack_fixture_with_candidate_pass_and_contract(
        db.pool(),
        true,
        "dual_write_read_v2_fallback",
    )
    .await;
    let hashes = seed_pending_shadow_fixture_from_current_v2(
        db.pool(),
        &fixture,
        fixture.org_a,
        "dual_write_read_v2_fallback",
        "match",
    )
    .await;
    assert_eq!(hashes.legacy_record_hash, hashes.v2_record_hash);

    let mut wrong_source_tx = db
        .pool()
        .begin()
        .await
        .expect("begin fallback wrong source");
    let wrong_source = sqlx::query(
        r#"UPDATE attack_execution_shadow_reads
              SET comparison='v2_missing',selected_source='v2',
                  selected_record_hash=$3,compared_at=NOW(),updated_at=NOW()
            WHERE operation_id=$1 AND stage_run_unit_id=$2"#,
    )
    .bind(fixture.operation_id)
    .bind(fixture.org_a.stage_run_unit_id)
    .bind(&hashes.v2_record_hash)
    .execute(&mut *wrong_source_tx)
    .await;
    wrong_source_tx
        .rollback()
        .await
        .expect("rollback fallback wrong source");
    assert_sqlstate(
        wrong_source.map(|_| ()),
        "23514",
        "attest fallback V2-missing sample with a V2 source through raw SQL",
    );

    let mut wrong_hash_tx = db.pool().begin().await.expect("begin fallback wrong hash");
    let wrong_hash = sqlx::query(
        r#"UPDATE attack_execution_shadow_reads
              SET comparison='v2_missing',selected_source='legacy_fallback',
                  selected_record_hash=$3,compared_at=NOW(),updated_at=NOW()
            WHERE operation_id=$1 AND stage_run_unit_id=$2"#,
    )
    .bind(fixture.operation_id)
    .bind(fixture.org_a.stage_run_unit_id)
    .bind("0".repeat(64))
    .execute(&mut *wrong_hash_tx)
    .await;
    wrong_hash_tx
        .rollback()
        .await
        .expect("rollback fallback wrong hash");
    assert_sqlstate(
        wrong_hash.map(|_| ()),
        "23514",
        "attest fallback legacy record with a non-legacy hash through raw SQL",
    );

    let forged_v2_hash = "0".repeat(64);
    assert_ne!(forged_v2_hash, hashes.v2_record_hash);
    let forged_close = sqlx::query(
        r#"UPDATE attack_execution_shadow_reads
              SET comparison='match',selected_source='v2',
                  selected_record_hash=$3,compared_at=NOW(),updated_at=NOW()
            WHERE operation_id=$1 AND stage_run_unit_id=$2"#,
    )
    .bind(fixture.operation_id)
    .bind(fixture.org_a.stage_run_unit_id)
    .bind(&forged_v2_hash)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        forged_close.map(|_| ()),
        "23514",
        "claim the one-shot shadow selection authority through raw SQL",
    );

    let mut selection_tx = db
        .pool()
        .begin()
        .await
        .expect("begin exact server-owned fallback selection");
    attack_execution_shadow::record_unit_selection_with_connection(
        &mut selection_tx,
        fixture.operation_id,
        fixture.org_a.stage_run_unit_id,
        "match",
        "v2",
    )
    .await
    .expect("exact repository selector remains available after rejected raw close");
    selection_tx
        .commit()
        .await
        .expect("commit exact server-owned fallback selection");

    let mut promote_tx = db.pool().begin().await.expect("begin forged promotion");
    let error = attack_execution_rollout::promote_attack_execution_rollout(
        &mut promote_tx,
        2,
        AttackExecutionContract::V2Only,
    )
    .await
    .expect_err("an open Candidate cohort remains ineligible for promotion");
    assert!(
        error
            .to_string()
            .contains("attack_rollout_cohort_not_ready"),
        "unexpected open-cohort promotion error: {error}"
    );
    promote_tx
        .rollback()
        .await
        .expect("rollback rejected forged promotion");
    let rollout: (String, i16, i64) = sqlx::query_as(
        "SELECT contract,rank,row_version FROM attack_execution_rollout WHERE singleton=TRUE",
    )
    .fetch_one(db.pool())
    .await
    .expect("load rollout after forged promotion");
    assert_eq!(rollout, ("dual_write_read_v2_fallback".to_string(), 2, 2));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn fallback_shadow_seals_whole_legacy_record_when_v2_child_is_missing() {
    let (mut db, _data_dir) = migrated_db("shadow_real_legacy_fallback").await;
    let fixture = seed_attack_fixture_with_candidate_pass_and_contract(
        db.pool(),
        true,
        "dual_write_read_v2_fallback",
    )
    .await;
    let candidate = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    freeze_shadow_fixture_manifest(db.pool(), &fixture, fixture.org_a).await;
    let legacy_record = legacy_record_for_seeded_candidate(db.pool(), candidate).await;
    let legacy_hash = sha256_json(&legacy_record);

    sqlx::query(
        r#"INSERT INTO attack_execution_shadow_reads(
               stage_run_unit_id,operation_id,stage_execution_id,organization_id,
               attack_execution_contract,legacy_record,legacy_record_hash
           ) VALUES($1,$2,$3,$4,'dual_write_read_v2_fallback',$5,$6)"#,
    )
    .bind(fixture.org_a.stage_run_unit_id)
    .bind(fixture.operation_id)
    .bind(fixture.stage_execution_id)
    .bind(fixture.org_a.organization_id)
    .bind(&legacy_record)
    .bind(&legacy_hash)
    .execute(db.pool())
    .await
    .expect("DB owner seals a whole-record legacy fallback");

    let sealed: (String, String, String) = sqlx::query_as(
        "SELECT comparison,selected_source,selected_record_hash
           FROM attack_execution_shadow_reads WHERE stage_run_unit_id=$1",
    )
    .bind(fixture.org_a.stage_run_unit_id)
    .fetch_one(db.pool())
    .await
    .expect("load DB-owned fallback seal");
    assert_eq!(
        sealed,
        (
            "v2_missing".to_string(),
            "legacy_fallback".to_string(),
            legacy_hash
        )
    );
    let sample = attack_execution_shadow::load_unit_sample(
        db.pool(),
        fixture.operation_id,
        fixture.org_a.stage_run_unit_id,
    )
    .await
    .expect("load fallback sample")
    .expect("fallback sample exists");
    assert!(matches!(
        sample.v2_record,
        attack_execution_shadow::AttackShadowV2ReadRow::Incomplete
    ));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn sealed_shadow_freezes_handoff_and_exact_evidence_semantics() {
    let (mut db, _data_dir) = migrated_db("shadow_frozen_authority").await;
    let fixture = seed_attack_fixture_with_candidate_pass_and_contract(
        db.pool(),
        true,
        "dual_write_read_legacy",
    )
    .await;
    seed_pending_shadow_fixture_from_current_v2(
        db.pool(),
        &fixture,
        fixture.org_a,
        "dual_write_read_legacy",
        "match",
    )
    .await;
    let invalidation = sqlx::query(
        "UPDATE stage_handoffs SET invalidated_at=NOW()
          WHERE source_stage_run_unit_id=$1",
    )
    .bind(fixture.org_a.stage_run_unit_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        invalidation.map(|_| ()),
        "23514",
        "invalidate a sealed Candidate final handoff",
    );
    let evidence_id: i64 =
        sqlx::query_scalar("SELECT evidence_id FROM attack_candidate_evidence LIMIT 1")
            .fetch_one(db.pool())
            .await
            .expect("load sealed Candidate evidence");
    let semantic_rewrite =
        sqlx::query("UPDATE audit_log SET action=action || '-drift' WHERE id=$1")
            .bind(evidence_id)
            .execute(db.pool())
            .await;
    assert_sqlstate(
        semantic_rewrite.map(|_| ()),
        "23514",
        "rewrite sealed Candidate evidence semantics",
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn targetless_evidence_cannot_seal_a_target_bound_shadow() {
    let (mut db, _data_dir) = migrated_db("shadow_targetless_evidence").await;
    let fixture = seed_attack_fixture_with_candidate_pass_and_contract(
        db.pool(),
        true,
        "dual_write_read_legacy",
    )
    .await;
    let targetless_evidence_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO audit_log(
               action,category,details,project_path,audit_role,run_id,target_id,detail
           ) VALUES('targetless','attack','','/tmp/attack-v2','evidence',$1,NULL,$2)
           RETURNING id"#,
    )
    .bind(fixture.operation_id)
    .bind(serde_json::json!({"organization_id": fixture.org_a.organization_id}))
    .fetch_one(db.pool())
    .await
    .expect("insert targetless evidence fixture");
    let candidate = seed_candidate_with_support(
        db.pool(),
        &fixture,
        fixture.org_a,
        &[targetless_evidence_id],
    )
    .await;
    freeze_shadow_fixture_manifest(db.pool(), &fixture, fixture.org_a).await;
    let legacy_record = legacy_record_for_seeded_candidate(db.pool(), candidate).await;
    let legacy_hash = sha256_json(&legacy_record);
    let seal = sqlx::query(
        r#"INSERT INTO attack_execution_shadow_reads(
               stage_run_unit_id,operation_id,stage_execution_id,organization_id,
               attack_execution_contract,legacy_record,legacy_record_hash
           ) VALUES($1,$2,$3,$4,'dual_write_read_legacy',$5,$6)"#,
    )
    .bind(fixture.org_a.stage_run_unit_id)
    .bind(fixture.operation_id)
    .bind(fixture.stage_execution_id)
    .bind(fixture.org_a.organization_id)
    .bind(&legacy_record)
    .bind(&legacy_hash)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        seal.map(|_| ()),
        "23514",
        "seal target-bound Candidate with targetless evidence",
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn durable_shadow_mismatch_blocks_default_promotion() {
    let (mut db, _data_dir) = migrated_db("shadow_mismatch_blocks").await;
    let fixture = seed_attack_fixture_with_candidate_pass_and_contract(
        db.pool(),
        true,
        "dual_write_read_legacy",
    )
    .await;
    attest_shadow_fixture_from_current_v2(
        db.pool(),
        &fixture,
        fixture.org_a,
        "dual_write_read_legacy",
        "mismatch",
    )
    .await;
    reset_attack_rollout_to_legacy_for_transition_fixture(db.pool()).await;
    let mut enable_tx = db.pool().begin().await.expect("begin dual enable");
    let enabled = attack_execution_rollout::promote_attack_execution_rollout(
        &mut enable_tx,
        0,
        AttackExecutionContract::DualWriteReadLegacy,
    )
    .await
    .expect("enable dual sample rank");
    enable_tx.commit().await.expect("commit dual enable");
    let mut promote_tx = db.pool().begin().await.expect("begin mismatch promotion");
    let error = attack_execution_rollout::promote_attack_execution_rollout(
        &mut promote_tx,
        enabled.row_version,
        AttackExecutionContract::DualWriteReadV2Fallback,
    )
    .await
    .expect_err("durable mismatch must block default promotion");
    assert!(
        error
            .to_string()
            .contains("attack_rollout_cohort_not_ready"),
        "unexpected durable-mismatch cohort error: {error}"
    );
    promote_tx
        .commit()
        .await
        .expect("commit rejected mismatch promotion");
    db.stop().await;
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
               'sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963','WSTG-INPV-05',$7,$8
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
               'sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963',$8
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
    let execution_plan: serde_json::Value =
        sqlx::query_scalar("SELECT execution_plan FROM attack_candidates WHERE candidate_id=$1")
            .bind(candidate.candidate_id)
            .fetch_one(pool)
            .await?;
    let budget = execution_plan
        .get("budget")
        .cloned()
        .expect("fixture Candidate budget");
    sqlx::query(
        r#"INSERT INTO attack_candidate_approvals (
               id,candidate_id,operation_id,scope_snapshot_id,wave_run_id,wave_unit_id,
               organization_id,target_live_id,target_type_at_time,target_value_at_time,
               target_identity_hash,candidate_plan_hash,source_work_item_id,execution_plan,
               allowed_capability_ids,allowed_action_kinds,budget,expires_at,
               decision_version,status,decided_by
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,'url','https://shared.example.test/login',
               'sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963','sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5',$9,$10,$11,$12,$13,
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
    .bind(execution_plan)
    .bind(vec!["verify.sql_injection"])
    .bind(vec!["bounded_sql_injection_probe"])
    .bind(budget)
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
    let mut connection = pool.acquire().await?;
    insert_attempt_with_ordinal_on_connection(
        &mut connection,
        fixture,
        candidate,
        approval_id,
        owner,
        status,
        ordinal,
    )
    .await
}

async fn insert_attempt_with_ordinal_on_connection(
    connection: &mut PgConnection,
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
    .execute(&mut *connection)
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
    .execute(&mut *connection)
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
    .execute(&mut *connection)
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
               'https://shared.example.test/login','sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963',
               'sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5',$10,$11,$12,$13,$14,
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
    .execute(&mut *connection)
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
           VALUES($1,$2,$3,$4,$5,'verification',0,'candidate_verifier','queued')"#,
    )
    .bind(stage_run_unit_id)
    .bind(fixture.operation_id)
    .bind(stage_execution_id)
    .bind(fixture.scope_snapshot_id)
    .bind(owner.organization_id)
    .execute(pool)
    .await
    .expect("insert verification StageRunUnit");
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,organization_id,
               worker_generation,specialist,work_item_kind,work_item_key,agent_path,status)
           VALUES($1,$2,$3,$4,$5,0,'candidate_verifier','organization','verification',
                  $6,'queued')"#,
    )
    .bind(Uuid::new_v4())
    .bind(fixture.operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(owner.organization_id)
    .bind(format!(
        "main>stage_run:verification>org:{}>candidate_verifier",
        owner.organization_id
    ))
    .execute(pool)
    .await
    .expect("insert verification logical primary WorkerRun");
    (stage_execution_id, stage_run_unit_id)
}

async fn ensure_fixture_manifest_is_evidenced_for_close(
    pool: &PgPool,
    fixture: &AttackFixture,
    wave_unit_id: Uuid,
) {
    let (organization_id, manifest_frozen, mut work_item_count): (Uuid, bool, i64) =
        sqlx::query_as(
            r#"SELECT wave_unit.organization_id,
                      wave_unit.manifest_frozen_at IS NOT NULL,
                      (SELECT COUNT(*) FROM attack_candidate_work_items AS work
                        WHERE work.wave_unit_id=wave_unit.id
                          AND work.operation_id=wave_unit.operation_id
                          AND work.scope_snapshot_id=wave_unit.scope_snapshot_id
                          AND work.organization_id=wave_unit.organization_id)
                 FROM attack_wave_units AS wave_unit
                WHERE wave_unit.id=$1 AND wave_unit.operation_id=$2
                  AND wave_unit.scope_snapshot_id=$3"#,
        )
        .bind(wave_unit_id)
        .bind(fixture.operation_id)
        .bind(fixture.scope_snapshot_id)
        .fetch_one(pool)
        .await
        .expect("load fixture manifest authority");
    if manifest_frozen {
        return;
    }
    if work_item_count == 0 {
        let (target_id, target_type, target_value): (Uuid, String, String) = sqlx::query_as(
            "SELECT id,target_type::TEXT,value FROM targets
              WHERE organization_id=$1 AND project_path='/tmp/attack-v2'
              ORDER BY id LIMIT 1",
        )
        .bind(organization_id)
        .fetch_one(pool)
        .await
        .expect("load fixture target for evidenced checked-empty manifest");
        let seed_id = Uuid::new_v4();
        let work_item_id = Uuid::new_v4();
        let target_identity_hash = format!("sha256:fixture-target-{target_id}");
        let observation_hash = format!("sha256:fixture-observation-{wave_unit_id}");
        let work_item_key = format!("fixture:checked-empty:{wave_unit_id}");
        let evidence_id = insert_audit(
            pool,
            fixture.operation_id,
            organization_id,
            target_id,
            "evidence",
        )
        .await;
        let mut tx = pool
            .begin()
            .await
            .expect("begin fixture checked-empty manifest");
        sqlx::query(
            r#"INSERT INTO attack_candidate_seeds (
                   id,wave_unit_id,operation_id,scope_snapshot_id,organization_id,
                   target_live_id,target_type_at_time,target_value_at_time,
                   target_identity_hash,technique,observation,observation_hash
               ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,'GOLISH-FIXTURE-CHECKED-EMPTY',$10,$11)"#,
        )
        .bind(seed_id)
        .bind(wave_unit_id)
        .bind(fixture.operation_id)
        .bind(fixture.scope_snapshot_id)
        .bind(organization_id)
        .bind(target_id)
        .bind(&target_type)
        .bind(&target_value)
        .bind(&target_identity_hash)
        .bind(serde_json::json!({"fixture": "evidenced_checked_empty"}))
        .bind(&observation_hash)
        .execute(&mut *tx)
        .await
        .expect("insert fixture checked-empty seed");
        sqlx::query(
            r#"INSERT INTO attack_candidate_work_items (
                   id,seed_id,wave_unit_id,operation_id,scope_snapshot_id,organization_id,
                   target_live_id,target_type_at_time,target_value_at_time,
                   target_identity_hash,work_item_key
               ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
        )
        .bind(work_item_id)
        .bind(seed_id)
        .bind(wave_unit_id)
        .bind(fixture.operation_id)
        .bind(fixture.scope_snapshot_id)
        .bind(organization_id)
        .bind(target_id)
        .bind(&target_type)
        .bind(&target_value)
        .bind(&target_identity_hash)
        .bind(&work_item_key)
        .execute(&mut *tx)
        .await
        .expect("insert fixture checked-empty work item");
        sqlx::query(
            "INSERT INTO attack_candidate_work_item_evidence(work_item_id,evidence_id,role)
             VALUES($1,$2,'decision')",
        )
        .bind(work_item_id)
        .bind(evidence_id)
        .execute(&mut *tx)
        .await
        .expect("link fixture checked-empty evidence");
        sqlx::query(
            "UPDATE attack_candidate_work_items
                SET decision_kind='no_candidate',no_candidate_reason_code='checked_empty',
                    no_candidate_detail='fixture evidence proves this manifest item checked empty',
                    decided_at=NOW()
              WHERE id=$1",
        )
        .bind(work_item_id)
        .execute(&mut *tx)
        .await
        .expect("terminalize fixture checked-empty work item");
        tx.commit()
            .await
            .expect("commit fixture checked-empty manifest");
        work_item_count = 1;
    }
    sqlx::query(
        "UPDATE attack_wave_units
            SET manifest_hash=$2,manifest_count=$3,manifest_frozen_at=NOW(),updated_at=NOW()
          WHERE id=$1 AND manifest_frozen_at IS NULL",
    )
    .bind(wave_unit_id)
    .bind(format!(
        "sha256:fixture-manifest-{wave_unit_id}-{work_item_count}"
    ))
    .bind(i32::try_from(work_item_count).expect("fixture manifest count fits i32"))
    .execute(pool)
    .await
    .expect("freeze exact fixture manifest before Verification close");
}

async fn close_fixture_verification_units_through_typed_handoff(
    pool: &PgPool,
    fixture: &AttackFixture,
    wave_run_id: Uuid,
    wave_unit_ids: &[Uuid],
) {
    let generation: i32 = sqlx::query_scalar(
        "SELECT generation FROM attack_wave_runs
         WHERE id=$1 AND operation_id=$2 AND scope_snapshot_id=$3",
    )
    .bind(wave_run_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .fetch_one(pool)
    .await
    .expect("load fixture Verification generation");
    for wave_unit_id in wave_unit_ids {
        ensure_fixture_manifest_is_evidenced_for_close(pool, fixture, *wave_unit_id).await;
    }
    sqlx::query(
        "UPDATE attack_wave_runs SET status='verification',terminal_at=NULL,updated_at=NOW()
         WHERE id=$1 AND operation_id=$2 AND scope_snapshot_id=$3",
    )
    .bind(wave_run_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .execute(pool)
    .await
    .expect("advance fixture Wave to Verification");
    for wave_unit_id in wave_unit_ids {
        let organization_id: Uuid = sqlx::query_scalar(
            "UPDATE attack_wave_units
             SET status='verification',review_closed=TRUE,updated_at=NOW()
             WHERE id=$1 AND wave_run_id=$2 AND operation_id=$3
               AND scope_snapshot_id=$4 AND terminal_at IS NULL
             RETURNING organization_id",
        )
        .bind(wave_unit_id)
        .bind(wave_run_id)
        .bind(fixture.operation_id)
        .bind(fixture.scope_snapshot_id)
        .fetch_one(pool)
        .await
        .expect("make fixture WaveUnit close-ready");
        let stage_execution_id = Uuid::new_v4();
        let stage_run_unit_id = Uuid::new_v4();
        let primary_worker_run_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO stage_runs(id,operation_id,stage_kind,status)
             VALUES($1,$2,'verification','started')",
        )
        .bind(stage_execution_id)
        .bind(fixture.operation_id)
        .execute(pool)
        .await
        .expect("insert fixture Verification StageRun");
        sqlx::query(
            r#"INSERT INTO stage_run_units(
                   id,operation_id,stage_execution_id,scope_snapshot_id,organization_id,
                   stage_kind,generation,specialist,status)
               VALUES($1,$2,$3,$4,$5,'verification',$6,'candidate_verifier','queued')"#,
        )
        .bind(stage_run_unit_id)
        .bind(fixture.operation_id)
        .bind(stage_execution_id)
        .bind(fixture.scope_snapshot_id)
        .bind(organization_id)
        .bind(generation)
        .execute(pool)
        .await
        .expect("insert fixture Verification StageRunUnit");
        sqlx::query(
            r#"INSERT INTO stage_worker_runs(
                   id,operation_id,stage_execution_id,stage_run_unit_id,organization_id,
                   worker_generation,specialist,work_item_kind,work_item_key,agent_path,status)
               VALUES($1,$2,$3,$4,$5,$6,'candidate_verifier','organization','verification',
                      $7,'queued')"#,
        )
        .bind(primary_worker_run_id)
        .bind(fixture.operation_id)
        .bind(stage_execution_id)
        .bind(stage_run_unit_id)
        .bind(organization_id)
        .bind(generation)
        .bind(format!(
            "main>fixture:verification>org:{organization_id}>generation:{generation}"
        ))
        .execute(pool)
        .await
        .expect("insert fixture aggregate Verification WorkerRun");
        let mut tx = pool
            .begin()
            .await
            .expect("begin fixture Verification close");
        golish_db::repo::verification_truth::close_verification_unit(
            &mut tx,
            golish_db::repo::verification_truth::CloseVerificationUnit {
                operation_id: fixture.operation_id,
                scope_snapshot_id: fixture.scope_snapshot_id,
                wave_run_id,
                wave_unit_id: *wave_unit_id,
                organization_id,
                verification_stage_execution_id: stage_execution_id,
                verification_stage_run_unit_id: stage_run_unit_id,
            },
        )
        .await
        .expect("close fixture Verification through server-authored typed handoff");
        tx.commit()
            .await
            .expect("commit fixture Verification typed handoff");
    }
}

async fn close_all_active_fixture_verification_units(
    pool: &PgPool,
    fixture: &AttackFixture,
    wave_run_id: Uuid,
) {
    let wave_unit_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM attack_wave_units
         WHERE wave_run_id=$1 AND operation_id=$2 AND scope_snapshot_id=$3
           AND terminal_at IS NULL
         ORDER BY ordinal,organization_id",
    )
    .bind(wave_run_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .fetch_all(pool)
    .await
    .expect("load all active fixture Verification WaveUnits");
    close_fixture_verification_units_through_typed_handoff(
        pool,
        fixture,
        wave_run_id,
        &wave_unit_ids,
    )
    .await;
}

#[tokio::test]
#[serial]
async fn verification_pass_without_typed_handoff_is_rejected_at_commit() {
    let (mut db, _data_dir) = migrated_db("verification_pass_requires_typed_handoff").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let (stage_execution_id, stage_run_unit_id) =
        seed_verification_unit(db.pool(), &fixture, fixture.org_a).await;
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin partial Verification close");
    sqlx::query(
        "UPDATE attack_wave_units
         SET status='verification',review_closed=TRUE,verification_closed=TRUE,
             consolidation_status='ready',row_version=row_version+1
         WHERE id=$1",
    )
    .bind(fixture.org_a.wave_unit_id)
    .execute(&mut *tx)
    .await
    .expect("partially close WaveUnit without typed handoff");
    sqlx::query(
        "UPDATE stage_worker_runs SET status='passed',terminal_at=NOW()
         WHERE stage_run_unit_id=$1 AND work_item_kind='organization'
           AND work_item_key='verification'",
    )
    .bind(stage_run_unit_id)
    .execute(&mut *tx)
    .await
    .expect("partially close aggregate Verification WorkerRun");
    sqlx::query(
        "UPDATE stage_run_units SET status='passed',terminal_at=NOW()
         WHERE id=$1 AND stage_execution_id=$2",
    )
    .bind(stage_run_unit_id)
    .bind(stage_execution_id)
    .execute(&mut *tx)
    .await
    .expect("partially close Verification StageRunUnit");
    let error = tx
        .commit()
        .await
        .expect_err("passed Verification Unit without typed handoff must fail at commit");
    assert!(
        error
            .to_string()
            .contains("VERIFICATION_TYPED_HANDOFF_REQUIRED"),
        "unexpected missing typed handoff error: {error}"
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn verification_close_replay_rejects_missing_typed_handoff() {
    let (mut db, _data_dir) = migrated_db("verification_replay_requires_typed_handoff").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let (stage_execution_id, stage_run_unit_id) =
        seed_verification_unit(db.pool(), &fixture, fixture.org_a).await;
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin partial Verification replay");
    sqlx::query(
        "UPDATE attack_wave_units
         SET status='verification',review_closed=TRUE,verification_closed=TRUE,
             consolidation_status='ready',row_version=row_version+1
         WHERE id=$1",
    )
    .bind(fixture.org_a.wave_unit_id)
    .execute(&mut *tx)
    .await
    .expect("partially close WaveUnit before response loss");
    sqlx::query(
        "UPDATE stage_worker_runs SET status='passed',terminal_at=NOW()
         WHERE stage_run_unit_id=$1 AND work_item_kind='organization'
           AND work_item_key='verification'",
    )
    .bind(stage_run_unit_id)
    .execute(&mut *tx)
    .await
    .expect("partially close aggregate Worker before response loss");
    sqlx::query(
        "UPDATE stage_run_units SET status='passed',terminal_at=NOW()
         WHERE id=$1 AND stage_execution_id=$2",
    )
    .bind(stage_run_unit_id)
    .bind(stage_execution_id)
    .execute(&mut *tx)
    .await
    .expect("partially close StageRunUnit before response loss");
    let error = golish_db::repo::verification_truth::close_verification_unit(
        &mut tx,
        golish_db::repo::verification_truth::CloseVerificationUnit {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_a.wave_unit_id,
            organization_id: fixture.org_a.organization_id,
            verification_stage_execution_id: stage_execution_id,
            verification_stage_run_unit_id: stage_run_unit_id,
        },
    )
    .await
    .expect_err("response-loss replay without typed handoff must fail closed");
    assert!(
        error
            .to_string()
            .contains("Verification typed handoff is missing"),
        "unexpected partial replay error: {error}"
    );
    tx.rollback()
        .await
        .expect("rollback partial Verification replay");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn verification_typed_handoff_rejects_raw_hash_and_evidence_drift() {
    let (mut db, _data_dir) = migrated_db("verification_handoff_raw_drift").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let (stage_execution_id, stage_run_unit_id) =
        seed_verification_unit(db.pool(), &fixture, fixture.org_a).await;
    let primary_worker_run_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM stage_worker_runs
         WHERE stage_run_unit_id=$1 AND work_item_kind='organization'
           AND work_item_key='verification'",
    )
    .bind(stage_run_unit_id)
    .fetch_one(db.pool())
    .await
    .expect("load aggregate Verification WorkerRun");

    let mut malformed_hash_tx = db
        .pool()
        .begin()
        .await
        .expect("begin malformed hash insert");
    let row_version: i64 = sqlx::query_scalar(
        "UPDATE attack_wave_units
         SET status='verification',review_closed=TRUE,verification_closed=TRUE,
             consolidation_status='ready',row_version=row_version+1
         WHERE id=$1 RETURNING row_version",
    )
    .bind(fixture.org_a.wave_unit_id)
    .fetch_one(&mut *malformed_hash_tx)
    .await
    .expect("close WaveUnit in hostile hash transaction");
    sqlx::query("UPDATE stage_worker_runs SET status='passed',terminal_at=NOW() WHERE id=$1")
        .bind(primary_worker_run_id)
        .execute(&mut *malformed_hash_tx)
        .await
        .expect("close WorkerRun in hostile hash transaction");
    sqlx::query("UPDATE stage_run_units SET status='passed',terminal_at=NOW() WHERE id=$1")
        .bind(stage_run_unit_id)
        .execute(&mut *malformed_hash_tx)
        .await
        .expect("close StageRunUnit in hostile hash transaction");
    let empty_payload = serde_json::json!({
        "schema_version": 1,
        "canonical_fact_refs": [],
        "typed_claims": [],
        "coverage_watermark": {},
        "evidence_ids": [],
        "verification_truth_hash": "b".repeat(64),
    });
    let malformed_hash = sqlx::query(
        r#"INSERT INTO verification_stage_handoffs(
               id,operation_id,scope_snapshot_id,wave_run_id,wave_unit_id,
               organization_id,stage_execution_id,source_stage_run_unit_id,
               primary_worker_run_id,wave_generation,
               wave_unit_row_version_after_close,payload,payload_sha256,
               evidence_ids,coverage_watermark,verification_truth_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,0,$10,$11,'hash-drift',
                    '{}'::BIGINT[],'{}'::JSONB,$12)"#,
    )
    .bind(Uuid::new_v5(
        &fixture.org_a.wave_unit_id,
        b"verification-stage-handoff:v1",
    ))
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.wave_run_id)
    .bind(fixture.org_a.wave_unit_id)
    .bind(fixture.org_a.organization_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(primary_worker_run_id)
    .bind(row_version)
    .bind(&empty_payload)
    .bind("b".repeat(64))
    .execute(&mut *malformed_hash_tx)
    .await
    .expect_err("raw malformed payload hash must be rejected");
    assert!(
        malformed_hash
            .to_string()
            .contains("VERIFICATION_TYPED_HANDOFF_HASH_MISMATCH"),
        "unexpected malformed hash error: {malformed_hash}"
    );
    malformed_hash_tx
        .rollback()
        .await
        .expect("rollback malformed hash transaction");

    let foreign_evidence_id = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_b.organization_id,
        fixture.org_b.target_id,
        "evidence",
    )
    .await;
    let mut foreign_evidence_tx = db
        .pool()
        .begin()
        .await
        .expect("begin foreign evidence insert");
    let row_version: i64 = sqlx::query_scalar(
        "UPDATE attack_wave_units
         SET status='verification',review_closed=TRUE,verification_closed=TRUE,
             consolidation_status='ready',row_version=row_version+1
         WHERE id=$1 RETURNING row_version",
    )
    .bind(fixture.org_a.wave_unit_id)
    .fetch_one(&mut *foreign_evidence_tx)
    .await
    .expect("close WaveUnit in hostile evidence transaction");
    sqlx::query("UPDATE stage_worker_runs SET status='passed',terminal_at=NOW() WHERE id=$1")
        .bind(primary_worker_run_id)
        .execute(&mut *foreign_evidence_tx)
        .await
        .expect("close WorkerRun in hostile evidence transaction");
    sqlx::query("UPDATE stage_run_units SET status='passed',terminal_at=NOW() WHERE id=$1")
        .bind(stage_run_unit_id)
        .execute(&mut *foreign_evidence_tx)
        .await
        .expect("close StageRunUnit in hostile evidence transaction");
    let mut foreign_payload = serde_json::json!({
        "schema_version": 1,
        "canonical_fact_refs": [],
        "typed_claims": [],
        "coverage_watermark": {},
        "evidence_ids": [foreign_evidence_id],
        "verification_truth_hash": "",
    });
    let foreign_truth_hash = sha256_json(&serde_json::json!({
        "schema_version": 1,
        "operation_id": fixture.operation_id,
        "scope_snapshot_id": fixture.scope_snapshot_id,
        "wave_run_id": fixture.wave_run_id,
        "wave_unit_id": fixture.org_a.wave_unit_id,
        "organization_id": fixture.org_a.organization_id,
        "canonical_fact_refs": foreign_payload["canonical_fact_refs"],
        "typed_claims": foreign_payload["typed_claims"],
        "coverage_watermark": foreign_payload["coverage_watermark"],
        "evidence_ids": foreign_payload["evidence_ids"],
    }));
    foreign_payload["verification_truth_hash"] = serde_json::json!(foreign_truth_hash);
    let foreign_payload_sha = sha256_json(&foreign_payload);
    let foreign_evidence = sqlx::query(
        r#"INSERT INTO verification_stage_handoffs(
               id,operation_id,scope_snapshot_id,wave_run_id,wave_unit_id,
               organization_id,stage_execution_id,source_stage_run_unit_id,
               primary_worker_run_id,wave_generation,
               wave_unit_row_version_after_close,payload,payload_sha256,
               evidence_ids,coverage_watermark,verification_truth_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,0,$10,$11,$12,$13,
                    '{}'::JSONB,$14)"#,
    )
    .bind(Uuid::new_v5(
        &fixture.org_a.wave_unit_id,
        b"verification-stage-handoff:v1",
    ))
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.wave_run_id)
    .bind(fixture.org_a.wave_unit_id)
    .bind(fixture.org_a.organization_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(primary_worker_run_id)
    .bind(row_version)
    .bind(&foreign_payload)
    .bind(foreign_payload_sha)
    .bind(vec![foreign_evidence_id])
    .bind(foreign_truth_hash)
    .execute(&mut *foreign_evidence_tx)
    .await
    .expect_err("foreign unattached raw evidence must be rejected");
    assert!(
        foreign_evidence
            .to_string()
            .contains("VERIFICATION_TYPED_HANDOFF_EVIDENCE_MISMATCH"),
        "unexpected foreign evidence error: {foreign_evidence}"
    );
    foreign_evidence_tx
        .rollback()
        .await
        .expect("rollback foreign evidence transaction");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn verification_raw_handoff_rejects_unfrozen_empty_manifest() {
    let (mut db, _data_dir) = migrated_db("verification_unfrozen_manifest_forgery").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let (stage_execution_id, stage_run_unit_id) =
        seed_verification_unit(db.pool(), &fixture, fixture.org_a).await;
    sqlx::query(
        "UPDATE attack_wave_units SET status='verification',review_closed=TRUE,updated_at=NOW()
         WHERE id=$1",
    )
    .bind(fixture.org_a.wave_unit_id)
    .execute(db.pool())
    .await
    .expect("stage hostile unfrozen Verification WaveUnit");
    let primary_worker_run_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM stage_worker_runs
         WHERE stage_run_unit_id=$1 AND work_item_kind='organization'
           AND work_item_key='verification'",
    )
    .bind(stage_run_unit_id)
    .fetch_one(db.pool())
    .await
    .expect("load hostile aggregate Verification WorkerRun");
    let coverage_watermark = serde_json::json!({
        "approved_candidate_count": 0,
        "terminal_attempt_count": 0,
        "verified_finding_count": 0,
        "no_candidate_decision_count": 0,
        "fact_delta_proposal_count": 0,
    });
    let mut handoff = golish_db::repo::stage_handoffs::VerificationStageHandoffRow {
        id: Uuid::new_v5(
            &fixture.org_a.wave_unit_id,
            b"verification-stage-handoff:v1",
        ),
        operation_id: fixture.operation_id,
        scope_snapshot_id: fixture.scope_snapshot_id,
        wave_run_id: fixture.wave_run_id,
        wave_unit_id: fixture.org_a.wave_unit_id,
        organization_id: fixture.org_a.organization_id,
        stage_execution_id,
        source_stage_run_unit_id: stage_run_unit_id,
        primary_worker_run_id,
        wave_generation: 0,
        wave_unit_row_version_after_close: 1,
        from_stage_kind: "verification".to_string(),
        authority_kind: "verification_wave_close".to_string(),
        payload: serde_json::Value::Null,
        payload_sha256: String::new(),
        evidence_ids: Vec::new(),
        coverage_watermark: coverage_watermark.clone(),
        verification_truth_hash: String::new(),
        gate_passed_at: chrono::Utc::now(),
        schema_version: 1,
    };
    let mut payload = serde_json::json!({
        "schema_version": 1,
        "canonical_fact_refs": [],
        "typed_claims": [],
        "coverage_watermark": coverage_watermark,
        "evidence_ids": [],
        "verification_truth_hash": "",
    });
    let payload_sha256 = refresh_verification_handoff_hashes(&handoff, &mut payload);
    handoff.verification_truth_hash = payload["verification_truth_hash"]
        .as_str()
        .expect("hostile Verification truth hash")
        .to_string();
    handoff.payload_sha256 = payload_sha256.clone();
    handoff.payload = payload.clone();
    let result =
        try_raw_verification_handoff_insert(db.pool(), &handoff, &payload, &payload_sha256).await;
    assert!(
        result.as_ref().is_err_and(|error| error
            .to_string()
            .contains("VERIFICATION_TYPED_HANDOFF_PROJECTION_MISMATCH")),
        "an unfrozen zero-row manifest cannot be forged into checked-empty truth: {result:?}"
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn verification_rejects_raw_direct_terminal_attempt_without_receipt_bundle() {
    let (mut db, _data_dir) = migrated_db("verification_raw_direct_terminal").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let candidate = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    ensure_fixture_manifest_is_evidenced_for_close(db.pool(), &fixture, fixture.org_a.wave_unit_id)
        .await;
    let approval_id = insert_approval(db.pool(), &fixture, candidate, fixture.org_a)
        .await
        .expect("insert raw-terminal approval fixture");
    sqlx::query("UPDATE attack_candidates SET disposition='approved' WHERE candidate_id=$1")
        .bind(candidate.candidate_id)
        .execute(db.pool())
        .await
        .expect("advance hostile raw-terminal Candidate through approved state");
    let attempt_id = insert_attempt(
        db.pool(),
        &fixture,
        candidate,
        approval_id,
        fixture.org_a,
        "running",
    )
    .await
    .expect("insert running Attempt before hostile direct terminal update");
    let result_json = serde_json::json!({
        "blocker_evidence_ids": [],
        "blocker_reason_code": "raw_direct_terminal",
        "disposition": "blocked",
        "proof_evidence_ids": [],
        "refutation_evidence_ids": [],
    });
    let result_hash = format!("sha256:{}", sha256_json(&result_json));
    sqlx::query(
        "UPDATE candidate_attempts
            SET status='blocked',result_json=$2,result_hash=$3,terminal_at=NOW(),updated_at=NOW()
          WHERE id=$1 AND status='running'",
    )
    .bind(attempt_id)
    .bind(&result_json)
    .bind(&result_hash)
    .execute(db.pool())
    .await
    .expect("hostile raw SQL directly terminalizes running Attempt");
    sqlx::query(
        "UPDATE attack_candidates
            SET disposition='blocked',terminal_attempt_id=$2,terminal_finding_id=NULL,
                updated_at=NOW()
          WHERE candidate_id=$1",
    )
    .bind(candidate.candidate_id)
    .bind(attempt_id)
    .execute(db.pool())
    .await
    .expect("hostile raw SQL binds forged terminal Candidate");
    sqlx::query(
        "UPDATE attack_wave_units SET status='verification',review_closed=TRUE,updated_at=NOW()
         WHERE id=$1",
    )
    .bind(fixture.org_a.wave_unit_id)
    .execute(db.pool())
    .await
    .expect("stage raw-terminal WaveUnit for Verification close");
    let (stage_execution_id, stage_run_unit_id) =
        seed_verification_unit(db.pool(), &fixture, fixture.org_a).await;
    let mut close_tx = db
        .pool()
        .begin()
        .await
        .expect("begin raw-terminal Verification close");
    let error = golish_db::repo::verification_truth::close_verification_unit(
        &mut close_tx,
        golish_db::repo::verification_truth::CloseVerificationUnit {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_a.wave_unit_id,
            organization_id: fixture.org_a.organization_id,
            verification_stage_execution_id: stage_execution_id,
            verification_stage_run_unit_id: stage_run_unit_id,
        },
    )
    .await
    .expect_err("raw direct terminal state without exact receipt bundle must fail closed");
    assert!(
        error.to_string().contains("pending or invalid terminal"),
        "unexpected raw direct-terminal close error: {error}"
    );
    close_tx
        .rollback()
        .await
        .expect("rollback raw-terminal Verification close");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn verification_zero_approved_handoff_retains_evidenced_checked_empty_truth() {
    let (mut db, _data_dir) = migrated_db("verification_checked_empty_handoff").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let work_item =
        seed_pending_work_item(db.pool(), &fixture, fixture.org_a, "checked-empty").await;
    ensure_fixture_manifest_is_evidenced_for_close(db.pool(), &fixture, fixture.org_a.wave_unit_id)
        .await;
    sqlx::query(
        "UPDATE attack_wave_units SET status='verification',review_closed=TRUE WHERE id=$1",
    )
    .bind(fixture.org_a.wave_unit_id)
    .execute(db.pool())
    .await
    .expect("make checked-empty WaveUnit verification-ready");
    let (stage_execution_id, stage_run_unit_id) =
        seed_verification_unit(db.pool(), &fixture, fixture.org_a).await;
    let close = golish_db::repo::verification_truth::CloseVerificationUnit {
        operation_id: fixture.operation_id,
        scope_snapshot_id: fixture.scope_snapshot_id,
        wave_run_id: fixture.wave_run_id,
        wave_unit_id: fixture.org_a.wave_unit_id,
        organization_id: fixture.org_a.organization_id,
        verification_stage_execution_id: stage_execution_id,
        verification_stage_run_unit_id: stage_run_unit_id,
    };

    let mut unchecked_tx = db
        .pool()
        .begin()
        .await
        .expect("begin unchecked-empty close");
    let unchecked_error =
        golish_db::repo::verification_truth::close_verification_unit(&mut unchecked_tx, close)
            .await
            .expect_err("an unchecked work item must not become checked-empty truth");
    assert!(
        unchecked_error.to_string().contains("pending or invalid"),
        "unexpected unchecked-empty error: {unchecked_error}"
    );
    unchecked_tx
        .rollback()
        .await
        .expect("rollback unchecked-empty close");

    let decision_evidence_id = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_a.organization_id,
        fixture.org_a.target_id,
        "evidence",
    )
    .await;
    let mut decision_tx = db
        .pool()
        .begin()
        .await
        .expect("begin checked-empty decision");
    sqlx::query(
        "INSERT INTO attack_candidate_work_item_evidence(work_item_id,evidence_id,role)
         VALUES($1,$2,'decision')",
    )
    .bind(work_item.work_item_id)
    .bind(decision_evidence_id)
    .execute(&mut *decision_tx)
    .await
    .expect("link checked-empty decision evidence");
    sqlx::query(
        "UPDATE attack_candidate_work_items
         SET decision_kind='no_candidate',no_candidate_reason_code='checked_empty',
             no_candidate_detail='evidence-backed check produced no Candidate',decided_at=NOW()
         WHERE id=$1",
    )
    .bind(work_item.work_item_id)
    .execute(&mut *decision_tx)
    .await
    .expect("persist checked-empty terminal decision");
    decision_tx
        .commit()
        .await
        .expect("commit checked-empty terminal decision");

    let mut template_tx = db
        .pool()
        .begin()
        .await
        .expect("begin checked-empty typed-handoff template");
    golish_db::repo::verification_truth::close_verification_unit(&mut template_tx, close)
        .await
        .expect("build checked-empty typed-handoff template");
    let template =
        sqlx::query_as::<_, golish_db::repo::stage_handoffs::VerificationStageHandoffRow>(
            "SELECT * FROM verification_stage_handoffs WHERE source_stage_run_unit_id=$1",
        )
        .bind(stage_run_unit_id)
        .fetch_one(&mut *template_tx)
        .await
        .expect("load checked-empty typed-handoff template");
    template_tx
        .rollback()
        .await
        .expect("rollback checked-empty typed-handoff template");
    let mut forged_no_candidate_payload = template.payload.clone();
    let checked_empty_claim = forged_no_candidate_payload["typed_claims"]
        .as_array_mut()
        .expect("checked-empty template claims")
        .iter_mut()
        .find(|claim| claim["kind"] == "attack_no_candidate_decision")
        .expect("checked-empty template claim");
    checked_empty_claim["payload"]["reason_code"] = serde_json::json!("forged_reason");
    checked_empty_claim["payload"]["detail"] = serde_json::json!("forged checked-empty detail");
    let forged_no_candidate_sha =
        refresh_verification_handoff_hashes(&template, &mut forged_no_candidate_payload);
    let forged_no_candidate = try_raw_verification_handoff_insert(
        db.pool(),
        &template,
        &forged_no_candidate_payload,
        &forged_no_candidate_sha,
    )
    .await;
    assert!(
        forged_no_candidate
            .as_ref()
            .is_err_and(|error| error
                .to_string()
                .contains("VERIFICATION_TYPED_HANDOFF_PROJECTION_MISMATCH")),
        "raw no-candidate reason/detail drift must fail exact DB projection: {forged_no_candidate:?}"
    );

    let mut close_tx = db.pool().begin().await.expect("begin checked-empty close");
    let closed = golish_db::repo::verification_truth::close_verification_unit(&mut close_tx, close)
        .await
        .expect("evidenced checked-empty truth closes Verification");
    close_tx.commit().await.expect("commit checked-empty close");
    let handoff: (serde_json::Value, Vec<i64>, serde_json::Value) = sqlx::query_as(
        "SELECT payload,evidence_ids,coverage_watermark
         FROM verification_stage_handoffs WHERE id=$1",
    )
    .bind(closed.verification_handoff_id)
    .fetch_one(db.pool())
    .await
    .expect("load checked-empty typed handoff");
    assert_eq!(handoff.1, vec![decision_evidence_id]);
    assert_eq!(handoff.2["approved_candidate_count"], 0);
    assert_eq!(handoff.2["terminal_attempt_count"], 0);
    assert_eq!(handoff.2["no_candidate_decision_count"], 1);
    let checked_empty = handoff.0["typed_claims"]
        .as_array()
        .expect("checked-empty typed claims")
        .iter()
        .find(|claim| claim["kind"] == "attack_no_candidate_decision")
        .expect("checked-empty handoff claim");
    assert_eq!(
        checked_empty["payload"]["work_item_id"],
        serde_json::json!(work_item.work_item_id)
    );
    assert_eq!(checked_empty["payload"]["reason_code"], "checked_empty");
    assert_eq!(
        checked_empty["payload"]["evidence_ids"],
        serde_json::json!([decision_evidence_id])
    );
    let evidence_identity_tamper =
        sqlx::query("UPDATE audit_log SET audit_role='action' WHERE id=$1")
            .bind(decision_evidence_id)
            .execute(db.pool())
            .await;
    assert_sqlstate(
        evidence_identity_tamper.map(|_| ()),
        "23514",
        "Verification handoff evidence identity mutation",
    );
    let decision_membership_delete = sqlx::query(
        "DELETE FROM attack_candidate_work_item_evidence
         WHERE work_item_id=$1 AND evidence_id=$2 AND role='decision'",
    )
    .bind(work_item.work_item_id)
    .bind(decision_evidence_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        decision_membership_delete.map(|_| ()),
        "23514",
        "Verification handoff decision evidence membership deletion",
    );
    sqlx::query("DELETE FROM targets WHERE id=$1")
        .bind(fixture.org_a.target_id)
        .execute(db.pool())
        .await
        .expect("true live-target deletion preserves the Verification handoff");
    let retained_after_target_delete: (i64, Option<Uuid>) = sqlx::query_as(
        "SELECT
             (SELECT COUNT(*) FROM verification_stage_handoffs WHERE id=$1),
             (SELECT target_id FROM audit_log WHERE id=$2)",
    )
    .bind(closed.verification_handoff_id)
    .bind(decision_evidence_id)
    .fetch_one(db.pool())
    .await
    .expect("reload retained Verification handoff and nullable live evidence pointer");
    assert_eq!(retained_after_target_delete, (1, None));
    db.stop().await;
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
           WHERE operation_uuid=$1 AND target_identity_hash='sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963'
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
async fn fact_delta_seed_rejects_cross_owner_binding() {
    let (mut db, _data_dir) = migrated_db("fact_delta_seed_exact_owner").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let candidate = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    let approval_id = insert_approval(db.pool(), &fixture, candidate, fixture.org_a)
        .await
        .expect("insert exact Candidate approval");
    let attempt_id = insert_attempt(
        db.pool(),
        &fixture,
        candidate,
        approval_id,
        fixture.org_a,
        "blocked",
    )
    .await
    .expect("insert terminal Candidate Attempt");
    sqlx::query("UPDATE attack_wave_runs SET status='verification',updated_at=NOW() WHERE id=$1")
        .bind(fixture.wave_run_id)
        .execute(db.pool())
        .await
        .expect("advance exact source Wave to verification");
    let fact_delta_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO attack_fact_deltas (
               id,source_attempt_id,candidate_id,operation_id,scope_snapshot_id,
               wave_run_id,wave_unit_id,organization_id,target_live_id,
               target_type_at_time,target_value_at_time,target_identity_hash,
               candidate_plan_hash,canonical_ref_kind,canonical_ref_id,
               canonical_ref_version,canonical_ref_hash,delta_kind,dedupe_hash
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,$9,'url',
               'https://shared.example.test/login',
               'sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963',
               'sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5',
               'attack_candidate_work_item',$10,1,'sha256:canonical-owner-fixture',
               'new_surface','sha256:fact-delta-owner-fixture'
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
    .bind(candidate.work_item_id)
    .execute(db.pool())
    .await
    .expect("insert source FactDelta");
    let observation = serde_json::json!({
        "schema": "nuclei_match_v1",
        "fact_delta_id": fact_delta_id,
        "delta_kind": "new_surface",
        "observation_kind": "nuclei_match_v1",
        "allowed_techniques": ["GOLISH-NDAY"],
        "enrichment_required": false,
    });
    let cross_owner = sqlx::query(
        r#"INSERT INTO attack_candidate_seeds (
               id,wave_unit_id,operation_id,scope_snapshot_id,organization_id,
               target_live_id,target_type_at_time,target_value_at_time,
               target_identity_hash,technique,observation,observation_hash,
               source_fact_delta_id,delta_kind,observation_kind,
               allowed_techniques,enrichment_required
           ) VALUES (
               $1,$2,$3,$4,$5,$6,'url','https://shared.example.test/login',
               'sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963',
               'GOLISH-NDAY',$7,'sha256:cross-owner-observation',$8,
               'new_surface','nuclei_match_v1',ARRAY['GOLISH-NDAY'],FALSE
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(fixture.org_b.wave_unit_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.org_b.organization_id)
    .bind(fixture.org_b.target_id)
    .bind(&observation)
    .bind(fact_delta_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        cross_owner.map(|_| ()),
        "23503",
        "cross-owner FactDelta-backed Candidate seed",
    );

    sqlx::query(
        r#"INSERT INTO attack_candidate_seeds (
               id,wave_unit_id,operation_id,scope_snapshot_id,organization_id,
               target_live_id,target_type_at_time,target_value_at_time,
               target_identity_hash,technique,observation,observation_hash,
               source_fact_delta_id,delta_kind,observation_kind,
               allowed_techniques,enrichment_required
           ) VALUES (
               $1,$2,$3,$4,$5,$6,'url','https://shared.example.test/login',
               'sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963',
               'GOLISH-NDAY',$7,'sha256:exact-owner-observation',$8,
               'new_surface','nuclei_match_v1',ARRAY['GOLISH-NDAY'],FALSE
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(fixture.org_a.wave_unit_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.org_a.organization_id)
    .bind(fixture.org_a.target_id)
    .bind(observation)
    .bind(fact_delta_id)
    .execute(db.pool())
    .await
    .expect("exact-owner FactDelta-backed Candidate seed must be accepted");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn deleting_live_org_and_target_retains_attack_audit_rows_and_nulls_live_target_ref() {
    let (mut db, _data_dir) = migrated_db("retention").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let evidence_id = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_a.organization_id,
        fixture.org_a.target_id,
        "evidence",
    )
    .await;
    let candidate =
        seed_candidate_with_support(db.pool(), &fixture, fixture.org_a, &[evidence_id]).await;
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
    sqlx::query("UPDATE attack_candidates SET disposition='approved' WHERE candidate_id=$1")
        .bind(candidate.candidate_id)
        .execute(db.pool())
        .await
        .expect("mark retained Candidate approved before its Attempt");
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
    let delta_evidence_id = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_a.organization_id,
        fixture.org_a.target_id,
        "evidence",
    )
    .await;
    sqlx::query(
        "INSERT INTO candidate_attempt_evidence(attempt_id,evidence_id,role) VALUES ($1,$2,'proof')",
    )
    .bind(attempt_id)
    .bind(evidence_id)
    .execute(db.pool())
    .await
    .expect("link attempt proof evidence");
    sqlx::query(
        "INSERT INTO candidate_attempt_evidence(attempt_id,evidence_id,role)
         VALUES ($1,$2,'fact_delta')",
    )
    .bind(attempt_id)
    .bind(delta_evidence_id)
    .execute(db.pool())
    .await
    .expect("link retained FactDelta evidence to its source Attempt");
    let terminal_result_json = serde_json::json!({
        "disposition": "verified",
        "proof_evidence_ids": [evidence_id]
    });
    sqlx::query(
        "UPDATE candidate_attempts SET status='submitted',
             result_json=$2,result_hash=$3,updated_at=NOW()
         WHERE id=$1",
    )
    .bind(attempt_id)
    .bind(&terminal_result_json)
    .bind(format!("sha256:{}", sha256_json(&terminal_result_json)))
    .execute(db.pool())
    .await
    .expect("submit retained Attempt after freezing proof membership");
    sqlx::query(
        "UPDATE candidate_attempts
            SET status='verified',terminal_at=NOW(),updated_at=NOW()
          WHERE id=$1 AND status='submitted'",
    )
    .bind(attempt_id)
    .execute(db.pool())
    .await
    .expect("terminalize retained submitted Attempt");
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
               'https://shared.example.test/login','sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963',
               'sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5'
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
    let mut canonical_ref_tx = db
        .pool()
        .begin()
        .await
        .expect("begin retained target canonical resolution");
    let retained_target_ref = canonical_fact_refs::resolve_for_handoff(
        &mut canonical_ref_tx,
        fixture.operation_id,
        fixture.org_a.organization_id,
        "/tmp/attack-v2",
        chrono::Utc::now(),
        &[canonical_fact_refs::CanonicalFactKey::Target {
            target_id: fixture.org_a.target_id,
        }],
    )
    .await
    .expect("resolve retained live target")
    .pop()
    .expect("one retained target canonical ref");
    canonical_ref_tx
        .rollback()
        .await
        .expect("finish retained target canonical resolution");
    let retained_delta_dedupe = golish_db::repo::attack_fact_deltas::semantic_dedupe_hash(
        "sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963",
        "target",
        fixture.org_a.target_id,
        1,
        &retained_target_ref.content_sha256,
        "refuted",
    )
    .expect("hash retained FactDelta semantic identity");
    sqlx::query("UPDATE attack_wave_runs SET status='verification',updated_at=NOW() WHERE id=$1")
        .bind(fixture.wave_run_id)
        .execute(db.pool())
        .await
        .expect("advance retained FactDelta source Wave to verification");
    let fact_delta_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO attack_fact_deltas (
               id,source_attempt_id,candidate_id,operation_id,scope_snapshot_id,
               wave_run_id,wave_unit_id,organization_id,target_live_id,
               target_type_at_time,target_value_at_time,target_identity_hash,
               candidate_plan_hash,canonical_ref_kind,canonical_ref_id,
               canonical_ref_version,canonical_ref_hash,delta_kind,dedupe_hash
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,$9,'url',
               'https://shared.example.test/login','sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963',
               'sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5','target',$10,1,$11,
               'refuted',$12
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
    .bind(fixture.org_a.target_id)
    .bind(&retained_target_ref.content_sha256)
    .bind(&retained_delta_dedupe)
    .execute(db.pool())
    .await
    .expect("insert retained fact delta");
    sqlx::query(
        "INSERT INTO attack_fact_delta_evidence(fact_delta_id,evidence_id,role) VALUES ($1,$2,'fact_delta')",
    )
    .bind(fact_delta_id)
    .bind(delta_evidence_id)
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
               'sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963','attempt_cap','cap reached','sha256:policy',1,1,0,1
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
    assert_eq!(
        retained_join_count, 5,
        "the original evidence must retain its five independent provenance memberships"
    );
    let retained_delta_evidence: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM attack_fact_delta_evidence
          WHERE fact_delta_id=$1 AND evidence_id=$2 AND role='fact_delta'",
    )
    .bind(fact_delta_id)
    .bind(delta_evidence_id)
    .fetch_one(db.pool())
    .await
    .expect("count retained Attempt-interval FactDelta evidence membership");
    assert_eq!(retained_delta_evidence, 1);
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
               'https://shared.example.test/login','sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963',
               'sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5'
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
async fn runtime_cutover_keeps_attack_at_dual_write_legacy_for_new_operations() {
    let (mut db, _data_dir) = migrated_db("full_cutover_defaults").await;

    let runtime_rollout: (String, i16, i64) = sqlx::query_as(
        "SELECT contract,contract_rank,row_version
           FROM runtime_memory_rollout WHERE singleton_id=1",
    )
    .fetch_one(db.pool())
    .await
    .expect("read post-cutover runtime rollout");
    assert_eq!(
        runtime_rollout,
        ("dual_write_legacy_read".to_string(), 1, 1)
    );

    let attack_rollout: (String, i16, i64) = sqlx::query_as(
        "SELECT contract,rank,row_version
           FROM attack_execution_rollout WHERE singleton=TRUE",
    )
    .fetch_one(db.pool())
    .await
    .expect("read post-cutover attack rollout");
    assert_eq!(attack_rollout, ("dual_write_read_legacy".to_string(), 1, 1));

    let session_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO sessions (id,title,status,project_path)
         VALUES ($1,'cutover operation','running','/tmp/attack-v2-cutover')",
    )
    .bind(session_id)
    .execute(db.pool())
    .await
    .expect("insert cutover fixture session");
    sqlx::query(
        "INSERT INTO project_scopes (
             project_scope_id,canonical_project_path,path_sha256
         ) VALUES ($1,'/tmp/attack-v2-cutover','sha256:attack-v2-cutover')",
    )
    .bind(project_scope_id)
    .execute(db.pool())
    .await
    .expect("insert cutover fixture project scope");

    let operation_id = Uuid::new_v4();
    let created = runtime_memory_tx::create_runtime_operation(
        db.pool(),
        &runtime_memory_tx::CreateRuntimeOperationRow {
            operation_id,
            initial_stage_execution_id: Uuid::new_v4(),
            session_id,
            title: Some("post-cutover V2 operation".to_string()),
            input: "run frozen V2 operation".to_string(),
            profile: "red_team".to_string(),
            entry_stage: "scoping".to_string(),
            project_scope_id,
            cli_scope: None,
        },
    )
    .await
    .expect("new operation must freeze both deployment singleton contracts atomically");
    assert_eq!(
        created.operation.runtime_memory_contract,
        "dual_write_legacy_read"
    );
    let frozen: (String, String) = sqlx::query_as(
        "SELECT runtime_memory_contract,attack_execution_contract
           FROM operation_state WHERE operation_id=$1",
    )
    .bind(operation_id)
    .fetch_one(db.pool())
    .await
    .expect("read new operation frozen contracts");
    assert_eq!(
        frozen,
        (
            "dual_write_legacy_read".to_string(),
            "dual_write_read_legacy".to_string()
        )
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn v2_contract_requires_runtime_memory_v2_and_is_immutable() {
    let (mut db, _data_dir) = migrated_db("contract").await;
    reset_attack_rollout_to_legacy_for_transition_fixture(db.pool()).await;
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
                    target_identity_hash:
                        "sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963"
                            .to_string(),
                    technique: "WSTG-INPV-05".to_string(),
                    observation: serde_json::json!({"outcome": "found"}),
                    observation_hash: "sha256:candidate-observation".to_string(),
                    source_fact_delta_id: None,
                    delta_kind: None,
                    observation_kind: "legacy_observation".to_string(),
                    allowed_techniques: vec!["WSTG-INPV-05".to_string()],
                    enrichment_required: false,
                    evidence_ids: vec![evidence_id],
                },
                golish_db::repo::attack_candidate_work_items::SeedAttackObservation {
                    work_item_key: "formulaic:checked-empty".to_string(),
                    target_live_id: Some(fixture.org_a.target_id),
                    target_type_at_time: "url".to_string(),
                    target_value_at_time: "https://shared.example.test/login".to_string(),
                    target_identity_hash:
                        "sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963"
                            .to_string(),
                    technique: "WSTG-INPV-01".to_string(),
                    observation: serde_json::json!({"outcome": "empty"}),
                    observation_hash: "sha256:checked-empty-observation".to_string(),
                    source_fact_delta_id: None,
                    delta_kind: None,
                    observation_kind: "legacy_observation".to_string(),
                    allowed_techniques: vec!["WSTG-INPV-01".to_string()],
                    enrichment_required: false,
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

    let non_passed_stage_execution_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status)
         VALUES($1,$2,'attack_candidate','started')",
    )
    .bind(non_passed_stage_execution_id)
    .bind(fixture.operation_id)
    .execute(db.pool())
    .await
    .expect("insert non-passed Candidate StageExecution hostile fixture");
    let non_passed_stage_run_unit_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,organization_id,
               stage_kind,generation,specialist,status,started_at,terminal_at
           ) VALUES(
               $1,$2,$3,$4,$5,'attack_candidate',0,'attack_analyst',
               'gate_blocked',NOW(),NOW()
           )"#,
    )
    .bind(non_passed_stage_run_unit_id)
    .bind(fixture.operation_id)
    .bind(non_passed_stage_execution_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.org_a.organization_id)
    .execute(db.pool())
    .await
    .expect("insert non-passed Candidate source Unit hostile fixture");
    let mut non_passed_command = command.clone();
    non_passed_command.decision_stage_execution_id = non_passed_stage_execution_id;
    non_passed_command.decision_stage_run_unit_id = non_passed_stage_run_unit_id;
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin rejected gate transaction");
    assert!(
        accept_gate_passed_candidate_batch(&mut tx, non_passed_command)
            .await
            .is_err()
    );
    tx.rollback()
        .await
        .expect("rollback rejected gate transaction");

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

    let mut decision_evidence_drift_tx = db
        .pool()
        .begin()
        .await
        .expect("begin decided evidence membership drift");
    let decision_evidence_drift = sqlx::query(
        "DELETE FROM attack_candidate_work_item_evidence
          WHERE work_item_id=$1 AND evidence_id=$2 AND role='decision'",
    )
    .bind(no_candidate_item.work_item_id)
    .bind(evidence_id)
    .execute(&mut *decision_evidence_drift_tx)
    .await;
    decision_evidence_drift_tx
        .rollback()
        .await
        .expect("rollback decided evidence membership drift");
    assert_sqlstate(
        decision_evidence_drift.map(|_| ()),
        "23514",
        "rewrite no-candidate evidence membership after the final handoff",
    );

    let mut audit_semantic_drift_tx = db
        .pool()
        .begin()
        .await
        .expect("begin frozen evidence semantic drift");
    let audit_semantic_drift = sqlx::query(
        "UPDATE audit_log
            SET detail=jsonb_build_object('organization_id',$2::text,'kind','forged')
          WHERE id=$1",
    )
    .bind(evidence_id)
    .bind(fixture.org_b.organization_id)
    .execute(&mut *audit_semantic_drift_tx)
    .await;
    audit_semantic_drift_tx
        .rollback()
        .await
        .expect("rollback frozen evidence semantic drift");
    assert_sqlstate(
        audit_semantic_drift.map(|_| ()),
        "23514",
        "rewrite evidence owner semantics after the final handoff",
    );

    let mut target_delete_tx = db
        .pool()
        .begin()
        .await
        .expect("begin true target-FK deletion");
    sqlx::query("DELETE FROM targets WHERE id=$1 AND organization_id=$2")
        .bind(fixture.org_a.target_id)
        .bind(fixture.org_a.organization_id)
        .execute(&mut *target_delete_tx)
        .await
        .expect("true target deletion may clear live evidence pointers");
    let cleared_evidence_target: Option<Uuid> =
        sqlx::query_scalar("SELECT target_id FROM audit_log WHERE id=$1")
            .bind(evidence_id)
            .fetch_one(&mut *target_delete_tx)
            .await
            .expect("read FK-cleared evidence target pointer");
    assert_eq!(cleared_evidence_target, None);
    target_delete_tx
        .rollback()
        .await
        .expect("rollback true target-FK deletion");

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
    reset_attack_rollout_to_legacy_for_transition_fixture(db.pool()).await;
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

    let mut cohort_blocked_tx = db
        .pool()
        .begin()
        .await
        .expect("begin empty-cohort adjacent rollout transaction");
    let cohort_error = attack_execution_rollout::advance_attack_execution_rollout(
        &mut cohort_blocked_tx,
        1,
        AttackExecutionContract::DualWriteReadV2Fallback,
    )
    .await
    .expect_err("rank two requires a complete admitted Candidate cohort");
    assert!(
        cohort_error
            .to_string()
            .contains("ATTACK_ROLLOUT_COHORT_NOT_READY: candidate_cohort_empty"),
        "unexpected empty-cohort rollout error: {cohort_error}"
    );
    cohort_blocked_tx
        .rollback()
        .await
        .expect("rollback empty-cohort rollout transaction");
    let preferred_runtime_operation_id = Uuid::new_v4();
    operation_state::insert(
        db.pool(),
        preferred_runtime_operation_id,
        "red_team",
        "scoping",
        "dual_write_v2_preferred",
    )
    .await
    .expect("dual-write attack may run with the preferred V2 runtime-memory reader");
    let v2_only_runtime_operation_id = Uuid::new_v4();
    operation_state::insert(
        db.pool(),
        v2_only_runtime_operation_id,
        "red_team",
        "scoping",
        "v2_only",
    )
    .await
    .expect("dual-write attack may run with the V2-only runtime-memory contract");
    let incompatible_legacy_runtime = operation_state::insert(
        db.pool(),
        Uuid::new_v4(),
        "red_team",
        "scoping",
        "legacy_v1",
    )
    .await;
    assert!(
        incompatible_legacy_runtime.is_err(),
        "a dual-write attack contract must reject legacy-only runtime memory"
    );
    let compatible_contracts: Vec<(String, String)> = sqlx::query_as(
        "SELECT runtime_memory_contract,attack_execution_contract
           FROM operation_state WHERE operation_id=ANY($1) ORDER BY operation_id",
    )
    .bind(vec![
        preferred_runtime_operation_id,
        v2_only_runtime_operation_id,
    ])
    .fetch_all(db.pool())
    .await
    .expect("read compatible frozen runtime/attack contract pairs");
    assert_eq!(compatible_contracts.len(), 2);
    assert!(compatible_contracts
        .iter()
        .all(|(_, attack)| { attack == AttackExecutionContract::DualWriteReadLegacy.as_str() }));
    assert_eq!(
        compatible_contracts
            .iter()
            .map(|(runtime, _)| runtime.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
        ["dual_write_v2_preferred", "v2_only"].into_iter().collect(),
    );
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
async fn attack_rollout_shadow_promotion_requires_complete_matching_samples() {
    let (mut db, _data_dir) = migrated_db("rollout_shadow_promotion_gate").await;
    reset_attack_rollout_to_legacy_for_transition_fixture(db.pool()).await;

    let mut enable_tx = db.pool().begin().await.expect("begin dual enable");
    let enabled = attack_execution_rollout::promote_attack_execution_rollout(
        &mut enable_tx,
        0,
        AttackExecutionContract::DualWriteReadLegacy,
    )
    .await
    .expect("legacy to first dual rank enables sampling without impossible prior samples");
    enable_tx.commit().await.expect("commit dual enable");
    assert_eq!(enabled.contract, "dual_write_read_legacy");

    let mut blocked_tx = db.pool().begin().await.expect("begin empty promotion");
    let error = attack_execution_rollout::promote_attack_execution_rollout(
        &mut blocked_tx,
        enabled.row_version,
        AttackExecutionContract::DualWriteReadV2Fallback,
    )
    .await
    .expect_err("rank one cannot promote without persisted comparisons");
    assert!(
        error
            .to_string()
            .contains("attack_rollout_cohort_not_ready"),
        "empty persisted sample set must fail closed: {error}"
    );
    blocked_tx
        .commit()
        .await
        .expect("commit rejected promotion transaction");

    reset_attack_rollout_to_legacy_for_transition_fixture(db.pool()).await;
    let pool_a = db.pool().clone();
    let pool_b = db.pool().clone();
    let promote_once = |pool: PgPool| async move {
        let mut tx = pool.begin().await.expect("begin concurrent promotion");
        let result = attack_execution_rollout::promote_attack_execution_rollout(
            &mut tx,
            0,
            AttackExecutionContract::DualWriteReadLegacy,
        )
        .await;
        if result.is_ok() {
            tx.commit().await.expect("commit winning promotion");
        } else {
            tx.rollback().await.expect("rollback losing promotion");
        }
        result
    };
    let (left, right) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::join!(promote_once(pool_a), promote_once(pool_b))
    })
    .await
    .expect("two production promoters must serialize without lock-upgrade deadlock");
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);

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
        expected_candidate_plan_hash:
            "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5".to_string(),
        expected_candidate_row_version: 0,
        approve: true,
        start_before: Some(chrono::Utc::now() + chrono::Duration::milliseconds(300)),
        expires_at: Some(chrono::Utc::now() + chrono::Duration::milliseconds(300)),
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
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

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
            expected_candidate_plan_hash:
                "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5"
                    .to_string(),
            expected_candidate_row_version: 0,
            approve: true,
            start_before: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
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
async fn listing_partial_wave_review_keeps_global_wave_open_for_unready_sibling() {
    let (mut db, _data_dir) = migrated_db("review_partial_wave_stays_open").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let _candidate_a = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    sqlx::query(
        "UPDATE attack_wave_units SET status='review',updated_at=NOW()
         WHERE id=$1 AND wave_run_id=$2 AND status='open'",
    )
    .bind(fixture.org_a.wave_unit_id)
    .bind(fixture.wave_run_id)
    .execute(db.pool())
    .await
    .expect("move only org A into durable review");

    let first = list_candidate_reviews(db.pool(), fixture.operation_id, fixture.wave_run_id)
        .await
        .expect("list a partially review-ready Wave");
    assert_eq!(first.barrier.status, "open");
    assert!(!first.review_closed);

    let first_statuses: (String, String, String) = sqlx::query_as(
        r#"SELECT wave.status,
                  (SELECT status FROM attack_wave_units WHERE id=$2),
                  (SELECT status FROM attack_wave_units WHERE id=$3)
             FROM attack_wave_runs wave WHERE wave.id=$1"#,
    )
    .bind(fixture.wave_run_id)
    .bind(fixture.org_a.wave_unit_id)
    .bind(fixture.org_b.wave_unit_id)
    .fetch_one(db.pool())
    .await
    .expect("read partial Wave statuses after listing reviews");
    assert_eq!(
        first_statuses,
        ("open".into(), "review".into(), "open".into()),
        "global Wave must remain open while org B is still schedulable"
    );

    let retry = list_candidate_reviews(db.pool(), fixture.operation_id, fixture.wave_run_id)
        .await
        .expect("retry the partial review read");
    assert_eq!(retry.barrier.status, "open");
    let retry_statuses: (String, String) = sqlx::query_as(
        r#"SELECT wave.status,unit.status
             FROM attack_wave_runs wave
             JOIN attack_wave_units unit ON unit.wave_run_id=wave.id
            WHERE wave.id=$1 AND unit.id=$2"#,
    )
    .bind(fixture.wave_run_id)
    .bind(fixture.org_b.wave_unit_id)
    .fetch_one(db.pool())
    .await
    .expect("read retry scheduling authority for org B");
    assert_eq!(
        retry_statuses,
        ("open".into(), "open".into()),
        "review polling must leave the unready sibling runnable on retry"
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
                expected_candidate_plan_hash:
                    "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5"
                        .to_string(),
                expected_candidate_row_version: 0,
                approve: false,
                start_before: None,
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
        expected_candidate_plan_hash:
            "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5".to_string(),
        expected_candidate_row_version: 0,
        approve: true,
        start_before: Some(chrono::Utc::now() + chrono::Duration::minutes(30)),
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
                expected_candidate_plan_hash:
                    "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5"
                        .to_string(),
                expected_candidate_row_version: 0,
                approve: false,
                start_before: None,
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
                expected_candidate_plan_hash:
                    "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5"
                        .to_string(),
                expected_candidate_row_version: 0,
                approve: false,
                start_before: None,
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
    let claim_query = CandidateClaimQuery {
        operation_id: fixture.operation_id,
        scope_snapshot_id: fixture.scope_snapshot_id,
        wave_run_id: fixture.wave_run_id,
        wave_unit_id: fixture.org_a.wave_unit_id,
        organization_id: fixture.org_a.organization_id,
        verification_stage_execution_id: stage_execution_a,
        verification_stage_run_unit_id: stage_unit_a,
        lease_owner: "claim-test-a".to_string(),
        lease_seconds: 60,
    };
    let claimed = claim_next_candidate_attempt(db.pool(), claim_query.clone())
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
    let replayed = claim_next_candidate_attempt(db.pool(), claim_query)
        .await
        .expect("response-loss retry does not error")
        .expect("same exact claim is replayed");
    assert_eq!(replayed.attempt.id, claimed.attempt.id);
    assert_eq!(replayed.worker.id, claimed.worker.id);
    assert_eq!(replayed.worker.lease_token, claimed.worker.lease_token);
    let retained_rows: (i64, i64) = sqlx::query_as(
        r#"SELECT
               (SELECT COUNT(*) FROM candidate_attempts WHERE candidate_id=$1),
               (SELECT COUNT(*) FROM stage_worker_runs
                 WHERE work_item_kind='candidate_attempt' AND work_item_key=$2)"#,
    )
    .bind(candidate_a.candidate_id)
    .bind(claimed.attempt.id.to_string())
    .fetch_one(db.pool())
    .await
    .expect("count response-loss claim rows");
    assert_eq!(
        retained_rows,
        (1, 1),
        "claim replay cannot consume fuel twice"
    );

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

    let release = CandidateExecutionRelease {
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
    };
    assert_eq!(
        candidate_execution_continuation(db.pool(), &release)
            .await
            .expect("classify pristine Candidate release"),
        CandidateExecutionContinuation::SafeRelease
    );
    let released = release_candidate_execution(db.pool(), release)
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
    let release_result = ownership.4.as_ref().expect("durable retry release result");
    assert_eq!(release_result["disposition"], "retryable_failed");
    assert_eq!(release_result["reason_code"], "worker_released_for_retry");
    assert_eq!(release_result["schema_version"], 1);
    assert!(
        release_result["release_fence_hash"]
            .as_str()
            .is_some_and(|hash| hash.starts_with("sha256:")),
        "retry release must persist the full command fence"
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
async fn release_preserves_retry_fuel_and_replays_after_live_target_deletion() {
    let (mut db, _data_dir) = migrated_db("candidate_release_retry_fuel").await;
    let fixture = seed_attack_fixture(db.pool()).await;

    let retry_candidate = seed_candidate(db.pool(), &fixture, fixture.org_b).await;
    let retry_approval = insert_approval(db.pool(), &fixture, retry_candidate, fixture.org_b)
        .await
        .expect("approve the pre-existing retry Candidate");
    sqlx::query("UPDATE attack_candidates SET disposition='approved' WHERE candidate_id=$1")
        .bind(retry_candidate.candidate_id)
        .execute(db.pool())
        .await
        .expect("mark the pre-existing retry Candidate approved");
    sqlx::query(
        r#"INSERT INTO candidate_attempts (
               id,candidate_id,approval_id,operation_id,scope_snapshot_id,
               wave_run_id,wave_unit_id,organization_id,target_live_id,
               target_type_at_time,target_value_at_time,target_identity_hash,
               candidate_plan_hash,ordinal,status,result_json,result_hash
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,$9,'url',
               'https://shared.example.test/login',
               'sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963',
               'sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5',
               0,'retryable_failed',$10,'sha256:preexisting-retry'
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(retry_candidate.candidate_id)
    .bind(retry_approval)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.wave_run_id)
    .bind(fixture.org_b.wave_unit_id)
    .bind(fixture.org_b.organization_id)
    .bind(fixture.org_b.target_id)
    .bind(serde_json::json!({
        "disposition": "retryable_failed",
        "reason_code": "preexisting_retry"
    }))
    .execute(db.pool())
    .await
    .expect("seed one durable retry backlog row");

    let support_evidence_id = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_a.organization_id,
        fixture.org_a.target_id,
        "evidence",
    )
    .await;
    let current_candidate =
        seed_candidate_with_support(db.pool(), &fixture, fixture.org_a, &[support_evidence_id])
            .await;
    let current_approval = insert_approval(db.pool(), &fixture, current_candidate, fixture.org_a)
        .await
        .expect("approve the current fuel Candidate");
    sqlx::query("UPDATE attack_candidates SET disposition='approved' WHERE candidate_id=$1")
        .bind(current_candidate.candidate_id)
        .execute(db.pool())
        .await
        .expect("mark the current fuel Candidate approved");
    sqlx::query(
        r#"INSERT INTO candidate_attempts (
               id,candidate_id,approval_id,operation_id,scope_snapshot_id,
               wave_run_id,wave_unit_id,organization_id,target_live_id,
               target_type_at_time,target_value_at_time,target_identity_hash,
               candidate_plan_hash,ordinal,status
           )
           SELECT uuid_generate_v4(),$1,$2,$3,$4,$5,$6,$7,$8,'url',
                  'https://shared.example.test/login',
                  'sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963',
                  'sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5',
                  ordinal,'abandoned'
             FROM generate_series(0,196) AS ordinal"#,
    )
    .bind(current_candidate.candidate_id)
    .bind(current_approval)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.wave_run_id)
    .bind(fixture.org_a.wave_unit_id)
    .bind(fixture.org_a.organization_id)
    .bind(fixture.org_a.target_id)
    .execute(db.pool())
    .await
    .expect("seed 197 historical Attempts before the current claim");

    let late_support_evidence_id = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_a.organization_id,
        fixture.org_a.target_id,
        "evidence",
    )
    .await;
    let late_support = sqlx::query(
        "INSERT INTO attack_candidate_evidence(candidate_id,evidence_id,role)
         VALUES($1,$2,'support')",
    )
    .bind(current_candidate.candidate_id)
    .bind(late_support_evidence_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        late_support.map(|_| ()),
        "23514",
        "Candidate support mutation after WorkItem decision",
    );
    let candidate_semantic_tamper = sqlx::query(
        "UPDATE attack_candidates SET hypothesis='caller-mutated' WHERE candidate_id=$1",
    )
    .bind(current_candidate.candidate_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        candidate_semantic_tamper.map(|_| ()),
        "23514",
        "Candidate semantic identity mutation",
    );
    let approval_plan_tamper =
        sqlx::query("UPDATE attack_candidate_approvals SET budget='{}'::jsonb WHERE id=$1")
            .bind(current_approval)
            .execute(db.pool())
            .await;
    assert_sqlstate(
        approval_plan_tamper.map(|_| ()),
        "23514",
        "Approval frozen plan mutation",
    );
    let work_item_decision_tamper = sqlx::query(
        "UPDATE attack_candidate_work_items SET work_item_key='caller-mutated' WHERE id=$1",
    )
    .bind(current_candidate.work_item_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        work_item_decision_tamper.map(|_| ()),
        "23514",
        "decided Candidate WorkItem mutation",
    );

    sqlx::query(
        "UPDATE attack_wave_units
            SET review_closed=TRUE,status='verification',updated_at=NOW()
          WHERE id=ANY($1)",
    )
    .bind(vec![fixture.org_a.wave_unit_id, fixture.org_b.wave_unit_id])
    .execute(db.pool())
    .await
    .expect("close both Candidate reviews for retry-fuel verification");
    let (current_stage_execution, current_stage_unit) =
        seed_verification_unit(db.pool(), &fixture, fixture.org_a).await;
    let (retry_stage_execution, retry_stage_unit) =
        seed_verification_unit(db.pool(), &fixture, fixture.org_b).await;
    let claimed = claim_next_candidate_attempt(
        db.pool(),
        CandidateClaimQuery {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_a.wave_unit_id,
            organization_id: fixture.org_a.organization_id,
            verification_stage_execution_id: current_stage_execution,
            verification_stage_run_unit_id: current_stage_unit,
            lease_owner: "retry-fuel-current".to_string(),
            lease_seconds: 120,
        },
    )
    .await
    .expect("claim current Candidate at E=198 and R=1")
    .expect("one current Candidate slot remains");
    assert_eq!(claimed.attempt.ordinal, 197);
    let lease_token = claimed
        .worker
        .lease_token
        .expect("current fuel lease token");
    let release = CandidateExecutionRelease {
        operation_id: fixture.operation_id,
        scope_snapshot_id: fixture.scope_snapshot_id,
        wave_run_id: fixture.wave_run_id,
        wave_unit_id: fixture.org_a.wave_unit_id,
        organization_id: fixture.org_a.organization_id,
        attempt_id: claimed.attempt.id,
        worker_run_id: claimed.worker.id,
        stage_execution_id: current_stage_execution,
        stage_run_unit_id: current_stage_unit,
        lease_token,
        lease_owner: "retry-fuel-current".to_string(),
        attempt_epoch: claimed.worker.attempt_epoch,
        expected_checkpoint_version: claimed.worker.checkpoint_version,
    };
    let released = release_candidate_execution(db.pool(), release.clone())
        .await
        .expect("release at E=199,R=1,F=1 must converge terminally");
    assert!(!released.requeued);
    let fuel_result: (String, serde_json::Value, String, Uuid) = sqlx::query_as(
        r#"SELECT attempt.status,attempt.result_json,residual.reason_code,residual.id
             FROM candidate_attempts AS attempt
             JOIN attack_residual_risks AS residual
               ON residual.operation_id=attempt.operation_id
              AND residual.wave_run_id=attempt.wave_run_id
              AND residual.organization_id=attempt.organization_id
            WHERE attempt.id=$1 AND residual.reason_code='max_attempts_total'"#,
    )
    .bind(claimed.attempt.id)
    .fetch_one(db.pool())
    .await
    .expect("load terminal fuel-preservation release");
    assert_eq!(fuel_result.0, "blocked");
    assert_eq!(fuel_result.1["disposition"], "blocked");
    assert_eq!(fuel_result.1["residual"]["attempt_count"], 199);
    assert_eq!(fuel_result.2, "max_attempts_total");
    let exact_replay = release_candidate_execution(db.pool(), release.clone())
        .await
        .expect("release response loss must replay the exact residual graph");
    assert!(!exact_replay.requeued);

    let candidate_delete = sqlx::query("DELETE FROM attack_candidates WHERE candidate_id=$1")
        .bind(current_candidate.candidate_id)
        .execute(db.pool())
        .await;
    assert_sqlstate(
        candidate_delete.map(|_| ()),
        "23514",
        "Candidate audit ledger deletion",
    );
    let attempt_delete = sqlx::query("DELETE FROM candidate_attempts WHERE id=$1")
        .bind(claimed.attempt.id)
        .execute(db.pool())
        .await;
    assert_sqlstate(
        attempt_delete.map(|_| ()),
        "23514",
        "Attempt audit ledger deletion",
    );
    let attempt_result_tamper =
        sqlx::query("UPDATE candidate_attempts SET result_hash='sha256:tampered' WHERE id=$1")
            .bind(claimed.attempt.id)
            .execute(db.pool())
            .await;
    assert_sqlstate(
        attempt_result_tamper.map(|_| ()),
        "23514",
        "terminal Attempt result mutation",
    );
    let late_action = sqlx::query(
        r#"INSERT INTO candidate_attempt_actions(
               attempt_id,action_ordinal,capability_id,action_kind,canonical_args,status,
               outcome,outcome_hash,started_at,completed_at)
           VALUES($1,0,'verify.sql_injection','bounded_sql_injection_probe','{}',
                  'completed','{}','sha256:late-action',NOW(),NOW())"#,
    )
    .bind(claimed.attempt.id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        late_action.map(|_| ()),
        "23514",
        "terminal Attempt action journal mutation",
    );
    let residual_tamper =
        sqlx::query("UPDATE attack_residual_risks SET reason_detail='caller-mutated' WHERE id=$1")
            .bind(fuel_result.3)
            .execute(db.pool())
            .await;
    assert_sqlstate(
        residual_tamper.map(|_| ()),
        "23514",
        "fuel residual canonical mutation",
    );

    sqlx::query("DELETE FROM targets WHERE id=$1")
        .bind(fixture.org_a.target_id)
        .execute(db.pool())
        .await
        .expect("delete only the live target pointer while retaining audit rows");
    type RetainedTargetPointers = (
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
    );
    let retained_target_pointers: RetainedTargetPointers = sqlx::query_as(
        r#"SELECT candidate.target_live_id,candidate.live_target_id,
                  approval.target_live_id,approval.live_target_id,
                  attempt.target_live_id,residual.target_live_id
             FROM attack_candidates AS candidate
             JOIN attack_candidate_approvals AS approval
               ON approval.candidate_id=candidate.candidate_id
             JOIN candidate_attempts AS attempt ON attempt.id=$2
             JOIN attack_residual_risks AS residual ON residual.id=$3
            WHERE candidate.candidate_id=$1 AND approval.id=$4"#,
    )
    .bind(current_candidate.candidate_id)
    .bind(claimed.attempt.id)
    .bind(fuel_result.3)
    .bind(current_approval)
    .fetch_one(db.pool())
    .await
    .expect("load retained at-time rows after live target deletion");
    assert_eq!(
        retained_target_pointers,
        (None, None, None, None, None, None)
    );
    let post_delete_replay = release_candidate_execution(db.pool(), release)
        .await
        .expect("release replay must normalize the deleted live target pointer");
    assert!(!post_delete_replay.requeued);

    let retried = claim_next_candidate_attempt(
        db.pool(),
        CandidateClaimQuery {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_b.wave_unit_id,
            organization_id: fixture.org_b.organization_id,
            verification_stage_execution_id: retry_stage_execution,
            verification_stage_run_unit_id: retry_stage_unit,
            lease_owner: "retry-fuel-preserved".to_string(),
            lease_seconds: 120,
        },
    )
    .await
    .expect("claim the preserved retry slot")
    .expect("the pre-existing retry Candidate must retain the last slot");
    assert_eq!(retried.attempt.ordinal, 1);
    let final_fuel: (i64, i64) = sqlx::query_as(
        r#"SELECT
               (SELECT COUNT(*) FROM candidate_attempts WHERE operation_id=$1),
               (SELECT COUNT(*) FROM attack_candidates AS candidate
                 WHERE candidate.operation_uuid=$1 AND candidate.disposition='approved'
                   AND (
                       SELECT latest.status FROM candidate_attempts AS latest
                        WHERE latest.candidate_id=candidate.candidate_id
                        ORDER BY latest.ordinal DESC LIMIT 1
                   )='retryable_failed')"#,
    )
    .bind(fixture.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("load final E/R convergence counts");
    assert_eq!(final_fuel, (200, 0));

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn terminalizer_replay_returns_same_finding_and_lineage() {
    let (mut db, _data_dir) = migrated_db("terminalize_verified").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let candidate = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    ensure_fixture_manifest_is_evidenced_for_close(db.pool(), &fixture, fixture.org_a.wave_unit_id)
        .await;
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
        },
        "fact_deltas": [{
            "fact_kind": "new_surface",
            "canonical_ref_kind": "attack_candidate_work_item",
            "canonical_ref_id": candidate.work_item_id,
            "canonical_ref_version": 1,
            "canonical_ref_hash": "sha256:terminalizer-canonical-ref",
            "summary": "The bounded proof changes the next-wave attack surface.",
            "evidence_ids": [proof_evidence_id]
        }]
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
        candidate_plan_hash:
            "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5".to_string(),
        worker_run_id: claimed.worker.id,
        stage_execution_id,
        stage_run_unit_id,
        lease_token,
        lease_owner: "terminalizer-test".to_string(),
        attempt_epoch: claimed.worker.attempt_epoch,
        expected_checkpoint_version: claimed.worker.checkpoint_version,
        result_json,
        evidence: vec![
            AttemptEvidenceLink {
                evidence_id: proof_evidence_id,
                role: "proof".to_string(),
            },
            AttemptEvidenceLink {
                evidence_id: proof_evidence_id,
                role: "fact_delta".to_string(),
            },
        ],
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
        candidate_plan_hash:
            "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5".to_string(),
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
    let generic_terminalize = TerminalizeCandidateAttempt {
        operation_id: exact.operation_id,
        scope_snapshot_id: exact.scope_snapshot_id,
        wave_run_id: exact.wave_run_id,
        wave_unit_id: exact.wave_unit_id,
        organization_id: exact.organization_id,
        candidate_id: exact.candidate_id,
        approval_id: exact.approval_id,
        attempt_id: exact.attempt_id,
        candidate_plan_hash: exact.candidate_plan_hash.clone(),
        expected_result_hash: exact.expected_result_hash.clone(),
        worker_run_id: exact.worker_run_id,
        stage_execution_id: exact.stage_execution_id,
        stage_run_unit_id: exact.stage_run_unit_id,
        lease_token: exact.lease_token,
        lease_owner: exact.lease_owner.clone(),
        attempt_epoch: exact.attempt_epoch,
        expected_checkpoint_version: exact.expected_checkpoint_version,
    };
    let mut terminal_tx = db.pool().begin().await.expect("begin terminalization");
    let terminal = terminalize_candidate_attempt(&mut terminal_tx, generic_terminalize.clone())
        .await
        .expect("terminalize exact verified proof");
    terminal_tx.commit().await.expect("commit terminalization");
    assert!(!terminal.replayed);
    assert_eq!(terminal.scope_snapshot_id, fixture.scope_snapshot_id);
    assert_eq!(terminal.wave_run_id, fixture.wave_run_id);
    assert_eq!(terminal.wave_unit_id, fixture.org_a.wave_unit_id);
    assert_eq!(terminal.organization_id, fixture.org_a.organization_id);
    assert_eq!(terminal.candidate_id, candidate.candidate_id);
    assert_eq!(terminal.attempt_id, claimed.attempt.id);
    assert_eq!(terminal.status, "verified");
    assert_eq!(terminal.disposition, "verified");
    assert_eq!(terminal.evidence_count, 1);
    assert_eq!(terminal.fact_delta_count, 1);
    let terminal_finding_id = terminal.finding_id.expect("verified Finding id");

    let mut replay_tx = db.pool().begin().await.expect("begin terminal replay");
    let replay = terminalize_candidate_attempt(&mut replay_tx, generic_terminalize)
        .await
        .expect("terminalization response-loss replay");
    replay_tx.commit().await.expect("commit terminal replay");
    assert!(replay.replayed);
    assert_eq!(replay.finding_id, terminal.finding_id);
    assert_eq!(replay.scope_snapshot_id, terminal.scope_snapshot_id);
    assert_eq!(replay.wave_run_id, terminal.wave_run_id);
    assert_eq!(replay.wave_unit_id, terminal.wave_unit_id);
    assert_eq!(replay.organization_id, terminal.organization_id);
    assert_eq!(replay.candidate_id, terminal.candidate_id);
    assert_eq!(replay.attempt_id, terminal.attempt_id);
    assert_eq!(replay.status, terminal.status);
    assert_eq!(replay.disposition, terminal.disposition);
    assert_eq!(replay.evidence_count, terminal.evidence_count);
    assert_eq!(replay.fact_delta_count, terminal.fact_delta_count);
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
    assert_eq!(state.3, Some(terminal_finding_id));
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
        serde_json::json!(terminal_finding_id)
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
    assert_eq!(retained_replay.finding_id, terminal_finding_id);
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
        Some(terminal_finding_id)
    );
    assert!(truth.snapshots[0].attempts[0].finding_lineage_exact);
    let primary_worker_run_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM stage_worker_runs
         WHERE stage_run_unit_id=$1 AND work_item_kind='organization'
           AND work_item_key='verification' AND worker_generation=0",
    )
    .bind(stage_run_unit_id)
    .fetch_one(db.pool())
    .await
    .expect("load verification logical primary WorkerRun");

    sqlx::query("UPDATE stage_worker_runs SET work_item_key='wrong-verification' WHERE id=$1")
        .bind(primary_worker_run_id)
        .execute(db.pool())
        .await
        .expect("corrupt verification logical primary WorkerRun identity");
    let mut wrong_primary_tx = db
        .pool()
        .begin()
        .await
        .expect("begin wrong-primary close attempt");
    let wrong_primary_error = golish_db::repo::verification_truth::close_verification_unit(
        &mut wrong_primary_tx,
        golish_db::repo::verification_truth::CloseVerificationUnit {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_a.wave_unit_id,
            organization_id: fixture.org_a.organization_id,
            verification_stage_execution_id: stage_execution_id,
            verification_stage_run_unit_id: stage_run_unit_id,
        },
    )
    .await
    .expect_err("wrong logical primary WorkerRun must fail closed");
    assert!(
        wrong_primary_error
            .to_string()
            .contains("logical primary WorkerRun"),
        "unexpected wrong-primary error: {wrong_primary_error}"
    );
    wrong_primary_tx
        .rollback()
        .await
        .expect("rollback wrong-primary close attempt");
    sqlx::query("UPDATE stage_worker_runs SET work_item_key='verification' WHERE id=$1")
        .bind(primary_worker_run_id)
        .execute(db.pool())
        .await
        .expect("restore verification logical primary WorkerRun identity");

    sqlx::query(
        "UPDATE stage_worker_runs
         SET status='running',lease_token=$2,lease_owner='verification-close-test',
             lease_acquired_at=NOW(),lease_expires_at=NOW()+INTERVAL '1 minute',
             heartbeat_at=NOW(),started_at=NOW()
         WHERE id=$1",
    )
    .bind(primary_worker_run_id)
    .bind(Uuid::new_v4())
    .execute(db.pool())
    .await
    .expect("lease verification logical primary WorkerRun");
    let mut leased_primary_tx = db
        .pool()
        .begin()
        .await
        .expect("begin leased-primary close attempt");
    let leased_primary_error = golish_db::repo::verification_truth::close_verification_unit(
        &mut leased_primary_tx,
        golish_db::repo::verification_truth::CloseVerificationUnit {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_a.wave_unit_id,
            organization_id: fixture.org_a.organization_id,
            verification_stage_execution_id: stage_execution_id,
            verification_stage_run_unit_id: stage_run_unit_id,
        },
    )
    .await
    .expect_err("an active verification primary lease must fail closed");
    assert!(
        leased_primary_error
            .to_string()
            .contains("logical primary WorkerRun is not close-ready"),
        "unexpected leased-primary error: {leased_primary_error}"
    );
    leased_primary_tx
        .rollback()
        .await
        .expect("rollback leased-primary close attempt");
    sqlx::query(
        "UPDATE stage_worker_runs
         SET status='queued',lease_token=NULL,lease_owner=NULL,lease_acquired_at=NULL,
             lease_expires_at=NULL,heartbeat_at=NULL,started_at=NULL
         WHERE id=$1",
    )
    .bind(primary_worker_run_id)
    .execute(db.pool())
    .await
    .expect("restore queued verification logical primary WorkerRun");

    let close_command = golish_db::repo::verification_truth::CloseVerificationUnit {
        operation_id: fixture.operation_id,
        scope_snapshot_id: fixture.scope_snapshot_id,
        wave_run_id: fixture.wave_run_id,
        wave_unit_id: fixture.org_a.wave_unit_id,
        organization_id: fixture.org_a.organization_id,
        verification_stage_execution_id: stage_execution_id,
        verification_stage_run_unit_id: stage_run_unit_id,
    };
    let mut template_tx = db
        .pool()
        .begin()
        .await
        .expect("begin exact typed-handoff template close");
    golish_db::repo::verification_truth::close_verification_unit(&mut template_tx, close_command)
        .await
        .expect("build exact typed-handoff template");
    let template =
        sqlx::query_as::<_, golish_db::repo::stage_handoffs::VerificationStageHandoffRow>(
            "SELECT * FROM verification_stage_handoffs WHERE source_stage_run_unit_id=$1",
        )
        .bind(stage_run_unit_id)
        .fetch_one(&mut *template_tx)
        .await
        .expect("load exact typed-handoff template");
    template_tx
        .rollback()
        .await
        .expect("rollback exact typed-handoff template close");

    let preseed = try_unready_verification_handoff_preseed(db.pool(), &template).await;
    assert_sqlstate(
        preseed,
        "23514",
        "Verification handoff authority key before its owner close transition",
    );
    sqlx::raw_sql(
        r#"CREATE FUNCTION fixture_nested_verification_handoff_preseed()
           RETURNS trigger AS $$
           BEGIN
               IF pg_trigger_depth() = 1 THEN
                   INSERT INTO verification_stage_handoffs SELECT (NEW).*;
               END IF;
               RETURN NEW;
           END;
           $$ LANGUAGE plpgsql;
           CREATE TRIGGER fixture_nested_verification_handoff_preseed
           BEFORE INSERT ON verification_stage_handoffs
           FOR EACH ROW EXECUTE FUNCTION fixture_nested_verification_handoff_preseed();"#,
    )
    .execute(db.pool())
    .await
    .expect("install nested Verification handoff preseed fixture");
    let nested_preseed = try_unready_verification_handoff_preseed(db.pool(), &template).await;
    assert_sqlstate(
        nested_preseed,
        "23514",
        "nested Verification handoff authority key before its owner close transition",
    );
    sqlx::raw_sql(
        r#"DROP TRIGGER fixture_nested_verification_handoff_preseed
               ON verification_stage_handoffs;
           DROP FUNCTION fixture_nested_verification_handoff_preseed();"#,
    )
    .execute(db.pool())
    .await
    .expect("remove nested Verification handoff preseed fixture");
    let supplied_gate_passed_at = chrono::DateTime::parse_from_rfc3339("2000-01-01T00:00:00Z")
        .expect("fixed caller-authored gate timestamp")
        .with_timezone(&chrono::Utc);
    let before_server_gate = chrono::Utc::now();
    let server_gate_passed_at = try_raw_verification_handoff_insert_with_gate_time(
        db.pool(),
        &template,
        supplied_gate_passed_at,
    )
    .await
    .expect("ready authority overwrites a caller-authored gate timestamp");
    let after_server_gate = chrono::Utc::now();
    assert_ne!(server_gate_passed_at, supplied_gate_passed_at);
    assert!(server_gate_passed_at >= before_server_gate);
    assert!(server_gate_passed_at <= after_server_gate);

    let mut exact_payload = template.payload.clone();
    refresh_verification_handoff_hashes(&template, &mut exact_payload);

    let mut extra_top_level_payload = exact_payload.clone();
    extra_top_level_payload["free_prose"] =
        serde_json::json!("caller-controlled prose is not typed Verification truth");
    let extra_top_level_sha = sha256_json(&extra_top_level_payload);
    let extra_top_level = try_raw_verification_handoff_insert(
        db.pool(),
        &template,
        &extra_top_level_payload,
        &extra_top_level_sha,
    )
    .await;
    assert!(
        extra_top_level.as_ref().is_err_and(|error| error
            .to_string()
            .contains("VERIFICATION_TYPED_HANDOFF_PAYLOAD_INVALID")),
        "an untyped top-level payload key must not enter the final seal: {extra_top_level:?}"
    );

    let mut extra_claim_wrapper_payload = exact_payload.clone();
    extra_claim_wrapper_payload["typed_claims"]
        .as_array_mut()
        .expect("typed Verification claims")[0]["free_prose"] =
        serde_json::json!("caller-controlled claim wrapper metadata");
    let extra_claim_wrapper_sha =
        refresh_verification_handoff_hashes(&template, &mut extra_claim_wrapper_payload);
    let extra_claim_wrapper = try_raw_verification_handoff_insert(
        db.pool(),
        &template,
        &extra_claim_wrapper_payload,
        &extra_claim_wrapper_sha,
    )
    .await;
    assert!(
        extra_claim_wrapper.as_ref().is_err_and(|error| error
            .to_string()
            .contains("VERIFICATION_TYPED_HANDOFF_PAYLOAD_INVALID")),
        "an untyped claim-wrapper key must not enter the final seal: {extra_claim_wrapper:?}"
    );

    let mut extra_claim_inner_payload = exact_payload.clone();
    extra_claim_inner_payload["typed_claims"]
        .as_array_mut()
        .expect("typed Verification claims")[0]["payload"]["free_prose"] =
        serde_json::json!("caller-controlled claim payload metadata");
    let extra_claim_inner_sha =
        refresh_verification_handoff_hashes(&template, &mut extra_claim_inner_payload);
    let extra_claim_inner = try_raw_verification_handoff_insert(
        db.pool(),
        &template,
        &extra_claim_inner_payload,
        &extra_claim_inner_sha,
    )
    .await;
    assert!(
        extra_claim_inner.as_ref().is_err_and(|error| error
            .to_string()
            .contains("VERIFICATION_TYPED_HANDOFF_PAYLOAD_INVALID")),
        "an untyped claim-payload key must not enter the final seal: {extra_claim_inner:?}"
    );

    let mut foreign_target_evidence_tx = db
        .pool()
        .begin()
        .await
        .expect("begin pre-seal foreign-target evidence drift");
    sqlx::query("UPDATE audit_log SET target_id=$2 WHERE id=$1")
        .bind(proof_evidence_id)
        .bind(fixture.org_b.target_id)
        .execute(&mut *foreign_target_evidence_tx)
        .await
        .expect("drift linked Attempt evidence to a foreign target before the seal");
    let foreign_target_evidence = try_raw_verification_handoff_insert_on_connection(
        &mut foreign_target_evidence_tx,
        &template,
        &exact_payload,
        &template.payload_sha256,
        None,
    )
    .await;
    assert!(
        foreign_target_evidence.as_ref().is_err_and(|error| error
            .to_string()
            .contains("VERIFICATION_TYPED_HANDOFF_EVIDENCE_MISMATCH")),
        "pre-seal evidence target drift must fail exact owner revalidation: {foreign_target_evidence:?}"
    );
    foreign_target_evidence_tx
        .rollback()
        .await
        .expect("rollback pre-seal foreign-target evidence drift");

    let mut evidence_time_drift_tx = db
        .pool()
        .begin()
        .await
        .expect("begin pre-seal FactDelta evidence time drift");
    sqlx::query(
        "UPDATE audit_log
            SET created_at=(
                SELECT created_at-INTERVAL '1 second'
                  FROM candidate_attempts WHERE id=$2
            )
          WHERE id=$1",
    )
    .bind(proof_evidence_id)
    .bind(claimed.attempt.id)
    .execute(&mut *evidence_time_drift_tx)
    .await
    .expect("drift linked FactDelta evidence outside its source Attempt interval");
    let evidence_time_drift = try_raw_verification_handoff_insert_on_connection(
        &mut evidence_time_drift_tx,
        &template,
        &exact_payload,
        &template.payload_sha256,
        None,
    )
    .await;
    assert!(
        evidence_time_drift.as_ref().is_err_and(|error| error
            .to_string()
            .contains("VERIFICATION_TYPED_HANDOFF_EVIDENCE_MISMATCH")),
        "pre-seal FactDelta evidence time drift must fail exact Attempt-window revalidation: {evidence_time_drift:?}"
    );
    evidence_time_drift_tx
        .rollback()
        .await
        .expect("rollback pre-seal FactDelta evidence time drift");

    let mut forged_identity = template.clone();
    forged_identity.id = Uuid::new_v4();
    let forged_identity_insert = try_raw_verification_handoff_insert(
        db.pool(),
        &forged_identity,
        &exact_payload,
        &template.payload_sha256,
    )
    .await;
    assert!(
        forged_identity_insert.as_ref().is_err_and(|error| error
            .to_string()
            .contains("VERIFICATION_TYPED_HANDOFF_PROJECTION_MISMATCH")),
        "raw handoff identity must equal the server UUIDv5 projection: {forged_identity_insert:?}"
    );

    let mut reordered_payload = exact_payload.clone();
    reordered_payload["typed_claims"]
        .as_array_mut()
        .expect("ordered Verification typed claims")
        .reverse();
    let reordered_payload_sha =
        refresh_verification_handoff_hashes(&template, &mut reordered_payload);
    let reordered = try_raw_verification_handoff_insert(
        db.pool(),
        &template,
        &reordered_payload,
        &reordered_payload_sha,
    )
    .await;
    assert!(
        reordered.as_ref().is_err_and(|error| error
            .to_string()
            .contains("VERIFICATION_TYPED_HANDOFF_PROJECTION_MISMATCH")),
        "reordered exact claims must not acquire a different authoritative hash: {reordered:?}"
    );

    let payload_hash_drift =
        try_raw_verification_handoff_insert(db.pool(), &template, &exact_payload, &"0".repeat(64))
            .await;
    assert!(
        payload_hash_drift.as_ref().is_err_and(|error| error
            .to_string()
            .contains("VERIFICATION_TYPED_HANDOFF_HASH_MISMATCH")),
        "raw payload hash drift must fail exact DB projection: {payload_hash_drift:?}"
    );

    let mut forged_attempt_payload = exact_payload.clone();
    let terminal_claim = forged_attempt_payload["typed_claims"]
        .as_array_mut()
        .expect("template typed claims")
        .iter_mut()
        .find(|claim| claim["kind"] == "candidate_attempt_terminal")
        .expect("terminal Attempt claim");
    terminal_claim["payload"]["candidate_id"] = serde_json::json!(Uuid::new_v4());
    terminal_claim["payload"]["disposition"] = serde_json::json!("refuted");
    terminal_claim["payload"]["finding_id"] = serde_json::Value::Null;
    terminal_claim["payload"]["finding_ref"] = serde_json::Value::Null;
    terminal_claim["payload"]["evidence_ids"] = serde_json::json!([]);
    let forged_attempt_sha =
        refresh_verification_handoff_hashes(&template, &mut forged_attempt_payload);
    let forged_attempt = try_raw_verification_handoff_insert(
        db.pool(),
        &template,
        &forged_attempt_payload,
        &forged_attempt_sha,
    )
    .await;
    assert!(
        forged_attempt.as_ref().is_err_and(|error| error
            .to_string()
            .contains("VERIFICATION_TYPED_HANDOFF_PROJECTION_MISMATCH")),
        "same-Wave evidence union cannot authorize a forged Attempt claim: {forged_attempt:?}"
    );

    let mut forged_delta_payload = exact_payload.clone();
    let delta_claim = forged_delta_payload["typed_claims"]
        .as_array_mut()
        .expect("template typed claims")
        .iter_mut()
        .find(|claim| claim["kind"] == "attack_fact_delta_proposal")
        .expect("FactDelta proposal claim");
    delta_claim["payload"]["status"] = serde_json::json!("accepted");
    delta_claim["payload"]["canonical_ref_hash"] = serde_json::json!("sha256:forged-ref");
    let forged_delta_sha =
        refresh_verification_handoff_hashes(&template, &mut forged_delta_payload);
    let forged_delta = try_raw_verification_handoff_insert(
        db.pool(),
        &template,
        &forged_delta_payload,
        &forged_delta_sha,
    )
    .await;
    assert!(
        forged_delta.as_ref().is_err_and(|error| error
            .to_string()
            .contains("VERIFICATION_TYPED_HANDOFF_PROJECTION_MISMATCH")),
        "raw FactDelta status/ref drift must fail exact DB projection: {forged_delta:?}"
    );

    let mut forged_finding_payload = exact_payload.clone();
    forged_finding_payload["canonical_fact_refs"][0]["content_sha256"] =
        serde_json::json!("d".repeat(64));
    for claim in forged_finding_payload["typed_claims"]
        .as_array_mut()
        .expect("template typed claims")
    {
        if claim["payload"]["finding_ref"].is_object() {
            claim["payload"]["finding_ref"]["content_sha256"] = serde_json::json!("d".repeat(64));
        }
    }
    let forged_finding_sha =
        refresh_verification_handoff_hashes(&template, &mut forged_finding_payload);
    let forged_finding = try_raw_verification_handoff_insert(
        db.pool(),
        &template,
        &forged_finding_payload,
        &forged_finding_sha,
    )
    .await;
    assert!(
        forged_finding.as_ref().is_err_and(|error| error
            .to_string()
            .contains("VERIFICATION_TYPED_HANDOFF_PROJECTION_MISMATCH")),
        "raw canonical Finding hash drift must fail exact DB projection: {forged_finding:?}"
    );

    // Keep the deliberately inconsistent historical Attempt/Finding projection
    // inside one transaction. Audit rows are immutable, so fixture isolation
    // must come from rollback rather than deleting lineage after the assertion.
    let mut orphan_fixture_tx = db
        .pool()
        .begin()
        .await
        .expect("begin isolated unbound Finding fixture");
    let orphan_attempt_id = insert_attempt_with_ordinal_on_connection(
        &mut orphan_fixture_tx,
        &fixture,
        candidate,
        approval_id,
        fixture.org_a,
        "running",
        1,
    )
    .await
    .expect("insert same-Wave historical verified Attempt fixture");
    sqlx::query(
        "INSERT INTO candidate_attempt_evidence(attempt_id,evidence_id,role)
         VALUES($1,$2,'proof')",
    )
    .bind(orphan_attempt_id)
    .bind(proof_evidence_id)
    .execute(&mut *orphan_fixture_tx)
    .await
    .expect("link exact proof to historical verified Attempt");
    let orphan_result_json = serde_json::json!({
        "disposition": "verified",
        "proof_evidence_ids": [proof_evidence_id]
    });
    sqlx::query(
        "UPDATE candidate_attempts
            SET status='submitted',result_json=$2,result_hash=$3,updated_at=NOW()
          WHERE id=$1 AND status='running'",
    )
    .bind(orphan_attempt_id)
    .bind(&orphan_result_json)
    .bind(format!("sha256:{}", sha256_json(&orphan_result_json)))
    .execute(&mut *orphan_fixture_tx)
    .await
    .expect("submit historical verified Attempt after freezing proof");
    sqlx::query(
        "UPDATE candidate_attempts
            SET status='verified',updated_at=NOW()
          WHERE id=$1 AND status='submitted'",
    )
    .bind(orphan_attempt_id)
    .execute(&mut *orphan_fixture_tx)
    .await
    .expect("terminalize historical submitted Attempt");
    let orphan_finding_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO findings(
               id,title,sev,cvss,url,target,description,steps,remediation,
               tags,tool,template,refs,evidence,status,project_path,source,target_id
           )
           SELECT $1,title,sev,cvss,url,target,description,steps,remediation,
                  tags,tool,template,refs,jsonb_build_array($2::BIGINT),status,
                  project_path,source,target_id
             FROM findings WHERE id=$3"#,
    )
    .bind(orphan_finding_id)
    .bind(proof_evidence_id)
    .bind(terminal_finding_id)
    .execute(&mut *orphan_fixture_tx)
    .await
    .expect("insert historical verified Finding fixture");
    sqlx::query(
        r#"INSERT INTO finding_lineage(
               id,finding_id,candidate_attempt_id,candidate_id,operation_id,
               scope_snapshot_id,wave_run_id,wave_unit_id,organization_id,target_live_id,
               target_type_at_time,target_value_at_time,target_identity_hash,candidate_plan_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'url',
                    'https://shared.example.test/login','sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963',
                    'sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5')"#,
    )
    .bind(Uuid::new_v4())
    .bind(orphan_finding_id)
    .bind(orphan_attempt_id)
    .bind(candidate.candidate_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.wave_run_id)
    .bind(fixture.org_a.wave_unit_id)
    .bind(fixture.org_a.organization_id)
    .bind(fixture.org_a.target_id)
    .execute(&mut *orphan_fixture_tx)
    .await
    .expect("insert same-Wave historical Finding lineage fixture");
    let orphan_finding_ref: serde_json::Value = sqlx::query_scalar(
        r#"SELECT jsonb_build_object(
                   'key',jsonb_build_object('kind','finding','finding_id',finding.id),
                   'organization_id',lineage.organization_id,
                   'source_table','findings',
                   'source_row_version',finding.row_version,
                   'observed_at_unix_micros',
                       (EXTRACT(EPOCH FROM finding.updated_at)*1000000)::BIGINT,
                   'content_sha256',verification_sha256_jsonb(
                       (to_jsonb(finding)-'target_id') || jsonb_build_object(
                           'finding_lineage_id',lineage.id,
                           'finding_lineage_row_version',lineage.row_version,
                           'canonical_target_snapshot',lineage.canonical_target_snapshot
                       )
                   ),
                   'evidence_ids',to_jsonb(ARRAY[$2::BIGINT])
               )
              FROM finding_lineage AS lineage
              JOIN findings AS finding ON finding.id=lineage.finding_id
             WHERE finding.id=$1"#,
    )
    .bind(orphan_finding_id)
    .bind(proof_evidence_id)
    .fetch_one(&mut *orphan_fixture_tx)
    .await
    .expect("project historically valid but non-terminal Finding ref");
    let mut forged_unbound_finding_payload = exact_payload.clone();
    forged_unbound_finding_payload["canonical_fact_refs"][0] = orphan_finding_ref;
    let forged_unbound_finding_sha =
        refresh_verification_handoff_hashes(&template, &mut forged_unbound_finding_payload);
    let forged_unbound_finding = try_raw_verification_handoff_insert_on_connection(
        &mut orphan_fixture_tx,
        &template,
        &forged_unbound_finding_payload,
        &forged_unbound_finding_sha,
        None,
    )
    .await;
    assert!(
        forged_unbound_finding.as_ref().is_err_and(|error| error
            .to_string()
            .contains("VERIFICATION_TYPED_HANDOFF_PROJECTION_MISMATCH")),
        "a valid same-Wave Finding that is not the terminal approved Candidate Finding must be rejected: {forged_unbound_finding:?}"
    );
    orphan_fixture_tx
        .rollback()
        .await
        .expect("rollback isolated unbound Finding fixture");

    let mut verification_close_tx = db
        .pool()
        .begin()
        .await
        .expect("begin exact VerificationUnit close");
    let closed = golish_db::repo::verification_truth::close_verification_unit(
        &mut verification_close_tx,
        close_command,
    )
    .await
    .expect("exact terminal truth must close its VerificationUnit");
    verification_close_tx
        .commit()
        .await
        .expect("commit exact VerificationUnit close");
    assert!(!closed.replayed);
    assert!(closed.verification_closed);
    assert_eq!(closed.consolidation_status, "ready");
    assert_eq!(closed.verification_stage_run_unit_id, stage_run_unit_id);
    assert_eq!(closed.verification_stage_run_unit_status, "passed");
    assert_eq!(
        closed.verification_primary_worker_run_id,
        primary_worker_run_id
    );
    assert_eq!(closed.verification_primary_worker_status, "passed");
    let runtime_close_state: (
        String,
        Option<chrono::DateTime<chrono::Utc>>,
        String,
        Option<chrono::DateTime<chrono::Utc>>,
    ) = sqlx::query_as(
        "SELECT unit.status,unit.terminal_at,worker.status,worker.terminal_at
             FROM stage_run_units unit
             JOIN stage_worker_runs worker ON worker.stage_run_unit_id=unit.id
              AND worker.work_item_kind='organization'
              AND worker.work_item_key='verification'
              AND worker.worker_generation=unit.generation
             WHERE unit.id=$1 AND worker.id=$2",
    )
    .bind(stage_run_unit_id)
    .bind(primary_worker_run_id)
    .fetch_one(db.pool())
    .await
    .expect("load compound Verification runtime close state");
    assert_eq!(runtime_close_state.0, "passed");
    assert!(runtime_close_state.1.is_some());
    assert_eq!(runtime_close_state.2, "passed");
    assert!(runtime_close_state.3.is_some());
    let inherited_verification =
        golish_db::repo::stage_handoffs::list_latest_final_sealed_for_sources(
            db.pool(),
            fixture.operation_id,
            fixture.org_a.organization_id,
            &["verification".to_string()],
        )
        .await
        .expect("load server-authored Verification handoff");
    assert_eq!(inherited_verification.len(), 1);
    assert_eq!(inherited_verification[0].from_stage_kind, "verification");
    assert_eq!(
        inherited_verification[0].authority_kind,
        "verification_wave_close"
    );
    assert_eq!(inherited_verification[0].deliverable_submission_id, None);
    assert_eq!(inherited_verification[0].id, closed.verification_handoff_id);
    assert_eq!(
        inherited_verification[0].payload_sha256,
        closed.verification_handoff_payload_sha256
    );
    let inherited_claim_kinds = inherited_verification[0].payload["typed_claims"]
        .as_array()
        .expect("Verification typed handoff claims")
        .iter()
        .filter_map(|claim| claim["kind"].as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(inherited_claim_kinds.contains("candidate_attempt_terminal"));
    assert!(inherited_claim_kinds.contains("verified_candidate_attempt"));
    assert!(inherited_claim_kinds.contains("attack_fact_delta_proposal"));
    let fact_delta_proposal = inherited_verification[0].payload["typed_claims"]
        .as_array()
        .expect("Verification typed handoff claims")
        .iter()
        .find(|claim| claim["kind"] == "attack_fact_delta_proposal")
        .expect("Verification FactDelta proposal claim");
    assert_eq!(fact_delta_proposal["payload"]["status"], "proposed");
    assert_eq!(
        fact_delta_proposal["payload"]["source_attempt_id"],
        serde_json::json!(claimed.attempt.id)
    );
    assert_eq!(
        fact_delta_proposal["payload"]["evidence_ids"],
        serde_json::json!([proof_evidence_id])
    );
    let fact_delta_membership_delete = sqlx::query(
        "DELETE FROM attack_fact_delta_evidence
         WHERE fact_delta_id=(
             SELECT id FROM attack_fact_deltas WHERE source_attempt_id=$1
         ) AND evidence_id=$2 AND role='fact_delta'",
    )
    .bind(claimed.attempt.id)
    .bind(proof_evidence_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        fact_delta_membership_delete.map(|_| ()),
        "23514",
        "Verification handoff FactDelta evidence membership deletion",
    );
    let canonical_findings = inherited_verification[0].payload["canonical_fact_refs"]
        .as_array()
        .expect("Verification canonical Finding refs");
    assert_eq!(canonical_findings.len(), 1);
    assert_eq!(canonical_findings[0]["key"]["kind"], "finding");
    assert_eq!(
        canonical_findings[0]["key"]["finding_id"],
        serde_json::json!(terminal_finding_id)
    );
    assert_eq!(canonical_findings[0]["source_table"], "findings");
    assert!(canonical_findings[0]["source_row_version"]
        .as_i64()
        .is_some_and(|version| version >= 0));
    assert_eq!(
        canonical_findings[0]["evidence_ids"],
        serde_json::json!([proof_evidence_id])
    );
    assert_eq!(
        canonical_findings[0]["content_sha256"]
            .as_str()
            .map(str::len),
        Some(64)
    );
    let pass_watermark: serde_json::Value =
        sqlx::query_scalar("SELECT pass_watermark FROM stage_run_units WHERE id=$1")
            .bind(stage_run_unit_id)
            .fetch_one(db.pool())
            .await
            .expect("load Verification typed-handoff pass watermark");
    assert_eq!(
        pass_watermark["typed_handoff_id"],
        serde_json::json!(closed.verification_handoff_id.to_string())
    );
    assert_eq!(
        pass_watermark["handoff_payload_sha256"],
        closed.verification_handoff_payload_sha256
    );
    let immutable_update =
        sqlx::query("UPDATE verification_stage_handoffs SET payload=payload WHERE id=$1")
            .bind(closed.verification_handoff_id)
            .execute(db.pool())
            .await
            .expect_err("Verification typed handoff is immutable");
    assert!(
        immutable_update
            .to_string()
            .contains("VERIFICATION_TYPED_HANDOFF_IMMUTABLE"),
        "unexpected typed handoff immutability error: {immutable_update}"
    );
    let mut close_replay_tx = db
        .pool()
        .begin()
        .await
        .expect("begin VerificationUnit close replay");
    let close_replay = golish_db::repo::verification_truth::close_verification_unit(
        &mut close_replay_tx,
        golish_db::repo::verification_truth::CloseVerificationUnit {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_a.wave_unit_id,
            organization_id: fixture.org_a.organization_id,
            verification_stage_execution_id: stage_execution_id,
            verification_stage_run_unit_id: stage_run_unit_id,
        },
    )
    .await
    .expect("VerificationUnit close response-loss replay is idempotent");
    close_replay_tx
        .commit()
        .await
        .expect("commit VerificationUnit close replay");
    assert!(close_replay.replayed);
    assert_eq!(close_replay.row_version, closed.row_version);
    assert_eq!(
        close_replay.verification_stage_run_unit_id,
        stage_run_unit_id
    );
    assert_eq!(close_replay.verification_stage_run_unit_status, "passed");
    assert_eq!(
        close_replay.verification_primary_worker_run_id,
        primary_worker_run_id
    );
    assert_eq!(close_replay.verification_primary_worker_status, "passed");
    sqlx::query("UPDATE stage_worker_runs SET status='queued',terminal_at=NULL WHERE id=$1")
        .bind(primary_worker_run_id)
        .execute(db.pool())
        .await
        .expect("simulate an inconsistent partial response-loss close");
    let mut inconsistent_replay_tx = db
        .pool()
        .begin()
        .await
        .expect("begin inconsistent close replay");
    let inconsistent_replay_error = golish_db::repo::verification_truth::close_verification_unit(
        &mut inconsistent_replay_tx,
        golish_db::repo::verification_truth::CloseVerificationUnit {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_a.wave_unit_id,
            organization_id: fixture.org_a.organization_id,
            verification_stage_execution_id: stage_execution_id,
            verification_stage_run_unit_id: stage_run_unit_id,
        },
    )
    .await
    .expect_err("partial response-loss state must fail closed");
    assert!(
        inconsistent_replay_error
            .to_string()
            .contains("VerificationUnit is not close-ready"),
        "unexpected inconsistent replay error: {inconsistent_replay_error}"
    );
    inconsistent_replay_tx
        .rollback()
        .await
        .expect("rollback inconsistent close replay");
    sqlx::query("UPDATE stage_worker_runs SET status='passed',terminal_at=NOW() WHERE id=$1")
        .bind(primary_worker_run_id)
        .execute(db.pool())
        .await
        .expect("restore compound Verification runtime close state");
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
        .bind(terminal_finding_id)
        .fetch_one(db.pool())
        .await
        .expect("load terminalizer Finding CVSS");
    assert_eq!(persisted_cvss, Some(8.1));

    let empty_array = serde_json::json!([]);
    let tamper = golish_db::repo::findings::FindingUpsert {
        id: terminal_finding_id,
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
            candidate_plan_hash:
                "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5"
                    .to_string(),
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
            candidate_plan_hash:
                "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5"
                    .to_string(),
            expected_result_hash: submitted.attempt.result_hash.clone().expect("result hash"),
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
    assert_eq!(terminal.scope_snapshot_id, fixture.scope_snapshot_id);
    assert_eq!(terminal.wave_run_id, fixture.wave_run_id);
    assert_eq!(terminal.wave_unit_id, fixture.org_a.wave_unit_id);
    assert_eq!(terminal.organization_id, fixture.org_a.organization_id);
    assert_eq!(terminal.candidate_id, candidate.candidate_id);
    assert_eq!(terminal.attempt_id, claimed.attempt.id);
    assert_eq!(terminal.status, "blocked");
    assert_eq!(terminal.disposition, "blocked");
    assert_eq!(terminal.finding_id, None);
    assert_eq!(terminal.evidence_count, 0);
    assert_eq!(terminal.fact_delta_count, 0);
    assert!(!terminal.replayed);
    let mut replay_tx = db.pool().begin().await.expect("begin terminal replay");
    let replay = terminalize_candidate_attempt(
        &mut replay_tx,
        TerminalizeCandidateAttempt {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_a.wave_unit_id,
            organization_id: fixture.org_a.organization_id,
            candidate_id: candidate.candidate_id,
            approval_id,
            attempt_id: claimed.attempt.id,
            candidate_plan_hash:
                "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5"
                    .to_string(),
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
    .expect("replay reason-only blocked Attempt");
    replay_tx.commit().await.expect("commit terminal replay");
    assert!(replay.replayed);
    assert_eq!(replay.scope_snapshot_id, terminal.scope_snapshot_id);
    assert_eq!(replay.wave_run_id, terminal.wave_run_id);
    assert_eq!(replay.wave_unit_id, terminal.wave_unit_id);
    assert_eq!(replay.organization_id, terminal.organization_id);
    assert_eq!(replay.candidate_id, terminal.candidate_id);
    assert_eq!(replay.attempt_id, terminal.attempt_id);
    assert_eq!(replay.status, terminal.status);
    assert_eq!(replay.disposition, terminal.disposition);
    assert_eq!(replay.finding_id, terminal.finding_id);
    assert_eq!(replay.evidence_count, terminal.evidence_count);
    assert_eq!(replay.fact_delta_count, terminal.fact_delta_count);
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
               'https://shared.example.test/login','sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963',
               'sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5',0,'running',$10
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

    sqlx::query(
        "UPDATE attack_candidate_approvals
            SET status='revoked',row_version=row_version+1 WHERE id=$1",
    )
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
               'https://shared.example.test/login','sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963',
               'sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5',1,'queued'
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
    let orphan_execution_plan = serde_json::json!({"schema_version": "candidate-plan-v1"});
    let orphan_plan_hash = canonical_execution_plan_hash(&orphan_execution_plan)
        .expect("hash orphan Candidate execution plan");
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
               'https://shared.example.test/login','sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963',$13,
               $14,'exploit'
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
    .bind(orphan_execution_plan)
    .bind(orphan_plan_hash)
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
                   'https://shared.example.test/login','sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963',
                   'sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5',$10,'queued'
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
               'https://shared.example.test/login','sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963',
               'sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5'
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
        "UPDATE candidate_attempts SET status='submitted',
             result_json=$2,result_hash='sha256:verified'
         WHERE id=$1 AND status='running'",
    )
    .bind(verified_attempt)
    .bind(serde_json::json!({"disposition": "verified"}))
    .execute(db.pool())
    .await
    .expect("submit exact Attempt for Finding authority test");
    sqlx::query(
        "UPDATE candidate_attempts SET status='verified'
         WHERE id=$1 AND status='submitted'",
    )
    .bind(verified_attempt)
    .execute(db.pool())
    .await
    .expect("make exact submitted Attempt verified for Finding authority test");
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
               'https://shared.example.test/login','sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963',
               'sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5'
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

#[tokio::test]
#[serial]
async fn attack_wave_entry_is_exactly_one_of_handoff_or_fact_delta_consolidation() {
    let (mut db, _data_dir) = migrated_db("fact_delta_wave_entry_shape").await;
    let typed_tables: Vec<(String, bool)> = sqlx::query_as(
        r#"SELECT name, to_regclass('public.' || name) IS NOT NULL
             FROM unnest(ARRAY[
                 'attack_fact_delta_decisions',
                 'attack_wave_consolidations',
                 'attack_wave_consolidation_members'
             ]::TEXT[]) AS name
            ORDER BY name"#,
    )
    .fetch_all(db.pool())
    .await
    .expect("inspect typed FactDelta consolidation tables");
    assert_eq!(
        typed_tables,
        vec![
            ("attack_fact_delta_decisions".to_string(), true),
            ("attack_wave_consolidation_members".to_string(), true),
            ("attack_wave_consolidations".to_string(), true),
        ],
        "00012 must install immutable FactDelta decisions and typed Wave provenance"
    );

    let entry_columns: Vec<(String, String)> = sqlx::query_as(
        r#"SELECT column_name,is_nullable
             FROM information_schema.columns
            WHERE table_schema='public' AND table_name='attack_wave_units'
              AND column_name IN (
                  'entry_stage_execution_id',
                  'entry_stage_run_unit_id',
                  'entry_deliverable_submission_id',
                  'entry_stage_kind',
                  'entry_consolidation_id'
              )
            ORDER BY column_name"#,
    )
    .fetch_all(db.pool())
    .await
    .expect("inspect attack Wave entry union");
    assert_eq!(
        entry_columns,
        vec![
            ("entry_consolidation_id".to_string(), "YES".to_string()),
            (
                "entry_deliverable_submission_id".to_string(),
                "YES".to_string(),
            ),
            ("entry_stage_execution_id".to_string(), "YES".to_string()),
            ("entry_stage_kind".to_string(), "YES".to_string()),
            ("entry_stage_run_unit_id".to_string(), "YES".to_string()),
        ],
        "WaveUnit entry must be a nullable-column XOR between initial handoff and immutable consolidation"
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn follow_on_wave_unit_rejects_reused_vuln_triage_handoff() {
    let (mut db, _data_dir) = migrated_db("fact_delta_follow_on_entry").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let mut read_tx = db.pool().begin().await.expect("begin initial entry read");
    let initial = attack_waves::lock_wave_unit(
        &mut read_tx,
        fixture.operation_id,
        fixture.scope_snapshot_id,
        fixture.wave_run_id,
        fixture.org_a.wave_unit_id,
        fixture.org_a.organization_id,
    )
    .await
    .expect("typed repo must decode an initial handoff WaveUnit");
    assert!(matches!(
        initial.entry,
        attack_waves::AttackWaveEntry::VulnTriageHandoff { .. }
    ));
    read_tx.rollback().await.expect("finish initial entry read");

    let reused_handoff =
        sqlx::query("UPDATE attack_wave_units SET entry_consolidation_id=$2 WHERE id=$1")
            .bind(fixture.org_a.wave_unit_id)
            .bind(Uuid::new_v4())
            .execute(db.pool())
            .await;
    assert_sqlstate(
        reused_handoff.map(|_| ()),
        "P0001",
        "initial WaveUnit cannot also claim a follow-on consolidation",
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn fact_delta_acceptance_rejects_missing_or_forged_typed_decision() {
    let (mut db, _data_dir) = migrated_db("fact_delta_decision_authority").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let candidate = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    let approval_id = insert_approval(db.pool(), &fixture, candidate, fixture.org_a)
        .await
        .expect("insert exact Candidate approval");
    let attempt_id = insert_attempt(
        db.pool(),
        &fixture,
        candidate,
        approval_id,
        fixture.org_a,
        "blocked",
    )
    .await
    .expect("insert terminal source Attempt");
    sqlx::query("UPDATE attack_wave_runs SET status='verification',updated_at=NOW() WHERE id=$1")
        .bind(fixture.wave_run_id)
        .execute(db.pool())
        .await
        .expect("advance exact FactDelta source Wave to verification");
    let fact_delta_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO attack_fact_deltas (
               id,source_attempt_id,candidate_id,operation_id,scope_snapshot_id,
               wave_run_id,wave_unit_id,organization_id,target_live_id,
               target_type_at_time,target_value_at_time,target_identity_hash,
               candidate_plan_hash,canonical_ref_kind,canonical_ref_id,
               canonical_ref_version,canonical_ref_hash,delta_kind,dedupe_hash
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,$9,'url',
               'https://shared.example.test/login','sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963',
               'sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5','attack_candidate_work_item',$10,1,
               'sha256:canonical-ref','new_surface','sha256:decision-delta'
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
    .bind(candidate.work_item_id)
    .execute(db.pool())
    .await
    .expect("insert proposed FactDelta");

    let mut unauthorized = db
        .pool()
        .begin()
        .await
        .expect("begin unauthorized acceptance");
    sqlx::query(
        "UPDATE attack_fact_deltas
            SET status='accepted',accepted_at=NOW(),updated_at=NOW() WHERE id=$1",
    )
    .bind(fact_delta_id)
    .execute(&mut *unauthorized)
    .await
    .expect("stage unauthorized accepted status for deferred authority check");
    assert_sqlstate(
        unauthorized.commit().await,
        "P0001",
        "FactDelta accepted status without immutable decision",
    );

    let mut forged = db
        .pool()
        .begin()
        .await
        .expect("begin forged typed acceptance");
    sqlx::query(
        r#"INSERT INTO attack_fact_delta_decisions (
               fact_delta_id,source_attempt_id,candidate_id,operation_id,
               scope_snapshot_id,source_wave_run_id,source_wave_unit_id,
               organization_id,disposition,reason_code,canonical_ref_kind,
               canonical_ref_id,canonical_ref_version,proposed_ref_hash,
               resolved_ref_version,resolved_ref_hash,evidence_set_hash,decision_hash
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,'accepted','accepted',
               'attack_candidate_work_item',$9,1,'sha256:canonical-ref',1,
               'sha256:canonical-ref','sha256:evidence-set','sha256:decision'
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
    .bind(candidate.work_item_id)
    .execute(&mut *forged)
    .await
    .expect("stage forged accepted decision for deferred authority check");
    sqlx::query(
        "UPDATE attack_fact_deltas
            SET status='accepted',accepted_at=NOW(),updated_at=NOW() WHERE id=$1",
    )
    .bind(fact_delta_id)
    .execute(&mut *forged)
    .await
    .expect("stage accepted status with forged decision material");
    assert_sqlstate(
        forged.commit().await,
        "P0001",
        "FactDelta accepted status with caller-authored decision hashes",
    );
    let rollback_truth: (String, i64) = sqlx::query_as(
        r#"SELECT delta.status,
                  (SELECT COUNT(*) FROM attack_fact_delta_decisions AS decision
                    WHERE decision.fact_delta_id=delta.id)
             FROM attack_fact_deltas AS delta
            WHERE delta.id=$1"#,
    )
    .bind(fact_delta_id)
    .fetch_one(db.pool())
    .await
    .expect("inspect forged FactDelta acceptance rollback");
    assert_eq!(rollback_truth, ("proposed".to_string(), 0));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn all_org_units_must_be_terminal_before_global_cursor_advances() {
    let (mut db, _data_dir) = migrated_db("fact_delta_all_org_barrier").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    sqlx::query("UPDATE attack_wave_runs SET status='verification',updated_at=NOW() WHERE id=$1")
        .bind(fixture.wave_run_id)
        .execute(db.pool())
        .await
        .expect("advance source Wave to verification");
    sqlx::query(
        "UPDATE attack_wave_units
            SET review_closed=TRUE,status='verification',updated_at=NOW()
          WHERE wave_run_id=$1",
    )
    .bind(fixture.wave_run_id)
    .execute(db.pool())
    .await
    .expect("close Candidate review for every frozen org");
    close_fixture_verification_units_through_typed_handoff(
        db.pool(),
        &fixture,
        fixture.wave_run_id,
        &[fixture.org_a.wave_unit_id],
    )
    .await;

    let mut tx = db.pool().begin().await.expect("begin global consolidation");
    let error = attack_wave_consolidations::consolidate_attack_wave(
        &mut tx,
        attack_wave_consolidations::ConsolidateAttackWave {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            source_wave_run_id: fixture.wave_run_id,
        },
    )
    .await
    .expect_err("one non-terminal sibling org must hold the global cursor");
    assert!(
        error.to_string().contains("attack_wave_not_ready"),
        "unexpected all-org barrier error: {error}"
    );
    tx.rollback()
        .await
        .expect("rollback blocked consolidation transaction");

    let wave_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM attack_wave_runs WHERE operation_id=$1 AND scope_snapshot_id=$2",
    )
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .fetch_one(db.pool())
    .await
    .expect("count Waves after blocked consolidation");
    let consolidation_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM attack_wave_consolidations WHERE source_wave_run_id=$1",
    )
    .bind(fixture.wave_run_id)
    .fetch_one(db.pool())
    .await
    .expect("count cursor decisions after blocked consolidation");
    let source_status: String =
        sqlx::query_scalar("SELECT status FROM attack_wave_runs WHERE id=$1")
            .bind(fixture.wave_run_id)
            .fetch_one(db.pool())
            .await
            .expect("read source Wave after blocked consolidation");
    assert_eq!(
        wave_count, 1,
        "barrier failure must not create an orphan Wave"
    );
    assert_eq!(
        consolidation_count, 0,
        "barrier failure must not advance cursor"
    );
    assert_eq!(
        source_status, "verification",
        "source Wave must remain resumable"
    );
    db.stop().await;
}

#[derive(Clone, Copy)]
struct RootWaveFixture {
    organization_id: Uuid,
    wave_unit_id: Uuid,
}

async fn add_root_source_wave_unit(pool: &PgPool, fixture: &AttackFixture) -> RootWaveFixture {
    let organization_id: Uuid = sqlx::query_scalar(
        "SELECT root_organization_id FROM operation_org_scope_snapshots
          WHERE id=$1 AND operation_id=$2",
    )
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.operation_id)
    .fetch_one(pool)
    .await
    .expect("load frozen root organization");
    let target_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO targets (
               id,name,target_type,value,scope,project_path,organization_id
           ) VALUES (
               $1,'Root app','url','https://root.example.test/','in',
               '/tmp/attack-v2',$2
           )"#,
    )
    .bind(target_id)
    .bind(organization_id)
    .execute(pool)
    .await
    .expect("insert root target for exact initial handoff");
    let entry_stage_run_unit_id = Uuid::new_v4();
    let entry_worker_run_id = Uuid::new_v4();
    let entry_lease_token = Uuid::new_v4();
    let entry_submission_id = Uuid::new_v4();
    insert_final_passed_unit(
        pool,
        fixture,
        organization_id,
        fixture.entry_stage_execution_id,
        entry_stage_run_unit_id,
        entry_worker_run_id,
        entry_lease_token,
        entry_submission_id,
        "vuln_triage",
        "formulaic_scanner",
        0,
        0,
        true,
    )
    .await;
    let wave_unit_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO attack_wave_units (
               id,wave_run_id,operation_id,scope_snapshot_id,organization_id,
               entry_stage_execution_id,entry_stage_run_unit_id,
               entry_deliverable_submission_id,entry_stage_kind,ordinal,status
           ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'vuln_triage',0,'open')"#,
    )
    .bind(wave_unit_id)
    .bind(fixture.wave_run_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(organization_id)
    .bind(fixture.entry_stage_execution_id)
    .bind(entry_stage_run_unit_id)
    .bind(entry_submission_id)
    .execute(pool)
    .await
    .expect("insert root source WaveUnit");
    RootWaveFixture {
        organization_id,
        wave_unit_id,
    }
}

async fn resolve_work_item_ref(
    pool: &PgPool,
    fixture: &AttackFixture,
    organization_id: Uuid,
    work_item_id: Uuid,
) -> canonical_fact_refs::CanonicalFactRef {
    let mut tx = pool.begin().await.expect("begin canonical ref resolution");
    let resolved = canonical_fact_refs::resolve_for_handoff(
        &mut tx,
        fixture.operation_id,
        organization_id,
        "/tmp/attack-v2",
        chrono::Utc::now(),
        &[canonical_fact_refs::CanonicalFactKey::AttackCandidateWorkItem { work_item_id }],
    )
    .await
    .expect("resolve frozen Candidate work item")
    .pop()
    .expect("one canonical work-item ref");
    tx.rollback()
        .await
        .expect("finish canonical ref resolution");
    resolved
}

async fn insert_old_api_endpoint_ref(
    pool: &PgPool,
    fixture: &AttackFixture,
    org: OrgFixture,
) -> canonical_fact_refs::CanonicalFactRef {
    let api_endpoint_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO api_endpoints (
               id,target_id,project_path,url,method,path,source,discovered_at,updated_at
           ) VALUES (
               $1,$2,'/tmp/attack-v2','https://shared.example.test/api/old',
               'GET','/api/old','crawler',NOW() - INTERVAL '2 hours',
               NOW() - INTERVAL '2 hours'
           )"#,
    )
    .bind(api_endpoint_id)
    .bind(org.target_id)
    .execute(pool)
    .await
    .expect("insert canonical API endpoint observed before the Attempt");

    let mut tx = pool
        .begin()
        .await
        .expect("begin old canonical ref resolution");
    let resolved = canonical_fact_refs::resolve_for_handoff(
        &mut tx,
        fixture.operation_id,
        org.organization_id,
        "/tmp/attack-v2",
        chrono::Utc::now() - chrono::Duration::hours(3),
        &[canonical_fact_refs::CanonicalFactKey::ApiEndpoint { api_endpoint_id }],
    )
    .await
    .expect("resolve old canonical API endpoint")
    .pop()
    .expect("one canonical API endpoint ref");
    tx.rollback()
        .await
        .expect("finish old canonical ref resolution");
    resolved
}

async fn insert_current_api_endpoint_ref(
    pool: &PgPool,
    fixture: &AttackFixture,
    org: OrgFixture,
) -> canonical_fact_refs::CanonicalFactRef {
    let api_endpoint_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO api_endpoints (
               id,target_id,project_path,url,method,path,source,discovered_at,updated_at
           ) VALUES (
               $1,$2,'/tmp/attack-v2','https://shared.example.test/api/current',
               'GET','/api/current','candidate_verifier',NOW(),NOW()
           )"#,
    )
    .bind(api_endpoint_id)
    .bind(org.target_id)
    .execute(pool)
    .await
    .expect("insert canonical API endpoint observed during the Attempt");

    let mut tx = pool
        .begin()
        .await
        .expect("begin current canonical ref resolution");
    let resolved = canonical_fact_refs::resolve_for_handoff(
        &mut tx,
        fixture.operation_id,
        org.organization_id,
        "/tmp/attack-v2",
        chrono::Utc::now() - chrono::Duration::hours(1),
        &[canonical_fact_refs::CanonicalFactKey::ApiEndpoint { api_endpoint_id }],
    )
    .await
    .expect("resolve current canonical API endpoint")
    .pop()
    .expect("one current canonical API endpoint ref");
    tx.rollback()
        .await
        .expect("finish current canonical ref resolution");
    resolved
}

async fn propose_test_delta(
    pool: &PgPool,
    fixture: &AttackFixture,
    candidate: CandidateFixture,
    attempt_id: Uuid,
    evidence_id: i64,
    canonical_ref: (&str, Uuid, String),
    delta_kind: &str,
) -> golish_db::repo::attack_fact_deltas::AttackFactDeltaRow {
    let dedupe_hash = golish_db::repo::attack_fact_deltas::semantic_dedupe_hash(
        "sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963",
        canonical_ref.0,
        canonical_ref.1,
        1,
        &canonical_ref.2,
        delta_kind,
    )
    .expect("hash test FactDelta semantic identity");
    let mut tx = pool.begin().await.expect("begin FactDelta proposal");
    let row = golish_db::repo::attack_fact_deltas::propose_fact_delta(
        &mut tx,
        golish_db::repo::attack_fact_deltas::ProposeAttackFactDelta {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_a.wave_unit_id,
            organization_id: fixture.org_a.organization_id,
            source_attempt_id: attempt_id,
            candidate_id: candidate.candidate_id,
            candidate_plan_hash:
                "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5"
                    .to_string(),
            canonical_ref_kind: canonical_ref.0.to_string(),
            canonical_ref_id: canonical_ref.1,
            canonical_ref_version: 1,
            canonical_ref_hash: canonical_ref.2,
            delta_kind: delta_kind.to_string(),
            dedupe_hash,
            evidence_ids: vec![evidence_id],
        },
    )
    .await
    .expect("propose terminal Attempt FactDelta");
    tx.commit().await.expect("commit FactDelta proposal");
    row
}

#[derive(Clone, Copy)]
enum FollowOnEvidenceFixture {
    Generic,
    RecognizedUnsupported,
}

struct PreparedFollowOnFixture {
    delta: golish_db::repo::attack_fact_deltas::AttackFactDeltaRow,
    attempt_id: Uuid,
    evidence_id: i64,
    canonical_ref_id: Uuid,
}

async fn prepare_follow_on_route_fixture(
    pool: &PgPool,
    fixture: &AttackFixture,
    evidence_fixture: FollowOnEvidenceFixture,
) -> PreparedFollowOnFixture {
    let candidate = seed_candidate(pool, fixture, fixture.org_a).await;
    sqlx::query(
        "UPDATE attack_wave_units
            SET manifest_hash='sha256:follow-on-route-manifest',manifest_count=1,
                manifest_frozen_at=NOW(),updated_at=NOW()
          WHERE id=$1",
    )
    .bind(fixture.org_a.wave_unit_id)
    .execute(pool)
    .await
    .expect("freeze follow-on route Candidate manifest");
    let approval_id = insert_approval(pool, fixture, candidate, fixture.org_a)
        .await
        .expect("insert follow-on route Candidate approval");
    sqlx::query("UPDATE attack_candidates SET disposition='approved' WHERE candidate_id=$1")
        .bind(candidate.candidate_id)
        .execute(pool)
        .await
        .expect("mark follow-on route Candidate approved");
    sqlx::query(
        "UPDATE attack_wave_units SET review_closed=TRUE,status='verification' WHERE id=$1",
    )
    .bind(fixture.org_a.wave_unit_id)
    .execute(pool)
    .await
    .expect("make follow-on route Candidate claimable");
    let (stage_execution_id, stage_run_unit_id) =
        seed_verification_unit(pool, fixture, fixture.org_a).await;
    let claimed = claim_next_candidate_attempt(
        pool,
        CandidateClaimQuery {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_a.wave_unit_id,
            organization_id: fixture.org_a.organization_id,
            verification_stage_execution_id: stage_execution_id,
            verification_stage_run_unit_id: stage_run_unit_id,
            lease_owner: "follow-on-route-test".to_string(),
            lease_seconds: 120,
        },
    )
    .await
    .expect("claim follow-on route Candidate")
    .expect("follow-on route Candidate is available");
    let attempt_id = claimed.attempt.id;
    let canonical = insert_current_api_endpoint_ref(pool, fixture, fixture.org_a).await;
    let canonical_ref_id = match canonical.key {
        canonical_fact_refs::CanonicalFactKey::ApiEndpoint { api_endpoint_id } => api_endpoint_id,
        _ => panic!("expected current API endpoint canonical key"),
    };
    let evidence_id = insert_audit(
        pool,
        fixture.operation_id,
        fixture.org_a.organization_id,
        fixture.org_a.target_id,
        "evidence",
    )
    .await;
    match evidence_fixture {
        FollowOnEvidenceFixture::Generic => {
            sqlx::query(
                r#"UPDATE audit_log
                      SET tool_name='query_target_data',evidence_outcome='found',detail=$2
                    WHERE id=$1"#,
            )
            .bind(evidence_id)
            .bind(serde_json::json!({
                "organization_id": fixture.org_a.organization_id,
                "kind": "target.snapshot_v1",
            }))
            .execute(pool)
            .await
            .expect("mark generic follow-on evidence");
        }
        FollowOnEvidenceFixture::RecognizedUnsupported => {
            let raw_output = serde_json::json!({
                "schema": "verification.future_adapter_v1",
                "evidence_role": "proof",
                "result": {},
            })
            .to_string();
            sqlx::query(
                r#"UPDATE audit_log
                      SET tool_name='verify_execute_candidate_action',
                          evidence_technique='WSTG-INFO',evidence_outcome='found',detail=$2
                    WHERE id=$1"#,
            )
            .bind(evidence_id)
            .bind(serde_json::json!({
                "organization_id": fixture.org_a.organization_id,
                "kind": "verification.future_adapter_v1",
                "raw_output": raw_output,
            }))
            .execute(pool)
            .await
            .expect("mark recognized unsupported follow-on evidence");
        }
    }
    sqlx::query(
        "INSERT INTO candidate_attempt_actions(
             attempt_id,action_ordinal,capability_id,action_kind,canonical_args,status,
             outcome,outcome_hash,started_at,completed_at)
         VALUES($1,0,'verify.sql_injection','bounded_sql_injection_probe',
                '{\"target\":\"https://shared.example.test/login\"}'::jsonb,
                'completed','{}'::jsonb,'sha256:follow-on-route-action',NOW(),NOW())",
    )
    .bind(attempt_id)
    .execute(pool)
    .await
    .expect("finish follow-on route Candidate action");
    let result_json = serde_json::json!({
        "disposition": "blocked",
        "blocker_reason_code": "follow_on_only",
        "fact_deltas": [{
            "fact_kind": "new_surface",
            "canonical_ref_kind": "api_endpoint",
            "canonical_ref_id": canonical_ref_id,
            "canonical_ref_version": 1,
            "canonical_ref_hash": canonical.content_sha256.clone(),
            "summary": "Verification observed a follow-on API surface.",
            "evidence_ids": [evidence_id]
        }]
    });
    let lease_token = claimed
        .worker
        .lease_token
        .expect("claimed route lease token");
    let mut submission_tx = pool
        .begin()
        .await
        .expect("begin follow-on route submission");
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
            attempt_id,
            candidate_plan_hash:
                "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5"
                    .to_string(),
            worker_run_id: claimed.worker.id,
            stage_execution_id,
            stage_run_unit_id,
            lease_token,
            lease_owner: "follow-on-route-test".to_string(),
            attempt_epoch: claimed.worker.attempt_epoch,
            expected_checkpoint_version: claimed.worker.checkpoint_version,
            result_json,
            evidence: vec![AttemptEvidenceLink {
                evidence_id,
                role: "fact_delta".to_string(),
            }],
        },
    )
    .await
    .expect("record follow-on route Attempt submission");
    submission_tx
        .commit()
        .await
        .expect("commit follow-on route Attempt submission");
    let mut terminal_tx = pool
        .begin()
        .await
        .expect("begin follow-on route Attempt terminalization");
    terminalize_candidate_attempt(
        &mut terminal_tx,
        TerminalizeCandidateAttempt {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_a.wave_unit_id,
            organization_id: fixture.org_a.organization_id,
            candidate_id: candidate.candidate_id,
            approval_id,
            attempt_id,
            candidate_plan_hash:
                "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5"
                    .to_string(),
            expected_result_hash: submitted
                .attempt
                .result_hash
                .expect("follow-on route result hash"),
            worker_run_id: claimed.worker.id,
            stage_execution_id,
            stage_run_unit_id,
            lease_token,
            lease_owner: "follow-on-route-test".to_string(),
            attempt_epoch: claimed.worker.attempt_epoch,
            expected_checkpoint_version: claimed.worker.checkpoint_version,
        },
    )
    .await
    .expect("terminalize follow-on route Attempt");
    terminal_tx
        .commit()
        .await
        .expect("commit follow-on route Attempt terminalization");
    let delta = propose_test_delta(
        pool,
        fixture,
        candidate,
        attempt_id,
        evidence_id,
        ("api_endpoint", canonical_ref_id, canonical.content_sha256),
        "new_surface",
    )
    .await;
    close_all_active_fixture_verification_units(pool, fixture, fixture.wave_run_id).await;
    PreparedFollowOnFixture {
        delta,
        attempt_id,
        evidence_id,
        canonical_ref_id,
    }
}

#[tokio::test]
#[serial]
async fn pending_fact_delta_enrichment_is_stable_and_does_not_advance_wave() {
    let (mut db, _data_dir) = migrated_db("fact_delta_pending_enrichment").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    add_root_source_wave_unit(db.pool(), &fixture).await;
    let prepared =
        prepare_follow_on_route_fixture(db.pool(), &fixture, FollowOnEvidenceFixture::Generic)
            .await;
    let source_before: (String, i64) =
        sqlx::query_as("SELECT status,row_version FROM attack_wave_runs WHERE id=$1")
            .bind(fixture.wave_run_id)
            .fetch_one(db.pool())
            .await
            .expect("load source Wave before pending enrichment");
    let command = attack_wave_consolidations::ConsolidateAttackWave {
        operation_id: fixture.operation_id,
        scope_snapshot_id: fixture.scope_snapshot_id,
        source_wave_run_id: fixture.wave_run_id,
    };
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin pending enrichment route");
    let first = attack_wave_consolidations::consolidate_attack_wave(&mut tx, command)
        .await
        .expect("generic FactDelta evidence must persist pending enrichment");
    tx.commit().await.expect("commit pending enrichment route");
    assert_eq!(first.decision_kind, "pending_enrichment");
    assert_eq!(first.target_wave_run_id, None);
    assert_eq!(first.accepted_fact_delta_ids, vec![prepared.delta.id]);
    assert_eq!(first.pending_enrichment_count, 1);
    assert!(!first.replayed);

    let durable: (String, i64, String, Option<Uuid>, i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT wave.status,wave.row_version,delta.status,delta.consumed_by_wave_run_id,
                  (SELECT COUNT(*) FROM attack_wave_runs WHERE operation_id=$2),
                  (SELECT COUNT(*) FROM attack_wave_consolidations
                    WHERE source_wave_run_id=$1),
                  (SELECT COUNT(*) FROM attack_candidate_seeds
                    WHERE source_fact_delta_id=$3),
                  (SELECT COUNT(*) FROM attack_fact_delta_enrichment_items
                    WHERE fact_delta_id=$3 AND status='pending')
             FROM attack_wave_runs wave
             JOIN attack_fact_deltas delta ON delta.id=$3
            WHERE wave.id=$1"#,
    )
    .bind(fixture.wave_run_id)
    .bind(fixture.operation_id)
    .bind(prepared.delta.id)
    .fetch_one(db.pool())
    .await
    .expect("load pending enrichment durable state");
    assert_eq!((durable.0, durable.1), source_before);
    assert_eq!(durable.2, "accepted");
    assert_eq!(durable.3, None);
    assert_eq!((durable.4, durable.5, durable.6, durable.7), (1, 0, 0, 1));

    let queue = golish_db::repo::candidate_recovery::list_verification_queue(
        db.pool(),
        fixture.operation_id,
        fixture.wave_run_id,
    )
    .await
    .expect("load pending enrichment Verification queue");
    assert_eq!(queue.pending_enrichment_count, 1);
    assert_eq!(queue.pending_enrichments.len(), 1);
    assert!(queue.consolidation.is_none());
    let pending = &queue.pending_enrichments[0];
    assert_eq!(pending.fact_delta_id, prepared.delta.id);
    assert_eq!(pending.source_attempt_id, prepared.attempt_id);
    assert_eq!(pending.subject_kind, "api_endpoint");
    assert_eq!(pending.subject_id, prepared.canonical_ref_id);
    assert_eq!(pending.delta_kind, "new_surface");
    assert_eq!(pending.observation_kind, "surface_analysis_v2");
    assert!(pending.enrichment_required);
    assert_eq!(pending.reason_code, "typed_observation_required");
    assert!(pending
        .allowed_techniques
        .contains(&"GOLISH-NDAY".to_string()));

    let mut replay_tx = db
        .pool()
        .begin()
        .await
        .expect("begin pending enrichment replay");
    let replay = attack_wave_consolidations::consolidate_attack_wave(&mut replay_tx, command)
        .await
        .expect("pending enrichment response-loss replay must be stable");
    replay_tx
        .commit()
        .await
        .expect("commit pending enrichment replay");
    assert!(replay.replayed);
    assert_eq!(replay.consolidation_id, first.consolidation_id);
    assert_eq!(
        replay.accepted_fact_delta_ids,
        first.accepted_fact_delta_ids
    );
    assert_eq!(replay.pending_enrichment_count, 1);
    let row_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM attack_fact_delta_enrichment_items WHERE fact_delta_id=$1",
    )
    .bind(prepared.delta.id)
    .fetch_one(db.pool())
    .await
    .expect("count pending enrichment rows after replay");
    assert_eq!(row_count, 1);
    assert!(prepared.evidence_id > 0);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn recognized_unsupported_fact_delta_route_rolls_back_atomically() {
    let (mut db, _data_dir) = migrated_db("fact_delta_unsupported_route").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    add_root_source_wave_unit(db.pool(), &fixture).await;
    let prepared = prepare_follow_on_route_fixture(
        db.pool(),
        &fixture,
        FollowOnEvidenceFixture::RecognizedUnsupported,
    )
    .await;
    let source_before: (String, i64) =
        sqlx::query_as("SELECT status,row_version FROM attack_wave_runs WHERE id=$1")
            .bind(fixture.wave_run_id)
            .fetch_one(db.pool())
            .await
            .expect("load source Wave before unsupported route");
    let mut tx = db.pool().begin().await.expect("begin unsupported route");
    let error = attack_wave_consolidations::consolidate_attack_wave(
        &mut tx,
        attack_wave_consolidations::ConsolidateAttackWave {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            source_wave_run_id: fixture.wave_run_id,
        },
    )
    .await
    .expect_err("recognized unsupported FactDelta evidence must fail closed");
    assert!(error
        .to_string()
        .contains("attack_fact_delta_route_unsupported"));
    tx.rollback().await.expect("rollback unsupported route");
    let durable: (String, i64, String, i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT wave.status,wave.row_version,delta.status,
                  (SELECT COUNT(*) FROM attack_fact_delta_decisions
                    WHERE fact_delta_id=$3),
                  (SELECT COUNT(*) FROM attack_fact_delta_enrichment_items
                    WHERE fact_delta_id=$3),
                  (SELECT COUNT(*) FROM attack_wave_consolidations
                    WHERE source_wave_run_id=$1),
                  (SELECT COUNT(*) FROM attack_wave_runs WHERE operation_id=$2)
             FROM attack_wave_runs wave
             JOIN attack_fact_deltas delta ON delta.id=$3
            WHERE wave.id=$1"#,
    )
    .bind(fixture.wave_run_id)
    .bind(fixture.operation_id)
    .bind(prepared.delta.id)
    .fetch_one(db.pool())
    .await
    .expect("load unsupported route rollback state");
    assert_eq!((durable.0, durable.1), source_before);
    assert_eq!(durable.2, "proposed");
    assert_eq!((durable.3, durable.4, durable.5, durable.6), (0, 0, 0, 1));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn fact_delta_evidence_must_be_observed_during_source_attempt() {
    let (mut db, _data_dir) = migrated_db("fact_delta_evidence_time").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    sqlx::query("UPDATE attack_wave_runs SET status='verification',updated_at=NOW() WHERE id=$1")
        .bind(fixture.wave_run_id)
        .execute(db.pool())
        .await
        .expect("advance evidence-time source Wave to verification");
    let candidate = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    let approval_id = insert_approval(db.pool(), &fixture, candidate, fixture.org_a)
        .await
        .expect("approve Candidate for evidence-time fixture");
    let pre_attempt_evidence_id = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_a.organization_id,
        fixture.org_a.target_id,
        "evidence",
    )
    .await;
    sqlx::query("UPDATE audit_log SET created_at=NOW() - INTERVAL '1 hour' WHERE id=$1")
        .bind(pre_attempt_evidence_id)
        .execute(db.pool())
        .await
        .expect("make audit evidence unambiguously predate the Attempt");
    let attempt_id = insert_attempt(
        db.pool(),
        &fixture,
        candidate,
        approval_id,
        fixture.org_a,
        "running",
    )
    .await
    .expect("insert evidence-time source Attempt");
    sqlx::query(
        "INSERT INTO candidate_attempt_evidence(attempt_id,evidence_id,role)
         VALUES($1,$2,'fact_delta')",
    )
    .bind(attempt_id)
    .bind(pre_attempt_evidence_id)
    .execute(db.pool())
    .await
    .expect("legacy membership permits the pre-Attempt evidence fixture");
    sqlx::query(
        "UPDATE candidate_attempts
            SET status='blocked',result_json=$2,result_hash='sha256:evidence-time',
                terminal_at=NOW(),updated_at=NOW()
          WHERE id=$1",
    )
    .bind(attempt_id)
    .bind(serde_json::json!({
        "disposition": "blocked",
        "blocker_reason_code": "evidence_time_fixture"
    }))
    .execute(db.pool())
    .await
    .expect("terminalize evidence-time Attempt");
    let canonical_ref_hash = "sha256:evidence-time-canonical";
    let dedupe_hash = golish_db::repo::attack_fact_deltas::semantic_dedupe_hash(
        "sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963",
        "attack_candidate_work_item",
        candidate.work_item_id,
        1,
        canonical_ref_hash,
        "refuted",
    )
    .expect("hash evidence-time FactDelta");
    let fact_delta_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO attack_fact_deltas (
               id,source_attempt_id,candidate_id,operation_id,scope_snapshot_id,
               wave_run_id,wave_unit_id,organization_id,target_live_id,
               target_type_at_time,target_value_at_time,target_identity_hash,
               candidate_plan_hash,canonical_ref_kind,canonical_ref_id,
               canonical_ref_version,canonical_ref_hash,delta_kind,dedupe_hash
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,$9,'url',
               'https://shared.example.test/login','sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963',
               'sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5','attack_candidate_work_item',$10,1,$11,
               'refuted',$12
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
    .bind(candidate.work_item_id)
    .bind(canonical_ref_hash)
    .bind(dedupe_hash)
    .execute(db.pool())
    .await
    .expect("insert raw FactDelta before exercising the evidence guard");
    let link_result = sqlx::query(
        "INSERT INTO attack_fact_delta_evidence(fact_delta_id,evidence_id,role)
         VALUES($1,$2,'fact_delta')",
    )
    .bind(fact_delta_id)
    .bind(pre_attempt_evidence_id)
    .execute(db.pool())
    .await;
    assert!(
        link_result.is_err(),
        "FactDelta evidence observed before the source Attempt must fail closed"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn fact_delta_semantic_dedupe_keeps_first_provenance_and_evidence_immutable() {
    let (mut db, _data_dir) = migrated_db("fact_delta_semantic_dedupe").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    sqlx::query("UPDATE attack_wave_runs SET status='verification',updated_at=NOW() WHERE id=$1")
        .bind(fixture.wave_run_id)
        .execute(db.pool())
        .await
        .expect("advance semantic-dedupe source Wave to verification");
    let candidate = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    let approval_id = insert_approval(db.pool(), &fixture, candidate, fixture.org_a)
        .await
        .expect("approve Candidate for semantic duplicate Attempts");
    let first_attempt_id = insert_attempt_with_ordinal(
        db.pool(),
        &fixture,
        candidate,
        approval_id,
        fixture.org_a,
        "running",
        0,
    )
    .await
    .expect("insert first terminal Attempt");
    let first_evidence_id = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_a.organization_id,
        fixture.org_a.target_id,
        "evidence",
    )
    .await;
    let first_alternate_evidence_id = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_a.organization_id,
        fixture.org_a.target_id,
        "evidence",
    )
    .await;
    for evidence_id in [first_evidence_id, first_alternate_evidence_id] {
        sqlx::query(
            "INSERT INTO candidate_attempt_evidence(attempt_id,evidence_id,role)
             VALUES($1,$2,'fact_delta')",
        )
        .bind(first_attempt_id)
        .bind(evidence_id)
        .execute(db.pool())
        .await
        .expect("freeze first Attempt FactDelta evidence");
    }
    sqlx::query(
        "UPDATE candidate_attempts
            SET status='blocked',result_json=$2,result_hash=$3,
                terminal_at=NOW(),updated_at=NOW()
          WHERE id=$1 AND status='running'",
    )
    .bind(first_attempt_id)
    .bind(serde_json::json!({
        "disposition": "blocked",
        "blocker_reason_code": "follow_on_only"
    }))
    .bind(format!("sha256:terminal-{first_attempt_id}"))
    .execute(db.pool())
    .await
    .expect("terminalize first FactDelta source Attempt after freezing evidence");
    let second_attempt_id = insert_attempt_with_ordinal(
        db.pool(),
        &fixture,
        candidate,
        approval_id,
        fixture.org_a,
        "running",
        1,
    )
    .await
    .expect("insert re-observing terminal Attempt");
    let second_evidence_id = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_a.organization_id,
        fixture.org_a.target_id,
        "evidence",
    )
    .await;
    sqlx::query(
        "INSERT INTO candidate_attempt_evidence(attempt_id,evidence_id,role)
         VALUES($1,$2,'fact_delta')",
    )
    .bind(second_attempt_id)
    .bind(second_evidence_id)
    .execute(db.pool())
    .await
    .expect("freeze re-observing Attempt evidence");
    sqlx::query(
        "UPDATE candidate_attempts
            SET status='blocked',result_json=$2,result_hash=$3,
                terminal_at=NOW(),updated_at=NOW()
          WHERE id=$1 AND status='running'",
    )
    .bind(second_attempt_id)
    .bind(serde_json::json!({
        "disposition": "blocked",
        "blocker_reason_code": "follow_on_only"
    }))
    .bind(format!("sha256:terminal-{second_attempt_id}"))
    .execute(db.pool())
    .await
    .expect("terminalize re-observing Attempt after freezing evidence");

    let canonical_ref_hash = "sha256:canonical-fact-v1";
    let dedupe_hash = golish_db::repo::attack_fact_deltas::semantic_dedupe_hash(
        "sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963",
        "attack_candidate_work_item",
        candidate.work_item_id,
        1,
        canonical_ref_hash,
        "new_surface",
    )
    .expect("hash the closed semantic FactDelta identity");
    let first_command = golish_db::repo::attack_fact_deltas::ProposeAttackFactDelta {
        operation_id: fixture.operation_id,
        scope_snapshot_id: fixture.scope_snapshot_id,
        wave_run_id: fixture.wave_run_id,
        wave_unit_id: fixture.org_a.wave_unit_id,
        organization_id: fixture.org_a.organization_id,
        source_attempt_id: first_attempt_id,
        candidate_id: candidate.candidate_id,
        candidate_plan_hash:
            "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5".to_string(),
        canonical_ref_kind: "attack_candidate_work_item".to_string(),
        canonical_ref_id: candidate.work_item_id,
        canonical_ref_version: 1,
        canonical_ref_hash: canonical_ref_hash.to_string(),
        delta_kind: "new_surface".to_string(),
        dedupe_hash: dedupe_hash.clone(),
        evidence_ids: vec![first_evidence_id],
    };
    let mut first_tx = db.pool().begin().await.expect("begin first delta proposal");
    let first = golish_db::repo::attack_fact_deltas::propose_fact_delta(
        &mut first_tx,
        first_command.clone(),
    )
    .await
    .expect("store first semantic FactDelta");
    first_tx
        .commit()
        .await
        .expect("commit first delta proposal");

    let mut duplicate_command = first_command.clone();
    duplicate_command.source_attempt_id = second_attempt_id;
    duplicate_command.evidence_ids = vec![second_evidence_id];
    let mut duplicate_tx = db
        .pool()
        .begin()
        .await
        .expect("begin semantic duplicate proposal");
    let duplicate = golish_db::repo::attack_fact_deltas::propose_fact_delta(
        &mut duplicate_tx,
        duplicate_command,
    )
    .await
    .expect("a rephrased/re-evidenced Attempt must resolve to the existing semantic delta");
    duplicate_tx
        .commit()
        .await
        .expect("commit semantic duplicate replay");
    assert_eq!(duplicate.id, first.id);
    assert_eq!(duplicate.source_attempt_id, first_attempt_id);

    let unlinked_duplicate_evidence_id = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_a.organization_id,
        fixture.org_a.target_id,
        "evidence",
    )
    .await;
    let sibling_duplicate_evidence_id = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_b.organization_id,
        fixture.org_b.target_id,
        "evidence",
    )
    .await;
    for (evidence_id, label) in [
        (unlinked_duplicate_evidence_id, "unlinked"),
        (sibling_duplicate_evidence_id, "sibling"),
    ] {
        let mut invalid_duplicate = first_command.clone();
        invalid_duplicate.source_attempt_id = second_attempt_id;
        invalid_duplicate.evidence_ids = vec![evidence_id];
        let mut invalid_duplicate_tx = db
            .pool()
            .begin()
            .await
            .expect("begin invalid semantic duplicate");
        let error = golish_db::repo::attack_fact_deltas::propose_fact_delta(
            &mut invalid_duplicate_tx,
            invalid_duplicate,
        )
        .await
        .expect_err("semantic duplicate provenance evidence must still be Attempt-owned");
        assert!(
            error.to_string().contains("source Attempt"),
            "unexpected {label} duplicate error: {error}"
        );
        invalid_duplicate_tx
            .rollback()
            .await
            .expect("rollback invalid semantic duplicate");
    }

    let rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM attack_fact_deltas
          WHERE operation_id=$1 AND organization_id=$2 AND dedupe_hash=$3",
    )
    .bind(fixture.operation_id)
    .bind(fixture.org_a.organization_id)
    .bind(&dedupe_hash)
    .fetch_one(db.pool())
    .await
    .expect("count the consolidator-visible semantic delta set");
    assert_eq!(
        rows, 1,
        "one semantic fact can fuel at most one follow-on Wave"
    );
    let retained_evidence: Vec<i64> = sqlx::query_scalar(
        "SELECT evidence_id FROM attack_fact_delta_evidence
          WHERE fact_delta_id=$1 ORDER BY evidence_id",
    )
    .bind(first.id)
    .fetch_all(db.pool())
    .await
    .expect("read immutable first-observation evidence");
    assert_eq!(retained_evidence, vec![first_evidence_id]);

    let mut exact_replay_tx = db.pool().begin().await.expect("begin exact replay");
    let exact_replay = golish_db::repo::attack_fact_deltas::propose_fact_delta(
        &mut exact_replay_tx,
        first_command.clone(),
    )
    .await
    .expect("exact first proposal replay remains idempotent");
    exact_replay_tx.commit().await.expect("commit exact replay");
    assert_eq!(exact_replay.id, first.id);

    let mut evidence_drift = first_command.clone();
    evidence_drift.evidence_ids = vec![first_alternate_evidence_id];
    let mut evidence_drift_tx = db.pool().begin().await.expect("begin evidence drift");
    let evidence_drift_error = golish_db::repo::attack_fact_deltas::propose_fact_delta(
        &mut evidence_drift_tx,
        evidence_drift,
    )
    .await
    .expect_err("same-Attempt response-loss replay requires the exact first evidence set");
    assert!(evidence_drift_error
        .to_string()
        .contains("idempotency payload drift"));
    evidence_drift_tx
        .rollback()
        .await
        .expect("rollback same-Attempt evidence drift");
    let evidence_after_duplicate: Vec<i64> = sqlx::query_scalar(
        "SELECT evidence_id FROM attack_fact_delta_evidence
          WHERE fact_delta_id=$1 ORDER BY evidence_id",
    )
    .bind(first.id)
    .fetch_all(db.pool())
    .await
    .expect("re-read immutable first-observation evidence");
    assert_eq!(evidence_after_duplicate, vec![first_evidence_id]);

    let mut semantic_hash_drift = first_command.clone();
    semantic_hash_drift.dedupe_hash = "sha256:caller-chosen-drift".to_string();
    let mut semantic_hash_drift_tx = db.pool().begin().await.expect("begin semantic hash drift");
    let semantic_hash_error = golish_db::repo::attack_fact_deltas::propose_fact_delta(
        &mut semantic_hash_drift_tx,
        semantic_hash_drift,
    )
    .await
    .expect_err("caller-supplied dedupe identity drift must fail closed");
    assert!(semantic_hash_error
        .to_string()
        .contains("semantic dedupe hash drift"));
    semantic_hash_drift_tx
        .rollback()
        .await
        .expect("rollback semantic hash drift");

    let mut unknown_kind = first_command;
    unknown_kind.delta_kind = "model_invented_prose".to_string();
    unknown_kind.dedupe_hash = "sha256:unknown-kind".to_string();
    let mut unknown_tx = db.pool().begin().await.expect("begin unknown kind");
    let unknown_error =
        golish_db::repo::attack_fact_deltas::propose_fact_delta(&mut unknown_tx, unknown_kind)
            .await
            .expect_err("repo command must reject unknown FactDelta kinds");
    assert!(unknown_error.to_string().contains("FactDelta proposal"));
    unknown_tx.rollback().await.expect("rollback unknown kind");

    let raw_unknown_kind = sqlx::query(
        r#"INSERT INTO attack_fact_deltas (
               id,source_attempt_id,candidate_id,operation_id,scope_snapshot_id,
               wave_run_id,wave_unit_id,organization_id,target_live_id,
               target_type_at_time,target_value_at_time,target_identity_hash,
               candidate_plan_hash,canonical_ref_kind,canonical_ref_id,
               canonical_ref_version,canonical_ref_hash,delta_kind,dedupe_hash
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,$9,'url',
               'https://shared.example.test/login','sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963',
               'sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5','attack_candidate_work_item',$10,1,$11,
               'model_invented_prose','sha256:raw-unknown-kind'
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(first_attempt_id)
    .bind(candidate.candidate_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.wave_run_id)
    .bind(fixture.org_a.wave_unit_id)
    .bind(fixture.org_a.organization_id)
    .bind(fixture.org_a.target_id)
    .bind(candidate.work_item_id)
    .bind(canonical_ref_hash)
    .execute(db.pool())
    .await;
    assert!(
        raw_unknown_kind.is_err(),
        "the database boundary must reject model-invented FactDelta kinds"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn sibling_or_stale_canonical_ref_delta_is_rejected() {
    let (mut db, _data_dir) = migrated_db("fact_delta_canonical_acceptance").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    let root = add_root_source_wave_unit(db.pool(), &fixture).await;
    let candidate_a = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    let work_item_b =
        seed_pending_work_item(db.pool(), &fixture, fixture.org_b, "sibling-ref").await;
    sqlx::query(
        "UPDATE attack_wave_units
            SET manifest_hash=$2,manifest_count=1,manifest_frozen_at=NOW(),updated_at=NOW()
          WHERE id=$1",
    )
    .bind(fixture.org_a.wave_unit_id)
    .bind("sha256:manifest-a")
    .execute(db.pool())
    .await
    .expect("freeze org A canonical manifest");
    sqlx::query(
        "UPDATE attack_wave_units
            SET manifest_hash=$2,manifest_count=1,manifest_frozen_at=NOW(),updated_at=NOW()
          WHERE id=$1",
    )
    .bind(fixture.org_b.wave_unit_id)
    .bind("sha256:manifest-b")
    .execute(db.pool())
    .await
    .expect("freeze org B canonical manifest");
    let canonical_a = resolve_work_item_ref(
        db.pool(),
        &fixture,
        fixture.org_a.organization_id,
        candidate_a.work_item_id,
    )
    .await;
    let canonical_b = resolve_work_item_ref(
        db.pool(),
        &fixture,
        fixture.org_b.organization_id,
        work_item_b.work_item_id,
    )
    .await;
    let old_canonical_api = insert_old_api_endpoint_ref(db.pool(), &fixture, fixture.org_a).await;
    let old_api_endpoint_id = match &old_canonical_api.key {
        canonical_fact_refs::CanonicalFactKey::ApiEndpoint { api_endpoint_id } => *api_endpoint_id,
        _ => panic!("expected API endpoint canonical key"),
    };
    let approval_id = insert_approval(db.pool(), &fixture, candidate_a, fixture.org_a)
        .await
        .expect("insert exact Candidate approval");
    sqlx::query("UPDATE attack_candidates SET disposition='approved' WHERE candidate_id=$1")
        .bind(candidate_a.candidate_id)
        .execute(db.pool())
        .await
        .expect("mark canonical-ref Candidate approved");
    sqlx::query(
        "UPDATE attack_wave_units SET review_closed=TRUE,status='verification' WHERE id=$1",
    )
    .bind(fixture.org_a.wave_unit_id)
    .execute(db.pool())
    .await
    .expect("make canonical-ref Candidate claimable");
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
            lease_owner: "canonical-ref-test".to_string(),
            lease_seconds: 120,
        },
    )
    .await
    .expect("claim canonical-ref Candidate")
    .expect("canonical-ref Candidate is available");
    let attempt_id = claimed.attempt.id;
    let current_canonical_api =
        insert_current_api_endpoint_ref(db.pool(), &fixture, fixture.org_a).await;
    let current_api_endpoint_id = match &current_canonical_api.key {
        canonical_fact_refs::CanonicalFactKey::ApiEndpoint { api_endpoint_id } => *api_endpoint_id,
        _ => panic!("expected current API endpoint canonical key"),
    };
    let evidence_id = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_a.organization_id,
        fixture.org_a.target_id,
        "evidence",
    )
    .await;
    let typed_route_output = serde_json::json!({
        "schema": "verification.nuclei_template_replay_v1",
        "evidence_role": "proof",
        "result": {
            "target_id": fixture.org_a.target_id,
            "matched_url": "https://shared.example.test/login",
            "template_id": "CVE-2099-0001",
            "technique": "GOLISH-NDAY",
            "completion": "complete",
            "match_count": 1,
            "matches": [{"template_id": "CVE-2099-0001"}],
            "errors": []
        }
    })
    .to_string();
    sqlx::query(
        r#"UPDATE audit_log
              SET tool_name='verify_execute_candidate_action',
                  evidence_technique='GOLISH-NDAY',evidence_outcome='found',
                  detail=$2
            WHERE id=$1"#,
    )
    .bind(evidence_id)
    .bind(serde_json::json!({
        "organization_id": fixture.org_a.organization_id,
        "kind": "verification.nuclei_template_replay_v1",
        "raw_output": typed_route_output,
    }))
    .execute(db.pool())
    .await
    .expect("upgrade FactDelta evidence to an exact typed replay proof");
    let unattached_evidence_id = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_a.organization_id,
        fixture.org_a.target_id,
        "evidence",
    )
    .await;
    sqlx::query(
        "INSERT INTO candidate_attempt_actions(
             attempt_id,action_ordinal,capability_id,action_kind,canonical_args,status,
             outcome,outcome_hash,started_at,completed_at)
         VALUES($1,0,'verify.sql_injection','bounded_sql_injection_probe',
                '{\"target\":\"https://shared.example.test/login\"}'::jsonb,
                'completed','{}'::jsonb,'sha256:canonical-ref-action',NOW(),NOW())",
    )
    .bind(attempt_id)
    .execute(db.pool())
    .await
    .expect("finish canonical-ref Candidate action");
    let result_json = serde_json::json!({
        "disposition": "blocked",
        "blocker_reason_code": "follow_on_only",
        "fact_deltas": [
            {
                "fact_kind": "refuted",
                "canonical_ref_kind": "api_endpoint",
                "canonical_ref_id": old_api_endpoint_id,
                "canonical_ref_version": 1,
                "canonical_ref_hash": old_canonical_api.content_sha256.clone(),
                "summary": "The historical API endpoint was refuted by bounded verification.",
                "evidence_ids": [evidence_id]
            },
            {
                "fact_kind": "new_surface",
                "canonical_ref_kind": "attack_candidate_work_item",
                "canonical_ref_id": candidate_a.work_item_id,
                "canonical_ref_version": 1,
                "canonical_ref_hash": "sha256:stale-ref",
                "summary": "A deliberately stale canonical reference must be rejected.",
                "evidence_ids": [evidence_id]
            },
            {
                "fact_kind": "new_surface",
                "canonical_ref_kind": "attack_candidate_work_item",
                "canonical_ref_id": work_item_b.work_item_id,
                "canonical_ref_version": 1,
                "canonical_ref_hash": canonical_b.content_sha256.clone(),
                "summary": "A sibling-owned canonical reference must be rejected.",
                "evidence_ids": [evidence_id]
            },
            {
                "fact_kind": "created",
                "canonical_ref_kind": "attack_candidate_work_item",
                "canonical_ref_id": candidate_a.work_item_id,
                "canonical_ref_version": 1,
                "canonical_ref_hash": canonical_a.content_sha256.clone(),
                "summary": "A valid proposal whose stored dedupe hash will be forged.",
                "evidence_ids": [evidence_id]
            },
            {
                "fact_kind": "new_surface",
                "canonical_ref_kind": "api_endpoint",
                "canonical_ref_id": current_api_endpoint_id,
                "canonical_ref_version": 1,
                "canonical_ref_hash": current_canonical_api.content_sha256.clone(),
                "summary": "An exact typed replay observation may open the next Candidate Wave.",
                "evidence_ids": [evidence_id]
            }
        ]
    });
    let lease_token = claimed.worker.lease_token.expect("claimed lease token");
    let mut submission_tx = db
        .pool()
        .begin()
        .await
        .expect("begin canonical-ref submission");
    let submitted = record_attempt_submission(
        &mut submission_tx,
        RecordAttemptSubmission {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_a.wave_unit_id,
            organization_id: fixture.org_a.organization_id,
            candidate_id: candidate_a.candidate_id,
            approval_id,
            attempt_id,
            candidate_plan_hash:
                "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5"
                    .to_string(),
            worker_run_id: claimed.worker.id,
            stage_execution_id,
            stage_run_unit_id,
            lease_token,
            lease_owner: "canonical-ref-test".to_string(),
            attempt_epoch: claimed.worker.attempt_epoch,
            expected_checkpoint_version: claimed.worker.checkpoint_version,
            result_json,
            evidence: vec![AttemptEvidenceLink {
                evidence_id,
                role: "fact_delta".to_string(),
            }],
        },
    )
    .await
    .expect("record exact canonical-ref Attempt submission");
    submission_tx
        .commit()
        .await
        .expect("commit canonical-ref submission");
    let mut terminal_tx = db
        .pool()
        .begin()
        .await
        .expect("begin canonical-ref terminalization");
    terminalize_candidate_attempt(
        &mut terminal_tx,
        TerminalizeCandidateAttempt {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_a.wave_unit_id,
            organization_id: fixture.org_a.organization_id,
            candidate_id: candidate_a.candidate_id,
            approval_id,
            attempt_id,
            candidate_plan_hash:
                "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5"
                    .to_string(),
            expected_result_hash: submitted
                .attempt
                .result_hash
                .expect("canonical-ref result hash"),
            worker_run_id: claimed.worker.id,
            stage_execution_id,
            stage_run_unit_id,
            lease_token,
            lease_owner: "canonical-ref-test".to_string(),
            attempt_epoch: claimed.worker.attempt_epoch,
            expected_checkpoint_version: claimed.worker.checkpoint_version,
        },
    )
    .await
    .expect("terminalize canonical-ref source Attempt through exact receipt bundle");
    terminal_tx
        .commit()
        .await
        .expect("commit canonical-ref terminalization");

    let refuted = propose_test_delta(
        db.pool(),
        &fixture,
        candidate_a,
        attempt_id,
        evidence_id,
        (
            "api_endpoint",
            old_api_endpoint_id,
            old_canonical_api.content_sha256.clone(),
        ),
        "refuted",
    )
    .await;
    let valid = propose_test_delta(
        db.pool(),
        &fixture,
        candidate_a,
        attempt_id,
        evidence_id,
        (
            "api_endpoint",
            current_api_endpoint_id,
            current_canonical_api.content_sha256.clone(),
        ),
        "new_surface",
    )
    .await;
    let stale = propose_test_delta(
        db.pool(),
        &fixture,
        candidate_a,
        attempt_id,
        evidence_id,
        (
            "attack_candidate_work_item",
            candidate_a.work_item_id,
            "sha256:stale-ref".to_string(),
        ),
        "new_surface",
    )
    .await;
    let sibling = propose_test_delta(
        db.pool(),
        &fixture,
        candidate_a,
        attempt_id,
        evidence_id,
        (
            "attack_candidate_work_item",
            work_item_b.work_item_id,
            canonical_b.content_sha256,
        ),
        "new_surface",
    )
    .await;
    let forged_dedupe = propose_test_delta(
        db.pool(),
        &fixture,
        candidate_a,
        attempt_id,
        evidence_id,
        (
            "attack_candidate_work_item",
            candidate_a.work_item_id,
            canonical_a.content_sha256.clone(),
        ),
        "created",
    )
    .await;
    sqlx::query(
        "ALTER TABLE attack_fact_deltas
         DISABLE TRIGGER attack_fact_deltas_state_transition",
    )
    .execute(db.pool())
    .await
    .expect("disable immutable-field trigger for hostile raw SQL fixture");
    sqlx::query("UPDATE attack_fact_deltas SET dedupe_hash=$2 WHERE id=$1")
        .bind(forged_dedupe.id)
        .bind("sha256:raw-forged-dedupe")
        .execute(db.pool())
        .await
        .expect("forge persisted FactDelta dedupe hash before typed handoff");
    sqlx::query(
        "ALTER TABLE attack_fact_deltas
         ENABLE TRIGGER attack_fact_deltas_state_transition",
    )
    .execute(db.pool())
    .await
    .expect("restore immutable-field trigger after hostile raw SQL fixture");
    let receipt_exact: bool =
        sqlx::query_scalar("SELECT verification_attempt_terminal_bundle_exact($1,$2,$3,$4,$5,$6)")
            .bind(attempt_id)
            .bind(fixture.operation_id)
            .bind(fixture.scope_snapshot_id)
            .bind(fixture.wave_run_id)
            .bind(fixture.org_a.wave_unit_id)
            .bind(fixture.org_a.organization_id)
            .fetch_one(db.pool())
            .await
            .expect("evaluate exact terminal receipt before typed handoff");
    assert!(
        receipt_exact,
        "production terminalizer must leave an exact Candidate receipt bundle"
    );
    let sibling_decision_evidence_id = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_b.organization_id,
        fixture.org_b.target_id,
        "evidence",
    )
    .await;
    let mut sibling_decision_tx = db
        .pool()
        .begin()
        .await
        .expect("begin sibling checked-empty decision");
    sqlx::query(
        "INSERT INTO attack_candidate_work_item_evidence(work_item_id,evidence_id,role)
         VALUES($1,$2,'decision')",
    )
    .bind(work_item_b.work_item_id)
    .bind(sibling_decision_evidence_id)
    .execute(&mut *sibling_decision_tx)
    .await
    .expect("link sibling checked-empty decision evidence");
    sqlx::query(
        "UPDATE attack_candidate_work_items
         SET decision_kind='no_candidate',no_candidate_reason_code='checked_empty',
             no_candidate_detail='sibling reference fixture checked empty',decided_at=NOW()
         WHERE id=$1",
    )
    .bind(work_item_b.work_item_id)
    .execute(&mut *sibling_decision_tx)
    .await
    .expect("terminalize sibling reference work item");
    sibling_decision_tx
        .commit()
        .await
        .expect("commit sibling checked-empty decision");
    let database_handoff_id: Uuid =
        sqlx::query_scalar("SELECT uuid_generate_v5($1, 'verification-stage-handoff:v1')")
            .bind(fixture.org_a.wave_unit_id)
            .fetch_one(db.pool())
            .await
            .expect("derive Verification handoff id in PostgreSQL");
    assert_eq!(
        database_handoff_id,
        Uuid::new_v5(
            &fixture.org_a.wave_unit_id,
            b"verification-stage-handoff:v1",
        ),
        "Rust and PostgreSQL must derive the same Verification handoff identity",
    );
    close_all_active_fixture_verification_units(db.pool(), &fixture, fixture.wave_run_id).await;
    let forged_claim_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM verification_stage_handoffs AS handoff,
                  LATERAL jsonb_array_elements(handoff.payload -> 'typed_claims') AS claim(value)
            WHERE handoff.wave_unit_id=$1
              AND claim.value ->> 'kind'='attack_fact_delta_proposal'
              AND claim.value #>> '{payload,fact_delta_id}'=$2"#,
    )
    .bind(fixture.org_a.wave_unit_id)
    .bind(forged_dedupe.id.to_string())
    .fetch_one(db.pool())
    .await
    .expect("inspect typed handoff for raw forged-dedupe proposal");
    assert_eq!(
        forged_claim_count, 1,
        "typed handoff may retain the immutable proposal, but it cannot make a forged dedupe hash valid next-wave fuel"
    );

    // Deliberately append a malformed raw proposal after the server-authored
    // Verification seal. It is therefore absent from the typed handoff and
    // remains only a defense-in-depth input for the consolidator to reject.
    let unattached_evidence_delta_id = Uuid::new_v4();
    let unattached_dedupe_hash = golish_db::repo::attack_fact_deltas::semantic_dedupe_hash(
        "sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963",
        "attack_candidate_work_item",
        candidate_a.work_item_id,
        1,
        &canonical_a.content_sha256,
        "new_surface",
    )
    .expect("hash raw unattached-evidence FactDelta fixture");
    sqlx::query(
        r#"INSERT INTO attack_fact_deltas (
               id,source_attempt_id,candidate_id,operation_id,scope_snapshot_id,
               wave_run_id,wave_unit_id,organization_id,target_live_id,
               target_type_at_time,target_value_at_time,target_identity_hash,
               candidate_plan_hash,canonical_ref_kind,canonical_ref_id,
               canonical_ref_version,canonical_ref_hash,delta_kind,dedupe_hash
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,$9,'url',
               'https://shared.example.test/login','sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963',
               'sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5','attack_candidate_work_item',$10,1,$11,
               'new_surface',$12
           )"#,
    )
    .bind(unattached_evidence_delta_id)
    .bind(attempt_id)
    .bind(candidate_a.candidate_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.wave_run_id)
    .bind(fixture.org_a.wave_unit_id)
    .bind(fixture.org_a.organization_id)
    .bind(fixture.org_a.target_id)
    .bind(candidate_a.work_item_id)
    .bind(&canonical_a.content_sha256)
    .bind(unattached_dedupe_hash)
    .execute(db.pool())
    .await
    .expect("insert raw malformed FactDelta for consolidation defense-in-depth");
    sqlx::query(
        "INSERT INTO attack_fact_delta_evidence(fact_delta_id,evidence_id,role)
         VALUES($1,$2,'fact_delta')",
    )
    .bind(unattached_evidence_delta_id)
    .bind(unattached_evidence_id)
    .execute(db.pool())
    .await
    .expect("attach evidence omitted from the source Attempt fixture");

    sqlx::query(
        r#"CREATE FUNCTION fixture_reject_fact_delta_memory_event()
           RETURNS trigger AS $$
           BEGIN
               RAISE EXCEPTION 'fixture rejects FactDelta accepted outbox';
           END;
           $$ LANGUAGE plpgsql"#,
    )
    .execute(db.pool())
    .await
    .expect("install FactDelta outbox failure fixture function");
    sqlx::query(
        r#"CREATE TRIGGER fixture_reject_fact_delta_memory_event
           BEFORE INSERT ON knowledge_outbox_events
           FOR EACH ROW WHEN (NEW.event_name = 'FactDeltaAccepted.v1')
           EXECUTE FUNCTION fixture_reject_fact_delta_memory_event()"#,
    )
    .execute(db.pool())
    .await
    .expect("install FactDelta outbox failure fixture trigger");
    let mut failed_promotion = db
        .pool()
        .begin()
        .await
        .expect("begin failed atomic memory promotion");
    let failed = attack_wave_consolidations::consolidate_attack_wave(
        &mut failed_promotion,
        attack_wave_consolidations::ConsolidateAttackWave {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            source_wave_run_id: fixture.wave_run_id,
        },
    )
    .await;
    assert!(
        failed.is_err(),
        "an outbox failure must fail the entire Wave consolidation"
    );
    failed_promotion
        .rollback()
        .await
        .expect("rollback failed atomic memory promotion");
    let rollback_truth: (String, String, i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT
               (SELECT status FROM attack_wave_runs WHERE id=$1),
               (SELECT status FROM attack_fact_deltas WHERE id=$2),
               (SELECT COUNT(*) FROM attack_wave_consolidations WHERE source_wave_run_id=$1),
               (SELECT COUNT(*) FROM attack_wave_runs WHERE operation_id=$3),
               (SELECT COUNT(*) FROM knowledge_assertions WHERE source_operation_id=$3),
               (SELECT COUNT(*) FROM knowledge_outbox_events
                 WHERE source_operation_id=$3
                   AND event_name='FactDeltaAccepted.v1')"#,
    )
    .bind(fixture.wave_run_id)
    .bind(valid.id)
    .bind(fixture.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("inspect failed memory-promotion rollback");
    assert_eq!(
        rollback_truth,
        (
            "verification".to_string(),
            "proposed".to_string(),
            0,
            1,
            0,
            0
        ),
        "failed memory promotion must leave no partial decision, graph, Wave, Assertion, or event"
    );
    sqlx::query("DROP TRIGGER fixture_reject_fact_delta_memory_event ON knowledge_outbox_events")
        .execute(db.pool())
        .await
        .expect("remove FactDelta outbox failure fixture trigger");
    sqlx::query("DROP FUNCTION fixture_reject_fact_delta_memory_event()")
        .execute(db.pool())
        .await
        .expect("remove FactDelta outbox failure fixture function");

    let mut caller_time_tx = db
        .pool()
        .begin()
        .await
        .expect("begin caller-selected FactDelta time attack");
    let accepted_at: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "UPDATE attack_fact_deltas
            SET status='accepted',accepted_at='2000-01-01 00:00:00+00',updated_at=NOW()
          WHERE id=$1 RETURNING accepted_at",
    )
    .bind(valid.id)
    .fetch_one(&mut *caller_time_tx)
    .await
    .expect("FactDelta acceptance trigger must replace caller time");
    let consumed_at: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "UPDATE attack_fact_deltas
            SET status='consumed',consumed_by_wave_run_id=$2,
                consumed_at='2000-01-01 00:00:00+00',updated_at=NOW()
          WHERE id=$1 RETURNING consumed_at",
    )
    .bind(valid.id)
    .bind(fixture.wave_run_id)
    .fetch_one(&mut *caller_time_tx)
    .await
    .expect("FactDelta consumption trigger must replace caller time");
    let recent_floor = chrono::Utc::now() - chrono::Duration::minutes(1);
    assert!(accepted_at >= recent_floor && consumed_at >= accepted_at);
    caller_time_tx
        .rollback()
        .await
        .expect("rollback isolated caller-selected FactDelta time attack");

    let racing_canonical_hash = "sha256:racing-stale-canonical-ref";
    let racing_dedupe_hash = golish_db::repo::attack_fact_deltas::semantic_dedupe_hash(
        "sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963",
        "attack_candidate_work_item",
        candidate_a.work_item_id,
        1,
        racing_canonical_hash,
        "created",
    )
    .expect("hash proposal-before-consolidation FactDelta identity");
    let racing_command = golish_db::repo::attack_fact_deltas::ProposeAttackFactDelta {
        operation_id: fixture.operation_id,
        scope_snapshot_id: fixture.scope_snapshot_id,
        wave_run_id: fixture.wave_run_id,
        wave_unit_id: fixture.org_a.wave_unit_id,
        organization_id: fixture.org_a.organization_id,
        source_attempt_id: attempt_id,
        candidate_id: candidate_a.candidate_id,
        candidate_plan_hash:
            "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5".to_string(),
        canonical_ref_kind: "attack_candidate_work_item".to_string(),
        canonical_ref_id: candidate_a.work_item_id,
        canonical_ref_version: 1,
        canonical_ref_hash: racing_canonical_hash.to_string(),
        delta_kind: "created".to_string(),
        dedupe_hash: racing_dedupe_hash,
        evidence_ids: vec![evidence_id],
    };
    let mut proposal_first_tx = db
        .pool()
        .begin()
        .await
        .expect("begin proposal-before-consolidation race");
    let racing_delta = golish_db::repo::attack_fact_deltas::propose_fact_delta(
        &mut proposal_first_tx,
        racing_command.clone(),
    )
    .await
    .expect("proposal-first writer must lock the exact source Wave");

    let command = attack_wave_consolidations::ConsolidateAttackWave {
        operation_id: fixture.operation_id,
        scope_snapshot_id: fixture.scope_snapshot_id,
        source_wave_run_id: fixture.wave_run_id,
    };
    let (writer_ready_tx, mut writer_ready_rx) = tokio::sync::oneshot::channel();
    let (writer_commit_tx, writer_commit_rx) = tokio::sync::oneshot::channel();
    let writer_pool = db.pool().clone();
    let writer = tokio::spawn(async move {
        let mut tx = writer_pool
            .begin()
            .await
            .expect("begin canonical consolidation writer");
        let result = attack_wave_consolidations::consolidate_attack_wave(&mut tx, command)
            .await
            .expect("canonical consolidation writer must succeed");
        writer_ready_tx
            .send(result.clone())
            .expect("publish uncommitted consolidation result");
        writer_commit_rx
            .await
            .expect("receive consolidation commit permit");
        tx.commit()
            .await
            .expect("commit canonical consolidation writer");
        result
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        matches!(
            writer_ready_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ),
        "a proposal holding the source Wave lock must serialize ahead of consolidation"
    );
    proposal_first_tx
        .commit()
        .await
        .expect("commit proposal-before-consolidation writer");
    let uncommitted_result = writer_ready_rx
        .await
        .expect("consolidation proceeds after proposal-first commit");
    assert!(
        uncommitted_result
            .rejected_fact_delta_ids
            .contains(&racing_delta.id),
        "the proposal committed before Wave closure must enter the exact consolidation set"
    );

    let consolidate_once = |pool: PgPool| async move {
        let mut tx = pool.begin().await.expect("begin concurrent consolidation");
        let result = attack_wave_consolidations::consolidate_attack_wave(&mut tx, command)
            .await
            .expect("concurrent consolidation must create or replay one exact Wave");
        tx.commit()
            .await
            .expect("commit concurrent canonical consolidation");
        result
    };
    let replay = tokio::spawn(consolidate_once(db.pool().clone()));
    let mut late_command = racing_command;
    late_command.canonical_ref_hash = canonical_a.content_sha256.clone();
    late_command.dedupe_hash = golish_db::repo::attack_fact_deltas::semantic_dedupe_hash(
        "sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963",
        "attack_candidate_work_item",
        candidate_a.work_item_id,
        1,
        &late_command.canonical_ref_hash,
        "created",
    )
    .expect("hash consolidation-before-proposal FactDelta identity");
    let late_pool = db.pool().clone();
    let late = tokio::spawn(async move {
        let mut tx = late_pool
            .begin()
            .await
            .expect("begin consolidation-before-proposal race");
        let result =
            golish_db::repo::attack_fact_deltas::propose_fact_delta(&mut tx, late_command).await;
        tx.rollback()
            .await
            .expect("finish consolidation-before-proposal race");
        result
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        !replay.is_finished() && !late.is_finished(),
        "replay and proposal must both wait behind the uncommitted source Wave closure"
    );
    writer_commit_tx
        .send(())
        .expect("permit canonical consolidation commit");
    let left = writer.await.expect("join canonical consolidation writer");
    let right = replay.await.expect("join exact consolidation replay");
    let late_result = late.await.expect("join late proposal racer");
    assert!(
        late_result.is_err(),
        "a consolidation committed first must reject the waiting late proposal"
    );
    assert_ne!(
        left.replayed, right.replayed,
        "two concurrent closers must serialize into one writer and one exact replay"
    );
    assert_eq!(left.consolidation_id, right.consolidation_id);
    assert_eq!(left.target_wave_run_id, right.target_wave_run_id);
    let result = if left.replayed { right } else { left };

    assert_eq!(
        result.decision_kind, "opened_next_wave",
        "consolidation result: {result:?}"
    );
    assert_eq!(
        result
            .accepted_fact_delta_ids
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>(),
        [refuted.id, valid.id].into_iter().collect(),
    );
    let accepted_routes: std::collections::BTreeSet<(Uuid, String)> = sqlx::query_as(
        "SELECT fact_delta_id,route_kind
           FROM attack_wave_consolidation_members
          WHERE consolidation_id=$1",
    )
    .bind(result.consolidation_id)
    .fetch_all(db.pool())
    .await
    .expect("load exact direct and no-attack consolidation routes")
    .into_iter()
    .collect();
    assert_eq!(
        accepted_routes,
        [
            (refuted.id, "no_attack".to_string()),
            (valid.id, "direct".to_string()),
        ]
        .into_iter()
        .collect(),
    );
    assert_eq!(
        result
            .rejected_fact_delta_ids
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>(),
        [
            stale.id,
            sibling.id,
            forged_dedupe.id,
            unattached_evidence_delta_id,
            racing_delta.id,
        ]
        .into_iter()
        .collect(),
    );
    let target_wave_run_id = result
        .target_wave_run_id
        .expect("accepted FactDelta must bind a target Wave");
    let opened_response_loss_truth = golish_db::repo::verification_truth::load_for_operation(
        db.pool(),
        fixture.operation_id,
        None,
    )
    .await
    .expect("opened-next-Wave response loss must retain the exact source Verification truth");
    assert_eq!(opened_response_loss_truth.wave_run_id, fixture.wave_run_id);
    assert_eq!(opened_response_loss_truth.expected_units.len(), 3);
    assert_eq!(opened_response_loss_truth.snapshots.len(), 3);
    assert_eq!(
        opened_response_loss_truth
            .snapshots
            .iter()
            .map(|snapshot| snapshot.organization_id)
            .collect::<std::collections::BTreeSet<_>>(),
        [
            root.organization_id,
            fixture.org_a.organization_id,
            fixture.org_b.organization_id,
        ]
        .into_iter()
        .collect(),
    );
    assert!(opened_response_loss_truth.snapshots.iter().any(|snapshot| {
        snapshot.organization_id == root.organization_id
            && snapshot.pending_work_items == 0
            && snapshot.attempts.is_empty()
    }));
    let target_wave_unit_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM attack_wave_units
          WHERE wave_run_id=$1 AND organization_id=$2",
    )
    .bind(target_wave_run_id)
    .bind(fixture.org_a.organization_id)
    .fetch_one(db.pool())
    .await
    .expect("load exact delta-backed consumer WaveUnit id");
    let mut entry_tx = db
        .pool()
        .begin()
        .await
        .expect("begin typed follow-on entry read");
    let follow_on = attack_waves::lock_wave_unit(
        &mut entry_tx,
        fixture.operation_id,
        fixture.scope_snapshot_id,
        target_wave_run_id,
        target_wave_unit_id,
        fixture.org_a.organization_id,
    )
    .await
    .expect("typed repo must decode a consolidation-backed WaveUnit");
    assert_eq!(
        follow_on.entry,
        attack_waves::AttackWaveEntry::FactDeltaConsolidation {
            consolidation_id: result.consolidation_id,
        }
    );
    entry_tx.rollback().await.expect("finish typed entry read");
    let target_units: Vec<(Uuid, String, Option<String>)> = sqlx::query_as(
        "SELECT organization_id,status,manifest_hash
           FROM attack_wave_units WHERE wave_run_id=$1 ORDER BY ordinal",
    )
    .bind(target_wave_run_id)
    .fetch_all(db.pool())
    .await
    .expect("load all follow-on WaveUnits");
    assert_eq!(
        target_units.len(),
        3,
        "follow-on Wave must retain every frozen org"
    );
    assert!(target_units
        .iter()
        .any(|(organization_id, status, manifest_hash)| {
            *organization_id == fixture.org_a.organization_id
                && status == "open"
                && manifest_hash.is_some()
        }));
    assert!(target_units
        .iter()
        .any(|(organization_id, status, manifest_hash)| {
            *organization_id == fixture.org_b.organization_id
                && status == "terminal"
                && manifest_hash.is_none()
        }));
    assert!(target_units
        .iter()
        .any(|(organization_id, status, manifest_hash)| {
            *organization_id == root.organization_id
                && status == "terminal"
                && manifest_hash.is_none()
        }));
    let current_authority = attack_waves::load_current_authority(db.pool(), fixture.operation_id)
        .await
        .expect("follow-on Wave must be the exact current DB authority");
    let current_authority = match current_authority {
        attack_waves::AttackWaveAuthority::Current(current) => current,
        other => panic!("expected follow-on current Wave authority, got {other:?}"),
    };
    assert_eq!(current_authority.wave.id, target_wave_run_id);
    assert_eq!(current_authority.wave.generation, 1);
    assert_eq!(current_authority.units.len(), 3);
    assert!(current_authority.units.iter().all(|unit| {
        unit.unit.entry
            == (attack_waves::AttackWaveEntry::FactDeltaConsolidation {
                consolidation_id: result.consolidation_id,
            })
    }));
    assert_eq!(
        current_authority
            .units
            .iter()
            .filter(|unit| matches!(
                unit.state,
                attack_waves::CurrentAttackWaveUnitState::Runnable { .. }
            ))
            .count(),
        1,
        "only the organization receiving an accepted FactDelta is runnable"
    );
    assert_eq!(
        current_authority
            .units
            .iter()
            .filter(|unit| matches!(
                unit.state,
                attack_waves::CurrentAttackWaveUnitState::TerminalNoInput
            ))
            .count(),
        2,
        "zero-input frozen organizations remain explicit terminal authorities"
    );
    let statuses: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT id,status FROM attack_fact_deltas WHERE id=ANY($1) ORDER BY id")
            .bind(vec![
                refuted.id,
                valid.id,
                stale.id,
                sibling.id,
                forged_dedupe.id,
                unattached_evidence_delta_id,
            ])
            .fetch_all(db.pool())
            .await
            .expect("read terminal FactDelta decisions");
    assert!(statuses.contains(&(refuted.id, "accepted".to_string())));
    assert!(statuses.contains(&(valid.id, "consumed".to_string())));
    assert!(statuses.contains(&(stale.id, "rejected".to_string())));
    assert!(statuses.contains(&(sibling.id, "rejected".to_string())));
    assert!(statuses.contains(&(forged_dedupe.id, "rejected".to_string())));
    assert!(statuses.contains(&(unattached_evidence_delta_id, "rejected".to_string())));
    let consumed_timestamp_tamper = sqlx::query(
        "UPDATE attack_fact_deltas
            SET consumed_at='2000-01-01 00:00:00+00',updated_at=NOW()
          WHERE id=$1",
    )
    .bind(valid.id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        consumed_timestamp_tamper.map(|_| ()),
        "P0001",
        "consumed FactDelta terminal timestamp immutability",
    );
    let consumed_owner_tamper = sqlx::query(
        "UPDATE attack_fact_deltas
            SET consumed_by_wave_run_id=$2,updated_at=NOW()
          WHERE id=$1",
    )
    .bind(valid.id)
    .bind(fixture.wave_run_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        consumed_owner_tamper.map(|_| ()),
        "P0001",
        "consumed FactDelta terminal owner immutability",
    );
    let post_consolidation_replay = propose_test_delta(
        db.pool(),
        &fixture,
        candidate_a,
        attempt_id,
        evidence_id,
        (
            "api_endpoint",
            old_api_endpoint_id,
            old_canonical_api.content_sha256.clone(),
        ),
        "refuted",
    )
    .await;
    assert_eq!(post_consolidation_replay.id, refuted.id);
    assert_eq!(post_consolidation_replay.status, "accepted");
    assert_ne!(root.wave_unit_id, Uuid::nil());
    let promotion_counts: (i64, i64, i64) = sqlx::query_as(
        r#"SELECT
               (SELECT COUNT(*) FROM knowledge_assertions
                 WHERE source_kind='fact_delta' AND source_id_value=$1),
               (SELECT COUNT(*) FROM knowledge_outbox_events
                 WHERE event_name='FactDeltaAccepted.v1'
                   AND source_kind='fact_delta' AND source_id_value=$1),
               (SELECT COUNT(*) FROM knowledge_projection_deliveries AS delivery
                  JOIN knowledge_outbox_events AS event ON event.event_id=delivery.event_id
                 WHERE event.event_name='FactDeltaAccepted.v1'
                   AND event.source_kind='fact_delta' AND event.source_id_value=$1)"#,
    )
    .bind(valid.id.to_string())
    .fetch_one(db.pool())
    .await
    .expect("count atomic FactDelta memory promotion");
    assert_eq!(
        promotion_counts,
        (1, 1, 4),
        "one accepted FactDelta must atomically produce one Assertion, one event, and four deliveries"
    );
    let expected_project_scope_id: Uuid = sqlx::query_scalar(
        "SELECT project_scope_id FROM operation_org_scope_snapshots WHERE id=$1",
    )
    .bind(fixture.scope_snapshot_id)
    .fetch_one(db.pool())
    .await
    .expect("load frozen project scope id");
    let memory_event: (
        String,
        String,
        String,
        String,
        i64,
        Uuid,
        Uuid,
        serde_json::Value,
        chrono::DateTime<chrono::Utc>,
    ) = sqlx::query_as(
        r#"SELECT event_name,source_kind,source_id_value,source_stream_key,
                  source_version,project_scope_id,organization_id_at_time,payload,occurred_at
             FROM knowledge_outbox_events
            WHERE event_name='FactDeltaAccepted.v1' AND source_id_value=$1"#,
    )
    .bind(valid.id.to_string())
    .fetch_one(db.pool())
    .await
    .expect("load exact FactDelta accepted event");
    assert_eq!(memory_event.0, "FactDeltaAccepted.v1");
    assert_eq!(memory_event.1, "fact_delta");
    assert_eq!(memory_event.2, valid.id.to_string());
    assert_eq!(memory_event.3, format!("fact-delta:{}", valid.id));
    assert_eq!(memory_event.4, 1);
    assert_eq!(memory_event.5, expected_project_scope_id);
    assert_eq!(memory_event.6, fixture.org_a.organization_id);
    let structured_payload = &memory_event.7["structured_payload"];
    assert_eq!(structured_payload["fact_delta_id"], valid.id.to_string());
    assert_eq!(
        structured_payload["consolidation_id"],
        result.consolidation_id.to_string()
    );
    assert_eq!(
        structured_payload["canonical_ref"]["id"],
        current_api_endpoint_id.to_string()
    );
    assert_eq!(
        structured_payload["evidence_ids"],
        serde_json::json!([evidence_id])
    );
    let memory_assertion: (
        String,
        Uuid,
        Uuid,
        String,
        String,
        i64,
        Vec<i64>,
        chrono::DateTime<chrono::Utc>,
        serde_json::Value,
    ) = sqlx::query_as(
        r#"SELECT visibility,project_scope_id,organization_id_at_time,source_kind,
                  source_stream_key,source_version,evidence_refs,valid_from,object_value
             FROM knowledge_assertions
            WHERE source_kind='fact_delta' AND source_id_value=$1"#,
    )
    .bind(valid.id.to_string())
    .fetch_one(db.pool())
    .await
    .expect("load exact FactDelta producer Assertion");
    assert_eq!(memory_assertion.0, "organization_long_term");
    assert_eq!(memory_assertion.1, expected_project_scope_id);
    assert_eq!(memory_assertion.2, fixture.org_a.organization_id);
    assert_eq!(memory_assertion.3, "fact_delta");
    assert_eq!(memory_assertion.4, format!("fact-delta:{}", valid.id));
    assert_eq!(memory_assertion.5, 1);
    assert_eq!(memory_assertion.6, vec![evidence_id]);
    assert_eq!(memory_assertion.7, memory_event.8);
    assert_eq!(
        memory_assertion.8["fact_delta"]["consolidation_id"],
        result.consolidation_id.to_string()
    );
    let rejected_event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM knowledge_outbox_events
          WHERE event_name='FactDeltaAccepted.v1' AND source_id_value=ANY($1)",
    )
    .bind(vec![
        stale.id.to_string(),
        sibling.id.to_string(),
        forged_dedupe.id.to_string(),
        unattached_evidence_delta_id.to_string(),
    ])
    .fetch_one(db.pool())
    .await
    .expect("count rejected FactDelta events");
    assert_eq!(rejected_event_count, 0);

    let mut replay_tx = db.pool().begin().await.expect("begin consolidation replay");
    let replay = attack_wave_consolidations::consolidate_attack_wave(
        &mut replay_tx,
        attack_wave_consolidations::ConsolidateAttackWave {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            source_wave_run_id: fixture.wave_run_id,
        },
    )
    .await
    .expect("response-loss replay must return the persisted consolidation");
    replay_tx.commit().await.expect("commit read-only replay");
    assert!(replay.replayed);
    assert_eq!(replay.consolidation_id, result.consolidation_id);
    assert_eq!(replay.target_wave_run_id, result.target_wave_run_id);
    assert_eq!(
        replay.accepted_fact_delta_ids,
        result.accepted_fact_delta_ids
    );
    assert_eq!(
        replay
            .rejected_fact_delta_ids
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>(),
        result.rejected_fact_delta_ids.iter().copied().collect(),
    );
    let replay_wave_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM attack_wave_runs WHERE operation_id=$1")
            .bind(fixture.operation_id)
            .fetch_one(db.pool())
            .await
            .expect("count Waves after exact replay");
    assert_eq!(replay_wave_count, 2, "replay must not create generation 2");
    let replay_promotion_counts: (i64, i64, i64) = sqlx::query_as(
        r#"SELECT
               (SELECT COUNT(*) FROM knowledge_assertions
                 WHERE source_kind='fact_delta' AND source_id_value=$1),
               (SELECT COUNT(*) FROM knowledge_outbox_events
                 WHERE event_name='FactDeltaAccepted.v1' AND source_id_value=$1),
               (SELECT COUNT(*) FROM knowledge_projection_deliveries AS delivery
                  JOIN knowledge_outbox_events AS event ON event.event_id=delivery.event_id
                 WHERE event.event_name='FactDeltaAccepted.v1' AND event.source_id_value=$1)"#,
    )
    .bind(valid.id.to_string())
    .fetch_one(db.pool())
    .await
    .expect("count exact memory rows after consolidation replay");
    assert_eq!(replay_promotion_counts, (1, 1, 4));
    let mut follow_on_evidence_tx = db
        .pool()
        .begin()
        .await
        .expect("begin follow-on entry evidence read");
    let follow_on_entry_evidence = golish_db::repo::attack_candidate_work_items::load_frozen_entry_evidence_ids_with_connection(
        &mut follow_on_evidence_tx,
        fixture.operation_id,
        fixture.scope_snapshot_id,
        target_wave_run_id,
        target_wave_unit_id,
        fixture.org_a.organization_id,
    )
    .await
    .expect("follow-on Candidate final seal must inherit exact FactDelta evidence");
    assert_eq!(follow_on_entry_evidence, vec![evidence_id]);
    follow_on_evidence_tx
        .rollback()
        .await
        .expect("finish follow-on entry evidence read");
    let follow_on_stage_execution_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status)
         VALUES($1,$2,'attack_candidate','started')",
    )
    .bind(follow_on_stage_execution_id)
    .bind(fixture.operation_id)
    .execute(db.pool())
    .await
    .expect("insert follow-on attack_candidate StageExecution");
    let follow_on_stage_run_unit_id = Uuid::new_v4();
    let follow_on_worker_run_id = Uuid::new_v4();
    let follow_on_lease_token = Uuid::new_v4();
    let follow_on_submission_id = Uuid::new_v4();
    insert_final_passed_unit(
        db.pool(),
        &fixture,
        fixture.org_a.organization_id,
        follow_on_stage_execution_id,
        follow_on_stage_run_unit_id,
        follow_on_worker_run_id,
        follow_on_lease_token,
        follow_on_submission_id,
        "attack_candidate",
        "attack_analyst",
        1,
        1,
        true,
    )
    .await;
    let follow_on_manifest = golish_db::repo::attack_candidate_work_items::load_for_wave_unit(
        db.pool(),
        fixture.operation_id,
        fixture.scope_snapshot_id,
        target_wave_run_id,
        target_wave_unit_id,
        fixture.org_a.organization_id,
    )
    .await
    .expect("load exact follow-on Candidate manifest");
    assert_eq!(follow_on_manifest.items.len(), 1);
    let follow_on_work_item_id = follow_on_manifest.items[0].work_item.id;
    let follow_on_acceptance = AcceptCandidateBatch {
        operation_id: fixture.operation_id,
        scope_snapshot_id: fixture.scope_snapshot_id,
        wave_run_id: target_wave_run_id,
        wave_unit_id: target_wave_unit_id,
        organization_id: fixture.org_a.organization_id,
        decision_stage_execution_id: follow_on_stage_execution_id,
        decision_stage_run_unit_id: follow_on_stage_run_unit_id,
        decision_deliverable_submission_id: follow_on_submission_id,
        manifest_hash: golish_db::repo::attack_candidate_work_items::canonical_manifest_hash(
            &follow_on_manifest,
        ),
        expected_work_item_ids: vec![follow_on_work_item_id],
        candidates: Vec::new(),
        no_candidate_decisions: vec![NoCandidateDecision {
            work_item_id: follow_on_work_item_id,
            reason_code: "fact_delta_checked_empty".to_string(),
            detail: "follow-on FactDelta produced no additional Candidate".to_string(),
            evidence_ids: vec![evidence_id],
        }],
    };
    let mut follow_on_acceptance_tx = db
        .pool()
        .begin()
        .await
        .expect("begin follow-on Candidate acceptance");
    let follow_on_accepted =
        accept_gate_passed_candidate_batch(&mut follow_on_acceptance_tx, follow_on_acceptance)
            .await
            .expect("consolidation-backed WaveUnit must accept an exact final-pass decision");
    follow_on_acceptance_tx
        .commit()
        .await
        .expect("commit follow-on Candidate acceptance");
    assert!(follow_on_accepted.candidate_ids.is_empty());
    assert_eq!(
        follow_on_accepted.no_candidate_work_item_ids,
        vec![follow_on_work_item_id]
    );

    sqlx::query("UPDATE attack_wave_runs SET status='verification',updated_at=NOW() WHERE id=$1")
        .bind(target_wave_run_id)
        .execute(db.pool())
        .await
        .expect("advance follow-on Wave to verification");
    close_all_active_fixture_verification_units(db.pool(), &fixture, target_wave_run_id).await;
    let follow_on_truth = golish_db::repo::verification_truth::load_for_operation(
        db.pool(),
        fixture.operation_id,
        None,
    )
    .await
    .expect("follow-on Verification truth must retain terminal zero-input siblings");
    assert_eq!(follow_on_truth.wave_run_id, target_wave_run_id);
    assert_eq!(follow_on_truth.expected_units.len(), 3);
    assert_eq!(follow_on_truth.snapshots.len(), 3);
    assert_eq!(
        follow_on_truth
            .snapshots
            .iter()
            .filter(|snapshot| snapshot.organization_id != fixture.org_a.organization_id)
            .count(),
        2,
        "both frozen zero-input organizations remain explicit terminal truth"
    );
    assert!(follow_on_truth
        .snapshots
        .iter()
        .filter(|snapshot| snapshot.organization_id != fixture.org_a.organization_id)
        .all(|snapshot| {
            snapshot.pending_work_items == 0
                && snapshot.approved_ever == 0
                && snapshot.attempts.is_empty()
        }));
    let mut next_wave_tx = db
        .pool()
        .begin()
        .await
        .expect("begin follow-on no-delta consolidation");
    let next_wave_result = attack_wave_consolidations::consolidate_attack_wave(
        &mut next_wave_tx,
        attack_wave_consolidations::ConsolidateAttackWave {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            source_wave_run_id: target_wave_run_id,
        },
    )
    .await
    .expect("terminal no-input sibling units must not block the global cursor");
    next_wave_tx
        .commit()
        .await
        .expect("commit follow-on no-delta consolidation");
    assert_eq!(next_wave_result.decision_kind, "closed_no_delta");
    assert_eq!(next_wave_result.target_wave_run_id, None);
    let closed_response_loss_truth = golish_db::repo::verification_truth::load_for_operation(
        db.pool(),
        fixture.operation_id,
        None,
    )
    .await
    .expect("closed-no-delta response loss must retain normal and no-input source Unit truth");
    assert_eq!(closed_response_loss_truth.wave_run_id, target_wave_run_id);
    assert_eq!(closed_response_loss_truth.expected_units.len(), 3);
    assert_eq!(closed_response_loss_truth.snapshots.len(), 3);
    assert_eq!(
        closed_response_loss_truth
            .snapshots
            .iter()
            .filter(|snapshot| snapshot.organization_id != fixture.org_a.organization_id)
            .count(),
        2,
        "both terminal-no-input siblings remain explicit during response-loss replay"
    );
    let generation_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM attack_wave_runs WHERE operation_id=$1")
            .bind(fixture.operation_id)
            .fetch_one(db.pool())
            .await
            .expect("count Waves after follow-on no-delta close");
    assert_eq!(generation_count, 2);

    let target_work_item_id: Uuid = sqlx::query_scalar(
        "SELECT target_work_item_id
           FROM attack_wave_consolidation_members
          WHERE consolidation_id=$1 AND fact_delta_id=$2",
    )
    .bind(result.consolidation_id)
    .bind(valid.id)
    .fetch_one(db.pool())
    .await
    .expect("load exact consumer work item for graph tamper test");
    let mut incomplete_graph = db
        .pool()
        .begin()
        .await
        .expect("begin immutable FactDelta-set graph tamper");
    sqlx::query(
        r#"INSERT INTO attack_wave_consolidation_members (
               consolidation_id,ordinal,fact_delta_id,source_attempt_id,
               candidate_id,operation_id,scope_snapshot_id,source_wave_run_id,
               source_wave_unit_id,organization_id,target_wave_run_id,
               target_wave_unit_id,target_work_item_id,route_kind,member_hash
           ) VALUES (
               $1,2,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,'direct',
               'sha256:late-rejected-member'
           )"#,
    )
    .bind(result.consolidation_id)
    .bind(stale.id)
    .bind(attempt_id)
    .bind(candidate_a.candidate_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.wave_run_id)
    .bind(fixture.org_a.wave_unit_id)
    .bind(fixture.org_a.organization_id)
    .bind(target_wave_run_id)
    .bind(target_wave_unit_id)
    .bind(target_work_item_id)
    .execute(&mut *incomplete_graph)
    .await
    .expect("stage a member outside the frozen parent count");
    assert_sqlstate(
        incomplete_graph.commit().await,
        "P0001",
        "consolidation parent count and exact member graph must match at commit",
    );

    let mut late_proposal_tx = db
        .pool()
        .begin()
        .await
        .expect("begin late FactDelta proposal");
    let late_dedupe_hash = golish_db::repo::attack_fact_deltas::semantic_dedupe_hash(
        "sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963",
        "attack_candidate_work_item",
        candidate_a.work_item_id,
        1,
        &canonical_a.content_sha256,
        "created",
    )
    .expect("hash late FactDelta semantic identity");
    let late_proposal = golish_db::repo::attack_fact_deltas::propose_fact_delta(
        &mut late_proposal_tx,
        golish_db::repo::attack_fact_deltas::ProposeAttackFactDelta {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_a.wave_unit_id,
            organization_id: fixture.org_a.organization_id,
            source_attempt_id: attempt_id,
            candidate_id: candidate_a.candidate_id,
            candidate_plan_hash:
                "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5"
                    .to_string(),
            canonical_ref_kind: "attack_candidate_work_item".to_string(),
            canonical_ref_id: candidate_a.work_item_id,
            canonical_ref_version: 1,
            canonical_ref_hash: canonical_a.content_sha256.clone(),
            delta_kind: "created".to_string(),
            dedupe_hash: late_dedupe_hash,
            evidence_ids: vec![evidence_id],
        },
    )
    .await;
    assert!(
        late_proposal.is_err(),
        "a frozen source Wave must reject a late FactDelta proposal"
    );
    late_proposal_tx
        .rollback()
        .await
        .expect("finish late FactDelta proposal test");

    let consolidation_tamper = sqlx::query(
        "UPDATE attack_wave_consolidations
            SET source_barrier_hash='sha256:tampered' WHERE id=$1",
    )
    .bind(result.consolidation_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        consolidation_tamper.map(|_| ()),
        "P0001",
        "immutable Wave consolidation update",
    );
    let member_tamper = sqlx::query(
        "UPDATE attack_wave_consolidation_members
            SET member_hash='sha256:tampered' WHERE consolidation_id=$1",
    )
    .bind(result.consolidation_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        member_tamper.map(|_| ()),
        "P0001",
        "immutable Wave consolidation member update",
    );
    let member_delete =
        sqlx::query("DELETE FROM attack_wave_consolidation_members WHERE consolidation_id=$1")
            .bind(result.consolidation_id)
            .execute(db.pool())
            .await;
    assert_sqlstate(
        member_delete.map(|_| ()),
        "P0001",
        "immutable Wave consolidation member delete",
    );
    let source_wave_tamper =
        sqlx::query("UPDATE attack_wave_runs SET policy_hash='sha256:tampered' WHERE id=$1")
            .bind(fixture.wave_run_id)
            .execute(db.pool())
            .await;
    assert_sqlstate(
        source_wave_tamper.map(|_| ()),
        "23514",
        "consolidated source Wave update",
    );
    let source_unit_tamper =
        sqlx::query("UPDATE attack_wave_units SET verification_closed=FALSE WHERE id=$1")
            .bind(fixture.org_a.wave_unit_id)
            .execute(db.pool())
            .await;
    assert_sqlstate(
        source_unit_tamper.map(|_| ()),
        "23514",
        "consolidated source WaveUnit update",
    );
    let late_evidence_id = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_a.organization_id,
        fixture.org_a.target_id,
        "evidence",
    )
    .await;
    let late_evidence = sqlx::query(
        "INSERT INTO attack_fact_delta_evidence(fact_delta_id,evidence_id,role)
         VALUES($1,$2,'fact_delta')",
    )
    .bind(valid.id)
    .bind(late_evidence_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        late_evidence.map(|_| ()),
        "P0001",
        "accepted or consumed FactDelta evidence membership change",
    );
    let decision_update = sqlx::query(
        "UPDATE attack_fact_delta_decisions
            SET reason_code='canonical_ref_stale' WHERE fact_delta_id=$1",
    )
    .bind(valid.id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        decision_update.map(|_| ()),
        "P0001",
        "immutable FactDelta decision update",
    );
    let decision_delete =
        sqlx::query("DELETE FROM attack_fact_delta_decisions WHERE fact_delta_id=$1")
            .bind(valid.id)
            .execute(db.pool())
            .await;
    assert_sqlstate(
        decision_delete.map(|_| ()),
        "P0001",
        "immutable FactDelta decision delete",
    );
    sqlx::query(
        "ALTER TABLE attack_fact_deltas
         DISABLE TRIGGER attack_fact_deltas_state_transition",
    )
    .execute(db.pool())
    .await
    .expect("disable FactDelta immutable-field trigger for hostile graph fixture");
    sqlx::query(
        "ALTER TABLE attack_fact_delta_decisions
         DISABLE TRIGGER attack_fact_delta_decisions_immutable",
    )
    .execute(db.pool())
    .await
    .expect("disable decision immutable-field trigger for hostile graph fixture");
    let mut forged_graph_tx = db
        .pool()
        .begin()
        .await
        .expect("begin privileged forged consolidation graph transaction");
    sqlx::query("UPDATE attack_fact_deltas SET dedupe_hash=$2 WHERE id=$1")
        .bind(valid.id)
        .bind("sha256:forged-consumed-dedupe")
        .execute(&mut *forged_graph_tx)
        .await
        .expect("forge consumed FactDelta semantic identity behind an existing graph");
    sqlx::query(
        "UPDATE attack_fact_delta_decisions
            SET evidence_set_hash='sha256:forged-evidence-set',
                decision_hash='sha256:forged-decision'
          WHERE fact_delta_id=$1",
    )
    .bind(valid.id)
    .execute(&mut *forged_graph_tx)
    .await
    .expect("forge accepted decision material behind an existing graph");
    let forged_graph_commit = forged_graph_tx.commit().await;
    sqlx::query(
        "ALTER TABLE attack_fact_deltas
         ENABLE TRIGGER attack_fact_deltas_state_transition",
    )
    .execute(db.pool())
    .await
    .expect("restore FactDelta immutable-field trigger after hostile graph fixture");
    sqlx::query(
        "ALTER TABLE attack_fact_delta_decisions
         ENABLE TRIGGER attack_fact_delta_decisions_immutable",
    )
    .execute(db.pool())
    .await
    .expect("restore decision immutable-field trigger after hostile graph fixture");
    assert_sqlstate(
        forged_graph_commit,
        "P0001",
        "deferred consolidation graph semantic material recomputation",
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn no_delta_close_is_terminal_memberless_and_replay_safe() {
    let (mut db, _data_dir) = migrated_db("fact_delta_no_delta_close").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    add_root_source_wave_unit(db.pool(), &fixture).await;
    sqlx::query("UPDATE attack_wave_runs SET status='verification',updated_at=NOW() WHERE id=$1")
        .bind(fixture.wave_run_id)
        .execute(db.pool())
        .await
        .expect("advance empty source Wave to verification");
    close_all_active_fixture_verification_units(db.pool(), &fixture, fixture.wave_run_id).await;

    let command = attack_wave_consolidations::ConsolidateAttackWave {
        operation_id: fixture.operation_id,
        scope_snapshot_id: fixture.scope_snapshot_id,
        source_wave_run_id: fixture.wave_run_id,
    };
    let mut tx = db.pool().begin().await.expect("begin no-delta close");
    let result = attack_wave_consolidations::consolidate_attack_wave(&mut tx, command)
        .await
        .expect("empty exact set must close the source Wave");
    tx.commit().await.expect("commit no-delta close");
    assert_eq!(result.decision_kind, "closed_no_delta");
    assert_eq!(result.target_wave_run_id, None);
    assert!(result.accepted_fact_delta_ids.is_empty());
    assert!(result.rejected_fact_delta_ids.is_empty());
    let graph: (String, i32, i64) = sqlx::query_as(
        r#"SELECT source.status,consolidation.fact_delta_count,
                  (SELECT COUNT(*) FROM attack_wave_consolidation_members AS member
                    WHERE member.consolidation_id=consolidation.id)
             FROM attack_wave_consolidations AS consolidation
             JOIN attack_wave_runs AS source ON source.id=consolidation.source_wave_run_id
            WHERE consolidation.id=$1"#,
    )
    .bind(result.consolidation_id)
    .fetch_one(db.pool())
    .await
    .expect("load closed no-delta graph");
    assert_eq!(graph, ("terminal".to_string(), 0, 0));
    let no_delta_memory_events: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM knowledge_outbox_events
          WHERE source_operation_id=$1 AND event_name='FactDeltaAccepted.v1'",
    )
    .bind(fixture.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("count no-delta memory events");
    assert_eq!(no_delta_memory_events, 0);
    let terminal_units: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM attack_wave_units
          WHERE wave_run_id=$1 AND status='terminal' AND terminal_at IS NOT NULL
            AND review_closed AND verification_closed AND consolidation_status='terminal'",
    )
    .bind(fixture.wave_run_id)
    .fetch_one(db.pool())
    .await
    .expect("count terminal empty source WaveUnits");
    assert_eq!(terminal_units, 3);
    let terminal_authority = attack_waves::load_current_authority(db.pool(), fixture.operation_id)
        .await
        .expect("converged Wave history must remain explicitly terminal");
    let terminal_authority = match terminal_authority {
        attack_waves::AttackWaveAuthority::Terminal(terminal) => terminal,
        other => panic!("expected terminal Wave authority, got {other:?}"),
    };
    assert_eq!(terminal_authority.last_wave.id, fixture.wave_run_id);
    assert_eq!(terminal_authority.last_wave.generation, 0);

    let response_loss_truth = golish_db::repo::verification_truth::load_for_operation(
        db.pool(),
        fixture.operation_id,
        None,
    )
    .await
    .expect("closed-no-delta response loss must reload the exact terminal source Wave");
    assert_eq!(response_loss_truth.wave_run_id, fixture.wave_run_id);
    assert_eq!(response_loss_truth.expected_units.len(), 3);
    assert_eq!(response_loss_truth.snapshots.len(), 3);

    let mut replay_tx = db.pool().begin().await.expect("begin no-delta replay");
    let replay = attack_wave_consolidations::consolidate_attack_wave(&mut replay_tx, command)
        .await
        .expect("no-delta response-loss replay must return the same graph");
    replay_tx.commit().await.expect("finish no-delta replay");
    assert!(replay.replayed);
    assert_eq!(replay.consolidation_id, result.consolidation_id);
    let wave_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM attack_wave_runs WHERE operation_id=$1")
            .bind(fixture.operation_id)
            .fetch_one(db.pool())
            .await
            .expect("count Waves after no-delta replay");
    assert_eq!(wave_count, 1);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn verification_truth_never_replays_an_unconsolidated_terminal_wave() {
    let (mut db, _data_dir) = migrated_db("verification_truth_unconsolidated_terminal").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    add_root_source_wave_unit(db.pool(), &fixture).await;
    sqlx::query(
        "UPDATE attack_wave_units
            SET status='terminal',review_closed=TRUE,verification_closed=TRUE,
                consolidation_status='terminal',terminal_at=NOW(),updated_at=NOW()
          WHERE wave_run_id=$1",
    )
    .bind(fixture.wave_run_id)
    .execute(db.pool())
    .await
    .expect("terminalize unconsolidated fixture WaveUnits");
    sqlx::query(
        "UPDATE attack_wave_runs
            SET status='terminal',terminal_at=NOW(),updated_at=NOW()
          WHERE id=$1",
    )
    .bind(fixture.wave_run_id)
    .execute(db.pool())
    .await
    .expect("terminalize unconsolidated fixture Wave");

    let error = golish_db::repo::verification_truth::load_for_operation(
        db.pool(),
        fixture.operation_id,
        None,
    )
    .await
    .expect_err("an arbitrary terminal Wave cannot become Verification replay authority");
    assert!(
        error
            .to_string()
            .contains("exact Verification wave authority is missing"),
        "unconsolidated terminal history must fail closed: {error}"
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn fuel_cap_closes_wave_and_persists_reportable_residual_risk() {
    let (mut db, _data_dir) = migrated_db("fact_delta_fuel_residual").await;
    let fixture = seed_attack_fixture(db.pool()).await;
    add_root_source_wave_unit(db.pool(), &fixture).await;
    let candidate = seed_candidate(db.pool(), &fixture, fixture.org_a).await;
    sqlx::query(
        "UPDATE attack_wave_units
            SET manifest_hash='sha256:fuel-manifest',manifest_count=1,
                manifest_frozen_at=NOW(),updated_at=NOW()
          WHERE id=$1",
    )
    .bind(fixture.org_a.wave_unit_id)
    .execute(db.pool())
    .await
    .expect("freeze fuel fixture manifest");
    let approval_id = insert_approval(db.pool(), &fixture, candidate, fixture.org_a)
        .await
        .expect("insert fuel fixture approval");
    sqlx::query("UPDATE attack_candidates SET disposition='approved' WHERE candidate_id=$1")
        .bind(candidate.candidate_id)
        .execute(db.pool())
        .await
        .expect("mark fuel fixture Candidate approved");
    sqlx::query(
        r#"INSERT INTO candidate_attempts (
               id,candidate_id,approval_id,operation_id,scope_snapshot_id,
               wave_run_id,wave_unit_id,organization_id,target_live_id,
               target_type_at_time,target_value_at_time,target_identity_hash,
               candidate_plan_hash,ordinal,status
           )
           SELECT uuid_generate_v4(),$1,$2,$3,$4,$5,$6,$7,$8,'url',
                  'https://shared.example.test/login',
                  'sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963',
                  'sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5',
                  ordinal,'abandoned'
             FROM generate_series(0,198) AS ordinal"#,
    )
    .bind(candidate.candidate_id)
    .bind(approval_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.wave_run_id)
    .bind(fixture.org_a.wave_unit_id)
    .bind(fixture.org_a.organization_id)
    .bind(fixture.org_a.target_id)
    .execute(db.pool())
    .await
    .expect("seed 199 durable abandoned Attempt fuel rows under the canonical cap");
    sqlx::query(
        "UPDATE attack_wave_units SET review_closed=TRUE,status='verification' WHERE id=$1",
    )
    .bind(fixture.org_a.wave_unit_id)
    .execute(db.pool())
    .await
    .expect("make fuel fixture Candidate claimable");
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
            lease_owner: "fuel-cap-test".to_string(),
            lease_seconds: 120,
        },
    )
    .await
    .expect("claim fuel fixture Candidate")
    .expect("fuel fixture Candidate is available");
    assert_eq!(claimed.attempt.ordinal, 199);
    let attempt_id = claimed.attempt.id;
    let current_canonical_api =
        insert_current_api_endpoint_ref(db.pool(), &fixture, fixture.org_a).await;
    let current_api_endpoint_id = match &current_canonical_api.key {
        canonical_fact_refs::CanonicalFactKey::ApiEndpoint { api_endpoint_id } => *api_endpoint_id,
        _ => panic!("expected API endpoint canonical key"),
    };
    let evidence_id = insert_audit(
        db.pool(),
        fixture.operation_id,
        fixture.org_a.organization_id,
        fixture.org_a.target_id,
        "evidence",
    )
    .await;
    let typed_route_output = serde_json::json!({
        "schema": "verification.nuclei_template_replay_v1",
        "evidence_role": "proof",
        "result": {
            "target_id": fixture.org_a.target_id,
            "matched_url": "https://shared.example.test/api/current",
            "template_id": "CVE-2099-0001",
            "technique": "GOLISH-NDAY",
            "completion": "complete",
            "match_count": 1,
            "matches": [{"template_id": "CVE-2099-0001"}],
            "errors": []
        }
    })
    .to_string();
    sqlx::query(
        r#"UPDATE audit_log
              SET tool_name='verify_execute_candidate_action',
                  evidence_technique='GOLISH-NDAY',evidence_outcome='found',
                  detail=$2
            WHERE id=$1"#,
    )
    .bind(evidence_id)
    .bind(serde_json::json!({
        "organization_id": fixture.org_a.organization_id,
        "kind": "verification.nuclei_template_replay_v1",
        "raw_output": typed_route_output,
    }))
    .execute(db.pool())
    .await
    .expect("upgrade fuel-cap evidence to an exact typed replay proof");
    sqlx::query(
        "INSERT INTO candidate_attempt_actions(
             attempt_id,action_ordinal,capability_id,action_kind,canonical_args,status,
             outcome,outcome_hash,started_at,completed_at)
         VALUES($1,0,'verify.sql_injection','bounded_sql_injection_probe',
                '{\"target\":\"https://shared.example.test/login\"}'::jsonb,
                'completed','{}'::jsonb,'sha256:fuel-action',NOW(),NOW())",
    )
    .bind(attempt_id)
    .execute(db.pool())
    .await
    .expect("finish fuel fixture Candidate action");
    let lease_token = claimed
        .worker
        .lease_token
        .expect("claimed fuel lease token");
    let mut submission_tx = db.pool().begin().await.expect("begin fuel submission");
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
            attempt_id,
            candidate_plan_hash:
                "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5"
                    .to_string(),
            worker_run_id: claimed.worker.id,
            stage_execution_id,
            stage_run_unit_id,
            lease_token,
            lease_owner: "fuel-cap-test".to_string(),
            attempt_epoch: claimed.worker.attempt_epoch,
            expected_checkpoint_version: claimed.worker.checkpoint_version,
            result_json: serde_json::json!({
                "disposition": "blocked",
                "blocker_reason_code": "fuel",
                "fact_deltas": [{
                    "fact_kind": "new_surface",
                    "canonical_ref_kind": "api_endpoint",
                    "canonical_ref_id": current_api_endpoint_id,
                    "canonical_ref_version": 1,
                    "canonical_ref_hash": current_canonical_api.content_sha256.clone(),
                    "summary": "The bounded verification result changes next-wave fuel.",
                    "evidence_ids": [evidence_id]
                }]
            }),
            evidence: vec![AttemptEvidenceLink {
                evidence_id,
                role: "fact_delta".to_string(),
            }],
        },
    )
    .await
    .expect("record exact fuel fixture submission");
    submission_tx
        .commit()
        .await
        .expect("commit fuel submission");
    let mut terminal_tx = db.pool().begin().await.expect("begin fuel terminalization");
    terminalize_candidate_attempt(
        &mut terminal_tx,
        TerminalizeCandidateAttempt {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            wave_run_id: fixture.wave_run_id,
            wave_unit_id: fixture.org_a.wave_unit_id,
            organization_id: fixture.org_a.organization_id,
            candidate_id: candidate.candidate_id,
            approval_id,
            attempt_id,
            candidate_plan_hash:
                "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5"
                    .to_string(),
            expected_result_hash: submitted.attempt.result_hash.expect("fuel result hash"),
            worker_run_id: claimed.worker.id,
            stage_execution_id,
            stage_run_unit_id,
            lease_token,
            lease_owner: "fuel-cap-test".to_string(),
            attempt_epoch: claimed.worker.attempt_epoch,
            expected_checkpoint_version: claimed.worker.checkpoint_version,
        },
    )
    .await
    .expect("terminalize exact fuel fixture Attempt");
    terminal_tx
        .commit()
        .await
        .expect("commit fuel terminalization");
    let over_cap_attempt = sqlx::query(
        r#"INSERT INTO candidate_attempts (
               id,candidate_id,approval_id,operation_id,scope_snapshot_id,
               wave_run_id,wave_unit_id,organization_id,target_live_id,
               target_type_at_time,target_value_at_time,target_identity_hash,
               candidate_plan_hash,ordinal,status
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,$9,'url',
               'https://shared.example.test/login',
               'sha256:370c645a4c6a0bef678de24216f63890e7e8324ee2ce8bae74a41c78f9d88963',
               'sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5',
               200,'abandoned'
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
        over_cap_attempt.map(|_| ()),
        "23514",
        "canonical max_attempts_total hard cap",
    );
    let delta = propose_test_delta(
        db.pool(),
        &fixture,
        candidate,
        attempt_id,
        evidence_id,
        (
            "api_endpoint",
            current_api_endpoint_id,
            current_canonical_api.content_sha256,
        ),
        "new_surface",
    )
    .await;
    close_all_active_fixture_verification_units(db.pool(), &fixture, fixture.wave_run_id).await;

    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin exhausted consolidation");
    let result = attack_wave_consolidations::consolidate_attack_wave(
        &mut tx,
        attack_wave_consolidations::ConsolidateAttackWave {
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            source_wave_run_id: fixture.wave_run_id,
        },
    )
    .await
    .expect("fuel exhaustion must close with a reportable residual");
    tx.commit().await.expect("commit exhausted consolidation");

    assert_eq!(result.decision_kind, "exhausted");
    assert_eq!(result.target_wave_run_id, None);
    assert_eq!(result.accepted_fact_delta_ids, vec![delta.id]);
    assert_eq!(result.residual_risk_ids.len(), 1);
    let residual_id = result.residual_risk_ids[0];
    let residual: (String, String, i32, i32, i32, i32, String) = sqlx::query_as(
        r#"SELECT reason_code,policy_hash,wave_count,candidate_count,
                  chain_depth,attempt_count,disclosure_status
             FROM attack_residual_risks WHERE id=$1"#,
    )
    .bind(residual_id)
    .fetch_one(db.pool())
    .await
    .expect("load persisted fuel residual");
    assert_eq!(
        residual,
        (
            "max_attempts_total".to_string(),
            "sha256:66e50329b4bb217eb060bcccb38f78f4b0eafc163471bebd6554d271d1a6b326".to_string(),
            1,
            1,
            0,
            200,
            "pending".to_string(),
        )
    );
    let residual_evidence: Vec<i64> = sqlx::query_scalar(
        "SELECT evidence_id FROM attack_residual_risk_evidence
          WHERE residual_risk_id=$1 AND role='residual' ORDER BY evidence_id",
    )
    .bind(residual_id)
    .fetch_all(db.pool())
    .await
    .expect("load residual evidence membership");
    assert_eq!(residual_evidence, vec![evidence_id]);
    let delta_state: (String, Option<Uuid>) =
        sqlx::query_as("SELECT status,consumed_by_wave_run_id FROM attack_fact_deltas WHERE id=$1")
            .bind(delta.id)
            .fetch_one(db.pool())
            .await
            .expect("load exhausted FactDelta state");
    assert_eq!(delta_state, ("accepted".to_string(), None));
    let exhausted_memory_rows: (i64, i64, i64) = sqlx::query_as(
        r#"SELECT
               (SELECT COUNT(*) FROM knowledge_assertions
                 WHERE source_kind='fact_delta' AND source_id_value=$1),
               (SELECT COUNT(*) FROM knowledge_outbox_events
                 WHERE event_name='FactDeltaAccepted.v1' AND source_id_value=$1),
               (SELECT COUNT(*) FROM knowledge_projection_deliveries AS delivery
                  JOIN knowledge_outbox_events AS event ON event.event_id=delivery.event_id
                 WHERE event.event_name='FactDeltaAccepted.v1' AND event.source_id_value=$1)"#,
    )
    .bind(delta.id.to_string())
    .fetch_one(db.pool())
    .await
    .expect("count exhausted FactDelta memory promotion");
    assert_eq!(exhausted_memory_rows, (1, 1, 4));
    let wave_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM attack_wave_runs WHERE operation_id=$1")
            .bind(fixture.operation_id)
            .fetch_one(db.pool())
            .await
            .expect("count Waves after fuel exhaustion");
    assert_eq!(
        wave_count, 1,
        "fuel exhaustion must not create a target Wave"
    );
    db.stop().await;
}

struct CandidateRecoveryFixture {
    attack: AttackFixture,
    approval_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    claimed: golish_db::repo::candidate_attempts::ClaimedCandidateAttempt,
}

async fn seed_claimed_candidate_recovery_fixture(
    pool: &PgPool,
    lease_owner: &str,
) -> CandidateRecoveryFixture {
    let attack = seed_attack_fixture(pool).await;
    let candidate = seed_candidate(pool, &attack, attack.org_a).await;
    let approval_id = insert_approval(pool, &attack, candidate, attack.org_a)
        .await
        .expect("approve Candidate recovery fixture");
    sqlx::query("UPDATE attack_candidates SET disposition='approved' WHERE candidate_id=$1")
        .bind(candidate.candidate_id)
        .execute(pool)
        .await
        .expect("mark Candidate recovery fixture approved");
    sqlx::query(
        "UPDATE attack_wave_units SET review_closed=TRUE,status='verification' WHERE id=$1",
    )
    .bind(attack.org_a.wave_unit_id)
    .execute(pool)
    .await
    .expect("close Candidate recovery fixture review");
    let (stage_execution_id, stage_run_unit_id) =
        seed_verification_unit(pool, &attack, attack.org_a).await;
    let claimed = claim_next_candidate_attempt(
        pool,
        CandidateClaimQuery {
            operation_id: attack.operation_id,
            scope_snapshot_id: attack.scope_snapshot_id,
            wave_run_id: attack.wave_run_id,
            wave_unit_id: attack.org_a.wave_unit_id,
            organization_id: attack.org_a.organization_id,
            verification_stage_execution_id: stage_execution_id,
            verification_stage_run_unit_id: stage_run_unit_id,
            lease_owner: lease_owner.to_string(),
            lease_seconds: 120,
        },
    )
    .await
    .expect("claim Candidate recovery fixture")
    .expect("Candidate recovery fixture must be claimable");
    CandidateRecoveryFixture {
        attack,
        approval_id,
        stage_execution_id,
        stage_run_unit_id,
        claimed,
    }
}

async fn authorize_candidate_recovery_action(
    pool: &PgPool,
    fixture: &CandidateRecoveryFixture,
    request_id: &str,
) -> (Uuid, Uuid) {
    let action_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO candidate_attempt_actions(
               id,attempt_id,action_ordinal,capability_id,action_kind,canonical_args,status)
           VALUES($1,$2,0,'verify.sql_injection','bounded_sql_injection_probe',
                  '{"target":"https://shared.example.test/login"}'::jsonb,'planned')"#,
    )
    .bind(action_id)
    .bind(fixture.claimed.attempt.id)
    .execute(pool)
    .await
    .expect("insert planned Candidate recovery action");
    let authorization_receipt_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO candidate_action_authorization_receipts(
               id,request_id,attempt_id,action_id,lease_token,execution_deadline)
           VALUES($1,$2,$3,$4,$5,NOW()+INTERVAL '5 minutes')"#,
    )
    .bind(authorization_receipt_id)
    .bind(request_id)
    .bind(fixture.claimed.attempt.id)
    .bind(action_id)
    .bind(fixture.claimed.worker.lease_token.expect("recovery lease"))
    .execute(pool)
    .await
    .expect("persist DB-derived action authorization receipt");
    sqlx::query(
        "UPDATE candidate_attempt_actions
            SET status='started',authorization_receipt_id=$2,started_at=NOW(),updated_at=NOW()
          WHERE id=$1 AND status='planned'",
    )
    .bind(action_id)
    .bind(authorization_receipt_id)
    .execute(pool)
    .await
    .expect("start authorized Candidate recovery action");
    (action_id, authorization_receipt_id)
}

async fn expire_candidate_recovery_fixture(pool: &PgPool, fixture: &CandidateRecoveryFixture) {
    sqlx::query(
        "UPDATE stage_worker_runs
            SET lease_acquired_at=NOW()-INTERVAL '2 minutes',
                heartbeat_at=NOW()-INTERVAL '90 seconds',
                lease_expires_at=NOW()-INTERVAL '1 minute',updated_at=NOW()
          WHERE id=$1",
    )
    .bind(fixture.claimed.worker.id)
    .execute(pool)
    .await
    .expect("expire Candidate recovery Worker");
    sqlx::query(
        "UPDATE attack_execution_lanes
            SET lease_expires_at=NOW()-INTERVAL '1 minute',updated_at=NOW()
          WHERE lane_key='global:exploit' AND stage_worker_run_id=$1",
    )
    .bind(fixture.claimed.worker.id)
    .execute(pool)
    .await
    .expect("expire Candidate recovery lane");
}

async fn trigger_candidate_lane_recovery(pool: &PgPool, fixture: &CandidateRecoveryFixture) {
    let replacement = claim_next_candidate_attempt(
        pool,
        CandidateClaimQuery {
            operation_id: fixture.attack.operation_id,
            scope_snapshot_id: fixture.attack.scope_snapshot_id,
            wave_run_id: fixture.attack.wave_run_id,
            wave_unit_id: fixture.attack.org_a.wave_unit_id,
            organization_id: fixture.attack.org_a.organization_id,
            verification_stage_execution_id: fixture.stage_execution_id,
            verification_stage_run_unit_id: fixture.stage_run_unit_id,
            lease_owner: "candidate-recovery-reclaimer".to_string(),
            lease_seconds: 120,
        },
    )
    .await
    .expect("recover expired Candidate lane");
    assert!(
        replacement.is_none(),
        "an outcome-unknown Attempt must be parked, never replaced"
    );
}

#[tokio::test]
#[serial]
async fn candidate_recovery_opener_parks_crashed_started_action_with_durable_case() {
    let (mut db, _data_dir) = migrated_db("candidate_recovery_opener_started").await;
    let fixture =
        seed_claimed_candidate_recovery_fixture(db.pool(), "recovery-opener-started").await;
    let (action_id, _authorization_receipt_id) =
        authorize_candidate_recovery_action(db.pool(), &fixture, "recovery-opener-started-auth")
            .await;
    expire_candidate_recovery_fixture(db.pool(), &fixture).await;
    trigger_candidate_lane_recovery(db.pool(), &fixture).await;

    let expected_case_id = Uuid::new_v5(&action_id, b"candidate-recovery:outcome-unknown:v1");
    let (
        persisted_case_id,
        request_id,
        case_operation_id,
        case_candidate_id,
        case_worker_id,
        case_status,
        action_status,
        worker_status,
        case_count,
    ): (Uuid, String, Uuid, Uuid, Uuid, String, String, String, i64) = sqlx::query_as(
        r#"SELECT recovery.id,recovery.request_id,recovery.operation_id,
                  recovery.candidate_id,recovery.worker_run_id,recovery.status,
                  action.status,worker.status,
                  (SELECT COUNT(*) FROM candidate_recovery_cases counted
                    WHERE counted.attempt_id=attempt.id
                      AND counted.action_id=action.id
                      AND counted.case_kind='outcome_unknown')
             FROM candidate_attempts attempt
             JOIN candidate_attempt_actions action ON action.attempt_id=attempt.id
             JOIN candidate_recovery_cases recovery
               ON recovery.attempt_id=attempt.id AND recovery.action_id=action.id
              AND recovery.case_kind='outcome_unknown'
             JOIN stage_worker_runs worker ON worker.id=attempt.stage_worker_run_id
            WHERE attempt.id=$1 AND action.id=$2"#,
    )
    .bind(fixture.claimed.attempt.id)
    .bind(action_id)
    .fetch_one(db.pool())
    .await
    .expect("load opened Candidate recovery case");
    assert_eq!(persisted_case_id, expected_case_id);
    assert_eq!(
        request_id,
        format!("candidate-recovery:outcome-unknown:{action_id}")
    );
    assert_eq!(case_operation_id, fixture.attack.operation_id);
    assert_eq!(case_candidate_id, fixture.claimed.attempt.candidate_id);
    assert_eq!(case_worker_id, fixture.claimed.worker.id);
    assert_eq!(case_status, "open");
    assert_eq!(action_status, "outcome_unknown");
    assert_eq!(worker_status, "recovery_required");
    assert_eq!(case_count, 1);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn candidate_recovery_opener_repairs_already_outcome_unknown_action() {
    let (mut db, _data_dir) = migrated_db("candidate_recovery_opener_existing_unknown").await;
    let fixture =
        seed_claimed_candidate_recovery_fixture(db.pool(), "recovery-opener-existing").await;
    let (action_id, _authorization_receipt_id) =
        authorize_candidate_recovery_action(db.pool(), &fixture, "recovery-opener-existing-auth")
            .await;
    sqlx::query(
        "UPDATE candidate_attempt_actions
            SET status='outcome_unknown',error_code='transport_response_lost',
                completed_at=NOW(),updated_at=NOW()
          WHERE id=$1 AND status='started'",
    )
    .bind(action_id)
    .execute(db.pool())
    .await
    .expect("persist outcome-unknown action before recovery opener");
    expire_candidate_recovery_fixture(db.pool(), &fixture).await;
    trigger_candidate_lane_recovery(db.pool(), &fixture).await;

    let (case_id, reason_code, action_status, worker_status, case_count): (
        Uuid,
        String,
        String,
        String,
        i64,
    ) = sqlx::query_as(
        r#"SELECT recovery.id,recovery.reason_code,action.status,worker.status,
                  (SELECT COUNT(*) FROM candidate_recovery_cases counted
                    WHERE counted.attempt_id=attempt.id
                      AND counted.action_id=action.id
                      AND counted.case_kind='outcome_unknown')
             FROM candidate_attempts attempt
             JOIN candidate_attempt_actions action ON action.attempt_id=attempt.id
             JOIN candidate_recovery_cases recovery
               ON recovery.attempt_id=attempt.id AND recovery.action_id=action.id
              AND recovery.case_kind='outcome_unknown'
             JOIN stage_worker_runs worker ON worker.id=attempt.stage_worker_run_id
            WHERE attempt.id=$1 AND action.id=$2"#,
    )
    .bind(fixture.claimed.attempt.id)
    .bind(action_id)
    .fetch_one(db.pool())
    .await
    .expect("load repaired Candidate recovery case");
    assert_eq!(
        case_id,
        Uuid::new_v5(&action_id, b"candidate-recovery:outcome-unknown:v1")
    );
    assert_eq!(reason_code, "transport_response_lost");
    assert_eq!(action_status, "outcome_unknown");
    assert_eq!(worker_status, "recovery_required");
    assert_eq!(case_count, 1);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn candidate_terminal_intent_schema_is_relational_and_immutable() {
    let (mut db, _data_dir) = migrated_db("candidate_terminal_intent_schema").await;
    for table in [
        "candidate_action_authorization_receipts",
        "candidate_attempt_terminal_intents",
        "candidate_attempt_terminal_barriers",
        "candidate_attempt_terminal_receipts",
        "candidate_recovery_cases",
        "candidate_recovery_evidence",
    ] {
        let exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
            .bind(format!("public.{table}"))
            .fetch_one(db.pool())
            .await
            .expect("inspect Candidate recovery table");
        assert!(exists, "missing Candidate recovery table {table}");
    }

    let approval_start_before: (String, String) = sqlx::query_as(
        r#"SELECT is_nullable,data_type
             FROM information_schema.columns
            WHERE table_schema='public' AND table_name='attack_candidate_approvals'
              AND column_name='start_before'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect approval start-before column");
    assert_eq!(
        approval_start_before,
        ("NO".to_string(), "timestamp with time zone".to_string())
    );

    let attempt_status_constraint: String = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(oid)
             FROM pg_constraint
            WHERE conrelid='candidate_attempts'::regclass
              AND conname='candidate_attempts_status_check'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect CandidateAttempt status constraint");
    assert!(
        attempt_status_constraint.contains("terminalization_pending"),
        "terminal intent must have an explicit durable Attempt state: {attempt_status_constraint}"
    );

    let immutable_triggers: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM pg_trigger
            WHERE NOT tgisinternal AND tgname IN (
                'candidate_action_authorization_receipts_immutable',
                'candidate_attempt_terminal_intents_immutable',
                'candidate_attempt_terminal_barriers_immutable',
                'candidate_attempt_terminal_receipts_immutable',
                'candidate_recovery_case_transition_guard',
                'candidate_recovery_evidence_immutable'
            )"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect Candidate recovery immutability triggers");
    assert_eq!(immutable_triggers, 6);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn candidate_approval_start_before_authorizes_one_frozen_action_receipt() {
    let (mut db, _data_dir) = migrated_db("candidate_approval_start_before").await;
    let fixture =
        seed_claimed_candidate_recovery_fixture(db.pool(), "authorization-receipt-test").await;
    let (expires_at, start_before): (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) =
        sqlx::query_as("SELECT expires_at,start_before FROM attack_candidate_approvals WHERE id=$1")
            .bind(fixture.approval_id)
            .fetch_one(db.pool())
            .await
            .expect("load compatibility approval deadlines");
    assert_eq!(
        start_before, expires_at,
        "legacy expires_at must backfill start_before"
    );
    let shifted_start_before = sqlx::query(
        "UPDATE attack_candidate_approvals
            SET start_before=start_before-INTERVAL '1 second' WHERE id=$1",
    )
    .bind(fixture.approval_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        shifted_start_before.map(|_| ()),
        "23514",
        "mutated approval start-before",
    );

    let (action_id, receipt_id) =
        authorize_candidate_recovery_action(db.pool(), &fixture, "authorization-receipt-request")
            .await;
    type AuthorizationProjection = (
        Uuid,
        i64,
        String,
        String,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
        String,
    );
    let receipt: AuthorizationProjection = sqlx::query_as(
        r#"SELECT approval_id,decision_version,candidate_plan_hash,scope_hash,
                  authorized_at,start_before,receipt_hash
             FROM candidate_action_authorization_receipts WHERE id=$1"#,
    )
    .bind(receipt_id)
    .fetch_one(db.pool())
    .await
    .expect("load frozen action authorization receipt");
    assert_eq!(receipt.0, fixture.approval_id);
    assert_eq!(receipt.1, 1);
    assert_eq!(
        receipt.2,
        "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5"
    );
    assert_eq!(receipt.3, "sha256:scope");
    assert!(receipt.4 <= receipt.5);
    assert!(receipt.6.starts_with("sha256:"));

    let changed = sqlx::query(
        "UPDATE candidate_action_authorization_receipts
            SET candidate_plan_hash='sha256:tampered' WHERE id=$1",
    )
    .bind(receipt_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        changed.map(|_| ()),
        "23514",
        "mutated action authorization receipt",
    );
    let deleted = sqlx::query("DELETE FROM candidate_action_authorization_receipts WHERE id=$1")
        .bind(receipt_id)
        .execute(db.pool())
        .await;
    assert_sqlstate(
        deleted.map(|_| ()),
        "23514",
        "deleted action authorization receipt",
    );
    let action_receipt: Option<Uuid> = sqlx::query_scalar(
        "SELECT authorization_receipt_id FROM candidate_attempt_actions WHERE id=$1",
    )
    .bind(action_id)
    .fetch_one(db.pool())
    .await
    .expect("load action authorization back-reference");
    assert_eq!(action_receipt, Some(receipt_id));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn candidate_action_begin_commits_receipt_before_started_and_finish_requires_it() {
    let (mut db, _data_dir) = migrated_db("candidate_action_begin_receipt").await;
    let fixture =
        seed_claimed_candidate_recovery_fixture(db.pool(), "candidate-action-receipt-test").await;
    let lease_token = fixture
        .claimed
        .worker
        .lease_token
        .expect("Candidate action receipt lease");
    let started = begin_candidate_action(
        db.pool(),
        BeginCandidateAction {
            operation_id: fixture.attack.operation_id,
            stage_execution_id: fixture.stage_execution_id,
            stage_run_unit_id: fixture.stage_run_unit_id,
            organization_id: fixture.attack.org_a.organization_id,
            worker_run_id: fixture.claimed.worker.id,
            lease_token,
            attempt_epoch: fixture.claimed.worker.attempt_epoch,
            candidate_id: fixture.claimed.attempt.candidate_id,
            approval_id: fixture.approval_id,
            attempt_id: fixture.claimed.attempt.id,
            candidate_plan_hash: fixture.claimed.attempt.candidate_plan_hash.clone(),
            workspace_path_sha256: "sha256:attack-v2".to_string(),
            action_ordinal: 0,
        },
    )
    .await
    .expect("begin Candidate action with durable authorization receipt");
    let action_id = match started {
        CandidateActionStart::Authorized(action) => action.action_id,
        other => panic!("expected newly authorized Candidate action, got {other:?}"),
    };
    type ReceiptProjection = (
        String,
        Uuid,
        Uuid,
        Uuid,
        i64,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
    );
    let receipt: ReceiptProjection = sqlx::query_as(
        r#"SELECT action.status,action.authorization_receipt_id,
                  receipt.worker_run_id,receipt.lease_token,receipt.attempt_epoch,
                  receipt.authorized_at,receipt.execution_deadline
             FROM candidate_attempt_actions action
             JOIN candidate_action_authorization_receipts receipt
               ON receipt.id=action.authorization_receipt_id
            WHERE action.id=$1"#,
    )
    .bind(action_id)
    .fetch_one(db.pool())
    .await
    .expect("load begin-action authorization receipt");
    assert_eq!(receipt.0, "started");
    assert_eq!(receipt.2, fixture.claimed.worker.id);
    assert_eq!(receipt.3, lease_token);
    assert_eq!(receipt.4, fixture.claimed.worker.attempt_epoch);
    assert!(receipt.6 > receipt.5);

    finish_candidate_action(
        db.pool(),
        FinishCandidateAction {
            operation_id: fixture.attack.operation_id,
            stage_execution_id: fixture.stage_execution_id,
            stage_run_unit_id: fixture.stage_run_unit_id,
            organization_id: fixture.attack.org_a.organization_id,
            worker_run_id: fixture.claimed.worker.id,
            lease_token,
            attempt_epoch: fixture.claimed.worker.attempt_epoch,
            candidate_id: fixture.claimed.attempt.candidate_id,
            approval_id: fixture.approval_id,
            attempt_id: fixture.claimed.attempt.id,
            candidate_plan_hash: fixture.claimed.attempt.candidate_plan_hash.clone(),
            action_id,
            success: true,
            outcome: serde_json::json!({"bounded_probe": "completed"}),
            error_code: None,
        },
    )
    .await
    .expect("finish receipt-backed Candidate action");
    let status: String =
        sqlx::query_scalar("SELECT status FROM candidate_attempt_actions WHERE id=$1")
            .bind(action_id)
            .fetch_one(db.pool())
            .await
            .expect("load receipt-backed action terminal state");
    assert_eq!(status, "completed");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn candidate_action_finish_rejects_started_row_without_authorization_receipt() {
    let (mut db, _data_dir) = migrated_db("candidate_action_finish_requires_receipt").await;
    let fixture =
        seed_claimed_candidate_recovery_fixture(db.pool(), "candidate-action-no-receipt-test")
            .await;
    let action_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO candidate_attempt_actions(
               id,attempt_id,action_ordinal,capability_id,action_kind,canonical_args,status,
               started_at)
           VALUES($1,$2,0,'verify.sql_injection','bounded_sql_injection_probe',
                  '{"target":"https://shared.example.test/login"}'::jsonb,'started',NOW())"#,
    )
    .bind(action_id)
    .bind(fixture.claimed.attempt.id)
    .execute(db.pool())
    .await
    .expect("seed legacy started action without receipt");
    let result = finish_candidate_action(
        db.pool(),
        FinishCandidateAction {
            operation_id: fixture.attack.operation_id,
            stage_execution_id: fixture.stage_execution_id,
            stage_run_unit_id: fixture.stage_run_unit_id,
            organization_id: fixture.attack.org_a.organization_id,
            worker_run_id: fixture.claimed.worker.id,
            lease_token: fixture
                .claimed
                .worker
                .lease_token
                .expect("receiptless action lease"),
            attempt_epoch: fixture.claimed.worker.attempt_epoch,
            candidate_id: fixture.claimed.attempt.candidate_id,
            approval_id: fixture.approval_id,
            attempt_id: fixture.claimed.attempt.id,
            candidate_plan_hash: fixture.claimed.attempt.candidate_plan_hash.clone(),
            action_id,
            success: true,
            outcome: serde_json::json!({"bounded_probe": "completed"}),
            error_code: None,
        },
    )
    .await;
    assert!(
        result.is_err(),
        "receiptless started action must not reach the finish path"
    );
    let status: String =
        sqlx::query_scalar("SELECT status FROM candidate_attempt_actions WHERE id=$1")
            .bind(action_id)
            .fetch_one(db.pool())
            .await
            .expect("load receiptless action state");
    assert_eq!(status, "started");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn candidate_terminal_intent_and_barrier_freeze_exact_worker_checkpoint() {
    let (mut db, _data_dir) = migrated_db("candidate_terminal_intent_barrier").await;
    let fixture = seed_claimed_candidate_recovery_fixture(db.pool(), "terminal-intent-test").await;
    let (action_id, _authorization_receipt_id) =
        authorize_candidate_recovery_action(db.pool(), &fixture, "terminal-action-auth").await;
    sqlx::query(
        "UPDATE candidate_attempt_actions
            SET status='completed',outcome='{}'::jsonb,outcome_hash='sha256:action-result',
                completed_at=NOW(),updated_at=NOW()
          WHERE id=$1 AND status='started'",
    )
    .bind(action_id)
    .execute(db.pool())
    .await
    .expect("complete authorized Candidate action");

    let tool_call_record_id = Uuid::new_v4();
    let lease_token = fixture
        .claimed
        .worker
        .lease_token
        .expect("terminal intent lease");
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,status,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token)
           VALUES($1,'terminal-intent-submit',$2,$3,'pentester','submit_candidate_attempt',
                  '{}'::jsonb,'running',$3,$4,$5,$6,$7,$8,$9)"#,
    )
    .bind(tool_call_record_id)
    .bind(fixture.attack.session_id)
    .bind(fixture.attack.operation_id)
    .bind(fixture.stage_execution_id)
    .bind(fixture.stage_run_unit_id)
    .bind(fixture.claimed.worker.id)
    .bind(fixture.attack.org_a.organization_id)
    .bind(fixture.claimed.worker.attempt_epoch)
    .bind(lease_token)
    .execute(db.pool())
    .await
    .expect("insert terminal-intent submit tool call");
    sqlx::query(
        "UPDATE stage_worker_runs
            SET active_tool_call_id=$2,active_tool_started_at=NOW(),updated_at=NOW()
          WHERE id=$1",
    )
    .bind(fixture.claimed.worker.id)
    .bind(tool_call_record_id)
    .execute(db.pool())
    .await
    .expect("mark terminal-intent tool active");

    let intent_id = Uuid::new_v4();
    let submitted_result = serde_json::json!({
        "disposition": "blocked",
        "blocker_reason_code": "bounded_probe_inconclusive",
        "proof_evidence_ids": [],
        "refutation_evidence_ids": [],
        "blocker_evidence_ids": [],
        "fact_deltas": []
    });
    let tool_result_text = serde_json::to_string(&serde_json::json!({
        "ok": true,
        "terminalization_pending": true,
        "attempt_id": fixture.claimed.attempt.id,
    }))
    .expect("serialize deterministic Candidate tool result");
    let mut intent_tx = db
        .pool()
        .begin()
        .await
        .expect("begin terminal intent commit");
    sqlx::query(
        r#"INSERT INTO candidate_attempt_terminal_intents(
               id,request_id,attempt_id,tool_call_record_id,lease_token,
               disposition,submitted_result,tool_result_text)
           VALUES($1,'terminal-intent-request',$2,$3,$4,'blocked',$5,$6)"#,
    )
    .bind(intent_id)
    .bind(fixture.claimed.attempt.id)
    .bind(tool_call_record_id)
    .bind(lease_token)
    .bind(&submitted_result)
    .bind(&tool_result_text)
    .execute(&mut *intent_tx)
    .await
    .expect("persist immutable Candidate terminal intent");
    sqlx::query(
        "UPDATE candidate_attempts
            SET status='terminalization_pending',row_version=row_version+1,updated_at=NOW()
          WHERE id=$1 AND status='running'",
    )
    .bind(fixture.claimed.attempt.id)
    .execute(&mut *intent_tx)
    .await
    .expect("derive durable terminalization-pending Attempt state");
    intent_tx
        .commit()
        .await
        .expect("commit terminal intent and Attempt state");

    let extra_action = sqlx::query(
        r#"INSERT INTO candidate_attempt_actions(
               attempt_id,action_ordinal,capability_id,action_kind,canonical_args,status)
           VALUES($1,1,'verify.sql_injection','bounded_sql_injection_probe','{}','planned')"#,
    )
    .bind(fixture.claimed.attempt.id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        extra_action.map(|_| ()),
        "23514",
        "action after terminal intent",
    );

    sqlx::query("UPDATE tool_calls SET status='finished',result=$2,updated_at=NOW() WHERE id=$1")
        .bind(tool_call_record_id)
        .bind(&tool_result_text)
        .execute(db.pool())
        .await
        .expect("finish terminal-intent tool call");
    let checkpoint_version: i64 = sqlx::query_scalar(
        r#"UPDATE stage_worker_runs
              SET active_tool_call_id=NULL,active_tool_started_at=NULL,
                  checkpoint=jsonb_build_object(
                      'tool_call_id',$2::TEXT,'tool_result',$3::TEXT
                  ),checkpoint_version=checkpoint_version+1,updated_at=NOW()
            WHERE id=$1
        RETURNING checkpoint_version"#,
    )
    .bind(fixture.claimed.worker.id)
    .bind(tool_call_record_id)
    .bind(&tool_result_text)
    .fetch_one(db.pool())
    .await
    .expect("checkpoint exact terminal ToolCall/ToolResult");
    let barrier_id = Uuid::new_v4();
    let barrier_hash: String = sqlx::query_scalar(
        r#"INSERT INTO candidate_attempt_terminal_barriers(
               id,request_id,intent_id,checkpoint_version)
           VALUES($1,'terminal-barrier-request',$2,$3)
           RETURNING barrier_hash"#,
    )
    .bind(barrier_id)
    .bind(intent_id)
    .bind(checkpoint_version)
    .fetch_one(db.pool())
    .await
    .expect("persist exact terminal barrier");
    assert!(barrier_hash.starts_with("sha256:"));

    let premature_receipt = sqlx::query(
        r#"INSERT INTO candidate_attempt_terminal_receipts(
               id,request_id,intent_id,barrier_id)
           VALUES($1,'premature-terminal-receipt',$2,$3)"#,
    )
    .bind(Uuid::new_v4())
    .bind(intent_id)
    .bind(barrier_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        premature_receipt.map(|_| ()),
        "23514",
        "terminal receipt before canonical terminal state",
    );

    let intent_changed = sqlx::query(
        "UPDATE candidate_attempt_terminal_intents
            SET disposition='refuted' WHERE id=$1",
    )
    .bind(intent_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        intent_changed.map(|_| ()),
        "23514",
        "mutated terminal intent",
    );
    let barrier_deleted =
        sqlx::query("DELETE FROM candidate_attempt_terminal_barriers WHERE id=$1")
            .bind(barrier_id)
            .execute(db.pool())
            .await;
    assert_sqlstate(
        barrier_deleted.map(|_| ()),
        "23514",
        "deleted terminal barrier",
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn candidate_recovery_case_rejects_cross_owner_evidence_and_unknown_decisions() {
    let (mut db, _data_dir) = migrated_db("candidate_recovery_case_authority").await;
    let fixture =
        seed_claimed_candidate_recovery_fixture(db.pool(), "candidate-recovery-case-test").await;
    let (action_id, _authorization_receipt_id) =
        authorize_candidate_recovery_action(db.pool(), &fixture, "recovery-action-auth").await;
    sqlx::query(
        "UPDATE candidate_attempt_actions
            SET status='outcome_unknown',error_code='transport_response_lost',updated_at=NOW()
          WHERE id=$1 AND status='started'",
    )
    .bind(action_id)
    .execute(db.pool())
    .await
    .expect("mark action outcome unknown");
    let recovery_case_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO candidate_recovery_cases(
               id,request_id,attempt_id,action_id,case_kind,reason_code)
           VALUES($1,'candidate-recovery-open',$2,$3,'outcome_unknown',
                  'transport_response_lost')"#,
    )
    .bind(recovery_case_id)
    .bind(fixture.claimed.attempt.id)
    .bind(action_id)
    .execute(db.pool())
    .await
    .expect("open exact Candidate recovery case");

    let foreign_evidence_id = insert_audit(
        db.pool(),
        fixture.attack.operation_id,
        fixture.attack.org_b.organization_id,
        fixture.attack.org_b.target_id,
        "evidence",
    )
    .await;
    let foreign = sqlx::query(
        "INSERT INTO candidate_recovery_evidence(recovery_case_id,evidence_id,role)
         VALUES($1,$2,'external_result')",
    )
    .bind(recovery_case_id)
    .bind(foreign_evidence_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        foreign.map(|_| ()),
        "P0001",
        "cross-owner recovery evidence",
    );

    let unknown_decision = sqlx::query(
        "UPDATE candidate_recovery_cases
            SET status='decision_recorded',resolution_kind='rewrite_frozen_plan',
                resolution_request_id='illegal-recovery-decision',row_version=row_version+1
          WHERE id=$1 AND status='open'",
    )
    .bind(recovery_case_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        unknown_decision.map(|_| ()),
        "23514",
        "unknown recovery decision",
    );

    let exact_evidence_id = insert_audit(
        db.pool(),
        fixture.attack.operation_id,
        fixture.attack.org_a.organization_id,
        fixture.attack.org_a.target_id,
        "evidence",
    )
    .await;
    let operator_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM operator_principals WHERE principal_kind='local_operator' AND active",
    )
    .fetch_one(db.pool())
    .await
    .expect("load trusted recovery operator");
    let decision = ResolveCandidateRecovery {
        request_id: "exact-recovery-decision".to_string(),
        operation_id: fixture.attack.operation_id,
        recovery_case_id,
        expected_row_version: 0,
        expected_attempt_row_version: fixture.claimed.attempt.row_version,
        resolved_by: operator_id,
        resolution: CandidateRecoveryResolution::AcceptExternalResultWithExactEvidence,
        evidence_ids: vec![exact_evidence_id],
    };
    let resolved = resolve_candidate_recovery(db.pool(), decision.clone())
        .await
        .expect("record one legal recovery decision by CAS");
    assert!(!resolved.replayed);
    assert_eq!(resolved.recovery_case.status, "decision_recorded");
    assert_eq!(resolved.recovery_case.row_version, 1);
    let replay = resolve_candidate_recovery(db.pool(), decision)
        .await
        .expect("replay exact Candidate recovery decision");
    assert!(replay.replayed);
    assert_eq!(replay.recovery_case.id, resolved.recovery_case.id);
    assert!(
        resolve_candidate_recovery(
            db.pool(),
            ResolveCandidateRecovery {
                request_id: "drifted-recovery-decision".to_string(),
                operation_id: fixture.attack.operation_id,
                recovery_case_id,
                expected_row_version: 0,
                expected_attempt_row_version: fixture.claimed.attempt.row_version,
                resolved_by: operator_id,
                resolution: CandidateRecoveryResolution::TerminalizeBlockedOutcomeUnknown,
                evidence_ids: vec![],
            },
        )
        .await
        .is_err(),
        "a recorded recovery decision must reject semantic replay drift"
    );
    let identity_change = sqlx::query(
        "UPDATE candidate_recovery_cases
            SET candidate_plan_hash='sha256:tampered' WHERE id=$1",
    )
    .bind(recovery_case_id)
    .execute(db.pool())
    .await;
    assert_sqlstate(
        identity_change.map(|_| ()),
        "23514",
        "mutated recovery identity",
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn candidate_recovery_repo_replays_intent_barrier_and_server_terminal_receipt() {
    let (mut db, _data_dir) = migrated_db("candidate_recovery_repo_protocol").await;
    let fixture =
        seed_claimed_candidate_recovery_fixture(db.pool(), "candidate-recovery-repo").await;
    sqlx::query(
        "UPDATE stage_runs
            SET status='completed',completed_at=NOW()
          WHERE operation_id=$1 AND id<>$2 AND status='started'",
    )
    .bind(fixture.attack.operation_id)
    .bind(fixture.stage_execution_id)
    .execute(db.pool())
    .await
    .expect("leave one active Verification StageExecution for runtime checkpointing");
    sqlx::query("UPDATE operation_state SET current_stage='verification' WHERE operation_id=$1")
        .bind(fixture.attack.operation_id)
        .execute(db.pool())
        .await
        .expect("advance Candidate recovery fixture runtime cursor");
    let (action_id, _authorization_receipt_id) =
        authorize_candidate_recovery_action(db.pool(), &fixture, "candidate-recovery-repo-auth")
            .await;
    sqlx::query(
        "UPDATE candidate_attempt_actions
            SET status='completed',outcome='{}'::jsonb,outcome_hash='sha256:action-result',
                completed_at=NOW(),updated_at=NOW()
          WHERE id=$1 AND status='started'",
    )
    .bind(action_id)
    .execute(db.pool())
    .await
    .expect("complete Candidate recovery action");

    let tool_call_record_id = Uuid::new_v4();
    let lease_token = fixture
        .claimed
        .worker
        .lease_token
        .expect("Candidate recovery repo lease");
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,status,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token)
           VALUES($1,'candidate-recovery-repo-submit',$2,$3,'pentester',
                  'submit_candidate_attempt','{}'::jsonb,'running',$3,$4,$5,$6,$7,$8,$9)"#,
    )
    .bind(tool_call_record_id)
    .bind(fixture.attack.session_id)
    .bind(fixture.attack.operation_id)
    .bind(fixture.stage_execution_id)
    .bind(fixture.stage_run_unit_id)
    .bind(fixture.claimed.worker.id)
    .bind(fixture.attack.org_a.organization_id)
    .bind(fixture.claimed.worker.attempt_epoch)
    .bind(lease_token)
    .execute(db.pool())
    .await
    .expect("insert Candidate recovery repo submit tool call");
    sqlx::query(
        "UPDATE stage_worker_runs
            SET active_tool_call_id=$2,active_tool_started_at=NOW(),updated_at=NOW()
          WHERE id=$1",
    )
    .bind(fixture.claimed.worker.id)
    .bind(tool_call_record_id)
    .execute(db.pool())
    .await
    .expect("mark Candidate recovery repo tool active");

    let submitted_result = serde_json::json!({
        "disposition": "blocked",
        "blocker_reason_code": "bounded_probe_inconclusive",
        "proof_evidence_ids": [],
        "refutation_evidence_ids": [],
        "blocker_evidence_ids": [],
        "fact_deltas": []
    });
    let tool_result_text = serde_json::to_string(&serde_json::json!({
        "ok": true,
        "terminalization_pending": true,
        "attempt_id": fixture.claimed.attempt.id,
    }))
    .expect("serialize Candidate recovery repo ToolResult");
    let intent_command = RecordCandidateTerminalIntent {
        request_id: "candidate-recovery-repo-intent".to_string(),
        operation_id: fixture.attack.operation_id,
        organization_id: fixture.attack.org_a.organization_id,
        candidate_id: fixture.claimed.attempt.candidate_id,
        approval_id: fixture.approval_id,
        attempt_id: fixture.claimed.attempt.id,
        candidate_plan_hash: fixture.claimed.attempt.candidate_plan_hash.clone(),
        worker_run_id: fixture.claimed.worker.id,
        lease_token,
        attempt_epoch: fixture.claimed.worker.attempt_epoch,
        tool_call_record_id,
        disposition: "blocked".to_string(),
        submitted_result: submitted_result.clone(),
        evidence: vec![],
        tool_result_text: tool_result_text.clone(),
    };
    let mut intent_tx = db.pool().begin().await.expect("begin repo terminal intent");
    let intent = record_candidate_terminal_intent(&mut intent_tx, intent_command.clone())
        .await
        .expect("record repo terminal intent");
    intent_tx
        .commit()
        .await
        .expect("commit repo terminal intent");
    assert!(!intent.replayed);

    let mut replay_tx = db.pool().begin().await.expect("begin intent replay");
    let replay = record_candidate_terminal_intent(&mut replay_tx, intent_command)
        .await
        .expect("replay exact repo terminal intent");
    replay_tx.commit().await.expect("commit intent replay");
    assert!(replay.replayed);
    assert_eq!(replay.intent.id, intent.intent.id);
    let pending = next_candidate_terminal_intent(db.pool(), fixture.attack.operation_id)
        .await
        .expect("query pre-barrier terminal queue")
        .expect("pending terminal intent must block the queue");
    assert_eq!(pending.intent_id, intent.intent.id);
    assert_eq!(pending.barrier_id, None);

    sqlx::query("UPDATE tool_calls SET status='finished',result=$2,updated_at=NOW() WHERE id=$1")
        .bind(tool_call_record_id)
        .bind(&tool_result_text)
        .execute(db.pool())
        .await
        .expect("finish Candidate recovery repo tool call");
    sqlx::query(
        "UPDATE stage_worker_runs
            SET active_tool_call_id=NULL,active_tool_started_at=NULL,updated_at=NOW()
          WHERE id=$1",
    )
    .bind(fixture.claimed.worker.id)
    .execute(db.pool())
    .await
    .expect("finish Candidate recovery repo Worker tool fence");
    let mut chain: serde_json::Value =
        sqlx::query_scalar("SELECT chain FROM message_chains WHERE id=$1")
            .bind(
                fixture
                    .claimed
                    .worker
                    .message_chain_id
                    .expect("Candidate recovery repo chain"),
            )
            .fetch_one(db.pool())
            .await
            .expect("load Candidate recovery repo chain");
    chain
        .as_array_mut()
        .expect("Candidate recovery repo chain must be an array")
        .push(serde_json::json!({
            "role": "tool",
            "tool_call_id": tool_call_record_id,
            "content": tool_result_text,
        }));
    let checkpoint = serde_json::json!({
        "tool_call_id": tool_call_record_id,
        "tool_result": tool_result_text,
    });
    let barrier_command = CheckpointCandidateTerminalBarrier {
        request_id: "candidate-recovery-repo-barrier".to_string(),
        intent_id: intent.intent.id,
        expected_intent_hash: intent.intent.intent_hash.clone(),
        checkpoint: runtime_memory_tx::CheckpointBoundWorkerChainRow {
            fence: runtime_memory_tx::RuntimeMemoryTxFence {
                operation_id: fixture.attack.operation_id,
                stage_execution_id: fixture.stage_execution_id,
                stage_run_unit_id: fixture.stage_run_unit_id,
                worker_run_id: fixture.claimed.worker.id,
                lease_token,
                attempt_epoch: fixture.claimed.worker.attempt_epoch,
                expected_checkpoint_version: fixture.claimed.worker.checkpoint_version,
            },
            message_chain_id: fixture
                .claimed
                .worker
                .message_chain_id
                .expect("Candidate recovery repo chain"),
            chain: chain.clone(),
            checkpoint: checkpoint.clone(),
        },
    };
    let barrier = checkpoint_candidate_terminal_barrier(db.pool(), barrier_command.clone())
        .await
        .expect("atomically checkpoint Candidate ToolResult and terminal barrier");
    assert!(!barrier.replayed);
    assert_eq!(
        barrier.barrier.checkpoint_version,
        fixture.claimed.worker.checkpoint_version + 1
    );
    let barrier_replay = checkpoint_candidate_terminal_barrier(db.pool(), barrier_command)
        .await
        .expect("replay atomic Candidate terminal checkpoint barrier");
    assert!(barrier_replay.replayed);
    assert_eq!(barrier_replay.barrier.id, barrier.barrier.id);

    let ready = next_candidate_terminal_intent(db.pool(), fixture.attack.operation_id)
        .await
        .expect("query barrier-ready terminal intent")
        .expect("barrier-ready terminal intent");
    assert_eq!(ready.intent_id, intent.intent.id);
    assert_eq!(ready.barrier_id, Some(barrier.barrier.id));
    let terminal_command = TerminalizeCandidateTerminalIntent {
        request_id: "candidate-recovery-repo-terminal".to_string(),
        operation_id: fixture.attack.operation_id,
        intent_id: intent.intent.id,
        barrier_id: barrier.barrier.id,
    };
    let terminal = terminalize_candidate_terminal_intent(db.pool(), terminal_command.clone())
        .await
        .expect("server-authority terminalize Candidate intent");
    assert!(!terminal.replayed);
    assert_eq!(terminal.receipt.disposition, "blocked");
    let terminal_replay = terminalize_candidate_terminal_intent(db.pool(), terminal_command)
        .await
        .expect("replay server-authority terminal receipt");
    assert!(terminal_replay.replayed);
    assert_eq!(terminal_replay.receipt.id, terminal.receipt.id);

    let (attempt_status, worker_status, lane_owner): (String, String, Option<Uuid>) =
        sqlx::query_as(
            r#"SELECT attempt.status,worker.status,lane.stage_worker_run_id
                 FROM candidate_attempts attempt
                 JOIN stage_worker_runs worker ON worker.id=attempt.stage_worker_run_id
                 CROSS JOIN attack_execution_lanes lane
                WHERE attempt.id=$1 AND lane.lane_key='global:exploit'"#,
        )
        .bind(fixture.claimed.attempt.id)
        .fetch_one(db.pool())
        .await
        .expect("load terminal Candidate recovery repo state");
    assert_eq!(attempt_status, "blocked");
    assert_eq!(worker_status, "passed");
    assert_eq!(lane_owner, None);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn candidate_recovery_decision_converges_outcome_unknown_to_blocked_terminal_truth() {
    let (mut db, _data_dir) = migrated_db("candidate_recovery_converge_blocked").await;
    let fixture =
        seed_claimed_candidate_recovery_fixture(db.pool(), "recovery-converge-blocked").await;
    let (action_id, _authorization_receipt_id) =
        authorize_candidate_recovery_action(db.pool(), &fixture, "recovery-converge-blocked-auth")
            .await;
    expire_candidate_recovery_fixture(db.pool(), &fixture).await;
    trigger_candidate_lane_recovery(db.pool(), &fixture).await;

    let recovery_case_id = Uuid::new_v5(&action_id, b"candidate-recovery:outcome-unknown:v1");
    let (case_row_version, attempt_row_version, case_status): (i64, i64, String) = sqlx::query_as(
        r#"SELECT recovery.row_version,recovery.attempt_row_version,recovery.status
                 FROM candidate_recovery_cases recovery
                WHERE recovery.id=$1 AND recovery.operation_id=$2"#,
    )
    .bind(recovery_case_id)
    .bind(fixture.attack.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("load open outcome-unknown recovery case");
    assert_eq!(case_status, "open");
    let operator_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM operator_principals WHERE principal_kind='local_operator' AND active",
    )
    .fetch_one(db.pool())
    .await
    .expect("load trusted recovery operator");
    let recorded = resolve_candidate_recovery(
        db.pool(),
        ResolveCandidateRecovery {
            request_id: "recovery-converge-blocked-decision".to_string(),
            operation_id: fixture.attack.operation_id,
            recovery_case_id,
            expected_row_version: case_row_version,
            expected_attempt_row_version: attempt_row_version,
            resolved_by: operator_id,
            resolution: CandidateRecoveryResolution::TerminalizeBlockedOutcomeUnknown,
            evidence_ids: vec![],
        },
    )
    .await
    .expect("record blocked outcome-unknown recovery decision");
    assert_eq!(recorded.recovery_case.status, "decision_recorded");

    let converged =
        converge_candidate_recovery(db.pool(), fixture.attack.operation_id, recovery_case_id)
            .await
            .expect("converge blocked outcome-unknown recovery decision");
    assert!(!converged.replayed);
    assert!(!converged.candidate_reopened);
    assert_eq!(converged.recovery_case.status, "resolved");
    assert_eq!(converged.recovery_case.row_version, case_row_version + 2);
    let terminalized = converged
        .terminalized
        .expect("blocked recovery must return terminal truth");
    assert_eq!(terminalized.attempt_id, fixture.claimed.attempt.id);
    assert_eq!(terminalized.status, "blocked");

    type RecoveryTerminalProjection = (
        String,
        Option<String>,
        String,
        String,
        Option<Uuid>,
        String,
        Option<String>,
        Option<Uuid>,
    );
    let terminal: RecoveryTerminalProjection = sqlx::query_as(
        r#"SELECT attempt.status,attempt.result_json->>'blocker_reason_code',
                  candidate.disposition,worker.status,candidate.terminal_attempt_id,
                  recovery.status,recovery.resolution_kind,lane.stage_worker_run_id
             FROM candidate_attempts attempt
             JOIN attack_candidates candidate ON candidate.candidate_id=attempt.candidate_id
             JOIN stage_worker_runs worker ON worker.id=attempt.stage_worker_run_id
             JOIN candidate_recovery_cases recovery
               ON recovery.attempt_id=attempt.id AND recovery.id=$2
             CROSS JOIN attack_execution_lanes lane
            WHERE attempt.id=$1 AND lane.lane_key='global:exploit'"#,
    )
    .bind(fixture.claimed.attempt.id)
    .bind(recovery_case_id)
    .fetch_one(db.pool())
    .await
    .expect("load converged recovery terminal projection");
    assert_eq!(terminal.0, "blocked");
    assert_eq!(terminal.1.as_deref(), Some("operator_outcome_unknown"));
    assert_eq!(terminal.2, "blocked");
    assert_eq!(terminal.3, "exhausted");
    assert_eq!(terminal.4, Some(fixture.claimed.attempt.id));
    assert_eq!(terminal.5, "resolved");
    assert_eq!(
        terminal.6.as_deref(),
        Some("terminalize_blocked_outcome_unknown")
    );
    assert_eq!(terminal.7, None);

    let replay =
        converge_candidate_recovery(db.pool(), fixture.attack.operation_id, recovery_case_id)
            .await
            .expect("replay converged Candidate recovery");
    assert!(replay.replayed);
    assert_eq!(replay.recovery_case.status, "resolved");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn expired_candidate_start_abandons_attempt_and_reopens_review() {
    let (mut db, _data_dir) = migrated_db("candidate_start_before_reaper").await;
    let attack = seed_attack_fixture(db.pool()).await;
    let candidate = seed_candidate(db.pool(), &attack, attack.org_a).await;
    mark_candidate_wave_review_ready(db.pool(), &attack).await;
    let start_before = chrono::Utc::now() + chrono::Duration::milliseconds(1_500);
    let reviewed = review_wave_candidates(
        db.pool(),
        ReviewCandidateBatch {
            operation_id: attack.operation_id,
            wave_run_id: attack.wave_run_id,
            decisions: vec![CandidateReviewDecision {
                candidate_id: candidate.candidate_id,
                expected_candidate_plan_hash:
                    "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5"
                        .to_string(),
                expected_candidate_row_version: 0,
                approve: true,
                start_before: Some(start_before),
                expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            }],
        },
    )
    .await
    .expect("approve Candidate with a bounded start-before deadline");
    assert!(reviewed.state.review_closed);
    let approval_id = reviewed.approvals[0].id;
    let (stage_execution_id, stage_run_unit_id) =
        seed_verification_unit(db.pool(), &attack, attack.org_a).await;
    let claimed = claim_next_candidate_attempt(
        db.pool(),
        CandidateClaimQuery {
            operation_id: attack.operation_id,
            scope_snapshot_id: attack.scope_snapshot_id,
            wave_run_id: attack.wave_run_id,
            wave_unit_id: attack.org_a.wave_unit_id,
            organization_id: attack.org_a.organization_id,
            verification_stage_execution_id: stage_execution_id,
            verification_stage_run_unit_id: stage_run_unit_id,
            lease_owner: "candidate-start-before-reaper".to_string(),
            lease_seconds: 120,
        },
    )
    .await
    .expect("claim Candidate before start-before deadline")
    .expect("approved Candidate must be claimable before start-before deadline");
    let preconditions: (i64, i64) = sqlx::query_as(
        r#"SELECT
               (SELECT COUNT(*) FROM candidate_attempt_actions WHERE attempt_id=$1),
               (SELECT COUNT(*) FROM candidate_attempt_terminal_intents WHERE attempt_id=$1)"#,
    )
    .bind(claimed.attempt.id)
    .fetch_one(db.pool())
    .await
    .expect("verify unstarted Candidate recovery preconditions");
    assert_eq!(preconditions, (0, 0));

    let remaining = start_before - chrono::Utc::now();
    if let Ok(wait) = remaining.to_std() {
        tokio::time::sleep(wait + std::time::Duration::from_millis(150)).await;
    }
    let expired = expire_candidate_starts_before_claim(db.pool(), attack.operation_id)
        .await
        .expect("reap Candidate whose start-before deadline elapsed");
    assert_eq!(expired, 1);

    type StartExpiryProjection = (
        String,
        bool,
        String,
        String,
        String,
        bool,
        String,
        String,
        Option<Uuid>,
    );
    let state: StartExpiryProjection = sqlx::query_as(
        r#"SELECT attempt.status,attempt.result_json IS NULL,worker.status,
                  approval.status,candidate.disposition,unit.review_closed,
                  wave.status,barrier.status,lane.stage_worker_run_id
             FROM candidate_attempts attempt
             JOIN stage_worker_runs worker ON worker.id=attempt.stage_worker_run_id
             JOIN attack_candidate_approvals approval ON approval.id=attempt.approval_id
             JOIN attack_candidates candidate ON candidate.candidate_id=attempt.candidate_id
             JOIN attack_wave_units unit ON unit.id=attempt.wave_unit_id
             JOIN attack_wave_runs wave ON wave.id=attempt.wave_run_id
             JOIN candidate_review_barriers barrier ON barrier.wave_run_id=attempt.wave_run_id
             CROSS JOIN attack_execution_lanes lane
            WHERE attempt.id=$1 AND approval.id=$2
              AND lane.lane_key='global:exploit'"#,
    )
    .bind(claimed.attempt.id)
    .bind(approval_id)
    .fetch_one(db.pool())
    .await
    .expect("load Candidate start-before expiry projection");
    assert_eq!(state.0, "abandoned");
    assert!(state.1, "abandoned Attempt must not fabricate a result");
    assert_eq!(state.2, "superseded");
    assert_eq!(state.3, "expired");
    assert_eq!(state.4, "proposed");
    assert!(
        !state.5,
        "expired Candidate must reopen its WaveUnit review"
    );
    assert_eq!(state.6, "review");
    assert_eq!(state.7, "open");
    assert_eq!(state.8, None);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn expired_unclaimed_candidate_start_reopens_review_without_creating_attempt() {
    let (mut db, _data_dir) = migrated_db("candidate_unclaimed_start_before_reaper").await;
    let attack = seed_attack_fixture(db.pool()).await;
    let candidate = seed_candidate(db.pool(), &attack, attack.org_a).await;
    mark_candidate_wave_review_ready(db.pool(), &attack).await;
    let start_before = chrono::Utc::now() + chrono::Duration::milliseconds(1_500);
    let reviewed = review_wave_candidates(
        db.pool(),
        ReviewCandidateBatch {
            operation_id: attack.operation_id,
            wave_run_id: attack.wave_run_id,
            decisions: vec![CandidateReviewDecision {
                candidate_id: candidate.candidate_id,
                expected_candidate_plan_hash:
                    "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5"
                        .to_string(),
                expected_candidate_row_version: 0,
                approve: true,
                start_before: Some(start_before),
                expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            }],
        },
    )
    .await
    .expect("approve unclaimed Candidate with a bounded start-before deadline");
    assert!(reviewed.state.review_closed);
    let approval_id = reviewed.approvals[0].id;
    let attempt_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM candidate_attempts WHERE approval_id=$1")
            .bind(approval_id)
            .fetch_one(db.pool())
            .await
            .expect("count Candidate Attempts before unclaimed expiry");
    assert_eq!(attempt_count, 0, "expiry fixture must remain unclaimed");

    let remaining = start_before - chrono::Utc::now();
    if let Ok(wait) = remaining.to_std() {
        tokio::time::sleep(wait + std::time::Duration::from_millis(150)).await;
    }
    let expired = expire_candidate_starts_before_claim(db.pool(), attack.operation_id)
        .await
        .expect("expire unclaimed Candidate start-before deadline");
    assert_eq!(expired, 1);

    type UnclaimedStartExpiryProjection = (String, String, bool, String, String, i64);
    let state: UnclaimedStartExpiryProjection = sqlx::query_as(
        r#"SELECT approval.status,candidate.disposition,unit.review_closed,
                  wave.status,barrier.status,
                  (SELECT COUNT(*) FROM candidate_attempts counted
                    WHERE counted.approval_id=approval.id)
             FROM attack_candidate_approvals approval
             JOIN attack_candidates candidate ON candidate.candidate_id=approval.candidate_id
             JOIN attack_wave_units unit ON unit.id=approval.wave_unit_id
             JOIN attack_wave_runs wave ON wave.id=approval.wave_run_id
             JOIN candidate_review_barriers barrier ON barrier.wave_run_id=approval.wave_run_id
            WHERE approval.id=$1"#,
    )
    .bind(approval_id)
    .fetch_one(db.pool())
    .await
    .expect("load unclaimed Candidate start-before expiry projection");
    assert_eq!(state.0, "expired");
    assert_eq!(state.1, "proposed");
    assert!(
        !state.2,
        "expired unclaimed Candidate must reopen its WaveUnit review"
    );
    assert_eq!(state.3, "review");
    assert_eq!(state.4, "open");
    assert_eq!(state.5, 0, "expiry must not synthesize a CandidateAttempt");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn pending_candidate_terminal_intent_recovers_all_post_submit_crash_windows() {
    let (mut db, _data_dir) = migrated_db("candidate_pending_intent_crash_recovery").await;
    let fixture =
        seed_claimed_candidate_recovery_fixture(db.pool(), "pending-intent-recovery").await;
    sqlx::query(
        "UPDATE stage_runs
            SET status='completed',completed_at=NOW()
          WHERE operation_id=$1 AND id<>$2 AND status='started'",
    )
    .bind(fixture.attack.operation_id)
    .bind(fixture.stage_execution_id)
    .execute(db.pool())
    .await
    .expect("leave one active Verification StageExecution for recovery");
    sqlx::query("UPDATE operation_state SET current_stage='verification' WHERE operation_id=$1")
        .bind(fixture.attack.operation_id)
        .execute(db.pool())
        .await
        .expect("advance pending-intent fixture runtime cursor");

    let (action_id, _authorization_receipt_id) = authorize_candidate_recovery_action(
        db.pool(),
        &fixture,
        "pending-intent-recovery-action-auth",
    )
    .await;
    sqlx::query(
        "UPDATE candidate_attempt_actions
            SET status='completed',outcome='{}'::jsonb,outcome_hash='sha256:action-result',
                completed_at=NOW(),updated_at=NOW()
          WHERE id=$1 AND status='started'",
    )
    .bind(action_id)
    .execute(db.pool())
    .await
    .expect("complete the one authorized external action before submit");

    let worker_id = fixture.claimed.worker.id;
    let lease_token = fixture
        .claimed
        .worker
        .lease_token
        .expect("pending-intent Worker lease");
    let tool_call_record_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,status,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token)
           VALUES($1,'pending-intent-submit-call',$2,$3,'pentester',
                  'submit_candidate_attempt','{"disposition":"blocked"}'::jsonb,'running',
                  $3,$4,$5,$6,$7,$8,$9)"#,
    )
    .bind(tool_call_record_id)
    .bind(fixture.attack.session_id)
    .bind(fixture.attack.operation_id)
    .bind(fixture.stage_execution_id)
    .bind(fixture.stage_run_unit_id)
    .bind(worker_id)
    .bind(fixture.attack.org_a.organization_id)
    .bind(fixture.claimed.worker.attempt_epoch)
    .bind(lease_token)
    .execute(db.pool())
    .await
    .expect("insert unfinished submit tool call");
    sqlx::query(
        "UPDATE stage_worker_runs
            SET active_tool_call_id=$2,active_tool_started_at=NOW(),updated_at=NOW()
          WHERE id=$1",
    )
    .bind(worker_id)
    .bind(tool_call_record_id)
    .execute(db.pool())
    .await
    .expect("bind unfinished submit tool to Worker");

    let submitted_result = serde_json::json!({
        "disposition": "blocked",
        "blocker_reason_code": "bounded_probe_inconclusive",
        "proof_evidence_ids": [],
        "refutation_evidence_ids": [],
        "blocker_evidence_ids": [],
        "fact_deltas": []
    });
    let tool_result_text = serde_json::to_string(&serde_json::json!({
        "ok": true,
        "terminalization_pending": true,
        "attempt_id": fixture.claimed.attempt.id,
    }))
    .expect("serialize immutable submit ToolResult");
    let mut intent_tx = db.pool().begin().await.expect("begin pending intent");
    let intent = record_candidate_terminal_intent(
        &mut intent_tx,
        RecordCandidateTerminalIntent {
            request_id: "pending-intent-recovery-intent".to_string(),
            operation_id: fixture.attack.operation_id,
            organization_id: fixture.attack.org_a.organization_id,
            candidate_id: fixture.claimed.attempt.candidate_id,
            approval_id: fixture.approval_id,
            attempt_id: fixture.claimed.attempt.id,
            candidate_plan_hash: fixture.claimed.attempt.candidate_plan_hash.clone(),
            worker_run_id: worker_id,
            lease_token,
            attempt_epoch: fixture.claimed.worker.attempt_epoch,
            tool_call_record_id,
            disposition: "blocked".to_string(),
            submitted_result,
            evidence: vec![],
            tool_result_text: tool_result_text.clone(),
        },
    )
    .await
    .expect("commit immutable pending TerminalIntent");
    intent_tx.commit().await.expect("commit pending intent");

    // Crash window A: the intent committed, but the generic tool lifecycle did
    // not finish. The ordinary worker reaper may already have parked the exact
    // Worker while preserving its original lease identity.
    sqlx::query(
        "UPDATE stage_worker_runs
            SET status='recovery_required',
                lease_acquired_at=NOW()-INTERVAL '2 minutes',
                heartbeat_at=NOW()-INTERVAL '90 seconds',
                lease_expires_at=NOW()-INTERVAL '1 minute',updated_at=NOW()
          WHERE id=$1 AND active_tool_call_id=$2",
    )
    .bind(worker_id)
    .bind(tool_call_record_id)
    .execute(db.pool())
    .await
    .expect("park expired Worker with unfinished exact submit tool");
    sqlx::query(
        "UPDATE attack_execution_lanes
            SET lease_expires_at=NOW()-INTERVAL '1 minute',updated_at=NOW()
          WHERE lane_key='global:exploit' AND stage_worker_run_id=$1",
    )
    .bind(worker_id)
    .execute(db.pool())
    .await
    .expect("expire original Candidate lane");

    let recovery = RecoverCandidateTerminalIntent {
        operation_id: fixture.attack.operation_id,
        intent_id: intent.intent.id,
        expected_intent_hash: intent.intent.intent_hash.clone(),
    };
    let recovered = recover_candidate_terminal_intent_barrier(db.pool(), recovery.clone())
        .await
        .expect("server must recover exact ToolResult/checkpoint/barrier without an action replay");
    assert!(!recovered.replayed);
    assert!(recovered.tool_reconciled);
    assert!(recovered.worker_reconciled);
    assert_eq!(recovered.barrier.intent_id, intent.intent.id);
    assert_eq!(
        recovered.barrier.checkpoint_version,
        fixture.claimed.worker.checkpoint_version + 1
    );

    type RecoveredProjection = (
        String,
        Option<String>,
        String,
        Option<Uuid>,
        i64,
        serde_json::Value,
        serde_json::Value,
        i64,
        String,
        Option<String>,
    );
    let recovered_state: RecoveredProjection = sqlx::query_as(
        r#"SELECT tool.status::TEXT,tool.result,worker.status,
                  worker.active_tool_call_id,worker.checkpoint_version,
                  worker.checkpoint,chain.chain,
                  (SELECT COUNT(*) FROM candidate_attempt_actions action
                    WHERE action.attempt_id=intent.attempt_id),
                  (SELECT status FROM candidate_attempt_actions action
                    WHERE action.id=$3),
                  (SELECT outcome_hash FROM candidate_attempt_actions action
                    WHERE action.id=$3)
             FROM candidate_attempt_terminal_intents intent
             JOIN tool_calls tool ON tool.id=intent.tool_call_record_id
             JOIN stage_worker_runs worker ON worker.id=intent.worker_run_id
             JOIN message_chains chain ON chain.id=worker.message_chain_id
            WHERE intent.id=$1 AND worker.id=$2"#,
    )
    .bind(intent.intent.id)
    .bind(worker_id)
    .bind(action_id)
    .fetch_one(db.pool())
    .await
    .expect("load recovered terminal protocol projection");
    assert_eq!(recovered_state.0, "finished");
    assert_eq!(
        recovered_state.1.as_deref(),
        Some(tool_result_text.as_str())
    );
    assert_eq!(recovered_state.2, "recovery_required");
    assert_eq!(recovered_state.3, None);
    assert_eq!(
        recovered_state.4,
        fixture.claimed.worker.checkpoint_version + 1
    );
    assert_eq!(recovered_state.5, recovered_state.6);
    assert_eq!(
        recovered_state.7, 1,
        "recovery must not create another Action"
    );
    assert_eq!(recovered_state.8, "completed");
    assert_eq!(recovered_state.9.as_deref(), Some("sha256:action-result"));
    let recovered_chain = recovered_state
        .6
        .as_array()
        .expect("recovered chain must remain provider-shaped");
    assert!(recovered_chain.len() >= 2);
    assert_eq!(
        recovered_chain[recovered_chain.len() - 2]["role"],
        "assistant"
    );
    assert_eq!(recovered_chain[recovered_chain.len() - 1]["role"], "user");

    let barrier_replay = recover_candidate_terminal_intent_barrier(db.pool(), recovery)
        .await
        .expect("recovery response loss must replay the exact barrier");
    assert!(barrier_replay.replayed);
    assert!(!barrier_replay.tool_reconciled);
    assert!(!barrier_replay.worker_reconciled);
    assert_eq!(barrier_replay.barrier.id, recovered.barrier.id);

    // Crash window C: an ordinary reaper may have already removed the expired
    // executor lease and lane after the barrier became durable. Terminalization
    // authority must come from intent+barrier, not from resurrecting that lease.
    sqlx::query(
        "UPDATE attack_execution_lanes
            SET stage_worker_run_id=NULL,lease_token=NULL,lease_owner=NULL,
                lease_expires_at=NULL,updated_at=NOW()
          WHERE lane_key='global:exploit' AND stage_worker_run_id=$1",
    )
    .bind(worker_id)
    .execute(db.pool())
    .await
    .expect("clear expired original lane before server terminalization");
    sqlx::query(
        "UPDATE stage_worker_runs
            SET status='queued',lease_token=NULL,lease_owner=NULL,
                lease_acquired_at=NULL,lease_expires_at=NULL,heartbeat_at=NULL,
                updated_at=NOW()
          WHERE id=$1 AND status='recovery_required' AND active_tool_call_id IS NULL",
    )
    .bind(worker_id)
    .execute(db.pool())
    .await
    .expect("clear expired original Worker lease before server terminalization");

    let terminal_command = TerminalizeCandidateTerminalIntent {
        request_id: "pending-intent-recovery-terminal".to_string(),
        operation_id: fixture.attack.operation_id,
        intent_id: intent.intent.id,
        barrier_id: recovered.barrier.id,
    };
    let terminal = terminalize_candidate_terminal_intent(db.pool(), terminal_command.clone())
        .await
        .expect("intent+barrier must terminalize without the expired executor lease");
    assert!(!terminal.replayed);
    assert_eq!(terminal.receipt.disposition, "blocked");
    let terminal_replay = terminalize_candidate_terminal_intent(db.pool(), terminal_command)
        .await
        .expect("terminalizer response loss must replay the exact terminal receipt");
    assert!(terminal_replay.replayed);
    assert_eq!(terminal_replay.receipt.id, terminal.receipt.id);

    let final_state: (String, String, Option<Uuid>, i64, String, Option<String>) = sqlx::query_as(
        r#"SELECT attempt.status,worker.status,lane.stage_worker_run_id,
                      (SELECT COUNT(*) FROM candidate_attempt_actions action
                        WHERE action.attempt_id=attempt.id),
                      (SELECT status FROM candidate_attempt_actions action WHERE action.id=$2),
                      (SELECT outcome_hash FROM candidate_attempt_actions action WHERE action.id=$2)
                 FROM candidate_attempts attempt
                 JOIN stage_worker_runs worker ON worker.id=attempt.stage_worker_run_id
                 CROSS JOIN attack_execution_lanes lane
                WHERE attempt.id=$1 AND lane.lane_key='global:exploit'"#,
    )
    .bind(fixture.claimed.attempt.id)
    .bind(action_id)
    .fetch_one(db.pool())
    .await
    .expect("load final recovered Candidate state");
    assert_eq!(final_state.0, "blocked");
    assert_eq!(final_state.1, "passed");
    assert_eq!(final_state.2, None);
    assert_eq!(final_state.3, 1);
    assert_eq!(final_state.4, "completed");
    assert_eq!(final_state.5.as_deref(), Some("sha256:action-result"));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn pending_candidate_terminal_recovery_reuses_exact_checkpointed_pair_and_rejects_drift() {
    let (mut db, _data_dir) = migrated_db("candidate_checkpointed_pair_recovery").await;
    let fixture =
        seed_claimed_candidate_recovery_fixture(db.pool(), "checkpointed-pair-recovery").await;
    sqlx::query(
        "UPDATE stage_runs
            SET status='completed',completed_at=NOW()
          WHERE operation_id=$1 AND id<>$2 AND status='started'",
    )
    .bind(fixture.attack.operation_id)
    .bind(fixture.stage_execution_id)
    .execute(db.pool())
    .await
    .expect("leave one active Verification StageExecution");
    sqlx::query("UPDATE operation_state SET current_stage='verification' WHERE operation_id=$1")
        .bind(fixture.attack.operation_id)
        .execute(db.pool())
        .await
        .expect("advance exact-pair fixture runtime cursor");
    let (action_id, _authorization_receipt_id) =
        authorize_candidate_recovery_action(db.pool(), &fixture, "checkpointed-pair-action-auth")
            .await;
    sqlx::query(
        "UPDATE candidate_attempt_actions
            SET status='completed',outcome='{}'::jsonb,outcome_hash='sha256:action-result',
                completed_at=NOW(),updated_at=NOW()
          WHERE id=$1 AND status='started'",
    )
    .bind(action_id)
    .execute(db.pool())
    .await
    .expect("complete exact-pair action");

    let worker_id = fixture.claimed.worker.id;
    let lease_token = fixture
        .claimed
        .worker
        .lease_token
        .expect("exact-pair Worker lease");
    let tool_call_record_id = Uuid::new_v4();
    let tool_args = serde_json::json!({"disposition": "blocked"});
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,status,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token)
           VALUES($1,'checkpointed-pair-submit-call',$2,$3,'pentester',
                  'submit_candidate_attempt',$10,'running',$3,$4,$5,$6,$7,$8,$9)"#,
    )
    .bind(tool_call_record_id)
    .bind(fixture.attack.session_id)
    .bind(fixture.attack.operation_id)
    .bind(fixture.stage_execution_id)
    .bind(fixture.stage_run_unit_id)
    .bind(worker_id)
    .bind(fixture.attack.org_a.organization_id)
    .bind(fixture.claimed.worker.attempt_epoch)
    .bind(lease_token)
    .bind(&tool_args)
    .execute(db.pool())
    .await
    .expect("insert exact-pair submit tool");
    sqlx::query(
        "UPDATE stage_worker_runs
            SET active_tool_call_id=$2,active_tool_started_at=NOW(),updated_at=NOW()
          WHERE id=$1",
    )
    .bind(worker_id)
    .bind(tool_call_record_id)
    .execute(db.pool())
    .await
    .expect("bind exact-pair submit tool");
    let submitted_result = serde_json::json!({
        "disposition": "blocked",
        "blocker_reason_code": "bounded_probe_inconclusive",
        "proof_evidence_ids": [],
        "refutation_evidence_ids": [],
        "blocker_evidence_ids": [],
        "fact_deltas": []
    });
    let tool_result_text = serde_json::to_string(&serde_json::json!({
        "ok": true,
        "terminalization_pending": true,
        "attempt_id": fixture.claimed.attempt.id,
    }))
    .expect("serialize exact-pair ToolResult");
    let mut intent_tx = db.pool().begin().await.expect("begin exact-pair intent");
    let intent = record_candidate_terminal_intent(
        &mut intent_tx,
        RecordCandidateTerminalIntent {
            request_id: "checkpointed-pair-intent".to_string(),
            operation_id: fixture.attack.operation_id,
            organization_id: fixture.attack.org_a.organization_id,
            candidate_id: fixture.claimed.attempt.candidate_id,
            approval_id: fixture.approval_id,
            attempt_id: fixture.claimed.attempt.id,
            candidate_plan_hash: fixture.claimed.attempt.candidate_plan_hash.clone(),
            worker_run_id: worker_id,
            lease_token,
            attempt_epoch: fixture.claimed.worker.attempt_epoch,
            tool_call_record_id,
            disposition: "blocked".to_string(),
            submitted_result,
            evidence: vec![],
            tool_result_text: tool_result_text.clone(),
        },
    )
    .await
    .expect("record exact-pair intent");
    intent_tx.commit().await.expect("commit exact-pair intent");

    sqlx::query("UPDATE tool_calls SET status='finished',result=$2,updated_at=NOW() WHERE id=$1")
        .bind(tool_call_record_id)
        .bind(&tool_result_text)
        .execute(db.pool())
        .await
        .expect("finish exact-pair tool ledger");
    let mut chain: serde_json::Value =
        sqlx::query_scalar("SELECT chain FROM message_chains WHERE id=$1")
            .bind(
                fixture
                    .claimed
                    .worker
                    .message_chain_id
                    .expect("exact-pair chain"),
            )
            .fetch_one(db.pool())
            .await
            .expect("load exact-pair chain");
    let provider_call_id = "provider-exact-submit-call";
    chain
        .as_array_mut()
        .expect("exact-pair chain array")
        .extend([
            serde_json::json!({
                "role": "assistant",
                "id": null,
                "content": [{
                    "id": "provider-internal-submit-call",
                    "call_id": provider_call_id,
                    "function": {
                        "name": "submit_candidate_attempt",
                        "arguments": tool_args,
                    },
                    "signature": null,
                    "additional_params": null,
                }],
            }),
            serde_json::json!({
                "role": "user",
                "content": [{
                    "type": "toolresult",
                    "id": "provider-internal-submit-call",
                    "call_id": provider_call_id,
                    "content": [{"type": "text", "text": "drifted-result"}],
                }],
            }),
        ]);
    let checkpoint_version: i64 = sqlx::query_scalar(
        "UPDATE stage_worker_runs
            SET active_tool_call_id=NULL,active_tool_started_at=NULL,
                checkpoint=$2,checkpoint_version=checkpoint_version+1,updated_at=NOW()
          WHERE id=$1 RETURNING checkpoint_version",
    )
    .bind(worker_id)
    .bind(&chain)
    .fetch_one(db.pool())
    .await
    .expect("persist drifted checkpoint pair before barrier");
    sqlx::query("UPDATE message_chains SET chain=$2,updated_at=NOW() WHERE id=$1")
        .bind(fixture.claimed.worker.message_chain_id.expect("chain id"))
        .bind(&chain)
        .execute(db.pool())
        .await
        .expect("persist drifted chain pair before barrier");
    let recovery = RecoverCandidateTerminalIntent {
        operation_id: fixture.attack.operation_id,
        intent_id: intent.intent.id,
        expected_intent_hash: intent.intent.intent_hash.clone(),
    };
    assert!(
        recover_candidate_terminal_intent_barrier(db.pool(), recovery.clone())
            .await
            .is_err(),
        "a checkpointed submit pair with ToolResult drift must fail closed"
    );
    let barrier_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM candidate_attempt_terminal_barriers WHERE intent_id=$1",
    )
    .bind(intent.intent.id)
    .fetch_one(db.pool())
    .await
    .expect("count barriers after rejected drift");
    assert_eq!(barrier_count, 0);

    let terminal_result_index = chain.as_array().expect("chain array").len() - 1;
    chain[terminal_result_index]["content"][0]["content"][0]["text"] =
        serde_json::json!(tool_result_text);
    sqlx::query("UPDATE message_chains SET chain=$2,updated_at=NOW() WHERE id=$1")
        .bind(fixture.claimed.worker.message_chain_id.expect("chain id"))
        .bind(&chain)
        .execute(db.pool())
        .await
        .expect("repair exact chain pair");
    sqlx::query("UPDATE stage_worker_runs SET checkpoint=$2,updated_at=NOW() WHERE id=$1")
        .bind(worker_id)
        .bind(&chain)
        .execute(db.pool())
        .await
        .expect("repair exact checkpoint pair");
    let exact_chain_before = chain.clone();
    let recovered = recover_candidate_terminal_intent_barrier(db.pool(), recovery)
        .await
        .expect("exact already-checkpointed pair must create only the missing barrier");
    assert!(!recovered.replayed);
    assert!(!recovered.tool_reconciled);
    assert!(!recovered.worker_reconciled);
    assert_eq!(recovered.barrier.checkpoint_version, checkpoint_version);
    let after: (serde_json::Value, serde_json::Value, i64) = sqlx::query_as(
        r#"SELECT chain.chain,worker.checkpoint,worker.checkpoint_version
             FROM stage_worker_runs worker
             JOIN message_chains chain ON chain.id=worker.message_chain_id
            WHERE worker.id=$1"#,
    )
    .bind(worker_id)
    .fetch_one(db.pool())
    .await
    .expect("load exact pair after missing-barrier recovery");
    assert_eq!(after.0, exact_chain_before);
    assert_eq!(after.1, exact_chain_before);
    assert_eq!(after.2, checkpoint_version);
    assert_eq!(
        after
            .0
            .as_array()
            .expect("exact chain array")
            .iter()
            .filter(|message| {
                message
                    .get("content")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .any(|content| {
                        content
                            .get("function")
                            .and_then(|function| function.get("name"))
                            .and_then(serde_json::Value::as_str)
                            == Some("submit_candidate_attempt")
                    })
            })
            .count(),
        1,
        "recovery must not append a duplicate submit pair"
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn retry_release_rejects_a_terminal_action_and_preserves_bound_authority() {
    let (mut db, _data_dir) = migrated_db("candidate_release_terminal_action").await;
    let fixture =
        seed_claimed_candidate_recovery_fixture(db.pool(), "release-terminal-action").await;
    let (action_id, _receipt_id) =
        authorize_candidate_recovery_action(db.pool(), &fixture, "release-terminal-action-auth")
            .await;
    let lease_token = fixture
        .claimed
        .worker
        .lease_token
        .expect("release fixture lease");
    let release = CandidateExecutionRelease {
        operation_id: fixture.attack.operation_id,
        scope_snapshot_id: fixture.attack.scope_snapshot_id,
        wave_run_id: fixture.attack.wave_run_id,
        wave_unit_id: fixture.attack.org_a.wave_unit_id,
        organization_id: fixture.attack.org_a.organization_id,
        attempt_id: fixture.claimed.attempt.id,
        worker_run_id: fixture.claimed.worker.id,
        stage_execution_id: fixture.stage_execution_id,
        stage_run_unit_id: fixture.stage_run_unit_id,
        lease_token,
        lease_owner: "release-terminal-action".to_string(),
        attempt_epoch: fixture.claimed.worker.attempt_epoch,
        expected_checkpoint_version: fixture.claimed.worker.checkpoint_version,
    };
    assert_eq!(
        candidate_execution_continuation(db.pool(), &release)
            .await
            .expect("classify started-action continuation"),
        CandidateExecutionContinuation::RecoveryRequired
    );
    let started_release_error = release_candidate_execution(db.pool(), release.clone())
        .await
        .expect_err("a started action must never be released or replayed");
    assert!(started_release_error
        .to_string()
        .contains("entirely unstarted planned action journal"));
    sqlx::query(
        "UPDATE candidate_attempt_actions
            SET status='completed',outcome='{}'::jsonb,outcome_hash='sha256:completed-once',
                completed_at=NOW(),updated_at=NOW()
          WHERE id=$1 AND status='started'",
    )
    .bind(action_id)
    .execute(db.pool())
    .await
    .expect("complete side-effect action before provider failure");

    let continuation = candidate_execution_continuation(db.pool(), &release)
        .await
        .expect("classify completed-action continuation");
    assert_eq!(continuation, CandidateExecutionContinuation::SubmitOnly);
    let error = release_candidate_execution(db.pool(), release)
        .await
        .expect_err("a completed action must never be rewritten as retryable failure");
    assert!(
        error
            .to_string()
            .contains("entirely unstarted planned action journal"),
        "unexpected unsafe-release error: {error}"
    );

    let preserved: (String, String, String, Option<Uuid>) = sqlx::query_as(
        r#"SELECT attempt.status,worker.status,action.status,lane.stage_worker_run_id
             FROM candidate_attempts attempt
             JOIN stage_worker_runs worker ON worker.id=attempt.stage_worker_run_id
             JOIN candidate_attempt_actions action ON action.attempt_id=attempt.id
             CROSS JOIN attack_execution_lanes lane
            WHERE attempt.id=$1 AND action.id=$2 AND lane.lane_key='global:exploit'"#,
    )
    .bind(fixture.claimed.attempt.id)
    .bind(action_id)
    .fetch_one(db.pool())
    .await
    .expect("load authority after rejected retry release");
    assert_eq!(preserved.0, "running");
    assert_eq!(preserved.1, "running");
    assert_eq!(preserved.2, "completed");
    assert_eq!(preserved.3, Some(fixture.claimed.worker.id));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn approval_expiry_after_action_allows_finish_submit_and_terminalize() {
    let (mut db, _data_dir) = migrated_db("candidate_expiry_after_action").await;
    let attack = seed_attack_fixture(db.pool()).await;
    let candidate = seed_candidate(db.pool(), &attack, attack.org_a).await;
    mark_candidate_wave_review_ready(db.pool(), &attack).await;
    let deadline = chrono::Utc::now() + chrono::Duration::milliseconds(1_500);
    let reviewed = review_wave_candidates(
        db.pool(),
        ReviewCandidateBatch {
            operation_id: attack.operation_id,
            wave_run_id: attack.wave_run_id,
            decisions: vec![CandidateReviewDecision {
                candidate_id: candidate.candidate_id,
                expected_candidate_plan_hash:
                    "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5"
                        .to_string(),
                expected_candidate_row_version: 0,
                approve: true,
                start_before: Some(deadline),
                expires_at: Some(deadline),
            }],
        },
    )
    .await
    .expect("approve Candidate with one short action-start boundary");
    let approval_id = reviewed.approvals[0].id;
    let (stage_execution_id, stage_run_unit_id) =
        seed_verification_unit(db.pool(), &attack, attack.org_a).await;
    let claimed = claim_next_candidate_attempt(
        db.pool(),
        CandidateClaimQuery {
            operation_id: attack.operation_id,
            scope_snapshot_id: attack.scope_snapshot_id,
            wave_run_id: attack.wave_run_id,
            wave_unit_id: attack.org_a.wave_unit_id,
            organization_id: attack.org_a.organization_id,
            verification_stage_execution_id: stage_execution_id,
            verification_stage_run_unit_id: stage_run_unit_id,
            lease_owner: "expiry-after-action".to_string(),
            lease_seconds: 120,
        },
    )
    .await
    .expect("claim before action-start deadline")
    .expect("Candidate must be claimable before action-start deadline");
    let lease_token = claimed.worker.lease_token.expect("expiry action lease");
    let started = begin_candidate_action(
        db.pool(),
        BeginCandidateAction {
            operation_id: attack.operation_id,
            stage_execution_id,
            stage_run_unit_id,
            organization_id: attack.org_a.organization_id,
            worker_run_id: claimed.worker.id,
            lease_token,
            attempt_epoch: claimed.worker.attempt_epoch,
            candidate_id: candidate.candidate_id,
            approval_id,
            attempt_id: claimed.attempt.id,
            candidate_plan_hash: claimed.attempt.candidate_plan_hash.clone(),
            workspace_path_sha256: "sha256:attack-v2".to_string(),
            action_ordinal: 0,
        },
    )
    .await
    .expect("start action before approval boundary");
    let action_id = match started {
        CandidateActionStart::Authorized(action) => action.action_id,
        other => panic!("expected fresh action authorization, got {other:?}"),
    };

    let remaining = deadline - chrono::Utc::now();
    if let Ok(wait) = remaining.to_std() {
        tokio::time::sleep(wait + std::time::Duration::from_millis(150)).await;
    }
    let deadlines_elapsed: bool = sqlx::query_scalar(
        "SELECT start_before<=NOW() AND expires_at<=NOW()
           FROM attack_candidate_approvals WHERE id=$1",
    )
    .bind(approval_id)
    .fetch_one(db.pool())
    .await
    .expect("check elapsed Candidate approval deadlines");
    assert!(deadlines_elapsed);

    finish_candidate_action(
        db.pool(),
        FinishCandidateAction {
            operation_id: attack.operation_id,
            stage_execution_id,
            stage_run_unit_id,
            organization_id: attack.org_a.organization_id,
            worker_run_id: claimed.worker.id,
            lease_token,
            attempt_epoch: claimed.worker.attempt_epoch,
            candidate_id: candidate.candidate_id,
            approval_id,
            attempt_id: claimed.attempt.id,
            candidate_plan_hash: claimed.attempt.candidate_plan_hash.clone(),
            action_id,
            success: true,
            outcome: serde_json::json!({"bounded_probe": "completed"}),
            error_code: None,
        },
    )
    .await
    .expect("finish an already-authorized action after start-before expiry");

    // Simulate process loss after the external action was durably completed
    // but before the model submitted. Expired-lane recovery must reclaim this
    // exact Attempt/chain in submit-only mode even though start_before elapsed.
    sqlx::query(
        "UPDATE stage_worker_runs
            SET lease_acquired_at=NOW()-INTERVAL '2 minutes',
                heartbeat_at=NOW()-INTERVAL '90 seconds',
                lease_expires_at=NOW()-INTERVAL '1 minute',updated_at=NOW()
          WHERE id=$1",
    )
    .bind(claimed.worker.id)
    .execute(db.pool())
    .await
    .expect("expire completed-action Worker lease");
    sqlx::query(
        "UPDATE attack_execution_lanes
            SET lease_expires_at=NOW()-INTERVAL '1 minute',updated_at=NOW()
          WHERE lane_key='global:exploit' AND stage_worker_run_id=$1",
    )
    .bind(claimed.worker.id)
    .execute(db.pool())
    .await
    .expect("expire completed-action global lane");
    let resumed = claim_next_candidate_attempt(
        db.pool(),
        CandidateClaimQuery {
            operation_id: attack.operation_id,
            scope_snapshot_id: attack.scope_snapshot_id,
            wave_run_id: attack.wave_run_id,
            wave_unit_id: attack.org_a.wave_unit_id,
            organization_id: attack.org_a.organization_id,
            verification_stage_execution_id: stage_execution_id,
            verification_stage_run_unit_id: stage_run_unit_id,
            lease_owner: "expiry-after-action-submit-only".to_string(),
            lease_seconds: 120,
        },
    )
    .await
    .expect("reclaim completed action after start-before expiry")
    .expect("completed action must remain submit-only claimable");
    assert_eq!(resumed.attempt.id, claimed.attempt.id);
    assert_eq!(resumed.worker.id, claimed.worker.id);
    assert_eq!(
        resumed.worker.message_chain_id, claimed.worker.message_chain_id,
        "submit-only recovery must preserve the exact verifier chain"
    );
    assert!(resumed.submit_only);
    let resumed_lease_token = resumed
        .worker
        .lease_token
        .expect("resumed submit-only lease");

    let mut submission_tx = db.pool().begin().await.expect("begin expired submission");
    let submitted = record_attempt_submission(
        &mut submission_tx,
        RecordAttemptSubmission {
            operation_id: attack.operation_id,
            scope_snapshot_id: attack.scope_snapshot_id,
            wave_run_id: attack.wave_run_id,
            wave_unit_id: attack.org_a.wave_unit_id,
            organization_id: attack.org_a.organization_id,
            candidate_id: candidate.candidate_id,
            approval_id,
            attempt_id: claimed.attempt.id,
            candidate_plan_hash: claimed.attempt.candidate_plan_hash.clone(),
            worker_run_id: resumed.worker.id,
            stage_execution_id,
            stage_run_unit_id,
            lease_token: resumed_lease_token,
            lease_owner: "expiry-after-action-submit-only".to_string(),
            attempt_epoch: resumed.worker.attempt_epoch,
            expected_checkpoint_version: resumed.worker.checkpoint_version,
            result_json: serde_json::json!({
                "disposition": "blocked",
                "blocker_reason_code": "bounded_probe_inconclusive"
            }),
            evidence: Vec::new(),
        },
    )
    .await
    .expect("submit the durable terminal action journal after approval expiry");
    submission_tx
        .commit()
        .await
        .expect("commit expired Candidate submission");

    let mut terminal_tx = db.pool().begin().await.expect("begin expired terminalizer");
    let terminal = terminalize_candidate_attempt(
        &mut terminal_tx,
        TerminalizeCandidateAttempt {
            operation_id: attack.operation_id,
            scope_snapshot_id: attack.scope_snapshot_id,
            wave_run_id: attack.wave_run_id,
            wave_unit_id: attack.org_a.wave_unit_id,
            organization_id: attack.org_a.organization_id,
            candidate_id: candidate.candidate_id,
            approval_id,
            attempt_id: claimed.attempt.id,
            candidate_plan_hash: claimed.attempt.candidate_plan_hash.clone(),
            expected_result_hash: submitted.attempt.result_hash.expect("expired result hash"),
            worker_run_id: resumed.worker.id,
            stage_execution_id,
            stage_run_unit_id,
            lease_token: resumed_lease_token,
            lease_owner: "expiry-after-action-submit-only".to_string(),
            attempt_epoch: resumed.worker.attempt_epoch,
            expected_checkpoint_version: resumed.worker.checkpoint_version,
        },
    )
    .await
    .expect("terminalize an already-executed action after approval expiry");
    terminal_tx
        .commit()
        .await
        .expect("commit expired Candidate terminalizer");
    assert_eq!(terminal.status, "blocked");
    assert_eq!(terminal.attempt_id, claimed.attempt.id);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn approval_start_before_expiry_blocks_a_new_action_on_an_existing_attempt() {
    let (mut db, _data_dir) = migrated_db("candidate_expiry_blocks_new_action").await;
    let attack = seed_attack_fixture(db.pool()).await;
    let candidate = seed_candidate(db.pool(), &attack, attack.org_a).await;
    mark_candidate_wave_review_ready(db.pool(), &attack).await;
    let start_before = chrono::Utc::now() + chrono::Duration::milliseconds(1_500);
    let reviewed = review_wave_candidates(
        db.pool(),
        ReviewCandidateBatch {
            operation_id: attack.operation_id,
            wave_run_id: attack.wave_run_id,
            decisions: vec![CandidateReviewDecision {
                candidate_id: candidate.candidate_id,
                expected_candidate_plan_hash:
                    "sha256:16452624f0a24a8f73f27cfe10e03d0e1ad46e65d662778a50096137be39c8b5"
                        .to_string(),
                expected_candidate_row_version: 0,
                approve: true,
                start_before: Some(start_before),
                expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            }],
        },
    )
    .await
    .expect("approve Candidate before action-start boundary test");
    let approval_id = reviewed.approvals[0].id;
    let (stage_execution_id, stage_run_unit_id) =
        seed_verification_unit(db.pool(), &attack, attack.org_a).await;
    let claimed = claim_next_candidate_attempt(
        db.pool(),
        CandidateClaimQuery {
            operation_id: attack.operation_id,
            scope_snapshot_id: attack.scope_snapshot_id,
            wave_run_id: attack.wave_run_id,
            wave_unit_id: attack.org_a.wave_unit_id,
            organization_id: attack.org_a.organization_id,
            verification_stage_execution_id: stage_execution_id,
            verification_stage_run_unit_id: stage_run_unit_id,
            lease_owner: "expiry-block-new-action".to_string(),
            lease_seconds: 120,
        },
    )
    .await
    .expect("claim Candidate before action-start deadline")
    .expect("Candidate must be claimable before deadline");
    let remaining = start_before - chrono::Utc::now();
    if let Ok(wait) = remaining.to_std() {
        tokio::time::sleep(wait + std::time::Duration::from_millis(150)).await;
    }
    let rejected = begin_candidate_action(
        db.pool(),
        BeginCandidateAction {
            operation_id: attack.operation_id,
            stage_execution_id,
            stage_run_unit_id,
            organization_id: attack.org_a.organization_id,
            worker_run_id: claimed.worker.id,
            lease_token: claimed.worker.lease_token.expect("new-action test lease"),
            attempt_epoch: claimed.worker.attempt_epoch,
            candidate_id: candidate.candidate_id,
            approval_id,
            attempt_id: claimed.attempt.id,
            candidate_plan_hash: claimed.attempt.candidate_plan_hash.clone(),
            workspace_path_sha256: "sha256:attack-v2".to_string(),
            action_ordinal: 0,
        },
    )
    .await;
    assert!(
        rejected.is_err(),
        "expired start-before must block a new action"
    );
    let action_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM candidate_attempt_actions WHERE attempt_id=$1")
            .bind(claimed.attempt.id)
            .fetch_one(db.pool())
            .await
            .expect("count actions after expired start rejection");
    assert_eq!(action_count, 0);
    db.stop().await;
}
