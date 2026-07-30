//! Inner orchestrator for a single sub-agent invocation.
//!
//! [`execute_sub_agent_inner`] is the heart of the sub-agent system: it
//! drives the iterate-stream-dispatch loop until either a barrier tool is
//! called, the iteration cap is exceeded, or an error/timeout fires. The
//! one-shot setup phases (prompt assembly, tool list build, chain restore)
//! and one-shot teardown phases (chain persist, final summary) are
//! delegated to dedicated sibling modules so this file can focus on the
//! per-iteration loop.

use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use rig::completion::{AssistantContent, CompletionModel as RigCompletionModel, Message};
use rig::message::{Text, UserContent};
use rig::one_or_many::OneOrMany;
use uuid::Uuid;

use crate::definition::{SubAgentContext, SubAgentDefinition, SubAgentResult};
use crate::executor_helpers::{build_assistant_content, epoch_secs};
use crate::executor_types::{
    cancellation_requested, wait_for_cancelled, SubAgentChainError, SubAgentExecutorContext,
    ToolProvider,
};
use crate::executor_udiff::process_coder_udiff;
use crate::transcript::SubAgentTranscriptWriter;
use golish_core::events::AiEvent;
use golish_core::utils::truncate_str;
use golish_llm_providers::ModelCapabilities;

use super::chain_persist::{
    append_durable_chain_marker, checkpoint_chain, maybe_restore_chain, persist_chain,
};
use super::final_summary::run_final_summary;
use super::history_compaction::compact_history_for_provider;
use super::prompt_assembly::assemble_effective_system_prompt;
use super::response_parsing::{
    dispatch_tool_calls, StageStallGuard, SubmitRepairModeUpdate, STAGE_STALL_THRESHOLD,
};
use super::stream_processing::process_llm_stream;
use super::tool_setup::{build_tool_definitions, validate_closed_candidate_analysis_definition};
use super::CheckpointedChainId;

const MAX_BOUND_STAGE_SUBMIT_REPROMPTS: usize = 2;

fn bound_stage_submit_reprompt(
    bound_worker: bool,
    tools: &[rig::completion::ToolDefinition],
    reprompts_used: usize,
) -> Option<String> {
    if !bound_worker
        || reprompts_used >= MAX_BOUND_STAGE_SUBMIT_REPROMPTS
        || !tools
            .iter()
            .any(|tool| tool.name == "submit_stage_deliverable")
    {
        return None;
    }
    Some(
        "BOUND STAGE SUBMISSION REQUIRED: your previous response ended without calling \
         submit_stage_deliverable, so the deterministic per-organization gate received nothing. \
         Do not narrate, restate the manifest, list keys in prose, or stop with a summary. Your \
         entire next response must be one submit_stage_deliverable tool call. Copy exact \
         server-provided identities directly into compact structured fields and submit now."
            .to_string(),
    )
}

/// Classify provider failures that deterministically mean the request history
/// exceeds the model's input window. Keep this deliberately narrower than
/// generic HTTP 400 handling: malformed requests, auth failures, and rate
/// limits must retain their existing ordinary-failure policy.
fn is_provider_context_limit_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();

    if normalized.contains("context_length_exceeded")
        || normalized.contains("maximum context length")
        || normalized.contains("max context length")
        || normalized.contains("prompt is too long")
        || normalized.contains("prompt too long")
    {
        return true;
    }

    let rate_limited = normalized.contains("rate limit")
        || normalized.contains("tokens per minute")
        || normalized.contains("tokens/min")
        || normalized.contains(" tpm");
    if rate_limited {
        return false;
    }

    let request_token_count = (normalized.contains("request body has")
        || normalized.contains("request messages"))
        && normalized.contains("tokens")
        && (normalized.contains("limit") || normalized.contains("maximum"));
    let explicit_token_overflow = normalized.contains("token")
        && (normalized.contains("exceeds") || normalized.contains("exceeded"))
        && (normalized.contains("context") || normalized.contains("limit"));

    normalized.contains("too many tokens") || request_token_count || explicit_token_overflow
}

