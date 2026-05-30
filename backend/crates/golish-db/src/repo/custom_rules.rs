//! `custom_passive_rules` project-scoped repo helpers (AGENTS.md I2).
//!
//! The command layer (`golish::tools::custom_rules`) routes its scoped `list`
//! and `clear-all` operations through here. Upserts and delete-by-id stay in
//! the command layer.

use crate::Result;
use sqlx::postgres::PgRow;
use sqlx::{FromRow, PgPool};

fn build_list_by_project_sql() -> String {
    "SELECT id, name, pattern, scope, severity, enabled FROM custom_passive_rules WHERE project_path = $1 ORDER BY created_at ASC".to_string()
}

fn build_clear_by_project_sql() -> String {
    "DELETE FROM custom_passive_rules WHERE project_path = $1".to_string()
}

/// List custom passive rules for a project, oldest first. Generic over the
/// caller's row type (the command layer's 6-tuple).
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

/// Delete every custom passive rule for a project. Returns rows affected.
pub async fn clear_by_project(pool: &PgPool, project_path: Option<&str>) -> Result<u64> {
    let res = sqlx::query(&build_clear_by_project_sql())
        .bind(project_path)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_rules_sql_matches_command_layer() {
        assert_eq!(
            build_list_by_project_sql(),
            "SELECT id, name, pattern, scope, severity, enabled FROM custom_passive_rules WHERE project_path = $1 ORDER BY created_at ASC"
        );
        assert_eq!(
            build_clear_by_project_sql(),
            "DELETE FROM custom_passive_rules WHERE project_path = $1"
        );
    }
}
