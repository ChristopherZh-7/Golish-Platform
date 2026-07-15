use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration as StdDuration, SystemTime};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use golish_agent_app::ai::commands::attack::AttackCandidateReviewRequest;
use golish_agent_app::ai::commands::cleanup::CleanupWaiverSubmitRequest;
use golish_agent_app::ai::commands::reporting::ReportingFinalizeRequest;
use golish_agent_app::ai::db_bridge::knowledge_memory::{
    KnowledgeEmbeddingProvider, KnowledgeMemoryRuntime,
};
use golish_agent_app::ai::db_bridge::reporting::{
    load_report_bundle, PgReportPublicationPort, PgReportTruthPort,
};
use golish_core::Tool;
use golish_db::embeddings::Embedder;
use golish_db::repo::candidate_attempts::{
    claim_next_candidate_attempt, record_attempt_submission, AttemptEvidenceLink,
    CandidateClaimQuery, RecordAttemptSubmission,
};
use golish_db::repo::finding_lineage::{terminalize_verified_finding, TerminalizeVerifiedFinding};
use golish_db::repo::{
    cleanup_absence_checks, cleanup_attempts, cleanup_obligations, footholds, knowledge_outbox,
    objective_attempts, operator_principals, post_exploit_actions, post_exploit_approvals,
};
use golish_db::{DbConfig, GolishDb, PgPool};
use golish_memory_domain::event_catalog::ProjectorId;
use golish_memory_domain::EMBEDDING_DIMENSION_V1;
use golish_pentest_app::pentest_bridge::PostExploitExecuteActionTool;
use golish_reporting_app::{
    ArtifactPublicationReservation, ContentAddressedArtifact, ExplicitFinalizeRequest,
    ReportArtifactStore, ReportFinalizer, ReportFormat, ReportReadModelBuilder, ReportingAppError,
    StagedArtifact,
};
use golish_reporting_domain::{PublicationStatus, ValidationStatus};
use serde_json::json;
use serial_test::serial;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const FORGED_TOOL_PROJECT_PATH: &str = "/tmp/golish-v2-closeout-contract";
const REPORT_ARTIFACT_ORPHAN_GRACE: StdDuration = StdDuration::from_secs(24 * 60 * 60);

fn report_format_to_storage(
    format: ReportFormat,
) -> golish_projects::file_storage::ReportArtifactFormat {
    match format {
        ReportFormat::Markdown => golish_projects::file_storage::ReportArtifactFormat::Markdown,
        ReportFormat::Json => golish_projects::file_storage::ReportArtifactFormat::Json,
    }
}

fn report_format_from_storage(
    format: golish_projects::file_storage::ReportArtifactFormat,
) -> ReportFormat {
    match format {
        golish_projects::file_storage::ReportArtifactFormat::Markdown => ReportFormat::Markdown,
        golish_projects::file_storage::ReportArtifactFormat::Json => ReportFormat::Json,
    }
}

fn report_artifact_error(error: impl std::fmt::Display) -> ReportingAppError {
    ReportingAppError::Artifact(error.to_string())
}

struct TestArtifactPublicationReservation {
    artifact: ContentAddressedArtifact,
    _storage_reservation: golish_projects::file_storage::ReservedReportArtifact,
}

impl ArtifactPublicationReservation for TestArtifactPublicationReservation {
    fn artifact(&self) -> &ContentAddressedArtifact {
        &self.artifact
    }
}

#[derive(Clone, Debug)]
struct TestProjectReportArtifactStore {
    project_root: PathBuf,
}

impl TestProjectReportArtifactStore {
    fn new(project_root: PathBuf) -> Self {
        Self { project_root }
    }

    fn stored_staging(
        staged: &StagedArtifact,
    ) -> golish_projects::file_storage::StagedReportArtifact {
        golish_projects::file_storage::StagedReportArtifact {
            revision_id: staged.revision_id.to_string(),
            format: report_format_to_storage(staged.format),
            staging_key: staged.staging_key.clone(),
            sha256: staged.sha256.clone(),
            byte_len: staged.byte_len,
        }
    }

    fn stored_artifact(
        artifact: &ContentAddressedArtifact,
    ) -> golish_projects::file_storage::StoredReportArtifact {
        golish_projects::file_storage::StoredReportArtifact {
            format: report_format_to_storage(artifact.format),
            content_key: artifact.content_key.clone(),
            storage_path: format!(".golish/reports/blobs/{}", artifact.content_key),
            sha256: artifact.sha256.clone(),
            byte_len: artifact.byte_len,
        }
    }
}

#[async_trait]
impl ReportArtifactStore for TestProjectReportArtifactStore {
    async fn stage(
        &self,
        revision_id: Uuid,
        format: ReportFormat,
        bytes: &[u8],
    ) -> Result<StagedArtifact, ReportingAppError> {
        let staged = golish_projects::file_storage::stage_report_artifact(
            &self.project_root,
            &revision_id.to_string(),
            report_format_to_storage(format),
            bytes,
        )
        .await
        .map_err(report_artifact_error)?;
        Ok(StagedArtifact {
            revision_id,
            format: report_format_from_storage(staged.format),
            staging_key: staged.staging_key,
            sha256: staged.sha256,
            byte_len: staged.byte_len,
        })
    }

    async fn promote(
        &self,
        staged: &StagedArtifact,
    ) -> Result<Box<dyn ArtifactPublicationReservation>, ReportingAppError> {
        let storage_reservation = golish_projects::file_storage::promote_report_artifact(
            &self.project_root,
            &Self::stored_staging(staged),
        )
        .await
        .map_err(report_artifact_error)?;
        let stored = storage_reservation.artifact();
        let artifact = ContentAddressedArtifact {
            format: report_format_from_storage(stored.format),
            content_key: stored.content_key.clone(),
            sha256: stored.sha256.clone(),
            byte_len: stored.byte_len,
        };
        Ok(Box::new(TestArtifactPublicationReservation {
            artifact,
            _storage_reservation: storage_reservation,
        }))
    }

    async fn verify(&self, artifact: &ContentAddressedArtifact) -> Result<bool, ReportingAppError> {
        golish_projects::file_storage::verify_report_artifact(
            &self.project_root,
            &Self::stored_artifact(artifact),
        )
        .await
        .map_err(report_artifact_error)
    }

    async fn discard_staging(&self, staged: &StagedArtifact) -> Result<(), ReportingAppError> {
        golish_projects::file_storage::discard_staged_report_artifact(
            &self.project_root,
            &Self::stored_staging(staged),
        )
        .await
        .map_err(report_artifact_error)
    }

    async fn gc(
        &self,
        now: DateTime<Utc>,
        referenced_content_keys: BTreeSet<String>,
    ) -> Result<(), ReportingAppError> {
        let now = SystemTime::UNIX_EPOCH
            + StdDuration::from_secs(u64::try_from(now.timestamp()).unwrap_or_default())
            + StdDuration::from_nanos(u64::from(now.timestamp_subsec_nanos()));
        golish_projects::file_storage::gc_report_artifacts(
            &self.project_root,
            now,
            REPORT_ARTIFACT_ORPHAN_GRACE,
            &referenced_content_keys,
        )
        .await
        .map(|_| ())
        .map_err(report_artifact_error)
    }
}

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read local postgres port")
        .port()
}

