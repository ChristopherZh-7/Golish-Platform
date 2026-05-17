//! Organizations Tauri commands.
//!
//! 多级组织树形结构，由 §S3 引入。`grp` 字段在 §S1 的字符串分级仍兼容
//! 保留作为回退；新建 target 可以直接关联 `organization_id`。

use crate::error::GolishError;
use crate::state::DbState;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Organization {
    pub id: String,
    pub project_path: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub description: String,
    pub owner: String,
    pub sort_order: i32,
    pub created_at: u64,
    pub updated_at: u64,
}

fn to_org(o: golish_db::models::Organization) -> Organization {
    Organization {
        id: o.id.to_string(),
        project_path: o.project_path,
        name: o.name,
        parent_id: o.parent_id.map(|u| u.to_string()),
        description: o.description,
        owner: o.owner,
        sort_order: o.sort_order,
        created_at: o.created_at.timestamp() as u64,
        updated_at: o.updated_at.timestamp() as u64,
    }
}

#[tauri::command]
pub async fn organization_list(
    state: tauri::State<'_, DbState>,
    project_path: Option<String>,
) -> Result<Vec<Organization>, GolishError> {
    let pool = state.pool_ready().await?;
    let pp = project_path.as_deref().unwrap_or("");
    let rows = golish_db::repo::organizations::list(pool, pp).await?;
    Ok(rows.into_iter().map(to_org).collect())
}

#[tauri::command]
pub async fn organization_create(
    state: tauri::State<'_, DbState>,
    project_path: Option<String>,
    name: String,
    parent_id: Option<String>,
    description: Option<String>,
    owner: Option<String>,
) -> Result<Organization, GolishError> {
    let pool = state.pool_ready().await?;
    let pp = project_path.as_deref().unwrap_or("");
    let pid: Option<Uuid> = parent_id.and_then(|s| s.parse().ok());
    let row = golish_db::repo::organizations::create(
        pool,
        pp,
        name.trim(),
        pid,
        description.as_deref().unwrap_or(""),
        owner.as_deref().unwrap_or(""),
    )
    .await?;
    Ok(to_org(row))
}

#[tauri::command]
pub async fn organization_update(
    state: tauri::State<'_, DbState>,
    id: String,
    name: Option<String>,
    description: Option<String>,
    owner: Option<String>,
    sort_order: Option<i32>,
) -> Result<Organization, GolishError> {
    let pool = state.pool_ready().await?;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let row = golish_db::repo::organizations::update(
        pool,
        uid,
        name.as_deref(),
        description.as_deref(),
        owner.as_deref(),
        sort_order,
    )
    .await?;
    Ok(to_org(row))
}

#[tauri::command]
pub async fn organization_move(
    state: tauri::State<'_, DbState>,
    id: String,
    new_parent_id: Option<String>,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let new_parent: Option<Uuid> = new_parent_id.and_then(|s| s.parse().ok());
    golish_db::repo::organizations::move_to(pool, uid, new_parent).await?;
    Ok(())
}

#[tauri::command]
pub async fn organization_delete(
    state: tauri::State<'_, DbState>,
    id: String,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    golish_db::repo::organizations::delete(pool, uid).await?;
    Ok(())
}
