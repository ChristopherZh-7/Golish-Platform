//! Contract tests for the additive Enumeration endpoint-provenance V2 schema.
//!
//! The data-heavy repository tests use the same migration fixture as the rest
//! of `golish-db`; these source-level assertions deliberately keep the closed
//! authority, immutability and projection rules visible even when a local
//! embedded Postgres is unavailable.

use golish_db::{
    repo::{capability_execution_receipts, enumeration_endpoint_occurrences as enumeration},
    DbConfig, GolishDb,
};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

const MIGRATION: &str =
    include_str!("../migrations/20260802000003_enumeration_endpoint_provenance_v2.sql");
const CLOSURE_GRAPH_SNAPSHOT_MIGRATION: &str =
    include_str!("../migrations/20260802000011_enumeration_lane_closure_graph_snapshot.sql");
const MULTI_OCCURRENCE_RESOLUTION_MIGRATION: &str =
    include_str!("../migrations/20260802000012_enumeration_multi_occurrence_resolution.sql");
const RESOLUTION_ANALYST_AGENT_TYPE_MIGRATION: &str =
    include_str!("../migrations/20260806000001_add_resolution_analyst_agent_type.sql");
const REPOSITORY: &str = include_str!("../src/repo/enumeration_endpoint_occurrences.rs");
const STAGE_PURGE: &str = include_str!("../src/repo/stage_purge.rs");

fn migration_has(parts: &[&str]) {
    for part in parts {
        assert!(MIGRATION.contains(part), "migration is missing `{part}`");
    }
}

fn repository_has(parts: &[&str]) {
    for part in parts {
        assert!(REPOSITORY.contains(part), "repository is missing `{part}`");
    }
}

#[test]
fn same_endpoint_keeps_browser_and_two_js_occurrences() {
    migration_has(&[
        "CREATE TABLE enumeration_endpoint_occurrences",
        "UNIQUE(execution_authority_id,stable_occurrence_request_id)",
        "PRIMARY KEY(occurrence_id,capture_event_id)",
        "observation_kind IN ('runtime_request','html_form','static_ast','ai_analysis')",
    ]);
}

#[test]
fn canonicalizer_links_runtime_and_static_only_on_unique_template_match() {
    repository_has(&[
        "unique_template_matches",
        "runtime_sample_url",
        "project_endpoint_groups",
    ]);
}

#[test]
fn canonicalizer_keeps_ambiguous_template_matches_separate() {
    repository_has(&["HAVING COUNT(*) = 1", "route_match_ambiguous"]);
}

#[test]
fn same_request_keeps_distinct_body_shape_parameters() {
    migration_has(&[
        "CREATE TABLE enumeration_endpoint_occurrence_parameters",
        "UNIQUE(assessment_id,location,name)",
        "'body','form','path'",
    ]);
}

#[test]
fn sealed_script_checked_empty_is_distinct_from_missing_receipt_input() {
    migration_has(&[
        "enumeration_js_analysis_items",
        "terminal_receipt_input_id",
        "sealed_at IS NOT NULL",
    ]);
}

#[test]
fn sealed_candidate_without_terminal_occurrence_blocks_closure() {
    repository_has(&[
        "candidate_without_terminal_occurrence",
        "terminal_receipt_input_id",
    ]);
}

#[test]
fn parameter_checked_empty_is_distinct_from_missing_receipt_input() {
    migration_has(&[
        "parameter_outcome IN ('found','checked_empty','unresolved','not_applicable')",
        "terminal_receipt_input_id UUID NOT NULL",
    ]);
}

#[test]
fn unresolved_occurrence_does_not_create_canonical_endpoint() {
    repository_has(&["AND o.promotion_eligible", "INSERT INTO api_endpoints"]);
}

#[test]
fn v2_projection_reuses_global_endpoint_without_mutating_prior_sealed_payload() {
    let projection = REPOSITORY
        .split("pub async fn project_endpoint_groups")
        .nth(1)
        .expect("project_endpoint_groups body")
        .split("pub async fn count_candidate_without_terminal_occurrence")
        .next()
        .expect("project_endpoint_groups boundary");
    assert!(projection.contains("ON CONFLICT(target_id,url,method) DO NOTHING"));
    assert!(projection.contains("SELECT id FROM api_endpoints"));
    assert!(!projection.contains("ON CONFLICT(target_id,url,method) DO UPDATE"));
}

#[test]
fn closure_graph_snapshot_uses_link_time_and_canonical_timezone() {
    for part in [
        "OR link.created_at<=receipt.created_at",
        "OR occurrence_link.created_at<=receipt.created_at",
        "SET timezone TO 'UTC'",
    ] {
        assert!(
            CLOSURE_GRAPH_SNAPSHOT_MIGRATION.contains(part),
            "closure graph snapshot migration is missing `{part}`"
        );
    }
}

#[test]
fn multi_occurrence_resolution_closes_each_occurrence_before_candidate_barrier() {
    for part in [
        "enumeration_multi_occurrence_lane_validator_source_drift",
        "enumeration_resolution_closeout_receipts closeout",
        "closeout.parent_occurrence_id=sibling.id",
        "producer.execution_authority_id=sibling.execution_authority_id",
    ] {
        assert!(
            MULTI_OCCURRENCE_RESOLUTION_MIGRATION.contains(part),
            "multi-occurrence Resolution migration is missing `{part}`"
        );
    }
}

#[test]
fn resolution_analyst_is_a_database_tracking_role() {
    assert!(RESOLUTION_ANALYST_AGENT_TYPE_MIGRATION
        .contains("ALTER TYPE agent_type ADD VALUE IF NOT EXISTS 'resolution_analyst'"));
}

#[test]
fn scope_excluded_occurrence_cannot_target_foreign_origin() {
    migration_has(&[
        "enumeration_scope_excluded_resolved_target_forbidden",
        "resolved_web_origin_id",
    ]);
}

#[test]
fn cross_origin_source_a_resolved_b_keeps_both_authorities() {
    migration_has(&[
        "source_web_origin_id UUID NOT NULL",
        "resolved_web_origin_id UUID",
    ]);
}

#[test]
fn occurrence_rejects_wrong_operation_org_origin_and_worker() {
    migration_has(&[
        "enumeration_occurrence_authority_mismatch",
        "enumeration_occurrence_source_origin_not_in_frozen_root",
        "enumeration_occurrence_resolved_origin_not_in_frozen_root",
        "enumeration_worker_root_has_exact_origin",
        "tool_truth_worker_fence_mismatch",
    ]);
}

#[test]
fn occurrence_evidence_requires_normalized_tool_truth_authority() {
    migration_has(&[
        "tool_truth_evidence_authority_id",
        "authority_hash",
        "REFERENCES tool_truth_evidence_authorities",
    ]);
}

#[test]
fn occurrence_evidence_rejects_cross_execution_authority() {
    migration_has(&[
        "FOREIGN KEY(tool_truth_evidence_authority_id,execution_authority_id,authority_hash)",
    ]);
}

#[test]
fn retry_reuses_denominator_input_key_while_capture_event_ids_differ() {
    migration_has(&["logical_input_key", "capture_event_id", "duplicate_ordinal"]);
}

#[test]
fn derived_occurrence_rejects_cross_scope_parent() {
    migration_has(&[
        "parent_occurrence_id",
        "enumeration_occurrence_parent_authority_mismatch",
    ]);
}

#[test]
fn shadow_occurrence_never_mutates_canonical_or_manifest() {
    repository_has(&[
        "agent_team_v2_shadow",
        "ENUMERATION_SHADOW_PROJECTION_FORBIDDEN",
    ]);
}

#[test]
fn legacy_contract_rejects_v2_writer() {
    repository_has(&["legacy_v1", "ENUMERATION_V2_WRITER_DISABLED"]);
}

#[test]
fn production_v2_requires_receipt_v1_tool_truth_contract() {
    repository_has(&["agent_team_v2", "receipt_v1"]);
}

#[test]
fn operation_insert_freezes_server_rollout_contract() {
    migration_has(&[
        "enumeration_analysis_rollout",
        "BEFORE INSERT ON operation_state",
        "NEW.enumeration_analysis_contract := deployed",
    ]);
}

#[test]
fn stage_reset_preserves_contract_occurrences_and_authorities() {
    for part in [
        "Immutable Enumeration V2 truth is intentionally absent from stage_purge",
        "enumeration_endpoint_occurrences",
        "tool_truth_business_ref_authorities",
    ] {
        assert!(
            STAGE_PURGE.contains(part),
            "stage purge is missing `{part}`"
        );
    }
}

#[test]
fn occurrence_and_assessment_updates_are_rejected() {
    migration_has(&[
        "enumeration_endpoint_occurrences_immutable",
        "enumeration_endpoint_parameter_assessments_immutable",
    ]);
}

#[test]
fn js_analysis_item_allows_one_terminal_cas_and_rejects_update_delete() {
    migration_has(&[
        "enumeration_guard_js_analysis_item",
        "enumeration_js_analysis_terminal_cas_required",
        "enumeration_js_analysis_item_immutable",
    ]);
}

#[test]
fn graphql_operations_form_distinct_groups() {
    migration_has(&[
        "graphql_operation_name",
        "enumeration_endpoint_groups_identity",
    ]);
}

#[test]
fn websocket_group_never_projects_http_api_endpoint() {
    repository_has(&[
        "protocol IN ('http','https')",
        "websocket_group_not_projectable",
    ]);
}

#[test]
fn legacy_manifest_parameter_locations_remain_readable() {
    migration_has(&[
        "'query','body_or_form','body','form','path','header','graphql_variable','unknown'",
    ]);
}

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read local postgres port")
        .port()
}

fn digest_v1(nibble: char) -> String {
    assert!(nibble.is_ascii_hexdigit() && !nibble.is_ascii_uppercase());
    format!("sha256:{}", nibble.to_string().repeat(64))
}

#[derive(Debug)]
struct EnumerationFixture {
    session_id: Uuid,
    operation_id: Uuid,
    project_path: String,
    organization_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    scope_snapshot_id: Uuid,
    team_plan_id: Uuid,
    root_work_item_id: Uuid,
    worker_run_id: Uuid,
    worker_attempt_epoch: i64,
    lease_token: Uuid,
    source_tool_call_id: Uuid,
    target_id: Uuid,
    web_origin_id: Uuid,
}

