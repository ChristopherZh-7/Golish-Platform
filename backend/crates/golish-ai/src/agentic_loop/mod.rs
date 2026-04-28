//! Agentic tool loop for LLM execution.
//!
//! This module contains the main agentic loop that handles:
//! - Tool execution with HITL approval
//! - Loop detection and prevention
//! - Context window management
//! - Message history management
//! - Extended thinking (streaming reasoning content)

use anyhow::Result;
use rig::completion::{AssistantContent, Message};
use tracing::Instrument;

// Re-exports kept private to this module for the test child (`mod tests;`),
// which uses `use super::*` to pull these names into scope.
#[cfg(test)]
#[allow(unused_imports)]
use {
    crate::agentic_loop::sub_agent_dispatch::{detect_repetitive_text, partition_tool_calls},
    rig::message::{Text, ToolCall, ToolResult, ToolResultContent, UserContent},
    rig::one_or_many::OneOrMany,
    serde_json::json,
    std::sync::Arc,
};

use super::system_hooks::HookRegistry;
use super::tool_definitions::ToolConfig;
use super::tool_executors::normalize_run_pty_cmd_args;
use golish_context::token_budget::TokenUsage;
use golish_core::events::AiEvent;
use golish_core::utils::truncate_str;
use golish_sub_agents::SubAgentContext;

mod assistant_message;
mod compaction_loop;
mod config;
mod context;
mod entry;
mod first_iter_hooks;
mod helpers;
mod llm_helpers;
mod llm_stream_start;
mod reflector;
mod single_tool_call;
pub(crate) mod sub_agent_dispatch;
mod stream_processor;
mod tool_dispatch;
mod tool_execution;
mod tool_list;
pub mod toolcall_fixer;

use assistant_message::push_assistant_message;
use compaction_loop::{inter_turn_compaction, pre_turn_compaction};
use first_iter_hooks::run_first_iteration_hooks;
use llm_stream_start::start_completion_stream;
use reflector::{maybe_run_reflector, ReflectorOutcome};
use stream_processor::{process_stream, StreamOutcome};
use tool_dispatch::dispatch_tool_calls;
use tool_list::build_tool_list;

use helpers::estimate_message_tokens;
pub use tool_execution::{execute_tool_direct_generic, execute_with_hitl_generic};

/// Maximum number of tool call iterations before stopping
pub const MAX_TOOL_ITERATIONS: usize = 100;

/// Timeout for approval requests in seconds (30 minutes)
pub const APPROVAL_TIMEOUT_SECS: u64 = 1800;

/// Maximum tokens for a single completion request
pub const MAX_COMPLETION_TOKENS: u32 = 10_000;

/// Token threshold above which truncated tool output is further summarized by the LLM.
/// Outputs shorter than this after truncation are passed through as-is.
const SUMMARIZE_THRESHOLD_TOKENS: usize = 2000;

mod stream_retry;

pub mod compaction;
pub use compaction::{
    apply_compaction, get_artifacts_dir, get_artifacts_dir_for, get_summaries_dir,
    get_summaries_dir_for, get_transcript_dir, get_transcript_dir_for, maybe_compact,
    CompactionResult,
};

pub use context::{
    AgenticLoopContext, LoopAccessControl, LoopCaptureContext, LoopEventRefs, LoopLlmRefs,
    OutputClassifier, PostShellHook, TerminalErrorEmitted, ToolExecutionResult,
};
use context::{emit_event, emit_to_frontend};


pub use entry::{run_agentic_loop, run_agentic_loop_generic};
pub use config::AgenticLoopConfig;

