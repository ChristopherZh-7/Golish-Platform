const COMPILER: &str = include_str!("../src/repo/investigation_hypothesis_compiler.rs");
const ASSET_BINDING_MIGRATION: &str =
    include_str!("../migrations/20260813000004_investigation_asset_hypothesis_binding.sql");
const DYNAMIC_TASK_ADMISSION_MIGRATION: &str =
    include_str!("../migrations/20260814000004_investigation_dynamic_task_admission.sql");
const ASSET_GENERATION_ORDINAL_MIGRATION: &str =
    include_str!("../migrations/20260814000014_investigation_asset_generation_ordinal.sql");

use golish_db::{DbConfig, GolishDb};
use serial_test::serial;
use uuid::Uuid;

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read reserved local postgres port")
        .port()
}

#[test]
fn compiler_resolves_every_proposal_subject_to_one_exact_asset_lane() {
    assert!(
        COMPILER.contains("resolve_proposal_asset_lane_on("),
        "canonical compilation must resolve the model-provided subject through server-owned subject authority before creating a root"
    );
    assert!(
        COMPILER.contains("proposal.subject_kind")
            && COMPILER.contains("proposal.subject_identity_hash")
            && COMPILER.contains("asset_lane_id"),
        "asset resolution must use the exact typed subject identity and return an asset lane"
    );
    assert!(
        COMPILER.contains("candidate_analysis_snapshot_inputs")
            || COMPILER.contains("investigation_asset_queue_members"),
        "asset resolution must be anchored to frozen server authority, never prompt text alone"
    );
}

#[test]
fn new_candidate_freeze_and_compiler_have_no_organization_wide_fallback() {
    const KIT: &str = include_str!("../../golish-agent-kit/src/db_traits/hypothesis_registry.rs");
    const FREEZE: &str = include_str!("../src/repo/candidate_analysis.rs");
    let freeze_request = KIT
        .split_once("pub struct FreezeCandidateAnalysisSnapshot")
        .expect("freeze request")
        .1
        .split_once('}')
        .expect("freeze request terminator")
        .0;
    assert!(freeze_request.contains("pub asset_lane_id: Uuid"));
    assert!(!freeze_request.contains("Option<Uuid>"));
    assert!(FREEZE.contains("input.asset_lane_id.is_nil()"));
    assert!(FREEZE.contains("WHERE asset_lane_id=$1"));
    assert!(!COMPILER.contains("asset_lane_id IS NOT DISTINCT FROM"));
    assert!(!COMPILER.contains("COALESCE($10,'subject_identity_hash')"));
    assert!(COMPILER.contains("asset_lane_initial_root_id("));
}

#[test]
fn canonical_revision_persists_exact_live_and_at_time_target_identity() {
    let insert = COMPILER
        .split_once("INSERT INTO attack_hypothesis_revisions(")
        .expect("canonical revision insert")
        .1
        .split_once(".execute(&mut **tx)")
        .expect("canonical revision insert terminator")
        .0;

    assert!(
        insert.contains("asset_lane_id") && insert.contains("target_live_id"),
        "a new-contract revision must persist both permanent lane ownership and the resolved live target"
    );
    assert!(
        insert.contains("target_type_at_time") && insert.contains("target_value_at_time"),
        "a revision must retain real at-time target metadata rather than only a subject hash"
    );
    assert!(
        !insert.contains("'subject_identity_hash',$8"),
        "the placeholder target_type/value encoding must not survive exact target resolution"
    );
}

#[test]
fn registry_generation_and_verification_authorities_expose_one_asset_lane() {
    for table in [
        "candidate_analysis_snapshots",
        "candidate_analysis_attempts",
        "attack_hypotheses",
        "attack_hypothesis_revisions",
        "hypothesis_generations",
        "hypothesis_generation_members",
    ] {
        assert!(
            ASSET_BINDING_MIGRATION.contains(&format!(
                "ALTER TABLE {table}\n    ADD COLUMN asset_lane_id"
            )),
            "{table} must carry mandatory asset_lane_id for the new queue contract"
        );
    }

    for table in [
        "hypothesis_verification_tasks",
        "hypothesis_verification_task_campaigns",
    ] {
        assert!(
            ASSET_BINDING_MIGRATION.contains(&format!(
                "ALTER TABLE {table}\n    ADD COLUMN asset_lane_id"
            )),
            "{table} must preserve the hypothesis asset lane"
        );
    }

    assert!(
        ASSET_BINDING_MIGRATION
            .contains("ALTER TABLE verification_campaigns\n    ADD COLUMN asset_lane_id"),
        "the executable Campaign authority must preserve the same asset lane"
    );
}

