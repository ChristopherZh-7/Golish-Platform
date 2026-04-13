use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub content: String,
    pub color: String,
    pub created_at: u64,
    pub updated_at: u64,
}

fn ts(dt: chrono::DateTime<chrono::Utc>) -> u64 {
    dt.timestamp() as u64
}

#[derive(sqlx::FromRow)]
struct NoteRow {
    id: Uuid,
    entity_type: String,
    entity_id: String,
    content: String,
    color: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<NoteRow> for Note {
    fn from(r: NoteRow) -> Self {
        Note {
            id: r.id.to_string(),
            entity_type: r.entity_type,
            entity_id: r.entity_id,
            content: r.content,
            color: r.color,
            created_at: ts(r.created_at),
            updated_at: ts(r.updated_at),
        }
    }
}

#[tauri::command]
pub async fn notes_list(
    state: tauri::State<'_, AppState>,
    entity_type: Option<String>,
    entity_id: Option<String>,
    project_path: Option<String>,
) -> Result<Vec<Note>, String> {
    let pool = state.db_pool_ready().await?;
    let rows = sqlx::query_as::<_, NoteRow>(
        r#"SELECT id, entity_type, entity_id, content, color, created_at, updated_at
           FROM notes
           WHERE ($1::text IS NULL OR entity_type = $1)
             AND ($2::text IS NULL OR entity_id = $2)
             AND project_path IS NOT DISTINCT FROM $3
           ORDER BY created_at DESC"#,
    )
    .bind(entity_type.as_deref())
    .bind(entity_id.as_deref())
    .bind(project_path.as_deref())
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(Note::from).collect())
}

#[tauri::command]
pub async fn notes_add(
    state: tauri::State<'_, AppState>,
    entity_type: String,
    entity_id: String,
    content: String,
    color: Option<String>,
    project_path: Option<String>,
) -> Result<String, String> {
    let pool = state.db_pool_ready().await?;
    let c = color.unwrap_or_else(|| "yellow".to_string());
    let id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO notes (entity_type, entity_id, content, color, project_path)
           VALUES ($1, $2, $3, $4, $5) RETURNING id"#,
    )
    .bind(&entity_type)
    .bind(&entity_id)
    .bind(&content)
    .bind(&c)
    .bind(project_path.as_deref())
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(id.to_string())
}

#[tauri::command]
pub async fn notes_update(
    state: tauri::State<'_, AppState>,
    id: String,
    content: String,
    color: Option<String>,
    project_path: Option<String>,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    let _ = project_path;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let c = color.unwrap_or_else(|| "yellow".to_string());
    sqlx::query("UPDATE notes SET content=$1, color=$2, updated_at=NOW() WHERE id=$3")
        .bind(&content)
        .bind(&c)
        .bind(uid)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn notes_delete(
    state: tauri::State<'_, AppState>,
    id: String,
    project_path: Option<String>,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    let _ = project_path;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    sqlx::query("DELETE FROM notes WHERE id=$1")
        .bind(uid)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
