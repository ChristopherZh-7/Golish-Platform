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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use golish_llm_providers::LlmClient;
    use rig::completion::ToolDefinition;
    use rig::message::UserContent;
    use tokio::sync::RwLock;

    use crate::test_utils::TestContextBuilder;

    use super::*;

    fn config_with_reflector(enabled: bool) -> AgenticLoopConfig {
        let mut cfg = AgenticLoopConfig::main_agent_generic();
        cfg.enable_reflector = enabled;
        cfg
    }

    fn make_tool(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: format!("Mock {name} tool"),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    #[tokio::test]
    async fn tool_calls_present_resets_counter_and_continues() {
        let test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);
        let mut state = TurnState {
            consecutive_no_tool_turns: 2,
            ..TurnState::default()
        };
        let mut history: Vec<Message> = vec![];
        let sub_ctx = SubAgentContext::default();
        let cfg = config_with_reflector(true);

        let outcome = run(
            &mut state,
            &ctx,
            &sub_ctx,
            &cfg,
            &mut history,
            true,
            "any text",
            &[],
        )
        .await;

        assert!(matches!(outcome, ReflectorPhaseOutcome::Continue));
        assert_eq!(
            state.consecutive_no_tool_turns, 0,
            "tool calls present must reset the no-tool counter"
        );
    }

    #[tokio::test]
    async fn no_tool_calls_with_inactive_reflector_breaks_after_increment() {
        let test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);
        let mut state = TurnState {
            reflector_active: false,
            consecutive_no_tool_turns: 0,
            ..TurnState::default()
        };
        let mut history: Vec<Message> = vec![];
        let sub_ctx = SubAgentContext::default();
        let cfg = config_with_reflector(true);

        let outcome = run(
            &mut state,
            &ctx,
            &sub_ctx,
            &cfg,
            &mut history,
            false,
            "I cannot help with that.",
            &[],
        )
        .await;

        assert!(matches!(outcome, ReflectorPhaseOutcome::Break));
        assert_eq!(
            state.consecutive_no_tool_turns, 1,
            "no-tool-call branch increments counter even when reflector is inactive"
        );
        assert_eq!(
            state.total_reflector_nudges, 0,
            "nudge counter must NOT increment when reflector is skipped"
        );
    }

    #[tokio::test]
    async fn textual_tool_call_markup_retries_even_when_reflector_is_inactive() {
        let test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);
        let mut state = TurnState {
            reflector_active: false,
            ..TurnState::default()
        };
        let mut history: Vec<Message> = vec![];
        let sub_ctx = SubAgentContext::default();
        let cfg = config_with_reflector(true);
        let tools = vec![make_tool("manage_targets")];

        let outcome = run(
            &mut state,
            &ctx,
            &sub_ctx,
            &cfg,
            &mut history,
            false,
            "好的，我先检查目标。\n<tool_call>\n</tool_call>",
            &tools,
        )
        .await;

        assert!(matches!(outcome, ReflectorPhaseOutcome::Repeat));
        assert_eq!(state.consecutive_no_tool_turns, 1);
        assert_eq!(state.total_reflector_nudges, 1);
        assert_eq!(history.len(), 1);

        let Message::User { content } = &history[0] else {
            panic!("expected injected correction as user message");
        };
        let Some(UserContent::Text(text)) = content.iter().next() else {
            panic!("expected text correction");
        };
        assert!(text
            .text
            .contains("native structured tool/function-calling"));
        assert!(text.text.contains("manage_targets"));
    }

    #[tokio::test]
    async fn no_tool_calls_with_empty_text_breaks_immediately() {
        let test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);
        let mut state = TurnState::default();
        let mut history: Vec<Message> = vec![];
        let sub_ctx = SubAgentContext::default();
        let cfg = config_with_reflector(true);

        let outcome = run(
            &mut state,
            &ctx,
            &sub_ctx,
            &cfg,
            &mut history,
            false,
            "   ",
            &[],
        )
        .await;

        assert!(matches!(outcome, ReflectorPhaseOutcome::Break));
        assert_eq!(state.consecutive_no_tool_turns, 1);
        assert_eq!(state.total_reflector_nudges, 0);
    }
}
