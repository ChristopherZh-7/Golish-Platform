//! [`AiEvent`] — the core event enum streamed from `AgentBridge` to the frontend.
//!
//! This is intentionally a single (large) enum: it serves as the wire contract
//! between backend and frontend, and adding a new variant here is the canonical
//! way to expose a new agent-side event. Variants are organized into logical
//! sections by comments (lifecycle, tool I/O, sub-agents, context, workflow,
//! plan, server tools, prompt-gen, task-mode).

use serde::{Deserialize, Serialize};

use super::tool_source::ToolSource;
use crate::hitl::{ApprovalPattern, RiskLevel};

/// Simplified AI events for the frontend.
/// We emit these directly from AgentBridge instead of converting from vtcode's ThreadEvent,
/// since ThreadEvent uses tuple structs that are harder to work with.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AiEvent {
    /// Agent started processing a turn
    Started { turn_id: String },

    /// User message that initiated this turn
    UserMessage { content: String },

    /// System hooks were injected into the conversation (e.g., post-tool hooks)
    SystemHooksInjected { hooks: Vec<String> },

    /// Streaming text chunk from the LLM
    TextDelta { delta: String, accumulated: String },

    /// Tool execution requested (for approval UI / HITL)
    /// This is the legacy event - kept for backward compatibility
    ToolRequest {
        tool_name: String,
        args: serde_json::Value,
        request_id: String,
        /// Source of this tool call (main agent, sub-agent, or workflow)
        #[serde(default)]
        source: ToolSource,
    },

    /// Tool approval request with HITL metadata
    /// The frontend should show an approval dialog and respond with ToolApprovalResponse
    ToolApprovalRequest {
        request_id: String,
        tool_name: String,
        args: serde_json::Value,
        /// Current approval stats for this tool (if any)
        stats: Option<ApprovalPattern>,
        /// Risk level of this operation
        risk_level: RiskLevel,
        /// Whether this tool can be auto-approved in the future
        can_learn: bool,
        /// Suggestion message (e.g., "2 more approvals needed for auto-approve")
        suggestion: Option<String>,
        /// Source of this tool call (main agent, sub-agent, or workflow)
        #[serde(default)]
        source: ToolSource,
    },

    /// Tool was auto-approved based on learned patterns
    ToolAutoApproved {
        request_id: String,
        tool_name: String,
        args: serde_json::Value,
        /// Reason for auto-approval
        reason: String,
        /// Source of this tool call (main agent, sub-agent, or workflow)
        #[serde(default)]
        source: ToolSource,
    },

    /// Tool was denied by policy or constraint
    ToolDenied {
        request_id: String,
        tool_name: String,
        args: serde_json::Value,
        /// Reason for denial
        reason: String,
        /// Source of this tool call (main agent, sub-agent, or workflow)
        #[serde(default)]
        source: ToolSource,
    },

    /// Tool execution completed
    ToolResult {
        tool_name: String,
        result: serde_json::Value,
        success: bool,
        request_id: String,
        /// Source of this tool call (main agent, sub-agent, or workflow)
        #[serde(default)]
        source: ToolSource,
    },

    /// Streaming output chunk from a tool (e.g., run_command stdout/stderr)
    ToolOutputChunk {
        request_id: String,
        tool_name: String,
        /// Raw output chunk (may contain ANSI codes)
        chunk: String,
        /// Which stream this came from: "stdout" or "stderr"
        stream: String,
        /// Source of this tool call (main agent, sub-agent, or workflow)
        #[serde(default)]
        source: ToolSource,
    },

    /// Agent reasoning/thinking (for models that support extended thinking)
    Reasoning { content: String },

    /// Turn completed successfully
    Completed {
        response: String,
        /// Accumulated reasoning/thinking content (for models with extended thinking)
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning: Option<String>,
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
        duration_ms: Option<u64>,
    },

    /// Error occurred during processing
    Error { message: String, error_type: String },

    // Human-in-the-loop interaction events
    /// AI agent is requesting input from the user (barrier tool — pauses execution)
    AskHumanRequest {
        request_id: String,
        /// The question or information the AI needs
        question: String,
        /// Type of input expected: "credentials", "choice", "freetext", "confirmation"
        input_type: String,
        /// Options for "choice" type (empty for other types)
        #[serde(default)]
        options: Vec<String>,
        /// Additional context about why this is needed
        #[serde(default)]
        context: String,
    },

    /// User responded to an ask_human request
    AskHumanResponse {
        request_id: String,
        /// The user's text response
        response: String,
        /// Whether the user skipped this request
        skipped: bool,
    },

    // Sub-agent events
    /// Sub-agent started executing a task
    SubAgentStarted {
        agent_id: String,
        agent_name: String,
        task: String,
        depth: usize,
        parent_request_id: String,
    },

    /// Sub-agent tool request (for visibility into sub-agent's tool usage)
    SubAgentToolRequest {
        agent_id: String,
        tool_name: String,
        args: serde_json::Value,
        request_id: String,
        parent_request_id: String,
    },

    /// Sub-agent tool result
    SubAgentToolResult {
        agent_id: String,
        tool_name: String,
        success: bool,
        result: serde_json::Value,
        request_id: String,
        parent_request_id: String,
    },

    /// Sub-agent streaming text delta (thinking/reasoning/output)
    SubAgentTextDelta {
        agent_id: String,
        delta: String,
        accumulated: String,
        parent_request_id: String,
    },

    /// Sub-agent completed its task
    SubAgentCompleted {
        agent_id: String,
        response: String,
        duration_ms: u64,
        parent_request_id: String,
    },

    /// Sub-agent encountered an error
    SubAgentError {
        agent_id: String,
        error: String,
        parent_request_id: String,
    },

    // Context management events
    /// Context warning threshold exceeded
    ContextWarning {
        utilization: f64,
        total_tokens: usize,
        max_tokens: usize,
    },

    /// Tool response was truncated due to size limits
    ToolResponseTruncated {
        tool_name: String,
        original_tokens: usize,
        truncated_tokens: usize,
    },

    /// Generic warning message (e.g., images stripped for non-vision model)
    Warning { message: String },

    // Context compaction events
    /// Context compaction started
    CompactionStarted {
        /// Number of tokens before compaction
        tokens_before: u64,
        /// Number of messages before compaction
        messages_before: usize,
    },

    /// Context compaction completed successfully
    CompactionCompleted {
        /// Number of tokens before compaction
        tokens_before: u64,
        /// Number of messages before compaction
        messages_before: usize,
        /// Number of messages after compaction
        messages_after: usize,
        /// Length of the generated summary
        summary_length: usize,
        /// The generated summary text
        summary: Option<String>,
        /// The summarizer input that was used
        summarizer_input: Option<String>,
    },

    /// Context compaction failed
    CompactionFailed {
        /// Number of tokens before compaction
        tokens_before: u64,
        /// Number of messages before compaction
        messages_before: usize,
        /// Error message
        error: String,
        /// The summarizer input that was used
        summarizer_input: Option<String>,
    },

    // Loop protection events
    /// Warning: approaching loop detection threshold
    LoopWarning {
        tool_name: String,
        current_count: usize,
        max_count: usize,
        message: String,
    },

    /// Tool call blocked due to loop detection
    LoopBlocked {
        tool_name: String,
        repeat_count: usize,
        max_count: usize,
        message: String,
    },

    /// Maximum tool iterations reached for this turn
    MaxIterationsReached {
        iterations: usize,
        max_iterations: usize,
        message: String,
    },

    // Workflow events
    /// Workflow started
    WorkflowStarted {
        workflow_id: String,
        workflow_name: String,
        session_id: String,
    },

    /// Workflow step started
    WorkflowStepStarted {
        workflow_id: String,
        step_name: String,
        step_index: usize,
        total_steps: usize,
    },

    /// Workflow step completed
    WorkflowStepCompleted {
        workflow_id: String,
        step_name: String,
        output: Option<String>,
        duration_ms: u64,
    },

    /// Workflow completed
    WorkflowCompleted {
        workflow_id: String,
        final_output: String,
        total_duration_ms: u64,
    },

    /// Workflow error
    WorkflowError {
        workflow_id: String,
        step_name: Option<String>,
        error: String,
    },

    // Plan management events
    /// Task plan updated
    PlanUpdated {
        /// Plan version (increments with each update)
        version: u32,
        /// Summary statistics
        summary: crate::plan::PlanSummary,
        /// The updated steps
        steps: Vec<crate::plan::PlanStep>,
        /// Optional explanation
        explanation: Option<String>,
    },

    // Server tool events (Claude's native web_search/web_fetch)
    // These are server-side tools executed by Anthropic's API, not by the client
    /// Server tool (web_search/web_fetch) started by Claude
    /// Unlike regular tools, these don't require HITL approval
    ServerToolStarted {
        /// Unique identifier for this tool use
        request_id: String,
        /// Tool name (web_search or web_fetch)
        tool_name: String,
        /// Tool input parameters
        input: serde_json::Value,
    },

    /// Web search results received from Claude's native web search
    WebSearchResult {
        /// Tool use ID that this result corresponds to
        request_id: String,
        /// Search results (array of {url, title, content, page_age})
        results: serde_json::Value,
    },

    /// Web fetch result received from Claude's native web fetch
    WebFetchResult {
        /// Tool use ID that this result corresponds to
        request_id: String,
        /// URL that was fetched
        url: String,
        /// Preview of fetched content (truncated for events)
        content_preview: String,
    },

    // Prompt generation events (sub-agent system prompt generation via architect LLM)
    /// Sub-agent system prompt generation started
    PromptGenerationStarted {
        /// The sub-agent this prompt is being generated for
        agent_id: String,
        /// The parent request that triggered this sub-agent
        parent_request_id: String,
        /// The system prompt sent to the architect LLM (the meta-prompt template)
        architect_system_prompt: String,
        /// The user message sent to the architect LLM (task + context)
        architect_user_message: String,
    },

    /// Sub-agent system prompt generation completed
    PromptGenerationCompleted {
        /// The sub-agent this prompt is being generated for
        agent_id: String,
        /// The parent request that triggered this sub-agent
        parent_request_id: String,
        /// The generated system prompt (None if generation failed)
        generated_prompt: Option<String>,
        /// Whether generation succeeded
        success: bool,
        /// Duration of the generation call in milliseconds
        duration_ms: u64,
    },

    // Task mode events (PentAGI-style automated execution)
    /// Task progress update (status changes, phase transitions)
    TaskProgress {
        task_id: String,
        status: String,
        message: String,
    },

    /// A new subtask was created by the Generator or Refiner
    SubtaskCreated {
        task_id: String,
        subtask_id: String,
        title: String,
        agent: Option<String>,
    },

    /// A subtask finished executing
    SubtaskCompleted {
        task_id: String,
        subtask_id: String,
        title: String,
        result: String,
    },

    /// A subtask is waiting for user input before continuing
    SubtaskWaitingForInput {
        task_id: String,
        subtask_id: String,
        title: String,
        prompt: String,
    },

    /// User provided input for a waiting subtask
    SubtaskUserInput {
        task_id: String,
        subtask_id: String,
        input: String,
    },

    /// A previously interrupted task is being resumed
    TaskResumed {
        task_id: String,
        subtask_index: usize,
        total_subtasks: usize,
    },

    /// Enricher gathered additional context after a subtask
    EnricherResult {
        task_id: String,
        subtask_id: String,
        context_added: String,
    },
}

impl AiEvent {
    /// Returns the event type name as a snake_case string (matches serde serialization)
    pub fn event_type(&self) -> &'static str {
        match self {
            AiEvent::Started { .. } => "started",
            AiEvent::UserMessage { .. } => "user_message",
            AiEvent::SystemHooksInjected { .. } => "system_hooks_injected",
            AiEvent::TextDelta { .. } => "text_delta",
            AiEvent::ToolRequest { .. } => "tool_request",
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