async fn fixture(label: &str) -> (GolishDb, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let config = DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("v2_closeout_{label}_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    };
    let db = GolishDb::start(config)
        .await
        .expect("start migrated embedded postgres");
    (db, data_dir)
}

fn sha256_json(value: &serde_json::Value) -> String {
    let bytes = serde_json::to_vec(value).expect("serialize fixture JSON");
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn target_identity_hash(target_type: &str, target_value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(target_type.as_bytes());
    hasher.update([0]);
    hasher.update(target_value.as_bytes());
    let digest = hasher.finalize();
    format!(
        "sha256:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

#[derive(Clone, Copy)]
struct StageSeal {
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    submission_id: Uuid,
}

#[derive(Clone)]
struct CandidateSubmissionFixture {
    operation_id: Uuid,
    project_scope_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    target_id: Uuid,
    attempt_id: Uuid,
    proof_evidence_id: i64,
    terminalize: TerminalizeVerifiedFinding,
}

async fn insert_evidence(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
    target_id: Option<Uuid>,
    label: &str,
    project_path: &str,
) -> i64 {
    sqlx::query_scalar(
        r#"INSERT INTO audit_log(
               action,category,details,project_path,source,status,detail,
               run_id,target_id,audit_role
           ) VALUES($1,'harness',$2,$3,'harness','completed',$4,$5,$6,'evidence')
           RETURNING id"#,
    )
    .bind(label)
    .bind(format!("{label} evidence"))
    .bind(project_path)
    .bind(json!({"organization_id": organization_id}))
    .bind(operation_id)
    .bind(target_id)
    .fetch_one(pool)
    .await
    .expect("insert exact operation/org evidence")
}

#[allow(clippy::too_many_arguments)]
async fn insert_passed_stage(
    pool: &PgPool,
    session_id: Uuid,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    stage_kind: &str,
    specialist: &str,
    evidence_ids: Vec<i64>,
) -> StageSeal {
    let stage_execution_id = Uuid::new_v4();
    let stage_run_unit_id = Uuid::new_v4();
    let worker_run_id = Uuid::new_v4();
    let lease_token = Uuid::new_v4();
    let tool_call_id = Uuid::new_v4();
    let submission_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status,completed_at) \
         VALUES($1,$2,$3,'completed',NOW())",
    )
    .bind(stage_execution_id)
    .bind(operation_id)
    .bind(stage_kind)
    .execute(pool)
    .await
    .expect("insert stage run");
    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,
               organization_id,stage_kind,generation,specialist,status,
               terminal_at,pass_watermark
           ) VALUES($1,$2,$3,$4,$5,$6,0,$7,'passed',NOW(),$8)"#,
    )
    .bind(stage_run_unit_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(stage_kind)
    .bind(specialist)
    .bind(json!({"final_gate_passed": true, "deliverable_submission_id": submission_id}))
    .execute(pool)
    .await
    .expect("insert passed stage unit");
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,organization_id,
               worker_generation,specialist,work_item_kind,work_item_key,agent_path,
               status,lease_token,lease_owner,lease_acquired_at,lease_expires_at,
               heartbeat_at,attempt_epoch
           ) VALUES($1,$2,$3,$4,$5,0,$6,'stage_unit',$7,$8,'running',$9,
                    'v2-closeout-fixture',NOW(),NOW()+INTERVAL '5 minutes',NOW(),0)"#,
    )
    .bind(worker_run_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(organization_id)
    .bind(specialist)
    .bind(format!("{stage_kind}:{organization_id}"))
    .bind(format!("main>{stage_kind}"))
    .bind(lease_token)
    .execute(pool)
    .await
    .expect("insert source worker");
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,result,status,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token
           ) VALUES($1,$2,$3,$4,'primary','submit_stage_deliverable','{}','{}','finished',
                    $4,$5,$6,$7,$8,0,$9)"#,
    )
    .bind(tool_call_id)
    .bind(format!("{stage_kind}-submit"))
    .bind(session_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(worker_run_id)
    .bind(organization_id)
    .bind(lease_token)
    .execute(pool)
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
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(worker_run_id)
    .bind(organization_id)
    .bind(tool_call_id)
    .bind(format!("{stage_kind}-submit"))
    .bind(stage_kind)
    .bind(lease_token)
    .bind(json!({"schema_version": 1, "candidates": []}))
    .bind(format!("sha256:{stage_kind}-submission"))
    .execute(pool)
    .await
    .expect("insert deliverable submission");
    sqlx::query(
        r#"INSERT INTO stage_handoffs(
               id,operation_id,organization_id,scope_snapshot_id,from_stage_kind,
               stage_execution_id,source_stage_run_unit_id,deliverable_submission_id,
               scope_hash,payload,payload_sha256,evidence_ids,
               unit_gate_decision_hash,gate_passed_at
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,'sha256:scope',$9,$10,$11,$12,NOW())"#,
    )
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(organization_id)
    .bind(scope_snapshot_id)
    .bind(stage_kind)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(submission_id)
    .bind(json!({
        "canonical_fact_refs": [],
        "typed_claims": [],
        "coverage_watermark": {},
        "evidence_ids": evidence_ids,
    }))
    .bind(format!("sha256:{stage_kind}-handoff"))
    .bind(&evidence_ids)
    .bind(format!("sha256:{stage_kind}-gate"))
    .execute(pool)
    .await
    .expect("insert final stage handoff");
    StageSeal {
        stage_execution_id,
        stage_run_unit_id,
        submission_id,
    }
}