async fn seed_enumeration_fixture(pool: &PgPool) -> EnumerationFixture {
    let session_id = Uuid::new_v4();
    let operation_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    let project_path = format!("/tmp/enumeration-v2-{}", Uuid::new_v4().simple());
    let scope_decision_id = Uuid::new_v4();
    let scope_snapshot_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    let stage_run_unit_id = Uuid::new_v4();
    let team_plan_id = Uuid::new_v4();
    let root_work_item_id = Uuid::new_v4();
    let worker_run_id = Uuid::new_v4();
    let worker_attempt_epoch = 0_i64;
    let lease_token = Uuid::new_v4();
    let source_tool_call_id = Uuid::new_v4();
    let target_id = Uuid::new_v4();
    let web_origin_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO sessions(id,title,status,project_path) VALUES($1,'enumeration v2 fixture','running',$2)",
    )
    .bind(session_id)
    .bind(&project_path)
    .execute(pool)
    .await
    .expect("insert fixture session");
    sqlx::query(
        "INSERT INTO tasks(id,session_id,title,input,status) VALUES($1,$2,'enumeration operation','fixture','running')",
    )
    .bind(operation_id)
    .bind(session_id)
    .execute(pool)
    .await
    .expect("insert fixture task");
    sqlx::query(
        "INSERT INTO project_scopes(project_scope_id,canonical_project_path,path_sha256) VALUES($1,$2,$3)",
    )
    .bind(project_scope_id)
    .bind(&project_path)
    .bind(digest_v1('1'))
    .execute(pool)
    .await
    .expect("insert project scope");
    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,project_scope_id
           ) VALUES($1,'red_team','enumeration','legacy_v1',$2)"#,
    )
    .bind(operation_id)
    .bind(project_scope_id)
    .execute(pool)
    .await
    .expect("insert operation");
    sqlx::query(
        "ALTER TABLE operation_state DISABLE TRIGGER operation_state_tool_truth_contract_immutable",
    )
    .execute(pool)
    .await
    .expect("disable Tool Truth immutability in isolated fixture");
    sqlx::query(
        "ALTER TABLE operation_state DISABLE TRIGGER operation_state_enumeration_contract_immutable",
    )
    .execute(pool)
    .await
    .expect("disable Enumeration immutability in isolated fixture");
    sqlx::query(
        "ALTER TABLE operation_state DISABLE TRIGGER operation_state_investigation_contract_immutable",
    )
    .execute(pool)
    .await
    .expect("disable Investigation immutability in isolated fixture");
    sqlx::query(
        "UPDATE operation_state
            SET tool_truth_contract='receipt_v1',
                investigation_contract_version='hypothesis_registry_v1',
                investigation_rollout_mode='dual_read_compare',
                enumeration_analysis_contract='agent_team_v2'
          WHERE operation_id=$1",
    )
    .bind(operation_id)
    .execute(pool)
    .await
    .expect("freeze production contracts in isolated fixture");
    sqlx::query(
        "ALTER TABLE operation_state ENABLE TRIGGER operation_state_investigation_contract_immutable",
    )
    .execute(pool)
    .await
    .expect("restore Investigation immutability");
    sqlx::query(
        "ALTER TABLE operation_state ENABLE TRIGGER operation_state_enumeration_contract_immutable",
    )
    .execute(pool)
    .await
    .expect("restore Enumeration immutability");
    sqlx::query(
        "ALTER TABLE operation_state ENABLE TRIGGER operation_state_tool_truth_contract_immutable",
    )
    .execute(pool)
    .await
    .expect("restore Tool Truth immutability");
    sqlx::query(
        "INSERT INTO organizations(id,project_path,name) VALUES($1,$2,'Enumeration Scoped Org')",
    )
    .bind(organization_id)
    .bind(&project_path)
    .execute(pool)
    .await
    .expect("insert organization");
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status) VALUES($1,$2,'enumeration','started')",
    )
    .bind(stage_execution_id)
    .bind(operation_id)
    .execute(pool)
    .await
    .expect("insert stage execution");
    sqlx::query(
        r#"INSERT INTO operation_scope_decisions(
               id,operation_id,project_scope_id,stage_execution_id,
               root_organization_id,mode,decision_rows,decision_hash
           ) VALUES($1,$2,$3,$4,$5,'cli_flags',$6,$7)"#,
    )
    .bind(scope_decision_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(stage_execution_id)
    .bind(organization_id)
    .bind(serde_json::json!([{"organization_id": organization_id}]))
    .bind(digest_v1('2'))
    .execute(pool)
    .await
    .expect("insert scope decision");
    let mut scope_tx = pool.begin().await.expect("begin scope transaction");
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
    .bind(organization_id)
    .bind(digest_v1('3'))
    .execute(&mut *scope_tx)
    .await
    .expect("insert frozen scope");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units(
               snapshot_id,organization_id,organization_name_at_freeze,
               role,depth,ordinal,decision_row_id,approval_source
           ) VALUES($1,$2,'Enumeration Scoped Org','root',0,0,'root',$3)"#,
    )
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(serde_json::json!({"source":"fixture"}))
    .execute(&mut *scope_tx)
    .await
    .expect("insert frozen scope member");
    sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
        .bind(scope_snapshot_id)
        .execute(&mut *scope_tx)
        .await
        .expect("seal scope");
    scope_tx.commit().await.expect("commit frozen scope");
    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,
               organization_id,stage_kind,generation,specialist,status,started_at
           ) VALUES($1,$2,$3,$4,$5,'enumeration',0,'enumeration_fixture','running',NOW())"#,
    )
    .bind(stage_run_unit_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .execute(pool)
    .await
    .expect("insert stage unit");
    sqlx::query(
        r#"INSERT INTO stage_team_plans(
               id,operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
               organization_id,stage_kind,unit_generation,schema_version,plan_version,
               plan_hash,leader_role,aggregator_kind,aggregator_role,allowed_worker_roles,
               max_workers_total,max_workers_active,dynamic_requests_allowed,
               dynamic_request_policy,dispatch_epoch,final_submitter_kind,
               created_from_stage_spec_hash
           ) VALUES(
               $1,$2,$3,$4,$5,$6,'enumeration',0,1,1,$7,
               'enumeration_fixture','worker','enumeration_fixture',$8,
               16,8,TRUE,$9,0,'worker',$10
           )"#,
    )
    .bind(team_plan_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(digest_v1('a'))
    .bind(serde_json::json!([
        "enumeration_fixture",
        "enumeration_lane_fixture",
        "resolution_analyst"
    ]))
    .bind(serde_json::json!({"max_requests": 8}))
    .bind(digest_v1('b'))
    .execute(pool)
    .await
    .expect("insert Enumeration fixture team plan");
    sqlx::query(
        r#"INSERT INTO stage_work_items(
               id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
               input_manifest_hash,input_refs,required_for_barrier,priority,status,
               attempt_policy,budget,output_schema,created_by,started_at
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,0,'stage_unit','root','enumeration_fixture',
               $8,'[]'::JSONB,FALSE,100,'running','{}'::JSONB,'{}'::JSONB,
               'stage_worker_output.v1','server_seed',NOW()
           )"#,
    )
    .bind(root_work_item_id)
    .bind(team_plan_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(digest_v1('c'))
    .execute(pool)
    .await
    .expect("insert Enumeration fixture root work item");
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,
               organization_id,worker_generation,specialist,work_item_kind,
               work_item_key,agent_path,status,lease_token,lease_owner,
               lease_acquired_at,lease_expires_at,heartbeat_at,attempt_epoch,work_item_id
           ) VALUES(
               $1,$2,$3,$4,$5,0,'enumeration_fixture','stage_unit','root',
               'main>enumeration','running',$6,'enumeration-fixture',NOW(),
               NOW()+INTERVAL '5 minutes',NOW(),$7,$8
           )"#,
    )
    .bind(worker_run_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(organization_id)
    .bind(lease_token)
    .bind(worker_attempt_epoch)
    .bind(root_work_item_id)
    .execute(pool)
    .await
    .expect("insert live worker");
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,status,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token
           ) VALUES(
               $1,'enumeration-source',$2,$3,'primary','enumeration_fixture','{}','running',
               $3,$4,$5,$6,$7,$8,$9
           )"#,
    )
    .bind(source_tool_call_id)
    .bind(session_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(worker_run_id)
    .bind(organization_id)
    .bind(worker_attempt_epoch)
    .bind(lease_token)
    .execute(pool)
    .await
    .expect("insert source tool call");
    sqlx::query(
        "UPDATE stage_worker_runs SET active_tool_call_id=$2,active_tool_started_at=NOW(),updated_at=NOW() WHERE id=$1",
    )
    .bind(worker_run_id)
    .bind(source_tool_call_id)
    .execute(pool)
    .await
    .expect("bind live active tool fence");
    sqlx::query(
        r#"INSERT INTO targets(
               id,name,target_type,value,scope,project_path,organization_id,source,ports
           ) VALUES(
               $1,'app.example.test','domain','app.example.test','in',$2,$3,
               'enumeration_fixture',
               '[{"port":443,"state":"open","service":"https","url":"https://app.example.test:443/"}]'
           )"#,
    )
    .bind(target_id)
    .bind(&project_path)
    .bind(organization_id)
    .execute(pool)
    .await
    .expect("insert scoped target");
    sqlx::query(
        "UPDATE stage_runs SET started_at=statement_timestamp()+INTERVAL '1 second' WHERE id=$1",
    )
    .bind(stage_execution_id)
    .execute(pool)
    .await
    .expect("place target inside stage-unit source cutoff");
    sqlx::query(
        r#"INSERT INTO web_origins(
               id,organization_id,project_path,scheme,host,host_type,port,origin,source,confidence
           ) VALUES($1,$2,$3,'https','app.example.test','domain',443,
                    'https://app.example.test:443','enumeration_fixture',1.0)"#,
    )
    .bind(web_origin_id)
    .bind(organization_id)
    .bind(&project_path)
    .execute(pool)
    .await
    .expect("insert scoped web origin");
    sqlx::query(
        r#"INSERT INTO web_origin_observations(
               id,organization_id,project_path,web_origin_id,target_id,status_code,
               confidence,source,raw
           ) VALUES($1,$2,$3,$4,$5,200,1.0,'enumeration_fixture','{}')"#,
    )
    .bind(Uuid::new_v4())
    .bind(organization_id)
    .bind(&project_path)
    .bind(web_origin_id)
    .bind(target_id)
    .execute(pool)
    .await
    .expect("bind target to web origin");

    EnumerationFixture {
        session_id,
        operation_id,
        project_path,
        organization_id,
        stage_execution_id,
        stage_run_unit_id,
        scope_snapshot_id,
        team_plan_id,
        root_work_item_id,
        worker_run_id,
        worker_attempt_epoch,
        lease_token,
        source_tool_call_id,
        target_id,
        web_origin_id,
    }
}

async fn insert_normalized_evidence(
    pool: &PgPool,
    fixture: &EnumerationFixture,
    authority: &capability_execution_receipts::ToolTruthExecutionAuthorityRef,
    evidence_technique: &str,
) -> capability_execution_receipts::EvidenceAuthorityRef {
    let (worker_run_id, worker_attempt_epoch, lease_token, source_tool_call_id) =
        sqlx::query_as::<_, (Uuid, i64, Uuid, Uuid)>(
            r#"SELECT worker_run_id,worker_attempt_epoch,lease_token,source_tool_call_id
                 FROM tool_truth_execution_authorities WHERE id=$1"#,
        )
        .bind(authority.id)
        .fetch_one(pool)
        .await
        .expect("read exact worker evidence fence");
    let audit_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO audit_log(
               action,category,details,project_path,run_id,audit_role,detail,status,
               session_id,tool_name,evidence_technique,evidence_outcome,source
           ) VALUES(
               'enumeration_fixture','test','normalized discovery',$1,$2,'evidence',$3,
               'completed',$4,'enumeration_fixture',$5,'found','tool'
           ) RETURNING id"#,
    )
    .bind(&fixture.project_path)
    .bind(fixture.operation_id)
    .bind(serde_json::json!({
        "organization_id": fixture.organization_id,
        "tool_truth_producer": {
            "organization_id": fixture.organization_id,
            "stage_execution_id": fixture.stage_execution_id,
            "source_tool_call_id": source_tool_call_id,
            "worker_run_id": worker_run_id,
            "worker_attempt_epoch": worker_attempt_epoch,
            "lease_token": lease_token,
        }
    }))
    .bind(fixture.session_id.to_string())
    .bind(evidence_technique)
    .fetch_one(pool)
    .await
    .expect("insert evidence audit");
    let classification_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO evidence_classifications(
               evidence_audit_id,classification,scope_version,reason,
               classified_by_session,producing_stage_run_id
           ) VALUES($1,'in_scope',1,'fixture',$2,$3) RETURNING id"#,
    )
    .bind(audit_id)
    .bind(fixture.session_id.to_string())
    .bind(fixture.stage_execution_id)
    .fetch_one(pool)
    .await
    .expect("insert evidence classification");
    let binding_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO tool_truth_evidence_production_bindings(
               id,execution_authority_id,operation_id,project_scope_id,
               project_path_at_freeze,scope_snapshot_id,organization_id,
               stage_execution_id,stage_kind,execution_authority_hash,
               evidence_audit_id,evidence_classification_id,production_binding_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,'enumeration',$9,$10,$11,$12)"#,
    )
    .bind(binding_id)
    .bind(authority.id)
    .bind(authority.operation_id)
    .bind(authority.project_scope_id)
    .bind(&authority.project_path_at_freeze)
    .bind(authority.scope_snapshot_id)
    .bind(authority.organization_id)
    .bind(authority.stage_execution_id)
    .bind(&authority.authority_hash)
    .bind(audit_id)
    .bind(classification_id)
    .bind(digest_v1('4'))
    .execute(pool)
    .await
    .expect("bind evidence to worker production authority");
    let evidence_id = Uuid::new_v4();
    let authority_hash: String = sqlx::query_scalar(
        r#"INSERT INTO tool_truth_evidence_authorities(
               id,production_binding_id,execution_authority_id,operation_id,
               project_scope_id,project_path_at_freeze,scope_snapshot_id,
               organization_id,stage_execution_id,stage_kind,execution_authority_hash,
               evidence_audit_id,evidence_classification_id,audit_row_hash,
               classification_row_hash,evidence_chain_hash,authority_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'enumeration',$10,$11,$12,$13,$14,$15,$16)
           RETURNING authority_hash"#,
    )
    .bind(evidence_id)
    .bind(binding_id)
    .bind(authority.id)
    .bind(authority.operation_id)
    .bind(authority.project_scope_id)
    .bind(&authority.project_path_at_freeze)
    .bind(authority.scope_snapshot_id)
    .bind(authority.organization_id)
    .bind(authority.stage_execution_id)
    .bind(&authority.authority_hash)
    .bind(audit_id)
    .bind(classification_id)
    .bind(digest_v1('5'))
    .bind(digest_v1('6'))
    .bind(digest_v1('7'))
    .bind(digest_v1('8'))
    .fetch_one(pool)
    .await
    .expect("seal normalized evidence authority");
    capability_execution_receipts::EvidenceAuthorityRef {
        id: evidence_id,
        authority_hash,
        role: "discovery".to_string(),
    }
}

