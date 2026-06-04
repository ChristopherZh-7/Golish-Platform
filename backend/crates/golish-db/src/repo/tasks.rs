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

/// SQL for [`latest_resumable_by_session`]. Const so its shape is unit-testable
/// without a live DB.
///
/// "Resumable" = a task that (a) belongs to this chat session's DB session,
/// (b) is still non-terminal (`running` = zombie left by a killed/disconnected
/// process, or `waiting` = paused for input/approval), and (c) has a harness
/// checkpoint persisted (`operation_state.state_blob -> 'graph_flow'`) so the
/// engine can actually resume from `next_node`. `finished` / `failed` are
/// excluded (terminal). Newest first so a fresh disconnect wins.
const LATEST_RESUMABLE_BY_SESSION_SQL: &str = "SELECT t.* FROM tasks t \
     JOIN operation_state os ON os.operation_id = t.id \
     WHERE t.session_id = $1 \
       AND t.status IN ('running', 'waiting') \
       AND os.state_blob -> 'graph_flow' IS NOT NULL \
     ORDER BY t.created_at DESC \
     LIMIT 1";

/// Find the most recent **resumable** harness operation for a chat session's DB
/// session, or `None` if there is nothing to resume (→ caller starts fresh).
///
/// This is the state-driven signal that lets task mode decide resume-vs-new
/// without parsing the user's text (no "继续" keyword special-case): if a
/// checkpointed non-terminal task exists, the next message resumes it.
pub async fn latest_resumable_by_session(pool: &PgPool, session_id: Uuid) -> Result<Option<Task>> {
    let row = sqlx::query_as::<_, Task>(LATEST_RESUMABLE_BY_SESSION_SQL)
        .bind(session_id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
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
///
/// Carve-out (Task 断线恢复 · L4b): a task that has a harness checkpoint
/// (`operation_state.state_blob -> 'graph_flow'`) is **resumable**, so the reaper
/// must NOT fail it — [`pause_resumable_abandoned`] flips those to `waiting`
/// instead. Only truly-dead non-terminal rows (no checkpoint) get failed here.
const FAIL_ABANDONED_TASKS_SQL: &str = "UPDATE tasks \
     SET status = 'failed', \
         result = COALESCE(result, 'Abandoned: the process exited before this task finished.'), \
         updated_at = NOW() \
     WHERE status IN ('running', 'waiting') \
       AND updated_at < $1 \
       AND NOT EXISTS ( \
           SELECT 1 FROM operation_state os \
           WHERE os.operation_id = tasks.id \
             AND os.state_blob -> 'graph_flow' IS NOT NULL \
       )";

/// SQL for [`pause_resumable_abandoned`]. Const for the same unit-test reason.
///
/// A `running` task abandoned by a dead process but holding a harness checkpoint
/// is not dead — it is *paused & resumable*. Demote it to `waiting` (the paused
/// state) so it stops zombieing as `running` yet remains eligible for
/// [`latest_resumable_by_session`]. `waiting` rows are left as-is (already
/// paused). Time-bounded like the fail reaper so a live run is never touched.
const PAUSE_RESUMABLE_ABANDONED_TASKS_SQL: &str = "UPDATE tasks \
     SET status = 'waiting', updated_at = NOW() \
     WHERE status = 'running' \
       AND updated_at < $1 \
       AND EXISTS ( \
           SELECT 1 FROM operation_state os \
           WHERE os.operation_id = tasks.id \
             AND os.state_blob -> 'graph_flow' IS NOT NULL \
       )";

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

/// Pause (→ `waiting`) abandoned `running` tasks that still hold a harness
/// checkpoint, instead of failing them, so a killed/disconnected operation stays
/// resumable on the next user message. Counterpart to [`fail_abandoned`] (which
/// now skips checkpointed rows). Fire-and-forget like the rest of the startup
/// reaper; returns the number of rows demoted.
pub async fn pause_resumable_abandoned(pool: &PgPool, threshold: Duration) -> Result<u64> {
    let cutoff = crate::repo::audit::reclaim_cutoff(threshold);
    let result = sqlx::query(PAUSE_RESUMABLE_ABANDONED_TASKS_SQL)
        .bind(cutoff)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::{
        FAIL_ABANDONED_TASKS_SQL, LATEST_RESUMABLE_BY_SESSION_SQL,
        PAUSE_RESUMABLE_ABANDONED_TASKS_SQL,
    };

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

    /// L4b carve-out: the fail reaper must NOT fail tasks that hold a harness
    /// checkpoint (those are resumable) — they are demoted to `waiting` by
    /// [`super::pause_resumable_abandoned`] instead.
    #[test]
    fn fail_abandoned_sql_skips_checkpointed_resumable_tasks() {
        let sql = FAIL_ABANDONED_TASKS_SQL;
        assert!(
            sql.contains("NOT EXISTS"),
            "must carve out resumable rows: {sql}"
        );
        assert!(
            sql.contains("'graph_flow'"),
            "carve-out must key on the harness checkpoint: {sql}"
        );
    }

    /// The pause reaper must only demote abandoned `running` rows WITH a
    /// checkpoint to `waiting`, time-bounded, and never touch `failed`/`finished`.
    #[test]
    fn pause_resumable_sql_only_pauses_checkpointed_running_rows() {
        let sql = PAUSE_RESUMABLE_ABANDONED_TASKS_SQL;
        assert!(sql.contains("UPDATE tasks"), "sql={sql}");
        assert!(
            sql.contains("status = 'waiting'"),
            "must pause, not fail: {sql}"
        );
        assert!(
            sql.contains("status = 'running'"),
            "must only demote running zombies: {sql}"
        );
        assert!(
            sql.contains("updated_at < $1"),
            "must be time-bounded: {sql}"
        );
        assert!(
            sql.contains("EXISTS") && sql.contains("'graph_flow'"),
            "must require a checkpoint to be resumable: {sql}"
        );
        assert!(!sql.contains("'failed'"), "pause must not fail: {sql}");
    }

    /// The resumable-lookup must exclude terminal rows, require a checkpoint, and
    /// pick the newest task so a fresh disconnect wins.
    #[test]
    fn latest_resumable_sql_is_nonterminal_checkpointed_newest_first() {
        let sql = LATEST_RESUMABLE_BY_SESSION_SQL;
        assert!(sql.contains("FROM tasks t"), "sql={sql}");
        assert!(
            sql.contains("status IN ('running', 'waiting')"),
            "must exclude terminal rows: {sql}"
        );
        assert!(
            sql.contains("'graph_flow'"),
            "must require a harness checkpoint: {sql}"
        );
        assert!(
            sql.contains("ORDER BY t.created_at DESC") && sql.contains("LIMIT 1"),
            "must pick the newest resumable task: {sql}"
        );
    }
}
