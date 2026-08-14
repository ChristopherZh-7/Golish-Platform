use golish_core::hypothesis_semantic_key::{CandidateMutationEpistemicState, ClaimPolarity};
use golish_core::hypothesis_verification::{
    compile_claim_components_v1, HypothesisClaimComponentInputV1, HypothesisClaimComponentKindV1,
    HypothesisVerificationObjectiveOutcomeRequirementV1, HypothesisVerificationPlanBuildInputV1,
    HypothesisVerificationPlanObjectiveInputV1, HypothesisVerificationPlanPathInputV1,
    HypothesisVerificationPlanPathMemberInputV1, HypothesisVerificationPlanPathMemberRoleV1,
    HypothesisVerificationPlanV1,
};
use golish_core::verification_contract::{
    CanonicalJsonObject, ContractCombinatorV1, PredicateComponentInputV1,
    VerificationContractBuildInputV1, VerificationContractV1,
};
use golish_db::{
    repo::{
        hypothesis_registry::{
            CandidateMutationRouteRow, CandidateMutationRow, CandidateRevisionSourceRefRow,
        },
        investigation_hypothesis_compiler::{
            apply_investigation_compilation, prepare_investigation_compilation,
            reseal_investigation_mutation, ApplyInvestigationCompilationInput,
            InvestigationProofRefInput, InvestigationProposalInput,
            PrepareInvestigationCompilationInput,
        },
    },
    DbConfig, GolishDb,
};
use serde_json::{json, Value};
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
        database: format!("investigation_compiler_{label}_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start isolated migrated postgres");
    (db, data_dir)
}

#[derive(Debug, Clone)]
struct Fixture {
    input: PrepareInvestigationCompilationInput,
    proof: InvestigationProofRefInput,
}

