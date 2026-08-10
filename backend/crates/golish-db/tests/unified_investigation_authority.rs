#![allow(dead_code)]

#[path = "../src/repo/hypothesis_verification_tasks.rs"]
mod hypothesis_verification_tasks;
#[path = "../src/repo/investigation_fuel_ledger.rs"]
mod investigation_fuel_ledger;
#[path = "../src/repo/investigation_main_sessions.rs"]
mod investigation_main_sessions;

use golish_core::hypothesis_verification_task::{
    HypothesisVerificationTaskHeaderV1, HypothesisVerificationTaskStateV1,
    NewHypothesisVerificationTaskV1, TaskObjectiveResidualKindV1,
    VerificationAdmissionDispositionV1,
};
use golish_core::investigation_fuel::{
    InvestigationFuelAxisV1, InvestigationFuelReservationStateV1,
    InvestigationSemanticCycleReceiptV1,
};
use golish_core::investigation_main_read_session::{
    BindMainOrganizationReadSessionV1, MainOrganizationReadSessionV1,
};
use golish_db::repo::operation_default_rollout::{
    operation_promotion_component_member_hash, operation_promotion_component_set_hash,
    promote_operation_defaults, OperationPromotionComponentRow, PromoteOperationDefaults,
    OPERATION_PROMOTION_COMPONENT_KINDS,
};
use golish_db::repo::operation_rollout_safety_hold::{
    set_operation_safety_hold, OperationSafetyHoldScope, SetOperationSafetyHold,
};
use golish_db::{DbConfig, GolishDb};
use hypothesis_verification_tasks::{
    AdmissionMemberInput, CampaignOutcomeInput, CampaignOutcomeKind, ObjectiveAssignmentInput,
    ObjectiveAssignmentMemberInput, SealAdmissionSetInput, SealCampaignOutcomesInput,
    SealObjectiveAssignmentsInput,
};
use investigation_fuel_ledger::{
    CreateFuelBudgetInput, FuelBudgetScope, RecordSemanticCycleInput, ReserveFuelInput,
    SemanticCycleDisposition, TransitionFuelReservationInput,
};
use investigation_main_sessions::{
    BeginMainSessionSet, RegisterInvestigationStageAuthority, SealInvestigationAnalysisSnapshot,
};
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
        database: format!("unified_authority_{label}_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start isolated migrated postgres");
    (db, data_dir)
}

#[derive(Debug, Clone)]
struct RuntimeFixture {
    operation_id: Uuid,
    project_scope_id: Uuid,
    stage_execution_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_ids: [Uuid; 2],
    stage_run_unit_ids: [Uuid; 2],
    authority_id: Uuid,
    owning_request_id: String,
}

async fn runtime_fixture(pool: &PgPool, label: &str) -> RuntimeFixture {
    let operation_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    let project_path = format!("/tmp/unified-authority-{label}-{}", Uuid::new_v4().simple());
    let au_execution_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    let scope_decision_id = Uuid::new_v4();
    let scope_snapshot_id = Uuid::new_v4();
    let organization_ids = [Uuid::new_v4(), Uuid::new_v4()];
    let stage_run_unit_ids = [Uuid::new_v4(), Uuid::new_v4()];

    sqlx::query(
        "INSERT INTO project_scopes(project_scope_id,canonical_project_path,path_sha256) VALUES($1,$2,$3)",
    )
    .bind(project_scope_id)
    .bind(&project_path)
    .bind(digest('1'))
    .execute(pool)
    .await
    .expect("insert project scope");
    let mut deployment = pool.begin().await.expect("begin deployment fixture");
    for statement in [
        "ALTER TABLE tool_truth_rollout DISABLE TRIGGER tool_truth_rollout_direct_mutation_guard",
        "ALTER TABLE investigation_rollout DISABLE TRIGGER investigation_rollout_direct_mutation_guard",
    ] {
        sqlx::query(statement)
            .execute(&mut *deployment)
            .await
            .expect("disable isolated rollout guard");
    }
    sqlx::query(
        "UPDATE tool_truth_rollout SET new_operation_contract='receipt_v1',row_version=row_version+1 WHERE singleton=TRUE",
    )
    .execute(&mut *deployment)
    .await
    .expect("select receipt Tool Truth");
    sqlx::query(
        r#"UPDATE investigation_rollout
              SET contract_version='hypothesis_registry_v1',rollout_mode='new_only',
                  mode_rank=4,row_version=row_version+1 WHERE singleton=TRUE"#,
    )
    .execute(&mut *deployment)
    .await
    .expect("select unified Investigation");
    for statement in [
        "ALTER TABLE investigation_rollout ENABLE TRIGGER investigation_rollout_direct_mutation_guard",
        "ALTER TABLE tool_truth_rollout ENABLE TRIGGER tool_truth_rollout_direct_mutation_guard",
    ] {
        sqlx::query(statement)
            .execute(&mut *deployment)
            .await
            .expect("restore isolated rollout guard");
    }
    deployment
        .commit()
        .await
        .expect("commit deployment fixture");

    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,project_scope_id,
               tool_truth_contract,investigation_contract_version,investigation_rollout_mode
           ) VALUES($1,'red_team','application_understanding','legacy_v1',$2,
                    'receipt_v1','hypothesis_registry_v1','new_only')"#,
    )
    .bind(operation_id)
    .bind(project_scope_id)
    .execute(pool)
    .await
    .expect("insert unified operation");
    sqlx::query(
        r#"INSERT INTO organizations(id,project_path,name) VALUES
               ($1,$3,'Root Authority Org'),($2,$3,'Child Authority Org')"#,
    )
    .bind(organization_ids[0])
    .bind(organization_ids[1])
    .bind(&project_path)
    .execute(pool)
    .await
    .expect("insert organizations");
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status) VALUES($1,$2,'application_understanding','started')",
    )
    .bind(au_execution_id)
    .bind(operation_id)
    .execute(pool)
    .await
    .expect("insert AU execution");
    sqlx::query(
        r#"INSERT INTO operation_scope_decisions(
               id,operation_id,project_scope_id,stage_execution_id,
               root_organization_id,mode,decision_rows,decision_hash
           ) VALUES($1,$2,$3,$4,$5,'cli_flags',$6,$7)"#,
    )
    .bind(scope_decision_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(au_execution_id)
    .bind(organization_ids[0])
    .bind(serde_json::json!([
        {"organization_id": organization_ids[0]},
        {"organization_id": organization_ids[1]}
    ]))
    .bind(digest('2'))
    .execute(pool)
    .await
    .expect("insert exact scope decision");

    let mut scope_tx = pool.begin().await.expect("begin scope seal");
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
    .bind(&project_path)
    .bind(organization_ids[0])
    .bind(digest('3'))
    .execute(&mut *scope_tx)
    .await
    .expect("insert scope snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units(
               snapshot_id,organization_id,parent_organization_id,
               organization_name_at_freeze,role,depth,ordinal,
               decision_row_id,approval_source
           ) VALUES
               ($1,$2,NULL,'Root Authority Org','root',0,0,'root',$4),
               ($1,$3,$2,'Child Authority Org','subsidiary',1,1,'child',$4)"#,
    )
    .bind(scope_snapshot_id)
    .bind(organization_ids[0])
    .bind(organization_ids[1])
    .bind(serde_json::json!({"source":"isolated_fixture"}))
    .execute(&mut *scope_tx)
    .await
    .expect("insert exact scope members");
    sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
        .bind(scope_snapshot_id)
        .execute(&mut *scope_tx)
        .await
        .expect("seal exact scope");
    scope_tx.commit().await.expect("commit exact scope");

    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status) VALUES($1,$2,'investigation','started')",
    )
    .bind(stage_execution_id)
    .bind(operation_id)
    .execute(pool)
    .await
    .expect("insert one Investigation execution");
    sqlx::query("UPDATE operation_state SET current_stage='investigation' WHERE operation_id=$1")
        .bind(operation_id)
        .execute(pool)
        .await
        .expect("enter unified Investigation");
    for index in 0..2 {
        sqlx::query(
            r#"INSERT INTO stage_run_units(
                   id,operation_id,stage_execution_id,scope_snapshot_id,
                   organization_id,stage_kind,generation,status,started_at
               ) VALUES($1,$2,$3,$4,$5,'investigation',0,'running',NOW())"#,
        )
        .bind(stage_run_unit_ids[index])
        .bind(operation_id)
        .bind(stage_execution_id)
        .bind(scope_snapshot_id)
        .bind(organization_ids[index])
        .execute(pool)
        .await
        .expect("insert per-organization Investigation unit");
    }

    RuntimeFixture {
        operation_id,
        project_scope_id,
        stage_execution_id,
        scope_snapshot_id,
        organization_ids,
        stage_run_unit_ids,
        authority_id: Uuid::new_v4(),
        owning_request_id: format!("stage-run-request-{label}"),
    }
}

