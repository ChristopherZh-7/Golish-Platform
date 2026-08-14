use golish_db::repo::investigation_asset_queue::{
    claim_next_asset, claim_next_company, close_asset_backlog_and_advance, complete_company,
    freeze_company_asset_queue, load_asset_backlog, load_resolution_closure_publication,
    transition_asset_lane, ClaimNextInvestigationAssetRow, ClaimNextInvestigationCompanyRow,
    CloseInvestigationAssetBacklogAndAdvanceRow, CompleteInvestigationCompanyRow,
    FreezeInvestigationCompanyAssetQueueRow, InvestigationAssetLaneRow,
    InvestigationAssetProgressionDispositionRow, LoadInvestigationAssetBacklogRow,
    TransitionInvestigationAssetLaneRow,
};
use golish_db::{DbConfig, GolishDb};
use serial_test::serial;
use uuid::Uuid;

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read local postgres port")
        .port()
}

async fn migrated_db() -> (GolishDb, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("investigation_asset_queue_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start isolated migrated postgres");
    (db, data_dir)
}

struct Fixture {
    authority_id: Uuid,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    scope_snapshot_id: Uuid,
    root_organization_id: Uuid,
    child_organization_id: Uuid,
    ordered_target_ids: Vec<Uuid>,
}

async fn fixture(db: &GolishDb) -> Fixture {
    let authority_id = Uuid::new_v4();
    let operation_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    let scope_snapshot_id = Uuid::new_v4();
    let root_organization_id = Uuid::new_v4();
    let child_organization_id = Uuid::new_v4();
    let root_early_target_id = Uuid::new_v4();
    let root_late_target_id = Uuid::new_v4();
    let child_early_target_id = Uuid::new_v4();
    let child_late_target_id = Uuid::new_v4();
    let mut tx = db.pool().begin().await.expect("begin queue fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("disable unrelated fixture triggers");
    sqlx::query(
        r#"INSERT INTO organizations(id,project_path,name) VALUES
               ($1,'/fixture/project','Root'),
               ($2,'/fixture/project','Child')"#,
    )
    .bind(root_organization_id)
    .bind(child_organization_id)
    .execute(&mut *tx)
    .await
    .expect("insert frozen company organizations");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_snapshots(
               id,operation_id,project_scope_id,scope_decision_id,project_path_at_freeze,
               root_organization_id,mode,scope_hash,sealed_at)
           VALUES($1,$2,$3,$4,'/fixture/project',$5,'included',$6,statement_timestamp())"#,
    )
    .bind(scope_snapshot_id)
    .bind(operation_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(root_organization_id)
    .bind(format!("sha256:{}", "1".repeat(64)))
    .execute(&mut *tx)
    .await
    .expect("insert sealed scope snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units(
               snapshot_id,organization_id,parent_organization_id,
               organization_name_at_freeze,role,depth,ordinal,decision_row_id,approval_source)
           VALUES
               ($1,$2,NULL,'Root','root',0,0,'root','{}'::JSONB),
               ($1,$3,$2,'Child','subsidiary',1,0,'child','{}'::JSONB)"#,
    )
    .bind(scope_snapshot_id)
    .bind(root_organization_id)
    .bind(child_organization_id)
    .execute(&mut *tx)
    .await
    .expect("insert ordered company scope");
    for (target_id, organization_id, value, created_at) in [
        (
            root_late_target_id,
            root_organization_id,
            "z-root.example",
            "2026-08-13T02:00:00Z",
        ),
        (
            root_early_target_id,
            root_organization_id,
            "a-root.example",
            "2026-08-13T01:00:00Z",
        ),
        (
            child_late_target_id,
            child_organization_id,
            "z-child.example",
            "2026-08-13T04:00:00Z",
        ),
        (
            child_early_target_id,
            child_organization_id,
            "a-child.example",
            "2026-08-13T03:00:00Z",
        ),
    ] {
        sqlx::query(
            r#"INSERT INTO targets(
                   id,name,target_type,value,scope,project_path,organization_id,source,created_at)
               VALUES($1,$2,'domain',$2,'in','/fixture/project',$3,'manual',$4::TIMESTAMPTZ)"#,
        )
        .bind(target_id)
        .bind(value)
        .bind(organization_id)
        .bind(created_at)
        .execute(&mut *tx)
        .await
        .expect("insert in-scope target");
    }
    sqlx::query(
        r#"INSERT INTO investigation_run_heads(
               authority_id,stable_start_request_id,operation_id,stage_execution_id,
               owning_stage_run_request_id,scope_snapshot_id,run_state,admission_open,
               stop_epoch,change_seq,head_version,head_sha256)
           VALUES($1,$2,$3,$4,'fixture-stage-request',$5,'running',TRUE,0,0,0,
                  unified_investigation_runtime_head_sha256($1,'running',TRUE,0,0,0))"#,
    )
    .bind(authority_id)
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(scope_snapshot_id)
    .execute(&mut *tx)
    .await
    .expect("insert Investigation run head");
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status,stage_topology_contract) \
         VALUES($1,$2,'investigation','started','unified_investigation_v1')",
    )
    .bind(stage_execution_id)
    .bind(operation_id)
    .execute(&mut *tx)
    .await
    .expect("insert Investigation stage execution");
    for organization_id in [root_organization_id, child_organization_id] {
        let stage_run_unit_id = Uuid::new_v4();
        let stage_team_plan_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO stage_run_units(
                   id,operation_id,stage_execution_id,scope_snapshot_id,organization_id,
                   stage_kind,generation,status,started_at)
               VALUES($1,$2,$3,$4,$5,'investigation',0,'running',statement_timestamp())"#,
        )
        .bind(stage_run_unit_id)
        .bind(operation_id)
        .bind(stage_execution_id)
        .bind(scope_snapshot_id)
        .bind(organization_id)
        .execute(&mut *tx)
        .await
        .expect("insert Investigation company stage unit");
        sqlx::query(
            r#"INSERT INTO stage_team_plans(
                   id,operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
                   organization_id,stage_kind,unit_generation,schema_version,plan_version,
                   plan_hash,leader_role,aggregator_kind,allowed_worker_roles,
                   max_workers_total,max_workers_active,dynamic_requests_allowed,
                   final_submitter_kind,created_from_stage_spec_hash)
               VALUES($1,$2,$3,$4,$5,$6,'investigation',0,1,1,$7,'primary',
                      'deterministic',$8,4,4,TRUE,'deterministic',$9)"#,
        )
        .bind(stage_team_plan_id)
        .bind(operation_id)
        .bind(stage_execution_id)
        .bind(stage_run_unit_id)
        .bind(scope_snapshot_id)
        .bind(organization_id)
        .bind(format!("sha256:{}", "7".repeat(64)))
        .bind(serde_json::json!([
            "primary",
            "browser",
            "researcher",
            "pentester",
            "adviser"
        ]))
        .bind(format!("sha256:{}", "8".repeat(64)))
        .execute(&mut *tx)
        .await
        .expect("insert Investigation company StageTeam plan");
    }
    tx.commit().await.expect("commit queue fixture");
    Fixture {
        authority_id,
        operation_id,
        stage_execution_id,
        scope_snapshot_id,
        root_organization_id,
        child_organization_id,
        ordered_target_ids: vec![
            root_early_target_id,
            root_late_target_id,
            child_early_target_id,
            child_late_target_id,
        ],
    }
}

fn digest(fill: char) -> String {
    format!("sha256:{}", fill.to_string().repeat(64))
}

fn uuid_digest(value: Uuid) -> String {
    format!("sha256:{}{}", "0".repeat(32), value.simple())
}

