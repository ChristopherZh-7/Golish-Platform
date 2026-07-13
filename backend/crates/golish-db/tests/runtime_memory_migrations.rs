use golish_db::{embedded::EmbeddedPg, repo, DbConfig, GolishDb};
use serde_json::json;
use serial_test::serial;
use sha2::{Digest, Sha256, Sha384};
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::Row;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::{oneshot, Barrier};
use tokio::time::timeout;
use uuid::Uuid;

const LEGACY_SCHEMA: &str = include_str!("fixtures/runtime_memory_legacy_schema.sql");
const FOUNDATION_MIGRATION: &str =
    include_str!("../migrations/20260712000001_runtime_memory_foundation.sql");
const FROZEN_FOUNDATION_SHA384: &str =
    "ffda87b53920699abfdd8fe4a985e76f7624ad918498d14ab7dba5a17c1e17dcba7d1cc760c007f7328d1725b6047ba4";

const REQUIRED_RUNTIME_TABLES: &[&str] = &[
    "runtime_memory_rollout",
    "project_scopes",
    "operation_scope_decisions",
    "operation_org_scope_snapshots",
    "operation_org_scope_units",
    "stage_run_units",
    "stage_worker_runs",
    "stage_deliverable_submissions",
    "stage_handoffs",
];

#[test]
fn runtime_memory_foundation_checksum_is_frozen_after_hostile_schema_audit() {
    let digest = Sha384::digest(FOUNDATION_MIGRATION.as_bytes());
    let actual = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(actual, FROZEN_FOUNDATION_SHA384);
}

fn reserve_local_port() -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

fn fixture_id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn assert_sqlstate<T>(result: Result<T, sqlx::Error>, expected: &str, context: &str) {
    let error = match result {
        Ok(_) => panic!("{context} must fail"),
        Err(error) => error,
    };
    let sqlstate = match &error {
        sqlx::Error::Database(database_error) => {
            database_error.code().map(|code| code.into_owned())
        }
        _ => None,
    };
    assert_eq!(
        sqlstate.as_deref(),
        Some(expected),
        "{context} must fail with SQLSTATE {expected}, got {error}"
    );
}