#[derive(Debug, Clone, Copy)]
struct HypothesisFixture {
    revision_id: Uuid,
    revision_hash_nibble: char,
    plan_id: Uuid,
    plan_hash_nibble: char,
    plan_objective_id: Uuid,
    generation_ids: [Uuid; 2],
    generation_member_ids: [Uuid; 2],
}

async fn seed_hypothesis_authority(pool: &PgPool, runtime: &RuntimeFixture) -> HypothesisFixture {
    let root_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let objective_id = Uuid::new_v4();
    let contract_id = Uuid::new_v4();
    let plan_id = Uuid::new_v4();
    let plan_objective_id = Uuid::new_v4();
    let generation_ids = [Uuid::new_v4(), Uuid::new_v4()];
    let generation_member_ids = [Uuid::new_v4(), Uuid::new_v4()];
    let revision_hash_nibble = 'b';
    let plan_hash_nibble = 'd';

    // These rows are existing sealed Registry authority, not behavior under
    // test here. In this isolated fixture FK/append triggers are disabled only
    // while seeding the already-valid parent graph; our new tables and all
    // their triggers run with the normal origin role below.
    let mut tx = pool.begin().await.expect("begin Registry authority seed");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("enter isolated Registry seed role");
    sqlx::query(
        r#"INSERT INTO attack_hypotheses(
               root_id,operation_id,organization_id,root_kind,
               identity_ingredients,identity_ingredients_hash
           ) VALUES($1,$2,$3,'initial','{}'::jsonb,$4)"#,
    )
    .bind(root_id)
    .bind(runtime.operation_id)
    .bind(runtime.organization_ids[0])
    .bind(digest('a'))
    .execute(&mut *tx)
    .await
    .expect("seed hypothesis root");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_revisions(
               revision_id,root_id,operation_id,organization_id,revision_ordinal,
               semantic_key,semantic_key_hash,subject_kind,subject_identity_hash,
               target_type_at_time,target_value_at_time,predicate_schema,predicate_version,
               normalized_arguments,trust_boundary,polarity,epistemic_state,lifecycle_state,
               planning_readiness,structured_claim,assumptions,missing_facts,priority,
               risk_impact,origin_decision_hash,revision_ingredients_hash,revision_hash
           ) VALUES(
               $1,$2,$3,$4,0,'{}'::jsonb,$5,'endpoint',$6,'domain','example.test',
               'reachable_service',1,'{}'::jsonb,'internet','positive','proposed','current',
               'ready_for_strategy','{}'::jsonb,'[]'::jsonb,'[]'::jsonb,10,
               '{}'::jsonb,$7,$8,$9
           )"#,
    )
    .bind(revision_id)
    .bind(root_id)
    .bind(runtime.operation_id)
    .bind(runtime.organization_ids[0])
    .bind(digest('1'))
    .bind(digest('2'))
    .bind(digest('3'))
    .bind(digest('c'))
    .bind(digest(revision_hash_nibble))
    .execute(&mut *tx)
    .await
    .expect("seed hypothesis revision");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_verification_objectives(
               objective_id,revision_id,objective_ordinal,objective_intent,
               stopping_criteria,stopping_criteria_hash,objective_hash
           ) VALUES($1,$2,0,'{}'::jsonb,'{}'::jsonb,$3,$4)"#,
    )
    .bind(objective_id)
    .bind(revision_id)
    .bind(digest('e'))
    .bind(digest('f'))
    .execute(&mut *tx)
    .await
    .expect("seed verification objective");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_verification_contracts(
               contract_id,revision_id,revision_hash,objective_id,combinator,
               predicate_count,predicate_set_hash,required_control_count,
               required_control_set_hash,explicit_no_required_control,
               paired_differential_count,paired_differential_set_hash,
               ordered_step_count,ordered_step_set_hash,stopping_criteria_hash,
               compiler_digest,rule_digest,policy_snapshot_hash,contract_hash
           ) VALUES(
               $1,$2,$3,$4,'all_of',1,$5,0,$6,TRUE,0,$7,0,$8,$9,$10,$11,$12,$13
           )"#,
    )
    .bind(contract_id)
    .bind(revision_id)
    .bind(digest(revision_hash_nibble))
    .bind(objective_id)
    .bind(digest('1'))
    .bind(digest('2'))
    .bind(digest('3'))
    .bind(digest('4'))
    .bind(digest('e'))
    .bind(digest('5'))
    .bind(digest('6'))
    .bind(digest('7'))
    .bind(digest('8'))
    .execute(&mut *tx)
    .await
    .expect("seed verification contract");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_verification_plans(
               plan_id,revision_id,revision_hash,revision_ingredients_hash,
               required_claim_component_count,required_claim_component_set_hash,
               objective_count,objective_set_hash,proof_path_count,proof_path_set_hash,
               outer_aggregation_policy_version,outer_aggregation_policy_digest,
               plan_hash,sealed_at
           ) VALUES($1,$2,$3,$4,1,$5,1,$6,1,$7,1,$8,$9,NOW())"#,
    )
    .bind(plan_id)
    .bind(revision_id)
    .bind(digest(revision_hash_nibble))
    .bind(digest('c'))
    .bind(digest('1'))
    .bind(digest('2'))
    .bind(digest('3'))
    .bind(digest('4'))
    .bind(digest(plan_hash_nibble))
    .execute(&mut *tx)
    .await
    .expect("seed sealed verification plan");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_verification_plan_objectives(
               plan_objective_id,plan_id,revision_id,objective_id,
               verification_contract_id,ordinal,objective_hash,
               verification_contract_version,verification_contract_hash,
               claim_component_count,claim_component_set_hash,stopping_criteria_hash,
               outcome_requirement,member_hash
           ) VALUES($1,$2,$3,$4,$5,0,$6,1,$7,0,$8,$9,
                    'satisfy_bound_components',$10)"#,
    )
    .bind(plan_objective_id)
    .bind(plan_id)
    .bind(revision_id)
    .bind(objective_id)
    .bind(contract_id)
    .bind(digest('f'))
    .bind(digest('8'))
    .bind(digest('9'))
    .bind(digest('e'))
    .bind(digest('a'))
    .execute(&mut *tx)
    .await
    .expect("seed plan objective");
    for index in 0..2 {
        sqlx::query(
            r#"INSERT INTO hypothesis_generations(
                   generation_id,operation_id,organization_id,generation_ordinal,
                   candidate_snapshot_id,candidate_gate_decision_id,
                   candidate_snapshot_authority_hash,previous_generation_id
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
        )
        .bind(generation_ids[index])
        .bind(runtime.operation_id)
        .bind(runtime.organization_ids[0])
        .bind(index as i32)
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(digest(if index == 0 { '1' } else { '2' }))
        .bind(if index == 0 {
            None
        } else {
            Some(generation_ids[0])
        })
        .execute(&mut *tx)
        .await
        .expect("seed hypothesis generation");
        sqlx::query(
            r#"INSERT INTO hypothesis_generation_members(
                   generation_member_id,generation_id,operation_id,organization_id,
                   revision_id,ordinal,member_hash
               ) VALUES($1,$2,$3,$4,$5,0,$6)"#,
        )
        .bind(generation_member_ids[index])
        .bind(generation_ids[index])
        .bind(runtime.operation_id)
        .bind(runtime.organization_ids[0])
        .bind(revision_id)
        .bind(digest(if index == 0 { '3' } else { '4' }))
        .execute(&mut *tx)
        .await
        .expect("seed hypothesis generation member");
    }
    tx.commit().await.expect("commit Registry authority seed");

    HypothesisFixture {
        revision_id,
        revision_hash_nibble,
        plan_id,
        plan_hash_nibble,
        plan_objective_id,
        generation_ids,
        generation_member_ids,
    }
}

