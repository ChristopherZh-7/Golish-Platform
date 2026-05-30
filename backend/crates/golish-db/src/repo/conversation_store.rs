//! `conversations` / `workspace_preferences` project-scoped repo helpers
//! (AGENTS.md I2).
//!
//! Sinks the scoped conversation list, workspace-preferences load, and the
//! transactional stale-conversation cleanup (dynamic `NOT IN` placeholder list)
//! from `golish::tools::conversation_store`. Per-conversation upserts stay in
//! the command layer.

use crate::Result;
use sqlx::postgres::PgRow;
use sqlx::{FromRow, PgConnection, PgPool};

fn build_list_by_project_sql() -> String {
    "SELECT id, title, ai_session_id, project_path, sort_order, created_at FROM conversations WHERE ($1::text IS NULL OR project_path = $1) ORDER BY sort_order ASC, created_at ASC".to_string()
}

fn build_load_preferences_sql() -> String {
    "SELECT active_conversation_id, ai_model, approval_mode, approval_patterns FROM workspace_preferences WHERE project_path = $1".to_string()
}

/// Build the stale-delete SQL. With zero surviving ids this is an unconditional
/// project delete; otherwise it appends a `NOT IN ($2, $3, ...)` placeholder
/// list (project_path is `$1`). Mirrors the original command-layer construction
/// exactly so behaviour does not drift.
fn build_delete_stale_sql(surviving_count: usize) -> String {
    if surviving_count == 0 {
        "DELETE FROM conversations WHERE project_path = $1".to_string()
    } else {
        let placeholders: Vec<String> = (0..surviving_count)
            .map(|i| format!("${}", i + 2))
            .collect();
        format!(
            "DELETE FROM conversations WHERE project_path = $1 AND id NOT IN ({})",
            placeholders.join(", ")
        )
    }
}

/// List conversations visible to a project (`$1 IS NULL` widens to all),
/// ordered by sort order then creation. Generic over the caller's row type.
pub async fn list_by_project<T>(pool: &PgPool, project_path: Option<&str>) -> Result<Vec<T>>
where
    T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
{
    let rows = sqlx::query_as::<_, T>(&build_list_by_project_sql())
        .bind(project_path)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// Load the workspace-preferences row for a project (exact match). `None` == no
/// preferences saved yet. Generic over the caller's row type.
pub async fn load_preferences<T>(pool: &PgPool, project_path: &str) -> Result<Option<T>>
where
    T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
{
    let row = sqlx::query_as::<_, T>(&build_load_preferences_sql())
        .bind(project_path)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// Delete conversations for a project that are **not** in `surviving_ids`. When
/// `surviving_ids` is empty, deletes every conversation for the project. Takes a
/// `&mut PgConnection` so the batch-save path can run it inside its transaction
/// (`&mut *tx`). Returns rows affected.
pub async fn delete_stale_conversations(
    conn: &mut PgConnection,
    project_path: &str,
    surviving_ids: &[String],
) -> Result<u64> {
    let sql = build_delete_stale_sql(surviving_ids.len());
    let mut q = sqlx::query(&sql).bind(project_path);
    for id in surviving_ids {
        q = q.bind(id);
    }
    let res = q.execute(&mut *conn).await?;
    Ok(res.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_and_prefs_sql_match_command_layer() {
        assert_eq!(
            build_list_by_project_sql(),
            "SELECT id, title, ai_session_id, project_path, sort_order, created_at FROM conversations WHERE ($1::text IS NULL OR project_path = $1) ORDER BY sort_order ASC, created_at ASC"
        );
        assert_eq!(
            build_load_preferences_sql(),
            "SELECT active_conversation_id, ai_model, approval_mode, approval_patterns FROM workspace_preferences WHERE project_path = $1"
        );
    }

    #[test]
    fn delete_stale_sql_shapes() {
        assert_eq!(
            build_delete_stale_sql(0),
            "DELETE FROM conversations WHERE project_path = $1"
        );
        assert_eq!(
            build_delete_stale_sql(1),
            "DELETE FROM conversations WHERE project_path = $1 AND id NOT IN ($2)"
        );
        assert_eq!(
            build_delete_stale_sql(3),
            "DELETE FROM conversations WHERE project_path = $1 AND id NOT IN ($2, $3, $4)"
        );
    }
}
