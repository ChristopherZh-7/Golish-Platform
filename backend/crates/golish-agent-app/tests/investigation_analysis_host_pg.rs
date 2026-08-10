use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use golish_agent_app::ai::db_bridge::{
    GolishDbRepoProvider, PgInvestigationAnalysisHostRepository,
};
use golish_agent_kit::db_traits::*;
use golish_db::{DbConfig, GolishDb};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read local postgres port")
        .port()
}

fn digest(nibble: char) -> String {
    format!("sha256:{}", nibble.to_string().repeat(64))
}

async fn migrated_db(label: &str) -> (GolishDb, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("analysis_host_{label}_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start isolated migrated postgres");
    (db, data_dir)
}

#[derive(Clone)]
struct FrozenSnapshotRegistry {
    expected_request: FreezeCandidateAnalysisSnapshot,
    snapshot: CandidateAnalysisSnapshotView,
    calls: Arc<Mutex<Vec<FreezeCandidateAnalysisSnapshot>>>,
}

#[async_trait]
impl HypothesisRegistryRepository for FrozenSnapshotRegistry {
    async fn freeze_candidate_snapshot(
        &self,
        request: FreezeCandidateAnalysisSnapshot,
    ) -> Result<CandidateAnalysisSnapshotView, HypothesisRegistryError> {
        if request != self.expected_request {
            return Err(HypothesisRegistryError::AuthorityMismatch(
                "unexpected freeze identity".to_owned(),
            ));
        }
        self.calls.lock().expect("freeze call lock").push(request);
        Ok(self.snapshot.clone())
    }

    async fn load_snapshot_page(
        &self,
        _request: LoadCandidateAnalysisPage,
    ) -> Result<CandidateAnalysisPageView, HypothesisRegistryError> {
        unreachable!("analysis host fixture only freezes snapshots")
    }

    async fn load_snapshot_chunk_page(
        &self,
        _request: LoadCandidateInputChunkPage,
    ) -> Result<CandidateInputChunkPageView, HypothesisRegistryError> {
        unreachable!("analysis host fixture only freezes snapshots")
    }

    async fn record_analysis_artifact(
        &self,
        _request: RecordCandidateAnalysisArtifact,
    ) -> Result<CandidateAnalysisArtifactReceipt, HypothesisRegistryError> {
        unreachable!("analysis host fixture only freezes snapshots")
    }

    async fn seal_analysis_census(
        &self,
        _request: SealCandidateAnalysisCensus,
    ) -> Result<CandidateAnalysisCensusView, HypothesisRegistryError> {
        unreachable!("analysis host fixture only freezes snapshots")
    }

    async fn seal_hypothesis_coverage_subreview_census(
        &self,
        _request: SealHypothesisCoverageSubreviewCensus,
    ) -> Result<HypothesisCoverageSubreviewCensusView, HypothesisRegistryError> {
        unreachable!("analysis host fixture only freezes snapshots")
    }

    async fn record_hypothesis_coverage_subreview(
        &self,
        _request: RecordHypothesisCoverageSubreview,
    ) -> Result<HypothesisCoverageSubreviewReceipt, HypothesisRegistryError> {
        unreachable!("analysis host fixture only freezes snapshots")
    }

    async fn seal_hypothesis_coverage_synthesis_census(
        &self,
        _request: SealHypothesisCoverageSynthesisCensus,
    ) -> Result<HypothesisCoverageSynthesisCensusView, HypothesisRegistryError> {
        unreachable!("analysis host fixture only freezes snapshots")
    }

    async fn record_hypothesis_coverage_synthesis_review(
        &self,
        _request: RecordHypothesisCoverageSynthesisReview,
    ) -> Result<HypothesisCoverageSynthesisReceipt, HypothesisRegistryError> {
        unreachable!("analysis host fixture only freezes snapshots")
    }

    async fn reduce_hypothesis_coverage_review(
        &self,
        _request: ReduceHypothesisCoverageReview,
    ) -> Result<HypothesisCoverageReviewReceipt, HypothesisRegistryError> {
        unreachable!("analysis host fixture only freezes snapshots")
    }

