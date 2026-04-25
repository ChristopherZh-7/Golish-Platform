//! [`CaptureContext`] — bridge between the agentic loop and the sidecar
//! event capture pipeline.
//!
//! Per-turn `process(&mut self, event)` consumes `AiEvent`s, correlates tool
//! requests with results, generates unified diffs for edits, and forwards
//! structured `SessionEvent`s to the sidecar.

use std::path::PathBuf;
use std::sync::Arc;

use tracing::{debug, info, trace};

use golish_core::events::AiEvent;

use super::super::events::{FeedbackType, FileOperation, SessionEvent};
use super::super::state::SidecarState;

use super::diff::generate_unified_diff;
use super::extractors::{
    extract_files_from_result, extract_files_modified, extract_path_from_args, extract_tool_output,
};
use super::format::{infer_decision_type, summarize_args, truncate};
use super::tool_classification::{is_edit_tool, is_read_tool, is_write_tool};

/// Maximum length for tool output storage
pub(super) const MAX_TOOL_OUTPUT_LEN: usize = 2000;
/// Maximum length for diff storage
pub(super) const MAX_DIFF_LEN: usize = 4000;


/// Capture bridge that processes AI events and forwards them to the sidecar
pub struct CaptureContext {
    /// Reference to sidecar state
    sidecar: Arc<SidecarState>,
    /// Last tool name (for correlating requests with results)
    pub(super) last_tool_name: Option<String>,
    /// Last tool args (for file operations)
    pub(super) last_tool_args: Option<serde_json::Value>,
    /// Pending old content for generating diffs (path -> content)
    pub(super) pending_old_content: Option<(PathBuf, String)>,
    /// Accumulated reasoning from streaming chunks (flushed on turn end)
    pub(super) accumulated_reasoning: String,
}

impl CaptureContext {
    /// Create a new capture context
    pub fn new(sidecar: Arc<SidecarState>) -> Self {
        Self {
            sidecar,
            last_tool_name: None,
            last_tool_args: None,
            pending_old_content: None,
            accumulated_reasoning: String::new(),
        }
    }

    /// Flush accumulated reasoning as a single event
    fn flush_reasoning(&mut self, session_id: &str) {
        if !self.accumulated_reasoning.is_empty() {
            let decision_type = infer_decision_type(&self.accumulated_reasoning);
            let event = SessionEvent::reasoning(
                session_id.to_string(),
                &self.accumulated_reasoning,
                decision_type,
            );
            self.sidecar.capture(event);
            self.accumulated_reasoning.clear();
        }
    }

