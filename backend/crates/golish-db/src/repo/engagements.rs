use anyhow::Result;
use sqlx::PgPool;

use crate::models::Engagement;

/// Fetch the engagement metadata for a project. Returns None when there is
/// no row yet (most projects start without HVV / time-window context).
pub async fn get(pool: &PgPool, project_path: &str) -> Result<Option<Engagement>> {
    let row = sqlx::query_as::<_, Engagement>(
        "SELECT * FROM engagements WHERE project_path = $1",
    )
    .bind(project_path)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Insert-or-update the engagement metadata. The natural PK is
/// `project_path`, so we ON CONFLICT update the mutable fields and bump
/// `updated_at`.
pub async fn upsert(
    pool: &PgPool,
    project_path: &str,
    hvv_name: &str,
    team_members: &serde_json::Value,
    start_at: Option<chrono::DateTime<chrono::Utc>>,
    end_at: Option<chrono::DateTime<chrono::Utc>>,
    notes: &str,
) -> Result<Engagement> {
    let row = sqlx::query_as::<_, Engagement>(
        r#"INSERT INTO engagements
              (project_path, hvv_name, team_members, start_at, end_at, notes)
           VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (project_path) DO UPDATE SET
              hvv_name     = EXCLUDED.hvv_name,
              team_members = EXCLUDED.team_members,
              start_at     = EXCLUDED.start_at,
              end_at       = EXCLUDED.end_at,
              notes        = EXCLUDED.notes,
              updated_at   = NOW()
           RETURNING *"#,
    )
    .bind(project_path)
    .bind(hvv_name)
    .bind(team_members)
    .bind(start_at)
    .bind(end_at)
    .bind(notes)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn delete(pool: &PgPool, project_path: &str) -> Result<()> {
    sqlx::query("DELETE FROM engagements WHERE project_path = $1")
        .bind(project_path)
        .execute(pool)
        .await?;
    Ok(())
}
