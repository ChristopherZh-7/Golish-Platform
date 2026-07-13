use chrono::Duration;
use sqlx::{Executor, PgPool, Postgres};
use uuid::Uuid;

use crate::models::{NewTask, Task, TaskStatus};
use crate::Result;

const INSERT_TASK_WITH_ID_SQL: &str = r#"INSERT INTO tasks (id, session_id, title, input)
           VALUES ($1, $2, $3, $4)
           RETURNING *"#;

/// Insert a task with a server-preallocated identity through any PostgreSQL
/// executor. Runtime operation creation passes a transaction connection here
/// so the task and its `operation_state` row cannot commit independently.
pub async fn insert_with_id<'e, E>(
    executor: E,
    id: Uuid,
    session_id: Uuid,
    title: Option<&str>,
    input: &str,
) -> Result<Task>
where
    E: Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_as::<_, Task>(INSERT_TASK_WITH_ID_SQL)
        .bind(id)
        .bind(session_id)
        .bind(title)
        .bind(input)
        .fetch_one(executor)
        .await?;
    Ok(row)
}

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
fn latest_resumable_by_session_sql() -> String {
    let recoverable = latest_resumable_checkpoint_sql();
    format!(
        "SELECT tasks.* FROM tasks \
         JOIN operation_state os ON os.operation_id = tasks.id \
         WHERE tasks.session_id = $1 \
           AND tasks.status IN ('running', 'waiting') \
           AND {recoverable} \
         ORDER BY tasks.created_at DESC \
         LIMIT 1"
    )
}