async fn seed_fixture(pool: &PgPool) -> Fixture {
    let project_scope_id = Uuid::new_v4();
    let operation_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    let stage_run_unit_id = Uuid::new_v4();
    let scope_snapshot_id = Uuid::new_v4();
    let authority_id = Uuid::new_v4();
    let work_id = Uuid::new_v4();
    let snapshot_id = Uuid::new_v4();
    let asset_lane_id = Uuid::new_v4();
    let target_id = Uuid::new_v4();
    let attempt_id = Uuid::new_v4();
    let binding_id = Uuid::new_v4();
    let task_plan_id = Uuid::new_v4();
    let primary_worker_run_id = Uuid::new_v4();
    let census_id = Uuid::new_v4();
    let dispatch_id = Uuid::new_v4();
    let input_id = Uuid::new_v4();
    let chunk_census_id = Uuid::new_v4();
    let chunk_id = Uuid::new_v4();
    let owning_request = "investigation-compiler-fixture";
    let project_path = format!("/tmp/investigation-compiler-{}", Uuid::new_v4().simple());
    let source_hash = digest('a');

    let mut tx = pool.begin().await.expect("begin compact authority seed");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("disable seed-only triggers");
    sqlx::query(
        "INSERT INTO project_scopes(project_scope_id,canonical_project_path,path_sha256) VALUES($1,$2,$3)",
    )
    .bind(project_scope_id)
    .bind(&project_path)
    .bind(digest('1'))
    .execute(&mut *tx)
    .await
    .expect("seed project scope");
    sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$2,'Compiler Org')")
        .bind(organization_id)
        .bind(&project_path)
        .execute(&mut *tx)
        .await
        .expect("seed organization");
    sqlx::query(
        r#"INSERT INTO targets(
               id,name,target_type,value,project_path,organization_id,scope,source)
           VALUES($1,'compiler.example','domain','compiler.example',$2,$3,'in','fixture')"#,
    )
    .bind(target_id)
    .bind(&project_path)
    .bind(organization_id)
    .execute(&mut *tx)
    .await
    .expect("seed compiler live target");
    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,project_scope_id,
               enumeration_analysis_contract,stage_topology_contract,
               stage_topology_canonical_json,stage_topology_sha256,
               stage_topology_freeze_source,tool_truth_contract,
               investigation_contract_version,investigation_rollout_mode)
           VALUES($1,'red_team','investigation','v2_only',$2,'legacy_v1',
                  'unified_investigation_v1',
                  stage_topology_canonical_json('unified_investigation_v1'),
                  stage_topology_contract_sha256('unified_investigation_v1'),
                  'deployment_pair_v1','receipt_v1','hypothesis_registry_v1','new_only')"#,
    )
    .bind(operation_id)
    .bind(project_scope_id)
    .execute(&mut *tx)
    .await
    .expect("seed operation");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units(
               snapshot_id,organization_id,parent_organization_id,
               organization_name_at_freeze,role,depth,ordinal,decision_row_id,approval_source)
           VALUES($1,$2,NULL,'Compiler Org','root',0,0,'root','{}')"#,
    )
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .execute(&mut *tx)
    .await
    .expect("seed scope membership");
    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,organization_id,
               stage_kind,generation,status,started_at)
           VALUES($1,$2,$3,$4,$5,'investigation',0,'running',NOW())"#,
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
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,organization_id,
               worker_generation,specialist,work_item_kind,work_item_key,agent_path,status)
           VALUES($1,$2,$3,$4,$5,0,'primary','investigation_analysis','primary',
                  'main/primary','passed')"#,
    )
    .bind(primary_worker_run_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(organization_id)
    .execute(&mut *tx)
    .await
    .expect("seed Primary worker");
    sqlx::query(
        r#"INSERT INTO investigation_run_heads(
               authority_id,stable_start_request_id,operation_id,stage_execution_id,
               owning_stage_run_request_id,scope_snapshot_id,run_state,admission_open,
               stop_epoch,change_seq,head_version,head_sha256)
           VALUES($1,$2,$3,$4,$5,$6,'running',TRUE,0,0,0,
                  unified_investigation_runtime_head_sha256($1,'running',TRUE,0,0,0))"#,
    )
    .bind(authority_id)
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(owning_request)
    .bind(scope_snapshot_id)
    .execute(&mut *tx)
    .await
    .expect("seed run head");
    sqlx::query(
        r#"INSERT INTO investigation_asset_lanes(
               asset_lane_id,asset_queue_id,company_queue_id,company_member_id,authority_id,
               operation_id,stage_execution_id,scope_snapshot_id,organization_id,target_id,
               target_type_at_freeze,target_value_at_freeze,target_source_at_freeze,
               target_created_at,target_identity_sha256,ordinal,state,evolution_epoch,
               max_evolution_epochs)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'domain','compiler.example','fixture',
                  NOW(),$11,0,'analyzing',0,2)"#,
    )
    .bind(asset_lane_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(authority_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(target_id)
    .bind(digest('2'))
    .execute(&mut *tx)
    .await
    .expect("seed compiler asset lane");
    let asset_subject_identity_sha256: String = sqlx::query_scalar(
        r#"SELECT tool_truth_sha256(jsonb_build_object(
               'domain','investigation_subject_identity.v1',
               'subject_kind','asset',
               'subject_id',$1::UUID,
               'display_value','compiler.example'
           )::TEXT)"#,
    )
    .bind(target_id)
    .fetch_one(&mut *tx)
    .await
    .expect("derive compiler asset subject identity");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_snapshots(
               snapshot_id,operation_id,organization_id,wave_ordinal,scope_snapshot_id,
               genesis,source_set_hash,capability_revision_hash,policy_revision_hash,
               credential_revision_hash,snapshot_status,tool_truth_authority_bundle_seal_id,
               stable_consumer_request_id,relevant_root_count,relevant_root_set_hash,
               bundle_member_count,bundle_member_set_hash,semantic_authority_bundle_hash,
               freshness_attestation_bundle_hash,temporal_validity_bundle_hash,
               temporal_validity_policy_set_hash,target_state_epoch_set_hash,
               observation_window_hash,bundle_sealed_at,candidate_snapshot_authority_hash,
               asset_lane_id)
           VALUES($1,$2,$3,0,$4,TRUE,$5,$6,$7,$8,'sealed_ready',$9,$10,4,$11,4,$12,
                  $13,$14,$15,$16,$17,$18,NOW(),$19,$20)"#,
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
    .bind(Uuid::new_v4())
    .bind(digest('7'))
    .bind(digest('8'))
    .bind(digest('9'))
    .bind(digest('b'))
    .bind(digest('c'))
    .bind(digest('d'))
    .bind(digest('e'))
    .bind(digest('f'))
    .bind(digest('0'))
    .bind(asset_lane_id)
    .execute(&mut *tx)
    .await
    .expect("seed Candidate snapshot");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_attempts(
               analysis_attempt_id,snapshot_id,operation_id,organization_id,attempt_ordinal,
               attempt_input_hash,attack_class_checklist_version,attack_class_checklist_digest,
               trust_boundary_checklist_version,trust_boundary_checklist_digest,
               coverage_sampling_contract_version,coverage_sampling_contract_digest,retry_limit,
               asset_lane_id)
           VALUES($1,$2,$3,$4,0,$5,'1',$6,'1',$7,'1',$8,1,$9)"#,
    )
    .bind(attempt_id)
    .bind(snapshot_id)
    .bind(operation_id)
    .bind(organization_id)
    .bind(digest('1'))
    .bind(digest('2'))
    .bind(digest('3'))
    .bind(digest('4'))
    .bind(asset_lane_id)
    .execute(&mut *tx)
    .await
    .expect("seed analysis attempt");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_snapshot_inputs(
               snapshot_input_id,snapshot_id,stable_input_key,source_kind,source_ref,
               source_ref_hash,source_content_hash,source_byte_count,subject_kind_at_time,
               subject_identity_hash,server_chunking_disposition,input_hash)
           VALUES($1,$2,'input:0','evidence','evidence:0',$3,$4,16,'asset',$5,'complete',$6)"#,
    )
    .bind(input_id)
    .bind(snapshot_id)
    .bind(digest('5'))
    .bind(&source_hash)
    .bind(&asset_subject_identity_sha256)
    .bind(digest('7'))
    .execute(&mut *tx)
    .await
    .expect("seed snapshot input");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_input_chunk_censuses(
               chunk_census_id,snapshot_input_id,snapshot_id,chunking_contract_version,
               redaction_contract_version,source_content_hash,source_byte_count,disposition,
               chunk_count,chunk_member_set_hash,census_hash)
           VALUES($1,$2,$3,'1','1',$4,16,'complete',1,$5,$6)"#,
    )
    .bind(chunk_census_id)
    .bind(input_id)
    .bind(snapshot_id)
    .bind(&source_hash)
    .bind(digest('8'))
    .bind(digest('9'))
    .execute(&mut *tx)
    .await
    .expect("seed chunk census");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_input_chunk_census_members(
               chunk_id,chunk_census_id,snapshot_input_id,snapshot_id,ordinal,
               source_range_start,source_range_end,envelope_schema,immutable_redacted_body,
               body_or_blob_hash,chunking_contract_version,redaction_contract_version,chunk_hash)
           VALUES($1,$2,$3,$4,0,0,16,'bounded.v1','{}',$5,'1','1',$6)"#,
    )
    .bind(chunk_id)
    .bind(chunk_census_id)
    .bind(input_id)
    .bind(snapshot_id)
    .bind(digest('a'))
    .bind(digest('b'))
    .execute(&mut *tx)
    .await
    .expect("seed chunk");
    sqlx::query(
        r#"INSERT INTO investigation_analysis_attempt_bindings(
               binding_id,stable_request_id,authority_id,operation_id,stage_execution_id,
               owning_stage_run_request_id,stage_run_unit_id,scope_snapshot_id,organization_id,
               work_id,candidate_snapshot_id,analysis_attempt_id)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)"#,
    )
    .bind(binding_id)
    .bind(Uuid::new_v4())
    .bind(authority_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(owning_request)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(work_id)
    .bind(snapshot_id)
    .bind(attempt_id)
    .execute(&mut *tx)
    .await
    .expect("seed analysis binding");
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_task_plans(
               task_plan_id,stable_request_id,authority_id,stage_team_plan_id,operation_id,
               stage_execution_id,owning_stage_run_request_id,stage_run_unit_id,scope_snapshot_id,
               organization_id,subject_kind,subject_id,subject_fingerprint_sha256,
               task_plan_version,task_plan_sha256,allowed_role_catalog,
               cognitive_tool_envelope_sha256,status,subtask_count,subtask_set_sha256,sealed_at)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'analysis_attempt',$11,$12,1,$13,
                  '["primary"]',$14,'sealed',1,$15,NOW())"#,
    )
    .bind(task_plan_id)
    .bind(Uuid::new_v4())
    .bind(authority_id)
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(owning_request)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(attempt_id)
    .bind(digest('1'))
    .bind(digest('2'))
    .bind(digest('3'))
    .bind(digest('4'))
    .execute(&mut *tx)
    .await
    .expect("seed sealed PentAGI plan");
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_pipeline_events(
               pipeline_event_id,stable_request_id,task_plan_id,event_ordinal,event_kind,
               actor_worker_run_id,parent_dispatch_receipt_id,event_sha256)
           VALUES($1,$2,$3,0,'primary_synthesis',$4,$5,$6)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(task_plan_id)
    .bind(primary_worker_run_id)
    .bind(dispatch_id)
    .bind(digest('5'))
    .execute(&mut *tx)
    .await
    .expect("seed Primary synthesis");
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_delegation_census_seals(
               census_seal_id,stable_request_id,task_plan_id,primary_dispatch_receipt_id,
               primary_worker_run_id,runnable_subtask_count,runnable_subtask_set_sha256,
               dispatch_count,dispatch_set_sha256,pipeline_event_count,
               pipeline_event_set_sha256,seal_sha256)
           VALUES($1,$2,$3,$4,$5,0,$6,1,$7,1,$8,$9)"#,
    )
    .bind(census_id)
    .bind(Uuid::new_v4())
    .bind(task_plan_id)
    .bind(dispatch_id)
    .bind(primary_worker_run_id)
    .bind(digest('6'))
    .bind(digest('7'))
    .bind(digest('8'))
    .bind(digest('9'))
    .execute(&mut *tx)
    .await
    .expect("seed delegation census");
    sqlx::query("INSERT INTO investigation_projection_source_heads(operation_id) VALUES($1)")
        .bind(operation_id)
        .execute(&mut *tx)
        .await
        .expect("seed projection source head");
    sqlx::query("INSERT INTO investigation_projection_heads(operation_id) VALUES($1)")
        .bind(operation_id)
        .execute(&mut *tx)
        .await
        .expect("seed projection head");
    tx.commit().await.expect("commit compact authority seed");

    let proof = InvestigationProofRefInput {
        input_id,
        chunk_id,
        source_hash,
        source_role: "support".to_owned(),
    };
    let proposal_id = Uuid::new_v4();
    Fixture {
        input: PrepareInvestigationCompilationInput {
            stable_compilation_request_id: Uuid::new_v4(),
            authority_id,
            operation_id,
            stage_execution_id,
            stage_run_unit_id,
            scope_snapshot_id,
            organization_id,
            binding_id,
            work_id,
            candidate_snapshot_id: snapshot_id,
            analysis_attempt_id: attempt_id,
            task_plan_id,
            delegation_census_seal_id: census_id,
            primary_worker_run_id,
            pending_evolution_authority_id: None,
            proposals: vec![InvestigationProposalInput {
                proposal_id,
                canonical_proposal: json!({
                    "proposal_id":proposal_id,
                    "subject_kind":"asset",
                    "subject_identity_hash":asset_subject_identity_sha256,
                    "predicate_schema":"http_header_observed.v1",
                    "predicate_version":1,
                    "predicate_arguments":{"header":"server"},
                    "trust_boundary":"public_http",
                    "polarity":"positive",
                    "structured_claim":"The host exposes a server header",
                    "preconditions":[],
                    "impact":"informational",
                    "proof_refs":[proof.clone()],
                    "knowledge_signals":[],
                    "readiness":"ready_for_strategy"
                }),
                proof_refs: vec![proof.clone()],
            }],
            canonical_action_intents: vec![json!({
                "intent_id":Uuid::new_v4(),
                "proposal_id":proposal_id,
                "capability":"http_observation",
                "purpose_code":"confirm_header",
                "evidence_authority_refs":[digest('a')]
            })],
        },
        proof,
    }
}