#[test]
fn generation_ordinals_are_unique_per_asset_lane_not_per_organization() {
    assert!(ASSET_GENERATION_ORDINAL_MIGRATION
        .contains("ARRAY['operation_id','organization_id','generation_ordinal']::TEXT[]"));
    assert!(ASSET_GENERATION_ORDINAL_MIGRATION.contains("UNIQUE(asset_lane_id,generation_ordinal)"));
    assert!(COMPILER.contains("WHERE operation_id=$1 AND organization_id=$2 AND asset_lane_id=$3"));
}

#[test]
fn migration_defers_exact_cross_authority_lane_guards_to_transaction_commit() {
    assert!(ASSET_BINDING_MIGRATION.contains("investigation_asset_lanes(asset_lane_id)"));
    assert!(ASSET_BINDING_MIGRATION.contains("investigation_resolve_proposal_asset_lane"));
    assert!(ASSET_BINDING_MIGRATION.contains("fingerprint_origin_observations"));
    assert!(ASSET_BINDING_MIGRATION.contains("endpoint.target_id=lane.target_id"));
    assert!(ASSET_BINDING_MIGRATION.contains("revision.asset_lane_id=NEW.asset_lane_id"));
    assert!(ASSET_BINDING_MIGRATION.contains("generation.asset_lane_id=NEW.asset_lane_id"));
    assert!(ASSET_BINDING_MIGRATION.contains("reservation.asset_lane_id=NEW.asset_lane_id"));
    assert!(ASSET_BINDING_MIGRATION.contains("wave.asset_lane_id=NEW.asset_lane_id"));
    assert!(ASSET_BINDING_MIGRATION.contains("pending.asset_lane_id=NEW.asset_lane_id"));
    assert!(ASSET_BINDING_MIGRATION.contains("DEFERRABLE INITIALLY DEFERRED"));
}

#[test]
fn compiler_admits_one_lane_bound_dynamic_task_without_campaign_reservation() {
    let task_creation = COMPILER
        .split_once("INSERT INTO hypothesis_verification_tasks(")
        .expect("VerificationTask insert")
        .1
        .split_once(".execute(&mut **tx)")
        .expect("VerificationTask insert terminator")
        .0;
    assert!(
        task_creation.contains("asset_lane_id"),
        "VerificationTask creation must copy the canonical revision lane"
    );
    assert!(
        COMPILER.contains("HypothesisVerificationTaskHeaderV1::host_create_dynamic"),
        "new root admission must select the dynamic verification task contract"
    );
    assert!(
        !COMPILER.contains("INSERT INTO hypothesis_verification_task_campaigns(")
            && !COMPILER.contains("INSERT INTO hypothesis_verification_task_assignment_sets("),
        "the compiler must not pre-reserve Campaign or objective assignment authority"
    );
    assert!(
        COMPILER.contains("generation_asset_lane_id")
            || COMPILER.contains("generation.asset_lane_id"),
        "generation admission must compare the task/revision lane with its exact generation lane"
    );
}

