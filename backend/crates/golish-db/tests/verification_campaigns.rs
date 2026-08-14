use golish_db::{repo, DbConfig, GolishDb};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

const PLAN_C_MIGRATION_VERSION: i64 = 20260729000007;
const SCHEDULER_AUTHORITY_MIGRATION_VERSION: i64 = 20260809000001;

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read local postgres port")
        .port()
}

async fn fixture(label: &str) -> (GolishDb, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("vc_{label}_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start isolated migrated postgres");
    (db, data_dir)
}

async fn relation_exists(pool: &PgPool, relation: &str) -> bool {
    sqlx::query_scalar::<_, bool>("SELECT to_regclass('public.' || $1) IS NOT NULL")
        .bind(relation)
        .fetch_one(pool)
        .await
        .expect("inspect relation")
}

#[test]
fn campaign_repository_modules_are_public_typed_ports() {
    let ports = [
        std::any::type_name::<repo::verification_campaigns::AdmitCampaign>(),
        std::any::type_name::<repo::verification_prepared_actions::PersistPreparedAction>(),
        std::any::type_name::<repo::verification_prepared_actions::SealBudgetContract>(),
        std::any::type_name::<
            repo::verification_prepared_actions::PreparedActionAuthorizationDecision,
        >(),
        std::any::type_name::<repo::verification_oracles::RecordActionOracle>(),
        std::any::type_name::<repo::verification_fact_delta_bundles::CloseCampaignObjective>(),
        std::any::type_name::<repo::verification_campaign_coverage::SealWaveCoverageDenominator>(),
        std::any::type_name::<repo::verification_capability_assessments::RecordCapabilityAssessment>(
        ),
        std::any::type_name::<repo::hypothesis_objective_outcomes::SealObjectiveOutcomeSet>(),
        std::any::type_name::<repo::hypothesis_revision_adjudications::AdjudicateRevision>(),
        std::any::type_name::<repo::hypothesis_consolidations::RecordFactDeltaConsumption>(),
    ];
    assert!(ports.iter().all(|port| port.contains("golish_db::repo")));
}

#[tokio::test]
#[serial]
async fn plan_c_migration_installs_the_complete_authority_spine() {
    let (db, _data_dir) = fixture("relations").await;
    let installed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM _sqlx_migrations WHERE version=$1 AND success)",
    )
    .bind(PLAN_C_MIGRATION_VERSION)
    .fetch_one(db.pool())
    .await
    .expect("read migration ledger");
    assert!(installed);

    for relation in [
        "verification_campaigns",
        "verification_campaign_rounds",
        "verification_consults",
        "verification_strategy_artifacts",
        "verification_strategy_obligations",
        "verification_prepared_actions",
        "verification_prepared_action_group_members",
        "verification_prepared_action_authorizations",
        "verification_action_executions",
        "verification_action_subexecutions",
        "verification_action_conflict_sets",
        "verification_action_conflict_set_members",
        "verification_conflict_key_heads",
        "verification_conflict_key_events",
        "verification_budget_contracts",
        "verification_budget_contract_axes",
        "verification_budget_scope_heads",
        "verification_budget_reservations",
        "verification_budget_ledger_entries",
        "verification_cleanup_obligations",
        "verification_callback_obligations",
        "verification_oracle_assessments",
        "verification_oracle_census_seals",
        "verification_oracle_census_members",
        "verification_campaign_adjudications",
        "verification_campaign_terminal_decisions",
        "hypothesis_objective_outcome_receipts",
        "hypothesis_objective_claim_component_outcome_seals",
        "hypothesis_objective_claim_component_outcome_members",
        "hypothesis_objective_outcome_heads",
        "hypothesis_objective_outcome_set_seals",
        "hypothesis_objective_outcome_set_members",
        "hypothesis_revision_adjudications",
        "hypothesis_revision_terminal_decisions",
        "verification_fact_delta_bundles",
        "fact_delta_consumptions",
        "hypothesis_evolution_proposals",
        "hypothesis_evolution_decisions",
        "hypothesis_consolidation_batches",
        "hypothesis_consolidation_receipts",
        "hypothesis_fixed_point_receipts",
        "enrichment_obligations",
        "application_fact_refinement_obligations",
        "verification_wave_coverage_denominators",
        "verification_wave_coverage_members",
        "verification_campaign_coverage_denominators",
        "verification_campaign_coverage_members",
        "verification_campaign_coverage_results",
        "verification_campaign_coverage_receipts",
        "verification_wave_coverage_receipts",
        "verification_wave_unassigned_coverage_results",
        "verification_campaign_shadow_evaluations",
        "verification_campaign_shadow_evaluation_items",
        "verification_capability_assessments",
        "verification_capability_assessment_set_seals",
        "verification_capability_assessment_set_members",
        "verification_authority_quarantine_events",
        "verification_authority_quarantine_members",
        "verification_authority_temporal_staleness_events",
        "hypothesis_re_adjudication_obligations",
        "verification_authority_correction_bundles",
        "verification_authority_correction_consumptions",
        "verification_campaign_safety_holds",
        "investigation_verification_task_advisory_receipts",
        "investigation_verification_task_advisory_members",
        "investigation_verification_advisory_campaign_applies",
        "investigation_verification_task_advisory_seals",
    ] {
        assert!(
            relation_exists(db.pool(), relation).await,
            "missing {relation}"
        );
    }
}

#[tokio::test]
#[serial]
async fn terminal_action_requires_its_durable_oracle_landing() {
    let (db, _data_dir) = fixture("oracle-commit-marker").await;
    let execution_id = Uuid::new_v4();
    let prepared_action_id = Uuid::new_v4();
    let capability_receipt_id = Uuid::new_v4();
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin compact action fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("isolate compact action fixture");
    sqlx::query(
        r#"INSERT INTO verification_action_executions(
               action_execution_id,stable_request_id,prepared_action_id,
               authorization_receipt_id,budget_reservation_id,conflict_set_id,
               operation_id,project_scope_id,organization_id,execution_ordinal,
               execution_kind,state,campaign_dispatch_generation,durable_begin_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,1,'single_action_v1','started',0,$10)"#,
    )
    .bind(execution_id)
    .bind(Uuid::new_v4())
    .bind(prepared_action_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(format!("sha256:{}", "a".repeat(64)))
    .execute(&mut *tx)
    .await
    .expect("seed started action execution");
    tx.commit().await.expect("commit compact action fixture");

    let error = sqlx::query(
        r#"UPDATE verification_action_executions
              SET state='succeeded',capability_execution_receipt_id=$1,
                  closeout_hash=$2,completed_at=statement_timestamp(),row_version=row_version+1
            WHERE action_execution_id=$3"#,
    )
    .bind(capability_receipt_id)
    .bind(format!("sha256:{}", "b".repeat(64)))
    .bind(execution_id)
    .execute(db.pool())
    .await
    .expect_err("terminal action without Oracle must fail closed");
    assert!(
        error
            .to_string()
            .contains("VERIFICATION_ACTION_ORACLE_LANDING_REQUIRED"),
        "unexpected commit-marker error: {error}"
    );
    let state: String = sqlx::query_scalar(
        "SELECT state FROM verification_action_executions WHERE action_execution_id=$1",
    )
    .bind(execution_id)
    .fetch_one(db.pool())
    .await
    .expect("load action state after rejected terminalization");
    assert_eq!(state, "started");
}

