use golish_db::{DbConfig, GolishDb};
use serial_test::serial;
use uuid::Uuid;

const FACT_DELTA_WAVE_ENTRY_MIGRATION: &str =
    include_str!("../migrations/20260712000012_attack_fact_delta_wave_entry.sql");

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read local postgres port")
        .port()
}

#[tokio::test]
#[serial]
async fn fuel_migration_freezes_policy_and_retains_actual_delete_contracts() {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let mut db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("attack_fuel_schema_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start migrated embedded postgres");

    let trigger_names: Vec<String> = sqlx::query_scalar(
        r#"SELECT tgname
             FROM pg_trigger
            WHERE tgrelid='attack_wave_runs'::regclass
              AND NOT tgisinternal
              AND tgname IN (
                  'attack_wave_policy_immutable',
                  'attack_wave_follow_on_policy_exact'
              )
            ORDER BY tgname"#,
    )
    .fetch_all(db.pool())
    .await
    .expect("inspect immutable Wave policy triggers");
    assert_eq!(
        trigger_names,
        vec![
            "attack_wave_follow_on_policy_exact".to_string(),
            "attack_wave_policy_immutable".to_string(),
        ],
        "frozen Wave policy needs both an UPDATE guard and a deferred follow-on exact-copy check"
    );
    let wave_policy_guard: String = sqlx::query_scalar(
        r#"SELECT pg_get_functiondef(
               'enforce_follow_on_attack_wave_policy_exact()'::regprocedure
           )"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect initial/follow-on Wave policy guard");
    for required in [
        "uuid_generate_v5",
        "max_candidates_total <> 100",
        "max_attempts_total <> 200",
        "66e50329b4bb217eb060bcccb38f78f4b0eafc163471bebd6554d271d1a6b326",
    ] {
        assert!(
            wave_policy_guard.contains(required),
            "generation-zero policy guard is missing {required}"
        );
    }

    let target_tuple_triggers: Vec<String> = sqlx::query_scalar(
        r#"SELECT tgname
             FROM pg_trigger
            WHERE NOT tgisinternal
              AND tgname IN (
                  'attack_candidate_approvals_target_tuple_exact',
                  'candidate_attempts_target_tuple_exact'
              )
            ORDER BY tgname"#,
    )
    .fetch_all(db.pool())
    .await
    .expect("inspect exact Candidate target tuple triggers");
    assert_eq!(
        target_tuple_triggers,
        vec![
            "attack_candidate_approvals_target_tuple_exact".to_string(),
            "candidate_attempts_target_tuple_exact".to_string(),
        ],
        "Candidate -> Approval -> Attempt must retain one exact at-time target tuple"
    );

    let delete_guard: String = sqlx::query_scalar(
        r#"SELECT pg_get_functiondef('reject_attack_fuel_ledger_delete()'::regprocedure)"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect fuel ledger delete guard");
    assert!(delete_guard.contains("ATTACK_FUEL_LEDGER_DELETE_REJECTED"));
    assert!(
        !delete_guard.contains("pg_trigger_depth"),
        "the current schema has RESTRICT owners, so fuel deletion must not invent a cascade escape"
    );
    let operation_delete_actions: Vec<String> = sqlx::query_scalar(
        r#"SELECT confdeltype::TEXT
             FROM pg_constraint
            WHERE contype='f'
              AND conrelid='attack_candidates'::regclass
              AND confrelid='operation_state'::regclass
            ORDER BY conname"#,
    )
    .fetch_all(db.pool())
    .await
    .expect("inspect Candidate operation owner delete action");
    assert_eq!(operation_delete_actions, vec!["r".to_string()]);
    let retained_target_delete_actions: Vec<String> = sqlx::query_scalar(
        r#"SELECT conrelid::regclass::TEXT || ':' || confdeltype::TEXT
             FROM pg_constraint
            WHERE contype='f'
              AND confrelid='targets'::regclass
              AND conrelid IN (
                  'attack_candidates'::regclass,
                  'attack_candidate_approvals'::regclass,
                  'candidate_attempts'::regclass,
                  'attack_residual_risks'::regclass
              )
              AND pg_get_constraintdef(oid) LIKE '%target_live_id%'
            ORDER BY conrelid::regclass::TEXT"#,
    )
    .fetch_all(db.pool())
    .await
    .expect("inspect retained live-target delete actions");
    assert_eq!(
        retained_target_delete_actions,
        vec![
            "attack_candidate_approvals:n".to_string(),
            "attack_candidates:n".to_string(),
            "attack_residual_risks:n".to_string(),
            "candidate_attempts:n".to_string(),
        ]
    );

    let migration_validation: String = sqlx::query_scalar(
        r#"SELECT pg_get_functiondef(
               'validate_existing_attack_fuel_state()'::regprocedure
           )"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect migration-time fuel validation");
    for required in [
        "ATTACK_EXISTING_WAVE_POLICY_INVALID",
        "ATTACK_EXISTING_TARGET_TUPLE_INVALID",
        "ATTACK_EXISTING_FUEL_INVALID",
        "retryable_failed",
        "digest",
    ] {
        assert!(
            migration_validation.contains(required),
            "migration-time validation is missing {required}"
        );
    }

    let residual_guard: String = sqlx::query_scalar(
        r#"SELECT pg_get_functiondef(
               'reject_attack_fuel_residual_canonical_change()'::regprocedure
           )"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect immutable fuel residual guard");
    for required in [
        "max_waves",
        "max_candidates_total",
        "max_chain_depth",
        "max_attempts_total",
        "ATTACK_FUEL_RESIDUAL_IMMUTABLE",
        "target_live_id",
    ] {
        assert!(
            residual_guard.contains(required),
            "fuel residual immutable guard is missing {required}"
        );
    }

    let fact_delta_kind_validated: bool = sqlx::query_scalar(
        r#"SELECT convalidated
             FROM pg_constraint
            WHERE conrelid='attack_fact_deltas'::regclass
              AND conname='attack_fact_deltas_delta_kind_closed'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect validated closed FactDelta kind catalog");
    assert!(fact_delta_kind_validated);
    let proposal_gate: String = sqlx::query_scalar(
        r#"SELECT pg_get_functiondef(
               'reject_late_attack_fact_delta_proposal()'::regprocedure
           )"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect source Wave proposal lock");
    for required in ["FOR UPDATE", "verification", "terminal_at IS NULL"] {
        assert!(
            proposal_gate.contains(required),
            "source Wave proposal lock is missing {required}"
        );
    }
    let fact_delta_state_guard: String = sqlx::query_scalar(
        r#"SELECT pg_get_functiondef(
               'enforce_attack_fact_delta_state_transition()'::regprocedure
           )"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect FactDelta transition guard");
    let fact_delta_state_guard = fact_delta_state_guard.to_ascii_lowercase();
    for required in [
        "new.accepted_at := now()",
        "new.consumed_at := now()",
        "consumed FactDelta terminal audit row is immutable",
    ] {
        assert!(
            fact_delta_state_guard.contains(&required.to_ascii_lowercase()),
            "FactDelta transition guard is missing {required}"
        );
    }
    for required in [
        "existing materialized FactDelta lacks one matching immutable decision",
        "VALIDATE CONSTRAINT attack_fact_deltas_delta_kind_closed",
    ] {
        assert!(
            FACT_DELTA_WAVE_ENTRY_MIGRATION.contains(required),
            "FactDelta migration fail-closed scan is missing {required}"
        );
    }

    let audit_triggers: Vec<String> = sqlx::query_scalar(
        r#"SELECT tgname
             FROM pg_trigger
            WHERE NOT tgisinternal
              AND tgname IN (
                  'attack_candidates_canonical_immutable',
                  'attack_candidate_approvals_decision_immutable',
                  'zz_finding_lineage_audit_immutable',
                  'zz_candidate_attempts_audit_transition',
                  'attack_candidate_work_items_decision_immutable',
                  'attack_candidate_evidence_immutable',
                  'candidate_attempt_actions_audit_immutable',
                  'attack_fuel_residual_canonical_immutable'
              )
            ORDER BY tgname"#,
    )
    .fetch_all(db.pool())
    .await
    .expect("inspect Candidate/Attempt audit triggers");
    for required in [
        "attack_candidates_canonical_immutable",
        "attack_candidate_approvals_decision_immutable",
        "zz_finding_lineage_audit_immutable",
        "zz_candidate_attempts_audit_transition",
        "attack_candidate_work_items_decision_immutable",
        "attack_candidate_evidence_immutable",
        "candidate_attempt_actions_audit_immutable",
        "attack_fuel_residual_canonical_immutable",
    ] {
        assert!(
            audit_triggers.iter().any(|name| name == required),
            "missing retained audit trigger {required}"
        );
    }

    db.stop().await;
}
