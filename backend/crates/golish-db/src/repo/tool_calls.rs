use crate::{DbError, Result};
use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::models::{NewToolCall, ToolCall, ToolcallStatus};

const RECORD_TRACKED_START_SQL: &str = r#"INSERT INTO tool_calls (
        call_id, session_id, task_id, subtask_id, agent, name, args, status, source,
        operation_id, stage_execution_id, stage_run_unit_id, worker_run_id,
        organization_id, attempt_epoch, lease_token
    ) VALUES (
        $1, $2, $3, $4, 'primary'::agent_type, $5, $6,
        'running'::toolcall_status, 'ai', $7, $8, $9, $10, $11, $12, $13
    )
    RETURNING id"#;

const RECORD_TRACKED_FINISH_SQL: &str = r#"UPDATE tool_calls
    SET status = $1::toolcall_status,
        result = $2,
        duration_ms = $3,
        updated_at = NOW()
    WHERE id = $4
      AND session_id = $5
      AND status IN ('received', 'running')"#;

/// Concrete DB-side projection of the sqlx-free agent-kit runtime identity.
/// The database constraints and worker-fence trigger remain the final authority
/// for which optional-field shapes are valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeToolIdentity {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Option<Uuid>,
    pub worker_run_id: Option<Uuid>,
    pub organization_id: Option<Uuid>,
    pub attempt_epoch: Option<i64>,
    pub lease_token: Option<Uuid>,
}

