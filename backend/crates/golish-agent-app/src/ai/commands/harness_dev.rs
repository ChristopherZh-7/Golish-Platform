//! Developer-only harness checkpoint controls.
//!
//! These commands are intentionally narrow: they adjust resumability checkpoints
//! in `operation_state` so local stage testing can restart from a chosen stage
//! without deleting evidence, assets, or target facts.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::State;
use uuid::Uuid;

use crate::error::GolishError;
use crate::state::AgentState;
use golish_agent_kit::harness::operation_flow::OperationFlowState;
use golish_agent_kit::harness::{
    base_operation_graph, load_embedded_profile, AllowedDag, StageKind,
};
use golish_agent_kit::task_orchestrator::agent_run_checkpoint::{
    agent_run_from_state_blob, state_blob_without_agent_run, AgentRunCheckpoint,
};

const RESOLVE_CHAT_SESSION_UUID_SQL: &str =
    "SELECT id FROM sessions WHERE chat_session_key = $1 ORDER BY updated_at DESC LIMIT 1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HarnessDevResetMode {
    ClearRepair,
    RestartStage,
    RestartFromStage,
    /// Full reset: like `RestartFromStage` (cursor + checkpoint rewind to the
    /// selected stage) AND deletes the discovered facts produced by the selected
    /// stage and its DAG descendants, in the engagement org subtree, so re-testing
    /// the stage starts from a clean slate.
    RestartFromStagePurge,
}

impl HarnessDevResetMode {
    fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "clear_repair" => Some(Self::ClearRepair),
            "restart_stage" => Some(Self::RestartStage),
            "restart_from_stage" => Some(Self::RestartFromStage),
            "restart_from_stage_purge" => Some(Self::RestartFromStagePurge),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::ClearRepair => "clear_repair",
            Self::RestartStage => "restart_stage",
            Self::RestartFromStage => "restart_from_stage",
            Self::RestartFromStagePurge => "restart_from_stage_purge",
        }
    }

    fn resets_stage_cursor(self) -> bool {
        matches!(
            self,
            Self::RestartStage | Self::RestartFromStage | Self::RestartFromStagePurge
        )
    }

    /// Whether this mode deletes discovered facts (not just the resume checkpoint).
    fn purges_facts(self) -> bool {
        matches!(self, Self::RestartFromStagePurge)
    }
}

/// Data domains that map to harness stages; each is purged once even if several
/// affected stages share it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum FactDomain {
    TargetIntel,
    Eas,
    Enumeration,
    Vuln,
}

/// Which fact domain a stage produces (None = stage has no purgeable facts).
fn fact_domain(stage: StageKind) -> Option<FactDomain> {
    Some(match stage {
        StageKind::TargetIntel => FactDomain::TargetIntel,
        StageKind::ExternalAttackSurface => FactDomain::Eas,
        StageKind::Enumeration => FactDomain::Enumeration,
        StageKind::VulnTriage
        | StageKind::Verification
        | StageKind::AccessValidation
        | StageKind::InternalDiscovery
        | StageKind::ObjectivePathing
        | StageKind::ObjectiveSimulation => FactDomain::Vuln,
        StageKind::Scoping | StageKind::Reporting | StageKind::Cleanup => return None,
    })
}