async fn denominator_item_id(pool: &PgPool, denominator_id: Uuid) -> Uuid {
    sqlx::query_scalar(
        "SELECT id FROM coverage_denominator_items WHERE denominator_id=$1 ORDER BY ordinal LIMIT 1",
    )
    .bind(denominator_id)
    .fetch_one(pool)
    .await
    .expect("read exact denominator item")
}

async fn denominator_item_id_for_technique(
    pool: &PgPool,
    denominator_id: Uuid,
    target_id: Uuid,
    exact_asset: &str,
    technique: &str,
    expected_capability: &str,
) -> Uuid {
    let items = sqlx::query_scalar(
        r#"SELECT id
             FROM coverage_denominator_items
            WHERE denominator_id=$1
              AND target_id=$2
              AND exact_asset=$3
              AND technique=$4
              AND expected_capability=$5
            ORDER BY ordinal"#,
    )
    .bind(denominator_id)
    .bind(target_id)
    .bind(exact_asset)
    .bind(technique)
    .bind(expected_capability)
    .fetch_all(pool)
    .await
    .expect("read exact subject-axis denominator item");
    assert_eq!(items.len(), 1, "root subject-axis must be unique");
    items[0]
}

async fn seal_one_terminal_input(
    pool: &PgPool,
    authority: &capability_execution_receipts::ToolTruthExecutionAuthorityRef,
    denominator_id: Uuid,
    denominator_item_id: Uuid,
    capability: &str,
    outcome: enumeration::EnumerationTerminalInputOutcome,
    evidence: &capability_execution_receipts::EvidenceAuthorityRef,
) -> capability_execution_receipts::CapabilityReceiptInputRef {
    let receipt = capability_execution_receipts::begin(
        pool,
        &capability_execution_receipts::BeginCapabilityReceipt {
            id: Uuid::new_v4(),
            denominator_id,
            capability: capability.to_string(),
            attempt_ordinal: 1,
        },
    )
    .await
    .expect("begin Enumeration receipt");
    let mut inputs = enumeration::seal_enumeration_terminal_receipt_inputs(
        pool,
        authority,
        &enumeration::SealEnumerationTerminalReceiptInputs {
            stable_seal_request_id: Uuid::new_v4(),
            receipt_id: receipt.id,
            inputs: vec![enumeration::EnumerationTerminalReceiptInputWrite {
                denominator_item_id,
                outcome,
                evidence_authorities: vec![evidence.clone()],
            }],
        },
    )
    .await
    .expect("seal exact terminal input census");
    assert_eq!(inputs.len(), 1);
    inputs.remove(0)
}

const TERMINAL_COVERAGE_REDUCER_SQL: &str = r#"
WITH js_expected AS (
    SELECT denominator.id AS denominator_id,item.id AS item_id
      FROM coverage_denominators denominator
      JOIN coverage_denominator_items item ON item.denominator_id=denominator.id
     WHERE denominator.execution_authority_id=$1
       AND denominator.sealed_at IS NOT NULL
       AND item.expected_capability='enumeration.javascript'
       AND enumeration_denominator_has_worker_root(denominator.id,$1)
), js_terminal AS (
    SELECT DISTINCT expected.item_id
      FROM js_expected expected
      JOIN enumeration_js_analysis_items descriptor
        ON descriptor.denominator_id=expected.denominator_id
       AND descriptor.denominator_item_id=expected.item_id
       AND descriptor.execution_authority_id=$1
      JOIN capability_execution_receipt_inputs input
        ON input.id=descriptor.terminal_receipt_input_id
       AND input.receipt_id=descriptor.terminal_receipt_id
       AND input.denominator_item_id=expected.item_id
       AND input.execution_authority_id=$1
       AND input.sealed_at IS NOT NULL
      JOIN enumeration_receipt_input_census_seals census
        ON census.receipt_id=input.receipt_id
       AND census.denominator_id=expected.denominator_id
       AND census.execution_authority_id=$1
), candidate_expected AS (
    SELECT denominator.id AS denominator_id,item.id AS item_id
      FROM coverage_denominators denominator
      JOIN coverage_denominator_items item ON item.denominator_id=denominator.id
      JOIN js_expected parent
        ON parent.denominator_id=denominator.parent_denominator_id
       AND parent.item_id=denominator.parent_denominator_item_id
     WHERE denominator.execution_authority_id=$1
       AND denominator.sealed_at IS NOT NULL
       AND item.expected_capability='enumeration.candidate'
), candidate_terminal AS (
    SELECT DISTINCT expected.item_id
      FROM candidate_expected expected
      JOIN enumeration_endpoint_candidate_inputs candidate
        ON candidate.denominator_id=expected.denominator_id
       AND candidate.denominator_item_id=expected.item_id
       AND candidate.execution_authority_id=$1
      JOIN capability_execution_receipt_inputs input
        ON input.id=candidate.terminal_receipt_input_id
       AND input.receipt_id=candidate.terminal_receipt_id
       AND input.sealed_at IS NOT NULL
      JOIN enumeration_receipt_input_census_seals census
        ON census.receipt_id=input.receipt_id
       AND census.denominator_id=expected.denominator_id
       AND census.execution_authority_id=$1
      JOIN enumeration_endpoint_candidate_closure_receipts closure
        ON closure.candidate_input_id=candidate.id
       AND closure.execution_authority_id=$1
      JOIN enumeration_endpoint_candidate_denominator_closure_receipts denominator_closure
        ON denominator_closure.denominator_id=expected.denominator_id
       AND denominator_closure.execution_authority_id=$1
), parameter_expected AS (
    SELECT denominator.id AS denominator_id,item.id AS item_id
      FROM coverage_denominators denominator
      JOIN coverage_denominator_items item ON item.denominator_id=denominator.id
      JOIN candidate_expected parent
        ON parent.denominator_id=denominator.parent_denominator_id
       AND parent.item_id=denominator.parent_denominator_item_id
     WHERE denominator.execution_authority_id=$1
       AND denominator.sealed_at IS NOT NULL
       AND item.expected_capability='enumeration.parameter'
), parameter_terminal AS (
    SELECT DISTINCT expected.item_id
      FROM parameter_expected expected
      JOIN enumeration_endpoint_parameter_assessments assessment
        ON assessment.denominator_id=expected.denominator_id
       AND assessment.denominator_item_id=expected.item_id
       AND assessment.execution_authority_id=$1
      JOIN capability_execution_receipt_inputs input
        ON input.id=assessment.terminal_receipt_input_id
       AND input.receipt_id=assessment.terminal_receipt_id
       AND input.sealed_at IS NOT NULL
      JOIN enumeration_receipt_input_census_seals census
        ON census.receipt_id=input.receipt_id
       AND census.denominator_id=expected.denominator_id
       AND census.execution_authority_id=$1
      JOIN enumeration_endpoint_occurrence_evidence evidence
        ON evidence.parameter_assessment_id=assessment.id
       AND evidence.parameter_assessment_execution_authority_id=$1
       AND evidence.evidence_execution_authority_id=$1
       AND evidence.evidence_role='parameter'
     WHERE assessment.parameter_outcome<>'found' OR EXISTS (
         SELECT 1 FROM enumeration_endpoint_occurrence_parameters parameter
          WHERE parameter.assessment_id=assessment.id
     )
), missing AS (
    SELECT item_id FROM js_expected EXCEPT SELECT item_id FROM js_terminal
    UNION ALL
    SELECT item_id FROM candidate_expected EXCEPT SELECT item_id FROM candidate_terminal
    UNION ALL
    SELECT item_id FROM parameter_expected EXCEPT SELECT item_id FROM parameter_terminal
)
SELECT (SELECT COUNT(*) FROM js_expected)::BIGINT,
       (SELECT COUNT(*) FROM js_terminal)::BIGINT,
       (SELECT COUNT(*) FROM candidate_expected)::BIGINT,
       (SELECT COUNT(*) FROM candidate_terminal)::BIGINT,
       (SELECT COUNT(*) FROM parameter_expected)::BIGINT,
       (SELECT COUNT(*) FROM parameter_terminal)::BIGINT,
       (SELECT COUNT(*) FROM missing)::BIGINT
"#;

async fn mint_worker_authority_root(
    pool: &PgPool,
    fixture: &EnumerationFixture,
    source_root_denominator_id: Uuid,
) -> enumeration::EnumerationWorkerAuthorityRoot {
    let work_item_id = Uuid::new_v4();
    let worker_run_id = Uuid::new_v4();
    let lease_token = Uuid::new_v4();
    let source_tool_call_id = Uuid::new_v4();
    let work_key = format!("lane:{}", worker_run_id.simple());
    sqlx::query(
        r#"INSERT INTO stage_work_items(
               id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
               input_manifest_hash,input_refs,required_for_barrier,priority,status,
               attempt_policy,budget,output_schema,created_by,started_at
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,0,'formulaic_enumeration',$8,
               'enumeration_lane_fixture',$9,'[]'::JSONB,FALSE,10,'running',
               '{}'::JSONB,'{}'::JSONB,'stage_worker_output.v1','server_seed',NOW()
           )"#,
    )
    .bind(work_item_id)
    .bind(fixture.team_plan_id)
    .bind(fixture.operation_id)
    .bind(fixture.stage_execution_id)
    .bind(fixture.stage_run_unit_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.organization_id)
    .bind(&work_key)
    .bind(digest_v1('d'))
    .execute(pool)
    .await
    .expect("insert independent Enumeration lane work item");
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,
               organization_id,worker_generation,specialist,work_item_kind,
               work_item_key,agent_path,status,lease_token,lease_owner,
               lease_acquired_at,lease_expires_at,heartbeat_at,attempt_epoch,work_item_id
           ) VALUES(
               $1,$2,$3,$4,$5,0,'enumeration_lane_fixture','formulaic_enumeration',$6,$7,
               'running',$8,'enumeration-lane-fixture',NOW(),NOW()+INTERVAL '5 minutes',NOW(),0,$9
           )"#,
    )
    .bind(worker_run_id)
    .bind(fixture.operation_id)
    .bind(fixture.stage_execution_id)
    .bind(fixture.stage_run_unit_id)
    .bind(fixture.organization_id)
    .bind(&work_key)
    .bind(format!("main>enumeration>{work_key}"))
    .bind(lease_token)
    .bind(work_item_id)
    .execute(pool)
    .await
    .expect("insert independent Enumeration lane worker");
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,status,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token
           ) VALUES(
               $1,$2,$3,$4,'primary','enumeration_lane_fixture','{}','running',
               $4,$5,$6,$7,$8,0,$9
           )"#,
    )
    .bind(source_tool_call_id)
    .bind(format!("enumeration-lane-{}", source_tool_call_id.simple()))
    .bind(fixture.session_id)
    .bind(fixture.operation_id)
    .bind(fixture.stage_execution_id)
    .bind(fixture.stage_run_unit_id)
    .bind(worker_run_id)
    .bind(fixture.organization_id)
    .bind(lease_token)
    .execute(pool)
    .await
    .expect("insert independent Enumeration lane tool call");
    sqlx::query(
        "UPDATE stage_worker_runs SET active_tool_call_id=$2,active_tool_started_at=NOW(),updated_at=NOW() WHERE id=$1",
    )
    .bind(worker_run_id)
    .bind(source_tool_call_id)
    .execute(pool)
    .await
    .expect("bind independent Enumeration lane tool fence");
    enumeration::seal_enumeration_worker_authority_root(
        pool,
        &enumeration::SealEnumerationWorkerAuthorityRoot {
            stable_authority_request_id: Uuid::new_v4(),
            stable_root_request_id: Uuid::new_v4(),
            source_root_denominator_id,
            worker_fence: enumeration::EnumerationWorkerFence {
                worker_run_id,
                worker_attempt_epoch: 0,
                lease_token,
                source_tool_call_id,
            },
        },
    )
    .await
    .expect("mint independent Enumeration lane authority")
}

