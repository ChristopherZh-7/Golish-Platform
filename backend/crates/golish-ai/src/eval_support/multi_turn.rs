//! Multi-turn eval support: a runner that drives the same agentic loop across
//! a sequence of user prompts, threading the conversation history between
//! turns.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use rig::completion::{CompletionModel as RigCompletionModel, Message};
use tokio::sync::{mpsc, oneshot, RwLock};

use crate::agent_mode::AgentMode;
use crate::agentic_loop::{
    AgenticLoopConfig, AgenticLoopContext, LoopAccessControl, LoopEventRefs, LoopLlmRefs,
};
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

use super::extractors::extract_tool_calls_and_files;
use super::types::{EvalAgentOutput, EvalConfig};

/// Output from a multi-turn eval agentic loop run.
#[derive(Debug, Clone)]
pub struct MultiTurnEvalOutput {
    /// Outputs from each turn in order.
    pub turns: Vec<EvalAgentOutput>,
    /// Total duration of all turns in milliseconds.
    pub total_duration_ms: u64,
    /// Final message history after all turns.
    pub final_history: Vec<Message>,
}

/// Run a multi-turn evaluation to test conversation history handling.
///
/// This is critical for testing OpenAI Responses API reasoning item preservation,
/// as the bug only manifests across multiple turns where reasoning IDs must be
/// preserved in history.
///
/// # Arguments
/// * `model` - The completion model to use
/// * `system_prompt` - System prompt for the agent
/// * `user_prompts` - Sequence of user prompts for each turn
/// * `config` - Eval configuration
///
/// # Returns
/// * `MultiTurnEvalOutput` containing outputs from each turn
pub async fn run_multi_turn_eval<M>(
    model: &M,
    system_prompt: &str,
    user_prompts: &[&str],
    config: EvalConfig,
) -> Result<MultiTurnEvalOutput>
where
    M: RigCompletionModel + Sync,
{
    let total_start = Instant::now();
    let mut turns = Vec::new();
    let mut current_history: Vec<Message> = Vec::new();

    // Create shared resources that persist across turns
    let tool_registry = Arc::new(RwLock::new(
        ToolRegistry::new(config.workspace.clone()).await,
    ));
    // Create sub-agent registry with default sub-agents
    let mut registry = SubAgentRegistry::new();
    registry.register_multiple(golish_sub_agents::create_default_sub_agents());
    let sub_agent_registry = Arc::new(RwLock::new(registry));
    let temp_dir = std::env::temp_dir().join("golish-eval-multiturn");
    std::fs::create_dir_all(&temp_dir).ok();
    let approval_recorder = Arc::new(ApprovalRecorder::new(temp_dir.clone()).await);
    let pending_approvals: Arc<RwLock<HashMap<String, oneshot::Sender<ApprovalDecision>>>> =
        Arc::new(RwLock::new(HashMap::new()));
    let tool_policy_manager = Arc::new(ToolPolicyManager::new(&config.workspace).await);
    let context_manager = Arc::new(ContextManager::with_config(
        &config.model_name,
        ContextManagerConfig {
            enabled: false,
            ..Default::default()
        },
    ));
    let loop_detector = Arc::new(RwLock::new(LoopDetector::with_defaults()));
    let compaction_state = Arc::new(RwLock::new(CompactionState::new()));
    let agent_mode = Arc::new(RwLock::new(AgentMode::AutoApprove));
    let plan_manager = Arc::new(PlanManager::new());
    let workspace_arc = Arc::new(RwLock::new(config.workspace.clone()));
    let llm_client = Arc::new(RwLock::new(LlmClient::Mock));
    let tool_config = ToolConfig::default();
    let capabilities = ModelCapabilities::detect(&config.provider_name, &config.model_name);

    for (turn_idx, user_prompt) in user_prompts.iter().enumerate() {
        let turn_start = Instant::now();
        tracing::info!(
            "Starting multi-turn eval turn {}/{}",
            turn_idx + 1,
            user_prompts.len()
        );

        // Create event channel for this turn
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AiEvent>();

        let api_request_stats = Arc::new(ApiRequestStats::new());

        let ctx = AgenticLoopContext {
            llm: LoopLlmRefs {
                client: &llm_client,
                provider_name: &config.provider_name,
                model_name: &config.model_name,
                openai_web_search_config: None,
                openai_reasoning_effort: None,
                openrouter_provider_preferences: None,
                model_factory: None,
            },
            access: LoopAccessControl {
                approval_recorder: &approval_recorder,
                pending_approvals: &pending_approvals,
                tool_policy_manager: &tool_policy_manager,
                agent_mode: &agent_mode,
                loop_detector: &loop_detector,
                coordinator: None,
            },
            events: LoopEventRefs {
                event_tx: &event_tx,
                transcript_writer: None,
                transcript_base_dir: None,
                session_id: None,
                db_tracker: None,
                runtime: None,
            },
            tool_registry: &tool_registry,
            sub_agent_registry: &sub_agent_registry,
            indexer_state: None,
            workspace: &workspace_arc,
            context_manager: &context_manager,
            compaction_state: &compaction_state,
            tool_config: &tool_config,
            sidecar_state: None,
            plan_manager: &plan_manager,
            api_request_stats: &api_request_stats,
            additional_tool_definitions: vec![],
            custom_tool_executor: None,
            cancelled: None,
            execution_monitor: None,
            execution_mode: crate::execution_mode::ExecutionMode::Chat,
            post_shell_hook: None,
            output_classifier: None,
        };

        let loop_config = AgenticLoopConfig {
            capabilities: capabilities.clone(),
            require_hitl: config.require_hitl,
            is_sub_agent: false,
            enable_reflector: false,
            tool_names_for_reflector: None,
        };

        // Add user message to current history
        current_history.push(Message::User {
            content: rig::one_or_many::OneOrMany::one(rig::message::UserContent::Text(
                rig::message::Text {
                    text: user_prompt.to_string(),
                },
            )),
        });

        let sub_agent_context = SubAgentContext {
            original_request: user_prompt.to_string(),
            ..Default::default()
        };

        // Run the unified loop with accumulated history
        let (response, _reasoning, new_history, tokens) =
            crate::agentic_loop::run_agentic_loop_unified(
                model,
                system_prompt,
                current_history.clone(),
                sub_agent_context,
                &ctx,
                loop_config,
            )
            .await?;

        // Update history with the new history from this turn
        current_history = new_history.clone();

        let turn_duration_ms = turn_start.elapsed().as_millis() as u64;

        // Collect events for this turn
        drop(event_tx);
        let mut events = Vec::new();
        while let Ok(event) = event_rx.try_recv() {
            events.push(event);
        }

        let (tool_calls, files_modified) = extract_tool_calls_and_files(&events, &config.workspace);

        let tokens_used = tokens.map(|t| (t.input_tokens + t.output_tokens) as u32);

        turns.push(EvalAgentOutput {
            response,
            tool_calls,
            files_modified,
            duration_ms: turn_duration_ms,
            tokens_used,
            history: new_history,
            events,
        });

        tracing::info!(
            "Completed multi-turn eval turn {}/{} in {}ms",
            turn_idx + 1,
            user_prompts.len(),
            turn_duration_ms
        );
    }

    let total_duration_ms = total_start.elapsed().as_millis() as u64;

    Ok(MultiTurnEvalOutput {
        turns,
        total_duration_ms,
        final_history: current_history,
    })
}

