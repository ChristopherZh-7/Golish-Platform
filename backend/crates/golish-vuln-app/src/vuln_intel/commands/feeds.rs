//! Tauri commands: vuln-feed CRUD.

use golish_vuln_intel::{FeedRow, PgVulnIntelStore, VulnFeed, VulnIntelStore as _};

use golish_app_core::DbState;
use golish_app_core::GolishError;

#[tauri::command]
pub async fn intel_list_feeds(
    state: tauri::State<'_, DbState>,
) -> Result<Vec<VulnFeed>, GolishError> {
    let pool = state.pool_ready().await?;
    PgVulnIntelStore::new(pool).ensure_default_feeds().await?;
    let rows: Vec<FeedRow> =
        sqlx::query_as("SELECT id, name, feed_type, url, enabled, last_fetched FROM vuln_feeds")
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(VulnFeed::from).collect())
}

#[tauri::command]
pub async fn intel_add_feed(
    state: tauri::State<'_, DbState>,
    name: String,
    feed_type: String,
    url: String,
) -> Result<String, GolishError> {
    let pool = state.pool_ready().await?;
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO vuln_feeds (id, name, feed_type, url, enabled) VALUES ($1, $2, $3, $4, true)",
    )
    .bind(&id)
    .bind(&name)
    .bind(&feed_type)
    .bind(&url)
    .execute(pool)
    .await?;
    Ok(id)
}

#[tauri::command]
pub async fn intel_toggle_feed(
    state: tauri::State<'_, DbState>,
    id: String,
    enabled: bool,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    sqlx::query("UPDATE vuln_feeds SET enabled=$1 WHERE id=$2")
        .bind(enabled)
        .bind(&id)
        .execute(pool)
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn intel_delete_feed(
    state: tauri::State<'_, DbState>,
    id: String,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    sqlx::query("DELETE FROM vuln_feeds WHERE id=$1")
        .bind(&id)
        .execute(pool)
        .await?;
    Ok(())
}
