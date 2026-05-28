//! Inherent helpers on [`AiEvent`].
//!
//! Lives in its own file so the wire-protocol enum in [`super::event`] can
//! stay focused on the variant declarations. Right now this just holds the
//! 46-arm `event_type()` lookup, but new derived helpers (e.g. severity,
//! persistence policy, sampling weight) belong here too.

use super::event::AiEvent;

impl AiEvent {
    /// Returns the event type name as a snake_case string (matches serde serialization).
    pub fn event_type(&self) -> &'static str {
        match self {
            AiEvent::Started { .. } => "started",
            AiEvent::UserMessage { .. } => "user_message",
            AiEvent::SystemHooksInjected { .. } => "system_hooks_injected",
            AiEvent::TextDelta { .. } => "text_delta",
            AiEvent::ToolRequest { .. } => "tool_request",
            AiEvent::ToolIntentObservation { .. } => "tool_intent_observation",
            AiEvent::ToolApprovalRequest { .. } => "tool_approval_request",
            AiEvent::ToolAutoApproved { .. } => "tool_auto_approved",
            AiEvent::ToolDenied { .. } => "tool_denied",
            AiEvent::ToolResult { .. } => "tool_result",
            AiEvent::ToolOutputChunk { .. } => "tool_output_chunk",
            AiEvent::Reasoning { .. } => "reasoning",
            AiEvent::Completed { .. } => "completed",
            AiEvent::Error { .. } => "error",
            AiEvent::SubAgentStarted { .. } => "sub_agent_started",
            AiEvent::SubAgentToolRequest { .. } => "sub_agent_tool_request",
            AiEvent::SubAgentToolResult { .. } => "sub_agent_tool_result",
            AiEvent::SubAgentTextDelta { .. } => "sub_agent_text_delta",
            AiEvent::SubAgentReasoning { .. } => "sub_agent_reasoning",
            AiEvent::SubAgentCompleted { .. } => "sub_agent_completed",
            AiEvent::SubAgentError { .. } => "sub_agent_error",
            AiEvent::ContextWarning { .. } => "context_warning",
            AiEvent::ToolResponseTruncated { .. } => "tool_response_truncated",
            AiEvent::Warning { .. } => "warning",
            AiEvent::CompactionStarted { .. } => "compaction_started",
            AiEvent::CompactionCompleted { .. } => "compaction_completed",
            AiEvent::CompactionFailed { .. } => "compaction_failed",
            AiEvent::LoopWarning { .. } => "loop_warning",
            AiEvent::LoopBlocked { .. } => "loop_blocked",
            AiEvent::MaxIterationsReached { .. } => "max_iterations_reached",
            AiEvent::WorkflowStarted { .. } => "workflow_started",
            AiEvent::WorkflowStepStarted { .. } => "workflow_step_started",
            AiEvent::WorkflowStepCompleted { .. } => "workflow_step_completed",
            AiEvent::WorkflowCompleted { .. } => "workflow_completed",
            AiEvent::WorkflowError { .. } => "workflow_error",
            AiEvent::PlanUpdated { .. } => "plan_updated",
            AiEvent::ServerToolStarted { .. } => "server_tool_started",
            AiEvent::WebSearchResult { .. } => "web_search_result",
            AiEvent::WebFetchResult { .. } => "web_fetch_result",
            AiEvent::PromptGenerationStarted { .. } => "prompt_generation_started",
            AiEvent::PromptGenerationCompleted { .. } => "prompt_generation_completed",
            AiEvent::AskHumanRequest { .. } => "ask_human_request",
            AiEvent::AskHumanResponse { .. } => "ask_human_response",
            AiEvent::TaskProgress { .. } => "task_progress",
            AiEvent::SubtaskCreated { .. } => "subtask_created",
            AiEvent::SubtaskCompleted { .. } => "subtask_completed",
            AiEvent::SubtaskWaitingForInput { .. } => "subtask_waiting_for_input",
            AiEvent::SubtaskUserInput { .. } => "subtask_user_input",
            AiEvent::TaskResumed { .. } => "task_resumed",
            AiEvent::EnricherResult { .. } => "enricher_result",
        }
    }
}
