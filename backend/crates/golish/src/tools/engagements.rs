//! Engagement (project metadata) Tauri commands.
//!
//! 一个 project_path 对应一个 engagement（HVV 项目 / 红队任务的元信息），
//! 含 hvv_name / team_members / time_window 等。
//! 表是 §S2 引入的，PK 为 project_path，无 FK。

use crate::error::GolishError;
use crate::state::DbState;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Engagement {
    pub project_path: String,
    pub hvv_name: String,
    pub team_members: Vec<String>,
    pub start_at: Option<u64>,
    pub end_at: Option<u64>,
    pub notes: String,
    pub created_at: u64,
    pub updated_at: u64,
}

fn to_engagement(e: golish_db::models::Engagement) -> Engagement {
    Engagement {
        project_path: e.project_path,
        hvv_name: e.hvv_name,
        team_members: serde_json::from_value(e.team_members).unwrap_or_default(),
        start_at: e.start_at.map(|dt| dt.timestamp() as u64),
        end_at: e.end_at.map(|dt| dt.timestamp() as u64),
        notes: e.notes,
        created_at: e.created_at.timestamp() as u64,
        updated_at: e.updated_at.timestamp() as u64,
    }
}

fn parse_iso8601(s: Option<&str>) -> Option<chrono::DateTime<chrono::Utc>> {
    let trimmed = s?.trim();
    if trimmed.is_empty() {
        return None;
    }
    chrono::DateTime::parse_from_rfc3339(trimmed)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

#[tauri::command]
pub async fn engagement_get(
    state: tauri::State<'_, DbState>,
    project_path: String,
) -> Result<Option<Engagement>, GolishError> {
    let pool = state.pool_ready().await?;
    let row = golish_db::repo::engagements::get(pool, &project_path).await?;
    Ok(row.map(to_engagement))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn engagement_save(
    state: tauri::State<'_, DbState>,
    project_path: String,
    hvv_name: Option<String>,
    team_members: Option<Vec<String>>,
    start_at: Option<String>,
    end_at: Option<String>,
    notes: Option<String>,
) -> Result<Engagement, GolishError> {
    let pool = state.pool_ready().await?;
    let team_json = serde_json::to_value(team_members.unwrap_or_default()).unwrap_or_default();
    let row = golish_db::repo::engagements::upsert(
        pool,
        &project_path,
        hvv_name.as_deref().unwrap_or(""),
        &team_json,
        parse_iso8601(start_at.as_deref()),
        parse_iso8601(end_at.as_deref()),
        notes.as_deref().unwrap_or(""),
    )
    .await?;
    Ok(to_engagement(row))
}

#[tauri::command]
pub async fn engagement_delete(
    state: tauri::State<'_, DbState>,
    project_path: String,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    golish_db::repo::engagements::delete(pool, &project_path).await?;
    Ok(())
}
