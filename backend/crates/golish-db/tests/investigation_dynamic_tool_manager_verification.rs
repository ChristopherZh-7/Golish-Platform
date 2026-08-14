const MIGRATION: &str = include_str!(
    "../migrations/20260813000008_investigation_dynamic_tool_manager_verification.sql"
);
const PENDING_DISCOVERY_SNAPSHOT_SOURCE_MIGRATION: &str = include_str!(
    "../migrations/20260813000009_candidate_analysis_pending_hypothesis_discovery_source.sql"
);
const GENERIC_ASSET_LANE_GUARD_MIGRATION: &str = include_str!(
    "../migrations/20260813000010_investigation_asset_lane_guard_generic_work_kind.sql"
);
const DYNAMIC_PRIMARY_MIGRATION: &str =
    include_str!("../migrations/20260814000001_investigation_asset_primary_dynamic_schedule.sql");
const DYNAMIC_VERIFICATION_MIGRATION: &str =
    include_str!("../migrations/20260814000002_investigation_dynamic_verification_rounds.sql");
const NATIVE_RUNTIME_VERSION_MIGRATION: &str =
    include_str!("../migrations/20260814000012_investigation_native_tool_runtime_version.sql");
const CURRENT_PRIMARY_ROUND_MIGRATION: &str =
    include_str!("../migrations/20260814000015_investigation_dynamic_round_current_primary.sql");
const REPO: &str = include_str!("../src/repo/investigation_asset_verification.rs");
const PORT: &str =
    include_str!("../../golish-agent-kit/src/db_traits/investigation_asset_verification.rs");

use golish_db::repo::investigation_asset_verification as verification;
use golish_db::repo::runtime_memory_tx::{
    ensure_investigation_asset_primary_schedule, seed_stage_team_runtime,
    EnsureInvestigationAssetPrimaryScheduleRow, SeedStageRuntimeRow, SeedStageTeamRuntimeRow,
    StageTeamPlanSeedRow,
};
use golish_db::{DbConfig, GolishDb};
use serde_json::json;
use serial_test::serial;
use uuid::Uuid;

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve port")
        .local_addr()
        .expect("local addr")
        .port()
}

fn digest(nibble: char) -> String {
    format!("sha256:{}", nibble.to_string().repeat(64))
}

#[test]
fn dynamic_verification_has_no_fixed_capability_or_campaign_gate() {
    assert!(!MIGRATION.contains("prepared_action_id"));
    assert!(!MIGRATION.contains("verification_campaigns"));
    assert!(!MIGRATION.contains("campaign_id"));
    assert!(MIGRATION.contains("investigation_asset_verification_authorizations"));
    assert!(MIGRATION.contains("investigation_asset_verification_budget_envelopes"));
    assert!(MIGRATION.contains("allowed_effect_classes"));
    assert!(MIGRATION.contains("allowed_credential_binding_sha256s"));
    assert!(MIGRATION.contains("CHECK(jsonb_typeof(allowed_credential_binding_sha256s)='array')"));
    assert!(!MIGRATION.contains(
        "CHECK(stage_team_json_string_array_is_valid(allowed_credential_binding_sha256s))"
    ));
    assert!(!MIGRATION.contains("jsonb_array_length(NEW.allowed_credential_binding_sha256s)=0"));
}

#[test]
fn dynamic_inventory_and_invocation_are_unbounded_by_tool_kind() {
    assert!(MIGRATION.contains("investigation_dynamic_tool_inventory_snapshots"));
    assert!(MIGRATION.contains("installed BOOLEAN NOT NULL CHECK(installed)"));
    assert!(MIGRATION.contains("environment_ready BOOLEAN NOT NULL CHECK(environment_ready)"));
    assert!(MIGRATION.contains("selected_tool_name TEXT"));
    assert!(!MIGRATION.contains("selected_tool_name IN("));
    assert!(REPO.contains("pub async fn begin_invocation("));
    assert!(REPO.contains("pub async fn complete_invocation("));
    assert!(REPO.contains("pub async fn load_invocation_guard("));
    assert!(NATIVE_RUNTIME_VERSION_MIGRATION
        .contains("CHECK(runtime='native' OR BTRIM(runtime_version)<>'')"));
    assert!(!NATIVE_RUNTIME_VERSION_MIGRATION.contains("runtime_version='unknown'"));
}

#[test]
fn resolution_is_independent_of_tool_count_and_discovers_more_hypotheses() {
    assert!(MIGRATION.contains("disposition IN('verified','refuted','invalid')"));
    assert!(!MIGRATION.contains("IN('open','inconclusive','blocked')"));
    let resolution = MIGRATION
        .split_once("CREATE TABLE investigation_hypothesis_resolution_authorities")
        .expect("resolution")
        .1;
    assert!(!resolution
        .split_once(");")
        .expect("table end")
        .0
        .contains("invocation_count"));
    assert!(PORT.contains("new_hypothesis_proposals"));
    assert!(MIGRATION.contains("investigation_pending_hypothesis_discoveries"));
    assert!(MIGRATION.contains("investigation_pending_hypothesis_discovery_consumptions"));
    assert!(MIGRATION.contains("investigation_pending_hypothesis_discovery_backlog"));
    assert!(PORT.contains("list_pending_hypothesis_discoveries"));
    assert!(PORT.contains("admit_or_dismiss_pending_hypothesis_discovery"));
    let admission = PORT
        .split_once("pub struct AdmitOrDismissInvestigationPendingHypothesisDiscovery")
        .expect("server-owned discovery admission DTO")
        .1
        .split_once('}')
        .expect("server-owned discovery admission DTO end")
        .0;
    assert!(!admission.contains("admitted_root_id"));
    assert!(!admission.contains("compiler_receipt_id"));
    assert!(!DYNAMIC_VERIFICATION_MIGRATION.contains("adviser_review_output_id"));
    assert!(!DYNAMIC_VERIFICATION_MIGRATION.contains("adviser_worker_fence"));
    assert!(!PORT.contains("adviser_worker_fence"));
    assert!(PENDING_DISCOVERY_SNAPSHOT_SOURCE_MIGRATION.contains("'pending_hypothesis_discovery'"));
    assert!(GENERIC_ASSET_LANE_GUARD_MIGRATION
        .contains("row_work_kind TEXT := to_jsonb(NEW)->>'work_kind'"));
    assert!(!GENERIC_ASSET_LANE_GUARD_MIGRATION.contains("AND NEW.work_kind"));
}

#[test]
fn next_hypothesis_is_server_selected_from_the_current_asset_head() {
    assert!(PORT.contains("load_next_unresolved_current_hypothesis"));
    assert!(PORT.contains("LoadNextInvestigationAssetVerificationCandidate"));
    let selector = REPO
        .split_once("pub async fn load_next_unresolved_current_hypothesis(")
        .expect("server-owned candidate selector")
        .1;
    assert!(selector.contains("head.head_lifecycle_state='current'"));
    assert!(selector.contains("revision.epistemic_state NOT IN('verified','refuted','invalid')"));
    assert!(selector.contains("lane.operation_id=$1 AND lane.asset_lane_id=$2"));
    assert!(selector.contains("dynamic_round.session_id AS existing_open_round_id"));
}

#[test]
fn session_operator_is_server_derived_and_discovery_blocks_asset_close() {
    let request = PORT
        .split_once("pub struct AuthorizeInvestigationAssetVerificationSession")
        .expect("authorize DTO")
        .1
        .split_once("}")
        .expect("authorize DTO end")
        .0;
    assert!(!request.contains("authorized_by"));
    assert!(!request.contains("operator_channel"));
    assert!(REPO.contains("WHERE principal_kind='local_operator' AND active FOR SHARE"));
    assert!(MIGRATION.contains("NEW.operator_channel<>'local_cli'"));
    assert!(MIGRATION.contains("investigation_guard_asset_fixed_point_pending_discoveries"));
}

