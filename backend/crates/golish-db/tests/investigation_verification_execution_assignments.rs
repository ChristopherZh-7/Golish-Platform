const MIGRATION: &str = include_str!(
    "../migrations/20260812000003_investigation_verification_execution_assignments.sql"
);
const ASSET_BINDING_MIGRATION: &str = include_str!(
    "../migrations/20260813000007_investigation_verification_execution_asset_tools.sql"
);
const STAGE_TEAM_REPOSITORY: &str = include_str!("../src/repo/stage_teams.rs");
const RUNTIME_MEMORY_TX_REPOSITORY: &str = include_str!("../src/repo/runtime_memory_tx.rs");
const UNIFIED_RUNTIME_REPOSITORY: &str =
    include_str!("../src/repo/unified_investigation_runtime.rs");

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

async fn migrated_db() -> (GolishDb, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("verification_assignment_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .unwrap_or_else(|error| panic!("start isolated migrated postgres: {error:#?}"));
    (db, data_dir)
}

fn migration_has(parts: &[&str]) {
    for part in parts {
        assert!(MIGRATION.contains(part), "migration is missing `{part}`");
    }
}

#[test]
fn immutable_assignment_freezes_the_complete_execution_authority() {
    migration_has(&[
        "verification_task_id UUID NOT NULL",
        "task_plan_id UUID NOT NULL",
        "operation_id UUID NOT NULL",
        "stage_execution_id UUID NOT NULL",
        "stage_run_unit_id UUID NOT NULL",
        "scope_snapshot_id UUID NOT NULL",
        "organization_id UUID NOT NULL",
        "hypothesis_revision_id UUID NOT NULL",
        "campaign_id UUID NOT NULL",
        "plan_objective_id UUID NOT NULL",
        "verification_objective_id UUID NOT NULL",
        "prepared_action_id UUID NOT NULL",
        "action_execution_id UUID NOT NULL UNIQUE",
        "authorization_receipt_id UUID NOT NULL",
        "budget_reservation_id UUID NOT NULL",
        "conflict_set_id UUID NOT NULL",
        "stage_worker_request_id UUID NOT NULL UNIQUE",
        "execution_work_item_id UUID NOT NULL UNIQUE",
        "allowed_tool_names JSONB NOT NULL",
        "canonical_args JSONB NOT NULL",
        "evidence_contract_sha256 TEXT NOT NULL",
        "oracle_contract_sha256 TEXT NOT NULL",
        "assignment_authority_sha256 TEXT NOT NULL",
        "investigation_verification_execution_assignments_append_only",
    ]);
}

#[test]
fn materialization_fails_closed_without_exact_jit_scope_and_worker_authority() {
    migration_has(&[
        "authorization.expires_at<=statement_timestamp()",
        "authorization.campaign_dispatch_generation<>execution.campaign_dispatch_generation",
        "task_plan.subject_kind<>'verification_task'",
        "task_plan.subject_id<>NEW.verification_task_id",
        "task_plan.status<>'sealed'",
        "task_plan.sealed_at IS NULL",
        "worker_request.status<>'accepted'",
        "worker_request.accepted_work_item_id<>NEW.execution_work_item_id",
        "worker_request.request_kind<>'investigation_verification_execution'",
        "work_item.created_by<>'accepted_worker_request'",
        "INVESTIGATION_EXECUTION_ASSIGNMENT_FOREIGN_SCOPE",
        "NEW.canonical_args IS DISTINCT FROM action.private_manifest",
        "INVESTIGATION_EXECUTION_ASSIGNMENT_MANIFEST_DRIFT",
    ]);
}

#[test]
fn execution_assignment_requires_the_exact_asset_lane_and_live_target() {
    for witness in [
        "asset_lane_id UUID",
        "target_live_id UUID",
        "investigation_verification_execution_assignments_asset_lane_required",
        "investigation_verification_execution_assignments_target_required",
        "INVESTIGATION_EXECUTION_ASSIGNMENT_ASSET_LANE_DRIFT",
        "task.asset_lane_id",
        "campaign.asset_lane_id",
        "revision.asset_lane_id",
        "revision.target_live_id",
        "action.target_live_id",
        "lane.target_id",
    ] {
        assert!(
            ASSET_BINDING_MIGRATION.contains(witness),
            "asset-bound execution migration lost `{witness}`"
        );
    }
    for witness in [
        "pub asset_lane_id: Uuid",
        "pub target_live_id: Uuid",
        "task.asset_lane_id",
        "action.target_live_id",
        "assignment.asset_lane_id",
        "assignment.target_live_id",
    ] {
        assert!(
            UNIFIED_RUNTIME_REPOSITORY.contains(witness),
            "runtime assignment propagation lost `{witness}`"
        );
    }
}

#[test]
fn lease_loss_and_recovery_are_durable_without_external_io_replay() {
    migration_has(&[
        "investigation_verification_execution_assignment_heads",
        "active_tool_call_request_id UUID",
        "recovery_required",
        "investigation_verification_execution_assignment_events",
        "investigation_verification_execution_assignment_recoveries",
        "recovery_authority_sha256",
        "outcome_unknown",
    ]);
    assert!(!MIGRATION.contains("http://"));
    assert!(!MIGRATION.contains("https://"));
}

#[test]
fn execution_worker_output_is_admitted_only_from_the_terminal_assignment_head() {
    for witness in [
        "investigation_verification_execution_output.v1",
        "assignment.execution_work_item_id=$2",
        "head.worker_run_id=$7",
        "head.capability_execution_receipt_id",
        "head.evidence_ids",
        "head.evidence_set_sha256",
        "head.oracle_receipt_id",
        "head.oracle_receipt_sha256",
        "head.terminal_authority_sha256",
        "hash != output_evidence_set_sha256",
        "hash != output_oracle_receipt_sha256",
        "hash != output_terminal_authority_sha256",
        "investigation_verification_execution_output_authority_mismatch",
    ] {
        assert!(
            STAGE_TEAM_REPOSITORY.contains(witness),
            "execution output admission lost `{witness}`"
        );
    }
}

#[test]
fn request_schema_exception_is_bounded_to_the_exact_verification_primary() {
    for witness in [
        "exact_task_primary_coordinator",
        "input.requested_kind == \"investigation_verification_execution\"",
        "investigation_verification_execution_output.v1",
        "investigation_verification_execution_output_schema_invalid",
    ] {
        assert!(
            RUNTIME_MEMORY_TX_REPOSITORY.contains(witness),
            "execution WorkItem schema admission lost `{witness}`"
        );
    }
}

#[test]
fn execution_primary_rearm_preserves_history_without_dual_chain_ownership() {
    migration_has(&[
        "investigation_verification_execution_primary_rearms",
        "cognitive_primary_worker_run_id UUID NOT NULL",
        "primary_message_chain_id UUID NOT NULL",
        "execution_primary_message_chain_id UUID NOT NULL UNIQUE",
        "INVESTIGATION_VERIFICATION_EXECUTION_PRIMARY_REARM_AUTHORITY_MISMATCH",
        "investigation_execution_primary_rearm_advance",
    ]);
    for witness in [
        "investigation-verification-execution-primary-chain-v1",
        "SELECT session_id,agent,model,provider,chain FROM message_chains",
        "investigation_verification_execution_primary_rearm_apply_failed",
    ] {
        assert!(
            RUNTIME_MEMORY_TX_REPOSITORY.contains(witness),
            "execution Primary rearm lost `{witness}`"
        );
    }
}

#[tokio::test]
#[serial]
async fn additive_migration_installs_the_assignment_authority_spine() {
    let (db, _data_dir) = migrated_db().await;
    let applied: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM _sqlx_migrations WHERE version=$1 AND success)",
    )
    .bind(20260812000003_i64)
    .fetch_one(db.pool())
    .await
    .expect("read assignment migration ledger");
    assert!(applied);
    let asset_binding_applied: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM _sqlx_migrations WHERE version=$1 AND success)",
    )
    .bind(20260813000007_i64)
    .fetch_one(db.pool())
    .await
    .expect("read assignment asset-binding migration ledger");
    assert!(asset_binding_applied);
    let relation_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pg_class
          WHERE relname = ANY($1::TEXT[]) AND relkind='r'",
    )
    .bind(vec![
        "investigation_verification_execution_assignments",
        "investigation_verification_execution_assignment_heads",
        "investigation_verification_execution_assignment_events",
        "investigation_verification_execution_assignment_recoveries",
        "investigation_verification_execution_primary_rearms",
    ])
    .fetch_one(db.pool())
    .await
    .expect("inspect installed assignment relations");
    assert_eq!(relation_count, 5);
}