async fn wait_until_backend_is_blocked(
    lock_holder: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    blocker_pid: i32,
    blocked_pid: i32,
) -> bool {
    timeout(Duration::from_secs(5), async {
        loop {
            let is_blocked: bool = sqlx::query_scalar("SELECT $1 = ANY(pg_blocking_pids($2))")
                .bind(blocker_pid)
                .bind(blocked_pid)
                .fetch_one(&mut **lock_holder)
                .await
                .expect("observe PostgreSQL lock waiter");
            if is_blocked {
                return true;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or(false)
}

struct LegacyPg {
    embedded: EmbeddedPg,
    pool: PgPool,
    _data_dir: TempDir,
}

impl LegacyPg {
    async fn start(label: &str) -> Self {
        let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
        let config = DbConfig {
            pg_data_dir: data_dir.path().join("pgdata"),
            port: reserve_local_port().expect("available postgres port"),
            database: format!("runtime_memory_{label}_{}", Uuid::new_v4().simple()),
            ..DbConfig::default()
        };
        let connection_string = config.connection_string();
        let embedded = EmbeddedPg::start(config)
            .await
            .expect("start legacy embedded postgres");
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(&connection_string)
            .await
            .expect("connect to legacy embedded postgres");
        sqlx::raw_sql(LEGACY_SCHEMA)
            .execute(&pool)
            .await
            .expect("install legacy runtime-memory fixture schema");
        Self {
            embedded,
            pool,
            _data_dir: data_dir,
        }
    }

    async fn try_apply_foundation(&self) -> Result<(), sqlx::Error> {
        sqlx::raw_sql(FOUNDATION_MIGRATION)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn apply_foundation(&self) {
        self.try_apply_foundation()
            .await
            .expect("apply runtime-memory foundation migration");
    }

    async fn stop(mut self) {
        self.pool.close().await;
        self.embedded.stop().await;
    }
}

#[tokio::test]
#[serial]
async fn runtime_memory_contract_is_frozen_and_database_constrained() {
    let data = tempfile::tempdir().expect("temporary postgres data directory");
    let config = DbConfig {
        pg_data_dir: data.path().join("pgdata"),
        port: reserve_local_port().expect("available postgres port"),
        database: format!("runtime_memory_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    };

    let mut db = GolishDb::start(config)
        .await
        .expect("start migrated embedded postgres");
    let pool = db.pool();

    let runtime_rollout: (String, i16, i64) = sqlx::query_as(
        "SELECT contract, contract_rank, row_version FROM runtime_memory_rollout WHERE singleton_id=1",
    )
    .fetch_one(pool)
    .await
    .expect("read cutover runtime rollout");
    assert_eq!(
        runtime_rollout,
        ("dual_write_legacy_read".to_string(), 1, 1),
        "fresh migrations enable sampling but cannot self-attest live parity"
    );
    let attack_rollout: (String, i16, i64) = sqlx::query_as(
        "SELECT contract, rank, row_version FROM attack_execution_rollout WHERE singleton=TRUE",
    )
    .fetch_one(pool)
    .await
    .expect("read cutover attack rollout");
    assert_eq!(
        attack_rollout,
        ("dual_write_read_legacy".to_string(), 1, 1),
        "fresh migrations keep Candidate reads on the retained legacy contract"
    );

    let legacy_operation = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO operation_state (operation_id, profile, current_stage) VALUES ($1, $2, $3)",
    )
    .bind(legacy_operation)
    .bind("assessment")
    .bind("scoping")
    .execute(pool)
    .await
    .expect("insert legacy-shaped operation");
    let legacy_contract: String =
        sqlx::query("SELECT runtime_memory_contract FROM operation_state WHERE operation_id = $1")
            .bind(legacy_operation)
            .fetch_one(pool)
            .await
            .expect("read legacy contract default")
            .get("runtime_memory_contract");
    assert_eq!(legacy_contract, "legacy_v1");

    for contract in [
        "legacy_v1",
        "dual_write_legacy_read",
        "dual_write_v2_preferred",
        "v2_only",
    ] {
        let operation_id = Uuid::new_v4();
        let attack_contract = if contract == "v2_only" {
            "v2_only"
        } else {
            "legacy"
        };
        sqlx::query(
            r#"INSERT INTO operation_state
               (operation_id, profile, current_stage, runtime_memory_contract,
                attack_execution_contract)
               VALUES ($1, 'assessment', 'scoping', $2, $3)"#,
        )
        .bind(operation_id)
        .bind(contract)
        .bind(attack_contract)
        .execute(pool)
        .await
        .expect("insert supported compatible frozen contracts");
        let row = repo::operation_state::get(pool, operation_id)
            .await
            .expect("read operation")
            .expect("operation exists");
        assert_eq!(row.runtime_memory_contract, contract);
    }

    let invalid = sqlx::query(
        r#"INSERT INTO operation_state
           (operation_id, profile, current_stage, runtime_memory_contract,
            attack_execution_contract)
           VALUES ($1, 'assessment', 'scoping', 'legacy_read_v2_write', 'legacy')"#,
    )
    .bind(Uuid::new_v4())
    .execute(pool)
    .await;
    assert!(
        invalid.is_err(),
        "unsafe independent read/write pair must fail"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn foundation_migrates_empty_and_legacy_rows_without_rewriting_checkpoint() {
    for with_legacy_rows in [false, true] {
        let pg = LegacyPg::start(if with_legacy_rows { "legacy" } else { "empty" }).await;
        let operation_id = fixture_id(0x1001);
        let legacy_blob = json!({
            "current_stage_run_id": fixture_id(0x1002),
            "stage_run_workers": {
                "enumeration": {
                    fixture_id(0x1003).to_string(): {
                        "chain_id": fixture_id(0x1004),
                        "specialist": "enumerator"
                    }
                }
            }
        });
        if with_legacy_rows {
            let session_id = fixture_id(0x1000);
            sqlx::query(
                "INSERT INTO sessions (id, title, status) VALUES ($1, 'legacy', 'running')",
            )
            .bind(session_id)
            .execute(&pg.pool)
            .await
            .expect("insert legacy session");
            sqlx::query(
                r#"INSERT INTO tasks (id, session_id, title, input, status)
                   VALUES ($1, $2, 'legacy task', 'resume', 'running')"#,
            )
            .bind(operation_id)
            .bind(session_id)
            .execute(&pg.pool)
            .await
            .expect("insert legacy task");
            sqlx::query(
                r#"INSERT INTO operation_state
                   (operation_id, profile, current_stage, state_blob)
                   VALUES ($1, 'red_team', 'enumeration', $2)"#,
            )
            .bind(operation_id)
            .bind(&legacy_blob)
            .execute(&pg.pool)
            .await
            .expect("insert legacy operation checkpoint");
            sqlx::query(
                "INSERT INTO stage_runs (id, operation_id, stage_kind) VALUES ($1, $2, 'enumeration')",
            )
            .bind(fixture_id(0x1002))
            .bind(operation_id)
            .execute(&pg.pool)
            .await
            .expect("insert legacy stage run");
            sqlx::query(
                r#"INSERT INTO message_chains
                   (id, session_id, task_id, agent, chain)
                   VALUES ($1, $2, $3, 'pentester', $4)"#,
            )
            .bind(fixture_id(0x1004))
            .bind(session_id)
            .bind(operation_id)
            .bind(json!([{"role": "user", "content": "resume"}]))
            .execute(&pg.pool)
            .await
            .expect("insert legacy message chain");
        }

        pg.apply_foundation().await;

        for table in REQUIRED_RUNTIME_TABLES {
            let exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
                .bind(format!("public.{table}"))
                .fetch_one(&pg.pool)
                .await
                .expect("lookup migrated table");
            assert!(exists, "missing runtime-memory table {table}");
        }
        for column in ["runtime_memory_contract", "project_scope_id"] {
            let exists: bool = sqlx::query_scalar(
                r#"SELECT EXISTS (
                       SELECT 1 FROM information_schema.columns
                       WHERE table_schema='public' AND table_name='operation_state'
                         AND column_name=$1
                   )"#,
            )
            .bind(column)
            .fetch_one(&pg.pool)
            .await
            .expect("lookup operation_state column");
            assert!(exists, "missing operation_state.{column}");
        }
        for column in [
            "operation_id",
            "stage_execution_id",
            "stage_run_unit_id",
            "worker_run_id",
            "organization_id",
            "attempt_epoch",
            "lease_token",
        ] {
            let exists: bool = sqlx::query_scalar(
                r#"SELECT EXISTS (
                       SELECT 1 FROM information_schema.columns
                       WHERE table_schema='public' AND table_name='tool_calls'
                         AND column_name=$1
                   )"#,
            )
            .bind(column)
            .fetch_one(&pg.pool)
            .await
            .expect("lookup tool_calls column");
            assert!(exists, "missing tool_calls.{column}");
        }
        let rollout: (String, i16, i64) = sqlx::query_as(
            "SELECT contract, contract_rank, row_version FROM runtime_memory_rollout WHERE singleton_id=1",
        )
        .fetch_one(&pg.pool)
        .await
        .expect("read rollout singleton");
        assert_eq!(rollout, ("legacy_v1".to_string(), 0, 0));

        if with_legacy_rows {
            let migrated: (String, serde_json::Value, Option<Uuid>) = sqlx::query_as(
                r#"SELECT runtime_memory_contract, state_blob, project_scope_id
                   FROM operation_state WHERE operation_id=$1"#,
            )
            .bind(operation_id)
            .fetch_one(&pg.pool)
            .await
            .expect("read migrated legacy operation");
            assert_eq!(migrated.0, "legacy_v1");
            assert_eq!(migrated.1, legacy_blob);
            assert_eq!(migrated.2, None);
        }
        pg.stop().await;
    }
}

async fn advance_rollout(
    pool: &PgPool,
    expected_contract: &str,
    next_contract: &str,
    next_rank: i16,
    expected_version: i64,
) -> Option<(String, i16, i64)> {
    sqlx::query_as(
        r#"UPDATE runtime_memory_rollout
           SET contract=$2, contract_rank=$3, row_version=row_version+1, updated_at=NOW()
           WHERE singleton_id=1 AND contract=$1 AND contract_rank+1=$3 AND row_version=$4
           RETURNING contract, contract_rank, row_version"#,
    )
    .bind(expected_contract)
    .bind(next_contract)
    .bind(next_rank)
    .bind(expected_version)
    .fetch_optional(pool)
    .await
    .expect("advance rollout with repository contract SQL")
}

#[tokio::test]
#[serial]
async fn operation_contract_is_immutable_and_rollout_advances_one_state_at_a_time() {
    let pg = LegacyPg::start("contract").await;
    pg.apply_foundation().await;
    let operation_id = fixture_id(0x2001);
    sqlx::query(
        r#"INSERT INTO operation_state
           (operation_id, profile, current_stage, runtime_memory_contract)
           VALUES ($1, 'red_team', 'scoping', 'legacy_v1')"#,
    )
    .bind(operation_id)
    .execute(&pg.pool)
    .await
    .expect("insert frozen operation contract");

    let changed = sqlx::query(
        "UPDATE operation_state SET runtime_memory_contract='v2_only' WHERE operation_id=$1",
    )
    .bind(operation_id)
    .execute(&pg.pool)
    .await;
    assert!(
        changed.is_err(),
        "an in-flight operation contract must be immutable"
    );

    assert_eq!(
        advance_rollout(&pg.pool, "legacy_v1", "v2_only", 3, 0).await,
        None,
        "rollout must reject skipped states"
    );
    assert_eq!(
        advance_rollout(&pg.pool, "legacy_v1", "dual_write_legacy_read", 1, 0,).await,
        Some(("dual_write_legacy_read".to_string(), 1, 1))
    );
    assert_eq!(
        advance_rollout(
            &pg.pool,
            "dual_write_legacy_read",
            "dual_write_v2_preferred",
            2,
            0,
        )
        .await,
        None,
        "stale row_version must not advance rollout"
    );
    pg.stop().await;
}

#[derive(Debug, Clone, Copy)]
struct RuntimeGraphIds {
    project_a: Uuid,
    project_b: Uuid,
    session_a: Uuid,
    session_b: Uuid,
    operation_a: Uuid,
    operation_b: Uuid,
    stage_a: Uuid,
    stage_b: Uuid,
    decision_a: Uuid,
    snapshot_a: Uuid,
    snapshot_b: Uuid,
    organization_a: Uuid,
    organization_b: Uuid,
    organization_child_a: Uuid,
    unit_a: Uuid,
    unit_b: Uuid,
    tool_call_a: Uuid,
    submission_a: Uuid,
    chain_a: Uuid,
    chain_b: Uuid,
    worker_a: Uuid,
    lease_a: Uuid,
}

#[derive(Debug, Clone, Copy)]
struct ScopeHeadIds {
    project: Uuid,
    operation: Uuid,
    stage: Uuid,
    decision: Uuid,
    snapshot: Uuid,
    root_organization: Uuid,
}

async fn seed_scope_head(pool: &PgPool, base: u128) -> ScopeHeadIds {
    let ids = ScopeHeadIds {
        project: fixture_id(base + 1),
        operation: fixture_id(base + 2),
        stage: fixture_id(base + 3),
        decision: fixture_id(base + 4),
        snapshot: fixture_id(base + 5),
        root_organization: fixture_id(base + 6),
    };
    sqlx::query(
        r#"INSERT INTO project_scopes
           (project_scope_id, canonical_project_path, path_sha256)
           VALUES ($1, $2, $3)"#,
    )
    .bind(ids.project)
    .bind(format!("/fixture/scope-{base:x}"))
    .bind(format!("{:064x}", ids.project.as_u128()))
    .execute(pool)
    .await
    .expect("insert standalone project scope");
    sqlx::query(
        r#"INSERT INTO operation_state
           (operation_id, profile, current_stage, runtime_memory_contract, project_scope_id)
           VALUES ($1, 'red_team', 'scoping', 'dual_write_legacy_read', $2)"#,
    )
    .bind(ids.operation)
    .bind(ids.project)
    .execute(pool)
    .await
    .expect("insert standalone operation");
    sqlx::query("INSERT INTO stage_runs (id, operation_id, stage_kind) VALUES ($1, $2, 'scoping')")
        .bind(ids.stage)
        .bind(ids.operation)
        .execute(pool)
        .await
        .expect("insert standalone scoping execution");
    sqlx::query(
        r#"INSERT INTO operation_scope_decisions
           (id, operation_id, project_scope_id, stage_execution_id,
            root_organization_id, mode, decision_rows, decision_hash)
           VALUES ($1, $2, $3, $4, $5, 'root_only', '[]'::jsonb, $6)"#,
    )
    .bind(ids.decision)
    .bind(ids.operation)
    .bind(ids.project)
    .bind(ids.stage)
    .bind(ids.root_organization)
    .bind(format!("decision-{:x}", ids.decision.as_u128()))
    .execute(pool)
    .await
    .expect("insert standalone scope decision");
    ids
}

async fn seed_runtime_graph(pool: &PgPool) -> RuntimeGraphIds {
    let project_a = fixture_id(0x3001);
    let project_b = fixture_id(0x3002);
    let session_a = fixture_id(0x3003);
    let session_b = fixture_id(0x3004);
    let operation_a = fixture_id(0x3011);
    let operation_b = fixture_id(0x3012);
    let stage_a = fixture_id(0x3021);
    let stage_b = fixture_id(0x3022);
    let decision_a = fixture_id(0x3031);
    let decision_b = fixture_id(0x3032);
    let snapshot_a = fixture_id(0x3041);
    let snapshot_b = fixture_id(0x3042);
    let organization_a = fixture_id(0x3051);
    let organization_b = fixture_id(0x3052);
    let organization_child_a = fixture_id(0x3053);
    let unit_a = fixture_id(0x3061);
    let unit_b = fixture_id(0x3062);
    let tool_call_a = fixture_id(0x3071);
    let submission_a = fixture_id(0x3081);
    let chain_a = fixture_id(0x3091);
    let chain_b = fixture_id(0x3092);
    let worker_a = fixture_id(0x30a1);
    let lease_a = fixture_id(0x30a2);

    for (session, title) in [(session_a, "Session A"), (session_b, "Session B")] {
        sqlx::query("INSERT INTO sessions (id, title, status) VALUES ($1, $2, 'running')")
            .bind(session)
            .bind(title)
            .execute(pool)
            .await
            .expect("insert runtime session");
    }
    for (operation, session, title) in [
        (operation_a, session_a, "Task A"),
        (operation_b, session_b, "Task B"),
    ] {
        sqlx::query(
            r#"INSERT INTO tasks (id, session_id, title, input, status)
               VALUES ($1, $2, $3, 'runtime fixture', 'running')"#,
        )
        .bind(operation)
        .bind(session)
        .bind(title)
        .execute(pool)
        .await
        .expect("insert runtime task");
    }

    for (project, path) in [(project_a, "/fixture/a"), (project_b, "/fixture/b")] {
        sqlx::query(
            r#"INSERT INTO project_scopes
               (project_scope_id, canonical_project_path, path_sha256)
               VALUES ($1, $2, $3)"#,
        )
        .bind(project)
        .bind(path)
        .bind(format!("{:064x}", project.as_u128()))
        .execute(pool)
        .await
        .expect("insert project scope");
    }
    for (operation, project) in [(operation_a, project_a), (operation_b, project_b)] {
        sqlx::query(
            r#"INSERT INTO operation_state
               (operation_id, profile, current_stage, runtime_memory_contract, project_scope_id)
               VALUES ($1, 'red_team', 'target_intel', 'dual_write_legacy_read', $2)"#,
        )
        .bind(operation)
        .bind(project)
        .execute(pool)
        .await
        .expect("insert runtime operation");
    }
    for (stage, operation) in [(stage_a, operation_a), (stage_b, operation_b)] {
        sqlx::query(
            "INSERT INTO stage_runs (id, operation_id, stage_kind) VALUES ($1, $2, 'target_intel')",
        )
        .bind(stage)
        .bind(operation)
        .execute(pool)
        .await
        .expect("insert stage execution");
    }
    for (decision, operation, project, stage, organization) in [
        (decision_a, operation_a, project_a, stage_a, organization_a),
        (decision_b, operation_b, project_b, stage_b, organization_b),
    ] {
        sqlx::query(
            r#"INSERT INTO operation_scope_decisions
               (id, operation_id, project_scope_id, stage_execution_id,
                root_organization_id, mode,
                decision_rows, decision_hash)
               VALUES ($1, $2, $3, $4, $5, 'root_only', '[]'::jsonb, $6)"#,
        )
        .bind(decision)
        .bind(operation)
        .bind(project)
        .bind(stage)
        .bind(organization)
        .bind(format!("decision-{decision}"))
        .execute(pool)
        .await
        .expect("insert scope decision");
    }
    for (snapshot, operation, project, decision, organization, path) in [
        (
            snapshot_a,
            operation_a,
            project_a,
            decision_a,
            organization_a,
            "/fixture/a",
        ),
        (
            snapshot_b,
            operation_b,
            project_b,
            decision_b,
            organization_b,
            "/fixture/b",
        ),
    ] {
        let mut tx = pool.begin().await.expect("begin frozen scope transaction");
        sqlx::query(
            r#"INSERT INTO operation_org_scope_snapshots
               (id, operation_id, project_scope_id, scope_decision_id,
                project_path_at_freeze, root_organization_id, mode, scope_hash)
               VALUES ($1, $2, $3, $4, $5, $6, 'root_only', $7)"#,
        )
        .bind(snapshot)
        .bind(operation)
        .bind(project)
        .bind(decision)
        .bind(path)
        .bind(organization)
        .bind(format!("scope-{snapshot}"))
        .execute(&mut *tx)
        .await
        .expect("insert scope snapshot");
        sqlx::query(
            r#"INSERT INTO operation_org_scope_units
               (snapshot_id, organization_id, organization_name_at_freeze, role,
                depth, ordinal, decision_row_id, approval_source)
               VALUES ($1, $2, $3, 'root', 0, 0, $4, '{}'::jsonb)"#,
        )
        .bind(snapshot)
        .bind(organization)
        .bind(format!("Org-{organization}"))
        .bind(format!("row-{organization}"))
        .execute(&mut *tx)
        .await
        .expect("insert scope unit");
        if snapshot == snapshot_a {
            sqlx::query(
                r#"INSERT INTO operation_org_scope_units
                   (snapshot_id, organization_id, parent_organization_id,
                    organization_name_at_freeze, role, depth, ordinal,
                    decision_row_id, approval_source)
                   VALUES ($1, $2, $3, 'Child A', 'subsidiary', 1, 1,
                           'row-child-a', '{}'::jsonb)"#,
            )
            .bind(snapshot_a)
            .bind(organization_child_a)
            .bind(organization_a)
            .execute(&mut *tx)
            .await
            .expect("insert child before scope seal");
        }
        sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
            .bind(snapshot)
            .execute(&mut *tx)
            .await
            .expect("seal immutable scope snapshot");
        tx.commit().await.expect("commit frozen scope transaction");
    }
    for (unit, operation, stage, snapshot, organization) in [
        (unit_a, operation_a, stage_a, snapshot_a, organization_a),
        (unit_b, operation_b, stage_b, snapshot_b, organization_b),
    ] {
        sqlx::query(
            r#"INSERT INTO stage_run_units
               (id, operation_id, stage_execution_id, scope_snapshot_id,
                organization_id, stage_kind, generation, specialist, status)
               VALUES ($1, $2, $3, $4, $5, 'target_intel', 0, 'recon', 'running')"#,
        )
        .bind(unit)
        .bind(operation)
        .bind(stage)
        .bind(snapshot)
        .bind(organization)
        .execute(pool)
        .await
        .expect("insert stage unit");
    }
    for (chain, session, operation) in [
        (chain_a, session_a, operation_a),
        (chain_b, session_b, operation_b),
    ] {
        sqlx::query(
            r#"INSERT INTO message_chains (id, session_id, task_id, agent, chain)
               VALUES ($1, $2, $3, 'pentester', '[]'::jsonb)"#,
        )
        .bind(chain)
        .bind(session)
        .bind(operation)
        .execute(pool)
        .await
        .expect("insert bound chain");
    }
    sqlx::query(
        r#"INSERT INTO stage_worker_runs
           (id, operation_id, stage_execution_id, stage_run_unit_id,
            organization_id, worker_generation, specialist, work_item_kind,
            work_item_key, agent_path, message_chain_id, status, lease_token,
            lease_owner, lease_acquired_at, lease_expires_at, attempt_epoch)
           VALUES ($1, $2, $3, $4, $5, 0, 'recon', 'org', 'root', 'main>recon',
                   $6, 'running', $7, 'worker-a', NOW(),
                   NOW() + INTERVAL '30 seconds', 1)"#,
    )
    .bind(worker_a)
    .bind(operation_a)
    .bind(stage_a)
    .bind(unit_a)
    .bind(organization_a)
    .bind(chain_a)
    .bind(lease_a)
    .execute(pool)
    .await
    .expect("insert first worker chain owner");
    sqlx::query(
        r#"INSERT INTO tool_calls
           (id, call_id, session_id, task_id, name, operation_id,
            stage_execution_id, stage_run_unit_id, worker_run_id,
            organization_id, attempt_epoch, lease_token)
           VALUES ($1, 'submit-a', $2, $3, 'submit_stage_deliverable',
                   $3, $4, $5, $6, $7, 1, $8)"#,
    )
    .bind(tool_call_a)
    .bind(session_a)
    .bind(operation_a)
    .bind(stage_a)
    .bind(unit_a)
    .bind(worker_a)
    .bind(organization_a)
    .bind(lease_a)
    .execute(pool)
    .await
    .expect("insert trusted tool call");
    sqlx::query(
        r#"INSERT INTO stage_deliverable_submissions
           (id, operation_id, stage_execution_id, stage_run_unit_id,
            worker_run_id, organization_id, tool_call_record_id,
            tool_request_id, stage_kind, attempt_epoch, lease_token,
            payload, payload_sha256)
           VALUES ($1, $2, $3, $4, $5, $6, $7, 'submit-a',
                   'target_intel', 1, $8, '{}'::jsonb, 'submission-a')"#,
    )
    .bind(submission_a)
    .bind(operation_a)
    .bind(stage_a)
    .bind(unit_a)
    .bind(worker_a)
    .bind(organization_a)
    .bind(tool_call_a)
    .bind(lease_a)
    .execute(pool)
    .await
    .expect("insert trusted submission");

    RuntimeGraphIds {
        project_a,
        project_b,
        session_a,
        session_b,
        operation_a,
        operation_b,
        stage_a,
        stage_b,
        decision_a,
        snapshot_a,
        snapshot_b,
        organization_a,
        organization_b,
        organization_child_a,
        unit_a,
        unit_b,
        tool_call_a,
        submission_a,
        chain_a,
        chain_b,
        worker_a,
        lease_a,
    }
}

#[tokio::test]
#[serial]
async fn stage_deliverable_submission_repo_roundtrips_trusted_runtime_identity() {
    let pg = LegacyPg::start("submission-repo-roundtrip").await;
    pg.apply_foundation().await;
    let ids = seed_runtime_graph(&pg.pool).await;
    sqlx::query("UPDATE tool_calls SET status='finished' WHERE id=$1")
        .bind(ids.tool_call_a)
        .execute(&pg.pool)
        .await
        .expect("finish seeded submission tool call");

    let tool_call_record_id = fixture_id(0x30b1);
    let tool_request_id = "trusted-submission-roundtrip";
    sqlx::query(
        r#"INSERT INTO tool_calls
           (id, call_id, session_id, task_id, name, status, operation_id,
            stage_execution_id, stage_run_unit_id, worker_run_id,
            organization_id, attempt_epoch, lease_token)
           VALUES ($1, $2, $3, $4, 'submit_stage_deliverable', 'running',
                   $4, $5, $6, $7, $8, 1, $9)"#,
    )
    .bind(tool_call_record_id)
    .bind(tool_request_id)
    .bind(ids.session_a)
    .bind(ids.operation_a)
    .bind(ids.stage_a)
    .bind(ids.unit_a)
    .bind(ids.worker_a)
    .bind(ids.organization_a)
    .bind(ids.lease_a)
    .execute(&pg.pool)
    .await
    .expect("insert trusted submission tool call");

    let canonical_payload = format!(
        r#"{{"claims":[],"stage_id":"target_intel","stage_run_id":"{}"}}"#,
        ids.stage_a
    );
    let payload_sha256 = Sha256::digest(canonical_payload.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let input = repo::stage_deliverable_submissions::NewStageDeliverableSubmission {
        operation_id: ids.operation_a,
        stage_execution_id: ids.stage_a,
        stage_run_unit_id: Some(ids.unit_a),
        worker_run_id: Some(ids.worker_a),
        organization_id: Some(ids.organization_a),
        tool_call_record_id,
        tool_request_id: tool_request_id.to_string(),
        stage_kind: "target_intel".to_string(),
        attempt_epoch: Some(1),
        lease_token: Some(ids.lease_a),
        canonical_payload_json: canonical_payload.clone(),
        payload_sha256: payload_sha256.clone(),
    };

    let inserted = repo::stage_deliverable_submissions::insert(&pg.pool, &input)
        .await
        .expect("insert trusted stage deliverable submission");
    assert_eq!(inserted.operation_id, ids.operation_a);
    assert_eq!(inserted.stage_execution_id, ids.stage_a);
    assert_eq!(inserted.stage_run_unit_id, Some(ids.unit_a));
    assert_eq!(inserted.tool_call_record_id, tool_call_record_id);
    assert_eq!(inserted.payload_sha256, payload_sha256);
    assert_eq!(inserted.payload["stage_run_id"], ids.stage_a.to_string());

    let loaded = repo::stage_deliverable_submissions::load_scoped(
        &pg.pool,
        inserted.id,
        ids.operation_a,
        ids.stage_a,
    )
    .await
    .expect("load trusted submission")
    .expect("trusted submission exists");
    assert_eq!(loaded, inserted);
    assert!(repo::stage_deliverable_submissions::load_scoped(
        &pg.pool,
        inserted.id,
        ids.operation_b,
        ids.stage_b,
    )
    .await
    .expect("cross-operation load remains non-disclosing")
    .is_none());

    pg.stop().await;
}

#[tokio::test]
#[serial]
async fn stage_deliverable_submission_repo_rejects_hostile_tool_identity_rebinding() {
    let pg = LegacyPg::start("submission-repo-hostile").await;
    pg.apply_foundation().await;
    let ids = seed_runtime_graph(&pg.pool).await;
    let canonical_payload = format!(
        r#"{{"claims":[],"stage_id":"target_intel","stage_run_id":"{}"}}"#,
        ids.stage_b
    );
    let payload_sha256 = Sha256::digest(canonical_payload.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let hostile = repo::stage_deliverable_submissions::NewStageDeliverableSubmission {
        operation_id: ids.operation_b,
        stage_execution_id: ids.stage_b,
        stage_run_unit_id: Some(ids.unit_b),
        worker_run_id: Some(ids.worker_a),
        organization_id: Some(ids.organization_b),
        tool_call_record_id: ids.tool_call_a,
        tool_request_id: "submit-a".to_string(),
        stage_kind: "target_intel".to_string(),
        attempt_epoch: Some(1),
        lease_token: Some(ids.lease_a),
        canonical_payload_json: canonical_payload,
        payload_sha256,
    };

    let error = repo::stage_deliverable_submissions::insert(&pg.pool, &hostile)
        .await
        .expect_err("cross-operation tool-call rebinding must fail closed");
    assert_eq!(error.code(), "submission_tool_operation_mismatch");
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM stage_deliverable_submissions WHERE tool_call_record_id=$1",
    )
    .bind(ids.tool_call_a)
    .fetch_one(&pg.pool)
    .await
    .expect("count trusted submission rows");
    assert_eq!(count, 1, "hostile insert must not create a second row");

    pg.stop().await;
}

#[tokio::test]
#[serial]
async fn hostile_cross_operation_cross_snapshot_and_duplicate_chain_rows_are_rejected() {
    let pg = LegacyPg::start("hostile").await;
    pg.apply_foundation().await;
    let ids = seed_runtime_graph(&pg.pool).await;

    let cross_operation_unit = sqlx::query(
        r#"INSERT INTO stage_run_units
           (id, operation_id, stage_execution_id, scope_snapshot_id,
            organization_id, stage_kind, generation, specialist, status)
           VALUES ($1, $2, $3, $4, $5, 'target_intel', 1, 'recon', 'queued')"#,
    )
    .bind(fixture_id(0x4001))
    .bind(ids.operation_a)
    .bind(ids.stage_a)
    .bind(ids.snapshot_b)
    .bind(ids.organization_b)
    .execute(&pg.pool)
    .await;
    assert!(
        cross_operation_unit.is_err(),
        "snapshot from another operation must fail"
    );

    let cross_snapshot_handoff = sqlx::query(
        r#"INSERT INTO stage_handoffs
           (id, operation_id, organization_id, scope_snapshot_id, from_stage_kind,
            stage_execution_id, source_stage_run_unit_id, deliverable_submission_id,
            scope_hash, payload, payload_sha256, unit_gate_decision_hash, gate_passed_at)
           VALUES ($1, $2, $3, $4, 'target_intel', $5, $6, $7,
                   'wrong-scope', '{}'::jsonb, 'payload', 'gate', NOW())"#,
    )
    .bind(fixture_id(0x4002))
    .bind(ids.operation_a)
    .bind(ids.organization_a)
    .bind(ids.snapshot_b)
    .bind(ids.stage_a)
    .bind(ids.unit_a)
    .bind(ids.submission_a)
    .execute(&pg.pool)
    .await;
    assert!(
        cross_snapshot_handoff.is_err(),
        "foreign snapshot handoff must fail"
    );

    let duplicate_chain_owner = sqlx::query(
        r#"INSERT INTO stage_worker_runs
           (id, operation_id, stage_execution_id, stage_run_unit_id,
            organization_id, worker_generation, specialist, work_item_kind,
            work_item_key, agent_path, message_chain_id, status)
           VALUES ($1, $2, $3, $4, $5, 1, 'recon', 'asset_wave', 'wave-2',
                   'main>recon', $6, 'queued')"#,
    )
    .bind(fixture_id(0x4003))
    .bind(ids.operation_a)
    .bind(ids.stage_a)
    .bind(ids.unit_a)
    .bind(ids.organization_a)
    .bind(ids.chain_a)
    .execute(&pg.pool)
    .await;
    assert!(
        duplicate_chain_owner.is_err(),
        "one chain cannot own two workers"
    );

    pg.stop().await;
}

#[tokio::test]
#[serial]
async fn direct_rollout_jump_is_rejected_by_database() {
    let pg = LegacyPg::start("rollout-jump").await;
    pg.apply_foundation().await;
    let result = sqlx::query(
        r#"UPDATE runtime_memory_rollout
           SET contract='v2_only', contract_rank=3, row_version=99
           WHERE singleton_id=1"#,
    )
    .execute(&pg.pool)
    .await;
    let accepted = result.is_ok();
    pg.stop().await;
    assert!(!accepted, "database must reject a direct rollout rank jump");
}

#[tokio::test]
#[serial]
async fn direct_rollout_delete_is_rejected_by_database() {
    let pg = LegacyPg::start("rollout-delete").await;
    pg.apply_foundation().await;
    let result = sqlx::query("DELETE FROM runtime_memory_rollout WHERE singleton_id=1")
        .execute(&pg.pool)
        .await;
    let accepted = result.is_ok();
    pg.stop().await;
    assert!(
        !accepted,
        "database must reject deletion of the rollout singleton"
    );
}

#[tokio::test]
#[serial]
async fn foundation_accepts_legacy_duplicate_started_stage_runs_until_cutover() {
    let pg = LegacyPg::start("duplicate-started").await;
    let operation_id = fixture_id(0x5001);
    sqlx::query(
        "INSERT INTO operation_state (operation_id, profile, current_stage) VALUES ($1, 'red_team', 'enumeration')",
    )
    .bind(operation_id)
    .execute(&pg.pool)
    .await
    .expect("insert legacy operation");
    for run_id in [fixture_id(0x5002), fixture_id(0x5003)] {
        sqlx::query(
            "INSERT INTO stage_runs (id, operation_id, stage_kind, status) VALUES ($1, $2, 'enumeration', 'started')",
        )
        .bind(run_id)
        .bind(operation_id)
        .execute(&pg.pool)
        .await
        .expect("insert duplicate legacy active stage run");
    }
    let migration_error = pg
        .try_apply_foundation()
        .await
        .err()
        .map(|error| error.to_string());
    pg.stop().await;
    assert!(
        migration_error.is_none(),
        "additive foundation must preserve duplicate legacy active rows: {migration_error:?}"
    );
}

#[tokio::test]
#[serial]
async fn foundation_accepts_unknown_legacy_stage_status_until_cutover() {
    let pg = LegacyPg::start("unknown-status").await;
    let operation_id = fixture_id(0x5011);
    sqlx::query(
        "INSERT INTO operation_state (operation_id, profile, current_stage) VALUES ($1, 'red_team', 'enumeration')",
    )
    .bind(operation_id)
    .execute(&pg.pool)
    .await
    .expect("insert legacy operation");
    sqlx::query(
        "INSERT INTO stage_runs (id, operation_id, stage_kind, status) VALUES ($1, $2, 'enumeration', 'legacy_custom')",
    )
    .bind(fixture_id(0x5012))
    .bind(operation_id)
    .execute(&pg.pool)
    .await
    .expect("insert unknown legacy stage status");
    let migration_error = pg
        .try_apply_foundation()
        .await
        .err()
        .map(|error| error.to_string());
    pg.stop().await;
    assert!(
        migration_error.is_none(),
        "NOT VALID foundation constraint must not reject legacy status: {migration_error:?}"
    );
}

#[tokio::test]
#[serial]
async fn null_organization_cannot_bypass_tool_call_unit_owner() {
    let pg = LegacyPg::start("null-org").await;
    pg.apply_foundation().await;
    let ids = seed_runtime_graph(&pg.pool).await;
    let result = sqlx::query(
        r#"INSERT INTO tool_calls
           (id, call_id, session_id, task_id, name, operation_id,
            stage_execution_id, stage_run_unit_id, organization_id)
           VALUES ($1, 'null-org-bypass', $2, $3, 'submit_stage_deliverable',
                   $3, $4, $5, NULL)"#,
    )
    .bind(fixture_id(0x5101))
    .bind(ids.session_a)
    .bind(ids.operation_a)
    .bind(ids.stage_a)
    .bind(ids.unit_b)
    .execute(&pg.pool)
    .await;
    let accepted = result.is_ok();
    pg.stop().await;
    assert!(
        !accepted,
        "NULL organization_id must not disable the composite unit-owner FK"
    );
}

#[tokio::test]
#[serial]
async fn submission_cannot_reference_worker_from_another_operation() {
    let pg = LegacyPg::start("cross-worker").await;
    pg.apply_foundation().await;
    let ids = seed_runtime_graph(&pg.pool).await;
    let tool_call_b = fixture_id(0x5111);
    sqlx::query(
        r#"INSERT INTO tool_calls
           (id, call_id, session_id, task_id, name, operation_id,
            stage_execution_id, stage_run_unit_id, organization_id)
           VALUES ($1, 'submit-b', $2, $3, 'submit_stage_deliverable',
                   $3, $4, $5, $6)"#,
    )
    .bind(tool_call_b)
    .bind(ids.session_b)
    .bind(ids.operation_b)
    .bind(ids.stage_b)
    .bind(ids.unit_b)
    .bind(ids.organization_b)
    .execute(&pg.pool)
    .await
    .expect("insert operation-B tool call");
    let result = sqlx::query(
        r#"INSERT INTO stage_deliverable_submissions
           (id, operation_id, stage_execution_id, stage_run_unit_id, worker_run_id,
            organization_id, tool_call_record_id, tool_request_id, stage_kind,
            attempt_epoch, lease_token, payload, payload_sha256)
           VALUES ($1, $2, $3, $4, $5, $6, $7, 'cross-worker', 'target_intel',
                   1, $8, '{}'::jsonb, 'cross-worker')"#,
    )
    .bind(fixture_id(0x5112))
    .bind(ids.operation_b)
    .bind(ids.stage_b)
    .bind(ids.unit_b)
    .bind(ids.worker_a)
    .bind(ids.organization_b)
    .bind(tool_call_b)
    .bind(ids.lease_a)
    .execute(&pg.pool)
    .await;
    let accepted = result.is_ok();
    pg.stop().await;
    assert!(
        !accepted,
        "submission must not bind an operation-B unit to operation-A worker"
    );
}

#[tokio::test]
#[serial]
async fn snapshot_decision_must_match_operation_project_and_root() {
    let pg = LegacyPg::start("snapshot-decision").await;
    pg.apply_foundation().await;
    let ids = seed_runtime_graph(&pg.pool).await;
    let operation_c = fixture_id(0x5113);
    let decision_b2 = fixture_id(0x5114);
    let stage_b2 = fixture_id(0x5116);
    sqlx::query(
        r#"INSERT INTO operation_state
           (operation_id, profile, current_stage, runtime_memory_contract, project_scope_id)
           VALUES ($1, 'red_team', 'target_intel', 'dual_write_legacy_read', $2)"#,
    )
    .bind(operation_c)
    .bind(ids.project_a)
    .execute(&pg.pool)
    .await
    .expect("insert operation-C without frozen scope");
    sqlx::query(
        "INSERT INTO stage_runs (id, operation_id, stage_kind) VALUES ($1, $2, 'target_intel')",
    )
    .bind(stage_b2)
    .bind(ids.operation_b)
    .execute(&pg.pool)
    .await
    .expect("insert second operation-B stage execution");
    sqlx::query(
        r#"INSERT INTO operation_scope_decisions
           (id, operation_id, project_scope_id, stage_execution_id,
            root_organization_id, mode, decision_rows, decision_hash)
           VALUES ($1, $2, $3, $4, $5, 'root_only', '[]'::jsonb, 'decision-b2')"#,
    )
    .bind(decision_b2)
    .bind(ids.operation_b)
    .bind(ids.project_b)
    .bind(stage_b2)
    .bind(ids.organization_b)
    .execute(&pg.pool)
    .await
    .expect("insert unused operation-B decision");
    let result = sqlx::query(
        r#"INSERT INTO operation_org_scope_snapshots
           (id, operation_id, project_scope_id, scope_decision_id,
            project_path_at_freeze, root_organization_id, mode, scope_hash)
           VALUES ($1, $2, $3, $4, '/fixture/a', $5, 'root_only', 'cross-decision')"#,
    )
    .bind(fixture_id(0x5115))
    .bind(operation_c)
    .bind(ids.project_a)
    .bind(decision_b2)
    .bind(ids.organization_a)
    .execute(&pg.pool)
    .await;
    let accepted = result.is_ok();
    pg.stop().await;
    assert!(
        !accepted,
        "snapshot must not cite a decision from another operation/project/root"
    );
}

#[tokio::test]
#[serial]
async fn stage_run_unit_kind_must_match_stage_execution_kind() {
    let pg = LegacyPg::start("unit-stage-kind").await;
    pg.apply_foundation().await;
    let ids = seed_runtime_graph(&pg.pool).await;
    let result = sqlx::query(
        r#"INSERT INTO stage_run_units
           (id, operation_id, stage_execution_id, scope_snapshot_id,
            organization_id, stage_kind, generation, specialist, status)
           VALUES ($1, $2, $3, $4, $5, 'verification', 0, 'recon', 'queued')"#,
    )
    .bind(fixture_id(0x5122))
    .bind(ids.operation_a)
    .bind(ids.stage_a)
    .bind(ids.snapshot_a)
    .bind(ids.organization_child_a)
    .execute(&pg.pool)
    .await;
    let accepted = result.is_ok();
    pg.stop().await;
    assert!(
        !accepted,
        "unit stage_kind must equal its referenced stage_runs.stage_kind"
    );
}

#[tokio::test]
#[serial]
async fn handoff_submission_must_match_source_unit_organization() {
    let pg = LegacyPg::start("handoff-submission-org").await;
    pg.apply_foundation().await;
    let ids = seed_runtime_graph(&pg.pool).await;
    let unit_c = fixture_id(0x5132);
    sqlx::query(
        r#"INSERT INTO stage_run_units
           (id, operation_id, stage_execution_id, scope_snapshot_id,
            organization_id, stage_kind, generation, specialist, status)
           VALUES ($1, $2, $3, $4, $5, 'target_intel', 0, 'recon', 'passed')"#,
    )
    .bind(unit_c)
    .bind(ids.operation_a)
    .bind(ids.stage_a)
    .bind(ids.snapshot_a)
    .bind(ids.organization_child_a)
    .execute(&pg.pool)
    .await
    .expect("insert org-C stage unit");
    let result = sqlx::query(
        r#"INSERT INTO stage_handoffs
           (id, operation_id, organization_id, scope_snapshot_id, from_stage_kind,
            stage_execution_id, source_stage_run_unit_id, deliverable_submission_id,
            scope_hash, payload, payload_sha256, unit_gate_decision_hash, gate_passed_at)
           VALUES ($1, $2, $3, $4, 'target_intel', $5, $6, $7,
                   'scope-a', '{}'::jsonb, 'handoff-a', 'gate-a', NOW())"#,
    )
    .bind(fixture_id(0x5135))
    .bind(ids.operation_a)
    .bind(ids.organization_child_a)
    .bind(ids.snapshot_a)
    .bind(ids.stage_a)
    .bind(unit_c)
    .bind(ids.submission_a)
    .execute(&pg.pool)
    .await;
    let accepted = result.is_ok();
    pg.stop().await;
    assert!(
        !accepted,
        "handoff for org C must not cite the org-A deliverable submission"
    );
}

#[tokio::test]
#[serial]
async fn worker_chain_task_must_match_worker_operation() {
    let pg = LegacyPg::start("worker-chain-task").await;
    pg.apply_foundation().await;
    let ids = seed_runtime_graph(&pg.pool).await;
    let result = sqlx::query(
        r#"INSERT INTO stage_worker_runs
           (id, operation_id, stage_execution_id, stage_run_unit_id,
            organization_id, worker_generation, specialist, work_item_kind,
            work_item_key, agent_path, message_chain_id, status)
           VALUES ($1, $2, $3, $4, $5, 1, 'recon', 'asset_wave',
                   'cross-task-chain', 'main>recon', $6, 'queued')"#,
    )
    .bind(fixture_id(0x5141))
    .bind(ids.operation_a)
    .bind(ids.stage_a)
    .bind(ids.unit_a)
    .bind(ids.organization_a)
    .bind(ids.chain_b)
    .execute(&pg.pool)
    .await;
    let accepted = result.is_ok();
    pg.stop().await;
    assert!(
        !accepted,
        "operation-A worker must not bind a chain owned by operation-B task"
    );
}

#[tokio::test]
#[serial]
async fn operation_foreign_keys_cannot_cascade_runtime_history() {
    let pg = LegacyPg::start("operation-retention").await;
    pg.apply_foundation().await;
    let delete_rules = sqlx::query_as::<_, (String, String)>(
        r#"SELECT conrelid::regclass::text, confdeltype::text
           FROM pg_constraint
           WHERE contype='f'
             AND confrelid='operation_state'::regclass
             AND conrelid IN (
                 'operation_scope_decisions'::regclass,
                 'operation_org_scope_snapshots'::regclass,
                 'stage_run_units'::regclass
             )
           ORDER BY conrelid::regclass::text"#,
    )
    .fetch_all(&pg.pool)
    .await
    .expect("read operation retention foreign keys");
    pg.stop().await;
    assert_eq!(
        delete_rules.len(),
        3,
        "all runtime history tables need an operation FK"
    );
    assert!(
        delete_rules
            .iter()
            .all(|(_, delete_rule)| matches!(delete_rule.as_str(), "a" | "r")),
        "runtime history FKs must be NO ACTION/RESTRICT, got {delete_rules:?}"
    );
}

#[tokio::test]
#[serial]
async fn valid_tool_call_and_submission_identity_shapes_are_writable() {
    let pg = LegacyPg::start("valid-identity-shapes").await;
    pg.apply_foundation().await;
    let ids = seed_runtime_graph(&pg.pool).await;
    let scoping_stage = fixture_id(0x6001);
    let scoping_unit = fixture_id(0x6002);
    let legacy_tool = fixture_id(0x6003);
    let execution_tool = fixture_id(0x6004);
    let execution_submission = fixture_id(0x6005);
    let unit_tool = fixture_id(0x6006);
    let unit_submission = fixture_id(0x6007);
    let worker_tool = fixture_id(0x6008);
    let worker_submission = fixture_id(0x6009);

    sqlx::query(
        r#"INSERT INTO tool_calls
           (id, call_id, session_id, task_id, name, status)
           VALUES ($1, 'legacy-all-null', $2, $3, 'legacy_tool', 'finished')"#,
    )
    .bind(legacy_tool)
    .bind(ids.session_a)
    .bind(ids.operation_a)
    .execute(&pg.pool)
    .await
    .expect("legacy tool call with an entirely NULL runtime identity remains writable");

    sqlx::query("INSERT INTO stage_runs (id, operation_id, stage_kind) VALUES ($1, $2, 'scoping')")
        .bind(scoping_stage)
        .bind(ids.operation_a)
        .execute(&pg.pool)
        .await
        .expect("insert scoping execution");
    sqlx::query(
        r#"INSERT INTO tool_calls
           (id, call_id, session_id, task_id, name, status,
            operation_id, stage_execution_id)
           VALUES ($1, 'scoping-execution', $2, $3, 'submit_stage_deliverable',
                   'finished', $3, $4)"#,
    )
    .bind(execution_tool)
    .bind(ids.session_a)
    .bind(ids.operation_a)
    .bind(scoping_stage)
    .execute(&pg.pool)
    .await
    .expect("scoping execution-bound tool call is writable");
    sqlx::query(
        r#"INSERT INTO stage_deliverable_submissions
           (id, operation_id, stage_execution_id, tool_call_record_id,
            tool_request_id, stage_kind, payload, payload_sha256)
           VALUES ($1, $2, $3, $4, 'scoping-execution', 'scoping',
                   '{}'::jsonb, 'scoping-execution')"#,
    )
    .bind(execution_submission)
    .bind(ids.operation_a)
    .bind(scoping_stage)
    .bind(execution_tool)
    .execute(&pg.pool)
    .await
    .expect("scoping execution-bound submission is writable");

    sqlx::query(
        r#"INSERT INTO stage_run_units
           (id, operation_id, stage_execution_id, scope_snapshot_id,
            organization_id, stage_kind, generation, specialist, status)
           VALUES ($1, $2, $3, $4, $5, 'scoping', 0, 'scope_planner', 'running')"#,
    )
    .bind(scoping_unit)
    .bind(ids.operation_a)
    .bind(scoping_stage)
    .bind(ids.snapshot_a)
    .bind(ids.organization_a)
    .execute(&pg.pool)
    .await
    .expect("insert scoping unit");
    sqlx::query(
        r#"INSERT INTO tool_calls
           (id, call_id, session_id, task_id, name, status, operation_id,
            stage_execution_id, stage_run_unit_id, organization_id)
           VALUES ($1, 'scoping-unit', $2, $3, 'submit_stage_deliverable',
                   'finished', $3, $4, $5, $6)"#,
    )
    .bind(unit_tool)
    .bind(ids.session_a)
    .bind(ids.operation_a)
    .bind(scoping_stage)
    .bind(scoping_unit)
    .bind(ids.organization_a)
    .execute(&pg.pool)
    .await
    .expect("scoping unit-bound tool call is writable");
    sqlx::query(
        r#"INSERT INTO stage_deliverable_submissions
           (id, operation_id, stage_execution_id, stage_run_unit_id,
            organization_id, tool_call_record_id, tool_request_id, stage_kind,
            payload, payload_sha256)
           VALUES ($1, $2, $3, $4, $5, $6, 'scoping-unit', 'scoping',
                   '{}'::jsonb, 'scoping-unit')"#,
    )
    .bind(unit_submission)
    .bind(ids.operation_a)
    .bind(scoping_stage)
    .bind(scoping_unit)
    .bind(ids.organization_a)
    .bind(unit_tool)
    .execute(&pg.pool)
    .await
    .expect("scoping unit-bound submission is writable");

    sqlx::query(
        r#"INSERT INTO tool_calls
           (id, call_id, session_id, task_id, name, status, operation_id,
            stage_execution_id, stage_run_unit_id, worker_run_id,
            organization_id, attempt_epoch, lease_token)
           VALUES ($1, 'specialist-worker', $2, $3, 'submit_stage_deliverable',
                   'finished', $3, $4, $5, $6, $7, 1, $8)"#,
    )
    .bind(worker_tool)
    .bind(ids.session_a)
    .bind(ids.operation_a)
    .bind(ids.stage_a)
    .bind(ids.unit_a)
    .bind(ids.worker_a)
    .bind(ids.organization_a)
    .bind(ids.lease_a)
    .execute(&pg.pool)
    .await
    .expect("specialist worker-bound tool call is writable");
    sqlx::query(
        r#"INSERT INTO stage_deliverable_submissions
           (id, operation_id, stage_execution_id, stage_run_unit_id,
            worker_run_id, organization_id, tool_call_record_id,
            tool_request_id, stage_kind, attempt_epoch, lease_token,
            payload, payload_sha256)
           VALUES ($1, $2, $3, $4, $5, $6, $7, 'specialist-worker',
                   'target_intel', 1, $8, '{}'::jsonb, 'specialist-worker')"#,
    )
    .bind(worker_submission)
    .bind(ids.operation_a)
    .bind(ids.stage_a)
    .bind(ids.unit_a)
    .bind(ids.worker_a)
    .bind(ids.organization_a)
    .bind(worker_tool)
    .bind(ids.lease_a)
    .execute(&pg.pool)
    .await
    .expect("specialist worker-bound submission is writable");

    pg.stop().await;
}

#[tokio::test]
#[serial]
async fn partial_runtime_identity_shapes_and_cross_worker_active_tool_are_rejected() {
    let pg = LegacyPg::start("invalid-identity-shapes").await;
    pg.apply_foundation().await;
    let ids = seed_runtime_graph(&pg.pool).await;

    let operation_without_execution = sqlx::query(
        r#"INSERT INTO tool_calls
           (id, call_id, session_id, task_id, name, status, operation_id)
           VALUES ($1, 'partial-operation', $2, $3, 'runtime_tool', 'finished', $3)"#,
    )
    .bind(fixture_id(0x6101))
    .bind(ids.session_a)
    .bind(ids.operation_a)
    .execute(&pg.pool)
    .await;
    assert!(
        operation_without_execution.is_err(),
        "operation_id without stage_execution_id must fail the identity-shape contract"
    );

    let unit_without_organization = sqlx::query(
        r#"INSERT INTO tool_calls
           (id, call_id, session_id, task_id, name, status, operation_id,
            stage_execution_id, stage_run_unit_id)
           VALUES ($1, 'partial-unit', $2, $3, 'runtime_tool', 'finished',
                   $3, $4, $5)"#,
    )
    .bind(fixture_id(0x6102))
    .bind(ids.session_a)
    .bind(ids.operation_a)
    .bind(ids.stage_a)
    .bind(ids.unit_a)
    .execute(&pg.pool)
    .await;
    assert!(
        unit_without_organization.is_err(),
        "stage_run_unit_id without organization_id must fail the identity-shape contract"
    );

    let worker_without_lease = sqlx::query(
        r#"INSERT INTO tool_calls
           (id, call_id, session_id, task_id, name, status, operation_id,
            stage_execution_id, stage_run_unit_id, worker_run_id,
            organization_id, attempt_epoch)
           VALUES ($1, 'partial-worker', $2, $3, 'runtime_tool', 'finished',
                   $3, $4, $5, $6, $7, 1)"#,
    )
    .bind(fixture_id(0x6103))
    .bind(ids.session_a)
    .bind(ids.operation_a)
    .bind(ids.stage_a)
    .bind(ids.unit_a)
    .bind(ids.worker_a)
    .bind(ids.organization_a)
    .execute(&pg.pool)
    .await;
    assert!(
        worker_without_lease.is_err(),
        "worker identity without lease_token must fail the identity-shape contract"
    );

    let worker_b = fixture_id(0x6104);
    sqlx::query(
        r#"INSERT INTO stage_worker_runs
           (id, operation_id, stage_execution_id, stage_run_unit_id,
            organization_id, worker_generation, specialist, work_item_kind,
            work_item_key, agent_path, status)
           VALUES ($1, $2, $3, $4, $5, 0, 'recon', 'asset_wave',
                   'cross-active-tool', 'main>recon', 'queued')"#,
    )
    .bind(worker_b)
    .bind(ids.operation_a)
    .bind(ids.stage_a)
    .bind(ids.unit_a)
    .bind(ids.organization_a)
    .execute(&pg.pool)
    .await
    .expect("insert second worker for active-tool ownership test");
    let cross_worker_active_tool = sqlx::query(
        r#"UPDATE stage_worker_runs
           SET active_tool_call_id=$1, active_tool_started_at=NOW()
           WHERE id=$2"#,
    )
    .bind(ids.tool_call_a)
    .bind(worker_b)
    .execute(&pg.pool)
    .await;
    assert!(
        cross_worker_active_tool.is_err(),
        "a worker must not adopt another worker's active tool call"
    );

    pg.stop().await;
}

#[tokio::test]
#[serial]
async fn worker_fence_trigger_rejects_wrong_epoch_and_token_for_tools_and_submissions() {
    let pg = LegacyPg::start("worker-fence-trigger").await;
    pg.apply_foundation().await;
    let ids = seed_runtime_graph(&pg.pool).await;
    let wrong_token = fixture_id(0x6201);

    let wrong_epoch_tool = sqlx::query(
        r#"INSERT INTO tool_calls
           (id, call_id, session_id, task_id, name, status, operation_id,
            stage_execution_id, stage_run_unit_id, worker_run_id,
            organization_id, attempt_epoch, lease_token)
           VALUES ($1, 'wrong-epoch-tool', $2, $3, 'runtime_tool', 'finished',
                   $3, $4, $5, $6, $7, 2, $8)"#,
    )
    .bind(fixture_id(0x6202))
    .bind(ids.session_a)
    .bind(ids.operation_a)
    .bind(ids.stage_a)
    .bind(ids.unit_a)
    .bind(ids.worker_a)
    .bind(ids.organization_a)
    .bind(ids.lease_a)
    .execute(&pg.pool)
    .await;
    assert_sqlstate(
        wrong_epoch_tool,
        "P0001",
        "tool call with stale attempt_epoch",
    );

    let wrong_token_tool = sqlx::query(
        r#"INSERT INTO tool_calls
           (id, call_id, session_id, task_id, name, status, operation_id,
            stage_execution_id, stage_run_unit_id, worker_run_id,
            organization_id, attempt_epoch, lease_token)
           VALUES ($1, 'wrong-token-tool', $2, $3, 'runtime_tool', 'finished',
                   $3, $4, $5, $6, $7, 1, $8)"#,
    )
    .bind(fixture_id(0x6203))
    .bind(ids.session_a)
    .bind(ids.operation_a)
    .bind(ids.stage_a)
    .bind(ids.unit_a)
    .bind(ids.worker_a)
    .bind(ids.organization_a)
    .bind(wrong_token)
    .execute(&pg.pool)
    .await;
    assert_sqlstate(
        wrong_token_tool,
        "P0001",
        "tool call with foreign lease_token",
    );

    let valid_tool = fixture_id(0x6204);
    sqlx::query(
        r#"INSERT INTO tool_calls
           (id, call_id, session_id, task_id, name, status, operation_id,
            stage_execution_id, stage_run_unit_id, worker_run_id,
            organization_id, attempt_epoch, lease_token)
           VALUES ($1, 'valid-fenced-tool', $2, $3, 'submit_stage_deliverable',
                   'finished', $3, $4, $5, $6, $7, 1, $8)"#,
    )
    .bind(valid_tool)
    .bind(ids.session_a)
    .bind(ids.operation_a)
    .bind(ids.stage_a)
    .bind(ids.unit_a)
    .bind(ids.worker_a)
    .bind(ids.organization_a)
    .bind(ids.lease_a)
    .execute(&pg.pool)
    .await
    .expect("insert correctly fenced tool for submission trigger checks");

    let wrong_epoch_submission = sqlx::query(
        r#"INSERT INTO stage_deliverable_submissions
           (id, operation_id, stage_execution_id, stage_run_unit_id,
            worker_run_id, organization_id, tool_call_record_id,
            tool_request_id, stage_kind, attempt_epoch, lease_token,
            payload, payload_sha256)
           VALUES ($1, $2, $3, $4, $5, $6, $7, 'wrong-epoch-submission',
                   'target_intel', 2, $8, '{}'::jsonb, 'wrong-epoch-submission')"#,
    )
    .bind(fixture_id(0x6205))
    .bind(ids.operation_a)
    .bind(ids.stage_a)
    .bind(ids.unit_a)
    .bind(ids.worker_a)
    .bind(ids.organization_a)
    .bind(valid_tool)
    .bind(ids.lease_a)
    .execute(&pg.pool)
    .await;
    assert_sqlstate(
        wrong_epoch_submission,
        "P0001",
        "submission with stale attempt_epoch",
    );

    let wrong_token_submission = sqlx::query(
        r#"INSERT INTO stage_deliverable_submissions
           (id, operation_id, stage_execution_id, stage_run_unit_id,
            worker_run_id, organization_id, tool_call_record_id,
            tool_request_id, stage_kind, attempt_epoch, lease_token,
            payload, payload_sha256)
           VALUES ($1, $2, $3, $4, $5, $6, $7, 'wrong-token-submission',
                   'target_intel', 1, $8, '{}'::jsonb, 'wrong-token-submission')"#,
    )
    .bind(fixture_id(0x6206))
    .bind(ids.operation_a)
    .bind(ids.stage_a)
    .bind(ids.unit_a)
    .bind(ids.worker_a)
    .bind(ids.organization_a)
    .bind(valid_tool)
    .bind(wrong_token)
    .execute(&pg.pool)
    .await;
    assert_sqlstate(
        wrong_token_submission,
        "P0001",
        "submission with foreign lease_token",
    );

    pg.stop().await;
}

#[tokio::test]
#[serial]
async fn sealed_scope_rejects_late_unit_insert() {
    let pg = LegacyPg::start("sealed-scope-late-unit").await;
    pg.apply_foundation().await;
    let ids = seed_runtime_graph(&pg.pool).await;
    let result = sqlx::query(
        r#"INSERT INTO operation_org_scope_units
           (snapshot_id, organization_id, parent_organization_id,
            organization_name_at_freeze, role, depth, ordinal,
            decision_row_id, approval_source)
           VALUES ($1, $2, $3, 'Late Child', 'subsidiary', 1, 2,
                   'row-late-child', '{}'::jsonb)"#,
    )
    .bind(ids.snapshot_a)
    .bind(fixture_id(0x6301))
    .bind(ids.organization_a)
    .execute(&pg.pool)
    .await;
    let accepted = result.is_ok();
    pg.stop().await;
    assert!(
        !accepted,
        "sealed scope snapshots must reject scope-unit inserts after freeze"
    );
}

#[tokio::test]
#[serial]
async fn snapshot_without_declared_root_cannot_be_sealed_or_committed() {
    let pg = LegacyPg::start("scope-missing-root").await;
    pg.apply_foundation().await;
    let ids = seed_scope_head(&pg.pool, 0x6400).await;
    let mut tx = pg.pool.begin().await.expect("begin missing-root snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_snapshots
           (id, operation_id, project_scope_id, scope_decision_id,
            project_path_at_freeze, root_organization_id, mode, scope_hash)
           VALUES ($1, $2, $3, $4, $5, $6, 'root_only', 'missing-root')"#,
    )
    .bind(ids.snapshot)
    .bind(ids.operation)
    .bind(ids.project)
    .bind(ids.decision)
    .bind("/fixture/missing-root")
    .bind(ids.root_organization)
    .execute(&mut *tx)
    .await
    .expect("deferred root FK permits snapshot assembly inside transaction");
    let seal_result =
        sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
            .bind(ids.snapshot)
            .execute(&mut *tx)
            .await;
    if seal_result.is_err() {
        assert_sqlstate(
            seal_result,
            "P0001",
            "seal attempt for snapshot that omits its declared root organization",
        );
        tx.rollback()
            .await
            .expect("rollback missing-root seal rejection");
    } else {
        assert_sqlstate(
            tx.commit().await,
            "23503",
            "snapshot that omits its declared root organization",
        );
    }
    pg.stop().await;
}

#[tokio::test]
#[serial]
async fn cross_snapshot_parent_cannot_commit() {
    let pg = LegacyPg::start("scope-cross-parent").await;
    pg.apply_foundation().await;
    let existing = seed_runtime_graph(&pg.pool).await;
    let ids = seed_scope_head(&pg.pool, 0x6500).await;
    let mut tx = pg.pool.begin().await.expect("begin cross-parent snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_snapshots
           (id, operation_id, project_scope_id, scope_decision_id,
            project_path_at_freeze, root_organization_id, mode, scope_hash)
           VALUES ($1, $2, $3, $4, $5, $6, 'root_only', 'cross-parent')"#,
    )
    .bind(ids.snapshot)
    .bind(ids.operation)
    .bind(ids.project)
    .bind(ids.decision)
    .bind("/fixture/cross-parent")
    .bind(ids.root_organization)
    .execute(&mut *tx)
    .await
    .expect("insert unsealed snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units
           (snapshot_id, organization_id, organization_name_at_freeze, role,
            depth, ordinal, decision_row_id, approval_source)
           VALUES ($1, $2, 'New Root', 'root', 0, 0, 'new-root', '{}'::jsonb)"#,
    )
    .bind(ids.snapshot)
    .bind(ids.root_organization)
    .execute(&mut *tx)
    .await
    .expect("insert new snapshot root");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units
           (snapshot_id, organization_id, parent_organization_id,
            organization_name_at_freeze, role, depth, ordinal,
            decision_row_id, approval_source)
           VALUES ($1, $2, $3, 'Cross Parent Child', 'subsidiary', 1, 1,
                   'cross-parent-child', '{}'::jsonb)"#,
    )
    .bind(ids.snapshot)
    .bind(fixture_id(0x6507))
    .bind(existing.organization_a)
    .execute(&mut *tx)
    .await
    .expect("deferred parent FK permits scope assembly inside transaction");
    sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
        .bind(ids.snapshot)
        .execute(&mut *tx)
        .await
        .expect("seal cross-parent snapshot before deferred validation");
    let result = tx.commit().await;
    assert_sqlstate(result, "23503", "scope unit with a cross-snapshot parent");
    pg.stop().await;
}

#[tokio::test]
#[serial]
async fn second_root_cannot_enter_a_scope_snapshot() {
    let pg = LegacyPg::start("scope-second-root").await;
    pg.apply_foundation().await;
    let ids = seed_scope_head(&pg.pool, 0x6600).await;
    let mut tx = pg.pool.begin().await.expect("begin second-root snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_snapshots
           (id, operation_id, project_scope_id, scope_decision_id,
            project_path_at_freeze, root_organization_id, mode, scope_hash)
           VALUES ($1, $2, $3, $4, $5, $6, 'root_only', 'second-root')"#,
    )
    .bind(ids.snapshot)
    .bind(ids.operation)
    .bind(ids.project)
    .bind(ids.decision)
    .bind("/fixture/second-root")
    .bind(ids.root_organization)
    .execute(&mut *tx)
    .await
    .expect("insert unsealed snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units
           (snapshot_id, organization_id, organization_name_at_freeze, role,
            depth, ordinal, decision_row_id, approval_source)
           VALUES ($1, $2, 'Declared Root', 'root', 0, 0,
                   'declared-root', '{}'::jsonb)"#,
    )
    .bind(ids.snapshot)
    .bind(ids.root_organization)
    .execute(&mut *tx)
    .await
    .expect("insert declared root");
    let result = sqlx::query(
        r#"INSERT INTO operation_org_scope_units
           (snapshot_id, organization_id, organization_name_at_freeze, role,
            depth, ordinal, decision_row_id, approval_source)
           VALUES ($1, $2, 'Second Root', 'root', 0, 1,
                   'second-root', '{}'::jsonb)"#,
    )
    .bind(ids.snapshot)
    .bind(fixture_id(0x6607))
    .execute(&mut *tx)
    .await;
    assert_sqlstate(result, "23505", "second root in the same scope snapshot");
    tx.rollback()
        .await
        .expect("rollback rejected second-root scope");
    pg.stop().await;
}

#[tokio::test]
#[serial]
async fn frozen_decision_snapshot_and_unit_reject_update_and_delete() {
    let pg = LegacyPg::start("frozen-scope-immutability").await;
    pg.apply_foundation().await;
    let ids = seed_runtime_graph(&pg.pool).await;

    let decision_update =
        sqlx::query("UPDATE operation_scope_decisions SET decision_hash='mutated' WHERE id=$1")
            .bind(ids.decision_a)
            .execute(&pg.pool)
            .await;
    assert!(
        decision_update.is_err(),
        "frozen decision update must be rejected"
    );
    let decision_delete = sqlx::query("DELETE FROM operation_scope_decisions WHERE id=$1")
        .bind(ids.decision_a)
        .execute(&pg.pool)
        .await;
    assert!(
        decision_delete.is_err(),
        "frozen decision delete must be rejected"
    );

    let snapshot_update =
        sqlx::query("UPDATE operation_org_scope_snapshots SET scope_hash='mutated' WHERE id=$1")
            .bind(ids.snapshot_a)
            .execute(&pg.pool)
            .await;
    assert!(
        snapshot_update.is_err(),
        "frozen snapshot update must be rejected"
    );
    let snapshot_delete = sqlx::query("DELETE FROM operation_org_scope_snapshots WHERE id=$1")
        .bind(ids.snapshot_a)
        .execute(&pg.pool)
        .await;
    assert!(
        snapshot_delete.is_err(),
        "frozen snapshot delete must be rejected"
    );

    let unit_update = sqlx::query(
        r#"UPDATE operation_org_scope_units
           SET organization_name_at_freeze='Mutated'
           WHERE snapshot_id=$1 AND organization_id=$2"#,
    )
    .bind(ids.snapshot_a)
    .bind(ids.organization_a)
    .execute(&pg.pool)
    .await;
    assert!(
        unit_update.is_err(),
        "frozen scope-unit update must be rejected"
    );
    let unit_delete = sqlx::query(
        r#"DELETE FROM operation_org_scope_units
           WHERE snapshot_id=$1 AND organization_id=$2"#,
    )
    .bind(ids.snapshot_a)
    .bind(ids.organization_a)
    .execute(&pg.pool)
    .await;
    assert!(
        unit_delete.is_err(),
        "frozen scope-unit delete must be rejected"
    );

    pg.stop().await;
}

#[tokio::test]
#[serial]
async fn scope_seal_waits_for_inflight_unit_insert_then_rejects_late_units() {
    let pg = LegacyPg::start("scope-seal-lock").await;
    pg.apply_foundation().await;
    let ids = seed_scope_head(&pg.pool, 0x6700).await;

    let mut setup = pg.pool.begin().await.expect("begin unsealed scope setup");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_snapshots
           (id, operation_id, project_scope_id, scope_decision_id,
            project_path_at_freeze, root_organization_id, mode, scope_hash)
           VALUES ($1, $2, $3, $4, '/fixture/scope-seal-lock', $5,
                   'root_only', 'scope-seal-lock')"#,
    )
    .bind(ids.snapshot)
    .bind(ids.operation)
    .bind(ids.project)
    .bind(ids.decision)
    .bind(ids.root_organization)
    .execute(&mut *setup)
    .await
    .expect("insert unsealed scope snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units
           (snapshot_id, organization_id, organization_name_at_freeze, role,
            depth, ordinal, decision_row_id, approval_source)
           VALUES ($1, $2, 'Lock Root', 'root', 0, 0,
                   'lock-root', '{}'::jsonb)"#,
    )
    .bind(ids.snapshot)
    .bind(ids.root_organization)
    .execute(&mut *setup)
    .await
    .expect("insert root before concurrent scope assembly");
    setup.commit().await.expect("commit unsealed scope setup");

    let mut unit_insert = pg.pool.begin().await.expect("begin in-flight unit insert");
    let insert_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *unit_insert)
        .await
        .expect("read unit-insert backend pid");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units
           (snapshot_id, organization_id, parent_organization_id,
            organization_name_at_freeze, role, depth, ordinal,
            decision_row_id, approval_source)
           VALUES ($1, $2, $3, 'In Flight Child', 'subsidiary', 1, 1,
                   'in-flight-child', '{}'::jsonb)"#,
    )
    .bind(ids.snapshot)
    .bind(fixture_id(0x6707))
    .bind(ids.root_organization)
    .execute(&mut *unit_insert)
    .await
    .expect("insert child while retaining snapshot lock");

    let barrier = Arc::new(Barrier::new(2));
    let seal_barrier = Arc::clone(&barrier);
    let (pid_sender, pid_receiver) = oneshot::channel();
    let seal_pool = pg.pool.clone();
    let snapshot_id = ids.snapshot;
    let seal_task = tokio::spawn(async move {
        let mut seal = seal_pool.begin().await?;
        let seal_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *seal)
            .await?;
        let _ = pid_sender.send(seal_pid);
        seal_barrier.wait().await;
        sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
            .bind(snapshot_id)
            .execute(&mut *seal)
            .await?;
        seal.commit().await?;
        Ok::<(), sqlx::Error>(())
    });
    let seal_pid = timeout(Duration::from_secs(5), pid_receiver)
        .await
        .expect("seal task must publish its backend pid")
        .expect("seal task pid channel remains open");
    timeout(Duration::from_secs(5), barrier.wait())
        .await
        .expect("seal task must reach update barrier");
    let seal_was_blocked =
        wait_until_backend_is_blocked(&mut unit_insert, insert_pid, seal_pid).await;
    let seal_still_waiting = !seal_task.is_finished();

    unit_insert
        .commit()
        .await
        .expect("commit in-flight unit before seal resumes");
    timeout(Duration::from_secs(5), seal_task)
        .await
        .expect("seal must resume after unit commit")
        .expect("seal task must join")
        .expect("seal transaction must commit");

    let late_insert = sqlx::query(
        r#"INSERT INTO operation_org_scope_units
           (snapshot_id, organization_id, parent_organization_id,
            organization_name_at_freeze, role, depth, ordinal,
            decision_row_id, approval_source)
           VALUES ($1, $2, $3, 'Late Child', 'subsidiary', 1, 2,
                   'late-child-after-lock', '{}'::jsonb)"#,
    )
    .bind(ids.snapshot)
    .bind(fixture_id(0x6708))
    .bind(ids.root_organization)
    .execute(&pg.pool)
    .await;
    let late_insert_rejected = late_insert.is_err();
    pg.stop().await;

    assert!(
        seal_was_blocked && seal_still_waiting,
        "seal UPDATE must wait on the lock held by the in-flight scope-unit INSERT"
    );
    assert!(
        late_insert_rejected,
        "once the waiting seal commits, later scope-unit INSERTs must fail"
    );
}

#[tokio::test]
#[serial]
async fn worker_lease_rotation_waits_for_inflight_tool_fence_then_rejects_old_fence() {
    let pg = LegacyPg::start("worker-fence-lock").await;
    pg.apply_foundation().await;
    let ids = seed_runtime_graph(&pg.pool).await;

    let mut tool_insert = pg.pool.begin().await.expect("begin fenced tool insert");
    let insert_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *tool_insert)
        .await
        .expect("read tool-insert backend pid");
    sqlx::query(
        r#"INSERT INTO tool_calls
           (id, call_id, session_id, task_id, name, status, operation_id,
            stage_execution_id, stage_run_unit_id, worker_run_id,
            organization_id, attempt_epoch, lease_token)
           VALUES ($1, 'in-flight-fenced-tool', $2, $3, 'runtime_tool',
                   'finished', $3, $4, $5, $6, $7, 1, $8)"#,
    )
    .bind(fixture_id(0x6801))
    .bind(ids.session_a)
    .bind(ids.operation_a)
    .bind(ids.stage_a)
    .bind(ids.unit_a)
    .bind(ids.worker_a)
    .bind(ids.organization_a)
    .bind(ids.lease_a)
    .execute(&mut *tool_insert)
    .await
    .expect("insert tool while retaining worker fence lock");

    let next_lease_token = fixture_id(0x6802);
    let barrier = Arc::new(Barrier::new(2));
    let rotation_barrier = Arc::clone(&barrier);
    let (pid_sender, pid_receiver) = oneshot::channel();
    let rotation_pool = pg.pool.clone();
    let worker_id = ids.worker_a;
    let rotation_task = tokio::spawn(async move {
        let mut rotation = rotation_pool.begin().await?;
        let rotation_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *rotation)
            .await?;
        let _ = pid_sender.send(rotation_pid);
        rotation_barrier.wait().await;
        sqlx::query(
            r#"UPDATE stage_worker_runs
               SET attempt_epoch=2, lease_token=$1, lease_owner='worker-rotated',
                   lease_acquired_at=NOW(), lease_expires_at=NOW() + INTERVAL '30 seconds'
               WHERE id=$2"#,
        )
        .bind(next_lease_token)
        .bind(worker_id)
        .execute(&mut *rotation)
        .await?;
        rotation.commit().await?;
        Ok::<(), sqlx::Error>(())
    });
    let rotation_pid = timeout(Duration::from_secs(5), pid_receiver)
        .await
        .expect("rotation task must publish its backend pid")
        .expect("rotation task pid channel remains open");
    timeout(Duration::from_secs(5), barrier.wait())
        .await
        .expect("rotation task must reach update barrier");
    let rotation_was_blocked =
        wait_until_backend_is_blocked(&mut tool_insert, insert_pid, rotation_pid).await;
    let rotation_still_waiting = !rotation_task.is_finished();

    tool_insert
        .commit()
        .await
        .expect("commit fenced tool before lease rotation resumes");
    timeout(Duration::from_secs(5), rotation_task)
        .await
        .expect("rotation must resume after tool commit")
        .expect("rotation task must join")
        .expect("lease rotation transaction must commit");

    let stale_tool = sqlx::query(
        r#"INSERT INTO tool_calls
           (id, call_id, session_id, task_id, name, status, operation_id,
            stage_execution_id, stage_run_unit_id, worker_run_id,
            organization_id, attempt_epoch, lease_token)
           VALUES ($1, 'stale-fence-after-rotation', $2, $3, 'runtime_tool',
                   'finished', $3, $4, $5, $6, $7, 1, $8)"#,
    )
    .bind(fixture_id(0x6803))
    .bind(ids.session_a)
    .bind(ids.operation_a)
    .bind(ids.stage_a)
    .bind(ids.unit_a)
    .bind(ids.worker_a)
    .bind(ids.organization_a)
    .bind(ids.lease_a)
    .execute(&pg.pool)
    .await;
    let stale_tool_rejected = stale_tool.is_err();
    pg.stop().await;

    assert!(
        rotation_was_blocked && rotation_still_waiting,
        "lease epoch/token rotation must wait on the in-flight tool fence lock"
    );
    assert!(
        stale_tool_rejected,
        "after rotation commits, a new tool call using the old fence must fail"
    );
}

