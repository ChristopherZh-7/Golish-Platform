const RUNTIME_MEMORY_PORT: &str =
    include_str!("../../golish-agent-kit/src/db_traits/runtime_memory.rs");
const RUNTIME_MEMORY_STORE: &str = include_str!("../src/repo/runtime_memory_tx.rs");
const ASSET_PRIMARY_MIGRATION: &str =
    include_str!("../migrations/20260813000005_investigation_asset_primary_scheduling.sql");
const PRIMARY_DYNAMIC_MIGRATION: &str =
    include_str!("../migrations/20260814000001_investigation_asset_primary_dynamic_schedule.sql");
const EFFECTIVE_CONTRACT_MIGRATION: &str =
    include_str!("../migrations/20260814000005_investigation_stage_team_effective_contract.sql");
const PRIMARY_REARM_MIGRATION: &str =
    include_str!("../migrations/20260814000006_investigation_asset_primary_execution_rearm.sql");
const PRIMARY_MULTI_REARM_MIGRATION: &str = include_str!(
    "../migrations/20260814000009_investigation_asset_primary_multi_execution_rearm.sql"
);
const GENERATOR_RECOVERY_MIGRATION: &str =
    include_str!("../migrations/20260814000008_investigation_generator_atomic_recovery.sql");
const DYNAMIC_DELEGATION_CENSUS_MIGRATION: &str =
    include_str!("../migrations/20260814000010_investigation_dynamic_delegation_census.sql");
const GENERATOR_ACTIVE_DENOMINATOR_MIGRATION: &str =
    include_str!("../migrations/20260814000011_investigation_generator_active_denominator.sql");
const ASSET_ROLE_DISPATCH_MIGRATION: &str =
    include_str!("../migrations/20260813000012_investigation_asset_role_dispatch_epoch_key.sql");

use golish_db::models::AgentType;
use golish_db::repo::runtime_memory_tx::{
    claim_stage_team_leader, claim_stage_work_item, close_stage_request_epoch,
    ensure_investigation_asset_primary_schedule, request_stage_worker, seed_stage_team_runtime,
    stage_worker_request_payload_hash, ClaimStageTeamLeaderRow, ClaimStageWorkItemRow,
    EnsureInvestigationAssetPrimaryScheduleRow, RequestStageWorkerRow, RuntimeMemoryTxFence,
    SeedStageRuntimeRow, SeedStageTeamRuntimeRow, StageTeamPlanSeedRow,
};
use golish_db::repo::tasks;
use golish_db::repo::unified_investigation_runtime::{
    AdoptInvestigationOrphanGeneratorInput, EnsureDynamicAssetAnalysisWorkInput,
    InvestigationGeneratorConsumerFenceInput, InvestigationGeneratorSubtaskInput,
    InvestigationStageIdentity, InvestigationUnitIdentity, MaterializeInvestigationGeneratorInput,
    PgUnifiedInvestigationRuntimeRepository, SealPentagiDelegationCensusInput,
};
use golish_db::{DbConfig, GolishDb};
use serde_json::json;
use serial_test::serial;
use std::sync::Arc;
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

fn unified_stable_id(namespace: Uuid, kind: &str, parts: &[&str]) -> Uuid {
    let mut material = format!("unified-investigation.v1\n{kind}");
    for part in parts {
        material.push('\n');
        material.push_str(part);
    }
    Uuid::new_v5(&namespace, material.as_bytes())
}

async fn migrated_db() -> (GolishDb, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("asset_primary_schedule_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start isolated migrated postgres");
    (db, data_dir)
}

#[test]
fn asset_primary_schedule_port_does_not_accept_a_caller_supplied_roster() {
    let request = RUNTIME_MEMORY_PORT
        .split_once("pub struct EnsureInvestigationAssetPrimarySchedule")
        .expect("asset Primary scheduling request DTO")
        .1
        .split_once('}')
        .expect("asset Primary scheduling request terminator")
        .0;

    for required in [
        "operation_id",
        "stage_execution_id",
        "stage_run_unit_id",
        "stage_team_plan_id",
        "asset_lane_id",
        "target_id",
        "asset_context_sha256",
    ] {
        assert!(request.contains(required), "request must bind {required}");
    }
    for required in [
        "predecessor_rearm_receipt_id",
        "execution_ordinal BETWEEN 1 AND 32",
        "investigation_asset_primary_continuation_receipt_sha256",
        "investigation_asset_primary_dispatch_in_current_lineage",
        "investigation-asset-primary-execution-continuation-v2",
        "investigation-task-primary-infrastructure-recovery:",
        "successor.predecessor_rearm_receipt_id=rearm.rearm_receipt_id",
    ] {
        assert!(
            PRIMARY_MULTI_REARM_MIGRATION.contains(required),
            "missing multi-generation Asset Primary rearm authority: {required}"
        );
    }
    assert!(
        !request.contains("roles") && !request.contains("roster"),
        "the server owns dynamic child creation; callers cannot preseed a roster"
    );
}

#[test]
fn forward_contract_freezes_primary_only_and_preserves_legacy_receipts() {
    assert!(ASSET_PRIMARY_MIGRATION.contains("investigation_asset_primary_schedules"));
    assert!(ASSET_PRIMARY_MIGRATION.contains("asset_context_sha256"));
    assert!(ASSET_PRIMARY_MIGRATION.contains("primary_message_chain_id"));
    assert!(ASSET_PRIMARY_MIGRATION.contains("roster_set_sha256"));
    assert!(ASSET_PRIMARY_MIGRATION.contains("UNIQUE(asset_lane_id,evolution_epoch)"));

    assert!(PRIMARY_DYNAMIC_MIGRATION.contains("primary_dynamic_v2"));
    assert!(PRIMARY_DYNAMIC_MIGRATION.contains("fixed_roster_v1"));
    for role in ["browser", "researcher", "pentester", "adviser"] {
        assert!(PRIMARY_DYNAMIC_MIGRATION
            .contains(&format!("ALTER COLUMN {role}_work_item_id DROP NOT NULL")));
    }
    assert!(PRIMARY_DYNAMIC_MIGRATION.contains("ALTER COLUMN roster_set_sha256 DROP NOT NULL"));
    assert!(PRIMARY_DYNAMIC_MIGRATION
        .contains("investigation_asset_primary_dynamic_schedule_receipt_sha256"));
    for authority in [
        "investigation_stage_team_effective_contracts",
        "source_plan_material",
        "effective_plan_material",
        "source_schedule_receipt_id",
        "investigation_contract_upgrade",
        "source_schedule.schedule_contract<>'fixed_roster_v1'",
        "worker.active_tool_call_id IS NOT NULL",
    ] {
        assert!(
            EFFECTIVE_CONTRACT_MIGRATION.contains(authority),
            "missing retained effective-contract authority: {authority}"
        );
    }
    assert!(
        !EFFECTIVE_CONTRACT_MIGRATION.contains("enforce_stage_work_item_contract"),
        "effective contract upgrades the canonical plan; it must not create a shadow WorkItem policy"
    );
    for required in [
        "investigation_asset_primary_rearms",
        "source_exhaustion_output_sha256",
        "investigation_asset_primary_current_authorities",
        "investigation_stage_team_effective_contracts effective",
        "investigation_asset_primary_rearms_complete",
    ] {
        assert!(
            PRIMARY_REARM_MIGRATION.contains(required),
            "missing exhausted Asset Primary rearm authority: {required}"
        );
    }
    assert!(RUNTIME_MEMORY_STORE.contains("investigation_asset_primary_rearm_fuel_exhausted"));
    assert!(include_str!("../src/repo/stage_teams.rs").contains(
        "ROW(schedule.primary_work_item_id,\n                                                   schedule.primary_worker_run_id)"
    ));
}

#[test]
fn runtime_store_seeds_only_primary_and_reuses_the_asset_chain() {
    let ensure = RUNTIME_MEMORY_STORE
        .split_once("pub async fn ensure_investigation_asset_primary_schedule")
        .expect("asset Primary scheduling transaction")
        .1
        .split_once("pub async fn rearm_investigation_task_primary")
        .expect("asset Primary scheduling transaction terminator")
        .0;

    assert!(!ensure.contains("for (priority, (role, role_work_item_id))"));
    assert!(!ensure.contains("investigation_asset_role"));
    assert!(ensure.contains("required_for_barrier: false"));
    assert!(ensure.contains("investigation_asset_primary"));
    for exact_legacy_cutover_binding in [
        "legacy_receipt.target_id != input.target_id",
        "legacy_receipt.asset_context_sha256 != input.asset_context_sha256",
        "legacy_receipt.stage_team_plan_id != plan.id",
        "legacy_receipt.operation_id != plan.operation_id",
        "legacy_receipt.stage_execution_id != plan.stage_execution_id",
        "legacy_receipt.stage_run_unit_id != plan.stage_run_unit_id",
        "legacy_receipt.scope_snapshot_id != plan.scope_snapshot_id",
        "legacy_receipt.organization_id != plan.organization_id",
        "legacy_receipt.resume_dispatch_epoch != plan.dispatch_epoch",
    ] {
        assert!(
            ensure.contains(exact_legacy_cutover_binding),
            "legacy cutover must bind {exact_legacy_cutover_binding} before superseding rows"
        );
    }
    assert!(ensure.contains("investigation_asset_legacy_schedule_cutover_authority_mismatch"));
    assert!(ensure.contains("investigation_asset_legacy_schedule_cutover_authority_ambiguous"));
    assert!(
        !ensure.contains("let legacy_fixed_exists: bool"),
        "a boolean existence probe cannot authorize a shared-plan asset cutover"
    );
    assert!(
        RUNTIME_MEMORY_STORE.contains("investigation-asset-primary-chain-v1"),
        "all epochs for one asset lane must reuse its deterministic Primary chain"
    );
    assert!(RUNTIME_MEMORY_STORE
        .contains("worker.status NOT IN('passed','failed','exhausted','superseded')"));
    assert!(RUNTIME_MEMORY_STORE.contains("live_chain_owners.len() > 1"));
    assert!(
        !RUNTIME_MEMORY_STORE.contains(
            "WHERE worker.message_chain_id=$1 FOR UPDATE\""
        ),
        "terminal Analysis and Verification Primary workers are durable chain history, not live owners"
    );
    let audit_filter = RUNTIME_MEMORY_STORE
        .split_once("async fn investigation_legacy_fixed_schedule_item_is_audit_only")
        .expect("legacy fixed receipt audit filter")
        .1
        .split_once("async fn investigation_governance_plan_replay_is_authorized")
        .expect("legacy audit filter terminator")
        .0;
    for exact_binding in [
        "schedule.schedule_contract='fixed_roster_v1'",
        "schedule.stage_team_plan_id=$1",
        "schedule.resume_dispatch_epoch=$7",
        "schedule.primary_work_item_id",
        "schedule.browser_work_item_id",
        "schedule.adviser_work_item_id",
    ] {
        assert!(audit_filter.contains(exact_binding));
    }
    let dynamic_filter = RUNTIME_MEMORY_STORE
        .split_once("async fn investigation_dynamic_primary_schedule_item_is_authorized")
        .expect("dynamic Primary receipt authority")
        .1
        .split_once("async fn investigation_governance_plan_replay_is_authorized")
        .expect("dynamic Primary authority terminator")
        .0;
    for exact_binding in [
        "FROM investigation_asset_primary_current_authorities schedule",
        "schedule.stage_team_plan_id=$1",
        "schedule.resume_dispatch_epoch=$7",
        "schedule.primary_work_item_id=$8",
    ] {
        assert!(dynamic_filter.contains(exact_binding));
    }
}

#[test]
fn migration_guards_current_lane_owner_and_exact_replay() {
    for guard in [
        "investigation_asset_lanes",
        "target_identity_sha256",
        "lane.state<>'analyzing'",
        "stage_team_plan_id",
        "scope_snapshot_id",
        "organization_id",
        "status='building'",
        "status='applied'",
    ] {
        assert!(
            ASSET_PRIMARY_MIGRATION.contains(guard),
            "missing scheduling authority guard: {guard}"
        );
    }
    assert!(ASSET_PRIMARY_MIGRATION.contains("APPEND_ONLY"));
    assert!(ASSET_PRIMARY_MIGRATION.contains("REPLAY"));
    assert!(ASSET_ROLE_DISPATCH_MIGRATION
        .contains("INVESTIGATION_ASSET_ROLE_LOGICAL_DISPATCH_AUTHORITY_MISMATCH"));
    assert!(ASSET_ROLE_DISPATCH_MIGRATION.contains("stage_worker_request_id IS NULL"));
    assert!(ASSET_ROLE_DISPATCH_MIGRATION.contains("investigation_asset_primary_schedules"));
    assert!(ASSET_ROLE_DISPATCH_MIGRATION.contains("server_phase_transition"));
    assert!(
        ASSET_ROLE_DISPATCH_MIGRATION.contains("subtask.label,':',schedule.evolution_epoch::TEXT")
    );
}

#[test]
fn dynamic_refiner_v2_has_zero_add_drop_reorder_and_final_denominator_contracts() {
    for authority in [
        "create_investigation_refiner_plan_ledger_v2",
        "append_investigation_refiner_plan_patch_v2",
        "seal_investigation_refiner_plan_ledger_v2",
        "dynamic_ordered_v2",
        "p_ordered_active_subtask_ids",
        "WITH ORDINALITY",
        "final_patch.active_realized_subtask_count",
        "INVESTIGATION_REFINER_V2_ASSET_AUTHORITY_MISMATCH",
        "INVESTIGATION_REFINER_V2_PATCH_REPLAY_MISMATCH",
    ] {
        assert!(
            PRIMARY_DYNAMIC_MIGRATION.contains(authority),
            "missing dynamic Refiner authority: {authority}"
        );
    }
    assert!(PRIMARY_DYNAMIC_MIGRATION.contains("CHECK(generator_subtask_count>=0)"));
    assert!(
        !PRIMARY_DYNAMIC_MIGRATION.contains("ACTIVE_SUBTASK_SET_REGRESSED"),
        "v2 must permit an ordered active set to drop a prior member"
    );
    for required in [
        "investigation_refiner_primary_source_is_current_v3",
        "investigation_asset_primary_current_authorities",
        "investigation_asset_primary_rearms ancestor_rearm",
        "investigation_generator_source_receipts",
        "orphan_adoption",
        "CREATE OR REPLACE FUNCTION create_investigation_refiner_plan_ledger_v2",
        "CREATE OR REPLACE FUNCTION append_investigation_refiner_plan_patch_v2",
        "CREATE OR REPLACE FUNCTION seal_investigation_refiner_plan_ledger_v2",
    ] {
        assert!(
            GENERATOR_RECOVERY_MIGRATION.contains(required),
            "missing atomic Generator recovery authority: {required}"
        );
    }
}

#[test]
fn dynamic_delegation_census_counts_only_exact_executed_members() {
    for authority in [
        "investigation_effective_delegation_census_v2",
        "ledger.ledger_contract='dynamic_ordered_v2'",
        "seal.final_active_realized_subtask_count",
        "final_active_count<>0",
        "barrier.subtask_id=dispatch.subtask_id",
        "barrier.actor_worker_run_id=dispatch.worker_run_id",
        "barrier.parent_dispatch_receipt_id=dispatch.dispatch_receipt_id",
        "attempt.result_sha256=barrier.event_sha256",
        "dispatch.worker_run_id<>actual_primary_worker",
        "synthesis.event_kind='primary_synthesis'",
        "synthesis.event_sha256=primary_attempt.result_sha256",
        "PENTAGI_DYNAMIC_WORKER_DISPATCH_REQUIRES_RESULT_BARRIER",
        "PENTAGI_DYNAMIC_RESULT_BARRIER_REQUIRES_WORKER_DISPATCH",
        "PENTAGI_DYNAMIC_SUBTASK_NEVER_ENTERED_ACTIVE_DENOMINATOR",
        "PENTAGI_RUNNABLE_SUBTASK_REQUIRES_DISTINCT_WORKER",
        "pentagi_plan_open_or_nonnegative_sealed",
        "subtask_count>=0",
    ] {
        assert!(
            DYNAMIC_DELEGATION_CENSUS_MIGRATION.contains(authority),
            "missing dynamic delegation census authority: {authority}"
        );
    }
    for authority in [
        "subtask.subtask_ordinal<generator.generator_subtask_count",
        "generator.generator_subtask_set_sha256=",
        "investigation_refiner_generator_subtasks.v2",
        "investigation_refiner_plan_patch_members member",
    ] {
        assert!(
            GENERATOR_ACTIVE_DENOMINATOR_MIGRATION.contains(authority),
            "missing Generator initial active-denominator authority: {authority}"
        );
    }
    let replay_guard = RUNTIME_MEMORY_STORE
        .split_once("async fn investigation_governance_plan_replay_is_authorized")
        .expect("Investigation governance replay guard")
        .1
        .split_once("fn investigation_task_primary_work_item_id")
        .expect("Investigation governance replay guard terminator")
        .0;
    assert!(replay_guard.contains("for item in existing_items"));
    assert!(replay_guard.contains("validate_stage_team_replay_extra"));
    assert!(replay_guard.contains("investigation_asset_primary_current_authorities"));
    assert!(
        !replay_guard.contains("return Ok(existing_items.is_empty())"),
        "closed dynamic plans must not bypass exact item authorities"
    );
    for virgin_governance_fence in [
        "plan.requests_closed_at.is_none()",
        "plan.dispatch_epoch != 0",
        "FROM stage_worker_requests request WHERE request.team_plan_id=$1",
        "FROM investigation_asset_primary_schedules schedule",
    ] {
        assert!(replay_guard.contains(virgin_governance_fence));
    }
    let accepted_replay = RUNTIME_MEMORY_STORE
        .split_once("\"accepted_worker_request\" =>")
        .expect("accepted dynamic child replay branch")
        .1
        .split_once("\"target_intel_review_freeze\" =>")
        .expect("accepted dynamic child replay terminator")
        .0;
    for historical_parent_fence in [
        "investigation_asset_primary_binding(plan, &parent_item)",
        "investigation_dynamic_primary_schedule_item_is_authorized",
        "investigation_historical_asset_primary_binding(plan, &parent_item)",
    ] {
        assert!(accepted_replay.contains(historical_parent_fence));
    }
    assert!(RUNTIME_MEMORY_STORE.contains("source_epoch_plan.dispatch_epoch = item.dispatch_epoch"));
    for required in [
        "claim_closed_investigation_asset_post_synthesis_primary",
        "primary_attempt.result_sha256=synthesis.event_sha256",
        "consuming ordinary producer retry fuel here",
        "investigation_closed_post_synthesis_primary_claim_busy",
    ] {
        assert!(
            RUNTIME_MEMORY_STORE.contains(required),
            "missing exact closed post-synthesis Primary recovery guard: {required}"
        );
    }
}

