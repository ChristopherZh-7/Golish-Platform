use golish_db::{DbConfig, GolishDb};
use serial_test::serial;
use tempfile::TempDir;
use uuid::Uuid;

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read reserved local postgres port")
        .port()
}

struct CandidateApplicationModelDb {
    db: GolishDb,
    _data_dir: TempDir,
}

impl CandidateApplicationModelDb {
    async fn start(label: &str) -> Self {
        let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
        let config = DbConfig {
            pg_data_dir: data_dir.path().join("pgdata"),
            port: reserve_local_port(),
            database: format!(
                "candidate_application_model_{label}_{}",
                Uuid::new_v4().simple()
            ),
            ..DbConfig::default()
        };
        let db = GolishDb::start(config)
            .await
            .expect("start fresh migrated embedded postgres");
        Self {
            db,
            _data_dir: data_dir,
        }
    }
}

async fn insert_operation_state(
    db: &CandidateApplicationModelDb,
    application_model_contract: Option<&str>,
) -> Result<Uuid, sqlx::Error> {
    let session_id = Uuid::new_v4();
    let operation_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    let project_path = format!("/tmp/candidate-model-{operation_id}");
    let path_sha256 = format!("sha256:{operation_id}");
    sqlx::query(
        r#"INSERT INTO sessions(id,title,status,project_path)
           VALUES($1,'Candidate Application Model contract','running',$2)"#,
    )
    .bind(session_id)
    .bind(&project_path)
    .execute(db.db.pool())
    .await?;
    sqlx::query(
        r#"INSERT INTO tasks(id,session_id,title,input,status)
           VALUES($1,$2,'Candidate Application Model contract','contract','running')"#,
    )
    .bind(operation_id)
    .bind(session_id)
    .execute(db.db.pool())
    .await?;
    sqlx::query(
        r#"INSERT INTO project_scopes(project_scope_id,canonical_project_path,path_sha256)
           VALUES($1,$2,$3)"#,
    )
    .bind(project_scope_id)
    .bind(&project_path)
    .bind(&path_sha256)
    .execute(db.db.pool())
    .await?;
    match application_model_contract {
        Some(contract) => {
            sqlx::query(
                r#"INSERT INTO operation_state(
                       operation_id,profile,current_stage,runtime_memory_contract,
                       attack_execution_contract,application_model_contract,project_scope_id
                   ) VALUES($1,'red_team','attack_candidate','v2_only','v2_only',$2,$3)"#,
            )
            .bind(operation_id)
            .bind(contract)
            .bind(project_scope_id)
            .execute(db.db.pool())
            .await?;
        }
        None => {
            sqlx::query(
                r#"INSERT INTO operation_state(
                       operation_id,profile,current_stage,runtime_memory_contract,
                       attack_execution_contract,project_scope_id
                   ) VALUES($1,'red_team','attack_candidate','v2_only','v2_only',$2)"#,
            )
            .bind(operation_id)
            .bind(project_scope_id)
            .execute(db.db.pool())
            .await?;
        }
    }
    Ok(operation_id)
}

#[tokio::test]
#[serial]
async fn application_model_contract_defaults_legacy_and_is_immutable() {
    let database = CandidateApplicationModelDb::start("contract").await;
    let legacy_operation = insert_operation_state(&database, None)
        .await
        .expect("insert legacy operation");
    let strict_operation = insert_operation_state(&database, Some("application_model_v1"))
        .await
        .expect("insert strict operation");

    let legacy: String = sqlx::query_scalar(
        "SELECT application_model_contract FROM operation_state WHERE operation_id=$1",
    )
    .bind(legacy_operation)
    .fetch_one(database.db.pool())
    .await
    .expect("read legacy contract");
    let strict: String = sqlx::query_scalar(
        "SELECT application_model_contract FROM operation_state WHERE operation_id=$1",
    )
    .bind(strict_operation)
    .fetch_one(database.db.pool())
    .await
    .expect("read strict contract");

    assert_eq!(legacy, "legacy_no_model");
    assert_eq!(strict, "application_model_v1");

    let update = sqlx::query(
        "UPDATE operation_state SET application_model_contract='application_model_v1' \
         WHERE operation_id=$1",
    )
    .bind(legacy_operation)
    .execute(database.db.pool())
    .await;
    assert!(
        update.is_err(),
        "an operation contract must never be upgraded in place"
    );
}

