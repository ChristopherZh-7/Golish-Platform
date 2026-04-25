use std::sync::Arc;
use rig::completion::{CompletionModel as RigCompletionModel, Message};
use rig::message::{Text, ToolCall, ToolResult, ToolResultContent, UserContent};
use rig::one_or_many::OneOrMany;
use serde_json::json;
use tokio::sync::RwLock;
use golish_core::events::{AiEvent, ToolSource};
use golish_core::utils::truncate_str;
use golish_sub_agents::SubAgentContext;
use super::context::{AgenticLoopContext, LoopCaptureContext, ToolExecutionResult, emit_to_frontend};
use super::helpers::handle_loop_detection;
use super::llm_helpers::{summarize_tool_output, mentor_one_shot};
use super::tool_execution::execute_with_hitl_generic;
use super::{normalize_run_pty_cmd_args, toolcall_fixer, SUMMARIZE_THRESHOLD_TOKENS};
use crate::system_hooks::{HookRegistry, PostToolContext};

pub(super) async fn execute_single_tool_call<M>(
    tool_call: ToolCall,
    ctx: &AgenticLoopContext<'_>,
    capture_ctx: &LoopCaptureContext,
    model: &M,
    sub_agent_context: &SubAgentContext,
    hook_registry: &HookRegistry,
    llm_span: &tracing::Span,
) -> (UserContent, Vec<String>)
where
    M: RigCompletionModel + Sync,
{
    let tool_name = &tool_call.function.name;
    let tool_args = if tool_name == "run_pty_cmd" || tool_name == "run_command" {
        normalize_run_pty_cmd_args(tool_call.function.arguments.clone())
    } else {
        tool_call.function.arguments.clone()
    };
    let tool_id = tool_call.id.clone();
    let tool_call_id = tool_call.call_id.clone().unwrap_or_else(|| tool_id.clone());

    tracing::info!(
        "[tool-dispatch] Executing tool: name={}, id={}, args_len={}",
        tool_name,
        tool_id,
        serde_json::to_string(&tool_args).map(|s| s.len()).unwrap_or(0),
    );

    // Create span for tool call
    let args_str = serde_json::to_string(&tool_args).unwrap_or_default();
    let args_for_span = if args_str.len() > 1000 {
        format!("{}... [truncated]", truncate_str(&args_str, 1000))
    } else {
        args_str
    };
    let tool_span = tracing::info_span!(
        parent: llm_span,
        "tool_call",
        "otel.name" = %tool_name,
        "langfuse.span.name" = %tool_name,
        "langfuse.observation.type" = "tool",
        "langfuse.session.id" = ctx.session_id.unwrap_or(""),
        tool.name = %tool_name,
        tool.id = %tool_id,
        "langfuse.observation.input" = %args_for_span,
        "langfuse.observation.output" = tracing::field::Empty,
        success = tracing::field::Empty,
    );

    // Check for loop detection
    let loop_result = {
        let mut detector = ctx.loop_detector.write().await;
        detector.record_tool_call(tool_name, &tool_args)
    };

    // Handle loop detection (may return a blocked result)
    if let Some(blocked_result) =
        handle_loop_detection(&loop_result, &tool_id, &tool_call_id, ctx.event_tx)
    {
        let loop_info = match &loop_result {
            crate::loop_detection::LoopDetectionResult::Blocked {
                repeat_count,
                max_count,
                ..
            } => format!("repeat_count={}, max={}", repeat_count, max_count),
            crate::loop_detection::LoopDetectionResult::MaxIterationsReached {
                iterations,
                max_iterations,
                ..
            } => format!("iterations={}, max={}", iterations, max_iterations),
            _ => String::new(),
        };
        let _loop_event = tracing::info_span!(
            parent: llm_span,
            "loop_blocked",
            "langfuse.observation.type" = "event",
            "langfuse.session.id" = ctx.session_id.unwrap_or(""),
            tool_name = %tool_name,
            details = %loop_info,
        );
        tool_span.record("success", false);
        tool_span.record("langfuse.observation.output", "blocked by loop detection");
        return (blocked_result, vec![]);
    }

    // Start DB tracking for tool call timing
    let db_guard = ctx
        .db_tracker
        .map(|t| t.start_tool_call(&tool_id, tool_name, &tool_args));

    // Execute tool with HITL approval check
    let mut result = execute_with_hitl_generic(
        tool_name,
        &tool_args,
        &tool_id,
        ctx,
        capture_ctx,
        model,
        sub_agent_context,
    )
    .await
    .unwrap_or_else(|e| ToolExecutionResult {
        value: json!({ "error": e.to_string() }),
        success: false,
    });

    // Tool Call Auto-Fixer: if execution failed with a schema/argument error,
    // try a lightweight LLM call to repair the args and retry once.
    if !result.success {
        let error_text = result.value.get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let tool_schema = {
            let registry = ctx.tool_registry.read().await;
            registry.get_tool_definitions()
                .into_iter()
                .find(|td| td.name == *tool_name)
                .map(|td| td.parameters)
        };

        if let Some(fixed_args) = toolcall_fixer::try_fix_tool_args(
            model,
            tool_name,
            &tool_args,
            &error_text,
            tool_schema.as_ref(),
        ).await {
            tracing::info!(
                "[toolcall-fixer] Retrying '{}' with repaired args",
                tool_name
            );
            result = execute_with_hitl_generic(
                tool_name,
                &fixed_args,
                &tool_id,
                ctx,
                capture_ctx,
                model,
                sub_agent_context,
            )
            .await
            .unwrap_or_else(|e| ToolExecutionResult {
                value: json!({ "error": e.to_string() }),
                success: false,
            });
        }
    }

    // Finish DB tracking with result
    if let (Some(tracker), Some(guard)) = (ctx.db_tracker, db_guard) {
        let result_text = serde_json::to_string(&result.value).unwrap_or_default();
        tracker.finish_tool_call(guard, result.success, &result_text);

        // Record search logs for web search tools
        if tool_name.starts_with("tavily_") || tool_name.starts_with("web_search") {
            let query = tool_args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let result_preview = serde_json::to_string(&result.value)
                .ok()
                .map(|s| truncate_str(&s, 10000).to_string());
            tracker.record_search(
                if tool_name.starts_with("tavily_") { "tavily" } else { "web" },
                query,
                result_preview.as_deref(),
            );
        }

        // Record terminal logs for shell/PTY commands
        if tool_name == "run_pty_cmd" || tool_name == "run_command" || tool_name == "run_shell_cmd" {
            let output = result.value.get("output")
                .or_else(|| result.value.get("stdout"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !output.is_empty() {
                tracker.record_terminal_output("stdout", output);
            }
            if let Some(stderr) = result.value.get("stderr").and_then(|v| v.as_str()) {
                if !stderr.is_empty() {
                    tracker.record_terminal_output("stderr", stderr);
                }
            }
        }

        // Skip memory storage for shell commands that have structured output storage
        let skip_memory = if result.success
            && (tool_name == "run_pty_cmd" || tool_name == "run_command")
        {
            let cmd = tool_args
                .get("command")
                .and_then(|c| c.as_str())
                .unwrap_or("");
            let stdout = result
                .value
                .get("stdout")
                .and_then(|s| s.as_str())
                .unwrap_or("");
            golish_pentest::output_store::has_structured_storage(cmd, stdout)
        } else {
            false
        };

        if !skip_memory {
            tracker.maybe_store_tool_memory(tool_name, &tool_args, &result.value, result.success);
        }
    }

    // Record tool result in span
    let result_str = serde_json::to_string(&result.value).unwrap_or_default();
    let result_for_span = if result_str.len() > 1000 {
        format!("{}... [truncated]", truncate_str(&result_str, 1000))
    } else {
        result_str
    };
    tool_span.record("langfuse.observation.output", result_for_span.as_str());
    tool_span.record("success", result.success);

    // Emit tool result event
    let result_event = AiEvent::ToolResult {
        tool_name: tool_name.clone(),
        result: result.value.clone(),
        success: result.success,
        request_id: tool_id.clone(),
        source: golish_core::events::ToolSource::Main,
    };
    emit_to_frontend(ctx, result_event.clone());
    capture_ctx.process(&result_event);

    // Execution Mentor check (PentAGI pattern): when the monitor detects
    // repetitive tool usage, generate corrective advice and append it.
    let mentor_advice = if let Some(ref monitor) = ctx.execution_monitor {
        let args_summary = serde_json::to_string(&tool_args).unwrap_or_default();
        let should_mentor = {
            let mut mon = monitor.write().await;
            mon.record_and_check(tool_name, &args_summary)
        };
        if should_mentor {
            let (repeated_tool, repeat_count, recent_summary) = {
                let mon = monitor.read().await;
                (
                    mon.repeated_tool_name().to_string(),
                    mon.same_tool_count(),
                    mon.recent_calls_summary(),
                )
            };
            tracing::info!(
                "[ExecutionMentor] Monitor triggered: '{}' called {} times, invoking LLM mentor",
                repeated_tool,
                repeat_count,
            );
            let advice = {
                let mentor_system = crate::task_orchestrator::prompts::mentor_system_prompt();
                let mentor_user = crate::task_orchestrator::prompts::mentor_user_prompt(
                    tool_name,
                    &repeated_tool,
                    repeat_count,
                    &recent_summary,
                );
                match mentor_one_shot(ctx.client, mentor_system, &mentor_user).await {
                    Ok(llm_advice) => {
                        tracing::info!(
                            "[ExecutionMentor] LLM mentor produced {} chars of advice",
                            llm_advice.len()
                        );
                        format!("\n\n--- EXECUTION ADVISOR ---\n{}\n-------------------------", llm_advice)
                    }
                    Err(e) => {
                        tracing::warn!("[ExecutionMentor] LLM mentor failed, using static fallback: {}", e);
                        format!(
                            "\n\n--- EXECUTION ADVISOR ---\n\
                             You have called '{}' {} times. Consider a different approach:\n\
                             - Try a different tool to make progress\n\
                             - Check if previous results already contain the information you need\n\
                             - If stuck, use a different strategy entirely\n\
                             Recent calls: {}\n\
                             -------------------------",
                            repeated_tool, repeat_count, recent_summary,
                        )
                    }
                }
            };
            {
                let mut mon = monitor.write().await;
                mon.reset_after_mentor();
            }
            Some(advice)
        } else {
            None
        }
    } else {
        None
    };

    // Convert result to text and truncate if necessary
    let mut raw_result_text = serde_json::to_string(&result.value).unwrap_or_default();
    if let Some(ref advice) = mentor_advice {
        raw_result_text.push_str(advice);
    }
    let truncation_result = ctx
        .context_manager
        .truncate_tool_response(&raw_result_text, tool_name)
        .await;

    let final_content = if truncation_result.truncated {
        let original_tokens = golish_context::TokenBudgetManager::estimate_tokens(&raw_result_text);
        let truncated_tokens =
            golish_context::TokenBudgetManager::estimate_tokens(&truncation_result.content);
        let _ = ctx.event_tx.send(AiEvent::ToolResponseTruncated {
            tool_name: tool_name.clone(),
            original_tokens,
            truncated_tokens,
        });

        // If truncated output is still large, attempt LLM summarization
        if truncated_tokens > SUMMARIZE_THRESHOLD_TOKENS {
            match summarize_tool_output(ctx.client, tool_name, &truncation_result.content).await {
                Ok(summary) => {
                    tracing::info!(
                        "[ToolSummarizer] Summarized '{}' output: {} -> {} tokens",
                        tool_name,
                        truncated_tokens,
                        golish_context::TokenBudgetManager::estimate_tokens(&summary),
                    );
                    summary
                }
                Err(e) => {
                    tracing::warn!(
                        "[ToolSummarizer] Failed for '{}', using truncated: {}",
                        tool_name,
                        e
                    );
                    truncation_result.content
                }
            }
        } else {
            truncation_result.content
        }
    } else {
        truncation_result.content
    };

    let user_content = UserContent::ToolResult(ToolResult {
        id: tool_id.clone(),
        call_id: Some(tool_call_id),
        content: OneOrMany::one(ToolResultContent::Text(Text {
            text: final_content,
        })),
    });

    // Run post-tool hooks
    let post_ctx = PostToolContext::new(
        tool_name,
        &tool_args,
        &result.value,
        result.success,
        0,
        ctx.session_id.unwrap_or(""),
    );
    let hooks = hook_registry.run_post_tool_hooks(&post_ctx);

    (user_content, hooks)
}