#[tokio::test]
#[serial]
async fn fresh_database_installs_asset_hypothesis_lane_authority() {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let mut db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("asset_hypothesis_binding_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .unwrap_or_else(|error| panic!("start isolated migrated postgres: {error:#?}"));

    let installed_columns: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM information_schema.columns
            WHERE column_name='asset_lane_id'
              AND table_name=ANY($1::TEXT[])"#,
    )
    .bind(vec![
        "candidate_analysis_snapshots",
        "candidate_analysis_attempts",
        "investigation_run_work_items",
        "attack_hypotheses",
        "attack_hypothesis_revisions",
        "hypothesis_generations",
        "hypothesis_generation_members",
        "hypothesis_verification_tasks",
        "hypothesis_verification_task_campaigns",
        "verification_campaigns",
        "verification_wave_coverage_denominators",
        "hypothesis_pending_evolution_authorities",
        "investigation_evolution_analysis_primary_rearms",
        "hypothesis_fixed_point_receipts",
    ])
    .fetch_one(db.pool())
    .await
    .expect("count installed asset lane columns");
    assert_eq!(installed_columns, 14);

    let resolver_definition: String = sqlx::query_scalar(
        "SELECT pg_get_functiondef('investigation_resolve_proposal_asset_lane(UUID,TEXT,TEXT)'::REGPROCEDURE)",
    )
    .fetch_one(db.pool())
    .await
    .expect("load installed proposal resolver");
    assert!(resolver_definition.contains("INVESTIGATION_PROPOSAL_ASSET_LANE_REQUIRED"));
    assert!(resolver_definition.contains("endpoint.target_id"));
    assert!(resolver_definition.contains("lane.target_id"));

    let required_constraints: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM pg_constraint constraint_row
            JOIN pg_class table_row ON table_row.oid=constraint_row.conrelid
            WHERE table_row.relname=ANY($1::TEXT[])
              AND contype='c' AND NOT convalidated
              AND pg_get_constraintdef(constraint_row.oid) LIKE '%asset_lane_id IS NOT NULL%'"#,
    )
    .bind(vec![
        "candidate_analysis_snapshots",
        "candidate_analysis_attempts",
        "attack_hypotheses",
        "attack_hypothesis_revisions",
        "hypothesis_generations",
        "hypothesis_generation_members",
        "hypothesis_verification_tasks",
        "hypothesis_verification_task_campaigns",
        "verification_campaigns",
        "verification_wave_coverage_denominators",
        "hypothesis_pending_evolution_authorities",
        "investigation_evolution_analysis_primary_rearms",
        "hypothesis_fixed_point_receipts",
    ])
    .fetch_one(db.pool())
    .await
    .expect("count new-write asset lane requirements");
    assert_eq!(required_constraints, 13);

    let deferred_guards: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM pg_trigger trigger
            JOIN pg_proc function ON function.oid=trigger.tgfoid
            WHERE function.proname='investigation_guard_asset_hypothesis_lane'
              AND tgconstraint <> 0
              AND tgdeferrable AND tginitdeferred"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("count installed deferred lane guards");
    assert_eq!(deferred_guards, 14);

    let dynamic_contract_installed: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM pg_constraint constraint_row
                WHERE constraint_row.conrelid='hypothesis_verification_tasks'::regclass
                  AND pg_get_constraintdef(constraint_row.oid)
                      LIKE '%hypothesis_verification_task.dynamic_v2%'
           )"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("load dynamic task contract constraint");
    assert!(dynamic_contract_installed);
    assert!(DYNAMIC_TASK_ADMISSION_MIGRATION
        .contains("INVESTIGATION_DYNAMIC_TASK_ASSIGNMENT_FORBIDDEN"));
    let read_only_history_triggers: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM pg_trigger trigger_row
             JOIN pg_class table_row ON table_row.oid=trigger_row.tgrelid
            WHERE NOT trigger_row.tgisinternal
              AND table_row.relname=ANY($1::TEXT[])
              AND trigger_row.tgname LIKE '%history_read_on%'"#,
    )
    .bind(vec![
        "hypothesis_verification_task_campaigns",
        "hypothesis_verification_task_assignment_members",
    ])
    .fetch_one(db.pool())
    .await
    .expect("count retained v1 history read-only triggers");
    assert_eq!(read_only_history_triggers, 2);

    let generation_ordinal_unique_columns: Vec<String> = sqlx::query_scalar(
        r#"SELECT array_to_string(
                  ARRAY(
                      SELECT attribute.attname
                        FROM unnest(constraint_row.conkey) WITH ORDINALITY
                             key_column(attnum,ordinality)
                        JOIN pg_attribute attribute
                          ON attribute.attrelid=constraint_row.conrelid
                         AND attribute.attnum=key_column.attnum
                       ORDER BY key_column.ordinality
                  ),
                  ','
              )
             FROM pg_constraint constraint_row
            WHERE constraint_row.conrelid='hypothesis_generations'::regclass
              AND constraint_row.contype='u'
              AND constraint_row.conname LIKE '%generation_ordinal%'
            ORDER BY constraint_row.conname"#,
    )
    .fetch_all(db.pool())
    .await
    .expect("load generation ordinal unique constraints");
    assert_eq!(
        generation_ordinal_unique_columns,
        vec!["asset_lane_id,generation_ordinal"]
    );

    db.stop().await;
}
