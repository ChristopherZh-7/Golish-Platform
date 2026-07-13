use golish_db::{DbConfig, GolishDb};
use serial_test::serial;
use uuid::Uuid;

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read local postgres port")
        .port()
}

async fn migrated_db() -> (GolishDb, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let config = DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("trusted_operator_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    };
    let db = GolishDb::start(config)
        .await
        .expect("start migrated embedded postgres");
    (db, data_dir)
}

#[tokio::test]
#[serial]
async fn migration_seeds_exactly_one_immutable_active_local_operator() {
    let (mut db, _data_dir) = migrated_db().await;
    let rows = sqlx::query_as::<_, (Uuid, String, bool)>(
        "SELECT id, principal_kind, active FROM operator_principals ORDER BY created_at",
    )
    .fetch_all(db.pool())
    .await
    .expect("read seeded local principal");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, "local_operator");
    assert!(rows[0].2);

    let delete = sqlx::query("DELETE FROM operator_principals WHERE id=$1")
        .bind(rows[0].0)
        .execute(db.pool())
        .await;
    assert!(delete.is_err(), "operator identity history is retained");
    let rewrite =
        sqlx::query("UPDATE operator_principals SET id=$2, principal_kind='remote' WHERE id=$1")
            .bind(rows[0].0)
            .bind(Uuid::new_v4())
            .execute(db.pool())
            .await;
    assert!(rewrite.is_err(), "operator identity cannot be rebound");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn repository_returns_the_seeded_active_principal_without_request_identity() {
    let (mut db, _data_dir) = migrated_db().await;
    let first = golish_db::repo::operator_principals::current_local(db.pool())
        .await
        .expect("load active local principal");
    let second = golish_db::repo::operator_principals::current_local(db.pool())
        .await
        .expect("reload active local principal");
    assert_eq!(first.id, second.id);
    assert_eq!(first.principal_kind, "local_operator");
    assert!(first.active);

    db.stop().await;
}