#[tokio::test]
#[serial]
async fn runtime_tool_call_requires_non_null_matching_task_owner() {
    let pg = LegacyPg::start("runtime-tool-task-owner").await;
    pg.apply_foundation().await;
    let ids = seed_runtime_graph(&pg.pool).await;

    let missing_task = sqlx::query(
        r#"INSERT INTO tool_calls
           (id, call_id, session_id, task_id, name, status,
            operation_id, stage_execution_id)
           VALUES ($1, 'runtime-missing-task', $2, NULL, 'runtime_tool',
                   'finished', $3, $4)"#,
    )
    .bind(fixture_id(0x6901))
    .bind(ids.session_a)
    .bind(ids.operation_a)
    .bind(ids.stage_a)
    .execute(&pg.pool)
    .await;
    assert_sqlstate(
        missing_task,
        "23514",
        "runtime tool call without task owner",
    );

    let wrong_task = sqlx::query(
        r#"INSERT INTO tool_calls
           (id, call_id, session_id, task_id, name, status,
            operation_id, stage_execution_id)
           VALUES ($1, 'runtime-wrong-task', $2, $3, 'runtime_tool',
                   'finished', $4, $5)"#,
    )
    .bind(fixture_id(0x6902))
    .bind(ids.session_b)
    .bind(ids.operation_b)
    .bind(ids.operation_a)
    .bind(ids.stage_a)
    .execute(&pg.pool)
    .await;
    assert_sqlstate(
        wrong_task,
        "23514",
        "runtime tool call owned by another task",
    );

    pg.stop().await;
}