async fn seed_pending_evolution_fixture(
    pool: &PgPool,
    source: &Fixture,
    source_generation_id: Uuid,
    source_generation_seal_id: Uuid,
) -> (Fixture, Uuid, Uuid) {
    let pending_evolution_authority_id = Uuid::new_v4();
    let consolidation_batch_id = Uuid::new_v4();
    let wave_denominator_id = Uuid::new_v4();
    let wave_coverage_receipt_id = Uuid::new_v4();
    let snapshot_id = Uuid::new_v4();
    let attempt_id = Uuid::new_v4();
    let binding_id = Uuid::new_v4();
    let task_plan_id = Uuid::new_v4();
    let census_id = Uuid::new_v4();
    let input_id = Uuid::new_v4();
    let chunk_census_id = Uuid::new_v4();
    let chunk_id = Uuid::new_v4();
    let dispatch_id = Uuid::new_v4();
    let work_id = Uuid::new_v4();
    let primary_worker_run_id = Uuid::new_v4();
    let project_scope_id: Uuid =
        sqlx::query_scalar("SELECT project_scope_id FROM operation_state WHERE operation_id=$1")
            .bind(source.input.operation_id)
            .fetch_one(pool)
            .await
            .expect("load source project scope");
    let source_asset_lane_id: Uuid = sqlx::query_scalar(
        "SELECT asset_lane_id FROM candidate_analysis_snapshots WHERE snapshot_id=$1",
    )
    .bind(source.input.candidate_snapshot_id)
    .fetch_one(pool)
    .await
    .expect("load source asset lane");
    let source_snapshot_hash = digest('3');
    let applied_fact_delta_set_hash = digest('4');
    let residual_set_hash = digest('5');

    let mut tx = pool.begin().await.expect("begin pending evolution seed");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("disable seed-only triggers");
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,organization_id,
               worker_generation,specialist,work_item_kind,work_item_key,agent_path,status)
           SELECT $1,operation_id,stage_execution_id,stage_run_unit_id,organization_id,
                  worker_generation,specialist,work_item_kind,'evolution-primary',
                  'main/evolution-primary',status
             FROM stage_worker_runs WHERE id=$2"#,
    )
    .bind(primary_worker_run_id)
    .bind(source.input.primary_worker_run_id)
    .execute(&mut *tx)
    .await
    .expect("clone successor Primary worker");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_snapshots(
               snapshot_id,operation_id,organization_id,wave_ordinal,scope_snapshot_id,
               genesis,previous_generation_seal_id,source_set_hash,fact_delta_watermark,
               capability_revision_hash,policy_revision_hash,credential_revision_hash,
               snapshot_status,tool_truth_authority_bundle_seal_id,
               stable_consumer_request_id,relevant_root_count,relevant_root_set_hash,
               bundle_member_count,bundle_member_set_hash,semantic_authority_bundle_hash,
               freshness_attestation_bundle_hash,temporal_validity_bundle_hash,
               temporal_validity_policy_set_hash,target_state_epoch_set_hash,
               observation_window_hash,bundle_sealed_at,candidate_snapshot_authority_hash,
               asset_lane_id)
           SELECT $1,operation_id,organization_id,1,scope_snapshot_id,
                  FALSE,$2,source_set_hash,1,capability_revision_hash,
                  policy_revision_hash,credential_revision_hash,snapshot_status,
                  tool_truth_authority_bundle_seal_id,$3,relevant_root_count,
                  relevant_root_set_hash,bundle_member_count,bundle_member_set_hash,
                  semantic_authority_bundle_hash,freshness_attestation_bundle_hash,
                  temporal_validity_bundle_hash,temporal_validity_policy_set_hash,
                  target_state_epoch_set_hash,observation_window_hash,NOW(),$4,asset_lane_id
             FROM candidate_analysis_snapshots WHERE snapshot_id=$5"#,
    )
    .bind(snapshot_id)
    .bind(source_generation_seal_id)
    .bind(Uuid::new_v4())
    .bind(digest('6'))
    .bind(source.input.candidate_snapshot_id)
    .execute(&mut *tx)
    .await
    .expect("clone successor Candidate snapshot");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_attempts(
               analysis_attempt_id,snapshot_id,operation_id,organization_id,attempt_ordinal,
               attempt_input_hash,attack_class_checklist_version,attack_class_checklist_digest,
               trust_boundary_checklist_version,trust_boundary_checklist_digest,
               coverage_sampling_contract_version,coverage_sampling_contract_digest,retry_limit,
               asset_lane_id)
           SELECT $1,$2,operation_id,organization_id,0,$3,
                  attack_class_checklist_version,attack_class_checklist_digest,
                  trust_boundary_checklist_version,trust_boundary_checklist_digest,
                  coverage_sampling_contract_version,coverage_sampling_contract_digest,retry_limit,
                  asset_lane_id
             FROM candidate_analysis_attempts WHERE analysis_attempt_id=$4"#,
    )
    .bind(attempt_id)
    .bind(snapshot_id)
    .bind(digest('7'))
    .bind(source.input.analysis_attempt_id)
    .execute(&mut *tx)
    .await
    .expect("clone successor analysis attempt");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_snapshot_inputs(
               snapshot_input_id,snapshot_id,stable_input_key,source_kind,source_ref,
               source_ref_hash,source_content_hash,source_byte_count,subject_kind_at_time,
               subject_identity_hash,server_chunking_disposition,input_hash)
           SELECT $1,$2,stable_input_key,source_kind,source_ref,source_ref_hash,
                  source_content_hash,source_byte_count,subject_kind_at_time,
                  subject_identity_hash,server_chunking_disposition,input_hash
             FROM candidate_analysis_snapshot_inputs WHERE snapshot_id=$3"#,
    )
    .bind(input_id)
    .bind(snapshot_id)
    .bind(source.input.candidate_snapshot_id)
    .execute(&mut *tx)
    .await
    .expect("clone successor snapshot input");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_input_chunk_censuses(
               chunk_census_id,snapshot_input_id,snapshot_id,chunking_contract_version,
               redaction_contract_version,source_content_hash,source_byte_count,disposition,
               chunk_count,chunk_member_set_hash,census_hash)
           SELECT $1,$2,$3,chunking_contract_version,redaction_contract_version,
                  source_content_hash,source_byte_count,disposition,chunk_count,
                  chunk_member_set_hash,census_hash
             FROM candidate_analysis_input_chunk_censuses WHERE snapshot_id=$4"#,
    )
    .bind(chunk_census_id)
    .bind(input_id)
    .bind(snapshot_id)
    .bind(source.input.candidate_snapshot_id)
    .execute(&mut *tx)
    .await
    .expect("clone successor chunk census");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_input_chunk_census_members(
               chunk_id,chunk_census_id,snapshot_input_id,snapshot_id,ordinal,
               source_range_start,source_range_end,envelope_schema,immutable_redacted_body,
               body_or_blob_hash,chunking_contract_version,redaction_contract_version,chunk_hash)
           SELECT $1,$2,$3,$4,ordinal,source_range_start,source_range_end,envelope_schema,
                  immutable_redacted_body,body_or_blob_hash,chunking_contract_version,
                  redaction_contract_version,chunk_hash
             FROM candidate_analysis_input_chunk_census_members WHERE snapshot_id=$5"#,
    )
    .bind(chunk_id)
    .bind(chunk_census_id)
    .bind(input_id)
    .bind(snapshot_id)
    .bind(source.input.candidate_snapshot_id)
    .execute(&mut *tx)
    .await
    .expect("clone successor proof chunk");
    sqlx::query(
        r#"INSERT INTO investigation_analysis_attempt_bindings(
               binding_id,stable_request_id,authority_id,operation_id,stage_execution_id,
               owning_stage_run_request_id,stage_run_unit_id,scope_snapshot_id,organization_id,
               work_id,candidate_snapshot_id,analysis_attempt_id)
           SELECT $1,$2,authority_id,operation_id,stage_execution_id,
                  owning_stage_run_request_id,stage_run_unit_id,scope_snapshot_id,organization_id,
                  $3,$4,$5 FROM investigation_analysis_attempt_bindings WHERE binding_id=$6"#,
    )
    .bind(binding_id)
    .bind(Uuid::new_v4())
    .bind(work_id)
    .bind(snapshot_id)
    .bind(attempt_id)
    .bind(source.input.binding_id)
    .execute(&mut *tx)
    .await
    .expect("clone successor analysis binding");
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_task_plans(
               task_plan_id,stable_request_id,authority_id,stage_team_plan_id,operation_id,
               stage_execution_id,owning_stage_run_request_id,stage_run_unit_id,scope_snapshot_id,
               organization_id,subject_kind,subject_id,subject_fingerprint_sha256,
               task_plan_version,task_plan_sha256,allowed_role_catalog,
               cognitive_tool_envelope_sha256,status,subtask_count,subtask_set_sha256,sealed_at)
           SELECT $1,$2,authority_id,stage_team_plan_id,operation_id,stage_execution_id,
                  owning_stage_run_request_id,stage_run_unit_id,scope_snapshot_id,organization_id,
                  subject_kind,$3,$4,task_plan_version,$5,allowed_role_catalog,
                  cognitive_tool_envelope_sha256,status,subtask_count,subtask_set_sha256,NOW()
             FROM investigation_pentagi_task_plans WHERE task_plan_id=$6"#,
    )
    .bind(task_plan_id)
    .bind(Uuid::new_v4())
    .bind(attempt_id)
    .bind(digest('8'))
    .bind(digest('9'))
    .bind(source.input.task_plan_id)
    .execute(&mut *tx)
    .await
    .expect("clone successor task plan");
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_pipeline_events(
               pipeline_event_id,stable_request_id,task_plan_id,event_ordinal,event_kind,
               actor_worker_run_id,parent_dispatch_receipt_id,event_sha256)
           VALUES($1,$2,$3,0,'primary_synthesis',$4,$5,$6)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(task_plan_id)
    .bind(primary_worker_run_id)
    .bind(dispatch_id)
    .bind(digest('a'))
    .execute(&mut *tx)
    .await
    .expect("seed successor synthesis");
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_delegation_census_seals(
               census_seal_id,stable_request_id,task_plan_id,primary_dispatch_receipt_id,
               primary_worker_run_id,runnable_subtask_count,runnable_subtask_set_sha256,
               dispatch_count,dispatch_set_sha256,pipeline_event_count,
               pipeline_event_set_sha256,seal_sha256)
           VALUES($1,$2,$3,$4,$5,0,$6,1,$7,1,$8,$9)"#,
    )
    .bind(census_id)
    .bind(Uuid::new_v4())
    .bind(task_plan_id)
    .bind(dispatch_id)
    .bind(primary_worker_run_id)
    .bind(digest('b'))
    .bind(digest('c'))
    .bind(digest('d'))
    .bind(digest('e'))
    .execute(&mut *tx)
    .await
    .expect("seed successor census");
    sqlx::query(
        r#"INSERT INTO verification_wave_coverage_denominators(
               wave_denominator_id,stable_request_id,operation_id,project_scope_id,
               organization_id,asset_lane_id,generation_seal_id,contract_version,source_snapshot_hash,
               member_set_hash,member_count,sealed_at)
           VALUES($1,$2,$3,$4,$5,$6,$7,'test.v1',$8,$9,1,NOW())"#,
    )
    .bind(wave_denominator_id)
    .bind(Uuid::new_v4())
    .bind(source.input.operation_id)
    .bind(project_scope_id)
    .bind(source.input.organization_id)
    .bind(source_asset_lane_id)
    .bind(source_generation_seal_id)
    .bind(&source_snapshot_hash)
    .bind(digest('f'))
    .execute(&mut *tx)
    .await
    .expect("seed source wave");
    sqlx::query(
        r#"INSERT INTO verification_wave_coverage_receipts(
               wave_coverage_receipt_id,stable_request_id,wave_denominator_id,
               operation_id,project_scope_id,organization_id,
               selected_campaign_receipt_set_hash,unassigned_result_set_hash,
               result_member_count,result_member_set_hash,coverage_status,receipt_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,1,$9,'complete',$10)"#,
    )
    .bind(wave_coverage_receipt_id)
    .bind(Uuid::new_v4())
    .bind(wave_denominator_id)
    .bind(source.input.operation_id)
    .bind(project_scope_id)
    .bind(source.input.organization_id)
    .bind(digest('0'))
    .bind(&residual_set_hash)
    .bind(digest('1'))
    .bind(digest('2'))
    .execute(&mut *tx)
    .await
    .expect("seed source wave receipt");
    sqlx::query(
        r#"INSERT INTO hypothesis_consolidation_batches(
               consolidation_batch_id,stable_request_id,operation_id,project_scope_id,
               organization_id,generation_id,wave_coverage_receipt_id,
               fact_delta_member_count,fact_delta_member_set_hash,
               unassigned_residual_set_hash,source_snapshot_hash,sealed_at)
           VALUES($1,$2,$3,$4,$5,$6,$7,1,$8,$9,$10,NOW())"#,
    )
    .bind(consolidation_batch_id)
    .bind(Uuid::new_v4())
    .bind(source.input.operation_id)
    .bind(project_scope_id)
    .bind(source.input.organization_id)
    .bind(source_generation_id)
    .bind(wave_coverage_receipt_id)
    .bind(digest('3'))
    .bind(&residual_set_hash)
    .bind(&source_snapshot_hash)
    .execute(&mut *tx)
    .await
    .expect("seed consolidation batch");
    sqlx::query(
        r#"INSERT INTO hypothesis_pending_evolution_authorities(
               pending_evolution_authority_id,stable_request_id,consolidation_batch_id,
               operation_id,project_scope_id,organization_id,asset_lane_id,source_generation_id,
               source_wave_denominator_id,wave_coverage_receipt_id,fact_delta_member_count,
               applied_fact_delta_set_hash,residual_set_hash,source_snapshot_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,1,$11,$12,$13)"#,
    )
    .bind(pending_evolution_authority_id)
    .bind(Uuid::new_v4())
    .bind(consolidation_batch_id)
    .bind(source.input.operation_id)
    .bind(project_scope_id)
    .bind(source.input.organization_id)
    .bind(source_asset_lane_id)
    .bind(source_generation_id)
    .bind(wave_denominator_id)
    .bind(wave_coverage_receipt_id)
    .bind(&applied_fact_delta_set_hash)
    .bind(&residual_set_hash)
    .bind(&source_snapshot_hash)
    .execute(&mut *tx)
    .await
    .expect("seed pending evolution authority");
    tx.commit().await.expect("commit pending evolution seed");

    let proof = InvestigationProofRefInput {
        input_id,
        chunk_id,
        source_hash: source.proof.source_hash.clone(),
        source_role: source.proof.source_role.clone(),
    };
    let mut input = source.input.clone();
    input.stable_compilation_request_id = Uuid::new_v4();
    input.binding_id = binding_id;
    input.work_id = work_id;
    input.candidate_snapshot_id = snapshot_id;
    input.analysis_attempt_id = attempt_id;
    input.task_plan_id = task_plan_id;
    input.delegation_census_seal_id = census_id;
    input.primary_worker_run_id = primary_worker_run_id;
    input.pending_evolution_authority_id = Some(pending_evolution_authority_id);
    input.proposals.clear();
    input.canonical_action_intents.clear();
    (
        Fixture { input, proof },
        pending_evolution_authority_id,
        consolidation_batch_id,
    )
}

