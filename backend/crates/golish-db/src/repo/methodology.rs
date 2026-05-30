use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::MethodologyProject;

pub async fn upsert(
    pool: &PgPool,
    id: Uuid,
    data: &serde_json::Value,
    project_path: Option<&str>,
) -> Result<()> {
    super::scoped::upsert_json_data(pool, "methodology_projects", id, data, project_path).await
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<MethodologyProject>> {
    super::scoped::get_by_id(pool, "methodology_projects", id).await
}

pub async fn list(pool: &PgPool, project_path: Option<&str>) -> Result<Vec<MethodologyProject>> {
    super::scoped::list_by_project(
        pool,
        "methodology_projects",
        "updated_at DESC",
        project_path,
    )
    .await
}

pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
    super::scoped::delete_by_id(pool, "methodology_projects", id).await?;
    Ok(())
}

// ── Project-scoped helpers (AGENTS.md I2). Command layer must route through
// these instead of writing scoped SQL inline. ─────────────────────────────

/// Insert a new methodology project row.
pub async fn insert_data(
    pool: &PgPool,
    id: Uuid,
    data: &serde_json::Value,
    project_path: Option<&str>,
) -> Result<()> {
    sqlx::query("INSERT INTO methodology_projects (id, data, project_path) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(data)
        .bind(project_path)
        .execute(pool)
        .await?;
    Ok(())
}

/// List raw `data` JSON for a project (exact `project_path = $1` match).
pub async fn list_data_by_project(
    pool: &PgPool,
    project_path: Option<&str>,
) -> Result<Vec<serde_json::Value>> {
    super::scoped::list_json_data_by_project(
        pool,
        "methodology_projects",
        "updated_at DESC",
        project_path,
    )
    .await
}

/// Load a methodology project's `data`, scoped to project_path (IDOR).
/// `None` == row missing or owned by another project.
pub async fn get_data_scoped(
    pool: &PgPool,
    id: Uuid,
    project_path: Option<&str>,
) -> Result<Option<serde_json::Value>> {
    super::scoped::get_json_data_scoped(pool, "methodology_projects", id, project_path).await
}

/// Overwrite a methodology project's `data` (by id only; caller must scope-guard first).
pub async fn update_data(pool: &PgPool, id: Uuid, data: &serde_json::Value) -> Result<()> {
    sqlx::query("UPDATE methodology_projects SET data=$1, updated_at=NOW() WHERE id=$2")
        .bind(data)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Delete a methodology project scoped to project_path (IDOR). Returns rows affected.
pub async fn delete_scoped(pool: &PgPool, id: Uuid, project_path: Option<&str>) -> Result<u64> {
    super::scoped::delete_scoped(pool, "methodology_projects", id, project_path).await
}
