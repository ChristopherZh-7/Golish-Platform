use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{ExecutionPlan, NewExecutionPlan, PlanStatus};

pub async fn create(pool: &PgPool, plan: NewExecutionPlan) -> Result<ExecutionPlan> {
    let row = sqlx::query_as::<_, ExecutionPlan>(
        r#"INSERT INTO execution_plans (session_id, project_path, title, description, steps)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING *"#,
    )
    .bind(plan.session_id)
    .bind(&plan.project_path)
    .bind(&plan.title)
    .bind(&plan.description)
    .bind(&plan.steps)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<ExecutionPlan>> {
    let row = sqlx::query_as::<_, ExecutionPlan>("SELECT * FROM execution_plans WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

pub async fn list_by_project(
    pool: &PgPool,
    project_path: &str,
    include_completed: bool,
) -> Result<Vec<ExecutionPlan>> {
    let query = if include_completed {
        "SELECT * FROM execution_plans WHERE project_path = $1 ORDER BY updated_at DESC"
    } else {
        "SELECT * FROM execution_plans WHERE project_path = $1 AND status NOT IN ('completed', 'cancelled', 'failed') ORDER BY updated_at DESC"
    };
    let rows = sqlx::query_as::<_, ExecutionPlan>(query)
        .bind(project_path)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

pub async fn list_active(pool: &PgPool, project_path: &str) -> Result<Vec<ExecutionPlan>> {
    let rows = sqlx::query_as::<_, ExecutionPlan>(
        "SELECT * FROM execution_plans WHERE project_path = $1 AND status IN ('planning', 'in_progress', 'paused') ORDER BY updated_at DESC",
    )
    .bind(project_path)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn update_status(pool: &PgPool, id: Uuid, status: PlanStatus) -> Result<()> {
    sqlx::query(
        "UPDATE execution_plans SET status = $1, updated_at = NOW() WHERE id = $2",
    )
    .bind(status)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_steps(
    pool: &PgPool,
    id: Uuid,
    steps: &serde_json::Value,
    current_step: i32,
    status: PlanStatus,
) -> Result<()> {
    sqlx::query(
        "UPDATE execution_plans SET steps = $1, current_step = $2, status = $3, updated_at = NOW() WHERE id = $4",
    )
    .bind(steps)
    .bind(current_step)
    .bind(status)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_context(
    pool: &PgPool,
    id: Uuid,
    context: &serde_json::Value,
) -> Result<()> {
    sqlx::query(
        "UPDATE execution_plans SET context = $1, updated_at = NOW() WHERE id = $2",
    )
    .bind(context)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM execution_plans WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
