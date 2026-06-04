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
use crate::executor_types::{SubAgentExecutorContext, ToolProvider};
use crate::executor_udiff::process_coder_udiff;
use crate::transcript::SubAgentTranscriptWriter;
use golish_core::events::AiEvent;
use golish_core::utils::truncate_str;
use golish_llm_providers::ModelCapabilities;

use super::chain_persist::{maybe_restore_chain, persist_chain};
use super::final_summary::run_final_summary;
use super::prompt_assembly::assemble_effective_system_prompt;
use super::response_parsing::dispatch_tool_calls;
use super::stream_processing::process_llm_stream;
use super::tool_setup::build_tool_definitions;

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
) -> Result<SubAgentResult>
where
    M: RigCompletionModel + Sync,
    P: ToolProvider,
{
    let agent_id = &agent_def.id;

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

    let effective_system_prompt = assemble_effective_system_prompt(
        agent_def,
        task,
        additional_context,
        &ctx,
        parent_request_id,
        model,
    )
    .await;

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

    let tools = build_tool_definitions(agent_def, &sub_context, &ctx, tool_provider).await;

    let (chain_id, restored_messages): (Option<Uuid>, Vec<Message>) =
        maybe_restore_chain(&ctx, parent_context, agent_id).await;

    // On `resume`, seed the conversation with the prior chain (incl. tool
    // results / evidence ids) so the worker continues where it left off; then
    // append the new task as the next user turn.
    let mut chat_history: Vec<Message> = restored_messages;
    chat_history.push(Message::User {
        content: OneOrMany::one(UserContent::Text(Text {
            text: sub_prompt.clone(),
        })),
    });

    let mut accumulated_response = String::new();
    let mut iteration = 0;

    loop {
        iteration += 1;
        if iteration > agent_def.max_iterations {
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

        let mut stream = match model.stream(request).await {
            Ok(s) => {
                if let Some(stats) = ctx.api_request_stats {
                    stats.record_received(ctx.provider_name).await;
                }
                s
            }
            Err(e) => {
                let _ = ctx.event_tx.send(AiEvent::SubAgentError {
                    agent_id: agent_id.to_string(),
                    error: e.to_string(),
                    parent_request_id: parent_request_id.to_string(),
                });
                return Ok(SubAgentResult {
                    agent_id: agent_id.to_string(),
                    response: format!("Error: {}", e),
                    context: sub_context,
                    success: false,
                    duration_ms: start_time.elapsed().as_millis() as u64,
                    files_modified: files_modified.clone(),
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
            &llm_span,
            &quirks,
        )
        .await;

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
                });
            }
        }

        if !sr.text_content.is_empty() {
            accumulated_response.push_str(&sr.text_content);
        }

        if !sr.has_tool_calls {
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
        let dispatch = dispatch_tool_calls(
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
            &transcript_writer,
            &mut files_modified,
            &llm_span,
        )
        .await;

        if dispatch.barrier_hit {
            if let Some(resp) = dispatch.barrier_response {
                accumulated_response = resp;
            }
            break;
        }

        chat_history.push(Message::User {
            content: OneOrMany::many(dispatch.tool_results).unwrap_or_else(|_| {
                OneOrMany::one(UserContent::Text(Text {
                    text: "Tool executed".to_string(),
                }))
            }),
        });
    }

    // ── Teardown ────────────────────────────────────────────────────────
    let duration_ms = start_time.elapsed().as_millis() as u64;

    persist_chain(&ctx, chain_id, &chat_history, duration_ms, agent_id).await;

    let final_response = if agent_def.id == "coder" {
        let workspace = ctx.workspace.read().await;
        process_coder_udiff(&accumulated_response, &workspace, &mut files_modified)
    } else {
        accumulated_response.clone()
    };

    // Surface the resumable session handle so the orchestrator can later call
    // this sub-agent again with `resume: "<id>"` to continue THIS exact worker
    // (which keeps its tool runs + evidence ids), instead of starting fresh.
    let final_response = match chain_id {
        Some(cid) => format!("{final_response}\n\n[sub_agent_session_id: {cid}]"),
        None => final_response,
    };

    let _ = ctx.event_tx.send(AiEvent::SubAgentCompleted {
        agent_id: agent_id.to_string(),
        response: final_response.clone(),
        duration_ms,
        parent_request_id: parent_request_id.to_string(),
    });

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
    })
}