#[test]
fn asset_backlog_contract_has_no_fixed_tool_action_cardinality_assumption() {
    let kit_port =
        include_str!("../../golish-agent-kit/src/db_traits/investigation_asset_queue.rs");
    let migration =
        include_str!("../migrations/20260813000006_investigation_asset_backlog_fixed_point.sql");
    let provenance_migration = include_str!(
        "../migrations/20260814000003_investigation_asset_backlog_dynamic_resolution_provenance.sql"
    );
    let discovery_migration = include_str!(
        "../migrations/20260813000008_investigation_dynamic_tool_manager_verification.sql"
    );
    let repository = include_str!("../src/repo/investigation_asset_queue.rs");
    let runtime_memory = include_str!("../src/repo/runtime_memory_tx.rs");
    for forbidden in [
        "actual_campaign_count<>actual_fact_delta_count",
        "actual_execution_count<>actual_oracle_count",
        "backlog.campaign_count != backlog.fact_delta_count",
        "backlog.action_execution_count != backlog.oracle_count",
    ] {
        assert!(!migration.contains(forbidden));
        assert!(!repository.contains(forbidden));
    }
    assert!(migration.contains("head.head_epistemic_state IN('verified','refuted','invalid')"));
    assert!(provenance_migration.contains("investigation_asset_backlog_dynamic_resolution_members"));
    assert!(provenance_migration.contains("dynamic_verification_resolution"));
    assert!(
        provenance_migration.contains("INVESTIGATION_ASSET_BACKLOG_DYNAMIC_RESOLUTION_REQUIRED")
    );
    assert!(repository.contains("dynamically_resolved_root_count"));
    assert!(!migration.contains("verification_campaign_terminal_decisions terminal"));
    assert!(!migration.contains("hypothesis_fixed_point_receipt_id"));
    assert!(
        migration.contains("hypothesis_root_count BIGINT NOT NULL CHECK(hypothesis_root_count>0)")
    );
    assert!(migration
        .contains("fixed_point_wave_count BIGINT NOT NULL CHECK(fixed_point_wave_count>=0)"));
    assert!(repository.contains("backlog.hypothesis_root_count == 0"));
    assert!(!repository.contains("backlog.hypothesis_fixed_point_receipt_id"));
    for required_backlog_member in ["hypothesis_resolution:", "pending_hypothesis_discovery:"] {
        assert!(repository.contains(required_backlog_member));
    }
    assert!(!repository.contains("'pending_evolution:'||"));
    assert!(discovery_migration
        .contains("CREATE FUNCTION investigation_guard_asset_fixed_point_pending_discoveries()"));
    assert!(discovery_migration
        .contains("FROM investigation_pending_hypothesis_discovery_backlog backlog"));
    assert!(discovery_migration
        .contains("BEFORE INSERT ON investigation_asset_backlog_fixed_point_receipts"));
    let zero_request = kit_port
        .split("pub struct SealZeroHypothesisAssetFixedPoint")
        .nth(1)
        .expect("zero fixed request")
        .split('}')
        .next()
        .expect("zero fixed request body");
    for server_derived_authority in [
        "compilation_decision_id",
        "generation_id",
        "generation_seal_id",
        "canonical_apply_receipt_id",
        "backlog_set_sha256",
        "obligation_set_sha256",
        "residual_set_sha256",
    ] {
        assert!(!zero_request.contains(server_derived_authority));
    }
    assert!(repository.contains("WITH latest_generation AS"));
    assert!(repository.contains("investigation_hypothesis_canonical_apply_receipts apply_receipt"));
    assert!(repository.contains("decision.proposal_count=0"));
    assert!(repository.contains("apply_receipt.revision_count=0"));
    for tool_audit_not_gate in [
        "'task:'||",
        "'action:'||",
        "'execution:'||",
        "'campaign_closeout:'||",
        "'fact_delta_consumption:'||",
        "'wave_consolidation:'||",
    ] {
        assert!(!repository.contains(tool_audit_not_gate));
    }
    let transition_guard = runtime_memory
        .split("if current.stage_kind == \"investigation\"")
        .nth(1)
        .expect("Investigation stage transition guard")
        .split("let previous_stage_execution")
        .next()
        .expect("bounded Investigation stage transition guard");
    assert!(transition_guard.contains("investigation_asset_queue_closure_publications"));
    assert!(transition_guard.contains("investigation_asset_queue_closure_publication_members"));
    assert!(transition_guard.contains("investigation_asset_progression_receipts"));
    assert!(transition_guard.contains("stage_team_plans"));
    assert!(transition_guard.contains("investigation_asset_queue_closure_publication.v1"));
    assert!(!transition_guard.contains("investigation_stage_closure_publications"));
}

#[test]
fn evolution_authority_selector_is_server_derived_and_exactly_asset_epoch_scoped() {
    let kit_port =
        include_str!("../../golish-agent-kit/src/db_traits/investigation_asset_queue.rs");
    let repository = include_str!("../src/repo/investigation_asset_queue.rs");
    let selector = repository
        .split_once("pub async fn load_current_evolution_authority")
        .expect("server evolution authority selector")
        .1
        .split_once("#[derive(Debug, sqlx::FromRow)]")
        .expect("selector boundary")
        .0;

    assert!(!kit_port
        .split_once("pub struct LoadCurrentInvestigationAssetEvolutionAuthority")
        .expect("portable selector")
        .1
        .split_once('}')
        .expect("portable selector boundary")
        .0
        .contains("pending_evolution_authority_id"));
    for exact_guard in [
        "lane.operation_id=$2",
        "lane.stage_execution_id=$3",
        "lane.scope_snapshot_id=$4",
        "lane.organization_id=$5",
        "lane.evolution_epoch=$6",
        "generation.generation_ordinal+1=lane.evolution_epoch",
        "schedule.schedule_contract='primary_dynamic_v2'",
        "terminal.consolidation_batch_id=pending.consolidation_batch_id",
    ] {
        assert!(selector.contains(exact_guard), "missing {exact_guard}");
    }
    assert!(!selector.contains("LIMIT 1"));
    assert!(selector.contains("let [row] = rows.as_slice()"));
}