async fn mint_resolution_worker_authority_root(
    pool: &PgPool,
    fixture: &EnumerationFixture,
    source_root_denominator_id: Uuid,
    unresolved_occurrence_id: Uuid,
) -> (
    enumeration::EnumerationWorkerAuthorityRoot,
    Uuid,
    enumeration::EnumerationWorkerFence,
) {
    let work_item_id = Uuid::new_v4();
    let worker_run_id = Uuid::new_v4();
    let lease_token = Uuid::new_v4();
    let source_tool_call_id = Uuid::new_v4();
    let stable_key = format!("resolution:{unresolved_occurrence_id}");
    let output_schema = "enumeration_resolution_suggestion.v1";
    let mut assignment_tx = pool.begin().await.expect("begin Resolution assignment tx");
    sqlx::query(
        r#"INSERT INTO stage_work_items(
               id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
               input_manifest_hash,input_refs,required_for_barrier,priority,status,
               attempt_policy,budget,output_schema,created_by
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,0,'enumeration_resolution',$8,
               'resolution_analyst',$9,$10,FALSE,20,'queued','{}'::JSONB,
               '{"max_wrapper_calls":1}'::JSONB,$11,'accepted_worker_request'
           )"#,
    )
    .bind(work_item_id)
    .bind(fixture.team_plan_id)
    .bind(fixture.operation_id)
    .bind(fixture.stage_execution_id)
    .bind(fixture.stage_run_unit_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.organization_id)
    .bind(&stable_key)
    .bind(digest_v1('e'))
    .bind(serde_json::json!([{
        "kind": "enumeration_occurrence",
        "id": unresolved_occurrence_id,
    }]))
    .bind(output_schema)
    .execute(&mut *assignment_tx)
    .await
    .expect("insert Resolution work item");
    sqlx::query(
        r#"INSERT INTO stage_worker_requests(
               id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,parent_work_item_id,parent_worker_run_id,
               dispatch_epoch,requested_role,request_kind,bounded_subject_refs,reason_code,
               expected_output_schema,budget_hint,dedupe_key,request_payload_hash,status,
               accepted_work_item_id
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,0,'resolution_analyst',
               'enumeration_resolution',$10,$11,$12,'{"max_wrapper_calls":1}'::JSONB,
               $13,$14,'accepted',$15
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(fixture.team_plan_id)
    .bind(fixture.operation_id)
    .bind(fixture.stage_execution_id)
    .bind(fixture.stage_run_unit_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.organization_id)
    .bind(fixture.root_work_item_id)
    .bind(fixture.worker_run_id)
    .bind(serde_json::json!([{
        "kind": "enumeration_occurrence",
        "id": unresolved_occurrence_id,
    }]))
    .bind(
        serde_json::json!({
            "objective": {"unresolved_cluster_id": unresolved_occurrence_id}
        })
        .to_string(),
    )
    .bind(output_schema)
    .bind(format!("resolution:{unresolved_occurrence_id}"))
    .bind(digest_v1('f'))
    .bind(work_item_id)
    .execute(&mut *assignment_tx)
    .await
    .expect("insert accepted Resolution worker request");
    assignment_tx
        .commit()
        .await
        .expect("commit exact Resolution assignment pair");

    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,
               organization_id,worker_generation,specialist,work_item_kind,
               work_item_key,agent_path,status,lease_token,lease_owner,
               lease_acquired_at,lease_expires_at,heartbeat_at,attempt_epoch,work_item_id
           ) VALUES(
               $1,$2,$3,$4,$5,1,'resolution_analyst','enumeration_resolution',$6,$7,
               'running',$8,'enumeration-resolution-fixture',NOW(),
               NOW()+INTERVAL '5 minutes',NOW(),0,$9
           )"#,
    )
    .bind(worker_run_id)
    .bind(fixture.operation_id)
    .bind(fixture.stage_execution_id)
    .bind(fixture.stage_run_unit_id)
    .bind(fixture.organization_id)
    .bind(&stable_key)
    .bind(format!("main>enumeration>{stable_key}"))
    .bind(lease_token)
    .bind(work_item_id)
    .execute(pool)
    .await
    .expect("insert Resolution worker");
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,status,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token
           ) VALUES(
               $1,$2,$3,$4,'primary','resolve_js_api_cluster',$5,'running',
               $4,$6,$7,$8,$9,0,$10
           )"#,
    )
    .bind(source_tool_call_id)
    .bind(format!(
        "enumeration-resolution-{}",
        source_tool_call_id.simple()
    ))
    .bind(fixture.session_id)
    .bind(fixture.operation_id)
    .bind(serde_json::json!({"unresolved_cluster_id": unresolved_occurrence_id}))
    .bind(fixture.stage_execution_id)
    .bind(fixture.stage_run_unit_id)
    .bind(worker_run_id)
    .bind(fixture.organization_id)
    .bind(lease_token)
    .execute(pool)
    .await
    .expect("insert Resolution tool call");
    sqlx::query(
        "UPDATE stage_worker_runs SET active_tool_call_id=$2,active_tool_started_at=NOW(),updated_at=NOW() WHERE id=$1",
    )
    .bind(worker_run_id)
    .bind(source_tool_call_id)
    .execute(pool)
    .await
    .expect("bind Resolution active tool fence");
    let fence = enumeration::EnumerationWorkerFence {
        worker_run_id,
        worker_attempt_epoch: 0,
        lease_token,
        source_tool_call_id,
    };
    let root = enumeration::seal_enumeration_worker_authority_root(
        pool,
        &enumeration::SealEnumerationWorkerAuthorityRoot {
            stable_authority_request_id: Uuid::new_v4(),
            stable_root_request_id: Uuid::new_v4(),
            source_root_denominator_id,
            worker_fence: fence.clone(),
        },
    )
    .await
    .expect("mint Resolution worker authority");
    (root, work_item_id, fence)
}

async fn evidence_audit_id(
    pool: &PgPool,
    evidence: &capability_execution_receipts::EvidenceAuthorityRef,
) -> i64 {
    sqlx::query_scalar("SELECT evidence_audit_id FROM tool_truth_evidence_authorities WHERE id=$1")
        .bind(evidence.id)
        .fetch_one(pool)
        .await
        .expect("read normalized evidence audit id")
}

async fn seal_checked_empty_browser_lane(
    pool: &PgPool,
    fixture: &EnumerationFixture,
    source_root_denominator_id: Uuid,
) -> enumeration::EnumerationLaneCommitReceiptRow {
    let root = mint_worker_authority_root(pool, fixture, source_root_denominator_id).await;
    let evidence =
        insert_normalized_evidence(pool, fixture, &root.authority, "GOLISH-ENUM-JS").await;
    let root_item = denominator_item_id_for_technique(
        pool,
        root.root_denominator.id,
        fixture.target_id,
        "https://app.example.test:443",
        "GOLISH-ENUM-JS",
        "enum.collect_browser_surface",
    )
    .await;
    let script_denominator = enumeration::seal_enumeration_derived_denominator(
        pool,
        &root.authority,
        &enumeration::SealEnumerationDerivedDenominator {
            stable_seal_request_id: Uuid::new_v4(),
            parent_denominator_id: root.root_denominator.id,
            parent_denominator_item_id: root_item,
            derived_ordinal: 1,
            items: vec![],
        },
    )
    .await
    .expect("seal checked-empty Browser script denominator");
    let candidate_denominator = enumeration::seal_enumeration_derived_denominator(
        pool,
        &root.authority,
        &enumeration::SealEnumerationDerivedDenominator {
            stable_seal_request_id: Uuid::new_v4(),
            parent_denominator_id: root.root_denominator.id,
            parent_denominator_item_id: root_item,
            derived_ordinal: 2,
            items: vec![],
        },
    )
    .await
    .expect("seal checked-empty Browser candidate denominator");
    enumeration::seal_enumeration_candidate_denominator_closure(
        pool,
        &root.authority,
        &enumeration::SealEnumerationCandidateDenominatorClosure {
            stable_closure_request_id: Uuid::new_v4(),
            denominator_id: candidate_denominator.id,
        },
    )
    .await
    .expect("seal checked-empty Browser candidate closure");
    let audit_id = evidence_audit_id(pool, &evidence).await;
    let mut tx = pool.begin().await.expect("begin Browser lane receipt tx");
    let (receipt, replayed) = enumeration::seal_enumeration_lane_commit_receipt(
        &mut tx,
        &root.authority,
        &enumeration::SealEnumerationLaneCommitReceipt {
            stable_commit_request_id: Uuid::new_v4(),
            lane: "browser".to_string(),
            target_id: fixture.target_id,
            exact_origin: "https://app.example.test:443".to_string(),
            artifact_sha256: digest_v1('b'),
            dependency_receipt_ids: vec![],
            evidence_audit_ids: vec![audit_id],
            script_denominator_id: Some(script_denominator.id),
            candidate_denominator_ids: vec![candidate_denominator.id],
            parameter_denominator_ids: vec![],
            resolution_occurrence_id: None,
            resolution_terminal_receipt_id: None,
            resolution_terminal_receipt_input_id: None,
        },
    )
    .await
    .expect("seal checked-empty Browser lane receipt");
    assert!(!replayed);
    tx.commit().await.expect("commit Browser lane receipt");
    assert_eq!(receipt.lane, "browser");
    assert_eq!(receipt.terminal_disposition, "checked_empty");
    assert!(receipt.closure_graph_sha256.starts_with("sha256:"));
    receipt
}

