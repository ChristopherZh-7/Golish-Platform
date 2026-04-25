//! Inner orchestrator for a single sub-agent invocation.
//!
//! [`execute_sub_agent_inner`] is the heart of the sub-agent system: it
//! drives the iterate-stream-dispatch loop until either a barrier tool is
//! called, the iteration cap is exceeded, or an error/timeout fires. The
//! one-shot setup phases (prompt assembly, tool list build, chain restore)
//! and one-shot teardown phases (chain persist, final summary) are
//! delegated to dedicated sibling modules so this file can focus on the
//! per-iteration loop.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures::StreamExt;
use rig::completion::{AssistantContent, CompletionModel as RigCompletionModel, Message};
use rig::message::{Text, ToolCall, ToolFunction, ToolResult, ToolResultContent, UserContent};
use rig::one_or_many::OneOrMany;
use rig::streaming::StreamedAssistantContent;
use uuid::Uuid;

use crate::definition::{SubAgentContext, SubAgentDefinition, SubAgentResult};
use crate::executor_helpers::{
    build_assistant_content, epoch_secs, extract_file_path, is_write_tool,
};
use crate::executor_types::{SubAgentExecutorContext, ToolProvider, BARRIER_TOOL_NAME};
use crate::executor_udiff::process_coder_udiff;
use crate::transcript::SubAgentTranscriptWriter;
use golish_core::events::{AiEvent, ToolSource};
use golish_core::utils::truncate_str;
use golish_llm_providers::ModelCapabilities;

