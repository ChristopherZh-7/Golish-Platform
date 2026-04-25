//! Single-turn eval entry points: [`run_eval_agentic_loop`] and the
//! tool-augmented variant [`run_eval_agentic_loop_with_tools`].

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use rig::completion::{CompletionModel as RigCompletionModel, Message, ToolDefinition};
use tokio::sync::{mpsc, oneshot, RwLock};

use crate::agent_mode::AgentMode;
use crate::agentic_loop::{AgenticLoopConfig, AgenticLoopContext};
use crate::hitl::ApprovalRecorder;
use crate::loop_detection::LoopDetector;
use crate::planner::PlanManager;
use crate::tool_definitions::ToolConfig;
use crate::tool_policy::ToolPolicyManager;
use golish_context::{CompactionState, ContextManager, ContextManagerConfig};
use golish_core::events::AiEvent;
use golish_core::hitl::ApprovalDecision;
use golish_core::ApiRequestStats;
use golish_llm_providers::{LlmClient, ModelCapabilities};
use golish_sub_agents::{SubAgentContext, SubAgentRegistry};
use golish_tools::ToolRegistry;

use super::extractors::{extract_tool_calls_and_files, print_event_verbose};
use super::types::{EvalAgentOutput, EvalConfig};


/// Run the unified agentic loop for evaluation purposes.
///
/// This function sets up minimal mock dependencies and runs the same agentic loop
/// used by the main application, ensuring evaluations test real behavior.
///
/// # Arguments
/// * `model` - The completion model to use
/// * `system_prompt` - System prompt for the agent
/// * `user_prompt` - Initial user prompt
/// * `config` - Eval configuration
///
/// # Returns
/// * `EvalAgentOutput` containing response, tool calls, files modified, etc.
pub async fn run_eval_agentic_loop<M>(
    model: &M,
    system_prompt: &str,
    user_prompt: &str,
    config: EvalConfig,
) -> Result<EvalAgentOutput>
where
    M: RigCompletionModel + Sync,
{
    let start = Instant::now();
    let verbose = config.verbose;

    // Create event channel to capture events
    let (event_tx, event_rx) = mpsc::unbounded_channel::<AiEvent>();

    // If verbose, spawn a task to print events in real-time
    let (collected_events_tx, mut collected_events_rx) = mpsc::unbounded_channel::<AiEvent>();
    let printer_handle = if verbose {
        let mut rx = event_rx;
        let tx = collected_events_tx.clone();
        Some(tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                print_event_verbose(&event);
                let _ = tx.send(event);
            }
        }))
    } else {
        // Just forward events without printing
        let mut rx = event_rx;
        let tx = collected_events_tx.clone();
        Some(tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                let _ = tx.send(event);
            }
        }))
    };

    // Create tool registry for the workspace
    let tool_registry = Arc::new(RwLock::new(
        ToolRegistry::new(config.workspace.clone()).await,
    ));

    // Create sub-agent registry with default sub-agents (coder, analyzer, explorer, researcher, executor)
    let mut registry = SubAgentRegistry::new();
    registry.register_multiple(golish_sub_agents::create_default_sub_agents());
    let sub_agent_registry = Arc::new(RwLock::new(registry));

    // Create approval recorder (uses temp dir for storage)
    let temp_dir = std::env::temp_dir().join("golish-eval");
    std::fs::create_dir_all(&temp_dir).ok();
    let approval_recorder = Arc::new(ApprovalRecorder::new(temp_dir.clone()).await);

    // Create empty pending approvals
    let pending_approvals: Arc<RwLock<HashMap<String, oneshot::Sender<ApprovalDecision>>>> =
        Arc::new(RwLock::new(HashMap::new()));

    // Create permissive tool policy manager
    let tool_policy_manager = Arc::new(ToolPolicyManager::new(&config.workspace).await);

    // Create context manager with high limits (no pruning in evals)
    let context_manager = Arc::new(ContextManager::with_config(
        &config.model_name,
        ContextManagerConfig {
            enabled: false, // Disable pruning for evals
            ..Default::default()
        },
    ));

    // Create loop detector with default config
    let loop_detector = Arc::new(RwLock::new(LoopDetector::with_defaults()));

    // Create compaction state
    let compaction_state = Arc::new(RwLock::new(CompactionState::new()));

    // Create agent mode set to auto-approve
    let agent_mode = Arc::new(RwLock::new(AgentMode::AutoApprove));

    // Create plan manager
    let plan_manager = Arc::new(PlanManager::new());

    // Create workspace Arc
    let workspace_arc = Arc::new(RwLock::new(config.workspace.clone()));

    // Create a mock LLM client (used only to check supports_native_web_tools)
    let llm_client = Arc::new(RwLock::new(LlmClient::Mock));

    // Tool config - enable all tools
    let tool_config = ToolConfig::default();

    let api_request_stats = Arc::new(ApiRequestStats::new());

    // Build the context
    let ctx = AgenticLoopContext {
        event_tx: &event_tx,
        tool_registry: &tool_registry,
        sub_agent_registry: &sub_agent_registry,
        indexer_state: None,
        workspace: &workspace_arc,
        client: &llm_client,
        approval_recorder: &approval_recorder,
        pending_approvals: &pending_approvals,
        tool_policy_manager: &tool_policy_manager,
        context_manager: &context_manager,
        compaction_state: &compaction_state,
        loop_detector: &loop_detector,
        tool_config: &tool_config,
        sidecar_state: None,
        runtime: None,
        agent_mode: &agent_mode,
        plan_manager: &plan_manager,
        provider_name: &config.provider_name,
        model_name: &config.model_name,
        api_request_stats: &api_request_stats,
        openai_web_search_config: None,
        openai_reasoning_effort: None,
        openrouter_provider_preferences: None,
        model_factory: None,
        session_id: None,
        transcript_writer: None,
        transcript_base_dir: None,
        additional_tool_definitions: vec![],
        custom_tool_executor: None,
        coordinator: None, // Evals use legacy path
        db_tracker: None,
        cancelled: None,
        execution_monitor: None,
        execution_mode: crate::execution_mode::ExecutionMode::Chat,
    };

    // Detect capabilities from provider/model
    let capabilities = ModelCapabilities::detect(&config.provider_name, &config.model_name);

    let loop_config = AgenticLoopConfig {
        capabilities,
        require_hitl: config.require_hitl,
        is_sub_agent: false,
        enable_reflector: false,
        tool_names_for_reflector: None,
    };

    // Create initial history with user prompt
    let initial_history = vec![Message::User {
        content: rig::one_or_many::OneOrMany::one(rig::message::UserContent::Text(
            rig::message::Text {
                text: user_prompt.to_string(),
            },
        )),
    }];

    // Create sub-agent context
    let sub_agent_context = SubAgentContext {
        original_request: user_prompt.to_string(),
        ..Default::default()
    };

    // Run the unified loop
    let (response, _reasoning, history, tokens) = crate::agentic_loop::run_agentic_loop_unified(
        model,
        system_prompt,
        initial_history,
        sub_agent_context,
        &ctx,
        loop_config,
    )
    .await?;

    let duration_ms = start.elapsed().as_millis() as u64;

    // Close sender and wait for printer task to finish
    drop(event_tx);
    drop(collected_events_tx);
    if let Some(handle) = printer_handle {
        let _ = handle.await;
    }

    // Collect all events from the forwarded channel
    let mut events = Vec::new();
    while let Ok(event) = collected_events_rx.try_recv() {
        events.push(event);
    }

    // Extract tool calls and file modifications from events
    let (tool_calls, files_modified) = extract_tool_calls_and_files(&events, &config.workspace);

    // Convert token usage (sum of input and output tokens)
    let tokens_used = tokens.map(|t| (t.input_tokens + t.output_tokens) as u32);

    Ok(EvalAgentOutput {
        response,
        tool_calls,
        files_modified,
        duration_ms,
        tokens_used,
        history,
        events,
    })
}