#[test]
fn dynamic_round_is_primary_continuous_and_has_no_fixed_roster_gate() {
    assert!(DYNAMIC_PRIMARY_MIGRATION.contains("primary_dynamic_v2"));
    assert!(DYNAMIC_VERIFICATION_MIGRATION
        .contains("investigation_dynamic_verification_primary_continuities"));
    assert!(
        DYNAMIC_VERIFICATION_MIGRATION.contains("investigation_dynamic_verification_primary_turns")
    );
    assert!(DYNAMIC_VERIFICATION_MIGRATION
        .contains("specialist_role IN('browser','researcher','pentester','adviser','coder'"));
    assert!(DYNAMIC_VERIFICATION_MIGRATION.contains("UNIQUE(session_id,actor_ordinal)"));
    assert!(DYNAMIC_VERIFICATION_MIGRATION.contains("decision_kind IN('delegate','resolve')"));
    assert!(DYNAMIC_VERIFICATION_MIGRATION.contains("actor_call_count BETWEEN 0 AND 8"));
    assert!(!PORT.contains("async fn open_session("));
    assert!(!PORT.contains("async fn claim_actor("));
    assert!(!PORT.contains("async fn resolve_hypothesis("));
    assert!(PORT.contains("async fn open_dynamic_round("));
    assert!(PORT.contains("async fn dispatch_dynamic_actor_batch("));
    assert!(PORT.contains("async fn resolve_dynamic_hypothesis("));
    assert!(PORT.contains("async fn load_pending_dynamic_actor_submission("));
    assert!(REPO.contains("created_by: \"accepted_worker_request\".into()"));
    assert!(!REPO.contains("server_primary_dispatch"));
    let open_round = REPO
        .split_once("pub async fn open_dynamic_round(")
        .expect("dynamic round opener")
        .1
        .split_once("pub async fn renew_dynamic_authorization(")
        .expect("dynamic round opener end")
        .0;
    assert!(open_round.contains("lane.evolution_epoch=current_primary.evolution_epoch"));
    assert!(open_round.contains("AND lane.state='verifying'"));
    assert!(CURRENT_PRIMARY_ROUND_MIGRATION
        .contains("FROM investigation_asset_primary_current_authorities current_primary"));
    assert!(CURRENT_PRIMARY_ROUND_MIGRATION
        .contains("current_primary.evolution_epoch=NEW.evolution_epoch"));
}

#[test]
fn invocation_authority_is_server_derived_from_frozen_inventory_and_budget() {
    let begin = PORT
        .split_once("pub struct BeginInvestigationAssetVerificationInvocation")
        .expect("begin DTO")
        .1
        .split_once('}')
        .expect("begin DTO end")
        .0;
    for forbidden in [
        "inventory_snapshot_id",
        "selected_tool_config_sha256",
        "effect_class",
        "risk_tier",
        "network_request_limit",
        "wall_time_limit_ms",
        "output_byte_limit",
        "invocation_authorization_expires_at",
        "request_manifest_sha256",
    ] {
        assert!(
            !begin.contains(forbidden),
            "caller still controls {forbidden}"
        );
    }
    assert!(REPO.contains("ORDER BY snapshot.sealed_at DESC"));
    assert!(REPO.contains("remaining_invocations"));
    assert_eq!(
        MIGRATION
            .matches("SET remaining_invocations=remaining_invocations-1")
            .count(),
        1,
        "the invocation trigger is the sole budget debit authority"
    );
    assert!(!REPO.contains("SET remaining_invocations=remaining_invocations-1"));
}