/// Find the most recent **resumable** harness operation for a chat session's DB
/// session, or `None` if there is nothing to resume (→ caller starts fresh).
///
/// This is the state-driven signal that lets task mode decide resume-vs-new
/// without parsing the user's text (no "继续" keyword special-case): if a
/// checkpointed non-terminal task exists, the next message resumes it.
pub async fn latest_resumable_by_session(pool: &PgPool, session_id: Uuid) -> Result<Option<Task>> {
    let sql = latest_resumable_by_session_sql();
    let row = sqlx::query_as::<_, Task>(&sql)
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
const LEGACY_RECOVERABLE_CHECKPOINT_SQL: &str = r#"os.superseded_by IS NULL
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

const LEGACY_GRAPH_RESUME_SQL: &str = r#"os.superseded_by IS NULL
             AND jsonb_typeof(os.state_blob)='object'
             AND jsonb_typeof(os.state_blob -> 'graph_flow')='object'
             AND jsonb_typeof(os.state_blob -> 'graph_flow' -> 'state')='object'
             AND jsonb_typeof(os.state_blob -> 'graph_flow' -> 'state' -> 'seeded')='object'
             AND jsonb_typeof(os.state_blob -> 'graph_flow' -> 'state' -> 'visited')='array'
             AND jsonb_typeof(os.state_blob -> 'graph_flow' -> 'state' -> 'applied')='object'
             AND jsonb_typeof(os.state_blob -> 'graph_flow' -> 'next_node')='string'
             AND NULLIF(BTRIM(os.state_blob -> 'graph_flow' ->> 'next_node'),'') IS NOT NULL"#;

/// Complete relational V2 shape. Scoping is resumable only in the exact
/// pre-freeze state. Every later stage is owned by one sealed snapshot and one
/// exact active execution. Specialist stages own exactly one Unit/Worker per
/// frozen organization; non-specialist stages own exactly one root Unit and no
/// WorkerRun. Any partial/cross-op row makes the whole operation unavailable
/// rather than field-merging sources.
const V2_RELATIONAL_RECOVERABLE_SQL: &str = r#"os.superseded_by IS NULL
             AND os.project_scope_id IS NOT NULL
             AND (
                 SELECT COUNT(*) FROM stage_runs active_count
                 WHERE active_count.operation_id=os.operation_id
                   AND active_count.status='started'
             ) = 1
             AND EXISTS (
                 SELECT 1 FROM stage_runs active_execution
                 WHERE active_execution.operation_id=os.operation_id
                   AND active_execution.status='started'
                   AND active_execution.stage_kind=os.current_stage
             )
             AND (
                 (
                     os.current_stage='scoping'
                     AND NOT EXISTS (
                         SELECT 1 FROM operation_org_scope_snapshots snapshot
                         WHERE snapshot.operation_id=os.operation_id
                     )
                     AND NOT EXISTS (
                         SELECT 1 FROM stage_run_units unit
                         WHERE unit.operation_id=os.operation_id
                     )
                     AND NOT EXISTS (
                         SELECT 1 FROM stage_worker_runs worker
                         WHERE worker.operation_id=os.operation_id
                     )
                 )
                 OR
                 (
                     os.current_stage<>'scoping'
                     AND (
                         SELECT COUNT(*) FROM operation_org_scope_snapshots snapshot_count
                         WHERE snapshot_count.operation_id=os.operation_id
                           AND snapshot_count.project_scope_id=os.project_scope_id
                           AND snapshot_count.sealed_at IS NOT NULL
                     ) = 1
                     AND EXISTS (
                         SELECT 1
                         FROM operation_org_scope_snapshots snapshot
                         JOIN stage_runs active_execution
                           ON active_execution.operation_id=os.operation_id
                          AND active_execution.status='started'
                          AND active_execution.stage_kind=os.current_stage
                         WHERE snapshot.operation_id=os.operation_id
                           AND snapshot.project_scope_id=os.project_scope_id
                           AND snapshot.sealed_at IS NOT NULL
                           AND (
                               SELECT COUNT(*) FROM operation_org_scope_units member_count
                               WHERE member_count.snapshot_id=snapshot.id
                           ) > 0
                           AND NOT EXISTS (
                               SELECT 1
                               FROM stage_run_units stray_unit
                               WHERE stray_unit.operation_id=os.operation_id
                                 AND stray_unit.stage_execution_id=active_execution.id
                                 AND (
                                     stray_unit.scope_snapshot_id<>snapshot.id
                                     OR stray_unit.stage_kind<>os.current_stage
                                     OR NOT EXISTS (
                                         SELECT 1 FROM operation_org_scope_units member
                                         WHERE member.snapshot_id=snapshot.id
                                           AND member.organization_id=stray_unit.organization_id
                                     )
                                 )
                           )
                           AND (
                               (
                                   (
                                       SELECT COUNT(*) FROM stage_run_units root_unit_count
                                       WHERE root_unit_count.operation_id=os.operation_id
                                         AND root_unit_count.stage_execution_id=active_execution.id
                                   ) = 1
                                   AND EXISTS (
                                       SELECT 1 FROM stage_run_units root_unit
                                       WHERE root_unit.operation_id=os.operation_id
                                         AND root_unit.stage_execution_id=active_execution.id
                                         AND root_unit.scope_snapshot_id=snapshot.id
                                         AND root_unit.organization_id=snapshot.root_organization_id
                                         AND root_unit.stage_kind=os.current_stage
                                         AND root_unit.specialist IS NULL
                                         AND root_unit.status IN ('queued','running','gate_blocked','passed')
                                   )
                                   AND NOT EXISTS (
                                       SELECT 1 FROM stage_worker_runs unexpected_worker
                                       WHERE unexpected_worker.operation_id=os.operation_id
                                         AND unexpected_worker.stage_execution_id=active_execution.id
                                   )
                               )
                               OR
                               (
                                   (
                                       SELECT COUNT(*) FROM stage_run_units unit_count
                                       WHERE unit_count.operation_id=os.operation_id
                                         AND unit_count.stage_execution_id=active_execution.id
                                         AND unit_count.scope_snapshot_id=snapshot.id
                                   ) = (
                                       SELECT COUNT(*) FROM operation_org_scope_units member_count
                                       WHERE member_count.snapshot_id=snapshot.id
                                   )
                                   AND NOT EXISTS (
                                       SELECT 1
                                       FROM operation_org_scope_units member
                                       WHERE member.snapshot_id=snapshot.id
                                         AND (
                                             SELECT COUNT(*)
                                             FROM stage_run_units unit
                                             WHERE unit.operation_id=os.operation_id
                                               AND unit.stage_execution_id=active_execution.id
                                               AND unit.scope_snapshot_id=snapshot.id
                                               AND unit.organization_id=member.organization_id
                                               AND unit.stage_kind=os.current_stage
                                               AND NULLIF(BTRIM(unit.specialist),'') IS NOT NULL
                                         ) <> 1
                                   )
                                   AND NOT EXISTS (
                                       SELECT 1
                                       FROM stage_run_units unit
                                       WHERE unit.operation_id=os.operation_id
                                         AND unit.stage_execution_id=active_execution.id
                                         AND (
                                             NULLIF(BTRIM(unit.specialist),'') IS NULL
                                             OR (
                                                 SELECT COUNT(*) FROM stage_worker_runs worker_count
                                                 WHERE worker_count.operation_id=os.operation_id
                                                   AND worker_count.stage_execution_id=active_execution.id
                                                   AND worker_count.stage_run_unit_id=unit.id
                                                   AND worker_count.organization_id=unit.organization_id
                                                   AND worker_count.specialist=unit.specialist
                                             ) <> 1
                                         )
                                   )
                                   AND NOT EXISTS (
                                       SELECT 1
                                       FROM stage_worker_runs worker
                                       JOIN stage_run_units unit
                                         ON unit.id=worker.stage_run_unit_id
                                        AND unit.operation_id=worker.operation_id
                                        AND unit.stage_execution_id=worker.stage_execution_id
                                        AND unit.organization_id=worker.organization_id
                                       WHERE worker.operation_id=os.operation_id
                                         AND worker.stage_execution_id=active_execution.id
                                         AND (
                                             unit.scope_snapshot_id<>snapshot.id
                                             OR unit.stage_kind<>os.current_stage
                                             OR NULLIF(BTRIM(unit.specialist),'') IS NULL
                                             OR unit.specialist<>worker.specialist
                                             OR (
                                                 worker.message_chain_id IS NULL
                                                 AND worker.status<>'queued'
                                             )
                                             OR (
                                                 worker.message_chain_id IS NOT NULL
                                                 AND NOT EXISTS (
                                                     SELECT 1 FROM message_chains bound_chain
                                                     WHERE bound_chain.id=worker.message_chain_id
                                                       AND bound_chain.session_id=tasks.session_id
                                                       AND bound_chain.task_id=tasks.id
                                                       AND bound_chain.agent=(CASE
                                                           WHEN worker.specialist='reporter'
                                                               THEN 'reporter'::agent_type
                                                           WHEN worker.specialist IN (
                                                               'recon','prober','enumerator',
                                                               'vuln_scanner','attack_analyst',
                                                               'candidate_verifier','pentester'
                                                           ) THEN 'pentester'::agent_type
                                                           ELSE NULL
                                                       END)
                                                       AND jsonb_typeof(bound_chain.chain)='array'
                                                 )
                                             )
                                             OR (
                                                 unit.status='passed'
                                                 AND worker.status<>'passed'
                                             )
                                             OR (
                                                 unit.status IN ('queued','running','gate_blocked')
                                                 AND NOT (
                                                     (
                                                         worker.status='running'
                                                         AND worker.lease_token IS NOT NULL
                                                         AND worker.lease_expires_at IS NOT NULL
                                                     )
                                                     OR (
                                                         worker.status IN ('queued','gate_blocked','waiting_background')
                                                         AND worker.active_tool_call_id IS NULL
                                                         AND (
                                                             worker.lease_token IS NULL
                                                             OR worker.lease_expires_at<=NOW()
                                                         )
                                                     )
                                                     OR (
                                                         worker.status='recovery_required'
                                                         AND worker.active_tool_call_id IS NOT NULL
                                                     )
                                                 )
                                             )
                                             OR unit.status NOT IN ('queued','running','gate_blocked','passed')
                                             OR (
                                                 worker.active_tool_call_id IS NOT NULL
                                                 AND NOT EXISTS (
                                                     SELECT 1 FROM tool_calls active_tool
                                                     WHERE active_tool.id=worker.active_tool_call_id
                                                       AND active_tool.worker_run_id=worker.id
                                                       AND active_tool.operation_id=worker.operation_id
                                                       AND active_tool.stage_execution_id=worker.stage_execution_id
                                                       AND active_tool.stage_run_unit_id=worker.stage_run_unit_id
                                                       AND active_tool.organization_id=worker.organization_id
                                                       AND active_tool.attempt_epoch=worker.attempt_epoch
                                                       AND active_tool.lease_token=worker.lease_token
                                                       AND active_tool.status IN ('received','running')
                                                 )
                                             )
                                         )
                                   )
                               )
                           )
                     )
                 )
             )"#;

/// Persisted contract chooses one complete source. `dual_write_v2_preferred`
/// may fall back to a complete legacy checkpoint, but never combines fields;
/// `v2_only` has no fallback.
const RECOVERABLE_ABANDONED_CHECKPOINT_SQL: &str = r#"(
                 (
                     os.runtime_memory_contract IN ('legacy_v1','dual_write_legacy_read')
                     AND (LEGACY_RECOVERABLE_CHECKPOINT_SQL)
                 )
                 OR (
                     os.runtime_memory_contract='dual_write_v2_preferred'
                     AND (
                         (V2_RELATIONAL_RECOVERABLE_SQL)
                         OR (LEGACY_RECOVERABLE_CHECKPOINT_SQL)
                     )
                 )
                 OR (
                     os.runtime_memory_contract='v2_only'
                     AND (V2_RELATIONAL_RECOVERABLE_SQL)
                 )
             )"#;