async fn seed_submitted_candidate(pool: &PgPool, project_path: &str) -> CandidateSubmissionFixture {
    let session_id = Uuid::new_v4();
    let operation_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    let scope_decision_id = Uuid::new_v4();
    let scope_snapshot_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let target_id = Uuid::new_v4();
    let target_identity_hash = target_identity_hash("url", "https://closeout.example.test/login");
    sqlx::query(
        "INSERT INTO sessions(id,title,status,project_path) \
         VALUES($1,'V2 closeout contract','running',$2)",
    )
    .bind(session_id)
    .bind(project_path)
    .execute(pool)
    .await
    .expect("insert session");
    sqlx::query(
        "INSERT INTO tasks(id,session_id,title,input,status) \
         VALUES($1,$2,'V2 closeout operation','candidate to report','running')",
    )
    .bind(operation_id)
    .bind(session_id)
    .execute(pool)
    .await
    .expect("insert operation task");
    sqlx::query(
        r#"INSERT INTO project_scopes(project_scope_id,canonical_project_path,path_sha256)
           VALUES($1,$2,$3)"#,
    )
    .bind(project_scope_id)
    .bind(project_path)
    .bind("4".repeat(64))
    .execute(pool)
    .await
    .expect("insert project scope");
    // This integration test exercises the post-cutover closeout path, not the
    // rollout promotion algorithm (covered by attack_rollout_cohort_migrations).
    // Position both deployment singletons at an already-promoted V2-only
    // snapshot before freezing the operation contract.
    let mut rollout_tx = pool.begin().await.expect("begin V2 rollout fixture");
    sqlx::query("SET LOCAL session_replication_role = 'replica'")
        .execute(&mut *rollout_tx)
        .await
        .expect("isolate rollout promotion fixture");
    sqlx::query(
        r#"UPDATE runtime_memory_rollout
              SET contract='v2_only',contract_rank=3,row_version=3,updated_at=NOW()
            WHERE singleton_id=1"#,
    )
    .execute(&mut *rollout_tx)
    .await
    .expect("position runtime rollout at V2-only");
    sqlx::query(
        r#"UPDATE attack_execution_rollout
              SET contract='v2_only',rank=3,row_version=3,updated_at=NOW()
            WHERE singleton=TRUE"#,
    )
    .execute(&mut *rollout_tx)
    .await
    .expect("position attack rollout at V2-only");
    sqlx::query("SET LOCAL session_replication_role = 'origin'")
        .execute(&mut *rollout_tx)
        .await
        .expect("restore rollout trigger authority");
    rollout_tx
        .commit()
        .await
        .expect("commit V2 rollout fixture");
    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,
               attack_execution_contract,project_scope_id
           ) VALUES($1,'red_team','attack_candidate','v2_only','v2_only',$2)"#,
    )
    .bind(operation_id)
    .bind(project_scope_id)
    .execute(pool)
    .await
    .expect("insert V2 operation state");
    sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$2,'Closeout Org')")
        .bind(organization_id)
        .bind(project_path)
        .execute(pool)
        .await
        .expect("insert organization");
    sqlx::query(
        r#"INSERT INTO targets(id,name,target_type,value,scope,project_path,organization_id)
           VALUES($1,'Closeout target','url','https://closeout.example.test/login','in',$2,$3)"#,
    )
    .bind(target_id)
    .bind(project_path)
    .bind(organization_id)
    .execute(pool)
    .await
    .expect("insert target");

    let decision_stage_execution_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status,completed_at) \
         VALUES($1,$2,'scoping','completed',NOW())",
    )
    .bind(decision_stage_execution_id)
    .bind(operation_id)
    .execute(pool)
    .await
    .expect("insert scope decision stage");
    sqlx::query(
        r#"INSERT INTO operation_scope_decisions(
               id,operation_id,project_scope_id,stage_execution_id,
               root_organization_id,mode,decision_rows,decision_hash
           ) VALUES($1,$2,$3,$4,$5,'cli_flags',$6,$7)"#,
    )
    .bind(scope_decision_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(decision_stage_execution_id)
    .bind(organization_id)
    .bind(json!([{"organization_id": organization_id}]))
    .bind("5".repeat(64))
    .execute(pool)
    .await
    .expect("insert scope decision");
    let mut scope_tx = pool.begin().await.expect("begin scope freeze");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_snapshots(
               id,operation_id,project_scope_id,scope_decision_id,
               project_path_at_freeze,root_organization_id,mode,scope_hash
           ) VALUES($1,$2,$3,$4,$5,$6,'cli_flags',$7)"#,
    )
    .bind(scope_snapshot_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(scope_decision_id)
    .bind(project_path)
    .bind(organization_id)
    .bind("6".repeat(64))
    .execute(&mut *scope_tx)
    .await
    .expect("insert scope snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units(
               snapshot_id,organization_id,organization_name_at_freeze,role,
               depth,ordinal,decision_row_id,approval_source
           ) VALUES($1,$2,'Closeout Org','root',0,0,'root',$3)"#,
    )
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(json!({"source": "cli_flags"}))
    .execute(&mut *scope_tx)
    .await
    .expect("insert scope unit");
    sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
        .bind(scope_snapshot_id)
        .execute(&mut *scope_tx)
        .await
        .expect("seal scope");
    scope_tx.commit().await.expect("commit scope freeze");

    let entry_evidence_id = insert_evidence(
        pool,
        operation_id,
        organization_id,
        Some(target_id),
        "entry",
        project_path,
    )
    .await;
    let entry = insert_passed_stage(
        pool,
        session_id,
        operation_id,
        scope_snapshot_id,
        organization_id,
        "vuln_triage",
        "formulaic_scanner",
        vec![entry_evidence_id],
    )
    .await;
    let wave_run_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{operation_id}:candidate-wave:0").as_bytes(),
    );
    let wave_unit_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO attack_wave_runs(
               id,operation_id,scope_snapshot_id,generation,status,policy_snapshot,
               policy_hash,max_waves,max_candidates_total,max_chain_depth,max_attempts_total
           ) VALUES(
               $1,$2,$3,0,'open',$4,
               'sha256:66e50329b4bb217eb060bcccb38f78f4b0eafc163471bebd6554d271d1a6b326',
               3,100,3,200
           )"#,
    )
    .bind(wave_run_id)
    .bind(operation_id)
    .bind(scope_snapshot_id)
    .bind(json!({"max_waves":3,"max_candidates_total":100,"max_chain_depth":3,"max_attempts_total":200}))
    .execute(pool)
    .await
    .expect("insert wave run");
    sqlx::query(
        r#"INSERT INTO attack_wave_units(
               id,wave_run_id,operation_id,scope_snapshot_id,organization_id,
               entry_stage_execution_id,entry_stage_run_unit_id,
               entry_deliverable_submission_id,entry_stage_kind,ordinal,status
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,'vuln_triage',0,'open')"#,
    )
    .bind(wave_unit_id)
    .bind(wave_run_id)
    .bind(operation_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(entry.stage_execution_id)
    .bind(entry.stage_run_unit_id)
    .bind(entry.submission_id)
    .execute(pool)
    .await
    .expect("insert wave unit");

    // Keep the real Wave-before-Candidate ordering even though a V2-only
    // operation no longer participates in the dual-read admission cohort.
    let decision = insert_passed_stage(
        pool,
        session_id,
        operation_id,
        scope_snapshot_id,
        organization_id,
        "attack_candidate",
        "attack_analyst",
        Vec::new(),
    )
    .await;

    let seed_id = Uuid::new_v4();
    let work_item_id = Uuid::new_v4();
    let candidate_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO attack_candidate_seeds(
               id,wave_unit_id,operation_id,scope_snapshot_id,organization_id,
               target_live_id,target_type_at_time,target_value_at_time,
               target_identity_hash,technique,observation,observation_hash
           ) VALUES($1,$2,$3,$4,$5,$6,'url','https://closeout.example.test/login',
                    $7,'WSTG-INPV-05',$8,'sha256:observation')"#,
    )
    .bind(seed_id)
    .bind(wave_unit_id)
    .bind(operation_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(target_id)
    .bind(&target_identity_hash)
    .bind(json!({"parameter":"username"}))
    .execute(pool)
    .await
    .expect("insert candidate seed");
    sqlx::query(
        r#"INSERT INTO attack_candidate_work_items(
               id,seed_id,wave_unit_id,operation_id,scope_snapshot_id,organization_id,
               target_live_id,target_type_at_time,target_value_at_time,
               target_identity_hash,work_item_key
           ) VALUES($1,$2,$3,$4,$5,$6,$7,'url','https://closeout.example.test/login',
                    $8,$9)"#,
    )
    .bind(work_item_id)
    .bind(seed_id)
    .bind(wave_unit_id)
    .bind(operation_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(target_id)
    .bind(&target_identity_hash)
    .bind(format!("seed:{seed_id}:v1:sha256:observation"))
    .execute(pool)
    .await
    .expect("insert pending candidate work item");
    let execution_plan = json!({
        "schema_version": "candidate-plan-v1",
        "classifier_version": "candidate-classifier-v1",
        "candidate_id": candidate_id,
        "target_identity_hash": target_identity_hash.clone(),
        "foreground_only": true,
        "actions": [{
            "ordinal": 0,
            "capability_id": "verify.sql_injection",
            "action_kind": "bounded_sql_injection_probe",
            "canonical_args": {"target":"https://closeout.example.test/login"},
            "side_effect_class": "exploit",
            "required_evidence_role": "proof"
        }],
        "budget": {"max_actions":1,"max_requests":8,"max_runtime_ms":120000}
    });
    let candidate_plan_hash: String =
        sqlx::query_scalar("SELECT 'sha256:' || attack_fact_delta_sha256_jsonb($1::jsonb)")
            .bind(&execution_plan)
            .fetch_one(pool)
            .await
            .expect("derive canonical Candidate plan hash");
    let mut candidate_tx = pool.begin().await.expect("begin candidate acceptance");
    sqlx::query(
        r#"INSERT INTO attack_candidates(
               candidate_id,operation_id,organization_id,target,hypothesis,
               hypothesis_hash,technique,rationale,prior_refs,suggested_approach,
               priority,wave,disposition,operation_uuid,scope_snapshot_id,
               wave_run_id,wave_unit_id,source_work_item_id,
               decision_stage_execution_id,decision_stage_run_unit_id,
               decision_deliverable_submission_id,decision_stage_kind,
               target_live_id,target_type_at_time,target_value_at_time,
               target_identity_hash,execution_plan,candidate_plan_hash,risk_class
           ) VALUES($1,$2,$3,'https://closeout.example.test/login','SQL injection hypothesis',
                    'sha256:closeout-hypothesis','WSTG-INPV-05','evidence grounded','[]',
                    'bounded verifier','high',0,'approved',$4,$5,$6,$7,$8,$9,$10,$11,
                    'attack_candidate',$12,'url','https://closeout.example.test/login',
                    $13,$14,$15,'exploit')"#,
    )
    .bind(candidate_id)
    .bind(operation_id.to_string())
    .bind(organization_id)
    .bind(operation_id)
    .bind(scope_snapshot_id)
    .bind(wave_run_id)
    .bind(wave_unit_id)
    .bind(work_item_id)
    .bind(decision.stage_execution_id)
    .bind(decision.stage_run_unit_id)
    .bind(decision.submission_id)
    .bind(target_id)
    .bind(&target_identity_hash)
    .bind(&execution_plan)
    .bind(&candidate_plan_hash)
    .execute(&mut *candidate_tx)
    .await
    .expect("insert approved candidate");
    sqlx::query(
        r#"UPDATE attack_candidate_work_items
              SET decision_kind='candidate',candidate_id=$2,decided_at=NOW()
            WHERE id=$1"#,
    )
    .bind(work_item_id)
    .bind(candidate_id)
    .execute(&mut *candidate_tx)
    .await
    .expect("terminalize candidate work item");
    candidate_tx
        .commit()
        .await
        .expect("commit candidate acceptance");

    let principal = operator_principals::current_local(pool)
        .await
        .expect("load local operator");
    let approval_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO attack_candidate_approvals(
               id,candidate_id,operation_id,scope_snapshot_id,wave_run_id,wave_unit_id,
               organization_id,target_live_id,target_type_at_time,target_value_at_time,
               target_identity_hash,candidate_plan_hash,source_work_item_id,execution_plan,
               allowed_capability_ids,allowed_action_kinds,budget,expires_at,
               decision_version,status,decided_by
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,'url','https://closeout.example.test/login',
                    $9,$10,$11,$12,$13,$14,$15,
                    NOW()+INTERVAL '1 hour',1,'approved',$16)"#,
    )
    .bind(approval_id)
    .bind(candidate_id)
    .bind(operation_id)
    .bind(scope_snapshot_id)
    .bind(wave_run_id)
    .bind(wave_unit_id)
    .bind(organization_id)
    .bind(target_id)
    .bind(&target_identity_hash)
    .bind(&candidate_plan_hash)
    .bind(work_item_id)
    .bind(&execution_plan)
    .bind(vec!["verify.sql_injection"])
    .bind(vec!["bounded_sql_injection_probe"])
    .bind(execution_plan["budget"].clone())
    .bind(principal.id)
    .execute(pool)
    .await
    .expect("insert approved plan snapshot");
    sqlx::query(
        "UPDATE attack_wave_units SET review_closed=TRUE,status='verification' WHERE id=$1",
    )
    .bind(wave_unit_id)
    .execute(pool)
    .await
    .expect("close candidate review");
    sqlx::query("UPDATE attack_wave_runs SET status='verification' WHERE id=$1")
        .bind(wave_run_id)
        .execute(pool)
        .await
        .expect("advance wave to verification");
    let verification_stage_execution_id = Uuid::new_v4();
    let verification_stage_run_unit_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status) \
         VALUES($1,$2,'verification','started')",
    )
    .bind(verification_stage_execution_id)
    .bind(operation_id)
    .execute(pool)
    .await
    .expect("insert verification stage");
    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,organization_id,
               stage_kind,generation,specialist,status
           ) VALUES($1,$2,$3,$4,$5,'verification',0,'candidate_verifier','running')"#,
    )
    .bind(verification_stage_run_unit_id)
    .bind(operation_id)
    .bind(verification_stage_execution_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .execute(pool)
    .await
    .expect("insert verification unit");
    let claimed = claim_next_candidate_attempt(
        pool,
        CandidateClaimQuery {
            operation_id,
            scope_snapshot_id,
            wave_run_id,
            wave_unit_id,
            organization_id,
            verification_stage_execution_id,
            verification_stage_run_unit_id,
            lease_owner: "v2-closeout".to_string(),
            lease_seconds: 120,
        },
    )
    .await
    .expect("claim candidate attempt")
    .expect("approved candidate is claimable");
    sqlx::query(
        r#"INSERT INTO candidate_attempt_actions(
               attempt_id,action_ordinal,capability_id,action_kind,canonical_args,
               status,outcome,outcome_hash,started_at,completed_at
           ) VALUES($1,0,'verify.sql_injection','bounded_sql_injection_probe',$2,
                    'completed','{}','sha256:verified',NOW(),NOW())"#,
    )
    .bind(claimed.attempt.id)
    .bind(json!({"target":"https://closeout.example.test/login"}))
    .execute(pool)
    .await
    .expect("record terminal candidate action journal");
    let proof_evidence_id = insert_evidence(
        pool,
        operation_id,
        organization_id,
        Some(target_id),
        "candidate proof",
        project_path,
    )
    .await;
    let result_json = json!({
        "disposition":"verified",
        "proof_evidence_ids":[proof_evidence_id],
        "finding":{
            "title":"Verified bounded SQL injection",
            "severity":"high",
            "cvss":8.1,
            "affected_target":"https://closeout.example.test/login",
            "description":"Deterministic verifier reproduced the bounded condition.",
            "steps":"Replay the evidence-backed bounded action journal.",
            "remediation":"Use parameterized queries and least privilege."
        }
    });
    let lease_token = claimed.worker.lease_token.expect("candidate lease token");
    let mut submission_tx = pool.begin().await.expect("begin candidate submission");
    let submitted = record_attempt_submission(
        &mut submission_tx,
        RecordAttemptSubmission {
            operation_id,
            scope_snapshot_id,
            wave_run_id,
            wave_unit_id,
            organization_id,
            candidate_id,
            approval_id,
            attempt_id: claimed.attempt.id,
            candidate_plan_hash: candidate_plan_hash.clone(),
            worker_run_id: claimed.worker.id,
            stage_execution_id: verification_stage_execution_id,
            stage_run_unit_id: verification_stage_run_unit_id,
            lease_token,
            lease_owner: "v2-closeout".to_string(),
            attempt_epoch: claimed.worker.attempt_epoch,
            expected_checkpoint_version: claimed.worker.checkpoint_version,
            result_json,
            evidence: vec![AttemptEvidenceLink {
                evidence_id: proof_evidence_id,
                role: "proof".to_string(),
            }],
        },
    )
    .await
    .expect("record immutable candidate result");
    submission_tx
        .commit()
        .await
        .expect("commit candidate result");
    let terminalize = TerminalizeVerifiedFinding {
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        candidate_id,
        approval_id,
        attempt_id: claimed.attempt.id,
        candidate_plan_hash,
        expected_result_hash: submitted.attempt.result_hash.expect("result hash"),
        proof_evidence_ids: vec![proof_evidence_id],
        worker_run_id: claimed.worker.id,
        stage_execution_id: verification_stage_execution_id,
        stage_run_unit_id: verification_stage_run_unit_id,
        lease_token,
        lease_owner: "v2-closeout".to_string(),
        attempt_epoch: claimed.worker.attempt_epoch,
        expected_checkpoint_version: claimed.worker.checkpoint_version,
    };
    CandidateSubmissionFixture {
        operation_id,
        project_scope_id,
        scope_snapshot_id,
        organization_id,
        target_id,
        attempt_id: claimed.attempt.id,
        proof_evidence_id,
        terminalize,
    }
}