#[tokio::test]
#[serial]
async fn terminal_action_accepts_a_complete_proof_oracle_landing() {
    let (db, _data_dir) = fixture("oracle-proof-commit-marker").await;
    let execution_id = Uuid::new_v4();
    let prepared_action_id = Uuid::new_v4();
    let capability_receipt_id = Uuid::new_v4();
    let operation_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let receipt_authority_hash = format!("sha256:{}", "c".repeat(64));
    let mut tx = db.pool().begin().await.expect("begin proof marker fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("isolate proof marker fixture");
    sqlx::query(
        r#"INSERT INTO capability_execution_receipts(
               id,denominator_id,execution_authority_id,capability,attempt_ordinal,
               receipt_authority_hash,input_manifest_hash,destination_policy_id,
               destination_policy_hash,temporal_validity_policy_id,
               temporal_validity_policy_hash,attempt_state,landing_state,
               observation_state,coverage_extent,coverage_gap_reason,
               reconciliation_state,security_interpretation,typed_landing,
               raw_witness_artifact_id,parser_census_id,temporal_census_id,
               observation_completed_at,valid_until,finalized_at)
           VALUES($1,$2,$3,'fixture_http_get',1,$4,$5,$6,$7,$8,$9,
                  'succeeded','committed','found','complete','none','consistent','signal',
                  $10,$11,$12,$13,statement_timestamp(),statement_timestamp()+INTERVAL '60 seconds',
                  statement_timestamp())"#,
    )
    .bind(capability_receipt_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(&receipt_authority_hash)
    .bind(format!("sha256:{}", "d".repeat(64)))
    .bind(Uuid::new_v4())
    .bind(format!("sha256:{}", "e".repeat(64)))
    .bind(Uuid::new_v4())
    .bind(format!("sha256:{}", "f".repeat(64)))
    .bind(serde_json::json!({"contract_version": "fixture.v1"}))
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .execute(&mut *tx)
    .await
    .expect("seed finalized capability receipt");
    sqlx::query(
        r#"INSERT INTO verification_action_executions(
               action_execution_id,stable_request_id,prepared_action_id,
               authorization_receipt_id,budget_reservation_id,conflict_set_id,
               operation_id,project_scope_id,organization_id,execution_ordinal,
               execution_kind,state,campaign_dispatch_generation,durable_begin_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,1,'single_action_v1','started',0,$10)"#,
    )
    .bind(execution_id)
    .bind(Uuid::new_v4())
    .bind(prepared_action_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(format!("sha256:{}", "a".repeat(64)))
    .execute(&mut *tx)
    .await
    .expect("seed started proof action");
    sqlx::query(
        r#"INSERT INTO verification_oracle_assessments(
               oracle_assessment_id,stable_request_id,campaign_id,prepared_action_id,
               action_execution_id,campaign_coverage_member_id,operation_id,project_scope_id,
               organization_id,oracle_revision_ordinal,oracle_contract_version,
               oracle_contract_hash,observation_receipt_hash,precondition_validity,
               control_validity,verdict,assessment_body,assessment_hash,residual_id)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,1,'fixture-oracle.v1',$10,$11,
                  'valid','not_required','proof',$12,$13,NULL)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(prepared_action_id)
    .bind(execution_id)
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(format!("sha256:{}", "1".repeat(64)))
    .bind(&receipt_authority_hash)
    .bind(serde_json::json!({"verdict": "proof"}))
    .bind(format!("sha256:{}", "2".repeat(64)))
    .execute(&mut *tx)
    .await
    .expect("seed exact proof Oracle");
    tx.commit().await.expect("commit proof marker fixture");

    sqlx::query(
        r#"UPDATE verification_action_executions
              SET state='succeeded',capability_execution_receipt_id=$1,
                  closeout_hash=$2,completed_at=statement_timestamp(),row_version=row_version+1
            WHERE action_execution_id=$3"#,
    )
    .bind(capability_receipt_id)
    .bind(format!("sha256:{}", "b".repeat(64)))
    .bind(execution_id)
    .execute(db.pool())
    .await
    .expect("typed proof Oracle must satisfy the generic semantic commit marker");
    let state: String = sqlx::query_scalar(
        "SELECT state FROM verification_action_executions WHERE action_execution_id=$1",
    )
    .bind(execution_id)
    .fetch_one(db.pool())
    .await
    .expect("load proof action state");
    assert_eq!(state, "succeeded");
}