    async fn seal_candidate_compilation(
        &self,
        _request: SealCandidateCompilation,
    ) -> Result<CandidateCompilationSealView, HypothesisRegistryError> {
        unreachable!("analysis host fixture only freezes snapshots")
    }

    async fn load_candidate_gate_material(
        &self,
        _request: LoadCandidateGateMaterial,
    ) -> Result<CandidateGateMaterial, HypothesisRegistryError> {
        unreachable!("analysis host fixture only freezes snapshots")
    }

    async fn apply_candidate_gate_pass(
        &self,
        _request: ApplyCandidateGatePass,
    ) -> Result<CandidateGenerationSealView, HypothesisRegistryError> {
        unreachable!("analysis host fixture only freezes snapshots")
    }
}

#[derive(Clone)]
struct Fixture {
    identity: UnifiedInvestigationUnitIdentity,
    work_id: Uuid,
    stable_request_id: Uuid,
    snapshot_id: Uuid,
    snapshot_hash: String,
    analysis_attempt_id: Uuid,
    attempt_input_hash: String,
    authority_input_id: Uuid,
    authority_chunk_id: Uuid,
    authority_source_hash: String,
    authority_body: String,
}

struct VerificationSubjectFixture {
    task_id: Uuid,
}

async fn seed_fixture(pool: &PgPool) -> Fixture {
    let operation_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    let scope_snapshot_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let stage_run_unit_id = Uuid::new_v4();
    let authority_id = Uuid::new_v4();
    let work_id = Uuid::new_v4();
    let stable_request_id = Uuid::new_v4();
    let snapshot_id = Uuid::new_v4();
    let analysis_attempt_id = Uuid::new_v4();
    let owning_request_id = "analysis-host-pg-request".to_owned();
    let snapshot_hash = digest('8');
    let attempt_input_hash = digest('9');
    let authority_input_id = Uuid::new_v4();
    let authority_chunk_id = Uuid::new_v4();
    let authority_chunk_census_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    let authority_source_hash = digest('4');
    let authority_body =
        r#"{"evidence_id":17,"schema":"investigation_predecessor_evidence.v1"}"#.to_owned();
    let authority_body_hex = authority_body
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let authority_byte_count = i64::try_from(authority_body.len()).expect("bounded fixture body");

    let mut tx = pool.begin().await.expect("begin compact host fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("isolate compact authority fixture");
    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,project_scope_id,
               tool_truth_contract,investigation_contract_version,investigation_rollout_mode,
               stage_topology_contract,stage_topology_canonical_json,
               stage_topology_sha256,stage_topology_freeze_source,
               enumeration_analysis_contract
           ) VALUES($1,'red_team','investigation','v2_only',$2,
                    'receipt_v1','hypothesis_registry_v1','new_only',
                    'unified_investigation_v1',
                    stage_topology_canonical_json('unified_investigation_v1'),
                    stage_topology_contract_sha256('unified_investigation_v1'),
                    'deployment_pair_v1','legacy_v1')"#,
    )
    .bind(operation_id)
    .bind(project_scope_id)
    .execute(&mut *tx)
    .await
    .expect("seed operation-frozen unified Investigation contract");
    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,
               organization_id,stage_kind,generation,status,started_at
           ) VALUES($1,$2,$3,$4,$5,'investigation',0,'running',NOW())"#,
    )
    .bind(stage_run_unit_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .execute(&mut *tx)
    .await
    .expect("seed Investigation unit");
    sqlx::query(
        r#"INSERT INTO investigation_stage_run_authorities(
               authority_id,operation_id,stage_execution_id,
               owning_stage_run_request_id,scope_snapshot_id
           ) VALUES($1,$2,$3,$4,$5)"#,
    )
    .bind(authority_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(&owning_request_id)
    .bind(scope_snapshot_id)
    .execute(&mut *tx)
    .await
    .expect("seed Investigation authority");
    sqlx::query(
        r#"INSERT INTO investigation_run_heads(
               authority_id,stable_start_request_id,operation_id,stage_execution_id,
               owning_stage_run_request_id,scope_snapshot_id,run_state,admission_open,
               stop_epoch,change_seq,head_version,head_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,'running',TRUE,0,0,0,
                    unified_investigation_runtime_head_sha256($1,'running',TRUE,0,0,0))"#,
    )
    .bind(authority_id)
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(&owning_request_id)
    .bind(scope_snapshot_id)
    .execute(&mut *tx)
    .await
    .expect("seed Investigation run head");
    sqlx::query(
        r#"INSERT INTO investigation_run_work_items(
               work_id,stable_work_key_sha256,authority_id,operation_id,
               stage_execution_id,owning_stage_run_request_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,work_kind,external_identity_sha256,
               current_state,observed_stop_epoch
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'analysis',$10,'running',0)"#,
    )
    .bind(work_id)
    .bind(digest('1'))
    .bind(authority_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(&owning_request_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(digest('2'))
    .execute(&mut *tx)
    .await
    .expect("seed registered analysis work");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_snapshots(
               snapshot_id,operation_id,organization_id,wave_ordinal,scope_snapshot_id,
               genesis,source_set_hash,capability_revision_hash,policy_revision_hash,
               credential_revision_hash,snapshot_status,tool_truth_authority_bundle_seal_id,
               stable_consumer_request_id,relevant_root_count,relevant_root_set_hash,
               bundle_member_count,bundle_member_set_hash,semantic_authority_bundle_hash,
               freshness_attestation_bundle_hash,temporal_validity_bundle_hash,
               temporal_validity_policy_set_hash,target_state_epoch_set_hash,
               observation_window_hash,bundle_sealed_at,candidate_snapshot_authority_hash
           ) VALUES($1,$2,$3,0,$4,TRUE,$5,$6,$7,$8,'sealed_ready',$9,$10,4,$11,4,$12,
                    $13,$14,$15,$16,$17,$18,NOW(),$19)"#,
    )
    .bind(snapshot_id)
    .bind(operation_id)
    .bind(organization_id)
    .bind(scope_snapshot_id)
    .bind(digest('3'))
    .bind(digest('4'))
    .bind(digest('5'))
    .bind(digest('6'))
    .bind(Uuid::new_v4())
    .bind(stable_request_id)
    .bind(digest('a'))
    .bind(digest('b'))
    .bind(digest('c'))
    .bind(digest('d'))
    .bind(digest('e'))
    .bind(digest('f'))
    .bind(digest('0'))
    .bind(digest('7'))
    .bind(digest('8'))
    .execute(&mut *tx)
    .await
    .expect("seed Candidate snapshot");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_attempts(
               analysis_attempt_id,snapshot_id,operation_id,organization_id,attempt_ordinal,
               attempt_input_hash,attack_class_checklist_version,
               attack_class_checklist_digest,trust_boundary_checklist_version,
               trust_boundary_checklist_digest,coverage_sampling_contract_version,
               coverage_sampling_contract_digest,retry_limit
           ) VALUES($1,$2,$3,$4,0,$5,'1',$6,'1',$7,'1',$8,1)"#,
    )
    .bind(analysis_attempt_id)
    .bind(snapshot_id)
    .bind(operation_id)
    .bind(organization_id)
    .bind(&attempt_input_hash)
    .bind(digest('a'))
    .bind(digest('b'))
    .bind(digest('c'))
    .execute(&mut *tx)
    .await
    .expect("seed ordinal-zero attempt");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_snapshot_inputs(
               snapshot_input_id,snapshot_id,stable_input_key,source_kind,source_ref,
               source_ref_hash,source_content_hash,source_byte_count,subject_kind_at_time,
               subject_identity_hash,server_chunking_disposition,instruction_authority,input_hash)
           VALUES($1,$2,'source-set:predecessor_evidence:source:17',
                  'predecessor_evidence','candidate_snapshot_source_member:fixture',
                  $3,$4,$5,'organization',$6,'complete',FALSE,$7)"#,
    )
    .bind(authority_input_id)
    .bind(snapshot_id)
    .bind(digest('1'))
    .bind(&authority_source_hash)
    .bind(authority_byte_count)
    .bind(digest('2'))
    .bind(digest('3'))
    .execute(&mut *tx)
    .await
    .expect("seed frozen predecessor evidence input");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_input_chunk_censuses(
               chunk_census_id,snapshot_input_id,snapshot_id,chunking_contract_version,
               redaction_contract_version,source_content_hash,source_byte_count,disposition,
               chunk_count,chunk_member_set_hash,census_hash)
           VALUES($1,$2,$3,'candidate-fixed-bytes-v1','candidate-redaction-v1',$4,$5,
                  'complete',1,$6,$7)"#,
    )
    .bind(authority_chunk_census_id)
    .bind(authority_input_id)
    .bind(snapshot_id)
    .bind(&authority_source_hash)
    .bind(authority_byte_count)
    .bind(digest('5'))
    .bind(digest('6'))
    .execute(&mut *tx)
    .await
    .expect("seed predecessor evidence chunk census");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_input_chunk_census_members(
               chunk_id,chunk_census_id,snapshot_input_id,snapshot_id,ordinal,
               source_range_start,source_range_end,envelope_schema,immutable_redacted_body,
               body_or_blob_hash,chunking_contract_version,redaction_contract_version,chunk_hash)
           VALUES($1,$2,$3,$4,0,0,$5,'candidate-input-chunk.v1',$6,$7,
                  'candidate-fixed-bytes-v1','candidate-redaction-v1',$8)"#,
    )
    .bind(authority_chunk_id)
    .bind(authority_chunk_census_id)
    .bind(authority_input_id)
    .bind(snapshot_id)
    .bind(authority_byte_count)
    .bind(serde_json::json!({"canonical_source_fragment": authority_body_hex}))
    .bind(digest('7'))
    .bind(digest('8'))
    .execute(&mut *tx)
    .await
    .expect("seed exact predecessor evidence chunk");
    tx.commit().await.expect("commit compact host fixture");

    Fixture {
        identity: UnifiedInvestigationUnitIdentity {
            stage: UnifiedInvestigationStageIdentity {
                authority_id,
                operation_id,
                stage_execution_id,
                owning_stage_run_request_id: owning_request_id,
                scope_snapshot_id,
            },
            stage_run_unit_id,
            organization_id,
        },
        work_id,
        stable_request_id,
        snapshot_id,
        snapshot_hash,
        analysis_attempt_id,
        attempt_input_hash,
        authority_input_id,
        authority_chunk_id,
        authority_source_hash,
        authority_body,
    }
}

