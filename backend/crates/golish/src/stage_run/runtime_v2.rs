//! Trusted headless-CLI Runtime Memory V2 bootstrap/report adapters plus the
//! pure, fail-closed resume classification for one stage unit.

use std::collections::{HashMap, HashSet, VecDeque};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use golish_agent_kit::db_traits::{
    CliRuntimeScope, CliRuntimeScopeUnit, RuntimeStageUnitStatus, RuntimeWorkerStatus,
};
use golish_agent_kit::harness::StageKind;
use golish_agent_kit::runtime_memory::{RuntimeMemoryContract, RuntimeMemoryWriteStrategy};
use golish_core::AttackExecutionContract;
use golish_db::models::{AgentType, Organization};
use golish_db::repo::stage_run_units::StageRunUnitRow;
use golish_db::repo::stage_teams::{StageTeamPlanRow, StageWorkItemRow, StageWorkerOutputRow};
use golish_db::repo::stage_worker_runs::StageWorkerRunRow;
use uuid::Uuid;

use super::scheduler::{FleetReport, OrgRunOutcome, OrgRunStatus};

#[derive(Debug)]
struct RelationalResumeIncomplete {
    detail: String,
}

impl std::fmt::Display for RelationalResumeIncomplete {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "relational V2 resume source is structurally incomplete: {}",
            self.detail
        )
    }
}

impl std::error::Error for RelationalResumeIncomplete {}

/// A complete relational source exists but another worker still owns a live
/// lease. This is an availability/busy condition, never a missing-record signal
/// and therefore never eligible for preferred-mode legacy fallback.
#[derive(Debug)]
pub(crate) struct RelationalResumeBusy;

impl std::fmt::Display for RelationalResumeBusy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("relational V2 resume source has a live worker lease")
    }
}

impl std::error::Error for RelationalResumeBusy {}

fn relational_resume_incomplete(detail: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(RelationalResumeIncomplete {
        detail: detail.into(),
    })
}

pub(crate) fn relational_resume_error_allows_legacy_fallback(error: &anyhow::Error) -> bool {
    error.downcast_ref::<RelationalResumeIncomplete>().is_some()
}

fn decode_relational_message_chain(
    chain: &serde_json::Value,
) -> Result<Vec<rig::completion::Message>> {
    serde_json::from_value::<Vec<rig::completion::Message>>(chain.clone()).map_err(|error| {
        relational_resume_incomplete(format!("bound message chain cannot be decoded: {error}"))
    })
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeV2CliReport {
    pub operation_id: Uuid,
    pub scope_unit_count: usize,
    pub stage_unit_count: usize,
    pub fleet: FleetReport,
}

/// Proof marker returned only after one complete relational runtime source has
/// passed validation. The CLI resolver either selects this whole authority or
/// validates the whole legacy checkpoint; it never combines fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeV2ResumeAuthority {
    pub active_stage_execution_id: Uuid,
    pub organization_id: Uuid,
}

pub(crate) fn persisted_contract(value: &str) -> Result<RuntimeMemoryContract> {
    RuntimeMemoryContract::try_from(value)
        .map_err(|error| anyhow!("invalid persisted runtime-memory contract: {error}"))
}

pub(crate) fn contract_writes_v2(contract: RuntimeMemoryContract) -> bool {
    contract.policy().write != RuntimeMemoryWriteStrategy::LegacyOnly
}

fn effective_resume_specialist(
    stage: StageKind,
    configured: Option<&str>,
    runtime_contract: RuntimeMemoryContract,
    attack_contract: AttackExecutionContract,
) -> Option<String> {
    let configured = configured
        .map(str::trim)
        .filter(|specialist| !specialist.is_empty())
        .map(ToOwned::to_owned);
    let candidate_v2_enabled = match stage {
        StageKind::AttackCandidate => {
            contract_writes_v2(runtime_contract) && attack_contract.writes_v2()
        }
        StageKind::Verification => {
            runtime_contract == RuntimeMemoryContract::V2Only
                && attack_contract.executes_v2_verifier()
        }
        _ => false,
    };
    match stage {
        StageKind::Verification if candidate_v2_enabled => Some("candidate_verifier".to_string()),
        StageKind::AttackCandidate if candidate_v2_enabled => Some("attack_analyst".to_string()),
        _ => configured,
    }
}

fn exact_root_stage_preclaim(
    specialist: Option<&str>,
    execution_status: &str,
    unit_count: usize,
    worker_count: usize,
) -> bool {
    specialist.is_none() && execution_status == "started" && unit_count == 0 && worker_count == 0
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StageSliceBoundary {
    source_stage: StageKind,
    source_stage_execution_id: Uuid,
}

fn exact_stage_slice_boundary(
    operation: &golish_db::repo::operation_state::OperationStateRow,
    execution: &golish_db::repo::stage_runs::StageRunRow,
    unit_count: usize,
    worker_count: usize,
) -> Option<StageSliceBoundary> {
    if execution.status != "started"
        || execution.completed_at.is_some()
        || execution.started_at != operation.stage_started_at
        || unit_count != 0
        || worker_count != 0
    {
        return None;
    }
    let marker = operation
        .state_blob
        .get("stage_slice_boundary_v1")?
        .as_object()?;
    if marker.len() != 5
        || marker.get("schema_version")?.as_u64()? != 1
        || marker.get("successor_stage")?.as_str()? != operation.current_stage
        || marker.get("successor_stage_execution_id")?.as_str()? != execution.id.to_string()
    {
        return None;
    }
    let source_stage = StageKind::try_parse(marker.get("source_stage")?.as_str()?)?;
    let source_stage_execution_id =
        Uuid::parse_str(marker.get("source_stage_execution_id")?.as_str()?).ok()?;
    (source_stage != StageKind::try_parse(&operation.current_stage)?
        && source_stage_execution_id != execution.id)
        .then_some(StageSliceBoundary {
            source_stage,
            source_stage_execution_id,
        })
}

/// A stage specialist is the durable scheduler role, while message chains use
/// the coarser DB agent enum. Keep this mapping identical to the worker claim
/// path; comparing the two strings directly rejects every valid specialist
/// chain (for example `enumerator` is persisted as `agent = pentester`).
pub(crate) fn resume_worker_chain_agent(specialist: &str) -> Option<AgentType> {
    match specialist.trim() {
        "reporter" => Some(AgentType::Reporter),
        "recon"
        | "prober"
        | "enumerator"
        | "vuln_scanner"
        | "application_understanding"
        | "attack_analyst"
        | "candidate_verifier"
        | "pentester"
        | "investigation"
        | "researcher"
        | "browser"
        | "coder"
        | "installer"
        | "enricher"
        | "memorist"
        | "adviser" => Some(AgentType::Pentester),
        _ => None,
    }
}

pub(crate) const fn persisted_agent_name(agent: AgentType) -> &'static str {
    match agent {
        AgentType::Primary => "primary",
        AgentType::Pentester => "pentester",
        AgentType::Coder => "coder",
        AgentType::Searcher => "searcher",
        AgentType::Memorist => "memorist",
        AgentType::Reporter => "reporter",
        AgentType::Adviser => "adviser",
        AgentType::Reflector => "reflector",
        AgentType::Enricher => "enricher",
        AgentType::Installer => "installer",
        AgentType::Summarizer => "summarizer",
        AgentType::Assistant => "assistant",
    }
}

/// Read the contract frozen on the CLI session's one operation. The deployment
/// rollout is only a bootstrap hint; this persisted value is the authority for
/// deciding whether the LegacyV1 child-operation adapter is reachable.
pub(crate) async fn load_session_operation_contract(
    pool: &sqlx::PgPool,
    session_id: Uuid,
) -> Result<RuntimeMemoryContract> {
    let tasks = golish_db::repo::tasks::list_by_session(pool, session_id)
        .await
        .context("load CLI session operations")?;
    let [task] = tasks.as_slice() else {
        return Err(anyhow!(
            "CLI expected exactly one parent operation, found {}",
            tasks.len()
        ));
    };
    let operation = golish_db::repo::operation_state::get(pool, task.id)
        .await
        .context("load CLI parent operation")?
        .ok_or_else(|| anyhow!("CLI parent operation_state is missing"))?;
    persisted_contract(&operation.runtime_memory_contract)
}

fn ownership_percent(organization: &Organization) -> Option<f64> {
    let value = organization
        .intel
        .get("asset_intel_discovery")
        .and_then(|value| {
            value
                .get("ownershipPercent")
                .or_else(|| value.get("ownership_percent"))
        })?;
    match value {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(value) => value
            .trim()
            .trim_end_matches('%')
            .trim()
            .parse::<f64>()
            .ok(),
        _ => None,
    }
    .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
}

#[derive(Debug)]
struct StageTeamResumeAuthority {
    plan: StageTeamPlanRow,
    work_items: Vec<StageWorkItemRow>,
    outputs: Vec<StageWorkerOutputRow>,
    completed_synthesis_primary_worker_ids: HashSet<Uuid>,
    recoverable_company_finalizer_worker_ids: HashSet<Uuid>,
    recoverable_target_intel_finalizer_worker_ids: HashSet<Uuid>,
}

fn exact_target_intel_finalizer_recovery_preflight(
    worker: &StageWorkerRunRow,
    team: &StageTeamResumeAuthority,
) -> bool {
    let plan = &team.plan;
    let leader_items = team
        .work_items
        .iter()
        .filter(|item| {
            item.role == plan.leader_role
                && item.stable_key == "leader:primary"
                && !item.required_for_barrier
                && item.created_by == "server_seed"
        })
        .collect::<Vec<_>>();
    let [item] = leader_items.as_slice() else {
        return false;
    };
    let outputs = team
        .outputs
        .iter()
        .filter(|output| output.work_item_id == item.id && output.worker_run_id == worker.id)
        .collect::<Vec<_>>();
    let [output] = outputs.as_slice() else {
        return false;
    };
    plan.stage_kind == "target_intel"
        && plan.requests_closed_at.is_some()
        && plan.final_submitter_worker_run_id == Some(worker.id)
        && plan.final_submitter_kind == "worker"
        && plan.aggregator_kind == "worker"
        && plan.aggregator_role.as_deref() == Some(plan.leader_role.as_str())
        && plan
            .dynamic_request_policy
            .get("coordination_mode")
            .and_then(serde_json::Value::as_str)
            == Some("company_controller")
        && item.status == "exhausted"
        && item.terminal_at.is_some()
        && worker.work_item_id == Some(item.id)
        && worker.status == "failed"
        && worker.terminal_at.is_some()
        && worker.lease_token.is_none()
        && worker.active_tool_call_id.is_none()
        && worker.message_chain_id.is_some()
        && worker
            .checkpoint
            .pointer("/stage_team_execution_failure/code")
            .and_then(serde_json::Value::as_str)
            == Some("stage_team_worker_lease_expired")
        && output.business_disposition == "blocked"
        && output
            .canonical_output
            .get("kind")
            .and_then(serde_json::Value::as_str)
            == Some("stage_team_attempts_exhausted")
        && output
            .canonical_output
            .get("failure_code")
            .and_then(serde_json::Value::as_str)
            == Some("stage_team_worker_lease_expired")
        && output
            .blocker_codes
            .iter()
            .any(|code| code == "STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED")
        && team
            .recoverable_target_intel_finalizer_worker_ids
            .contains(&worker.id)
}

fn exact_company_finalizer_recovery_preflight(
    worker: &StageWorkerRunRow,
    team: &StageTeamResumeAuthority,
) -> bool {
    let plan = &team.plan;
    let leader_items = team
        .work_items
        .iter()
        .filter(|item| {
            item.role == plan.leader_role
                && item.stable_key == "leader:primary"
                && !item.required_for_barrier
                && item.created_by == "server_seed"
        })
        .collect::<Vec<_>>();
    let [item] = leader_items.as_slice() else {
        return false;
    };
    let outputs = team
        .outputs
        .iter()
        .filter(|output| output.work_item_id == item.id && output.worker_run_id == worker.id)
        .collect::<Vec<_>>();
    let [output] = outputs.as_slice() else {
        return false;
    };
    let required_items = team
        .work_items
        .iter()
        .filter(|candidate| candidate.required_for_barrier)
        .collect::<Vec<_>>();
    let required_outputs_are_complete = required_items.iter().all(|required| {
        matches!(required.status.as_str(), "completed" | "exhausted")
            && team
                .outputs
                .iter()
                .any(|candidate| candidate.work_item_id == required.id)
    });
    plan.stage_kind != "target_intel"
        && plan.requests_closed_at.is_some()
        && plan.final_submitter_worker_run_id == Some(worker.id)
        && plan.final_submitter_kind == "worker"
        && plan.aggregator_kind == "worker"
        && plan.aggregator_role.as_deref() == Some(plan.leader_role.as_str())
        && plan
            .dynamic_request_policy
            .get("coordination_mode")
            .and_then(serde_json::Value::as_str)
            == Some("company_controller")
        && item.status == "exhausted"
        && item.terminal_at.is_some()
        && worker.work_item_id == Some(item.id)
        && worker.status == "failed"
        && worker.terminal_at.is_some()
        && worker.lease_token.is_none()
        && worker.active_tool_call_id.is_none()
        && worker.message_chain_id.is_some()
        && worker
            .checkpoint
            .pointer("/stage_team_execution_failure/code")
            .and_then(serde_json::Value::as_str)
            == Some("stage_team_worker_lease_expired")
        && output.business_disposition == "blocked"
        && output
            .canonical_output
            .get("kind")
            .and_then(serde_json::Value::as_str)
            == Some("stage_team_attempts_exhausted")
        && output
            .canonical_output
            .get("failure_code")
            .and_then(serde_json::Value::as_str)
            == Some("stage_team_worker_lease_expired")
        && output
            .blocker_codes
            .iter()
            .any(|code| code == "STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED")
        && required_outputs_are_complete
        && team
            .recoverable_company_finalizer_worker_ids
            .contains(&worker.id)
}

