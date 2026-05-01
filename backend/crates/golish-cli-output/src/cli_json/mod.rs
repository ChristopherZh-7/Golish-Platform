//! Standardised JSON output for the CLI.
//!
//! [`CliJsonEvent`] is the wire-format JSON object emitted to stdout in
//! `--json` mode. [`convert_to_cli_json`] is the single dispatcher that
//! converts every [`AiEvent`] variant into a [`CliJsonEvent`] by routing the
//! destructured fields to a small per-category helper module:
//!
//! - [`lifecycle`] — Started / UserMessage / Completed / Error / Warning
//! - [`streaming`] — TextDelta / Reasoning
//! - [`tools`] — main-agent tool calls + Claude server tools
//! - [`sub_agent`] — sub-agent lifecycle, streaming, tools, prompt generation
//! - [`context`] — context warnings, compaction, system-hook injections
//! - [`loop_guard`] — loop warnings, blocks, max-iteration ceilings
//! - [`workflow`] — workflow lifecycle + plan updates
//! - [`hitl`] — ask-human request / response
//! - [`task`] — PentAGI-style task / subtask events
//!
//! IMPORTANT: this module does NOT truncate any data. All tool inputs,
//! outputs, reasoning content, and text deltas are passed through completely.
//! Truncation is only applied in terminal mode for readability.

use std::time::{SystemTime, UNIX_EPOCH};

use golish_core::events::AiEvent;
use serde::Serialize;

mod context;
mod hitl;
mod lifecycle;
mod loop_guard;
mod streaming;
mod sub_agent;
mod task;
mod tools;
mod workflow;

/// Standardised JSON output event for the CLI.
///
/// Provides a consistent format for all CLI events that is easy to parse
/// in evaluation frameworks and scripts. Key differences from raw
/// [`AiEvent`]:
///
/// - Uses `event` field instead of `type`
/// - Adds `timestamp` to all events
/// - Renames `args` to `input` and `result` to `output` for tool events
/// - NO TRUNCATION of any data (truncation only happens in terminal mode)
#[derive(Debug, Serialize)]
pub struct CliJsonEvent {
    /// Event type (started, text_delta, tool_call, tool_result, reasoning, completed, error, etc.)
    event: String,

    /// Unix timestamp in milliseconds
    timestamp: u64,

    /// Event-specific data (flattened into the top-level object)
    #[serde(flatten)]
    data: serde_json::Value,
}

impl CliJsonEvent {
    /// Create a new CLI JSON event with current timestamp.
    fn new(event: &str, data: serde_json::Value) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        Self {
            event: event.to_string(),
            timestamp,
            data,
        }
    }
}