async fn seed_verification_subject_fixture(
    pool: &PgPool,
    fixture: &Fixture,
) -> VerificationSubjectFixture {
    let task_id = Uuid::new_v4();
    let assignment_set_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let revision_sha256 = digest('2');
    let plan_id = Uuid::new_v4();
    let plan_sha256 = digest('3');
    let generation_id = Uuid::new_v4();
    let relevant_evidence_snapshot_id = Uuid::new_v4();
    let campaigns = (0..2)
        .map(|ordinal| InvestigationVerificationCampaignSubjectV1 {
            campaign_id: Uuid::new_v4(),
            plan_objective_id: Uuid::new_v4(),
            objective_id: Uuid::new_v4(),
            reservation_sha256: digest(if ordinal == 0 { '4' } else { '5' }),
            capability_assessment_set_sha256: digest(if ordinal == 0 { '6' } else { '7' }),
            available_capability_ids: Vec::new(),
        })
        .collect::<Vec<_>>();

    let mut tx = pool.begin().await.expect("begin VerificationTask fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("isolate VerificationTask authority fixture");
    sqlx::query(
        r#"INSERT INTO hypothesis_verification_tasks(
               task_id,stable_task_key_sha256,operation_id,project_scope_id,
               stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id,
               hypothesis_revision_id,hypothesis_revision_sha256,
               verification_plan_id,verification_plan_sha256,
               relevant_evidence_snapshot_id,semantic_evidence_set_sha256,
               open_obligation_set_sha256,semantic_attempt_fingerprint,
               task_contract_version,first_admission_generation_id
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,
                    'hypothesis_verification_task.v1',$17)"#,
    )
    .bind(task_id)
    .bind(digest('1'))
    .bind(fixture.identity.stage.operation_id)
    .bind(Uuid::new_v4())
    .bind(fixture.identity.stage.stage_execution_id)
    .bind(fixture.identity.stage_run_unit_id)
    .bind(fixture.identity.stage.scope_snapshot_id)
    .bind(fixture.identity.organization_id)
    .bind(revision_id)
    .bind(&revision_sha256)
    .bind(plan_id)
    .bind(&plan_sha256)
    .bind(relevant_evidence_snapshot_id)
    .bind(digest('6'))
    .bind(digest('7'))
    .bind(digest('8'))
    .bind(generation_id)
    .execute(&mut *tx)
    .await
    .expect("seed VerificationTask");
    sqlx::query(
        r#"INSERT INTO hypothesis_verification_task_assignment_sets(
               assignment_set_id,stable_request_id,task_id,hypothesis_revision_id,
               verification_plan_id,status,member_count,member_set_sha256,row_version,sealed_at
           ) VALUES($1,$2,$3,$4,$5,'sealed',$6,$7,1,NOW())"#,
    )
    .bind(assignment_set_id)
    .bind(Uuid::new_v4())
    .bind(task_id)
    .bind(revision_id)
    .bind(plan_id)
    .bind(i64::try_from(campaigns.len()).expect("bounded fixture campaign count"))
    .bind(digest('9'))
    .execute(&mut *tx)
    .await
    .expect("seed sealed VerificationTask assignment");
    for campaign in &campaigns {
        sqlx::query(
            r#"INSERT INTO hypothesis_verification_task_campaigns(
                   campaign_id,assignment_set_id,task_id,hypothesis_revision_id,
                   verification_plan_id,plan_objective_id,verification_objective_id,
                   reservation_sha256)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
        )
        .bind(campaign.campaign_id)
        .bind(assignment_set_id)
        .bind(task_id)
        .bind(revision_id)
        .bind(plan_id)
        .bind(campaign.plan_objective_id)
        .bind(campaign.objective_id)
        .bind(&campaign.reservation_sha256)
        .execute(&mut *tx)
        .await
        .expect("seed Campaign/objective reservation");
    }
    tx.commit()
        .await
        .expect("commit VerificationTask subject fixture");

    VerificationSubjectFixture { task_id }
}