#[tokio::test]
#[serial]
async fn invalid_application_model_contract_is_rejected() {
    let database = CandidateApplicationModelDb::start("invalid-contract").await;
    let result = insert_operation_state(&database, Some("model_when_convenient")).await;
    assert!(
        result.is_err(),
        "an unversioned optional-model contract must fail closed"
    );
}

#[tokio::test]
#[serial]
async fn nested_stage_fork_input_authority_schema_is_additive() {
    let database = CandidateApplicationModelDb::start("nested-stage-fork-input").await;
    let column = sqlx::query_as::<_, (String, String, String)>(
        r#"SELECT column_name,data_type,is_nullable
             FROM information_schema.columns
            WHERE table_schema='public'
              AND table_name='operation_stage_fork_inputs'
              AND column_name='fork_source_operation_id'"#,
    )
    .fetch_optional(database.db.pool())
    .await
    .expect("read nested stage-fork source column");
    assert_eq!(
        column,
        Some((
            "fork_source_operation_id".to_string(),
            "uuid".to_string(),
            "NO".to_string(),
        ))
    );

    let validator: String = sqlx::query_scalar(
        r#"SELECT pg_get_functiondef('validate_operation_stage_fork_input()'::REGPROCEDURE)"#,
    )
    .fetch_one(database.db.pool())
    .await
    .expect("read nested stage-fork input validator");
    for required in [
        "fork_source_operation_id",
        "operation_stage_fork_parent_input_is_authorized",
    ] {
        assert!(
            validator.contains(required),
            "nested stage-fork validator must bind {required}"
        );
    }

    let wave_entry_validator: String = sqlx::query_scalar(
        r#"SELECT pg_get_functiondef('enforce_attack_wave_entry_final_pass()'::REGPROCEDURE)"#,
    )
    .fetch_one(database.db.pool())
    .await
    .expect("read nested Candidate Wave entry validator");
    for required in [
        "fork_source_operation_id",
        "operation_stage_fork_parent_input_is_authorized(vuln_input)",
        "operation_stage_fork_parent_input_is_authorized(enumeration_input)",
    ] {
        assert!(
            wave_entry_validator.contains(required),
            "nested Candidate Wave entry validator must bind {required}"
        );
    }

    let application_model_validator: String = sqlx::query_scalar(
        r#"SELECT pg_get_functiondef('validate_attack_wave_application_model_authority()'::REGPROCEDURE)"#,
    )
    .fetch_one(database.db.pool())
    .await
    .expect("read nested Candidate Application Model authority validator");
    for required in [
        "wave_entry_stage_fork_input_id IS NULL",
        "model_fork_input.fork_source_operation_id",
        "operation_stage_fork_parent_input_is_authorized(input)",
    ] {
        assert!(
            application_model_validator.contains(required),
            "nested Candidate Application Model validator must bind {required}"
        );
    }
}