fn ensure_bound_worker_lease(ctx: &SubAgentExecutorContext<'_>) -> anyhow::Result<()> {
    let Some(bound) = ctx.bound_worker_chain.as_ref() else {
        return Ok(());
    };
    if bound.lease_is_lost() {
        return Err(SubAgentChainError::BoundWorkerUnavailable {
            worker_run_id: bound.worker_lease.worker_run_id,
            reason: "worker lease was lost before the next provider/tool turn".to_string(),
        }
        .into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_sub_agent_inner<M, P>(
    agent_def: &SubAgentDefinition,
    args: &serde_json::Value,
    parent_context: &SubAgentContext,
    model: &M,
    ctx: SubAgentExecutorContext<'_>,
    tool_provider: &P,
    parent_request_id: &str,
    start_time: std::time::Instant,
    sub_agent_span: &tracing::Span,
    tool_fallback_timeout: Duration,
    idle_timeout: Option<Duration>,
    checkpointed_chain_id: CheckpointedChainId,
) -> Result<SubAgentResult>
where
    M: RigCompletionModel + Sync,
    P: ToolProvider,
{
    let agent_id = &agent_def.id;
    validate_closed_candidate_analysis_definition(agent_def)?;

    let transcript_writer: Option<Arc<SubAgentTranscriptWriter>> = if let (
        Some(base_dir),
        Some(session_id),
    ) =
        (ctx.transcript_base_dir, ctx.session_id)
    {
        match SubAgentTranscriptWriter::new(base_dir, session_id, agent_id, parent_request_id).await
        {
            Ok(writer) => Some(Arc::new(writer)),
            Err(e) => {
                tracing::warn!(
                    "Failed to create sub-agent transcript writer: {}. Continuing without transcript.",
                    e
                );
                None
            }
        }
    } else {
        None
    };

    let last_activity = Arc::new(AtomicU64::new(epoch_secs()));
    let mut files_modified: Vec<String> = vec![];

    let task = args
        .get("task")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Sub-agent call missing 'task' parameter"))?;
    let additional_context = args.get("context").and_then(|v| v.as_str()).unwrap_or("");
    let initial_submit_repair_mode = ctx.initial_submit_repair_mode.clone();

    let sub_context = SubAgentContext {
        original_request: parent_context.original_request.clone(),
        conversation_summary: parent_context.conversation_summary.clone(),
        variables: parent_context.variables.clone(),
        depth: parent_context.depth + 1,
        parent_agent: parent_context.parent_agent.clone(),
        task_id: parent_context.task_id.clone(),
        subtask_id: parent_context.subtask_id.clone(),
        execution_history: parent_context.execution_history.clone(),
    };

    let sub_prompt = if additional_context.is_empty() {
        task.to_string()
    } else {
        format!("{}\n\nAdditional context: {}", task, additional_context)
    };

    let input_truncated = if sub_prompt.len() > 1000 {
        format!("{}...[truncated]", truncate_str(&sub_prompt, 1000))
    } else {
        sub_prompt.clone()
    };
    sub_agent_span.record("langfuse.observation.input", &input_truncated);

    let _ = ctx.event_tx.send(AiEvent::SubAgentStarted {
        agent_id: agent_id.to_string(),
        agent_name: agent_def.name.clone(),
        task: task.to_string(),
        depth: sub_context.depth,
        parent_request_id: parent_request_id.to_string(),
    });
    if let Some(mode) = initial_submit_repair_mode.as_ref() {
        let message = format!(
            "Resuming submit repair: {} Allowed next tools: [{}].",
            mode.model_instruction(),
            mode.allowed_tool_names().join(", ")
        );
        let _ = ctx.event_tx.send(AiEvent::SubAgentTextDelta {
            agent_id: agent_id.to_string(),
            delta: message.clone(),
            accumulated: message,
            parent_request_id: parent_request_id.to_string(),
        });
    }

    let tools = build_tool_definitions(agent_def, &sub_context, &ctx, tool_provider).await;

    let (chain_id, restored_messages): (Option<Uuid>, Vec<Message>) =
        maybe_restore_chain(&ctx, parent_context, agent_id).await?;

    // On `resume`, seed the conversation with the prior chain (incl. tool
    // results / evidence ids) so the worker continues where it left off; then
    // append the new task as the next user turn.
    let mut chat_history: Vec<Message> = restored_messages;
    if !ctx
        .bound_worker_chain
        .as_ref()
        .is_some_and(|bound| bound.initial_prompt_already_checkpointed)
    {
        chat_history.push(Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: sub_prompt.clone(),
            })),
        });
    }

    let mut accumulated_response = String::new();
    let mut iteration = 0;
    let mut bound_stage_submit_reprompts = 0;
    // Stage-stall circuit breaker: a weak worker can re-submit the SAME stage
    // gate BLOCK every turn until it burns the whole iteration cap (observed:
    // 22× "never attempted" on one org). Bail after N identical blocks.
    let mut stall = StageStallGuard::default();
    let mut submit_repair_mode = initial_submit_repair_mode;
    if let Some(mode) = submit_repair_mode.as_ref() {
        chat_history.push(Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: format!(
                    "RESUME REPAIR DIRECTIVE (deterministic): {}\nAllowed next tools: [{}]. \
                     Continue this exact repair path; do not restart discovery.",
                    mode.model_instruction(),
                    mode.allowed_tool_names().join(", ")
                ),
            })),
        });
    }

    // Make a fresh or resumed invocation addressable before the first provider
    // request. The UUID is published only after the provider-valid body update
    // completes; a failed update leaves the shared slot unchanged.
    let durable_chain_id = checkpoint_chain(&ctx, chain_id, &chat_history, agent_id).await?;
    checkpointed_chain_id.publish(durable_chain_id);

    // Prompt-template optimization may itself call the provider. Keep it after
    // the initial body checkpoint so every provider request in this invocation
    // has an already-addressable recovery point.
    ensure_bound_worker_lease(&ctx)?;
    let effective_system_prompt = assemble_effective_system_prompt(
        agent_def,
        task,
        additional_context,
        &ctx,
        parent_request_id,
        model,
    )
    .await;

    loop {
        iteration += 1;
        ensure_bound_worker_lease(&ctx)?;
        if cancellation_requested(ctx.cancelled) {
            let message = "Agent stopped by user".to_string();
            tracing::info!("[sub-agent:{}] cancelled before iteration", agent_id);
            let _ = ctx.event_tx.send(AiEvent::SubAgentError {
                agent_id: agent_id.to_string(),
                error: message.clone(),
                parent_request_id: parent_request_id.to_string(),
            });
            return Ok(SubAgentResult {
                agent_id: agent_id.to_string(),
                response: message,
                context: sub_context,
                success: false,
                duration_ms: start_time.elapsed().as_millis() as u64,
                files_modified,
                chain_id: checkpointed_chain_id.get(),
            });
        }
        // Durable chains and long same-segment workers share the same strict
        // provider-history budget. This runs before *every* model request
        // (including the max-iteration final summary), not only at restore, so
        // newly appended batch results cannot accumulate past the provider
        // limit. It also collapses repeated resume repair directives to the
        // newest bounded projection.
        let (provider_history, compaction) =
            compact_history_for_provider(std::mem::take(&mut chat_history))?;
        chat_history = provider_history;
        if compaction.changed() {
            tracing::info!(
                agent_id = %agent_id,
                iteration,
                before_bytes = compaction.before_bytes,
                after_bytes = compaction.after_bytes,
                compacted_tool_results = compaction.compacted_tool_results,
                collapsed_repair_directives = compaction.collapsed_repair_directives,
                omitted_messages = compaction.omitted_messages,
                "compacted sub-agent history before provider request"
            );
        }
        if iteration > agent_def.max_iterations {
            ensure_bound_worker_lease(&ctx)?;
            run_final_summary(
                agent_def,
                &chat_history,
                &ctx,
                agent_id,
                parent_request_id,
                &mut accumulated_response,
                model,
            )
            .await;
            break;
        }

        // ── Build LLM request ───────────────────────────────────────────
        let caps = ModelCapabilities::detect(ctx.provider_name, ctx.model_name);
        let temperature = if caps.supports_temperature {
            Some(ctx.temperature_override.unwrap_or(0.3) as f64)
        } else {
            tracing::debug!(
                "Model {} does not support temperature parameter in sub-agent, omitting",
                ctx.model_name
            );
            None
        };
        let max_tokens = ctx.max_tokens_override.unwrap_or(8192) as u64;
        let additional_params = ctx
            .top_p_override
            .map(|tp| serde_json::json!({ "top_p": tp }));

        let is_nvidia = ctx.provider_name == "nvidia";
        let (preamble, effective_history) = if is_nvidia {
            let mut h = vec![Message::User {
                content: OneOrMany::one(UserContent::text(&*effective_system_prompt)),
            }];
            h.extend(chat_history.clone());
            (None, h)
        } else {
            (Some(effective_system_prompt.clone()), chat_history.clone())
        };
        let request = rig::completion::CompletionRequest {
            preamble,
            chat_history: OneOrMany::many(effective_history.clone())
                .unwrap_or_else(|_| OneOrMany::one(effective_history[0].clone())),
            documents: vec![],
            tools: tools.clone(),
            temperature,
            max_tokens: Some(max_tokens),
            tool_choice: None,
            additional_params,
            model: None,
            output_schema: None,
        };

        let llm_span = tracing::info_span!(
            parent: sub_agent_span,
            "llm_completion",
            "gen_ai.operation.name" = "chat_completion",
            "gen_ai.request.model" = %ctx.model_name,
            "gen_ai.system" = %ctx.provider_name,
            "gen_ai.usage.prompt_tokens" = tracing::field::Empty,
            "gen_ai.usage.completion_tokens" = tracing::field::Empty,
            "gen_ai.reasoning" = tracing::field::Empty,
            "langfuse.observation.type" = "generation",
            "langfuse.session.id" = ctx.session_id.unwrap_or(""),
            iteration = iteration,
        );
        let _llm_guard = llm_span.enter();

        // ── Stream LLM response ─────────────────────────────────────────
        if let Some(stats) = ctx.api_request_stats {
            stats.record_sent(ctx.provider_name).await;
        }

        ensure_bound_worker_lease(&ctx)?;
        let stream_result = tokio::select! {
            _ = wait_for_cancelled(ctx.cancelled) => {
                tracing::info!("[sub-agent:{}] cancelled before LLM stream started", agent_id);
                let message = "Agent stopped by user".to_string();
                let _ = ctx.event_tx.send(AiEvent::SubAgentError {
                    agent_id: agent_id.to_string(),
                    error: message.clone(),
                    parent_request_id: parent_request_id.to_string(),
                });
                return Ok(SubAgentResult {
                    agent_id: agent_id.to_string(),
                    response: message,
                    context: sub_context,
                    success: false,
                    duration_ms: start_time.elapsed().as_millis() as u64,
                    files_modified,
                    chain_id: checkpointed_chain_id.get(),
                });
            }
            result = model.stream(request) => result,
        };

        let mut stream = match stream_result {
            Ok(s) => {
                if let Some(stats) = ctx.api_request_stats {
                    stats.record_received(ctx.provider_name).await;
                }
                s
            }
            Err(e) => {
                let error = e.to_string();
                let _ = ctx.event_tx.send(AiEvent::SubAgentError {
                    agent_id: agent_id.to_string(),
                    error: error.clone(),
                    parent_request_id: parent_request_id.to_string(),
                });
                if is_provider_context_limit_error(&error) {
                    return Err(SubAgentChainError::ProviderContextLimitExceeded {
                        chain_id: checkpointed_chain_id.get(),
                        reason: error,
                    }
                    .into());
                }
                return Ok(SubAgentResult {
                    agent_id: agent_id.to_string(),
                    response: format!("Error: {error}"),
                    context: sub_context,
                    success: false,
                    duration_ms: start_time.elapsed().as_millis() as u64,
                    files_modified: files_modified.clone(),
                    chain_id: checkpointed_chain_id.get(),
                });
            }
        };

        let supports_thinking_history = caps.supports_thinking_history;

        let quirks =
            golish_llm_providers::resolve_stream_quirks(ctx.provider_name, ctx.model_name, None);
        tracing::debug!(
            "[sub-agent quirks] provider={} model={} reasoning_handling={:?}",
            ctx.provider_name,
            ctx.model_name,
            quirks.reasoning_handling,
        );

        let sr = process_llm_stream(
            &mut stream,
            agent_id,
            parent_request_id,
            ctx.event_tx,
            &last_activity,
            idle_timeout,
            ctx.cancelled,
            &llm_span,
            &quirks,
        )
        .await;

        if sr.cancelled {
            let message = "Agent stopped by user".to_string();
            tracing::info!("[sub-agent:{}] cancelled during LLM stream", agent_id);
            let _ = ctx.event_tx.send(AiEvent::SubAgentError {
                agent_id: agent_id.to_string(),
                error: message.clone(),
                parent_request_id: parent_request_id.to_string(),
            });
            return Ok(SubAgentResult {
                agent_id: agent_id.to_string(),
                response: message,
                context: sub_context,
                success: false,
                duration_ms: start_time.elapsed().as_millis() as u64,
                files_modified,
                chain_id: checkpointed_chain_id.get(),
            });
        }

        if let Some(error) = sr.stream_error {
            let message = format!("Provider stream error: {error}");
            let _ = ctx.event_tx.send(AiEvent::SubAgentError {
                agent_id: agent_id.to_string(),
                error: message.clone(),
                parent_request_id: parent_request_id.to_string(),
            });
            if is_provider_context_limit_error(&error) {
                return Err(SubAgentChainError::ProviderContextLimitExceeded {
                    chain_id: checkpointed_chain_id.get(),
                    reason: error,
                }
                .into());
            }
            return Ok(SubAgentResult {
                agent_id: agent_id.to_string(),
                response: format!("Error: {message}"),
                context: sub_context,
                success: false,
                duration_ms: start_time.elapsed().as_millis() as u64,
                files_modified,
                chain_id: checkpointed_chain_id.get(),
            });
        }

        // ── Handle idle timeout ─────────────────────────────────────────
        if sr.idle_timeout_hit {
            if let Some(idle_dur) = idle_timeout {
                let error_msg = format!(
                    "Sub-agent idle timeout: no activity for {}s",
                    idle_dur.as_secs()
                );
                tracing::warn!("[sub-agent] {}", error_msg);

                let _ = ctx.event_tx.send(AiEvent::SubAgentError {
                    agent_id: agent_id.to_string(),
                    error: error_msg.clone(),
                    parent_request_id: parent_request_id.to_string(),
                });

                return Ok(SubAgentResult {
                    agent_id: agent_id.to_string(),
                    response: format!("Error: {}", error_msg),
                    context: sub_context.clone(),
                    success: false,
                    duration_ms: start_time.elapsed().as_millis() as u64,
                    files_modified: files_modified.clone(),
                    chain_id: checkpointed_chain_id.get(),
                });
            }
        }

        if !sr.text_content.is_empty() {
            accumulated_response.push_str(&sr.text_content);
        }

        // Persist this turn's reasoning + narration to the sub-agent transcript.
        // The per-delta `event_tx` stream above is frontend-only; without this
        // on-disk snapshot the *why* behind each tool batch is unrecoverable
        // offline (only `text_chars`/`thinking_chars` counts hit the logs).
        // Bounded per turn so long reasoning can't balloon `transcript.json`.
        if !sr.thinking_text.is_empty() {
            let snap = truncate_str(&sr.thinking_text, SUBAGENT_PROSE_PERSIST_CAP).to_string();
            persist_subagent_prose(
                &transcript_writer,
                AiEvent::SubAgentReasoning {
                    agent_id: agent_id.to_string(),
                    delta: snap.clone(),
                    accumulated: snap,
                    parent_request_id: parent_request_id.to_string(),
                },
            );
        }
        if !sr.text_content.is_empty() {
            let snap = truncate_str(&sr.text_content, SUBAGENT_PROSE_PERSIST_CAP).to_string();
            persist_subagent_prose(
                &transcript_writer,
                AiEvent::SubAgentTextDelta {
                    agent_id: agent_id.to_string(),
                    delta: snap.clone(),
                    accumulated: snap,
                    parent_request_id: parent_request_id.to_string(),
                },
            );
        }

        if !sr.has_tool_calls {
            let assistant_content = build_assistant_content(
                supports_thinking_history,
                &sr.thinking_text,
                sr.thinking_id.clone(),
                sr.thinking_signature.clone(),
                &sr.text_content,
                &[],
            );
            if assistant_content.is_empty() {
                let message = "Provider returned an empty sub-agent completion".to_string();
                let _ = ctx.event_tx.send(AiEvent::SubAgentError {
                    agent_id: agent_id.to_string(),
                    error: message.clone(),
                    parent_request_id: parent_request_id.to_string(),
                });
                return Ok(SubAgentResult {
                    agent_id: agent_id.to_string(),
                    response: format!("Error: {message}"),
                    context: sub_context,
                    success: false,
                    duration_ms: start_time.elapsed().as_millis() as u64,
                    files_modified,
                    chain_id: checkpointed_chain_id.get(),
                });
            }
            chat_history.push(Message::Assistant {
                id: None,
                content: OneOrMany::many(assistant_content)
                    .expect("non-empty assistant content was checked above"),
            });
            if let Some(directive) = bound_stage_submit_reprompt(
                ctx.bound_worker_chain.is_some(),
                &tools,
                bound_stage_submit_reprompts,
            ) {
                bound_stage_submit_reprompts += 1;
                chat_history.push(Message::User {
                    content: OneOrMany::one(UserContent::Text(Text { text: directive })),
                });
                accumulated_response.clear();
                let durable_chain_id =
                    checkpoint_chain(&ctx, chain_id, &chat_history, agent_id).await?;
                checkpointed_chain_id.publish(durable_chain_id);
                tracing::warn!(
                    target: "harness::hook",
                    agent_id = %agent_id,
                    reprompt = bound_stage_submit_reprompts,
                    "bound stage worker returned prose without a StageDeliverable; continuing the same durable chain"
                );
                continue;
            }
            break;
        }

        // ── Build assistant message for chat history ─────────────────────
        let assistant_content = build_assistant_content(
            supports_thinking_history,
            &sr.thinking_text,
            sr.thinking_id.clone(),
            sr.thinking_signature.clone(),
            &sr.text_content,
            &sr.tool_calls,
        );

        chat_history.push(Message::Assistant {
            id: None,
            content: OneOrMany::many(assistant_content).unwrap_or_else(|_| {
                OneOrMany::one(AssistantContent::Text(Text {
                    text: String::new(),
                }))
            }),
        });

        // ── Dispatch tool calls ─────────────────────────────────────────
        if cancellation_requested(ctx.cancelled) {
            let message = "Agent stopped by user".to_string();
            tracing::info!("[sub-agent:{}] cancelled before tool dispatch", agent_id);
            let _ = ctx.event_tx.send(AiEvent::SubAgentError {
                agent_id: agent_id.to_string(),
                error: message.clone(),
                parent_request_id: parent_request_id.to_string(),
            });
            return Ok(SubAgentResult {
                agent_id: agent_id.to_string(),
                response: message,
                context: sub_context,
                success: false,
                duration_ms: start_time.elapsed().as_millis() as u64,
                files_modified,
                chain_id: checkpointed_chain_id.get(),
            });
        }
        let mut dispatch = dispatch_tool_calls(
            sr.tool_calls,
            agent_def,
            &sub_context,
            &ctx,
            tool_provider,
            model,
            parent_request_id,
            &last_activity,
            tool_fallback_timeout,
            idle_timeout,
            submit_repair_mode.as_ref(),
            &transcript_writer,
            &mut files_modified,
            &llm_span,
        )
        .await;

        if dispatch.cancelled {
            let message = "Agent stopped by user".to_string();
            tracing::info!("[sub-agent:{}] cancelled during tool dispatch", agent_id);
            let _ = ctx.event_tx.send(AiEvent::SubAgentError {
                agent_id: agent_id.to_string(),
                error: message.clone(),
                parent_request_id: parent_request_id.to_string(),
            });
            return Ok(SubAgentResult {
                agent_id: agent_id.to_string(),
                response: message,
                context: sub_context,
                success: false,
                duration_ms: start_time.elapsed().as_millis() as u64,
                files_modified,
                chain_id: checkpointed_chain_id.get(),
            });
        }

        // Persist a complete provider turn before any terminal control-flow
        // branch (submit_result barrier or stage-stall breaker). The assistant
        // tool-call message was already appended above, so breaking first would
        // durable-store an invalid history that no OpenAI-compatible provider
        // can replay.
        let tool_results = std::mem::take(&mut dispatch.tool_results);
        let tool_results = OneOrMany::many(tool_results).map_err(|_| {
            anyhow::anyhow!("tool dispatch returned no result for a tool-call turn")
        })?;
        chat_history.push(Message::User {
            content: tool_results,
        });

        // A completed assistant tool-call batch and its full ToolResult turn are
        // one durable checkpoint. Write them before any barrier/stall branch and
        // before the next provider request so cancellation, stream failure, or
        // an outer timeout cannot roll back tools that already completed.
        // Serialization validates every call/result id and fails closed rather
        // than persisting a dangling or partial batch.
        let durable_chain_id = checkpoint_chain(&ctx, chain_id, &chat_history, agent_id).await?;
        checkpointed_chain_id.publish(durable_chain_id);

        if dispatch.barrier_hit {
            if let Some(resp) = dispatch.barrier_response {
                accumulated_response = resp;
            }
            break;
        }

        match dispatch.submit_repair_update {
            Some(SubmitRepairModeUpdate::Set(mode)) => {
                submit_repair_mode = Some(mode);
            }
            Some(SubmitRepairModeUpdate::Clear) => {
                submit_repair_mode = None;
            }
            None => {}
        }

        // Stage-stall circuit breaker: if submit_stage_deliverable returned the
        // SAME gate BLOCK STAGE_STALL_THRESHOLD times in a row, the worker is
        // making no progress — stop and hand back to the orchestrator instead of
        // burning the rest of the iteration cap. The per-org gate is adjudicated
        // from DB truth regardless, so bailing early only saves wasted LLM turns.
        let streak = stall.record(dispatch.stage_block_signature.clone());
        if streak >= STAGE_STALL_THRESHOLD {
            let reason = dispatch.stage_block_signature.unwrap_or_default();
            tracing::warn!(
                target: "harness::stage_stall",
                agent_id = %agent_id,
                streak,
                reason = %reason,
                "stage submit stalled on an identical BLOCK; breaking sub-agent loop"
            );
            accumulated_response = format!(
                "[stage-stall circuit-breaker] The stage gate returned the same BLOCK {streak}× \
                 with no progress, so this worker stopped re-submitting and handed back to the \
                 orchestrator. Last blocking reason: {reason}"
            );
            break;
        }
    }

    // ── Teardown ────────────────────────────────────────────────────────
    let duration_ms = start_time.elapsed().as_millis() as u64;

    let durable_chain_id =
        persist_chain(&ctx, chain_id, &chat_history, duration_ms, agent_id).await?;
    checkpointed_chain_id.publish(durable_chain_id);

    let final_response = if agent_def.id == "coder" {
        let workspace = ctx.workspace.read().await;
        process_coder_udiff(&accumulated_response, &workspace, &mut files_modified)
    } else {
        accumulated_response.clone()
    };

    // Surface the resumable session handle so the orchestrator can later call
    // this sub-agent again with `resume: "<id>"` to continue THIS exact worker
    // (which keeps its tool runs + evidence ids), instead of starting fresh.
    let final_response = append_durable_chain_marker(final_response, durable_chain_id);

    let _ = ctx.event_tx.send(AiEvent::SubAgentCompleted {
        agent_id: agent_id.to_string(),
        response: final_response.clone(),
        duration_ms,
        parent_request_id: parent_request_id.to_string(),
    });

    // Persist the worker's conclusion to the sub-agent transcript too (the
    // `event_tx` send above only reaches the frontend), so a later agent can
    // read each sub-agent's final answer offline without replaying the UI.
    persist_subagent_prose(
        &transcript_writer,
        AiEvent::SubAgentCompleted {
            agent_id: agent_id.to_string(),
            response: truncate_str(&final_response, SUBAGENT_PROSE_PERSIST_CAP).to_string(),
            duration_ms,
            parent_request_id: parent_request_id.to_string(),
        },
    );

    if !files_modified.is_empty() {
        tracing::info!(
            "[sub-agent] {} modified {} files: {:?}",
            agent_id,
            files_modified.len(),
            files_modified
        );
    }

    let output_truncated = if final_response.len() > 1000 {
        format!("{}...[truncated]", truncate_str(&final_response, 1000))
    } else {
        final_response.clone()
    };
    sub_agent_span.record("langfuse.observation.output", &output_truncated);

    Ok(SubAgentResult {
        agent_id: agent_id.to_string(),
        response: final_response,
        context: sub_context,
        success: true,
        duration_ms,
        files_modified,
        chain_id: durable_chain_id,
    })
}

