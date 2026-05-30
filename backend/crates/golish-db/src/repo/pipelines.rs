use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::Pipeline;

pub async fn upsert(
    pool: &PgPool,
    id: Uuid,
    data: &serde_json::Value,
    project_path: Option<&str>,
) -> Result<()> {
    super::scoped::upsert_json_data(pool, "pipelines", id, data, project_path).await
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Pipeline>> {
    super::scoped::get_by_id(pool, "pipelines", id).await
}

pub async fn list(pool: &PgPool, project_path: Option<&str>) -> Result<Vec<Pipeline>> {
    super::scoped::list_by_project(pool, "pipelines", "updated_at DESC", project_path).await
}

pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
    super::scoped::delete_by_id(pool, "pipelines", id).await?;
    Ok(())
}

// ── Project-scoped helpers (AGENTS.md I2). Command layer must route through
// these instead of writing scoped SQL inline. ─────────────────────────────

/// List raw `data` JSON for a project (exact `project_path = $1` match).
pub async fn list_data_by_project(
    pool: &PgPool,
    project_path: Option<&str>,
) -> Result<Vec<serde_json::Value>> {
    super::scoped::list_json_data_by_project(pool, "pipelines", "updated_at DESC", project_path)
        .await
}

/// Return the id if this pipeline exists but is owned by a *different* project.
/// Used as the upsert cross-project guard so `ON CONFLICT` can't overwrite
/// another project's pipeline.
pub async fn find_cross_project(
    pool: &PgPool,
    id: Uuid,
    project_path: Option<&str>,
) -> Result<Option<Uuid>> {
    let row = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM pipelines WHERE id = $1 AND project_path IS DISTINCT FROM $2",
    )
    .bind(id)
    .bind(project_path)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Load a pipeline's `data`, scoped to project_path (IDOR).
pub async fn load_data_scoped(
    pool: &PgPool,
    id: Uuid,
    project_path: Option<&str>,
) -> Result<Option<serde_json::Value>> {
    super::scoped::get_json_data_scoped(pool, "pipelines", id, project_path).await
}

/// Delete a pipeline scoped to project_path (IDOR). Returns rows affected.
pub async fn delete_scoped(pool: &PgPool, id: Uuid, project_path: Option<&str>) -> Result<u64> {
    super::scoped::delete_scoped(pool, "pipelines", id, project_path).await
}
