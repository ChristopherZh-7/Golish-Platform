//! Hooks that run once at the start of an agent turn, before the first LLM call.
//!
//! Two things happen here:
//! 1. Synchronous message hooks declared by [`HookRegistry`] are executed and
//!    their messages collected.
//! 2. The memory gatekeeper classifies whether the user message warrants a
//!    `search_memories` call; if so, an extra `[Memory-First]` hook message is
//!    appended.
//!
//! The collected messages (if any) are formatted via [`format_system_hooks`]
//! and injected into the chat history as a system-style user message.
//!
//! This function only runs when `iteration == 1` AND the agent is not a
//! sub-agent — sub-agents inherit hooks from the orchestrator.

use rig::completion::Message;
use rig::message::{Text, UserContent};
use rig::one_or_many::OneOrMany;

use golish_core::events::AiEvent;

use super::context::AgenticLoopContext;
use super::super::system_hooks::{format_system_hooks, HookRegistry, MessageHookContext};

/// Result of running the first-iteration hooks.
pub(super) struct FirstIterationOutcome {
    /// Whether the memory gatekeeper said memory search is warranted.
    /// Currently only logged; the hook message it produces drives behavior.
    pub gatekeeper_wants_memory: bool,
    /// Whether the reflector should be active for the rest of the turn.
    /// Disabled for trivial messages (short greetings/acks) that clearly don't
    /// need tool calls — re-enabled by `wants_memory` since memory-relevant
    /// requests benefit from reflection.
    pub reflector_active: bool,
}

impl Default for FirstIterationOutcome {
    fn default() -> Self {
        Self {
            gatekeeper_wants_memory: false,
            // Reflector defaults active so pentest/task prompts always get
            // reflector coverage, regardless of gatekeeper decision.
            reflector_active: true,
        }
    }
}

/// Run synchronous message hooks + the async memory gatekeeper, and inject any
/// resulting system-hook user message into `chat_history`.
pub(super) async fn run_first_iteration_hooks(
    ctx: &AgenticLoopContext<'_>,
    hook_registry: &HookRegistry,
    chat_history: &mut Vec<Message>,
) -> FirstIterationOutcome {
    let mut outcome = FirstIterationOutcome::default();

    let last_user_text = chat_history.iter().rev().find_map(|msg| {
        if let Message::User { content } = msg {
            content.iter().find_map(|c| {
                if let UserContent::Text(t) = c {
                    Some(t.text.as_str())
                } else {
                    None
                }
            })
        } else {
            None
        }
    });

    let Some(user_text) = last_user_text else {
        return outcome;
    };

    let msg_ctx = MessageHookContext::user_input(user_text, ctx.session_id.unwrap_or(""));
    let mut hook_messages = hook_registry.run_message_hooks(&msg_ctx);

    // Memory gatekeeper: classifies whether memory search is warranted for this message.
    {
        let client = ctx.client.read().await;
        let wants_memory =
            crate::memory_gatekeeper::should_search_memory(&client, user_text).await;
        outcome.gatekeeper_wants_memory = wants_memory;
        if wants_memory {
            hook_messages.push(
                "[Memory-First] The gatekeeper determined this message may benefit \
                 from prior context. Call `search_memories` with relevant keywords \
                 before responding."
                    .to_string(),
            );
        }

        // Reflector should be active for any substantive request — only disable
        // it for trivial messages that clearly don't need tools.
        let trimmed = user_text.trim();
        let is_trivial = trimmed.len() < 20
            && !trimmed.contains("scan")
            && !trimmed.contains("test")
            && !trimmed.contains("exploit");
        outcome.reflector_active = !is_trivial || wants_memory;
    }

    if !hook_messages.is_empty() {
        let formatted = format_system_hooks(&hook_messages);
        tracing::info!(
            count = hook_messages.len(),
            "Injecting message hooks before first LLM call"
        );

        let _ = ctx.event_tx.send(AiEvent::SystemHooksInjected {
            hooks: hook_messages,
        });

        chat_history.push(Message::User {
            content: OneOrMany::one(UserContent::Text(Text { text: formatted })),
        });
    }

    outcome
}
