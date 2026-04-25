//! Sub-agent execution.
//!
//! [`execute_sub_agent`] is the public entry point: it wraps the inner
//! orchestrator with an overall timeout and uniform error handling. The
//! actual iterate-stream-dispatch loop lives in [`inner`], with one-shot
//! setup/teardown phases delegated to dedicated submodules:
//!
//! - [`prompt_assembly`]: build the effective system prompt (optimized
//!   prompt + briefing + skills + barrier instruction).
//! - [`tool_setup`]: build the tool list (allowed tools + barrier + nested
//!   delegation shims).
//! - [`chain_persist`]: restore/persist the message chain row.
//! - [`final_summary`]: tool-less final call when iteration cap is hit.

use std::time::Duration;

use anyhow::Result;
use rig::completion::CompletionModel as RigCompletionModel;
use tracing::Instrument;

use crate::definition::{SubAgentContext, SubAgentDefinition, SubAgentResult};
pub use crate::executor_types::{SubAgentExecutorContext, ToolProvider, BARRIER_TOOL_NAME};
use golish_core::events::AiEvent;

mod chain_persist;
mod final_summary;
mod inner;
mod prompt_assembly;
mod tool_setup;

/// Execute a sub-agent with the given task and context.
///
/// This is the public entry point. It wraps [`inner::execute_sub_agent_inner`]
/// with an overall timeout and emits a graceful [`AiEvent::SubAgentError`]
/// when the timeout fires.
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

    let timeout_duration = agent_def
        .timeout_secs
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(600));
    let idle_timeout_duration = agent_def.idle_timeout_secs.map(Duration::from_secs);

    // Clone event_tx for timeout error handling (ctx is borrowed, not moved).
    let event_tx_clone = ctx.event_tx.clone();

    match tokio::time::timeout(
        timeout_duration,
        inner::execute_sub_agent_inner(
            agent_def,
            args,
            parent_context,
            model,
            ctx,
            tool_provider,
            parent_request_id,
            start_time,
            &sub_agent_span,
            timeout_duration,
            idle_timeout_duration,
        )
        .instrument(sub_agent_span.clone()),
    )
    .await
    {
        Ok(result) => result,
        Err(_elapsed) => {
            let duration_ms = start_time.elapsed().as_millis() as u64;
            let error_msg = format!(
                "Sub-agent '{}' timed out after {}s",
                agent_def.id,
                timeout_duration.as_secs()
            );
            tracing::warn!("{}", error_msg);

            let _ = event_tx_clone.send(AiEvent::SubAgentError {
                agent_id: agent_def.id.clone(),
                error: error_msg.clone(),
                parent_request_id: parent_request_id.to_string(),
            });

            Ok(SubAgentResult {
                agent_id: agent_def.id.clone(),
                response: format!("Error: {}", error_msg),
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
            })
        }
    }
}
