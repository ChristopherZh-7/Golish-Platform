//! Agent executor for evaluations using the unified agentic loop.
//!
//! This executor runs the same agentic loop as the main application, ensuring
//! evaluations test actual production behavior. It sets up minimal mock
//! dependencies and auto-approves all tool calls.
//!
//! Supports multiple LLM providers:
//! - Vertex AI Claude Sonnet (default)
//! - Z.AI GLM-4.7
//! - OpenAI GPT-5.1

use std::path::Path;
use std::sync::Arc;

use tokio::sync::RwLock;

use golish_ai::agent_mode::AgentMode;
use golish_ai::contributors::create_default_contributors;
use golish_ai::prompt_registry::PromptContributorRegistry;
use golish_ai::system_prompt::build_system_prompt_with_contributions;
use golish_core::PromptContext;
use golish_sub_agents::SubAgentRegistry;

use crate::config::EvalProvider;


/// Build the production system prompt with all contributions.
///
/// This builds the same prompt as the main agent (agent_bridge.rs), including:
/// - Sub-agent documentation (when has_sub_agents = true)
/// - Provider-specific tool instructions (when has_web_search = true)
///
/// # Arguments
/// * `workspace` - The workspace directory
/// * `provider` - The provider being used for this eval
///
/// # Returns
/// The complete system prompt string with all contributions appended.
pub fn build_production_system_prompt(workspace: &Path, provider: EvalProvider) -> String {
    // Create sub-agent registry with default agents (same as main agent)
    let sub_agent_registry = Arc::new(RwLock::new(SubAgentRegistry::new()));

    // Create prompt contributor registry with default contributors
    let contributors = create_default_contributors(sub_agent_registry);
    let mut registry = PromptContributorRegistry::new();
    for contributor in contributors {
        registry.register(contributor);
    }

    // Map eval provider to provider name for context
    let provider_name = match provider {
        EvalProvider::VertexClaude => "anthropic",
        EvalProvider::Zai => "zai",
        EvalProvider::OpenAi => "openai",
    };

    // Create prompt context with provider, model, and feature flags
    // For evals:
    // - has_web_search is true for Vertex Claude (native web search enabled)
    // - has_sub_agents is true (same as main agent)
    let has_web_search = matches!(provider, EvalProvider::VertexClaude);
    let has_sub_agents = true;

    let prompt_context = PromptContext::new(provider_name, "eval-model")
        .with_web_search(has_web_search)
        .with_sub_agents(has_sub_agents)
        .with_workspace(workspace.display().to_string());

    // No memory file for evals - testbeds are isolated workspaces
    build_system_prompt_with_contributions(
        workspace,
        AgentMode::AutoApprove, // Evals always auto-approve
        None,                   // No memory file
        Some(&registry),
        Some(&prompt_context),
    )
}

mod multi_turn;
mod single_turn;

#[cfg(test)]
mod tests;

pub use multi_turn::{execute_multi_turn_eval, MultiTurnAgentOutput};
pub use single_turn::{
    execute_eval_prompt, execute_eval_prompt_with_model, execute_eval_prompt_with_options,
    execute_eval_prompt_with_provider, execute_eval_prompt_with_system,
    execute_eval_prompt_with_tools,
};
