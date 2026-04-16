use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::AppState;

fn non_empty(s: &str) -> Option<&str> {
    if s.is_empty() { None } else { Some(s) }
}

// ─── Audit / Operation Logs (unified) ──────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditRow {
    pub id: i64,
    pub action: String,
    pub category: String,
    pub details: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub source: String,
    pub project_path: Option<String>,
    pub target_id: Option<String>,
    pub session_id: Option<String>,
    pub tool_name: Option<String>,
    pub status: String,
    pub detail: serde_json::Value,
    pub created_at: i64,
}

impl From<golish_db::models::AuditEntry> for AuditRow {
    fn from(m: golish_db::models::AuditEntry) -> Self {
        Self {
            id: m.id,
            action: m.action,
            category: m.category,
            details: m.details,
            entity_type: m.entity_type,
            entity_id: m.entity_id,
            source: m.source,
            project_path: m.project_path,
            target_id: m.target_id.map(|id| id.to_string()),
            session_id: m.session_id,
            tool_name: m.tool_name,
            status: m.status,
            detail: m.detail,
            created_at: m.created_at.timestamp_millis(),
        }
    }
}

#[tauri::command]
pub async fn oplog_list(
    state: tauri::State<'_, AppState>,
    project_path: String,
    limit: Option<i64>,
) -> Result<Vec<AuditRow>, String> {
    let pool = state.db_pool_ready().await?;
    let rows = golish_db::repo::audit::list(pool, non_empty(&project_path), limit.unwrap_or(100))
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(AuditRow::from).collect())
}

#[tauri::command]
pub async fn oplog_list_by_target(
    state: tauri::State<'_, AppState>,
    target_id: String,
    limit: Option<i64>,
) -> Result<Vec<AuditRow>, String> {
    let pool = state.db_pool_ready().await?;
    let tid = Uuid::parse_str(&target_id).map_err(|e| e.to_string())?;
    let rows = golish_db::repo::audit::list_by_target(pool, tid, limit.unwrap_or(100))
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(AuditRow::from).collect())
}

#[tauri::command]
pub async fn oplog_list_by_type(
    state: tauri::State<'_, AppState>,
    project_path: String,
    op_type: String,
    limit: Option<i64>,
) -> Result<Vec<AuditRow>, String> {
    let pool = state.db_pool_ready().await?;
    let rows = golish_db::repo::audit::list_by_category(
        pool,
        &op_type,
        non_empty(&project_path),
        limit.unwrap_or(100),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(AuditRow::from).collect())
}

#[tauri::command]
pub async fn oplog_search(
    state: tauri::State<'_, AppState>,
    project_path: String,
    query: String,
    limit: Option<i64>,
) -> Result<Vec<AuditRow>, String> {
    let pool = state.db_pool_ready().await?;
    let rows = golish_db::repo::audit::search(
        pool,
        non_empty(&project_path),
        &query,
        limit.unwrap_or(100),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(AuditRow::from).collect())
}

#[tauri::command]
pub async fn oplog_count(
    state: tauri::State<'_, AppState>,
    project_path: String,
) -> Result<i64, String> {
    let pool = state.db_pool_ready().await?;
    golish_db::repo::audit::count(pool, non_empty(&project_path))
        .await
        .map_err(|e| e.to_string())
}

// ─── Target Assets ─────────────────────────────────────────────────────

#[tauri::command]
pub async fn target_assets_list(
    state: tauri::State<'_, AppState>,
    target_id: String,
) -> Result<serde_json::Value, String> {
    let pool = state.db_pool_ready().await?;
    let tid = Uuid::parse_str(&target_id).map_err(|e| e.to_string())?;
    let rows = golish_db::repo::target_assets::list_by_target(pool, tid)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(rows).map_err(|e| e.to_string())
}

// ─── API Endpoints ─────────────────────────────────────────────────────