/// The `targets.status` floor a target should hold at the *start* of `stage`
/// (i.e. after its predecessor). None = no rollback for this stage.
fn target_status_floor(stage: StageKind) -> Option<&'static str> {
    Some(match stage {
        StageKind::TargetIntel => "new",
        StageKind::ExternalAttackSurface => "passive",
        StageKind::Enumeration => "active",
        StageKind::VulnTriage => "enumerated",
        StageKind::Verification => "vuln_scan",
        _ => return None,
    })
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct StateBlobResetStats {
    cleared_agent_run_checkpoints: usize,
    cleared_stage_run_workers: usize,
    reset_graph_flow: bool,
    trimmed_graph_flow_visited: usize,
    trimmed_graph_flow_applied: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessDevStageCheckpointResetResult {
    pub operation_id: String,
    pub stage: String,
    pub mode: String,
    pub affected_stages: Vec<String>,
    pub cleared_agent_run_checkpoints: usize,
    pub cleared_stage_run_workers: usize,
    pub reset_graph_flow: bool,
    pub trimmed_graph_flow_visited: usize,
    pub trimmed_graph_flow_applied: usize,
    pub refreshed_stage_cursor: bool,
    pub previous_stage: String,
    pub current_stage: String,
    pub message: String,
    /// True when discovered facts were deleted (not just the resume checkpoint).
    pub purged_facts: bool,
    /// Number of engagement-org-subtree orgs the purge was scoped to.
    pub purge_scope_org_count: usize,
    /// Per-table affected-row counts (None unless `purged_facts`).
    pub purge_counts: Option<golish_db::repo::stage_purge::StagePurgeCounts>,
    /// Non-fatal note (e.g. purge skipped because the operation has no engagement
    /// org binding).
    pub purge_note: Option<String>,
}

#[tauri::command]
pub async fn harness_dev_reset_stage_checkpoint(
    state: State<'_, AgentState>,
    operation_id: Option<String>,
    session_id: Option<String>,
    organization_id: Option<String>,
    stage: String,
    mode: String,
) -> Result<HarnessDevStageCheckpointResetResult, GolishError> {
    ensure_dev_checkpoint_reset_allowed()?;
    let stage_kind = StageKind::try_parse(&stage)
        .ok_or_else(|| GolishError::Validation(format!("unknown stage: {stage}")))?;
    let mode = HarnessDevResetMode::parse(&mode)
        .ok_or_else(|| GolishError::Validation(format!("unknown reset mode: {mode}")))?;
    let operation_id = resolve_operation_id(
        &state.db_pool,
        operation_id.as_deref(),
        session_id.as_deref(),
    )
    .await?;
    let Some(row) = golish_db::repo::operation_state::get(&state.db_pool, operation_id).await?
    else {
        return Err(GolishError::NotFound(format!(
            "operation_state not found: {operation_id}"
        )));
    };
    let dag = projected_dag_for_profile(&row.profile)?;
    if !dag.contains(stage_kind) {
        return Err(GolishError::Validation(format!(
            "stage '{}' is not enabled for profile '{}'",
            stage_kind.as_str(),
            row.profile
        )));
    }

    let affected = affected_stages(mode, stage_kind, &dag);
    let affected_names = dag_ordered_stage_names(&dag, &affected);
    let (next_blob, stats) = reset_state_blob(
        row.state_blob.clone(),
        stage_kind,
        &affected,
        organization_id.as_deref(),
        mode,
    );
    golish_db::repo::operation_state::write_state_blob(&state.db_pool, operation_id, next_blob)
        .await?;

    if mode.resets_stage_cursor() {
        golish_db::repo::operation_state::advance_stage(
            &state.db_pool,
            operation_id,
            stage_kind.as_str(),
        )
        .await?;
    }

    let (purged_facts, purge_scope_org_count, purge_counts, purge_note) = if mode.purges_facts() {
        let (counts, org_count, note) = purge_stage_facts(
            &state.db_pool,
            operation_id,
            row.engagement_org_id,
            &affected,
            stage_kind,
        )
        .await?;
        (true, org_count, Some(counts), note)
    } else {
        (false, 0, None, None)
    };

    let current_stage = golish_db::repo::operation_state::get(&state.db_pool, operation_id)
        .await?
        .map(|row| row.current_stage)
        .unwrap_or_else(|| stage_kind.as_str().to_string());

    Ok(HarnessDevStageCheckpointResetResult {
        operation_id: operation_id.to_string(),
        stage: stage_kind.as_str().to_string(),
        mode: mode.as_str().to_string(),
        affected_stages: affected_names,
        cleared_agent_run_checkpoints: stats.cleared_agent_run_checkpoints,
        cleared_stage_run_workers: stats.cleared_stage_run_workers,
        reset_graph_flow: stats.reset_graph_flow,
        trimmed_graph_flow_visited: stats.trimmed_graph_flow_visited,
        trimmed_graph_flow_applied: stats.trimmed_graph_flow_applied,
        refreshed_stage_cursor: mode.resets_stage_cursor(),
        previous_stage: row.current_stage,
        current_stage,
        message: match mode {
            HarnessDevResetMode::ClearRepair => "cleared matching repair checkpoint".to_string(),
            HarnessDevResetMode::RestartStage => "reset selected stage checkpoint".to_string(),
            HarnessDevResetMode::RestartFromStage => {
                "reset graph-flow checkpoint from selected stage".to_string()
            }
            HarnessDevResetMode::RestartFromStagePurge => {
                "reset checkpoint and purged discovered facts from selected stage".to_string()
            }
        },
        purged_facts,
        purge_scope_org_count,
        purge_counts,
        purge_note,
    })
}

/// Delete the discovered facts produced by `affected` stages (selected stage +
/// DAG descendants), scoped to the operation's engagement org subtree, plus the
/// per-stage completion/wave ledgers, and roll `targets.status` back to the floor
/// of `selected_stage`. Returns the counts, the subtree org count, and an optional
/// note (e.g. when the operation has no engagement org binding to scope by).
async fn purge_stage_facts(
    pool: &sqlx::PgPool,
    operation_id: Uuid,
    engagement_org_id: Option<Uuid>,
    affected: &HashSet<StageKind>,
    selected_stage: StageKind,
) -> Result<
    (
        golish_db::repo::stage_purge::StagePurgeCounts,
        usize,
        Option<String>,
    ),
    GolishError,
> {
    use golish_db::repo::stage_purge;

    let mut counts = stage_purge::StagePurgeCounts::default();
    let Some(root_org) = engagement_org_id else {
        return Ok((
            counts,
            0,
            Some(
                "operation has no engagement org binding; reset the checkpoint only (no facts purged)"
                    .to_string(),
            ),
        ));
    };

    let org_ids = golish_db::repo::organizations::subtree_ids(pool, root_org).await?;
    let project_path = golish_db::repo::organizations::get_one(pool, root_org)
        .await?
        .map(|org| org.project_path);
    let project_path = project_path.as_deref();

    let stage_names: Vec<String> = affected.iter().map(|s| s.as_str().to_string()).collect();

    let domains: HashSet<FactDomain> = affected.iter().copied().filter_map(fact_domain).collect();
    for domain in domains {
        match domain {
            FactDomain::TargetIntel => {
                stage_purge::purge_target_intel_domain(pool, &org_ids, &mut counts).await?
            }
            FactDomain::Eas => {
                stage_purge::purge_eas_domain(pool, &org_ids, project_path, &mut counts).await?
            }
            FactDomain::Enumeration => {
                stage_purge::purge_enumeration_domain(pool, &org_ids, &mut counts).await?
            }
            FactDomain::Vuln => {
                stage_purge::purge_vuln_domain(pool, &org_ids, project_path, &mut counts).await?
            }
        }
    }

    counts.org_stage_completions +=
        stage_purge::delete_org_stage_completions(pool, &org_ids, &stage_names).await?;
    counts.stage_asset_waves +=
        stage_purge::delete_stage_asset_waves(pool, operation_id, &org_ids, &stage_names).await?;

    if let Some(floor) = target_status_floor(selected_stage) {
        counts.target_status_rolled_back +=
            stage_purge::rollback_target_status(pool, &org_ids, floor).await?;
    }

    Ok((counts, org_ids.len(), None))
}

fn ensure_dev_checkpoint_reset_allowed() -> Result<(), GolishError> {
    if cfg!(debug_assertions) || env_flag_enabled("GOLISH_ENABLE_DEV_STAGE_RESET") {
        return Ok(());
    }
    Err(GolishError::Validation(
        "harness dev checkpoint reset is disabled in this build".to_string(),
    ))
}

fn env_flag_enabled(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            matches!(
                value.trim(),
                "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON"
            )
        })
        .unwrap_or(false)
}

