//! Agentic tool loop for LLM execution.
//!
//! This module contains the main agentic loop that handles:
//! - Tool execution with HITL approval
//! - Loop detection and prevention
//! - Context window management
//! - Message history management
//! - Extended thinking (streaming reasoning content)

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use futures::StreamExt;
use rig::completion::{
    AssistantContent, GetTokenUsage, Message,
};
use rig::message::{
    Reasoning, ReasoningContent, Text, ToolCall, ToolResult, ToolResultContent, UserContent,
};
use rig::one_or_many::OneOrMany;
use rig::streaming::StreamedAssistantContent;
use serde_json::json;
use tokio::sync::{mpsc, oneshot, RwLock};
use tracing::Instrument;

use golish_tools::ToolRegistry;

use super::system_hooks::{format_system_hooks, HookRegistry, MessageHookContext, PostToolContext};
use super::tool_definitions::{
    get_all_tool_definitions_with_config, get_ask_human_tool_definition,
    get_run_command_tool_definition, get_sub_agent_tool_definitions, sanitize_schema, ToolConfig,
};
use super::tool_executors::normalize_run_pty_cmd_args;
use crate::hitl::ApprovalRecorder;
use crate::indexer::IndexerState;
use crate::loop_detection::LoopDetector;
use crate::tool_policy::ToolPolicyManager;
use golish_context::token_budget::TokenUsage;
use golish_context::{CompactionState, ContextManager};
use golish_core::events::AiEvent;
use golish_core::hitl::ApprovalDecision;
use golish_core::runtime::GolishRuntime;
use golish_core::utils::truncate_str;
use golish_core::ApiRequestStats;
use golish_sidecar::{CaptureContext, SidecarState};
use golish_sub_agents::{SubAgentContext, SubAgentRegistry, MAX_AGENT_DEPTH};

use crate::event_coordinator::CoordinatorHandle;

mod config;
mod context;
mod entry;
mod helpers;
mod llm_helpers;
mod single_tool_call;
use single_tool_call::execute_single_tool_call;
pub(crate) mod sub_agent_dispatch;
mod tool_execution;
pub mod toolcall_fixer;

use helpers::estimate_message_tokens;
use sub_agent_dispatch::{detect_repetitive_text, partition_tool_calls};
pub use tool_execution::{
    execute_tool_direct_generic, execute_with_hitl_generic,
};

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
use stream_retry::*;

pub mod compaction;
pub use compaction::{
    apply_compaction, get_artifacts_dir, get_artifacts_dir_for, get_summaries_dir,
    get_summaries_dir_for, get_transcript_dir, get_transcript_dir_for, maybe_compact,
    CompactionResult,
};

