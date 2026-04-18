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
    let pool = state.db_pool_ready().await?;
    let src = source.unwrap_or_else(|| "manual".to_string());
    let pp = project_path.unwrap_or_default();
    sqlx::query(
        r#"INSERT INTO audit_log (action, category, details, entity_type, entity_id, project_path, source)
           VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
    )
    .bind(&action)
    .bind(&category)
    .bind(&details)
    .bind(entity_type.as_deref())
    .bind(entity_id.as_deref())
    .bind(&pp)
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
    let pool = state.db_pool_ready().await?;
    let lim = limit.unwrap_or(500);
    let pp = project_path.unwrap_or_default();
    let rows = sqlx::query_as::<_, AuditRow>(
        r#"SELECT created_at, action, category, details, entity_type, entity_id, source
           FROM audit_log
           WHERE ($1::text IS NULL OR category = $1)
             AND project_path = $2
           ORDER BY created_at DESC LIMIT $3"#,
    )
    .bind(category.as_deref())
    .bind(&pp)
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
    let pool = state.db_pool_ready().await?;
    let pp = project_path.unwrap_or_default();
    sqlx::query("DELETE FROM audit_log WHERE project_path = $1")
        .bind(&pp)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ── Passive scan logs (global) ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct PassiveScanRow {
    id: uuid::Uuid,
    target_id: uuid::Uuid,
    test_type: String,
    payload: String,
    url: String,
    result: String,
    severity: String,
    tool_used: String,
    tested_at: chrono::DateTime<chrono::Utc>,
}

#[tauri::command]
pub async fn passive_scans_global(
    state: tauri::State<'_, AppState>,
    project_path: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<PassiveScanRow>, String> {
    let pool = state.db_pool_ready().await?;
    let lim = limit.unwrap_or(200);
    let pp = project_path.unwrap_or_default();
    let rows = sqlx::query_as::<_, PassiveScanRow>(
        "SELECT id, target_id, test_type, payload, url, result, severity, tool_used, tested_at \
         FROM passive_scan_logs WHERE project_path = $1 ORDER BY tested_at DESC LIMIT $2",
    )
    .bind(&pp)
    .bind(lim)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows)
}

// ── Agent logs ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AgentLogRow {
    id: uuid::Uuid,
    session_id: uuid::Uuid,
    task_id: Option<uuid::Uuid>,
    subtask_id: Option<uuid::Uuid>,
    initiator: String,
    executor: String,
    task: String,
    result: Option<String>,
    duration_ms: Option<i32>,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[tauri::command]
pub async fn agent_logs_list(
    state: tauri::State<'_, AppState>,
    project_path: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<AgentLogRow>, String> {
    let pool = state.db_pool_ready().await?;
    let lim = limit.unwrap_or(200);
    let pp = project_path.unwrap_or_default();
    let rows = sqlx::query_as::<_, AgentLogRow>(
        "SELECT id, session_id, task_id, subtask_id, initiator::text, executor::text, task, result, duration_ms, created_at \
         FROM agent_logs WHERE project_path = $1 ORDER BY created_at DESC LIMIT $2",
    )
    .bind(&pp)
    .bind(lim)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows)
}

// ── Terminal logs ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct TerminalLogRow {
    id: uuid::Uuid,
    session_id: uuid::Uuid,
    task_id: Option<uuid::Uuid>,
    subtask_id: Option<uuid::Uuid>,
    stream: String,
    content: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[tauri::command]
pub async fn terminal_logs_list(
    state: tauri::State<'_, AppState>,
    project_path: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<TerminalLogRow>, String> {
    let pool = state.db_pool_ready().await?;
    let lim = limit.unwrap_or(200);
    let pp = project_path.unwrap_or_default();
    let rows = sqlx::query_as::<_, TerminalLogRow>(
        "SELECT id, session_id, task_id, subtask_id, stream::text, content, created_at \
         FROM terminal_logs WHERE project_path = $1 ORDER BY created_at DESC LIMIT $2",
    )
    .bind(&pp)
    .bind(lim)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows)
}

// ── Search logs ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SearchLogRow {
    id: uuid::Uuid,
    session_id: uuid::Uuid,
    task_id: Option<uuid::Uuid>,
    subtask_id: Option<uuid::Uuid>,
    initiator: Option<String>,
    engine: String,
    query: String,
    result: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[tauri::command]
pub async fn search_logs_list(
    state: tauri::State<'_, AppState>,
    project_path: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<SearchLogRow>, String> {
    let pool = state.db_pool_ready().await?;
    let lim = limit.unwrap_or(200);
    let pp = project_path.unwrap_or_default();
    let rows = sqlx::query_as::<_, SearchLogRow>(
        "SELECT id, session_id, task_id, subtask_id, initiator::text, engine, query, result, created_at \
         FROM search_logs WHERE project_path = $1 ORDER BY created_at DESC LIMIT $2",
    )
    .bind(&pp)
    .bind(lim)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows)
}