fn snapshot_authority(
    runtime: &RuntimeFixture,
    index: usize,
    snapshot_id: Uuid,
) -> SealInvestigationAnalysisSnapshot {
    SealInvestigationAnalysisSnapshot {
        snapshot_id,
        authority_id: runtime.authority_id,
        operation_id: runtime.operation_id,
        stage_execution_id: runtime.stage_execution_id,
        owning_stage_run_request_id: runtime.owning_request_id.clone(),
        stage_run_unit_id: runtime.stage_run_unit_ids[index],
        scope_snapshot_id: runtime.scope_snapshot_id,
        organization_id: runtime.organization_ids[index],
        snapshot_sha256: digest(if index == 0 { 'a' } else { 'b' }),
        context_item_count: 2 + index as u32,
        context_item_set_sha256: digest(if index == 0 { 'c' } else { 'd' }),
        methodology_hit_count: 1,
        methodology_result_set_sha256: digest(if index == 0 { 'e' } else { 'f' }),
        omission_count: index as u32,
        omission_set_sha256: digest(if index == 0 { '1' } else { '2' }),
    }
}

fn main_session(
    runtime: &RuntimeFixture,
    index: usize,
    snapshot: &SealInvestigationAnalysisSnapshot,
) -> MainOrganizationReadSessionV1 {
    MainOrganizationReadSessionV1::host_bind(BindMainOrganizationReadSessionV1 {
        operation_id: runtime.operation_id,
        stage_execution_id: runtime.stage_execution_id,
        owning_stage_run_request_id: runtime.owning_request_id.clone(),
        stage_run_unit_id: runtime.stage_run_unit_ids[index],
        organization_id: runtime.organization_ids[index],
        snapshot_id: snapshot.snapshot_id,
        snapshot_sha256: snapshot.snapshot_sha256.clone(),
        context_chain_id: Uuid::new_v4(),
        transcript_partition_id: Uuid::new_v4(),
    })
    .expect("bind exact organization read session")
}