async fn seed_lane_resolved_hypothesis(
    db: &GolishDb,
    fixture: &Fixture,
    lane: &InvestigationAssetLaneRow,
    generation_ordinal: i32,
    epistemic_state: &str,
    dynamic_provenance: bool,
) {
    let generation_id = Uuid::new_v4();
    let generation_seal_id = Uuid::new_v4();
    let root_id = Uuid::new_v4();
    let source_revision_id = Uuid::new_v4();
    let terminal_revision_id = Uuid::new_v4();
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin lane fixed authority seed");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("disable seed-only authority triggers");
    sqlx::query(
        r#"INSERT INTO attack_hypotheses(
               root_id,operation_id,organization_id,root_kind,
               identity_ingredients,identity_ingredients_hash,asset_lane_id)
           VALUES($1,$2,$3,'initial','{}'::JSONB,$4,$5)"#,
    )
    .bind(root_id)
    .bind(fixture.operation_id)
    .bind(lane.organization_id)
    .bind(uuid_digest(root_id))
    .bind(lane.asset_lane_id)
    .execute(&mut *tx)
    .await
    .expect("seed lane hypothesis root");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_revisions(
               revision_id,root_id,operation_id,organization_id,predecessor_revision_id,
               revision_ordinal,semantic_key,semantic_key_hash,subject_kind,
               subject_identity_hash,target_live_id,target_type_at_time,target_value_at_time,
               predicate_schema,predicate_version,normalized_arguments,trust_boundary,
               polarity,epistemic_state,lifecycle_state,planning_readiness,structured_claim,
               assumptions,missing_facts,priority,risk_impact,origin_decision_hash,
               revision_ingredients_hash,revision_hash,asset_lane_id)
           VALUES($1,$2,$3,$4,NULL,0,'{}'::JSONB,$5,'asset',$6,$7,'domain',$8,
                  'fixture',1,'{}'::JSONB,'external','positive','proposed','current',
                  'ready_for_strategy','{}'::JSONB,'[]'::JSONB,'[]'::JSONB,0,'{}'::JSONB,
                  $9,$10,$11,$12)"#,
    )
    .bind(source_revision_id)
    .bind(root_id)
    .bind(fixture.operation_id)
    .bind(lane.organization_id)
    .bind(uuid_digest(source_revision_id))
    .bind(lane.target_identity_sha256.clone())
    .bind(lane.target_id)
    .bind(lane.target_value_at_freeze.clone())
    .bind(digest('a'))
    .bind(digest('b'))
    .bind(uuid_digest(source_revision_id))
    .bind(lane.asset_lane_id)
    .execute(&mut *tx)
    .await
    .expect("seed lane hypothesis revision");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_revisions(
               revision_id,root_id,operation_id,organization_id,predecessor_revision_id,
               revision_ordinal,semantic_key,semantic_key_hash,subject_kind,
               subject_identity_hash,target_live_id,target_type_at_time,target_value_at_time,
               predicate_schema,predicate_version,normalized_arguments,trust_boundary,
               polarity,epistemic_state,lifecycle_state,planning_readiness,structured_claim,
               assumptions,missing_facts,priority,risk_impact,origin_decision_hash,
               revision_ingredients_hash,revision_hash,asset_lane_id)
           SELECT $1,root_id,operation_id,organization_id,revision_id,1,semantic_key,
                  semantic_key_hash,subject_kind,subject_identity_hash,target_live_id,
                  target_type_at_time,target_value_at_time,predicate_schema,predicate_version,
                  normalized_arguments,trust_boundary,polarity,$2,'closed','deferred',
                  structured_claim,assumptions,missing_facts,priority,risk_impact,$3,$4,$5,asset_lane_id
             FROM attack_hypothesis_revisions WHERE revision_id=$6"#,
    )
    .bind(terminal_revision_id)
    .bind(epistemic_state)
    .bind(digest('c'))
    .bind(digest('d'))
    .bind(uuid_digest(terminal_revision_id))
    .bind(source_revision_id)
    .execute(&mut *tx)
    .await
    .expect("seed dynamic terminal successor revision");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_heads(
               root_id,operation_id,organization_id,head_revision_id,
               head_revision_hash,head_semantic_key_hash,head_epistemic_state,
               head_lifecycle_state,head_version)
           VALUES($1,$2,$3,$4,$5,$6,$7,'closed',0)"#,
    )
    .bind(root_id)
    .bind(fixture.operation_id)
    .bind(lane.organization_id)
    .bind(terminal_revision_id)
    .bind(uuid_digest(terminal_revision_id))
    .bind(uuid_digest(source_revision_id))
    .bind(epistemic_state)
    .execute(&mut *tx)
    .await
    .expect("seed canonical resolved hypothesis head");
    if dynamic_provenance {
        let session_id = Uuid::new_v4();
        let primary_turn_id = Uuid::new_v4();
        let resolution_id = Uuid::new_v4();
        let transition_id = Uuid::new_v4();
        let state_event_id = Uuid::new_v4();
        let primary_work_item_id = Uuid::new_v4();
        let primary_worker_run_id = Uuid::new_v4();
        let message_chain_id = Uuid::new_v4();
        let resolution_sha256 = digest('e');
        sqlx::query(
            r#"INSERT INTO investigation_dynamic_verification_rounds(
                   session_id,stable_request_id,operation_id,project_scope_id,
                   stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id,
                   asset_lane_id,target_live_id,hypothesis_revision_id,verification_task_id,
                   asset_primary_schedule_receipt_id,evolution_epoch,round_rearm_id,
                   stage_team_plan_id,dispatch_epoch,session_authorization_id,
                   session_budget_envelope_id,authorization_expires_at,
                   source_primary_work_item_id,source_primary_worker_run_id,
                   primary_work_item_id,primary_worker_run_id,primary_message_chain_id,
                   state,head_version,resolution_authority_id,resolved_at)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,0,$14,$15,0,$16,$17,
                      statement_timestamp()+INTERVAL '1 hour',$18,$19,$18,$19,$20,
                      'resolved',1,$21,statement_timestamp())"#,
        )
        .bind(session_id)
        .bind(Uuid::new_v4())
        .bind(fixture.operation_id)
        .bind(Uuid::new_v4())
        .bind(fixture.stage_execution_id)
        .bind(Uuid::new_v4())
        .bind(fixture.scope_snapshot_id)
        .bind(lane.organization_id)
        .bind(lane.asset_lane_id)
        .bind(lane.target_id)
        .bind(source_revision_id)
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(primary_work_item_id)
        .bind(primary_worker_run_id)
        .bind(message_chain_id)
        .bind(resolution_id)
        .execute(&mut *tx)
        .await
        .expect("seed resolved dynamic verification round");
        sqlx::query(
            r#"INSERT INTO investigation_dynamic_verification_primary_turns(
                   primary_turn_id,stable_request_id,session_id,turn_ordinal,decision_kind,
                   expected_session_head_version,source_primary_work_item_id,
                   source_primary_worker_run_id,source_primary_lease_token,
                   source_primary_attempt_epoch,consumer_primary_lease_token,
                   consumer_primary_attempt_epoch,consumer_primary_checkpoint_version,
                   consumer_primary_checkpoint_sha256,source_tool_call_record_id,
                   source_provider_call_id,canonical_turn_sha256,actor_call_count,
                   actor_call_set_sha256)
               VALUES($1,$2,$3,1,'resolve',0,$4,$5,$6,0,$7,0,0,$8,$9,'fixture-call',$10,0,$11)"#,
        )
        .bind(primary_turn_id)
        .bind(Uuid::new_v4())
        .bind(session_id)
        .bind(primary_work_item_id)
        .bind(primary_worker_run_id)
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(digest('1'))
        .bind(Uuid::new_v4())
        .bind(digest('2'))
        .bind(digest('3'))
        .execute(&mut *tx)
        .await
        .expect("seed dynamic Primary resolve turn");
        sqlx::query(
            r#"INSERT INTO investigation_dynamic_hypothesis_resolutions(
                   resolution_authority_id,stable_request_id,session_id,primary_turn_id,
                   asset_lane_id,target_live_id,hypothesis_revision_id,
                   expected_session_head_version,primary_work_item_id,primary_worker_run_id,
                   primary_message_chain_id,primary_lease_token,primary_attempt_epoch,
                   primary_checkpoint_version,disposition,primary_conclusion_sha256,
                   conclusion_redacted,citation_count,citation_set_sha256,resolution_sha256)
               VALUES($1,$2,$3,$4,$5,$6,$7,0,$8,$9,$10,$11,0,0,$12,$13,'{}',0,$14,$15)"#,
        )
        .bind(resolution_id)
        .bind(Uuid::new_v4())
        .bind(session_id)
        .bind(primary_turn_id)
        .bind(lane.asset_lane_id)
        .bind(lane.target_id)
        .bind(source_revision_id)
        .bind(primary_work_item_id)
        .bind(primary_worker_run_id)
        .bind(message_chain_id)
        .bind(Uuid::new_v4())
        .bind(epistemic_state)
        .bind(digest('4'))
        .bind(digest('5'))
        .bind(&resolution_sha256)
        .execute(&mut *tx)
        .await
        .expect("seed dynamic resolution authority");
        let event_kind = if epistemic_state == "invalid" {
            "invalidated"
        } else {
            epistemic_state
        };
        sqlx::query(
            r#"INSERT INTO attack_hypothesis_state_events(
                   event_id,operation_id,organization_id,root_id,predecessor_revision_id,
                   successor_revision_id,event_kind,origin_authority,successor_epistemic_state,
                   authority_receipt_kind,authority_receipt_id,authority_receipt_hash,event_hash,
                   server_decision_id,server_decision_hash)
               VALUES($1,$2,$3,$4,$5,$6,$7,'dynamic_verification_resolution',$8,
                      'dynamic_resolution',$9,$10,$11,$9,$10)"#,
        )
        .bind(state_event_id)
        .bind(fixture.operation_id)
        .bind(lane.organization_id)
        .bind(root_id)
        .bind(source_revision_id)
        .bind(terminal_revision_id)
        .bind(event_kind)
        .bind(epistemic_state)
        .bind(resolution_id)
        .bind(&resolution_sha256)
        .bind(digest('6'))
        .execute(&mut *tx)
        .await
        .expect("seed dynamic terminal state event");
        sqlx::query(
            r#"INSERT INTO investigation_dynamic_hypothesis_terminal_transitions(
                   terminal_transition_id,stable_request_id,resolution_authority_id,
                   asset_lane_id,source_revision_id,terminal_revision_id,state_event_id,
                   disposition,transition_sha256)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)"#,
        )
        .bind(transition_id)
        .bind(Uuid::new_v4())
        .bind(resolution_id)
        .bind(lane.asset_lane_id)
        .bind(source_revision_id)
        .bind(terminal_revision_id)
        .bind(state_event_id)
        .bind(epistemic_state)
        .bind(digest('7'))
        .execute(&mut *tx)
        .await
        .expect("seed dynamic terminal transition");
    }
    sqlx::query(
        r#"INSERT INTO hypothesis_generations(
               generation_id,operation_id,organization_id,generation_ordinal,
               candidate_snapshot_id,candidate_gate_decision_id,
               candidate_snapshot_authority_hash,previous_generation_id,asset_lane_id)
           VALUES($1,$2,$3,$4,$5,$6,$7,NULL,$8)"#,
    )
    .bind(generation_id)
    .bind(fixture.operation_id)
    .bind(lane.organization_id)
    .bind(generation_ordinal)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(digest('a'))
    .bind(lane.asset_lane_id)
    .execute(&mut *tx)
    .await
    .expect("seed exact lane generation");
    sqlx::query(
        r#"INSERT INTO hypothesis_generation_members(
               generation_member_id,generation_id,operation_id,organization_id,
               revision_id,ordinal,member_hash,asset_lane_id)
           VALUES($1,$2,$3,$4,$5,0,$6,$7)"#,
    )
    .bind(Uuid::new_v4())
    .bind(generation_id)
    .bind(fixture.operation_id)
    .bind(lane.organization_id)
    .bind(terminal_revision_id)
    .bind(uuid_digest(generation_id))
    .bind(lane.asset_lane_id)
    .execute(&mut *tx)
    .await
    .expect("seed exact generation member");
    sqlx::query(
        r#"INSERT INTO hypothesis_generation_seals(
               seal_id,generation_id,member_count,member_set_hash,event_count,event_set_hash,
               open_obligation_set_hash,controller_worker_run_id,generation_hash)
           VALUES($1,$2,1,$3,0,$4,$5,$6,$7)"#,
    )
    .bind(generation_seal_id)
    .bind(generation_id)
    .bind(digest('b'))
    .bind(digest('c'))
    .bind(digest('d'))
    .bind(Uuid::new_v4())
    .bind(digest('e'))
    .execute(&mut *tx)
    .await
    .expect("seal exact lane generation");
    tx.commit()
        .await
        .expect("commit resolved lane hypothesis seed");
}