async fn resolve_operation_id(
    pool: &sqlx::PgPool,
    operation_id: Option<&str>,
    session_id: Option<&str>,
) -> Result<Uuid, GolishError> {
    if let Some(operation_id) = operation_id.filter(|s| !s.trim().is_empty()) {
        return Uuid::parse_str(operation_id)
            .map_err(|e| GolishError::Validation(format!("invalid operation_id: {e}")));
    }

    let Some(session_id) = session_id.filter(|s| !s.trim().is_empty()) else {
        return Err(GolishError::Validation(
            "operation_id or session_id is required".to_string(),
        ));
    };
    let session_uuid = match Uuid::parse_str(session_id) {
        Ok(session_uuid) => session_uuid,
        Err(_) => resolve_chat_session_uuid(pool, session_id).await?,
    };

    if golish_db::repo::operation_state::get(pool, session_uuid)
        .await?
        .is_some()
    {
        return Ok(session_uuid);
    }

    golish_db::repo::tasks::latest_resumable_by_session(pool, session_uuid)
        .await?
        .map(|task| task.id)
        .ok_or_else(|| {
            GolishError::NotFound(format!(
                "no resumable harness operation found for session {session_uuid}"
            ))
        })
}

async fn resolve_chat_session_uuid(
    pool: &sqlx::PgPool,
    chat_session_key: &str,
) -> Result<Uuid, GolishError> {
    sqlx::query_scalar::<_, Uuid>(RESOLVE_CHAT_SESSION_UUID_SQL)
        .bind(chat_session_key)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| {
            GolishError::NotFound(format!(
                "no DB session anchored to chat session {chat_session_key}"
            ))
        })
}