/// Type alias for a custom tool executor function.
///
/// Takes (tool_name, tool_args) and returns Some((result, success)) if handled,
/// None if not handled (falls through to standard executors).
pub type CustomToolExecutor = Arc<
    dyn Fn(
            &str,
            &serde_json::Value,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Option<(serde_json::Value, bool)>> + Send>,
        > + Send
        + Sync,
>;

/// Run the unified agentic loop with custom tools for evaluation purposes.
///
/// This variant allows injecting custom tool definitions and executors,
/// which is needed for specialized benchmarks like SWE-bench.
///
/// # Arguments
/// * `model` - The completion model to use
/// * `system_prompt` - System prompt for the agent
/// * `user_prompt` - Initial user prompt
/// * `config` - Eval configuration
/// * `additional_tools` - Additional tool definitions to include
/// * `custom_executor` - Optional executor for custom tools
///
/// # Returns
/// * `EvalAgentOutput` containing response, tool calls, files modified, etc.
pub async fn run_eval_agentic_loop_with_tools<M>(
    model: &M,
    system_prompt: &str,
    user_prompt: &str,
    config: EvalConfig,
    additional_tools: Vec<ToolDefinition>,
    custom_executor: Option<CustomToolExecutor>,
) -> Result<EvalAgentOutput>
where
    M: RigCompletionModel + Sync,
{
    let start = Instant::now();
    let verbose = config.verbose;

    // Create event channel to capture events
    let (event_tx, event_rx) = mpsc::unbounded_channel::<AiEvent>();

    // If verbose, spawn a task to print events in real-time
    let (collected_events_tx, mut collected_events_rx) = mpsc::unbounded_channel::<AiEvent>();
    let printer_handle = if verbose {
        let mut rx = event_rx;
        let tx = collected_events_tx.clone();
        Some(tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                print_event_verbose(&event);
                let _ = tx.send(event);
            }
        }))
    } else {
        // Just forward events without printing
        let mut rx = event_rx;
        let tx = collected_events_tx.clone();
        Some(tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                let _ = tx.send(event);
            }
        }))
    };

    // Create tool registry for the workspace
    let tool_registry = Arc::new(RwLock::new(
        ToolRegistry::new(config.workspace.clone()).await,
    ));

    // Create sub-agent registry with default sub-agents
    let mut registry = SubAgentRegistry::new();
    registry.register_multiple(golish_sub_agents::create_default_sub_agents());
    let sub_agent_registry = Arc::new(RwLock::new(registry));

    // Create approval recorder (uses temp dir for storage)
    let temp_dir = std::env::temp_dir().join("golish-eval-custom-tools");
    std::fs::create_dir_all(&temp_dir).ok();
    let approval_recorder = Arc::new(ApprovalRecorder::new(temp_dir.clone()).await);

    // Create empty pending approvals
    let pending_approvals: Arc<RwLock<HashMap<String, oneshot::Sender<ApprovalDecision>>>> =
        Arc::new(RwLock::new(HashMap::new()));

    // Create permissive tool policy manager
    let tool_policy_manager = Arc::new(ToolPolicyManager::new(&config.workspace).await);

    // Create context manager with high limits (no pruning in evals)
    let context_manager = Arc::new(ContextManager::with_config(
        &config.model_name,
        ContextManagerConfig {
            enabled: false, // Disable pruning for evals
            ..Default::default()
        },
    ));

    // Create loop detector with default config
    let loop_detector = Arc::new(RwLock::new(LoopDetector::with_defaults()));

    // Create compaction state
    let compaction_state = Arc::new(RwLock::new(CompactionState::new()));

    // Create agent mode set to auto-approve
    let agent_mode = Arc::new(RwLock::new(AgentMode::AutoApprove));

    // Create plan manager
    let plan_manager = Arc::new(PlanManager::new());

    // Create workspace Arc
    let workspace_arc = Arc::new(RwLock::new(config.workspace.clone()));

    // Create a mock LLM client
    let llm_client = Arc::new(RwLock::new(LlmClient::Mock));

    // Tool config - enable all tools
    let tool_config = ToolConfig::default();

    let api_request_stats = Arc::new(ApiRequestStats::new());

    // Build the context with custom tools
    let ctx = AgenticLoopContext {
        event_tx: &event_tx,
        tool_registry: &tool_registry,
        sub_agent_registry: &sub_agent_registry,
        indexer_state: None,
        workspace: &workspace_arc,
        client: &llm_client,
        approval_recorder: &approval_recorder,
        pending_approvals: &pending_approvals,
        tool_policy_manager: &tool_policy_manager,
        context_manager: &context_manager,
        compaction_state: &compaction_state,
        loop_detector: &loop_detector,
        tool_config: &tool_config,
        sidecar_state: None,
        runtime: None,
        agent_mode: &agent_mode,
        plan_manager: &plan_manager,
        provider_name: &config.provider_name,
        model_name: &config.model_name,
        api_request_stats: &api_request_stats,
        openai_web_search_config: None,
        openai_reasoning_effort: None,
        openrouter_provider_preferences: None,
        model_factory: None,
        session_id: None,
        transcript_writer: None,
        transcript_base_dir: None,
        additional_tool_definitions: additional_tools,
        custom_tool_executor: custom_executor,
        coordinator: None, // Evals use legacy path
        db_tracker: None,
        cancelled: None,
        execution_monitor: None,
        execution_mode: crate::execution_mode::ExecutionMode::Chat,
    };

    // Detect capabilities from provider/model
    let capabilities = ModelCapabilities::detect(&config.provider_name, &config.model_name);

    let loop_config = AgenticLoopConfig {
        capabilities,
        require_hitl: config.require_hitl,
        is_sub_agent: false,
        enable_reflector: false,
        tool_names_for_reflector: None,
    };

    // Create initial history with user prompt
    let initial_history = vec![Message::User {
        content: rig::one_or_many::OneOrMany::one(rig::message::UserContent::Text(
            rig::message::Text {
                text: user_prompt.to_string(),
            },
        )),
    }];

    // Create sub-agent context
    let sub_agent_context = SubAgentContext {
        original_request: user_prompt.to_string(),
        ..Default::default()
    };

    // Run the unified loop
    let (response, _reasoning, history, tokens) = crate::agentic_loop::run_agentic_loop_unified(
        model,
        system_prompt,
        initial_history,
        sub_agent_context,
        &ctx,
        loop_config,
    )
    .await?;

    let duration_ms = start.elapsed().as_millis() as u64;

    // Close sender and wait for printer task to finish
    drop(event_tx);
    drop(collected_events_tx);
    if let Some(handle) = printer_handle {
        let _ = handle.await;
    }

    // Collect all events from the forwarded channel
    let mut events = Vec::new();
    while let Ok(event) = collected_events_rx.try_recv() {
        events.push(event);
    }

    // Extract tool calls and file modifications from events
    let (tool_calls, files_modified) = extract_tool_calls_and_files(&events, &config.workspace);

    // Convert token usage
    let tokens_used = tokens.map(|t| (t.input_tokens + t.output_tokens) as u32);

    Ok(EvalAgentOutput {
        response,
        tool_calls,
        files_modified,
        duration_ms,
        tokens_used,
        history,
        events,
    })
}