async fn install_first_assertion_ack_loss(pool: &PgPool, event_id: Uuid) {
    sqlx::query(
        "UPDATE knowledge_projector_registry SET lifecycle='enabled',disabled_reason=NULL \
         WHERE projector_name=$1 AND projector_schema_version=$2",
    )
    .bind(ProjectorId::AssertionPromoterV1.name())
    .bind(ProjectorId::AssertionPromoterV1.schema_version())
    .execute(pool)
    .await
    .expect("enable projector fixture");
    sqlx::raw_sql(&format!(
        r#"CREATE SEQUENCE assertion_ack_loss_attempts START WITH 1;
           CREATE FUNCTION reject_first_assertion_ack_after_write()
           RETURNS trigger AS $$
           BEGIN
               IF NEW.event_id='{event_id}'::uuid
                  AND NEW.projector_name='assertion-promoter'
                  AND NEW.projector_schema_version=1
                  AND OLD.attempt_count=1
                  AND NEW.status='succeeded'
               THEN
                   PERFORM nextval('assertion_ack_loss_attempts');
                   RAISE EXCEPTION 'fixture lost assertion ACK after projector write';
               END IF;
               RETURN NEW;
           END;
           $$ LANGUAGE plpgsql;
           CREATE TRIGGER reject_first_assertion_ack_after_write
           BEFORE UPDATE ON knowledge_projection_deliveries
           FOR EACH ROW EXECUTE FUNCTION reject_first_assertion_ack_after_write();"#,
    ))
    .execute(pool)
    .await
    .expect("install first assertion ACK-loss fixture");
}

async fn wait_for_real_assertion_write_then_replay(pool: &PgPool, event_id: Uuid) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let (projection_count, status, attempt_count, ack_rejected): (i64, String, i32, bool) =
                sqlx::query_as(
                    r#"SELECT
                           (SELECT COUNT(*)
                              FROM knowledge_assertions AS assertion
                              JOIN knowledge_outbox_events AS event
                                ON event.source_stream_key=assertion.source_stream_key
                               AND event.source_version=assertion.source_version
                             WHERE event.event_id=$1),
                           status,
                           attempt_count,
                           (SELECT is_called FROM assertion_ack_loss_attempts)
                      FROM knowledge_projection_deliveries
                     WHERE event_id=$1
                       AND projector_name='assertion-promoter'
                       AND projector_schema_version=1"#,
                )
                .bind(event_id)
                .fetch_one(pool)
                .await
                .expect("observe write-before-ACK delivery state");
            if projection_count == 1 && status == "leased" && attempt_count == 1 && ack_rejected {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("real assertion write must commit before its first ACK is lost");

    sqlx::raw_sql(
        r#"DROP TRIGGER reject_first_assertion_ack_after_write
             ON knowledge_projection_deliveries;
           DROP FUNCTION reject_first_assertion_ack_after_write();
           DROP SEQUENCE assertion_ack_loss_attempts;"#,
    )
    .execute(pool)
    .await
    .expect("remove assertion ACK-loss fixture before replay");
    let expired = sqlx::query(
        r#"UPDATE knowledge_projection_deliveries
              SET lease_expires_at=NOW()-INTERVAL '1 second'
            WHERE event_id=$1
              AND projector_name='assertion-promoter'
              AND projector_schema_version=1
              AND status='leased'
              AND attempt_count=1"#,
    )
    .bind(event_id)
    .execute(pool)
    .await
    .expect("expire lost-ACK assertion lease");
    assert_eq!(
        expired.rows_affected(),
        1,
        "the exact first lost-ACK delivery must be reclaimed"
    );
}

