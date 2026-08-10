//! Developer-only harness checkpoint controls.
//!
//! These commands are dev-only: the checkpoint modes adjust resumability state,
//! while the explicit `restart_from_stage_purge` mode additionally removes the
//! selected stage's affected facts through the scoped, transactional purge path.

use std::collections::{BTreeSet, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::State;
use ts_rs::TS;
use uuid::Uuid;

use crate::error::GolishError;
use crate::state::AgentState;
use golish_agent_kit::harness::operation_flow::OperationFlowState;
use golish_agent_kit::harness::{
    load_embedded_profile, load_embedded_stage_spec, operation_graph_for_topology, AllowedDag,
    StageKind, StageTopologyContract,
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
    /// stage and its DAG descendants, in the operation's exact frozen scope, so
    /// re-testing the stage starts from a clean slate.
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

/// Which fact domain a stage produces (None = stage has no purgeable facts).
fn fact_domain(stage: StageKind) -> Option<golish_db::repo::stage_purge::StagePurgeDomain> {
    use golish_db::repo::stage_purge::StagePurgeDomain;

    Some(match stage {
        StageKind::TargetIntel => StagePurgeDomain::TargetIntel,
        StageKind::ExternalAttackSurface => StagePurgeDomain::Eas,
        StageKind::Enumeration => StagePurgeDomain::Enumeration,
        StageKind::VulnTriage => StagePurgeDomain::Vuln,
        StageKind::Scoping
        | StageKind::ApplicationUnderstanding
        | StageKind::Investigation
        | StageKind::AttackCandidate
        | StageKind::Verification
        | StageKind::AccessValidation
        | StageKind::InternalDiscovery
        | StageKind::ObjectivePathing
        | StageKind::ObjectiveSimulation
        | StageKind::Reporting
        | StageKind::Cleanup => return None,
    })
}

fn is_in_place_company_stage(stage: StageKind) -> bool {
    matches!(
        stage,
        StageKind::TargetIntel
            | StageKind::ExternalAttackSurface
            | StageKind::Enumeration
            | StageKind::VulnTriage
    )
}

fn validate_in_place_full_reset(
    selected_stage: StageKind,
    current_stage: StageKind,
    reached_stages: &HashSet<StageKind>,
    dag: &AllowedDag,
) -> Result<(), &'static str> {
    if !is_in_place_company_stage(selected_stage)
        || !is_in_place_company_stage(current_stage)
        || reached_stages
            .iter()
            .any(|stage| *stage != StageKind::Scoping && !is_in_place_company_stage(*stage))
    {
        return Err("stage_reset_requires_new_operation");
    }
    if !reached_stages.contains(&selected_stage) {
        return Err("stage_reset_stage_not_reached");
    }
    if !dag
        .descendants_inclusive(selected_stage)
        .contains(&current_stage)
    {
        return Err("stage_reset_stage_not_ancestor");
    }
    Ok(())
}

fn parse_reached_stages<'a>(
    stages: impl IntoIterator<Item = &'a str>,
) -> Result<HashSet<StageKind>, &'static str> {
    stages
        .into_iter()
        .map(|stage| StageKind::try_parse(stage).ok_or("stage_reset_unknown_history_stage"))
        .collect()
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

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
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
    /// Number of exact frozen-scope organizations the purge was scoped to.
    pub purge_scope_org_count: usize,
    /// Per-table affected-row counts (None unless `purged_facts`). The detailed
    /// DB counter shape remains diagnostic-only at the IPC boundary.
    #[ts(type = "unknown")]
    pub purge_counts: Option<golish_db::repo::stage_purge::StagePurgeCounts>,
    /// Reserved for non-fatal diagnostics. Full purge never silently skips.
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
    let operation_view = crate::ai::db_bridge::operation_state_view_from_db(row.clone())
        .map_err(|error| GolishError::Internal(format!("validate operation contracts: {error}")))?;
    let dag = projected_dag_for_profile(
        &row.profile,
        operation_view.stage_topology_contract.topology,
    )?;
    if !dag.contains(stage_kind) {
        return Err(GolishError::Validation(format!(
            "stage '{}' is not enabled for profile '{}'",
            stage_kind.as_str(),
            row.profile
        )));
    }

    if mode.purges_facts() {
        let current_stage = StageKind::try_parse(&row.current_stage).ok_or_else(|| {
            GolishError::Validation(format!(
                "stage_reset_requires_new_operation: current stage '{}' is not resettable in place",
                row.current_stage
            ))
        })?;
        let stage_runs =
            golish_db::repo::stage_runs::list_for_operation(&state.db_pool, operation_id).await?;
        let reached_stages = parse_reached_stages(
            stage_runs.iter().map(|run| run.stage_kind.as_str()),
        )
        .map_err(|code| {
            GolishError::Validation(format!(
                "{code}: operation history contains an unknown stage and cannot be reset safely"
            ))
        })?;
        validate_in_place_full_reset(stage_kind, current_stage, &reached_stages, &dag).map_err(
            |code| {
                let detail = match code {
                    "stage_reset_stage_not_reached" => "selected stage was not reached by this operation",
                    "stage_reset_stage_not_ancestor" => "selected stage is not the current stage or one of its DAG ancestors",
                    _ => "this stage family owns immutable history; create a new test operation or stage fork",
                };
                GolishError::Validation(format!("{code}: {detail}"))
            },
        )?;
    }

    let affected = affected_stages(mode, stage_kind, &dag);
    let affected_names = dag_ordered_stage_names(&dag, &affected);
    let fact_purge = mode
        .purges_facts()
        .then(|| stage_checkpoint_purge_plan(&affected, &affected_names, stage_kind))
        .transpose()?;
    let (next_blob, stats) = reset_state_blob(
        row.state_blob.clone(),
        stage_kind,
        &affected,
        organization_id.as_deref(),
        mode,
    );
    let runtime_stats = golish_db::repo::runtime_memory_tx::supersede_stage_checkpoint(
        &state.db_pool,
        &golish_db::repo::runtime_memory_tx::SupersedeStageCheckpointRow {
            operation_id,
            expected_active_stage_execution_id: None,
            expected_current_stage: row.current_stage.clone(),
            selected_stage: stage_kind.as_str().to_string(),
            affected_stage_kinds: affected_names.clone(),
            next_state_blob: next_blob,
            replacement_specialist: load_embedded_stage_spec(stage_kind)
                .ok()
                .and_then(|spec| spec.specialist)
                .filter(|specialist| !specialist.trim().is_empty()),
            replacement_stage_execution_id: mode.resets_stage_cursor().then(Uuid::new_v4),
            fact_purge,
            finalizer_recovery_witness: None,
        },
    )
    .await
    .map_err(|error| match error {
        golish_db::repo::runtime_memory_tx::RuntimeMemoryStoreError::Conflict {
            code: "stage_checkpoint_reset_active_tool_in_flight",
        } => GolishError::Validation(
            "stage_checkpoint_reset_active_tool_in_flight: wait for or stop the active stage tool before resetting"
                .to_string(),
        ),
        error => GolishError::Internal(format!(
            "atomically supersede runtime stage checkpoint: {error}"
        )),
    })?;

    let purged_facts = mode.purges_facts();
    let purge_scope_org_count = runtime_stats.purge_scope_org_count;
    let purge_counts = runtime_stats.purge_counts.clone();
    let purge_note = None;

    let current_stage = if mode.resets_stage_cursor() {
        stage_kind.as_str().to_string()
    } else {
        row.current_stage.clone()
    };

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
        message: format!(
            "{}; superseded workers={}, units={}, stage_executions={}, invalidated_handoffs={}",
            match mode {
                HarnessDevResetMode::ClearRepair =>
                    "cleared matching repair checkpoint".to_string(),
                HarnessDevResetMode::RestartStage => "reset selected stage checkpoint".to_string(),
                HarnessDevResetMode::RestartFromStage => {
                    "reset graph-flow checkpoint from selected stage".to_string()
                }
                HarnessDevResetMode::RestartFromStagePurge => {
                    "reset checkpoint and purged discovered facts from selected stage".to_string()
                }
            },
            runtime_stats.workers_superseded,
            runtime_stats.units_superseded,
            runtime_stats.executions_superseded,
            runtime_stats.handoffs_invalidated,
        ),
        purged_facts,
        purge_scope_org_count,
        purge_counts,
        purge_note,
    })
}

