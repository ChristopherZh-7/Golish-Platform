//! FirstIterHooks phase — run synchronous message hooks + memory
//! gatekeeper on the first iteration of the main agent.
//!
//! No-op for sub-agents (they don't own the hook registry).
//! Delegates to the existing `first_iter_hooks::run_first_iteration_hooks`
//! helper and folds its `reflector_active` output back into `TurnState`.

use rig::completion::Message;

use golish_agent_kit::system_hooks::HookRegistry;

use super::super::super::config::AgenticLoopConfig;
use super::super::super::context::AgenticLoopContext;
use super::super::super::first_iter_hooks::run_first_iteration_hooks;
use super::super::state::TurnState;
use super::PhaseOutcome;

/// Run first-iteration hooks (only on iteration 1 of the main agent).
pub async fn run(
    state: &mut TurnState,
    ctx: &AgenticLoopContext<'_>,
    config: &AgenticLoopConfig,
    hook_registry: &HookRegistry,
    chat_history: &mut Vec<Message>,
) -> PhaseOutcome {
    if state.iteration == 1 && !config.is_sub_agent {
        let outcome = run_first_iteration_hooks(ctx, hook_registry, chat_history).await;
        state.reflector_active = outcome.reflector_active;
    }
    PhaseOutcome::Continue
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use golish_llm_providers::{LlmClient, ModelCapabilities};
    use tokio::sync::RwLock;

    use crate::test_utils::TestContextBuilder;

    use super::*;

    fn sub_agent_config() -> AgenticLoopConfig {
        AgenticLoopConfig::sub_agent(ModelCapabilities::conservative_defaults())
    }

    #[tokio::test]
    async fn iteration_two_short_circuits_without_running_hooks() {
        let test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);
        let mut state = TurnState {
            iteration: 2,
            reflector_active: false,
            ..TurnState::default()
        };
        let config = AgenticLoopConfig::main_agent_generic();
        let registry = HookRegistry::new();
        let mut history: Vec<Message> = vec![];
        let history_len_before = history.len();

        let outcome = run(&mut state, &ctx, &config, &registry, &mut history).await;

        assert!(matches!(outcome, PhaseOutcome::Continue));
        assert!(
            !state.reflector_active,
            "iteration > 1 must NOT touch reflector_active"
        );
        assert_eq!(
            history.len(),
            history_len_before,
            "iteration > 1 must NOT modify chat history"
        );
    }

    #[tokio::test]
    async fn sub_agent_skips_hooks_even_on_iteration_one() {
        let test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);
        let mut state = TurnState {
            iteration: 1,
            reflector_active: false,
            ..TurnState::default()
        };
        let config = sub_agent_config();
        let registry = HookRegistry::new();
        let mut history: Vec<Message> = vec![];

        let outcome = run(&mut state, &ctx, &config, &registry, &mut history).await;

        assert!(matches!(outcome, PhaseOutcome::Continue));
        assert!(
            !state.reflector_active,
            "sub-agent must NOT alter reflector_active"
        );
        assert!(history.is_empty(), "sub-agent must NOT inject hook messages");
    }
}
