//! Reflector chain integration: when an iteration produces text without a tool
//! call, optionally invoke a `reflector` sub-agent to diagnose why and inject a
//! corrective user message into the history so the model retries with a tool.
//!
//! The reflector budget is bounded by:
//! - `consecutive_no_tool_turns <= 3` — guards against runaway no-tool loops.
//! - `total_reflector_nudges < 3`     — caps total reflections per agent run.
//! - `config.enable_reflector`        — feature flag.
//! - `reflector_active`               — disabled for trivial messages
//!   (greetings/acks) where a text-only response is expected.

use rig::completion::Message;
use rig::message::{Text, UserContent};
use rig::one_or_many::OneOrMany;

use golish_core::utils::truncate_str;
use golish_sub_agents::SubAgentContext;

use super::config::AgenticLoopConfig;
use super::context::AgenticLoopContext;

/// Outcome of [`maybe_run_reflector`].
pub(super) enum ReflectorOutcome {
    /// A correction was injected into `chat_history` — the loop should `continue`.
    Injected,
    /// No reflection happened — caller decides next step.
    Skipped,
}

/// If the agent produced text without tool calls and reflection budget remains,
/// invoke the reflector sub-agent and append a corrective user message.
///
/// `total_reflector_nudges` is incremented when reflection actually fires.
pub(super) async fn maybe_run_reflector(
    ctx: &AgenticLoopContext<'_>,
    sub_agent_context: &SubAgentContext,
    config: &AgenticLoopConfig,
    chat_history: &mut Vec<Message>,
    text_content: &str,
    consecutive_no_tool_turns: u32,
    total_reflector_nudges: &mut u32,
    reflector_active: bool,
    tools: &[rig::completion::ToolDefinition],
) -> ReflectorOutcome {
    let should_reflect = consecutive_no_tool_turns <= 3
        && *total_reflector_nudges < 3
        && !text_content.trim().is_empty()
        && config.enable_reflector
        && reflector_active;

    if !should_reflect {
        return ReflectorOutcome::Skipped;
    }

    let reflector_def = {
        let registry = ctx.sub_agent_registry.read().await;
        registry.get("reflector").cloned()
    };

    let Some(reflector_def) = reflector_def else {
        return ReflectorOutcome::Skipped;
    };

    *total_reflector_nudges += 1;
    tracing::info!(
        attempt = consecutive_no_tool_turns,
        total_nudges = *total_reflector_nudges,
        text_len = text_content.len(),
        "[reflector] Agent produced text without tool calls, invoking reflector chain"
    );

    // Build a diagnostic prompt for the reflector with the agent's response and
    // available tool names so it can suggest specific tools.
    let tool_list = config
        .tool_names_for_reflector
        .as_ref()
        .map(|names| names.join(", "))
        .unwrap_or_else(|| {
            tools
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        });

    let reflector_prompt = format!(
        "The agent was given a task and responded with text instead of using tools.\n\n\
         ## Agent's text response (attempt {}/3):\n```\n{}\n```\n\n\
         ## Available tools:\n{}\n\n\
         Diagnose why the agent didn't use tools and write a corrective instruction \
         that will get it to take action. Be specific about which tool to use and with what arguments.",
        *total_reflector_nudges,
        truncate_str(text_content, 2000),
        tool_list
    );

    let reflector_args = serde_json::json!({
        "task": reflector_prompt,
    });

    let correction = {
        use crate::tool_provider_impl::DefaultToolProvider;
        let tool_provider = DefaultToolProvider::with_db_tracker(ctx.db_tracker);
        let sub_ctx = golish_sub_agents::SubAgentExecutorContext {
            event_tx: ctx.event_tx,
            tool_registry: ctx.tool_registry,
            workspace: ctx.workspace,
            provider_name: ctx.provider_name,
            model_name: ctx.model_name,
            session_id: ctx.session_id,
            transcript_base_dir: ctx.transcript_base_dir,
            api_request_stats: Some(ctx.api_request_stats),
            briefing: None,
            temperature_override: reflector_def.temperature,
            max_tokens_override: reflector_def.max_tokens,
            top_p_override: reflector_def.top_p,
            db_pool: ctx.db_tracker.map(|t| t.pool_arc()),
            sub_agent_registry: Some(ctx.sub_agent_registry),
            post_shell_hook: None,
        };

        match super::sub_agent_dispatch::execute_sub_agent_with_client(
            &reflector_def,
            &reflector_args,
            sub_agent_context,
            &*ctx.client.read().await,
            sub_ctx,
            &tool_provider,
            "reflector",
        )
        .await
        {
            Ok(result) => {
                tracing::info!(
                    "[reflector] Chain returned {} chars of correction",
                    result.response.len()
                );
                result.response
            }
            Err(e) => {
                tracing::warn!("[reflector] Chain failed, using fallback nudge: {}", e);
                format!(
                    "[System: You responded with text but did not use any tools. \
                     Please execute the next step using the appropriate tool. \
                     Available tools: {}. Attempt {}/3]",
                    tool_list, *total_reflector_nudges
                )
            }
        }
    };

    chat_history.push(Message::User {
        content: OneOrMany::one(UserContent::Text(Text { text: correction })),
    });

    ReflectorOutcome::Injected
}