fn projected_dag_for_profile(profile_id: &str) -> Result<AllowedDag, GolishError> {
    let profile = match load_embedded_profile(profile_id) {
        Ok(Some(profile)) => profile,
        _ => load_embedded_profile("assessment")
            .map_err(|e| GolishError::Internal(format!("load assessment profile: {e}")))?
            .ok_or_else(|| GolishError::Internal("assessment profile missing".to_string()))?,
    };
    let graph = base_operation_graph()
        .map_err(|e| GolishError::Internal(format!("load operation graph: {e}")))?;
    Ok(graph.project(&profile.allowed_stage_set()))
}

fn affected_stages(
    mode: HarnessDevResetMode,
    stage: StageKind,
    dag: &AllowedDag,
) -> HashSet<StageKind> {
    match mode {
        HarnessDevResetMode::RestartFromStage | HarnessDevResetMode::RestartFromStagePurge => {
            let descendants = dag.descendants_inclusive(stage);
            if descendants.is_empty() {
                HashSet::from([stage])
            } else {
                descendants
            }
        }
        HarnessDevResetMode::ClearRepair | HarnessDevResetMode::RestartStage => {
            HashSet::from([stage])
        }
    }
}

fn dag_ordered_stage_names(dag: &AllowedDag, affected: &HashSet<StageKind>) -> Vec<String> {
    let mut names: Vec<String> = dag
        .nodes
        .iter()
        .copied()
        .filter(|stage| affected.contains(stage))
        .map(|stage| stage.as_str().to_string())
        .collect();
    if names.is_empty() {
        names.extend(
            affected
                .iter()
                .copied()
                .map(|stage| stage.as_str().to_string()),
        );
        names.sort();
    }
    names
}

fn reset_state_blob(
    mut blob: Value,
    stage: StageKind,
    affected: &HashSet<StageKind>,
    organization_id: Option<&str>,
    mode: HarnessDevResetMode,
) -> (Value, StateBlobResetStats) {
    if !blob.is_object() {
        blob = json!({});
    }

    let mut stats = StateBlobResetStats::default();
    if remove_matching_agent_run(&mut blob, affected, organization_id) {
        stats.cleared_agent_run_checkpoints = 1;
    }
    if mode.resets_stage_cursor() {
        stats.cleared_stage_run_workers = remove_stage_run_workers(&mut blob, affected, None);
        let graph_stats = reset_graph_flow(&mut blob, stage, affected);
        stats.reset_graph_flow = graph_stats.reset_graph_flow;
        stats.trimmed_graph_flow_visited = graph_stats.trimmed_graph_flow_visited;
        stats.trimmed_graph_flow_applied = graph_stats.trimmed_graph_flow_applied;
    }

    (blob, stats)
}

fn remove_matching_agent_run(
    blob: &mut Value,
    affected: &HashSet<StageKind>,
    organization_id: Option<&str>,
) -> bool {
    let Some(checkpoint) = agent_run_from_state_blob(blob) else {
        return false;
    };
    if !agent_run_matches(&checkpoint, affected, organization_id) {
        return false;
    }
    *blob = state_blob_without_agent_run(std::mem::take(blob));
    true
}

fn agent_run_matches(
    checkpoint: &AgentRunCheckpoint,
    affected: &HashSet<StageKind>,
    organization_id: Option<&str>,
) -> bool {
    let stage_matches = checkpoint
        .stage
        .as_deref()
        .and_then(StageKind::try_parse)
        .map(|stage| affected.contains(&stage))
        .unwrap_or(false)
        || affected.iter().any(|stage| {
            checkpoint
                .agent_path
                .contains(&format!("stage_run:{}", stage.as_str()))
        });
    if !stage_matches {
        return false;
    }
    organization_id.is_none_or(|org_id| {
        checkpoint.agent_path.contains(&format!("org:{org_id}"))
            || checkpoint
                .last_tool
                .as_ref()
                .map(|tool| tool.tool_call_id.contains(org_id))
                .unwrap_or(false)
    })
}