fn recoverable_abandoned_checkpoint_sql() -> String {
    RECOVERABLE_ABANDONED_CHECKPOINT_SQL
        .replace(
            "LEGACY_RECOVERABLE_CHECKPOINT_SQL",
            LEGACY_RECOVERABLE_CHECKPOINT_SQL,
        )
        .replace(
            "V2_RELATIONAL_RECOVERABLE_SQL",
            V2_RELATIONAL_RECOVERABLE_SQL,
        )
}

fn latest_resumable_checkpoint_sql() -> String {
    format!(
        r#"(
            (
                os.runtime_memory_contract IN ('legacy_v1','dual_write_legacy_read')
                AND ({LEGACY_GRAPH_RESUME_SQL})
            )
            OR (
                os.runtime_memory_contract='dual_write_v2_preferred'
                AND (({V2_RELATIONAL_RECOVERABLE_SQL}) OR ({LEGACY_GRAPH_RESUME_SQL}))
            )
            OR (
                os.runtime_memory_contract='v2_only'
                AND ({V2_RELATIONAL_RECOVERABLE_SQL})
            )
        )"#
    )
}

/// Select the one complete runtime-memory source for an exact production
/// resume. Preferred mode tests the complete relational record first and only
/// then selects the complete graph checkpoint as a legacy fallback; the caller
/// receives one source token and must pin every resume-time read to it.
fn exact_resumable_runtime_source_sql() -> String {
    format!(
        r#"SELECT CASE
                 WHEN os.runtime_memory_contract IN ('legacy_v1','dual_write_legacy_read')
                      AND ({LEGACY_GRAPH_RESUME_SQL})
                   THEN 'legacy'
                 WHEN os.runtime_memory_contract='dual_write_v2_preferred'
                      AND ({V2_RELATIONAL_RECOVERABLE_SQL})
                   THEN 'v2'
                 WHEN os.runtime_memory_contract='dual_write_v2_preferred'
                      AND ({LEGACY_GRAPH_RESUME_SQL})
                   THEN 'legacy_fallback'
                 WHEN os.runtime_memory_contract='v2_only'
                      AND ({V2_RELATIONAL_RECOVERABLE_SQL})
                   THEN 'v2'
             END AS source
             FROM tasks
             JOIN operation_state os ON os.operation_id=tasks.id
            WHERE tasks.id=$1 AND tasks.session_id=$2
              AND tasks.status='waiting' AND tasks.result IS NULL
              AND NOT EXISTS (
                  SELECT 1 FROM stage_worker_runs live_worker
                   WHERE live_worker.operation_id=tasks.id
                     AND live_worker.lease_token IS NOT NULL
                     AND live_worker.lease_expires_at>NOW()
              )"#
    )
}