#[tokio::test]
#[serial]
async fn forward_migration_installs_dynamic_verification_authority_spine() {
    let root = tempfile::tempdir().expect("tempdir");
    let mut db = GolishDb::start(DbConfig {
        pg_data_dir: root.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("golish_dynamic_verification_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start migrated embedded postgres");
    for relation in [
        "investigation_asset_verification_authorizations",
        "investigation_asset_verification_budget_envelopes",
        "investigation_asset_verification_sessions",
        "investigation_asset_verification_chain_continuities",
        "investigation_dynamic_tool_inventory_snapshots",
        "investigation_dynamic_tool_inventory_members",
        "investigation_asset_verification_invocations",
        "investigation_hypothesis_resolution_authorities",
        "investigation_pending_hypothesis_discoveries",
        "investigation_pending_hypothesis_discovery_consumptions",
        "investigation_dynamic_verification_rounds",
        "investigation_dynamic_verification_primary_turns",
        "investigation_dynamic_verification_actor_calls",
        "investigation_dynamic_hypothesis_resolutions",
        "investigation_dynamic_hypothesis_terminal_transitions",
        "investigation_dynamic_verification_primary_completions",
    ] {
        let installed: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
            .bind(relation)
            .fetch_one(db.pool())
            .await
            .expect("relation check");
        assert!(installed, "{relation} missing");
    }
    let disposition_constraint: String = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(oid)
  FROM pg_constraint WHERE conrelid='investigation_hypothesis_resolution_authorities'::regclass
  AND pg_get_constraintdef(oid) LIKE '%disposition%'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("resolution check");
    assert!(
        disposition_constraint.contains("verified")
            && disposition_constraint.contains("refuted")
            && disposition_constraint.contains("invalid")
    );
    assert!(
        !disposition_constraint.contains("inconclusive")
            && !disposition_constraint.contains("blocked")
    );
    let snapshot_source_constraint: String = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(oid)
  FROM pg_constraint
 WHERE conrelid='candidate_analysis_snapshot_source_sets'::regclass
   AND conname='candidate_analysis_snapshot_source_sets_source_kind_check'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("candidate snapshot source-kind check");
    assert!(snapshot_source_constraint.contains("pending_hypothesis_discovery"));
    let lane_guard: String = sqlx::query_scalar(
        "SELECT pg_get_functiondef('investigation_guard_asset_hypothesis_lane()'::regprocedure)",
    )
    .fetch_one(db.pool())
    .await
    .expect("asset lane guard definition");
    let lane_guard = lane_guard.to_ascii_lowercase();
    assert!(lane_guard.contains("to_jsonb(new"));
    assert!(lane_guard.contains("'work_kind'"));
    assert!(!lane_guard.contains("new.work_kind"));
    db.stop().await;
}

#[derive(Clone, Copy)]
struct LifecycleScope {
    operation_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    asset_lane_id: Uuid,
    target_id: Uuid,
    first_revision_id: Uuid,
    first_task_id: Uuid,
    second_revision_id: Uuid,
    second_task_id: Uuid,
}

async fn seed_lifecycle_scope(db: &GolishDb) -> LifecycleScope {
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
    let stage_team_plan_id = Uuid::new_v5(&stage_run_unit_id, b"stage-team-plan:v1");
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin lifecycle authority fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("isolate upstream fixture triggers");
    sqlx::query("INSERT INTO sessions(id,title,status,project_path) VALUES($1,'verification lifecycle','running','/tmp/verification-lifecycle')")
        .bind(session_id).execute(&mut *tx).await.expect("insert session");
    sqlx::query("INSERT INTO tasks(id,session_id,input,status) VALUES($1,$2,'verification lifecycle','running')")
        .bind(operation_id).bind(session_id).execute(&mut *tx).await.expect("insert operation task");
    sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,'/tmp/verification-lifecycle','Lifecycle Org')")
        .bind(organization_id).execute(&mut *tx).await.expect("insert organization");
    sqlx::query("INSERT INTO targets(id,name,target_type,value,project_path,organization_id,scope,source) VALUES($1,'asset.example','domain','asset.example','/tmp/verification-lifecycle',$2,'in','manual')")
        .bind(target_id).bind(organization_id).execute(&mut *tx).await.expect("insert target");
    sqlx::query(
        r#"INSERT INTO operation_state(operation_id,profile,current_stage,runtime_memory_contract,
           project_scope_id,enumeration_analysis_contract,stage_topology_contract,
           stage_topology_canonical_json,stage_topology_sha256,stage_topology_freeze_source,
           investigation_contract_version,investigation_rollout_mode,tool_truth_contract)
           VALUES($1,'red_team','investigation','v2_only',$2,'legacy_v1',
             'unified_investigation_v1',stage_topology_canonical_json('unified_investigation_v1'),
             stage_topology_contract_sha256('unified_investigation_v1'),
             'deployment_pair_v1','hypothesis_registry_v1','new_only','receipt_v1')"#,
    )
    .bind(operation_id)
    .bind(project_scope_id)
    .execute(&mut *tx)
    .await
    .expect("insert operation state");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_snapshots(id,operation_id,project_scope_id,
           scope_decision_id,project_path_at_freeze,root_organization_id,mode,scope_hash,sealed_at)
           VALUES($1,$2,$3,$4,'/tmp/verification-lifecycle',$5,'cli_flags',$6,NOW())"#,
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
    sqlx::query("INSERT INTO operation_org_scope_units(snapshot_id,organization_id,organization_name_at_freeze,role,depth,ordinal,decision_row_id,approval_source) VALUES($1,$2,'Lifecycle Org','root',0,0,'root','{}')")
        .bind(scope_snapshot_id).bind(organization_id).execute(&mut *tx).await.expect("insert scope unit");
    sqlx::query("INSERT INTO stage_runs(id,operation_id,stage_kind,status,stage_topology_contract) VALUES($1,$2,'investigation','started','unified_investigation_v1')")
        .bind(stage_execution_id).bind(operation_id).execute(&mut *tx).await.expect("insert stage run");
    sqlx::query("INSERT INTO stage_run_units(id,operation_id,stage_execution_id,scope_snapshot_id,organization_id,stage_kind,generation,status,started_at) VALUES($1,$2,$3,$4,$5,'investigation',0,'running',NOW())")
        .bind(stage_run_unit_id).bind(operation_id).bind(stage_execution_id).bind(scope_snapshot_id)
        .bind(organization_id).execute(&mut *tx).await.expect("insert stage unit");
    sqlx::query("INSERT INTO investigation_run_heads(authority_id,stable_start_request_id,operation_id,stage_execution_id,owning_stage_run_request_id,scope_snapshot_id,run_state,admission_open,stop_epoch,change_seq,head_version,head_sha256) VALUES($1,$2,$3,$4,'verification-lifecycle',$5,'running',TRUE,0,0,0,unified_investigation_runtime_head_sha256($1,'running',TRUE,0,0,0))")
        .bind(authority_id).bind(Uuid::new_v4()).bind(operation_id).bind(stage_execution_id)
        .bind(scope_snapshot_id).execute(&mut *tx).await.expect("insert investigation head");
    sqlx::query("INSERT INTO investigation_company_queues(company_queue_id,stable_freeze_request_id,authority_id,operation_id,stage_execution_id,owning_stage_run_request_id,scope_snapshot_id,member_count,member_set_sha256,max_evolution_epochs) VALUES($1,$2,$3,$4,$5,'verification-lifecycle',$6,1,$7,2)")
        .bind(company_queue_id).bind(Uuid::new_v4()).bind(authority_id).bind(operation_id)
        .bind(stage_execution_id).bind(scope_snapshot_id).bind(digest('2'))
        .execute(&mut *tx).await.expect("insert company queue");
    sqlx::query("INSERT INTO investigation_company_queue_members(company_member_id,company_queue_id,authority_id,operation_id,stage_execution_id,scope_snapshot_id,organization_id,organization_name_at_freeze,depth,ordinal,state) VALUES($1,$2,$3,$4,$5,$6,$7,'Lifecycle Org',0,0,'active')")
        .bind(company_member_id).bind(company_queue_id).bind(authority_id).bind(operation_id)
        .bind(stage_execution_id).bind(scope_snapshot_id).bind(organization_id)
        .execute(&mut *tx).await.expect("insert company member");
    sqlx::query("INSERT INTO investigation_asset_queues(asset_queue_id,company_queue_id,company_member_id,authority_id,operation_id,stage_execution_id,scope_snapshot_id,organization_id,member_count,member_set_sha256) VALUES($1,$2,$3,$4,$5,$6,$7,$8,1,$9)")
        .bind(asset_queue_id).bind(company_queue_id).bind(company_member_id).bind(authority_id)
        .bind(operation_id).bind(stage_execution_id).bind(scope_snapshot_id).bind(organization_id)
        .bind(digest('3')).execute(&mut *tx).await.expect("insert asset queue");
    sqlx::query("INSERT INTO investigation_asset_lanes(asset_lane_id,asset_queue_id,company_queue_id,company_member_id,authority_id,operation_id,stage_execution_id,scope_snapshot_id,organization_id,target_id,target_type_at_freeze,target_value_at_freeze,target_source_at_freeze,target_created_at,target_identity_sha256,ordinal,state,evolution_epoch,max_evolution_epochs) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'domain','asset.example','manual',NOW(),$11,0,'analyzing',0,2)")
        .bind(asset_lane_id).bind(asset_queue_id).bind(company_queue_id).bind(company_member_id)
        .bind(authority_id).bind(operation_id).bind(stage_execution_id).bind(scope_snapshot_id)
        .bind(organization_id).bind(target_id).bind(digest('a'))
        .execute(&mut *tx).await.expect("insert asset lane");
    tx.commit()
        .await
        .expect("commit lifecycle authority fixture");

    let governed = seed_stage_team_runtime(db.pool(), &SeedStageTeamRuntimeRow {
        base: SeedStageRuntimeRow {
            operation_id, stage_execution_id, stage_kind: "investigation".into(),
            unit_generation: 0, specialist: "investigation".into(), worker_generation: 0,
            work_item_kind: "organization".into(), work_item_key: "investigation".into(),
            agent_path_prefix: "main>stage_run:investigation".into(), organization_ids: None,
        },
        plan: StageTeamPlanSeedRow {
            schema_version: 1, plan_version: 1, plan_hash: digest('4'),
            leader_role: "investigation".into(),
            allowed_roles: ["investigation","pentester","researcher","browser","coder","installer","enricher","memorist","adviser"].map(str::to_string).to_vec(),
            aggregator_kind: "worker".into(), aggregator_role: Some("investigation".into()),
            max_workers_total: 64, max_workers_active: 16, dynamic_requests_enabled: true,
            dynamic_request_policy: json!({"allowed_request_kinds":[],"coordination_mode":"investigation_task_orchestrator"}),
            final_submitter_kind: "worker".into(), created_from_stage_spec_hash: digest('5'),
        }, work_items: vec![],
    }).await.expect("seed governance plan");
    assert_eq!(governed[0].plan.id, stage_team_plan_id);
    let scheduled = ensure_investigation_asset_primary_schedule(
        db.pool(),
        &EnsureInvestigationAssetPrimaryScheduleRow {
            operation_id,
            stage_execution_id,
            stage_run_unit_id,
            stage_team_plan_id,
            asset_lane_id,
            target_id,
            asset_context_sha256: digest('a'),
        },
    )
    .await
    .expect("seed durable asset roster");

    let mut finish = db
        .pool()
        .begin()
        .await
        .expect("begin predecessor completion fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *finish)
        .await
        .expect("isolate predecessor terminal fixture");
    sqlx::query("UPDATE stage_work_items SET status='completed',terminal_at=NOW() WHERE id=$1")
        .bind(scheduled.primary_work_item.id)
        .execute(&mut *finish)
        .await
        .expect("complete predecessor primary item");
    sqlx::query("UPDATE stage_worker_runs SET status='passed',terminal_at=NOW() WHERE id=$1")
        .bind(scheduled.primary_worker.id)
        .execute(&mut *finish)
        .await
        .expect("complete predecessor primary worker");
    sqlx::query("UPDATE stage_team_plans SET requests_closed_at=NOW(),final_submitter_worker_run_id=NULL,row_version=row_version+1 WHERE id=$1")
        .bind(stage_team_plan_id).execute(&mut *finish).await.expect("close predecessor plan");
    sqlx::query("UPDATE investigation_asset_lanes SET state='verifying',row_version=row_version+1 WHERE asset_lane_id=$1")
        .bind(asset_lane_id).execute(&mut *finish).await.expect("advance asset to verification");

    let mut revisions = Vec::new();
    for (ordinal, nibble) in [(0_i32, '6'), (1_i32, '7')] {
        let root_id = Uuid::new_v4();
        let revision_id = Uuid::new_v4();
        let plan_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();
        let revision_hash = digest(nibble);
        let semantic_hash = digest(if ordinal == 0 { '8' } else { '9' });
        sqlx::query("INSERT INTO attack_hypotheses(root_id,operation_id,organization_id,root_kind,identity_ingredients,identity_ingredients_hash,asset_lane_id) VALUES($1,$2,$3,'initial','{}',$4,$5)")
            .bind(root_id).bind(operation_id).bind(organization_id).bind(digest(if ordinal == 0 {'b'} else {'c'})).bind(asset_lane_id)
            .execute(&mut *finish).await.expect("insert hypothesis root");
        sqlx::query("INSERT INTO attack_hypothesis_revisions(revision_id,root_id,operation_id,organization_id,revision_ordinal,semantic_key,semantic_key_hash,subject_kind,subject_identity_hash,target_live_id,target_type_at_time,target_value_at_time,predicate_schema,predicate_version,normalized_arguments,trust_boundary,polarity,epistemic_state,lifecycle_state,planning_readiness,structured_claim,assumptions,missing_facts,priority,risk_impact,origin_decision_hash,revision_ingredients_hash,revision_hash,asset_lane_id) VALUES($1,$2,$3,$4,$5,'{}',$6,'asset',$7,$8,'domain','asset.example','fixture.v1',1,'{}','external','positive','proposed','current','ready_for_strategy',$9,'[]','[]',$10,'{}',$11,$12,$13,$14)")
            .bind(revision_id).bind(root_id).bind(operation_id).bind(organization_id).bind(ordinal)
            .bind(&semantic_hash).bind(digest('d')).bind(target_id)
            .bind(json!({"claim":format!("hypothesis {ordinal}")})).bind(100-ordinal)
            .bind(digest('e')).bind(digest('f')).bind(&revision_hash).bind(asset_lane_id)
            .execute(&mut *finish).await.expect("insert hypothesis revision");
        sqlx::query("INSERT INTO attack_hypothesis_heads(root_id,operation_id,organization_id,head_revision_id,head_revision_hash,head_semantic_key_hash,head_epistemic_state,head_lifecycle_state) VALUES($1,$2,$3,$4,$5,$6,'proposed','current')")
            .bind(root_id).bind(operation_id).bind(organization_id).bind(revision_id).bind(&revision_hash).bind(&semantic_hash)
            .execute(&mut *finish).await.expect("insert hypothesis head");
        let component_id = Uuid::new_v4();
        let objective_id = Uuid::new_v4();
        let contract_id = Uuid::new_v4();
        let predicate_component_id = Uuid::new_v4();
        let plan_objective_id = Uuid::new_v4();
        let path_id = Uuid::new_v4();
        let component_member_hash = digest('1');
        let objective_hash = digest('2');
        let stopping_criteria_hash = digest('3');
        let predicate_member_hash = digest('4');
        let contract_hash = digest('5');
        let plan_objective_member_hash = digest('6');
        let path_hash = digest('7');
        let path_member_hash = digest('8');
        let (predicate_set_hash, control_set_hash, pair_set_hash, ordered_set_hash): (
            String,
            String,
            String,
            String,
        ) = sqlx::query_as(
            r#"SELECT
               verification_contract_exact_member_set_hash(
                   'verification_predicate_set.v1',ARRAY[$1]::TEXT[]),
               verification_contract_exact_member_set_hash(
                   'verification_control_set.v1',ARRAY[]::TEXT[]),
               verification_contract_exact_member_set_hash(
                   'verification_paired_differential_set.v1',ARRAY[]::TEXT[]),
               verification_contract_exact_member_set_hash(
                   'verification_ordered_step_set.v1',ARRAY[]::TEXT[])"#,
        )
        .bind(&predicate_member_hash)
        .fetch_one(&mut *finish)
        .await
        .expect("derive exact VerificationContract sets");
        let (
            required_component_set_hash,
            objective_component_set_hash,
            objective_set_hash,
            falsifier_set_hash,
            path_member_set_hash,
            proof_path_set_hash,
        ): (String, String, String, String, String, String) = sqlx::query_as(
            r#"SELECT
               investigation_exact_member_set_hash(
                   'hypothesis_verification_plan_required_components.v1',ARRAY[$1]::TEXT[]),
               investigation_exact_member_set_hash(
                   'hypothesis_verification_plan_objective_components.v1',ARRAY[$1]::TEXT[]),
               investigation_exact_member_set_hash(
                   'hypothesis_verification_plan_objectives.v1',ARRAY[$2]::TEXT[]),
               investigation_exact_member_set_hash(
                   'hypothesis_verification_plan_path_falsifiers.v1',ARRAY[$1]::TEXT[]),
               investigation_exact_member_set_hash(
                   'hypothesis_verification_plan_path_members.v1',ARRAY[$3]::TEXT[]),
               investigation_exact_member_set_hash(
                   'hypothesis_verification_plan_paths.v1',ARRAY[$4]::TEXT[])"#,
        )
        .bind(&component_member_hash)
        .bind(&plan_objective_member_hash)
        .bind(&path_member_hash)
        .bind(&path_hash)
        .fetch_one(&mut *finish)
        .await
        .expect("derive exact HypothesisPlan sets");
        sqlx::query(
            r#"INSERT INTO attack_hypothesis_claim_components(
               component_id,revision_id,revision_hash,component_ordinal,component_key,kind,
               canonical_fragment_hash,canonical_condition_hash,required,
               derivation_contract_version,derivation_contract_digest,member_hash)
               VALUES($1,$2,$3,0,'claim','claim_clause',$4,$5,TRUE,1,$6,$7)"#,
        )
        .bind(component_id)
        .bind(revision_id)
        .bind(&revision_hash)
        .bind(digest('9'))
        .bind(digest('a'))
        .bind(digest('b'))
        .bind(&component_member_hash)
        .execute(&mut *finish)
        .await
        .expect("insert required claim component");
        sqlx::query("INSERT INTO attack_hypothesis_verification_objectives(objective_id,revision_id,objective_ordinal,objective_intent,stopping_criteria,stopping_criteria_hash,objective_hash) VALUES($1,$2,0,'{}','{}',$3,$4)")
            .bind(objective_id).bind(revision_id).bind(&stopping_criteria_hash).bind(&objective_hash)
            .execute(&mut *finish).await.expect("insert objective");
        sqlx::query(
            r#"INSERT INTO attack_hypothesis_verification_contracts(
               contract_id,revision_id,revision_hash,objective_id,combinator,
               predicate_count,predicate_set_hash,required_control_count,
               required_control_set_hash,explicit_no_required_control,
               paired_differential_count,paired_differential_set_hash,
               ordered_step_count,ordered_step_set_hash,stopping_criteria_hash,
               compiler_digest,rule_digest,policy_snapshot_hash,contract_hash)
               VALUES($1,$2,$3,$4,'all_of',1,$5,0,$6,TRUE,0,$7,0,$8,$9,$10,$11,$12,$13)"#,
        )
        .bind(contract_id)
        .bind(revision_id)
        .bind(&revision_hash)
        .bind(objective_id)
        .bind(&predicate_set_hash)
        .bind(&control_set_hash)
        .bind(&pair_set_hash)
        .bind(&ordered_set_hash)
        .bind(&stopping_criteria_hash)
        .bind(digest('c'))
        .bind(digest('d'))
        .bind(digest('e'))
        .bind(&contract_hash)
        .execute(&mut *finish)
        .await
        .expect("insert verification contract");
        sqlx::query(
            r#"INSERT INTO attack_hypothesis_verification_predicate_components(
               predicate_component_id,contract_id,ordinal,semantic_key,predicate_schema,
               predicate_version,normalized_arguments,normalized_arguments_hash,
               expected_polarity,prerequisite_hash,member_hash)
               VALUES($1,$2,0,'claim','fixture.v1',1,'{}',$3,'positive',$4,$5)"#,
        )
        .bind(predicate_component_id)
        .bind(contract_id)
        .bind(digest('f'))
        .bind(digest('0'))
        .bind(&predicate_member_hash)
        .execute(&mut *finish)
        .await
        .expect("insert verification predicate");
        sqlx::query(
            r#"INSERT INTO attack_hypothesis_verification_objective_claim_components(
               binding_id,contract_id,revision_id,objective_id,claim_component_id,
               ordinal,component_member_hash,binding_member_hash)
               VALUES($1,$2,$3,$4,$5,0,$6,$7)"#,
        )
        .bind(Uuid::new_v4())
        .bind(contract_id)
        .bind(revision_id)
        .bind(objective_id)
        .bind(component_id)
        .bind(&component_member_hash)
        .bind(digest('1'))
        .execute(&mut *finish)
        .await
        .expect("bind objective claim component");
        sqlx::query("INSERT INTO attack_hypothesis_verification_plans(plan_id,revision_id,revision_hash,revision_ingredients_hash,required_claim_component_count,required_claim_component_set_hash,objective_count,objective_set_hash,proof_path_count,proof_path_set_hash,outer_aggregation_policy_version,outer_aggregation_policy_digest,plan_hash,sealed_at) VALUES($1,$2,$3,$4,1,$5,1,$6,1,$7,1,$8,$9,NOW())")
            .bind(plan_id).bind(revision_id).bind(&revision_hash).bind(digest('f')).bind(&required_component_set_hash).bind(&objective_set_hash).bind(&proof_path_set_hash).bind(digest('5')).bind(digest('0'))
            .execute(&mut *finish).await.expect("insert verification plan");
        sqlx::query(
            r#"INSERT INTO attack_hypothesis_verification_plan_objectives(
               plan_objective_id,plan_id,revision_id,objective_id,verification_contract_id,
               ordinal,objective_hash,verification_contract_version,
               verification_contract_hash,claim_component_count,claim_component_set_hash,
               stopping_criteria_hash,outcome_requirement,member_hash)
               VALUES($1,$2,$3,$4,$5,0,$6,1,$7,1,$8,$9,
                      'satisfy_or_falsify_bound_required_components',$10)"#,
        )
        .bind(plan_objective_id)
        .bind(plan_id)
        .bind(revision_id)
        .bind(objective_id)
        .bind(contract_id)
        .bind(&objective_hash)
        .bind(&contract_hash)
        .bind(&objective_component_set_hash)
        .bind(&stopping_criteria_hash)
        .bind(&plan_objective_member_hash)
        .execute(&mut *finish)
        .await
        .expect("insert plan objective");
        sqlx::query(
            "INSERT INTO attack_hypothesis_verification_plan_paths(path_id,plan_id,path_ordinal,path_key,member_count,member_set_hash,path_hash) VALUES($1,$2,0,'primary',1,$3,$4)",
        )
        .bind(path_id)
        .bind(plan_id)
        .bind(&path_member_set_hash)
        .bind(&path_hash)
        .execute(&mut *finish)
        .await
        .expect("insert proof path");
        sqlx::query(
            r#"INSERT INTO attack_hypothesis_verification_plan_path_members(
               path_member_id,path_id,plan_id,plan_objective_id,plan_objective_member_hash,
               revision_id,member_ordinal,verification_contract_hash,claim_component_set_hash,
               role,falsifier_claim_component_member_hashes,falsifier_claim_component_count,
               falsifier_claim_component_set_hash,member_hash)
               VALUES($1,$2,$3,$4,$5,$6,0,$7,$8,
                      'required_proof_and_path_falsifier',ARRAY[$9]::TEXT[],1,$10,$11)"#,
        )
        .bind(Uuid::new_v4())
        .bind(path_id)
        .bind(plan_id)
        .bind(plan_objective_id)
        .bind(&plan_objective_member_hash)
        .bind(revision_id)
        .bind(&contract_hash)
        .bind(&objective_component_set_hash)
        .bind(&component_member_hash)
        .bind(&falsifier_set_hash)
        .bind(&path_member_hash)
        .execute(&mut *finish)
        .await
        .expect("insert proof path member");
        sqlx::query("INSERT INTO hypothesis_verification_tasks(task_id,stable_task_key_sha256,operation_id,project_scope_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id,hypothesis_revision_id,hypothesis_revision_sha256,verification_plan_id,verification_plan_sha256,relevant_evidence_snapshot_id,semantic_evidence_set_sha256,open_obligation_set_sha256,semantic_attempt_fingerprint,task_contract_version,first_admission_generation_id,asset_lane_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,'hypothesis_verification_task.v1',$17,$18)")
            .bind(task_id).bind(digest(if ordinal == 0 {'a'} else {'b'})).bind(operation_id).bind(project_scope_id)
            .bind(stage_execution_id).bind(stage_run_unit_id).bind(scope_snapshot_id).bind(organization_id)
            .bind(revision_id).bind(&revision_hash).bind(plan_id).bind(digest('0')).bind(Uuid::new_v4())
            .bind(digest('1')).bind(digest('2')).bind(digest('3')).bind(Uuid::new_v4()).bind(asset_lane_id)
            .execute(&mut *finish).await.expect("insert verification task");
        revisions.push((revision_id, task_id));
    }
    finish
        .commit()
        .await
        .expect("commit verification predecessor/hypothesis fixture");
    LifecycleScope {
        operation_id,
        stage_execution_id,
        stage_run_unit_id,
        scope_snapshot_id,
        organization_id,
        asset_lane_id,
        target_id,
        first_revision_id: revisions[0].0,
        first_task_id: revisions[0].1,
        second_revision_id: revisions[1].0,
        second_task_id: revisions[1].1,
    }
}

