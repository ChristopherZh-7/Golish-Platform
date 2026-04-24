//! Test utilities for the AI agent system.
//!
//! This module provides mock implementations and helpers for testing the
//! agentic loop, HITL approval flows, and tool routing logic.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use futures::stream::{self, BoxStream};
use futures::StreamExt;
use rig::completion::{
    self, AssistantContent, CompletionError, CompletionRequest, CompletionResponse, GetTokenUsage,
    Usage,
};
use rig::message::{Reasoning, ReasoningContent, Text, ToolCall, ToolFunction};
use rig::one_or_many::OneOrMany;
use rig::streaming::{RawStreamingChoice, RawStreamingToolCall, StreamingCompletionResponse};
use serde::{Deserialize, Serialize};

/// A mock response that the MockCompletionModel will return.
#[derive(Debug, Clone)]
pub struct MockResponse {
    /// Text content to return (if any)
    pub text: Option<String>,
    /// Tool calls to return (if any)
    pub tool_calls: Vec<MockToolCall>,
    /// Thinking/reasoning content to return (if any)
    pub thinking: Option<String>,
}

impl Default for MockResponse {
    fn default() -> Self {
        Self {
            text: Some("Mock response".to_string()),
            tool_calls: vec![],
            thinking: None,
        }
    }
}

impl MockResponse {
    /// Create a text-only response.
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            text: Some(content.into()),
            tool_calls: vec![],
            thinking: None,
        }
    }

    /// Create a response with a tool call.
    pub fn tool_call(name: impl Into<String>, args: serde_json::Value) -> Self {
        Self {
            text: None,
            tool_calls: vec![MockToolCall {
                name: name.into(),
                args,
            }],
            thinking: None,
        }
    }

    /// Create a response with multiple tool calls.
    pub fn tool_calls(calls: Vec<MockToolCall>) -> Self {
        Self {
            text: None,
            tool_calls: calls,
            thinking: None,
        }
    }

    /// Create a response with thinking content.
    pub fn with_thinking(mut self, thinking: impl Into<String>) -> Self {
        self.thinking = Some(thinking.into());
        self
    }

    /// Create a response with text and thinking.
    pub fn text_with_thinking(text: impl Into<String>, thinking: impl Into<String>) -> Self {
        Self {
            text: Some(text.into()),
            tool_calls: vec![],
            thinking: Some(thinking.into()),
        }
    }
}

/// A mock tool call.
#[derive(Debug, Clone)]
pub struct MockToolCall {
    pub name: String,
    pub args: serde_json::Value,
}

impl MockToolCall {
    pub fn new(name: impl Into<String>, args: serde_json::Value) -> Self {
        Self {
            name: name.into(),
            args,
        }
    }
}

/// Streaming response data for the mock model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockStreamingResponseData {
    pub text: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl Default for MockStreamingResponseData {
    fn default() -> Self {
        Self {
            text: String::new(),
            input_tokens: 100,
            output_tokens: 50,
        }
    }
}

impl GetTokenUsage for MockStreamingResponseData {
    fn token_usage(&self) -> Option<Usage> {
        Some(Usage {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            total_tokens: self.input_tokens + self.output_tokens,
            cached_input_tokens: 0,
        })
    }
}

/// A mock CompletionModel for testing agentic loop behavior.
///
/// This model returns predefined responses in sequence, allowing
/// multi-turn testing of the agentic loop.
#[derive(Debug, Clone)]
pub struct MockCompletionModel {
    responses: Arc<Vec<MockResponse>>,
    current_index: Arc<AtomicUsize>,
}

