//! Shared trusted operation-resume service.
//!
//! Candidate review uses this after a durable DB `resume_pending -> dispatching`
//! CAS. No in-memory wake flag is consulted. The service validates the exact
//! task/session/project operation binding before spawning the existing
//! orchestrator resume path.

use std::sync::Arc;

use anyhow::Context;
use golish_agent_bridge::bridge_executor::BridgeAgentExecutor;
use golish_agent_kit::db_traits::{
    DbRepoProvider, ProjectScopeRegistration, RuntimeMemoryRecordSource, RuntimeMemoryRepository,
};
use golish_agent_kit::task_orchestrator::TaskOrchestrator;
use golish_core::events::AiEvent;
use golish_db::repo::attack_candidate_approvals::ReviewResumeClaim;
use uuid::Uuid;

use crate::ai::AgentBridge;
use crate::state::AgentState;

pub(super) async fn select_exact_resume_runtime_source(
    pool: &sqlx::PgPool,
    operation_id: Uuid,
    session_id: Uuid,
) -> anyhow::Result<RuntimeMemoryRecordSource> {
    use golish_db::repo::runtime_memory_tx::RuntimeMemoryRecordSource as DbSource;

    let source =
        golish_db::repo::tasks::exact_resumable_runtime_source(pool, operation_id, session_id)
            .await
            .context("select one complete runtime-memory source for exact resume")?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "resume refused: operation has no complete idle runtime-memory source"
                )
            })?;
    let source = match source {
        DbSource::Legacy => RuntimeMemoryRecordSource::Legacy,
        DbSource::V2 => RuntimeMemoryRecordSource::V2,
        DbSource::LegacyFallback => RuntimeMemoryRecordSource::LegacyFallback,
    };
    if source == RuntimeMemoryRecordSource::V2 {
        let chains = golish_db::repo::message_chains::list_exact_resume_bound_chains(
            pool,
            operation_id,
            session_id,
        )
        .await
        .context("load exact relational resume worker chains")?;
        for row in chains {
            match row.message_chain_id {
                Some(expected_chain_id) => {
                    anyhow::ensure!(
                        row.exact_chain_id == Some(expected_chain_id),
                        "resume refused: Worker {} chain crossed session/task/agent ownership",
                        row.worker_run_id
                    );
                    let chain = row.chain.ok_or_else(|| {
                        anyhow::anyhow!(
                            "resume refused: Worker {} bound chain body is missing",
                            row.worker_run_id
                        )
                    })?;
                    serde_json::from_value::<Vec<rig::completion::Message>>(chain).with_context(
                        || {
                            format!(
                                "resume refused: Worker {} bound chain cannot be decoded",
                                row.worker_run_id
                            )
                        },
                    )?;
                }
                None => anyhow::ensure!(
                    row.worker_status == "queued",
                    "resume refused: non-queued Worker {} has no bound chain",
                    row.worker_run_id
                ),
            }
        }
    }
    Ok(source)
}

pub(super) async fn claim_exact_resume_runtime_source(
    pool: &sqlx::PgPool,
    operation_id: Uuid,
    session_id: Uuid,
    source: RuntimeMemoryRecordSource,
) -> anyhow::Result<()> {
    use golish_db::repo::runtime_memory_tx::RuntimeMemoryRecordSource as DbSource;

    let source = match source {
        RuntimeMemoryRecordSource::Legacy => DbSource::Legacy,
        RuntimeMemoryRecordSource::V2 => DbSource::V2,
        RuntimeMemoryRecordSource::LegacyFallback => DbSource::LegacyFallback,
    };
    let claimed = golish_db::repo::tasks::claim_exact_resumable_runtime_source(
        pool,
        operation_id,
        session_id,
        source,
    )
    .await
    .context("atomically claim exact runtime-memory resume source")?;
    anyhow::ensure!(
        claimed,
        "resume refused: task or selected runtime-memory source changed before the durable claim"
    );
    Ok(())
}

pub(super) async fn has_resumable_task_for_session(
    state: &AgentState,
    bridge: &AgentBridge,
    session_id: &str,
    task_input: &str,
) -> anyhow::Result<bool> {
    use golish_db::{models::NewSession, repo::sessions};

    let session_row = sessions::upsert_by_chat_key(
        &state.db_pool,
        session_id,
        NewSession {
            title: Some(task_input.chars().take(80).collect()),
            workspace_path: None,
            workspace_label: None,
            model: Some(bridge.model_name().to_string()),
            provider: Some(bridge.provider_name().to_string()),
            project_path: None,
        },
    )
    .await
    .context("Failed to upsert session row for task resume preflight")?;
    Ok(
        golish_db::repo::tasks::latest_resumable_by_session(&state.db_pool, session_row.id)
            .await
            .context("Failed to query latest resumable task")?
            .is_some(),
    )
}