#[tokio::test]
#[serial]
async fn main_sessions_seal_one_exact_partition_per_organization_and_reject_gaps() {
    let (db, _data_dir) = migrated_db("main-partitions").await;
    let runtime = runtime_fixture(db.pool(), "main-partitions").await;
    investigation_main_sessions::register_stage_authority(
        db.pool(),
        &RegisterInvestigationStageAuthority {
            authority_id: runtime.authority_id,
            operation_id: runtime.operation_id,
            stage_execution_id: runtime.stage_execution_id,
            owning_stage_run_request_id: runtime.owning_request_id.clone(),
            scope_snapshot_id: runtime.scope_snapshot_id,
        },
    )
    .await
    .expect("register one real Investigation stage request");

    let snapshots = [
        snapshot_authority(&runtime, 0, Uuid::new_v4()),
        snapshot_authority(&runtime, 1, Uuid::new_v4()),
    ];
    for snapshot in &snapshots {
        investigation_main_sessions::seal_analysis_snapshot(db.pool(), snapshot)
            .await
            .expect("seal typed S1 snapshot authority");
    }
    let set_id = Uuid::new_v4();
    investigation_main_sessions::begin_session_set(
        db.pool(),
        &BeginMainSessionSet {
            session_set_id: set_id,
            stable_request_id: Uuid::new_v4(),
            authority_id: runtime.authority_id,
            operation_id: runtime.operation_id,
            stage_execution_id: runtime.stage_execution_id,
            owning_stage_run_request_id: runtime.owning_request_id.clone(),
            scope_snapshot_id: runtime.scope_snapshot_id,
            session_set_ordinal: 0,
        },
    )
    .await
    .expect("begin exact Main session set");
    let sessions = [
        main_session(&runtime, 0, &snapshots[0]),
        main_session(&runtime, 1, &snapshots[1]),
    ];
    for (index, session) in sessions.iter().enumerate() {
        investigation_main_sessions::insert_read_session(
            db.pool(),
            set_id,
            runtime.authority_id,
            runtime.operation_id,
            runtime.stage_execution_id,
            runtime.scope_snapshot_id,
            session,
        )
        .await
        .expect("persist exact per-organization partition identity");
        let snapshot = &snapshots[index];
        let receipt = session
            .host_receipt(
                snapshot.context_item_count,
                snapshot.context_item_set_sha256.clone(),
                snapshot.methodology_hit_count,
                snapshot.methodology_result_set_sha256.clone(),
                snapshot.omission_count,
                snapshot.omission_set_sha256.clone(),
            )
            .expect("derive typed redacted read receipt");
        investigation_main_sessions::record_read_receipt(db.pool(), Uuid::new_v4(), &receipt)
            .await
            .expect("persist receipt without raw ContextPack body");
    }
    let sealed = investigation_main_sessions::seal_session_set(db.pool(), set_id, 0)
        .await
        .expect("seal complete organization partition set");
    assert_eq!(sealed.member_count, Some(2));
    assert_eq!(sealed.row_version, 1);

    let forbidden_raw_columns: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM information_schema.columns
            WHERE table_schema='public'
              AND table_name IN (
                  'investigation_main_read_sessions',
                  'investigation_main_read_session_receipts'
              )
              AND column_name IN (
                  'context_pack','context_pack_body','raw_context','transcript_body'
              )"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect coordinator-visible persistence shape");
    assert_eq!(forbidden_raw_columns, 0);

    let incomplete_set_id = Uuid::new_v4();
    investigation_main_sessions::begin_session_set(
        db.pool(),
        &BeginMainSessionSet {
            session_set_id: incomplete_set_id,
            stable_request_id: Uuid::new_v4(),
            authority_id: runtime.authority_id,
            operation_id: runtime.operation_id,
            stage_execution_id: runtime.stage_execution_id,
            owning_stage_run_request_id: runtime.owning_request_id.clone(),
            scope_snapshot_id: runtime.scope_snapshot_id,
            session_set_ordinal: 1,
        },
    )
    .await
    .expect("begin intentionally incomplete set");
    let only_root = main_session(&runtime, 0, &snapshots[0]);
    investigation_main_sessions::insert_read_session(
        db.pool(),
        incomplete_set_id,
        runtime.authority_id,
        runtime.operation_id,
        runtime.stage_execution_id,
        runtime.scope_snapshot_id,
        &only_root,
    )
    .await
    .expect("persist only one of two required partitions");
    let root_receipt = only_root
        .host_receipt(
            snapshots[0].context_item_count,
            snapshots[0].context_item_set_sha256.clone(),
            snapshots[0].methodology_hit_count,
            snapshots[0].methodology_result_set_sha256.clone(),
            snapshots[0].omission_count,
            snapshots[0].omission_set_sha256.clone(),
        )
        .expect("derive incomplete-set receipt");
    investigation_main_sessions::record_read_receipt(db.pool(), Uuid::new_v4(), &root_receipt)
        .await
        .expect("persist root receipt");
    let missing_partition =
        investigation_main_sessions::seal_session_set(db.pool(), incomplete_set_id, 0)
            .await
            .expect_err("exact-set gate must reject a missing organization");
    assert!(
        missing_partition
            .to_string()
            .contains("INVESTIGATION_MAIN_SESSION_PARTITION_SET_INCOMPLETE"),
        "unexpected missing-partition error: {missing_partition}"
    );

    let wrong_set_id = Uuid::new_v4();
    investigation_main_sessions::begin_session_set(
        db.pool(),
        &BeginMainSessionSet {
            session_set_id: wrong_set_id,
            stable_request_id: Uuid::new_v4(),
            authority_id: runtime.authority_id,
            operation_id: runtime.operation_id,
            stage_execution_id: runtime.stage_execution_id,
            owning_stage_run_request_id: runtime.owning_request_id.clone(),
            scope_snapshot_id: runtime.scope_snapshot_id,
            session_set_ordinal: 2,
        },
    )
    .await
    .expect("begin identity-mismatch set");
    let wrong_unit = MainOrganizationReadSessionV1::host_bind(BindMainOrganizationReadSessionV1 {
        operation_id: runtime.operation_id,
        stage_execution_id: runtime.stage_execution_id,
        owning_stage_run_request_id: runtime.owning_request_id.clone(),
        stage_run_unit_id: runtime.stage_run_unit_ids[0],
        organization_id: runtime.organization_ids[1],
        snapshot_id: snapshots[1].snapshot_id,
        snapshot_sha256: snapshots[1].snapshot_sha256.clone(),
        context_chain_id: Uuid::new_v4(),
        transcript_partition_id: Uuid::new_v4(),
    })
    .expect("construct cross-organization adversarial session");
    assert!(
        investigation_main_sessions::insert_read_session(
            db.pool(),
            wrong_set_id,
            runtime.authority_id,
            runtime.operation_id,
            runtime.stage_execution_id,
            runtime.scope_snapshot_id,
            &wrong_unit,
        )
        .await
        .is_err(),
        "DB identity authority must reject a unit borrowed from another organization"
    );
}

