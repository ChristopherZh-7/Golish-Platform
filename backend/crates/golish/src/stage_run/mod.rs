//! 方案 2 · headless single/range stage runner (`golish --stage-run`).
//!
//! Boots the real backend **without the GUI** (embedded Postgres + the full
//! pentest tool surface + a real LLM), runs one harness stage — or a
//! `--from`..=`--to` slice of the stage DAG — drives any scoping HITL
//! automatically (`--auto-approve`), prints a structured report (gate
//! PASS/BLOCK + reasons, tools called, evidence booked), and exits. The run's
//! full `transcript.json` is written exactly like a GUI run, so
//! `golish --replay <session>` (and the GUI) can replay the same timeline.
//!
//! See `docs/design/2026-06-06-headless-single-stage-runner.md`.

pub(crate) mod fleet;
pub(crate) mod runtime_v2;
/// Stage-agnostic per-org scheduling kernel (K-controlled concurrency, resume
/// skip, failure isolation). A general, unit-tested component owned by
/// `stage_run`; exposed `pub` so its full tested API isn't flagged as crate-dead
/// even though the CLI subsidiary fan-out currently drives only the checklist path.
pub mod scheduler;
mod scope_authority;

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use tokio::sync::mpsc;

use golish_agent_kit::harness::stage_fanout::{build_child_objective, filter_child_orgs};
use golish_agent_kit::harness::{active_profile_id, StageKind};
use golish_core::agent_mode::AgentMode;
use golish_core::events::{AiEvent, HarnessTraceKind};
use golish_core::hitl::ApprovalDecision;
use golish_core::runtime::{GolishRuntime, RuntimeEvent};

use crate::ai::agent_bridge::AgentBridge;
use crate::ai::commands::core::operation_resume::{
    claim_exact_resume_runtime_source, select_exact_resume_runtime_source,
};
use crate::ai::task_operation::{FreshOperationScope, SubsidiaryScopePolicy};
use crate::cli::Args;
use crate::runtime::CliRuntime;
use crate::stage_run::fleet::{AlwaysRunOracle, CliFleetProgress, NoopScorer, OrgFleetExecutor};
use crate::stage_run::scheduler::{
    run_legacy_child_operation_fleet, FleetConfig, FleetMode, FleetReport, OrgRunTask,
};

/// `resolve_slice` moved to `golish_agent_kit::harness::slice` (Phase B,
/// 设计 2026-06-13-engagement-scoping-fanout §6.3) so the engagement worker
/// sessions (chat task mode) and this headless CLI resolve slices identically.
/// Thin anyhow adapter kept here so existing call sites / tests read the same.
fn resolve_slice(
    profile_id: &str,
    from: Option<StageKind>,
    to: StageKind,
) -> Result<(StageKind, HashSet<StageKind>)> {
    golish_agent_kit::harness::resolve_slice(profile_id, from, to).map_err(|e| anyhow!(e))
}

fn resolve_slice_for_topology(
    profile_id: &str,
    topology: golish_core::StageTopologyContract,
    from: Option<StageKind>,
    to: StageKind,
) -> Result<(StageKind, HashSet<StageKind>)> {
    golish_agent_kit::harness::resolve_slice_for_topology(profile_id, topology, from, to)
        .map_err(|e| anyhow!(e))
}

/// Fresh operation creation freezes its topology inside the DB transaction.
/// Preflight therefore accepts the union of the closed catalog without reading
/// a mutable rollout default; execution immediately reloads and projects the
/// one persisted topology.
fn resolve_fresh_slice(
    profile_id: &str,
    from: Option<StageKind>,
    to: StageKind,
) -> Result<(StageKind, HashSet<StageKind>)> {
    golish_agent_kit::harness::resolve_slice_for_any_topology(profile_id, from, to)
        .map_err(|e| anyhow!(e))
}

fn validate_persisted_stage_topology(
    topology: &str,
    canonical_json: &str,
    sha256: &str,
    freeze_source: &str,
    investigation_rollout_mode: &str,
) -> Result<golish_core::FrozenStageTopologyContractMaterial> {
    let topology = golish_core::StageTopologyContract::try_parse(topology)
        .context("unknown persisted stage topology contract")?;
    let freeze_source = golish_core::StageTopologyFreezeSource::try_parse(freeze_source)
        .context("unknown persisted stage topology freeze source")?;
    let rollout_mode = golish_core::InvestigationRolloutMode::try_from(investigation_rollout_mode)
        .context("unknown persisted Investigation rollout mode")?;
    let material = golish_core::FrozenStageTopologyContractMaterial {
        topology,
        canonical_json: canonical_json.to_string(),
        sha256: sha256.to_string(),
    };
    material
        .validate_for_operation(freeze_source, rollout_mode)
        .context("invalid persisted operation stage topology witness")?;
    Ok(material)
}

/// Resolve an explicit forward continuation for an exact resume. The default
/// remains the persisted current stage only; callers must opt in to every
/// wider testing slice without changing the operation's frozen profile/scope.
fn resolve_resume_slice(
    profile_id: &str,
    topology: golish_core::StageTopologyContract,
    current: StageKind,
    requested_to: Option<&str>,
) -> Result<(StageKind, HashSet<StageKind>)> {
    let terminal = requested_to
        .map(|value| {
            StageKind::try_parse(value).ok_or_else(|| anyhow!("unknown --resume-to stage: {value}"))
        })
        .transpose()?
        .unwrap_or(current);
    let (entry, allowlist) =
        resolve_slice_for_topology(profile_id, topology, Some(current), terminal)
            .context("resume refused: requested continuation is outside the frozen profile")?;
    anyhow::ensure!(
        entry == current,
        "resume refused: requested continuation does not begin at the persisted current stage"
    );
    Ok((terminal, allowlist))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedForkSlice {
    stage_topology_contract: golish_core::FrozenStageTopologyContractMaterial,
    entry_stage: StageKind,
    terminal_stage: StageKind,
    allowlist: HashSet<StageKind>,
    adopted_stage_kinds: Vec<StageKind>,
}

/// Resolve a source-operation fork without ever allowing Scoping itself to be
/// executed. A fork must name one exact stage or a fully-bounded forward
/// range; its adopted inputs are the profile-DAG strict ancestors of the
/// selected entry, in canonical graph order.
fn resolve_stage_run_fork_slice(
    profile_id: &str,
    stage_topology_contract: golish_core::FrozenStageTopologyContractMaterial,
    args: &Args,
) -> Result<ResolvedForkSlice> {
    let parse =
        |value: &str| StageKind::try_parse(value).ok_or_else(|| anyhow!("unknown stage: {value}"));
    let (entry_stage, terminal_stage) = match (
        args.only.as_deref(),
        args.from.as_deref(),
        args.to.as_deref(),
    ) {
        (Some(only), None, None) => {
            let stage = parse(only)?;
            (stage, stage)
        }
        (None, Some(from), Some(to)) => (parse(from)?, parse(to)?),
        _ => anyhow::bail!(
            "--stage-run-fork requires --only <stage> or both --from <stage> and --to <stage>"
        ),
    };
    anyhow::ensure!(
        entry_stage != StageKind::Scoping,
        "--stage-run-fork adopts Scoping from the source operation; use --stage-run to execute Scoping"
    );

    let topology = stage_topology_contract.topology;
    let (resolved_entry, allowlist) =
        resolve_slice_for_topology(profile_id, topology, Some(entry_stage), terminal_stage)?;
    anyhow::ensure!(
        resolved_entry == entry_stage,
        "fork slice entry diverges from the requested stage"
    );
    let profile = golish_agent_kit::harness::load_embedded_profile(profile_id)
        .with_context(|| format!("load fork profile {profile_id}"))?
        .ok_or_else(|| anyhow!("unknown harness profile: {profile_id}"))?;
    let graph = golish_agent_kit::harness::operation_graph_for_topology(topology)
        .context("load frozen operation graph for stage fork")?;
    let allowed = profile
        .allowed_stage_set_for_topology(topology)
        .context("project fork profile through frozen topology")?;
    let dag = graph.project(&allowed);
    let strict_ancestors = dag.ancestors_inclusive(entry_stage);
    let adopted_stage_kinds = dag
        .nodes
        .iter()
        .copied()
        .filter(|stage| *stage != entry_stage && strict_ancestors.contains(stage))
        .collect::<Vec<_>>();
    anyhow::ensure!(
        adopted_stage_kinds.first() == Some(&StageKind::Scoping),
        "fork entry has no adopted Scoping authority in profile {profile_id}"
    );

    Ok(ResolvedForkSlice {
        stage_topology_contract,
        entry_stage,
        terminal_stage,
        allowlist,
        adopted_stage_kinds,
    })
}

const EXACT_RESUME_CHAIN_SQL: &str = r#"SELECT session_id, task_id, agent::text,
              chain IS NOT NULL AS has_persisted_chain
       FROM message_chains
       WHERE id = $1
         AND session_id = $2
         AND (task_id IS NULL OR task_id = $3)
         AND agent = $4::agent_type"#;

const REPAIR_GRAPH_FLOW_SQL: &str = r#"UPDATE operation_state
       SET state_blob = jsonb_set(state_blob, '{graph_flow}', $4::jsonb, true)
       WHERE operation_id = $1
         AND current_stage = $2
         AND state_blob = $3
         AND superseded_by IS NULL
         AND runtime_memory_contract IN (
             'legacy_v1','dual_write_legacy_read','dual_write_v2_preferred'
         )
         AND state_blob -> 'graph_flow' IS NULL"#;

const REPAIR_REAPED_TASK_SQL: &str = r#"WITH repaired_task AS (
       UPDATE tasks
          SET status = 'waiting', result = NULL, updated_at = NOW()
        WHERE id = $1
          AND session_id = $2
          AND status = 'failed'
          AND result = $3
          AND updated_at = $4
          AND EXISTS (
              SELECT 1 FROM operation_state os
               WHERE os.operation_id = tasks.id
                 AND os.operation_id = $1
                 AND os.profile = $5
                 AND os.current_stage = $6
                 AND os.engagement_org_id IS NOT DISTINCT FROM $7
                 AND os.superseded_by IS NULL
                 AND os.state_blob = $8
          )
       RETURNING id
     ), latest_failed_turn AS (
       SELECT turn.id,turn.operation_id,turn.ordinal,turn.trigger_input
         FROM operation_turns turn
         JOIN repaired_task task ON task.id=turn.operation_id
        WHERE turn.status='failed' AND turn.terminal_at IS NOT NULL
          AND turn.ordinal=(SELECT MAX(candidate.ordinal)
                              FROM operation_turns candidate
                             WHERE candidate.operation_id=turn.operation_id)
          AND NOT EXISTS(SELECT 1 FROM operation_turns open_turn
                          WHERE open_turn.operation_id=turn.operation_id
                            AND open_turn.status IN ('running','waiting'))
     )
     INSERT INTO operation_turns(id,operation_id,ordinal,trigger_input,status)
     SELECT uuid_generate_v5(id,'stage-run-reaped-task-repair-v1'),operation_id,
            ordinal+1,trigger_input,'waiting'
       FROM latest_failed_turn"#;

#[derive(Debug, Clone, PartialEq, Eq)]
enum ResumeSelector {
    ChatKey(String),
    Uuid(uuid::Uuid),
}

fn classify_resume_selector(selector: &str) -> ResumeSelector {
    match uuid::Uuid::parse_str(selector.trim()) {
        Ok(id) => ResumeSelector::Uuid(id),
        Err(_) => ResumeSelector::ChatKey(selector.trim().to_string()),
    }
}

fn is_supported_resume_chat_key(chat_key: &str) -> bool {
    chat_key.starts_with("stage-run-") || chat_key.starts_with("pentest-chat-")
}

#[derive(Debug, Clone, Default)]
struct ResumeExpectations {
    allow_orphan_running: bool,
    repair_missing_graph_flow: bool,
    repair_reaped_task: bool,
    session_id: Option<uuid::Uuid>,
    task_id: Option<uuid::Uuid>,
    operation_id: Option<uuid::Uuid>,
    organization_id: Option<uuid::Uuid>,
    stage: Option<StageKind>,
}

impl ResumeExpectations {
    fn from_args(args: &Args) -> Result<Self> {
        let stage = args
            .expect_stage
            .as_deref()
            .map(|value| {
                StageKind::try_parse(value)
                    .ok_or_else(|| anyhow!("unknown --expect-stage value: {value}"))
            })
            .transpose()?;
        let expectations = Self {
            allow_orphan_running: args.allow_orphan_running,
            repair_missing_graph_flow: args.repair_missing_graph_flow,
            repair_reaped_task: args.repair_reaped_task,
            session_id: args.expect_session,
            task_id: args.expect_task,
            operation_id: args.expect_operation,
            organization_id: args.expect_org,
            stage,
        };
        if expectations.allow_orphan_running
            || expectations.repair_missing_graph_flow
            || expectations.repair_reaped_task
        {
            anyhow::ensure!(
                expectations.has_complete_identity(),
                "--allow-orphan-running/--repair-missing-graph-flow/--repair-reaped-task require --expect-session, --expect-task, --expect-operation, --expect-org, and --expect-stage"
            );
        }
        Ok(expectations)
    }