pub async fn exact_resumable_runtime_source(
    pool: &PgPool,
    task_id: Uuid,
    session_id: Uuid,
) -> Result<Option<super::runtime_memory_tx::RuntimeMemoryRecordSource>> {
    use super::runtime_memory_tx::RuntimeMemoryRecordSource;

    let source = sqlx::query_scalar::<_, Option<String>>(&exact_resumable_runtime_source_sql())
        .bind(task_id)
        .bind(session_id)
        .fetch_optional(pool)
        .await?
        .flatten();
    source
        .map(|source| match source.as_str() {
            "legacy" => Ok(RuntimeMemoryRecordSource::Legacy),
            "v2" => Ok(RuntimeMemoryRecordSource::V2),
            "legacy_fallback" => Ok(RuntimeMemoryRecordSource::LegacyFallback),
            _ => Err(anyhow::anyhow!("unknown exact resume runtime-memory source {source}").into()),
        })
        .transpose()
}

/// Atomically claim the exact idle task after the caller has decoded the
/// selected whole-record source. The source is rebuilt inside the UPDATE and
/// must still equal `expected_source`; two concurrent resume callers can
/// therefore never both cross the waiting -> running boundary.
fn claim_exact_resumable_runtime_source_sql() -> String {
    let selected_source = exact_resumable_runtime_source_sql();
    format!(
        r#"WITH exact_source AS (
               {selected_source}
           )
           UPDATE tasks
              SET status='running', updated_at=NOW()
             FROM exact_source
            WHERE tasks.id=$1
              AND tasks.session_id=$2
              AND tasks.status='waiting'
              AND tasks.result IS NULL
              AND exact_source.source=$3
           RETURNING exact_source.source"#
    )
}