async fn authorize_and_open(
    db: &GolishDb,
    scope: LifecycleScope,
    revision_id: Uuid,
    task_id: Uuid,
) -> verification::DynamicVerificationRoundRow {
    let credential_binding_set_sha256: String =
        sqlx::query_scalar("SELECT tool_truth_sha256('[]')")
            .fetch_one(db.pool())
            .await
            .expect("hash empty credential set");
    let authorization_id = Uuid::new_v4();
    let budget_id = Uuid::new_v4();
    let authorization = verification::authorize_session(
        db.pool(),
        &verification::AuthorizeAssetVerificationSessionInput {
            stable_request_id: Uuid::new_v4(),
            session_authorization_id: authorization_id,
            session_budget_envelope_id: budget_id,
            operation_id: scope.operation_id,
            stage_execution_id: scope.stage_execution_id,
            stage_run_unit_id: scope.stage_run_unit_id,
            scope_snapshot_id: scope.scope_snapshot_id,
            organization_id: scope.organization_id,
            asset_lane_id: scope.asset_lane_id,
            target_live_id: scope.target_id,
            hypothesis_revision_id: revision_id,
            verification_task_id: task_id,
            allowed_effect_classes: vec!["read_only".into(), "passive_network".into()],
            maximum_risk_tier: "T1".into(),
            allowed_credential_binding_sha256s: vec![],
            credential_binding_set_sha256,
            maximum_invocations: 32,
            maximum_network_requests: 128,
            maximum_wall_time_ms: 600_000,
            maximum_output_bytes: 16_000_000,
            maximum_parallel_invocations: 2,
        },
    )
    .await
    .expect("authorize exact asset verification round");
    assert_eq!(authorization.remaining_invocations, 32);

    let open = verification::OpenDynamicVerificationRoundInput {
        stable_request_id: Uuid::new_v4(),
        operation_id: scope.operation_id,
        stage_execution_id: scope.stage_execution_id,
        stage_run_unit_id: scope.stage_run_unit_id,
        scope_snapshot_id: scope.scope_snapshot_id,
        organization_id: scope.organization_id,
        asset_lane_id: scope.asset_lane_id,
        target_live_id: scope.target_id,
        hypothesis_revision_id: revision_id,
        verification_task_id: task_id,
        session_authorization_id: authorization_id,
        session_budget_envelope_id: budget_id,
    };
    let round = verification::open_dynamic_round(db.pool(), &open)
        .await
        .expect("server-open dynamic verification round");
    assert!(!round.replayed);
    let replay = verification::open_dynamic_round(db.pool(), &open)
        .await
        .expect("exact dynamic round open replay");
    assert!(replay.replayed);
    assert_eq!(replay.session_id, round.session_id);
    round
}