fn snapshot_view(fixture: &Fixture) -> CandidateAnalysisSnapshotView {
    CandidateAnalysisSnapshotView {
        snapshot_id: fixture.snapshot_id,
        stable_consumer_request_id: fixture.stable_request_id,
        operation_id: fixture.identity.stage.operation_id,
        scope_snapshot_id: fixture.identity.stage.scope_snapshot_id,
        organization_id: fixture.identity.organization_id,
        disposition: CandidateAnalysisSnapshotDispositionV1::SealedReady,
        snapshot_hash: fixture.snapshot_hash.clone(),
        candidate_snapshot_authority_hash: digest('1'),
        tool_truth_authority_bundle_seal_id: Uuid::new_v4(),
        tool_truth_authority_root_count: 4,
        tool_truth_authority_root_set_hash: digest('2'),
        tool_truth_authority_bundle_member_count: 4,
        tool_truth_authority_bundle_member_set_hash: digest('3'),
        tool_truth_authority_receipt_count: 0,
        tool_truth_authority_receipt_set_hash: digest('4'),
        denominator_graph_bundle_hash: digest('5'),
        semantic_authority_bundle_hash: digest('6'),
        freshness_attestation_bundle_hash: digest('7'),
        temporal_validity_bundle_hash: digest('8'),
        temporal_validity_policy_set_hash: digest('9'),
        temporal_validity_decision_set_hash: digest('a'),
        observation_window_hash: digest('b'),
        target_state_epoch_set_hash: digest('c'),
        authority_roots: Vec::new(),
        knowledge_feed_catalog_policy_seal_hash: digest('d'),
        knowledge_feed_required_member_set_hash: digest('e'),
        knowledge_feed_signature_algorithm_set_hash: digest('f'),
        knowledge_feed_trust_store_hash: digest('0'),
        knowledge_feed_key_revocation_epoch_hash: digest('1'),
        knowledge_feed_snapshot_set_hash: digest('2'),
        product_version_census_hash: digest('3'),
        knowledge_feed_match_census_hash: digest('4'),
        stale_revalidation_obligation_set_hash: digest('5'),
        knowledge_feed_obligation_set_hash: digest('6'),
        row_version: 0,
        sealed_at: Utc::now(),
    }
}