#[tauri::command]
pub async fn api_endpoints_list(
    state: tauri::State<'_, AppState>,
    target_id: String,
) -> Result<serde_json::Value, String> {
    let pool = state.db_pool_ready().await?;
    let tid = Uuid::parse_str(&target_id).map_err(|e| e.to_string())?;
    let rows = golish_db::repo::api_endpoints::list_by_target(pool, tid)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(rows).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn api_endpoints_untested(
    state: tauri::State<'_, AppState>,
    target_id: String,
) -> Result<serde_json::Value, String> {
    let pool = state.db_pool_ready().await?;
    let tid = Uuid::parse_str(&target_id).map_err(|e| e.to_string())?;
    let rows = golish_db::repo::api_endpoints::list_untested(pool, tid)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(rows).map_err(|e| e.to_string())
}

// ─── Fingerprints ──────────────────────────────────────────────────────

#[tauri::command]
pub async fn fingerprints_list(
    state: tauri::State<'_, AppState>,
    target_id: String,
) -> Result<serde_json::Value, String> {
    let pool = state.db_pool_ready().await?;
    let tid = Uuid::parse_str(&target_id).map_err(|e| e.to_string())?;
    let rows = golish_db::repo::fingerprints::list_by_target(pool, tid)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(rows).map_err(|e| e.to_string())
}

// ─── JS Analysis ───────────────────────────────────────────────────────

#[tauri::command]
pub async fn js_analysis_list(
    state: tauri::State<'_, AppState>,
    target_id: String,
) -> Result<serde_json::Value, String> {
    let pool = state.db_pool_ready().await?;
    let tid = Uuid::parse_str(&target_id).map_err(|e| e.to_string())?;
    let rows = golish_db::repo::js_analysis::list_by_target(pool, tid)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(rows).map_err(|e| e.to_string())
}

// ─── Passive Scans ─────────────────────────────────────────────────────

#[tauri::command]
pub async fn passive_scans_list(
    state: tauri::State<'_, AppState>,
    target_id: String,
    limit: Option<i64>,
) -> Result<serde_json::Value, String> {
    let pool = state.db_pool_ready().await?;
    let tid = Uuid::parse_str(&target_id).map_err(|e| e.to_string())?;
    let rows = golish_db::repo::passive_scans::list_by_target(pool, tid, limit.unwrap_or(100))
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(rows).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn passive_scans_vulnerable(
    state: tauri::State<'_, AppState>,
    target_id: String,
) -> Result<serde_json::Value, String> {
    let pool = state.db_pool_ready().await?;
    let tid = Uuid::parse_str(&target_id).map_err(|e| e.to_string())?;
    let rows = golish_db::repo::passive_scans::list_vulnerable(pool, tid)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(rows).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn passive_scans_stats(
    state: tauri::State<'_, AppState>,
    target_id: String,
) -> Result<serde_json::Value, String> {
    let pool = state.db_pool_ready().await?;
    let tid = Uuid::parse_str(&target_id).map_err(|e| e.to_string())?;
    golish_db::repo::passive_scans::stats_by_target(pool, tid)
        .await
        .map_err(|e| e.to_string())
}

// ─── Target Data Aggregate ─────────────────────────────────────────────

#[tauri::command]
pub async fn target_security_overview(
    state: tauri::State<'_, AppState>,
    target_id: String,
) -> Result<serde_json::Value, String> {
    let pool = state.db_pool_ready().await?;
    let tid = Uuid::parse_str(&target_id).map_err(|e| e.to_string())?;

    let assets_count = golish_db::repo::target_assets::count_by_target(pool, tid)
        .await
        .unwrap_or(0);
    let (endpoints_total, endpoints_tested) =
        golish_db::repo::api_endpoints::count_by_target(pool, tid)
            .await
            .unwrap_or((0, 0));
    let scan_stats = golish_db::repo::passive_scans::stats_by_target(pool, tid)
        .await
        .unwrap_or_else(|_| serde_json::json!({}));

    Ok(serde_json::json!({
        "assets_count": assets_count,
        "endpoints_total": endpoints_total,
        "endpoints_tested": endpoints_tested,
        "scan_stats": scan_stats,
    }))
}
