use crate::error::GolishError;
use crate::state::DbState;
use serde::{Deserialize, Serialize};

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
    state: tauri::State<'_, DbState>,
    action: String,
    category: String,
    details: String,
    entity_type: Option<String>,
    entity_id: Option<String>,
    project_path: Option<String>,
    source: Option<String>,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
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
?;
    Ok(())
}

#[tauri::command]
pub async fn audit_list(
    state: tauri::State<'_, DbState>,
    limit: Option<i64>,
    category: Option<String>,
    project_path: Option<String>,
) -> Result<Vec<AuditEntry>, GolishError> {
    let pool = state.pool_ready().await?;
    let lim = limit.unwrap_or(500);
    let pp = project_path.unwrap_or_default();
    let rows = golish_db::repo::audit::list_by_project_exact::<AuditRow>(
        pool,
        category.as_deref(),
        &pp,
        lim,
    )
    .await?;
    Ok(rows.into_iter().map(AuditEntry::from).collect())
}

#[tauri::command]
pub async fn audit_clear(
    state: tauri::State<'_, DbState>,
    project_path: Option<String>,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    let pp = project_path.unwrap_or_default();
    golish_db::repo::audit::clear_by_project_exact(pool, &pp).await?;
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
    state: tauri::State<'_, DbState>,
    project_path: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<PassiveScanRow>, GolishError> {
    let pool = state.pool_ready().await?;
    let lim = limit.unwrap_or(200);
    let pp = project_path.unwrap_or_default();
    let rows =
        golish_db::repo::passive_scans::list_global_by_project::<PassiveScanRow>(pool, &pp, lim)
            .await?;
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
    state: tauri::State<'_, DbState>,
    project_path: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<AgentLogRow>, GolishError> {
    let pool = state.pool_ready().await?;
    let lim = limit.unwrap_or(200);
    let pp = project_path.unwrap_or_default();
    let rows = golish_db::repo::agent_logs::list_by_project::<AgentLogRow>(pool, &pp, lim).await?;
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
    state: tauri::State<'_, DbState>,
    project_path: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<TerminalLogRow>, GolishError> {
    let pool = state.pool_ready().await?;
    let lim = limit.unwrap_or(200);
    let pp = project_path.unwrap_or_default();
    let rows =
        golish_db::repo::terminal_logs::list_by_project::<TerminalLogRow>(pool, &pp, lim).await?;
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
    state: tauri::State<'_, DbState>,
    project_path: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<SearchLogRow>, GolishError> {
    let pool = state.pool_ready().await?;
    let lim = limit.unwrap_or(200);
    let pp = project_path.unwrap_or_default();
    let rows =
        golish_db::repo::search_logs::list_by_project::<SearchLogRow>(pool, &pp, lim).await?;
    Ok(rows)
}

// ── Target activity timeline (cross-table aggregate) ───────────────────

/// Return a per-target activity timeline aggregated from `audit_log`,
/// `target_assets`, `api_endpoints`, `passive_scan_logs`, and `findings`,
/// newest event first. The shape is `golish_db::repo::audit::TimelineEntry`
/// (`source / event / category / details / toolName / status / detail /
/// createdAt`); see the DAO docstring for the exact field semantics.
#[tauri::command]
pub async fn target_timeline(
    state: tauri::State<'_, DbState>,
    target_id: String,
    limit: Option<i64>,
) -> Result<Vec<golish_db::repo::audit::TimelineEntry>, GolishError> {
    let pool = state.pool_ready().await?;
    let tid = uuid::Uuid::parse_str(&target_id)?;
    let rows = golish_db::repo::audit::target_timeline(pool, tid, limit.unwrap_or(200)).await?;
    Ok(rows)
}
