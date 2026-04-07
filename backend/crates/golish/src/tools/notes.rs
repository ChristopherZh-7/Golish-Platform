use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::db::open_db;

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

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn row_to_note(row: &rusqlite::Row) -> rusqlite::Result<Note> {
    Ok(Note {
        id: row.get(0)?,
        entity_type: row.get(1)?,
        entity_id: row.get(2)?,
        content: row.get(3)?,
        color: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

#[tauri::command]
pub async fn notes_list(
    entity_type: Option<String>,
    entity_id: Option<String>,
    project_path: Option<String>,
) -> Result<Vec<Note>, String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        let mut sql = "SELECT id, entity_type, entity_id, content, color, created_at, updated_at FROM notes WHERE 1=1".to_string();
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(ref et) = entity_type {
            sql.push_str(&format!(" AND entity_type=?{}", param_values.len() + 1));
            param_values.push(Box::new(et.clone()));
        }
        if let Some(ref eid) = entity_id {
            sql.push_str(&format!(" AND entity_id=?{}", param_values.len() + 1));
            param_values.push(Box::new(eid.clone()));
        }
        sql.push_str(" ORDER BY created_at DESC");

        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let params_ref: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|b| b.as_ref()).collect();
        let notes: Vec<Note> = stmt
            .query_map(params_ref.as_slice(), |row| row_to_note(row))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(notes)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn notes_add(
    entity_type: String,
    entity_id: String,
    content: String,
    color: Option<String>,
    project_path: Option<String>,
) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        let ts = now_ts();
        let id = Uuid::new_v4().to_string();
        let c = color.unwrap_or_else(|| "yellow".to_string());
        conn.execute(
            "INSERT INTO notes (id, entity_type, entity_id, content, color, created_at, updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![id, entity_type, entity_id, content, c, ts, ts],
        ).map_err(|e| e.to_string())?;
        Ok(id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn notes_update(
    id: String,
    content: String,
    color: Option<String>,
    project_path: Option<String>,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        let ts = now_ts();
        conn.execute(
            "UPDATE notes SET content=?1, updated_at=?2 WHERE id=?3",
            params![content, ts, id],
        ).map_err(|e| e.to_string())?;
        if let Some(c) = color {
            conn.execute("UPDATE notes SET color=?1 WHERE id=?2", params![c, id])
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn notes_delete(id: String, project_path: Option<String>) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        conn.execute("DELETE FROM notes WHERE id=?1", params![id]).map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}
