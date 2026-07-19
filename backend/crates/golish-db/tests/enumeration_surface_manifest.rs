use golish_db::{DbConfig, GolishDb};
use serial_test::serial;
use sqlx::Row;
use tempfile::TempDir;
use uuid::Uuid;

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

    pg.stop().await;
}
