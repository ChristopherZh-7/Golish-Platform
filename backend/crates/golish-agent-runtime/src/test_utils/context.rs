use super::*;

use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::{mpsc, oneshot, RwLock};

use golish_agent_kit::hitl::ApprovalRecorder;
use golish_agent_kit::loop_detection::LoopDetector;
use golish_agent_kit::planner::PlanManager;
use golish_agent_kit::tool_policy::{ToolPolicy, ToolPolicyConfig, ToolPolicyManager};
use golish_context::context_manager::ContextTrimConfig;
use golish_context::token_budget::TokenBudgetConfig;
use golish_context::{CompactionState, ContextManager};
use golish_core::events::AiEvent;
use golish_core::hitl::ApprovalDecision;
use golish_core::ApiRequestStats;
use golish_llm_providers::LlmClient;
use golish_sub_agents::SubAgentRegistry;
use golish_tools::ToolRegistry;

use golish_agent_kit::agent_mode::AgentMode;
use crate::agentic_loop::{
    AgenticLoopContext, LoopAccessControl, LoopCaptureContext, LoopEventRefs, LoopLlmRefs,
};
use golish_agent_kit::tool_definitions::ToolConfig;

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
            llm: LoopLlmRefs {
                client,
                provider_name: "mock",
                model_name: "mock-model",
                openai_web_search_config: None,
                openai_reasoning_effort: None,
                openrouter_provider_preferences: None,
                model_factory: None,
            },
            access: LoopAccessControl {
                approval_recorder: &self.approval_recorder,
                pending_approvals: &self.pending_approvals,
                tool_policy_manager: &self.tool_policy_manager,
                agent_mode: &self.agent_mode,
                loop_detector: &self.loop_detector,
                coordinator: None,
            },
            events: LoopEventRefs {
                event_tx: &self.event_tx,
                transcript_writer: None,
                transcript_base_dir: None,
                session_id: None,
                db_tracker: None,
                runtime: self.runtime.as_ref(),
            },
            tool_registry: &self.tool_registry,
            sub_agent_registry: &self.sub_agent_registry,
            indexer_state: None,
            workspace: &self.workspace,
            context_manager: &self.context_manager,
            compaction_state: &self.compaction_state,
            tool_config: &self.tool_config,
            graph_backend: None,
            sidecar_state: None,
            chain_persistence: None,
            plan_manager: &self.plan_manager,
            api_request_stats: &self.api_request_stats,
            additional_tool_definitions: vec![],
            custom_tool_executor: None,
            cancelled: None,
            execution_monitor: None,
            execution_mode: golish_agent_kit::execution_mode::ExecutionMode::Chat,
            post_shell_hook: None,
            output_classifier: None,
            web_fetcher: None,
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
