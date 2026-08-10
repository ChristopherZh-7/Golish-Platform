use golish_db::{DbConfig, GolishDb};
use serial_test::serial;
use sqlx::Row;
use tempfile::TempDir;
use uuid::Uuid;

const DEFAULT_PORT_ORIGIN_MIGRATION: &str =
    include_str!("../migrations/20260810000001_enumeration_endpoint_default_port_origin.sql");
const DEFAULT_PORT_OCCURRENCE_MIGRATION: &str =
    include_str!("../migrations/20260810000002_enumeration_occurrence_default_port_origin.sql");

fn reserve_local_port() -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

struct TestPg {
    db: GolishDb,
    _data_dir: TempDir,
}

impl TestPg {
    async fn start() -> Self {
        let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
        let config = DbConfig {
            pg_data_dir: data_dir.path().join("pgdata"),
            port: reserve_local_port().expect("available postgres port"),
            database: format!("enumeration_surface_{}", Uuid::new_v4().simple()),
            ..DbConfig::default()
        };
        let db = GolishDb::start(config)
            .await
            .expect("start migrated embedded postgres");
        Self {
            db,
            _data_dir: data_dir,
        }
    }

    async fn stop(mut self) {
        self.db.stop().await;
    }
}

#[tokio::test]
#[serial]
async fn enumeration_surface_manifest_migration_creates_normalized_relations() {
    let pg = TestPg::start().await;
    let pool = pg.db.pool();

    for table in [
        "fingerprint_origin_observations",
        "enumeration_endpoint_observations",
        "enumeration_endpoint_parameters",
    ] {
        let row = sqlx::query("SELECT to_regclass($1)::text AS relation")
            .bind(format!("public.{table}"))
            .fetch_one(pool)
            .await
            .expect("query migration relation");
        let relation: Option<String> = row.get("relation");
        assert_eq!(relation.as_deref(), Some(table), "missing table {table}");
    }

    for constraint in [
        "fingerprint_origin_source_not_empty",
        "enumeration_endpoint_source_not_empty",
        "enumeration_endpoint_parameter_name_not_empty",
        "enumeration_endpoint_parameter_source_not_empty",
        "enumeration_endpoint_parameter_location_check",
    ] {
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = $1)",
        )
        .bind(constraint)
        .fetch_one(pool)
        .await
        .expect("query migration constraint");
        assert!(exists, "missing constraint {constraint}");
    }

    for trigger in [
        "fingerprint_origin_observations_validate",
        "enumeration_endpoint_observations_validate",
    ] {
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM pg_trigger WHERE tgname = $1 AND NOT tgisinternal)",
        )
        .bind(trigger)
        .fetch_one(pool)
        .await
        .expect("query migration trigger");
        assert!(exists, "missing trigger {trigger}");
    }

    let validator = sqlx::query_scalar::<_, String>(
        "SELECT pg_get_functiondef('validate_enumeration_endpoint_observation()'::regprocedure)",
    )
    .fetch_one(pool)
    .await
    .expect("read installed endpoint observation validator");
    for required in [
        "enumeration_url_matches_web_origin",
        "split_part(split_part(ae.url, '#', 1), '?', 1)",
        "NEW.web_origin_id",
    ] {
        assert!(
            validator.contains(required),
            "installed endpoint observation validator is missing `{required}`"
        );
        assert!(
            DEFAULT_PORT_ORIGIN_MIGRATION.contains(required),
            "default-port migration is missing `{required}`"
        );
    }
    assert!(!validator.contains("ae.url = wo.origin"));

    let occurrence_validator = sqlx::query_scalar::<_, String>(
        "SELECT pg_get_functiondef('enumeration_validate_occurrence()'::regprocedure)",
    )
    .fetch_one(pool)
    .await
    .expect("read installed endpoint occurrence validator");
    for required in [
        "enumeration_url_matches_web_origin",
        "NEW.canonical_request_url",
        "NEW.resolved_web_origin_id",
    ] {
        assert!(
            occurrence_validator.contains(required),
            "installed endpoint occurrence validator is missing `{required}`"
        );
        assert!(
            DEFAULT_PORT_OCCURRENCE_MIGRATION.contains(required),
            "default-port occurrence migration is missing `{required}`"
        );
    }
    assert!(!occurrence_validator.contains("NEW.canonical_request_url=origin.origin"));

    pg.stop().await;
}