#[tokio::test]
#[serial]
async fn scope_seal_rejects_declared_root_with_subsidiary_role() {
    let pg = LegacyPg::start("scope-wrong-root-role").await;
    pg.apply_foundation().await;
    let ids = seed_scope_head(&pg.pool, 0x6a00).await;
    let actual_root = fixture_id(0x6a07);
    let mut tx = pg.pool.begin().await.expect("begin wrong-root-role scope");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_snapshots
           (id, operation_id, project_scope_id, scope_decision_id,
            project_path_at_freeze, root_organization_id, mode, scope_hash)
           VALUES ($1, $2, $3, $4, '/fixture/wrong-root-role', $5,
                   'root_only', 'wrong-root-role')"#,
    )
    .bind(ids.snapshot)
    .bind(ids.operation)
    .bind(ids.project)
    .bind(ids.decision)
    .bind(ids.root_organization)
    .execute(&mut *tx)
    .await
    .expect("insert unsealed wrong-root-role snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units
           (snapshot_id, organization_id, organization_name_at_freeze, role,
            depth, ordinal, decision_row_id, approval_source)
           VALUES ($1, $2, 'Actual Root', 'root', 0, 0,
                   'actual-root', '{}'::jsonb)"#,
    )
    .bind(ids.snapshot)
    .bind(actual_root)
    .execute(&mut *tx)
    .await
    .expect("insert actual root-role unit");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units
           (snapshot_id, organization_id, parent_organization_id,
            organization_name_at_freeze, role, depth, ordinal,
            decision_row_id, approval_source)
           VALUES ($1, $2, $3, 'Declared Root As Child', 'subsidiary', 1, 1,
                   'declared-root-as-child', '{}'::jsonb)"#,
    )
    .bind(ids.snapshot)
    .bind(ids.root_organization)
    .bind(actual_root)
    .execute(&mut *tx)
    .await
    .expect("insert declared root identity with wrong role");
    let result =
        sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
            .bind(ids.snapshot)
            .execute(&mut *tx)
            .await;
    assert_sqlstate(result, "P0001", "seal with subsidiary-role declared root");
    tx.rollback().await.expect("rollback wrong-root-role scope");
    pg.stop().await;
}