#[tokio::test]
#[serial]
async fn verification_task_exact_sets_state_events_and_fuel_close_as_one_authority() {
    let (db, _data_dir) = migrated_db("task-fuel").await;
    let runtime = runtime_fixture(db.pool(), "task-fuel").await;
    investigation_main_sessions::register_stage_authority(
        db.pool(),
        &RegisterInvestigationStageAuthority {
            authority_id: runtime.authority_id,
            operation_id: runtime.operation_id,
            stage_execution_id: runtime.stage_execution_id,
            owning_stage_run_request_id: runtime.owning_request_id.clone(),
            scope_snapshot_id: runtime.scope_snapshot_id,
        },
    )
    .await
    .expect("register Investigation authority for task fuel");
    let hypothesis = seed_hypothesis_authority(db.pool(), &runtime).await;

    let header = HypothesisVerificationTaskHeaderV1::host_create(NewHypothesisVerificationTaskV1 {
        operation_id: runtime.operation_id,
        stage_execution_id: runtime.stage_execution_id,
        stage_run_unit_id: runtime.stage_run_unit_ids[0],
        organization_id: runtime.organization_ids[0],
        scope_snapshot_id: runtime.scope_snapshot_id,
        hypothesis_revision_id: hypothesis.revision_id,
        hypothesis_revision_sha256: digest(hypothesis.revision_hash_nibble),
        verification_plan_sha256: digest(hypothesis.plan_hash_nibble),
        relevant_evidence_snapshot_id: Uuid::new_v4(),
        semantic_evidence_set_sha256: digest('1'),
        open_obligation_set_sha256: digest('2'),
        semantic_attempt_fingerprint: digest('3'),
        first_admission_generation_id: hypothesis.generation_ids[0],
        host_rerun_receipt_id: None,
        host_rerun_receipt_sha256: None,
        rerun_contract_version: None,
    })
    .expect("derive stable verification task identity");
    let created = hypothesis_verification_tasks::create_or_replay_task(db.pool(), &header)
        .await
        .expect("create automatically admitted task");
    assert!(!created.replayed);
    assert_eq!(created.task.project_scope_id, runtime.project_scope_id);
    assert_eq!(created.task.verification_plan_id, hypothesis.plan_id);

    let replay_header =
        HypothesisVerificationTaskHeaderV1::host_create(NewHypothesisVerificationTaskV1 {
            relevant_evidence_snapshot_id: Uuid::new_v4(),
            first_admission_generation_id: hypothesis.generation_ids[1],
            operation_id: header.operation_id,
            stage_execution_id: header.stage_execution_id,
            stage_run_unit_id: header.stage_run_unit_id,
            organization_id: header.organization_id,
            scope_snapshot_id: header.scope_snapshot_id,
            hypothesis_revision_id: header.hypothesis_revision_id,
            hypothesis_revision_sha256: header.hypothesis_revision_sha256.clone(),
            verification_plan_sha256: header.verification_plan_sha256.clone(),
            semantic_evidence_set_sha256: header.semantic_evidence_set_sha256.clone(),
            open_obligation_set_sha256: header.open_obligation_set_sha256.clone(),
            semantic_attempt_fingerprint: header.semantic_attempt_fingerprint.clone(),
            host_rerun_receipt_id: None,
            host_rerun_receipt_sha256: None,
            rerun_contract_version: None,
        })
        .expect("derive semantic replay independent of wrapper/generation ids");
    assert_eq!(
        replay_header.stable_task_key_sha256,
        header.stable_task_key_sha256
    );
    let replayed = hypothesis_verification_tasks::create_or_replay_task(db.pool(), &replay_header)
        .await
        .expect("replay same semantic task");
    assert!(replayed.replayed);
    assert_eq!(
        replayed.task.first_admission_generation_id,
        hypothesis.generation_ids[0]
    );

    let incomplete_admission = hypothesis_verification_tasks::seal_admission_set(
        db.pool(),
        &SealAdmissionSetInput {
            admission_set_id: Uuid::new_v4(),
            stable_request_id: Uuid::new_v4(),
            operation_id: runtime.operation_id,
            stage_execution_id: runtime.stage_execution_id,
            stage_run_unit_id: runtime.stage_run_unit_ids[0],
            scope_snapshot_id: runtime.scope_snapshot_id,
            organization_id: runtime.organization_ids[0],
            generation_id: hypothesis.generation_ids[1],
            members: vec![],
        },
    )
    .await
    .expect_err("generation census cannot omit a Registry member");
    assert!(
        incomplete_admission
            .to_string()
            .contains("INVESTIGATION_ADMISSION_EXACT_SET_INCOMPLETE"),
        "unexpected incomplete-admission error: {incomplete_admission}"
    );

    let admission = hypothesis_verification_tasks::seal_admission_set(
        db.pool(),
        &SealAdmissionSetInput {
            admission_set_id: Uuid::new_v4(),
            stable_request_id: Uuid::new_v4(),
            operation_id: runtime.operation_id,
            stage_execution_id: runtime.stage_execution_id,
            stage_run_unit_id: runtime.stage_run_unit_ids[0],
            scope_snapshot_id: runtime.scope_snapshot_id,
            organization_id: runtime.organization_ids[0],
            generation_id: hypothesis.generation_ids[0],
            members: vec![AdmissionMemberInput {
                admission_member_id: Uuid::new_v4(),
                generation_member_id: hypothesis.generation_member_ids[0],
                disposition: VerificationAdmissionDispositionV1::Scheduled,
                reason_code: "automatic_ready_for_strategy".into(),
                semantic_attempt_fingerprint: header.semantic_attempt_fingerprint.clone(),
                task_id: Some(header.task_id),
            }],
        },
    )
    .await
    .expect("seal exact automatic admission census");
    assert_eq!(admission.member_count, Some(1));

    let assignment_set_id = Uuid::new_v4();
    let campaign_id = Uuid::new_v4();
    let assignment = hypothesis_verification_tasks::seal_objective_assignments(
        db.pool(),
        &SealObjectiveAssignmentsInput {
            assignment_set_id,
            stable_request_id: Uuid::new_v4(),
            task_id: header.task_id,
            members: vec![ObjectiveAssignmentMemberInput {
                assignment_member_id: Uuid::new_v4(),
                plan_objective_id: hypothesis.plan_objective_id,
                assignment: ObjectiveAssignmentInput::Campaign { campaign_id },
            }],
        },
    )
    .await
    .expect("seal exact task objective denominator");
    assert_eq!(assignment.member_count, Some(1));

    let outcome_set_id = Uuid::new_v4();
    let outcome_request_id = Uuid::new_v4();
    let missing_outcome = hypothesis_verification_tasks::seal_campaign_outcomes(
        db.pool(),
        &SealCampaignOutcomesInput {
            outcome_set_id,
            stable_request_id: outcome_request_id,
            assignment_set_id,
            task_id: header.task_id,
            outcomes: vec![],
        },
    )
    .await
    .expect_err("campaign reservation must receive one terminal outcome");
    assert!(
        missing_outcome
            .to_string()
            .contains("INVESTIGATION_OUTCOME_CAMPAIGN_SET_MISMATCH"),
        "unexpected missing-outcome error: {missing_outcome}"
    );
    let outcome = hypothesis_verification_tasks::seal_campaign_outcomes(
        db.pool(),
        &SealCampaignOutcomesInput {
            outcome_set_id,
            stable_request_id: outcome_request_id,
            assignment_set_id,
            task_id: header.task_id,
            outcomes: vec![CampaignOutcomeInput {
                outcome_member_id: Uuid::new_v4(),
                campaign_id,
                outcome_kind: CampaignOutcomeKind::Completed,
                terminal_receipt_id: Uuid::new_v4(),
                terminal_receipt_sha256: digest('4'),
            }],
        },
    )
    .await
    .expect("seal exact campaign outcome census");
    assert_eq!(outcome.member_count, Some(1));

    let illegal_state = hypothesis_verification_tasks::append_task_state(
        db.pool(),
        header.task_id,
        Uuid::new_v4(),
        Uuid::new_v4(),
        0,
        HypothesisVerificationTaskStateV1::Running,
        "skip_queue_and_plan",
    )
    .await
    .expect_err("state authority must reject non-graph transitions");
    assert!(
        illegal_state
            .to_string()
            .contains("INVESTIGATION_TASK_STATE_CAS_INVALID"),
        "unexpected illegal-state error: {illegal_state}"
    );
    for (expected, state) in [
        (0, HypothesisVerificationTaskStateV1::Queued),
        (1, HypothesisVerificationTaskStateV1::Planning),
        (2, HypothesisVerificationTaskStateV1::Running),
        (3, HypothesisVerificationTaskStateV1::Consolidating),
        (4, HypothesisVerificationTaskStateV1::Terminal),
    ] {
        let head = hypothesis_verification_tasks::append_task_state(
            db.pool(),
            header.task_id,
            Uuid::new_v4(),
            Uuid::new_v4(),
            expected,
            state,
            "host_state_event",
        )
        .await
        .expect("append legal task state event and CAS head");
        assert_eq!(head.head_version, expected + 1);
    }

    let budget_id = Uuid::new_v4();
    investigation_fuel_ledger::create_budget(
        db.pool(),
        &CreateFuelBudgetInput {
            budget_id,
            stable_request_id: Uuid::new_v4(),
            authority_id: runtime.authority_id,
            operation_id: runtime.operation_id,
            stage_execution_id: runtime.stage_execution_id,
            owning_stage_run_request_id: runtime.owning_request_id.clone(),
            scope: FuelBudgetScope::Task {
                stage_run_unit_id: runtime.stage_run_unit_ids[0],
                scope_snapshot_id: runtime.scope_snapshot_id,
                organization_id: runtime.organization_ids[0],
                task_id: header.task_id,
            },
            limits: vec![
                (InvestigationFuelAxisV1::Campaign, 1),
                (InvestigationFuelAxisV1::PreparedAction, 1),
            ],
        },
    )
    .await
    .expect("create exact task-scoped fuel budget");

    let reserve_a = ReserveFuelInput {
        reservation_id: Uuid::new_v4(),
        event_id: Uuid::new_v4(),
        stable_request_id: Uuid::new_v4(),
        budget_id,
        axis: InvestigationFuelAxisV1::Campaign,
        amount: 1,
        work_key_sha256: digest('5'),
        expected_head_version: 0,
    };
    let reserve_b = ReserveFuelInput {
        reservation_id: Uuid::new_v4(),
        event_id: Uuid::new_v4(),
        stable_request_id: Uuid::new_v4(),
        budget_id,
        axis: InvestigationFuelAxisV1::Campaign,
        amount: 1,
        work_key_sha256: digest('6'),
        expected_head_version: 0,
    };
    let (first, second) = tokio::join!(
        investigation_fuel_ledger::reserve_fuel(db.pool(), &reserve_a),
        investigation_fuel_ledger::reserve_fuel(db.pool(), &reserve_b)
    );
    let (winner, loser) = match (first, second) {
        (Ok(winner), Err(loser)) | (Err(loser), Ok(winner)) => (winner, loser),
        other => panic!("exactly one concurrent reservation must win: {other:?}"),
    };
    assert!(matches!(
        loser,
        investigation_fuel_ledger::InvestigationFuelStoreError::CasConflict(_)
            | investigation_fuel_ledger::InvestigationFuelStoreError::Exhausted
    ));
    assert_eq!(winner.head.reserved_amount, 1);
    let consumed = investigation_fuel_ledger::transition_reservation(
        db.pool(),
        &TransitionFuelReservationInput {
            reservation_id: winner.reservation.reservation_id,
            event_id: Uuid::new_v4(),
            stable_request_id: Uuid::new_v4(),
            next_state: InvestigationFuelReservationStateV1::Consumed,
            expected_head_version: 1,
        },
    )
    .await
    .expect("consume durable campaign begin");
    assert_eq!(consumed.head.consumed_amount, 1);
    assert!(matches!(
        investigation_fuel_ledger::transition_reservation(
            db.pool(),
            &TransitionFuelReservationInput {
                reservation_id: winner.reservation.reservation_id,
                event_id: Uuid::new_v4(),
                stable_request_id: Uuid::new_v4(),
                next_state: InvestigationFuelReservationStateV1::RefundedBeforeBegin,
                expected_head_version: 2,
            },
        )
        .await,
        Err(investigation_fuel_ledger::InvestigationFuelStoreError::IllegalTransition)
    ));

    let prepared = investigation_fuel_ledger::reserve_fuel(
        db.pool(),
        &ReserveFuelInput {
            reservation_id: Uuid::new_v4(),
            event_id: Uuid::new_v4(),
            stable_request_id: Uuid::new_v4(),
            budget_id,
            axis: InvestigationFuelAxisV1::PreparedAction,
            amount: 1,
            work_key_sha256: digest('7'),
            expected_head_version: 0,
        },
    )
    .await
    .expect("reserve prepared-action fuel");
    let held = investigation_fuel_ledger::transition_reservation(
        db.pool(),
        &TransitionFuelReservationInput {
            reservation_id: prepared.reservation.reservation_id,
            event_id: Uuid::new_v4(),
            stable_request_id: Uuid::new_v4(),
            next_state: InvestigationFuelReservationStateV1::UnknownHeld,
            expected_head_version: 1,
        },
    )
    .await
    .expect("hold ambiguous prepared-action execution conservatively");
    assert_eq!(held.head.unknown_held_amount, 1);
    assert!(matches!(
        investigation_fuel_ledger::transition_reservation(
            db.pool(),
            &TransitionFuelReservationInput {
                reservation_id: prepared.reservation.reservation_id,
                event_id: Uuid::new_v4(),
                stable_request_id: Uuid::new_v4(),
                next_state: InvestigationFuelReservationStateV1::RefundedBeforeBegin,
                expected_head_version: 2,
            },
        )
        .await,
        Err(investigation_fuel_ledger::InvestigationFuelStoreError::IllegalTransition)
    ));

    let semantic_receipt = InvestigationSemanticCycleReceiptV1::host_create(
        digest('8'),
        digest('9'),
        digest('a'),
        digest('b'),
        digest('c'),
    )
    .expect("derive timestamp-independent semantic cycle");
    investigation_fuel_ledger::record_semantic_cycle(
        db.pool(),
        &RecordSemanticCycleInput {
            semantic_cycle_receipt_id: Uuid::new_v4(),
            stable_request_id: Uuid::new_v4(),
            task_id: header.task_id,
            receipt: semantic_receipt.clone(),
            disposition: SemanticCycleDisposition::FixedPoint,
            residual_reason_code: None,
            stop_receipt_id: None,
        },
    )
    .await
    .expect("record semantic fixed point");
    assert!(
        investigation_fuel_ledger::record_semantic_cycle(
            db.pool(),
            &RecordSemanticCycleInput {
                semantic_cycle_receipt_id: Uuid::new_v4(),
                stable_request_id: Uuid::new_v4(),
                task_id: header.task_id,
                receipt: semantic_receipt,
                disposition: SemanticCycleDisposition::FixedPoint,
                residual_reason_code: None,
                stop_receipt_id: None,
            },
        )
        .await
        .is_err(),
        "same task semantic fingerprint must never be appended twice"
    );
}

