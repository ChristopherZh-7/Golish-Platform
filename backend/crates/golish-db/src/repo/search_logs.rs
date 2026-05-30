use anyhow::Result;
use sqlx::postgres::PgRow;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::models::{AgentType, SearchLog};

pub async fn create(
    pool: &PgPool,
    session_id: Uuid,
    task_id: Option<Uuid>,
    subtask_id: Option<Uuid>,
    initiator: Option<AgentType>,
    engine: &str,
    query: &str,
    result: Option<&str>,
) -> Result<SearchLog> {
    let row = sqlx::query_as::<_, SearchLog>(
        r#"INSERT INTO search_logs (session_id, task_id, subtask_id, initiator, engine, query, result)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING *"#,
    )
    .bind(session_id)
    .bind(task_id)
    .bind(subtask_id)
    .bind(initiator)
    .bind(engine)
    .bind(query)
    .bind(result)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list_by_session(pool: &PgPool, session_id: Uuid) -> Result<Vec<SearchLog>> {
    let rows = sqlx::query_as::<_, SearchLog>(
        "SELECT * FROM search_logs WHERE session_id = $1 ORDER BY created_at ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

const SEARCH_LOG_LIST_COLS: &str =
    "id, session_id, task_id, subtask_id, initiator::text, engine, query, result, created_at";

fn build_list_by_project_sql() -> String {
    format!(
        "SELECT {SEARCH_LOG_LIST_COLS} FROM search_logs WHERE project_path = $1 ORDER BY created_at DESC LIMIT $2"
    )
}

/// Project-wide search-log list (subset projection with `initiator` cast to
/// text), newest first. Generic over the caller's row type.
pub async fn list_by_project<T>(pool: &PgPool, project_path: &str, limit: i64) -> Result<Vec<T>>
where
    T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
{
    let rows = sqlx::query_as::<_, T>(&build_list_by_project_sql())
        .bind(project_path)
        .bind(limit)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_by_project_sql_matches_command_layer() {
        assert_eq!(
            build_list_by_project_sql(),
            "SELECT id, session_id, task_id, subtask_id, initiator::text, engine, query, result, created_at FROM search_logs WHERE project_path = $1 ORDER BY created_at DESC LIMIT $2"
        );
    }
}