    fn has_complete_identity(&self) -> bool {
        self.session_id.is_some()
            && self.task_id.is_some()
            && self.operation_id.is_some()
            && self.organization_id.is_some()
            && self.stage.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResumeWorkerRef {
    chain_id: uuid::Uuid,
    specialist: String,
}

#[derive(Debug, Clone)]
struct ResumeWorkerOwnership {
    chain_id: uuid::Uuid,
    specialist: String,
    stored_session_id: Option<uuid::Uuid>,
    stored_task_id: Option<uuid::Uuid>,
    stored_agent: Option<String>,
    has_persisted_chain: bool,
}

#[derive(Debug, Clone)]
struct ResumeCandidate {
    session_id: uuid::Uuid,
    chat_session_key: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    task_id: uuid::Uuid,
    task_session_id: uuid::Uuid,
    task_status: golish_db::models::TaskStatus,
    task_result: Option<String>,
    task_updated_at: chrono::DateTime<chrono::Utc>,
    operation_id: uuid::Uuid,
    profile: String,
    current_stage: String,
    runtime_memory_contract: String,
    investigation_rollout_mode: String,
    stage_topology_contract: String,
    stage_topology_canonical_json: String,
    stage_topology_sha256: String,
    stage_topology_freeze_source: String,
    engagement_org_id: Option<uuid::Uuid>,
    superseded_by: Option<uuid::Uuid>,
    state_blob: serde_json::Value,
    worker_chains: Vec<ResumeWorkerOwnership>,
    relational_v2: Option<runtime_v2::RuntimeV2ResumeAuthority>,
    expectations: ResumeExpectations,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResumeAuthorityKind {
    LegacyCheckpoint,
    RelationalV2,
}

fn selected_resume_record_source(
    authority: ResumeAuthorityKind,
    contract: golish_agent_kit::runtime_memory::RuntimeMemoryContract,
) -> Result<golish_agent_kit::db_traits::RuntimeMemoryRecordSource> {
    use golish_agent_kit::db_traits::RuntimeMemoryRecordSource as Source;
    use golish_agent_kit::runtime_memory::RuntimeMemoryContract as Contract;

    match (authority, contract) {
        (
            ResumeAuthorityKind::LegacyCheckpoint,
            Contract::LegacyV1 | Contract::DualWriteLegacyRead,
        ) => Ok(Source::Legacy),
        (ResumeAuthorityKind::LegacyCheckpoint, Contract::DualWriteV2Preferred) => {
            Ok(Source::LegacyFallback)
        }
        (ResumeAuthorityKind::RelationalV2, Contract::DualWriteV2Preferred | Contract::V2Only) => {
            Ok(Source::V2)
        }
        _ => anyhow::bail!(
            "resume refused: selected runtime-memory authority is invalid for frozen contract"
        ),
    }
}

#[derive(Debug, Clone)]
struct ValidatedResumeTarget {
    session_id: uuid::Uuid,
    chat_session_key: String,
    provider: Option<String>,
    model: Option<String>,
    operation_id: uuid::Uuid,
    runtime_memory_contract: golish_agent_kit::runtime_memory::RuntimeMemoryContract,
    stage_topology_contract: golish_core::FrozenStageTopologyContractMaterial,
    authority: ResumeAuthorityKind,
    relational_stage_execution_id: Option<uuid::Uuid>,
    task_updated_at: chrono::DateTime<chrono::Utc>,
    profile: String,
    stage: StageKind,
    organization_id: uuid::Uuid,
    state_blob: serde_json::Value,
    needs_graph_repair: bool,
    needs_task_repair: bool,
}

#[derive(Debug, Clone)]
struct TerminalResumeCandidate {
    session_id: uuid::Uuid,
    chat_session_key: Option<String>,
    task_id: uuid::Uuid,
    task_session_id: uuid::Uuid,
    task_status: golish_db::models::TaskStatus,
    task_result: Option<String>,
    operation_id: uuid::Uuid,
    profile: String,
    current_stage: String,
    runtime_memory_contract: String,
    investigation_rollout_mode: String,
    stage_topology_contract: String,
    stage_topology_canonical_json: String,
    stage_topology_sha256: String,
    stage_topology_freeze_source: String,
    engagement_org_id: Option<uuid::Uuid>,
    superseded_by: Option<uuid::Uuid>,
    expectations: ResumeExpectations,
}

#[derive(Debug, Clone)]
struct ValidatedTerminalResumeReplay {
    session_id: uuid::Uuid,
    chat_session_key: String,
    operation_id: uuid::Uuid,
    organization_id: uuid::Uuid,
    profile: String,
    stage: StageKind,
    stage_topology_contract: golish_core::FrozenStageTopologyContractMaterial,
    result: String,
}

fn stage_worker_refs_from_blob(
    state_blob: &serde_json::Value,
    stage: StageKind,
) -> Result<Vec<ResumeWorkerRef>> {
    let workers = state_blob
        .get("stage_run_workers")
        .and_then(|value| value.get(stage.as_str()))
        .and_then(serde_json::Value::as_object)
        .filter(|workers| !workers.is_empty())
        .ok_or_else(|| {
            anyhow!(
                "resume refused: no exact stage_run worker map for {}",
                stage.as_str()
            )
        })?;

    let mut refs = Vec::with_capacity(workers.len());
    for (org_id, worker) in workers {
        uuid::Uuid::parse_str(org_id)
            .with_context(|| format!("resume refused: invalid worker organization id {org_id}"))?;
        let chain_id = worker
            .get("chain_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow!("resume refused: worker {org_id} has no exact chain_id"))
            .and_then(|value| {
                uuid::Uuid::parse_str(value)
                    .with_context(|| format!("resume refused: invalid worker chain id {value}"))
            })?;
        anyhow::ensure!(
            !chain_id.is_nil(),
            "resume refused: worker {org_id} has a nil chain id"
        );
        let specialist = worker
            .get("specialist")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("resume refused: worker {org_id} has no specialist"))?;
        refs.push(ResumeWorkerRef {
            chain_id,
            specialist: specialist.to_string(),
        });
    }
    refs.sort_by_key(|worker| worker.chain_id);
    refs.dedup_by_key(|worker| worker.chain_id);
    anyhow::ensure!(
        refs.len() == workers.len(),
        "resume refused: duplicate exact worker chain ids"
    );
    Ok(refs)
}

fn validate_expected_identity(
    candidate: &ResumeCandidate,
    stage: StageKind,
    organization_id: uuid::Uuid,
) -> Result<()> {
    let expected = &candidate.expectations;
    if let Some(id) = expected.session_id {
        anyhow::ensure!(
            id == candidate.session_id,
            "resume refused: expected DB session {id}, found {}",
            candidate.session_id
        );
    }
    if let Some(id) = expected.task_id {
        anyhow::ensure!(
            id == candidate.task_id,
            "resume refused: expected task {id}, found {}",
            candidate.task_id
        );
    }
    if let Some(id) = expected.operation_id {
        anyhow::ensure!(
            id == candidate.operation_id,
            "resume refused: expected operation {id}, found {}",
            candidate.operation_id
        );
    }
    if let Some(id) = expected.organization_id {
        anyhow::ensure!(
            id == organization_id,
            "resume refused: expected organization {id}, found {organization_id}",
        );
    }
    if let Some(expected_stage) = expected.stage {
        anyhow::ensure!(
            expected_stage == stage,
            "resume refused: expected stage {}, found {}",
            expected_stage.as_str(),
            stage.as_str()
        );
    }
    Ok(())
}

fn validate_resume_candidate(candidate: &ResumeCandidate) -> Result<ValidatedResumeTarget> {
    let chat_session_key = candidate
        .chat_session_key
        .as_deref()
        .filter(|key| is_supported_resume_chat_key(key))
        .ok_or_else(|| {
            anyhow!(
                "resume refused: DB session is not owned by a supported stage-run or pentest Task chat key"
            )
        })?;
    anyhow::ensure!(
        candidate.task_session_id == candidate.session_id,
        "resume refused: task does not belong to the selected DB session"
    );
    anyhow::ensure!(
        candidate.operation_id == candidate.task_id,
        "resume refused: operation id does not equal the selected task id"
    );
    anyhow::ensure!(
        candidate.superseded_by.is_none(),
        "resume refused: operation was superseded"
    );
    let stage = StageKind::try_parse(&candidate.current_stage).ok_or_else(|| {
        anyhow!(
            "resume refused: unknown current stage {}",
            candidate.current_stage
        )
    })?;
    let stage_topology_contract = validate_persisted_stage_topology(
        &candidate.stage_topology_contract,
        &candidate.stage_topology_canonical_json,
        &candidate.stage_topology_sha256,
        &candidate.stage_topology_freeze_source,
        &candidate.investigation_rollout_mode,
    )
    .context("resume refused: persisted topology is invalid")?;
    resolve_slice_for_topology(
        &candidate.profile,
        stage_topology_contract.topology,
        Some(stage),
        stage,
    )
    .context("resume refused: current stage is not allowed by the persisted profile")?;
    let runtime_memory_contract =
        runtime_v2::persisted_contract(&candidate.runtime_memory_contract)
            .context("resume refused: invalid frozen runtime-memory contract")?;
    use golish_agent_kit::runtime_memory::RuntimeMemoryContract;
    let authority = match runtime_memory_contract {
        RuntimeMemoryContract::LegacyV1 | RuntimeMemoryContract::DualWriteLegacyRead => {
            ResumeAuthorityKind::LegacyCheckpoint
        }
        RuntimeMemoryContract::DualWriteV2Preferred if candidate.relational_v2.is_some() => {
            ResumeAuthorityKind::RelationalV2
        }
        RuntimeMemoryContract::DualWriteV2Preferred => ResumeAuthorityKind::LegacyCheckpoint,
        RuntimeMemoryContract::V2Only if candidate.relational_v2.is_some() => {
            ResumeAuthorityKind::RelationalV2
        }
        RuntimeMemoryContract::V2Only => anyhow::bail!(
            "resume refused: V2-only operation has no complete relational runtime authority"
        ),
    };
    let organization_id = match authority {
        ResumeAuthorityKind::LegacyCheckpoint => candidate
            .engagement_org_id
            .ok_or_else(|| anyhow!("resume refused: operation has no engagement organization"))?,
        ResumeAuthorityKind::RelationalV2 => {
            let relational = candidate
                .relational_v2
                .as_ref()
                .expect("relational authority selected only when present");
            if stage == StageKind::Scoping && candidate.engagement_org_id.is_none() {
                anyhow::ensure!(
                    candidate.expectations.organization_id == Some(relational.organization_id),
                    "resume refused: pre-freeze Scoping authority requires the exact expected organization"
                );
            } else {
                anyhow::ensure!(
                    candidate.engagement_org_id == Some(relational.organization_id),
                    "resume refused: relational scope root does not equal operation engagement organization"
                );
            }
            relational.organization_id
        }
    };
    validate_expected_identity(candidate, stage, organization_id)?;

    let needs_task_repair = match candidate.task_status {
        golish_db::models::TaskStatus::Waiting => false,
        golish_db::models::TaskStatus::Running => {
            anyhow::ensure!(
                candidate.task_result.is_none(),
                "resume refused: running operation carries a non-null task result"
            );
            false
        }
        golish_db::models::TaskStatus::Failed => {
            anyhow::ensure!(
                candidate.expectations.repair_reaped_task
                    && candidate.expectations.has_complete_identity(),
                "resume refused: failed task requires --repair-reaped-task and all expected identities"
            );
            anyhow::ensure!(
                candidate.task_result.as_deref()
                    == Some(golish_db::repo::tasks::ABANDONED_TASK_RESULT),
                "resume refused: failed task does not carry the exact startup-reaper abandoned marker"
            );
            true
        }
        status => anyhow::bail!(
            "resume refused: task status {status:?} is not resumable (running or waiting required)"
        ),
    };

    let needs_graph_repair = if authority == ResumeAuthorityKind::RelationalV2 {
        false
    } else {
        let mapped_workers = stage_worker_refs_from_blob(&candidate.state_blob, stage)?;
        anyhow::ensure!(
            mapped_workers.len() == candidate.worker_chains.len(),
            "resume refused: exact worker ownership rows are incomplete"
        );
        for mapped in &mapped_workers {
            let expected_agent = runtime_v2::resume_worker_chain_agent(&mapped.specialist)
                .ok_or_else(|| {
                    anyhow!(
                        "resume refused: specialist {} has no durable message-chain agent class",
                        mapped.specialist
                    )
                })?;
            let expected_agent_name = runtime_v2::persisted_agent_name(expected_agent);
            let ownership = candidate
                .worker_chains
                .iter()
                .find(|owned| {
                    owned.chain_id == mapped.chain_id && owned.specialist == mapped.specialist
                })
                .ok_or_else(|| {
                    anyhow!(
                        "resume refused: exact chain {} has no ownership row",
                        mapped.chain_id
                    )
                })?;
            anyhow::ensure!(
                ownership.stored_session_id == Some(candidate.session_id)
                    && ownership
                        .stored_task_id
                        .is_none_or(|task_id| task_id == candidate.task_id)
                    && ownership.stored_agent.as_deref() == Some(expected_agent_name)
                    && ownership.has_persisted_chain,
                "resume refused: exact chain {} is outside the selected session/task/agent scope",
                mapped.chain_id
            );
        }

        match candidate.state_blob.get("graph_flow") {
            Some(graph_flow) => {
                let graph_state = graph_flow
                    .get("state")
                    .cloned()
                    .ok_or_else(|| anyhow!("resume refused: graph_flow state is missing"))?;
                serde_json::from_value::<
                    golish_agent_kit::harness::operation_flow::OperationFlowState,
                >(graph_state)
                .context("resume refused: graph_flow state is malformed")?;
                let next_node = graph_flow
                    .get("next_node")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                anyhow::ensure!(
                next_node == candidate.current_stage,
                "resume refused: graph checkpoint next_node {next_node:?} does not equal current stage {}",
                candidate.current_stage
            );
                false
            }
            None => {
                anyhow::ensure!(
                candidate.expectations.repair_missing_graph_flow
                    && candidate.expectations.has_complete_identity(),
                "resume refused: graph_flow checkpoint is missing; explicit repair and all expected identities are required"
            );
                let flat = serde_json::from_value::<
                    golish_agent_kit::task_orchestrator::harness_resume::HarnessResumeState,
                >(candidate.state_blob.clone())
                .context("resume refused: flat harness checkpoint is malformed")?;
                anyhow::ensure!(
                    flat.profile == candidate.profile,
                    "resume refused: flat checkpoint profile does not match operation_state"
                );
                anyhow::ensure!(
                    flat.current_stage == candidate.current_stage,
                    "resume refused: flat checkpoint current_stage does not match operation_state"
                );
                anyhow::ensure!(
                    flat.current_stage_run_id.is_some_and(|id| !id.is_nil()),
                    "resume refused: flat checkpoint has no valid current_stage_run_id"
                );
                anyhow::ensure!(
                flat.completed_count == 0,
                "resume refused: missing graph_flow is only repairable before the first graph node completes"
            );
                true
            }
        }
    };

    Ok(ValidatedResumeTarget {
        session_id: candidate.session_id,
        chat_session_key: chat_session_key.to_string(),
        provider: candidate.provider.clone(),
        model: candidate.model.clone(),
        operation_id: candidate.operation_id,
        runtime_memory_contract,
        stage_topology_contract,
        authority,
        relational_stage_execution_id: candidate
            .relational_v2
            .as_ref()
            .map(|authority| authority.active_stage_execution_id),
        task_updated_at: candidate.task_updated_at,
        profile: candidate.profile.clone(),
        stage,
        organization_id,
        state_blob: candidate.state_blob.clone(),
        needs_graph_repair,
        needs_task_repair,
    })
}

fn validate_terminal_resume_candidate(
    candidate: &TerminalResumeCandidate,
) -> Result<Option<ValidatedTerminalResumeReplay>> {
    if candidate.task_status != golish_db::models::TaskStatus::Finished {
        return Ok(None);
    }
    anyhow::ensure!(
        candidate.expectations.has_complete_identity(),
        "terminal replay requires --expect-session, --expect-task, --expect-operation, --expect-org, and --expect-stage"
    );
    let chat_session_key = candidate
        .chat_session_key
        .as_deref()
        .filter(|key| is_supported_resume_chat_key(key))
        .ok_or_else(|| {
            anyhow!(
                "resume refused: terminal task DB session is not owned by a supported stage-run or pentest Task chat key"
            )
        })?;
    anyhow::ensure!(
        candidate.task_session_id == candidate.session_id,
        "resume refused: terminal task does not belong to the selected DB session"
    );
    anyhow::ensure!(
        candidate.operation_id == candidate.task_id,
        "resume refused: terminal operation id does not equal the selected task id"
    );
    anyhow::ensure!(
        candidate.superseded_by.is_none(),
        "resume refused: terminal operation was superseded"
    );
    let stage = StageKind::try_parse(&candidate.current_stage).ok_or_else(|| {
        anyhow!(
            "resume refused: unknown terminal operation stage {}",
            candidate.current_stage
        )
    })?;
    let stage_topology_contract = validate_persisted_stage_topology(
        &candidate.stage_topology_contract,
        &candidate.stage_topology_canonical_json,
        &candidate.stage_topology_sha256,
        &candidate.stage_topology_freeze_source,
        &candidate.investigation_rollout_mode,
    )
    .context("resume refused: terminal persisted topology is invalid")?;
    resolve_slice_for_topology(
        &candidate.profile,
        stage_topology_contract.topology,
        Some(stage),
        stage,
    )
    .context("resume refused: terminal stage is not allowed by the persisted profile")?;
    runtime_v2::persisted_contract(&candidate.runtime_memory_contract)
        .context("resume refused: invalid terminal frozen runtime-memory contract")?;
    let organization_id = candidate.engagement_org_id.ok_or_else(|| {
        anyhow!("resume refused: terminal operation has no engagement organization")
    })?;
    anyhow::ensure!(
        candidate.expectations.session_id == Some(candidate.session_id)
            && candidate.expectations.task_id == Some(candidate.task_id)
            && candidate.expectations.operation_id == Some(candidate.operation_id)
            && candidate.expectations.organization_id == Some(organization_id)
            && candidate.expectations.stage == Some(stage),
        "resume refused: terminal replay expected identity does not match the durable operation"
    );
    let result = candidate
        .task_result
        .as_deref()
        .map(str::trim)
        .filter(|result| !result.is_empty())
        .ok_or_else(|| anyhow!("resume refused: finished task has no durable result"))?;
    Ok(Some(ValidatedTerminalResumeReplay {
        session_id: candidate.session_id,
        chat_session_key: chat_session_key.to_string(),
        operation_id: candidate.operation_id,
        organization_id,
        profile: candidate.profile.clone(),
        stage,
        stage_topology_contract,
        result: result.to_string(),
    }))
}

fn synthesize_graph_flow_checkpoint(
    mut state_blob: serde_json::Value,
    current_stage: StageKind,
) -> Result<serde_json::Value> {
    let root = state_blob
        .as_object_mut()
        .ok_or_else(|| anyhow!("resume refused: operation state_blob is not an object"))?;
    anyhow::ensure!(
        !root.contains_key("graph_flow"),
        "resume refused: graph_flow already exists; refusing to overwrite checkpoint"
    );
    root.insert(
        "graph_flow".to_string(),
        serde_json::json!({
            "state": golish_agent_kit::harness::operation_flow::OperationFlowState::default(),
            "next_node": current_stage.as_str(),
        }),
    );
    Ok(state_blob)
}

fn resume_advisory_lock_keys(operation_id: uuid::Uuid) -> (i32, i32) {
    let raw = operation_id.as_u128();
    let folded = (raw >> 64) as u64 ^ raw as u64;
    (
        ((folded >> 32) as u32 ^ 0x5352_5253) as i32,
        folded as u32 as i32,
    )
}

async fn session_by_chat_key(
    pool: &sqlx::PgPool,
    chat_key: &str,
) -> Result<Option<golish_db::models::Session>> {
    sqlx::query_as::<_, golish_db::models::Session>(
        "SELECT * FROM sessions WHERE chat_session_key = $1",
    )
    .bind(chat_key)
    .fetch_optional(pool)
    .await
    .context("look up stage-run DB session by chat key")
}

fn resume_task_status_is_selectable(
    status: golish_db::models::TaskStatus,
    _allow_orphan_running: bool,
) -> bool {
    matches!(
        status,
        golish_db::models::TaskStatus::Waiting | golish_db::models::TaskStatus::Running
    )
}

async fn task_for_resume_session(
    pool: &sqlx::PgPool,
    session: &golish_db::models::Session,
    expectations: &ResumeExpectations,
) -> Result<golish_db::models::Task> {
    if let Some(task_id) = expectations.task_id {
        let task = golish_db::repo::tasks::get(pool, task_id)
            .await
            .context("look up expected stage-run task")?
            .ok_or_else(|| anyhow!("resume refused: expected task {task_id} does not exist"))?;
        anyhow::ensure!(
            task.session_id == session.id,
            "resume refused: expected task {task_id} is outside DB session {}",
            session.id
        );
        return Ok(task);
    }

    let tasks = golish_db::repo::tasks::list_by_session(pool, session.id)
        .await
        .context("list stage-run tasks for resume")?;
    let mut candidates = Vec::new();
    for task in tasks {
        if !resume_task_status_is_selectable(task.status, expectations.allow_orphan_running) {
            continue;
        }
        if golish_db::repo::operation_state::get(pool, task.id)
            .await
            .context("look up stage-run operation for task")?
            .is_some()
        {
            candidates.push(task);
        }
    }
    anyhow::ensure!(
        candidates.len() == 1,
        "resume refused: DB session {} has {} non-terminal operations; pass --expect-task/operation UUID to disambiguate",
        session.id,
        candidates.len()
    );
    Ok(candidates.remove(0))
}

async fn load_resume_rows(
    pool: &sqlx::PgPool,
    selector: &ResumeSelector,
    expectations: &ResumeExpectations,
) -> Result<(
    golish_db::models::Session,
    golish_db::models::Task,
    golish_db::repo::operation_state::OperationStateRow,
)> {
    match selector {
        ResumeSelector::ChatKey(chat_key) => {
            anyhow::ensure!(
                is_supported_resume_chat_key(chat_key),
                "resume refused: chat selector must be a stage-run-* or pentest-chat-* key"
            );
            let session = session_by_chat_key(pool, chat_key)
                .await?
                .ok_or_else(|| anyhow!("resume refused: stage-run session {chat_key} not found"))?;
            let task = task_for_resume_session(pool, &session, expectations).await?;
            let operation = golish_db::repo::operation_state::get(pool, task.id)
                .await
                .context("load selected stage-run operation")?
                .ok_or_else(|| {
                    anyhow!("resume refused: task {} has no operation_state", task.id)
                })?;
            Ok((session, task, operation))
        }
        ResumeSelector::Uuid(id) => {
            if let Some(operation) = golish_db::repo::operation_state::get(pool, *id)
                .await
                .context("resolve resume UUID as operation")?
            {
                let task = golish_db::repo::tasks::get(pool, operation.operation_id)
                    .await
                    .context("load task owning selected operation")?
                    .ok_or_else(|| {
                        anyhow!(
                            "resume refused: operation {} has no task row",
                            operation.operation_id
                        )
                    })?;
                let session = golish_db::repo::sessions::get(pool, task.session_id)
                    .await
                    .context("load DB session owning selected task")?
                    .ok_or_else(|| anyhow!("resume refused: task {} has no DB session", task.id))?;
                return Ok((session, task, operation));
            }

            let session = golish_db::repo::sessions::get(pool, *id)
                .await
                .context("resolve resume UUID as DB session")?
                .ok_or_else(|| {
                    anyhow!("resume refused: UUID {id} is neither an operation nor a DB session")
                })?;
            let task = task_for_resume_session(pool, &session, expectations).await?;
            let operation = golish_db::repo::operation_state::get(pool, task.id)
                .await
                .context("load selected stage-run operation")?
                .ok_or_else(|| {
                    anyhow!("resume refused: task {} has no operation_state", task.id)
                })?;
            Ok((session, task, operation))
        }
    }
}

#[derive(Debug, Clone)]
struct ValidatedStageForkSource {
    operation_id: uuid::Uuid,
    profile: String,
    stage_topology_contract: golish_core::FrozenStageTopologyContractMaterial,
    project_scope_id: uuid::Uuid,
    source_scope_snapshot_id: uuid::Uuid,
    root_organization_id: uuid::Uuid,
    root_organization_name: String,
    provider: Option<String>,
    model: Option<String>,
    runtime_scope: golish_agent_kit::db_traits::CliRuntimeScope,
}

fn require_unique_stage_fork_operation(
    session_id: uuid::Uuid,
    operation_ids: &[uuid::Uuid],
) -> Result<uuid::Uuid> {
    anyhow::ensure!(
        operation_ids.len() == 1,
        "stage fork refused: DB session {session_id} has {} operations; pass the exact operation UUID",
        operation_ids.len()
    );
    Ok(operation_ids[0])
}

async fn task_for_stage_fork_session(
    pool: &sqlx::PgPool,
    session: &golish_db::models::Session,
) -> Result<golish_db::models::Task> {
    let tasks = golish_db::repo::tasks::list_by_session(pool, session.id)
        .await
        .context("list source session tasks for stage fork")?;
    let mut operation_ids = Vec::new();
    for task in &tasks {
        if golish_db::repo::operation_state::get(pool, task.id)
            .await
            .context("look up source operation for stage fork")?
            .is_some()
        {
            operation_ids.push(task.id);
        }
    }
    let operation_id = require_unique_stage_fork_operation(session.id, &operation_ids)?;
    tasks
        .into_iter()
        .find(|task| task.id == operation_id)
        .ok_or_else(|| anyhow!("stage fork source task disappeared"))
}

async fn load_stage_fork_rows(
    pool: &sqlx::PgPool,
    selector: &ResumeSelector,
) -> Result<(
    golish_db::models::Session,
    golish_db::models::Task,
    golish_db::repo::operation_state::OperationStateRow,
)> {
    match selector {
        ResumeSelector::ChatKey(chat_key) => {
            anyhow::ensure!(
                is_supported_resume_chat_key(chat_key),
                "stage fork refused: chat selector must be a stage-run-* or pentest-chat-* key"
            );
            let session = session_by_chat_key(pool, chat_key)
                .await?
                .ok_or_else(|| anyhow!("stage fork source session {chat_key} not found"))?;
            let task = task_for_stage_fork_session(pool, &session).await?;
            let operation = golish_db::repo::operation_state::get(pool, task.id)
                .await
                .context("load selected stage fork source operation")?
                .ok_or_else(|| anyhow!("stage fork source task {} has no operation", task.id))?;
            Ok((session, task, operation))
        }
        ResumeSelector::Uuid(id) => {
            if let Some(operation) = golish_db::repo::operation_state::get(pool, *id)
                .await
                .context("resolve stage fork UUID as operation")?
            {
                let task = golish_db::repo::tasks::get(pool, operation.operation_id)
                    .await
                    .context("load stage fork source task")?
                    .ok_or_else(|| anyhow!("stage fork source operation {id} has no task"))?;
                let session = golish_db::repo::sessions::get(pool, task.session_id)
                    .await
                    .context("load stage fork source session")?
                    .ok_or_else(|| anyhow!("stage fork source task {} has no session", task.id))?;
                return Ok((session, task, operation));
            }
            let session = golish_db::repo::sessions::get(pool, *id)
                .await
                .context("resolve stage fork UUID as DB session")?
                .ok_or_else(|| {
                    anyhow!(
                        "stage fork refused: UUID {id} is neither an operation nor a DB session"
                    )
                })?;
            let task = task_for_stage_fork_session(pool, &session).await?;
            let operation = golish_db::repo::operation_state::get(pool, task.id)
                .await
                .context("load selected stage fork source operation")?
                .ok_or_else(|| anyhow!("stage fork source task {} has no operation", task.id))?;
            Ok((session, task, operation))
        }
    }
}

async fn validate_stage_fork_source(
    pool: &sqlx::PgPool,
    selector: &ResumeSelector,
    workspace: &Path,
    resolved: &ResolvedForkSlice,
) -> Result<ValidatedStageForkSource> {
    let (session, task, operation) = load_stage_fork_rows(pool, selector).await?;
    anyhow::ensure!(
        task.id == operation.operation_id,
        "stage fork source task/operation mismatch"
    );
    anyhow::ensure!(
        operation.superseded_by.is_none(),
        "stage fork source operation was superseded"
    );
    let stage_topology_contract = validate_persisted_stage_topology(
        &operation.stage_topology_contract,
        &operation.stage_topology_canonical_json,
        &operation.stage_topology_sha256,
        &operation.stage_topology_freeze_source,
        &operation.investigation_rollout_mode,
    )
    .context("stage fork source has an invalid frozen topology")?;
    anyhow::ensure!(
        stage_topology_contract == resolved.stage_topology_contract,
        "stage fork source topology changed during preflight"
    );
    resolve_slice_for_topology(
        &operation.profile,
        stage_topology_contract.topology,
        Some(resolved.entry_stage),
        resolved.terminal_stage,
    )
    .context("stage fork slice is outside the source operation profile")?;
    let project_scope_id = operation
        .project_scope_id
        .ok_or_else(|| anyhow!("stage fork source has no frozen project scope"))?;
    let frozen =
        golish_db::repo::operation_org_scope::load_for_operation(pool, operation.operation_id)
            .await
            .map_err(anyhow::Error::new)
            .context("load stage fork source organization scope")?
            .ok_or_else(|| anyhow!("stage fork source has no organization scope snapshot"))?;
    anyhow::ensure!(
        frozen.snapshot.sealed_at.is_some(),
        "stage fork source scope is not sealed"
    );
    anyhow::ensure!(
        frozen.snapshot.project_scope_id == project_scope_id,
        "stage fork source project/scope identity mismatch"
    );
    let (canonical_workspace, _) =
        golish_agent_kit::runtime_memory::canonical_workspace_identity(workspace)
            .map_err(anyhow::Error::new)
            .context("canonicalize stage fork workspace")?;
    anyhow::ensure!(
        frozen.snapshot.project_path_at_freeze == canonical_workspace,
        "stage fork source belongs to workspace {}, current workspace is {}",
        frozen.snapshot.project_path_at_freeze,
        canonical_workspace
    );
    anyhow::ensure!(!frozen.units.is_empty(), "stage fork source scope is empty");

    let adopted_names = resolved
        .adopted_stage_kinds
        .iter()
        .map(|stage| stage.as_str().to_string())
        .collect::<Vec<_>>();
    for unit in &frozen.units {
        let expected = adopted_names
            .iter()
            // Scoping authority is the already-validated sealed organization
            // snapshot above; normal Scoping completion deliberately has no
            // Worker-backed StageHandoff.
            .filter(|stage| stage.as_str() != StageKind::Scoping.as_str())
            .cloned()
            .collect::<Vec<_>>();
        let seals = golish_db::repo::stage_handoffs::list_latest_final_sealed_for_sources(
            pool,
            operation.operation_id,
            unit.organization_id,
            &expected,
        )
        .await
        .map_err(anyhow::Error::new)
        .with_context(|| format!("validate adopted final seals for {}", unit.organization_id))?;
        let actual = seals
            .iter()
            .map(|seal| seal.from_stage_kind.as_str())
            .collect::<BTreeSet<_>>();
        let expected_set = expected.iter().map(String::as_str).collect::<BTreeSet<_>>();
        anyhow::ensure!(
            actual == expected_set,
            "stage fork source is incomplete for organization {}: expected {:?}, found {:?}",
            unit.organization_id,
            expected_set,
            actual
        );
    }

    let root = frozen
        .units
        .iter()
        .find(|unit| unit.organization_id == frozen.snapshot.root_organization_id)
        .ok_or_else(|| anyhow!("stage fork source scope has no root unit"))?;
    let runtime_scope = golish_agent_kit::db_traits::CliRuntimeScope {
        root_organization_id: frozen.snapshot.root_organization_id,
        include_subsidiaries: frozen.units.len() > 1,
        subsidiary_threshold: 51,
        units: frozen
            .units
            .iter()
            .map(|unit| golish_agent_kit::db_traits::CliRuntimeScopeUnit {
                organization_id: unit.organization_id,
                parent_organization_id: unit.parent_organization_id,
                organization_name: unit.organization_name_at_freeze.clone(),
                depth: unit.depth,
                ordinal: unit.ordinal,
                ownership_percent: unit.ownership_percent.clone(),
                approval_source: serde_json::json!({
                    "kind": "stage_fork_source_scope",
                    "source_operation_id": operation.operation_id,
                    "source_scope_snapshot_id": frozen.snapshot.id,
                    "source_decision_row_id": unit.decision_row_id,
                }),
            })
            .collect(),
    };
    Ok(ValidatedStageForkSource {
        operation_id: operation.operation_id,
        profile: operation.profile,
        stage_topology_contract,
        project_scope_id,
        source_scope_snapshot_id: frozen.snapshot.id,
        root_organization_id: frozen.snapshot.root_organization_id,
        root_organization_name: root.organization_name_at_freeze.clone(),
        provider: session.provider,
        model: session.model,
        runtime_scope,
    })
}

async fn resolve_stage_run_resume_target(
    pool: &sqlx::PgPool,
    selector: &ResumeSelector,
    expectations: &ResumeExpectations,
) -> Result<ValidatedResumeTarget> {
    let (session, task, operation) = load_resume_rows(pool, selector, expectations).await?;
    let stage = StageKind::try_parse(&operation.current_stage).ok_or_else(|| {
        anyhow!(
            "resume refused: unknown operation stage {}",
            operation.current_stage
        )
    })?;
    let contract = runtime_v2::persisted_contract(&operation.runtime_memory_contract)?;
    use golish_agent_kit::runtime_memory::RuntimeMemoryContract;
    let relational_v2 = match contract {
        RuntimeMemoryContract::DualWriteV2Preferred => {
            match runtime_v2::load_relational_resume_authority(
                pool,
                session.id,
                &operation,
                stage,
                expectations.organization_id,
            )
            .await
            {
                Ok(authority) => Some(authority),
                Err(error)
                    if runtime_v2::relational_resume_error_allows_legacy_fallback(&error) =>
                {
                    tracing::debug!(
                        operation_id = %operation.operation_id,
                        error = %error,
                        "relational V2 resume record is structurally incomplete; selecting whole legacy fallback"
                    );
                    None
                }
                Err(error) => {
                    return Err(error).context(
                        "resume refused: relational V2 authority is busy or failed identity/storage validation",
                    )
                }
            }
        }
        RuntimeMemoryContract::V2Only => Some(
            runtime_v2::load_relational_resume_authority(
                pool,
                session.id,
                &operation,
                stage,
                expectations.organization_id,
            )
            .await
            .context("resume refused: incomplete V2-only relational runtime authority")?,
        ),
        RuntimeMemoryContract::LegacyV1 | RuntimeMemoryContract::DualWriteLegacyRead => None,
    };
    let select_legacy = matches!(
        contract,
        RuntimeMemoryContract::LegacyV1 | RuntimeMemoryContract::DualWriteLegacyRead
    ) || (contract == RuntimeMemoryContract::DualWriteV2Preferred
        && relational_v2.is_none());
    let mut worker_chains = Vec::new();
    if select_legacy {
        let mapped_workers = stage_worker_refs_from_blob(&operation.state_blob, stage)?;
        worker_chains.reserve(mapped_workers.len());
        for worker in mapped_workers {
            let expected_agent = runtime_v2::resume_worker_chain_agent(&worker.specialist)
                .ok_or_else(|| {
                    anyhow!(
                        "resume refused: specialist {} has no durable message-chain agent class",
                        worker.specialist
                    )
                })?;
            let row: Option<(uuid::Uuid, Option<uuid::Uuid>, String, bool)> =
                sqlx::query_as(EXACT_RESUME_CHAIN_SQL)
                    .bind(worker.chain_id)
                    .bind(session.id)
                    .bind(task.id)
                    .bind(expected_agent)
                    .fetch_optional(pool)
                    .await
                    .with_context(|| {
                        format!("validate exact worker chain {} ownership", worker.chain_id)
                    })?;
            let (stored_session_id, stored_task_id, stored_agent, has_persisted_chain) = row
                .map_or((None, None, None, false), |(sid, tid, agent, has_chain)| {
                    (Some(sid), tid, Some(agent), has_chain)
                });
            worker_chains.push(ResumeWorkerOwnership {
                chain_id: worker.chain_id,
                specialist: worker.specialist,
                stored_session_id,
                stored_task_id,
                stored_agent,
                has_persisted_chain,
            });
        }
    }

    validate_resume_candidate(&ResumeCandidate {
        session_id: session.id,
        chat_session_key: session.chat_session_key,
        provider: session.provider,
        model: session.model,
        task_id: task.id,
        task_session_id: task.session_id,
        task_status: task.status,
        task_result: task.result,
        task_updated_at: task.updated_at,
        operation_id: operation.operation_id,
        profile: operation.profile,
        current_stage: operation.current_stage,
        runtime_memory_contract: operation.runtime_memory_contract,
        investigation_rollout_mode: operation.investigation_rollout_mode,
        stage_topology_contract: operation.stage_topology_contract,
        stage_topology_canonical_json: operation.stage_topology_canonical_json,
        stage_topology_sha256: operation.stage_topology_sha256,
        stage_topology_freeze_source: operation.stage_topology_freeze_source,
        engagement_org_id: operation.engagement_org_id,
        superseded_by: operation.superseded_by,
        state_blob: operation.state_blob,
        worker_chains,
        relational_v2,
        expectations: expectations.clone(),
    })
}

async fn resolve_terminal_stage_run_resume_replay(
    pool: &sqlx::PgPool,
    selector: &ResumeSelector,
    expectations: &ResumeExpectations,
) -> Result<Option<ValidatedTerminalResumeReplay>> {
    let (session, task, operation) = load_resume_rows(pool, selector, expectations).await?;
    validate_terminal_resume_candidate(&TerminalResumeCandidate {
        session_id: session.id,
        chat_session_key: session.chat_session_key,
        task_id: task.id,
        task_session_id: task.session_id,
        task_status: task.status,
        task_result: task.result,
        operation_id: operation.operation_id,
        profile: operation.profile,
        current_stage: operation.current_stage,
        runtime_memory_contract: operation.runtime_memory_contract,
        investigation_rollout_mode: operation.investigation_rollout_mode,
        stage_topology_contract: operation.stage_topology_contract,
        stage_topology_canonical_json: operation.stage_topology_canonical_json,
        stage_topology_sha256: operation.stage_topology_sha256,
        stage_topology_freeze_source: operation.stage_topology_freeze_source,
        engagement_org_id: operation.engagement_org_id,
        superseded_by: operation.superseded_by,
        expectations: expectations.clone(),
    })
}

struct StageRunResumeClaim {
    connection: Option<sqlx::PgConnection>,
    keys: (i32, i32),
}

impl StageRunResumeClaim {
    async fn acquire(pool: &sqlx::PgPool, operation_id: uuid::Uuid) -> Result<Self> {
        let keys = resume_advisory_lock_keys(operation_id);
        let mut connection = pool
            .acquire()
            .await
            .context("acquire dedicated stage-run resume claim connection")?
            .detach();
        let claimed: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1, $2)")
            .bind(keys.0)
            .bind(keys.1)
            .fetch_one(&mut connection)
            .await
            .context("claim stage-run operation advisory lock")?;
        anyhow::ensure!(
            claimed,
            "resume refused: operation {operation_id} is already claimed by another process"
        );
        Ok(Self {
            connection: Some(connection),
            keys,
        })
    }

    fn connection_mut(&mut self) -> Result<&mut sqlx::PgConnection> {
        self.connection
            .as_mut()
            .ok_or_else(|| anyhow!("stage-run resume claim connection is closed"))
    }

    async fn release(mut self) -> Result<()> {
        use sqlx::Connection;

        let mut connection = self
            .connection
            .take()
            .ok_or_else(|| anyhow!("stage-run resume claim was already released"))?;
        let unlocked: bool = sqlx::query_scalar("SELECT pg_advisory_unlock($1, $2)")
            .bind(self.keys.0)
            .bind(self.keys.1)
            .fetch_one(&mut connection)
            .await
            .context("release stage-run operation advisory lock")?;
        anyhow::ensure!(unlocked, "stage-run resume advisory lock was not held");
        connection
            .close()
            .await
            .context("close dedicated stage-run resume claim connection")
    }
}

async fn repair_missing_graph_flow(
    claim: &mut StageRunResumeClaim,
    target: &ValidatedResumeTarget,
) -> Result<()> {
    anyhow::ensure!(
        target.needs_graph_repair,
        "graph-flow repair requested for an operation that already has a checkpoint"
    );
    anyhow::ensure!(
        target.authority == ResumeAuthorityKind::LegacyCheckpoint
            && target.runtime_memory_contract
                != golish_agent_kit::runtime_memory::RuntimeMemoryContract::V2Only,
        "graph-flow repair is forbidden for relational V2 resume authority"
    );
    let repaired = synthesize_graph_flow_checkpoint(target.state_blob.clone(), target.stage)?;
    let graph_flow = repaired
        .get("graph_flow")
        .cloned()
        .ok_or_else(|| anyhow!("synthesized graph_flow checkpoint is missing"))?;
    let result = sqlx::query(REPAIR_GRAPH_FLOW_SQL)
        .bind(target.operation_id)
        .bind(target.stage.as_str())
        .bind(&target.state_blob)
        .bind(graph_flow)
        .execute(claim.connection_mut()?)
        .await
        .context("compare-and-set missing graph_flow checkpoint")?;
    anyhow::ensure!(
        result.rows_affected() == 1,
        "resume refused: graph_flow checkpoint changed before repair claim completed"
    );
    Ok(())
}

async fn repair_reaped_task(
    claim: &mut StageRunResumeClaim,
    target: &ValidatedResumeTarget,
) -> Result<()> {
    anyhow::ensure!(
        target.needs_task_repair,
        "reaped-task repair requested for a task that is not the selected abandoned failure"
    );
    let result = sqlx::query(REPAIR_REAPED_TASK_SQL)
        .bind(target.operation_id)
        .bind(target.session_id)
        .bind(golish_db::repo::tasks::ABANDONED_TASK_RESULT)
        .bind(target.task_updated_at)
        .bind(&target.profile)
        .bind(target.stage.as_str())
        .bind(target.organization_id)
        .bind(&target.state_blob)
        .execute(claim.connection_mut()?)
        .await
        .context("compare-and-set startup-reaped task back to waiting")?;
    anyhow::ensure!(
        result.rows_affected() == 1,
        "resume refused: reaped task changed before exact repair claim completed"
    );
    Ok(())
}

async fn ensure_reaped_task_open_turn(
    claim: &mut StageRunResumeClaim,
    target: &ValidatedResumeTarget,
) -> Result<()> {
    let restored: bool = sqlx::query_scalar(
        r#"WITH latest_failed_turn AS (
               SELECT turn.id,turn.operation_id,turn.ordinal,turn.trigger_input
                 FROM operation_turns turn
                 JOIN tasks task ON task.id=turn.operation_id
                 JOIN operation_state operation ON operation.operation_id=task.id
                WHERE task.id=$1 AND task.session_id=$2
                  AND task.status='waiting' AND task.result IS NULL
                  AND operation.profile=$3 AND operation.current_stage=$4
                  AND operation.engagement_org_id IS NOT DISTINCT FROM $5
                  AND operation.superseded_by IS NULL AND operation.state_blob=$6
                  AND turn.status='failed' AND turn.terminal_at IS NOT NULL
                  AND turn.ordinal=(SELECT MAX(candidate.ordinal)
                                      FROM operation_turns candidate
                                     WHERE candidate.operation_id=turn.operation_id)
                  AND NOT EXISTS(SELECT 1 FROM operation_turns open_turn
                                  WHERE open_turn.operation_id=turn.operation_id
                                    AND open_turn.status IN ('running','waiting'))
           ), inserted AS (
               INSERT INTO operation_turns(id,operation_id,ordinal,trigger_input,status)
               SELECT uuid_generate_v5(id,'stage-run-reaped-task-repair-v1'),operation_id,
                      ordinal+1,trigger_input,'waiting'
                 FROM latest_failed_turn
               ON CONFLICT (operation_id,ordinal) DO NOTHING
               RETURNING id
           )
           SELECT EXISTS(SELECT 1 FROM inserted)
               OR EXISTS(SELECT 1 FROM operation_turns open_turn
                           WHERE open_turn.operation_id=$1
                             AND open_turn.status IN ('running','waiting'))"#,
    )
    .bind(target.operation_id)
    .bind(target.session_id)
    .bind(&target.profile)
    .bind(target.stage.as_str())
    .bind(target.organization_id)
    .bind(&target.state_blob)
    .fetch_one(claim.connection_mut()?)
    .await
    .context("restore exact startup-reaped operation Turn")?;
    anyhow::ensure!(
        restored,
        "resume refused: repaired task has no exact failed Turn to continue"
    );
    Ok(())
}

/// Test-database-only escape hatch for a stage whose sole Company Controller
/// consumed its frozen producer retry budget while validating a code fix. The
/// old execution is superseded atomically and its business facts are retained;
/// normal resume authority then has to validate the newly seeded execution.
/// This deliberately does not make terminal failed Workers generally
/// resumable and cannot select the user's default database.
#[allow(dead_code)]
async fn restart_exhausted_test_stage_runtime(
    pool: &sqlx::PgPool,
    selector: &ResumeSelector,
    expectations: &ResumeExpectations,
    workspace: &Path,
) -> Result<()> {
    anyhow::ensure!(
        expectations.has_complete_identity(),
        "test exhausted-stage restart requires all expected identities"
    );
    let (session, task, operation) = load_resume_rows(pool, selector, expectations).await?;
    let stage = StageKind::try_parse(&operation.current_stage).ok_or_else(|| {
        anyhow!(
            "test exhausted-stage restart refused unknown current stage {}",
            operation.current_stage
        )
    })?;
    let organization_id = operation.engagement_org_id.ok_or_else(|| {
        anyhow!("test exhausted-stage restart requires an engagement organization")
    })?;
    anyhow::ensure!(
        task.id == operation.operation_id && task.session_id == session.id,
        "test exhausted-stage restart task/session/operation identity mismatch"
    );
    anyhow::ensure!(
        operation.superseded_by.is_none()
            && operation.runtime_memory_contract == "v2_only"
            && matches!(
                task.status,
                golish_db::models::TaskStatus::Running | golish_db::models::TaskStatus::Waiting
            ),
        "test exhausted-stage restart requires one active V2-only operation"
    );
    let candidate = ResumeCandidate {
        session_id: session.id,
        chat_session_key: session.chat_session_key.clone(),
        provider: session.provider.clone(),
        model: session.model.clone(),
        task_id: task.id,
        task_session_id: task.session_id,
        task_status: task.status,
        task_result: task.result.clone(),
        task_updated_at: task.updated_at,
        operation_id: operation.operation_id,
        profile: operation.profile.clone(),
        current_stage: operation.current_stage.clone(),
        runtime_memory_contract: operation.runtime_memory_contract.clone(),
        investigation_rollout_mode: operation.investigation_rollout_mode.clone(),
        stage_topology_contract: operation.stage_topology_contract.clone(),
        stage_topology_canonical_json: operation.stage_topology_canonical_json.clone(),
        stage_topology_sha256: operation.stage_topology_sha256.clone(),
        stage_topology_freeze_source: operation.stage_topology_freeze_source.clone(),
        engagement_org_id: operation.engagement_org_id,
        superseded_by: operation.superseded_by,
        state_blob: operation.state_blob.clone(),
        worker_chains: Vec::new(),
        relational_v2: None,
        expectations: expectations.clone(),
    };
    validate_expected_identity(&candidate, stage, organization_id)?;

    let frozen =
        golish_db::repo::operation_org_scope::load_for_operation(pool, operation.operation_id)
            .await
            .map_err(anyhow::Error::new)?
            .ok_or_else(|| anyhow!("test exhausted-stage restart requires frozen scope"))?;
    let (canonical_workspace, _) =
        golish_agent_kit::runtime_memory::canonical_workspace_identity(workspace)
            .map_err(anyhow::Error::new)?;
    anyhow::ensure!(
        frozen.snapshot.sealed_at.is_some()
            && frozen.snapshot.root_organization_id == organization_id
            && frozen.snapshot.project_path_at_freeze == canonical_workspace
            && !frozen.units.is_empty(),
        "test exhausted-stage restart scope/workspace authority mismatch"
    );

    let executions =
        golish_db::repo::stage_runs::list_for_operation(pool, operation.operation_id).await?;
    let active = executions
        .iter()
        .filter(|execution| execution.status == "started")
        .collect::<Vec<_>>();
    let [active_execution] = active.as_slice() else {
        anyhow::bail!(
            "test exhausted-stage restart requires one active execution, found {}",
            active.len()
        );
    };
    anyhow::ensure!(
        active_execution.stage_kind == stage.as_str(),
        "test exhausted-stage restart active execution/stage mismatch"
    );

    let exhausted_controller_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM stage_team_plans plan
             JOIN stage_run_units unit
               ON unit.id=plan.stage_run_unit_id
              AND unit.operation_id=plan.operation_id
              AND unit.stage_execution_id=plan.stage_execution_id
              AND unit.organization_id=plan.organization_id
            WHERE plan.operation_id=$1
              AND plan.stage_execution_id=$2
              AND plan.stage_kind=$3
              AND plan.dynamic_request_policy->>'coordination_mode'='company_controller'
              AND (
                (
                  plan.requests_closed_at IS NULL
                  AND unit.status='running'
                  AND EXISTS (
                    SELECT 1
                      FROM stage_work_items item
                      JOIN stage_worker_runs worker
                        ON worker.work_item_id=item.id
                       AND worker.operation_id=plan.operation_id
                       AND worker.stage_execution_id=plan.stage_execution_id
                       AND worker.stage_run_unit_id=plan.stage_run_unit_id
                       AND worker.organization_id=plan.organization_id
                      JOIN stage_worker_outputs output
                        ON output.team_plan_id=plan.id
                       AND output.work_item_id=item.id
                       AND output.worker_run_id=worker.id
                     WHERE item.team_plan_id=plan.id
                       AND item.stable_key='leader:primary'
                       AND item.role=plan.leader_role
                       AND item.required_for_barrier=FALSE
                       AND item.status='exhausted'
                       AND item.terminal_at IS NOT NULL
                       AND worker.status='failed'
                       AND worker.lease_token IS NULL
                       AND worker.active_tool_call_id IS NULL
                       AND worker.checkpoint #>> '{stage_team_execution_failure,code}'=
                           'stage_team_worker_lease_expired'
                       AND output.business_disposition='blocked'
                       AND output.canonical_output->>'kind'='stage_team_attempts_exhausted'
                       AND output.canonical_output->>'failure_code'=
                           'stage_team_worker_lease_expired'
                       AND 'STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED'=
                           ANY(output.blocker_codes)
                  )
                )
                OR
                (
                  plan.requests_closed_at IS NOT NULL
                  AND plan.final_submitter_worker_run_id IS NULL
                  AND unit.status='gate_blocked'
                  AND EXISTS (
                    SELECT 1 FROM stage_work_items leader
                     WHERE leader.team_plan_id=plan.id
                       AND leader.stable_key='leader:primary'
                       AND leader.role=plan.leader_role
                       AND leader.required_for_barrier=FALSE
                       AND leader.status='superseded'
                       AND leader.started_at IS NULL
                       AND leader.terminal_at IS NOT NULL
                       AND NOT EXISTS (
                         SELECT 1 FROM stage_worker_runs leader_worker
                          WHERE leader_worker.work_item_id=leader.id
                       )
                  )
                  AND EXISTS (
                    SELECT 1
                      FROM stage_work_items producer
                      JOIN stage_worker_outputs output
                        ON output.team_plan_id=plan.id
                       AND output.work_item_id=producer.id
                     WHERE producer.team_plan_id=plan.id
                       AND producer.required_for_barrier=TRUE
                       AND producer.status='exhausted'
                       AND producer.terminal_at IS NOT NULL
                       AND output.business_disposition='blocked'
                       AND output.canonical_output->>'kind'='stage_team_attempts_exhausted'
                       AND 'STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED'=
                           ANY(output.blocker_codes)
                  )
                )
              )"#,
    )
    .bind(operation.operation_id)
    .bind(active_execution.id)
    .bind(stage.as_str())
    .fetch_one(pool)
    .await?;
    let interrupted_investigation_count: i64 = if stage == StageKind::Investigation {
        sqlx::query_scalar(
            r#"SELECT COUNT(DISTINCT plan.id)
                 FROM stage_team_plans plan
                 JOIN stage_run_units unit
                   ON unit.id=plan.stage_run_unit_id
                  AND unit.operation_id=plan.operation_id
                  AND unit.stage_execution_id=plan.stage_execution_id
                  AND unit.organization_id=plan.organization_id
                 JOIN stage_work_items item
                   ON item.team_plan_id=plan.id
                  AND item.stable_key='leader:primary'
                  AND item.role=plan.leader_role
                  AND item.required_for_barrier=FALSE
                 JOIN stage_worker_runs worker
                   ON worker.work_item_id=item.id
                  AND worker.operation_id=plan.operation_id
                  AND worker.stage_execution_id=plan.stage_execution_id
                  AND worker.stage_run_unit_id=plan.stage_run_unit_id
                  AND worker.organization_id=plan.organization_id
                 JOIN investigation_stage_run_authorities authority
                   ON authority.operation_id=plan.operation_id
                  AND authority.stage_execution_id=plan.stage_execution_id
                 JOIN investigation_main_session_sets session_set
                   ON session_set.authority_id=authority.authority_id
                  AND session_set.status='sealed'
                 JOIN investigation_main_read_sessions read_session
                   ON read_session.session_set_id=session_set.session_set_id
                  AND read_session.stage_run_unit_id=unit.id
                  AND read_session.organization_id=unit.organization_id
                 JOIN investigation_main_read_session_receipts read_receipt
                   ON read_receipt.main_read_session_id=read_session.main_read_session_id
                 JOIN investigation_pentagi_task_plans task_plan
                   ON task_plan.authority_id=authority.authority_id
                  AND task_plan.stage_run_unit_id=unit.id
                  AND task_plan.organization_id=unit.organization_id
                  AND task_plan.status='open'
                WHERE plan.operation_id=$1
                  AND plan.stage_execution_id=$2
                  AND plan.stage_kind='investigation'
                  AND plan.dynamic_request_policy->>'coordination_mode'=
                      'investigation_task_orchestrator'
                  AND plan.requests_closed_at IS NULL
                  AND plan.final_submitter_worker_run_id IS NULL
                  AND unit.status='running'
                  AND item.status='running'
                  AND worker.status IN ('queued','running','gate_blocked','recovery_required')
                  AND worker.active_tool_call_id IS NULL
                  AND (worker.lease_token IS NULL
                       OR worker.lease_expires_at<=clock_timestamp())
                  AND EXISTS(
                      SELECT 1 FROM investigation_run_work_items work
                       WHERE work.authority_id=authority.authority_id
                         AND work.stage_run_unit_id=unit.id
                         AND work.organization_id=unit.organization_id
                         AND work.work_kind='read_session'
                         AND work.current_state='completed'
                  )
                  AND EXISTS(
                      SELECT 1 FROM investigation_run_work_items work
                       WHERE work.authority_id=authority.authority_id
                         AND work.stage_run_unit_id=unit.id
                         AND work.organization_id=unit.organization_id
                         AND work.work_kind='analysis'
                         AND work.current_state='running'
                  )
                  AND NOT EXISTS(
                      SELECT 1 FROM investigation_pentagi_subtasks subtask
                       WHERE subtask.task_plan_id=task_plan.task_plan_id
                  )"#,
        )
        .bind(operation.operation_id)
        .bind(active_execution.id)
        .fetch_one(pool)
        .await?
    } else {
        0
    };
    let sealed_unapplied_investigation_count: i64 = if stage == StageKind::Investigation {
        sqlx::query_scalar(
            r#"SELECT COUNT(DISTINCT plan.id)
                 FROM stage_team_plans plan
                 JOIN stage_run_units unit
                   ON unit.id=plan.stage_run_unit_id
                  AND unit.operation_id=plan.operation_id
                  AND unit.stage_execution_id=plan.stage_execution_id
                  AND unit.organization_id=plan.organization_id
                 JOIN stage_work_items item
                   ON item.team_plan_id=plan.id
                  AND item.stable_key='leader:primary'
                  AND item.role=plan.leader_role
                  AND item.required_for_barrier=FALSE
                 JOIN stage_worker_runs worker
                   ON worker.work_item_id=item.id
                  AND worker.operation_id=plan.operation_id
                  AND worker.stage_execution_id=plan.stage_execution_id
                  AND worker.stage_run_unit_id=plan.stage_run_unit_id
                  AND worker.organization_id=plan.organization_id
                 JOIN investigation_stage_run_authorities authority
                   ON authority.operation_id=plan.operation_id
                  AND authority.stage_execution_id=plan.stage_execution_id
                 JOIN investigation_main_session_sets session_set
                   ON session_set.authority_id=authority.authority_id
                  AND session_set.status='sealed'
                 JOIN investigation_main_read_sessions read_session
                   ON read_session.session_set_id=session_set.session_set_id
                  AND read_session.stage_run_unit_id=unit.id
                  AND read_session.organization_id=unit.organization_id
                 JOIN investigation_main_read_session_receipts read_receipt
                   ON read_receipt.main_read_session_id=read_session.main_read_session_id
                 JOIN investigation_pentagi_task_plans task_plan
                   ON task_plan.authority_id=authority.authority_id
                  AND task_plan.stage_run_unit_id=unit.id
                  AND task_plan.organization_id=unit.organization_id
                  AND task_plan.subject_kind='analysis_attempt'
                  AND task_plan.status='sealed'
                 JOIN investigation_pentagi_delegation_census_seals census
                   ON census.task_plan_id=task_plan.task_plan_id
                  AND census.primary_worker_run_id=worker.id
                WHERE plan.operation_id=$1
                  AND plan.stage_execution_id=$2
                  AND plan.stage_kind='investigation'
                  AND plan.dynamic_request_policy->>'coordination_mode'=
                      'investigation_task_orchestrator'
                  AND plan.requests_closed_at IS NOT NULL
                  AND plan.final_submitter_worker_run_id IS NULL
                  AND unit.status='running'
                  AND (
                    (
                      item.status='running'
                      AND worker.status IN
                          ('queued','running','gate_blocked','recovery_required')
                      AND worker.active_tool_call_id IS NULL
                      AND (worker.lease_token IS NULL
                           OR worker.lease_expires_at<=clock_timestamp())
                    )
                    OR
                    (
                      item.status='exhausted'
                      AND item.terminal_at IS NOT NULL
                      AND worker.status='failed'
                      AND worker.terminal_at IS NOT NULL
                      AND worker.lease_token IS NULL
                      AND worker.active_tool_call_id IS NULL
                      AND worker.checkpoint #>>
                          '{stage_team_execution_failure,code}'=
                          'stage_team_worker_lease_expired'
                      AND EXISTS (
                        SELECT 1
                          FROM stage_worker_outputs output
                         WHERE output.team_plan_id=plan.id
                           AND output.work_item_id=item.id
                           AND output.worker_run_id=worker.id
                           AND output.business_disposition='blocked'
                           AND output.canonical_output->>'kind'=
                               'stage_team_attempts_exhausted'
                           AND output.canonical_output->>'failure_code'=
                               'stage_team_worker_lease_expired'
                           AND 'STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED'=
                               ANY(output.blocker_codes)
                      )
                    )
                  )
                  AND EXISTS(
                      SELECT 1 FROM investigation_pentagi_pipeline_events event
                       WHERE event.task_plan_id=task_plan.task_plan_id
                         AND event.event_kind='primary_synthesis'
                         AND event.actor_worker_run_id=worker.id
                  )
                  AND EXISTS(
                      SELECT 1
                        FROM investigation_run_work_items work
                        JOIN investigation_run_work_state_events event
                          ON event.event_id=work.latest_event_id
                       WHERE work.authority_id=authority.authority_id
                         AND work.stage_run_unit_id=unit.id
                         AND work.organization_id=unit.organization_id
                         AND work.work_kind='analysis'
                         AND work.current_state='blocked'
                         AND event.to_state='blocked'
                         AND event.reason_code=
                             'investigation_analysis_host_authority_mismatch'
                  )
                  AND NOT EXISTS(
                      SELECT 1
                        FROM investigation_hypothesis_compilation_decisions decision
                       WHERE decision.operation_id=plan.operation_id
                         AND decision.stage_execution_id=plan.stage_execution_id
                         AND decision.stage_run_unit_id=plan.stage_run_unit_id
                         AND decision.organization_id=plan.organization_id
                  )
                  AND NOT EXISTS(
                      SELECT 1 FROM hypothesis_verification_tasks task
                       WHERE task.operation_id=plan.operation_id
                         AND task.stage_execution_id=plan.stage_execution_id
                         AND task.stage_run_unit_id=plan.stage_run_unit_id
                         AND task.organization_id=plan.organization_id
                  )"#,
        )
        .bind(operation.operation_id)
        .bind(active_execution.id)
        .fetch_one(pool)
        .await?
    } else {
        0
    };
    let frozen_organization_count = i64::try_from(frozen.units.len()).unwrap_or(i64::MAX);
    let sealed_synthesis_retry_rows: Vec<(
        uuid::Uuid,
        uuid::Uuid,
        i64,
        uuid::Uuid,
        i64,
        i64,
        uuid::Uuid,
    )> = if stage == StageKind::Investigation {
        sqlx::query_as(
            r#"SELECT plan.id,item.id,item.row_version,worker.id,worker.attempt_epoch,
                      worker.checkpoint_version,output.id
                 FROM stage_team_plans plan
                 JOIN stage_run_units unit ON unit.id=plan.stage_run_unit_id
                 JOIN stage_work_items item
                   ON item.team_plan_id=plan.id
                  AND item.stable_key='leader:primary'
                  AND item.role=plan.leader_role
                  AND item.required_for_barrier=FALSE
                 JOIN stage_worker_runs worker ON worker.work_item_id=item.id
                 JOIN stage_worker_outputs output
                   ON output.team_plan_id=plan.id
                  AND output.work_item_id=item.id
                  AND output.worker_run_id=worker.id
                 JOIN investigation_stage_run_authorities authority
                   ON authority.operation_id=plan.operation_id
                  AND authority.stage_execution_id=plan.stage_execution_id
                 JOIN investigation_pentagi_task_plans task_plan
                   ON task_plan.authority_id=authority.authority_id
                  AND task_plan.stage_run_unit_id=unit.id
                  AND task_plan.organization_id=unit.organization_id
                  AND task_plan.subject_kind='analysis_attempt'
                  AND task_plan.status='open'
                 JOIN investigation_refiner_plan_ledger_seals refiner_seal
                   ON refiner_seal.task_plan_id=task_plan.task_plan_id
                WHERE plan.operation_id=$1
                  AND plan.stage_execution_id=$2
                  AND plan.stage_kind='investigation'
                  AND plan.dynamic_request_policy->>'coordination_mode'=
                      'investigation_task_orchestrator'
                  AND plan.requests_closed_at IS NULL
                  AND plan.final_submitter_worker_run_id IS NULL
                  AND unit.status='running'
                  AND item.status='exhausted'
                  AND item.terminal_at IS NOT NULL
                  AND worker.status='failed'
                  AND worker.terminal_at IS NOT NULL
                  AND worker.lease_token IS NULL
                  AND worker.active_tool_call_id IS NULL
                  AND worker.checkpoint #>>
                      '{stage_team_execution_failure,code}'=
                      'stage_team_worker_lease_expired'
                  AND output.business_disposition='blocked'
                  AND output.canonical_output->>'kind'=
                      'stage_team_attempts_exhausted'
                  AND output.canonical_output->>'failure_code'=
                      'stage_team_worker_lease_expired'
                  AND 'STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED'=
                      ANY(output.blocker_codes)
                  AND EXISTS(
                      SELECT 1 FROM investigation_run_work_items work
                       WHERE work.authority_id=authority.authority_id
                         AND work.stage_run_unit_id=unit.id
                         AND work.organization_id=unit.organization_id
                         AND work.work_kind='analysis'
                         AND work.current_state='running'
                  )
                  AND NOT EXISTS(
                      SELECT 1 FROM investigation_pentagi_pipeline_events event
                       WHERE event.task_plan_id=task_plan.task_plan_id
                         AND event.event_kind='primary_synthesis'
                  )
                  AND NOT EXISTS(
                      SELECT 1 FROM investigation_hypothesis_compilation_decisions decision
                       WHERE decision.operation_id=plan.operation_id
                         AND decision.stage_execution_id=plan.stage_execution_id
                         AND decision.stage_run_unit_id=plan.stage_run_unit_id
                         AND decision.organization_id=plan.organization_id
                  )
                  AND NOT EXISTS(
                      SELECT 1 FROM hypothesis_verification_tasks task
                       WHERE task.operation_id=plan.operation_id
                         AND task.stage_execution_id=plan.stage_execution_id
                         AND task.stage_run_unit_id=plan.stage_run_unit_id
                         AND task.organization_id=plan.organization_id
                  )"#,
        )
        .bind(operation.operation_id)
        .bind(active_execution.id)
        .fetch_all(pool)
        .await?
    } else {
        Vec::new()
    };
    if i64::try_from(sealed_synthesis_retry_rows.len()).unwrap_or(i64::MAX)
        == frozen_organization_count
    {
        let mut tx = pool.begin().await?;
        for (plan_id, item_id, _, _, _, _, _) in sealed_synthesis_retry_rows {
            let recovery_v1_item_id = uuid::Uuid::new_v5(
                &item_id,
                b"sealed-investigation-synthesis-recovery-primary-v1",
            );
            let recovery_stable_key = format!("leader:synthesis-recovery:{item_id}");
            let recovery_v1 = sqlx::query_as::<_, (String, String)>(
                "SELECT kind,status FROM stage_work_items WHERE id=$1 AND team_plan_id=$2 FOR UPDATE",
            )
            .bind(recovery_v1_item_id)
            .bind(plan_id)
            .fetch_optional(&mut *tx)
            .await?;
            let (recovery_item_id, recovery_kind, recovery_generation) = match recovery_v1 {
                None => (recovery_v1_item_id, None, "v1"),
                Some((kind, status)) if status == "queued" || status == "running" => {
                    anyhow::ensure!(
                        kind != "investigation_primary_recovery",
                        "sealed Investigation synthesis recovery v1 kind drifted"
                    );
                    (recovery_v1_item_id, None, "v1")
                }
                Some((_kind, status)) if status == "exhausted" => {
                    let recovery_v1_failure_exact: bool = sqlx::query_scalar(
                        r#"SELECT EXISTS(
                               SELECT 1
                                 FROM stage_work_items recovery
                                 JOIN stage_work_items source
                                   ON source.id=$2 AND source.team_plan_id=recovery.team_plan_id
                                 JOIN stage_worker_runs worker
                                   ON worker.work_item_id=recovery.id
                                 JOIN stage_worker_outputs output
                                   ON output.team_plan_id=recovery.team_plan_id
                                  AND output.work_item_id=recovery.id
                                  AND output.worker_run_id=worker.id
                                WHERE recovery.id=$1 AND recovery.team_plan_id=$3
                                  AND recovery.kind=source.kind
                                  AND recovery.stable_key=$4
                                  AND recovery.role=source.role
                                  AND recovery.input_manifest_hash=source.input_manifest_hash
                                  AND recovery.input_refs=source.input_refs
                                  AND recovery.required_for_barrier=FALSE
                                  AND recovery.conflict_key IS NULL
                                  AND recovery.priority=source.priority
                                  AND recovery.attempt_policy=source.attempt_policy
                                  AND recovery.budget=source.budget
                                  AND recovery.output_schema=source.output_schema
                                  AND recovery.created_by='server_seed'
                                  AND recovery.status='exhausted'
                                  AND recovery.terminal_at IS NOT NULL
                                  AND worker.status='failed'
                                  AND worker.terminal_at IS NOT NULL
                                  AND worker.lease_token IS NULL
                                  AND worker.active_tool_call_id IS NULL
                                  AND worker.checkpoint #>>
                                      '{stage_team_execution_failure,code}'=
                                      'stage_team_worker_lease_expired'
                                  AND output.business_disposition='blocked'
                                  AND output.canonical_output->>'kind'=
                                      'stage_team_attempts_exhausted'
                                  AND output.canonical_output->>'failure_code'=
                                      'stage_team_worker_lease_expired'
                                  AND 'STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED'=
                                      ANY(output.blocker_codes)
                                  AND (SELECT COUNT(*) FROM stage_worker_runs all_worker
                                        WHERE all_worker.work_item_id=recovery.id)=1
                                  AND (SELECT COUNT(*) FROM stage_worker_outputs all_output
                                        WHERE all_output.work_item_id=recovery.id)=1
                           )"#,
                    )
                    .bind(recovery_v1_item_id)
                    .bind(item_id)
                    .bind(plan_id)
                    .bind(&recovery_stable_key)
                    .fetch_one(&mut *tx)
                    .await?;
                    anyhow::ensure!(
                        recovery_v1_failure_exact,
                        "sealed Investigation synthesis recovery v1 exhaustion witness is not exact"
                    );
                    (
                        uuid::Uuid::new_v5(
                            &recovery_v1_item_id,
                            b"sealed-investigation-synthesis-recovery-primary-v2",
                        ),
                        Some("investigation_primary_recovery".to_string()),
                        "v2",
                    )
                }
                Some((_kind, status)) => anyhow::bail!(
                    "sealed Investigation synthesis recovery v1 has non-restartable status {status}"
                ),
            };
            sqlx::query(
                r#"INSERT INTO stage_work_items(
                       id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                       scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
                       input_manifest_hash,input_refs,required_for_barrier,conflict_key,
                       priority,status,attempt_policy,budget,output_schema,created_by
                   )
                   SELECT $3,source.team_plan_id,source.operation_id,
                          source.stage_execution_id,source.stage_run_unit_id,
                          source.scope_snapshot_id,source.organization_id,
                          source.dispatch_epoch,COALESCE($4,source.kind),
                          'leader:synthesis-recovery:' || source.id::TEXT,
                          source.role,source.input_manifest_hash,source.input_refs,FALSE,NULL,
                          source.priority,'queued',source.attempt_policy,source.budget,
                          source.output_schema,'server_seed'
                     FROM stage_work_items source
                    WHERE source.id=$1 AND source.team_plan_id=$2
                      AND source.stable_key='leader:primary'
                      AND source.required_for_barrier=FALSE
                      AND source.status='exhausted'
                   ON CONFLICT (id) DO NOTHING"#,
            )
            .bind(item_id)
            .bind(plan_id)
            .bind(recovery_item_id)
            .bind(recovery_kind.as_deref())
            .execute(&mut *tx)
            .await?;
            let recovery_exact: bool = sqlx::query_scalar(
                r#"SELECT EXISTS(
                       SELECT 1
                         FROM stage_work_items recovery
                         JOIN stage_work_items source
                           ON source.id=$4 AND source.team_plan_id=recovery.team_plan_id
                        WHERE recovery.id=$1 AND recovery.team_plan_id=$2
                          AND recovery.stable_key=$3
                          AND recovery.role=(SELECT leader_role FROM stage_team_plans WHERE id=$2)
                          AND recovery.kind=COALESCE($5,source.kind)
                          AND recovery.input_manifest_hash=source.input_manifest_hash
                          AND recovery.input_refs=source.input_refs
                          AND recovery.required_for_barrier=FALSE
                          AND recovery.conflict_key IS NULL
                          AND recovery.priority=source.priority
                          AND recovery.attempt_policy=source.attempt_policy
                          AND recovery.budget=source.budget
                          AND recovery.output_schema=source.output_schema
                          AND recovery.created_by='server_seed'
                          AND recovery.status IN ('queued','running')
                          AND recovery.terminal_at IS NULL
                   )"#,
            )
            .bind(recovery_item_id)
            .bind(plan_id)
            .bind(&recovery_stable_key)
            .bind(item_id)
            .bind(recovery_kind.as_deref())
            .fetch_one(&mut *tx)
            .await?;
            anyhow::ensure!(
                recovery_exact,
                "sealed Investigation synthesis recovery {recovery_generation} Primary was not inserted exactly"
            );
        }
        tx.commit().await?;
        eprintln!(
            "[stage-run-resume] admitted exact sealed Investigation synthesis recovery generation without replaying child WorkItems: operation={} execution={}",
            operation.operation_id, active_execution.id
        );
        return Ok(());
    }
    let exhausted_stage_exact = exhausted_controller_count == frozen_organization_count;
    let interrupted_investigation_exact =
        interrupted_investigation_count == frozen_organization_count;
    let sealed_unapplied_investigation_exact =
        sealed_unapplied_investigation_count == frozen_organization_count;
    let restartable_investigation_exact =
        interrupted_investigation_exact || sealed_unapplied_investigation_exact;
    anyhow::ensure!(
        exhausted_stage_exact || restartable_investigation_exact,
        "test stage restart requires one exact exhausted Controller, sealed pre-subtask Investigation interruption, or sealed compiler-unapplied Investigation interruption per frozen organization"
    );
    if exhausted_stage_exact && stage == StageKind::VulnTriage {
        let exhausted_controllers: Vec<(uuid::Uuid, uuid::Uuid, uuid::Uuid, uuid::Uuid, i64, i64)> =
            sqlx::query_as(
                r#"SELECT plan.id,item.id,unit.id,worker.id,
                          worker.attempt_epoch,worker.checkpoint_version
                     FROM stage_team_plans plan
                     JOIN stage_run_units unit
                       ON unit.id=plan.stage_run_unit_id
                      AND unit.operation_id=plan.operation_id
                      AND unit.stage_execution_id=plan.stage_execution_id
                      AND unit.organization_id=plan.organization_id
                     JOIN stage_work_items item
                       ON item.team_plan_id=plan.id
                      AND item.stable_key='leader:primary'
                      AND item.role=plan.leader_role
                      AND item.required_for_barrier=FALSE
                     JOIN stage_worker_runs worker
                       ON worker.work_item_id=item.id
                      AND worker.operation_id=plan.operation_id
                      AND worker.stage_execution_id=plan.stage_execution_id
                      AND worker.stage_run_unit_id=plan.stage_run_unit_id
                      AND worker.organization_id=plan.organization_id
                    WHERE plan.operation_id=$1 AND plan.stage_execution_id=$2
                      AND plan.stage_kind='vuln_triage'
                      AND plan.dynamic_request_policy->>'coordination_mode'='company_controller'
                      AND plan.requests_closed_at IS NULL
                      AND plan.final_submitter_worker_run_id IS NULL
                      AND unit.status='running'
                      AND item.status='exhausted' AND item.terminal_at IS NOT NULL
                      AND worker.status='failed' AND worker.terminal_at IS NOT NULL
                      AND worker.lease_token IS NULL AND worker.active_tool_call_id IS NULL
                      AND worker.checkpoint #>> '{stage_team_execution_failure,code}'=
                          'stage_team_worker_lease_expired'
                      AND (SELECT COUNT(*) FROM stage_worker_outputs output
                            WHERE output.team_plan_id=plan.id
                              AND output.work_item_id=item.id
                              AND output.worker_run_id=worker.id
                              AND output.business_disposition='blocked'
                              AND output.canonical_output->>'kind'=
                                  'stage_team_attempts_exhausted'
                              AND output.canonical_output->>'failure_code'=
                                  'stage_team_worker_lease_expired'
                              AND 'STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED'=
                                  ANY(output.blocker_codes))=1
                    ORDER BY plan.organization_id"#,
            )
            .bind(operation.operation_id)
            .bind(active_execution.id)
            .fetch_all(pool)
            .await?;
        anyhow::ensure!(
            i64::try_from(exhausted_controllers.len()).unwrap_or(i64::MAX)
                == frozen_organization_count,
            "test Vuln restart lost its exact exhausted Controller census"
        );
        let mut sealed_cells = 0usize;
        let mut found_cells = 0usize;
        let mut blocked_cells = 0usize;
        for (plan_id, item_id, unit_id, worker_id, attempt_epoch, checkpoint_version) in
            exhausted_controllers
        {
            let sealed = golish_db::repo::runtime_memory_tx::seal_exhausted_vuln_residual_outcomes(
                pool,
                &golish_db::repo::runtime_memory_tx::SealExhaustedVulnResidualOutcomesRow {
                    fence: golish_db::repo::runtime_memory_tx::RuntimeMemoryTxFence {
                        operation_id: operation.operation_id,
                        stage_execution_id: active_execution.id,
                        stage_run_unit_id: unit_id,
                        worker_run_id: worker_id,
                        // Nil is a server-only compatibility sentinel. The DB
                        // transaction accepts it solely with the exact exhausted
                        // Controller and completed producer census above.
                        lease_token: uuid::Uuid::nil(),
                        attempt_epoch,
                        expected_checkpoint_version: checkpoint_version,
                    },
                    stage_team_plan_id: plan_id,
                    leader_work_item_id: item_id,
                    derive_terminal_leader_fence: false,
                    expected_attempt_ordinal: 3,
                    expected_anonymous_attempt_ordinal: 2,
                },
            )
            .await
            .map_err(anyhow::Error::new)
            .context("seal exhausted Vuln evidence before controlled stage-shell restart")?;
            sealed_cells = sealed_cells.saturating_add(sealed.sealed_cells);
            found_cells = found_cells.saturating_add(sealed.found_cells);
            blocked_cells = blocked_cells.saturating_add(sealed.blocked_cells);
        }
        eprintln!(
            "[stage-run-resume] sealed {sealed_cells} exhausted Vuln cell(s) before controlled runtime-shell restart ({found_cells} positive, {blocked_cells} inconclusive residual)"
        );
    }
    let live_workers: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM stage_worker_runs
            WHERE operation_id=$1 AND stage_execution_id=$2
              AND (active_tool_call_id IS NOT NULL
                   OR ($3=FALSE AND (lease_token IS NOT NULL
                       OR status IN ('queued','running','waiting_background','recovery_required')))
                   OR ($3=TRUE AND lease_token IS NOT NULL
                       AND lease_expires_at>clock_timestamp()))"#,
    )
    .bind(operation.operation_id)
    .bind(active_execution.id)
    .bind(restartable_investigation_exact)
    .fetch_one(pool)
    .await?;
    anyhow::ensure!(
        live_workers == 0,
        "test exhausted-stage restart refused while live/recoverable Workers remain"
    );

    let replacement_stage_execution_id = uuid::Uuid::new_v4();
    golish_db::repo::runtime_memory_tx::supersede_stage_checkpoint(
        pool,
        &golish_db::repo::runtime_memory_tx::SupersedeStageCheckpointRow {
            operation_id: operation.operation_id,
            expected_active_stage_execution_id: Some(active_execution.id),
            expected_current_stage: operation.current_stage.clone(),
            selected_stage: operation.current_stage.clone(),
            affected_stage_kinds: vec![operation.current_stage.clone()],
            next_state_blob: operation.state_blob,
            replacement_specialist: golish_agent_kit::harness::load_embedded_stage_spec(stage)
                .ok()
                .and_then(|spec| spec.specialist)
                .filter(|specialist| !specialist.trim().is_empty()),
            replacement_stage_execution_id: Some(replacement_stage_execution_id),
            fact_purge: None,
            finalizer_recovery_witness: None,
        },
    )
    .await
    .map_err(anyhow::Error::new)
    .context("atomically restart exhausted test stage runtime")?;
    eprintln!(
        "[stage-run-resume] restarted exhausted test stage runtime: operation={} stage={} replacement_execution={}",
        operation.operation_id,
        stage.as_str(),
        replacement_stage_execution_id
    );
    Ok(())
}