async fn advance_lane_to_consolidating(
    db: &GolishDb,
    fixture: &Fixture,
    frozen_company_queue_id: Uuid,
    lane: InvestigationAssetLaneRow,
) -> InvestigationAssetLaneRow {
    let lane = transition_asset_lane(
        db.pool(),
        &TransitionInvestigationAssetLaneRow {
            stable_request_id: Uuid::new_v4(),
            company_queue_id: frozen_company_queue_id,
            company_member_id: lane.company_member_id,
            asset_queue_id: lane.asset_queue_id,
            asset_lane_id: lane.asset_lane_id,
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            organization_id: lane.organization_id,
            expected_queue_head_version: lane.asset_queue_head_version,
            expected_lane_row_version: lane.row_version,
            from_state: "analyzing",
            to_state: "verifying",
        },
    )
    .await
    .expect("start exact lane verification");
    transition_asset_lane(
        db.pool(),
        &TransitionInvestigationAssetLaneRow {
            stable_request_id: Uuid::new_v4(),
            company_queue_id: frozen_company_queue_id,
            company_member_id: lane.company_member_id,
            asset_queue_id: lane.asset_queue_id,
            asset_lane_id: lane.asset_lane_id,
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            organization_id: lane.organization_id,
            expected_queue_head_version: lane.asset_queue_head_version,
            expected_lane_row_version: lane.row_version,
            from_state: "verifying",
            to_state: "consolidating",
        },
    )
    .await
    .expect("start exact lane consolidation")
}