#[tokio::test]
#[serial]
async fn semantic_landing_repairs_a_finalized_receipt_and_replays_exactly() {
    use repo::verification_prepared_actions::{
        finalize_verification_action_semantic_landing, FinalizeVerificationActionSemanticLanding,
        VerificationActionSemanticLanding,
    };

    let (db, _data_dir) = fixture("semantic-landing-repair").await;
    let stable_request_id = Uuid::new_v4();
    let operation_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let campaign_id = Uuid::new_v4();
    let prepared_action_id = Uuid::new_v4();
    let authorization_receipt_id = Uuid::new_v4();
    let action_execution_id = Uuid::new_v4();
    let capability_receipt_id = Uuid::new_v4();
    let budget_reservation_id = Uuid::new_v4();
    let conflict_set_id = Uuid::new_v4();
    let denominator_id = Uuid::new_v4();
    let coverage_member_id = Uuid::new_v4();
    let coverage_member_hash = format!("sha256:{}", "3".repeat(64));
    let binding_id = Uuid::new_v4();
    let receipt_authority_hash = format!("sha256:{}", "4".repeat(64));
    let oracle_contract_hash = format!("sha256:{}", "5".repeat(64));
    let residual_id = Uuid::new_v5(
        &stable_request_id,
        b"verification-action-raw-witness-incomplete.v1",
    );
    let observation = serde_json::json!({
        "contract_version": "verification-action-observation.v1",
        "witness_completeness": "metadata_only",
        "recovery_disposition": "durable_begin_without_terminal_receipt",
    });
    let observation_hash: String =
        sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
            .bind(&observation)
            .fetch_one(db.pool())
            .await
            .expect("hash recovery observation");
    let typed_landing = serde_json::json!({
        "contract_version": "verification-action-observation.v1",
        "witness_completeness": "metadata_only",
        "observation_hash": observation_hash,
        "observation": observation,
    });
    let finalization_request_id = Uuid::new_v5(
        &stable_request_id,
        b"verification-action-capability-receipt-finalize.v1",
    );
    let finalization_hash: String =
        sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
            .bind(serde_json::json!({
                "binding_id": binding_id,
                "action_execution_id": action_execution_id,
                "prepared_action_id": prepared_action_id,
                "capability_execution_receipt_id": capability_receipt_id,
                "terminal_state": "outcome_unknown",
                "witness_completeness": "metadata_only",
                "observation_hash": observation_hash,
            }))
            .fetch_one(db.pool())
            .await
            .expect("hash recovery finalization");
    let oracle_body = serde_json::json!({
        "contract_version": "verification-action-oracle-assessment.v1",
        "witness_completeness": "metadata_only",
        "reason_code": "raw_witness_incomplete",
        "typed_landing": typed_landing,
    });
    let oracle_assessment_hash: String =
        sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
            .bind(&oracle_body)
            .fetch_one(db.pool())
            .await
            .expect("hash recovery Oracle");
    let oracle_stable_request_id = Uuid::new_v5(
        &stable_request_id,
        b"verification-action-inconclusive-oracle.v1",
    );
    let oracle_assessment_id =
        Uuid::new_v5(&oracle_stable_request_id, b"verification-action-oracle.v1");

    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin semantic repair fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("isolate semantic repair fixture");
    sqlx::query(
        "INSERT INTO organizations(id,project_path,name) VALUES($1,'/tmp/semantic-repair','Semantic Repair Org')",
    )
    .bind(organization_id)
    .execute(&mut *tx)
    .await
    .expect("seed semantic repair organization");
    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,project_scope_id,
               enumeration_analysis_contract,stage_topology_contract,
               stage_topology_canonical_json,stage_topology_sha256,
               stage_topology_freeze_source,investigation_contract_version,
               investigation_rollout_mode,tool_truth_contract)
           VALUES($1,'red_team','investigation','v2_only',$2,'legacy_v1',
                  'unified_investigation_v1',
                  stage_topology_canonical_json('unified_investigation_v1'),
                  stage_topology_contract_sha256('unified_investigation_v1'),
                  'deployment_pair_v1','hypothesis_registry_v1','new_only','receipt_v1')"#,
    )
    .bind(operation_id)
    .bind(project_scope_id)
    .execute(&mut *tx)
    .await
    .expect("seed semantic repair operation");
    sqlx::query(
        r#"INSERT INTO verification_prepared_actions(
               prepared_action_id,stable_request_id,campaign_id,round_id,strategy_artifact_id,
               operation_id,project_scope_id,organization_id,capability_assessment_id,
               action_ordinal,action_contract_kind,action_kind,canonical_request_hash,
               display_projection,display_projection_hash,renderer_version,private_manifest,
               private_manifest_hash,review_expires_at,target_type_at_time,target_value_at_time,
               target_identity_hash,policy_snapshot_hash,upper_budget_set_hash,
               oracle_contract_hash,risk_tier,state)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,0,'single_action_v1','fixture_http_get',$10,
                  '{}'::JSONB,$11,'fixture.v1',$12,$13,NOW()+INTERVAL '1 hour','url',
                  'https://fixture.invalid/',$14,$15,$16,$17,'T0','started')"#,
    )
    .bind(prepared_action_id)
    .bind(Uuid::new_v4())
    .bind(campaign_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(Uuid::new_v4())
    .bind(format!("sha256:{}", "6".repeat(64)))
    .bind(format!("sha256:{}", "7".repeat(64)))
    .bind(serde_json::json!({
        "oracle_contract_version": "verification-action-oracle.v1",
        "coverage_member_hash": coverage_member_hash,
    }))
    .bind(format!("sha256:{}", "8".repeat(64)))
    .bind(format!("sha256:{}", "9".repeat(64)))
    .bind(format!("sha256:{}", "a".repeat(64)))
    .bind(format!("sha256:{}", "b".repeat(64)))
    .bind(&oracle_contract_hash)
    .execute(&mut *tx)
    .await
    .expect("seed started prepared action");
    sqlx::query(
        r#"INSERT INTO verification_budget_reservations(
               budget_reservation_id,stable_request_id,prepared_action_id,
               authorization_receipt_id,operation_id,project_scope_id,organization_id,
               contract_set_hash,upper_bound_membership_hash,state)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'active')"#,
    )
    .bind(budget_reservation_id)
    .bind(Uuid::new_v4())
    .bind(prepared_action_id)
    .bind(authorization_receipt_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(format!("sha256:{}", "c".repeat(64)))
    .bind(format!("sha256:{}", "d".repeat(64)))
    .execute(&mut *tx)
    .await
    .expect("seed active budget reservation");
    sqlx::query(
        r#"INSERT INTO verification_action_executions(
               action_execution_id,stable_request_id,prepared_action_id,
               authorization_receipt_id,budget_reservation_id,conflict_set_id,
               operation_id,project_scope_id,organization_id,execution_ordinal,
               execution_kind,state,campaign_dispatch_generation,durable_begin_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,1,'single_action_v1','started',0,$10)"#,
    )
    .bind(action_execution_id)
    .bind(Uuid::new_v4())
    .bind(prepared_action_id)
    .bind(authorization_receipt_id)
    .bind(budget_reservation_id)
    .bind(conflict_set_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(format!("sha256:{}", "e".repeat(64)))
    .execute(&mut *tx)
    .await
    .expect("seed started action execution");
    sqlx::query(
        r#"INSERT INTO verification_campaign_coverage_denominators(
               campaign_denominator_id,stable_request_id,operation_id,project_scope_id,
               organization_id,campaign_id,hypothesis_revision_id,wave_denominator_id,
               contract_version,source_snapshot_hash,member_set_hash,member_count,sealed_at)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,'fixture.v1',$9,$10,1,NOW())"#,
    )
    .bind(denominator_id)
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(campaign_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(format!("sha256:{}", "f".repeat(64)))
    .bind(format!("sha256:{}", "1".repeat(64)))
    .execute(&mut *tx)
    .await
    .expect("seed campaign denominator");
    sqlx::query(
        r#"INSERT INTO verification_campaign_coverage_members(
               campaign_coverage_member_id,campaign_denominator_id,wave_coverage_member_id,
               wave_denominator_id,operation_id,project_scope_id,organization_id,
               member_ordinal,semantic_key,claim_component_id,claim_component_hash,
               obligation_kind,control_binding_kind,capability_assessment_id,
               expected_capability_kind,expected_action_kind,expected_oracle_kind,member_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,0,'fixture-member',$8,$9,'predicate',
                  'explicit_no_control',$10,'fixture_http_get','fixture_http_get',
                  'verification-action-oracle.v1',$11)"#,
    )
    .bind(coverage_member_id)
    .bind(denominator_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(Uuid::new_v4())
    .bind(format!("sha256:{}", "2".repeat(64)))
    .bind(Uuid::new_v4())
    .bind(&coverage_member_hash)
    .execute(&mut *tx)
    .await
    .expect("seed campaign coverage member");
    sqlx::query(
        r#"INSERT INTO capability_execution_receipts(
               id,denominator_id,execution_authority_id,capability,attempt_ordinal,
               receipt_authority_hash,input_manifest_hash,destination_policy_id,
               destination_policy_hash,temporal_validity_policy_id,
               temporal_validity_policy_hash,attempt_state,landing_state,
               observation_state,coverage_extent,coverage_gap_reason,
               reconciliation_state,security_interpretation,typed_landing,
               finalization_request_hash,observation_completed_at,valid_until,finalized_at)
           VALUES($1,$2,$3,'fixture_http_get',1,$4,$5,$6,$7,$8,$9,
                  'outcome_unknown','failed','indeterminate','partial','transport',
                  'consistent','inconclusive',$10,$11,NOW(),NOW()+INTERVAL '60 seconds',NOW())"#,
    )
    .bind(capability_receipt_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(&receipt_authority_hash)
    .bind(format!("sha256:{}", "3".repeat(64)))
    .bind(Uuid::new_v4())
    .bind(format!("sha256:{}", "4".repeat(64)))
    .bind(Uuid::new_v4())
    .bind(format!("sha256:{}", "5".repeat(64)))
    .bind(&typed_landing)
    .bind(&finalization_hash)
    .execute(&mut *tx)
    .await
    .expect("seed response-loss finalized receipt");
    sqlx::query(
        r#"INSERT INTO verification_action_capability_receipt_bindings(
               binding_id,stable_request_id,action_execution_id,prepared_action_id,campaign_id,
               operation_id,project_scope_id,organization_id,capability_execution_receipt_id,
               derived_denominator_id,parent_denominator_id,parent_denominator_item_id,
               execution_authority_id,binding_hash)
           SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,receipt.denominator_id,$10,$11,
                  receipt.execution_authority_id,$12
             FROM capability_execution_receipts receipt WHERE receipt.id=$9"#,
    )
    .bind(binding_id)
    .bind(Uuid::new_v4())
    .bind(action_execution_id)
    .bind(prepared_action_id)
    .bind(campaign_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(capability_receipt_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(format!("sha256:{}", "6".repeat(64)))
    .execute(&mut *tx)
    .await
    .expect("seed action receipt binding");
    sqlx::query(
        r#"INSERT INTO verification_action_capability_receipt_finalizations(
               finalization_id,stable_request_id,binding_id,action_execution_id,
               prepared_action_id,capability_execution_receipt_id,terminal_state,
               witness_completeness,observation_hash,finalization_hash)
           VALUES($1,$2,$3,$4,$5,$6,'outcome_unknown','metadata_only',$7,$8)"#,
    )
    .bind(Uuid::new_v4())
    .bind(finalization_request_id)
    .bind(binding_id)
    .bind(action_execution_id)
    .bind(prepared_action_id)
    .bind(capability_receipt_id)
    .bind(&observation_hash)
    .bind(&finalization_hash)
    .execute(&mut *tx)
    .await
    .expect("seed finalized receipt marker");
    sqlx::query(
        r#"INSERT INTO verification_oracle_assessments(
               oracle_assessment_id,stable_request_id,campaign_id,prepared_action_id,
               action_execution_id,campaign_coverage_member_id,operation_id,project_scope_id,
               organization_id,oracle_revision_ordinal,oracle_contract_version,
               oracle_contract_hash,observation_receipt_hash,precondition_validity,
               control_validity,verdict,assessment_body,assessment_hash,residual_id)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,1,'verification-action-oracle.v1',$10,$11,
                  'unknown','not_required','inconclusive',$12,$13,$14)"#,
    )
    .bind(oracle_assessment_id)
    .bind(oracle_stable_request_id)
    .bind(campaign_id)
    .bind(prepared_action_id)
    .bind(action_execution_id)
    .bind(coverage_member_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(&oracle_contract_hash)
    .bind(&receipt_authority_hash)
    .bind(&oracle_body)
    .bind(&oracle_assessment_hash)
    .bind(residual_id)
    .execute(&mut *tx)
    .await
    .expect("seed response-loss Oracle before residual/action closeout");
    tx.commit().await.expect("commit semantic repair fixture");

    let command = FinalizeVerificationActionSemanticLanding {
        stable_request_id,
        operation_id,
        campaign_id,
        prepared_action_id,
        authorization_receipt_id,
        action_execution_id,
        capability_execution_receipt_id: capability_receipt_id,
        terminal_state: "outcome_unknown".to_owned(),
        observation,
    };
    let repaired = finalize_verification_action_semantic_landing(db.pool(), &command)
        .await
        .expect("repair post-receipt response loss without sending again");
    assert_eq!(repaired.oracle_assessment_id, oracle_assessment_id);
    assert_eq!(repaired.residual_id, Some(residual_id));
    assert!(!repaired.replayed);
    let replay = finalize_verification_action_semantic_landing(db.pool(), &command)
        .await
        .expect("replay exact semantic landing");
    assert!(replay.replayed);
    assert_eq!(
        replay,
        VerificationActionSemanticLanding {
            replayed: true,
            ..repaired.clone()
        }
    );
    let exact: (String, String, i64, i64, i64) = sqlx::query_as(
        r#"SELECT execution.state,action.state,
                  (SELECT COUNT(*) FROM hypothesis_residual_risks WHERE residual_id=$2),
                  (SELECT COUNT(*) FROM verification_oracle_assessments WHERE oracle_assessment_id=$3),
                  (SELECT COUNT(*) FROM verification_conflict_key_events WHERE owner_prepared_action_id=$4)
             FROM verification_action_executions execution
             JOIN verification_prepared_actions action
               ON action.prepared_action_id=execution.prepared_action_id
            WHERE execution.action_execution_id=$1"#,
    )
    .bind(action_execution_id)
    .bind(residual_id)
    .bind(oracle_assessment_id)
    .bind(prepared_action_id)
    .fetch_one(db.pool())
    .await
    .expect("load repaired semantic landing exact set");
    assert_eq!(
        exact,
        ("outcome_unknown".into(), "outcome_unknown".into(), 1, 1, 0)
    );
}

