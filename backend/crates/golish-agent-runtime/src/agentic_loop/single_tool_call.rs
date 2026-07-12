use super::context::{
    emit_to_frontend, AgenticLoopContext, LoopCaptureContext, ToolExecutionResult,
};
use super::helpers::handle_loop_detection;
use super::llm_helpers::{runtime_supervisor_one_shot, summarize_tool_output};
use super::tool_execution::execute_with_hitl_generic;
use super::{normalize_run_pty_cmd_args, toolcall_fixer, SUMMARIZE_THRESHOLD_TOKENS};
use golish_agent_kit::loop_detection::ExecutionMonitorMode;
use golish_agent_kit::system_hooks::{HookRegistry, PostToolContext};
use golish_agent_kit::task_orchestrator::runtime_supervisor::{
    directive_from_model_response, runtime_supervisor_system_prompt,
    runtime_supervisor_user_prompt, RuntimeSupervisorContext, StrategyDirective,
};
use golish_core::events::{AiEvent, HarnessTraceKind, ToolSource};
use golish_core::utils::truncate_str;
use golish_core::AgentToolContext;
use golish_sub_agents::SubAgentContext;
use rig::completion::CompletionModel as RigCompletionModel;
use rig::message::{Text, ToolCall, ToolResult, ToolResultContent, UserContent};
use rig::one_or_many::OneOrMany;
use serde_json::json;

fn runtime_supervisor_agent_path(sub_agent_context: &SubAgentContext) -> String {
    if sub_agent_context.depth == 0 {
        return "main".to_string();
    }

    let current = format!("subagent_depth_{}", sub_agent_context.depth);
    match sub_agent_context.parent_agent.as_deref() {
        Some("main" | "main-agent") | None => format!("main>{current}"),
        Some(parent) if parent.starts_with("main>") => format!("{parent}>{current}"),
        Some(parent) => format!("main>{parent}>{current}"),
    }
}

fn emit_runtime_supervisor_trace(
    ctx: &AgenticLoopContext<'_>,
    sub_agent_context: &SubAgentContext,
    mode: ExecutionMonitorMode,
    repeated_tool: &str,
    repeat_count: usize,
    directive: &StrategyDirective,
    injected: bool,
) {
    let operation_id = ctx
        .harness_operation_id
        .map(|id| id.to_string())
        .or_else(|| ctx.events.session_id.map(str::to_string));
    let Some(operation_id) = operation_id else {
        return;
    };

    let trace = AiEvent::HarnessTrace {
        operation_id,
        stage: ctx
            .harness_stage
            .map(|stage| stage.as_str().to_string())
            .unwrap_or_default(),
        agent_path: runtime_supervisor_agent_path(sub_agent_context),
        trace: HarnessTraceKind::RuntimeSupervisorDecision {
            mode: mode.as_str().to_string(),
            trigger: "execution_monitor".to_string(),
            tool: repeated_tool.to_string(),
            repeat_count: repeat_count.min(u32::MAX as usize) as u32,
            injected,
            strategy_kind: directive.strategy_kind_label().to_string(),
            root_cause: directive.root_cause.clone(),
            action_count: directive.actions.len().min(u32::MAX as usize) as u32,
            directive_hash: directive.directive_hash.clone(),
        },
    };
    let _ = ctx.events.event_tx.send(trace);
}