fn remove_stage_run_workers(
    blob: &mut Value,
    affected: &HashSet<StageKind>,
    organization_id: Option<&str>,
) -> usize {
    let Some(workers) = blob
        .get_mut("stage_run_workers")
        .and_then(Value::as_object_mut)
    else {
        return 0;
    };

    let mut removed = 0usize;
    for stage in affected {
        let key = stage.as_str();
        if let Some(org_id) = organization_id {
            if let Some(stage_value) = workers.get_mut(key) {
                if let Some(stage_map) = stage_value.as_object_mut() {
                    if stage_map.remove(org_id).is_some() {
                        removed += 1;
                    }
                    if stage_map.is_empty() {
                        workers.remove(key);
                    }
                }
            }
            continue;
        }
        if let Some(stage_value) = workers.remove(key) {
            removed += stage_value.as_object().map(|m| m.len()).unwrap_or(1);
        }
    }
    if workers.is_empty() {
        if let Some(root) = blob.as_object_mut() {
            root.remove("stage_run_workers");
        }
    }
    removed
}

fn reset_graph_flow(
    blob: &mut Value,
    stage: StageKind,
    affected: &HashSet<StageKind>,
) -> StateBlobResetStats {
    let mut stats = StateBlobResetStats {
        reset_graph_flow: true,
        ..StateBlobResetStats::default()
    };
    let root = ensure_object(blob);
    let graph_flow = root
        .entry("graph_flow".to_string())
        .or_insert_with(|| json!({}));
    let graph_flow = ensure_object(graph_flow);
    graph_flow.insert("next_node".to_string(), json!(stage.as_str()));
    let state_value = graph_flow
        .entry("state".to_string())
        .or_insert_with(|| json!(OperationFlowState::default()));

    match serde_json::from_value::<OperationFlowState>(state_value.clone()) {
        Ok(mut flow) => {
            let before_visited = flow.visited.len();
            let before_applied = flow.applied.len();
            flow.visited.retain(|visited| !affected.contains(visited));
            flow.applied.retain(|stage, _| !affected.contains(stage));
            flow.seeded.retain(|stage, _| !affected.contains(stage));
            stats.trimmed_graph_flow_visited = before_visited.saturating_sub(flow.visited.len());
            stats.trimmed_graph_flow_applied = before_applied.saturating_sub(flow.applied.len());
            *state_value = serde_json::to_value(flow).unwrap_or_else(|_| json!({}));
        }
        Err(_) => {
            let raw_stats = trim_raw_graph_flow_state(state_value, affected);
            stats.trimmed_graph_flow_visited = raw_stats.trimmed_graph_flow_visited;
            stats.trimmed_graph_flow_applied = raw_stats.trimmed_graph_flow_applied;
        }
    }

    stats
}

fn ensure_object(value: &mut Value) -> &mut serde_json::Map<String, Value> {
    if !value.is_object() {
        *value = json!({});
    }
    value.as_object_mut().unwrap()
}