#[tokio::test]
#[serial]
async fn directory_fingerprint_witness_v1_is_a_distinct_non_raw_finalization_kind() {
    let (db, _data_dir) = fixture("directory-fingerprint-witness-kind").await;
    let mut tx = db.pool().begin().await.expect("begin witness-kind fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("disable foreign-key triggers for isolated check-constraint fixture");
    sqlx::query(
        r#"INSERT INTO verification_action_capability_receipt_finalizations(
               finalization_id,stable_request_id,binding_id,action_execution_id,
               prepared_action_id,capability_execution_receipt_id,terminal_state,
               witness_completeness,observation_hash,finalization_hash)
           VALUES($1,$2,$3,$4,$5,$6,'succeeded','complete_fingerprint_v1',$7,$8)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(format!("sha256:{}", "a".repeat(64)))
    .bind(format!("sha256:{}", "b".repeat(64)))
    .execute(&mut *tx)
    .await
    .expect("expanded witness constraint accepts the versioned fingerprint kind");
    tx.commit().await.expect("commit witness-kind fixture");
}

#[tokio::test]
#[serial]
#[ignore = "historical isolated replica fixture does not seed the complete typed Tool Truth authority; the runnable assignment/send path has been removed"]
async fn directory_fingerprint_complete_witness_lands_a_recomputed_proof_oracle() {
    use repo::verification_prepared_actions::{
        finalize_verification_action_semantic_landing, FinalizeVerificationActionSemanticLanding,
    };

    let (db, _data_dir) = fixture("directory-fingerprint-proof-oracle").await;
    let stable_request_id = Uuid::new_v4();
    let operation_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let campaign_id = Uuid::new_v4();
    let prepared_action_id = Uuid::new_v4();
    let authorization_receipt_id = Uuid::new_v4();
    let action_execution_id = Uuid::new_v4();
    let capability_receipt_id = Uuid::new_v4();
    let budget_reservation_id = Uuid::new_v4();
    let conflict_set_id = Uuid::new_v4();
    let denominator_id = Uuid::new_v4();
    let wave_denominator_id = Uuid::new_v4();
    let wave_member_id = Uuid::new_v4();
    let coverage_member_id = Uuid::new_v4();
    let capability_assessment_id = Uuid::new_v4();
    let coverage_member_hash = format!("sha256:{}", "3".repeat(64));
    let receipt_authority_hash = format!("sha256:{}", "4".repeat(64));
    let oracle_contract_hash = format!("sha256:{}", "5".repeat(64));
    let binding_id = Uuid::new_v4();
    let nonce = prepared_action_id.simple().to_string();
    let http_observation = |url: String, body_byte: char| {
        serde_json::json!({
            "final_url": url,
            "hops": [{
                "url": url,
                "status": 200,
                "response_bytes": 8,
                "body_sha256": format!("sha256:{}", body_byte.to_string().repeat(64)),
                "content_type": "text/html",
            }],
        })
    };
    let observation = serde_json::json!({
        "assessment": {"controls_consistent": true, "verdict": "verified"},
        "candidate": http_observation("https://example.test/admin".to_owned(), 'a'),
        "capability_id": "verify.directory_fingerprint.v1",
        "contract_version": "directory-soft404-fingerprint-observation.v1",
        "controls": (1..=3).map(|ordinal| http_observation(
            format!("https://example.test/.golish-soft404-{nonce}-{ordinal}"),
            'b',
        )).collect::<Vec<_>>(),
        "request_count": 4,
        "witness_completeness": "complete_fingerprint_v1",
    });

    let mut tx = db.pool().begin().await.expect("begin proof Oracle fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("isolate proof Oracle fixture authority rows");
    sqlx::query(
        r#"INSERT INTO verification_prepared_actions(
               prepared_action_id,stable_request_id,campaign_id,round_id,strategy_artifact_id,
               operation_id,project_scope_id,organization_id,capability_assessment_id,
               action_ordinal,action_contract_kind,action_kind,canonical_request_hash,
               display_projection,display_projection_hash,renderer_version,private_manifest,
               private_manifest_hash,review_expires_at,target_type_at_time,target_value_at_time,
               target_identity_hash,policy_snapshot_hash,upper_budget_set_hash,
               oracle_contract_hash,risk_tier,state)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,0,'single_action_v1',
                  'verify.directory_fingerprint.v1',$10,'{}'::JSONB,$11,'fixture.v1',$12,$13,
                  NOW()+INTERVAL '1 hour','url','https://example.test/admin',$14,$15,$16,$17,
                  'T1','started')"#,
    )
    .bind(prepared_action_id)
    .bind(Uuid::new_v4())
    .bind(campaign_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(capability_assessment_id)
    .bind(format!("sha256:{}", "6".repeat(64)))
    .bind(format!("sha256:{}", "7".repeat(64)))
    .bind(serde_json::json!({
        "oracle_contract_version": "verification-action-oracle.v1",
        "coverage_member_hash": coverage_member_hash,
    }))
    .bind(format!("sha256:{}", "8".repeat(64)))
    .bind(format!("sha256:{}", "9".repeat(64)))
    .bind(format!("sha256:{}", "a".repeat(64)))
    .bind(format!("sha256:{}", "b".repeat(64)))
    .bind(&oracle_contract_hash)
    .execute(&mut *tx)
    .await
    .expect("seed directory prepared action");
    sqlx::query(
        r#"INSERT INTO verification_budget_reservations(
               budget_reservation_id,stable_request_id,prepared_action_id,
               authorization_receipt_id,operation_id,project_scope_id,organization_id,
               contract_set_hash,upper_bound_membership_hash,state)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'active')"#,
    )
    .bind(budget_reservation_id)
    .bind(Uuid::new_v4())
    .bind(prepared_action_id)
    .bind(authorization_receipt_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(format!("sha256:{}", "c".repeat(64)))
    .bind(format!("sha256:{}", "d".repeat(64)))
    .execute(&mut *tx)
    .await
    .expect("seed directory budget reservation");
    sqlx::query(
        r#"INSERT INTO verification_action_executions(
               action_execution_id,stable_request_id,prepared_action_id,
               authorization_receipt_id,budget_reservation_id,conflict_set_id,
               operation_id,project_scope_id,organization_id,execution_ordinal,
               execution_kind,state,campaign_dispatch_generation,durable_begin_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,1,'single_action_v1','started',0,$10)"#,
    )
    .bind(action_execution_id)
    .bind(Uuid::new_v4())
    .bind(prepared_action_id)
    .bind(authorization_receipt_id)
    .bind(budget_reservation_id)
    .bind(conflict_set_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(format!("sha256:{}", "e".repeat(64)))
    .execute(&mut *tx)
    .await
    .expect("seed directory execution");
    sqlx::query(
        r#"INSERT INTO verification_campaign_coverage_denominators(
               campaign_denominator_id,stable_request_id,operation_id,project_scope_id,
               organization_id,campaign_id,hypothesis_revision_id,wave_denominator_id,
               contract_version,source_snapshot_hash,member_set_hash,member_count,sealed_at)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,'fixture.v1',$9,$10,1,NOW())"#,
    )
    .bind(denominator_id)
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(campaign_id)
    .bind(Uuid::new_v4())
    .bind(wave_denominator_id)
    .bind(format!("sha256:{}", "f".repeat(64)))
    .bind(format!("sha256:{}", "1".repeat(64)))
    .execute(&mut *tx)
    .await
    .expect("seed directory campaign denominator");
    sqlx::query(
        r#"INSERT INTO verification_wave_coverage_members(
               wave_coverage_member_id,wave_denominator_id,operation_id,project_scope_id,
               organization_id,member_ordinal,semantic_key,input_ref_kind,input_ref_id,
               input_identity_hash,hypothesis_revision_id,claim_component_id,
               claim_component_hash,verification_objective_id,predicate_component_id,
               control_binding_kind,no_control_marker_hash,capability_assessment_id,
               expected_capability_kind,expected_action_kind,expected_oracle_kind,member_hash)
           VALUES($1,$2,$3,$4,$5,0,'directory-fixture','fixture',$6,$7,$8,$9,$10,$11,$12,
                  'explicit_no_control',$13,$14,'verify.directory_fingerprint.v1',
                  'trusted_http_directory_fingerprint.v1','directory_soft404_fingerprint.v1',$15)"#,
    )
    .bind(wave_member_id)
    .bind(wave_denominator_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(Uuid::new_v4())
    .bind(format!("sha256:{}", "2".repeat(64)))
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(format!("sha256:{}", "3".repeat(64)))
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(format!("sha256:{}", "4".repeat(64)))
    .bind(capability_assessment_id)
    .bind(format!("sha256:{}", "5".repeat(64)))
    .execute(&mut *tx)
    .await
    .expect("seed directory wave member");
    sqlx::query(
        r#"INSERT INTO verification_campaign_coverage_members(
               campaign_coverage_member_id,campaign_denominator_id,wave_coverage_member_id,
               wave_denominator_id,operation_id,project_scope_id,organization_id,
               member_ordinal,semantic_key,claim_component_id,claim_component_hash,
               obligation_kind,control_binding_kind,capability_assessment_id,
               expected_capability_kind,expected_action_kind,expected_oracle_kind,member_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,0,'directory-fixture',$8,$9,'predicate',
                  'explicit_no_control',$10,'verify.directory_fingerprint.v1',
                  'trusted_http_directory_fingerprint.v1','directory_soft404_fingerprint.v1',$11)"#,
    )
    .bind(coverage_member_id)
    .bind(denominator_id)
    .bind(wave_member_id)
    .bind(wave_denominator_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(Uuid::new_v4())
    .bind(format!("sha256:{}", "6".repeat(64)))
    .bind(capability_assessment_id)
    .bind(&coverage_member_hash)
    .execute(&mut *tx)
    .await
    .expect("seed directory campaign member");
    sqlx::query(
        r#"INSERT INTO capability_execution_receipts(
               id,denominator_id,execution_authority_id,capability,attempt_ordinal,
               receipt_authority_hash,input_manifest_hash,destination_policy_id,
               destination_policy_hash,temporal_validity_policy_id,
               temporal_validity_policy_hash,attempt_state,landing_state,
               observation_state,coverage_extent,coverage_gap_reason,
               reconciliation_state,security_interpretation,typed_landing)
           VALUES($1,$2,$3,'verify.directory_fingerprint.v1',1,$4,$5,$6,$7,$8,$9,
                  'running','not_attempted','indeterminate','none','none',
                  'pending','not_assessed','{}'::JSONB)"#,
    )
    .bind(capability_receipt_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(&receipt_authority_hash)
    .bind(format!("sha256:{}", "7".repeat(64)))
    .bind(Uuid::new_v4())
    .bind(format!("sha256:{}", "8".repeat(64)))
    .bind(Uuid::new_v4())
    .bind(format!("sha256:{}", "9".repeat(64)))
    .execute(&mut *tx)
    .await
    .expect("seed running directory receipt");
    sqlx::query(
        r#"INSERT INTO verification_action_capability_receipt_bindings(
               binding_id,stable_request_id,action_execution_id,prepared_action_id,campaign_id,
               operation_id,project_scope_id,organization_id,capability_execution_receipt_id,
               derived_denominator_id,parent_denominator_id,parent_denominator_item_id,
               execution_authority_id,binding_hash)
           SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,receipt.denominator_id,$10,$11,
                  receipt.execution_authority_id,$12
             FROM capability_execution_receipts receipt WHERE receipt.id=$9"#,
    )
    .bind(binding_id)
    .bind(Uuid::new_v4())
    .bind(action_execution_id)
    .bind(prepared_action_id)
    .bind(campaign_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(capability_receipt_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(format!("sha256:{}", "a".repeat(64)))
    .execute(&mut *tx)
    .await
    .expect("seed directory receipt binding");
    tx.commit().await.expect("commit proof Oracle fixture");

    let landing = finalize_verification_action_semantic_landing(
        db.pool(),
        &FinalizeVerificationActionSemanticLanding {
            stable_request_id,
            operation_id,
            campaign_id,
            prepared_action_id,
            authorization_receipt_id,
            action_execution_id,
            capability_execution_receipt_id: capability_receipt_id,
            terminal_state: "succeeded".to_owned(),
            observation,
        },
    )
    .await
    .expect("complete directory witness lands a deterministic proof");
    assert_eq!(landing.residual_id, None);
    let exact: (String, String, String, String, String, i64) = sqlx::query_as(
        r#"SELECT oracle.verdict,oracle.precondition_validity,oracle.control_validity,
                  receipt.landing_state,receipt.coverage_extent,
                  (SELECT COUNT(*) FROM hypothesis_residual_risks
                    WHERE operation_id=$2 AND organization_id=$3)
             FROM verification_oracle_assessments oracle
             JOIN capability_execution_receipts receipt
               ON receipt.id=$4
            WHERE oracle.oracle_assessment_id=$1"#,
    )
    .bind(landing.oracle_assessment_id)
    .bind(operation_id)
    .bind(organization_id)
    .bind(capability_receipt_id)
    .fetch_one(db.pool())
    .await
    .expect("read exact proof landing");
    assert_eq!(
        exact,
        (
            "proof".to_owned(),
            "valid".to_owned(),
            "not_required".to_owned(),
            "committed".to_owned(),
            "sampled".to_owned(),
            0,
        )
    );
}

#[tokio::test]
#[serial]
async fn plan_b_authorities_are_reused_instead_of_redeclared() {
    let (db, _data_dir) = fixture("plan-b-reuse").await;
    let residual_relation_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM pg_class c
             JOIN pg_namespace n ON n.oid=c.relnamespace
            WHERE n.nspname='public' AND c.relname='hypothesis_residual_risks'
              AND c.relkind='r'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("count residual ledgers");
    assert_eq!(residual_relation_count, 1);

    let duplicate_plan_tables: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
            WHERE n.nspname='public' AND c.relkind='r'
              AND c.relname LIKE 'verification_campaign_plan%'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("find accidental Plan C plan copies");
    assert_eq!(duplicate_plan_tables, 0);
}

#[tokio::test]
#[serial]
async fn safety_hold_starts_dispatch_held_without_blocking_legacy_admission() {
    let (db, _data_dir) = fixture("safety-hold").await;
    let row: (bool, bool, i64, i64, i64, String) = sqlx::query_as(
        r#"SELECT campaign_dispatch_held,operation_admission_held,
                  campaign_dispatch_generation,operation_admission_generation,
                  row_version,reason_code
             FROM verification_campaign_safety_holds WHERE singleton=TRUE"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("read safety hold");
    assert_eq!(row, (true, false, 0, 0, 0, "initial_rollout_hold".into()));
}

#[tokio::test]
#[serial]
async fn schema_enforces_active_lane_exact_coverage_and_append_only_history() {
    let (db, _data_dir) = fixture("constraints").await;
    let indexes: Vec<String> = sqlx::query_scalar(
        r#"SELECT indexname FROM pg_indexes
            WHERE schemaname='public' AND indexname = ANY($1) ORDER BY indexname"#,
    )
    .bind(vec![
        "verification_campaigns_one_active_contract",
        "verification_prepared_actions_one_active_lane",
        "verification_action_executions_one_per_ordinal",
        "verification_fact_delta_one_per_terminal",
    ])
    .fetch_all(db.pool())
    .await
    .expect("inspect partial unique indexes");
    assert_eq!(indexes.len(), 4);

    let append_only_triggers: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM pg_trigger
            WHERE NOT tgisinternal AND tgname LIKE 'verification_%_append_only'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("count append-only triggers");
    assert!(append_only_triggers >= 30);

    let coverage_check: String = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(oid)
              FROM pg_constraint
             WHERE conname='verification_campaign_coverage_result_shape_check'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("read coverage shape constraint");
    assert!(coverage_check.contains("tested_degraded"));
    assert!(coverage_check.contains("not_required"));
    assert!(coverage_check.contains("explicit_no_control"));
}

#[tokio::test]
#[serial]
async fn shadow_storage_has_no_executable_or_canonical_verdict_ports() {
    let (db, _data_dir) = fixture("shadow-isolation").await;
    let forbidden: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM information_schema.columns
            WHERE table_schema='public'
              AND table_name IN (
                  'verification_campaign_shadow_evaluations',
                  'verification_campaign_shadow_evaluation_items'
              )
              AND column_name = ANY($1)"#,
    )
    .bind(vec![
        "authorization_token",
        "credential_id",
        "budget_reservation_id",
        "finding_id",
        "fact_delta_bundle_id",
        "comparison_state",
        "diff_summary",
    ])
    .fetch_one(db.pool())
    .await
    .expect("inspect shadow isolation");
    assert_eq!(forbidden, 0);
}

#[tokio::test]
#[serial]
async fn prepared_action_review_is_hash_bound_local_and_high_risk_only() {
    let (db, _data_dir) = fixture("prepared-review").await;
    let review_columns: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM information_schema.columns
            WHERE table_schema='public' AND table_name='verification_prepared_actions'
              AND column_name = ANY($1)"#,
    )
    .bind(vec![
        "display_projection",
        "display_projection_hash",
        "renderer_version",
        "private_manifest_hash",
        "review_expires_at",
    ])
    .fetch_one(db.pool())
    .await
    .expect("inspect safe review authority columns");
    assert_eq!(review_columns, 5);

    let guard: String = sqlx::query_scalar(
        "SELECT pg_get_functiondef('verification_guard_action_authorization()'::regprocedure)",
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect authorization guard");
    for required in [
        "local_operator",
        "T2",
        "T3",
        "display_projection_hash",
        "private_manifest_hash",
        "renderer_version",
        "review_expires_at",
    ] {
        assert!(guard.contains(required), "guard is missing {required}");
    }
}

#[tokio::test]
#[serial]
async fn scheduler_authority_auto_authorizes_low_risk_with_a_durable_policy_receipt() {
    use repo::verification_prepared_actions::{
        reconcile_prepared_action_scheduler_authority, PreparedActionSchedulerAuthorityDisposition,
        ReconcilePreparedActionSchedulerAuthority,
    };

    let (db, _data_dir) = fixture("scheduler-authority").await;
    let installed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM _sqlx_migrations WHERE version=$1 AND success)",
    )
    .bind(SCHEDULER_AUTHORITY_MIGRATION_VERSION)
    .fetch_one(db.pool())
    .await
    .expect("read scheduler authority migration ledger");
    assert!(installed);

    let operation_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let hypothesis_revision_id = Uuid::new_v4();
    let campaign_id = Uuid::new_v4();
    let capability_assessment_id = Uuid::new_v4();
    let prepared_action_id = Uuid::new_v4();
    let hash = |digit: char| format!("sha256:{}", digit.to_string().repeat(64));
    let mut tx = db.pool().begin().await.expect("begin policy fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("isolate compact policy fixture");
    sqlx::query(
        r#"INSERT INTO verification_campaigns(
               campaign_id,stable_request_id,operation_id,project_scope_id,organization_id,
               hypothesis_revision_id,verification_plan_id,verification_plan_hash,
               plan_objective_id,verification_objective_id,verification_contract_id,
               verification_contract_hash,capability_assessment_set_seal_id,wave_denominator_id,
               tool_truth_authority_bundle_seal_id,relevant_root_set_hash,
               authority_member_set_hash,semantic_authority_bundle_hash,
               freshness_attestation_bundle_hash,temporal_validity_bundle_hash,
               effective_valid_until,campaign_version,state,source_snapshot_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,
                  NOW()+INTERVAL '1 hour',1,'admitted',$21)"#,
    )
    .bind(campaign_id)
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(hypothesis_revision_id)
    .bind(Uuid::new_v4())
    .bind(hash('1'))
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(hash('2'))
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(hash('3'))
    .bind(hash('4'))
    .bind(hash('5'))
    .bind(hash('6'))
    .bind(hash('7'))
    .bind(hash('8'))
    .execute(&mut *tx)
    .await
    .expect("seed campaign");
    sqlx::query(
        r#"INSERT INTO verification_capability_assessments(
               assessment_id,stable_request_id,operation_id,project_scope_id,organization_id,
               hypothesis_revision_id,verification_objective_id,verification_contract_id,
               verification_contract_hash,capability_key,capability_contract_version,
               capability_contract_hash,policy_snapshot_id,policy_snapshot_hash,
               assessment_ordinal,status,adapter_contract_version,adapter_contract_digest,
               source_snapshot_hash,assessment_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'fixture_http_get','fixture.v1',$10,$11,$12,
                  0,'available','fixture.adapter.v1',$13,$14,$15)"#,
    )
    .bind(capability_assessment_id)
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(hypothesis_revision_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(hash('2'))
    .bind(hash('9'))
    .bind(Uuid::new_v4())
    .bind(hash('a'))
    .bind(hash('b'))
    .bind(hash('c'))
    .bind(hash('d'))
    .execute(&mut *tx)
    .await
    .expect("seed available capability");
    sqlx::query(
        r#"INSERT INTO verification_prepared_actions(
               prepared_action_id,stable_request_id,campaign_id,round_id,strategy_artifact_id,
               operation_id,project_scope_id,organization_id,capability_assessment_id,
               action_ordinal,action_contract_kind,action_kind,canonical_request_hash,
               display_projection,display_projection_hash,renderer_version,private_manifest,
               private_manifest_hash,review_expires_at,target_live_id,target_type_at_time,
               target_value_at_time,target_identity_hash,policy_snapshot_hash,
               upper_budget_set_hash,oracle_contract_hash,risk_tier,state)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,0,'single_action_v1','fixture_http_get',$10,
                  '{}'::JSONB,$11,'fixture.renderer.v1','{}'::JSONB,$12,
                  NOW()+INTERVAL '1 hour',$13,'url','https://fixture.invalid/',$14,$15,$16,$17,
                  'T1','pending_authorization')"#,
    )
    .bind(prepared_action_id)
    .bind(Uuid::new_v4())
    .bind(campaign_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(capability_assessment_id)
    .bind(hash('e'))
    .bind(hash('f'))
    .bind(hash('0'))
    .bind(Uuid::new_v4())
    .bind(hash('1'))
    .bind(hash('a'))
    .bind(hash('b'))
    .bind(hash('c'))
    .execute(&mut *tx)
    .await
    .expect("seed pending T1 action");
    sqlx::query(
        r#"INSERT INTO verification_campaign_coverage_denominators(
               campaign_denominator_id,stable_request_id,operation_id,project_scope_id,
               organization_id,campaign_id,hypothesis_revision_id,wave_denominator_id,
               contract_version,source_snapshot_hash,member_set_hash,member_count,sealed_at)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,'fixture.v1',$9,$10,1,NOW())"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(campaign_id)
    .bind(hypothesis_revision_id)
    .bind(Uuid::new_v4())
    .bind(hash('d'))
    .bind(hash('e'))
    .execute(&mut *tx)
    .await
    .expect("seed sealed campaign denominator");
    sqlx::query(
        r#"INSERT INTO verification_action_conflict_sets(
               conflict_set_id,stable_request_id,prepared_action_id,campaign_id,
               operation_id,project_scope_id,organization_id,member_count,member_set_hash,sealed_at)
           VALUES($1,$2,$3,$4,$5,$6,$7,1,$8,NOW())"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(prepared_action_id)
    .bind(campaign_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(hash('f'))
    .execute(&mut *tx)
    .await
    .expect("seed sealed conflict set");
    sqlx::query(
        r#"UPDATE verification_campaign_safety_holds
              SET campaign_dispatch_held=FALSE,campaign_dispatch_generation=1,
                  row_version=1,reason_code='fixture_release' WHERE singleton=TRUE"#,
    )
    .execute(&mut *tx)
    .await
    .expect("release fixture dispatch hold");
    tx.commit().await.expect("commit policy fixture");

    let receipt = reconcile_prepared_action_scheduler_authority(
        db.pool(),
        &ReconcilePreparedActionSchedulerAuthority {
            prepared_action_id,
            campaign_id,
            operation_id,
            expected_action_row_version: 0,
        },
    )
    .await
    .expect("auto-authorize live T1 action");
    assert_eq!(
        receipt.disposition,
        PreparedActionSchedulerAuthorityDisposition::AuthorizedByServerPolicy
    );
    assert_eq!(receipt.current_action_row_version, 1);
    assert!(receipt.authorization_receipt_id.is_some());

    let action: (String, i64) = sqlx::query_as(
        "SELECT state,row_version FROM verification_prepared_actions WHERE prepared_action_id=$1",
    )
    .bind(prepared_action_id)
    .fetch_one(db.pool())
    .await
    .expect("read authorized T1 action");
    assert_eq!(action, ("authorized".to_owned(), 1));
    let authority: (String, Option<Uuid>, String, String, i64) = sqlx::query_as(
        r#"SELECT actor_kind,decided_by,operator_channel,decision,campaign_dispatch_generation
             FROM verification_prepared_action_authorizations
            WHERE prepared_action_id=$1"#,
    )
    .bind(prepared_action_id)
    .fetch_one(db.pool())
    .await
    .expect("read durable server-policy receipt");
    assert_eq!(
        authority,
        (
            "server_policy".to_owned(),
            None,
            "server_policy".to_owned(),
            "authorized".to_owned(),
            1,
        )
    );
}

