use std::borrow::Cow;

use golish_db::{embedded::EmbeddedPg, DbConfig, GolishDb};
use serial_test::serial;
use sqlx::{
    migrate::Migrator,
    postgres::{PgPool, PgPoolOptions},
    Error as SqlxError,
};
use uuid::Uuid;

const PLAN_A_MIGRATION_VERSION: i64 = 20260729000005;
const PLAN_B_MIGRATION_VERSION: i64 = 20260729000006;

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read local postgres port")
        .port()
}

fn migration_subset(min_version: i64, max_version: i64) -> Migrator {
    let all = sqlx::migrate!("./migrations");
    Migrator {
        migrations: Cow::Owned(
            all.iter()
                .filter(|migration| {
                    migration.version >= min_version && migration.version <= max_version
                })
                .cloned()
                .collect(),
        ),
        ignore_missing: true,
        locking: true,
        no_tx: false,
    }
}

async fn fixture(label: &str) -> (GolishDb, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("hr_{label}_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start isolated migrated postgres");
    (db, data_dir)
}

fn assert_database_rejection(error: &SqlxError, stable_marker: &str) {
    let database_error = error
        .as_database_error()
        .unwrap_or_else(|| panic!("expected PostgreSQL database error, got {error}"));
    assert!(
        database_error.message().contains(stable_marker)
            || database_error.constraint() == Some(stable_marker),
        "expected stable marker {stable_marker}, got message={} constraint={:?}",
        database_error.message(),
        database_error.constraint()
    );
}

async fn insert_operation(pool: &PgPool, operation_id: Uuid) {
    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,
               attack_execution_contract,tool_truth_contract
           ) VALUES($1,'assessment','target_intel','legacy_v1','legacy','legacy_v1')"#,
    )
    .bind(operation_id)
    .execute(pool)
    .await
    .expect("insert legacy operation");
}

fn digest(nibble: char) -> String {
    format!("sha256:{}", nibble.to_string().repeat(64))
}

async fn assert_tables_exist(pool: &PgPool, tables: &[&str]) {
    for table in tables {
        let exists: Option<String> = sqlx::query_scalar("SELECT to_regclass($1)::text")
            .bind(table)
            .fetch_one(pool)
            .await
            .unwrap_or_else(|error| panic!("inspect table {table}: {error}"));
        assert_eq!(exists.as_deref(), Some(*table), "missing table {table}");
    }
}

