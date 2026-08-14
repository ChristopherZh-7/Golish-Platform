//! Sub-agent execution.
//!
//! [`execute_sub_agent`] is the public entry point: it wraps the inner
//! orchestrator with an *optional* overall timeout and uniform error handling.
//! When `timeout_secs` is `None` the sub-agent runs to completion, bounded only
//! by its idle timeout, per-tool timeouts, and `max_iterations` — it keeps going
//! as long as it is making progress. The actual iterate-stream-dispatch loop
//! lives in [`inner`], with one-shot setup/teardown phases delegated to
//! dedicated submodules:
//!
//! - [`prompt_assembly`]: build the effective system prompt (optimized
//!   prompt + briefing + skills + barrier instruction).
//! - [`tool_setup`]: build the tool list (allowed tools + barrier + nested
//!   delegation shims).
//! - [`chain_persist`]: restore/persist the message chain row.
//! - [`final_summary`]: tool-less final call when iteration cap is hit.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use rig::completion::CompletionModel as RigCompletionModel;
use tracing::Instrument;

use crate::definition::{SubAgentContext, SubAgentDefinition, SubAgentResult};
use crate::executor_types::SubAgentChainError;
pub use crate::executor_types::{SubAgentExecutorContext, ToolProvider, BARRIER_TOOL_NAME};
use golish_core::events::AiEvent;

mod chain_persist;
mod final_summary;
mod history_compaction;
mod inner;
mod prompt_assembly;
mod response_parsing;
mod stream_processing;
mod tool_setup;

/// In-memory handoff for the last chain body whose database update completed.
///
/// The outer timeout wrapper owns one clone so it can still read the durable
/// identity after dropping the inner execution future. Poison recovery is safe:
/// the protected value is a copy-only UUID, not a compound mutable invariant.
#[derive(Clone, Default)]
pub(super) struct CheckpointedChainId {
    value: Arc<Mutex<Option<uuid::Uuid>>>,
}

