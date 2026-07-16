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

use std::collections::{BTreeMap, BTreeSet, HashSet};
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

/// Resolve an explicit forward continuation for an exact resume. The default
/// remains the persisted current stage only; callers must opt in to every
/// wider testing slice without changing the operation's frozen profile/scope.
fn resolve_resume_slice(
    profile_id: &str,
    current: StageKind,
    requested_to: Option<&str>,
) -> Result<(StageKind, HashSet<StageKind>)> {
    let terminal = requested_to
        .map(|value| {
            StageKind::try_parse(value).ok_or_else(|| anyhow!("unknown --resume-to stage: {value}"))
        })
        .transpose()?
        .unwrap_or(current);
    let (entry, allowlist) = resolve_slice(profile_id, Some(current), terminal)
        .context("resume refused: requested continuation is outside the frozen profile")?;
    anyhow::ensure!(
        entry == current,
        "resume refused: requested continuation does not begin at the persisted current stage"
    );
    Ok((terminal, allowlist))
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

const REPAIR_REAPED_TASK_SQL: &str = r#"UPDATE tasks
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
         )"#;

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
    resolve_slice(&candidate.profile, Some(stage), stage)
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
            anyhow::ensure!(
                candidate.engagement_org_id == Some(relational.organization_id),
                "resume refused: relational scope root does not equal operation engagement organization"
            );
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
            match runtime_v2::load_relational_resume_authority(pool, session.id, &operation, stage)
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
            runtime_v2::load_relational_resume_authority(pool, session.id, &operation, stage)
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
        engagement_org_id: operation.engagement_org_id,
        superseded_by: operation.superseded_by,
        state_blob: operation.state_blob,
        worker_chains,
        relational_v2,
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