/// Convert an [`AiEvent`] to the standardised CLI JSON format.
///
/// IMPORTANT: this function does NOT truncate any data. All tool inputs,
/// outputs, reasoning content, and text deltas are passed through completely.
/// Truncation is only applied in terminal mode for readability.
pub fn convert_to_cli_json(event: &AiEvent) -> CliJsonEvent {
    match event {
        // ── lifecycle ────────────────────────────────────────────────────
        AiEvent::Started { turn_id } => {
            CliJsonEvent::new("started", lifecycle::started(turn_id))
        }
        AiEvent::UserMessage { content } => {
            CliJsonEvent::new("user_message", lifecycle::user_message(content))
        }
        AiEvent::Completed {
            response,
            reasoning,
            input_tokens,
            output_tokens,
            duration_ms,
        } => CliJsonEvent::new(
            "completed",
            lifecycle::completed(response, reasoning, *input_tokens, *output_tokens, *duration_ms),
        ),
        AiEvent::Error {
            message,
            error_type,
        } => CliJsonEvent::new("error", lifecycle::error(message, error_type)),
        AiEvent::Warning { message } => {
            CliJsonEvent::new("warning", lifecycle::warning(message))
        }

        // ── streaming ────────────────────────────────────────────────────
        AiEvent::TextDelta { delta, accumulated } => {
            CliJsonEvent::new("text_delta", streaming::text_delta(delta, accumulated))
        }
        AiEvent::Reasoning { content } => {
            CliJsonEvent::new("reasoning", streaming::reasoning(content))
        }

        // ── tools (main-agent + Claude server tools) ─────────────────────
        AiEvent::ToolRequest {
            tool_name,
            args,
            request_id,
            source,
        } => CliJsonEvent::new(
            "tool_call",
            tools::tool_request(tool_name, args, request_id, source),
        ),
        AiEvent::ToolApprovalRequest {
            request_id,
            tool_name,
            args,
            stats,
            risk_level,
            can_learn,
            suggestion,
            source,
        } => CliJsonEvent::new(
            "tool_approval",
            tools::tool_approval_request(
                request_id,
                tool_name,
                args,
                stats,
                risk_level,
                *can_learn,
                suggestion,
                source,
            ),
        ),
        AiEvent::ToolAutoApproved {
            request_id,
            tool_name,
            args,
            reason,
            source,
        } => CliJsonEvent::new(
            "tool_auto_approved",
            tools::tool_auto_approved(request_id, tool_name, args, reason, source),
        ),
        AiEvent::ToolDenied {
            request_id,
            tool_name,
            args,
            reason,
            source,
        } => CliJsonEvent::new(
            "tool_denied",
            tools::tool_denied(request_id, tool_name, args, reason, source),
        ),
        AiEvent::ToolResult {
            tool_name,
            result,
            success,
            request_id,
            source,
        } => CliJsonEvent::new(
            "tool_result",
            tools::tool_result(tool_name, result, *success, request_id, source),
        ),
        AiEvent::ToolOutputChunk {
            request_id,
            tool_name,
            chunk,
            stream,
            source,
        } => CliJsonEvent::new(
            "tool_output_chunk",
            tools::tool_output_chunk(request_id, tool_name, chunk, stream, source),
        ),
        AiEvent::ToolResponseTruncated {
            tool_name,
            original_tokens,
            truncated_tokens,
        } => CliJsonEvent::new(
            "tool_response_truncated",
            tools::tool_response_truncated(tool_name, *original_tokens, *truncated_tokens),
        ),
        AiEvent::ServerToolStarted {
            request_id,
            tool_name,
            input,
        } => CliJsonEvent::new(
            "server_tool_started",
            tools::server_tool_started(request_id, tool_name, input),
        ),
        AiEvent::WebSearchResult {
            request_id,
            results,
        } => CliJsonEvent::new(
            "web_search_result",
            tools::web_search_result(request_id, results),
        ),
        AiEvent::WebFetchResult {
            request_id,
            url,
            content_preview,
        } => CliJsonEvent::new(
            "web_fetch_result",
            tools::web_fetch_result(request_id, url, content_preview),
        ),

        // ── sub-agent ────────────────────────────────────────────────────
        AiEvent::SubAgentStarted {
            agent_id,
            agent_name,
            task,
            depth,
            parent_request_id,
        } => CliJsonEvent::new(
            "sub_agent_started",
            sub_agent::sub_agent_started(agent_id, agent_name, task, *depth, parent_request_id),
        ),
        AiEvent::SubAgentToolRequest {
            agent_id,
            tool_name,
            args,
            request_id,
            parent_request_id,
        } => CliJsonEvent::new(
            "sub_agent_tool_request",
            sub_agent::sub_agent_tool_request(
                agent_id,
                tool_name,
                args,
                request_id,
                parent_request_id,
            ),
        ),
        AiEvent::SubAgentToolResult {
            agent_id,
            tool_name,
            success,
            result,
            request_id,
            parent_request_id,
        } => CliJsonEvent::new(
            "sub_agent_tool_result",
            sub_agent::sub_agent_tool_result(
                agent_id,
                tool_name,
                *success,
                result,
                request_id,
                parent_request_id,
            ),
        ),
        AiEvent::SubAgentTextDelta {
            agent_id,
            delta,
            accumulated,
            parent_request_id,
        } => CliJsonEvent::new(
            "sub_agent_text_delta",
            sub_agent::sub_agent_text_delta(agent_id, delta, accumulated, parent_request_id),
        ),
        AiEvent::SubAgentCompleted {
            agent_id,
            response,
            duration_ms,
            parent_request_id,
        } => CliJsonEvent::new(
            "sub_agent_completed",
            sub_agent::sub_agent_completed(agent_id, response, *duration_ms, parent_request_id),
        ),
        AiEvent::SubAgentError {
            agent_id,
            error,
            parent_request_id,
        } => CliJsonEvent::new(
            "sub_agent_error",
            sub_agent::sub_agent_error(agent_id, error, parent_request_id),
        ),
        AiEvent::PromptGenerationStarted {
            agent_id,
            parent_request_id,
            architect_system_prompt,
            architect_user_message,
        } => CliJsonEvent::new(
            "prompt_generation_started",
            sub_agent::prompt_generation_started(
                agent_id,
                parent_request_id,
                architect_system_prompt,
                architect_user_message,
            ),
        ),
        AiEvent::PromptGenerationCompleted {
            agent_id,
            parent_request_id,
            generated_prompt,
            success,
            duration_ms,
        } => CliJsonEvent::new(
            "prompt_generation_completed",
            sub_agent::prompt_generation_completed(
                agent_id,
                parent_request_id,
                generated_prompt,
                *success,
                *duration_ms,
            ),
        ),

        // ── context management & compaction ──────────────────────────────
        AiEvent::ContextWarning {
            utilization,
            total_tokens,
            max_tokens,
        } => CliJsonEvent::new(
            "context_warning",
            context::context_warning(*utilization, *total_tokens, *max_tokens),
        ),
        AiEvent::CompactionStarted {
            tokens_before,
            messages_before,
        } => CliJsonEvent::new(
            "compaction_started",
            context::compaction_started(*tokens_before, *messages_before),
        ),
        AiEvent::CompactionCompleted {
            tokens_before,
            messages_before,
            messages_after,
            summary_length,
            ..
        } => CliJsonEvent::new(
            "compaction_completed",
            context::compaction_completed(
                *tokens_before,
                *messages_before,
                *messages_after,
                *summary_length,
            ),
        ),
        AiEvent::CompactionFailed {
            tokens_before,
            messages_before,
            error,
            ..
        } => CliJsonEvent::new(
            "compaction_failed",
            context::compaction_failed(*tokens_before, *messages_before, error),
        ),
        AiEvent::SystemHooksInjected { hooks } => CliJsonEvent::new(
            "system_hooks_injected",
            context::system_hooks_injected(hooks),
        ),

        // ── loop protection ──────────────────────────────────────────────
        AiEvent::LoopWarning {
            tool_name,
            current_count,
            max_count,
            message,
        } => CliJsonEvent::new(
            "loop_warning",
            loop_guard::loop_warning(tool_name, *current_count, *max_count, message),
        ),
        AiEvent::LoopBlocked {
            tool_name,
            repeat_count,
            max_count,
            message,
        } => CliJsonEvent::new(
            "loop_blocked",
            loop_guard::loop_blocked(tool_name, *repeat_count, *max_count, message),
        ),
        AiEvent::MaxIterationsReached {
            iterations,
            max_iterations,
            message,
        } => CliJsonEvent::new(
            "max_iterations_reached",
            loop_guard::max_iterations_reached(*iterations, *max_iterations, message),
        ),

        // ── workflow + plan ──────────────────────────────────────────────
        AiEvent::WorkflowStarted {
            workflow_id,
            workflow_name,
            session_id,
        } => CliJsonEvent::new(
            "workflow_started",
            workflow::workflow_started(workflow_id, workflow_name, session_id),
        ),
        AiEvent::WorkflowStepStarted {
            workflow_id,
            step_name,
            step_index,
            total_steps,
        } => CliJsonEvent::new(
            "workflow_step_started",
            workflow::workflow_step_started(workflow_id, step_name, *step_index, *total_steps),
        ),
        AiEvent::WorkflowStepCompleted {
            workflow_id,
            step_name,
            output,
            duration_ms,
        } => CliJsonEvent::new(
            "workflow_step_completed",
            workflow::workflow_step_completed(workflow_id, step_name, output, *duration_ms),
        ),
        AiEvent::WorkflowCompleted {
            workflow_id,
            final_output,
            total_duration_ms,
        } => CliJsonEvent::new(
            "workflow_completed",
            workflow::workflow_completed(workflow_id, final_output, *total_duration_ms),
        ),
        AiEvent::WorkflowError {
            workflow_id,
            step_name,
            error,
        } => CliJsonEvent::new(
            "workflow_error",
            workflow::workflow_error(workflow_id, step_name, error),
        ),
        AiEvent::PlanUpdated {
            version,
            summary,
            steps,
            explanation,
        } => CliJsonEvent::new(
            "plan_updated",
            workflow::plan_updated(*version, summary, steps, explanation),
        ),

        // ── HITL ─────────────────────────────────────────────────────────
        AiEvent::AskHumanRequest {
            request_id,
            question,
            input_type,
            options,
            context,
        } => CliJsonEvent::new(
            "ask_human_request",
            hitl::ask_human_request(request_id, question, input_type, options, context),
        ),
        AiEvent::AskHumanResponse {
            request_id,
            response,
            skipped,
        } => CliJsonEvent::new(
            "ask_human_response",
            hitl::ask_human_response(request_id, response, *skipped),
        ),

        // ── task mode ────────────────────────────────────────────────────
        AiEvent::TaskProgress {
            task_id,
            status,
            message,
        } => CliJsonEvent::new("task_progress", task::task_progress(task_id, status, message)),
        AiEvent::SubtaskCreated {
            task_id,
            subtask_id,
            title,
            agent,
        } => CliJsonEvent::new(
            "subtask_created",
            task::subtask_created(task_id, subtask_id, title, agent),
        ),
        AiEvent::SubtaskCompleted {
            task_id,
            subtask_id,
            title,
            result,
        } => CliJsonEvent::new(
            "subtask_completed",
            task::subtask_completed(task_id, subtask_id, title, result),
        ),
        AiEvent::SubtaskWaitingForInput {
            task_id,
            subtask_id,
            title,
            prompt,
        } => CliJsonEvent::new(
            "subtask_waiting_for_input",
            task::subtask_waiting_for_input(task_id, subtask_id, title, prompt),
        ),
        AiEvent::SubtaskUserInput {
            task_id,
            subtask_id,
            input,
        } => CliJsonEvent::new(
            "subtask_user_input",
            task::subtask_user_input(task_id, subtask_id, input),
        ),
        AiEvent::TaskResumed {
            task_id,
            subtask_index,
            total_subtasks,
        } => CliJsonEvent::new(
            "task_resumed",
            task::task_resumed(task_id, *subtask_index, *total_subtasks),
        ),
        AiEvent::EnricherResult {
            task_id,
            subtask_id,
            context_added,
        } => CliJsonEvent::new(
            "enricher_result",
            task::enricher_result(task_id, subtask_id, context_added),
        ),
    }
}
