//! Analytics commands powered by the PostgreSQL database layer.
//!
//! Provides tool call stats, token usage stats, memories, and audit log access.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::state::AppState;

// -- Tool call analytics ---------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolCallStats {
    pub name: String,
    pub total_count: i64,
    pub total_duration_ms: i64,
    pub avg_duration_ms: f64,
}

#[tauri::command]
pub async fn get_tool_call_stats(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<Vec<ToolCallStats>, String> {
    let sid = session_id
        .as_deref()
        .and_then(|s| uuid::Uuid::parse_str(s).ok());

    let rows = golish_db::repo::tool_calls::stats_by_name(&state.db_pool, sid)
        .await
        .map_err(|e| e.to_string())?;

    Ok(rows
        .into_iter()
        .map(|r| ToolCallStats {
            name: r.name,
            total_count: r.total_count,
            total_duration_ms: r.total_duration_ms,
            avg_duration_ms: r.avg_duration_ms,
        })
        .collect())
}

// -- Token usage analytics -------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct TokenUsageStats {
    pub total_tokens_in: i64,
    pub total_tokens_out: i64,
    pub total_cost_in: f64,
    pub total_cost_out: f64,
}

#[tauri::command]
pub async fn get_db_token_usage_stats(
    state: State<'_, AppState>,
) -> Result<TokenUsageStats, String> {
    let stats = golish_db::repo::message_chains::usage_stats_total(&state.db_pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(TokenUsageStats {
        total_tokens_in: stats.total_tokens_in,
        total_tokens_out: stats.total_tokens_out,
        total_cost_in: stats.total_cost_in,
        total_cost_out: stats.total_cost_out,
    })
}

// -- Audit log -------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: i64,
    pub action: String,
    pub category: String,
    pub details: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub project_path: Option<String>,
    pub created_at: String,
}

#[tauri::command]
pub async fn get_audit_log(
    state: State<'_, AppState>,
    project_path: Option<String>,
    category: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<AuditEntry>, String> {
    let lim = limit.unwrap_or(100);

    let rows = if let Some(ref cat) = category {
        golish_db::repo::audit::list_by_category(
            &state.db_pool,
            cat,
            project_path.as_deref(),
            lim,
        )
        .await
        .map_err(|e| e.to_string())?
    } else {
        golish_db::repo::audit::list(&state.db_pool, project_path.as_deref(), lim)
            .await
            .map_err(|e| e.to_string())?
    };

    Ok(rows
        .into_iter()
        .map(|r| AuditEntry {
            id: r.id,
            action: r.action,
            category: r.category,
            details: r.details,
            entity_type: r.entity_type,
            entity_id: r.entity_id,
            project_path: r.project_path,
            created_at: r.created_at.to_rfc3339(),
        })
        .collect())
}

// -- Memory management -----------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub content: String,
    pub mem_type: String,
    pub metadata: Option<serde_json::Value>,
    pub created_at: String,
}

#[tauri::command]
pub async fn search_memories(
    state: State<'_, AppState>,
    query: String,
    limit: Option<i64>,
) -> Result<Vec<MemoryEntry>, String> {
    let lim = limit.unwrap_or(20);
    let rows = golish_db::repo::memories::search_text(&state.db_pool, &query, None, lim)
        .await
        .map_err(|e| e.to_string())?;

    Ok(rows
        .into_iter()
        .map(|r| MemoryEntry {
            id: r.id.to_string(),
            content: r.content,
            mem_type: format!("{:?}", r.mem_type),
            metadata: r.metadata,
            created_at: r.created_at.to_rfc3339(),
        })
        .collect())
}

#[tauri::command]
pub async fn list_recent_memories(
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> Result<Vec<MemoryEntry>, String> {
    let lim = limit.unwrap_or(50);
    let rows = golish_db::repo::memories::list_recent(&state.db_pool, lim)
        .await
        .map_err(|e| e.to_string())?;

    Ok(rows
        .into_iter()
        .map(|r| MemoryEntry {
            id: r.id.to_string(),
            content: r.content,
            mem_type: format!("{:?}", r.mem_type),
            metadata: r.metadata,
            created_at: r.created_at.to_rfc3339(),
        })
        .collect())
}

#[tauri::command]
pub async fn get_memory_count(
    state: State<'_, AppState>,
) -> Result<i64, String> {
    golish_db::repo::memories::count(&state.db_pool)
        .await
        .map_err(|e| e.to_string())
}