fn recipe_uuid(value: &Value, path: &[&str]) -> Uuid {
    let mut cursor = value;
    for key in path {
        cursor = &cursor[*key];
    }
    Uuid::parse_str(cursor.as_str().expect("recipe UUID string")).expect("valid recipe UUID")
}

fn recipe_text<'a>(value: &'a Value, path: &[&str]) -> &'a str {
    let mut cursor = value;
    for key in path {
        cursor = &cursor[*key];
    }
    cursor.as_str().expect("recipe text")
}

fn compile_apply_input(
    prepared: golish_db::repo::investigation_hypothesis_compiler::PreparedInvestigationCompilation,
) -> ApplyInvestigationCompilationInput {
    let pending_evolution_authority_id = prepared.input.pending_evolution_authority_id;
    let item = &prepared.server_recipe["items"][0];
    let proposal_id = recipe_uuid(item, &["proposal_id"]);
    let root_id = recipe_uuid(item, &["route", "root_id"]);
    let revision_id = recipe_uuid(item, &["revision", "revision_id"]);
    let revision_hash = recipe_text(item, &["revision", "revision_hash"]).to_owned();
    let revision_ingredients_hash =
        recipe_text(item, &["revision", "revision_ingredients_hash"]).to_owned();
    let component_hash = recipe_text(item, &["revision", "claim_clause_hash"]).to_owned();
    let derivation_digest = recipe_text(item, &["revision", "derivation_digest"]).to_owned();
    let components = compile_claim_components_v1(
        revision_id,
        revision_hash.clone(),
        1,
        derivation_digest,
        vec![HypothesisClaimComponentInputV1 {
            component_key: "claim_clause".to_owned(),
            kind: HypothesisClaimComponentKindV1::ClaimClause,
            canonical_fragment_hash: component_hash.clone(),
            canonical_condition_hash: component_hash,
            required: true,
        }],
    )
    .expect("compile canonical claim component");
    let objective_id = recipe_uuid(item, &["revision", "objective_id"]);
    let contract = VerificationContractV1::compile(VerificationContractBuildInputV1 {
        revision_id,
        revision_hash: revision_hash.clone(),
        objective_id,
        combinator: ContractCombinatorV1::AllOf,
        predicate_components: vec![PredicateComponentInputV1 {
            semantic_key: recipe_text(item, &["semantic_key_hash"]).to_owned(),
            predicate_schema: recipe_text(item, &["predicate_schema"]).to_owned(),
            predicate_version: 1,
            normalized_arguments: CanonicalJsonObject::try_from_value(
                item.get("predicate_arguments")
                    .cloned()
                    .expect("server recipe predicate arguments"),
            )
            .expect("canonical predicate args"),
            expected_polarity: ClaimPolarity::try_from(recipe_text(item, &["polarity"]))
                .expect("canonical predicate polarity"),
            prerequisite_hash: recipe_text(item, &["revision", "identity_hash"]).to_owned(),
        }],
        required_controls: Vec::new(),
        paired_differential_bindings: Vec::new(),
        ordered_steps: Vec::new(),
        stopping_criteria_hash: recipe_text(item, &["revision", "stopping_criteria_hash"])
            .to_owned(),
        compiler_digest: recipe_text(item, &["revision", "compiler_digest"]).to_owned(),
        rule_digest: recipe_text(item, &["revision", "rule_digest"]).to_owned(),
        policy_snapshot_hash: recipe_text(item, &["revision", "policy_snapshot_hash"]).to_owned(),
    })
    .expect("compile verification contract");
    let component_member_hash = components[0].member_hash().to_owned();
    let plan = HypothesisVerificationPlanV1::compile(HypothesisVerificationPlanBuildInputV1 {
        revision_id,
        revision_hash,
        revision_ingredients_hash,
        required_claim_components: components.clone(),
        objectives: vec![HypothesisVerificationPlanObjectiveInputV1 {
            objective_hash: recipe_text(item, &["revision", "objective_hash"]).to_owned(),
            verification_contract: contract.clone(),
            claim_component_member_hashes: vec![component_member_hash.clone()],
            outcome_requirement:
                HypothesisVerificationObjectiveOutcomeRequirementV1::SatisfyOrFalsifyBoundRequiredComponents,
        }],
        proof_paths: vec![HypothesisVerificationPlanPathInputV1 {
            path_key: "primary".to_owned(),
            members: vec![HypothesisVerificationPlanPathMemberInputV1 {
                objective_id,
                role: HypothesisVerificationPlanPathMemberRoleV1::RequiredProofAndPathFalsifier,
                falsifier_claim_component_member_hashes: vec![component_member_hash],
            }],
        }],
        outer_aggregation_policy_version: 1,
        outer_aggregation_policy_digest: recipe_text(
            item,
            &["revision", "outer_policy_digest"],
        )
        .to_owned(),
    })
    .expect("compile verification plan");
    let mutation = reseal_investigation_mutation(CandidateMutationRow {
        proposal_id,
        organization_id: prepared.input.organization_id,
        semantic_key_hash: recipe_text(item, &["semantic_key_hash"]).to_owned(),
        operator_rank: 0,
        state: CandidateMutationEpistemicState::Proposed,
        proof_refs: vec![CandidateRevisionSourceRefRow::ToolTruthEvidence(
            prepared.input.proposals[0].proof_refs[0]
                .source_hash
                .clone(),
        )],
        refutation_refs: Vec::new(),
        generation_transition_hash: recipe_text(item, &["generation_transition_hash"]).to_owned(),
        mutation_hash: String::new(),
        route: CandidateMutationRouteRow::CreateInitial { root_id },
    });
    ApplyInvestigationCompilationInput {
        prepared,
        stable_apply_request_id: Uuid::new_v4(),
        stable_admission_request_id: Uuid::new_v4(),
        pending_evolution_authority_id,
        mutations: vec![mutation],
        claim_components: components,
        verification_contracts: vec![contract],
        verification_plans: vec![plan],
    }
}