fn exact_replacement_preclaim(
    operation: &golish_db::repo::operation_state::OperationStateRow,
    execution: &golish_db::repo::stage_runs::StageRunRow,
    unit: &StageRunUnitRow,
) -> bool {
    let marker = operation.state_blob.get("runtime_v2_dev_reset");
    marker
        .and_then(|value| value.get("replacement_stage_execution_id"))
        .and_then(serde_json::Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        == Some(execution.id)
        && marker
            .and_then(|value| value.get("selected_stage"))
            .and_then(serde_json::Value::as_str)
            == Some(execution.stage_kind.as_str())
        && operation.current_stage == execution.stage_kind
        && unit.operation_id == operation.operation_id
        && unit.stage_execution_id == execution.id
        && unit.stage_kind == execution.stage_kind
        && unit.status == "queued"
        && unit.generation == 1
        && unit.started_at.is_none()
        && unit.terminal_at.is_none()
}

fn select_stage_team_primary_worker<'a>(
    unit: &StageRunUnitRow,
    workers: &'a [&StageWorkerRunRow],
    team: Option<&StageTeamResumeAuthority>,
) -> Result<Option<&'a StageWorkerRunRow>> {
    let Some(team) = team else {
        return match workers {
            [] => Ok(None),
            [worker] => Ok(Some(*worker)),
            _ => Err(anyhow!("multiple non-Team Workers own one stage Unit")),
        };
    };

    let plan = &team.plan;
    anyhow::ensure!(
        plan.operation_id == unit.operation_id
            && plan.stage_execution_id == unit.stage_execution_id
            && plan.stage_run_unit_id == unit.id
            && plan.scope_snapshot_id == unit.scope_snapshot_id
            && plan.organization_id == unit.organization_id
            && plan.stage_kind == unit.stage_kind,
        "Stage Team plan identity crossed Unit/operation/scope"
    );
    let aggregator_role = plan
        .aggregator_role
        .as_deref()
        .ok_or_else(|| anyhow!("Stage Team plan has no aggregator role"))?;
    anyhow::ensure!(
        plan.leader_role == aggregator_role
            && plan.aggregator_kind == "worker"
            && plan.final_submitter_kind == "worker",
        "Stage Team leader/aggregator/final-submitter contract diverged"
    );

    let leader_items = team
        .work_items
        .iter()
        .filter(|item| {
            item.role == plan.leader_role
                && item.stable_key == "leader:primary"
                && !item.required_for_barrier
                && item.created_by == "server_seed"
        })
        .collect::<Vec<_>>();
    let [leader_item] = leader_items.as_slice() else {
        return Err(anyhow!(
            "Stage Team Unit requires exactly one leader WorkItem, found {}",
            leader_items.len()
        ));
    };

    for item in &team.work_items {
        anyhow::ensure!(
            item.team_plan_id == plan.id
                && item.operation_id == unit.operation_id
                && item.stage_execution_id == unit.stage_execution_id
                && item.stage_run_unit_id == unit.id
                && item.scope_snapshot_id == unit.scope_snapshot_id
                && item.organization_id == unit.organization_id,
            "Stage Team WorkItem identity crossed plan/Unit/operation/scope"
        );
    }
    for output in &team.outputs {
        anyhow::ensure!(
            output.team_plan_id == plan.id
                && output.operation_id == unit.operation_id
                && output.stage_execution_id == unit.stage_execution_id
                && output.stage_run_unit_id == unit.id
                && output.scope_snapshot_id == unit.scope_snapshot_id
                && output.organization_id == unit.organization_id,
            "Stage Team WorkerOutput identity crossed plan/Unit/operation/scope"
        );
    }

    if workers.is_empty() {
        anyhow::ensure!(
            unit.status == "queued"
                && unit.started_at.is_none()
                && unit.terminal_at.is_none()
                && plan.dispatch_epoch == 0
                && plan.requests_closed_at.is_none()
                && plan.final_submitter_worker_run_id.is_none()
                && team.work_items.len() == 1
                && leader_item.status == "queued"
                && leader_item.dispatch_epoch == 0
                && leader_item.started_at.is_none()
                && leader_item.terminal_at.is_none(),
            "Stage Team Unit has no leader Worker outside the exact fresh pre-claim shape"
        );
        return Ok(None);
    }

    let mut leader_workers = Vec::new();
    for worker in workers {
        let work_item_id = worker
            .work_item_id
            .ok_or_else(|| anyhow!("Stage Team Worker has no bound WorkItem"))?;
        let item = team
            .work_items
            .iter()
            .find(|item| item.id == work_item_id)
            .ok_or_else(|| anyhow!("Stage Team Worker references a foreign WorkItem"))?;
        anyhow::ensure!(
            worker.operation_id == unit.operation_id
                && worker.stage_execution_id == unit.stage_execution_id
                && worker.stage_run_unit_id == unit.id
                && worker.organization_id == unit.organization_id
                && worker.specialist == item.role
                && worker.work_item_kind == item.kind
                && worker.work_item_key == item.stable_key,
            "Stage Team Worker identity crossed WorkItem/Unit/operation"
        );
        if item.id == leader_item.id {
            leader_workers.push(*worker);
        }
    }
    let [leader_worker] = leader_workers.as_slice() else {
        return Err(anyhow!(
            "Stage Team Unit requires exactly one leader Worker, found {}",
            leader_workers.len()
        ));
    };
    let recovery_stable_key = format!("leader:synthesis-recovery:{}", leader_item.id);
    let recovery_v1_item_id = Uuid::new_v5(
        &leader_item.id,
        b"sealed-investigation-synthesis-recovery-primary-v1",
    );
    let recovery_v2_item_id = Uuid::new_v5(
        &recovery_v1_item_id,
        b"sealed-investigation-synthesis-recovery-primary-v2",
    );
    let synthesis_recovery_items = team
        .work_items
        .iter()
        .filter(|item| {
            item.role == plan.leader_role
                && item.stable_key.starts_with("leader:synthesis-recovery:")
                && !item.required_for_barrier
                && item.created_by == "server_seed"
        })
        .collect::<Vec<_>>();
    if synthesis_recovery_items.is_empty() {
        if leader_item.kind == "investigation_primary"
            && (leader_item.status == "completed" || leader_worker.status == "passed")
        {
            anyhow::ensure!(
                leader_item.status == "completed"
                    && leader_item.started_at.is_some()
                    && leader_item.terminal_at.is_some()
                    && leader_worker.status == "passed"
                    && leader_worker.terminal_at.is_some()
                    && leader_worker.lease_token.is_none()
                    && leader_worker.active_tool_call_id.is_none()
                    && team
                        .completed_synthesis_primary_worker_ids
                        .contains(&leader_worker.id),
                "completed Stage Team Primary has no sealed synthesis resume witness"
            );
        }
        return Ok(Some(*leader_worker));
    }
    anyhow::ensure!(
        synthesis_recovery_items.len() <= 2
            && synthesis_recovery_items
                .iter()
                .all(|item| item.stable_key == recovery_stable_key),
        "Stage Team Unit has foreign or duplicate synthesis recovery WorkItems"
    );
    let recovery_v1_items = synthesis_recovery_items
        .iter()
        .copied()
        .filter(|item| item.id == recovery_v1_item_id && item.kind == leader_item.kind)
        .collect::<Vec<_>>();
    let [recovery_v1] = recovery_v1_items.as_slice() else {
        return Err(anyhow!(
            "Stage Team Unit requires one deterministic synthesis recovery v1 WorkItem"
        ));
    };
    let recovery_v2_items = synthesis_recovery_items
        .iter()
        .copied()
        .filter(|item| {
            item.id == recovery_v2_item_id && item.kind == "investigation_primary_recovery"
        })
        .collect::<Vec<_>>();
    anyhow::ensure!(
        recovery_v2_items.len() <= 1,
        "Stage Team Unit has duplicate synthesis recovery v2 WorkItems"
    );
    let exact_recovery_identity = |item: &StageWorkItemRow| {
        item.team_plan_id == leader_item.team_plan_id
            && item.operation_id == leader_item.operation_id
            && item.stage_execution_id == leader_item.stage_execution_id
            && item.stage_run_unit_id == leader_item.stage_run_unit_id
            && item.scope_snapshot_id == leader_item.scope_snapshot_id
            && item.organization_id == leader_item.organization_id
            && item.dispatch_epoch == leader_item.dispatch_epoch
            && item.role == leader_item.role
            && item.input_manifest_hash == leader_item.input_manifest_hash
            && item.input_refs == leader_item.input_refs
            && !item.required_for_barrier
            && item.conflict_key.is_none()
            && item.priority == leader_item.priority
            && item.attempt_policy == leader_item.attempt_policy
            && item.budget == leader_item.budget
            && item.output_schema == leader_item.output_schema
            && item.created_by == "server_seed"
    };
    anyhow::ensure!(
        exact_recovery_identity(recovery_v1),
        "sealed synthesis recovery v1 immutable identity drifted"
    );
    let exact_failed_attempt = |item: &StageWorkItemRow, worker: &StageWorkerRunRow| {
        let outputs = team
            .outputs
            .iter()
            .filter(|output| output.work_item_id == item.id)
            .collect::<Vec<_>>();
        let [output] = outputs.as_slice() else {
            return false;
        };
        item.status == "exhausted"
            && item.terminal_at.is_some()
            && worker.work_item_id == Some(item.id)
            && worker.status == "failed"
            && worker.terminal_at.is_some()
            && worker.lease_token.is_none()
            && worker.active_tool_call_id.is_none()
            && output.worker_run_id == worker.id
            && output.business_disposition == "blocked"
            && output
                .canonical_output
                .get("kind")
                .and_then(|value| value.as_str())
                == Some("stage_team_attempts_exhausted")
            && output
                .canonical_output
                .get("failure_code")
                .and_then(|value| value.as_str())
                == Some("stage_team_worker_lease_expired")
            && output
                .blocker_codes
                .iter()
                .any(|code| code == "STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED")
    };
    let exact_completed_attempt =
        |item: &StageWorkItemRow, item_workers: &[&'a StageWorkerRunRow]| {
            let [worker] = item_workers else {
                return None;
            };
            (item.status == "completed"
                && item.started_at.is_some()
                && item.terminal_at.is_some()
                && worker.work_item_id == Some(item.id)
                && worker.status == "passed"
                && worker.terminal_at.is_some()
                && worker.lease_token.is_none()
                && worker.active_tool_call_id.is_none()
                && team
                    .completed_synthesis_primary_worker_ids
                    .contains(&worker.id))
            .then_some(*worker)
        };
    anyhow::ensure!(
        exact_failed_attempt(leader_item, leader_worker),
        "sealed synthesis recovery source failure witness is incomplete"
    );
    let recovery_v1_workers = workers
        .iter()
        .copied()
        .filter(|worker| worker.work_item_id == Some(recovery_v1.id))
        .collect::<Vec<_>>();
    match (recovery_v1.status.as_str(), recovery_v2_items.as_slice()) {
        ("queued", []) => {
            anyhow::ensure!(
                recovery_v1.started_at.is_none()
                    && recovery_v1.terminal_at.is_none()
                    && recovery_v1_workers.is_empty()
                    && !team
                        .outputs
                        .iter()
                        .any(|output| output.work_item_id == recovery_v1.id),
                "sealed synthesis recovery v1 queued pre-claim is incomplete"
            );
            Ok(None)
        }
        ("running", []) => {
            let [worker] = recovery_v1_workers.as_slice() else {
                return Err(anyhow!(
                    "running synthesis recovery v1 requires exactly one Worker"
                ));
            };
            anyhow::ensure!(
                recovery_v1.started_at.is_some()
                    && recovery_v1.terminal_at.is_none()
                    && worker.worker_generation > leader_worker.worker_generation
                    && matches!(
                        worker.status.as_str(),
                        "queued" | "running" | "gate_blocked" | "recovery_required"
                    )
                    && worker.terminal_at.is_none()
                    && !team
                        .outputs
                        .iter()
                        .any(|output| output.work_item_id == recovery_v1.id),
                "running synthesis recovery v1 authority is incomplete"
            );
            Ok(Some(*worker))
        }
        ("completed", []) => {
            let worker = exact_completed_attempt(recovery_v1, &recovery_v1_workers)
                .ok_or_else(|| anyhow!("completed synthesis recovery v1 witness is incomplete"))?;
            anyhow::ensure!(
                worker.worker_generation > leader_worker.worker_generation,
                "completed synthesis recovery v1 Worker generation is not monotonic"
            );
            Ok(Some(worker))
        }
        ("exhausted", [recovery_v2]) => {
            let [recovery_v1_worker] = recovery_v1_workers.as_slice() else {
                return Err(anyhow!(
                    "exhausted synthesis recovery v1 requires exactly one Worker"
                ));
            };
            anyhow::ensure!(
                exact_failed_attempt(recovery_v1, recovery_v1_worker)
                    && exact_recovery_identity(recovery_v2),
                "sealed synthesis recovery v1/v2 authority is incomplete"
            );
            let recovery_v2_workers = workers
                .iter()
                .copied()
                .filter(|worker| worker.work_item_id == Some(recovery_v2.id))
                .collect::<Vec<_>>();
            match recovery_v2.status.as_str() {
                "queued" => {
                    anyhow::ensure!(
                        recovery_v2.started_at.is_none()
                            && recovery_v2.terminal_at.is_none()
                            && recovery_v2_workers.is_empty()
                            && !team
                                .outputs
                                .iter()
                                .any(|output| output.work_item_id == recovery_v2.id),
                        "sealed synthesis recovery v2 queued pre-claim is incomplete"
                    );
                    Ok(None)
                }
                "running" => {
                    let [worker] = recovery_v2_workers.as_slice() else {
                        return Err(anyhow!(
                            "running synthesis recovery v2 requires exactly one Worker"
                        ));
                    };
                    anyhow::ensure!(
                        recovery_v2.started_at.is_some()
                            && recovery_v2.terminal_at.is_none()
                            && worker.worker_generation > recovery_v1_worker.worker_generation
                            && matches!(
                                worker.status.as_str(),
                                "queued" | "running" | "gate_blocked" | "recovery_required"
                            )
                            && worker.terminal_at.is_none()
                            && !team
                                .outputs
                                .iter()
                                .any(|output| output.work_item_id == recovery_v2.id),
                        "running synthesis recovery v2 authority is incomplete"
                    );
                    Ok(Some(*worker))
                }
                "completed" => {
                    let worker = exact_completed_attempt(recovery_v2, &recovery_v2_workers)
                        .ok_or_else(|| {
                            anyhow!("completed synthesis recovery v2 witness is incomplete")
                        })?;
                    anyhow::ensure!(
                        worker.worker_generation > recovery_v1_worker.worker_generation,
                        "completed synthesis recovery v2 Worker generation is not monotonic"
                    );
                    Ok(Some(worker))
                }
                status => Err(anyhow!(
                    "synthesis recovery v2 has non-resumable status {status}"
                )),
            }
        }
        (status, _) => Err(anyhow!(
            "synthesis recovery chain has non-resumable v1 status/shape {status}"
        )),
    }
}