/// Parse `--from`/`--to`/`--only` into `(from, to)` stages.
fn resolve_from_to(args: &Args) -> Result<(Option<StageKind>, StageKind)> {
    let parse = |s: &str| StageKind::try_parse(s).ok_or_else(|| anyhow!("unknown stage: {s}"));
    if let Some(only) = args.only.as_deref() {
        let s = parse(only)?;
        return Ok((Some(s), s));
    }
    let to = args
        .to
        .as_deref()
        .ok_or_else(|| anyhow!("--stage-run requires --to <stage> (or --only <stage>)"))?;
    let to = parse(to)?;
    let from = match args.from.as_deref() {
        Some(f) => Some(parse(f)?),
        None => None,
    };
    Ok((from, to))
}

fn is_active_stage(stage: StageKind) -> bool {
    matches!(
        stage,
        StageKind::ExternalAttackSurface
            | StageKind::Enumeration
            | StageKind::VulnTriage
            | StageKind::AttackCandidate
            | StageKind::Verification
            | StageKind::ApplicationUnderstanding
            | StageKind::Investigation
            | StageKind::AccessValidation
            | StageKind::InternalDiscovery
            | StageKind::ObjectivePathing
            | StageKind::ObjectiveSimulation
    )
}

/// A fresh slice that bypasses Scoping and can reach active recon may not borrow
/// durable target rows merely because `--org` reused an existing organization.
/// The caller must repeat at least one exact `--target` in this invocation so
/// the trusted seed path upgrades/freezes current launch authority. A full flow
/// beginning at Scoping revalidates the current operation's review lifecycle;
/// a passive-only Target Intel slice remains legal without a target.
fn validate_fresh_slice_target_intake(
    entry_stage: StageKind,
    allowlist: &HashSet<StageKind>,
    targets: &[String],
) -> Result<()> {
    let direct_target_authority_required =
        entry_stage != StageKind::Scoping && allowlist.iter().copied().any(is_active_stage);
    if direct_target_authority_required && !targets.iter().any(|target| !target.trim().is_empty()) {
        anyhow::bail!(
            "fresh slice starting at '{}' and crossing into active recon requires at least one exact --target from this CLI invocation; an existing --org row is engagement context, not current target authority",
            entry_stage.as_str()
        );
    }
    Ok(())
}

struct StageRunDbConfig {
    config: golish_db::DbConfig,
    temp_dir: Option<tempfile::TempDir>,
}

impl StageRunDbConfig {
    fn keep_temp_dir(&mut self) -> Option<PathBuf> {
        self.temp_dir.take().map(tempfile::TempDir::keep)
    }
}

fn prepare_stage_run_db(args: &Args) -> Result<StageRunDbConfig> {
    let mut config = golish_db::DbConfig::default();
    if let Some(database) = args.stage_run_test_database.as_deref() {
        let valid = database.starts_with("golish_gatefix_")
            && database.len() <= 63
            && database
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
        anyhow::ensure!(
            valid,
            "--stage-run-test-database accepts only lowercase golish_gatefix_* database names"
        );
        config.database = database.to_string();
    }
    if let Some(pg_data_dir) = args.stage_run_resume_pgdata.as_ref() {
        anyhow::ensure!(
            pg_data_dir.is_absolute(),
            "--stage-run-resume-pgdata must be an absolute path"
        );
        anyhow::ensure!(
            pg_data_dir.join("PG_VERSION").is_file(),
            "--stage-run-resume-pgdata is not an initialized PostgreSQL data directory: {}",
            pg_data_dir.display()
        );
        config.pg_data_dir = pg_data_dir.clone();
        config.port = allocate_local_port().context("allocate retained-resume PostgreSQL port")?;
        return Ok(StageRunDbConfig {
            config,
            temp_dir: None,
        });
    }
    if !args.ephemeral_db {
        return Ok(StageRunDbConfig {
            config,
            temp_dir: None,
        });
    }

    let temp_dir = tempfile::Builder::new()
        .prefix("golish-stage-run-db-")
        .tempdir()
        .context("create ephemeral stage-run database directory")?;
    config.pg_data_dir = temp_dir.path().join("pgdata");
    config.port = allocate_local_port().context("allocate ephemeral PostgreSQL port")?;

    Ok(StageRunDbConfig {
        config,
        temp_dir: Some(temp_dir),
    })
}

fn allocate_local_port() -> Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).context("bind local ephemeral port")?;
    Ok(listener
        .local_addr()
        .context("read local ephemeral port")?
        .port())
}

fn local_port_is_open(port: u16) -> bool {
    std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
}

fn maybe_keep_ephemeral_db(stage_db: &mut StageRunDbConfig, keep: bool) {
    if !keep {
        return;
    }
    if let Some(path) = stage_db.keep_temp_dir() {
        eprintln!(
            "[stage-run] kept ephemeral database directory: {}",
            path.display()
        );
    }
}

const STAGE_RUN_DB_DIAGNOSTIC_ACK_ENV: &str = "GOLISH_STAGE_RUN_DB_DIAGNOSTIC_ACK";

/// Keep the owned ephemeral PostgreSQL process alive long enough for the
/// external run-tree diagnostic to query the exact operation. The wrapper
/// acknowledges completion by creating a fresh one-shot file; without the
/// explicit environment variable this seam is completely dormant.
async fn wait_for_live_db_diagnostic(stage_db: &StageRunDbConfig) -> Result<()> {
    let Some(ack_path) = std::env::var_os(STAGE_RUN_DB_DIAGNOSTIC_ACK_ENV).map(PathBuf::from)
    else {
        return Ok(());
    };
    anyhow::ensure!(
        stage_db.temp_dir.is_some(),
        "live DB diagnostic requires an owned ephemeral stage-run database"
    );
    anyhow::ensure!(
        ack_path.is_absolute(),
        "live DB diagnostic acknowledgement path must be absolute"
    );
    anyhow::ensure!(
        !ack_path.exists(),
        "live DB diagnostic acknowledgement path already exists"
    );

    println!(
        "{}",
        serde_json::json!({
            "type": "db_smoke_diagnostic_ready",
            "dbUrl": stage_db.config.connection_string(),
        })
    );
    std::io::stdout()
        .flush()
        .context("flush live DB diagnostic handshake")?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        if ack_path.is_file() {
            return Ok(());
        }
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for live DB diagnostic acknowledgement"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn bootstrap_ephemeral_joint_rollout(
    pool: &sqlx::PgPool,
    args: &Args,
    stage_db: &StageRunDbConfig,
) -> Result<()> {
    let Some(target_rank) = args.stage_run_test_joint_rank else {
        return Ok(());
    };
    anyhow::ensure!(
        args.ephemeral_db && stage_db.temp_dir.is_some(),
        "stage-run test rollout bootstrap requires an owned ephemeral database"
    );
    let target_mode = match target_rank {
        5 => "registry_authoritative_legacy_projection",
        6 => "new_only",
        _ => anyhow::bail!("stage-run test rollout bootstrap accepts only joint rank 5 or 6"),
    };

    let mut tx = pool
        .begin()
        .await
        .context("begin ephemeral rollout bootstrap")?;
    let operation_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM operation_state")
        .fetch_one(&mut *tx)
        .await
        .context("verify empty ephemeral operation catalog")?;
    anyhow::ensure!(
        operation_count == 0,
        "ephemeral rollout bootstrap refused after operation creation"
    );
    let runtime_initial: (String, i16, i64) = sqlx::query_as(
        "SELECT contract,contract_rank,row_version FROM runtime_memory_rollout WHERE singleton_id=1 FOR UPDATE",
    )
    .fetch_one(&mut *tx)
    .await
    .context("lock initial Runtime Memory rollout")?;
    let attack_initial: (String, i16, i64) = sqlx::query_as(
        "SELECT contract,rank,row_version FROM attack_execution_rollout WHERE singleton=TRUE FOR UPDATE",
    )
    .fetch_one(&mut *tx)
    .await
    .context("lock initial Attack Execution rollout")?;
    let enumeration_initial: (String, i64) = sqlx::query_as(
        "SELECT new_operation_contract,generation FROM enumeration_analysis_rollout WHERE singleton=TRUE FOR UPDATE",
    )
    .fetch_one(&mut *tx)
    .await
    .context("lock initial Enumeration Analysis rollout")?;
    let tool_initial: (String, i64) = sqlx::query_as(
        "SELECT new_operation_contract,row_version FROM tool_truth_rollout WHERE singleton=TRUE FOR UPDATE",
    )
    .fetch_one(&mut *tx)
    .await
    .context("lock initial Tool Truth rollout")?;
    let investigation_initial: (String, String, i16, i64) = sqlx::query_as(
        "SELECT contract_version,rollout_mode,mode_rank,row_version FROM investigation_rollout WHERE singleton=TRUE FOR UPDATE",
    )
    .fetch_one(&mut *tx)
    .await
    .context("lock initial Investigation rollout")?;
    anyhow::ensure!(
        runtime_initial == ("dual_write_legacy_read".to_owned(), 1, 1)
            && attack_initial == ("dual_write_read_legacy".to_owned(), 1, 1)
            && enumeration_initial == ("legacy_v1".to_owned(), 0)
            && tool_initial == ("legacy_v1".to_owned(), 0)
            && investigation_initial
                == (
                    "legacy_candidate_v1".to_owned(),
                    "legacy_only".to_owned(),
                    0,
                    0,
                ),
        "ephemeral rollout bootstrap requires pristine migration defaults"
    );

    for statement in [
        "ALTER TABLE runtime_memory_rollout DISABLE TRIGGER runtime_memory_rollout_forward_only",
        "ALTER TABLE runtime_memory_rollout DISABLE TRIGGER zz_runtime_memory_rollout_attestation_gate",
        "ALTER TABLE runtime_memory_rollout DISABLE TRIGGER zz_runtime_memory_rollout_promotion_receipt",
        "ALTER TABLE attack_execution_rollout DISABLE TRIGGER attack_execution_rollout_forward_only",
        "ALTER TABLE attack_execution_rollout DISABLE TRIGGER zz_attack_execution_rollout_promotion_receipt",
        "ALTER TABLE enumeration_analysis_rollout DISABLE TRIGGER enumeration_analysis_rollout_mutation_guard",
        "ALTER TABLE tool_truth_rollout DISABLE TRIGGER tool_truth_rollout_direct_mutation_guard",
        "ALTER TABLE investigation_rollout DISABLE TRIGGER investigation_rollout_direct_mutation_guard",
    ] {
        sqlx::query(statement)
            .execute(&mut *tx)
            .await
            .context("disable isolated rollout fixture guard")?;
    }
    let runtime_updated = sqlx::query(
        "UPDATE runtime_memory_rollout SET contract='v2_only',contract_rank=3,row_version=3,updated_at=statement_timestamp() WHERE singleton_id=1 AND contract='dual_write_legacy_read' AND contract_rank=1 AND row_version=1",
    )
    .execute(&mut *tx)
    .await
    .context("select Runtime Memory V2 in ephemeral database")?;
    let attack_updated = sqlx::query(
        "UPDATE attack_execution_rollout SET contract='v2_only',rank=3,row_version=3,updated_at=statement_timestamp() WHERE singleton=TRUE AND contract='dual_write_read_legacy' AND rank=1 AND row_version=1",
    )
    .execute(&mut *tx)
    .await
    .context("select Attack Execution V2 in ephemeral database")?;
    let enumeration_updated = sqlx::query(
        "UPDATE enumeration_analysis_rollout SET new_operation_contract='agent_team_v2',generation=2,updated_at=statement_timestamp() WHERE singleton=TRUE AND new_operation_contract='legacy_v1' AND generation=0",
    )
    .execute(&mut *tx)
    .await
    .context("select Enumeration Analysis V2 in ephemeral database")?;
    let tool_updated = sqlx::query(
        "UPDATE tool_truth_rollout SET new_operation_contract='receipt_v1',row_version=1,updated_at=statement_timestamp() WHERE singleton=TRUE AND new_operation_contract='legacy_v1' AND row_version=0",
    )
    .execute(&mut *tx)
    .await
    .context("select receipt Tool Truth in ephemeral database")?;
    let investigation_updated = sqlx::query(
        r#"UPDATE investigation_rollout
              SET contract_version='hypothesis_registry_v1',rollout_mode=$1,
                  mode_rank=$2,row_version=1,updated_at=statement_timestamp()
            WHERE singleton=TRUE AND contract_version='legacy_candidate_v1'
              AND rollout_mode='legacy_only' AND mode_rank=0 AND row_version=0"#,
    )
    .bind(target_mode)
    .bind(if target_rank == 5 { 3_i16 } else { 4_i16 })
    .execute(&mut *tx)
    .await
    .context("select unified Investigation in ephemeral database")?;
    anyhow::ensure!(
        runtime_updated.rows_affected() == 1
            && attack_updated.rows_affected() == 1
            && enumeration_updated.rows_affected() == 1
            && tool_updated.rows_affected() == 1
            && investigation_updated.rows_affected() == 1,
        "ephemeral rollout bootstrap CAS changed"
    );
    for statement in [
        "ALTER TABLE investigation_rollout ENABLE TRIGGER investigation_rollout_direct_mutation_guard",
        "ALTER TABLE tool_truth_rollout ENABLE TRIGGER tool_truth_rollout_direct_mutation_guard",
        "ALTER TABLE enumeration_analysis_rollout ENABLE TRIGGER enumeration_analysis_rollout_mutation_guard",
        "ALTER TABLE attack_execution_rollout ENABLE TRIGGER zz_attack_execution_rollout_promotion_receipt",
        "ALTER TABLE attack_execution_rollout ENABLE TRIGGER attack_execution_rollout_forward_only",
        "ALTER TABLE runtime_memory_rollout ENABLE TRIGGER zz_runtime_memory_rollout_promotion_receipt",
        "ALTER TABLE runtime_memory_rollout ENABLE TRIGGER zz_runtime_memory_rollout_attestation_gate",
        "ALTER TABLE runtime_memory_rollout ENABLE TRIGGER runtime_memory_rollout_forward_only",
    ] {
        sqlx::query(statement)
            .execute(&mut *tx)
            .await
            .context("restore isolated rollout fixture guard")?;
    }
    let execution_selected: (String, i16, i64, String, i16, i64, String, i64) = sqlx::query_as(
        r#"SELECT runtime.contract,runtime.contract_rank,runtime.row_version,
                  attack.contract,attack.rank,attack.row_version,
                  enumeration.new_operation_contract,enumeration.generation
             FROM runtime_memory_rollout runtime
             CROSS JOIN attack_execution_rollout attack
             CROSS JOIN enumeration_analysis_rollout enumeration
            WHERE runtime.singleton_id=1 AND attack.singleton=TRUE
              AND enumeration.singleton=TRUE"#,
    )
    .fetch_one(&mut *tx)
    .await
    .context("verify ephemeral execution rollout selection")?;
    anyhow::ensure!(
        execution_selected
            == (
                "v2_only".to_owned(),
                3,
                3,
                "v2_only".to_owned(),
                3,
                3,
                "agent_team_v2".to_owned(),
                2,
            ),
        "ephemeral execution rollout verification failed"
    );
    let authority_selected: (String, i64, String, String, i16, i64, Option<i16>) = sqlx::query_as(
        r#"SELECT tool.new_operation_contract,tool.row_version,
                  investigation.contract_version,investigation.rollout_mode,
                  investigation.mode_rank,investigation.row_version,
                  operation_joint_contract_rank(tool.new_operation_contract,
                      investigation.contract_version,investigation.rollout_mode)
             FROM tool_truth_rollout tool
             CROSS JOIN investigation_rollout investigation
            WHERE tool.singleton=TRUE AND investigation.singleton=TRUE"#,
    )
    .fetch_one(&mut *tx)
    .await
    .context("verify ephemeral authority rollout selection")?;
    anyhow::ensure!(
        authority_selected
            == (
                "receipt_v1".to_owned(),
                1,
                "hypothesis_registry_v1".to_owned(),
                target_mode.to_owned(),
                if target_rank == 5 { 3 } else { 4 },
                1,
                Some(target_rank),
            ),
        "ephemeral authority rollout verification failed"
    );
    tx.commit()
        .await
        .context("commit ephemeral rollout bootstrap")?;
    eprintln!(
        "[stage-run] isolated rollout bootstrap: runtime=v2_only attack=v2_only enumeration=agent_team_v2 joint_rank={target_rank} mode={target_mode} topology=unified_investigation_v1"
    );
    Ok(())
}

