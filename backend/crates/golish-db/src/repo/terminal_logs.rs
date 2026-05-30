use crate::Result;
use sqlx::postgres::PgRow;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::models::{StreamType, TerminalLog};

pub async fn create(
    pool: &PgPool,
    session_id: Uuid,
    task_id: Option<Uuid>,
    subtask_id: Option<Uuid>,
    stream: StreamType,
    content: &str,
) -> Result<TerminalLog> {
    let row = sqlx::query_as::<_, TerminalLog>(
        r#"INSERT INTO terminal_logs (session_id, task_id, subtask_id, stream, content)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING *"#,
    )
    .bind(session_id)
    .bind(task_id)
    .bind(subtask_id)
    .bind(stream)
    .bind(content)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list_by_session(pool: &PgPool, session_id: Uuid) -> Result<Vec<TerminalLog>> {
    let rows = sqlx::query_as::<_, TerminalLog>(
        "SELECT * FROM terminal_logs WHERE session_id = $1 ORDER BY created_at ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

const TERMINAL_LOG_LIST_COLS: &str =
    "id, session_id, task_id, subtask_id, stream::text, content, created_at";

fn build_list_by_project_sql() -> String {
    format!(
        "SELECT {TERMINAL_LOG_LIST_COLS} FROM terminal_logs WHERE project_path = $1 ORDER BY created_at DESC LIMIT $2"
    )
}

/// Project-wide terminal-log list (subset projection with `stream` cast to
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
            "SELECT id, session_id, task_id, subtask_id, stream::text, content, created_at FROM terminal_logs WHERE project_path = $1 ORDER BY created_at DESC LIMIT $2"
        );
    }
}