impl MockCompletionModel {
    /// Create a new mock model with a sequence of responses.
    pub fn new(responses: Vec<MockResponse>) -> Self {
        Self {
            responses: Arc::new(responses),
            current_index: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Create a mock model that returns a single text response.
    pub fn with_text(text: impl Into<String>) -> Self {
        Self::new(vec![MockResponse::text(text)])
    }

    /// Create a mock model that returns a single tool call then text.
    pub fn with_tool_call_then_text(
        tool_name: impl Into<String>,
        tool_args: serde_json::Value,
        final_text: impl Into<String>,
    ) -> Self {
        Self::new(vec![
            MockResponse::tool_call(tool_name, tool_args),
            MockResponse::text(final_text),
        ])
    }

    /// Get the next response in the sequence.
    fn next_response(&self) -> MockResponse {
        let index = self.current_index.fetch_add(1, Ordering::SeqCst);
        if index < self.responses.len() {
            self.responses[index].clone()
        } else {
            // Return empty text response if we've exhausted all responses
            MockResponse::text("")
        }
    }

    /// Reset the response index to start from the beginning.
    pub fn reset(&self) {
        self.current_index.store(0, Ordering::SeqCst);
    }

    /// Get the number of times a response has been requested.
    pub fn call_count(&self) -> usize {
        self.current_index.load(Ordering::SeqCst)
    }

    /// Build a CompletionResponse from a MockResponse.
    fn build_completion_response(
        &self,
        mock_response: &MockResponse,
        call_count: usize,
    ) -> CompletionResponse<MockStreamingResponseData> {
        let mut content: Vec<AssistantContent> = vec![];

        // Add thinking content first (if any)
        if let Some(thinking) = &mock_response.thinking {
            content.push(AssistantContent::Reasoning(
                Reasoning::new(thinking).optional_id(Some(format!("mock-thinking-{}", call_count))),
            ));
        }

        // Add text content (if any)
        if let Some(text) = &mock_response.text {
            content.push(AssistantContent::Text(Text { text: text.clone() }));
        }

        // Add tool calls (if any)
        for (i, tool_call) in mock_response.tool_calls.iter().enumerate() {
            let id = format!("mock-tool-{}-{}", call_count, i);
            content.push(AssistantContent::ToolCall(ToolCall {
                id: id.clone(),
                call_id: Some(id),
                function: ToolFunction {
                    name: tool_call.name.clone(),
                    arguments: tool_call.args.clone(),
                },
                signature: None,
                additional_params: None,
            }));
        }

        let choice = if content.len() == 1 {
            OneOrMany::one(content.pop().unwrap())
        } else if content.is_empty() {
            OneOrMany::one(AssistantContent::Text(Text {
                text: String::new(),
            }))
        } else {
            OneOrMany::many(content).unwrap()
        };

        CompletionResponse {
            choice,
            usage: Usage {
                input_tokens: 100,
                output_tokens: 50,
                total_tokens: 150,
                cached_input_tokens: 0,
            },
            raw_response: MockStreamingResponseData::default(),
            message_id: None,
        }
    }

    /// Build streaming chunks from a MockResponse.
    fn build_stream_chunks(
        mock_response: &MockResponse,
        call_count: usize,
    ) -> Vec<RawStreamingChoice<MockStreamingResponseData>> {
        let mut chunks: Vec<RawStreamingChoice<MockStreamingResponseData>> = vec![];

        // Add thinking content first (if any)
        if let Some(thinking) = &mock_response.thinking {
            chunks.push(RawStreamingChoice::Reasoning {
                id: Some(format!("mock-thinking-{}", call_count)),
                content: ReasoningContent::Text {
                    text: thinking.clone(),
                    signature: Some("mock-signature".to_string()),
                },
            });
        }

        // Add text content (if any)
        if let Some(text) = &mock_response.text {
            chunks.push(RawStreamingChoice::Message(text.clone()));
        }

        // Add tool calls (if any)
        for (i, tool_call) in mock_response.tool_calls.iter().enumerate() {
            let id = format!("mock-tool-{}-{}", call_count, i);
            chunks.push(RawStreamingChoice::ToolCall(RawStreamingToolCall {
                id: id.clone(),
                internal_call_id: id.clone(),
                call_id: Some(id),
                name: tool_call.name.clone(),
                arguments: tool_call.args.clone(),
                signature: None,
                additional_params: None,
            }));
        }

        // Add final response
        chunks.push(RawStreamingChoice::FinalResponse(
            MockStreamingResponseData {
                text: mock_response.text.clone().unwrap_or_default(),
                input_tokens: 100,
                output_tokens: 50,
            },
        ));

        chunks
    }
}

impl completion::CompletionModel for MockCompletionModel {
    type Response = MockStreamingResponseData;
    type StreamingResponse = MockStreamingResponseData;
    type Client = ();

    fn make(_client: &Self::Client, _model: impl Into<String>) -> Self {
        Self::new(vec![MockResponse::default()])
    }

    async fn completion(
        &self,
        _request: CompletionRequest,
    ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
        let mock_response = self.next_response();
        let call_count = self.call_count();
        Ok(self.build_completion_response(&mock_response, call_count))
    }

    async fn stream(
        &self,
        _request: CompletionRequest,
    ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
        let mock_response = self.next_response();
        let call_count = self.call_count();
        let chunks = Self::build_stream_chunks(&mock_response, call_count);

        // Convert to stream of RawStreamingChoice
        let stream: BoxStream<
            'static,
            Result<RawStreamingChoice<MockStreamingResponseData>, CompletionError>,
        > = stream::iter(chunks.into_iter().map(Ok)).boxed();

        Ok(StreamingCompletionResponse::stream(Box::pin(stream)))
    }
}

// ============================================================================
// Test Context Infrastructure
// ============================================================================

use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::{mpsc, oneshot, RwLock};

use crate::hitl::ApprovalRecorder;
use crate::loop_detection::LoopDetector;
use crate::planner::PlanManager;
use crate::tool_policy::{ToolPolicy, ToolPolicyConfig, ToolPolicyManager};
use golish_context::context_manager::ContextTrimConfig;
use golish_context::token_budget::TokenBudgetConfig;
use golish_context::{CompactionState, ContextManager};
use golish_core::events::AiEvent;
use golish_core::hitl::ApprovalDecision;
use golish_core::ApiRequestStats;
use golish_llm_providers::LlmClient;
use golish_sub_agents::SubAgentRegistry;
use golish_tools::ToolRegistry;

use crate::agent_mode::AgentMode;
use crate::agentic_loop::{AgenticLoopContext, LoopCaptureContext};
use crate::tool_definitions::ToolConfig;

// ============================================================================
// Mock Runtime for Testing
// ============================================================================

use async_trait::async_trait;
use golish_core::runtime::{ApprovalResult, GolishRuntime, RuntimeError, RuntimeEvent};
use std::any::Any;

/// A mock runtime for testing HITL approval flows.
#[derive(Debug)]
pub struct MockRuntime {
    auto_approve: bool,
    interactive: bool,
}

impl MockRuntime {
    /// Create a new mock runtime.
    pub fn new() -> Self {
        Self {
            auto_approve: false,
            interactive: true,
        }
    }

    /// Create a mock runtime with auto-approve enabled.
    pub fn with_auto_approve() -> Self {
        Self {
            auto_approve: true,
            interactive: true,
        }
    }

    /// Set whether auto-approve is enabled.
    pub fn set_auto_approve(&mut self, auto_approve: bool) {
        self.auto_approve = auto_approve;
    }
}

impl Default for MockRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl GolishRuntime for MockRuntime {
    fn emit(&self, _event: RuntimeEvent) -> Result<(), RuntimeError> {
        Ok(())
    }

    async fn request_approval(
        &self,
        _request_id: String,
        _tool_name: String,
        _args: serde_json::Value,
        _risk_level: String,
    ) -> Result<ApprovalResult, RuntimeError> {
        // In tests, we control approval via other mechanisms
        // Timeout of 0 indicates immediate timeout for testing
        Err(RuntimeError::ApprovalTimeout(0))
    }

    fn is_interactive(&self) -> bool {
        self.interactive
    }

    fn auto_approve(&self) -> bool {
        self.auto_approve
    }

    async fn shutdown(&self) -> Result<(), RuntimeError> {
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Builder for creating test contexts for the agentic loop.
pub struct TestContextBuilder {
    workspace: PathBuf,
    agent_mode: AgentMode,
    runtime: Option<Arc<dyn GolishRuntime>>,
    denied_tools: Vec<String>,
    allowed_tools: Vec<String>,
}

impl Default for TestContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TestContextBuilder {
    /// Create a new test context builder with default settings.
    pub fn new() -> Self {
        Self {
            workspace: PathBuf::from("/tmp/golish-test"),
            agent_mode: AgentMode::default(),
            runtime: None,
            denied_tools: vec![],
            allowed_tools: vec![],
        }
    }

    /// Set the workspace path.
    pub fn workspace(mut self, path: impl Into<PathBuf>) -> Self {
        self.workspace = path.into();
        self
    }

    /// Set the agent mode.
    pub fn agent_mode(mut self, mode: AgentMode) -> Self {
        self.agent_mode = mode;
        self
    }

    /// Set a runtime for testing.
    pub fn runtime(mut self, runtime: Arc<dyn GolishRuntime>) -> Self {
        self.runtime = Some(runtime);
        self
    }

    /// Add a tool that should be denied by policy.
    pub fn deny_tool(mut self, tool_name: impl Into<String>) -> Self {
        self.denied_tools.push(tool_name.into());
        self
    }

    /// Add a tool that should be allowed by policy (bypasses HITL).
    pub fn allow_tool(mut self, tool_name: impl Into<String>) -> Self {
        self.allowed_tools.push(tool_name.into());
        self
    }

    /// Build the test context with all required dependencies.
    pub async fn build(self) -> TestContext {
        // Create temp directory for test data
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let storage_dir = temp_dir.path().to_path_buf();

        // Use the temp dir as the workspace (unless explicitly set)
        let workspace_path = if self.workspace.as_path() == std::path::Path::new("/tmp/golish-test") {
            temp_dir.path().to_path_buf()
        } else {
            self.workspace.clone()
        };

        // Create all required components
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let tool_registry = Arc::new(RwLock::new(ToolRegistry::new(workspace_path.clone()).await));
        let sub_agent_registry = Arc::new(RwLock::new(SubAgentRegistry::new()));
        let approval_recorder = Arc::new(ApprovalRecorder::new(storage_dir.clone()).await);
        let pending_approvals = Arc::new(RwLock::new(HashMap::new()));

        // Create tool policy config with custom policies
        let mut policy_config = ToolPolicyConfig::default();
        for tool in &self.denied_tools {
            policy_config
                .policies
                .insert(tool.clone(), ToolPolicy::Deny);
        }
        for tool in &self.allowed_tools {
            policy_config
                .policies
                .insert(tool.clone(), ToolPolicy::Allow);
        }
        let tool_policy_manager = Arc::new(ToolPolicyManager::with_config(
            policy_config,
            workspace_path.join(".golish").join("tool-policy.json"),
        ));

        let context_manager = Arc::new(ContextManager::new(
            TokenBudgetConfig::default(),
            ContextTrimConfig::default(),
        ));
        let compaction_state = Arc::new(RwLock::new(CompactionState::new()));
        let loop_detector = Arc::new(RwLock::new(LoopDetector::with_defaults()));
        let workspace = Arc::new(RwLock::new(workspace_path));
        let agent_mode = Arc::new(RwLock::new(self.agent_mode));
        let plan_manager = Arc::new(PlanManager::new());
        let tool_config = ToolConfig::default();

        TestContext {
            event_tx,
            event_rx,
            tool_registry,
            sub_agent_registry,
            approval_recorder,
            pending_approvals,
            tool_policy_manager,
            context_manager,
            compaction_state,
            loop_detector,
            workspace,
            agent_mode,
            plan_manager,
            tool_config,
            api_request_stats: Arc::new(ApiRequestStats::new()),
            runtime: self.runtime,
            _temp_dir: temp_dir,
        }
    }
}

/// Test context holding all dependencies needed for agentic loop tests.
pub struct TestContext {
    pub event_tx: mpsc::UnboundedSender<AiEvent>,
    pub event_rx: mpsc::UnboundedReceiver<AiEvent>,
    pub tool_registry: Arc<RwLock<ToolRegistry>>,
    pub sub_agent_registry: Arc<RwLock<SubAgentRegistry>>,
    pub approval_recorder: Arc<ApprovalRecorder>,
    pub pending_approvals: Arc<RwLock<HashMap<String, oneshot::Sender<ApprovalDecision>>>>,
    pub tool_policy_manager: Arc<ToolPolicyManager>,
    pub context_manager: Arc<ContextManager>,
    pub compaction_state: Arc<RwLock<CompactionState>>,
    pub loop_detector: Arc<RwLock<LoopDetector>>,
    pub workspace: Arc<RwLock<PathBuf>>,
    pub agent_mode: Arc<RwLock<AgentMode>>,
    pub plan_manager: Arc<PlanManager>,
    pub tool_config: ToolConfig,
    pub api_request_stats: Arc<ApiRequestStats>,
    /// Optional runtime for testing auto-approve flag
    pub runtime: Option<Arc<dyn GolishRuntime>>,
    // Keep temp dir alive for the duration of the test
    _temp_dir: tempfile::TempDir,
}

impl TestContext {
    /// Create an AgenticLoopContext from this test context.
    ///
    /// Note: The `client` field in AgenticLoopContext is required but we need
    /// to provide one externally since LlmClient is an enum without a default variant.
    pub fn as_agentic_context_with_client<'a>(
        &'a self,
        client: &'a Arc<RwLock<LlmClient>>,
    ) -> AgenticLoopContext<'a> {
        AgenticLoopContext {
            event_tx: &self.event_tx,
            tool_registry: &self.tool_registry,
            sub_agent_registry: &self.sub_agent_registry,
            indexer_state: None,
            workspace: &self.workspace,
            client,
            approval_recorder: &self.approval_recorder,
            pending_approvals: &self.pending_approvals,
            tool_policy_manager: &self.tool_policy_manager,
            context_manager: &self.context_manager,
            compaction_state: &self.compaction_state,
            loop_detector: &self.loop_detector,
            tool_config: &self.tool_config,
            sidecar_state: None,
            runtime: self.runtime.as_ref(),
            agent_mode: &self.agent_mode,
            plan_manager: &self.plan_manager,
            provider_name: "mock",
            model_name: "mock-model",
            api_request_stats: &self.api_request_stats,
            openai_web_search_config: None,
            openai_reasoning_effort: None,
            openrouter_provider_preferences: None,
            model_factory: None,
            session_id: None,
            transcript_writer: None,
            transcript_base_dir: None,
            additional_tool_definitions: vec![],
            custom_tool_executor: None,
            coordinator: None, // Tests use legacy path
            db_tracker: None,
            cancelled: None,
            execution_monitor: None,
            execution_mode: crate::execution_mode::ExecutionMode::Chat,
        }
    }

    /// Collect all events that have been emitted.
    pub fn collect_events(&mut self) -> Vec<AiEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }
        events
    }