/// Per-turn byte cap for prose/reasoning persisted to the sub-agent transcript.
/// The frontend receives the full per-delta stream over `event_tx`; the on-disk
/// copy exists only for offline "why did it do that" debugging, so it is bounded
/// to keep `transcript.json` from ballooning on long reasoning turns.
const SUBAGENT_PROSE_PERSIST_CAP: usize = 8000;

/// Append a sub-agent prose / reasoning / completion snapshot to the sub-agent
/// transcript, best-effort and off the hot path (spawned).
///
/// These complement the tool-call events written in `response_parsing.rs`: the
/// reasoning + narration that the model streams to the frontend over `event_tx`
/// is otherwise never written to disk, so a later agent could see *what* tools a
/// sub-agent ran but not *why*. A no-op when no transcript writer is configured.
fn persist_subagent_prose(writer: &Option<Arc<SubAgentTranscriptWriter>>, event: AiEvent) {
    let Some(writer) = writer else {
        return;
    };
    let writer = Arc::clone(writer);
    tokio::spawn(async move {
        if let Err(e) = writer.append(&event).await {
            tracing::warn!("Failed to write sub-agent prose to transcript: {}", e);
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use futures::stream;
    use rig::completion::{
        self, CompletionError, CompletionRequest, CompletionResponse, GetTokenUsage, Usage,
    };
    use rig::streaming::{RawStreamingChoice, RawStreamingToolCall, StreamingCompletionResponse};
    use serde::{Deserialize, Serialize};
    use tokio::sync::{mpsc, RwLock};
    use uuid::Uuid;

    use super::{
        bound_stage_submit_reprompt, is_provider_context_limit_error,
        MAX_BOUND_STAGE_SUBMIT_REPROMPTS,
    };
    use crate::definition::{SubAgentContext, SubAgentDefinition};
    use crate::executor_types::{
        BoundWorkerChainContext, SubAgentChainPersistence, SubAgentExecutorContext, ToolProvider,
    };
    use golish_core::events::AiEvent;
    use golish_tools::ToolRegistry;
    use rig::completion::request::ToolDefinition;

    #[test]
    fn bound_stage_worker_gets_a_bounded_submit_reprompt_only_when_the_tool_is_visible() {
        let tools = vec![ToolDefinition {
            name: "submit_stage_deliverable".to_string(),
            description: "submit".to_string(),
            parameters: serde_json::json!({"type": "object"}),
        }];

        let directive = bound_stage_submit_reprompt(true, &tools, 0)
            .expect("bound stage worker must not terminate on prose before submission");
        assert!(directive.contains("entire next response"));
        assert!(directive.contains("submit_stage_deliverable"));
        assert!(bound_stage_submit_reprompt(false, &tools, 0).is_none());
        assert!(bound_stage_submit_reprompt(true, &[], 0).is_none());
        assert!(
            bound_stage_submit_reprompt(true, &tools, MAX_BOUND_STAGE_SUBMIT_REPROMPTS).is_none()
        );
    }

    #[derive(Clone, Debug, Default, Deserialize, Serialize)]
    struct SingleToolStreamResponse;

    impl GetTokenUsage for SingleToolStreamResponse {
        fn token_usage(&self) -> Option<Usage> {
            None
        }
    }

    #[derive(Clone, Debug)]
    struct SingleToolModel;

    impl completion::CompletionModel for SingleToolModel {
        type Response = SingleToolStreamResponse;
        type StreamingResponse = SingleToolStreamResponse;
        type Client = ();

        fn make(_client: &Self::Client, _model: impl Into<String>) -> Self {
            Self
        }

        async fn completion(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
            panic!("the sub-agent executor test uses only streaming completion")
        }

        async fn stream(
            &self,
            _request: CompletionRequest,
        ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
            let chunks = vec![
                RawStreamingChoice::ToolCall(RawStreamingToolCall {
                    id: "tool-id".to_string(),
                    internal_call_id: "tool-id".to_string(),
                    call_id: Some("provider-call-id".to_string()),
                    name: "web_fetch".to_string(),
                    arguments: serde_json::json!({"url": "https://example.test"}),
                    signature: None,
                    additional_params: None,
                }),
                RawStreamingChoice::FinalResponse(SingleToolStreamResponse),
            ];
            let chunks = stream::iter(chunks.into_iter().map(Ok));
            Ok(StreamingCompletionResponse::stream(Box::pin(chunks)))
        }
    }

    struct CancellingToolProvider {
        cancelled: Arc<AtomicBool>,
    }

    #[async_trait::async_trait]
    impl ToolProvider for CancellingToolProvider {
        fn get_all_tool_definitions(&self) -> Vec<ToolDefinition> {
            vec![ToolDefinition {
                name: "web_fetch".to_string(),
                description: "return a deterministic test result".to_string(),
                parameters: serde_json::json!({"type": "object"}),
            }]
        }

        fn filter_tools_by_allowed(
            &self,
            tools: Vec<ToolDefinition>,
            allowed: &[String],
        ) -> Vec<ToolDefinition> {
            tools
                .into_iter()
                .filter(|tool| allowed.contains(&tool.name))
                .collect()
        }

        async fn execute_web_fetch_tool(
            &self,
            _tool_name: &str,
            _args: &serde_json::Value,
        ) -> (serde_json::Value, bool) {
            self.cancelled.store(true, Ordering::SeqCst);
            (serde_json::json!({"status": "complete"}), true)
        }

        async fn execute_memory_tool(
            &self,
            _tool_name: &str,
            _args: &serde_json::Value,
        ) -> Option<(serde_json::Value, bool)> {
            None
        }

        async fn execute_knowledge_base_tool(
            &self,
            _tool_name: &str,
            _args: &serde_json::Value,
        ) -> Option<(serde_json::Value, bool)> {
            None
        }

        fn normalize_run_pty_cmd_args(&self, args: serde_json::Value) -> serde_json::Value {
            args
        }
    }

    struct RecordingPersistence {
        chain_id: Uuid,
        updates: Mutex<Vec<serde_json::Value>>,
        usage_updates: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl SubAgentChainPersistence for RecordingPersistence {
        async fn chain_create(
            &self,
            _session_id: Uuid,
            _task_id: Option<Uuid>,
            _subtask_id: Option<Uuid>,
            _agent_type: &str,
            _parent_chain_id: Option<Uuid>,
            _model: Option<&str>,
        ) -> anyhow::Result<Uuid> {
            Ok(self.chain_id)
        }

        async fn chain_update(
            &self,
            _id: Uuid,
            chain_json: &serde_json::Value,
        ) -> anyhow::Result<()> {
            self.updates
                .lock()
                .expect("recording mutex")
                .push(chain_json.clone());
            Ok(())
        }

        async fn chain_update_usage(
            &self,
            _id: Uuid,
            _input_tokens: i32,
            _output_tokens: i32,
            _cache_read_tokens: i32,
            _input_cost: f64,
            _output_cost: f64,
            _duration_ms: i32,
        ) -> anyhow::Result<()> {
            self.usage_updates.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn load_prompt_template_overrides(&self) -> Vec<(String, String)> {
            Vec::new()
        }
    }

    #[derive(Clone)]
    struct StreamStartErrorModel {
        persistence: Arc<RecordingPersistence>,
        saw_checkpoint_before_stream: Arc<AtomicBool>,
    }

    #[derive(Clone)]
    struct PromptGenerationOrderModel {
        persistence: Arc<RecordingPersistence>,
        saw_checkpoint_before_completion: Arc<AtomicBool>,
    }

    impl completion::CompletionModel for PromptGenerationOrderModel {
        type Response = SingleToolStreamResponse;
        type StreamingResponse = SingleToolStreamResponse;
        type Client = ();

        fn make(_client: &Self::Client, _model: impl Into<String>) -> Self {
            panic!("test model must be constructed with its recording persistence")
        }

        async fn completion(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
            let checkpoint_exists = !self
                .persistence
                .updates
                .lock()
                .expect("recording mutex")
                .is_empty();
            self.saw_checkpoint_before_completion
                .store(checkpoint_exists, Ordering::SeqCst);
            Err(CompletionError::ProviderError(
                "synthetic prompt-generation failure".to_string(),
            ))
        }

        async fn stream(
            &self,
            _request: CompletionRequest,
        ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
            Err(CompletionError::ProviderError(
                "synthetic provider stream-start failure".to_string(),
            ))
        }
    }

    impl completion::CompletionModel for StreamStartErrorModel {
        type Response = SingleToolStreamResponse;
        type StreamingResponse = SingleToolStreamResponse;
        type Client = ();

        fn make(_client: &Self::Client, _model: impl Into<String>) -> Self {
            panic!("test model must be constructed with its recording persistence")
        }

        async fn completion(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
            panic!("the sub-agent executor test uses only streaming completion")
        }

        async fn stream(
            &self,
            _request: CompletionRequest,
        ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
            let checkpoint_exists = !self
                .persistence
                .updates
                .lock()
                .expect("recording mutex")
                .is_empty();
            self.saw_checkpoint_before_stream
                .store(checkpoint_exists, Ordering::SeqCst);
            Err(CompletionError::ProviderError(
                "synthetic provider stream-start failure".to_string(),
            ))
        }
    }

    #[derive(Clone)]
    struct ProviderCallCounterModel {
        calls: Arc<AtomicUsize>,
    }

    impl completion::CompletionModel for ProviderCallCounterModel {
        type Response = SingleToolStreamResponse;
        type StreamingResponse = SingleToolStreamResponse;
        type Client = ();

        fn make(_client: &Self::Client, _model: impl Into<String>) -> Self {
            panic!("test model must be constructed with its counter")
        }

        async fn completion(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(CompletionError::ProviderError(
                "provider must not be reached".to_string(),
            ))
        }

        async fn stream(
            &self,
            _request: CompletionRequest,
        ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(CompletionError::ProviderError(
                "provider must not be reached".to_string(),
            ))
        }
    }

    #[derive(Clone, Debug)]
    struct PendingStreamModel;

    impl completion::CompletionModel for PendingStreamModel {
        type Response = SingleToolStreamResponse;
        type StreamingResponse = SingleToolStreamResponse;
        type Client = ();

        fn make(_client: &Self::Client, _model: impl Into<String>) -> Self {
            Self
        }

        async fn completion(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
            panic!("the sub-agent executor test uses only streaming completion")
        }

        async fn stream(
            &self,
            _request: CompletionRequest,
        ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn initial_checkpoint_is_published_before_provider_stream_start_failure() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = Arc::new(RwLock::new(temp.path().to_path_buf()));
        let registry = Arc::new(RwLock::new(
            ToolRegistry::new(temp.path().to_path_buf()).await,
        ));
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<AiEvent>();
        let persistence = Arc::new(RecordingPersistence {
            chain_id: Uuid::new_v4(),
            updates: Mutex::new(Vec::new()),
            usage_updates: AtomicUsize::new(0),
        });
        let persistence_backend: Arc<dyn SubAgentChainPersistence> = persistence.clone();
        let saw_checkpoint_before_stream = Arc::new(AtomicBool::new(false));
        let model = StreamStartErrorModel {
            persistence: Arc::clone(&persistence),
            saw_checkpoint_before_stream: Arc::clone(&saw_checkpoint_before_stream),
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let ctx = SubAgentExecutorContext {
            event_tx: &event_tx,
            tool_registry: &registry,
            workspace: &workspace,
            provider_name: "test",
            model_name: "test",
            session_id: Some("stage-run-test"),
            persistence_session_id: Some(Uuid::new_v4()),
            transcript_base_dir: None,
            api_request_stats: None,
            cancelled: Some(&cancelled),
            briefing: None,
            temperature_override: None,
            max_tokens_override: None,
            top_p_override: None,
            chain_persistence: Some(&persistence_backend),
            bound_worker_chain: None,
            sub_agent_registry: None,
            post_shell_hook: None,
            resume: None,
            sub_tool_router: None,
            active_org_id_source: None,
            active_org_id_override: None,
            operation_id: None,
            post_tool_result_hook: None,
            tool_observer: None,
            initial_submit_repair_mode: None,
            stage_tool_guard: None,
            hide_tool_in_stage: None,
        };
        let agent =
            SubAgentDefinition::new("test-agent", "Test", "Test", "Test").with_max_iterations(1);

        let result = super::super::execute_sub_agent(
            &agent,
            &serde_json::json!({"task": "checkpoint before provider work"}),
            &SubAgentContext::default(),
            &model,
            ctx,
            &CancellingToolProvider {
                cancelled: Arc::clone(&cancelled),
            },
            "parent-request",
        )
        .await
        .expect("ordinary provider failure returns a sub-agent result");

        assert!(!result.success);
        assert!(
            saw_checkpoint_before_stream.load(Ordering::SeqCst),
            "the initial provider-valid history must be durable before model.stream"
        );
        assert_eq!(result.chain_id, Some(persistence.chain_id));
        let updates = persistence.updates.lock().expect("recording mutex");
        assert_eq!(
            updates.len(),
            1,
            "only the initial body checkpoint is expected"
        );
        let restored = crate::executor_helpers::deserialize_chat_history(&updates[0])
            .expect("the initial checkpoint is provider-valid history");
        assert_eq!(restored.len(), 1);
        assert_eq!(persistence.usage_updates.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn provider_never_runs_when_prebound_worker_load_is_not_committed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = Arc::new(RwLock::new(temp.path().to_path_buf()));
        let registry = Arc::new(RwLock::new(
            ToolRegistry::new(temp.path().to_path_buf()).await,
        ));
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<AiEvent>();
        let persistence = Arc::new(RecordingPersistence {
            chain_id: Uuid::new_v4(),
            updates: Mutex::new(Vec::new()),
            usage_updates: AtomicUsize::new(0),
        });
        let persistence_backend: Arc<dyn SubAgentChainPersistence> = persistence;
        let provider_calls = Arc::new(AtomicUsize::new(0));
        let model = ProviderCallCounterModel {
            calls: Arc::clone(&provider_calls),
        };
        let persistence_session_id = Uuid::new_v4();
        let bound = BoundWorkerChainContext {
            operation_id: Uuid::new_v4(),
            stage_execution_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            worker_lease: golish_core::WorkerLeaseContext {
                worker_run_id: Uuid::new_v4(),
                stage_run_unit_id: Uuid::new_v4(),
                lease_token: Uuid::new_v4(),
                attempt_epoch: 1,
            },
            candidate_attempt: None,
            candidate_submit_only: false,
            return_on_first_durable_stage_submission: false,
            stage_team_leader: None,
            chain_id: Uuid::new_v4(),
            session_id: persistence_session_id,
            agent_type: "test-agent".to_string(),
            runtime_memory_source: None,
            initial_chain: serde_json::json!([]),
            initial_prompt_already_checkpointed: false,
            checkpoint_version: Arc::new(AtomicI64::new(0)),
            checkpoint_body: Arc::new(std::sync::RwLock::new(serde_json::json!([]))),
            lease_lost: Arc::new(AtomicBool::new(false)),
            mutation_lock: Arc::new(tokio::sync::Mutex::new(())),
            tool_lifecycle: None,
        };
        let ctx = SubAgentExecutorContext {
            event_tx: &event_tx,
            tool_registry: &registry,
            workspace: &workspace,
            provider_name: "test",
            model_name: "test",
            session_id: Some("stage-run-test"),
            persistence_session_id: Some(persistence_session_id),
            transcript_base_dir: None,
            api_request_stats: None,
            cancelled: None,
            briefing: None,
            temperature_override: None,
            max_tokens_override: None,
            top_p_override: None,
            chain_persistence: Some(&persistence_backend),
            bound_worker_chain: Some(bound),
            sub_agent_registry: None,
            post_shell_hook: None,
            resume: Some("latest".to_string()),
            sub_tool_router: None,
            active_org_id_source: None,
            active_org_id_override: None,
            operation_id: None,
            post_tool_result_hook: None,
            tool_observer: None,
            initial_submit_repair_mode: None,
            stage_tool_guard: None,
            hide_tool_in_stage: None,
        };
        let agent =
            SubAgentDefinition::new("test-agent", "Test", "Test", "Test").with_max_iterations(1);

        let error = super::super::execute_sub_agent(
            &agent,
            &serde_json::json!({"task": "must wait for committed bind"}),
            &SubAgentContext::default(),
            &model,
            ctx,
            &CancellingToolProvider {
                cancelled: Arc::new(AtomicBool::new(false)),
            },
            "parent-request",
        )
        .await
        .expect_err("missing exact bound checkpoint must fail closed");

        assert!(error.to_string().contains("prebound stage worker"));
        assert_eq!(provider_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn initial_checkpoint_precedes_prompt_generation_provider_request() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = Arc::new(RwLock::new(temp.path().to_path_buf()));
        let registry = Arc::new(RwLock::new(
            ToolRegistry::new(temp.path().to_path_buf()).await,
        ));
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<AiEvent>();
        let persistence = Arc::new(RecordingPersistence {
            chain_id: Uuid::new_v4(),
            updates: Mutex::new(Vec::new()),
            usage_updates: AtomicUsize::new(0),
        });
        let persistence_backend: Arc<dyn SubAgentChainPersistence> = persistence.clone();
        let saw_checkpoint_before_completion = Arc::new(AtomicBool::new(false));
        let model = PromptGenerationOrderModel {
            persistence: Arc::clone(&persistence),
            saw_checkpoint_before_completion: Arc::clone(&saw_checkpoint_before_completion),
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let ctx = SubAgentExecutorContext {
            event_tx: &event_tx,
            tool_registry: &registry,
            workspace: &workspace,
            provider_name: "test",
            model_name: "test",
            session_id: Some("stage-run-test"),
            persistence_session_id: Some(Uuid::new_v4()),
            transcript_base_dir: None,
            api_request_stats: None,
            cancelled: Some(&cancelled),
            briefing: None,
            temperature_override: None,
            max_tokens_override: None,
            top_p_override: None,
            chain_persistence: Some(&persistence_backend),
            bound_worker_chain: None,
            sub_agent_registry: None,
            post_shell_hook: None,
            resume: None,
            sub_tool_router: None,
            active_org_id_source: None,
            active_org_id_override: None,
            operation_id: None,
            post_tool_result_hook: None,
            tool_observer: None,
            initial_submit_repair_mode: None,
            stage_tool_guard: None,
            hide_tool_in_stage: None,
        };
        let agent = SubAgentDefinition::new("test-agent", "Test", "Test", "fallback")
            .with_prompt_template("generate a worker prompt")
            .with_max_iterations(1);

        let result = super::super::execute_sub_agent(
            &agent,
            &serde_json::json!({"task": "checkpoint before every provider request"}),
            &SubAgentContext::default(),
            &model,
            ctx,
            &CancellingToolProvider {
                cancelled: Arc::clone(&cancelled),
            },
            "parent-request",
        )
        .await
        .expect("provider failures return a graceful sub-agent result");

        assert!(!result.success);
        assert!(
            saw_checkpoint_before_completion.load(Ordering::SeqCst),
            "the initial snapshot must precede prompt-generation completion"
        );
        assert_eq!(result.chain_id, Some(persistence.chain_id));
        assert_eq!(
            persistence.updates.lock().expect("recording mutex").len(),
            1
        );
        assert_eq!(persistence.usage_updates.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn outer_timeout_returns_last_successfully_checkpointed_chain_id() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = Arc::new(RwLock::new(temp.path().to_path_buf()));
        let registry = Arc::new(RwLock::new(
            ToolRegistry::new(temp.path().to_path_buf()).await,
        ));
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<AiEvent>();
        let persistence = Arc::new(RecordingPersistence {
            chain_id: Uuid::new_v4(),
            updates: Mutex::new(Vec::new()),
            usage_updates: AtomicUsize::new(0),
        });
        let persistence_backend: Arc<dyn SubAgentChainPersistence> = persistence.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let ctx = SubAgentExecutorContext {
            event_tx: &event_tx,
            tool_registry: &registry,
            workspace: &workspace,
            provider_name: "test",
            model_name: "test",
            session_id: Some("stage-run-test"),
            persistence_session_id: Some(Uuid::new_v4()),
            transcript_base_dir: None,
            api_request_stats: None,
            cancelled: Some(&cancelled),
            briefing: None,
            temperature_override: None,
            max_tokens_override: None,
            top_p_override: None,
            chain_persistence: Some(&persistence_backend),
            bound_worker_chain: None,
            sub_agent_registry: None,
            post_shell_hook: None,
            resume: None,
            sub_tool_router: None,
            active_org_id_source: None,
            active_org_id_override: None,
            operation_id: None,
            post_tool_result_hook: None,
            tool_observer: None,
            initial_submit_repair_mode: None,
            stage_tool_guard: None,
            hide_tool_in_stage: None,
        };
        let agent = SubAgentDefinition::new("test-agent", "Test", "Test", "Test")
            .with_max_iterations(1)
            .with_timeout(1);

        let result = super::super::execute_sub_agent(
            &agent,
            &serde_json::json!({"task": "checkpoint before waiting forever"}),
            &SubAgentContext::default(),
            &PendingStreamModel,
            ctx,
            &CancellingToolProvider {
                cancelled: Arc::clone(&cancelled),
            },
            "parent-request",
        )
        .await
        .expect("outer timeout returns a graceful sub-agent result");

        assert!(!result.success);
        assert_eq!(result.chain_id, Some(persistence.chain_id));
        assert!(result
            .response
            .contains(&format!("[sub_agent_session_id: {}]", persistence.chain_id)));
        assert_eq!(
            persistence.updates.lock().expect("recording mutex").len(),
            1,
            "timeout must expose the existing snapshot without another async write"
        );
        assert_eq!(persistence.usage_updates.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn completed_tool_batch_is_checkpointed_before_next_iteration_cancellation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = Arc::new(RwLock::new(temp.path().to_path_buf()));
        let registry = Arc::new(RwLock::new(
            ToolRegistry::new(temp.path().to_path_buf()).await,
        ));
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<AiEvent>();
        let cancelled = Arc::new(AtomicBool::new(false));
        let persistence = Arc::new(RecordingPersistence {
            chain_id: Uuid::new_v4(),
            updates: Mutex::new(Vec::new()),
            usage_updates: AtomicUsize::new(0),
        });
        let persistence_backend: Arc<dyn SubAgentChainPersistence> = persistence.clone();
        let tool_provider = CancellingToolProvider {
            cancelled: Arc::clone(&cancelled),
        };
        let ctx = SubAgentExecutorContext {
            event_tx: &event_tx,
            tool_registry: &registry,
            workspace: &workspace,
            provider_name: "test",
            model_name: "test",
            session_id: Some("stage-run-test"),
            persistence_session_id: Some(Uuid::new_v4()),
            transcript_base_dir: None,
            api_request_stats: None,
            cancelled: Some(&cancelled),
            briefing: None,
            temperature_override: None,
            max_tokens_override: None,
            top_p_override: None,
            chain_persistence: Some(&persistence_backend),
            bound_worker_chain: None,
            sub_agent_registry: None,
            post_shell_hook: None,
            resume: None,
            sub_tool_router: None,
            active_org_id_source: None,
            active_org_id_override: None,
            operation_id: None,
            post_tool_result_hook: None,
            tool_observer: None,
            initial_submit_repair_mode: None,
            stage_tool_guard: None,
            hide_tool_in_stage: None,
        };
        let agent = SubAgentDefinition::new("test-agent", "Test", "Test", "Test")
            .with_tools(vec!["web_fetch".to_string()])
            .with_max_iterations(2);

        let result = super::super::execute_sub_agent(
            &agent,
            &serde_json::json!({"task": "checkpoint the complete tool batch"}),
            &SubAgentContext::default(),
            &SingleToolModel,
            ctx,
            &tool_provider,
            "parent-request",
        )
        .await
        .expect("graceful cancellation returns a sub-agent result");

        assert!(!result.success);
        assert_eq!(result.chain_id, Some(persistence.chain_id));
        let updates = persistence.updates.lock().expect("recording mutex");
        assert_eq!(
            updates.len(),
            2,
            "the initial snapshot and completed batch must both be durable"
        );
        let restored = crate::executor_helpers::deserialize_chat_history(&updates[1])
            .expect("the checkpoint must contain a provider-valid complete tool pair");
        assert_eq!(restored.len(), 3);
        assert_eq!(persistence.usage_updates.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn provider_context_limit_classifier_covers_start_and_stream_error_shapes() {
        let positives = [
            r#"HTTP 400 Bad Request: {"error":{"code":"context_length_exceeded"}}"#,
            "This model's maximum context length is 1048565 tokens",
            "prompt is too long for this model",
            "HTTP 400 Bad Request: Request body has 1325879 weighted tokens; limit is 1048565",
            "Request messages contain 1204069 tokens, exceeding the model limit of 1048565",
            "input token count exceeded the context window",
            "too many tokens for the requested context window",
        ];
        for error in positives {
            assert!(
                is_provider_context_limit_error(error),
                "expected context-limit classification for: {error}"
            );
        }
    }

    #[test]
    fn provider_context_limit_classifier_rejects_unrelated_400_and_rate_limits() {
        let negatives = [
            "HTTP 400 Bad Request: invalid tool schema",
            "HTTP 400 Bad Request: authentication failed",
            "HTTP 429: rate limit exceeded: too many tokens per minute",
            "token rate limit exceeded; retry after 30 seconds",
            "max_tokens must be less than or equal to the configured output limit",
            "provider stream closed before final response",
        ];
        for error in negatives {
            assert!(
                !is_provider_context_limit_error(error),
                "unexpected context-limit classification for: {error}"
            );
        }
    }
}