#[tokio::test]
#[serial]
async fn hypothesis_registry_schema_defaults_existing_operations_to_legacy() {
    let data_dir = tempfile::tempdir().expect("temporary upgrade postgres directory");
    let config = DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("hypothesis_upgrade_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    };
    let connection_string = config.connection_string();
    let mut embedded = EmbeddedPg::start(config)
        .await
        .expect("start pre-Plan-B embedded postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&connection_string)
        .await
        .expect("connect pre-Plan-B pool");
    migration_subset(i64::MIN, PLAN_A_MIGRATION_VERSION)
        .run(&pool)
        .await
        .expect("apply migrations through Plan A");

    let operation_id = Uuid::new_v4();
    insert_operation(&pool, operation_id).await;

    migration_subset(PLAN_B_MIGRATION_VERSION, PLAN_B_MIGRATION_VERSION)
        .run(&pool)
        .await
        .expect("apply the unique Plan B migration");

    let defaults: (String, String, i16) = sqlx::query_as(
        "SELECT contract_version,rollout_mode,mode_rank FROM investigation_rollout WHERE singleton=TRUE",
    )
    .fetch_one(&pool)
    .await
    .expect("read investigation rollout singleton");
    assert_eq!(
        defaults,
        ("legacy_candidate_v1".into(), "legacy_only".into(), 0)
    );

    let frozen: (String, String) = sqlx::query_as(
        "SELECT investigation_contract_version,investigation_rollout_mode FROM operation_state WHERE operation_id=$1",
    )
    .bind(operation_id)
    .fetch_one(&pool)
    .await
    .expect("read historical operation backfill");
    assert_eq!(frozen, ("legacy_candidate_v1".into(), "legacy_only".into()));

    let head_counts: (i64, i64) = sqlx::query_as(
        r#"SELECT
               (SELECT COUNT(*) FROM investigation_projection_source_heads WHERE operation_id=$1),
               (SELECT COUNT(*) FROM investigation_projection_heads WHERE operation_id=$1)"#,
    )
    .bind(operation_id)
    .fetch_one(&pool)
    .await
    .expect("read exact projection heads");
    assert_eq!(head_counts, (1, 1));

    pool.close().await;
    embedded.stop().await;
}

#[tokio::test]
#[serial]
async fn hypothesis_registry_schema_joint_pairs_and_operation_freeze_are_closed() {
    let (mut db, _data_dir) = fixture("joint_pairs").await;
    let ranks: Vec<Option<i16>> = sqlx::query_scalar(
        r#"SELECT operation_joint_contract_rank(tool_truth,contract_version,rollout_mode)
           FROM (VALUES
             ('legacy_v1','legacy_candidate_v1','legacy_only'),
             ('shadow_v1','legacy_candidate_v1','legacy_only'),
             ('shadow_v1','hypothesis_registry_v1','shadow_registry'),
             ('shadow_v1','hypothesis_registry_v1','dual_read_compare'),
             ('receipt_v1','hypothesis_registry_v1','dual_read_compare'),
             ('receipt_v1','hypothesis_registry_v1','registry_authoritative_legacy_projection'),
             ('receipt_v1','hypothesis_registry_v1','new_only'),
             ('legacy_v1','hypothesis_registry_v1','shadow_registry')
           ) AS pairs(tool_truth,contract_version,rollout_mode)"#,
    )
    .fetch_all(db.pool())
    .await
    .expect("evaluate the single joint-pair function");
    assert_eq!(
        ranks,
        vec![
            Some(0),
            Some(1),
            Some(2),
            Some(3),
            Some(4),
            Some(5),
            Some(6),
            None
        ]
    );

    let operation_id = Uuid::new_v4();
    insert_operation(db.pool(), operation_id).await;
    let immutable = sqlx::query(
        "UPDATE operation_state SET investigation_rollout_mode='shadow_registry' WHERE operation_id=$1",
    )
    .bind(operation_id)
    .execute(db.pool())
    .await
    .expect_err("operation-frozen investigation mode must reject mutation");
    assert_database_rejection(&immutable, "OPERATION_INVESTIGATION_CONTRACT_IMMUTABLE");

    let invalid_id = Uuid::new_v4();
    let invalid = sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,
               attack_execution_contract,tool_truth_contract,
               investigation_contract_version,investigation_rollout_mode
           ) VALUES($1,'assessment','target_intel','legacy_v1','legacy','legacy_v1',
                    'hypothesis_registry_v1','shadow_registry')"#,
    )
    .bind(invalid_id)
    .execute(db.pool())
    .await
    .expect_err("an illegal joint pair must fail at the database boundary");
    assert_database_rejection(&invalid, "operation_joint_contract_not_deployed_or_adopted");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn hypothesis_registry_schema_adoption_is_adjacent_and_append_only() {
    let (mut db, _data_dir) = fixture("adoption").await;
    let source_operation_id = Uuid::new_v4();
    insert_operation(db.pool(), source_operation_id).await;
    let target_operation_id = Uuid::new_v4();
    let adoption_id = Uuid::new_v4();

    let mut tx = db.pool().begin().await.expect("begin adjacent adoption");
    sqlx::query(
        r#"INSERT INTO operation_contract_adoptions(
               adoption_id,source_operation_id,target_operation_id,
               source_tool_truth_contract,source_investigation_contract_version,
               source_investigation_rollout_mode,source_joint_rank,
               target_tool_truth_contract,target_investigation_contract_version,
               target_investigation_rollout_mode,target_joint_rank,
               source_final_seal_hash,adoption_set_hash,stable_request_id,receipt_hash
           ) VALUES(
               $1,$2,$3,'legacy_v1','legacy_candidate_v1','legacy_only',0,
               'shadow_v1','legacy_candidate_v1','legacy_only',1,$4,$5,$6,$7
           )"#,
    )
    .bind(adoption_id)
    .bind(source_operation_id)
    .bind(target_operation_id)
    .bind(digest('a'))
    .bind(digest('b'))
    .bind(Uuid::new_v4())
    .bind(digest('c'))
    .execute(&mut *tx)
    .await
    .expect("insert adjacent adoption before deferred target");
    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,
               attack_execution_contract,tool_truth_contract,
               investigation_contract_version,investigation_rollout_mode
           ) VALUES($1,'assessment','target_intel','legacy_v1','legacy','shadow_v1',
                    'legacy_candidate_v1','legacy_only')"#,
    )
    .bind(target_operation_id)
    .execute(&mut *tx)
    .await
    .expect("insert the exact adopted target pair");
    tx.commit()
        .await
        .expect("commit adjacent adoption atomically");

    let frozen: (String, String, String) = sqlx::query_as(
        r#"SELECT tool_truth_contract,investigation_contract_version,investigation_rollout_mode
             FROM operation_state WHERE operation_id=$1"#,
    )
    .bind(target_operation_id)
    .fetch_one(db.pool())
    .await
    .expect("read adopted operation contract");
    assert_eq!(
        frozen,
        (
            "shadow_v1".into(),
            "legacy_candidate_v1".into(),
            "legacy_only".into()
        )
    );

    let mutation =
        sqlx::query("UPDATE operation_contract_adoptions SET receipt_hash=$2 WHERE adoption_id=$1")
            .bind(adoption_id)
            .bind(digest('d'))
            .execute(db.pool())
            .await
            .expect_err("adoption receipts are append-only");
    assert_database_rejection(&mutation, "investigation_append_only");

    let jump = sqlx::query(
        r#"INSERT INTO operation_contract_adoptions(
               adoption_id,source_operation_id,target_operation_id,
               source_tool_truth_contract,source_investigation_contract_version,
               source_investigation_rollout_mode,source_joint_rank,
               target_tool_truth_contract,target_investigation_contract_version,
               target_investigation_rollout_mode,target_joint_rank,
               source_final_seal_hash,adoption_set_hash,stable_request_id,receipt_hash
           ) VALUES(
               $1,$2,$3,'legacy_v1','legacy_candidate_v1','legacy_only',0,
               'shadow_v1','hypothesis_registry_v1','shadow_registry',2,$4,$5,$6,$7
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(source_operation_id)
    .bind(Uuid::new_v4())
    .bind(digest('e'))
    .bind(digest('f'))
    .bind(Uuid::new_v4())
    .bind(digest('9'))
    .execute(db.pool())
    .await
    .expect_err("joint-rank adoption cannot skip a state");
    assert_database_rejection(&jump, "operation_contract_adoption_adjacent_check");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn hypothesis_registry_schema_stage_team_extensions_are_exact() {
    let (mut db, _data_dir) = fixture("stage_extensions").await;
    let work_item_check: String = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(oid)
             FROM pg_constraint
            WHERE conname='stage_work_items_created_by_check'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("load work-item authority constraint");
    assert!(work_item_check.contains("server_phase_transition"));

    let output_check: String = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(oid)
             FROM pg_constraint
            WHERE conname='stage_worker_outputs_business_disposition_check'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("load worker-output disposition constraint");
    assert!(output_check.contains("artifact_recorded"));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn hypothesis_registry_schema_owns_plan_b_tables_but_not_plan_c_authority() {
    let (mut db, _data_dir) = fixture("plan_boundaries").await;
    for table in [
        "attack_hypotheses",
        "attack_hypothesis_revisions",
        "attack_hypothesis_verification_contracts",
        "attack_hypothesis_verification_plans",
        "candidate_analysis_snapshots",
        "candidate_analysis_attempts",
        "investigation_projection_outbox_batches",
        "investigation_projection_entity_versions",
        "investigation_projection_batch_receipts",
    ] {
        let exists: Option<String> = sqlx::query_scalar("SELECT to_regclass($1)::text")
            .bind(table)
            .fetch_one(db.pool())
            .await
            .expect("inspect Plan B table");
        assert_eq!(
            exists.as_deref(),
            Some(table),
            "missing Plan B table {table}"
        );
    }
    for table in [
        "verification_capability_assessments",
        "hypothesis_revision_adjudications",
        "hypothesis_revision_terminal_decisions",
    ] {
        let exists: Option<String> = sqlx::query_scalar("SELECT to_regclass($1)::text")
            .bind(table)
            .fetch_one(db.pool())
            .await
            .expect("inspect Plan C-owned table");
        assert!(
            exists.is_none(),
            "Plan B must not create Plan C table {table}"
        );
    }
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn candidate_snapshot_tool_truth_authority_schema_has_compound_plan_a_binding() {
    let (mut db, _data_dir) = fixture("bundle_binding").await;
    let columns: Vec<String> = sqlx::query_scalar(
        r#"SELECT column_name
             FROM information_schema.columns
            WHERE table_name='candidate_analysis_snapshots'
              AND column_name IN (
                  'tool_truth_authority_bundle_seal_id','operation_id','organization_id',
                  'relevant_root_set_hash','bundle_member_set_hash',
                  'semantic_authority_bundle_hash','freshness_attestation_bundle_hash',
                  'temporal_validity_bundle_hash','temporal_validity_policy_set_hash',
                  'target_state_epoch_set_hash','stable_consumer_request_id','snapshot_status'
              )
            ORDER BY column_name"#,
    )
    .fetch_all(db.pool())
    .await
    .expect("load snapshot authority columns");
    assert_eq!(
        columns.len(),
        12,
        "snapshot authority must not collapse the A bundle"
    );

    let member_fk: String = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(c.oid)
             FROM pg_constraint c
             JOIN pg_class child ON child.oid=c.conrelid
             JOIN pg_class parent ON parent.oid=c.confrelid
            WHERE c.contype='f'
              AND child.relname='candidate_analysis_snapshot_authority_bundle_members'
              AND parent.relname='tool_truth_authority_bundle_members'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect exact Plan A member binding");
    for column in [
        "root_execution_authority_id",
        "root_denominator_hash",
        "authority_set_semantic_hash",
        "authority_set_graph_hash",
        "authority_set_freshness_hash",
        "temporal_validity_policy_set_hash",
        "target_state_epoch_set_hash",
        "member_status",
        "member_hash",
    ] {
        assert!(
            member_fk.contains(column),
            "compound member FK omits {column}"
        );
    }
    let header_fk: String = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(c.oid)
             FROM pg_constraint c
             JOIN pg_class child ON child.oid=c.conrelid
             JOIN pg_class parent ON parent.oid=c.confrelid
            WHERE c.contype='f'
              AND child.relname='candidate_analysis_snapshots'
              AND parent.relname='tool_truth_authority_bundle_seals'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect exact Plan A bundle header binding");
    for column in [
        "stable_consumer_request_id",
        "relevant_root_set_hash",
        "bundle_member_set_hash",
        "semantic_authority_bundle_hash",
        "freshness_attestation_bundle_hash",
        "temporal_validity_bundle_hash",
        "temporal_validity_policy_set_hash",
        "target_state_epoch_set_hash",
        "bundle_sealed_at",
    ] {
        assert!(
            header_fk.contains(column),
            "compound header FK omits {column}"
        );
    }
    let exact_set_trigger: String = sqlx::query_scalar(
        r#"SELECT pg_get_triggerdef(oid) FROM pg_trigger
            WHERE tgname='candidate_analysis_snapshot_exact_authority_bundle'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("load deferred exact bundle trigger");
    assert!(exact_set_trigger.contains("DEFERRABLE INITIALLY DEFERRED"));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn hypothesis_state_authority_schema_rejects_terminal_forgery() {
    let (mut db, _data_dir) = fixture("terminal_authority").await;
    let operation_id = Uuid::new_v4();
    insert_operation(db.pool(), operation_id).await;
    let organization_id = Uuid::new_v4();
    sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$2,'Registry Org')")
        .bind(organization_id)
        .bind(format!("/tmp/hypothesis-org-{}", Uuid::new_v4().simple()))
        .execute(db.pool())
        .await
        .expect("insert registry organization");
    let root_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO attack_hypotheses(
               root_id,operation_id,organization_id,root_kind,
               identity_ingredients,identity_ingredients_hash
           ) VALUES($1,$2,$3,'initial','{}'::jsonb,$4)"#,
    )
    .bind(root_id)
    .bind(operation_id)
    .bind(organization_id)
    .bind(format!("sha256:{}", "1".repeat(64)))
    .execute(db.pool())
    .await
    .expect("insert hypothesis root");

    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin forged terminal transaction");
    let revision_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_revisions(
               revision_id,root_id,operation_id,organization_id,revision_ordinal,
               semantic_key,semantic_key_hash,subject_kind,subject_identity_hash,
               target_type_at_time,target_value_at_time,predicate_schema,predicate_version,
               normalized_arguments,trust_boundary,polarity,epistemic_state,lifecycle_state,
               planning_readiness,structured_claim,priority,risk_impact,revision_hash
           ) VALUES(
               $1,$2,$3,$4,0,'{}'::jsonb,$5,'origin',$6,
               'domain','example.test','predicate.v1',1,'{}'::jsonb,'internet','positive',
               'verified','closed','deferred','{}'::jsonb,0,'{}'::jsonb,$7
           )"#,
    )
    .bind(revision_id)
    .bind(root_id)
    .bind(operation_id)
    .bind(organization_id)
    .bind(format!("sha256:{}", "2".repeat(64)))
    .bind(format!("sha256:{}", "3".repeat(64)))
    .bind(format!("sha256:{}", "4".repeat(64)))
    .execute(&mut *tx)
    .await
    .expect("deferred creating-event authority allows statement execution");
    let commit_error = tx
        .commit()
        .await
        .expect_err("verified revision without Plan C authority must fail at commit");
    assert_database_rejection(&commit_error, "HYPOTHESIS_CREATING_EVENT_REQUIRED");

    let retained: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM attack_hypothesis_revisions WHERE revision_id=$1")
            .bind(revision_id)
            .fetch_one(db.pool())
            .await
            .expect("confirm forged revision rollback");
    assert_eq!(retained, 0);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn hypothesis_state_authority_schema_accepts_nonterminal_and_rejects_candidate_terminal() {
    let (mut db, _data_dir) = fixture("state_events").await;
    let operation_id = Uuid::new_v4();
    insert_operation(db.pool(), operation_id).await;
    let organization_id = Uuid::new_v4();
    sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$2,'State Org')")
        .bind(organization_id)
        .bind(format!("/tmp/state-org-{}", Uuid::new_v4().simple()))
        .execute(db.pool())
        .await
        .expect("insert state-event organization");

    let root_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO attack_hypotheses(
               root_id,operation_id,organization_id,root_kind,
               identity_ingredients,identity_ingredients_hash
           ) VALUES($1,$2,$3,'initial','{}'::jsonb,$4)"#,
    )
    .bind(root_id)
    .bind(operation_id)
    .bind(organization_id)
    .bind(digest('1'))
    .execute(db.pool())
    .await
    .expect("insert nonterminal root");
    let revision_id = Uuid::new_v4();
    let mut legal = db.pool().begin().await.expect("begin legal creating event");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_revisions(
               revision_id,root_id,operation_id,organization_id,revision_ordinal,
               semantic_key,semantic_key_hash,subject_kind,subject_identity_hash,
               target_type_at_time,target_value_at_time,predicate_schema,predicate_version,
               normalized_arguments,trust_boundary,polarity,epistemic_state,lifecycle_state,
               planning_readiness,structured_claim,priority,risk_impact,revision_hash
           ) VALUES($1,$2,$3,$4,0,'{}'::jsonb,$5,'origin',$6,'domain','example.test',
                    'predicate.v1',1,'{}'::jsonb,'internet','positive','proposed','current',
                    'ready_for_strategy','{}'::jsonb,1,'{}'::jsonb,$7)"#,
    )
    .bind(revision_id)
    .bind(root_id)
    .bind(operation_id)
    .bind(organization_id)
    .bind(digest('2'))
    .bind(digest('3'))
    .bind(digest('4'))
    .execute(&mut *legal)
    .await
    .expect("insert proposed revision");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_state_events(
               event_id,operation_id,organization_id,root_id,successor_revision_id,
               event_kind,origin_authority,successor_epistemic_state,event_hash,server_decision_id
           ) VALUES($1,$2,$3,$4,$5,'created','candidate_analysis','proposed',$6,$7)"#,
    )
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(organization_id)
    .bind(root_id)
    .bind(revision_id)
    .bind(digest('5'))
    .bind(Uuid::new_v4())
    .execute(&mut *legal)
    .await
    .expect("insert exact creating event");
    legal
        .commit()
        .await
        .expect("commit legal nonterminal authority");

    let terminal_root_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO attack_hypotheses(
               root_id,operation_id,organization_id,root_kind,
               identity_ingredients,identity_ingredients_hash
           ) VALUES($1,$2,$3,'initial','{}'::jsonb,$4)"#,
    )
    .bind(terminal_root_id)
    .bind(operation_id)
    .bind(organization_id)
    .bind(digest('6'))
    .execute(db.pool())
    .await
    .expect("insert forged terminal root");
    let terminal_revision_id = Uuid::new_v4();
    let mut forged = db
        .pool()
        .begin()
        .await
        .expect("begin forged terminal event");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_revisions(
               revision_id,root_id,operation_id,organization_id,revision_ordinal,
               semantic_key,semantic_key_hash,subject_kind,subject_identity_hash,
               target_type_at_time,target_value_at_time,predicate_schema,predicate_version,
               normalized_arguments,trust_boundary,polarity,epistemic_state,lifecycle_state,
               planning_readiness,structured_claim,priority,risk_impact,revision_hash
           ) VALUES($1,$2,$3,$4,0,'{}'::jsonb,$5,'origin',$6,'domain','terminal.test',
                    'predicate.v1',1,'{}'::jsonb,'internet','positive','verified','closed',
                    'deferred','{}'::jsonb,1,'{}'::jsonb,$7)"#,
    )
    .bind(terminal_revision_id)
    .bind(terminal_root_id)
    .bind(operation_id)
    .bind(organization_id)
    .bind(digest('7'))
    .bind(digest('8'))
    .bind(digest('9'))
    .execute(&mut *forged)
    .await
    .expect("insert terminal revision before deferred authority check");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_state_events(
               event_id,operation_id,organization_id,root_id,successor_revision_id,
               event_kind,origin_authority,successor_epistemic_state,event_hash,server_decision_id
           ) VALUES($1,$2,$3,$4,$5,'verified','candidate_analysis','verified',$6,$7)"#,
    )
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(organization_id)
    .bind(terminal_root_id)
    .bind(terminal_revision_id)
    .bind(digest('a'))
    .bind(Uuid::new_v4())
    .execute(&mut *forged)
    .await
    .expect("insert forged Candidate terminal event before deferred authority check");
    let error = forged
        .commit()
        .await
        .expect_err("Candidate Analysis cannot authorize verified terminal state");
    assert_database_rejection(&error, "HYPOTHESIS_CANDIDATE_TERMINAL_FORBIDDEN");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn candidate_analysis_attempt_schema_has_immutable_two_wave_spine() {
    let (mut db, _data_dir) = fixture("attempt_spine").await;
    assert_tables_exist(
        db.pool(),
        &[
            "candidate_analysis_attempts",
            "candidate_analysis_attempt_state_events",
            "candidate_analysis_page_receipts",
            "candidate_analysis_work_items",
            "candidate_analysis_artifacts",
            "hypothesis_proposals",
            "candidate_analysis_proposal_censuses",
            "candidate_analysis_proposal_census_members",
            "candidate_analysis_critic_censuses",
            "candidate_analysis_critic_census_members",
        ],
    )
    .await;
    let append_only: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM pg_trigger trigger
            JOIN pg_class table_ref ON table_ref.oid=trigger.tgrelid
            JOIN pg_proc function_ref ON function_ref.oid=trigger.tgfoid
           WHERE NOT trigger.tgisinternal
             AND table_ref.relname IN (
                 'candidate_analysis_attempts','candidate_analysis_attempt_state_events',
                 'candidate_analysis_page_receipts','candidate_analysis_artifacts'
             )
             AND function_ref.proname='investigation_reject_append_only'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect attempt append-only triggers");
    assert_eq!(append_only, 4);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn candidate_knowledge_feed_schema_freezes_expected_and_observed_members() {
    let (mut db, _data_dir) = fixture("knowledge_feed").await;
    assert_tables_exist(
        db.pool(),
        &[
            "candidate_analysis_knowledge_feed_denominators",
            "candidate_analysis_knowledge_feed_denominator_members",
            "candidate_analysis_knowledge_feed_snapshots",
            "candidate_analysis_knowledge_feed_snapshot_members",
            "candidate_analysis_product_version_censuses",
            "candidate_analysis_product_version_census_members",
            "candidate_analysis_feed_match_censuses",
            "candidate_analysis_feed_match_census_members",
            "candidate_analysis_enrichment_obligations",
        ],
    )
    .await;
    let disposition_check: String = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(c.oid)
             FROM pg_constraint c JOIN pg_class t ON t.oid=c.conrelid
            WHERE t.relname='candidate_analysis_knowledge_feed_snapshot_members'
              AND pg_get_constraintdef(c.oid) LIKE '%signature_invalid%'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("load closed feed disposition constraint");
    for value in [
        "current",
        "stale",
        "signature_invalid",
        "signer_revoked",
        "unavailable",
    ] {
        assert!(disposition_check.contains(value));
    }
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn candidate_input_chunk_census_schema_keeps_replayable_exact_sets() {
    let (mut db, _data_dir) = fixture("input_chunks").await;
    assert_tables_exist(
        db.pool(),
        &[
            "candidate_analysis_snapshot_inputs",
            "candidate_analysis_input_chunk_censuses",
            "candidate_analysis_input_chunk_census_members",
            "candidate_analysis_snapshot_source_sets",
            "candidate_analysis_input_proposal_dispositions",
        ],
    )
    .await;
    let forbidden_instruction_authority: String = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(c.oid)
             FROM pg_constraint c JOIN pg_class t ON t.oid=c.conrelid
            WHERE t.relname='candidate_analysis_snapshot_inputs'
              AND pg_get_constraintdef(c.oid) LIKE '%instruction_authority%'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("load untrusted-input instruction constraint");
    assert!(forbidden_instruction_authority.contains("NOT instruction_authority"));
    let body_columns: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM information_schema.columns
            WHERE table_name='candidate_analysis_input_chunk_census_members'
              AND column_name IN (
                  'immutable_redacted_body','content_blob_id','chunk_hash',
                  'source_range_start','source_range_end'
              )"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect replayable chunk body columns");
    assert_eq!(body_columns, 5);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn hypothesis_coverage_review_schema_has_recursive_exact_reducer_tables() {
    let (mut db, _data_dir) = fixture("coverage_review").await;
    assert_tables_exist(
        db.pool(),
        &[
            "candidate_analysis_hypothesis_coverage_checklist_members",
            "candidate_analysis_hypothesis_coverage_chunk_partitions",
            "candidate_analysis_hypothesis_coverage_subreview_censuses",
            "candidate_analysis_hypothesis_coverage_subreview_census_members",
            "candidate_analysis_hypothesis_coverage_subreviews",
            "candidate_analysis_hypothesis_coverage_synthesis_censuses",
            "candidate_analysis_hypothesis_coverage_synthesis_census_members",
            "candidate_analysis_hypothesis_coverage_synthesis_reviews",
            "candidate_analysis_hypothesis_coverage_global_reviews",
            "candidate_analysis_hypothesis_coverage_reviews",
        ],
    )
    .await;
    let outcome_constraint_tables: Vec<String> = sqlx::query_scalar(
        r#"SELECT DISTINCT t.relname
             FROM pg_constraint c JOIN pg_class t ON t.oid=c.conrelid
            WHERE t.relname IN (
                'candidate_analysis_hypothesis_coverage_subreviews',
                'candidate_analysis_hypothesis_coverage_global_reviews',
                'candidate_analysis_hypothesis_coverage_reviews'
            ) AND pg_get_constraintdef(c.oid) LIKE '%missed_hypothesis%'"#,
    )
    .fetch_all(db.pool())
    .await
    .expect("load coverage outcome constraints");
    assert_eq!(outcome_constraint_tables.len(), 3);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn verification_contract_schema_has_closed_component_control_shapes() {
    let (mut db, _data_dir) = fixture("verification_contract").await;
    assert_tables_exist(
        db.pool(),
        &[
            "attack_hypothesis_verification_objectives",
            "attack_hypothesis_verification_contracts",
            "attack_hypothesis_verification_objective_claim_components",
            "attack_hypothesis_verification_predicate_components",
            "attack_hypothesis_verification_required_controls",
            "attack_hypothesis_verification_pair_bindings",
            "attack_hypothesis_verification_ordered_steps",
        ],
    )
    .await;
    let combinator_check: String = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(c.oid)
             FROM pg_constraint c JOIN pg_class t ON t.oid=c.conrelid
            WHERE t.relname='attack_hypothesis_verification_contracts'
              AND pg_get_constraintdef(c.oid) LIKE '%paired_differential%'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("load VerificationContract combinator constraint");
    assert!(combinator_check.contains("ordered_sequence"));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn hypothesis_claim_component_schema_is_closed_and_revision_scoped() {
    let (mut db, _data_dir) = fixture("claim_component").await;
    let component_check: String = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(c.oid)
             FROM pg_constraint c JOIN pg_class t ON t.oid=c.conrelid
            WHERE t.relname='attack_hypothesis_claim_components'
              AND pg_get_constraintdef(c.oid) LIKE '%trust_boundary_condition%'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("load claim component kind constraint");
    for value in [
        "claim_clause",
        "impact_qualifier",
        "trust_boundary_condition",
        "identity_condition",
    ] {
        assert!(component_check.contains(value));
    }
    let objective_component_fk: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM pg_constraint c
            JOIN pg_class child ON child.oid=c.conrelid
            JOIN pg_class parent ON parent.oid=c.confrelid
           WHERE c.contype='f'
             AND child.relname='attack_hypothesis_verification_objective_claim_components'
             AND parent.relname='attack_hypothesis_claim_components'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect objective claim-component authority");
    assert_eq!(objective_component_fk, 1);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn hypothesis_verification_plan_schema_has_objective_and_path_exact_sets() {
    let (mut db, _data_dir) = fixture("verification_plan").await;
    assert_tables_exist(
        db.pool(),
        &[
            "attack_hypothesis_verification_plans",
            "attack_hypothesis_verification_plan_objectives",
            "attack_hypothesis_verification_plan_paths",
            "attack_hypothesis_verification_plan_path_members",
        ],
    )
    .await;
    let plan_shape_columns: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM information_schema.columns
            WHERE table_name='attack_hypothesis_verification_plans'
              AND column_name IN (
                  'required_claim_component_count','required_claim_component_set_hash',
                  'objective_count','objective_set_hash','proof_path_count','proof_path_set_hash',
                  'outer_aggregation_policy_version','outer_aggregation_policy_digest','plan_hash'
              )"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect frozen verification plan exact sets");
    assert_eq!(plan_shape_columns, 9);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn projection_catalog_schema_is_closed_and_rejects_unknown_mapping() {
    let (mut db, _data_dir) = fixture("projection_catalog").await;
    let valid: bool = sqlx::query_scalar(
        "SELECT projection_timeline_mapping_is_valid('hypothesis','insert','hypothesis_inserted')",
    )
    .fetch_one(db.pool())
    .await
    .expect("evaluate known projection mapping");
    assert!(valid);
    let unknown: bool = sqlx::query_scalar(
        "SELECT projection_timeline_mapping_is_valid('future_entity','insert','hypothesis_inserted')",
    )
    .fetch_one(db.pool())
    .await
    .expect("evaluate unknown projection mapping");
    assert!(!unknown);
    assert_tables_exist(db.pool(), &["investigation_projection_changes"]).await;
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn projection_plan_b_verification_plan_route_is_exact_one() {
    let (mut db, _data_dir) = fixture("plan_b_route").await;
    let plan_route: bool = sqlx::query_scalar(
        "SELECT projection_timeline_mapping_is_valid('hypothesis_verification_plan','close','hypothesis_verification_plan_sealed')",
    )
    .fetch_one(db.pool())
    .await
    .expect("evaluate Plan B plan-seal route");
    assert!(plan_route);
    let campaign_substitution: bool = sqlx::query_scalar(
        "SELECT projection_timeline_mapping_is_valid('campaign_terminal','close','hypothesis_verification_plan_sealed')",
    )
    .fetch_one(db.pool())
    .await
    .expect("evaluate forbidden Campaign substitution");
    assert!(!campaign_substitution);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn projection_plan_c_route_catalog_keeps_future_kinds_without_authority_tables() {
    let (mut db, _data_dir) = fixture("plan_c_routes").await;
    for (entity, change, event) in [
        (
            "hypothesis_revision_adjudication",
            "close",
            "hypothesis_revision_adjudication_closed",
        ),
        (
            "hypothesis_revision_terminal_decision",
            "close",
            "hypothesis_revision_terminal_decision_closed",
        ),
        ("finding", "insert", "finding_inserted"),
        (
            "hypothesis_state_event",
            "insert",
            "hypothesis_state_event_inserted",
        ),
        ("hypothesis", "insert", "hypothesis_inserted"),
    ] {
        let valid: bool =
            sqlx::query_scalar("SELECT projection_timeline_mapping_is_valid($1,$2,$3)")
                .bind(entity)
                .bind(change)
                .bind(event)
                .fetch_one(db.pool())
                .await
                .expect("evaluate frozen Plan C route vocabulary");
        assert!(valid, "missing future route {entity}/{change}/{event}");
    }
    for table in [
        "hypothesis_revision_adjudications",
        "hypothesis_revision_terminal_decisions",
    ] {
        let exists: Option<String> = sqlx::query_scalar("SELECT to_regclass($1)::text")
            .bind(table)
            .fetch_one(db.pool())
            .await
            .expect("inspect absent future authority table");
        assert!(exists.is_none());
    }
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn projection_source_snapshot_schema_freezes_inline_or_blob_payload() {
    let (mut db, _data_dir) = fixture("projection_snapshot").await;
    assert_tables_exist(
        db.pool(),
        &[
            "investigation_projection_source_blobs",
            "investigation_projection_outbox",
        ],
    )
    .await;
    let source_columns: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM information_schema.columns
            WHERE table_name='investigation_projection_outbox'
              AND column_name IN (
                  'source_snapshot_schema','source_snapshot_version','source_snapshot_hash',
                  'immutable_source_body','source_blob_id','source_blob_hash',
                  'source_occurred_at','source_time_status'
              )"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect frozen projection source snapshot");
    assert_eq!(source_columns, 8);
    let live_locator_columns: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM information_schema.columns
            WHERE table_name='investigation_projection_outbox'
              AND column_name IN ('source_table','source_path','loader','live_locator')"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect forbidden live source locators");
    assert_eq!(live_locator_columns, 0);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn projection_entity_predecessor_schema_binds_direct_version_and_hash() {
    let (mut db, _data_dir) = fixture("entity_predecessor").await;
    let self_fk_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM pg_constraint c
            JOIN pg_class child ON child.oid=c.conrelid
            JOIN pg_class parent ON parent.oid=c.confrelid
           WHERE c.contype='f'
             AND child.relname='investigation_projection_entity_versions'
             AND parent.relname='investigation_projection_entity_versions'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect direct predecessor self-FK");
    assert_eq!(self_fk_count, 1);
    let predecessor_check: String = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(c.oid)
             FROM pg_constraint c JOIN pg_class t ON t.oid=c.conrelid
            WHERE t.relname='investigation_projection_entity_versions'
              AND pg_get_constraintdef(c.oid) LIKE '%predecessor_absent%'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("load entity predecessor shape constraint");
    assert!(predecessor_check.contains("entity_version"));
    assert!(predecessor_check.contains("predecessor_projection_hash"));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn projection_batch_schema_has_receipt_truth_and_no_processed_flag() {
    let (mut db, _data_dir) = fixture("projection_batch").await;
    assert_tables_exist(
        db.pool(),
        &[
            "investigation_projection_source_heads",
            "investigation_projection_heads",
            "investigation_projection_outbox_batches",
            "investigation_projection_outbox",
            "investigation_projection_entity_versions",
            "investigation_projection_changes",
            "investigation_projection_batch_receipts",
        ],
    )
    .await;
    let processed_columns: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM information_schema.columns
            WHERE table_name='investigation_projection_outbox'
              AND column_name IN ('processed','processed_at','is_processed')"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect forbidden per-member processing markers");
    assert_eq!(processed_columns, 0);
    let receipt_unique: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM pg_constraint c JOIN pg_class t ON t.oid=c.conrelid
            WHERE t.relname='investigation_projection_batch_receipts'
              AND c.contype='u' AND pg_get_constraintdef(c.oid) LIKE '%batch_id%'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect exact-one batch receipt");
    assert!(receipt_unique >= 1);
    db.stop().await;
}
