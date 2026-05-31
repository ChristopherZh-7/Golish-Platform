use golish_app_core::GolishError;
use serde::{Deserialize, Serialize};

use golish_app_core::DbState;

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct Note {
    pub id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub content: String,
    pub color: String,
    #[ts(type = "number")]
    pub created_at: u64,
    #[ts(type = "number")]
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
    state: tauri::State<'_, DbState>,
    entity_type: Option<String>,
    entity_id: Option<String>,
    project_path: Option<String>,
) -> Result<Vec<Note>, GolishError> {
    let pool = state.pool_ready().await?;
    let rows = golish_db::repo::notes::list_filtered(
        pool,
        entity_type.as_deref(),
        entity_id.as_deref(),
        project_path.as_deref(),
    )
    .await?;
    Ok(rows.into_iter().map(to_note).collect())
}

#[tauri::command]
pub async fn notes_add(
    state: tauri::State<'_, DbState>,
    entity_type: String,
    entity_id: String,
    content: String,
    color: Option<String>,
    project_path: Option<String>,
) -> Result<String, GolishError> {
    let pool = state.pool_ready().await?;
    let c = color.unwrap_or_else(|| "yellow".to_string());
    let note = golish_db::repo::notes::create(
        pool,
        &entity_type,
        &entity_id,
        &content,
        &c,
        project_path.as_deref(),
    )
    .await?;
    Ok(note.id.to_string())
}

#[tauri::command]
pub async fn notes_update(
    state: tauri::State<'_, DbState>,
    id: String,
    content: String,
    color: Option<String>,
    project_path: Option<String>,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    let uid: uuid::Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let c = color.unwrap_or_else(|| "yellow".to_string());
    // Scoping guard (AGENTS.md I2): only update a note in the caller's project.
    let affected =
        golish_db::repo::notes::update(pool, uid, &content, &c, project_path.as_deref()).await?;
    golish_app_core::scoping::ensure_scoped_mutation(affected)
}

#[tauri::command]
pub async fn notes_delete(
    state: tauri::State<'_, DbState>,
    id: String,
    project_path: Option<String>,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    let uid: uuid::Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    // Scoping guard (AGENTS.md I2): only delete a note in the caller's project.
    let affected = golish_db::repo::notes::delete(pool, uid, project_path.as_deref()).await?;
    golish_app_core::scoping::ensure_scoped_mutation(affected)
}
