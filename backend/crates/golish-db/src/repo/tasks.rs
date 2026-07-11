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

/// Stable result marker written by [`fail_abandoned`]. The exact-resume CLI
/// uses this as one of its compare-and-set witnesses, so changing the text is a
/// data-contract change rather than a cosmetic edit.
pub const ABANDONED_TASK_RESULT: &str = "Abandoned: the process exited before this task finished.";

/// Shared, fail-closed definition of a checkpoint that the startup reaper may
/// preserve. Both reaper updates embed this exact predicate so no row can be
/// selected by the pause branch and then failed by a divergent definition.
///
/// Normal resume still uses `graph_flow`. The second branch is deliberately
/// narrower: it recognizes only the complete flat *first-stage* checkpoint
/// that the explicit exact-resume CLI can repair. Partial/malformed JSON, a
/// mismatched operation identity, a nil/non-UUID run id, an empty worker map,
/// or a superseded operation all fail closed and remain eligible for cleanup.
const RECOVERABLE_ABANDONED_CHECKPOINT_SQL: &str = r#"os.superseded_by IS NULL
             AND jsonb_typeof(os.state_blob) = 'object'
             AND (
                 (
                     jsonb_typeof(os.state_blob -> 'graph_flow') = 'object'
                     AND jsonb_typeof(os.state_blob -> 'graph_flow' -> 'state') = 'object'
                     AND jsonb_typeof(os.state_blob -> 'graph_flow' -> 'state' -> 'seeded') = 'object'
                     AND jsonb_typeof(os.state_blob -> 'graph_flow' -> 'state' -> 'visited') = 'array'
                     AND jsonb_typeof(os.state_blob -> 'graph_flow' -> 'state' -> 'applied') = 'object'
                     AND jsonb_typeof(os.state_blob -> 'graph_flow' -> 'next_node') = 'string'
                     AND NULLIF(BTRIM(os.state_blob -> 'graph_flow' ->> 'next_node'), '') IS NOT NULL
                 )
                 OR
                 (
                     os.state_blob -> 'graph_flow' IS NULL
                     AND jsonb_typeof(os.state_blob -> 'profile') = 'string'
                     AND os.state_blob ->> 'profile' = os.profile
                     AND NULLIF(BTRIM(os.profile), '') IS NOT NULL
                     AND jsonb_typeof(os.state_blob -> 'current_stage') = 'string'
                     AND os.state_blob ->> 'current_stage' = os.current_stage
                     AND NULLIF(BTRIM(os.current_stage), '') IS NOT NULL
                     AND jsonb_typeof(os.state_blob -> 'current_stage_run_id') = 'string'
                     AND NULLIF(BTRIM(os.state_blob ->> 'current_stage_run_id'), '') IS NOT NULL
                     AND os.state_blob ->> 'current_stage_run_id' <> '00000000-0000-0000-0000-000000000000'
                     AND os.state_blob ->> 'current_stage_run_id' ~* '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
                     AND jsonb_typeof(os.state_blob -> 'queue_titles') = 'array'
                     AND jsonb_typeof(os.state_blob -> 'completed_count') = 'number'
                     AND os.state_blob ->> 'completed_count' = '0'
                     AND CASE
                         WHEN jsonb_typeof(os.state_blob #> ARRAY['stage_run_workers', os.current_stage]) = 'object'
                         THEN
                             EXISTS (
                                 SELECT 1
                                 FROM jsonb_each(
                                     os.state_blob #> ARRAY['stage_run_workers', os.current_stage]
                                 ) AS current_stage_worker(org_id, worker)
                             )
                             AND NOT EXISTS (
                                 SELECT 1
                                 FROM jsonb_each(
                                     os.state_blob #> ARRAY['stage_run_workers', os.current_stage]
                                 ) AS current_stage_worker(org_id, worker)
                                 WHERE current_stage_worker.org_id !~* '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
                                    OR current_stage_worker.org_id = '00000000-0000-0000-0000-000000000000'
                                    OR jsonb_typeof(current_stage_worker.worker) <> 'object'
                                    OR jsonb_typeof(current_stage_worker.worker -> 'chain_id') <> 'string'
                                    OR current_stage_worker.worker ->> 'chain_id' !~* '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
                                    OR current_stage_worker.worker ->> 'chain_id' = '00000000-0000-0000-0000-000000000000'
                                    OR jsonb_typeof(current_stage_worker.worker -> 'specialist') <> 'string'
                                    OR NULLIF(BTRIM(current_stage_worker.worker ->> 'specialist'), '') IS NULL
                                    OR NOT EXISTS (
                                        SELECT 1
                                        FROM message_chains mc
                                        WHERE mc.id::text = LOWER(current_stage_worker.worker ->> 'chain_id')
                                          AND mc.session_id = tasks.session_id
                                          AND (mc.task_id IS NULL OR mc.task_id = tasks.id)
                                          AND mc.agent::text = current_stage_worker.worker ->> 'specialist'
                                          AND mc.chain IS NOT NULL
                                    )
                             )
                             AND (
                                 SELECT COUNT(*) = COUNT(DISTINCT LOWER(current_stage_worker.worker ->> 'chain_id'))
                                 FROM jsonb_each(
                                     os.state_blob #> ARRAY['stage_run_workers', os.current_stage]
                                 ) AS current_stage_worker(org_id, worker)
                             )
                         ELSE FALSE
                     END
                 )
             )"#;

/// SQL for [`fail_abandoned`]. Built from the shared recoverability predicate
/// so its complement is exactly the set handled by
/// [`pause_resumable_abandoned`].
fn fail_abandoned_tasks_sql() -> String {
    format!(
        "UPDATE tasks \
         SET status = 'failed', \
             result = COALESCE(result, '{ABANDONED_TASK_RESULT}'), \
             updated_at = NOW() \
         WHERE status IN ('running', 'waiting') \
           AND updated_at < $1 \
           AND NOT EXISTS ( \
               SELECT 1 FROM operation_state os \
               WHERE os.operation_id = tasks.id \
                 AND {RECOVERABLE_ABANDONED_CHECKPOINT_SQL} \
           )"
    )
}

/// SQL for [`pause_resumable_abandoned`], built from the same shared predicate.
///
/// A `running` task abandoned by a dead process but holding a harness checkpoint
/// is not dead — it is *paused & resumable*. Demote it to `waiting` (the paused
/// state) so it stops zombieing as `running` yet remains eligible for
/// [`latest_resumable_by_session`]. `waiting` rows are left as-is (already
/// paused). Time-bounded like the fail reaper so a live run is never touched.
fn pause_resumable_abandoned_tasks_sql() -> String {
    format!(
        "UPDATE tasks \
         SET status = 'waiting', updated_at = NOW() \
         WHERE status = 'running' \
           AND updated_at < $1 \
           AND EXISTS ( \
               SELECT 1 FROM operation_state os \
               WHERE os.operation_id = tasks.id \
                 AND {RECOVERABLE_ABANDONED_CHECKPOINT_SQL} \
           )"
    )
}

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
    let sql = fail_abandoned_tasks_sql();
    let result = sqlx::query(&sql).bind(cutoff).execute(pool).await?;
    Ok(result.rows_affected())
}