    /// Create a LoopCaptureContext for testing.
    pub fn create_capture_context(&self) -> LoopCaptureContext {
        LoopCaptureContext::new(None)
    }

    /// Get workspace path.
    pub async fn workspace_path(&self) -> PathBuf {
        self.workspace.read().await.clone()
    }

    /// Find events of a specific type.
    pub fn find_events<F>(&mut self, predicate: F) -> Vec<AiEvent>
    where
        F: Fn(&AiEvent) -> bool,
    {
        self.collect_events()
            .into_iter()
            .filter(predicate)
            .collect()
    }

    /// Check if any event matches the predicate.
    pub fn has_event<F>(&mut self, predicate: F) -> bool
    where
        F: Fn(&AiEvent) -> bool,
    {
        self.collect_events().iter().any(predicate)
    }

    /// Add a tool to the always-approve list in the approval recorder.
    pub async fn always_approve_tool(&self, tool_name: &str) {
        let _ = self.approval_recorder.add_always_allow(tool_name).await;
    }

    /// Record a manual approval for a tool (to test learned patterns).
    pub async fn record_tool_approval(&self, tool_name: &str, approved: bool) {
        let _ = self
            .approval_recorder
            .record_approval(tool_name, approved, None, false)
            .await;
    }
}

#[cfg(test)]
#[path = "test_utils_tests.rs"]
mod test_utils_tests;