#[tokio::test]
#[serial]
async fn red_team_entity_chain_reaches_non_vacuous_terminal_closure() {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let mut db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("enumeration_v2_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start isolated migrated postgres");
    let fixture = seed_enumeration_fixture(db.pool()).await;
    // Browser/runtime discovery can create the legacy endpoint before the
    // typed JS lane exists. Parameter projection links that old endpoint only
    // after JS closes; this later association must not rewrite the immutable
    // JS closure graph snapshot.
    sqlx::query(
        r#"INSERT INTO api_endpoints(
               target_id,project_path,url,method,path,source,risk_level)
           VALUES($1,$2,'https://app.example.test:443/api/items','POST',
                  '/api/items','browser_route_probe','unknown')"#,
    )
    .bind(fixture.target_id)
    .bind(&fixture.project_path)
    .execute(db.pool())
    .await
    .expect("seed pre-JS legacy endpoint");
    let host_root = capability_execution_receipts::seal_source_denominator(
        db.pool(),
        &capability_execution_receipts::SealSourceDenominator {
            stable_seal_request_id: Uuid::new_v4(),
            stage_execution_id: fixture.stage_execution_id,
            source: capability_execution_receipts::DenominatorSourceRef::StageTeamUnit(
                fixture.stage_run_unit_id,
            ),
        },
        |_stage, assets| {
            Ok(assets
                .iter()
                .flat_map(|asset| {
                    [
                        ("GOLISH-ENUM-JSAPI", "enum.extract_js_apis"),
                        ("GOLISH-ENUM-DIR", "enum.preflight_web_origins"),
                        ("GOLISH-ENUM-JS", "enum.collect_browser_surface"),
                        ("GOLISH-ENUM-PARAM", "enum.collect_browser_surface"),
                    ]
                    .into_iter()
                    .map(|(technique, expected_capability)| {
                        capability_execution_receipts::CompiledDenominatorItem {
                            input_key: format!(
                                "{}:{}:{technique}",
                                asset.target_id, asset.exact_asset
                            ),
                            target_id: asset.target_id,
                            exact_asset: asset.exact_asset.clone(),
                            technique: technique.to_string(),
                            expected_capability: expected_capability.to_string(),
                        }
                    })
                })
                .collect())
        },
    )
    .await
    .expect("seal host StageTeamUnit source root");
    assert_eq!(host_root.member_count, Some(4));

    let rejected = enumeration::seal_enumeration_worker_authority_root(
        db.pool(),
        &enumeration::SealEnumerationWorkerAuthorityRoot {
            stable_authority_request_id: Uuid::new_v4(),
            stable_root_request_id: Uuid::new_v4(),
            source_root_denominator_id: host_root.id,
            worker_fence: enumeration::EnumerationWorkerFence {
                worker_run_id: fixture.worker_run_id,
                worker_attempt_epoch: fixture.worker_attempt_epoch,
                lease_token: Uuid::new_v4(),
                source_tool_call_id: fixture.source_tool_call_id,
            },
        },
    )
    .await
    .expect_err("wrong lease must not mint an Enumeration worker authority");
    assert!(rejected
        .to_string()
        .contains("ENUMERATION_AUTHORITY_MISMATCH"));

    let root = enumeration::seal_enumeration_worker_authority_root(
        db.pool(),
        &enumeration::SealEnumerationWorkerAuthorityRoot {
            stable_authority_request_id: Uuid::new_v4(),
            stable_root_request_id: Uuid::new_v4(),
            source_root_denominator_id: host_root.id,
            worker_fence: enumeration::EnumerationWorkerFence {
                worker_run_id: fixture.worker_run_id,
                worker_attempt_epoch: fixture.worker_attempt_epoch,
                lease_token: fixture.lease_token,
                source_tool_call_id: fixture.source_tool_call_id,
            },
        },
    )
    .await
    .expect("mint worker authority from exact live tool fence");
    assert_ne!(root.authority.id, host_root.execution_authority_id);
    assert_eq!(root.root_denominator.member_count, Some(4));

    let host_authority = sqlx::query_as::<_, (Uuid, Uuid, Uuid, String, Uuid, Uuid, Uuid, String)>(
        r#"SELECT id,operation_id,project_scope_id,project_path_at_freeze,
                  scope_snapshot_id,organization_id,stage_execution_id,authority_hash
             FROM tool_truth_execution_authorities WHERE id=$1"#,
    )
    .bind(host_root.execution_authority_id)
    .fetch_one(db.pool())
    .await
    .expect("read host source authority");
    let host_authority = capability_execution_receipts::ToolTruthExecutionAuthorityRef {
        id: host_authority.0,
        operation_id: host_authority.1,
        project_scope_id: host_authority.2,
        project_path_at_freeze: host_authority.3,
        scope_snapshot_id: host_authority.4,
        organization_id: host_authority.5,
        stage_execution_id: host_authority.6,
        authority_hash: host_authority.7,
    };
    let host_parent_item = denominator_item_id(db.pool(), host_root.id).await;
    let host_child_error = enumeration::seal_enumeration_derived_denominator(
        db.pool(),
        &host_authority,
        &enumeration::SealEnumerationDerivedDenominator {
            stable_seal_request_id: Uuid::new_v4(),
            parent_denominator_id: host_root.id,
            parent_denominator_item_id: host_parent_item,
            derived_ordinal: 1,
            items: vec![enumeration::EnumerationDerivedDenominatorItemWrite {
                input_key: "forged-host-child".to_string(),
                target_id: fixture.target_id,
                exact_asset: "https://app.example.test/app.js".to_string(),
                technique: "analyze_script".to_string(),
                expected_capability: "enumeration.javascript".to_string(),
            }],
        },
    )
    .await
    .expect_err("host authority cannot bypass worker-root ancestry");
    assert!(host_child_error
        .to_string()
        .contains("ENUMERATION_AUTHORITY_MISMATCH"));

    let browser_receipt = seal_checked_empty_browser_lane(db.pool(), &fixture, host_root.id).await;

    let evidence =
        insert_normalized_evidence(db.pool(), &fixture, &root.authority, "GOLISH-ENUM-JSAPI").await;
    let root_item = denominator_item_id_for_technique(
        db.pool(),
        root.root_denominator.id,
        fixture.target_id,
        "https://app.example.test:443",
        "GOLISH-ENUM-JSAPI",
        "enum.extract_js_apis",
    )
    .await;
    let js_denominator = enumeration::seal_enumeration_derived_denominator(
        db.pool(),
        &root.authority,
        &enumeration::SealEnumerationDerivedDenominator {
            stable_seal_request_id: Uuid::new_v4(),
            parent_denominator_id: root.root_denominator.id,
            parent_denominator_item_id: root_item,
            derived_ordinal: 1,
            items: vec![enumeration::EnumerationDerivedDenominatorItemWrite {
                input_key: "script:https://app.example.test/app.js".to_string(),
                target_id: fixture.target_id,
                exact_asset: "https://app.example.test/app.js".to_string(),
                technique: "analyze_script".to_string(),
                expected_capability: "enumeration.javascript".to_string(),
            }],
        },
    )
    .await
    .expect("seal JS denominator");
    let js_item = denominator_item_id(db.pool(), js_denominator.id).await;
    let unbound_js_input = capability_execution_receipts::CapabilityReceiptInputRef {
        receipt_id: Uuid::nil(),
        receipt_input_id: Uuid::nil(),
        denominator_id: js_denominator.id,
        denominator_item_id: js_item,
        logical_input_key: "script:https://app.example.test/app.js".to_string(),
    };
    let js_descriptor = enumeration::JsAnalysisDescriptorWrite {
        id: Uuid::new_v4(),
        stable_descriptor_request_id: Uuid::new_v4(),
        manifest_url: "https://app.example.test/app.js".to_string(),
        page_url: "https://app.example.test/".to_string(),
        document_url: Some("https://app.example.test/".to_string()),
        chunk_ordinal: 0,
        source_map_url: None,
        script_sha256: Some(digest_v1('9')),
        descriptor_metadata: serde_json::json!({"capture_kind":"browser"}),
    };
    let mut js_tx = db.pool().begin().await.expect("begin JS descriptor tx");
    enumeration::persist_js_analysis_descriptor(
        &mut js_tx,
        &root.authority,
        &unbound_js_input,
        &js_descriptor,
    )
    .await
    .expect("persist unbound JS descriptor");
    js_tx.commit().await.expect("commit JS descriptor");
    let js_input = seal_one_terminal_input(
        db.pool(),
        &root.authority,
        js_denominator.id,
        js_item,
        "enumeration.javascript",
        enumeration::EnumerationTerminalInputOutcome::Found,
        &evidence,
    )
    .await;
    let mut js_bind_tx = db.pool().begin().await.expect("begin JS terminal bind");
    enumeration::bind_js_analysis_terminal_receipt(
        &mut js_bind_tx,
        &root.authority,
        js_descriptor.id,
        &js_input,
    )
    .await
    .expect("bind JS terminal receipt");
    js_bind_tx.commit().await.expect("commit JS terminal bind");

    let candidate_denominator = enumeration::seal_enumeration_derived_denominator(
        db.pool(),
        &root.authority,
        &enumeration::SealEnumerationDerivedDenominator {
            stable_seal_request_id: Uuid::new_v4(),
            parent_denominator_id: js_denominator.id,
            parent_denominator_item_id: js_item,
            derived_ordinal: 2,
            items: vec![enumeration::EnumerationDerivedDenominatorItemWrite {
                input_key: "candidate:app.js:fetch:1".to_string(),
                target_id: fixture.target_id,
                exact_asset: "https://app.example.test/api/items".to_string(),
                technique: "extract_endpoint_candidate".to_string(),
                expected_capability: "enumeration.candidate".to_string(),
            }],
        },
    )
    .await
    .expect("seal candidate denominator");
    let candidate_item = denominator_item_id(db.pool(), candidate_denominator.id).await;
    let candidate_input = seal_one_terminal_input(
        db.pool(),
        &root.authority,
        candidate_denominator.id,
        candidate_item,
        "enumeration.candidate",
        enumeration::EnumerationTerminalInputOutcome::Found,
        &evidence,
    )
    .await;
    let capture_event_id = Uuid::new_v4();
    let candidate_id = Uuid::new_v4();
    let candidate = enumeration::CandidateDescriptorWrite {
        id: candidate_id,
        stable_candidate_request_id: Uuid::new_v4(),
        js_analysis_item_id: Some(js_descriptor.id),
        source_anchor: "app.js:1:1".to_string(),
        callsite_fingerprint: digest_v1('a'),
        capture_event_id,
        capture_attempt_ordinal: 1,
        captured_at: chrono::Utc::now(),
        event_fingerprint: digest_v1('b'),
        duplicate_ordinal: 0,
        resolution_input: "/api/items/${userId}".to_string(),
    };
    let mut candidate_tx = db.pool().begin().await.expect("begin candidate tx");
    enumeration::persist_candidate_descriptor(
        &mut candidate_tx,
        &root.authority,
        &candidate_input,
        &candidate,
    )
    .await
    .expect("persist candidate descriptor");
    candidate_tx
        .commit()
        .await
        .expect("commit candidate descriptor");
    let mut candidate_replay_tx = db.pool().begin().await.expect("begin candidate replay tx");
    let replayed_candidate_id = enumeration::persist_candidate_descriptor(
        &mut candidate_replay_tx,
        &root.authority,
        &candidate_input,
        &candidate,
    )
    .await
    .expect("replay exact candidate descriptor after response loss");
    assert_eq!(replayed_candidate_id, candidate_id);
    candidate_replay_tx
        .commit()
        .await
        .expect("commit candidate replay tx");
    let capture_event_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM enumeration_endpoint_candidate_capture_events
            WHERE capture_event_id=$1 AND candidate_input_id=$2 AND execution_authority_id=$3"#,
    )
    .bind(capture_event_id)
    .bind(candidate_id)
    .bind(root.authority.id)
    .fetch_one(db.pool())
    .await
    .expect("count exact immutable candidate capture event");
    assert_eq!(capture_event_count, 1);

    let occurrence_id = Uuid::new_v4();
    let occurrence = enumeration::EndpointOccurrenceWrite {
        id: occurrence_id,
        stable_occurrence_request_id: Uuid::new_v4(),
        candidate_input_id: candidate_id,
        capture_event_id,
        source_target_id: fixture.target_id,
        source_web_origin_id: fixture.web_origin_id,
        resolved_target_id: Some(fixture.target_id),
        resolved_web_origin_id: Some(fixture.web_origin_id),
        parent_occurrence_id: None,
        source_url: "https://app.example.test/".to_string(),
        document_url: Some("https://app.example.test/".to_string()),
        script_url: Some("https://app.example.test/app.js".to_string()),
        script_sha256: Some(digest_v1('9')),
        source_span: serde_json::json!({"start_line":1,"start_column":0}),
        initiator_url: None,
        initiator_status: "not_applicable".to_string(),
        initiator_line: None,
        initiator_column: None,
        cdp_request_id_hash: None,
        protocol: "https".to_string(),
        method: "POST".to_string(),
        graphql_operation_name: None,
        websocket_subprotocol: None,
        raw_expression: Some("/api/items/${userId}".to_string()),
        receiver_kind: Some("fetch".to_string()),
        observation_kind: "static_ast".to_string(),
        inference_level: "deterministic".to_string(),
        resolution_status: "resolved".to_string(),
        scope_decision: "in_scope".to_string(),
        candidate_classification: "endpoint".to_string(),
        canonical_request_url: Some(
            "https://app.example.test:443/api/items/$%7BuserId%7D".to_string(),
        ),
        display_url: Some("https://app.example.test/api/items/${userId}".to_string()),
        resolution_reason: "static_base_resolution".to_string(),
        resolution_base_facts: serde_json::json!({}),
        resolution_candidates: serde_json::json!([]),
        resolution_chain: serde_json::json!([]),
        route_kind: "template".to_string(),
        route_template: Some("https://app.example.test:443/api/items/$%7BuserId%7D".to_string()),
        request_sent: false,
        request_schema: serde_json::json!({
            "body":{"fields":[{"name":"user_id","type":"string"}]}
        }),
        redaction_metadata: serde_json::json!({
            "redacted":true,"field_count":1,"policy_version":"v1"
        }),
        request_body_length: None,
        runtime_sample_url: None,
        observed_at: chrono::Utc::now(),
    };
    let discovery_evidence = capability_execution_receipts::EvidenceAuthorityRef {
        role: "discovery".to_string(),
        ..evidence.clone()
    };
    let resolution_evidence = capability_execution_receipts::EvidenceAuthorityRef {
        role: "resolution".to_string(),
        ..evidence.clone()
    };

    // A target/origin that becomes current and in-scope after the immutable
    // root was sealed must not gain cross-origin authority retroactively.
    let late_target_id = Uuid::new_v4();
    let late_web_origin_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO targets(
               id,name,target_type,value,scope,project_path,organization_id,source,ports
           ) VALUES(
               $1,'late.example.test','domain','late.example.test','in',$2,$3,
               'enumeration_fixture',
               '[{"port":443,"state":"open","service":"https","url":"https://late.example.test:443/"}]'
           )"#,
    )
    .bind(late_target_id)
    .bind(&fixture.project_path)
    .bind(fixture.organization_id)
    .execute(db.pool())
    .await
    .expect("insert post-root in-scope target");
    sqlx::query(
        r#"INSERT INTO web_origins(
               id,organization_id,project_path,scheme,host,host_type,port,origin,
               source,confidence,last_confirmed_at
           ) VALUES(
               $1,$2,$3,'https','late.example.test','domain',443,
               'https://late.example.test:443','httpx',1.0,NOW()
           )"#,
    )
    .bind(late_web_origin_id)
    .bind(fixture.organization_id)
    .bind(&fixture.project_path)
    .execute(db.pool())
    .await
    .expect("insert post-root exact origin");
    sqlx::query(
        r#"INSERT INTO web_origin_observations(
               id,organization_id,project_path,web_origin_id,target_id,status_code,
               confidence,source,raw
           ) VALUES($1,$2,$3,$4,$5,200,1.0,'httpx','{}')"#,
    )
    .bind(Uuid::new_v4())
    .bind(fixture.organization_id)
    .bind(&fixture.project_path)
    .bind(late_web_origin_id)
    .bind(late_target_id)
    .execute(db.pool())
    .await
    .expect("bind post-root target and origin");
    let mut late_cross_origin = occurrence.clone();
    late_cross_origin.id = Uuid::new_v4();
    late_cross_origin.stable_occurrence_request_id = Uuid::new_v4();
    late_cross_origin.resolved_target_id = Some(late_target_id);
    late_cross_origin.resolved_web_origin_id = Some(late_web_origin_id);
    late_cross_origin.canonical_request_url =
        Some("https://late.example.test:443/api/items".to_string());
    late_cross_origin.display_url = Some("https://late.example.test/api/items".to_string());
    let mut late_tx = db
        .pool()
        .begin()
        .await
        .expect("begin late-origin rejection tx");
    let late_error = enumeration::persist_endpoint_occurrence(
        &mut late_tx,
        &root.authority,
        &candidate_input,
        &late_cross_origin,
        &[discovery_evidence.clone(), resolution_evidence.clone()],
    )
    .await
    .expect_err("current scope must not widen the frozen cross-origin root");
    assert!(late_error
        .to_string()
        .contains("enumeration_occurrence_resolved_origin_not_in_frozen_root"));
    late_tx
        .rollback()
        .await
        .expect("rollback rejected late-origin occurrence");

    let mut occurrence_tx = db.pool().begin().await.expect("begin occurrence tx");
    let persisted = enumeration::persist_endpoint_occurrence(
        &mut occurrence_tx,
        &root.authority,
        &candidate_input,
        &occurrence,
        &[discovery_evidence.clone(), resolution_evidence.clone()],
    )
    .await
    .expect("persist immutable occurrence");
    occurrence_tx
        .commit()
        .await
        .expect("commit occurrence and evidence");
    assert!(persisted.promotion_eligible);

    let unresolved_candidate_denominator = enumeration::seal_enumeration_derived_denominator(
        db.pool(),
        &root.authority,
        &enumeration::SealEnumerationDerivedDenominator {
            stable_seal_request_id: Uuid::new_v4(),
            parent_denominator_id: js_denominator.id,
            parent_denominator_item_id: js_item,
            derived_ordinal: 3,
            items: vec![enumeration::EnumerationDerivedDenominatorItemWrite {
                input_key: "candidate:app.js:fetch:dynamic:1".to_string(),
                target_id: fixture.target_id,
                exact_asset: "https://app.example.test/dynamic-api".to_string(),
                technique: "extract_endpoint_candidate".to_string(),
                expected_capability: "enumeration.candidate".to_string(),
            }],
        },
    )
    .await
    .expect("seal unresolved candidate denominator");
    let unresolved_candidate_item =
        denominator_item_id(db.pool(), unresolved_candidate_denominator.id).await;
    let unresolved_candidate_input = seal_one_terminal_input(
        db.pool(),
        &root.authority,
        unresolved_candidate_denominator.id,
        unresolved_candidate_item,
        "enumeration.candidate",
        enumeration::EnumerationTerminalInputOutcome::Found,
        &evidence,
    )
    .await;
    let unresolved_capture_event_id = Uuid::new_v4();
    let unresolved_candidate_id = Uuid::new_v4();
    let unresolved_candidate = enumeration::CandidateDescriptorWrite {
        id: unresolved_candidate_id,
        stable_candidate_request_id: Uuid::new_v4(),
        js_analysis_item_id: Some(js_descriptor.id),
        source_anchor: "app.js:4:7".to_string(),
        callsite_fingerprint: digest_v1('c'),
        capture_event_id: unresolved_capture_event_id,
        capture_attempt_ordinal: 1,
        captured_at: chrono::Utc::now(),
        event_fingerprint: digest_v1('d'),
        duplicate_ordinal: 0,
        resolution_input: "apiBase + '/v2/data'".to_string(),
    };
    let mut unresolved_candidate_tx = db
        .pool()
        .begin()
        .await
        .expect("begin unresolved candidate tx");
    enumeration::persist_candidate_descriptor(
        &mut unresolved_candidate_tx,
        &root.authority,
        &unresolved_candidate_input,
        &unresolved_candidate,
    )
    .await
    .expect("persist unresolved candidate descriptor");
    unresolved_candidate_tx
        .commit()
        .await
        .expect("commit unresolved candidate descriptor");
    let unresolved_occurrence_id = Uuid::new_v4();
    let unresolved_occurrence = enumeration::EndpointOccurrenceWrite {
        id: unresolved_occurrence_id,
        stable_occurrence_request_id: Uuid::new_v4(),
        candidate_input_id: unresolved_candidate_id,
        capture_event_id: unresolved_capture_event_id,
        source_target_id: fixture.target_id,
        source_web_origin_id: fixture.web_origin_id,
        resolved_target_id: None,
        resolved_web_origin_id: None,
        parent_occurrence_id: None,
        source_url: "https://app.example.test/".to_string(),
        document_url: Some("https://app.example.test/".to_string()),
        script_url: Some("https://app.example.test/app.js".to_string()),
        script_sha256: Some(digest_v1('9')),
        source_span: serde_json::json!({"start_line":4,"start_column":7}),
        initiator_url: None,
        initiator_status: "not_applicable".to_string(),
        initiator_line: None,
        initiator_column: None,
        cdp_request_id_hash: None,
        protocol: "https".to_string(),
        method: "GET".to_string(),
        graphql_operation_name: None,
        websocket_subprotocol: None,
        raw_expression: Some("apiBase + '/v2/data'".to_string()),
        receiver_kind: Some("fetch".to_string()),
        observation_kind: "static_ast".to_string(),
        inference_level: "deterministic".to_string(),
        resolution_status: "unresolved".to_string(),
        scope_decision: "in_scope".to_string(),
        candidate_classification: "endpoint".to_string(),
        canonical_request_url: None,
        display_url: None,
        resolution_reason: "dynamic_base_unresolved".to_string(),
        resolution_base_facts: serde_json::json!({}),
        resolution_candidates: serde_json::json!([]),
        resolution_chain: serde_json::json!([]),
        route_kind: "dynamic_unresolved".to_string(),
        route_template: None,
        request_sent: false,
        request_schema: serde_json::json!({}),
        redaction_metadata: serde_json::json!({
            "redacted":true,"field_count":0,"policy_version":"v1"
        }),
        request_body_length: None,
        runtime_sample_url: None,
        observed_at: chrono::Utc::now(),
    };
    let mut unresolved_occurrence_tx = db
        .pool()
        .begin()
        .await
        .expect("begin unresolved occurrence tx");
    let unresolved_persisted = enumeration::persist_endpoint_occurrence(
        &mut unresolved_occurrence_tx,
        &root.authority,
        &unresolved_candidate_input,
        &unresolved_occurrence,
        &[discovery_evidence, resolution_evidence],
    )
    .await
    .expect("persist bounded unresolved occurrence");
    unresolved_occurrence_tx
        .commit()
        .await
        .expect("commit bounded unresolved occurrence");
    assert!(!unresolved_persisted.promotion_eligible);

    let candidate_closure = enumeration::seal_enumeration_candidate_closure(
        db.pool(),
        &root.authority,
        &enumeration::SealEnumerationCandidateClosure {
            stable_closure_request_id: Uuid::new_v4(),
            candidate_input_id: candidate_id,
            resolution_terminal_input: None,
        },
    )
    .await
    .expect("seal candidate occurrence closure");
    assert_eq!(candidate_closure.terminal_disposition, "resolved");
    assert_eq!(candidate_closure.occurrence_count, 1);
    let denominator_closure = enumeration::seal_enumeration_candidate_denominator_closure(
        db.pool(),
        &root.authority,
        &enumeration::SealEnumerationCandidateDenominatorClosure {
            stable_closure_request_id: Uuid::new_v4(),
            denominator_id: candidate_denominator.id,
        },
    )
    .await
    .expect("seal exact candidate denominator closure");
    assert_eq!(denominator_closure.member_count, 1);
    let js_evidence_audit_id = evidence_audit_id(db.pool(), &evidence).await;
    let mut js_lane_tx = db
        .pool()
        .begin()
        .await
        .expect("begin JS/API lane receipt tx");
    let (js_api_receipt, js_replayed) = enumeration::seal_enumeration_lane_commit_receipt(
        &mut js_lane_tx,
        &root.authority,
        &enumeration::SealEnumerationLaneCommitReceipt {
            stable_commit_request_id: Uuid::new_v4(),
            lane: "js_api".to_string(),
            target_id: fixture.target_id,
            exact_origin: "https://app.example.test:443".to_string(),
            artifact_sha256: digest_v1('c'),
            dependency_receipt_ids: vec![browser_receipt.id],
            evidence_audit_ids: vec![js_evidence_audit_id],
            script_denominator_id: Some(js_denominator.id),
            candidate_denominator_ids: {
                let mut ids = vec![
                    candidate_denominator.id,
                    unresolved_candidate_denominator.id,
                ];
                ids.sort_unstable();
                ids
            },
            parameter_denominator_ids: vec![],
            resolution_occurrence_id: None,
            resolution_terminal_receipt_id: None,
            resolution_terminal_receipt_input_id: None,
        },
    )
    .await
    .expect("seal JS/API lane receipt");
    assert!(!js_replayed);
    js_lane_tx
        .commit()
        .await
        .expect("commit JS/API lane receipt");

    let parameter_root = mint_worker_authority_root(db.pool(), &fixture, host_root.id).await;
    let parameter_root_item = denominator_item_id_for_technique(
        db.pool(),
        parameter_root.root_denominator.id,
        fixture.target_id,
        "https://app.example.test:443",
        "GOLISH-ENUM-PARAM",
        "enum.collect_browser_surface",
    )
    .await;
    let parameter_lane_evidence = insert_normalized_evidence(
        db.pool(),
        &fixture,
        &parameter_root.authority,
        "GOLISH-ENUM-PARAM",
    )
    .await;
    let parameter_denominator = enumeration::seal_enumeration_derived_denominator(
        db.pool(),
        &parameter_root.authority,
        &enumeration::SealEnumerationDerivedDenominator {
            stable_seal_request_id: Uuid::new_v4(),
            parent_denominator_id: parameter_root.root_denominator.id,
            parent_denominator_item_id: parameter_root_item,
            derived_ordinal: 1,
            items: vec![enumeration::EnumerationDerivedDenominatorItemWrite {
                input_key: "parameter:app.js:fetch:1".to_string(),
                target_id: fixture.target_id,
                exact_asset: "https://app.example.test/api/items".to_string(),
                technique: "reduce_parameter_facts".to_string(),
                expected_capability: "enumeration.parameter".to_string(),
            }],
        },
    )
    .await
    .expect("seal parameter denominator");
    let parameter_item = denominator_item_id(db.pool(), parameter_denominator.id).await;
    let parameter_evidence = capability_execution_receipts::EvidenceAuthorityRef {
        role: "parameter".to_string(),
        ..parameter_lane_evidence.clone()
    };
    let parameter_input = seal_one_terminal_input(
        db.pool(),
        &parameter_root.authority,
        parameter_denominator.id,
        parameter_item,
        "enumeration.parameter",
        enumeration::EnumerationTerminalInputOutcome::Found,
        &parameter_evidence,
    )
    .await;
    let assessment = enumeration::ParameterAssessmentWrite {
        id: Uuid::new_v4(),
        occurrence_id,
        outcome: "found".to_string(),
        reason_code: "static_request_shape".to_string(),
        parameters: vec![enumeration::OccurrenceParameterWrite {
            id: Uuid::new_v4(),
            name: "user_id".to_string(),
            location: "body".to_string(),
            value_type: "string".to_string(),
            requirement: "required".to_string(),
            confidence: 1.0,
            source_anchor_ids: vec!["app.js:1:1".to_string()],
        }],
    };
    let mut missing_evidence_tx = db
        .pool()
        .begin()
        .await
        .expect("begin rejected assessment tx");
    enumeration::persist_parameter_assessment(
        &mut missing_evidence_tx,
        &parameter_root.authority,
        &parameter_input,
        &assessment,
    )
    .await
    .expect("assessment is provisionally written before deferred evidence check");
    let missing_evidence_error = missing_evidence_tx
        .commit()
        .await
        .expect_err("assessment without normalized parameter evidence must fail closed");
    assert!(missing_evidence_error
        .to_string()
        .contains("enumeration_parameter_assessment_terminal_shape_invalid"));

    let mut assessment_tx = db.pool().begin().await.expect("begin assessment tx");
    enumeration::persist_parameter_assessment(
        &mut assessment_tx,
        &parameter_root.authority,
        &parameter_input,
        &assessment,
    )
    .await
    .expect("persist terminal parameter assessment");
    enumeration::bind_parameter_assessment_evidence(
        &mut assessment_tx,
        &parameter_root.authority,
        assessment.id,
        std::slice::from_ref(&parameter_evidence),
    )
    .await
    .expect("bind normalized parameter evidence");
    assessment_tx
        .commit()
        .await
        .expect("commit assessment, facts and evidence atomically");

    let unresolved_parameter_denominator = enumeration::seal_enumeration_derived_denominator(
        db.pool(),
        &parameter_root.authority,
        &enumeration::SealEnumerationDerivedDenominator {
            stable_seal_request_id: Uuid::new_v4(),
            parent_denominator_id: parameter_root.root_denominator.id,
            parent_denominator_item_id: parameter_root_item,
            derived_ordinal: 2,
            items: vec![enumeration::EnumerationDerivedDenominatorItemWrite {
                input_key: "parameter:app.js:fetch:dynamic:1".to_string(),
                target_id: fixture.target_id,
                exact_asset: "https://app.example.test/dynamic-api".to_string(),
                technique: "reduce_parameter_facts".to_string(),
                expected_capability: "enumeration.parameter".to_string(),
            }],
        },
    )
    .await
    .expect("seal unresolved occurrence parameter denominator");
    let unresolved_parameter_item =
        denominator_item_id(db.pool(), unresolved_parameter_denominator.id).await;
    let unresolved_parameter_input = seal_one_terminal_input(
        db.pool(),
        &parameter_root.authority,
        unresolved_parameter_denominator.id,
        unresolved_parameter_item,
        "enumeration.parameter",
        enumeration::EnumerationTerminalInputOutcome::CheckedEmpty,
        &parameter_lane_evidence,
    )
    .await;
    let unresolved_assessment = enumeration::ParameterAssessmentWrite {
        id: Uuid::new_v4(),
        occurrence_id: unresolved_occurrence_id,
        outcome: "checked_empty".to_string(),
        reason_code: "no_static_parameter_shape".to_string(),
        parameters: vec![],
    };
    let mut unresolved_assessment_tx = db
        .pool()
        .begin()
        .await
        .expect("begin unresolved occurrence assessment tx");
    enumeration::persist_parameter_assessment(
        &mut unresolved_assessment_tx,
        &parameter_root.authority,
        &unresolved_parameter_input,
        &unresolved_assessment,
    )
    .await
    .expect("persist unresolved occurrence terminal parameter assessment");
    enumeration::bind_parameter_assessment_evidence(
        &mut unresolved_assessment_tx,
        &parameter_root.authority,
        unresolved_assessment.id,
        &[parameter_evidence],
    )
    .await
    .expect("bind unresolved occurrence parameter evidence");
    unresolved_assessment_tx
        .commit()
        .await
        .expect("commit unresolved occurrence parameter assessment");

    let mut projection_tx = db.pool().begin().await.expect("begin projection tx");
    let projection = enumeration::project_endpoint_groups(
        &mut projection_tx,
        &parameter_root.authority,
        browser_receipt.id,
        js_api_receipt.id,
    )
    .await
    .expect("project endpoint groups");
    projection_tx.commit().await.expect("commit projection");
    let operation_manifest =
        golish_db::repo::enumeration_surface_manifest::list_endpoints_for_operation_target_origin(
            db.pool(),
            fixture.operation_id,
            fixture.target_id,
            "https://app.example.test:443",
        )
        .await
        .expect("read operation-owned endpoint planning manifest");
    assert_eq!(operation_manifest.len(), 1);
    assert_eq!(
        operation_manifest[0].url,
        "https://app.example.test:443/api/items/$%7BuserId%7D"
    );

    assert_eq!(projection.groups_created, 1);
    assert_eq!(projection.api_links_created, 1);
    let projected_parameter: (String, String, bool) = sqlx::query_as(
        r#"SELECT parameter.name,parameter.location,parameter.required
             FROM enumeration_endpoint_parameters parameter
             JOIN enumeration_endpoint_observations observation
               ON observation.id=parameter.endpoint_observation_id
            WHERE observation.operation_id=$1"#,
    )
    .bind(fixture.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("read canonical parameter projection");
    assert_eq!(
        projected_parameter,
        ("user_id".to_string(), "body".to_string(), true)
    );

    let mut parameter_dependencies = vec![browser_receipt.id, js_api_receipt.id];
    parameter_dependencies.sort_unstable();
    let parameter_audit_id = evidence_audit_id(db.pool(), &parameter_lane_evidence).await;
    let mut parameter_lane_tx = db
        .pool()
        .begin()
        .await
        .expect("begin Parameter lane receipt tx");
    let (parameter_receipt, parameter_replayed) =
        enumeration::seal_enumeration_lane_commit_receipt(
            &mut parameter_lane_tx,
            &parameter_root.authority,
            &enumeration::SealEnumerationLaneCommitReceipt {
                stable_commit_request_id: Uuid::new_v4(),
                lane: "parameter".to_string(),
                target_id: fixture.target_id,
                exact_origin: "https://app.example.test:443".to_string(),
                artifact_sha256: digest_v1('d'),
                dependency_receipt_ids: parameter_dependencies,
                evidence_audit_ids: vec![parameter_audit_id],
                script_denominator_id: None,
                candidate_denominator_ids: vec![],
                parameter_denominator_ids: {
                    let mut ids = vec![
                        parameter_denominator.id,
                        unresolved_parameter_denominator.id,
                    ];
                    ids.sort_unstable();
                    ids
                },
                resolution_occurrence_id: None,
                resolution_terminal_receipt_id: None,
                resolution_terminal_receipt_input_id: None,
            },
        )
        .await
        .expect("seal Parameter lane receipt");
    assert!(!parameter_replayed);
    parameter_lane_tx
        .commit()
        .await
        .expect("commit Parameter lane receipt");

    let (resolution_root, resolution_work_item_id, resolution_fence) =
        mint_resolution_worker_authority_root(
            db.pool(),
            &fixture,
            host_root.id,
            unresolved_occurrence_id,
        )
        .await;
    let resolution_root_item = denominator_item_id_for_technique(
        db.pool(),
        resolution_root.root_denominator.id,
        fixture.target_id,
        "https://app.example.test:443",
        "GOLISH-ENUM-JSAPI",
        "enum.extract_js_apis",
    )
    .await;
    let resolution_lane_evidence = insert_normalized_evidence(
        db.pool(),
        &fixture,
        &resolution_root.authority,
        "GOLISH-ENUM-JSAPI",
    )
    .await;
    let resolution_suggestion_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO enumeration_js_resolution_suggestions(
               id,operation_id,organization_id,stage_execution_id,stage_run_unit_id,
               worker_run_id,source_tool_call_id,worker_attempt_epoch,lease_token,
               assigned_work_item_id,assigned_cluster_id,parent_occurrence_id,
               candidate_input_id,disposition,capture_anchor_id,parameter_names,reason_code
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$11,$12,
               'unresolved','app.js:4:7','[]'::JSONB,'bounded_ai_could_not_resolve_dynamic_base'
           )"#,
    )
    .bind(resolution_suggestion_id)
    .bind(fixture.operation_id)
    .bind(fixture.organization_id)
    .bind(fixture.stage_execution_id)
    .bind(fixture.stage_run_unit_id)
    .bind(resolution_fence.worker_run_id)
    .bind(resolution_fence.source_tool_call_id)
    .bind(resolution_fence.worker_attempt_epoch)
    .bind(resolution_fence.lease_token)
    .bind(resolution_work_item_id)
    .bind(unresolved_occurrence_id)
    .bind(unresolved_candidate_id)
    .execute(db.pool())
    .await
    .expect("persist bounded AI Resolution suggestion");
    let resolution_denominator = enumeration::seal_enumeration_derived_denominator(
        db.pool(),
        &resolution_root.authority,
        &enumeration::SealEnumerationDerivedDenominator {
            stable_seal_request_id: Uuid::new_v4(),
            parent_denominator_id: resolution_root.root_denominator.id,
            parent_denominator_item_id: resolution_root_item,
            derived_ordinal: 1,
            items: vec![enumeration::EnumerationDerivedDenominatorItemWrite {
                input_key: format!("resolution:{unresolved_occurrence_id}"),
                target_id: fixture.target_id,
                exact_asset: "https://app.example.test:443".to_string(),
                technique: "resolve_unresolved_occurrence".to_string(),
                expected_capability: "enumeration.js_api".to_string(),
            }],
        },
    )
    .await
    .expect("seal one-member Resolution denominator");
    let resolution_item = denominator_item_id(db.pool(), resolution_denominator.id).await;
    let resolution_input = seal_one_terminal_input(
        db.pool(),
        &resolution_root.authority,
        resolution_denominator.id,
        resolution_item,
        "enumeration.js_api",
        enumeration::EnumerationTerminalInputOutcome::UnresolvedExhausted {
            coverage_gap_reason: "unsupported".to_string(),
        },
        &resolution_lane_evidence,
    )
    .await;
    let resolution_closeout_command = enumeration::SealEnumerationResolutionCloseout {
        stable_closeout_request_id: Uuid::new_v4(),
        assigned_work_item_id: resolution_work_item_id,
        worker_fence: resolution_fence,
        parent_occurrence_id: unresolved_occurrence_id,
        producer_lane_receipt_id: js_api_receipt.id,
        terminal_state: "advisory_residual".to_string(),
        reason_code: "bounded_ai_dynamic_base_unresolved".to_string(),
        suggestion_ids: vec![resolution_suggestion_id],
        terminal_receipt_id: resolution_input.receipt_id,
        terminal_receipt_input_id: resolution_input.receipt_input_id,
    };
    let mut resolution_closeout_tx = db
        .pool()
        .begin()
        .await
        .expect("begin Resolution closeout tx");
    let (resolution_closeout, resolution_closeout_replayed) =
        enumeration::seal_enumeration_resolution_closeout(
            &mut resolution_closeout_tx,
            &resolution_root.authority,
            &resolution_closeout_command,
        )
        .await
        .expect("seal typed Resolution closeout");
    resolution_closeout_tx
        .commit()
        .await
        .expect("commit typed Resolution closeout");
    assert!(!resolution_closeout_replayed);
    assert_eq!(
        resolution_closeout.suggestion_ids,
        vec![resolution_suggestion_id]
    );
    let mut resolution_closeout_replay_tx = db
        .pool()
        .begin()
        .await
        .expect("begin Resolution closeout response-loss replay tx");
    let (replayed_resolution_closeout, resolution_closeout_was_replayed) =
        enumeration::seal_enumeration_resolution_closeout(
            &mut resolution_closeout_replay_tx,
            &resolution_root.authority,
            &resolution_closeout_command,
        )
        .await
        .expect("replay typed Resolution closeout after response loss");
    resolution_closeout_replay_tx
        .commit()
        .await
        .expect("commit Resolution closeout response-loss replay");
    assert!(resolution_closeout_was_replayed);
    assert_eq!(replayed_resolution_closeout, resolution_closeout);
    let unresolved_candidate_closure = enumeration::seal_enumeration_candidate_closure(
        db.pool(),
        &root.authority,
        &enumeration::SealEnumerationCandidateClosure {
            stable_closure_request_id: Uuid::new_v4(),
            candidate_input_id: unresolved_candidate_id,
            resolution_terminal_input: Some(resolution_input.clone()),
        },
    )
    .await
    .expect("seal producer-owned unresolved candidate closeout");
    assert_eq!(
        unresolved_candidate_closure.terminal_disposition,
        "unresolved_exhausted"
    );
    let unresolved_denominator_closure =
        enumeration::seal_enumeration_candidate_denominator_closure(
            db.pool(),
            &root.authority,
            &enumeration::SealEnumerationCandidateDenominatorClosure {
                stable_closure_request_id: Uuid::new_v4(),
                denominator_id: unresolved_candidate_denominator.id,
            },
        )
        .await
        .expect("seal unresolved candidate denominator exact closeout");
    assert_eq!(unresolved_denominator_closure.member_count, 1);
    let resolution_audit_id = evidence_audit_id(db.pool(), &resolution_lane_evidence).await;
    let resolution_lane_command = enumeration::SealEnumerationLaneCommitReceipt {
        stable_commit_request_id: Uuid::new_v4(),
        lane: "resolution".to_string(),
        target_id: fixture.target_id,
        exact_origin: "https://app.example.test:443".to_string(),
        artifact_sha256: digest_v1('e'),
        dependency_receipt_ids: vec![js_api_receipt.id],
        evidence_audit_ids: vec![resolution_audit_id],
        script_denominator_id: None,
        candidate_denominator_ids: vec![],
        parameter_denominator_ids: vec![],
        resolution_occurrence_id: Some(unresolved_occurrence_id),
        resolution_terminal_receipt_id: Some(resolution_input.receipt_id),
        resolution_terminal_receipt_input_id: Some(resolution_input.receipt_input_id),
    };
    let mut resolution_lane_tx = db
        .pool()
        .begin()
        .await
        .expect("begin Resolution lane receipt tx");
    let (resolution_receipt, resolution_replayed) =
        enumeration::seal_enumeration_lane_commit_receipt(
            &mut resolution_lane_tx,
            &resolution_root.authority,
            &resolution_lane_command,
        )
        .await
        .expect("seal Resolution lane receipt");
    assert!(!resolution_replayed);
    assert_eq!(resolution_receipt.occurrence_count, 1);
    assert_eq!(resolution_receipt.unresolved_count, 1);
    assert_eq!(
        resolution_receipt.terminal_disposition,
        "terminal_with_residual"
    );
    resolution_lane_tx
        .commit()
        .await
        .expect("commit Resolution lane receipt");
    let mut resolution_lane_replay_tx = db
        .pool()
        .begin()
        .await
        .expect("begin Resolution lane response-loss replay tx");
    let (replayed_resolution_receipt, resolution_lane_was_replayed) =
        enumeration::seal_enumeration_lane_commit_receipt(
            &mut resolution_lane_replay_tx,
            &resolution_root.authority,
            &resolution_lane_command,
        )
        .await
        .expect("replay Resolution lane receipt after response loss");
    resolution_lane_replay_tx
        .commit()
        .await
        .expect("commit Resolution lane response-loss replay");
    assert!(resolution_lane_was_replayed);
    assert_eq!(replayed_resolution_receipt, resolution_receipt);

    let coverage_root = mint_worker_authority_root(db.pool(), &fixture, host_root.id).await;
    let coverage_evidence = insert_normalized_evidence(
        db.pool(),
        &fixture,
        &coverage_root.authority,
        "GOLISH-ENUM-JSAPI",
    )
    .await;
    let coverage_audit_id = evidence_audit_id(db.pool(), &coverage_evidence).await;
    let mut coverage_dependencies = vec![
        browser_receipt.id,
        js_api_receipt.id,
        parameter_receipt.id,
        resolution_receipt.id,
    ];
    coverage_dependencies.sort_unstable();
    let coverage_command = enumeration::SealEnumerationLaneCommitReceipt {
        stable_commit_request_id: Uuid::new_v4(),
        lane: "coverage".to_string(),
        target_id: fixture.target_id,
        exact_origin: "https://app.example.test:443".to_string(),
        artifact_sha256: digest_v1('f'),
        dependency_receipt_ids: coverage_dependencies,
        evidence_audit_ids: vec![coverage_audit_id],
        script_denominator_id: None,
        candidate_denominator_ids: vec![],
        parameter_denominator_ids: vec![],
        resolution_occurrence_id: None,
        resolution_terminal_receipt_id: None,
        resolution_terminal_receipt_input_id: None,
    };
    let mut coverage_tx = db
        .pool()
        .begin()
        .await
        .expect("begin Coverage lane receipt tx");
    let (coverage_receipt, coverage_replayed) = enumeration::seal_enumeration_lane_commit_receipt(
        &mut coverage_tx,
        &coverage_root.authority,
        &coverage_command,
    )
    .await
    .expect("seal Coverage lane receipt");
    assert!(!coverage_replayed);
    coverage_tx
        .commit()
        .await
        .expect("commit Coverage lane receipt");
    let mut coverage_replay_tx = db
        .pool()
        .begin()
        .await
        .expect("begin Coverage response-loss replay tx");
    let (replayed_coverage_receipt, coverage_was_replayed) =
        enumeration::seal_enumeration_lane_commit_receipt(
            &mut coverage_replay_tx,
            &coverage_root.authority,
            &coverage_command,
        )
        .await
        .expect("replay Coverage receipt after response loss");
    coverage_replay_tx
        .commit()
        .await
        .expect("commit Coverage response-loss replay");
    assert!(coverage_was_replayed);
    assert_eq!(replayed_coverage_receipt, coverage_receipt);
    assert_eq!(coverage_receipt.missing, 0);
    assert_eq!(coverage_receipt.occurrence_count, 2);
    assert_eq!(coverage_receipt.parameter_assessment_count, 2);
    assert_eq!(coverage_receipt.parameter_fact_count, 1);
    assert_eq!(coverage_receipt.unresolved_count, 1);
    assert_eq!(coverage_receipt.group_count, 1);
    assert_eq!(coverage_receipt.api_link_count, 1);
    assert_eq!(
        coverage_receipt.terminal_disposition,
        "terminal_with_residual"
    );
    let late_resolution_suggestion = sqlx::query(
        r#"INSERT INTO enumeration_js_resolution_suggestions(
               id,operation_id,organization_id,stage_execution_id,stage_run_unit_id,
               worker_run_id,source_tool_call_id,worker_attempt_epoch,lease_token,
               assigned_work_item_id,assigned_cluster_id,parent_occurrence_id,
               candidate_input_id,disposition,artifact_id,artifact_sha256,
               source_start_byte,source_end_byte,capture_anchor_id,suggested_url,
               method,parameter_names,reason_code
           )
           SELECT $1,operation_id,organization_id,stage_execution_id,stage_run_unit_id,
                  worker_run_id,source_tool_call_id,worker_attempt_epoch,lease_token,
                  assigned_work_item_id,assigned_cluster_id,parent_occurrence_id,
                  candidate_input_id,disposition,artifact_id,artifact_sha256,
                  source_start_byte,source_end_byte,capture_anchor_id,suggested_url,
                  method,parameter_names,'late_ai_rewrite'
             FROM enumeration_js_resolution_suggestions
            WHERE id=$2"#,
    )
    .bind(Uuid::new_v4())
    .bind(resolution_suggestion_id)
    .execute(db.pool())
    .await
    .expect_err("sealed Resolution lane must reject late AI suggestion mutation");
    assert!(late_resolution_suggestion
        .to_string()
        .contains("enumeration_resolution_suggestion_after_closeout"));

    let late_js_evidence_binding = sqlx::query(
        r#"INSERT INTO tool_truth_evidence_production_bindings(
               id,execution_authority_id,operation_id,project_scope_id,
               project_path_at_freeze,scope_snapshot_id,organization_id,
               stage_execution_id,stage_kind,execution_authority_hash,
               evidence_audit_id,evidence_classification_id,production_binding_hash
           )
           SELECT $1,execution_authority_id,operation_id,project_scope_id,
                  project_path_at_freeze,scope_snapshot_id,organization_id,
                  stage_execution_id,stage_kind,execution_authority_hash,
                  evidence_audit_id,evidence_classification_id,$2
             FROM tool_truth_evidence_production_bindings
            WHERE execution_authority_id=$3
            ORDER BY id LIMIT 1"#,
    )
    .bind(Uuid::new_v4())
    .bind(digest_v1('0'))
    .bind(root.authority.id)
    .execute(db.pool())
    .await
    .expect_err("sealed JS/API authority must reject late evidence binding");
    assert!(late_js_evidence_binding
        .to_string()
        .contains("enumeration_lane_entity_write_after_seal"));

    let receipt_ids = vec![
        browser_receipt.id,
        js_api_receipt.id,
        parameter_receipt.id,
        resolution_receipt.id,
        coverage_receipt.id,
    ];
    let mut timezone_tx = db
        .pool()
        .begin()
        .await
        .expect("begin non-UTC closure graph validation tx");
    sqlx::query("SET LOCAL TIME ZONE 'Asia/Shanghai'")
        .execute(&mut *timezone_tx)
        .await
        .expect("set non-UTC validation timezone");
    let closure_graph_drift: Vec<String> = sqlx::query_scalar(
        r#"SELECT receipt.lane
             FROM enumeration_lane_closure_graph_seals seal
             JOIN enumeration_lane_commit_receipts receipt
               ON receipt.id=seal.lane_receipt_id
            WHERE seal.lane_receipt_id=ANY($1)
              AND seal.closure_graph_sha256<>
                  enumeration_compute_lane_closure_graph_sha256(seal.lane_receipt_id)
            ORDER BY receipt.lane"#,
    )
    .bind(&receipt_ids)
    .fetch_all(&mut *timezone_tx)
    .await
    .expect("recompute immutable Enumeration closure graph seals");
    assert!(
        closure_graph_drift.is_empty(),
        "all lane closure graph seals must remain stable: {closure_graph_drift:?}"
    );
    timezone_tx
        .rollback()
        .await
        .expect("rollback non-UTC closure graph validation tx");
    db.stop().await;
}