async fn visible_tool_names(ctx: &AgenticLoopContext<'_>) -> Vec<String> {
    let registry = ctx.tool_registry.read().await;
    registry
        .get_tool_definitions()
        .into_iter()
        .map(|tool| tool.name)
        .collect()
}

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
        serde_json::to_string(&tool_args)
            .map(|s| s.len())
            .unwrap_or(0),
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
        "langfuse.session.id" = ctx.events.session_id.unwrap_or(""),
        tool.name = %tool_name,
        tool.id = %tool_id,
        "langfuse.observation.input" = %args_for_span,
        "langfuse.observation.output" = tracing::field::Empty,
        success = tracing::field::Empty,
    );

    // Check for loop detection
    let loop_result = {
        let mut detector = ctx.access.loop_detector.write().await;
        detector.record_tool_call(tool_name, &tool_args)
    };

    // Handle loop detection (may return a blocked result)
    if let Some(blocked_result) =
        handle_loop_detection(&loop_result, &tool_id, &tool_call_id, ctx.events.event_tx)
    {
        let loop_info = match &loop_result {
            golish_agent_kit::loop_detection::LoopDetectionResult::Blocked {
                repeat_count,
                max_count,
                ..
            } => format!("repeat_count={}, max={}", repeat_count, max_count),
            golish_agent_kit::loop_detection::LoopDetectionResult::MaxIterationsReached {
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
            "langfuse.session.id" = ctx.events.session_id.unwrap_or(""),
            tool_name = %tool_name,
            details = %loop_info,
        );
        tool_span.record("success", false);
        tool_span.record("langfuse.observation.output", "blocked by loop detection");
        return (blocked_result, vec![]);
    }

    // Start DB tracking for tool call timing
    let db_guard = if let Some(tracker) = ctx.events.db_tracker {
        Some(
            tracker
                .start_tool_call(&tool_id, tool_name, &tool_args)
                .await,
        )
    } else {
        None
    };

    // Execute tool with HITL approval check
    let tool_context = AgentToolContext {
        request_id: tool_id.clone(),
        tool_name: tool_name.to_string(),
        source: ToolSource::Main,
        operation_id: ctx.harness_operation_id,
        organization_id: ctx.harness_org_id,
    };
    let mut result = golish_core::with_agent_tool_context(
        Some(tool_context.clone()),
        golish_core::with_agent_tool_output_sender(Some(ctx.events.event_tx.clone()), async {
            execute_with_hitl_generic(
                tool_name,
                &tool_args,
                &tool_id,
                ctx,
                capture_ctx,
                model,
                sub_agent_context,
            )
            .await
        }),
    )
    .await
    .unwrap_or_else(|e| ToolExecutionResult {
        value: json!({ "error": e.to_string() }),
        success: false,
    });

    // Tool Call Auto-Fixer: if execution failed with a schema/argument error,
    // try a lightweight LLM call to repair the args and retry once.
    if !result.success {
        let error_text = result
            .value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let tool_schema = {
            let registry = ctx.tool_registry.read().await;
            registry
                .get_tool_definitions()
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
        )
        .await
        {
            tracing::info!(
                "[toolcall-fixer] Retrying '{}' with repaired args",
                tool_name
            );
            result = golish_core::with_agent_tool_context(
                Some(tool_context.clone()),
                golish_core::with_agent_tool_output_sender(
                    Some(ctx.events.event_tx.clone()),
                    async {
                        execute_with_hitl_generic(
                            tool_name,
                            &fixed_args,
                            &tool_id,
                            ctx,
                            capture_ctx,
                            model,
                            sub_agent_context,
                        )
                        .await
                    },
                ),
            )
            .await
            .unwrap_or_else(|e| ToolExecutionResult {
                value: json!({ "error": e.to_string() }),
                success: false,
            });
        }
    }

    // Finish DB tracking with result
    if let (Some(tracker), Some(guard)) = (ctx.events.db_tracker, db_guard) {
        let result_text = serde_json::to_string(&result.value).unwrap_or_default();
        tracker
            .finish_tool_call(guard, result.success, &result_text)
            .await;

        // Record search logs for web search tools
        if tool_name.starts_with("tavily_") || tool_name.starts_with("web_search") {
            let query = tool_args
                .get("query")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let result_preview = serde_json::to_string(&result.value)
                .ok()
                .map(|s| truncate_str(&s, 10000).to_string());
            tracker.record_search(
                if tool_name.starts_with("tavily_") {
                    "tavily"
                } else {
                    "web"
                },
                query,
                result_preview.as_deref(),
            );
        }

        // Record terminal logs for shell/PTY commands
        if tool_name == "run_pty_cmd" || tool_name == "run_command" || tool_name == "run_shell_cmd"
        {
            let output = result
                .value
                .get("output")
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
        let skip_memory =
            if result.success && (tool_name == "run_pty_cmd" || tool_name == "run_command") {
                let cmd = tool_args
                    .get("command")
                    .and_then(|c| c.as_str())
                    .unwrap_or("");
                let stdout = result
                    .value
                    .get("stdout")
                    .and_then(|s| s.as_str())
                    .unwrap_or("");
                ctx.output_classifier
                    .as_ref()
                    .is_some_and(|classifier| classifier(cmd, stdout))
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
        result_str.clone()
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

    // RuntimeSupervisor check (PentAGI-inspired pattern): when the monitor
    // detects repeated failed or stalled tool results, generate stage-aware
    // strategy guidance.
    let supervisor_note: Option<String> = if let Some(ref monitor) = ctx.execution_monitor {
        let args_summary = serde_json::to_string(&tool_args).unwrap_or_default();
        let monitor_tool_name =
            golish_agent_kit::harness::underlying_tool_name(tool_name, &tool_args);
        let should_supervise = {
            let mut mon = monitor.write().await;
            mon.record_result_and_check(
                &monitor_tool_name,
                &args_summary,
                result.success,
                &result_str,
            )
        };
        if should_supervise {
            let (mode, repeated_tool, repeat_count, recent_summary) = {
                let mon = monitor.read().await;
                (
                    mon.mode(),
                    mon.repeated_tool_name().to_string(),
                    mon.same_tool_count(),
                    mon.recent_calls_summary(),
                )
            };
            tracing::info!(
                "[RuntimeSupervisor] Monitor recorded repeated failed tool pattern: '{}' failed {} times",
                repeated_tool,
                repeat_count,
            );
            let supervisor_ctx = RuntimeSupervisorContext {
                stage: ctx.harness_stage,
                agent_path: runtime_supervisor_agent_path(sub_agent_context),
                agent_role: if sub_agent_context.depth == 0 {
                    "main".to_string()
                } else {
                    format!("subagent_depth_{}", sub_agent_context.depth)
                },
                task: sub_agent_context.original_request.clone(),
                trigger: "execution_monitor".to_string(),
                repeated_tool: repeated_tool.clone(),
                repeat_count,
                recent_calls: recent_summary.clone(),
                last_tool_name: tool_name.clone(),
                last_tool_result: serde_json::to_string(&result.value).unwrap_or_default(),
                visible_tools: visible_tool_names(ctx).await,
                active_repair_directive: None,
            };
            let user_prompt = runtime_supervisor_user_prompt(&supervisor_ctx);
            let model_response = match runtime_supervisor_one_shot(
                ctx.llm.client,
                runtime_supervisor_system_prompt(),
                &user_prompt,
            )
            .await
            {
                Ok(response) => Some(response),
                Err(e) => {
                    tracing::warn!(
                        target: "harness::runtime_supervisor",
                        error = %e,
                        "runtime supervisor LLM call failed; using deterministic fallback"
                    );
                    None
                }
            };
            let directive =
                directive_from_model_response(&supervisor_ctx, model_response.as_deref());
            let injected = mode.injects();
            tracing::info!(
                target: "harness::runtime_supervisor",
                mode = mode.as_str(),
                repeated_tool = %repeated_tool,
                repeat_count,
                strategy_kind = directive.strategy_kind_label(),
                directive_hash = %directive.directive_hash,
                root_cause = %truncate_str(&directive.root_cause, 500),
                injected,
                "runtime supervisor decision recorded"
            );
            emit_runtime_supervisor_trace(
                ctx,
                sub_agent_context,
                mode,
                &repeated_tool,
                repeat_count,
                &directive,
                injected,
            );
            {
                let mut mon = monitor.write().await;
                mon.reset_after_supervisor();
            }
            if injected {
                Some(directive.model_instruction(matches!(mode, ExecutionMonitorMode::HardInject)))
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    // Convert result to text and truncate if necessary
    let mut raw_result_text = serde_json::to_string(&result.value).unwrap_or_default();
    if let Some(ref note) = supervisor_note {
        raw_result_text.push_str("\n\n");
        raw_result_text.push_str(note);
    }
    let truncation_result = ctx
        .context_manager
        .truncate_tool_response(&raw_result_text, tool_name)
        .await;

    let final_content = if truncation_result.truncated {
        let original_tokens = golish_context::TokenBudgetManager::estimate_tokens(&raw_result_text);
        let truncated_tokens =
            golish_context::TokenBudgetManager::estimate_tokens(&truncation_result.content);
        let _ = ctx.events.event_tx.send(AiEvent::ToolResponseTruncated {
            tool_name: tool_name.clone(),
            original_tokens,
            truncated_tokens,
        });

        // If truncated output is still large, attempt LLM summarization
        if truncated_tokens > SUMMARIZE_THRESHOLD_TOKENS {
            match summarize_tool_output(ctx.llm.client, tool_name, &truncation_result.content).await
            {
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
        ctx.events.session_id.unwrap_or(""),
    );
    let hooks = hook_registry.run_post_tool_hooks(&post_ctx);

    (user_content, hooks)
}