fn fence(
    claimed: &golish_db::repo::runtime_memory_tx::ClaimedStageWorkItemRow,
) -> verification::VerificationWorkerFenceInput {
    verification::VerificationWorkerFenceInput {
        worker_run_id: claimed.worker.id,
        lease_token: claimed.worker.lease_token.expect("claimed worker lease"),
        attempt_epoch: claimed.worker.attempt_epoch,
        checkpoint_version: claimed.worker.checkpoint_version,
    }
}

async fn claim_primary(
    db: &GolishDb,
    session_id: Uuid,
) -> golish_db::repo::runtime_memory_tx::ClaimedStageWorkItemRow {
    verification::claim_dynamic_primary(
        db.pool(),
        &verification::ClaimDynamicVerificationPrimaryInput {
            session_id,
            lease_owner: "dynamic-verification-primary-test".into(),
            lease_seconds: 120,
        },
    )
    .await
    .expect("claim dynamic verification Primary")
}

async fn record_submit_result(
    db: &GolishDb,
    scope: LifecycleScope,
    claimed: &golish_db::repo::runtime_memory_tx::ClaimedStageWorkItemRow,
    canonical_result: serde_json::Value,
) -> (Uuid, String) {
    let task_session_id: Uuid = sqlx::query_scalar("SELECT session_id FROM tasks WHERE id=$1")
        .bind(scope.operation_id)
        .fetch_one(db.pool())
        .await
        .expect("load task session");
    let record_id = Uuid::new_v4();
    let provider_call_id = format!("submit-result-{record_id}");
    sqlx::query(
        r#"INSERT INTO tool_calls(
             id,call_id,session_id,task_id,name,args,result,status,source,
             operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
             organization_id,attempt_epoch,lease_token)
           VALUES($1,$2,$3,$4,'submit_result',$5,$6,'finished','ai',
             $4,$7,$8,$9,$10,$11,$12)"#,
    )
    .bind(record_id)
    .bind(&provider_call_id)
    .bind(task_session_id)
    .bind(scope.operation_id)
    .bind(json!({"result":canonical_result}))
    .bind(r#"{"status":"result submitted"}"#)
    .bind(scope.stage_execution_id)
    .bind(scope.stage_run_unit_id)
    .bind(claimed.worker.id)
    .bind(scope.organization_id)
    .bind(claimed.worker.attempt_epoch)
    .bind(claimed.worker.lease_token.expect("claimed worker lease"))
    .execute(db.pool())
    .await
    .expect("record durable submit_result authority");
    (record_id, provider_call_id)
}

