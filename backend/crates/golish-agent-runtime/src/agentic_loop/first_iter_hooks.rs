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
use golish_agent_kit::system_hooks::{format_system_hooks, HookRegistry, MessageHookContext};

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

/// Strip the `[System Context]\n…\n\n[User Message]\n` prefix that
/// `frontend/components/AIChatPanel/hooks/useChatSend.ts` prepends to the
/// very first message of a conversation. Returns the original `text` when no
/// such prefix is found so heuristics still see the raw user input on every
/// subsequent turn.
fn extract_real_user_text(text: &str) -> &str {
    const MARKER: &str = "[User Message]\n";
    if text.starts_with("[System Context]\n") {
        if let Some(idx) = text.find(MARKER) {
            return text[idx + MARKER.len()..].trim_start();
        }
    }
    text
}

/// Whether the user's first-turn input is too low-signal to drive any tool
/// call. In chat mode this triggers an Input Gate hook that tells the model
/// to ask for clarification in plain text instead of dispatching to
/// `sub_agent_*`, `list_files`, `search_memories`, etc.
///
/// Heuristics:
/// - shorter than 8 chars (after trim)
/// - all-digit / all-whitespace (e.g. "123", "  42  ")
/// - common chitchat openers ("hi", "hello", "test", "你好", ...)
pub(super) fn is_low_signal_input(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }
    if trimmed.len() < 8 {
        return true;
    }
    let all_digits_or_ws = trimmed
        .chars()
        .all(|c| c.is_ascii_digit() || c.is_whitespace());
    if all_digits_or_ws {
        return true;
    }
    matches!(
        trimmed.to_ascii_lowercase().as_str(),
        "hi" | "hello"
            | "hey"
            | "test"
            | "ok"
            | "okay"
            | "你好"
            | "哈喽"
            | "测试"
            | "在吗"
            | "嗯"
    )
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

    let msg_ctx = MessageHookContext::user_input(user_text, ctx.events.session_id.unwrap_or(""));
    let mut hook_messages = hook_registry.run_message_hooks(&msg_ctx);

    // Memory gatekeeper: classifies whether memory search is warranted for this message.
    {
        let client = ctx.llm.client.read().await;
        let wants_memory =
            golish_agent_kit::memory_gatekeeper::should_search_memory(&client, user_text).await;
        outcome.gatekeeper_wants_memory = wants_memory;
        if wants_memory {
            hook_messages.push(
                "[Memory-First] The gatekeeper determined this message may benefit \
                 from prior context. Call `search_memories` with relevant keywords \
                 before responding."
                    .to_string(),
            );
        }

        // Reflector nudges the model to "go use a tool" whenever a turn
        // produces text only. That's helpful in Task mode (the orchestrator
        // is supposed to drive a workflow) but actively hostile in Chat
        // mode, where greetings, code-Q&A and casual replies should stay
        // text-only without being flipped into a recon/list_files spree.
        //
        // **Critical**: the first user message is wrapped as
        //   `[System Context]\n<huge system prompt>\n\n[User Message]\n<real text>`
        // by `useChatSend`. We MUST strip the system-context prefix before
        // running keyword / triviality heuristics, otherwise the system
        // prompt's mentions of "scan / test / exploit" make every turn look
        // like a pentest request and reflector fires forever.
        let real_user_text = extract_real_user_text(user_text);
        let trimmed = real_user_text.trim();
        let lower = trimmed.to_ascii_lowercase();
        let has_pentest_keyword = lower.contains("scan")
            || lower.contains("recon")
            || lower.contains("exploit")
            || lower.contains("fuzz")
            || lower.contains("pentest")
            || trimmed.contains("渗透")
            || trimmed.contains("扫描");
        let is_trivial = trimmed.len() < 20 && !has_pentest_keyword;

        // Low-signal inputs ("123", "hi", pure digits, common chitchat) must
        // never trigger tool calls in chat mode — the chat agent is supposed
        // to handle the whole turn itself, so a trivial input should produce
        // a plain-text clarification request rather than a `search_memories`
        // / `list_files` / sub-agent dispatch spree.
        //
        // Task mode is exempt: the orchestrator is meant to drive a workflow
        // and gets to ask the user via `sub_agent_*` channels.
        let is_low_signal = is_low_signal_input(real_user_text);
        if is_low_signal && !ctx.execution_mode.is_task() {
            hook_messages.push(
                "[Input Gate] 用户输入过短或语义不清（如 \"123\" / \"hi\" / 纯数字）。\
                 请用一两句话礼貌地请用户补充意图（例如「你想做什么？扫描某个目标 / 分析某个文件 / 排查某个 bug？」），\
                 然后立即结束本轮。\
                 严禁调用任何工具——包括 search_memories、list_files、grep_file、sub_agent_*、shell 等。"
                    .to_string(),
            );
        }

        outcome.reflector_active = if ctx.execution_mode.is_task() {
            !is_trivial || wants_memory
        } else {
            // Chat mode: only nudge when the user explicitly used pentest
            // keywords. Wants-memory alone is not enough to justify a nudge.
            // Also skip the nudge entirely when the input gate fired.
            !is_low_signal && (has_pentest_keyword || wants_memory)
        };
    }

    if !hook_messages.is_empty() {
        let formatted = format_system_hooks(&hook_messages);
        tracing::info!(
            count = hook_messages.len(),
            "Injecting message hooks before first LLM call"
        );

        let _ = ctx.events.event_tx.send(AiEvent::SystemHooksInjected {
            hooks: hook_messages,
        });

        chat_history.push(Message::User {
            content: OneOrMany::one(UserContent::Text(Text { text: formatted })),
        });
    }

    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_real_user_text_strips_system_context_prefix() {
        let input = "[System Context]\n<lots of stuff>\n\n[User Message]\nreal text";
        assert_eq!(extract_real_user_text(input), "real text");
    }

    #[test]
    fn extract_real_user_text_passthrough_when_no_prefix() {
        assert_eq!(extract_real_user_text("just a message"), "just a message");
    }

    #[test]
    fn low_signal_pure_digits() {
        assert!(is_low_signal_input("123"));
        assert!(is_low_signal_input("  42  "));
        assert!(is_low_signal_input("0000000000"));
    }

    #[test]
    fn low_signal_short_strings() {
        assert!(is_low_signal_input(""));
        assert!(is_low_signal_input("?"));
        assert!(is_low_signal_input("hi"));
        assert!(is_low_signal_input("hello"));
        assert!(is_low_signal_input("HELLO"));
    }

    #[test]
    fn low_signal_chitchat_cn() {
        assert!(is_low_signal_input("你好"));
        assert!(is_low_signal_input("测试"));
        assert!(is_low_signal_input("在吗"));
    }

    #[test]
    fn high_signal_real_request() {
        assert!(!is_low_signal_input(
            "scan 192.168.1.1 with nmap and report open ports"
        ));
        assert!(!is_low_signal_input("分析 src/auth.rs 的鉴权逻辑"));
        assert!(!is_low_signal_input(
            "请帮我看下登录流程的潜在 IDOR 漏洞"
        ));
    }
}
