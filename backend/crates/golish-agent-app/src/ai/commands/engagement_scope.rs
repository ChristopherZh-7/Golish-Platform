//! Engagement worker-scope commands (设计 2026-06-13-engagement-scoping-fanout
//! §6.3, Phase B).
//!
//! The fan-out pool spawns each worker as a real chat session, then pins the
//! session to one org + a stage slice via these commands BEFORE seeding the
//! task prompt. The Task-mode router (`core/chat.rs::execute_task_mode`) reads
//! the pinned scope and applies the same hard constraints the headless
//! `--stage-run` CLI uses (`set_harness_org_id` / `set_subsidiary_scope` /
//! `set_stage_allowlist` / `run_stage`).

use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use golish_agent_bridge::EngagementWorkerScope;
use golish_agent_kit::harness::StageKind;

use crate::error::GolishError;
use crate::state::AgentState;

use super::ai_session_not_initialized_error;

/// Wire DTO for the worker scope (camelCase to match the frontend API layer).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngagementWorkerScopeDto {
    pub org_id: String,
    pub from: Option<String>,
    pub to: String,
    pub include_subsidiaries: bool,
    pub subsidiary_threshold_pct: u8,
}

impl From<&EngagementWorkerScope> for EngagementWorkerScopeDto {
    fn from(s: &EngagementWorkerScope) -> Self {
        Self {
            org_id: s.org_id.to_string(),
            from: s.from.map(|k| k.as_str().to_string()),
            to: s.to.as_str().to_string(),
            include_subsidiaries: s.include_subsidiaries,
            subsidiary_threshold_pct: s.subsidiary_threshold_pct,
        }
    }
}

/// Parse + validate the wire DTO into the bridge scope. Free function so the
/// validation rules are unit-testable without Tauri state.
fn parse_scope(dto: &EngagementWorkerScopeDto) -> Result<EngagementWorkerScope, String> {
    let org_id: Uuid = dto
        .org_id
        .parse()
        .map_err(|_| format!("invalid org_id (not a UUID): {}", dto.org_id))?;
    let to = StageKind::try_parse(&dto.to).ok_or_else(|| format!("unknown stage: {}", dto.to))?;
    let from = match dto.from.as_deref() {
        Some(f) => Some(StageKind::try_parse(f).ok_or_else(|| format!("unknown stage: {f}"))?),
        None => None,
    };
    Ok(EngagementWorkerScope {
        org_id,
        from,
        to,
        include_subsidiaries: dto.include_subsidiaries,
        subsidiary_threshold_pct: dto.subsidiary_threshold_pct,
    })
}

/// Pin an engagement worker scope onto a session. Call after `init_ai_session`
/// (+ `set_execution_mode` to a harness profile) and BEFORE the worker prompt.
#[tauri::command]
pub async fn engagement_set_worker_scope(
    session_id: String,
    scope: EngagementWorkerScopeDto,
    state: State<'_, AgentState>,
) -> Result<(), GolishError> {
    let parsed = parse_scope(&scope).map_err(GolishError::Validation)?;

    // The org must at least exist; the run-time org axis (coverage gate /
    // tool-level IDOR guards) enforces project ownership per call. Read via
    // the SHARED engagement_truth repo (ownership guard: agent-app must not
    // couple to the recon-owned organizations repo directly).
    let pool = state.db_pool.clone();
    let exists = golish_db::repo::engagement_truth::get_org(&pool, parsed.org_id)
        .await
        .map_err(|e| GolishError::Internal(e.to_string()))?
        .is_some();
    if !exists {
        return Err(GolishError::NotFound(format!(
            "organization {}",
            parsed.org_id
        )));
    }

    let bridges = state.ai_state.bridges.read().await;
    let bridge = bridges
        .get(&session_id)
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    bridge
        .set_engagement_worker_scope(Some(parsed.clone()))
        .await;
    tracing::info!(
        target: "engagement::worker",
        session_id = %session_id,
        org_id = %parsed.org_id,
        from = parsed.from.map(|k| k.as_str()).unwrap_or("(entry)"),
        to = parsed.to.as_str(),
        include_subsidiaries = parsed.include_subsidiaries,
        "worker scope pinned"
    );
    Ok(())
}

/// Clear the worker scope (reverts the session to a normal chat/task session).
#[tauri::command]
pub async fn engagement_clear_worker_scope(
    session_id: String,
    state: State<'_, AgentState>,
) -> Result<(), GolishError> {
    let bridges = state.ai_state.bridges.read().await;
    let bridge = bridges
        .get(&session_id)
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    bridge.set_engagement_worker_scope(None).await;
    Ok(())
}

/// Read back the worker scope pinned on a session (debug / pool resync).
#[tauri::command]
pub async fn engagement_get_worker_scope(
    session_id: String,
    state: State<'_, AgentState>,
) -> Result<Option<EngagementWorkerScopeDto>, GolishError> {
    let bridges = state.ai_state.bridges.read().await;
    let bridge = bridges
        .get(&session_id)
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    Ok(bridge
        .get_engagement_worker_scope()
        .await
        .as_ref()
        .map(EngagementWorkerScopeDto::from))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dto(org: &str, from: Option<&str>, to: &str) -> EngagementWorkerScopeDto {
        EngagementWorkerScopeDto {
            org_id: org.into(),
            from: from.map(|s| s.to_string()),
            to: to.into(),
            include_subsidiaries: true,
            subsidiary_threshold_pct: 51,
        }
    }

    #[test]
    fn parse_scope_accepts_recon_family_slice() {
        let id = Uuid::new_v4().to_string();
        let s = parse_scope(&dto(&id, Some("target_intel"), "enumeration")).expect("valid");
        assert_eq!(s.from, Some(StageKind::TargetIntel));
        assert_eq!(s.to, StageKind::Enumeration);
        assert!(s.include_subsidiaries);
    }

    #[test]
    fn parse_scope_rejects_bad_uuid_and_stage() {
        assert!(parse_scope(&dto("not-a-uuid", None, "enumeration"))
            .unwrap_err()
            .contains("invalid org_id"));
        let id = Uuid::new_v4().to_string();
        assert!(parse_scope(&dto(&id, None, "no_such_stage"))
            .unwrap_err()
            .contains("unknown stage"));
        assert!(parse_scope(&dto(&id, Some("bogus"), "enumeration"))
            .unwrap_err()
            .contains("unknown stage"));
    }
}