#[tokio::test]
#[serial]
async fn pg_host_freezes_binds_and_replays_exact_analysis_subject() {
    let (db, _data_dir) = migrated_db("prepare-replay").await;
    let fixture = seed_fixture(db.pool()).await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let registry = Arc::new(FrozenSnapshotRegistry {
        expected_request: FreezeCandidateAnalysisSnapshot {
            stable_consumer_request_id: fixture.stable_request_id,
            operation_id: fixture.identity.stage.operation_id,
            scope_snapshot_id: fixture.identity.stage.scope_snapshot_id,
            organization_id: fixture.identity.organization_id,
        },
        snapshot: snapshot_view(&fixture),
        calls: calls.clone(),
    });
    let host =
        PgInvestigationAnalysisHostRepository::with_registry(Arc::new(db.pool().clone()), registry);
    let request = PrepareInvestigationAnalysisSubject {
        stable_request_id: fixture.stable_request_id,
        identity: fixture.identity.clone(),
        work_id: fixture.work_id,
    };

    let first = host
        .prepare_analysis_subject(request.clone())
        .await
        .expect("prepare exact analysis subject");
    assert!(!first.replayed);
    assert_eq!(first.analysis_attempt_id, fixture.analysis_attempt_id);
    assert_eq!(first.candidate_snapshot_id, fixture.snapshot_id);
    assert_eq!(first.candidate_snapshot_sha256, fixture.snapshot_hash);
    assert_eq!(first.subject_fingerprint_sha256, fixture.attempt_input_hash);
    assert_eq!(first.authority_inputs.len(), 1);
    assert_eq!(
        first.authority_inputs[0].input_id,
        fixture.authority_input_id
    );
    assert_eq!(
        first.authority_inputs[0].source_kind,
        "predecessor_evidence"
    );
    assert_eq!(
        first.authority_inputs[0].source_sha256,
        fixture.authority_source_hash
    );
    assert_eq!(first.authority_inputs[0].body, fixture.authority_body);
    assert_eq!(first.authority_inputs[0].chunks.len(), 1);
    assert_eq!(
        first.authority_inputs[0].chunks[0].chunk_id,
        fixture.authority_chunk_id
    );

    let replay = host
        .prepare_analysis_subject(request)
        .await
        .expect("replay exact analysis subject");
    assert!(replay.replayed);
    assert_eq!(replay.analysis_attempt_id, first.analysis_attempt_id);
    assert_eq!(replay.binding_id, first.binding_id);
    assert_eq!(replay.authority_inputs, first.authority_inputs);
    assert_eq!(calls.lock().expect("freeze calls").len(), 2);
    let binding_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM investigation_analysis_attempt_bindings")
            .fetch_one(db.pool())
            .await
            .expect("count exact binding");
    assert_eq!(binding_count, 1);

    let provider = GolishDbRepoProvider::new(Arc::new(db.pool().clone()));
    assert!(provider.investigation_analysis_host_repository().is_ok());
}