async fn record_started_submit_result(
    db: &GolishDb,
    scope: LifecycleScope,
    claimed: &golish_db::repo::runtime_memory_tx::ClaimedStageWorkItemRow,
    canonical_result: serde_json::Value,
) -> (Uuid, String) {
    let task_session_id: Uuid = sqlx::query_scalar("SELECT session_id FROM tasks WHERE id=$1")
        .bind(scope.operation_id)
        .fetch_one(db.pool())
        .await
        .expect("load task session");
    let record_id = Uuid::new_v4();
    let provider_call_id = format!("started-submit-result-{record_id}");
    sqlx::query(
        r#"INSERT INTO tool_calls(
             id,call_id,session_id,task_id,name,args,status,source,
             operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
             organization_id,attempt_epoch,lease_token)
           VALUES($1,$2,$3,$4,'submit_result',$5,'running','ai',
             $4,$6,$7,$8,$9,$10,$11)"#,
    )
    .bind(record_id)
    .bind(&provider_call_id)
    .bind(task_session_id)
    .bind(scope.operation_id)
    .bind(json!({"result":canonical_result}))
    .bind(scope.stage_execution_id)
    .bind(scope.stage_run_unit_id)
    .bind(claimed.worker.id)
    .bind(scope.organization_id)
    .bind(claimed.worker.attempt_epoch)
    .bind(claimed.worker.lease_token.expect("claimed worker lease"))
    .execute(db.pool())
    .await
    .expect("record started internal submit_result");
    (record_id, provider_call_id)
}