#[tokio::test]
#[serial]
async fn zero_campaign_closes_with_a_sealed_empty_outcome_set() {
    let (db, _data_dir) = migrated_db("zero-campaign").await;
    let runtime = runtime_fixture(db.pool(), "zero-campaign").await;
    investigation_main_sessions::register_stage_authority(
        db.pool(),
        &RegisterInvestigationStageAuthority {
            authority_id: runtime.authority_id,
            operation_id: runtime.operation_id,
            stage_execution_id: runtime.stage_execution_id,
            owning_stage_run_request_id: runtime.owning_request_id.clone(),
            scope_snapshot_id: runtime.scope_snapshot_id,
        },
    )
    .await
    .expect("register Investigation authority for zero-Campaign task");
    let hypothesis = seed_hypothesis_authority(db.pool(), &runtime).await;
    let header = HypothesisVerificationTaskHeaderV1::host_create(NewHypothesisVerificationTaskV1 {
        operation_id: runtime.operation_id,
        stage_execution_id: runtime.stage_execution_id,
        stage_run_unit_id: runtime.stage_run_unit_ids[0],
        organization_id: runtime.organization_ids[0],
        scope_snapshot_id: runtime.scope_snapshot_id,
        hypothesis_revision_id: hypothesis.revision_id,
        hypothesis_revision_sha256: digest(hypothesis.revision_hash_nibble),
        verification_plan_sha256: digest(hypothesis.plan_hash_nibble),
        relevant_evidence_snapshot_id: Uuid::new_v4(),
        semantic_evidence_set_sha256: digest('5'),
        open_obligation_set_sha256: digest('6'),
        semantic_attempt_fingerprint: digest('7'),
        first_admission_generation_id: hypothesis.generation_ids[0],
        host_rerun_receipt_id: None,
        host_rerun_receipt_sha256: None,
        rerun_contract_version: None,
    })
    .expect("derive zero-Campaign task identity");
    hypothesis_verification_tasks::create_or_replay_task(db.pool(), &header)
        .await
        .expect("create zero-Campaign task");

    let assignment_set_id = Uuid::new_v4();
    let assignment = hypothesis_verification_tasks::seal_objective_assignments(
        db.pool(),
        &SealObjectiveAssignmentsInput {
            assignment_set_id,
            stable_request_id: Uuid::new_v4(),
            task_id: header.task_id,
            members: vec![ObjectiveAssignmentMemberInput {
                assignment_member_id: Uuid::new_v4(),
                plan_objective_id: hypothesis.plan_objective_id,
                assignment: ObjectiveAssignmentInput::Residual {
                    residual_kind: TaskObjectiveResidualKindV1::NoKnownCapability,
                    reason_code: "no_campaign_capability".into(),
                    owner: "investigation_gate".into(),
                    next_action: "record_typed_residual".into(),
                    residual_receipt_id: Uuid::new_v4(),
                    residual_receipt_sha256: digest('8'),
                },
            }],
        },
    )
    .await
    .expect("seal terminal non-Campaign objective assignment");
    assert_eq!(assignment.member_count, Some(1));

    let outcome = hypothesis_verification_tasks::seal_campaign_outcomes(
        db.pool(),
        &SealCampaignOutcomesInput {
            outcome_set_id: Uuid::new_v4(),
            stable_request_id: Uuid::new_v4(),
            assignment_set_id,
            task_id: header.task_id,
            outcomes: vec![],
        },
    )
    .await
    .expect("seal the exact empty Campaign outcome set");
    let expected_empty_hash: String = sqlx::query_scalar(
        "SELECT unified_investigation_exact_set_hash(\
         'hypothesis_verification_task_outcomes.v1',ARRAY[]::TEXT[])",
    )
    .fetch_one(db.pool())
    .await
    .expect("derive canonical empty Campaign outcome hash");
    assert_eq!(outcome.status, "sealed");
    assert_eq!(outcome.member_count, Some(0));
    assert_eq!(
        outcome.member_set_sha256.as_deref(),
        Some(expected_empty_hash.as_str())
    );
    let persisted_members: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hypothesis_verification_task_outcome_members \
         WHERE outcome_set_id=$1",
    )
    .bind(outcome.outcome_set_id)
    .fetch_one(db.pool())
    .await
    .expect("read persisted empty Campaign outcome census");
    assert_eq!(persisted_members, 0);
}