struct Fake1536Embedder;

#[async_trait]
impl Embedder for Fake1536Embedder {
    async fn embed(&self, _text: &str) -> anyhow::Result<Vec<f32>> {
        Ok(vec![0.25; EMBEDDING_DIMENSION_V1])
    }

    fn dimension(&self) -> usize {
        EMBEDDING_DIMENSION_V1
    }

    fn model_name(&self) -> &str {
        "fake-1536-v1"
    }
}

async fn wait_for_exact_projection_dag(pool: &PgPool, event_ids: &[Uuid]) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let mut complete = true;
            for event_id in event_ids {
                let deliveries = knowledge_outbox::list_deliveries(pool, *event_id)
                    .await
                    .expect("load production projector deliveries");
                complete &= deliveries.len() == 4
                    && deliveries.iter().all(|delivery| {
                        delivery.status == knowledge_outbox::DeliveryStatus::Succeeded
                    });
            }
            if complete {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("production projector DAG must persist every routed projection");
}

async fn assert_one_projection_version(
    pool: &PgPool,
    event_id: Uuid,
    expected_assertion_attempts: i32,
) {
    let event = knowledge_outbox::get_event(pool, event_id)
        .await
        .expect("load canonical projection event");
    let stream = &event.payload.source_stream_key;
    let version = event.payload.source_version;
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM knowledge_outbox_events \
             WHERE source_stream_key=$1 AND source_version=$2",
        )
        .bind(stream)
        .bind(version)
        .fetch_one(pool)
        .await
        .expect("count canonical event source version"),
        1,
        "producer replay must retain one canonical event source version"
    );
    let projection_counts: (i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT
               (SELECT COUNT(*) FROM knowledge_assertions
                 WHERE source_stream_key=$1 AND source_version=$2),
               (SELECT COUNT(*) FROM knowledge_documents
                 WHERE source_stream_key=$1 AND source_version=$2),
               (SELECT COUNT(*) FROM knowledge_embeddings
                 WHERE source_stream_key=$1 AND source_version=$2),
               (SELECT COUNT(*)
                  FROM knowledge_graph_entity_assertions AS lineage
                  JOIN knowledge_assertions AS assertion
                    ON assertion.assertion_id=lineage.assertion_id
                 WHERE assertion.source_stream_key=$1
                   AND assertion.source_version=$2)"#,
    )
    .bind(stream)
    .bind(version)
    .fetch_one(pool)
    .await
    .expect("count exact projection version");
    assert_eq!(
        projection_counts,
        (1, 1, 1, 1),
        "one event source version must produce one idempotent row per projection layer"
    );
    let assertion_authority: (Uuid, Uuid, Uuid, String, String, i64) = sqlx::query_as(
        r#"SELECT source_operation_id,project_scope_id,organization_id_at_time,
                  source_kind,source_id_value,source_version
             FROM knowledge_assertions
            WHERE source_stream_key=$1 AND source_version=$2"#,
    )
    .bind(stream)
    .bind(version)
    .fetch_one(pool)
    .await
    .expect("load derived assertion authority");
    let expected_source_id = match &event.payload.source.row_id {
        golish_memory_domain::CanonicalRowId::Uuid(source_id) => source_id.to_string(),
        _ => panic!("closeout acceptance events use UUID canonical sources"),
    };
    assert_eq!(assertion_authority.0, event.source_operation_id);
    assert_eq!(
        Some(assertion_authority.1),
        event.project_scope_id.map(|id| id.0)
    );
    assert_eq!(Some(assertion_authority.2), event.organization_id_at_time);
    assert_eq!(
        assertion_authority.3,
        event.payload.source.source_kind.as_str()
    );
    assert_eq!(assertion_authority.4, expected_source_id);
    assert_eq!(assertion_authority.5, event.payload.source.version);
    let deliveries = knowledge_outbox::list_deliveries(pool, event_id)
        .await
        .expect("load completed projection deliveries");
    assert_eq!(deliveries.len(), 4);
    assert!(deliveries.iter().all(|delivery| {
        delivery.status == knowledge_outbox::DeliveryStatus::Succeeded
            && delivery.terminal_reason.is_none()
    }));
    assert_eq!(
        deliveries
            .iter()
            .find(|delivery| delivery.projector_name == "assertion-promoter")
            .expect("assertion delivery")
            .attempt_count,
        expected_assertion_attempts
    );
}

fn assert_database_rejection(error: sqlx::Error, code: &str, mutation: &str) {
    assert!(
        error.to_string().contains(code),
        "{mutation} returned the wrong database rejection: {error}"
    );
}

async fn assert_finalized_report_history_is_immutable(pool: &PgPool, revision_id: Uuid) {
    let (claim_id, section_id): (Uuid, Uuid) = sqlx::query_as(
        "SELECT claim_id,section_id FROM report_claims WHERE revision_id=$1 ORDER BY ordinal LIMIT 1",
    )
    .bind(revision_id)
    .fetch_one(pool)
    .await
    .expect("final report contains at least one cited claim");
    let (artifact_kind, content_key): (String, String) = sqlx::query_as(
        "SELECT artifact_kind,content_key FROM report_revision_artifacts \
         WHERE revision_id=$1 ORDER BY artifact_kind LIMIT 1",
    )
    .bind(revision_id)
    .fetch_one(pool)
    .await
    .expect("final report contains at least one attached artifact");

    for (mutation, statement) in [
        (
            "final revision no-op update",
            "UPDATE report_revisions SET publication_status=publication_status WHERE revision_id=$1",
        ),
        (
            "final revision delete",
            "DELETE FROM report_revisions WHERE revision_id=$1",
        ),
        (
            "final claim no-op update",
            "UPDATE report_claims SET ordinal=ordinal WHERE revision_id=$1",
        ),
        (
            "final claim delete",
            "DELETE FROM report_claims WHERE revision_id=$1",
        ),
        (
            "final artifact no-op update",
            "UPDATE report_revision_artifacts SET redaction_version=redaction_version WHERE revision_id=$1",
        ),
        (
            "final artifact delete",
            "DELETE FROM report_revision_artifacts WHERE revision_id=$1",
        ),
    ] {
        let error = sqlx::query(statement)
            .bind(revision_id)
            .execute(pool)
            .await
            .expect_err(mutation);
        assert_database_rejection(error, "FINAL_HISTORY_IMMUTABLE", mutation);
    }

    let claim_insert = sqlx::query(
        r#"INSERT INTO report_claims(
               claim_id,revision_id,section_id,claim_kind,subject_ref,
               predicate,object_value,claim_hash,ordinal
           ) VALUES($1,$2,$3,'finding','late-claim','verified','{}',$4,2147483646)"#,
    )
    .bind(Uuid::new_v4())
    .bind(revision_id)
    .bind(section_id)
    .bind("c".repeat(64))
    .execute(pool)
    .await
    .expect_err("final claim insert");
    assert_database_rejection(
        claim_insert,
        "FINAL_HISTORY_IMMUTABLE",
        "final claim insert",
    );

    let artifact_insert = sqlx::query(
        r#"INSERT INTO report_revision_artifacts(
               revision_id,artifact_kind,content_key,redaction_version
           ) VALUES($1,'pdf',$2,1)"#,
    )
    .bind(revision_id)
    .bind(&content_key)
    .execute(pool)
    .await
    .expect_err("final artifact insert");
    assert_database_rejection(
        artifact_insert,
        "FINAL_HISTORY_IMMUTABLE",
        "final artifact insert",
    );

    let blob_update =
        sqlx::query("UPDATE report_artifact_blobs SET byte_len=byte_len WHERE content_key=$1")
            .bind(&content_key)
            .execute(pool)
            .await
            .expect_err("referenced artifact blob no-op update");
    assert_database_rejection(
        blob_update,
        "REPORT_ARTIFACT_BLOB_IMMUTABLE",
        "referenced artifact blob no-op update",
    );
    sqlx::query("DELETE FROM report_artifact_blobs WHERE content_key=$1")
        .bind(&content_key)
        .execute(pool)
        .await
        .expect_err("referenced artifact blob delete must be restricted");

    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            r#"SELECT
                   (SELECT COUNT(*) FROM report_revisions WHERE revision_id=$1)
                 + (SELECT COUNT(*) FROM report_claims WHERE claim_id=$2)
                 + (SELECT COUNT(*) FROM report_revision_artifacts
                     WHERE revision_id=$1 AND artifact_kind=$3 AND content_key=$4)
                 + (SELECT COUNT(*) FROM report_artifact_blobs WHERE content_key=$4)"#,
        )
        .bind(revision_id)
        .bind(claim_id)
        .bind(&artifact_kind)
        .bind(&content_key)
        .fetch_one(pool)
        .await
        .expect("count retained final history rows"),
        4,
        "rejected in-place mutations must retain revision, claim, attachment and blob"
    );
}