pub(super) async fn authorize_operation_resume(
    repo: &dyn DbRepoProvider,
    operation_id: Uuid,
    current_project_scope: &ProjectScopeRegistration,
) -> anyhow::Result<golish_agent_kit::db_traits::OperationStateView> {
    let operation = repo
        .operation_state_get(operation_id)
        .await
        .context("Load persisted operation project scope for resume")?
        .ok_or_else(|| anyhow::anyhow!("resume operation_state is missing"))?;
    golish_agent_kit::runtime_memory::authorize_operation_project_scope(
        operation.project_scope_id,
        operation.runtime_memory_contract,
        current_project_scope.project_scope_id,
    )
    .map_err(anyhow::Error::new)
    .context("Authorize current project scope for operation resume")?;
    Ok(operation)
}

/// Start the existing TaskOrchestrator resume path after the caller has claimed
/// the durable Candidate review barrier. Synchronous setup errors are returned
/// so the caller can CAS `dispatching -> resume_pending`; runtime completion is
/// intentionally asynchronous and remains represented by durable task state.
pub(crate) async fn start_trusted_candidate_review_resume(
    state: &AgentState,
    claim: &ReviewResumeClaim,
) -> anyhow::Result<Arc<AgentBridge>> {
    let bridge = state
        .ai_state
        .get_session_bridge(&claim.chat_session_key)
        .await
        .ok_or_else(|| {
            anyhow::anyhow!(
                "trusted Candidate review session is not initialized: {}",
                claim.chat_session_key
            )
        })?;
    let request = bridge
        .begin_top_level_request()
        .await
        .context("claim Candidate review resume request lease")?;
    bridge.set_tracker_session_uuid(claim.session_id);

    let provider = Arc::new(crate::ai::db_bridge::GolishDbRepoProvider::new(
        state.db_pool.clone(),
    ));
    let db_repo: Arc<dyn DbRepoProvider> = provider.clone();
    let runtime_repo: Arc<dyn RuntimeMemoryRepository> = provider;
    let workspace = bridge.workspace().read().await.clone();
    let (canonical_path, path_sha256) =
        golish_agent_kit::runtime_memory::canonical_workspace_identity(&workspace)
            .map_err(anyhow::Error::new)
            .context("resolve trusted workspace identity for Candidate resume")?;
    let current_project_scope = runtime_repo
        .project_scope_register_first_open(&canonical_path, &path_sha256)
        .await
        .map_err(anyhow::Error::new)
        .context("register trusted project scope for Candidate resume")?;
    let operation =
        authorize_operation_resume(db_repo.as_ref(), claim.operation_id, &current_project_scope)
            .await?;
    if operation.project_scope_id != Some(claim.project_scope_id)
        || operation.profile != claim.profile
        || operation.current_stage != "attack_candidate"
    {
        anyhow::bail!("Candidate resume operation/project/profile/stage identity drifted");
    }
    let resume_source = select_exact_resume_runtime_source(
        state.db_pool.as_ref(),
        claim.operation_id,
        claim.session_id,
    )
    .await?;

    let executor = BridgeAgentExecutor::from_request(bridge.clone(), request.clone())
        .context("upgrade Candidate resume request into Task execution")?;
    let mut orchestrator = TaskOrchestrator::new(
        db_repo,
        runtime_repo,
        claim.session_id,
        bridge.get_or_create_event_tx(),
    );
    orchestrator.set_profile_override(Some(claim.profile.clone()));
    orchestrator.set_chat_session_id(&claim.chat_session_key);
    orchestrator.set_approval_coordinator(bridge.coordinator().cloned());
    claim_exact_resume_runtime_source(
        state.db_pool.as_ref(),
        claim.operation_id,
        claim.session_id,
        resume_source,
    )
    .await?;
    orchestrator.set_resume_runtime_memory_source(resume_source);
    orchestrator.set_resume_task_preclaimed(true);
    bridge.set_resume_runtime_memory_source(resume_source).await;

    let operation_id = claim.operation_id;
    let task_bridge = bridge.clone();
    tokio::spawn(async move {
        let result = orchestrator
            .resume(
                operation_id,
                "Resume verification after durable Candidate review.",
                &executor,
            )
            .await;
        if let Err(error) = task_bridge.clear_top_level_request_state(&request).await {
            tracing::warn!(
                target: "harness::candidate_review",
                operation_id = %operation_id,
                error = %error,
                "failed to clear Candidate resume request-local state"
            );
        }
        if let Err(error) = result {
            task_bridge.emit_event(AiEvent::Error {
                message: format!("Candidate review resume failed: {error:#}"),
                error_type: "candidate_review_resume".to_string(),
            });
        }
    });
    Ok(bridge)
}