fn stage_checkpoint_purge_plan(
    affected: &HashSet<StageKind>,
    affected_names: &[String],
    selected_stage: StageKind,
) -> Result<golish_db::repo::stage_purge::StageCheckpointPurgePlan, GolishError> {
    let mut domains = Vec::new();
    for stage in affected {
        if let Some(domain) = fact_domain(*stage) {
            if !domains.contains(&domain) {
                domains.push(domain);
            }
        }
    }
    Ok(golish_db::repo::stage_purge::StageCheckpointPurgePlan {
        domains,
        stage_kinds: affected_names.to_vec(),
        techniques: affected_stage_techniques(affected)?,
        target_status_floor: target_status_floor(selected_stage).map(str::to_string),
    })
}

fn affected_stage_techniques(affected: &HashSet<StageKind>) -> Result<Vec<String>, GolishError> {
    let mut techniques = BTreeSet::new();
    for stage in affected {
        let spec = load_embedded_stage_spec(*stage).map_err(|error| {
            GolishError::Internal(format!(
                "load embedded stage spec for '{}': {error}",
                stage.as_str()
            ))
        })?;
        techniques.extend(spec.expected_techniques);
    }
    Ok(techniques.into_iter().collect())
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

fn projected_dag_for_profile(
    profile_id: &str,
    topology: StageTopologyContract,
) -> Result<AllowedDag, GolishError> {
    let profile = load_embedded_profile(profile_id)
        .map_err(|e| GolishError::Internal(format!("load profile {profile_id}: {e}")))?
        .ok_or_else(|| GolishError::Internal(format!("profile {profile_id} is missing")))?;
    let graph = operation_graph_for_topology(topology)
        .map_err(|e| GolishError::Internal(format!("load operation graph for {topology}: {e}")))?;
    let allowed = profile
        .allowed_stage_set_for_topology(topology)
        .map_err(|e| {
            GolishError::Internal(format!("project profile {profile_id} for {topology}: {e}"))
        })?;
    Ok(graph.project(&allowed))
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
        projected_dag_for_profile(
            "assessment",
            StageTopologyContract::LegacyCandidateVerificationV1,
        )
        .expect("assessment dag")
    }

    #[test]
    fn stage_reset_policy_accepts_only_reached_company_stages_before_immutable_truth() {
        let dag = assessment_dag();
        let reached = HashSet::from([
            StageKind::Scoping,
            StageKind::TargetIntel,
            StageKind::ExternalAttackSurface,
        ]);

        assert_eq!(
            validate_in_place_full_reset(
                StageKind::TargetIntel,
                StageKind::ExternalAttackSurface,
                &reached,
                &dag,
            ),
            Ok(())
        );
        assert_eq!(
            validate_in_place_full_reset(
                StageKind::ExternalAttackSurface,
                StageKind::ExternalAttackSurface,
                &reached,
                &dag,
            ),
            Ok(())
        );
        assert_eq!(
            validate_in_place_full_reset(
                StageKind::Enumeration,
                StageKind::ExternalAttackSurface,
                &reached,
                &dag,
            ),
            Err("stage_reset_stage_not_reached")
        );

        let reached_after_rewind = HashSet::from([
            StageKind::Scoping,
            StageKind::TargetIntel,
            StageKind::ExternalAttackSurface,
            StageKind::Enumeration,
            StageKind::VulnTriage,
        ]);
        assert_eq!(
            validate_in_place_full_reset(
                StageKind::VulnTriage,
                StageKind::ExternalAttackSurface,
                &reached_after_rewind,
                &dag,
            ),
            Err("stage_reset_stage_not_ancestor"),
            "historical reachability must not authorize a forward jump after rewind"
        );
    }

    #[test]
    fn stage_reset_policy_rejects_scoping_and_immutable_stage_families() {
        let dag = assessment_dag();
        let reached = HashSet::from([
            StageKind::Scoping,
            StageKind::TargetIntel,
            StageKind::ExternalAttackSurface,
            StageKind::Enumeration,
            StageKind::VulnTriage,
            StageKind::AttackCandidate,
            StageKind::Reporting,
        ]);

        for selected in [
            StageKind::Scoping,
            StageKind::AttackCandidate,
            StageKind::Verification,
            StageKind::Reporting,
            StageKind::Cleanup,
        ] {
            assert_eq!(
                validate_in_place_full_reset(selected, StageKind::VulnTriage, &reached, &dag),
                Err("stage_reset_requires_new_operation"),
                "selected={}",
                selected.as_str()
            );
        }
        assert_eq!(
            validate_in_place_full_reset(
                StageKind::VulnTriage,
                StageKind::AttackCandidate,
                &reached,
                &dag,
            ),
            Err("stage_reset_requires_new_operation")
        );
    }

    #[test]
    fn stage_reset_policy_rejects_unknown_historical_stage_names() {
        assert_eq!(
            parse_reached_stages(["scoping", "target_intel", "future_unknown_stage"]),
            Err("stage_reset_unknown_history_stage")
        );
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

    #[test]
    fn affected_techniques_are_the_embedded_stage_spec_union_only() {
        let affected = HashSet::from([StageKind::TargetIntel, StageKind::Enumeration]);
        let techniques = affected_stage_techniques(&affected).expect("embedded stage specs");

        assert_eq!(techniques.len(), 10);
        for expected in [
            "GOLISH-INTEL-DNS",
            "GOLISH-INTEL-SUBDOMAIN",
            "GOLISH-INTEL-ASN",
            "GOLISH-INTEL-CT",
            "GOLISH-INTEL-WHOIS",
            "GOLISH-INTEL-OSINT",
            "GOLISH-ENUM-JS",
            "GOLISH-ENUM-DIR",
            "GOLISH-ENUM-PARAM",
            "GOLISH-ENUM-JSAPI",
        ] {
            assert!(techniques.iter().any(|technique| technique == expected));
        }
        assert!(!techniques
            .iter()
            .any(|technique| technique == "GOLISH-EAS-LIVENESS"));
    }
}