async fn load_stage_team_resume_authorities(
    pool: &sqlx::PgPool,
    units: &[StageRunUnitRow],
) -> Result<HashMap<Uuid, StageTeamResumeAuthority>> {
    let mut authorities = HashMap::new();
    for unit in units {
        let Some(plan) =
            golish_db::repo::stage_teams::get_plan_for_unit_with_executor(pool, unit.id)
                .await
                .context("load Stage Team plan for V2 runtime authority")?
        else {
            continue;
        };
        let work_items = golish_db::repo::stage_teams::list_work_items_with_executor(pool, plan.id)
            .await
            .context("load Stage Team WorkItems for V2 runtime authority")?;
        let outputs = golish_db::repo::stage_teams::list_outputs_with_executor(pool, plan.id)
            .await
            .context("load Stage Team WorkerOutputs for V2 runtime authority")?;
        let completed_synthesis_primary_worker_ids = sqlx::query_scalar::<_, Uuid>(
            r#"SELECT DISTINCT census.primary_worker_run_id
                 FROM investigation_pentagi_task_plans task_plan
                 JOIN investigation_pentagi_delegation_census_seals census
                   ON census.task_plan_id=task_plan.task_plan_id
                 JOIN investigation_pentagi_pipeline_events event
                   ON event.task_plan_id=task_plan.task_plan_id
                  AND event.event_kind='primary_synthesis'
                  AND event.actor_worker_run_id=census.primary_worker_run_id
                  AND event.parent_dispatch_receipt_id=census.primary_dispatch_receipt_id
                 JOIN investigation_refiner_plan_ledger_seals refiner_seal
                   ON refiner_seal.task_plan_id=task_plan.task_plan_id
                 JOIN stage_worker_runs worker ON worker.id=census.primary_worker_run_id
                 JOIN stage_work_items item ON item.id=worker.work_item_id
                 JOIN investigation_stage_run_authorities authority
                   ON authority.operation_id=task_plan.operation_id
                  AND authority.stage_execution_id=task_plan.stage_execution_id
                  AND authority.authority_id=task_plan.authority_id
                 JOIN investigation_analysis_attempt_bindings binding
                   ON binding.authority_id=authority.authority_id
                  AND binding.stage_run_unit_id=task_plan.stage_run_unit_id
                  AND binding.organization_id=task_plan.organization_id
                  AND binding.analysis_attempt_id=task_plan.subject_id
                 JOIN investigation_run_work_items work
                   ON work.work_id=binding.work_id
                  AND work.authority_id=authority.authority_id
                  AND work.work_kind='analysis'
                 JOIN investigation_run_work_state_events latest
                   ON latest.event_id=work.latest_event_id AND latest.work_id=work.work_id
                 LEFT JOIN investigation_hypothesis_compilation_decisions decision
                   ON decision.binding_id=binding.binding_id
                  AND decision.task_plan_id=task_plan.task_plan_id
                  AND decision.primary_worker_run_id=worker.id
                 LEFT JOIN investigation_hypothesis_canonical_apply_receipts receipt
                   ON receipt.decision_id=decision.decision_id
                  AND receipt.operation_id=task_plan.operation_id
                  AND receipt.organization_id=task_plan.organization_id
                 LEFT JOIN hypothesis_generation_seals generation_seal
                   ON generation_seal.seal_id=receipt.generation_seal_id
                  AND generation_seal.generation_id=receipt.generation_id
                  AND generation_seal.controller_worker_run_id=worker.id
                 LEFT JOIN verification_admission_sets admission
                   ON admission.generation_id=receipt.generation_id
                  AND admission.operation_id=task_plan.operation_id
                  AND admission.stage_execution_id=task_plan.stage_execution_id
                  AND admission.stage_run_unit_id=task_plan.stage_run_unit_id
                  AND admission.scope_snapshot_id=authority.scope_snapshot_id
                  AND admission.organization_id=task_plan.organization_id
                  AND admission.status='sealed'
                WHERE task_plan.stage_team_plan_id=$1
                  AND task_plan.subject_kind='analysis_attempt'
                  AND task_plan.status='sealed'
                  AND item.created_by='server_seed'
                  AND (
                      item.stable_key LIKE 'leader:synthesis-recovery:%'
                      OR (
                          item.stable_key='leader:primary'
                          AND item.kind='investigation_primary'
                          AND jsonb_typeof(worker.checkpoint)='array'
                          AND (SELECT COUNT(*) FROM stage_worker_runs all_worker
                                WHERE all_worker.work_item_id=item.id)=1
                          AND (SELECT COUNT(*) FROM investigation_pentagi_pipeline_events synthesis
                                WHERE synthesis.task_plan_id=task_plan.task_plan_id
                                  AND synthesis.event_kind='primary_synthesis')=1
                          AND (SELECT COUNT(*) FROM jsonb_path_query(
                                worker.checkpoint,
                                'strict $.** ? (@.name == "submit_result")'))=1
                          AND unified_investigation_submit_result_v1(worker.checkpoint)
                                IS NOT NULL
                          AND (
                              (work.current_state='blocked'
                               AND latest.to_state='blocked'
                               AND latest.reason_code IN (
                                   'investigation_analysis_host_infrastructure',
                                   'investigation_analysis_host_authority_mismatch'
                               )
                               AND decision.decision_id IS NULL)
                              OR
                              (work.current_state='running'
                               AND latest.to_state='running'
                               AND latest.reason_code=
                                   'post_synthesis_analysis_primary_recovery.v1|'
                                   || event.event_sha256 || '|'
                                   || tool_truth_sha256(worker.checkpoint::TEXT)
                               AND (
                                   decision.decision_id IS NULL
                                   OR (
                                       receipt.apply_receipt_id IS NOT NULL
                                       AND generation_seal.seal_id IS NOT NULL
                                       AND admission.admission_set_id IS NOT NULL
                                       AND admission.member_count=generation_seal.member_count
                                   )
                               ))
                              OR
                              (work.current_state IN ('completed','residual')
                               AND latest.to_state=work.current_state
                               AND latest.reason_code IN (
                                   'canonical_generation_sealed_and_admitted',
                                   'canonical_generation_admitted_with_residuals'
                               )
                               AND receipt.apply_receipt_id IS NOT NULL
                               AND generation_seal.seal_id IS NOT NULL
                               AND admission.admission_set_id IS NOT NULL)
                          )
                      )
                  )
                  AND item.status='completed'
                  AND item.team_plan_id=task_plan.stage_team_plan_id
                  AND item.role=(SELECT leader_role FROM stage_team_plans
                                  WHERE id=task_plan.stage_team_plan_id)
                  AND item.required_for_barrier=FALSE
                  AND worker.status='passed'
                  AND worker.terminal_at IS NOT NULL
                  AND worker.lease_token IS NULL
                  AND worker.active_tool_call_id IS NULL"#,
        )
        .bind(plan.id)
        .fetch_all(pool)
        .await
        .context("load sealed Investigation synthesis completion witnesses")?
        .into_iter()
        .collect();
        let recoverable_target_intel_finalizer_worker_ids = if let (true, Some(final_submitter)) = (
            plan.stage_kind == "target_intel",
            plan.final_submitter_worker_run_id,
        ) {
            let rows = sqlx::query_scalar::<_, Uuid>(
                r#"SELECT review.controller_worker_run_id
                         FROM target_intel_goal_reviews review
                         JOIN target_intel_goal_epochs epoch
                           ON epoch.id=review.goal_epoch_id
                          AND epoch.operation_id=review.operation_id
                          AND epoch.organization_id=review.organization_id
                          AND epoch.team_plan_id=review.team_plan_id
                          AND epoch.status='sealed_for_review'
                         JOIN target_intel_goal_operation_contracts contract
                           ON contract.operation_id=review.operation_id
                          AND contract.goal_contract_sha256=review.operation_contract_sha256
                         JOIN stage_work_items controller_item
                           ON controller_item.id=review.controller_work_item_id
                          AND controller_item.team_plan_id=review.team_plan_id
                         JOIN stage_worker_runs controller_worker
                           ON controller_worker.id=review.controller_worker_run_id
                          AND controller_worker.work_item_id=controller_item.id
                          AND controller_worker.message_chain_id=review.controller_message_chain_id
                         JOIN stage_work_items reviewer_item
                           ON reviewer_item.id=review.reviewer_work_item_id
                          AND reviewer_item.team_plan_id=review.team_plan_id
                          AND reviewer_item.status='completed'
                         JOIN stage_worker_runs reviewer_worker
                           ON reviewer_worker.id=review.reviewer_worker_run_id
                          AND reviewer_worker.work_item_id=reviewer_item.id
                          AND reviewer_worker.status='passed'
                         JOIN stage_deliverable_submissions submission
                           ON submission.id=((review.completion_claim->>'completion_claim')::jsonb
                                              ->>'deliverable_submission_id')::uuid
                          AND submission.operation_id=review.operation_id
                          AND submission.stage_execution_id=review.stage_execution_id
                          AND submission.stage_run_unit_id=review.stage_run_unit_id
                          AND submission.organization_id=review.organization_id
                          AND submission.worker_run_id=review.controller_worker_run_id
                          AND submission.stage_kind='target_intel'
                        WHERE review.team_plan_id=$1
                          AND review.operation_id=$2
                          AND review.organization_id=$3
                          AND review.stage_execution_id=$4
                          AND review.stage_run_unit_id=$5
                          AND review.controller_worker_run_id=$6
                          AND review.status='pass'
                          AND review.verdict->>'decision'='PASS'
                          AND review.bundle_sha256 LIKE 'sha256:%'
                          AND review.verdict_sha256 LIKE 'sha256:%'"#,
            )
            .bind(plan.id)
            .bind(plan.operation_id)
            .bind(plan.organization_id)
            .bind(plan.stage_execution_id)
            .bind(plan.stage_run_unit_id)
            .bind(final_submitter)
            .fetch_all(pool)
            .await
            .context("load frozen Target Intel finalizer recovery witness")?;
            if rows.len() == 1 {
                rows.into_iter().collect()
            } else {
                HashSet::new()
            }
        } else {
            HashSet::new()
        };
        let recoverable_company_finalizer_worker_ids = if let (false, Some(final_submitter)) = (
            plan.stage_kind == "target_intel",
            plan.final_submitter_worker_run_id,
        ) {
            let rows = sqlx::query_scalar::<_, Uuid>(
                r#"SELECT submission.worker_run_id
                     FROM operation_state operation
                     JOIN stage_worker_runs worker
                       ON worker.id=$6
                      AND worker.operation_id=operation.operation_id
                      AND worker.stage_execution_id=$3
                      AND worker.stage_run_unit_id=$4
                      AND worker.organization_id=$2
                      AND worker.status='failed'
                      AND worker.terminal_at IS NOT NULL
                      AND worker.lease_token IS NULL
                      AND worker.active_tool_call_id IS NULL
                      AND worker.checkpoint #>> '{stage_team_execution_failure,code}'=
                          'stage_team_worker_lease_expired'
                     JOIN stage_deliverable_submissions submission
                       ON submission.operation_id=operation.operation_id
                      AND submission.stage_execution_id=$3
                      AND submission.stage_run_unit_id=$4
                      AND submission.organization_id=$2
                      AND submission.worker_run_id=worker.id
                      AND submission.stage_kind=$5
                      AND submission.attempt_epoch IS NOT NULL
                      AND submission.attempt_epoch<=worker.attempt_epoch
                      AND submission.lease_token IS NOT NULL
                     JOIN tool_calls tool
                       ON tool.id=submission.tool_call_record_id
                      AND tool.call_id=submission.tool_request_id
                      AND tool.name='submit_stage_deliverable'
                      AND tool.status='finished'
                      AND tool.operation_id=submission.operation_id
                      AND tool.stage_execution_id=submission.stage_execution_id
                      AND tool.stage_run_unit_id=submission.stage_run_unit_id
                      AND tool.worker_run_id=submission.worker_run_id
                      AND tool.organization_id=submission.organization_id
                      AND tool.attempt_epoch=submission.attempt_epoch
                      AND tool.lease_token=submission.lease_token
                      AND tool.result::jsonb->>'status'='accepted'
                      AND (tool.result::jsonb->>'deliverable_submission_id')::uuid=submission.id
                    WHERE operation.operation_id=$1
                      AND operation.superseded_by IS NULL
                      AND operation.current_stage=$5
                      AND operation.runtime_memory_contract='v2_only'
                    ORDER BY submission.submitted_at DESC,submission.id DESC
                    LIMIT 1"#,
            )
            .bind(plan.operation_id)
            .bind(plan.organization_id)
            .bind(plan.stage_execution_id)
            .bind(plan.stage_run_unit_id)
            .bind(&plan.stage_kind)
            .bind(final_submitter)
            .fetch_all(pool)
            .await
            .context("load ordinary Company finalizer recovery witness")?;
            if rows.len() == 1 {
                rows.into_iter().collect()
            } else {
                HashSet::new()
            }
        } else {
            HashSet::new()
        };
        authorities.insert(
            unit.id,
            StageTeamResumeAuthority {
                plan,
                work_items,
                outputs,
                completed_synthesis_primary_worker_ids,
                recoverable_company_finalizer_worker_ids,
                recoverable_target_intel_finalizer_worker_ids,
            },
        );
    }
    Ok(authorities)
}