    /// Process an AI event and capture relevant information
    pub fn process(&mut self, event: &AiEvent) {
        // Skip if no active session
        let session_id = match self.sidecar.current_session_id() {
            Some(id) => id,
            None => {
                trace!("[sidecar-capture] No active session, skipping event");
                return;
            }
        };

        match event {
            AiEvent::ToolRequest {
                tool_name, args, ..
            } => {
                debug!("[sidecar-capture] Tool request: {}", tool_name);
                // Store for later correlation with result
                self.last_tool_name = Some(tool_name.clone());
                self.last_tool_args = Some(args.clone());

                // For edit operations, try to capture old content for diff generation
                if is_edit_tool(tool_name) {
                    if let Some(path) = extract_path_from_args(args) {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            self.pending_old_content = Some((path, content));
                        }
                    }
                }
            }

            AiEvent::ToolResult {
                tool_name,
                result,
                success,
                ..
            } => {
                debug!(
                    "[sidecar-capture] Tool result: {} success={}",
                    tool_name, success
                );

                // Capture file operations
                if let Some(event) = self.create_file_event(&session_id, tool_name, *success) {
                    debug!("[sidecar-capture] Captured file event for {}", tool_name);
                    self.sidecar.capture(event);
                }

                // Extract tool output
                let tool_output = extract_tool_output(result);

                // Extract files_accessed for read operations
                let files_accessed = if is_read_tool(tool_name) {
                    extract_files_from_result(tool_name, &self.last_tool_args, result)
                } else {
                    None
                };

                // Extract files_modified for write operations
                let is_write = is_write_tool(tool_name);
                info!(
                    "[sidecar-capture] ToolResult: {} is_write={} success={} has_args={}",
                    tool_name,
                    is_write,
                    success,
                    self.last_tool_args.is_some()
                );
                let files_modified = if is_write && *success {
                    let files = extract_files_modified(tool_name, self.last_tool_args.as_ref());
                    if files.is_empty() {
                        info!(
                            "[sidecar-capture] No files extracted for write tool {} (args: {:?})",
                            tool_name,
                            self.last_tool_args.as_ref().map(|a| a
                                .to_string()
                                .chars()
                                .take(200)
                                .collect::<String>())
                        );
                    } else {
                        info!(
                            "[sidecar-capture] Extracted {} files for write tool {}: {:?}",
                            files.len(),
                            tool_name,
                            files
                        );
                    }
                    files
                } else {
                    vec![]
                };

                // Generate diff for edit operations
                let diff = if is_edit_tool(tool_name) && *success {
                    self.generate_diff(tool_name, &self.last_tool_args)
                } else {
                    None
                };

                // Capture tool call summary with enhanced data
                let args_summary = self.last_tool_args.as_ref().map(summarize_args);
                let mut event = SessionEvent::tool_call_with_output(
                    session_id.clone(),
                    tool_name.clone(),
                    args_summary,
                    None,
                    *success,
                    tool_output,
                    diff,
                );

                // Add files_accessed if present
                if let Some(files) = files_accessed {
                    event.files_accessed = Some(files);
                }

                // Add files_modified if present
                if !files_modified.is_empty() {
                    event.files_modified = files_modified;
                }

                self.sidecar.capture(event);

                // Clear pending state
                self.last_tool_name = None;
                self.last_tool_args = None;
                self.pending_old_content = None;
            }

            AiEvent::Reasoning { content } => {
                // Accumulate reasoning chunks - will be flushed on turn completion
                // This avoids flooding the sidecar with per-chunk events during streaming
                trace!(
                    "[sidecar-capture] Accumulating reasoning chunk: {} chars (total: {})",
                    content.len(),
                    self.accumulated_reasoning.len() + content.len()
                );
                self.accumulated_reasoning.push_str(content);
            }

            AiEvent::ToolApprovalRequest { tool_name, .. } => {
                debug!("[sidecar-capture] Tool approval request: {}", tool_name);
                // We'll capture the actual feedback when user responds
            }

            AiEvent::ToolAutoApproved {
                tool_name, reason, ..
            } => {
                debug!("[sidecar-capture] Tool auto-approved: {}", tool_name);
                let event = SessionEvent::feedback(
                    session_id,
                    FeedbackType::Approve,
                    Some(tool_name.clone()),
                    Some(format!("Auto-approved: {}", reason)),
                );
                self.sidecar.capture(event);
            }

            AiEvent::ToolDenied {
                tool_name, reason, ..
            } => {
                debug!("[sidecar-capture] Tool denied: {}", tool_name);
                let event = SessionEvent::feedback(
                    session_id,
                    FeedbackType::Deny,
                    Some(tool_name.clone()),
                    Some(reason.clone()),
                );
                self.sidecar.capture(event);
            }

            AiEvent::Error { message, .. } => {
                debug!("[sidecar-capture] Error: {}", message);

                // Flush any accumulated reasoning before recording the error
                self.flush_reasoning(&session_id);

                let event = SessionEvent::error(session_id, message, None);
                self.sidecar.capture(event);
            }

            AiEvent::Completed { response, .. } => {
                debug!("[sidecar-capture] Turn completed");

                // Flush any accumulated reasoning before recording the response
                self.flush_reasoning(&session_id);

                if !response.is_empty() {
                    let event = SessionEvent::ai_response(session_id, response);
                    self.sidecar.capture(event);
                }
            }

            // Events we don't capture
            AiEvent::Started { .. }
            | AiEvent::TextDelta { .. }
            | AiEvent::ContextWarning { .. }
            | AiEvent::ToolResponseTruncated { .. }
            | AiEvent::LoopWarning { .. }
            | AiEvent::LoopBlocked { .. }
            | AiEvent::MaxIterationsReached { .. }
            | AiEvent::SubAgentStarted { .. }
            | AiEvent::SubAgentToolRequest { .. }
            | AiEvent::SubAgentToolResult { .. }
            | AiEvent::UserMessage { .. }
            | AiEvent::SubAgentTextDelta { .. }
            | AiEvent::SubAgentCompleted { .. }
            | AiEvent::SubAgentError { .. }
            | AiEvent::WorkflowStarted { .. }
            | AiEvent::WorkflowStepStarted { .. }
            | AiEvent::WorkflowStepCompleted { .. }
            | AiEvent::WorkflowCompleted { .. }
            | AiEvent::WorkflowError { .. }
            | AiEvent::PlanUpdated { .. }
            | AiEvent::ServerToolStarted { .. }
            | AiEvent::WebSearchResult { .. }
            | AiEvent::WebFetchResult { .. }
            | AiEvent::Warning { .. }
            | AiEvent::CompactionStarted { .. }
            | AiEvent::CompactionCompleted { .. }
            | AiEvent::CompactionFailed { .. }
            | AiEvent::SystemHooksInjected { .. }
            | AiEvent::ToolOutputChunk { .. }
            | AiEvent::PromptGenerationStarted { .. }
            | AiEvent::PromptGenerationCompleted { .. }
            | AiEvent::AskHumanRequest { .. }
            | AiEvent::AskHumanResponse { .. }
            | AiEvent::TaskProgress { .. }
            | AiEvent::SubtaskCreated { .. }
            | AiEvent::SubtaskCompleted { .. }
            | AiEvent::SubtaskWaitingForInput { .. }
            | AiEvent::SubtaskUserInput { .. }
            | AiEvent::TaskResumed { .. }
            | AiEvent::EnricherResult { .. } => {
                // These events are not captured (ToolOutputChunk is streaming output)
            }
        }
    }

    /// Create a file event from a tool result
    fn create_file_event(
        &self,
        session_id: &str,
        tool_name: &str,
        success: bool,
    ) -> Option<SessionEvent> {
        if !success {
            return None;
        }

        let args = self.last_tool_args.as_ref()?;

        match tool_name {
            "write_file" | "create_file" => {
                let path = extract_path_from_args(args)?;
                let summary = args
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| truncate(s, 100).to_string());
                Some(SessionEvent::file_edit(
                    session_id.to_string(),
                    path,
                    FileOperation::Create,
                    summary,
                ))
            }
            "edit_file" => {
                let path = extract_path_from_args(args)?;
                let summary = args
                    .get("display_description")
                    .and_then(|v| v.as_str())
                    .map(|s| truncate(s, 100).to_string());

                // Generate diff if we have old content
                let diff = self.generate_diff(tool_name, &self.last_tool_args);

                let mut event = SessionEvent::file_edit(
                    session_id.to_string(),
                    path,
                    FileOperation::Modify,
                    summary,
                );
                event.diff = diff;
                Some(event)
            }
            "delete_file" | "delete_path" => {
                let path = extract_path_from_args(args)?;
                Some(SessionEvent::file_edit(
                    session_id.to_string(),
                    path,
                    FileOperation::Delete,
                    None,
                ))
            }
            "rename_file" | "move_file" | "move_path" => {
                let from_path = args
                    .get("source_path")
                    .or_else(|| args.get("from"))
                    .and_then(|v| v.as_str())
                    .map(PathBuf::from)?;
                let to_path = args
                    .get("destination_path")
                    .or_else(|| args.get("to"))
                    .and_then(|v| v.as_str())
                    .map(PathBuf::from)?;
                Some(SessionEvent::file_edit(
                    session_id.to_string(),
                    to_path,
                    FileOperation::Rename { from: from_path },
                    None,
                ))
            }
            _ => None,
        }
    }

    /// Generate a diff for edit operations
    fn generate_diff(&self, tool_name: &str, args: &Option<serde_json::Value>) -> Option<String> {
        if tool_name != "edit_file" {
            return None;
        }

        let args = args.as_ref()?;
        let path = extract_path_from_args(args)?;

        // Get old content from pending or read current file
        let old_content = if let Some((pending_path, content)) = &self.pending_old_content {
            if pending_path == &path {
                content.clone()
            } else {
                return None;
            }
        } else {
            return None;
        };

        // Get new content by reading the file
        let new_content = std::fs::read_to_string(&path).ok()?;

        // Generate unified diff
        let diff = generate_unified_diff(&old_content, &new_content, &path.display().to_string());

        // Truncate if too long
        Some(truncate(&diff, MAX_DIFF_LEN).to_string())
    }
}