#[tokio::test]
#[serial]
async fn pg_host_rejects_reservation_only_verification_campaign_authority() {
    let (db, _data_dir) = migrated_db("verification-campaign-mapping").await;
    let fixture = seed_fixture(db.pool()).await;
    let expected = seed_verification_subject_fixture(db.pool(), &fixture).await;
    let host = PgInvestigationAnalysisHostRepository::new(Arc::new(db.pool().clone()));

    let error = host
        .prepare_verification_task_subject(PrepareInvestigationVerificationTaskSubject {
            stable_request_id: Uuid::new_v4(),
            identity: fixture.identity.clone(),
            verification_task_id: expected.task_id,
        })
        .await
        .expect_err("reservation-only fixture must not become executable Campaign authority");
    assert!(matches!(
        error,
        InvestigationAnalysisHostError::SnapshotBlocked { .. }
    ));
    assert!(
        error
            .to_string()
            .contains("Campaign reservation generation is incomplete"),
        "unexpected fail-closed reason: {error}"
    );
    let campaign_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM verification_campaigns WHERE operation_id=$1")
            .bind(fixture.identity.stage.operation_id)
            .fetch_one(db.pool())
            .await
            .expect("count materialized Campaigns after rejected authority");
    assert_eq!(campaign_count, 0);
}
