const LOGICAL_STRATEGY_MIGRATION: &str =
    include_str!("../migrations/20260813000001_investigation_advisory_logical_strategy_guard.sql");
const SHARED_ACTION_MIGRATION: &str = include_str!(
    "../migrations/20260813000002_investigation_advisory_shared_action_obligations.sql"
);

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
fn advisory_apply_guard_binds_the_compiler_logical_strategy_id() {
    assert!(
        LOGICAL_STRATEGY_MIGRATION.contains("action.private_manifest->>'strategy_decision_id'=(")
    );
    assert!(LOGICAL_STRATEGY_MIGRATION.contains("SELECT strategy.typed_strategy->>'strategy_id'"));
    assert!(!LOGICAL_STRATEGY_MIGRATION.contains(
        "action.private_manifest->>'strategy_decision_id'=\n                   NEW.strategy_artifact_id::TEXT"
    ));
}

#[test]
fn advisory_apply_guard_allows_one_exact_action_to_cover_sibling_obligations() {
    assert!(SHARED_ACTION_MIGRATION.contains(
        "compiled_obligation.obligation_id::TEXT=\n                           action.private_manifest->>'strategy_obligation_id'"
    ));
    assert!(SHARED_ACTION_MIGRATION.contains(
        "compiled_coverage.member_hash=\n                           action.private_manifest->>'coverage_member_hash'"
    ));
    assert!(!SHARED_ACTION_MIGRATION.contains(
        "action.private_manifest->>'strategy_obligation_id'=\n                   NEW.strategy_obligation_id::TEXT"
    ));
}

#[tokio::test]
#[serial]
async fn fresh_database_installs_the_corrected_advisory_apply_guard() {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let mut db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("advisory_strategy_guard_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .unwrap_or_else(|error| panic!("start isolated migrated postgres: {error:#?}"));

    let definition: String = sqlx::query_scalar(
        "SELECT pg_get_functiondef('enforce_investigation_verification_advisory_campaign_apply()'::REGPROCEDURE)",
    )
    .fetch_one(db.pool())
    .await
    .expect("load installed advisory guard");

    let compact = definition.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        compact.contains("strategy.typed_strategy->>'strategy_id'"),
        "installed guard must read the logical strategy id: {compact}"
    );
    assert!(
        !compact.contains("'strategy_decision_id'::text) = (new.strategy_artifact_id)::text"),
        "installed guard still compares the manifest to the artifact id: {compact}"
    );
    assert!(
        compact.contains("compiled_obligation.obligation_id"),
        "installed guard must bind the shared action to its compiled obligation: {compact}"
    );
    assert!(
        compact.contains("compiled_coverage.member_hash"),
        "installed guard must bind the shared action to its compiled coverage: {compact}"
    );

    db.stop().await;
}