fn deterministic_investigation_settings(
    endpoint: &url::Url,
) -> Result<golish_settings::GolishSettings> {
    let loopback_host = match endpoint.host() {
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        Some(url::Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        None => false,
    };
    anyhow::ensure!(
        endpoint.scheme() == "http"
            && loopback_host
            && endpoint.username().is_empty()
            && endpoint.password().is_none()
            && endpoint.query().is_none()
            && endpoint.fragment().is_none(),
        "deterministic Investigation LLM endpoint must be a credential-free loopback HTTP URL"
    );

    // Start from defaults instead of the user's loaded settings. This removes
    // remote provider credentials, MCP settings, telemetry, embeddings and
    // inherited proxy configuration from the isolated process before any
    // application service or model client is constructed.
    let mut settings = golish_settings::GolishSettings::default();
    settings.ai.default_provider = golish_settings::schema::AiProvider::Deepseek;
    settings.ai.default_model = "golish-investigation-scripted-v1".to_string();
    settings.ai.deepseek.api_key = Some("local-scripted-fixture".to_string());
    settings.ai.deepseek.base_url = Some(endpoint.as_str().trim_end_matches('/').to_string());
    settings.ai.sub_agent_models.clear();
    settings.ai.model_overrides.clear();
    settings.ai.summarizer_model = None;
    settings.ai.research_provider = None;
    settings.ai.research_model = None;
    settings.context.enabled = false;
    settings.telemetry.langfuse.enabled = false;
    settings.mcp_servers.clear();
    settings.network = golish_settings::schema::NetworkSettings::default();
    settings.api_keys = golish_settings::schema::ApiKeysSettings::default();
    Ok(settings)
}

/// Headless entry point for `golish --stage-run`.
pub async fn run(mut args: Args) -> Result<()> {
    if args.stage_run_test_investigation_llm_endpoint.is_some() {
        anyhow::ensure!(
            (args.stage_run && args.ephemeral_db) || args.stage_run_fork.is_some(),
            "deterministic Investigation LLM endpoint requires an isolated ephemeral stage-run or stage fork"
        );
    }
    if args.stage_run_resume.is_some() {
        return run_resume(args).await;
    }
    if args.stage_run_fork.is_some() {
        return run_fork(args).await;
    }

    // 1) Resolve profile + stage slice up front (cheap, fails fast on bad input).
    let profile_id = args
        .profile
        .clone()
        .unwrap_or_else(|| active_profile_id().to_string());
    let (from_opt, to_stage) = resolve_from_to(&args)?;
    let (entry_stage, allowlist) = resolve_fresh_slice(&profile_id, from_opt, to_stage)?;
    validate_fresh_slice_target_intake(entry_stage, &allowlist, &args.target)?;
    crate::ai::task_operation::validate_current_invocation_exact_targets(&args.target)?;

    let mut slice_sorted: Vec<&str> = allowlist.iter().map(|s| s.as_str()).collect();
    slice_sorted.sort_unstable();
    eprintln!(
        "[stage-run] profile={profile_id} entry={} to={} slice={slice_sorted:?} auto_approve={}",
        entry_stage.as_str(),
        to_stage.as_str(),
        args.auto_approve
    );

    // 2) Settings + tracing (so backend.log captures the run like the GUI does).
    let workspace = args.resolve_workspace().context("resolve workspace")?;
    let settings_manager = Arc::new(
        crate::settings::SettingsManager::new()
            .await
            .context("init settings manager")?,
    );
    settings_manager.ensure_settings_file().await.ok();
    let settings = if let Some(endpoint) = args.stage_run_test_investigation_llm_endpoint.as_ref() {
        anyhow::ensure!(
            entry_stage == StageKind::Investigation
                && to_stage == StageKind::Investigation
                && allowlist.len() == 1,
            "deterministic Investigation LLM endpoint is restricted to --only investigation"
        );
        let settings = deterministic_investigation_settings(endpoint)?;
        settings_manager
            .replace_process_cache(settings.clone())
            .await;
        args.provider = Some("deepseek".to_string());
        args.model = Some(settings.ai.default_model.clone());
        args.api_key = Some("local-scripted-fixture".to_string());
        eprintln!(
            "[stage-run] deterministic Investigation model endpoint installed: {}",
            endpoint
        );
        settings
    } else {
        settings_manager.get().await
    };
    golish_settings::apply_proxy_env(&settings);
    init_tracing_best_effort(&settings, args.verbose);

    // 3) Boot embedded Postgres (lazy pool + ready gate, mirroring the GUI) and
    //    build a headless AppState — AppState::new takes no Tauri AppHandle.
    let mut stage_db = prepare_stage_run_db(&args)?;
    if args.ephemeral_db {
        eprintln!(
            "[stage-run] using ephemeral database: pgdata={} port={} keep={}",
            stage_db.config.pg_data_dir.display(),
            stage_db.config.port,
            args.keep_ephemeral_db
        );
    }
    let preexisting_pg_on_port = local_port_is_open(stage_db.config.port);

    let (db_pool, db_ready) =
        crate::app::bootstrap::create_lazy_db_pool_with_config(&stage_db.config);
    // Own the PG handle (don't leak it like the GUI) so we can stop the server
    // on exit — otherwise each run orphans a postgres holding port 15432 and
    // blocks the next --stage-run.
    let pg_handle_rx = crate::app::bootstrap::spawn_embedded_pg_owned_with_config(
        db_ready.clone(),
        stage_db.config.clone(),
    );
    eprintln!("[stage-run] waiting for embedded Postgres (first run may download pg-embed)...");
    if !wait_for_db(&db_ready).await {
        maybe_keep_ephemeral_db(&mut stage_db, args.keep_ephemeral_db);
        return Err(anyhow!("embedded Postgres did not become ready in time"));
    }
    eprintln!("[stage-run] database ready.");

    if let Err(error) = bootstrap_ephemeral_joint_rollout(&db_pool, &args, &stage_db).await {
        finish_embedded_pg(pg_handle_rx, !preexisting_pg_on_port).await;
        maybe_keep_ephemeral_db(&mut stage_db, args.keep_ephemeral_db);
        return Err(error);
    }

    // Test enablement: optionally seed an intel-provider API key into the vault
    // so a headless run can populate organizations.* via enrich without the GUI.
    maybe_seed_vault_key(&db_pool).await;

    // P1 · seed minimal upstream (org + in-scope targets) so an isolated
    // downstream stage (e.g. --only target_intel) has real data to work on.
    // Scoped to the workspace project_path the agent's manage_targets /
    // manage_organizations tools use; the seeded org id is then bound to the
    // orchestrator so the gate's in_scope_assets(org_id) only sees THIS org's
    // targets (coverage asset-axis isolation, design 2026-06-09).
    let workspace_str = workspace.to_string_lossy().to_string();
    let seed = match maybe_seed(&db_pool, &workspace_str, entry_stage, &args).await {
        Ok(seed) => seed,
        Err(error) => {
            finish_embedded_pg(pg_handle_rx, !preexisting_pg_on_port).await;
            maybe_keep_ephemeral_db(&mut stage_db, args.keep_ephemeral_db);
            return Err(error);
        }
    };
    maybe_seed_open_ports(&db_pool, &workspace_str).await;
    if let Err(error) =
        maybe_seed_controlled_web_origins(&db_pool, &workspace_str, entry_stage, &args).await
    {
        finish_embedded_pg(pg_handle_rx, !preexisting_pg_on_port).await;
        maybe_keep_ephemeral_db(&mut stage_db, args.keep_ephemeral_db);
        return Err(error);
    }

    // Runtime Memory V2 freezes the trusted CLI scope once, before the one
    // operation is created. LegacyV1 retains the historical per-org fallback.
    let runtime_rollout = golish_db::repo::runtime_memory_rollout::get(&db_pool)
        .await
        .map_err(anyhow::Error::new)
        .context("read persisted runtime-memory rollout for stage-run")?;
    let runtime_contract = runtime_v2::persisted_contract(&runtime_rollout.contract)?;
    let cli_runtime_scope = if runtime_v2::contract_writes_v2(runtime_contract) {
        match seed.as_ref().and_then(|seed| seed.org_id) {
            Some(root_organization_id) => {
                let organizations = golish_db::repo::organizations::list(&db_pool, &workspace_str)
                    .await
                    .context("load trusted CLI organization tree")?;
                Some(runtime_v2::build_cli_runtime_scope(
                    &organizations,
                    root_organization_id,
                    args.include_subsidiaries,
                    args.subsidiary_threshold,
                )?)
            }
            None => None, // V2 Scoping pre-freeze is a legal resumable shape.
        }
    } else {
        None
    };
    let has_cli_runtime_scope = cli_runtime_scope.is_some();
    let to_stage_has_specialist = golish_agent_kit::harness::load_embedded_stage_spec(to_stage)
        .ok()
        .and_then(|spec| spec.specialist)
        .is_some_and(|specialist| !specialist.trim().is_empty());

    let app_state = crate::state::AppState::new(
        settings_manager.clone(),
        false,
        None,
        db_pool.clone(),
        db_ready,
    )
    .await;
    if let (Some(toolsconfig_dir), Some(intel_providers_dir), Some(intel_endpoint)) = (
        args.stage_run_test_toolsconfig_dir.as_ref(),
        args.stage_run_test_intel_providers_dir.as_ref(),
        args.stage_run_test_intel_provider_endpoint.as_ref(),
    ) {
        let toolsconfig_dir = toolsconfig_dir.canonicalize().with_context(|| {
            format!(
                "resolve controlled tools-config directory {}",
                toolsconfig_dir.display()
            )
        })?;
        let intel_providers_dir = intel_providers_dir.canonicalize().with_context(|| {
            format!(
                "resolve controlled intel-provider directory {}",
                intel_providers_dir.display()
            )
        })?;
        anyhow::ensure!(
            toolsconfig_dir.is_dir() && intel_providers_dir.is_dir(),
            "controlled provider overrides must both be directories"
        );
        let intel_transport =
            golish_pentest::config::ControlledFixtureIntelTransportAuthority::loopback_http(
                intel_endpoint.clone(),
            )
            .map_err(anyhow::Error::msg)?;
        app_state
            .pentest_config_manager
            .update(|config| {
                config.toolsconfig_dir = toolsconfig_dir;
                config.intel_providers_dir = intel_providers_dir;
                config.controlled_fixture_intel_transport = Some(intel_transport);
            })
            .await;
        eprintln!(
            "[stage-run] controlled fixture provider directories installed (real provider destinations disabled)"
        );
    }
    let agent_state = app_state.extract_agent_state();

    // 4) Build a CliRuntime whose event stream we own (for HITL auto-approve +
    //    report), then build + configure the bridge exactly like a GUI session.
    let (rt_tx, rt_rx) = mpsc::unbounded_channel::<RuntimeEvent>();
    let runtime: Arc<dyn GolishRuntime> =
        Arc::new(CliRuntime::new(rt_tx, args.auto_approve, args.json));

    let session_id = format!("stage-run-{}", uuid::Uuid::new_v4());

    // `session_id` is this run's single identity across ALL session-keyed
    // surfaces: the bridge's event/evidence session (evidence ledger rows,
    // background-job attribution — passed here), the terminal session
    // (`set_session_id`), the orchestrator's chat session
    // (`set_chat_session_id`), and the transcript directory. The harness
    // gate/refiner read the evidence ledger `WHERE session_id = <chat session>`,
    // so the write side MUST book evidence under the same id — passing
    // anything else here makes every booked evidence id invisible to them
    // (ledger facts = 0, submit-only lock unreachable).
    let (mut bridge, mcp_manager) = match crate::cli::initialize_agent(
        &workspace,
        &settings,
        &args,
        runtime,
        app_state.indexer_state.clone(),
        app_state.sidecar_state.clone(),
        &session_id,
        Some(app_state.memory_supervisor.unit_of_work()),
        Some(Arc::new(
            golish_agent_app::ai::db_bridge::knowledge_context::PgKnowledgeContextAdapter::with_query_embedding(
                app_state.db_pool.clone(),
                app_state.memory_supervisor.query_embedding_provider(),
            )
            .context("build stage-run ContextPack adapter")?,
        )),
    )
    .await
    .context("build agent bridge")
    {
        Ok(v) => v,
        Err(e) => {
            // Don't orphan the embedded PG we just started.
            finish_embedded_pg(pg_handle_rx, !preexisting_pg_on_port).await;
            maybe_keep_ephemeral_db(&mut stage_db, args.keep_ephemeral_db);
            return Err(e);
        }
    };

    crate::ai::commands::configure_bridge(&mut bridge, &agent_state, &session_id, None).await;
    if args.stage_run_test_investigation_llm_endpoint.is_some() {
        // Even if a DB prompt-template refresh races with this point and
        // preserves stale per-agent override metadata, execution has no
        // alternate client factory and therefore remains on the loopback main
        // client.
        bridge.clear_model_factory();
    }

    // Persist this run's transcript exactly where the GUI / `--replay` look.
    let transcripts_dir = golish_events::op_trace::resolve_transcript_base(Some(&workspace));
    // Mirror the GUI session init: register the active transcripts base so the
    // per-run tracing layer (telemetry::session_log) co-locates this run's
    // `run.log` next to its `transcript.json` instead of falling back to
    // `~/.golish/transcripts` (the home default when no base is registered).
    golish_events::op_trace::set_active_transcript_base(transcripts_dir.clone());
    match golish_events::TranscriptWriter::new(&transcripts_dir, &session_id).await {
        Ok(writer) => bridge.set_transcript_writer(writer, transcripts_dir.clone()),
        Err(e) => tracing::warn!("stage-run: transcript writer init failed: {e}"),
    }

    bridge.set_session_id(Some(session_id.clone())).await;
    bridge
        .set_execution_mode(golish_agent_kit::execution_mode::ExecutionMode::Task)
        .await;
    bridge.set_harness_profile(Some(profile_id.clone())).await;
    if args.auto_approve {
        bridge.set_agent_mode(AgentMode::AutoApprove).await;
    }

    if let Err(error) = app_state.memory_supervisor.start().await {
        if let Some(manager) = mcp_manager {
            manager.shutdown().await;
        }
        finish_embedded_pg(pg_handle_rx, !preexisting_pg_on_port).await;
        maybe_keep_ephemeral_db(&mut stage_db, args.keep_ephemeral_db);
        return Err(anyhow!(
            "start CLI Memory Supervisor after DB readiness: {}",
            error.code()
        ));
    }

    let bridge = Arc::new(bridge);
    crate::ai::commands::configure_bridge_background_listeners(&bridge, &agent_state).await;
    // Flush + enable live event emission so the coordinator forwards events to
    // our CliRuntime stream (otherwise they buffer waiting for a "frontend").
    bridge.mark_frontend_ready().await;

    // 5) Consume the event stream: auto-resolve scoping HITL and collect events
    //    for the report.
    let collected: Arc<Mutex<Vec<AiEvent>>> = Arc::new(Mutex::new(Vec::new()));
    spawn_event_consumer(
        rt_rx,
        bridge.clone(),
        collected.clone(),
        args.auto_approve,
        StageRunAutoApprovalPolicy {
            trusted_scope_response: trusted_scope_review_response(&args.target),
            retained_scope_authority: None,
            include_subsidiaries: Some(args.include_subsidiaries),
            confirmed_organization_id: seed.as_ref().and_then(|seed| seed.org_id),
            confirmed_organization_name: args.org.clone(),
            approve_phase_boundaries: args.approve_phase_boundaries,
        },
    );

    // 6) Orchestrate the slice (mirrors execute_task_mode, headless).
    let task_input = build_objective(&args, to_stage, seed.as_ref());
    let execution = orchestrate(
        &bridge,
        &db_pool,
        &session_id,
        &profile_id,
        entry_stage,
        allowlist,
        &task_input,
        args.org.as_deref(),
        &args.target,
        seed.as_ref().and_then(|s| s.org_id),
        args.include_subsidiaries,
        args.subsidiary_threshold,
        cli_runtime_scope,
    )
    .await;
    let tracker_session_id = execution.as_ref().ok().map(|outcome| outcome.session_id);
    let mut result = execution.map(|outcome| outcome.response);

    // The deployment rollout read before bootstrap can change concurrently;
    // only the contract frozen on the newly created operation may authorize
    // LegacyV1 child operations. A drifted rollout fails closed instead of
    // dispatching through the wrong adapter.
    let frozen_runtime_contract = if result.is_ok() {
        match tracker_session_id {
            Some(session_id) => {
                match runtime_v2::load_session_operation_contract(&db_pool, session_id).await {
                    Ok(contract) if contract == runtime_contract => contract,
                    Ok(contract) => {
                        result = Err(anyhow!(
                            "runtime-memory rollout changed during CLI bootstrap: preflight={runtime_contract}, operation={contract}"
                        ));
                        contract
                    }
                    Err(error) => {
                        result = Err(error.context("load frozen CLI runtime-memory contract"));
                        runtime_contract
                    }
                }
            }
            None => {
                result = Err(anyhow!(
                    "CLI cannot authorize its runtime adapter without the durable session id"
                ));
                runtime_contract
            }
        }
    } else {
        runtime_contract
    };

    // 6.5) Phase 3 (2026-06-12-redteam-phase3, 方案 A): after the parent run
    // succeeded, run the slice's post-scoping stages once per subsidiary org
    // that Phase 2 landed (organizations.parent_id = parent). Serial on purpose
    // (provider rate limits); each child run is its own task/operation_state
    // bound to the child's org id, so the coverage gate's asset axis + DB-truth
    // projection isolate per org automatically (06-10). A child failure does
    // not stop its siblings — the engagement aggregates at the end (one run
    // exposes ALL gaps). Without --include-subsidiaries this whole step is
    // skipped (zero behaviour change).
    let mut fleet_report: Option<FleetReport> = None;
    if runtime_v2::contract_writes_v2(frozen_runtime_contract)
        && has_cli_runtime_scope
        && to_stage_has_specialist
        && result.is_ok()
    {
        match tracker_session_id {
            Some(session_id) => {
                match runtime_v2::load_cli_report(&db_pool, session_id, to_stage).await {
                    Ok(report) => {
                        tracing::info!(
                            target: "harness::stage_run",
                            operation_id = %report.operation_id,
                            scope_units = report.scope_unit_count,
                            stage_units = report.stage_unit_count,
                            "V2 CLI report aggregated from one relational operation"
                        );
                        fleet_report = Some(report.fleet);
                    }
                    Err(error) => {
                        result = Err(error.context("aggregate V2 CLI relational report"));
                    }
                }
            }
            None => {
                result = Err(anyhow!(
                    "V2 CLI cannot validate its single operation without the durable session id"
                ));
            }
        }
    } else if !runtime_v2::contract_writes_v2(frozen_runtime_contract)
        && args.include_subsidiaries
        && result.is_ok()
    {
        match (
            seed.as_ref().and_then(|s| s.org_id),
            child_slice(&profile_id, to_stage),
        ) {
            (Some(parent_id), Some((child_entry, child_allowlist))) => {
                let parent_name = seed
                    .as_ref()
                    .and_then(|s| s.org_name.clone())
                    .unwrap_or_else(|| "the engagement parent".into());
                let children =
                    match golish_db::repo::organizations::list(&db_pool, &workspace_str).await {
                        Ok(orgs) => filter_child_orgs(orgs, parent_id),
                        Err(e) => {
                            eprintln!(
                                "[stage-run] subsidiary lookup failed (skipping child runs): {e:#}"
                            );
                            Vec::new()
                        }
                    };
                if !children.is_empty() {
                    // 收敛到共享调度内核 `run_fleet_scheduler`（方案 C / fleet Phase B,
                    // 计划 docs/superpowers/plans/2026-06-14-engagement-fleet-scheduler-convergence.md）：
                    // 每个子公司构造成一个 OrgRunTask，per-org 跑一个完整 run_stage（独立
                    // gate + org 隔离），与未来 chat 后端扇出共用同一条调度路径，CLI 因此真
                    // 测生产调度逻辑。串行（concurrency=1）—— 共享同一个 bridge 下并行
                    // run_stage 会互相覆盖 harness 阶段态/会话历史/取消标志，不安全。
                    let tasks: Vec<OrgRunTask> = children
                        .iter()
                        .map(|child| OrgRunTask {
                            org_id: child.id,
                            org_name: child.name.clone(),
                            parent_id: Some(parent_id),
                            entry_stage: child_entry,
                            to_stage,
                            allowlist: child_allowlist.clone(),
                            objective: build_child_objective(child, &parent_name, to_stage),
                        })
                        .collect();
                    eprintln!(
                        "[stage-run] dispatching {} subsidiary run(s) via fleet scheduler (serial)",
                        tasks.len()
                    );
                    let executor = OrgFleetExecutor {
                        bridge: bridge.clone(),
                        db_pool: db_pool.clone(),
                        session_id: session_id.clone(),
                        profile_id: profile_id.clone(),
                        subsidiary_threshold: args.subsidiary_threshold,
                        runtime_memory_contract:
                            golish_agent_kit::runtime_memory::RuntimeMemoryContract::LegacyV1,
                        // CLI 无单卡：不 emit StageRunOrgProgress（事件只进 transcript，无害）。
                        emit_progress: false,
                    };
                    let report = run_legacy_child_operation_fleet(
                        frozen_runtime_contract,
                        FleetConfig {
                            concurrency: 1,
                            mode: FleetMode::Checklist,
                        },
                        tasks,
                        &executor,
                        // T1 行为保持：照跑所有子公司（DB 真值续跑 oracle = T3）。
                        &AlwaysRunOracle,
                        // checklist 模式不评分；NoopScorer 仅满足签名。
                        &NoopScorer,
                        // CLI 无单卡 → 逐子打 eprintln（恢复 T1 前的「── subsidiary i/N ──」
                        // 中途可见性，调度器把 eprintln 副作用外置到此 progress 实现）。
                        &CliFleetProgress {
                            label: "subsidiary",
                        },
                    )
                    .await
                    .expect("LegacyV1 branch must enable child-operation fleet");
                    fleet_report = Some(report);
                }
            }
            (None, _) if args.include_subsidiaries => {
                eprintln!(
                    "[stage-run] --include-subsidiaries given but no parent org was seeded \
                     (--org missing) — skipping subsidiary runs"
                );
            }
            _ => {} // --to scoping: tree-build only, no per-subsidiary stages to run.
        }
    }

    // Give the event consumer a moment to drain trailing events.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 7) Report. Also write the replay artifacts so the timeline is on disk.
    let _ = golish_events::op_trace::write_trace_artifacts(&transcripts_dir, &session_id);
    let events = collected.lock().map(|v| v.clone()).unwrap_or_default();
    let db_smoke_summary = if args.db_smoke_summary {
        Some(
            collect_db_smoke_summary(
                &db_pool,
                &session_id,
                seed.as_ref().and_then(|s| s.org_id),
                &workspace_str,
            )
            .await?,
        )
    } else {
        None
    };
    let report = format_report(
        &events,
        &result,
        &profile_id,
        entry_stage,
        to_stage,
        &session_id,
        &transcripts_dir,
    );
    if args.json {
        for ev in &events {
            if let Ok(s) = serde_json::to_string(ev) {
                println!("{s}");
            }
        }
        if let Some(summary) = &db_smoke_summary {
            if let Ok(s) = serde_json::to_string(&serde_json::json!({
                "type": "db_smoke_summary",
                "summary": summary,
            })) {
                println!("{s}");
            }
        }
    } else {
        println!("{report}");
        if let Some(summary) = &db_smoke_summary {
            println!("{}", format_db_smoke_summary(summary));
        }
        // 子公司 engagement 聚合（无 --include-subsidiaries 时为 None）。
        if let Some(fr) = &fleet_report {
            if !fr.outcomes.is_empty() {
                println!("{}", fr.render());
            }
        }
    }

    wait_for_live_db_diagnostic(&stage_db).await?;

    if let Some(mgr) = mcp_manager {
        mgr.shutdown().await;
    }

    if let Err(error) = app_state.memory_supervisor.shutdown().await {
        tracing::warn!(
            error_code = error.code(),
            "stage-run Memory Supervisor shutdown failed"
        );
    }

    // Stop only the PG this headless run actually started. When the normal app
    // DB is already listening on 15432, pg-embed falls back to "port in use,
    // assume running"; stopping that handle would shut down the user's live DB.
    finish_embedded_pg(pg_handle_rx, !preexisting_pg_on_port).await;
    maybe_keep_ephemeral_db(&mut stage_db, args.keep_ephemeral_db);

    // Phase 3 engagement aggregation: the parent result decides first; any
    // failed subsidiary then flips the whole engagement to FAILED ("an org
    // tree is only covered when EVERY org in it passed", design §2).
    let failed_subs: Vec<String> = fleet_report
        .as_ref()
        .map(|fr| {
            fr.outcomes
                .iter()
                .filter(|o| !o.status.is_covered())
                .map(|o| o.org_name.clone())
                .collect()
        })
        .unwrap_or_default();
    match result {
        Err(e) => Err(e),
        Ok(_) if !failed_subs.is_empty() => Err(anyhow!(
            "subsidiary stage runs failed: [{}]",
            failed_subs.join(", ")
        )),
        Ok(_) => Ok(()),
    }
}

/// Create a new operation from one exact GUI/CLI source operation and execute
/// only the selected post-Scoping slice. Source selection and prefix validation
/// complete before the bridge can dispatch any provider or pentest tool.
async fn run_fork(mut args: Args) -> Result<()> {
    let selector_text = args
        .stage_run_fork
        .clone()
        .ok_or_else(|| anyhow!("--stage-run-fork selector is required"))?;
    let selector = classify_resume_selector(&selector_text);
    let workspace = args
        .resolve_workspace()
        .context("resolve stage fork workspace")?;
    let settings_manager = Arc::new(
        crate::settings::SettingsManager::new()
            .await
            .context("init settings manager")?,
    );
    settings_manager.ensure_settings_file().await.ok();
    let settings = if let Some(endpoint) = args.stage_run_test_investigation_llm_endpoint.as_ref() {
        anyhow::ensure!(
            args.only.as_deref() == Some("investigation")
                && args.from.is_none()
                && args.to.is_none(),
            "deterministic Investigation LLM endpoint is restricted to an Investigation-only stage fork"
        );
        let settings = deterministic_investigation_settings(endpoint)?;
        settings_manager
            .replace_process_cache(settings.clone())
            .await;
        args.provider = Some("deepseek".to_string());
        args.model = Some(settings.ai.default_model.clone());
        args.api_key = Some("local-scripted-fixture".to_string());
        eprintln!(
            "[stage-run-fork] deterministic Investigation model endpoint installed: {}",
            endpoint
        );
        settings
    } else {
        settings_manager.get().await
    };
    golish_settings::apply_proxy_env(&settings);
    init_tracing_best_effort(&settings, args.verbose);

    let mut stage_db = prepare_stage_run_db(&args)?;
    let preexisting_pg_on_port = local_port_is_open(stage_db.config.port);
    let (db_pool, db_ready) =
        crate::app::bootstrap::create_lazy_db_pool_with_config(&stage_db.config);
    let pg_handle_rx = crate::app::bootstrap::spawn_embedded_pg_owned_with_config(
        db_ready.clone(),
        stage_db.config.clone(),
    );
    eprintln!("[stage-run-fork] waiting for shared embedded Postgres...");
    if !wait_for_db(&db_ready).await {
        return Err(anyhow!("embedded Postgres did not become ready in time"));
    }

    let execution_result: Result<()> = async {
        let (_, _, preview_operation) = load_stage_fork_rows(&db_pool, &selector).await?;
        let preview_topology = validate_persisted_stage_topology(
            &preview_operation.stage_topology_contract,
            &preview_operation.stage_topology_canonical_json,
            &preview_operation.stage_topology_sha256,
            &preview_operation.stage_topology_freeze_source,
            &preview_operation.investigation_rollout_mode,
        )
        .context("stage fork preview has an invalid frozen topology")?;
        let resolved = resolve_stage_run_fork_slice(
            &preview_operation.profile,
            preview_topology,
            &args,
        )?;
        let source = validate_stage_fork_source(&db_pool, &selector, &workspace, &resolved).await?;
        anyhow::ensure!(
            source.profile == preview_operation.profile,
            "stage fork source changed during preflight"
        );
        anyhow::ensure!(
            source.stage_topology_contract == resolved.stage_topology_contract,
            "stage fork source topology changed during preflight"
        );
        if args.provider.is_none() {
            args.provider = source.provider.clone();
        }
        if args.model.is_none() {
            args.model = source.model.clone();
        }
        eprintln!(
            "[stage-run-fork] source={} scope={} project={} profile={} topology={} entry={} to={} adopted={:?}",
            source.operation_id,
            source.source_scope_snapshot_id,
            source.project_scope_id,
            source.profile,
            source.stage_topology_contract.topology,
            resolved.entry_stage.as_str(),
            resolved.terminal_stage.as_str(),
            resolved
                .adopted_stage_kinds
                .iter()
                .map(|stage| stage.as_str())
                .collect::<Vec<_>>()
        );

        let app_state = crate::state::AppState::new(
            settings_manager.clone(),
            false,
            None,
            db_pool.clone(),
            db_ready,
        )
        .await;
        let agent_state = app_state.extract_agent_state();
        let (rt_tx, rt_rx) = mpsc::unbounded_channel::<RuntimeEvent>();
        let runtime: Arc<dyn GolishRuntime> =
            Arc::new(CliRuntime::new(rt_tx, args.auto_approve, args.json));
        let session_id = format!("stage-run-{}", uuid::Uuid::new_v4());
        let (mut bridge, mcp_manager) = crate::cli::initialize_agent(
            &workspace,
            &settings,
            &args,
            runtime,
            app_state.indexer_state.clone(),
            app_state.sidecar_state.clone(),
            &session_id,
            Some(app_state.memory_supervisor.unit_of_work()),
            Some(Arc::new(
                golish_agent_app::ai::db_bridge::knowledge_context::PgKnowledgeContextAdapter::with_query_embedding(
                    app_state.db_pool.clone(),
                    app_state.memory_supervisor.query_embedding_provider(),
                )
                .context("build stage-fork ContextPack adapter")?,
            )),
        )
        .await
        .context("build stage fork agent bridge")?;
        crate::ai::commands::configure_bridge(&mut bridge, &agent_state, &session_id, None).await;
        if args.stage_run_test_investigation_llm_endpoint.is_some() {
            bridge.clear_model_factory();
        }

        let transcripts_dir = golish_events::op_trace::resolve_transcript_base(Some(&workspace));
        golish_events::op_trace::set_active_transcript_base(transcripts_dir.clone());
        match golish_events::TranscriptWriter::new(&transcripts_dir, &session_id).await {
            Ok(writer) => bridge.set_transcript_writer(writer, transcripts_dir.clone()),
            Err(error) => tracing::warn!("stage-run-fork transcript init failed: {error}"),
        }
        bridge.set_session_id(Some(session_id.clone())).await;
        bridge
            .set_execution_mode(golish_agent_kit::execution_mode::ExecutionMode::Task)
            .await;
        bridge
            .set_harness_profile(Some(source.profile.clone()))
            .await;
        if args.auto_approve {
            bridge.set_agent_mode(AgentMode::AutoApprove).await;
        }
        if let Err(error) = app_state.memory_supervisor.start().await {
            if let Some(manager) = mcp_manager {
                manager.shutdown().await;
            }
            return Err(anyhow!(
                "start stage fork Memory Supervisor: {}",
                error.code()
            ));
        }

        let bridge = Arc::new(bridge);
        crate::ai::commands::configure_bridge_background_listeners(&bridge, &agent_state).await;
        bridge.mark_frontend_ready().await;
        let collected: Arc<Mutex<Vec<AiEvent>>> = Arc::new(Mutex::new(Vec::new()));
        spawn_event_consumer(
            rt_rx,
            bridge.clone(),
            collected.clone(),
            args.auto_approve,
            StageRunAutoApprovalPolicy {
                trusted_scope_response: None,
                retained_scope_authority: None,
                include_subsidiaries: None,
                confirmed_organization_id: Some(source.root_organization_id),
                confirmed_organization_name: None,
                approve_phase_boundaries: args.approve_phase_boundaries,
            },
        );

        let objective = args.execute.clone().unwrap_or_else(|| {
            format!(
                "基于 operation {} 的完整前置数据，只重新测试 {} 到 {}",
                source.operation_id,
                resolved.entry_stage.as_str(),
                resolved.terminal_stage.as_str()
            )
        });
        let result = orchestrate_stage_fork(
            &bridge,
            &db_pool,
            &session_id,
            &source,
            &resolved,
            &objective,
        )
        .await
        .map(|outcome| outcome.response);
        tokio::time::sleep(Duration::from_millis(300)).await;
        let _ = golish_events::op_trace::write_trace_artifacts(&transcripts_dir, &session_id);
        let events = collected
            .lock()
            .map(|events| events.clone())
            .unwrap_or_default();
        let report = format_report(
            &events,
            &result,
            &source.profile,
            resolved.entry_stage,
            resolved.terminal_stage,
            &session_id,
            &transcripts_dir,
        );
        if args.json {
            for event in &events {
                if let Ok(line) = serde_json::to_string(event) {
                    println!("{line}");
                }
            }
        } else {
            println!("{report}");
        }
        if let Some(manager) = mcp_manager {
            manager.shutdown().await;
        }
        let shutdown = app_state.memory_supervisor.shutdown().await;
        match (result, shutdown) {
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(anyhow!(
                "shutdown stage fork Memory Supervisor: {}",
                error.code()
            )),
            (Ok(_), Ok(())) => Ok(()),
        }
    }
    .await;

    finish_embedded_pg(pg_handle_rx, !preexisting_pg_on_port).await;
    maybe_keep_ephemeral_db(&mut stage_db, args.keep_ephemeral_db);
    execution_result
}

/// Resume one exact interrupted headless stage-run without allocating a new
/// chat session, task, operation, or freshness epoch.
async fn run_resume(mut args: Args) -> Result<()> {
    let selector_text = args
        .stage_run_resume
        .clone()
        .ok_or_else(|| anyhow!("--stage-run-resume selector is required"))?;
    let selector = classify_resume_selector(&selector_text);
    let expectations = ResumeExpectations::from_args(&args)?;

    let workspace = args
        .resolve_workspace()
        .context("resolve resume workspace")?;
    let settings_manager = Arc::new(
        crate::settings::SettingsManager::new()
            .await
            .context("init settings manager")?,
    );
    settings_manager.ensure_settings_file().await.ok();
    let settings = settings_manager.get().await;
    golish_settings::apply_proxy_env(&settings);
    init_tracing_best_effort(&settings, args.verbose);

    let mut stage_db = prepare_stage_run_db(&args)?;
    let preexisting_pg_on_port = local_port_is_open(stage_db.config.port);
    let (db_pool, db_ready) =
        crate::app::bootstrap::create_lazy_db_pool_with_config(&stage_db.config);
    let pg_handle_rx = crate::app::bootstrap::spawn_embedded_pg_owned_with_config(
        db_ready.clone(),
        stage_db.config.clone(),
    );
    eprintln!("[stage-run-resume] waiting for embedded Postgres...");
    if !wait_for_db(&db_ready).await {
        maybe_keep_ephemeral_db(&mut stage_db, args.keep_ephemeral_db);
        return Err(anyhow!("embedded Postgres did not become ready in time"));
    }

    if args.stage_run_test_restart_exhausted_stage {
        let restart_result: Result<()> = async {
            anyhow::ensure!(
                args.stage_run_test_database
                    .as_deref()
                    .is_some_and(|database| database.starts_with("golish_gatefix_"))
                    || args.stage_run_resume_pgdata.is_some(),
                "--stage-run-test-restart-exhausted-stage requires either an explicit golish_gatefix_* database or --stage-run-resume-pgdata"
            );
            restart_exhausted_test_stage_runtime(&db_pool, &selector, &expectations, &workspace)
                .await
        }
        .await;
        if let Err(error) = restart_result {
            finish_embedded_pg(pg_handle_rx, !preexisting_pg_on_port).await;
            maybe_keep_ephemeral_db(&mut stage_db, args.keep_ephemeral_db);
            return Err(error);
        }
    }

    let execution_result: Result<()> = async {
        if let Some(terminal) =
            resolve_terminal_stage_run_resume_replay(&db_pool, &selector, &expectations).await?
        {
            let (requested_terminal, _) = resolve_resume_slice(
                &terminal.profile,
                terminal.stage_topology_contract.topology,
                terminal.stage,
                args.resume_to.as_deref(),
            )?;
            anyhow::ensure!(
                requested_terminal == terminal.stage,
                "resume refused: a finished operation can only replay its terminal stage result"
            );
            let transcripts_dir =
                golish_events::op_trace::resolve_transcript_base(Some(&workspace));
            let transcript_path =
                golish_events::transcript_path(&transcripts_dir, &terminal.chat_session_key);
            anyhow::ensure!(
                transcript_path.is_file(),
                "resume refused: terminal session transcript is not in this workspace ({})",
                transcript_path.display()
            );
            eprintln!(
                "[stage-run-resume] terminal replay session={} db_session={} operation={} org={} stage={}",
                terminal.chat_session_key,
                terminal.session_id,
                terminal.operation_id,
                terminal.organization_id,
                terminal.stage.as_str(),
            );
            let workspace_str = workspace.to_string_lossy().to_string();
            let db_smoke_summary = if args.db_smoke_summary {
                Some(
                    collect_db_smoke_summary(
                        &db_pool,
                        &terminal.chat_session_key,
                        Some(terminal.organization_id),
                        &workspace_str,
                    )
                    .await?,
                )
            } else {
                None
            };
            if args.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "type": "stage_run_terminal_replay",
                        "sessionId": terminal.chat_session_key,
                        "operationId": terminal.operation_id,
                        "stage": terminal.stage.as_str(),
                        "result": terminal.result,
                    })
                );
                if let Some(summary) = &db_smoke_summary {
                    println!(
                        "{}",
                        serde_json::json!({
                            "type": "db_smoke_summary",
                            "summary": summary,
                        })
                    );
                }
            } else {
                println!(
                    "\n══════════ stage-run terminal replay ══════════\noperation = {}\nstage     = {}\n\n{}\n\nNo stage, worker, model, or report revision was re-executed.\n════════════════════════════════════════════════\n",
                    terminal.operation_id,
                    terminal.stage.as_str(),
                    terminal.result,
                );
                if let Some(summary) = &db_smoke_summary {
                    println!("{}", format_db_smoke_summary(summary));
                }
            }
            wait_for_live_db_diagnostic(&stage_db).await?;
            return Ok(());
        }

        let initial = resolve_stage_run_resume_target(&db_pool, &selector, &expectations).await?;
        let (resume_terminal, resume_allowlist) = resolve_resume_slice(
            &initial.profile,
            initial.stage_topology_contract.topology,
            initial.stage,
            args.resume_to.as_deref(),
        )?;
        let transcripts_dir = golish_events::op_trace::resolve_transcript_base(Some(&workspace));
        let transcript_path =
            golish_events::transcript_path(&transcripts_dir, &initial.chat_session_key);
        anyhow::ensure!(
            transcript_path.is_file(),
            "resume refused: selected session transcript is not in this workspace ({})",
            transcript_path.display()
        );

        let provider = match (args.provider.as_deref(), initial.provider.as_deref()) {
            (Some(requested), Some(stored)) => {
                anyhow::ensure!(
                    requested == stored,
                    "resume refused: provider override {requested:?} differs from stored {stored:?}"
                );
                stored.to_string()
            }
            (Some(_), None) => anyhow::bail!(
                "resume refused: original session has no stored provider to verify override"
            ),
            (None, Some(stored)) => stored.to_string(),
            (None, None) => {
                anyhow::bail!("resume refused: original session has no stored provider identity")
            }
        };
        args.provider = Some(provider);
        let model = match (args.model.as_deref(), initial.model.as_deref()) {
            (Some(requested), Some(stored)) => {
                anyhow::ensure!(
                    requested == stored,
                    "resume refused: model override {requested:?} differs from stored {stored:?}"
                );
                stored.to_string()
            }
            (Some(_), None) => anyhow::bail!(
                "resume refused: original session has no stored model to verify override"
            ),
            (None, Some(stored)) => stored.to_string(),
            (None, None) => {
                anyhow::bail!("resume refused: original session has no stored model identity")
            }
        };
        args.model = Some(model);

        let mut claim = StageRunResumeClaim::acquire(&db_pool, initial.operation_id).await?;
        if initial.needs_task_repair {
            repair_reaped_task(&mut claim, &initial).await?;
        }
        if expectations.repair_reaped_task {
            ensure_reaped_task_open_turn(&mut claim, &initial).await?;
        }
        if initial.needs_graph_repair {
            repair_missing_graph_flow(&mut claim, &initial).await?;
        }
        let target = resolve_stage_run_resume_target(&db_pool, &selector, &expectations).await?;
        anyhow::ensure!(
            target.operation_id == initial.operation_id,
            "resume refused: selected operation changed while acquiring the claim"
        );
        anyhow::ensure!(
            target.session_id == initial.session_id
                && target.chat_session_key == initial.chat_session_key
                && target.organization_id == initial.organization_id
                && target.profile == initial.profile
                && target.stage == initial.stage
                && target.runtime_memory_contract == initial.runtime_memory_contract
                && target.stage_topology_contract == initial.stage_topology_contract
                && target.authority == initial.authority
                && target.relational_stage_execution_id
                    == initial.relational_stage_execution_id
                && target.provider == initial.provider
                && target.model == initial.model,
            "resume refused: persisted session/operation identity changed while acquiring the claim"
        );
        anyhow::ensure!(
            !target.needs_graph_repair,
            "resume refused: graph_flow checkpoint is still missing after repair"
        );
        anyhow::ensure!(
            !target.needs_task_repair,
            "resume refused: task is still marked as startup-reaped after repair"
        );
        anyhow::ensure!(
            transcript_path.is_file(),
            "resume refused: selected session transcript disappeared while acquiring the claim ({})",
            transcript_path.display()
        );

        let retained_scope_authority =
            if let Some(packet_path) = args.stage_run_active_recon_scope_authority.as_deref() {
                anyhow::ensure!(
                    args.stage_run_resume_pgdata.is_some(),
                    "active-recon scope authority packets are accepted only for an exact retained-DB resume"
                );
                anyhow::ensure!(
                    target.stage == StageKind::TargetIntel,
                    "active-recon scope authority packets are accepted only while resuming Target Intel"
                );
                Some(scope_authority::read_retained_scope_authority(
                    packet_path,
                    target.operation_id,
                    target.organization_id,
                )?)
            } else {
                None
            };

        eprintln!(
            "[stage-run-resume] session={} db_session={} operation={} org={} profile={} topology={} stage={} to={}",
            target.chat_session_key,
            target.session_id,
            target.operation_id,
            target.organization_id,
            target.profile,
            target.stage_topology_contract.topology,
            target.stage.as_str(),
            resume_terminal.as_str(),
        );

        let app_state = crate::state::AppState::new(
            settings_manager.clone(),
            false,
            None,
            db_pool.clone(),
            db_ready.clone(),
        )
        .await;
        let agent_state = app_state.extract_agent_state();
        let (rt_tx, rt_rx) = mpsc::unbounded_channel::<RuntimeEvent>();
        let runtime: Arc<dyn GolishRuntime> =
            Arc::new(CliRuntime::new(rt_tx, args.auto_approve, args.json));
        let (mut bridge, mcp_manager) = crate::cli::initialize_agent(
            &workspace,
            &settings,
            &args,
            runtime,
            app_state.indexer_state.clone(),
            app_state.sidecar_state.clone(),
            &target.chat_session_key,
            Some(app_state.memory_supervisor.unit_of_work()),
            Some(Arc::new(
                golish_agent_app::ai::db_bridge::knowledge_context::PgKnowledgeContextAdapter::with_query_embedding(
                    app_state.db_pool.clone(),
                    app_state.memory_supervisor.query_embedding_provider(),
                )
                .context("build exact-resume ContextPack adapter")?,
            )),
        )
        .await
        .context("build exact-resume agent bridge")?;
        crate::ai::commands::configure_bridge(
            &mut bridge,
            &agent_state,
            &target.chat_session_key,
            None,
        )
        .await;

        golish_events::op_trace::set_active_transcript_base(transcripts_dir.clone());
        match golish_events::TranscriptWriter::new(&transcripts_dir, &target.chat_session_key).await
        {
            Ok(writer) => bridge.set_transcript_writer(writer, transcripts_dir.clone()),
            Err(error) => anyhow::bail!("resume transcript writer init failed: {error}"),
        }
        bridge
            .set_session_id(Some(target.chat_session_key.clone()))
            .await;
        bridge
            .set_execution_mode(golish_agent_kit::execution_mode::ExecutionMode::Task)
            .await;
        bridge
            .set_harness_profile(Some(target.profile.clone()))
            .await;
        bridge.set_tracker_session_uuid(target.session_id);
        if args.auto_approve {
            bridge.set_agent_mode(AgentMode::AutoApprove).await;
        }

        app_state
            .memory_supervisor
            .start()
            .await
            .map_err(|error| anyhow!(
                "start resume Memory Supervisor after DB readiness: {}",
                error.code()
            ))?;

        let bridge = Arc::new(bridge);
        crate::ai::commands::configure_bridge_background_listeners(&bridge, &agent_state).await;
        bridge.mark_frontend_ready().await;
        let collected: Arc<Mutex<Vec<AiEvent>>> = Arc::new(Mutex::new(Vec::new()));
        spawn_event_consumer(
            rt_rx,
            bridge.clone(),
            collected.clone(),
            args.auto_approve,
            StageRunAutoApprovalPolicy {
                trusted_scope_response: None,
                retained_scope_authority,
                // Exact resume must use frozen operation truth. The resume CLI
                // has no authority to invent a new subsidiary decision.
                include_subsidiaries: None,
                confirmed_organization_id: None,
                confirmed_organization_name: None,
                approve_phase_boundaries: args.approve_phase_boundaries,
            },
        );

        let continuation = args.execute.as_deref().unwrap_or("继续");
        let result = orchestrate_resume(
            &bridge,
            &db_pool,
            &mut claim,
            &target,
            &resume_allowlist,
            continuation,
        )
        .await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let _ = golish_events::op_trace::write_trace_artifacts(
            &transcripts_dir,
            &target.chat_session_key,
        );
        let events = collected
            .lock()
            .map(|events| events.clone())
            .unwrap_or_default();
        let report = format_report(
            &events,
            &result,
            &target.profile,
            target.stage,
            resume_terminal,
            &target.chat_session_key,
            &transcripts_dir,
        );
        let workspace_str = workspace.to_string_lossy().to_string();
        let db_smoke_summary = if args.db_smoke_summary {
            Some(
                collect_db_smoke_summary(
                    &db_pool,
                    &target.chat_session_key,
                    Some(target.organization_id),
                    &workspace_str,
                )
                .await?,
            )
        } else {
            None
        };
        if args.json {
            for event in &events {
                if let Ok(line) = serde_json::to_string(event) {
                    println!("{line}");
                }
            }
            if let Some(summary) = &db_smoke_summary {
                println!(
                    "{}",
                    serde_json::json!({
                        "type": "db_smoke_summary",
                        "summary": summary,
                    })
                );
            }
        } else {
            println!("{report}");
            if let Some(summary) = &db_smoke_summary {
                println!("{}", format_db_smoke_summary(summary));
            }
        }

        wait_for_live_db_diagnostic(&stage_db).await?;

        if let Some(manager) = mcp_manager {
            manager.shutdown().await;
        }
        let supervisor_shutdown = app_state.memory_supervisor.shutdown().await;
        let resume_result = result.map(|_| ());
        let release_result = claim.release().await;
        match (resume_result, release_result, supervisor_shutdown) {
            (Err(error), _, _) => Err(error),
            (Ok(()), Err(error), _) => Err(error),
            (Ok(()), Ok(()), Err(error)) => Err(anyhow!(
                "shutdown resume Memory Supervisor: {}",
                error.code()
            )),
            (Ok(()), Ok(()), Ok(())) => Ok(()),
        }
    }
    .await;

    finish_embedded_pg(pg_handle_rx, !preexisting_pg_on_port).await;
    maybe_keep_ephemeral_db(&mut stage_db, args.keep_ephemeral_db);
    execution_result
}

