use anyhow::Result;
use sqlx::PgPool;

use crate::models::KbResearchLog;

pub async fn upsert_log(
    pool: &PgPool,
    cve_id: &str,
    session_id: &str,
    turns: &serde_json::Value,
    status: &str,
) -> Result<KbResearchLog> {
    let row = sqlx::query_as::<_, KbResearchLog>(
        r#"INSERT INTO kb_research_log (cve_id, session_id, turns, status)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (cve_id) DO UPDATE SET
               session_id = $2, turns = $3, status = $4, updated_at = NOW()
           RETURNING *"#,
    )
    .bind(cve_id)
    .bind(session_id)
    .bind(turns)
    .bind(status)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn append_turn(
    pool: &PgPool,
    cve_id: &str,
    turn: &serde_json::Value,
) -> Result<KbResearchLog> {
    let row = sqlx::query_as::<_, KbResearchLog>(
        r#"UPDATE kb_research_log
           SET turns = turns || $2::jsonb, updated_at = NOW()
           WHERE cve_id = $1
           RETURNING *"#,
    )
    .bind(cve_id)
    .bind(serde_json::json!([turn]))
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn get_log(pool: &PgPool, cve_id: &str) -> Result<Option<KbResearchLog>> {
    let row = sqlx::query_as::<_, KbResearchLog>(
        "SELECT * FROM kb_research_log WHERE cve_id = $1",
    )
    .bind(cve_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn set_status(pool: &PgPool, cve_id: &str, status: &str) -> Result<()> {
    sqlx::query("UPDATE kb_research_log SET status = $2, updated_at = NOW() WHERE cve_id = $1")
        .bind(cve_id)
        .bind(status)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_log(pool: &PgPool, cve_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM kb_research_log WHERE cve_id = $1")
        .bind(cve_id)
        .execute(pool)
        .await?;
    Ok(())
}
