use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::db::open_db;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub timestamp: u64,
    pub action: String,
    pub category: String,
    pub details: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
}

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[tauri::command]
pub async fn audit_log(
    action: String,
    category: String,
    details: String,
    entity_type: Option<String>,
    entity_id: Option<String>,
    project_path: Option<String>,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        conn.execute(
            "INSERT INTO audit_log (timestamp, action, category, details, entity_type, entity_id) VALUES (?1,?2,?3,?4,?5,?6)",
            params![now_ts(), action, category, details, entity_type, entity_id],
        ).map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn audit_list(
    limit: Option<usize>,
    category: Option<String>,
    project_path: Option<String>,
) -> Result<Vec<AuditEntry>, String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        let mut sql = "SELECT timestamp, action, category, details, entity_type, entity_id FROM audit_log".to_string();
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(ref cat) = category {
            sql.push_str(&format!(" WHERE category=?{}", param_values.len() + 1));
            param_values.push(Box::new(cat.clone()));
        }
        sql.push_str(" ORDER BY timestamp DESC");
        if let Some(lim) = limit {
            sql.push_str(&format!(" LIMIT {}", lim));
        }

        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let params_ref: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|b| b.as_ref()).collect();
        let entries: Vec<AuditEntry> = stmt
            .query_map(params_ref.as_slice(), |row| {
                Ok(AuditEntry {
                    timestamp: row.get(0)?,
                    action: row.get(1)?,
                    category: row.get(2)?,
                    details: row.get(3)?,
                    entity_type: row.get(4)?,
                    entity_id: row.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(entries)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn audit_clear(project_path: Option<String>) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        conn.execute("DELETE FROM audit_log", []).map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}