#[tokio::test]
#[serial]
async fn migrated_dynamic_round_repeats_roles_archives_unused_calls_and_reuses_primary_chain() {
    let root = tempfile::tempdir().expect("tempdir");
    let mut db = GolishDb::start(DbConfig {
        pg_data_dir: root.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("golish_dynamic_lifecycle_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start migrated embedded postgres");
    let scope = seed_lifecycle_scope(&db).await;

    let first = authorize_and_open(&db, scope, scope.first_revision_id, scope.first_task_id).await;
    assert!(first.actor_calls.is_empty());
    let mut primary = claim_primary(&db, first.session_id).await;
    let actor_ids = [Uuid::new_v4(), Uuid::new_v4()];
    let delegate_result = json!({
        "decision":"delegate",
        "schema_version":1,
        "session_id":first.session_id,
        "hypothesis_revision_id":first.hypothesis_revision_id,
        "subtasks":[
            {"stable_key":"research-first","role":"researcher",
             "objective":"Check the first bounded observation",
             "rationale":"Independent corroboration",
             "subject_refs":[{"kind":"target","id":first.target_live_id},
                             {"kind":"hypothesis_revision","id":first.hypothesis_revision_id}]},
            {"stable_key":"research-second","role":"researcher",
             "objective":"Check a second bounded observation",
             "rationale":"Repeated roles are legal",
             "subject_refs":[{"kind":"target","id":first.target_live_id},
                             {"kind":"hypothesis_revision","id":first.hypothesis_revision_id}]}
        ]
    });
    let (delegate_source_id, delegate_provider_id) =
        record_submit_result(&db, scope, &primary, delegate_result).await;
    let dispatch = verification::DispatchDynamicVerificationActorBatchInput {
        stable_request_id: Uuid::new_v4(),
        primary_turn_id: Uuid::new_v4(),
        session_id: first.session_id,
        expected_session_head_version: first.head_version,
        primary_worker_fence: fence(&primary),
        source_tool_call_record_id: delegate_source_id,
        source_provider_call_id: delegate_provider_id,
        actors: actor_ids
            .into_iter()
            .map(
                |actor_call_id| verification::DynamicVerificationActorRequestInput {
                    actor_call_id,
                },
            )
            .collect(),
    };
    let delegated = verification::dispatch_dynamic_actor_batch(db.pool(), &dispatch)
        .await
        .expect("atomically persist repeated-role Primary turn");
    assert_eq!(delegated.actors.len(), 2);
    assert!(delegated
        .actors
        .iter()
        .all(|actor| actor.specialist_role == "researcher"));
    for actor in &delegated.actors {
        let persisted_agent: String =
            sqlx::query_scalar("SELECT agent::TEXT FROM message_chains WHERE id=$1")
                .bind(actor.message_chain_id)
                .fetch_one(db.pool())
                .await
                .expect("load dynamic actor message-chain agent");
        assert_eq!(persisted_agent, "searcher");
    }
    let replay = verification::dispatch_dynamic_actor_batch(db.pool(), &dispatch)
        .await
        .expect("exact Primary turn replay");
    assert!(replay.replayed);
    assert_eq!(replay.actors.len(), 2);

    let mut expiry_tx = db.pool().begin().await.expect("begin expiry fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *expiry_tx)
        .await
        .expect("isolate append-only expiry fixture");
    let expired_at: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "UPDATE investigation_asset_verification_authorizations \
         SET expires_at=statement_timestamp()-INTERVAL '1 minute' \
         WHERE session_authorization_id=$1 RETURNING expires_at",
    )
    .bind(first.session_authorization_id)
    .fetch_one(&mut *expiry_tx)
    .await
    .expect("expire immutable base authorization fixture");
    sqlx::query(
        "UPDATE investigation_dynamic_verification_rounds SET authorization_expires_at=$2 \
         WHERE session_id=$1",
    )
    .bind(first.session_id)
    .bind(expired_at)
    .execute(&mut *expiry_tx)
    .await
    .expect("expire effective round authorization fixture");
    expiry_tx.commit().await.expect("commit expiry fixture");
    let renewal_request_id = Uuid::new_v4();
    let renewal_id = Uuid::new_v4();
    let renewed = verification::renew_dynamic_authorization(
        db.pool(),
        renewal_request_id,
        renewal_id,
        first.session_id,
    )
    .await
    .expect("renew exact dynamic round authorization");
    assert_eq!(renewed.previous_expires_at, expired_at);
    assert!(renewed.renewed_expires_at > chrono::Utc::now());
    let renewal_replay = verification::renew_dynamic_authorization(
        db.pool(),
        renewal_request_id,
        renewal_id,
        first.session_id,
    )
    .await
    .expect("exact authorization renewal replay");
    assert!(renewal_replay.replayed);
    assert_eq!(
        renewal_replay.renewed_expires_at,
        renewed.renewed_expires_at
    );
    let effective_expiry: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "SELECT authorization_expires_at FROM investigation_dynamic_verification_rounds \
         WHERE session_id=$1",
    )
    .bind(first.session_id)
    .fetch_one(db.pool())
    .await
    .expect("load renewed effective expiry");
    assert_eq!(effective_expiry, renewed.renewed_expires_at);

    let first_actor = &delegated.actors[0];
    let crashed_actor = verification::claim_dynamic_actor(
        db.pool(),
        &verification::ClaimDynamicVerificationActorInput {
            session_id: first.session_id,
            actor_call_id: first_actor.actor_call_id,
            lease_owner: "dynamic-recovery-actor-test".into(),
            lease_seconds: 120,
        },
    )
    .await
    .expect("claim dynamic actor before simulated crash");
    let actor_observation = json!({
        "schema_version":1,
        "session_id":first.session_id,
        "hypothesis_revision_id":first.hypothesis_revision_id,
        "actor_call_id":first_actor.actor_call_id,
        "actor_ordinal":first_actor.actor_ordinal,
        "subtask_id":first_actor.subtask_id,
        "specialist_role":first_actor.specialist_role,
        "summary":"The bounded observation completed before response loss.",
        "cited_evidence_ids":[],
        "new_hypothesis_proposals":[]
    });
    let (actor_source_id, actor_provider_id) =
        record_started_submit_result(&db, scope, &crashed_actor, actor_observation).await;
    sqlx::query(
        "UPDATE stage_worker_runs SET status='recovery_required',active_tool_call_id=$2, \
         active_tool_started_at=statement_timestamp()-INTERVAL '2 minutes', \
         lease_acquired_at=statement_timestamp()-INTERVAL '3 minutes', \
         lease_expires_at=statement_timestamp()-INTERVAL '1 minute',updated_at=NOW() \
         WHERE id=$1 AND status='running'",
    )
    .bind(crashed_actor.worker.id)
    .bind(actor_source_id)
    .execute(db.pool())
    .await
    .expect("simulate stage-team reaper worker state");
    sqlx::query(
        "UPDATE stage_work_items SET status='recovery_required',row_version=row_version+1, \
         updated_at=NOW() WHERE id=$1 AND status='running'",
    )
    .bind(crashed_actor.work_item.id)
    .execute(db.pool())
    .await
    .expect("simulate stage-team reaper work-item state");
    let pending_actor = verification::load_pending_dynamic_actor_submission(
        db.pool(),
        first.session_id,
        first_actor.actor_call_id,
    )
    .await
    .expect("reconcile started internal submit_result")
    .expect("load reconciled actor submission");
    assert_eq!(pending_actor.source_tool_call_record_id, actor_source_id);
    let (worker_status, item_status, actor_state, active_tool_call_id): (
        String,
        String,
        String,
        Option<Uuid>,
    ) = sqlx::query_as(
        "SELECT worker.status,item.status,actor.state,worker.active_tool_call_id \
         FROM investigation_dynamic_verification_actor_calls actor \
         JOIN stage_worker_runs worker ON worker.id=actor.worker_run_id \
         JOIN stage_work_items item ON item.id=actor.work_item_id \
         WHERE actor.actor_call_id=$1",
    )
    .bind(first_actor.actor_call_id)
    .fetch_one(db.pool())
    .await
    .expect("load reconciled actor state");
    assert_eq!(worker_status, "waiting_background");
    assert_eq!(item_status, "waiting_dependency");
    assert_eq!(actor_state, "parked");
    assert_eq!(active_tool_call_id, None);
    let reclaimed_actor = verification::claim_dynamic_actor(
        db.pool(),
        &verification::ClaimDynamicVerificationActorInput {
            session_id: first.session_id,
            actor_call_id: first_actor.actor_call_id,
            lease_owner: "dynamic-recovered-actor-test".into(),
            lease_seconds: 120,
        },
    )
    .await
    .expect("reclaim reconciled actor with a fresh fence");

    let inventory = verification::freeze_dynamic_inventory(
        db.pool(),
        &verification::FreezeDynamicToolInventoryInput {
            stable_request_id: Uuid::new_v4(),
            session_id: first.session_id,
            inventory_source_sha256: digest('c'),
            members: vec![verification::DynamicToolInventoryMemberInput {
                tool_id: "native-tool".into(),
                tool_name: "native-tool".into(),
                config_sha256: digest('1'),
                executable_identity_sha256: digest('2'),
                runtime: "native".into(),
                runtime_version: String::new(),
                launch_mode: "cli".into(),
                parameter_schema: json!({}),
                output_schema: json!({}),
                tags: vec!["verification".into()],
            }],
        },
    )
    .await
    .expect("freeze a ready native inventory without inventing a runtime version");
    assert_eq!(inventory.members.len(), 1);
    assert_eq!(inventory.members[0].runtime, "native");
    assert_eq!(inventory.members[0].runtime_version, "");
    assert_eq!(inventory.member_count, 1);
    let list_args = json!({"operation":"list_ready_tools"});
    let list_args_sha256: String = sqlx::query_scalar("SELECT tool_truth_sha256($1::JSONB::TEXT)")
        .bind(&list_args)
        .fetch_one(db.pool())
        .await
        .expect("hash list-tools args");
    let invocation_id = Uuid::new_v4();
    let invocation = verification::begin_invocation(
        db.pool(),
        &verification::BeginAssetVerificationInvocationInput {
            stable_request_id: Uuid::new_v4(),
            invocation_id,
            session_id: first.session_id,
            actor_call_id: first_actor.actor_call_id,
            worker_fence: fence(&reclaimed_actor),
            wrapper_name: "pentest_list_tools".into(),
            selected_tool_name: None,
            credential_binding_sha256: None,
            model_args_redacted: list_args,
            model_args_sha256: list_args_sha256,
        },
    )
    .await
    .expect("begin read-only invocation against renewed effective authorization");
    assert_eq!(invocation.effect_class, "read_only");
    assert!(invocation.invocation_authorization_expires_at > chrono::Utc::now());
    assert!(invocation.invocation_authorization_expires_at <= renewed.renewed_expires_at);
    let list_result = json!({"tools":[]});
    let list_result_sha256: String =
        sqlx::query_scalar("SELECT tool_truth_sha256($1::JSONB::TEXT)")
            .bind(&list_result)
            .fetch_one(db.pool())
            .await
            .expect("hash list-tools result");
    let empty_evidence_set_sha256: String = sqlx::query_scalar("SELECT tool_truth_sha256('[]')")
        .fetch_one(db.pool())
        .await
        .expect("hash empty invocation evidence set");
    let completed_invocation = verification::complete_invocation(
        db.pool(),
        &verification::CompleteAssetVerificationInvocationInput {
            stable_request_id: invocation.stable_request_id,
            invocation_id,
            expected_row_version: invocation.row_version,
            worker_fence: fence(&reclaimed_actor),
            disposition: "succeeded".into(),
            capability_execution_receipt_id: None,
            oracle_receipt_id: None,
            audit_evidence_ids: vec![],
            evidence_set_sha256: empty_evidence_set_sha256,
            redacted_result: list_result,
            result_sha256: list_result_sha256,
        },
    )
    .await
    .expect("complete renewed read-only invocation audit");
    assert_eq!(completed_invocation.state, "succeeded");

    let completed_actor = verification::complete_dynamic_actor(
        db.pool(),
        &verification::CompleteDynamicVerificationActorInput {
            session_id: first.session_id,
            actor_call_id: first_actor.actor_call_id,
            worker_fence: fence(&reclaimed_actor),
            expected_work_item_row_version: reclaimed_actor.work_item.row_version,
            source_tool_call_record_id: actor_source_id,
            source_provider_call_id: actor_provider_id,
            terminal_checkpoint: json!({"actor_call_id":first_actor.actor_call_id}),
            evidence_watermark: None,
        },
    )
    .await
    .expect("complete actor from reconciled immutable source");
    let completion_replay = verification::load_dynamic_actor_completion(
        db.pool(),
        first.session_id,
        first_actor.actor_call_id,
    )
    .await
    .expect("load actor completion after response loss")
    .expect("completed actor authority");
    assert!(completion_replay.replayed);
    assert_eq!(completion_replay.output.id, completed_actor.output.id);

    verification::park_dynamic_primary(
        db.pool(),
        &verification::ParkDynamicVerificationPrimaryInput {
            session_id: first.session_id,
            worker_fence: fence(&primary),
            checkpoint: json!({"completed_primary_turns":1}),
            evidence_watermark: None,
        },
    )
    .await
    .expect("park Primary after accepted delegate turn");
    primary = claim_primary(&db, first.session_id).await;

    let duplicate_proposal = json!({
        "predicate_schema":"dynamic_verification_follow_up.v1",
        "predicate_version":1,
        "predicate_arguments":[["surface","http"]],
        "trust_boundary":"external",
        "polarity":"positive",
        "structured_claim":"A bounded follow-up hypothesis remains to be verified.",
        "preconditions":["The asset remains in the frozen scope."],
        "impact":"The follow-up may change the asset conclusion.",
        "rationale":"Exercise the dynamic discovery backlog projection."
    });
    let resolution_id = Uuid::new_v4();
    let resolve_result = json!({
        "decision":"resolve",
        "schema_version":1,
        "session_id":first.session_id,
        "hypothesis_revision_id":first.hypothesis_revision_id,
        "subtasks":[],
        "disposition":"verified",
        "conclusion":"Primary reached an explicit verified conclusion.",
        "cited_evidence_ids":[],
        "new_hypothesis_proposals":[duplicate_proposal.clone()]
    });
    let (resolve_source_id, resolve_provider_id) =
        record_submit_result(&db, scope, &primary, resolve_result).await;
    let resolve = verification::ResolveDynamicHypothesisInput {
        stable_request_id: Uuid::new_v4(),
        resolution_authority_id: resolution_id,
        session_id: first.session_id,
        expected_session_head_version: first.head_version,
        primary_worker_fence: fence(&primary),
        primary_turn_id: Uuid::new_v4(),
        source_tool_call_record_id: resolve_source_id,
        source_provider_call_id: resolve_provider_id,
    };
    let resolution = verification::resolve_dynamic_hypothesis(db.pool(), &resolve)
        .await
        .expect("commit Primary-owned dynamic resolution");
    assert_eq!(resolution.disposition, "verified");
    assert!(
        verification::resolve_dynamic_hypothesis(db.pool(), &resolve)
            .await
            .expect("exact dynamic resolution replay")
            .replayed
    );
    let pending_discoveries = verification::list_pending_hypothesis_discoveries(
        db.pool(),
        scope.operation_id,
        scope.asset_lane_id,
    )
    .await
    .expect("list the dynamic-resolution discovery backlog");
    assert_eq!(pending_discoveries.len(), 1);
    assert_eq!(
        pending_discoveries[0].resolution_authority_id,
        resolution_id
    );
    assert_eq!(pending_discoveries[0].session_id, first.session_id);

    let archived_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM investigation_dynamic_verification_actor_calls \
         WHERE session_id=$1 AND state='archived'",
    )
    .bind(first.session_id)
    .fetch_one(db.pool())
    .await
    .expect("count resolution-authority archives");
    assert_eq!(archived_count, 1);
    let completed_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM investigation_dynamic_verification_actor_calls \
         WHERE session_id=$1 AND state='completed'",
    )
    .bind(first.session_id)
    .fetch_one(db.pool())
    .await
    .expect("count completed dynamic actors");
    assert_eq!(completed_count, 1);
    let transition_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM investigation_dynamic_hypothesis_terminal_transitions \
         WHERE resolution_authority_id=$1",
    )
    .bind(resolution_id)
    .fetch_one(db.pool())
    .await
    .expect("load immutable terminal transition");
    assert_eq!(transition_count, 1);

    let pending = verification::load_pending_dynamic_primary_terminalization(
        db.pool(),
        scope.operation_id,
        scope.asset_lane_id,
    )
    .await
    .expect("load response-loss terminalization")
    .expect("resolved Primary still pending");
    assert_eq!(pending.round.session_id, first.session_id);
    let completion_input = verification::CompleteDynamicVerificationPrimaryInput {
        session_id: first.session_id,
        resolution_authority_id: resolution_id,
        primary_worker_fence: fence(&primary),
        expected_work_item_row_version: pending.expected_work_item_row_version,
        expected_plan_row_version: pending.expected_plan_row_version,
        terminal_checkpoint: json!({"resolution_authority_id":resolution_id}),
    };
    let terminalized = verification::complete_dynamic_primary(db.pool(), &completion_input)
        .await
        .expect("terminalize Primary from immutable resolution");
    assert_eq!(terminalized.1.plan.final_submitter_worker_run_id, None);
    assert!(
        verification::complete_dynamic_primary(db.pool(), &completion_input)
            .await
            .expect("exact Primary terminalization replay")
            .1
            .replayed
    );
    assert!(verification::load_pending_dynamic_primary_terminalization(
        db.pool(),
        scope.operation_id,
        scope.asset_lane_id,
    )
    .await
    .expect("reload terminalization head")
    .is_none());

    let second =
        authorize_and_open(&db, scope, scope.second_revision_id, scope.second_task_id).await;
    assert_ne!(first.primary.worker_run_id, second.primary.worker_run_id);
    assert_eq!(
        first.primary.message_chain_id,
        second.primary.message_chain_id
    );
    assert!(
        second.actor_calls.is_empty(),
        "zero specialist calls are legal"
    );
    let second_primary = claim_primary(&db, second.session_id).await;
    let second_resolution_id = Uuid::new_v4();
    let second_resolve_result = json!({
        "decision":"resolve",
        "schema_version":1,
        "session_id":second.session_id,
        "hypothesis_revision_id":second.hypothesis_revision_id,
        "subtasks":[],
        "disposition":"refuted",
        "conclusion":"The second Primary independently reached a bounded conclusion.",
        "cited_evidence_ids":[],
        "new_hypothesis_proposals":[duplicate_proposal]
    });
    let (second_source_id, second_provider_id) =
        record_submit_result(&db, scope, &second_primary, second_resolve_result).await;
    verification::resolve_dynamic_hypothesis(
        db.pool(),
        &verification::ResolveDynamicHypothesisInput {
            stable_request_id: Uuid::new_v4(),
            resolution_authority_id: second_resolution_id,
            session_id: second.session_id,
            expected_session_head_version: second.head_version,
            primary_worker_fence: fence(&second_primary),
            primary_turn_id: Uuid::new_v4(),
            source_tool_call_record_id: second_source_id,
            source_provider_call_id: second_provider_id,
        },
    )
    .await
    .expect("retain an independently sourced duplicate discovery");
    let duplicate_backlog = verification::list_pending_hypothesis_discoveries(
        db.pool(),
        scope.operation_id,
        scope.asset_lane_id,
    )
    .await
    .expect("load both independently sourced duplicate discoveries");
    assert_eq!(duplicate_backlog.len(), 2);
    assert_eq!(
        duplicate_backlog[0].semantic_key_sha256,
        duplicate_backlog[1].semantic_key_sha256
    );
    assert_ne!(
        duplicate_backlog[0].resolution_authority_id,
        duplicate_backlog[1].resolution_authority_id
    );
    db.stop().await;
}
