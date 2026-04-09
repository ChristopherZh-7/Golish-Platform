use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{NewSession, Session, SessionStatus};

pub async fn create(pool: &PgPool, s: NewSession) -> Result<Session> {
    let row = sqlx::query_as::<_, Session>(
        r#"INSERT INTO sessions (title, workspace_path, workspace_label, model, provider, project_path)
           VALUES ($1, $2, $3, $4, $5, $6)
           RETURNING *"#,
    )
    .bind(&s.title)
    .bind(&s.workspace_path)
    .bind(&s.workspace_label)
    .bind(&s.model)
    .bind(&s.provider)
    .bind(&s.project_path)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Session>> {
    let row = sqlx::query_as::<_, Session>("SELECT * FROM sessions WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

pub async fn list(pool: &PgPool, limit: i64) -> Result<Vec<Session>> {
    let rows = sqlx::query_as::<_, Session>(
        "SELECT * FROM sessions ORDER BY created_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn update_status(pool: &PgPool, id: Uuid, status: SessionStatus) -> Result<()> {
    sqlx::query("UPDATE sessions SET status = $1, updated_at = NOW() WHERE id = $2")
        .bind(status)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM sessions WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
