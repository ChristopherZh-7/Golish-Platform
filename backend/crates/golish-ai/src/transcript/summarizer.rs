use std::path::{Path, PathBuf};
use golish_core::utils::truncate_head_tail;
use golish_core::events::AiEvent;
use super::{TranscriptEvent, read_transcript};

pub fn format_for_summarizer(events: &[TranscriptEvent]) -> String {
    let mut output = String::new();
    let mut current_turn: u32 = 0;

    for te in events {
        match &te.event {
            AiEvent::Started { .. } => {
                current_turn += 1;
                // Don't output anything for Started - the turn header comes with content
            }

            AiEvent::UserMessage { content } => {
                output.push_str(&format!(
                    "[turn {:03}] USER:\n{}\n\n",
                    current_turn, content
                ));
            }

            AiEvent::Completed {
                response,
                input_tokens,
                output_tokens,
                ..
            } => {
                // Reasoning/thinking is intentionally excluded — it's the model's
                // internal chain-of-thought, already reflected in the response text,
                // and would waste summarizer context budget.
                output.push_str(&format!(
                    "[turn {:03}] ASSISTANT ({} in / {} out tokens):\n{}\n\n",
                    current_turn,
                    input_tokens.unwrap_or(0),
                    output_tokens.unwrap_or(0),
                    response
                ));
            }

            AiEvent::ToolRequest {
                tool_name,
                args,
                request_id,
                ..
            } => {
                let args_str = serde_json::to_string_pretty(args).unwrap_or_default();
                output.push_str(&format!(
                    "[turn {:03}] TOOL_REQUEST (tool={}, id={}):\n{}\n\n",
                    current_turn, tool_name, request_id, args_str
                ));
            }

            AiEvent::ToolResult {
                tool_name,
                result,
                success,
                ..
            } => {
                let result_str = if let Some(s) = result.as_str() {
                    s.to_string()
                } else {
                    serde_json::to_string_pretty(result).unwrap_or_default()
                };
                // Truncate very long results using head+tail strategy (preserves start and end)
                let result_display = truncate_head_tail(&result_str, 4000);
                output.push_str(&format!(
                    "[turn {:03}] TOOL_RESULT (tool={}, success={}):\n{}\n\n",
                    current_turn, tool_name, success, result_display
                ));
            }

            AiEvent::ToolApprovalRequest {
                tool_name,
                args,
                risk_level,
                ..
            } => {
                let args_str = serde_json::to_string_pretty(args).unwrap_or_default();
                output.push_str(&format!(
                    "[turn {:03}] TOOL_APPROVAL_REQUEST (tool={}, risk={:?}):\n{}\n\n",
                    current_turn, tool_name, risk_level, args_str
                ));
            }

            AiEvent::ToolAutoApproved {
                tool_name, reason, ..
            } => {
                output.push_str(&format!(
                    "[turn {:03}] TOOL_AUTO_APPROVED (tool={}): {}\n\n",
                    current_turn, tool_name, reason
                ));
            }

            AiEvent::ToolDenied {
                tool_name, reason, ..
            } => {
                output.push_str(&format!(
                    "[turn {:03}] TOOL_DENIED (tool={}): {}\n\n",
                    current_turn, tool_name, reason
                ));
            }

            AiEvent::Error {
                message,
                error_type,
            } => {
                output.push_str(&format!(
                    "[turn {:03}] ERROR ({}): {}\n\n",
                    current_turn, error_type, message
                ));
            }

            AiEvent::SubAgentStarted {
                agent_name, task, ..
            } => {
                output.push_str(&format!(
                    "[turn {:03}] SUB_AGENT_STARTED (agent={}):\n{}\n\n",
                    current_turn, agent_name, task
                ));
            }

            AiEvent::SubAgentCompleted {
                agent_id, response, ..
            } => {
                // Truncate long sub-agent responses using head+tail strategy
                let response_display = truncate_head_tail(response, 6000);
                output.push_str(&format!(
                    "[turn {:03}] SUB_AGENT_COMPLETED (agent={}):\n{}\n\n",
                    current_turn, agent_id, response_display
                ));
            }

            AiEvent::SubAgentError {
                agent_id, error, ..
            } => {
                output.push_str(&format!(
                    "[turn {:03}] SUB_AGENT_ERROR (agent={}): {}\n\n",
                    current_turn, agent_id, error
                ));
            }

            // Skip these events - streaming or not useful for summarization
            AiEvent::TextDelta { .. } => {}
            AiEvent::Reasoning { .. } => {}
            AiEvent::ContextWarning { .. } => {}
            AiEvent::ToolResponseTruncated { .. } => {}
            AiEvent::LoopWarning { .. } => {}
            AiEvent::LoopBlocked { .. } => {}
            AiEvent::MaxIterationsReached { .. } => {}
            AiEvent::Warning { .. } => {}
            AiEvent::SubAgentToolRequest { .. } => {} // Too verbose
            AiEvent::SubAgentToolResult { .. } => {}  // Too verbose
            AiEvent::WorkflowStarted { .. } => {}
            AiEvent::WorkflowStepStarted { .. } => {}
            AiEvent::WorkflowStepCompleted { .. } => {}
            AiEvent::WorkflowCompleted { .. } => {}
            AiEvent::WorkflowError { .. } => {}
            AiEvent::PlanUpdated { .. } => {}
            AiEvent::ServerToolStarted { .. } => {}
            AiEvent::WebSearchResult { .. } => {}
            AiEvent::WebFetchResult { .. } => {}
            AiEvent::CompactionStarted { .. } => {}
            AiEvent::CompactionCompleted { .. } => {}
            AiEvent::CompactionFailed { .. } => {}
            AiEvent::SystemHooksInjected { .. } => {}
            AiEvent::ToolOutputChunk { .. } => {} // Streaming output, not needed for summarization
            AiEvent::PromptGenerationStarted { .. } => {} // Internal sub-agent detail
            AiEvent::PromptGenerationCompleted { .. } => {} // Internal sub-agent detail
            AiEvent::SubAgentTextDelta { .. } => {} // Streaming delta, not needed for summarization
            AiEvent::AskHumanRequest { question, input_type, .. } => {
                output.push_str(&format!("\n**[Ask Human - {}]** {}\n", input_type, question));
            }
            AiEvent::AskHumanResponse { response, skipped, .. } => {
                if *skipped {
                    output.push_str("\n*[User skipped the question]*\n");
                } else {
                    output.push_str(&format!("\n**[User Response]** {}\n", response));
                }
            }
            AiEvent::TaskProgress { status, message, .. } => {
                output.push_str(&format!("\n**[Task {}]** {}\n", status, message));
            }
            AiEvent::SubtaskCreated { title, agent, .. } => {
                let agent_str = agent.as_deref().unwrap_or("auto");
                output.push_str(&format!("\n**[Subtask Created]** {} (agent: {})\n", title, agent_str));
            }
            AiEvent::SubtaskCompleted { title, result, .. } => {
                output.push_str(&format!("\n**[Subtask Completed]** {}\n{}\n", title, result));
            }
            AiEvent::SubtaskWaitingForInput { title, prompt, .. } => {
                output.push_str(&format!("\n**[Waiting for Input]** {}: {}\n", title, prompt));
            }
            AiEvent::SubtaskUserInput { input, .. } => {
                output.push_str(&format!("\n**[User Input]** {}\n", input));
            }
            AiEvent::TaskResumed { subtask_index, total_subtasks, .. } => {
                output.push_str(&format!("\n**[Task Resumed]** from subtask {}/{}\n", subtask_index, total_subtasks));
            }
            AiEvent::EnricherResult { context_added, .. } => {
                output.push_str(&format!("\n**[Enricher]** {}\n", context_added));
            }
        }
    }

    output
}