/// Unified agentic loop that handles all model types.
///
/// This function replaces both `run_agentic_loop` (Anthropic) and
/// `run_agentic_loop_generic` by using configuration to control behavior.
///
/// # Key Differences from Separate Loops
///
/// 1. **Thinking History**: When `config.capabilities.supports_thinking_history` is true,
///    reasoning content from the model is preserved in the message history
///    (required by Anthropic API when extended thinking is enabled).
///
/// 2. **HITL Approval**: When `config.require_hitl` is true, tool execution
///    requires human-in-the-loop approval (unless auto-approved by policy).
///
/// 3. **Sub-Agent Restrictions**: When `config.is_sub_agent` is true,
///    certain tool restrictions may apply.
///
/// # Arguments
/// * `model` - The completion model to use
/// * `system_prompt` - System prompt for the agent
/// * `initial_history` - Starting conversation history
/// * `sub_agent_context` - Sub-agent execution context (includes depth tracking)
/// * `ctx` - Agent loop context with dependencies
/// * `config` - Configuration controlling behavior
///
/// # Returns
/// Tuple of (response_text, updated_history, token_usage)
///
/// # Example
/// ```ignore
/// use golish_ai::agentic_loop::{run_agentic_loop_unified, AgenticLoopConfig};
///
/// // For Anthropic models (with thinking support)
/// let config = AgenticLoopConfig::main_agent_anthropic();
/// let (response, history, usage) = run_agentic_loop_unified(
///     &model, system_prompt, history, context, &ctx, config
/// ).await?;
///
/// // For generic models (without thinking support)
/// let config = AgenticLoopConfig::main_agent_generic();
/// let (response, history, usage) = run_agentic_loop_unified(
///     &model, system_prompt, history, context, &ctx, config
/// ).await?;
/// ```
pub async fn run_agentic_loop_unified<M>(
    model: &M,
    system_prompt: &str,
    initial_history: Vec<Message>,
    sub_agent_context: SubAgentContext,
    ctx: &AgenticLoopContext<'_>,
    config: AgenticLoopConfig,
) -> Result<(String, Option<String>, Vec<Message>, Option<TokenUsage>)>
where
    M: rig::completion::CompletionModel + Sync,
{
    let supports_thinking = config.capabilities.supports_thinking_history;

    let agent_label = if config.is_sub_agent {
        format!("sub-agent (depth={})", sub_agent_context.depth)
    } else {
        "main-agent".to_string()
    };

    tracing::info!(
        "[{}] Starting agentic loop: provider={}, model={}, thinking={}, temperature={}",
        agent_label,
        ctx.llm.provider_name,
        ctx.llm.model_name,
        supports_thinking,
        config.capabilities.supports_temperature
    );

    // Create root span for the entire agent turn (this becomes the Langfuse trace)
    // All child spans (llm_completion, tool_call) will be nested under this
    // Extract user input from initial history for the trace input
    let trace_input: String = initial_history
        .iter()
        .rev()
        .find_map(|msg| {
            if let Message::User { content } = msg {
                Some(
                    content
                        .iter()
                        .filter_map(|c| {
                            if let rig::message::UserContent::Text(text) = c {
                                Some(text.text.clone())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
            } else {
                None
            }
        })
        .unwrap_or_default();
    let trace_input_truncated = if trace_input.len() > 2000 {
        format!("{}... [truncated]", truncate_str(&trace_input, 2000))
    } else {
        trace_input
    };

    // Create outer trace span (this becomes the Langfuse trace)
    let chat_message_span = tracing::info_span!(
        "chat_message",
        "langfuse.session.id" = ctx.events.session_id.unwrap_or(""),
        "langfuse.observation.input" = %trace_input_truncated,
        "langfuse.observation.output" = tracing::field::Empty,
    );

    // Create agent span as child of trace (this is the main agent observation)
    let agent_span = tracing::info_span!(
        parent: &chat_message_span,
        "agent",
        "langfuse.observation.type" = "agent",
        "langfuse.session.id" = ctx.events.session_id.unwrap_or(""),
        "langfuse.observation.input" = %trace_input_truncated,
        "langfuse.observation.output" = tracing::field::Empty,
        agent_type = %agent_label,
        model = %ctx.llm.model_name,
        provider = %ctx.llm.provider_name,
    );
    // Instrument the main loop body with both spans so they're properly exported to OpenTelemetry.
    // Using nested .instrument() ensures both spans are entered for the duration of the loop.
    let (accumulated_response, accumulated_thinking, chat_history, total_usage) = async {
        // Reset loop detector for new turn
        {
        let mut detector = ctx.access.loop_detector.write().await;
        detector.reset();
    }

    // Create persistent capture context for file event correlation
    let capture_ctx = LoopCaptureContext::new(ctx.sidecar_state);

    // Create hook registry for system hooks
    let hook_registry = HookRegistry::new();

    let tools = build_tool_list(ctx, &sub_agent_context).await;

    let mut chat_history = initial_history;

    // Update context manager with current history
    ctx.context_manager
        .update_from_messages(&chat_history)
        .await;

    // Note: Context compaction is now handled by the summarizer agent
    // which is triggered via should_compact() in the agentic loop

    // Audit: record agent turn start + msg_log for user message
    if let Some(tracker) = ctx.events.db_tracker {
        tracker.audit(
            "agent_turn_start",
            "ai",
            &format!("model={} provider={}", ctx.llm.model_name, ctx.llm.provider_name),
        );
        let user_msg_preview = chat_history
            .last()
            .map(|m| match m {
                rig::message::Message::User { content } => content
                    .iter()
                    .filter_map(|c| match c {
                        rig::message::UserContent::Text(t) => Some(t.text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => String::new(),
            })
            .unwrap_or_default();
        if !user_msg_preview.is_empty() {
            tracker.record_msg_log("user_message", "primary", &user_msg_preview, None);
        }
    }

    let mut accumulated_response = String::new();
    // Thinking history tracking - only used when supports_thinking is true
    let mut accumulated_thinking = String::new();
    let mut total_usage = TokenUsage::default();
    let mut iteration = 0;
    let mut consecutive_no_tool_turns: u32 = 0;
    let mut total_reflector_nudges: u32 = 0;
    // Mutated by `run_first_iteration_hooks` once at iteration 1; see
    // [`first_iter_hooks::FirstIterationOutcome`].
    let mut reflector_active = true;

    loop {
        iteration += 1;

        // Reset compaction state for this turn (preserves last_input_tokens)
        {
            let mut compaction_state = ctx.compaction_state.write().await;
            compaction_state.reset_turn();
        }

        // Compaction at start of turn (using tokens from the previous turn).
        // Important when the agent completes in a single iteration.
        if iteration == 1 {
            pre_turn_compaction(ctx, &mut chat_history).await;
        }

        if let Some(flag) = &ctx.cancelled {
            if flag.load(std::sync::atomic::Ordering::SeqCst) {
                tracing::info!("Agent loop cancelled by user (iteration {})", iteration);
                let _ = ctx.events.event_tx.send(AiEvent::Error {
                    message: "Agent stopped by user".to_string(),
                    error_type: "cancelled".to_string(),
                });
                break;
            }
        }

        if iteration > MAX_TOOL_ITERATIONS {
            // Record max iterations event in Langfuse
            let _max_iter_event = tracing::info_span!(
                parent: &agent_span,
                "max_iterations_reached",
                "langfuse.observation.type" = "event",
                "langfuse.session.id" = ctx.events.session_id.unwrap_or(""),
                max_iterations = MAX_TOOL_ITERATIONS,
            );

            let _ = ctx.events.event_tx.send(AiEvent::Error {
                message: "Maximum tool iterations reached".to_string(),
                error_type: "max_iterations".to_string(),
            });
            break;
        }

        // Compaction check between iterations (after iteration 1).
        if iteration > 1 {
            inter_turn_compaction(ctx, &mut chat_history, iteration, &accumulated_response).await?;
        }

        // First-iteration hooks: synchronous message hooks + memory gatekeeper.
        if iteration == 1 && !config.is_sub_agent {
            let outcome =
                run_first_iteration_hooks(ctx, &hook_registry, &mut chat_history).await;
            reflector_active = outcome.reflector_active;
        }

        // Create span for Langfuse observability (child of agent_span)
        // Token usage fields are Empty and will be recorded when available
        // Note: Langfuse expects prompt_tokens/completion_tokens per GenAI semantic conventions
        // Using both gen_ai.* and langfuse.observation.* for maximum compatibility
        let llm_span = tracing::info_span!(
            parent: &agent_span,
            "llm_completion",
            "gen_ai.operation.name" = "chat_completion",
            "gen_ai.request.model" = %ctx.llm.model_name,
            "gen_ai.system" = %ctx.llm.provider_name,
            "gen_ai.request.temperature" = 0.3_f64,
            "gen_ai.request.max_tokens" = MAX_COMPLETION_TOKENS as i64,
            "langfuse.observation.type" = "generation",
            "langfuse.session.id" = ctx.events.session_id.unwrap_or(""),
            iteration = iteration,
            "gen_ai.usage.prompt_tokens" = tracing::field::Empty,
            "gen_ai.usage.completion_tokens" = tracing::field::Empty,
            // Use both gen_ai.* and langfuse.observation.* for input/output mapping
            "gen_ai.reasoning" = tracing::field::Empty,
            "gen_ai.prompt" = tracing::field::Empty,
            "gen_ai.completion" = tracing::field::Empty,
            "langfuse.observation.input" = tracing::field::Empty,
            "langfuse.observation.output" = tracing::field::Empty,
        );
        // Note: We use explicit parent instead of span.enter() for async compatibility

        // Extract user text for Langfuse prompt tracking
        // Only record actual user text - tool results are already in previous tool spans
        let last_user_text: String = chat_history
            .iter()
            .rev()
            .find_map(|msg| {
                if let Message::User { content } = msg {
                    let text_parts: Vec<String> = content
                        .iter()
                        .filter_map(|c| {
                            if let rig::message::UserContent::Text(text) = c {
                                if !text.text.is_empty() {
                                    Some(text.text.clone())
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        })
                        .collect();
                    if !text_parts.is_empty() {
                        Some(text_parts.join("\n"))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .unwrap_or_default();

        // Only record input if there's actual user text (not just tool results)
        if !last_user_text.is_empty() {
            let prompt_for_span = if last_user_text.len() > 2000 {
                format!("{}... [truncated]", truncate_str(&last_user_text, 2000))
            } else {
                last_user_text
            };
            llm_span.record("gen_ai.prompt", prompt_for_span.as_str());
            llm_span.record("langfuse.observation.input", prompt_for_span.as_str());
        }
        // When continuing after tool results: don't record input, context is in previous spans

        // Diagnostic logging — only traverse history when log level permits
        if tracing::enabled!(tracing::Level::DEBUG) {
            let image_count: usize = chat_history
                .iter()
                .map(|msg| {
                    if let Message::User { content } = msg {
                        content
                            .iter()
                            .filter(|c| matches!(c, rig::message::UserContent::Image(_)))
                            .count()
                    } else {
                        0
                    }
                })
                .sum();
            if image_count > 0 {
                tracing::debug!(
                    "[Unified] Chat history contains {} image(s) across {} messages",
                    image_count,
                    chat_history.len()
                );
            }

            let has_reasoning_in_history = chat_history.iter().any(|m| {
                if let Message::Assistant { content, .. } = m {
                    content
                        .iter()
                        .any(|c| matches!(c, AssistantContent::Reasoning(_)))
                } else {
                    false
                }
            });
            tracing::debug!(
                "[OpenAI Debug] Starting stream: iteration={}, history_len={}, provider={}, has_reasoning_history={}, thinking={}",
                iteration,
                chat_history.len(),
                ctx.llm.provider_name,
                has_reasoning_in_history,
                supports_thinking
            );
        }

        // Proactive token count: estimate tokens BEFORE sending to detect
        // compaction need early. This is a leading indicator vs the lagging
        // provider-reported count after the response.
        {
            let system_prompt_tokens = tokenx_rs::estimate_token_count(system_prompt);
            let history_tokens: usize = chat_history.iter().map(estimate_message_tokens).sum();
            let estimated_input_tokens = (system_prompt_tokens + history_tokens) as u64;

            let mut compaction_state = ctx.compaction_state.write().await;
            compaction_state.update_tokens_estimated(estimated_input_tokens);
            tracing::debug!(
                "[compaction] Pre-call estimate: ~{} tokens (system={}, history={})",
                estimated_input_tokens,
                system_prompt_tokens,
                history_tokens,
            );
        }

        let stream = start_completion_stream(
            ctx,
            &config,
            model,
            system_prompt,
            &chat_history,
            &tools,
            &llm_span,
            &accumulated_response,
        )
        .await?;

        let outcome = match process_stream::<M>(
            stream,
            ctx,
            &chat_history,
            &llm_span,
            iteration,
            supports_thinking,
            &mut accumulated_response,
            &mut accumulated_thinking,
            &mut total_usage,
        )
        .await?
        {
            StreamOutcome::Continue(outcome) => outcome,
            StreamOutcome::BreakAgentLoop => break,
        };

        let stream_processor::StreamProcessOutcome {
            has_tool_calls,
            tool_calls_to_execute,
            text_content,
            thinking_content,
            thinking_signature,
            thinking_id,
        } = outcome;

        push_assistant_message(
            &mut chat_history,
            &text_content,
            &thinking_content,
            &thinking_signature,
            &thinking_id,
            &tool_calls_to_execute,
            has_tool_calls,
            supports_thinking,
            ctx.llm.provider_name,
        );

        // If no tool calls, either invoke the reflector or finish.
        if !has_tool_calls {
            consecutive_no_tool_turns += 1;

            match maybe_run_reflector(
                ctx,
                &sub_agent_context,
                &config,
                &mut chat_history,
                &text_content,
                consecutive_no_tool_turns,
                &mut total_reflector_nudges,
                reflector_active,
                &tools,
            )
            .await
            {
                ReflectorOutcome::Injected => continue,
                ReflectorOutcome::Skipped => break,
            }
        } else {
            consecutive_no_tool_turns = 0;
        }

        dispatch_tool_calls(
            tool_calls_to_execute,
            ctx,
            &capture_ctx,
            model,
            &sub_agent_context,
            &hook_registry,
            &llm_span,
            &mut chat_history,
        )
        .await;
    }

    // Log thinking stats at debug level
    if supports_thinking && !accumulated_thinking.is_empty() {
        tracing::debug!(
            "[Unified] Total thinking content: {} chars",
            accumulated_thinking.len()
        );
    }

    let agent_label = if config.is_sub_agent {
        format!("sub-agent (depth={})", sub_agent_context.depth)
    } else {
        "main-agent".to_string()
    };
    tracing::info!(
        "[{}] Turn complete: provider={}, model={}, tokens={{input={}, output={}, total={}}}",
        agent_label,
        ctx.llm.provider_name,
        ctx.llm.model_name,
        total_usage.input_tokens,
        total_usage.output_tokens,
        total_usage.total()
    );

        Ok::<_, anyhow::Error>((accumulated_response, accumulated_thinking, chat_history, total_usage))
    }
    .instrument(agent_span.clone())
    .instrument(chat_message_span.clone())
    .await?;

    // Record the final output on both trace and agent spans
    let output_for_span = if accumulated_response.len() > 2000 {
        format!("{}... [truncated]", truncate_str(&accumulated_response, 2000))
    } else {
        accumulated_response.clone()
    };
    chat_message_span.record("langfuse.observation.output", output_for_span.as_str());
    agent_span.record("langfuse.observation.output", output_for_span.as_str());

    // Record token usage to DB
    if let Some(tracker) = ctx.events.db_tracker {
        if total_usage.input_tokens > 0 || total_usage.output_tokens > 0 {
            tracker.record_token_usage(
                total_usage.input_tokens,
                total_usage.output_tokens,
                ctx.llm.model_name,
                ctx.llm.provider_name,
                0,
            );
        }
    }

    // Convert accumulated_thinking to Option (None if empty)
    let reasoning = if accumulated_thinking.is_empty() {
        None
    } else {
        Some(accumulated_thinking)
    };

    Ok((
        accumulated_response,
        reasoning,
        chat_history,
        Some(total_usage),
    ))
}

// =============================================================================
// CONTEXT COMPACTION ORCHESTRATION
// =============================================================================

#[cfg(test)]
mod tests;
