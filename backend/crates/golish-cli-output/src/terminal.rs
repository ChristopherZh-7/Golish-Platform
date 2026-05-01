use std::io::{self, Write};

use anyhow::Result;
use golish_core::events::AiEvent;

use super::formatting::{
    format_json_pretty, truncate, truncate_output, BOX_BOT, BOX_MID, BOX_TOP, TERMINAL_REASONING_MAX,
    TERMINAL_TOOL_OUTPUT_MAX,
};

/// Handle AI events for terminal (non-JSON) output.
///
/// Uses box-drawing characters for enhanced readability. Tool inputs are shown
/// in full, while tool outputs and reasoning are truncated for terminal display.
pub(super) fn handle_ai_event_terminal(event: &AiEvent) -> Result<()> {
    match event {
        AiEvent::Started { .. } => {
            // Optionally show a spinner or indicator
        }
        AiEvent::TextDelta { delta, .. } => {
            // Stream text as it arrives
            print!("{}", delta);
            io::stdout().flush()?;
        }
        // ─── Tool Request (box-drawing format with full input) ───
        AiEvent::ToolRequest {
            tool_name, args, ..
        } => {
            eprintln!();
            eprintln!("\x1b[2m{}\x1b[0m tool: {}", BOX_TOP, tool_name);
            eprintln!("\x1b[2m{}\x1b[0m input:", BOX_MID);
            for line in format_json_pretty(args).lines() {
                eprintln!("\x1b[2m{}\x1b[0m   {}", BOX_MID, line);
            }
            eprintln!("\x1b[2m{}\x1b[0m", BOX_BOT);
        }
        // ─── Tool Approval Request (shows risk level) ───
        AiEvent::ToolApprovalRequest {
            tool_name,
            args,
            risk_level,
            ..
        } => {
            let risk_str = format!("{:?}", risk_level).to_lowercase();
            eprintln!();
            eprintln!(
                "\x1b[2m{}\x1b[0m tool: {} \x1b[33m[{}]\x1b[0m",
                BOX_TOP, tool_name, risk_str
            );
            eprintln!("\x1b[2m{}\x1b[0m input:", BOX_MID);
            for line in format_json_pretty(args).lines() {
                eprintln!("\x1b[2m{}\x1b[0m   {}", BOX_MID, line);
            }
            eprintln!("\x1b[2m{}\x1b[0m", BOX_BOT);
        }
        AiEvent::ToolAutoApproved {
            tool_name, reason, ..
        } => {
            eprintln!("\x1b[32m[auto-approved]\x1b[0m {} ({})", tool_name, reason);
        }
        AiEvent::ToolDenied {
            tool_name, reason, ..
        } => {
            eprintln!("\x1b[31m[denied]\x1b[0m {} ({})", tool_name, reason);
        }
        // ─── Tool Result (box-drawing format with truncated output) ───
        AiEvent::ToolResult {
            tool_name,
            result,
            success,
            ..
        } => {
            let icon = if *success {
                "\x1b[32m+\x1b[0m"
            } else {
                "\x1b[31m!\x1b[0m"
            };
            eprintln!();
            eprintln!("\x1b[2m{}\x1b[0m {} {}", BOX_TOP, icon, tool_name);
            eprintln!("\x1b[2m{}\x1b[0m output:", BOX_MID);

            // Format and truncate output for terminal readability
            let output_str = format_json_pretty(result);
            let output_chars = output_str.chars().count();
            let truncated = truncate_output(&output_str, TERMINAL_TOOL_OUTPUT_MAX);

            for line in truncated.lines() {
                eprintln!("\x1b[2m{}\x1b[0m   {}", BOX_MID, line);
            }

            if output_chars > TERMINAL_TOOL_OUTPUT_MAX {
                eprintln!(
                    "\x1b[2m{}\x1b[0m   \x1b[2m... ({} chars total)\x1b[0m",
                    BOX_MID, output_chars
                );
            }
            eprintln!("\x1b[2m{}\x1b[0m", BOX_BOT);
        }
        // ─── Reasoning (box-drawing format with truncated content) ───
        AiEvent::Reasoning { content } => {
            eprintln!();
            eprintln!("\x1b[2m{}\x1b[0m \x1b[36mreasoning\x1b[0m", BOX_TOP);

            // Truncate reasoning for terminal readability
            let content_chars = content.chars().count();
            let truncated = truncate_output(content, TERMINAL_REASONING_MAX);

            for line in truncated.lines() {
                eprintln!("\x1b[2m{}\x1b[0m {}", BOX_MID, line);
            }

            if content_chars > TERMINAL_REASONING_MAX {
                eprintln!(
                    "\x1b[2m{}\x1b[0m \x1b[2m... ({} chars total)\x1b[0m",
                    BOX_MID, content_chars
                );
            }
            eprintln!("\x1b[2m{}\x1b[0m", BOX_BOT);
        }
        AiEvent::SubAgentStarted {
            agent_name, task, ..
        } => {
            eprintln!(
                "[sub-agent] {} starting: {}",
                agent_name,
                truncate(task, 80)
            );
        }
        AiEvent::SubAgentTextDelta { .. } => {
            // Streaming delta — not shown in terminal mode
        }
        AiEvent::SubAgentCompleted {
            response,
            duration_ms,
            ..
        } => {
            eprintln!(
                "[sub-agent] completed in {}ms: {}",
                duration_ms,
                truncate(response, 80)
            );
        }
        AiEvent::SubAgentError {
            agent_id, error, ..
        } => {
            eprintln!("[sub-agent] {} error: {}", agent_id, error);
        }
        AiEvent::ContextWarning {
            utilization,
            total_tokens,
            max_tokens,
        } => {
            eprintln!(
                "[context] Warning: {:.1}% used ({}/{})",
                utilization * 100.0,
                total_tokens,
                max_tokens
            );
        }
        AiEvent::LoopWarning {
            tool_name,
            current_count,
            max_count,
            ..
        } => {
            eprintln!(
                "[loop] Warning: {} called {}/{} times",
                tool_name, current_count, max_count
            );
        }
        AiEvent::LoopBlocked {
            tool_name, message, ..
        } => {
            eprintln!("[loop] Blocked: {} - {}", tool_name, message);
        }
        AiEvent::WorkflowStarted { workflow_name, .. } => {
            eprintln!("[workflow] Starting: {}", workflow_name);
        }
        AiEvent::WorkflowStepStarted {
            step_name,
            step_index,
            total_steps,
            ..
        } => {
            eprintln!(
                "[workflow] Step {}/{}: {}",
                step_index + 1,
                total_steps,
                step_name
            );
        }
        AiEvent::WorkflowCompleted {
            final_output,
            total_duration_ms,
            ..
        } => {
            eprintln!(
                "[workflow] Completed in {}ms: {}",
                total_duration_ms,
                truncate(final_output, 100)
            );
        }
        AiEvent::WorkflowError { error, .. } => {
            eprintln!("[workflow] Error: {}", error);
        }
        // Context compaction events
        AiEvent::CompactionStarted {
            tokens_before,
            messages_before,
        } => {
            eprintln!(
                "[compaction] Starting context compaction ({} tokens, {} messages)...",
                tokens_before, messages_before
            );
        }
        AiEvent::CompactionCompleted {
            tokens_before,
            messages_before,
            messages_after,
            summary_length,
            ..
        } => {
            eprintln!(
                "[compaction] Completed: {} tokens, {} -> {} messages (summary: {} chars)",
                tokens_before, messages_before, messages_after, summary_length
            );
        }
        AiEvent::CompactionFailed {
            tokens_before,
            messages_before,
            error,
            ..
        } => {
            eprintln!(
                "[compaction] Failed ({} tokens, {} messages): {}",
                tokens_before, messages_before, error
            );
        }
        // Events handled in the main match or not displayed in terminal mode
        AiEvent::Completed { .. } | AiEvent::Error { .. } => {}
        _ => {}
    }

    Ok(())
}