#[tokio::test]
#[serial]
async fn pending_evolution_zero_revision_closes_fixed_point_without_empty_successor_and_replays() {
    let (mut db, _data_dir) = migrated_db("evolution-fixed-point").await;
    let source_fixture = seed_fixture(db.pool()).await;
    let source_prepared =
        prepare_investigation_compilation(db.pool(), source_fixture.input.clone())
            .await
            .expect("prepare source generation");
    let source_apply = compile_apply_input(source_prepared);
    let source = apply_investigation_compilation(db.pool(), source_apply)
        .await
        .expect("seal source generation");
    let (evolution_fixture, pending_id, batch_id) = seed_pending_evolution_fixture(
        db.pool(),
        &source_fixture,
        source.generation_id,
        source.generation_seal_id,
    )
    .await;
    let prepared = prepare_investigation_compilation(db.pool(), evolution_fixture.input)
        .await
        .expect("prepare zero-revision evolution");
    let apply = ApplyInvestigationCompilationInput {
        pending_evolution_authority_id: Some(pending_id),
        prepared,
        stable_apply_request_id: Uuid::new_v4(),
        stable_admission_request_id: Uuid::new_v4(),
        mutations: Vec::new(),
        claim_components: Vec::new(),
        verification_contracts: Vec::new(),
        verification_plans: Vec::new(),
    };
    let first = apply_investigation_compilation(db.pool(), apply.clone())
        .await
        .expect("close pending evolution at fixed point");
    assert!(first.evolution_fixed_point);
    assert!(!first.replayed);
    assert_eq!(first.generation_id, source.generation_id);
    assert!(first.verification_task_ids.is_empty());

    let terminal: (String, Option<Uuid>, Uuid) = sqlx::query_as(
        r#"SELECT consolidation.disposition,consolidation.successor_generation_id,
                  fixed.generation_id
             FROM hypothesis_consolidation_receipts consolidation
             JOIN hypothesis_fixed_point_receipts fixed
               ON fixed.consolidation_receipt_id=consolidation.consolidation_receipt_id
            WHERE consolidation.consolidation_batch_id=$1"#,
    )
    .bind(batch_id)
    .fetch_one(db.pool())
    .await
    .expect("load fixed-point terminal receipts");
    assert_eq!(
        terminal,
        ("fixed_point".to_owned(), None, source.generation_id)
    );
    let counts_before: (i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT
              (SELECT COUNT(*) FROM hypothesis_generations),
              (SELECT COUNT(*) FROM hypothesis_consolidation_receipts),
              (SELECT COUNT(*) FROM hypothesis_fixed_point_receipts),
              (SELECT COUNT(*) FROM investigation_evolution_fixed_point_apply_receipts)"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("count fixed-point rows");
    assert_eq!(counts_before, (1, 1, 1, 1));

    let replay = apply_investigation_compilation(db.pool(), apply)
        .await
        .expect("exact fixed-point replay");
    assert!(replay.evolution_fixed_point);
    assert!(replay.replayed);
    assert_eq!(replay.generation_id, source.generation_id);
    let counts_after: (i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT
              (SELECT COUNT(*) FROM hypothesis_generations),
              (SELECT COUNT(*) FROM hypothesis_consolidation_receipts),
              (SELECT COUNT(*) FROM hypothesis_fixed_point_receipts),
              (SELECT COUNT(*) FROM investigation_evolution_fixed_point_apply_receipts)"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("count rows after fixed-point replay");
    assert_eq!(counts_after, counts_before);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn pending_evolution_new_revision_creates_exact_successor_and_advanced_receipt_and_replays() {
    let (mut db, _data_dir) = migrated_db("evolution-advanced").await;
    let source_fixture = seed_fixture(db.pool()).await;
    let source_subject_kind = source_fixture.input.proposals[0].canonical_proposal["subject_kind"]
        .as_str()
        .expect("source subject kind")
        .to_owned();
    let source_subject_identity = source_fixture.input.proposals[0].canonical_proposal
        ["subject_identity_hash"]
        .as_str()
        .expect("source subject identity")
        .to_owned();
    let source_prepared =
        prepare_investigation_compilation(db.pool(), source_fixture.input.clone())
            .await
            .expect("prepare source generation");
    let source_apply = compile_apply_input(source_prepared);
    let source = apply_investigation_compilation(db.pool(), source_apply)
        .await
        .expect("seal source generation");
    let (mut evolution_fixture, pending_id, batch_id) = seed_pending_evolution_fixture(
        db.pool(),
        &source_fixture,
        source.generation_id,
        source.generation_seal_id,
    )
    .await;
    add_numbered_proposal_for_subject(
        &mut evolution_fixture.input,
        &evolution_fixture.proof,
        source_subject_kind,
        source_subject_identity,
        2,
        '7',
    );
    let prepared = prepare_investigation_compilation(db.pool(), evolution_fixture.input)
        .await
        .expect("prepare successor revision");
    let apply = compile_apply_input(prepared);
    let first = apply_investigation_compilation(db.pool(), apply.clone())
        .await
        .expect("atomically seal successor and advanced receipt");
    assert!(!first.evolution_fixed_point);
    assert!(!first.replayed);
    assert_ne!(first.generation_id, source.generation_id);
    assert_eq!(first.verification_task_ids.len(), 1);

    let advanced: (String, Option<Uuid>, Uuid, Option<Uuid>) = sqlx::query_as(
        r#"SELECT consolidation.disposition,consolidation.successor_generation_id,
                  generation.generation_id,generation.previous_generation_id
             FROM hypothesis_consolidation_receipts consolidation
             JOIN hypothesis_generations generation
               ON generation.generation_id=consolidation.successor_generation_id
            WHERE consolidation.consolidation_batch_id=$1"#,
    )
    .bind(batch_id)
    .fetch_one(db.pool())
    .await
    .expect("load advanced successor authority");
    assert_eq!(advanced.0, "advanced");
    assert_eq!(advanced.1, Some(first.generation_id));
    assert_eq!(advanced.2, first.generation_id);
    assert_eq!(advanced.3, Some(source.generation_id));
    let exact_pending: (Uuid, Uuid, String, String) = sqlx::query_as(
        r#"SELECT pending.pending_evolution_authority_id,pending.source_generation_id,
                  pending.applied_fact_delta_set_hash,pending.residual_set_hash
             FROM hypothesis_pending_evolution_authorities pending
             JOIN hypothesis_consolidation_receipts consolidation
               ON consolidation.consolidation_batch_id=pending.consolidation_batch_id
            WHERE pending.pending_evolution_authority_id=$1
              AND consolidation.successor_generation_id=$2"#,
    )
    .bind(pending_id)
    .bind(first.generation_id)
    .fetch_one(db.pool())
    .await
    .expect("load exact pending-to-successor chain");
    assert_eq!(exact_pending.0, pending_id);
    assert_eq!(exact_pending.1, source.generation_id);
    assert_eq!(exact_pending.2, digest('4'));
    assert_eq!(exact_pending.3, digest('5'));
    let counts_before: (i64, i64, i64) = sqlx::query_as(
        r#"SELECT
              (SELECT COUNT(*) FROM hypothesis_generations),
              (SELECT COUNT(*) FROM hypothesis_consolidation_receipts),
              (SELECT COUNT(*) FROM investigation_hypothesis_canonical_apply_receipts)"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("count advanced rows");
    assert_eq!(counts_before, (2, 1, 2));

    let replay = apply_investigation_compilation(db.pool(), apply)
        .await
        .expect("exact advanced replay");
    assert!(replay.replayed);
    assert!(!replay.evolution_fixed_point);
    assert_eq!(replay.generation_id, first.generation_id);
    let counts_after: (i64, i64, i64) = sqlx::query_as(
        r#"SELECT
              (SELECT COUNT(*) FROM hypothesis_generations),
              (SELECT COUNT(*) FROM hypothesis_consolidation_receipts),
              (SELECT COUNT(*) FROM investigation_hypothesis_canonical_apply_receipts)"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("count rows after advanced replay");
    assert_eq!(counts_after, counts_before);
    db.stop().await;
}

fn compile_multi_apply_input(
    prepared: golish_db::repo::investigation_hypothesis_compiler::PreparedInvestigationCompilation,
) -> ApplyInvestigationCompilationInput {
    let mut combined = compile_apply_input(prepared.clone());
    for item in prepared.server_recipe["items"]
        .as_array()
        .expect("server recipe items")
        .iter()
        .skip(1)
    {
        let proposal_id = recipe_uuid(item, &["proposal_id"]);
        let mut single = prepared.clone();
        single.server_recipe["items"] = Value::Array(vec![item.clone()]);
        single.input.proposals = vec![prepared
            .input
            .proposals
            .iter()
            .find(|proposal| proposal.proposal_id == proposal_id)
            .expect("recipe proposal")
            .clone()];
        let compiled = compile_apply_input(single);
        combined.mutations.extend(compiled.mutations);
        combined.claim_components.extend(compiled.claim_components);
        combined
            .verification_contracts
            .extend(compiled.verification_contracts);
        combined
            .verification_plans
            .extend(compiled.verification_plans);
    }
    combined
}

fn add_second_proposal(
    input: &mut PrepareInvestigationCompilationInput,
    proof: &InvestigationProofRefInput,
) {
    add_numbered_proposal(input, proof, 2, '7');
}

fn add_numbered_proposal(
    input: &mut PrepareInvestigationCompilationInput,
    proof: &InvestigationProofRefInput,
    ordinal: usize,
    subject_hash_nibble: char,
) {
    let subject_kind = input.proposals[0].canonical_proposal["subject_kind"]
        .as_str()
        .expect("fixture subject kind")
        .to_owned();
    let subject_identity_hash = input.proposals[0].canonical_proposal["subject_identity_hash"]
        .as_str()
        .expect("fixture subject identity")
        .to_owned();
    add_numbered_proposal_for_subject(
        input,
        proof,
        subject_kind,
        subject_identity_hash,
        ordinal,
        subject_hash_nibble,
    );
}

fn add_numbered_proposal_for_subject(
    input: &mut PrepareInvestigationCompilationInput,
    proof: &InvestigationProofRefInput,
    subject_kind: String,
    subject_identity_hash: String,
    ordinal: usize,
    subject_hash_nibble: char,
) {
    let proposal_id = Uuid::new_v4();
    let header_value = format!("server-{ordinal}-{subject_hash_nibble}");
    input.proposals.push(InvestigationProposalInput {
        proposal_id,
        canonical_proposal: json!({
            "proposal_id":proposal_id,
            "subject_kind":subject_kind,
            "subject_identity_hash":subject_identity_hash,
            "predicate_schema":"http_header_observed.v1",
            "predicate_version":1,
            "predicate_arguments":{"header":header_value},
            "trust_boundary":"public_http",
            "polarity":"positive",
            "structured_claim":format!("Host identity {ordinal} exposes a server header"),
            "preconditions":[],
            "impact":"informational",
            "proof_refs":[proof.clone()],
            "knowledge_signals":[],
            "readiness":"ready_for_strategy"
        }),
        proof_refs: vec![proof.clone()],
    });
    input.canonical_action_intents.push(json!({
        "intent_id":Uuid::new_v4(),
        "proposal_id":proposal_id,
        "capability":"http_observation",
        "purpose_code":format!("confirm_header_{ordinal}"),
        "evidence_authority_refs":[digest('a')]
    }));
}

#[tokio::test]
#[serial]
async fn investigation_compile_seal_and_admit_is_atomic_and_exactly_replayable() {
    let (mut db, _data_dir) = migrated_db("atomic").await;
    let fixture = seed_fixture(db.pool()).await;
    let prepared = prepare_investigation_compilation(db.pool(), fixture.input.clone())
        .await
        .expect("prepare exact Investigation authority");
    assert_eq!(prepared.resolved_proofs.len(), 1);
    assert_eq!(
        prepared.resolved_proofs[0].source_hash,
        fixture.proof.source_hash
    );
    assert_eq!(
        prepared.server_recipe["items"][0]["route"]["kind"],
        "create_initial"
    );

    let apply_input = compile_apply_input(prepared);
    let first = apply_investigation_compilation(db.pool(), apply_input.clone())
        .await
        .expect("atomically compile, seal, and admit");
    assert!(!first.replayed);
    assert_eq!(first.generation_member_count, 1);
    assert_eq!(first.verification_task_ids.len(), 1);

    for (table, expected) in [
        ("investigation_hypothesis_compilation_decisions", 1_i64),
        ("investigation_hypothesis_compilation_members", 1),
        ("investigation_hypothesis_compilation_proof_members", 1),
        ("attack_hypotheses", 1),
        ("attack_hypothesis_revisions", 1),
        ("attack_hypothesis_claim_components", 1),
        ("attack_hypothesis_verification_contracts", 1),
        ("attack_hypothesis_verification_plans", 1),
        ("hypothesis_generations", 1),
        ("hypothesis_generation_seals", 1),
        ("hypothesis_verification_tasks", 1),
        ("verification_admission_sets", 1),
        ("verification_admission_members", 1),
        ("hypothesis_verification_task_assignment_sets", 0),
        ("hypothesis_verification_task_assignment_members", 0),
        ("hypothesis_verification_task_campaigns", 0),
        ("investigation_projection_outbox_batches", 1),
        ("investigation_hypothesis_canonical_apply_receipts", 1),
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(db.pool())
            .await
            .unwrap_or_else(|error| panic!("count {table}: {error}"));
        assert_eq!(count, expected, "unexpected count for {table}");
    }
    let task_contract: String = sqlx::query_scalar(
        "SELECT task_contract_version FROM hypothesis_verification_tasks WHERE task_id=$1",
    )
    .bind(first.verification_task_ids[0])
    .fetch_one(db.pool())
    .await
    .expect("load dynamic task admission contract");
    assert_eq!(task_contract, "hypothesis_verification_task.dynamic_v2");

    let assignment_error = sqlx::query(
        r#"INSERT INTO hypothesis_verification_task_assignment_sets(
               assignment_set_id,stable_request_id,task_id,hypothesis_revision_id,
               verification_plan_id)
           SELECT $1,$2,task_id,hypothesis_revision_id,verification_plan_id
             FROM hypothesis_verification_tasks WHERE task_id=$3"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(first.verification_task_ids[0])
    .execute(db.pool())
    .await
    .expect_err("dynamic task must reject historical objective assignment authority");
    assert!(
        assignment_error
            .to_string()
            .contains("INVESTIGATION_DYNAMIC_TASK_ASSIGNMENT_FORBIDDEN"),
        "unexpected dynamic assignment error: {assignment_error}"
    );

    let replay = apply_investigation_compilation(db.pool(), apply_input.clone())
        .await
        .expect("exact replay");
    let mut expected_replay = first.clone();
    expected_replay.replayed = true;
    assert_eq!(replay, expected_replay);

    let mut drift = apply_input;
    drift.mutations[0].state = CandidateMutationEpistemicState::Contested;
    drift.mutations[0] = reseal_investigation_mutation(drift.mutations[0].clone());
    let error = apply_investigation_compilation(db.pool(), drift)
        .await
        .expect_err("same stable apply id with changed compiled material must be fenced");
    assert!(
        error
            .to_string()
            .contains("INVESTIGATION_HYPOTHESIS_COMPILER_REPLAY_DRIFT"),
        "unexpected replay drift error: {error}"
    );
    let receipt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM investigation_hypothesis_canonical_apply_receipts",
    )
    .fetch_one(db.pool())
    .await
    .expect("count durable apply receipt after drift");
    assert_eq!(receipt_count, 1);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn investigation_compiler_seals_a_zero_proposal_generation_without_tasks() {
    let (mut db, _data_dir) = migrated_db("zero-proposal").await;
    let mut fixture = seed_fixture(db.pool()).await;
    fixture.input.proposals.clear();
    fixture.input.canonical_action_intents.clear();
    let prepared = prepare_investigation_compilation(db.pool(), fixture.input)
        .await
        .expect("prepare the exact empty proposal set");
    assert_eq!(prepared.server_recipe["items"], json!([]));

    let applied = apply_investigation_compilation(
        db.pool(),
        ApplyInvestigationCompilationInput {
            prepared,
            stable_apply_request_id: Uuid::new_v4(),
            stable_admission_request_id: Uuid::new_v4(),
            pending_evolution_authority_id: None,
            mutations: Vec::new(),
            claim_components: Vec::new(),
            verification_contracts: Vec::new(),
            verification_plans: Vec::new(),
        },
    )
    .await
    .expect("seal an auditable zero-proposal generation");
    assert_eq!(applied.generation_member_count, 0);
    assert!(applied.verification_task_ids.is_empty());

    let counts: (i64, i64, i64) = sqlx::query_as(
        r#"SELECT
               (SELECT COUNT(*) FROM investigation_hypothesis_compilation_decisions),
               (SELECT COUNT(*) FROM hypothesis_generation_seals),
               (SELECT COUNT(*) FROM verification_admission_sets)"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("load zero-proposal durable artifact counts");
    assert_eq!(counts, (1, 1, 1));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn investigation_compiler_apply_preserves_residual_ready_snapshot_authority() {
    let (mut db, _data_dir) = migrated_db("residual-ready-snapshot").await;
    let fixture = seed_fixture(db.pool()).await;
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin residual-ready fixture adjustment");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("disable append-only triggers for fixture adjustment");
    sqlx::query(
        "UPDATE candidate_analysis_snapshots
            SET snapshot_status='sealed_analysis_ready_with_residuals'
          WHERE snapshot_id=$1",
    )
    .bind(fixture.input.candidate_snapshot_id)
    .execute(&mut *tx)
    .await
    .expect("mark Candidate snapshot analysis-ready with residuals");
    tx.commit()
        .await
        .expect("commit residual-ready fixture adjustment");

    let prepared = prepare_investigation_compilation(db.pool(), fixture.input)
        .await
        .expect("prepare residual-ready Candidate authority");
    let applied = apply_investigation_compilation(db.pool(), compile_apply_input(prepared))
        .await
        .expect("apply must accept the same residual-ready authority as prepare");
    assert_eq!(applied.generation_member_count, 1);
    assert_eq!(applied.verification_task_ids.len(), 1);

    let decision_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM investigation_hypothesis_compilation_decisions")
            .fetch_one(db.pool())
            .await
            .expect("count residual-ready compilation decision");
    assert_eq!(decision_count, 1);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn zero_proposal_preparation_is_canonical_and_writes_nothing() {
    let (mut db, _data_dir) = migrated_db("zero").await;
    let mut fixture = seed_fixture(db.pool()).await;
    fixture.input.proposals.clear();
    fixture.input.canonical_action_intents.clear();
    let prepared = prepare_investigation_compilation(db.pool(), fixture.input)
        .await
        .expect("typed zero-hypothesis synthesis may prepare an empty canonical set");
    assert!(prepared.input.proposals.is_empty());
    assert!(prepared.input.canonical_action_intents.is_empty());
    assert_eq!(prepared.server_recipe["items"], json!([]));
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM investigation_hypothesis_compilation_decisions")
            .fetch_one(db.pool())
            .await
            .expect("count zero-proposal decisions");
    assert_eq!(count, 0);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn multi_proposal_compiles_an_exact_canonical_and_admission_set() {
    let (mut db, _data_dir) = migrated_db("multi").await;
    let mut fixture = seed_fixture(db.pool()).await;
    add_second_proposal(&mut fixture.input, &fixture.proof);
    for (ordinal, nibble) in (3..=8).zip(['8', '9', 'b', 'c', 'd', 'e']) {
        add_numbered_proposal(&mut fixture.input, &fixture.proof, ordinal, nibble);
    }
    let mut reordered = fixture.input.clone();
    reordered.proposals.reverse();
    reordered.canonical_action_intents.reverse();
    let reordered_prepared = prepare_investigation_compilation(db.pool(), reordered)
        .await
        .expect("prepare the same proposal set in reverse AI presentation order");
    let prepared = prepare_investigation_compilation(db.pool(), fixture.input)
        .await
        .expect("prepare eight exact proposals");
    assert_eq!(
        reordered_prepared.proposal_set_sha256,
        prepared.proposal_set_sha256
    );
    assert_eq!(
        reordered_prepared.action_intent_set_sha256,
        prepared.action_intent_set_sha256
    );
    assert_eq!(
        reordered_prepared.proof_member_set_sha256,
        prepared.proof_member_set_sha256
    );
    assert_eq!(reordered_prepared.server_recipe, prepared.server_recipe);
    assert_eq!(
        reordered_prepared.preparation_sha256,
        prepared.preparation_sha256
    );
    assert_eq!(reordered_prepared.resolved_proofs, prepared.resolved_proofs);
    assert_eq!(prepared.resolved_proofs.len(), 8);
    let view = apply_investigation_compilation(db.pool(), compile_multi_apply_input(prepared))
        .await
        .expect("compile and admit eight canonical hypotheses");
    assert_eq!(view.generation_member_count, 8);
    assert_eq!(view.verification_task_ids.len(), 8);
    for (table, expected) in [
        ("investigation_hypothesis_compilation_members", 8_i64),
        ("investigation_hypothesis_compilation_proof_members", 8),
        ("attack_hypothesis_revisions", 8),
        ("hypothesis_generation_members", 8),
        ("hypothesis_verification_tasks", 8),
        ("verification_admission_members", 8),
        ("hypothesis_verification_task_assignment_sets", 0),
        ("hypothesis_verification_task_campaigns", 0),
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(db.pool())
            .await
            .unwrap_or_else(|error| panic!("count {table}: {error}"));
        assert_eq!(count, expected, "unexpected exact-set count for {table}");
    }
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn live_shaped_negative_proposals_compile_and_admit_without_provider_io() {
    let (mut db, _data_dir) = migrated_db("live-shaped-negative").await;
    let mut fixture = seed_fixture(db.pool()).await;
    let subject_kind = fixture.input.proposals[0].canonical_proposal["subject_kind"]
        .as_str()
        .expect("fixture subject kind")
        .to_owned();
    let subject_identity_hash = fixture.input.proposals[0].canonical_proposal
        ["subject_identity_hash"]
        .as_str()
        .expect("fixture subject identity")
        .to_owned();
    fixture.input.proposals.clear();
    fixture.input.canonical_action_intents.clear();
    for (ordinal, (predicate_schema, predicate_arguments)) in [
        (
            "typed.endpoint.anonymous_access_verdict",
            vec![
                ("technique", "WSTG-ATHN-04"),
                ("observation", "uniform_9_byte_404_across_api_namespace"),
                ("verdict", "inconclusive"),
                ("catch_all_signature", "identical_body_prefix_0019dfc4"),
            ],
        ),
        (
            "typed.endpoint.reachability_verdict",
            vec![
                ("technique", "WSTG-ATHN-04"),
                ("observation", "request_timeout"),
                ("verdict", "transport_inconclusive"),
                (
                    "endpoints",
                    "/license/api/v1/mcdpCert,/license/api/v1/aftermarket",
                ),
            ],
        ),
        (
            "typed.web_origin.technique_coverage_verdict",
            vec![
                ("technique", "WSTG-INFO"),
                ("outcome", "blocked"),
                (
                    "related_techniques",
                    "WSTG-SESS-02,WSTG-ATHN-02,WSTG-INPV-01,WSTG-INPV-05,WSTG-INPV-12",
                ),
                ("cause", "wrapper_failure_and_unsupported"),
            ],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let proposal_id = Uuid::new_v4();
        let predicate_arguments = predicate_arguments
            .into_iter()
            .map(|(key, value)| (key.to_owned(), json!(value)))
            .collect::<serde_json::Map<_, _>>();
        fixture.input.proposals.push(InvestigationProposalInput {
            proposal_id,
            canonical_proposal: json!({
                "proposal_id": proposal_id,
                "subject_kind": subject_kind,
                "subject_identity_hash": subject_identity_hash,
                "predicate_schema": predicate_schema,
                "predicate_version": 1,
                "predicate_arguments": predicate_arguments,
                "trust_boundary": "customer_confidential",
                "polarity": "negative",
                "structured_claim": format!("Captured live-shaped bounded claim {ordinal}"),
                "preconditions": ["A bounded follow-up observation is required"],
                "impact": "The bounded evidence remains inconclusive",
                "proof_refs": [fixture.proof.clone()],
                "knowledge_signals": [],
                "readiness": "ready_for_strategy"
            }),
            proof_refs: vec![fixture.proof.clone()],
        });
        fixture.input.canonical_action_intents.push(json!({
            "intent_id": Uuid::new_v4(),
            "proposal_id": proposal_id,
            "capability": "http_observation",
            "purpose_code": format!("captured_live_follow_up_{ordinal}"),
            "evidence_authority_refs": [fixture.proof.source_hash]
        }));
    }

    let prepared = prepare_investigation_compilation(db.pool(), fixture.input)
        .await
        .expect("prepare captured live-shaped proposal set");
    let applied = apply_investigation_compilation(db.pool(), compile_multi_apply_input(prepared))
        .await
        .expect("compile and admit captured live-shaped proposal set");
    assert_eq!(applied.generation_member_count, 3);
    assert_eq!(applied.verification_task_ids.len(), 3);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn forged_proof_is_rejected_before_any_canonical_write() {
    let (mut db, _data_dir) = migrated_db("forged-proof").await;
    let mut fixture = seed_fixture(db.pool()).await;
    fixture.input.proposals[0].proof_refs[0].source_hash = digest('e');
    fixture.input.proposals[0].canonical_proposal["proof_refs"][0]["source_hash"] =
        Value::String(digest('e'));
    let error = prepare_investigation_compilation(db.pool(), fixture.input)
        .await
        .expect_err("forged proof source hash must fail authority revalidation");
    assert!(
        error
            .to_string()
            .contains("INVESTIGATION_HYPOTHESIS_COMPILER_AUTHORITY_MISMATCH"),
        "unexpected forged-proof error: {error}"
    );
    let counts: (i64, i64, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT COUNT(*) FROM investigation_hypothesis_compilation_decisions),
             (SELECT COUNT(*) FROM attack_hypothesis_revisions),
             (SELECT COUNT(*) FROM hypothesis_verification_tasks)"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("count canonical rows after forged proof");
    assert_eq!(counts, (0, 0, 0));
    db.stop().await;
}
