use serde::{Deserialize, Serialize};

use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingMeta {
    pub id: String,
    pub title: String,
    pub session_id: String,
    pub width: u16,
    pub height: u16,
    pub duration_ms: u64,
    pub event_count: usize,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recording {
    pub meta: RecordingMeta,
    pub events: Vec<(f64, String)>,
}

#[derive(sqlx::FromRow)]
struct RecordingRow {
    id: String,
    title: String,
    session_id: String,
    width: i16,
    height: i16,
    duration_ms: i64,
    event_count: i32,
    events: serde_json::Value,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(sqlx::FromRow)]
struct MetaRow {
    id: String,
    title: String,
    session_id: String,
    width: i16,
    height: i16,
    duration_ms: i64,
    event_count: i32,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl From<MetaRow> for RecordingMeta {
    fn from(r: MetaRow) -> Self {
        Self {
            id: r.id,
            title: r.title,
            session_id: r.session_id,
            width: r.width as u16,
            height: r.height as u16,
            duration_ms: r.duration_ms as u64,
            event_count: r.event_count as usize,
            created_at: r.created_at.to_rfc3339(),
        }
    }
}

impl From<RecordingRow> for Recording {
    fn from(r: RecordingRow) -> Self {
        let events: Vec<(f64, String)> =
            serde_json::from_value(r.events).unwrap_or_default();
        Self {
            meta: RecordingMeta {
                id: r.id,
                title: r.title,
                session_id: r.session_id,
                width: r.width as u16,
                height: r.height as u16,
                duration_ms: r.duration_ms as u64,
                event_count: r.event_count as usize,
                created_at: r.created_at.to_rfc3339(),
            },
            events,
        }
    }
}

#[tauri::command]
pub async fn recording_save(
    state: tauri::State<'_, AppState>,
    recording: Recording,
    _project_path: Option<String>,
) -> Result<String, String> {
    let pool = state.db_pool_ready().await?;
    let events_json = serde_json::to_value(&recording.events).map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO recordings (id, title, session_id, width, height, duration_ms, event_count, events, project_path) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
         ON CONFLICT (id) DO UPDATE SET \
           title = EXCLUDED.title, events = EXCLUDED.events, \
           duration_ms = EXCLUDED.duration_ms, event_count = EXCLUDED.event_count",
    )
    .bind(&recording.meta.id)
    .bind(&recording.meta.title)
    .bind(&recording.meta.session_id)
    .bind(recording.meta.width as i16)
    .bind(recording.meta.height as i16)
    .bind(recording.meta.duration_ms as i64)
    .bind(recording.meta.event_count as i32)
    .bind(&events_json)
    .bind(&_project_path)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    tracing::debug!(
        "[recording_save] Saved recording {} ({} events)",
        recording.meta.id,
        recording.meta.event_count
    );
    Ok(recording.meta.id)
}

#[tauri::command]
pub async fn recording_load(
    state: tauri::State<'_, AppState>,
    id: String,
    _project_path: Option<String>,
) -> Result<Recording, String> {
    let pool = state.db_pool_ready().await?;
    let row: RecordingRow = sqlx::query_as(
        "SELECT id, title, session_id, width, height, duration_ms, event_count, events, created_at \
         FROM recordings WHERE id = $1",
    )
    .bind(&id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| format!("Recording {id} not found"))?;

    Ok(Recording::from(row))
}

#[tauri::command]
pub async fn recording_list(
    state: tauri::State<'_, AppState>,
    _project_path: Option<String>,
) -> Result<Vec<RecordingMeta>, String> {
    let pool = state.db_pool_ready().await?;
    let rows: Vec<MetaRow> = sqlx::query_as(
        "SELECT id, title, session_id, width, height, duration_ms, event_count, created_at \
         FROM recordings ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows.into_iter().map(RecordingMeta::from).collect())
}

#[tauri::command]
pub async fn recording_delete(
    state: tauri::State<'_, AppState>,
    id: String,
    _project_path: Option<String>,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    sqlx::query("DELETE FROM recordings WHERE id = $1")
        .bind(&id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    tracing::debug!("[recording_delete] Deleted recording {id}");
    Ok(())
}