/// Best-effort: stop the embedded PostgreSQL server started for this run.
///
/// The handle arrives via the oneshot from
/// [`spawn_embedded_pg_owned_with_config`](crate::app::bootstrap::spawn_embedded_pg_owned_with_config).
/// If startup failed (sender dropped) there is nothing to stop.
async fn finish_embedded_pg(rx: tokio::sync::oneshot::Receiver<golish_db::GolishDb>, stop: bool) {
    match rx.await {
        Ok(mut db) => {
            if stop {
                db.stop().await;
                eprintln!("[stage-run] embedded Postgres stopped.");
            } else {
                db.pool().close().await;
                std::mem::forget(db);
                eprintln!("[stage-run] left pre-existing PostgreSQL running.");
            }
        }
        Err(_) => {
            tracing::debug!("stage-run: no embedded PG handle to stop (startup failed)");
        }
    }
}

/// Run a CLI slice through the same prepared task-operation kernel used by GUI
/// Task/Profile. `org_id` (from the upstream seed) binds the coverage gate's
/// asset axis to THIS run's organization (coverage asset-axis isolation,
/// design 2026-06-09).
/// `include_subsidiaries` + `subsidiary_threshold` (Phase 2,
/// 2026-06-12-redteam-phase2) opt the run into the scoping subsidiary gate.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn orchestrate(
    bridge: &Arc<AgentBridge>,
    db_pool: &Arc<sqlx::PgPool>,
    session_id: &str,
    profile_id: &str,
    entry_stage: StageKind,
    allowlist: HashSet<StageKind>,
    task_input: &str,
    subject_label: Option<&str>,
    current_invocation_targets: &[String],
    org_id: Option<uuid::Uuid>,
    include_subsidiaries: bool,
    subsidiary_threshold: u8,
    cli_runtime_scope: Option<golish_agent_kit::db_traits::CliRuntimeScope>,
) -> Result<crate::ai::task_operation::FreshTaskOperationOutcome> {
    use crate::ai::task_operation::{
        prepare_task_operation, FreshOperationEntry, FreshTaskOperationLaunch,
    };

    let subsidiary_policy = SubsidiaryScopePolicy {
        include_subsidiaries,
        ownership_threshold_percent: subsidiary_threshold,
    };
    let cli_runtime_scope = cli_runtime_scope_for_entry(entry_stage, cli_runtime_scope);
    let scope = build_fresh_cli_scope(
        entry_stage,
        task_input,
        subject_label,
        current_invocation_targets,
        org_id,
        cli_runtime_scope,
        &subsidiary_policy,
    )?;
    let launch = FreshTaskOperationLaunch::new(
        task_input,
        profile_id,
        FreshOperationEntry::StageSlice {
            entry_stage,
            allowlist,
        },
        scope,
        subsidiary_policy,
        None,
    )?;

    let request = bridge
        .begin_top_level_request()
        .await
        .context("start stage-run request for this agent session")?;
    let prepared = prepare_task_operation(
        bridge.clone(),
        db_pool.clone(),
        session_id,
        task_input,
        request,
    )
    .await?;
    prepared.run_fresh(launch).await
}

async fn orchestrate_stage_fork(
    bridge: &Arc<AgentBridge>,
    db_pool: &Arc<sqlx::PgPool>,
    session_id: &str,
    source: &ValidatedStageForkSource,
    resolved: &ResolvedForkSlice,
    objective: &str,
) -> Result<crate::ai::task_operation::FreshTaskOperationOutcome> {
    use crate::ai::task_operation::{
        prepare_task_operation, FreshOperationScope, StageForkTaskOperationLaunch,
        SubsidiaryScopePolicy,
    };

    let subsidiary_policy = SubsidiaryScopePolicy {
        include_subsidiaries: source.runtime_scope.include_subsidiaries,
        ownership_threshold_percent: source.runtime_scope.subsidiary_threshold,
    };
    let scope = FreshOperationScope::confirmed_organization_intake(
        source.root_organization_name.clone(),
        source.root_organization_id,
        Some(source.runtime_scope.clone()),
        &subsidiary_policy,
    )?;
    let launch = StageForkTaskOperationLaunch::new(
        objective,
        &source.profile,
        source.operation_id,
        source.source_scope_snapshot_id,
        resolved.entry_stage,
        resolved.terminal_stage,
        resolved.allowlist.clone(),
        resolved.adopted_stage_kinds.clone(),
        scope,
        subsidiary_policy,
    )?;
    let request = bridge
        .begin_top_level_request()
        .await
        .context("start stage fork request")?;
    let prepared = prepare_task_operation(
        bridge.clone(),
        db_pool.clone(),
        session_id,
        objective,
        request,
    )
    .await?;
    prepared.run_stage_fork(launch).await
}

/// Scoping must derive and freeze its organization scope from the persisted
/// typed human decision and the trusted deliverable submission. Pre-freezing
/// the CLI scope during operation creation would make finalization enter its
/// replay branch before the Scoping root unit/submission identities exist.
/// Direct post-Scoping entries already carry an explicit CLI scope decision and
/// may freeze it atomically with operation creation.
fn cli_runtime_scope_for_entry(
    entry_stage: StageKind,
    runtime_scope: Option<golish_agent_kit::db_traits::CliRuntimeScope>,
) -> Option<golish_agent_kit::db_traits::CliRuntimeScope> {
    (entry_stage != StageKind::Scoping)
        .then_some(runtime_scope)
        .flatten()
}

/// Convert explicit fresh CLI intake into the shared typed scope contract.
///
/// `--org` is a deliberate headless Scoping shortcut: the exact label is
/// get-or-created by the trusted seed path and becomes confirmed organization
/// authority immediately, so Scoping does not need to reconcile the company
/// identity with GUI prompt state. It still carries no target authority. Only
/// current-invocation `--target` values can populate `ConfirmedTargetIntake`,
/// and the shared pre-EAS barrier remains responsible for holding an empty
/// trusted target snapshot.
#[allow(clippy::too_many_arguments)]
fn build_fresh_cli_scope(
    _entry_stage: StageKind,
    task_input: &str,
    subject_label: Option<&str>,
    current_invocation_targets: &[String],
    org_id: Option<uuid::Uuid>,
    cli_runtime_scope: Option<golish_agent_kit::db_traits::CliRuntimeScope>,
    subsidiary_policy: &SubsidiaryScopePolicy,
) -> Result<FreshOperationScope> {
    if !current_invocation_targets.is_empty() {
        return FreshOperationScope::confirmed_target_intake(
            subject_label.map(str::to_owned),
            current_invocation_targets.to_vec(),
            org_id,
            cli_runtime_scope,
            subsidiary_policy,
        );
    }
    if let (Some(subject_label), Some(organization_id)) = (subject_label, org_id) {
        return FreshOperationScope::confirmed_organization_intake(
            subject_label,
            organization_id,
            cli_runtime_scope,
            subsidiary_policy,
        );
    }
    anyhow::ensure!(
        org_id.is_none() && cli_runtime_scope.is_none(),
        "fresh CLI organization authority requires an explicit --org label"
    );
    FreshOperationScope::unconfirmed_subject(subject_label.unwrap_or(task_input))
}

/// Re-drive the exact persisted operation selected by `--stage-run-resume`.
/// Unlike [`orchestrate`], this path never inserts a task/operation and never
/// calls `run_stage`; the graph resumes from the checkpoint attached to
/// `target.operation_id`.
async fn orchestrate_resume(
    bridge: &Arc<AgentBridge>,
    db_pool: &Arc<sqlx::PgPool>,
    _claim: &mut StageRunResumeClaim,
    target: &ValidatedResumeTarget,
    stage_allowlist: &HashSet<StageKind>,
    continuation: &str,
) -> Result<String> {
    use crate::ai::task_operation::{prepare_task_operation, TaskOperationConfig};

    let request = bridge
        .begin_top_level_request()
        .await
        .context("start exact stage-run resume request")?;
    let prepared = prepare_task_operation(
        bridge.clone(),
        db_pool.clone(),
        &target.chat_session_key,
        continuation,
        request,
    )
    .await?;
    let prepared_session_id = prepared.session_id();
    if prepared_session_id != target.session_id {
        return prepared
            .finish(Err(anyhow!(
                "exact-resume chat key resolved to session {}, expected {}",
                prepared_session_id,
                target.session_id
            )))
            .await;
    }

    let result = async {
        let current_project_scope = prepared.register_project_scope().await?;
        let operation = prepared
            .db_repo()
            .operation_state_get(target.operation_id)
            .await
            .context("load exact-resume operation project scope")?
            .ok_or_else(|| anyhow!("resume operation_state is missing"))?;
        anyhow::ensure!(
            operation.stage_topology_contract == target.stage_topology_contract,
            "resume refused: operation-frozen stage topology changed after validation"
        );
        golish_agent_kit::runtime_memory::authorize_operation_project_scope(
            operation.project_scope_id,
            operation.runtime_memory_contract,
            current_project_scope.project_scope_id,
        )
        .map_err(anyhow::Error::new)
        .context("authorize exact-resume project scope")?;
        let stage_fork_target_authority =
            immutable_stage_fork_target_authority(db_pool.as_ref(), target).await?;

        let mut orchestrator = prepared.build_orchestrator(TaskOperationConfig {
            profile_override: Some(target.profile.clone()),
            stage_allowlist: Some(stage_allowlist.clone()),
            harness_org_id: Some(target.organization_id),
            current_invocation_target_authority: persisted_resume_target_authority(
                &target.state_blob,
                stage_fork_target_authority,
            )?,
            ..TaskOperationConfig::default()
        });
        orchestrator.set_force_stage_run_on_resume_once(true);
        let expected_resume_source =
            selected_resume_record_source(target.authority, target.runtime_memory_contract)?;
        let selected_resume = select_exact_resume_runtime_source(
            db_pool.as_ref(),
            target.operation_id,
            target.session_id,
        )
        .await?;
        anyhow::ensure!(
            selected_resume.source == expected_resume_source,
            "resume refused: shared runtime source selection disagrees with the validated CLI authority"
        );
        let resume_source = selected_resume.source;
        orchestrator.set_resume_runtime_memory_source(resume_source);
        bridge.set_resume_runtime_memory_source(resume_source).await;

        // The GUI and CLI share the same source + open-Turn witness. Closing
        // that Turn and appending its successor is the durable claim; the CLI
        // advisory lock remains held as an additional cross-process guard.
        claim_exact_resume_runtime_source(
            db_pool.as_ref(),
            target.operation_id,
            target.session_id,
            selected_resume,
            continuation,
        )
        .await?;
        orchestrator.set_resume_task_preclaimed(true);
        orchestrator
            .resume(target.operation_id, continuation, prepared.executor())
            .await
    }
    .await;
    prepared.finish(result).await
}

async fn immutable_stage_fork_target_authority(
    pool: &sqlx::PgPool,
    target: &ValidatedResumeTarget,
) -> Result<bool> {
    let Some(fork) = golish_db::repo::operation_stage_forks::get(pool, target.operation_id).await?
    else {
        return Ok(false);
    };
    anyhow::ensure!(
        fork.operation_id == target.operation_id
            && fork.target_profile == target.profile
            && fork.target_runtime_memory_contract == target.runtime_memory_contract.as_str()
            && fork.expected_target_count > 0,
        "resume refused: immutable stage-fork target authority does not match the selected operation"
    );
    let frozen_targets =
        golish_db::repo::operation_stage_forks::list_targets(pool, target.operation_id).await?;
    anyhow::ensure!(
        immutable_stage_fork_target_manifest_is_complete(&fork, &frozen_targets),
        "resume refused: immutable stage-fork target manifest is incomplete or inconsistent"
    );
    Ok(true)
}

fn immutable_stage_fork_target_manifest_is_complete(
    fork: &golish_db::repo::operation_stage_forks::OperationStageForkRow,
    frozen_targets: &[golish_db::repo::operation_stage_forks::OperationStageForkTargetRow],
) -> bool {
    let Some(manifest_targets) = fork
        .manifest
        .get("targets")
        .and_then(serde_json::Value::as_array)
    else {
        return false;
    };
    if fork.expected_target_count <= 0
        || frozen_targets.len() != fork.expected_target_count as usize
        || manifest_targets.len() != frozen_targets.len()
    {
        return false;
    }

    let mut saw_in_scope = false;
    let mut ordinals = std::collections::HashSet::new();
    frozen_targets.iter().all(|row| {
        let scope = row.target_scope_at_fork.trim().to_ascii_lowercase();
        saw_in_scope |= scope == "in";
        row.operation_id == fork.operation_id
            && row.scope_snapshot_id == fork.target_scope_snapshot_id
            && matches!(scope.as_str(), "in" | "out")
            && !row.canonical_identity_sha256.trim().is_empty()
            && row.ordinal >= 0
            && ordinals.insert(row.ordinal)
            && manifest_targets.iter().any(|member| {
                member.get("id").and_then(serde_json::Value::as_str)
                    == Some(row.id.to_string().as_str())
                    && member.get("ordinal").and_then(serde_json::Value::as_i64)
                        == Some(i64::from(row.ordinal))
                    && member
                        .get("live_target_id")
                        .and_then(serde_json::Value::as_str)
                        == Some(row.live_target_id.to_string().as_str())
                    && member
                        .get("organization_id")
                        .and_then(serde_json::Value::as_str)
                        == Some(row.organization_id.to_string().as_str())
                    && member
                        .get("canonical_identity_sha256")
                        .and_then(serde_json::Value::as_str)
                        == Some(row.canonical_identity_sha256.as_str())
            })
    }) && saw_in_scope
}

fn persisted_resume_target_authority(
    state_blob: &serde_json::Value,
    immutable_stage_fork_target_authority: bool,
) -> Result<Option<bool>> {
    golish_agent_kit::task_orchestrator::harness_resume::current_invocation_target_authority_from_state_blob(
        state_blob,
    )
    .context("restore persisted fresh target authority for exact resume")
    // Exact resume is a headless stage-run surface, not the GUI interactive
    // lifecycle. A missing marker is therefore not permission to consult an
    // organization's historical targets. An immutable stage fork is the only
    // marker-free exception because its exact non-empty target manifest was
    // validated and frozen in the operation-creation transaction.
    .map(|authority| Some(authority.unwrap_or(immutable_stage_fork_target_authority)))
}