fn canonical_percent(value: f64) -> String {
    let mut rendered = format!("{value:.6}");
    while rendered.contains('.') && rendered.ends_with('0') {
        rendered.pop();
    }
    if rendered.ends_with('.') {
        rendered.pop();
    }
    rendered
}

/// Resolve the CLI's root/descendant set once, before operation creation. A
/// descendant with missing/below-threshold ownership is excluded together with
/// its subtree; later stage execution never re-reads the mutable org tree.
pub(crate) fn build_cli_runtime_scope(
    organizations: &[Organization],
    root_organization_id: Uuid,
    include_subsidiaries: bool,
    subsidiary_threshold: u8,
) -> Result<CliRuntimeScope> {
    let root = organizations
        .iter()
        .find(|organization| organization.id == root_organization_id)
        .ok_or_else(|| anyhow!("CLI scope root organization is missing"))?;
    let mut children: HashMap<Uuid, Vec<&Organization>> = HashMap::new();
    for organization in organizations {
        if let Some(parent_id) = organization.parent_id {
            children.entry(parent_id).or_default().push(organization);
        }
    }
    for values in children.values_mut() {
        values.sort_by(|left, right| {
            left.sort_order
                .cmp(&right.sort_order)
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.id.cmp(&right.id))
        });
    }

    let mut units = vec![CliRuntimeScopeUnit {
        organization_id: root.id,
        parent_organization_id: None,
        organization_name: root.name.clone(),
        depth: 0,
        ordinal: 0,
        ownership_percent: None,
        approval_source: serde_json::json!({
            "kind": "cli_flags",
            "include_subsidiaries": include_subsidiaries,
            "subsidiary_threshold": subsidiary_threshold,
        }),
    }];
    if include_subsidiaries {
        let mut queue = VecDeque::from([(root.id, 0_i32)]);
        let mut selected = HashSet::from([root.id]);
        while let Some((parent_id, parent_depth)) = queue.pop_front() {
            for child in children.get(&parent_id).into_iter().flatten() {
                let Some(percent) = ownership_percent(child) else {
                    continue;
                };
                if percent < f64::from(subsidiary_threshold) || !selected.insert(child.id) {
                    continue;
                }
                let ownership_percent = canonical_percent(percent);
                units.push(CliRuntimeScopeUnit {
                    organization_id: child.id,
                    parent_organization_id: Some(parent_id),
                    organization_name: child.name.clone(),
                    depth: parent_depth + 1,
                    ordinal: units.len() as i32,
                    ownership_percent: Some(ownership_percent.clone()),
                    approval_source: serde_json::json!({
                        "kind": "cli_flags",
                        "include_subsidiaries": true,
                        "subsidiary_threshold": subsidiary_threshold,
                        "ownership_percent": ownership_percent,
                    }),
                });
                queue.push_back((child.id, parent_depth + 1));
            }
        }
    }
    Ok(CliRuntimeScope {
        root_organization_id,
        include_subsidiaries,
        subsidiary_threshold,
        units,
    })
}

/// Aggregate a completed V2-writing CLI run from relational truth. Exactly one
/// task/operation is allowed for the session; each report row is joined to the
/// immutable scope snapshot and the selected stage execution.
fn select_cli_report_execution(
    executions: &[golish_db::repo::stage_runs::StageRunRow],
    stage: StageKind,
) -> Result<&golish_db::repo::stage_runs::StageRunRow> {
    let active = executions
        .iter()
        .filter(|execution| execution.status == "started")
        .collect::<Vec<_>>();
    match active.as_slice() {
        [execution] if execution.stage_kind == stage.as_str() => return Ok(*execution),
        [execution] => {
            return Err(anyhow!(
                "V2 CLI active execution is {}, expected {}",
                execution.stage_kind,
                stage.as_str()
            ));
        }
        [] => {}
        _ => {
            return Err(anyhow!(
                "V2 CLI requires at most one active stage execution, found {}",
                active.len()
            ));
        }
    }

    executions
        .iter()
        .filter(|execution| {
            execution.stage_kind == stage.as_str()
                && execution.status == "completed"
                && execution.completed_at.is_some()
        })
        .max_by_key(|execution| (execution.started_at, execution.id))
        .ok_or_else(|| {
            anyhow!(
                "V2 CLI completed flow has no successful {} stage execution",
                stage.as_str()
            )
        })
}

pub(crate) async fn load_cli_report(
    pool: &sqlx::PgPool,
    session_id: Uuid,
    stage: StageKind,
) -> Result<RuntimeV2CliReport> {
    let tasks = golish_db::repo::tasks::list_by_session(pool, session_id)
        .await
        .context("load CLI session operations")?;
    let [task] = tasks.as_slice() else {
        return Err(anyhow!(
            "V2 CLI expected exactly one operation, found {}",
            tasks.len()
        ));
    };
    let operation = golish_db::repo::operation_state::get(pool, task.id)
        .await
        .context("load V2 CLI operation")?
        .ok_or_else(|| anyhow!("V2 CLI operation_state is missing"))?;
    let contract = persisted_contract(&operation.runtime_memory_contract)?;
    if !contract_writes_v2(contract) {
        return Err(anyhow!("LegacyV1 operation cannot use V2 CLI report"));
    }
    let scope = golish_db::repo::operation_org_scope::load_for_operation(pool, task.id)
        .await
        .context("load frozen CLI organization scope")?
        .ok_or_else(|| anyhow!("V2 CLI frozen scope snapshot is missing"))?;
    if scope.snapshot.sealed_at.is_none() {
        return Err(anyhow!("V2 CLI scope snapshot is not sealed"));
    }
    let executions = golish_db::repo::stage_runs::list_for_operation(pool, task.id)
        .await
        .context("load V2 CLI stage executions")?;
    let execution = select_cli_report_execution(&executions, stage)?;
    let stage_units =
        golish_db::repo::stage_run_units::list_for_execution(pool, task.id, execution.id)
            .await
            .context("load V2 CLI stage units")?;
    if stage_units.iter().any(|unit| {
        unit.operation_id != task.id
            || unit.stage_execution_id != execution.id
            || unit.scope_snapshot_id != scope.snapshot.id
    }) {
        return Err(anyhow!(
            "V2 CLI stage unit identity crossed operation/snapshot"
        ));
    }
    let workers =
        golish_db::repo::stage_worker_runs::list_for_execution(pool, task.id, execution.id)
            .await
            .context("load V2 CLI stage workers")?;
    let stage_team_authorities = load_stage_team_resume_authorities(pool, &stage_units).await?;
    let mut workers_by_unit: HashMap<Uuid, Vec<_>> = HashMap::new();
    for worker in &workers {
        workers_by_unit
            .entry(worker.stage_run_unit_id)
            .or_default()
            .push(worker);
    }
    let by_org = stage_units
        .iter()
        .map(|unit| (unit.organization_id, unit))
        .collect::<HashMap<_, _>>();
    let outcomes = scope
        .units
        .iter()
        .map(|scope_unit| {
            let decision = match by_org.get(&scope_unit.organization_id) {
                Some(unit) => {
                    let status = RuntimeStageUnitStatus::try_parse(&unit.status);
                    let worker_rows = workers_by_unit.get(&unit.id).cloned().unwrap_or_default();
                    let primary_worker = match select_stage_team_primary_worker(
                        unit,
                        &worker_rows,
                        stage_team_authorities.get(&unit.id),
                    ) {
                        Ok(worker) => worker,
                        Err(error) => {
                            return OrgRunOutcome {
                                org_id: scope_unit.organization_id,
                                org_name: scope_unit.organization_name_at_freeze.clone(),
                                status: OrgRunStatus::Failed,
                                detail: Some(format!(
                                    "V2 stage Unit worker authority rejected: {error}"
                                )),
                            };
                        }
                    };
                    let sealed_stage_team_completed_primary =
                        primary_worker.is_some_and(|worker| {
                            stage_team_authorities
                                .get(&unit.id)
                                .is_some_and(|authority| {
                                    authority
                                        .completed_synthesis_primary_worker_ids
                                        .contains(&worker.id)
                                })
                        });
                    let recoverable_terminalized_finalizer = primary_worker.is_some_and(|worker| {
                        stage_team_authorities
                            .get(&unit.id)
                            .is_some_and(|authority| {
                                exact_target_intel_finalizer_recovery_preflight(worker, authority)
                                    || exact_company_finalizer_recovery_preflight(worker, authority)
                            })
                    });
                    let worker = primary_worker.and_then(|worker| {
                        RuntimeWorkerStatus::try_parse(&worker.status).map(|status| {
                            ResumeWorkerSnapshot {
                                id: worker.id,
                                operation_id: worker.operation_id,
                                stage_execution_id: worker.stage_execution_id,
                                stage_run_unit_id: worker.stage_run_unit_id,
                                organization_id: worker.organization_id,
                                status,
                                lease_expires_at: worker.lease_expires_at,
                                active_tool_call_id: worker.active_tool_call_id,
                            }
                        })
                    });
                    let seeded_stage_team_preclaim = worker.is_none()
                        && stage_team_authorities.contains_key(&unit.id)
                        && unit.specialist.is_some();
                    match status {
                        Some(status) => classify_runtime_v2_resume(&RuntimeV2ResumeSnapshot {
                            operation_id: task.id,
                            active_stage_execution_id: execution.id,
                            // Report selection has already proven exactly one
                            // target execution. A successful completed CLI flow
                            // correctly has zero *active* executions, but the
                            // Unit/Worker terminal classifier still needs the
                            // selected execution cardinality.
                            active_stage_execution_count: 1,
                            operation_superseded: operation.superseded_by.is_some(),
                            current_stage_is_scoping: stage == StageKind::Scoping,
                            scope_sealed: scope.snapshot.sealed_at.is_some(),
                            scope_unit_count: scope.units.len(),
                            unit: Some(ResumeUnitSnapshot {
                                id: unit.id,
                                operation_id: unit.operation_id,
                                stage_execution_id: unit.stage_execution_id,
                                organization_id: unit.organization_id,
                                is_root: unit.organization_id
                                    == scope.snapshot.root_organization_id,
                                specialist: unit.specialist.clone(),
                                status,
                            }),
                            worker,
                            seeded_stage_team_preclaim,
                            sealed_stage_team_completed_primary,
                            recoverable_terminalized_finalizer,
                            now: Utc::now(),
                        }),
                        None => RuntimeV2ResumeDecision::Reject(
                            RuntimeV2ResumeReject::InvalidWorkerState,
                        ),
                    }
                }
                None => RuntimeV2ResumeDecision::Reject(RuntimeV2ResumeReject::MissingUnit),
            };
            let (status, detail) = match decision {
                RuntimeV2ResumeDecision::AlreadyPassed => (OrgRunStatus::Passed, None),
                RuntimeV2ResumeDecision::ResumeScoping
                | RuntimeV2ResumeDecision::WaitForLease
                | RuntimeV2ResumeDecision::RequeueExpiredWorker
                | RuntimeV2ResumeDecision::RecoveryRequired
                | RuntimeV2ResumeDecision::ResumeSpecialist
                | RuntimeV2ResumeDecision::ResumeRootUnit => (
                    OrgRunStatus::Blocked,
                    Some(format!("V2 stage unit is nonterminal: {decision:?}")),
                ),
                RuntimeV2ResumeDecision::Reject(reason) => (
                    OrgRunStatus::Failed,
                    Some(format!("V2 stage unit rejected: {reason:?}")),
                ),
            };
            OrgRunOutcome {
                org_id: scope_unit.organization_id,
                org_name: scope_unit.organization_name_at_freeze.clone(),
                status,
                detail,
            }
        })
        .collect();
    Ok(RuntimeV2CliReport {
        operation_id: task.id,
        scope_unit_count: scope.units.len(),
        stage_unit_count: stage_units.len(),
        fleet: FleetReport { outcomes },
    })
}