#[tokio::test]
#[serial]
async fn candidate_to_report_closeout_is_replay_safe() {
    let (mut db, _data_dir) = fixture("candidate_to_report").await;
    let project_dir = tempfile::tempdir().expect("temporary canonical project root");
    let canonical_project_root =
        std::fs::canonicalize(project_dir.path()).expect("canonicalize temporary project root");
    let project_path = canonical_project_root
        .to_str()
        .expect("temporary project path is UTF-8");
    let scope = seed_submitted_candidate(db.pool(), project_path).await;
    sqlx::raw_sql(
        r#"CREATE FUNCTION reject_candidate_terminal_outbox_cross_contract()
           RETURNS trigger AS $$
           BEGIN
               IF NEW.event_name='CandidateAttemptTerminal.v1' THEN
                   RAISE EXCEPTION 'cross contract candidate outbox failure';
               END IF;
               RETURN NEW;
           END;
           $$ LANGUAGE plpgsql;
           CREATE TRIGGER reject_candidate_terminal_outbox_cross_contract
           BEFORE INSERT ON knowledge_outbox_events
           FOR EACH ROW EXECUTE FUNCTION reject_candidate_terminal_outbox_cross_contract();"#,
    )
    .execute(db.pool())
    .await
    .expect("install outbox failure");
    let mut rejected_tx = db
        .pool()
        .begin()
        .await
        .expect("begin rejected terminalization");
    terminalize_verified_finding(&mut rejected_tx, scope.terminalize.clone())
        .await
        .expect_err("outbox failure must reject canonical terminalization");
    rejected_tx
        .rollback()
        .await
        .expect("rollback terminalization");
    let rolled_back: (String, String, i64) = sqlx::query_as(
        r#"SELECT attempt.status,candidate.disposition,
                  (SELECT COUNT(*) FROM knowledge_outbox_events
                    WHERE event_name='CandidateAttemptTerminal.v1'
                      AND source_id_value=$2)
             FROM candidate_attempts AS attempt
             JOIN attack_candidates AS candidate ON candidate.candidate_id=attempt.candidate_id
            WHERE attempt.id=$1"#,
    )
    .bind(scope.attempt_id)
    .bind(scope.attempt_id.to_string())
    .fetch_one(db.pool())
    .await
    .expect("read rolled back candidate");
    assert_eq!(
        rolled_back,
        ("submitted".to_string(), "approved".to_string(), 0)
    );
    sqlx::raw_sql(
        r#"DROP TRIGGER reject_candidate_terminal_outbox_cross_contract ON knowledge_outbox_events;
           DROP FUNCTION reject_candidate_terminal_outbox_cross_contract();"#,
    )
    .execute(db.pool())
    .await
    .expect("remove outbox failure");

    let mut terminal_tx = db.pool().begin().await.expect("begin terminalization");
    let terminal = terminalize_verified_finding(&mut terminal_tx, scope.terminalize.clone())
        .await
        .expect("terminalize verified finding");
    terminal_tx.commit().await.expect("commit terminalization");
    let mut replay_tx = db.pool().begin().await.expect("begin terminal replay");
    let replay = terminalize_verified_finding(&mut replay_tx, scope.terminalize.clone())
        .await
        .expect("replay terminalization");
    replay_tx.commit().await.expect("commit terminal replay");
    assert!(replay.replayed);
    assert_eq!(replay.finding_id, terminal.finding_id);
    let event_id = Uuid::new_v5(&scope.attempt_id, b"CandidateAttemptTerminal.v1");
    assert_eq!(
        knowledge_outbox::list_deliveries(db.pool(), event_id)
            .await
            .unwrap()
            .len(),
        4
    );
    install_first_assertion_ack_loss(db.pool(), event_id).await;
    let embedding = KnowledgeEmbeddingProvider::new("fake-local", Arc::new(Fake1536Embedder))
        .expect("construct deterministic 1536-dimension provider");
    let candidate_memory_runtime =
        KnowledgeMemoryRuntime::new(Arc::new(db.pool().clone()), Some(embedding.clone()));
    candidate_memory_runtime
        .start()
        .await
        .expect("start production projector supervisor for Candidate replay");
    wait_for_real_assertion_write_then_replay(db.pool(), event_id).await;
    wait_for_exact_projection_dag(db.pool(), &[event_id]).await;
    assert_one_projection_version(db.pool(), event_id, 2).await;
    candidate_memory_runtime
        .shutdown()
        .await
        .expect("stop Candidate replay projector supervisor");

    let target_snapshot = footholds::load_access_validation_source(
        db.pool(),
        scope.operation_id,
        scope.scope_snapshot_id,
        scope.organization_id,
        "candidate_attempt",
        scope.attempt_id,
    )
    .await
    .expect("load trusted Candidate target snapshot for Post-Exploit");
    let foothold_id = Uuid::new_v4();
    let validate_foothold = footholds::ValidateFoothold {
        id: foothold_id,
        operation_id: scope.operation_id,
        project_scope_id: scope.project_scope_id,
        scope_snapshot_id: scope.scope_snapshot_id,
        organization_id_at_time: scope.organization_id,
        validation_unit_kind: "candidate_attempt".to_string(),
        validation_unit_id: scope.attempt_id,
        target_live_id: Some(scope.target_id),
        target_type_at_time: target_snapshot.target_type_at_time,
        target_value_at_time: target_snapshot.target_value_at_time,
        target_identity_hash: target_snapshot.target_identity_hash,
        vault_credential_ref: None,
        evidence: vec![(scope.proof_evidence_id, "validation".to_string())],
    };
    let foothold = footholds::validate_and_create(db.pool(), &validate_foothold)
        .await
        .expect("emit canonical foothold terminal event");
    assert_eq!(
        footholds::validate_and_create(db.pool(), &validate_foothold)
            .await
            .expect("replay canonical foothold terminal event"),
        foothold
    );
    let foothold_event_id: Uuid = sqlx::query_scalar(
        "SELECT event_id FROM knowledge_outbox_events \
         WHERE event_name='PostExploitFactTerminal.v1' AND source_id_value=$1",
    )
    .bind(foothold_id.to_string())
    .fetch_one(db.pool())
    .await
    .expect("load foothold terminal event");

    let objective_attempt_id = Uuid::new_v4();
    let simulation_plan = json!({"kind":"deterministic_simulation","objective":"fixture"});
    let objective = objective_attempts::NewObjectiveAttempt {
        id: objective_attempt_id,
        operation_id: scope.operation_id,
        project_scope_id: scope.project_scope_id,
        scope_snapshot_id: scope.scope_snapshot_id,
        organization_id_at_time: scope.organization_id,
        attack_path_id: None,
        objective_kind: "fixture_objective".to_string(),
        simulation_plan_hash: sha256_json(&simulation_plan),
        simulation_plan,
        outcome: "simulated_achievable".to_string(),
        completed_at: Utc::now(),
        evidence: vec![(scope.proof_evidence_id, "simulation".to_string())],
    };
    let objective_row = objective_attempts::create(db.pool(), &objective)
        .await
        .expect("emit canonical objective terminal event");
    assert_eq!(
        objective_attempts::create(db.pool(), &objective)
            .await
            .expect("replay canonical objective terminal event"),
        objective_row
    );
    let objective_event_id: Uuid = sqlx::query_scalar(
        "SELECT event_id FROM knowledge_outbox_events \
         WHERE event_name='PostExploitFactTerminal.v1' AND source_id_value=$1",
    )
    .bind(objective_attempt_id.to_string())
    .fetch_one(db.pool())
    .await
    .expect("load objective terminal event");

    let principal = operator_principals::current_local(db.pool())
        .await
        .expect("load trusted local principal");
    let action_id = Uuid::new_v4();
    let obligation_id = Uuid::new_v4();
    let action_plan = json!({"kind":"remote_state_change","resource":"fixture"});
    let action_plan_hash = sha256_json(&action_plan);
    let resource_identity_hash = sha256_json(&json!({"resource":"fixture"}));
    let prepare = cleanup_obligations::RecordActionAndObligation {
        action_id,
        obligation_id,
        operation_id: scope.operation_id,
        project_scope_id: scope.project_scope_id,
        scope_snapshot_id: scope.scope_snapshot_id,
        organization_id_at_time: scope.organization_id,
        principal_id: principal.id,
        capability_id: "post_exploit.remote_state_change".to_string(),
        side_effect_class: "remote_state_mutation".to_string(),
        action_plan,
        action_plan_hash: action_plan_hash.clone(),
        action_evidence: vec![(scope.proof_evidence_id, "plan".to_string())],
        affected_resource_snapshot: json!({"resource":"fixture","state":"original"}),
        resource_identity_hash: resource_identity_hash.clone(),
        cleanup_strategy: json!({"kind":"restore_snapshot"}),
        proof_requirements: json!(["independent_resource_lookup"]),
        deadline: Utc::now() + Duration::minutes(30),
        obligation_evidence: vec![(scope.proof_evidence_id, "source".to_string())],
    };
    let prepared = cleanup_obligations::record_action_and_obligation(db.pool(), &prepare)
        .await
        .expect("atomically record action and cleanup obligation");
    let prepared_replay = cleanup_obligations::record_action_and_obligation(db.pool(), &prepare)
        .await
        .expect("replay action and cleanup obligation");
    assert_eq!(prepared_replay, prepared);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM knowledge_outbox_events \
             WHERE event_name='PostExploitActionPrepared.v1' AND source_id_value=$1",
        )
        .bind(action_id.to_string())
        .fetch_one(db.pool())
        .await
        .expect("count prepared event"),
        1
    );
    let approval = post_exploit_approvals::create_pending(
        db.pool(),
        &post_exploit_approvals::NewPostExploitApproval {
            id: Uuid::new_v4(),
            action_id,
            operation_id: scope.operation_id,
            project_scope_id: scope.project_scope_id,
            scope_snapshot_id: scope.scope_snapshot_id,
            organization_id_at_time: scope.organization_id,
            action_plan_hash,
        },
    )
    .await
    .expect("create action approval");
    let approved = post_exploit_approvals::decide(
        db.pool(),
        approval.id,
        approval.row_version,
        post_exploit_approvals::ApprovalDecision::Approve,
        principal.id,
        Some(Utc::now() + Duration::minutes(10)),
    )
    .await
    .expect("approve action");
    let executing = post_exploit_actions::begin_approved_execution(
        db.pool(),
        post_exploit_actions::BeginApprovedExecution {
            action_id,
            approval_id: approved.id,
            expected_approval_row_version: approved.row_version,
            operation_id: scope.operation_id,
            project_scope_id: scope.project_scope_id,
            scope_snapshot_id: scope.scope_snapshot_id,
            organization_id_at_time: scope.organization_id,
        },
    )
    .await
    .expect("begin approved side effect only after obligation commit");
    let action_result_evidence = insert_evidence(
        db.pool(),
        scope.operation_id,
        scope.organization_id,
        Some(scope.target_id),
        "post exploit result",
        project_path,
    )
    .await;
    post_exploit_actions::finish_execution(
        db.pool(),
        action_id,
        executing.row_version,
        post_exploit_actions::ExecutionDisposition::Succeeded,
        &[(action_result_evidence, "result".to_string())],
    )
    .await
    .expect("finish side effect");

    let cleanup_result_evidence = insert_evidence(
        db.pool(),
        scope.operation_id,
        scope.organization_id,
        Some(scope.target_id),
        "cleanup result",
        project_path,
    )
    .await;
    let absence_evidence = insert_evidence(
        db.pool(),
        scope.operation_id,
        scope.organization_id,
        Some(scope.target_id),
        "independent absence",
        project_path,
    )
    .await;
    let cleanup_stage_execution_id = Uuid::new_v4();
    let cleanup_stage_run_unit_id = Uuid::new_v4();
    let cleanup_executor_worker_id = Uuid::new_v4();
    let cleanup_verifier_worker_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status) \
         VALUES($1,$2,'cleanup','started')",
    )
    .bind(cleanup_stage_execution_id)
    .bind(scope.operation_id)
    .execute(db.pool())
    .await
    .expect("insert cleanup stage");
    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,organization_id,
               stage_kind,generation,specialist,status
           ) VALUES($1,$2,$3,$4,$5,'cleanup',0,'cleanup','running')"#,
    )
    .bind(cleanup_stage_run_unit_id)
    .bind(scope.operation_id)
    .bind(cleanup_stage_execution_id)
    .bind(scope.scope_snapshot_id)
    .bind(scope.organization_id)
    .execute(db.pool())
    .await
    .expect("insert cleanup stage unit");
    for (worker_id, work_key, agent_path) in [
        (
            cleanup_executor_worker_id,
            "cleanup-executor",
            "main>cleanup-executor",
        ),
        (
            cleanup_verifier_worker_id,
            "cleanup-verifier",
            "main>cleanup-verifier",
        ),
    ] {
        sqlx::query(
            r#"INSERT INTO stage_worker_runs(
                   id,operation_id,stage_execution_id,stage_run_unit_id,
                   organization_id,worker_generation,specialist,work_item_kind,
                   work_item_key,agent_path,status
               ) VALUES($1,$2,$3,$4,$5,0,'cleanup','cleanup',$6,$7,'queued')"#,
        )
        .bind(worker_id)
        .bind(scope.operation_id)
        .bind(cleanup_stage_execution_id)
        .bind(cleanup_stage_run_unit_id)
        .bind(scope.organization_id)
        .bind(work_key)
        .bind(agent_path)
        .execute(db.pool())
        .await
        .expect("insert independent cleanup worker");
    }
    let claimed = cleanup_attempts::claim(
        db.pool(),
        &cleanup_attempts::ClaimCleanupAttempt {
            obligation_id,
            lease_token: Uuid::new_v4(),
            lease_expires_at: Utc::now() + Duration::minutes(5),
            worker_run_id: Some(cleanup_executor_worker_id),
        },
    )
    .await
    .expect("claim cleanup");
    let executing_cleanup = cleanup_attempts::transition(
        db.pool(),
        &cleanup_attempts::TransitionCleanupAttempt {
            attempt_id: claimed.id,
            lease_token: claimed.lease_token,
            expected_row_version: claimed.row_version,
            expected_status: "claimed".to_string(),
            next_status: "executing".to_string(),
            result: None,
            evidence: Vec::new(),
            terminal_note: None,
        },
    )
    .await
    .expect("start cleanup");
    let pending_absence = cleanup_attempts::transition(
        db.pool(),
        &cleanup_attempts::TransitionCleanupAttempt {
            attempt_id: executing_cleanup.id,
            lease_token: executing_cleanup.lease_token,
            expected_row_version: executing_cleanup.row_version,
            expected_status: "executing".to_string(),
            next_status: "cleaned_pending_verification".to_string(),
            result: Some(json!({"cleanup":"submitted"})),
            evidence: vec![(cleanup_result_evidence, "result".to_string())],
            terminal_note: None,
        },
    )
    .await
    .expect("mark cleanup awaiting independent proof");
    let absence = cleanup_absence_checks::RecordAbsenceCheck {
        id: Uuid::new_v4(),
        cleanup_attempt_id: pending_absence.id,
        verifier_worker_run_id: Some(cleanup_verifier_worker_id),
        verifier_key: "independent-db-lookup".to_string(),
        resource_identity_hash,
        disposition: "absent".to_string(),
        evidence: vec![(absence_evidence, "absence".to_string())],
    };
    let terminal_cleanup = cleanup_absence_checks::record_and_apply(db.pool(), &absence)
        .await
        .expect("terminalize independent absence");
    assert_eq!(terminal_cleanup.obligation.status, "verified_absent");
    let replayed_cleanup = cleanup_absence_checks::record_and_apply(db.pool(), &absence)
        .await
        .expect("replay independent absence");
    assert_eq!(replayed_cleanup, terminal_cleanup);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM knowledge_outbox_events \
             WHERE event_name='CleanupObligationTerminal.v1' AND source_id_value=$1",
        )
        .bind(obligation_id.to_string())
        .fetch_one(db.pool())
        .await
        .expect("count cleanup event"),
        1
    );
    let cleanup_event_id: Uuid = sqlx::query_scalar(
        "SELECT event_id FROM knowledge_outbox_events \
         WHERE event_name='CleanupObligationTerminal.v1' AND source_id_value=$1",
    )
    .bind(obligation_id.to_string())
    .fetch_one(db.pool())
    .await
    .expect("load cleanup terminal event");
    install_first_assertion_ack_loss(db.pool(), cleanup_event_id).await;

    let cleanup_memory_runtime =
        KnowledgeMemoryRuntime::new(Arc::new(db.pool().clone()), Some(embedding));
    cleanup_memory_runtime
        .start()
        .await
        .expect("start production projector supervisor");
    wait_for_real_assertion_write_then_replay(db.pool(), cleanup_event_id).await;
    wait_for_exact_projection_dag(
        db.pool(),
        &[foothold_event_id, objective_event_id, cleanup_event_id],
    )
    .await;
    assert_one_projection_version(db.pool(), foothold_event_id, 1).await;
    assert_one_projection_version(db.pool(), objective_event_id, 1).await;
    assert_one_projection_version(db.pool(), cleanup_event_id, 2).await;

    sqlx::query("UPDATE candidate_attempts SET status=status WHERE id=$1")
        .bind(scope.attempt_id)
        .execute(db.pool())
        .await
        .expect_err("terminal CandidateAttempt must reject even a no-op update");
    sqlx::query("UPDATE cleanup_obligations SET status=status WHERE id=$1")
        .bind(obligation_id)
        .execute(db.pool())
        .await
        .expect_err("terminal CleanupObligation must reject even a no-op update");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM knowledge_outbox_events WHERE event_id=$1 OR event_id=$2",
        )
        .bind(event_id)
        .bind(cleanup_event_id)
        .fetch_one(db.pool())
        .await
        .expect("terminal update rejection cannot append a replacement event"),
        2
    );

    let pool = Arc::new(db.pool().clone());
    let built = ReportReadModelBuilder::new(PgReportTruthPort::new(pool.clone()))
        .build_and_validate(scope.operation_id)
        .await
        .expect("build validated canonical report");
    for expected in ["finding", "candidate_attempt", "post_exploit_action"] {
        assert!(built
            .model
            .source_snapshot
            .ordered_sources
            .iter()
            .any(|source| source.kind.as_str() == expected));
    }
    assert!(built
        .model
        .citations
        .iter()
        .all(|citation| citation.evidence_audit_id.is_some()));
    let report_store = TestProjectReportArtifactStore::new(canonical_project_root.clone());
    let rendered_markdown = format!(
        "# Evidence-backed closeout\n\nRevision: `{}`\n",
        built.model.revision_id
    )
    .into_bytes();
    let rendered_json = serde_json::to_vec_pretty(&json!({
        "revision_id": built.model.revision_id,
        "operation_id": built.model.operation_id,
        "source_set_hash": built.model.source_snapshot.source_set_hash,
    }))
    .expect("render deterministic closeout JSON");
    let artifacts = ReportFinalizer::new(
        report_store.clone(),
        PgReportPublicationPort::new(pool.clone()),
    )
    .finalize(
        &built.model,
        ExplicitFinalizeRequest {
            principal_id: principal.id,
            confirm_final_publish: true,
            expected_row_version: built.expected_row_version,
            validation_status: ValidationStatus::Validated,
            publication_status: PublicationStatus::Unpublished,
        },
        vec![
            (ReportFormat::Markdown, rendered_markdown),
            (ReportFormat::Json, rendered_json),
        ],
    )
    .await
    .expect("stage, promote, verify and finalize cited canonical report");
    assert_eq!(artifacts.len(), 2);
    for artifact in &artifacts {
        assert!(
            report_store
                .verify(artifact)
                .await
                .expect("verify final content-addressed artifact"),
            "finalizer must retain a verified blob for {}",
            artifact.content_key
        );
        let blob_path = canonical_project_root
            .join(".golish/reports/blobs")
            .join(&artifact.content_key);
        let metadata = std::fs::metadata(&blob_path).expect("final artifact exists on disk");
        assert!(metadata.is_file());
        assert_eq!(metadata.len(), artifact.byte_len);
    }
    let bundle = load_report_bundle(&pool, scope.operation_id)
        .await
        .expect("load final bundle")
        .expect("report exists");
    assert_eq!(bundle.current_revision.unwrap().publication_status, "final");
    assert_eq!(bundle.artifacts.len(), 2);
    assert_eq!(
        bundle
            .artifacts
            .iter()
            .map(|artifact| artifact.content_key.as_str())
            .collect::<BTreeSet<_>>(),
        artifacts
            .iter()
            .map(|artifact| artifact.content_key.as_str())
            .collect::<BTreeSet<_>>()
    );
    assert_finalized_report_history_is_immutable(db.pool(), built.model.revision_id).await;

    cleanup_memory_runtime
        .shutdown()
        .await
        .expect("stop production projector supervisor");
    db.stop().await;
}

