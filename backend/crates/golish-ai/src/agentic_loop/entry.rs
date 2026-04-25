use anyhow::Result;
use rig::completion::{CompletionModel as RigCompletionModel, Message};
use golish_context::token_budget::TokenUsage;
use golish_sub_agents::SubAgentContext;

use super::config::AgenticLoopConfig;
use super::context::AgenticLoopContext;
use super::run_agentic_loop_unified;

/// Execute the main agentic loop with tool calling.
///
/// This function runs the LLM completion loop, handling:
/// - Tool calls and results
/// - Loop detection
/// - Context window management
/// - HITL approval
/// - Extended thinking (streaming reasoning content)
///
/// Returns a tuple of (response_text, message_history, token_usage)
///
/// Note: This is the Anthropic-specific entry point that delegates to the unified loop
/// with thinking history support enabled.
///
/// Returns: (response, reasoning, history, token_usage)
pub async fn run_agentic_loop(
    model: &rig_anthropic_vertex::CompletionModel,
    system_prompt: &str,
    initial_history: Vec<Message>,
    context: SubAgentContext,
    ctx: &AgenticLoopContext<'_>,
) -> Result<(String, Option<String>, Vec<Message>, Option<TokenUsage>)> {
    run_agentic_loop_unified(
        model,
        system_prompt,
        initial_history,
        context,
        ctx,
        AgenticLoopConfig::main_agent_anthropic(),
    )
    .await
}

/// Generic agentic loop that works with any rig CompletionModel.
///
/// This is a simplified version of `run_agentic_loop` that:
/// - Works with any model implementing `rig::completion::CompletionModel`
/// - Does NOT support extended thinking (Anthropic-specific)
/// - Supports sub-agent calls (uses the same model for sub-agents)
///
/// Returns: (response, reasoning, history, token_usage)
///
/// Note: This is the generic entry point that delegates to the unified loop.
/// Model capabilities are detected from the provider/model name in the context.
pub async fn run_agentic_loop_generic<M>(
    model: &M,
    system_prompt: &str,
    initial_history: Vec<Message>,
    context: SubAgentContext,
    ctx: &AgenticLoopContext<'_>,
) -> Result<(String, Option<String>, Vec<Message>, Option<TokenUsage>)>
where
    M: RigCompletionModel + Sync,
{
    let config = AgenticLoopConfig::with_detection(ctx.provider_name, ctx.model_name, false);

    run_agentic_loop_unified(model, system_prompt, initial_history, context, ctx, config).await
}