/// Headless entry point for `golish --stage-run`.
pub async fn run(args: Args) -> Result<()> {
    if args.stage_run_resume.is_some() {
        return run_resume(args).await;
    }

    // 1) Resolve profile + stage slice up front (cheap, fails fast on bad input).
    let profile_id = args
        .profile
        .clone()
        .unwrap_or_else(|| active_profile_id().to_string());
    let (from_opt, to_stage) = resolve_from_to(&args)?;
    let (entry_stage, allowlist) = resolve_slice(&profile_id, from_opt, to_stage)?;
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
    let settings = settings_manager.get().await;
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
            include_subsidiaries: Some(args.include_subsidiaries),
            confirmed_organization_id: seed.as_ref().and_then(|seed| seed.org_id),
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
            .await,
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

    let execution_result: Result<()> = async {
        let initial = resolve_stage_run_resume_target(&db_pool, &selector, &expectations).await?;
        let (resume_terminal, resume_allowlist) = resolve_resume_slice(
            &initial.profile,
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

        eprintln!(
            "[stage-run-resume] session={} db_session={} operation={} org={} profile={} stage={} to={}",
            target.chat_session_key,
            target.session_id,
            target.operation_id,
            target.organization_id,
            target.profile,
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
                // Exact resume must use frozen operation truth. The resume CLI
                // has no authority to invent a new subsidiary decision.
                include_subsidiaries: None,
                confirmed_organization_id: None,
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
                .await,
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
        golish_agent_kit::runtime_memory::authorize_operation_project_scope(
            operation.project_scope_id,
            operation.runtime_memory_contract,
            current_project_scope.project_scope_id,
        )
        .map_err(anyhow::Error::new)
        .context("authorize exact-resume project scope")?;

        let mut orchestrator = prepared.build_orchestrator(TaskOperationConfig {
            profile_override: Some(target.profile.clone()),
            stage_allowlist: Some(stage_allowlist.clone()),
            harness_org_id: Some(target.organization_id),
            current_invocation_target_authority: persisted_resume_target_authority(
                &target.state_blob,
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

fn persisted_resume_target_authority(state_blob: &serde_json::Value) -> Result<Option<bool>> {
    golish_agent_kit::task_orchestrator::harness_resume::current_invocation_target_authority_from_state_blob(
        state_blob,
    )
    .context("restore persisted fresh target authority for exact resume")
    // Exact resume is a headless stage-run surface, not the GUI interactive
    // lifecycle. A missing marker is therefore not permission to consult an
    // organization's historical targets: fail closed until a fresh invocation
    // supplies an exact --target again.
    .map(|authority| Some(authority.unwrap_or(false)))
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
    /// `Some` only for a fresh CLI invocation where the flag itself is trusted
    /// intake. Exact resume has no authority to create a new scope decision.
    include_subsidiaries: Option<bool>,
    /// Fresh CLI `--org` identity. A subsidiary-scope choice is machine
    /// resolvable only when its typed context names this exact seeded root.
    confirmed_organization_id: Option<uuid::Uuid>,
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
        _question: &str,
        options: &[String],
        context: &str,
    ) -> StageRunAutoResolution {
        match input_type {
            "scope_review" => self
                .trusted_scope_response
                .clone()
                .map(StageRunAutoResolution::Approve)
                .unwrap_or(StageRunAutoResolution::Decline(
                    "scope_review requires exact trusted CLI target rows",
                )),
            "confirmation" if self.approve_phase_boundaries => StageRunAutoResolution::Approve(
                "approved by explicit CLI --approve-phase-boundaries".to_string(),
            ),
            "confirmation" => StageRunAutoResolution::Decline(
                "phase confirmation requires --approve-phase-boundaries",
            ),
            "choice" => {
                let Some(request_organization_id) = subsidiary_scope_choice_org(context) else {
                    return StageRunAutoResolution::Decline(
                        "choice is not a typed subsidiary-scope decision",
                    );
                };
                if self.confirmed_organization_id != Some(request_organization_id) {
                    return StageRunAutoResolution::Decline(
                        "subsidiary choice organization does not match trusted CLI --org",
                    );
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

fn subsidiary_option_excludes(option: &str) -> bool {
    let normalized = option.trim().to_lowercase();
    [
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
    .any(|marker| normalized.contains(marker))
}

fn subsidiary_option_includes(option: &str) -> bool {
    let normalized = option.trim().to_lowercase();
    !subsidiary_option_excludes(option)
        && [
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
    let seed = seed_upstream(db_pool, project_path, args.org.as_deref(), &args.target)
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
            };
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
    organization_id: Option<String>,
    project_path: String,
    totals: BTreeMap<String, serde_json::Value>,
    run_scoped: BTreeMap<String, serde_json::Value>,
    project_scoped: BTreeMap<String, serde_json::Value>,
    org_scoped: BTreeMap<String, serde_json::Value>,
}

async fn collect_db_smoke_summary(
    pool: &sqlx::PgPool,
    session_id: &str,
    org_id: Option<uuid::Uuid>,
    project_path: &str,
) -> DbSmokeSummary {
    let totals = collect_unbound_counts(
        pool,
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
    .await;

    let run_scoped = collect_text_counts(
        pool,
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
    .await;

    let project_scoped = collect_text_counts(
        pool,
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
    .await;

    let org_scoped = match org_id {
        Some(org_id) => {
            collect_uuid_counts(
                pool,
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
            .await
        }
        None => BTreeMap::new(),
    };

    DbSmokeSummary {
        session_id: session_id.to_string(),
        organization_id: org_id.map(|id| id.to_string()),
        project_path: project_path.to_string(),
        totals,
        run_scoped,
        project_scoped,
        org_scoped,
    }
}

async fn collect_unbound_counts(
    pool: &sqlx::PgPool,
    queries: &[(&'static str, &'static str)],
) -> BTreeMap<String, serde_json::Value> {
    let mut out = BTreeMap::new();
    for (label, sql) in queries {
        out.insert((*label).to_string(), count_unbound(pool, sql).await);
    }
    out
}

async fn collect_text_counts(
    pool: &sqlx::PgPool,
    value: &str,
    queries: &[(&'static str, &'static str)],
) -> BTreeMap<String, serde_json::Value> {
    let mut out = BTreeMap::new();
    for (label, sql) in queries {
        out.insert((*label).to_string(), count_text(pool, sql, value).await);
    }
    out
}

async fn collect_uuid_counts(
    pool: &sqlx::PgPool,
    value: uuid::Uuid,
    queries: &[(&'static str, &'static str)],
) -> BTreeMap<String, serde_json::Value> {
    let mut out = BTreeMap::new();
    for (label, sql) in queries {
        out.insert((*label).to_string(), count_uuid(pool, sql, value).await);
    }
    out
}

async fn count_unbound(pool: &sqlx::PgPool, sql: &str) -> serde_json::Value {
    match sqlx::query_scalar::<_, i64>(sql).fetch_one(pool).await {
        Ok(count) => serde_json::json!(count),
        Err(e) => serde_json::json!({ "error": e.to_string() }),
    }
}

async fn count_text(pool: &sqlx::PgPool, sql: &str, value: &str) -> serde_json::Value {
    match sqlx::query_scalar::<_, i64>(sql)
        .bind(value)
        .fetch_one(pool)
        .await
    {
        Ok(count) => serde_json::json!(count),
        Err(e) => serde_json::json!({ "error": e.to_string() }),
    }
}

async fn count_uuid(pool: &sqlx::PgPool, sql: &str, value: uuid::Uuid) -> serde_json::Value {
    match sqlx::query_scalar::<_, i64>(sql)
        .bind(value)
        .fetch_one(pool)
        .await
    {
        Ok(count) => serde_json::json!(count),
        Err(e) => serde_json::json!({ "error": e.to_string() }),
    }
}

fn format_db_smoke_summary(summary: &DbSmokeSummary) -> String {
    let mut out = String::new();
    out.push_str("\n-- db smoke summary --\n");
    out.push_str(&format!("  session_id = {}\n", summary.session_id));
    out.push_str(&format!("  project_path = {}\n", summary.project_path));
    if let Some(org_id) = &summary.organization_id {
        out.push_str(&format!("  organization_id = {org_id}\n"));
    }
    push_db_summary_section(&mut out, "totals", &summary.totals);
    push_db_summary_section(&mut out, "run scoped", &summary.run_scoped);
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
                tool_name, success, ..
            } => {
                tool_lines.push(format!(
                    "  {tool_name}: {}",
                    if *success { "ok" } else { "err" }
                ));
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
            include_subsidiaries: Some(false),
            confirmed_organization_id: Some(ORG_ID),
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
            include_subsidiaries: Some(true),
            confirmed_organization_id: Some(ORG_ID),
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
            persisted_resume_target_authority(&company_only)
                .expect("valid persisted company-only marker"),
            Some(false)
        );

        let exact_target = golish_agent_kit::task_orchestrator::harness_resume::state_blob_with_current_invocation_target_authority(
            serde_json::json!({"profile": "red_team", "current_stage": "target_intel"}),
            true,
        );
        assert_eq!(
            persisted_resume_target_authority(&exact_target)
                .expect("valid persisted exact-target marker"),
            Some(true)
        );
        assert_eq!(
            persisted_resume_target_authority(
                &serde_json::json!({"profile": "red_team", "current_stage": "scoping"})
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
        assert!(persisted_resume_target_authority(&malformed).is_err());
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
        let (terminal, allowlist) =
            resolve_resume_slice("pentest", StageKind::TargetIntel, None).expect("same stage");
        assert_eq!(terminal, StageKind::TargetIntel);
        assert_eq!(allowlist, HashSet::from([StageKind::TargetIntel]));
    }

    #[test]
    fn resolve_resume_slice_expands_only_forward_to_candidate() {
        let (terminal, allowlist) =
            resolve_resume_slice("pentest", StageKind::TargetIntel, Some("attack_candidate"))
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
        assert!(resolve_resume_slice("pentest", StageKind::TargetIntel, Some("scoping")).is_err());
        assert!(resolve_resume_slice(
            "assessment",
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
    fn format_db_smoke_summary_lists_sections() {
        let mut totals = BTreeMap::new();
        totals.insert("targets".to_string(), serde_json::json!(2));
        let mut run_scoped = BTreeMap::new();
        run_scoped.insert("tool_calls_by_chat_key".to_string(), serde_json::json!(3));
        let summary = DbSmokeSummary {
            session_id: "stage-run-test".into(),
            organization_id: Some("org-1".into()),
            project_path: "/tmp/golish-smoke".into(),
            totals,
            run_scoped,
            project_scoped: BTreeMap::new(),
            org_scoped: BTreeMap::new(),
        };

        let rendered = format_db_smoke_summary(&summary);
        assert!(rendered.contains("-- db smoke summary --"));
        assert!(rendered.contains("targets: 2"));
        assert!(rendered.contains("tool_calls_by_chat_key: 3"));
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