impl CheckpointedChainId {
    pub(super) fn publish(&self, chain_id: Option<uuid::Uuid>) {
        if chain_id.is_some() {
            *self
                .value
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) = chain_id;
        }
    }

    pub(super) fn get(&self) -> Option<uuid::Uuid> {
        *self
            .value
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

fn map_inner_result(
    result: Result<SubAgentResult>,
    agent_def: &SubAgentDefinition,
    parent_context: &SubAgentContext,
    start_time: std::time::Instant,
    checkpointed_chain_id: &CheckpointedChainId,
) -> Result<SubAgentResult> {
    let error = match result {
        Ok(result) => return Ok(result),
        Err(error) => error,
    };

    if let Some(SubAgentChainError::FinalizeFailed {
        chain_id, reason, ..
    }) = error.downcast_ref::<SubAgentChainError>()
    {
        let Some(checkpointed_chain_id) = checkpointed_chain_id.get() else {
            return Err(error);
        };
        return Err(SubAgentChainError::FinalizeFailed {
            chain_id: *chain_id,
            checkpointed_chain_id: Some(checkpointed_chain_id),
            reason: reason.clone(),
        }
        .into());
    }

    // Keep the stable typed runtime policy for provider context limits and for
    // restore/create failures.
    let preserve_typed_error = matches!(
        error.downcast_ref::<SubAgentChainError>(),
        Some(
            SubAgentChainError::ProviderContextLimitExceeded { .. }
                | SubAgentChainError::ExactResumeUnavailable { .. }
                | SubAgentChainError::LatestResumeUnavailable { .. }
                | SubAgentChainError::CreateFreshFailed { .. }
                | SubAgentChainError::BoundWorkerUnavailable { .. }
        )
    );
    if preserve_typed_error {
        return Err(error);
    }

    let Some(chain_id) = checkpointed_chain_id.get() else {
        return Err(error);
    };
    let response =
        chain_persist::append_durable_chain_marker(format!("Error: {error}"), Some(chain_id));
    Ok(SubAgentResult {
        agent_id: agent_def.id.clone(),
        response,
        context: SubAgentContext {
            original_request: parent_context.original_request.clone(),
            conversation_summary: parent_context.conversation_summary.clone(),
            variables: parent_context.variables.clone(),
            depth: parent_context.depth + 1,
            parent_agent: parent_context.parent_agent.clone(),
            task_id: parent_context.task_id.clone(),
            subtask_id: parent_context.subtask_id.clone(),
            execution_history: parent_context.execution_history.clone(),
        },
        success: false,
        duration_ms: start_time.elapsed().as_millis() as u64,
        files_modified: Vec::new(),
        chain_id: Some(chain_id),
    })
}

pub use response_parsing::{
    refine_eas_web_repair_mode_from_worklist, retain_eas_web_repair_targets_for_same_gap,
    submit_coverage_gap_repair_mode_from_reasons, submit_repair_mode_from_submit_result,
};

/// Execute a sub-agent with the given task and context.
///
/// This is the public entry point. When `agent_def.timeout_secs` is `Some`, it
/// wraps [`inner::execute_sub_agent_inner`] with that overall (wall-clock)
/// timeout and emits a graceful [`AiEvent::SubAgentError`] when it fires. When
/// `timeout_secs` is `None`, no overall cap is applied and the agent runs until
/// it finishes, hits its idle timeout, or exhausts `max_iterations`.
///
/// # Arguments
/// * `agent_def` — sub-agent definition
/// * `args` — JSON arguments containing `task` and optional `context`
/// * `parent_context` — context from the parent agent
/// * `model` — LLM model implementing [`RigCompletionModel`]
/// * `ctx` — execution context with shared resources
/// * `tool_provider` — provider for tool definitions and execution
/// * `parent_request_id` — ID of the parent request that spawned this sub-agent
///
/// # Returns
/// The result of the sub-agent execution.
pub async fn execute_sub_agent<M, P>(
    agent_def: &SubAgentDefinition,
    args: &serde_json::Value,
    parent_context: &SubAgentContext,
    model: &M,
    ctx: SubAgentExecutorContext<'_>,
    tool_provider: &P,
    parent_request_id: &str,
) -> Result<SubAgentResult>
where
    M: RigCompletionModel + Sync,
    P: ToolProvider,
{
    let start_time = std::time::Instant::now();
    let agent_id = &agent_def.id;

    // Create span for sub-agent execution (Langfuse observability).
    //
    // IMPORTANT: explicitly parent this span to the current span so sub-agent
    // work is attached to the main trace even when crossing async/task
    // boundaries.
    let sub_agent_span = tracing::info_span!(
        parent: &tracing::Span::current(),
        "sub_agent",
        "langfuse.observation.type" = "agent",
        "langfuse.session.id" = ctx.session_id.unwrap_or(""),
        "langfuse.observation.input" = tracing::field::Empty,
        "langfuse.observation.output" = tracing::field::Empty,
        agent_type = %format!("sub-agent:{}", agent_id),
        agent_id = %agent_id,
        model = %ctx.model_name,
        provider = %ctx.provider_name,
        depth = parent_context.depth + 1,
    );

    // Overall (wall-clock) execution cap. `None` = run to completion, bounded
    // only by the idle timeout, per-tool timeouts, and `max_iterations`.
    let overall_timeout = agent_def.timeout_secs.map(Duration::from_secs);
    let idle_timeout_duration = agent_def.idle_timeout_secs.map(Duration::from_secs);

    // A single tool call stays bounded even when the overall cap is disabled, so
    // one hung tool can't wedge the loop forever. Prefer the idle timeout, then
    // any configured overall cap, then a conservative default.
    let tool_fallback_timeout = idle_timeout_duration
        .or(overall_timeout)
        .unwrap_or(Duration::from_secs(600));

    // Clone event_tx before `ctx` is moved into the inner future (the overall
    // timeout error path below needs it).
    let event_tx_clone = ctx.event_tx.clone();

    let checkpointed_chain_id = CheckpointedChainId::default();
    let inner_fut = inner::execute_sub_agent_inner(
        agent_def,
        args,
        parent_context,
        model,
        ctx,
        tool_provider,
        parent_request_id,
        start_time,
        &sub_agent_span,
        tool_fallback_timeout,
        idle_timeout_duration,
        checkpointed_chain_id.clone(),
    )
    .instrument(sub_agent_span.clone());

    let Some(overall_timeout) = overall_timeout else {
        return map_inner_result(
            inner_fut.await,
            agent_def,
            parent_context,
            start_time,
            &checkpointed_chain_id,
        );
    };

    match tokio::time::timeout(overall_timeout, inner_fut).await {
        Ok(result) => map_inner_result(
            result,
            agent_def,
            parent_context,
            start_time,
            &checkpointed_chain_id,
        ),
        Err(_elapsed) => {
            let duration_ms = start_time.elapsed().as_millis() as u64;
            let error_msg = format!(
                "Sub-agent '{}' timed out after {}s",
                agent_def.id,
                overall_timeout.as_secs()
            );
            tracing::warn!("{}", error_msg);

            let _ = event_tx_clone.send(AiEvent::SubAgentError {
                agent_id: agent_def.id.clone(),
                error: error_msg.clone(),
                parent_request_id: parent_request_id.to_string(),
            });

            let chain_id = checkpointed_chain_id.get();
            let response =
                chain_persist::append_durable_chain_marker(format!("Error: {error_msg}"), chain_id);

            Ok(SubAgentResult {
                agent_id: agent_def.id.clone(),
                response,
                context: SubAgentContext {
                    original_request: parent_context.original_request.clone(),
                    conversation_summary: parent_context.conversation_summary.clone(),
                    variables: parent_context.variables.clone(),
                    depth: parent_context.depth + 1,
                    parent_agent: parent_context.parent_agent.clone(),
                    task_id: parent_context.task_id.clone(),
                    subtask_id: parent_context.subtask_id.clone(),
                    execution_history: parent_context.execution_history.clone(),
                },
                success: false,
                duration_ms,
                files_modified: vec![],
                chain_id,
            })
        }
    }
}

#[cfg(test)]
mod checkpoint_error_tests {
    use super::*;
    use crate::executor_types::SubAgentChainError;

    fn test_agent() -> SubAgentDefinition {
        SubAgentDefinition::new("enumerator", "Enumerator", "test", "test")
    }

    #[test]
    fn checkpointed_generic_inner_error_becomes_addressable_failure_result() {
        let checkpoint_id = uuid::Uuid::new_v4();
        let checkpoint = CheckpointedChainId::default();
        checkpoint.publish(Some(checkpoint_id));

        let result = map_inner_result(
            Err(anyhow::anyhow!("synthetic post-checkpoint failure")),
            &test_agent(),
            &SubAgentContext::default(),
            std::time::Instant::now(),
            &checkpoint,
        )
        .expect("a generic error after a durable snapshot becomes graceful");

        assert!(!result.success);
        assert_eq!(result.chain_id, Some(checkpoint_id));
        assert!(result
            .response
            .contains("synthetic post-checkpoint failure"));
    }

    #[test]
    fn failed_later_finalize_publishes_only_the_previous_checkpoint_id() {
        let checkpoint_id = uuid::Uuid::new_v4();
        let failed_update_id = uuid::Uuid::new_v4();
        let checkpoint = CheckpointedChainId::default();
        checkpoint.publish(Some(checkpoint_id));

        let error = map_inner_result(
            Err(SubAgentChainError::FinalizeFailed {
                chain_id: failed_update_id,
                checkpointed_chain_id: None,
                reason: "synthetic later chain update failure".to_string(),
            }
            .into()),
            &test_agent(),
            &SubAgentContext::default(),
            std::time::Instant::now(),
            &checkpoint,
        )
        .expect_err("finalize failures retain their non-retryable typed contract");

        assert!(matches!(
            error.downcast_ref::<SubAgentChainError>(),
            Some(SubAgentChainError::FinalizeFailed {
                chain_id,
                checkpointed_chain_id: Some(id),
                ..
            }) if *chain_id == failed_update_id && *id == checkpoint_id
        ));
    }

    #[test]
    fn checkpointed_context_limit_error_keeps_typed_failure_semantics() {
        let checkpoint_id = uuid::Uuid::new_v4();
        let checkpoint = CheckpointedChainId::default();
        checkpoint.publish(Some(checkpoint_id));

        let error = map_inner_result(
            Err(SubAgentChainError::ProviderContextLimitExceeded {
                chain_id: Some(checkpoint_id),
                reason: "synthetic context limit".to_string(),
            }
            .into()),
            &test_agent(),
            &SubAgentContext::default(),
            std::time::Instant::now(),
            &checkpoint,
        )
        .expect_err("context-limit failures remain typed and non-retryable");

        assert!(matches!(
            error.downcast_ref::<SubAgentChainError>(),
            Some(SubAgentChainError::ProviderContextLimitExceeded {
                chain_id: Some(id),
                ..
            }) if *id == checkpoint_id
        ));
    }

    #[test]
    fn error_before_initial_checkpoint_remains_an_error() {
        let checkpoint = CheckpointedChainId::default();
        let error = map_inner_result(
            Err(anyhow::anyhow!("synthetic pre-checkpoint failure")),
            &test_agent(),
            &SubAgentContext::default(),
            std::time::Instant::now(),
            &checkpoint,
        )
        .expect_err("an absent durable snapshot must not expose a chain id");

        assert_eq!(error.to_string(), "synthetic pre-checkpoint failure");
    }
}
