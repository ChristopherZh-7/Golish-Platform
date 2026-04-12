use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::AppState;

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

#[tauri::command]
pub async fn scan_queue_list(
    state: tauri::State<'_, AppState>,
    project_path: Option<String>,
) -> Result<Vec<ScanEndpoint>, String> {
    let pool = &*state.db_pool;
    let rows: Vec<(String, String, Option<String>, i32, String, serde_json::Value, i64)> =
        sqlx::query_as(
            "SELECT id::text, url, scan_id, progress, status, alerts, added_at \
             FROM scan_queue WHERE project_path IS NOT DISTINCT FROM $1 \
             ORDER BY added_at ASC",
        )
        .bind(project_path.as_deref())
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(rows
        .into_iter()
        .map(|(id, url, scan_id, progress, status, alerts, added_at)| ScanEndpoint {
            id: Some(id),
            url,
            scan_id,
            progress,
            status,
            alerts,
            added_at,
        })
        .collect())
}

#[tauri::command]
pub async fn scan_queue_upsert(
    state: tauri::State<'_, AppState>,
    endpoint: ScanEndpoint,
    project_path: Option<String>,
) -> Result<String, String> {
    let pool = &*state.db_pool;
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
    .map_err(|e| e.to_string())?;

    Ok(id.to_string())
}

#[tauri::command]
pub async fn scan_queue_save_all(
    state: tauri::State<'_, AppState>,
    endpoints: Vec<ScanEndpoint>,
    project_path: Option<String>,
) -> Result<(), String> {
    let pool = &*state.db_pool;

    // Delete existing entries for this project, then re-insert
    sqlx::query("DELETE FROM scan_queue WHERE project_path IS NOT DISTINCT FROM $1")
        .bind(project_path.as_deref())
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

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
        .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[tauri::command]
pub async fn scan_queue_remove(
    state: tauri::State<'_, AppState>,
    url: String,
    project_path: Option<String>,
) -> Result<(), String> {
    let pool = &*state.db_pool;
    sqlx::query(
        "DELETE FROM scan_queue WHERE url = $1 AND project_path IS NOT DISTINCT FROM $2",
    )
    .bind(&url)
    .bind(project_path.as_deref())
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn scan_queue_clear_completed(
    state: tauri::State<'_, AppState>,
    project_path: Option<String>,
) -> Result<(), String> {
    let pool = &*state.db_pool;
    sqlx::query(
        "DELETE FROM scan_queue WHERE status = 'complete' AND project_path IS NOT DISTINCT FROM $1",
    )
    .bind(project_path.as_deref())
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}
