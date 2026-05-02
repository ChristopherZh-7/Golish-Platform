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