#[tokio::test]
#[serial]
async fn promotion_requires_one_hash_bound_seven_component_census() {
    let (db, _data_dir) = migrated_db("promotion-components").await;
    let principal_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM operator_principals \
         WHERE principal_kind='local_operator' AND active",
    )
    .fetch_one(db.pool())
    .await
    .expect("load server-owned local operator");
    let mut hold_tx = db.pool().begin().await.expect("begin safety hold");
    set_operation_safety_hold(
        &mut hold_tx,
        SetOperationSafetyHold {
            scope: OperationSafetyHoldScope::OperationAdmission,
            next_held: true,
            expected_generation: 0,
            expected_row_version: 0,
            reason_code: "promotion_component_test".into(),
            evidence_manifest_hash: digest('a'),
            principal_id,
        },
    )
    .await
    .expect("hold operation admission before promotion");
    hold_tx.commit().await.expect("commit safety hold");

    let (campaign_generation, admission_generation, hold_version): (i64, i64, i64) =
        sqlx::query_as(
            "SELECT campaign_dispatch_generation,operation_admission_generation,row_version \
             FROM verification_campaign_safety_holds WHERE singleton=TRUE",
        )
        .fetch_one(db.pool())
        .await
        .expect("load frozen promotion safety generations");
    let tool_version: i64 =
        sqlx::query_scalar("SELECT row_version FROM tool_truth_rollout WHERE singleton=TRUE")
            .fetch_one(db.pool())
            .await
            .expect("load Tool Truth rollout CAS");
    let investigation_version: i64 =
        sqlx::query_scalar("SELECT row_version FROM investigation_rollout WHERE singleton=TRUE")
            .fetch_one(db.pool())
            .await
            .expect("load Investigation rollout CAS");
    let request = PromoteOperationDefaults {
        expected_safety_hold_row_version: hold_version,
        expected_campaign_dispatch_generation: campaign_generation,
        expected_operation_admission_generation: admission_generation,
        expected_tool_truth_row_version: tool_version,
        expected_investigation_row_version: investigation_version,
        target_joint_rank: 1,
        expected_evidence_manifest_hash: None,
        principal_id,
        reason: "prove exact component census".into(),
    };
    let mut missing_tx = db
        .pool()
        .begin()
        .await
        .expect("begin missing census promotion");
    let missing = promote_operation_defaults(&mut missing_tx, request.clone())
        .await
        .expect_err("promotion without a component census must fail closed");
    assert_eq!(missing.code(), "OPERATION_PROMOTION_COMPONENT_CENSUS");
    missing_tx
        .rollback()
        .await
        .expect("rollback missing census promotion");

    let component_sha256 = digest('b');
    let rows = OPERATION_PROMOTION_COMPONENT_KINDS
        .iter()
        .map(|kind| OperationPromotionComponentRow {
            component_kind: (*kind).into(),
            component_sha256: component_sha256.clone(),
            member_sha256: operation_promotion_component_member_hash(kind, &component_sha256),
        })
        .collect::<Vec<_>>();
    let component_set_sha256 = operation_promotion_component_set_hash(&rows);
    let census_id = Uuid::new_v4();
    let mut census_tx = db.pool().begin().await.expect("begin component census");
    sqlx::query(
        r#"INSERT INTO operation_default_promotion_component_censuses(
               census_id,criteria_version,component_member_count,
               component_set_sha256,sealed_by_principal_id
           ) VALUES($1,'operation_default_promotion.v2',7,$2,$3)"#,
    )
    .bind(census_id)
    .bind(&component_set_sha256)
    .bind(principal_id)
    .execute(&mut *census_tx)
    .await
    .expect("insert promotion component census header");
    for (ordinal, row) in rows.iter().enumerate() {
        sqlx::query(
            r#"INSERT INTO operation_default_promotion_component_members(
                   census_id,ordinal,component_kind,component_sha256,member_sha256
               ) VALUES($1,$2,$3,$4,$5)"#,
        )
        .bind(census_id)
        .bind(i16::try_from(ordinal).expect("seven ordinals fit i16"))
        .bind(&row.component_kind)
        .bind(&row.component_sha256)
        .bind(&row.member_sha256)
        .execute(&mut *census_tx)
        .await
        .expect("insert exact promotion component member");
    }
    census_tx
        .commit()
        .await
        .expect("seal exact promotion component census");

    let mut validated_tx = db.pool().begin().await.expect("begin validated promotion");
    let after_census = promote_operation_defaults(&mut validated_tx, request)
        .await
        .expect_err("edge evidence is intentionally absent after census validation");
    assert_eq!(after_census.code(), "OPERATION_PROMOTION_READINESS_RECEIPT");
    validated_tx
        .rollback()
        .await
        .expect("rollback intentionally incomplete edge evidence");

    let drift_census_id = Uuid::new_v4();
    let mut drift_tx = db.pool().begin().await.expect("begin drifting census");
    sqlx::query(
        r#"INSERT INTO operation_default_promotion_component_censuses(
               census_id,criteria_version,component_member_count,
               component_set_sha256,sealed_by_principal_id
           ) VALUES($1,'operation_default_promotion.v2',7,$2,$3)"#,
    )
    .bind(drift_census_id)
    .bind(digest('c'))
    .bind(principal_id)
    .execute(&mut *drift_tx)
    .await
    .expect("insert drift census header");
    for (ordinal, row) in rows.iter().enumerate() {
        sqlx::query(
            r#"INSERT INTO operation_default_promotion_component_members(
                   census_id,ordinal,component_kind,component_sha256,member_sha256
               ) VALUES($1,$2,$3,$4,$5)"#,
        )
        .bind(drift_census_id)
        .bind(i16::try_from(ordinal).expect("seven ordinals fit i16"))
        .bind(&row.component_kind)
        .bind(&row.component_sha256)
        .bind(&row.member_sha256)
        .execute(&mut *drift_tx)
        .await
        .expect("insert member beneath drifting set hash");
    }
    let drift = drift_tx
        .commit()
        .await
        .expect_err("deferred exact-set trigger must reject census hash drift");
    assert!(
        drift
            .to_string()
            .contains("OPERATION_PROMOTION_COMPONENT_SET_HASH_DRIFT"),
        "unexpected set-hash drift error: {drift}"
    );
}