#[tokio::test]
#[serial]
async fn freezes_server_ordered_company_and_asset_queues_with_exact_replay() {
    let (mut db, _data_dir) = migrated_db().await;
    let fixture = fixture(&db).await;
    let request = FreezeInvestigationCompanyAssetQueueRow {
        stable_request_id: Uuid::new_v4(),
        authority_id: fixture.authority_id,
        operation_id: fixture.operation_id,
        stage_execution_id: fixture.stage_execution_id,
        owning_stage_run_request_id: "fixture-stage-request".to_string(),
        scope_snapshot_id: fixture.scope_snapshot_id,
        max_evolution_epochs: 2,
    };
    let frozen = freeze_company_asset_queue(db.pool(), &request)
        .await
        .expect("freeze authoritative nested queues");
    assert_eq!(frozen.company_member_count, 2);
    assert_eq!(
        frozen.companies[0].organization_id,
        fixture.root_organization_id
    );
    assert_eq!(
        frozen.companies[1].organization_id,
        fixture.child_organization_id
    );
    assert_eq!(frozen.companies[0].depth, 0);
    assert_eq!(frozen.companies[0].ordinal, 0);
    assert_eq!(frozen.companies[1].depth, 1);
    assert_eq!(frozen.companies[1].ordinal, 0);
    assert_eq!(
        frozen
            .assets
            .iter()
            .map(|lane| lane.target_id)
            .collect::<Vec<_>>(),
        fixture.ordered_target_ids
    );
    assert!(frozen
        .companies
        .iter()
        .all(|member| member.state == "queued"));
    assert!(frozen
        .companies
        .iter()
        .all(|member| member.company_queue_head_version == 0));
    assert!(frozen.assets.iter().all(|lane| lane.state == "queued"));
    assert!(frozen
        .assets
        .iter()
        .all(|lane| lane.asset_queue_head_version == 0));
    assert!(sqlx::query(
        "UPDATE investigation_asset_lanes SET target_value_at_freeze='drift.example'
          WHERE asset_lane_id=$1",
    )
    .bind(frozen.assets[0].asset_lane_id)
    .execute(db.pool())
    .await
    .expect_err("frozen asset identity cannot be rewritten")
    .to_string()
    .contains("INVESTIGATION_ASSET_QUEUE_EVENT_REQUIRED"));
    let mut raw_insert = db.pool().begin().await.expect("begin raw member drift");
    sqlx::query(
        r#"INSERT INTO investigation_asset_lanes(
               asset_lane_id,asset_queue_id,company_queue_id,company_member_id,
               authority_id,operation_id,stage_execution_id,scope_snapshot_id,
               organization_id,target_id,target_type_at_freeze,target_value_at_freeze,
               target_source_at_freeze,target_created_at,target_identity_sha256,ordinal,
               max_evolution_epochs)
           SELECT $1,asset_queue_id,company_queue_id,company_member_id,authority_id,
                  operation_id,stage_execution_id,scope_snapshot_id,organization_id,$2,
                  target_type_at_freeze,'raw-drift.example',target_source_at_freeze,
                  target_created_at,$3,99,max_evolution_epochs
             FROM investigation_asset_lanes WHERE asset_lane_id=$4"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(format!("sha256:{}", "f".repeat(64)))
    .bind(frozen.assets[0].asset_lane_id)
    .execute(&mut *raw_insert)
    .await
    .expect("deferred denominator guard permits statement ordering");
    assert!(raw_insert
        .commit()
        .await
        .expect_err("sealed asset denominator cannot be extended")
        .to_string()
        .contains("INVESTIGATION_ASSET_QUEUE_MEMBER_COUNT_DRIFT"));
    let replay = freeze_company_asset_queue(db.pool(), &request)
        .await
        .expect("exact freeze replay");
    assert_eq!(replay.company_queue_id, frozen.company_queue_id);
    assert!(replay.replayed);
    let queue_count: i64 = sqlx::query_scalar("SELECT count(*) FROM investigation_company_queues")
        .fetch_one(db.pool())
        .await
        .expect("count queue seals");
    assert_eq!(queue_count, 1);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn enforces_strict_order_cas_replay_evolution_fuel_and_company_barrier() {
    let (mut db, _data_dir) = migrated_db().await;
    let fixture = fixture(&db).await;
    let frozen = freeze_company_asset_queue(
        db.pool(),
        &FreezeInvestigationCompanyAssetQueueRow {
            stable_request_id: Uuid::new_v4(),
            authority_id: fixture.authority_id,
            operation_id: fixture.operation_id,
            stage_execution_id: fixture.stage_execution_id,
            owning_stage_run_request_id: "fixture-stage-request".to_string(),
            scope_snapshot_id: fixture.scope_snapshot_id,
            max_evolution_epochs: 1,
        },
    )
    .await
    .expect("freeze queue state machine");
    let root = frozen.companies[0].clone();
    let child = frozen.companies[1].clone();
    let root_assets = frozen
        .assets
        .iter()
        .filter(|lane| lane.organization_id == fixture.root_organization_id)
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(root_assets.len(), 2);

    let out_of_order_company = ClaimNextInvestigationCompanyRow {
        stable_request_id: Uuid::new_v4(),
        company_queue_id: frozen.company_queue_id,
        operation_id: fixture.operation_id,
        scope_snapshot_id: fixture.scope_snapshot_id,
        expected_company_member_id: child.company_member_id,
        expected_queue_head_version: 0,
        expected_member_row_version: 0,
    };
    assert!(claim_next_company(db.pool(), &out_of_order_company)
        .await
        .expect_err("cannot skip the root company")
        .to_string()
        .contains("INVESTIGATION_COMPANY_QUEUE_ORDER_CONFLICT"));
    let company_events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM investigation_company_queue_events")
            .fetch_one(db.pool())
            .await
            .expect("count rolled-back company events");
    assert_eq!(company_events, 0);

    let company_claim = ClaimNextInvestigationCompanyRow {
        stable_request_id: Uuid::new_v4(),
        expected_company_member_id: root.company_member_id,
        ..out_of_order_company
    };
    let active_root = claim_next_company(db.pool(), &company_claim)
        .await
        .expect("claim first company");
    assert_eq!(active_root.state, "active");
    assert_eq!(active_root.company_queue_head_version, 1);
    assert_eq!(active_root.row_version, 1);
    assert_eq!(
        claim_next_company(db.pool(), &company_claim)
            .await
            .expect("replay first company claim"),
        active_root
    );

    let second_lane = &root_assets[1];
    let out_of_order_asset = ClaimNextInvestigationAssetRow {
        stable_request_id: Uuid::new_v4(),
        company_queue_id: frozen.company_queue_id,
        company_member_id: root.company_member_id,
        asset_queue_id: second_lane.asset_queue_id,
        operation_id: fixture.operation_id,
        scope_snapshot_id: fixture.scope_snapshot_id,
        organization_id: fixture.root_organization_id,
        expected_asset_lane_id: second_lane.asset_lane_id,
        expected_queue_head_version: 0,
        expected_lane_row_version: 0,
    };
    assert!(claim_next_asset(db.pool(), &out_of_order_asset)
        .await
        .expect_err("cannot skip the first asset")
        .to_string()
        .contains("INVESTIGATION_ASSET_QUEUE_ORDER_CONFLICT"));
    let foreign_asset = ClaimNextInvestigationAssetRow {
        stable_request_id: Uuid::new_v4(),
        organization_id: fixture.child_organization_id,
        expected_asset_lane_id: root_assets[0].asset_lane_id,
        asset_queue_id: root_assets[0].asset_queue_id,
        ..out_of_order_asset
    };
    assert!(claim_next_asset(db.pool(), &foreign_asset).await.is_err());
    let asset_events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM investigation_asset_lane_events")
            .fetch_one(db.pool())
            .await
            .expect("count rolled-back asset events");
    assert_eq!(asset_events, 0);

    let first_lane = &root_assets[0];
    let first_claim = ClaimNextInvestigationAssetRow {
        stable_request_id: Uuid::new_v4(),
        company_queue_id: frozen.company_queue_id,
        company_member_id: root.company_member_id,
        asset_queue_id: first_lane.asset_queue_id,
        operation_id: fixture.operation_id,
        scope_snapshot_id: fixture.scope_snapshot_id,
        organization_id: fixture.root_organization_id,
        expected_asset_lane_id: first_lane.asset_lane_id,
        expected_queue_head_version: 0,
        expected_lane_row_version: 0,
    };
    let mut lane = claim_next_asset(db.pool(), &first_claim)
        .await
        .expect("claim first asset");
    assert_eq!(lane.state, "analyzing");
    assert_eq!(lane.asset_queue_head_version, 1);
    assert_eq!(
        claim_next_asset(db.pool(), &first_claim)
            .await
            .expect("replay first asset claim"),
        lane
    );
    let stale = ClaimNextInvestigationAssetRow {
        stable_request_id: Uuid::new_v4(),
        expected_queue_head_version: 0,
        expected_lane_row_version: 0,
        ..first_claim
    };
    assert!(claim_next_asset(db.pool(), &stale).await.is_err());
    let asset_events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM investigation_asset_lane_events")
            .fetch_one(db.pool())
            .await
            .expect("count exact asset events");
    assert_eq!(asset_events, 1);

    let blocked_complete = CompleteInvestigationCompanyRow {
        stable_request_id: Uuid::new_v4(),
        company_queue_id: frozen.company_queue_id,
        company_member_id: root.company_member_id,
        operation_id: fixture.operation_id,
        scope_snapshot_id: fixture.scope_snapshot_id,
        organization_id: fixture.root_organization_id,
        expected_queue_head_version: 1,
        expected_member_row_version: 1,
    };
    assert!(complete_company(db.pool(), &blocked_complete)
        .await
        .expect_err("company cannot complete with open assets")
        .to_string()
        .contains("INVESTIGATION_COMPANY_QUEUE_ASSETS_OPEN"));

    let mut asset_head_version = 1;
    for (from_state, to_state) in [
        ("analyzing", "verifying"),
        ("verifying", "consolidating"),
        ("consolidating", "evolving"),
        ("evolving", "analyzing"),
        ("analyzing", "verifying"),
        ("verifying", "consolidating"),
    ] {
        lane = transition_asset_lane(
            db.pool(),
            &TransitionInvestigationAssetLaneRow {
                stable_request_id: Uuid::new_v4(),
                company_queue_id: frozen.company_queue_id,
                company_member_id: root.company_member_id,
                asset_queue_id: lane.asset_queue_id,
                asset_lane_id: lane.asset_lane_id,
                operation_id: fixture.operation_id,
                scope_snapshot_id: fixture.scope_snapshot_id,
                organization_id: fixture.root_organization_id,
                expected_queue_head_version: asset_head_version,
                expected_lane_row_version: lane.row_version,
                from_state,
                to_state,
            },
        )
        .await
        .expect("advance asset lifecycle");
        asset_head_version += 1;
        assert_eq!(lane.asset_queue_head_version, asset_head_version);
    }
    assert_eq!(lane.evolution_epoch, 1);
    assert!(transition_asset_lane(
        db.pool(),
        &TransitionInvestigationAssetLaneRow {
            stable_request_id: Uuid::new_v4(),
            company_queue_id: frozen.company_queue_id,
            company_member_id: root.company_member_id,
            asset_queue_id: lane.asset_queue_id,
            asset_lane_id: lane.asset_lane_id,
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            organization_id: fixture.root_organization_id,
            expected_queue_head_version: asset_head_version,
            expected_lane_row_version: lane.row_version,
            from_state: "consolidating",
            to_state: "evolving",
        },
    )
    .await
    .expect_err("durable evolution fuel is exhausted")
    .to_string()
    .contains("INVESTIGATION_ASSET_QUEUE_EVOLUTION_FUEL_EXHAUSTED"));
    assert!(transition_asset_lane(
        db.pool(),
        &TransitionInvestigationAssetLaneRow {
            stable_request_id: Uuid::new_v4(),
            company_queue_id: frozen.company_queue_id,
            company_member_id: root.company_member_id,
            asset_queue_id: lane.asset_queue_id,
            asset_lane_id: lane.asset_lane_id,
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            organization_id: fixture.root_organization_id,
            expected_queue_head_version: asset_head_version,
            expected_lane_row_version: lane.row_version,
            from_state: "consolidating",
            to_state: "fixed_point",
        },
    )
    .await
    .expect_err("ordinary fixed point remains closed")
    .to_string()
    .contains("INVESTIGATION_ASSET_QUEUE_CONTRACT_INVALID"));
    lane = transition_asset_lane(
        db.pool(),
        &TransitionInvestigationAssetLaneRow {
            stable_request_id: Uuid::new_v4(),
            company_queue_id: frozen.company_queue_id,
            company_member_id: root.company_member_id,
            asset_queue_id: lane.asset_queue_id,
            asset_lane_id: lane.asset_lane_id,
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            organization_id: fixture.root_organization_id,
            expected_queue_head_version: asset_head_version,
            expected_lane_row_version: lane.row_version,
            from_state: "consolidating",
            to_state: "residual",
        },
    )
    .await
    .expect("seal first lane residual after fuel exhaustion");
    asset_head_version += 1;
    assert_eq!(lane.state, "residual");

    let mut second = claim_next_asset(
        db.pool(),
        &ClaimNextInvestigationAssetRow {
            stable_request_id: Uuid::new_v4(),
            company_queue_id: frozen.company_queue_id,
            company_member_id: root.company_member_id,
            asset_queue_id: second_lane.asset_queue_id,
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            organization_id: fixture.root_organization_id,
            expected_asset_lane_id: second_lane.asset_lane_id,
            expected_queue_head_version: asset_head_version,
            expected_lane_row_version: 0,
        },
    )
    .await
    .expect("claim second asset only after first terminal");
    asset_head_version += 1;
    for (from_state, to_state) in [
        ("analyzing", "verifying"),
        ("verifying", "consolidating"),
        ("consolidating", "residual"),
    ] {
        second = transition_asset_lane(
            db.pool(),
            &TransitionInvestigationAssetLaneRow {
                stable_request_id: Uuid::new_v4(),
                company_queue_id: frozen.company_queue_id,
                company_member_id: root.company_member_id,
                asset_queue_id: second.asset_queue_id,
                asset_lane_id: second.asset_lane_id,
                operation_id: fixture.operation_id,
                scope_snapshot_id: fixture.scope_snapshot_id,
                organization_id: fixture.root_organization_id,
                expected_queue_head_version: asset_head_version,
                expected_lane_row_version: second.row_version,
                from_state,
                to_state,
            },
        )
        .await
        .expect("drain second asset");
        asset_head_version += 1;
    }
    assert_eq!(second.state, "residual");

    let completion = CompleteInvestigationCompanyRow {
        stable_request_id: Uuid::new_v4(),
        ..blocked_complete
    };
    let completed = complete_company(db.pool(), &completion)
        .await
        .expect("complete company after all assets terminal");
    assert_eq!(completed.state, "completed");
    assert_eq!(completed.company_queue_head_version, 2);
    assert_eq!(
        complete_company(db.pool(), &completion)
            .await
            .expect("replay company completion"),
        completed
    );
    let drifted_completion = CompleteInvestigationCompanyRow {
        organization_id: fixture.child_organization_id,
        ..completion
    };
    assert!(complete_company(db.pool(), &drifted_completion)
        .await
        .expect_err("completion replay drift is rejected")
        .to_string()
        .contains("INVESTIGATION_ASSET_QUEUE_REPLAY_DRIFT"));

    let child_claim = claim_next_company(
        db.pool(),
        &ClaimNextInvestigationCompanyRow {
            stable_request_id: Uuid::new_v4(),
            company_queue_id: frozen.company_queue_id,
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            expected_company_member_id: child.company_member_id,
            expected_queue_head_version: 2,
            expected_member_row_version: 0,
        },
    )
    .await
    .expect("claim child only after root completion");
    assert_eq!(child_claim.state, "active");
    assert_eq!(child_claim.company_queue_head_version, 3);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn company_member_ordinal_is_scoped_by_depth_not_globally_unique() {
    let (mut db, _data_dir) = migrated_db().await;
    let fixture = fixture(&db).await;
    let frozen = freeze_company_asset_queue(
        db.pool(),
        &FreezeInvestigationCompanyAssetQueueRow {
            stable_request_id: Uuid::new_v4(),
            authority_id: fixture.authority_id,
            operation_id: fixture.operation_id,
            stage_execution_id: fixture.stage_execution_id,
            owning_stage_run_request_id: "fixture-stage-request".to_string(),
            scope_snapshot_id: fixture.scope_snapshot_id,
            max_evolution_epochs: 0,
        },
    )
    .await
    .expect("freeze queue");
    let duplicates: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM investigation_company_queue_members
          WHERE company_queue_id=$1 AND ordinal=0",
    )
    .bind(frozen.company_queue_id)
    .fetch_one(db.pool())
    .await
    .expect("count same ordinal at different depths");
    assert_eq!(duplicates, 2);
    assert_eq!(
        frozen
            .companies
            .iter()
            .map(|member| member.depth)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn asset_backlog_is_lane_scoped_and_server_derived() {
    let (mut db, _data_dir) = migrated_db().await;
    let fixture = fixture(&db).await;
    let frozen = freeze_company_asset_queue(
        db.pool(),
        &FreezeInvestigationCompanyAssetQueueRow {
            stable_request_id: Uuid::new_v4(),
            authority_id: fixture.authority_id,
            operation_id: fixture.operation_id,
            stage_execution_id: fixture.stage_execution_id,
            owning_stage_run_request_id: "fixture-stage-request".to_string(),
            scope_snapshot_id: fixture.scope_snapshot_id,
            max_evolution_epochs: 1,
        },
    )
    .await
    .expect("freeze lane backlog fixture");
    let lane = frozen.assets[0].clone();
    let backlog = load_asset_backlog(
        db.pool(),
        &LoadInvestigationAssetBacklogRow {
            company_queue_id: frozen.company_queue_id,
            company_member_id: lane.company_member_id,
            asset_queue_id: lane.asset_queue_id,
            asset_lane_id: lane.asset_lane_id,
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            organization_id: lane.organization_id,
        },
    )
    .await
    .expect("derive exact lane backlog without operation-global fallback");
    assert_eq!(backlog.asset_lane.asset_lane_id, lane.asset_lane_id);
    assert_eq!(backlog.generation_count, 0);
    assert_eq!(backlog.backlog_member_count, 0);
    assert_eq!(backlog.pending_evolution_count, 0);
    assert_eq!(backlog.pending_hypothesis_discovery_count, 0);
    assert_eq!(backlog.hypothesis_root_count, 0);
    assert_eq!(backlog.dynamically_resolved_root_count, 0);
    assert_eq!(backlog.revision_count, 0);
    seed_lane_resolved_hypothesis(&db, &fixture, &lane, 0, "verified", false).await;
    let legacy_closed = load_asset_backlog(
        db.pool(),
        &LoadInvestigationAssetBacklogRow {
            company_queue_id: frozen.company_queue_id,
            company_member_id: lane.company_member_id,
            asset_queue_id: lane.asset_queue_id,
            asset_lane_id: lane.asset_lane_id,
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            organization_id: lane.organization_id,
        },
    )
    .await
    .expect("load legacy closed head as unresolved dynamic backlog");
    assert_eq!(legacy_closed.hypothesis_root_count, 1);
    assert_eq!(legacy_closed.dynamically_resolved_root_count, 0);
    assert_eq!(legacy_closed.backlog_member_count, 1);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn fixed_point_close_drains_one_lane_and_atomically_advances_assets_and_companies() {
    let (mut db, _data_dir) = migrated_db().await;
    let fixture = fixture(&db).await;
    let frozen = freeze_company_asset_queue(
        db.pool(),
        &FreezeInvestigationCompanyAssetQueueRow {
            stable_request_id: Uuid::new_v4(),
            authority_id: fixture.authority_id,
            operation_id: fixture.operation_id,
            stage_execution_id: fixture.stage_execution_id,
            owning_stage_run_request_id: "fixture-stage-request".to_string(),
            scope_snapshot_id: fixture.scope_snapshot_id,
            max_evolution_epochs: 1,
        },
    )
    .await
    .expect("freeze progression fixture");
    let root = &frozen.companies[0];
    let root_active = claim_next_company(
        db.pool(),
        &ClaimNextInvestigationCompanyRow {
            stable_request_id: Uuid::new_v4(),
            company_queue_id: frozen.company_queue_id,
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            expected_company_member_id: root.company_member_id,
            expected_queue_head_version: 0,
            expected_member_row_version: 0,
        },
    )
    .await
    .expect("claim root company");
    let first_root_lane = frozen
        .assets
        .iter()
        .find(|lane| lane.organization_id == fixture.root_organization_id)
        .expect("root first lane");
    let mut active_lane = claim_next_asset(
        db.pool(),
        &ClaimNextInvestigationAssetRow {
            stable_request_id: Uuid::new_v4(),
            company_queue_id: frozen.company_queue_id,
            company_member_id: root.company_member_id,
            asset_queue_id: first_root_lane.asset_queue_id,
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            organization_id: fixture.root_organization_id,
            expected_asset_lane_id: first_root_lane.asset_lane_id,
            expected_queue_head_version: 0,
            expected_lane_row_version: 0,
        },
    )
    .await
    .expect("claim first root asset");
    let mut company_queue_head = root_active.company_queue_head_version;
    let mut company_member_row = root_active.row_version;
    let mut generation_ordinals = std::collections::HashMap::<Uuid, i32>::new();
    let expected_dispositions = [
        InvestigationAssetProgressionDispositionRow::NextAsset,
        InvestigationAssetProgressionDispositionRow::NextCompany,
        InvestigationAssetProgressionDispositionRow::NextAsset,
        InvestigationAssetProgressionDispositionRow::InvestigationComplete,
    ];
    let mut first_close_request = None;
    let mut final_close_request = None;
    let mut final_closure_publication_id = None;

    for (index, expected_disposition) in expected_dispositions.into_iter().enumerate() {
        active_lane =
            advance_lane_to_consolidating(&db, &fixture, frozen.company_queue_id, active_lane)
                .await;
        let generation_ordinal = *generation_ordinals
            .entry(active_lane.organization_id)
            .or_insert(0);
        generation_ordinals.insert(active_lane.organization_id, generation_ordinal + 1);
        let resolved_state = ["verified", "refuted", "invalid", "refuted"][index];
        seed_lane_resolved_hypothesis(
            &db,
            &fixture,
            &active_lane,
            generation_ordinal,
            resolved_state,
            true,
        )
        .await;
        let backlog_request = LoadInvestigationAssetBacklogRow {
            company_queue_id: frozen.company_queue_id,
            company_member_id: active_lane.company_member_id,
            asset_queue_id: active_lane.asset_queue_id,
            asset_lane_id: active_lane.asset_lane_id,
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            organization_id: active_lane.organization_id,
        };
        let backlog = load_asset_backlog(db.pool(), &backlog_request)
            .await
            .expect("load server-derived terminal lane census");
        assert_eq!(backlog.backlog_member_count, 0);
        assert_eq!(backlog.pending_evolution_count, 0);
        assert_eq!(backlog.pending_hypothesis_discovery_count, 0);
        assert_eq!(backlog.hypothesis_root_count, 1);
        assert_eq!(backlog.dynamically_resolved_root_count, 1);
        assert_eq!(backlog.revision_count, 2);
        assert_eq!(backlog.fixed_point_wave_count, 0);

        let close = CloseInvestigationAssetBacklogAndAdvanceRow {
            stable_request_id: Uuid::new_v4(),
            company_queue_id: frozen.company_queue_id,
            company_member_id: active_lane.company_member_id,
            asset_queue_id: active_lane.asset_queue_id,
            asset_lane_id: active_lane.asset_lane_id,
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            organization_id: active_lane.organization_id,
            expected_company_queue_head_version: company_queue_head,
            expected_company_member_row_version: company_member_row,
            expected_asset_queue_head_version: active_lane.asset_queue_head_version,
            expected_asset_lane_row_version: active_lane.row_version,
        };
        if index == 0 {
            first_close_request = Some(close.clone());
            let generation_id: Uuid = sqlx::query_scalar(
                "SELECT generation_id FROM hypothesis_generations WHERE asset_lane_id=$1",
            )
            .bind(active_lane.asset_lane_id)
            .fetch_one(db.pool())
            .await
            .expect("load source generation for audit-only pending evolution fixture");
            let pending_id = Uuid::new_v4();
            let mut seed = db
                .pool()
                .begin()
                .await
                .expect("begin pending evolution seed");
            sqlx::query("SET LOCAL session_replication_role='replica'")
                .execute(&mut *seed)
                .await
                .expect("disable seed-only pending evolution authority guards");
            sqlx::query(
                r#"INSERT INTO hypothesis_pending_evolution_authorities(
                       pending_evolution_authority_id,stable_request_id,consolidation_batch_id,
                       operation_id,project_scope_id,organization_id,source_generation_id,
                       source_wave_denominator_id,wave_coverage_receipt_id,
                       fact_delta_member_count,applied_fact_delta_set_hash,residual_set_hash,
                       source_snapshot_hash,asset_lane_id)
                   VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,1,$10,$11,$12,$13)"#,
            )
            .bind(pending_id)
            .bind(Uuid::new_v4())
            .bind(Uuid::new_v4())
            .bind(fixture.operation_id)
            .bind(Uuid::new_v4())
            .bind(active_lane.organization_id)
            .bind(generation_id)
            .bind(Uuid::new_v4())
            .bind(Uuid::new_v4())
            .bind(digest('2'))
            .bind(digest('3'))
            .bind(digest('4'))
            .bind(active_lane.asset_lane_id)
            .execute(&mut *seed)
            .await
            .expect("seed same-lane pending evolution authority");
            seed.commit().await.expect("commit pending evolution seed");
            let audit_backlog = load_asset_backlog(db.pool(), &backlog_request)
                .await
                .expect("pending evolution remains visible as audit census");
            assert_eq!(audit_backlog.pending_evolution_count, 1);
            assert_eq!(audit_backlog.backlog_member_count, 0);
        }
        let progressed = close_asset_backlog_and_advance(db.pool(), &close)
            .await
            .expect("atomically close lane and advance strict nested queues");
        assert_eq!(progressed.disposition, expected_disposition);
        assert!(!progressed.replayed);
        if expected_disposition
            == InvestigationAssetProgressionDispositionRow::InvestigationComplete
        {
            let closure = progressed
                .stage_closure
                .as_ref()
                .expect("final queue drain publishes resolution-only closure");
            assert_eq!(closure.members.len(), frozen.companies.len());
            final_close_request = Some(close.clone());
            final_closure_publication_id = Some(closure.publication_id);
        } else {
            assert!(progressed.stage_closure.is_none());
        }
        let fixed_state: String = sqlx::query_scalar(
            "SELECT state FROM investigation_asset_lanes WHERE asset_lane_id=$1",
        )
        .bind(active_lane.asset_lane_id)
        .fetch_one(db.pool())
        .await
        .expect("load closed lane state");
        assert_eq!(fixed_state, "fixed_point");
        if index == 0 {
            let replay = close_asset_backlog_and_advance(db.pool(), &close)
                .await
                .expect("exact close replay");
            assert_eq!(
                replay.progression_receipt_id,
                progressed.progression_receipt_id
            );
            assert_eq!(replay.disposition, progressed.disposition);
            assert!(replay.replayed);
            let drift = CloseInvestigationAssetBacklogAndAdvanceRow {
                organization_id: fixture.child_organization_id,
                ..close.clone()
            };
            assert!(close_asset_backlog_and_advance(db.pool(), &drift)
                .await
                .expect_err("stable close replay drift is rejected")
                .to_string()
                .contains("INVESTIGATION_ASSET_QUEUE_REPLAY_DRIFT"));
        }
        company_queue_head = progressed.company_queue_head_version;
        if let Some(next_lane) = progressed.next_asset_lane {
            if expected_disposition == InvestigationAssetProgressionDispositionRow::NextCompany {
                company_member_row = 1;
            }
            active_lane = next_lane;
        }
    }
    let queue_state: String = sqlx::query_scalar(
        "SELECT state FROM investigation_company_queues WHERE company_queue_id=$1",
    )
    .bind(frozen.company_queue_id)
    .fetch_one(db.pool())
    .await
    .expect("load fully drained company queue");
    assert_eq!(queue_state, "completed");
    let receipt_counts: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT count(*) FROM investigation_asset_backlog_fixed_point_receipts),
             (SELECT count(*) FROM investigation_asset_backlog_dynamic_resolution_members),
             (SELECT count(*) FROM investigation_asset_progression_receipts),
             (SELECT count(*) FROM investigation_asset_queue_closure_publications),
             (SELECT count(*) FROM investigation_asset_queue_closure_publication_members),
             (SELECT count(*) FROM org_stage_completions
               WHERE stage_kind='investigation' AND stage_run_id=$1)"#,
    )
    .bind(fixture.operation_id.to_string())
    .fetch_one(db.pool())
    .await
    .expect("count exact fixed/progression receipts");
    assert_eq!(receipt_counts, (4, 4, 4, 1, 2, 2));
    let terminalized_runtime: (i64, i64, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT count(*) FROM stage_run_units
               WHERE operation_id=$1 AND stage_execution_id=$2 AND status='passed'
                 AND pass_watermark->>'schema'=
                     'investigation_asset_queue_closure_publication.v1'),
             (SELECT count(*) FROM stage_team_plans
               WHERE operation_id=$1 AND stage_execution_id=$2
                 AND requests_closed_at IS NOT NULL),
             (SELECT count(*)
                FROM investigation_asset_queue_closure_publication_members member
                JOIN stage_run_units unit ON unit.id=member.stage_run_unit_id
                JOIN stage_team_plans plan ON plan.id=member.stage_team_plan_id
               WHERE member.publication_id=$3
                 AND unit.status='passed' AND unit.terminal_at=member.passed_at
                 AND plan.requests_closed_at IS NOT NULL)"#,
    )
    .bind(fixture.operation_id)
    .bind(fixture.stage_execution_id)
    .bind(final_closure_publication_id.expect("final closure publication"))
    .fetch_one(db.pool())
    .await
    .expect("read exact queue-to-runtime terminalization");
    assert_eq!(terminalized_runtime, (2, 2, 2));
    let final_replay = close_asset_backlog_and_advance(
        db.pool(),
        final_close_request
            .as_ref()
            .expect("retain final progression request"),
    )
    .await
    .expect("final closure exact replay");
    assert!(final_replay.replayed);
    assert_eq!(
        final_replay
            .stage_closure
            .expect("replayed resolution closure")
            .publication_id,
        final_closure_publication_id.expect("original resolution closure id")
    );
    let durable_closure = load_resolution_closure_publication(db.pool(), fixture.operation_id)
        .await
        .expect("load server-authored pass-token authority")
        .expect("resolution closure exists");
    assert_eq!(
        durable_closure.publication_id,
        final_closure_publication_id.expect("original resolution closure id")
    );
    assert_eq!(durable_closure.members.len(), frozen.companies.len());
    let late_replay = close_asset_backlog_and_advance(
        db.pool(),
        first_close_request
            .as_ref()
            .expect("retain first progression request"),
    )
    .await
    .expect("late replay preserves original claimed-lane projection");
    assert!(late_replay.replayed);
    assert_eq!(
        late_replay
            .next_asset_lane
            .expect("original next lane projection")
            .state,
        "analyzing"
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn zero_hypothesis_fixed_lane_uses_the_same_atomic_progression_api() {
    let (mut db, _data_dir) = migrated_db().await;
    let fixture = fixture(&db).await;
    let frozen = freeze_company_asset_queue(
        db.pool(),
        &FreezeInvestigationCompanyAssetQueueRow {
            stable_request_id: Uuid::new_v4(),
            authority_id: fixture.authority_id,
            operation_id: fixture.operation_id,
            stage_execution_id: fixture.stage_execution_id,
            owning_stage_run_request_id: "fixture-stage-request".to_string(),
            scope_snapshot_id: fixture.scope_snapshot_id,
            max_evolution_epochs: 0,
        },
    )
    .await
    .expect("freeze zero progression fixture");
    let root = &frozen.companies[0];
    let active_root = claim_next_company(
        db.pool(),
        &ClaimNextInvestigationCompanyRow {
            stable_request_id: Uuid::new_v4(),
            company_queue_id: frozen.company_queue_id,
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            expected_company_member_id: root.company_member_id,
            expected_queue_head_version: 0,
            expected_member_row_version: 0,
        },
    )
    .await
    .expect("claim zero fixture company");
    let first_lane = frozen
        .assets
        .iter()
        .find(|lane| lane.organization_id == fixture.root_organization_id)
        .expect("first zero fixture lane");
    let analyzing = claim_next_asset(
        db.pool(),
        &ClaimNextInvestigationAssetRow {
            stable_request_id: Uuid::new_v4(),
            company_queue_id: frozen.company_queue_id,
            company_member_id: root.company_member_id,
            asset_queue_id: first_lane.asset_queue_id,
            operation_id: fixture.operation_id,
            scope_snapshot_id: fixture.scope_snapshot_id,
            organization_id: fixture.root_organization_id,
            expected_asset_lane_id: first_lane.asset_lane_id,
            expected_queue_head_version: 0,
            expected_lane_row_version: 0,
        },
    )
    .await
    .expect("claim zero fixture lane");
    let next_lane = frozen
        .assets
        .iter()
        .find(|lane| {
            lane.organization_id == fixture.root_organization_id
                && lane.asset_lane_id != analyzing.asset_lane_id
        })
        .expect("next zero fixture lane")
        .clone();

    // Compiler authority is exercised in investigation_hypothesis_compiler;
    // this fixture bypasses only that upstream guard to isolate and prove the
    // shared post-fixed-point progression transaction.
    let zero_receipt_id = Uuid::new_v4();
    let zero_event_id = Uuid::new_v4();
    let zero_stable = Uuid::new_v4();
    let mut seed = db
        .pool()
        .begin()
        .await
        .expect("begin zero progression seed");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *seed)
        .await
        .expect("disable upstream zero compiler guards");
    sqlx::query(
        r#"INSERT INTO investigation_asset_zero_hypothesis_fixed_point_receipts(
               fixed_point_receipt_id,stable_request_id,asset_lane_id,asset_queue_id,
               company_queue_id,company_member_id,operation_id,scope_snapshot_id,
               organization_id,compilation_decision_id,generation_id,generation_seal_id,
               canonical_apply_receipt_id,backlog_set_sha256,obligation_set_sha256,
               residual_set_sha256,receipt_sha256)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)"#,
    )
    .bind(zero_receipt_id)
    .bind(zero_stable)
    .bind(analyzing.asset_lane_id)
    .bind(analyzing.asset_queue_id)
    .bind(frozen.company_queue_id)
    .bind(root.company_member_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.root_organization_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(digest('5'))
    .bind(digest('6'))
    .bind(digest('7'))
    .bind(digest('8'))
    .execute(&mut *seed)
    .await
    .expect("seed upstream zero fixed receipt");
    sqlx::query(
        "UPDATE investigation_asset_lanes SET state='fixed_point',row_version=2,
                latest_event_id=$1 WHERE asset_lane_id=$2",
    )
    .bind(zero_event_id)
    .bind(analyzing.asset_lane_id)
    .execute(&mut *seed)
    .await
    .expect("project upstream zero fixed lane");
    sqlx::query(
        "UPDATE investigation_asset_queues SET head_version=2,latest_event_id=$1
          WHERE asset_queue_id=$2",
    )
    .bind(zero_event_id)
    .bind(analyzing.asset_queue_id)
    .execute(&mut *seed)
    .await
    .expect("project upstream zero fixed queue");
    seed.commit().await.expect("commit zero progression seed");

    let request = CloseInvestigationAssetBacklogAndAdvanceRow {
        stable_request_id: Uuid::new_v4(),
        company_queue_id: frozen.company_queue_id,
        company_member_id: root.company_member_id,
        asset_queue_id: analyzing.asset_queue_id,
        asset_lane_id: analyzing.asset_lane_id,
        operation_id: fixture.operation_id,
        scope_snapshot_id: fixture.scope_snapshot_id,
        organization_id: fixture.root_organization_id,
        expected_company_queue_head_version: active_root.company_queue_head_version,
        expected_company_member_row_version: active_root.row_version,
        expected_asset_queue_head_version: 2,
        expected_asset_lane_row_version: 2,
    };
    let progressed = close_asset_backlog_and_advance(db.pool(), &request)
        .await
        .expect("zero fixed lane uses shared progression transaction");
    assert_eq!(
        progressed.disposition,
        InvestigationAssetProgressionDispositionRow::NextAsset
    );
    assert_eq!(
        progressed
            .next_asset_lane
            .as_ref()
            .expect("next root asset")
            .state,
        "analyzing"
    );
    assert_eq!(
        progressed
            .next_asset_lane
            .as_ref()
            .expect("next root asset")
            .asset_lane_id,
        next_lane.asset_lane_id
    );
    let replay = close_asset_backlog_and_advance(db.pool(), &request)
        .await
        .expect("zero fixed progression exact replay");
    assert!(replay.replayed);
    assert_eq!(
        replay.progression_receipt_id,
        progressed.progression_receipt_id
    );
    db.stop().await;
}