#[tokio::test]
#[serial]
async fn scheduler_authority_terminalizes_expired_review_and_jit_exactly_once() {
    use repo::verification_prepared_actions::{
        reconcile_prepared_action_scheduler_authority, PreparedActionSchedulerAuthorityDisposition,
        ReconcilePreparedActionSchedulerAuthority,
    };

    let (db, _data_dir) = fixture("scheduler-expiry").await;
    let operation_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let hypothesis_revision_id = Uuid::new_v4();
    let campaign_id = Uuid::new_v4();
    let capability_assessment_id = Uuid::new_v4();
    let pending_action_id = Uuid::new_v4();
    let hash = |digit: char| format!("sha256:{}", digit.to_string().repeat(64));
    let mut tx = db.pool().begin().await.expect("begin expiry fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("isolate compact expiry fixture");
    sqlx::query(
        "INSERT INTO organizations(id,project_path,name) VALUES($1,'/tmp/scheduler-expiry','Scheduler Expiry Org')",
    )
    .bind(organization_id)
    .execute(&mut *tx)
    .await
    .expect("seed expiry organization");
    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,project_scope_id,
               enumeration_analysis_contract,stage_topology_contract,
               stage_topology_canonical_json,stage_topology_sha256,
               stage_topology_freeze_source,investigation_contract_version,
               investigation_rollout_mode,tool_truth_contract)
           VALUES($1,'red_team','investigation','v2_only',$2,'legacy_v1',
                  'unified_investigation_v1',
                  stage_topology_canonical_json('unified_investigation_v1'),
                  stage_topology_contract_sha256('unified_investigation_v1'),
                  'deployment_pair_v1','hypothesis_registry_v1','new_only','receipt_v1')"#,
    )
    .bind(operation_id)
    .bind(project_scope_id)
    .execute(&mut *tx)
    .await
    .expect("seed expiry operation");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_revisions(
               revision_id,root_id,operation_id,organization_id,revision_ordinal,
               semantic_key,semantic_key_hash,subject_kind,subject_identity_hash,
               target_type_at_time,target_value_at_time,predicate_schema,predicate_version,
               normalized_arguments,trust_boundary,polarity,epistemic_state,lifecycle_state,
               planning_readiness,structured_claim,assumptions,missing_facts,priority,
               risk_impact,origin_decision_hash,revision_ingredients_hash,revision_hash)
           VALUES($1,$2,$3,$4,0,'{}'::JSONB,$5,'target',$6,'url',
                  'https://fixture.invalid/','fixture_predicate',1,'{}'::JSONB,'internet',
                  'positive','proposed','current','ready_for_strategy','{}'::JSONB,
                  '[]'::JSONB,'[]'::JSONB,1,'{}'::JSONB,$7,$8,$9)"#,
    )
    .bind(hypothesis_revision_id)
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(organization_id)
    .bind(hash('1'))
    .bind(hash('2'))
    .bind(hash('3'))
    .bind(hash('4'))
    .bind(hash('5'))
    .execute(&mut *tx)
    .await
    .expect("seed expiry hypothesis revision");
    sqlx::query(
        r#"INSERT INTO verification_campaigns(
               campaign_id,stable_request_id,operation_id,project_scope_id,organization_id,
               hypothesis_revision_id,verification_plan_id,verification_plan_hash,
               plan_objective_id,verification_objective_id,verification_contract_id,
               verification_contract_hash,capability_assessment_set_seal_id,wave_denominator_id,
               tool_truth_authority_bundle_seal_id,relevant_root_set_hash,
               authority_member_set_hash,semantic_authority_bundle_hash,
               freshness_attestation_bundle_hash,temporal_validity_bundle_hash,
               effective_valid_until,campaign_version,state,source_snapshot_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,
                  NOW()+INTERVAL '1 hour',1,'admitted',$21)"#,
    )
    .bind(campaign_id)
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(hypothesis_revision_id)
    .bind(Uuid::new_v4())
    .bind(hash('6'))
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(hash('7'))
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(hash('8'))
    .bind(hash('9'))
    .bind(hash('a'))
    .bind(hash('b'))
    .bind(hash('c'))
    .bind(hash('d'))
    .execute(&mut *tx)
    .await
    .expect("seed expiry campaign");
    sqlx::query(
        r#"INSERT INTO verification_prepared_actions(
               prepared_action_id,stable_request_id,campaign_id,round_id,strategy_artifact_id,
               operation_id,project_scope_id,organization_id,capability_assessment_id,
               action_ordinal,action_contract_kind,action_kind,canonical_request_hash,
               display_projection,display_projection_hash,renderer_version,private_manifest,
               private_manifest_hash,review_expires_at,target_live_id,target_type_at_time,
               target_value_at_time,target_identity_hash,policy_snapshot_hash,
               upper_budget_set_hash,oracle_contract_hash,risk_tier,state)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,0,'single_action_v1','fixture_http_get',$10,
                  '{}'::JSONB,$11,'fixture.renderer.v1','{}'::JSONB,$12,
                  NOW()-INTERVAL '1 minute',$13,'url','https://fixture.invalid/',$14,$15,$16,$17,
                  'T3','pending_authorization')"#,
    )
    .bind(pending_action_id)
    .bind(Uuid::new_v4())
    .bind(campaign_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(capability_assessment_id)
    .bind(hash('e'))
    .bind(hash('f'))
    .bind(hash('0'))
    .bind(Uuid::new_v4())
    .bind(hash('1'))
    .bind(hash('2'))
    .bind(hash('3'))
    .bind(hash('4'))
    .execute(&mut *tx)
    .await
    .expect("seed expired T3 review");
    tx.commit().await.expect("commit expired T3 fixture");

    let pending_command = ReconcilePreparedActionSchedulerAuthority {
        prepared_action_id: pending_action_id,
        campaign_id,
        operation_id,
        expected_action_row_version: 0,
    };
    let pending_receipt =
        reconcile_prepared_action_scheduler_authority(db.pool(), &pending_command)
            .await
            .expect("terminalize expired T3 review");
    assert_eq!(
        pending_receipt.disposition,
        PreparedActionSchedulerAuthorityDisposition::Expired
    );
    assert_eq!(pending_receipt.current_action_row_version, 1);
    let pending_replay = reconcile_prepared_action_scheduler_authority(db.pool(), &pending_command)
        .await
        .expect("replay exact expired T3 review command");
    assert_eq!(pending_replay, pending_receipt);
    let pending_shape: (
        String,
        String,
        i64,
        bool,
        bool,
        String,
        String,
        bool,
        i64,
        i64,
    ) = sqlx::query_as(
        r#"SELECT action.state,action.reason_code,action.row_version,
                      action.residual_id IS NOT NULL,action.terminal_at IS NOT NULL,
                      receipt.actor_kind,receipt.decision,receipt.expires_at IS NULL,
                      (SELECT COUNT(*) FROM verification_prepared_action_authorizations
                        WHERE prepared_action_id=action.prepared_action_id),
                      (SELECT COUNT(*) FROM hypothesis_residual_risks
                        WHERE residual_id=action.residual_id)
                 FROM verification_prepared_actions action
                 JOIN verification_prepared_action_authorizations receipt
                   ON receipt.prepared_action_id=action.prepared_action_id
                WHERE action.prepared_action_id=$1"#,
    )
    .bind(pending_action_id)
    .fetch_one(db.pool())
    .await
    .expect("read exact expired T3 landing");
    assert_eq!(
        pending_shape,
        (
            "expired".to_owned(),
            "server_policy_review_expired".to_owned(),
            1,
            true,
            true,
            "server_policy".to_owned(),
            "expired".to_owned(),
            true,
            1,
            1,
        )
    );

    let authorized_action_id = Uuid::new_v4();
    let original_authorization_id = Uuid::new_v4();
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin authorized expiry fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("isolate authorized expiry fixture");
    sqlx::query(
        r#"INSERT INTO verification_prepared_actions(
               prepared_action_id,stable_request_id,campaign_id,round_id,strategy_artifact_id,
               operation_id,project_scope_id,organization_id,capability_assessment_id,
               action_ordinal,action_contract_kind,action_kind,canonical_request_hash,
               display_projection,display_projection_hash,renderer_version,private_manifest,
               private_manifest_hash,review_expires_at,target_live_id,target_type_at_time,
               target_value_at_time,target_identity_hash,policy_snapshot_hash,
               upper_budget_set_hash,oracle_contract_hash,risk_tier,state,row_version)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,1,'single_action_v1','fixture_http_get',$10,
                  '{}'::JSONB,$11,'fixture.renderer.v1','{}'::JSONB,$12,
                  NOW()+INTERVAL '1 hour',$13,'url','https://fixture.invalid/',$14,$15,$16,$17,
                  'T1','authorized',1)"#,
    )
    .bind(authorized_action_id)
    .bind(Uuid::new_v4())
    .bind(campaign_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(capability_assessment_id)
    .bind(hash('5'))
    .bind(hash('6'))
    .bind(hash('7'))
    .bind(Uuid::new_v4())
    .bind(hash('8'))
    .bind(hash('9'))
    .bind(hash('a'))
    .bind(hash('b'))
    .execute(&mut *tx)
    .await
    .expect("seed authorized T1 action");
    sqlx::query(
        r#"INSERT INTO verification_prepared_action_authorizations(
               authorization_receipt_id,stable_request_id,prepared_action_id,campaign_id,
               operation_id,project_scope_id,organization_id,decision,decision_reason_code,
               expected_action_row_version,campaign_dispatch_generation,renderer_version,
               reviewed_action_hash,expected_display_projection_hash,
               expected_private_manifest_hash,authorization_hash,decided_by,actor_kind,
               operator_channel,expires_at,residual_id)
           VALUES($1,$2,$3,$4,$5,$6,$7,'authorized','server_policy_auto_authorized_t0_t1',
                  0,0,'fixture.renderer.v1',$8,$8,$9,$10,NULL,'server_policy',
                  'server_policy',NOW()-INTERVAL '1 minute',NULL)"#,
    )
    .bind(original_authorization_id)
    .bind(Uuid::new_v4())
    .bind(authorized_action_id)
    .bind(campaign_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .bind(hash('6'))
    .bind(hash('7'))
    .bind(hash('c'))
    .execute(&mut *tx)
    .await
    .expect("seed elapsed JIT receipt");
    tx.commit().await.expect("commit elapsed JIT fixture");

    let authorized_command = ReconcilePreparedActionSchedulerAuthority {
        prepared_action_id: authorized_action_id,
        campaign_id,
        operation_id,
        expected_action_row_version: 1,
    };
    let authorized_receipt =
        reconcile_prepared_action_scheduler_authority(db.pool(), &authorized_command)
            .await
            .expect("terminalize elapsed JIT authority");
    assert_eq!(
        authorized_receipt.disposition,
        PreparedActionSchedulerAuthorityDisposition::Expired
    );
    assert_ne!(
        authorized_receipt.authorization_receipt_id,
        Some(original_authorization_id)
    );
    let authorized_replay =
        reconcile_prepared_action_scheduler_authority(db.pool(), &authorized_command)
            .await
            .expect("replay exact elapsed JIT command");
    assert_eq!(authorized_replay, authorized_receipt);
    let authorized_shape: (String, String, i64, bool, bool, i64, i64) = sqlx::query_as(
        r#"SELECT state,reason_code,row_version,residual_id IS NOT NULL,terminal_at IS NOT NULL,
                  (SELECT COUNT(*) FROM verification_prepared_action_authorizations
                    WHERE prepared_action_id=$1),
                  (SELECT COUNT(*) FROM hypothesis_residual_risks
                    WHERE residual_id=verification_prepared_actions.residual_id)
             FROM verification_prepared_actions WHERE prepared_action_id=$1"#,
    )
    .bind(authorized_action_id)
    .fetch_one(db.pool())
    .await
    .expect("read exact elapsed JIT landing");
    assert_eq!(
        authorized_shape,
        (
            "expired".to_owned(),
            "server_policy_authorization_expired".to_owned(),
            2,
            true,
            true,
            2,
            1,
        )
    );
}

#[tokio::test]
#[serial]
async fn budget_contracts_have_exact_hierarchy_and_mutable_cas_heads_only() {
    let (db, _data_dir) = fixture("budget-hierarchy").await;
    let hierarchy_trigger: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM pg_trigger
                WHERE tgname='verification_budget_contract_hierarchy_guard'
                  AND NOT tgisinternal
           )"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect budget hierarchy trigger");
    assert!(hierarchy_trigger);

    let axis_check: String = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(oid) FROM pg_constraint
            WHERE conrelid='verification_budget_contract_axes'::regclass
              AND contype='c' AND pg_get_constraintdef(oid) LIKE '%axis_kind%'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect closed budget axis enum");
    for axis in [
        "requests",
        "response_bytes",
        "wall_clock_ms",
        "retries",
        "browser_steps",
        "oast_tokens",
    ] {
        assert!(axis_check.contains(axis), "axis enum is missing {axis}");
    }
}
