//! Sub-agent dispatch commands (P0-4).
//!
//! Exposes a single Tauri command that lists every dispatch row whose
//! `status = 'running'` for a given session. The frontend can call this
//! on session activation to detect mid-flight sub-agent invocations
//! that were left over when a previous app instance died.
//!
//! See docs/design/2026-05-17-dispatch-resume.md for the bigger picture
//! and the cleanup roadmap (out of P0-4 scope).

use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use crate::error::GolishError;
use crate::state::AppState;

/// Minimal payload returned to the frontend for each running dispatch.
///
/// Keeps `args` as raw `serde_json::Value` so the UI can decide how to
/// surface them (full details vs. summary), but trims everything we
/// don't currently use (result / error_message / finished_at — those
/// are null while running anyway).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunningSubAgentDispatch {
    pub id: String,
    pub parent_dispatch_id: Option<String>,
    pub agent_id: String,
    pub tool_call_id: Option<String>,
    pub depth: i32,
    pub args: serde_json::Value,
    pub started_at: String,
}

/// Cancel-after threshold (seconds): rows older than this stuck in
/// `running` are reaped on the next list query.
///
/// 24 hours is generous enough that long real work isn't auto-cancelled
/// (a single sub-agent rarely runs that long), yet short enough that a
/// dead row from yesterday's crash gets cleaned up quickly.
const STALE_RUNNING_CANCEL_AFTER_SECS: i64 = 60 * 60 * 24;

#[tauri::command]
pub async fn list_running_sub_agent_dispatches(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<RunningSubAgentDispatch>, GolishError> {
    let sid = match Uuid::parse_str(&session_id) {
        Ok(id) => id,
        Err(e) => {
            return Err(GolishError::Internal(format!(
                "invalid session_id '{}': {}",
                session_id, e
            )))
        }
    };

    // P0-4.c: before listing, reap dispatches that are still tagged
    // `running` but started > STALE_RUNNING_CANCEL_AFTER_SECS ago. These
    // are leftovers from a previous crash where record_finish never
    // fired. Best-effort: failures here log a warn and don't block the
    // list query.
    match golish_db::repo::sub_agent_dispatches::mark_session_running_as_cancelled(
        &state.db_pool,
        sid,
        STALE_RUNNING_CANCEL_AFTER_SECS,
    )
    .await
    {
        Ok(0) => {}
        Ok(n) => tracing::info!(
            session_id = %session_id,
            cancelled = n,
            stale_after_secs = STALE_RUNNING_CANCEL_AFTER_SECS,
            "[dispatch-track] reaped stale running dispatches"
        ),
        Err(e) => tracing::warn!(
            session_id = %session_id,
            error = %e,
            "[dispatch-track] stale-cleanup failed (continuing with list)"
        ),
    }

    // Use the repo function directly to avoid an extra trait round-trip.
    let rows = match golish_db::repo::sub_agent_dispatches::list_running(&state.db_pool, sid).await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "list_running_sub_agent_dispatches: DB query failed, returning empty",
            );
            return Ok(vec![]);
        }
    };

    Ok(rows
        .into_iter()
        .map(|r| RunningSubAgentDispatch {
            id: r.id.to_string(),
            parent_dispatch_id: r.parent_dispatch_id.map(|p| p.to_string()),
            agent_id: r.agent_id,
            tool_call_id: r.tool_call_id,
            depth: r.depth,
            args: r.args,
            started_at: r.started_at.to_rfc3339(),
        })
        .collect())
}