/// Watch the runtime event stream: resolve only typed, policy-backed
/// `ask_human` requests when `--auto-approve`, and collect events for the
/// post-run report. Unsupported/security-sensitive prompts are declined rather
/// than receiving a fabricated generic approval string.
fn trusted_scope_review_response(targets: &[String]) -> Option<String> {
    let rows = targets
        .iter()
        .map(|target| target.trim())
        .filter(|target| !target.is_empty())
        .map(|target| {
            let target_type = golish_app_core::domain::targets::detect_type(target);
            serde_json::json!({
                "value": target,
                "type": target_type.as_str(),
                "scope": "in",
            })
        })
        .collect::<Vec<_>>();
    (!rows.is_empty()).then(|| serde_json::Value::Array(rows).to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StageRunAutoApprovalPolicy {
    trusted_scope_response: Option<String>,
    /// Explicit operation/org/exact-candidate authority supplied only to an
    /// exact retained-DB resume. Unlike `--auto-approve`, this may resolve the
    /// TargetIntel -> EAS target review because it is bound to the complete
    /// presented set and an unchanged selected subset.
    retained_scope_authority: Option<scope_authority::RetainedScopeAuthority>,
    /// `Some` only for a fresh CLI invocation where the flag itself is trusted
    /// intake. Exact resume has no authority to create a new scope decision.
    include_subsidiaries: Option<bool>,
    /// Fresh CLI `--org` identity. A subsidiary-scope choice is machine
    /// resolvable only when its typed context names this exact seeded root.
    confirmed_organization_id: Option<uuid::Uuid>,
    /// Exact fresh CLI `--org` label. This only permits the backward-compatible
    /// natural-language form when the model names that root verbatim; it never
    /// gives a sibling or generic choice scope authority.
    confirmed_organization_name: Option<String>,
    /// Exact CLI authority corresponding to the GUI's phase Confirm action.
    approve_phase_boundaries: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StageRunAutoResolution {
    Approve(String),
    Decline(&'static str),
}

impl StageRunAutoApprovalPolicy {
    fn resolve(
        &self,
        input_type: &str,
        question: &str,
        options: &[String],
        context: &str,
    ) -> StageRunAutoResolution {
        match input_type {
            "scope_review" => {
                if let Some(authority) = &self.retained_scope_authority {
                    return authority
                        .resolve_response(context)
                        .map(StageRunAutoResolution::Approve)
                        .unwrap_or(StageRunAutoResolution::Decline(
                            "scope_review candidates differ from the explicit retained authority packet",
                        ));
                }
                self.trusted_scope_response
                    .clone()
                    .map(StageRunAutoResolution::Approve)
                    .unwrap_or(StageRunAutoResolution::Decline(
                        "scope_review requires exact trusted CLI target rows",
                    ))
            }
            "confirmation" if self.approve_phase_boundaries => StageRunAutoResolution::Approve(
                "approved by explicit CLI --approve-phase-boundaries".to_string(),
            ),
            "confirmation" => StageRunAutoResolution::Decline(
                "phase confirmation requires --approve-phase-boundaries",
            ),
            "choice" => {
                match subsidiary_scope_choice_org(context) {
                    Some(request_organization_id)
                        if self.confirmed_organization_id == Some(request_organization_id) => {}
                    Some(_) => {
                        return StageRunAutoResolution::Decline(
                            "subsidiary choice organization does not match trusted CLI --org",
                        );
                    }
                    None if self
                        .confirmed_organization_name
                        .as_deref()
                        .is_some_and(|name| {
                            legacy_subsidiary_choice_names_exact_root(
                                question, options, context, name,
                            )
                        }) => {}
                    None => {
                        return StageRunAutoResolution::Decline(
                            "choice is not a typed or exact-root subsidiary-scope decision",
                        );
                    }
                }
                let Some(include) = self.include_subsidiaries else {
                    return StageRunAutoResolution::Decline(
                        "resume cannot create a new subsidiary scope decision",
                    );
                };
                options
                    .iter()
                    .find(|option| {
                        if include {
                            subsidiary_option_includes(option)
                        } else {
                            subsidiary_option_excludes(option)
                        }
                    })
                    .cloned()
                    .map(StageRunAutoResolution::Approve)
                    .unwrap_or(StageRunAutoResolution::Decline(
                        "subsidiary choice has no option matching trusted CLI policy",
                    ))
            }
            "unit_review" => StageRunAutoResolution::Decline(
                "unit_review requires an explicit reviewed organization table",
            ),
            "credentials" => StageRunAutoResolution::Decline(
                "credentials cannot be synthesized by headless auto policy",
            ),
            "freetext" => StageRunAutoResolution::Decline(
                "freetext cannot be synthesized by headless auto policy",
            ),
            _ => StageRunAutoResolution::Decline(
                "unknown ask_human input type is not auto-authorized",
            ),
        }
    }
}

fn subsidiary_scope_choice_org(raw: &str) -> Option<uuid::Uuid> {
    // Match the persisted tool-call parser exactly: `context` is one JSON
    // object, never a JSON string containing a second JSON document.
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    (value.get("decision")?.as_str()? == "subsidiary_scope").then_some(())?;
    value
        .get("organization_id")?
        .as_str()?
        .parse::<uuid::Uuid>()
        .ok()
}

fn legacy_subsidiary_choice_names_exact_root(
    question: &str,
    options: &[String],
    context: &str,
    exact_root_name: &str,
) -> bool {
    let exact_root_name = exact_root_name.trim().to_lowercase();
    if exact_root_name.is_empty() {
        return false;
    }
    let authority_text = format!("{context} {question}").to_lowercase();
    let prompt = format!("{context} {question} {}", options.join(" ")).to_lowercase();
    authority_text.contains(&exact_root_name)
        && (prompt.contains("subsidiar")
            || prompt.contains("controlled holding")
            || prompt.contains("子公司")
            || prompt.contains("分支机构"))
}

fn subsidiary_option_excludes(option: &str) -> bool {
    let normalized = option.trim().to_lowercase();
    [
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
    .any(|marker| normalized.contains(marker))
}

fn subsidiary_option_includes(option: &str) -> bool {
    let normalized = option.trim().to_lowercase();
    !subsidiary_option_excludes(option)
        && [
            "include_subsidiaries",
            "included —",
            "included -",
            "纳入：",
            "纳入:",
            "纳入子公司",
            "include subsidiaries",
            "subsidiaries in scope",
        ]
        .iter()
        .any(|marker| normalized.contains(marker))
}

fn spawn_event_consumer(
    mut rx: mpsc::UnboundedReceiver<RuntimeEvent>,
    bridge: Arc<AgentBridge>,
    collected: Arc<Mutex<Vec<AiEvent>>>,
    auto_approve: bool,
    policy: StageRunAutoApprovalPolicy,
) {
    tokio::spawn(async move {
        while let Some(rt_ev) = rx.recv().await {
            let event = match rt_ev {
                RuntimeEvent::Ai { event, .. } => *event,
                RuntimeEvent::AiEnvelope { envelope, .. } => envelope.event,
                _ => continue,
            };

            if auto_approve {
                if let AiEvent::AskHumanRequest {
                    request_id,
                    input_type,
                    question,
                    options,
                    context,
                } = &event
                {
                    let (approved, reason) =
                        match policy.resolve(input_type, question, options, context) {
                            StageRunAutoResolution::Approve(response) => (true, response),
                            StageRunAutoResolution::Decline(reason) => {
                                tracing::warn!(
                                    input_type,
                                    reason,
                                    "stage-run: typed auto policy declined ask_human request"
                                );
                                (false, reason.to_string())
                            }
                        };
                    let decision = ApprovalDecision {
                        request_id: request_id.clone(),
                        approved,
                        reason: Some(reason),
                        remember: false,
                        always_allow: false,
                    };
                    let b = bridge.clone();
                    tokio::spawn(async move {
                        if let Err(e) = b.respond_to_approval(decision).await {
                            tracing::warn!("stage-run: auto-approve failed: {e}");
                        }
                    });
                }
            }

            if let Ok(mut v) = collected.lock() {
                v.push(event);
            }
        }
    });
}

/// Outcome of [`seed_upstream`]: what was created, for the report + objective.
struct SeedResult {
    org_id: Option<uuid::Uuid>,
    org_name: Option<String>,
    targets_added: usize,
}

/// Decide whether the current adapter input is trusted upstream intake.
/// Explicit CLI `--org` is a headless-only confirmed organization shortcut,
/// including at Scoping; it never implies a domain/IP/CIDR/URL target. An
/// explicit target is separate current-invocation target authority.
fn should_seed_upstream(
    _entry_stage: StageKind,
    org_name: Option<&str>,
    targets: &[String],
) -> bool {
    let has_target = targets.iter().any(|target| !target.trim().is_empty());
    let has_org = org_name.is_some_and(|name| !name.trim().is_empty());
    has_target || has_org
}

/// Run the trusted upstream seed only when [`should_seed_upstream`] accepts the
/// current adapter input. A failed write is fatal: continuing would silently
/// discard explicit CLI authority and make the shared orchestrator observe a
/// different scope than the caller requested.
async fn maybe_seed(
    db_pool: &Arc<sqlx::PgPool>,
    project_path: &str,
    entry_stage: StageKind,
    args: &Args,
) -> Result<Option<SeedResult>> {
    if !should_seed_upstream(entry_stage, args.org.as_deref(), &args.target) {
        return Ok(None);
    }
    let seed = seed_upstream(
        db_pool,
        project_path,
        args.org.as_deref(),
        args.stage_run_test_organization_id,
        &args.target,
    )
    .await
    .context("persist trusted CLI organization/target intake")?;
    eprintln!(
        "[stage-run] seeded upstream: org={:?} (id={:?}) targets={} project_path={project_path}",
        seed.org_name, seed.org_id, seed.targets_added
    );
    Ok(Some(seed))
}

/// Test enablement: seed an intel-provider API key into the vault from the file
/// path in `GOLISH_SEED_VAULT_KEY_FILE` (single line `provider=key`), so a
/// headless `--stage-run` can populate `organizations.*` via enrich without the
/// GUI. The value is obfuscated to match the vault read path
/// ([`golish_core::vault::deobfuscate`]) and upserted to both the canonical
/// `<provider>.default.api_key` row and the legacy provider-named row. Opt-in:
/// env unset → no-op. The key is read from a FILE
/// (never argv / process list / shell history). Best-effort: failures are logged
/// and the run continues (enrich will then surface the missing-credential gap).
async fn maybe_seed_vault_key(pool: &sqlx::PgPool) {
    let Ok(path) = std::env::var("GOLISH_SEED_VAULT_KEY_FILE") else {
        return;
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[stage-run] vault key seed: cannot read {path}: {e}");
            return;
        }
    };
    let Some((provider, key)) = raw.trim().split_once('=') else {
        eprintln!("[stage-run] vault key seed: file must contain 'provider=key'");
        return;
    };
    let (provider, key) = (provider.trim(), key.trim());
    if provider.is_empty() || key.is_empty() {
        eprintln!("[stage-run] vault key seed: empty provider or key");
        return;
    }
    let obfuscated = golish_core::vault::obfuscate(key);
    let (legacy_name, canonical_name) = vault_seed_entry_names(provider);
    let res: Result<(), sqlx::Error> = async {
        let mut tx = pool.begin().await?;
        sqlx::query("DELETE FROM vault_entries WHERE name = $1 AND entry_type = 'api_key'")
            .bind(&legacy_name)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO vault_entries (name, entry_type, value, notes, tags) \
             VALUES ($1, 'api_key'::vault_entry_type, $2, 'seeded by --stage-run', '[\"intel-provider\"]'::jsonb)",
        )
        .bind(&legacy_name)
        .bind(&obfuscated)
        .execute(&mut *tx)
        .await?;
        let canonical_tags = serde_json::json!([
            "integration",
            provider,
            "default",
            "intel-provider"
        ]);
        let updated = sqlx::query(
            "UPDATE vault_entries \
             SET value = $1, tags = $2, updated_at = NOW() \
             WHERE name = $3",
        )
        .bind(&obfuscated)
        .bind(&canonical_tags)
        .bind(&canonical_name)
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() == 0 {
            sqlx::query(
                "INSERT INTO vault_entries (name, entry_type, value, notes, tags) \
                 VALUES ($1, 'api_key'::vault_entry_type, $2, 'seeded by --stage-run', $3)",
            )
            .bind(&canonical_name)
            .bind(&obfuscated)
            .bind(&canonical_tags)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    .await;
    match res {
        Ok(()) => eprintln!("[stage-run] seeded vault api_key for '{provider}' (value redacted)"),
        Err(e) => eprintln!("[stage-run] vault key seed failed (continuing): {e}"),
    }
}

fn vault_seed_entry_names(provider: &str) -> (String, String) {
    (provider.to_string(), format!("{provider}.default.api_key"))
}

/// Create an organization (if named) + in-scope targets bound to it, scoped to
/// `project_path` (matching `manage_targets`/`manage_organizations`). Mirrors the
/// `manage_targets add` path (`target_add`, which defaults `scope='in'`) so the
/// gate's `in_scope_assets` and the recon tools both see the seed.
async fn seed_upstream(
    db_pool: &Arc<sqlx::PgPool>,
    project_path: &str,
    org_name: Option<&str>,
    exact_org_id: Option<uuid::Uuid>,
    targets: &[String],
) -> Result<SeedResult> {
    use golish_app_core::ports::recon::{PgReconTargetsAdapter, ReconTargetsPort};

    let mut org_id: Option<uuid::Uuid> = None;
    let mut org_name_out: Option<String> = None;
    if let Some(name) = org_name.map(str::trim).filter(|s| !s.is_empty()) {
        // get-or-create: the embedded PG persists across runs, so a repeated
        // `--org` would hit the `uq_orgs_root_name` unique constraint, abort the
        // whole seed, drop `org_id`, and silently fall back to the legacy
        // whole-DB coverage axis (org isolation never exercised). Reuse the
        // existing root org by name when present.
        let id =
            match golish_db::repo::organizations::find_root_id_by_name(db_pool, project_path, name)
                .await
                .context("seed organization lookup")?
            {
                Some(existing) => existing,
                None => match exact_org_id {
                    Some(exact_id) => sqlx::query_scalar::<_, uuid::Uuid>(
                        r#"INSERT INTO organizations(
                               id,project_path,name,parent_id,description,owner
                           ) VALUES($1,$2,$3,NULL,'','')
                           RETURNING id"#,
                    )
                    .bind(exact_id)
                    .bind(project_path)
                    .bind(name)
                    .fetch_one(db_pool.as_ref())
                    .await
                    .context("seed organization with exact isolated identity")?,
                    None => {
                        golish_db::repo::organizations::create(
                            db_pool,
                            project_path,
                            name,
                            None,
                            "",
                            "",
                        )
                        .await
                        .context("seed organization")?
                        .id
                    }
                },
            };
        if let Some(expected) = exact_org_id {
            if expected != id {
                return Err(anyhow!(
                    "exact isolated organization identity mismatch: expected {expected}, found {id}"
                ));
            }
        }
        org_id = Some(id);
        org_name_out = Some(name.to_string());
    }

    let adapter = PgReconTargetsAdapter::new(db_pool.clone());
    let mut targets_added = 0usize;
    for t in targets {
        let t = t.trim();
        if t.is_empty() {
            continue;
        }
        adapter
            .target_add(
                t,                  // name
                t,                  // value
                None,               // target_type (auto-detect)
                None,               // grp
                None,               // owner
                None,               // time_window_start
                None,               // time_window_end
                org_id,             // organization_id
                Some(project_path), // project_path
                "stage-run-seed",   // source
                None,               // parent_id
            )
            .await
            .with_context(|| format!("seed target {t}"))?;
        targets_added += 1;
    }

    Ok(SeedResult {
        org_id,
        org_name: org_name_out,
        targets_added,
    })
}

/// Smoke-test enablement: seed confirmed-open target ports into the current
/// stage-run DB. This is intentionally env-only and best-effort so normal
/// `--stage-run` behavior stays unchanged.
///
/// Format: `GOLISH_STAGE_RUN_SEED_OPEN_PORTS='192.0.2.10=80,443;192.0.2.11=9001'`.
async fn maybe_seed_open_ports(db_pool: &sqlx::PgPool, project_path: &str) {
    let Ok(raw) = std::env::var("GOLISH_STAGE_RUN_SEED_OPEN_PORTS") else {
        return;
    };
    let specs = parse_seed_open_ports(&raw);
    if specs.is_empty() {
        eprintln!("[stage-run] open-port seed ignored: no valid host=ports entries");
        return;
    }
    let mut updated = 0u64;
    for (target, ports) in specs {
        let ports_json = serde_json::Value::Array(
            ports
                .iter()
                .map(|port| {
                    serde_json::json!({
                        "port": port,
                        "protocol": "tcp",
                        "state": "open",
                        "source": "stage-run-seed",
                    })
                })
                .collect(),
        );
        match sqlx::query(
            "UPDATE targets \
                SET ports = $1, \
                    ports_scanned_at = NOW() + interval '1 hour', \
                    liveness_checked_at = NOW() + interval '1 hour', \
                    liveness_state = 'alive', \
                    liveness_reason = 'stage-run open-port seed', \
                    updated_at = NOW() \
              WHERE project_path = $2 \
                AND value = $3 \
                AND scope::text = 'in'",
        )
        .bind(ports_json)
        .bind(project_path)
        .bind(&target)
        .execute(db_pool)
        .await
        {
            Ok(result) => updated += result.rows_affected(),
            Err(err) => eprintln!("[stage-run] open-port seed failed for {target}: {err}"),
        }
    }
    eprintln!("[stage-run] seeded open ports for {updated} target row(s)");
}

const CONTROLLED_WEB_ORIGIN_SEED_ENV: &str = "GOLISH_STAGE_RUN_SEED_CONFIRMED_WEB_ORIGINS";

fn controlled_web_origin_identity(value: &str) -> Option<(String, String)> {
    let parsed = url::Url::parse(value.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.host().is_none()
    {
        return None;
    }
    let normalized = golish_db::repo::surface_identity::normalize_web_origin(value)?;
    let root_url = format!("{}/", normalized.origin);
    Some((normalized.origin, root_url))
}

fn parse_controlled_web_origin_seeds(raw: &str) -> Result<Vec<String>> {
    let mut seeds = BTreeMap::new();
    for candidate in raw
        .split(';')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let (key, root_url) = controlled_web_origin_identity(candidate)
            .ok_or_else(|| anyhow!("controlled Web Origin seed is not an absolute HTTP(S) URL"))?;
        seeds.insert(key, root_url);
    }
    if seeds.is_empty() {
        return Err(anyhow!("controlled Web Origin seed set is empty"));
    }
    Ok(seeds.into_values().collect())
}

/// Controlled-fixture enablement for a direct Enumeration entry.
///
/// A normal production run must obtain these rows from EAS.  The smoke wrapper
/// may bypass EAS to keep one focused Enumeration repair loop short, so this
/// hook materializes the exact local fixture origin before the operation and
/// stage timestamps are frozen.  The hook is accepted only with the paired
/// ephemeral controlled-provider overlay and only for a direct Enumeration
/// entry; it cannot widen an ordinary or full-chain run.
async fn maybe_seed_controlled_web_origins(
    db_pool: &sqlx::PgPool,
    project_path: &str,
    entry_stage: StageKind,
    args: &Args,
) -> Result<()> {
    let Ok(raw) = std::env::var(CONTROLLED_WEB_ORIGIN_SEED_ENV) else {
        return Ok(());
    };
    if entry_stage != StageKind::Enumeration
        || !args.ephemeral_db
        || args.stage_run_test_toolsconfig_dir.is_none()
        || args.stage_run_test_intel_providers_dir.is_none()
        || args.stage_run_test_intel_provider_endpoint.is_none()
    {
        return Err(anyhow!(
            "controlled Web Origin seed requires a direct Enumeration entry and the paired ephemeral controlled-provider overlay"
        ));
    }

    let seeds = parse_controlled_web_origin_seeds(&raw)?;
    let mut seeded = 0usize;
    for root_url in seeds {
        let (canonical_key, _) = controlled_web_origin_identity(&root_url)
            .ok_or_else(|| anyhow!("controlled Web Origin seed normalization failed"))?;
        let trusted_target = args
            .target
            .iter()
            .find(|target| {
                controlled_web_origin_identity(target)
                    .is_some_and(|(target_key, _)| target_key == canonical_key)
            })
            .ok_or_else(|| {
                anyhow!(
                    "controlled Web Origin seed is not present in the current invocation target set: {}",
                    canonical_key
                )
            })?;
        let targets = sqlx::query_as::<_, (uuid::Uuid, Option<uuid::Uuid>)>(
            r#"SELECT id,organization_id
                 FROM targets
                WHERE project_path=$1 AND value=$2 AND scope::text='in'
                ORDER BY id"#,
        )
        .bind(project_path)
        .bind(trusted_target)
        .fetch_all(db_pool)
        .await
        .context("load exact controlled fixture target")?;
        let [(target_id, Some(organization_id))] = targets.as_slice() else {
            return Err(anyhow!(
                "controlled Web Origin seed requires exactly one organization-owned in-scope target"
            ));
        };
        let normalized = golish_db::repo::surface_identity::normalize_web_origin(&root_url)
            .ok_or_else(|| anyhow!("controlled Web Origin DB normalization failed"))?;
        let origin = golish_db::repo::web_origins::upsert_by_identity(
            db_pool,
            Some(*organization_id),
            Some(project_path),
            &normalized,
            Some("stage-run-controlled-fixture"),
            Some(1.0),
            true,
        )
        .await
        .context("seed controlled fixture Web Origin")?;
        let raw = serde_json::json!({
            "controlled_fixture": true,
            "authority": "current_invocation_exact_target",
            "transport": "local_stage_smoke"
        });
        golish_db::repo::web_origin_observations::insert_observation(
            db_pool,
            &golish_db::repo::web_origin_observations::NewWebOriginObservation {
                organization_id: Some(*organization_id),
                project_path: Some(project_path),
                web_origin_id: origin.id,
                network_endpoint_id: None,
                target_id: Some(*target_id),
                observed_ip: None,
                sni: None,
                host_header: None,
                status_code: Some(200),
                title: Some("Golish controlled fixture"),
                final_url: Some(&root_url),
                redirect_chain: None,
                body_hash: None,
                favicon_hash: None,
                screenshot_path: None,
                capture_path: None,
                confidence: Some(1.0),
                source: Some("eas_probe_http_liveness"),
                raw: Some(&raw),
            },
        )
        .await
        .context("seed controlled fixture Web Origin observation")?;
        seeded += 1;
    }
    eprintln!("[stage-run] seeded {seeded} controlled fixture Web Origin observation(s)");
    Ok(())
}

fn parse_seed_open_ports(raw: &str) -> Vec<(String, Vec<u16>)> {
    raw.split(';')
        .filter_map(|entry| {
            let (target, ports_raw) = entry.split_once('=')?;
            let target = target.trim();
            if target.is_empty() {
                return None;
            }
            let ports = ports_raw
                .split(',')
                .filter_map(|port| {
                    let port = port.trim().parse::<u16>().ok()?;
                    (port > 0).then_some(port)
                })
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            (!ports.is_empty()).then(|| (target.to_string(), ports))
        })
        .collect()
}

/// Phase 3 (2026-06-12-redteam-phase3, 方案 A) · the stage slice a subsidiary
/// org run covers: subsidiaries never re-run scoping (authorization + org-tree
/// construction are engagement-level actions, done once in the parent run), so
/// the child entry is pinned to `target_intel`. `--to scoping` (tree-build
/// only) yields `None` — no per-subsidiary runs.
fn child_slice(profile_id: &str, to: StageKind) -> Option<(StageKind, HashSet<StageKind>)> {
    if to == StageKind::Scoping {
        return None;
    }
    // This subsidiary fan-out is reached only from the LegacyV1 execution
    // branch. Unified operations use the in-graph team runtime instead.
    resolve_slice(profile_id, Some(StageKind::TargetIntel), to).ok()
}

/// Build the task objective. `-e/--execute` controls the operator prose, but it
/// cannot erase server-owned intake metadata: the seeded organization name/id
/// and current-invocation targets are always appended so typed Scoping choices
/// can name the exact root without guessing across a persistent workspace.
fn build_objective(args: &Args, to: StageKind, seed: Option<&SeedResult>) -> String {
    let mut s = args
        .execute
        .clone()
        .unwrap_or_else(|| format!("Run the {} stage for this engagement.", to.as_str()));
    match seed {
        Some(sd) => {
            if let (Some(name), Some(id)) = (&sd.org_name, &sd.org_id) {
                s.push_str(&format!(" Organization: {name} (organization_id: {id})."));
            } else if let Some(name) = &sd.org_name {
                s.push_str(&format!(" Organization: {name}."));
            }
        }
        None => {
            if let Some(org) = &args.org {
                s.push_str(&format!(" Organization: {org}."));
            }
        }
    }
    if !args.target.is_empty() {
        s.push_str(&format!(" In-scope targets: {}.", args.target.join(", ")));
    }
    s
}

/// Wait (up to ~3 min) for the embedded DB to flip its ready gate.
async fn wait_for_db(db_ready: &golish_db::DbReadyGate) -> bool {
    for _ in 0..18 {
        if db_ready.is_ready() {
            return true;
        }
        if db_ready.is_failed() {
            return false;
        }
        if db_ready.wait_timeout(Duration::from_secs(10)).await {
            return true;
        }
    }
    db_ready.is_ready()
}

fn init_tracing_best_effort(settings: &golish_settings::GolishSettings, verbose: bool) {
    let log_level = if verbose { "debug" } else { "info" };
    let directives_owned: Vec<String> = [
        "golish",
        "golish_agent_kit",
        "golish_agent_runtime",
        "golish_agent_bridge",
        "golish_prompts",
        "harness",
    ]
    .iter()
    .map(|c| format!("{c}={log_level}"))
    .collect();
    let directives: Vec<&str> = directives_owned.iter().map(|s| s.as_str()).collect();
    let langfuse = crate::telemetry::LangfuseConfig::from_settings(&settings.telemetry.langfuse);
    let _ = crate::telemetry::init_tracing(langfuse, log_level, &directives);
}

#[derive(Debug, serde::Serialize)]
struct DbSmokeSummary {
    session_id: String,
    operation_id: Option<String>,
    operation_identity: serde_json::Value,
    organization_id: Option<String>,
    project_path: String,
    totals: BTreeMap<String, serde_json::Value>,
    run_scoped: BTreeMap<String, serde_json::Value>,
    operation_scoped: BTreeMap<String, serde_json::Value>,
    operation_exact_sets: BTreeMap<String, serde_json::Value>,
    project_scoped: BTreeMap<String, serde_json::Value>,
    org_scoped: BTreeMap<String, serde_json::Value>,
}

#[allow(clippy::explicit_auto_deref)]
async fn collect_db_smoke_summary(
    pool: &sqlx::PgPool,
    session_id: &str,
    org_id: Option<uuid::Uuid>,
    project_path: &str,
) -> Result<DbSmokeSummary> {
    let mut snapshot = pool
        .begin()
        .await
        .context("begin DB smoke summary snapshot")?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *snapshot)
        .await
        .context("configure DB smoke summary snapshot")?;
    let operation_ids = sqlx::query_scalar::<_, uuid::Uuid>(
        r#"SELECT DISTINCT operation.operation_id
             FROM sessions session
             JOIN tasks task ON task.session_id=session.id
             JOIN operation_state operation ON operation.operation_id=task.id
            WHERE session.chat_session_key=$1
            ORDER BY operation.operation_id"#,
    )
    .bind(session_id)
    .fetch_all(&mut *snapshot)
    .await
    .context("resolve exact stage-run operation")?;
    if operation_ids.len() != 1 {
        return Err(anyhow!(
            "stage_run_operation_resolution_not_exact: expected 1 operation for session {}, found {} ({:?})",
            session_id,
            operation_ids.len(),
            operation_ids
        ));
    }
    let operation_id = operation_ids[0];
    let operation_identity = sqlx::query_scalar::<_, serde_json::Value>(
        r#"SELECT jsonb_build_object(
                        'operationId',operation.operation_id,
                        'taskStatus',task.status,
                        'profile',operation.profile,
                        'currentStage',operation.current_stage,
                        'runtimeMemoryContract',operation.runtime_memory_contract,
                        'attackExecutionContract',operation.attack_execution_contract,
                        'enumerationAnalysisContract',operation.enumeration_analysis_contract,
                        'toolTruthContract',operation.tool_truth_contract,
                        'investigationContractVersion',operation.investigation_contract_version,
                        'investigationRolloutMode',operation.investigation_rollout_mode,
                        'stageTopologyContract',operation.stage_topology_contract,
                        'stageTopologySha256',operation.stage_topology_sha256,
                        'stageTopologyFreezeSource',operation.stage_topology_freeze_source,
                        'projectScopeId',operation.project_scope_id,
                        'engagementOrganizationId',operation.engagement_org_id
                    )
                    FROM operation_state operation
                    JOIN tasks task ON task.id=operation.operation_id
                   WHERE operation.operation_id=$1"#,
    )
    .bind(operation_id)
    .fetch_one(&mut *snapshot)
    .await
    .unwrap_or_else(|error| serde_json::json!({ "error": error.to_string() }));

    let totals = collect_unbound_counts(
        &mut snapshot,
        &[
            ("sessions", "SELECT COUNT(*) FROM sessions"),
            ("tasks", "SELECT COUNT(*) FROM tasks"),
            ("tool_calls", "SELECT COUNT(*) FROM tool_calls"),
            ("audit_log", "SELECT COUNT(*) FROM audit_log"),
            (
                "evidence_audit_log",
                "SELECT COUNT(*) FROM audit_log WHERE audit_role = 'evidence'",
            ),
            ("organizations", "SELECT COUNT(*) FROM organizations"),
            ("targets", "SELECT COUNT(*) FROM targets"),
            ("target_assets", "SELECT COUNT(*) FROM target_assets"),
            ("dns_records", "SELECT COUNT(*) FROM dns_records"),
            ("api_endpoints", "SELECT COUNT(*) FROM api_endpoints"),
            (
                "technique_outcomes",
                "SELECT COUNT(*) FROM technique_outcomes",
            ),
            ("source_query_log", "SELECT COUNT(*) FROM source_query_log"),
            (
                "org_stage_completions",
                "SELECT COUNT(*) FROM org_stage_completions",
            ),
        ],
    )
    .await?;

    let run_scoped = collect_text_counts(
        &mut snapshot,
        session_id,
        &[
            (
                "sessions_by_chat_key",
                "SELECT COUNT(*) FROM sessions WHERE chat_session_key = $1",
            ),
            (
                "tasks_by_chat_key",
                "SELECT COUNT(*) FROM tasks t \
                 JOIN sessions s ON t.session_id = s.id \
                 WHERE s.chat_session_key = $1",
            ),
            (
                "tool_calls_by_chat_key",
                "SELECT COUNT(*) FROM tool_calls tc \
                 JOIN sessions s ON tc.session_id = s.id \
                 WHERE s.chat_session_key = $1",
            ),
            (
                "technique_outcomes_by_run",
                "SELECT COUNT(*) FROM technique_outcomes WHERE run_id = $1",
            ),
            (
                "source_query_log_by_run",
                "SELECT COUNT(*) FROM source_query_log WHERE run_id = $1",
            ),
            (
                "org_stage_completions_by_run",
                "SELECT COUNT(*) FROM org_stage_completions WHERE stage_run_id = $1",
            ),
        ],
    )
    .await?;

    let operation_scoped =
            collect_uuid_counts(
                &mut snapshot,
                operation_id,
                &[
                    (
                        "stage_runs_by_operation",
                        "SELECT COUNT(*) FROM stage_runs WHERE operation_id=$1",
                    ),
                    (
                        "stage_units_by_operation",
                        "SELECT COUNT(*) FROM stage_run_units WHERE operation_id=$1",
                    ),
                    (
                        "team_plans_by_operation",
                        "SELECT COUNT(*) FROM stage_team_plans WHERE operation_id=$1",
                    ),
                    (
                        "work_items_by_operation",
                        "SELECT COUNT(*) FROM stage_work_items WHERE operation_id=$1",
                    ),
                    (
                        "worker_outputs_by_operation",
                        "SELECT COUNT(*) FROM stage_worker_outputs WHERE operation_id=$1",
                    ),
                    (
                        "deliverable_submissions_by_operation",
                        "SELECT COUNT(*) FROM stage_deliverable_submissions WHERE operation_id=$1",
                    ),
                    (
                        "tool_calls_by_operation",
                        "SELECT COUNT(*) FROM tool_calls WHERE operation_id=$1",
                    ),
                    (
                        "capability_receipts_by_operation",
                        "SELECT COUNT(*) FROM capability_execution_receipts receipt JOIN tool_truth_execution_authorities authority ON authority.id=receipt.execution_authority_id WHERE authority.operation_id=$1",
                    ),
                    (
                        "enumeration_lane_receipts_by_operation",
                        "SELECT COUNT(*) FROM enumeration_lane_commit_receipts WHERE operation_id=$1",
                    ),
                    (
                        "hypothesis_revisions_by_operation",
                        "SELECT COUNT(*) FROM attack_hypothesis_revisions WHERE operation_id=$1",
                    ),
                    (
                        "verification_campaigns_by_operation",
                        "SELECT COUNT(*) FROM verification_campaigns WHERE operation_id=$1",
                    ),
                    (
                        "reports_by_operation",
                        "SELECT COUNT(*) FROM reports WHERE operation_id=$1",
                    ),
        ],
    )
    .await?;
    let operation_exact_sets = collect_operation_exact_sets(&mut snapshot, operation_id).await?;

    let project_scoped = collect_text_counts(
        &mut snapshot,
        project_path,
        &[
            (
                "organizations_in_workspace",
                "SELECT COUNT(*) FROM organizations WHERE project_path = $1",
            ),
            (
                "targets_in_workspace",
                "SELECT COUNT(*) FROM targets WHERE project_path = $1",
            ),
            (
                "audit_log_in_workspace",
                "SELECT COUNT(*) FROM audit_log WHERE project_path = $1",
            ),
            (
                "evidence_audit_log_in_workspace",
                "SELECT COUNT(*) FROM audit_log \
                 WHERE project_path = $1 AND audit_role = 'evidence'",
            ),
            (
                "target_assets_in_workspace",
                "SELECT COUNT(*) FROM target_assets WHERE project_path = $1",
            ),
            (
                "api_endpoints_in_workspace",
                "SELECT COUNT(*) FROM api_endpoints WHERE project_path = $1",
            ),
        ],
    )
    .await?;

    let org_scoped = match org_id {
        Some(org_id) => {
            collect_uuid_counts(
                &mut snapshot,
                org_id,
                &[
                    (
                        "targets_by_org",
                        "SELECT COUNT(*) FROM targets WHERE organization_id = $1",
                    ),
                    (
                        "target_assets_by_org",
                        "SELECT COUNT(*) FROM target_assets ta \
                         JOIN targets t ON ta.target_id = t.id \
                         WHERE t.organization_id = $1",
                    ),
                    (
                        "api_endpoints_by_org",
                        "SELECT COUNT(*) FROM api_endpoints ae \
                         JOIN targets t ON ae.target_id = t.id \
                         WHERE t.organization_id = $1",
                    ),
                    (
                        "technique_outcomes_by_org",
                        "SELECT COUNT(*) FROM technique_outcomes WHERE organization_id = $1",
                    ),
                    (
                        "source_query_log_by_org",
                        "SELECT COUNT(*) FROM source_query_log WHERE organization_id = $1",
                    ),
                    (
                        "org_stage_completions_by_org",
                        "SELECT COUNT(*) FROM org_stage_completions WHERE organization_id = $1",
                    ),
                ],
            )
            .await?
        }
        None => BTreeMap::new(),
    };

    snapshot
        .commit()
        .await
        .context("commit DB smoke summary snapshot")?;

    Ok(DbSmokeSummary {
        session_id: session_id.to_string(),
        operation_id: Some(operation_id.to_string()),
        operation_identity,
        organization_id: org_id.map(|id| id.to_string()),
        project_path: project_path.to_string(),
        totals,
        run_scoped,
        operation_scoped,
        operation_exact_sets,
        project_scoped,
        org_scoped,
    })
}

async fn collect_operation_exact_sets(
    connection: &mut sqlx::PgConnection,
    operation_id: uuid::Uuid,
) -> Result<BTreeMap<String, serde_json::Value>> {
    let queries = [
        (
            "stage_runs",
            r#"WITH rows AS (
                    SELECT run.id AS row_id,
                           operation_stage_rank_for_topology(
                               operation.stage_topology_contract,run.stage_kind
                           ) AS ordinal,
                           jsonb_build_object(
                               'stageExecutionId',run.id,'stage',run.stage_kind,
                               'status',run.status,'completedAt',run.completed_at
                           ) AS member
                      FROM stage_runs run
                      JOIN operation_state operation ON operation.operation_id=run.operation_id
                     WHERE run.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(
                               jsonb_agg(member ORDER BY ordinal,row_id),
                               '[]'::jsonb
                           ) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "deliverable_submissions",
            r#"WITH rows AS (
                    SELECT operation_stage_rank_for_topology(
                               operation.stage_topology_contract,submission.stage_kind
                           ) AS stage_ordinal,
                           submission.id AS row_id,
                           jsonb_build_object(
                               'submissionId',submission.id,'stage',submission.stage_kind,
                               'organizationId',submission.organization_id,
                               'payloadSha256',submission.payload_sha256
                           ) AS member
                      FROM stage_deliverable_submissions submission
                      JOIN operation_state operation
                        ON operation.operation_id=submission.operation_id
                     WHERE submission.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(
                               jsonb_agg(member ORDER BY stage_ordinal,row_id),
                               '[]'::jsonb
                           ) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "capability_receipts",
            r#"WITH rows AS (
                    SELECT receipt.id AS row_id,
                           jsonb_build_object(
                               'receiptId',receipt.id,'capability',receipt.capability,
                               'attemptState',receipt.attempt_state,
                               'landingState',receipt.landing_state,
                               'observationState',receipt.observation_state,
                               'coverageExtent',receipt.coverage_extent,
                               'receiptAuthorityHash',receipt.receipt_authority_hash,
                               'reconciliationState',receipt.reconciliation_state
                           ) AS member
                      FROM capability_execution_receipts receipt
                      JOIN tool_truth_execution_authorities authority
                        ON authority.id=receipt.execution_authority_id
                     WHERE authority.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "evidence_ledger",
            r#"WITH rows AS (
                    SELECT audit.id AS row_id,
                           jsonb_build_object(
                               'evidenceAuditId',audit.id,
                               'action',audit.action,
                               'category',audit.category,
                               'targetId',audit.target_id,
                               'runId',audit.run_id,
                               'technique',audit.evidence_technique,
                               'asset',audit.evidence_asset,
                               'outcome',audit.evidence_outcome,
                               'status',audit.status,
                               'createdAt',audit.created_at
                           ) AS member
                      FROM audit_log audit
                     WHERE audit.audit_role='evidence'
                       AND (
                           audit.run_id=$1
                           OR audit.session_id IN (
                               SELECT session.chat_session_key
                                 FROM sessions session
                                 JOIN tasks task ON task.session_id=session.id
                                WHERE task.id=$1
                           )
                           OR EXISTS (
                               SELECT 1
                                 FROM evidence_classifications classification
                                 JOIN stage_runs run
                                   ON run.id=classification.producing_stage_run_id
                                WHERE classification.evidence_audit_id=audit.id
                                  AND run.operation_id=$1
                           )
                           OR EXISTS (
                               SELECT 1
                                 FROM tool_truth_evidence_production_bindings binding
                                WHERE binding.evidence_audit_id=audit.id
                                  AND binding.operation_id=$1
                           )
                       )
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "enumeration_lane_receipts",
            r#"WITH rows AS (
                    SELECT id AS row_id,
                           jsonb_build_object(
                               'receiptId',id,'lane',lane,'targetId',target_id,
                               'terminalDisposition',terminal_disposition,
                               'resolutionOccurrenceId',resolution_occurrence_id,
                               'entitySetSha256',entity_set_sha256,
                               'denominatorSetSha256',denominator_set_sha256,
                               'receiptSetSha256',receipt_set_sha256,
                               'missing',missing,'unresolvedCount',unresolved_count
                           ) AS member
                      FROM enumeration_lane_commit_receipts
                     WHERE operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "enumeration_endpoint_occurrences",
            r#"WITH rows AS (
                    SELECT occurrence.id AS row_id,
                           jsonb_build_object(
                               'occurrenceId',occurrence.id,
                               'organizationId',occurrence.organization_id,
                               'executionAuthorityId',occurrence.execution_authority_id,
                               'sourceTargetId',occurrence.source_target_id,
                               'sourceWebOriginId',occurrence.source_web_origin_id,
                               'resolvedTargetId',occurrence.resolved_target_id,
                               'resolvedWebOriginId',occurrence.resolved_web_origin_id,
                               'parentOccurrenceId',occurrence.parent_occurrence_id,
                               'sourceUrl',occurrence.source_url,
                               'documentUrl',occurrence.document_url,
                               'scriptUrl',occurrence.script_url,
                               'protocol',occurrence.protocol,'method',occurrence.method,
                               'observationKind',occurrence.observation_kind,
                               'inferenceLevel',occurrence.inference_level,
                               'resolutionStatus',occurrence.resolution_status,
                               'canonicalRequestUrl',occurrence.canonical_request_url,
                               'displayUrl',occurrence.display_url,
                               'resolutionReason',occurrence.resolution_reason,
                               'scopeDecision',occurrence.scope_decision,
                               'candidateClassification',occurrence.candidate_classification,
                               'routeKind',occurrence.route_kind,
                               'routeTemplate',occurrence.route_template,
                               'requestSent',occurrence.request_sent,
                               'requestSchemaHash',occurrence.request_schema_hash,
                               'runtimeSampleUrl',occurrence.runtime_sample_url,
                               'promotionEligible',occurrence.promotion_eligible,
                               'observedAt',occurrence.observed_at,
                               'createdAt',occurrence.created_at
                           ) AS member
                      FROM enumeration_endpoint_occurrences occurrence
                     WHERE occurrence.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "enumeration_parameter_assessments",
            r#"WITH rows AS (
                    SELECT assessment.id AS row_id,
                           jsonb_build_object(
                               'assessmentId',assessment.id,
                               'occurrenceId',assessment.occurrence_id,
                               'organizationId',assessment.organization_id,
                               'executionAuthorityId',assessment.execution_authority_id,
                               'denominatorId',assessment.denominator_id,
                               'denominatorItemId',assessment.denominator_item_id,
                               'terminalReceiptId',assessment.terminal_receipt_id,
                               'terminalReceiptInputId',assessment.terminal_receipt_input_id,
                               'parameterOutcome',assessment.parameter_outcome,
                               'reasonCode',assessment.reason_code,
                               'createdAt',assessment.created_at
                           ) AS member
                      FROM enumeration_endpoint_parameter_assessments assessment
                     WHERE assessment.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "enumeration_occurrence_parameters",
            r#"WITH rows AS (
                    SELECT parameter.id AS row_id,
                           jsonb_build_object(
                               'parameterId',parameter.id,
                               'assessmentId',parameter.assessment_id,
                               'name',parameter.name,
                               'location',parameter.location,
                               'valueType',parameter.value_type,
                               'requirement',parameter.requirement,
                               'confidence',parameter.confidence,
                               'sourceAnchorHash',tool_truth_sha256(to_jsonb(parameter.source_anchor)::TEXT),
                               'createdAt',parameter.created_at
                           ) AS member
                      FROM enumeration_endpoint_occurrence_parameters parameter
                      JOIN enumeration_endpoint_parameter_assessments assessment
                        ON assessment.id=parameter.assessment_id
                     WHERE assessment.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "enumeration_parameter_provenance",
            r#"WITH rows AS (
                    SELECT anchor.parameter_id::TEXT||':'||anchor.anchor_ordinal::TEXT AS row_id,
                           jsonb_build_object(
                               'parameterId',anchor.parameter_id,
                               'assessmentId',anchor.assessment_id,
                               'anchorOrdinal',anchor.anchor_ordinal,
                               'sourceAnchorHash',tool_truth_sha256(to_jsonb(anchor.source_anchor)::TEXT),
                               'createdAt',anchor.created_at
                           ) AS member
                      FROM enumeration_endpoint_occurrence_parameter_source_anchors anchor
                      JOIN enumeration_endpoint_parameter_assessments assessment
                        ON assessment.id=anchor.assessment_id
                     WHERE assessment.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "enumeration_resolution_closeouts",
            r#"WITH rows AS (
                    SELECT closeout.id AS row_id,
                           jsonb_build_object(
                               'closeoutId',closeout.id,
                               'organizationId',closeout.organization_id,
                               'parentOccurrenceId',closeout.parent_occurrence_id,
                               'producerLaneReceiptId',closeout.producer_lane_receipt_id,
                               'terminalState',closeout.terminal_state,
                               'reasonCode',closeout.reason_code,
                               'suggestionIds',closeout.suggestion_ids,
                               'terminalReceiptId',closeout.terminal_receipt_id,
                               'terminalReceiptInputId',closeout.terminal_receipt_input_id,
                               'evidenceSetSha256',closeout.evidence_set_sha256,
                               'closeoutSha256',closeout.closeout_sha256,
                               'createdAt',closeout.created_at
                           ) AS member
                      FROM enumeration_resolution_closeout_receipts closeout
                     WHERE closeout.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "target_intel_goal_epochs",
            r#"WITH rows AS (
                    SELECT epoch.id AS row_id,
                           jsonb_build_object(
                               'goalEpochId',epoch.id,
                               'organizationId',epoch.organization_id,
                               'epoch',epoch.epoch,'status',epoch.status,
                               'rowVersion',epoch.row_version,
                               'sealedAt',epoch.sealed_at,'terminalAt',epoch.terminal_at
                           ) AS member
                      FROM target_intel_goal_epochs epoch
                     WHERE epoch.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "target_intel_goal_reviews",
            r#"WITH rows AS (
                    SELECT review.id AS row_id,
                           jsonb_build_object(
                               'goalReviewId',review.id,
                               'organizationId',review.organization_id,
                               'goalEpoch',review.goal_epoch,
                               'reviewGeneration',review.review_generation,
                               'round',review.round,'status',review.status,
                               'bundleSha256',review.bundle_sha256,
                               'verdictSha256',review.verdict_sha256,
                               'rowVersion',review.row_version
                           ) AS member
                      FROM target_intel_goal_reviews review
                     WHERE review.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "target_intel_goal_frontier",
            r#"WITH rows AS (
                    SELECT frontier.id AS row_id,
                           jsonb_build_object(
                               'frontierId',frontier.id,
                               'organizationId',frontier.organization_id,
                               'goalEpoch',frontier.goal_epoch,
                               'pivotKind',frontier.pivot_kind,
                               'pivotValueSha256',frontier.pivot_value_sha256,
                               'intent',frontier.intent,'status',frontier.status,
                               'materiality',frontier.materiality,
                               'rowVersion',frontier.row_version,
                               'terminalRefCount',jsonb_array_length(frontier.terminal_refs)
                           ) AS member
                      FROM target_intel_goal_frontier_v2 frontier
                     WHERE frontier.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "target_intel_goal_work_journal",
            r#"WITH rows AS (
                    SELECT entry.id AS row_id,
                           jsonb_build_object(
                               'journalEntryId',entry.id,
                               'organizationId',entry.organization_id,
                               'teamPlanId',entry.team_plan_id,
                               'goalEpochId',entry.goal_epoch_id,
                               'goalEpoch',entry.goal_epoch,
                               'controllerWorkerRunId',entry.controller_worker_run_id,
                               'controllerMessageChainId',entry.controller_message_chain_id,
                               'ordinal',entry.ordinal,
                               'entryKind',entry.entry_kind,
                               'frontierRefCount',jsonb_array_length(entry.related_frontier_refs),
                               'evidenceRefCount',jsonb_array_length(entry.evidence_refs),
                               'toolCallRefCount',jsonb_array_length(entry.tool_call_refs),
                               'observationRefCount',jsonb_array_length(entry.observation_refs),
                               'entrySha256',entry.entry_sha256,
                               'createdAt',entry.created_at
                           ) AS member
                      FROM target_intel_goal_work_journal_entries entry
                     WHERE entry.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "target_intel_semantic_artifacts",
            r#"WITH rows AS (
                    SELECT artifact.organization_id::TEXT||':'||artifact.session_id::TEXT||':'||artifact.artifact_ref AS row_id,
                           jsonb_build_object(
                               'artifactRef',artifact.artifact_ref,
                               'organizationId',artifact.organization_id,
                               'sessionId',artifact.session_id,
                               'artifactSha256',artifact.artifact_sha256,
                               'createdAt',artifact.created_at
                           ) AS member
                      FROM target_intel_semantic_artifacts artifact
                     WHERE artifact.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "target_intel_asset_observations",
            r#"WITH rows AS (
                    SELECT observation.id AS row_id,
                           jsonb_build_object(
                               'observationId',observation.id,
                               'organizationId',observation.organization_id,
                               'goalEpochId',observation.goal_epoch_id,
                               'producerWorkerRunId',observation.producer_worker_run_id,
                               'producerToolCallId',observation.producer_tool_call_id,
                               'semanticReceiptAuditId',observation.semantic_receipt_audit_id,
                               'evidenceId',observation.evidence_id,
                               'artifactRef',observation.artifact_ref,
                               'providerId',observation.provider_id,
                               'providerQueryType',observation.provider_query_type,
                               'adapterVersion',observation.adapter_version,
                               'stableQueryKey',observation.stable_query_key,
                               'assetKind',observation.asset_kind,
                               'canonicalIdentitySha256',observation.canonical_identity_sha256,
                               'observationSha256',observation.observation_sha256,
                               'attributionDisposition',observation.attribution_disposition,
                               'reachabilityState',observation.reachability_state,
                               'promotionTargetId',observation.promotion_target_id,
                               'rowVersion',observation.row_version,
                               'observedAt',observation.observed_at
                           ) AS member
                      FROM target_intel_asset_observations observation
                     WHERE observation.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "stage_handoffs",
            r#"WITH rows AS (
                    SELECT handoff.id AS row_id,
                           jsonb_build_object(
                               'handoffId',handoff.id,
                               'organizationId',handoff.organization_id,
                               'scopeSnapshotId',handoff.scope_snapshot_id,
                               'fromStage',handoff.from_stage_kind,
                               'stageExecutionId',handoff.stage_execution_id,
                               'sourceStageRunUnitId',handoff.source_stage_run_unit_id,
                               'deliverableSubmissionId',handoff.deliverable_submission_id,
                               'scopeHash',handoff.scope_hash,
                               'payloadSha256',handoff.payload_sha256,
                               'evidenceCount',cardinality(handoff.evidence_ids),
                               'unitGateDecisionHash',handoff.unit_gate_decision_hash,
                               'aggregatePassTokenHash',handoff.aggregate_pass_token_hash,
                               'invalidatedAt',handoff.invalidated_at,
                               'schemaVersion',handoff.schema_version
                           ) AS member
                      FROM stage_handoffs handoff
                     WHERE handoff.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "application_model_revisions",
            r#"WITH rows AS (
                    SELECT revision.id AS row_id,
                           jsonb_build_object(
                               'applicationModelRevisionId',revision.id,
                               'manifestId',revision.manifest_id,
                               'scopeSnapshotId',revision.scope_snapshot_id,
                               'stageExecutionId',revision.stage_execution_id,
                               'stageRunUnitId',revision.stage_run_unit_id,
                               'organizationId',revision.organization_id,
                               'revisionOrdinal',revision.revision_ordinal,
                               'schemaVersion',revision.schema_version,
                               'status',revision.status,
                               'modelHash',revision.model_hash,
                               'replayMaterialHash',revision.replay_material_hash,
                               'sourceSubmissionId',revision.source_submission_id,
                               'rowVersion',revision.row_version,
                               'createdAt',revision.created_at,
                               'finalizedAt',revision.finalized_at
                           ) AS member
                      FROM application_model_revisions revision
                     WHERE revision.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "investigation_run_heads",
            r#"WITH rows AS (
                    SELECT head.authority_id AS row_id,
                           jsonb_build_object(
                               'authorityId',head.authority_id,
                               'stageExecutionId',head.stage_execution_id,
                               'scopeSnapshotId',head.scope_snapshot_id,
                               'runState',head.run_state,
                               'admissionOpen',head.admission_open,
                               'stopEpoch',head.stop_epoch,
                               'changeSeq',head.change_seq,
                               'headVersion',head.head_version,
                               'headSha256',head.head_sha256
                           ) AS member
                      FROM investigation_run_heads head
                     WHERE head.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "investigation_main_read_sessions",
            r#"WITH rows AS (
                    SELECT read_session.main_read_session_id AS row_id,
                           jsonb_build_object(
                               'mainReadSessionId',read_session.main_read_session_id,
                               'authorityId',read_session.authority_id,
                               'organizationId',read_session.organization_id,
                               'snapshotId',read_session.snapshot_id,
                               'snapshotSha256',read_session.snapshot_sha256,
                               'contextChainId',read_session.context_chain_id,
                               'transcriptPartitionId',read_session.transcript_partition_id,
                               'sessionContractVersion',read_session.session_contract_version,
                               'memberSha256',read_session.member_sha256
                           ) AS member
                      FROM investigation_main_read_sessions read_session
                     WHERE read_session.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "investigation_analysis_bindings",
            r#"WITH rows AS (
                    SELECT binding.binding_id AS row_id,
                           jsonb_build_object(
                               'bindingId',binding.binding_id,
                               'authorityId',binding.authority_id,
                               'organizationId',binding.organization_id,
                               'workId',binding.work_id,
                               'candidateSnapshotId',binding.candidate_snapshot_id,
                               'analysisAttemptId',binding.analysis_attempt_id,
                               'attemptOrdinal',binding.attempt_ordinal,
                               'contractVersion',binding.contract_version
                           ) AS member
                      FROM investigation_analysis_attempt_bindings binding
                     WHERE binding.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "investigation_task_plans",
            r#"WITH rows AS (
                    SELECT plan.task_plan_id AS row_id,
                           jsonb_build_object(
                               'taskPlanId',plan.task_plan_id,
                               'authorityId',plan.authority_id,
                               'organizationId',plan.organization_id,
                               'subjectKind',plan.subject_kind,
                               'subjectId',plan.subject_id,
                               'subjectFingerprintSha256',plan.subject_fingerprint_sha256,
                               'taskPlanVersion',plan.task_plan_version,
                               'taskPlanSha256',plan.task_plan_sha256,
                               'status',plan.status,
                               'subtaskCount',plan.subtask_count,
                               'subtaskSetSha256',plan.subtask_set_sha256,
                               'rowVersion',plan.row_version
                           ) AS member
                      FROM investigation_pentagi_task_plans plan
                     WHERE plan.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "investigation_delegation_census",
            r#"WITH rows AS (
                    SELECT census.census_seal_id AS row_id,
                           jsonb_build_object(
                               'censusSealId',census.census_seal_id,
                               'taskPlanId',census.task_plan_id,
                               'organizationId',plan.organization_id,
                               'primaryDispatchReceiptId',census.primary_dispatch_receipt_id,
                               'primaryWorkerRunId',census.primary_worker_run_id,
                               'runnableSubtaskCount',census.runnable_subtask_count,
                               'runnableSubtaskSetSha256',census.runnable_subtask_set_sha256,
                               'dispatchCount',census.dispatch_count,
                               'dispatchSetSha256',census.dispatch_set_sha256,
                               'pipelineEventCount',census.pipeline_event_count,
                               'pipelineEventSetSha256',census.pipeline_event_set_sha256,
                               'sealSha256',census.seal_sha256
                           ) AS member
                      FROM investigation_pentagi_delegation_census_seals census
                      JOIN investigation_pentagi_task_plans plan
                        ON plan.task_plan_id=census.task_plan_id
                     WHERE plan.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "hypothesis_revisions",
            r#"WITH rows AS (
                    SELECT revision_id AS row_id,
                           jsonb_build_object(
                               'revisionId',revision_id,'rootId',root_id,
                               'epistemicState',epistemic_state,
                               'lifecycleState',lifecycle_state,
                               'planningReadiness',planning_readiness,
                               'revisionHash',revision_hash
                           ) AS member
                      FROM attack_hypothesis_revisions
                     WHERE operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "hypothesis_verification_tasks",
            r#"WITH rows AS (
                    SELECT task.task_id AS row_id,
                           jsonb_build_object(
                               'taskId',task.task_id,
                               'organizationId',task.organization_id,
                               'hypothesisRevisionId',task.hypothesis_revision_id,
                               'hypothesisRevisionSha256',task.hypothesis_revision_sha256,
                               'verificationPlanId',task.verification_plan_id,
                               'verificationPlanSha256',task.verification_plan_sha256,
                               'semanticEvidenceSetSha256',task.semantic_evidence_set_sha256,
                               'openObligationSetSha256',task.open_obligation_set_sha256,
                               'semanticAttemptFingerprint',task.semantic_attempt_fingerprint,
                               'currentState',head.current_state,
                               'headVersion',head.head_version
                           ) AS member
                      FROM hypothesis_verification_tasks task
                      JOIN hypothesis_verification_task_state_heads head
                        ON head.task_id=task.task_id
                     WHERE task.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "verification_campaigns",
            r#"WITH rows AS (
                    SELECT campaign_id AS row_id,
                           jsonb_build_object(
                               'campaignId',campaign_id,'state',state,
                               'hypothesisRevisionId',hypothesis_revision_id,
                               'verificationContractHash',verification_contract_hash,
                               'sourceSnapshotHash',source_snapshot_hash
                           ) AS member
                      FROM verification_campaigns
                     WHERE operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "verification_prepared_actions",
            r#"WITH rows AS (
                    SELECT action.prepared_action_id AS row_id,
                           jsonb_build_object(
                               'preparedActionId',action.prepared_action_id,
                               'campaignId',action.campaign_id,
                               'organizationId',action.organization_id,
                               'actionOrdinal',action.action_ordinal,
                               'actionContractKind',action.action_contract_kind,
                               'actionKind',action.action_kind,
                               'canonicalRequestHash',action.canonical_request_hash,
                               'rendererVersion',action.renderer_version,
                               'riskTier',action.risk_tier,
                               'state',action.state,'reasonCode',action.reason_code,
                               'residualId',action.residual_id,
                               'rowVersion',action.row_version
                           ) AS member
                      FROM verification_prepared_actions action
                     WHERE action.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "verification_action_authorizations",
            r#"WITH rows AS (
                    SELECT auth.authorization_receipt_id AS row_id,
                           jsonb_build_object(
                               'authorizationReceiptId',auth.authorization_receipt_id,
                               'preparedActionId',auth.prepared_action_id,
                               'campaignId',auth.campaign_id,
                               'organizationId',auth.organization_id,
                               'decision',auth.decision,
                               'decisionReasonCode',auth.decision_reason_code,
                               'expectedActionRowVersion',auth.expected_action_row_version,
                               'campaignDispatchGeneration',auth.campaign_dispatch_generation,
                               'rendererVersion',auth.renderer_version,
                               'reviewedActionHash',auth.reviewed_action_hash,
                               'authorizationHash',auth.authorization_hash,
                               'actorKind',auth.actor_kind,
                               'operatorChannel',auth.operator_channel,
                               'residualId',auth.residual_id
                           ) AS member
                      FROM verification_prepared_action_authorizations auth
                     WHERE auth.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "verification_action_executions",
            r#"WITH rows AS (
                    SELECT execution.action_execution_id AS row_id,
                           jsonb_build_object(
                               'actionExecutionId',execution.action_execution_id,
                               'preparedActionId',execution.prepared_action_id,
                               'authorizationReceiptId',execution.authorization_receipt_id,
                               'organizationId',execution.organization_id,
                               'executionOrdinal',execution.execution_ordinal,
                               'executionKind',execution.execution_kind,
                               'state',execution.state,
                               'campaignDispatchGeneration',execution.campaign_dispatch_generation,
                               'durableBeginHash',execution.durable_begin_hash,
                               'capabilityExecutionReceiptId',execution.capability_execution_receipt_id,
                               'closeoutHash',execution.closeout_hash,
                               'rowVersion',execution.row_version
                           ) AS member
                      FROM verification_action_executions execution
                     WHERE execution.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "verification_fact_deltas",
            r#"WITH rows AS (
                    SELECT delta.fact_delta_bundle_id AS row_id,
                           jsonb_build_object(
                               'factDeltaBundleId',delta.fact_delta_bundle_id,
                               'campaignId',delta.campaign_id,
                               'organizationId',delta.organization_id,
                               'hypothesisRevisionId',delta.hypothesis_revision_id,
                               'verificationObjectiveId',delta.verification_objective_id,
                               'deltaKind',delta.delta_kind,
                               'evidenceRefSetHash',delta.evidence_ref_set_hash,
                               'sourceAuthorityHash',delta.source_authority_hash,
                               'factDeltaHash',delta.fact_delta_hash
                           ) AS member
                      FROM verification_fact_delta_bundles delta
                     WHERE delta.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "hypothesis_residual_risks",
            r#"WITH rows AS (
                    SELECT residual.residual_id AS row_id,
                           jsonb_build_object(
                               'residualId',residual.residual_id,
                               'organizationId',residual.organization_id,
                               'revisionId',residual.revision_id,
                               'snapshotId',residual.snapshot_id,
                               'reasonCode',residual.reason_code,
                               'ownerKind',residual.owner_kind,
                               'residualHash',residual.residual_hash,
                               'closedAt',residual.closed_at
                           ) AS member
                      FROM hypothesis_residual_risks residual
                     WHERE residual.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "investigation_run_closures",
            r#"WITH rows AS (
                    SELECT closure.closure_id AS row_id,
                           jsonb_build_object(
                               'closureId',closure.closure_id,
                               'authorityId',closure.authority_id,
                               'disposition',closure.disposition,
                               'stopEpoch',closure.stop_epoch,
                               'workCount',closure.work_count,
                               'workSetSha256',closure.work_set_sha256,
                               'taskPlanCount',closure.task_plan_count,
                               'taskPlanSetSha256',closure.task_plan_set_sha256,
                               'dispatchCount',closure.dispatch_count,
                               'dispatchSetSha256',closure.dispatch_set_sha256,
                               'residualSetSha256',closure.residual_set_sha256,
                               'closureSha256',closure.closure_sha256
                           ) AS member
                      FROM investigation_run_closures closure
                     WHERE closure.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "investigation_stop_intents",
            r#"WITH rows AS (
                    SELECT intent.stop_intent_id AS row_id,
                           jsonb_build_object(
                               'stopIntentId',intent.stop_intent_id,
                               'authorityId',intent.authority_id,
                               'stageExecutionId',intent.stage_execution_id,
                               'owningStageRunRequestId',intent.owning_stage_run_request_id,
                               'expectedRunHeadSha256',intent.expected_run_head_sha256,
                               'expectedChangeSeq',intent.expected_change_seq,
                               'stopEpoch',intent.stop_epoch,
                               'frozenWorkCount',intent.frozen_work_count,
                               'frozenWorkSetSha256',intent.frozen_work_set_sha256,
                               'receiptSha256',intent.receipt_sha256,
                               'createdAt',intent.created_at
                           ) AS member
                      FROM investigation_stop_intents intent
                     WHERE intent.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "investigation_projection_outbox",
            r#"WITH rows AS (
                    SELECT outbox.outbox_member_id AS row_id,
                           jsonb_build_object(
                               'outboxMemberId',outbox.outbox_member_id,
                               'batchId',outbox.batch_id,
                               'sourceBatchSeq',outbox.source_batch_seq,
                               'memberOrdinal',outbox.member_ordinal,
                               'entityKind',outbox.entity_kind,
                               'changeKind',outbox.change_kind,
                               'sourceEntityId',outbox.source_entity_id,
                               'sourceEntityVersion',outbox.source_entity_version,
                               'sourceEntityHash',outbox.source_entity_hash,
                               'sourceSnapshotHash',outbox.source_snapshot_hash,
                               'timelineEventKind',outbox.timeline_event_kind,
                               'memberHash',outbox.member_hash,
                               'createdAt',outbox.created_at
                           ) AS member
                      FROM investigation_projection_outbox outbox
                     WHERE outbox.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "investigation_closure_publications",
            r#"WITH rows AS (
                    SELECT publication.publication_id AS row_id,
                           jsonb_build_object(
                               'publicationId',publication.publication_id,
                               'closureId',publication.closure_id,
                               'authorityId',publication.authority_id,
                               'stageExecutionId',publication.stage_execution_id,
                               'scopeSnapshotId',publication.scope_snapshot_id,
                               'disposition',publication.disposition,
                               'memberCount',publication.member_count,
                               'memberSetSha256',publication.member_set_sha256,
                               'closureSha256',publication.closure_sha256,
                               'publicationSha256',publication.publication_sha256
                           ) AS member
                      FROM investigation_stage_closure_publications publication
                     WHERE publication.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "report_revisions",
            r#"WITH rows AS (
                    SELECT revision.revision_id AS row_id,
                           jsonb_build_object(
                               'reportId',report.report_id,
                               'revisionId',revision.revision_id,
                               'revisionNumber',revision.revision_number,
                               'validationStatus',revision.validation_status,
                               'publicationStatus',revision.publication_status,
                               'sourceSetHash',revision.source_set_hash
                           ) AS member
                      FROM reports report
                      JOIN report_revisions revision ON revision.report_id=report.report_id
                     WHERE report.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "report_input_seals",
            r#"WITH rows AS (
                    SELECT seal.seal_id AS row_id,
                           jsonb_build_object(
                               'sealId',seal.seal_id,
                               'openId',seal.open_id,
                               'revisionId',seal.revision_id,
                               'toolTruthAuthoritySetId',seal.tool_truth_authority_set_id,
                               'revisionAdjudicationAuthoritySetId',seal.revision_adjudication_authority_set_id,
                               'legacyReportAuthoritySealId',seal.legacy_report_authority_seal_id,
                               'sourceMemberCount',seal.source_member_count,
                               'sourceSetHash','sha256:'||encode(seal.source_set_hash,'hex'),
                               'reportInputHash','sha256:'||encode(seal.report_input_hash,'hex'),
                               'effectiveValidUntil',seal.effective_valid_until,
                               'sealedAt',seal.sealed_at
                           ) AS member
                      FROM report_input_seals seal
                     WHERE seal.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "report_revision_artifacts",
            r#"WITH rows AS (
                    SELECT artifact.revision_id::TEXT||':'||artifact.artifact_kind AS row_id,
                           jsonb_build_object(
                               'revisionId',artifact.revision_id,
                               'artifactKind',artifact.artifact_kind,
                               'contentKey',artifact.content_key,
                               'redactionVersion',artifact.redaction_version
                           ) AS member
                      FROM report_revision_artifacts artifact
                      JOIN report_revisions revision
                        ON revision.revision_id=artifact.revision_id
                      JOIN reports report ON report.report_id=revision.report_id
                     WHERE report.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "report_source_manifest",
            r#"WITH rows AS (
                    SELECT manifest.revision_id::TEXT||':'||manifest.ordinal::TEXT AS row_id,
                           jsonb_build_object(
                               'revisionId',manifest.revision_id,
                               'ordinal',manifest.ordinal,
                               'sourceKind',manifest.source_kind,
                               'sourceIdKind',manifest.source_id_kind,
                               'sourceIdValue',manifest.source_id_value,
                               'sourceRowVersion',manifest.source_row_version,
                               'contentHash','sha256:'||encode(manifest.content_hash,'hex')
                           ) AS member
                      FROM report_source_manifest manifest
                      JOIN report_revisions revision ON revision.revision_id=manifest.revision_id
                      JOIN reports report ON report.report_id=revision.report_id
                     WHERE report.operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
        (
            "operation_contract_adoptions",
            r#"WITH rows AS (
                    SELECT adoption.adoption_id AS row_id,
                           jsonb_build_object(
                               'adoptionId',adoption.adoption_id,
                               'sourceOperationId',adoption.source_operation_id,
                               'targetOperationId',adoption.target_operation_id,
                               'sourceJointRank',adoption.source_joint_rank,
                               'targetJointRank',adoption.target_joint_rank,
                               'sourceFinalSealHash',adoption.source_final_seal_hash,
                               'adoptionSetHash',adoption.adoption_set_hash,
                               'receiptHash',adoption.receipt_hash,
                               'createdAt',adoption.created_at
                           ) AS member
                      FROM operation_contract_adoptions adoption
                     WHERE adoption.target_operation_id=$1
                ), aggregate AS (
                    SELECT COUNT(*)::BIGINT AS member_count,
                           COALESCE(jsonb_agg(member ORDER BY row_id), '[]'::jsonb) AS members
                      FROM rows
                )
                SELECT jsonb_build_object(
                    'memberCount',member_count,
                    'memberSetHash',tool_truth_sha256(members::TEXT),
                    'members',members
                ) FROM aggregate"#,
        ),
    ];
    let mut sets = BTreeMap::new();
    for (label, sql) in queries {
        let value = sqlx::query_scalar::<_, serde_json::Value>(sql)
            .bind(operation_id)
            .fetch_one(&mut *connection)
            .await
            .with_context(|| format!("collect operation exact set {label}"))?;
        sets.insert(label.to_owned(), value);
    }
    Ok(sets)
}

async fn collect_unbound_counts(
    connection: &mut sqlx::PgConnection,
    queries: &[(&'static str, &'static str)],
) -> Result<BTreeMap<String, serde_json::Value>> {
    let mut out = BTreeMap::new();
    for (label, sql) in queries {
        out.insert(
            (*label).to_string(),
            count_unbound(&mut *connection, sql)
                .await
                .with_context(|| format!("collect DB smoke count {label}"))?,
        );
    }
    Ok(out)
}

async fn collect_text_counts(
    connection: &mut sqlx::PgConnection,
    value: &str,
    queries: &[(&'static str, &'static str)],
) -> Result<BTreeMap<String, serde_json::Value>> {
    let mut out = BTreeMap::new();
    for (label, sql) in queries {
        out.insert(
            (*label).to_string(),
            count_text(&mut *connection, sql, value)
                .await
                .with_context(|| format!("collect DB smoke count {label}"))?,
        );
    }
    Ok(out)
}

async fn collect_uuid_counts(
    connection: &mut sqlx::PgConnection,
    value: uuid::Uuid,
    queries: &[(&'static str, &'static str)],
) -> Result<BTreeMap<String, serde_json::Value>> {
    let mut out = BTreeMap::new();
    for (label, sql) in queries {
        out.insert(
            (*label).to_string(),
            count_uuid(&mut *connection, sql, value)
                .await
                .with_context(|| format!("collect DB smoke count {label}"))?,
        );
    }
    Ok(out)
}

async fn count_unbound(
    connection: &mut sqlx::PgConnection,
    sql: &str,
) -> Result<serde_json::Value> {
    let count = sqlx::query_scalar::<_, i64>(sql)
        .fetch_one(&mut *connection)
        .await?;
    Ok(serde_json::json!(count))
}

async fn count_text(
    connection: &mut sqlx::PgConnection,
    sql: &str,
    value: &str,
) -> Result<serde_json::Value> {
    let count = sqlx::query_scalar::<_, i64>(sql)
        .bind(value)
        .fetch_one(&mut *connection)
        .await?;
    Ok(serde_json::json!(count))
}

async fn count_uuid(
    connection: &mut sqlx::PgConnection,
    sql: &str,
    value: uuid::Uuid,
) -> Result<serde_json::Value> {
    let count = sqlx::query_scalar::<_, i64>(sql)
        .bind(value)
        .fetch_one(&mut *connection)
        .await?;
    Ok(serde_json::json!(count))
}

fn format_db_smoke_summary(summary: &DbSmokeSummary) -> String {
    let mut out = String::new();
    out.push_str("\n-- db smoke summary --\n");
    out.push_str(&format!("  session_id = {}\n", summary.session_id));
    if let Some(operation_id) = &summary.operation_id {
        out.push_str(&format!("  operation_id = {operation_id}\n"));
    }
    out.push_str(&format!(
        "  operation_identity = {}\n",
        format_db_summary_value(&summary.operation_identity)
    ));
    out.push_str(&format!("  project_path = {}\n", summary.project_path));
    if let Some(org_id) = &summary.organization_id {
        out.push_str(&format!("  organization_id = {org_id}\n"));
    }
    push_db_summary_section(&mut out, "totals", &summary.totals);
    push_db_summary_section(&mut out, "run scoped", &summary.run_scoped);
    if !summary.operation_scoped.is_empty() {
        push_db_summary_section(&mut out, "operation scoped", &summary.operation_scoped);
    }
    if !summary.operation_exact_sets.is_empty() {
        push_db_summary_section(
            &mut out,
            "operation exact sets",
            &summary.operation_exact_sets,
        );
    }
    push_db_summary_section(&mut out, "project scoped", &summary.project_scoped);
    if !summary.org_scoped.is_empty() {
        push_db_summary_section(&mut out, "org scoped", &summary.org_scoped);
    }
    out
}

fn push_db_summary_section(
    out: &mut String,
    title: &str,
    values: &BTreeMap<String, serde_json::Value>,
) {
    out.push_str(&format!("  {title}:\n"));
    for (label, value) in values {
        out.push_str(&format!(
            "    {label}: {}\n",
            format_db_summary_value(value)
        ));
    }
}

fn format_db_summary_value(value: &serde_json::Value) -> String {
    if let Some(count) = value.as_i64() {
        return count.to_string();
    }
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}

fn redacted_typed_tool_failure(result: &serde_json::Value) -> Option<(String, String)> {
    fn bounded_single_line(value: &str, max_chars: usize) -> String {
        let single_line = value.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut chars = single_line.chars();
        let mut bounded = chars.by_ref().take(max_chars).collect::<String>();
        if chars.next().is_some() {
            bounded.push('…');
        }
        bounded
    }

    let code = result.get("code")?.as_str()?.trim();
    let error = result.get("error")?.as_str()?.trim();
    if !code.starts_with("INVESTIGATION_")
        || !code.chars().all(|character| {
            character.is_ascii_uppercase() || character == '_' || character.is_ascii_digit()
        })
    {
        return None;
    }
    let typed_error_offset = ["investigation_analysis_host_revalidation_required:"]
        .into_iter()
        .filter_map(|marker| error.find(marker))
        .min()?;
    let error = &error[typed_error_offset..];
    Some((
        bounded_single_line(code, 96),
        bounded_single_line(error, 640),
    ))
}

/// Render the human-readable post-run report from collected events.
fn format_report(
    events: &[AiEvent],
    result: &Result<String>,
    profile: &str,
    entry: StageKind,
    to: StageKind,
    session_id: &str,
    transcripts_dir: &Path,
) -> String {
    let mut out = String::new();
    out.push_str("\n══════════════ stage-run report ══════════════\n");
    out.push_str(&format!(
        "profile = {profile}\nslice   = {} ..= {}\n",
        entry.as_str(),
        to.as_str()
    ));

    let mut gate_lines = Vec::new();
    let mut evidence_lines = Vec::new();
    let mut tool_lines = Vec::new();
    let mut askhuman = 0usize;
    let mut askhuman_approved = 0usize;
    let mut askhuman_declined = 0usize;
    let mut errors = Vec::new();

    for ev in events {
        match ev {
            AiEvent::HarnessTrace { stage, trace, .. } => match trace {
                HarnessTraceKind::GateDecision {
                    gate,
                    findings,
                    first_blocking_reason,
                    fabricated_evidence_refs,
                    ..
                } => {
                    let mut l = format!("  [{gate}] {stage} (findings={findings})");
                    if gate == "BLOCK" {
                        if let Some(r) = first_blocking_reason {
                            l.push_str(&format!("\n         reason: {r}"));
                        }
                        if !fabricated_evidence_refs.is_empty() {
                            l.push_str(&format!(
                                "\n         fabricated evidence refs: {fabricated_evidence_refs:?}"
                            ));
                        }
                    }
                    gate_lines.push(l);
                }
                HarnessTraceKind::EvidenceBooked {
                    tool,
                    evidence_id,
                    source,
                } => {
                    evidence_lines.push(format!("  #{evidence_id} from {tool} ({source})"));
                }
                _ => {}
            },
            AiEvent::ToolResult {
                tool_name,
                result,
                success,
                ..
            } => {
                let mut line = format!("  {tool_name}: {}", if *success { "ok" } else { "err" });
                if !*success && tool_name == "stage_run" {
                    if let Some((code, error)) = redacted_typed_tool_failure(result) {
                        line.push_str(&format!(
                            " [typed code={code}; error={error}; other_payload=redacted]"
                        ));
                    }
                }
                tool_lines.push(line);
            }
            AiEvent::AskHumanRequest { .. } => askhuman += 1,
            AiEvent::AskHumanResponse { skipped, .. } => {
                if *skipped {
                    askhuman_declined += 1;
                } else {
                    askhuman_approved += 1;
                }
            }
            AiEvent::Error { message, .. } => errors.push(format!("  {message}")),
            _ => {}
        }
    }

    out.push_str("\n-- gate decisions --\n");
    out.push_str(&if gate_lines.is_empty() {
        "  (none recorded)\n".to_string()
    } else {
        format!("{}\n", gate_lines.join("\n"))
    });

    out.push_str("\n-- tools invoked --\n");
    out.push_str(&if tool_lines.is_empty() {
        "  (none)\n".to_string()
    } else {
        format!("{}\n", tool_lines.join("\n"))
    });

    out.push_str("\n-- evidence booked --\n");
    out.push_str(&if evidence_lines.is_empty() {
        "  (none)\n".to_string()
    } else {
        format!("{}\n", evidence_lines.join("\n"))
    });

    if askhuman > 0 {
        out.push_str(&format!(
            "\n-- HITL --\n  {askhuman} ask_human request(s); typed policy responses: approved={askhuman_approved}, declined={askhuman_declined}\n"
        ));
    }
    if !errors.is_empty() {
        out.push_str(&format!("\n-- errors --\n{}\n", errors.join("\n")));
    }

    out.push_str("\n-- result --\n");
    match result {
        Ok(r) => {
            let preview: String = r.chars().take(800).collect();
            out.push_str(&format!("  OK ({} chars)\n{preview}\n", r.len()));
        }
        Err(e) => out.push_str(&format!("  FAILED: {e:#}\n")),
    }

    out.push_str(&format!(
        "\nfull transcript: {}/{}\nreplay:          golish --replay {session_id}\n",
        transcripts_dir.display(),
        session_id
    ));
    out.push_str("══════════════════════════════════════════════\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    const SESSION_ID: uuid::Uuid = uuid::Uuid::from_u128(0xa15c0b0f23ff42f9b9507dcaf25de860);
    const TASK_ID: uuid::Uuid = uuid::Uuid::from_u128(0x462b6c9f2a0d48af8ff08b5c08416196);
    const ORG_ID: uuid::Uuid = uuid::Uuid::from_u128(0x0a431390772648e5b0a8e692a9070e33);
    const CHAIN_ID: uuid::Uuid = uuid::Uuid::from_u128(0x552240a76050460b876bbd51a4ccba5f);

    fn complete_expectations() -> ResumeExpectations {
        ResumeExpectations {
            allow_orphan_running: true,
            repair_missing_graph_flow: true,
            repair_reaped_task: false,
            session_id: Some(SESSION_ID),
            task_id: Some(TASK_ID),
            operation_id: Some(TASK_ID),
            organization_id: Some(ORG_ID),
            stage: Some(StageKind::Enumeration),
        }
    }

    #[test]
    fn headless_typed_approval_policy_uses_only_explicit_cli_authority() {
        let rows = trusted_scope_review_response(&["portal.gzyouchuang.test".to_string()])
            .expect("CLI target becomes a trusted review row");
        let root_only = StageRunAutoApprovalPolicy {
            trusted_scope_response: Some(rows.clone()),
            retained_scope_authority: None,
            include_subsidiaries: Some(false),
            confirmed_organization_id: Some(ORG_ID),
            confirmed_organization_name: Some("Golish Fixture Corporation".to_string()),
            approve_phase_boundaries: true,
        };
        assert_eq!(
            root_only.resolve("scope_review", "confirm targets", &[], ""),
            StageRunAutoResolution::Approve(rows)
        );
        assert_eq!(
            root_only.resolve("confirmation", "continue?", &[], ""),
            StageRunAutoResolution::Approve(
                "approved by explicit CLI --approve-phase-boundaries".to_string()
            )
        );
        let options = vec![
            "不纳入子公司（仅母公司）".to_string(),
            "纳入：≥51% 控股子公司".to_string(),
        ];
        let context = format!(r#"{{"decision":"subsidiary_scope","organization_id":"{ORG_ID}"}}"#);
        assert_eq!(
            root_only.resolve("choice", "是否纳入子公司？", &options, &context),
            StageRunAutoResolution::Approve(options[0].clone())
        );
        let typed_options = vec![
            "root_only — subsidiaries/branches are OUT of scope (default, no subsidiary discovery)"
                .to_string(),
            "included — subsidiaries/branches ARE in scope (triggers evidence-backed subsidiary discovery + unit review)"
                .to_string(),
        ];
        assert_eq!(
            root_only.resolve("choice", "subsidiary scope", &typed_options, &context),
            StageRunAutoResolution::Approve(typed_options[0].clone())
        );
        assert!(matches!(
            root_only.resolve(
                "choice",
                "是否纳入子公司？",
                &options,
                &format!(
                    r#"{{"decision":"subsidiary_scope","organization_id":"{}"}}"#,
                    uuid::Uuid::new_v4()
                ),
            ),
            StageRunAutoResolution::Decline(_)
        ));
        assert!(matches!(
            root_only.resolve("choice", "ordinary choice", &options, ""),
            StageRunAutoResolution::Decline(_)
        ));
        let natural_options = vec![
            "Root-only: only Golish Fixture Corporation is in scope".to_string(),
            "Include subsidiaries/controlled holdings/branches".to_string(),
        ];
        assert_eq!(
            root_only.resolve(
                "choice",
                "Confirm subsidiary scope for Golish Fixture Corporation",
                &natural_options,
                "Confirmed enterprise: Golish Fixture Corporation",
            ),
            StageRunAutoResolution::Approve(natural_options[0].clone()),
            "fresh CLI may compile an exact-root natural-language request from trusted flags"
        );
        assert!(matches!(
            root_only.resolve(
                "choice",
                "Confirm subsidiary scope for Another Corporation",
                &natural_options,
                "Confirmed enterprise: Another Corporation",
            ),
            StageRunAutoResolution::Decline(_)
        ));
        let double_encoded_context = serde_json::to_string(&context).expect("encode context");
        assert!(matches!(
            root_only.resolve(
                "choice",
                "是否纳入子公司？",
                &options,
                &double_encoded_context,
            ),
            StageRunAutoResolution::Decline(_)
        ));
        assert!(matches!(
            root_only.resolve(
                "choice",
                "是否纳入子公司？",
                &["仅根组织".to_string(), "纳入控股单位".to_string()],
                &context,
            ),
            StageRunAutoResolution::Decline(_)
        ));

        let include = StageRunAutoApprovalPolicy {
            trusted_scope_response: None,
            retained_scope_authority: None,
            include_subsidiaries: Some(true),
            confirmed_organization_id: Some(ORG_ID),
            confirmed_organization_name: Some("Golish Fixture Corporation".to_string()),
            approve_phase_boundaries: false,
        };
        assert!(matches!(
            include.resolve("confirmation", "continue?", &[], ""),
            StageRunAutoResolution::Decline(_)
        ));
        assert_eq!(
            include.resolve("choice", "是否纳入子公司？", &options, &context),
            StageRunAutoResolution::Approve(options[1].clone())
        );
        assert_eq!(
            include.resolve("choice", "subsidiary scope", &typed_options, &context),
            StageRunAutoResolution::Approve(typed_options[1].clone())
        );
        assert!(matches!(
            include.resolve("scope_review", "confirm targets", &[], ""),
            StageRunAutoResolution::Decline(_)
        ));
        assert!(matches!(
            include.resolve("unit_review", "review units", &[], ""),
            StageRunAutoResolution::Decline(_)
        ));
        assert!(matches!(
            include.resolve("credentials", "login", &[], ""),
            StageRunAutoResolution::Decline(_)
        ));
        assert!(matches!(
            include.resolve("unexpected", "?", &[], ""),
            StageRunAutoResolution::Decline(_)
        ));
    }

    fn valid_resume_candidate() -> ResumeCandidate {
        let topology =
            golish_core::StageTopologyContract::LegacyCandidateVerificationV1.freeze_material();
        ResumeCandidate {
            session_id: SESSION_ID,
            chat_session_key: Some("stage-run-476558c3-c22a-4009-a82e-17e086a005de".to_string()),
            provider: Some("deepseek".to_string()),
            model: Some("deepseek-v4-flash".to_string()),
            task_id: TASK_ID,
            task_session_id: SESSION_ID,
            task_status: golish_db::models::TaskStatus::Waiting,
            task_result: None,
            task_updated_at: chrono::DateTime::parse_from_rfc3339("2026-07-11T03:11:37Z")
                .expect("fixture time")
                .with_timezone(&chrono::Utc),
            operation_id: TASK_ID,
            profile: "pentest".to_string(),
            current_stage: "enumeration".to_string(),
            runtime_memory_contract: "legacy_v1".to_string(),
            investigation_rollout_mode: "legacy_only".to_string(),
            stage_topology_contract: topology.topology.as_str().to_string(),
            stage_topology_canonical_json: topology.canonical_json,
            stage_topology_sha256: topology.sha256,
            stage_topology_freeze_source: "legacy_backfill_v1".to_string(),
            engagement_org_id: Some(ORG_ID),
            superseded_by: None,
            state_blob: serde_json::json!({
                "profile": "pentest",
                "current_stage": "enumeration",
                "current_stage_run_id": "364c72e7-4ded-4ef0-901a-44dd02f7752a",
                "queue_titles": [],
                "completed_count": 0,
                "schema_v": 1,
                "route_probe_checkpoints": {"origin": {"pending": ["/admin"]}},
                "stage_run_workers": {
                    "enumeration": {
                        ORG_ID.to_string(): {
                            "chain_id": CHAIN_ID.to_string(),
                            "specialist": "enumerator"
                        }
                    }
                },
                "graph_flow": {
                    "state": golish_agent_kit::harness::operation_flow::OperationFlowState::default(),
                    "next_node": "enumeration"
                }
            }),
            worker_chains: vec![ResumeWorkerOwnership {
                chain_id: CHAIN_ID,
                specialist: "enumerator".to_string(),
                stored_session_id: Some(SESSION_ID),
                stored_task_id: None,
                stored_agent: Some("pentester".to_string()),
                has_persisted_chain: true,
            }],
            relational_v2: None,
            expectations: ResumeExpectations::default(),
        }
    }

    fn valid_terminal_resume_candidate() -> TerminalResumeCandidate {
        let candidate = valid_resume_candidate();
        TerminalResumeCandidate {
            session_id: candidate.session_id,
            chat_session_key: candidate.chat_session_key,
            task_id: candidate.task_id,
            task_session_id: candidate.task_session_id,
            task_status: golish_db::models::TaskStatus::Finished,
            task_result: Some("canonical evidence summary".to_string()),
            operation_id: candidate.operation_id,
            profile: candidate.profile,
            current_stage: candidate.current_stage,
            runtime_memory_contract: candidate.runtime_memory_contract,
            investigation_rollout_mode: candidate.investigation_rollout_mode,
            stage_topology_contract: candidate.stage_topology_contract,
            stage_topology_canonical_json: candidate.stage_topology_canonical_json,
            stage_topology_sha256: candidate.stage_topology_sha256,
            stage_topology_freeze_source: candidate.stage_topology_freeze_source,
            engagement_org_id: candidate.engagement_org_id,
            superseded_by: candidate.superseded_by,
            expectations: complete_expectations(),
        }
    }

    #[test]
    fn terminal_resume_replays_the_durable_result_without_runtime_authority() {
        let replay = validate_terminal_resume_candidate(&valid_terminal_resume_candidate())
            .expect("terminal replay validates")
            .expect("finished task selects terminal replay");

        assert_eq!(replay.operation_id, TASK_ID);
        assert_eq!(replay.organization_id, ORG_ID);
        assert_eq!(replay.stage, StageKind::Enumeration);
        assert_eq!(replay.result, "canonical evidence summary");
    }

    #[test]
    fn terminal_resume_requires_complete_exact_identity_and_a_durable_result() {
        let mut missing_identity = valid_terminal_resume_candidate();
        missing_identity.expectations.organization_id = None;
        assert!(validate_terminal_resume_candidate(&missing_identity).is_err());

        let mut drifted_identity = valid_terminal_resume_candidate();
        drifted_identity.expectations.organization_id = Some(uuid::Uuid::new_v4());
        assert!(validate_terminal_resume_candidate(&drifted_identity).is_err());

        let mut missing_result = valid_terminal_resume_candidate();
        missing_result.task_result = None;
        assert!(validate_terminal_resume_candidate(&missing_result).is_err());

        let mut waiting = valid_terminal_resume_candidate();
        waiting.task_status = golish_db::models::TaskStatus::Waiting;
        assert!(validate_terminal_resume_candidate(&waiting)
            .expect("nonterminal is not a terminal replay error")
            .is_none());
    }

    #[test]
    fn resume_candidate_accepts_waiting_exact_operation() {
        let validated = validate_resume_candidate(&valid_resume_candidate())
            .expect("valid waiting exact resume");

        assert_eq!(validated.operation_id, TASK_ID);
        assert_eq!(validated.session_id, SESSION_ID);
        assert_eq!(validated.organization_id, ORG_ID);
        assert_eq!(validated.stage, StageKind::Enumeration);
        assert!(!validated.needs_graph_repair);
    }

    #[test]
    fn v2_only_resume_uses_relational_authority_with_server_only_state_blob() {
        let mut candidate = valid_resume_candidate();
        candidate.runtime_memory_contract = "v2_only".to_string();
        candidate.state_blob = serde_json::json!({
            "eas_web_transport_failures": {"server-slot": {"attempts": 2}}
        });
        candidate.worker_chains.clear();
        candidate.relational_v2 = Some(runtime_v2::RuntimeV2ResumeAuthority {
            active_stage_execution_id: uuid::Uuid::new_v4(),
            organization_id: ORG_ID,
        });

        let validated = validate_resume_candidate(&candidate)
            .expect("V2-only resume selects complete relational authority");
        assert_eq!(validated.stage, StageKind::Enumeration);
        assert!(!validated.needs_graph_repair);
    }

    #[test]
    fn scoping_pre_freeze_resume_requires_the_exact_expected_root_authority() {
        let mut candidate = valid_resume_candidate();
        candidate.current_stage = "scoping".to_string();
        candidate.runtime_memory_contract = "v2_only".to_string();
        candidate.engagement_org_id = None;
        candidate.worker_chains.clear();
        candidate.relational_v2 = Some(runtime_v2::RuntimeV2ResumeAuthority {
            active_stage_execution_id: uuid::Uuid::new_v4(),
            organization_id: ORG_ID,
        });
        candidate.expectations.organization_id = Some(ORG_ID);
        candidate.expectations.stage = Some(StageKind::Scoping);

        let validated = validate_resume_candidate(&candidate)
            .expect("exact expected root may resume the witnessed pre-freeze Scoping state");
        assert_eq!(validated.stage, StageKind::Scoping);
        assert_eq!(validated.organization_id, ORG_ID);

        candidate.expectations.organization_id = Some(uuid::Uuid::new_v4());
        assert!(validate_resume_candidate(&candidate).is_err());

        candidate.expectations.organization_id = None;
        assert!(validate_resume_candidate(&candidate).is_err());
    }

    #[test]
    fn v2_preferred_selects_whole_relational_authority_before_legacy_blob() {
        let mut candidate = valid_resume_candidate();
        candidate.runtime_memory_contract = "dual_write_v2_preferred".to_string();
        candidate.state_blob = serde_json::json!({"graph_flow": {"malformed": true}});
        candidate.worker_chains.clear();
        candidate.relational_v2 = Some(runtime_v2::RuntimeV2ResumeAuthority {
            active_stage_execution_id: uuid::Uuid::new_v4(),
            organization_id: ORG_ID,
        });

        validate_resume_candidate(&candidate)
            .expect("complete relational V2 must be selected without reading legacy fields");

        candidate.relational_v2 = None;
        let error = validate_resume_candidate(&candidate)
            .expect_err("preferred fallback must validate the complete legacy checkpoint");
        assert!(error.to_string().contains("stage_run worker map"));
    }

    #[test]
    fn legacy_resume_compares_specialist_chain_to_persisted_agent_class() {
        let mut candidate = valid_resume_candidate();
        candidate.worker_chains[0].stored_agent = Some("pentester".to_string());

        validate_resume_candidate(&candidate).expect(
            "the enumerator specialist is durably persisted as the coarser pentester agent class",
        );
    }

    #[test]
    fn v2_preferred_does_not_fallback_while_relational_worker_has_live_lease() {
        let error = anyhow::Error::new(runtime_v2::RelationalResumeBusy);
        assert!(
            !runtime_v2::relational_resume_error_allows_legacy_fallback(&error),
            "a complete relational source owned by a live worker must fail closed",
        );
    }

    #[test]
    fn selected_resume_authority_maps_to_one_complete_runtime_record() {
        use golish_agent_kit::db_traits::RuntimeMemoryRecordSource as Source;
        use golish_agent_kit::runtime_memory::RuntimeMemoryContract as Contract;

        assert_eq!(
            selected_resume_record_source(
                ResumeAuthorityKind::RelationalV2,
                Contract::DualWriteV2Preferred,
            )
            .expect("preferred V2 authority"),
            Source::V2
        );
        assert_eq!(
            selected_resume_record_source(
                ResumeAuthorityKind::LegacyCheckpoint,
                Contract::DualWriteV2Preferred,
            )
            .expect("preferred legacy fallback"),
            Source::LegacyFallback
        );
        assert!(selected_resume_record_source(
            ResumeAuthorityKind::LegacyCheckpoint,
            Contract::V2Only,
        )
        .is_err());
    }

    #[test]
    fn resume_candidate_accepts_running_task_via_open_turn_claim() {
        let mut candidate = valid_resume_candidate();
        candidate.task_status = golish_db::models::TaskStatus::Running;

        let validated = validate_resume_candidate(&candidate)
            .expect("a running task is fenced by the shared source and open-Turn claim");
        assert!(!validated.needs_task_repair);
    }

    #[test]
    fn resume_session_selection_includes_running_without_orphan_assertion() {
        use golish_db::models::TaskStatus;

        assert!(resume_task_status_is_selectable(TaskStatus::Waiting, false));
        assert!(resume_task_status_is_selectable(TaskStatus::Running, false));
        assert!(resume_task_status_is_selectable(TaskStatus::Running, true));
        assert!(!resume_task_status_is_selectable(TaskStatus::Created, true));
        assert!(!resume_task_status_is_selectable(
            TaskStatus::Finished,
            true
        ));
        assert!(!resume_task_status_is_selectable(TaskStatus::Failed, true));
    }

    #[test]
    fn resume_candidate_rejects_malformed_graph_flow_state() {
        let mut candidate = valid_resume_candidate();
        candidate.state_blob["graph_flow"]["state"] = serde_json::json!({});

        let error = validate_resume_candidate(&candidate)
            .expect_err("malformed graph state must fail before resume");
        assert!(error.to_string().contains("graph_flow state is malformed"));
    }

    #[test]
    fn running_resume_flag_does_not_downgrade_the_durable_turn() {
        let mut candidate = valid_resume_candidate();
        candidate.task_status = golish_db::models::TaskStatus::Running;
        candidate.expectations = complete_expectations();
        candidate.expectations.stage = None;

        let validated = validate_resume_candidate(&candidate)
            .expect("running resume uses its open Turn instead of a waiting downgrade");
        assert!(!validated.needs_task_repair);

        candidate.expectations.stage = Some(StageKind::Enumeration);
        validate_resume_candidate(&candidate)
            .expect("legacy exact identity assertions remain compatible");

        candidate.task_result = Some("unexpected partial result".to_string());
        let error = validate_resume_candidate(&candidate)
            .expect_err("running operation with a result must fail closed");
        assert!(error.to_string().contains("non-null task result"));
    }

    #[test]
    fn resume_candidate_repairs_only_exact_startup_reaper_failure() {
        let mut candidate = valid_resume_candidate();
        candidate.task_status = golish_db::models::TaskStatus::Failed;
        candidate.task_result = Some(golish_db::repo::tasks::ABANDONED_TASK_RESULT.to_string());
        candidate.expectations = complete_expectations();

        let error = validate_resume_candidate(&candidate)
            .expect_err("failed task needs an explicit repair flag");
        assert!(error.to_string().contains("--repair-reaped-task"));

        candidate.expectations.repair_reaped_task = true;
        let validated = validate_resume_candidate(&candidate)
            .expect("the exact startup-reaped task may be repaired");
        assert!(validated.needs_task_repair);

        candidate.task_result = Some("ordinary provider failure".to_string());
        let error = validate_resume_candidate(&candidate)
            .expect_err("ordinary failed tasks must remain terminal");
        assert!(error
            .to_string()
            .contains("startup-reaper abandoned marker"));
    }

    #[test]
    fn resume_candidate_rejects_session_task_operation_or_org_drift() {
        let mut candidate = valid_resume_candidate();
        candidate.expectations = complete_expectations();
        candidate.task_session_id = uuid::Uuid::new_v4();
        assert!(validate_resume_candidate(&candidate).is_err());

        let mut candidate = valid_resume_candidate();
        candidate.expectations = complete_expectations();
        candidate.operation_id = uuid::Uuid::new_v4();
        assert!(validate_resume_candidate(&candidate).is_err());

        let mut candidate = valid_resume_candidate();
        candidate.expectations = complete_expectations();
        candidate.engagement_org_id = Some(uuid::Uuid::new_v4());
        assert!(validate_resume_candidate(&candidate).is_err());

        let mut candidate = valid_resume_candidate();
        candidate.expectations = complete_expectations();
        candidate.chat_session_key = Some("normal-chat".to_string());
        assert!(validate_resume_candidate(&candidate).is_err());
    }

    #[test]
    fn resume_candidate_accepts_exact_gui_task_session() {
        let mut candidate = valid_resume_candidate();
        candidate.chat_session_key = Some("pentest-chat-1784179823492-1".to_string());

        let target = validate_resume_candidate(&candidate)
            .expect("an exact GUI Task session uses the same durable operation resume contract");

        assert_eq!(target.operation_id, candidate.operation_id);
        assert_eq!(target.session_id, candidate.session_id);
        assert_eq!(target.chat_session_key, "pentest-chat-1784179823492-1");
    }

    #[test]
    fn resume_candidate_rejects_superseded_or_cross_scope_worker_chain() {
        let mut candidate = valid_resume_candidate();
        candidate.superseded_by = Some(uuid::Uuid::new_v4());
        assert!(validate_resume_candidate(&candidate).is_err());

        let mut candidate = valid_resume_candidate();
        candidate.worker_chains[0].stored_session_id = Some(uuid::Uuid::new_v4());
        assert!(validate_resume_candidate(&candidate).is_err());

        let mut candidate = valid_resume_candidate();
        candidate.worker_chains[0].stored_task_id = Some(uuid::Uuid::new_v4());
        assert!(validate_resume_candidate(&candidate).is_err());

        let mut candidate = valid_resume_candidate();
        candidate.worker_chains[0].stored_agent = Some("browser".to_string());
        assert!(validate_resume_candidate(&candidate).is_err());
    }

    #[test]
    fn resume_candidate_missing_graph_requires_explicit_repair_and_flat_checkpoint() {
        let mut candidate = valid_resume_candidate();
        candidate
            .state_blob
            .as_object_mut()
            .expect("state object")
            .remove("graph_flow");

        assert!(validate_resume_candidate(&candidate).is_err());

        candidate.expectations = complete_expectations();
        let validated = validate_resume_candidate(&candidate)
            .expect("fully asserted flat checkpoint may be repaired");
        assert!(validated.needs_graph_repair);

        candidate.state_blob["profile"] = serde_json::json!("assessment");
        assert!(validate_resume_candidate(&candidate).is_err());

        candidate.state_blob["profile"] = serde_json::json!("pentest");
        candidate.state_blob["completed_count"] = serde_json::json!(1);
        let error = validate_resume_candidate(&candidate)
            .expect_err("later checkpoint without graph_flow must not be synthesized");
        assert!(error.to_string().contains("first graph node"));
    }

    #[test]
    fn resume_checkpoint_synthesis_preserves_every_sibling_key() {
        let candidate = valid_resume_candidate();
        let mut flat = candidate.state_blob.clone();
        flat.as_object_mut()
            .expect("state object")
            .remove("graph_flow");
        flat["unknown_future_key"] = serde_json::json!({"keep": true});

        let repaired = synthesize_graph_flow_checkpoint(flat.clone(), StageKind::Enumeration)
            .expect("synthesize graph checkpoint");

        for key in [
            "profile",
            "current_stage",
            "current_stage_run_id",
            "queue_titles",
            "completed_count",
            "schema_v",
            "route_probe_checkpoints",
            "stage_run_workers",
            "unknown_future_key",
        ] {
            assert_eq!(repaired.get(key), flat.get(key), "sibling {key} drifted");
        }
        assert_eq!(
            repaired["graph_flow"]["next_node"],
            serde_json::json!("enumeration")
        );
        assert!(repaired["graph_flow"]["state"].is_object());
    }

    #[test]
    fn resume_selector_and_sql_are_exact_scoped() {
        assert_eq!(
            classify_resume_selector("stage-run-abc"),
            ResumeSelector::ChatKey("stage-run-abc".to_string())
        );
        assert_eq!(
            classify_resume_selector(&TASK_ID.to_string()),
            ResumeSelector::Uuid(TASK_ID)
        );
        assert!(EXACT_RESUME_CHAIN_SQL.contains("session_id = $2"));
        assert!(EXACT_RESUME_CHAIN_SQL.contains("task_id IS NULL OR task_id = $3"));
        assert!(EXACT_RESUME_CHAIN_SQL.contains("agent = $4::agent_type"));
        assert!(REPAIR_GRAPH_FLOW_SQL.contains("jsonb_set"));
        assert!(REPAIR_GRAPH_FLOW_SQL.contains("state_blob -> 'graph_flow' IS NULL"));
        assert!(REPAIR_GRAPH_FLOW_SQL.contains("superseded_by IS NULL"));
        assert!(REPAIR_GRAPH_FLOW_SQL.contains("runtime_memory_contract IN"));
        assert!(REPAIR_GRAPH_FLOW_SQL.contains("'dual_write_v2_preferred'"));
        assert!(!REPAIR_GRAPH_FLOW_SQL.contains("'v2_only'"));
        assert!(REPAIR_REAPED_TASK_SQL.contains("status = 'failed'"));
        assert!(REPAIR_REAPED_TASK_SQL.contains("result = $3"));
        assert!(REPAIR_REAPED_TASK_SQL.contains("updated_at = $4"));
        assert!(REPAIR_REAPED_TASK_SQL.contains("os.state_blob = $8"));
    }

    #[test]
    fn durable_resume_claim_runs_immediately_before_orchestrator_resume() {
        let source = include_str!("mod.rs");
        let body = source
            .split_once("async fn orchestrate_resume(")
            .expect("orchestrate_resume definition")
            .1;
        let claim = body
            .find("claim_exact_resume_runtime_source(")
            .expect("shared durable source and Turn claim");
        let resume = body
            .find(".resume(target.operation_id, continuation, prepared.executor())")
            .expect("orchestrator resume call");
        assert!(claim < resume, "durable task claim must precede resume");
    }

    #[test]
    fn stage_run_resume_uses_shared_operation_turn_claim() {
        let source = include_str!("mod.rs");
        let body = source
            .split_once("async fn orchestrate_resume(")
            .expect("orchestrate_resume definition")
            .1;
        let select = body
            .find("select_exact_resume_runtime_source(")
            .expect("shared exact runtime source and open Turn selection");
        let claim = body
            .find("claim_exact_resume_runtime_source(")
            .expect("shared exact operation Turn claim");
        let resume = body
            .find(".resume(target.operation_id, continuation, prepared.executor())")
            .expect("orchestrator resume call");

        assert!(select < claim && claim < resume);
        assert!(
            !body[..resume].contains("claim_exact_resume_task(claim, target)"),
            "CLI must not keep a second waiting-to-running resume protocol"
        );
    }

    #[test]
    fn explicit_repairs_precede_resume_reresolution() {
        let source = include_str!("mod.rs");
        let body = source
            .split_once("async fn run_resume(")
            .expect("run_resume definition")
            .1
            .split_once("async fn orchestrate(")
            .expect("run_resume body")
            .0;
        let reaped = body
            .find("repair_reaped_task(&mut claim, &initial).await?")
            .expect("startup-reaped task repair");
        let graph = body
            .find("repair_missing_graph_flow(&mut claim, &initial).await?")
            .expect("graph repair");
        let rerun = body
            .rfind("resolve_stage_run_resume_target(&db_pool, &selector, &expectations).await?")
            .expect("post-repair target resolution");
        assert!(reaped < graph && graph < rerun);
    }

    #[test]
    fn resume_advisory_key_is_stable_and_operation_specific() {
        let keys = resume_advisory_lock_keys(TASK_ID);
        assert_eq!(keys, resume_advisory_lock_keys(TASK_ID));
        assert_ne!(keys, resume_advisory_lock_keys(uuid::Uuid::new_v4()));
    }

    #[test]
    fn exact_resume_target_authority_is_strict_and_missing_marker_fails_closed() {
        let company_only = golish_agent_kit::task_orchestrator::harness_resume::state_blob_with_current_invocation_target_authority(
            serde_json::json!({"profile": "red_team", "current_stage": "target_intel"}),
            false,
        );
        assert_eq!(
            persisted_resume_target_authority(&company_only, false)
                .expect("valid persisted company-only marker"),
            Some(false)
        );

        let exact_target = golish_agent_kit::task_orchestrator::harness_resume::state_blob_with_current_invocation_target_authority(
            serde_json::json!({"profile": "red_team", "current_stage": "target_intel"}),
            true,
        );
        assert_eq!(
            persisted_resume_target_authority(&exact_target, false)
                .expect("valid persisted exact-target marker"),
            Some(true)
        );
        assert_eq!(
            persisted_resume_target_authority(
                &serde_json::json!({"profile": "red_team", "current_stage": "scoping"}),
                false,
            )
            .expect("old headless operation without a marker must fail closed"),
            Some(false)
        );

        let malformed = serde_json::json!({
            "fresh_launch_authority": {
                "schema_v": 1,
                "current_invocation_target_authority": "false"
            }
        });
        assert!(persisted_resume_target_authority(&malformed, false).is_err());
    }

    #[test]
    fn exact_resume_uses_immutable_stage_fork_targets_when_marker_is_missing() {
        let missing_marker =
            serde_json::json!({"profile": "red_team", "current_stage": "external_attack_surface"});

        assert_eq!(
            persisted_resume_target_authority(&missing_marker, true)
                .expect("immutable stage-fork targets are trusted launch authority"),
            Some(true)
        );
    }

    #[test]
    fn immutable_fork_resume_preserves_out_of_scope_rejection_targets() {
        use golish_db::repo::operation_stage_forks::{
            OperationStageForkRow, OperationStageForkTargetRow,
        };

        let operation_id = uuid::Uuid::new_v4();
        let scope_snapshot_id = uuid::Uuid::new_v4();
        let organization_id = uuid::Uuid::new_v4();
        let in_target_id = uuid::Uuid::new_v4();
        let out_target_id = uuid::Uuid::new_v4();
        let now = chrono::Utc::now();
        let targets = vec![
            OperationStageForkTargetRow {
                id: uuid::Uuid::new_v4(),
                operation_id,
                scope_snapshot_id,
                organization_id,
                ordinal: 0,
                live_target_id: in_target_id,
                target_name_at_fork: "allowed.example".to_string(),
                target_type_at_fork: "domain".to_string(),
                target_value_at_fork: "allowed.example".to_string(),
                target_scope_at_fork: "in".to_string(),
                target_source_at_fork: "customer_provided".to_string(),
                project_path_at_fork: "/tmp/project".to_string(),
                canonical_identity_sha256: "sha256:in".to_string(),
                schema_version: 1,
                frozen_at: now,
            },
            OperationStageForkTargetRow {
                id: uuid::Uuid::new_v4(),
                operation_id,
                scope_snapshot_id,
                organization_id,
                ordinal: 1,
                live_target_id: out_target_id,
                target_name_at_fork: "denied.example".to_string(),
                target_type_at_fork: "domain".to_string(),
                target_value_at_fork: "denied.example".to_string(),
                target_scope_at_fork: "out".to_string(),
                target_source_at_fork: "target_intel_goal".to_string(),
                project_path_at_fork: "/tmp/project".to_string(),
                canonical_identity_sha256: "sha256:out".to_string(),
                schema_version: 1,
                frozen_at: now,
            },
        ];
        let manifest_targets = targets
            .iter()
            .map(|row| {
                serde_json::json!({
                    "id": row.id,
                    "ordinal": row.ordinal,
                    "live_target_id": row.live_target_id,
                    "organization_id": row.organization_id,
                    "canonical_identity_sha256": row.canonical_identity_sha256,
                })
            })
            .collect::<Vec<_>>();
        let fork = OperationStageForkRow {
            operation_id,
            source_operation_id: uuid::Uuid::new_v4(),
            project_scope_id: uuid::Uuid::new_v4(),
            source_scope_snapshot_id: uuid::Uuid::new_v4(),
            target_scope_snapshot_id: scope_snapshot_id,
            source_profile: "red_team".to_string(),
            target_profile: "red_team".to_string(),
            source_runtime_memory_contract: "v2_only".to_string(),
            target_runtime_memory_contract: "v2_only".to_string(),
            source_attack_execution_contract: "v2_only".to_string(),
            target_attack_execution_contract: "v2_only".to_string(),
            source_stage_topology_contract: "unified_investigation_v1".to_string(),
            target_stage_topology_contract: "unified_investigation_v1".to_string(),
            entry_stage: "external_attack_surface".to_string(),
            terminal_stage: "investigation".to_string(),
            adopted_stage_kinds: vec!["scoping".to_string(), "target_intel".to_string()],
            expected_input_count: 2,
            expected_target_count: 2,
            manifest: serde_json::json!({"targets": manifest_targets}),
            manifest_sha256: "sha256:manifest".to_string(),
            schema_version: 2,
            created_at: now,
        };

        assert!(immutable_stage_fork_target_manifest_is_complete(
            &fork, &targets
        ));

        let mut foreign = targets.clone();
        foreign[1].canonical_identity_sha256 = "sha256:drift".to_string();
        assert!(!immutable_stage_fork_target_manifest_is_complete(
            &fork, &foreign
        ));
    }

    #[test]
    fn resolve_slice_only_single_stage() {
        let (entry, allowlist) = resolve_slice(
            "assessment",
            Some(StageKind::TargetIntel),
            StageKind::TargetIntel,
        )
        .expect("target_intel is in the assessment profile");
        assert_eq!(entry, StageKind::TargetIntel);
        assert_eq!(allowlist, HashSet::from([StageKind::TargetIntel]));
    }

    #[test]
    fn resolve_slice_to_target_intel_from_entry() {
        let (entry, allowlist) =
            resolve_slice("assessment", None, StageKind::TargetIntel).expect("reachable");
        assert_eq!(entry, StageKind::Scoping);
        assert_eq!(
            allowlist,
            HashSet::from([StageKind::Scoping, StageKind::TargetIntel])
        );
    }

    #[test]
    fn resolve_slice_unknown_profile_errs() {
        assert!(resolve_slice("does_not_exist", None, StageKind::Scoping).is_err());
    }

    #[test]
    fn resolve_slice_to_not_in_profile_errs() {
        // vuln_triage is forbidden in the assessment profile.
        assert!(resolve_slice("assessment", None, StageKind::VulnTriage).is_err());
    }

    #[test]
    fn resolve_resume_slice_defaults_to_current_stage() {
        let (terminal, allowlist) = resolve_resume_slice(
            "pentest",
            golish_core::StageTopologyContract::LegacyCandidateVerificationV1,
            StageKind::TargetIntel,
            None,
        )
        .expect("same stage");
        assert_eq!(terminal, StageKind::TargetIntel);
        assert_eq!(allowlist, HashSet::from([StageKind::TargetIntel]));
    }

    #[test]
    fn resolve_resume_slice_expands_only_forward_to_candidate() {
        let (terminal, allowlist) = resolve_resume_slice(
            "pentest",
            golish_core::StageTopologyContract::LegacyCandidateVerificationV1,
            StageKind::TargetIntel,
            Some("attack_candidate"),
        )
        .expect("forward pentest slice");
        assert_eq!(terminal, StageKind::AttackCandidate);
        assert_eq!(
            allowlist,
            HashSet::from([
                StageKind::TargetIntel,
                StageKind::ExternalAttackSurface,
                StageKind::Enumeration,
                StageKind::VulnTriage,
                StageKind::AttackCandidate,
            ])
        );
        assert!(resolve_resume_slice(
            "pentest",
            golish_core::StageTopologyContract::LegacyCandidateVerificationV1,
            StageKind::TargetIntel,
            Some("scoping")
        )
        .is_err());
        assert!(resolve_resume_slice(
            "assessment",
            golish_core::StageTopologyContract::LegacyCandidateVerificationV1,
            StageKind::TargetIntel,
            Some("attack_candidate")
        )
        .is_err());
    }

    #[test]
    fn resolve_from_to_only_sets_both() {
        let args = Args::parse_from(["golish", "--stage-run", "--only", "scoping"]);
        let (from, to) = resolve_from_to(&args).unwrap();
        assert_eq!(from, Some(StageKind::Scoping));
        assert_eq!(to, StageKind::Scoping);
    }

    #[test]
    fn resolve_from_to_requires_to_or_only() {
        let args = Args::parse_from(["golish", "--stage-run"]);
        assert!(resolve_from_to(&args).is_err());
    }

    #[test]
    fn resolve_stage_run_fork_only_candidate_adopts_exact_prefix() {
        let args = Args::parse_from([
            "golish",
            "--stage-run-fork",
            "425c7693-99fb-4598-8361-62275c9413b1",
            "--only",
            "attack_candidate",
        ]);
        let resolved = resolve_stage_run_fork_slice(
            "pentest",
            golish_core::StageTopologyContract::LegacyCandidateVerificationV1.freeze_material(),
            &args,
        )
        .expect("candidate fork");
        assert_eq!(resolved.entry_stage, StageKind::AttackCandidate);
        assert_eq!(resolved.terminal_stage, StageKind::AttackCandidate);
        assert_eq!(
            resolved.allowlist,
            HashSet::from([StageKind::AttackCandidate])
        );
        assert_eq!(
            resolved.adopted_stage_kinds,
            vec![
                StageKind::Scoping,
                StageKind::TargetIntel,
                StageKind::ExternalAttackSurface,
                StageKind::Enumeration,
                StageKind::VulnTriage,
            ]
        );
    }

    #[test]
    fn resolve_stage_run_fork_range_adopts_only_strict_prefix() {
        let args = Args::parse_from([
            "golish",
            "--stage-run-fork",
            "425c7693-99fb-4598-8361-62275c9413b1",
            "--from",
            "enumeration",
            "--to",
            "attack_candidate",
        ]);
        let resolved = resolve_stage_run_fork_slice(
            "pentest",
            golish_core::StageTopologyContract::LegacyCandidateVerificationV1.freeze_material(),
            &args,
        )
        .expect("range fork");
        assert_eq!(resolved.entry_stage, StageKind::Enumeration);
        assert_eq!(resolved.terminal_stage, StageKind::AttackCandidate);
        assert_eq!(
            resolved.allowlist,
            HashSet::from([
                StageKind::Enumeration,
                StageKind::VulnTriage,
                StageKind::AttackCandidate,
            ])
        );
        assert_eq!(
            resolved.adopted_stage_kinds,
            vec![
                StageKind::Scoping,
                StageKind::TargetIntel,
                StageKind::ExternalAttackSurface,
            ]
        );
    }

    #[test]
    fn stale_investigation_retry_from_eas_adopts_only_scoping_and_target_intel() {
        let args = Args::parse_from([
            "golish",
            "--stage-run-fork",
            "425c7693-99fb-4598-8361-62275c9413b1",
            "--from",
            "external_attack_surface",
            "--to",
            "investigation",
        ]);
        let resolved = resolve_stage_run_fork_slice(
            "pentest",
            golish_core::StageTopologyContract::UnifiedInvestigationV1.freeze_material(),
            &args,
        )
        .expect("stale Investigation prerequisites restart from EAS");
        assert_eq!(resolved.entry_stage, StageKind::ExternalAttackSurface);
        assert_eq!(resolved.terminal_stage, StageKind::Investigation);
        assert_eq!(
            resolved.adopted_stage_kinds,
            vec![StageKind::Scoping, StageKind::TargetIntel]
        );
        assert!(resolved
            .allowlist
            .contains(&StageKind::ExternalAttackSurface));
        assert!(resolved.allowlist.contains(&StageKind::Enumeration));
        assert!(resolved.allowlist.contains(&StageKind::VulnTriage));
        assert!(resolved
            .allowlist
            .contains(&StageKind::ApplicationUnderstanding));
        assert!(resolved.allowlist.contains(&StageKind::Investigation));
        assert!(!resolved.allowlist.contains(&StageKind::AttackCandidate));
    }

    #[test]
    fn resolve_stage_run_fork_rejects_scoping_and_incomplete_range() {
        for argv in [
            vec![
                "golish",
                "--stage-run-fork",
                "425c7693-99fb-4598-8361-62275c9413b1",
                "--only",
                "scoping",
            ],
            vec![
                "golish",
                "--stage-run-fork",
                "425c7693-99fb-4598-8361-62275c9413b1",
                "--from",
                "enumeration",
            ],
            vec![
                "golish",
                "--stage-run-fork",
                "425c7693-99fb-4598-8361-62275c9413b1",
                "--to",
                "attack_candidate",
            ],
        ] {
            let args = Args::parse_from(argv);
            assert!(resolve_stage_run_fork_slice(
                "pentest",
                golish_core::StageTopologyContract::LegacyCandidateVerificationV1.freeze_material(),
                &args,
            )
            .is_err());
        }
    }

    #[test]
    fn stage_run_fork_session_selector_requires_exactly_one_operation() {
        let session_id = uuid::Uuid::new_v4();
        let operation_id = uuid::Uuid::new_v4();
        assert_eq!(
            require_unique_stage_fork_operation(session_id, &[operation_id]).unwrap(),
            operation_id
        );
        assert!(require_unique_stage_fork_operation(session_id, &[]).is_err());
        assert!(require_unique_stage_fork_operation(
            session_id,
            &[operation_id, uuid::Uuid::new_v4()]
        )
        .is_err());
    }

    #[test]
    fn fresh_direct_active_slice_requires_targets_from_this_cli_invocation() {
        for stage in [
            StageKind::ExternalAttackSurface,
            StageKind::Enumeration,
            StageKind::VulnTriage,
            StageKind::AttackCandidate,
        ] {
            let error = validate_fresh_slice_target_intake(stage, &HashSet::from([stage]), &[])
                .expect_err("a reused organization must not lend stale targets to a fresh slice");
            assert!(format!("{error:#}").contains("--target"));
        }

        validate_fresh_slice_target_intake(
            StageKind::ExternalAttackSurface,
            &HashSet::from([StageKind::ExternalAttackSurface]),
            &["portal.gzyouchuang.test".to_string()],
        )
        .expect("an exact target supplied by this invocation is trusted intake");
    }

    #[test]
    fn fresh_company_only_full_flow_reaches_the_shared_pre_eas_barrier() {
        validate_fresh_slice_target_intake(
            StageKind::Scoping,
            &HashSet::from([
                StageKind::Scoping,
                StageKind::TargetIntel,
                StageKind::ExternalAttackSurface,
                StageKind::Enumeration,
                StageKind::VulnTriage,
                StageKind::AttackCandidate,
            ]),
            &[],
        )
        .expect("company-only full flow is legal until the shared pre-EAS barrier");
        validate_fresh_slice_target_intake(
            StageKind::TargetIntel,
            &HashSet::from([StageKind::TargetIntel]),
            &[],
        )
        .expect("passive target intel may start without an active target");
    }

    #[test]
    fn target_intel_slice_cannot_cross_into_active_recon_on_historical_targets() {
        let active_slice = HashSet::from([
            StageKind::TargetIntel,
            StageKind::ExternalAttackSurface,
            StageKind::Enumeration,
            StageKind::VulnTriage,
            StageKind::AttackCandidate,
        ]);
        let error = validate_fresh_slice_target_intake(StageKind::TargetIntel, &active_slice, &[])
            .expect_err(
                "a direct passive entry must not carry historical targets into active recon",
            );
        assert!(format!("{error:#}").contains("--target"));

        validate_fresh_slice_target_intake(
            StageKind::TargetIntel,
            &active_slice,
            &["http://127.0.0.1:18080".to_string()],
        )
        .expect("current-invocation exact target can authorize the active boundary");
    }

    #[test]
    fn scoping_explicit_company_is_confirmed_org_but_not_target_authority() {
        let company_only = Args::parse_from([
            "golish",
            "--stage-run",
            "--profile",
            "red_team",
            "--to",
            "attack_candidate",
            "--org",
            "广州有创网络科技有限公司",
        ]);
        assert!(should_seed_upstream(
            StageKind::Scoping,
            company_only.org.as_deref(),
            &company_only.target,
        ));

        let policy = crate::ai::task_operation::SubsidiaryScopePolicy::default();
        let scope = build_fresh_cli_scope(
            StageKind::Scoping,
            "Run Scoping for 广州有创网络科技有限公司",
            company_only.org.as_deref(),
            &company_only.target,
            Some(ORG_ID),
            None,
            &policy,
        )
        .expect("explicit CLI --org becomes confirmed organization intake");
        assert!(matches!(
            &scope,
            crate::ai::task_operation::FreshOperationScope::ConfirmedOrganizationIntake {
                subject_label,
                organization_id,
                runtime_scope: None,
            } if subject_label == "广州有创网络科技有限公司" && *organization_id == ORG_ID
        ));
        let launch = crate::ai::task_operation::FreshTaskOperationLaunch::new(
            "Run Scoping for 广州有创网络科技有限公司",
            "red_team",
            crate::ai::task_operation::FreshOperationEntry::StageSlice {
                entry_stage: StageKind::Scoping,
                allowlist: HashSet::from([
                    StageKind::Scoping,
                    StageKind::TargetIntel,
                    StageKind::ExternalAttackSurface,
                ]),
            },
            scope,
            policy,
            None,
        )
        .expect("confirmed organization launch remains target-empty");
        let authority = launch
            .normalized_authority_projection()
            .expect("project typed launch authority");
        assert_eq!(authority.organization_id, Some(ORG_ID));
        assert!(authority.current_invocation_targets.is_empty());

        let confirmed_target = Args::parse_from([
            "golish",
            "--stage-run",
            "--profile",
            "red_team",
            "--to",
            "attack_candidate",
            "--org",
            "广州有创网络科技有限公司",
            "--target",
            "http://127.0.0.1:18080",
        ]);
        assert!(should_seed_upstream(
            StageKind::Scoping,
            confirmed_target.org.as_deref(),
            &confirmed_target.target,
        ));
    }

    #[test]
    fn scoping_defers_runtime_scope_freeze_until_persisted_human_decision() {
        let runtime_scope = golish_agent_kit::db_traits::CliRuntimeScope {
            root_organization_id: ORG_ID,
            include_subsidiaries: false,
            subsidiary_threshold: 51,
            units: vec![golish_agent_kit::db_traits::CliRuntimeScopeUnit {
                organization_id: ORG_ID,
                parent_organization_id: None,
                organization_name: "广州有创网络科技有限公司".to_string(),
                depth: 0,
                ordinal: 0,
                ownership_percent: None,
                approval_source: serde_json::json!({"kind": "cli_flags"}),
            }],
        };

        assert!(
            cli_runtime_scope_for_entry(StageKind::Scoping, Some(runtime_scope.clone())).is_none()
        );
        assert_eq!(
            cli_runtime_scope_for_entry(StageKind::TargetIntel, Some(runtime_scope.clone())),
            Some(runtime_scope)
        );
    }

    #[test]
    fn direct_passive_slice_may_seed_an_explicit_company_label() {
        assert!(should_seed_upstream(
            StageKind::TargetIntel,
            Some("广州有创网络科技有限公司"),
            &[],
        ));
    }

    #[test]
    fn subsidiary_threshold_defaults_to_majority_control_boundary() {
        let args = Args::parse_from(["golish", "--stage-run", "--to", "scoping"]);
        assert_eq!(args.subsidiary_threshold, 51);
    }

    #[test]
    fn stage_run_db_defaults_to_app_database() {
        let args = Args::parse_from(["golish", "--stage-run", "--to", "scoping"]);
        let stage_db = prepare_stage_run_db(&args).expect("default db config");
        let default_config = golish_db::DbConfig::default();

        assert!(stage_db.temp_dir.is_none());
        assert_eq!(stage_db.config.pg_data_dir, default_config.pg_data_dir);
        assert_eq!(stage_db.config.port, default_config.port);
    }

    #[test]
    fn stage_run_db_ephemeral_uses_temp_pgdata() {
        let args = Args::parse_from([
            "golish",
            "--stage-run",
            "--ephemeral-db",
            "--keep-ephemeral-db",
            "--db-smoke-summary",
            "--to",
            "scoping",
        ]);
        assert!(args.ephemeral_db);
        assert!(args.keep_ephemeral_db);
        assert!(args.db_smoke_summary);

        let stage_db = prepare_stage_run_db(&args).expect("ephemeral db config");
        let default_config = golish_db::DbConfig::default();
        let temp_root = stage_db
            .temp_dir
            .as_ref()
            .expect("temp dir")
            .path()
            .to_path_buf();

        assert_eq!(stage_db.config.pg_data_dir, temp_root.join("pgdata"));
        assert_eq!(
            stage_db.config.pg_bin_cache_dir,
            default_config.pg_bin_cache_dir
        );
        assert!(stage_db.config.port > 0);
    }

    #[test]
    fn stage_run_db_retained_resume_uses_exact_pgdata_and_fresh_port() {
        let retained = tempfile::tempdir().expect("retained pgdata root");
        let pgdata = retained.path().join("pgdata");
        std::fs::create_dir(&pgdata).expect("create retained pgdata");
        std::fs::write(pgdata.join("PG_VERSION"), "17\n").expect("write PG_VERSION marker");
        let args = Args::parse_from([
            "golish",
            "--stage-run-resume",
            "stage-run-476558c3-c22a-4009-a82e-17e086a005de",
            "--stage-run-resume-pgdata",
            pgdata.to_str().expect("utf8 pgdata"),
        ]);

        let stage_db = prepare_stage_run_db(&args).expect("retained resume db config");
        let default_config = golish_db::DbConfig::default();
        assert!(stage_db.temp_dir.is_none());
        assert_eq!(stage_db.config.pg_data_dir, pgdata);
        assert_eq!(
            stage_db.config.pg_bin_cache_dir,
            default_config.pg_bin_cache_dir
        );
        assert_ne!(stage_db.config.port, default_config.port);
        assert!(stage_db.config.port > 0);
    }

    #[test]
    fn unified_test_rollout_requires_ephemeral_stage_run_and_closed_rank() {
        let args = Args::try_parse_from([
            "golish",
            "--stage-run",
            "--ephemeral-db",
            "--stage-run-test-joint-rank",
            "6",
            "--to",
            "reporting",
        ])
        .expect("isolated unified test rollout parses");
        assert_eq!(args.stage_run_test_joint_rank, Some(6));

        assert!(Args::try_parse_from([
            "golish",
            "--stage-run",
            "--stage-run-test-joint-rank",
            "6",
            "--to",
            "reporting",
        ])
        .is_err());
        assert!(Args::try_parse_from([
            "golish",
            "--stage-run",
            "--ephemeral-db",
            "--stage-run-test-joint-rank",
            "4",
            "--to",
            "reporting",
        ])
        .is_err());
    }

    #[test]
    fn controlled_provider_directories_require_each_other_and_ephemeral_stage_run() {
        let args = Args::try_parse_from([
            "golish",
            "--stage-run",
            "--ephemeral-db",
            "--stage-run-test-toolsconfig-dir",
            "/tmp/controlled-toolsconfig",
            "--stage-run-test-intel-providers-dir",
            "/tmp/controlled-intel-providers",
            "--stage-run-test-intel-provider-endpoint",
            "http://127.0.0.1:32123/intel/company.json",
            "--to",
            "reporting",
        ])
        .expect("paired controlled provider directories parse");
        assert_eq!(
            args.stage_run_test_toolsconfig_dir.as_deref(),
            Some(std::path::Path::new("/tmp/controlled-toolsconfig"))
        );
        assert_eq!(
            args.stage_run_test_intel_providers_dir.as_deref(),
            Some(std::path::Path::new("/tmp/controlled-intel-providers"))
        );
        assert_eq!(
            args.stage_run_test_intel_provider_endpoint
                .as_ref()
                .map(url::Url::as_str),
            Some("http://127.0.0.1:32123/intel/company.json")
        );

        assert!(Args::try_parse_from([
            "golish",
            "--stage-run",
            "--ephemeral-db",
            "--stage-run-test-toolsconfig-dir",
            "/tmp/controlled-toolsconfig",
            "--to",
            "reporting",
        ])
        .is_err());
        assert!(Args::try_parse_from([
            "golish",
            "--stage-run",
            "--stage-run-test-toolsconfig-dir",
            "/tmp/controlled-toolsconfig",
            "--stage-run-test-intel-providers-dir",
            "/tmp/controlled-intel-providers",
            "--stage-run-test-intel-provider-endpoint",
            "http://127.0.0.1:32123/intel/company.json",
            "--to",
            "reporting",
        ])
        .is_err());
    }

    #[test]
    fn deterministic_investigation_settings_are_loopback_only_and_credential_free() {
        let endpoint = url::Url::parse("http://127.0.0.1:32124/v1").expect("loopback URL");
        let settings = deterministic_investigation_settings(&endpoint)
            .expect("build deterministic Investigation settings");
        assert_eq!(
            settings.ai.default_provider,
            golish_settings::schema::AiProvider::Deepseek
        );
        assert_eq!(
            settings.ai.deepseek.base_url.as_deref(),
            Some("http://127.0.0.1:32124/v1")
        );
        assert_eq!(
            settings.ai.deepseek.api_key.as_deref(),
            Some("local-scripted-fixture")
        );
        assert!(settings.ai.sub_agent_models.is_empty());
        assert!(settings.mcp_servers.is_empty());
        assert!(settings.api_keys.tavily.is_none());
        assert!(settings.network.proxy_url.is_none());
        assert!(!settings.telemetry.langfuse.enabled);

        for rejected in [
            "https://127.0.0.1:32124/v1",
            "http://example.com/v1",
            "http://user:pass@127.0.0.1:32124/v1",
            "http://127.0.0.1:32124/v1?remote=true",
        ] {
            let rejected = url::Url::parse(rejected).expect("rejected URL parses");
            assert!(deterministic_investigation_settings(&rejected).is_err());
        }
    }

    #[test]
    fn exact_test_organization_identity_requires_owned_ephemeral_database() {
        let args = Args::try_parse_from([
            "golish",
            "--stage-run",
            "--ephemeral-db",
            "--org",
            "杭州默安科技有限公司",
            "--stage-run-test-organization-id",
            "19d56caa-d894-4bd3-954a-9a6709a6f560",
            "--to",
            "reporting",
        ])
        .expect("exact isolated organization identity parses");
        assert_eq!(
            args.stage_run_test_organization_id,
            Some(
                uuid::Uuid::parse_str("19d56caa-d894-4bd3-954a-9a6709a6f560")
                    .expect("fixture organization uuid")
            )
        );

        assert!(Args::try_parse_from([
            "golish",
            "--stage-run",
            "--org",
            "杭州默安科技有限公司",
            "--stage-run-test-organization-id",
            "19d56caa-d894-4bd3-954a-9a6709a6f560",
            "--to",
            "reporting",
        ])
        .is_err());
    }

    #[test]
    fn stage_run_db_accepts_only_explicit_gatefix_clone_names() {
        let args = Args::parse_from([
            "golish",
            "--stage-run-resume",
            "stage-run-test",
            "--stage-run-test-database",
            "golish_gatefix_20260720",
        ]);
        let stage_db = prepare_stage_run_db(&args).expect("isolated clone db config");
        assert_eq!(stage_db.config.database, "golish_gatefix_20260720");

        let mut invalid = args;
        invalid.stage_run_test_database = Some("golish".to_string());
        assert!(prepare_stage_run_db(&invalid).is_err());
    }

    #[test]
    fn format_db_smoke_summary_lists_sections() {
        let mut totals = BTreeMap::new();
        totals.insert("targets".to_string(), serde_json::json!(2));
        let mut run_scoped = BTreeMap::new();
        run_scoped.insert("tool_calls_by_chat_key".to_string(), serde_json::json!(3));
        let summary = DbSmokeSummary {
            session_id: "stage-run-test".into(),
            operation_id: Some("operation-1".into()),
            operation_identity: serde_json::json!({
                "runtimeMemoryContract":"v2_only",
                "attackExecutionContract":"v2_only",
                "enumerationAnalysisContract":"agent_team_v2",
                "toolTruthContract":"receipt_v1",
                "investigationContractVersion":"hypothesis_registry_v1",
                "investigationRolloutMode":"new_only",
                "stageTopologyContract":"unified_investigation_v1"
            }),
            organization_id: Some("org-1".into()),
            project_path: "/tmp/golish-smoke".into(),
            totals,
            run_scoped,
            operation_scoped: BTreeMap::new(),
            operation_exact_sets: BTreeMap::new(),
            project_scoped: BTreeMap::new(),
            org_scoped: BTreeMap::new(),
        };

        let rendered = format_db_smoke_summary(&summary);
        assert!(rendered.contains("-- db smoke summary --"));
        assert!(rendered.contains("targets: 2"));
        assert!(rendered.contains("tool_calls_by_chat_key: 3"));
        assert!(rendered.contains("operation_id = operation-1"));
        assert!(rendered.contains("\"attackExecutionContract\":\"v2_only\""));
        assert!(rendered.contains("\"enumerationAnalysisContract\":\"agent_team_v2\""));
    }

    #[test]
    fn format_report_includes_gate_and_evidence() {
        let events = vec![
            AiEvent::HarnessTrace {
                operation_id: "op".into(),
                stage: "scoping".into(),
                agent_path: "main".into(),
                trace: HarnessTraceKind::GateDecision {
                    gate: "PASS".into(),
                    findings: 0,
                    fabricated_evidence_refs: vec![],
                    available_real_ids: vec![],
                    first_blocking_reason: None,
                },
            },
            AiEvent::HarnessTrace {
                operation_id: "op".into(),
                stage: "target_intel".into(),
                agent_path: "main".into(),
                trace: HarnessTraceKind::EvidenceBooked {
                    tool: "recon_map_assets".into(),
                    evidence_id: 42,
                    source: "sync".into(),
                },
            },
        ];
        let result: Result<String> = Ok("done".into());
        let report = format_report(
            &events,
            &result,
            "assessment",
            StageKind::Scoping,
            StageKind::TargetIntel,
            "stage-run-x",
            Path::new("/tmp/t"),
        );
        assert!(report.contains("[PASS] scoping"));
        assert!(report.contains("#42 from recon_map_assets"));
        assert!(report.contains("golish --replay stage-run-x"));
    }

    #[test]
    fn format_report_shows_block_reason_and_failure() {
        let events = vec![AiEvent::HarnessTrace {
            operation_id: "op".into(),
            stage: "scoping".into(),
            agent_path: "main".into(),
            trace: HarnessTraceKind::GateDecision {
                gate: "BLOCK".into(),
                findings: 0,
                fabricated_evidence_refs: vec![1, 2],
                available_real_ids: vec![],
                first_blocking_reason: Some("missing scope_human_approved claim".into()),
            },
        }];
        let result: Result<String> = Err(anyhow!("stage blocked"));
        let report = format_report(
            &events,
            &result,
            "pentest",
            StageKind::Scoping,
            StageKind::Scoping,
            "s",
            Path::new("/tmp/t"),
        );
        assert!(report.contains("[BLOCK] scoping"));
        assert!(report.contains("missing scope_human_approved claim"));
        assert!(report.contains("fabricated evidence refs: [1, 2]"));
        assert!(report.contains("FAILED: stage blocked"));
    }

    #[test]
    fn format_report_surfaces_redacted_typed_stage_run_failure() {
        let operation_id = uuid::Uuid::parse_str("425c7693-99fb-4598-8361-62275c9413b1").unwrap();
        let obligation_id = uuid::Uuid::new_v4();
        let typed_error =
            golish_agent_kit::db_traits::InvestigationAnalysisHostError::RevalidationRequired {
                operation_id,
                revalidation_obligation_ids: vec![obligation_id],
                stale_roots: vec!["external_attack_surface:expired".to_owned()],
            };
        let events = vec![AiEvent::ToolResult {
            tool_name: "stage_run".to_owned(),
            result: serde_json::json!({
                "code": "INVESTIGATION_UNIT_BLOCKED",
                "error": format!("run Investigation Analysis Primary:\n{typed_error}"),
                "credential": "must-not-reach-cli-report",
                "unit_results": [{"private": "must-not-reach-cli-report"}],
            }),
            success: false,
            request_id: "stage-run-tool-call".to_owned(),
            source: golish_core::events::ToolSource::Main,
        }];
        let result: Result<String> = Ok("model handled the failed tool result".to_owned());
        let report = format_report(
            &events,
            &result,
            "pentest",
            StageKind::Investigation,
            StageKind::Investigation,
            "stage-run-test",
            Path::new("/tmp/golish-stage-run-test"),
        );
        assert!(report.contains("stage_run: err [typed code=INVESTIGATION_UNIT_BLOCKED"));
        assert!(report.contains("investigation_analysis_host_revalidation_required"));
        assert!(report.contains(&format!("operation_id={operation_id}")));
        assert!(!report.contains("must-not-reach-cli-report"));
        assert!(!report.contains("run Investigation Analysis Primary"));
        assert!(report.contains("other_payload=redacted"));
    }

    #[test]
    fn build_objective_includes_seeded_org_id_and_targets() {
        let args = Args::parse_from([
            "golish",
            "--stage-run",
            "--only",
            "target_intel",
            "--org",
            "ACME",
            "--target",
            "acme.com",
        ]);
        let seed = SeedResult {
            org_id: Some(uuid::Uuid::nil()),
            org_name: Some("ACME".into()),
            targets_added: 1,
        };
        let obj = build_objective(&args, StageKind::TargetIntel, Some(&seed));
        assert!(obj.contains("organization_id: 00000000-0000-0000-0000-000000000000"));
        assert!(obj.contains("Organization: ACME"));
        assert!(obj.contains("In-scope targets: acme.com"));
    }

    #[test]
    fn build_objective_prefers_explicit_execute() {
        let args = Args::parse_from([
            "golish",
            "--stage-run",
            "--only",
            "scoping",
            "-e",
            "custom obj",
        ]);
        assert_eq!(
            build_objective(&args, StageKind::Scoping, None),
            "custom obj"
        );
    }

    #[test]
    fn build_objective_keeps_custom_text_and_appends_trusted_seed_metadata() {
        let args = Args::parse_from([
            "golish",
            "--stage-run",
            "--only",
            "scoping",
            "--org",
            "广州有创网络科技有限公司",
            "-e",
            "custom smoke objective",
        ]);
        let seed = SeedResult {
            org_id: Some(ORG_ID),
            org_name: Some("广州有创网络科技有限公司".into()),
            targets_added: 0,
        };

        let objective = build_objective(&args, StageKind::Scoping, Some(&seed));
        assert!(objective.starts_with("custom smoke objective"));
        assert!(objective.contains("广州有创网络科技有限公司"));
        assert!(objective.contains(&format!("organization_id: {ORG_ID}")));
        assert!(!objective.contains("In-scope targets:"));
    }

    #[test]
    fn parse_seed_open_ports_accepts_sorted_unique_ports() {
        let specs = parse_seed_open_ports("192.0.2.10=443,80,80; 192.0.2.11 = 9001 ");

        assert_eq!(
            specs,
            vec![
                ("192.0.2.10".to_string(), vec![80, 443]),
                ("192.0.2.11".to_string(), vec![9001]),
            ]
        );
    }

    #[test]
    fn controlled_web_origin_seed_parser_is_http_exact_set() {
        assert_eq!(
            parse_controlled_web_origin_seeds(
                "http://127.0.0.1:18080/path; HTTP://127.0.0.1:18080/other"
            )
            .expect("parse and deduplicate exact origin"),
            vec!["http://127.0.0.1:18080/".to_string()]
        );
        assert!(parse_controlled_web_origin_seeds("file:///tmp/not-network").is_err());
        assert!(parse_controlled_web_origin_seeds("https://user:secret@example.com/").is_err());
        assert!(parse_controlled_web_origin_seeds(" ; ").is_err());
    }

    #[test]
    fn vault_seed_updates_runtime_canonical_and_legacy_names() {
        assert_eq!(
            vault_seed_entry_names("quake"),
            ("quake".to_string(), "quake.default.api_key".to_string())
        );
    }

    // ── Phase 3 (2026-06-12-redteam-phase3, 方案 A): per-subsidiary dispatch ──
    // `filter_child_orgs` / `build_child_objective` (+ their tests) moved to
    // `golish_agent_kit::harness::stage_fanout` so the CLI and the chat
    // `stage_run` tool share one implementation.

    #[test]
    fn child_slice_none_for_scoping_only_and_skips_scoping_otherwise() {
        // --to scoping = tree-build only → no per-subsidiary runs.
        assert!(child_slice("red_team", StageKind::Scoping).is_none());
        // --to target_intel → child runs cover target_intel only (never scoping).
        let (entry, allow) =
            child_slice("red_team", StageKind::TargetIntel).expect("slice resolves");
        assert_eq!(entry, StageKind::TargetIntel);
        assert!(allow.contains(&StageKind::TargetIntel));
        assert!(
            !allow.contains(&StageKind::Scoping),
            "subsidiary runs must never re-run scoping (engagement-level stage)"
        );
    }
}
