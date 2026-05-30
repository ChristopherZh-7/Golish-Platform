use crate::error::GolishError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::DbState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanEndpoint {
    #[serde(default)]
    pub id: Option<String>,
    pub url: String,
    #[serde(rename = "scanId", default)]
    pub scan_id: Option<String>,
    #[serde(default)]
    pub progress: i32,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default)]
    pub alerts: serde_json::Value,
    #[serde(rename = "addedAt", default)]
    pub added_at: i64,
}

fn default_status() -> String {
    "queued".to_string()
}

type ScanQueueRow = (
    String,
    String,
    Option<String>,
    i32,
    String,
    serde_json::Value,
    i64,
);

#[tauri::command]
pub async fn scan_queue_list(
    state: tauri::State<'_, DbState>,
    project_path: Option<String>,
) -> Result<Vec<ScanEndpoint>, GolishError> {
    let pool = state.pool_ready().await?;
    let rows: Vec<ScanQueueRow> =
        golish_db::repo::scan_queue::list_by_project(pool, project_path.as_deref()).await?;

    Ok(rows
        .into_iter()
        .map(
            |(id, url, scan_id, progress, status, alerts, added_at)| ScanEndpoint {
                id: Some(id),
                url,
                scan_id,
                progress,
                status,
                alerts,
                added_at,
            },
        )
        .collect())
}

#[tauri::command]
pub async fn scan_queue_upsert(
    state: tauri::State<'_, DbState>,
    endpoint: ScanEndpoint,
    project_path: Option<String>,
) -> Result<String, GolishError> {
    let pool = state.pool_ready().await?;
    let id: Uuid = endpoint
        .id
        .as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(Uuid::new_v4);

    sqlx::query(
        r#"INSERT INTO scan_queue (id, url, scan_id, progress, status, alerts, added_at, project_path)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           ON CONFLICT (id) DO UPDATE SET
             scan_id = EXCLUDED.scan_id,
             progress = EXCLUDED.progress,
             status = EXCLUDED.status,
             alerts = EXCLUDED.alerts,
             updated_at = NOW()"#,
    )
    .bind(id)
    .bind(&endpoint.url)
    .bind(&endpoint.scan_id)
    .bind(endpoint.progress)
    .bind(&endpoint.status)
    .bind(&endpoint.alerts)
    .bind(endpoint.added_at)
    .bind(project_path.as_deref())
    .execute(pool)
    .await
?;

    Ok(id.to_string())
}

#[tauri::command]
pub async fn scan_queue_save_all(
    state: tauri::State<'_, DbState>,
    endpoints: Vec<ScanEndpoint>,
    project_path: Option<String>,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;

    // Delete existing entries for this project, then re-insert
    golish_db::repo::scan_queue::clear_by_project(pool, project_path.as_deref()).await?;

    for ep in &endpoints {
        let id: Uuid = ep
            .id
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(Uuid::new_v4);

        sqlx::query(
            r#"INSERT INTO scan_queue (id, url, scan_id, progress, status, alerts, added_at, project_path)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
        )
        .bind(id)
        .bind(&ep.url)
        .bind(&ep.scan_id)
        .bind(ep.progress)
        .bind(&ep.status)
        .bind(&ep.alerts)
        .bind(ep.added_at)
        .bind(project_path.as_deref())
        .execute(pool)
        .await
?;
    }

    Ok(())
}

#[tauri::command]
pub async fn scan_queue_remove(
    state: tauri::State<'_, DbState>,
    url: String,
    project_path: Option<String>,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    golish_db::repo::scan_queue::delete_by_url(pool, &url, project_path.as_deref()).await?;
    Ok(())
}

#[tauri::command]
pub async fn scan_queue_clear_completed(
    state: tauri::State<'_, DbState>,
    project_path: Option<String>,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    golish_db::repo::scan_queue::clear_completed(pool, project_path.as_deref()).await?;
    Ok(())
}
