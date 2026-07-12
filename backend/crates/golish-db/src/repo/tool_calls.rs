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

fn subsidiary_scope_decision(
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
        "不纳入子公司",
        "不包含子公司",
        "仅母公司",
        "仅测试母公司",
        "只测试母公司",
        "no subsidiaries",
        "exclude subsidiaries",
        "parent company only",
        "root only",
    ]
    .iter()
    .any(|marker| response.contains(marker))
    {
        return Some(true);
    }
    if [
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
        .and_then(|response| response.as_array().map(|_| ()))
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
        org_ids_from_create_result, scoping_scope_review_results_sql, subsidiary_scope_decision,
        unit_flow_for_org,
    };
    use uuid::Uuid;

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