/// Validate the exact relational resume authority for one selected operation.
/// This is deliberately stricter than merely finding a Unit row: execution,
/// frozen scope, every Unit/Worker identity, bound message chain and active
/// tool fence must form one complete source.
pub(crate) async fn load_relational_resume_authority(
    pool: &sqlx::PgPool,
    session_id: Uuid,
    operation: &golish_db::repo::operation_state::OperationStateRow,
    stage: StageKind,
) -> Result<RuntimeV2ResumeAuthority> {
    let runtime_contract = persisted_contract(&operation.runtime_memory_contract)?;
    if operation.project_scope_id.is_none() {
        return Err(relational_resume_incomplete(
            "frozen project scope is missing",
        ));
    }
    anyhow::ensure!(
        operation.superseded_by.is_none(),
        "relational V2 resume operation is superseded"
    );
    let executions = golish_db::repo::stage_runs::list_for_operation(pool, operation.operation_id)
        .await
        .context("load relational V2 stage executions")?;
    let active = executions
        .iter()
        .filter(|execution| execution.status == "started")
        .collect::<Vec<_>>();
    let execution = match active.as_slice() {
        [execution] => *execution,
        [] => {
            return Err(relational_resume_incomplete(
                "active stage execution is missing",
            ))
        }
        _ => {
            return Err(anyhow!(
                "relational V2 resume requires one active execution, found {}",
                active.len()
            ))
        }
    };
    anyhow::ensure!(
        execution.operation_id == operation.operation_id
            && execution.stage_kind == operation.current_stage
            && execution.stage_kind == stage.as_str(),
        "relational V2 active execution does not match the operation cursor"
    );
    let units = golish_db::repo::stage_run_units::list_for_execution(
        pool,
        operation.operation_id,
        execution.id,
    )
    .await
    .context("load relational V2 stage units")?;
    let workers = golish_db::repo::stage_worker_runs::list_for_execution(
        pool,
        operation.operation_id,
        execution.id,
    )
    .await
    .context("load relational V2 workers")?;
    let scope =
        golish_db::repo::operation_org_scope::load_for_operation(pool, operation.operation_id)
            .await
            .context("load relational V2 frozen scope")?;

    if stage == StageKind::Scoping {
        anyhow::ensure!(
            scope.is_none() && units.is_empty() && workers.is_empty(),
            "relational V2 scoping resume is not the exact pre-freeze shape"
        );
        let organization_id = operation.engagement_org_id.ok_or_else(|| {
            anyhow!("relational V2 scoping resume has no persisted engagement organization")
        })?;
        let decision = classify_runtime_v2_resume(&RuntimeV2ResumeSnapshot {
            operation_id: operation.operation_id,
            active_stage_execution_id: execution.id,
            active_stage_execution_count: active.len(),
            operation_superseded: false,
            current_stage_is_scoping: true,
            scope_sealed: false,
            scope_unit_count: 0,
            unit: None,
            worker: None,
            seeded_stage_team_preclaim: false,
            sealed_stage_team_completed_primary: false,
            recoverable_terminalized_finalizer: false,
            now: Utc::now(),
        });
        anyhow::ensure!(
            decision == RuntimeV2ResumeDecision::ResumeScoping,
            "relational V2 scoping authority rejected: {decision:?}"
        );
        return Ok(RuntimeV2ResumeAuthority {
            active_stage_execution_id: execution.id,
            organization_id,
        });
    }

    let scope = scope
        .ok_or_else(|| relational_resume_incomplete("frozen organization scope is missing"))?;
    anyhow::ensure!(
        scope.snapshot.sealed_at.is_some()
            && Some(scope.snapshot.project_scope_id) == operation.project_scope_id
            && operation.engagement_org_id == Some(scope.snapshot.root_organization_id)
            && !scope.units.is_empty(),
        "relational V2 frozen scope does not match operation authority"
    );
    if let Some(boundary) =
        exact_stage_slice_boundary(operation, execution, units.len(), workers.len())
    {
        let source = executions
            .iter()
            .find(|candidate| candidate.id == boundary.source_stage_execution_id)
            .ok_or_else(|| {
                relational_resume_incomplete("stage-slice source execution is missing")
            })?;
        anyhow::ensure!(
            source.operation_id == operation.operation_id
                && source.stage_kind == boundary.source_stage.as_str()
                && source.status == "completed"
                && source
                    .completed_at
                    .is_some_and(|completed_at| completed_at <= execution.started_at)
                && executions
                    .iter()
                    .filter(|candidate| candidate.id != execution.id)
                    .max_by_key(|candidate| (candidate.started_at, candidate.id))
                    .is_some_and(|latest| latest.id == source.id),
            "relational V2 stage-slice boundary source is not the latest completed execution"
        );
        return Ok(RuntimeV2ResumeAuthority {
            active_stage_execution_id: execution.id,
            organization_id: scope.snapshot.root_organization_id,
        });
    }
    let configured_specialist = golish_agent_kit::harness::load_embedded_stage_spec(stage)
        .context("load stage spec for relational V2 resume")?
        .specialist
        .filter(|value| !value.trim().is_empty());
    let attack_contract = match stage {
        StageKind::AttackCandidate | StageKind::Verification => {
            golish_db::repo::operation_state::get_attack_execution_contract(
                pool,
                operation.operation_id,
            )
            .await
            .context("load frozen attack contract for relational V2 resume")?
            .ok_or_else(|| anyhow!("relational V2 frozen attack contract is missing"))?
        }
        _ => AttackExecutionContract::Legacy,
    };
    let specialist = effective_resume_specialist(
        stage,
        configured_specialist.as_deref(),
        runtime_contract,
        attack_contract,
    );
    if exact_root_stage_preclaim(
        specialist.as_deref(),
        &execution.status,
        units.len(),
        workers.len(),
    ) {
        return Ok(RuntimeV2ResumeAuthority {
            active_stage_execution_id: execution.id,
            organization_id: scope.snapshot.root_organization_id,
        });
    }
    let expected_unit_count = if specialist.is_some() {
        scope.units.len()
    } else {
        1
    };
    if units.len() < expected_unit_count {
        return Err(relational_resume_incomplete(format!(
            "stage Unit rows are missing: expected {expected_unit_count}, found {}",
            units.len()
        )));
    }
    anyhow::ensure!(
        units.len() == expected_unit_count,
        "relational V2 stage Unit cardinality exceeds frozen scope"
    );
    let stage_team_authorities = load_stage_team_resume_authorities(pool, &units).await?;
    let chains = golish_db::repo::message_chains::list_by_session(pool, session_id)
        .await
        .context("load relational V2 message chains")?;
    let now = Utc::now();
    let mut preclaim_unit_ids = HashSet::new();

    for unit in &units {
        let scope_member = scope
            .units
            .iter()
            .find(|member| member.organization_id == unit.organization_id)
            .ok_or_else(|| anyhow!("relational V2 Unit is outside the frozen scope"))?;
        anyhow::ensure!(
            unit.operation_id == operation.operation_id
                && unit.stage_execution_id == execution.id
                && unit.scope_snapshot_id == scope.snapshot.id
                && unit.stage_kind == stage.as_str(),
            "relational V2 Unit identity crossed execution/snapshot/stage"
        );
        let unit_status = RuntimeStageUnitStatus::try_parse(&unit.status)
            .ok_or_else(|| relational_resume_incomplete("stage Unit status cannot be decoded"))?;
        let unit_workers = workers
            .iter()
            .filter(|worker| worker.stage_run_unit_id == unit.id)
            .collect::<Vec<_>>();
        let worker_snapshot = match specialist.as_deref() {
            Some(expected_specialist) => {
                anyhow::ensure!(
                    unit.specialist.as_deref() == Some(expected_specialist),
                    "relational V2 Unit specialist identity drifted"
                );
                let team_authority = stage_team_authorities.get(&unit.id);
                let worker = select_stage_team_primary_worker(unit, &unit_workers, team_authority)?;
                let seeded_stage_team_preclaim = worker.is_none()
                    && (team_authority.is_some()
                        || exact_replacement_preclaim(operation, execution, unit));
                if worker.is_none() && !seeded_stage_team_preclaim {
                    return Err(relational_resume_incomplete(format!(
                        "Worker row is missing for Unit {}",
                        unit.id
                    )));
                }
                let expected_chain_agent = resume_worker_chain_agent(expected_specialist)
                    .ok_or_else(|| {
                        anyhow!(
                            "relational V2 specialist '{expected_specialist}' has no durable chain agent"
                        )
                    })?;
                let mut worker_status = None;
                for bound_worker in &unit_workers {
                    anyhow::ensure!(
                        bound_worker.operation_id == operation.operation_id
                            && bound_worker.stage_execution_id == execution.id
                            && bound_worker.stage_run_unit_id == unit.id
                            && bound_worker.organization_id == unit.organization_id
                            && (team_authority.is_some()
                                || bound_worker.specialist == expected_specialist),
                        "relational V2 Worker identity crossed Unit/operation"
                    );
                    let bound_status = RuntimeWorkerStatus::try_parse(&bound_worker.status)
                        .ok_or_else(|| {
                            relational_resume_incomplete("Worker status cannot be decoded")
                        })?;
                    if worker.is_some_and(|worker| bound_worker.id == worker.id) {
                        worker_status = Some(bound_status);
                    } else if bound_status == RuntimeWorkerStatus::Running
                        && bound_worker
                            .lease_expires_at
                            .is_some_and(|expires_at| expires_at > now)
                    {
                        return Err(anyhow::Error::new(RelationalResumeBusy));
                    }
                    match bound_worker.message_chain_id {
                        Some(chain_id) => {
                            let chain = match chains.iter().find(|chain| chain.id == chain_id) {
                                Some(chain) => chain,
                                None => {
                                    let exists_outside_session =
                                        golish_db::repo::message_chains::exists_by_id(
                                            pool, chain_id,
                                        )
                                        .await
                                        .context(
                                            "classify relational V2 message-chain ownership",
                                        )?;
                                    if exists_outside_session {
                                        return Err(anyhow!(
                                            "relational V2 message chain crossed session identity"
                                        ));
                                    }
                                    return Err(relational_resume_incomplete(
                                        "bound message chain row is missing",
                                    ));
                                }
                            };
                            anyhow::ensure!(
                                chain.session_id == session_id
                                    && chain.task_id == Some(operation.operation_id)
                                    && chain.agent == expected_chain_agent,
                                "relational V2 message chain crossed session/task/agent scope"
                            );
                            let chain_body = chain.chain.as_ref().ok_or_else(|| {
                                relational_resume_incomplete("bound message chain body is missing")
                            })?;
                            decode_relational_message_chain(chain_body)?;
                        }
                        None => anyhow::ensure!(
                            bound_status == RuntimeWorkerStatus::Queued,
                            "relational V2 non-queued Worker has no bound message chain"
                        ),
                    }
                    if let Some(active_tool_call_id) = bound_worker.active_tool_call_id {
                        let exact_active_tool =
                            golish_db::repo::tool_calls::has_exact_active_worker_fence(
                                pool,
                                active_tool_call_id,
                                bound_worker.id,
                                bound_worker.operation_id,
                                bound_worker.stage_execution_id,
                                bound_worker.stage_run_unit_id,
                                bound_worker.organization_id,
                                bound_worker.attempt_epoch,
                                bound_worker.lease_token,
                            )
                            .await
                            .context("validate relational V2 active tool fence")?;
                        anyhow::ensure!(
                            exact_active_tool,
                            "relational V2 active tool fence is stale or cross-owned"
                        );
                    }
                }
                match worker {
                    Some(worker) => {
                        let worker_status = worker_status.ok_or_else(|| {
                            relational_resume_incomplete("Unit owner Worker status is missing")
                        })?;
                        Some(ResumeWorkerSnapshot {
                            id: worker.id,
                            operation_id: worker.operation_id,
                            stage_execution_id: worker.stage_execution_id,
                            stage_run_unit_id: worker.stage_run_unit_id,
                            organization_id: worker.organization_id,
                            status: worker_status,
                            lease_expires_at: worker.lease_expires_at,
                            active_tool_call_id: worker.active_tool_call_id,
                        })
                    }
                    None => {
                        if !unit_workers.is_empty() {
                            anyhow::ensure!(
                                team_authority.is_some() && unit.status == "running",
                                "fresh or replacement pre-claim unexpectedly owns Worker rows"
                            );
                        }
                        preclaim_unit_ids.insert(unit.id);
                        None
                    }
                }
            }
            None => {
                anyhow::ensure!(
                    unit.organization_id == scope.snapshot.root_organization_id
                        && scope_member.role == "root"
                        && unit.specialist.is_none()
                        && unit_workers.is_empty()
                        && workers.is_empty(),
                    "relational V2 root-only Unit shape is invalid"
                );
                None
            }
        };
        let seeded_stage_team_preclaim = worker_snapshot.is_none()
            && (stage_team_authorities.contains_key(&unit.id)
                || exact_replacement_preclaim(operation, execution, unit))
            && unit.specialist.is_some();
        let sealed_stage_team_completed_primary = worker_snapshot.as_ref().is_some_and(|worker| {
            stage_team_authorities
                .get(&unit.id)
                .is_some_and(|authority| {
                    authority
                        .completed_synthesis_primary_worker_ids
                        .contains(&worker.id)
                })
        });
        let recoverable_terminalized_finalizer = worker_snapshot.as_ref().is_some_and(|worker| {
            stage_team_authorities
                .get(&unit.id)
                .is_some_and(|authority| {
                    unit_workers
                        .iter()
                        .find(|candidate| candidate.id == worker.id)
                        .is_some_and(|candidate| {
                            exact_target_intel_finalizer_recovery_preflight(candidate, authority)
                                || exact_company_finalizer_recovery_preflight(candidate, authority)
                        })
                })
        });
        let decision = classify_runtime_v2_resume(&RuntimeV2ResumeSnapshot {
            operation_id: operation.operation_id,
            active_stage_execution_id: execution.id,
            active_stage_execution_count: active.len(),
            operation_superseded: false,
            current_stage_is_scoping: false,
            scope_sealed: true,
            scope_unit_count: scope.units.len(),
            unit: Some(ResumeUnitSnapshot {
                id: unit.id,
                operation_id: unit.operation_id,
                stage_execution_id: unit.stage_execution_id,
                organization_id: unit.organization_id,
                is_root: unit.organization_id == scope.snapshot.root_organization_id,
                specialist: unit.specialist.clone(),
                status: unit_status,
            }),
            worker: worker_snapshot,
            seeded_stage_team_preclaim,
            sealed_stage_team_completed_primary,
            recoverable_terminalized_finalizer,
            now,
        });
        match decision {
            RuntimeV2ResumeDecision::WaitForLease => {
                return Err(anyhow::Error::new(RelationalResumeBusy));
            }
            RuntimeV2ResumeDecision::Reject(
                RuntimeV2ResumeReject::SupersededOperation
                | RuntimeV2ResumeReject::ActiveExecutionCardinality
                | RuntimeV2ResumeReject::CrossOperationIdentity,
            ) => {
                anyhow::bail!("relational V2 runtime identity is invalid: {decision:?}");
            }
            RuntimeV2ResumeDecision::Reject(_) => {
                return Err(relational_resume_incomplete(format!(
                    "runtime state cannot be decoded: {decision:?}"
                )));
            }
            _ => {}
        }
    }
    let minimum_worker_count = if specialist.is_some() {
        units.len().saturating_sub(preclaim_unit_ids.len())
    } else {
        0
    };
    if workers.len() < minimum_worker_count {
        return Err(relational_resume_incomplete(format!(
            "Worker rows are missing: expected at least {minimum_worker_count}, found {}",
            workers.len()
        )));
    }
    let unit_ids = units.iter().map(|unit| unit.id).collect::<HashSet<_>>();
    anyhow::ensure!(
        workers
            .iter()
            .all(|worker| unit_ids.contains(&worker.stage_run_unit_id)),
        "relational V2 Worker references a foreign Unit"
    );

    Ok(RuntimeV2ResumeAuthority {
        active_stage_execution_id: execution.id,
        organization_id: scope.snapshot.root_organization_id,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResumeUnitSnapshot {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub organization_id: Uuid,
    pub is_root: bool,
    pub specialist: Option<String>,
    pub status: RuntimeStageUnitStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResumeWorkerSnapshot {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub status: RuntimeWorkerStatus,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub active_tool_call_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeV2ResumeSnapshot {
    pub operation_id: Uuid,
    pub active_stage_execution_id: Uuid,
    pub active_stage_execution_count: usize,
    pub operation_superseded: bool,
    pub current_stage_is_scoping: bool,
    pub scope_sealed: bool,
    pub scope_unit_count: usize,
    pub unit: Option<ResumeUnitSnapshot>,
    pub worker: Option<ResumeWorkerSnapshot>,
    pub seeded_stage_team_preclaim: bool,
    pub sealed_stage_team_completed_primary: bool,
    pub recoverable_terminalized_finalizer: bool,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeV2ResumeReject {
    SupersededOperation,
    ActiveExecutionCardinality,
    InvalidScopingShape,
    ScopeNotSealed,
    MissingUnit,
    CrossOperationIdentity,
    WorkerRequired,
    UnexpectedWorker,
    InvalidWorkerState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeV2ResumeDecision {
    ResumeScoping,
    WaitForLease,
    RequeueExpiredWorker,
    RecoveryRequired,
    ResumeSpecialist,
    ResumeRootUnit,
    AlreadyPassed,
    Reject(RuntimeV2ResumeReject),
}

pub(crate) fn classify_runtime_v2_resume(
    snapshot: &RuntimeV2ResumeSnapshot,
) -> RuntimeV2ResumeDecision {
    use RuntimeStageUnitStatus as Unit;
    use RuntimeV2ResumeDecision as Decision;
    use RuntimeV2ResumeReject as Reject;
    use RuntimeWorkerStatus as Worker;

    if snapshot.operation_superseded {
        return Decision::Reject(Reject::SupersededOperation);
    }
    if snapshot.active_stage_execution_count != 1 {
        return Decision::Reject(Reject::ActiveExecutionCardinality);
    }
    if snapshot.current_stage_is_scoping {
        return if !snapshot.scope_sealed
            && snapshot.scope_unit_count == 0
            && snapshot.unit.is_none()
            && snapshot.worker.is_none()
        {
            Decision::ResumeScoping
        } else {
            Decision::Reject(Reject::InvalidScopingShape)
        };
    }
    if !snapshot.scope_sealed || snapshot.scope_unit_count == 0 {
        return Decision::Reject(Reject::ScopeNotSealed);
    }
    let Some(unit) = snapshot.unit.as_ref() else {
        return Decision::Reject(Reject::MissingUnit);
    };
    if unit.operation_id != snapshot.operation_id
        || unit.stage_execution_id != snapshot.active_stage_execution_id
    {
        return Decision::Reject(Reject::CrossOperationIdentity);
    }
    let specialist = unit
        .specialist
        .as_deref()
        .filter(|specialist| !specialist.trim().is_empty());
    if specialist.is_none() {
        if !unit.is_root {
            return Decision::Reject(Reject::MissingUnit);
        }
        if snapshot.worker.is_some() {
            return Decision::Reject(Reject::UnexpectedWorker);
        }
        if unit.status == Unit::Passed {
            return Decision::AlreadyPassed;
        }
        return if matches!(
            unit.status,
            Unit::Queued | Unit::Running | Unit::GateBlocked
        ) {
            Decision::ResumeRootUnit
        } else {
            Decision::Reject(Reject::InvalidWorkerState)
        };
    }

    let Some(worker) = snapshot.worker.as_ref() else {
        return if snapshot.seeded_stage_team_preclaim
            && matches!(unit.status, Unit::Queued | Unit::Running)
        {
            Decision::ResumeSpecialist
        } else {
            Decision::Reject(Reject::WorkerRequired)
        };
    };
    if worker.id.is_nil()
        || worker.operation_id != snapshot.operation_id
        || worker.stage_execution_id != snapshot.active_stage_execution_id
        || worker.stage_run_unit_id != unit.id
        || worker.organization_id != unit.organization_id
    {
        return Decision::Reject(Reject::CrossOperationIdentity);
    }
    if unit.status == Unit::Passed {
        return if worker.status == Worker::Passed {
            Decision::AlreadyPassed
        } else {
            Decision::Reject(Reject::InvalidWorkerState)
        };
    }
    if snapshot.sealed_stage_team_completed_primary
        && unit.status == Unit::Running
        && worker.status == Worker::Passed
    {
        return Decision::ResumeSpecialist;
    }
    if snapshot.recoverable_terminalized_finalizer
        && unit.status == Unit::Running
        && worker.status == Worker::Failed
        && worker.lease_expires_at.is_none()
        && worker.active_tool_call_id.is_none()
    {
        return Decision::ResumeSpecialist;
    }

    match worker.status {
        Worker::Running => match worker.lease_expires_at {
            Some(expires_at) if expires_at > snapshot.now => Decision::WaitForLease,
            Some(_) if worker.active_tool_call_id.is_some() => Decision::RecoveryRequired,
            Some(_) => Decision::RequeueExpiredWorker,
            None => Decision::Reject(Reject::InvalidWorkerState),
        },
        Worker::RecoveryRequired => Decision::RecoveryRequired,
        Worker::Queued | Worker::GateBlocked | Worker::WaitingBackground
            if worker.active_tool_call_id.is_none()
                && matches!(
                    unit.status,
                    Unit::Queued | Unit::Running | Unit::GateBlocked
                ) =>
        {
            Decision::ResumeSpecialist
        }
        _ => Decision::Reject(Reject::InvalidWorkerState),
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    fn stage_execution(
        id: Uuid,
        operation_id: Uuid,
        stage: StageKind,
        status: &str,
        started_at: DateTime<Utc>,
    ) -> golish_db::repo::stage_runs::StageRunRow {
        golish_db::repo::stage_runs::StageRunRow {
            id,
            operation_id,
            stage_kind: stage.as_str().to_string(),
            started_at,
            completed_at: (status == "completed").then_some(started_at + Duration::seconds(1)),
            status: status.to_string(),
            active_sprint_contract_id: None,
        }
    }

    #[test]
    fn cli_report_selects_completed_terminal_execution_after_success() {
        let operation_id = Uuid::new_v4();
        let now = Utc::now();
        let scoping = stage_execution(
            Uuid::new_v4(),
            operation_id,
            StageKind::Scoping,
            "completed",
            now - Duration::minutes(2),
        );
        let target_intel = stage_execution(
            Uuid::new_v4(),
            operation_id,
            StageKind::TargetIntel,
            "completed",
            now - Duration::minutes(1),
        );

        let executions = [scoping, target_intel.clone()];
        let selected = select_cli_report_execution(&executions, StageKind::TargetIntel)
            .expect("successful terminal CLI report selection");

        assert_eq!(selected.id, target_intel.id);
        assert_eq!(selected.status, "completed");
    }

    #[test]
    fn preferred_legacy_fallback_requires_a_typed_structural_gap() {
        let incomplete = relational_resume_incomplete("missing Worker row");
        assert!(relational_resume_error_allows_legacy_fallback(&incomplete));
        assert!(!relational_resume_error_allows_legacy_fallback(
            &anyhow::Error::new(RelationalResumeBusy)
        ));
        assert!(!relational_resume_error_allows_legacy_fallback(
            &anyhow::anyhow!("cross-operation identity")
        ));
    }

    #[test]
    fn relational_message_chain_requires_real_rig_message_decode() {
        assert!(decode_relational_message_chain(&serde_json::json!([
            {"not_a_rig_message": true}
        ]))
        .is_err());
        assert!(decode_relational_message_chain(&serde_json::json!([])).is_ok());
    }

    fn organization(
        id: Uuid,
        name: &str,
        parent_id: Option<Uuid>,
        ownership: Option<f64>,
    ) -> Organization {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "project_path": "/tmp/runtime-v2-cli",
            "name": name,
            "parent_id": parent_id,
            "description": "",
            "owner": "",
            "sort_order": 0,
            "intel": ownership.map(|value| serde_json::json!({
                "asset_intel_discovery": {"ownershipPercent": value}
            })).unwrap_or_else(|| serde_json::json!({})),
            "created_at": "2026-07-13T00:00:00Z",
            "updated_at": "2026-07-13T00:00:00Z"
        }))
        .expect("organization fixture")
    }

    fn stage_team_resume_unit() -> StageRunUnitRow {
        let now = Utc::now();
        StageRunUnitRow {
            id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            stage_execution_id: Uuid::new_v4(),
            scope_snapshot_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            stage_kind: StageKind::Enumeration.as_str().to_string(),
            generation: 1,
            specialist: Some("enumerator".to_string()),
            status: "passed".to_string(),
            gate_attempt: 0,
            pass_watermark: serde_json::json!({}),
            row_version: 2,
            started_at: Some(now),
            updated_at: now,
            terminal_at: Some(now),
        }
    }

    fn stage_team_resume_plan(unit: &StageRunUnitRow) -> StageTeamPlanRow {
        let now = Utc::now();
        StageTeamPlanRow {
            id: Uuid::new_v4(),
            operation_id: unit.operation_id,
            stage_execution_id: unit.stage_execution_id,
            stage_run_unit_id: unit.id,
            scope_snapshot_id: unit.scope_snapshot_id,
            organization_id: unit.organization_id,
            stage_kind: unit.stage_kind.clone(),
            unit_generation: unit.generation,
            schema_version: 1,
            plan_version: 1,
            plan_hash: "sha256:test".to_string(),
            leader_role: "company_stage_controller".to_string(),
            aggregator_kind: "worker".to_string(),
            aggregator_role: Some("company_stage_controller".to_string()),
            allowed_worker_roles: serde_json::json!(["company_stage_controller", "enumerator"]),
            max_workers_total: 35,
            max_workers_active: 3,
            dynamic_requests_allowed: true,
            dynamic_request_policy: serde_json::json!({}),
            dispatch_epoch: 0,
            requests_closed_at: Some(now),
            final_submitter_kind: "worker".to_string(),
            final_submitter_worker_run_id: None,
            created_from_stage_spec_hash: "sha256:test".to_string(),
            row_version: 1,
            created_at: now,
            updated_at: now,
        }
    }

    fn stage_team_resume_item(
        unit: &StageRunUnitRow,
        plan: &StageTeamPlanRow,
        role: &str,
        kind: &str,
        stable_key: &str,
        required_for_barrier: bool,
    ) -> StageWorkItemRow {
        let now = Utc::now();
        StageWorkItemRow {
            id: Uuid::new_v4(),
            team_plan_id: plan.id,
            operation_id: unit.operation_id,
            stage_execution_id: unit.stage_execution_id,
            stage_run_unit_id: unit.id,
            scope_snapshot_id: unit.scope_snapshot_id,
            organization_id: unit.organization_id,
            dispatch_epoch: 0,
            kind: kind.to_string(),
            stable_key: stable_key.to_string(),
            role: role.to_string(),
            input_manifest_hash: "sha256:test".to_string(),
            input_refs: serde_json::json!({}),
            required_for_barrier,
            conflict_key: None,
            priority: 0,
            status: "completed".to_string(),
            attempt_policy: serde_json::json!({}),
            budget: serde_json::json!({}),
            output_schema: "test.v1".to_string(),
            created_by: if stable_key == "leader:primary" {
                "server_seed".to_string()
            } else {
                "accepted_worker_request".to_string()
            },
            row_version: 1,
            created_at: now,
            updated_at: now,
            started_at: Some(now),
            terminal_at: Some(now),
        }
    }

    fn stage_team_resume_worker(
        unit: &StageRunUnitRow,
        item: &StageWorkItemRow,
    ) -> StageWorkerRunRow {
        let now = Utc::now();
        StageWorkerRunRow {
            id: Uuid::new_v4(),
            operation_id: unit.operation_id,
            stage_execution_id: unit.stage_execution_id,
            stage_run_unit_id: unit.id,
            work_item_id: Some(item.id),
            organization_id: unit.organization_id,
            worker_generation: 0,
            specialist: item.role.clone(),
            work_item_kind: item.kind.clone(),
            work_item_key: item.stable_key.clone(),
            agent_path: "main>stage_run:test".to_string(),
            parent_request_id: None,
            message_chain_id: Some(Uuid::new_v4()),
            status: "passed".to_string(),
            gate_attempt: 0,
            checkpoint: serde_json::json!({}),
            checkpoint_version: 1,
            lease_token: None,
            lease_owner: None,
            lease_acquired_at: None,
            lease_expires_at: None,
            heartbeat_at: None,
            attempt_epoch: 1,
            active_tool_call_id: None,
            active_tool_started_at: None,
            evidence_watermark: None,
            started_at: Some(now),
            updated_at: now,
            terminal_at: Some(now),
        }
    }

    fn exhausted_primary_output(
        unit: &StageRunUnitRow,
        plan: &StageTeamPlanRow,
        item: &StageWorkItemRow,
        worker: &StageWorkerRunRow,
    ) -> StageWorkerOutputRow {
        StageWorkerOutputRow {
            id: Uuid::new_v4(),
            team_plan_id: plan.id,
            work_item_id: item.id,
            worker_run_id: worker.id,
            operation_id: unit.operation_id,
            stage_execution_id: unit.stage_execution_id,
            stage_run_unit_id: unit.id,
            scope_snapshot_id: unit.scope_snapshot_id,
            organization_id: unit.organization_id,
            output_schema: item.output_schema.clone(),
            output_version: 1,
            business_disposition: "blocked".to_string(),
            canonical_output: serde_json::json!({
                "kind": "stage_team_attempts_exhausted",
                "failure_code": "stage_team_worker_lease_expired"
            }),
            canonical_fact_refs: serde_json::json!([]),
            evidence_ids: Vec::new(),
            checked_empty_cells: serde_json::json!([]),
            blocker_codes: vec!["STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED".to_string()],
            output_hash: "sha256:exhausted".to_string(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn stage_team_resume_selects_unique_controller_and_accepts_dynamic_children() {
        let unit = stage_team_resume_unit();
        let plan = stage_team_resume_plan(&unit);
        let leader_item = stage_team_resume_item(
            &unit,
            &plan,
            "company_stage_controller",
            "aggregate_stage_unit",
            "leader:primary",
            false,
        );
        let child_item = stage_team_resume_item(
            &unit,
            &plan,
            "enumerator",
            "content_enumeration",
            "dynamic:test",
            true,
        );
        let leader_worker = stage_team_resume_worker(&unit, &leader_item);
        let child_worker = stage_team_resume_worker(&unit, &child_item);
        let authority = StageTeamResumeAuthority {
            plan,
            work_items: vec![leader_item, child_item],
            outputs: Vec::new(),
            completed_synthesis_primary_worker_ids: HashSet::new(),
            recoverable_company_finalizer_worker_ids: HashSet::new(),
            recoverable_target_intel_finalizer_worker_ids: HashSet::new(),
        };

        let workers = [&child_worker, &leader_worker];
        let selected = select_stage_team_primary_worker(&unit, &workers, Some(&authority))
            .expect("valid Stage Team resume authority")
            .expect("leader Worker");

        assert_eq!(selected.id, leader_worker.id);
    }

    #[test]
    fn ordinary_company_finalizer_preflight_requires_the_exact_terminal_witness() {
        let mut unit = stage_team_resume_unit();
        unit.status = "running".to_string();
        unit.terminal_at = None;
        let mut plan = stage_team_resume_plan(&unit);
        plan.dynamic_request_policy =
            serde_json::json!({"coordination_mode": "company_controller"});
        let mut leader_item = stage_team_resume_item(
            &unit,
            &plan,
            "company_stage_controller",
            "aggregate_stage_unit",
            "leader:primary",
            false,
        );
        leader_item.status = "exhausted".to_string();
        let mut leader_worker = stage_team_resume_worker(&unit, &leader_item);
        leader_worker.status = "failed".to_string();
        leader_worker.checkpoint = serde_json::json!({
            "stage_team_execution_failure": {
                "code": "stage_team_worker_lease_expired"
            }
        });
        let output = exhausted_primary_output(&unit, &plan, &leader_item, &leader_worker);
        plan.final_submitter_worker_run_id = Some(leader_worker.id);
        let mut authority = StageTeamResumeAuthority {
            plan,
            work_items: vec![leader_item],
            outputs: vec![output],
            completed_synthesis_primary_worker_ids: HashSet::new(),
            recoverable_company_finalizer_worker_ids: HashSet::new(),
            recoverable_target_intel_finalizer_worker_ids: HashSet::new(),
        };

        assert!(!exact_company_finalizer_recovery_preflight(
            &leader_worker,
            &authority
        ));
        authority
            .recoverable_company_finalizer_worker_ids
            .insert(leader_worker.id);
        assert!(exact_company_finalizer_recovery_preflight(
            &leader_worker,
            &authority
        ));

        authority.outputs[0].canonical_output["failure_code"] =
            serde_json::json!("different_failure");
        assert!(!exact_company_finalizer_recovery_preflight(
            &leader_worker,
            &authority
        ));
    }

    #[test]
    fn stage_team_resume_accepts_only_witnessed_completed_normal_primary() {
        let unit = stage_team_resume_unit();
        let mut plan = stage_team_resume_plan(&unit);
        plan.leader_role = "investigation".to_string();
        plan.aggregator_role = Some("investigation".to_string());
        let mut leader_item = stage_team_resume_item(
            &unit,
            &plan,
            "investigation",
            "investigation_primary",
            "leader:primary",
            false,
        );
        leader_item.status = "completed".to_string();
        leader_item.terminal_at = Some(Utc::now());
        let leader_worker = stage_team_resume_worker(&unit, &leader_item);
        let mut authority = StageTeamResumeAuthority {
            plan,
            work_items: vec![leader_item],
            outputs: Vec::new(),
            completed_synthesis_primary_worker_ids: HashSet::new(),
            recoverable_company_finalizer_worker_ids: HashSet::new(),
            recoverable_target_intel_finalizer_worker_ids: HashSet::new(),
        };

        assert!(
            select_stage_team_primary_worker(&unit, &[&leader_worker], Some(&authority)).is_err(),
            "a generic passed Primary must remain terminal"
        );

        authority
            .completed_synthesis_primary_worker_ids
            .insert(leader_worker.id);
        let workers = [&leader_worker];
        let selected = select_stage_team_primary_worker(&unit, &workers, Some(&authority))
            .expect("sealed normal Primary is resumable")
            .expect("completed normal Primary remains the logical Primary");
        assert_eq!(selected.id, leader_worker.id);
    }

    #[test]
    fn stage_team_resume_accepts_exact_fresh_preclaim_without_worker() {
        let mut unit = stage_team_resume_unit();
        unit.status = "queued".to_string();
        unit.started_at = None;
        unit.terminal_at = None;
        let mut plan = stage_team_resume_plan(&unit);
        plan.requests_closed_at = None;
        let mut leader_item = stage_team_resume_item(
            &unit,
            &plan,
            "company_stage_controller",
            "aggregate_stage_unit",
            "leader:primary",
            false,
        );
        leader_item.status = "queued".to_string();
        leader_item.started_at = None;
        leader_item.terminal_at = None;
        let authority = StageTeamResumeAuthority {
            plan,
            work_items: vec![leader_item],
            outputs: Vec::new(),
            completed_synthesis_primary_worker_ids: HashSet::new(),
            recoverable_company_finalizer_worker_ids: HashSet::new(),
            recoverable_target_intel_finalizer_worker_ids: HashSet::new(),
        };

        assert!(
            select_stage_team_primary_worker(&unit, &[], Some(&authority))
                .expect("fresh Stage Team pre-claim is resumable")
                .is_none()
        );

        unit.status = "running".to_string();
        assert!(select_stage_team_primary_worker(&unit, &[], Some(&authority)).is_err());
    }

    #[test]
    fn stage_team_resume_rejects_duplicate_leader_or_foreign_child() {
        let unit = stage_team_resume_unit();
        let plan = stage_team_resume_plan(&unit);
        let leader_item = stage_team_resume_item(
            &unit,
            &plan,
            "company_stage_controller",
            "aggregate_stage_unit",
            "leader:primary",
            false,
        );
        let child_item = stage_team_resume_item(
            &unit,
            &plan,
            "enumerator",
            "content_enumeration",
            "dynamic:test",
            true,
        );
        let leader_worker = stage_team_resume_worker(&unit, &leader_item);
        let duplicate_leader = stage_team_resume_worker(&unit, &leader_item);
        let mut foreign_child = stage_team_resume_worker(&unit, &child_item);
        foreign_child.organization_id = Uuid::new_v4();
        let authority = StageTeamResumeAuthority {
            plan,
            work_items: vec![leader_item, child_item],
            outputs: Vec::new(),
            completed_synthesis_primary_worker_ids: HashSet::new(),
            recoverable_company_finalizer_worker_ids: HashSet::new(),
            recoverable_target_intel_finalizer_worker_ids: HashSet::new(),
        };

        assert!(select_stage_team_primary_worker(
            &unit,
            &[&leader_worker, &duplicate_leader],
            Some(&authority),
        )
        .is_err());
        assert!(select_stage_team_primary_worker(
            &unit,
            &[&leader_worker, &foreign_child],
            Some(&authority),
        )
        .is_err());
    }

    #[test]
    fn stage_team_resume_accepts_only_exact_sealed_synthesis_recovery_preclaim() {
        let mut unit = stage_team_resume_unit();
        unit.status = "running".to_string();
        unit.terminal_at = None;
        let plan = stage_team_resume_plan(&unit);
        let mut leader_item = stage_team_resume_item(
            &unit,
            &plan,
            "company_stage_controller",
            "aggregate_stage_unit",
            "leader:primary",
            false,
        );
        leader_item.status = "exhausted".to_string();
        let mut leader_worker = stage_team_resume_worker(&unit, &leader_item);
        leader_worker.status = "failed".to_string();
        let output = exhausted_primary_output(&unit, &plan, &leader_item, &leader_worker);
        let mut recovery_item = leader_item.clone();
        recovery_item.id = Uuid::new_v5(
            &leader_item.id,
            b"sealed-investigation-synthesis-recovery-primary-v1",
        );
        recovery_item.stable_key = format!("leader:synthesis-recovery:{}", leader_item.id);
        recovery_item.status = "queued".to_string();
        recovery_item.started_at = None;
        recovery_item.terminal_at = None;
        let authority = StageTeamResumeAuthority {
            plan,
            work_items: vec![leader_item, recovery_item],
            outputs: vec![output],
            completed_synthesis_primary_worker_ids: HashSet::new(),
            recoverable_company_finalizer_worker_ids: HashSet::new(),
            recoverable_target_intel_finalizer_worker_ids: HashSet::new(),
        };

        assert!(
            select_stage_team_primary_worker(&unit, &[&leader_worker], Some(&authority))
                .expect("exact sealed synthesis recovery is resumable")
                .is_none()
        );

        let mut invalid = authority;
        invalid.outputs[0].canonical_output["failure_code"] =
            serde_json::json!("different_failure");
        assert!(
            select_stage_team_primary_worker(&unit, &[&leader_worker], Some(&invalid)).is_err()
        );
    }

    #[test]
    fn stage_team_resume_accepts_historical_exhausted_v1_and_exact_active_v2() {
        let mut unit = stage_team_resume_unit();
        unit.status = "running".to_string();
        unit.terminal_at = None;
        let plan = stage_team_resume_plan(&unit);
        let mut leader_item = stage_team_resume_item(
            &unit,
            &plan,
            "company_stage_controller",
            "investigation_primary",
            "leader:primary",
            false,
        );
        leader_item.status = "exhausted".to_string();
        let mut leader_worker = stage_team_resume_worker(&unit, &leader_item);
        leader_worker.status = "failed".to_string();
        let leader_output = exhausted_primary_output(&unit, &plan, &leader_item, &leader_worker);

        let mut recovery_v1 = leader_item.clone();
        recovery_v1.id = Uuid::new_v5(
            &leader_item.id,
            b"sealed-investigation-synthesis-recovery-primary-v1",
        );
        recovery_v1.stable_key = format!("leader:synthesis-recovery:{}", leader_item.id);
        let mut recovery_v1_worker = stage_team_resume_worker(&unit, &recovery_v1);
        recovery_v1_worker.worker_generation = 1;
        recovery_v1_worker.status = "failed".to_string();
        let recovery_v1_output =
            exhausted_primary_output(&unit, &plan, &recovery_v1, &recovery_v1_worker);

        let mut recovery_v2 = recovery_v1.clone();
        recovery_v2.id = Uuid::new_v5(
            &recovery_v1.id,
            b"sealed-investigation-synthesis-recovery-primary-v2",
        );
        recovery_v2.kind = "investigation_primary_recovery".to_string();
        recovery_v2.status = "queued".to_string();
        recovery_v2.started_at = None;
        recovery_v2.terminal_at = None;
        let mut authority = StageTeamResumeAuthority {
            plan,
            work_items: vec![leader_item, recovery_v1, recovery_v2.clone()],
            outputs: vec![leader_output, recovery_v1_output],
            completed_synthesis_primary_worker_ids: HashSet::new(),
            recoverable_company_finalizer_worker_ids: HashSet::new(),
            recoverable_target_intel_finalizer_worker_ids: HashSet::new(),
        };

        assert!(select_stage_team_primary_worker(
            &unit,
            &[&leader_worker, &recovery_v1_worker],
            Some(&authority),
        )
        .expect("exact queued synthesis recovery v2 is resumable")
        .is_none());

        let mut recovery_v2_worker = stage_team_resume_worker(&unit, &recovery_v2);
        recovery_v2_worker.worker_generation = 2;
        recovery_v2_worker.status = "running".to_string();
        recovery_v2_worker.terminal_at = None;
        recovery_v2.status = "running".to_string();
        recovery_v2.started_at = Some(Utc::now());
        authority.work_items[2] = recovery_v2.clone();
        let active_workers = [&leader_worker, &recovery_v1_worker, &recovery_v2_worker];
        let selected = select_stage_team_primary_worker(&unit, &active_workers, Some(&authority))
            .expect("exact running synthesis recovery v2 is resumable")
            .expect("v2 Worker is the logical Primary");
        assert_eq!(selected.id, recovery_v2_worker.id);

        recovery_v2.status = "completed".to_string();
        recovery_v2.terminal_at = Some(Utc::now());
        authority.work_items[2] = recovery_v2;
        recovery_v2_worker.status = "passed".to_string();
        recovery_v2_worker.terminal_at = Some(Utc::now());
        authority
            .completed_synthesis_primary_worker_ids
            .insert(recovery_v2_worker.id);
        let completed_workers = [&leader_worker, &recovery_v1_worker, &recovery_v2_worker];
        let completed =
            select_stage_team_primary_worker(&unit, &completed_workers, Some(&authority))
                .expect("sealed completed synthesis recovery v2 is resumable")
                .expect("completed v2 Worker remains the logical Primary");
        assert_eq!(completed.id, recovery_v2_worker.id);
    }

    #[test]
    fn cli_descendants_share_one_operation_and_snapshot() {
        let operation_id = Uuid::new_v4();
        let root = Uuid::new_v4();
        let child = Uuid::new_v4();
        let grandchild = Uuid::new_v4();
        let excluded = Uuid::new_v4();
        let scope = build_cli_runtime_scope(
            &[
                organization(root, "Root", None, None),
                organization(child, "Child", Some(root), Some(75.0)),
                organization(grandchild, "Grandchild", Some(child), Some(60.0)),
                organization(excluded, "Excluded", Some(root), Some(49.0)),
            ],
            root,
            true,
            51,
        )
        .expect("build trusted CLI scope");

        assert_eq!(scope.units.len(), 3);
        assert_eq!(scope.units[0].organization_id, root);
        assert_eq!(scope.units[1].parent_organization_id, Some(root));
        assert_eq!(scope.units[2].parent_organization_id, Some(child));
        assert!(scope.units.iter().all(|unit| {
            unit.approval_source
                .get("kind")
                .and_then(serde_json::Value::as_str)
                == Some("cli_flags")
        }));
        let operation_ids = scope
            .units
            .iter()
            .map(|_| operation_id)
            .collect::<HashSet<_>>();
        assert_eq!(operation_ids, HashSet::from([operation_id]));
        assert_eq!(scope.units.len(), 3, "scope unit count");
        assert_eq!(
            scope.units.len(),
            3,
            "stage_run seeds one unit per frozen org"
        );
    }

    #[test]
    fn v2_writing_contracts_never_use_legacy_child_operation_fleet() {
        assert!(!contract_writes_v2(RuntimeMemoryContract::LegacyV1));
        for contract in [
            RuntimeMemoryContract::DualWriteLegacyRead,
            RuntimeMemoryContract::DualWriteV2Preferred,
            RuntimeMemoryContract::V2Only,
        ] {
            assert!(contract_writes_v2(contract), "contract={contract}");
        }
    }

    #[test]
    fn relational_resume_uses_effective_candidate_specialist_and_chain_agent_class() {
        use golish_core::AttackExecutionContract;
        use golish_db::models::AgentType;

        assert_eq!(
            effective_resume_specialist(
                StageKind::AttackCandidate,
                Some("analyst"),
                RuntimeMemoryContract::V2Only,
                AttackExecutionContract::V2Only,
            )
            .as_deref(),
            Some("attack_analyst")
        );
        assert_eq!(
            effective_resume_specialist(
                StageKind::Verification,
                None,
                RuntimeMemoryContract::V2Only,
                AttackExecutionContract::V2Only,
            )
            .as_deref(),
            Some("candidate_verifier")
        );
        assert_eq!(
            effective_resume_specialist(
                StageKind::Verification,
                None,
                RuntimeMemoryContract::DualWriteV2Preferred,
                AttackExecutionContract::DualWriteReadV2Fallback,
            ),
            None
        );
        assert_eq!(
            resume_worker_chain_agent("attack_analyst"),
            Some(AgentType::Pentester)
        );
        assert_eq!(
            resume_worker_chain_agent("candidate_verifier"),
            Some(AgentType::Pentester)
        );
        assert_eq!(
            resume_worker_chain_agent("reporter"),
            Some(AgentType::Reporter)
        );
        assert_eq!(
            resume_worker_chain_agent("application_understanding"),
            Some(AgentType::Pentester)
        );
        assert_eq!(
            resume_worker_chain_agent("investigation"),
            Some(AgentType::Pentester)
        );
    }

    #[test]
    fn root_stage_may_resume_between_execution_start_and_unit_seed() {
        assert!(exact_root_stage_preclaim(None, "started", 0, 0));
        assert!(!exact_root_stage_preclaim(
            Some("reporter"),
            "started",
            0,
            0
        ));
        assert!(!exact_root_stage_preclaim(None, "completed", 0, 0));
        assert!(!exact_root_stage_preclaim(None, "started", 1, 0));
        assert!(!exact_root_stage_preclaim(None, "started", 0, 1));
    }

    fn base() -> RuntimeV2ResumeSnapshot {
        RuntimeV2ResumeSnapshot {
            operation_id: Uuid::new_v4(),
            active_stage_execution_id: Uuid::new_v4(),
            active_stage_execution_count: 1,
            operation_superseded: false,
            current_stage_is_scoping: false,
            scope_sealed: true,
            scope_unit_count: 1,
            unit: None,
            worker: None,
            seeded_stage_team_preclaim: false,
            sealed_stage_team_completed_primary: false,
            recoverable_terminalized_finalizer: false,
            now: Utc::now(),
        }
    }

    fn specialist(live_lease: bool, active_tool: bool) -> RuntimeV2ResumeSnapshot {
        let mut snapshot = base();
        let unit = ResumeUnitSnapshot {
            id: Uuid::new_v4(),
            operation_id: snapshot.operation_id,
            stage_execution_id: snapshot.active_stage_execution_id,
            organization_id: Uuid::new_v4(),
            is_root: true,
            specialist: Some("enumerator".to_string()),
            status: RuntimeStageUnitStatus::Running,
        };
        snapshot.worker = Some(ResumeWorkerSnapshot {
            id: Uuid::new_v4(),
            operation_id: snapshot.operation_id,
            stage_execution_id: snapshot.active_stage_execution_id,
            stage_run_unit_id: unit.id,
            organization_id: unit.organization_id,
            status: RuntimeWorkerStatus::Running,
            lease_expires_at: Some(if live_lease {
                snapshot.now + Duration::minutes(1)
            } else {
                snapshot.now - Duration::minutes(1)
            }),
            active_tool_call_id: active_tool.then(Uuid::new_v4),
        });
        snapshot.unit = Some(unit);
        snapshot
    }

    #[test]
    fn resumability_distinguishes_scoping_specialist_and_root_only_units() {
        let mut scoping = base();
        scoping.current_stage_is_scoping = true;
        scoping.scope_sealed = false;
        scoping.scope_unit_count = 0;
        assert_eq!(
            classify_runtime_v2_resume(&scoping),
            RuntimeV2ResumeDecision::ResumeScoping
        );

        assert_eq!(
            classify_runtime_v2_resume(&specialist(true, false)),
            RuntimeV2ResumeDecision::WaitForLease
        );

        let mut root = base();
        root.unit = Some(ResumeUnitSnapshot {
            id: Uuid::new_v4(),
            operation_id: root.operation_id,
            stage_execution_id: root.active_stage_execution_id,
            organization_id: Uuid::new_v4(),
            is_root: true,
            specialist: None,
            status: RuntimeStageUnitStatus::Running,
        });
        assert_eq!(
            classify_runtime_v2_resume(&root),
            RuntimeV2ResumeDecision::ResumeRootUnit
        );

        assert_eq!(
            classify_runtime_v2_resume(&specialist(false, true)),
            RuntimeV2ResumeDecision::RecoveryRequired
        );
        assert_eq!(
            classify_runtime_v2_resume(&specialist(false, false)),
            RuntimeV2ResumeDecision::RequeueExpiredWorker
        );

        let mut seeded_team = base();
        seeded_team.seeded_stage_team_preclaim = true;
        seeded_team.unit = Some(ResumeUnitSnapshot {
            id: Uuid::new_v4(),
            operation_id: seeded_team.operation_id,
            stage_execution_id: seeded_team.active_stage_execution_id,
            organization_id: Uuid::new_v4(),
            is_root: true,
            specialist: Some("investigation".to_string()),
            status: RuntimeStageUnitStatus::Queued,
        });
        assert_eq!(
            classify_runtime_v2_resume(&seeded_team),
            RuntimeV2ResumeDecision::ResumeSpecialist
        );

        seeded_team.unit.as_mut().expect("seeded team Unit").status =
            RuntimeStageUnitStatus::Running;
        assert_eq!(
            classify_runtime_v2_resume(&seeded_team),
            RuntimeV2ResumeDecision::ResumeSpecialist
        );

        seeded_team.seeded_stage_team_preclaim = false;
        assert_eq!(
            classify_runtime_v2_resume(&seeded_team),
            RuntimeV2ResumeDecision::Reject(RuntimeV2ResumeReject::WorkerRequired)
        );

        let mut completed_recovery = specialist(false, false);
        completed_recovery
            .worker
            .as_mut()
            .expect("completed recovery Worker")
            .status = RuntimeWorkerStatus::Passed;
        completed_recovery.sealed_stage_team_completed_primary = true;
        assert_eq!(
            classify_runtime_v2_resume(&completed_recovery),
            RuntimeV2ResumeDecision::ResumeSpecialist
        );

        completed_recovery.sealed_stage_team_completed_primary = false;
        assert_eq!(
            classify_runtime_v2_resume(&completed_recovery),
            RuntimeV2ResumeDecision::Reject(RuntimeV2ResumeReject::InvalidWorkerState)
        );
    }

    #[test]
    fn target_intel_finalizer_recovery_admits_only_the_exact_terminal_witness() {
        let mut recovery = specialist(false, false);
        let worker = recovery.worker.as_mut().expect("Target Intel Controller");
        worker.status = RuntimeWorkerStatus::Failed;
        worker.lease_expires_at = None;

        assert_eq!(
            classify_runtime_v2_resume(&recovery),
            RuntimeV2ResumeDecision::Reject(RuntimeV2ResumeReject::InvalidWorkerState)
        );

        recovery.recoverable_terminalized_finalizer = true;
        assert_eq!(
            classify_runtime_v2_resume(&recovery),
            RuntimeV2ResumeDecision::ResumeSpecialist
        );

        recovery
            .worker
            .as_mut()
            .expect("Target Intel Controller")
            .active_tool_call_id = Some(Uuid::new_v4());
        assert_eq!(
            classify_runtime_v2_resume(&recovery),
            RuntimeV2ResumeDecision::Reject(RuntimeV2ResumeReject::InvalidWorkerState)
        );
    }

    #[test]
    fn malformed_or_cross_operation_runtime_identity_fails_closed() {
        let mut cross = specialist(true, false);
        cross.worker.as_mut().expect("worker").operation_id = Uuid::new_v4();
        assert_eq!(
            classify_runtime_v2_resume(&cross),
            RuntimeV2ResumeDecision::Reject(RuntimeV2ResumeReject::CrossOperationIdentity)
        );

        let mut malformed_scoping = base();
        malformed_scoping.current_stage_is_scoping = true;
        malformed_scoping.scope_sealed = false;
        malformed_scoping.scope_unit_count = 1;
        assert_eq!(
            classify_runtime_v2_resume(&malformed_scoping),
            RuntimeV2ResumeDecision::Reject(RuntimeV2ResumeReject::InvalidScopingShape)
        );
    }
}
