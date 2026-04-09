use serde::{Deserialize, Serialize};

use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub timestamp: u64,
    pub action: String,
    pub category: String,
    pub details: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    #[serde(default = "default_source")]
    pub source: String,
}

fn default_source() -> String {
    "manual".to_string()
}

#[derive(sqlx::FromRow)]
struct AuditRow {
    created_at: chrono::DateTime<chrono::Utc>,
    action: String,
    category: String,
    details: String,
    entity_type: Option<String>,
    entity_id: Option<String>,
    source: String,
}

impl From<AuditRow> for AuditEntry {
    fn from(r: AuditRow) -> Self {
        AuditEntry {
            timestamp: r.created_at.timestamp() as u64,
            action: r.action,
            category: r.category,
            details: r.details,
            entity_type: r.entity_type,
            entity_id: r.entity_id,
            source: r.source,
        }
    }
}

#[tauri::command]
pub async fn audit_log(
    state: tauri::State<'_, AppState>,
    action: String,
    category: String,
    details: String,
    entity_type: Option<String>,
    entity_id: Option<String>,
    project_path: Option<String>,
    source: Option<String>,
) -> Result<(), String> {
    let pool = &*state.db_pool;
    let src = source.unwrap_or_else(|| "manual".to_string());
    sqlx::query(
        r#"INSERT INTO audit_log (action, category, details, entity_type, entity_id, project_path, source)
           VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
    )
    .bind(&action)
    .bind(&category)
    .bind(&details)
    .bind(entity_type.as_deref())
    .bind(entity_id.as_deref())
    .bind(project_path.as_deref())
    .bind(&src)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn audit_list(
    state: tauri::State<'_, AppState>,
    limit: Option<i64>,
    category: Option<String>,
    project_path: Option<String>,
) -> Result<Vec<AuditEntry>, String> {
    let pool = &*state.db_pool;
    let lim = limit.unwrap_or(500);
    let rows = sqlx::query_as::<_, AuditRow>(
        r#"SELECT created_at, action, category, details, entity_type, entity_id, source
           FROM audit_log
           WHERE ($1::text IS NULL OR category = $1)
             AND project_path IS NOT DISTINCT FROM $2
           ORDER BY created_at DESC LIMIT $3"#,
    )
    .bind(category.as_deref())
    .bind(project_path.as_deref())
    .bind(lim)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(AuditEntry::from).collect())
}

#[tauri::command]
pub async fn audit_clear(
    state: tauri::State<'_, AppState>,
    project_path: Option<String>,
) -> Result<(), String> {
    let pool = &*state.db_pool;
    sqlx::query("DELETE FROM audit_log WHERE project_path IS NOT DISTINCT FROM $1")
        .bind(project_path.as_deref())
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
