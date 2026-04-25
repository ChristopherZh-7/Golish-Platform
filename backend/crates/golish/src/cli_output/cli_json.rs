use std::time::{SystemTime, UNIX_EPOCH};

use golish_core::events::AiEvent;
use serde::Serialize;

/// Standardized JSON output event for CLI.
///
/// This provides a consistent format for all CLI events that is easy to parse
/// in evaluation frameworks and scripts. Key differences from raw AiEvent:
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

/// Convert an AiEvent to the standardized CLI JSON format.
///
/// IMPORTANT: This function does NOT truncate any data. All tool inputs,
/// outputs, reasoning content, and text deltas are passed through completely.
/// Truncation is only applied in terminal mode for readability.
pub fn convert_to_cli_json(event: &AiEvent) -> CliJsonEvent {
    match event {
        AiEvent::Started { turn_id } => {
            CliJsonEvent::new("started", serde_json::json!({ "turn_id": turn_id }))
        }

        AiEvent::UserMessage { content } => {
            CliJsonEvent::new("user_message", serde_json::json!({ "content": content }))
        }

        AiEvent::TextDelta { delta, accumulated } => CliJsonEvent::new(
            "text_delta",
            serde_json::json!({
                "delta": delta,
                "accumulated": accumulated
            }),
        ),

        AiEvent::ToolRequest {
            tool_name,
            args,
            request_id,
            source,
        } => CliJsonEvent::new(
            "tool_call",
            serde_json::json!({
                "tool_name": tool_name,
                "input": args,  // Renamed from "args"
                "request_id": request_id,
                "source": source
            }),
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
            serde_json::json!({
                "request_id": request_id,
                "tool_name": tool_name,
                "input": args,  // Renamed from "args"
                "stats": stats,
                "risk_level": risk_level,
                "can_learn": can_learn,
                "suggestion": suggestion,
                "source": source
            }),
        ),

        AiEvent::ToolAutoApproved {
            request_id,
            tool_name,
            args,
            reason,
            source,
        } => CliJsonEvent::new(
            "tool_auto_approved",
            serde_json::json!({
                "request_id": request_id,
                "tool_name": tool_name,
                "input": args,  // Renamed from "args"
                "reason": reason,
                "source": source
            }),
        ),

        AiEvent::ToolDenied {
            request_id,
            tool_name,
            args,
            reason,
            source,
        } => CliJsonEvent::new(
            "tool_denied",
            serde_json::json!({
                "request_id": request_id,
                "tool_name": tool_name,
                "input": args,  // Renamed from "args"
                "reason": reason,
                "source": source
            }),
        ),

        AiEvent::ToolResult {
            tool_name,
            result,
            success,
            request_id,
            source,
        } => CliJsonEvent::new(
            "tool_result",
            serde_json::json!({
                "tool_name": tool_name,
                "output": result,  // Renamed from "result"
                "success": success,
                "request_id": request_id,
                "source": source
            }),
        ),

        AiEvent::Reasoning { content } => {
            CliJsonEvent::new("reasoning", serde_json::json!({ "content": content }))
        }

        AiEvent::Completed {
            response,
            reasoning,
            input_tokens,
            output_tokens,
            duration_ms,
        } => CliJsonEvent::new(
            "completed",
            serde_json::json!({
                "response": response,
                "reasoning": reasoning,
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
                "duration_ms": duration_ms
            }),
        ),

        AiEvent::Error {
            message,
            error_type,
        } => CliJsonEvent::new(
            "error",
            serde_json::json!({
                "message": message,
                "error_type": error_type
            }),
        ),

        // Sub-agent events
        AiEvent::SubAgentStarted {
            agent_id,
            agent_name,
            task,
            depth,
            parent_request_id,
        } => CliJsonEvent::new(
            "sub_agent_started",
            serde_json::json!({
                "agent_id": agent_id,
                "agent_name": agent_name,
                "task": task,
                "depth": depth,
                "parent_request_id": parent_request_id
            }),
        ),

        AiEvent::SubAgentToolRequest {
            agent_id,
            tool_name,
            args,
            request_id,
            parent_request_id,
        } => CliJsonEvent::new(
            "sub_agent_tool_request",
            serde_json::json!({
                "agent_id": agent_id,
                "tool_name": tool_name,
                "request_id": request_id,
                "input": args,
                "parent_request_id": parent_request_id
            }),
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
            serde_json::json!({
                "agent_id": agent_id,
                "tool_name": tool_name,
                "request_id": request_id,
                "success": success,
                "result": result,
                "parent_request_id": parent_request_id
            }),
        ),

        AiEvent::SubAgentTextDelta {
            agent_id,
            delta,
            accumulated,
            parent_request_id,
        } => CliJsonEvent::new(
            "sub_agent_text_delta",
            serde_json::json!({
                "agent_id": agent_id,
                "delta": delta,
                "accumulated": accumulated,
                "parent_request_id": parent_request_id
            }),
        ),

        AiEvent::SubAgentCompleted {
            agent_id,
            response,
            duration_ms,
            parent_request_id,
        } => CliJsonEvent::new(
            "sub_agent_completed",
            serde_json::json!({
                "agent_id": agent_id,
                "response": response,
                "duration_ms": duration_ms,
                "parent_request_id": parent_request_id
            }),
        ),

        AiEvent::SubAgentError {
            agent_id,
            error,
            parent_request_id,
        } => CliJsonEvent::new(
            "sub_agent_error",
            serde_json::json!({
                "agent_id": agent_id,
                "error": error,
                "parent_request_id": parent_request_id
            }),
        ),

        // Context management events
        AiEvent::ContextWarning {
            utilization,
            total_tokens,
            max_tokens,
        } => CliJsonEvent::new(
            "context_warning",
            serde_json::json!({
                "utilization": utilization,
                "total_tokens": total_tokens,
                "max_tokens": max_tokens
            }),
        ),

        AiEvent::ToolResponseTruncated {
            tool_name,
            original_tokens,
            truncated_tokens,
        } => CliJsonEvent::new(
            "tool_response_truncated",
            serde_json::json!({
                "tool_name": tool_name,
                "original_tokens": original_tokens,
                "truncated_tokens": truncated_tokens
            }),
        ),

        // Loop protection events
        AiEvent::LoopWarning {
            tool_name,
            current_count,
            max_count,
            message,
        } => CliJsonEvent::new(
            "loop_warning",
            serde_json::json!({
                "tool_name": tool_name,
                "current_count": current_count,
                "max_count": max_count,
                "message": message
            }),
        ),

        AiEvent::LoopBlocked {
            tool_name,
            repeat_count,
            max_count,
            message,
        } => CliJsonEvent::new(
            "loop_blocked",
            serde_json::json!({
                "tool_name": tool_name,
                "repeat_count": repeat_count,
                "max_count": max_count,
                "message": message
            }),
        ),

        AiEvent::MaxIterationsReached {
            iterations,
            max_iterations,
            message,
        } => CliJsonEvent::new(
            "max_iterations_reached",
            serde_json::json!({
                "iterations": iterations,
                "max_iterations": max_iterations,
                "message": message
            }),
        ),

        // Workflow events
        AiEvent::WorkflowStarted {
            workflow_id,
            workflow_name,
            session_id,
        } => CliJsonEvent::new(
            "workflow_started",
            serde_json::json!({
                "workflow_id": workflow_id,
                "workflow_name": workflow_name,
                "session_id": session_id
            }),
        ),

        AiEvent::WorkflowStepStarted {
            workflow_id,
            step_name,
            step_index,
            total_steps,
        } => CliJsonEvent::new(
            "workflow_step_started",
            serde_json::json!({
                "workflow_id": workflow_id,
                "step_name": step_name,
                "step_index": step_index,
                "total_steps": total_steps
            }),
        ),

        AiEvent::WorkflowStepCompleted {
            workflow_id,
            step_name,
            output,
            duration_ms,
        } => CliJsonEvent::new(
            "workflow_step_completed",
            serde_json::json!({
                "workflow_id": workflow_id,
                "step_name": step_name,
                "output": output,
                "duration_ms": duration_ms
            }),
        ),

        AiEvent::WorkflowCompleted {
            workflow_id,
            final_output,
            total_duration_ms,
        } => CliJsonEvent::new(
            "workflow_completed",
            serde_json::json!({
                "workflow_id": workflow_id,
                "final_output": final_output,
                "total_duration_ms": total_duration_ms
            }),
        ),

        AiEvent::WorkflowError {
            workflow_id,
            step_name,
            error,
        } => CliJsonEvent::new(
            "workflow_error",
            serde_json::json!({
                "workflow_id": workflow_id,
                "step_name": step_name,
                "error": error
            }),
        ),

        // Plan management events
        AiEvent::PlanUpdated {
            version,
            summary,
            steps,
            explanation,
        } => CliJsonEvent::new(
            "plan_updated",
            serde_json::json!({
                "version": version,
                "summary": summary,
                "steps": steps,
                "explanation": explanation
            }),
        ),

        // Server tool events (Claude's native web_search/web_fetch)
        AiEvent::ServerToolStarted {
            request_id,
            tool_name,
            input,
        } => CliJsonEvent::new(
            "server_tool_started",
            serde_json::json!({
                "request_id": request_id,
                "tool_name": tool_name,
                "input": input
            }),
        ),

        AiEvent::WebSearchResult {
            request_id,
            results,
        } => CliJsonEvent::new(
            "web_search_result",
            serde_json::json!({
                "request_id": request_id,
                "results": results
            }),
        ),

        AiEvent::WebFetchResult {
            request_id,
            url,
            content_preview,
        } => CliJsonEvent::new(
            "web_fetch_result",
            serde_json::json!({
                "request_id": request_id,
                "url": url,
                "content_preview": content_preview
            }),
        ),

        AiEvent::Warning { message } => {
            CliJsonEvent::new("warning", serde_json::json!({ "message": message }))
        }

        // Context compaction events
        AiEvent::CompactionStarted {
            tokens_before,
            messages_before,
        } => CliJsonEvent::new(
            "compaction_started",
            serde_json::json!({
                "tokens_before": tokens_before,
                "messages_before": messages_before
            }),
        ),

        AiEvent::CompactionCompleted {
            tokens_before,
            messages_before,
            messages_after,
            summary_length,
            ..
        } => CliJsonEvent::new(
            "compaction_completed",
            serde_json::json!({
                "tokens_before": tokens_before,
                "messages_before": messages_before,
                "messages_after": messages_after,
                "summary_length": summary_length
            }),
        ),

        AiEvent::CompactionFailed {
            tokens_before,
            messages_before,
            error,
            ..
        } => CliJsonEvent::new(
            "compaction_failed",
            serde_json::json!({
                "tokens_before": tokens_before,
                "messages_before": messages_before,
                "error": error
            }),
        ),

        AiEvent::SystemHooksInjected { hooks } => CliJsonEvent::new(
            "system_hooks_injected",
            serde_json::json!({
                "hooks": hooks
            }),
        ),

        AiEvent::ToolOutputChunk {
            request_id,
            tool_name,
            chunk,
            stream,
            source,
        } => CliJsonEvent::new(
            "tool_output_chunk",
            serde_json::json!({
                "request_id": request_id,
                "tool_name": tool_name,
                "chunk": chunk,
                "stream": stream,
                "source": source
            }),
        ),

        AiEvent::PromptGenerationStarted {
            agent_id,
            parent_request_id,
            architect_system_prompt,
            architect_user_message,
        } => CliJsonEvent::new(
            "prompt_generation_started",
            serde_json::json!({
                "agent_id": agent_id,
                "parent_request_id": parent_request_id,
                "architect_system_prompt": architect_system_prompt,
                "architect_user_message": architect_user_message
            }),
        ),

        AiEvent::PromptGenerationCompleted {
            agent_id,
            parent_request_id,
            generated_prompt,
            success,
            duration_ms,
        } => CliJsonEvent::new(
            "prompt_generation_completed",
            serde_json::json!({
                "agent_id": agent_id,
                "parent_request_id": parent_request_id,
                "generated_prompt": generated_prompt,
                "success": success,
                "duration_ms": duration_ms
            }),
        ),

        AiEvent::AskHumanRequest {
            request_id,
            question,
            input_type,
            options,
            context,
        } => CliJsonEvent::new(
            "ask_human_request",
            serde_json::json!({
                "request_id": request_id,
                "question": question,
                "input_type": input_type,
                "options": options,
                "context": context
            }),
        ),

        AiEvent::AskHumanResponse {
            request_id,
            response,
            skipped,
        } => CliJsonEvent::new(
            "ask_human_response",
            serde_json::json!({
                "request_id": request_id,
                "response": response,
                "skipped": skipped
            }),
        ),
        AiEvent::TaskProgress {
            task_id,
            status,
            message,
        } => CliJsonEvent::new(
            "task_progress",
            serde_json::json!({
                "task_id": task_id,
                "status": status,
                "message": message
            }),
        ),
        AiEvent::SubtaskCreated {
            task_id,
            subtask_id,
            title,
            agent,
        } => CliJsonEvent::new(
            "subtask_created",
            serde_json::json!({
                "task_id": task_id,
                "subtask_id": subtask_id,
                "title": title,
                "agent": agent
            }),
        ),
        AiEvent::SubtaskCompleted {
            task_id,
            subtask_id,
            title,
            result,
        } => CliJsonEvent::new(
            "subtask_completed",
            serde_json::json!({
                "task_id": task_id,
                "subtask_id": subtask_id,
                "title": title,
                "result": result
            }),
        ),

        AiEvent::SubtaskWaitingForInput {
            task_id,
            subtask_id,
            title,
            prompt,
        } => CliJsonEvent::new(
            "subtask_waiting_for_input",
            serde_json::json!({
                "task_id": task_id,
                "subtask_id": subtask_id,
                "title": title,
                "prompt": prompt
            }),
        ),

        AiEvent::SubtaskUserInput {
            task_id,
            subtask_id,
            input,
        } => CliJsonEvent::new(
            "subtask_user_input",
            serde_json::json!({
                "task_id": task_id,
                "subtask_id": subtask_id,
                "input": input
            }),
        ),

        AiEvent::TaskResumed {
            task_id,
            subtask_index,
            total_subtasks,
        } => CliJsonEvent::new(
            "task_resumed",
            serde_json::json!({
                "task_id": task_id,
                "subtask_index": subtask_index,
                "total_subtasks": total_subtasks
            }),
        ),

        AiEvent::EnricherResult {
            task_id,
            subtask_id,
            context_added,
        } => CliJsonEvent::new(
            "enricher_result",
            serde_json::json!({
                "task_id": task_id,
                "subtask_id": subtask_id,
                "context_added": context_added
            }),
        ),
    }
}
