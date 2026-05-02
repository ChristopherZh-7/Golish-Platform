//! ReflectorOrBreak phase — handle the no-tool-call branch of one
//! iteration: either invoke the reflector to inject a corrective prompt
//! (and repeat the iteration) or break the loop entirely.
//!
//! Returns `ReflectorPhaseOutcome` (a phase-local enum) instead of the
//! generic `PhaseOutcome` because this is the one phase that needs a
//! "skip the rest of this iteration and start a fresh one" signal in
//! addition to Continue/Break. The scheduler maps `Repeat` to a
//! `continue` on the outer loop.
//!
//! State updates this phase performs on `TurnState`:
//! - `consecutive_no_tool_turns += 1` when there are no tool calls.
//! - `consecutive_no_tool_turns  = 0` when there are tool calls.
//! - `total_reflector_nudges`    is bumped inside `maybe_run_reflector`
//!   only if a correction is actually injected.

use rig::completion::Message;

use golish_sub_agents::SubAgentContext;

use super::super::super::config::AgenticLoopConfig;
use super::super::super::context::AgenticLoopContext;
use super::super::super::reflector::{maybe_run_reflector, ReflectorOutcome};
use super::super::state::TurnState;

/// Outcome of the ReflectorOrBreak phase.
pub enum ReflectorPhaseOutcome {
    /// Has tool calls (or this phase is otherwise satisfied) — proceed
    /// to ToolDispatch.
    Continue,
    /// Reflector injected a correction; the scheduler must skip the
    /// remaining phases of this iteration and start a fresh iteration.
    Repeat,
    /// No tool calls and the reflector skipped — terminate the loop.
    Break,
}

/// Decide what to do at the end of an LLM turn that produced no tool
/// calls: optionally run the reflector and tell the scheduler whether
/// to continue, repeat, or break.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    state: &mut TurnState,
    ctx: &AgenticLoopContext<'_>,
    sub_agent_context: &SubAgentContext,
    config: &AgenticLoopConfig,
    chat_history: &mut Vec<Message>,
    has_tool_calls: bool,
    text_content: &str,
    tools: &[rig::completion::ToolDefinition],
) -> ReflectorPhaseOutcome {
    if has_tool_calls {
        state.consecutive_no_tool_turns = 0;
        return ReflectorPhaseOutcome::Continue;
    }

    state.consecutive_no_tool_turns += 1;

    match maybe_run_reflector(
        ctx,
        sub_agent_context,
        config,
        chat_history,
        text_content,
        state.consecutive_no_tool_turns,
        &mut state.total_reflector_nudges,
        state.reflector_active,
        tools,
    )
    .await
    {
        ReflectorOutcome::Injected => ReflectorPhaseOutcome::Repeat,
        ReflectorOutcome::Skipped => ReflectorPhaseOutcome::Break,
    }
}