pub async fn claim_exact_resumable_runtime_source(
    pool: &PgPool,
    task_id: Uuid,
    session_id: Uuid,
    expected_source: super::runtime_memory_tx::RuntimeMemoryRecordSource,
) -> Result<bool> {
    use super::runtime_memory_tx::RuntimeMemoryRecordSource;

    let expected_source = match expected_source {
        RuntimeMemoryRecordSource::Legacy => "legacy",
        RuntimeMemoryRecordSource::V2 => "v2",
        RuntimeMemoryRecordSource::LegacyFallback => "legacy_fallback",
    };
    let claimed =
        sqlx::query_scalar::<_, Option<String>>(&claim_exact_resumable_runtime_source_sql())
            .bind(task_id)
            .bind(session_id)
            .bind(expected_source)
            .fetch_optional(pool)
            .await?
            .flatten();
    Ok(claimed.as_deref() == Some(expected_source))
}

const V2_LIVE_LEASE_SQL: &str = r#"os.runtime_memory_contract IN (
                 'dual_write_v2_preferred','v2_only'
             )
             AND EXISTS (
                 SELECT 1
                 FROM stage_worker_runs live_worker
                 JOIN stage_runs live_execution
                   ON live_execution.id=live_worker.stage_execution_id
                  AND live_execution.operation_id=os.operation_id
                  AND live_execution.status='started'
                  AND live_execution.stage_kind=os.current_stage
                 JOIN stage_run_units live_unit
                   ON live_unit.id=live_worker.stage_run_unit_id
                  AND live_unit.operation_id=os.operation_id
                  AND live_unit.stage_execution_id=live_execution.id
                  AND live_unit.organization_id=live_worker.organization_id
                 WHERE live_worker.operation_id=os.operation_id
                   AND live_worker.status='running'
                   AND live_worker.lease_token IS NOT NULL
                   AND live_worker.lease_expires_at>NOW()
             )"#;