#[tokio::test]
#[serial]
async fn candidate_generation_application_model_authority_schema_is_additive_and_generation_aware()
{
    let database = CandidateApplicationModelDb::start("generation-authority-schema").await;
    let columns = sqlx::query_as::<_, (String, String)>(
        r#"SELECT column_name,data_type
             FROM information_schema.columns
            WHERE table_schema='public'
              AND table_name='attack_wave_application_model_authorities'
              AND column_name IN (
                    'source_consolidation_id',
                    'parent_wave_unit_id',
                    'parent_input_authority_hash'
              )
            ORDER BY column_name"#,
    )
    .fetch_all(database.db.pool())
    .await
    .expect("read additive generation authority columns");
    assert_eq!(
        columns,
        vec![
            (
                "parent_input_authority_hash".to_string(),
                "text".to_string()
            ),
            ("parent_wave_unit_id".to_string(), "uuid".to_string()),
            ("source_consolidation_id".to_string(), "uuid".to_string()),
        ]
    );

    let function_body: String = sqlx::query_scalar(
        r#"SELECT pg_get_functiondef(proc.oid)
             FROM pg_proc AS proc
             JOIN pg_namespace AS namespace ON namespace.oid=proc.pronamespace
            WHERE namespace.nspname='public'
              AND proc.proname='validate_attack_wave_application_model_authority'"#,
    )
    .fetch_one(database.db.pool())
    .await
    .expect("read generation-aware authority validator");
    for required in [
        "candidate_input_authority.v1",
        "candidate_input_authority.v2",
        "opened_next_wave",
        "parent_input_authority_hash",
        "source_consolidation_id",
    ] {
        assert!(
            function_body.contains(required),
            "generation-aware validator must bind {required}"
        );
    }

    let immutable_trigger_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM pg_trigger
            WHERE tgrelid='attack_wave_application_model_authorities'::regclass
              AND NOT tgisinternal
              AND tgname='attack_wave_application_model_authority_immutable'"#,
    )
    .fetch_one(database.db.pool())
    .await
    .expect("read immutable authority trigger");
    assert_eq!(immutable_trigger_count, 1);
}

#[tokio::test]
#[serial]
async fn candidate_adopted_application_model_authority_schema_is_additive() {
    let database = CandidateApplicationModelDb::start("adopted-authority-schema").await;
    let columns = sqlx::query_as::<_, (String, String, String)>(
        r#"SELECT column_name,data_type,is_nullable
             FROM information_schema.columns
            WHERE table_schema='public'
              AND table_name='attack_wave_application_model_authorities'
              AND column_name IN (
                    'application_model_operation_id',
                    'application_model_scope_snapshot_id',
                    'application_model_stage_fork_input_id'
              )
            ORDER BY column_name"#,
    )
    .fetch_all(database.db.pool())
    .await
    .expect("read adopted Application Model authority columns");
    assert_eq!(
        columns,
        vec![
            (
                "application_model_operation_id".to_string(),
                "uuid".to_string(),
                "NO".to_string(),
            ),
            (
                "application_model_scope_snapshot_id".to_string(),
                "uuid".to_string(),
                "NO".to_string(),
            ),
            (
                "application_model_stage_fork_input_id".to_string(),
                "uuid".to_string(),
                "YES".to_string(),
            ),
        ]
    );

    let function_body: String = sqlx::query_scalar(
        r#"SELECT pg_get_functiondef(proc.oid)
             FROM pg_proc AS proc
             JOIN pg_namespace AS namespace ON namespace.oid=proc.pronamespace
            WHERE namespace.nspname='public'
              AND proc.proname='validate_attack_wave_application_model_authority'"#,
    )
    .fetch_one(database.db.pool())
    .await
    .expect("read adopted authority validator");
    for required in [
        "candidate_input_authority.v3",
        "application_model_stage_fork_input_id",
        "application_understanding",
        "manifest_input_sha256",
    ] {
        assert!(
            function_body.contains(required),
            "adopted authority validator must bind {required}"
        );
    }

    let input_check: String = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(constraint_row.oid)
             FROM pg_constraint AS constraint_row
            WHERE constraint_row.conrelid='operation_stage_fork_inputs'::regclass
              AND constraint_row.contype='c'
              AND pg_get_constraintdef(constraint_row.oid)
                    LIKE '%operation_stage_fork_stage_rank(source_stage_kind)%'"#,
    )
    .fetch_one(database.db.pool())
    .await
    .expect("read stage-fork input source-stage check");
    assert!(
        input_check.contains("operation_stage_fork_stage_rank(source_stage_kind) >= 1")
            && input_check.contains("operation_stage_fork_stage_rank(source_stage_kind) <= 6"),
        "Candidate-only adoption must permit an AU final-seal fork input: {input_check}"
    );
}