use super::chain_persist::{maybe_restore_chain, persist_chain};
use super::final_summary::run_final_summary;
use super::prompt_assembly::assemble_effective_system_prompt;
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
    timeout_duration: Duration,
    idle_timeout: Option<Duration>,
) -> Result<SubAgentResult>
where
    M: RigCompletionModel + Sync,
    P: ToolProvider,
{
    let agent_id = &agent_def.id;

    // Create transcript writer for sub-agent internal events if transcript_base_dir is set
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

    // Idle timeout tracking: stores epoch seconds of last activity
    let last_activity = Arc::new(AtomicU64::new(epoch_secs()));

    // Track files modified by this sub-agent
    let mut files_modified: Vec<String> = vec![];

    // Extract task and additional context from args
    let task = args
        .get("task")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Sub-agent call missing 'task' parameter"))?;
    let additional_context = args.get("context").and_then(|v| v.as_str()).unwrap_or("");

    // Compose the effective system prompt (optimized prompt + briefing + skills + barrier).
    let effective_system_prompt = assemble_effective_system_prompt(
        agent_def,
        task,
        additional_context,
        &ctx,
        parent_request_id,
        model,
    )
    .await;

    // Build the sub-agent context with incremented depth
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

    // Build the prompt for the sub-agent
    let sub_prompt = if additional_context.is_empty() {
        task.to_string()
    } else {
        format!("{}\n\nAdditional context: {}", task, additional_context)
    };

    // Record input on the sub-agent span (truncated for Langfuse, use truncate_str for UTF-8 safety)
    let input_truncated = if sub_prompt.len() > 1000 {
        format!("{}...[truncated]", truncate_str(&sub_prompt, 1000))
    } else {
        sub_prompt.clone()
    };
    sub_agent_span.record("langfuse.observation.input", &input_truncated);

    // Emit sub-agent start event
    let _ = ctx.event_tx.send(AiEvent::SubAgentStarted {
        agent_id: agent_id.to_string(),
        agent_name: agent_def.name.clone(),
        task: task.to_string(),
        depth: sub_context.depth,
        parent_request_id: parent_request_id.to_string(),
    });

    // Build the tool list (filter + dynamic + barrier + nested delegation).
    let tools = build_tool_definitions(agent_def, &sub_context, &ctx, tool_provider).await;

    // Restore conversation chain from DB if available (PentAGI-style persistent chains).
    let chain_id: Option<Uuid> = maybe_restore_chain(&ctx, parent_context, agent_id).await;

    // Build chat history for sub-agent
    let mut chat_history: Vec<Message> = vec![Message::User {
        content: OneOrMany::one(UserContent::Text(Text {
            text: sub_prompt.clone(),
        })),
    }];

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

        // Build request with sub-agent's system prompt
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
        let additional_params = ctx.top_p_override.map(|tp| serde_json::json!({ "top_p": tp }));

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

        // Create LLM completion span for this iteration (Langfuse observability).
        // Explicit parent ensures this appears nested under sub_agent_span in Langfuse.
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

        // Make streaming completion request (streaming works better with Z.AI for tool calls)
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

        // Check if model supports thinking history (for proper conversation history)
        let supports_thinking_history = caps.supports_thinking_history;

        // Process streaming response
        let mut has_tool_calls = false;
        let mut tool_calls_to_execute: Vec<ToolCall> = vec![];
        let mut text_content = String::new();
        let mut thinking_text = String::new();
        let mut thinking_signature: Option<String> = None;
        let mut thinking_id: Option<String> = None;

        // Track tool call state for streaming (tool args come as deltas)
        let mut current_tool_id: Option<String> = None;
        let mut current_tool_call_id: Option<String> = None;
        let mut current_tool_name: Option<String> = None;
        let mut current_tool_args = String::new();

        // Update activity before stream processing
        last_activity.store(epoch_secs(), Ordering::Relaxed);

        // Stream processing loop with idle timeout check
        let mut idle_timeout_hit = false;
        loop {
            let chunk_opt = if let Some(idle_dur) = idle_timeout {
                let last = last_activity.load(Ordering::Relaxed);
                let now = epoch_secs();
                let remaining = idle_dur.as_secs().saturating_sub(now.saturating_sub(last));

                if remaining == 0 {
                    idle_timeout_hit = true;
                    break;
                }

                match tokio::time::timeout(Duration::from_secs(remaining), stream.next()).await {
                    Ok(v) => v,
                    Err(_) => {
                        idle_timeout_hit = true;
                        break;
                    }
                }
            } else {
                stream.next().await
            };

            let Some(chunk_result) = chunk_opt else {
                break; // Stream ended normally
            };

            // Update activity on chunk received
            last_activity.store(epoch_secs(), Ordering::Relaxed);

            match chunk_result {
                Ok(chunk) => match chunk {
                    StreamedAssistantContent::Text(text_msg) => {
                        text_content.push_str(&text_msg.text);
                        let _ = ctx.event_tx.send(AiEvent::SubAgentTextDelta {
                            agent_id: agent_id.to_string(),
                            delta: text_msg.text,
                            accumulated: text_content.clone(),
                            parent_request_id: parent_request_id.to_string(),
                        });
                    }
                    StreamedAssistantContent::Reasoning(reasoning) => {
                        for item in &reasoning.content {
                            if let rig::message::ReasoningContent::Text { text, signature } = item {
                                if !text.is_empty() {
                                    tracing::debug!("[sub-agent] Thinking: {} chars", text.len());
                                    thinking_text.push_str(text);
                                }
                                if signature.is_some() && thinking_signature.is_none() {
                                    thinking_signature = signature.clone();
                                }
                            }
                        }
                        if reasoning.id.is_some() && thinking_id.is_none() {
                            thinking_id = reasoning.id.clone();
                        }
                    }
                    StreamedAssistantContent::ReasoningDelta { id, reasoning } => {
                        if !reasoning.is_empty() {
                            thinking_text.push_str(&reasoning);
                        }
                        // Capture id from delta (OpenAI Responses API sends id in deltas)
                        if id.is_some() && thinking_id.is_none() {
                            thinking_id = id;
                        }
                    }
                    StreamedAssistantContent::ToolCall { tool_call, .. } => {
                        tracing::debug!(
                            "[sub-agent] Received tool call: {} (id: {})",
                            tool_call.function.name,
                            tool_call.id
                        );

                        // Finalize any previous pending tool call first
                        if let (Some(prev_id), Some(prev_name)) =
                            (current_tool_id.take(), current_tool_name.take())
                        {
                            let args = golish_json_repair::parse_tool_args(&current_tool_args);
                            tracing::debug!(
                                "[sub-agent] Finalizing previous tool call: {} with args: {}",
                                prev_name,
                                current_tool_args
                            );
                            has_tool_calls = true;
                            let prev_call_id = current_tool_call_id
                                .take()
                                .unwrap_or_else(|| prev_id.clone());
                            tool_calls_to_execute.push(ToolCall {
                                id: prev_id,
                                call_id: Some(prev_call_id),
                                function: ToolFunction {
                                    name: prev_name,
                                    arguments: args,
                                },
                                signature: None,
                                additional_params: None,
                            });
                            current_tool_args.clear();
                        }

                        // Check if this tool call has complete args (non-streaming case)
                        let has_complete_args = !tool_call.function.arguments.is_null()
                            && tool_call.function.arguments != serde_json::json!({});

                        if has_complete_args {
                            tracing::debug!(
                                "[sub-agent] Tool call has complete args: {:?}",
                                tool_call.function.arguments
                            );
                            has_tool_calls = true;
                            let mut tc = tool_call;
                            if tc.call_id.is_none() {
                                tc.call_id = Some(tc.id.clone());
                            }
                            tool_calls_to_execute.push(tc);
                        } else {
                            tracing::debug!(
                                "[sub-agent] Tool call has empty args, tracking for delta accumulation"
                            );
                            current_tool_id = Some(tool_call.id.clone());
                            // Preserve the OpenAI call_id (e.g. "call_abc") separately from
                            // the item id (e.g. "fc_abc") — these differ in the Responses API.
                            current_tool_call_id = tool_call.call_id.clone();
                            current_tool_name = Some(tool_call.function.name.clone());
                        }
                    }
                    StreamedAssistantContent::ToolCallDelta { id, content, .. } => {
                        // If we don't have a current tool ID but the delta has one, use it
                        if current_tool_id.is_none() && !id.is_empty() {
                            current_tool_id = Some(id);
                        }
                        // Accumulate tool call argument deltas (extract string from enum)
                        if let rig::streaming::ToolCallDeltaContent::Delta(delta) = content {
                            current_tool_args.push_str(&delta);
                        }
                    }
                    StreamedAssistantContent::Final(ref resp) => {
                        // Record token usage on the llm_completion span.
                        use rig::completion::GetTokenUsage;
                        if let Some(usage) = resp.token_usage() {
                            llm_span
                                .record("gen_ai.usage.prompt_tokens", usage.input_tokens as i64);
                            llm_span.record(
                                "gen_ai.usage.completion_tokens",
                                usage.output_tokens as i64,
                            );
                        }

                        // Extract reasoning encrypted_content from OpenAI Responses API.
                        // Required for stateless multi-turn with reasoning models (GPT-5.x, o3, o4-mini).
                        if let Ok(json_value) = serde_json::to_value(resp) {
                            if let Some(encrypted_map) = json_value
                                .get("reasoning_encrypted_content")
                                .and_then(|v| v.as_object())
                            {
                                if let Some(ref tid) = thinking_id {
                                    if let Some(encrypted) =
                                        encrypted_map.get(tid).and_then(|v| v.as_str())
                                    {
                                        tracing::debug!(
                                            "[sub-agent] Found encrypted_content for reasoning item {}: {} bytes",
                                            tid,
                                            encrypted.len()
                                        );
                                        thinking_signature = Some(encrypted.to_string());
                                    }
                                }
                                // Fallback: use single entry if only one reasoning item
                                if thinking_signature.is_none() && encrypted_map.len() == 1 {
                                    if let Some((id, encrypted)) = encrypted_map.iter().next() {
                                        if let Some(encrypted_str) = encrypted.as_str() {
                                            tracing::debug!(
                                                "[sub-agent] Using single encrypted_content for reasoning item {}: {} bytes",
                                                id,
                                                encrypted_str.len()
                                            );
                                            thinking_signature = Some(encrypted_str.to_string());
                                            if thinking_id.is_none() {
                                                thinking_id = Some(id.clone());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
                Err(e) => {
                    tracing::warn!("[sub-agent] Stream error: {}", e);
                }
            }
        }

        // Check if we exited the streaming loop due to idle timeout
        if idle_timeout_hit {
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

        // Finalize any remaining pending tool call after stream ends
        if let (Some(prev_id), Some(prev_name)) = (current_tool_id.take(), current_tool_name.take())
        {
            let args = golish_json_repair::parse_tool_args(&current_tool_args);
            tracing::debug!(
                "[sub-agent] Finalizing final tool call: {} with args: {}",
                prev_name,
                current_tool_args
            );
            has_tool_calls = true;
            let prev_call_id = current_tool_call_id
                .take()
                .unwrap_or_else(|| prev_id.clone());
            tool_calls_to_execute.push(ToolCall {
                id: prev_id,
                call_id: Some(prev_call_id),
                function: ToolFunction {
                    name: prev_name,
                    arguments: args,
                },
                signature: None,
                additional_params: None,
            });
        }

        // Record reasoning/thinking content on the llm_completion span if present.
        if !thinking_text.is_empty() {
            let mut end = thinking_text.len().min(2000);
            while end > 0 && !thinking_text.is_char_boundary(end) {
                end -= 1;
            }
            let reasoning_for_span = if thinking_text.len() > 2000 {
                format!("{}... [truncated]", &thinking_text[..end])
            } else {
                thinking_text.clone()
            };
            llm_span.record("gen_ai.reasoning", reasoning_for_span.as_str());
        }

        if !text_content.is_empty() {
            accumulated_response.push_str(&text_content);
        }

        if !has_tool_calls {
            break;
        }

        // Build assistant content for chat history using helper function
        // (ensures correct ordering: Reasoning -> Text -> ToolCalls)
        let assistant_content = build_assistant_content(
            supports_thinking_history,
            &thinking_text,
            thinking_id.clone(),
            thinking_signature.clone(),
            &text_content,
            &tool_calls_to_execute,
        );

        chat_history.push(Message::Assistant {
            id: None,
            content: OneOrMany::many(assistant_content).unwrap_or_else(|_| {
                OneOrMany::one(AssistantContent::Text(Text {
                    text: String::new(),
                }))
            }),
        });

        // Execute tool calls — check for barrier tool first.
        let mut tool_results: Vec<UserContent> = vec![];
        let mut barrier_hit = false;

        for tool_call in tool_calls_to_execute {
            let tool_name = &tool_call.function.name;

            // Barrier tool: capture structured result and terminate the loop.
            if tool_name == BARRIER_TOOL_NAME {
                let args = &tool_call.function.arguments;
                let result_text = args
                    .get("result")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let summary = args
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                tracing::info!(
                    "[sub-agent] Barrier tool '{}' called: summary='{}', result_len={}",
                    BARRIER_TOOL_NAME,
                    summary,
                    result_text.len()
                );

                accumulated_response = if result_text.is_empty() {
                    summary.to_string()
                } else {
                    result_text
                };

                let _ = ctx.event_tx.send(AiEvent::SubAgentToolResult {
                    agent_id: agent_id.to_string(),
                    tool_name: BARRIER_TOOL_NAME.to_string(),
                    success: true,
                    result: serde_json::json!({ "status": "result submitted" }),
                    request_id: Uuid::new_v4().to_string(),
                    parent_request_id: parent_request_id.to_string(),
                });

                barrier_hit = true;
                break;
            }

            // Nested delegation: dispatch sub_agent_* calls to child sub-agents.
            if tool_name.starts_with("sub_agent_") {
                let delegate_id = &tool_name["sub_agent_".len()..];
                let delegate_task = tool_call
                    .function
                    .arguments
                    .get("task")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                tracing::info!(
                    "[sub-agent:{}] Nested delegation to '{}': {}",
                    agent_id,
                    delegate_id,
                    truncate_str(&delegate_task, 100)
                );

                let delegate_result = if let Some(registry) = ctx.sub_agent_registry {
                    let reg = registry.read().await;
                    if let Some(delegate_def) = reg.get(delegate_id) {
                        let delegate_def = delegate_def.clone();
                        drop(reg);
                        let nested_ctx = SubAgentExecutorContext {
                            event_tx: ctx.event_tx,
                            tool_registry: ctx.tool_registry,
                            workspace: ctx.workspace,
                            provider_name: ctx.provider_name,
                            model_name: ctx.model_name,
                            session_id: ctx.session_id,
                            transcript_base_dir: ctx.transcript_base_dir,
                            api_request_stats: ctx.api_request_stats,
                            briefing: None,
                            temperature_override: delegate_def.temperature,
                            max_tokens_override: delegate_def.max_tokens,
                            top_p_override: delegate_def.top_p,
                            db_pool: ctx.db_pool,
                            sub_agent_registry: ctx.sub_agent_registry,
                        };
                        match Box::pin(super::execute_sub_agent(
                            &delegate_def,
                            &tool_call.function.arguments,
                            &sub_context,
                            model,
                            nested_ctx,
                            tool_provider,
                            parent_request_id,
                        ))
                        .await
                        {
                            Ok(result) => serde_json::json!({
                                "success": result.success,
                                "response": result.response,
                            }),
                            Err(e) => serde_json::json!({
                                "success": false,
                                "error": e.to_string(),
                            }),
                        }
                    } else {
                        serde_json::json!({
                            "error": format!("Unknown delegate agent: {}", delegate_id),
                        })
                    }
                } else {
                    serde_json::json!({
                        "error": "Sub-agent registry not available for nested delegation",
                    })
                };

                let tool_id = tool_call.id.clone();
                let tool_call_id = tool_call
                    .call_id
                    .clone()
                    .unwrap_or_else(|| tool_call.id.clone());
                let result_text = serde_json::to_string(&delegate_result).unwrap_or_default();
                tool_results.push(UserContent::ToolResult(ToolResult {
                    id: tool_id,
                    call_id: Some(tool_call_id),
                    content: OneOrMany::one(ToolResultContent::Text(Text { text: result_text })),
                }));

                last_activity.store(epoch_secs(), Ordering::Relaxed);
                continue;
            }

            let tool_args = if tool_name == "run_pty_cmd" {
                tool_provider.normalize_run_pty_cmd_args(tool_call.function.arguments.clone())
            } else {
                tool_call.function.arguments.clone()
            };
            // For OpenAI Responses API, the actual call ID is in call_id field.
            // For Chat Completions API, call_id is None and we use id.
            let tool_id = tool_call.id.clone();
            let tool_call_id = tool_call
                .call_id
                .clone()
                .unwrap_or_else(|| tool_call.id.clone());

            // Emit tool request event
            let request_id = Uuid::new_v4().to_string();
            let tool_request_event = AiEvent::SubAgentToolRequest {
                agent_id: agent_id.to_string(),
                tool_name: tool_name.to_string(),
                args: tool_args.clone(),
                request_id: request_id.clone(),
                parent_request_id: parent_request_id.to_string(),
            };
            let _ = ctx.event_tx.send(tool_request_event.clone());

            // Write to sub-agent transcript (internal events go to separate file)
            if let Some(ref writer) = transcript_writer {
                let writer = Arc::clone(writer);
                let event = tool_request_event;
                tokio::spawn(async move {
                    if let Err(e) = writer.append(&event).await {
                        tracing::warn!("Failed to write to sub-agent transcript: {}", e);
                    }
                });
            }

            // Create tool call span (Langfuse observability)
            let args_for_span =
                serde_json::to_string(&tool_args).unwrap_or_else(|_| "{}".to_string());
            let args_truncated = if args_for_span.chars().count() > 500 {
                format!("{}...[truncated]", truncate_str(&args_for_span, 500))
            } else {
                args_for_span
            };
            let tool_span = tracing::info_span!(
                parent: &llm_span,
                "tool_call",
                "otel.name" = %tool_name,
                "langfuse.span.name" = %tool_name,
                "langfuse.observation.type" = "tool",
                "langfuse.session.id" = ctx.session_id.unwrap_or(""),
                tool.name = %tool_name,
                tool.id = %tool_id,
                "langfuse.observation.input" = %args_truncated,
                "langfuse.observation.output" = tracing::field::Empty,
                success = tracing::field::Empty,
            );
            let _tool_guard = tool_span.enter();

            // Execute the tool with a timeout guard.
            let tool_timeout = idle_timeout.unwrap_or(timeout_duration);
            let tool_result = tokio::time::timeout(tool_timeout, async {
                if tool_name == "web_fetch" {
                    tool_provider
                        .execute_web_fetch_tool(tool_name, &tool_args)
                        .await
                } else if let Some(result) = tool_provider
                    .execute_memory_tool(tool_name, &tool_args)
                    .await
                {
                    result
                } else if tool_name == "run_pty_cmd" || tool_name == "run_command" {
                    let command = tool_args.get("command").and_then(|c| c.as_str()).unwrap_or("");
                    let cwd = tool_args.get("cwd").and_then(|c| c.as_str());
                    let timeout_secs = tool_args.get("timeout").and_then(|t| t.as_u64()).unwrap_or(120);
                    let workspace = ctx.workspace.read().await;

                    let (chunk_tx, mut chunk_rx) = tokio::sync::mpsc::channel::<golish_shell_exec::OutputChunk>(64);

                    let event_tx = ctx.event_tx.clone();
                    let chunk_request_id = request_id.clone();
                    let chunk_tool_name = tool_name.to_string();
                    let chunk_agent_id = agent_id.to_string();
                    let chunk_agent_name = agent_def.name.clone();
                    tokio::spawn(async move {
                        while let Some(chunk) = chunk_rx.recv().await {
                            let _ = event_tx.send(AiEvent::ToolOutputChunk {
                                request_id: chunk_request_id.clone(),
                                tool_name: chunk_tool_name.clone(),
                                chunk: chunk.data,
                                stream: chunk.stream.as_str().to_string(),
                                source: ToolSource::SubAgent {
                                    agent_id: chunk_agent_id.clone(),
                                    agent_name: chunk_agent_name.clone(),
                                },
                            });
                        }
                    });

                    match golish_shell_exec::execute_streaming(
                        command, cwd, timeout_secs, &workspace, None, chunk_tx,
                    ).await {
                        Ok(r) => {
                            let ok = r.exit_code == 0;
                            let mut v = serde_json::json!({
                                "stdout": r.stdout,
                                "stderr": r.stderr,
                                "exit_code": r.exit_code,
                                "command": command,
                            });
                            if let Some(c) = cwd {
                                v["cwd"] = serde_json::json!(c);
                            }
                            if !ok {
                                let err_detail = if r.stderr.is_empty() { &r.stdout } else { &r.stderr };
                                v["error"] = serde_json::json!(format!(
                                    "Command exited with code {}: {}", r.exit_code, err_detail
                                ));
                            }
                            if r.timed_out {
                                v["timeout"] = serde_json::json!(true);
                            }
                            (v, ok)
                        }
                        Err(e) => (serde_json::json!({ "error": e.to_string() }), false),
                    }
                } else {
                    let registry = ctx.tool_registry.read().await;
                    let result = registry.execute_tool(tool_name, tool_args.clone()).await;

                    match &result {
                        Ok(v) => (v.clone(), true),
                        Err(e) => (serde_json::json!({ "error": e.to_string() }), false),
                    }
                }
            })
            .await;

            let (result_value, success) = match tool_result {
                Ok(result) => result,
                Err(_) => {
                    let error_msg = format!(
                        "Sub-agent tool '{}' timed out after {}s",
                        tool_name,
                        tool_timeout.as_secs()
                    );
                    tracing::warn!("[sub-agent] {}", error_msg);
                    let _ = ctx.event_tx.send(AiEvent::SubAgentError {
                        agent_id: agent_id.to_string(),
                        error: error_msg.clone(),
                        parent_request_id: parent_request_id.to_string(),
                    });
                    (serde_json::json!({ "error": error_msg }), false)
                }
            };

            // Auto-detect and store structured pentest output for shell-style tools.
            if success && (tool_name == "run_pty_cmd" || tool_name == "run_command") {
                if let Some(db_pool) = ctx.db_pool {
                    let pool = Arc::clone(db_pool);
                    let cmd = result_value
                        .get("command")
                        .and_then(|c| c.as_str())
                        .unwrap_or("")
                        .to_string();
                    let stdout = result_value
                        .get("stdout")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    let pp = {
                        let ws = ctx.workspace.read().await;
                        ws.to_string_lossy().to_string()
                    };
                    tokio::spawn(async move {
                        let _ = golish_pentest::output_store::maybe_detect_and_store(
                            &pool, &cmd, &stdout, Some(&pp),
                        )
                        .await;
                    });
                }
            }

            // Record tool result on span
            let result_str = serde_json::to_string(&result_value).unwrap_or_default();
            let result_truncated = if result_str.chars().count() > 500 {
                format!("{}...[truncated]", truncate_str(&result_str, 500))
            } else {
                result_str
            };
            tool_span.record("langfuse.observation.output", &result_truncated);
            tool_span.record("success", success);

            // Emit tool result event
            let tool_result_event = AiEvent::SubAgentToolResult {
                agent_id: agent_id.to_string(),
                tool_name: tool_name.to_string(),
                success,
                result: result_value.clone(),
                request_id: request_id.clone(),
                parent_request_id: parent_request_id.to_string(),
            };
            let _ = ctx.event_tx.send(tool_result_event.clone());

            // Update idle timeout activity tracker after tool execution
            last_activity.store(epoch_secs(), Ordering::Relaxed);

            // Write to sub-agent transcript (internal events go to separate file)
            if let Some(ref writer) = transcript_writer {
                let writer = Arc::clone(writer);
                let event = tool_result_event;
                tokio::spawn(async move {
                    if let Err(e) = writer.append(&event).await {
                        tracing::warn!("Failed to write to sub-agent transcript: {}", e);
                    }
                });
            }

            // Track files modified by write tools
            if success && is_write_tool(tool_name) {
                if let Some(file_path) = extract_file_path(tool_name, &tool_args) {
                    if !files_modified.contains(&file_path) {
                        tracing::debug!(
                            "[sub-agent] Tracking modified file: {} (tool: {})",
                            file_path,
                            tool_name
                        );
                        files_modified.push(file_path);
                    }
                }
            }

            let result_text = serde_json::to_string(&result_value).unwrap_or_default();
            tool_results.push(UserContent::ToolResult(ToolResult {
                id: tool_id,
                call_id: Some(tool_call_id),
                content: OneOrMany::one(ToolResultContent::Text(Text { text: result_text })),
            }));
        }

        // If the barrier tool was hit, break out of the loop immediately
        if barrier_hit {
            break;
        }

        chat_history.push(Message::User {
            content: OneOrMany::many(tool_results).unwrap_or_else(|_| {
                OneOrMany::one(UserContent::Text(Text {
                    text: "Tool executed".to_string(),
                }))
            }),
        });
    }

    let duration_ms = start_time.elapsed().as_millis() as u64;

    // Persist conversation chain to DB for cross-invocation context retention
    persist_chain(&ctx, chain_id, &chat_history, duration_ms, agent_id).await;

    let final_response = if agent_def.id == "coder" {
        let workspace = ctx.workspace.read().await;
        process_coder_udiff(&accumulated_response, &workspace, &mut files_modified)
    } else {
        accumulated_response.clone()
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

    // Record output on the sub-agent span (truncated for Langfuse, use truncate_str for UTF-8 safety)
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