/// SQL for [`fail_abandoned`]. Built from the shared recoverability predicate
/// so its complement is exactly the set handled by
/// [`pause_resumable_abandoned`].
fn fail_abandoned_tasks_sql() -> String {
    let recoverable = recoverable_abandoned_checkpoint_sql();
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
                 AND {recoverable} \
           ) \
           AND NOT EXISTS ( \
               SELECT 1 FROM operation_state os \
               WHERE os.operation_id = tasks.id \
                 AND {V2_RELATIONAL_RECOVERABLE_SQL} \
                 AND {V2_LIVE_LEASE_SQL} \
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
    let recoverable = recoverable_abandoned_checkpoint_sql();
    format!(
        "UPDATE tasks \
         SET status = 'waiting', updated_at = NOW() \
         WHERE status = 'running' \
           AND updated_at < $1 \
           AND EXISTS ( \
               SELECT 1 FROM operation_state os \
               WHERE os.operation_id = tasks.id \
                 AND {recoverable} \
           ) \
           AND NOT EXISTS ( \
               SELECT 1 FROM operation_state os \
               WHERE os.operation_id = tasks.id \
                 AND {V2_RELATIONAL_RECOVERABLE_SQL} \
                 AND {V2_LIVE_LEASE_SQL} \
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StartupTaskReaperStats {
    pub paused: u64,
    pub failed: u64,
    pub workers_requeued: u64,
    pub workers_recovery_required: u64,
    pub runtime_shadow_samples_written: u64,
}

/// One startup transaction reconciles expired V2 worker leases first, then
/// pauses complete resumable operations and fails every malformed/incomplete
/// remainder. A live relational lease is excluded from both task updates.
pub async fn startup_reap_abandoned(
    pool: &PgPool,
    threshold: Duration,
) -> Result<StartupTaskReaperStats> {
    let cutoff = crate::repo::audit::reclaim_cutoff(threshold);
    let mut tx = pool.begin().await?;
    let workers = super::runtime_memory_tx::reap_expired_workers_on_startup(&mut tx, cutoff)
        .await
        .map_err(|error| crate::DbError::Other(anyhow::Error::new(error)))?;
    let pause_sql = pause_resumable_abandoned_tasks_sql();
    let paused = sqlx::query(&pause_sql)
        .bind(cutoff)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    let fail_sql = fail_abandoned_tasks_sql();
    let failed = sqlx::query(&fail_sql)
        .bind(cutoff)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    tx.commit().await?;
    if workers.shadow_samples_written > 0 {
        super::runtime_memory_tx::reconcile_deployment_rollouts_best_effort(
            pool,
            "startup_reap_abandoned",
        )
        .await;
    }
    Ok(StartupTaskReaperStats {
        paused,
        failed,
        workers_requeued: workers.requeued,
        workers_recovery_required: workers.recovery_required,
        runtime_shadow_samples_written: workers.shadow_samples_written,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        claim_exact_resumable_runtime_source_sql, exact_resumable_runtime_source_sql,
        fail_abandoned_tasks_sql, latest_resumable_by_session_sql,
        pause_resumable_abandoned_tasks_sql, recoverable_abandoned_checkpoint_sql,
        ABANDONED_TASK_RESULT, INSERT_TASK_WITH_ID_SQL, LEGACY_GRAPH_RESUME_SQL,
        LEGACY_RECOVERABLE_CHECKPOINT_SQL, V2_LIVE_LEASE_SQL, V2_RELATIONAL_RECOVERABLE_SQL,
    };

    #[test]
    fn exact_resume_source_prefers_one_complete_v2_record_before_legacy_fallback() {
        let sql = exact_resumable_runtime_source_sql();
        let preferred_v2 = sql
            .find("THEN 'v2'")
            .expect("preferred relational source branch");
        let preferred_legacy = sql
            .find("THEN 'legacy_fallback'")
            .expect("preferred legacy fallback branch");
        assert!(preferred_v2 < preferred_legacy, "sql={sql}");
        assert!(sql.contains(V2_RELATIONAL_RECOVERABLE_SQL), "sql={sql}");
        assert!(sql.contains(LEGACY_GRAPH_RESUME_SQL), "sql={sql}");
        assert!(sql.contains("tasks.id=$1 AND tasks.session_id=$2"));
        assert!(sql.contains("tasks.status='waiting' AND tasks.result IS NULL"));
        assert!(sql.contains("live_worker.lease_expires_at>NOW()"));
    }

    #[test]
    fn exact_resume_source_claim_is_one_atomic_waiting_to_running_cas() {
        let sql = claim_exact_resumable_runtime_source_sql();
        assert!(sql.contains("WITH exact_source AS"), "sql={sql}");
        assert!(sql.contains("UPDATE tasks"), "sql={sql}");
        assert!(sql.contains("SET status='running'"), "sql={sql}");
        assert!(sql.contains("tasks.status='waiting'"), "sql={sql}");
        assert!(sql.contains("tasks.result IS NULL"), "sql={sql}");
        assert!(sql.contains("exact_source.source=$3"), "sql={sql}");
        assert!(sql.contains("RETURNING exact_source.source"), "sql={sql}");
    }

    #[test]
    fn runtime_memory_store_task_insert_accepts_server_preallocated_identity() {
        assert!(INSERT_TASK_WITH_ID_SQL.contains("INSERT INTO tasks"));
        assert!(INSERT_TASK_WITH_ID_SQL.contains("(id, session_id, title, input)"));
        assert!(INSERT_TASK_WITH_ID_SQL.contains("VALUES ($1, $2, $3, $4)"));
        assert!(INSERT_TASK_WITH_ID_SQL.contains("RETURNING *"));
    }

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
        let predicate = recoverable_abandoned_checkpoint_sql();
        let fail_sql = fail_abandoned_tasks_sql();
        let pause_sql = pause_resumable_abandoned_tasks_sql();

        assert_eq!(fail_sql.matches(&predicate).count(), 1, "sql={fail_sql}");
        assert_eq!(pause_sql.matches(&predicate).count(), 1, "sql={pause_sql}");
    }

    /// A flat first-stage checkpoint is recoverable only when every identity
    /// witness needed by the explicit CLI repair path is present. Merely having
    /// one or two JSON keys must never exempt a zombie from the fail reaper.
    #[test]
    fn recoverable_checkpoint_predicate_requires_complete_flat_identity() {
        let sql = LEGACY_RECOVERABLE_CHECKPOINT_SQL;

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
        let sql = LEGACY_RECOVERABLE_CHECKPOINT_SQL;
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
                   WHERE {LEGACY_RECOVERABLE_CHECKPOINT_SQL}
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
        let sql = latest_resumable_by_session_sql();
        assert!(sql.contains("FROM tasks"), "sql={sql}");
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
            sql.contains("ORDER BY tasks.created_at DESC") && sql.contains("LIMIT 1"),
            "must pick the newest resumable task: {sql}"
        );
    }

    #[test]
    fn startup_reaper_branches_by_contract_and_relational_worker_safety() {
        let recoverable = recoverable_abandoned_checkpoint_sql();
        for contract in [
            "legacy_v1",
            "dual_write_legacy_read",
            "dual_write_v2_preferred",
            "v2_only",
        ] {
            assert!(
                recoverable.contains(contract),
                "contract={contract}: {recoverable}"
            );
        }
        for required in [
            "active_count.status='started'",
            "snapshot.sealed_at IS NOT NULL",
            "unit_count.scope_snapshot_id=snapshot.id",
            "root_unit.specialist IS NULL",
            "root_unit.organization_id=snapshot.root_organization_id",
            "unexpected_worker.stage_execution_id=active_execution.id",
            "worker.status='running'",
            "worker.status='recovery_required'",
            "worker.active_tool_call_id IS NOT NULL",
            "active_tool.attempt_epoch=worker.attempt_epoch",
            "active_tool.lease_token=worker.lease_token",
        ] {
            assert!(
                V2_RELATIONAL_RECOVERABLE_SQL.contains(required),
                "missing {required}: {V2_RELATIONAL_RECOVERABLE_SQL}"
            );
        }
        assert!(V2_LIVE_LEASE_SQL.contains("lease_expires_at>NOW()"));
        assert!(fail_abandoned_tasks_sql().contains(V2_LIVE_LEASE_SQL));
        assert!(pause_resumable_abandoned_tasks_sql().contains(V2_LIVE_LEASE_SQL));
    }
}
