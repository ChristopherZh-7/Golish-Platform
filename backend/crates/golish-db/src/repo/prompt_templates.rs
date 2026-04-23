use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::PromptTemplate;

pub async fn get_active(pool: &PgPool, template_name: &str) -> Result<Option<PromptTemplate>> {
    let row = sqlx::query_as::<_, PromptTemplate>(
        "SELECT * FROM prompt_templates WHERE template_name = $1 AND is_active = true LIMIT 1",
    )
    .bind(template_name)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn get_all_active(pool: &PgPool) -> Result<Vec<PromptTemplate>> {
    let rows = sqlx::query_as::<_, PromptTemplate>(
        "SELECT * FROM prompt_templates WHERE is_active = true ORDER BY template_name",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn upsert(
    pool: &PgPool,
    template_name: &str,
    content: &str,
    description: &str,
    project_path: Option<&str>,
) -> Result<PromptTemplate> {
    let row = sqlx::query_as::<_, PromptTemplate>(
        r#"INSERT INTO prompt_templates (template_name, content, description, project_path)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (template_name)
           DO UPDATE SET content = $2, description = $3, project_path = $4, updated_at = NOW()
           RETURNING *"#,
    )
    .bind(template_name)
    .bind(content)
    .bind(description)
    .bind(project_path)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn deactivate(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("UPDATE prompt_templates SET is_active = false, updated_at = NOW() WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn list_all(pool: &PgPool) -> Result<Vec<PromptTemplate>> {
    let rows = sqlx::query_as::<_, PromptTemplate>(
        "SELECT * FROM prompt_templates ORDER BY template_name",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
