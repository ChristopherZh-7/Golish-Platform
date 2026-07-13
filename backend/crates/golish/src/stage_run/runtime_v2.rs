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
use golish_db::models::Organization;
use uuid::Uuid;

use super::scheduler::{FleetReport, OrgRunOutcome, OrgRunStatus};

#[derive(Debug, Clone)]
pub(crate) struct RuntimeV2CliReport {
    pub operation_id: Uuid,
    pub scope_unit_count: usize,
    pub stage_unit_count: usize,
    pub fleet: FleetReport,
}

pub(crate) fn persisted_contract(value: &str) -> Result<RuntimeMemoryContract> {
    RuntimeMemoryContract::try_from(value)
        .map_err(|error| anyhow!("invalid persisted runtime-memory contract: {error}"))
}

pub(crate) fn contract_writes_v2(contract: RuntimeMemoryContract) -> bool {
    contract.policy().write != RuntimeMemoryWriteStrategy::LegacyOnly
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
    let active_executions = executions
        .iter()
        .filter(|execution| execution.status == "started")
        .collect::<Vec<_>>();
    let [execution] = active_executions.as_slice() else {
        return Err(anyhow!(
            "V2 CLI requires one exact active stage execution, found {}",
            active_executions.len()
        ));
    };
    if execution.stage_kind != stage.as_str() {
        return Err(anyhow!(
            "V2 CLI active execution is {}, expected {}",
            execution.stage_kind,
            stage.as_str()
        ));
    }
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
                    let worker = match worker_rows.as_slice() {
                        [worker] => RuntimeWorkerStatus::try_parse(&worker.status).map(|status| {
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
                        }),
                        [] => None,
                        _ => {
                            return OrgRunOutcome {
                                org_id: scope_unit.organization_id,
                                org_name: scope_unit.organization_name_at_freeze.clone(),
                                status: OrgRunStatus::Failed,
                                detail: Some("multiple V2 workers own one stage unit".to_string()),
                            };
                        }
                    };
                    match status {
                        Some(status) => classify_runtime_v2_resume(&RuntimeV2ResumeSnapshot {
                            operation_id: task.id,
                            active_stage_execution_id: execution.id,
                            active_stage_execution_count: active_executions.len(),
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
        return Decision::Reject(Reject::WorkerRequired);
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