async fn lock_runtime_tool_identity_for_start(
    tx: &mut Transaction<'_, Postgres>,
    identity: &RuntimeToolIdentity,
) -> Result<()> {
    let operation = sqlx::query_scalar::<_, Uuid>(
        "SELECT operation_id FROM operation_state WHERE operation_id = $1 FOR SHARE",
    )
    .bind(identity.operation_id)
    .fetch_optional(&mut **tx)
    .await?;
    if operation.is_none() {
        return Err(DbError::NotFound(format!(
            "runtime tool operation {}",
            identity.operation_id
        )));
    }

    if let Some(stage_run_unit_id) = identity.stage_run_unit_id {
        let organization_id = identity.organization_id.ok_or_else(|| {
            DbError::Other(anyhow::anyhow!(
                "runtime tool unit identity requires organization_id"
            ))
        })?;
        let unit = sqlx::query_scalar::<_, Uuid>(
            r#"SELECT id FROM stage_run_units
               WHERE id = $1 AND operation_id = $2 AND stage_execution_id = $3
                 AND organization_id = $4
               FOR SHARE"#,
        )
        .bind(stage_run_unit_id)
        .bind(identity.operation_id)
        .bind(identity.stage_execution_id)
        .bind(organization_id)
        .fetch_optional(&mut **tx)
        .await?;
        if unit.is_none() {
            return Err(DbError::NotFound(format!(
                "runtime tool stage unit {stage_run_unit_id}"
            )));
        }
    }

    let stage = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT id FROM stage_runs
           WHERE id = $1 AND operation_id = $2
           FOR SHARE"#,
    )
    .bind(identity.stage_execution_id)
    .bind(identity.operation_id)
    .fetch_optional(&mut **tx)
    .await?;
    if stage.is_none() {
        return Err(DbError::NotFound(format!(
            "runtime tool stage execution {}",
            identity.stage_execution_id
        )));
    }

    if let Some(worker_run_id) = identity.worker_run_id {
        let stage_run_unit_id = identity.stage_run_unit_id.ok_or_else(|| {
            DbError::Other(anyhow::anyhow!(
                "runtime worker tool identity requires stage_run_unit_id"
            ))
        })?;
        let organization_id = identity.organization_id.ok_or_else(|| {
            DbError::Other(anyhow::anyhow!(
                "runtime worker tool identity requires organization_id"
            ))
        })?;
        let attempt_epoch = identity.attempt_epoch.ok_or_else(|| {
            DbError::Other(anyhow::anyhow!(
                "runtime worker tool identity requires attempt_epoch"
            ))
        })?;
        let lease_token = identity.lease_token.ok_or_else(|| {
            DbError::Other(anyhow::anyhow!(
                "runtime worker tool identity requires lease_token"
            ))
        })?;
        let worker = sqlx::query_scalar::<_, Uuid>(
            r#"SELECT id FROM stage_worker_runs
               WHERE id = $1 AND operation_id = $2 AND stage_execution_id = $3
                 AND stage_run_unit_id = $4 AND organization_id = $5
                 AND attempt_epoch = $6 AND lease_token = $7
               FOR SHARE"#,
        )
        .bind(worker_run_id)
        .bind(identity.operation_id)
        .bind(identity.stage_execution_id)
        .bind(stage_run_unit_id)
        .bind(organization_id)
        .bind(attempt_epoch)
        .bind(lease_token)
        .fetch_optional(&mut **tx)
        .await?;
        if worker.is_none() {
            return Err(DbError::NotFound(format!(
                "runtime tool worker fence {worker_run_id}"
            )));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn has_exact_active_worker_fence(
    pool: &PgPool,
    id: Uuid,
    worker_run_id: Uuid,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    organization_id: Uuid,
    attempt_epoch: i64,
    lease_token: Option<Uuid>,
) -> Result<bool> {
    Ok(sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM tool_calls
                WHERE id=$1 AND worker_run_id=$2 AND operation_id=$3
                  AND stage_execution_id=$4 AND stage_run_unit_id=$5
                  AND organization_id=$6 AND attempt_epoch=$7
                  AND lease_token=$8 AND status IN ('received','running')
           )"#,
    )
    .bind(id)
    .bind(worker_run_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(organization_id)
    .bind(attempt_epoch)
    .bind(lease_token)
    .fetch_one(pool)
    .await?)
}

/// One terminal Scoping control-plane call bound to the exact operation and
/// StageExecution. The ordered lifecycle is the only input accepted by the V2
/// scope-decision derivation; session/time-window rows are legacy gate hints.
#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]
pub struct ExactScopingLifecycleRow {
    pub id: Uuid,
    pub call_id: String,
    pub session_id: Uuid,
    pub name: String,
    pub args: serde_json::Value,
    pub result: Option<String>,
    pub created_at: DateTime<Utc>,
}

const EXACT_SCOPING_LIFECYCLE_SQL: &str = r#"SELECT id, call_id, session_id,
       name, args, result, created_at
FROM tool_calls
WHERE operation_id = $1
  AND stage_execution_id = $2
  AND task_id = $1
  AND status = 'finished'::toolcall_status
  AND (
      (name = 'ask_human'
       AND args->>'input_type' IN ('choice', 'unit_review', 'scope_review'))
      OR
      (name = 'manage_organizations'
       AND args->>'action' IN ('propose_candidates', 'create', 'create_batch'))
  )
ORDER BY created_at ASC, id ASC"#;

pub async fn scoping_lifecycle_for_execution(
    pool: &PgPool,
    operation_id: Uuid,
    stage_execution_id: Uuid,
) -> Result<Vec<ExactScopingLifecycleRow>> {
    let mut connection = pool.acquire().await?;
    scoping_lifecycle_for_execution_with_connection(
        &mut connection,
        operation_id,
        stage_execution_id,
    )
    .await
}

pub async fn scoping_lifecycle_for_execution_with_connection(
    connection: &mut PgConnection,
    operation_id: Uuid,
    stage_execution_id: Uuid,
) -> Result<Vec<ExactScopingLifecycleRow>> {
    Ok(
        sqlx::query_as::<_, ExactScopingLifecycleRow>(EXACT_SCOPING_LIFECYCLE_SQL)
            .bind(operation_id)
            .bind(stage_execution_id)
            .fetch_all(connection)
            .await?,
    )
}

/// Insert the durable start row and return its DB primary key. Runtime-aware
/// callers must receive any FK/fence/shape failure before dispatching the tool.
#[allow(clippy::too_many_arguments)]
pub async fn record_tracked_start(
    pool: &PgPool,
    call_id: &str,
    session_id: Uuid,
    task_id: Option<Uuid>,
    subtask_id: Option<Uuid>,
    tool_name: &str,
    args: &serde_json::Value,
    runtime: Option<&RuntimeToolIdentity>,
) -> Result<Uuid> {
    let mut tx = pool.begin().await?;
    if let Some(runtime) = runtime {
        // Match the runtime-memory transaction order before the INSERT's
        // immediate FKs and worker-fence trigger acquire their own share
        // locks. This prevents stage->unit / worker->stage lock inversions
        // against concurrent heartbeat and checkpoint transactions.
        lock_runtime_tool_identity_for_start(&mut tx, runtime).await?;
    }
    let record_id = sqlx::query_scalar::<_, Uuid>(RECORD_TRACKED_START_SQL)
        .bind(call_id)
        .bind(session_id)
        .bind(task_id)
        .bind(subtask_id)
        .bind(tool_name)
        .bind(args)
        .bind(runtime.map(|identity| identity.operation_id))
        .bind(runtime.map(|identity| identity.stage_execution_id))
        .bind(runtime.and_then(|identity| identity.stage_run_unit_id))
        .bind(runtime.and_then(|identity| identity.worker_run_id))
        .bind(runtime.and_then(|identity| identity.organization_id))
        .bind(runtime.and_then(|identity| identity.attempt_epoch))
        .bind(runtime.and_then(|identity| identity.lease_token))
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(record_id)
}

/// Finish exactly one start row by trusted DB identity plus its start-session
/// owner. The running-state predicate makes duplicate/late finalization a CAS
/// miss instead of silently rewriting terminal telemetry.
pub async fn record_tracked_finish(
    pool: &PgPool,
    record_id: Uuid,
    session_id: Uuid,
    status: &str,
    result: &str,
    duration_ms: i32,
) -> Result<()> {
    let rows_affected = sqlx::query(RECORD_TRACKED_FINISH_SQL)
        .bind(status)
        .bind(result)
        .bind(duration_ms)
        .bind(record_id)
        .bind(session_id)
        .execute(pool)
        .await?
        .rows_affected();
    require_exactly_one_finish(rows_affected, record_id, session_id)
}

fn require_exactly_one_finish(rows_affected: u64, record_id: Uuid, session_id: Uuid) -> Result<()> {
    if rows_affected == 1 {
        return Ok(());
    }
    Err(DbError::NotFound(format!(
        "active tool_call id={record_id} session_id={session_id}"
    )))
}

pub async fn create(pool: &PgPool, tc: NewToolCall) -> Result<ToolCall> {
    let row = sqlx::query_as::<_, ToolCall>(
        r#"INSERT INTO tool_calls (call_id, session_id, task_id, subtask_id, agent, name, args, source)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           RETURNING *"#,
    )
    .bind(&tc.call_id)
    .bind(tc.session_id)
    .bind(tc.task_id)
    .bind(tc.subtask_id)
    .bind(tc.agent)
    .bind(&tc.name)
    .bind(&tc.args)
    .bind(&tc.source)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<ToolCall>> {
    super::scoped::get_by_id(pool, "tool_calls", id).await
}

pub async fn list_by_session(pool: &PgPool, session_id: Uuid) -> Result<Vec<ToolCall>> {
    let rows = sqlx::query_as::<_, ToolCall>(
        "SELECT * FROM tool_calls WHERE session_id = $1 ORDER BY created_at ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Parse the org UUID(s) a `manage_organizations(action="create"/"create_batch")`
/// call reported on success. The single-create tool returns
/// `{"action":"create","id":"<uuid>",...}` on success
/// and `{"error":...}` on failure — and it **swallows** DB errors (e.g. a
/// duplicate-name unique-constraint hit) into an `Ok({"error":...})` body, so the
/// recorded `result` payload — NOT the tool_call `status` (always `finished`) —
/// is the only reliable success signal. Returns an empty list for a failed/garbage
/// result so a mere create *attempt* never counts as an actual creation.
fn org_ids_from_create_result(result: &str) -> Vec<Uuid> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(result) else {
        return Vec::new();
    };
    if v.get("error").is_some() {
        return Vec::new();
    }
    if let Some(id) = v
        .get("id")
        .and_then(|x| x.as_str())
        .and_then(|s| s.parse().ok())
    {
        return vec![id];
    }

    let mut ids = Vec::new();
    for key in ["created", "existing"] {
        let Some(items) = v.get(key).and_then(|x| x.as_array()) else {
            continue;
        };
        ids.extend(items.iter().filter_map(|item| {
            item.get("id")
                .and_then(|x| x.as_str())
                .and_then(|s| s.parse::<Uuid>().ok())
        }));
    }
    ids
}

fn approved_human_response(result: Option<&str>) -> Option<String> {
    let result = serde_json::from_str::<serde_json::Value>(result?).ok()?;
    if result.get("error").is_some()
        || result
            .get("skipped")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    {
        return None;
    }
    let response = result.get("response")?.as_str()?.trim();
    (!response.is_empty()).then(|| response.to_string())
}

fn context_organization_id(args: &serde_json::Value) -> Option<Uuid> {
    let context = args.get("context")?.as_str()?;
    serde_json::from_str::<serde_json::Value>(context)
        .ok()?
        .get("organization_id")?
        .as_str()?
        .parse()
        .ok()
}

pub(crate) fn subsidiary_scope_decision(
    args: &serde_json::Value,
    result: Option<&str>,
    expected_organization_id: Uuid,
    expected_organization_name: Option<&str>,
) -> Option<bool> {
    if args.get("input_type").and_then(serde_json::Value::as_str) != Some("choice") {
        return None;
    }

    let context = args
        .get("context")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let question = args
        .get("question")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let options = args
        .get("options")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .collect::<Vec<_>>()
        .join(" ");
    let structured_context = serde_json::from_str::<serde_json::Value>(context).ok();
    let structured_decision = structured_context
        .as_ref()
        .and_then(|value| value.get("decision"))
        .and_then(serde_json::Value::as_str);
    let prompt_text = format!("{context} {question} {options}").to_ascii_lowercase();
    let subsidiary_words = prompt_text.contains("subsidiar")
        || prompt_text.contains("子公司")
        || prompt_text.contains("分支机构");
    let subsidiary_decision = match structured_decision {
        Some(decision) if decision.eq_ignore_ascii_case("subsidiary_scope") => {
            context_organization_id(args) == Some(expected_organization_id)
        }
        Some(_) => false,
        None => {
            // Backward compatibility for in-flight runs created before the
            // structured context contract landed. The exact current root name
            // must occur in the prompt; a generic subsidiary choice from a
            // sibling engagement can never satisfy this gate.
            let exact_root_named = expected_organization_name
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(|name| prompt_text.contains(&name.to_ascii_lowercase()))
                .unwrap_or(false);
            subsidiary_words && exact_root_named
        }
    };
    if !subsidiary_decision {
        return None;
    }

    let response = approved_human_response(result)?.to_ascii_lowercase();
    if [
        "root_only",
        "不纳入子公司",
        "不包含子公司",
        "仅母公司",
        "仅测试母公司",
        "只测试母公司",
        "no subsidiaries",
        "exclude subsidiaries",
        "parent company only",
        "root only",
        "root-only",
    ]
    .iter()
    .any(|marker| response.contains(marker))
    {
        return Some(true);
    }
    if [
        "include_51",
        "include_100",
        "纳入：",
        "纳入:",
        "纳入子公司",
        "include subsidiaries",
        "subsidiaries in scope",
    ]
    .iter()
    .any(|marker| response.contains(marker))
    {
        return Some(false);
    }
    None
}

fn successful_candidate_proposal(
    args: &serde_json::Value,
    result: Option<&str>,
    expected_organization_id: Uuid,
) -> bool {
    if args.get("action").and_then(serde_json::Value::as_str) != Some("propose_candidates")
        || args
            .get("organization_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|id| id.parse::<Uuid>().ok())
            != Some(expected_organization_id)
    {
        return false;
    }
    let Some(result) =
        result.and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
    else {
        return false;
    };
    result.get("error").is_none()
        && result.get("action").and_then(serde_json::Value::as_str) == Some("propose_candidates")
        && result
            .get("organization_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|id| id.parse::<Uuid>().ok())
            == Some(expected_organization_id)
        && result
            .get("recorded")
            .and_then(serde_json::Value::as_i64)
            .is_some()
}

fn approved_unit_review(
    args: &serde_json::Value,
    result: Option<&str>,
    expected_organization_id: Uuid,
) -> bool {
    if args.get("input_type").and_then(serde_json::Value::as_str) != Some("unit_review")
        || context_organization_id(args) != Some(expected_organization_id)
    {
        return false;
    }
    approved_human_response(result)
        .and_then(|response| serde_json::from_str::<serde_json::Value>(&response).ok())
        .and_then(|response| {
            response
                .as_array()
                .map(|_| ())
                .or_else(|| response.get("rows")?.as_array().map(|_| ()))
        })
        .is_some()
}

fn unit_flow_for_org(
    rows: &[(String, serde_json::Value, Option<String>)],
    expected_organization_id: Uuid,
) -> (bool, bool) {
    let mut proposal_completed = false;
    let mut review_completed = false;
    for (name, args, result) in rows {
        if name == "manage_organizations"
            && successful_candidate_proposal(args, result.as_deref(), expected_organization_id)
        {
            proposal_completed = true;
        } else if name == "ask_human"
            && proposal_completed
            && approved_unit_review(args, result.as_deref(), expected_organization_id)
        {
            review_completed = true;
        }
    }
    (proposal_completed, review_completed)
}

/// For the red_team scoping gate: cross-verify (against real recorded tool calls
/// AND the resulting DB state) whether this session actually performed the
/// unit-candidate review flow rather than just asserting a claim or merely
/// *attempting* a create.
///
/// Returns `(total_calls, unit_candidates_proposed, unit_review_invoked,
/// subsidiaries_excluded, organization_created, scope_review_results)`:
/// - only rows created at/after `not_before` belong to the current operation's
///   Scoping stage; an older approval in the same chat session is never reused.
/// - `total_calls` lets the caller distinguish absent tracking from a completed
///   lifecycle that simply omitted the required review.
/// - `unit_review_invoked`: a successful, non-skipped, parseable same-root
///   `ask_human(input_type="unit_review")` completed after candidate proposal.
/// - `subsidiaries_excluded`: the latest parseable persisted subsidiary-scope
///   choice explicitly limited this engagement to the root/parent organization.
/// - `organization_created`: a `manage_organizations(action="create"/"create_batch")`
///   call this session reported a real org id for, AND that org row actually
///   exists in `organizations` now. A swallowed duplicate-key failure (no id in
///   the result) or a since-deleted row ⇒ `false`, so a failed create can no
///   longer pass the gate (AGENTS.md I7/I8: "attempted" ≠ "actually recorded").
pub async fn scoping_actions_for_session(
    pool: &PgPool,
    session_id: Uuid,
    organization_id: Uuid,
    not_before: chrono::DateTime<chrono::Utc>,
) -> Result<(i64, bool, bool, bool, bool, Vec<Option<String>>)> {
    let total: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
           FROM tool_calls
           WHERE session_id = $1
             AND created_at >= $2"#,
    )
    .bind(session_id)
    .bind(not_before)
    .fetch_one(pool)
    .await?;

    let organization_name: Option<String> =
        sqlx::query_scalar("SELECT name FROM organizations WHERE id = $1")
            .bind(organization_id)
            .fetch_optional(pool)
            .await?;

    let subsidiary_choice_rows: Vec<(serde_json::Value, Option<String>)> = sqlx::query_as(
        r#"SELECT args, result
           FROM tool_calls
           WHERE session_id = $1
             AND created_at >= $2
             AND name = 'ask_human'
             AND status = 'finished'::toolcall_status
             AND args->>'input_type' = 'choice'
           ORDER BY created_at ASC, id ASC"#,
    )
    .bind(session_id)
    .bind(not_before)
    .fetch_all(pool)
    .await?;
    let subsidiaries_excluded = subsidiary_choice_rows
        .iter()
        .filter_map(|(args, result)| {
            subsidiary_scope_decision(
                args,
                result.as_deref(),
                organization_id,
                organization_name.as_deref(),
            )
        })
        .next_back()
        .unwrap_or(false);

    // Preserve lifecycle order and validate both payloads against the same
    // trusted root org. A skipped/error review or a review issued before the
    // proposal cannot satisfy the included-subsidiary branch.
    let unit_flow_rows: Vec<(String, serde_json::Value, Option<String>)> = sqlx::query_as(
        r#"SELECT name, args, result
           FROM tool_calls
           WHERE session_id = $1
             AND created_at >= $2
             AND status = 'finished'::toolcall_status
             AND (
               (name = 'manage_organizations' AND args->>'action' = 'propose_candidates')
               OR (name = 'ask_human' AND args->>'input_type' = 'unit_review')
             )
           ORDER BY created_at ASC, id ASC"#,
    )
    .bind(session_id)
    .bind(not_before)
    .fetch_all(pool)
    .await?;
    let (unit_candidates_proposed, unit_review_invoked) =
        unit_flow_for_org(&unit_flow_rows, organization_id);

    // Collect the result payloads of this session's create calls, then keep only
    // the org ids that a SUCCESSFUL create reported (parsed in Rust to avoid
    // fragile SQL casts over arbitrary tool result text).
    let create_results: Vec<(Option<String>,)> = sqlx::query_as(
        r#"SELECT result FROM tool_calls
           WHERE session_id = $1
             AND created_at >= $2
             AND name = 'manage_organizations'
             AND status = 'finished'::toolcall_status
             AND args->>'action' IN ('create', 'create_batch')"#,
    )
    .bind(session_id)
    .bind(not_before)
    .fetch_all(pool)
    .await?;

    let created_ids: Vec<Uuid> = create_results
        .iter()
        .flat_map(|(r,)| {
            r.as_deref()
                .map(org_ids_from_create_result)
                .unwrap_or_default()
        })
        .collect();

    // "查 organizations 表": a reported id only counts if the row truly exists.
    let organization_created = if created_ids.is_empty() {
        false
    } else {
        let existing: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM organizations WHERE id = ANY($1)")
                .bind(&created_ids)
                .fetch_one(pool)
                .await?;
        existing > 0
    };

    // Read EVERY completed review in this operation window. Looking only at the
    // latest row lets a model wash away an earlier human edit/rejection by
    // opening a second dialog and confirming the stale DB snapshot.
    let scope_review_results: Vec<Option<String>> =
        sqlx::query_scalar(scoping_scope_review_results_sql())
            .bind(session_id)
            .bind(not_before)
            .fetch_all(pool)
            .await?;

    Ok((
        total,
        unit_candidates_proposed,
        unit_review_invoked,
        subsidiaries_excluded,
        organization_created,
        scope_review_results,
    ))
}

fn scoping_scope_review_results_sql() -> &'static str {
    r#"SELECT result
       FROM tool_calls
       WHERE session_id = $1
         AND created_at >= $2
         AND name = 'ask_human'
         AND status = 'finished'::toolcall_status
         AND args->>'input_type' = 'scope_review'
       ORDER BY created_at ASC, id ASC"#
}

pub async fn list_by_name(pool: &PgPool, name: &str, limit: i64) -> Result<Vec<ToolCall>> {
    let rows = sqlx::query_as::<_, ToolCall>(
        "SELECT * FROM tool_calls WHERE name = $1 ORDER BY created_at DESC LIMIT $2",
    )
    .bind(name)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn update_status(
    pool: &PgPool,
    id: Uuid,
    status: ToolcallStatus,
    result: Option<&str>,
    duration_ms: Option<i32>,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE tool_calls
           SET status = $1, result = COALESCE($2, result),
               duration_ms = COALESCE($3, duration_ms), updated_at = NOW()
           WHERE id = $4"#,
    )
    .bind(status)
    .bind(result)
    .bind(duration_ms)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Aggregate stats for analytics
pub async fn stats_by_name(pool: &PgPool, session_id: Option<Uuid>) -> Result<Vec<ToolCallStats>> {
    let rows = if let Some(sid) = session_id {
        sqlx::query_as::<_, ToolCallStats>(
            r#"SELECT name, COUNT(*) as total_count,
                      COALESCE(SUM(duration_ms), 0) as total_duration_ms,
                      COALESCE(AVG(duration_ms), 0) as avg_duration_ms
               FROM tool_calls WHERE session_id = $1
               GROUP BY name ORDER BY total_count DESC"#,
        )
        .bind(sid)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, ToolCallStats>(
            r#"SELECT name, COUNT(*) as total_count,
                      COALESCE(SUM(duration_ms), 0) as total_duration_ms,
                      COALESCE(AVG(duration_ms), 0) as avg_duration_ms
               FROM tool_calls
               GROUP BY name ORDER BY total_count DESC"#,
        )
        .fetch_all(pool)
        .await?
    };
    Ok(rows)
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct ToolCallStats {
    pub name: String,
    pub total_count: i64,
    pub total_duration_ms: i64,
    pub avg_duration_ms: f64,
}

#[cfg(test)]
mod tests {
    use super::{
        org_ids_from_create_result, require_exactly_one_finish, scoping_scope_review_results_sql,
        subsidiary_scope_decision, unit_flow_for_org, RECORD_TRACKED_FINISH_SQL,
        RECORD_TRACKED_START_SQL,
    };
    use crate::models::NewSession;
    use crate::repo::sessions;
    use crate::{DbConfig, GolishDb};
    use serial_test::serial;
    use uuid::Uuid;

    fn reserve_local_port() -> u16 {
        std::net::TcpListener::bind(("127.0.0.1", 0))
            .expect("reserve local postgres port")
            .local_addr()
            .expect("read reserved postgres port")
            .port()
    }

    #[test]
    fn runtime_tool_tracking_start_returns_db_id_and_writes_the_complete_identity() {
        for column in [
            "operation_id",
            "stage_execution_id",
            "stage_run_unit_id",
            "worker_run_id",
            "organization_id",
            "attempt_epoch",
            "lease_token",
        ] {
            assert!(
                RECORD_TRACKED_START_SQL.contains(column),
                "missing {column}"
            );
        }
        assert!(RECORD_TRACKED_START_SQL.contains("task_id"));
        assert!(RECORD_TRACKED_START_SQL.contains("subtask_id"));
        assert!(RECORD_TRACKED_START_SQL.contains("RETURNING id"));
        assert!(!RECORD_TRACKED_START_SQL.contains("ON CONFLICT DO NOTHING"));
    }

    #[test]
    fn runtime_tool_tracking_finish_is_a_record_and_session_scoped_cas() {
        assert!(RECORD_TRACKED_FINISH_SQL.contains("WHERE id = $4"));
        assert!(RECORD_TRACKED_FINISH_SQL.contains("session_id = $5"));
        assert!(RECORD_TRACKED_FINISH_SQL.contains("status IN ('received', 'running')"));
        assert!(require_exactly_one_finish(1, Uuid::new_v4(), Uuid::new_v4()).is_ok());
        assert!(require_exactly_one_finish(0, Uuid::new_v4(), Uuid::new_v4()).is_err());
    }

    #[tokio::test]
    #[serial]
    async fn runtime_tool_tracking_record_id_finish_cas_executes_on_postgres() {
        let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
        let config = DbConfig {
            pg_data_dir: data_dir.path().join("pgdata"),
            port: reserve_local_port(),
            database: format!("runtime_tool_tracking_{}", Uuid::new_v4().simple()),
            ..DbConfig::default()
        };
        let mut db = GolishDb::start(config)
            .await
            .expect("start migrated embedded postgres");
        let session = sessions::create(
            db.pool(),
            NewSession {
                title: Some("runtime tool tracking".to_string()),
                workspace_path: None,
                workspace_label: None,
                model: None,
                provider: None,
                project_path: None,
            },
        )
        .await
        .expect("create tracking session");

        let record_id = super::record_tracked_start(
            db.pool(),
            "tool-request-id",
            session.id,
            None,
            None,
            "query_target_data",
            &serde_json::json!({"section": "targets"}),
            None,
        )
        .await
        .expect("insert durable tool-call start");
        let persisted_id: Uuid =
            sqlx::query_scalar("SELECT id FROM tool_calls WHERE call_id = $1 AND session_id = $2")
                .bind("tool-request-id")
                .bind(session.id)
                .fetch_one(db.pool())
                .await
                .expect("read persisted start id");
        assert_eq!(persisted_id, record_id);

        assert!(super::record_tracked_finish(
            db.pool(),
            record_id,
            Uuid::new_v4(),
            "finished",
            "wrong session",
            1,
        )
        .await
        .is_err());
        super::record_tracked_finish(db.pool(), record_id, session.id, "finished", "done", 2)
            .await
            .expect("finish exact record and start session");
        assert!(super::record_tracked_finish(
            db.pool(),
            record_id,
            session.id,
            "failed",
            "late duplicate",
            3,
        )
        .await
        .is_err());
        let persisted: (String, Option<String>, Option<i32>) = sqlx::query_as(
            "SELECT status::text, result, duration_ms FROM tool_calls WHERE id = $1",
        )
        .bind(record_id)
        .fetch_one(db.pool())
        .await
        .expect("read terminal tool call");
        assert_eq!(
            persisted,
            ("finished".to_string(), Some("done".to_string()), Some(2))
        );

        db.stop().await;
    }

    #[test]
    fn persisted_parent_only_choice_is_a_deterministic_subsidiary_exclusion() {
        let organization_id = Uuid::new_v4();
        let args = serde_json::json!({
            "input_type": "choice",
            "context": serde_json::json!({
                "decision": "subsidiary_scope",
                "organization_id": organization_id
            }).to_string(),
            "question": "是否纳入子公司？",
            "options": ["不纳入子公司（仅测试母公司本身）", "纳入：≥51% 控股子公司"]
        });
        assert_eq!(
            subsidiary_scope_decision(
                &args,
                Some(r#"{"response":"不纳入子公司（仅测试母公司本身）","skipped":false}"#),
                organization_id,
                Some("杭州默安科技有限公司")
            ),
            Some(true)
        );
        assert_eq!(
            subsidiary_scope_decision(
                &args,
                Some(r#"{"response":"纳入：≥51% 控股子公司","skipped":false}"#),
                organization_id,
                Some("杭州默安科技有限公司")
            ),
            Some(false)
        );
        assert_eq!(
            subsidiary_scope_decision(
                &args,
                Some(r#"{"response":"root_only","skipped":false}"#),
                organization_id,
                Some("杭州默安科技有限公司")
            ),
            Some(true),
            "the canonical root_only enum emitted by the Scoping prompt must be accepted"
        );
        assert_eq!(
            subsidiary_scope_decision(
                &args,
                Some(r#"{"response":"include_51","skipped":false}"#),
                organization_id,
                Some("杭州默安科技有限公司")
            ),
            Some(false),
            "canonical include enums must select the reviewed-unit branch"
        );
        assert_eq!(
            subsidiary_scope_decision(
                &args,
                Some(r#"{"response":"不纳入子公司","skipped":false}"#),
                Uuid::new_v4(),
                Some("另一家公司")
            ),
            None,
            "a choice bound to another root must not satisfy this engagement"
        );
        assert_eq!(
            subsidiary_scope_decision(
                &args,
                Some(r#"{"skipped":true}"#),
                organization_id,
                Some("杭州默安科技有限公司")
            ),
            None,
            "skipping the choice is not an explicit parent-only decision"
        );

        let entity_args = serde_json::json!({
            "input_type": "choice",
            "context": "Company lookup result confirmation",
            "question": "这是目标公司吗？",
            "options": ["杭州默安科技有限公司"]
        });
        assert_eq!(
            subsidiary_scope_decision(
                &entity_args,
                Some(r#"{"response":"杭州默安科技有限公司","skipped":false}"#),
                organization_id,
                Some("杭州默安科技有限公司")
            ),
            None
        );

        let legacy_args = serde_json::json!({
            "input_type": "choice",
            "context": "Subsidiary scope decision",
            "question": "杭州默安科技有限公司是否纳入子公司？",
            "options": ["不纳入子公司（仅测试母公司本身）", "纳入子公司"]
        });
        assert_eq!(
            subsidiary_scope_decision(
                &legacy_args,
                Some(r#"{"response":"不纳入子公司","skipped":false}"#),
                organization_id,
                Some("杭州默安科技有限公司")
            ),
            Some(true),
            "an in-flight legacy choice remains usable only when it names the exact root"
        );
        assert_eq!(
            subsidiary_scope_decision(
                &legacy_args,
                Some(r#"{"response":"不纳入子公司","skipped":false}"#),
                organization_id,
                Some("另一家公司")
            ),
            None
        );
    }

    #[test]
    fn included_subsidiary_flow_requires_same_org_success_and_order() {
        let organization_id = Uuid::new_v4();
        let proposal = (
            "manage_organizations".to_string(),
            serde_json::json!({
                "action": "propose_candidates",
                "organization_id": organization_id
            }),
            Some(
                serde_json::json!({
                    "action": "propose_candidates",
                    "organization_id": organization_id,
                    "recorded": 0
                })
                .to_string(),
            ),
        );
        let review = (
            "ask_human".to_string(),
            serde_json::json!({
                "input_type": "unit_review",
                "context": serde_json::json!({"organization_id": organization_id}).to_string()
            }),
            Some(r#"{"response":"[]","skipped":false}"#.to_string()),
        );

        assert_eq!(
            unit_flow_for_org(&[proposal.clone(), review.clone()], organization_id),
            (true, true)
        );
        let current_protocol_review = (
            "ask_human".to_string(),
            serde_json::json!({
                "input_type": "unit_review",
                "context": serde_json::json!({"organization_id": organization_id}).to_string()
            }),
            Some(r#"{"response":"{\"rows\":[]}","skipped":false}"#.to_string()),
        );
        assert_eq!(
            unit_flow_for_org(
                &[proposal.clone(), current_protocol_review],
                organization_id
            ),
            (true, true),
            "the current AskHuman unit-review protocol wraps rows in an object"
        );
        assert_eq!(
            unit_flow_for_org(&[review.clone(), proposal.clone()], organization_id),
            (true, false),
            "a review before candidate proposal cannot approve the flow"
        );

        let mut skipped_review = review.clone();
        skipped_review.2 = Some(r#"{"skipped":true}"#.to_string());
        assert_eq!(
            unit_flow_for_org(&[proposal.clone(), skipped_review], organization_id),
            (true, false)
        );
        assert_eq!(
            unit_flow_for_org(&[proposal, review], Uuid::new_v4()),
            (false, false),
            "another organization's proposal and review cannot satisfy this root"
        );
    }

    #[test]
    fn scoping_review_query_preserves_every_attempt_in_order() {
        let sql = scoping_scope_review_results_sql();
        assert!(sql.contains("created_at >= $2"));
        assert!(sql.contains("ORDER BY created_at ASC, id ASC"));
        assert!(!sql.contains("LIMIT 1"));
    }

    /// A successful `manage_organizations(action="create")` result carries a real
    /// `id` ⇒ that's the org the gate must confirm exists.
    #[test]
    fn create_result_yields_id_on_success() {
        let id = uuid::Uuid::new_v4();
        let r = format!(r#"{{"action":"create","id":"{id}","name":"ACME Corp"}}"#);
        assert_eq!(org_ids_from_create_result(&r), vec![id]);
    }

    /// `create_batch` is the recommended way to record multiple confirmed
    /// subsidiaries; both newly-created and already-existing rows count as real
    /// organization records for the scoping audit.
    #[test]
    fn create_batch_result_yields_created_and_existing_ids() {
        let created = uuid::Uuid::new_v4();
        let existing = uuid::Uuid::new_v4();
        let r = format!(
            r#"{{
                "action":"create_batch",
                "created":[{{"id":"{created}","name":"New Unit"}}],
                "existing":[{{"id":"{existing}","name":"Existing Unit"}}],
                "failed":[]
            }}"#
        );
        assert_eq!(org_ids_from_create_result(&r), vec![created, existing]);
    }

    /// The tool swallows DB errors (e.g. duplicate root-org name) into an
    /// `Ok({"error":...})` body. Such a result reports NO created org, so a mere
    /// failed attempt must not satisfy the red_team scoping gate.
    #[test]
    fn create_result_is_none_on_swallowed_error() {
        let r =
            r#"{"error":"duplicate key value violates unique constraint \"uq_orgs_root_name\""}"#;
        assert!(org_ids_from_create_result(r).is_empty());
    }

    /// Defensive: missing id, a non-uuid id, and non-JSON garbage all yield None
    /// rather than a false positive.
    #[test]
    fn create_result_is_none_on_missing_or_garbage_id() {
        assert!(org_ids_from_create_result(r#"{"action":"create"}"#).is_empty());
        assert!(org_ids_from_create_result(r#"{"action":"create","id":"not-a-uuid"}"#).is_empty());
        assert!(org_ids_from_create_result("not json at all").is_empty());
        // An `id` present alongside an `error` still counts as a failure.
        let id = uuid::Uuid::new_v4();
        let r = format!(r#"{{"id":"{id}","error":"boom"}}"#);
        assert!(org_ids_from_create_result(&r).is_empty());
    }
}