#[tokio::test]
#[serial]
async fn scoping_tool_and_submission_bind_once_in_dependency_order() {
    let pg = LegacyPg::start("scoping-bind-order").await;
    pg.apply_foundation().await;
    let ids = seed_runtime_graph(&pg.pool).await;
    let stage = fixture_id(0x6b01);
    let unit = fixture_id(0x6b02);
    sqlx::query("INSERT INTO stage_runs (id, operation_id, stage_kind) VALUES ($1, $2, 'scoping')")
        .bind(stage)
        .bind(ids.operation_a)
        .execute(&pg.pool)
        .await
        .expect("insert scoping execution for bind tests");
    sqlx::query(
        r#"INSERT INTO stage_run_units
           (id, operation_id, stage_execution_id, scope_snapshot_id,
            organization_id, stage_kind, generation, specialist, status)
           VALUES ($1, $2, $3, $4, $5, 'scoping', 0, 'scope_planner', 'running')"#,
    )
    .bind(unit)
    .bind(ids.operation_a)
    .bind(stage)
    .bind(ids.snapshot_a)
    .bind(ids.organization_a)
    .execute(&pg.pool)
    .await
    .expect("insert sealed-scope scoping unit");

    let tool = fixture_id(0x6b03);
    let submission = fixture_id(0x6b04);
    sqlx::query(
        r#"INSERT INTO tool_calls
           (id, call_id, session_id, task_id, name, status,
            operation_id, stage_execution_id)
           VALUES ($1, 'ordered-scoping-bind', $2, $3,
                   'submit_stage_deliverable', 'finished', $3, $4)"#,
    )
    .bind(tool)
    .bind(ids.session_a)
    .bind(ids.operation_a)
    .bind(stage)
    .execute(&pg.pool)
    .await
    .expect("insert execution-bound scoping tool");
    sqlx::query(
        r#"INSERT INTO stage_deliverable_submissions
           (id, operation_id, stage_execution_id, tool_call_record_id,
            tool_request_id, stage_kind, payload, payload_sha256)
           VALUES ($1, $2, $3, $4, 'ordered-scoping-bind', 'scoping',
                   '{"phase":"execution"}'::jsonb, 'ordered-scoping-bind')"#,
    )
    .bind(submission)
    .bind(ids.operation_a)
    .bind(stage)
    .bind(tool)
    .execute(&pg.pool)
    .await
    .expect("insert execution-bound scoping submission");

    let mut ordered = pg.pool.begin().await.expect("begin ordered scoping bind");
    sqlx::query(
        r#"UPDATE tool_calls
           SET stage_run_unit_id=$1, organization_id=$2
           WHERE id=$3"#,
    )
    .bind(unit)
    .bind(ids.organization_a)
    .bind(tool)
    .execute(&mut *ordered)
    .await
    .expect("bind tool before dependent submission");
    sqlx::query(
        r#"UPDATE stage_deliverable_submissions
           SET stage_run_unit_id=$1, organization_id=$2
           WHERE id=$3"#,
    )
    .bind(unit)
    .bind(ids.organization_a)
    .bind(submission)
    .execute(&mut *ordered)
    .await
    .expect("bind submission after its tool in the same transaction");
    ordered.commit().await.expect("commit ordered scoping bind");

    let second_tool_bind = sqlx::query(
        r#"UPDATE tool_calls
           SET stage_run_unit_id=NULL, organization_id=NULL
           WHERE id=$1"#,
    )
    .bind(tool)
    .execute(&pg.pool)
    .await;
    assert_sqlstate(second_tool_bind, "P0001", "second tool context bind");
    let second_submission_bind = sqlx::query(
        r#"UPDATE stage_deliverable_submissions
           SET stage_run_unit_id=NULL, organization_id=NULL
           WHERE id=$1"#,
    )
    .bind(submission)
    .execute(&pg.pool)
    .await;
    assert_sqlstate(
        second_submission_bind,
        "P0001",
        "second submission context bind",
    );
    let payload_mutation = sqlx::query(
        r#"UPDATE stage_deliverable_submissions
           SET payload='{"phase":"mutated"}'::jsonb, payload_sha256='mutated'
           WHERE id=$1"#,
    )
    .bind(submission)
    .execute(&pg.pool)
    .await;
    assert_sqlstate(
        payload_mutation,
        "P0001",
        "submission payload and hash mutation",
    );

    let non_scoping_tool = fixture_id(0x6b05);
    sqlx::query(
        r#"INSERT INTO tool_calls
           (id, call_id, session_id, task_id, name, status,
            operation_id, stage_execution_id)
           VALUES ($1, 'non-scoping-bind', $2, $3, 'runtime_tool',
                   'finished', $3, $4)"#,
    )
    .bind(non_scoping_tool)
    .bind(ids.session_a)
    .bind(ids.operation_a)
    .bind(ids.stage_a)
    .execute(&pg.pool)
    .await
    .expect("insert execution-bound non-scoping tool");
    let non_scoping_bind = sqlx::query(
        r#"UPDATE tool_calls
           SET stage_run_unit_id=$1, organization_id=$2
           WHERE id=$3"#,
    )
    .bind(ids.unit_a)
    .bind(ids.organization_a)
    .bind(non_scoping_tool)
    .execute(&pg.pool)
    .await;
    assert_sqlstate(
        non_scoping_bind,
        "P0001",
        "non-scoping execution-to-unit tool bind",
    );

    let reverse_tool = fixture_id(0x6b06);
    let reverse_submission = fixture_id(0x6b07);
    sqlx::query(
        r#"INSERT INTO tool_calls
           (id, call_id, session_id, task_id, name, status,
            operation_id, stage_execution_id)
           VALUES ($1, 'reverse-scoping-bind', $2, $3,
                   'submit_stage_deliverable', 'finished', $3, $4)"#,
    )
    .bind(reverse_tool)
    .bind(ids.session_a)
    .bind(ids.operation_a)
    .bind(stage)
    .execute(&pg.pool)
    .await
    .expect("insert reverse-order scoping tool");
    sqlx::query(
        r#"INSERT INTO stage_deliverable_submissions
           (id, operation_id, stage_execution_id, tool_call_record_id,
            tool_request_id, stage_kind, payload, payload_sha256)
           VALUES ($1, $2, $3, $4, 'reverse-scoping-bind', 'scoping',
                   '{}'::jsonb, 'reverse-scoping-bind')"#,
    )
    .bind(reverse_submission)
    .bind(ids.operation_a)
    .bind(stage)
    .bind(reverse_tool)
    .execute(&pg.pool)
    .await
    .expect("insert reverse-order scoping submission");
    let mut reverse = pg.pool.begin().await.expect("begin reverse scoping bind");
    let reverse_result = sqlx::query(
        r#"UPDATE stage_deliverable_submissions
           SET stage_run_unit_id=$1, organization_id=$2
           WHERE id=$3"#,
    )
    .bind(unit)
    .bind(ids.organization_a)
    .bind(reverse_submission)
    .execute(&mut *reverse)
    .await;
    assert_sqlstate(
        reverse_result,
        "23503",
        "submission bind performed before its tool bind",
    );
    reverse
        .rollback()
        .await
        .expect("rollback reverse-order scoping bind");

    pg.stop().await;
}