pub use context::{
    AgenticLoopContext, LoopCaptureContext, TerminalErrorEmitted, ToolExecutionResult,
};
use context::{emit_event, emit_to_frontend, is_cancelled};


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
        ctx.provider_name,
        ctx.model_name,
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
        "langfuse.session.id" = ctx.session_id.unwrap_or(""),
        "langfuse.observation.input" = %trace_input_truncated,
        "langfuse.observation.output" = tracing::field::Empty,
    );

    // Create agent span as child of trace (this is the main agent observation)
    let agent_span = tracing::info_span!(
        parent: &chat_message_span,
        "agent",
        "langfuse.observation.type" = "agent",
        "langfuse.session.id" = ctx.session_id.unwrap_or(""),
        "langfuse.observation.input" = %trace_input_truncated,
        "langfuse.observation.output" = tracing::field::Empty,
        agent_type = %agent_label,
        model = %ctx.model_name,
        provider = %ctx.provider_name,
    );
    // Instrument the main loop body with both spans so they're properly exported to OpenTelemetry.
    // Using nested .instrument() ensures both spans are entered for the duration of the loop.
    let (accumulated_response, accumulated_thinking, chat_history, total_usage) = async {
        // Reset loop detector for new turn
        {
        let mut detector = ctx.loop_detector.write().await;
        detector.reset();
    }

    // Create persistent capture context for file event correlation
    let capture_ctx = LoopCaptureContext::new(ctx.sidecar_state);

    // Create hook registry for system hooks
    let hook_registry = HookRegistry::new();

    // Build the tool list based on execution mode.
    // Task mode: delegation-only (sub-agents + planning + ask_human) — matches PentAGI primary agent.
    // Chat mode: full tool set (file, shell, web, sub-agents, etc.)
    let is_task_primary = ctx.execution_mode.is_task() && sub_agent_context.depth == 0;

    let is_task_subtask = ctx.execution_mode.is_task() && sub_agent_context.depth > 0;

    let mut tools: Vec<rig::completion::ToolDefinition> = if is_task_primary {
        tracing::info!("[Task mode] Primary agent: delegation-only tools (no direct environment access)");
        // Only planning tool from the static catalog
        let plan_tool = get_all_tool_definitions_with_config(ctx.tool_config)
            .into_iter()
            .filter(|t| t.name == "update_plan")
            .collect::<Vec<_>>();
        plan_tool
    } else {
        // Chat mode or sub-agent execution: full tool set
        let mut t = get_all_tool_definitions_with_config(ctx.tool_config);
        t.push(get_run_command_tool_definition());
        // Subtask agents in Task mode must not call update_plan — only the
        // orchestrator's refiner phase manages plan modifications.
        if is_task_subtask {
            t.retain(|tool| tool.name != "update_plan");
        }
        t
    };

    // Always add ask_human barrier tool (HITL: AI asks user for input)
    tools.push(get_ask_human_tool_definition());

    if !is_task_primary {
        // Add any additional tools (e.g., SWE-bench test tool, MCP tools)
        tools.extend(ctx.additional_tool_definitions.iter().cloned());
    }

    if !is_task_primary {
        // Add dynamically registered tools from the registry (Tavily, PTY interactive, pentest, etc.)
        let registry = ctx.tool_registry.read().await;
        let registry_tools = registry.get_tool_definitions();
        drop(registry);

        let existing_names: HashSet<String> = tools.iter().map(|t| t.name.clone()).collect();

        for tool in registry_tools {
            if existing_names.contains(&tool.name) {
                continue;
            }

            let always_include = tool.name.starts_with("pentest_");
            let tavily_enabled = tool.name.starts_with("tavily_")
                && ctx.tool_config.is_tool_enabled(&tool.name);

            if always_include || tavily_enabled {
                tools.push(rig::completion::ToolDefinition {
                    name: tool.name,
                    description: tool.description,
                    parameters: sanitize_schema(tool.parameters),
                });
            }
        }
    } else {
        // Task-mode primary: add knowledge/memory tools for context retrieval
        let registry = ctx.tool_registry.read().await;
        let registry_tools = registry.get_tool_definitions();
        drop(registry);

        for tool in registry_tools {
            let is_knowledge = tool.name == "search_knowledge_base"
                || tool.name == "read_knowledge"
                || tool.name == "search_memories"
                || tool.name == "query_target_data";
            if is_knowledge {
                tools.push(rig::completion::ToolDefinition {
                    name: tool.name,
                    description: tool.description,
                    parameters: sanitize_schema(tool.parameters),
                });
            }
        }
    }

    // Only add sub-agent tools if we're not at max depth
    // Sub-agents are controlled by the registry, not the tool config
    if sub_agent_context.depth < MAX_AGENT_DEPTH - 1 {
        let registry = ctx.sub_agent_registry.read().await;
        tools.extend(get_sub_agent_tool_definitions(&registry).await);
    }

    tracing::debug!(
        "Available tools (unified loop, mode={}, depth={}): {:?}",
        ctx.execution_mode,
        sub_agent_context.depth,
        tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>()
    );

    let mut chat_history = initial_history;

    // Update context manager with current history
    ctx.context_manager
        .update_from_messages(&chat_history)
        .await;

    // Note: Context compaction is now handled by the summarizer agent
    // which is triggered via should_compact() in the agentic loop

    // Audit: record agent turn start + msg_log for user message
    if let Some(tracker) = ctx.db_tracker {
        tracker.audit(
            "agent_turn_start",
            "ai",
            &format!("model={} provider={}", ctx.model_name, ctx.provider_name),
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
    // Whether the gatekeeper decided memory search is warranted (controls memory-first hook).
    let mut _gatekeeper_wants_memory = false;
    // Whether the reflector should nudge the agent when it produces text without tool calls.
    // Defaults to true — decoupled from gatekeeper so pentest/task prompts always get
    // reflector coverage. Only set to false for trivial messages (greetings, acks).
    let mut reflector_active = true;

    loop {
        iteration += 1;

        // Reset compaction state for this turn (preserves last_input_tokens)
        {
            let mut compaction_state = ctx.compaction_state.write().await;
            compaction_state.reset_turn();
        }

        // Check for compaction at start of turn (using tokens from previous turn)
        // This is important when the agent completes in a single iteration
        if iteration == 1 {
            {
                let compaction_state = ctx.compaction_state.read().await;
                if compaction_state.last_input_tokens.is_some() {
                    tracing::info!(
                        "[compaction] Pre-turn check - tokens: {:?}, using_heuristic: {}",
                        compaction_state.last_input_tokens,
                        compaction_state.using_heuristic
                    );
                }
            }

            if let Some(session_id) = ctx.session_id {
                match maybe_compact(ctx, session_id, &mut chat_history).await {
                    Ok(Some(result)) => {
                        if result.success {
                            let _ = ctx.event_tx.send(AiEvent::CompactionCompleted {
                                tokens_before: result.tokens_before,
                                messages_before: result.messages_before,
                                messages_after: chat_history.len(),
                                summary_length: result.summary.as_ref().map(|s| s.len()).unwrap_or(0),
                                summary: result.summary.clone(),
                                summarizer_input: result.summarizer_input.clone(),
                            });
                            ctx.context_manager
                                .update_from_messages(&chat_history)
                                .await;
                        } else {
                            let _ = ctx.event_tx.send(AiEvent::CompactionFailed {
                                tokens_before: result.tokens_before,
                                messages_before: result.messages_before,
                                error: result.error.clone().unwrap_or_else(|| "Unknown error".to_string()),
                                summarizer_input: result.summarizer_input.clone(),
                            });
                        }
                    }
                    Ok(None) => {} // No compaction needed
                    Err(e) => {
                        tracing::error!("[compaction] Pre-turn compaction error: {}", e);
                    }
                }
            }
        }

        if let Some(flag) = &ctx.cancelled {
            if flag.load(std::sync::atomic::Ordering::SeqCst) {
                tracing::info!("Agent loop cancelled by user (iteration {})", iteration);
                let _ = ctx.event_tx.send(AiEvent::Error {
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
                "langfuse.session.id" = ctx.session_id.unwrap_or(""),
                max_iterations = MAX_TOOL_ITERATIONS,
            );

            let _ = ctx.event_tx.send(AiEvent::Error {
                message: "Maximum tool iterations reached".to_string(),
                error_type: "max_iterations".to_string(),
            });
            break;
        }

        // Check for context compaction need (between turns, after iteration 1)
        if iteration > 1 {
            // Log compaction state at start of each iteration
            {
                let compaction_state = ctx.compaction_state.read().await;
                tracing::info!(
                    "[compaction] Iteration {} - tokens: {:?}, using_heuristic: {}, attempted: {}",
                    iteration,
                    compaction_state.last_input_tokens,
                    compaction_state.using_heuristic,
                    compaction_state.attempted_this_turn
                );
            }

            if let Some(session_id) = ctx.session_id {
                // Check if compaction is needed and perform it if so
                match maybe_compact(ctx, session_id, &mut chat_history).await {
                    Ok(Some(result)) => {
                        if result.success {
                            // Emit success event
                            let _ = ctx.event_tx.send(AiEvent::CompactionCompleted {
                                tokens_before: result.tokens_before,
                                messages_before: result.messages_before,
                                messages_after: chat_history.len(),
                                summary_length: result.summary.as_ref().map(|s| s.len()).unwrap_or(0),
                                summary: result.summary.clone(),
                                summarizer_input: result.summarizer_input.clone(),
                            });

                            // Update context manager with new (compacted) history
                            ctx.context_manager
                                .update_from_messages(&chat_history)
                                .await;
                        } else {
                            // Emit failure event
                            let _ = ctx.event_tx.send(AiEvent::CompactionFailed {
                                tokens_before: result.tokens_before,
                                messages_before: result.messages_before,
                                error: result.error.clone().unwrap_or_else(|| "Unknown error".to_string()),
                                summarizer_input: result.summarizer_input.clone(),
                            });

                            // Check if we're still over the limit after failed compaction
                            let compaction_state = ctx.compaction_state.read().await;
                            let check = ctx
                                .context_manager
                                .should_compact(&compaction_state, ctx.model_name);
                            drop(compaction_state);

                            if check.should_compact {
                                // We needed compaction but it failed, and we're still over limit
                                tracing::error!(
                                    "[compaction] Failed and context still exceeded: {} tokens",
                                    check.current_tokens
                                );
                                let _ = ctx.event_tx.send(AiEvent::Error {
                                    message: format!(
                                        "Context compaction failed and limit exceeded ({} tokens). {}",
                                        check.current_tokens,
                                        result.error.unwrap_or_else(|| "Unknown error".to_string())
                                    ),
                                    error_type: "compaction_failed".to_string(),
                                });
                                return Err(TerminalErrorEmitted::with_partial_state(
                                    "Context compaction failed and limit exceeded",
                                    (!accumulated_response.is_empty())
                                        .then(|| accumulated_response.clone()),
                                    Some(chat_history.clone()),
                                )
                                .into());
                            }
                        }
                    }
                    Ok(None) => {
                        // No compaction needed, continue normally
                    }
                    Err(e) => {
                        // Error checking compaction (non-fatal, log and continue)
                        tracing::warn!("[compaction] Error during compaction check: {}", e);
                    }
                }
            }
        }

        // Fire message hooks and memory gatekeeper on first iteration (before first LLM call).
        if iteration == 1 && !config.is_sub_agent {
            let last_user_text = chat_history.iter().rev().find_map(|msg| {
                if let Message::User { content } = msg {
                    content.iter().find_map(|c| {
                        if let UserContent::Text(t) = c {
                            Some(t.text.as_str())
                        } else {
                            None
                        }
                    })
                } else {
                    None
                }
            });

            if let Some(user_text) = last_user_text {
                // Run synchronous message hooks
                let msg_ctx = MessageHookContext::user_input(
                    user_text,
                    ctx.session_id.unwrap_or(""),
                );
                let mut hook_messages = hook_registry.run_message_hooks(&msg_ctx);

                // Run async memory gatekeeper: classifies whether memory search
                // is warranted for this message.
                {
                    let client = ctx.client.read().await;
                    let wants_memory = crate::memory_gatekeeper::should_search_memory(&client, user_text).await;
                    _gatekeeper_wants_memory = wants_memory;
                    if wants_memory {
                        hook_messages.push(
                            "[Memory-First] The gatekeeper determined this message may benefit \
                             from prior context. Call `search_memories` with relevant keywords \
                             before responding."
                                .to_string(),
                        );
                    }

                    // Reflector should be active for any substantive request.
                    // Only disable it for trivial messages that clearly don't need tools.
                    let trimmed = user_text.trim();
                    let is_trivial = trimmed.len() < 20
                        && !trimmed.contains("scan")
                        && !trimmed.contains("test")
                        && !trimmed.contains("exploit");
                    reflector_active = !is_trivial || wants_memory;
                }

                if !hook_messages.is_empty() {
                    let formatted = format_system_hooks(&hook_messages);
                    tracing::info!(
                        count = hook_messages.len(),
                        "Injecting message hooks before first LLM call"
                    );

                    let _ = ctx.event_tx.send(AiEvent::SystemHooksInjected {
                        hooks: hook_messages,
                    });

                    chat_history.push(Message::User {
                        content: OneOrMany::one(UserContent::Text(Text {
                            text: formatted,
                        })),
                    });
                }
            }
        }

        // Create span for Langfuse observability (child of agent_span)
        // Token usage fields are Empty and will be recorded when available
        // Note: Langfuse expects prompt_tokens/completion_tokens per GenAI semantic conventions
        // Using both gen_ai.* and langfuse.observation.* for maximum compatibility
        let llm_span = tracing::info_span!(
            parent: &agent_span,
            "llm_completion",
            "gen_ai.operation.name" = "chat_completion",
            "gen_ai.request.model" = %ctx.model_name,
            "gen_ai.system" = %ctx.provider_name,
            "gen_ai.request.temperature" = 0.3_f64,
            "gen_ai.request.max_tokens" = MAX_COMPLETION_TOKENS as i64,
            "langfuse.observation.type" = "generation",
            "langfuse.session.id" = ctx.session_id.unwrap_or(""),
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

        // Build request - conditionally set temperature based on model support
        let temperature = if config.capabilities.supports_temperature {
            Some(0.3)
        } else {
            tracing::debug!(
                "Model {} does not support temperature parameter, omitting",
                ctx.model_name
            );
            None
        };

        // Build additional_params for provider-specific features
        let mut additional_params_json = serde_json::Map::new();

        // Add web search if enabled (OpenAI)
        if let Some(web_config) = ctx.openai_web_search_config {
            tracing::info!(
                "Adding OpenAI web_search_preview tool with context_size={}",
                web_config.search_context_size
            );
            additional_params_json.insert(
                "tools".to_string(),
                json!([web_config.to_tool_json()]),
            );
        }

        // Add reasoning config if set (for OpenAI o-series and GPT-5 Codex models)
        // OpenAI Responses API expects a nested "reasoning" object with:
        // - effort: how much thinking the model should do
        // - summary: enables streaming reasoning text to the client ("detailed" shows full reasoning)
        if let Some(effort) = ctx.openai_reasoning_effort {
            tracing::info!("Setting OpenAI reasoning.effort={}, reasoning.summary=detailed", effort);
            additional_params_json.insert(
                "reasoning".to_string(),
                json!({
                    "effort": effort,
                    "summary": "detailed"
                }),
            );
        }

        // Add OpenRouter provider preferences if set
        if let Some(prefs) = ctx.openrouter_provider_preferences {
            if let serde_json::Value::Object(prefs_map) = prefs {
                for (key, value) in prefs_map {
                    tracing::info!("Adding OpenRouter provider preference: {}={}", key, value);
                    additional_params_json.insert(key.clone(), value.clone());
                }
            }
        }

        let additional_params = if additional_params_json.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(additional_params_json))
        };

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
                ctx.provider_name,
                has_reasoning_in_history,
                supports_thinking
            );
        }

        // Wrap stream request in timeout to prevent infinite hangs (3 minutes)
        let stream_timeout = std::time::Duration::from_secs(180);

        // Proactive token count: estimate tokens BEFORE sending to detect compaction need early.
        // This is a leading indicator vs the lagging provider-reported count after the response.
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

        let mut stream_start_failure: Option<(String, StreamStartErrorClassification)> = None;
        let mut started_stream = None;

        // NVIDIA NIM workaround: rig-core's OpenAI provider serializes system
        // message content as [{"type":"text","text":"..."}] (array), but NVIDIA
        // NIM only accepts plain strings. Move the system prompt into a user
        // message so it's serialized correctly via rig-core's user content
        // flattener.
        let is_nvidia_provider = ctx.provider_name == "nvidia";

        // Build request components once before the retry loop to avoid
        // re-cloning chat_history, tools, and additional_params on each attempt.
        let (preamble, request_history) = if is_nvidia_provider {
            let mut nvidia_history = vec![Message::User {
                content: OneOrMany::one(UserContent::text(system_prompt)),
            }];
            nvidia_history.extend(chat_history.clone());
            (None, nvidia_history)
        } else {
            (Some(system_prompt.to_string()), chat_history.clone())
        };
        let request_chat_history = OneOrMany::many(request_history.clone())
            .unwrap_or_else(|_| OneOrMany::one(request_history[0].clone()));
        let request_tools = tools.clone();

        for attempt in 1..=STREAM_START_MAX_ATTEMPTS {
            let request = rig::completion::CompletionRequest {
                preamble: preamble.clone(),
                chat_history: request_chat_history.clone(),
                documents: vec![],
                tools: request_tools.clone(),
                temperature,
                max_tokens: Some(MAX_COMPLETION_TOKENS as u64),
                tool_choice: None,
                additional_params: additional_params.clone(),
                model: None,
                output_schema: None,
            };

            if is_cancelled(ctx) {
                tracing::info!("Agent cancelled before LLM call (attempt {})", attempt);
                let _ = ctx.event_tx.send(AiEvent::Error {
                    message: "Agent stopped by user".to_string(),
                    error_type: "cancelled".to_string(),
                });
                return Err(anyhow::anyhow!("Agent stopped by user"));
            }

            // Record outgoing request at the stream boundary (main agent)
            ctx.api_request_stats.record_sent(ctx.provider_name).await;

            let stream_result = tokio::time::timeout(
                stream_timeout,
                async { model.stream(request).await }.instrument(llm_span.clone()),
            )
            .await;

            match stream_result {
                Ok(Ok(s)) => {
                    ctx.api_request_stats.record_received(ctx.provider_name).await;
                    tracing::info!(
                        "[OpenAI Debug] Stream created successfully on attempt {}",
                        attempt
                    );
                    started_stream = Some(s);
                    break;
                }
                Ok(Err(e)) => {
                    let error_str = e.to_string();
                    let classification = classify_stream_start_error(&error_str);
                    tracing::warn!(
                        "Stream start failed (attempt {}/{}): {}",
                        attempt,
                        STREAM_START_MAX_ATTEMPTS,
                        error_str
                    );

                    if should_retry_stream_start(attempt, &classification) {
                        let delay = compute_retry_backoff_delay(attempt);
                        let delay_ms = delay.as_millis();
                        let _ = ctx.event_tx.send(AiEvent::Warning {
                            message: format!(
                                "AI request failed ({}). Retrying in {}ms (attempt {}/{})",
                                classification.error_type,
                                delay_ms,
                                attempt + 1,
                                STREAM_START_MAX_ATTEMPTS
                            ),
                        });
                        sleep_for_retry_delay(delay).await;
                        continue;
                    }

                    stream_start_failure = Some((error_str, classification));
                    break;
                }
                Err(_elapsed) => {
                    let timeout_secs = stream_timeout.as_secs();
                    let error_str = format!("Stream request timeout after {}s", timeout_secs);
                    let classification = stream_start_timeout_classification(timeout_secs);
                    tracing::warn!(
                        "[OpenAI Debug] Stream request timed out (attempt {}/{}): {}",
                        attempt,
                        STREAM_START_MAX_ATTEMPTS,
                        error_str
                    );

                    if should_retry_stream_start(attempt, &classification) {
                        let delay = compute_retry_backoff_delay(attempt);
                        let delay_ms = delay.as_millis();
                        let _ = ctx.event_tx.send(AiEvent::Warning {
                            message: format!(
                                "AI request timed out. Retrying in {}ms (attempt {}/{})",
                                delay_ms,
                                attempt + 1,
                                STREAM_START_MAX_ATTEMPTS
                            ),
                        });
                        sleep_for_retry_delay(delay).await;
                        continue;
                    }

                    stream_start_failure = Some((error_str, classification));
                    break;
                }
            }
        }

        let mut stream = if let Some(stream) = started_stream {
            stream
        } else {
            let (error_str, classification) = stream_start_failure.unwrap_or_else(|| {
                (
                    "Failed to start streaming response".to_string(),
                    StreamStartErrorClassification {
                        error_type: "api_error",
                        user_message: "Failed to start streaming response".to_string(),
                        retriable: false,
                    },
                )
            });

            let _ = ctx.event_tx.send(AiEvent::Error {
                message: classification.user_message,
                error_type: classification.error_type.to_string(),
            });

            return Err(TerminalErrorEmitted::with_partial_state(
                error_str,
                (!accumulated_response.is_empty()).then(|| accumulated_response.clone()),
                Some(chat_history.clone()),
            )
            .into());
        };

        tracing::debug!("[Unified] Stream started - listening for content");

        // Process streaming response
        let mut has_tool_calls = false;
        let mut tool_calls_to_execute: Vec<ToolCall> = vec![];
        let mut text_content = String::new();
        // Per-iteration thinking tracking (for history building)
        let mut thinking_content = String::new();
        let mut thinking_signature: Option<String> = None;
        // Reasoning ID for OpenAI Responses API (rs_... IDs that function calls reference)
        let mut thinking_id: Option<String> = None;
        let mut chunk_count = 0;
        let mut last_stream_chunk_error: Option<String> = None;
        let mut last_repetition_check_len: usize = 0;

        // Track tool call state for streaming
        let mut current_tool_id: Option<String> = None;
        // Separate call_id (OpenAI's call_id, e.g. "call_abc") from item id (e.g. "fc_abc").
        // These differ in the OpenAI Responses API and must be tracked independently.
        let mut current_tool_call_id: Option<String> = None;
        let mut current_tool_name: Option<String> = None;
        let mut current_tool_args = String::new();

        while let Some(chunk_result) = stream.next().await {
            if is_cancelled(ctx) {
                tracing::info!("Agent cancelled during stream processing (chunk {})", chunk_count);
                drop(stream);
                let _ = ctx.event_tx.send(AiEvent::Error {
                    message: "Agent stopped by user".to_string(),
                    error_type: "cancelled".to_string(),
                });
                return Err(anyhow::anyhow!("Agent stopped by user"));
            }
            chunk_count += 1;
            // Log progress every 50 chunks to avoid spam but track stream activity
            if chunk_count % 50 == 0 {
                tracing::debug!(
                    "[OpenAI Debug] Stream progress: {} chunks processed",
                    chunk_count
                );
            }
            match chunk_result {
                Ok(chunk) => {
                    match chunk {
                        StreamedAssistantContent::Text(text_msg) => {
                            // Check if this is thinking content (prefixed by our streaming impl)
                            // This handles the case where thinking is sent as a [Thinking] prefixed message
                            if let Some(thinking) = text_msg.text.strip_prefix("[Thinking] ") {
                                if supports_thinking {
                                    tracing::trace!(
                                        "[Unified] Received [Thinking]-prefixed text chunk #{}: {} chars",
                                        chunk_count,
                                        thinking.len()
                                    );
                                    thinking_content.push_str(thinking);
                                    accumulated_thinking.push_str(thinking);
                                }
                                // Always emit reasoning event (to frontend and sidecar)
                                emit_event(
                                    ctx,
                                    AiEvent::Reasoning {
                                        content: thinking.to_string(),
                                    },
                                );
                            } else {
                                // Check for server tool result markers
                                if let Some(rest) =
                                    text_msg.text.strip_prefix("[WEB_SEARCH_RESULT:")
                                {
                                    // Parse: [WEB_SEARCH_RESULT:tool_use_id:json_results]
                                    if let Some(colon_pos) = rest.find(':') {
                                        let tool_use_id = &rest[..colon_pos];
                                        let json_rest = rest[colon_pos + 1..].trim_end_matches(']');
                                        if let Ok(results) =
                                            serde_json::from_str::<serde_json::Value>(json_rest)
                                        {
                                            tracing::info!(
                                                "Parsed web search results for {}",
                                                tool_use_id
                                            );
                                            emit_event(
                                                ctx,
                                                AiEvent::WebSearchResult {
                                                    request_id: tool_use_id.to_string(),
                                                    results,
                                                },
                                            );
                                        }
                                    }
                                } else if let Some(rest) =
                                    text_msg.text.strip_prefix("[WEB_FETCH_RESULT:")
                                {
                                    // Parse: [WEB_FETCH_RESULT:tool_use_id:url:json_content]
                                    let parts: Vec<&str> = rest.splitn(3, ':').collect();
                                    if parts.len() >= 3 {
                                        let tool_use_id = parts[0];
                                        let url = parts[1];
                                        let json_rest = parts[2].trim_end_matches(']');
                                        let content_preview = if json_rest.len() > 200 {
                                            format!("{}...", truncate_str(json_rest, 200))
                                        } else {
                                            json_rest.to_string()
                                        };
                                        tracing::info!(
                                            "Parsed web fetch result for {}: {}",
                                            tool_use_id,
                                            url
                                        );
                                        emit_event(
                                            ctx,
                                            AiEvent::WebFetchResult {
                                                request_id: tool_use_id.to_string(),
                                                url: url.to_string(),
                                                content_preview,
                                            },
                                        );
                                    }
                                } else {
                                    // Regular text content
                                    text_content.push_str(&text_msg.text);
                                    accumulated_response.push_str(&text_msg.text);
                                    let _ = ctx.event_tx.send(AiEvent::TextDelta {
                                        delta: text_msg.text,
                                        accumulated: accumulated_response.clone(),
                                    });

                                    // Detect degenerate repetitive generation
                                    if text_content.len() > last_repetition_check_len + 200 {
                                        last_repetition_check_len = text_content.len();
                                        if detect_repetitive_text(&text_content) {
                                            tracing::warn!(
                                                text_len = text_content.len(),
                                                "Repetitive text detected, stopping generation"
                                            );
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        StreamedAssistantContent::Reasoning(reasoning) => {
                            // Native reasoning/thinking content from extended thinking models
                            let reasoning_text = reasoning
                                .content
                                .iter()
                                .filter_map(|c| {
                                    if let ReasoningContent::Text { text, .. } = c {
                                        Some(text.as_str())
                                    } else {
                                        None
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join("");
                            let chunk_signature = reasoning.content.iter().find_map(|c| {
                                if let ReasoningContent::Text { signature, .. } = c {
                                    signature.clone()
                                } else {
                                    None
                                }
                            });
                            if supports_thinking {
                                tracing::trace!(
                                    "[Unified] Received native reasoning chunk #{}: {} chars, has_signature: {}",
                                    chunk_count,
                                    reasoning_text.len(),
                                    chunk_signature.is_some()
                                );
                                thinking_content.push_str(&reasoning_text);
                                accumulated_thinking.push_str(&reasoning_text);
                                // Capture the signature (needed for Anthropic API when sending back history)
                                if chunk_signature.is_some() {
                                    thinking_signature = chunk_signature;
                                }
                                // Capture the ID (needed for OpenAI Responses API - rs_... IDs that function calls reference)
                                if reasoning.id.is_some() {
                                    thinking_id = reasoning.id.clone();
                                }
                            }
                            // Always emit reasoning event (to frontend and sidecar)
                            emit_event(
                                ctx,
                                AiEvent::Reasoning {
                                    content: reasoning_text,
                                },
                            );
                        }
                        StreamedAssistantContent::ReasoningDelta { id, reasoning } => {
                            // Streaming reasoning delta (similar to Reasoning but delivered as deltas)
                            if supports_thinking {
                                tracing::trace!(
                                    "[Unified] Received reasoning delta chunk #{}: {} chars",
                                    chunk_count,
                                    reasoning.len()
                                );
                                thinking_content.push_str(&reasoning);
                                accumulated_thinking.push_str(&reasoning);
                                // Capture the ID if present (for OpenAI Responses API)
                                if id.is_some() && thinking_id.is_none() {
                                    thinking_id = id;
                                }
                            }
                            // Always emit reasoning event (to frontend and sidecar)
                            emit_event(ctx, AiEvent::Reasoning { content: reasoning });
                        }
                        StreamedAssistantContent::ToolCall { tool_call, .. } => {
                            // Check if this is a server tool (executed by provider, not us)
                            let is_server_tool = tool_call
                                .call_id
                                .as_ref()
                                .map(|id: &String| id.starts_with("server:"))
                                .unwrap_or(false);

                            if is_server_tool {
                                // Server tool (web_search/web_fetch) - already executed by provider
                                tracing::info!(
                                    "Server tool detected: {} ({})",
                                    tool_call.function.name,
                                    tool_call.id
                                );
                                emit_event(
                                    ctx,
                                    AiEvent::ServerToolStarted {
                                        request_id: tool_call.id.clone(),
                                        tool_name: tool_call.function.name.clone(),
                                        input: tool_call.function.arguments.clone(),
                                    },
                                );
                                // Don't add to tool_calls_to_execute - provider handles execution
                                continue;
                            }

                            has_tool_calls = true;

                            // Finalize any previous pending tool call first
                            if let (Some(prev_id), Some(prev_name)) =
                                (current_tool_id.take(), current_tool_name.take())
                            {
                                let args = golish_json_repair::parse_tool_args(&current_tool_args);
                                let prev_call_id = current_tool_call_id.take().unwrap_or_else(|| prev_id.clone());
                                tool_calls_to_execute.push(ToolCall {
                                    id: prev_id,
                                    call_id: Some(prev_call_id),
                                    function: rig::message::ToolFunction {
                                        name: prev_name,
                                        arguments: args,
                                    },
                                    signature: None,
                                    additional_params: None,
                                });
                                current_tool_args.clear();
                            }

                            // Check if this tool call has complete args (non-streaming case)
                            // If args are empty object {}, we'll wait for deltas
                            let has_complete_args = !tool_call.function.arguments.is_null()
                                && tool_call.function.arguments != serde_json::json!({});

                            if has_complete_args {
                                // Tool call came complete, add directly
                                // Ensure call_id is set for OpenAI compatibility
                                let mut tool_call = tool_call;
                                if tool_call.call_id.is_none() {
                                    tool_call.call_id = Some(tool_call.id.clone());
                                }
                                tool_calls_to_execute.push(tool_call);
                            } else {
                                // Tool call has empty args, wait for deltas
                                current_tool_id = Some(tool_call.id.clone());
                                // Preserve the OpenAI call_id (e.g. "call_abc") separately from
                                // the item id (e.g. "fc_abc") — these differ in the Responses API
                                // and the call_id must match when sending function_call_output back.
                                current_tool_call_id = tool_call.call_id.clone();
                                current_tool_name = Some(tool_call.function.name.clone());
                                // Start with any existing args (might be empty object serialized)
                                if !tool_call.function.arguments.is_null()
                                    && tool_call.function.arguments != serde_json::json!({})
                                {
                                    current_tool_args = tool_call.function.arguments.to_string();
                                }
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
                            // Extract and accumulate token usage
                            if let Some(usage) = resp.token_usage() {
                                total_usage.input_tokens += usage.input_tokens;
                                total_usage.output_tokens += usage.output_tokens;
                                // Record token usage as span attributes for Langfuse
                                // Using prompt_tokens/completion_tokens per GenAI semantic conventions
                                llm_span.record(
                                    "gen_ai.usage.prompt_tokens",
                                    usage.input_tokens as i64,
                                );
                                llm_span.record(
                                    "gen_ai.usage.completion_tokens",
                                    usage.output_tokens as i64,
                                );
                                tracing::info!(
                                    "[compaction] Token usage iter {}: input={}, output={}, cumulative={}",
                                    iteration,
                                    usage.input_tokens,
                                    usage.output_tokens,
                                    total_usage.total()
                                );

                                // Update compaction state with provider token count
                                {
                                    let mut compaction_state = ctx.compaction_state.write().await;
                                    compaction_state.update_tokens(usage.input_tokens);
                                    tracing::info!(
                                        "[compaction] State updated: {} input tokens from provider",
                                        usage.input_tokens
                                    );
                                }

                                // Emit context utilization event for frontend
                                let model_config = golish_context::TokenBudgetConfig::for_model(ctx.model_name);
                                let max_tokens = model_config.max_context_tokens;
                                let utilization = usage.input_tokens as f64 / max_tokens as f64;
                                let _ = ctx.event_tx.send(AiEvent::ContextWarning {
                                    utilization,
                                    total_tokens: usage.input_tokens as usize,
                                    max_tokens,
                                });
                            } else {
                                // Fallback: estimate tokens from message content using tokenx-rs
                                let estimated_tokens: usize = chat_history
                                    .iter()
                                    .map(estimate_message_tokens)
                                    .sum();

                                // Update total_usage with estimate so it's reported to frontend
                                // We split roughly 80/20 input/output as a reasonable approximation
                                let estimated_input = (estimated_tokens as f64 * 0.8) as u64;
                                let estimated_output = (estimated_tokens as f64 * 0.2) as u64;
                                total_usage.input_tokens += estimated_input;
                                total_usage.output_tokens += estimated_output;

                                {
                                    let mut compaction_state = ctx.compaction_state.write().await;
                                    compaction_state.update_tokens_estimated(estimated_tokens as u64);
                                    tracing::info!(
                                        "[compaction] State updated (tokenx-rs estimate): ~{} estimated tokens",
                                        estimated_tokens,
                                    );
                                }

                                // Emit context utilization event for frontend (heuristic)
                                let model_config = golish_context::TokenBudgetConfig::for_model(ctx.model_name);
                                let max_tokens = model_config.max_context_tokens;
                                let utilization = estimated_tokens as f64 / max_tokens as f64;
                                let _ = ctx.event_tx.send(AiEvent::ContextWarning {
                                    utilization,
                                    total_tokens: estimated_tokens,
                                    max_tokens,
                                });
                            }

                            // Extract reasoning encrypted_content from OpenAI Responses API
                            // The Final response may contain reasoning_encrypted_content which is
                            // required for stateless multi-turn conversations with reasoning models.
                            // We serialize to JSON and check for the OpenAI-specific field.
                            if let Ok(json_value) = serde_json::to_value(resp) {
                                // Log what we're seeing in the Final response
                                let has_encrypted_field = json_value.get("reasoning_encrypted_content").is_some();
                                tracing::info!(
                                    "[OpenAI Debug] Final response: has_reasoning_encrypted_content={}, thinking_id={:?}, thinking_signature_before={:?}",
                                    has_encrypted_field,
                                    thinking_id,
                                    thinking_signature.as_ref().map(|s| s.len())
                                );

                                if let Some(encrypted_map) = json_value
                                    .get("reasoning_encrypted_content")
                                    .and_then(|v| v.as_object())
                                {
                                    tracing::info!(
                                        "[OpenAI Debug] encrypted_map has {} entries: {:?}",
                                        encrypted_map.len(),
                                        encrypted_map.keys().collect::<Vec<_>>()
                                    );

                                    // If we have accumulated thinking and captured a thinking_id,
                                    // look up the encrypted_content for that reasoning item
                                    if let Some(ref tid) = thinking_id {
                                        if let Some(encrypted) = encrypted_map.get(tid).and_then(|v| v.as_str()) {
                                            tracing::info!(
                                                "[OpenAI Debug] Found encrypted_content for reasoning item {}: {} bytes",
                                                tid,
                                                encrypted.len()
                                            );
                                            thinking_signature = Some(encrypted.to_string());
                                        } else {
                                            tracing::warn!(
                                                "[OpenAI Debug] thinking_id {} NOT FOUND in encrypted_map!",
                                                tid
                                            );
                                        }
                                    }
                                    // If we don't have a thinking_id but have exactly one reasoning item,
                                    // use that one (common case: single reasoning block in response)
                                    if thinking_signature.is_none() && encrypted_map.len() == 1 {
                                        if let Some((id, encrypted)) = encrypted_map.iter().next() {
                                            if let Some(encrypted_str) = encrypted.as_str() {
                                                tracing::info!(
                                                    "[OpenAI Debug] Using single encrypted_content for reasoning item {}: {} bytes",
                                                    id,
                                                    encrypted_str.len()
                                                );
                                                thinking_signature = Some(encrypted_str.to_string());
                                                // Also set thinking_id if not set
                                                if thinking_id.is_none() {
                                                    thinking_id = Some(id.clone());
                                                }
                                            }
                                        }
                                    }
                                }
                            } else {
                                tracing::warn!("[OpenAI Debug] Failed to serialize Final response to JSON");
                            }

                            // Finalize any pending tool call from deltas
                            if let (Some(id), Some(name)) =
                                (current_tool_id.take(), current_tool_name.take())
                            {
                                let args = golish_json_repair::parse_tool_args(&current_tool_args);
                                let call_id = current_tool_call_id.take().unwrap_or_else(|| id.clone());
                                tool_calls_to_execute.push(ToolCall {
                                    id,
                                    call_id: Some(call_id),
                                    function: rig::message::ToolFunction {
                                        name,
                                        arguments: args,
                                    },
                                    signature: None,
                                    additional_params: None,
                                });
                                current_tool_args.clear();
                            }
                        }
                    }
                }
                Err(e) => {
                    last_stream_chunk_error = Some(e.to_string());
                    tracing::warn!("Stream chunk error at #{}: {}", chunk_count, e);
                }
            }
        }

        // If the stream produced no usable content but had errors, surface the error
        if text_content.is_empty()
            && thinking_content.is_empty()
            && tool_calls_to_execute.is_empty()
            && current_tool_name.is_none()
        {
            if let Some(ref err_msg) = last_stream_chunk_error {
                let classification = classify_stream_start_error(err_msg);
                let _ = ctx.event_tx.send(AiEvent::Error {
                    message: classification.user_message.clone(),
                    error_type: classification.error_type.to_string(),
                });
                tracing::error!(
                    "Stream produced no content; last chunk error: {}",
                    err_msg
                );
                break;
            }
        }

        tracing::info!(
            "[OpenAI Debug] Stream completed: iteration={}, chunks={}, text_chars={}, thinking_chars={}, tool_calls={}",
            iteration,
            chunk_count,
            text_content.len(),
            thinking_content.len(),
            tool_calls_to_execute.len()
        );
        tracing::debug!(
            "Stream completed (unified): {} chunks, {} chars text, {} chars thinking, {} tool calls",
            chunk_count,
            text_content.len(),
            thinking_content.len(),
            tool_calls_to_execute.len()
        );

        // Record the completion for Langfuse (truncated to avoid huge spans)
        // Only record text content - tool call details are in child tool spans
        let completion_for_span = if !text_content.is_empty() {
            // Model produced text: record it (truncated)
            let mut end = text_content.len().min(2000);
            while end > 0 && !text_content.is_char_boundary(end) {
                end -= 1;
            }
            if text_content.len() > 2000 {
                format!("{}... [truncated]", &text_content[..end])
            } else {
                text_content.clone()
            }
        } else if !tool_calls_to_execute.is_empty() {
            // Model produced only tool calls (common for GPT-5.2/Codex): record tool names
            // so the span is not empty and traces show what the model decided to do.
            let names: Vec<&str> = tool_calls_to_execute
                .iter()
                .map(|tc| tc.function.name.as_str())
                .collect();
            format!("[tool_calls: {}]", names.join(", "))
        } else {
            String::new()
        };
        if !completion_for_span.is_empty() {
            llm_span.record("gen_ai.completion", completion_for_span.as_str());
            llm_span.record("langfuse.observation.output", completion_for_span.as_str());
        }

        // Record reasoning/thinking content on the span if present.
        // This is the model's internal reasoning displayed in the UI ThinkingBlock —
        // it must also appear in traces so Langfuse shows what the model was thinking.
        if !thinking_content.is_empty() {
            let mut end = thinking_content.len().min(2000);
            while end > 0 && !thinking_content.is_char_boundary(end) {
                end -= 1;
            }
            let reasoning_for_span = if thinking_content.len() > 2000 {
                format!("{}... [truncated]", &thinking_content[..end])
            } else {
                thinking_content.clone()
            };
            llm_span.record("gen_ai.reasoning", reasoning_for_span.as_str());
        }

        // Finalize any remaining tool call that wasn't closed by FinalResponse
        if let (Some(id), Some(name)) = (current_tool_id.take(), current_tool_name.take()) {
            let args = golish_json_repair::parse_tool_args(&current_tool_args);
            let call_id = current_tool_call_id.take().unwrap_or_else(|| id.clone());
            tool_calls_to_execute.push(ToolCall {
                id,
                call_id: Some(call_id),
                function: rig::message::ToolFunction {
                    name,
                    arguments: args,
                },
                signature: None,
                additional_params: None,
            });
            has_tool_calls = true;
        }

        // Log thinking content if present (for debugging)
        if supports_thinking && !thinking_content.is_empty() {
            tracing::debug!("Model thinking: {} chars", thinking_content.len());
        }

        // Build assistant content for history
        // IMPORTANT: When thinking is enabled, thinking blocks MUST come first (required by Anthropic API)
        let mut assistant_content: Vec<AssistantContent> = vec![];

        // Conditionally add thinking content first (required by Anthropic API when thinking is enabled)
        // OpenAI Responses API reasoning handling differs between providers:
        //
        // "openai_reasoning" (rig-openai-responses, gpt-5.2, Codex, o-series):
        //   - Always include reasoning in history when present. OpenAI tracks rs_... IDs
        //     server-side and requires them to be echoed back in every subsequent turn.
        //   - A reasoning item MUST be followed by the next output item (text OR tool call).
        //   - Omitting a reasoning item from a turn where it was generated causes:
        //     "Item 'rs_...' of type 'reasoning' was provided without its required following item"
        //
        // "openai_responses" (rig-core built-in, non-reasoning models via Responses API):
        //   - Only include reasoning when there are tool calls. Without a following function_call
        //     the API returns: "reasoning was provided without its required following item"
        //   - These models use internal reasoning IDs that are only meaningful when paired with
        //     a function call; standalone reasoning items are not valid for text-only turns.
        let is_openai_reasoning_provider = ctx.provider_name == "openai_reasoning";
        let is_openai_responses_api = ctx.provider_name == "openai_responses";
        let has_reasoning = !thinking_content.is_empty() || thinking_id.is_some();
        let should_include_reasoning = if is_openai_reasoning_provider {
            // Always include reasoning for openai_reasoning — rs_ IDs must be echoed back
            has_reasoning
        } else if is_openai_responses_api {
            // For openai_responses: only include reasoning when paired with a tool call
            has_reasoning && has_tool_calls
        } else {
            // For other providers (Anthropic, etc.): include reasoning when present
            has_reasoning
        };
        if supports_thinking && should_include_reasoning {
            tracing::info!(
                "[OpenAI Debug] Building assistant content with reasoning: id={:?}, signature_len={:?}",
                thinking_id,
                thinking_signature.as_ref().map(|s| s.len())
            );
            assistant_content.push(AssistantContent::Reasoning(
                Reasoning::new_with_signature(&thinking_content, thinking_signature.clone())
                    .optional_id(thinking_id.clone()),
            ));
        }

        if !text_content.is_empty() {
            assistant_content.push(AssistantContent::Text(Text {
                text: text_content.clone(),
            }));
        }

        // Add tool calls to assistant content if present
        for tool_call in &tool_calls_to_execute {
            assistant_content.push(AssistantContent::ToolCall(tool_call.clone()));
        }

        // ALWAYS add assistant message to history (even when no tool calls)
        // This is critical for maintaining conversation context across turns
        if !assistant_content.is_empty() {
            chat_history.push(Message::Assistant {
                id: None,
                content: OneOrMany::many(assistant_content).unwrap_or_else(|_| {
                    OneOrMany::one(AssistantContent::Text(Text {
                        text: String::new(),
                    }))
                }),
            });
        }

        // If no tool calls, either invoke reflector or finish
        if !has_tool_calls {
            consecutive_no_tool_turns += 1;

            // Reflector: if the agent produced text but no tool calls, and we haven't
            // exhausted reflector attempts, invoke the reflector to diagnose and correct.
            // Skip reflector only for trivial messages (greetings, acks) where a
            // text-only response is expected.
            let should_reflect = consecutive_no_tool_turns <= 3
                && total_reflector_nudges < 3
                && !text_content.trim().is_empty()
                && config.enable_reflector
                && reflector_active;

            if should_reflect {
                let registry = ctx.sub_agent_registry.read().await;
                let reflector_def = registry.get("reflector").cloned();
                drop(registry);

                if reflector_def.is_some() {
                    total_reflector_nudges += 1;
                    tracing::info!(
                        attempt = consecutive_no_tool_turns,
                        total_nudges = total_reflector_nudges,
                        text_len = text_content.len(),
                        "[reflector] Agent produced text without tool calls, invoking reflector chain"
                    );

                    // Build a diagnostic prompt for the reflector with the agent's response
                    // and available tool names so it can suggest specific tools.
                    let tool_list = config
                        .tool_names_for_reflector
                        .as_ref()
                        .map(|names| names.join(", "))
                        .unwrap_or_else(|| {
                            tools
                                .iter()
                                .map(|t| t.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        });

                    let reflector_prompt = format!(
                        "The agent was given a task and responded with text instead of using tools.\n\n\
                         ## Agent's text response (attempt {}/3):\n```\n{}\n```\n\n\
                         ## Available tools:\n{}\n\n\
                         Diagnose why the agent didn't use tools and write a corrective instruction \
                         that will get it to take action. Be specific about which tool to use and with what arguments.",
                        total_reflector_nudges,
                        truncate_str(&text_content, 2000),
                        tool_list
                    );

                    // Run the reflector as a proper sub-agent chain (PentAGI-style).
                    let reflector_args = serde_json::json!({
                        "task": reflector_prompt,
                    });

                    let correction = if let Some(ref reflector) = reflector_def {
                        use crate::tool_provider_impl::DefaultToolProvider;
                        let tool_provider = DefaultToolProvider::new();
                        let sub_ctx = golish_sub_agents::SubAgentExecutorContext {
                            event_tx: ctx.event_tx,
                            tool_registry: ctx.tool_registry,
                            workspace: ctx.workspace,
                            provider_name: ctx.provider_name,
                            model_name: ctx.model_name,
                            session_id: ctx.session_id,
                            transcript_base_dir: ctx.transcript_base_dir,
                            api_request_stats: Some(ctx.api_request_stats),
                            briefing: None,
                            temperature_override: reflector.temperature,
                            max_tokens_override: reflector.max_tokens,
                            top_p_override: reflector.top_p,
                            db_pool: ctx.db_tracker.map(|t| t.pool_arc()),
                            sub_agent_registry: Some(ctx.sub_agent_registry),
                        };

                        match crate::agentic_loop::sub_agent_dispatch::execute_sub_agent_with_client(
                            reflector,
                            &reflector_args,
                            &sub_agent_context,
                            &*ctx.client.read().await,
                            sub_ctx,
                            &tool_provider,
                            "reflector",
                        )
                        .await
                        {
                            Ok(result) => {
                                tracing::info!(
                                    "[reflector] Chain returned {} chars of correction",
                                    result.response.len()
                                );
                                result.response
                            }
                            Err(e) => {
                                tracing::warn!("[reflector] Chain failed, using fallback nudge: {}", e);
                                format!(
                                    "[System: You responded with text but did not use any tools. \
                                     Please execute the next step using the appropriate tool. \
                                     Available tools: {}. Attempt {}/3]",
                                    tool_list, total_reflector_nudges
                                )
                            }
                        }
                    } else {
                        format!(
                            "[System: You responded with text but did not use any tools. \
                             Please execute the next step using the appropriate tool. Attempt {}/3]",
                            total_reflector_nudges
                        )
                    };

                    chat_history.push(Message::User {
                        content: OneOrMany::one(UserContent::Text(rig::message::Text {
                            text: correction,
                        })),
                    });
                    continue;
                }
            }

            break;
        } else {
            consecutive_no_tool_turns = 0;
        }

        // Execute tool calls and collect results (with concurrent dispatch for sub-agents)
        let total_tool_count = tool_calls_to_execute.len();
        let (sub_agent_calls, other_calls) = partition_tool_calls(tool_calls_to_execute);
        let has_concurrent_sub_agents = sub_agent_calls.len() >= 2;

        // Pre-allocate indexed results: (UserContent, Vec<system_hooks>)
        let mut indexed_results: Vec<Option<(UserContent, Vec<String>)>> = vec![None; total_tool_count];

        // Execute sub-agent calls concurrently if there are 2+
        if has_concurrent_sub_agents {
            tracing::info!(
                count = sub_agent_calls.len(),
                "Executing sub-agent tool calls concurrently"
            );

            let futures: Vec<_> = sub_agent_calls
                .into_iter()
                .map(|(original_idx, tool_call)| {
                    let llm_span = &llm_span;
                    let capture_ctx = &capture_ctx;
                    let sub_agent_context = &sub_agent_context;
                    let hook_registry = &hook_registry;
                    async move {
                        let result = execute_single_tool_call(
                            tool_call, ctx, capture_ctx, model, sub_agent_context,
                            hook_registry, llm_span,
                        )
                        .await;
                        (original_idx, result)
                    }
                })
                .collect();

            let concurrent_results = futures::future::join_all(futures).await;
            for (idx, result) in concurrent_results {
                indexed_results[idx] = Some(result);
            }
        } else {
            // 0 or 1 sub-agent calls — execute sequentially (no spawn overhead)
            for (original_idx, tool_call) in sub_agent_calls {
                if is_cancelled(ctx) {
                    tracing::info!("Agent cancelled before sub-agent call: {}", tool_call.function.name);
                    break;
                }
                let result = execute_single_tool_call(
                    tool_call, ctx, &capture_ctx, model, &sub_agent_context,
                    &hook_registry, &llm_span,
                )
                .await;
                indexed_results[original_idx] = Some(result);
            }
        }

        // Execute non-sub-agent calls sequentially (always)
        for (original_idx, tool_call) in other_calls {
            if is_cancelled(ctx) {
                tracing::info!("Agent cancelled before tool execution: {}", tool_call.function.name);
                break;
            }
            let result = execute_single_tool_call(
                tool_call, ctx, &capture_ctx, model, &sub_agent_context,
                &hook_registry, &llm_span,
            )
            .await;
            indexed_results[original_idx] = Some(result);
        }

        // Flatten results in original order
        let mut tool_results: Vec<UserContent> = Vec::with_capacity(total_tool_count);
        let mut system_hooks: Vec<String> = vec![];
        for (user_content, hooks) in indexed_results.into_iter().flatten() {
            tool_results.push(user_content);
            system_hooks.extend(hooks);
        }

        // Merge system hooks into the tool results message to avoid
        // "user after tool" ordering violations with OpenAI-compatible APIs.
        if !system_hooks.is_empty() {
            let formatted_hooks = format_system_hooks(&system_hooks);

            tracing::info!(
                count = system_hooks.len(),
                content_len = formatted_hooks.len(),
                "Injecting system hooks into tool results message"
            );

            let _ = ctx
                .event_tx
                .send(AiEvent::SystemHooksInjected { hooks: system_hooks.clone() });

            let _system_hook_event = tracing::info_span!(
                parent: &llm_span,
                "system_hooks_injected",
                "langfuse.observation.type" = "event",
                "langfuse.observation.level" = "DEFAULT",
                "langfuse.session.id" = ctx.session_id.unwrap_or(""),
                hook_count = system_hooks.len(),
                "langfuse.observation.input" = %formatted_hooks,
            );

            tool_results.push(UserContent::Text(Text {
                text: formatted_hooks,
            }));
        }

        // Add tool results (+ any system hooks) as a single user message
        chat_history.push(Message::User {
            content: OneOrMany::many(tool_results).unwrap_or_else(|_| {
                OneOrMany::one(UserContent::Text(Text {
                    text: "Tool executed".to_string(),
                }))
            }),
        });
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
        ctx.provider_name,
        ctx.model_name,
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
    if let Some(tracker) = ctx.db_tracker {
        if total_usage.input_tokens > 0 || total_usage.output_tokens > 0 {
            tracker.record_token_usage(
                total_usage.input_tokens,
                total_usage.output_tokens,
                ctx.model_name,
                ctx.provider_name,
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
