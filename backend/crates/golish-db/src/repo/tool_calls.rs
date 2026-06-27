use crate::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{NewToolCall, ToolCall, ToolcallStatus};

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

/// For the red_team scoping gate: cross-verify (against real recorded tool calls
/// AND the resulting DB state) whether this session actually performed the
/// unit-candidate review flow rather than just asserting a claim or merely
/// *attempting* a create.
///
/// Returns `(total_calls, unit_review_invoked, organization_created)`:
/// - `total_calls` lets the caller FAIL OPEN when `0` (tracking disabled / no
///   calls recorded), never blocking on infra absence.
/// - `unit_review_invoked`: a `ask_human(input_type="unit_review")` call exists.
/// - `organization_created`: a `manage_organizations(action="create"/"create_batch")`
///   call this session reported a real org id for, AND that org row actually
///   exists in `organizations` now. A swallowed duplicate-key failure (no id in
///   the result) or a since-deleted row ⇒ `false`, so a failed create can no
///   longer pass the gate (AGENTS.md I7/I8: "attempted" ≠ "actually recorded").
pub async fn scoping_actions_for_session(
    pool: &PgPool,
    session_id: Uuid,
) -> Result<(i64, bool, bool)> {
    let (total, unit_review_invoked): (i64, bool) = sqlx::query_as(
        r#"SELECT
             COUNT(*) AS total,
             COALESCE(BOOL_OR(name = 'ask_human' AND args->>'input_type' = 'unit_review'), false) AS unit_review
           FROM tool_calls
           WHERE session_id = $1"#,
    )
    .bind(session_id)
    .fetch_one(pool)
    .await?;

    // Collect the result payloads of this session's create calls, then keep only
    // the org ids that a SUCCESSFUL create reported (parsed in Rust to avoid
    // fragile SQL casts over arbitrary tool result text).
    let create_results: Vec<(Option<String>,)> = sqlx::query_as(
        r#"SELECT result FROM tool_calls
           WHERE session_id = $1
             AND name = 'manage_organizations'
             AND args->>'action' IN ('create', 'create_batch')"#,
    )
    .bind(session_id)
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

    Ok((total, unit_review_invoked, organization_created))
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
    use super::org_ids_from_create_result;

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
