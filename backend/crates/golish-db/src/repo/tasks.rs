use chrono::Duration;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{NewTask, Task, TaskStatus};
use crate::Result;

pub async fn create(pool: &PgPool, t: NewTask) -> Result<Task> {
    let row = sqlx::query_as::<_, Task>(
        r#"INSERT INTO tasks (session_id, title, input)
           VALUES ($1, $2, $3)
           RETURNING *"#,
    )
    .bind(t.session_id)
    .bind(&t.title)
    .bind(&t.input)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Task>> {
    super::scoped::get_by_id(pool, "tasks", id).await
}

pub async fn list_by_session(pool: &PgPool, session_id: Uuid) -> Result<Vec<Task>> {
    let rows = sqlx::query_as::<_, Task>(
        "SELECT * FROM tasks WHERE session_id = $1 ORDER BY created_at ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn update_status(pool: &PgPool, id: Uuid, status: TaskStatus) -> Result<()> {
    sqlx::query("UPDATE tasks SET status = $1, updated_at = NOW() WHERE id = $2")
        .bind(status)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_result(pool: &PgPool, id: Uuid, result: &str, status: TaskStatus) -> Result<()> {
    sqlx::query("UPDATE tasks SET result = $1, status = $2, updated_at = NOW() WHERE id = $3")
        .bind(result)
        .bind(status)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// SQL for [`fail_abandoned`]. Kept as a const so it can be asserted by a unit
/// test without a live DB (mirrors `repo::audit`'s SQL-string tests).
const FAIL_ABANDONED_TASKS_SQL: &str = "UPDATE tasks \
     SET status = 'failed', \
         result = COALESCE(result, 'Abandoned: the process exited before this task finished.'), \
         updated_at = NOW() \
     WHERE status IN ('running', 'waiting') \
       AND updated_at < $1";

/// Reap "abandoned" tasks: rows left in a non-terminal state (`running` /
/// `waiting`) by a process that died mid-run (OOM / crash / kill) with no
/// orchestrator alive to finalize them. Mark them `failed` so they stop leaking
/// as eternal "running" (a status leak that also fed the orphan-PG problem).
///
/// Mirrors [`crate::reclaim_abandoned_audits`]: only rows older than `threshold`
/// (anchored on `updated_at`, which `update_status` / `set_result` bump) are
/// reaped, so a task started moments ago is never clobbered. Returns the number
/// of rows updated; `GolishDb::start` logs + continues on error (fire-and-forget,
/// must not abort startup).
pub async fn fail_abandoned(pool: &PgPool, threshold: Duration) -> Result<u64> {
    let cutoff = crate::repo::audit::reclaim_cutoff(threshold);
    let result = sqlx::query(FAIL_ABANDONED_TASKS_SQL)
        .bind(cutoff)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::FAIL_ABANDONED_TASKS_SQL;

    /// Guard the reaper SQL: it must only touch non-terminal rows and finalize
    /// them as `failed`, never clobber `finished`, and stay time-bounded.
    #[test]
    fn fail_abandoned_sql_targets_only_nonterminal_rows() {
        let sql = FAIL_ABANDONED_TASKS_SQL;
        assert!(sql.contains("UPDATE tasks"), "sql={sql}");
        assert!(sql.contains("status = 'failed'"), "sql={sql}");
        assert!(
            sql.contains("status IN ('running', 'waiting')"),
            "must only reap non-terminal rows: {sql}"
        );
        assert!(
            sql.contains("updated_at < $1"),
            "must be time-bounded: {sql}"
        );
        // Preserve an existing result and never resurrect a terminal row.
        assert!(sql.contains("COALESCE(result"), "sql={sql}");
        assert!(
            !sql.contains("'finished'"),
            "must not touch finished: {sql}"
        );
    }
}