/// Build summarizer input from a session's transcript.
///
/// This is the main entry point - reads the transcript file and formats it.
///
/// # Arguments
///
/// * `base_dir` - The base directory for transcripts (e.g., `~/.golish/transcripts`)
/// * `session_id` - The unique identifier for the session
///
/// # Returns
///
/// A formatted string suitable for the summarizer agent.
///
/// # Errors
///

pub async fn build_summarizer_input(base_dir: &Path, session_id: &str) -> anyhow::Result<String> {
    let events = read_transcript(base_dir, session_id).await?;
    Ok(format_for_summarizer(&events))
}

/// Save summarizer input to an artifact file.
///
/// Creates the directory if it doesn't exist and writes the content to a
/// timestamped file for debugging and auditing purposes.
///
/// # Arguments
///
/// * `base_dir` - The base directory for artifacts (e.g., `~/.golish/artifacts/compaction`)
/// * `session_id` - The unique identifier for the session
/// * `content` - The summarizer input content to save
///
/// # Returns
///
/// The path to the saved file.
///
/// # Errors
///
/// Returns an error if the directory cannot be created or the file cannot be written.
pub fn save_summarizer_input(
    base_dir: &Path,
    session_id: &str,
    content: &str,
) -> anyhow::Result<PathBuf> {
    // Ensure the directory exists
    std::fs::create_dir_all(base_dir)?;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let filename = format!("summarizer-input-{}-{}.md", session_id, timestamp);
    let path = base_dir.join(filename);

    std::fs::write(&path, content)?;

    Ok(path)
}

/// Save a summary to an artifact file.
///
/// Creates the directory if it doesn't exist and writes the summary to a
/// timestamped file for debugging and auditing purposes.
///
/// # Arguments
///
/// * `base_dir` - The base directory for artifacts (e.g., `~/.golish/artifacts/summaries`)
/// * `session_id` - The unique identifier for the session
/// * `summary` - The summary content to save
///
/// # Returns
///
/// The path to the saved file.
///
/// # Errors
///
/// Returns an error if the directory cannot be created or the file cannot be written.
pub fn save_summary(base_dir: &Path, session_id: &str, summary: &str) -> anyhow::Result<PathBuf> {
    // Ensure the directory exists
    std::fs::create_dir_all(base_dir)?;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let filename = format!("summary-{}-{}.md", session_id, timestamp);
    let path = base_dir.join(filename);

    std::fs::write(&path, summary)?;

    Ok(path)
}

