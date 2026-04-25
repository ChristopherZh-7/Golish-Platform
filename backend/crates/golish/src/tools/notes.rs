use serde::{Deserialize, Serialize};

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

fn to_note(n: golish_db::models::Note) -> Note {
    Note {
        id: n.id.to_string(),
        entity_type: n.entity_type,
        entity_id: n.entity_id,
        content: n.content,
        color: n.color,
        created_at: n.created_at.timestamp() as u64,
        updated_at: n.updated_at.timestamp() as u64,
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
    let rows = golish_db::repo::notes::list_filtered(
        pool,
        entity_type.as_deref(),
        entity_id.as_deref(),
        project_path.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(to_note).collect())
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
    let note = golish_db::repo::notes::create(
        pool,
        &entity_type,
        &entity_id,
        &content,
        &c,
        project_path.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(note.id.to_string())
}

#[tauri::command]
pub async fn notes_update(
    state: tauri::State<'_, AppState>,
    id: String,
    content: String,
    color: Option<String>,
    project_path: Option<String>,
) -> Result<(), String> {
    let _ = project_path;
    let pool = state.db_pool_ready().await?;
    let uid: uuid::Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let c = color.unwrap_or_else(|| "yellow".to_string());
    golish_db::repo::notes::update(pool, uid, &content, &c)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn notes_delete(
    state: tauri::State<'_, AppState>,
    id: String,
    project_path: Option<String>,
) -> Result<(), String> {
    let _ = project_path;
    let pool = state.db_pool_ready().await?;
    let uid: uuid::Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    golish_db::repo::notes::delete(pool, uid)
        .await
        .map_err(|e| e.to_string())
}