#[tokio::test]
#[serial]
async fn handoff_allows_only_one_way_invalidation() {
    let pg = LegacyPg::start("handoff-one-way-invalidation").await;
    pg.apply_foundation().await;
    let ids = seed_runtime_graph(&pg.pool).await;
    let handoff = fixture_id(0x6c01);
    sqlx::query(
        r#"INSERT INTO stage_handoffs
           (id, operation_id, organization_id, scope_snapshot_id, from_stage_kind,
            stage_execution_id, source_stage_run_unit_id, deliverable_submission_id,
            scope_hash, payload, payload_sha256, unit_gate_decision_hash, gate_passed_at)
           VALUES ($1, $2, $3, $4, 'target_intel', $5, $6, $7,
                   'scope-a', '{"result":"passed"}'::jsonb, 'handoff-payload',
                   'gate-a', NOW() - INTERVAL '1 second')"#,
    )
    .bind(handoff)
    .bind(ids.operation_a)
    .bind(ids.organization_a)
    .bind(ids.snapshot_a)
    .bind(ids.stage_a)
    .bind(ids.unit_a)
    .bind(ids.submission_a)
    .execute(&pg.pool)
    .await
    .expect("insert immutable handoff");

    let payload_mutation = sqlx::query(
        r#"UPDATE stage_handoffs
           SET payload='{"result":"mutated"}'::jsonb, payload_sha256='mutated'
           WHERE id=$1"#,
    )
    .bind(handoff)
    .execute(&pg.pool)
    .await;
    assert_sqlstate(
        payload_mutation,
        "P0001",
        "handoff payload and hash mutation",
    );
    sqlx::query("UPDATE stage_handoffs SET invalidated_at=NOW() WHERE id=$1")
        .bind(handoff)
        .execute(&pg.pool)
        .await
        .expect("invalidate handoff exactly once");
    let clear_invalidation =
        sqlx::query("UPDATE stage_handoffs SET invalidated_at=NULL WHERE id=$1")
            .bind(handoff)
            .execute(&pg.pool)
            .await;
    assert_sqlstate(clear_invalidation, "P0001", "clearing handoff invalidation");
    let second_invalidation = sqlx::query(
        "UPDATE stage_handoffs SET invalidated_at=NOW() + INTERVAL '1 second' WHERE id=$1",
    )
    .bind(handoff)
    .execute(&pg.pool)
    .await;
    assert_sqlstate(second_invalidation, "P0001", "second handoff invalidation");
    let delete = sqlx::query("DELETE FROM stage_handoffs WHERE id=$1")
        .bind(handoff)
        .execute(&pg.pool)
        .await;
    assert_sqlstate(delete, "P0001", "handoff deletion");

    pg.stop().await;
}