/// Pause (→ `waiting`) abandoned `running` tasks that still hold a harness
/// checkpoint, instead of failing them, so a killed/disconnected operation stays
/// resumable on the next user message. Counterpart to [`fail_abandoned`] (which
/// now skips checkpointed rows). Fire-and-forget like the rest of the startup
/// reaper; returns the number of rows demoted.
pub async fn pause_resumable_abandoned(pool: &PgPool, threshold: Duration) -> Result<u64> {
    let cutoff = crate::repo::audit::reclaim_cutoff(threshold);
    let sql = pause_resumable_abandoned_tasks_sql();
    let result = sqlx::query(&sql).bind(cutoff).execute(pool).await?;
    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::{
        fail_abandoned_tasks_sql, pause_resumable_abandoned_tasks_sql, ABANDONED_TASK_RESULT,
        LATEST_RESUMABLE_BY_SESSION_SQL, RECOVERABLE_ABANDONED_CHECKPOINT_SQL,
    };

    /// Guard the reaper SQL: it must only touch non-terminal rows and finalize
    /// them as `failed`, never clobber `finished`, and stay time-bounded.
    #[test]
    fn fail_abandoned_sql_targets_only_nonterminal_rows() {
        let sql = fail_abandoned_tasks_sql();
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
        let sql = fail_abandoned_tasks_sql();
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
        let sql = pause_resumable_abandoned_tasks_sql();
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

    /// Both startup reaper branches must share one fail-closed checkpoint
    /// definition. Otherwise a row can be paused by one predicate and failed by
    /// a subtly different one in the immediately following update.
    #[test]
    fn abandoned_reapers_share_the_same_recoverable_checkpoint_predicate() {
        let predicate = RECOVERABLE_ABANDONED_CHECKPOINT_SQL.trim();
        let fail_sql = fail_abandoned_tasks_sql();
        let pause_sql = pause_resumable_abandoned_tasks_sql();

        assert_eq!(fail_sql.matches(predicate).count(), 1, "sql={fail_sql}");
        assert_eq!(pause_sql.matches(predicate).count(), 1, "sql={pause_sql}");
    }

    /// A flat first-stage checkpoint is recoverable only when every identity
    /// witness needed by the explicit CLI repair path is present. Merely having
    /// one or two JSON keys must never exempt a zombie from the fail reaper.
    #[test]
    fn recoverable_checkpoint_predicate_requires_complete_flat_identity() {
        let sql = RECOVERABLE_ABANDONED_CHECKPOINT_SQL;

        for required in [
            "jsonb_typeof(os.state_blob) = 'object'",
            "jsonb_typeof(os.state_blob -> 'profile') = 'string'",
            "os.state_blob ->> 'profile' = os.profile",
            "jsonb_typeof(os.state_blob -> 'current_stage') = 'string'",
            "os.state_blob ->> 'current_stage' = os.current_stage",
            "current_stage_run_id",
            "jsonb_typeof(os.state_blob -> 'completed_count') = 'number'",
            "os.state_blob ->> 'completed_count' = '0'",
            "stage_run_workers",
            "jsonb_each",
            "message_chains",
            "mc.session_id = tasks.session_id",
            "mc.task_id IS NULL OR mc.task_id = tasks.id",
            "mc.agent::text = current_stage_worker.worker ->> 'specialist'",
            "mc.chain IS NOT NULL",
            "os.superseded_by IS NULL",
        ] {
            assert!(sql.contains(required), "missing {required:?}: {sql}");
        }
        assert!(
            sql.contains("~* '^[0-9a-f]"),
            "run id must be UUID-shaped: {sql}"
        );
        assert!(
            sql.contains("#> ARRAY['stage_run_workers', os.current_stage]"),
            "worker map must be selected for the persisted current stage: {sql}"
        );
        assert!(
            sql.contains("os.state_blob -> 'graph_flow' IS NULL"),
            "flat repair must not mask a malformed graph checkpoint: {sql}"
        );
    }

    #[test]
    fn recoverable_checkpoint_predicate_requires_valid_graph_shape() {
        let sql = RECOVERABLE_ABANDONED_CHECKPOINT_SQL;
        assert!(
            sql.contains("jsonb_typeof(os.state_blob -> 'graph_flow') = 'object'"),
            "graph checkpoint must be an object: {sql}"
        );
        assert!(
            sql.contains("jsonb_typeof(os.state_blob -> 'graph_flow' -> 'state') = 'object'"),
            "graph state must be an object: {sql}"
        );
        assert!(
            sql.contains(
                "NULLIF(BTRIM(os.state_blob -> 'graph_flow' ->> 'next_node'), '') IS NOT NULL"
            ),
            "graph next node must be non-empty: {sql}"
        );
        assert!(
            !sql.contains("os.state_blob -> 'graph_flow' ->> 'next_node' = os.current_stage"),
            "a checkpoint may durably advance next_node before current_stage: {sql}"
        );
    }

    #[test]
    fn abandoned_result_marker_matches_fail_reaper_sql() {
        let sql = fail_abandoned_tasks_sql();
        assert!(!ABANDONED_TASK_RESULT.trim().is_empty());
        assert!(
            sql.contains(ABANDONED_TASK_RESULT),
            "exact-resume CAS marker drifted from fail SQL: {sql}"
        );
    }

    async fn evaluate_recoverable_checkpoint(
        pool: &sqlx::PgPool,
        state_blob: serde_json::Value,
        chain_id: uuid::Uuid,
        chain_agent: &str,
        include_chain: bool,
        chain_session_matches: bool,
    ) -> bool {
        let task_id = uuid::Uuid::new_v4();
        let task_session_id = uuid::Uuid::new_v4();
        let chain_session_id = if chain_session_matches {
            task_session_id
        } else {
            uuid::Uuid::new_v4()
        };
        let query = format!(
            r#"WITH tasks(id, session_id) AS (
                   VALUES ($1::uuid, $2::uuid)
               ),
               operation_state(operation_id, profile, current_stage, state_blob, superseded_by) AS (
                   VALUES ($1::uuid, 'pentest'::text, 'enumeration'::text, $3::jsonb, NULL::uuid)
               ),
               message_chains(id, session_id, task_id, agent, chain) AS (
                   SELECT $4::uuid, $5::uuid, $6::uuid, $7::agent_type, '[]'::jsonb
                   WHERE $8::bool
               )
               SELECT EXISTS (
                   SELECT 1
                   FROM operation_state os
                   CROSS JOIN tasks
                   WHERE {RECOVERABLE_ABANDONED_CHECKPOINT_SQL}
               )"#
        );
        sqlx::query_scalar::<_, bool>(&query)
            .bind(task_id)
            .bind(task_session_id)
            .bind(state_blob)
            .bind(chain_id)
            .bind(chain_session_id)
            .bind(Option::<uuid::Uuid>::None)
            .bind(chain_agent)
            .bind(include_chain)
            .fetch_one(pool)
            .await
            .expect("recoverable checkpoint predicate executes")
    }

    #[tokio::test]
    async fn recoverable_checkpoint_predicate_executes_on_postgres() {
        let Ok(database_url) = std::env::var("GOLISH_TEST_DATABASE_URL") else {
            eprintln!(
                "skip recoverable_checkpoint_predicate_executes_on_postgres: \
                 set GOLISH_TEST_DATABASE_URL to a migrated Postgres"
            );
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .expect("connect GOLISH_TEST_DATABASE_URL");
        let stage_run_id = uuid::Uuid::new_v4();
        let worker_org_id = uuid::Uuid::new_v4();
        let chain_id = uuid::Uuid::new_v4();
        let state_blob = serde_json::json!({
            "profile": "pentest",
            "current_stage": "enumeration",
            "current_stage_run_id": stage_run_id,
            "queue_titles": [],
            "completed_count": 0,
            "stage_run_workers": {
                "enumeration": {
                    worker_org_id.to_string(): {
                        "chain_id": chain_id,
                        "specialist": "enumerator"
                    }
                }
            }
        });
        assert!(
            evaluate_recoverable_checkpoint(
                &pool,
                state_blob.clone(),
                chain_id,
                "enumerator",
                true,
                true,
            )
            .await,
            "complete first-stage flat checkpoint must be recoverable"
        );
        assert!(
            !evaluate_recoverable_checkpoint(
                &pool,
                serde_json::json!({
                    "profile": "pentest",
                    "current_stage": "enumeration",
                    "current_stage_run_id": stage_run_id,
                    "queue_titles": [],
                    "completed_count": 0,
                    "stage_run_workers": {"enumeration": {"junk": {}}}
                }),
                chain_id,
                "enumerator",
                true,
                true,
            )
            .await,
            "malformed worker maps must not remain resumable forever"
        );
        assert!(
            !evaluate_recoverable_checkpoint(
                &pool,
                state_blob.clone(),
                chain_id,
                "enumerator",
                false,
                true,
            )
            .await,
            "missing durable chains must fail closed"
        );
        assert!(
            !evaluate_recoverable_checkpoint(
                &pool,
                state_blob.clone(),
                chain_id,
                "enumerator",
                true,
                false,
            )
            .await,
            "foreign-session chains must fail closed"
        );
        assert!(
            !evaluate_recoverable_checkpoint(
                &pool,
                state_blob.clone(),
                chain_id,
                "browser",
                true,
                true,
            )
            .await,
            "worker specialist and durable chain agent must match"
        );

        let duplicate_chain_workers = serde_json::json!({
            "profile": "pentest",
            "current_stage": "enumeration",
            "current_stage_run_id": stage_run_id,
            "queue_titles": [],
            "completed_count": 0,
            "stage_run_workers": {
                "enumeration": {
                    worker_org_id.to_string(): {
                        "chain_id": chain_id,
                        "specialist": "enumerator"
                    },
                    uuid::Uuid::new_v4().to_string(): {
                        "chain_id": chain_id,
                        "specialist": "enumerator"
                    }
                }
            }
        });
        assert!(
            !evaluate_recoverable_checkpoint(
                &pool,
                duplicate_chain_workers,
                chain_id,
                "enumerator",
                true,
                true,
            )
            .await,
            "two workers cannot claim the same exact chain"
        );

        let cross_stage_graph = serde_json::json!({
            "graph_flow": {
                "state": {"seeded": {}, "visited": ["enumeration"], "applied": {}, "wave": 0, "reopen_wave": false},
                "next_node": "vuln_triage"
            }
        });
        assert!(
            evaluate_recoverable_checkpoint(
                &pool,
                cross_stage_graph,
                chain_id,
                "enumerator",
                false,
                true,
            )
            .await,
            "a durable Continue(next) checkpoint may advance before current_stage"
        );
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
            !sql.contains("stage_run_workers") && !sql.contains("current_stage_run_id"),
            "general chat resume must not be broadened to CLI-only flat repair: {sql}"
        );
        assert!(
            sql.contains("ORDER BY t.created_at DESC") && sql.contains("LIMIT 1"),
            "must pick the newest resumable task: {sql}"
        );
    }
}