struct WrongDimensionEmbedder;

#[async_trait]
impl Embedder for WrongDimensionEmbedder {
    async fn embed(&self, _text: &str) -> anyhow::Result<Vec<f32>> {
        Ok(vec![0.0; 1024])
    }

    fn dimension(&self) -> usize {
        1024
    }

    fn model_name(&self) -> &str {
        "wrong-dimension-fixture"
    }
}

#[test]
fn startup_rejects_a_1024_dimension_embedding_provider() {
    let error = match KnowledgeEmbeddingProvider::new("fixture", Arc::new(WrongDimensionEmbedder)) {
        Ok(_) => panic!("1024 dimensions must be rejected before runtime startup"),
        Err(error) => error,
    };
    assert_eq!(error.code(), "memory_policy_rejected");
    assert!(error
        .to_string()
        .contains("memory_embedding_provider_dimension_mismatch"));
}

#[tokio::test]
async fn model_payloads_cannot_forge_approval_waiver_action_or_finalize_actors() {
    assert!(
        serde_json::from_value::<AttackCandidateReviewRequest>(json!({
            "operationId": Uuid::new_v4().to_string(),
            "waveRunId": Uuid::new_v4().to_string(),
            "decisions": [],
            "principalId": Uuid::new_v4().to_string()
        }))
        .is_err()
    );
    assert!(serde_json::from_value::<CleanupWaiverSubmitRequest>(json!({
        "waiverId": Uuid::new_v4().to_string(),
        "obligationId": Uuid::new_v4().to_string(),
        "operationId": Uuid::new_v4().to_string(),
        "projectScopeId": Uuid::new_v4().to_string(),
        "scopeSnapshotId": Uuid::new_v4().to_string(),
        "organizationIdAtTime": Uuid::new_v4().to_string(),
        "expectedRowVersion": 0,
        "reason": "fixture",
        "residualSummary": "fixture",
        "residualSeverity": "low",
        "evidenceIds": [1],
        "actorId": Uuid::new_v4().to_string()
    }))
    .is_err());
    assert!(serde_json::from_value::<ReportingFinalizeRequest>(json!({
        "operationId": Uuid::new_v4().to_string(),
        "revisionId": Uuid::new_v4().to_string(),
        "expectedSourceHash": "a".repeat(64),
        "expectedRevisionVersion": 0,
        "confirmFinalPublish": true,
        "principalId": Uuid::new_v4().to_string()
    }))
    .is_err());

    let lazy_pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgresql://localhost/golish")
        .expect("lazy postgres pool");
    let action_tool = PostExploitExecuteActionTool::new(Arc::new(lazy_pool));
    let result = action_tool
        .execute(
            json!({
                "mode":"execute",
                "action_id":Uuid::new_v4(),
                "approval_id":Uuid::new_v4(),
                "expected_approval_row_version":0,
                "actor_id":Uuid::new_v4()
            }),
            Path::new(FORGED_TOOL_PROJECT_PATH),
        )
        .await
        .expect("closed action payload returns typed error");
    assert_eq!(result["code"], "post_exploit_args_invalid");
}