#[tokio::test]
#[serial]
async fn ensure_seeds_primary_only_and_primary_can_request_asset_bound_children() {
    let (mut db, _data_dir) = migrated_db().await;
    let session_id = Uuid::new_v4();
    let operation_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    let stage_run_unit_id = Uuid::new_v4();
    let scope_snapshot_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let authority_id = Uuid::new_v4();
    let company_queue_id = Uuid::new_v4();
    let company_member_id = Uuid::new_v4();
    let asset_queue_id = Uuid::new_v4();
    let asset_lane_id = Uuid::new_v4();
    let target_id = Uuid::new_v4();
    let sibling_asset_lanes = [
        (Uuid::new_v4(), Uuid::new_v4(), digest('b')),
        (Uuid::new_v4(), Uuid::new_v4(), digest('c')),
    ];
    let stage_team_plan_id = Uuid::new_v5(&stage_run_unit_id, b"stage-team-plan:v1");
    let asset_context_sha256 = digest('a');
    let mut tx = db.pool().begin().await.expect("begin schedule fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("isolate unrelated fixture triggers");
    sqlx::query("INSERT INTO sessions(id,title,status) VALUES($1,'asset schedule','running')")
        .bind(session_id)
        .execute(&mut *tx)
        .await
        .expect("insert session");
    sqlx::query(
        "INSERT INTO tasks(id,session_id,input,status) VALUES($1,$2,'asset schedule','running')",
    )
    .bind(operation_id)
    .bind(session_id)
    .execute(&mut *tx)
    .await
    .expect("insert task");
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
    .expect("insert operation");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_snapshots(
               id,operation_id,project_scope_id,scope_decision_id,project_path_at_freeze,
               root_organization_id,mode,scope_hash,sealed_at)
           VALUES($1,$2,$3,$4,'/tmp/asset-schedule',$5,'cli_flags',$6,NOW())"#,
    )
    .bind(scope_snapshot_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(Uuid::new_v4())
    .bind(organization_id)
    .bind(digest('1'))
    .execute(&mut *tx)
    .await
    .expect("insert scope snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units(
               snapshot_id,organization_id,organization_name_at_freeze,role,depth,ordinal,
               decision_row_id,approval_source)
           VALUES($1,$2,'Asset Fixture','root',0,0,'root','{}'::JSONB)"#,
    )
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .execute(&mut *tx)
    .await
    .expect("insert scope unit");
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status,stage_topology_contract)
         VALUES($1,$2,'investigation','started','unified_investigation_v1')",
    )
    .bind(stage_execution_id)
    .bind(operation_id)
    .execute(&mut *tx)
    .await
    .expect("insert stage run");
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
    .expect("insert stage unit");
    sqlx::query(
        r#"INSERT INTO investigation_run_heads(
               authority_id,stable_start_request_id,operation_id,stage_execution_id,
               owning_stage_run_request_id,scope_snapshot_id,run_state,admission_open,
               stop_epoch,change_seq,head_version,head_sha256)
           VALUES($1,$2,$3,$4,'asset-schedule',$5,'running',TRUE,0,0,0,
                  unified_investigation_runtime_head_sha256($1,'running',TRUE,0,0,0))"#,
    )
    .bind(authority_id)
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(scope_snapshot_id)
    .execute(&mut *tx)
    .await
    .expect("insert run head");
    sqlx::query(
        r#"INSERT INTO investigation_company_queues(
               company_queue_id,stable_freeze_request_id,authority_id,operation_id,
               stage_execution_id,owning_stage_run_request_id,scope_snapshot_id,
               member_count,member_set_sha256,max_evolution_epochs)
           VALUES($1,$2,$3,$4,$5,'asset-schedule',$6,1,$7,2)"#,
    )
    .bind(company_queue_id)
    .bind(Uuid::new_v4())
    .bind(authority_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(scope_snapshot_id)
    .bind(digest('2'))
    .execute(&mut *tx)
    .await
    .expect("insert company queue");
    sqlx::query(
        r#"INSERT INTO investigation_company_queue_members(
               company_member_id,company_queue_id,authority_id,operation_id,
               stage_execution_id,scope_snapshot_id,organization_id,
               organization_name_at_freeze,depth,ordinal,state)
           VALUES($1,$2,$3,$4,$5,$6,$7,'Asset Fixture',0,0,'active')"#,
    )
    .bind(company_member_id)
    .bind(company_queue_id)
    .bind(authority_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .execute(&mut *tx)
    .await
    .expect("insert company member");
    sqlx::query(
        r#"INSERT INTO investigation_asset_queues(
               asset_queue_id,company_queue_id,company_member_id,authority_id,operation_id,
               stage_execution_id,scope_snapshot_id,organization_id,member_count,
               member_set_sha256)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,3,$9)"#,
    )
    .bind(asset_queue_id)
    .bind(company_queue_id)
    .bind(company_member_id)
    .bind(authority_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(digest('3'))
    .execute(&mut *tx)
    .await
    .expect("insert asset queue");
    sqlx::query(
        r#"INSERT INTO investigation_asset_lanes(
               asset_lane_id,asset_queue_id,company_queue_id,company_member_id,authority_id,
               operation_id,stage_execution_id,scope_snapshot_id,organization_id,target_id,
               target_type_at_freeze,target_value_at_freeze,target_source_at_freeze,
               target_created_at,target_identity_sha256,ordinal,state,evolution_epoch,
               max_evolution_epochs)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'domain','asset.example','manual',
                  NOW(),$11,0,'analyzing',0,2)"#,
    )
    .bind(asset_lane_id)
    .bind(asset_queue_id)
    .bind(company_queue_id)
    .bind(company_member_id)
    .bind(authority_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(target_id)
    .bind(&asset_context_sha256)
    .execute(&mut *tx)
    .await
    .expect("insert active asset lane");
    for (offset, (sibling_lane_id, sibling_target_id, sibling_context_sha256)) in
        sibling_asset_lanes.iter().enumerate()
    {
        sqlx::query(
            r#"INSERT INTO investigation_asset_lanes(
                   asset_lane_id,asset_queue_id,company_queue_id,company_member_id,authority_id,
                   operation_id,stage_execution_id,scope_snapshot_id,organization_id,target_id,
                   target_type_at_freeze,target_value_at_freeze,target_source_at_freeze,
                   target_created_at,target_identity_sha256,ordinal,state,evolution_epoch,
                   max_evolution_epochs)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'domain',$11,'manual',NOW(),$12,$13,
                      'queued',0,2)"#,
        )
        .bind(sibling_lane_id)
        .bind(asset_queue_id)
        .bind(company_queue_id)
        .bind(company_member_id)
        .bind(authority_id)
        .bind(operation_id)
        .bind(stage_execution_id)
        .bind(scope_snapshot_id)
        .bind(organization_id)
        .bind(sibling_target_id)
        .bind(format!("sibling-{}.asset.example", offset + 1))
        .bind(sibling_context_sha256)
        .bind(i32::try_from(offset + 1).expect("sibling asset ordinal"))
        .execute(&mut *tx)
        .await
        .expect("insert queued sibling asset lane");
    }
    tx.commit().await.expect("commit schedule fixture");

    let governance_seed = SeedStageTeamRuntimeRow {
        base: SeedStageRuntimeRow {
            operation_id,
            stage_execution_id,
            stage_kind: "investigation".to_string(),
            unit_generation: 0,
            specialist: "investigation".to_string(),
            worker_generation: 0,
            work_item_kind: "organization".to_string(),
            work_item_key: "investigation".to_string(),
            agent_path_prefix: "main>stage_run:investigation".to_string(),
            organization_ids: None,
        },
        plan: StageTeamPlanSeedRow {
            schema_version: 1,
            plan_version: 1,
            plan_hash: digest('4'),
            leader_role: "investigation".to_string(),
            allowed_roles: [
                "investigation",
                "browser",
                "researcher",
                "pentester",
                "adviser",
            ]
            .map(str::to_string)
            .to_vec(),
            aggregator_kind: "worker".to_string(),
            aggregator_role: Some("investigation".to_string()),
            max_workers_total: 200,
            max_workers_active: 4,
            dynamic_requests_enabled: true,
            dynamic_request_policy: json!({
                "allowed_request_kinds": ["analysis_task", "cognitive_support", "verification_task"],
                "canonical_subject_refs_only": true,
                "child_budget": {},
                "child_output_schema": "investigation_cognitive_output.v1",
                "coordination_mode": "investigation_task_orchestrator",
                "global_provider_cap": 4,
                "max_company_units_active": 1,
                "max_controller_gate_repairs": 2,
                "max_controller_turn_resumes": 2,
                "max_repair_generations": 2,
                "max_requests": 64,
                "max_subject_refs": 32,
                "organization_scope_implicit": true
            }),
            final_submitter_kind: "worker".to_string(),
            created_from_stage_spec_hash: digest('5'),
        },
        work_items: Vec::new(),
    };
    let governed = seed_stage_team_runtime(db.pool(), &governance_seed)
        .await
        .expect("seed closed governance-only Investigation plan");
    let [governed] = governed.as_slice() else {
        panic!("one frozen organization must produce one governance plan")
    };
    assert_eq!(governed.plan.id, stage_team_plan_id);
    assert!(governed.plan.requests_closed_at.is_some());
    assert!(governed.work_items.is_empty());
    let governance_replay = seed_stage_team_runtime(db.pool(), &governance_seed)
        .await
        .expect("replay exact closed governance plan");
    assert!(governance_replay[0].replayed);
    assert!(governance_replay[0].plan.requests_closed_at.is_some());
    assert!(governance_replay[0].work_items.is_empty());

    // Migration-forward fixture: an old fixed-roster round already occupies
    // this lane/epoch and its Browser failed. It is audit-only; the new ensure
    // must create a fresh dynamic round and reuse only the durable chain.
    let legacy_receipt_id = Uuid::new_v5(
        &asset_lane_id,
        b"investigation-asset-primary-schedule-receipt-v1:0",
    );
    let legacy_primary_item_id = Uuid::new_v5(
        &asset_lane_id,
        b"investigation-asset-primary-work-item-v1:0",
    );
    let legacy_primary_worker_id =
        Uuid::new_v5(&asset_lane_id, b"investigation-asset-primary-worker-v1:0");
    let primary_chain_id = Uuid::new_v5(&asset_lane_id, b"investigation-asset-primary-chain-v1");
    let mut legacy_tx = db.pool().begin().await.expect("begin legacy fixed round");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *legacy_tx)
        .await
        .expect("seed immutable legacy authority");
    sqlx::query(
        "INSERT INTO message_chains(id,session_id,task_id,agent,chain)
         VALUES($1,$2,$3,'primary','[]'::JSONB)",
    )
    .bind(primary_chain_id)
    .bind(session_id)
    .bind(operation_id)
    .execute(&mut *legacy_tx)
    .await
    .expect("seed legacy durable Primary chain");
    let legacy_roles = ["browser", "researcher", "pentester", "adviser"];
    let legacy_role_ids = legacy_roles.map(|role| {
        Uuid::new_v5(
            &asset_lane_id,
            format!("investigation-asset-role-work-item-v1:0:{role}").as_bytes(),
        )
    });
    sqlx::query(
        r#"INSERT INTO stage_work_items(
               id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
               input_manifest_hash,input_refs,required_for_barrier,priority,status,
               attempt_policy,budget,output_schema,created_by,terminal_at)
           VALUES($1,$2,$3,$4,$5,$6,$7,1,'investigation_asset_primary',$8,
                  'investigation',$9,$10,FALSE,0,'waiting_dependency','{"max_attempts":3}',
                  '{}','stage_unit_aggregate.v1','server_phase_transition',NULL)"#,
    )
    .bind(legacy_primary_item_id)
    .bind(stage_team_plan_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(format!("asset:{asset_lane_id}:primary:0"))
    .bind(&asset_context_sha256)
    .bind(
        json!([{"kind":"investigation_asset_lane","asset_lane_id":asset_lane_id,
                  "target_id":target_id,"asset_context_sha256":asset_context_sha256,
                  "evolution_epoch":0}]),
    )
    .execute(&mut *legacy_tx)
    .await
    .expect("seed legacy Primary item");
    for (priority, (role, role_id)) in legacy_roles.iter().zip(legacy_role_ids).enumerate() {
        sqlx::query(
            r#"INSERT INTO stage_work_items(
                   id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                   scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
                   input_manifest_hash,input_refs,required_for_barrier,priority,status,
                   attempt_policy,budget,output_schema,created_by,terminal_at)
               VALUES($1,$2,$3,$4,$5,$6,$7,1,'investigation_asset_role',$8,$9,$10,
                      $11,TRUE,$12,$13,'{"max_attempts":3}','{}',
                      'investigation_cognitive_output.v1','server_phase_transition',$14)"#,
        )
        .bind(role_id)
        .bind(stage_team_plan_id)
        .bind(operation_id)
        .bind(stage_execution_id)
        .bind(stage_run_unit_id)
        .bind(scope_snapshot_id)
        .bind(organization_id)
        .bind(format!("asset:{asset_lane_id}:role:{role}:0"))
        .bind(*role)
        .bind(&asset_context_sha256)
        .bind(
            json!([{"kind":"investigation_asset_lane","asset_lane_id":asset_lane_id,
                      "target_id":target_id,"asset_context_sha256":asset_context_sha256,
                      "evolution_epoch":0,"role_slot":role}]),
        )
        .bind(i32::try_from(priority + 1).expect("legacy priority"))
        .bind(if *role == "browser" {
            "exhausted"
        } else {
            "queued"
        })
        .bind((*role == "browser").then(chrono::Utc::now))
        .execute(&mut *legacy_tx)
        .await
        .expect("seed exhausted legacy role");
    }
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,organization_id,work_item_id,
               worker_generation,specialist,work_item_kind,work_item_key,agent_path,
               message_chain_id,status)
           VALUES($1,$2,$3,$4,$5,$6,0,'investigation','investigation_asset_primary',$7,
                  'legacy-asset-primary',$8,'waiting_background')"#,
    )
    .bind(legacy_primary_worker_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(organization_id)
    .bind(legacy_primary_item_id)
    .bind(format!("asset:{asset_lane_id}:primary:0"))
    .bind(primary_chain_id)
    .execute(&mut *legacy_tx)
    .await
    .expect("seed exhausted legacy Primary worker");
    sqlx::query(
        r#"INSERT INTO investigation_asset_primary_schedules(
               schedule_receipt_id,stable_request_id,asset_lane_id,target_id,
               asset_context_sha256,evolution_epoch,schedule_round,schedule_contract,
               stage_team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,source_dispatch_epoch,resume_dispatch_epoch,
               source_plan_row_version,primary_work_item_id,primary_worker_run_id,
               primary_message_chain_id,browser_work_item_id,researcher_work_item_id,
               pentester_work_item_id,adviser_work_item_id,roster_set_sha256,
               receipt_sha256,status,applied_at)
           VALUES($1,$2,$3,$4,$5,0,0,'fixed_roster_v1',$6,$7,$8,$9,$10,$11,0,1,1,
                  $12,$13,$14,$15,$16,$17,$18,$19,$20,'applied',NOW())"#,
    )
    .bind(legacy_receipt_id)
    .bind(Uuid::new_v4())
    .bind(asset_lane_id)
    .bind(target_id)
    .bind(&asset_context_sha256)
    .bind(stage_team_plan_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(legacy_primary_item_id)
    .bind(legacy_primary_worker_id)
    .bind(primary_chain_id)
    .bind(legacy_role_ids[0])
    .bind(legacy_role_ids[1])
    .bind(legacy_role_ids[2])
    .bind(legacy_role_ids[3])
    .bind(digest('6'))
    .bind(digest('7'))
    .execute(&mut *legacy_tx)
    .await
    .expect("seed immutable legacy fixed receipt");
    sqlx::query(
        "UPDATE stage_team_plans SET dispatch_epoch=1,requests_closed_at=NULL,
                row_version=2,updated_at=NOW() WHERE id=$1",
    )
    .bind(stage_team_plan_id)
    .execute(&mut *legacy_tx)
    .await
    .expect("close legacy dispatch epoch");
    legacy_tx.commit().await.expect("commit legacy fixed round");

    // Exact retained-DB reproduction: freeze the five-role source contract,
    // upgrade only its canonical plan material under the append-only 00005
    // authority, then let every generic plan/work-item path read the same
    // effective nine-role contract.
    let mut current_governance_seed = governance_seed.clone();
    current_governance_seed.plan.plan_hash =
        "sha256:03399895cd72367a2c25c3ea497954bc9d7747d4b7c922ffb0e514ff93ada979".to_string();
    current_governance_seed.plan.created_from_stage_spec_hash =
        "sha256:6c469929b3961ba3a1412201b16640ad75374883e245abd87414da4b013f9821".to_string();
    current_governance_seed.plan.allowed_roles = [
        "adviser",
        "browser",
        "coder",
        "enricher",
        "installer",
        "investigation",
        "memorist",
        "pentester",
        "researcher",
    ]
    .map(str::to_string)
    .to_vec();
    current_governance_seed.plan.max_workers_total = 200;
    current_governance_seed.plan.max_workers_active = 8;
    current_governance_seed.plan.dynamic_request_policy = json!({
        "allowed_request_kinds": ["analysis_task", "cognitive_support", "verification_task"],
        "canonical_subject_refs_only": true,
        "child_budget": {},
        "child_output_schema": "investigation_cognitive_output.v1",
        "coordination_mode": "investigation_task_orchestrator",
        "global_provider_cap": 4,
        "max_company_units_active": 1,
        "max_controller_gate_repairs": 2,
        "max_controller_turn_resumes": 2,
        "max_repair_generations": 2,
        "max_requests": 64,
        "max_subject_refs": 32,
        "organization_scope_implicit": true
    });
    let contract_authority_id = Uuid::new_v5(
        &stage_team_plan_id,
        format!(
            "investigation-stage-team-effective-contract-v1:{}:{}",
            current_governance_seed.plan.plan_hash,
            current_governance_seed.plan.created_from_stage_spec_hash
        )
        .as_bytes(),
    );
    let retained_replay = seed_stage_team_runtime(db.pool(), &current_governance_seed)
        .await
        .expect("current governance seed must atomically upgrade and replay retained plan");
    assert!(retained_replay[0].replayed);
    assert_eq!(retained_replay[0].plan.id, stage_team_plan_id);
    assert_eq!(
        retained_replay[0].plan.plan_hash,
        current_governance_seed.plan.plan_hash
    );
    assert!(retained_replay[0].work_items.is_empty());
    let effective_contract: (String, String, String, i64) = sqlx::query_as(
        "SELECT status,source_plan_hash,effective_plan_hash,
                (SELECT row_version FROM stage_team_plans WHERE id=stage_team_plan_id)
           FROM investigation_stage_team_effective_contracts
          WHERE contract_authority_id=$1",
    )
    .bind(contract_authority_id)
    .fetch_one(db.pool())
    .await
    .expect("load applied retained effective contract");
    assert_eq!(
        effective_contract,
        (
            "applied".to_string(),
            digest('4'),
            current_governance_seed.plan.plan_hash.clone(),
            3,
        )
    );
    let authority_drift = sqlx::query(
        "UPDATE investigation_stage_team_effective_contracts
            SET effective_plan_hash=$2 WHERE contract_authority_id=$1",
    )
    .bind(contract_authority_id)
    .bind(digest('8'))
    .execute(db.pool())
    .await
    .expect_err("applied effective contract must remain immutable");
    assert!(authority_drift
        .to_string()
        .contains("INVESTIGATION_EFFECTIVE_CONTRACT_APPEND_ONLY"));
    let downgrade = sqlx::query(
        "UPDATE stage_team_plans SET plan_hash=$2,created_from_stage_spec_hash=$3,
             allowed_worker_roles=$4,max_workers_active=4,
             row_version=row_version+1,updated_at=NOW() WHERE id=$1",
    )
    .bind(stage_team_plan_id)
    .bind(digest('4'))
    .bind(digest('5'))
    .bind(json!(governance_seed.plan.allowed_roles))
    .execute(db.pool())
    .await
    .expect_err("an applied authority cannot authorize a downgrade");
    assert!(downgrade.to_string().contains("STAGE_TEAM_PLAN_IMMUTABLE"));
    let authority_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM investigation_stage_team_effective_contracts
          WHERE stage_team_plan_id=$1 AND status='applied'",
    )
    .bind(stage_team_plan_id)
    .fetch_one(db.pool())
    .await
    .expect("count immutable effective contract");
    assert_eq!(authority_count, 1);

    let request = EnsureInvestigationAssetPrimaryScheduleRow {
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        stage_team_plan_id,
        asset_lane_id,
        target_id,
        asset_context_sha256: asset_context_sha256.clone(),
    };
    let same_operation_asset_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM investigation_asset_lanes
          WHERE operation_id=$1 AND asset_queue_id=$2",
    )
    .bind(operation_id)
    .bind(asset_queue_id)
    .fetch_one(db.pool())
    .await
    .expect("count same-operation asset lanes");
    assert_eq!(same_operation_asset_count, 3);
    // One Investigation company plan is reused across its ordered asset lanes.
    // Model an out-of-order resume after that shared plan has advanced across
    // two sibling assets: the old receipt for this lane is bound to epoch 1,
    // while the live plan is at epoch 3. The ensure transaction must reject
    // before superseding any WorkItem/Worker or inserting a dynamic receipt.
    let mut out_of_order_fixture_tx = db
        .pool()
        .begin()
        .await
        .expect("begin out-of-order plan fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *out_of_order_fixture_tx)
        .await
        .expect("isolate out-of-order fixture from plan transition triggers");
    sqlx::query(
        "UPDATE stage_team_plans SET dispatch_epoch=3,row_version=4,updated_at=NOW()
          WHERE id=$1 AND dispatch_epoch=1 AND row_version=3",
    )
    .bind(stage_team_plan_id)
    .execute(&mut *out_of_order_fixture_tx)
    .await
    .expect("simulate shared plan advanced to the third asset");
    out_of_order_fixture_tx
        .commit()
        .await
        .expect("commit out-of-order plan fixture");
    let out_of_order_before: (String, String, String, String) = sqlx::query_as(
        r#"SELECT
             tool_truth_sha256(COALESCE((SELECT jsonb_agg(to_jsonb(schedule)
                 ORDER BY schedule.schedule_receipt_id)::TEXT
                 FROM investigation_asset_primary_schedules schedule),'[]')),
             tool_truth_sha256(COALESCE((SELECT jsonb_agg(to_jsonb(item) ORDER BY item.id)::TEXT
                 FROM stage_work_items item WHERE item.team_plan_id=$1),'[]')),
             tool_truth_sha256(COALESCE((SELECT jsonb_agg(to_jsonb(worker) ORDER BY worker.id)::TEXT
                 FROM stage_worker_runs worker WHERE worker.stage_run_unit_id=$2),'[]')),
             tool_truth_sha256((SELECT to_jsonb(plan)::TEXT FROM stage_team_plans plan
                 WHERE plan.id=$1))"#,
    )
    .bind(stage_team_plan_id)
    .bind(stage_run_unit_id)
    .fetch_one(db.pool())
    .await
    .expect("snapshot out-of-order cutover census");
    let out_of_order = ensure_investigation_asset_primary_schedule(db.pool(), &request)
        .await
        .expect_err("an older asset receipt must not cut over another asset's live plan epoch");
    assert!(format!("{out_of_order:?}")
        .contains("investigation_asset_legacy_schedule_cutover_authority_mismatch"));
    let out_of_order_after: (String, String, String, String) = sqlx::query_as(
        r#"SELECT
             tool_truth_sha256(COALESCE((SELECT jsonb_agg(to_jsonb(schedule)
                 ORDER BY schedule.schedule_receipt_id)::TEXT
                 FROM investigation_asset_primary_schedules schedule),'[]')),
             tool_truth_sha256(COALESCE((SELECT jsonb_agg(to_jsonb(item) ORDER BY item.id)::TEXT
                 FROM stage_work_items item WHERE item.team_plan_id=$1),'[]')),
             tool_truth_sha256(COALESCE((SELECT jsonb_agg(to_jsonb(worker) ORDER BY worker.id)::TEXT
                 FROM stage_worker_runs worker WHERE worker.stage_run_unit_id=$2),'[]')),
             tool_truth_sha256((SELECT to_jsonb(plan)::TEXT FROM stage_team_plans plan
                 WHERE plan.id=$1))"#,
    )
    .bind(stage_team_plan_id)
    .bind(stage_run_unit_id)
    .fetch_one(db.pool())
    .await
    .expect("reload out-of-order cutover census");
    assert_eq!(
        out_of_order_after, out_of_order_before,
        "out-of-order same-operation asset cutover must be zero-write"
    );
    let mut restore_fixture_tx = db.pool().begin().await.expect("begin plan fixture restore");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *restore_fixture_tx)
        .await
        .expect("isolate plan fixture restore from transition triggers");
    sqlx::query(
        "UPDATE stage_team_plans SET dispatch_epoch=1,row_version=3,updated_at=NOW()
          WHERE id=$1 AND dispatch_epoch=3 AND row_version=4",
    )
    .bind(stage_team_plan_id)
    .execute(&mut *restore_fixture_tx)
    .await
    .expect("restore exact current asset plan epoch");
    restore_fixture_tx
        .commit()
        .await
        .expect("commit exact current asset plan epoch restore");
    let missing_lane = ensure_investigation_asset_primary_schedule(
        db.pool(),
        &EnsureInvestigationAssetPrimaryScheduleRow {
            asset_lane_id: Uuid::new_v4(),
            ..request.clone()
        },
    )
    .await;
    assert!(
        missing_lane.is_err(),
        "a missing asset lane must fail closed"
    );

    let scheduled = ensure_investigation_asset_primary_schedule(db.pool(), &request)
        .await
        .expect("seed the durable Asset Primary");
    assert!(!scheduled.replayed);
    assert_eq!(scheduled.plan.dispatch_epoch, 2);
    assert!(!scheduled.primary_work_item.required_for_barrier);
    assert_eq!(
        scheduled.primary_worker.message_chain_id,
        Some(scheduled.primary_message_chain_id)
    );
    assert_eq!(scheduled.primary_message_chain_id, primary_chain_id);
    let detached_legacy_chain: Option<Uuid> =
        sqlx::query_scalar("SELECT message_chain_id FROM stage_worker_runs WHERE id=$1")
            .bind(legacy_primary_worker_id)
            .fetch_one(db.pool())
            .await
            .expect("load detached legacy Primary worker");
    assert_eq!(detached_legacy_chain, None);

    let replay = ensure_investigation_asset_primary_schedule(db.pool(), &request)
        .await
        .expect("exact schedule replay");
    assert!(replay.replayed);
    assert_eq!(replay.primary_work_item.id, scheduled.primary_work_item.id);
    assert_eq!(
        replay.primary_message_chain_id,
        scheduled.primary_message_chain_id
    );
    let counts: (i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT COUNT(*) FROM investigation_asset_primary_schedules),
             (SELECT COUNT(*) FROM stage_work_items WHERE team_plan_id=$1),
             (SELECT COUNT(*) FROM stage_worker_runs WHERE stage_run_unit_id=$2),
             (SELECT COUNT(*) FROM stage_work_item_dependencies WHERE team_plan_id=$1)"#,
    )
    .bind(stage_team_plan_id)
    .bind(stage_run_unit_id)
    .fetch_one(db.pool())
    .await
    .expect("count durable schedule rows");
    assert_eq!(counts, (2, 6, 2, 0));
    let legacy_unchanged: (String, String) = sqlx::query_as(
        "SELECT schedule_contract,status FROM investigation_asset_primary_schedules
          WHERE schedule_receipt_id=$1",
    )
    .bind(legacy_receipt_id)
    .fetch_one(db.pool())
    .await
    .expect("load audit-only legacy receipt");
    assert_eq!(
        legacy_unchanged,
        ("fixed_roster_v1".to_string(), "applied".to_string())
    );
    let sibling_lane_ids = sibling_asset_lanes
        .iter()
        .map(|(lane_id, _, _)| *lane_id)
        .collect::<Vec<_>>();
    let sibling_census: (i64, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT COUNT(*) FROM investigation_asset_lanes
               WHERE asset_lane_id=ANY($1) AND state='queued'),
             (SELECT COUNT(*) FROM investigation_asset_primary_schedules
               WHERE asset_lane_id=ANY($1))"#,
    )
    .bind(&sibling_lane_ids)
    .fetch_one(db.pool())
    .await
    .expect("load untouched sibling asset census");
    assert_eq!(
        sibling_census,
        (2, 0),
        "cutting over the current lane must not mutate or schedule sibling assets"
    );

    let replay_census = sqlx::query_as::<_, (Uuid, String, String, i64, String, bool, bool)>(
        r#"SELECT item.id,item.kind,item.status,item.dispatch_epoch,item.created_by,
                  EXISTS(SELECT 1 FROM investigation_asset_primary_schedules schedule
                          WHERE schedule.schedule_contract='fixed_roster_v1'
                            AND schedule.stage_team_plan_id=item.team_plan_id
                            AND schedule.resume_dispatch_epoch=item.dispatch_epoch
                            AND item.id=ANY(ARRAY[schedule.primary_work_item_id,
                                schedule.browser_work_item_id,schedule.researcher_work_item_id,
                                schedule.pentester_work_item_id,schedule.adviser_work_item_id])),
                  EXISTS(SELECT 1 FROM investigation_asset_primary_schedules schedule
                          WHERE schedule.schedule_contract='primary_dynamic_v2'
                            AND schedule.stage_team_plan_id=item.team_plan_id
                            AND schedule.resume_dispatch_epoch=item.dispatch_epoch
                            AND schedule.primary_work_item_id=item.id)
             FROM stage_work_items item WHERE item.team_plan_id=$1
             ORDER BY item.dispatch_epoch,item.kind,item.id"#,
    )
    .bind(stage_team_plan_id)
    .fetch_all(db.pool())
    .await
    .expect("load exact governance replay census");
    for row in &replay_census {
        assert!(
            row.5 || row.6 || row.4 == "accepted_worker_request",
            "unmapped governance replay row: {row:?}; census={replay_census:?}"
        );
    }

    let active_governance_replay = seed_stage_team_runtime(db.pool(), &current_governance_seed)
        .await
        .expect("replay governance plan after the exact asset schedule is applied");
    assert!(active_governance_replay[0].replayed);
    assert_eq!(active_governance_replay[0].plan.dispatch_epoch, 2);
    assert!(active_governance_replay[0]
        .plan
        .requests_closed_at
        .is_none());
    assert!(active_governance_replay[0].work_items.is_empty());

    let claimed_primary = claim_stage_team_leader(
        db.pool(),
        &ClaimStageTeamLeaderRow {
            claim: ClaimStageWorkItemRow {
                operation_id,
                stage_execution_id,
                stage_run_unit_id,
                stage_team_plan_id,
                exact_work_item_id: Some(scheduled.primary_work_item.id),
                lease_owner: "asset-primary-first-claim".to_string(),
                lease_seconds: 60,
                session_id,
                subtask_id: None,
                agent: AgentType::Pentester,
                model: Some("test-model".to_string()),
                provider: Some("test-provider".to_string()),
                parent_chain_id: None,
                initial_chain: json!([]),
                initial_checkpoint: json!([]),
            },
        },
    )
    .await
    .expect("claim exact Asset Primary")
    .expect("the exact applied Asset Primary must be initially claimable");
    assert_eq!(claimed_primary.work_item.id, scheduled.primary_work_item.id);
    assert_eq!(claimed_primary.worker.id, scheduled.primary_worker.id);
    assert_eq!(
        claimed_primary.message_chain_id,
        scheduled.primary_message_chain_id
    );
    assert_eq!(claimed_primary.work_item.status, "running");
    assert_eq!(claimed_primary.worker.status, "running");

    let mut dynamic_request = RequestStageWorkerRow {
        fence: RuntimeMemoryTxFence {
            operation_id,
            stage_execution_id,
            stage_run_unit_id,
            worker_run_id: claimed_primary.worker.id,
            lease_token: claimed_primary
                .worker
                .lease_token
                .expect("claimed Primary lease token"),
            attempt_epoch: claimed_primary.worker.attempt_epoch,
            expected_checkpoint_version: claimed_primary.worker.checkpoint_version,
        },
        stage_team_plan_id,
        parent_work_item_id: claimed_primary.work_item.id,
        expected_dispatch_epoch: scheduled.plan.dispatch_epoch,
        requested_role: "browser".to_string(),
        requested_kind: "analysis_task".to_string(),
        subject_refs: Vec::new(),
        reason: json!({
            "schema": "investigation_task_orchestrator_request.v1",
            "objective": "Inspect this asset's web surface",
            "parent_tool_request_id": "asset-browser-1"
        })
        .to_string(),
        output_schema: json!("investigation_cognitive_output.v1"),
        budget_hint: json!({}),
        dedupe_key: "asset-browser-1".to_string(),
        request_sha256: String::new(),
    };
    dynamic_request.request_sha256 = stage_worker_request_payload_hash(&dynamic_request);
    let accepted = request_stage_worker(db.pool(), &dynamic_request)
        .await
        .expect("Primary dynamically requests one asset-bound child");
    assert_eq!(accepted.request.status, "accepted");
    let browser_item = accepted.work_item.expect("accepted browser WorkItem");
    assert_eq!(browser_item.role, "browser");
    assert_eq!(browser_item.created_by, "accepted_worker_request");
    let binding = browser_item
        .input_refs
        .as_array()
        .and_then(|refs| refs.first())
        .and_then(|value| value.get("asset_lane"))
        .expect("server-authored asset lane binding");
    let asset_lane_id_text = asset_lane_id.to_string();
    let target_id_text = target_id.to_string();
    assert_eq!(
        binding
            .get("asset_lane_id")
            .and_then(|value| value.as_str()),
        Some(asset_lane_id_text.as_str())
    );
    assert_eq!(
        binding.get("target_id").and_then(|value| value.as_str()),
        Some(target_id_text.as_str())
    );
    assert_eq!(
        binding
            .get("asset_context_sha256")
            .and_then(|value| value.as_str()),
        Some(asset_context_sha256.as_str())
    );
    let claimed_browser = claim_stage_work_item(
        db.pool(),
        &ClaimStageWorkItemRow {
            operation_id,
            stage_execution_id,
            stage_run_unit_id,
            stage_team_plan_id,
            exact_work_item_id: Some(browser_item.id),
            lease_owner: "asset-browser-first-claim".to_string(),
            lease_seconds: 60,
            session_id,
            subtask_id: None,
            agent: AgentType::Pentester,
            model: Some("test-model".to_string()),
            provider: Some("test-provider".to_string()),
            parent_chain_id: None,
            initial_chain: json!([]),
            initial_checkpoint: json!([]),
        },
    )
    .await
    .expect("claim exact browser role")
    .expect("the exact browser role must be claimable");
    assert_eq!(claimed_browser.work_item.id, browser_item.id);
    assert_eq!(claimed_browser.worker.status, "running");
    let expected_browser_parent_request_id = "asset-browser-1";
    assert_eq!(
        claimed_browser.worker.parent_request_id.as_deref(),
        Some(expected_browser_parent_request_id)
    );

    let mut executed_children = vec![(
        accepted.request.id,
        browser_item.id,
        claimed_browser.worker.id,
    )];
    let mut terminalize_child = db.pool().begin().await.expect("terminalize browser child");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *terminalize_child)
        .await
        .expect("isolate child terminal fixture");
    sqlx::query(
        "UPDATE stage_worker_runs SET status='passed',terminal_at=NOW(),
                lease_token=NULL,lease_owner=NULL,lease_acquired_at=NULL,
                lease_expires_at=NULL,heartbeat_at=NULL,updated_at=NOW() WHERE id=$1",
    )
    .bind(claimed_browser.worker.id)
    .execute(&mut *terminalize_child)
    .await
    .expect("pass executed Browser worker");
    sqlx::query(
        "UPDATE stage_work_items SET status='completed',terminal_at=NOW(),
                row_version=row_version+1,updated_at=NOW() WHERE id=$1",
    )
    .bind(browser_item.id)
    .execute(&mut *terminalize_child)
    .await
    .expect("complete executed Browser item");
    sqlx::query(
        r#"INSERT INTO stage_worker_outputs(
               id,team_plan_id,work_item_id,worker_run_id,operation_id,stage_execution_id,
               stage_run_unit_id,scope_snapshot_id,organization_id,output_schema,output_version,
               business_disposition,canonical_output,canonical_fact_refs,evidence_ids,
               checked_empty_cells,blocker_codes,output_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'investigation_cognitive_output.v1',1,
                  'found','{}'::JSONB,'[]'::JSONB,ARRAY[]::BIGINT[],
                  '[]'::JSONB,ARRAY[]::TEXT[],$10)"#,
    )
    .bind(Uuid::new_v4())
    .bind(stage_team_plan_id)
    .bind(browser_item.id)
    .bind(claimed_browser.worker.id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(digest('b'))
    .execute(&mut *terminalize_child)
    .await
    .expect("persist executed Browser output");
    terminalize_child
        .commit()
        .await
        .expect("commit Browser terminal fixture");

    for (ordinal, role) in ["researcher", "pentester", "adviser"]
        .into_iter()
        .enumerate()
    {
        let dedupe_key = format!("asset-executed-{role}");
        let mut child_request = RequestStageWorkerRow {
            requested_role: role.to_string(),
            reason: json!({
                "schema": "investigation_task_orchestrator_request.v1",
                "objective": format!("Execute direct census member {role}"),
                "parent_tool_request_id": dedupe_key,
            })
            .to_string(),
            dedupe_key: dedupe_key.clone(),
            ..dynamic_request.clone()
        };
        child_request.request_sha256 = stage_worker_request_payload_hash(&child_request);
        let child = request_stage_worker(db.pool(), &child_request)
            .await
            .expect("request direct executed child");
        let child_item = child.work_item.expect("accepted direct child WorkItem");
        let claimed = claim_stage_work_item(
            db.pool(),
            &ClaimStageWorkItemRow {
                operation_id,
                stage_execution_id,
                stage_run_unit_id,
                stage_team_plan_id,
                exact_work_item_id: Some(child_item.id),
                lease_owner: format!("asset-{role}-claim"),
                lease_seconds: 60,
                session_id,
                subtask_id: None,
                agent: AgentType::Pentester,
                model: Some("test-model".to_string()),
                provider: Some("test-provider".to_string()),
                parent_chain_id: None,
                initial_chain: json!([]),
                initial_checkpoint: json!([]),
            },
        )
        .await
        .expect("claim direct executed child")
        .expect("direct child must be claimable");
        executed_children.push((child.request.id, child_item.id, claimed.worker.id));

        let mut terminal = db.pool().begin().await.expect("terminalize direct child");
        sqlx::query("SET LOCAL session_replication_role='replica'")
            .execute(&mut *terminal)
            .await
            .expect("isolate direct child terminal fixture");
        sqlx::query(
            "UPDATE stage_worker_runs SET status='passed',terminal_at=NOW(),
                    lease_token=NULL,lease_owner=NULL,lease_acquired_at=NULL,
                    lease_expires_at=NULL,heartbeat_at=NULL,updated_at=NOW() WHERE id=$1",
        )
        .bind(claimed.worker.id)
        .execute(&mut *terminal)
        .await
        .expect("pass direct child worker");
        sqlx::query(
            "UPDATE stage_work_items SET status='completed',terminal_at=NOW(),
                    row_version=row_version+1,updated_at=NOW() WHERE id=$1",
        )
        .bind(child_item.id)
        .execute(&mut *terminal)
        .await
        .expect("complete direct child item");
        sqlx::query(
            r#"INSERT INTO stage_worker_outputs(
                   id,team_plan_id,work_item_id,worker_run_id,operation_id,stage_execution_id,
                   stage_run_unit_id,scope_snapshot_id,organization_id,output_schema,
                   output_version,business_disposition,canonical_output,canonical_fact_refs,
                   evidence_ids,checked_empty_cells,blocker_codes,output_hash)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'investigation_cognitive_output.v1',1,
                      'found','{}'::JSONB,'[]'::JSONB,ARRAY[]::BIGINT[],
                      '[]'::JSONB,ARRAY[]::TEXT[],$10)"#,
        )
        .bind(Uuid::new_v4())
        .bind(stage_team_plan_id)
        .bind(child_item.id)
        .bind(claimed.worker.id)
        .bind(operation_id)
        .bind(stage_execution_id)
        .bind(stage_run_unit_id)
        .bind(scope_snapshot_id)
        .bind(organization_id)
        .bind(digest(
            char::from_digit((ordinal + 5) as u32, 16).expect("digest nibble"),
        ))
        .execute(&mut *terminal)
        .await
        .expect("persist direct child output");
        terminal
            .commit()
            .await
            .expect("commit direct child terminal fixture");
    }
    assert_eq!(executed_children.len(), 4);

    // Exercise the dynamic Refiner ledger against the real migrated schema.
    // The Generator freezes four initial members. Later patches replace that
    // initial denominator with revised members, so three Generator originals
    // are legitimately dropped without appearing in a patch-member row.
    let task_plan_id = Uuid::new_v4();
    let primary_dispatch_receipt_id = Uuid::new_v4();
    let foreign_task_plan_id = Uuid::new_v4();
    let foreign_dispatch_receipt_id = Uuid::new_v4();
    let mut refiner_fixture = db.pool().begin().await.expect("begin Refiner v2 fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *refiner_fixture)
        .await
        .expect("isolate PentAGI authority fixture triggers");
    for (plan_id, stable_request_id, subject_id) in [
        (task_plan_id, Uuid::new_v4(), Uuid::new_v4()),
        (foreign_task_plan_id, Uuid::new_v4(), Uuid::new_v4()),
    ] {
        sqlx::query(
            r#"INSERT INTO investigation_pentagi_task_plans(
                   task_plan_id,stable_request_id,authority_id,stage_team_plan_id,
                   operation_id,stage_execution_id,owning_stage_run_request_id,
                   stage_run_unit_id,scope_snapshot_id,organization_id,subject_kind,
                   subject_id,subject_fingerprint_sha256,task_plan_version,task_plan_sha256,
                   allowed_role_catalog,cognitive_tool_envelope_sha256,status,row_version)
               VALUES($1,$2,$3,$4,$5,$6,'asset-schedule',$7,$8,$9,'analysis_attempt',
                      $10,$11,1,$12,'["primary","refiner"]'::JSONB,$13,'open',0)"#,
        )
        .bind(plan_id)
        .bind(stable_request_id)
        .bind(authority_id)
        .bind(stage_team_plan_id)
        .bind(operation_id)
        .bind(stage_execution_id)
        .bind(stage_run_unit_id)
        .bind(scope_snapshot_id)
        .bind(organization_id)
        .bind(subject_id)
        .bind(digest('1'))
        .bind(digest('2'))
        .bind(digest('3'))
        .execute(&mut *refiner_fixture)
        .await
        .expect("seed open PentAGI task plan");
    }
    sqlx::query(
        r#"INSERT INTO pentagi_logical_dispatch_receipts(
               dispatch_receipt_id,stable_request_id,logical_dispatch_key_sha256,
               task_plan_id,dispatch_ordinal,actor_kind,stage_work_item_id,worker_run_id,
               operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
               organization_id,transcript_request_id,snapshot_sha256,receipt_sha256)
           VALUES($1,$2,$3,$4,0,'primary',$5,$6,$7,$8,$9,$10,$11,
                  'asset-primary-refiner-v2',$12,$13)"#,
    )
    .bind(primary_dispatch_receipt_id)
    .bind(Uuid::new_v4())
    .bind(digest('4'))
    .bind(task_plan_id)
    .bind(claimed_primary.work_item.id)
    .bind(claimed_primary.worker.id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(digest('5'))
    .bind(digest('6'))
    .execute(&mut *refiner_fixture)
    .await
    .expect("seed current dynamic Primary dispatch");
    sqlx::query(
        r#"INSERT INTO pentagi_logical_dispatch_receipts(
               dispatch_receipt_id,stable_request_id,logical_dispatch_key_sha256,
               task_plan_id,dispatch_ordinal,actor_kind,stage_work_item_id,worker_run_id,
               operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
               organization_id,transcript_request_id,snapshot_sha256,receipt_sha256)
           VALUES($1,$2,$3,$4,0,'primary',$5,$6,$7,$8,$9,$10,$11,
                  'legacy-fixed-primary-refiner-v2',$12,$13)"#,
    )
    .bind(foreign_dispatch_receipt_id)
    .bind(Uuid::new_v4())
    .bind(digest('7'))
    .bind(foreign_task_plan_id)
    .bind(legacy_primary_item_id)
    .bind(legacy_primary_worker_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(digest('8'))
    .bind(digest('9'))
    .execute(&mut *refiner_fixture)
    .await
    .expect("seed audit-only fixed Primary dispatch");
    refiner_fixture
        .commit()
        .await
        .expect("commit Refiner v2 fixture");

    let foreign_asset_error = sqlx::query(
        "SELECT ledger_id FROM create_investigation_refiner_plan_ledger_v2($1,$2,$3,$4,$5)",
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(foreign_task_plan_id)
    .bind(Uuid::new_v4())
    .bind(json!({"contract_version":"dynamic_refiner_v2","generator":"zero"}))
    .execute(db.pool())
    .await
    .expect_err("a fixed-roster Primary cannot authorize a dynamic-v2 asset ledger");
    assert!(
        foreign_asset_error
            .to_string()
            .contains("INVESTIGATION_REFINER_V2_ASSET_AUTHORITY_MISMATCH"),
        "unexpected foreign asset error: {foreign_asset_error}"
    );

    let generator_manifest = json!({"contract_version":"dynamic_refiner_v2","generator":"four"});
    let source_tool_call_id = Uuid::new_v4();
    let source_provider_call_id = format!("generator-source-{source_tool_call_id}");
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,result,status,source,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token)
           VALUES($1,$2,$3,$4,'pentester','submit_result',$5,$6,'finished','ai',
                  $4,$7,$8,$9,$10,$11,$12)"#,
    )
    .bind(source_tool_call_id)
    .bind(&source_provider_call_id)
    .bind(session_id)
    .bind(operation_id)
    .bind(json!({"result": generator_manifest}))
    .bind(json!({"status":"result submitted"}).to_string())
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(claimed_primary.worker.id)
    .bind(organization_id)
    .bind(claimed_primary.worker.attempt_epoch)
    .bind(claimed_primary.worker.lease_token.expect("Primary lease"))
    .execute(db.pool())
    .await
    .expect("record durable Generator submit_result source");

    let runtime = PgUnifiedInvestigationRuntimeRepository::new(Arc::new(db.pool().clone()));
    let generator_identity = InvestigationUnitIdentity {
        stage: InvestigationStageIdentity {
            authority_id,
            operation_id,
            stage_execution_id,
            owning_stage_run_request_id: "asset-schedule".to_string(),
            scope_snapshot_id,
        },
        stage_run_unit_id,
        organization_id,
    };
    let consumer_fence = InvestigationGeneratorConsumerFenceInput {
        current_consumer_work_item_id: claimed_primary.work_item.id,
        current_consumer_worker_run_id: claimed_primary.worker.id,
        current_consumer_lease_token: claimed_primary.worker.lease_token.expect("Primary lease"),
        expected_consumer_attempt_epoch: claimed_primary.worker.attempt_epoch as u64,
        expected_consumer_checkpoint_version: claimed_primary.worker.checkpoint_version as u64,
    };
    let ledger_id = Uuid::new_v4();
    let ledger_request_id = Uuid::new_v4();
    let generator_event_id = Uuid::new_v4();
    let source_receipt_id = unified_stable_id(
        task_plan_id,
        "generator-source-receipt",
        &[source_tool_call_id.to_string().as_str()],
    );
    let failed_subtask_id = Uuid::new_v4();
    let failed = runtime
        .materialize_generator(&MaterializeInvestigationGeneratorInput {
            identity: generator_identity.clone(),
            task_plan_id,
            ledger_id,
            stable_request_id: ledger_request_id,
            generator_pipeline_event_id: generator_event_id,
            source_receipt_id,
            source_tool_call_id,
            consumer_fence: InvestigationGeneratorConsumerFenceInput {
                current_consumer_lease_token: Uuid::new_v4(),
                ..consumer_fence.clone()
            },
            subtasks: vec![InvestigationGeneratorSubtaskInput {
                subtask_id: failed_subtask_id,
                subtask_ordinal: 0,
                label: "must roll back".to_string(),
                runnable: true,
                input_manifest_sha256: digest('1'),
                expected_output_schema: "investigation_cognitive_output.v1".to_string(),
                member_sha256: digest('2'),
            }],
        })
        .await;
    assert!(failed.is_err(), "foreign consumer fence must fail closed");
    let failed_write_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM investigation_pentagi_subtasks WHERE subtask_id=$1",
    )
    .bind(failed_subtask_id)
    .fetch_one(db.pool())
    .await
    .expect("count rolled-back Generator member");
    assert_eq!(failed_write_count, 0);

    let dynamic_subtask_ids = (0..10).map(|_| Uuid::new_v4()).collect::<Vec<_>>();
    let materialize = MaterializeInvestigationGeneratorInput {
        identity: generator_identity.clone(),
        task_plan_id,
        ledger_id,
        stable_request_id: ledger_request_id,
        generator_pipeline_event_id: generator_event_id,
        source_receipt_id,
        source_tool_call_id,
        consumer_fence: consumer_fence.clone(),
        subtasks: dynamic_subtask_ids[..4]
            .iter()
            .copied()
            .enumerate()
            .map(|(ordinal, subtask_id)| InvestigationGeneratorSubtaskInput {
                subtask_id,
                subtask_ordinal: ordinal as u32,
                label: format!("Generator hypothesis {ordinal}"),
                runnable: true,
                input_manifest_sha256: digest('c'),
                expected_output_schema: "InvestigationDynamicRefinerResultV2".to_string(),
                member_sha256: format!("sha256:{0}{0}", subtask_id.simple()),
            })
            .collect(),
    };
    let materialized = runtime
        .materialize_generator(&materialize)
        .await
        .expect("atomically materialize four-subtask Generator ledger");
    assert!(!materialized.replayed);
    assert_eq!(materialized.ledger.generator_subtask_count, 4);
    assert_eq!(materialized.source.source_tool_call_id, source_tool_call_id);
    let replayed = runtime
        .materialize_generator(&materialize)
        .await
        .expect("response-loss replay exact Generator materialization");
    assert!(replayed.replayed);
    assert_eq!(replayed.ledger, materialized.ledger);
    let ledger = (
        materialized.ledger.ledger_sha256.clone(),
        materialized.ledger.generator_subtask_count,
        "dynamic_ordered_v2".to_string(),
    );

    for (ordinal, subtask_id) in dynamic_subtask_ids.iter().copied().enumerate().skip(4) {
        sqlx::query(
            r#"INSERT INTO investigation_pentagi_subtasks(
                   subtask_id,task_plan_id,authority_id,operation_id,stage_execution_id,
                   stage_run_unit_id,organization_id,subtask_ordinal,label,runnable,
                   input_manifest_sha256,expected_output_schema,member_sha256)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,TRUE,$10,
                      'InvestigationDynamicRefinerResultV2',$11)"#,
        )
        .bind(subtask_id)
        .bind(task_plan_id)
        .bind(authority_id)
        .bind(operation_id)
        .bind(stage_execution_id)
        .bind(stage_run_unit_id)
        .bind(organization_id)
        .bind(i32::try_from(ordinal).expect("subtask ordinal"))
        .bind(format!("dynamic hypothesis {ordinal}"))
        .bind(digest('c'))
        .bind(format!("sha256:{0}{0}", subtask_id.simple()))
        .execute(db.pool())
        .await
        .expect("dynamically append a Refiner subtask");
    }

    let completed_subtask_ids = [0_usize, 4, 7, 9]
        .into_iter()
        .map(|ordinal| dynamic_subtask_ids[ordinal])
        .collect::<Vec<_>>();
    let patch_specs = [
        (
            json!({
                "step":"complete_generator_and_revise",
                "completed_subtask_ids":[dynamic_subtask_ids[0]],
            }),
            dynamic_subtask_ids[4..7].to_vec(),
        ),
        (
            json!({
                "step":"complete_first_revision",
                "completed_subtask_ids":[dynamic_subtask_ids[0],dynamic_subtask_ids[4]],
            }),
            dynamic_subtask_ids[7..9].to_vec(),
        ),
        (
            json!({
                "step":"complete_second_revision",
                "completed_subtask_ids":[
                    dynamic_subtask_ids[0],dynamic_subtask_ids[4],dynamic_subtask_ids[7]
                ],
            }),
            vec![dynamic_subtask_ids[9]],
        ),
        (
            json!({
                "step":"complete_and_drop",
                "completed_subtask_ids":completed_subtask_ids.clone(),
            }),
            Vec::new(),
        ),
    ];
    let mut previous_state_sha256 = ledger.0.clone();
    let mut persisted_patches = Vec::new();
    for (expected_ordinal, (payload, active_subtask_ids)) in patch_specs.into_iter().enumerate() {
        let patch_id = Uuid::new_v4();
        let patch_request_id = Uuid::new_v4();
        let patch_event_id = Uuid::new_v4();
        let patch: (String, i64, i64, String) = sqlx::query_as(
            r#"SELECT patch_sha256,patch_ordinal,active_realized_subtask_count,patch_contract
                 FROM append_investigation_refiner_plan_patch_v2(
                     $1,$2,$3,$4,$5,$6,$7,$8)"#,
        )
        .bind(patch_id)
        .bind(patch_request_id)
        .bind(ledger_id)
        .bind(task_plan_id)
        .bind(patch_event_id)
        .bind(&previous_state_sha256)
        .bind(&payload)
        .bind(&active_subtask_ids)
        .fetch_one(db.pool())
        .await
        .expect("append ordered dynamic Refiner patch");
        assert_eq!(patch.1, expected_ordinal as i64);
        assert_eq!(patch.2, active_subtask_ids.len() as i64);
        assert_eq!(patch.3, "dynamic_ordered_v2");
        let replayed_patch: (String, i64, i64, String) = sqlx::query_as(
            r#"SELECT patch_sha256,patch_ordinal,active_realized_subtask_count,patch_contract
                 FROM append_investigation_refiner_plan_patch_v2(
                     $1,$2,$3,$4,$5,$6,$7,$8)"#,
        )
        .bind(patch_id)
        .bind(patch_request_id)
        .bind(ledger_id)
        .bind(task_plan_id)
        .bind(patch_event_id)
        .bind(&previous_state_sha256)
        .bind(&payload)
        .bind(&active_subtask_ids)
        .fetch_one(db.pool())
        .await
        .expect("replay ordered dynamic Refiner patch");
        assert_eq!(replayed_patch, patch);
        previous_state_sha256 = patch.0.clone();
        persisted_patches.push((
            patch_id,
            patch_request_id,
            patch_event_id,
            payload,
            active_subtask_ids,
        ));
    }

    let foreign_task_error = sqlx::query(
        r#"SELECT patch_id FROM append_investigation_refiner_plan_patch_v2(
               $1,$2,$3,$4,$5,$6,$7,$8)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(ledger_id)
    .bind(task_plan_id)
    .bind(Uuid::new_v4())
    .bind(&previous_state_sha256)
    .bind(json!({"step":"foreign-subtask"}))
    .bind(vec![Uuid::new_v4()])
    .execute(db.pool())
    .await
    .expect_err("a foreign subtask cannot enter this asset's active denominator");
    assert!(
        foreign_task_error
            .to_string()
            .contains("INVESTIGATION_REFINER_V2_ASSET_AUTHORITY_MISMATCH"),
        "unexpected foreign task error: {foreign_task_error}"
    );

    let seal_id = Uuid::new_v4();
    let seal_request_id = Uuid::new_v4();
    let barrier_event_id = Uuid::new_v4();
    let seal: (i64, i64, i64, String, String) = sqlx::query_as(
        r#"SELECT patch_count,generator_subtask_count,
                  final_active_realized_subtask_count,
                  final_active_realized_subtask_set_sha256,seal_contract
             FROM seal_investigation_refiner_plan_ledger_v2($1,$2,$3,$4,$5,$6)"#,
    )
    .bind(seal_id)
    .bind(seal_request_id)
    .bind(ledger_id)
    .bind(task_plan_id)
    .bind(barrier_event_id)
    .bind(&previous_state_sha256)
    .fetch_one(db.pool())
    .await
    .expect("seal final ordered dynamic Refiner denominator");
    assert_eq!(seal.0, 4);
    assert_eq!(seal.1, 4);
    assert_eq!(seal.2, 0);
    assert_eq!(seal.4, "dynamic_ordered_v2");
    let final_member_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM investigation_refiner_plan_ledger_seals seal
             JOIN investigation_refiner_plan_patch_members member
               ON member.patch_id=seal.final_patch_id
            WHERE seal.seal_id=$1"#,
    )
    .bind(seal_id)
    .fetch_one(db.pool())
    .await
    .expect("load empty final Refiner denominator");
    assert_eq!(final_member_count, 0);
    let replayed_seal: (i64, i64, i64, String, String) = sqlx::query_as(
        r#"SELECT patch_count,generator_subtask_count,
                  final_active_realized_subtask_count,
                  final_active_realized_subtask_set_sha256,seal_contract
             FROM seal_investigation_refiner_plan_ledger_v2($1,$2,$3,$4,$5,$6)"#,
    )
    .bind(seal_id)
    .bind(seal_request_id)
    .bind(ledger_id)
    .bind(task_plan_id)
    .bind(barrier_event_id)
    .bind(&previous_state_sha256)
    .fetch_one(db.pool())
    .await
    .expect("replay final dynamic Refiner seal");
    assert_eq!(replayed_seal, seal);
    let refiner_counts: (i64, i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT COUNT(*) FROM investigation_refiner_plan_ledgers WHERE ledger_id=$1),
             (SELECT COUNT(*) FROM investigation_refiner_plan_patches WHERE ledger_id=$1),
             (SELECT COUNT(*) FROM investigation_refiner_plan_patch_members member
                JOIN investigation_refiner_plan_patches patch ON patch.patch_id=member.patch_id
               WHERE patch.ledger_id=$1),
             (SELECT COUNT(*) FROM investigation_refiner_plan_ledger_seals WHERE ledger_id=$1),
             (SELECT COUNT(*) FROM investigation_pentagi_pipeline_events WHERE task_plan_id=$2)"#,
    )
    .bind(ledger_id)
    .bind(task_plan_id)
    .fetch_one(db.pool())
    .await
    .expect("count replay-stable Refiner v2 rows");
    assert_eq!(refiner_counts, (1, 4, 6, 1, 6));
    assert_eq!(persisted_patches.len(), 4);

    let primary_result_sha256: String = sqlx::query_scalar(
        "SELECT seal_sha256 FROM investigation_refiner_plan_ledger_seals WHERE seal_id=$1",
    )
    .bind(seal_id)
    .fetch_one(db.pool())
    .await
    .expect("load Primary terminal result hash");
    let direct_dispatches = executed_children
        .iter()
        .zip(completed_subtask_ids.iter())
        .enumerate()
        .map(
            |(ordinal, ((request_id, item_id, worker_id), subtask_id))| {
                (
                    Uuid::new_v4(),
                    *request_id,
                    *item_id,
                    *worker_id,
                    *subtask_id,
                    i32::try_from(ordinal + 1).expect("direct dispatch ordinal"),
                    digest(char::from_digit((ordinal + 10) as u32, 16).expect("result nibble")),
                )
            },
        )
        .collect::<Vec<_>>();
    let mut dispatch_fixture = db.pool().begin().await.expect("begin delegation fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *dispatch_fixture)
        .await
        .expect("isolate delegation fixture triggers");
    sqlx::query(
        r#"INSERT INTO pentagi_logical_dispatch_attempts(
               dispatch_attempt_id,stable_request_id,dispatch_receipt_id,attempt_epoch,
               lease_token,fence_sha256,outcome,result_sha256)
           VALUES($1,$2,$3,0,$4,$5,'completed',$6)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(primary_dispatch_receipt_id)
    .bind(Uuid::new_v4())
    .bind(digest('d'))
    .bind(&primary_result_sha256)
    .execute(&mut *dispatch_fixture)
    .await
    .expect("settle Primary logical dispatch");
    for (dispatch_id, request_id, item_id, worker_id, subtask_id, ordinal, result_hash) in
        &direct_dispatches
    {
        sqlx::query(
            r#"INSERT INTO pentagi_logical_dispatch_receipts(
                   dispatch_receipt_id,stable_request_id,logical_dispatch_key_sha256,
                   task_plan_id,subtask_id,parent_dispatch_receipt_id,dispatch_ordinal,
                   actor_kind,stage_work_item_id,stage_worker_request_id,worker_run_id,
                   operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
                   organization_id,transcript_request_id,parent_actor_transcript_request_id,
                   parent_dispatch_tool_request_id,snapshot_sha256,receipt_sha256)
               VALUES($1,$2,$3,$4,$5,$6,$7,'worker',$8,$9,$10,$11,$12,$13,$14,$15,
                      $16,'asset-primary-refiner-v2',$17,$18,$19)"#,
        )
        .bind(dispatch_id)
        .bind(Uuid::new_v4())
        .bind(format!("sha256:{0}{0}", dispatch_id.simple()))
        .bind(task_plan_id)
        .bind(subtask_id)
        .bind(primary_dispatch_receipt_id)
        .bind(ordinal)
        .bind(item_id)
        .bind(request_id)
        .bind(worker_id)
        .bind(operation_id)
        .bind(stage_execution_id)
        .bind(stage_run_unit_id)
        .bind(scope_snapshot_id)
        .bind(organization_id)
        .bind(format!("direct-worker-{worker_id}"))
        .bind(format!("asset-executed-{ordinal}"))
        .bind(digest('e'))
        .bind(format!("sha256:{0}{0}", request_id.simple()))
        .execute(&mut *dispatch_fixture)
        .await
        .expect("seed direct worker dispatch");
        sqlx::query(
            r#"INSERT INTO pentagi_logical_dispatch_attempts(
                   dispatch_attempt_id,stable_request_id,dispatch_receipt_id,attempt_epoch,
                   lease_token,fence_sha256,outcome,result_sha256)
               VALUES($1,$2,$3,0,$4,$5,'completed',$6)"#,
        )
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(dispatch_id)
        .bind(Uuid::new_v4())
        .bind(digest('f'))
        .bind(result_hash)
        .execute(&mut *dispatch_fixture)
        .await
        .expect("settle direct worker dispatch");
    }
    for (event_offset, dispatch) in direct_dispatches.iter().take(3).enumerate() {
        sqlx::query(
            r#"INSERT INTO investigation_pentagi_pipeline_events(
                   pipeline_event_id,stable_request_id,task_plan_id,subtask_id,event_ordinal,
                   event_kind,actor_worker_run_id,parent_dispatch_receipt_id,event_sha256)
               VALUES($1,$2,$3,$4,$5,'result_barrier',$6,$7,$8)"#,
        )
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(task_plan_id)
        .bind(dispatch.4)
        .bind(i64::try_from(6 + event_offset).expect("barrier ordinal"))
        .bind(dispatch.3)
        .bind(dispatch.0)
        .bind(&dispatch.6)
        .execute(&mut *dispatch_fixture)
        .await
        .expect("seed completed direct result barrier");
    }
    dispatch_fixture
        .commit()
        .await
        .expect("commit incomplete delegation fixture");

    let census_input = SealPentagiDelegationCensusInput {
        identity: generator_identity.clone(),
        census_seal_id: Uuid::new_v4(),
        stable_request_id: Uuid::new_v4(),
        task_plan_id,
        primary_dispatch_receipt_id,
        primary_worker_run_id: claimed_primary.worker.id,
        seal_sha256: digest('0'),
    };
    let missing_barrier = runtime
        .seal_delegation_census(&census_input)
        .await
        .expect_err("a direct worker without its exact result barrier must fail closed");
    assert!(missing_barrier
        .to_string()
        .contains("PENTAGI_DYNAMIC_WORKER_DISPATCH_REQUIRES_RESULT_BARRIER"));

    let final_dispatch = &direct_dispatches[3];
    let mut foreign_barrier = db
        .pool()
        .begin()
        .await
        .expect("begin foreign barrier fixture");
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_pipeline_events(
               pipeline_event_id,stable_request_id,task_plan_id,subtask_id,event_ordinal,
               event_kind,actor_worker_run_id,parent_dispatch_receipt_id,event_sha256)
           VALUES($1,$2,$3,$4,9,'result_barrier',$5,$6,$7)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(task_plan_id)
    .bind(final_dispatch.4)
    .bind(final_dispatch.3)
    .bind(final_dispatch.0)
    .bind(&final_dispatch.6)
    .execute(&mut *foreign_barrier)
    .await
    .expect("stage exact final result barrier before foreign witness");
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_pipeline_events(
               pipeline_event_id,stable_request_id,task_plan_id,subtask_id,event_ordinal,
               event_kind,actor_worker_run_id,parent_dispatch_receipt_id,event_sha256)
           VALUES($1,$2,$3,$4,10,'result_barrier',$5,$6,$7)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(task_plan_id)
    .bind(final_dispatch.4)
    .bind(direct_dispatches[0].3)
    .bind(final_dispatch.0)
    .bind(&final_dispatch.6)
    .execute(&mut *foreign_barrier)
    .await
    .expect("stage foreign worker result barrier");
    let foreign_barrier_error = sqlx::query(
        r#"WITH census AS (
               SELECT * FROM investigation_effective_delegation_census_v2($3)
           )
           INSERT INTO investigation_pentagi_delegation_census_seals(
               census_seal_id,stable_request_id,task_plan_id,primary_dispatch_receipt_id,
               primary_worker_run_id,runnable_subtask_count,runnable_subtask_set_sha256,
               dispatch_count,dispatch_set_sha256,pipeline_event_count,
               pipeline_event_set_sha256,seal_sha256)
           SELECT $1,$2,$3,$4,$5,census.runnable_subtask_count,
                  census.runnable_subtask_set_sha256,census.dispatch_count,
                  census.dispatch_set_sha256,census.pipeline_event_count,
                  census.pipeline_event_set_sha256,$6 FROM census"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(task_plan_id)
    .bind(primary_dispatch_receipt_id)
    .bind(claimed_primary.worker.id)
    .bind(digest('1'))
    .execute(&mut *foreign_barrier)
    .await
    .expect_err("a foreign worker result barrier must fail closed");
    assert!(foreign_barrier_error
        .to_string()
        .contains("PENTAGI_DYNAMIC_RESULT_BARRIER_REQUIRES_WORKER_DISPATCH"));
    foreign_barrier
        .rollback()
        .await
        .expect("roll back foreign worker barrier");

    sqlx::query(
        r#"INSERT INTO investigation_pentagi_pipeline_events(
               pipeline_event_id,stable_request_id,task_plan_id,subtask_id,event_ordinal,
               event_kind,actor_worker_run_id,parent_dispatch_receipt_id,event_sha256)
           VALUES($1,$2,$3,$4,9,'result_barrier',$5,$6,$7)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(task_plan_id)
    .bind(final_dispatch.4)
    .bind(final_dispatch.3)
    .bind(final_dispatch.0)
    .bind(&final_dispatch.6)
    .execute(db.pool())
    .await
    .expect("persist final exact direct result barrier");
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_pipeline_events(
               pipeline_event_id,stable_request_id,task_plan_id,subtask_id,event_ordinal,
               event_kind,actor_worker_run_id,parent_dispatch_receipt_id,event_sha256)
           VALUES($1,$2,$3,NULL,10,'primary_synthesis',$4,$5,$6)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(task_plan_id)
    .bind(claimed_primary.worker.id)
    .bind(primary_dispatch_receipt_id)
    .bind(&primary_result_sha256)
    .execute(db.pool())
    .await
    .expect("persist exact post-Refiner Primary synthesis witness");
    let census = runtime
        .seal_delegation_census(&census_input)
        .await
        .expect("seal exact dynamic delegation census");
    assert_eq!(census.runnable_subtask_count, 4);
    assert_eq!(census.dispatch_count, 5);
    assert_eq!(census.pipeline_event_count, 11);
    let census_replay = runtime
        .seal_delegation_census(&census_input)
        .await
        .expect("replay exact dynamic delegation census");
    assert_eq!(census_replay, census);

    let active_schedule_replay = ensure_investigation_asset_primary_schedule(db.pool(), &request)
        .await
        .expect("active Primary-only schedule replay");
    assert!(active_schedule_replay.replayed);
    assert_eq!(
        active_schedule_replay.primary_work_item.id,
        scheduled.primary_work_item.id
    );

    let drift = ensure_investigation_asset_primary_schedule(
        db.pool(),
        &EnsureInvestigationAssetPrimaryScheduleRow {
            asset_context_sha256: digest('f'),
            ..request.clone()
        },
    )
    .await;
    assert!(drift.is_err(), "foreign asset context must fail closed");

    let replayed_request = request_stage_worker(db.pool(), &dynamic_request)
        .await
        .expect("exact dynamic child request replay");
    assert!(replayed_request.replayed);
    assert_eq!(replayed_request.work_item.unwrap().id, browser_item.id);

    // Retained-entity recovery fixture: the dynamic Primary spent its exact
    // attempt fuel and already owns the immutable blocked output.  Every child
    // is terminal and no live lease/tool remains.
    let exhaustion_output_id = Uuid::new_v4();
    let mut exhausted_fixture = db
        .pool()
        .begin()
        .await
        .expect("begin exhausted Primary fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *exhausted_fixture)
        .await
        .expect("isolate retained terminal fixture triggers");
    sqlx::query(
        "UPDATE stage_worker_runs SET status='failed',attempt_epoch=3,
                checkpoint_version=3,terminal_at=NOW(),lease_token=NULL,lease_owner=NULL,
                lease_acquired_at=NULL,lease_expires_at=NULL,heartbeat_at=NULL,
                active_tool_call_id=NULL,active_tool_started_at=NULL,updated_at=NOW()
          WHERE id=$1",
    )
    .bind(scheduled.primary_worker.id)
    .execute(&mut *exhausted_fixture)
    .await
    .expect("exhaust dynamic Primary worker fixture");
    sqlx::query(
        "UPDATE stage_work_items SET status='exhausted',terminal_at=NOW(),
                row_version=row_version+1,updated_at=NOW() WHERE id=$1",
    )
    .bind(scheduled.primary_work_item.id)
    .execute(&mut *exhausted_fixture)
    .await
    .expect("exhaust dynamic Primary item fixture");
    sqlx::query(
        r#"INSERT INTO stage_worker_outputs(
               id,team_plan_id,work_item_id,worker_run_id,operation_id,stage_execution_id,
               stage_run_unit_id,scope_snapshot_id,organization_id,output_schema,output_version,
               business_disposition,canonical_output,canonical_fact_refs,evidence_ids,
               checked_empty_cells,blocker_codes,output_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'stage_unit_aggregate.v1',1,'blocked',
             '{"kind":"stage_team_attempts_exhausted","failure_code":"stage_team_worker_lease_expired","schema_version":1}'::JSONB,
             '[]'::JSONB,ARRAY[]::BIGINT[],'[]'::JSONB,
             ARRAY['STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED']::TEXT[],$10)"#,
    )
    .bind(exhaustion_output_id)
    .bind(stage_team_plan_id)
    .bind(scheduled.primary_work_item.id)
    .bind(scheduled.primary_worker.id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(digest('e'))
    .execute(&mut *exhausted_fixture)
    .await
    .expect("insert immutable exhaustion output fixture");
    exhausted_fixture
        .commit()
        .await
        .expect("commit retained exhausted Primary fixture");

    let rearmed = ensure_investigation_asset_primary_schedule(db.pool(), &request)
        .await
        .expect("rearm exhausted Asset Primary on the same durable chain");
    assert!(!rearmed.replayed);
    assert_eq!(rearmed.execution_ordinal, 1);
    assert!(rearmed.execution_rearm_receipt_id.is_some());
    assert_eq!(
        rearmed.plan.dispatch_epoch,
        scheduled.plan.dispatch_epoch + 1
    );
    assert_ne!(rearmed.primary_work_item.id, scheduled.primary_work_item.id);
    assert_ne!(rearmed.primary_worker.id, scheduled.primary_worker.id);
    assert_eq!(
        rearmed.primary_message_chain_id,
        scheduled.primary_message_chain_id
    );
    assert_eq!(
        rearmed.primary_worker.message_chain_id,
        Some(scheduled.primary_message_chain_id)
    );
    let claimed_rearmed_primary = claim_stage_team_leader(
        db.pool(),
        &ClaimStageTeamLeaderRow {
            claim: ClaimStageWorkItemRow {
                operation_id,
                stage_execution_id,
                stage_run_unit_id,
                stage_team_plan_id,
                exact_work_item_id: Some(rearmed.primary_work_item.id),
                lease_owner: "asset-primary-rearm-claim".to_string(),
                lease_seconds: 60,
                session_id,
                subtask_id: None,
                agent: AgentType::Pentester,
                model: Some("test-model".to_string()),
                provider: Some("test-provider".to_string()),
                parent_chain_id: None,
                initial_chain: json!([]),
                initial_checkpoint: json!([]),
            },
        },
    )
    .await
    .expect("claim fresh rearmed Asset Primary")
    .expect("fresh rearmed Asset Primary must be claimable");
    assert_eq!(
        claimed_rearmed_primary.work_item.id,
        rearmed.primary_work_item.id
    );
    assert_eq!(claimed_rearmed_primary.worker.id, rearmed.primary_worker.id);
    assert_eq!(
        claimed_rearmed_primary.message_chain_id,
        rearmed.primary_message_chain_id
    );
    // Freeze the torn Generator source on generation one. The adoption below
    // occurs only after generation two becomes the current consumer, proving
    // that a task-plan dispatch may remain bound to an exact ancestor without
    // reusing one Worker across two logical dispatch receipts.
    let orphan_task_plan_id = Uuid::new_v4();
    let orphan_dispatch_receipt_id = Uuid::new_v4();
    let orphan_subtask_id = Uuid::new_v4();
    let orphan_tool_call_id = Uuid::new_v4();
    let orphan_provider_call_id = format!("generator-orphan-{orphan_tool_call_id}");
    let orphan_manifest = json!({
        "contract_version":"dynamic_refiner_v2",
        "generator":"retained_orphan",
        "subtasks":[{"stable_key":"retained orphan Generator member"}]
    });
    let mut orphan_fixture = db.pool().begin().await.expect("begin orphan fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *orphan_fixture)
        .await
        .expect("isolate retained orphan fixture triggers");
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_task_plans(
               task_plan_id,stable_request_id,authority_id,stage_team_plan_id,
               operation_id,stage_execution_id,owning_stage_run_request_id,
               stage_run_unit_id,scope_snapshot_id,organization_id,subject_kind,
               subject_id,subject_fingerprint_sha256,task_plan_version,task_plan_sha256,
               allowed_role_catalog,cognitive_tool_envelope_sha256,status,row_version)
           VALUES($1,$2,$3,$4,$5,$6,'asset-schedule',$7,$8,$9,'analysis_attempt',
                  $10,$11,1,$12,'["primary","refiner"]'::JSONB,$13,'open',0)"#,
    )
    .bind(orphan_task_plan_id)
    .bind(Uuid::new_v4())
    .bind(authority_id)
    .bind(stage_team_plan_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(Uuid::new_v4())
    .bind(digest('1'))
    .bind(digest('2'))
    .bind(digest('3'))
    .execute(&mut *orphan_fixture)
    .await
    .expect("seed orphan task plan");
    sqlx::query(
        r#"INSERT INTO pentagi_logical_dispatch_receipts(
               dispatch_receipt_id,stable_request_id,logical_dispatch_key_sha256,
               task_plan_id,dispatch_ordinal,actor_kind,stage_work_item_id,worker_run_id,
               operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
               organization_id,transcript_request_id,snapshot_sha256,receipt_sha256)
           VALUES($1,$2,$3,$4,0,'primary',$5,$6,$7,$8,$9,$10,$11,
                  'asset-primary-orphan-recovery',$12,$13)"#,
    )
    .bind(orphan_dispatch_receipt_id)
    .bind(Uuid::new_v4())
    .bind(format!(
        "sha256:{0}{0}",
        orphan_dispatch_receipt_id.simple()
    ))
    .bind(orphan_task_plan_id)
    .bind(rearmed.primary_work_item.id)
    .bind(rearmed.primary_worker.id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(digest('5'))
    .bind(digest('6'))
    .execute(&mut *orphan_fixture)
    .await
    .expect("seed generation-one orphan dispatch");
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_subtasks(
               subtask_id,task_plan_id,authority_id,operation_id,stage_execution_id,
               stage_run_unit_id,organization_id,subtask_ordinal,label,runnable,
               input_manifest_sha256,expected_output_schema,member_sha256)
           VALUES($1,$2,$3,$4,$5,$6,$7,0,'retained orphan Generator member',TRUE,$8,
                  'investigation_cognitive_output.v1',$9)"#,
    )
    .bind(orphan_subtask_id)
    .bind(orphan_task_plan_id)
    .bind(authority_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(organization_id)
    .bind(digest('3'))
    .bind(digest('4'))
    .execute(&mut *orphan_fixture)
    .await
    .expect("seed retained orphan Generator member");
    orphan_fixture
        .commit()
        .await
        .expect("commit generation-one orphan fixture");
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,result,status,source,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token)
           VALUES($1,$2,$3,$4,'pentester','submit_result',$5,$6,'finished','ai',
                  $4,$7,$8,$9,$10,$11,$12)"#,
    )
    .bind(orphan_tool_call_id)
    .bind(&orphan_provider_call_id)
    .bind(session_id)
    .bind(operation_id)
    .bind(json!({"result": orphan_manifest.clone()}))
    .bind(json!({"status":"result submitted"}).to_string())
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(rearmed.primary_worker.id)
    .bind(organization_id)
    .bind(claimed_rearmed_primary.worker.attempt_epoch)
    .bind(
        claimed_rearmed_primary
            .worker
            .lease_token
            .expect("generation-one Primary lease"),
    )
    .execute(db.pool())
    .await
    .expect("record generation-one retained Generator source");
    let mut park_orphan_plan = db.pool().begin().await.expect("park orphan task plan");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *park_orphan_plan)
        .await
        .expect("isolate orphan plan parking transition");
    sqlx::query(
        "UPDATE investigation_pentagi_task_plans
            SET status='sealed',subtask_count=1,subtask_set_sha256=$2,
                row_version=row_version+1,sealed_at=NOW()
          WHERE task_plan_id=$1",
    )
    .bind(orphan_task_plan_id)
    .bind(digest('4'))
    .execute(&mut *park_orphan_plan)
    .await
    .expect("park orphan task plan outside rearm open-plan census");
    park_orphan_plan
        .commit()
        .await
        .expect("commit parked orphan task plan");
    let dispatch_epoch_census: (i64, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT COUNT(*) FROM stage_work_items
               WHERE team_plan_id=$1 AND created_by='accepted_worker_request'
                 AND dispatch_epoch=$2),
             (SELECT COUNT(*) FROM stage_work_items
               WHERE team_plan_id=$1 AND created_by='accepted_worker_request'
                 AND dispatch_epoch=$3)"#,
    )
    .bind(stage_team_plan_id)
    .bind(rearmed.plan.dispatch_epoch)
    .bind(scheduled.plan.dispatch_epoch)
    .fetch_one(db.pool())
    .await
    .expect("count old and current dynamic child epochs");
    assert_eq!(
        dispatch_epoch_census,
        (0, 4),
        "the prior epoch retains the four completed direct children used by the dynamic census"
    );
    let unchanged_source_output: (Uuid, Uuid, Uuid, String) = sqlx::query_as(
        "SELECT id,work_item_id,worker_run_id,output_hash FROM stage_worker_outputs WHERE id=$1",
    )
    .bind(exhaustion_output_id)
    .fetch_one(db.pool())
    .await
    .expect("reload immutable source exhaustion output");
    assert_eq!(unchanged_source_output.0, exhaustion_output_id);
    assert_eq!(unchanged_source_output.1, scheduled.primary_work_item.id);
    assert_eq!(unchanged_source_output.2, scheduled.primary_worker.id);
    assert_eq!(unchanged_source_output.3, digest('e'));

    let rearm_replay = ensure_investigation_asset_primary_schedule(db.pool(), &request)
        .await
        .expect("exact exhausted Primary rearm replay");
    assert!(rearm_replay.replayed);
    assert_eq!(
        rearm_replay.execution_rearm_receipt_id,
        rearmed.execution_rearm_receipt_id
    );
    assert_eq!(
        rearm_replay.primary_work_item.id,
        rearmed.primary_work_item.id
    );

    let closed_rearm = close_stage_request_epoch(
        db.pool(),
        &golish_db::repo::runtime_memory_tx::CloseStageRequestEpochRow {
            operation_id,
            stage_execution_id,
            stage_run_unit_id,
            stage_team_plan_id,
            expected_dispatch_epoch: rearmed.plan.dispatch_epoch,
            expected_plan_row_version: rearmed.plan.row_version,
        },
    )
    .await
    .expect("close fresh rearmed Primary request epoch");
    assert_eq!(
        closed_rearm.barrier.dispatch_epoch,
        rearmed.plan.dispatch_epoch
    );
    assert_eq!(closed_rearm.barrier.live_workers, 0);
    assert_eq!(closed_rearm.barrier.recovery_required_workers, 0);
    assert_eq!(closed_rearm.barrier.missing_outputs, 1);
    assert!(
        !closed_rearm.barrier.ready_to_finalize(),
        "a claimed but incomplete recovery Primary must remain an explicit barrier member"
    );
    let closed_replay_census_before: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM stage_work_items WHERE team_plan_id=$1")
            .bind(stage_team_plan_id)
            .fetch_one(db.pool())
            .await
            .expect("count closed dynamic plan items before replay");
    let closed_dynamic_replay = seed_stage_team_runtime(db.pool(), &current_governance_seed)
        .await
        .expect("replay exact closed dynamic plan with completed accepted children");
    assert!(closed_dynamic_replay[0].replayed);
    assert!(closed_dynamic_replay[0].plan.requests_closed_at.is_some());
    let closed_replay_census_after: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM stage_work_items WHERE team_plan_id=$1")
            .bind(stage_team_plan_id)
            .fetch_one(db.pool())
            .await
            .expect("count closed dynamic plan items after replay");
    assert_eq!(closed_replay_census_after, closed_replay_census_before);
    let foreign_rearm = ensure_investigation_asset_primary_schedule(
        db.pool(),
        &EnsureInvestigationAssetPrimaryScheduleRow {
            target_id: Uuid::new_v4(),
            ..request.clone()
        },
    )
    .await;
    assert!(
        foreign_rearm.is_err(),
        "foreign target must not replay a rearm"
    );

    let successor_output_id = Uuid::new_v4();
    let mut second_exhaustion = db
        .pool()
        .begin()
        .await
        .expect("begin successor exhaustion fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *second_exhaustion)
        .await
        .expect("isolate successor terminal fixture triggers");
    sqlx::query(
        "UPDATE stage_worker_runs SET status='failed',attempt_epoch=3,
                checkpoint_version=3,terminal_at=NOW(),lease_token=NULL,lease_owner=NULL,
                lease_acquired_at=NULL,lease_expires_at=NULL,heartbeat_at=NULL,
                active_tool_call_id=NULL,active_tool_started_at=NULL,updated_at=NOW() WHERE id=$1",
    )
    .bind(rearmed.primary_worker.id)
    .execute(&mut *second_exhaustion)
    .await
    .expect("exhaust successor worker fixture");
    sqlx::query(
        "UPDATE stage_work_items SET status='exhausted',terminal_at=NOW(),
                row_version=row_version+1,updated_at=NOW() WHERE id=$1",
    )
    .bind(rearmed.primary_work_item.id)
    .execute(&mut *second_exhaustion)
    .await
    .expect("exhaust successor item fixture");
    sqlx::query(
        r#"INSERT INTO stage_worker_outputs(
               id,team_plan_id,work_item_id,worker_run_id,operation_id,stage_execution_id,
               stage_run_unit_id,scope_snapshot_id,organization_id,output_schema,output_version,
               business_disposition,canonical_output,canonical_fact_refs,evidence_ids,
               checked_empty_cells,blocker_codes,output_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'stage_unit_aggregate.v1',1,'blocked',
             '{"kind":"stage_team_attempts_exhausted","failure_code":"stage_team_worker_lease_expired","schema_version":1}'::JSONB,
             '[]'::JSONB,ARRAY[]::BIGINT[],'[]'::JSONB,
             ARRAY['STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED']::TEXT[],$10)"#,
    )
    .bind(successor_output_id)
    .bind(stage_team_plan_id)
    .bind(rearmed.primary_work_item.id)
    .bind(rearmed.primary_worker.id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(digest('9'))
    .execute(&mut *second_exhaustion)
    .await
    .expect("insert successor exhaustion output fixture");
    second_exhaustion
        .commit()
        .await
        .expect("commit successor exhaustion fixture");
    let second_rearm = ensure_investigation_asset_primary_schedule(db.pool(), &request)
        .await
        .expect("a terminal successor must receive the next bounded execution shell");
    assert!(!second_rearm.replayed);
    assert_eq!(second_rearm.execution_ordinal, 2);
    assert_eq!(
        second_rearm.primary_message_chain_id,
        rearmed.primary_message_chain_id
    );
    assert_ne!(
        second_rearm.execution_rearm_receipt_id,
        rearmed.execution_rearm_receipt_id
    );
    assert_eq!(
        second_rearm.primary_worker.worker_generation,
        rearmed.primary_worker.worker_generation + 1
    );
    let expected_infrastructure_parent = format!(
        "investigation-task-primary-infrastructure-recovery:{}",
        rearmed.primary_worker.id
    );
    assert_eq!(
        second_rearm.primary_worker.parent_request_id.as_deref(),
        Some(expected_infrastructure_parent.as_str())
    );
    let second_replay = ensure_investigation_asset_primary_schedule(db.pool(), &request)
        .await
        .expect("exact second-generation response-loss replay");
    assert!(second_replay.replayed);
    assert_eq!(
        second_replay.execution_rearm_receipt_id,
        second_rearm.execution_rearm_receipt_id
    );
    assert_eq!(
        second_replay.primary_worker.id,
        second_rearm.primary_worker.id
    );
    let rearm_census: (i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT COUNT(*) FROM investigation_asset_primary_rearms
               WHERE source_schedule_receipt_id=(
                 SELECT source_schedule_receipt_id FROM investigation_asset_primary_rearms
                  WHERE rearm_receipt_id=$1)),
             (SELECT COUNT(*) FROM stage_work_items WHERE team_plan_id=$2),
             (SELECT COUNT(*) FROM stage_worker_runs WHERE stage_run_unit_id=$3),
             (SELECT COUNT(*) FROM pentagi_logical_dispatch_receipts
               WHERE dispatch_receipt_id=$4 AND worker_run_id=$5)"#,
    )
    .bind(rearmed.execution_rearm_receipt_id.expect("rearm receipt"))
    .bind(stage_team_plan_id)
    .bind(stage_run_unit_id)
    .bind(primary_dispatch_receipt_id)
    .bind(claimed_primary.worker.id)
    .fetch_one(db.pool())
    .await
    .expect("count multi-generation rearm census");
    assert_eq!(rearm_census, (2, 12, 8, 1));
    let current_authority: (i32, Uuid, Uuid, Uuid) = sqlx::query_as(
        r#"SELECT execution_ordinal,execution_rearm_receipt_id,
                  authority_primary_worker_run_id,primary_worker_run_id
             FROM investigation_asset_primary_current_authorities
            WHERE asset_lane_id=$1 AND evolution_epoch=0"#,
    )
    .bind(asset_lane_id)
    .fetch_one(db.pool())
    .await
    .expect("load unique latest multi-generation authority");
    assert_eq!(current_authority.0, 2);
    assert_eq!(
        current_authority.1,
        second_rearm
            .execution_rearm_receipt_id
            .expect("second rearm receipt")
    );
    assert_eq!(current_authority.2, rearmed.primary_worker.id);
    assert_eq!(current_authority.3, second_rearm.primary_worker.id);
    let lineage_authority: (bool, bool, bool) = sqlx::query_as(
        r#"SELECT
             investigation_asset_primary_dispatch_in_current_lineage(
                 $1,$2,$3,$4,$5,$6,$7),
             investigation_asset_primary_dispatch_in_current_lineage(
                 $1,$2,$3,$4,$5,$6,$8),
             investigation_asset_primary_dispatch_in_current_lineage(
                 $1,$2,$3,$4,$5,$6,$9)"#,
    )
    .bind(stage_team_plan_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(claimed_primary.worker.id)
    .bind(rearmed.primary_worker.id)
    .bind(Uuid::new_v4())
    .fetch_one(db.pool())
    .await
    .expect("verify root and immediate predecessor task-plan dispatch ancestry");
    assert_eq!(lineage_authority, (true, true, false));
    let immutable_root_schedule: (Uuid, Uuid, Uuid, String, String) = sqlx::query_as(
        r#"SELECT schedule.primary_work_item_id,schedule.primary_worker_run_id,
                  schedule.primary_message_chain_id,schedule.receipt_sha256,schedule.status
             FROM investigation_asset_primary_schedules schedule
             JOIN investigation_asset_primary_rearms rearm
               ON rearm.source_schedule_receipt_id=schedule.schedule_receipt_id
            WHERE rearm.rearm_receipt_id=$1"#,
    )
    .bind(
        rearmed
            .execution_rearm_receipt_id
            .expect("first rearm receipt"),
    )
    .fetch_one(db.pool())
    .await
    .expect("reload immutable root schedule after two continuation shells");
    assert_eq!(immutable_root_schedule.0, scheduled.primary_work_item.id);
    assert_eq!(immutable_root_schedule.1, scheduled.primary_worker.id);
    assert_eq!(
        immutable_root_schedule.2,
        scheduled.primary_message_chain_id
    );
    assert!(!immutable_root_schedule.3.is_empty());
    assert_eq!(immutable_root_schedule.4, "applied");
    let immutable_successor_source: (Uuid, Uuid, String) = sqlx::query_as(
        "SELECT work_item_id,worker_run_id,output_hash FROM stage_worker_outputs WHERE id=$1",
    )
    .bind(successor_output_id)
    .fetch_one(db.pool())
    .await
    .expect("reload immutable generation-one exhaustion source");
    assert_eq!(immutable_successor_source.0, rearmed.primary_work_item.id);
    assert_eq!(immutable_successor_source.1, rearmed.primary_worker.id);
    assert_eq!(immutable_successor_source.2, digest('9'));

    let mut reopen_orphan_plan = db.pool().begin().await.expect("reopen orphan task plan");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *reopen_orphan_plan)
        .await
        .expect("isolate orphan plan recovery transition");
    sqlx::query(
        "UPDATE investigation_pentagi_task_plans
            SET status='open',subtask_count=NULL,subtask_set_sha256=NULL,
                row_version=row_version+1,sealed_at=NULL
          WHERE task_plan_id=$1",
    )
    .bind(orphan_task_plan_id)
    .execute(&mut *reopen_orphan_plan)
    .await
    .expect("reopen orphan task plan under current execution head");
    reopen_orphan_plan
        .commit()
        .await
        .expect("commit reopened orphan task plan");

    let claimed_second_primary = claim_stage_team_leader(
        db.pool(),
        &ClaimStageTeamLeaderRow {
            claim: ClaimStageWorkItemRow {
                operation_id,
                stage_execution_id,
                stage_run_unit_id,
                stage_team_plan_id,
                exact_work_item_id: Some(second_rearm.primary_work_item.id),
                lease_owner: "asset-primary-second-rearm-claim".to_string(),
                lease_seconds: 60,
                session_id,
                subtask_id: None,
                agent: AgentType::Pentester,
                model: Some("test-model".to_string()),
                provider: Some("test-provider".to_string()),
                parent_chain_id: None,
                initial_chain: json!([]),
                initial_checkpoint: json!([]),
            },
        },
    )
    .await
    .expect("claim generation-two current Primary")
    .expect("generation-two current Primary must be claimable");
    let pending = runtime
        .load_pending_generator_recovery(&generator_identity, orphan_task_plan_id)
        .await
        .expect("load pending ancestor Generator recovery")
        .expect("open orphan task plan has pending recovery view");
    assert_eq!(pending.primary_worker_run_id, rearmed.primary_worker.id);
    assert_eq!(pending.existing_subtasks.len(), 1);
    assert_eq!(pending.existing_subtasks[0].subtask_id, orphan_subtask_id);
    assert_eq!(pending.candidates.len(), 1);
    assert_eq!(
        pending.candidates[0].source_tool_call_id,
        orphan_tool_call_id
    );
    let orphan_ledger_id = Uuid::new_v4();
    let orphan_ledger_request_id = Uuid::new_v4();
    let orphan_event_id = Uuid::new_v4();
    let orphan_receipt_id = unified_stable_id(
        orphan_task_plan_id,
        "generator-orphan-adoption-receipt",
        &[orphan_tool_call_id.to_string().as_str()],
    );
    let orphan_adoption_request_id = unified_stable_id(
        orphan_task_plan_id,
        "adopt-generator-orphan",
        &[orphan_tool_call_id.to_string().as_str()],
    );
    let adoption = AdoptInvestigationOrphanGeneratorInput {
        identity: generator_identity.clone(),
        task_plan_id: orphan_task_plan_id,
        adoption_receipt_id: orphan_receipt_id,
        stable_request_id: orphan_adoption_request_id,
        ledger_id: orphan_ledger_id,
        ledger_stable_request_id: orphan_ledger_request_id,
        generator_pipeline_event_id: orphan_event_id,
        source_tool_call_id: orphan_tool_call_id,
        consumer_fence: InvestigationGeneratorConsumerFenceInput {
            current_consumer_work_item_id: claimed_second_primary.work_item.id,
            current_consumer_worker_run_id: claimed_second_primary.worker.id,
            current_consumer_lease_token: claimed_second_primary
                .worker
                .lease_token
                .expect("generation-two current Primary lease"),
            expected_consumer_attempt_epoch: claimed_second_primary.worker.attempt_epoch as u64,
            expected_consumer_checkpoint_version: claimed_second_primary.worker.checkpoint_version
                as u64,
        },
        expected_existing_subtask_ids: vec![orphan_subtask_id],
    };
    let adopted = runtime
        .adopt_orphan_generator(&adoption)
        .await
        .expect("adopt generation-one orphan under generation-two consumer fence");
    assert!(!adopted.replayed);
    assert_eq!(adopted.adoption_receipt_id, Some(orphan_receipt_id));
    assert_eq!(adopted.ledger.generator_manifest, orphan_manifest);
    assert_eq!(adopted.ledger.generator_subtask_count, 1);
    let adopted_replay = runtime
        .adopt_orphan_generator(&adoption)
        .await
        .expect("replay ancestor Generator adoption after response loss");
    assert!(adopted_replay.replayed);
    let adoption_census: (i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT COUNT(*) FROM investigation_generator_source_receipts
               WHERE source_receipt_id=$1 AND receipt_kind='orphan_adoption'),
             (SELECT COUNT(*) FROM investigation_refiner_plan_ledgers WHERE ledger_id=$2),
             (SELECT COUNT(*) FROM investigation_pentagi_pipeline_events
               WHERE pipeline_event_id=$3),
             (SELECT COUNT(*) FROM investigation_pentagi_subtasks WHERE task_plan_id=$4)"#,
    )
    .bind(orphan_receipt_id)
    .bind(orphan_ledger_id)
    .bind(orphan_event_id)
    .bind(orphan_task_plan_id)
    .fetch_one(db.pool())
    .await
    .expect("count exact ancestor orphan adoption rows");
    assert_eq!(adoption_census, (1, 1, 1, 1));

    let closed_post_synthesis = close_stage_request_epoch(
        db.pool(),
        &golish_db::repo::runtime_memory_tx::CloseStageRequestEpochRow {
            operation_id,
            stage_execution_id,
            stage_run_unit_id,
            stage_team_plan_id,
            expected_dispatch_epoch: claimed_second_primary.plan.dispatch_epoch,
            expected_plan_row_version: claimed_second_primary.plan.row_version,
        },
    )
    .await
    .expect("close the exact post-synthesis request epoch");
    let mut expired_post_synthesis = db
        .pool()
        .begin()
        .await
        .expect("begin expired post-synthesis fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *expired_post_synthesis)
        .await
        .expect("isolate expired post-synthesis lease fixture");
    sqlx::query(
        "UPDATE stage_worker_runs
            SET lease_acquired_at=NOW()-INTERVAL '3 seconds',
                heartbeat_at=NOW()-INTERVAL '2 seconds',
                lease_expires_at=NOW()-INTERVAL '1 second',updated_at=NOW()
          WHERE id=$1 AND status='running' AND active_tool_call_id IS NULL",
    )
    .bind(claimed_second_primary.worker.id)
    .execute(&mut *expired_post_synthesis)
    .await
    .expect("expire the current post-synthesis Primary lease");
    expired_post_synthesis
        .commit()
        .await
        .expect("commit expired post-synthesis lease fixture");
    let startup = tasks::startup_reap_abandoned(db.pool(), chrono::Duration::zero())
        .await
        .expect("startup reaper must preserve exact post-synthesis authority");
    assert_eq!(startup.workers_requeued, 0);
    let preserved: (String, String, i64, i64) = sqlx::query_as(
        r#"SELECT item.status,worker.status,item.row_version,worker.attempt_epoch
             FROM stage_work_items item
             JOIN stage_worker_runs worker ON worker.work_item_id=item.id
            WHERE item.id=$1 AND worker.id=$2"#,
    )
    .bind(claimed_second_primary.work_item.id)
    .bind(claimed_second_primary.worker.id)
    .fetch_one(db.pool())
    .await
    .expect("reload post-synthesis Primary after startup reaper");
    assert_eq!(preserved.0, "running");
    assert_eq!(preserved.1, "running");
    assert_eq!(preserved.2, claimed_second_primary.work_item.row_version);
    assert_eq!(preserved.3, claimed_second_primary.worker.attempt_epoch);
    let recovered_post_synthesis = claim_stage_team_leader(
        db.pool(),
        &ClaimStageTeamLeaderRow {
            claim: ClaimStageWorkItemRow {
                operation_id,
                stage_execution_id,
                stage_run_unit_id,
                stage_team_plan_id,
                exact_work_item_id: Some(claimed_second_primary.work_item.id),
                lease_owner: "post-synthesis-infrastructure-recovery".to_string(),
                lease_seconds: 60,
                session_id,
                subtask_id: None,
                agent: AgentType::Pentester,
                model: Some("test-model".to_string()),
                provider: Some("test-provider".to_string()),
                parent_chain_id: None,
                initial_chain: json!([]),
                initial_checkpoint: json!([]),
            },
        },
    )
    .await
    .expect("claim exact post-synthesis infrastructure recovery")
    .expect("exact post-synthesis current Primary must be recoverable");
    assert_eq!(
        recovered_post_synthesis.plan.id,
        closed_post_synthesis.plan.id
    );
    assert_eq!(
        recovered_post_synthesis.work_item.id,
        claimed_second_primary.work_item.id
    );
    assert_eq!(
        recovered_post_synthesis.worker.id,
        claimed_second_primary.worker.id
    );
    assert_eq!(
        recovered_post_synthesis.message_chain_id,
        second_rearm.primary_message_chain_id
    );
    assert_eq!(
        recovered_post_synthesis.worker.attempt_epoch,
        claimed_second_primary.worker.attempt_epoch + 1
    );
    let recovered_replay = claim_stage_team_leader(
        db.pool(),
        &ClaimStageTeamLeaderRow {
            claim: ClaimStageWorkItemRow {
                operation_id,
                stage_execution_id,
                stage_run_unit_id,
                stage_team_plan_id,
                exact_work_item_id: Some(claimed_second_primary.work_item.id),
                lease_owner: "post-synthesis-infrastructure-recovery".to_string(),
                lease_seconds: 60,
                session_id,
                subtask_id: None,
                agent: AgentType::Pentester,
                model: Some("test-model".to_string()),
                provider: Some("test-provider".to_string()),
                parent_chain_id: None,
                initial_chain: json!([]),
                initial_checkpoint: json!([]),
            },
        },
    )
    .await
    .expect("replay exact post-synthesis recovery claim")
    .expect("same-owner post-synthesis claim must replay");
    assert_eq!(
        recovered_replay.worker.attempt_epoch,
        recovered_post_synthesis.worker.attempt_epoch
    );

    let zero_task_plan_id = Uuid::new_v4();
    let zero_run_request_id = Uuid::new_v4();
    let zero_subject_id = Uuid::new_v4();
    let zero_primary_dispatch_id = Uuid::new_v4();
    let mut zero_fixture = db.pool().begin().await.expect("begin zero-plan fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *zero_fixture)
        .await
        .expect("isolate zero-plan authority fixture");
    sqlx::query(
        r#"INSERT INTO pentagi_task_run_requests(
               run_request_id,stable_request_id,task_plan_id,authority_id,operation_id,
               stage_execution_id,owning_stage_run_request_id,stage_run_unit_id,
               organization_id,subject_kind,subject_id,subject_fingerprint_sha256,request_sha256)
           VALUES($1,$2,NULL,$3,$4,$5,'asset-schedule',$6,$7,'analysis_attempt',$8,$9,$10)"#,
    )
    .bind(zero_run_request_id)
    .bind(Uuid::new_v4())
    .bind(authority_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(organization_id)
    .bind(zero_subject_id)
    .bind(digest('2'))
    .bind(digest('3'))
    .execute(&mut *zero_fixture)
    .await
    .expect("seed zero task run request");
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_task_plans(
               task_plan_id,stable_request_id,run_request_id,authority_id,stage_team_plan_id,
               operation_id,stage_execution_id,owning_stage_run_request_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,subject_kind,subject_id,
               subject_fingerprint_sha256,task_plan_version,task_plan_sha256,
               allowed_role_catalog,cognitive_tool_envelope_sha256,status,row_version)
           VALUES($1,$2,$3,$4,$5,$6,$7,'asset-schedule',$8,$9,$10,'analysis_attempt',
                  $11,$12,1,$13,'["primary","refiner"]'::JSONB,$14,'open',0)"#,
    )
    .bind(zero_task_plan_id)
    .bind(Uuid::new_v4())
    .bind(zero_run_request_id)
    .bind(authority_id)
    .bind(stage_team_plan_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(zero_subject_id)
    .bind(digest('2'))
    .bind(digest('4'))
    .bind(digest('5'))
    .execute(&mut *zero_fixture)
    .await
    .expect("seed open zero task plan");
    sqlx::query(
        r#"INSERT INTO pentagi_logical_dispatch_receipts(
               dispatch_receipt_id,stable_request_id,logical_dispatch_key_sha256,
               task_plan_id,dispatch_ordinal,actor_kind,stage_work_item_id,worker_run_id,
               operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
               organization_id,transcript_request_id,snapshot_sha256,receipt_sha256)
           VALUES($1,$2,$3,$4,0,'primary',$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"#,
    )
    .bind(zero_primary_dispatch_id)
    .bind(Uuid::new_v4())
    .bind(format!("sha256:{0}{0}", zero_primary_dispatch_id.simple()))
    .bind(zero_task_plan_id)
    .bind(second_rearm.primary_work_item.id)
    .bind(second_rearm.primary_worker.id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind("zero-plan-primary")
    .bind(digest('6'))
    .bind(digest('7'))
    .execute(&mut *zero_fixture)
    .await
    .expect("seed current zero-plan Primary dispatch");
    zero_fixture
        .commit()
        .await
        .expect("commit zero-plan fixture");

    let zero_ledger_id = Uuid::new_v4();
    let zero_ledger: (String,) = sqlx::query_as(
        "SELECT ledger_sha256 FROM create_investigation_refiner_plan_ledger_v2($1,$2,$3,$4,$5)",
    )
    .bind(zero_ledger_id)
    .bind(Uuid::new_v4())
    .bind(zero_task_plan_id)
    .bind(Uuid::new_v4())
    .bind(json!({"contract_version":"dynamic_refiner_v2","generator":"zero"}))
    .fetch_one(db.pool())
    .await
    .expect("create zero dynamic Refiner ledger");
    let zero_patch: (String,) = sqlx::query_as(
        r#"SELECT patch_sha256 FROM append_investigation_refiner_plan_patch_v2(
               $1,$2,$3,$4,$5,$6,$7,$8)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(zero_ledger_id)
    .bind(zero_task_plan_id)
    .bind(Uuid::new_v4())
    .bind(&zero_ledger.0)
    .bind(json!({"completed_subtask_ids":[]}))
    .bind(Vec::<Uuid>::new())
    .fetch_one(db.pool())
    .await
    .expect("append empty final Refiner patch");
    let _zero_refiner_seal: (String,) = sqlx::query_as(
        r#"SELECT seal_sha256 FROM seal_investigation_refiner_plan_ledger_v2(
               $1,$2,$3,$4,$5,$6)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(zero_ledger_id)
    .bind(zero_task_plan_id)
    .bind(Uuid::new_v4())
    .bind(&zero_patch.0)
    .fetch_one(db.pool())
    .await
    .expect("seal empty final Refiner denominator");
    let zero_synthesis_sha256 = digest('a');
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_pipeline_events(
               pipeline_event_id,stable_request_id,task_plan_id,subtask_id,event_ordinal,
               event_kind,actor_worker_run_id,parent_dispatch_receipt_id,event_sha256)
           VALUES($1,$2,$3,NULL,3,'primary_synthesis',$4,$5,$6)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(zero_task_plan_id)
    .bind(second_rearm.primary_worker.id)
    .bind(zero_primary_dispatch_id)
    .bind(&zero_synthesis_sha256)
    .execute(db.pool())
    .await
    .expect("persist zero-plan Primary synthesis witness");
    sqlx::query(
        r#"INSERT INTO pentagi_logical_dispatch_attempts(
               dispatch_attempt_id,stable_request_id,dispatch_receipt_id,attempt_epoch,
               lease_token,fence_sha256,outcome,result_sha256)
           VALUES($1,$2,$3,0,$4,$5,'completed',$6)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(zero_primary_dispatch_id)
    .bind(Uuid::new_v4())
    .bind(digest('8'))
    .bind(&zero_synthesis_sha256)
    .execute(db.pool())
    .await
    .expect("settle zero-plan Primary dispatch");
    let zero_census = runtime
        .seal_delegation_census(&SealPentagiDelegationCensusInput {
            identity: generator_identity.clone(),
            census_seal_id: Uuid::new_v4(),
            stable_request_id: Uuid::new_v4(),
            task_plan_id: zero_task_plan_id,
            primary_dispatch_receipt_id: zero_primary_dispatch_id,
            primary_worker_run_id: second_rearm.primary_worker.id,
            seal_sha256: digest('9'),
        })
        .await
        .expect("seal zero dynamic delegation census");
    assert_eq!(zero_census.runnable_subtask_count, 0);
    let sealed_zero_plan = runtime
        .seal_pentagi_plan(&generator_identity, zero_task_plan_id, 0)
        .await
        .expect("seal exact zero-subtask dynamic task plan");
    assert_eq!(sealed_zero_plan.status, "sealed");
    assert_eq!(sealed_zero_plan.subtask_count, Some(0));
    assert_eq!(
        runtime
            .seal_pentagi_plan(&generator_identity, zero_task_plan_id, 0)
            .await
            .expect("replay exact zero-subtask task plan"),
        sealed_zero_plan
    );

    // A retained fixed-roster Analysis plan that never dispatched a child or
    // ran a real tool may be atomically superseded by one lane-bound dynamic
    // Analysis work. The old plan remains open audit material, but every old
    // writer is fenced by the applied cutover receipt.
    let legacy_work_id = Uuid::new_v4();
    let legacy_work_key = digest('a');
    let legacy_external_identity = digest('b');
    let dynamic_work_id = Uuid::new_v4();
    let dynamic_work_key = digest('c');
    let dynamic_external_identity = digest('d');
    let cutover_request_id = Uuid::new_v4();
    let foreign_subject_id: Uuid = sqlx::query_scalar(
        "SELECT subject_id FROM investigation_pentagi_task_plans WHERE task_plan_id=$1",
    )
    .bind(foreign_task_plan_id)
    .fetch_one(db.pool())
    .await
    .expect("load retained fixed-plan subject");
    let mut cutover_fixture = db.pool().begin().await.expect("begin cutover fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *cutover_fixture)
        .await
        .expect("isolate retained fixed Analysis fixture");
    sqlx::query(
        r#"INSERT INTO investigation_run_work_items(
               work_id,asset_lane_id,stable_work_key_sha256,authority_id,operation_id,
               stage_execution_id,owning_stage_run_request_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,work_kind,external_identity_sha256,
               current_state,observed_stop_epoch)
           VALUES($1,$2,$3,$4,$5,$6,'asset-schedule',$7,$8,$9,'analysis',$10,'running',0)"#,
    )
    .bind(legacy_work_id)
    .bind(asset_lane_id)
    .bind(&legacy_work_key)
    .bind(authority_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(&legacy_external_identity)
    .execute(&mut *cutover_fixture)
    .await
    .expect("seed retained fixed Analysis work");
    sqlx::query(
        r#"INSERT INTO investigation_analysis_attempt_bindings(
               binding_id,stable_request_id,authority_id,operation_id,stage_execution_id,
               owning_stage_run_request_id,stage_run_unit_id,scope_snapshot_id,organization_id,
               work_id,candidate_snapshot_id,analysis_attempt_id)
           VALUES($1,$2,$3,$4,$5,'asset-schedule',$6,$7,$8,$9,$10,$11)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(authority_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(legacy_work_id)
    .bind(Uuid::new_v4())
    .bind(foreign_subject_id)
    .execute(&mut *cutover_fixture)
    .await
    .expect("bind retained work to its fixed plan subject");
    for ordinal in 0_i32..4 {
        let subtask_id = Uuid::new_v4();
        let member_sha256 = digest(char::from_digit((ordinal + 1) as u32, 16).unwrap());
        sqlx::query(
            r#"INSERT INTO investigation_pentagi_subtasks(
                   subtask_id,task_plan_id,authority_id,operation_id,stage_execution_id,
                   stage_run_unit_id,organization_id,subtask_ordinal,label,runnable,
                   input_manifest_sha256,expected_output_schema,member_sha256)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,TRUE,$10,
                      'investigation_cognitive_output.v1',$11)"#,
        )
        .bind(subtask_id)
        .bind(foreign_task_plan_id)
        .bind(authority_id)
        .bind(operation_id)
        .bind(stage_execution_id)
        .bind(stage_run_unit_id)
        .bind(organization_id)
        .bind(ordinal)
        .bind(format!("retained fixed role {ordinal}"))
        .bind(digest('e'))
        .bind(&member_sha256)
        .execute(&mut *cutover_fixture)
        .await
        .expect("seed retained fixed subtask");
    }
    let retained_subtask_set_sha256: String = sqlx::query_scalar(
        r#"SELECT unified_investigation_exact_set_hash(
               'investigation_refiner_generator_subtasks.v1',
               array_agg(subtask_id::TEXT || ':' || member_sha256 ORDER BY subtask_ordinal))
             FROM investigation_pentagi_subtasks WHERE task_plan_id=$1"#,
    )
    .bind(foreign_task_plan_id)
    .fetch_one(&mut *cutover_fixture)
    .await
    .expect("compute retained fixed denominator");
    let generator_event_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO investigation_pentagi_pipeline_events(
               pipeline_event_id,stable_request_id,task_plan_id,event_ordinal,event_kind,
               actor_worker_run_id,parent_dispatch_receipt_id,event_sha256)
           VALUES($1,$2,$3,0,'generator_sealed',$4,$5,$6)"#,
    )
    .bind(generator_event_id)
    .bind(Uuid::new_v4())
    .bind(foreign_task_plan_id)
    .bind(legacy_primary_worker_id)
    .bind(foreign_dispatch_receipt_id)
    .bind(digest('f'))
    .execute(&mut *cutover_fixture)
    .await
    .expect("seed retained Generator receipt");
    sqlx::query(
        r#"INSERT INTO investigation_refiner_plan_ledgers(
               ledger_id,stable_request_id,task_plan_id,authority_id,operation_id,
               stage_execution_id,owning_stage_run_request_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,generator_pipeline_event_id,
               generator_manifest,generator_manifest_sha256,generator_subtask_count,
               generator_subtask_set_sha256,ledger_sha256,ledger_contract)
           VALUES($1,$2,$3,$4,$5,$6,'asset-schedule',$7,$8,$9,$10,
                  '{"contract":"fixed_denominator_v1"}'::JSONB,$11,4,$12,$13,
                  'fixed_denominator_v1')"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(foreign_task_plan_id)
    .bind(authority_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(generator_event_id)
    .bind(digest('0'))
    .bind(&retained_subtask_set_sha256)
    .bind(digest('1'))
    .execute(&mut *cutover_fixture)
    .await
    .expect("seed retained fixed Refiner ledger");
    cutover_fixture
        .commit()
        .await
        .expect("commit retained fixed Analysis fixture");

    let source_diagnostic: serde_json::Value = sqlx::query_scalar(
        r#"SELECT jsonb_build_array(
            EXISTS(SELECT 1 FROM investigation_run_work_items work
                    WHERE work.work_id=$1 AND work.current_state='running'
                      AND work.head_version=0 AND work.latest_event_id IS NULL),
            EXISTS(SELECT 1 FROM investigation_analysis_attempt_bindings binding
                    WHERE binding.work_id=$1 AND binding.analysis_attempt_id=$2),
            EXISTS(SELECT 1 FROM investigation_pentagi_task_plans plan
                    WHERE plan.task_plan_id=$3 AND plan.subject_id=$2
                      AND plan.status='open' AND plan.row_version=0),
            EXISTS(SELECT 1 FROM investigation_asset_primary_schedules schedule
                    WHERE schedule.stage_team_plan_id=$4 AND schedule.asset_lane_id=$5
                      AND schedule.schedule_contract='fixed_roster_v1'
                      AND schedule.status='applied'),
            (SELECT COUNT(*)=1 FROM pentagi_logical_dispatch_receipts dispatch
              WHERE dispatch.task_plan_id=$3 AND dispatch.actor_kind='primary'
                AND dispatch.subtask_id IS NULL),
            (SELECT COUNT(*)=1 FROM investigation_pentagi_pipeline_events event
              WHERE event.task_plan_id=$3 AND event.event_kind='generator_sealed'
                AND event.event_ordinal=0 AND event.subtask_id IS NULL),
            (SELECT COUNT(*)=1 FROM investigation_refiner_plan_ledgers ledger
              WHERE ledger.task_plan_id=$3 AND ledger.ledger_contract='fixed_denominator_v1'
                AND ledger.generator_subtask_count=4),
            (SELECT COUNT(*)=4 FROM investigation_pentagi_subtasks subtask
              WHERE subtask.task_plan_id=$3 AND subtask.runnable),
            NOT EXISTS(SELECT 1 FROM pentagi_logical_dispatch_attempts attempt
              JOIN pentagi_logical_dispatch_receipts dispatch
                ON dispatch.dispatch_receipt_id=attempt.dispatch_receipt_id
             WHERE dispatch.task_plan_id=$3),
            NOT EXISTS(SELECT 1 FROM investigation_nested_dispatch_begins nested
                        WHERE nested.task_plan_id=$3),
            NOT EXISTS(SELECT 1 FROM investigation_nested_dispatch_finishes nested
                        WHERE nested.task_plan_id=$3),
            NOT EXISTS(SELECT 1 FROM investigation_refiner_plan_patches patch
                        WHERE patch.task_plan_id=$3)
             AND NOT EXISTS(SELECT 1 FROM investigation_refiner_plan_ledger_seals seal
                             WHERE seal.task_plan_id=$3),
            NOT EXISTS(SELECT 1 FROM tool_calls tool
              JOIN stage_worker_runs worker ON worker.id=tool.worker_run_id
              JOIN stage_work_items item ON item.id=worker.work_item_id
             WHERE (worker.id=$6 OR item.id=ANY($7::UUID[]))
               AND tool.name NOT IN('submit_result','update_plan')),
            EXISTS(SELECT 1 FROM investigation_pentagi_pipeline_events generator
              JOIN pentagi_logical_dispatch_receipts dispatch
                ON dispatch.dispatch_receipt_id=generator.parent_dispatch_receipt_id
               AND dispatch.worker_run_id=generator.actor_worker_run_id
             WHERE generator.task_plan_id=$3 AND dispatch.task_plan_id=$3
               AND generator.event_kind='generator_sealed'),
            EXISTS(SELECT 1 FROM investigation_refiner_plan_ledgers ledger
              JOIN investigation_pentagi_pipeline_events generator
                ON generator.pipeline_event_id=ledger.generator_pipeline_event_id
             WHERE ledger.task_plan_id=$3 AND generator.task_plan_id=$3
               AND ledger.generator_subtask_set_sha256=$8),
            EXISTS(SELECT 1 FROM investigation_asset_primary_schedules schedule
              JOIN investigation_pentagi_task_plans plan
                ON plan.stage_team_plan_id=schedule.stage_team_plan_id
             WHERE plan.task_plan_id=$3 AND schedule.asset_lane_id=$5
               AND ROW(schedule.operation_id,schedule.stage_execution_id,
                       schedule.stage_run_unit_id,schedule.scope_snapshot_id,
                       schedule.organization_id)
                   = ROW(plan.operation_id,plan.stage_execution_id,
                         plan.stage_run_unit_id,plan.scope_snapshot_id,
                         plan.organization_id)),
            EXISTS(SELECT 1 FROM investigation_run_work_items work
              WHERE work.work_id=$1 AND work.asset_lane_id=$5
                AND ROW(work.authority_id,work.operation_id,work.stage_execution_id,
                        work.stage_run_unit_id,work.scope_snapshot_id,work.organization_id)
                    = ROW($9::UUID,$10::UUID,$11::UUID,$12::UUID,$13::UUID,$14::UUID)),
            $8=(SELECT unified_investigation_exact_set_hash(
                    'investigation_refiner_generator_subtasks.v1',
                    COALESCE(array_agg(subtask.subtask_id::TEXT || ':' || subtask.member_sha256
                                       ORDER BY subtask.subtask_ordinal),ARRAY[]::TEXT[]))
                  FROM investigation_pentagi_subtasks subtask
                 WHERE subtask.task_plan_id=$3))"#,
    )
    .bind(legacy_work_id)
    .bind(foreign_subject_id)
    .bind(foreign_task_plan_id)
    .bind(stage_team_plan_id)
    .bind(asset_lane_id)
    .bind(legacy_primary_worker_id)
    .bind(legacy_role_ids.to_vec())
    .bind(&retained_subtask_set_sha256)
    .bind(authority_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .fetch_one(db.pool())
    .await
    .expect("diagnose retained cutover source");
    assert_eq!(
        source_diagnostic,
        json!([
            true, true, true, true, true, true, true, true, true, true, true, true, true, true,
            true, true, true, true,
        ]),
        "retained cutover source predicate diagnostic"
    );

    let runtime = PgUnifiedInvestigationRuntimeRepository::new(Arc::new(db.pool().clone()));
    let cutover = EnsureDynamicAssetAnalysisWorkInput {
        identity: InvestigationUnitIdentity {
            stage: InvestigationStageIdentity {
                authority_id,
                operation_id,
                stage_execution_id,
                owning_stage_run_request_id: "asset-schedule".to_string(),
                scope_snapshot_id,
            },
            stage_run_unit_id,
            organization_id,
        },
        stable_cutover_request_id: cutover_request_id,
        asset_lane_id,
        legacy_stable_work_key_sha256: legacy_work_key,
        dynamic_work_id,
        dynamic_stable_work_key_sha256: dynamic_work_key,
        dynamic_external_identity_sha256: dynamic_external_identity,
        observed_stop_epoch: 0,
    };
    let ensured = runtime
        .ensure_dynamic_asset_analysis_work(&cutover)
        .await
        .expect("atomically cut retained fixed Analysis over to dynamic work");
    assert_eq!(ensured.work.work_id, dynamic_work_id);
    assert!(ensured.cutover_authority_id.is_some());
    assert_eq!(
        runtime
            .ensure_dynamic_asset_analysis_work(&cutover)
            .await
            .expect("exact cutover replay"),
        ensured
    );
    let cutover_states: (String, String, String) = sqlx::query_as(
        r#"SELECT legacy.current_state,dynamic.current_state,cutover.status
             FROM investigation_dynamic_analysis_work_cutovers cutover
             JOIN investigation_run_work_items legacy ON legacy.work_id=cutover.legacy_work_id
             JOIN investigation_run_work_items dynamic ON dynamic.work_id=cutover.dynamic_work_id
            WHERE cutover.stable_request_id=$1"#,
    )
    .bind(cutover_request_id)
    .fetch_one(db.pool())
    .await
    .expect("load applied cutover states");
    assert_eq!(
        cutover_states,
        ("superseded".into(), "running".into(), "applied".into())
    );
    let mut drift = cutover.clone();
    drift.dynamic_external_identity_sha256 = digest('2');
    assert!(runtime
        .ensure_dynamic_asset_analysis_work(&drift)
        .await
        .expect_err("stable cutover request drift must fail closed")
        .to_string()
        .contains("CUTOVER_REPLAY_MISMATCH"));
    let old_dispatch_error = sqlx::query(
        r#"INSERT INTO pentagi_logical_dispatch_attempts(
               dispatch_attempt_id,stable_request_id,dispatch_receipt_id,attempt_epoch,
               lease_token,fence_sha256,outcome,result_sha256)
           VALUES($1,$2,$3,0,$4,$5,'completed',$6)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(foreign_dispatch_receipt_id)
    .bind(Uuid::new_v4())
    .bind(digest('3'))
    .bind(digest('4'))
    .execute(db.pool())
    .await
    .expect_err("applied cutover must fence the historical fixed dispatch");
    assert!(old_dispatch_error
        .to_string()
        .contains("INVESTIGATION_FIXED_ANALYSIS_PLAN_SUPERSEDED"));

    let foreign_extra_item_id = Uuid::new_v4();
    let mut foreign_extra = db.pool().begin().await.expect("begin foreign replay extra");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *foreign_extra)
        .await
        .expect("isolate foreign replay fixture");
    sqlx::query(
        r#"INSERT INTO stage_work_items(
               id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
               input_manifest_hash,input_refs,required_for_barrier,priority,status,
               attempt_policy,budget,output_schema,created_by,terminal_at)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,'analysis_task',$9,'browser',$10,
                  '[]'::JSONB,TRUE,99,'completed','{"max_attempts":1}'::JSONB,
                  '{}'::JSONB,'investigation_cognitive_output.v1',
                  'accepted_worker_request',NOW())"#,
    )
    .bind(foreign_extra_item_id)
    .bind(stage_team_plan_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(second_rearm.plan.dispatch_epoch)
    .bind(format!("foreign-replay-extra:{foreign_extra_item_id}"))
    .bind(digest('f'))
    .execute(&mut *foreign_extra)
    .await
    .expect("seed unbacked foreign replay extra");
    foreign_extra
        .commit()
        .await
        .expect("commit unbacked foreign replay extra");
    let foreign_replay = seed_stage_team_runtime(db.pool(), &current_governance_seed)
        .await
        .expect_err("an unbacked accepted child must fail exact closed-plan replay");
    assert!(foreign_replay
        .to_string()
        .contains("stage_team_dynamic_work_item_authority_missing"));
    db.stop().await;
}