fn trim_raw_graph_flow_state(
    state_value: &mut Value,
    affected: &HashSet<StageKind>,
) -> StateBlobResetStats {
    let affected_names: HashSet<&'static str> =
        affected.iter().map(|stage| stage.as_str()).collect();
    let mut stats = StateBlobResetStats::default();
    let Some(state) = state_value.as_object_mut() else {
        return stats;
    };
    if let Some(visited) = state.get_mut("visited").and_then(Value::as_array_mut) {
        let before = visited.len();
        visited.retain(|value| {
            value
                .as_str()
                .map(|stage| !affected_names.contains(stage))
                .unwrap_or(true)
        });
        stats.trimmed_graph_flow_visited = before.saturating_sub(visited.len());
    }
    for key in ["applied", "seeded"] {
        if let Some(map) = state.get_mut(key).and_then(Value::as_object_mut) {
            let before = map.len();
            map.retain(|stage, _| !affected_names.contains(stage.as_str()));
            if key == "applied" {
                stats.trimmed_graph_flow_applied = before.saturating_sub(map.len());
            }
        }
    }
    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use golish_agent_kit::harness::operation_flow::StageFlowOutcome;

    fn state_with_flow() -> Value {
        let flow = OperationFlowState {
            visited: vec![
                StageKind::Scoping,
                StageKind::TargetIntel,
                StageKind::ExternalAttackSurface,
                StageKind::Enumeration,
                StageKind::Reporting,
            ],
            applied: HashMap::from([
                (StageKind::Scoping, StageFlowOutcome::pass_with_progress()),
                (
                    StageKind::TargetIntel,
                    StageFlowOutcome::pass_with_progress(),
                ),
                (
                    StageKind::ExternalAttackSurface,
                    StageFlowOutcome::pass_with_progress(),
                ),
                (StageKind::Enumeration, StageFlowOutcome::blocked()),
                (StageKind::Reporting, StageFlowOutcome::pass_with_progress()),
            ]),
            ..Default::default()
        };
        json!({
            "graph_flow": {
                "state": flow,
                "next_node": "reporting"
            },
            "agent_run": {
                "operation_id": "11111111-1111-1111-1111-111111111111",
                "stage": "enumeration",
                "agent_path": "main>stage_run:enumeration>org:22222222-2222-2222-2222-222222222222>enumerator",
                "status": "gate_blocked",
                "updated_at": "2026-06-29T00:00:00Z"
            },
            "stage_run_workers": {
                "enumeration": {
                    "22222222-2222-2222-2222-222222222222": {
                        "chain_id": "33333333-3333-3333-3333-333333333333",
                        "specialist": "enumerator"
                    }
                },
                "reporting": {
                    "22222222-2222-2222-2222-222222222222": {
                        "chain_id": "44444444-4444-4444-4444-444444444444",
                        "specialist": "reporter"
                    }
                }
            }
        })
    }

    fn assessment_dag() -> AllowedDag {
        projected_dag_for_profile("assessment").expect("assessment dag")
    }

    #[test]
    fn chat_session_lookup_uses_stable_chat_anchor() {
        assert!(RESOLVE_CHAT_SESSION_UUID_SQL.contains("sessions"));
        assert!(RESOLVE_CHAT_SESSION_UUID_SQL.contains("chat_session_key = $1"));
        assert!(RESOLVE_CHAT_SESSION_UUID_SQL.contains("ORDER BY updated_at DESC"));
        assert!(RESOLVE_CHAT_SESSION_UUID_SQL.contains("LIMIT 1"));
    }

    #[test]
    fn restart_from_stage_repoints_graph_flow_and_clears_descendant_workers() {
        let dag = assessment_dag();
        let affected = affected_stages(
            HarnessDevResetMode::RestartFromStage,
            StageKind::Enumeration,
            &dag,
        );
        let (next, stats) = reset_state_blob(
            state_with_flow(),
            StageKind::Enumeration,
            &affected,
            None,
            HarnessDevResetMode::RestartFromStage,
        );

        assert!(next.get("agent_run").is_none());
        assert_eq!(next["graph_flow"]["next_node"], "enumeration");
        assert!(next["stage_run_workers"].get("enumeration").is_none());
        assert!(next["stage_run_workers"].get("reporting").is_none());
        assert_eq!(stats.cleared_agent_run_checkpoints, 1);
        assert_eq!(stats.cleared_stage_run_workers, 2);
        assert!(stats.reset_graph_flow);
        assert_eq!(stats.trimmed_graph_flow_visited, 2);
        assert_eq!(stats.trimmed_graph_flow_applied, 2);

        let flow: OperationFlowState =
            serde_json::from_value(next["graph_flow"]["state"].clone()).expect("flow state");
        assert_eq!(
            flow.visited,
            vec![
                StageKind::Scoping,
                StageKind::TargetIntel,
                StageKind::ExternalAttackSurface
            ]
        );
        assert!(!flow.applied.contains_key(&StageKind::Enumeration));
        assert!(!flow.applied.contains_key(&StageKind::Reporting));
    }

    #[test]
    fn clear_repair_removes_matching_agent_run_but_keeps_worker_chains() {
        let affected = HashSet::from([StageKind::Enumeration]);
        let (next, stats) = reset_state_blob(
            state_with_flow(),
            StageKind::Enumeration,
            &affected,
            Some("22222222-2222-2222-2222-222222222222"),
            HarnessDevResetMode::ClearRepair,
        );

        assert!(next.get("agent_run").is_none());
        assert!(next["stage_run_workers"].get("enumeration").is_some());
        assert_eq!(next["graph_flow"]["next_node"], "reporting");
        assert_eq!(stats.cleared_agent_run_checkpoints, 1);
        assert_eq!(stats.cleared_stage_run_workers, 0);
        assert!(!stats.reset_graph_flow);
    }
}
